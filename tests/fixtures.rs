// Copyright 2025-2026 Lamco Development LLC
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

// Integration-test code: panicking on failure (unwrap/expect) is intended.
#![expect(clippy::unwrap_used, clippy::expect_used, clippy::expect_fun_call)]

use std::{collections::BTreeMap, fs::File, io::Read, path::PathBuf};

use lambutter::{Btrfs, Path};
use sha2::{Digest, Sha256};

#[derive(serde::Deserialize)]
struct Expected {
    fixture: String,
    files: BTreeMap<String, String>,
    #[serde(default)]
    symlinks: BTreeMap<String, String>,
    /// Directory path to the number of entries `read_dir` must return.
    #[serde(default)]
    #[cfg_attr(
        not(feature = "zstd"),
        expect(
            dead_code,
            reason = "only the zstd-gated F10 test checks directory counts"
        )
    )]
    dirs: BTreeMap<String, usize>,
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

/// Read `path` 64 KiB at a time through `read_file_at`.
fn read_chunked(fs: &mut Btrfs<&[u8]>, path: &str) -> Vec<u8> {
    let inode = fs.resolve(Path::new(path.as_bytes()).unwrap()).unwrap();
    let mut out = Vec::new();
    let mut chunk = vec![0u8; 65536];
    loop {
        let n = fs
            .read_file_at(&inode, out.len() as u64, &mut chunk)
            .unwrap_or_else(|e| panic!("read_file_at({path}) at {}: {e:?}", out.len()));
        if n == 0 {
            return out;
        }
        out.extend_from_slice(&chunk[..n]);
    }
}

#[test]
#[cfg(feature = "zstd")]
fn f10_multi_leaf_reads_extents_and_entries_across_leaves() {
    // 4 KiB nodes: the 8 MiB file's compressed extents and the entries
    // directory's index (with gaps from deletions) both span leaves.
    let img = load_image("f10_multi_leaf");
    let expected = load_expected("f10_multi_leaf");

    let reader: &[u8] = &img;
    let mut fs = Btrfs::open(reader, img.len() as u64).expect("open f10");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).unwrap();
        let whole = fs.read_file(p).expect(&format!("read {path}"));
        assert_eq!(sha256_hex(&whole), *expected_sha, "read_file {path}");
        assert_eq!(
            sha256_hex(&read_chunked(&mut fs, path)),
            *expected_sha,
            "read_file_at {path}"
        );
    }
    assert!(!expected.dirs.is_empty());
    for (path, count) in &expected.dirs {
        let entries = fs
            .read_dir(Path::new(path.as_bytes()).unwrap())
            .expect(&format!("read_dir {path}"));
        assert_eq!(entries.len(), *count, "entries in {path}");
    }
}

#[test]
#[cfg(feature = "lzo")]
fn f11_lzo_segments_reads_kernel_padded_segments() {
    // Segment lengths vary, and some start with a zero byte at an offset
    // that isn't a multiple of four.
    let img = load_image("f11_lzo_segments");
    let expected = load_expected("f11_lzo_segments");

    let reader: &[u8] = &img;
    let mut fs = Btrfs::open(reader, img.len() as u64).expect("open f11");

    for (path, expected_sha) in &expected.files {
        let p = Path::new(path.as_bytes()).unwrap();
        let whole = fs.read_file(p).expect(&format!("read {path}"));
        assert_eq!(sha256_hex(&whole), *expected_sha, "read_file {path}");
        assert_eq!(
            sha256_hex(&read_chunked(&mut fs, path)),
            *expected_sha,
            "read_file_at {path}"
        );
    }
}

#[derive(serde::Deserialize)]
struct SubvolExpected {
    files: BTreeMap<String, String>,
    subvols: BTreeMap<String, u64>,
    default_subvol: u64,
    empty_dirs: Vec<String>,
}

fn path(p: &str) -> Path<'_> {
    Path::new(p.as_bytes()).unwrap()
}

