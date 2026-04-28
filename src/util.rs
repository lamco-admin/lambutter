// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Small primitives used across the crate. Kept tight on purpose; if a
//! helper grows beyond a few lines it belongs in its domain module.

#[inline]
pub(crate) fn read_le_u16(buf: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([buf[offset], buf[offset + 1]])
}

#[inline]
pub(crate) fn read_le_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
    ])
}

#[inline]
pub(crate) fn read_le_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        buf[offset],
        buf[offset + 1],
        buf[offset + 2],
        buf[offset + 3],
        buf[offset + 4],
        buf[offset + 5],
        buf[offset + 6],
        buf[offset + 7],
    ])
}

#[inline]
pub(crate) fn read_le_u8(buf: &[u8], offset: usize) -> u8 {
    buf[offset]
}

/// Round `value` up to the next multiple of `align`. `align` must be a power
/// of two; debug builds assert this. Caller is expected to pass btrfs-spec
/// alignments (sector size, node size).
#[inline]
pub(crate) fn align_up(value: u64, align: u64) -> u64 {
    debug_assert!(align.is_power_of_two(), "alignment must be a power of two");
    (value + align - 1) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_le_round_trip() {
        let buf = [0x78, 0x56, 0x34, 0x12, 0xff, 0xee, 0xdd, 0xcc];
        assert_eq!(read_le_u16(&buf, 0), 0x5678);
        assert_eq!(read_le_u32(&buf, 0), 0x1234_5678);
        assert_eq!(read_le_u64(&buf, 0), 0xccdd_eeff_1234_5678);
    }

    #[test]
    fn align_up_basic() {
        assert_eq!(align_up(0, 4096), 0);
        assert_eq!(align_up(1, 4096), 4096);
        assert_eq!(align_up(4096, 4096), 4096);
        assert_eq!(align_up(4097, 4096), 8192);
    }
}
