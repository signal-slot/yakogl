#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/glmark2-photo-probes-selftest.sh
#
# Host-only: tools/cts/glmark2-photo-probes.py on synthetic panel screenshots
# generated under $TMPDIR: a 1280x800 panel with the 800x600 window at (0,0)
# and known probe colours, shown upright and turned 90/180/270 degrees, scaled
# to 1280x720 and 800x500, all black, with a colour cast, and bad inputs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-.}/photo-probes.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
PROBES="$HERE/glmark2-photo-probes.py"
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=$((fail + 1)); }

cat > "$T/gen.py" <<'PY'
import struct, sys, zlib
# gen.py <out> <w> <h> <rotation> <cast R,G,B added> <black 0|1>
out, W, H, rot = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
cast = [int(v) for v in sys.argv[5].split(",")]
black = sys.argv[6] == "1"
PW, PH = 1280, 800
COL = {"c": (200, 160, 40), "tl": (24, 28, 24), "tr": (30, 80, 220),
       "bl": (220, 30, 30), "br": (40, 200, 90)}
POS = {"c": (400, 300), "tl": (200, 150), "tr": (600, 150), "bl": (200, 450), "br": (600, 450)}
BG = (60, 70, 80)
def panel(x, y):
    if black or x >= 800 or y >= 600:
        return (0, 0, 0)
    for k, (px, py) in POS.items():
        if abs(x - px) <= 20 and abs(y - py) <= 20:
            return COL[k]
    return BG
