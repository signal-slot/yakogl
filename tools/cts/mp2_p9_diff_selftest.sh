#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_p9_diff_selftest.sh
#
# Host-only check of tools/cts/mp2-p9-diff.py on small synthetic baselines,
# results.tsv files and UART logs in a temporary directory: every class
# (shared, new, fixed, accepted changed, missing), both input forms, the
# exit codes, and the real i.MX8MP baseline against itself. Nothing here is a
# real capture.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-p9-diff.py
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-p9.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

P=dEQP-GLES2.functional
H=$(printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names')
# baseline: A, B, C fail
printf '%s\n0\tf\t9\t6\t3\t0\t0\t0\t%s.a;%s.b;%s.c\n' "$H" $P $P $P >"$D/base.tsv"
printf '%s.acc1\n%s.acc2  # reason\n' $P $P >"$D/acc.txt"
printf '%s.a\n%s.b\n%s.c\n%s.d\n%s.acc1\n%s.acc2\n%s.e\n' $P $P $P $P $P $P $P >"$D/must.txt"
ARGS=(--baseline "$D/base.tsv" --accepted "$D/acc.txt" --mustpass "$D/must.txt")
# after <text> <pattern> <name>: the line after the first line matching <pattern> is <name>
after() { grep -A1 -- "$2" <<<"$1" | sed -n 2p | grep -qx -- "$3"; }
run() { python3 -I "$TOOL" "${ARGS[@]}" "$@" 2>&1; }

# DK results.tsv: a, d fail  -> shared a; new d; fixed b, c
printf '%s\n0\tf\t9\t7\t2\t0\t0\t0\t%s.a;%s.d\n' "$H" $P $P >"$D/dk.tsv"
out=$(run "$D/dk.tsv"); rc=$?
chk "tsv: summary line" grep -qx 'mp2-p9-diff: shared 1, new 1, fixed 2, missing 0' <<<"$out"
chk "tsv: new fails the exit" test $rc -eq 1
chk "tsv: shared name" grep -qx "$P.a" <<<"$out"
chk "tsv: new name listed under new" after "$out" 'new): 1' "$P.d"
chk "tsv: fixed names" bash -c "grep -A2 'fixed): 2' <<<'$out' | grep -q '$P.b'"
run --report-only "$D/dk.tsv" >/dev/null; chk "tsv: --report-only exits 0" test $? -eq 0

# a clean tsv: nothing new
printf '%s\n0\tf\t9\t8\t1\t0\t0\t0\t%s.b\n' "$H" $P >"$D/clean.tsv"
out=$(run "$D/clean.tsv"); rc=$?
chk "tsv clean: exit 0" test $rc -eq 0
chk "tsv clean: summary" grep -qx 'mp2-p9-diff: shared 1, new 0, fixed 2, missing 0' <<<"$out"

# an accepted name failing on the DK (tsv)
printf '%s\n0\tf\t9\t8\t1\t0\t0\t0\t%s.acc1\n' "$H" $P >"$D/acc.tsv"
out=$(run --report-only "$D/acc.tsv")
chk "tsv: accepted name now Fail" grep -q "$P.acc1 (NotSupported -> Fail)" <<<"$out"

# UART log: a Pass, b Fail (shared), c absent, d Fail (new), acc1 NotSupported
# (unchanged), acc2 Pass (changed), e absent (missing); c is absent too
cat >"$D/dk.log" <<EOF
deqp: case filter '$P.*'
13:00:01 deqp: Pass $P.a
13:00:02 deqp: Fail $P.b
13:00:03 deqp: Fail $P.d
13:00:04 deqp: NotSupported $P.acc1
13:00:05 deqp: Pass $P.acc2
deqp: done: 2 passed, 2 failed, 1 not supported, 0 warnings
deqp: summary: 5 cases begun, 5 with a result, 0 without
EOF
out=$(run "$D/dk.log"); rc=$?
chk "log: summary line" grep -qx 'mp2-p9-diff: shared 1, new 1, fixed 1, missing 2' <<<"$out"
chk "log: exit 1" test $rc -eq 1
chk "log: shared b" after "$out" 'shared with the i.MX8MP: 1' "$P.b"
chk "log: fixed a" after "$out" 'DK passes (fixed): 1' "$P.a"
chk "log: missing c and e (sorted)" bash -c "grep -A2 'missing from the DK run: 2' <<<'$out' | tail -2 | tr '\n' ' ' | grep -qx '$P.c $P.e '"
chk "log: accepted changed only acc2" bash -c "grep -A1 'changed state: 1' <<<'$out' | grep -qx '$P.acc2 (NotSupported -> Pass)'"
run --report-only "$D/dk.log" >/dev/null; chk "log: --report-only exits 0" test $? -eq 0

# case filter scope: a filter that excludes everything but .a leaves nothing missing
sed "s/case filter '[^']*'/case filter '$P.a'/" "$D/dk.log" >"$D/scoped.log"
out=$(run --report-only "$D/scoped.log")
chk "log: filtered scope has no missing" grep -q 'missing 0$' <<<"$out"

# a log that is missing only, no failures, exits 1 (M > 0)
cat >"$D/miss.log" <<EOF
deqp: Pass $P.a
EOF
run "$D/miss.log" >/dev/null; chk "log: missing alone exits 1" test $? -eq 1

# error exits
run "$D/nonexistent" >/dev/null; chk "missing input exits 2" test $? -eq 2
run "$D/dk.tsv" "$D/dk.log" >/dev/null; chk "mixed forms exit 2" test $? -eq 2

# the real baseline against itself: 19 shared, none new
out=$(python3 -I "$TOOL" --report-only "$HERE/imx8mp-full-tree-29460f96.results.tsv"); rc=$?
chk "i.MX8MP baseline: 19 shared, 0 new" grep -qx 'mp2-p9-diff: shared 19, new 0, fixed 0, missing 0' <<<"$out"
chk "i.MX8MP baseline: --report-only exit 0" test $rc -eq 0
python3 -I "$TOOL" "$HERE/imx8mp-full-tree-29460f96.results.tsv" >/dev/null
chk "i.MX8MP baseline: exit 0 without --report-only" test $? -eq 0

echo "mp2_p9_diff selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
