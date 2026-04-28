# lambutter

**`no_std` read-only btrfs reader for UEFI bootloaders and embedded contexts.**

Lambutter is the btrfs counterpart to [`ext4-view`](https://github.com/nicholasbishop/ext4-view-rs):
a pure-Rust, allocation-aware, read-only btrfs filesystem reader designed for
contexts that cannot link `std` — UEFI applications, microcontrollers,
bare-metal kernels, and recovery tooling.

## Status

**Pre-alpha.** Authoring in progress; specification at `docs/SPEC-LAMBUTTER.md`.

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
