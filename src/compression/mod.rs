// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Compression dispatch. Each algorithm lives behind a Cargo feature; an
//! algorithm that's encountered but not enabled at compile time produces
//! `Error::BadCompression` so consumers fail loud.

use alloc::vec::Vec;

use crate::{
    error::{Error, Result},
    format::constants::{COMPRESS_LZO, COMPRESS_NONE, COMPRESS_ZLIB, COMPRESS_ZSTD},
};

#[cfg(feature = "lzo")]
mod lzo;
#[cfg(feature = "zlib")]
mod zlib;
#[cfg(feature = "zstd")]
mod zstd;

/// Decoded plaintext capacity cap. Bounded by the format-spec recommended
/// 16 MiB (`MAX_DECOMPRESSED_EXTENT_BYTES`); decompressors that try to grow
/// past this bound return `Error::BadCompression`.
pub(crate) use crate::format::constants::MAX_DECOMPRESSED_EXTENT_BYTES;

/// Decompress `src` into `dst`, dispatching on `algorithm`. The output
/// buffer is appended to (not replaced); callers that want exact-size
/// behavior should clear `dst` before calling.
pub(crate) fn decode(algorithm: u8, src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    match algorithm {
        COMPRESS_NONE => {
            dst.extend_from_slice(src);
            Ok(())
        }
        COMPRESS_ZLIB => decode_zlib(src, dst),
        COMPRESS_LZO => decode_lzo(src, dst),
        COMPRESS_ZSTD => decode_zstd(src, dst),
        _ => Err(Error::BadCompression {
            algorithm: "comp_unknown",
        }),
    }
}

#[cfg(feature = "zlib")]
fn decode_zlib(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    zlib::decode(src, dst)
}
#[cfg(not(feature = "zlib"))]
fn decode_zlib(_src: &[u8], _dst: &mut Vec<u8>) -> Result<()> {
    Err(Error::BadCompression {
        algorithm: "comp_zlib",
    })
}

#[cfg(feature = "zstd")]
fn decode_zstd(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    zstd::decode(src, dst)
}
#[cfg(not(feature = "zstd"))]
fn decode_zstd(_src: &[u8], _dst: &mut Vec<u8>) -> Result<()> {
    Err(Error::BadCompression {
        algorithm: "comp_zstd",
    })
}

#[cfg(feature = "lzo")]
fn decode_lzo(src: &[u8], dst: &mut Vec<u8>) -> Result<()> {
    lzo::decode(src, dst)
}
#[cfg(not(feature = "lzo"))]
fn decode_lzo(_src: &[u8], _dst: &mut Vec<u8>) -> Result<()> {
    Err(Error::BadCompression {
        algorithm: "comp_lzo",
    })
}
