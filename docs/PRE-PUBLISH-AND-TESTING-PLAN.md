# lambutter — Pre-publish Hygiene + Deep-Testing Plan

**Created:** 2026-05-24
**Status:** Active checklist; finish all items before declaring lambutter
"done" and publishing to crates.io / public GitHub.
**Companion to:** `docs/TESTING-AND-FUZZING-PLAN.md` (which already declared
the v0.1.0 feature scope complete; this doc covers the work between that
declaration and the public release).
**Authoritative scope statement:** `docs/SPEC-LAMBUTTER.md §2`.

---

## 0. Why this doc exists

Lambutter v0.1.1 is **code-complete for its declared scope**. The feature
audit in `docs/TESTING-AND-FUZZING-PLAN.md §3` shows every mandatory
v0.1.0 item as DONE; the only feature gap that was ever open (`read_link`
for symlinks) shipped in v0.1.1. Source-tree grep for
`TODO|FIXME|XXX|unimplemented!|todo!|HACK` returns zero hits.

But "code-complete for scope" is not the same as "ready to publish to
crates.io and pin a LamBoot release against." Two layers of work sit
between those states:

1. **Hygiene** — small polish items that don't affect correctness but
   would embarrass us in a public crate (test-harness defects, warning
   count, stale comments, never-compiled-in-target verification).
2. **Deep testing** — exercising the parser against *deployed reality*
   (real-distro `/boot` images) rather than only synthetic
   `mkfs.btrfs --rootdir` fixtures, plus running the fuzz harnesses for
   meaningful durations rather than just scaffolding them.

Both are bounded work. This doc enumerates them so neither slips.

---

## 1. Hygiene punch list (pre-publish polish)

Each item is small. Total budget: roughly half a day if you're picky
about warnings.

### 1.1 Fix F3 zlib fixture test gating

**Status:** OPEN

**Symptom:** `cargo test --release` (no feature flags) fails on
`f3_single_zlib_reads_known_files_through_decompression` with
`BadCompression { algorithm: "comp_zlib" }`.

**Cause:** `tests/fixtures.rs` line 110 has the F3 test without a
`#[cfg(feature = "zlib")]` gate. The F4 LZO test at line 130 has
`#[cfg(feature = "lzo")]` correctly. F3 is the inconsistency.

**Fix:** Add `#[cfg(feature = "zlib")]` immediately above `fn
f3_single_zlib_reads_known_files_through_decompression`. Verify
`cargo test --release` (no features) is green and
`cargo test --release --features zlib` is green. Both states should
pass.

### 1.2 Release-build warning sweep

**Status:** OPEN

**Symptom:** `cargo build --release` reports `lambutter (lib) generated
67 warnings`. Most are likely `unreachable_pub` and the `pedantic` clippy
set, but some may be genuine cleanups.

**Why it matters for publish:** docs.rs renders the build log publicly.
67 warnings on the first release tells visitors the crate isn't
maintained carefully.

**Fix approach:**
- `cargo build --release 2>&1 | grep "^warning" | sort -u` to see the
  distinct warning kinds.
- For each kind, decide: legitimate code change vs `#[expect(..., reason
  = "...")]` annotation (per CLAUDE.md project convention — `expect` over
  `allow`, with a reason).
- Avoid blanket `#[allow]` at module scope unless the spec-citation
  pattern justifies it (see existing `cast_lossless` etc. allows in
  `Cargo.toml [lints.clippy]`).

**Acceptance:** `cargo build --release` reports 0 warnings.

### 1.3 Stale `v0.1.0` comments in `btree.rs` and `dir.rs`

**Status:** OPEN

**Symptom:** Two source comments still describe cross-leaf iteration as
unimplemented:
- `src/dir.rs:40` — "Lambutter does not paginate across leaves in
  v0.1.0; for very large [...]"
- `src/btree.rs:300` — "follow leaf-to-leaf chaining in v0.1.0 (we stop
  at end of leaf); callers [...]"

