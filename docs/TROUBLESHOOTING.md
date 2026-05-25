# lambutter — Troubleshooting

**Format:** symptom → cause → resolution → prevention. Indexed by what
the operator (or developer integrating lambutter) sees first.

**Companion docs:**
- `FEATURES.md` — what lambutter supports / doesn't
- `SUPPORTED-SCENARIOS.md` — distro × btrfs-config matrix
- `CONSUMER-GUIDE.md` — integration patterns

---

## 1. `Btrfs::open` errors

### 1.1 `Error::BadSuperblock(BadMagic)`

**Symptom:** `open()` returns `Err(Error::BadSuperblock(SuperblockReason::BadMagic))`.

**Cause:** the bytes at the four canonical superblock locations
(`0x10000`, `0x4000000`, `0x4000000000`, `0x4000000000000`) don't
contain the btrfs magic `_BHRfS_M` at offset 0x40 from the superblock
start. Usually means:
- The volume isn't actually btrfs (mis-identified by caller — try
  `file -s /dev/<part>` from a Linux host)
- The volume is encrypted at the block layer (LUKS) and the caller is
  reading the encrypted blocks directly
- The device path is wrong (e.g. handed `/dev/sda` instead of
  `/dev/sda2`)

**Resolution:**
1. From a Linux host, confirm with `sudo file -s /dev/<part>` —
   should print `BTRFS Filesystem`.
2. If LUKS: decrypt via the kernel first, then point lambutter at
   the device-mapper node (`/dev/mapper/<name>`), NOT the underlying
   encrypted partition.
3. Verify the offset arithmetic in your `BlockRead` impl — if you're
   mistakenly treating partition offsets as disk offsets, you'd hit
   `BadMagic` because you'd be reading the wrong region.

**Prevention:** confirm filesystem type via probe (e.g. read magic
bytes yourself before passing to lambutter) rather than blindly
opening every block device.

---

### 1.2 `Error::BadSuperblock(BadCsum)`

**Symptom:** `open()` returns `Err(Error::BadSuperblock(SuperblockReason::BadCsum))`.

**Cause:** every readable superblock copy has a CRC32C body checksum
that doesn't match the stored value. Either:
- Disk corruption at the superblock locations (rare)
- Caller's `BlockRead` is returning wrong bytes (more likely — your
  read implementation has a bug)

**Resolution:**
1. Run `btrfs check --readonly /dev/<part>` from Linux as oracle. If
   it agrees the superblock is bad, you have real corruption — try
   `btrfs rescue super-recover` from Linux.
2. If kernel btrfs mounts cleanly but lambutter says BadCsum, your
   `BlockRead` impl is wrong. Common bugs:
   - Reading from the wrong offset (e.g. forgetting partition base)
   - Off-by-one in block alignment maths
   - Returning shorter-than-requested data silently

**Prevention:** test your `BlockRead` impl against a slice-backed
reader (`&[u8]`) using a known-good btrfs image as fixture.

---

### 1.3 `Error::BadSuperblock(UnsupportedIncompat)`

**Symptom:** `open()` errors with this on an otherwise valid btrfs
volume.

**Cause:** the superblock has an INCOMPAT flag set that lambutter
doesn't implement. Specifically:
- `ZONED` (bit 12) — for SMR drives and ZNS SSDs
- `RAID_STRIPE_TREE` (bit 14) — a newer chunk-addressing model

Lambutter rejects unknown INCOMPAT flags defensively rather than
risk reading garbage.

**Resolution:**
- ZONED: not supported. Use the kernel btrfs driver. Lambutter's
  scope is non-zoned (sequential) media.
- RAID_STRIPE_TREE: not yet supported. File an issue if you encounter
  a real consumer needing this; we can prioritize.

**Prevention:** check `btrfs inspect-internal dump-super /dev/<part>`
from Linux to inventory INCOMPAT flags before attempting to read with
lambutter.

---

### 1.4 `Error::BadSuperblock(BadGeometry)`

**Symptom:** `open()` rejects the volume citing geometry.

