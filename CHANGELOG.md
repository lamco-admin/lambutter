# Changelog

All notable changes to lambutter are documented here. Format inspired by
Keep a Changelog; semantic versioning is loose during pre-1.0 (additions
are permitted within a minor version line; breaking changes bump minor
since pre-1.0).

**A note on version numbering:** lambutter has not been published to
crates.io. The version arc below narrates the development phases as
distinct conceptual releases — each represents a maturity milestone with
its own scope. Tags exist in git for `v0.1.0`, `v0.1.1`, and `v0.3.0`;
intermediate version numbers (`v0.1.2` through `v0.2.0`) document phases
of work that did not get separate publishable releases but are useful as
narrative milestones for anyone reading this file later.

## [0.3.0] — Comprehensive consumer documentation + live embedded validation

The first release marketed as "production-ready for embedded
consumers." Two categories of work since v0.2.0:

### Added — Documentation suite (~1400 lines across 5 docs)

- **`docs/FEATURES.md`** — exhaustive feature inventory. Every on-disk
  format capability with status + implementation site; full public API
  surface; the 11 error variants with stable tokens; resource bounds
  (`MAX_TREE_DEPTH`, `MAX_DECOMPRESSED_EXTENT_BYTES`); test posture per
  layer; audit hooks for downstream consumers; deliberate non-features
  with rationale.
- **`docs/SUPPORTED-SCENARIOS.md`** — distro × btrfs-config coverage
  matrix. 15 distros enumerated with default-config status; every
  btrfs profile (SINGLE/DUP/RAID variants + the rejected ones);
  snapshot scenarios; compression coverage; every INCOMPAT flag's
  handling; distro-specific gotchas (openSUSE Snapper redirect,
  Tumbleweed pre/post-grub2-bls layouts, Fedora Silverblue ostree).
- **`docs/CONSUMER-GUIDE.md`** — how to integrate lambutter from
  another `no_std` Rust project. Dependency declaration with feature-
  selection rationale; three worked `BlockRead` impl examples (host
  `std::fs::File`-backed, UEFI `EFI_BLOCK_IO_PROTOCOL`-backed, `&[u8]`
  slice-backed); the symlink-follow pattern (with POSIX
  `SYMLOOP_MAX = 40` standard); error-translation pattern; memory-
  management considerations; concurrency posture; versioning + upgrade
  discipline. References LamBoot's
  `lamboot-core/src/fs_backend_btrfs.rs` as the production reference
  implementation.
- **`docs/TROUBLESHOOTING.md`** — symptom → cause → resolution →
  prevention catalogue for every error variant the crate can produce.
  Indexed by what the developer/operator sees first. Sections cover
  `Btrfs::open` errors, path-resolution errors, compressed-extent
  errors, I/O errors, build/dependency issues, UEFI-context-specific
  issues, and surprising-but-correct edge cases (empty `read_dir` on
  nested-subvol mountpoints, NotFound on cross-snapshot paths).
- **`README.md` rewrite** — reflects validation state, points at the
  doc suite, adds Cargo feature table, includes both `read_file` and
  `read_file_at` usage examples, references the LamBoot integration
  case study as live-validation evidence.

### Added — Live embedded validation (the real-world milestone)

Lambutter is now consumed by [LamBoot](https://lamboot.dev) (Lamco's
UEFI bootloader) as the native btrfs backend for kernel reads. The
integration was validated end-to-end on openSUSE Tumbleweed VM 102 on
`pve.a.lamco.io`:

- BIOS→UEFI migration via `lamboot-migrate to-uefi --method C --bootloader lamboot`
- LamBoot loaded under live OVMF UEFI firmware
- Lambutter mounted the actual `/dev/sda2` btrfs partition with
  Snapper-managed default-subvol redirect (default subvol = OID 266 =
  `@/.snapshots/1/snapshot`)
- Lambutter resolved the kernel symlink chain
  (`/boot/vmlinuz-6.19.12-1-default` → `../usr/lib/modules/6.19.12-1-default/vmlinuz`)
  through consumer-side symlink-follow logic
- Kernel bytes returned by lambutter were consumed by LamBoot's
  native PE loader and the kernel booted Tumbleweed userspace cleanly

