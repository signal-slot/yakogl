#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c3-check-selftest.sh
#
# Host-only: pr-c3-check.sh on the fake records of tools/cts/testdata/pr-c3-check/
# (each <case>/log/<date>-pr-regress/ and <case>/RESULTS.md): ok, mismatch
# (a gl_test case fails with the knob on), newreset (the knob-on glmark2 leg
# has a recovery and a 'render: stopped'), noon (no knob-on glmark2 log),
# noresults (RESULTS cites another directory), and an empty log directory
# (made here: git does not track empty directories).
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHK=$HERE/pr-c3-check.sh
FIX=$HERE/testdata/pr-c3-check
TMP=$(mktemp -d "${TMPDIR:-/var/tmp}/pr-c3.XXXXXX") || exit 2
trap 'rm -rf "$TMP"' EXIT
fail=0

# check NAME WANT-EXIT LOGDIR RESULTS PATTERN
check() {
    local out rc
    out=$("$CHK" "$3" "$4" 2>&1); rc=$?
    if [ "$rc" = "$2" ] && printf '%s\n' "$out" | grep -Fq -- "$5"; then
        echo "ok   $1"
    else
        echo "FAIL $1: rc $rc (want $2), want '$5' in:"
        printf '%s\n' "$out" | sed 's/^/       /'
        fail=1
    fi
}

check ok 0 "$FIX/ok/log" "$FIX/ok/RESULTS.md" "pr-c3-check: OK 2026-12-01-pr-regress"
check "pass-set mismatch" 1 "$FIX/mismatch/log" "$FIX/mismatch/RESULTS.md" "pr-c3-check: FAIL 2026-12-01-pr-regress: the gl_test pass sets differ"
check "new reset" 1 "$FIX/newreset/log" "$FIX/newreset/RESULTS.md" "pr-c3-check: FAIL 2026-12-01-pr-regress: the knob-on glmark2 leg shows GPU recoveries 1"
check "missing knob-on leg" 1 "$FIX/noon/log" "$FIX/noon/RESULTS.md" "pr-c3-check: FAIL 2026-12-01-pr-regress lacks glmark2-on.log"
check "RESULTS not citing" 1 "$FIX/noresults/log" "$FIX/noresults/RESULTS.md" "pr-c3-check: FAIL 2026-12-01-pr-regress: no AM62P gl_test row"
mkdir "$TMP/log"
check "no records" 1 "$TMP/log" "$FIX/ok/RESULTS.md" "pr-c3-check: FAIL no pr-regress records"
check "no log dir" 2 "$TMP/nonexistent" "$FIX/ok/RESULTS.md" "missing"

[ "$fail" = 0 ] && echo "pr-c3-check selftest: all passed"
exit "$fail"
