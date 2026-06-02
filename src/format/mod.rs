// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! On-disk format primitives. Constants, flag bit definitions, and parsers
//! for fixed-layout structs. No public surface — consumers use higher-level
//! modules (`superblock`, `chunk_tree`, `btree`, `items`, etc.).

pub(crate) mod constants;
pub(crate) mod flags;
pub(crate) mod repr;
