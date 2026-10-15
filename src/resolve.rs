// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Path → inode resolver. Walks FS trees component-by-component, looking up
//! each name via DIR_ITEM hashing, and crosses into a subvolume's own tree
//! wherever an entry names one, as a Linux mount of the parent shows it.

use crate::{
    block_read::BlockRead,
    btree::find_exact,
    chunk_tree::ChunkMap,
    error::{Error, Result},
    format::{
        constants::{
            DIR_ITEM_KEY, EMPTY_SUBVOL_DIR_OBJECTID, FT_ENCRYPTED, INODE_ITEM_KEY, ROOT_ITEM_KEY,
        },
        repr::{DirEntry, DiskKey},
    },
    inode::Inode,
    path::Path,
    root_tree::{name_hash, root_ref_links, subvol_root, SubvolLink},
};

/// The trees a walk reads: the root tree, for crossing into subvolumes, and
/// the node size of both.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Trees<'a> {
    pub(crate) chunk_map: &'a ChunkMap,
    pub(crate) nodesize: u32,
    pub(crate) root_tree_root: u64,
}

/// Resolve `path` component by component from the directory `start`.
pub(crate) fn resolve_path<R: BlockRead>(
    reader: &mut R,
    trees: Trees<'_>,
    start: Inode,
    path: Path<'_>,
) -> Result<Inode> {
    let mut current = start;
    for component in path.components() {
        current = lookup_in_dir(reader, trees, current, component)?;
    }
    Ok(current)
}

/// Look up `name` within directory `dir`. The DIR_ITEM key is
/// `(dir, DIR_ITEM_KEY, name_hash(name))`.
fn lookup_in_dir<R: BlockRead>(
    reader: &mut R,
    trees: Trees<'_>,
    dir: Inode,
    name: &[u8],
) -> Result<Inode> {
    let target = DiskKey {
        objectid: dir.objectid,
        item_type: DIR_ITEM_KEY,
        offset: name_hash(name),
    };

    let Some((leaf, idx)) = find_exact(
        reader,
        trees.chunk_map,
        trees.nodesize,
        dir.tree_root,
        &target,
    )?
    else {
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
        if entry.ty & FT_ENCRYPTED != 0 {
            return Err(Error::UnsupportedFeature("feat_encryption"));
        }
        if entry.name == name {
            return entry_target(reader, trees, dir, name, entry.location, leaf.header.bytenr);
        }
        p = next;
    }
    Err(Error::NotFound)
}

/// The inode a directory entry of `dir` names. An entry whose location is a
/// `ROOT_ITEM` names a subvolume: when the root tree links that subvolume
/// here, the entry is the subvolume's root directory; when it doesn't, Linux
/// shows an empty directory, and so does this.
pub(crate) fn entry_target<R: BlockRead>(
    reader: &mut R,
    trees: Trees<'_>,
    dir: Inode,
    name: &[u8],
    location: DiskKey,
    leaf_bytenr: u64,
) -> Result<Inode> {
    match location.item_type {
        INODE_ITEM_KEY => Ok(Inode {
            objectid: location.objectid,
            ..dir
        }),
        ROOT_ITEM_KEY => {
            let link = SubvolLink {
                parent: dir.subvol,
                dirid: dir.objectid,
                name,
                child: location.objectid,
            };
            if root_ref_links(
                reader,
                trees.chunk_map,
                trees.nodesize,
                trees.root_tree_root,
                link,
            )? {
                subvol_root(
                    reader,
                    trees.chunk_map,
                    trees.nodesize,
                    trees.root_tree_root,
                    location.objectid,
                )
            } else {
                Ok(Inode {
                    objectid: EMPTY_SUBVOL_DIR_OBJECTID,
                    ..dir
                })
            }
        }
        _ => Err(Error::CorruptBtree {
            token: "dir_entry_location",
            logical: leaf_bytenr,
        }),
    }
}
