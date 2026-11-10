#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# One NanoPC-T6 glmark2-es2 board session whose output proves the board was
# left as found: preflight, /boot + U-Boot env + module snapshots around the
# runner, the comparison, an ssh-reachability line and the tally.
#
#   BOARD_HOST=<user@t6> tools/cts/mali-glmark2-session.sh [--dry-run] [--dev] \
#       [--scenes <file|list>] [--timeout <s>] <builddir> <outdir>
#   tools/cts/mali-glmark2-session.sh --dry-run [runner options] [<builddir> <outdir>]
#
# The runner options (--dev, --scenes, --timeout) go to mali-glmark2-run.sh
# unchanged. BOARD_HOST comes from the lease (target:nanopc-t6), never from a
# file; BOARD_SSH_OPTS (optional) goes to ssh. In order:
#   1. mali-glmark2-preflight.sh --build <builddir>; a failure stops here;
#   2. mali-board-state.sh snapshot -> <outdir>/state-before.txt;
#   3. mali-glmark2-run.sh -> <outdir>/run (its run directory; its raw files go
#      to RAW_DIR, default build/mali-glmark2-session/<outdir's name>/);
#   4. always (also when the runner failed or the session was interrupted):
#      a second snapshot -> <outdir>/state-after.txt, then mali-board-state.sh
#      compare, its lines printed verbatim and kept in <outdir>/state-compare.txt;
#   5. a final line "mali-glmark2-session: board reachable over ssh, boot_id
#      <same|changed>" or "... NOT reachable: needs a person, not retried";
#   6. glmark2-mali-tally.py on <outdir>/run: tally-table.txt,
#      tally-results-row.txt, tally-status-row.txt, tally-picture-lines.txt and
#      tally-causes.txt under <outdir>.
# A last line "mali-glmark2-session: exit N (<meaning>)" names the result.
#
# --dry-run prints the plan and touches nothing: no ssh, no scp, no
# BOARD_HOST, nothing written; <builddir> and <outdir> are optional then.
#
# The wrapper itself sends one command, `cat /proc/sys/kernel/random/boot_id`,
# for step 5; everything else is what the scripts it calls send (reading
# commands, and the runner's stage under the login's home). It touches no
# /boot, U-Boot environment, partition or module, and nothing of tailscale or
# sshd.
#
# Exit: 0 all clean (runner exit 0, tally read, /boot, env and modules
# unchanged, board reachable with the same boot_id); 1 the runner or the tally
# failed (any exit but 0) with the board intact; 2 usage; 3 board state
# CHANGED or not proven (compare said CHANGED, the second snapshot could not
# be taken though the board answers, or the boot_id changed); 4 board lost
# (not reachable after the run: needs a person); 5 stopped before the runner
# (preflight failed, or the first snapshot could not be taken): nothing ran.
# When several hold, 4 beats 3 beats 1. Scene verdicts are the result, not a
# failure: a run in which every scene was refused still exits 0.
#
# Environment for the self-test (tools/cts/mali-glmark2-session-selftest.sh):
# MALI_GLMARK2_PREFLIGHT, MALI_GLMARK2_STATE, MALI_GLMARK2_RUN and
# MALI_GLMARK2_TALLY name another script of the four (default: the ones beside
# this one).
#
# Its gate row (the self-test, in tools/check.sh) is added by a separate item
# of plan #332 (gate-row).
set -uo pipefail
me=mali-glmark2-session
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/../.." && pwd)
PREFLIGHT=${MALI_GLMARK2_PREFLIGHT:-$HERE/mali-glmark2-preflight.sh}
STATE=${MALI_GLMARK2_STATE:-$HERE/mali-board-state.sh}
RUN=${MALI_GLMARK2_RUN:-$HERE/mali-glmark2-run.sh}
TALLY=${MALI_GLMARK2_TALLY:-$HERE/glmark2-mali-tally.py}

usage() {
  sed -n '8,10p' "$0" | sed 's/^# \{0,1\}//' >&2
  exit 2
}

dry=0 ropts=() pos=()
while [ $# -gt 0 ]; do
  case $1 in
    --dry-run) dry=1; shift ;;
    --dev) ropts+=("$1"); shift ;;
    --scenes | --timeout)
      [ $# -ge 2 ] || { echo "$me: $1 needs a value" >&2; usage; }
      ropts+=("$1" "$2"); shift 2 ;;
    -h | --help) usage ;;
    --) shift; pos+=("$@"); break ;;
    -*) echo "$me: unknown option '$1'" >&2; usage ;;
    *) pos+=("$1"); shift ;;
  esac
done
[ "${#pos[@]}" -le 2 ] || usage
build=${pos[0]-} out=${pos[1]-}
if [ "$dry" = 0 ] && { [ -z "$build" ] || [ -z "$out" ]; }; then usage; fi

if [ "$dry" = 1 ]; then
  b=${build:-<builddir>} o=${out:-<outdir>}
  echo "$me: dry run, no ssh, nothing written; the session would run, in order:"
  echo "+ 1. BOARD_HOST=\$BOARD_HOST $PREFLIGHT --build $b"
  echo "+ 2. $STATE snapshot \$BOARD_HOST $o/state-before.txt"
  echo "+ 3. $RUN ${ropts[*]-} $b $o/run"
  echo "+ 4. (always, also after a failure or an interrupt) $STATE snapshot \$BOARD_HOST $o/state-after.txt"
  echo "+    $STATE compare $o/state-before.txt $o/state-after.txt  (lines kept in $o/state-compare.txt)"
  echo "+ 5. ssh \$BOARD_HOST 'cat /proc/sys/kernel/random/boot_id'  (the final reachability line)"
  echo "+ 6. python3 -I $TALLY [--results-row|--status-row|--picture-lines|--causes] $o/run  (tally-*.txt under $o)"
  echo "$me: exit codes: 0 all clean, 1 runner failed with the board intact, 2 usage, 3 board state CHANGED, 4 board lost, 5 stopped before the runner"
  exit 0
