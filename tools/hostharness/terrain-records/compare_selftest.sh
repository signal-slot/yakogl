#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Runs compare.py on synthetic console logs written here (no board data).
#   sh tools/hostharness/terrain-records/compare_selftest.sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${TMPDIR:-.}/compare-selftest.XXXXXX")
trap 'rm -rf "$t"' EXIT

# eight entries: noise is 2, terrain is 4 (before the tfu 1/806us entry 5), blurs 8 and 1
mk() { # file noise_render terrain_render wait_ns mip_queued
cat > "$1" <<EOT
12:00:00 sample: build tree=$6
12:00:09 [terrain] <default>: FPS: 28 FrameTime: $7 ms
12:00:09 glmark2-frame: [terrain] <default>: 61 frames; us per frame: gl 26000, swap 500, show 10, other 20, frame 34700 (end waited for in 61 of 61)
12:00:09 glmark2-counters: [terrain] <default>: frames 61 jobs 549 waits 61 wait.ns $4 mip.queued $5
12:00:09 glmark2-jobs: [terrain] <default>: frame 10: 8 job(s): total 5000 render 4436~ idle 14 tfu 1/804us bin 100 prep 5; total 8000 render $2 idle 80 bin 120 prep 1; total 200 render 191 idle 0 bin 10 prep 1; total 12000 render $3 idle 0 bin 3529 prep 2; total 900 render 800 idle 0 tfu 1/806us bin 90 prep 1; total 900 render 800 idle 0 tfu 1/114us bin 90 prep 1; total 1800 render 1754 idle 0 tfu 1/114us bin 90 prep 1; total 2900 render 2801 idle 0 tfu 1/804us bin 90 prep 1
EOT
}
mk "$t/a.txt" 7931 9374 1730000000 304 aaaaaaaa 34.7
mk "$t/b.txt" 5000 7000 1280000000 244 bbbbbbbb 30.1

out=$(python3 -I "$here/compare.py" "$t/a.txt" "$t/b.txt")
echo "$out"
ck() { printf '%s\n' "$out" | grep -F -- "$1" >/dev/null || { echo "selftest FAILED: missing: $1" >&2; exit 1; }; }
ck '| entry 2 (noise) render us | 7931 | 5000 | -2931 |'
ck '| entry 4 (terrain) render us | 9374 | 7000 | -2374 |'
ck '| entry 4 (terrain) bin us | 3529 | 3529 | +0 |'
ck '| entry 8 (blur) render us | 2801 | 2801 | +0 |'
ck '| entry 1 (blur) idle us | 14 | 14 | +0 |'
ck '| TFU entries (count) | 5 | 5 | +0 |'
ck '| TFU entries (sum us) | 2642 | 2642 | +0 |'
ck '| FrameTime ms | 34.700 | 30.100 | -4.600 |'
ck '| wait.ns ms a frame |'
ck '| counter mip.queued (a frame) | 4.984 | 4 | -0.984 |'
ck 'aaaaaaaa'

# three records and a chosen counter
out=$(python3 -I "$here/compare.py" --counter jobs "$t/a.txt" "$t/b.txt" "$t/a.txt")
ck '| counter jobs (a frame) | 9 | 9 | +0 | 9 | +0 |'

python3 -I "$here/compare.py" --help | grep -F -- '--counter' >/dev/null
python3 -I "$here/compare.py" --help | grep -F -- '--scene' >/dev/null
if python3 -I "$here/compare.py" "$t/a.txt" >/dev/null 2>&1; then echo "selftest FAILED: one record accepted" >&2; exit 1; fi
echo "compare selftest ok"
