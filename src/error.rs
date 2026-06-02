// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Crate-wide `Error` type and stable `&'static str` token vocabulary.
//!
//! The token vocabulary is part of the public API: token values are stable
//! across patch releases per `docs/SPEC-LAMBUTTER.md` §11. Adding a token
//! is a minor bump; renaming or removing a token is a major bump.

use core::fmt;

/// Crate-wide result alias.
pub type Result<T> = core::result::Result<T, Error>;

/// All error conditions the crate can surface to its caller.
///
/// Variants carry stable `&'static str` tokens (see `docs/SPEC-LAMBUTTER.md`
/// §11) so audit consumers (e.g. LamBoot's trust log) can match on them
/// without parsing free text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Underlying block read failed. `token` is supplied by the caller's
    /// `BlockRead::Error: Debug` impl indirectly — we erase it to a static
    /// string at the boundary so the error type stays cheap to copy.
    Io {
        /// Stable token describing the I/O failure category.
        token: &'static str,
        /// Byte offset where the read attempt was made.
        offset: u64,
    },

    /// Superblock is missing, malformed, or carries an unsupported magic.
    BadSuperblock(SuperblockReason),

    /// Encountered an `INCOMPAT` flag the crate does not implement.
    /// See `docs/SPEC-LAMBUTTER.md` §11 for token vocabulary.
    UnsupportedFeature(&'static str),

    /// Encountered a chunk profile the crate does not implement
    /// (RAID0/RAID10/RAID5/RAID6 in v0.1.0).
    UnsupportedProfile(&'static str),

    /// Encountered a checksum type other than CRC32C.
    UnsupportedChecksum(&'static str),

    /// Metadata-block CRC32C mismatch at the named logical address.
    CsumMismatch {
        /// Logical bytenr of the offending tree block.
        logical: u64,
    },

    /// B-tree structure violation (key ordering, item region overflow,
    /// child count, or recursion depth).
    CorruptBtree {
        /// Stable token describing the violation category.
        token: &'static str,
        /// Logical bytenr where the violation was observed.
        logical: u64,
    },

    /// Path resolution failed because a component does not exist.
    NotFound,

    /// Path resolved but does not point at a regular file.
    NotARegularFile,

    /// Path resolved but does not point at a symbolic link.
    NotASymlink,

    /// Decompression of a compressed extent failed.
    BadCompression {
        /// Stable token: `comp_zlib` / `comp_lzo` / `comp_zstd`.
        algorithm: &'static str,
    },

    /// Allocation failed during a read-path operation.
    OutOfMemory {
        /// Stable token describing the allocation site.
        token: &'static str,
    },
}

/// Why the superblock was rejected. Present as a sub-enum so callers that
/// only care about the boundary can match on `Error::BadSuperblock(_)`
/// without enumerating the inner variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperblockReason {
    /// Magic field did not match the canonical `_BHRfS_M`.
    BadMagic,
    /// CRC32C over the superblock body did not match the stored csum.
    BadCsum,
    /// All four superblock copies disagreed irreconcilably on generation.
    GenerationSkew,
    /// All known superblock locations failed to read or validate.
    NoValidCopy,
    /// Superblock claims a `csum_type` other than CRC32C.
    UnsupportedCsumType,
    /// Superblock claims an `INCOMPAT` flag the crate does not implement.
    UnsupportedIncompat,
    /// `nodesize` / `sectorsize` outside the supported range.
    BadGeometry,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { token, offset } => {
                write!(f, "io error ({token}) at offset {offset}")
            }
            Self::BadSuperblock(reason) => write!(f, "bad superblock: {reason:?}"),
            Self::UnsupportedFeature(t) => write!(f, "unsupported feature: {t}"),
            Self::UnsupportedProfile(t) => write!(f, "unsupported chunk profile: {t}"),
            Self::UnsupportedChecksum(t) => write!(f, "unsupported checksum type: {t}"),
            Self::CsumMismatch { logical } => {
                write!(f, "metadata csum mismatch at logical {logical:#x}")
            }
            Self::CorruptBtree { token, logical } => {
                write!(f, "corrupt b-tree ({token}) at logical {logical:#x}")
            }
            Self::NotFound => write!(f, "path component not found"),
            Self::NotARegularFile => write!(f, "path does not resolve to a regular file"),
            Self::NotASymlink => write!(f, "path does not resolve to a symlink"),
            Self::BadCompression { algorithm } => write!(f, "bad compressed extent ({algorithm})"),
            Self::OutOfMemory { token } => write!(f, "out of memory ({token})"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
