// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! The single trait callers implement to give lambutter access to a btrfs
//! volume's bytes. Lambutter never seeks or buffers internally — it requests
//! exact byte ranges starting at absolute offsets and expects the caller to
//! fill them.

use core::fmt::Debug;

/// A read-only block-level reader.
///
/// Implementations must:
/// - Fill `buf` with `buf.len()` bytes starting at `offset_bytes`.
/// - Return `Err(_)` rather than partial reads. If a partial read happens at
///   the underlying medium, the implementation should re-issue the read or
///   surface the failure.
///
/// `offset_bytes` is the absolute byte offset within the volume's logical
/// linear address space — i.e., what btrfs calls "physical bytenr" once
/// the chunk tree has resolved a logical address. The crate's chunk-tree
/// resolver translates btrfs logical bytenrs into physical offsets and
/// then calls `read_at`.
pub trait BlockRead {
    /// The error type returned by the implementation on read failure.
    /// Lambutter erases this to a `&'static str` token at the boundary.
    type Error: Debug;

    /// Read exactly `buf.len()` bytes starting at `offset_bytes`. The buffer
    /// is fully populated on success.
    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8]) -> Result<(), Self::Error>;
}

/// Adapter implementation for `&[u8]` slices. Useful for tests and for
/// in-memory volume images.
impl BlockRead for &[u8] {
    type Error = SliceReadError;

    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8]) -> Result<(), Self::Error> {
        let start: usize = offset_bytes
            .try_into()
            .map_err(|_| SliceReadError::OffsetTooLarge)?;
        let end = start
            .checked_add(buf.len())
            .ok_or(SliceReadError::Overflow)?;
        if end > self.len() {
            return Err(SliceReadError::OutOfBounds);
        }
        buf.copy_from_slice(&self[start..end]);
        Ok(())
    }
}

/// Error variants for the `&[u8]` adapter. Test/diagnostic only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceReadError {
    /// Requested offset doesn't fit in `usize`.
    OffsetTooLarge,
    /// Read range arithmetic overflowed.
    Overflow,
    /// Range extends past slice length.
    OutOfBounds,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slice_adapter_reads_in_bounds() {
        let data: &[u8] = &[1, 2, 3, 4, 5, 6, 7, 8];
        let mut reader: &[u8] = data;
        let mut buf = [0u8; 4];
        reader.read_at(2, &mut buf).unwrap();
        assert_eq!(&buf, &[3, 4, 5, 6]);
    }

    #[test]
    fn slice_adapter_rejects_out_of_bounds() {
        let data: &[u8] = &[1, 2, 3];
        let mut reader: &[u8] = data;
        let mut buf = [0u8; 4];
        assert_eq!(
            reader.read_at(0, &mut buf),
            Err(SliceReadError::OutOfBounds)
        );
    }
}
