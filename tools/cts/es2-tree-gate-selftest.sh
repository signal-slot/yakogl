#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/es2-tree-gate-selftest.sh
#
# Host-only check of tools/cts/es2-tree-gate.py on synthetic tree records under
# $TMPDIR: a Pi 4 form record (one console per row) that passes; the same with
# one Fail, one Crash, one mustpass case without a result, one NotSupported
# case that is not accepted, and an accepted one that passed instead; an
# i.MX8MP selector form record (results.tsv and consoles) that passes, one
# whose results.tsv disagrees with its console and one whose row has no
# console; and a directory that is no record (exit 2). Each must give the exit
# status and the fields named here.
set -u

HERE=$(dirname "$0")
GATE=$HERE/es2-tree-gate.py
D=$(mktemp -d "${TMPDIR:-.}/es2-tree-gate.XXXXXX") || exit 2
trap 'rm -rf "$D"' EXIT
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

P=dEQP-GLES2.functional
printf '%s\n' "$P.clip_control.errors  # not claimed" "$P.multisample.depth  # no multisample surface" > "$D/accepted.txt"
printf '%s\n' "$P.a.one" "$P.a.two" "$P.b.one" "$P.b.two" "$P.clip_control.errors" "$P.multisample.depth" > "$D/mustpass.txt"

# row DIR NAME STATUS_AND_CASE...: a run directory whose console holds the
# "deqp: <Status> <case>" lines and the image's own done/summary lines
row() {
    local rec=$1 name=$2 d n=0 s c line
    shift 2
    d=$rec/20261112-1200-$name-abcd1234
    mkdir -p "$d"
    {
        echo "12:00:00 deqp: case filter '$P.*'"
        for line in "$@"; do
            s=${line%% *}
            c=${line#* }
            echo "12:00:01 deqp: $s $c"
            n=$((n + 1))
        done
        echo "12:00:09 deqp: done: $n passed, 0 failed, 0 not supported, 0 warnings, complete=1"
        echo "12:00:09 deqp: summary: $n cases begun, $n with a result, 0 without"
    } > "$d/console.txt"
}

# pi4_rec DIR LINES...: image A passes, image B holds LINES (the Pi 4 form: tree.txt, a run per image)
pi4_rec() {
    local rec=$1
    shift
    mkdir -p "$rec"
    printf 'A\t-\t-\t%s.a.*\nB\t-\t-\t%s.b.*\n' "$P" "$P" > "$rec/tree.txt"
    row "$rec" A "Pass $P.a.one" "Pass $P.a.two"
    row "$rec" B "$@"
}
NS1="NotSupported $P.clip_control.errors"
NS2="NotSupported $P.multisample.depth"

# run NAME WANT_RC REGEX... DIR
run() {
    local name=$1 want=$2 rec out rc pat
    shift 2
    rec=${!#}
    out=$(python3 "$GATE" --accepted "$D/accepted.txt" --mustpass "$D/mustpass.txt" "$rec" 2>&1)
    rc=$?
    if [ "$rc" != "$want" ]; then
        bad "$name: exit $rc, wanted $want"
        printf '%s\n' "$out" | sed 's/^/        /'
        return
    fi
    for pat in "${@:1:$#-1}"; do
        if ! grep -qE -- "$pat" <<<"$out"; then
            bad "$name: no line matching '$pat'"
            printf '%s\n' "$out" | sed 's/^/        /'
            return
        fi
    done
    ok "$name"
}

pi4_rec "$D/pass" "Pass $P.b.one" "Pass $P.b.two" "$NS1" "$NS2"
run "pass: exit 0, all fields" 0 '^Fail 0$' '^NoResult 0$' '^crashes 0$' '^NotSupported 2$' \
    '^NotSupported names: all in es2-accepted.txt$' '^stage1: PASS$' "$D/pass"

pi4_rec "$D/fail" "Pass $P.b.one" "Fail $P.b.two" "$NS1" "$NS2"
run "Fail: exit 1" 1 '^Fail 1$' '^stage1: FAIL \(.*Fail 1' "$D/fail"

pi4_rec "$D/crash" "Pass $P.b.one" "Crash $P.b.two" "$NS1" "$NS2"
run "crash: exit 1" 1 '^crashes 1$' '^stage1: FAIL \(.*crashes 1' "$D/crash"

pi4_rec "$D/noresult" "Pass $P.b.one" "$NS1" "$NS2"
run "NoResult: exit 1" 1 '^NoResult 1$' '^stage1: FAIL \(.*NoResult 1' "$D/noresult"

pi4_rec "$D/ns" "Pass $P.b.one" "NotSupported $P.b.two" "$NS1" "$NS2"
run "unaccepted NotSupported: exit 1" 1 '^NotSupported 3$' "^NotSupported names: not in es2-accepted.txt: $P.b.two$" \
    '^stage1: FAIL' "$D/ns"

pi4_rec "$D/passed" "Pass $P.b.one" "Pass $P.b.two" "Pass $P.clip_control.errors" "$NS2"
run "an accepted case that passed: exit 1" 1 '^NotSupported 1$' '^stage1: FAIL \(.*accepted name' "$D/passed"

# i.MX8MP selector form: one table image, rows e00 and e01 (selectors 0 and 1), results.tsv beside the consoles.
# imx_rec DIR FAILED_COUNT_OF_ROW_1 (the Fail column of results.tsv's second row)
imx_rec() {
    local rec=$1
    mkdir -p "$rec"
    printf 'e00\timg.bin\t0\t%s.a.*\ne01\timg.bin\t1\t%s.b.*\n' "$P" "$P" > "$rec/tree.txt"
    row "$rec" e00 "Pass $P.a.one" "Pass $P.a.two"
    row "$rec" e01 "Pass $P.b.one" "Pass $P.b.two" "$NS1" "$NS2"
    printf '# index\tfilter\tbegun\tpassed\tfailed\tnotsup\tother\tnoresult\tfailing names\n' > "$rec/results.tsv"
    printf '0\t%s.a.*\t2\t2\t0\t0\t0\t0\t-\n' "$P" >> "$rec/results.tsv"
    printf '1\t%s.b.*\t4\t2\t%s\t2\t0\t0\t-\n' "$P" "$2" >> "$rec/results.tsv"
}
imx_rec "$D/imx" 0
run "i.MX8MP selector form: exit 0" 0 '^Fail 0$' '^NotSupported 2$' '^stage1: PASS$' "$D/imx"
imx_rec "$D/imx-bad" 1
run "i.MX8MP results.tsv disagrees with the console: exit 1" 1 '^stage1: FAIL \(.*results.tsv says Fail 1' "$D/imx-bad"

# the row's console is gone: results.tsv numbers alone cannot prove the NotSupported names
imx_rec "$D/imx-nocon" 0
rm -r "$D"/imx-nocon/*-e01-*
run "i.MX8MP row without a console: names not checkable, exit 1" 1 '^NotSupported 2$' \
    '^stage1: FAIL \(.*names not checkable' "$D/imx-nocon"

mkdir -p "$D/empty"
run "no record: exit 2" 2 "$D/empty"

echo "es2-tree-gate-selftest: $pass ok, $fail failed"
[ "$fail" = 0 ]
