// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! `no_std` read-only btrfs reader for UEFI bootloaders and embedded contexts.
//!
//! Lambutter is the btrfs counterpart to [`ext4-view`]: a pure-Rust,
//! allocation-aware, read-only btrfs filesystem reader for contexts that cannot
//! link `std`. It opens regular files, symlinks, and directory listings on
//! SINGLE / DUP / RAID1 / RAID1C3 / RAID1C4 volumes and decodes zstd, zlib, and
//! LZO extents; unsupported profiles surface as typed errors.
//!
//! See the [README] for scope, validation posture, and usage; `CHANGELOG.md`
//! for the version history.
//!
//! [`ext4-view`]: https://github.com/nicholasbishop/ext4-view-rs
//! [README]: https://github.com/lamco-admin/lambutter#readme

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(rustdoc::broken_intra_doc_links)]
// Test code may panic on failure (unwrap/expect); keep the library strict.
#![cfg_attr(test, expect(clippy::unwrap_used, clippy::expect_used))]

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
mod le;
mod path;
mod resolve;
mod root_tree;
mod superblock;

pub use block_read::{BlockRead, SliceReadError};
use chunk_tree::ChunkMap;
pub use dir::DirEntry;
pub use error::{Error, Result, SuperblockReason};
use format::constants::{FIRST_FREE_OBJECTID, FS_TREE_OBJECTID};
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
        crate::compression::decode(algorithm, src, dst, 4096)
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
/// Construct via [`Btrfs::open`] (the default subvolume, as a plain Linux
/// mount sees it) or [`Btrfs::open_subvol_id`] / [`Btrfs::open_subvol_path`]
/// (a chosen subvolume, as `subvolid=` and `subvol=` mount it), then resolve
/// paths and read files using the methods below. Paths cross into nested
/// subvolumes the way a mount shows them. The instance owns the block reader
/// for its lifetime; drop the `Btrfs` to release it.
pub struct Btrfs<R: BlockRead> {
    reader: R,
    chunk_map: ChunkMap,
    geometry: file::Geometry,
    /// Logical address of the root tree, for crossing into subvolumes.
    root_tree_root: u64,
    /// Root directory of the subvolume this instance was opened at.
    root: Inode,
}

impl<R: BlockRead> Btrfs<R> {
    /// Open a btrfs filesystem on top of `reader`, at its default subvolume.
    /// `device_size_bytes` is the byte length of the underlying volume;
    /// lambutter consults it to decide which superblock copies are
    /// addressable.
    pub fn open(reader: R, device_size_bytes: u64) -> Result<Self> {
        let (mut fs, default_hint) = Self::mount(reader, device_size_bytes)?;
        fs.root = root_tree::resolve_default_subvol(
            &mut fs.reader,
            &fs.chunk_map,
            fs.geometry.nodesize,
            fs.root_tree_root,
            default_hint,
        )?;
        Ok(fs)
    }

    /// Open a btrfs filesystem at the subvolume with id `id`, as a Linux
    /// `subvolid=` mount does. Id 5 is the top level. Returns
    /// [`Error::NotFound`] when no live subvolume has that id.
    pub fn open_subvol_id(reader: R, device_size_bytes: u64, id: u64) -> Result<Self> {
        let (mut fs, _) = Self::mount(reader, device_size_bytes)?;
        fs.root = root_tree::subvol_root(
            &mut fs.reader,
            &fs.chunk_map,
            fs.geometry.nodesize,
            fs.root_tree_root,
            id,
        )?;
        Ok(fs)
    }

    /// Open a btrfs filesystem at the subvolume `path` names, as a Linux
    /// `subvol=` mount does: the path is taken from the top level, whatever
    /// the default subvolume is, and `/` is the top level itself. Returns
    /// [`Error::NotASubvolume`] when the path names anything but a
    /// subvolume's root directory.
    pub fn open_subvol_path(reader: R, device_size_bytes: u64, path: Path<'_>) -> Result<Self> {
        let (mut fs, _) = Self::mount(reader, device_size_bytes)?;
        let top = root_tree::subvol_root(
            &mut fs.reader,
            &fs.chunk_map,
            fs.geometry.nodesize,
            fs.root_tree_root,
            FS_TREE_OBJECTID,
        )?;
        let trees = resolve::Trees {
            chunk_map: &fs.chunk_map,
            nodesize: fs.geometry.nodesize,
            root_tree_root: fs.root_tree_root,
        };
        let target = resolve::resolve_path(&mut fs.reader, trees, top, path)?;
        if target.objectid != FIRST_FREE_OBJECTID {
            return Err(Error::NotASubvolume);
        }
        fs.root = target;
        Ok(fs)
    }

    /// Everything an open needs before choosing a subvolume: the superblock,
    /// the full chunk map, and the superblock's default-subvolume hint.
    fn mount(mut reader: R, device_size_bytes: u64) -> Result<(Self, u64)> {
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

        let fs = Self {
            reader,
            chunk_map,
            geometry: file::Geometry {
                nodesize: sb.nodesize,
                sectorsize: sb.sectorsize,
            },
            root_tree_root: sb.root,
            // Replaced by the caller once it has chosen the subvolume.
            root: Inode {
                subvol: FS_TREE_OBJECTID,
                tree_root: 0,
                objectid: FIRST_FREE_OBJECTID,
            },
        };
        Ok((fs, sb.root_dir_objectid))
    }

    /// Id of the subvolume this instance was opened at: the default one for
    /// [`Btrfs::open`], the requested one otherwise. For audit / diagnostic
    /// use.
    pub fn subvolume(&self) -> u64 {
        self.root.subvol
    }

    /// The subvolume [`Btrfs::open`] selected.
    #[deprecated(
        since = "0.4.0",
        note = "use `subvolume`, which also covers `open_subvol_*`"
    )]
    pub fn default_subvol_objectid(&self) -> u64 {
        self.root.subvol
    }

    /// Resolve `path` from the root of the subvolume this instance was
    /// opened at, crossing into nested subvolumes on the way.
    pub fn resolve(&mut self, path: Path<'_>) -> Result<Inode> {
        let trees = resolve::Trees {
            chunk_map: &self.chunk_map,
            nodesize: self.geometry.nodesize,
            root_tree_root: self.root_tree_root,
        };
        resolve::resolve_path(&mut self.reader, trees, self.root, path)
    }

    /// Read inode metadata.
    pub fn metadata(&mut self, inode: &Inode) -> Result<Metadata> {
        file::read_metadata(
            &mut self.reader,
            &self.chunk_map,
            self.geometry.nodesize,
            inode.tree_root,
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
            self.geometry,
            inode.tree_root,
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
            self.geometry,
            inode.tree_root,
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
            self.geometry,
            inode.tree_root,
            inode.objectid,
        )
    }

    /// Iterate the entries of the directory at `path`.
    pub fn read_dir(&mut self, path: Path<'_>) -> Result<Vec<DirEntry>> {
        let inode = self.resolve(path)?;
        let trees = resolve::Trees {
            chunk_map: &self.chunk_map,
            nodesize: self.geometry.nodesize,
            root_tree_root: self.root_tree_root,
        };
        dir::read_dir(&mut self.reader, trees, inode)
    }
}
