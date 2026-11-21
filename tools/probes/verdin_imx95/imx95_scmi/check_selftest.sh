#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of check.sh on canned consoles (testdata/): the canned pass
# matches expected.txt; a changed value, a missing key, a FAIL verdict and a
# bad expectation line each fail with their own line.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
t=$(mktemp -d "${TMPDIR:-/tmp}/vi95scmi.XXXXXX")
trap 'rm -rf "$t"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
good=$here/testdata/console-pass.txt

out=$(sh "$here/check.sh" "$good") || fail "the canned pass did not pass: $out"
echo "$out" | grep -qx 'imx95_scmi check: 24 expected, 24 match, 0 differ, 0 missing; verdict PASS' ||
    fail "summary: $out"

# a level another agent lowered
sed 's/perf 12 level = 3/perf 12 level = 2/' "$good" > "$t/differ"
if out=$(sh "$here/check.sh" "$t/differ"); then fail "a changed value passed"; fi
echo "$out" | grep -qx 'differs: perf 12 level = 2 (want 3)' || fail "differ line: $out"

# a clock whose line is not there
grep -v 'clk 155 permissions' "$good" > "$t/missing"
if out=$(sh "$here/check.sh" "$t/missing"); then fail "a missing key passed"; fi
echo "$out" | grep -qx 'missing: clk 155 permissions' || fail "missing line: $out"

# the probe's own FAIL
sed 's/imx95_scmi: PASS .*/imx95_scmi: FAIL rc -5: the base protocol did not answer (1 facts)/' "$good" > "$t/fail"
if out=$(sh "$here/check.sh" "$t/fail"); then fail "a FAIL verdict passed"; fi
echo "$out" | grep -q 'verdict FAIL$' || fail "verdict: $out"

# an expectation line without a tab
printf 'base agent 2 AP-NS\n' > "$t/exp"
if out=$(sh "$here/check.sh" "$good" "$t/exp"); then fail "a bad expectation passed"; fi
echo "$out" | grep -q '^expected.txt: bad line' || fail "bad line: $out"

# usage
if sh "$here/check.sh" 2>/dev/null; then fail "no argument passed"; else [ $? = 2 ] || fail "usage exit"; fi

echo "imx95_scmi check selftest: ok"
