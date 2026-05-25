# lambutter

**`no_std` read-only btrfs reader for UEFI bootloaders and embedded contexts.**

Lambutter is the btrfs counterpart to [`ext4-view`](https://github.com/nicholasbishop/ext4-view-rs):
a pure-Rust, allocation-aware, read-only btrfs filesystem reader designed for
contexts that cannot link `std` — UEFI applications, microcontrollers,
bare-metal kernels, and recovery tooling.

## Status

**v0.1.x — feature-complete for the declared scope** (see
[`docs/SPEC-LAMBUTTER.md` §2](docs/SPEC-LAMBUTTER.md)). Open and read regular
files, symlinks, and directory listings on SINGLE / DUP / RAID1 / RAID1C3 /
RAID1C4 btrfs volumes; decode zstd, zlib, and LZO extents; surface unsupported
profiles (RAID0/10/5/6, ZONED, RAID_STRIPE_TREE) as typed errors. The public
API may add but not break within the v0.1.x line. Data-block CSUM verification
and snapshot enumeration are tracked for v0.2.0+.

Test coverage: 35 host unit tests, 9 fixture-based oracle tests (uncompressed,
zstd, zlib, LZO, DUP-metadata, NO_HOLES sparse, symlinks, read-at chunked),
5 fuzz harnesses.

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

## License

Dual-licensed under MIT or Apache-2.0, at your option. See
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE).
