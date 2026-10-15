// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! LZO decompression for btrfs.
//!
//! btrfs compresses LZO data one sector at a time. Each sector's output is a
//! segment: a u32 little-endian length followed by that many compressed bytes,
//! and the segments are wrapped by an outer u32 total length. A segment header
//! never straddles a sector boundary of the compressed stream: when fewer than
//! four bytes are left in a sector, the writer zero-pads to the next one.
//! Each segment's payload goes to `lzokay`'s LZO1X decoder. Real-world
//! prevalence of LZO on stock `/boot` is essentially zero (Fedora 33+,
//! Tumbleweed, CachyOS, Garuda all default to zstd) but the path is exercised
//! by fixture F4 and is correct against `mkfs.btrfs --rootdir` +
//! `mount -o compress=lzo`.

use alloc::{vec, vec::Vec};

use super::MAX_DECOMPRESSED_EXTENT_BYTES;
use crate::error::{Error, Result};

/// Bytes of a segment header.
const SEGMENT_HEADER: usize = 4;

/// Decode one compressed extent. `sectorsize` is the filesystem's, already
/// validated as a power of two from 4 KiB to 64 KiB: it bounds each segment's
/// plaintext and places the padding between segments.
pub(super) fn decode(src: &[u8], dst: &mut Vec<u8>, sectorsize: usize) -> Result<()> {
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

    let bad = || Error::BadCompression {
        algorithm: "comp_lzo",
    };
    let mut scratch = vec![0u8; sectorsize];
    let mut p = 4usize;
    while p < total_compressed {
        let left_in_sector = sectorsize - p % sectorsize;
        if left_in_sector < SEGMENT_HEADER {
            p += left_in_sector;
            if p >= total_compressed {
                break;
            }
        }
        if p + SEGMENT_HEADER > total_compressed {
            return Err(bad());
        }
        let segment_len = u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]) as usize;
        p += SEGMENT_HEADER;
        if segment_len == 0 {
            // Trailing sector header with zero size signals end-of-stream
            // padding under some btrfs configurations.
            break;
        }
        let end = p.checked_add(segment_len).ok_or_else(bad)?;
        if end > total_compressed {
            return Err(bad());
        }
        let segment = &src[p..end];
        p = end;

        if dst.len() + sectorsize > MAX_DECOMPRESSED_EXTENT_BYTES {
            return Err(bad());
        }
        let n = lzokay::decompress::decompress(segment, &mut scratch).map_err(|_| bad())?;
        dst.extend_from_slice(&scratch[..n]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECTOR: usize = 4096;

    /// An LZO1X stream of `n` literal bytes of `fill` (n from 4 to 238 uses
    /// the short first-byte form, longer runs the zero-extended form), then
    /// the end-of-stream match.
    fn literals(n: usize, fill: u8) -> Vec<u8> {
        let mut s = if n <= 238 {
            vec![(n + 17) as u8]
        } else {
            // Run length is t + 3 with t = 15 + the extension byte.
            vec![0, (n - 18) as u8]
        };
        s.resize(s.len() + n, fill);
        s.extend_from_slice(&[0x11, 0, 0]);
        s
    }

    /// Lay out segments the way the writer does: pad to the next sector when
    /// a header would cross it, then patch in the total length.
    fn extent(segments: &[Vec<u8>]) -> Vec<u8> {
        let mut out = vec![0u8; 4];
        for seg in segments {
            let left = SECTOR - out.len() % SECTOR;
            if left < SEGMENT_HEADER {
                out.resize(out.len() + left, 0);
            }
            out.extend_from_slice(&(seg.len() as u32).to_le_bytes());
            out.extend_from_slice(seg);
        }
        let total = out.len() as u32;
        out[..4].copy_from_slice(&total.to_le_bytes());
        out
    }

    #[test]
    fn header_with_a_zero_low_byte_at_an_unaligned_offset() {
        // The 9-byte first segment leaves the next header at offset 17, and a
        // 256-byte segment's length starts with a zero byte.
        let first = literals(5, b'a');
        let second = literals(251, b'b');
        assert_eq!(second.len(), 256);
        let src = extent(&[first, second]);
        let mut dst = Vec::new();
        decode(&src, &mut dst, SECTOR).unwrap();
        assert_eq!(dst.len(), 256);
        assert!(dst[..5].iter().all(|&b| b == b'a'));
        assert!(dst[5..].iter().all(|&b| b == b'b'));
    }

    #[test]
    fn header_moves_to_the_next_sector_when_three_bytes_are_left() {
        // Nineteen 208-byte segments end at 3956; a 137-byte one ends at 4093,
        // three bytes short of the sector, so the last header sits at 4096.
        let mut segs = vec![literals(200, b'x'); 19];
        segs.push(literals(129, b'y'));
        segs.push(literals(10, b'z'));
        let src = extent(&segs);
        assert_eq!(&src[4093..4096], &[0, 0, 0]);
        let mut dst = Vec::new();
        decode(&src, &mut dst, SECTOR).unwrap();
        assert_eq!(dst.len(), 19 * 200 + 129 + 10);
        assert!(dst[dst.len() - 10..].iter().all(|&b| b == b'z'));
    }

    #[test]
    fn segment_plaintext_may_fill_a_larger_sector() {
        let big = 16384;
        // A zero opcode, then 64 zero bytes worth 255 each and a final 46:
        // t = 255 * 64 + 15 + 46 = 16381, and the run is t + 3 literals.
        let mut seg = vec![0u8; 65];
        seg.push(46);
        seg.resize(seg.len() + big, b'q');
        seg.extend_from_slice(&[0x11, 0, 0]);
        let mut src = vec![0u8; 4];
        src.extend_from_slice(&(seg.len() as u32).to_le_bytes());
        src.extend_from_slice(&seg);
        let total = src.len() as u32;
        src[..4].copy_from_slice(&total.to_le_bytes());

        let mut dst = Vec::new();
        decode(&src, &mut dst, big).unwrap();
        assert_eq!(dst.len(), big);
        assert!(decode(&src, &mut Vec::new(), SECTOR).is_err());
    }

    #[test]
    fn rejects_short_input() {
        let mut dst = Vec::new();
        assert!(decode(b"", &mut dst, SECTOR).is_err());
        assert!(decode(&[0, 0, 0], &mut dst, SECTOR).is_err());
    }

    #[test]
    fn rejects_oversized_total() {
        let mut dst = Vec::new();
        // Claim total = 1024 but supply only 4 bytes.
        let buf = [0, 4, 0, 0];
        assert!(decode(&buf, &mut dst, SECTOR).is_err());
    }

    #[test]
    fn rejects_truncated_sector() {
        let mut dst = Vec::new();
        // total = 16 bytes, then sector header claims 1024 bytes
        let mut buf = vec![16, 0, 0, 0]; // total
        buf.extend_from_slice(&1024u32.to_le_bytes()); // sector size
        buf.extend_from_slice(&[0u8; 8]); // not enough room for 1024
        assert!(decode(&buf, &mut dst, SECTOR).is_err());
    }
}
