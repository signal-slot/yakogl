#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# triangle_selftest.sh - triangle.py against synthetic screenshots, no board.
#
#   tools/probes/stm32mp257f_dk/display_check/triangle_selftest.sh
#
# Images are written with colorbars.py's PNG writer: gc_triangle's picture
# (alone, scaled, letterboxed, red/blue swapped, upside down), a blank
# screen, the no-signal screen and colour bars. Exits 0 when all match.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool=$here/triangle.py
work=$(mktemp -d "${TMPDIR:-/tmp}/triangle-selftest.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT

gen=$work/gen.py
cat > "$gen" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import colorbars as cb

out = sys.argv[2]
BG = (16, 32, 48)          # gc_triangle's 0x00102030


def tri(w, h, bw=None, bh=None, ox=0, oy=0, swap=False, flip=False):
    """gc_triangle's picture: a bw x bh target at ox,oy in a w x h screen
    (black outside), Gouraud-shaded like the GPU does it."""
    bw, bh = bw or w, bh or h
    A = [(0.5, 0.1), (0.1, 0.9), (0.9, 0.9)]
    cols = [(255, 0, 0), (0, 255, 0), (0, 0, 255)]
    if swap:
        cols = [cols[2], cols[1], cols[0]]
    if flip:
        A = [(x, 1 - y) for x, y in A]
    (ax, ay), (cx, cy), (dx, dy) = [(x * bw, y * bh) for x, y in A]
    det = (cy - dy) * (ax - dx) + (dx - cx) * (ay - dy)
    rows = []
    for y in range(h):
        r = bytearray(3 * w)
        ty = y - oy
        if 0 <= ty < bh:
            for x in range(ox, min(w, ox + bw)):
                px = x - ox + 0.5
                l0 = ((cy - dy) * (px - dx) + (dx - cx) * (ty + .5 - dy)) / det
                l1 = ((dy - ay) * (px - dx) + (ax - dx) * (ty + .5 - dy)) / det
                l2 = 1 - l0 - l1
                if min(l0, l1, l2) >= 0:
                    v = [int(l0 * cols[0][i] + l1 * cols[1][i] + l2 * cols[2][i])
                         for i in range(3)]
                else:
                    v = BG
                r[3 * x:3 * x + 3] = bytes(v)
        rows.append(bytes(r))
    return rows


def put(name, rows, w, h):
    with open(out + "/" + name + ".png", "wb") as f:
        f.write(cb.encode_png(w, h, rows))


W, H = 320, 180
put("tri", tri(W, H), W, H)
put("big", tri(1280, 720), 1280, 720)
put("small", tri(W, H, 160, 96, 20, 10), W, H)       # smaller target, off-origin
put("scaled", tri(213, 120), 213, 120)
put("letter", tri(W, H, 240, 180, 40, 0), W, H)      # pillarboxed 4:3
put("swapped", tri(W, H, swap=True), W, H)
put("flipped", tri(W, H, flip=True), W, H)
put("blank", [bytes(BG) * W] * H, W, H)              # cleared, nothing drawn
put("black", [bytes(3 * W)] * H, W, H)
put("nosignal", [bytes((40, 40, 70)) * W] * H, W, H)
row = b"".join(bytes(c) * (W // 8) for _, c in cb.BARS)
put("bars", [row] * H, W, H)
PY
python3 -I "$gen" "$here" "$work" || { echo "selftest: cannot generate the images" >&2; exit 2; }

fail=0
n=0
# check NAME EXPECTED_EXIT EXPECTED_TEXT ARGS...
check() {
  local name=$1 want=$2 text=$3 got out
  shift 3
  out=$(python3 -I "$tool" "$@" 2>&1)
  got=$?
  n=$((n + 1))
  if [ "$got" = "$want" ] && [[ $out == *"$text"* ]]; then
    printf 'ok   %-10s %s\n' "$name" "$out"
  else
    printf 'FAIL %-10s exit %s (want %s), want "%s" in: %s\n' "$name" "$got" "$want" "$text" "$out"
    fail=$((fail + 1))
  fi
}

check tri       0 "triangle: PASS" "$work/tri.png"
check 720p      0 "triangle: PASS" "$work/big.png"
check small     0 "triangle: PASS" "$work/small.png"
check scaled    0 "triangle: PASS" "$work/scaled.png"
check letterbox 0 "triangle: PASS" "$work/letter.png"
check swapped   1 "triangle: FAIL: red and blue swapped" "$work/swapped.png"
check flipped   1 "triangle: FAIL: wrong shape" "$work/flipped.png"
check blank     1 "triangle: FAIL: no triangle" "$work/blank.png"
check black     1 "triangle: FAIL: no triangle" "$work/black.png"
check nosignal  1 "triangle: FAIL: no triangle" "$work/nosignal.png"
check bars      1 "triangle: FAIL" "$work/bars.png"
check missing   2 "unreadable input" "$work/none.png"

echo "selftest: $((n - fail)) ok, $fail failed"
[ "$fail" = 0 ]
