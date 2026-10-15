#!/usr/bin/env bash
# Generate the test-fixture btrfs images. Each fixture targets a specific
# scenario from docs/TESTING-AND-FUZZING-PLAN.md §5.1. Fixtures are
# committed to the repo as zstd-compressed images plus a JSON file listing
# the canonical (path, sha256) tuples the test harness asserts against.
#
# Requires: btrfs-progs (including btrfs-map-logical), sudo (for loop mounts
# and loop devices), zstd, sha256sum, python3.
# This script SHOULD be re-runnable. It produces deterministic output via:
#   - fixed file contents
#   - btrfs-progs `--nodesize` and `--sectorsize` pinned values
#   - mtime-clobbered with `find ... -exec touch -d "@0" {} +` before mkfs
#
# Run all fixtures:    ./build.sh all
# Run one fixture:     ./build.sh f1
# Recompress only:     ./build.sh recompress

set -euo pipefail
export PATH="/usr/sbin:/sbin:$PATH"

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

build_f10_multi_leaf() {
    echo "=== F10 multi_leaf ==="
    # 4 KiB nodes hold about fifty extent items, so the 8 MiB file's 64
    # compressed extents and the entries directory's index both span leaves.
    # Deleting every other entry leaves gaps in the index keys, which is
    # what a search that stops at a leaf end trips over.
    local content="$WORK/f10-content"
    mkdir -p "$content/entries"
    seq 1 1100000 | awk '{ printf "line %08d of the multi-leaf fixture\n", $1 }' \
        | head -c 8388608 > "$content/big.bin" || true
    truncate -s 8M "$content/big.bin"
    for i in $(seq 1 300); do
        printf 'title Entry %d\nversion 6.%d.0\nlinux /vmlinuz-6.%d.0\n' "$i" "$i" "$i" \
            > "$content/entries/e$(printf %03d "$i").conf"
    done

    local img="$WORK/f10.img"
    truncate -s 128M "$img"
    mkfs.btrfs -q -f --nodesize 4096 --sectorsize 4096 "$img"
    mkdir -p "$WORK/mnt"
    sudo mount -o "loop,compress-force=zstd:3" "$img" "$WORK/mnt"
    sudo cp -a "$content"/. "$WORK/mnt"/
    sudo sync
    for i in $(seq 2 2 300); do
        sudo rm "$WORK/mnt/entries/e$(printf %03d "$i").conf"
        rm "$content/entries/e$(printf %03d "$i").conf"
    done
    sudo sync
    sudo umount "$WORK/mnt"

    local sha_big sha_first sha_last
    sha_big=$(sha256sum < "$content/big.bin" | awk '{print $1}')
    sha_first=$(sha256sum < "$content/entries/e001.conf" | awk '{print $1}')
    sha_last=$(sha256sum < "$content/entries/e299.conf" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f10_multi_leaf.img.zst"
    cat > "$DATA/f10_multi_leaf.expected.json" <<EOF
{
  "fixture": "f10_multi_leaf",
  "files": {
    "/big.bin": "$sha_big",
    "/entries/e001.conf": "$sha_first",
    "/entries/e299.conf": "$sha_last"
  },
  "dirs": {
    "/entries": 150
  }
}
EOF
}

build_f11_lzo_segments() {
    echo "=== F11 lzo_segments ==="
    # Text from a small vocabulary compresses unevenly, so segment lengths
    # vary sector to sector the way real files do. Some segment lengths
    # start with a zero byte at an offset that isn't a multiple of four,
    # which is where a decoder that treats every unaligned zero as padding
    # goes wrong.
    local content="$WORK/f11-content"
    mkdir -p "$content"
    awk 'BEGIN { srand(42); split("boot loader kernel initrd entry version linux options title machine", w);
                 for (i = 0; i < 700000; i++) printf "%s%s", w[int(rand() * 10) + 1], (i % 9 == 8 ? "\n" : " ") }' \
        | head -c 4194304 > "$content/words.txt" || true

    local img="$WORK/f11.img"
    mkfs_with_mount "$img" "$content" "compress-force=lzo"

    local sha_words
    sha_words=$(sha256sum < "$content/words.txt" | awk '{print $1}')

    compress_fixture "$img" "$DATA/f11_lzo_segments.img.zst"
    emit_expected_json "f11_lzo_segments" \
        "/words.txt" "$sha_words"
}

