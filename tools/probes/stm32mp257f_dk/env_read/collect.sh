#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# collect.sh OUTDIR: read the STM32MP257F-DK's U-Boot environment and /boot
# over ssh, into OUTDIR. READ ONLY: every command sent to the board is a
# read (fw_printenv, findmnt, lsblk, cat /proc/partitions, a dd READ of the
# u-boot-env partition into a host file, ls, find, sha256sum, df). This
# script never runs fw_setenv, never writes with dd, never mounts or
# remounts anything and never touches modules, tailscale or sshd. The
# selftest greps this file for them.
#
# Environment (the same as tools/board-stm32mp2.sh; there is no default):
#   BOARD_HOST       user@host of the board's Linux
#   MP2_SUDO         prefix for root on the board ("" for root, "sudo -n ")
#   MP2_KNOWN_HOSTS  a known_hosts file of its own (the host key is checked
#                    and never recorded)
# Files written into OUTDIR (read by summarize.py):
#   fw_printenv.txt  the whole environment as fw_printenv prints it
#   boot-mount.txt   /boot's source device and file system (findmnt)
#   partitions.txt   lsblk of the card /boot is on, then /proc/partitions
#   u-boot-env.bin   the whole u-boot-env partition (dd if=, bs=64k)
#   u-boot-env.dev   its device path
#   boot-ls.txt      ls -lR /boot
#   boot-sizes.txt   "size ./name" for every regular file under /boot
#   boot-sha256.txt  sha256sum of the same files, paths from /boot
#   boot-df.txt      df -P /boot
# Exit: 0 all read; 2 usage or a refusal before any read; 3 a read failed.
set -u
TAG=env_read
die() { echo "$TAG: $1" >&2; exit "${2:-3}"; }
[ $# = 1 ] && [ -n "$1" ] || die "usage: collect.sh OUTDIR" 2
OUT=$1
[ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=user@host of the board's Linux (it comes from the lease)" 2
SU=${MP2_SUDO-}
SSH_OPTS=(-o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=10 -o BatchMode=yes)
[ -z "${MP2_KNOWN_HOSTS:-}" ] || SSH_OPTS+=(-o "UserKnownHostsFile=$MP2_KNOWN_HOSTS" -o GlobalKnownHostsFile=/dev/null)
PART_RE='^/dev/mmcblk[0-9]+p[0-9]+$'
mkdir -p "$OUT" || die "cannot create $OUT" 2

# rd SECONDS FILE REMOTE-COMMAND: run it on the board, stdout into FILE
rd() {
    local t=$1 f=$2
    shift 2
    timeout "$t" ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "$@" > "$OUT/$f" || die "the read for $f failed: $*"
}

rd 60 boot-mount.txt 'findmnt -n -v -o SOURCE,FSTYPE -T /boot'
read -r BOOT_DEV FSTYPE < "$OUT/boot-mount.txt"
[[ ${BOOT_DEV:-} =~ $PART_RE ]] || die "/boot's source '${BOOT_DEV:-}' is not a partition of an mmcblk device"
DISK=${BOOT_DEV%p*}
rd 60 partitions.txt "lsblk -o NAME,SIZE,TYPE,PARTLABEL,MOUNTPOINT $DISK; echo; cat /proc/partitions"
rd 60 .lsblk-labels "lsblk -rno PATH,PARTLABEL $DISK"
ENV_DEV=$(awk '$2 == "u-boot-env" { print $1 }' "$OUT/.lsblk-labels" | head -1)
rm -f "$OUT/.lsblk-labels"
[[ $ENV_DEV =~ $PART_RE ]] || die "no u-boot-env partition on $DISK (the card /boot is on)"
{ [ "${ENV_DEV%p*}" = "$DISK" ] && [ "$ENV_DEV" != "$BOOT_DEV" ]; } || die "$ENV_DEV is not a separate partition of $DISK"
echo "$ENV_DEV" > "$OUT/u-boot-env.dev"
rd 60 fw_printenv.txt "${SU}fw_printenv"
rd 120 u-boot-env.bin "${SU}dd if=$ENV_DEV bs=64k status=none"
rd 60 boot-ls.txt 'ls -lR /boot'
rd 60 boot-sizes.txt "cd /boot && find . -type f -printf '%s %p\\n' | sort -k2"
rd 300 boot-sha256.txt "cd /boot && find . -type f -print0 | sort -z | ${SU}xargs -0 sha256sum"
rd 60 boot-df.txt 'df -P /boot'
echo "$TAG: read into $OUT (fstype ${FSTYPE:-?})"
