// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! On-disk struct **byte-offset constants and parser helpers**.
//!
//! Lambutter does not use `#[repr(C)]` zerocopy struct overlays. Reasons:
//! - Btrfs on-disk packing relies on byte ordering and alignment that Rust
//!   `repr(C)` cannot express in a portable, no-padding way.
//! - Zerocopy crates would couple us to a third-party API surface.
//! - Explicit offset parsing is more legible against the spec — every field
//!   read sites the offset and width, and the format reference doc cites
//!   them in the same form.
//!
//! Each struct defined here exists as a pair: a `pub(crate) const`-ant
//! offset table, and a `parse(buf: &[u8]) -> Self` function that reads the
//! fields. Mutation is not supported — every type is constructed by
//! parsing a slice, and there is no `to_bytes`.

use crate::{
    format::constants::{CSUM_LEN, FSID_LEN, UUID_LEN},
    util::{read_le_u16, read_le_u32, read_le_u64, read_le_u8},
};

// ---------------------------------------------------------------------------
// Superblock layout
// ---------------------------------------------------------------------------
// spec: BTRFS-FORMAT-READONLY-REFERENCE §4

mod sb {
    use super::UUID_LEN;

    pub(super) const OFFSET_CSUM: usize = 0;
    pub(super) const OFFSET_FSID: usize = 32; // CSUM_LEN
    pub(super) const OFFSET_BYTENR: usize = 48;
    pub(super) const OFFSET_FLAGS: usize = 56;
    pub(super) const OFFSET_MAGIC: usize = 64;
    pub(super) const OFFSET_GENERATION: usize = 72;
    pub(super) const OFFSET_ROOT: usize = 80;
    pub(super) const OFFSET_CHUNK_ROOT: usize = 88;
    pub(super) const OFFSET_LOG_ROOT: usize = 96;
    pub(super) const OFFSET_LOG_ROOT_TRANSID: usize = 104;
    pub(super) const OFFSET_TOTAL_BYTES: usize = 112;
    pub(super) const OFFSET_BYTES_USED: usize = 120;
    pub(super) const OFFSET_ROOT_DIR_OBJECTID: usize = 128;
    pub(super) const OFFSET_NUM_DEVICES: usize = 136;
    pub(super) const OFFSET_SECTORSIZE: usize = 144;
    pub(super) const OFFSET_NODESIZE: usize = 148;
    pub(super) const OFFSET_LEAFSIZE: usize = 152;
    pub(super) const OFFSET_STRIPESIZE: usize = 156;
    pub(super) const OFFSET_SYS_CHUNK_ARRAY_SIZE: usize = 160;
    pub(super) const OFFSET_CHUNK_ROOT_GENERATION: usize = 164;
    pub(super) const OFFSET_COMPAT_FLAGS: usize = 172;
    pub(super) const OFFSET_COMPAT_RO_FLAGS: usize = 180;
    pub(super) const OFFSET_INCOMPAT_FLAGS: usize = 188;
    pub(super) const OFFSET_CSUM_TYPE: usize = 196;
    pub(super) const OFFSET_ROOT_LEVEL: usize = 198;
    pub(super) const OFFSET_CHUNK_ROOT_LEVEL: usize = 199;
    pub(super) const OFFSET_LOG_ROOT_LEVEL: usize = 200;
    pub(super) const OFFSET_DEV_ITEM: usize = 201;
    pub(super) const SIZE_DEV_ITEM: usize = 98;
    // label, cache_generation, uuid_tree_generation follow
    pub(super) const OFFSET_LABEL: usize = OFFSET_DEV_ITEM + SIZE_DEV_ITEM; // 299
    pub(super) const SIZE_LABEL: usize = 256;
    pub(super) const OFFSET_CACHE_GENERATION: usize = OFFSET_LABEL + SIZE_LABEL; // 555
    pub(super) const OFFSET_UUID_TREE_GENERATION: usize = OFFSET_CACHE_GENERATION + 8; // 563
    pub(super) const OFFSET_METADATA_UUID: usize = OFFSET_UUID_TREE_GENERATION + 8; // 571
    pub(super) const OFFSET_NR_GLOBAL_ROOTS: usize = OFFSET_METADATA_UUID + UUID_LEN; // 587
                                                                                      // 27 reserved u64 follow (216 bytes), then sys_chunk_array (2048),
                                                                                      // then super_roots, then last 565-byte reserved area.
    pub(super) const OFFSET_SYS_CHUNK_ARRAY: usize = 811; // per upstream layout
    pub(super) const SIZE_SYS_CHUNK_ARRAY: usize = 2048;
}

