#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/mali-kmsg-selftest.sh
#
# Host-only check of tools/cts/mali-kmsg.py on the synthetic kernel message
# captures in tools/cts/testdata/mali-kmsg/ (built from the panthor v6.18
# format strings: a clean capture, a job timeout plus reset, a lone job timeout, an MMU fault, CS_FATAL / CS_FAULT / tiler heap lines
# (csfault.txt, synthetic like reset.txt and mmu.txt: not yet seen on the board),
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

for n in clean reset jobtimeout mmu csfault noise empty; do
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

# a CS_FATAL line of the 6.18 format string is a cs_fault, not other_panthor
r=$(python3 "$TOOL" "$DATA/csfault.txt")
c=$(printf '%s' "$r" | sed -n 's/.* cs_faults=\([0-9]*\) .*/\1/p')
o=$(printf '%s' "$r" | sed -n 's/.* other_panthor=\([0-9]*\) .*/\1/p')
if [ "${c:-0}" -eq 3 ] && [ "${o:-1}" -eq 0 ]; then ok "csfault fixture: cs_faults=3 other_panthor=0"; else bad "csfault fixture: cs_faults=3 other_panthor=0"; fi

# the reset fixture must report at least one reset
n=$(python3 "$TOOL" "$DATA/reset.txt" | sed -n 's/.* resets=\([0-9]*\) .*/\1/p')
if [ "${n:-0}" -ge 1 ]; then ok "reset fixture: resets>=1"; else bad "reset fixture: resets>=1"; fi

# --between-markers: lines strictly between the run markers, the after-end
# lines apart; synthetic captures are written to a temporary directory
tmp=$(mktemp -d "${TMPDIR:-/tmp}/kmsg-markers.XXXXXX")
trap 'rm -rf "$tmp"' EXIT
S='<6>[  10.000000] yakogl-run-start gltest 20261126-000000'
E='<6>[  20.000000] yakogl-run-end gltest 20261126-000000'
PL='<3>[  15.000000] panthor fb000000.gpu: job timeout: pid=1 (x)'
printf '%s\n%s\n' "$S" "$E" > "$tmp/zero.txt"
printf '%s\n%s\n%s\n%s\n' "$S" "$PL" "<6>[  16.000000] other line" "$E" > "$tmp/between.txt"
printf '%s\n%s\n%s\n%s\n%s\n%s\n' "$PL" "<6>[  1.0] before" "$S" "$E" "$PL" "<6>[  21.0] after" > "$tmp/outside.txt"
printf '%s\n%s\n' "$S" "$PL" > "$tmp/noend.txt"
want=$(printf 'kmsg-markers: between=0 after=0\nkmsg: resets=0 timeouts=0 mmu_faults=0 fw_faults=0 gpu_faults=0 cs_faults=0 other_panthor=0 first=-')
got=$(python3 "$TOOL" --between-markers "$tmp/zero.txt" 2>&1)
if [ "$got" = "$want" ]; then ok "markers: 0 lines between"; else bad "markers: 0 lines between: got '$got'"; fi
got=$(python3 "$TOOL" --between-markers "$tmp/between.txt" 2>&1 | sed -n 1p)
if [ "$got" = "kmsg-markers: between=2 after=0" ]; then ok "markers: 2 lines between"; else bad "markers: 2 lines between: got '$got'"; fi
got=$(python3 "$TOOL" --between-markers "$tmp/between.txt" 2>&1 | sed -n 2p)
case $got in 'kmsg: resets=0 timeouts=1 '*) ok "markers: panthor line between counted";; *) bad "markers: panthor line between: got '$got'";; esac
got=$(python3 "$TOOL" --between-markers "$tmp/outside.txt" 2>&1)
case $got in
    *'between=0 after=2'*'timeouts=0 '*) ok "markers: lines outside not counted as between";;
    *) bad "markers: lines outside: got '$got'";;
esac
got=$(python3 "$TOOL" --between-markers --json "$tmp/between.txt" 2>&1)
case $got in *'"between": 2'*'"after": 0'*) ok "markers --json";; *) bad "markers --json: got '$got'";; esac
python3 "$TOOL" --between-markers "$tmp/noend.txt" >/dev/null 2>&1; rc=$?
if [ "$rc" -eq 5 ]; then ok "markers: missing end marker exits 5"; else bad "markers: missing end marker exit $rc"; fi
# the default mode counts the marker file's panthor lines everywhere
got=$(python3 "$TOOL" "$tmp/outside.txt" | sed -n 's/.* timeouts=\([0-9]*\) .*/\1/p')
if [ "$got" = "2" ]; then ok "markers: default mode unchanged (counts outside lines)"; else bad "default mode on outside.txt: '$got'"; fi

printf 'mali-kmsg-selftest: %d ok, %d failed\n' "$pass" "$fail"
[ "$fail" -eq 0 ]
