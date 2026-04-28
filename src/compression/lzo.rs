// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! LZO compression for btrfs.
//!
//! Status: **partial — not shipping in v0.1.0.** This module implements the
//! outer btrfs LZO sector-wrapper parser (which is documented and small)
//! but defers the inner LZO1X-1 decoder to a follow-up release. A
//! safety-first decision: a wrong LZO decoder in a Secure-Boot-relevant
//! read path is far more dangerous than a missing one. v0.1.1 will land
//! the validated decoder; until then encountering an LZO-compressed
//! extent surfaces `Error::BadCompression { algorithm: "comp_lzo" }`.
//!
//! btrfs LZO outer-wrapper layout:
//!
//!   u32 total_compressed_size       (LE)
//!   for each sector:
//!     u32 sector_compressed_size    (LE)
//!     u8[sector_compressed_size] sector_payload   (LZO1X-1)
//!
//! Real-world prevalence: Fedora 33+ defaults to zstd; openSUSE Tumbleweed
//! defaults to zstd (with grub2-bls migration in progress); CachyOS and
//! Garuda use zstd. LZO-compressed /boot is rare and the deferral is low
//! risk for the v0.1.0 release target.

use alloc::vec::Vec;

use crate::error::{Error, Result};

pub(super) fn decode(src: &[u8], _dst: &mut Vec<u8>) -> Result<()> {
    // Validate the outer wrapper header so callers that hit an LZO extent
    // get a faithful "we recognize this is LZO" response rather than a
    // generic "unknown compression" error.
    if src.len() < 4 {
        return Err(Error::BadCompression {
            algorithm: "comp_lzo",
        });
    }
    let total = u32::from_le_bytes([src[0], src[1], src[2], src[3]]) as usize;
    if total > src.len() {
        return Err(Error::BadCompression {
            algorithm: "comp_lzo",
        });
    }
    // Inner LZO1X-1 decode is deferred to v0.1.1.
    Err(Error::BadCompression {
        algorithm: "comp_lzo",
    })
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
    fn returns_bad_compression_for_well_formed_wrapper() {
        // Valid outer wrapper (total = 4, just the header itself) — but
        // inner decode is unimplemented, so we expect BadCompression.
        let mut dst = Vec::new();
        let buf = [4, 0, 0, 0];
        assert!(matches!(
            decode(&buf, &mut dst),
            Err(Error::BadCompression {
                algorithm: "comp_lzo"
            })
        ));
    }
}
