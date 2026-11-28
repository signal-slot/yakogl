#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Per tree row of a whole-tree board record: did its console show a GPU
# recovery, a fence timeout or a watchdog reset?
#
#   tools/cts/console-scan.sh <tree-file> <sha8>
#   tools/cts/console-scan.sh --selftest      (console-scan-selftest.sh)
#
# Rows and labels as tools/cts/tree-verdicts-check.sh reads them ("# image
# LABEL:" labels the next filter, else img<N>). Slice directories:
# tools/cts/log/*<set>-<sha8>* (<set> = the tree file's stem minus "-tree").
# Console of a row: <dir>/<label>.console.txt, else <dir>/<label>/console.txt,
# else <dir>/*-<label>-*/console.txt (a run directory); the first found.
# Verdict: the last "board.sh <board> <label> rc=N verdict=<v>" line of the
# directories (the line tools/board.sh prints).
#
# One line per row:  <label> <image> console=<yes|no> hwr=N fence=N wdog=N verdict=<v|missing>
# <image> is the row's first filter, shortened to its first 60 characters.
#
# Patterns (grep -aE, after the console's time prefix; each from its source):
#  hwr    "render: HWR during fence"      driver/rust/src/gpu/pvr/render.rs,
#                                          counted by tools/board.sh pvr_resets
#  fence  "render: fence [0-9]+ timeout"  render.rs wait_for_at (km_log Err)
#         "pvr_backend: kick [0-9]+ did not complete"
#                                          src/pvr/backend.rs, glog_once
#                                          (docs/notes/pvr-3d-mipmap-hang.md
#                                          recipe step 4, step 1 grep)
#  wdog   "rti_arm: feeding bound of"      driver/zephyr/lib/board/am62p/rti_arm.c
#         "RTI feeder lifetime over"       board.sh END_RE_DEFAULT (am62p)
#         "ZEPHYR FATAL ERROR"             board.sh END_RE_DEFAULT
#         "Kernel panic|Oops:"             a Linux-side fault (board.sh x2/Pi 4)
# Not counted: "U-Boot SPL" (a boot banner; every capture can start with it).
# Failing verdicts: watchdog*, no_reset, done_with_resets (board.sh).
# Exit 0: no row has a failing verdict or nonzero hwr/fence; 1 otherwise;
# 2 on usage errors. Never prints board addresses (it prints counts only).
set -euo pipefail

me=${0##*/}
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)

if [ "${1:-}" = --selftest ]; then
  exec "$here/console-scan-selftest.sh"
fi
if [ $# -ne 2 ]; then
  echo "usage: $me <tree-file> <sha8>" >&2
  exit 2
fi
tree=$1
sha=$2
if [ ! -r "$tree" ]; then
  echo "$me: cannot read $tree" >&2
  exit 2
fi
case $sha in
  [0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f][0-9a-f]) ;;
  *) echo "$me: sha8 must be 8 lowercase hex digits: $sha" >&2; exit 2 ;;
esac

logdir=${CONSOLE_SCAN_LOGDIR:-$here/log}
stem=$(basename "$tree" .txt)
set_name=${stem%-tree}

hwr_re='render: HWR during fence'
fence_re='render: fence [0-9]+ timeout|pvr_backend: kick [0-9]+ did not complete'
wdog_re='rti_arm: feeding bound of|RTI feeder lifetime over|ZEPHYR FATAL ERROR|Kernel panic|Oops:'

dirs=()
for d in "$logdir"/*"$set_name"-"$sha"*; do
  [ -d "$d" ] && dirs+=("$d")
done

tmp=$(mktemp -d "${TMPDIR:-/tmp}/cscan.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
verd=$tmp/verdicts
: >"$verd"
for d in "${dirs[@]}"; do
  for f in "$d"/*; do
    [ -f "$f" ] || continue
    grep -aE '^board\.sh [^ ]+ [^ ]+ rc=[0-9-]+ verdict=' "$f" >>"$verd" 2>/dev/null || true
  done
done

# count FILE REGEX -> number of matching lines
count() { grep -acE "$2" "$1" 2>/dev/null || true; }

# find_console LABEL -> path of the row's console or empty
find_console() {
  local l=$1 d c
  for d in "${dirs[@]}"; do
    for c in "$d/$l.console.txt" "$d/$l/console.txt"; do
      if [ -f "$c" ]; then echo "$c"; return; fi
    done
    for c in "$d"/*-"$l"-*/console.txt; do
      if [ -f "$c" ]; then echo "$c"; return; fi
    done
  done
}

rows=0 bad=0 noc=0 nov=0
while IFS=$'\t' read -r lab img; do
  rows=$((rows + 1))
  v=$(awk -v l="$lab" '$1 == "board.sh" && $3 == l { x = $5 } END { sub(/^verdict=/, "", x); print x }' "$verd")
  [ -n "$v" ] || { v=missing; nov=$((nov + 1)); }
  con=$(find_console "$lab")
  h=0 f=0 w=0 has=no
  if [ -n "$con" ]; then
    has=yes
    h=$(count "$con" "$hwr_re")
    f=$(count "$con" "$fence_re")
    w=$(count "$con" "$wdog_re")
  else
    noc=$((noc + 1))
  fi
  flag=""
  case $v in
    watchdog* | no_reset | done_with_resets) flag=1 ;;
  esac
  { [ "$h" -gt 0 ] || [ "$f" -gt 0 ]; } && flag=1
  if [ -n "$flag" ]; then bad=$((bad + 1)); fi
  printf '%s %s console=%s hwr=%s fence=%s wdog=%s verdict=%s%s\n' \
    "$lab" "${img:0:60}" "$has" "$h" "$f" "$w" "$v" "${flag:+  FLAG}"
done < <(awk '
  /^# image / { l = $3; sub(/:.*/, "", l); pending = l; next }
  /^#/ || /^[ \t]*$/ { next }
  { idx++; lab = (pending != "") ? pending : "img" idx; pending = ""
    split($0, a, ","); printf "%s\t%s\n", lab, a[1] }' "$tree")

echo "console-scan: tree $tree  sha8 $sha  directories ${#dirs[@]}  rows $rows  flagged $bad  no console $noc  no verdict $nov"
if [ "$rows" -gt 0 ] && [ "$bad" -eq 0 ]; then
  if [ "$noc" -gt 0 ] || [ "$nov" -gt 0 ]; then
    echo "console-scan: OK for the rows scanned ($noc without console, $nov without verdict: not checked)"
  else
    echo "console-scan: OK"
  fi
  exit 0
fi
echo "console-scan: FAIL"
exit 1
