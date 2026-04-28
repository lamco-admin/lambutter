// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Inode handle and metadata accessor.

/// An inode within the active subvolume. Currently a thin newtype wrapping
/// the inode objectid; future versions may carry cached metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Inode {
    /// Inode objectid (matches the `objectid` field of the corresponding
    /// `INODE_ITEM` key in the FS tree).
    pub objectid: u64,
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