**Cause:** superblock-reported `sectorsize` or `nodesize` is outside
the 4 KiB ≤ N ≤ 65 KiB range lambutter supports.

**Resolution:** real-world btrfs always uses 4 KiB or 16 KiB nodesize
in practice. If you see this on a real volume, something is corrupt;
verify with `btrfs filesystem df` from Linux.

**Prevention:** rare in practice.

---

### 1.5 `Error::UnsupportedProfile("raid0" | "raid10" | "raid5" | "raid6")`

**Symptom:** `open()` succeeds (superblock is OK), but the first
attempt to read from a chunk fails with this error.

**Cause:** the chunk being addressed lives on a profile lambutter
doesn't support. `/boot` partitions in the real world essentially
never use these profiles — failures here usually mean the caller
pointed lambutter at the wrong device (e.g. a `/var` partition that's
on a RAID5 array).

**Resolution:** verify with `btrfs filesystem df /<mountpoint>` that
the partition you're reading actually has SINGLE/DUP/RAID1/RAID1C3/
RAID1C4 chunks for both data and metadata.

**Prevention:** when designing your boot configuration, keep `/boot`
on a profile lambutter supports. The Linux btrfs convention already
discourages RAID5/6 for boot.

---

## 2. Path resolution errors

### 2.1 `Error::NotFound` on a path you know exists

**Symptom:** `resolve("/boot/vmlinuz")` or similar returns NotFound
even though `ls /boot/vmlinuz` from the running system shows the file.

