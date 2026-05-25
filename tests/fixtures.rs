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
    use std::fmt::Write as _;
    let mut h = Sha256::new();
    h.update(bytes);
    let digest = h.finalize();
    let mut s = String::with_capacity(64);
    for b in digest {
        write!(&mut s, "{b:02x}").expect("write to String never fails");
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
#[cfg(feature = "zstd")]
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
#[cfg(feature = "zlib")]
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
#[cfg(feature = "zstd")]
fn read_file_at_chunked_matches_full_read() {
    // Reading a file in chunks via read_file_at must produce byte-identical
    // output to a single read_file call. Exercise across an uncompressed
    // fixture (regular extents), a compressed fixture (zstd extents), and
    // a sparse fixture (NO_HOLES gap-fill). Gated on zstd because the F2
    // case uses zstd-compressed extents.
    let cases: &[&str] = &[
        "f1_single_uncompressed",
        "f2_single_zstd",
        "f8_sparse_no_holes",
    ];

    for fixture in cases {
        let img = load_image(fixture);
        let expected = load_expected(fixture);

        for (path, expected_sha) in &expected.files {
            // Open twice — once for read_file (allocates full size), once
            // for read_file_at (independent reader stream).
            let reader_full: &[u8] = &img;
            let mut fs_full = Btrfs::open(reader_full, img.len() as u64).expect("open full");
            let p = Path::new(path.as_bytes()).unwrap();
            let full = fs_full
                .read_file(p)
                .unwrap_or_else(|e| panic!("{fixture}: read_file({path}) failed: {e:?}"));
            assert_eq!(sha256_hex(&full), *expected_sha, "{fixture}: full read sha");

            let reader_chunk: &[u8] = &img;
            let mut fs_chunk = Btrfs::open(reader_chunk, img.len() as u64).expect("open chunk");
            let inode = fs_chunk.resolve(p).expect("resolve");

            // Read in 4 KiB chunks until EOF.
            let mut accumulated = Vec::new();
            let mut offset: u64 = 0;
            let mut chunk = vec![0u8; 4096];
            loop {
                let n = fs_chunk
                    .read_file_at(&inode, offset, &mut chunk)
                    .unwrap_or_else(|e| panic!("{fixture}: read_file_at off={offset}: {e:?}"));
                if n == 0 {
                    break;
                }
                accumulated.extend_from_slice(&chunk[..n]);
                offset += n as u64;
            }
            assert_eq!(
                accumulated, full,
                "{fixture}: chunked read at {path} differs from full read",
            );

            // Read a single mid-file byte; must equal the same byte from the full read.
            if full.len() >= 1024 {
                let inode2 = fs_chunk.resolve(p).expect("resolve again");
                let mut one = [0u8; 1];
                let n = fs_chunk
                    .read_file_at(&inode2, 512, &mut one)
                    .expect("single byte");
                assert_eq!(n, 1);
                assert_eq!(one[0], full[512], "{fixture}: byte at 512 differs");
            }

            // Read past EOF: must return 0 without error.
            let n = fs_chunk
                .read_file_at(&inode, full.len() as u64 + 100, &mut chunk)
                .expect("past EOF should return 0");
            assert_eq!(n, 0, "{fixture}: past-EOF read should return 0");
        }
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
