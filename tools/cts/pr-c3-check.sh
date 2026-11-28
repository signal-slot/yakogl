#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-c3-check.sh [LOGDIR [RESULTS]]
#
# Mechanical check of acceptance c3 of the partial-render plan (issue #3):
# the regression set (gl_test default list, glmark2 common set) on the AM62P
# is unchanged with CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER on and off, and
# the AM62P rows of RESULTS.md cite those runs. LOGDIR defaults to
# tools/cts/log, RESULTS to tools/cts/RESULTS.md.
#
# (a) The record is the newest LOGDIR/*-pr-regress directory (names start
#     with the date, so the last in sort order) that holds all four legs.
#     tools/cts/pr-session.sh writes one knob state per run as
#     <date>-pr-<name>/gltest.log and glmark2.log, so a regress session keeps
#     the two runs side by side in one of two layouts:
#         gltest-off.log gltest-on.log glmark2-off.log glmark2-on.log
#     or  off/gltest.log off/glmark2.log on/gltest.log on/glmark2.log
#     (the flat names win when both exist).
# (b) tools/cts/pr-regress-compare.py compares knob off (run1) with knob on
#     (run2), gl_test and glmark2 separately. From its per-line output: the
#     set of gl_test cases with PASS must be the same, and the set of glmark2
#     scenes that complete (an FPS number) must be the same. Scores and FPS
#     may differ (compare's own SAME/DIFFERENT counts those; it is not used).
# (c) From compare's '== resets' lines: the knob-on leg must not have more
#     GPU resets, a higher HWR counter, or more 'render: stopped' lines than
#     the knob-off leg, in either pair.
# (d) In RESULTS.md's '## Common set' table, an AM62P row's glmark2 cell and
#     an AM62P row's gl_test cell (columns found by the header) each cite the
#     record directory's name.
#
# Prints one line, 'pr-c3-check: OK <dir>: ...' (exit 0) or
# 'pr-c3-check: FAIL <reason>' (exit 1). No *-pr-regress directory at all:
# 'pr-c3-check: FAIL no pr-regress records'.
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
LOGDIR=${1:-$HERE/log}
RESULTS=${2:-$HERE/RESULTS.md}
TOOL=$HERE/pr-regress-compare.py

fail() { echo "pr-c3-check: FAIL $1"; exit 1; }

[ -f "$TOOL" ] || fail "missing $TOOL"

