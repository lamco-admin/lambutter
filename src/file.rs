// Copyright 2025-2026 Lamco Development LLC
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! File-content reader. Resolves an inode's EXTENT_DATA items into a byte
//! sequence, decompressing on the fly when extents are compressed.

use alloc::vec::Vec;

use crate::{
    block_read::BlockRead,
    btree::find_first_ge,
    chunk_tree::ChunkMap,
    compression,
    error::{Error, Result},
    format::{
        constants::{
            COMPRESS_NONE, EXTENT_DATA_KEY, FILE_EXTENT_INLINE, FILE_EXTENT_PREALLOC,
            FILE_EXTENT_REG, INODE_ITEM_KEY, MAX_DECOMPRESSED_EXTENT_BYTES,
        },
        repr::{DiskKey, ExtentDataHeader, ExtentDataRegular, InodeItem},
    },
    inode::Metadata,
};

/// Read an inode's metadata.
pub(crate) fn read_metadata<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_tree_root: u64,
    objectid: u64,
) -> Result<Metadata> {
    let target = DiskKey {
        objectid,
        item_type: INODE_ITEM_KEY,
        offset: 0,
    };
    let (leaf, idx) = find_first_ge(reader, chunk_map, nodesize, fs_tree_root, &target)?
        .ok_or(Error::NotFound)?;
    let item = leaf.leaf_item(idx)?;
    if item.key.objectid != objectid || item.key.item_type != INODE_ITEM_KEY {
        return Err(Error::NotFound);
    }
    let data = leaf.leaf_item_data(item)?;
    let parsed = InodeItem::parse(data, 0).ok_or(Error::CorruptBtree {
        token: "inode_item_short",
        logical: leaf.header.bytenr,
    })?;
    Ok(Metadata {
        size: parsed.size,
        nbytes: parsed.nbytes,
        nlink: parsed.nlink,
        mode: parsed.mode,
        uid: parsed.uid,
        gid: parsed.gid,
        generation: parsed.generation,
    })
}

/// Read a symlink's target. Symlink targets in btrfs live as inline data
/// in a single EXTENT_DATA item attached to the symlink's inode (per
/// BTRFS-FORMAT-READONLY-REFERENCE §8). Returns the target path bytes
/// without any modification — callers handle relative-vs-absolute and any
/// recursive resolution.
pub(crate) fn read_link<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_tree_root: u64,
    objectid: u64,
) -> Result<Vec<u8>> {
    let metadata = read_metadata(reader, chunk_map, nodesize, fs_tree_root, objectid)?;
    if !metadata.is_symlink() {
        return Err(Error::NotASymlink);
    }
    let target = DiskKey {
        objectid,
        item_type: EXTENT_DATA_KEY,
        offset: 0,
    };
    let (leaf, idx) = find_first_ge(reader, chunk_map, nodesize, fs_tree_root, &target)?
        .ok_or(Error::NotFound)?;
    let item = leaf.leaf_item(idx)?;
    if item.key.objectid != objectid || item.key.item_type != EXTENT_DATA_KEY {
        return Err(Error::CorruptBtree {
            token: "symlink_no_extent_data",
            logical: leaf.header.bytenr,
        });
    }
    let data = leaf.leaf_item_data(item)?;
    let header = ExtentDataHeader::parse(data, 0).ok_or(Error::CorruptBtree {
        token: "extent_header_short",
        logical: leaf.header.bytenr,
    })?;
    if header.ty != FILE_EXTENT_INLINE {
        // Legal-but-rare: symlink targets longer than fit inline live in
        // regular extents. Reading those would mean wiring read_file's
        // regular-extent walker through here. The /boot symlinks lambutter
        // targets are all short (vmlinuz -> vmlinuz-X.Y.Z) and inline.
        return Err(Error::UnsupportedFeature("symlink_long"));
    }
    let payload = &data[ExtentDataHeader::SIZE..];
    let mut out = Vec::new();
    if header.compression == COMPRESS_NONE {
        out.extend_from_slice(payload);
    } else {
        compression::decode(header.compression, payload, &mut out)?;
    }
    // Use the symlink inode's `size` field as the canonical length;
    // the inline-extent payload may be padded out to the leaf-item
    // size with NULs that aren't part of the link target.
    let target_len = (metadata.size as usize).min(out.len());
    out.truncate(target_len);
    Ok(out)
}

