# lambutter — Supported Scenarios Matrix

**Purpose:** for every real-world btrfs configuration we know about,
say explicitly whether lambutter can read it, with what caveats, and
what evidence we have.

**Companion docs:**
- `FEATURES.md` — feature-by-feature implementation status
- `TROUBLESHOOTING.md` — when something fails, what the error means
- `SPEC-LAMBUTTER.md` §2 — formal in-scope / out-of-scope declaration

---

## 0. Status legend

| Symbol | Meaning |
|---|---|
| ✓✓ | Validated end-to-end on real distro install with boot-trust.log evidence |
| ✓ | Validated via host-side `examples/inspect.rs` against real volume OR via fixture-based oracle test |
| ⊕ | Code path implemented; never exercised on a real volume of this config; high confidence works |
| ~ | Code path implemented; not exercised; meaningful chance of edge-case issues |
| ✗ | Not supported (out of scope or actively rejected) |
| n/a | Doesn't apply to this combination |

---

## 1. Distro × default btrfs configuration

### 1.1 openSUSE family

| Distro / version | Default config | Status | Evidence |
|---|---|---|---|
| Tumbleweed (pre-2025-11-13 install) | `/boot` directory in `@` subvol; Snapper-managed default-subvol redirect to `@/.snapshots/1/snapshot` | ✓✓ | VM 102 case study, full UEFI boot |
| Tumbleweed (post-2025-11-13 install with `grub2-bls`) | `/boot` moved to FAT ESP; subvolume layout unchanged | ✓ (fixture-equivalent) | grub2-bls path doesn't traverse btrfs `/boot`; root traversal works |
| Leap 16+ | Same as Tumbleweed (Snapper btrfs) | ⊕ | Same code path; VM 130 provisioned for explicit test |
| SLES | Same Snapper layout | ⊕ | Same as Leap |
| MicroOS / Aeon | Transactional-update on btrfs | ~ | Read-only root subvolume + RW overlay; lambutter only ever reads the static base layer, should work but untested |

### 1.2 Fedora family

| Distro / version | Default config | Status | Notes |
|---|---|---|---|
| Fedora Workstation 33+ | `/boot` on ext4 (separate partition); root on btrfs | ⊕ | Only `/` is btrfs; `/boot` reads go through the ext4-view backend; lambutter is exercised for root-FS access only |
| Fedora Silverblue / Kinoite | ostree on ext4 `/boot`; root on btrfs subvolumes | ~ | Atomic-update flow; default subvolume changes per ostree commit |
| Fedora IoT | Similar to Silverblue | ~ | Same caveats |
| Fedora 44+ (grub2-bls rollout) | Same kernel-on-ESP path as modern Tumbleweed | ⊕ | Same UKI path |
| RHEL / Rocky / Alma | `/boot` xfs typically; root may be btrfs only as opt-in | n/a | Default RHEL doesn't put root on btrfs |

### 1.3 Arch family

| Distro / version | Default config | Status | Notes |
|---|---|---|---|
| Arch Linux (btrfs install) | Single-subvol or @-style depending on installer choice; no Snapper unless user adds | ⊕ | Single-subvol case is the simplest path; works |
| EndeavourOS (btrfs option) | Snapper auto-configured (similar to Tumbleweed) | ⊕ | Snapper-style; same code path as VM 102 |
| Manjaro | btrfs as opt-in | ⊕ | Single-subvol typically |
| CachyOS (btrfs default) | btrfs root, often with zstd compression | ~ | High priority next test (per scenarios matrix) |
| Garuda | btrfs default with Snapper (similar to Tumbleweed) | ⊕ | Same code path as VM 102 |
| Artix | Same as Arch but with different init | ⊕ | Same |

### 1.4 Debian / Ubuntu family

| Distro / version | Default config | Status |
|---|---|---|
| Debian (btrfs as opt-in) | Single subvol typically | ⊕ |
| Ubuntu (btrfs as opt-in) | Single subvol typically | ⊕ |
| Pop!_OS | Kernels on ESP via kernelstub; btrfs root if user chose it | ⊕ | LamBoot's Pop!_OS auto-discovery handles ESP-kernel path; lambutter exercised for btrfs root reads only |
| Linux Mint | Same as Ubuntu | ⊕ |

### 1.5 Other

| Distro / version | Status | Notes |
|---|---|---|
| NixOS (btrfs root) | ⊕ | Single-subvol via default config; immutable-style updates |
| Gentoo (btrfs option) | ⊕ | Manual install; whatever the user configured |
| Alpine | n/a | Doesn't default to btrfs |
| Void / Slackware | ⊕ | Same as Arch — single-subvol typically |

---

## 2. btrfs profile coverage

