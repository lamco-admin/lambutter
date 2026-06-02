// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Fuzz the DIR_ITEM parser via path resolution. Path components drive
//! `crc32c_with_seed` lookups against arbitrary on-disk DirEntry packings,
//! exercising the hash-collision unpacker.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 0x10000 + 65536 {
        return;
    }
    let reader: &[u8] = data;
    if let Ok(mut fs) = lambutter::Btrfs::open(reader, data.len() as u64) {
        for raw in [
            b"/a".as_ref(),
            b"/a/b".as_ref(),
            b"/a/b/c".as_ref(),
            b"/loader/entries/x.conf".as_ref(),
        ] {
            if let Some(p) = lambutter::Path::new(raw) {
                let _ = fs.resolve(p);
            }
        }
    }
});
