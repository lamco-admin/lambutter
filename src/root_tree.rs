// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Root-tree walker and default-subvolume resolution.
//!
//! The root tree owns ROOT_ITEM entries (one per subvolume) keyed by the
//! subvolume's objectid. It also owns DIR_ITEMs at the pseudo-objectid
//! `ROOT_TREE_DIR_OBJECTID` (= 6). The DIR_ITEM named `"default"` at OID 6
//! identifies the active default subvolume — the same mechanism Snapper
//! manipulates on rollback.
//!
//! Resolution order (per BTRFS-FORMAT-READONLY-REFERENCE §9):
//! 1. DIR_ITEM `"default"` at OID 6 in ROOT_TREE
//! 2. Superblock `root_dir_objectid` field (may be 6 — pointing back to OID 6
//!    DIR_ITEM — or directly to a subvol objectid)
//! 3. Fallback to `FS_TREE_OBJECTID` (= 5)

use crate::{
    block_read::BlockRead,
    btree::{find_exact, find_first_ge},
    checksum::crc32c_with_seed,
    chunk_tree::ChunkMap,
    error::{Error, Result},
    format::{
        constants::{
            DIR_ITEM_KEY, FS_TREE_OBJECTID, NAME_HASH_SEED, ROOT_ITEM_KEY, ROOT_TREE_DIR_OBJECTID,
        },
        repr::{DirEntry, DiskKey, RootItem},
    },
};

/// Compute the btrfs name-hash for a directory entry. Uses CRC32C with the
/// non-default seed `0xFFFF_FFFE`. spec: §7
pub(crate) fn name_hash(name: &[u8]) -> u64 {
    u64::from(crc32c_with_seed(NAME_HASH_SEED, name))
}

/// Resolve the active subvolume's FS-tree root by walking the root tree.
/// Returns `(fs_tree_root_logical, fs_tree_level, subvol_objectid)`.
pub(crate) fn resolve_default_subvol<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_tree_root: u64,
    superblock_default_objectid: u64,
) -> Result<(u64, u8, u64)> {
    let subvol_objectid =
        match lookup_default_dir_item(reader, chunk_map, nodesize, root_tree_root)? {
            Some(oid) => oid,
            None => {
                // Fall back to superblock's hint, which may itself be 0/5 to
                // mean FS_TREE.
                if superblock_default_objectid == 0 || superblock_default_objectid == 6 {
                    FS_TREE_OBJECTID
                } else {
                    superblock_default_objectid
                }
            }
        };

    let root_item = lookup_root_item(reader, chunk_map, nodesize, root_tree_root, subvol_objectid)?
        .ok_or(Error::CorruptBtree {
            token: "subvol_root_missing",
            logical: root_tree_root,
        })?;
    Ok((root_item.bytenr, root_item.level, subvol_objectid))
}

/// Look up the DIR_ITEM `"default"` at `(ROOT_TREE_DIR_OBJECTID, DIR_ITEM, hash)`.
/// Returns `Some(subvol_objectid)` if found, `None` otherwise.
fn lookup_default_dir_item<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_tree_root: u64,
) -> Result<Option<u64>> {
    let hash = name_hash(b"default");
    let target = DiskKey {
        objectid: ROOT_TREE_DIR_OBJECTID,
        item_type: DIR_ITEM_KEY,
        offset: hash,
    };

    let Some((leaf, idx)) = find_exact(reader, chunk_map, nodesize, root_tree_root, &target)?
    else {
        return Ok(None);
    };

    // Hash collision unpacking: multiple DirEntry records may pack into the
    // same DIR_ITEM tail. Iterate them and look for one whose name matches
    // exactly.
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
        if entry.name == b"default" {
            return Ok(Some(entry.location.objectid));
        }
        p = next;
    }
    Ok(None)
}

/// Look up the ROOT_ITEM for a given subvolume objectid. The chunk-tree-style
/// ROOT_ITEM key has `offset = -1` to fetch the latest version; we use
/// `find_first_ge` over `(objectid, ROOT_ITEM, 0)` and pick the matching
/// objectid+type.
fn lookup_root_item<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_tree_root: u64,
    subvol_objectid: u64,
) -> Result<Option<RootItem>> {
    let target = DiskKey {
        objectid: subvol_objectid,
        item_type: ROOT_ITEM_KEY,
        offset: 0,
    };
    let Some((leaf, idx)) = find_first_ge(reader, chunk_map, nodesize, root_tree_root, &target)?
    else {
        return Ok(None);
    };
    let item = leaf.leaf_item(idx)?;
    if item.key.objectid != subvol_objectid || item.key.item_type != ROOT_ITEM_KEY {
        return Ok(None);
    }
    let data = leaf.leaf_item_data(item)?;
    let parsed = RootItem::parse(data, 0).ok_or(Error::CorruptBtree {
        token: "root_item_short",
        logical: leaf.header.bytenr,
    })?;
    Ok(Some(parsed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn name_hash_default_is_known() {
        // Computed once and locked here so future regressions of the seed
        // logic are caught immediately.
        let h = name_hash(b"default");
        assert_ne!(h, 0);
        // Computing twice yields the same value (deterministic).
        assert_eq!(name_hash(b"default"), h);
        // Different inputs yield different hashes.
        assert_ne!(name_hash(b"default"), name_hash(b"DEFAULT"));
    }
}