build_f12_subvolumes() {
    echo "=== F12 subvolumes ==="
    # Subvolume ids follow creation order: boot 256, @ 257, @/var 258,
    # @snap 259. boot/etc/hello differs from the top level's etc/hello, so a
    # reader that resolves inside the wrong tree reads the wrong file. @snap
    # is a snapshot of @, so its var entry names subvolume 258 without a
    # root-tree link, and Linux shows it as an empty directory.
    local content="$WORK/f12-content"
    mkdir -p "$content"
    seq 1 20000 | awk '{ printf "kernel image line %06d\n", $1 }' > "$content/vmlinuz-1"
    printf 'top-level hello\n' > "$content/top-hello"
    printf 'boot subvolume hello\n' > "$content/boot-hello"
    printf 'title Entry A\nlinux /vmlinuz-1\n' > "$content/a.conf"
    printf 'NAME=fixture\n' > "$content/os-release"
    printf 'nested subvolume log\n' > "$content/log.txt"

    local img="$WORK/f12.img" m="$WORK/mnt"
    truncate -s 128M "$img"
    mkfs.btrfs -q -f --nodesize 16384 --sectorsize 4096 "$img"
    mkdir -p "$m"
    sudo mount -o loop "$img" "$m"
    sudo mkdir "$m/etc"
    sudo cp "$content/top-hello" "$m/etc/hello"
    sudo btrfs -q subvolume create "$m/boot"
    sudo mkdir -p "$m/boot/loader/entries" "$m/boot/etc"
    sudo cp "$content/vmlinuz-1" "$m/boot/vmlinuz-1"
    sudo cp "$content/a.conf" "$m/boot/loader/entries/a.conf"
    sudo cp "$content/boot-hello" "$m/boot/etc/hello"
    sudo btrfs -q subvolume create "$m/@"
    sudo mkdir "$m/@/etc"
    sudo cp "$content/os-release" "$m/@/etc/os-release"
    sudo btrfs -q subvolume create "$m/@/var"
    sudo cp "$content/log.txt" "$m/@/var/log.txt"
    sudo btrfs -q subvolume snapshot "$m/@" "$m/@snap"
    sudo btrfs -q subvolume set-default 257 "$m"
    sudo sync
    # The layout the test expects, as Linux reports it.
    [ "$(sudo btrfs inspect-internal rootid "$m/boot")" = 256 ]
    [ "$(sudo btrfs inspect-internal rootid "$m/@")" = 257 ]
    [ "$(sudo btrfs inspect-internal rootid "$m/@/var")" = 258 ]
    [ "$(sudo btrfs inspect-internal rootid "$m/@snap")" = 259 ]
    [ -d "$m/@snap/var" ] && [ -z "$(sudo ls -A "$m/@snap/var")" ]
    sudo umount "$m"
    sudo mount -o loop "$img" "$m"
    [ "$(sudo cat "$m/etc/os-release")" = "NAME=fixture" ]
    sudo umount "$m"

    sha() { sha256sum < "$1" | awk '{print $1}'; }
    compress_fixture "$img" "$DATA/f12_subvolumes.img.zst"
    cat > "$DATA/f12_subvolumes.expected.json" <<EOF
{
  "fixture": "f12_subvolumes",
  "files": {
    "/etc/hello": "$(sha "$content/top-hello")",
    "/boot/vmlinuz-1": "$(sha "$content/vmlinuz-1")",
    "/boot/etc/hello": "$(sha "$content/boot-hello")",
    "/boot/loader/entries/a.conf": "$(sha "$content/a.conf")",
    "/@/etc/os-release": "$(sha "$content/os-release")",
    "/@/var/log.txt": "$(sha "$content/log.txt")"
  },
  "subvols": {
    "/": 5,
    "/boot": 256,
    "/@": 257,
    "/@/var": 258,
    "/@snap": 259
  },
  "default_subvol": 257,
  "empty_dirs": ["/@snap/var"]
}
EOF
}