#[test]
fn f12_subvolumes_cross_and_open_like_a_linux_mount() {
    let img = load_image("f12_subvolumes");
    let s = std::fs::read_to_string(fixture_dir().join("f12_subvolumes.expected.json")).unwrap();
    let expected: SubvolExpected = serde_json::from_str(&s).unwrap();
    let reader: &[u8] = &img;
    let len = img.len() as u64;

    // The top level, as `subvolid=5` mounts it: every path crosses into the
    // subvolume it names. /boot/etc/hello must not alias /etc/hello.
    let mut top = Btrfs::open_subvol_id(reader, len, 5).expect("open top level");
    assert_eq!(top.subvolume(), 5);
    for (p, sha) in &expected.files {
        let whole = top.read_file(path(p)).expect(&format!("read {p}"));
        assert_eq!(sha256_hex(&whole), *sha, "read_file {p}");
        assert_eq!(
            sha256_hex(&read_chunked(&mut top, p)),
            *sha,
            "read_file_at {p}"
        );
    }
    assert_ne!(
        expected.files["/etc/hello"],
        expected.files["/boot/etc/hello"]
    );
    for (p, id) in &expected.subvols {
        let inode = top.resolve(path(p)).expect(&format!("resolve {p}"));
        assert_eq!((inode.subvolume(), inode.objectid()), (*id, 256), "{p}");
    }
    let listing = top.read_dir(path("/")).unwrap();
    let boot = listing
        .iter()
        .find(|e| e.name == b"boot")
        .expect("boot entry");
    assert_eq!(boot.inode.subvolume(), 256);
    assert_eq!(boot.inode_number, 256, "readdir reports the subvolume id");
    assert!(top.metadata(&boot.inode).unwrap().is_dir());

    // A snapshot's copy of a nested subvolume's entry is an empty directory.
    for p in &expected.empty_dirs {
        let inode = top.resolve(path(p)).unwrap();
        assert!(top.metadata(&inode).unwrap().is_dir(), "{p}");
        assert!(top.read_dir(path(p)).unwrap().is_empty(), "{p}");
        let inside = format!("{p}/log.txt");
        assert_eq!(
            top.read_file(path(&inside)),
            Err(lambutter::Error::NotFound)
        );
    }

    // The default subvolume, as a plain mount sees it, crossing into @/var.
    let mut default = Btrfs::open(reader, len).expect("open default");
    assert_eq!(default.subvolume(), expected.default_subvol);
    let os = default.read_file(path("/etc/os-release")).unwrap();
    assert_eq!(sha256_hex(&os), expected.files["/@/etc/os-release"]);
    let log = default.read_file(path("/var/log.txt")).unwrap();
    assert_eq!(sha256_hex(&log), expected.files["/@/var/log.txt"]);

    // `subvol=` and `subvolid=` land on the same root.
    let by_path = Btrfs::open_subvol_path(reader, len, path("/boot")).expect("subvol=/boot");
    assert_eq!(by_path.subvolume(), 256);
    let mut by_id = Btrfs::open_subvol_id(reader, len, 256).expect("subvolid=256");
    let vmlinuz = by_id.read_file(path("/vmlinuz-1")).unwrap();
    assert_eq!(sha256_hex(&vmlinuz), expected.files["/boot/vmlinuz-1"]);
    let mut nested = Btrfs::open_subvol_path(reader, len, path("/@/var")).unwrap();
    assert_eq!(nested.subvolume(), 258);
    let log = nested.read_file(path("/log.txt")).unwrap();
    assert_eq!(sha256_hex(&log), expected.files["/@/var/log.txt"]);
    let top_again = Btrfs::open_subvol_path(reader, len, path("/")).unwrap();
    assert_eq!(top_again.subvolume(), 5);

    // What Linux refuses, refused the same way.
    assert!(matches!(
        Btrfs::open_subvol_path(reader, len, path("/etc")),
        Err(lambutter::Error::NotASubvolume)
    ));
    assert!(matches!(
        Btrfs::open_subvol_path(reader, len, path("/@snap/var")),
        Err(lambutter::Error::NotASubvolume)
    ));
    assert!(matches!(
        Btrfs::open_subvol_path(reader, len, path("/missing")),
        Err(lambutter::Error::NotFound)
    ));
    assert!(matches!(
        Btrfs::open_subvol_id(reader, len, 9999),
        Err(lambutter::Error::NotFound)
    ));
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

/// Read every file a multi-device fixture lists from device image `dev`,
/// returning `(path, Ok(()) or the error)` and checking each read's hash.
fn read_from_device(name: &str, dev: u8) -> Vec<(String, Result<(), lambutter::Error>)> {
    let img = load_image(&format!("{name}.dev{dev}"));
    let expected = load_expected(name);
    let reader: &[u8] = &img;
    let mut fs = Btrfs::open(reader, img.len() as u64).expect(&format!("open {name} dev{dev}"));
    expected
        .files
        .iter()
        .map(|(path, sha)| {
            let p = Path::new(path.as_bytes()).expect("absolute path");
            let got = fs.read_file(p).map(|bytes| {
                assert_eq!(&sha256_hex(&bytes), sha, "{name} dev{dev}: {path} differs");
            });
            (path.clone(), got)
        })
        .collect()
}

/// A RAID1 filesystem across two devices, balanced from one, so each chunk's
/// copies sit at different offsets. Either device alone reads every file from
/// its own copy. 0.4.0 used a chunk's first stripe whatever its device, so
/// device 1 was read at device 2's offsets.
#[test]
fn f13_raid1_reads_from_either_device() {
    for dev in [1, 2] {
        for (path, got) in read_from_device("f13_raid1_two_devices", dev) {
            got.unwrap_or_else(|e| panic!("dev{dev}: {path}: {e:?}"));
        }
    }
}

/// Single data across two devices: A file is either read correctly from the
/// device holding it or refused with `prof_other_device`, never read from the
/// wrong device. 0.4.0 read the other device's offsets on this one.
#[test]
fn f14_single_data_on_the_other_device_is_refused() {
    for dev in [1, 2] {
        let (mut read, mut refused) = (0, 0);
        for (path, got) in read_from_device("f14_single_two_devices", dev) {
            match got {
                Ok(()) => read += 1,
                Err(lambutter::Error::UnsupportedProfile("prof_other_device")) => refused += 1,
                Err(e) => panic!("dev{dev}: {path}: {e:?}"),
            }
        }
        assert!(
            read > 0 && refused > 0,
            "dev{dev}: {read} read, {refused} refused"
        );
    }
}

/// fscrypt markers set by hand on an ordinary image: An extent with its
/// encryption byte set and a directory entry with `BTRFS_FT_ENCRYPTED` are
/// refused as `feat_encryption`. 0.4.0 returned the extent's bytes and listed
/// the entry.
#[test]
fn f15_fscrypt_markers_are_refused() {
    let img = load_image("f15_fscrypt_markers");
    let expected = load_expected("f15_fscrypt_markers");
    let reader: &[u8] = &img;
    let mut fs = Btrfs::open(reader, img.len() as u64).expect("open f15");
    let encrypted = Err(lambutter::Error::UnsupportedFeature("feat_encryption"));
    let p = |s: &'static str| Path::new(s.as_bytes()).expect("absolute path");
    assert_eq!(fs.read_file(p("/secret-extent.bin")).map(|_| ()), encrypted);
    assert_eq!(fs.read_file(p("/secret-name.txt")).map(|_| ()), encrypted);
    assert_eq!(fs.read_dir(p("/")).map(|_| ()), encrypted);
    for (path, sha) in &expected.files {
        let bytes = fs.read_file(Path::new(path.as_bytes()).unwrap()).unwrap();
        assert_eq!(&sha256_hex(&bytes), sha, "{path}");
    }
}
