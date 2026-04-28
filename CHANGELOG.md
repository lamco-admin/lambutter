# Changelog

All notable changes to lambutter are documented here. Format inspired by
Keep a Changelog; semantic versioning is loose during pre-1.0.

## [Unreleased]

## [0.1.1]

Filling the v0.1.0 deferred-list per
`docs/TESTING-AND-FUZZING-PLAN.md` §9.

### Added

- LZO decompression (real, not just outer-wrapper): `comp_lzo` extents
  now decode correctly through `lzokay` v2.0.1 (MIT, no_std, pure
  Rust). Verified against fixture F4 produced by `mkfs.btrfs --rootdir`
  + `mount -o compress-force=lzo`.
- Fixtures F3 (zlib), F4 (LZO), F5 (DUP metadata + SINGLE data — the
  mkfs.btrfs default for SSDs), F8 (NO_HOLES sparse file with a 1 MiB
  hole between two extents). All committed at `tests/fixtures/data/`
  with their `.expected.json` and `.sha256` companions.
- Fixture-test harness now exercises 8 scenarios (was 4 in v0.1.0).
  Coverage matrix:
  - F1: SINGLE uncompressed
  - F2: SINGLE zstd
  - F3: SINGLE zlib
  - F4: SINGLE LZO
  - F5: DUP metadata + SINGLE data
  - F8: NO_HOLES sparse file
  - F9: symlinks (relative + absolute)
  - random-bytes-doesn't-panic smoke

### Fixed

- (Already fixed in 0.1.0 release branch but documented here for the
  ledger) DIR_ITEM name-hash bug: the `crc` crate's
  `digest_with_initial(seed)` reflects the seed when `refin=true`, so
  passing 0xFFFFFFFE did NOT load that value into the running register.
  Replaced with a direct table-driven implementation matching
  python-btrfs and the kernel's `btrfs_name_hash` byte-for-byte.
- (Same status) zstd trailing-padding handling. btrfs zstd extents are
  sector-padded with zeros past the last frame; the decoder now checks
  the zstd frame magic before each iteration and stops when absent.

### Changed

- `compression/lzo.rs` now dispatches sector payloads through
  `lzokay::decompress::decompress`. The outer btrfs sector wrapper
  parser is unchanged.

### Dependencies

- Added `lzokay = { version = "2.0", default-features = false,
  features = ["decompress", "alloc"], optional = true }`. Feature
  `lzo` now activates this dependency. Total compression-decoder
  posture: ruzstd (zstd, default), miniz_oxide (zlib), lzokay (LZO) —
  three pure-Rust no_std decoders, each gated behind its own feature.

### Test totals

- 36 host unit tests (unchanged)
- 8 fixture-based oracle tests (was 4)
- 5 fuzz harnesses (unchanged)



### Added — initial implementation (M2 through M10 of `docs/SPEC-LAMBUTTER.md`)

- Superblock loader + validator. Reads all four canonical superblock
  locations (0x10000, 0x4000000, 0x4000000000, 0x4000000000000), validates
  CRC32C body csum, picks the highest-generation valid copy. Rejects bad
  magic, bad csum, unsupported csum types (xxhash / sha256 / blake2),
  unsupported INCOMPAT flags (`ZONED`, `RAID_STRIPE_TREE`), and bad
  geometry (sectorsize / nodesize outside power-of-two range).
- Chunk-tree resolver. Two-phase: bootstrap from the superblock's
  embedded `sys_chunk_array`, then full walk of the chunk tree using the
  generic B-tree walker. Resolves logical → physical addresses for
  SINGLE / DUP / RAID1 / RAID1C3 / RAID1C4 profiles. Cleanly rejects
  RAID0 / RAID10 / RAID5 / RAID6 with typed errors carrying stable token
  vocabulary.