# Loop devices this run attached, detached on exit.
LOOPS=()
detach_loops() {
    local l
    for l in "${LOOPS[@]}"; do sudo losetup -d "$l" 2>/dev/null || true; done
    LOOPS=()
}
trap 'sudo umount "$WORK/mnt" 2>/dev/null || true; detach_loops; rm -rf "$WORK"' EXIT

# attach_loop IMG VAR: attach IMG and store its loop device in VAR. Not a
# command substitution, which would run in a subshell and lose LOOPS.
attach_loop() {
    local dev
    dev="$(sudo losetup --find --show "$1")"
    LOOPS+=("$dev")
    printf -v "$2" '%s' "$dev"
}

# chunk_devids IMG: one line per chunk, "TYPE devid:offset ...", from the
# chunk tree, to check where the fixture's chunks landed.
chunk_devids() {
    btrfs inspect-internal dump-tree -t chunk "$1" 2>/dev/null | awk '
        /CHUNK_ITEM/ { if (line != "") print line; line = "" }
        /type DATA|type METADATA|type SYSTEM/ { line = $0; sub(/.*type /, "", line); sub(/ .*/, "", line) }
        /stripe [0-9]+ devid/ { line = line " " $4 ":" $6 }
        END { if (line != "") print line }'
}

build_f13_raid1_two_devices() {
    echo "=== F13 RAID1 across two devices ==="
    # Made on one device, then given a second and balanced to RAID1, so each
    # chunk's two copies sit at different offsets on the two devices: Reading
    # a device at the other device's offset gives the wrong bytes.
    local content="$WORK/f13-content"
    mkdir -p "$content"
    seq 1 30000 | awk '{ printf "raid1 kernel line %06d\n", $1 }' > "$content/vmlinuz"
    printf 'title RAID1\nlinux /vmlinuz\n' > "$content/entry.conf"
    local img0="$WORK/f13.dev1.img" img1="$WORK/f13.dev2.img" m="$WORK/mnt" l0 l1
    truncate -s 256M "$img0" "$img1"
    mkfs.btrfs -q -f --nodesize 16384 --sectorsize 4096 -d single -m single "$img0"
    attach_loop "$img0" l0
    attach_loop "$img1" l1
    mkdir -p "$m"
    sudo mount "$l0" "$m"
    sudo cp "$content/vmlinuz" "$content/entry.conf" "$m/"
    sudo sync
    sudo btrfs -q device add -f "$l1" "$m"
    sudo btrfs -q balance start -f -dconvert=raid1 -mconvert=raid1 -sconvert=raid1 "$m" >/dev/null
    sudo sync
    sudo umount "$m"
    detach_loops
    chunk_devids "$img0"
    # Every chunk must be RAID1 with copies at different offsets.
    chunk_devids "$img0" | awk '{ if ($1 !~ /RAID1/) bad = 1; split($2, a, ":"); split($3, b, ":"); if (a[2] == b[2]) same = 1 }
        END { exit (bad || same) }' || { echo "f13: chunks not RAID1 at distinct offsets" >&2; exit 1; }
    sha() { sha256sum < "$1" | awk '{print $1}'; }
    compress_fixture "$img0" "$DATA/f13_raid1_two_devices.dev1.img.zst"
    compress_fixture "$img1" "$DATA/f13_raid1_two_devices.dev2.img.zst"
    emit_expected_json f13_raid1_two_devices \
        "/vmlinuz" "$(sha "$content/vmlinuz")" \
        "/entry.conf" "$(sha "$content/entry.conf")"
}

