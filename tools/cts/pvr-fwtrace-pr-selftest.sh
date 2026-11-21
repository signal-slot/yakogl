#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/pvr-fwtrace-pr-selftest.sh   (runs under sh)
#
# Host-only checks of tools/cts/pvr-fwtrace-pr.py: first its own --selftest,
# then the synthetic AM62P logs in tools/cts/testdata/pvr-fwtrace-pr/ -
# trace lines in the exact shapes pvr_fw_trace_decode.c prints them (behind
# the board's "FWT| " prefix, pvr_selftest.c:111) and a stall dump in the
# shape render.rs prints it. Exit status: 0 every address matched (or the
# timeline was unchecked for want of a stall dump), 1 a trace address matched
# nothing or the log has no partial-render trace line, 2 unusable arguments
# or log.
#
#   match.log     SPM 17-20 addresses equal the dumped Z/S bases (a packed
#                 D24S8 frame: one address in both pairs): exit 0,
#                 'pvr-fwtrace-pr: OK', each address tagged
#                 '= Z base (dump 1), S base (dump 1)'
#   mismatch.log  a ZStore address outside the dumped bases: exit 1, FAIL
#                 'matches no dumped Z/S address or scratch'; the same log
#                 with --spm-scratch 0x0000000040123456 exits 0 tagging that
#                 address '= scratch'
#   notrace.log   a stall dump and a decoded SPM line that is not a
#                 partial-render line ('3D MemFree: Local FL 0x00000000'):
#                 exit 1, FAIL 'no SPM/MAIN partial-render trace lines in
#                 the log'
#   no-dump.log   partial-render address lines but no stall dump: exit 0,
#                 the timeline tagged 'unchecked (no dumped Z/S words)'
#   nozs.log      MAIN 47/48 only ('PR: no ZS buffer (store|load)'): exit 0,
#                 the lines in the timeline, no address to check
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
FIX=$HERE/testdata/pvr-fwtrace-pr
TOOL=$HERE/pvr-fwtrace-pr.py
fail=0

if ! out=$(python3 -I "$TOOL" --selftest 2>&1); then
    echo "FAIL pvr-fwtrace-pr.py --selftest:"
    printf '%s\n' "$out" | tail -n 12 | sed 's/^/       /'
    fail=1
else
    echo "ok   pvr-fwtrace-pr.py --selftest"
fi

# check NAME WANT-EXIT LOG [TOOL-ARG...] -- PATTERN...
# Tool arguments before '--' (a space-separated list, none contains a space);
# every argument after it is a grep pattern of the tool's output.
check() {
    name=$1 want=$2 log=$3 extra=""
    shift 3
    while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do extra="$extra $1"; shift; done
    shift
    out=$(python3 -I "$TOOL" $extra "$FIX/$log" 2>&1); rc=$?
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

check match 0 match.log -- "pvr-fwtrace-pr: OK" "= Z base (dump 1), S base (dump 1)"
check mismatch 1 mismatch.log -- "pvr-fwtrace-pr: FAIL" "no match" \
    "matches no dumped Z/S address or scratch"
check mismatch-scratch 0 mismatch.log --spm-scratch 0x0000000040123456 -- \
    "pvr-fwtrace-pr: OK" "= scratch"
check notrace 1 notrace.log -- "pvr-fwtrace-pr: FAIL" \
    "no SPM/MAIN partial-render trace lines in the log"
check no-dump 0 no-dump.log -- "pvr-fwtrace-pr: OK" "unchecked (no dumped Z/S words)"
check nozs 0 nozs.log -- "pvr-fwtrace-pr: OK" "no ZS buffer (store)" "no ZS buffer (load)"
check bad-scratch 2 match.log --spm-scratch notanaddr -- "is not an address"

if python3 -I "$TOOL" "$FIX/no-such.log" 2>/dev/null; then
    echo "FAIL unreadable log: want exit 2"
    fail=1
else
    echo "ok   unreadable log exits 2"
fi

exit $fail
