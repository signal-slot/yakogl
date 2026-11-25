#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of readme-glmark2-check.py --verdicts on its fixtures
# (testdata/readme-glmark2/verdicts-*-RESULTS.md with pass-README.md), in the
# style of cts-selftest.sh section 27. Each case: RESULTS fixture, extra
# options, expected exit status and a word the output must contain. Host
# only; no board, no git (--ref none). Exit 0 when every case passes.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
RG=$HERE/readme-glmark2-check.py
RGD=$HERE/testdata/readme-glmark2
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

vcase() {
    local name=$1 results=$2 extra=$3 want=$4 word=$5 out rc
    # shellcheck disable=SC2086 # $extra is a word list on purpose
    out=$(python3 -I "$RG" --readme "$RGD/pass-README.md" --results "$RGD/$results" \
          --ref none $extra 2>&1)
    rc=$?
    if [ "$rc" -eq "$want" ] && printf '%s\n' "$out" | grep -q -- "$word"; then
        ok "$name: exit $rc, output has '$word'"
    else
        bad "$name: exit $rc (want $want), output lacks '$word'"
        printf '%s\n' "$out" | sed 's/^/        | /'
    fi
}
# a case whose output must NOT contain a word (the one item it breaks is the
# only one reported)
vnot() {
    local name=$1 results=$2 word=$3 out
    out=$(python3 -I "$RG" --readme "$RGD/pass-README.md" --results "$RGD/$results" \
          --ref none --verdicts 2>&1)
    if printf '%s\n' "$out" | grep -q -- "$word"; then
        bad "$name: output has '$word'"
        printf '%s\n' "$out" | sed 's/^/        | /'
    else
        ok "$name: output lacks '$word'"
    fi
}

echo "== readme-glmark2-check.py --verdicts (testdata/readme-glmark2)"
# passing: every row states all items, in the three wordings of the grammar
vcase "all three rows state their verdicts" \
    verdicts-pass-RESULTS.md --verdicts 0 '^ok:'
vcase "with --scenes too, both checks run (no ms figures in this fixture)" \
    verdicts-pass-RESULTS.md "--verdicts --scenes terrain" 1 'AM62P glmark2 row has no .terrain <n> ms'
# off by default: every verdicts fixture passes without --verdicts
for f in pass noscenes nofailed noglerrors nooom noterrain norefract notable; do
    vcase "without --verdicts: verdicts-$f passes" \
        "verdicts-$f-RESULTS.md" "" 0 '^ok:'
done
# failing: one item missing per fixture
vcase "'33 scenes' missing (32 of 33)" \
    verdicts-noscenes-RESULTS.md --verdicts 1 "Raspberry Pi 4 glmark2 row does not state '33 scenes'"
vcase "'0 failed' missing (10 failed is not 0 failed)" \
    verdicts-nofailed-RESULTS.md --verdicts 1 "AM62P glmark2 row does not state '0 failed'"
vcase "'0 GL errors' missing" \
    verdicts-noglerrors-RESULTS.md --verdicts 1 "i.MX8MP glmark2 row does not state '0 GL errors'"
vcase "'0 out-of-memory' missing" \
    verdicts-nooom-RESULTS.md --verdicts 1 "AM62P glmark2 row does not state '0 out-of-memory'"
vcase "terrain picture verdict missing" \
    verdicts-noterrain-RESULTS.md --verdicts 1 "i.MX8MP glmark2 row does not state a terrain picture verdict"
vnot "terrain missing: refract still has its verdict" \
    verdicts-noterrain-RESULTS.md 'refract picture verdict'
vcase "refract picture verdict missing" \
    verdicts-norefract-RESULTS.md --verdicts 1 "Raspberry Pi 4 glmark2 row does not state a refract picture verdict"
vnot "refract missing: terrain still has its verdict" \
    verdicts-norefract-RESULTS.md 'terrain picture verdict'
vcase "match/mismatch without glmark2-pictures.txt is no verdict" \
    verdicts-notable-RESULTS.md --verdicts 1 "AM62P glmark2 row does not state a terrain picture verdict"
vcase "a row with none of the items (pass-RESULTS.md)" \
    pass-RESULTS.md --verdicts 1 "i.MX8MP glmark2 row does not state '0 out-of-memory'"

echo
if [ "$fail" -eq 0 ]; then
    echo "readme-glmark2-verdicts-selftest: all cases passed"
    exit 0
fi
echo "readme-glmark2-verdicts-selftest: $fail case(s) failed"
exit 1