**Possible causes:**
1. **Wrong subvolume.** Lambutter reads from the *active default
   subvolume only*. If `/boot` lives in a non-default subvolume in
   the FS hierarchy (rare), lambutter won't see it. Check `btrfs
   subvolume get-default /` from Linux to see what lambutter thinks
   the default is.
2. **Snapper default redirect.** On openSUSE etc., the default subvol
   is `@/.snapshots/N/snapshot`, not `@`. Lambutter follows this
   redirect transparently at `open()` time. If the running system is
   booted from a DIFFERENT snapshot than the current default (e.g.
   you're testing a snapshot you haven't rolled back to yet),
   lambutter sees the default, you see something else.
3. **Path canonicalization mismatch.** Lambutter's `Path::new()`
   requires absolute paths. Confirm your consumer code is constructing
   paths starting with `/`.
4. **Hash collision edge case.** Lambutter handles DIR_ITEM hash
   collisions by walking the packed tail — but if you're hitting a
   crate bug here, the symptom would be inconsistent NotFound.
   Reproduce with `examples/inspect.rs` from a host running the same
   lambutter version.

**Resolution:**
- Check `fs.default_subvol_objectid()` and compare against `btrfs
  subvolume get-default /` from Linux. Should match.
- Check the actual file path inside the default subvol — from Linux,
  `sudo btrfs subvolume snapshot -r / /mnt/active-default-snap; ls
  /mnt/active-default-snap/boot/`.

---

### 2.2 `Error::NotARegularFile` on what should be a file

**Symptom:** `read_file("/boot/vmlinuz")` returns this error.

**Cause:** the path resolves to a symlink, directory, or special file
— not a regular file. Lambutter's design is `read_file()` only works
on regular files.

**Resolution:** check what the path actually is:

```rust
let inode = fs.resolve(path)?;
let md = fs.metadata(&inode)?;
if md.is_symlink() { /* call read_link, then re-resolve */ }
else if md.is_dir() { /* call read_dir, not read_file */ }
else { /* unexpected; not a regular/symlink/dir */ }
```

If it IS a symlink: implement the symlink-follow pattern from
`CONSUMER-GUIDE.md` §3.

**Prevention:** always call `metadata()` first if you don't know
whether the path is a regular file, symlink, or directory.

---

### 2.3 `Error::NotASymlink`

**Symptom:** `read_link(path)` errors.

**Cause:** the path resolves but isn't a symlink. `read_link` is the
symlink-target accessor; calling it on a regular file or directory
errors deliberately.

**Resolution:** check `metadata().is_symlink()` first; only call
`read_link` when it returns true.

---

### 2.4 `Error::CorruptBtree { token, logical }`

**Symptom:** any tree walk returns this; common tokens are
`key_order`, `item_region_overflow`, `child_count`, `depth_limit`.

**Cause:** a B-tree node violates structural invariants. Either:
- Real disk corruption (rare but possible)
- A malicious or adversarially-crafted image (lambutter is fuzz-
  hardened against these; rejecting cleanly is the correct response)
- A lambutter parsing bug (very rare; if reproducible please file)

**Resolution:**
1. From Linux: `sudo btrfs check --readonly /dev/<part>`. If kernel
   also reports corruption, you have real disk damage — use `btrfs
   rescue` to attempt recovery, or restore from backup.
2. If kernel reports clean but lambutter errors: capture the first 64
   MiB of the volume (`dd if=/dev/<part> bs=1M count=64 of=/tmp/repro.img`)
   plus the lambutter version + error token + logical bytenr, file a
   bug.

**Prevention:** for boot-critical scenarios, periodic `btrfs scrub`
runs catch developing corruption before it bites.

---

## 3. Compressed extent errors

### 3.1 `Error::BadCompression { algorithm: "comp_zlib" }`

**Symptom:** a `read_file` against a file with zlib-compressed
extents errors.

**Cause:** one of:
1. The `zlib` Cargo feature wasn't enabled in your dependency
   declaration. Lambutter compiles with feature gates; if `zlib`
   isn't enabled, encountering a zlib-compressed extent surfaces as
   `BadCompression` rather than silently producing wrong data.
2. The compressed payload is itself corrupt (rare).

**Resolution:**
- Enable the feature: `features = ["zstd", "zlib", "lzo"]` in your
  consumer's Cargo.toml.
- If the feature IS enabled and you still see this, file a bug with
  the offending volume / extent.

---

### 3.2 `Error::BadCompression { algorithm: "comp_lzo" }` or `"comp_zstd"`

Same analysis as §3.1. Enable the corresponding feature flag.

---

### 3.3 `Error::OutOfMemory { token: "extent_decompress_buf" }`

**Symptom:** reading a compressed file errors with OOM.

**Cause:** the decompressed output would exceed
`MAX_DECOMPRESSED_EXTENT_BYTES = 16 MiB` for a single extent. Either:
- A genuinely huge extent (rare — btrfs typically splits files into
  smaller extents)
- A malformed compressed stream claiming output much larger than
  reality

**Resolution:** if this is a real-world file you need to read, the
16 MiB cap may need to be reconsidered. File an issue. For now, the
cap is a DOS protection.

---

## 4. I/O errors

### 4.1 `Error::Io { token, offset }`

**Symptom:** any operation can fail with this.

**Cause:** your `BlockRead` impl returned an error. The `token` is
your impl's error (erased to `&'static str` at the boundary); the
`offset` is the byte offset of the failed read.

**Resolution:**
- Check your `BlockRead` impl's error path. Common causes:
  - The underlying medium is failing (read errors at specific
    offsets)
  - Caller passed wrong device size to `Btrfs::open`, so lambutter
    asks for offsets beyond device end
  - In UEFI: BlockIO returned a non-success status; check
    `efi_block_io.read_blocks(...)` return value

**Prevention:** make your `BlockRead::Error` type carry enough
information to diagnose (e.g. include the underlying OS errno).
Lambutter erases to a token for its own error vocabulary, but if
you're debugging, having the rich error in your impl helps.

---

## 5. Build / dependency issues

### 5.1 "ruzstd not found" / "miniz_oxide not found" / "lzokay not found"

**Symptom:** `cargo build` fails complaining about a missing crate.

**Cause:** the corresponding compression feature isn't enabled.

**Resolution:** add the feature to your dependency line:
```toml
lambutter = { version = "...", features = ["zstd", "zlib", "lzo"] }
```

---

### 5.2 "std::error::Error not implemented for lambutter::Error" in no_std build

**Symptom:** consumer code trying to use `Box::new(err) as Box<dyn
Error>` fails to compile.

