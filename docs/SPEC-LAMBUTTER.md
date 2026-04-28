# SPEC-LAMBUTTER — Design Specification

**Crate:** `lambutter`
**Version targeted:** 0.1.0 (initial public release)
**Status:** DRAFT — pending founder sign-off
**Created:** 2026-04-27
**Authority:** This document and `~/lamboot-dev/docs/analysis/BTRFS-FORMAT-READONLY-REFERENCE-2026-04-27.md` together specify the entire crate. The format reference is the on-disk-format authority; this document is the implementation authority.

This is the design spec. It is signed off before any implementation begins.

---

## 1. Purpose

Lambutter is a `no_std` + `alloc` read-only btrfs filesystem reader. It exists so that UEFI bootloaders (LamBoot in particular) and other pre-OS contexts can read kernel images, initrds, BLS entries, and boot configuration directly from btrfs partitions without depending on third-party UEFI filesystem drivers, on `std`, or on the Linux kernel's btrfs driver.

Lambutter is the btrfs counterpart to `ext4-view`. It is intentionally narrow in scope: read-only, single-volume btrfs, the subset of the format actually present on `/boot` paths of stock distributions.

## 2. Scope

### 2.1 In scope for v0.1.0

- Open a btrfs volume backed by a caller-supplied block reader
- Resolve the active subvolume via `default_subvol_id` from the superblock OR via the DIR_ITEM named `"default"` at OID 6 in `ROOT_TREE` (see §6.4 — the OID-6 mechanism is what GRUB uses and what Snapper updates on rollback)
- Resolve a path within the active subvolume to an inode
- Read a regular file's contents, including:
  - inline data (data stored directly in `EXTENT_DATA`)
  - regular extents (data resolved via the chunk tree)
  - prealloc extents (read as zeros — they are conceptually unwritten)
  - holes (read as zeros — both `EXTENT_DATA hole` items and gaps under `NO_HOLES`)
  - compressed extents: zstd, zlib, LZO
- Iterate directory entries
- Verify metadata-block checksums on every read (CRC32C, the only csum type required by §13 of the format reference)
- Support RAID profiles: SINGLE, DUP, RAID1, RAID1C3, RAID1C4
- Cleanly reject (with a typed error) RAID0, RAID10, RAID5, RAID6 — these profiles are explicitly out of scope for v0.1.0

### 2.2 Out of scope for v0.1.0

- All write paths
- Snapshot enumeration (only the active default subvolume is reachable)
- Mid-path subvolume crossings (per BTRFS-FORMAT-READONLY-REFERENCE §13)
- Csum verification on file *data* (verifying *metadata* csums is in scope; data-csum verification via `CSUM_TREE` is deferred — the cost/benefit analysis lives in the format reference open questions)
- Free-space tree consumption — read-only paths must never consult it
- Extended attributes (`XATTR_ITEM`)
- Reflinks, quotas, send-stream parsing
- Hardware-accelerated CRC32C (SSE 4.2 / ARMv8 CRC) — software implementation only in v0.1.0; can be added behind a feature flag later
- Anything `std`-bound

### 2.3 v0.2.0+ provisional roadmap

Listed for context, not committed:

- v0.2.0: snapshot enumeration; full subvolume traversal
- v0.3.0: data-csum verification opt-in
- v0.4.0: RAID0 / RAID10 read support
- v0.5.0: hardware-accelerated CRC32C behind a feature flag

## 3. Public API

### 3.1 Top-level types

```rust
/// A mounted, read-only btrfs filesystem.
pub struct Btrfs<R: BlockRead> { /* opaque */ }

/// A trait implemented by callers to provide block-level reads.
/// Lambutter never seeks; it asks for a byte range starting at an
/// absolute offset and expects the caller to fill the buffer.
pub trait BlockRead {
    /// Read exactly `buf.len()` bytes starting at `offset_bytes`.
    /// Errors are reported via the associated `Error` type.
    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8]) -> Result<(), Self::Error>;
    type Error: core::fmt::Debug;
}

/// Path within the active subvolume. Always absolute (starts with `/`).
/// Borrowed; lambutter does not own paths.
pub struct Path<'a>(&'a [u8]);

/// An inode within the active subvolume.
pub struct Inode { /* opaque */ }

/// A directory iterator entry.
pub struct DirEntry<'a> {
    pub name: &'a [u8],
    pub kind: EntryKind,
    pub inode_number: u64,
}

pub enum EntryKind {
    Regular,
    Directory,
    Symlink,
    Other,
}
```

