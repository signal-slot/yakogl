#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The four host runs of docs/notes/pi4-slow-terrain.md: glmark2 terrain on
# the V3D build, 20 frames, GM2_PROFILE=1 and GM2_PERF=1, each with the
# host's TFU model (YAKOGL_V3D_TFU=1, the Pi 4 glmark2 image's setting) and
# without it (YAKOGL_V3D_TFU=0, the CPU path). It only calls
# tools/hostharness/glmark2/run.sh with those settings, one after the other,
# then summarize.py over each run's log.
#
# usage: run.sh <GLMARK2_SRC> [out]   (out: build/pi4-terrain)
#   PI4T_RUNS="prof-tfu1 prof-tfu0 perf-tfu1 perf-tfu0" (default: all four)
set -euo pipefail

src=${1:?usage: run.sh <GLMARK2_SRC> [out]}
out=${2:-build/pi4-terrain}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
runs=${PI4T_RUNS:-prof-tfu1 prof-tfu0 perf-tfu1 perf-tfu0}

mkdir -p "$out"
for r in $runs; do
  case $r in
    prof-tfu1) mode=GM2_PROFILE tfu=1 ;;
    prof-tfu0) mode=GM2_PROFILE tfu=0 ;;
    perf-tfu1) mode=GM2_PERF tfu=1 ;;
    perf-tfu0) mode=GM2_PERF tfu=0 ;;
    *) echo "run.sh: unknown run $r" >&2; exit 2 ;;
  esac
  echo "== $r: $mode=1 YAKOGL_V3D_TFU=$tfu" >&2
  env GM2_WRITE=0 "$mode=1" GM2_FRAMES=20 GM2_ENTRIES='^terrain$' \
      YAKOGL_V3D_TFU=$tfu \
      "$root/tools/hostharness/glmark2/run.sh" "$src" "$out/$r" v3d \
      >"$out/$r.log" 2>&1 || { echo "run.sh: $r failed, see $out/$r.log" >&2; exit 1; }
  # a GM2_PERF run whose perf could not open its events has no profile
  # lines (perf_event_paranoid); say so and go on with the other runs
  if python3 -I "$here/summarize.py" "$out/$r" >"$out/$r.summary.txt"; then
    cat "$out/$r.summary.txt"
  else
    echo "run.sh: $r has no profile lines; see $out/$r/logs/v3d/0.log" >&2
  fi
done
