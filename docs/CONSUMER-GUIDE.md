# lambutter — Consumer Integration Guide

**Audience:** Rust developers integrating lambutter into a downstream
`no_std` project — UEFI bootloaders, recovery tooling, embedded
inspectors, kernel-stage parsers.

**Companion docs:**
- `SPEC-LAMBUTTER.md` — design spec (what the crate intends to do)
- `FEATURES.md` — exhaustive feature inventory
- `SUPPORTED-SCENARIOS.md` — distro × btrfs-config matrix
- `TROUBLESHOOTING.md` — symptom → cause → fix catalogue

---

## 1. Dependency declaration

In your consumer crate's `Cargo.toml`:

```toml
[dependencies]
# Pin exactly while you stabilize against a specific lambutter behavior.
# Loosen to "^0.3" once you're confident across the v0.3.x line.
lambutter = { version = "=0.3.0", default-features = false, features = ["zstd", "zlib", "lzo"] }
```

Notes on the dependency line:

- **`default-features = false`** is mandatory in `no_std` contexts.
  The `zstd` feature is in `default` (it pulls `ruzstd`); explicit
  enumeration prevents your build from accidentally pulling other
  defaults if we add them later.
- **Feature selection**: enable whatever decompressors your target
  systems might encounter. For UEFI bootloaders targeting modern
  Linux installs, enable all three (zstd / zlib / lzo). For embedded
  contexts where you control the btrfs creation, `zstd` alone may be
  sufficient.
- **Exact-pin vs loose**: lambutter follows semver. Within 0.1.x,
  public API may *add* but not break. The exact-pin is for downstream
  audit reproducibility — if you trust the v0.3.x semver guarantee,
  use `^0.3.0`.

---

## 2. Implementing `BlockRead`

`lambutter::BlockRead` is the single trait your project implements to
give lambutter access to disk bytes:

```rust
pub trait BlockRead {
    type Error: Debug;
    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8]) -> Result<(), Self::Error>;
}
```

The contract:
- Fill `buf` with `buf.len()` bytes starting at the absolute byte
  offset `offset_bytes` (relative to the volume's logical address
  space, i.e. start-of-partition not start-of-disk).
- Return `Err(_)` rather than partial reads. If your underlying medium
  short-reads, either re-issue the read internally or surface the
  failure.
- The buffer is fully populated on success.

### 2.1 Example: `std::fs::File`-backed (host-side testing)

```rust
use std::os::unix::fs::FileExt;

struct FileBlock { f: std::fs::File }
#[derive(Debug)] struct IoErr(std::io::Error);

impl lambutter::BlockRead for FileBlock {
    type Error = IoErr;
    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8])
        -> Result<(), IoErr>
    {
        self.f.read_exact_at(buf, offset_bytes).map_err(IoErr)
    }
}
```

Full working example at `examples/inspect.rs` (built with `cargo build
--release --example inspect`). Run against a real device:
`sudo target/release/examples/inspect /dev/sda2`.

### 2.2 Example: UEFI `EFI_BLOCK_IO_PROTOCOL`-backed

UEFI Block IO reads in fixed-size LBA blocks (typically 512 bytes),
so a byte-offset → block-offset conversion is required:

```rust
use uefi::{boot::ScopedProtocol, proto::media::block::BlockIO};

pub struct BlockIoReader {
    block_io: ScopedProtocol<BlockIO>,
    media_id: u32,
    block_size: u64,
}

impl lambutter::BlockRead for BlockIoReader {
    type Error = alloc::boxed::Box<dyn core::error::Error + Send + Sync + 'static>;

    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8])
        -> Result<(), Self::Error>
    {
        if buf.is_empty() { return Ok(()); }
        let (first_lba, aligned_len, intra) =
            compute_aligned_read(offset_bytes, buf.len(), self.block_size)
                .ok_or_else(|| "read overflow")?;
        let mut aligned = alloc::vec![0u8; aligned_len];
        self.block_io
            .read_blocks(self.media_id, first_lba, &mut aligned)
            .map_err(|e| alloc::boxed::Box::new(e) as _)?;
        buf.copy_from_slice(&aligned[intra..intra + buf.len()]);
        Ok(())
    }
}
```

Where `compute_aligned_read(start_byte, dst_len, block_size) -> Option<(first_lba, aligned_len, intra)>`
computes:
- `first_lba = start_byte / block_size`
- the number of blocks needed to cover the read range
- the intra-block offset where the requested bytes begin in the aligned buffer

