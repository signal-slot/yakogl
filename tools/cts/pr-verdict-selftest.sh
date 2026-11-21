#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pr-verdict-selftest.sh
#
# Host-only checks of tools/cts/pr-verdict.py: first its own --selftest (the
# verdicts of the pass, knob-off stop and failure-table logs, unchanged), then
# the knob-on at-maximum wait line ('render: at maximum (list L, P pages),
# awaiting the firmware's partial render for up to 10 s', printed instead of
# the stop when the partial-render knob is on) on the synthetic logs in
# tools/cts/testdata/pr-verdict/:
#
#   atmax-pass.log     knob on: the wait, a partial render, no stop, recoveries
#                      0: PASS with the note 'at-maximum wait resolved by
#                      partial render'
#   atmax-nopr.log     knob on: the wait and a stop cause 4 after it: FAIL,
#                      classified as the firmware not rendering partially
#                      (fallback: C config, docs/notes/pvr-pr-atmax.md)
#   atmax-knoboff.log  the wait line in a knob-off log (--expect stop): FAIL
#                      'knob-off image printed the knob-on wait line'
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/pr-verdict
TOOL=$HERE/pr-verdict.py
fail=0

if ! out=$(python3 -I "$TOOL" --selftest 2>&1); then
    echo "FAIL pr-verdict.py --selftest:"
    printf '%s\n' "$out" | tail -n 10 | sed 's/^/       /'
    fail=1
else
    echo "ok   pr-verdict.py --selftest"
fi

# check NAME WANT-EXIT LOG EXPECT CASES... -- PATTERN...
check() {
    local name=$1 want=$2 log=$3 expect=$4 out rc pat
    shift 4
    local cases=()
    while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
        cases+=("$1"); shift
    done
    shift
    out=$(python3 -I "$TOOL" --expect "$expect" "$FIX/$log" "${cases[@]}" 2>&1); rc=$?
    if [ "$rc" != "$want" ]; then
        echo "FAIL $name: exit $rc (want $want)"
        printf '%s\n' "$out" | sed 's/^/       /'
        fail=1
        return
    fi
    for pat in "$@"; do
        if ! printf '%s\n' "$out" | grep -q -- "$pat"; then
            echo "FAIL $name: output lacks '$pat'"
            printf '%s\n' "$out" | sed 's/^/       /'
            fail=1
            return
        fi
    done
    echo "ok   $name"
}

check atmax-pass 0 atmax-pass.log pr refusehang forcedpr -- \
    "at-maximum wait resolved by partial render" "^pr-verdict: OK\$"
check atmax-nopr 1 atmax-nopr.log pr refusehang -- \
    "at-maximum wait: firmware did not render partially (fallback: run C config, see docs/notes/pvr-pr-atmax.md)"
check atmax-knoboff 1 atmax-knoboff.log stop refusehang -- \
    "knob-off image printed the knob-on wait line"

exit $fail
