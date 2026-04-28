#!/usr/bin/env bash
# Generate the test-fixture btrfs images. Each fixture targets a specific
# scenario from docs/TESTING-AND-FUZZING-PLAN.md §5.1. Fixtures are
# committed to the repo as zstd-compressed images plus a JSON file listing
# the canonical (path, sha256) tuples the test harness asserts against.
#
# Requires: btrfs-progs, sudo (for loop mount), zstd, sha256sum.
# This script SHOULD be re-runnable. It produces deterministic output via:
#   - fixed file contents
#   - btrfs-progs `--nodesize` and `--sectorsize` pinned values
#   - mtime-clobbered with `find ... -exec touch -d "@0" {} +` before mkfs
#
# Run all fixtures:    ./build.sh all
# Run one fixture:     ./build.sh f1
# Recompress only:     ./build.sh recompress

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
DATA="$HERE/../data"
WORK="$(mktemp -d -t lambutter-fixtures-XXXXXX)"
trap 'sudo umount "$WORK/mnt" 2>/dev/null || true; rm -rf "$WORK"' EXIT

ensure_data_dir() {
    mkdir -p "$DATA"
}

mkfs_with_rootdir() {
    # Create a btrfs image with a populated rootdir, no mount required.
    # mkfs.btrfs --rootdir is fully unprivileged; this is the path we
    # prefer when no kernel-side compression is needed.
    local img="$1"
    local content_dir="$2"
    local extra_flags="${3:-}"
    truncate -s 128M "$img"
    mkfs.btrfs -q -f \
        --rootdir "$content_dir" \
        --nodesize 16384 \
        --sectorsize 4096 \
        ${extra_flags} \
        "$img"
}

mkfs_with_mount() {
    # When we need kernel-side compression (zstd / zlib / lzo), mkfs.btrfs
    # --rootdir bypasses compression entirely. We have to mount and copy.
    local img="$1"
    local content_dir="$2"
    local mount_opts="$3"
    truncate -s 128M "$img"
    mkfs.btrfs -q -f --nodesize 16384 --sectorsize 4096 "$img"
    mkdir -p "$WORK/mnt"
    sudo mount -o "loop,$mount_opts" "$img" "$WORK/mnt"
    sudo cp -a "$content_dir"/. "$WORK/mnt"/
    sudo sync
    sudo umount "$WORK/mnt"
}

