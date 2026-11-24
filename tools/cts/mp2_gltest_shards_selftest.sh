#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_gltest_shards_selftest.sh
#
# Host-only check of tools/cts/mp2-gltest-shards.py: on a small runner
# fixture and on the real SET_ORDER every case (minus the skips) appears once,
# in order, no shard is over budget, and two runs give the same bytes.
set -u

HERE=$(dirname "$0")
TOOL=$HERE/mp2-gltest-shards.py
RUNNER=$HERE/../../driver/tools/pvrlab/runner/src/lib.rs
D=$(mktemp -d "${TMPDIR:-.}/mp2-gltest.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
run() { python3 -I "$TOOL" "$@"; }

printf 'pub static SET_ORDER: [i32; 6] = [\n    1, 2, 3, 10, 22, 11,\n];\n' >"$D/lib.rs"
printf '# case\tms\n1\t400\n2\t400\n3\t400\n22\t300\n11\t900\n' >"$D/times.tsv"

run --runner "$D/lib.rs" --out "$D/a" --times "$D/times.tsv" --budget-ms 1000 --default-ms 100 >/dev/null
chk "fixture: skip 10, shards 1,2 | 3,22 | 11" test "$(tail -n +2 "$D/a/manifest.tsv" | cut -f4 | tr '\n' ';')" = "1,2;3,22;11;"
chk "fixture: estimates" test "$(tail -n +2 "$D/a/manifest.tsv" | cut -f3 | tr '\n' ,)" = "800,700,900,"
cat "$D"/a/shard-*.txt >"$D/a.all"
chk "fixture: cases once, in order" test "$(tr '\n' ' ' <"$D/a.all")" = "1 2 3 22 11 "
run --runner "$D/lib.rs" --skip "" --out "$D/s" --budget-ms 100000 >/dev/null
chk "fixture: --skip '' keeps case 10" test "$(cut -f4 "$D/s/manifest.tsv" | tail -n 1)" = "1,2,3,10,22,11"
run --runner "$D/lib.rs" --out "$D/b" --times "$D/times.tsv" --budget-ms 1000 --default-ms 100 >/dev/null
chk "deterministic" diff -r "$D/a" "$D/b"
run --runner "$D/lib.rs" --out "$D/o" --times "$D/times.tsv" --budget-ms 1000 --overhead-ms 50 >/dev/null
chk "overhead: estimates include it, none over budget" test "$(tail -n +2 "$D/o/manifest.tsv" | cut -f3 | tr '\n' ,)" = "850,750,950,"

run --runner "$D/lib.rs" --out "$D/h" --times "$D/times.tsv" --budget-ms 500 2>/dev/null
chk "oversize case: exit 1" test "$?" = 1
chk "oversize case: nothing written" test ! -e "$D/h"
printf '1 400\n' >"$D/bad.tsv"
run --runner "$D/lib.rs" --out "$D/m" --times "$D/bad.tsv" 2>/dev/null
chk "malformed time table: exit 1" test "$?" = 1
printf 'nothing\n' >"$D/none.rs"
run --runner "$D/none.rs" --out "$D/n" 2>/dev/null
chk "no SET_ORDER: exit 1" test "$?" = 1

# the real runner
run --out "$D/r" --default-ms 500 >/dev/null
want=$(sed -n '/SET_ORDER: \[i32; 50\]/,/^\];/p' "$RUNNER" | sed 1d | grep -o '[0-9]\+' | grep -vx 10 | tr '\n' ' ')
cat "$D"/r/shard-*.txt | tr '\n' ' ' >"$D/r.all"
chk "real set: 49 cases (50 minus case 10)" test "$(wc -w <"$D/r.all")" = 49
chk "real set: no case twice" test "$(tr ' ' '\n' <"$D/r.all" | grep . | sort | uniq -d | wc -l)" = 0
chk "real set: order matches lib.rs" test "$(cat "$D/r.all")" = "$want"
chk "real set: no shard over 15000 ms" test "$(tail -n +2 "$D/r/manifest.tsv" | awk -F'\t' '$3 > 15000' | wc -l)" = 0

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
