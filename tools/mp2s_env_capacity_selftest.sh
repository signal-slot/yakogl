#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of tools/mp2s_env_capacity.py: no board, python3 only.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/mp2s_env_capacity.py"
tmp=$(mktemp -d "$TMPDIR/mp2s-cap.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=1; }
expect() { # name want-exit cmd...
    local name=$1 want=$2
    shift 2
    "$@" >"$tmp/out" 2>"$tmp/err"
    local rc=$?
    if [ "$rc" = "$want" ]; then ok "$name"; else bad "$name: exit $rc, want $want"; cat "$tmp/out" "$tmp/err"; fi
}

adds=(mp2s_ver=1 mp2s_left=2 "mp2s_prefixes=/ /mmc0_" "boot_prefixes=/yakogl/standalone/ /ssl/ /mmc0_")

# synthesize redundant 0x2000 images, copy 1 active, with a filler variable
python3 -I - "$tmp" <<'PY'
import sys, zlib
tmp = sys.argv[1]
def img(fill):
    area = 0x2000 - 5
    ents = [b"boot_prefixes=/ /mmc0_", b"bootdelay=1", b"filler=" + b"x" * fill]
    data = b"".join(e + b"\0" for e in ents) + b"\0"
    assert len(data) <= area
    data += b"\xff" * (area - len(data))
    c = zlib.crc32(data).to_bytes(4, "little")
    return (c + b"\x01" + data) + (c + b"\x00" + data)
# the additions take about 135 bytes: "near" leaves about 100 free, "full" a negative number
open(tmp + "/room.bin", "wb").write(img(3000))
open(tmp + "/near.bin", "wb").write(img(8187 - 40 - 7 - 100 - 3))
open(tmp + "/full.bin", "wb").write(img(8187 - 40 - 7 - 3))
open(tmp + "/room.txt", "w").write("bootdelay=1\nboot_prefixes=/ /mmc0_\n")
PY

expect "room: fits" 0 python3 -I "$tool" --image "$tmp/room.bin" "${adds[@]}"
expect "near-full: too tight" 1 python3 -I "$tool" --image "$tmp/near.bin" "${adds[@]}"
grep -q "TOO TIGHT" "$tmp/out" && ok "near-full: says too tight" || bad "near-full: message"
expect "near-full, margin 0 fits" 0 python3 -I "$tool" --image "$tmp/near.bin" --margin 0 "${adds[@]}"
expect "full: does not fit" 1 python3 -I "$tool" --image "$tmp/full.bin" --margin 0 "${adds[@]}"
grep -q "DOES NOT FIT" "$tmp/out" && ok "full: says does not fit" || bad "full: message"
expect "text env fits" 0 python3 -I "$tool" --text "$tmp/room.txt" "${adds[@]}"
expect "text env, tiny --size: no fit" 1 python3 -I "$tool" --text "$tmp/room.txt" --size 0x60 --margin 0 "${adds[@]}"
expect "usage: no additions" 2 python3 -I "$tool" --image "$tmp/room.bin"
expect "usage: bad addition" 2 python3 -I "$tool" --image "$tmp/room.bin" nonsense
expect "usage: missing file" 2 python3 -I "$tool" --image "$tmp/nope.bin" a=b
expect "usage: no env" 2 python3 -I "$tool" a=b

# the written image: both copies valid, CRC round trip, flags as env_save
expect "write out" 0 python3 -I "$tool" --image "$tmp/room.bin" --out "$tmp/out.bin" "${adds[@]}"
if python3 -I - "$tmp/out.bin" <<'PY'
import sys, zlib
d = open(sys.argv[1], "rb").read()
assert len(d) == 0x4000
flags = []
for off in (0, 0x2000):
    c, f, area = int.from_bytes(d[off:off+4], "little"), d[off+4], d[off+5:off+0x2000]
    assert zlib.crc32(area) == c, "crc"
    flags.append(f)
    ents = area[:area.index(b"\0\0") + 1].split(b"\0")
    assert b"mp2s_left=2" in ents and b"filler=" + b"x" * 3000 in ents
    assert len([e for e in ents if e.startswith(b"boot_prefixes=")]) == 1
    assert b"boot_prefixes=/yakogl/standalone/ /ssl/ /mmc0_" in ents
assert flags == [3, 2], flags   # copy 2 is written first with 1+1, then copy 1 with 2+1
PY
then ok "round trip: CRC32, flags, strings"; else bad "round trip"; fi
if python3 -I "$here/uboot_env.py" --size 0x2000 --redundant check "$tmp/out.bin" >/dev/null; then
    ok "uboot_env.py check agrees"
else
    bad "uboot_env.py check"
fi
if cmp -s "$tmp/room.bin" "$tmp/out.bin"; then bad "output equals input"; else ok "output differs from input"; fi

if [ "$fail" = 0 ]; then echo "mp2s_env_capacity selftest: all ok"; else echo "mp2s_env_capacity selftest: FAILED"; fi
exit "$fail"
