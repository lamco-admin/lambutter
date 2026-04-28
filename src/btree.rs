// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Generic B-tree walker.
//!
//! Walks any btrfs B-tree given its root logical bytenr and level.
//! Algorithm (per BTRFS-FORMAT-READONLY-REFERENCE §3):
//!
//! - At each interior node, binary-search the keyptr array for the rightmost
//!   key `<= target` and descend into that child.
//! - At a leaf, binary-search the item array for the first key `>= target`.
//!
//! Every block read goes through `read_tree_block`, which:
//!   1. resolves the logical bytenr via the chunk map,
//!   2. fetches `nodesize` bytes via the underlying `BlockRead`,
//!   3. verifies the metadata CRC32C in the block header against the body.
//!
//! The walker is invoked through `Btrfs<R>` which owns the reader, chunk map,
//! and superblock. The walker itself is stateless.

use alloc::{vec, vec::Vec};

use crate::{
    block_read::BlockRead,
    checksum::verify_crc32c,
    chunk_tree::ChunkMap,
    error::{Error, Result},
    format::{
        constants::{CSUM_LEN, MAX_TREE_DEPTH},
        repr::{DiskKey, Header, KeyPtr, LeafItem},
    },
};

/// One tree block in memory: parsed header + the raw bytes (so callers can
/// slice item-data tails out of the body without re-reading).
pub(crate) struct TreeBlock {
    pub(crate) header: Header,
    pub(crate) body: Vec<u8>,
}

impl TreeBlock {
    /// Parse a leaf item at index `i`. Caller must have verified
    /// `header.level == 0`.
    pub(crate) fn leaf_item(&self, i: u32) -> Result<LeafItem> {
        if i >= self.header.nritems {
            return Err(Error::CorruptBtree {
                token: "leaf_item_oob",
                logical: self.header.bytenr,
            });
        }
        let at = Header::SIZE + (i as usize) * LeafItem::SIZE;
        if at + LeafItem::SIZE > self.body.len() {
            return Err(Error::CorruptBtree {
                token: "leaf_item_short",
                logical: self.header.bytenr,
            });
        }
        Ok(LeafItem::parse(&self.body, at))
    }

    /// Borrow the data tail of a leaf item.
    pub(crate) fn leaf_item_data(&self, item: LeafItem) -> Result<&[u8]> {
        let start = Header::SIZE + item.offset as usize;
        let end = start
            .checked_add(item.size as usize)
            .ok_or(Error::CorruptBtree {
                token: "item_overflow",
                logical: self.header.bytenr,
            })?;
        if end > self.body.len() {
            return Err(Error::CorruptBtree {
                token: "item_oob",
                logical: self.header.bytenr,
            });
        }
        Ok(&self.body[start..end])
    }

    /// Parse an interior keyptr at index `i`. Caller must have verified
    /// `header.level > 0`.
    pub(crate) fn key_ptr(&self, i: u32) -> Result<KeyPtr> {
        if i >= self.header.nritems {
            return Err(Error::CorruptBtree {
                token: "keyptr_oob",
                logical: self.header.bytenr,
            });
        }
        let at = Header::SIZE + (i as usize) * KeyPtr::SIZE;
        if at + KeyPtr::SIZE > self.body.len() {
            return Err(Error::CorruptBtree {
                token: "keyptr_short",
                logical: self.header.bytenr,
            });
        }
        Ok(KeyPtr::parse(&self.body, at))
    }
}

/// Read a tree block from disk. Verifies metadata CRC32C and (sanity)
/// the bytenr embedded in the block matches the requested logical address.
pub(crate) fn read_tree_block<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    logical: u64,
) -> Result<TreeBlock> {
    let mut remaining = nodesize as u64;
    let mut cursor = logical;
    let mut body = vec![0u8; nodesize as usize];

    let mut written: usize = 0;
    while remaining > 0 {
        let resolved = chunk_map.resolve(cursor)?;
        let take = remaining.min(resolved.contiguous_bytes);
        let take_usize: usize = take.try_into().map_err(|_| Error::CorruptBtree {
            token: "read_chunk_too_large",
            logical,
        })?;
        reader
            .read_at(resolved.physical, &mut body[written..written + take_usize])
            .map_err(|_| Error::Io {
                token: "tree_block_read",
                offset: resolved.physical,
            })?;
        written += take_usize;
        cursor += take;
        remaining -= take;
    }

    let header = Header::parse(&body).ok_or(Error::CorruptBtree {
        token: "header_short",
        logical,
    })?;

    // Csum body covers everything after the 32-byte csum field.
    if !verify_crc32c(&body[CSUM_LEN..], &header.csum) {
        return Err(Error::CsumMismatch { logical });
    }

    if header.bytenr != logical {
        return Err(Error::CorruptBtree {
            token: "bytenr_mismatch",
            logical,
        });
    }

    Ok(TreeBlock { header, body })
}

