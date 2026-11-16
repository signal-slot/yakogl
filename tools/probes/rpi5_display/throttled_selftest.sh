#!/usr/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/probes/rpi5_display/throttled_selftest.sh
#
# Hermetic self-test of throttled.sh on fixed strings: 0x0, 0x50005,
# 0x80008, 0x20002, garbage, the Linux and the Zephyr input forms, files.
set -u
export LC_ALL=C
HERE=$(cd "$(dirname "$0")" && pwd)
S=$HERE/throttled.sh
[ -r "$S" ] || { echo "throttled_selftest: $S not found" >&2; exit 2; }
total=0
fail=0

# run <name> <expected exit> <expected substring or ''> <input>
run() {
    local out rc
    out=$(printf '%s\n' "$4" | bash "$S" 2>&1)
    rc=$?
    total=$((total + 1))
    if [ "$rc" -ne "$2" ]; then
        printf '  FAIL  %s: exit %s, want %s\n' "$1" "$rc" "$2"
        fail=$((fail + 1))
    elif [ -n "$3" ] && [[ $out != *"$3"* ]]; then
        printf '  FAIL  %s: no "%s" in: %s\n' "$1" "$3" "$out"
        fail=$((fail + 1))
    else
        printf '  ok    %s\n' "$1"
    fi
}

run zero 0 'throttled 0x00000000: now=none seen=none verdict=ok' 'throttled=0x0'
run uv 1 'throttled 0x00050005: now=under-voltage,throttled seen=under-voltage,throttled verdict=weak-supply' 'throttled=0x50005'
run temp 1 'throttled 0x00080008: now=soft-temp-limit seen=soft-temp-limit verdict=throttled' 'throttled=0x80008'
run cap 1 'throttled 0x00020002: now=arm-freq-capped seen=arm-freq-capped verdict=throttled' 'throttled=0x20002'
run seen-only 1 'now=none seen=under-voltage verdict=weak-supply' 'throttled=0x10000'
run zephyr 1 'throttled 0x00050000: now=none seen=under-voltage,throttled verdict=weak-supply' \
    'throttled=0x00050000 uv=1 cap=0 thr=1 temp=0'
run zephyr-ok 0 'verdict=ok' 'rpi5_display: x throttled=0x00000000 uv=0'
run garbage 2 'unparsable' 'throttled=zzz'
run no-hex 2 'unparsable' 'throttled=12'
run empty 2 'no throttled=' 'hello'
run mixed 2 'verdict=ok' $'throttled=0x0\nthrottled=bad'

T=$(mktemp -d "${TMPDIR:-/tmp}/throttled-selftest.XXXXXX") || exit 2
trap 'rm -rf "$T"' EXIT
printf 'throttled=0x0\n' > "$T/a"
printf 'noise\nthrottled=0x50005\n' > "$T/b"
total=$((total + 1))
out=$(bash "$S" "$T/a" "$T/b" 2>&1)
rc=$?
if [ "$rc" -eq 1 ] && [[ $out == *verdict=ok*verdict=weak-supply* ]]; then
    echo '  ok    two files'
else
    printf '  FAIL  two files: %s %s\n' "$rc" "$out"
    fail=$((fail + 1))
fi
total=$((total + 1))
bash "$S" "$T/missing" > /dev/null 2>&1
rc=$?
if [ "$rc" -eq 2 ]; then
    echo '  ok    missing file'
else
    printf '  FAIL  missing file: exit %s\n' "$rc"
    fail=$((fail + 1))
fi

echo "throttled_selftest: $((total - fail)) of $total checks, $fail failed"
[ "$fail" -eq 0 ]
