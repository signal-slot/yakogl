#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# No-board selftest of tools/mp2s_fakeboard.sh. The ssh target is the
# placeholder root@BOARD; the ssh shim never leaves the host.
#
#   tools/mp2s_fakeboard_selftest.sh
#
# Prints "mp2s_fakeboard selftest: all ok"; exit 0 only then.
set -u
here=$(dirname "$(readlink -f "$0")")
fb=$here/mp2s_fakeboard.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_fb.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
D=$work/dk
H=root@BOARD
fail=0
ok()  { echo "ok   $1"; }
bad() { echo "FAIL $1"; fail=$((fail+1)); }
t()   { local n=$1; shift; if "$@"; then ok "$n"; else bad "$n"; fi; }
has() { grep -qF -- "$2" <<< "$1"; }
chk() { if has "$1" "$2"; then ok "$3"; else bad "$3"; fi; }
lacks() { if has "$1" "$2"; then bad "$3"; else ok "$3"; fi; }

"$fb" init "$D" || { echo "init failed"; exit 1; }
eval "$("$fb" shell "$D")"

t "boot tree" test -s "$D/boot/extlinux/extlinux.conf" -a -s "$D/boot/Image" -a -s "$D/boot/stm32mp257f-dk.dtb"
t "env image valid" python3 -I "$here/uboot_env.py" check "$D/env/u-boot-env.bin"
t "stock bootcmd" test "$(fw_printenv -n bootcmd)" = "run bootcmd_stm32mp"
t "stock bootcount" test "$(fw_printenv -n bootcount)" = 0
t "stock altbootcmd" test "$(fw_printenv -n altbootcmd)" = "run bootcmd_stm32mp"
t "stock boot_prefixes" test "$(fw_printenv -n boot_prefixes)" = "/mmc0_"
t "findmnt source" test "$(findmnt -n -o SOURCE /boot)" = /dev/mmcblk0p8
chk "$(lsblk)" 'bootfs' "lsblk bootfs"
chk "$(blkid /dev/mmcblk0p8)" 'LABEL="bootfs"' "blkid bootfs"
t "ssh shim first in PATH" test "$(command -v ssh)" = "$D/bin/ssh"
t "sudo shim first in PATH" test "$(command -v sudo)" = "$D/bin/sudo"

# allowed writes
t "setenv boot_prefixes" fw_setenv boot_prefixes "/yakogl/standalone/ /mmc0_"
t "setenv read back" test "$(fw_printenv -n boot_prefixes)" = "/yakogl/standalone/ /mmc0_"
t "setenv mp2s_left" fw_setenv mp2s_left 2
t "unset mp2s_left" fw_setenv mp2s_left
t "unset gone" bash -c '! fw_printenv -n mp2s_left >/dev/null 2>&1'
t "unset absent ok" fw_setenv mp2s_absent
t "setenv -s stdin" bash -c 'printf "mp2s_ver 1\nmp2s_left 2\n# note\n\nmp2s_prefixes /mmc0_\n" | fw_setenv -s -'
t "setenv -s read back" test "$(fw_printenv -n mp2s_left)/$(fw_printenv -n mp2s_ver)" = "2/1"
t "ssh writes file" ssh -o BatchMode=yes "$H" 'mkdir -p /boot/yakogl/standalone && echo hi > /boot/yakogl/standalone/default.bin'
t "file landed in DIR/boot" test "$(cat "$D/boot/yakogl/standalone/default.bin")" = hi
t "ssh read-only ok" ssh "$H" 'cat /boot/extlinux/extlinux.conf >/dev/null'
t "ssh exit status passes" bash -c "! ssh $H 'exit 3'"
t "ssh sudo fw_setenv -s" bash -c "printf 'mp2s_ver\nmp2s_left\nmp2s_prefixes\n' | ssh $H 'sudo -n fw_setenv -s -'"
t "ssh sudo unset gone" bash -c '! fw_printenv -n mp2s_ver >/dev/null 2>&1'
t "ssh sudo rm" ssh "$H" 'sudo -n rm -f -- /boot/yakogl/standalone/default.bin'
t "put under /boot" bash -c "echo x | '$fb' put '$D' /boot/yakogl/standalone/boot.scr.uimg"

w=$("$fb" writes "$D")
chk "$w" "setenv boot_prefixes /yakogl/standalone/ /mmc0_" "log setenv"
chk "$w" "file /boot/yakogl/standalone/default.bin $(printf 'hi\n' | sha256sum | cut -d' ' -f1)" "log file sha256"
chk "$w" "file /boot/yakogl/standalone/default.bin removed" "log file removed"
chk "$w" "setenv mp2s_prefixes /mmc0_" "log setenv -s"
chk "$w" "file /boot/yakogl/standalone/boot.scr.uimg " "log put"
lacks "$w" VIOLATION "no violation yet"

# forbidden writes
for v in bootcmd bootcount bootlimit altbootcmd; do
    before=$(fw_printenv -n "$v")
    if fw_setenv "$v" evil 2>/dev/null; then bad "refuse $v"; else ok "refuse $v"; fi
    t "$v unchanged" test "$(fw_printenv -n "$v")" = "$before"
done
if printf 'mp2s_left 5\nbootcount 9\n' | fw_setenv -s - 2>/dev/null; then bad "refuse -s with bootcount"; else ok "refuse -s with bootcount"; fi
t "-s refused writes nothing" bash -c '! fw_printenv -n mp2s_left >/dev/null 2>&1'
if ssh "$H" 'echo x > stray.txt' 2>/dev/null; then bad "refuse ssh write outside boot"; else ok "refuse ssh write outside boot"; fi
t "outside write refused (put)" bash -c "! echo x | '$fb' put '$D' /etc/passwd 2>/dev/null"
t "dotdot refused (put)" bash -c "! echo x | '$fb' put '$D' /boot/../x 2>/dev/null"
w=$("$fb" writes "$D")
chk "$w" "VIOLATION setenv bootcmd evil" "VIOLATION logged bootcmd"
chk "$w" "VIOLATION setenv bootcount evil" "VIOLATION logged bootcount"
chk "$w" "VIOLATION setenv bootlimit evil" "VIOLATION logged bootlimit"
chk "$w" "VIOLATION setenv altbootcmd evil" "VIOLATION logged altbootcmd"
chk "$w" "VIOLATION setenv bootcount 9" "VIOLATION logged -s bootcount"
lacks "$w" "setenv mp2s_left 5" "refused -s not logged as a write"
chk "$w" "VIOLATION file /stray.txt" "VIOLATION logged outside file"
chk "$w" "VIOLATION file /etc/passwd" "VIOLATION logged put"

if [ $fail = 0 ]; then echo "mp2s_fakeboard selftest: all ok"; else echo "mp2s_fakeboard selftest: $fail failed"; exit 1; fi
