#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-pipeline-selftest.sh
#
# End-to-end self-test of the i.MX8MP dump path on a synthetic UART capture:
# one 800x600 frame dump in the sample's exact line format (driver/samples/
# zephyr/glmark2/src/main.cpp: "glmark2-fb: <scene> WxH begin", one
# "glmark2-fb: <y> <crc16> <base64>" per row, "glmark2-fb: end crc32 <hex>",
# "glmark2-picture: <scene>: ..."), with Zephyr UART3 console noise between
# the rows (tools/cts/imx8mp-console.conf). It then runs glmark2-fb.py,
# glmark2-picdiff.py and glmark2-sidebyside.py on it. Host only: no board, no
# network; everything is generated under $TMPDIR. The tools are not changed.
#
# Checks: (a) clean capture decodes, crc32 and picture line agree; (b) a
# capture with a dropped row, a corrupted row and a truncated tail is rejected
# naming the rows, never patched; (c) picdiff equal/different; (d) sidebyside
# writes a PNG. A decoder defect is reported as XFAIL (not FAIL) with its
# reproducer; the exit status is 0 for XFAIL and 1 for FAIL.
set -u
here=$(cd "$(dirname "$0")" && pwd)
base=${TMPDIR:-}
if [ -z "$base" ]; then echo "TMPDIR is not set" >&2; exit 2; fi
work=$(mktemp -d "$base/imx8mp-pipeline.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
fail=0
xfail=0

cat >"$work/gen.py" <<'PY'
import base64, binascii, importlib.util, os, sys
here, out = sys.argv[1], sys.argv[2]
spec = importlib.util.spec_from_file_location("fb", os.path.join(here, "glmark2-fb.py"))
fb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fb)
W, H = 800, 600
scene = "[build] use-vbo=false"


