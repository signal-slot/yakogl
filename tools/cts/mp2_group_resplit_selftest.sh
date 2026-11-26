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
# same input twice gives the same bytes; the output manifest is in
# mp2-deqp-shards.py's format (a plain-number index, shard-NNNN.txt) and goes
# back in as an input; a shard's ','-joined filter fits the case slot
# (--slot-bytes), and the defaults are the dEQP image's lifetime (its DK conf)
# and slot room (deqp/Kconfig).
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
chk "small: shard is the input" cmp -s "$F/small.txt" "$D/small/shard-0000.txt"
chk "small: last line" test "$(last "$D/small.out")" = "mp2-group-resplit: shards 1 cases 2 alone 0"

# --- a group that overruns is cut (3 x 30 s = 90 s fits, a 4th does not)
run "$F/g.txt" --out "$D/cut" --times "$F/times.tsv" --lifetime 100 >"$D/cut.out"
chk "cut: last line" test "$(last "$D/cut.out")" = "mp2-group-resplit: shards 2 cases 6 alone 0"
chk "cut: shard 0 holds c1..c3" test "$(cat "$D/cut/shard-0000.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 1 2 3)"
chk "cut: shard 1 holds c4..c6" test "$(cat "$D/cut/shard-0001.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 4 5 6)"
chk "cut: every shard under 100000 ms" \
    test -z "$(awk -F'\t' 'NR > 1 && $3 > 100000' "$D/cut/manifest.tsv")"
chk "cut: case order kept, each case once" \
    test "$(cat "$D"/cut/shard-0000.txt "$D"/cut/shard-0001.txt)" = "$(printf 'dEQP-GLES2.g.c%d\n' 1 2 3 4 5 6)"
# overhead counts once per shard: 15 s + 30 s x 2 = 75 s, a third case is 105 s
run "$F/g.txt" --out "$D/oh" --times "$F/times.tsv" --lifetime 100 --overhead-ms 15000 >"$D/oh.out"
chk "overhead: 3 shards of 2" test "$(last "$D/oh.out")" = "mp2-group-resplit: shards 3 cases 6 alone 0"
# default ms: no times, 100 ms each, lifetime 0.25 s -> 2 per shard
run "$F/g.txt" --out "$D/def" --lifetime 0.25 >"$D/def.out"
chk "default-ms: 3 shards" test "$(last "$D/def.out")" = "mp2-group-resplit: shards 3 cases 6 alone 0"

# --- a hung case goes alone and the cases after it still get a shard
run "$F/g.txt" --out "$D/hung" --times "$F/times.tsv" --lifetime 100 --missing "$F/missing.txt" >"$D/hung.out"
chk "hung: last line" test "$(last "$D/hung.out")" = "mp2-group-resplit: shards 3 cases 6 alone 1"
chk "hung: shard 0 holds c1, c2" test "$(cat "$D/hung/shard-0000.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 1 2)"
chk "hung: shard 1 is c3 alone" test "$(cat "$D/hung/shard-0001.txt")" = "dEQP-GLES2.g.c3"
chk "hung: shard 2 holds c4..c6" test "$(cat "$D/hung/shard-0002.txt")" = "$(printf 'dEQP-GLES2.g.c%d\n' 4 5 6)"

# --- a case over the lifetime alone gets its own shard, with a warning
printf 'dEQP-GLES2.g.c2\t200000\n' >"$F/long.tsv"
run "$F/g.txt" --out "$D/long" --times "$F/long.tsv" --lifetime 1 >"$D/long.out" 2>"$D/long.err"
chk "over: c2 alone" test "$(cat "$D/long/shard-0001.txt")" = "dEQP-GLES2.g.c2"
chk "over: warned" grep -q 'over the lifetime' "$D/long.err"
chk "over: last line" test "$(last "$D/long.out")" = "mp2-group-resplit: shards 3 cases 6 alone 1"

# --- an empty missing list changes nothing
run "$F/g.txt" --out "$D/em" --times "$F/times.tsv" --lifetime 100 --missing "$F/empty-missing.txt" >"$D/em.out"
chk "empty missing: same stdout" cmp -s "$D/cut.out" "$D/em.out"
chk "empty missing: same files" diff -r "$D/cut" "$D/em"

# --- a manifest input (mp2-deqp-shards.py format) is re-split per shard
M=$D/man
mkdir -p "$M"
cp "$F/g.txt" "$M/shard-0000.txt"
cp "$F/small.txt" "$M/shard-0001.txt"
printf 'index\tcases\test_ms\tfile\n0\t6\t600\tshard-0000.txt\n1\t2\t200\tshard-0001.txt\n' >"$M/manifest.tsv"
run "$M/manifest.tsv" --out "$D/mo" --times "$F/times.tsv" --lifetime 100 --missing "$F/missing.txt" >"$D/mo.out"
chk "manifest: last line" test "$(last "$D/mo.out")" = "mp2-group-resplit: shards 4 cases 8 alone 1"
chk "manifest: index and file columns" \
    test "$(awk -F'\t' 'NR > 1 { printf "%s %s;", $1, $4 }' "$D/mo/manifest.tsv")" = \
    "0 shard-0000.txt;1 shard-0001.txt;2 shard-0002.txt;3 shard-0003.txt;"
chk "manifest: origin.tsv names the input shard and why" \
    test "$(awk -F'\t' 'NR > 1 { printf "%s %s %s;", $1, $2, $3 }' "$D/mo/origin.tsv")" = \
    "shard-0000.txt shard-0000 fit;shard-0001.txt shard-0000 hung;shard-0002.txt shard-0000 fit;shard-0003.txt shard-0001 fit;"
