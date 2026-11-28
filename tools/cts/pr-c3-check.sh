#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c3-check.sh [LOGDIR [RESULTS]]
#
# Mechanical check of acceptance c3 of the partial-render plan (issue #3):
# the AM62P regression set (gl_test default list, glmark2 common set) is
# unchanged with CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER on and off.
# LOGDIR defaults to tools/cts/log, RESULTS to tools/cts/RESULTS.md.
#
# A record is a directory LOGDIR/<date>-pr-regress/ holding the four serial
# logs of the two legs:  gltest-off.log gltest-on.log glmark2-off.log
# glmark2-on.log  (knob off = the baseline, knob on). The check
#   (a) takes the newest (by name) *-pr-regress directory that has all four;
#   (b) compares each pair with tools/cts/pr-regress-compare.py (the parser is
#       not duplicated): the same gl_test cases have the same verdict, and the
#       same glmark2 scenes complete (a score differing in value is no
#       difference; a scene present, absent or "Set up failed" in one leg is);
#   (c) fails when the knob-on leg shows more GPU recoveries, HWR count, reset
#       lines or 'render: stopped' lines than the knob-off leg of the pair;
#   (d) fails unless the AM62P gl_test row and the AM62P glmark2 row of RESULTS
#       (table lines starting '| AM62P' that mention gl_test / glmark2) cite
#       that directory by name.
# Prints one line, 'pr-c3-check: OK <dir>' (exit 0) or 'pr-c3-check: FAIL
# <reason>' (exit 1). With no record at all: 'pr-c3-check: FAIL no pr-regress
# records'. Exit 2: LOGDIR, RESULTS or pr-regress-compare.py missing.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
LOGDIR=${1:-$HERE/log}
RESULTS=${2:-$HERE/RESULTS.md}
TOOL=$HERE/pr-regress-compare.py

[ -d "$LOGDIR" ] || { echo "pr-c3-check: missing $LOGDIR" >&2; exit 2; }
[ -f "$RESULTS" ] || { echo "pr-c3-check: missing $RESULTS" >&2; exit 2; }
[ -f "$TOOL" ] || { echo "pr-c3-check: missing $TOOL" >&2; exit 2; }

fail() { echo "pr-c3-check: FAIL $*"; exit 1; }

# (a) newest directory with all four logs; remember the newest incomplete one
dir=
partial=
for d in "$LOGDIR"/*-pr-regress; do
    [ -d "$d" ] || continue
    n=$(basename "$d")
    all=1
    for f in gltest-off gltest-on glmark2-off glmark2-on; do
        [ -f "$d/$f.log" ] || all=0
    done
    if [ "$all" = 1 ]; then
        dir=$n
    else
        partial=$n
    fi
done
if [ -z "$dir" ]; then
    if [ -n "$partial" ]; then
        missing=
        for f in gltest-off gltest-on glmark2-off glmark2-on; do
            [ -f "$LOGDIR/$partial/$f.log" ] || missing="$missing $f.log"
        done
        fail "$partial lacks$missing (need a knob-off and a knob-on gl_test and glmark2 log)"
    fi
    fail "no pr-regress records"
fi
rec=$LOGDIR/$dir

# (b) and (c) per pair
for k in gltest glmark2; do
    out=$(python3 -I "$TOOL" "$rec/$k-off.log" "$rec/$k-on.log" 2>&1)
    rc=$?
    [ "$rc" -le 1 ] || fail "$dir: pr-regress-compare.py failed on $k ($out)"
    if [ "$k" = gltest ]; then
        printf '%s\n' "$out" | grep -q '^case ' || fail "$dir: gltest logs hold no gl_test case line"
        printf '%s\n' "$out" | grep -q '^cases differing: 0 of ' ||
            fail "$dir: the gl_test pass sets differ ($(printf '%s\n' "$out" | grep 'DIFFERS' | tr -s ' ' | paste -sd';' | cut -c1-200))"
    else
        printf '%s\n' "$out" | grep -Eq '^\[.*run1=[0-9]+' || fail "$dir: glmark2 logs hold no completed scene"
        # a scene completes when its value is a number; compare the two legs
        diffs=$(printf '%s\n' "$out" | grep -E '^\[' | sed -E 's/^(.*): run1=([^ ]+) run2=([^ ]+).*$/\2 \3 \1/' |
            awk '{ a = ($1 ~ /^[0-9]+$/); b = ($2 ~ /^[0-9]+$/); if (a != b) { $1 = $2 = ""; print } }')
        [ -z "$diffs" ] || fail "$dir: the glmark2 scenes that complete differ:$(printf '%s' "$diffs" | paste -sd';')"
    fi
    off=$(printf '%s\n' "$out" | grep '^run1: GPU recoveries')
    on=$(printf '%s\n' "$out" | grep '^run2: GPU recoveries')
    read -r roff hoff loff soff <<< "$(printf '%s\n' "$off" | sed -E 's/.*recoveries ([0-9]+) \(HWR counter ([0-9]+)\), reset lines ([0-9]+), render: stopped ([0-9]+).*/\1 \2 \3 \4/')"
    read -r ron hon lon son <<< "$(printf '%s\n' "$on" | sed -E 's/.*recoveries ([0-9]+) \(HWR counter ([0-9]+)\), reset lines ([0-9]+), render: stopped ([0-9]+).*/\1 \2 \3 \4/')"
    [[ ${roff:-x}${hoff:-x}${loff:-x}${soff:-x}${ron:-x}${hon:-x}${lon:-x}${son:-x} =~ ^[0-9]+$ ]] ||
        fail "$dir: no reset line in the $k comparison"
    if [ "$ron" -gt "$roff" ] || [ "$hon" -gt "$hoff" ] || [ "$lon" -gt "$loff" ]; then
        fail "$dir: the knob-on $k leg shows GPU recoveries $ron (HWR $hon, reset lines $lon), the knob-off leg $roff (HWR $hoff, reset lines $loff)"
    fi
    if [ "$son" -gt "$soff" ]; then
        fail "$dir: the knob-on $k leg shows $son 'render: stopped' lines, the knob-off leg $soff"
    fi
done

# (d) RESULTS rows cite the directory
for k in gl_test glmark2; do
    grep -E '^\|[[:space:]]*AM62P' "$RESULTS" | grep -F "$k" | grep -Fq "$dir/" ||
        fail "$dir: no AM62P $k row of $(basename "$RESULTS") cites tools/cts/log/$dir/"
done

echo "pr-c3-check: OK $dir"