**Reality:** Cross-leaf iteration *is* implemented (re-descend pattern,
three call sites — see `docs/TESTING-AND-FUZZING-PLAN.md §3` audit row
"Cross-leaf iteration (re-descend pattern): DONE", and CHANGELOG v0.1.1
ledger). The comments are stale.

**Fix:** Either delete the obsolete sentences or rewrite them to
describe the re-descend pattern that is actually in place. No code
change. Per project comment discipline, prefer deleting over rewriting
unless the comment adds non-obvious "why" value.

### 1.4 UEFI-target compile smoke

**Status:** OPEN — never verified

**Why it matters:** Lambutter is `no_std + alloc` by design and the
public sell is "for UEFI bootloaders and embedded contexts." It has
never been actually compiled against `x86_64-unknown-uefi`. The lints
(`unsafe_code = "deny"`) and dep-feature configuration (every dep is
`default-features = false`) make this *should* just work, but "should"
is not "verified."

**Procedure:**
- Create a throwaway `examples/uefi_compile_check/` (or just add a check
  in CI) that does `let _ = Btrfs::open(reader, len);` against a stub
  `BlockRead` impl.
- Compile with `cargo build --target x86_64-unknown-uefi -Zbuild-std=core,compiler_builtins,alloc --no-default-features --features zstd`.
- Repeat with `--features zlib` and `--features lzo`.

**Acceptance:** All three compile clean. If anything pulls in `std`
transitively, surface it and either gate or replace.

**Note:** This doesn't have to live in the published crate — a script
under `tests/` or a CI job is fine. The point is one-time proof, then a
gate to keep it true.

### 1.5 README status update for publish

**Status:** OPEN

**Symptom:** `README.md` says "**Pre-alpha.** Authoring in progress;
specification at `docs/SPEC-LAMBUTTER.md`." That's fine while the repo
is private. On crates.io it's the wrong message for a v0.1.1 crate with
36 unit tests, 8 oracle fixtures, 5 fuzz harnesses, and full v0.1.0
feature coverage.

**Fix:** Update Status section to something accurate. Suggested wording
(reword to taste):
> **v0.1.x — feature-complete for the declared scope** (see
> `docs/SPEC-LAMBUTTER.md §2`). Not yet exercised against real-distro
> `/boot` deployments; data-block CSUM verification is out of scope for
> v0.1.x and tracked for v0.2.0. Public API may add but not break within
> the v0.1.x line.

Also add: build status badge, crates.io version badge, docs.rs badge —
all standard for a public Rust crate.

---

## 2. Deep-testing phase (before declaring "done")

The existing fixture suite (F1–F9) covers synthetic
`mkfs.btrfs --rootdir` images. That proves the parser handles
btrfs-the-format. It does **not** prove it handles btrfs-as-actually-
deployed by real distros.

These four phases are the legitimate next layer.

### 2.1 Real-distro `/boot` fixture capture

**Status:** OPEN — single highest-value testing item

**Goal:** Verify lambutter resolves `/vmlinuz-*` and `/initrd-*` (and
the BLS `loader/entries/*.conf` files, if present on btrfs `/boot`)
on actual distro installs, and that the byte contents match the
kernel-mounted oracle.

**Distros to capture (in priority order):**

| Distro | Why this one | Risk-register link |
|---|---|---|
| **openSUSE Tumbleweed** | **HIGH×HIGH** in `BTRFS-CRATE-ANALYSIS-2026-04-27 §8`. Snapper-managed default-subvol redirect — only distro that exercises the OID-6 DIR_ITEM `"default"` mechanism on `/boot` reads in a realistic way. This fixture is the one most likely to surface a bug. | YES — top of the risk register |
| Fedora 43 | Largest SB+modern-distro user base; default btrfs on `/` since F33, `/boot` on ext4, so this primarily tests root-fs parsing not `/boot` reads. Less urgent for LamBoot's `/boot` path but valuable for completeness. | — |
| CachyOS | Default btrfs install variant exists. Performance-tuned, may exercise different chunk-allocation patterns. | — |
| Garuda | Snapper-managed too, but with a different default profile than Tumbleweed. Catches Snapper-flavor variance. | — |

