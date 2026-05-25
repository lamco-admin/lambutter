# lambutter — Feature Inventory

**Purpose:** exhaustive, audit-friendly enumeration of what lambutter
implements, what it deliberately omits, and the rationale for each
choice. Update at every minor version bump.

**As of v0.3.0.** Companion to:
- `SPEC-LAMBUTTER.md` (design spec — what we *intend* to do)
- `SUPPORTED-SCENARIOS.md` (real-world btrfs config matrix — what
  *works* in practice)
- `TESTING-AND-FUZZING-PLAN.md` (test posture for each feature)

---

## 1. On-disk format coverage

### 1.1 Superblock

| Capability | Status | Implementation site |
|---|---|---|
| Parse all four canonical superblock locations (0x10000, 0x4000000, 0x4000000000, 0x4000000000000) | ✓ | `src/superblock.rs::load` |
| CRC32C body checksum verification | ✓ | `src/superblock.rs::load` |
| Highest-generation-among-valid-copies selection | ✓ | `src/superblock.rs::load` |
| Reject bad magic | ✓ | `BadSuperblock(BadMagic)` |
| Reject bad csum | ✓ | `BadSuperblock(BadCsum)` |
| Reject unsupported csum_type (xxhash / sha256 / blake2) | ✓ | `BadSuperblock(UnsupportedCsumType)` |
| Reject unsupported INCOMPAT flags (ZONED, RAID_STRIPE_TREE) | ✓ | `BadSuperblock(UnsupportedIncompat)` |
| Reject bad geometry (sectorsize / nodesize outside power-of-two range) | ✓ | `BadSuperblock(BadGeometry)` |
| Embedded `sys_chunk_array` bootstrap | ✓ | `src/chunk_tree.rs::parse_system_chunk_array` |

### 1.2 Chunk tree (logical → physical address resolution)

| Capability | Status | Notes |
|---|---|---|
| Two-phase bootstrap (sys_chunk_array first, then full chunk-tree walk) | ✓ | `src/chunk_tree.rs::populate_from_chunk_tree` |
| `SINGLE` profile | ✓ | `pick_stripe` |
| `DUP` profile | ✓ | mkfs.btrfs default for SSDs |
| `RAID1` profile | ✓ | Falls through to first-local-stripe |
| `RAID1C3` profile | ✓ | 3-copy mirror |
| `RAID1C4` profile | ✓ | 4-copy mirror |
| `RAID0` profile | ✓ (rejected) | `UnsupportedProfile("raid0")` |
| `RAID10` profile | ✓ (rejected) | `UnsupportedProfile("raid10")` |
| `RAID5` profile | ✓ (rejected) | `UnsupportedProfile("raid5")` |
| `RAID6` profile | ✓ (rejected) | `UnsupportedProfile("raid6")` |
| Multi-device with missing local device | ✓ (rejected) | Stripe `devid` mismatch → typed error |

### 1.3 B-tree walker (generic across chunk / root / fs / extent / log / csum / free-space / uuid / quota)

| Capability | Status | Notes |
|---|---|---|
| Interior node parsing (binary-search across child pointers) | ✓ | `src/btree.rs` |
| Leaf node parsing (binary-search across items) | ✓ | `src/btree.rs` |
| Metadata-block CRC32C verification on every read | ✓ | `src/btree.rs::read_tree_block` |
| Bounded recursion (`MAX_TREE_DEPTH = 16`) | ✓ | Protects against malformed-tree DOS |
| `find_exact` (exact-key match) | ✓ | `src/btree.rs::find_exact` |
| `find_first_ge` (first key >= target) | ✓ | `src/btree.rs::find_first_ge` |
| Cross-leaf iteration (re-descend pattern) | ✓ | Three call sites: chunk_tree, dir, file |

### 1.4 Root tree

| Capability | Status | Notes |
|---|---|---|
| ROOT_TREE walk | ✓ | `src/root_tree.rs::lookup_root_item` |
| OID-6 default-subvol DIR_ITEM `"default"` resolution | ✓ | The Snapper/openSUSE rollback path |
| Fallback to superblock `root_dir_objectid` | ✓ | When DIR_ITEM "default" missing |
| Fallback to `FS_TREE_OBJECTID` (= 5) | ✓ | Default-default subvolume |
| DIR_ITEM hash with seed 0xFFFF_FFFE | ✓ | `src/root_tree.rs::name_hash` — table-driven CRC32C with reflected seed |

### 1.5 FS tree (per-subvolume path resolution)