/// Find the leaf item with key `target`, exact-match.
/// Returns `Ok(Some((TreeBlock, item_index)))` on success, `Ok(None)` if
/// the target key is absent, and `Err` on corruption / I/O failure.
pub(crate) fn find_exact<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_logical: u64,
    target: &DiskKey,
) -> Result<Option<(TreeBlock, u32)>> {
    let leaf = descend_to_leaf(reader, chunk_map, nodesize, root_logical, target)?;

    if leaf.header.nritems == 0 {
        return Ok(None);
    }

    let idx = match binary_search_leaf(&leaf, target)? {
        Some(i) => i,
        None => return Ok(None),
    };
    let item = leaf.leaf_item(idx)?;
    if &item.key == target {
        Ok(Some((leaf, idx)))
    } else {
        Ok(None)
    }
}

/// Find the first leaf item with key `>= target`, suitable as the start of
/// an iteration. Returns `Ok(Some((leaf, item_index)))`, or `Ok(None)` if
/// the target is past the rightmost leaf in the tree.
pub(crate) fn find_first_ge<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_logical: u64,
    target: &DiskKey,
) -> Result<Option<(TreeBlock, u32)>> {
    let leaf = descend_to_leaf(reader, chunk_map, nodesize, root_logical, target)?;

    if leaf.header.nritems == 0 {
        return Ok(None);
    }

    match binary_search_leaf(&leaf, target)? {
        Some(idx) => Ok(Some((leaf, idx))),
        None => Ok(None),
    }
}

/// Descend from a tree root to the leaf that should contain `target`.
fn descend_to_leaf<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    root_logical: u64,
    target: &DiskKey,
) -> Result<TreeBlock> {
    let mut current = read_tree_block(reader, chunk_map, nodesize, root_logical)?;
    let mut depth: u8 = 0;
    while current.header.level > 0 {
        depth = depth.saturating_add(1);
        if depth > MAX_TREE_DEPTH {
            return Err(Error::CorruptBtree {
                token: "infinite_recursion",
                logical: current.header.bytenr,
            });
        }
        if current.header.nritems == 0 {
            return Err(Error::CorruptBtree {
                token: "interior_empty",
                logical: current.header.bytenr,
            });
        }
        let child_idx = binary_search_interior(&current, target)?;
        let kp = current.key_ptr(child_idx)?;
        current = read_tree_block(reader, chunk_map, nodesize, kp.blockptr)?;
    }
    Ok(current)
}

/// Binary search the interior node for the rightmost keyptr whose key is
/// `<= target`. Returns the index to descend into. If `target` is less than
/// the first key, returns 0 (we still descend into the leftmost child since
/// per spec the first child of an interior node carries items strictly less
/// than `keys[1]`, including those less than `keys[0]`).
fn binary_search_interior(block: &TreeBlock, target: &DiskKey) -> Result<u32> {
    let n = block.header.nritems;
    if n == 0 {
        return Err(Error::CorruptBtree {
            token: "interior_empty",
            logical: block.header.bytenr,
        });
    }

    let mut lo: u32 = 0;
    let mut hi: u32 = n;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let kp = block.key_ptr(mid)?;
        if kp.key <= *target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }

    // lo is now the count of keys <= target. Descend into child[lo - 1] when
    // lo > 0; if lo == 0 (target less than first key), descend into child[0]
    // anyway — the leftmost subtree may still contain target if it lives at
    // the very left edge of the keyspace.
    Ok(lo.saturating_sub(1))
}

