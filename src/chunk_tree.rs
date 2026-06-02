// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Chunk-tree resolver. Translates a btrfs *logical* bytenr into a *physical*
//! offset on the underlying block device.
//!
//! Architecture (per BTRFS-FORMAT-READONLY-REFERENCE §5):
//!
//! 1. **Bootstrap.** The superblock embeds a packed `sys_chunk_array` —
//!    `(disk_key, chunk_item, stripes...)` tuples covering the chunks that
//!    contain the chunk tree itself. We parse that array first; without it
//!    we cannot read the chunk tree's root block.
//! 2. **Full walk.** With bootstrap chunks resolved, we walk the chunk tree
//!    via the generic B-tree walker, harvesting every CHUNK_ITEM into the
//!    in-memory chunk map.
//! 3. **Resolution.** A logical bytenr is resolved by a sorted lookup over
//!    the chunk map. The crate supports SINGLE/DUP/RAID1/RAID1C3/RAID1C4
//!    profiles; RAID0/RAID10/RAID5/RAID6 are explicitly rejected.

use alloc::vec::Vec;

use crate::{
    error::{Error, Result},
    format::{
        constants::{
            BLOCK_GROUP_DUP, BLOCK_GROUP_PROFILE_MASK, BLOCK_GROUP_RAID0, BLOCK_GROUP_RAID1,
            BLOCK_GROUP_RAID10, BLOCK_GROUP_RAID1C3, BLOCK_GROUP_RAID1C4, BLOCK_GROUP_RAID5,
            BLOCK_GROUP_RAID6, CHUNK_ITEM_KEY,
        },
        repr::{ChunkItem, DiskKey, Stripe},
    },
};

/// One chunk's logical→physical mapping. After resolution, we know that the
/// `length` bytes starting at `logical` correspond to the bytes on devid
/// `stripe.devid` starting at `stripe.offset`.
#[derive(Debug, Clone)]
pub(crate) struct ChunkMapping {
    pub(crate) logical: u64,
    pub(crate) length: u64,
    pub(crate) ty: u64,
    /// Stripe length in bytes. Only used by RAID0/10/5/6 (rejected) for
    /// per-stripe addressing; retained on the mapping so the field-by-field
    /// correspondence with `btrfs_chunk` is preserved.
    #[expect(
        dead_code,
        reason = "RAID0/10/5/6-only field; profiles rejected in v0.1.x"
    )]
    pub(crate) stripe_len: u64,
    /// Number of stripes. Used for mirror profiles' parsing; the read path
    /// always picks the first stripe so this is informational.
    #[expect(
        dead_code,
        reason = "informational; pick_stripe always selects the first"
    )]
    pub(crate) num_stripes: u16,
    /// Sub-stripes (RAID10 sub-stripe count). RAID10 is rejected.
    #[expect(dead_code, reason = "RAID10-only field; profile rejected in v0.1.x")]
    pub(crate) sub_stripes: u16,
    pub(crate) stripes: Vec<Stripe>,
}

impl ChunkMapping {
    fn profile(&self) -> u64 {
        self.ty & BLOCK_GROUP_PROFILE_MASK
    }

    /// Pick a stripe to read from for this chunk. For single-device profiles
    /// this is the only stripe; for mirror profiles (RAID1/RAID1C3/RAID1C4/
    /// DUP) any stripe is equivalent and we pick the first.
    fn pick_stripe(&self) -> Result<&Stripe> {
        match self.profile() {
            0 | BLOCK_GROUP_DUP | BLOCK_GROUP_RAID1 | BLOCK_GROUP_RAID1C3 | BLOCK_GROUP_RAID1C4 => {
            }
            BLOCK_GROUP_RAID0 => return Err(Error::UnsupportedProfile("prof_raid0")),
            BLOCK_GROUP_RAID10 => return Err(Error::UnsupportedProfile("prof_raid10")),
            BLOCK_GROUP_RAID5 => return Err(Error::UnsupportedProfile("prof_raid5")),
            BLOCK_GROUP_RAID6 => return Err(Error::UnsupportedProfile("prof_raid6")),
            _ => return Err(Error::UnsupportedProfile("prof_unknown")),
        }
        self.stripes
            .first()
            .ok_or(Error::UnsupportedProfile("prof_no_stripes"))
    }
}

