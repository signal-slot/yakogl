#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c7-check-selftest.sh
#
# Host-only checks of tools/cts/pr-c7-check.sh on the fixture trees in
# tools/cts/testdata/pr-c7-check/: `pass` (2 records cited, both present; a
# record cited after the status paragraph must not count), `defaulty` ('default
# y'), `nocite` (no record cited) and `missing` (a cited directory is absent).
set -u

HERE=$(dirname "$0")
FIX=$HERE/testdata/pr-c7-check
TOOL=$HERE/pr-c7-check.sh
fail=0

check() { # name want-exit pattern
    local name=$1 want=$2 pat=$3 out rc
    out=$(bash "$TOOL" "$FIX/$name" 2>&1)
    rc=$?
    if [ "$rc" != "$want" ]; then
        echo "FAIL $name: exit $rc, want $want"
        fail=1
    elif ! printf '%s\n' "$out" | grep -q -- "$pat"; then
        echo "FAIL $name: missing '$pat' in: $out"
        fail=1
    fi
}

check pass 0 '^pr-c7-check: 2 records cited, all present; knob default n$'
check defaulty 1 "default y"
check nocite 1 "cites no tools/cts/log"
check missing 1 "tools/cts/log/gone/ does not exist"

[ "$fail" = 0 ] && echo "pr-c7-check-selftest: ok"
exit "$fail"
