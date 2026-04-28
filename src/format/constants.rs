// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! On-disk-format constants. Names mirror upstream `btrfs_tree.h` so spec
//! audits against the format reference (`~/lamboot-dev/docs/analysis/
//! BTRFS-FORMAT-READONLY-REFERENCE-2026-04-27.md`) are mechanical.

// ---------------------------------------------------------------------------
// Magic + superblock locations
// ---------------------------------------------------------------------------

/// Superblock magic, ASCII `_BHRfS_M` interpreted as a little-endian u64.
/// spec: §4 "Superblock"
pub(crate) const BTRFS_MAGIC: u64 = 0x4D5F_5366_5248_425F;

/// Primary superblock byte offset.
/// spec: §4
pub(crate) const SUPERBLOCK_PRIMARY_OFFSET: u64 = 0x10000;

/// All four canonical superblock byte offsets, in order. The crate consults
/// them in this order and selects the highest-generation valid copy.
/// spec: §4
pub(crate) const SUPERBLOCK_OFFSETS: [u64; 4] = [
    0x0001_0000,           // 64 KiB
    0x0400_0000,           // 64 MiB
    0x0040_0000_0000,      // 256 GiB
    0x0040_0000_0000_0000, // 1 PiB
];

/// On-disk size of the superblock. The first 4096 bytes are the superblock
/// itself; the rest of the 65536-byte region between `SUPERBLOCK_PRIMARY_
/// OFFSET` and the next 64 KiB is reserved.
/// spec: §4
pub(crate) const SUPERBLOCK_SIZE: usize = 4096;

/// FSID byte length.
pub(crate) const FSID_LEN: usize = 16;

/// UUID byte length (chunk-tree UUID, dev UUIDs).
pub(crate) const UUID_LEN: usize = 16;

/// Csum byte length on the superblock and on every tree block.
/// spec: §11
pub(crate) const CSUM_LEN: usize = 32;

// ---------------------------------------------------------------------------
// Reserved object IDs
// ---------------------------------------------------------------------------
// spec: §2 / §9.

/// `ROOT_TREE` root.
pub(crate) const ROOT_TREE_OBJECTID: u64 = 1;

/// `EXTENT_TREE` root. Read-only path skips this tree.
pub(crate) const EXTENT_TREE_OBJECTID: u64 = 2;

/// `CHUNK_TREE` root.
pub(crate) const CHUNK_TREE_OBJECTID: u64 = 3;

/// `DEV_TREE` root. Read-only path skips this tree.
pub(crate) const DEV_TREE_OBJECTID: u64 = 4;

/// `FS_TREE` root (the global / "no-subvol" filesystem tree).
pub(crate) const FS_TREE_OBJECTID: u64 = 5;

/// Pseudo-objectid that owns the `ROOT_TREE`'s top-level directory items.
/// The DIR_ITEM named `"default"` lives here and identifies the active
/// default subvolume — the load-bearing mechanism for Snapper rollback.
/// spec: §9 (default subvolume resolution)
pub(crate) const ROOT_TREE_DIR_OBJECTID: u64 = 6;

/// `CSUM_TREE` root. Read-only path skips this tree in v0.1.0.
pub(crate) const CSUM_TREE_OBJECTID: u64 = 7;

/// `QUOTA_TREE` root. Read-only path skips.
pub(crate) const QUOTA_TREE_OBJECTID: u64 = 8;

/// `UUID_TREE` root. Read-only path skips.
pub(crate) const UUID_TREE_OBJECTID: u64 = 9;

/// `FREE_SPACE_TREE` root. Read-only path explicitly skips.
/// spec: §10
pub(crate) const FREE_SPACE_TREE_OBJECTID: u64 = 10;

/// First valid user-allocated objectid for subvolumes.
pub(crate) const FIRST_FREE_OBJECTID: u64 = 256;