/// Parsed superblock. Field naming mirrors `btrfs_super_block` upstream.
#[derive(Debug, Clone)]
pub(crate) struct Superblock {
    pub(crate) csum: [u8; CSUM_LEN],
    pub(crate) fsid: [u8; FSID_LEN],
    pub(crate) bytenr: u64,
    pub(crate) flags: u64,
    pub(crate) magic: u64,
    pub(crate) generation: u64,
    pub(crate) root: u64,
    pub(crate) chunk_root: u64,
    pub(crate) log_root: u64,
    pub(crate) total_bytes: u64,
    pub(crate) bytes_used: u64,
    pub(crate) root_dir_objectid: u64,
    pub(crate) num_devices: u64,
    pub(crate) sectorsize: u32,
    pub(crate) nodesize: u32,
    pub(crate) leafsize: u32,
    pub(crate) stripesize: u32,
    pub(crate) sys_chunk_array_size: u32,
    pub(crate) chunk_root_generation: u64,
    pub(crate) compat_flags: u64,
    pub(crate) compat_ro_flags: u64,
    pub(crate) incompat_flags: u64,
    pub(crate) csum_type: u16,
    pub(crate) root_level: u8,
    pub(crate) chunk_root_level: u8,
    pub(crate) log_root_level: u8,
    pub(crate) sys_chunk_array: [u8; sb::SIZE_SYS_CHUNK_ARRAY],
}

impl Superblock {
    /// Parse the on-disk superblock from a 4096-byte buffer. Caller is
    /// responsible for first slicing the read region down to the canonical
    /// 4 KiB starting at the SB offset on disk.
    pub(crate) fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < crate::format::constants::SUPERBLOCK_SIZE {
            return None;
        }

        let mut csum = [0u8; CSUM_LEN];
        csum.copy_from_slice(&buf[sb::OFFSET_CSUM..sb::OFFSET_CSUM + CSUM_LEN]);

        let mut fsid = [0u8; FSID_LEN];
        fsid.copy_from_slice(&buf[sb::OFFSET_FSID..sb::OFFSET_FSID + FSID_LEN]);

        let mut sys_chunk_array = [0u8; sb::SIZE_SYS_CHUNK_ARRAY];
        sys_chunk_array.copy_from_slice(
            &buf[sb::OFFSET_SYS_CHUNK_ARRAY..sb::OFFSET_SYS_CHUNK_ARRAY + sb::SIZE_SYS_CHUNK_ARRAY],
        );

        Some(Self {
            csum,
            fsid,
            bytenr: read_le_u64(buf, sb::OFFSET_BYTENR),
            flags: read_le_u64(buf, sb::OFFSET_FLAGS),
            magic: read_le_u64(buf, sb::OFFSET_MAGIC),
            generation: read_le_u64(buf, sb::OFFSET_GENERATION),
            root: read_le_u64(buf, sb::OFFSET_ROOT),
            chunk_root: read_le_u64(buf, sb::OFFSET_CHUNK_ROOT),
            log_root: read_le_u64(buf, sb::OFFSET_LOG_ROOT),
            total_bytes: read_le_u64(buf, sb::OFFSET_TOTAL_BYTES),
            bytes_used: read_le_u64(buf, sb::OFFSET_BYTES_USED),
            root_dir_objectid: read_le_u64(buf, sb::OFFSET_ROOT_DIR_OBJECTID),
            num_devices: read_le_u64(buf, sb::OFFSET_NUM_DEVICES),
            sectorsize: read_le_u32(buf, sb::OFFSET_SECTORSIZE),
            nodesize: read_le_u32(buf, sb::OFFSET_NODESIZE),
            leafsize: read_le_u32(buf, sb::OFFSET_LEAFSIZE),
            stripesize: read_le_u32(buf, sb::OFFSET_STRIPESIZE),
            sys_chunk_array_size: read_le_u32(buf, sb::OFFSET_SYS_CHUNK_ARRAY_SIZE),
            chunk_root_generation: read_le_u64(buf, sb::OFFSET_CHUNK_ROOT_GENERATION),
            compat_flags: read_le_u64(buf, sb::OFFSET_COMPAT_FLAGS),
            compat_ro_flags: read_le_u64(buf, sb::OFFSET_COMPAT_RO_FLAGS),
            incompat_flags: read_le_u64(buf, sb::OFFSET_INCOMPAT_FLAGS),
            csum_type: read_le_u16(buf, sb::OFFSET_CSUM_TYPE),
            root_level: read_le_u8(buf, sb::OFFSET_ROOT_LEVEL),
            chunk_root_level: read_le_u8(buf, sb::OFFSET_CHUNK_ROOT_LEVEL),
            log_root_level: read_le_u8(buf, sb::OFFSET_LOG_ROOT_LEVEL),
            sys_chunk_array,
        })
    }

    /// Body slice covered by the superblock CRC32C: bytes 0x20..SUPERBLOCK_SIZE
    /// (everything past the 32-byte csum field). Caller wraps the `&[u8]`
    /// they used to parse the superblock and passes it back here.
    /// spec: §11
    pub(crate) const fn body_offset_for_csum() -> usize {
        CSUM_LEN
    }
}

