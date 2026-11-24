#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_model_sim_diff_selftest.sh - tools/mp2s_model_sim_diff.py without a
# board (plan #362, item model-sim-diff).
#
#   tools/mp2s_model_sim_diff_selftest.sh
#
# The real script and the real model must agree on every case (exit 0,
# "0 differences"). Then drift is injected on each side and must be found
# (exit 1, a DIFF line):
#   - the model: --model-mutant load_before_save, save_after_load
#   - the script: a copy with the count's decrement removed, and a copy whose
#     used-up branch does not restore boot_prefixes
# Usage errors exit 2.
#
# tools/check_selftests.py lists this file, so the gate runs it as a row.
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
diff_py="$here/mp2s_model_sim_diff.py"
script="$here/../driver/tools/mp2-standalone/mp2-standalone.cmd"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_diff.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
fail=0
bad() { echo "FAIL: $*" >&2; fail=1; }

out=$(python3 -I "$diff_py") || bad "the real design differs: $out"
echo "$out"
echo "$out" | grep -Eq '^diff: N=1\.\.9, [0-9]+ cases, 0 differences$' || bad "no clean verdict"
echo "$out" | grep -q '^DIFF' && bad "real design printed a DIFF"

expect_diff() { # label args...
    local label=$1 rc=0 o
    shift
    o=$(python3 -I "$diff_py" "$@") || rc=$?
    [ "$rc" = 1 ] || bad "$label: exit $rc, want 1"
    echo "$o" | grep -q '^DIFF' || bad "$label: no DIFF line"
    echo "$label: ok (found)"
}
expect_diff "model load_before_save" --nmax 3 --model-mutant load_before_save
expect_diff "model save_after_load" --nmax 3 --model-mutant save_after_load

# script mutants: remove the decrement; skip the restore of boot_prefixes
grep -q 'setexpr mp2s_left ${mp2s_left} - 1' "$script" || bad "script: decrement line moved"
sed 's/^\tsetexpr mp2s_left ${mp2s_left} - 1$/\t: no decrement/' "$script" >"$tmp/nodec.cmd"
cmp -s "$script" "$tmp/nodec.cmd" && bad "script mutant nodec: sed changed nothing"
rc=0
python3 -I "$diff_py" --nmax 3 --script "$tmp/nodec.cmd" >"$tmp/o" 2>&1 || rc=$?
# the sim stops on a command outside its subset (exit 2) or the diff finds it (1)
case "$rc" in 1|2) echo "script nodec: ok (rc $rc)";; *) bad "script nodec: exit $rc";; esac

grep -n 'boot_prefixes' "$script" >/dev/null || bad "script has no boot_prefixes"
sed 's/^\(\s*\)setenv boot_prefixes "${mp2s_prefixes}"\s*$/\1: no restore/' "$script" >"$tmp/norest.cmd"
if cmp -s "$script" "$tmp/norest.cmd"; then
    echo "script norest: skipped (restore line has another shape)"
else
    rc=0
    python3 -I "$diff_py" --nmax 3 --script "$tmp/norest.cmd" >"$tmp/o" 2>&1 || rc=$?
    case "$rc" in 1|2) echo "script norest: ok (rc $rc)";; *) bad "script norest: exit $rc";; esac
fi

rc=0
python3 -I "$diff_py" --nmax 0 >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || bad "--nmax 0: exit $rc, want 2"
rc=0
python3 -I "$diff_py" --model-mutant nonsense >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || bad "unknown mutant: exit $rc, want 2"

[ "$fail" = 0 ] && echo "mp2s_model_sim_diff_selftest: ok"
exit "$fail"
