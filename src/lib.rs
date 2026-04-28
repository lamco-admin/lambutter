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

        let mut chunk_map = ChunkMap::new();
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
