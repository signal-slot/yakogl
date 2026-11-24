#!/bin/sh
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   sh tools/probes/rpi_5/rpi5_ident/verdicts_selftest.sh
#
# Feeds verdicts.sh inline rpi5_ident console logs (the 2026-11-19 board run's
# lines, from the README) and checks its lines and exit status:
#
#   good       the run as printed (exit 0, PASS), also with CR line ends
#   fallback   the same with the fallback's extra lines (exit 0)
#   fail       the image's FAIL line (exit 3)
#   truncated  the log stops after CORE_IDENT0, no end line (exit 1)
#   wrong      HUB_IDENT1 differs, with `done` (exit 3, names HUB_IDENT1)
#   usage      no argument, an unreadable file (exit 2)
#
# Writes only under $TMPDIR. Prints one line per failed check and a last line
# "verdicts_selftest: N checks, M failed"; exits 0 when none failed.
set -u

here=$(cd "$(dirname "$0")" && pwd)
V="$here/verdicts.sh"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/rpi5_ident_verdicts.XXXXXX") || exit 1
trap 'rm -rf "$tmp"' EXIT INT TERM

checks=0
failed=0

fail() {
    failed=$((failed + 1))
    echo "FAIL: $*"
}

head_lines() {
    echo "board: watchdog armed"
    echo "*** Booting Zephyr OS build af17c0c8aa2a ***"
    echo "rpi5_ident: build 8eb2b3cf"
    echo "rpi5_ident: rpi_5, V3D 7.1, no power or clock setup, watchdog armed at boot"
    echo "rpi5_ident: hub and core0 mapped, reading"
}

words() {
    echo "rpi5_ident: HUB_IDENT0 0x42554856"
    echo "rpi5_ident: HUB_IDENT1 ${H1:-0x00081117}"
    echo "rpi5_ident: HUB_IDENT2 0x00001900"
    echo "rpi5_ident: HUB_IDENT3 0x00020a10"
    echo "rpi5_ident: CORE_IDENT0 0x07443356"
    echo "rpi5_ident: CORE_IDENT1 0x81001431"
    echo "rpi5_ident: CORE_IDENT2 0xc0078101"
    echo "rpi5_ident: MMU_DEBUG_INFO 0x20804664"
    echo "rpi5_ident: V3D 7.1, 1 core(s), 12 QPUs, VPM 65536 bytes, PA 36 / VA 36 bits, MMU yes"
}

( head_lines; words; echo "rpi5_ident: done" ) >"$tmp/good.log"
sed 's/$/\r/' "$tmp/good.log" >"$tmp/crlf.log"
(
    head_lines
    echo "rpi5_ident: fallback: mailbox 0"
    echo "rpi5_ident: fallback: PM_GRAFX before 0x00000001"
    words
    echo "rpi5_ident: done"
) >"$tmp/fallback.log"
( head_lines; words; echo "rpi5_ident: FAIL" ) >"$tmp/fail.log"
( head_lines; words | head -n 5 ) >"$tmp/truncated.log"
( head_lines; H1=0x00081118 words; echo "rpi5_ident: done" ) >"$tmp/wrong.log"

# run <name> <log> <want exit> [grep pattern ...]: every pattern must match a line
run() {
    name=$1
    log=$2
    want=$3
    shift 3
    out=$(sh "$V" "$log" 2>&1)
    rc=$?
    checks=$((checks + 1))
    [ "$rc" -eq "$want" ] || fail "$name: exit $rc, want $want"
    for p in "$@"; do
        checks=$((checks + 1))
        printf '%s\n' "$out" | grep -q -e "$p" || fail "$name: no line matching '$p'"
    done
}

run good "$tmp/good.log" 0 'HUB_IDENT0: ok' 'MMU_DEBUG_INFO: ok' 'decode: ok' 'end: ok' 'verdict: PASS$'
run crlf "$tmp/crlf.log" 0 'CORE_IDENT2: ok' 'verdict: PASS$'
run fallback "$tmp/fallback.log" 0 'verdict: PASS$'
run fail "$tmp/fail.log" 3 'end: FAIL' 'verdict: FAIL'
run truncated "$tmp/truncated.log" 1 'HUB_IDENT3: ok' 'CORE_IDENT1: FAIL (missing' 'end: missing' 'no end line'
run wrong "$tmp/wrong.log" 3 'HUB_IDENT1: FAIL (0x00081118 / expected 0x00081117)' 'HUB_IDENT0: ok' 'verdict: FAIL (HUB_IDENT1)'

checks=$((checks + 1))
sh "$V" >/dev/null 2>&1
[ $? -eq 2 ] || fail "usage: no argument not exit 2"
checks=$((checks + 1))
sh "$V" "$tmp/none.log" >/dev/null 2>&1
[ $? -eq 2 ] || fail "usage: unreadable file not exit 2"

echo "verdicts_selftest: $checks checks, $failed failed"
[ "$failed" -eq 0 ]