| Profile | Status | Behavior |
|---|---|---|
| `SINGLE` | ✓✓ | Default for `/boot`-on-btrfs single-disk installs |
| `DUP` | ✓ | `mkfs.btrfs` default for metadata on SSDs |
| `RAID1` | ⊕ | Two-copy mirror; we pick the first-locally-resolvable stripe |
| `RAID1C3` | ⊕ | Three-copy mirror |
| `RAID1C4` | ⊕ | Four-copy mirror |
| `RAID0` | ✗ (rejected cleanly) | Striped without parity; `Error::UnsupportedProfile("raid0")` |
| `RAID10` | ✗ (rejected cleanly) | Striped mirror; `Error::UnsupportedProfile("raid10")` |
| `RAID5` | ✗ (rejected cleanly) | Parity stripe; `Error::UnsupportedProfile("raid5")` |
| `RAID6` | ✗ (rejected cleanly) | Dual-parity stripe; `Error::UnsupportedProfile("raid6")` |

The "rejected cleanly" cases mean lambutter does not panic, does not
read garbage data — it returns a typed `Error::UnsupportedProfile`
that downstream consumers can surface as "use a different filesystem
or a different reader".

**Real-world frequency for `/boot`:** RAID0/10/5/6 are essentially
never used for `/boot` partitions (the failure modes are unacceptable
for a boot device). RAID1 is occasionally used on enterprise setups.
SINGLE and DUP dominate.

---

## 3. Snapshot / subvolume scenarios

### 3.1 Default subvolume resolution

| Configuration | Status |
|---|---|
| Default = `FS_TREE_OBJECTID` (= 5, the root of subvolume tree) | ✓ |
| Default = base `@` subvolume (no Snapper or pre-Snapper) | ✓ |
| Default = `@/.snapshots/N/snapshot` via OID-6 DIR_ITEM "default" (Snapper) | ✓✓ |
| Default = arbitrary subvolume picked by `btrfs subvolume set-default` post-install | ⊕ |
| Caller-supplied default override (future API) | ✗ (not implemented in v0.1.x) |

### 3.2 Reading from the active default

| Scenario | Status |
|---|---|
| Read a regular file in the active default subvolume | ✓✓ |
| Read a symlink target in the active default subvolume | ✓✓ |
| Read a directory listing in the active default subvolume | ✓ |
| Read a file in a DIFFERENT subvolume (manual subvol pin) | ✗ — out of scope |
| Cross-subvol references via ROOT_REF | ✗ — actively avoided |

The reasoning: bootloaders need to read kernels from whichever
subvolume the userland was booted into. That's the active default.
Cross-subvol traversal complexity isn't justified for this use case.

---

## 4. Compression coverage

| Compression | Default mode | Detection | Status |
|---|---|---|---|
| Uncompressed | Always works | EXTENT_DATA `compression == 0` | ✓✓ |
| zstd | Modern default (Fedora F34+, Tumbleweed, CachyOS, Garuda) | EXTENT_DATA `compression == 3` | ✓✓ (VM 102 reads zstd-compressed extents) |
| zlib | Legacy default | EXTENT_DATA `compression == 1` | ✓ (fixture F3) |
| LZO | Rare in modern installs | EXTENT_DATA `compression == 2` | ✓ (fixture F4, v0.1.1 added real decoder) |

**Kernel images and initrds on `/boot`:** typically stored uncompressed
on disk even when the filesystem is mounted with `compress=zstd`,
because btrfs's compression heuristic skips already-compressed
payloads. So in practice the most-exercised path is the uncompressed
branch.

---

## 5. Special btrfs features

