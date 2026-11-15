#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# regen.sh: build the DK u-boot-env fixtures of this directory (README.md)
# from ST's default environment, driver/tools/mp2-standalone/st-env/
# stm32mp25-dk-initial-env.txt, with tools/uboot_env.py.
#
#   regen.sh           write the *.bin files here (only those that differ)
#   regen.sh --check   build into $TMPDIR and compare byte for byte; exit 1
#                      on a difference or a missing file, write nothing here
#
# Every image is a redundant environment of two 0x2000-byte copies (CRC32,
# flags byte, data area), copy 1 at 0 and copy 2 at 0x2000, 0x4000 bytes in
# all: tools/uboot_env.py --redundant --size 0x2000. Deterministic: no time,
# no randomness, no host value goes into a byte.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
root=$(cd "$here/../../.." && pwd)
txt=$root/driver/tools/mp2-standalone/st-env/stm32mp25-dk-initial-env.txt
check=0
case "${1:-}" in
--check) check=1 ;;
"") ;;
*) echo "usage: regen.sh [--check]" >&2; exit 2 ;;
esac

say() { echo "regen.sh: $*" >&2; }
ENV() { python3 -I "$root/tools/uboot_env.py" --redundant --size 0x2000 "$@"; }

IMAGES="pristine installed counting2 counting1 left0 one-copy-bad both-bad"

# The values of driver/tools/mp2-standalone/mp2-standalone.cmd and the
# design note's section 3 (the runner's one environment write of "default").
PREFIXES=$(sed -n 's/^boot_prefixes=//p' "$txt")
[ "$PREFIXES" = "/ /boot/" ] || { say "boot_prefixes in $txt is '$PREFIXES', not '/ /boot/'"; exit 1; }
STANDALONE_PREFIX=/yakogl/standalone/

tmp=$(mktemp -d "${TMPDIR:?TMPDIR must be set (never /tmp)}/mp2s-env.XXXXXX")
trap 'rm -rf "$tmp"' EXIT

# pristine: ST's defaults as U-Boot's env_export writes them: one entry per
# name (a repeated name keeps its last value, as U-Boot's import does: the
# text has boot_net_usb_start twice), in the text's sorted order, the data
# area padded with 0x00; both copies equal, flags 0 (a tie: copy 1 active).
python3 -I - "$txt" "$tmp/pristine.bin" <<'EOF'
import sys, zlib
src, out = sys.argv[1], sys.argv[2]
SIZE = 0x2000
with open(src, "rb") as fh:
    lines = [ln for ln in fh.read().split(b"\n") if ln]
last = {}
for i, ln in enumerate(lines):
    name = ln.split(b"=", 1)[0]
    if b"=" not in ln or not name:
        sys.exit("regen.sh: %r is not NAME=VALUE" % ln)
    last[name] = i
entries = [ln for i, ln in enumerate(lines) if last[ln.split(b"=", 1)[0]] == i]
data = b"".join(e + b"\0" for e in entries) + b"\0"
area = SIZE - 5
if len(data) > area:
    sys.exit("regen.sh: the defaults need %d bytes, a copy holds %d" % (len(data), area))
data += b"\0" * (area - len(data))
copy = zlib.crc32(data).to_bytes(4, "little") + b"\0" + data
with open(out, "wb") as fh:
    fh.write(copy * 2)
EOF

# Each step is one save with uboot_env.py: the inactive copy gets the edit
# and flags = active + 1, the active one is left as it was.
ENV set "$tmp/pristine.bin" "$tmp/installed.bin" \
    mp2s_ver=1 "mp2s_prefixes=$PREFIXES" "boot_prefixes=$STANDALONE_PREFIX $PREFIXES" >/dev/null
ENV set "$tmp/installed.bin" "$tmp/counting2.bin" mp2s_left=2 >/dev/null
ENV set "$tmp/counting2.bin" "$tmp/counting1.bin" mp2s_left=1 >/dev/null
ENV set "$tmp/counting1.bin" "$tmp/left0.bin" mp2s_left=0 >/dev/null

# Broken CRCs: byte 0 of a copy's CRC32 inverted, nothing else changed.
python3 -I - "$tmp/counting1.bin" "$tmp/one-copy-bad.bin" "$tmp/both-bad.bin" <<'EOF'
import sys
src, one, both = sys.argv[1:]
with open(src, "rb") as fh:
    img = bytearray(fh.read())
img[0] ^= 0xff
with open(one, "wb") as fh:
    fh.write(img)
img[0x2000] ^= 0xff
with open(both, "wb") as fh:
    fh.write(img)
EOF

status=0
for n in $IMAGES; do
    f=$n.bin
    if [ "$check" = 1 ]; then
        if [ ! -f "$here/$f" ]; then
            say "$f: missing"; status=1
        elif ! cmp -s "$tmp/$f" "$here/$f"; then
            say "$f: differs from a rebuild"; status=1
        fi
    elif ! cmp -s "$tmp/$f" "$here/$f" 2>/dev/null; then
        cp "$tmp/$f" "$here/$f"
        say "wrote $f"
    fi
done
if [ "$check" = 1 ] && [ "$status" = 0 ]; then
    say "all images match a rebuild"
fi
exit "$status"
