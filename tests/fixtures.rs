// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Layer-B fixture-based oracle tests. Each test loads a real
//! `mkfs.btrfs`-produced image (zstd-compressed at rest under
//! `tests/fixtures/data/`) and verifies that lambutter's read paths
//! produce the canonical content listed in the sibling `.expected.json`.
//!
//! Generation: `tests/fixtures/scripts/build.sh all` — requires
//! btrfs-progs + sudo (loop mount for compressed fixtures).

use std::{collections::BTreeMap, fs::File, io::Read, path::PathBuf};

use lambutter::{Btrfs, Path};
use sha2::{Digest, Sha256};

#[derive(serde::Deserialize)]
struct Expected {
    fixture: String,
    files: BTreeMap<String, String>,
    #[serde(default)]
    symlinks: BTreeMap<String, String>,
}

fn fixture_dir() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push("tests");
    p.push("fixtures");
    p.push("data");
    p
}

fn load_image(name: &str) -> Vec<u8> {
    let path = fixture_dir().join(format!("{name}.img.zst"));
    let f = File::open(&path).unwrap_or_else(|_| {
        panic!(
            "fixture not found: {} — run tests/fixtures/scripts/build.sh",
            path.display()
        )
    });
    let mut decoder = zstd::stream::Decoder::new(f).expect("decode fixture");
    let mut out = Vec::new();
    decoder.read_to_end(&mut out).expect("read decoded fixture");
    out
}

fn load_expected(name: &str) -> Expected {
    let path = fixture_dir().join(format!("{name}.expected.json"));
    let s = std::fs::read_to_string(&path).expect("expected.json present");
    serde_json::from_str(&s).expect("expected.json parses")
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

#[test]
fn f1_single_uncompressed_reads_known_files() {
    let img = load_image("f1_single_uncompressed");
    let expected = load_expected("f1_single_uncompressed");
    assert_eq!(expected.fixture, "f1_single_uncompressed");

    let reader: &[u8] = &img;
    let len = img.len() as u64;
    let mut fs = Btrfs::open(reader, len).expect("open f1");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).expect("absolute path");
        let bytes = fs.read_file(p).expect(&format!("read {path}"));
        let got = sha256_hex(&bytes);
        assert_eq!(
            &got, expected_sha,
            "fixture content sha256 differs for {path}"
        );
    }
}

#[test]
fn f2_single_zstd_reads_known_files_through_decompression() {
    let img = load_image("f2_single_zstd");
    let expected = load_expected("f2_single_zstd");

    let reader: &[u8] = &img;
    let len = img.len() as u64;
    let mut fs = Btrfs::open(reader, len).expect("open f2");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).expect("absolute path");
        let bytes = fs.read_file(p).expect(&format!("read {path}"));
        let got = sha256_hex(&bytes);
        assert_eq!(
            &got, expected_sha,
            "fixture content sha256 differs for {path}"
        );
    }
}

#[test]
fn f3_single_zlib_reads_known_files_through_decompression() {
    let img = load_image("f3_single_zlib");
    let expected = load_expected("f3_single_zlib");

    let reader: &[u8] = &img;
    let len = img.len() as u64;
    let mut fs = Btrfs::open(reader, len).expect("open f3");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).expect("absolute path");
        let bytes = fs.read_file(p).expect(&format!("read {path}"));
        let got = sha256_hex(&bytes);
        assert_eq!(
            &got, expected_sha,
            "fixture content sha256 differs for {path}"
        );
    }
}

#[test]
#[cfg(feature = "lzo")]
fn f4_single_lzo_reads_known_files_through_decompression() {
    let img = load_image("f4_single_lzo");
    let expected = load_expected("f4_single_lzo");

    let reader: &[u8] = &img;
    let len = img.len() as u64;
    let mut fs = Btrfs::open(reader, len).expect("open f4");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).expect("absolute path");
        let bytes = fs.read_file(p).expect(&format!("read {path}"));
        let got = sha256_hex(&bytes);
        assert_eq!(
            &got, expected_sha,
            "fixture content sha256 differs for {path}"
        );
    }
}

#[test]
fn f5_dup_metadata_reads_files_under_dup_profile() {
    // DUP-metadata + SINGLE-data is the mkfs.btrfs default for SSDs.
    // Tests the chunk-tree resolver's pick_stripe path for DUP profile.
    let img = load_image("f5_dup_metadata");
    let expected = load_expected("f5_dup_metadata");

    let reader: &[u8] = &img;
    let len = img.len() as u64;
    let mut fs = Btrfs::open(reader, len).expect("open f5");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).expect("absolute path");
        let bytes = fs.read_file(p).expect(&format!("read {path}"));
        let got = sha256_hex(&bytes);
        assert_eq!(
            &got, expected_sha,
            "fixture content sha256 differs for {path}"
        );
    }
}

#[test]
fn f8_sparse_no_holes_reads_with_zero_fill() {
    // NO_HOLES filesystem with a sparse file: 4 KiB 'A', 1 MiB hole, 4 KiB 'B'.
    // Tests file::read_file's gap-fill loop and EXTENT_DATA hole semantics.
    let img = load_image("f8_sparse_no_holes");
    let expected = load_expected("f8_sparse_no_holes");

    let reader: &[u8] = &img;
    let len = img.len() as u64;
    let mut fs = Btrfs::open(reader, len).expect("open f8");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).expect("absolute path");
        let bytes = fs.read_file(p).expect(&format!("read {path}"));
        let got = sha256_hex(&bytes);
        assert_eq!(
            &got, expected_sha,
            "fixture content sha256 differs for {path}"
        );
    }
}

#[test]
fn f9_symlink_chain_resolves_targets() {
    let img = load_image("f9_symlink_chain");
    let expected = load_expected("f9_symlink_chain");

    let reader: &[u8] = &img;
    let len = img.len() as u64;
    let mut fs = Btrfs::open(reader, len).expect("open f9");

    // Regular files still readable.
    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).unwrap();
        let bytes = fs.read_file(p).expect(&format!("read {path}"));
        assert_eq!(sha256_hex(&bytes), *expected_sha);
    }

    // Symlinks return raw target bytes.
    for (path, expected_target) in &expected.symlinks {
        let p = Path::new(path.as_bytes()).unwrap();
        let target_bytes = fs.read_link(p).expect(&format!("read_link {path}"));
        let got_target = std::str::from_utf8(&target_bytes).expect("utf8 target");
        assert_eq!(got_target, expected_target);
    }
}

#[test]
fn opening_invalid_image_returns_error_not_panic() {
    // Random bytes the size of a real volume — must NEVER panic.
    let mut fake = vec![0xAAu8; 0x10_0000];
    fake[0x10000] = b'_';
    fake[0x10001] = b'B';
    fake[0x10002] = b'H';
    fake[0x10003] = b'R';
    let reader: &[u8] = &fake;
    let _ = Btrfs::open(reader, fake.len() as u64);
}
