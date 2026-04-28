// Copyright 2025-2026 Lamco Development
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

/// Compute CRC32C with a non-default initial value. btrfs name-hashing uses
/// `seed = 0xFFFF_FFFE` per the format spec; the conventional `crc32c(name)`
/// will NOT find the right DIR_ITEM.
/// spec: BTRFS-FORMAT-READONLY-REFERENCE §7
pub(crate) fn crc32c_with_seed(seed: u32, data: &[u8]) -> u32 {
    let mut digest = CRC32C.digest_with_initial(seed);
    digest.update(data);
    digest.finalize()
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
    fn name_hash_uses_seed() {
        // Verify the seeded variant differs from the seed-0 variant.
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
