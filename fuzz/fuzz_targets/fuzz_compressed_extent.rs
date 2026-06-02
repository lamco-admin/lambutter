// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Fuzz the compression decoders directly via the `__fuzz_internals`
//! re-export. The first byte selects algorithm (modulo 4: NONE, ZLIB, LZO,
//! ZSTD), the rest is the compressed payload. Bugs surface as panics,
//! out-of-bounds reads, runaway allocation, or output past the 16 MiB cap.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.is_empty() {
        return;
    }
    let algorithm = data[0] % 4;
    let payload = &data[1..];
    let mut dst = Vec::new();
    let _ = lambutter::__fuzz_internals::decode(algorithm, payload, &mut dst);
});
