#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c3-check-selftest.sh
#
# Host-only: tools/cts/pr-c3-check.sh on the synthetic trees of
# tools/cts/testdata/pr-c3-check/<case>/ (log/ and a RESULTS.md stub):
#   ok          flat legs, FPS and score differ (allowed); a newer incomplete
#               *-pr-regress record is skipped, a *-pr-217 one is ignored
#   ok-subdirs  off/ and on/ legs; FAIL/TIMEOUT and Unsupported/Set up failed
#               differ but no PASS or completed scene does; equal resets
#   mismatch    gl_test case 4 PASS with the knob off, FAIL with it on
#   gmmismatch  glmark2 [shading] completes off, 'Set up failed' on
#   newreset    the knob-on glmark2 leg has a GPU reset and HWR counter 1
#   newstop     the knob-on gl_test leg has a 'render: stopped' line
#   noon        the knob-on glmark2 log is missing
#   nocite      RESULTS.md's AM62P gl_test cell cites another directory
#               (a non-Common-set table that cites it must not count)
#   norecords   no *-pr-regress directory
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHK=$HERE/pr-c3-check.sh
FIX=$HERE/testdata/pr-c3-check
R=2026-12-01-pr-regress
fail=0

# check CASE WANT-EXIT PATTERN: one verdict line, the exit and the pattern
check() {
    local out rc
    out=$(bash "$CHK" "$FIX/$1/log" "$FIX/$1/RESULTS.md" 2>&1); rc=$?
    if [ "$rc" = "$2" ] && [ "$(printf '%s\n' "$out" | wc -l)" = 1 ] &&
        printf '%s\n' "$out" | grep -Eq -- "$3"; then
        echo "ok   $1"
    else
        echo "FAIL $1: rc $rc (want $2), want one line matching '$3' in:"
        printf '%s\n' "$out" | sed 's/^/       /'
        fail=1
    fi
}

check ok 0 "^pr-c3-check: OK $R: gl_test 3 of 4 cases PASS in both legs, glmark2 3 of 3 scenes complete in both, no new resets"
check ok-subdirs 0 "^pr-c3-check: OK $R: gl_test 2 of 4 cases PASS in both legs, glmark2 2 of 3 scenes complete in both, no new resets \(off/on: gl_test 1/1 glmark2 0/0\)"
check mismatch 1 "^pr-c3-check: FAIL $R: gl_test pass sets differ \(knob off/on\): case 4 \(PASS/FAIL\)$"
check gmmismatch 1 "^pr-c3-check: FAIL $R: glmark2 completed scenes differ \(knob off/on\): \[shading\] shading=phong \(30/set up failed\)$"
check newreset 1 "^pr-c3-check: FAIL $R: new reset on the knob-on glmark2 leg: GPU resets 1 \(knob off 0\), HWR counter 1 \(knob off 0\)$"
check newstop 1 "^pr-c3-check: FAIL $R: new reset on the knob-on gl_test leg: 'render: stopped' lines 1 \(knob off 0\)$"
check noon 1 "^pr-c3-check: FAIL no complete pr-regress record: newest $R lacks glmark2 knob-on log$"
check nocite 1 "^pr-c3-check: FAIL $R: RESULTS.md's AM62P gl_test cell does not cite $R$"
check norecords 1 "^pr-c3-check: FAIL no pr-regress records$"

[ "$fail" = 0 ] && echo "pr-c3-check selftest: all passed"
exit "$fail"
