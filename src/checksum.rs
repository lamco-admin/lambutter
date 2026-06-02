// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! CRC32C (Castagnoli) wrapper used for two distinct purposes:
//! - Tree block / superblock metadata-csum verification
//! - Name hashing for DIR_ITEM lookups (with a non-default seed)
//!
//! Implementation is delegated to the `crc` crate's `CRC_32_ISCSI` algorithm,
//! which is the iSCSI CRC32C polynomial — identical to btrfs's metadata
//! csum. The crate is `no_std`-clean.

use crc::{Crc, CRC_32_ISCSI};

const CRC32C: Crc<u32> = Crc::<u32>::new(&CRC_32_ISCSI);

/// Compute CRC32C over a byte slice with the standard seed of 0. Used for
/// tree-block csums (the first 4 bytes of the 32-byte csum field hold this
/// value little-endian; the remaining 28 bytes are zero).
/// spec: BTRFS-FORMAT-READONLY-REFERENCE §11
pub(crate) fn crc32c(data: &[u8]) -> u32 {
    CRC32C.checksum(data)
}

/// Compute btrfs's name-hash: a CRC32C variant with the seed loaded directly
/// into the running register (no implicit reflection by the algorithm
/// driver) and no final XOR. This is the form documented in
/// BTRFS-FORMAT-READONLY-REFERENCE §7 and used in the kernel's
/// `btrfs_name_hash` and python-btrfs's `name_hash`. Verified against the
/// `mkfs.btrfs --rootdir` output for fixture F1: hello.txt -> 0x415FEB59.
///
/// The reflected-form Castagnoli table is generated at compile time. We
/// don't reuse the `crc` crate here because its `digest_with_initial`
/// reflects the seed when `refin=true`, which silently changes the
/// running-register state and produces a different hash than the kernel.
pub(crate) fn crc32c_with_seed(seed: u32, data: &[u8]) -> u32 {
    let mut crc = seed;
    for &b in data {
        crc = NAME_HASH_TABLE[((crc ^ u32::from(b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    crc
}

/// Reflected Castagnoli CRC32C lookup table (poly 0x82F63B78).
const NAME_HASH_TABLE: [u32; 256] = generate_name_hash_table();

const fn generate_name_hash_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut n = 0u32;
    while n < 256 {
        let mut c = n;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
                c = (c >> 1) ^ 0x82F6_3B78;
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[n as usize] = c;
        n += 1;
    }
    table
}

/// Verify a btrfs csum field against a body slice. Returns `true` when the
/// stored 32-byte csum matches the computed CRC32C of the body. Only the
/// first 4 bytes of the csum field carry CRC32C data; the remaining 28 are
/// zero (used by xxhash/sha256/blake2 alternative csum types we do not
/// support).
pub(crate) fn verify_crc32c(body: &[u8], stored: &[u8]) -> bool {
    if stored.len() < 4 {
        return false;
    }
    let expected = u32::from_le_bytes([stored[0], stored[1], stored[2], stored[3]]);
    crc32c(body) == expected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_crc32c_value() {
        // Standard CRC32C test vector: empty input → 0
        assert_eq!(crc32c(&[]), 0);

        // Standard CRC32C: "123456789" → 0xE3069283
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }

    #[test]
    fn name_hash_matches_btrfs_kernel_output() {
        // Verified against `mkfs.btrfs --rootdir`-produced fixture F1:
        // dump-tree -t fs reports DIR_ITEM offsets matching these hashes.
        assert_eq!(crc32c_with_seed(0xFFFF_FFFE, b"hello.txt"), 0x415F_EB59);
        assert_eq!(crc32c_with_seed(0xFFFF_FFFE, b"dir-a"), 0x2C54_9827);
        assert_eq!(crc32c_with_seed(0xFFFF_FFFE, b"nested.txt"), 0x86F5_F6F8);
    }

    #[test]
    fn name_hash_uses_seed() {
        // Standard crc32c(seed=0) yields a different result than the
        // btrfs-specific seed=0xFFFFFFFE used for name hashing.
        let a = crc32c(b"default");
        let b = crc32c_with_seed(0xFFFF_FFFE, b"default");
        assert_ne!(a, b, "name-hash seed must change the result");
    }

    #[test]
    fn verify_csum_round_trip() {
        let body = b"hello, btrfs";
        let mut stored = [0u8; 32];
        let csum = crc32c(body);
        stored[0..4].copy_from_slice(&csum.to_le_bytes());
        assert!(verify_crc32c(body, &stored));
        // Flip a byte; verification must fail.
        stored[0] ^= 0x01;
        assert!(!verify_crc32c(body, &stored));
    }
}