**Cause:** lambutter's `std::error::Error` impl is gated on the
`std` Cargo feature. In `no_std` contexts, this impl isn't present,
and `core::error::Error` isn't auto-implemented either.

**Resolution:** in `no_std` consumers, don't try to box lambutter's
error as `dyn Error`. Translate to your own error type instead (see
CONSUMER-GUIDE.md §4 for the pattern). LamBoot solves this by
matching on the error variant and either translating to a stable
token or wrapping in a `Corrupt` variant.

---

### 5.3 "the trait `core::error::Error` is not implemented for `lambutter::Error`"

Same as §5.2. Solution: don't try to box; translate.

---

## 6. UEFI-context-specific issues

### 6.1 `BlockIO::read_blocks` returns `DEVICE_ERROR` randomly

**Symptom:** reads work most of the time but occasionally fail with
DEVICE_ERROR.

**Cause:** in OVMF, partition handles sometimes lose their underlying
BlockIO temporarily during driver-loading transitions. Pre-SDS-2
LamBoot hit this when loading EfiFs drivers via BS->LoadImage which
re-enumerated handles.

**Resolution:** open the BlockIO protocol once at backend
construction and hold it (LamBoot's BtrfsBackend takes ownership via
`open_protocol_exclusive`). Don't re-open per-read.

**Prevention:** the `BlockIoReader` pattern from CONSUMER-GUIDE §2.2
holds the ScopedProtocol for the lifetime of the reader, avoiding
this issue.

---

### 6.2 Reads silently corrupt on partition handles in some firmware

**Symptom:** `Btrfs::open` succeeds but file reads return garbage.

**Cause:** some UEFI firmware implementations have buggy
EFI_DISK_IO_PROTOCOL. EFI_BLOCK_IO_PROTOCOL is more reliable.
LamBoot's history with this: SDS-2 originally chose DiskIO; v1.1
amendment reverted to BlockIO after PR-3 bring-up showed DiskIO is
not installed on partition handles in plain OVMF Q35 firmware.

**Resolution:** use BlockIO, not DiskIO. The BlockIoReader pattern
from CONSUMER-GUIDE.md §2.2 does this.

---

## 7. Edge cases that aren't errors but surprise consumers

### 7.1 `default_subvol_objectid()` returns 5 even though the system has Snapper

**Cause:** the system was installed without Snapper, OR Snapper was
disabled and the default-subvolume DIR_ITEM was deleted, OR the user
ran `btrfs subvolume set-default 5 /` to reset to base.

**Resolution:** this is correct behavior. Compare with `btrfs
subvolume get-default /` from Linux to confirm.

---

### 7.2 `read_dir` on `/boot/grub2/x86_64-efi` returns empty on openSUSE

**Cause:** `/boot/grub2/x86_64-efi` is a *separate* btrfs subvolume on
openSUSE. From within the active default subvol, the mount point
appears as an empty directory (the nested subvol is only "filled in"
when mounted by the kernel via fstab).

**Resolution:** this is correct behavior — lambutter reads
single-subvol-at-a-time. To read the contents of the nested subvol,
you'd need to open the same `BlockRead` with a different lambutter
instance after setting up the right subvol resolution (not currently
in scope).

**Prevention:** when authoring per-distro support code, document that
nested-subvol contents won't be visible.

---

## 8. Reporting bugs

If your issue isn't in this catalogue:

1. Capture: lambutter version, your consumer's version,
   the failing call + arguments, the error variant + token, the
   distro of the volume being read.
2. From the running Linux on the same volume, capture:
   `sudo btrfs filesystem show /`, `sudo btrfs subvolume get-default /`,
   `sudo btrfs filesystem df /`, `sudo file -s /dev/<part>`.
3. If reproducible with a synthetic image, attach the image
   (zstd-compressed if >1 MiB).
4. File at `lamco-admin/lambutter-dev`.

---

## 9. Change log

- 2026-05-25: Initial troubleshooting catalogue. Covers superblock
  errors, path-resolution errors, compression issues, I/O errors,
  build issues, UEFI-context-specific issues, surprising edge cases.
