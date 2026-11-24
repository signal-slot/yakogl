#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_clear/verdicts_selftest.sh
#
# Feeds verdicts.sh inline rpi5_clear console logs (the line format of
# src/main.c) and checks the last line and the exit status:
#   good       (exit 0, PASS)        failimg   the image's FAIL line (exit 3)
#   readback   end says PASS beside "M4: 5 ... wrong" (exit 3)
#   guard      guard row changed, end says PASS (exit 3)
#   steprc     M3 render: -5, end says PASS (exit 3)
#   truncated  no end line (exit 1)  crlf  CR ends and a prefix (exit 0)
#   usage      no argument, unreadable file (exit 2)
# Never exits 77. Prints "verdicts_selftest: N checks, M failed".
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_clear_vt.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM
checks=0
failed=0

head_lines() {
    cat <<'X'
v3d_clear: build 0123abcd
v3d_clear: cache: CONFIG_CACHE_MANAGEMENT=1 CONFIG_DCACHE=1 sys_cache_data_flush_and_invd_range, data line 64 bytes
M1 mailbox: 0
M1 power_on: 0, PM_GRAFX 0x00001040, SMS REE_CS 0x00000000 TEE_CS 0x00000050
M1 identity: V3D 7.1, 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes: 0
M2 mmu_init: 0
frame 128x64: tile 64x64, 2x1 tiles, lists 19 / 99 / 27 bytes
M3 bin: 0
M3 render: 0
M3 caches_clean: 0
X
}
PASSL='v3d_clear: PASS (M1 to M4, 128x64 cleared; 46 pages used incl. page table and scratch, cache: x)'
FAILL='v3d_clear: FAIL (M4 pixels; 46 pages used incl. page table and scratch, cache: x)'
M4OK='M4: 0 of 8192 target pixels wrong, guard row 0 of 128 changed'

# run <name> <exit> <last-line-substring> <logfile>
run() {
    name=$1
    want=$2
    pat=$3
    log=$4
    checks=$((checks + 2))
    out=$(sh "$V" "$log" 2>&1)
    rc=$?
    last=$(printf '%s\n' "$out" | tail -n 1)
    if [ "$rc" -ne "$want" ]; then
        failed=$((failed + 1))
        echo "FAIL: $name: exit $rc, wanted $want"
    fi
    case "$last" in
    *"$pat"*) ;;
    *)
        failed=$((failed + 1))
        echo "FAIL: $name: last line '$last' lacks '$pat'"
        ;;
    esac
}

# mk <file> <m4 line> <end line> [sed script for the head]
mk() {
    head_lines | sed "${4:-}" >"$1"
    [ -n "$2" ] && echo "$2" >>"$1"
    [ -n "$3" ] && echo "$3" >>"$1"
}

mk "$tmp/good" "$M4OK" "$PASSL"
run good 0 "verdicts: PASS" "$tmp/good"

mk "$tmp/failimg" 'M4: 3 of 8192 target pixels wrong (first at 1,2: 0xffff00ff), guard row 0 of 128 changed' "$FAILL"
run failimg 3 "verdicts: FAIL" "$tmp/failimg"

mk "$tmp/readback" 'M4: 5 of 8192 target pixels wrong (first at 0,0: 0xffff00ff), guard row 0 of 128 changed' "$PASSL"
run readback 3 "m4 FAIL" "$tmp/readback"

mk "$tmp/guard" 'M4: 0 of 8192 target pixels wrong, guard row 4 of 128 changed' "$PASSL"
run guard 3 "m4 FAIL" "$tmp/guard"

mk "$tmp/steprc" "$M4OK" "$PASSL" 's/^M3 render: 0$/M3 render: -5/'
run steprc 3 "m3 FAIL" "$tmp/steprc"

mk "$tmp/truncated" "" ""
run truncated 1 "no end line" "$tmp/truncated"

mk "$tmp/plain" "$M4OK" "$PASSL"
sed 's/^/[00:00:01.000,000] /; s/$/\r/' "$tmp/plain" >"$tmp/crlf"
run crlf 0 "verdicts: PASS" "$tmp/crlf"

checks=$((checks + 1))
sh "$V" >/dev/null 2>&1
[ $? -eq 2 ] || { failed=$((failed + 1)); echo "FAIL: usage: no argument"; }
checks=$((checks + 1))
sh "$V" "$tmp/none" >/dev/null 2>&1
[ $? -eq 2 ] || { failed=$((failed + 1)); echo "FAIL: usage: unreadable file"; }

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