/// Read a file's full contents. Walks all EXTENT_DATA items for the inode,
/// fetches their backing extents (resolving compressed extents through the
/// compression dispatcher), and assembles the result into a byte vector.
///
/// Callers reading large files (bootloader kernel images, initrds) should
/// prefer [`read_file_at`] to avoid materialising the whole file at once.
pub(crate) fn read_file<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_tree_root: u64,
    objectid: u64,
) -> Result<Vec<u8>> {
    let metadata = read_metadata(reader, chunk_map, nodesize, fs_tree_root, objectid)?;
    if !metadata.is_file() {
        return Err(Error::NotARegularFile);
    }
    let file_size = metadata.size;

    let file_size_usize: usize = file_size.try_into().map_err(|_| Error::OutOfMemory {
        token: "file_size_overflow",
    })?;
    let mut out = Vec::new();
    out.try_reserve_exact(file_size_usize)
        .map_err(|_| Error::OutOfMemory {
            token: "file_buffer",
        })?;

    let mut next_target = DiskKey {
        objectid,
        item_type: EXTENT_DATA_KEY,
        offset: 0,
    };

    while (out.len() as u64) < file_size {
        let location = find_first_ge(reader, chunk_map, nodesize, fs_tree_root, &next_target)?;
        let Some((leaf, mut idx)) = location else {
            break;
        };
        let nritems = leaf.header.nritems;
        let mut advanced = false;
        while idx < nritems {
            let item = leaf.leaf_item(idx)?;
            if item.key.objectid != objectid || item.key.item_type != EXTENT_DATA_KEY {
                // Past this inode's extent items.
                return finalize(out, file_size);
            }
            let extent_offset_in_file = item.key.offset;
            // Out-of-order or overlap is a corruption signal.
            if extent_offset_in_file < out.len() as u64 {
                return Err(Error::CorruptBtree {
                    token: "extent_overlap",
                    logical: leaf.header.bytenr,
                });
            }
            // Hole-fill any gap, capped at the remaining file size so a
            // malformed EXTENT_DATA with key.offset >> file_size can't
            // drive an unbounded allocation. The cap also protects against
            // a 2^60-offset gap on inputs that bypassed the try_reserve at
            // the top.
            if extent_offset_in_file > file_size {
                return Err(Error::CorruptBtree {
                    token: "extent_past_eof",
                    logical: leaf.header.bytenr,
                });
            }
            let gap = extent_offset_in_file - out.len() as u64;
            if gap > 0 {
                let gap_usize: usize = gap
                    .try_into()
                    .map_err(|_| Error::OutOfMemory { token: "hole_fill" })?;
                out.try_reserve(gap_usize)
                    .map_err(|_| Error::OutOfMemory { token: "hole_fill" })?;
                out.resize(out.len() + gap_usize, 0);
            }

            let data = leaf.leaf_item_data(item)?;
            apply_extent(reader, chunk_map, &mut out, data, file_size)?;

            next_target = DiskKey {
                objectid,
                item_type: EXTENT_DATA_KEY,
                offset: extent_offset_in_file + 1,
            };
            idx += 1;
            advanced = true;
            if (out.len() as u64) >= file_size {
                break;
            }
        }
        if !advanced {
            break;
        }
    }

    finalize(out, file_size)
}

