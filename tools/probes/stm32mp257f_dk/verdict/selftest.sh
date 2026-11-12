#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# selftest.sh - dk_verdict.py against the synthetic logs in testdata/.
#
#   tools/probes/stm32mp257f_dk/verdict/selftest.sh [--regen]
#
# Every testdata/<check>.<case>.log is run with "--only <check>"; the whole
# output must equal testdata/<check>.<case>.want, and the exit code must be 0
# for a "pass"/"stamped" case, 1 for a "fail" case. Also checks that every
# check has a pass and a fail case, that stdin works, and that a log with no
# known program is not a pass. Ends with "N checks, 0 failed". --regen
# rewrites the .want files from the tool's output (review them in the diff).
set -u
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool=$here/dk_verdict.py
data=$here/testdata
regen=0
[ "${1:-}" = --regen ] && regen=1

n=0
failed=0
ok() { n=$((n + 1)); echo "ok   $1"; }
bad() { n=$((n + 1)); failed=$((failed + 1)); echo "FAIL $1"; }

checks=$(python3 -I "$tool" --expect nonsense </dev/null 2>&1 | sed -n 's/.*(\(.*\))$/\1/p' | tr -d ',')
[ -n "$checks" ] || { echo "cannot list the checks"; exit 1; }

for c in $checks; do
    for kind in pass fail; do
        if [ -f "$data/$c.$kind.log" ]; then ok "$c has a $kind case"; else bad "$c has no $kind case"; fi
    done
done

for log in "$data"/*.log; do
    base=${log##*/}
    base=${base%.log}
    c=${base%%.*}
    kind=${base#*.}
    got=$(python3 -I "$tool" --only "$c" "$log")
    rc=$?
    if [ "$regen" = 1 ]; then printf '%s\n' "$got" >"$data/$base.want"; fi
    want=$(cat "$data/$base.want" 2>/dev/null || echo "(no .want file)")
    case $kind in fail) wantrc=1 ;; *) wantrc=0 ;; esac
    if [ "$got" = "$want" ] && [ "$rc" = "$wantrc" ]; then
        ok "$base: ${got##*$'\n'}"
    else
        bad "$base: rc=$rc (want $wantrc)"
        diff <(printf '%s\n' "$want") <(printf '%s\n' "$got") | sed 's/^/     /'
    fi
done

# stdin, and no --expect: the checks are chosen from the log.
got=$(python3 -I "$tool" <"$data/gc-clear.pass.log"); rc=$?
if [ "$rc" = 0 ] && grep -q '^verdict gc-clear ' <<<"$got" && ! grep -q '^verdict gc-fence ' <<<"$got"; then
    ok "stdin, checks chosen from the log"
else bad "stdin, checks chosen from the log"; fi

# No flags on a whole mp2_gpu run: five checks, all PASS.
got=$(python3 -I "$tool" "$data/gpu-identity.pass.log"); rc=$?
if [ "$rc" = 0 ] && [ "$(grep -c '^verdict ' <<<"$got")" = 5 ] && [ "${got##*$'\n'}" = "5 checks, 0 failed" ]; then
    ok "auto mode on a whole mp2_gpu log"
else bad "auto mode on a whole mp2_gpu log"; fi

# Nothing recognisable: exit 1, "0 checks".
got=$(printf 'U-Boot 2025.01\nLinux login:\n' | python3 -I "$tool"); rc=$?
if [ "$rc" = 1 ] && [ "$got" = "0 checks, 0 failed" ]; then ok "unknown log is not a pass"
else bad "unknown log is not a pass"; fi

# An unknown check name is a usage error.
python3 -I "$tool" --expect nonsense </dev/null >/dev/null 2>&1; rc=$?
if [ "$rc" = 2 ]; then ok "unknown check name exits 2"; else bad "unknown check name exits 2"; fi

echo "$n checks, $failed failed"
[ "$failed" = 0 ]