**Per-distro procedure:**
1. Boot the distro in a Proxmox VM. Reach first-boot completion (so the
   default subvolume + snapshot tree look like a real install, not a
   minimal post-mkfs image).
2. Shut down cleanly.
3. Capture the btrfs partition (`dd if=/dev/zvol/...` or QEMU image
   export) to a `.img` file.
4. Redact: zero out swap/personal-data ranges if any landed on the
   partition. For `/boot` btrfs this is usually unnecessary — `/boot`
   contains kernels and initrds, which are public.
5. Zstd-compress and commit to `tests/fixtures/data/` as
   `r<N>_<distro>.img.zst` with an `.expected.json` enumerating the
   expected files + sha256s + an `.expected.subvol` for the resolved
   default subvolume objectid.
6. Add a fixture test mirroring the F1/F2 pattern.

**Acceptance:** Each fixture's enumerated files read byte-for-byte
identically to the same files obtained by loop-mounting the image with
the kernel.

**Capture-size note:** Even a minimal `/boot` partition is ~200–500 MiB
raw, ~50–150 MiB zstd-compressed. Four distros = ~300–600 MiB added to
the repo. That's significant; consider Git LFS or an out-of-tree
fixture store with download script before committing.

### 2.2 Fuzz-corpus run

**Status:** OPEN

**Goal:** Move from "fuzz harnesses scaffolded" to "fuzz harnesses ran
for meaningful duration against a seed corpus." Per
`TESTING-AND-FUZZING-PLAN §1` original target: 1 hour smoke per target.

**Procedure (per target, all five — `fuzz_superblock`, `fuzz_btree_node`,
`fuzz_extent_data`, `fuzz_dir_item`, `fuzz_compressed_extent`):**

1. Seed the corpus from real `mkfs.btrfs` images. For each target:
   - Extract the relevant byte ranges from the synthetic fixtures
     (F1–F9) and from the real-distro fixtures (§2.1) and drop them
     into `fuzz/corpus/<target>/`.
2. Run `cargo +nightly fuzz run <target> -- -max_total_time=3600` (1 h
   per target).
3. Triage any crashes / hangs / OOMs. Add regression tests to the host
   unit-test suite for each; do not just fix the bug and toss the
   reproducer.

**Acceptance:** All five targets run 1 hour each with no crashes. Any
reproducers found during the run land as regression tests.

**Stretch acceptance:** 8 hours per target (overnight). This is what
sustained adoption (e.g. if anyone external picks up the crate) will end
up doing anyway; better to find issues now.

### 2.3 Tumbleweed default-subvol redirect, isolated

**Status:** OPEN — partial overlap with §2.1 but explicit because of
risk-register prominence

**Why isolated:** Tumbleweed's `/boot` reads go through Snapper's
default-subvol indirection. Lambutter's `resolve_default_subvol` claims
to handle the OID-6 DIR_ITEM `"default"` path, but that code path has
never been hit by a test. The §2.1 fixture exercises it implicitly; this
item makes the test explicit and the success criterion sharp.

**Procedure:**
- From the Tumbleweed fixture in §2.1, add a host test that calls
  `Btrfs::open(...)`, then `fs.default_subvol_objectid()`, and asserts
  the resolved objectid matches what `btrfs subvolume get-default
  /mnt` reports against the same image.
- Then call `fs.resolve(Path::new(b"/vmlinuz").unwrap())` and assert it
  succeeds. If the default-subvol redirect is broken, `/vmlinuz` will
  fail to resolve.

**Acceptance:** Both assertions pass on the Tumbleweed fixture. If they
fail, that's the v0.1.2 (or pre-publish) fix work.