// ---------------------------------------------------------------------------
// Disk key (sort key for B-tree items + interior keys)
// ---------------------------------------------------------------------------
// spec: §3

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct DiskKey {
    pub(crate) objectid: u64,
    pub(crate) item_type: u8,
    pub(crate) offset: u64,
}

impl DiskKey {
    pub(crate) const SIZE: usize = 17; // 8 + 1 + 8

    pub(crate) fn parse(buf: &[u8], at: usize) -> Self {
        Self {
            objectid: read_le_u64(buf, at),
            item_type: read_le_u8(buf, at + 8),
            offset: read_le_u64(buf, at + 9),
        }
    }

    /// Strict ordering by (objectid, type, offset) per the spec. Used by the
    /// B-tree walker to locate items.
    pub(crate) fn cmp_tuple(&self) -> (u64, u8, u64) {
        (self.objectid, self.item_type, self.offset)
    }
}

impl PartialOrd for DiskKey {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for DiskKey {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        self.cmp_tuple().cmp(&other.cmp_tuple())
    }
}

// ---------------------------------------------------------------------------
// Tree-block header (common to interior nodes and leaves)
// ---------------------------------------------------------------------------
// spec: §3 (tree block header)

#[derive(Debug, Clone)]
pub(crate) struct Header {
    pub(crate) csum: [u8; CSUM_LEN],
    pub(crate) fsid: [u8; FSID_LEN],
    pub(crate) bytenr: u64,
    pub(crate) flags: u64,
    pub(crate) chunk_tree_uuid: [u8; UUID_LEN],
    pub(crate) generation: u64,
    pub(crate) owner: u64,
    pub(crate) nritems: u32,
    pub(crate) level: u8,
}

mod hdr {
    use super::{CSUM_LEN, FSID_LEN, UUID_LEN};

    pub(super) const OFFSET_CSUM: usize = 0;
    pub(super) const OFFSET_FSID: usize = OFFSET_CSUM + CSUM_LEN; // 32
    pub(super) const OFFSET_BYTENR: usize = OFFSET_FSID + FSID_LEN; // 48
    pub(super) const OFFSET_FLAGS: usize = OFFSET_BYTENR + 8; // 56
    pub(super) const OFFSET_CHUNK_UUID: usize = OFFSET_FLAGS + 8; // 64
    pub(super) const OFFSET_GENERATION: usize = OFFSET_CHUNK_UUID + UUID_LEN; // 80
    pub(super) const OFFSET_OWNER: usize = OFFSET_GENERATION + 8; // 88
    pub(super) const OFFSET_NRITEMS: usize = OFFSET_OWNER + 8; // 96
    pub(super) const OFFSET_LEVEL: usize = OFFSET_NRITEMS + 4; // 100
    pub(super) const SIZE: usize = OFFSET_LEVEL + 1; // 101
    /// One byte of padding for alignment makes the spec'd header 101 bytes
    /// but real on-disk layout is aligned. Walkers index the items area
    /// starting at `HEADER_END` = 101.
    pub(super) const HEADER_END: usize = 101;
}

impl Header {
    pub(crate) const SIZE: usize = hdr::HEADER_END;