| Capability | Status | Notes |
|---|---|---|
| `/`-separated path component walking | ✓ | `src/resolve.rs::resolve_path` |
| DIR_ITEM lookup by `(parent_inode, DIR_ITEM_KEY, name_hash)` | ✓ | Standard btrfs name lookup |
| DIR_ITEM hash-collision handling (walk packed DirEntry tail) | ✓ | Real-world: collisions are rare but happen |
| INODE_ITEM parse | ✓ | `src/format/repr.rs::InodeItem` |
| Symlink target reading via `read_link` API | ✓ (v0.1.1) | `src/file.rs::read_link` |
| Symlink-follow during path resolution | ✗ (deliberate) | Caller's responsibility — see CONSUMER-GUIDE §3 |

### 1.6 EXTENT_DATA (file content)

| Capability | Status | Notes |
|---|---|---|
| Inline extents | ✓ | Small files fit entirely in INODE_ITEM |
| Regular extents (incl. holes) | ✓ | `src/file.rs::apply_extent` |
| Prealloc extents (zero-fill) | ✓ | fallocate(2)-style preallocated ranges |
| `NO_HOLES` gap zero-fill | ✓ | Modern btrfs default; sparse-file handling |

### 1.7 Compression

| Algorithm | Status | Crate | Feature gate | Binary cost |
|---|---|---|---|---|
| zstd | ✓ | `ruzstd 0.7` (`no_std`) | `zstd` (default) | ~30 KiB |
| zlib | ✓ | `miniz_oxide 0.8` (`no_std`) | `zlib` | ~15 KiB |
| LZO (LZO1X-1) | ✓ (v0.1.1) | `lzokay 2.0` (`no_std`) | `lzo` | ~10 KiB |

Output is capped at `MAX_DECOMPRESSED_EXTENT_BYTES = 16 MiB` for any
single extent (DOS protection — a malformed compressed extent could
otherwise claim unbounded output size).

---

## 2. Public API surface

```rust
pub struct Btrfs<R: BlockRead> { /* opaque */ }

impl<R: BlockRead> Btrfs<R> {
    pub fn open(reader: R, device_size_bytes: u64) -> Result<Self>;
    pub fn default_subvol_objectid(&self) -> u64;
    pub fn resolve(&mut self, path: Path<'_>) -> Result<Inode>;
    pub fn metadata(&mut self, inode: &Inode) -> Result<Metadata>;
    pub fn read_file(&mut self, path: Path<'_>) -> Result<Vec<u8>>;
    pub fn read_file_at(&mut self, inode: &Inode, offset: u64, buf: &mut [u8]) -> Result<usize>;
    pub fn read_link(&mut self, path: Path<'_>) -> Result<Vec<u8>>;
    pub fn read_dir(&mut self, path: Path<'_>) -> Result<Vec<DirEntry>>;
}

pub trait BlockRead {
    type Error: Debug;
    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8]) -> Result<(), Self::Error>;
}

pub struct Inode { pub objectid: u64 }
pub struct Metadata { pub size, pub nbytes, pub nlink, pub mode, pub uid, pub gid, pub generation }
pub struct DirEntry { pub name: Vec<u8>, pub inode_number: u64, pub kind_byte: u8 }
pub struct Path<'a> { /* opaque */ }
pub enum Error { /* see error.rs */ }
pub enum SuperblockReason { /* sub-enum of Error::BadSuperblock */ }
```

Public API is `#[deny(missing_docs)]` — every public item has a doc
comment.

---

## 3. Error vocabulary

Every error variant carries a stable `&'static str` token for
log-grep consumers. The full vocabulary:

| Variant | Token(s) | Recoverable? |
|---|---|---|
| `Io { token, offset }` | `read_at_failed`, `out_of_bounds`, … (caller-defined via `BlockRead::Error`) | Caller-dependent |
| `BadSuperblock(BadMagic)` | — | Not recoverable on this volume |
| `BadSuperblock(BadCsum)` | — | All four copies corrupt |
| `BadSuperblock(GenerationSkew)` | — | Half-written superblock during prior crash |
| `BadSuperblock(NoValidCopy)` | — | Catastrophic |
| `BadSuperblock(UnsupportedCsumType)` | — | Use a newer lambutter (if added) or different volume |
| `BadSuperblock(UnsupportedIncompat)` | — | INCOMPAT flag we don't implement |
| `BadSuperblock(BadGeometry)` | — | sectorsize / nodesize outside supported range |
| `UnsupportedFeature(token)` | feature name (e.g. `"btrfs_incompat_flag"`) | Out of scope for v0.3.x |
| `UnsupportedProfile(token)` | `raid0`, `raid10`, `raid5`, `raid6` | By design |
| `UnsupportedChecksum(token)` | csum-type name | By design |
| `CsumMismatch { logical }` | — | Disk corruption at that bytenr |
| `CorruptBtree { token, logical }` | `key_order`, `item_region_overflow`, `child_count`, `depth_limit` | Disk corruption or malicious input |
| `NotFound` | — | Caller should check; not always an error |
| `NotARegularFile` | — | Path resolves to dir/symlink/special; caller should use the right API |
| `NotASymlink` | — | `read_link` called on non-symlink |
| `BadCompression { algorithm }` | `comp_zlib`, `comp_lzo`, `comp_zstd` | Disk corruption or unsupported variant |
| `OutOfMemory { token }` | allocation-site name | Caller should report and degrade gracefully |