/// `DEV_ITEMS_OBJECTID` — pseudo-objectid that owns DEV_ITEM keys in the
/// chunk tree.
pub(crate) const DEV_ITEMS_OBJECTID: u64 = 1;

// ---------------------------------------------------------------------------
// Item type values (the `type` byte in a `btrfs_disk_key`).
// ---------------------------------------------------------------------------
// spec: §3 (item type table)

pub(crate) const INODE_ITEM_KEY: u8 = 1;
pub(crate) const INODE_REF_KEY: u8 = 12;
pub(crate) const INODE_EXTREF_KEY: u8 = 13;
pub(crate) const XATTR_ITEM_KEY: u8 = 24;
pub(crate) const ORPHAN_ITEM_KEY: u8 = 48;
pub(crate) const DIR_LOG_ITEM_KEY: u8 = 60;
pub(crate) const DIR_LOG_INDEX_KEY: u8 = 72;
pub(crate) const DIR_ITEM_KEY: u8 = 84;
pub(crate) const DIR_INDEX_KEY: u8 = 96;
pub(crate) const EXTENT_DATA_KEY: u8 = 108;
pub(crate) const EXTENT_CSUM_KEY: u8 = 128;
pub(crate) const ROOT_ITEM_KEY: u8 = 132;
pub(crate) const ROOT_BACKREF_KEY: u8 = 144;
pub(crate) const ROOT_REF_KEY: u8 = 156;
pub(crate) const EXTENT_ITEM_KEY: u8 = 168;
pub(crate) const METADATA_ITEM_KEY: u8 = 169;
pub(crate) const TREE_BLOCK_REF_KEY: u8 = 176;
pub(crate) const EXTENT_DATA_REF_KEY: u8 = 178;
pub(crate) const SHARED_BLOCK_REF_KEY: u8 = 182;
pub(crate) const SHARED_DATA_REF_KEY: u8 = 184;
pub(crate) const BLOCK_GROUP_ITEM_KEY: u8 = 192;
pub(crate) const FREE_SPACE_INFO_KEY: u8 = 198;
pub(crate) const FREE_SPACE_EXTENT_KEY: u8 = 199;
pub(crate) const FREE_SPACE_BITMAP_KEY: u8 = 200;
pub(crate) const DEV_EXTENT_KEY: u8 = 204;
pub(crate) const DEV_ITEM_KEY: u8 = 216;
pub(crate) const CHUNK_ITEM_KEY: u8 = 228;
pub(crate) const QGROUP_STATUS_KEY: u8 = 240;
pub(crate) const QGROUP_INFO_KEY: u8 = 242;
pub(crate) const QGROUP_LIMIT_KEY: u8 = 244;
pub(crate) const QGROUP_RELATION_KEY: u8 = 246;
pub(crate) const TEMPORARY_ITEM_KEY: u8 = 248;
pub(crate) const PERSISTENT_ITEM_KEY: u8 = 249;
pub(crate) const DEV_REPLACE_KEY: u8 = 250;
pub(crate) const STRING_ITEM_KEY: u8 = 253;

// ---------------------------------------------------------------------------
// Csum types (BTRFS_CSUM_TYPE_*).
// ---------------------------------------------------------------------------
// spec: §11

pub(crate) const CSUM_TYPE_CRC32C: u16 = 0;
pub(crate) const CSUM_TYPE_XXHASH: u16 = 1;
pub(crate) const CSUM_TYPE_SHA256: u16 = 2;
pub(crate) const CSUM_TYPE_BLAKE2: u16 = 3;

// ---------------------------------------------------------------------------
// Compression types (BTRFS_COMPRESS_*).
// ---------------------------------------------------------------------------
// spec: §6

pub(crate) const COMPRESS_NONE: u8 = 0;
pub(crate) const COMPRESS_ZLIB: u8 = 1;
pub(crate) const COMPRESS_LZO: u8 = 2;
pub(crate) const COMPRESS_ZSTD: u8 = 3;