# mp2-deqp-sweep.sh reads the index as a number: every index is a plain one
chk "manifest: every index is a plain number" \
    test -z "$(awk -F'\t' 'NR > 1 && $1 !~ /^[0-9]+$/' "$D/mo/manifest.tsv")"
# --first-index numbers past the shards already run
run "$M/manifest.tsv" --out "$D/fi" --times "$F/times.tsv" --lifetime 100 --missing "$F/missing.txt" \
    --first-index 40 >"$D/fi.out"
chk "first-index: files from shard-0040" \
    test "$(awk -F'\t' 'NR > 1 { printf "%s %s;", $1, $4 }' "$D/fi/manifest.tsv")" = \
    "40 shard-0040.txt;41 shard-0041.txt;42 shard-0042.txt;43 shard-0043.txt;"
chk "first-index: same cases" cmp -s "$D/mo/shard-0003.txt" "$D/fi/shard-0043.txt"
chk "first-index: negative refused" eval '! run "$F/g.txt" --out "$D/neg" --first-index -1 2>/dev/null'
# the output is a manifest: a shard that overruns again is re-split from it
run "$D/mo/manifest.tsv" --out "$D/again" --lifetime 0.25 >"$D/again.out"
chk "again: the output manifest is an input" \
    test "$(last "$D/again.out")" = "mp2-group-resplit: shards 5 cases 8 alone 0"
chk "again: case order kept" \
    test "$(cat "$D"/again/shard-000[0-4].txt)" = "$(cat "$D"/mo/shard-000[0-3].txt)"
chk "manifest: --group refused" eval '! run "$M/manifest.tsv" --out "$D/mg" --group x 2>/dev/null'

# --- the same input twice gives identical output
run "$F/g.txt" --out "$D/twice" --times "$F/times.tsv" --lifetime 100 --missing "$F/missing.txt" >"$D/twice.out"
chk "deterministic: same stdout" cmp -s "$D/hung.out" "$D/twice.out"
chk "deterministic: same files" diff -r "$D/hung" "$D/twice"

# --- bad input is an error
printf 'dEQP-GLES2.g.c1\tfast\n' >"$F/bad.tsv"
chk "bad times: exit 1" eval '! run "$F/g.txt" --out "$D/bad" --times "$F/bad.tsv" 2>/dev/null'

# --- the case slot: a shard's names joined with ',' plus a NUL fit --slot-bytes
# (each name is 15 bytes: two make 31 + NUL = 32, three make 48)
run "$F/g.txt" --out "$D/slot" --slot-bytes 40 >"$D/slot.out"
chk "slot 40: 3 shards of 2" test "$(last "$D/slot.out")" = "mp2-group-resplit: shards 3 cases 6 alone 0"
chk "slot 40: every filter fits" test -z "$(for s in "$D"/slot/shard-*.txt; do
    n=$(paste -sd, "$s" | tr -d '\n' | wc -c); [ $((n + 1)) -le 40 ] || echo "$s"; done)"
run "$F/g.txt" --out "$D/slot32" --slot-bytes 32 >"$D/slot32.out"
chk "slot 32: an exact fit is kept" test "$(last "$D/slot32.out")" = "mp2-group-resplit: shards 3 cases 6 alone 0"
run "$F/g.txt" --out "$D/slot31" --slot-bytes 31 >"$D/slot31.out"
chk "slot 31: one name a shard" test "$(last "$D/slot31.out")" = "mp2-group-resplit: shards 6 cases 6 alone 0"
chk "slot 15: a name over the slot is exit 1" eval '! run "$F/g.txt" --out "$D/slot15" --slot-bytes 15 2>/dev/null'
chk "slot 15: nothing written" test ! -e "$D/slot15"
# the default slot holds 16384 bytes: 1,200 names of 25 bytes at 1 ms each fit
# the lifetime but not one slot (1,200 x 26 - 1 = 31,199 bytes; 630 a shard)
for i in $(seq 1000 2199); do echo "dEQP-GLES2.slot.case_$i"; done >"$F/many.txt"
run "$F/many.txt" --out "$D/many" --default-ms 1 >"$D/many.out"
chk "default slot: 1200 names cut in 2" test "$(last "$D/many.out")" = "mp2-group-resplit: shards 2 cases 1200 alone 0"
chk "default slot: shard 0 fits 16384 bytes" \
    test $(($(paste -sd, "$D/many/shard-0000.txt" | tr -d '\n' | wc -c) + 1)) -le 16384

# --- the defaults are the dEQP image's: its lifetime and its slot room
CONF=$HERE/../../driver/samples/zephyr/deqp/boards/stm32mp257f_dk_stm32mp257fxx_a35.conf
KCONFIG=$HERE/../../driver/samples/zephyr/deqp/Kconfig
conf_life=$(sed -n 's/^CONFIG_YAKOGL_STM32MP2_LIFETIME_S=//p' "$CONF")
tool_life=$(sed -n 's/.*"--lifetime", type=float, default=\([0-9]*\)\.0,.*/\1/p' "$TOOL")
chk "default lifetime is the dEQP conf's ($conf_life s)" test -n "$conf_life" -a "$conf_life" = "$tool_life"
kc_slot=$(awk '/^config DEQP_CASE_SLOT_SIZE/ { f = 1 } f && $1 == "default" { print $2; exit }' "$KCONFIG")
tool_slot=$(sed -n 's/.*"--slot-bytes", type=int, default=\([0-9]*\),.*/\1/p' "$TOOL")
chk "default slot is the Kconfig default ($kc_slot)" test -n "$kc_slot" -a "$kc_slot" = "$tool_slot"
chk "the DK conf keeps the Kconfig slot size" eval '! grep -q "^CONFIG_DEQP_CASE_SLOT_SIZE=" "$CONF"'

echo "mp2_group_resplit_selftest: $pass ok, $fail failed"
[ "$fail" -eq 0 ]