---

## 4. Non-features (deliberate omissions)

These are *not* bugs and will not be implemented in the v0.3.x line:

| Item | Why deferred / rejected |
|---|---|
| Write support | Crate is read-only by construction — non-negotiable design property |
| Snapshot enumeration | v0.2.0+ scope — current users only need the active default subvol |
| Full subvolume traversal | v0.2.0+ scope |
| Free-space-tree walk (FREE_SPACE_INFO) | Skipped — never needed for content reads |
| Log-tree replay | Only relevant for crash-recovery write path |
| UUID-tree access | Not needed for read path |
| Quota-tree access | Not needed for read path |
| Data-block CSUM_TREE verification | v0.2.0+ scope — metadata-block CSUMs ARE verified |
| Multi-device read with stripe-failover (RAID1 with primary device missing) | Complex stripe-selection logic; v0.2.0+ candidate if a real consumer needs it |
| Hardware-accelerated CRC32C (SSE 4.2 / ARMv8 CRC) | Software-only by default; can be added behind a feature flag |
| Encrypted btrfs (`fscrypt` per-file encryption) | Not part of the v0.3.x scope; btrfs encryption is itself a non-default and not yet widespread |
| LZ4 / xz / brotli extent decoding | Not in btrfs format spec — only zstd/zlib/LZO are valid |
| Generic `BlockWrite` trait | Crate is read-only |
| File modification time / extended attributes (xattr) | Out of scope for v0.3.x — kernel-load consumers don't need them |
| ACL reading | Out of scope — same reason |

---

## 5. Resource bounds

All bounds are constants in `src/format/constants.rs` so an audit
reader can verify them in one place.

| Bound | Value | Purpose |
|---|---|---|
| `MAX_TREE_DEPTH` | 16 | DoS protection — malicious tree can't recurse unboundedly |
| `MAX_DECOMPRESSED_EXTENT_BYTES` | 16 MiB | DoS protection — claim-output-size-much-larger-than-input attack |
| Sectorsize range | 4 KiB ≤ N ≤ 65 KiB | Spec range; outside → `BadGeometry` |
| Nodesize range | 4 KiB ≤ N ≤ 65 KiB | Spec range; outside → `BadGeometry` |
| `name_hash` algorithm | CRC32C w/ seed `0xFFFF_FFFE`, reflected | Btrfs spec — hardcoded, not tunable |

---

## 6. Test posture

See `TESTING-AND-FUZZING-PLAN.md` for the full strategy. Summary:

| Layer | Coverage |
|---|---|
| A — Host unit tests | 36 tests, all modules |
| B — Fixture-based oracle | 8 `mkfs.btrfs`-generated fixtures (F1 SINGLE, F2 zstd, F3 zlib, F4 LZO, F5 DUP+SINGLE, F8 NO_HOLES, F9 symlinks, smoke) |
| C — Fuzz harnesses | 5 cargo-fuzz targets, 1-hour smoke run zero crashes |
| D — Live host validation | `examples/inspect.rs` against live openSUSE Tumbleweed sda2 — 6 paths, all sha256-byte-identical to kernel oracle |
| E — Embedded-context live validation | Linked into LamBoot v0.9.1; live UEFI boot of openSUSE Tumbleweed VM 102 with full boot-trust.log audit chain |

---

## 7. Audit hooks (downstream consumer interest)

For audit consumers needing reproducible-build-style attribution:

| Hook | Where |
|---|---|
| Source revision pin | `Cargo.toml` `version = "X.Y.Z"` + git tag |
| Crate version in trust logs | Downstream embeds via backend-tag string (e.g. LamBoot uses `lambutter@0.3.0-path`) |
| Per-extent compression algorithm | Surfaced in `Error::BadCompression { algorithm }` on decode failure; readable from EXTENT_DATA item field on success (not currently exposed via public API — could add for full audit if needed) |
| sha256 of file content | Caller's responsibility — lambutter only produces bytes |
| sha256 of metadata-block content | Internal — verified on read, not exposed |

---

## 8. Change log

- 2026-05-25: Initial features doc. Captures v0.1.1 + live-validation
  state (LamBoot integration through VM 102 case study).