/// In-memory chunk map. Bootstrap entries are parsed from the system chunk
/// array embedded in the superblock; full entries are populated by walking
/// the chunk tree.
#[derive(Debug, Default)]
pub(crate) struct ChunkMap {
    /// Sorted by `logical`, non-overlapping. Lookup is binary-search.
    mappings: Vec<ChunkMapping>,
}

impl ChunkMap {
    /// Parse a packed system chunk array (the `sys_chunk_array` field of the
    /// superblock). The array layout is a sequence of `(disk_key, chunk_item,
    /// stripes...)` tuples. The chunk_item's `num_stripes` determines how
    /// many stripe entries follow. Only `CHUNK_ITEM_KEY` keys are present in
    /// the array per spec.
    pub(crate) fn parse_system_chunk_array(&mut self, array: &[u8], used: usize) -> Result<()> {
        let mut p = 0usize;
        while p < used {
            if p + DiskKey::SIZE > used {
                return Err(Error::CorruptBtree {
                    token: "sys_chunk_short_key",
                    logical: 0,
                });
            }
            let key = DiskKey::parse(array, p);
            p += DiskKey::SIZE;

            if key.item_type != CHUNK_ITEM_KEY {
                return Err(Error::CorruptBtree {
                    token: "sys_chunk_unexpected_key_type",
                    logical: key.offset,
                });
            }

            let chunk = ChunkItem::parse(array, p).ok_or(Error::CorruptBtree {
                token: "sys_chunk_short_chunk",
                logical: key.offset,
            })?;
            let total = ChunkItem::total_size(chunk.num_stripes);
            if p + total > used {
                return Err(Error::CorruptBtree {
                    token: "sys_chunk_short_stripes",
                    logical: key.offset,
                });
            }

            let mut stripes = Vec::with_capacity(chunk.num_stripes as usize);
            for s in 0..chunk.num_stripes {
                let at = p + ChunkItem::HEADER_SIZE + (s as usize) * Stripe::SIZE;
                stripes.push(Stripe::parse(array, at));
            }

            self.insert(ChunkMapping {
                logical: key.offset,
                length: chunk.length,
                ty: chunk.ty,
                stripe_len: chunk.stripe_len,
                num_stripes: chunk.num_stripes,
                sub_stripes: chunk.sub_stripes,
                stripes,
            })?;

            p += total;
        }
        Ok(())
    }

    /// Insert a mapping. Maintains sorted order by `logical`. Overlapping
    /// regions are rejected with a typed error. Addition is checked so that
    /// a malformed `length` (e.g. `u64::MAX`) cannot wrap past the prev/next
    /// bound check.
    pub(crate) fn insert(&mut self, mapping: ChunkMapping) -> Result<()> {
        let pos = self
            .mappings
            .partition_point(|m| m.logical < mapping.logical);
        if let Some(prev) = pos.checked_sub(1).and_then(|i| self.mappings.get(i)) {
            let prev_end = prev.logical.checked_add(prev.length).ok_or({
                Error::CorruptBtree {
                    token: "chunk_overflow",
                    logical: prev.logical,
                }
            })?;
            if prev_end > mapping.logical {
                return Err(Error::CorruptBtree {
                    token: "chunk_overlap",
                    logical: mapping.logical,
                });
            }
        }
        let new_end = mapping.logical.checked_add(mapping.length).ok_or({
            Error::CorruptBtree {
                token: "chunk_overflow",
                logical: mapping.logical,
            }
        })?;
        if let Some(next) = self.mappings.get(pos) {
            if new_end > next.logical {
                return Err(Error::CorruptBtree {
                    token: "chunk_overlap",
                    logical: mapping.logical,
                });
            }
        }
        self.mappings.insert(pos, mapping);
        Ok(())
    }

