#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/glmark2-photo-tolerance-selftest.sh
#
# Host-only: tools/cts/glmark2-photo-tolerance.py on synthetic 800x600 frames
# generated under $TMPDIR: a flat frame (geometry drift at or below 2 levels, no
# edge flag, total drift bounded by the colour model), a frame with a sharp
# vertical edge through the tl probe (tl and bl, both at x=200, flagged EDGE; c, tr, br not), the
# --distinct question for 000000 / 181c18, and bad inputs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-.}/photo-tol.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
TOOL="$HERE/glmark2-photo-tolerance.py"
fail=0
ok() { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=$((fail + 1)); }

cat > "$T/gen.py" <<'PY'
import struct, sys, zlib
# gen.py <out> <kind: flat|edge|small> [w h]
out, kind = sys.argv[1], sys.argv[2]
W, H = (int(sys.argv[3]), int(sys.argv[4])) if len(sys.argv) > 4 else (800, 600)
def px(x, y):
    if kind == "flat":
        return (120, 130, 140)
    if kind == "edge":  # dark | bright, the edge between x=199 and x=200 = the tl probe
        return (32, 32, 32) if x < 200 else (224, 224, 224)
    return (0, 0, 0)
rows = b"".join(b"\0" + bytes(v for x in range(W) for v in px(x, y)) for y in range(H))
def chunk(t, b):
    return struct.pack(">I", len(b)) + t + b + struct.pack(">I", zlib.crc32(t + b) & 0xFFFFFFFF)
with open(out, "wb") as f:
    f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 2, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))
PY

mkdir "$T/frames" || exit 2
python3 "$T/gen.py" "$T/frames/flat.png" flat || exit 2
python3 "$T/gen.py" "$T/frames/edge.png" edge || exit 2

python3 "$TOOL" --random 1 "$T/frames/flat.png" "$T/frames/edge.png" > "$T/out.txt" 2> "$T/err.txt"
rc=$?
[ "$rc" = 0 ] && ok "runs on flat and edge frames" || { bad "exit $rc: $(cat "$T/err.txt")"; }

# columns: scene probe fb geo all sens [EDGE]
flat_geo=$(awk '$1=="flat" && NF>=6 && $2!="avg" {if ($4>m) m=$4} END {print m+0}' "$T/out.txt")
flat_all=$(awk '$1=="flat" && NF>=6 && $2!="avg" {if ($5>m) m=$5} END {print m+0}' "$T/out.txt")
flat_flag=$(awk '$1=="flat" && $NF=="EDGE"' "$T/out.txt" | wc -l)
[ "$flat_geo" -le 2 ] && ok "flat: geometry drift $flat_geo <= 2" || bad "flat: geometry drift $flat_geo > 2"
[ "$flat_all" -le 45 ] && [ "$flat_all" -ge 10 ] && ok "flat: total drift $flat_all within the colour model (10..45)" \
    || bad "flat: total drift $flat_all outside 10..45"
[ "$flat_flag" = 0 ] && ok "flat: no probe flagged" || bad "flat: $flat_flag probes flagged"

edge_tl=$(awk '$1=="edge" && $2=="tl" && $NF=="EDGE"' "$T/out.txt" | wc -l)
edge_other=$(awk '$1=="edge" && $2!="tl" && $2!="bl" && $NF=="EDGE"' "$T/out.txt" | wc -l)
edge_geo=$(awk '$1=="edge" && $2=="tl" {print $4}' "$T/out.txt")
[ "$edge_tl" = 1 ] && ok "edge: the tl probe on the sharp edge is flagged" || bad "edge: tl not flagged"
[ "$edge_other" = 0 ] && ok "edge: the probes off the edge (c, tr, br) are not flagged" || bad "edge: $edge_other other probes flagged"
[ "${edge_geo:-0}" -ge 60 ] && ok "edge: tl geometry drift $edge_geo >= 60" || bad "edge: tl geometry drift ${edge_geo:-?} < 60"
grep -q '^# EDGE-flagged probes.*edge tl' "$T/out.txt" && ok "edge: listed in the summary line" \
    || bad "edge: summary line lacks 'edge tl'"

# a directory argument takes every png in it; --csv writes a row per probe
python3 "$TOOL" --random 0 --csv "$T/o.csv" "$T/frames" > "$T/out2.txt" 2>&1
[ "$(wc -l < "$T/o.csv")" = 11 ] && ok "directory + --csv: 10 rows and a header" || bad "csv has $(wc -l < "$T/o.csv") lines"

# can a photo tell 000000 from 181c18 (24 levels apart)?
python3 "$TOOL" --random 2 --distinct 000000 181c18 > "$T/dist.txt" 2>&1
rc=$?
if [ "$rc" = 0 ] && grep -q 'ranges disjoint on a channel' "$T/dist.txt" && grep -q 'reader says black in' "$T/dist.txt"; then
    ok "--distinct 000000 181c18 answers"
else
    bad "--distinct failed (exit $rc)"
fi

# bad inputs
python3 "$TOOL" "$T/missing.png" > /dev/null 2>&1; [ $? = 2 ] && ok "missing frame: exit 2" || bad "missing frame"
python3 "$T/gen.py" "$T/small.png" small 64 48 && python3 "$TOOL" "$T/small.png" > /dev/null 2>&1
[ $? = 2 ] && ok "wrong-size frame: exit 2" || bad "wrong-size frame"
python3 "$TOOL" --bogus "$T/frames/flat.png" > /dev/null 2>&1; [ $? = 2 ] && ok "unknown option: exit 2" || bad "unknown option"
python3 "$TOOL" > /dev/null 2>&1; [ $? = 2 ] && ok "no frames: exit 2" || bad "no frames"

# no host, home or user names in the new files
leak=$(grep -n -E '/home/|/dev/tty|\b[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}\b' "$TOOL" | wc -l)
[ "$leak" = 0 ] && ok "no path or address in the tool" || bad "path or address in the tool"

if [ "$fail" = 0 ]; then echo "all ok"; exit 0; fi
echo "$fail failed"; exit 1
