#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_redact_check_selftest.sh - tools/mp2s_redact_check.py without a board
# (plan #362, item redact-check).
#
#   tools/mp2s_redact_check_selftest.sh
#
# Synthetic positives (made-up values only: RFC 5737 192.0.2.0/24 and
# 198.51.100.0/24, RFC 3849 2001:db8::/32, example names) must each be
# flagged with their kind and the match masked; a U-Boot console sample
# (load addresses, "mp2s: loaded 123 bytes at 0xa0000000", sha256 sums,
# version strings) must produce no finding. Also: env-variable values,
# directories, stdin, and --fix (clean copy, never in place).
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-redact-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
C="$here/mp2s_redact_check.py"
fails=0
fail() { echo "FAIL: $*" >&2; fails=$((fails + 1)); }
# env -u: the caller's own BOARD_* must not leak into the cases
run() { env -u BOARD_HOST -u BOARD_UART -u BOARD_USER python3 -I "$C" "$@"; }
runenv() { env BOARD_HOST=lab9box BOARD_UART=/dev/ttyZZ9 BOARD_USER=zed9 python3 -I "$C" "$@"; }

# pos <name> <kind> <line> <secret-fragment>: one finding, that kind, masked
pos() {
  printf '%s\n' "$3" > "$T/p.txt"
  local out rc=0
  out=$(run "$T/p.txt") || rc=$?
  [ "$rc" = 1 ] || fail "$1: exit $rc, want 1"
  case "$out" in *":1:$2:"*) ;; *) fail "$1: no $2 finding: $out";; esac
  [ "$(printf '%s\n' "$out" | wc -l)" = 1 ] || fail "$1: not exactly one finding: $out"
  case "$out" in *"$4"*) fail "$1: match not masked: $out";; esac
}
pos ipv4      ipv4    "inet 192.0.2.44/24" 192.0.2.44
pos ipv6      ipv6    "inet6 2001:db8::1234 scope global" db8::1234
pos ipv6full  ipv6    "addr 2001:db8:0:0:0:0:0:5" 0:0:0:0
pos mac       mac     "ether 02:00:5e:10:00:01 txqueuelen 1000" 00:5e
pos tailnet   tailnet "ssh root@dk-7.tail1234.ts.net" tail1234
pos fqdn      fqdn    "server lab.example.com ok" example
pos tty       tty     "console on /dev/ttyUSB3 115200" ttyUSB3
pos byid      tty     "/dev/serial/by-id/usb-FTDI_Dual_RS232-HS-if00-port0" FTDI
pos iserial   serial  "  iSerial                 3 AB12CD34EF" AB12CD34
pos serialnum serial  "SerialNumber=ZX81Q9" ZX81Q9
pos password  secret  "login password=hunter2" hunter2
pos passwd    secret  "passwd: s3cret" s3cret
pos token     secret  "curl -d token=abcdef0123" abcdef

printf 'addr 192.0.2.44 gw 198.51.100.1\n' > "$T/p.txt"
[ "$(run "$T/p.txt" | wc -l)" = 2 ] || fail "two ipv4 on one line"