    pub(crate) fn parse(buf: &[u8]) -> Option<Self> {
        if buf.len() < Self::SIZE {
            return None;
        }
        let mut csum = [0u8; CSUM_LEN];
        csum.copy_from_slice(&buf[hdr::OFFSET_CSUM..hdr::OFFSET_CSUM + CSUM_LEN]);
        let mut fsid = [0u8; FSID_LEN];
        fsid.copy_from_slice(&buf[hdr::OFFSET_FSID..hdr::OFFSET_FSID + FSID_LEN]);
        let mut chunk_tree_uuid = [0u8; UUID_LEN];
        chunk_tree_uuid
            .copy_from_slice(&buf[hdr::OFFSET_CHUNK_UUID..hdr::OFFSET_CHUNK_UUID + UUID_LEN]);
        Some(Self {
            csum,
            fsid,
            bytenr: read_le_u64(buf, hdr::OFFSET_BYTENR),
            flags: read_le_u64(buf, hdr::OFFSET_FLAGS),
            chunk_tree_uuid,
            generation: read_le_u64(buf, hdr::OFFSET_GENERATION),
            owner: read_le_u64(buf, hdr::OFFSET_OWNER),
            nritems: read_le_u32(buf, hdr::OFFSET_NRITEMS),
            level: read_le_u8(buf, hdr::OFFSET_LEVEL),
        })
    }
}

// ---------------------------------------------------------------------------
// Leaf item entry (key + (offset, size) pointing into the node body)
// ---------------------------------------------------------------------------
// spec: §3 (leaf node)

#[derive(Debug, Clone, Copy)]
pub(crate) struct LeafItem {
    pub(crate) key: DiskKey,
    pub(crate) offset: u32,
    pub(crate) size: u32,
}

impl LeafItem {
    pub(crate) const SIZE: usize = DiskKey::SIZE + 4 + 4;

    pub(crate) fn parse(buf: &[u8], at: usize) -> Self {
        let key = DiskKey::parse(buf, at);
        let offset = read_le_u32(buf, at + DiskKey::SIZE);
        let size = read_le_u32(buf, at + DiskKey::SIZE + 4);
        Self { key, offset, size }
    }
}

// ---------------------------------------------------------------------------
// Key pointer (interior node entry: key + child bytenr + generation)
// ---------------------------------------------------------------------------
// spec: §3 (interior node)

#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyPtr {
    pub(crate) key: DiskKey,
    pub(crate) blockptr: u64,
    pub(crate) generation: u64,
}

impl KeyPtr {
    pub(crate) const SIZE: usize = DiskKey::SIZE + 8 + 8;

    pub(crate) fn parse(buf: &[u8], at: usize) -> Self {
        let key = DiskKey::parse(buf, at);
        let blockptr = read_le_u64(buf, at + DiskKey::SIZE);
        let generation = read_le_u64(buf, at + DiskKey::SIZE + 8);
        Self {
            key,
            blockptr,
            generation,
        }
    }
}

// ---------------------------------------------------------------------------
// Chunk item + stripe entry (in CHUNK_TREE; also packed in sys_chunk_array)
// ---------------------------------------------------------------------------
// spec: §5

#[derive(Debug, Clone)]
pub(crate) struct ChunkItem {
    pub(crate) length: u64,
    pub(crate) owner: u64,
    pub(crate) stripe_len: u64,
    pub(crate) ty: u64, // BLOCK_GROUP_* bit field
    pub(crate) io_align: u32,
    pub(crate) io_width: u32,
    pub(crate) sector_size: u32,
    pub(crate) num_stripes: u16,
    pub(crate) sub_stripes: u16,
    /// Stripe entries follow the fixed header; parsed separately via
    /// `Stripe::parse_array`.
    pub(crate) stripes_offset: usize,
}

impl ChunkItem {
    /// Fixed-size header before the stripe array.
    pub(crate) const HEADER_SIZE: usize = 48;

    pub(crate) fn parse(buf: &[u8], at: usize) -> Option<Self> {
        if buf.len() < at + Self::HEADER_SIZE {
            return None;
        }
        Some(Self {
            length: read_le_u64(buf, at),
            owner: read_le_u64(buf, at + 8),
            stripe_len: read_le_u64(buf, at + 16),
            ty: read_le_u64(buf, at + 24),
            io_align: read_le_u32(buf, at + 32),
            io_width: read_le_u32(buf, at + 36),
            sector_size: read_le_u32(buf, at + 40),
            num_stripes: read_le_u16(buf, at + 44),
            sub_stripes: read_le_u16(buf, at + 46),
            stripes_offset: at + Self::HEADER_SIZE,
        })
    }