### 2.4 Pin lambutter into one external `no_std` consumer end-to-end

**Status:** OPEN — overlaps §1.4 but is real-use, not stub-use

**Why:** §1.4 proves lambutter compiles in a UEFI target. This item
proves it *works* when actually called from a UEFI binary with a real
`BlockRead` impl over `EFI_BLOCK_IO_PROTOCOL`. That's the one thing the
host-side fixture suite cannot exercise, because all fixture readers are
`&[u8]`.

**Two paths to do this:**

- **Path A (faster):** stand up a minimal standalone UEFI app under
  `examples/uefi_cat/` (gated behind a `uefi-example` feature or just
  documented as out-of-cargo-workspace build) that:
  - takes a `BLOCK_IO_PROTOCOL` handle for a btrfs partition,
  - mounts via `Btrfs::open`,
  - reads `/vmlinuz` and prints its size + first 16 bytes via UEFI
    console.
  - Runs under QEMU + OVMF with one of the §2.1 fixtures attached as a
    raw disk.

- **Path B (deferred):** wait until LamBoot integration (separate work
  in `lamboot-dev`) and consider the in-anger boot to be sufficient.
  This couples lambutter's "done" gate to LamBoot's integration work,
  which may not be what you want.

**Recommendation:** Path A. It's a few-hundred-line throwaway example
that becomes the canonical "how do I use this crate from UEFI?" demo
that any future external adopter will copy.

**Acceptance:** Example builds under
`x86_64-unknown-uefi -Zbuild-std=...`, runs in QEMU+OVMF, reads
`/vmlinuz` from each of the §2.1 fixtures, prints expected size.

---

## 3. Definition of "done" (publish-readiness gate)

Lambutter is ready to publish when **all of §1 and all of §2 pass**.
Not before.

Checklist form (copy to a project tracker or check off in this doc):

**Hygiene (§1):**
- [ ] 1.1 F3 zlib fixture test feature-gated
- [ ] 1.2 Release build = 0 warnings
- [ ] 1.3 Stale `v0.1.0` comments in `btree.rs` / `dir.rs` removed
- [ ] 1.4 UEFI-target compile smoke verified for all feature combinations
- [ ] 1.5 README status updated for public-crate audience; badges added

**Deep testing (§2):**
- [ ] 2.1 Real-distro `/boot` fixtures captured + tests passing
  - [ ] openSUSE Tumbleweed
  - [ ] Fedora 43
  - [ ] CachyOS
  - [ ] Garuda
- [ ] 2.2 Fuzz-corpus 1 h smoke run per target, no crashes
- [ ] 2.3 Tumbleweed default-subvol redirect explicit test passing
- [ ] 2.4 `examples/uefi_cat/` reads `/vmlinuz` from a fixture under
  QEMU+OVMF

When the checkbox set is complete: lambutter is "done" for v0.1.x. Then
the publish-side work (crates.io publication, public
`github.com/lamco-admin/lambutter` repo push, badges activation,
LamBoot-side integration) is unblocked.

---

## 4. Out of scope for this doc

- **Publishing mechanics** (crates.io account, `cargo publish` ceremony,
  GitHub repo creation, mirror-script setup) — those live in
  `~/lamco-admin/projects/lamco-rust-crates/docs/PUBLISHING-GUIDE.md`.
- **LamBoot-side integration** (`fs_backend_btrfs.rs`, partition
  dispatcher wiring, QEMU harness extension) — those live in
  `~/lamboot-dev/`. The lambutter side does not need to know about them.
- **v0.2.0 scope decisions** (data-block CSUM verification, snapshot
  enumeration, full subvolume traversal) — recorded in
  `docs/SPEC-LAMBUTTER.md §2.3`. Decide once v0.1.x publishes; do not
  expand scope mid-publish-prep.

---

## 5. Change log

- 2026-05-24: Initial doc. Hygiene punch list (5 items) + deep-testing
  phases (4 items) + definition of done.
