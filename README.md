# lambutter

[![Crates.io](https://img.shields.io/crates/v/lambutter.svg)](https://crates.io/crates/lambutter)
[![Docs.rs](https://docs.rs/lambutter/badge.svg)](https://docs.rs/lambutter)
[![CI](https://github.com/lamco-admin/lambutter/actions/workflows/ci.yml/badge.svg)](https://github.com/lamco-admin/lambutter/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)
[![Website](https://img.shields.io/badge/website-lamco.ai-blue.svg)](https://lamco.ai/products/lamboot/)

**`no_std` read-only btrfs reader for UEFI bootloaders and embedded contexts.**

Lambutter is the btrfs counterpart to [`ext4-view`](https://github.com/nicholasbishop/ext4-view-rs):
a pure-Rust, allocation-aware, read-only btrfs filesystem reader designed for
contexts that cannot link `std` — UEFI applications, microcontrollers,
bare-metal kernels, and recovery tooling.

- **Website:** [LamBoot](https://lamco.ai/products/lamboot/), the bootloader lambutter was written for
- **Documentation:** [docs.rs/lambutter](https://docs.rs/lambutter) and the [LamBoot docs](https://lamco.ai/products/lamboot/docs/)
- **Repository:** [github.com/lamco-admin/lambutter](https://github.com/lamco-admin/lambutter)
- **Company:** [Lamco Development LLC](https://lamco.ai/)

## Status

**0.4.0.** Open and read regular files, symlinks and directory listings
on SINGLE / DUP / RAID1 / RAID1C3 / RAID1C4 btrfs volumes, at the default
subvolume or one chosen by id or path, crossing into nested subvolumes the
way a Linux mount does. Decodes zstd, zlib and LZO extents, and refuses
RAID0/10/5/6, ZONED, RAID_STRIPE_TREE, REMAP_TREE and any unknown
incompat feature with a typed error. See
[`docs/SPEC-LAMBUTTER.md` §2](https://github.com/lamco-admin/lambutter/blob/main/docs/SPEC-LAMBUTTER.md)
for scope and [`CHANGELOG.md`](https://github.com/lamco-admin/lambutter/blob/main/CHANGELOG.md)
for what changed. Data-block CSUM verification and snapshot enumeration
are not implemented.

**Validation posture:**
- 53 host unit tests across all modules
- 12 fixture-based oracle tests on images written by the Linux kernel
  (uncompressed, zstd, zlib, LZO, DUP metadata, NO_HOLES sparse,
  symlinks, extents and directories spanning B-tree leaves, varied LZO
  segments, subvolumes and snapshots, chunked reads, smoke)
- 5 cargo-fuzz harnesses
- Live host validation: `examples/inspect.rs` against live openSUSE
  Tumbleweed btrfs partition with Snapper-managed default-subvol
  redirect — 6 path reads (regular file 38 MB full-extent walk +
  inline 11 B + 1.2 KB normal, symlinks in both `/boot` and `/etc`)
  all sha256-byte-identical to kernel oracle
- **Live embedded validation**: linked into [LamBoot](https://lamco.ai/products/lamboot/)
  (Lamco's UEFI bootloader) as the native btrfs backend, booting
  openSUSE Tumbleweed end-to-end under live UEFI firmware with full
  trust-chain attribution.

## Installation

```sh
cargo add lambutter
```

or in `Cargo.toml`, with every codec a `/boot` might use:

```toml
[dependencies]
lambutter = { version = "0.4", features = ["zstd", "zlib", "lzo"] }
```

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
[Consumer Guide](https://github.com/lamco-admin/lambutter/blob/main/docs/CONSUMER-GUIDE.md).

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

- [`docs/SPEC-LAMBUTTER.md`](https://github.com/lamco-admin/lambutter/blob/main/docs/SPEC-LAMBUTTER.md) — design spec, on-disk format scope
- [`docs/FEATURES.md`](https://github.com/lamco-admin/lambutter/blob/main/docs/FEATURES.md) — exhaustive feature inventory + non-features
- [`docs/SUPPORTED-SCENARIOS.md`](https://github.com/lamco-admin/lambutter/blob/main/docs/SUPPORTED-SCENARIOS.md) — distro × btrfs-config coverage matrix
- [`docs/CONSUMER-GUIDE.md`](https://github.com/lamco-admin/lambutter/blob/main/docs/CONSUMER-GUIDE.md) — integrating lambutter into a downstream `no_std` Rust project
- [`docs/TROUBLESHOOTING.md`](https://github.com/lamco-admin/lambutter/blob/main/docs/TROUBLESHOOTING.md) — error catalogue (symptom → cause → fix)
- [`docs/TESTING-AND-FUZZING-PLAN.md`](https://github.com/lamco-admin/lambutter/blob/main/docs/TESTING-AND-FUZZING-PLAN.md) — test/fuzz strategy + validation results
- [`docs/PRE-PUBLISH-AND-TESTING-PLAN.md`](https://github.com/lamco-admin/lambutter/blob/main/docs/PRE-PUBLISH-AND-TESTING-PLAN.md) — pre-publish hygiene checklist
- [`CHANGELOG.md`](https://github.com/lamco-admin/lambutter/blob/main/CHANGELOG.md) — version history

## License

Dual-licensed under MIT or Apache-2.0, at your option, copyright Lamco
Development LLC. See
[`LICENSE-MIT`](https://github.com/lamco-admin/lambutter/blob/main/LICENSE-MIT) and [`LICENSE-APACHE`](https://github.com/lamco-admin/lambutter/blob/main/LICENSE-APACHE).

## Contributing

Issues and pull requests are welcome at
[github.com/lamco-admin/lambutter](https://github.com/lamco-admin/lambutter).
A change to the read path should come with a fixture:
`tests/fixtures/scripts/build.sh` builds every image from a real
`mkfs.btrfs` and kernel mount. Fixture scenarios and per-distro validation
reports are especially useful. See `docs/SPEC-LAMBUTTER.md` for design
constraints before opening a PR.
