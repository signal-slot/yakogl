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
env2() { python3 -I "$here/uboot_env.py" --size 0x2000 --offset 0x2000 --offset2 0 "$@"; }
envok() { env2 check "$1" >/dev/null; }

"$fb" init "$D" || { echo "init failed"; exit 1; }
eval "$("$fb" shell "$D")"

t "boot tree" test -s "$D/boot/extlinux/extlinux.conf" -a -s "$D/boot/Image" -a -s "$D/boot/stm32mp257f-dk.dtb"
t "saved prefix extlinux" test -s "$D/boot/mmc0_extlinux/stm32mp257f-dk_extlinux.conf"
t "env pair valid" envok "$D/env/u-boot-env.bin"
t "stock bootcmd" test "$(fw_printenv -n bootcmd)" = "run bootcmd_stm32mp"
t "no bootcount (BOOTCOUNT_LIMIT off)" bash -c '! fw_printenv -n bootcount >/dev/null 2>&1'
t "stock altbootcmd" test "$(fw_printenv -n altbootcmd)" = "run bootcmd_stm32mp"
t "stock boot_prefixes" test "$(fw_printenv -n boot_prefixes)" = "/mmc0_"
t "saved boot_syslinux_conf" test "$(fw_printenv -n boot_syslinux_conf)" = extlinux/stm32mp257f-dk_extlinux.conf
t "stock scriptaddr" test "$(fw_printenv -n scriptaddr)" = 0x90100000
t "no bootfile" bash -c '! fw_printenv -n bootfile >/dev/null 2>&1'
t "findmnt source" test "$(findmnt -n -o SOURCE /boot)" = /dev/mmcblk1p8
chk "$(lsblk)" 'bootfs     part bootfs       64M  /boot' "lsblk bootfs on /boot"
chk "$(blkid /dev/mmcblk1p8)" 'LABEL="bootfs"' "blkid bootfs"
t "ssh shim first in PATH" test "$(command -v ssh)" = "$D/bin/ssh"
t "sudo shim first in PATH" test "$(command -v sudo)" = "$D/bin/sudo"
t "ssh sd type" test "$(ssh "$H" 'cat /sys/block/mmcblk1/device/type')" = SD
t "ssh emmc type" test "$(ssh "$H" 'cat /sys/block/mmcblk0/device/type')" = MMC
t "log empty after init" test ! -s "$D/writes.log"

# allowed writes
t "setenv boot_prefixes" fw_setenv boot_prefixes "/yakogl/standalone/ /mmc0_"
t "setenv read back" test "$(fw_printenv -n boot_prefixes)" = "/yakogl/standalone/ /mmc0_"
t "env pair still valid" envok "$D/env/u-boot-env.bin"
t "setenv mp2s_left" fw_setenv mp2s_left 2
t "unset mp2s_left" fw_setenv mp2s_left
t "unset gone" bash -c '! fw_printenv -n mp2s_left >/dev/null 2>&1'
t "unset absent ok" fw_setenv mp2s_absent
t "setenv -s stdin" bash -c 'printf "mp2s_ver 1\nmp2s_left 2\n# note\n\nmp2s_prefixes /mmc0_\n" | fw_setenv -s -'
t "setenv -s read back" test "$(fw_printenv -n mp2s_left)/$(fw_printenv -n mp2s_ver)" = "2/1"
t "ssh writes file" ssh -o BatchMode=yes "$H" 'mkdir -p /boot/yakogl/standalone && echo hi > /boot/yakogl/standalone/default.bin && sync'
t "file landed in DIR/boot" test "$(cat "$D/boot/yakogl/standalone/default.bin")" = hi
t "ssh read-only ok" ssh "$H" 'cat /boot/extlinux/extlinux.conf >/dev/null'
t "ssh exit status passes" bash -c "! ssh $H 'exit 3'"
t "ssh sudo fw_setenv -s" bash -c "printf 'mp2s_ver\nmp2s_left\nmp2s_prefixes\n' | ssh $H 'sudo -n fw_setenv -s -'"
t "ssh sudo unset gone" bash -c '! fw_printenv -n mp2s_ver >/dev/null 2>&1'
t "ssh sudo rm" ssh "$H" 'sudo -n rm -f -- /boot/yakogl/standalone/default.bin'
t "put under /boot" bash -c "echo x | '$fb' put '$D' /boot/yakogl/standalone/boot.scr.uimg"
t "ssh reboot logged only" ssh "$H" 'sudo -n systemctl reboot'
# the runner's staging: tee to .part, then mv, image on stdin, a comment after
t "ssh tee+mv staging" bash -c "printf img | timeout 60 ssh -o BatchMode=yes $H 'sudo -n tee -- /boot/yakogl/standalone/s.bin.part >/dev/null && sudo -n mv -- /boot/yakogl/standalone/s.bin.part /boot/yakogl/standalone/s.bin   # < image'"
t "staged file landed" test "$(cat "$D/boot/yakogl/standalone/s.bin")" = img
t "ssh fd redirects ok" ssh "$H" 'ls /boot/nothing 2>/dev/null; echo x 2>&1 >/dev/null; true'
t "ssh multi-name fw_printenv" bash -c "ssh $H 'sudo -n fw_printenv boot_prefixes mp2s_absent' 2>&1 | grep -qF 'not defined'"

