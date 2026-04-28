# lambutter — Development Rules

**`no_std` read-only btrfs filesystem reader written in Rust.**

Authoritative design spec: `docs/SPEC-LAMBUTTER.md`.
Authoritative format spec: `~/lamboot-dev/docs/analysis/BTRFS-FORMAT-READONLY-REFERENCE-2026-04-27.md`.

This file inherits and applies the lamco-admin integrated-development framework
(`~/lamco-admin/shared/integrated-development/`). Sections below mirror the
project-specific subset of `CLAUDE-RULES.md` adapted to this crate's scope.

## Scope guards

- **Read-only.** `BlockRead::write_at` does not exist. Any change that
  introduces a write path requires the founder to amend the design spec first.
- **`no_std` + `alloc`.** No `std`, no `tokio`, no async, no threading
  primitives. The `std` feature exists only to gate `std::error::Error` impls.
- **No dependence on other btrfs Rust crates.** Lambutter is implemented
  independently from the on-disk-format specification cited in the design
  spec. Inspiration may be drawn from public references; verbatim code copy
  from any third-party crate is not permitted.

## Build

```
cargo check                  # default features (zstd)
cargo check --no-default-features
cargo check --all-features
cargo test
cargo clippy
rustup run nightly cargo fmt --check
```

The `.rustfmt.toml` uses nightly-only options (`imports_granularity`,
`group_imports`); `cargo fmt` invocations must use the nightly toolchain.

## Lint policy

`Cargo.toml` `[lints.rust]` and `[lints.clippy]` are the single source of
truth. The pre-commit hook runs `cargo clippy` (no `-D warnings` override) so
progressive `warn` lints (`unwrap_used`, `expect_used`) stay out of the way
until the codebase is clean enough to upgrade them to `deny`.

- Suppression uses `#[expect(..., reason = "...")]`. `#[allow(...)]` is
  rejected by `clippy::allow_attributes = "deny"`.
- `unsafe_code` is `deny`. Any module needing `unsafe` declares
  `#![expect(unsafe_code, reason = "...")]` at module scope with a documented
  justification.
- `wildcard_imports` is `deny`.

## Comment discipline

- Never write "what" comments that restate code.
- Only write "why" comments — non-obvious decisions, hidden invariants, spec
  citations, workarounds for specific bugs.
- No template doc comments. Skip `# Arguments` / `# Returns` for obvious
  signatures. `# Errors` only when the error set is non-obvious from the type.
- `// SAFETY:` is mandatory on every `unsafe` block. State the invariant.
- Calibrate density. Tricky code gets more comments. Mechanical code gets
  none.

## Naming discipline

- No generic suffixes: avoid `-Manager`, `-Helper`, `-Processor`, `-Utility`.
- No generic verbs: avoid `process()`, `handle()`, `do_thing()`, `run()`.
- No helper modules: never create `utils.rs`, `helpers.rs`, `common.rs`. (One
  small `util.rs` is permitted by the design spec; new helper buckets are not.)
- Domain-specific verbs: `parse_superblock()`, `resolve_chunk()`,
  `walk_btree_leaf()`, `decode_extent_data()`.
- Btrfs on-disk struct names mirror upstream (`btrfs_dir_item`,
  `btrfs_extent_data_ref`) where the field-by-field correspondence matters
  for spec audit; Rust idiomatic CamelCase is used for our own types
  (`ChunkResolver`, `FsTreeWalker`).

## Abstraction discipline

- No single-implementation traits. The one trait we expose is `BlockRead`,
  which has many caller-side implementations by design.
- No premature generics. Take the concrete type.
- Three similar lines is better than a premature abstraction.
- Max nesting: 3 levels. Use early returns for preconditions.
- Happy path left-aligned. Guard clauses at the top.

## Error handling calibration

- Boundaries get detailed errors: malformed superblock, unsupported feature,
  unsupported profile, csum mismatch — all distinct variants of `Error`.
- Internal code trusts invariants. Don't validate what the type system or a
  prior boundary check guarantees.
- Use `?` propagation. Don't wrap errors with redundant context.
- `Error` variants carry `&'static str` stable tokens documented in the
  design spec §11. Adding a token is a minor bump; renaming or removing is a
  major bump.

## Spec audit posture

Per founder direction 2026-04-27: **every change is audited against the
on-disk-format spec section by section.** Implementation rules:

- Each on-disk struct definition cites the format-reference section that
  authorizes it (a one-line `// spec: §X.Y` comment is sufficient).
- Each walker / parser cites the section whose algorithm it implements.
- Deviations from the spec require an inline justification comment and are
  flagged in the PR.

The PR template (to be added when GitHub repo lands) enforces this checklist.

## Test discipline

- Layer A — host unit tests (`cargo test`): pure logic against in-memory
  buffers. Bulk of test surface.
- Layer B — fixture tests: `mkfs.btrfs`-produced filesystem images,
  zstd-compressed in repo, decompressed at test-time. Generation scripts
  in `tests/fixtures/scripts/` so CI rebuilds them.
- Layer C — fuzz targets: `cargo-fuzz` per parser surface. Land alongside
  the code they cover, not deferred.

## Commit discipline

- Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `chore:`,
  `test:`, `perf:`.
- No AI attribution. Never `Co-Authored-By: Claude`, never any AI reference.
- Explain "why" in body. The diff shows "what".
- No phase numbering. Describe what changed.

## Pre-submission checks

- Pre-commit hook runs `cargo fmt --check` (nightly), `cargo clippy`, and
  `cargo test --lib`. Hooks live in `.githooks/`; `core.hooksPath` is set
  per-clone via `git config core.hooksPath .githooks`.
- Manual: scan the diff for `XX`, `TODO`, `FIXME`, `PLACEHOLDER`, `YYYY`
  before submitting.
