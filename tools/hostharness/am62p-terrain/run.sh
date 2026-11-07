#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# glmark2 terrain on the PowerVR host build, the three runs of
# docs/notes/am62p-slow-terrain.md: the profile (GM2_PROFILE), perf
# (GM2_PERF) and the per-draw kick log (GM2_KICKS), each through the glmark2
# harness unchanged (tools/hostharness/glmark2/run.sh), then the per-frame
# kick and draw counts of the kick log (kicks.py).
#
# usage: run.sh <GLMARK2_SRC> [out] [profile|perf|kicks...]
#   out   work directory (default: build/am62p-terrain); each run gets its
#         own subdirectory, out/prof, out/perf, out/kicks
#   runs  default: all three
# environment: GM2_FRAMES (default 20), as the harness's.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
[ $# -ge 1 ] || { sed -n '3,16p' "$0"; exit 2; }
SRC="$1"
OUT="${2:-$ROOT/build/am62p-terrain}"
shift $(( $# >= 2 ? 2 : 1 ))
RUNS=("$@")
[ ${#RUNS[@]} -gt 0 ] || RUNS=(profile perf kicks)
export GM2_WRITE=0 GM2_ENTRIES='^terrain$' GM2_FRAMES="${GM2_FRAMES:-20}"
mkdir -p "$OUT"
for r in "${RUNS[@]}"; do
  case "$r" in
    profile) GM2_PROFILE=1 "$ROOT/tools/hostharness/glmark2/run.sh" "$SRC" "$OUT/prof" powervr ;;
    perf) GM2_PERF=1 "$ROOT/tools/hostharness/glmark2/run.sh" "$SRC" "$OUT/perf" powervr ;;
    kicks)
      GM2_KICKS=1 "$ROOT/tools/hostharness/glmark2/run.sh" "$SRC" "$OUT/kicks" powervr
      python3 -I "$HERE/kicks.py" "$OUT/kicks/logs/powervr/0.log" | tee "$OUT/kicks/kicks.txt"
      ;;
    *) echo "am62p-terrain/run.sh: unknown run '$r' (profile, perf, kicks)" >&2; exit 1 ;;
  esac
done
