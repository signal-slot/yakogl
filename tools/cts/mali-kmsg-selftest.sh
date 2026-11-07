#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mali-kmsg-selftest.sh
#
# Host-only check of tools/cts/mali-kmsg.py on the synthetic kernel message
# captures in tools/cts/testdata/mali-kmsg/ (built from the panthor v6.18
# format strings: a clean capture, a job timeout plus reset, a lone job timeout, an MMU fault,
# unrelated noise, an empty one). Each NAME.txt must print exactly
# NAME.expected; the --json form is checked on mmu.txt, stdin on reset.txt.
# Exits 0 only if every comparison matches.
set -u

HERE=$(dirname "$0")
TOOL=$HERE/mali-kmsg.py
DATA=$HERE/testdata/mali-kmsg
fail=0
pass=0
ok()  { printf '  ok    %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  FAIL  %s\n' "$1"; fail=$((fail + 1)); }

for n in clean reset jobtimeout mmu noise empty; do
    got=$(python3 "$TOOL" "$DATA/$n.txt" 2>&1)
    want=$(cat "$DATA/$n.expected")
    if [ "$got" = "$want" ]; then ok "$n"; else bad "$n: got '$got', want '$want'"; fi
done

got=$(python3 "$TOOL" --json "$DATA/mmu.txt" 2>&1)
want=$(cat "$DATA/mmu.json.expected")
if [ "$got" = "$want" ]; then ok "mmu --json"; else bad "mmu --json: got '$got'"; fi

got=$(python3 "$TOOL" < "$DATA/reset.txt" 2>&1)
want=$(cat "$DATA/reset.expected")
if [ "$got" = "$want" ]; then ok "reset via stdin"; else bad "reset via stdin"; fi

# the lone job-timeout line (the shape the board logged) is one timeout, no reset
n=$(python3 "$TOOL" "$DATA/jobtimeout.txt" | sed -n 's/.* timeouts=\([0-9]*\) .*/\1/p')
if [ "${n:-0}" -eq 1 ]; then ok "jobtimeout fixture: timeouts=1"; else bad "jobtimeout fixture: timeouts=1"; fi

# the reset fixture must report at least one reset
n=$(python3 "$TOOL" "$DATA/reset.txt" | sed -n 's/.* resets=\([0-9]*\) .*/\1/p')
if [ "${n:-0}" -ge 1 ]; then ok "reset fixture: resets>=1"; else bad "reset fixture: resets>=1"; fi

printf 'mali-kmsg-selftest: %d ok, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
