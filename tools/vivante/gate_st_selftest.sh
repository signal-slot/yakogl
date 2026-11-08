#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# gate_st_selftest.sh - checks tools/vivante/gate_st.sh without a Mesa tree.
#
#   tools/vivante/gate_st_selftest.sh
#
# Prints "N ok, M failed" and exits 0 only if none failed. It covers the gap
# path, the usage error, synthetic trees that are not real Mesa trees, and the
# pinned mode list. It does NOT run `golden` or the hwdb pass path against a
# real tree; that needs MESA_ETNA_TREE (see gate_st.sh).

set -u
# The checks read the scripts' messages, so the locale must not matter.
export LC_ALL=C
unset MESA_ETNA_TREE MESA_ETNA_BUILD GATE_ST_WANT_FILE

HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
GATE=$HERE/gate_st.sh

ok=0
failed=0
pass() { ok=$((ok + 1)); }
fail() {
	failed=$((failed + 1))
	echo "FAIL: $1" >&2
}

TMP=$(mktemp -d "${TMPDIR:-/tmp}/gate_st_selftest.XXXXXX") || exit 1
trap 'rm -rf "$TMP"' EXIT

# run_gate NAME ARGS...: the exit code in $rc, stdout in $TMP/out, stderr in
# $TMP/err. The caller sets any environment with `env` in front of ARGS.
run_gate() {
	"$@" >"$TMP/out" 2>"$TMP/err"
	rc=$?
}

# expect_gap NAME ARGS...: exit 2, exactly one stderr line that starts with
# `gap:`, naming MESA_ETNA_TREE and saying the check was NOT run, and nothing
# on stdout. Other stderr lines are not counted (the shell may add its own).
expect_gap() {
	local name=$1 n
	shift
	run_gate "$@"
	n=$(grep -c '^gap:' "$TMP/err")
	if [ "$rc" -eq 2 ] && [ "$n" -eq 1 ] &&
		grep -q '^gap: .*MESA_ETNA_TREE' "$TMP/err" &&
		grep -q 'NOT run' "$TMP/err" && [ ! -s "$TMP/out" ]; then
		pass
	else
		fail "$name (rc=$rc, gap lines=$n)"
	fi
}

# The gap path: MESA_ETNA_TREE unset, for both subcommands.
expect_gap "hwdb without a tree" "$GATE" hwdb
expect_gap "golden without a tree" "$GATE" golden

# A directory that does not exist.
expect_gap "hwdb, missing directory" env MESA_ETNA_TREE="$TMP/nope" "$GATE" hwdb
expect_gap "golden, missing directory" env MESA_ETNA_TREE="$TMP/nope" "$GATE" golden

# A directory that is not a Mesa tree.
mkdir "$TMP/empty"
expect_gap "hwdb, not a Mesa tree" env MESA_ETNA_TREE="$TMP/empty" "$GATE" hwdb
expect_gap "golden, not a Mesa tree" env MESA_ETNA_TREE="$TMP/empty" "$GATE" golden

# A synthetic tree whose two hwdb headers exist but contain no rows. hwdb must
# not pass: hwdb_compare.py finds no matching row and exits 1 with a message
# (never a traceback). golden has no build directory there, so it is a gap.
HW=$TMP/hollow
mkdir -p "$HW/src/etnaviv/hwdb/st" "$HW/src/etnaviv/hwdb/nxp"
: >"$HW/src/etnaviv/hwdb/st/gc_feature_database.h"
: >"$HW/src/etnaviv/hwdb/nxp/gc_feature_database.h"
run_gate env MESA_ETNA_TREE="$HW" "$GATE" hwdb
if [ "$rc" -eq 1 ] && grep -q '0 rows match' "$TMP/err" "$TMP/out" &&
	! grep -q Traceback "$TMP/err" "$TMP/out"; then
	pass
else
	fail "hwdb on empty headers: want exit 1 and '0 rows match' with no Traceback (rc=$rc)"
fi
expect_gap "golden, empty headers and no build" env MESA_ETNA_TREE="$HW" "$GATE" golden

# A build directory without the shim (and without the rest).
mkdir -p "$TMP/build"
expect_gap "golden, build without the shim" env MESA_ETNA_TREE="$HW" MESA_ETNA_BUILD="$TMP/build" "$GATE" golden

# The usage error.
for args in "" "bogus" "hwdb golden" "HWDB" "hwdb;true"; do
	# shellcheck disable=SC2086 # word splitting of the literal list is the point
	run_gate "$GATE" $args
	if [ "$rc" -eq 64 ]; then pass; else fail "usage error for '$args' (rc=$rc)"; fi
done
run_gate env MESA_ETNA_TREE="$TMP/empty" "$GATE" bogus
if [ "$rc" -eq 64 ]; then pass; else fail "usage error is checked before the tree (rc=$rc)"; fi

# golden pins exactly three modes: 1, 8 and 15.
modes=$(grep -c '^mode [0-9][0-9]* renderer: ' "$GATE")
list=$(grep '^mode [0-9]' "$GATE" | sed "s/^mode \([0-9]*\) .*/\1/" | tr '\n' ' ')
if [ "$modes" -eq 3 ] && [ "$list" = "1 8 15 " ]; then
	pass
else
	fail "golden pins modes '$list' ($modes lines), want 1 8 15"
fi

# Both scripts parse.
for s in "$GATE" "$HERE/gate_st_selftest.sh"; do
	if bash -n "$s"; then pass; else fail "bash -n $s"; fi
done

echo "$ok ok, $failed failed"
[ "$failed" -eq 0 ]
