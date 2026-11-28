#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of tools/cts/board-stage-lossy.py on the fixtures of
# tools/cts/testdata/board-stage-lossy/: all done, one lossy, mixed, malformed.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$HERE/testdata/board-stage-lossy
H=$HERE/board-stage-lossy.py
fail=0
check() { # name want_rc want_out_regex(or empty) path
    local out rc
    out=$(python3 -I "$H" "$4" 2>/dev/null); rc=$?
    if [ "$rc" != "$2" ]; then echo "FAIL $1: exit $rc, want $2"; fail=1; return; fi
    if [ -n "$3" ] && ! printf '%s\n' "$out" | grep -Eq "$3"; then echo "FAIL $1: output lacks /$3/: $out"; fail=1; return; fi
    if [ -z "$3" ] && [ -n "$out" ]; then echo "FAIL $1: unexpected output: $out"; fail=1; return; fi
    echo "ok   $1"
}
check all-done 0 '' "$T/all-done"
check one-lossy 1 '^done_lossy deqp-11 lost=7$' "$T/one-lossy/stage.log"
check mixed 1 '^done_lossy deqp-05 lost=3$' "$T/mixed"
out=$(python3 -I "$H" "$T/mixed" 2>/dev/null)
printf '%s\n' "$out" | grep -q '^done_lossy deqp-06 lost=?$' || { echo "FAIL mixed: deqp-06 missing"; fail=1; }
printf '%s\n' "$out" | grep -q 'deqp-03\|deqp-07' && { echo "FAIL mixed: non-lossy row named"; fail=1; }
check malformed 2 '' "$T/malformed"
check missing 2 '' "$T/does-not-exist"
[ $fail = 0 ] && echo "board-stage-lossy-selftest: ok"
exit $fail
