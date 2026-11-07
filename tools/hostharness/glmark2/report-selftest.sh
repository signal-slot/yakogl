#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   bash tools/hostharness/glmark2/report-selftest.sh
#
# report.py --profile on the fixture run in testdata/draw-steps/out (the
# files a GM2_PROFILE run leaves, with made-up times: a V3D log with a
# "gm2: draw-steps" line, a PowerVR log whose clock was refused) against the
# profile-<gpu>.md it must write. To change the expected files on purpose,
# copy out/ to a scratch directory, rerun
#   python3 -I report.py --profile --commit fixture --size 800x600 --frames 11 <copy> v3d powervr
# and copy <copy>/profile-*.md over expected-profile-*.md after reviewing.
# Host only: python3 and diff.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
D=$HERE/testdata/draw-steps
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

W=$(mktemp -d)
trap 'rm -rf "$W"' EXIT
cp -r "$D/out" "$W/out"
if ! python3 -I "$HERE/report.py" --profile --commit fixture --size 800x600 --frames 11 "$W/out" v3d powervr \
	>"$W/report.txt" 2>&1; then
	bad "report.py failed"
	head -20 "$W/report.txt"
fi

for g in v3d powervr; do
	if diff "$W/out/profile-$g.md" "$D/expected-profile-$g.md" >/dev/null 2>&1; then
		ok "profile-$g.md (per-step us a draw)"
	else
		bad "profile-$g.md differs from expected-profile-$g.md"
		diff "$W/out/profile-$g.md" "$D/expected-profile-$g.md" 2>&1 | head -20
	fi
done

# a run without the line (an older log) prints no draw-steps section
sed -i '/^gm2: draw-steps /d' "$W/out/logs/v3d/0.log"
python3 -I "$HERE/report.py" --profile --commit fixture --size 800x600 --frames 11 "$W/out" v3d >/dev/null 2>&1
if grep -q 'Draw steps\|No draw reported' "$W/out/profile-v3d.md"; then
	bad "a log without a draw-steps line got a section"
else
	ok "no draw-steps line, no section"
fi

# a run without gc-cost lines (every run without GM2_GCCOST=1) writes no
# gc-cost file
if ls "$W"/out/gc-cost-*.md >/dev/null 2>&1; then
	bad "a run without gc-cost lines got a gc-cost file"
else
	ok "no gc-cost lines, no gc-cost file"
fi

# GM2_GCCOST=1: the fixture run in testdata/gc-cost/out (made-up counts: a
# lagging-GPU terrain with an undecoded kick and a scene-info prefix, a
# shadow of the done model, a refract without the lines) against the
# gc-cost-vivante.md it must write. To change it on purpose, rerun
#   python3 -I report.py --commit fixture --size 800x600 --frames 3 <copy> vivante
# on a copy of out/ and copy <copy>/gc-cost-vivante.md over
# expected-gc-cost-vivante.md after reviewing.
G=$HERE/testdata/gc-cost
cp -r "$G/out" "$W/gc"
if ! python3 -I "$HERE/report.py" --commit fixture --size 800x600 --frames 3 "$W/gc" vivante \
	>"$W/gc-report.txt" 2>&1; then
	bad "report.py failed on the gc-cost fixture"
	head -20 "$W/gc-report.txt"
fi
if diff "$W/gc/gc-cost-vivante.md" "$G/expected-gc-cost-vivante.md" >/dev/null 2>&1; then
	ok "gc-cost-vivante.md (waits, cache bytes, kicks and targets a frame)"
else
	bad "gc-cost-vivante.md differs from expected-gc-cost-vivante.md"
	diff "$W/gc/gc-cost-vivante.md" "$G/expected-gc-cost-vivante.md" 2>&1 | head -20
fi
# shim.c's gc lines are records, not findings
if grep -q '`gm2: gc-' "$W/gc-report.txt"; then
	bad "a gc-cost line was listed as a finding"
else
	ok "gc-cost lines are not findings"
fi

if [ "$fail" -ne 0 ]; then
	echo "report-selftest: $fail failed" >&2
	exit 1
fi
echo "report-selftest: ok"
