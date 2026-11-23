#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# End-to-end self-test of the i.MX8MP glmark2 picture path on a synthetic UART
# capture: no board, no network. One 800x600 frame dump is written in the exact
# line format the sample prints (driver/samples/zephyr/glmark2/src/main.cpp:
# "glmark2-fb: <scene> WxH begin", one "glmark2-fb: <y> <crc16> <base64>" per
# row, "glmark2-fb: end crc32 <hex>", then "glmark2-picture: <scene>: ..."),
# with UART3 console noise (tools/cts/imx8mp-console.conf: printk lines, CRLF,
# NULs at a reconnect, a prompt) between the lines. It is decoded by
# glmark2-fb.py, compared by glmark2-picdiff.py and laid out by
# glmark2-sidebyside.py. Checks:
#   (a) the clean capture decodes, its frame crc32 and picture line agree
#   (b) a capture with a dropped row, a corrupted row and a truncated tail is
#       rejected (exit 1), naming the bad rows, never patched
#   (c) picdiff: identical reference -> equal, changed reference -> different
#   (d) sidebyside writes a PNG
# Usage: bash tools/cts/imx8mp-glmark2-pipeline-selftest.sh
# Everything is generated under $TMPDIR (mktemp -d). Exit 0: all checks pass;
# 1: a failure. A known decoder defect would be an XFAIL line (none so far).
set -u
here=$(cd "$(dirname "$0")" && pwd)
fb=$here/glmark2-fb.py
pd=$here/glmark2-picdiff.py
sbs=$here/glmark2-sidebyside.py
if [ -z "${TMPDIR:-}" ]; then
    echo "TMPDIR is not set; this test never writes to /tmp" >&2
    exit 1
fi
tmp=$(mktemp -d "$TMPDIR/imx8mp-pipeline.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT

pass=0 fail=0 xfail=0
ok() { pass=$((pass + 1)); echo "ok   $1"; }
bad() { fail=$((fail + 1)); echo "FAIL $1"; }
check() { # name, command...
    local n=$1
    shift
    if "$@"; then ok "$n"; else bad "$n"; fi
}
has() { grep -q -- "$2" "$1"; }
lacks() { ! grep -q -- "$2" "$1"; }

# --- synthetic capture ------------------------------------------------------
python3 -I - "$tmp" "$fb" <<'PY' || { echo "FAIL cannot build the synthetic capture"; exit 1; }
import base64, binascii, importlib.util, os, sys
tmp, fbpath = sys.argv[1:3]
spec = importlib.util.spec_from_file_location("fb", fbpath)
fb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fb)
w, h = 800, 600
scene = "[build] use-vbo=false"

