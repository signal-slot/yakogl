#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_bootfs_check_selftest.sh - tools/mp2s_bootfs_check.py without a board
# (plan #362, item bootfs-check).
#
#   tools/mp2s_bootfs_check_selftest.sh
#
# Synthetic findmnt/lsblk/blkid/sysfs captures of a DK with a microSD
# (mmcblk1) and an eMMC (mmcblk2) that carry the same GPT labels. The good
# case is accepted; every refusal is one "refused:" line on stderr, exit 2,
# and nothing on stdout.
#
# A tools/check.sh row for this selftest waits for plan #341, so it is run
# by hand: tools/mp2s_bootfs_check_selftest.sh
set -euo pipefail
here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
T=$(mktemp -d "${TMPDIR:-$here}/mp2s-bootfs-selftest.XXXXXX")
trap 'rm -rf "$T"' EXIT
pass=0
fail=0

LABELS="fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b u-boot-env bootfs vendorfs rootfs userfs"

# lsblk_disk DISK [LABEL-TO-LEAVE-OUT]: bootfs is partition 8, rootfs 10
lsblk_disk() {
	local d=$1 skip=${2:-} n=1 l fs
	printf 'NAME="%s" PKNAME="" TYPE="disk" FSTYPE="" LABEL="" PARTLABEL=""\n' "$d"
	for l in $LABELS; do
		if [ "$l" != "$skip" ]; then
			fs=""
			case $l in bootfs | vendorfs | rootfs | userfs) fs=ext4 ;; esac
			printf 'NAME="%sp%s" PKNAME="%s" TYPE="part" FSTYPE="%s" LABEL="" PARTLABEL="%s"\n' \
				"$d" "$n" "$d" "$fs" "$l"
		fi
		n=$((n + 1))
	done
}

blkid_disk() { printf '/dev/%sp8: LABEL="bootfs" UUID="1" TYPE="ext4" PARTLABEL="bootfs"\n' "$1"; }

good() {
	echo /dev/mmcblk1p8 >"$T/findmnt"
	{ lsblk_disk mmcblk1; lsblk_disk mmcblk2; } >"$T/lsblk"
	{ blkid_disk mmcblk1; blkid_disk mmcblk2; } >"$T/blkid"
	echo SD >"$T/type"
}

run() {
	rc=0
	python3 "$here/mp2s_bootfs_check.py" --findmnt "$T/findmnt" --lsblk "$T/lsblk" \
		--blkid "$T/blkid" --disk-type "$T/type" >"$T/out" 2>"$T/err" || rc=$?
}

accept() {
	good
	run
	if [ "$rc" = 0 ] && grep -q '^ok: /boot is /dev/mmcblk1p8' "$T/out" && [ ! -s "$T/err" ]; then
		pass=$((pass + 1))
	else
		echo "FAIL accepted case (exit $rc)"; cat "$T/err"; fail=$((fail + 1))
	fi
}

# refuse NEEDLE MUTATOR: the mutator edits the good captures
refuse() {
	local needle=$1
	shift
	good
	"$@"
	run
	if [ "$rc" = 2 ] && [ "$(grep -c '^refused: ' "$T/err")" = 1 ] \
		&& grep -qF -- "$needle" "$T/err" && [ ! -s "$T/out" ]; then
		pass=$((pass + 1))
	else
		echo "FAIL want refusal '$needle' (exit $rc)"; cat "$T/err"; fail=$((fail + 1))
	fi
}

m_emmc_src() { echo /dev/mmcblk2p8 >"$T/findmnt"; echo MMC >"$T/type"; }
m_mmcblk0() {
	echo /dev/mmcblk0p8 >"$T/findmnt"
	lsblk_disk mmcblk0 >"$T/lsblk"
	blkid_disk mmcblk0 >"$T/blkid"
}
m_type_mmc() { echo MMC >"$T/type"; }
m_rootfs() {
	echo /dev/mmcblk1p10 >"$T/findmnt"
	echo '/dev/mmcblk1p10: LABEL="rootfs" TYPE="ext4"' >"$T/blkid"
}
m_two() { printf '/dev/mmcblk1p8\n/dev/mmcblk2p8\n' >"$T/findmnt"; }
m_nodev() { echo tmpfs >"$T/findmnt"; }
m_whole() { echo /dev/mmcblk1 >"$T/findmnt"; }
m_unknown() { echo /dev/sda1 >"$T/findmnt"; }
m_blkid_label() { sed -i 's|^/dev/mmcblk1p8: LABEL="bootfs"|/dev/mmcblk1p8: LABEL="other"|' "$T/blkid"; }
m_no_blkid() { blkid_disk mmcblk2 >"$T/blkid"; }
m_fat() { sed -i '/"mmcblk1p8"/s/FSTYPE="ext4"/FSTYPE="vfat"/' "$T/lsblk"; }
m_nolayout() { { lsblk_disk mmcblk1 u-boot-env; lsblk_disk mmcblk2; } >"$T/lsblk"; }
m_nopartlabel() { sed -i '/"mmcblk1p8"/s/PARTLABEL="bootfs"/PARTLABEL=""/' "$T/lsblk"; }

accept
refuse "not 'SD'" m_emmc_src
refuse "'MMC'" m_type_mmc
refuse "mmcblk0" m_mmcblk0
refuse "not 'bootfs'" m_rootfs
refuse "exactly one" m_two
refuse "not a /dev partition" m_nodev
refuse "not a partition" m_whole
refuse "not in the lsblk" m_unknown
refuse "blkid LABEL" m_blkid_label
refuse "not in the blkid" m_no_blkid
refuse "not ext4" m_fat
refuse "lacks ST's partitions: u-boot-env" m_nolayout
refuse "not 'bootfs'" m_nopartlabel

rc=0
python3 "$here/mp2s_bootfs_check.py" --findmnt "$T/none" --lsblk "$T/none" --blkid "$T/none" \
	--disk-type "$T/none" >"$T/out" 2>"$T/err" || rc=$?
if [ "$rc" = 2 ] && grep -q '^refused: cannot read' "$T/err" && [ ! -s "$T/out" ]; then
	pass=$((pass + 1))
else
	echo "FAIL unreadable input"; fail=$((fail + 1))
fi
rc=0
python3 "$here/mp2s_bootfs_check.py" >"$T/out" 2>"$T/err" || rc=$?
if [ "$rc" = 2 ] && [ ! -s "$T/out" ]; then
	pass=$((pass + 1))
else
	echo "FAIL usage"; fail=$((fail + 1))
fi

echo "$pass ok, $fail failed"
[ "$fail" = 0 ] && [ "$pass" = 16 ]