/// Read up to `buf.len()` bytes from `objectid` starting at file `offset`.
/// Returns the number of bytes written (0 = end of file). The implementation
/// only fetches and decodes extents that overlap the requested range, so
/// memory cost is bounded at the size of the single extent that contains the
/// start of the range (or the requested buffer length, whichever is smaller).
///
/// Holes and prealloc extents read as zeros; reading past EOF returns 0.
pub(crate) fn read_file_at<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    nodesize: u32,
    fs_tree_root: u64,
    objectid: u64,
    offset: u64,
    buf: &mut [u8],
) -> Result<usize> {
    if buf.is_empty() {
        return Ok(0);
    }
    let metadata = read_metadata(reader, chunk_map, nodesize, fs_tree_root, objectid)?;
    if !metadata.is_file() {
        return Err(Error::NotARegularFile);
    }
    let file_size = metadata.size;
    if offset >= file_size {
        return Ok(0);
    }
    let want = (buf.len() as u64).min(file_size - offset);
    let want_usize: usize = want.try_into().map_err(|_| Error::OutOfMemory {
        token: "read_at_overflow",
    })?;
    let buf = &mut buf[..want_usize];

    let mut written: usize = 0;
    // Seek to the extent containing `offset` by asking for the first
    // EXTENT_DATA item with key.offset <= offset. We do this by searching for
    // `(objectid, EXTENT_DATA, offset)` exact first, then falling back to the
    // greatest key <= that target by searching from key.offset = 0 forward.
    // Simpler: walk from offset=0 (cheap; typical /boot files have 1-5 extents).
    let mut next_target = DiskKey {
        objectid,
        item_type: EXTENT_DATA_KEY,
        offset: 0,
    };

    'outer: while written < buf.len() {
        let location = find_first_ge(reader, chunk_map, nodesize, fs_tree_root, &next_target)?;
        let Some((leaf, mut idx)) = location else {
            break;
        };
        let nritems = leaf.header.nritems;
        let mut advanced = false;
        while idx < nritems {
            let item = leaf.leaf_item(idx)?;
            if item.key.objectid != objectid || item.key.item_type != EXTENT_DATA_KEY {
                break 'outer;
            }
            let extent_start = item.key.offset;
            let data = leaf.leaf_item_data(item)?;
            let extent_logical_len = extent_logical_length(data)?;
            let extent_end = extent_start.saturating_add(extent_logical_len);

            let request_start = offset + written as u64;
            // Skip extents that end before our request.
            if extent_end <= request_start {
                next_target = DiskKey {
                    objectid,
                    item_type: EXTENT_DATA_KEY,
                    offset: extent_start + 1,
                };
                idx += 1;
                advanced = true;
                continue;
            }
            // Zero-fill any gap between request_start and the extent's start
            // (NO_HOLES sparse gap). Cap at the remaining buf room.
            if extent_start > request_start {
                let gap = extent_start - request_start;
                let take = (gap as usize).min(buf.len() - written);
                buf[written..written + take].fill(0);
                written += take;
                if written == buf.len() {
                    break 'outer;
                }
            }
            // Copy from this extent into buf.
            let request_start = offset + written as u64;
            let in_extent_offset = request_start - extent_start;
            let extent_remaining = extent_end - request_start;
            let want_here = (extent_remaining as usize).min(buf.len() - written);
            copy_extent_slice(
                reader,
                chunk_map,
                data,
                in_extent_offset,
                &mut buf[written..written + want_here],
            )?;
            written += want_here;

            next_target = DiskKey {
                objectid,
                item_type: EXTENT_DATA_KEY,
                offset: extent_start + 1,
            };
            idx += 1;
            advanced = true;
            if written == buf.len() {
                break 'outer;
            }
        }
        if !advanced {
            break;
        }
    }

    // Tail zero-fill: file_size > last extent end and request extends into
    // that region.
    if written < buf.len() {
        let rest = buf.len() - written;
        buf[written..written + rest].fill(0);
        written += rest;
    }
    Ok(written)
}

