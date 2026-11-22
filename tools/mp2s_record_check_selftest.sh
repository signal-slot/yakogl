#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_record_check_selftest.sh - tools/mp2s_record_check.py: its built-in
# --self-test plus black-box cases on synthetic console logs built in $TMPDIR.
# Console strings come from driver/tools/mp2-standalone/mp2-standalone.cmd.
# No board, no network.
set -uo pipefail
export LC_ALL=C
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool=$here/mp2s_record_check.py
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_rc.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
checks=0
failed=0

bl2() { printf 'NOTICE:  BL2: v2.10-stm32mp-r3.1(release):x\nNOTICE:  Reset reason: %s (0x4). Warm boot.\n' "${1:-System reset (SYSRST)}"; }
left() { printf 'mp2s: default /yakogl/standalone/default.bin, boots left %s\n' "$1"; }
goimg() {
    printf 'mp2s: loaded 1234 bytes at 0xa0000000, RAM footprint 0x2000 of the 0x10000000-byte window\n'
    printf 'mp2s: go 0xa0000000\n'
    printf 'mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud\n'
}
linux() { printf 'mp2s: default used up, booting Linux\nSCRIPT FAILED: continuing...\nstm32mp2 login: \n'; }
nolinux() { printf 'mp2s: default used up, booting Linux\nSCRIPT FAILED: continuing...\nBoot1: extlinux\n'; }

# check_ NAME WANT_EXIT PATTERN COUNT: the record is in $work/log
check_() {
    local name=$1 want=$2 pat=$3 cnt=$4 out rc
    out=$(python3 -I "$tool" "$work/log" --count "$cnt" 2>&1)
    rc=$?
    checks=$((checks + 1))
    if [ "$rc" != "$want" ] || ! grep -Eq -- "$pat" <<<"$out"; then
        echo "FAIL $name: exit $rc (want $want), pattern '$pat'"
        sed 's/^/  | /' <<<"$out"
        failed=$((failed + 1))
    else
        echo "ok $name"
    fi
}

checks=$((checks + 1))
if out=$(python3 -I "$tool" --self-test 2>&1); then
    echo "ok builtin --self-test"
else
    echo "FAIL builtin --self-test"
    sed 's/^/  | /' <<<"$out"
    failed=$((failed + 1))
fi

good() { bl2 "Pin reset from NRST"; bl2; left 1; goimg; bl2; left 0; goimg; bl2; linux; }

good >"$work/log"
check_ pass2 0 'VERDICT: PASS 2 image' 2

good | sed 's/^/\x1b[32m/; s/$/\x1b[0m\r/' >"$work/log"
check_ ansi_crlf 0 'VERDICT: PASS' 2

(bl2; left 1; goimg; bl2; goimg; bl2; linux) >"$work/log"
check_ no_left0 1 "VERDICT: FAIL boot 2: no 'boots left 0'" 2

(bl2; left 1; goimg; bl2 "Power-on reset"; left 0; goimg; bl2; linux) >"$work/log"
check_ poweron 1 'VERDICT: FAIL boot 2: .*supply suspect' 2

(bl2; left 1; goimg; bl2 "IWDG system reset (iwdg1_out_rst)"; left 0; goimg; bl2; linux) >"$work/log"
check_ iwdg 1 'VERDICT: FAIL boot 2: .*watchdog reset' 2

(bl2; left 1; goimg; bl2; left 0; goimg; bl2; nolinux) >"$work/log"
check_ no_linux 1 'VERDICT: FAIL boot 3: Linux userspace line not reached' 2

(bl2; left 0; goimg; bl2; linux) >"$work/log"
check_ pass1 0 'VERDICT: PASS 1 image' 1

echo "mp2s_record_check_selftest: $checks checks, $failed failed"
[ "$failed" = 0 ]
