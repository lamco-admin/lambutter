// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Fuzz the superblock loader by feeding arbitrary bytes through the
//! public `Btrfs::open` entry point. The harness ensures no panic,
//! out-of-bounds, or infinite-loop bug exists in the superblock
//! validator path under malformed inputs.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Pad to the primary superblock offset + a superblock so the loader
    // has somewhere to read from. Anything past that is the input under
    // test.
    if data.len() < 0x10000 + 4096 {
        return;
    }
    let reader: &[u8] = data;
    let _ = lambutter::Btrfs::open(reader, data.len() as u64);
});