w=$("$fb" writes "$D")
chk "$w" "setenv boot_prefixes /yakogl/standalone/ /mmc0_" "log setenv"
chk "$w" "file /boot/yakogl/standalone/default.bin $(printf 'hi\n' | sha256sum | cut -d' ' -f1)" "log file sha256"
chk "$w" "file /boot/yakogl/standalone/default.bin removed" "log file removed"
chk "$w" "setenv mp2s_prefixes /mmc0_" "log setenv -s"
chk "$w" "file /boot/yakogl/standalone/boot.scr.uimg $(printf 'x\n' | sha256sum | cut -d' ' -f1)" "log put"
chk "$w" "reboot" "log reboot"
lacks "$w" VIOLATION "no violation yet"

# forbidden writes: refused, nothing written, logged as VIOLATION
for v in bootcmd bootcount altbootcmd bootlimit boot_targets mp2z_go; do
    before=$(fw_printenv -n "$v" 2>/dev/null)
    if fw_setenv "$v" evil 2>/dev/null; then bad "refuse $v"; else ok "refuse $v"; fi
    t "$v unchanged" test "$(fw_printenv -n "$v" 2>/dev/null)" = "$before"
done
env_before=$(sha256sum < "$D/env/u-boot-env.bin")
if printf 'mp2s_left 5\nbootcount 9\n' | fw_setenv -s - 2>/dev/null; then bad "refuse -s with bootcount"; else ok "refuse -s with bootcount"; fi
t "-s refused writes nothing" test "$(sha256sum < "$D/env/u-boot-env.bin")" = "$env_before"
if ssh "$H" 'echo x > stray.txt' 2>/dev/null; then bad "refuse ssh write outside boot"; else ok "refuse ssh write outside boot"; fi
if ssh "$H" "echo x > $work/host.txt" 2>/dev/null; then bad "refuse ssh write to host"; else ok "refuse ssh write to host"; fi
t "host write not run" test ! -e "$work/host.txt"
echo keep > "$work/keep.txt"
if ssh "$H" "sudo -n tee -- $work/tee.txt >/dev/null" </dev/null 2>/dev/null; then bad "refuse tee -- to host"; else ok "refuse tee -- to host"; fi
t "tee to host not run" test ! -e "$work/tee.txt"
if ssh "$H" "sudo -n rm -rf -- $work/keep.txt" 2>/dev/null; then bad "refuse rm on host"; else ok "refuse rm on host"; fi
t "rm on host not run" test -e "$work/keep.txt"
if ssh "$H" "sudo -n mv -- /boot/Image $work/Image" 2>/dev/null; then bad "refuse mv out of boot"; else ok "refuse mv out of boot"; fi
t "mv out of boot not run" test -e "$D/boot/Image"
env_before=$(sha256sum < "$D/env/u-boot-env.bin")
if ssh "$H" 'cat /dev/zero | dd of=env/u-boot-env.bin bs=1 count=4 conv=notrunc' 2>/dev/null; then bad "refuse dd on env"; else ok "refuse dd on env"; fi
t "env untouched by dd" test "$(sha256sum < "$D/env/u-boot-env.bin")" = "$env_before"
if ssh "$H" "echo 'unterminated" 2>/dev/null; then bad "refuse unparsable"; else ok "refuse unparsable"; fi
t "outside write refused (put)" bash -c "! echo x | '$fb' put '$D' /etc/passwd 2>/dev/null"
t "dotdot refused (put)" bash -c "! echo x | '$fb' put '$D' /boot/../x 2>/dev/null"
w=$("$fb" writes "$D")
for v in bootcmd bootcount altbootcmd bootlimit boot_targets mp2z_go; do
    chk "$w" "VIOLATION setenv $v evil" "VIOLATION logged $v"
done
chk "$w" "VIOLATION setenv bootcount 9" "VIOLATION logged -s bootcount"
lacks "$w" "setenv mp2s_left 5" "refused -s not logged as a write"
chk "$w" "VIOLATION file /stray.txt" "VIOLATION logged outside file"
chk "$w" "VIOLATION file $work/host.txt" "VIOLATION logged host write"
chk "$w" "VIOLATION file /etc/passwd" "VIOLATION logged put"
chk "$w" "VIOLATION file $work/tee.txt" "VIOLATION logged tee --"
chk "$w" "VIOLATION file $work/keep.txt" "VIOLATION logged rm"
chk "$w" "VIOLATION file /env/u-boot-env.bin" "VIOLATION logged dd on env"
chk "$w" "VIOLATION ssh command does not parse" "VIOLATION logged unparsable"

if [ $fail = 0 ]; then echo "mp2s_fakeboard selftest: all ok"; else echo "mp2s_fakeboard selftest: $fail failed"; exit 1; fi
