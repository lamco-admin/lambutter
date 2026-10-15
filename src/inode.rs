// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Inode handle and metadata accessor.

/// A file, directory or symlink: the subvolume that holds it and its inode
/// number there. Inode numbers repeat across subvolumes (every subvolume's
/// root directory is 256), so the number alone doesn't name a file. Get one
/// from [`Btrfs::resolve`](crate::Btrfs::resolve) or a
/// [`DirEntry`](crate::DirEntry).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    pub(crate) subvol: u64,
    /// Logical address of the subvolume tree's root node, looked up once when
    /// the walk entered the subvolume.
    pub(crate) tree_root: u64,
    pub(crate) objectid: u64,
}

impl Inode {
    /// Inode number within its subvolume (the `objectid` of its
    /// `INODE_ITEM`).
    pub fn objectid(&self) -> u64 {
        self.objectid
    }

    /// Id of the subvolume that holds it: 5 for the top level, 256 and up
    /// for the others.
    pub fn subvolume(&self) -> u64 {
        self.subvol
    }
}

/// Metadata accessor — analogous to `std::fs::Metadata` for callers.
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    /// File size in bytes (logical).
    pub size: u64,
    /// Number of bytes occupied on disk.
    pub nbytes: u64,
    /// Hard link count.
    pub nlink: u32,
    /// Mode bits.
    pub mode: u32,
    /// Owning user ID.
    pub uid: u32,
    /// Owning group ID.
    pub gid: u32,
    /// Generation (transaction at which the inode was last modified).
    pub generation: u64,
}

impl Metadata {
    /// True if the mode bits indicate a regular file.
    pub fn is_file(&self) -> bool {
        (self.mode & 0o170000) == 0o100000
    }

    /// True if the mode bits indicate a directory.
    pub fn is_dir(&self) -> bool {
        (self.mode & 0o170000) == 0o040000
    }

    /// True if the mode bits indicate a symlink.
    pub fn is_symlink(&self) -> bool {
        (self.mode & 0o170000) == 0o120000
    }
}