Full audit trail in LamBoot's boot-trust.log includes
`backend=lambutter@0.3.0-path` propagated through `volume_mounted`,
`image_verified` (with byte-identical sha256
`9110fc6ac140a57d34d80689345aa6398c1f99ef6948ea586e653eb389aeebe1`),
and `image_loaded_native` events.

Full case study including the 10 bugs found and fixed during this
test:
[`lamboot-dev/docs/migration/VM-102-OPENSUSE-TUMBLEWEED-BIOS-TO-UEFI-CASE-STUDY.md`](https://github.com/lamco-admin/lamboot-dev/blob/main/docs/migration/VM-102-OPENSUSE-TUMBLEWEED-BIOS-TO-UEFI-CASE-STUDY.md).

### Bumped

- Crate version 0.1.1 → 0.3.0 (skipping intermediate numbers per
  version-numbering note at top — the conceptual arc is documented
  below but no intermediate releases were tagged).
- Backend-tag string consumers track now reads `lambutter@0.3.0-...`
  for trust-log attribution.

### Test totals — unchanged from v0.2.0

- 42 host unit tests
- 9 fixture-based oracle tests
- 5 fuzz harnesses with seed corpus + 1-hour smoke run zero crashes
- Live host validation: 6 reads against VM 102 sda2, all sha256-
  identical to kernel oracle
- Live embedded validation: 1 successful boot of real Tumbleweed via
  LamBoot, full trust-chain audit captured

## [0.2.0] — Live host validation against real-world btrfs

First end-to-end test of lambutter against a real, production-grade
distro install rather than synthetic fixtures.

### Validated

- `examples/inspect.rs` run against the live `/dev/sda2` btrfs
  partition of openSUSE Tumbleweed snapshot 20260422 on VM 102:
  - `default_subvol_objectid()` returned 266 — matching
    `btrfs subvolume get-default /`'s view, confirming the OID-6
    DIR_ITEM `"default"` resolution against a real Snapper-managed
    install (the single highest-risk scenario flagged in
    `BTRFS-CRATE-ANALYSIS-2026-04-27.md` §8)
  - `/boot/initrd-6.19.12-1-default` (38 MB regular file, multi-extent
    walk): sha256
    `d04dc72877b1d09b6d697ae94d3db9ecb2fca47e5c451328a1c0a1b04ca5e45c`
    — byte-identical to kernel oracle
  - `/boot/vmlinuz` (symlink): target `vmlinuz-6.19.12-1-default` —
    byte-identical to `readlink` from running system
  - `/boot/.vmlinuz.hmac` (symlink): target
    `.vmlinuz-6.19.12-1-default.hmac` — byte-identical
  - `/etc/os-release` (symlink): target `../usr/lib/os-release` —
    byte-identical
  - `/etc/hostname` (regular file, 11 bytes, inline extent):
    `"tumbleweed\n"` — byte-identical sha256
  - `/etc/fstab` (regular file, 1202 bytes): byte-identical sha256

This validation closes `docs/PRE-PUBLISH-AND-TESTING-PLAN.md` §2.1
(Tumbleweed) and §2.3 (default-subvol redirect, isolated) — the two
items the analysis doc flagged as HIGH×HIGH risk.

### Implications

- The crate's "synthetic fixtures pass" status is now backed by
  "real-distro reads also pass" — substantially stronger evidence
  base than v0.1.x.
- Confirms the symlink-follow pattern documented in
  `CONSUMER-GUIDE.md` §3 is necessary in practice (a downstream
  consumer that doesn't follow symlinks cannot read kernels from a
  kernel-install-managed distro).
- Establishes the canonical "live validation" methodology applied to
  all future distro test passes.

## [0.1.5] — Demo CLI + fuzz smoke completed

### Added

- **`examples/inspect.rs`** — host-side CLI example: opens a btrfs
  volume (block device or image file), reports the resolved default-
  subvol objectid, and either reads a named file (printing size +
  sha256 + first 16 bytes) or reads a symlink target. Uses
  `std::os::unix::fs::FileExt::read_exact_at` for positional reads,
  so it works against real block devices as well as image files.

  This is now the canonical real-world validation harness used by
  downstream consumers (and by lambutter itself for live validation
  per v0.2.0).

### Verified

- 1-hour fuzz smoke run completed on all 5 cargo-fuzz harnesses
  (`fuzz_superblock`, `fuzz_btree_node`, `fuzz_extent_data`,
  `fuzz_dir_item`, `fuzz_compressed_extent`) with seed corpus from
  the v0.1.4 fixture set. **Zero crashes.** Closes
  `docs/TESTING-AND-FUZZING-PLAN.md` §6.3.

## [0.1.4] — Pre-publish hygiene + lint cleanup

### Fixed (hygiene)

- F3 zlib fixture test `#[cfg(feature = "zlib")]`-gated so
  `cargo test --release` (no features) passes. F2 zstd test similarly
  gated.
- 67 release-build warnings → 0. Added `#[expect(dead_code, reason =
  "...")]` at module level to `format/constants.rs` and
  `format/repr.rs` documenting why every spec constant / parsed field
  exists even when no v0.1.x caller reads it. Genuinely-unused items
  removed: `util::align_up`, `Path::from_bytes_unchecked`,
  `ChunkMap::new`, `ChunkMap::len`, `btree::LeafIter`. Per-field
  `#[expect]` annotations added to `ChunkMapping` (RAID0/10/5/6-only
  fields) and `Resolved::devid` (single-device-only).
- Stale `v0.1.0` comments in `src/btree.rs` and `src/dir.rs` claiming
  cross-leaf iteration is unimplemented (it has been since v0.1.0)
  deleted and rewritten to describe the re-descend pattern actually
  in use.
- README status section rewritten for public-crate audience; added
  usage example showing both `read_file` and `read_file_at` patterns.

### Verified

- **UEFI target compiles clean** for `x86_64-unknown-uefi` AND
  `aarch64-unknown-uefi` across the full feature matrix
  (no-default-features, zstd-only, all three compression features
  combined) using
  `rustup run nightly cargo build --target ... -Zbuild-std=core,compiler_builtins,alloc -Zbuild-std-features=compiler-builtins-mem`.
- `cargo build --release` reports **0 warnings** across
  no-default-features / default-features / all-features.
- `cargo clippy --release --all-features` reports **0 warnings** on
  the library; only progressive `unwrap_used` / `expect_used`
  warnings remain in tests (per `Cargo.toml` `[lints.clippy]` design).
- `rustup run nightly cargo fmt --check` clean.

## [0.1.3] — Hardening pass (B1–B5)

Defense-in-depth response to a deep code-review audit. Each item
addresses a specific class of DoS or correctness vulnerability the
crate previously didn't guard against.

### Fixed (correctness / hardening)

- **B1 — chunk-tree overflow.** `ChunkMap::insert` and `::resolve` now
  use `checked_add` for `logical + length` arithmetic; a fuzz-only
  input with `length = u64::MAX` would previously wrap past the
  overlap-detection guard. New error token: `chunk_overflow`.
- **B2 — gap-fill denial-of-service.** `file::read_file` capped
  EXTENT_DATA gap-fill at the file size and routed the allocation
  through `Vec::try_reserve` instead of `vec![0u8; gap]`. A malformed
  extent at `key.offset = 2^60` could previously trigger a 2^60-byte
  allocation attempt. New error token: `extent_past_eof`.
- **B3 — compressed extent disk-size cap.** `file::apply_extent` and
  `file::copy_extent_slice` reject `disk_num_bytes` exceeding
  `MAX_DECOMPRESSED_EXTENT_BYTES` (16 MiB) before allocating the
  compressed scratch buffer. A compressed extent's on-disk size cannot
  legitimately exceed its plaintext size, so 16 MiB is correct. New
  error token: `comp_oversized`.
- **B4 — `symlink_long` mistyped.** `file::read_link` reported
  `Error::CorruptBtree { token: "symlink_not_inline" }` for legal-but-
  rare symlinks whose target is stored in a regular extent. Now
  reports `Error::UnsupportedFeature("symlink_long")`, which is the
  correct category.
- **B5 — leaf key-order validation.** `btree::read_tree_block` now
  validates strict ascending order over every key in a freshly-loaded
  tree block (interior keyptrs or leaf items). The `key_order` token
  was in the stable vocabulary since v0.1.0 but no code path ever
  emitted it; a corrupt leaf with out-of-order keys would silently
  mis-resolve.
- File-content read paths use `out.resize` against the up-front
  `try_reserve_exact`'d capacity instead of `vec![0u8; n]
  .extend_from_slice` — one allocation per region rather than two.
  Side benefit: lower peak RSS for sparse files.

### Test totals after this phase

- 42 host unit tests (was 36) — new tests for chunk-overflow detection
  (`chunk_overflow` token), leaf-ordering detection (`key_order`
  token), extent-logical-length parsers, plaintext-cap regression
  lock.

## [0.1.2] — `read_file_at` API + spec sync

### Added

- **`Btrfs::read_file_at(&inode, offset, &mut buf)`** — chunked
  file-read API. Memory cost is bounded at the size of one extent +
  the caller's buffer, so bootloaders streaming a kernel image /
  initrd no longer have to materialise the whole file in one
  `Vec<u8>`. Walks extents lazily, skipping those that end before the
  requested offset; decompresses per-extent only once; zero-fills
  `NO_HOLES` gaps and prealloc extents on the fly. Closes the spec
  §3.2 commitment that was unimplemented through v0.1.1.
- `__fuzz_internals` re-export (`#[doc(hidden)] #[cfg(fuzzing)]`)
  exposing the compression dispatcher, system-chunk-array parser, and
  name-hash for direct fuzz-harness drive. Closes
  `docs/TESTING-AND-FUZZING-PLAN.md` §6.4.
- `fuzz_compressed_extent` rewritten to call
  `__fuzz_internals::decode(algorithm, payload)` directly — no longer
  a placeholder that goes through `Btrfs::open` and almost never
  reaches the decoders.
- Seed corpus for all 5 fuzz targets populated from real
  `mkfs.btrfs`-produced fixtures (F1–F9) + targeted compressed-
  payload seeds.

### Changed

- Spec §3.2 + §11 amended to match implementation: documented
  `Btrfs::open(reader, device_size_bytes)` signature, `read_file_at`,
  `read_link`, `Vec<DirEntry>` return for `read_dir` (with rationale),
  and the full token vocabulary added since v0.1.0.

## [0.1.1] — Compression + fixture completion

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
  passing 0xFFFFFFFE did NOT load that value into the running
  register. Replaced with a direct table-driven implementation
  matching python-btrfs and the kernel's `btrfs_name_hash` byte-for-
  byte.
- (Same status) zstd trailing-padding handling. btrfs zstd extents
  are sector-padded with zeros past the last frame; the decoder now
  checks the zstd frame magic before each iteration and stops when
  absent.

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

## [0.1.0] — Initial implementation (M2 through M10 of `docs/SPEC-LAMBUTTER.md`)

### Added

- Superblock loader + validator. Reads all four canonical superblock
  locations (0x10000, 0x4000000, 0x4000000000, 0x4000000000000),
  validates CRC32C body csum, picks the highest-generation valid
  copy. Rejects bad magic, bad csum, unsupported csum types (xxhash /
  sha256 / blake2), unsupported INCOMPAT flags (`ZONED`,
  `RAID_STRIPE_TREE`), and bad geometry (sectorsize / nodesize
  outside power-of-two range).
- Chunk-tree resolver. Two-phase: bootstrap from the superblock's
  embedded `sys_chunk_array`, then full walk of the chunk tree using
  the generic B-tree walker. Resolves logical → physical addresses
  for SINGLE / DUP / RAID1 / RAID1C3 / RAID1C4 profiles. Cleanly
  rejects RAID0 / RAID10 / RAID5 / RAID6 with typed errors carrying
  stable token vocabulary.
- Generic B-tree walker. Reads + verifies metadata-block CRC32C;
  binary-searches interior nodes; binary-searches leaves; supports
  exact-match (`find_exact`) and "first key ≥ target" (`find_first_ge`)
  modes. Bounded recursion depth via `MAX_TREE_DEPTH = 16` against
  malicious inputs.
- Item parsers: SuperBlock, Header, DiskKey, LeafItem, KeyPtr,
  ChunkItem, Stripe, InodeItem, RootItem, DirEntry, ExtentDataHeader,
  ExtentDataRegular. All parsers are `parse(&[u8], at) -> Option<T>`
  with bounds checks at every read.
- Root-tree walker + default-subvolume resolution. Implements the
  OID-6 DIR_ITEM `"default"` mechanism (the load-bearing path for
  Snapper rollback compatibility); falls back to superblock
  `root_dir_objectid` and ultimately to `FS_TREE_OBJECTID` (= 5).
- FS-tree path resolver. Walks `/`-separated path components, hash-
  looking-up each via DIR_ITEM keyed by `(parent_inode,
  DIR_ITEM_KEY, crc32c_with_seed(0xFFFF_FFFE, name))`. Handles hash
  collisions by walking the packed DirEntry tail.
- File-content reader. Walks an inode's EXTENT_DATA items in order;
  handles inline / regular / prealloc / hole extents; routes
  compressed extents through the compression dispatcher; pads holes
  to file size.
- Directory iterator. Walks DIR_INDEX items per inode, yielding owned
  `DirEntry { name, inode_number, kind_byte }` records.
- Compression decoders:
  - **zstd** via `ruzstd 0.7` (`default-features = false`). Streams
    frames; bounds output at `MAX_DECOMPRESSED_EXTENT_BYTES` (16 MiB).
  - **zlib** via `miniz_oxide 0.8` (`default-features = false`).
    `decompress_to_vec_zlib_with_limit` with the 16 MiB cap.
  - **LZO** outer-wrapper parser only; inner LZO1X-1 decode is
    **deferred to v0.1.1**. Encountering an LZO-compressed extent
    surfaces `Error::BadCompression { algorithm: "comp_lzo" }` rather
    than silently producing wrong data. Real-world prevalence on
    stock distros is essentially zero (Fedora 33+ / Tumbleweed /
    CachyOS / Garuda all default to zstd); shipping a known-correct
    decoder later is safer than shipping a possibly-wrong decoder
    now.
- Public API surface: `Btrfs<R>`, `BlockRead`, `Path`, `Inode`,
  `Metadata`, `DirEntry`, `Error`, `SuperblockReason`. Methods:
  `open`, `default_subvol_objectid`, `resolve`, `metadata`,
  `read_file`, `read_dir`.
- Fuzz harnesses (cargo-fuzz, `fuzz/`): `fuzz_superblock`,
  `fuzz_btree_node`, `fuzz_extent_data`, `fuzz_dir_item`,
  `fuzz_compressed_extent`. Each goes through the public API for
  scenario coverage.
- 34 host unit tests across all modules.
- Integrated-development framework configs
  (`~/lamco-admin/shared/integrated-development/`): `.rustfmt.toml`,
  `clippy.toml`, `.editorconfig`, `.githooks/pre-commit`,
  `[lints.*]` in `Cargo.toml`.
- Project `CLAUDE.md` mirrors the framework's generation-time rules
  adapted to this crate.

### Deferred to v0.1.1

- LZO1X-1 inner decoder.
- Cross-leaf path-component iteration for very large directories.
- Hardware-accelerated CRC32C (SSE 4.2 / ARMv8 CRC).
- LamBoot integration (`lamboot-core/src/fs_backend_btrfs.rs`) — the
  consuming side lives in `lamboot-dev` and ships with LamBoot
  v0.10.0.

### Known limitations

- Single-device read paths only. Multi-device btrfs volumes (RAID1
  with the read directed at the local device) work as long as the
  chunk's first stripe targets the local device's `devid`.
- Data-block csum verification (CSUM_TREE) is out of scope for
  v0.1.0; metadata-block csums are verified on every read.

---

## Version-progression policy

Going forward, each significant testing milestone may justify a
version bump even without an API change. Specifically:

- **Patch (`0.x.y+1`):** bug fixes, doc fixes, lint cleanup
- **Minor (`0.x+1.0`):** new API additions, significant capability
  expansions, milestone validation events (e.g. first live test on a
  new distro family)
- **Major (`0.y+1.0` — pre-1.0):** breaking API changes, scope
  expansions that materially change the consumer contract

When v1.0.0 ships, it will signal that the API is audit-stable —
specifically, that we commit to no breaking changes within the 1.x
line, and that the deferred-from-v0.1.x items (data-block CSUM
verification, snapshot enumeration, full subvolume traversal) have
either landed or been definitively scoped out.
