#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Overdraw of glmark2 terrain's terrain draw, counted on the host (README.md).
#   tools/hostharness/terrain-overdraw/run.sh [out dir] [seconds ...]
# Output dir default: $TMPDIR/terrain-overdraw (else /tmp is NOT used: set it).
# Default frames: glmark2's elapsed time 0 1 2.5 5 7.5 10 s.
# Needs g++ only; no GL, no glmark2 checkout, no network.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-${TMPDIR:?set TMPDIR or pass an output directory}/terrain-overdraw}
[ $# -gt 0 ] && shift
mkdir -p "$out"
g++ -O2 -Wall -o "$out/overdraw" "$here/overdraw.cpp"
[ $# -gt 0 ] || set -- 0 1 2.5 5 7.5 10
"$out/overdraw" "$@" | tee "$out/overdraw.txt"
OVERDRAW_FLOAT_DEPTH=1 "$out/overdraw" "$@" > "$out/overdraw-float-depth.txt"
awk '{ for (i = 1; i <= NF; i++) { split($i, kv, "="); v[kv[1]] = kv[2] }
       o += v["overdraw"]; p += v["passed_per_covered"]; n++ }
     END { printf "mean over %d frames: overdraw=%.4f passed_per_covered=%.4f\n", n, o / n, p / n }' \
  "$out/overdraw.txt" | tee "$out/summary.txt"
echo "float-depth variant: $out/overdraw-float-depth.txt"