    /// Resolve a logical address to a (devid, physical_offset) pair.
    /// Returns the maximum byte length contiguous in the same chunk so the
    /// caller can read efficiently.
    pub(crate) fn resolve(&self, logical: u64) -> Result<Resolved> {
        let pos = self.mappings.partition_point(|m| m.logical <= logical);
        let mapping = pos
            .checked_sub(1)
            .and_then(|i| self.mappings.get(i))
            .ok_or(Error::CorruptBtree {
                token: "chunk_unmapped",
                logical,
            })?;

        let mapping_end = mapping.logical.checked_add(mapping.length).ok_or({
            Error::CorruptBtree {
                token: "chunk_overflow",
                logical: mapping.logical,
            }
        })?;
        if logical >= mapping_end {
            return Err(Error::CorruptBtree {
                token: "chunk_unmapped",
                logical,
            });
        }

        let stripe = mapping.pick_stripe()?;
        let offset_in_chunk = logical - mapping.logical;
        // For SINGLE, DUP, and RAID1/RAID1Cn the stripe covers the whole
        // chunk linearly: physical = stripe.offset + offset_in_chunk.
        // RAID0/RAID10/RAID5/RAID6 are rejected at pick_stripe.
        let physical = stripe
            .offset
            .checked_add(offset_in_chunk)
            .ok_or(Error::CorruptBtree {
                token: "chunk_overflow",
                logical,
            })?;
        let bytes_remaining = mapping.length - offset_in_chunk;

        Ok(Resolved {
            devid: stripe.devid,
            physical,
            contiguous_bytes: bytes_remaining,
        })
    }
}

/// Walk the chunk tree starting at its root and populate `map` with every
/// CHUNK_ITEM entry. The system chunk array must already cover enough
/// chunks to read the chunk tree itself (the bootstrap step).
///
/// Iteration descends to the leftmost leaf, harvests CHUNK_ITEMs, then
/// re-descends to the next leaf via "find first key strictly greater than
/// the last seen key." The walker module's `find_first_ge` covers this when
/// passed a synthetic target.
pub(crate) fn populate_from_chunk_tree<R: crate::block_read::BlockRead>(
    reader: &mut R,
    map: &mut ChunkMap,
    nodesize: u32,
    chunk_root: u64,
) -> Result<()> {
    use crate::format::constants::{CHUNK_ITEM_KEY, CHUNK_TREE_OBJECTID};

    let mut next_target = DiskKey {
        objectid: CHUNK_TREE_OBJECTID,
        item_type: CHUNK_ITEM_KEY,
        offset: 0,
    };

    loop {
        // Take a snapshot of the current map; we use it for resolution while
        // descending. The chunk tree's own blocks must be reachable via the
        // map already; data chunks may not be — we add them here.
        let location =
            crate::btree::find_first_ge(reader, map, nodesize, chunk_root, &next_target)?;
        let Some((leaf, mut idx)) = location else {
            break;
        };
        let nritems = leaf.header.nritems;
        let mut last_key: Option<DiskKey> = None;
        while idx < nritems {
            let item = leaf.leaf_item(idx)?;
            // Only CHUNK_ITEMs are interesting; iteration may surface other
            // key types (DEV_ITEM lives in the chunk tree at CHUNK_TREE_DIR_OBJECTID).
            if item.key.item_type == CHUNK_ITEM_KEY {
                let data = leaf.leaf_item_data(item)?;
                let chunk = ChunkItem::parse(data, 0).ok_or(Error::CorruptBtree {
                    token: "chunk_item_short",
                    logical: leaf.header.bytenr,
                })?;
                let total = ChunkItem::total_size(chunk.num_stripes);
                if data.len() < total {
                    return Err(Error::CorruptBtree {
                        token: "chunk_item_truncated",
                        logical: leaf.header.bytenr,
                    });
                }
                let mut stripes = Vec::with_capacity(chunk.num_stripes as usize);
                for s in 0..chunk.num_stripes {
                    let at = ChunkItem::HEADER_SIZE + (s as usize) * Stripe::SIZE;
                    stripes.push(Stripe::parse(data, at));
                }
                let mapping = ChunkMapping {
                    logical: item.key.offset,
                    length: chunk.length,
                    ty: chunk.ty,
                    stripe_len: chunk.stripe_len,
                    num_stripes: chunk.num_stripes,
                    sub_stripes: chunk.sub_stripes,
                    stripes,
                };
                // Idempotent insert: skip duplicates already present from
                // the system chunk array.
                if !map.contains(item.key.offset) {
                    map.insert(mapping)?;
                }
            }
            last_key = Some(item.key);
            idx += 1;
        }
        // Set up next iteration: target = (last_key + epsilon)
        let Some(last) = last_key else { break };
        next_target = match last.offset.checked_add(1) {
            Some(o) => DiskKey {
                objectid: last.objectid,
                item_type: last.item_type,
                offset: o,
            },
            None => break,
        };
    }
    Ok(())
}