| Feature | INCOMPAT flag bit | Status |
|---|---|---|
| `MIXED_BACKREF` | bit 0 | ✓ (tolerated — always set on modern filesystems) |
| `DEFAULT_SUBVOL` | bit 1 | ✓ |
| `MIXED_GROUPS` | bit 2 | ✓ (tolerated) |
| `COMPRESS_LZO` | bit 3 | ✓ (with `lzo` feature) |
| `COMPRESS_ZSTD` | bit 4 | ✓ (with `zstd` feature, default) |
| `BIG_METADATA` | bit 5 | ✓ (tolerated — nodesize > sectorsize) |
| `EXTENDED_IREF` | bit 6 | ✓ (tolerated) |
| `RAID56` | bit 7 | ✗ — rejected via UnsupportedProfile |
| `SKINNY_METADATA` | bit 8 | ✓ (tolerated — modern default) |
| `NO_HOLES` | bit 9 | ✓ (zero-fill applied at read time) |
| `METADATA_UUID` | bit 10 | ✓ (tolerated) |
| `RAID1C34` | bit 11 | ✓ |
| `ZONED` | bit 12 | ✗ — rejected via UnsupportedIncompat |
| `EXTENT_TREE_V2` | bit 13 | ✓ (tolerated) |
| `RAID_STRIPE_TREE` | bit 14 | ✗ — rejected via UnsupportedIncompat |
| `SIMPLE_QUOTA` | bit 15 | ✓ (tolerated) |
| (future bits) | — | ⊕ (we reject any flag we don't recognize, defensively) |

The pattern: we explicitly tolerate flags whose presence doesn't
change anything about the read path, and reject flags that would
change addressing or block layout in ways we can't handle.

---

## 6. Edge cases — what works in principle but is under-tested

These are scenarios the code SHOULD handle correctly based on the
implementation, but where we don't have direct test evidence yet:

| Edge case | Confidence | What we'd want as evidence |
|---|---|---|
| Very large file (1+ GiB) via `read_file_at` chunked reads | High | Live test with a hand-constructed large file |
| Very large directory (10000+ entries) | Medium | Synthetic fixture |
| Pathologically nested symlinks (near `SYMLOOP_MAX`) | High — bounded by depth limit | Synthetic fixture |
| Filesystem with many subvolumes (100+) | Medium | Snapper-rich Tumbleweed install |
| Cross-leaf B-tree iteration on huge trees | Medium | Synthetic fixture |
| Block size != 512 (4K-native disk) | High — `block_size` is queried at open | Live test on 4Kn disk |
| Files at the maximum supported size (inode `size` field is u64) | High | Synthetic fixture if/when relevant |
| Mixed BG (data + metadata in same block group) | High | Older mkfs.btrfs default |

---

## 7. Explicitly unsupported scenarios

For each: what users will see if they try.

| Scenario | What happens | Recovery |
|---|---|---|
| Writing to the volume | API doesn't exist; can't be attempted | n/a |
| ZONED btrfs (SMR drives, ZNS) | `Error::BadSuperblock(UnsupportedIncompat)` on `open()` | Use kernel btrfs driver |
| Encrypted btrfs (`fscrypt`) | Will read encrypted bytes (returns garbage); not detected | n/a — btrfs fscrypt isn't widely deployed |
| Reading from a snapshot OTHER than active default | Out of scope | Manually mount the snapshot in Linux, copy out via kernel driver |
| Data-block CSUM verification | Not performed | Trust upstream; metadata CSUMs ARE verified |
| Multi-device with primary device missing | Stripe-selection picks first listed device's blocks; if that device isn't available via the supplied `BlockRead`, reads fail with `Io` error | Provide a multi-device-aware `BlockRead` impl that falls back to remaining devices |

---

## 8. Distro-specific gotchas worth knowing

### openSUSE (any version with Snapper)

- Default subvol IS `@/.snapshots/N/snapshot`, not `@`. Lambutter
  follows the redirect transparently via `Btrfs::open` →
  `resolve_default_subvol` → OID-6 DIR_ITEM "default".
- After `snapper rollback`, the active default subvolume changes.
  Lambutter re-reads it on every `open()`, so a fresh open picks up
  the new default. (Holding a `Btrfs<R>` across a rollback won't —
  the in-memory state is fixed at open time.)
- `/boot/grub2/x86_64-efi` and `/boot/grub2/i386-pc` are separate
  subvolumes nested under `@/boot/grub2/`. From within
  `@/.snapshots/1/snapshot`, those paths are EMPTY directories — the
  nested subvolumes only exist when mounted via fstab. Lambutter
  reads single-subvol-at-a-time and will see them empty. **This is
  correct behavior** — when LamBoot reads `/boot/grub2/...`, it gets
  what the running system would see if it `ls`'d the path without
  the nested subvols mounted.

### Tumbleweed (post-2025-11-13 grub2-bls installs)

- Kernels live on the FAT ESP, not in btrfs `/boot`. Lambutter is
  still used for ROOT filesystem reads (e.g. parsing
  `/etc/os-release` for distro identification, reading
  `/etc/kernel-install/` configs).
- The `rootflags=subvol=...` in BLS entries must be honored by the
  consuming bootloader; lambutter doesn't itself care, but the
  CONSUMER should pass the correct subvol to mount.

### CachyOS (when using f2fs `/boot`)

- f2fs `/boot` doesn't use lambutter — it would use the EfiFs f2fs
  driver (which inherits the GRUB `extra_attr` bug). lambutter is
  irrelevant here.
- If CachyOS uses btrfs `/boot`, it behaves like Arch — single subvol
  typically, lambutter works.

### Fedora Silverblue / Kinoite (ostree-managed btrfs root)

- ostree maintains per-deployment subvolumes. Default subvolume may
  change after `rpm-ostree deploy` / `rpm-ostree rollback`.
- Same caveats as Snapper — fresh `Btrfs::open` always picks the
  current default.

---

## 9. Reporting unsupported scenarios

If you encounter a btrfs configuration lambutter doesn't handle:

1. Run `examples/inspect.rs` against the volume and capture output.
2. Get the `btrfs filesystem show` and `btrfs subvolume list` output
   from the running system (if accessible).
3. Capture the on-disk first 1 MiB:
   ```bash
   sudo dd if=/dev/<part> bs=1M count=1 status=none | xxd | head -50
   ```
4. File at `lamco-admin/lambutter-dev` with:
   - Distro + version
   - btrfs filesystem profile (`btrfs filesystem df /`)
   - mkfs options used at install (if known)
   - The lambutter error message + stable token
   - Whether the kernel btrfs driver can mount it (oracle check)

We'll add the scenario to this matrix once triaged.

---

## 10. Change log

- 2026-05-25: Initial matrix. Captures post-VM-102 validation state.