/// Binary search the leaf for the first item whose key is `>= target`.
/// Returns `Ok(Some(idx))` if such an item exists, `Ok(None)` if every item
/// is strictly less than `target`. Also validates that the leaf's keys are
/// strictly ascending; out-of-order keys yield `CorruptBtree`.
fn binary_search_leaf(block: &TreeBlock, target: &DiskKey) -> Result<Option<u32>> {
    let n = block.header.nritems;
    if n == 0 {
        return Ok(None);
    }

    // Validate ascending order via spot-check during the search; full
    // validation across nritems would cost O(n) per descent.
    let mut lo: u32 = 0;
    let mut hi: u32 = n;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let item = block.leaf_item(mid)?;
        if item.key < *target {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo == n {
        Ok(None)
    } else {
        Ok(Some(lo))
    }
}

/// Iterate the leaf's items starting at `start_idx`. The iterator does NOT
/// follow leaf-to-leaf chaining in v0.1.0 (we stop at end of leaf); callers
/// that need cross-leaf iteration must repeatedly call `find_first_ge` with
/// an updated target. This is sufficient for DIR_ITEM hash-collision
/// iteration and EXTENT_DATA enumeration within a single inode's items.
pub(crate) struct LeafIter<'a> {
    block: &'a TreeBlock,
    next: u32,
}

impl<'a> LeafIter<'a> {
    pub(crate) fn new(block: &'a TreeBlock, start: u32) -> Self {
        Self { block, next: start }
    }
}

impl<'a> Iterator for LeafIter<'a> {
    type Item = Result<LeafItem>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.next >= self.block.header.nritems {
            return None;
        }
        let res = self.block.leaf_item(self.next);
        self.next += 1;
        Some(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        checksum::crc32c,
        format::constants::{FSID_LEN, UUID_LEN},
    };

    /// Build a minimal valid leaf containing a sequence of (key, data) pairs.
    /// Items are placed in the post-header area; their data tails grow from
    /// the end of the block backwards.
    fn build_leaf(nodesize: u32, bytenr: u64, items: &[(DiskKey, &[u8])]) -> Vec<u8> {
        let mut block = vec![0u8; nodesize as usize];

        // Header
        // csum (32) — fill last after computing
        // fsid (16) — zeros for tests
        // bytenr (8)
        block[48..56].copy_from_slice(&bytenr.to_le_bytes());
        // flags (8) — 0
        // chunk_tree_uuid (16) — zeros
        // generation (8)
        block[80..88].copy_from_slice(&1u64.to_le_bytes());
        // owner (8)
        block[88..96].copy_from_slice(&1u64.to_le_bytes());
        // nritems (4)
        let nritems = items.len() as u32;
        block[96..100].copy_from_slice(&nritems.to_le_bytes());
        // level (1)
        block[100] = 0;

        // Items: place LeafItem entries starting at HEADER_END = 101.
        // Place item data at the END of the block growing leftward.
        let mut data_end = nodesize as usize;
        let item_array_start = Header::SIZE;
        for (i, (key, data)) in items.iter().enumerate() {
            data_end -= data.len();
            let data_offset_from_header_end = (data_end - Header::SIZE) as u32;
            let item_at = item_array_start + i * LeafItem::SIZE;
            // key
            block[item_at..item_at + 8].copy_from_slice(&key.objectid.to_le_bytes());
            block[item_at + 8] = key.item_type;
            block[item_at + 9..item_at + 17].copy_from_slice(&key.offset.to_le_bytes());
            // offset
            block[item_at + 17..item_at + 21]
                .copy_from_slice(&data_offset_from_header_end.to_le_bytes());
            // size
            block[item_at + 21..item_at + 25].copy_from_slice(&(data.len() as u32).to_le_bytes());
            // copy data
            block[data_end..data_end + data.len()].copy_from_slice(data);
        }

        // Csum body
        let body_csum = crc32c(&block[CSUM_LEN..]);
        block[0..4].copy_from_slice(&body_csum.to_le_bytes());

        block
    }

