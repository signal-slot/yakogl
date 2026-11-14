#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mkdisk.sh DISK BOOTFS_DIR [env]: a GPT disk image like the DK's microSD, no
# root. Partition 1 "fsbl" is a placeholder for ST's boot partitions, partition
# 2 "bootfs" is ext4 (label bootfs) holding the files of BOOTFS_DIR, flagged
# LegacyBIOSBootable as ST's image flags its bootfs (U-Boot's distro scan
# looks at the bootable partitions). With "env", partition 3 "u-boot-env"
# (512 KiB of zeros) holds the two environment copies of the redund U-Boot
# (mp2-standalone-redund.config). 32 MiB. Needs sfdisk, mke2fs, truncate
# (in /usr/sbin or /sbin too).
set -euo pipefail
export PATH=$PATH:/usr/sbin:/sbin

case "$#:${3:-}" in
2: | 3:env) ;;
*) echo "usage: mkdisk.sh DISK BOOTFS_DIR [env]" >&2; exit 2 ;;
esac
disk=$1 dir=$2
envpart=
if [ $# -eq 3 ]; then envpart='start=61440, size=1024, name=u-boot-env\n'; fi
rm -f "$disk"
truncate -s 32M "$disk"
printf 'label: gpt\nstart=2048, size=2048, name=fsbl\nstart=4096, size=57344, name=bootfs, attrs="LegacyBIOSBootable"\n'"$envpart" |
	sfdisk -q "$disk"
mke2fs -q -F -t ext4 -L bootfs -d "$dir" -E offset=$((4096 * 512)) "$disk" 28672k
