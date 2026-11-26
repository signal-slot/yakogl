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

# opt-in --status, --checklist, --roadmap (README at dfabc888, scores 468/195/42)
P=pass-README.md R=pass-RESULTS.md
case_ "--status: sha8 and the three scores" $P $R 0 '^ok:' --ref none --status --status-file "$D/status-pass.md"
case_ "--status: line names another commit" $P $R 1 'no line mentions glmark2 and names .dfabc888' --ref none --status --status-file "$D/status-nosha.md"
case_ "--status: Pi 4 score differs from RESULTS" $P $R 1 'lacks the RESULTS score of Raspberry Pi 4' --ref none --status --status-file "$D/status-score.md"
case_ "--checklist: row 9 closed" $P $R 0 '^ok:' --ref none --checklist --checklist-file "$D/checklist-pass.md"
case_ "--checklist: row 9 left to #1196" $P $R 1 "row 9 still says" --ref none --checklist --checklist-file "$D/checklist-fail.md"
case_ "--roadmap: commit, #17, no Pi 4 by value only" $P $R 0 '^ok:' --ref none --roadmap --roadmap-file "$D/roadmap-pass.md"
case_ "--roadmap: commit not named" $P $R 1 'does not name .dfabc888' --ref none --roadmap --roadmap-file "$D/roadmap-nosha.md"
case_ "--roadmap: #17 not named" $P $R 1 'does not name #17' --ref none --roadmap --roadmap-file "$D/roadmap-no17.md"
case_ "--roadmap: Pi 4 pictures by value only" $P $R 1 'Pi 4 pictures are checked by value only' --ref none --roadmap --roadmap-file "$D/roadmap-pi4value.md"
echo "all cases passed"