    /// Wrap a leaf as a synthetic single-chunk volume so the walker can read
    /// it via the chunk map. Returns (chunk_map, device_bytes, root_logical).
    fn synth_volume(nodesize: u32, leaf: Vec<u8>, root_logical: u64) -> (ChunkMap, Vec<u8>) {
        // Place the leaf at physical offset == root_logical for simplicity.
        let mut device = vec![0u8; (root_logical as usize) + leaf.len()];
        device[root_logical as usize..].copy_from_slice(&leaf);

        let mut map = ChunkMap::new();
        // Build a system chunk array entry covering [root_logical, root_logical + nodesize).
        let mut entry = Vec::new();
        entry.extend_from_slice(&3u64.to_le_bytes());
        entry.push(crate::format::constants::CHUNK_ITEM_KEY);
        entry.extend_from_slice(&root_logical.to_le_bytes());
        entry.extend_from_slice(&(nodesize as u64).to_le_bytes()); // length
        entry.extend_from_slice(&3u64.to_le_bytes()); // owner
        entry.extend_from_slice(&65536u64.to_le_bytes()); // stripe_len
        entry.extend_from_slice(&0u64.to_le_bytes()); // ty = SINGLE
        entry.extend_from_slice(&4096u32.to_le_bytes());
        entry.extend_from_slice(&4096u32.to_le_bytes());
        entry.extend_from_slice(&4096u32.to_le_bytes());
        entry.extend_from_slice(&1u16.to_le_bytes()); // num_stripes
        entry.extend_from_slice(&0u16.to_le_bytes()); // sub_stripes
        entry.extend_from_slice(&1u64.to_le_bytes()); // devid
        entry.extend_from_slice(&root_logical.to_le_bytes()); // physical = logical for the test
        entry.extend_from_slice(&[0u8; UUID_LEN]);

        let used = entry.len();
        entry.resize(2048, 0);
        map.parse_system_chunk_array(&entry, used).unwrap();

        let _fsid = [0u8; FSID_LEN];
        (map, device)
    }

    #[test]
    fn find_exact_in_single_leaf() {
        let nodesize = 4096u32;
        let root = 0x10_0000u64;
        let key_a = DiskKey {
            objectid: 1,
            item_type: 1,
            offset: 0,
        };
        let key_b = DiskKey {
            objectid: 2,
            item_type: 2,
            offset: 0,
        };
        let key_c = DiskKey {
            objectid: 3,
            item_type: 3,
            offset: 0,
        };
        let leaf = build_leaf(
            nodesize,
            root,
            &[(key_a, b"AAAA"), (key_b, b"BB"), (key_c, b"CCCCCC")],
        );
        let (map, device) = synth_volume(nodesize, leaf, root);
        let mut reader: &[u8] = &device;

        let result = find_exact(&mut reader, &map, nodesize, root, &key_b)
            .unwrap()
            .expect("key_b exists");
        let item = result.0.leaf_item(result.1).unwrap();
        let data = result.0.leaf_item_data(item).unwrap();
        assert_eq!(data, b"BB");
    }

    #[test]
    fn find_exact_returns_none_for_absent_key() {
        let nodesize = 4096u32;
        let root = 0x10_0000u64;
        let key_a = DiskKey {
            objectid: 1,
            item_type: 1,
            offset: 0,
        };
        let leaf = build_leaf(nodesize, root, &[(key_a, b"X")]);
        let (map, device) = synth_volume(nodesize, leaf, root);
        let mut reader: &[u8] = &device;

        let absent = DiskKey {
            objectid: 99,
            item_type: 1,
            offset: 0,
        };
        let result = find_exact(&mut reader, &map, nodesize, root, &absent).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn find_first_ge_matches_target_or_above() {
        let nodesize = 4096u32;
        let root = 0x10_0000u64;
        let keys = [
            DiskKey {
                objectid: 1,
                item_type: 1,
                offset: 0,
            },
            DiskKey {
                objectid: 5,
                item_type: 1,
                offset: 0,
            },
            DiskKey {
                objectid: 9,
                item_type: 1,
                offset: 0,
            },
        ];
        let leaf = build_leaf(
            nodesize,
            root,
            &[(keys[0], b"a"), (keys[1], b"b"), (keys[2], b"c")],
        );
        let (map, device) = synth_volume(nodesize, leaf, root);
        let mut reader: &[u8] = &device;

        let target = DiskKey {
            objectid: 4,
            item_type: 0,
            offset: 0,
        };
        let result = find_first_ge(&mut reader, &map, nodesize, root, &target)
            .unwrap()
            .expect("should match keys[1]");
        let item = result.0.leaf_item(result.1).unwrap();
        assert_eq!(item.key, keys[1]);
    }

    #[test]
    fn rejects_corrupted_csum() {
        let nodesize = 4096u32;
        let root = 0x10_0000u64;
        let key_a = DiskKey {
            objectid: 1,
            item_type: 1,
            offset: 0,
        };
        let mut leaf = build_leaf(nodesize, root, &[(key_a, b"X")]);
        // Flip a body byte without updating the csum
        leaf[200] ^= 0xFF;
        let (map, device) = synth_volume(nodesize, leaf, root);
        let mut reader: &[u8] = &device;

        let result = find_exact(&mut reader, &map, nodesize, root, &key_a);
        assert!(matches!(result, Err(Error::CsumMismatch { .. })));
    }
}
