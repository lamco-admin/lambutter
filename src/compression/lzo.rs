// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! LZO decompression for btrfs.
//!
//! btrfs LZO compression splits data into ~4 KiB sectors, each prefixed
//! by a u32 little-endian size header, all wrapped by an outer total-size
//! u32. We parse the wrapper here and route each sector's payload to
//! `lzokay`'s LZO1X-1 decoder. Real-world prevalence of LZO on stock
//! `/boot` is essentially zero (Fedora 33+, Tumbleweed, CachyOS, Garuda
//! all default to zstd) but the path is exercised by fixture F4 and is
//! correct against `mkfs.btrfs --rootdir` + `mount -o compress=lzo`.

use alloc::{vec, vec::Vec};

use super::MAX_DECOMPRESSED_EXTENT_BYTES;
use crate::error::{Error, Result};

/// Each btrfs LZO sector decompresses to at most 4 KiB. The sector size
/// is implicit in the on-disk format (always the filesystem's `sectorsize`,
/// which lambutter only supports at 4 KiB in v0.1.x); ramping it up later
/// requires plumbing `sectorsize` through but does not break the wire
/// format.
const LZO_SECTOR_PLAINTEXT_BYTES: usize = 4096;

pub(super) fn decode(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    if src.len() < 4 {
        return Err(Error::BadCompression {
            algorithm: "comp_lzo",
        });
    }
    let total_compressed = u32::from_le_bytes([src[0], src[1], src[2], src[3]]) as usize;
    if total_compressed > src.len() || total_compressed < 4 {
        return Err(Error::BadCompression {
            algorithm: "comp_lzo",
        });
    }

    let mut p = 4usize;
    while p < total_compressed {
        if p + 4 > src.len() {
            return Err(Error::BadCompression {
                algorithm: "comp_lzo",
            });
        }
        let sector_compressed_size =
            u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]) as usize;
        p += 4;
        if sector_compressed_size == 0 {
            // Trailing sector header with zero size signals end-of-stream
            // padding under some btrfs configurations.
            break;
        }
        if p + sector_compressed_size > src.len() {
            return Err(Error::BadCompression {
                algorithm: "comp_lzo",
            });
        }
        let sector = &src[p..p + sector_compressed_size];
        p += sector_compressed_size;

        if dst.len() + LZO_SECTOR_PLAINTEXT_BYTES > MAX_DECOMPRESSED_EXTENT_BYTES {
            return Err(Error::BadCompression {
                algorithm: "comp_lzo",
            });
        }

        let mut scratch = vec![0u8; LZO_SECTOR_PLAINTEXT_BYTES];
        let n = lzokay::decompress::decompress(sector, &mut scratch).map_err(|_| {
            Error::BadCompression {
                algorithm: "comp_lzo",
            }
        })?;
        dst.extend_from_slice(&scratch[..n]);

        // btrfs aligns every sector header to a 4-byte boundary inside the
        // outer compressed buffer; advance past padding zeros to the next
        // 4-byte multiple if any are present.
        while p < total_compressed && p % 4 != 0 && src[p] == 0 {
            p += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_short_input() {
        let mut dst = Vec::new();
        assert!(decode(b"", &mut dst).is_err());
        assert!(decode(&[0, 0, 0], &mut dst).is_err());
    }

    #[test]
    fn rejects_oversized_total() {
        let mut dst = Vec::new();
        // Claim total = 1024 but supply only 4 bytes.
        let buf = [0, 4, 0, 0];
        assert!(decode(&buf, &mut dst).is_err());
    }

    #[test]
    fn rejects_truncated_sector() {
        let mut dst = Vec::new();
        // total = 16 bytes, then sector header claims 1024 bytes
        let mut buf = vec![16, 0, 0, 0]; // total
        buf.extend_from_slice(&1024u32.to_le_bytes()); // sector size
        buf.extend_from_slice(&[0u8; 8]); // not enough room for 1024
        assert!(decode(&buf, &mut dst).is_err());
    }
}