/// Logical length of the file region described by one EXTENT_DATA item.
fn extent_logical_length(data: &[u8]) -> Result<u64> {
    let header = ExtentDataHeader::parse(data, 0).ok_or(Error::CorruptBtree {
        token: "extent_header_short",
        logical: 0,
    })?;
    match header.ty {
        FILE_EXTENT_INLINE => Ok(header.ram_bytes),
        FILE_EXTENT_REG | FILE_EXTENT_PREALLOC => {
            let regular = ExtentDataRegular::parse(data, ExtentDataHeader::SIZE).ok_or(
                Error::CorruptBtree {
                    token: "extent_tail_short",
                    logical: 0,
                },
            )?;
            Ok(regular.num_bytes)
        }
        _ => Err(Error::CorruptBtree {
            token: "extent_type_unknown",
            logical: 0,
        }),
    }
}

/// Copy the slice [in_extent_offset, in_extent_offset + dst.len()) of one
/// extent's logical (decompressed) content into `dst`. Holes and prealloc
/// emit zeros; compressed extents are decoded into a temp buffer once per
/// call (callers iterating across extents bear that cost per extent, not
/// per byte).
fn copy_extent_slice<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    data: &[u8],
    in_extent_offset: u64,
    dst: &mut [u8],
) -> Result<()> {
    let header = ExtentDataHeader::parse(data, 0).ok_or(Error::CorruptBtree {
        token: "extent_header_short",
        logical: 0,
    })?;
    match header.ty {
        FILE_EXTENT_INLINE => {
            let payload = &data[ExtentDataHeader::SIZE..];
            let decoded;
            let src: &[u8] = if header.compression == COMPRESS_NONE {
                payload
            } else {
                let mut v = Vec::new();
                compression::decode(header.compression, payload, &mut v)?;
                decoded = v;
                &decoded[..]
            };
            let off = in_extent_offset as usize;
            let end = off.checked_add(dst.len()).ok_or(Error::CorruptBtree {
                token: "extent_overflow",
                logical: 0,
            })?;
            if end > src.len() {
                return Err(Error::CorruptBtree {
                    token: "extent_underdecoded",
                    logical: 0,
                });
            }
            dst.copy_from_slice(&src[off..end]);
            Ok(())
        }
        FILE_EXTENT_REG | FILE_EXTENT_PREALLOC => {
            let regular = ExtentDataRegular::parse(data, ExtentDataHeader::SIZE).ok_or(
                Error::CorruptBtree {
                    token: "extent_tail_short",
                    logical: 0,
                },
            )?;
            if regular.disk_bytenr == 0 || header.ty == FILE_EXTENT_PREALLOC {
                dst.fill(0);
                return Ok(());
            }
            if header.compression == COMPRESS_NONE {
                let logical_at = regular
                    .disk_bytenr
                    .checked_add(regular.offset)
                    .and_then(|v| v.checked_add(in_extent_offset))
                    .ok_or(Error::CorruptBtree {
                        token: "extent_overflow",
                        logical: regular.disk_bytenr,
                    })?;
                read_through_chunks(reader, chunk_map, logical_at, dst)
            } else {
                let disk_size: usize =
                    regular
                        .disk_num_bytes
                        .try_into()
                        .map_err(|_| Error::OutOfMemory {
                            token: "compressed_disk",
                        })?;
                if disk_size > MAX_DECOMPRESSED_EXTENT_BYTES {
                    return Err(Error::BadCompression {
                        algorithm: "comp_oversized",
                    });
                }
                let mut compressed = Vec::new();
                compressed
                    .try_reserve_exact(disk_size)
                    .map_err(|_| Error::OutOfMemory {
                        token: "compressed_disk",
                    })?;
                compressed.resize(disk_size, 0);
                read_through_chunks(reader, chunk_map, regular.disk_bytenr, &mut compressed)?;
                let mut decoded = Vec::new();
                compression::decode(header.compression, &compressed, &mut decoded)?;
                let off: usize = (regular.offset + in_extent_offset)
                    .try_into()
                    .map_err(|_| Error::OutOfMemory {
                        token: "extent_offset",
                    })?;
                let end = off.checked_add(dst.len()).ok_or(Error::CorruptBtree {
                    token: "extent_overflow",
                    logical: regular.disk_bytenr,
                })?;
                if end > decoded.len() {
                    return Err(Error::CorruptBtree {
                        token: "extent_underdecoded",
                        logical: regular.disk_bytenr,
                    });
                }
                dst.copy_from_slice(&decoded[off..end]);
                Ok(())
            }
        }
        _ => Err(Error::CorruptBtree {
            token: "extent_type_unknown",
            logical: 0,
        }),
    }
}

