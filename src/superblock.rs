// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Superblock loader + validator.
//!
//! Reads all four canonical superblock locations, validates each, and
//! selects the highest-generation valid copy. spec: BTRFS-FORMAT-READONLY-
//! REFERENCE §4.

use alloc::vec;

use crate::{
    block_read::BlockRead,
    checksum::verify_crc32c,
    error::{Error, Result, SuperblockReason},
    format::{
        constants::{
            BTRFS_MAGIC, CSUM_TYPE_BLAKE2, CSUM_TYPE_CRC32C, CSUM_TYPE_SHA256, CSUM_TYPE_XXHASH,
            MAX_NODE_SIZE, MAX_SECTOR_SIZE, MIN_NODE_SIZE, MIN_SECTOR_SIZE, SUPERBLOCK_OFFSETS,
            SUPERBLOCK_SIZE,
        },
        flags::IncompatFlags,
        repr::Superblock,
    },
};

/// Outcome of attempting to load and validate the superblock from a single
/// canonical location.
#[derive(Debug)]
enum CopyOutcome {
    Valid(Superblock),
    BadMagic,
    BadCsum,
    UnsupportedCsumType(&'static str),
    UnsupportedIncompat(&'static str),
    BadGeometry,
    IoError,
}

/// Load and validate the superblock by trying each canonical location and
/// picking the highest-generation valid copy.
pub(crate) fn load<R: BlockRead>(reader: &mut R, device_size_bytes: u64) -> Result<Superblock> {
    let mut last_specific_reason: Option<SuperblockReason> = None;
    let mut best: Option<Superblock> = None;

    for offset in SUPERBLOCK_OFFSETS {
        // Per spec §4: the third and fourth copies are present only if the
        // device is large enough. Don't probe past EOD; that's a normal
        // condition, not an error.
        if offset >= device_size_bytes {
            continue;
        }

        match try_load_one(reader, offset) {
            CopyOutcome::Valid(sb) => match &best {
                None => best = Some(sb),
                Some(current) if sb.generation > current.generation => best = Some(sb),
                _ => {}
            },
            CopyOutcome::BadMagic => {
                last_specific_reason.get_or_insert(SuperblockReason::BadMagic);
            }
            CopyOutcome::BadCsum => {
                last_specific_reason.get_or_insert(SuperblockReason::BadCsum);
            }
            CopyOutcome::UnsupportedCsumType(token) => {
                // Surface this immediately — a wrong csum type at copy 0
                // means we can't trust the others either; they share fsid
                // and tooling.
                return Err(Error::UnsupportedChecksum(token));
            }
            CopyOutcome::UnsupportedIncompat(token) => {
                return Err(Error::UnsupportedFeature(token));
            }
            CopyOutcome::BadGeometry => {
                last_specific_reason.get_or_insert(SuperblockReason::BadGeometry);
            }
            CopyOutcome::IoError => {
                // Continue to the next copy on transient I/O.
            }
        }
    }

    best.ok_or_else(|| {
        Error::BadSuperblock(last_specific_reason.unwrap_or(SuperblockReason::NoValidCopy))
    })
}

fn try_load_one<R: BlockRead>(reader: &mut R, offset: u64) -> CopyOutcome {
    let mut buf = vec![0u8; SUPERBLOCK_SIZE];
    if reader.read_at(offset, &mut buf).is_err() {
        return CopyOutcome::IoError;
    }

    let Some(sb) = Superblock::parse(&buf) else {
        return CopyOutcome::BadMagic;
    };

    if sb.magic != BTRFS_MAGIC {
        return CopyOutcome::BadMagic;
    }

    // Csum verification covers everything past the 32-byte csum field.
    let body_offset = Superblock::body_offset_for_csum();
    if !verify_crc32c(&buf[body_offset..], &sb.csum) {
        return CopyOutcome::BadCsum;
    }

    match sb.csum_type {
        CSUM_TYPE_CRC32C => {}
        CSUM_TYPE_XXHASH => return CopyOutcome::UnsupportedCsumType("csum_xxhash"),
        CSUM_TYPE_SHA256 => return CopyOutcome::UnsupportedCsumType("csum_sha256"),
        CSUM_TYPE_BLAKE2 => return CopyOutcome::UnsupportedCsumType("csum_blake2"),
        _ => return CopyOutcome::UnsupportedCsumType("csum_unknown"),
    }

    let incompat = IncompatFlags::from_bits_truncate(sb.incompat_flags);
    let rejected = incompat.intersection(IncompatFlags::rejected_for_v0_1());
    if rejected.contains(IncompatFlags::ZONED) {
        return CopyOutcome::UnsupportedIncompat("feat_zoned");
    }
    if rejected.contains(IncompatFlags::RAID_STRIPE_TREE) {
        return CopyOutcome::UnsupportedIncompat("feat_raid_stripe_tree");
    }

    if !is_pow2_in_range(sb.sectorsize, MIN_SECTOR_SIZE, MAX_SECTOR_SIZE)
        || !is_pow2_in_range(sb.nodesize, MIN_NODE_SIZE, MAX_NODE_SIZE)
    {
        return CopyOutcome::BadGeometry;
    }

    CopyOutcome::Valid(sb)
}

#[inline]
fn is_pow2_in_range(value: u32, min: u32, max: u32) -> bool {
    value >= min && value <= max && value.is_power_of_two()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        checksum::crc32c,
        format::constants::{CSUM_LEN, FSID_LEN, SUPERBLOCK_PRIMARY_OFFSET},
    };

    fn build_minimal_valid_sb() -> [u8; SUPERBLOCK_SIZE] {
        let mut sb = [0u8; SUPERBLOCK_SIZE];

        // fsid (any non-zero value)
        sb[CSUM_LEN..CSUM_LEN + FSID_LEN].copy_from_slice(&[0xAA; FSID_LEN]);

        // bytenr: where this SB lives on disk
        sb[48..56].copy_from_slice(&SUPERBLOCK_PRIMARY_OFFSET.to_le_bytes());

        // magic
        sb[64..72].copy_from_slice(&BTRFS_MAGIC.to_le_bytes());

        // generation
        sb[72..80].copy_from_slice(&1u64.to_le_bytes());

        // root, chunk_root, log_root: any non-zero
        sb[80..88].copy_from_slice(&0x10_0000u64.to_le_bytes());
        sb[88..96].copy_from_slice(&0x20_0000u64.to_le_bytes());
        sb[96..104].copy_from_slice(&0u64.to_le_bytes());

        // total_bytes (just needs to be plausible)
        sb[112..120].copy_from_slice(&(1u64 << 30).to_le_bytes());
        sb[120..128].copy_from_slice(&(1u64 << 20).to_le_bytes());

        // root_dir_objectid = 6 (BTRFS_ROOT_TREE_DIR_OBJECTID)
        sb[128..136].copy_from_slice(&6u64.to_le_bytes());

        // num_devices = 1
        sb[136..144].copy_from_slice(&1u64.to_le_bytes());

        // sectorsize, nodesize, leafsize, stripesize: 4096
        sb[144..148].copy_from_slice(&4096u32.to_le_bytes());
        sb[148..152].copy_from_slice(&16384u32.to_le_bytes());
        sb[152..156].copy_from_slice(&16384u32.to_le_bytes());
        sb[156..160].copy_from_slice(&4096u32.to_le_bytes());

        // sys_chunk_array_size = 0 (we'll populate when chunk tree work begins)
        sb[160..164].copy_from_slice(&0u32.to_le_bytes());

        // chunk_root_generation = 1
        sb[164..172].copy_from_slice(&1u64.to_le_bytes());

        // compat_flags / compat_ro_flags / incompat_flags = 0 (all-tolerable)
        sb[172..180].copy_from_slice(&0u64.to_le_bytes());
        sb[180..188].copy_from_slice(&0u64.to_le_bytes());
        sb[188..196].copy_from_slice(&0u64.to_le_bytes());

        // csum_type = CRC32C (= 0); already zero-initialized.

        // root_level / chunk_root_level / log_root_level = 0; already zero.

        // Compute and write csum over body (everything past the 32-byte csum field)
        let body_csum = crc32c(&sb[CSUM_LEN..]);
        sb[0..4].copy_from_slice(&body_csum.to_le_bytes());
        // Bytes 4..32 of csum field stay zero (CRC32C uses only the first 4)

        sb
    }

    #[test]
    fn loads_minimal_valid_sb_from_primary() {
        let sb_bytes = build_minimal_valid_sb();
        // Synthesize a "device" with the SB at the primary offset.
        let mut device = vec![0u8; SUPERBLOCK_PRIMARY_OFFSET as usize + SUPERBLOCK_SIZE];
        device[SUPERBLOCK_PRIMARY_OFFSET as usize..].copy_from_slice(&sb_bytes);

        let mut reader: &[u8] = &device;
        let device_size = device.len() as u64;
        let sb = load(&mut reader, device_size).expect("valid superblock should load");
        assert_eq!(sb.magic, BTRFS_MAGIC);
        assert_eq!(sb.sectorsize, 4096);
        assert_eq!(sb.nodesize, 16384);
        assert_eq!(sb.csum_type, CSUM_TYPE_CRC32C);
    }

    #[test]
    fn rejects_bad_magic() {
        let mut sb_bytes = build_minimal_valid_sb();
        sb_bytes[64..72].copy_from_slice(&0xDEAD_BEEF_DEAD_BEEFu64.to_le_bytes());
        // Re-csum
        let body_csum = crc32c(&sb_bytes[CSUM_LEN..]);
        sb_bytes[0..4].copy_from_slice(&body_csum.to_le_bytes());

        let mut device = vec![0u8; SUPERBLOCK_PRIMARY_OFFSET as usize + SUPERBLOCK_SIZE];
        device[SUPERBLOCK_PRIMARY_OFFSET as usize..].copy_from_slice(&sb_bytes);

        let mut reader: &[u8] = &device;
        let device_size = device.len() as u64;
        assert!(matches!(
            load(&mut reader, device_size),
            Err(Error::BadSuperblock(SuperblockReason::BadMagic))
        ));
    }

    #[test]
    fn rejects_bad_csum() {
        let mut sb_bytes = build_minimal_valid_sb();
        sb_bytes[100] ^= 0xFF; // perturb body without re-csum

        let mut device = vec![0u8; SUPERBLOCK_PRIMARY_OFFSET as usize + SUPERBLOCK_SIZE];
        device[SUPERBLOCK_PRIMARY_OFFSET as usize..].copy_from_slice(&sb_bytes);

        let mut reader: &[u8] = &device;
        let device_size = device.len() as u64;
        assert!(matches!(
            load(&mut reader, device_size),
            Err(Error::BadSuperblock(SuperblockReason::BadCsum))
        ));
    }

    #[test]
    fn rejects_unsupported_csum_type() {
        let mut sb_bytes = build_minimal_valid_sb();
        sb_bytes[196..198].copy_from_slice(&CSUM_TYPE_SHA256.to_le_bytes());
        let body_csum = crc32c(&sb_bytes[CSUM_LEN..]);
        sb_bytes[0..4].copy_from_slice(&body_csum.to_le_bytes());

        let mut device = vec![0u8; SUPERBLOCK_PRIMARY_OFFSET as usize + SUPERBLOCK_SIZE];
        device[SUPERBLOCK_PRIMARY_OFFSET as usize..].copy_from_slice(&sb_bytes);

        let mut reader: &[u8] = &device;
        let device_size = device.len() as u64;
        assert!(matches!(
            load(&mut reader, device_size),
            Err(Error::UnsupportedChecksum("csum_sha256"))
        ));
    }

    #[test]
    fn rejects_zoned_incompat() {
        let mut sb_bytes = build_minimal_valid_sb();
        let zoned = IncompatFlags::ZONED.bits();
        sb_bytes[188..196].copy_from_slice(&zoned.to_le_bytes());
        let body_csum = crc32c(&sb_bytes[CSUM_LEN..]);
        sb_bytes[0..4].copy_from_slice(&body_csum.to_le_bytes());

        let mut device = vec![0u8; SUPERBLOCK_PRIMARY_OFFSET as usize + SUPERBLOCK_SIZE];
        device[SUPERBLOCK_PRIMARY_OFFSET as usize..].copy_from_slice(&sb_bytes);

        let mut reader: &[u8] = &device;
        let device_size = device.len() as u64;
        assert!(matches!(
            load(&mut reader, device_size),
            Err(Error::UnsupportedFeature("feat_zoned"))
        ));
    }

    #[test]
    fn picks_highest_generation_among_valid_copies() {
        // Two SBs at primary and secondary; secondary wins on generation.
        let mut sb_a = build_minimal_valid_sb();
        sb_a[72..80].copy_from_slice(&100u64.to_le_bytes());
        let body_a = crc32c(&sb_a[CSUM_LEN..]);
        sb_a[0..4].copy_from_slice(&body_a.to_le_bytes());

        let mut sb_b = build_minimal_valid_sb();
        sb_b[72..80].copy_from_slice(&200u64.to_le_bytes());
        let body_b = crc32c(&sb_b[CSUM_LEN..]);
        sb_b[0..4].copy_from_slice(&body_b.to_le_bytes());

        let secondary = SUPERBLOCK_OFFSETS[1] as usize;
        let mut device = vec![0u8; secondary + SUPERBLOCK_SIZE];
        device[SUPERBLOCK_PRIMARY_OFFSET as usize
            ..SUPERBLOCK_PRIMARY_OFFSET as usize + SUPERBLOCK_SIZE]
            .copy_from_slice(&sb_a);
        device[secondary..].copy_from_slice(&sb_b);

        let mut reader: &[u8] = &device;
        let device_size = device.len() as u64;
        let sb = load(&mut reader, device_size).expect("at least one copy is valid");
        assert_eq!(sb.generation, 200);
    }
}
