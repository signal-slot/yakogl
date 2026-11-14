#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# No-board selftest of tools/mp2s_fakeboard.sh.
set -u
here=$(dirname "$(readlink -f "$0")")
fb=$here/mp2s_fakeboard.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_fb.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
D=$work/dk
fail=0
ok()  { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=$((fail+1)); }
t()   { local n=$1; shift; if "$@"; then ok "$n"; else bad "$n"; fi; }
has() { grep -qF -- "$2" <<< "$1"; }

"$fb" init "$D" || { echo "init failed"; exit 1; }
eval "$("$fb" shell "$D")"

t "boot tree" test -s "$D/boot/extlinux/extlinux.conf" -a -s "$D/boot/Image" -a -s "$D/boot/stm32mp257f-dk.dtb"
t "env image valid" bash -c "python3 -I \"$here/uboot_env.py\" check \"$D/env/u-boot-env.bin\" >/dev/null"
t "stock bootcmd" test "$(fw_printenv -n bootcmd)" = "run bootcmd_stm32mp"
t "stock bootcount" test "$(fw_printenv -n bootcount)" = 0
t "stock altbootcmd" test "$(fw_printenv -n altbootcmd)" = "run bootcmd_stm32mp"
t "stock boot_prefixes" test "$(fw_printenv -n boot_prefixes)" = "/mmc0_"
t "findmnt source" test "$(findmnt -n -o SOURCE /boot)" = /dev/mmcblk0p8
has "$(lsblk)" 'bootfs' && ok "lsblk bootfs" || bad "lsblk bootfs"
has "$(blkid /dev/mmcblk0p8)" 'LABEL="bootfs"' && ok "blkid bootfs" || bad "blkid bootfs"
[ "$(command -v ssh)" = "$D/bin/ssh" ] && ok "ssh shim first in PATH" || bad "ssh shim first in PATH"

# allowed writes
t "setenv boot_prefixes" fw_setenv boot_prefixes "/yakogl/standalone/ /mmc0_"
t "setenv read back" test "$(fw_printenv -n boot_prefixes)" = "/yakogl/standalone/ /mmc0_"
t "setenv mp2s_left" fw_setenv mp2s_left 2
t "unset mp2s_left" fw_setenv mp2s_left
t "unset gone" bash -c '! fw_printenv -n mp2s_left >/dev/null 2>&1'
t "ssh writes file" ssh -o BatchMode=yes root@fake 'mkdir -p /boot/yakogl/standalone && echo hi > /boot/yakogl/standalone/default.bin'
t "file landed in DIR/boot" test "$(cat "$D/boot/yakogl/standalone/default.bin")" = hi
t "ssh read-only ok" ssh root@fake 'cat /boot/extlinux/extlinux.conf >/dev/null'
t "ssh exit status passes" bash -c '! ssh root@fake "exit 3"'
t "put under /boot" bash -c "echo x | '$fb' put '$D' /boot/yakogl/standalone/boot.scr.uimg"

w=$("$fb" writes "$D")
sum=$(sha256sum < "$D/boot/yakogl/standalone/default.bin" | cut -d' ' -f1)
has "$w" "setenv boot_prefixes /yakogl/standalone/ /mmc0_" && ok "log setenv" || bad "log setenv"
has "$w" "file /boot/yakogl/standalone/default.bin $sum" && ok "log file sha256" || bad "log file sha256"
has "$w" "file /boot/yakogl/standalone/boot.scr.uimg " && ok "log put" || bad "log put"
has "$w" VIOLATION && bad "no violation yet" || ok "no violation yet"

# forbidden writes
for v in bootcmd bootcount altbootcmd; do
    before=$(fw_printenv -n "$v")
    if fw_setenv "$v" evil 2>/dev/null; then bad "refuse $v"; else ok "refuse $v"; fi
    t "$v unchanged" test "$(fw_printenv -n "$v")" = "$before"
done
if ssh root@fake 'echo x > stray.txt' 2>/dev/null; then bad "refuse ssh write outside boot"; else ok "refuse ssh write outside boot"; fi
t "outside write refused (put)" bash -c "! echo x | '$fb' put '$D' /etc/passwd 2>/dev/null"
t "dotdot refused (put)" bash -c "! echo x | '$fb' put '$D' /boot/../x 2>/dev/null"
w=$("$fb" writes "$D")
has "$w" "VIOLATION setenv bootcmd evil" && ok "VIOLATION logged bootcmd" || bad "VIOLATION logged bootcmd"
has "$w" "VIOLATION setenv bootcount" && ok "VIOLATION logged bootcount" || bad "VIOLATION logged bootcount"
has "$w" "VIOLATION setenv altbootcmd" && ok "VIOLATION logged altbootcmd" || bad "VIOLATION logged altbootcmd"
has "$w" "VIOLATION file /stray.txt" && ok "VIOLATION logged outside file" || bad "VIOLATION logged outside file"
has "$w" "VIOLATION file /etc/passwd" && ok "VIOLATION logged put" || bad "VIOLATION logged put"

if [ $fail = 0 ]; then echo "mp2s_fakeboard selftest: all ok"; else echo "mp2s_fakeboard selftest: $fail failed"; exit 1; fi