build_f14_single_two_devices() {
    echo "=== F14 single data across two devices ==="
    # RAID1 metadata, single data: Each data chunk lives on one device, so a
    # file on the other device can't be read from this one.
    local content="$WORK/f14-content" i
    mkdir -p "$content"
    for i in 1 2 3 4 5 6; do
        seq 1 1200000 | awk -v f="$i" '{ printf "file %d line %08d\n", f, $1 }' > "$content/big$i.bin"
    done
    local img0="$WORK/f14.dev1.img" img1="$WORK/f14.dev2.img" m="$WORK/mnt" l0 l1
    truncate -s 256M "$img0" "$img1"
    mkfs.btrfs -q -f --nodesize 16384 --sectorsize 4096 -d single -m raid1 "$img0" "$img1"
    attach_loop "$img0" l0
    attach_loop "$img1" l1
    sudo btrfs device scan "$l0" "$l1" >/dev/null
    mkdir -p "$m"
    sudo mount "$l0" "$m"
    for i in 1 2 3 4 5 6; do sudo cp "$content/big$i.bin" "$m/"; sudo sync; done
    sudo umount "$m"
    detach_loops
    chunk_devids "$img0"
    chunk_devids "$img0" | awk '$1 == "DATA" || $1 == "DATA|single" { split($2, a, ":"); seen[a[1]] = 1 }
        END { exit !(seen[1] && seen[2]) }' || { echo "f14: data chunks not on both devices" >&2; exit 1; }
    sha() { sha256sum < "$1" | awk '{print $1}'; }
    compress_fixture "$img0" "$DATA/f14_single_two_devices.dev1.img.zst"
    compress_fixture "$img1" "$DATA/f14_single_two_devices.dev2.img.zst"
    local args=()
    for i in 1 2 3 4 5 6; do args+=("/big$i.bin" "$(sha "$content/big$i.bin")"); done
    emit_expected_json f14_single_two_devices "${args[@]}"
}

