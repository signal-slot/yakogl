#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Run one yakogl-mali-panthor binary on the board in several modes, each a
# number of times, one run-on-board.sh session per run (so each run takes
# the board lock and the render-node check of its own), into a record
# directory:
#
#   BOARD_HOST=root@<t6> tools/mali/panthor/run-modes.sh <outdir> <binary> <reps> <mode>...
#
# A mode is the binary's argument list as one word ("-" for none, "--depth",
# "--sweep"). Each run writes <outdir>/<NN>-<binary><mode>-<rep>.log and its
# dmesg capture .dmesg, and appends "<name> rc=<rc> <time>" to
# <outdir>/runs.txt (the time from `date`). NN continues from the runs
# already listed there.
#
# A capture begins with the kernel ring's boot history, which holds the
# board's MAC addresses and nothing from the runs: each .dmesg is cut to
# start at the first run marker of this invocation's first run, and the
# script refuses to leave a capture that still holds a MAC address. It
# exits 1 when a run's rc was not 0 (the runs after it still run).
#
# RETRY_BUSY=<s> (optional): a run that did not start because another
# session holds the board (the board lock's refusal, or a process on the
# render node) is not a run: its log and capture are dropped, nothing is
# listed, and it is tried again after <s> seconds, at most RETRY_MAX times
# (default 60). The lock is never waited on (LOCK_WAIT stays 0). Without
# it such a refusal is a run with rc 1 or 3, as before.
set -euo pipefail
out=${1:?usage: BOARD_HOST=<user@host> $0 <outdir> <binary> <reps> <mode>...}
bin=${2:?binary}
reps=${3:?reps}
shift 3
[ $# -gt 0 ] || { echo "run-modes.sh: no mode given" >&2; exit 2; }
mkdir -p "$out"
out=$(realpath "$out")
cd "$(dirname "$0")/../../.."
n=1
[ -f "$out/runs.txt" ] && n=$(($(wc -l < "$out/runs.txt") + 1))
first_marker=
failed=0
for mode in "$@"; do
  for rep in $(seq 1 "$reps"); do
    args=()
    [ "$mode" = - ] || read -r -a args <<< "$mode"
    name=$(printf '%02d' "$n")-$bin$(printf '%s' "${args[*]:-}" | tr ' ' '_')-$rep
    tries=0
    while :; do
      rc=0
      DMESG_LOG=$out/$name.dmesg tools/mali/panthor/run-on-board.sh "$bin" "${args[@]}" > "$out/$name.log" 2>&1 || rc=$?
      [ -n "${RETRY_BUSY:-}" ] || break
      grep -qE '^board_lock: busy|^run-on-board.sh: /dev/dri/renderD128 is in use, not starting' "$out/$name.log" || break
      tries=$((tries + 1))
      if [ "$tries" -ge "${RETRY_MAX:-60}" ]; then
        echo "run-modes.sh: $name: the board stayed busy for $tries tries" >&2
        break
      fi
      echo "run-modes.sh: $name: the board is busy, again in ${RETRY_BUSY}s ($(date +%T))" >&2
      rm -f "$out/$name.log" "$out/$name.dmesg"
      sleep "$RETRY_BUSY"
    done
    echo "$name rc=$rc $(date +%T)" >> "$out/runs.txt"
    [ "$rc" = 0 ] || failed=1
    if [ -z "$first_marker" ] && [ -f "$out/$name.dmesg" ]; then
      first_marker=$(grep -o "yakogl-run-start $bin [0-9-]*" "$out/$name.dmesg" | tail -1 || true)
    fi
    n=$((n + 1))
  done
done
shopt -s nullglob
captures=("$out"/*.dmesg)
for f in "${captures[@]}"; do
  [ -n "$first_marker" ] || break
  at=$(grep -n -F "$first_marker" "$f" | head -1 | cut -d: -f1 || true)
  [ -n "$at" ] || continue
  tail -n +"$at" "$f" > "$f.cut" && mv "$f.cut" "$f"
done
if [ ${#captures[@]} -gt 0 ] && grep -lqiE '([0-9a-f]{2}:){5}[0-9a-f]{2}' "${captures[@]}"; then
  echo "run-modes.sh: a capture still holds a MAC address; cut it before committing:" >&2
  grep -liE '([0-9a-f]{2}:){5}[0-9a-f]{2}' "${captures[@]}" >&2
  exit 8
fi
cat "$out/runs.txt"
exit "$failed"