    /// Total on-disk size of the chunk item including the stripe array.
    pub(crate) fn total_size(num_stripes: u16) -> usize {
        Self::HEADER_SIZE + Stripe::SIZE * num_stripes as usize
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct Stripe {
    pub(crate) devid: u64,
    pub(crate) offset: u64,
    pub(crate) dev_uuid: [u8; UUID_LEN],
}

impl Stripe {
    pub(crate) const SIZE: usize = 8 + 8 + UUID_LEN;

    pub(crate) fn parse(buf: &[u8], at: usize) -> Self {
        let devid = read_le_u64(buf, at);
        let offset = read_le_u64(buf, at + 8);
        let mut dev_uuid = [0u8; UUID_LEN];
        dev_uuid.copy_from_slice(&buf[at + 16..at + 16 + UUID_LEN]);
        Self {
            devid,
            offset,
            dev_uuid,
        }
    }
}

// ---------------------------------------------------------------------------
// Inode item
// ---------------------------------------------------------------------------
// spec: §3 (INODE_ITEM)

#[derive(Debug, Clone, Copy)]
pub(crate) struct InodeItem {
    pub(crate) generation: u64,
    pub(crate) transid: u64,
    pub(crate) size: u64,
    pub(crate) nbytes: u64,
    pub(crate) block_group: u64,
    pub(crate) nlink: u32,
    pub(crate) uid: u32,
    pub(crate) gid: u32,
    pub(crate) mode: u32,
    pub(crate) rdev: u64,
    pub(crate) flags: u64,
    pub(crate) sequence: u64,
    // 32 reserved bytes follow
    // atime / ctime / mtime / otime (4 * 12 bytes) follow
}

impl InodeItem {
    pub(crate) const SIZE: usize = 160;

    pub(crate) fn parse(buf: &[u8], at: usize) -> Option<Self> {
        if buf.len() < at + Self::SIZE {
            return None;
        }
        Some(Self {
            generation: read_le_u64(buf, at),
            transid: read_le_u64(buf, at + 8),
            size: read_le_u64(buf, at + 16),
            nbytes: read_le_u64(buf, at + 24),
            block_group: read_le_u64(buf, at + 32),
            nlink: read_le_u32(buf, at + 40),
            uid: read_le_u32(buf, at + 44),
            gid: read_le_u32(buf, at + 48),
            mode: read_le_u32(buf, at + 52),
            rdev: read_le_u64(buf, at + 56),
            flags: read_le_u64(buf, at + 64),
            sequence: read_le_u64(buf, at + 72),
        })
    }
}

// ---------------------------------------------------------------------------
// Root item (subvolume / tree root descriptor)
// ---------------------------------------------------------------------------
// spec: §9

#[derive(Debug, Clone)]
pub(crate) struct RootItem {
    pub(crate) inode: InodeItem,
    pub(crate) generation: u64,
    pub(crate) root_dirid: u64,
    pub(crate) bytenr: u64,
    pub(crate) byte_limit: u64,
    pub(crate) bytes_used: u64,
    pub(crate) last_snapshot: u64,
    pub(crate) flags: u64,
    pub(crate) refs: u32,
    pub(crate) drop_progress: DiskKey,
    pub(crate) drop_level: u8,
    pub(crate) level: u8,
    // followed by `generation_v2` and a number of newer fields we don't need
}

impl RootItem {
    pub(crate) const MIN_SIZE: usize =
        InodeItem::SIZE + 8 + 8 + 8 + 8 + 8 + 8 + 8 + 4 + DiskKey::SIZE + 1 + 1; // 239

    pub(crate) fn parse(buf: &[u8], at: usize) -> Option<Self> {
        if buf.len() < at + Self::MIN_SIZE {
            return None;
        }
        let inode = InodeItem::parse(buf, at)?;
        let mut p = at + InodeItem::SIZE;
        let generation = read_le_u64(buf, p);
        p += 8;
        let root_dirid = read_le_u64(buf, p);
        p += 8;
        let bytenr = read_le_u64(buf, p);
        p += 8;
        let byte_limit = read_le_u64(buf, p);
        p += 8;
        let bytes_used = read_le_u64(buf, p);
        p += 8;
        let last_snapshot = read_le_u64(buf, p);
        p += 8;
        let flags = read_le_u64(buf, p);
        p += 8;
        let refs = read_le_u32(buf, p);
        p += 4;
        let drop_progress = DiskKey::parse(buf, p);
        p += DiskKey::SIZE;
        let drop_level = read_le_u8(buf, p);
        p += 1;
        let level = read_le_u8(buf, p);
        Some(Self {
            inode,
            generation,
            root_dirid,
            bytenr,
            byte_limit,
            bytes_used,
            last_snapshot,
            flags,
            refs,
            drop_progress,
            drop_level,
            level,
        })
    }
}

// ---------------------------------------------------------------------------
// DIR_ITEM / DIR_INDEX entry (variable-tailed)
// ---------------------------------------------------------------------------
// spec: §7

#[derive(Debug, Clone)]
pub(crate) struct DirEntry<'a> {
    pub(crate) location: DiskKey,
    pub(crate) transid: u64,
    pub(crate) data_len: u16,
    pub(crate) name_len: u16,
    pub(crate) ty: u8,
    pub(crate) name: &'a [u8],
}

impl<'a> DirEntry<'a> {
    /// Fixed-size header before the variable name + data tail.
    pub(crate) const HEADER_SIZE: usize = DiskKey::SIZE + 8 + 2 + 2 + 1; // 30

