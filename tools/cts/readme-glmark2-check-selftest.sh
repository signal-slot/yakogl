#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Selftest of readme-glmark2-check.py: the one-commit verdict and --scenes
# (figures '<scene> <n> ms' and a dated record dir carrying the commit's sha8).
# Fixtures: testdata/readme-glmark2-scenes/. Needs python3 only; stops at the
# first wrong verdict.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
CHK=$HERE/readme-glmark2-check.py
D=$HERE/testdata/readme-glmark2-scenes
SC=terrain,shadow,refract

# case_ <name> <readme> <results> <want rc> <word> [extra args...]
case_() {
    local name=$1 readme=$2 results=$3 want=$4 word=$5 out rc
    shift 5
    out=$(python3 -I "$CHK" --readme "$D/$readme" --results "$D/$results" "$@" 2>&1)
    rc=$?
    if [ "$rc" -eq "$want" ] && printf '%s\n' "$out" | grep -q -- "$word"; then
        echo "ok: $name (exit $rc)"
    else
        echo "FAIL: $name: exit $rc (want $want), output lacks '$word'"
        printf '%s\n' "$out" | sed 's/^/    | /'
        exit 1
    fi
}

case_ "one commit, all scenes, dated record dirs" pass-README.md pass-RESULTS.md 0 '^ok:' --ref none --scenes $SC
case_ "no --scenes: the figures are not read" pass-README.md noshadow-RESULTS.md 0 '^ok:' --ref none
case_ "README mixes two commits" mixed-README.md pass-RESULTS.md 1 'name 2 commits' --ref none --scenes $SC
case_ "row lacks 'shadow <n> ms'" pass-README.md noshadow-RESULTS.md 1 "Raspberry Pi 4 glmark2 row has no .shadow <n> ms" --ref none --scenes $SC
case_ "'about 34.5 ms a frame' is no terrain figure" pass-README.md about-RESULTS.md 1 "Raspberry Pi 4 glmark2 row has no .terrain <n> ms" --ref none --scenes $SC
case_ "record dir carries another sha8" pass-README.md othersha-RESULTS.md 1 'record directories (87881c78) are not at' --ref none --scenes $SC
case_ "record dir has no date" pass-README.md nodate-RESULTS.md 1 'Raspberry Pi 4 glmark2 row cites no record directory' --ref none --scenes $SC
echo "all cases passed"
