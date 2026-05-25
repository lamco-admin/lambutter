// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! `no_std` read-only btrfs reader.
//!
//! See `docs/SPEC-LAMBUTTER.md` for the design specification and
//! `~/lamboot-dev/docs/analysis/BTRFS-FORMAT-READONLY-REFERENCE-2026-04-27.md`
//! for the on-disk format reference.

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]

extern crate alloc;

#[cfg(feature = "std")]
extern crate std;

use alloc::vec::Vec;

mod block_read;
mod btree;
mod checksum;
mod chunk_tree;
mod compression;
mod dir;
mod error;
mod file;
mod format;
mod inode;
mod path;
mod resolve;
mod root_tree;
mod superblock;
mod util;

pub use block_read::{BlockRead, SliceReadError};
use chunk_tree::ChunkMap;
pub use dir::DirEntry;
pub use error::{Error, Result, SuperblockReason};
pub use inode::{Inode, Metadata};
pub use path::Path;

/// Internals exposed solely for fuzz harnesses. NOT part of the public API
/// surface; covered by no SemVer guarantee; gated on `cfg(fuzzing)` so it
/// is unreachable from a normal build. Documented per
/// `docs/TESTING-AND-FUZZING-PLAN.md` §6.4.
#[doc(hidden)]
#[cfg(fuzzing)]
pub mod __fuzz_internals {
    use alloc::vec::Vec;

    /// Drive the compression dispatcher directly with arbitrary algorithm
    /// + payload bytes. Used by `fuzz_compressed_extent`.
    pub fn decode(algorithm: u8, src: &[u8], dst: &mut Vec<u8>) -> crate::Result<()> {
        crate::compression::decode(algorithm, src, dst)
    }

    /// Drive the chunk-map's system-array parser directly with arbitrary
    /// bytes. Used to fuzz the bootstrap path.
    pub fn parse_system_chunk_array(bytes: &[u8]) -> crate::Result<()> {
        let mut map = crate::chunk_tree::ChunkMap::default();
        map.parse_system_chunk_array(bytes, bytes.len())
    }

    /// Drive the name-hash function directly. Used by `fuzz_dir_item`-style
    /// targets that want to stress the table.
    pub fn name_hash(name: &[u8]) -> u32 {
        crate::checksum::crc32c_with_seed(0xFFFF_FFFE, name)
    }
}

/// A mounted, read-only btrfs filesystem.
///
/// Construct via [`Btrfs::open`], then resolve paths and read files using
/// the methods below. The instance owns the block reader for its lifetime;
/// drop the `Btrfs` to release it.
pub struct Btrfs<R: BlockRead> {
    reader: R,
    chunk_map: ChunkMap,
    nodesize: u32,
    fs_tree_root: u64,
    /// Subvolume objectid that was selected as the active default. Useful
    /// for diagnostics; the FS tree root is what actual reads go through.
    default_subvol_objectid: u64,
}

impl<R: BlockRead> Btrfs<R> {
    /// Open a btrfs filesystem on top of `reader`. `device_size_bytes` is
    /// the byte length of the underlying volume; lambutter consults it to
    /// decide which superblock copies are addressable.
    pub fn open(mut reader: R, device_size_bytes: u64) -> Result<Self> {
        let sb = superblock::load(&mut reader, device_size_bytes)?;

        let mut chunk_map = ChunkMap::default();
        chunk_map
            .parse_system_chunk_array(&sb.sys_chunk_array, sb.sys_chunk_array_size as usize)?;

        // Walk the chunk tree to extend the chunk map with all chunks
        // (data, metadata, system) — the system array only covers what's
        // needed to bootstrap into the chunk tree itself.
        chunk_tree::populate_from_chunk_tree(
            &mut reader,
            &mut chunk_map,
            sb.nodesize,
            sb.chunk_root,
        )?;

        // Resolve the active default subvolume's FS tree root.
        let (fs_tree_root, _fs_tree_level, default_subvol_objectid) =
            root_tree::resolve_default_subvol(
                &mut reader,
                &chunk_map,
                sb.nodesize,
                sb.root,
                sb.root_dir_objectid,
            )?;

        Ok(Self {
            reader,
            chunk_map,
            nodesize: sb.nodesize,
            fs_tree_root,
            default_subvol_objectid,
        })
    }

    /// Subvolume objectid resolved as the default at mount time. For audit /
    /// diagnostic use.
    pub fn default_subvol_objectid(&self) -> u64 {
        self.default_subvol_objectid
    }

    /// Resolve `path` to an inode within the active subvolume.
    pub fn resolve(&mut self, path: Path<'_>) -> Result<Inode> {
        let objectid = resolve::resolve_path(
            &mut self.reader,
            &self.chunk_map,
            self.nodesize,
            self.fs_tree_root,
            path,
        )?;
        Ok(Inode { objectid })
    }

    /// Read inode metadata.
    pub fn metadata(&mut self, inode: &Inode) -> Result<Metadata> {
        file::read_metadata(
            &mut self.reader,
            &self.chunk_map,
            self.nodesize,
            self.fs_tree_root,
            inode.objectid,
        )
    }

    /// Read the full contents of `path`. Errors if the path doesn't resolve
    /// or doesn't point to a regular file.
    ///
    /// Prefer [`Btrfs::read_file_at`] for files larger than a few MiB — this
    /// method allocates the entire file in one [`Vec<u8>`].
    pub fn read_file(&mut self, path: Path<'_>) -> Result<Vec<u8>> {
        let inode = self.resolve(path)?;
        file::read_file(
            &mut self.reader,
            &self.chunk_map,
            self.nodesize,
            self.fs_tree_root,
            inode.objectid,
        )
    }

    /// Read up to `buf.len()` bytes from the file at the resolved `inode`
    /// starting at byte `offset`. Returns the number of bytes written
    /// (zero indicates end-of-file). Holes and prealloc extents read as
    /// zeros; reads past EOF return 0.
    ///
    /// Memory cost is bounded at the size of one extent (typically a few
    /// MiB) plus `buf.len()`, so this is the right API for bootloaders
    /// streaming a kernel image or initrd in fixed-size chunks.
    pub fn read_file_at(&mut self, inode: &Inode, offset: u64, buf: &mut [u8]) -> Result<usize> {
        file::read_file_at(
            &mut self.reader,
            &self.chunk_map,
            self.nodesize,
            self.fs_tree_root,
            inode.objectid,
            offset,
            buf,
        )
    }

    /// Read the target of the symlink at `path`. Errors if the path doesn't
    /// resolve or doesn't point to a symlink. Returns the target as raw
    /// bytes; callers handle UTF-8 decoding and relative/absolute resolution.
    pub fn read_link(&mut self, path: Path<'_>) -> Result<Vec<u8>> {
        let inode = self.resolve(path)?;
        file::read_link(
            &mut self.reader,
            &self.chunk_map,
            self.nodesize,
            self.fs_tree_root,
            inode.objectid,
        )
    }

    /// Iterate the entries of the directory at `path`.
    pub fn read_dir(&mut self, path: Path<'_>) -> Result<Vec<DirEntry>> {
        let inode = self.resolve(path)?;
        dir::read_dir(
            &mut self.reader,
            &self.chunk_map,
            self.nodesize,
            self.fs_tree_root,
            inode.objectid,
        )
    }
}