fn finalize(mut out: Vec<u8>, expected_size: u64) -> Result<Vec<u8>> {
    let want: usize = expected_size
        .try_into()
        .map_err(|_| Error::OutOfMemory { token: "finalize" })?;
    if out.len() < want {
        // File ends with a hole — fill with zeros up to size.
        out.resize(want, 0);
    } else if out.len() > want {
        // Last extent overran the file size — trim.
        out.truncate(want);
    }
    Ok(out)
}

fn apply_extent<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    out: &mut Vec<u8>,
    data: &[u8],
    file_size: u64,
) -> Result<()> {
    let header = ExtentDataHeader::parse(data, 0).ok_or(Error::CorruptBtree {
        token: "extent_header_short",
        logical: 0,
    })?;

    match header.ty {
        FILE_EXTENT_INLINE => {
            // Inline data follows the 21-byte header. ram_bytes is the
            // logical size; the on-disk tail may be compressed and is
            // shorter.
            let payload = &data[ExtentDataHeader::SIZE..];
            if header.compression == COMPRESS_NONE {
                let take = (header.ram_bytes as usize).min(payload.len());
                out.extend_from_slice(&payload[..take]);
            } else {
                let mut decoded = Vec::new();
                compression::decode(header.compression, payload, &mut decoded)?;
                let take = (header.ram_bytes as usize).min(decoded.len());
                out.extend_from_slice(&decoded[..take]);
            }
        }
        FILE_EXTENT_REG | FILE_EXTENT_PREALLOC => {
            let regular = ExtentDataRegular::parse(data, ExtentDataHeader::SIZE).ok_or(
                Error::CorruptBtree {
                    token: "extent_tail_short",
                    logical: 0,
                },
            )?;

            let logical_bytes_to_emit = regular.num_bytes;
            // Cap to remaining file size.
            let remaining = file_size.saturating_sub(out.len() as u64);
            let emit = logical_bytes_to_emit.min(remaining);
            if emit == 0 {
                return Ok(());
            }
            let emit_usize: usize = emit.try_into().map_err(|_| Error::OutOfMemory {
                token: "extent_emit",
            })?;

            // Hole or prealloc: zeros. `read_file` already try_reserved
            // file_size up front so resize is allocation-free here.
            if regular.disk_bytenr == 0 || header.ty == FILE_EXTENT_PREALLOC {
                out.resize(out.len() + emit_usize, 0);
                return Ok(());
            }

            // Read the extent's bytes through the chunk map. Compressed
            // extents read disk_num_bytes and decompress to ram_bytes;
            // uncompressed extents read num_bytes from logical
            // (disk_bytenr + offset).
            if header.compression == COMPRESS_NONE {
                let read_start = out.len();
                out.resize(read_start + emit_usize, 0);
                let disk_at =
                    regular
                        .disk_bytenr
                        .checked_add(regular.offset)
                        .ok_or(Error::CorruptBtree {
                            token: "extent_overflow",
                            logical: regular.disk_bytenr,
                        })?;
                read_through_chunks(reader, chunk_map, disk_at, &mut out[read_start..])?;
            } else {
                // Cap disk_num_bytes at the plaintext decompression cap.
                // A compressed extent's on-disk size cannot legitimately
                // exceed its plaintext size, so 16 MiB is an upper bound;
                // this protects against malicious extents that claim huge
                // disk_num_bytes to force a large `vec![0u8; n]` allocation
                // before we ever attempt to read or decompress.
                let disk_size: usize =
                    regular
                        .disk_num_bytes
                        .try_into()
                        .map_err(|_| Error::OutOfMemory {
                            token: "compressed_disk",
                        })?;
                if disk_size > MAX_DECOMPRESSED_EXTENT_BYTES {
                    return Err(Error::BadCompression {
                        algorithm: "comp_oversized",
                    });
                }
                let mut compressed = Vec::new();
                compressed
                    .try_reserve_exact(disk_size)
                    .map_err(|_| Error::OutOfMemory {
                        token: "compressed_disk",
                    })?;
                compressed.resize(disk_size, 0);
                read_through_chunks(reader, chunk_map, regular.disk_bytenr, &mut compressed)?;
                let mut decoded = Vec::new();
                compression::decode(header.compression, &compressed, &mut decoded)?;
                // Slice [offset..offset+emit] from decoded; offset is into
                // the decompressed (logical) extent.
                let off: usize = regular.offset.try_into().map_err(|_| Error::OutOfMemory {
                    token: "extent_offset",
                })?;
                let end = off.checked_add(emit_usize).ok_or(Error::CorruptBtree {
                    token: "extent_overflow",
                    logical: regular.disk_bytenr,
                })?;
                if end > decoded.len() {
                    return Err(Error::CorruptBtree {
                        token: "extent_underdecoded",
                        logical: regular.disk_bytenr,
                    });
                }
                out.extend_from_slice(&decoded[off..end]);
            }
        }
        _ => {
            return Err(Error::CorruptBtree {
                token: "extent_type_unknown",
                logical: 0,
            });
        }
    }
    Ok(())
}