- Generic B-tree walker. Reads + verifies metadata-block CRC32C; binary-
  searches interior nodes; binary-searches leaves; supports exact-match
  (`find_exact`) and "first key ≥ target" (`find_first_ge`) modes.
  Bounded recursion depth via `MAX_TREE_DEPTH = 16` against malicious
  inputs.
- Item parsers: SuperBlock, Header, DiskKey, LeafItem, KeyPtr, ChunkItem,
  Stripe, InodeItem, RootItem, DirEntry, ExtentDataHeader,
  ExtentDataRegular. All parsers are `parse(&[u8], at) -> Option<T>` with
  bounds checks at every read.
- Root-tree walker + default-subvolume resolution. Implements the OID-6
  DIR_ITEM `"default"` mechanism (the load-bearing path for Snapper
  rollback compatibility); falls back to superblock `root_dir_objectid`
  and ultimately to `FS_TREE_OBJECTID` (= 5).
- FS-tree path resolver. Walks `/`-separated path components,
  hash-looking-up each via DIR_ITEM keyed by
  `(parent_inode, DIR_ITEM_KEY, crc32c_with_seed(0xFFFF_FFFE, name))`.
  Handles hash collisions by walking the packed DirEntry tail.
- File-content reader. Walks an inode's EXTENT_DATA items in order;
  handles inline / regular / prealloc / hole extents; routes compressed
  extents through the compression dispatcher; pads holes to file size.
- Directory iterator. Walks DIR_INDEX items per inode, yielding owned
  `DirEntry { name, inode_number, kind_byte }` records.
- Compression decoders:
  - **zstd** via `ruzstd 0.7` (`default-features = false`). Streams
    frames; bounds output at `MAX_DECOMPRESSED_EXTENT_BYTES` (16 MiB).
  - **zlib** via `miniz_oxide 0.8` (`default-features = false`).
    `decompress_to_vec_zlib_with_limit` with the 16 MiB cap.
  - **LZO** outer-wrapper parser only; inner LZO1X-1 decode is **deferred
    to v0.1.1**. Encountering an LZO-compressed extent surfaces
    `Error::BadCompression { algorithm: "comp_lzo" }` rather than
    silently producing wrong data. Real-world prevalence on stock
    distros is essentially zero (Fedora 33+ / Tumbleweed / CachyOS /
    Garuda all default to zstd); shipping a known-correct decoder later
    is safer than shipping a possibly-wrong decoder now.
- Public API surface: `Btrfs<R>`, `BlockRead`, `Path`, `Inode`,
  `Metadata`, `DirEntry`, `Error`, `SuperblockReason`. Methods: `open`,
  `default_subvol_objectid`, `resolve`, `metadata`, `read_file`,
  `read_dir`.
- Fuzz harnesses (cargo-fuzz, `fuzz/`): `fuzz_superblock`,
  `fuzz_btree_node`, `fuzz_extent_data`, `fuzz_dir_item`,
  `fuzz_compressed_extent`. Each goes through the public API for
  scenario coverage.
- 34 host unit tests across all modules.
- Integrated-development framework configs (`~/lamco-admin/shared/
  integrated-development/`): `.rustfmt.toml`, `clippy.toml`,
  `.editorconfig`, `.githooks/pre-commit`, `[lints.*]` in `Cargo.toml`.
- Project `CLAUDE.md` mirrors the framework's generation-time rules
  adapted to this crate.

### Deferred to v0.1.1

- LZO1X-1 inner decoder.
- Cross-leaf path-component iteration for very large directories.
- Hardware-accelerated CRC32C (SSE 4.2 / ARMv8 CRC).
- LamBoot integration (`lamboot-core/src/fs_backend_btrfs.rs`) — the
  consuming side lives in `lamboot-dev` and ships with LamBoot v0.10.0.

### Known limitations

- Single-device read paths only. Multi-device btrfs volumes (RAID1 with
  the read directed at the local device) work as long as the chunk's
  first stripe targets the local device's `devid`.
- Data-block csum verification (CSUM_TREE) is out of scope for v0.1.0;
  metadata-block csums are verified on every read.
