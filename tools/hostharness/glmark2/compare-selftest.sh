#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   bash tools/hostharness/glmark2/compare-selftest.sh
#
# compare.py on the fixture run directories in testdata/compare/ (before/ and
# after/: the files a GM2_PROFILE run leaves, with made-up times) against the
# tables it must print. To change the expected tables on purpose, rerun the
# commands below and review the diff:
#   python3 -I compare.py before after v3d > expected.md
#   python3 -I compare.py --entry-points 2 --before-label main --after-label fix before after v3d > expected-entry-points.md
#   python3 -I compare.py --before before-r1 before-r2 before-r3 --after after-r1 after-r2 after-r3 --entry-points 1 v3d > expected-repeats.md
# (before-r2 of terrain is a disturbed repeat: wall time 40% above cpu time.)
# Host only: python3 and diff.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
D=$HERE/testdata/compare
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

check() { # <name> <expected file> <compare.py arguments...>
	local name=$1 want=$2 got
	shift 2
	got=$(python3 -I "$HERE/compare.py" "$@" 2>&1)
	if diff <(printf '%s\n' "$got") "$D/$want" >/dev/null; then
		ok "$name"
	else
		bad "$name: output differs from $want"
		diff <(printf '%s\n' "$got") "$D/$want" | head -20
	fi
}

check "default table (GL time, scene without frames left out)" expected.md \
	"$D/before" "$D/after" v3d
check "entry points and labels" expected-entry-points.md \
	--entry-points 2 --before-label main --after-label fix "$D/before" "$D/after" v3d

R="$D/before-r1 $D/before-r2 $D/before-r3"
A="$D/after-r1 $D/after-r2 $D/after-r3"
# shellcheck disable=SC2086
check "repeats: minimum, n, CV, noise mark, disturbed repeat left out" expected-repeats.md \
	--before $R --after $A --entry-points 1 v3d
# shellcheck disable=SC2086
check "repeats: --gpu form gives the same table" expected-repeats.md \
	--gpu v3d --before $R --after $A --entry-points 1
# one directory a side in the new form is the old table
check "one directory a side in the new form: single-run table" expected.md \
	--before "$D/before" --after "$D/after" v3d

# a run compared with itself has no change in any row
same=$(python3 -I "$HERE/compare.py" "$D/before" "$D/before" v3d | grep -c ' | 1.00x |')
[ "$same" = 3 ] && ok "same run twice: ratio 1.00x" || bad "same run twice gave $same unit ratios, want 3"

# no scene in both runs is an error, not an empty table
if python3 -I "$HERE/compare.py" "$D/before" "$D/before" powervr >/dev/null 2>&1; then
	bad "a missing GPU's logs were accepted"
else
	ok "a GPU with no logs fails"
fi

if [ "$fail" -ne 0 ]; then
	echo "compare-selftest: $fail failed" >&2
	exit 1
fi
echo "compare-selftest: ok"
