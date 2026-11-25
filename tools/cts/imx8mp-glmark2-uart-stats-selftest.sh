#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/imx8mp-glmark2-uart-stats-selftest.sh
#
# Self-test of imx8mp-glmark2-uart-stats.py on tiny synthetic console.ts
# captures (an 8x6 frame in glmark2-fb.py's line format, generated under
# $TMPDIR): a whole dump, one with a lost row, one with a corrupted row, one
# whose end line lies past the watchdog window, one without an end line, and a
# record dir. Host only.
set -u
here=$(cd "$(dirname "$0")" && pwd)
base=${TMPDIR:-}
if [ -z "$base" ]; then echo "TMPDIR is not set" >&2; exit 2; fi
work=$(mktemp -d "$base/imx8mp-uartstats.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
fail=0

cat >"$work/gen.py" <<'PY'
import base64, binascii, importlib.util, os, sys
here, out = sys.argv[1], sys.argv[2]
spec = importlib.util.spec_from_file_location("fb", os.path.join(here, "glmark2-fb.py"))
fb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(fb)
W, H = 8, 6
rows = [bytes((i * 7 + y * 40) & 255 for i in range(W * 3)) for y in range(H)]


def make(name, drop=None, corrupt=None, end_at=170.0, noend=False):
    t0 = 1000000.0
    ls = [(t0, "U-Boot SPL"), (t0 + 5, "*** Booting Zephyr OS build v4.x ***"), (t0 + 6, "gpu: init")]
    ls.append((t0 + 100.0, "glmark2-fb: s1 %dx%d begin" % (W, H)))
    for y, r in enumerate(rows):
        t = t0 + 100.0 + (end_at - 101.0) * (y + 1) / H
        if y == drop:
            continue
        b = base64.b64encode(r).decode()
        if y == corrupt:
            b = b[:3] + ("B" if b[3] != "B" else "C") + b[4:]
        ls.append((t, "glmark2-fb: %d %04x %s" % (y, fb.crc16(r), b)))
    if not noend:
        ls.append((t0 + end_at, "glmark2-fb: end crc32 %08x" % (binascii.crc32(b"".join(rows)) & 0xFFFFFFFF)))
    with open(os.path.join(out, name), "w", newline="") as f:
        f.write("".join("%.3f %s\r\n" % x for x in ls))


make("whole.ts")
make("lost.ts", drop=2)
make("corrupt.ts", corrupt=3)
make("over.ts", end_at=640.0)
make("noend.ts", noend=True)
PY
python3 -I "$work/gen.py" "$here" "$work" || { echo "FAIL generating"; exit 1; }

stats() { python3 -I "$here/imx8mp-glmark2-uart-stats.py" "$@"; }
check() { if [ "$2" = 0 ]; then echo "ok   $1"; else echo "FAIL $1"; fail=$((fail + 1)); fi; }

stats --watchdog-s 600 "$work/whole.ts" >"$work/a.out" 2>&1
check "whole dump exits 0" $?
grep -q 'damaged 0 missing 0' "$work/a.out" && grep -q 'boot-to-end 165.0s margin 435.0s -> ok' "$work/a.out" &&
    grep -q 'duration 70.0s' "$work/a.out"
check "whole dump: duration, no damage, boot-to-end and margin" $?

stats --watchdog-s 600 "$work/lost.ts" >"$work/b.out" 2>&1; rc=$?
[ $rc = 1 ]; check "lost row exits 1" $?
grep -q 'missing 1' "$work/b.out" && grep -q 're-run' "$work/b.out"
check "lost row: counted and re-run" $?

stats --watchdog-s 600 "$work/corrupt.ts" >"$work/c.out" 2>&1; rc=$?
[ $rc = 1 ]; check "corrupt row exits 1" $?
grep -q 'damaged 1 missing 0' "$work/c.out" && grep -q 're-run' "$work/c.out"
check "corrupt row: counted and re-run" $?

stats --watchdog-s 600 "$work/over.ts" >"$work/d.out" 2>&1; rc=$?
[ $rc = 1 ]; check "over-window dump exits 1" $?
grep -q 'margin -35.0s' "$work/d.out" && grep -q 're-run (over the watchdog window' "$work/d.out"
check "over-window: negative margin, re-run" $?
stats --watchdog-s 700 "$work/over.ts" >/dev/null 2>&1
check "same capture inside a 700 s window exits 0" $?

stats --watchdog-s 600 "$work/noend.ts" >"$work/e.out" 2>&1; rc=$?
[ $rc = 1 ]; check "missing end line exits 1" $?
grep -q 'end line missing' "$work/e.out"; check "missing end line named" $?

mkdir "$work/rec" && cp "$work/whole.ts" "$work/rec/console.ts"
stats "$work/rec" >"$work/f.out" 2>&1; check "record dir, window from the dumps script" $?
stats "$work/nothing" >/dev/null 2>&1; [ $? = 2 ]; check "missing path exits 2" $?

if [ $fail = 0 ]; then echo "uart-stats self-test: all ok"; else echo "uart-stats self-test: $fail FAIL"; fi
[ $fail = 0 ]
