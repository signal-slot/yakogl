#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of mp2-case-times.py: synthetic logs, no board.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2ct.XXXXXX")
trap 'rm -rf "$T"' EXIT
PY="python3 -I"
TOOL="$HERE/mp2-case-times.py"
fail=0
ck() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: got '$2' want '$3'"; fail=1; fi; }

# console.txt format (whole seconds); t.c has no result, the last line is cut
printf '%s\n' \
    'runner noise without a stamp' \
    "10:00:00 deqp: case filter 't.*'" \
    '10:00:03 deqp: Pass t.a' \
    '10:00:03 deqp: Fail t.b' \
    '10:00:09 deqp: Pass t.d' \
    '10:00:10 deqp: Pass t.cu' > "$T/hms.log"
printf '10:00:11 deqp: Pass t.cut' >> "$T/hms.log"
# console.ts format (epoch, fractions); t.a repeats, this log is later: it wins
printf '%s\n' \
    "1790000000.000 deqp: case filter 'x'" \
    '1790000000.250 deqp: Pass t.a' \
    '1790000001.500 deqp: Pass t.e' > "$T/ts.log"
: > "$T/empty.log"
printf 't.a\nt.b\nt.c\nt.d\nt.e\nt.a\n' > "$T/cases.txt"

out=$($PY "$TOOL" "$T/hms.log" "$T/ts.log" "$T/empty.log" --cases "$T/cases.txt" \
    --times-out "$T/times.tsv" --missing-out "$T/missing.txt"); rc=$?
ck rc "$rc" 0
ck summary "$out" "mp2-case-times: cases 5, timed 5, missing 1, max_ms 6000"
ck missing "$(cat "$T/missing.txt")" "t.c"
ck times "$(grep -v '^#' "$T/times.tsv" | sort | tr '\t\n' ' ')" \
    "t.a 250 t.b 100 t.cu 1000 t.d 6000 t.e 1250 "

# midnight wrap, floor override
printf '%s\n' '23:59:59 deqp: case filter x' '00:00:01 deqp: Pass m.a' '00:00:01 deqp: Pass m.b' > "$T/mid.log"
$PY "$TOOL" "$T/mid.log" --times-out "$T/mid.tsv" --floor-ms 7 > "$T/mid.out"; rc=$?
ck mid-rc "$rc" 0
ck mid "$(grep -v '^#' "$T/mid.tsv" | tr '\t\n' ' ')" "m.a 2000 m.b 7 "
ck mid-sum "$(cat "$T/mid.out")" "mp2-case-times: cases 2, timed 2, missing 0, max_ms 2000"

# an empty log alone: every case missing, in --cases order
$PY "$TOOL" "$T/empty.log" --cases "$T/cases.txt" --missing-out "$T/m2.txt" > "$T/o2"; rc=$?
ck empty-rc "$rc" 0
ck empty "$(cat "$T/o2")" "mp2-case-times: cases 5, timed 0, missing 5, max_ms 0"
ck empty-order "$(tr '\n' ' ' < "$T/m2.txt")" "t.a t.b t.c t.d t.e "

# the table is accepted by mp2-deqp-shards.py --times (a+b 350, d 6000, e 1250 ms under a 6300 budget: 3 shards)
$PY "$TOOL" "$T/hms.log" "$T/ts.log" --times-out "$T/t2.tsv" > /dev/null
printf 't.a\nt.b\nt.d\nt.e\n' > "$T/tiny.txt"
$PY "$HERE/mp2-deqp-shards.py" "$T/tiny.txt" --out "$T/sh" --times "$T/t2.tsv" --budget-ms 6300 > "$T/sh.out" 2>&1
ck shards-rc "$?" 0
ck shards-n "$(ls "$T"/sh/shard-*.txt | wc -l)" 3

# usage and input errors: exit 2
$PY "$TOOL" > /dev/null 2>&1; ck no-args "$?" 2
$PY "$TOOL" "$T/nonexistent.log" > /dev/null 2>&1; ck missing-log "$?" 2
$PY "$TOOL" "$T/hms.log" --cases "$T/nonexistent" > /dev/null 2>&1; ck missing-cases "$?" 2
$PY "$TOOL" "$T/hms.log" --bogus > /dev/null 2>&1; ck bad-option "$?" 2

[ "$fail" = 0 ] && echo "mp2_case_times_selftest: all ok"
exit "$fail"
