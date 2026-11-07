#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# glmark2's default list entries FIRST..LAST in ONE host process, as the
# Pi 4's GM image runs the list (tools/hostharness/glmark2/run.sh runs one
# process per entry, so a scene there never inherits the V3D back end's
# state of the scenes before it). Uses a host binary run.sh built
# (<run.sh out>/glmark2-host-v3d) and the V3D settings it used
# (v3d-settings.txt); prints the back end's per-frame counter lines (the
# "at frame N" lines, printed when an arena high-water grows) with the
# scene-begin/end lines between them, so a scene's counts are the
# difference of two lines, as on the board (docs/notes/pi4-glmark2-cpu.md,
# "On the board: glmark2 from main c4af4917").
#
# usage: one-process.sh [--profile] <run.sh out> <FIRST> <LAST> [frames] [last frames] > log
#   FIRST, LAST  0-based indices into <run.sh out>/glmark2-host-v3d --gm2-list
#   frames       frames per entry (default 20)
#   last frames  frames of entry LAST (default: frames)
#   --profile (or PI4T_PROFILE=1)  also time every GL/EGL call (shim.c's GM2_PROFILE) over
#                the whole process after its first swap and print the
#                "gm2: gl-time", "gl-frames", "v3d-frames", "v3d-waits" and
#                "gpu-time" lines; two runs that differ only in "last frames"
#                give, by difference (framediff.py), entry LAST's own frames
set -euo pipefail

if [ "${1:-}" = --profile ]; then
  PI4T_PROFILE=1
  shift
fi
out=$(cd "${1:?usage: one-process.sh <run.sh out> <FIRST> <LAST> [frames] [last frames]}" && pwd)
first=${2:?FIRST}
last=${3:?LAST}
frames=${4:-20}
last_frames=${5:-$frames}
bin="$out/glmark2-host-v3d"
[ -x "$bin" ] || { echo "one-process.sh: no $bin (run tools/hostharness/glmark2/run.sh ... v3d first)" >&2; exit 2; }

mapfile -t entries < <("$bin" --gm2-list)
args=(--size 800x600 -b ":nframes=$frames")
for ((i = first; i < last; i++)); do args+=(-b "${entries[$i]}"); done
# the last entry with its own frame count (glmark2's per-benchmark option)
args+=(-b "${entries[$last]}:nframes=$last_frames")
echo "one-process: entries $first..$last of ${#entries[@]}, $frames frames each, $last_frames for the last:" \
  "${entries[*]:$first:$((last - first + 1))}"

# the settings run.sh used for this binary (the Pi 4 image's), unless set
while IFS== read -r k v; do
  [ -n "$k" ] && [ -z "${!k:-}" ] && export "$k=$v"
done < "$out/v3d-settings.txt"

keep='gm2: scene-(begin|end)|at frame [0-9]+[:,]|FrameTime|render image|TFU made|out of GPU memory|no GPU memory'
if [ "${PI4T_PROFILE:-0}" = 1 ]; then
  export GM2_PROFILE=1 GM2_FRAMES=1000000
  keep="$keep|gm2: (gl-time|gl-frames|v3d-frames|v3d-waits|gpu-time|draw-steps)"
fi
scratch="$out/one-process"
mkdir -p "$scratch"
( cd "$scratch" && YAKOGL_ERROR_LOG=1000000 timeout "${GM2_TIMEOUT:-1200}" "$bin" "${args[@]}" ) 2>&1 |
  grep -E --line-buffered "$keep"
