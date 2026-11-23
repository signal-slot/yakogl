#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_gltest_shards_selftest.sh
#
# Host-only check of tools/cts/mp2-gltest-shards.py: on the fixtures in
# testdata/mp2-gltest-shards/ every case is in exactly one shard, in the
# expected order, within the budget, with the expected selection text; on the
# in-tree gl_test.c every case of its case_names[] table (less 7) is covered
# exactly once. The last line is "N ok, M failed".
set -u

HERE=$(dirname "$0")
TOOL=$HERE/mp2-gltest-shards.py
FX=$HERE/testdata/mp2-gltest-shards
SRC=$HERE/../../driver/tools/pvrlab/gl_test.c
D=$(mktemp -d "${TMPDIR:-.}/mp2-gltest-shards.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
run() { python3 -I "$TOOL" "$@"; }
rows() { tail -n +2 "$1/manifest.tsv" | cut -f1-3 | tr '\t\n' ',;'; }

# --- fixture source: 7 left out, the commented [99] ignored, set order first
run --source "$FX/gl_test_mini.c" --runner "$FX/runner_mini.rs" --out "$D/a" \
    --times "$FX/times.tsv" --budget-ms 1500 --overhead-ms 100 --default-ms 250 >"$D/a.out"
chk "fixture: manifest rows" test "$(rows "$D/a")" = "0,3,1300;1,5,1350;2,2,600;"
cat "$D"/a/shard-*.txt | tr '\n' ' ' >"$D/a.all"
chk "fixture: every case once, set order then number" test "$(cat "$D/a.all")" = "1 2 3 8 10 11 0 4 5 25 "
chk "fixture: manifest header" test "$(head -1 "$D/a/manifest.tsv")" = "$(printf 'index\tcases\test_ms\tfile')"
chk "fixture: first shard is a SET_ORDER prefix -> SET_HEAD" \
    grep -q 'shard 0000: exact: CONFIG_GL_TEST_MP2_SET_HEAD=y CONFIG_GL_TEST_MP2_SET_HEAD_CASES=3$' "$D/a/selections.txt"
chk "fixture: other shard not selectable, names the change" \
    grep -q 'shard 0001: not selectable today.*CONFIG_GL_TEST_MP2_CASES="8,10,11,0,4"' "$D/a/selections.txt"
chk "fixture: selections also on stdout" grep -q 'shard 0002: not selectable' "$D/a.out"
chk "fixture: no shard over budget" test "$(tail -n +2 "$D/a/manifest.tsv" | awk -F'\t' '$3 > 1500' | wc -l)" = 0

# --- number order, excluded cases, one case per shard -> CONFIG_GL_TEST_MP2_CASE
run --source "$FX/gl_test_mini.c" --runner "$FX/runner_mini.rs" --out "$D/n" --order number \
    --exclude 25,10 --budget-ms 400 --overhead-ms 100 --default-ms 250 >/dev/null
cat "$D"/n/shard-*.txt | tr '\n' ' ' >"$D/n.all"
chk "number order: ascending, excluded out" test "$(cat "$D/n.all")" = "0 1 2 3 4 5 8 11 "
chk "number order: one case per shard" test "$(tail -n +2 "$D/n/manifest.tsv" | cut -f2 | sort -u | tr '\n' ,)" = "1,"
chk "number order: single case -> GL_TEST_MP2_CASE" grep -q 'shard 0007: exact: CONFIG_GL_TEST_MP2_CASE=11$' "$D/n/selections.txt"

# --- a list file: numbers, a name after the number allowed, a repeat kept once
run --list "$FX/list.txt" --runner "$FX/runner_mini.rs" --out "$D/l" --budget-ms 15000 >/dev/null
cat "$D"/l/shard-*.txt | tr '\n' ' ' >"$D/l.all"
chk "list file: 3,5,9 once each, set order first" test "$(cat "$D/l.all")" = "3 5 9 "

# --- determinism
run --source "$FX/gl_test_mini.c" --runner "$FX/runner_mini.rs" --out "$D/b" \
    --times "$FX/times.tsv" --budget-ms 1500 --overhead-ms 100 --default-ms 250 >/dev/null
chk "deterministic: second run byte-identical" diff -r "$D/a" "$D/b" >/dev/null

# --- errors write nothing
run --source "$FX/gl_test_mini.c" --out "$D/h" --times "$FX/huge-times.tsv" --budget-ms 1000 2>/dev/null
chk "oversize case: exit 1" test "$?" = 1
chk "oversize case: nothing written" test ! -e "$D/h"
printf '1 400\n' >"$D/bad.tsv"
run --source "$FX/gl_test_mini.c" --out "$D/m" --times "$D/bad.tsv" 2>/dev/null
chk "malformed time table: exit 1" test "$?" = 1
printf 'int x;\n' >"$D/nocases.c"
run --source "$D/nocases.c" --out "$D/s" 2>/dev/null
chk "source without a case table: exit 1" test "$?" = 1

# --- the in-tree gl_test.c: every case once
run --source "$SRC" --out "$D/r" --times "$FX/times.tsv" >"$D/r.out"
# independent count: the [N] = "name" entries of the table, less case 7
want=$(awk '/case_names\[\] = \{/{t=1;next} t&&/^};/{exit} t' "$SRC" | grep -o '\[[0-9]*\] = "' | tr -d '[] ="' | grep -vx 7 | sort -n)
got=$(cat "$D"/r/shard-*.txt | sort -n)
chk "gl_test.c: every case exactly once" test "$got" = "$want"
chk "gl_test.c: no case twice" test "$(cat "$D"/r/shard-*.txt | sort | uniq -d | wc -l)" = 0
chk "gl_test.c: more than 200 cases" test "$(printf '%s\n' "$got" | wc -l)" -gt 200
chk "gl_test.c: no shard over 15000 ms" test "$(tail -n +2 "$D/r/manifest.tsv" | awk -F'\t' '$3 > 15000' | wc -l)" = 0
chk "gl_test.c: manifest counts match the files" \
    test "$(tail -n +2 "$D/r/manifest.tsv" | awk -F'\t' '{s+=$2} END{print s}')" = "$(printf '%s\n' "$got" | wc -l)"
chk "gl_test.c: first shard is exact SET_HEAD" grep -q '^shard 0000: exact: CONFIG_GL_TEST_MP2_SET_HEAD=y' "$D/r/selections.txt"

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
