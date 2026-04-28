// Copyright 2025-2026 Lamco Development
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// https://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or https://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

//! File-content reader. Resolves an inode's EXTENT_DATA items into a byte
//! sequence, decompressing on the fly when extents are compressed.

use alloc::{vec, vec::Vec};

use crate::{
    block_read::BlockRead,
    btree::{find_first_ge, read_tree_block},
    chunk_tree::ChunkMap,
    compression,
    error::{Error, Result},
    format::{
        constants::{
            COMPRESS_NONE, EXTENT_DATA_KEY, FILE_EXTENT_INLINE, FILE_EXTENT_PREALLOC,
            FILE_EXTENT_REG, INODE_ITEM_KEY,
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
        // Symlinks longer than fits inline are extremely rare but legal;
        // surface a typed error rather than silently truncate.
        return Err(Error::CorruptBtree {
            token: "symlink_not_inline",
            logical: leaf.header.bytenr,
        });
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

    let mut out = Vec::new();
    out.try_reserve_exact(file_size as usize)
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
            // Hole-fill any gap between current `out.len()` and this
            // extent's starting offset.
            let gap = extent_offset_in_file - out.len() as u64;
            if gap > 0 {
                let gap_usize: usize = gap
                    .try_into()
                    .map_err(|_| Error::OutOfMemory { token: "hole_fill" })?;
                let pad = vec![0u8; gap_usize];
                out.extend_from_slice(&pad);
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

            // Hole or prealloc: zeros.
            if regular.disk_bytenr == 0 || header.ty == FILE_EXTENT_PREALLOC {
                let pad = vec![0u8; emit_usize];
                out.extend_from_slice(&pad);
                return Ok(());
            }

            // Read the extent's bytes through the chunk map. Compressed
            // extents read disk_num_bytes and decompress to ram_bytes;
            // uncompressed extents read num_bytes from logical
            // (disk_bytenr + offset).
            if header.compression == COMPRESS_NONE {
                let mut buf = vec![0u8; emit_usize];
                read_through_chunks(
                    reader,
                    chunk_map,
                    regular.disk_bytenr + regular.offset,
                    &mut buf,
                )?;
                out.extend_from_slice(&buf);
            } else {
                let disk_size: usize =
                    regular
                        .disk_num_bytes
                        .try_into()
                        .map_err(|_| Error::OutOfMemory {
                            token: "compressed_disk",
                        })?;
                let mut compressed = vec![0u8; disk_size];
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
    let _ = read_tree_block::<R>; // silence unused-import warning when only this fn is used
    Ok(())
}
