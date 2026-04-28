// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Path → inode resolver. Walks an FS tree component-by-component,
//! looking up each name via DIR_ITEM hashing, and returns the inode number
//! at the end of the walk.

use crate::{
    block_read::BlockRead,
    btree::find_exact,
    chunk_tree::ChunkMap,
    error::{Error, Result},
    format::{
        constants::{DIR_ITEM_KEY, FIRST_FREE_OBJECTID},
        repr::{DirEntry, DiskKey},
    },
    path::Path,
    root_tree::name_hash,
};

/// Resolve `path` within the FS tree rooted at `fs_tree_root` to an inode
/// number. The root inode of any FS tree is always
/// `FIRST_FREE_OBJECTID = 256`.
pub(crate) fn resolve_path<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_tree_root: u64,
    path: Path<'_>,
) -> Result<u64> {
    let mut current_inode = FIRST_FREE_OBJECTID;
    for component in path.components() {
        current_inode = lookup_in_dir(
            reader,
            chunk_map,
            nodesize,
            fs_tree_root,
            current_inode,
            component,
        )?;
    }
    Ok(current_inode)
}

/// Look up `name` within `parent_inode`'s directory listing. Returns the
/// child inode number on success. The DIR_ITEM key is
/// `(parent_inode, DIR_ITEM_KEY, name_hash(name))`.
fn lookup_in_dir<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_tree_root: u64,
    parent_inode: u64,
    name: &[u8],
) -> Result<u64> {
    let target = DiskKey {
        objectid: parent_inode,
        item_type: DIR_ITEM_KEY,
        offset: name_hash(name),
    };

    let Some((leaf, idx)) = find_exact(reader, chunk_map, nodesize, fs_tree_root, &target)? else {
        return Err(Error::NotFound);
    };

    // Hash collisions: walk the packed DirEntry records.
    let item = leaf.leaf_item(idx)?;
    let data = leaf.leaf_item_data(item)?;
    let mut p = 0usize;
    while p < data.len() {
        let Some((entry, next)) = DirEntry::parse(data, p) else {
            return Err(Error::CorruptBtree {
                token: "dir_entry_short",
                logical: leaf.header.bytenr,
            });
        };
        if entry.name == name {
            return Ok(entry.location.objectid);
        }
        p = next;
    }
    Err(Error::NotFound)
}
