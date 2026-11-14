#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_model_selftest.sh - tools/mp2s_model.py without a board (plan #362,
# item fault-model).
#
#   tools/mp2s_model_selftest.sh
#
# The real design must pass (exit 0, every verdict "ok", a trace count).
# Each known-bad variant, injected with --mutant, must make the model exit 1
# with a "counterexample" and the invariant it breaks:
#   load_before_save, save_after_load  -> I2 (the image boots more than N times)
#   no_restore                         -> I3 (boot_prefixes not restored)
#   runner_two_step                    -> I3 (a cut between the two writes)
#   bootcount_write                    -> I4
# Usage errors exit 2.
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_model_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
model="$here/mp2s_model.py"
fail=0
bad() { echo "FAIL: $*" >&2; fail=1; }

out=$(python3 -I "$model") || bad "the real design failed: $out"
echo "$out"
echo "$out" | grep -Eq '^model: .*: [0-9]+ traces$' || bad "no trace count"
[ "$(echo "$out" | grep -c '^I[1-4] ok\|^F  ok')" = 5 ] || bad "not five ok verdicts"
echo "$out" | grep -q counterexample && bad "real design printed a counterexample"

check_mutant() { # name invariant
    local rc=0 o
    o=$(python3 -I "$model" --mutant "$1") || rc=$?
    [ "$rc" = 1 ] || bad "mutant $1: exit $rc, want 1"
    echo "$o" | grep -q '^counterexample' || bad "mutant $1: no counterexample"
    echo "$o" | grep -q "^$2 .*FAIL" || bad "mutant $1: $2 did not fail"
    echo "$o" | grep -q '^  1\. ' || bad "mutant $1: no trace printed"
    echo "mutant $1: ok (fails $2)"
}
check_mutant load_before_save I2
check_mutant save_after_load I2
check_mutant no_restore I3
check_mutant runner_two_step I3
check_mutant bootcount_write I4

rc=0
python3 -I "$model" --mutant nonsense >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || bad "unknown mutant: exit $rc, want 2"

[ "$fail" = 0 ] && echo "mp2s_model_selftest: ok"
exit "$fail"
