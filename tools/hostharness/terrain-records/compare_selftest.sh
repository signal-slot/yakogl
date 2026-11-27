#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   bash tools/hostharness/terrain-records/compare_selftest.sh
#
# compare.py on two synthetic records written here (made-up times, no board
# data). Host only: python3.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/terrain-compare.XXXXXX") || exit 1
trap 'rm -rf "$T"' EXIT
fail=0

mk() { # <dir> <noise render> <terrain render> <mip.queued> <wait.ns>
	mkdir -p "$1"
	cat > "$1/console.txt" <<EOT
12:00:00 sample: build tree=$6 table=v3d
12:00:09 [terrain] <default>: FPS: 30 FrameTime: 33.300 ms
12:00:09 glmark2-frame: [terrain] <default>: 61 frames; us per frame: gl 30000, swap 2000, show 320, other 300, frame 33300 (end waited for in 61 of 61)
12:00:09 glmark2-counters: [terrain] <default>: frames 10 jobs 80 mip.queued $4 wait.ns $5
12:00:09 glmark2-jobs: [terrain] <default>: frame 10: 8 job(s): total 4500 render 4400~ idle 0 bin 40 prep 5; total $2 render $2 idle 80 bin 9 prep 1; total 200 render 190 idle 0 bin 6 prep 1; total 9500 render $3 idle 0 tfu 1/800us bin 3500 prep 20; total 470 render 460 idle 0 tfu 1/110us bin 7 prep 1; total 300 render 290 idle 0 tfu 1/110us bin 7 prep 1; total 1800 render 1750 idle 0 tfu 1/790us bin 36 prep 1; total 2900 render 2800 idle 0 tfu 1/800us bin 36 prep 1
EOT
}
mk "$T/a" 8000 9400 20 28000000 aaaaaaaa
mk "$T/b" 5000 7000 0 21000000 bbbbbbbb

out=$(python3 -I "$HERE/compare.py" --counters mip.queued,jobs "$T/a" "$T/b" 2>&1)
rc=$?
chk() { # <fixed string that must be in the output>
	if printf '%s\n' "$out" | grep -qF -- "$1"; then echo "  ok    $1"
	else echo "  FAIL  missing: $1"; fail=$((fail + 1)); fi
}
[ "$rc" -eq 0 ] || { echo "  FAIL  rc $rc"; fail=$((fail + 1)); }
chk '| 2 | noise (height map) | 8000 | 5000 | -3000 |'
chk '| 4 | terrain draw | 9400 | 7000 | -2400 |'
chk '| 5 | bloom H (blur) | 460 | 460 | +0 |'
chk '| entries with a TFU | 5 | 5 | +0 |'
chk '| TFU us | 2610 | 2610 | +0 |'
chk '| frame | 33300 | 33300 | +0 |'
chk '| wait.ns | 2.800 | 2.100 | -0.700 |'
chk '| mip.queued | 2.000 | 0.000 | -2.000 |'
chk '- a: commit aaaaaaaa, 33.3 ms a frame'
python3 -I "$HERE/compare.py" "$T/a" >/dev/null 2>&1
[ "$?" -eq 2 ] && echo "  ok    one record refused" || { echo "  FAIL  one record"; fail=$((fail + 1)); }
[ "$fail" -eq 0 ] && echo "compare_selftest ok" && exit 0
echo "compare_selftest FAILED: $fail"
exit 1