### 3.2 Methods

```rust
impl<R: BlockRead> Btrfs<R> {
    /// Open the filesystem. Reads the primary superblock, validates
    /// it, replays the chunk tree's bootstrap, walks the root tree to
    /// find the default subvolume, and returns a handle.
    pub fn open(reader: R) -> Result<Self, Error>;

    /// Resolve a path to an inode within the active subvolume.
    pub fn resolve(&mut self, path: Path<'_>) -> Result<Inode, Error>;

    /// Read a file's full contents into a freshly allocated `Vec<u8>`.
    /// Decompresses on the fly if the file's extents are compressed.
    pub fn read_file(&mut self, path: Path<'_>) -> Result<alloc::vec::Vec<u8>, Error>;

    /// Read a file in chunks.
    pub fn read_file_at(&mut self, inode: &Inode, offset: u64, buf: &mut [u8]) -> Result<usize, Error>;

    /// Iterate the entries of a directory.
    pub fn read_dir<'a>(&'a mut self, path: Path<'_>) -> Result<DirIter<'a, R>, Error>;

    /// Stat an inode (size, mode, link count, mtime, etc.).
    pub fn metadata(&mut self, inode: &Inode) -> Result<Metadata, Error>;
}
```

### 3.3 Error model

Single typed error enum:

```rust
pub enum Error {
    /// Underlying block read failed. Carries the caller's error type erased to
    /// a `&'static str` token plus a bytes-offset where the read attempt was made.
    Io { token: &'static str, offset: u64 },

    /// Superblock is missing, malformed, or has unsupported magic.
    BadSuperblock(SuperblockReason),

    /// Encountered an incompat flag we do not implement. Currently:
    /// `ZONED`, `RAID_STRIPE_TREE`. Carries the flag name as a stable token.
    UnsupportedFeature(&'static str),

    /// Encountered a chunk profile we do not implement. SINGLE/DUP/RAID1/
    /// RAID1C3/RAID1C4 are supported; RAID0/RAID10/RAID5/RAID6 hit this error.
    UnsupportedProfile(&'static str),

    /// Encountered a checksum type other than CRC32C.
    UnsupportedChecksum(&'static str),

    /// Metadata-block CRC32C mismatch. Carries the logical block address.
    CsumMismatch { logical: u64 },

    /// B-tree structure violation (e.g., key ordering broken, child count
    /// out of range, item region overflow). Token plus location.
    CorruptBtree { token: &'static str, where_: BtreeLocation },

    /// Path resolution failed. Carries which component failed and why.
    NotFound { component: &'static str },

    /// File was found but is not a regular file (directory, symlink, etc.).
    NotARegularFile,

    /// Compressed-extent decode failed.
    BadCompression { algorithm: &'static str },

    /// Allocation failure. Lambutter does not fall back; allocation
    /// failures are surfaced and the operation aborts.
    OutOfMemory,
}
```

All `&'static str` tokens are part of the stable vocabulary documented in §11.

### 3.4 Public surface stability

- v0.1.x: public API may add but not break.
- v0.2.0: any breaking change requires a major bump under the standard SemVer rules.

## 4. Module layout

```
lambutter/
├── Cargo.toml
├── LICENSE-APACHE
├── LICENSE-MIT
├── README.md
├── docs/
│   ├── SPEC-LAMBUTTER.md          (this file)
│   ├── ON-DISK-FORMAT-NOTES.md    (lambutter-specific notes pointing to upstream spec)
│   ├── TEST-FIXTURES.md           (how the test btrfs.img files are produced + verified)
│   └── CONTRIBUTING.md
├── src/
│   ├── lib.rs                     (crate-level docs, no_std attrs, public re-exports)
│   ├── block_read.rs              (BlockRead trait + helpers)
│   ├── error.rs                   (Error enum, Result alias, stable tokens)
│   ├── superblock.rs              (Superblock loader + validator)
│   ├── chunk_tree.rs              (Chunk tree replay; logical → physical resolver)
│   ├── btree.rs                   (Generic B-tree walker over arbitrary tree roots)
│   ├── checksum.rs                (CRC32C wrapper, metadata-block verify hook)
│   ├── items/
│   │   ├── mod.rs                 (Item-type dispatch enum)
│   │   ├── inode_item.rs
│   │   ├── inode_ref.rs
│   │   ├── dir_item.rs            (DIR_ITEM + DIR_INDEX, dir_hash function)
│   │   ├── extent_data.rs         (inline + regular + prealloc; compression dispatch)
│   │   ├── root_item.rs
│   │   ├── chunk_item.rs
│   │   └── dev_item.rs
│   ├── root_tree.rs               (root tree walker; default-subvol resolution)
│   ├── fs_tree.rs                 (FS tree walker bound to a specific subvolume)
│   ├── path.rs                    (Path type + canonicalization + component split)
│   ├── resolve.rs                 (Path → inode resolver; walks DIR_ITEM hash tables)
│   ├── inode.rs                   (Inode handle + Metadata accessor)
│   ├── file.rs                    (File reader: extent walker + decompression dispatch)
│   ├── dir.rs                     (Directory iterator)
│   ├── compression/
│   │   ├── mod.rs                 (Algorithm enum + decode entry point)
│   │   ├── zstd.rs                (zstd via ruzstd; feature = "zstd")
│   │   ├── zlib.rs                (zlib via miniz_oxide; feature = "zlib")
│   │   └── lzo.rs                 (LZO1X; feature = "lzo"; see §10 open questions)
│   ├── format/
│   │   ├── mod.rs                 (re-exports)
│   │   ├── constants.rs           (objectids, item types, magic numbers, offsets)
│   │   ├── flags.rs               (incompat / compat / ro_compat flag bit constants)
│   │   └── repr.rs                (POD struct definitions matching on-disk layout)
│   └── util.rs                    (small helpers: bytemuck-free read_le_u64 etc.)
└── tests/
    ├── superblock.rs              (host-runnable: malformed SB, valid SB, generation skew)
    ├── chunk_tree.rs              (SINGLE / DUP / RAID1 chunk resolution)
    ├── btree_walk.rs              (key ordering, leaf / interior dispatch)
    ├── path_resolve.rs            (DIR_ITEM lookup, hash collisions, default subvol)
    ├── extent_read.rs             (inline / regular / prealloc / hole)
    ├── compression.rs             (zstd / zlib / LZO decode against fixtures)
    ├── snapper_default_subvol.rs  (OID-6 redirect)
    └── fixtures/
        ├── README.md              (how to regenerate fixtures from mkfs.btrfs + scripts)
        ├── single_uncompressed.img.zst
        ├── single_zstd.img.zst
        ├── single_zlib.img.zst
        ├── single_lzo.img.zst
        ├── dup_metadata_single_data.img.zst
        ├── raid1.img.zst
        └── snapper_default_subvol.img.zst
```

LoC estimates per module (target, not contract — implementation may diverge):

| Module | Target LoC |
|---|---:|
| `lib.rs`, error, util | 250 |
| `block_read.rs` | 80 |
| `superblock.rs` | 250 |
| `chunk_tree.rs` | 350 |
| `btree.rs` | 400 |
| `checksum.rs` | 80 |
| `items/*.rs` | 700 |
| `root_tree.rs` | 200 |
| `fs_tree.rs` | 150 |
| `path.rs`, `resolve.rs` | 300 |
| `inode.rs`, `file.rs`, `dir.rs` | 500 |
| `compression/*` | 250 |
| `format/*` | 400 |
| **Implementation total** | **~3900** |
| Tests (host) | 1500 |
| Fixtures (binary, not LoC) | n/a |
| **Grand total LoC** | **~5400** |

This is in line with the format reference doc's "GRUB's `grub-core/fs/btrfs.c` is ~3.5 kLoC C" existence proof, and within the BTRFS-FORK-TARGET-ANALYSIS doc's range (which cited 1500–4000 LoC depending on harvesting strategy; we're at the upper bound because we are implementing fully independently).

## 5. Cargo features

| Feature | Default? | Pulls in | Effect |
|---|:---:|---|---|
| `zstd` | yes | `ruzstd` | Decode zstd-compressed extents |
| `zlib` | no | `miniz_oxide` | Decode zlib-compressed extents |
| `lzo` | no | (TBD; see §10) | Decode LZO-compressed extents |
| `std` | no | — | Implement `std::error::Error` for `Error`; otherwise no behavioral change |

A consumer that uses no compression features cannot read compressed extents; encountering one returns `Error::BadCompression { algorithm: "..." }`.

For LamBoot's v0.10.0 use, all three compression features will be enabled (per founder direction 2026-04-27).

## 6. Internal architecture

### 6.1 BlockRead

The trait is the single point of contact with the physical medium. Lambutter never assumes a seekable file; it requests byte ranges at absolute offsets and expects them filled.

UEFI consumers wrap `BLOCK_IO_PROTOCOL` reads behind a `BlockRead` adapter. Test consumers wrap a `&[u8]` or a `std::fs::File` (gated behind the `std` feature for the latter, which v0.1.0 will provide as `examples/cat.rs`).

### 6.2 Superblock loading

Per BTRFS-FORMAT-READONLY-REFERENCE §4:

1. Read the primary superblock at offset `0x10000` (64 KiB)
2. Validate magic (`_BHRfS_M`), checksum (CRC32C over the superblock minus its csum field), generation, root tree pointer, chunk tree pointer
3. If primary fails, attempt secondary at `0x4000000` and tertiary at `0x4000000000`. The fourth at `0x4000000000000` is only present on multi-TiB devices; consult its presence by device size.
4. Among valid superblocks, select the one with the highest generation
5. Cache the chunk-tree-bootstrap (`sys_chunk_array`) in memory; it is read once

### 6.3 Chunk tree resolution

Two-phase:

1. **Bootstrap phase:** the system chunk array embedded in the superblock provides enough chunks to read the chunk tree itself. This must happen before any logical-address read can be resolved.
2. **Full chunk tree walk:** with the bootstrap chunks providing physical access to the chunk tree's root, walk the chunk tree to populate an in-memory chunk map covering the whole logical address space.

Profile dispatch (§13.3 of format reference):

- SINGLE / DUP: pick the first stripe's `(devid, physical)`
- RAID1 / RAID1C3 / RAID1C4: pick any mirror; on metadata-csum-failure, fall through to next mirror; on exhaustion, return `CsumMismatch`
- RAID0 / RAID10 / RAID5 / RAID6: return `UnsupportedProfile`

### 6.4 Root tree walk + default-subvol resolution

Three-tier resolution, exactly as agreed in the format reference §9 and confirmed in OPENSUSE-YAST §3:

1. **Caller-supplied override (highest priority).** A future API extension may accept a `subvol_id_override` parameter; v0.1.0 does not expose this but the architecture must not preclude it.
2. **OID-6 DIR_ITEM "default" in ROOT_TREE.** Walk the root tree; locate the directory item at objectid 6 with a `name` of `"default"`. Its `location` field carries a `(objectid, type, offset)` key whose `objectid` is the subvolume ID to mount as default. This is what Snapper updates on rollback. **This is what GRUB uses. Lambutter mirrors GRUB.**
3. **Superblock `default_subvol_id` (fallback).** If the OID-6 DIR_ITEM is absent, fall back to the superblock field. If that is `0` or `5`, treat as FS_TREE (objectid 5) — the global root.

The chosen subvolume's `ROOT_ITEM` provides the bytenr of the FS tree root. Subsequent path resolution operates against that FS tree.

### 6.5 B-tree walking

A single generic walker covers all on-disk B-trees (chunk, root, FS, extent, log, csum, free-space, uuid, quota — though we only use chunk/root/FS in v0.1.0).

Algorithm:

- Start at a tree root bytenr
- For each node, verify metadata-block CRC32C; on mismatch, return `CsumMismatch` (no fallback to mirrors at this layer; mirroring is handled below in the BlockRead resolution)
- Interior node: binary-search the keys, descend
- Leaf node: return the matching item or "next greater" depending on caller intent

The walker is invoked with a target key; it returns either an exact-match item, a range of items, or a "key not found" condition.

### 6.6 DIR_ITEM hashing

Path-component lookup uses CRC32C with the **non-default seed `0xFFFFFFFE`** over the raw name bytes (per BTRFS-FORMAT-READONLY-REFERENCE §7). This is NOT the same as plain CRC32C with seed 0. Hash collisions cause multiple `btrfs_dir_item` records to pack back-to-back in the same DIR_ITEM leaf — the implementation must iterate them, comparing the literal name bytes.

### 6.7 EXTENT_DATA reading

Per format reference §8:

- 21-byte common header + variable tail
- `type=0` inline: data follows directly in the extent_data; trim to file size
- `type=1` regular: tail is 32 bytes carrying `(disk_bytenr, disk_num_bytes, offset, num_bytes)`. `disk_bytenr=0` means hole, return zeros
- `type=2` prealloc: same as regular but data is conceptually unwritten — return zeros
- Compression: applied to the entire extent; decompress fully into a buffer, then return the file-relevant slice
- `NO_HOLES` incompat flag changes the semantics of gaps between EXTENT_DATA items: under NO_HOLES, gaps are zero-fill; without NO_HOLES, an explicit hole EXTENT_DATA is present

### 6.8 Compression dispatch

```
compression/mod.rs:
    pub enum Algorithm { None, Zlib, Lzo, Zstd }
    pub fn decode(alg: Algorithm, src: &[u8], dst: &mut Vec<u8>) -> Result<(), Error>
```

- `None`: copy
- `Zlib`: `miniz_oxide::inflate::decompress_to_vec_with_limit`
- `Lzo`: see §10 open questions
- `Zstd`: `ruzstd::StreamingDecoder` (use streaming so a malformed extent doesn't cause unbounded allocation)

Each algorithm gates an output-size cap of 16 MiB to prevent decompression bombs against malicious volumes; this cap is configurable via a const in `compression/mod.rs`.

## 7. Test strategy

### 7.1 Three layers

**Layer A — host unit tests (`cargo test`):** purely synthetic. Construct in-memory buffers representing valid and invalid on-disk structures. Exercise the parser and walker logic without any block-IO dependency. This is the bulk of the test surface (~1500 LoC target).

**Layer B — fixture tests (`cargo test --test extent_read` etc.):** real `mkfs.btrfs`-produced filesystem images, compressed via zstd at rest, decompressed at test-time. Exercised against the real `BlockRead` interface. Each fixture targets a specific scenario:
- single-disk uncompressed / zstd / zlib / LZO
- DUP metadata + SINGLE data
- RAID1 (two-disk; lambutter sees only the first device but the chunk tree references both)
- Snapper-style default-subvol redirect

Fixtures are generated by scripts under `tests/fixtures/scripts/` so CI can rebuild them. Each fixture is committed with a `.sha256` companion to detect regeneration drift.

**Layer C — fuzz targets:** `cargo-fuzz`-driven harnesses for:
- `fuzz_target_superblock` — feed arbitrary bytes to the superblock validator, ensure no panic, no out-of-bounds, no infinite loop
- `fuzz_target_btree_node` — feed arbitrary bytes representing a tree node
- `fuzz_target_extent_data` — feed arbitrary EXTENT_DATA bytes
- `fuzz_target_dir_item` — feed arbitrary DIR_ITEM bytes
- `fuzz_target_compressed_extent` — feed arbitrary compressed bytes per algorithm

Fuzz targets land in v0.1.0 alongside the code they cover; they are not deferred.

### 7.2 Audit posture

Per founder direction 2026-04-27: any code merged into lambutter is audited against `~/lamboot-dev/docs/analysis/BTRFS-FORMAT-READONLY-REFERENCE-2026-04-27.md` section by section. The audit posture is:

- Each on-disk struct definition cites the format-reference section that authorizes it
- Each walker / parser cites the section whose algorithm it implements
- Deviations from the spec are PR-reviewable and require an inline justification comment

The PR template enforces this checklist.

## 8. LamBoot integration plan

### 8.1 Dependency declaration

`~/lamboot-dev/lamboot-core/Cargo.toml` gains:

```toml
lambutter = { path = "../../lambutter-dev", features = ["zstd", "zlib", "lzo"] }
```

(Path-dep during development; switches to a versioned crates.io dep at v0.1.0 release of lambutter.)

### 8.2 New module

`~/lamboot-dev/lamboot-core/src/fs_backend_btrfs.rs` — mirrors the existing `fs_backend_ext4.rs`. It implements the `FsBackend` trait via a `BtrfsBackend` newtype wrapping `lambutter::Btrfs<UefiBlockReader>`.

`UefiBlockReader` implements `lambutter::BlockRead` by delegating to `uefi::proto::media::block::BlockIO`.

### 8.3 Wiring point

`partitions::probe_superblock` already detects btrfs at offset `0x10000` (per the existing code path memory observation `7b4e6b56`). The dispatch site that mounts a probed partition currently routes ext4 through `Ext4Backend`; that dispatch grows a btrfs branch routing to `BtrfsBackend`.

### 8.4 Trust-log integration

Each metadata-block-csum failure surfaced by lambutter results in a `csum_mismatch` event recorded into LamBoot's trust log. Each `UnsupportedProfile` / `UnsupportedFeature` error similarly. The trust log gains stable vocabulary additions documented under SDS-4 §6.3.

## 9. Milestones

| Milestone | Deliverable | Target effort |
|---|---|---|
| M1 — scaffold | Crate compiles. Stub `lib.rs`. CI pipeline (`cargo check --no-default-features`, `cargo test`, `cargo clippy`, `cargo fmt --check`). | done (this commit) + 0.5 day |
| M2 — superblock | `Superblock` loads + validates against fixture images. Layer A + B tests pass. | 4 days |
| M3 — chunk tree | Bootstrap + full walk. SINGLE/DUP/RAID1 logical→physical resolution. | 1 week |
| M4 — B-tree walker | Generic walker, key search, leaf iteration. Metadata csum verify. | 1 week |
| M5 — items | All required item types parse cleanly with host tests. | 1 week |
| M6 — root tree + default subvol | OID-6 redirect, fallback chain, snapper-fixture test passes. | 3 days |
| M7 — FS tree + path resolve | DIR_ITEM hash, name lookup, full path resolution. | 1 week |
| M8 — file read uncompressed | inline + regular + prealloc + hole. cat-style example works on uncompressed fixture. | 4 days |
| M9 — compression | zstd + zlib + LZO decode integrated. cat-style example works on each fixture. | 1 week |
| M10 — fuzz | All five fuzz targets land. 24h corpus collected and committed. | 1 week |
| M11 — LamBoot integration | `fs_backend_btrfs.rs` lands in `lamboot-dev`. Boots Tumbleweed VM. | 1 week |
| M12 — release prep | Docs, CHANGELOG, crates.io publishing pipeline. | 3 days |
| **Total** | **lambutter v0.1.0 + LamBoot v0.10.0** | **~10–12 calendar weeks** |

This matches the BTRFS-FORK-TARGET-ANALYSIS doc's "from-scratch" estimate (8–12 weeks) at the upper end, reflecting the choice to implement fully independently rather than harvesting from any existing crate.

## 10. Open questions before code starts

1. **LZO crate.** Is there a clean Rust-native `no_std` LZO1X decoder? `lzokay` exists but its no_std story needs verification. If no clean option exists, options are: (a) write a minimal LZO1X decoder ourselves (the algorithm is small, ~150 LoC); (b) ship LZO support as `feature = "lzo"` requiring `std`; (c) descope LZO from v0.1.0. **Resolving this is the only blocker for me to start writing implementation code; everything else is locked.**

2. **`docsrs` cfg.** Should the crate use `#[cfg(docsrs)]` annotations for feature-gated docs from the start? Following ext4-view's pattern, yes. Confirm.

3. **MSRV pinning policy.** ext4-view pins `rust-version = "1.85"` (stable). Lambutter currently pins `1.85` to mirror. LamBoot itself uses nightly. If lambutter is to be consumable by stable-Rust projects, stable MSRV is correct; LamBoot will simply consume a stable-targeted crate from a nightly toolchain. Confirm this is the intent.

## 11. Stable vocabulary

Tokens used in `Error` variants and trust-log integration. Stable across patch releases; additions require minor bump; renames or removals require major bump.

| Token | Used in | Meaning |
|---|---|---|
| `bad_magic` | `Error::BadSuperblock` | superblock magic field did not match `_BHRfS_M` |
| `bad_csum_sb` | `Error::BadSuperblock` | superblock csum field did not validate |
| `gen_skew` | `Error::BadSuperblock` | superblock copies disagree on generation |
| `feat_zoned` | `Error::UnsupportedFeature` | `BTRFS_FEATURE_INCOMPAT_ZONED` set |
| `feat_raid_stripe_tree` | `Error::UnsupportedFeature` | `BTRFS_FEATURE_INCOMPAT_RAID_STRIPE_TREE` set |
| `prof_raid0` | `Error::UnsupportedProfile` | RAID0 profile encountered |
| `prof_raid10` | `Error::UnsupportedProfile` | RAID10 profile encountered |
| `prof_raid5` | `Error::UnsupportedProfile` | RAID5 profile encountered |
| `prof_raid6` | `Error::UnsupportedProfile` | RAID6 profile encountered |
| `csum_xxhash` | `Error::UnsupportedChecksum` | xxhash csum type |
| `csum_sha256` | `Error::UnsupportedChecksum` | sha256 csum type |
| `csum_blake2` | `Error::UnsupportedChecksum` | blake2 csum type |
| `key_order` | `Error::CorruptBtree` | leaf keys not strictly ascending |
| `item_oob` | `Error::CorruptBtree` | item region overruns leaf bounds |
| `child_count` | `Error::CorruptBtree` | interior node has 0 or > capacity children |
| `infinite_recursion` | `Error::CorruptBtree` | walker depth exceeded sane bound |
| `comp_zlib` / `comp_lzo` / `comp_zstd` | `Error::BadCompression` | decompressor rejected input |
| `oom_extent` | `Error::OutOfMemory` | allocation failed during file-read buffer expansion |

## 12. Non-goals (explicit)

- Lambutter will never write to a btrfs volume. The crate has no code path that calls `BlockRead::write_at` because the trait has no such method.
- Lambutter will not be a general-purpose btrfs library. It is purpose-built for read-only `/boot` access. Consumers wanting full-featured access should use `btrfs-progs` userspace, not lambutter.
- Lambutter will not depend on `tokio`, `async-std`, `std::io`, `std::fs`, or any threading primitive.
- Lambutter will not depend on any other btrfs Rust crate. Implemented from the on-disk-format specification, not derived from existing parsers.

## 13. Sign-off

Before any implementation lands, the founder signs off on this document. After sign-off, milestones M2–M12 begin per §9.

Open questions in §10 are the only items currently blocking the start of M2.

---

**End of SPEC-LAMBUTTER**
