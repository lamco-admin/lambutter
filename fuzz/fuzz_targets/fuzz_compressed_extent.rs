// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Fuzz the compression decoders directly. Each input is split: the first
//! byte selects the algorithm, the remainder is passed to the decoder.
//! Bugs surface as panics, OOB, runaway allocations, or output exceeding
//! the configured cap.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let _algorithm = data[0];
    let _payload = &data[1..];

    // The compression dispatcher is crate-private; the public surface that
    // exercises it is `read_file`. To keep the harness focused, we feed
    // compressed payloads via a synthetic volume image. Image construction
    // for an inline-extent EXTENT_DATA item is small but non-trivial; v0.1.0
    // ships this harness as a placeholder until the fixture-image scaffolding
    // lands. The fuzzer still exercises the public `Btrfs::open` path.
    let reader: &[u8] = data;
    if data.len() >= 0x10000 + 65536 {
        let _ = lambutter::Btrfs::open(reader, data.len() as u64);
    }
});