    /// Parse one entry starting at `at`. Returns the entry plus the byte
    /// offset of the next entry (relevant when multiple records are packed
    /// back-to-back due to hash collisions).
    pub(crate) fn parse(buf: &'a [u8], at: usize) -> Option<(Self, usize)> {
        if buf.len() < at + Self::HEADER_SIZE {
            return None;
        }
        let location = DiskKey::parse(buf, at);
        let transid = read_le_u64(buf, at + DiskKey::SIZE);
        let data_len = read_le_u16(buf, at + DiskKey::SIZE + 8);
        let name_len = read_le_u16(buf, at + DiskKey::SIZE + 10);
        let ty = read_le_u8(buf, at + DiskKey::SIZE + 12);
        let name_start = at + Self::HEADER_SIZE;
        let name_end = name_start + name_len as usize;
        let data_end = name_end + data_len as usize;
        if buf.len() < data_end {
            return None;
        }
        Some((
            Self {
                location,
                transid,
                data_len,
                name_len,
                ty,
                name: &buf[name_start..name_end],
            },
            data_end,
        ))
    }
}

// ---------------------------------------------------------------------------
// EXTENT_DATA (file-extent item)
// ---------------------------------------------------------------------------
// spec: §8

#[derive(Debug, Clone, Copy)]
pub(crate) struct ExtentDataHeader {
    pub(crate) generation: u64,
    pub(crate) ram_bytes: u64,
    pub(crate) compression: u8,
    pub(crate) encryption: u8,
    pub(crate) other_encoding: u16,
    pub(crate) ty: u8,
}

impl ExtentDataHeader {
    pub(crate) const SIZE: usize = 21;

    pub(crate) fn parse(buf: &[u8], at: usize) -> Option<Self> {
        if buf.len() < at + Self::SIZE {
            return None;
        }
        Some(Self {
            generation: read_le_u64(buf, at),
            ram_bytes: read_le_u64(buf, at + 8),
            compression: read_le_u8(buf, at + 16),
            encryption: read_le_u8(buf, at + 17),
            other_encoding: read_le_u16(buf, at + 18),
            ty: read_le_u8(buf, at + 20),
        })
    }
}

/// Tail body of a regular or prealloc extent (32 bytes).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExtentDataRegular {
    pub(crate) disk_bytenr: u64,
    pub(crate) disk_num_bytes: u64,
    pub(crate) offset: u64,
    pub(crate) num_bytes: u64,
}

impl ExtentDataRegular {
    pub(crate) const SIZE: usize = 32;

    pub(crate) fn parse(buf: &[u8], at: usize) -> Option<Self> {
        if buf.len() < at + Self::SIZE {
            return None;
        }
        Some(Self {
            disk_bytenr: read_le_u64(buf, at),
            disk_num_bytes: read_le_u64(buf, at + 8),
            offset: read_le_u64(buf, at + 16),
            num_bytes: read_le_u64(buf, at + 24),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_key_ordering() {
        let a = DiskKey {
            objectid: 1,
            item_type: 1,
            offset: 0,
        };
        let b = DiskKey {
            objectid: 1,
            item_type: 1,
            offset: 1,
        };
        let c = DiskKey {
            objectid: 1,
            item_type: 2,
            offset: 0,
        };
        let d = DiskKey {
            objectid: 2,
            item_type: 0,
            offset: 0,
        };
        assert!(a < b && b < c && c < d);
    }

    #[test]
    fn disk_key_parse_round_trips_offsets() {
        let mut buf = [0u8; DiskKey::SIZE];
        buf[0..8].copy_from_slice(&42u64.to_le_bytes());
        buf[8] = 84;
        buf[9..17].copy_from_slice(&7u64.to_le_bytes());
        let key = DiskKey::parse(&buf, 0);
        assert_eq!(key.objectid, 42);
        assert_eq!(key.item_type, 84);
        assert_eq!(key.offset, 7);
    }
}