def px(x, y):
    if 300 <= x < 500 and 200 <= y < 400:
        return bytes((250, 40 + (x & 63), 30))
    return bytes(((x * 255) // W, (y * 255) // H, ((x + y) * 255) // (W + H)))


rows = [b"".join(px(x, y) for x in range(W)) for y in range(H)]
probes, c, cq, mean = fb.picture_value(W, H, rows)
pic = ("%dx%d; probes c %s00, tl %s00, tr %s00, bl %s00, br %s00; crc16 %04x, crc16q %04x; "
       "mean %d %d %d; black no" % ((W, H) + tuple(p.hex() for p in probes) + (c, cq) + tuple(mean)))
lines = ["*** Booting Zephyr OS build v4.x ***", "[00:00:00.100,000] <inf> gpu: vivante init",
         "glmark2-fb: %s %dx%d begin" % (scene, W, H)]
for y, r in enumerate(rows):
    lines.append("glmark2-fb: %d %04x %s" % (y, fb.crc16(r), base64.b64encode(r).decode()))
    if y % 50 == 7:
        lines.append("[00:00:%02d.%03d,000] <inf> gpu: heartbeat %d" % (1 + y // 100, y, y))
    if y % 120 == 3:
        lines.append("uart:~$ ")
lines.append("glmark2-fb: end crc32 %08x" % (binascii.crc32(b"".join(rows)) & 0xFFFFFFFF))
lines.append("glmark2-picture: %s: %s" % (scene, pic))


def wr(name, ls):
    with open(os.path.join(out, name), "w", newline="") as f:
        f.write("".join(l + "\r\n" for l in ls))


def idx(ls, y):
    p = "glmark2-fb: %d " % y
    return next(i for i, l in enumerate(ls) if l.startswith(p))


wr("clean.log", lines)
bad = list(lines)
del bad[idx(bad, 100)]                      # row 100 dropped
i = idx(bad, 250)                           # row 250 corrupted (one base64 char)
bad[i] = bad[i][:30] + ("B" if bad[i][30] != "B" else "C") + bad[i][31:]
del bad[idx(bad, 500):]                     # tail truncated from row 500 on
wr("bad.log", bad)
fb.write_png(os.path.join(out, "board.png"), W, H, rows)
fb.write_png(os.path.join(out, "ref-same.png"), W, H, rows)
chg = [bytearray(r) for r in rows]
for y in range(150, 450):
    for x in range(100, 700):
        chg[y][x * 3:x * 3 + 3] = bytes((0, 0, 255))
fb.write_png(os.path.join(out, "ref-diff.png"), W, H, [bytes(r) for r in chg])
PY

cat >"$work/magenta.py" <<'PY'
import sys, zlib


def rows(p):
    d = open(p, "rb").read()
    z = zlib.decompress(d[d.index(b"IDAT") + 4:d.index(b"IEND") - 8])
    return [z[1 + y * 2401:1 + y * 2401 + 2400] for y in range(600)]


b, g = rows(sys.argv[1]), rows(sys.argv[2])
m = bytes((255, 0, 255)) * 800
sys.exit(0 if b[100] == m and b[250] == m and b[599] == m and b[0] == g[0] and b[499] == g[499] else 1)
PY

check() { # name, status
    if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=$((fail + 1)); fi
}
fb() { python3 -I "$here/glmark2-fb.py" "$@"; }

python3 -I "$work/gen.py" "$here" "$work" || { echo "FAIL generating the synthetic capture"; exit 1; }
echo "synthetic capture: 800x600, $(wc -c <"$work/clean.log") bytes, 600 rows, console noise interleaved"

# (a) clean capture
fb "$work/clean.log" "$work/clean.png" >"$work/a.out" 2>&1
check "(a) clean capture exits 0" $?
grep -q "600 of 600 rows good" "$work/a.out"; check "(a) all 600 rows good" $?
grep -q "frame crc32 [0-9a-f]* agrees" "$work/a.out"; check "(a) frame crc32 agrees" $?
grep -q "picture value agrees" "$work/a.out"; check "(a) picture line matches" $?
cmp -s "$work/clean.png" "$work/board.png"; check "(a) decoded PNG is the synthetic frame" $?

# (b) dropped row, corrupted row, truncated tail
fb "$work/bad.log" "$work/bad.png" >"$work/b.out" 2>&1
rc=$?
[ "$rc" = 1 ]; check "(b) damaged capture exits 1" $?
grep -q "damaged: row 100 missing" "$work/b.out"; check "(b) names the dropped row 100" $?
grep -Eq "damaged: row 250 (crc16 differs|not base64|has)" "$work/b.out"; check "(b) names the corrupted row 250" $?
# the decoder lists the first 40 problems, then "... N more": 2 + 100 in all
grep -q "damaged: row 500 missing" "$work/b.out" && grep -q "\.\.\. 62 more" "$work/b.out"
check "(b) names the truncated rows from 500 (40 listed, 62 more counted)" $?
grep -q "pixels are never patched" "$work/b.out"; check "(b) says rerun, never patch" $?
! grep -q "agrees" "$work/b.out"; check "(b) no agreement claimed" $?
grep -v "^glmark2-fb: end\|^glmark2-picture" "$work/clean.log" >"$work/noend.log"
fb "$work/noend.log" "$work/noend.png" >"$work/b2.out" 2>&1
rc=$?
[ "$rc" = 1 ] && grep -q "no end line" "$work/b2.out"; check "(b) missing end line rejected" $?
python3 -I "$work/magenta.py" "$work/bad.png" "$work/board.png"; check "(b) bad rows magenta, good rows untouched" $?

# (c) picdiff against a synthetic reference
python3 -I "$here/glmark2-picdiff.py" "$work/clean.png" "$work/ref-same.png" "$work/pd-same.png" >"$work/c1.out" 2>&1
rc1=$?
python3 -I "$here/glmark2-picdiff.py" "$work/clean.png" "$work/ref-diff.png" "$work/pd-diff.png" >"$work/c2.out" 2>&1
rc2=$?
[ "$rc1" = 0 ] && [ "$rc2" = 0 ]; check "(c) picdiff exits 0 on both" $?
sed 's/^/     same: /' "$work/c1.out"
sed 's/^/     diff: /' "$work/c2.out"
cmp -s "$work/c1.out" "$work/c2.out"; [ $? != 0 ]; check "(c) the two reports differ" $?
grep -Eq "(^| )0(\.0+)? ?%|none" "$work/c1.out"; check "(c) identical reference: no differing pixels" $?
grep -q "none" "$work/c2.out"; [ $? != 0 ]; check "(c) changed reference: a difference box is reported" $?
[ -s "$work/pd-same.png" ] && [ -s "$work/pd-diff.png" ]; check "(c) picdiff wrote its images" $?

# (d) side by side
python3 -I "$here/glmark2-sidebyside.py" "$work/clean.png" "$work/ref-diff.png" "$work/sbs.png" \
    --label-board IMX8MP --label-ref LLVMPIPE >"$work/d.out" 2>&1
rc=$?
[ "$rc" = 0 ] && [ "$(head -c 4 "$work/sbs.png" | tail -c 3)" = PNG ]; check "(d) sidebyside writes a PNG" $?

echo "pipeline selftest: $fail failed, $xfail expected failure(s)"
[ "$fail" = 0 ]
