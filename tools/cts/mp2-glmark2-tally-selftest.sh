#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/cts/mp2-glmark2-tally-selftest.sh
#
# Host-only checks of mp2-glmark2-tally.py on synthetic logs.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2gm.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
fail=0
run() { python3 -I "$HERE/mp2-glmark2-tally.py" "$@"; }
want() { case "$3" in *"$2"*) echo "  ok    $1" ;; *) echo "  FAIL  $1: no '$2' in: $3"; fail=$((fail+1)) ;; esac; }
rc() { if [ "$2" = "$3" ]; then echo "  ok    $1"; else echo "  FAIL  $1: rc $3, wanted $2"; fail=$((fail+1)); fi; }

scene() { # entry name result
  printf 'glmark2-mem: entry %s before setup: used 1\n[%s] <default>: %s\nglmark2-mem: entry %s end: used 2\n' "$1" "$2" "$3" "$1"
}

( echo 'glmark2: upstream abc (2023.11), size 1280x720, args ""'
  scene 0 build 'FPS: 120 FrameTime: 8.333 ms'
  scene 1 texture 'FPS: 60 FrameTime: 16.667 ms'
  scene 2 shadow 'Unsupported'
  printf 'glmark2: entry 2 [shadow] <default>: unsupported\n'
  printf 'glmark2-mem: entry 3 before setup: used 1\n[bump] <default>: Set up failed\nglmark2: entry 3 [bump] <default>: failed, why 7\nglmark2-mem: entry 3 end: used 1\n'
  echo '                                  glmark2 Score: 90 '
  echo 'glmark2: score 90: 2 scenes ran, 1 failed, 1 unsupported (4 list entries); window 1280x720, shown yes, GPU resets 0'
  echo 'glmark2: GL errors raised 0, draws that sampled an incomplete texture 0 (GL errors recorded 0)'
) > "$T/full.log"
out=$(run "$T/full.log"); r=$?
rc "full log with a failed scene exits 1" 1 "$r"
want "score" "score: 90" "$out"
want "fps" "120" "$out"
want "failed" "failed" "$out"
want "unsupported" "unsupported" "$out"
want "complete" "run: complete" "$out"
j=$(run --json "$T/full.log")
want "json score" '"score": 90' "$j"
want "json why" '"why": 7' "$j"
if printf '%s' "$j" | python3 -I -c 'import json,sys; json.load(sys.stdin)'; then echo "  ok    json parses"; else echo "  FAIL  json"; fail=$((fail+1)); fi

( scene 0 build 'FPS: 100 FrameTime: 10.000 ms'
  echo '                                  glmark2 Score: 100 '
  echo 'glmark2: score 100: 1 scenes ran, 0 failed, 0 unsupported (1 list entries); window 1280x720, shown yes, GPU resets 0'
) > "$T/ok.log"
run "$T/ok.log" >/dev/null; rc "clean run exits 0" 0 $?

# the end block's two score lines and its list lines (driver/docs/glmark2.md,
# "The list at the end")
( scene 0 build 'FPS: 100 FrameTime: 10.000 ms'
  echo '                                  glmark2 Score: 100 '
  echo 'glmark2: score 100: 1 scenes ran, 0 failed, 0 unsupported (1 list entries)'
  echo 'glmark2: 1280x720, shown yes, GPU resets 2, GL errors recorded 0'
  echo 'glmark2: GL errors raised 0, draws that sampled an incomplete texture 0'
  echo 'glmark2-list: 0 build                          ran 100 fps'
) > "$T/block.log"
out=$(run "$T/block.log"); r=$?
rc "the end block's run exits 0" 0 "$r"
want "the block's score" "score: 100" "$out"
want "the block's window line" "window 1280x720, shown yes, GPU resets 2" "$out"
want "the list line is no scene" "1 ok" "$out"

( scene 0 build 'FPS: 100 FrameTime: 10.000 ms'
  echo 'glmark2-mem: entry 1 before setup: used 1'
) > "$T/hung.log"
out=$(run "$T/hung.log"); r=$?
rc "hung and truncated exits 3" 3 "$r"
want "hung" "hung" "$out"
want "truncated" "TRUNCATED" "$out"
want "no score" "score: absent" "$out"

scene 0 build 'FPS: 100 FrameTime: 10.000 ms' > "$T/trunc.log"
out=$(run "$T/trunc.log"); r=$?
rc "truncated exits 3" 3 "$r"
want "truncated flagged" "TRUNCATED" "$out"
case "$out" in *hung*) echo "  FAIL  false hung"; fail=$((fail+1)) ;; *) echo "  ok    no false hung" ;; esac

out=$(run - < "$T/ok.log"); want "stdin" "score: 100" "$out"
run >/dev/null 2>&1; rc "no args exits 2" 2 $?
out=$(run --help)
want "help cites the score line" "main.cpp:1646" "$out"
want "help cites the failure lines" "main.cpp:1210" "$out"

if [ "$fail" = 0 ]; then echo "mp2-glmark2-tally-selftest: all ok"; exit 0; fi
echo "mp2-glmark2-tally-selftest: $fail failed"; exit 1