impl ChunkMap {
    pub(crate) fn contains(&self, logical: u64) -> bool {
        self.mappings
            .binary_search_by_key(&logical, |m| m.logical)
            .is_ok()
    }
}

/// The result of resolving a logical bytenr.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Resolved {
    /// Device ID the stripe lives on. v0.1.x supports single-device read
    /// paths only — `devid` is recorded for diagnostic / future multi-device
    /// dispatch but not consulted in the current read path. (Test builds
    /// read it; non-test builds don't.)
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "single-device only in v0.1.x; multi-device dispatch is a v0.2.0+ scope item"
        )
    )]
    pub(crate) devid: u64,
    pub(crate) physical: u64,
    /// Bytes contiguously available at `physical` before crossing into a
    /// different chunk; callers must re-resolve after consuming this many
    /// bytes.
    pub(crate) contiguous_bytes: u64,
}

#[cfg(test)]
mod tests {
    use alloc::vec;

    use super::*;
    use crate::format::constants::UUID_LEN;

    fn build_sys_chunk_entry(
        logical: u64,
        length: u64,
        num_stripes: u16,
        stripes: &[(u64, u64)],
    ) -> Vec<u8> {
        let mut out = Vec::new();
        // disk key (objectid, type, offset)
        out.extend_from_slice(&3u64.to_le_bytes()); // CHUNK_TREE_OBJECTID
        out.push(CHUNK_ITEM_KEY);
        out.extend_from_slice(&logical.to_le_bytes());

        // chunk_item header
        out.extend_from_slice(&length.to_le_bytes());
        out.extend_from_slice(&3u64.to_le_bytes()); // owner = CHUNK_TREE_OBJECTID
        out.extend_from_slice(&65536u64.to_le_bytes()); // stripe_len
        out.extend_from_slice(&0u64.to_le_bytes()); // ty = SINGLE (no profile bit)
        out.extend_from_slice(&4096u32.to_le_bytes()); // io_align
        out.extend_from_slice(&4096u32.to_le_bytes()); // io_width
        out.extend_from_slice(&4096u32.to_le_bytes()); // sector_size
        out.extend_from_slice(&num_stripes.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // sub_stripes

        // stripes
        for (devid, offset) in stripes {
            out.extend_from_slice(&devid.to_le_bytes());
            out.extend_from_slice(&offset.to_le_bytes());
            out.extend_from_slice(&[0u8; UUID_LEN]);
        }

        out
    }

    #[test]
    fn parse_sys_chunk_array_single_chunk() {
        let entry = build_sys_chunk_entry(0x0010_0000, 0x10_0000, 1, &[(1, 0x0010_0000)]);
        let mut map = ChunkMap::default();
        let mut padded = entry.clone();
        padded.resize(2048, 0);
        map.parse_system_chunk_array(&padded, entry.len()).unwrap();
        assert_eq!(map.mappings.len(), 1);
        let r = map.resolve(0x0010_1234).unwrap();
        assert_eq!(r.devid, 1);
        assert_eq!(r.physical, 0x0010_1234);
    }

    #[test]
    fn resolve_outside_any_chunk_errors() {
        let entry = build_sys_chunk_entry(0x0010_0000, 0x10_0000, 1, &[(1, 0x0010_0000)]);
        let mut map = ChunkMap::default();
        let mut padded = entry.clone();
        padded.resize(2048, 0);
        map.parse_system_chunk_array(&padded, entry.len()).unwrap();

        // Below first chunk
        assert!(matches!(
            map.resolve(0),
            Err(Error::CorruptBtree {
                token: "chunk_unmapped",
                ..
            })
        ));

        // Past end of last chunk
        assert!(matches!(
            map.resolve(0x0020_0000),
            Err(Error::CorruptBtree {
                token: "chunk_unmapped",
                ..
            })
        ));
    }

    #[test]
    fn insert_rejects_length_overflow() {
        // A chunk with length = u64::MAX - logical + 1 would wrap on
        // unchecked addition; the insert() guard must surface a typed
        // chunk_overflow error instead of accepting the mapping.
        let mut map = ChunkMap::default();
        let bad = ChunkMapping {
            logical: 1,
            length: u64::MAX,
            ty: 0,
            stripe_len: 65536,
            num_stripes: 1,
            sub_stripes: 0,
            stripes: vec![Stripe {
                devid: 1,
                offset: 0,
                dev_uuid: [0; 16],
            }],
        };
        assert!(matches!(
            map.insert(bad),
            Err(Error::CorruptBtree {
                token: "chunk_overflow",
                ..
            })
        ));
    }

    #[test]
    fn resolve_rejects_mapping_overflow() {
        // Build a mapping whose logical + length would wrap; resolve()
        // must surface chunk_overflow rather than incorrectly reporting
        // the byte as in-range.
        let mut map = ChunkMap::default();
        // Use insert direct to bypass the insert overflow guard so we
        // can land a malformed mapping in the map for resolve() to see.
        map.mappings.push(ChunkMapping {
            logical: u64::MAX - 100,
            length: 1000, // wraps past u64::MAX
            ty: 0,
            stripe_len: 65536,
            num_stripes: 1,
            sub_stripes: 0,
            stripes: vec![Stripe {
                devid: 1,
                offset: 0,
                dev_uuid: [0; 16],
            }],
        });
        assert!(matches!(
            map.resolve(u64::MAX - 50),
            Err(Error::CorruptBtree {
                token: "chunk_overflow",
                ..
            })
        ));
    }

    #[test]
    fn raid5_chunk_is_rejected() {
        // Manually build a RAID5 chunk entry.
        let mut out = Vec::new();
        out.extend_from_slice(&3u64.to_le_bytes());
        out.push(CHUNK_ITEM_KEY);
        out.extend_from_slice(&0x0010_0000u64.to_le_bytes());
        out.extend_from_slice(&0x10_0000u64.to_le_bytes());
        out.extend_from_slice(&3u64.to_le_bytes());
        out.extend_from_slice(&65536u64.to_le_bytes());
        out.extend_from_slice(&BLOCK_GROUP_RAID5.to_le_bytes()); // ty
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend_from_slice(&4096u32.to_le_bytes());
        out.extend_from_slice(&3u16.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        for d in 1..=3u64 {
            out.extend_from_slice(&d.to_le_bytes());
            out.extend_from_slice(&(d * 0x0010_0000).to_le_bytes());
            out.extend_from_slice(&[0u8; UUID_LEN]);
        }

        let mut map = ChunkMap::default();
        let used = out.len();
        out.resize(2048, 0);
        map.parse_system_chunk_array(&out, used).unwrap();

        assert!(matches!(
            map.resolve(0x0010_1000),
            Err(Error::UnsupportedProfile("prof_raid5"))
        ));
    }
}