# the upright screenshot of size uw x uh: nearest-neighbour from the panel
uw, uh = (H, W) if rot in (90, 270) else (W, H)
up = [[panel(x * PW // uw, y * PH // uh) for x in range(uw)] for y in range(uh)]
def turn(img, deg):  # clockwise
    for _ in range(deg // 90):
        img = [list(r) for r in zip(*img[::-1])]
    return img
img = turn(up, rot)
rows = b"".join(b"\0" + bytes(min(255, max(0, v + (0 if p == (0, 0, 0) and black else cast[i])))
                              for p in r for i, v in enumerate(p)) for r in img)
def chunk(t, b):
    return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xFFFFFFFF)
with open(out, "wb") as f:
    f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", len(img[0]), len(img), 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
PY

# the reference column: the probes of the generator, as the table writes them
cat > "$T/pictures.txt" <<'EOF'
# test table
tolerance 8
imx8mp | [test] scene=ref | 800x600; probes c c8a02800, tl 181c1800, tr 1e50dc00, bl dc1e1e00, br 28c85a00; crc16 0000, crc16q 0000; mean 1 2 3; black no
imx8mp | [test] scene=swap | 800x600; probes c 28c85a00, tl dc1e1e00, tr 181c1800, bl 1e50dc00, br c8a02800; crc16 0000, crc16q 0000; mean 1 2 3; black no
imx8mp | [test] scene=dark | 800x600; probes c 20202000, tl 10101000, tr 08080800, bl 00000000, br 18181800; crc16 0000, crc16q 0000; mean 1 2 3; black no
EOF

gen() { python3 -I "$T/gen.py" "$@"; }
REF="c c8a02800, tl 181c1800, tr 1e50dc00, bl dc1e1e00, br 28c85a00"

check_probes() { # name png args... : the probes line must equal REF (exact or within 2)
    local name=$1 png=$2; shift 2
    local out
    out=$(python3 -I "$PROBES" "$png" "$@") || { bad "$name: exit $?"; return; }
    python3 -I - "$out" "$REF" <<'PY' && ok "$name" || bad "$name: $out"
import re, sys
got = re.search(r"probes (.*?); mean", sys.argv[1]).group(1)
want = sys.argv[2]
g = re.findall(r"([a-z]+) ([0-9a-f]{8})", got)
w = re.findall(r"([a-z]+) ([0-9a-f]{8})", want)
assert [a for a, _ in g] == [a for a, _ in w] == ["c", "tl", "tr", "bl", "br"], g
for (_, a), (_, b) in zip(g, w):
    assert all(abs(int(a[i:i+2], 16) - int(b[i:i+2], 16)) <= 2 for i in (0, 2, 4)), (a, b)
assert "approximate" in sys.argv[1]
PY
}

gen "$T/a.png" 1280 800 0 0,0,0 0
check_probes "upright 1280x800" "$T/a.png"
gen "$T/b.png" 1280 720 0 0,0,0 0
check_probes "scaled to 1280x720" "$T/b.png"
gen "$T/c.png" 800 500 0 0,0,0 0
check_probes "scaled to 800x500" "$T/c.png"
for r in 90 180 270; do
    gen "$T/r$r.png" 1280 720 $r 0,0,0 0
    check_probes "rotation $r (1280x720 before turning)" "$T/r$r.png" --rotation $r
done
python3 -I "$PROBES" "$T/r90.png" --rotation 0 >/dev/null 2>&1 \
    && python3 -I "$PROBES" "$T/r90.png" --rotation 0 | grep -q "$REF" \
    && bad "wrong rotation read as right" || ok "wrong rotation does not give the probes"

# the window flag: the same picture shifted by a window offset
python3 -I "$PROBES" "$T/a.png" --window 0,0,800,600 | grep -q "800x600" \
    && ok "--window default size" || bad "--window default size"

# black window
gen "$T/k.png" 1280 720 0 0,0,0 1
out=$(python3 -I "$PROBES" "$T/k.png")
echo "$out" | grep -q "black yes" && ok "black window: black yes" || bad "black window: $out"
out=$(python3 -I "$PROBES" "$T/a.png")
echo "$out" | grep -q "black no" && ok "picture: black no" || bad "picture: $out"
# a dark-grey camera black (cast 6) is still black
gen "$T/k2.png" 1280 720 0 6,6,6 1
python3 -I "$PROBES" "$T/k2.png" | grep -q "black yes" \
    && ok "black with a dark-grey cast: black yes" || bad "black with a cast"

# compare: near, a slight cast, far (swapped), black where the reference is not
cmp() { python3 -I "$PROBES" "$1" --compare "$T/pictures.txt" imx8mp "$2" "${@:3}"; }
cmp "$T/a.png" "[test] scene=ref" >"$T/o"; rc=$?
[ $rc -eq 0 ] && [ "$(grep -c ': near' "$T/o")" = 5 ] && ok "compare exact: exit 0, 5 near" || bad "compare exact rc=$rc"
gen "$T/cast.png" 1280 720 0 20,-15,10 0
cmp "$T/cast.png" "[test] scene=ref" >"$T/o"; rc=$?
[ $rc -eq 0 ] && ok "compare with a slight colour cast: exit 0" || bad "compare cast rc=$rc"
cmp "$T/a.png" "[test] scene=swap" >"$T/o"; rc=$?
[ $rc -eq 1 ] && grep -q ': far' "$T/o" && ok "compare swapped: exit 1, far" || bad "compare swapped rc=$rc"
cmp "$T/k.png" "[test] scene=ref" >"$T/o"; rc=$?
[ $rc -eq 1 ] && grep -q 'black where the reference is not' "$T/o" \
    && ok "compare black window: exit 1, black where the reference is not" || bad "compare black rc=$rc"
cmp "$T/k.png" "[test] scene=dark" >"$T/o"; rc=$?
[ $rc -eq 0 ] && ok "compare black vs a near-black reference: exit 0" || bad "compare dark rc=$rc"
cmp "$T/cast.png" "[test] scene=ref" --tolerance 5 >"$T/o"; rc=$?
[ $rc -eq 1 ] && ok "--tolerance 5 turns the cast into far" || bad "tolerance rc=$rc"

# bad input: exit 2
bad2() { "$@" >/dev/null 2>&1; [ $? -eq 2 ] && ok "exit 2: $NAME" || bad "exit 2: $NAME"; }
NAME="missing file"; bad2 python3 -I "$PROBES" "$T/none.png"
printf 'not a png' > "$T/junk.png"
NAME="not a PNG"; bad2 python3 -I "$PROBES" "$T/junk.png"
NAME="bad rotation"; bad2 python3 -I "$PROBES" "$T/a.png" --rotation 45
NAME="unknown scene"; bad2 python3 -I "$PROBES" "$T/a.png" --compare "$T/pictures.txt" imx8mp nosuch
NAME="unknown column"; bad2 python3 -I "$PROBES" "$T/a.png" --compare "$T/pictures.txt" nocol "[test] scene=ref"
NAME="window outside panel"; bad2 python3 -I "$PROBES" "$T/a.png" --window 700,0,800,600
NAME="no screenshot"; bad2 python3 -I "$PROBES"

if [ "$fail" -ne 0 ]; then echo "glmark2-photo-probes-selftest: $fail FAILED"; exit 1; fi
echo "glmark2-photo-probes-selftest: all ok"