build_f15_fscrypt_markers() {
    echo "=== F15 fscrypt markers ==="
    # btrfs fscrypt isn't in mainline Linux, so no tool writes it. The image is
    # an ordinary one with the on-disk markers set by hand: secret-extent.bin's
    # extent has its encryption byte set to 1, and secret-name.txt's directory
    # entries (DIR_ITEM and DIR_INDEX) carry the BTRFS_FT_ENCRYPTED type flag
    # 0x80. The leaves' CRC32C checksums are recomputed. plain.txt is
    # untouched.
    local content="$WORK/f15-content"
    mkdir -p "$content"
    seq 1 2000 | awk '{ printf "extent marked encrypted %06d\n", $1 }' > "$content/secret-extent.bin"
    printf 'name marked encrypted\n' > "$content/secret-name.txt"
    printf 'plain file\n' > "$content/plain.txt"
    find "$content" -exec touch -d "@0" {} +
    local img="$WORK/f15.img"
    truncate -s 128M "$img"
    mkfs.btrfs -q -f --rootdir "$content" --nodesize 16384 --sectorsize 4096 -m single "$img"
    btrfs inspect-internal dump-tree -t fs "$img" > "$WORK/f15.fs.txt"
    python3 - "$img" "$WORK/f15.fs.txt" <<'PY'
import re, struct, subprocess, sys
img_path, dump = sys.argv[1], open(sys.argv[2]).read()
NODESIZE = 16384

def crc32c(data):
    crc = 0xffffffff
    for b in data:
        crc ^= b
        for _ in range(8):
            crc = (crc >> 1) ^ 0x82f63b78 if crc & 1 else crc >> 1
    return crc ^ 0xffffffff

def physical(logical):
    out = subprocess.run(['btrfs-map-logical', '-l', str(logical), img_path],
                         capture_output=True, text=True, check=True).stdout
    return [int(m) for m in re.findall(r'physical (\d+)', out)]

# Leaves with their items: (leaf logical, [(key text, itemoff, itemsize, body)])
leaves, cur = [], None
for line in dump.splitlines():
    m = re.match(r'\s*leaf (\d+) items', line)
    if m:
        cur = (int(m.group(1)), [])
        leaves.append(cur)
        continue
    m = re.match(r'\s*item \d+ key \((.*?)\) itemoff (\d+) itemsize (\d+)', line)
    if m and cur:
        cur[1].append([m.group(1), int(m.group(2)), int(m.group(3)), ''])
    elif cur and cur[1]:
        cur[1][-1][3] += line + '\n'

inode_of = {}
for _, items in leaves:
    for key, off, size, body in items:
        m = re.search(r'location key \((\d+) INODE_ITEM 0\).*?\n.*?name: (\S+)', body, re.S)
        if 'DIR_INDEX' in key and m:
            inode_of[m.group(2)] = int(m.group(1))

img = open(img_path, 'r+b')
patched = set()
def patch(leaf_logical, at, fn):
    for phys in physical(leaf_logical):
        img.seek(phys)
        leaf = bytearray(img.read(NODESIZE))
        fn(leaf, at)
        leaf[0:32] = struct.pack('<I', crc32c(leaf[32:])) + bytes(28)
        img.seek(phys)
        img.write(leaf)
    patched.add(leaf_logical)

def set_encryption(leaf, at):
    leaf[at + 17] = 1                      # file_extent_item.encryption

def set_ft_encrypted(leaf, at):
    leaf[at + 29] |= 0x80                  # dir_item.type

secret_extent, secret_name = inode_of['secret-extent.bin'], inode_of['secret-name.txt']
done = {'extent': 0, 'dir': 0}
for leaf_logical, items in leaves:
    for key, off, size, body in items:
        at = 101 + off                     # leaf header, then item data
        if key.startswith(f'{secret_extent} EXTENT_DATA '):
            patch(leaf_logical, at, set_encryption); done['extent'] += 1
        if ('DIR_ITEM' in key or 'DIR_INDEX' in key) and 'name: secret-name.txt' in body:
            patch(leaf_logical, at, set_ft_encrypted); done['dir'] += 1
assert done == {'extent': 1, 'dir': 2}, done
PY
    btrfs check --readonly "$img" >/dev/null 2>&1 || true
    sha() { sha256sum < "$1" | awk '{print $1}'; }
    compress_fixture "$img" "$DATA/f15_fscrypt_markers.img.zst"
    emit_expected_json f15_fscrypt_markers \
        "/plain.txt" "$(sha "$content/plain.txt")"
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
    f10) build_f10_multi_leaf ;;
    f11) build_f11_lzo_segments ;;
    f12) build_f12_subvolumes ;;
    f13) build_f13_raid1_two_devices ;;
    f14) build_f14_single_two_devices ;;
    f15) build_f15_fscrypt_markers ;;
    all)
        build_f1_single_uncompressed
        build_f2_single_zstd
        build_f3_single_zlib
        build_f4_single_lzo
        build_f5_dup_metadata
        build_f8_sparse_no_holes
        build_f9_symlink_chain
        build_f10_multi_leaf
        build_f11_lzo_segments
        build_f12_subvolumes
        build_f13_raid1_two_devices
        build_f14_single_two_devices
        build_f15_fscrypt_markers
        ;;
    *) echo "Usage: $0 [all|f1|f2|f3|f4|f5|f8|f9|f10|f11|f12|f13|f14|f15]" >&2; exit 2 ;;
esac

echo "Done. Fixtures in $DATA/"
ls -la "$DATA"
