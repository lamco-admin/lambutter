// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Fuzz the EXTENT_DATA reader by feeding arbitrary file-extent items
//! through the public read_file path. Bugs surface as panics, OOB reads,
//! or runaway allocations.

#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    if data.len() < 0x10000 + 65536 {
        return;
    }
    let reader: &[u8] = data;
    if let Ok(mut fs) = lambutter::Btrfs::open(reader, data.len() as u64) {
        // Try a handful of plausible /boot paths through both APIs.
        for raw in [
            b"/vmlinuz-current".as_ref(),
            b"/initrd.img".as_ref(),
            b"/loader/entries/x.conf".as_ref(),
        ] {
            if let Some(p) = lambutter::Path::new(raw) {
                let _ = fs.read_file(p);
                if let Ok(inode) = fs.resolve(p) {
                    let mut buf = [0u8; 1024];
                    let _ = fs.read_file_at(&inode, 0, &mut buf);
                    let _ = fs.read_file_at(&inode, 1 << 30, &mut buf);
                    let _ = fs.read_link(p);
                }
            }
        }
    }
});