fi

host=${BOARD_HOST-}
[ -n "$host" ] || { echo "$me: set BOARD_HOST=<user@host> (the target:nanopc-t6 lease)" >&2; exit 2; }
export BOARD_HOST
read -r -a sshopts <<< "${BOARD_SSH_OPTS-}"
[ -d "$build" ] || { echo "$me: no build directory $build" >&2; exit 2; }
if [ -e "$out" ] && [ -n "$(ls -A "$out" 2> /dev/null)" ]; then
  echo "$me: $out is not empty; a session wants a fresh directory" >&2
  exit 2
fi
mkdir -p "$out" || exit 2
out=$(cd "$out" && pwd)
export RAW_DIR=${RAW_DIR:-$REPO/build/mali-glmark2-session/$(basename "$out")}

echo "$me: session into $out, board from BOARD_HOST (not printed)"

# ---- 1. preflight ---------------------------------------------------------------
echo "$me: 1/6 preflight"
bash "$PREFLIGHT" --build "$build" 2>&1 | tee "$out/preflight.txt"
if [ "${PIPESTATUS[0]}" != 0 ]; then
  echo "$me: preflight failed: not starting; nothing was run on the board"
  echo "$me: exit 5 (stopped before the runner)"
  exit 5
fi

# ---- 2. first snapshot ----------------------------------------------------------
echo "$me: 2/6 snapshot before"
if ! bash "$STATE" snapshot "$host" "$out/state-before.txt"; then
  echo "$me: the first snapshot failed: not starting; nothing was run on the board"
  echo "$me: exit 5 (stopped before the runner)"
  exit 5
fi

# ---- 3. runner, then 4-6 whatever happens ---------------------------------------
runner_rc=
finished=0
finish() {
  [ "$finished" = 0 ] || return
  finished=1
  trap - EXIT INT TERM
  local code=0 snap_rc cmp_rc=0 bid_before bid_now reach=0 same='' tally_rc=0 m f why
  [ -n "$runner_rc" ] || runner_rc=interrupted
  echo "$me: 4/6 snapshot after and compare (runner exit $runner_rc)"
  bash "$STATE" snapshot "$host" "$out/state-after.txt"
  snap_rc=$?
  if [ "$snap_rc" = 0 ]; then
    bash "$STATE" compare "$out/state-before.txt" "$out/state-after.txt" | tee "$out/state-compare.txt"
    cmp_rc=${PIPESTATUS[0]}
  else
    echo "$me: no second snapshot (exit $snap_rc): the board state after the run is not proven"
  fi
  echo "$me: 5/6 ssh reachability"
  bid_before=$(sed -n 's/^boot_id=//p' "$out/state-before.txt" | tail -1)
  bid_now=$(ssh -o BatchMode=yes -o ConnectTimeout=10 "${sshopts[@]}" "$host" 'cat /proc/sys/kernel/random/boot_id' 2> /dev/null) && [ -n "$bid_now" ] && reach=1
  if [ "$reach" = 1 ]; then
    if [ "$bid_now" = "$bid_before" ]; then same=same; else same=changed; fi
    echo "$me: board reachable over ssh, boot_id $same"
  else
    echo "$me: board NOT reachable: needs a person, not retried"
  fi
  echo "$me: 6/6 tally"
  if [ -f "$out/run/scenes.tsv" ] && [ -f "$out/run/meta.txt" ]; then
    python3 -I "$TALLY" "$out/run" > "$out/tally-table.txt" 2> "$out/tally-table.err"
    tally_rc=$?
    for m in results-row status-row picture-lines causes; do
      python3 -I "$TALLY" "--$m" "$out/run" > "$out/tally-$m.txt" 2> "$out/tally-$m.err" || [ "$tally_rc" != 0 ] || tally_rc=$?
    done
    for f in "$out"/tally-*.err; do [ -s "$f" ] || rm -f "$f"; done
    cat "$out/tally-table.txt"
    echo "$me: tally exit $tally_rc; table and row drafts under $out (tally-*.txt)"
  else
    echo "$me: no run directory to tally ($out/run has no scenes.tsv)"
    tally_rc=1
  fi
  if [ "$reach" = 0 ]; then code=4 why="board lost"
  elif [ "$same" = changed ] || [ "$cmp_rc" != 0 ] || [ "$snap_rc" != 0 ]; then code=3 why="board state CHANGED or not proven"
  elif [ "$runner_rc" != 0 ] || [ "$tally_rc" != 0 ]; then code=1 why="runner or tally failed, board intact"
  else code=0 why="all clean, board left as found"; fi
  echo "$me: exit $code ($why)"
  exit "$code"
}
trap finish EXIT
trap 'runner_rc=interrupted; exit 130' INT TERM

echo "$me: 3/6 runner"
bash "$RUN" "${ropts[@]}" "$build" "$out/run"
runner_rc=$?
echo "$me: runner exit $runner_rc"
exit 0
