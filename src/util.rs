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
}