// ---------------------------------------------------------------------------
// EXTENT_DATA item types (the `type` field inside a file_extent_item).
// ---------------------------------------------------------------------------
// spec: §8

pub(crate) const FILE_EXTENT_INLINE: u8 = 0;
pub(crate) const FILE_EXTENT_REG: u8 = 1;
pub(crate) const FILE_EXTENT_PREALLOC: u8 = 2;

// ---------------------------------------------------------------------------
// Chunk profile / stripe-type bit flags. These are bits within a u64
// `type` field on `btrfs_chunk` items and on `btrfs_block_group_item`s.
// ---------------------------------------------------------------------------
// spec: §5

pub(crate) const BLOCK_GROUP_DATA: u64 = 1 << 0;
pub(crate) const BLOCK_GROUP_SYSTEM: u64 = 1 << 1;
pub(crate) const BLOCK_GROUP_METADATA: u64 = 1 << 2;
pub(crate) const BLOCK_GROUP_RAID0: u64 = 1 << 3;
pub(crate) const BLOCK_GROUP_RAID1: u64 = 1 << 4;
pub(crate) const BLOCK_GROUP_DUP: u64 = 1 << 5;
pub(crate) const BLOCK_GROUP_RAID10: u64 = 1 << 6;
pub(crate) const BLOCK_GROUP_RAID5: u64 = 1 << 7;
pub(crate) const BLOCK_GROUP_RAID6: u64 = 1 << 8;
pub(crate) const BLOCK_GROUP_RAID1C3: u64 = 1 << 9;
pub(crate) const BLOCK_GROUP_RAID1C4: u64 = 1 << 10;

/// Mask covering every chunk-profile bit (excludes DATA/SYSTEM/METADATA).
/// spec: §5
pub(crate) const BLOCK_GROUP_PROFILE_MASK: u64 = BLOCK_GROUP_RAID0
    | BLOCK_GROUP_RAID1
    | BLOCK_GROUP_DUP
    | BLOCK_GROUP_RAID10
    | BLOCK_GROUP_RAID5
    | BLOCK_GROUP_RAID6
    | BLOCK_GROUP_RAID1C3
    | BLOCK_GROUP_RAID1C4;

// ---------------------------------------------------------------------------
// B-tree node geometry constants.
// ---------------------------------------------------------------------------

/// Maximum tree-walker depth before refusing to descend further. Real btrfs
/// trees are shallow (typically 3–5 levels); 16 is a generous safety bound
/// against malformed inputs that try to drive the walker into infinite
/// recursion.
pub(crate) const MAX_TREE_DEPTH: u8 = 16;

/// Minimum legal `nodesize` per the on-disk format.
pub(crate) const MIN_NODE_SIZE: u32 = 4096;

/// Maximum legal `nodesize` per the on-disk format.
pub(crate) const MAX_NODE_SIZE: u32 = 65536;

/// Minimum legal `sectorsize`.
pub(crate) const MIN_SECTOR_SIZE: u32 = 4096;

/// Maximum legal `sectorsize` we accept (btrfs has not deployed > 64K in
/// practice as of 2026-04).
pub(crate) const MAX_SECTOR_SIZE: u32 = 65536;

// ---------------------------------------------------------------------------
// Compression decode upper bound — bounds memory use against decompression-
// bomb inputs. spec §6 calls for a configurable cap; we hard-code 16 MiB
// in v0.1.0. A future feature flag can make this caller-tunable.
// ---------------------------------------------------------------------------

pub(crate) const MAX_DECOMPRESSED_EXTENT_BYTES: usize = 16 * 1024 * 1024;

// ---------------------------------------------------------------------------
// DIR_ITEM hash seed.
// ---------------------------------------------------------------------------

/// CRC32C seed used by btrfs's name-hash. NOT the conventional 0; the spec
/// requires `0xFFFF_FFFE`. Getting this wrong silently fails every lookup.
/// spec: §7
pub(crate) const NAME_HASH_SEED: u32 = 0xFFFF_FFFE;
