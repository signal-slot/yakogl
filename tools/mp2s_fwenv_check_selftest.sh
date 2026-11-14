#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_fwenv_check_selftest.sh - tools/mp2s_fwenv_check.py on fake roots
# (plan #362, item fwenv-check). No board; fake roots under $TMPDIR.
#
#   tools/mp2s_fwenv_check_selftest.sh
#
# The fake DK: SD card mmcblk1 (type SD; u-boot-env p3 of 512 KiB, bootfs p4,
# rootfs p5) and eMMC mmcblk0 (type MMC; the same labels), /boot mounted from
# mmcblk1p4. Cases: good SD config (by path, by by-partlabel, with comments
# and optional fields); by-partlabel resolving to the eMMC; the eMMC by path;
# one copy on each; an eMMC claiming SD (mmcblk0 refused); overlapping copies;
# offsets past the partition end; wrong env size; a partition that is not
# u-boot-env; a whole disk; /boot on the eMMC; nothing on /boot; a missing
# config; a single-copy config.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
T=${TMPDIR:-/tmp}/mp2s_fwenv.$$
trap 'rm -rf "$T"' EXIT
fail=0

mkdisk() { # root disk type
    mkdir -p "$1/sys/block/$2/device"
    echo "$3" > "$1/sys/block/$2/device/type"
}
mkpart() { # root disk part number name sectors
    local dir=$1/sys/block/$2/$3
    mkdir -p "$dir" "$1/sys/class/block"
    echo "$4" > "$dir/partition"
    echo "$6" > "$dir/size"
    printf 'MAJOR=179\nPARTNAME=%s\n' "$5" > "$dir/uevent"
    ln -s "../../block/$2/$3" "$1/sys/class/block/$3"
}
mkroot() {
    local r=$1
    mkdisk "$r" mmcblk1 SD
    mkdisk "$r" mmcblk0 MMC
    mkpart "$r" mmcblk1 mmcblk1p3 3 u-boot-env 1024
    mkpart "$r" mmcblk1 mmcblk1p4 4 bootfs 65536
    mkpart "$r" mmcblk1 mmcblk1p5 5 rootfs 65536
    mkpart "$r" mmcblk0 mmcblk0p3 3 u-boot-env 1024
    mkpart "$r" mmcblk0 mmcblk0p4 4 bootfs 65536
    mkdir -p "$r/dev/disk/by-partlabel" "$r/etc" "$r/proc/self"
    ln -s ../../mmcblk1p3 "$r/dev/disk/by-partlabel/u-boot-env"
    echo "28 1 179:4 / /boot rw,relatime shared:3 - vfat /dev/mmcblk1p4 rw" \
        > "$r/proc/self/mountinfo"
    printf '/dev/mmcblk1p3 0x7e000 0x2000\n/dev/mmcblk1p3 0x7c000 0x2000\n' \
        > "$r/etc/fw_env.config"
}
# case_ name want_exit want_substring setup-function
case_() {
    local r=$T/$1
    mkroot "$r"
    ( cd "$r" && "$4" ) >/dev/null 2>&1
    local out rc
    out=$(python3 -I "$HERE/mp2s_fwenv_check.py" --root "$r" 2>&1); rc=$?
    if [ "$rc" = "$2" ] && [[ "$out" == *"$3"* ]]; then
        echo "ok   $1: $out"
    else
        echo "FAIL $1: exit $rc (want $2), '$out' (want '*$3*')"
        fail=1
    fi
}
cfg2() { printf '%s %s %s\n%s %s %s\n' "$@" > etc/fw_env.config; }

s_good() { :; }
s_partlabel() { cfg2 /dev/disk/by-partlabel/u-boot-env 0x7e000 0x2000 /dev/disk/by-partlabel/u-boot-env 0x7c000 0x2000; }
s_comments() { printf '# env\n\n/dev/mmcblk1p3 0x7e000 0x2000 0x200 16 # c\n/dev/mmcblk1p3 507904 8192\n' > etc/fw_env.config; }
s_partlabel_emmc() { ln -sf ../../mmcblk0p3 dev/disk/by-partlabel/u-boot-env; s_partlabel; }
s_emmc_direct() { cfg2 /dev/mmcblk0p3 0x7e000 0x2000 /dev/mmcblk0p3 0x7c000 0x2000; }
s_one_each() { cfg2 /dev/mmcblk1p3 0x7e000 0x2000 /dev/mmcblk0p3 0x7c000 0x2000; }
s_emmc_says_sd() { echo SD > sys/block/mmcblk0/device/type; s_emmc_direct; }
s_overlap() { cfg2 /dev/mmcblk1p3 0x7e000 0x2000 /dev/mmcblk1p3 0x7d000 0x2000; }
s_past_end() { cfg2 /dev/mmcblk1p3 0x7e000 0x2000 /dev/mmcblk1p3 0x80000 0x2000; }
s_wrong_size() { cfg2 /dev/mmcblk1p3 0x7e000 0x4000 /dev/mmcblk1p3 0x7a000 0x4000; }
s_not_env() { cfg2 /dev/mmcblk1p5 0 0x2000 /dev/mmcblk1p5 0x2000 0x2000; }
s_whole_disk() { cfg2 /dev/mmcblk1 0 0x2000 /dev/mmcblk1 0x2000 0x2000; }
s_boot_emmc() { sed -i s/mmcblk1p4/mmcblk0p4/ proc/self/mountinfo; }
s_no_boot() { : > proc/self/mountinfo; }
s_no_config() { rm etc/fw_env.config; }
s_single() { printf '/dev/mmcblk1p3 0x7e000 0x2000\n' > etc/fw_env.config; }

case_ good 0 "fwenv: ok SD u-boot-env copies at 0x7e000 and 0x7c000" s_good
case_ good_partlabel 0 "fwenv: ok" s_partlabel
case_ good_comments 0 "fwenv: ok" s_comments
case_ partlabel_emmc 2 "fwenv: REFUSE" s_partlabel_emmc
case_ emmc_direct 2 "type 'MMC'" s_emmc_direct
case_ one_each 2 "type 'MMC'" s_one_each
case_ emmc_says_sd 2 "mmcblk0" s_emmc_says_sd
case_ overlap 2 "overlap" s_overlap
case_ past_end 2 "runs past the end" s_past_end
case_ wrong_size 2 "env size" s_wrong_size
case_ not_env 2 "not u-boot-env" s_not_env
case_ whole_disk 2 "not a partition" s_whole_disk
case_ boot_emmc 2 "not on the disk holding /boot" s_boot_emmc
case_ no_boot 2 "nothing is mounted on /boot" s_no_boot
case_ no_config 2 "no etc/fw_env.config" s_no_config
case_ single_copy 2 "want 2" s_single

[ $fail = 0 ] && echo "mp2s_fwenv_check_selftest: all ok"
exit $fail
