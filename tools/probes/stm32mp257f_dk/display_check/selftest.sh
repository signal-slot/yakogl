#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# selftest.sh - colorbars.py against synthetic screenshots, no board.
#
#   tools/probes/stm32mp257f_dk/display_check/selftest.sh
#
# Generates PNG/PPM files with the tool's own writer (correct bars, RGBA,
# R/B swapped, black, half the bars, uneven widths, a no-signal screen, a
# letterboxed picture, a triangle), runs the tool on each and checks the
# verdict and the exit code. Prints each case's verdict; exits 0 when all
# match.
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool=$here/colorbars.py
work=$(mktemp -d "${TMPDIR:-/tmp}/colorbars-selftest.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT

gen=$work/gen.py
cat > "$gen" <<'PY'
import sys
sys.path.insert(0, sys.argv[1])
import colorbars as cb

out = sys.argv[2]
W, H = 320, 180
RGB = dict(cb.BARS)


def bars(names, widths=None, lead=0, alpha=False, w=W):
    n = 4 if alpha else 3
    pad = bytes([0, 0, 0] + ([255] if alpha else []))
    widths = widths or [(w - lead) // len(names)] * len(names)
    row = bytearray(pad * lead)
    for name, k in zip(names, widths):
        row += bytes(RGB[name] + ((255,) if alpha else ())) * k
    row += pad * (w - len(row) // n)
    return [bytes(row)] * H


def put(name, rows, w=W, alpha=False):
    with open(out + "/" + name + ".png", "wb") as f:
        f.write(cb.encode_png(w, H, rows, alpha))


good = cb.NAMES
put("good", bars(good))
put("rgba", bars(good, alpha=True), alpha=True)
with open(out + "/good.ppm", "wb") as f:
    f.write(b"P6\n# c\n%d %d\n255\n" % (W, H) + b"".join(bars(good)))
put("swapped", bars(cb.SWAPPED))
put("black", bars(["black"], [W]))
put("half", bars(good[:4] + ["black"] * 4))
put("scaled", bars(good, [80, 80, 40, 20, 20, 20, 20, 40]))
put("lead", bars(good, lead=24, w=W + 24), w=W + 24)
put("order", bars(good[::-1]))
# the KVM's no-signal screen: a flat dark blue-grey
put("nosignal", [bytes((40, 40, 70)) * W] * H)
# an orange triangle, apex at (0.5,0.1), base at y=0.9 from x=0.15 to 0.85
rows = []
for y in range(H):
    r = bytearray(3 * W)
    v = y / (H - 1)
    if 0.1 <= v <= 0.9:
        half = (v - 0.1) / 0.8 * 0.35
        for x in range(int((0.5 - half) * W), int((0.5 + half) * W)):
            r[3 * x:3 * x + 3] = bytes((255, 128, 0))
    rows.append(bytes(r))
put("tri", rows)
put("tri_empty", bars(["black"], [W]))
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

check good      0 "colorbars: PASS: 8 bars white yellow cyan green magenta red blue black" "$work/good.png"
check rgba      0 "colorbars: PASS" "$work/rgba.png"
check ppm       0 "colorbars: PASS" "$work/good.ppm"
check letterbox 0 "colorbars: PASS" "$work/lead.png"
check swapped   1 "colorbars: FAIL: red and blue swapped" "$work/swapped.png"
check black     1 "colorbars: FAIL: no signal" "$work/black.png"
check nosignal  1 "colorbars: FAIL: no signal" "$work/nosignal.png"
check half      1 "colorbars: FAIL: partial: 5 of 8" "$work/half.png"
check scaled    1 "colorbars: FAIL: scaled" "$work/scaled.png"
check order     1 "colorbars: FAIL: wrong order" "$work/order.png"
check missing   2 "unreadable input" "$work/none.png"
apex=(--triangle --apex 0.5,0.1 --apex 0.15,0.9 --apex 0.85,0.9)
check tri       0 "triangle: PASS" "${apex[@]}" --color 255,128,0 "$work/tri.png"
check tri_empty 1 "triangle: FAIL: apex 0" "${apex[@]}" --color 255,128,0 "$work/tri_empty.png"
check tri_color 1 "triangle: FAIL" "${apex[@]}" --color 0,0,255 "$work/tri.png"

echo "selftest: $((n - fail)) ok, $fail failed"
[ "$fail" = 0 ]
