// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! Live inspection of a btrfs volume on a block device or image file.
//!
//! Usage:
//!     cargo build --release --example inspect
//!     sudo target/release/examples/inspect <device-or-image> [path-inside-fs]
//!
//! With no path, prints volume header info (size + resolved default subvol
//! objectid). With a path, additionally reads the file and prints its size,
//! sha256, and first 16 bytes.
//!
//! This example uses `std::os::unix::fs::FileExt::read_exact_at` for
//! positional reads. It is host-only and is not part of the no_std API surface.

use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom},
    os::unix::fs::FileExt,
    process::ExitCode,
};

use lambutter::{BlockRead, Btrfs, Path};
use sha2::{Digest, Sha256};

struct FileBlock {
    f: std::fs::File,
}

#[derive(Debug)]
struct IoErr(#[allow(dead_code)] std::io::Error);

impl BlockRead for FileBlock {
    type Error = IoErr;

    fn read_at(&mut self, offset_bytes: u64, buf: &mut [u8]) -> Result<(), Self::Error> {
        self.f.read_exact_at(buf, offset_bytes).map_err(IoErr)
    }
}

fn sha256_hex(data: &[u8]) -> String {
    let out = Sha256::digest(data);
    let mut s = String::with_capacity(64);
    for b in out {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn hex_head(data: &[u8], n: usize) -> String {
    data.iter().take(n).map(|b| format!("{b:02x}")).collect()
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 || args.len() > 3 {
        return Err(format!(
            "usage: {} <device-or-image> [path-inside-fs]",
            args.first().map(String::as_str).unwrap_or("inspect")
        ));
    }
    let device = &args[1];
    let target_path = args.get(2);

    let mut f = OpenOptions::new()
        .read(true)
        .open(device)
        .map_err(|e| format!("open {device}: {e}"))?;
    let size = f
        .seek(SeekFrom::End(0))
        .map_err(|e| format!("seek-end {device}: {e}"))?;
    f.seek(SeekFrom::Start(0))
        .map_err(|e| format!("seek-start {device}: {e}"))?;

    println!("device:                  {device}");
    println!(
        "size:                    {size} bytes ({:.2} GiB)",
        size as f64 / (1024.0 * 1024.0 * 1024.0)
    );

    let reader = FileBlock { f };
    let mut fs = Btrfs::open(reader, size).map_err(|e| format!("Btrfs::open: {e:?}"))?;

    println!("default subvol objectid: {}", fs.default_subvol_objectid());

    if let Some(p) = target_path {
        let bp = Path::new(p.as_bytes())
            .ok_or_else(|| format!("Path::new({p}): rejected (must be absolute)"))?;
        let inode = fs.resolve(bp).map_err(|e| format!("resolve({p}): {e:?}"))?;
        let md = fs
            .metadata(&inode)
            .map_err(|e| format!("metadata({p}): {e:?}"))?;
        println!();
        println!("path:                    {p}");
        println!("inode objectid:          {}", inode.objectid);
        println!(
            "mode:                    0o{:o} ({})",
            md.mode,
            if md.is_file() {
                "regular file"
            } else if md.is_dir() {
                "directory"
            } else if md.is_symlink() {
                "symlink"
            } else {
                "other"
            }
        );
        println!("metadata size:           {} bytes", md.size);
        println!("nbytes on disk:          {}", md.nbytes);
        println!(
            "nlink / uid / gid:       {} / {} / {}",
            md.nlink, md.uid, md.gid
        );

        if md.is_symlink() {
            let target = fs
                .read_link(bp)
                .map_err(|e| format!("read_link({p}): {e:?}"))?;
            let as_str = std::str::from_utf8(&target).ok();
            println!(
                "symlink target:          {}",
                as_str.unwrap_or("(non-UTF8)")
            );
            println!("symlink target bytes:    {}", target.len());
        } else if md.is_dir() {
            let entries = fs
                .read_dir(bp)
                .map_err(|e| format!("read_dir({p}): {e:?}"))?;
            println!("directory entries:       {}", entries.len());
            for e in entries.iter().take(20) {
                let name = std::str::from_utf8(&e.name).unwrap_or("(non-UTF8)");
                println!(
                    "  inode={:8}  kind=0x{:02x}  {}",
                    e.inode_number, e.kind_byte, name
                );
            }
            if entries.len() > 20 {
                println!("  ... ({} more)", entries.len() - 20);
            }
        } else if md.is_file() {
            let bytes = fs
                .read_file(bp)
                .map_err(|e| format!("read_file({p}): {e:?}"))?;
            println!("content size:            {} bytes", bytes.len());
            println!("sha256:                  {}", sha256_hex(&bytes));
            let head_n = bytes.len().min(16);
            println!(
                "first {head_n} bytes:           {}",
                hex_head(&bytes, head_n)
            );
        }
    }

    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