emit_expected_json() {
    local fixture="$1"
    shift
    local out="$DATA/${fixture}.expected.json"
    {
        echo "{"
        echo "  \"fixture\": \"${fixture}\","
        echo "  \"files\": {"
        local first=1
        while [[ $# -gt 0 ]]; do
            local rel="$1" sha="$2"
            shift 2
            if (( first )); then first=0; else echo ","; fi
            printf '    "%s": "%s"' "$rel" "$sha"
        done
        echo
        echo "  }"
        echo "}"
    } > "$out"
}

compress_fixture() {
    local img="$1"
    local out="$2"
    zstd -q -19 -f "$img" -o "$out"
    sha256sum "$out" | awk '{print $1}' > "${out}.sha256"
}

# --------------------------------------------------------------------------
# Fixtures
# --------------------------------------------------------------------------

build_f1_single_uncompressed() {
    echo "=== F1 single_uncompressed ==="
    local content="$WORK/f1-content"
    mkdir -p "$content"
    echo -n "hello btrfs from F1" > "$content/hello.txt"
    mkdir -p "$content/dir-a"
    echo -n "nested file content" > "$content/dir-a/nested.txt"

    local img="$WORK/f1.img"
    mkfs_with_rootdir "$img" "$content"

    local sha_hello sha_nested
    sha_hello=$(sha256sum < "$content/hello.txt" | awk '{print $1}')
    sha_nested=$(sha256sum < "$content/dir-a/nested.txt" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f1_single_uncompressed.img.zst"
    emit_expected_json "f1_single_uncompressed" \
        "/hello.txt" "$sha_hello" \
        "/dir-a/nested.txt" "$sha_nested"
}

build_f2_single_zstd() {
    echo "=== F2 single_zstd ==="
    local content="$WORK/f2-content"
    mkdir -p "$content"
    # Larger, repetitive content to ensure zstd actually compresses.
    # `yes | head` would SIGPIPE-fail under `set -o pipefail`; do it
    # directly with `printf` + a counted loop.
    {
        for _ in $(seq 1 27000); do
            printf 'compressible-line-with-some-tail-data\n'
        done
    } | head -c 1048576 > "$content/big.bin" || true
    # Pad to exact 1 MiB if the loop produced slightly less.
    truncate -s 1M "$content/big.bin"
    echo -n "small inline content" > "$content/small.txt"

    local img="$WORK/f2.img"
    mkfs_with_mount "$img" "$content" "compress-force=zstd:3"

    local sha_big sha_small
    sha_big=$(sha256sum < "$content/big.bin" | awk '{print $1}')
    sha_small=$(sha256sum < "$content/small.txt" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f2_single_zstd.img.zst"
    emit_expected_json "f2_single_zstd" \
        "/big.bin" "$sha_big" \
        "/small.txt" "$sha_small"
}

build_f3_single_zlib() {
    echo "=== F3 single_zlib ==="
    local content="$WORK/f3-content"
    mkdir -p "$content"
    {
        for _ in $(seq 1 27000); do
            printf 'zlib-content-line-with-tail\n'
        done
    } | head -c 1048576 > "$content/big-zlib.bin" || true
    truncate -s 1M "$content/big-zlib.bin"
    echo -n "small inline zlib content" > "$content/small-zlib.txt"

    local img="$WORK/f3.img"
    mkfs_with_mount "$img" "$content" "compress-force=zlib:6"

    local sha_big sha_small
    sha_big=$(sha256sum < "$content/big-zlib.bin" | awk '{print $1}')
    sha_small=$(sha256sum < "$content/small-zlib.txt" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f3_single_zlib.img.zst"
    emit_expected_json "f3_single_zlib" \
        "/big-zlib.bin" "$sha_big" \
        "/small-zlib.txt" "$sha_small"
}

build_f4_single_lzo() {
    echo "=== F4 single_lzo ==="
    local content="$WORK/f4-content"
    mkdir -p "$content"
    {
        for _ in $(seq 1 27000); do
            printf 'lzo-content-line-with-tail\n'
        done
    } | head -c 1048576 > "$content/big-lzo.bin" || true
    truncate -s 1M "$content/big-lzo.bin"
    echo -n "small inline lzo content" > "$content/small-lzo.txt"

    local img="$WORK/f4.img"
    mkfs_with_mount "$img" "$content" "compress-force=lzo"

    local sha_big sha_small
    sha_big=$(sha256sum < "$content/big-lzo.bin" | awk '{print $1}')
    sha_small=$(sha256sum < "$content/small-lzo.txt" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f4_single_lzo.img.zst"
    emit_expected_json "f4_single_lzo" \
        "/big-lzo.bin" "$sha_big" \
        "/small-lzo.txt" "$sha_small"
}

build_f5_dup_metadata() {
    echo "=== F5 dup_metadata ==="
    local content="$WORK/f5-content"
    mkdir -p "$content"
    echo -n "DUP metadata test file" > "$content/dup-test.txt"
    mkdir -p "$content/sub"
    echo -n "nested under DUP metadata" > "$content/sub/nested.txt"

    local img="$WORK/f5.img"
    # mkfs.btrfs default for ssd-detected media is `--metadata dup --data single`;
    # force it explicitly so the fixture is reproducible regardless of host
    # storage detection.
    truncate -s 128M "$img"
    mkfs.btrfs -q -f \
        --rootdir "$content" \
        --nodesize 16384 \
        --sectorsize 4096 \
        --data single \
        --metadata dup \
        "$img"

    local sha_top sha_nested
    sha_top=$(sha256sum < "$content/dup-test.txt" | awk '{print $1}')
    sha_nested=$(sha256sum < "$content/sub/nested.txt" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f5_dup_metadata.img.zst"
    emit_expected_json "f5_dup_metadata" \
        "/dup-test.txt" "$sha_top" \
        "/sub/nested.txt" "$sha_nested"
}

build_f8_sparse_no_holes() {
    echo "=== F8 sparse_no_holes ==="
    local content="$WORK/f8-content"
    mkdir -p "$content"
    # Build a sparse file: 4 KiB of "AAAA", then a 1 MiB hole, then 4 KiB of "BBBB".
    # Btrfs with NO_HOLES (the default since ~2018) records this as two extents
    # with a gap; lambutter's read path must zero-fill the gap.
    {
        printf 'A%.0s' $(seq 1 4096)
        truncate -s 1052672 /dev/stdout 2>/dev/null || dd if=/dev/zero bs=1 count=$((1048576)) 2>/dev/null
    } > "$content/sparse.bin"
    # Replace the dd above (which appends 0xff vs 0x00 inconsistently across
    # shells) with an explicit construction:
    rm -f "$content/sparse.bin"
    printf 'A%.0s' $(seq 1 4096) > "$content/sparse.bin"
    truncate -s 1052672 "$content/sparse.bin"      # extends with zeros
    printf 'B%.0s' $(seq 1 4096) >> "$content/sparse.bin"

    local img="$WORK/f8.img"
    mkfs_with_rootdir "$img" "$content"

    local sha_sparse
    sha_sparse=$(sha256sum < "$content/sparse.bin" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f8_sparse_no_holes.img.zst"
    emit_expected_json "f8_sparse_no_holes" \
        "/sparse.bin" "$sha_sparse"
}

build_f9_symlink_chain() {
    echo "=== F9 symlink_chain ==="
    local content="$WORK/f9-content"
    mkdir -p "$content"
    echo -n "this is the real target" > "$content/target.txt"
    ln -s "target.txt" "$content/link-relative"
    ln -s "/target.txt" "$content/link-absolute"

    local img="$WORK/f9.img"
    mkfs_with_rootdir "$img" "$content"

    local sha_target
    sha_target=$(sha256sum < "$content/target.txt" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f9_symlink_chain.img.zst"

    # Special expected.json: also asserts symlink targets
    cat > "$DATA/f9_symlink_chain.expected.json" <<EOF
{
  "fixture": "f9_symlink_chain",
  "files": {
    "/target.txt": "$sha_target"
  },
  "symlinks": {
    "/link-relative": "target.txt",
    "/link-absolute": "/target.txt"
  }
}
EOF
}

# --------------------------------------------------------------------------
# Driver
# --------------------------------------------------------------------------

ensure_data_dir

case "${1:-all}" in
    f1) build_f1_single_uncompressed ;;
    f2) build_f2_single_zstd ;;
    f3) build_f3_single_zlib ;;
    f4) build_f4_single_lzo ;;
    f5) build_f5_dup_metadata ;;
    f8) build_f8_sparse_no_holes ;;
    f9) build_f9_symlink_chain ;;
    all)
        build_f1_single_uncompressed
        build_f2_single_zstd
        build_f3_single_zlib
        build_f4_single_lzo
        build_f5_dup_metadata
        build_f8_sparse_no_holes
        build_f9_symlink_chain
        ;;
    *) echo "Usage: $0 [all|f1|f2|f3|f5|f8|f9]" >&2; exit 2 ;;
esac

echo "Done. Fixtures in $DATA/"
ls -la "$DATA"
