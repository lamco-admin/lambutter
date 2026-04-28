// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Fuzz the B-tree walker via the public read-file path. Each iteration
//! constructs an arbitrary "volume" image and asks lambutter to read a
//! path. The walker descends through whatever tree structure the bytes
//! describe; bugs surface as panics or infinite loops.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 0x10000 + 65536 {
        return;
    }
    let reader: &[u8] = data;
    if let Ok(mut fs) = lambutter::Btrfs::open(reader, data.len() as u64) {
        if let Some(p) = lambutter::Path::new(b"/etc/passwd") {
            let _ = fs.read_file(p);
        }
    }
});
