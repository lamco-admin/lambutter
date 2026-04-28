# lambutter — Testing & Fuzzing Plan

**Created:** 2026-04-28
**Status:** Active design; supersedes the per-milestone test bullets in
`SPEC-LAMBUTTER.md` §7.
**Companion to:** `~/lamboot-dev/docs/analysis/BTRFS-FORMAT-READONLY-REFERENCE-2026-04-27.md`
**Audience:** lambutter developers + reviewers preparing v0.1.0 for ship.

This document is the deliverable from research-explore session
`1358184b-f1ed-4cf0-94a0-dcf9bb0d5804`, thread
`1a64de8b-488d-44c3-b45f-cbe517bb5223`.

---

## 1. Purpose

Lambutter is a security-relevant read path: a UEFI bootloader
ingesting on-disk bytes from an untrusted filesystem and resolving
them into kernel images that then run with full system privileges.
The crate must demonstrably:

1. **Refuse malformed inputs** without panicking, looping, or reading
   out of bounds.
2. **Produce byte-identical output** for valid inputs against an
   independent oracle (the Linux kernel's own btrfs reader, primarily).
3. **Cover every algorithmic decision** the format spec calls for —
   not as a generic "high coverage" goal but as a per-§13 audit.

The plan below has three layers (host unit tests, fixture-based
oracle tests, fuzz harnesses) and one feature gap to close before
v0.1.0 ships (symlink target reading).

## 2. Status snapshot

| Layer | What's done | What's pending for v0.1.0 |
|---|---|---|
| A — host unit tests | 34 tests, all modules covered for happy-path + targeted error paths | Symlink resolution test once `read_link` lands |
| B — fixture-based oracle tests | Architectural plan only; no `tests/fixtures/*.img.zst` exists yet | Generation scripts + 6 fixtures + harness |
| C — fuzz harnesses | 5 cargo-fuzz targets scaffolded; no corpus | Seed corpus from real `mkfs.btrfs` images + 1 hour smoke run per target |
| Feature audit | Cross-checked against format-reference §13 (see §3) | Close `read_link` gap; ship |

## 3. Feature-coverage audit (§13 of the format reference)

Verified by code inspection 2026-04-28:

```
+--------------------------------------------+--------+--------------------------------+
| Mandatory capability                       | Status | Implementation site            |
+--------------------------------------------+--------+--------------------------------+
| Superblock parse + crc32c verify           | DONE   | src/superblock.rs              |
| sys_chunk_array bootstrap                  | DONE   | chunk_tree::parse_system_..    |
| Chunk-tree full walk                       | DONE   | chunk_tree::populate_from_..   |
| logical->physical SINGLE/DUP/RAID1/1C3/1C4 | DONE   | chunk_tree::pick_stripe        |
| B-tree leaf + interior parsing             | DONE   | btree.rs                       |
| Metadata-block crc32c verification         | DONE   | btree::read_tree_block         |
| ROOT_TREE walk                             | DONE   | root_tree.rs                   |
| OID-6 default-subvol DIR_ITEM resolution   | DONE   | root_tree::lookup_default_..   |
| ROOT_ITEM lookup                           | DONE   | root_tree::lookup_root_item    |
| FS-tree path resolution                    | DONE   | resolve.rs                     |
| DIR_ITEM hash w/ seed 0xFFFF_FFFE          | DONE   | root_tree::name_hash           |
| Hash-collision DIR_ITEM unpacking          | DONE   | resolve.rs + root_tree.rs      |
| INODE_ITEM parse                           | DONE   | format/repr.rs::InodeItem      |
| EXTENT_DATA inline                         | DONE   | file::apply_extent             |
| EXTENT_DATA regular (incl. holes)          | DONE   | file::apply_extent             |
| EXTENT_DATA prealloc (zero-fill)           | DONE   | file::apply_extent             |
| NO_HOLES gap zero-fill                     | DONE   | file::read_file (gap loop)     |
| zstd decompression                         | DONE   | compression/zstd.rs            |
| zlib decompression                         | DONE   | compression/zlib.rs            |
| Incompat-flag tolerance (14 listed flags)  | DONE   | superblock.rs (only rejects 2) |
| RAID0/10/5/6 clean rejection               | DONE   | chunk_tree::pick_stripe        |
| ZONED + RAID_STRIPE_TREE clean rejection   | DONE   | superblock.rs                  |
| xxhash/sha256/blake2 csum-type rejection   | DONE   | superblock.rs                  |
| Cross-leaf iteration (re-descend pattern)  | DONE   | dir.rs, file.rs, chunk_tree.rs |
+--------------------------------------------+--------+--------------------------------+

GAP — needed before v0.1.0:
| Symlink target read (read_link)            | TODO   | New file/inode method needed   |
```

**Cross-leaf iteration is not a gap.** The crate iterates across leaf
boundaries in three places (`dir.rs:51-86`, `file.rs:87-141`,
`chunk_tree.rs:243-303`) using the re-descend-with-bumped-target
pattern — call `find_first_ge(last_key + 1)` from the tree root after
exhausting the current leaf. This is correct and is the same pattern
GRUB uses. It carries an O(depth log n) per-transition cost vs. a
sibling-pointer optimization, but for typical /boot trees (depth 1-3,
~200-300 items per 16 KiB leaf), the penalty is negligible. The
sibling-pointer optimization is a v0.2.0 perf improvement, not a v0.1.0
correctness gap. The CHANGELOG line "Cross-leaf path-component
iteration for very large directories" was misleading and is corrected
below.

**Symlink reading is the one real gap.** `/boot` commonly contains
symlinks like `vmlinuz -> vmlinuz-6.X.Y` and `initrd.img -> initrd.img-...`.
A bootloader that can't follow them fails on stock Tumbleweed and
Fedora. The fix is small (~40 LoC):

```rust
impl<R: BlockRead> Btrfs<R> {
    pub fn read_link(&mut self, path: Path<'_>) -> Result<Vec<u8>>;
}
```

The symlink target is stored as inline data in an EXTENT_DATA item on
the symlink's inode (per format-reference §8). The implementation
reuses the existing inline-extent reader; only the `is_symlink`
gate-then-read sequence is new.

## 4. Layer A — host unit tests (`cargo test`)

**Done.** 34 tests, all pure-logic against in-memory buffers. No
external dependencies. Runs on every commit via the `.githooks/pre-commit`
hook. Coverage spans:

- util.rs: `read_le_*`, `align_up`
- checksum.rs: known CRC32C vectors + seeded variant + verify round-trip
- block_read.rs: `&[u8]` adapter happy + OOB
- format/flags.rs: `IncompatFlags::rejected_for_v0_1` matrix
- format/repr.rs: `DiskKey` ordering + parse round-trip
- superblock.rs: minimal-valid SB load, bad-magic, bad-csum,
  unsupported-csum-type, ZONED-incompat, generation-arbitration
- chunk_tree.rs: sys_chunk_array parse, out-of-chunk error, RAID5
  rejection
- btree.rs: single-leaf find_exact, absent-key, find_first_ge match,
  csum-mismatch rejection
- root_tree.rs: name-hash deterministic + seed-sensitive
- path.rs: non-absolute rejection, components, dot-skipping,
  double-slash collapse, root-yields-empty

**Pending additions for v0.1.0:**

- `lib.rs` integration test using `&[u8]` reader against a 1-leaf
  synthetic FS that exercises the whole `Btrfs::open` → `read_file`
  flow (currently each layer is tested in isolation).
- `read_link` test once the API lands.

## 5. Layer B — fixture-based oracle tests

This is the largest piece of pending work. The plan:

### 5.1 Fixture generator (`tests/fixtures/scripts/`)

A bash script that uses real `mkfs.btrfs` to produce small btrfs
images of known content, then compresses them with zstd at rest. Each
fixture targets a specific scenario:

```
+----+----------------------------+--------+----------------------------+
| #  | Filename                   | Size   | What it exercises          |
+----+----------------------------+--------+----------------------------+
| F1 | single_uncompressed.img.zst| ~256K  | SINGLE profile, no compress|
| F2 | single_zstd.img.zst        | ~256K  | zstd extents (default)     |
| F3 | single_zlib.img.zst        | ~256K  | zlib extents (compress=zlib)|
| F4 | single_lzo.img.zst         | ~256K  | LZO extents - feeds the    |
|    |                            |        | "v0.1.0 errors cleanly"    |
|    |                            |        | path                       |
| F5 | dup_metadata.img.zst       | ~512K  | DUP metadata + SINGLE data |
|    |                            |        | (mkfs default for SSDs)    |
| F6 | snapper_default_subvol.img.| ~512K  | Tumbleweed-style: take a   |
|    |                            |        | snapshot, set it as the    |
|    |                            |        | default subvol via         |
|    |                            |        | btrfs subvolume set-default|
|    | zst                        |        | tests OID-6 redirect       |
| F7 | nested_dirs_with_collisions| ~256K  | DIR_ITEM hash collisions   |
|    | .img.zst                   |        | (synthetic via known       |
|    |                            |        | colliding name pairs)      |
| F8 | sparse_no_holes.img.zst    | ~256K  | NO_HOLES on, sparse file   |
|    |                            |        | with 4 KiB hole between    |
|    |                            |        | extents                    |
| F9 | symlink_chain.img.zst      | ~128K  | /a -> /b -> /target file   |
|    |                            |        | (gates `read_link`)        |
+----+----------------------------+--------+----------------------------+
```

Generation script outline:

```bash
#!/bin/bash
set -euo pipefail
OUT=$(dirname "$0")/..
case "$1" in
    F1) build_single_uncompressed ;;
    F2) build_single_zstd ;;
    # ...
esac

build_single_zstd() {
    truncate -s 256M /tmp/lb.img
    mkfs.btrfs -O ^extref -d single -m single /tmp/lb.img
    sudo mount -o loop,compress=zstd /tmp/lb.img /mnt/lb
    sudo cp /usr/lib/firmware/some_blob.bin /mnt/lb/test_file.bin
    sudo umount /mnt/lb
    zstd /tmp/lb.img -o "$OUT/single_zstd.img.zst"
}
```

Each fixture has a sibling `.expected.json` describing the canonical
content (file sha256s, directory listings, default-subvol objectid,
etc.) so the test harness can compare against a known-correct baseline
without requiring the kernel at test time.

CI policy: fixtures are committed, regenerated only when scenarios
change. Each fixture's `.sha256` companion is verified pre-decompress.

### 5.2 Harness

```rust
// tests/fixtures.rs
use std::io::Read;
use lambutter::{Btrfs, Path};

fn load_fixture(name: &str) -> Vec<u8> {
    let path = format!("tests/fixtures/{name}.img.zst");
    let f = std::fs::File::open(&path).unwrap();
    let mut decoded = Vec::new();
    zstd::stream::Decoder::new(f).unwrap().read_to_end(&mut decoded).unwrap();
    decoded
}

#[test]
fn f2_single_zstd_reads_files_correctly() {
    let img = load_fixture("single_zstd");
    let mut fs = Btrfs::open(&img[..], img.len() as u64).unwrap();
    let bytes = fs.read_file(Path::new(b"/test_file.bin").unwrap()).unwrap();
    assert_eq!(sha256(&bytes), expected_sha256_for("test_file.bin"));
}
```

### 5.3 Oracle differential testing (Linux-only CI gate)

A separate test target that:

1. Decompresses the fixture into `/tmp/lb-test.img`
2. `losetup -fP /tmp/lb-test.img` + `mount -o ro,loop /dev/loopX /mnt/lb-oracle`
3. For each path the harness checks: read via lambutter AND read via
   the kernel mount; assert byte-identical
4. Clean unmount + losetup -d

This requires CAP_SYS_ADMIN for loop mounts. CI matrix: run on Linux
runners only; macOS/Windows skip with a message.

For developers who can't loop-mount: fall back to the
`rustutils/btrfs-fs` Rust oracle. Add as `[dev-dependencies] btrfs-fs =
"0.12"` (gated behind a `dev-oracle` feature so the std/tokio
dependency doesn't pollute the main crate's dep graph). Use it only
inside `tests/oracle.rs`.

```rust
// tests/oracle.rs (gated on `--features dev-oracle`)
#[test]
#[cfg(feature = "dev-oracle")]
fn lambutter_matches_btrfs_fs_for_f2() {
    let img = load_fixture("single_zstd");
    let lb_bytes = lambutter::Btrfs::open(&img[..], img.len() as u64)
        .unwrap()
        .read_file(lambutter::Path::new(b"/test_file.bin").unwrap())
        .unwrap();

    let cursor = std::io::Cursor::new(&img);
    let oracle_fs = btrfs_fs::Filesystem::open(cursor).unwrap();
    let oracle_bytes = oracle_fs.read_path("/test_file.bin").unwrap();
    assert_eq!(lb_bytes, oracle_bytes);
}
```

License consideration: `btrfs-disk` and `btrfs-fs` (the rustutils
subcrates we'd vendor as `dev-dependencies`) declare MIT OR Apache-2.0
in their per-crate `Cargo.toml`s, but the workspace top-level
`LICENSE.md` is GPL-2.0. The clean answer for dev-only inclusion is to
use the published-on-crates.io versions (which inherit the per-crate
licenses, not the workspace-root file). If the upstream pushes a
release where the per-crate license becomes ambiguous, switch the
oracle to btrfs-progs subprocess invocation.

## 6. Layer C — fuzz harnesses

### 6.1 What's in place

Five `cargo-fuzz` harnesses scaffolded under `fuzz/fuzz_targets/`:

- `fuzz_superblock` — feeds arbitrary bytes through `Btrfs::open`
- `fuzz_btree_node` — same, plus a path-resolve attempt
- `fuzz_extent_data` — same, plus several plausible /boot path reads
- `fuzz_dir_item` — same, plus several path-resolve attempts
- `fuzz_compressed_extent` — placeholder (see §6.4 below)

All five route through the public API, so they exercise the
parser/walker/resolver layers as a unit rather than poking
crate-private internals.

### 6.2 Corpus seeding

Initial corpus per target:

```
fuzz/corpus/fuzz_superblock/
├── seed_01_real_mkfs.bin       (Fixture F1's first 4 MiB)
├── seed_02_truncated.bin       (Fixture F1 cut to 64 KiB + 1 SB)
├── seed_03_secondary_only.bin  (4 KiB at offset 0x4000000)
└── seed_04_zero.bin            (all zeros, baseline)
```

Each target gets 4-8 seed files extracted from the real fixtures (§5.1).
This dramatically accelerates path coverage since the fuzzer starts
from valid-shaped inputs and mutates outward.

### 6.3 Smoke run targets

Before v0.1.0 ship:

- 1 hour per target on a single core (5 hours total wall-clock).
- Track new-edge counts; flag any target where coverage flatlines in
  under 10 minutes (signals the harness isn't reaching deep code).
- Any crash → diagnose, fix, add a regression test, re-run.

### 6.4 `fuzz_compressed_extent` is a placeholder

The current implementation is a no-op (§9 of `fuzz_compressed_extent.rs`
is honest about it). Reaching the compression decoders via the public
API requires constructing a synthetic btrfs volume with a compressed
EXTENT_DATA — non-trivial. Two options:

1. **Build the synthetic image construction once** in
   `fuzz/build_compressed_volume.rs` and have the fuzzer use it as a
   harness wrapper. ~1 day of work.
2. **Expose `compression::decode` through a `#[doc(hidden)] #[cfg(fuzzing)]`
   re-export.** Cleaner code; the crate's main API surface stays intact.
   ~30 minutes.

Option 2 is the recommendation. Add to `lib.rs`:

```rust
#[doc(hidden)]
#[cfg(fuzzing)]
pub mod __fuzz_internals {
    pub use crate::compression::decode;
}
```

The harness then becomes:

```rust
fuzz_target!(|data: &[u8]| {
    if data.is_empty() { return; }
    let alg = data[0] % 4;
    let mut dst = Vec::new();
    let _ = lambutter::__fuzz_internals::decode(alg, &data[1..], &mut dst);
});
```

### 6.5 Continuous fuzzing (post-ship)

OSS-Fuzz integration is the long-term goal. lambutter is exactly the
shape of project OSS-Fuzz wants: pure Rust, no_std-ish, security
relevance, narrow input surface. Submission is a v0.2.0+ task; the
v0.1.0 fuzz pass demonstrates we've taken it seriously.

## 7. Comparison testing — what we still want to verify

Beyond differential read-equality, three properties worth comparing
across our implementation and the oracle:

1. **Error mapping.** When given a deliberately corrupted image, does
   lambutter surface an Error variant that's at least as informative
   as the kernel's `EIO` / `dmesg` line? This is a qualitative review,
   not an automated test, but worth running once per fixture.
2. **Memory bounds.** Decompression bombs: a malicious volume claims a
   compressed extent that decodes to 100 GiB. Lambutter caps at
   `MAX_DECOMPRESSED_EXTENT_BYTES = 16 MiB`; verify with a synthetic
   bomb fixture that we hit the cap and return `BadCompression` rather
   than allocating 100 GiB.
3. **Determinism across runs.** Same input → same output, byte for
   byte. Add a property test that opens the same image twice and
   compares results.

## 8. Suhteevah / btrfs-no-std: useful as test fodder, not as oracle

Two of the rejected candidates can be useful in lambutter's test
matrix without being trusted as oracles:

- **`suhteevah/btrfs-nostd`** has incorrect superblock field offsets
  (per the fork-target analysis). It is therefore *useful as a
  negative test*: feed lambutter the bytes that suhteevah's
  `to_bytes` produces, and lambutter MUST reject them as bad-csum or
  bad-magic. This catches accidental adoption of suhteevah's offsets
  if a future maintainer cargo-cults from it.
- **`btrfs-no-std` (kennystrawnmusic v0.2.1)** is stale and
  format-only. Its struct layouts are a useful cross-check for our
  byte-offset constants in `format/repr.rs`. Manual review only;
  not automated.

These are appendix items, not gating tests.

## 9. Sequencing for v0.1.0 ship

```
+--------------------------------------------+--------+----------+
| Task                                       | Effort | Blocker? |
+--------------------------------------------+--------+----------+
| Implement `Btrfs::read_link`               | 1 day  | YES      |
| Write fixture-generation scripts (F1-F9)   | 1 day  | YES      |
| Generate + commit F1 + F2 (uncompressed +  | 0.5 d  | YES      |
|   zstd) — bare minimum to demonstrate it   |        |          |
|   works on real images                     |        |          |
| Wire up `tests/fixtures.rs` harness        | 0.5 d  | YES      |
| Add `__fuzz_internals` + fix               | 0.5 d  | NO       |
|   fuzz_compressed_extent                   |        |          |
| Run 1-hour smoke on each fuzz target       | 5 h    | YES if   |
|                                            |        | crashes  |
| Generate F3-F9 fixtures + their tests      | 1 day  | NO       |
| Wire up `dev-oracle` feature with btrfs-fs | 1 day  | NO       |
|   for non-Linux developers                 |        |          |
| Linux loop-mount differential test         | 1 day  | NO       |
+--------------------------------------------+--------+----------+
```

**Minimum viable ship:** symlinks + F1/F2 fixtures + 1-hour fuzz pass,
the rest can land in v0.1.1.

## 10. Open questions for the founder

1. **`dev-oracle` feature gate vs. btrfs-progs subprocess.** Pulling in
   `rustutils/btrfs-fs` as a dev-dependency adds tokio/std to the test
   build (not the main build). The alternative is shelling out to
   `btrfs inspect-internal dump-tree` and parsing its output. Cleaner
   dep graph, more brittle string parsing. Pick one.
2. **Symlink-following depth.** Should `Btrfs::read_file` automatically
   follow symlinks (POSIX-like), or should it always require explicit
   `read_link` then re-resolve? POSIX-like is convenient; explicit is
   safer (no symlink-loop risk in a bootloader). Recommendation:
   explicit-only in v0.1.0; consider a depth-limited follow option in
   v0.2.0.
3. **OSS-Fuzz submission timing.** v0.1.0 ship + 30 days, or wait for
   v0.2.0?

## 11. Document control

- 2026-04-28 — initial creation; research session
  `1358184b-f1ed-4cf0-94a0-dcf9bb0d5804` thread
  `1a64de8b-488d-44c3-b45f-cbe517bb5223`. Findings:
  `f4820f29` (cross-leaf), `4790fffb` (oracles), `0b208e26` (audit).