Reference production implementation:
[`lamboot-core/src/fs_backend_ext4.rs::compute_aligned_read`](https://github.com/lamco-admin/lamboot-dev/blob/main/lamboot-core/src/fs_backend_ext4.rs)
(shared by both ext4 and btrfs backends in LamBoot).

### 2.3 Example: `&[u8]` slice-backed (the simplest case)

For in-memory images (test fixtures, recovery from a captured image),
lambutter ships a built-in impl:

```rust
let data: Vec<u8> = std::fs::read("/path/to/btrfs.img")?;
let reader: &[u8] = &data;
let mut fs = lambutter::Btrfs::open(reader, data.len() as u64)?;
```

This is what the fixture-based oracle tests use (see `tests/fixtures.rs`).

---

## 3. The symlink-follow pattern

**This is the single most common integration gotcha** — lambutter
deliberately does NOT follow symlinks during path resolution. The
crate's `Btrfs::resolve(path)` returns the inode of the symlink
itself; `read_file()` errors with `NotARegularFile` on a symlink.

Two reasons for this design:
1. POSIX-style stat-vs-lstat distinction needs to be preserved at the
   lowest layer; folding both into one call would lose information.
2. Symlink resolution requires path canonicalization (handling `..`,
   `.`, absolute vs relative) which is a higher-level concern.

The consumer is responsible for symlink-follow. The standard pattern:

```rust
const MAX_SYMLINK_DEPTH: u8 = 40;  // POSIX SYMLOOP_MAX (Linux uses 40)

fn resolve_following<R: lambutter::BlockRead>(
    fs: &mut lambutter::Btrfs<R>,
    path: &str,            // your project's path type
) -> Result<lambutter::Inode, MyError> {
    let mut current = String::from(path);
    for _ in 0..=MAX_SYMLINK_DEPTH {
        let lb_path = lambutter::Path::new(current.as_bytes())
            .ok_or(MyError::InvalidPath)?;
        let inode = fs.resolve(lb_path)?;
        let md = fs.metadata(&inode)?;
        if !md.is_symlink() {
            return Ok(inode);
        }
        let target_bytes = fs.read_link(lb_path)?;
        let target_str = core::str::from_utf8(&target_bytes)?;
        current = if target_str.starts_with('/') {
            // Absolute target — replaces current path entirely.
            String::from(target_str)
        } else {
            // Relative target — joins to parent of current path.
            let parent = current.rsplit_once('/').map(|(p, _)| p).unwrap_or("/");
            normalize_path(&format!("{parent}/{target_str}"))
                // your project's `..` / `.` normalization
        };
    }
    Err(MyError::SymlinkChainTooDeep)
}
```

**Real-world necessity:** openSUSE Tumbleweed (and any distro that
uses `kernel-install` from systemd) stores kernels as symlink chains:

```
/boot/vmlinuz                      → vmlinuz-6.19.12-1-default
/boot/vmlinuz-6.19.12-1-default    → ../usr/lib/modules/6.19.12-1-default/vmlinuz
```

A consumer that doesn't follow symlinks can't read `/boot/vmlinuz`.

Reference production implementation:
[`lamboot-core/src/fs_backend_btrfs.rs::resolve_following`](https://github.com/lamco-admin/lamboot-dev/blob/main/lamboot-core/src/fs_backend_btrfs.rs).

---

## 4. The error-translation pattern

`lambutter::Error` has 11 variants. Most consumers want to translate
to their own error type with stable log tokens. Lambutter ships each
error with a `&'static str` token from a documented vocabulary
(see `FEATURES.md` §3).

Recommended translation table:

```rust
fn translate(err: lambutter::Error) -> MyError {
    use lambutter::Error::*;
    match err {
        NotFound                         => MyError::not_found(),
        NotARegularFile                  => MyError::is_directory_or_symlink(),
        NotASymlink                      => MyError::unsupported("not_a_symlink"),
        BadSuperblock(reason)            => MyError::corrupt(reason_token(reason)),
        UnsupportedFeature(token)        => MyError::unsupported_feature(token),
        UnsupportedProfile(token)        => MyError::unsupported_feature(token),
        UnsupportedChecksum(token)       => MyError::unsupported_feature(token),
        CsumMismatch { logical: _ }      => MyError::corrupt("csum_mismatch"),
        CorruptBtree { token, .. }       => MyError::corrupt(token),
        BadCompression { algorithm }     => MyError::corrupt(algorithm),
        OutOfMemory { token }            => MyError::oom(token),
        Io { token, offset: _ }          => MyError::io(token),
    }
}
```

For audit-log-friendly consumers (security-relevant code paths), pass
the stable token through to log records. LamBoot does this via its
trust-log `verified_via` and per-event `note` fields.

---

## 5. Memory management considerations

lambutter is `no_std + alloc`. All allocations go through Rust's
global allocator. In UEFI contexts, that's typically the UEFI Boot
Services pool allocator (via `uefi-rs`'s `global_allocator` feature).

**What lambutter allocates:**

| Allocation | When | Size hint |
|---|---|---|
| `Vec<u8>` for tree blocks | Each `find_exact` / `find_first_ge` walk | One nodesize per level walked (typically 16-64 KiB) |
| `Vec<u8>` for `read_file` output | Once per call | File size |
| `Vec<u8>` for decompression buffer | Once per compressed extent | Up to `MAX_DECOMPRESSED_EXTENT_BYTES = 16 MiB` |
| `Vec<DirEntry>` for `read_dir` output | Once per call | One entry per directory child |
| Internal `ChunkMap` Vec | Once at `open()` | One entry per chunk in the chunk tree (typically tens to hundreds) |

**To control peak memory in UEFI:**
- Use `read_file_at` with chunked offsets instead of `read_file` for
  large files (e.g. multi-MB kernels) — bounds per-call allocation.
- Don't hold multiple `Btrfs<R>` instances open simultaneously unless
  necessary; each holds a chunk map.
- Decompression buffer is per-extent; if you're streaming large
  compressed files, peak memory is bounded by the largest extent (not
  the whole file).

**What lambutter does NOT do:**
- Cache tree blocks across calls (no LRU). Every walk re-reads.
  Trade-off: simpler code, but more I/O. If your I/O is fast, this
  doesn't matter. If you need caching, wrap your `BlockRead` impl in
  one.
- Hold open file handles. Each `read_file` is a fresh walk.

---

## 6. Concurrency

lambutter is single-threaded by design. `Btrfs<R>` is `!Sync` (the
underlying `R` typically can't be shared safely between threads
anyway). If you need concurrent reads, open multiple `Btrfs<R>`
instances on distinct `BlockRead` impls.

In UEFI, you're in single-thread execution anyway until
`ExitBootServices`, so this doesn't constrain you.

---

## 7. Testing your integration

### 7.1 Unit-test against the slice-backed reader

Construct a small btrfs image with `mkfs.btrfs --rootdir`, capture it
to a `.img` file, and read into a `Vec<u8>` for tests:

```rust
#[test]
fn integration_reads_real_btrfs_image() {
    let img = include_bytes!("../fixtures/test.img");
    let reader: &[u8] = img.as_slice();
    let mut fs = lambutter::Btrfs::open(reader, img.len() as u64).unwrap();
    // ... your assertions
}
```

### 7.2 Live-test against a real device with `examples/inspect.rs`

Build lambutter's bundled example, scp to a live system, run as root
against a real block device. Compare output against
`sha256sum`/`readlink` from the kernel oracle.

This is what proved Bug #10 in the LamBoot integration before the fix
shipped — `examples/inspect.rs` against VM 102's `/boot/vmlinuz`
returned a symlink target, not file content, revealing that the
consumer needed to follow symlinks.

### 7.3 Fuzz your translation layer

Lambutter ships 5 cargo-fuzz harnesses for the on-disk parsing path.
Your translation layer (BlockRead impl, error mapping, path
canonicalization) is a NEW attack surface — fuzz it independently if
your consumer is security-sensitive.

---

## 8. Versioning + upgrade discipline

For consumers exact-pinning lambutter (recommended for audit-
sensitive contexts like bootloaders):

1. Read lambutter's `CHANGELOG.md` for every bump.
2. Check if any of your error-translation arms reference a variant
   that changed shape. Rust will compile-error on mismatched match
   arms; the danger is silent behavior changes (e.g. an error that
   used to be `NotFound` now being `NotARegularFile`).
3. Re-run your integration test suite against the new lambutter
   version.
4. Bump your consumer's tag/version simultaneously (lockstep is
   easier than tracking compatibility windows).

For consumers using `^0.1` semver match:
- Within v0.3.x: API additions only. Your code should keep compiling.
- The first breaking change will be v0.4.0 (or v1.0.0 if we go straight
  to the stable line); expect to upgrade deliberately, not transparently.

---

## 9. Reference production consumer

The LamBoot UEFI bootloader uses lambutter as its native btrfs
backend. Its consumer code is at:

- [`lamboot-core/src/fs_backend_btrfs.rs`](https://github.com/lamco-admin/lamboot-dev/blob/main/lamboot-core/src/fs_backend_btrfs.rs)
  (~300 lines) — `FsBackend` trait impl wrapping lambutter
- BlockIO adapter for UEFI `EFI_BLOCK_IO_PROTOCOL`
- Error translation table from `lambutter::Error` → `FsError`
- Symlink-follow loop (the `resolve_following` pattern from §3)
- UUID extraction from the partition probe (lambutter v0.3.x doesn't
  expose superblock UUID; LamBoot reads it from byte offset 0x20
  before lambutter `open`s)

The integration was validated end-to-end on a real openSUSE Tumbleweed
install — see
[the case study](https://github.com/lamco-admin/lamboot-dev/blob/main/docs/migration/VM-102-OPENSUSE-TUMBLEWEED-BIOS-TO-UEFI-CASE-STUDY.md)
for the boot-trust.log audit trail.

---

## 10. Getting help

- Issues / questions: file in `lamco-admin/lambutter-dev` (the dev
  repo) or `lamco-admin/lambutter` (post-publish public repo)
- Specific scenarios not covered in `SUPPORTED-SCENARIOS.md`: see §9
  there for the info to capture
- Discussion: lambutter is a small enough crate that direct issue
  discussion is the right venue (no separate mailing list / Discord
  planned)

---

## 11. Change log

- 2026-05-25: Initial consumer guide. Captures patterns proven in the
  LamBoot integration through VM 102.
