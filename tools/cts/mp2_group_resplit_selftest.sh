#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mp2_group_resplit_selftest.sh
#
# Host-only check of tools/cts/mp2-group-resplit.py on fixtures built inline
# under $TMPDIR (or the current directory): a group under the lifetime stays
# whole; one that overruns is cut and every shard stays under the lifetime; a
# hung case goes alone and the cases after it still get a shard; an empty
# missing list changes nothing; a manifest input is re-split per shard; the
# same input twice gives the same bytes.
set -u

HERE=$(dirname "$0")
TOOL=$HERE/mp2-group-resplit.py
D=$(mktemp -d "${TMPDIR:-.}/mp2-resplit.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }
chk() { local l=$1; shift; if "$@"; then ok "$l"; else bad "$l"; fi; }
run() { python3 -I "$TOOL" "$@"; }
last() { tail -n 1 "$1"; }

# --- fixtures: group g, cases c1..c6, 30 s each (180 s, overruns 100 s)
F=$D/fx
mkdir -p "$F"
{
    echo "# group g"
    for i in 1 2 3 4 5 6; do echo "dEQP-GLES2.g.c$i"; done
    echo ""
    echo "dEQP-GLES2.g.c1"
} >"$F/g.txt"
printf '# name<TAB>ms\n' >"$F/times.tsv"
for i in 1 2 3 4 5 6; do printf 'dEQP-GLES2.g.c%d\t30000\n' "$i" >>"$F/times.tsv"; done
printf 'dEQP-GLES2.s.a\ndEQP-GLES2.s.b\n' >"$F/small.txt"
: >"$F/empty-missing.txt"
# c3 hung: c3..c6 got no result
printf 'dEQP-GLES2.g.c%d\n' 3 4 5 6 >"$F/missing.txt"

# --- --help
chk "--help exits 0" eval 'run --help >/dev/null'

# --- a group under the lifetime stays whole
run "$F/small.txt" --out "$D/small" >"$D/small.out"
chk "small: one shard" test "$(($(wc -l <"$D/small/manifest.tsv") - 1))" = 1
chk "small: shard is the input" cmp -s "$F/small.txt" "$D/small/small.1.txt"
chk "small: last line" test "$(last "$D/small.out")" = "mp2-group-resplit: shards 1 cases 2 alone 0"

# --- a group that overruns is cut (3 x 30 s = 90 s fits, a 4th does not)
run "$F/g.txt" --out "$D/cut" --times "$F/times.tsv" >"$D/cut.out"
chk "cut: last line" test "$(last "$D/cut.out")" = "mp2-group-resplit: shards 2 cases 6 alone 0"
chk "cut: g.1 holds c1..c3" test "$(cat "$D/cut/g.1.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 1 2 3)"
chk "cut: g.2 holds c4..c6" test "$(cat "$D/cut/g.2.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 4 5 6)"
chk "cut: every shard under 100000 ms" \
    test -z "$(awk -F'\t' 'NR > 1 && $3 > 100000' "$D/cut/manifest.tsv")"
chk "cut: case order kept, each case once" \
    test "$(cat "$D"/cut/g.1.txt "$D"/cut/g.2.txt)" = "$(printf 'dEQP-GLES2.g.c%d\n' 1 2 3 4 5 6)"
# overhead counts once per shard: 15 s + 30 s x 2 = 75 s, a third case is 105 s
run "$F/g.txt" --out "$D/oh" --times "$F/times.tsv" --overhead-ms 15000 >"$D/oh.out"
chk "overhead: 3 shards of 2" test "$(last "$D/oh.out")" = "mp2-group-resplit: shards 3 cases 6 alone 0"
# default ms: no times, 100 ms each, lifetime 0.25 s -> 2 per shard
run "$F/g.txt" --out "$D/def" --lifetime 0.25 >"$D/def.out"
chk "default-ms: 3 shards" test "$(last "$D/def.out")" = "mp2-group-resplit: shards 3 cases 6 alone 0"

# --- a hung case goes alone and the cases after it still get a shard
run "$F/g.txt" --out "$D/hung" --times "$F/times.tsv" --missing "$F/missing.txt" >"$D/hung.out"
chk "hung: last line" test "$(last "$D/hung.out")" = "mp2-group-resplit: shards 3 cases 6 alone 1"
chk "hung: g.1 holds c1, c2" test "$(cat "$D/hung/g.1.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 1 2)"
chk "hung: g.2 is c3 alone" test "$(cat "$D/hung/g.2.txt")" = "dEQP-GLES2.g.c3"
chk "hung: g.3 holds c4..c6" test "$(cat "$D/hung/g.3.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 4 5 6)"

# --- a case over the lifetime alone gets its own shard, with a warning
printf 'dEQP-GLES2.g.c2\t200000\n' >"$F/long.tsv"
run "$F/g.txt" --out "$D/long" --times "$F/long.tsv" --lifetime 1 >"$D/long.out" 2>"$D/long.err"
chk "over: c2 alone" test "$(cat "$D/long/g.2.txt")" = "dEQP-GLES2.g.c2"
chk "over: warned" grep -q 'over the lifetime' "$D/long.err"
chk "over: last line" test "$(last "$D/long.out")" = "mp2-group-resplit: shards 3 cases 6 alone 1"

# --- an empty missing list changes nothing
run "$F/g.txt" --out "$D/em" --times "$F/times.tsv" --missing "$F/empty-missing.txt" >"$D/em.out"
chk "empty missing: same stdout" cmp -s "$D/cut.out" "$D/em.out"
chk "empty missing: same files" diff -r "$D/cut" "$D/em"

# --- a manifest input (mp2-deqp-shards.py format) is re-split per shard
M=$D/man
mkdir -p "$M"
cp "$F/g.txt" "$M/shard-0000.txt"
cp "$F/small.txt" "$M/shard-0001.txt"
printf 'index\tcases\test_ms\tfile\n0\t6\t600\tshard-0000.txt\n1\t2\t200\tshard-0001.txt\n' >"$M/manifest.tsv"
run "$M/manifest.tsv" --out "$D/mo" --times "$F/times.tsv" --missing "$F/missing.txt" >"$D/mo.out"
chk "manifest: last line" test "$(last "$D/mo.out")" = "mp2-group-resplit: shards 4 cases 8 alone 1"
chk "manifest: index column" \
    test "$(awk -F'\t' 'NR > 1 { printf "%s %s;", $1, $4 }' "$D/mo/manifest.tsv")" = \
    "0000.1 shard-0000.1.txt;0000.2 shard-0000.2.txt;0000.3 shard-0000.3.txt;0001.1 shard-0001.1.txt;"
chk "manifest: --group refused" eval '! run "$M/manifest.tsv" --out "$D/mg" --group x 2>/dev/null'

# --- the same input twice gives identical output
run "$F/g.txt" --out "$D/twice" --times "$F/times.tsv" --missing "$F/missing.txt" >"$D/twice.out"
chk "deterministic: same stdout" cmp -s "$D/hung.out" "$D/twice.out"
chk "deterministic: same files" diff -r "$D/hung" "$D/twice"

# --- bad input is an error
printf 'dEQP-GLES2.g.c1\tfast\n' >"$F/bad.tsv"
chk "bad times: exit 1" eval '! run "$F/g.txt" --out "$D/bad" --times "$F/bad.tsv" 2>/dev/null'

echo "mp2_group_resplit_selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