# negatives: U-Boot / OP-TEE console, load addresses, sums, versions
cat > "$T/uboot.txt" <<'X'
NOTICE:  BL2: v2.10-stm32mp2-r1(release):
NOTICE:  BL2: Built : 10:11:12, Jan  1 2026
NOTICE:  Reset reason: Power-on reset (PWR)
I/TC: OP-TEE version: 4.0.0-stm32mp-r2 (gcc version 12.2.0) #1 arm64
U-Boot 2023.10-stm32mp-r2 (Jan 01 2026 - 10:11:12 +0000)
U-Boot 2023.10
OP-TEE 4.0
Model: STMicroelectronics STM32MP257F-DK Discovery Board
Hit any key to stop autoboot:  0
loadaddr=0xa0000000
scriptaddr=0x84000000
fdt_addr_r=0x8c000000
ramdisk_addr_r=0x8d000000
mp2s: boots left 1
mp2s: loaded 123 bytes at 0xa0000000
mp2s: go 0xa0000000
Loading Environment from MMC... OK
## Starting application at 0xa0000000 ...
8d19a1b8c3c1d0e2f0a4b5c6d7e8f9011223344556677889900aabbccddeeff0  default.bin
Linux version 6.6.78 (oe-user@oe-host) #1 SMP PREEMPT Wed Jan 1 10:11:12 UTC 2026
Zephyr 4.0.0 on stm32mp257f_dk 12:34:56 uptime
loopback 127.0.0.1 any 0.0.0.0 and ::1 and ::
see https://github.com/signal-slot/yakogl and https://www.kernel.org/ docs www.st.com
Found U-Boot script /boot/boot.scr.uimg
/dev/mmcblk1p5 /dev/mmcblk0 boot.scr.uimg stm32mp257f-dk.dtb u-boot.bin mp2_hello.elf README.md
Enter password:
bootcount=1 upgrade_available=0 altbootcmd=run bootcmd
X
run "$T/uboot.txt" > "$T/uboot.out" 2>&1 || fail "U-Boot sample flagged: $(cat "$T/uboot.out")"
runenv "$T/uboot.txt" > "$T/uboot.out" 2>&1 || fail "U-Boot sample flagged with env set: $(cat "$T/uboot.out")"

# environment values
printf 'on lab9box as zed9 via /dev/ttyZZ9\n' > "$T/e.txt"
out=$(runenv "$T/e.txt" || true)
for k in env:BOARD_HOST env:BOARD_USER env:BOARD_UART; do
  case "$out" in *"$k"*) ;; *) fail "$k not flagged: $out";; esac
done
case "$out" in *lab9box*|*zed9*|*ZZ9*) fail "env value printed: $out";; esac
printf 'nothing: zed90 lab9boxes\n' > "$T/e.txt"
runenv "$T/e.txt" >/dev/null || fail "env value inside a longer word flagged"

# directory, stdin, exit codes
mkdir -p "$T/d/sub"
printf 'ok line\n' > "$T/d/a.txt"
printf 'x 192.0.2.9\n' > "$T/d/sub/b.txt"
printf '\0\1\2 192.0.2.9' > "$T/d/bin.dat"
out=$(run "$T/d" 2>/dev/null || true)
case "$out" in *sub/b.txt:1:ipv4:*) ;; *) fail "dir scan: $out";; esac
case "$out" in *bin.dat*) fail "binary scanned";; esac
rc=0; printf 'x 192.0.2.9\n' | run >/dev/null || rc=$?
[ "$rc" = 1 ] || fail "stdin exit $rc"
rc=0; printf 'clean\n' | run - >/dev/null || rc=$?
[ "$rc" = 0 ] || fail "stdin clean exit $rc"
rc=0; run "$T/nonexistent" >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || fail "missing file exit $rc"

# --fix: clean copy, original untouched, never in place
printf 'a 192.0.2.9 b\nok 0xa0000000\nssh h.tail1.ts.net\n' > "$T/f.txt"
cp "$T/f.txt" "$T/f.orig"
rc=0; run --fix "$T/f.out" "$T/f.txt" >/dev/null || rc=$?
[ "$rc" = 1 ] || fail "--fix exit $rc"
cmp -s "$T/f.txt" "$T/f.orig" || fail "--fix changed the input"
run "$T/f.out" >/dev/null || fail "--fix copy still flagged"
grep -q '0xa0000000' "$T/f.out" || fail "--fix lost clean text"
rc=0; run --fix "$T/f.txt" "$T/f.txt" >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || fail "--fix in place not refused ($rc)"
cmp -s "$T/f.txt" "$T/f.orig" || fail "in-place refusal modified input"
rc=0; run --fix "$T/d/inner" "$T/d" >/dev/null 2>&1 || rc=$?
[ "$rc" = 2 ] || fail "--fix inside input not refused ($rc)"
run --fix "$T/dout" "$T/d" >/dev/null 2>&1 || true
run "$T/dout" >/dev/null || fail "--fix dir copy still flagged"
[ -f "$T/dout/a.txt" ] || fail "--fix dir copy missing file"

if [ "$fails" -ne 0 ]; then echo "mp2s_redact_check selftest: $fails failure(s)" >&2; exit 1; fi
echo "mp2s_redact_check selftest: ok"
