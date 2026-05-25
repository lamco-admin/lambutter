# lambutter

**`no_std` read-only btrfs reader for UEFI bootloaders and embedded contexts.**

Lambutter is the btrfs counterpart to [`ext4-view`](https://github.com/nicholasbishop/ext4-view-rs):
a pure-Rust, allocation-aware, read-only btrfs filesystem reader designed for
contexts that cannot link `std` — UEFI applications, microcontrollers,
bare-metal kernels, and recovery tooling.

## Status

**v0.3.0 — feature-complete + live-validated** (see
[`docs/SPEC-LAMBUTTER.md` §2](docs/SPEC-LAMBUTTER.md) for scope, and
[`CHANGELOG.md`](CHANGELOG.md) for the version arc through v0.1.0 →
v0.3.0). Open and read regular files, symlinks, and directory listings
on SINGLE / DUP / RAID1 / RAID1C3 / RAID1C4 btrfs volumes; decode zstd,
zlib, and LZO extents; surface unsupported profiles (RAID0/10/5/6,
ZONED, RAID_STRIPE_TREE) as typed errors. The public API may add but
not break within the v0.3.x line. Data-block CSUM verification and
snapshot enumeration are tracked for v0.4.0+.

**Validation posture:**
- 36 host unit tests, all modules
- 9 fixture-based oracle tests (uncompressed, zstd, zlib, LZO,
  DUP-metadata, NO_HOLES sparse, symlinks, read-at chunked, smoke)
- 5 cargo-fuzz harnesses, 1-hour smoke run completed with zero crashes
- Live host validation: `examples/inspect.rs` against live openSUSE
  Tumbleweed btrfs partition with Snapper-managed default-subvol
  redirect — 6 path reads (regular file 38 MB full-extent walk +
  inline 11 B + 1.2 KB normal, symlinks in both `/boot` and `/etc`)
  all sha256-byte-identical to kernel oracle
- **Live embedded validation**: linked into [LamBoot](https://lamboot.dev)
  (Lamco's UEFI bootloader) as the native btrfs backend, booting
  openSUSE Tumbleweed end-to-end under live UEFI firmware with full
  trust-chain attribution. See the
  [case study](https://github.com/lamco-admin/lamboot-dev/blob/main/docs/migration/VM-102-OPENSUSE-TUMBLEWEED-BIOS-TO-UEFI-CASE-STUDY.md).

## Use

```rust
use lambutter::{Btrfs, Path};

let mut fs = Btrfs::open(reader, device_size_bytes)?;

// Resolve and read a file in one shot (loads the whole file into memory).
let kernel = fs.read_file(Path::new(b"/vmlinuz")?)?;

// Or stream it in fixed-size chunks (bounded memory).
let inode = fs.resolve(Path::new(b"/vmlinuz")?)?;
let mut buf = [0u8; 64 * 1024];
let mut off = 0u64;
loop {
    let n = fs.read_file_at(&inode, off, &mut buf)?;
    if n == 0 { break; }
    process(&buf[..n]);
    off += n as u64;
}
```

For integrating from another `no_std` Rust project — including the
`BlockRead` impl patterns for UEFI `EFI_BLOCK_IO_PROTOCOL`, file-backed
host testing, and the standard symlink-follow pattern — see the
[Consumer Guide](docs/CONSUMER-GUIDE.md).

A worked end-to-end example lives at `examples/inspect.rs`: a host-side
CLI that opens any block device or image file, reports the resolved
default-subvol objectid, and reads a named file printing size + sha256
+ first 16 bytes. Built with `cargo build --release --example inspect`;
run as root against a real device for live validation.

## Goals

- Read-only by design. The crate cannot mutate a btrfs volume.
- `no_std` + `alloc`. No `std`, no `tokio`, no async machinery.
- Sufficient subset for `/boot` reading on stock Linux distributions
  (openSUSE Tumbleweed/Leap, Fedora Workstation, CachyOS, Garuda) under
  Secure Boot.
- Fuzz-tested against malformed inputs.
- Independent of any other btrfs crate. Implemented from the on-disk format
  specification, not derived from existing parsers.

## Non-goals

- Write support of any kind.
- `bcachefs` / `zfs` / `xfs`. Each is its own filesystem; this crate is
  btrfs-only.
- Replacing the kernel's btrfs driver. Lambutter is for code paths where
  the kernel is not yet running.

## Features (Cargo)

| Feature | Default | What it enables |
|---|---|---|
| `zstd` | yes | zstd-compressed extent decoding via `ruzstd` (Tumbleweed / Fedora F34+ default) |
| `zlib` | no | zlib-compressed extent decoding via `miniz_oxide` (legacy default) |
| `lzo` | no | LZO-compressed extent decoding via `lzokay` (rare in modern installs) |
| `std` | no | enables `std::error::Error` impl on `Error` (host-only) |

For UEFI bootloaders that may encounter any compression on `/boot`,
enable all three: `features = ["zstd", "zlib", "lzo"]`.

## Documentation

- [`docs/SPEC-LAMBUTTER.md`](docs/SPEC-LAMBUTTER.md) — design spec, on-disk format scope
- [`docs/FEATURES.md`](docs/FEATURES.md) — exhaustive feature inventory + non-features
- [`docs/SUPPORTED-SCENARIOS.md`](docs/SUPPORTED-SCENARIOS.md) — distro × btrfs-config coverage matrix
- [`docs/CONSUMER-GUIDE.md`](docs/CONSUMER-GUIDE.md) — integrating lambutter into a downstream `no_std` Rust project
- [`docs/TROUBLESHOOTING.md`](docs/TROUBLESHOOTING.md) — error catalogue (symptom → cause → fix)
- [`docs/TESTING-AND-FUZZING-PLAN.md`](docs/TESTING-AND-FUZZING-PLAN.md) — test/fuzz strategy + validation results
- [`docs/PRE-PUBLISH-AND-TESTING-PLAN.md`](docs/PRE-PUBLISH-AND-TESTING-PLAN.md) — pre-publish hygiene checklist
- [`CHANGELOG.md`](CHANGELOG.md) — version history

## License

Dual-licensed under MIT or Apache-2.0, at your option. See
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).

## Contributing

Contributions welcome — particularly:

- Additional fixture scenarios (`tests/fixtures/`)
- Per-distro real-world validation (the
  [LamBoot scenarios matrix](https://github.com/lamco-admin/lamboot-dev/blob/main/docs/migration/SCENARIOS-MATRIX.md)
  tracks what's been live-validated across the wider ecosystem)
- v0.2.x scope items: data-block CSUM_TREE verification, snapshot
  enumeration, full subvolume traversal

See `docs/SPEC-LAMBUTTER.md` for design constraints before opening a PR.