def rows_of(tweak):
    out = []
    for y in range(h):
        r = bytearray()
        for x in range(w):
            r += bytes(((x * 255 // w + tweak) & 255, (y * 255 // h) & 255, ((x ^ y) + tweak) & 255))
        out.append(bytes(r))
    return out

rows = rows_of(0)
probes, c, cq, mean = fb.picture_value(w, h, rows)
pic = ("%dx%d; probes c %s, tl %s, tr %s, bl %s, br %s; crc16 %04x, crc16q %04x; mean %d %d %d; black no"
       % ((w, h) + tuple(p.hex() + "ff" for p in probes) + (c, cq) + tuple(mean)))
crc32 = binascii.crc32(b"".join(rows)) & 0xFFFFFFFF
head = ["glmark2-fb: %s %dx%d begin" % (scene, w, h)]
body = ["glmark2-fb: %d %04x %s" % (y, fb.crc16(r), base64.b64encode(r).decode())
        for y, r in enumerate(rows)]
tail = ["glmark2-fb: end crc32 %08x" % crc32, "glmark2-picture: %s: %s" % (scene, pic)]

def noisy(lines):
    out = ["*** Booting Zephyr OS build v4 ***", "\x00\x00[00:00:00.500,000] <inf> gpu: ready"]
    for i, l in enumerate(lines):
        out.append(l + "\r")
        if i % 97 == 5:
            out.append("[00:00:01.%03d,000] <inf> gpu: heartbeat %d\r" % (i % 1000, i))
        if i % 211 == 7:
            out.append("\x00uart:~$ \r")
    return "\n".join(out) + "\n"

def put(name, text):
    with open(os.path.join(tmp, name), "w", newline="") as f:
        f.write(text)

put("clean.log", noisy(head + body + tail))
put("clean.info", "%08x\n" % crc32)
# damaged: row 100 dropped, row 200 one base64 char flipped, tail cut after
# row 450 with the last line cut mid-row; no end line, no picture line
d = list(body)
d[200] = d[200][:40] + ("A" if d[200][40] != "A" else "B") + d[200][41:]
del d[100]
d = d[:450]
d[-1] = d[-1][:30]
put("damaged.log", noisy(head + d))
# truncated alone: 300 good rows and a cut 301st
put("trunc.log", noisy(head + body[:300] + [body[300][:25]]))
# references: the same pixels, and a changed one
fb.write_png(os.path.join(tmp, "ref-same.png"), w, h, rows)
fb.write_png(os.path.join(tmp, "ref-diff.png"), w, h, rows_of(90))
PY

# --- (a) clean --------------------------------------------------------------
python3 "$fb" "$tmp/clean.log" "$tmp/clean.png" >"$tmp/a.out" 2>&1
rc=$?
check "(a) clean capture decodes, exit 0" test $rc -eq 0
check "(a) 800x600, 600 of 600 rows good" has "$tmp/a.out" "800x600, 600 of 600 rows good"
want_crc=$(sed -n 1p "$tmp/clean.info")
check "(a) frame crc32 $want_crc agrees" has "$tmp/a.out" "frame crc32 $want_crc agrees"
check "(a) picture line agrees" has "$tmp/a.out" "picture value agrees"
check "(a) PNG written" test -s "$tmp/clean.png"

# --- (b) damaged ------------------------------------------------------------
python3 "$fb" "$tmp/damaged.log" "$tmp/damaged.png" >"$tmp/b.out" 2>&1
rc=$?
check "(b) dropped+corrupted+truncated rejected, exit 1" test $rc -eq 1
check "(b) names the dropped row 100" has "$tmp/b.out" "row 100 missing"
check "(b) names the corrupted row 200" has "$tmp/b.out" "row 200 "
check "(b) names a missing tail row" has "$tmp/b.out" "row 451 missing"
check "(b) says rerun, never patched" has "$tmp/b.out" "pixels are never patched"
check "(b) no 'agrees' on a damaged dump" lacks "$tmp/b.out" "agrees"
python3 -I - "$tmp/damaged.png" <<'PY'
import sys, zlib
d = open(sys.argv[1], "rb").read()
raw = zlib.decompress(d[d.index(b"IDAT") + 4:d.index(b"IEND") - 8])
st = 800 * 3 + 1
row = lambda y: raw[y * st + 1:(y + 1) * st]
mag = bytes((255, 0, 255)) * 800
sys.exit(0 if row(100) == mag and row(200) == mag and row(99) != mag and row(101) != mag else 1)
PY
check "(b) damaged rows are magenta, neighbours untouched" test $? -eq 0
python3 "$fb" "$tmp/trunc.log" "$tmp/trunc.png" >"$tmp/b2.out" 2>&1
rc=$?
check "(b) truncated capture rejected, exit 1" test $rc -eq 1
check "(b) truncated capture names row 300" has "$tmp/b2.out" "row 300 "
check "(b) truncated capture names row 301 missing" has "$tmp/b2.out" "row 301 missing"

# --- (c) picdiff ------------------------------------------------------------
python3 "$pd" "$tmp/clean.png" "$tmp/ref-same.png" "$tmp/diff-same.png" >"$tmp/c1.out" 2>&1
check "(c) picdiff identical: exit 0" test $? -eq 0
python3 "$pd" "$tmp/clean.png" "$tmp/ref-diff.png" "$tmp/diff-diff.png" >"$tmp/c2.out" 2>&1
check "(c) picdiff changed: exit 0" test $? -eq 0
check "(c) picdiff identical: equal (no pixel over the threshold)" has "$tmp/c1.out" "none"
check "(c) picdiff changed: different (pixels over the threshold)" lacks "$tmp/c2.out" "none"
check "(c) picdiff identical and changed differ" test -n "$(cmp "$tmp/c1.out" "$tmp/c2.out" 2>&1)"
python3 "$pd" "$tmp/clean.png" "$tmp/nonexistent.png" "$tmp/x.png" >/dev/null 2>&1
check "(c) picdiff bad input exits 2" test $? -eq 2

# --- (d) sidebyside ---------------------------------------------------------
python3 "$sbs" "$tmp/clean.png" "$tmp/ref-same.png" "$tmp/sbs.png" \
    --label-board "BOARD" --label-ref "LLVMPIPE" >"$tmp/d.out" 2>&1
check "(d) sidebyside exit 0" test $? -eq 0
python3 -I - "$tmp/sbs.png" <<'PY'
import struct, sys
d = open(sys.argv[1], "rb").read()
if d[:8] != b"\x89PNG\r\n\x1a\n":
    sys.exit(1)
w, h = struct.unpack(">II", d[16:24])
sys.exit(0 if w >= 1600 and h >= 600 else 1)
PY
check "(d) sidebyside wrote a PNG holding both pictures (>=1600x600)" test $? -eq 0

echo "pipeline self-test: $pass ok, $fail failed, $xfail xfail"
[ "$fail" -eq 0 ]
