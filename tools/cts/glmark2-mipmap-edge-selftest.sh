#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of tools/cts/glmark2-mipmap-edge.py on synthetic 800x600 PNGs
# written under $TMPDIR (no board, no network). Exit 0: every case gave the
# expected exit code and verdict.
set -u
here=$(cd "$(dirname "$0")" && pwd)
tool="$here/glmark2-mipmap-edge.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mipmap-edge.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT

python3 -I - "$here" "$tmp" <<'PY' || exit 2
import sys

sys.path.insert(0, sys.argv[1])
import importlib.util
spec = importlib.util.spec_from_file_location("fb", sys.argv[1] + "/glmark2-fb.py")
fb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fb)

TEX = (120, 140, 100)


def make(name, bg, e150, e450, rim=0, black_tl=False, edge=True):
    """Left edge at e150 on rows 100..299 and e450 on rows 300..599 (a straight
    edge per half), aliased; rim: a black run of that many
    whole pixels between background and texture (integer edges only)."""
    rows = []
    for y in range(600):
        e = e150 if y < 300 else e450
        r = bytearray()
        for x in range(800):
            # The GPU does not anti-alias: a pixel is drawn when its centre
            # is inside the edge.
            if not edge or y < 100 or x + 0.5 < e:
                p = bg
            elif x < e + rim:
                p = (0, 0, 0)
            else:
                p = TEX
            r += bytes(p)
        if black_tl and y == 150:
            r[200 * 3:200 * 3 + 3] = b"\0\0\0"
        rows.append(bytes(r))
    fb.write_png(sys.argv[2] + "/" + name + ".png", 800, 600, rows)


k150, k450 = 0.0353, -0.212
make("d0", (0, 0, 0), 200.0, 200.0)
make("d20", (0, 0, 0), 200 + k150 * 20, 200 + k450 * 20)
make("black-tl", (30, 30, 30), 190.0, 190.0, black_tl=True)
make("rim", (30, 30, 30), 200.0, 196.0, rim=2)
make("noedge", (30, 30, 30), 0, 0, edge=False)
make("disagree", (0, 0, 0), 200 + k150 * 20, 200.0)
PY

fail=0
# case name, expected exit, expected text in the verdict line
check() {
    local name=$1 want=$2 text=$3 out rc
    out=$(python3 -I "$tool" "$tmp/$name.png" 2>&1); rc=$?
    if [ "$rc" != "$want" ] || ! grep -qF -- "$text" <<<"$out" ||
        ! grep -q '^mipmap-edge: ' <<<"$out"; then
        echo "FAIL $name: exit $rc (want $want), want '$text'"
        echo "$out" | sed 's/^/    /'
        fail=1
    else
        echo "ok   $name: exit $rc: $(grep '^mipmap-edge: ' <<<"$out")"
    fi
}

check d20 0 "tl uncovered 000000; rim none; timing explains tl: yes"
check black-tl 1 "tl covered 000000; rim none; timing explains tl: no"
check rim 1 "rim black; timing explains tl: no"
check noedge 2 "d=n/a (row150) d=n/a (row450);"
check disagree 2 "timing explains tl: inconclusive"

# d=0: tl is covered by the texture and not black: nothing for timing to explain.
out=$(python3 -I "$tool" "$tmp/d0.png" 2>&1); rc=$?
if [ "$rc" = 2 ] && grep -qF "tl covered 788c64; rim none; timing explains tl: inconclusive" <<<"$out"; then
    echo "ok   d0 (again): exit 2, tl covered 788c64, inconclusive"
else
    echo "FAIL d0: exit $rc"; echo "$out" | sed 's/^/    /'; fail=1
fi

# Unreadable inputs end in exit 2 without a verdict.
echo "not a picture" >"$tmp/junk"
python3 -I "$tool" "$tmp/junk" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && echo "ok   junk: exit 2" || { echo "FAIL junk: exit $rc"; fail=1; }
python3 -I "$tool" >/dev/null 2>&1; rc=$?
[ "$rc" = 2 ] && echo "ok   no argument: exit 2" || { echo "FAIL no argument: exit $rc"; fail=1; }

[ "$fail" = 0 ] && echo "glmark2-mipmap-edge-selftest: ok" || echo "glmark2-mipmap-edge-selftest: FAILED"
exit "$fail"
