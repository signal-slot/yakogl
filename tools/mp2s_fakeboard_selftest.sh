#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_fakeboard_selftest.sh - no-board selftest of tools/mp2s_fakeboard.sh:
# init, the shims through PATH and through the ssh shim with the commands the
# standalone runner sends, the merged read-only helpers against the fake, the
# writes log, and the refusals (a protected variable, a write outside /boot,
# a raw write of the environment, a command that does not parse). Everything
# happens in a mktemp directory under $TMPDIR. The ssh target is assembled
# from two words at run time; no login@host literal is in this file.
#
#   tools/mp2s_fakeboard_selftest.sh
#
# Prints one line per check and "mp2s_fakeboard selftest: N ok, 0 failed";
# exit 0 only then.
set -u
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/.." && pwd)
fb=$here/mp2s_fakeboard.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_fb.XXXXXX") || exit 2
trap 'rm -rf "$work"' EXIT
D=$work/dk
u=tester
h=fakeboard
H=$u@$h
LAY=(--redundant --size 0x2000 --offset 0x7e000 --offset2 0x7c000)
ENV=$D/dev/mmcblk1p7
npass=0
nfail=0
ok()  { npass=$((npass + 1)); echo "ok   $1"; }
bad() { nfail=$((nfail + 1)); echo "FAIL $1${2:+: $2}"; }
t()   { local n=$1; shift; if "$@" >/dev/null 2>&1; then ok "$n"; else bad "$n"; fi; }
refused() { local n=$1; shift; if "$@" >/dev/null 2>&1; then bad "$n" "not refused"; else ok "$n"; fi; }
has() { grep -qF -- "$2" <<< "$1"; }
chk() { if has "$1" "$2"; then ok "$3"; else bad "$3" "no '$2'"; fi; }
lacks() { if has "$1" "$2"; then bad "$3" "has '$2'"; else ok "$3"; fi; }
envpy() { python3 -I "$here/uboot_env.py" "${LAY[@]}" "$@"; }
sha() { sha256sum < "$1" | cut -d' ' -f1; }
# the sha256 of copy 1 and copy 2 of the environment partition
copies() { { tail -c +$((0x7e000 + 1)) "$ENV" | head -c 8192 | sha256sum; tail -c +$((0x7c000 + 1)) "$ENV" | head -c 8192 | sha256sum; } | cut -d' ' -f1 | tr '\n' ' '; }
# one_write BEFORE AFTER: exactly one of the two copies changed
one_write() {
    local b1=${1%% *} b2=${1#* } a1=${2%% *} a2=${2#* }
    b2=${b2% } a2=${a2% }
    { [ "$b1" = "$a1" ] && [ "$b2" != "$a2" ]; } || { [ "$b1" != "$a1" ] && [ "$b2" = "$a2" ]; }
}

# ---- init and the layout
"$fb" init "$D" >/dev/null || { echo "FAIL init"; exit 1; }
t "init prints the usage on a wrong verb" bash -c "! '$fb' frobnicate"
for f in boot/Image.gz boot/stm32mp257f-dk.dtb boot/boot.scr.uimg boot/extlinux/extlinux.conf \
    boot/mmc0_extlinux/extlinux.conf boot/mmc1_extlinux/extlinux.conf dev/mmcblk1p7 dev/mmcblk0p7 \
    dev/mmcblk1p3 captured/parts.tsv captured/findmnt.txt captured/lsblk.txt captured/blkid.txt \
    etc/fw_env.config proc/self/mountinfo sys/block/mmcblk1/device/type README.txt; do
    t "init made $f" test -s "$D/$f"
done
t "init made an empty writes.log" bash -c "test -f '$D/writes.log' && ! test -s '$D/writes.log'"
t "the env partition is 512 KiB" test "$(stat -c %s "$ENV")" = 524288
t "both env copies valid" envpy check "$ENV"
t "the eMMC env differs from the card's" bash -c "! cmp -s '$ENV' '$D/dev/mmcblk0p7'"

# ---- shell: the shims first in PATH
eval "$("$fb" shell "$D")"
t "shell exports MP2S_FAKE_ROOT" test "${MP2S_FAKE_ROOT:-}" = "$D"
for s in fw_printenv fw_setenv findmnt lsblk blkid ssh sudo sync uname reboot systemctl; do
    t "shim $s first in PATH" test "$(command -v "$s")" = "$D/bin/$s"
done

# ---- the stock environment (note, sections 2 and 3)
t "stock bootcmd" test "$(fw_printenv -n bootcmd)" = "run bootcmd_stm32mp"
t "stock altbootcmd" test "$(fw_printenv -n altbootcmd)" = "run bootcmd_stm32mp"
t "saved boot_prefixes" test "$(fw_printenv -n boot_prefixes)" = "/mmc0_"
t "saved boot_targets" test "$(fw_printenv -n boot_targets)" = "mmc0"
t "saved boot_syslinux_conf" test "$(fw_printenv -n boot_syslinux_conf)" = "extlinux/extlinux.conf"
t "stock scriptaddr" test "$(fw_printenv -n scriptaddr)" = "0x90100000"
t "no bootcount (CONFIG_BOOTCOUNT_LIMIT off)" bash -c '! fw_printenv -n bootcount'
t "no mp2s_* at init" bash -c '! fw_printenv | grep -q "^mp2s_"'
t "fw_printenv NAME=VALUE form" test "$(fw_printenv bootcmd)" = "bootcmd=run bootcmd_stm32mp"
t "fw_printenv lists 54 variables" test "$(fw_printenv | wc -l)" = 54
t "fw_printenv equals the partition read" bash -c "fw_printenv | sort | cmp -s - <(python3 -I '$here/uboot_env.py' ${LAY[*]} print '$ENV' | sort)"
t "a missing name is an error" bash -c '! fw_printenv -n nosuchvar'
if [ -f "$here/mp2s_guard.py" ]; then
    fw_printenv > "$work/pe.txt"
    t "mp2s_guard.py --for standalone allows the stock env" python3 -I "$here/mp2s_guard.py" --for standalone --printenv "$work/pe.txt"
fi

# ---- storage, in the runner's and the bootfs check's forms
t "findmnt -n -o SOURCE /boot" test "$(findmnt -n -o SOURCE /boot)" = /dev/mmcblk1p8
t "findmnt -n -v -o SOURCE,FSTYPE -T /boot" test "$(findmnt -n -v -o SOURCE,FSTYPE -T /boot)" = "/dev/mmcblk1p8 ext4"
t "findmnt -T / is the rootfs" test "$(findmnt -n -v -o SOURCE,FSTYPE -T /)" = "/dev/mmcblk1p10 ext4"
t "findmnt -T a file under /boot" test "$(findmnt -n -o TARGET -T /boot/yakogl/x)" = /boot
chk "$(findmnt /boot)" "TARGET SOURCE" "findmnt /boot has a header"
t "findmnt of an unmounted path fails" bash -c '! findmnt /nowhere'
tbl=$(lsblk -rno PATH,PARTLABEL -- /dev/mmcblk1)
chk "$tbl" "/dev/mmcblk1p7 u-boot-env" "lsblk -rno PATH,PARTLABEL: u-boot-env"
chk "$tbl" "/dev/mmcblk1p8 bootfs" "lsblk -rno PATH,PARTLABEL: bootfs"
chk "$tbl" "/dev/mmcblk1p3 metadata1" "lsblk -rno PATH,PARTLABEL: metadata1"
lacks "$tbl" "mmcblk0" "lsblk of the card lists no eMMC partition"
t "lsblk -rno lists the card and 11 partitions" test "$(wc -l <<< "$tbl")" = 12
chk "$(lsblk -P -o NAME,PKNAME,TYPE,FSTYPE,LABEL,PARTLABEL)" 'NAME="mmcblk1p8" PKNAME="mmcblk1" TYPE="part" FSTYPE="ext4" LABEL="bootfs" PARTLABEL="bootfs"' "lsblk -P pairs"
chk "$(lsblk)" "/boot" "lsblk shows /boot mounted"
t "lsblk -dno TYPE of the disk" test "$(lsblk -dno TYPE -- /dev/mmcblk1)" = disk
t "lsblk of an unknown device fails" bash -c '! lsblk /dev/nosuch'
chk "$(blkid /dev/mmcblk1p8)" 'LABEL="bootfs"' "blkid LABEL"
chk "$(blkid /dev/mmcblk1p8)" 'TYPE="ext4"' "blkid TYPE"
t "blkid -s LABEL -o value" test "$(blkid -s LABEL -o value /dev/mmcblk1p8)" = bootfs
chk "$(blkid)" "/dev/mmcblk1p7: PARTLABEL=\"u-boot-env\"" "blkid lists the env partition"
t "blkid of an unknown device fails" bash -c '! blkid /dev/nosuch'
lacks "$(blkid; lsblk; findmnt)" "ts.net" "no tailnet name in the captured storage"
if [ -f "$here/mp2s_bootfs_check.py" ]; then
    c=$D/captured/bootfs-check
    t "mp2s_bootfs_check.py accepts the fake's captured storage" python3 -I "$here/mp2s_bootfs_check.py" \
        --findmnt "$c/findmnt.txt" --lsblk "$c/lsblk.txt" --blkid "$c/blkid.txt" --disk-type "$c/disk-type.txt"
fi

# ---- the ssh shim: the runner's reads
t "ssh with options and sudo true" ssh -o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=10 -o BatchMode=yes "$H" 'sudo -n true'
t "ssh passes the exit status" bash -c "ssh '$H' 'exit 3'; test \$? = 3"
t "ssh without a command is refused" bash -c "! ssh '$H'"
t "ssh: the card is SD" test "$(ssh "$H" 'cat -- /sys/block/mmcblk1/device/type')" = SD
t "ssh: the eMMC is MMC" test "$(ssh "$H" 'cat /sys/block/mmcblk0/device/type')" = MMC
t "ssh: findmnt" test "$(ssh "$H" 'findmnt -n -o SOURCE /boot')" = /dev/mmcblk1p8
t "ssh: lsblk table" test "$(ssh "$H" 'lsblk -rno PATH,PARTLABEL -- /dev/mmcblk1')" = "$tbl"
t "ssh: an empty stage dir lists nothing" test -z "$(ssh "$H" 'ls -A -- /boot/yakogl/standalone 2>/dev/null; true')"
t "ssh: fw_printenv under sudo" test "$(ssh "$H" 'sudo -n fw_printenv' | wc -l)" = 54
t "ssh: head of the env partition equals the file" bash -c "ssh '$H' 'sudo -n head -c 1048576 -- /dev/mmcblk1p7' | cmp -s - '$ENV'"
t "ssh: the /boot listing" bash -c "ssh '$H' \"sudo -n sh -c 'cd /boot && find . -type f -print0 | sort -z | xargs -0 sha256sum'\" | grep -q ' ./Image.gz\$'"
t "ssh: df of /boot is a number" bash -c "ssh '$H' 'df -B1 --output=avail /boot | tail -n 1' | grep -Eq '^[0-9]+\$'"
t "ssh: uname -r answers" test -n "$(ssh "$H" 'uname -r')"
t "ssh: a boot id" test "$(ssh "$H" 'cat /proc/sys/kernel/random/boot_id | wc -c')" = 37
ssh "$H" 'sudo -n head -c 1048576 -- /dev/mmcblk1p3' > "$work/fwu1.bin"
ssh "$H" 'sudo -n head -c 1048576 -- /dev/mmcblk1p4' > "$work/fwu2.bin"
if [ -f "$here/fwu_metadata.py" ]; then
    t "fwu_metadata.py: bank 0 accepted" bash -c "python3 -I '$here/fwu_metadata.py' check '$work/fwu1.bin' '$work/fwu2.bin' | grep -q accepted"
fi
if [ -f "$here/mp2s_fwenv_check.py" ]; then
    t "mp2s_fwenv_check.py on the fake (python3 -I - --root /)" bash -c "ssh '$H' 'python3 -I - --root /' < '$here/mp2s_fwenv_check.py' | grep -q '^fwenv: ok'"
fi
if [ -f "$here/mp2s_comeback.sh" ]; then
    out=$(BOARD_HOST=$H timeout 120 bash "$here/mp2s_comeback.sh" 2>&1)
    if [ $? = 0 ] && has "$out" "comeback: ok"; then ok "mp2s_comeback.sh says OpenSTLinux is back on the fake"; else bad "mp2s_comeback.sh on the fake" "$out"; fi
fi
t "nothing logged by the reads" test ! -s "$D/writes.log"

# ---- the runner's writes: stage, read back, one environment write
printf 'image bytes' > "$work/img"
t "ssh: tee .part, sync, mv (the runner's staging)" bash -c "timeout 60 ssh -o BatchMode=yes '$H' 'sudo -n mkdir -p -- /boot/yakogl/standalone && sudo -n tee -- /boot/yakogl/standalone/default.bin.part >/dev/null && sudo -n sync && sudo -n mv -- /boot/yakogl/standalone/default.bin.part /boot/yakogl/standalone/default.bin && sudo -n sync' < '$work/img'"
t "the staged file landed under DIR/boot" cmp -s "$work/img" "$D/boot/yakogl/standalone/default.bin"
t "ssh: drop_caches is a sink, sha256sum reads back" test "$(ssh "$H" "sudo -n sh -c 'echo 3 > /proc/sys/vm/drop_caches' && sha256sum -- /boot/yakogl/standalone/default.bin" | cut -d' ' -f1)" = "$(sha "$work/img")"
t "ssh: ls -l of the stage dir" bash -c "ssh '$H' 'ls -l -- /boot/yakogl/standalone' | grep -q default.bin"
before=$(copies)
ssh "$H" 'sudo -n head -c 1048576 -- /dev/mmcblk1p7' > "$work/before.bin"
t "ssh: one fw_setenv -s - of the default write" bash -c "printf 'mp2s_ver=1\nmp2s_prefixes=/mmc0_\nboot_prefixes=/yakogl/standalone/ /mmc0_\nmp2s_left=2\n' | ssh '$H' 'sudo -n fw_setenv -s -'"
after=$(copies)
t "the default write changed exactly one copy" one_write "$before" "$after"
t "both copies still valid" envpy check "$ENV"
t "mp2s_left read back" test "$(fw_printenv -n mp2s_left)" = 2
t "boot_prefixes read back" test "$(fw_printenv -n boot_prefixes)" = "/yakogl/standalone/ /mmc0_"
t "the four names in one fw_printenv" test "$(ssh "$H" 'sudo -n fw_printenv mp2s_ver mp2s_left mp2s_prefixes boot_prefixes' | wc -l)" = 4
if [ -f "$here/mp2s_env_plan.py" ]; then
    ssh "$H" 'sudo -n head -c 1048576 -- /dev/mmcblk1p7' > "$work/after.bin"
    t "mp2s_env_plan.py verify --expect default on the partition before and after" \
        python3 -I "$here/mp2s_env_plan.py" verify "$work/before.bin" "$work/after.bin" --expect default "${LAY[@]}"
fi
before=$after
t "fw_setenv -s with sets and deletes (nodefault)" bash -c "printf 'boot_prefixes=/mmc0_\nmp2s_ver=\nmp2s_left=\nmp2s_prefixes=\n' | ssh '$H' 'sudo -n fw_setenv -s -'"
after=$(copies)
t "the nodefault write changed exactly one copy" one_write "$before" "$after"
t "mp2s_* gone" bash -c '! fw_printenv | grep -q "^mp2s_"'
t "boot_prefixes restored" test "$(fw_printenv -n boot_prefixes)" = "/mmc0_"
t "fw_setenv NAME VALUE (the CLI form)" fw_setenv mp2s_left 3
t "fw_setenv NAME VALUE read back" test "$(fw_printenv -n mp2s_left)" = 3
t "fw_setenv -s with NAME VALUE lines (U-Boot's form)" bash -c "printf '# a comment\n\nmp2s_ver 1\nmp2s_left\n' | fw_setenv -s -"
t "NAME VALUE line set" test "$(fw_printenv -n mp2s_ver)" = 1
t "a bare NAME line deletes" bash -c '! fw_printenv -n mp2s_left'
t "fw_setenv of an absent name deletes nothing and succeeds" fw_setenv mp2s_absent
t "fw_setenv -s FILE" bash -c "printf 'mp2s_ver=\n' > '$work/s.txt' && fw_setenv -s '$work/s.txt'"
t "ssh: rm -f of the staged file" ssh "$H" 'sudo -n rm -f -- /boot/yakogl/standalone/default.bin'
t "the staged file is gone" test ! -e "$D/boot/yakogl/standalone/default.bin"
t "ssh: rm -rf of the stage dir" ssh "$H" 'sudo -n rm -rf -- /boot/yakogl/standalone'
t "ssh: cp inside /boot" ssh "$H" 'sudo -n cp -- /boot/Image.gz /boot/Image.copy'
t "ssh: rm of the copy" ssh "$H" 'sudo -n rm -- /boot/Image.copy'
t "ssh: a file made and removed in one command leaves no trace" ssh "$H" 'sudo -n cp -- /boot/Image.gz /boot/Image.tmp && sudo -n rm -- /boot/Image.tmp'
t "ssh: a multi-line command" test "$(ssh "$H" $'echo "a=1"\necho "b=$(uname -s)"' | tail -n 1)" = "b=Linux"
t "ssh: systemctl reboot only logs" ssh "$H" 'sudo -n systemctl reboot'
t "the fake is still there after the reboot" test -f "$ENV"

w=$("$fb" writes "$D")
chk "$w" "file /boot/yakogl/standalone/default.bin $(sha "$work/img")" "log: the staged file with its sha256"
lacks "$w" "default.bin.part" "log: the .part name never appears (renamed in the same command)"
chk "$w" "setenv mp2s_left 2" "log: setenv from the -s script"
chk "$w" "setenv boot_prefixes /yakogl/standalone/ /mmc0_" "log: setenv with a value holding a space"
chk "$w" "setenv mp2s_prefixes" "log: a deletion is 'setenv NAME'"
chk "$w" "setenv mp2s_left 3" "log: the CLI form"
chk "$w" "file /boot/yakogl/standalone/default.bin removed" "log: the removal"
chk "$w" "file /boot/Image.copy $(sha "$D/boot/Image.gz")" "log: the copy inside /boot"
chk "$w" "file /boot/Image.copy removed" "log: the copy's removal"
lacks "$w" "Image.tmp" "log: a command's net effect only (made and removed in one command)"
t "ssh: comeback's findmnt through the shim" test "$(ssh "$H" 'findmnt -n -v -o SOURCE,FSTYPE -T /boot')" = "/dev/mmcblk1p8 ext4"
chk "$w" "reboot" "log: the reboot"
lacks "$w" "VIOLATION" "log: no violation so far"
first_file=$(grep -n -m1 '^file ' <<< "$w" | cut -d: -f1)
first_set=$(grep -n -m1 '^setenv ' <<< "$w" | cut -d: -f1)
t "log order: the file write comes before the first setenv" test "${first_file:-0}" -lt "${first_set:-0}"

# ---- the refusals
env_before=$(sha "$ENV")
for v in bootcmd bootcount altbootcmd bootlimit upgrade_available boot_targets bootcmd_stm32mp mp2z_token yako_once; do
    was=$(fw_printenv -n "$v" 2>/dev/null)
    refused "fw_setenv $v is refused" fw_setenv "$v" evil
    t "$v unchanged" test "$(fw_printenv -n "$v" 2>/dev/null)" = "$was"
done
refused "fw_setenv -s with one protected name writes nothing" bash -c 'printf "mp2s_left=5\nbootcount=9\n" | fw_setenv -s -'
refused "fw_setenv -s with a protected deletion is refused" bash -c 'printf "altbootcmd\n" | fw_setenv -s -'
refused "ssh: sudo fw_setenv of bootcmd is refused" ssh "$H" 'sudo -n fw_setenv bootcmd "run evil"'
t "the environment is unchanged after the refusals" test "$(sha "$ENV")" = "$env_before"
t "both copies still valid after the refusals" envpy check "$ENV"
refused "ssh: a relative write outside /boot" ssh "$H" 'echo x > stray.txt'
t "the relative write did not run" test ! -e "$D/stray.txt"
refused "ssh: a write to a host path" ssh "$H" "echo x > $work/host.txt"
t "the host write did not run" test ! -e "$work/host.txt"
refused "ssh: a write nested in sh -c" ssh "$H" "sudo -n sh -c 'echo x > $work/nested.txt'"
t "the nested write did not run" test ! -e "$work/nested.txt"
echo keep > "$work/keep.txt"
refused "ssh: tee to a host path" bash -c "ssh '$H' 'sudo -n tee -- $work/tee.txt >/dev/null' </dev/null"
t "tee did not run" test ! -e "$work/tee.txt"
refused "ssh: rm of a host path" ssh "$H" "sudo -n rm -rf -- $work/keep.txt"
t "rm did not run" test -e "$work/keep.txt"
refused "ssh: mv out of /boot" ssh "$H" "sudo -n mv -- /boot/Image.gz $work/Image.gz"
t "mv did not run" test -e "$D/boot/Image.gz"
refused "ssh: cp into the eMMC's bootfs path" ssh "$H" 'sudo -n cp -- /boot/Image.gz /dev/mmcblk0p8'
refused "ssh: dd of= the env partition" ssh "$H" 'dd if=/dev/zero of=/dev/mmcblk1p7 bs=1 count=4 conv=notrunc'
refused "ssh: a write to /etc" ssh "$H" 'sudo -n sh -c "echo x > /etc/fw_env.config"'
t "the fake's fw_env.config is unchanged" bash -c "grep -q '^/dev/mmcblk1p7 0x7e000 0x2000' '$D/etc/fw_env.config'"
refused "ssh: mkdir outside /boot" ssh "$H" 'sudo -n mkdir -p -- /opt/x'
refused "ssh: find -delete under /" ssh "$H" 'find /dev -name "*p7" -delete'
refused "ssh: find -exec rm of a host path" ssh "$H" "find $work -name keep.txt -exec rm -f -- {} +"
t "ssh: a read-only find -exec sha256sum of /boot runs" bash -c "ssh '$H' 'cd /boot && find . -type f -exec sudo -n sha256sum -- {} + | LC_ALL=C sort -k 2' | grep -q '  ./boot.scr.uimg\$'"
t "ssh: cd /dev && head -- ./<partition> reads the fake's partition" test "$(ssh "$H" 'cd /dev && sudo -n head -c 1048576 -- ./mmcblk1p7' | wc -c)" = 524288
refused "ssh: a write verb fed by xargs" ssh "$H" 'cat /boot/x | xargs rm -f'
refused "ssh: mount" ssh "$H" 'sudo -n mount /dev/mmcblk1p9 /mnt'
refused "ssh: a command that does not parse" ssh "$H" "echo 'unterminated"
refused "ssh: a write inside DIR but outside boot, seen after the run" ssh "$H" "python3 -c \"open('$D/sys/x', 'w').write('x')\""
refused "ssh: the env partition written by something but fw_setenv" ssh "$H" "python3 -c \"open('$D/dev/mmcblk1p7', 'r+b').write(b'x')\""
t "both copies valid after the refused raw writes" envpy check "$ENV"
rm -f "$D/sys/x"
w=$("$fb" writes "$D")
for v in bootcmd bootcount altbootcmd bootlimit boot_targets mp2z_token; do
    chk "$w" "VIOLATION setenv $v evil" "VIOLATION logged: $v"
done
chk "$w" "VIOLATION setenv bootcount 9" "VIOLATION logged: the -s script's bootcount"
lacks "$w" "setenv mp2s_left 5" "the refused -s script's other line was not written"
chk "$w" "VIOLATION setenv altbootcmd" "VIOLATION logged: the protected deletion"
chk "$w" "VIOLATION setenv bootcmd run evil" "VIOLATION logged: bootcmd over ssh"
chk "$w" "VIOLATION file /stray.txt outside /boot (not run)" "VIOLATION logged: the relative write"
chk "$w" "VIOLATION file $work/host.txt outside /boot (not run)" "VIOLATION logged: the host write"
chk "$w" "VIOLATION file $work/nested.txt outside /boot (not run)" "VIOLATION logged: the nested write"
chk "$w" "VIOLATION file $work/tee.txt outside /boot (not run)" "VIOLATION logged: tee"
chk "$w" "VIOLATION file $work/keep.txt outside /boot (not run)" "VIOLATION logged: rm"
chk "$w" "VIOLATION file $work/Image.gz outside /boot (not run)" "VIOLATION logged: mv"
chk "$w" "VIOLATION file /dev/mmcblk0p8 outside /boot (not run)" "VIOLATION logged: cp to the eMMC"
chk "$w" "VIOLATION file /dev/mmcblk1p7 outside /boot (not run)" "VIOLATION logged: dd of= the env"
chk "$w" "VIOLATION file /etc/fw_env.config outside /boot (not run)" "VIOLATION logged: /etc"
chk "$w" "VIOLATION file /opt/x outside /boot (not run)" "VIOLATION logged: mkdir"
chk "$w" "VIOLATION file /dev outside /boot (not run)" "VIOLATION logged: find -delete"
chk "$w" "VIOLATION file $work outside /boot (not run)" "VIOLATION logged: find -exec rm"
chk "$w" "VIOLATION ssh xargs rm: a write verb fed by xargs (not run)" "VIOLATION logged: xargs"
chk "$w" "VIOLATION ssh mount: refused on the fake board (not run)" "VIOLATION logged: mount"
chk "$w" "VIOLATION ssh command does not parse (not run)" "VIOLATION logged: no parse"
chk "$w" "VIOLATION file /sys/x outside /boot" "VIOLATION logged: the write seen after the run"
chk "$w" "VIOLATION env u-boot-env written outside fw_setenv" "VIOLATION logged: the raw env write"
t "writes refuses a directory without a log" bash -c "! '$fb' writes '$work'"
t "shell refuses a directory without the shims" bash -c "! '$fb' shell '$work'"

# ---- the library form
t "sourced, the library defines its functions and runs nothing" bash -c ". '$fb' && declare -F mp2s_fakeboard_init >/dev/null && declare -F mp2s_fakeboard_shell >/dev/null && mp2s_fakeboard_writes '$D' | grep -q VIOLATION"
t "sourced init makes a second fake" bash -c ". '$fb' && mp2s_fakeboard_init '$work/dk2' >/dev/null && test -s '$work/dk2/dev/mmcblk1p7'"

# ---- the text fallback: no uboot_env.py in the tools directory
mkdir -p "$work/notools"
(
    export MP2S_FAKEBOARD_TOOLS=$work/notools MP2S_FAKEBOARD_REPO=$repo
    "$fb" init "$work/dk3" >/dev/null || { echo "FAIL fallback init"; exit 1; }
    eval "$("$fb" shell "$work/dk3")"
    [ -s "$work/dk3/env/u-boot-env.txt" ] && [ -s "$work/dk3/env/NOTE.txt" ] && [ ! -e "$work/dk3/dev/mmcblk1p7" ] || { echo "FAIL fallback layout"; exit 1; }
    [ "$(fw_printenv -n boot_prefixes)" = /mmc0_ ] || { echo "FAIL fallback printenv"; exit 1; }
    [ "$(fw_printenv -n bootcmd)" = "run bootcmd_stm32mp" ] || { echo "FAIL fallback bootcmd"; exit 1; }
    [ "$(fw_printenv | grep -c '^boot_net_usb_start=')" = 1 ] || { echo "FAIL fallback: one line per name"; exit 1; }
    fw_setenv mp2s_left 2 && [ "$(fw_printenv -n mp2s_left)" = 2 ] || { echo "FAIL fallback setenv"; exit 1; }
    printf 'mp2s_left=\nmp2s_ver=1\n' | fw_setenv -s - && ! fw_printenv -n mp2s_left && [ "$(fw_printenv -n mp2s_ver)" = 1 ] || { echo "FAIL fallback -s"; exit 1; }
    fw_setenv bootcount 1 2>/dev/null && { echo "FAIL fallback: bootcount not refused"; exit 1; }
    grep -q '^VIOLATION setenv bootcount 1$' "$work/dk3/writes.log" || { echo "FAIL fallback: VIOLATION not logged"; exit 1; }
    exit 0
) >"$work/fallback.out" 2>&1
if [ $? = 0 ]; then ok "the text fallback without uboot_env.py"; else bad "the text fallback without uboot_env.py" "$(cat "$work/fallback.out")"; fi

echo "mp2s_fakeboard selftest: $npass ok, $nfail failed"
[ "$nfail" = 0 ]
