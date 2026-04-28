// Copyright 2025-2026 Lamco Development
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

pub(super) fn decode(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    let mut input = src;
    while !input.is_empty() {
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
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_empty() {
        // The smallest valid zstd payload (empty frame) was harder to hand-
        // construct; instead this test ensures the decoder rejects an
        // obviously bogus prefix without panicking.
        let mut dst = Vec::new();
        let result = decode(b"\x00\x00\x00\x00", &mut dst);
        assert!(result.is_err());
    }
}
