#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_p9_c3_check_selftest.sh
#
# Host-only check of tools/cts/mp2-p9-c3-check.sh on a synthetic baseline,
# mustpass list, UART shard logs and notes files in a temporary directory:
# the equal line, the last quoted line winning, a stale quote, no quote, no
# log, missing cases with and without their cause, the ", excluded E" form,
# and the usage errors. Nothing here is a real capture. No board.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
TOOL=$HERE/mp2-p9-c3-check.sh
D=$(mktemp -d "${TMPDIR:-/tmp}/mp2-p9-c3.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }

P=dEQP-GLES2.functional
H=$(printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names')
printf '%s\n0\tf\t3\t2\t1\t0\t0\t0\t%s.a\n' "$H" $P >"$D/base.tsv"
: >"$D/acc.txt"
printf '%s.a\n%s.b\n%s.c\n%s.d\n' $P $P $P $P >"$D/must.txt"
OPTS=(--diff-opt --baseline --diff-opt "$D/base.tsv" --diff-opt --accepted --diff-opt "$D/acc.txt"
  --diff-opt --mustpass --diff-opt "$D/must.txt")
run() { bash "$TOOL" "${OPTS[@]}" "$@" 2>&1; }
last_is() { tail -n 1 <<<"$1" | grep -qx -- "$2"; }

mkdir -p "$D/s1" "$D/s2"
printf "deqp: case filter '%s.*'\n13:00:01 deqp: Fail %s.a\n13:00:02 deqp: Pass %s.b\n" $P $P $P >"$D/s1/shard-0000.log"
printf "deqp: case filter '%s.*'\n13:00:03 deqp: Fail %s.c\n13:00:04 deqp: Pass %s.d\n" $P $P $P >"$D/s2/shard-0001.log"
LOGS=("$D/s1/shard-0000.log" "$D/s2/shard-0001.log")
LINE='mp2-p9-diff: shared 1, new 1, fixed 0, missing 0'

printf '# P9\n\n`%s` (slice 2).\n' "$LINE" >"$D/eq.md"
out=$(run --notes "$D/eq.md" "${LOGS[@]}"); rc=$?
chk "equal: ok" last_is "$out" 'mp2-p9-c3: ok'
chk "equal: exit 0" test $rc -eq 0
chk "equal: shows the diff line" grep -qx "diff:   $LINE" <<<"$out"

printf '`mp2-p9-diff: shared 0, new 1, fixed 0, missing 0` (slice 1).\n\n`%s` (slice 2).\n' "$LINE" >"$D/two.md"
out=$(run --notes "$D/two.md" "${LOGS[@]}"); rc=$?
chk "two quoted, last current: ok" test $rc -eq 0

printf '`%s` (slice 2).\n\n`mp2-p9-diff: shared 0, new 1, fixed 0, missing 0` (slice 1).\n' "$LINE" >"$D/stale.md"
out=$(run --notes "$D/stale.md" "${LOGS[@]}"); rc=$?
chk "last quoted stale: failed" last_is "$out" "mp2-p9-c3: failed: .* quotes 'mp2-p9-diff: shared 0, new 1, fixed 0, missing 0' but the shard logs give '$LINE'"
chk "last quoted stale: exit 1" test $rc -eq 1

out=$(run --notes "$D/eq.md" "$D/s1/shard-0000.log"); rc=$?
chk "a slice left out: failed" test $rc -eq 1

printf '# P9\n\nno line yet\n' >"$D/none.md"
out=$(run --notes "$D/none.md" "${LOGS[@]}"); rc=$?
chk "no quote: failed" last_is "$out" "mp2-p9-c3: failed: no 'mp2-p9-diff: shared ...' line quoted in .*"
chk "no quote: exit 1" test $rc -eq 1

# missing: d has no result in this slice
printf "deqp: case filter '%s.*'\n13:00:03 deqp: Fail %s.c\n" $P $P >"$D/s2/miss.log"
MLINE='mp2-p9-diff: shared 1, new 1, fixed 0, missing 1'
printf '`%s`\n' "$MLINE" >"$D/miss.md"
out=$(run --notes "$D/miss.md" "$D/s1/shard-0000.log" "$D/s2/miss.log"); rc=$?
chk "missing, no cause: failed" last_is "$out" "mp2-p9-c3: failed: missing 1: 1 case(s) without .*, first $P.d"
chk "missing, no cause: exit 1" test $rc -eq 1
printf '`%s`\n\n- `%s.d`: lost to the watchdog reset (cause: shard 1 hung).\n' "$MLINE" $P >"$D/cause.md"
out=$(run --notes "$D/cause.md" "$D/s1/shard-0000.log" "$D/s2/miss.log"); rc=$?
chk "missing with its cause: ok" test $rc -eq 0
printf '`%s`\n\n- `%s.d` was lost.\n' "$MLINE" $P >"$D/named.md"
out=$(run --notes "$D/named.md" "$D/s1/shard-0000.log" "$D/s2/miss.log"); rc=$?
chk "missing named without 'cause': failed" test $rc -eq 1

# ", excluded E": the whole line is compared
printf '%s.d\tQ11\n' $P >"$D/excl.txt"
XLINE='mp2-p9-diff: shared 1, new 1, fixed 0, missing 0, excluded 1'
printf '`%s`\n' "$XLINE" >"$D/ex.md"
out=$(run --diff-opt --exclude --diff-opt "$D/excl.txt" --notes "$D/ex.md" "$D/s1/shard-0000.log" "$D/s2/miss.log"); rc=$?
chk "excluded form: ok" test $rc -eq 0
out=$(run --notes "$D/ex.md" "${LOGS[@]}"); rc=$?
chk "excluded quoted, diff without: failed" test $rc -eq 1

# no log: a glob that matched nothing, or an unreadable file
out=$(run --notes "$D/eq.md" "$D/nonexistent.log"); rc=$?
chk "unreadable log: exit 1" test $rc -eq 1
out=$(bash "$TOOL" --notes "$D/nonexistent.md" "${LOGS[@]}" 2>&1); rc=$?
chk "unreadable notes: exit 2" test $rc -eq 2
out=$(bash "$TOOL" --bogus 2>&1); rc=$?
chk "unknown option: exit 2" test $rc -eq 2
out=$(bash "$TOOL" --notes 2>&1); rc=$?
chk "--notes without a file: exit 2" test $rc -eq 2

# the defaults run against the tree (logs and note in the repository)
out=$(bash "$TOOL" 2>&1); rc=$?
chk "defaults: a verdict line" last_is "$out" 'mp2-p9-c3: \(ok\|failed: .*\)'

echo "mp2_p9_c3_check_selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
