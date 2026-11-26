#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Self-test of readme-glmark2-check.py's row-own-run reading (scene ms and the
# commit come from the row's own run, not from comparison clauses) on
# testdata/readme-glmark2/strict-*-RESULTS.md with pass-README.md, in the
# style of readme-glmark2-verdicts-selftest.sh. Host only; no board, no git
# (--ref none). Exit 0 when every case passes.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
RG=$HERE/readme-glmark2-check.py
RGD=$HERE/testdata/readme-glmark2
fail=0
ok()  { printf '  ok    %s\n' "$1"; }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

scase() {
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

echo "== readme-glmark2-check.py strict reading (testdata/readme-glmark2)"
scase "own figures pass, comparison clauses ignored" \
    strict-pass-RESULTS.md "--scenes terrain,shadow,refract" 0 '^ok:'
scase "terrain only in an 'at <other sha>' clause fails --scenes" \
    strict-stale-RESULTS.md "--scenes terrain,shadow,refract" 1 "AM62P glmark2 row has no .terrain <n> ms"
scase "stale-figure row passes without --scenes (commit is its own)" \
    strict-stale-RESULTS.md "" 0 '^ok:'
scase "commit named only in passing fails the commit check" \
    strict-mention-RESULTS.md "" 1 "i.MX8MP glmark2 row does not cite"
scase "the plain scenes fixture still passes" \
    scenes-pass-RESULTS.md "--scenes terrain,shadow,refract" 0 '^ok:'

echo
if [ "$fail" -eq 0 ]; then
    echo "readme-glmark2-strict-selftest: all cases passed"
    exit 0
fi
echo "readme-glmark2-strict-selftest: $fail case(s) failed"
exit 1
