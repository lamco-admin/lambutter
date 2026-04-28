// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Zlib (deflate) decompression via `miniz_oxide`.
//!
//! btrfs zlib compression uses raw deflate streams wrapped in zlib (with the
//! 2-byte header + 4-byte adler32 trailer). `miniz_oxide` exposes
//! `decompress_to_vec_zlib_with_limit` which handles the header/trailer.

use alloc::vec::Vec;

use super::MAX_DECOMPRESSED_EXTENT_BYTES;
use crate::error::{Error, Result};

pub(super) fn decode(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    let max = MAX_DECOMPRESSED_EXTENT_BYTES.saturating_sub(dst.len());
    if max == 0 {
        return Err(Error::BadCompression {
            algorithm: "comp_zlib",
        });
    }
    let decoded =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(src, max).map_err(|_| {
            Error::BadCompression {
                algorithm: "comp_zlib",
            }
        })?;
    dst.extend_from_slice(&decoded);
    Ok(())
}
