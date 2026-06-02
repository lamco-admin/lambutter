// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Zstd decompression via `ruzstd` (pure-Rust, no_std-compatible).
//!
//! btrfs's zstd compression splits data into independent zstd frames per
//! filesystem block. We loop until the input is exhausted, decoding one
//! frame at a time. `ruzstd::StreamingDecoder` implements `Read` (its own
//! no_std trait when `default-features = false`).

use alloc::vec::Vec;

use ruzstd::{io::Read, StreamingDecoder};

use super::MAX_DECOMPRESSED_EXTENT_BYTES;
use crate::error::{Error, Result};

/// Magic bytes (little-endian u32) for the zstandard frame format.
/// btrfs may emit multiple back-to-back frames per extent (compress block
/// = filesystem block size); the on-disk slice we receive is sized to the
/// extent's `disk_num_bytes`, which includes alignment padding past the
/// last real frame.
const ZSTD_FRAME_MAGIC: u32 = 0xFD2F_B528;

pub(super) fn decode(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    let mut input = src;
    loop {
        // Stop when the remaining bytes are alignment padding rather than
        // another zstd frame. Without this guard ruzstd would surface a
        // bad-magic error for the trailing zeros and we'd misreport the
        // extent as malformed.
        if input.len() < 4 {
            return Ok(());
        }
        let magic = u32::from_le_bytes([input[0], input[1], input[2], input[3]]);
        if magic != ZSTD_FRAME_MAGIC {
            // Skippable-frame magics (0x184D2A50..0x184D2A5F) are also
            // legal zstd, but btrfs does not emit them; treat anything
            // non-frame as the end of payload.
            return Ok(());
        }

        let mut decoder = StreamingDecoder::new(&mut input).map_err(|_| Error::BadCompression {
            algorithm: "comp_zstd",
        })?;
        let mut buf = [0u8; 4096];
        loop {
            let n = decoder.read(&mut buf).map_err(|_| Error::BadCompression {
                algorithm: "comp_zstd",
            })?;
            if n == 0 {
                break;
            }
            if dst.len() + n > MAX_DECOMPRESSED_EXTENT_BYTES {
                return Err(Error::BadCompression {
                    algorithm: "comp_zstd",
                });
            }
            dst.extend_from_slice(&buf[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn non_zstd_prefix_terminates_cleanly() {
        // btrfs zstd extents are sector-padded with zeros past the last
        // frame. The decoder's outer loop must treat a non-frame-magic
        // run as end-of-payload (returns Ok with whatever it already
        // decoded), not as a corruption error. Bogus zeros at the start
        // are equivalent: empty output, no error.
        let mut dst = Vec::new();
        let result = decode(b"\x00\x00\x00\x00", &mut dst);
        assert!(result.is_ok());
        assert!(dst.is_empty());
    }

    #[test]
    fn malformed_zstd_frame_returns_bad_compression() {
        // A buffer that starts with the zstd magic but has a malformed
        // body must surface BadCompression, not silently truncate.
        use alloc::vec;
        let mut dst = Vec::new();
        let mut bogus = vec![0u8; 64];
        bogus[0..4].copy_from_slice(&0xFD2F_B528u32.to_le_bytes());
        let result = decode(&bogus, &mut dst);
        assert!(matches!(
            result,
            Err(crate::error::Error::BadCompression {
                algorithm: "comp_zstd"
            })
        ));
    }
}
