#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c2-check-selftest.sh
#
# Host-only: pr-c2-check.sh on the fake log directories of
# tools/cts/testdata/pr-c2-check/: ok (both legs), no-knob-on (the knob-on
# log is NOT EXERCISED, partial renders 0) and depth-mismatch (the knob-on
# log is FAIL depth).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHK=$HERE/pr-c2-check.sh
FIX=$HERE/testdata/pr-c2-check
fail=0

# check NAME WANT-EXIT DIR PATTERN
check() {
    local out rc
    out=$("$CHK" "$FIX/$3" 2>&1); rc=$?
    if [ "$rc" = "$2" ] && printf '%s\n' "$out" | grep -Fq -- "$4"; then
        echo "ok   $1"
    else
        echo "FAIL $1: rc $rc (want $2), want '$4' in:"
        printf '%s\n' "$out" | sed 's/^/       /'
        fail=1
    fi
}

check "both legs" 0 ok \
    "pr-c2-check: OK knob-off 2026-12-pr-negative/off.log knob-on 2026-12-pr-217/on.log"
check "knob-on missing" 1 no-knob-on \
    "pr-c2-check: FAIL knob-on leg missing"
check "knob-on depth mismatch" 1 depth-mismatch \
    "pr-c2-check: FAIL knob-on leg missing"
check "no log dir" 2 nonexistent "missing"

[ "$fail" = 0 ] && echo "pr-c2-check selftest: all passed"
exit "$fail"
