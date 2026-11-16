#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_reset_reason_selftest.sh - tools/mp2s_reset_reason.py on inline
# synthetic records, no board, no network (plan #362, item reset-reason).
set -uo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
tool=$here/mp2s_reset_reason.py
fail=0

# boot TEXT RAW [SUFFIX]: one synthetic BL2 boot
boot() {
    printf 'NOTICE:  BL2: v2.10-stm32mp-r3.1(release):x\n'
    printf 'NOTICE:  Reset reason: %s (0x%s). %s\n' "$1" "$2" "${3:-Warm boot.}"
}

# run NAME WANT_EXIT PATTERN [tool args]: the record is in REC
run() {
    local name=$1 want=$2 pat=$3 out rc
    shift 3
    out=$(printf '%s' "$REC" | python3 -I "$tool" "$@" 2>&1)
    rc=$?
    if [ "$rc" != "$want" ] || ! grep -Eq -- "$pat" <<<"$out"; then
        echo "FAIL $name: exit $rc (want $want), pattern '$pat'"
        sed 's/^/  | /' <<<"$out"
        fail=1
    else
        echo "ok $name"
    fi
}

REC=$(boot "Power-on reset (por_rstn)" 1 "Cold boot.")
run por 3 'boot 1: Power-on reset \(por_rstn\): supply'
run por-hint 3 '^check the USB-C supply first$'
REC=$(boot "Brownout reset (bor_rstn)" 2 "Cold boot.")
run brownout 3 'Brownout reset \(bor_rstn\): supply'
run brownout-hint 3 '^check the USB-C supply first$'
REC=$(boot "IWDG system reset (iwdgX_out_rst)" 100)
run watchdog 0 'IWDG system reset \(iwdgX_out_rst\): watchdog'
REC=$(boot "System reset (SYSRST)" c0)
run software 0 'System reset \(SYSRST\): software'
REC=$(boot "Pin reset from NRST" 4)
run pin 0 'Pin reset from NRST: software'
REC=$(boot "Clock failure on HSE" 8)
run unknown 0 'Clock failure on HSE: unknown'
REC=$(printf 'NOTICE:  BL2: v2.10-stm32mp-r3.1(release):x\nno reason here\n')
run no-reason 0 'boot 1: \(no Reset reason line\): unknown'
REC=$(printf 'notice: reset reason: BROWNOUT RESET\n')
run case-insensitive 3 'boot 1: .*: supply'

REC=$(boot "System reset (SYSRST)" c0; boot "System reset (SYSRST)" c0; boot "IWDG system reset (iwdgX_out_rst)" 100)
run mixed-clean 0 'summary: 3 boot\(s\): 0 supply, 1 watchdog, 2 software, 0 unknown'
REC=$(boot "System reset (SYSRST)" c0; boot "Power-on reset (por_rstn)" 1; boot "IWDG system reset (iwdgX_out_rst)" 100)
run mixed-boot2 3 'boot 2: .*: supply'
run mixed-summary 3 'summary: 3 boot\(s\): 1 supply, 1 watchdog, 1 software, 0 unknown'

REC=
run empty 0 'summary: 0 boot\(s\)'
run empty-expect0 0 'summary: 0 boot' --expect 0
run empty-expect1 2 'expected 1 boot' --expect 1
REC=$(boot "System reset (SYSRST)" c0; boot "System reset (SYSRST)" c0)
run expect-ok 0 'summary: 2 boot' --expect 2
run expect-bad 2 'expected 3 boot\(s\), found 2' --expect 3
REC=$(boot "Brownout reset (bor_rstn)" 2)
run expect-supply 3 'check the USB-C supply first' --expect 1

REC=
run bad-option 2 'unrecognized|usage' --bogus
run missing-file 2 'cannot read' /nonexistent/record.txt
run bad-expect 2 'invalid|usage' --expect x

[ "$fail" = 0 ] && echo "mp2s_reset_reason selftest: all ok"
exit "$fail"