/// Read a logical byte range through the chunk map, possibly spanning
/// multiple chunks.
fn read_through_chunks<R: BlockRead>(
    reader: &mut R,
    chunk_map: &ChunkMap,
    logical_start: u64,
    buf: &mut [u8],
) -> Result<()> {
    let mut written: usize = 0;
    let mut cursor = logical_start;
    while written < buf.len() {
        let resolved = chunk_map.resolve(cursor)?;
        let take = (buf.len() - written).min(resolved.contiguous_bytes as usize);
        reader
            .read_at(resolved.physical, &mut buf[written..written + take])
            .map_err(|_| Error::Io {
                token: "extent_read",
                offset: resolved.physical,
            })?;
        written += take;
        cursor += take as u64;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::format::constants::MAX_DECOMPRESSED_EXTENT_BYTES;

    /// Lock the 16 MiB plaintext cap so a refactor that changes
    /// `MAX_DECOMPRESSED_EXTENT_BYTES` surfaces the new value through this
    /// test and forces the spec / SDS to be updated in lockstep.
    #[test]
    fn plaintext_decompression_cap_is_16_mib() {
        const EXPECTED: usize = 16 * 1024 * 1024;
        assert_eq!(MAX_DECOMPRESSED_EXTENT_BYTES, EXPECTED);
    }

    /// Inline-extent length comes from the header's `ram_bytes`.
    #[test]
    fn extent_logical_length_inline() {
        let mut buf = [0u8; 21];
        buf[8..16].copy_from_slice(&1234u64.to_le_bytes());
        buf[20] = FILE_EXTENT_INLINE;
        assert_eq!(extent_logical_length(&buf).unwrap(), 1234);
    }

    /// Regular-extent length comes from the tail's `num_bytes`.
    #[test]
    fn extent_logical_length_regular() {
        let mut buf = [0u8; 21 + 32];
        buf[20] = FILE_EXTENT_REG;
        buf[21 + 24..21 + 32].copy_from_slice(&5555u64.to_le_bytes());
        assert_eq!(extent_logical_length(&buf).unwrap(), 5555);
    }

    #[test]
    fn extent_logical_length_unknown_type_errors() {
        let mut buf = [0u8; 21];
        buf[20] = 99;
        assert!(matches!(
            extent_logical_length(&buf),
            Err(Error::CorruptBtree {
                token: "extent_type_unknown",
                ..
            })
        ));
    }
}
