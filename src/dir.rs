// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Directory iteration. Walks DIR_INDEX items for the inode (which are
//! stable-ordered by `(inode, DIR_INDEX, sequence)`, mirroring the order
//! files were added), yielding one `DirEntry` per call.

use alloc::vec::Vec;

use crate::{
    block_read::BlockRead,
    btree::find_first_ge,
    error::{Error, Result},
    format::{
        constants::DIR_INDEX_KEY,
        repr::{DirEntry as DiskDirEntry, DiskKey},
    },
    inode::Inode,
    resolve::{entry_target, Trees},
};

/// One directory entry, owned (its name is copied so the iterator doesn't
/// hold a borrow on the underlying tree block).
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DirEntry {
    /// File / subdirectory name as stored on disk (raw bytes).
    pub name: Vec<u8>,
    /// Inode number as the directory stores it, which is also what Linux
    /// `readdir` reports. For a subvolume entry it's the subvolume's id.
    pub inode_number: u64,
    /// Filesystem-style "type" byte (kernel uses these, not lambutter; the
    /// canonical kind is determined by stat'ing the child inode).
    pub kind_byte: u8,
    /// The entry itself, for [`Btrfs::metadata`](crate::Btrfs::metadata) and
    /// the read methods. A subvolume entry is that subvolume's root
    /// directory.
    pub inode: Inode,
}

/// Iterate the directory's children. Crossing a leaf boundary re-descends
/// from the FS-tree root with the next key after the last seen one; the
/// per-leaf cost is O(depth log n), negligible for /boot trees (depth 1-3,
/// ~200-300 items per 16 KiB leaf).
pub(crate) fn read_dir<R: BlockRead>(
    reader: &mut R,
    trees: Trees<'_>,
    dir: Inode,
) -> Result<Vec<DirEntry>> {
    let mut out = Vec::new();
    let mut next_target = DiskKey {
        objectid: dir.objectid,
        item_type: DIR_INDEX_KEY,
        offset: 0,
    };

    loop {
        let location = find_first_ge(
            reader,
            trees.chunk_map,
            trees.nodesize,
            dir.tree_root,
            &next_target,
        )?;
        let Some((leaf, mut idx)) = location else {
            break;
        };
        let nritems = leaf.header.nritems;
        let mut last_seen: Option<DiskKey> = None;
        while idx < nritems {
            let item = leaf.leaf_item(idx)?;
            if item.key.objectid != dir.objectid || item.key.item_type != DIR_INDEX_KEY {
                return Ok(out);
            }
            let data = leaf.leaf_item_data(item)?;
            let (entry, _) = DiskDirEntry::parse(data, 0).ok_or(Error::CorruptBtree {
                token: "dir_index_short",
                logical: leaf.header.bytenr,
            })?;
            let inode = entry_target(
                reader,
                trees,
                dir,
                entry.name,
                entry.location,
                leaf.header.bytenr,
            )?;
            out.push(DirEntry {
                name: entry.name.to_vec(),
                inode_number: entry.location.objectid,
                kind_byte: entry.ty,
                inode,
            });
            last_seen = Some(item.key);
            idx += 1;
        }
        let Some(last) = last_seen else { break };
        next_target = match last.offset.checked_add(1) {
            Some(o) => DiskKey {
                objectid: last.objectid,
                item_type: last.item_type,
                offset: o,
            },
            None => break,
        };
    }

    Ok(out)
}