# legs DIR: sets GT_OFF GT_ON GM_OFF GM_ON to the leg logs that exist
legs() {
    local d=$1 l k v
    GT_OFF='' GT_ON='' GM_OFF='' GM_ON=''
    for l in gltest glmark2; do
        for k in off on; do
            v=
            if [ -f "$d/$l-$k.log" ]; then
                v=$d/$l-$k.log
            elif [ -f "$d/$k/$l.log" ]; then
                v=$d/$k/$l.log
            fi
            case $l-$k in
            gltest-off) GT_OFF=$v ;;
            gltest-on) GT_ON=$v ;;
            glmark2-off) GM_OFF=$v ;;
            glmark2-on) GM_ON=$v ;;
            esac
        done
    done
}
# missing_legs: the names of the legs legs() did not find
missing_legs() {
    local m=
    [ -n "$GT_OFF" ] || m+=" gl_test knob-off,"
    [ -n "$GT_ON" ] || m+=" gl_test knob-on,"
    [ -n "$GM_OFF" ] || m+=" glmark2 knob-off,"
    [ -n "$GM_ON" ] || m+=" glmark2 knob-on,"
    m=${m# }
    echo "${m%,}"
}

dirs=()
if [ -d "$LOGDIR" ]; then
    while IFS= read -r d; do
        [ -n "$d" ] && dirs+=("$d")
    done < <(LC_ALL=C find "$LOGDIR" -mindepth 1 -maxdepth 1 -type d -name '*-pr-regress' | LC_ALL=C sort -r)
fi
[ ${#dirs[@]} -gt 0 ] || fail "no pr-regress records"

REC=
for d in "${dirs[@]}"; do
    legs "$d"
    if [ -n "$GT_OFF" ] && [ -n "$GT_ON" ] && [ -n "$GM_OFF" ] && [ -n "$GM_ON" ]; then
        REC=$d
        break
    fi
done
if [ -z "$REC" ]; then
    legs "${dirs[0]}"
    fail "no complete pr-regress record: newest $(basename "${dirs[0]}") lacks $(missing_legs) log"
fi
NAME=$(basename "$REC")

# compare WHAT OFF ON: run pr-regress-compare.py, its output in OUT
compare() {
    local rc
    OUT=$(python3 -I "$TOOL" "$2" "$3" 2>&1)
    rc=$?
    [ "$rc" -le 1 ] || fail "$NAME: pr-regress-compare.py could not read the $1 logs: $(printf '%s\n' "$OUT" | tail -1)"
}

# resets WHAT: (c) on compare's '== resets' lines in OUT
resets() {
    local re='^run([12]): GPU recoveries ([0-9]+) \(HWR counter ([0-9]+)\), reset lines ([0-9]+), render: stopped ([0-9]+)$'
    local line r1='' h1='' s1='' r2='' h2='' s2='' why=''
    while IFS= read -r line; do
        [[ $line =~ $re ]] || continue
        if [ "${BASH_REMATCH[1]}" = 1 ]; then
            r1=${BASH_REMATCH[4]} h1=${BASH_REMATCH[3]} s1=${BASH_REMATCH[5]}
        else
            r2=${BASH_REMATCH[4]} h2=${BASH_REMATCH[3]} s2=${BASH_REMATCH[5]}
        fi
    done <<< "$OUT"
    if [ -z "$r1" ] || [ -z "$r2" ]; then fail "$NAME: no reset lines in pr-regress-compare.py's $1 output"; fi
    [ "$r2" -le "$r1" ] || why+=", GPU resets $r2 (knob off $r1)"
    [ "$h2" -le "$h1" ] || why+=", HWR counter $h2 (knob off $h1)"
    [ "$s2" -le "$s1" ] || why+=", 'render: stopped' lines $s2 (knob off $s1)"
    [ -z "$why" ] || fail "$NAME: new reset on the knob-on $1 leg:${why#,}"
    RESETS+=" $1 $r1/$r2"
}

RESETS=

# (b) gl_test: the PASS sets
compare gl_test "$GT_OFF" "$GT_ON"
re='^case +([0-9]+) +run1=([A-Za-z]+) run2=([A-Za-z]+)'
npass=0 ncase=0 diff=
while IFS= read -r line; do
    [[ $line =~ $re ]] || continue
    ncase=$((ncase + 1))
    a=${BASH_REMATCH[2]} b=${BASH_REMATCH[3]}
    if [ "$a" = PASS ] && [ "$b" = PASS ]; then
        npass=$((npass + 1))
    elif [ "$a" = PASS ] || [ "$b" = PASS ]; then
        diff+=" ${BASH_REMATCH[1]} ($a/$b)"
    fi
done <<< "$OUT"
[ "$ncase" -gt 0 ] || fail "$NAME: no gl_test case lines in the gl_test logs"
[ -z "$diff" ] || fail "$NAME: gl_test pass sets differ (knob off/on): case${diff}"
resets gl_test

# (b) glmark2: the scenes that complete
compare glmark2 "$GM_OFF" "$GM_ON"
re='^(\[[^]]+\] .*): run1=(.*) run2=(.*)$'
nscene=0 ndone=0 diff=
in_gm=0
while IFS= read -r line; do
    case $line in
    "== glmark2") in_gm=1; continue ;;
    "== "*) in_gm=0; continue ;;
    esac
    [ "$in_gm" = 1 ] || continue
    [[ $line =~ $re ]] || continue
    nscene=$((nscene + 1))
    s=${BASH_REMATCH[1]} a=${BASH_REMATCH[2]} b=${BASH_REMATCH[3]%  DIFFERS}
    ca=0 cb=0
    [[ $a =~ ^[0-9]+$ ]] && ca=1
    [[ $b =~ ^[0-9]+$ ]] && cb=1
    if [ "$ca" = 1 ] && [ "$cb" = 1 ]; then
        ndone=$((ndone + 1))
    elif [ "$ca" != "$cb" ]; then
        diff+="; $s ($a/$b)"
    fi
done <<< "$OUT"
[ "$nscene" -gt 0 ] || fail "$NAME: no glmark2 scene results in the glmark2 logs"
[ -z "$diff" ] || fail "$NAME: glmark2 completed scenes differ (knob off/on): ${diff#; }"
resets glmark2

# (d) RESULTS.md: the AM62P rows of the Common set table cite the record
[ -f "$RESULTS" ] || fail "missing $RESULTS"
cites=$(awk -v name="$NAME" '
    function trim(s) { gsub(/^[ \t]+|[ \t]+$/, "", s); return s }
    /^## / { sec = ($0 ~ /^## Common set/); gm = gt = 0; next }
    !sec || !/^\|/ { next }
    {
        n = split($0, c, "|")
        if (!gm && !gt) {            # the header row
            for (i = 2; i < n; i++) {
                h = trim(c[i])
                if (h ~ /^glmark2/) gm = i
                if (h ~ /^gl_test/) gt = i
            }
            next
        }
        if (trim(c[2]) !~ /^AM62P/) next
        if (gm && index(c[gm], name)) cgm = 1
        if (gt && index(c[gt], name)) cgt = 1
    }
    END { printf "%s%s", (cgm ? "" : " glmark2"), (cgt ? "" : " gl_test") }
' "$RESULTS")
if [ -n "$cites" ]; then
    m=${cites# }
    fail "$NAME: RESULTS.md's AM62P ${m/ / and } cell does not cite $NAME"
fi

echo "pr-c3-check: OK $NAME: gl_test $npass of $ncase cases PASS in both legs, glmark2 $ndone of $nscene scenes complete in both, no new resets (off/on:$RESETS), cited by RESULTS.md's AM62P glmark2 and gl_test cells"
