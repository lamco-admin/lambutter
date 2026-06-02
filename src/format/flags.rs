// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Superblock feature-flag bit constants.
//!
//! `incompat_flags` is the load-bearing one for a read-only consumer: any
//! bit set in the on-disk superblock that the crate does not know how to
//! tolerate is a hard reject. `compat_flags` and `compat_ro_flags` are
//! advisory for read-only access — the crate ignores `compat` entirely and
//! treats `compat_ro` as informational since we never write.

use bitflags::bitflags;

bitflags! {
    /// Bits in `superblock.incompat_flags`. Any bit not listed here that is
    /// set on disk is a hard reject (`Error::UnsupportedFeature`).
    /// spec: BTRFS-FORMAT-READONLY-REFERENCE §11 (incompat flag tolerance matrix)
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct IncompatFlags: u64 {
        const MIXED_BACKREF       = 1 << 0;
        const DEFAULT_SUBVOL      = 1 << 1;
        const MIXED_GROUPS        = 1 << 2;
        const COMPRESS_LZO        = 1 << 3;
        const COMPRESS_ZSTD       = 1 << 4;
        const BIG_METADATA        = 1 << 5;
        const EXTENDED_IREF       = 1 << 6;
        const RAID56              = 1 << 7;
        const SKINNY_METADATA     = 1 << 8;
        const NO_HOLES            = 1 << 9;
        const METADATA_UUID       = 1 << 10;
        const RAID1C34            = 1 << 11;
        const ZONED               = 1 << 12;
        const EXTENT_TREE_V2      = 1 << 13;
        const RAID_STRIPE_TREE    = 1 << 14;
        const SIMPLE_QUOTA        = 1 << 16;
    }
}

impl IncompatFlags {
    /// Bits the crate refuses to mount under in v0.1.0.
    /// `ZONED` requires zoned-block-device awareness; `RAID_STRIPE_TREE`
    /// changes how chunk addressing works; both are out of scope.
    /// spec: BTRFS-FORMAT-READONLY-REFERENCE §11
    pub(crate) const fn rejected_for_v0_1() -> Self {
        Self::ZONED.union(Self::RAID_STRIPE_TREE)
    }
}

bitflags! {
    /// Bits in `superblock.compat_flags`. Reserved for future use; the crate
    /// ignores all of them since none currently affect read-only access.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CompatFlags: u64 {
        const _RESERVED = 0;
    }
}

bitflags! {
    /// Bits in `superblock.compat_ro_flags`. Treated as advisory by a
    /// read-only consumer; recorded for completeness.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CompatRoFlags: u64 {
        const FREE_SPACE_TREE       = 1 << 0;
        const FREE_SPACE_TREE_VALID = 1 << 1;
        const VERITY                = 1 << 2;
        const BLOCK_GROUP_TREE      = 1 << 3;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_v0_1_covers_zoned_and_raid_stripe_tree() {
        let r = IncompatFlags::rejected_for_v0_1();
        assert!(r.contains(IncompatFlags::ZONED));
        assert!(r.contains(IncompatFlags::RAID_STRIPE_TREE));
        assert!(!r.contains(IncompatFlags::COMPRESS_ZSTD));
        assert!(!r.contains(IncompatFlags::SKINNY_METADATA));
    }

    #[test]
    fn unknown_bit_is_unknown() {
        let raw: u64 = 1 << 30;
        let parsed = IncompatFlags::from_bits_truncate(raw);
        assert!(parsed.is_empty(), "bit 30 is not yet defined");
    }
}
