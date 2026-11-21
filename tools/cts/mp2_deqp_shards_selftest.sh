#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_deqp_shards_selftest.sh
#
# Host-only check of tools/cts/mp2-deqp-shards.py: on the small fixtures in
# testdata/mp2-deqp-shards/ every case appears exactly once, every shard stays
# within the budget, the manifest matches the files, and two runs give the
# same bytes; on the real gles2-mustpass.txt every case is covered once.
set -u

HERE=$(dirname "$0")
TOOL=$HERE/mp2-deqp-shards.py
FX=$HERE/testdata/mp2-deqp-shards
D=$(mktemp -d "${TMPDIR:-.}/mp2-shards.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
run() { python3 -I "$TOOL" "$@"; }

# unique input cases of a list: comments and blanks out, repeats out, order kept
uniq_cases() { grep -v '^[[:space:]]*\(#\|$\)' "$1" | awk '!s[$0]++'; }

# --- fixtures, budget 1000
run "$FX/cases.txt" --out "$D/a" --times "$FX/times.tsv" --budget-ms 1000 >/dev/null
chk "fixture: manifest has 4 shards" test "$(($(wc -l <"$D/a/manifest.tsv") - 1))" = 4
cat "$D"/a/shard-*.txt >"$D/a.all"
uniq_cases "$FX/cases.txt" >"$D/a.want"
chk "fixture: every case once, in input order" cmp -s "$D/a.all" "$D/a.want"
chk "fixture: the repeated case is kept once" test "$(grep -c 'a\.two$' "$D/a.all")" = 1
chk "fixture: manifest rows" test "$(tail -n +2 "$D/a/manifest.tsv" | cut -f1-3 | tr '\t\n' ',;')" = "0,2,900;1,1,300;2,2,1000;3,1,100;"
over=$(tail -n +2 "$D/a/manifest.tsv" | awk -F'\t' '$3 > 1000' | wc -l)
chk "fixture: no shard over budget" test "$over" = 0
n=0; mok=1
while IFS=$'\t' read -r idx cnt ms file; do
    [ "$idx" = index ] && continue
    [ "$(wc -l <"$D/a/$file")" = "$cnt" ] || mok=0
    n=$((n + cnt))
done <"$D/a/manifest.tsv"
chk "fixture: manifest counts match the files" test "$mok" = 1 -a "$n" = "$(wc -l <"$D/a.want")"

# overhead counts against the budget (100 ms per shard; four fits alone at 1000)
run "$FX/cases.txt" --out "$D/o" --times "$FX/times.tsv" --budget-ms 1000 --overhead-ms 100 >/dev/null
chk "overhead: no shard over budget" test "$(tail -n +2 "$D/o/manifest.tsv" | awk -F'\t' '$3 > 1000' | wc -l)" = 0
cat "$D"/o/shard-*.txt >"$D/o.all"
chk "overhead: every case once" cmp -s "$D/o.all" "$D/a.want"

# no time table: --default-ms for all, 3 cases of 400 ms fit 1200
run "$FX/cases.txt" --out "$D/d" --budget-ms 1200 --default-ms 400 >/dev/null
chk "default-ms: 6 cases of 400 ms in 2 shards of 3" test "$(tail -n +2 "$D/d/manifest.tsv" | cut -f2 | tr '\n' ,)" = "3,3,"

# determinism
run "$FX/cases.txt" --out "$D/b" --times "$FX/times.tsv" --budget-ms 1000 >/dev/null
chk "deterministic: second run byte-identical" diff -r "$D/a" "$D/b" >/dev/null

# a case that cannot fit alone fails and writes nothing
run "$FX/cases.txt" --out "$D/h" --times "$FX/huge-times.tsv" --budget-ms 1000 2>/dev/null
rc=$?
chk "oversize case: exit 1" test "$rc" = 1
chk "oversize case: nothing written" test ! -e "$D/h"

# a malformed time table is refused
printf 'dEQP-GLES2.a.one 400\n' >"$D/bad.tsv"
run "$FX/cases.txt" --out "$D/m" --times "$D/bad.tsv" 2>/dev/null
chk "malformed time table: exit 1" test "$?" = 1

# the real mustpass, default budget
MP=$HERE/gles2-mustpass.txt
run "$MP" --out "$D/r" >/dev/null
cat "$D"/r/shard-*.txt >"$D/r.all"
uniq_cases "$MP" >"$D/r.want"
chk "mustpass: every case exactly once" cmp -s "$D/r.all" "$D/r.want"
chk "mustpass: no case twice" test "$(sort "$D/r.all" | uniq -d | wc -l)" = 0
chk "mustpass: no shard over 15000 ms" test "$(tail -n +2 "$D/r/manifest.tsv" | awk -F'\t' '$3 > 15000' | wc -l)" = 0

printf '%d ok, %d failed\n' "$pass" "$fail"
[ "$fail" = 0 ]
