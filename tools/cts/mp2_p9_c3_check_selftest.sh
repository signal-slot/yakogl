#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_p9_c3_check_selftest.sh
#
# Host-only check of tools/cts/mp2-p9-c3-check.sh on tiny synthetic shard logs,
# baseline, mustpass list and notes in a temporary directory: matching line ok,
# mismatched line FAIL, a missing case listed ok, a missing case not listed
# FAIL, no logs exit 2. Nothing here is a real capture.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-p9-c3-check.sh
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-c3.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

P=dEQP-GLES2.functional
H=$(printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names')
# baseline: a fails; mustpass: a b c d; accepted: none
printf '%s\n0\tf\t9\t8\t1\t0\t0\t0\t%s.a\n' "$H" $P >"$D/base.tsv"
: >"$D/acc.txt"
printf '%s.a\n%s.b\n%s.c\n%s.d\n' $P $P $P $P >"$D/must.txt"
OPTS=(--diff-opt --baseline --diff-opt "$D/base.tsv" --diff-opt --accepted --diff-opt "$D/acc.txt"
	--diff-opt --mustpass --diff-opt "$D/must.txt")

# two shards: a Fail (shared), b Pass, c Fail (new), d Pass
mkdir "$D/s1" "$D/s2"
cat >"$D/s1/shard-1.log" <<EOF
deqp: case filter '$P.*'
13:00:01 deqp: Fail $P.a
13:00:02 deqp: Pass $P.b
deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings
EOF
cat >"$D/s2/shard-2.log" <<EOF
deqp: case filter '$P.*'
13:01:01 deqp: Fail $P.c
13:01:02 deqp: Pass $P.d
deqp: done: 1 passed, 1 failed, 0 not supported, 0 warnings
EOF
LOGS=("$D/s1/shard-1.log" "$D/s2/shard-2.log")
run() { bash "$TOOL" "${OPTS[@]}" "$@" 2>&1; }
last() { tail -1 <<<"$1"; }

# matching line
printf 'Results\n`mp2-p9-diff: shared 1, new 1, fixed 0, missing 0` (built).\n' >"$D/n-ok.md"
out=$(run --notes "$D/n-ok.md" "${LOGS[@]}"); rc=$?
chk "match: exit 0" test $rc -eq 0
chk "match: last line ok" test "$(last "$out")" = "mp2-p9-c3-check: ok"

# several quoted lines: the latest one counts
printf 'old `mp2-p9-diff: shared 9, new 9, fixed 9, missing 9`\nnew `mp2-p9-diff: shared 1, new 1, fixed 0, missing 0`\n' >"$D/n-two.md"
out=$(run --notes "$D/n-two.md" "${LOGS[@]}"); rc=$?
chk "latest quoted line wins: exit 0" test $rc -eq 0

# mismatched line
printf '`mp2-p9-diff: shared 1, new 2, fixed 0, missing 0`\n' >"$D/n-bad.md"
out=$(run --notes "$D/n-bad.md" "${LOGS[@]}"); rc=$?
chk "mismatch: exit 1" test $rc -eq 1
chk "mismatch: last line FAIL" grep -q '^mp2-p9-c3-check: FAIL' <<<"$(last "$out")"

# no quoted line at all
printf 'nothing here\n' >"$D/n-none.md"
out=$(run --notes "$D/n-none.md" "${LOGS[@]}"); rc=$?
chk "no quoted line: exit 1" test $rc -eq 1

# missing case: shard 2 only has c; b, d (and a) are missing from a log of c alone
cat >"$D/s3.log" <<EOF
deqp: case filter '$P.*'
13:00:01 deqp: Fail $P.a
13:00:02 deqp: Pass $P.b
13:00:03 deqp: Fail $P.c
EOF
printf '`mp2-p9-diff: shared 1, new 1, fixed 0, missing 1`\nMissing: %s.d, the shard died on it.\n' $P >"$D/n-miss.md"
out=$(run --notes "$D/n-miss.md" "$D/s3.log"); rc=$?
chk "missing listed: exit 0" test $rc -eq 0
chk "missing listed: last line ok" test "$(last "$out")" = "mp2-p9-c3-check: ok"

printf '`mp2-p9-diff: shared 1, new 1, fixed 0, missing 1`\nNo names here.\n' >"$D/n-unl.md"
out=$(run --notes "$D/n-unl.md" "$D/s3.log"); rc=$?
chk "missing not listed: exit 1" test $rc -eq 1
chk "missing not listed: names the case" grep -q "not in the notes: $P.d" <<<"$out"
chk "missing not listed: last line FAIL" grep -q '^mp2-p9-c3-check: FAIL' <<<"$(last "$out")"

# no logs
out=$(run --notes "$D/n-ok.md" "$D/nonexistent/shard-*.log"); rc=$?
chk "no logs: exit 2" test $rc -eq 2
chk "no logs: message" grep -q 'no shard logs match' <<<"$out"

echo "mp2_p9_c3_check selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
