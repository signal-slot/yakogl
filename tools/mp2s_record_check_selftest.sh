#!/usr/bin/env bash
# Host selftest of tools/mp2s_record_check.py on synthetic console logs. No board.
set -u
here=$(cd "$(dirname "$0")" && pwd)
chk="$here/mp2s_record_check.py"
tmp=$(mktemp -d "${TMPDIR:-/tmp}/mp2s-rc.XXXXXX") || exit 2
trap 'rm -rf "$tmp"' EXIT
fail=0

bl2()    { printf 'NOTICE:  BL2: v2.10-stm32mp-r3.1(release):\r\nNOTICE:  Reset reason: %s (0x4).\r\n' "${1:-System reset (SYSRST)}"; }
left()   { printf '\033[0mmp2s: default /yakogl/standalone/default.bin, boots left %s\r\n' "$1"; }
load()   { printf 'mp2s: loaded 1234 bytes at 0xa0000000, RAM footprint 0x2000 of the 0x10000000-byte window\r\nmp2s: go 0xa0000000\r\n'; }
image()  { printf 'mp2_hello: stm32mp257f_dk, console usart2 at 115200 baud\r\nmp2: mp2_hello done: PSCI SYSTEM_RESET at 20000 ms\r\n'; }
linux()  { printf 'mp2s: default used up, booting Linux\r\nSCRIPT FAILED: continuing...\r\nStarting Kernel\r\nstm32mp2 login: \r\n'; }
linux_nologin() { printf 'mp2s: default used up, booting Linux\r\nSCRIPT FAILED: continuing...\r\nBoot1: extlinux\r\n'; }

expect() { # name want-rc pattern count
    out=$(python3 -I "$chk" "$tmp/$1.log" --count "$4" 2>&1); rc=$?
    if [ "$rc" -ne "$2" ] || ! printf '%s\n' "$out" | grep -q -- "$3"; then
        echo "FAIL $1: rc=$rc want $2, pattern '$3'"; printf '%s\n' "$out" | sed 's/^/    /'; fail=1
    else
        echo "ok   $1"
    fi
}

( bl2 'Pin reset from NRST'; echo 'Linux reboots'; bl2; left 1; load; image; bl2; left 0; load; image; bl2; linux ) >"$tmp/pass2.log"
expect pass2 0 'VERDICT: PASS' 2
( bl2; left 0; load; image; bl2; linux ) >"$tmp/pass1.log"
expect pass1 0 'VERDICT: PASS' 1
( bl2; left 1; load; image; bl2; load; image; bl2; linux ) >"$tmp/noleft0.log"
expect noleft0 1 "VERDICT: FAIL.*no 'boots left 0'" 2
( bl2; left 1; load; bl2; left 0; load; image; bl2; linux ) >"$tmp/noimage.log"
expect noimage 1 'VERDICT: FAIL.*image line not seen' 2
( bl2; left 1; load; image; bl2; left 0; load; image; bl2; linux_nologin ) >"$tmp/nolinux.log"
expect nolinux 1 'VERDICT: FAIL.*Linux userspace line not reached' 2
( bl2; left 1; load; image; bl2 'Brownout reset (bor_rstn)'; left 0; load; image; bl2; linux ) >"$tmp/brownout.log"
expect brownout 1 'VERDICT: FAIL.*supply suspect' 2
( bl2; left 1; load; image; bl2 'IWDG system reset (iwdg1_out_rst)'; left 0; load; image; bl2; linux ) >"$tmp/iwdg.log"
expect iwdg 1 'VERDICT: FAIL.*watchdog reset' 2
( bl2; left 1; load; image; bl2; printf 'mp2s: refused: image too big, booting Linux\r\n'; linux ) >"$tmp/refused.log"
expect refused 1 'VERDICT: FAIL.*refused: image too big' 2
( bl2; load; left 0; image; bl2; linux ) >"$tmp/order.log"
expect order 1 'VERDICT: FAIL.*comes after the load' 1

[ "$fail" -eq 0 ] && echo "mp2s_record_check_selftest: all ok"
exit "$fail"
