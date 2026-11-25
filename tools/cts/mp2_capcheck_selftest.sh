#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Host selftest of mp2-capcheck.py: synthetic console captures, no board.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=$(mktemp -d "${TMPDIR:-/tmp}/mp2cc.XXXXXX")
trap 'rm -rf "$T"' EXIT
PY="python3 -I"
TOOL="$HERE/mp2-capcheck.py"
fail=0
ck() { if [ "$2" = "$3" ]; then echo "ok   $1"; else echo "FAIL $1: got '$2' want '$3'"; fail=1; fi; }
has() { if printf '%s\n' "$2" | grep -qxF -- "$3"; then echo "ok   $1"; else echo "FAIL $1: no line '$3' in:"; printf '%s\n' "$2"; fail=1; fi; }

G=dEQP-GLES2.functional
printf '%s\n' '# shard 7' "$G.a.one" "$G.a.two" "$G.a.three" "$G.b.four" > "$T/shard.txt"
printf '%s\n' "$G.a.one" "$G.a.two" "$G.a.three" "$G.b.four" "$G.c.five" > "$T/list.txt"

head_lines() {
    printf '%s\n' 'U-Boot noise' "10:00:00 deqp: case filter '$G.a.*,$G.b.*'"
}
body_lines() {
    printf '%s\n' \
        "10:00:01 deqp: Pass $G.a.one" \
        "10:00:02 deqp: Fail $G.a.two" \
        "10:00:02 deqp: NotSupported $G.a.three" \
        "10:00:03 deqp: Pass $G.b.four"
}
tail_lines() {
    printf '%s\n' \
        '10:00:04 deqp: done: 2 passed, 1 failed, 1 not supported, 0 warnings, complete=1' \
        '10:00:04 deqp: summary: 4 cases begun, 4 with a result, 0 without' \
        '10:00:04 deqp: FAIL'
}

# 1. clean capture: ok, by --cases and by --list with the capture's own filter
{ head_lines; body_lines; tail_lines; } > "$T/clean.txt"
out=$($PY "$TOOL" "$T/clean.txt" --cases "$T/shard.txt"); rc=$?
ck "clean rc" "$rc" 0
ck "clean last" "$(printf '%s\n' "$out" | tail -n1)" "capcheck: ok"
has "clean tally" "$out" "capcheck: 4 shard case(s), 4 result line(s): 2 Pass, 1 failed, 1 NotSupported, 0 warnings"
out=$($PY "$TOOL" "$T/clean.txt" --list "$T/list.txt"); rc=$?
ck "clean list rc" "$rc" 0
has "clean list filter" "$out" "capcheck: filter '$G.a.*,$G.b.*' selects 4 of 5 listed case(s)"
out=$($PY "$TOOL" "$T/clean.txt" --list "$T/list.txt" --filter "$G.*"); rc=$?
ck "wide filter rc" "$rc" 1
has "wide filter missing" "$out" "capcheck: missing $G.c.five"

# 2. a repeated block (two copied ranges overlap by three lines, CRs too): dropped, ok
{ head_lines; body_lines | head -n3; body_lines | sed -n 1,3p; body_lines | tail -n1; tail_lines; } \
    | sed 's/$/\r/' > "$T/repeat.txt"
out=$($PY "$TOOL" "$T/repeat.txt" --cases "$T/shard.txt" --clean-out "$T/repeat.clean"); rc=$?
ck "repeat rc" "$rc" 0
has "repeat dropped" "$out" "capcheck: 12 line(s) read, 3 boundary-repeat line(s) dropped"
ck "repeat clean-out" "$(cat "$T/repeat.clean")" "$(cat "$T/clean.txt")"
# a repeat that is not adjacent is a duplicate, not a boundary repeat
{ head_lines; body_lines; body_lines | head -n1 | sed 's/^10:00:01/10:00:05/'; tail_lines; } > "$T/dup.txt"
out=$($PY "$TOOL" "$T/dup.txt" --cases "$T/shard.txt"); rc=$?
ck "dup rc" "$rc" 1
has "dup name" "$out" "capcheck: duplicate $G.a.one (2 lines: Pass Pass)"
has "dup done" "$out" "capcheck: done says 2 passed, 1 failed, 1 not supported, 0 warnings; the capture has 3, 1, 1, 0"
has "dup count" "$out" "capcheck: 3 problem(s)"

# 3. a lost line (a gap between two copied ranges): missing, counts disagree
{ head_lines; body_lines | grep -v a.three; tail_lines; } > "$T/lost.txt"
out=$($PY "$TOOL" "$T/lost.txt" --cases "$T/shard.txt"); rc=$?
ck "lost rc" "$rc" 1
has "lost name" "$out" "capcheck: missing $G.a.three"
has "lost summary" "$out" "capcheck: summary says 4 with a result, the capture has 3 result line(s)"
has "lost done" "$out" "capcheck: done says 2 passed, 1 failed, 1 not supported, 0 warnings; the capture has 2, 1, 0, 0"
has "lost count" "$out" "capcheck: 3 problem(s)"

# 4. summary mismatch: the image began 5, one without a result; plus an unexpected name
{ head_lines; body_lines; echo "10:00:03 deqp: Pass $G.c.five"; tail_lines | sed \
    -e 's/4 cases begun, 4 with a result, 0 without/6 cases begun, 5 with a result, 1 without/' \
    -e 's/2 passed/3 passed/'; } > "$T/summ.txt"
out=$($PY "$TOOL" "$T/summ.txt" --cases "$T/shard.txt"); rc=$?
ck "summary rc" "$rc" 1
has "summary unexpected" "$out" "capcheck: unexpected $G.c.five"
has "summary without" "$out" "capcheck: summary says 1 case(s) began without a result"
has "summary count" "$out" "capcheck: 2 problem(s)"
ck "summary last" "$(printf '%s\n' "$out" | tail -n1)" "capcheck: 2 problem(s)"
# a cut tail: no summary, no done
{ head_lines; body_lines; } > "$T/cut.txt"
out=$($PY "$TOOL" "$T/cut.txt" --cases "$T/shard.txt"); rc=$?
ck "cut rc" "$rc" 1
has "cut summary" "$out" "capcheck: no 'deqp: summary' line"
has "cut done" "$out" "capcheck: no 'deqp: done' line"

# bad arguments
$PY "$TOOL" "$T/none.txt" --cases "$T/shard.txt" >/dev/null 2>&1; ck "missing file rc" "$?" 2
$PY "$TOOL" "$T/cut.txt" --list "$T/list.txt" --filter '' >/dev/null 2>&1; ck "empty filter rc" "$?" 2

if [ "$fail" = 0 ]; then echo "mp2_capcheck_selftest: all ok"; else echo "mp2_capcheck_selftest: FAILED"; fi
exit "$fail"
