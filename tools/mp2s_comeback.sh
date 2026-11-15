#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# mp2s_comeback.sh: the read-only "OpenSTLinux is back" verdict for the
# STM32MP257F-DK after a standalone run (plan #362; the comeback terms of
# docs/notes/stm32mp257f-dk-standalone.md section 6: ssh, mounts, no mp2s_*).
# Host side only; the board sees one ssh session that reads and writes
# nothing. No file of this tool names a machine or an address.
#
#   tools/mp2s_comeback.sh [--wait SECONDS] [--before-boot-id ID]
#
# The ssh target comes only from the environment: BOARD_HOST first, the
# variable tools/board-stm32mp2.sh and tools/board-stm32mp2-standalone.sh
# use (user@host of the board's Linux, from the lease; there is no default
# and none is held here), else BOARD_SSH. The ssh options match the two
# runners': the host key is checked and never recorded, and MP2_KNOWN_HOSTS
# names a known_hosts file of its own (read only). MP2_SUDO prefixes the
# board's fw_printenv as the standalone runner does (default "sudo -n ").
#
# What it reads (one ssh session of cat, uname, findmnt and fw_printenv —
# no service call, no module, no write, no fw_setenv, and never a question
# about sshd or tailscale: the session's own existence is the evidence, and
# the note's section 8 never touches them):
#   boot id, uptime   --before-boot-id compares the id against the one read
#                       before the reboot; equal ids mean no reboot happened.
#   /boot, /          mounted; /boot ext4 on a partition of a disk whose
#                       /sys/block/<disk>/device/type reads SD — the MMC
#                       word is the eMMC, which this design never uses
#                       (section 1; the read of board-stm32mp2.sh's
#                       dev_guard rule).
#   fw_printenv       no mp2s_* left and no /yakogl/standalone/ in
#                       boot_prefixes (the counted default's used-up branch
#                       deletes both). A failing fw_printenv is a FAIL,
#                       never silence: the variables are not taken as unset
#                       (tools/board-stm32mp2.sh's rule).
#
# Verdict lines start "comeback:" and are redacted: the ssh target, every
# part of it four characters or longer, and any IPv4 or IPv6 the board
# printed are masked. A board that does not answer ssh is NEEDS A PERSON:
# nothing here reboots, resets or retries, and the USB-C supply is the
# first thing a person checks (stm32mp257f-dk-bringup.md, "supply first").
# Exit: 0 ok, 1 FAIL (the reasons are named), 2 usage or no target,
# 4 no ssh within --wait (whole seconds 0..900; 0 means one try).
# Selftest without a board: tools/mp2s_comeback_selftest.sh.
set -u

usage() {
    echo "usage: mp2s_comeback.sh [--wait SECONDS] [--before-boot-id ID]" >&2
    echo "read-only comeback check over the board's ssh; the target comes from BOARD_HOST (else BOARD_SSH)" >&2
    echo "exit: 0 ok, 1 FAIL, 2 usage or no target, 4 no ssh within --wait (0..900 s)" >&2
}

WAIT=0
BEFORE=
while [ $# -gt 0 ]; do
    case $1 in
    --wait) WAIT=${2:-}; shift 2 || true ;;
    --before-boot-id) BEFORE=${2:-}; shift 2 || true ;;
    -h|--help) usage; exit 2 ;;
    *) usage; exit 2 ;;
    esac
done
case ${WAIT} in ''|*[!0-9]*) echo "comeback: --wait takes whole seconds" >&2; exit 2 ;; esac
WAIT=$((10#$WAIT))
[ "$WAIT" -le 900 ] || { echo "comeback: --wait is capped at 900 s" >&2; exit 2; }
TARGET=${BOARD_HOST:-${BOARD_SSH:-}}
if [ -z "$TARGET" ]; then
    echo "comeback: set BOARD_HOST=user@host of the board's Linux (BOARD_SSH is read too); it comes from the lease, there is no default" >&2
    exit 2
fi

SU=${MP2_SUDO-sudo -n }
SSH_OPTS=(-o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=8 -o BatchMode=yes)
[ -z "${MP2_KNOWN_HOSTS:-}" ] || SSH_OPTS+=(-o "UserKnownHostsFile=$MP2_KNOWN_HOSTS" -o GlobalKnownHostsFile=/dev/null)

TMP=$(mktemp -d "${TMPDIR:-/tmp}/mp2s_comeback.XXXXXX") || exit 2
trap 'rm -rf "$TMP"' EXIT

# mask the target and its parts (four characters or longer), then addresses
redact() {
    local e=() s esc
    local host=${TARGET#*@} user=${TARGET%@*}
    for s in "$TARGET" "$host" "$user"; do
        [ ${#s} -ge 4 ] || continue
        esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
        e+=(-e "s/$esc/<board>/g")
    done
    LC_ALL=C sed -E ${e[@]+"${e[@]}"} \
        -e 's/([0-9]{1,3}\.){3}[0-9]{1,3}/<address>/g' \
        -e 's/([0-9A-Fa-f]{0,4}:){3,7}[0-9A-Fa-f]{0,4}/<address>/g'
}
say() { printf '%s\n' "$*" | redact; }

# the reads, run by the board's sh; key=value lines. @SU@ is replaced on the
# host before this is sent, so the sudo prefix travels inside the one command.
REMOTE='
echo "boot_id=$(cat /proc/sys/kernel/random/boot_id 2>/dev/null)"
echo "uptime=$(cut -d. -f1 /proc/uptime 2>/dev/null)"
echo "kernel=$(uname -r)"
m=$(findmnt -n -v -o SOURCE,FSTYPE -T /boot 2>/dev/null)
echo "boot_mount=$m"
d=${m%% *}; d=${d%p*}
echo "boot_type=$(cat /sys/block/${d##*/}/device/type 2>/dev/null)"
echo "root_mount=$(findmnt -n -v -o SOURCE,FSTYPE -T / 2>/dev/null)"
e=$(@SU@fw_printenv 2>/dev/null); erc=$?
echo "env_rc=$erc"
echo "env_mp2s=$(printf %s "$e" | sed -n "s/^\(mp2s_[A-Za-z0-9_]*\)=.*/\1/p" | tr "\n" " ")"
echo "env_pfx=$(printf %s "$e" | sed -n "s/^boot_prefixes=//p" | head -n1)"
echo "end=1"
'
REMOTE=${REMOTE//@SU@/$SU}

try_once() {
    timeout 25 ssh "${SSH_OPTS[@]}" "$TARGET" "$REMOTE" >"$TMP/out" 2>/dev/null </dev/null &&
        grep -q '^end=1$' "$TMP/out"
}

deadline=$((SECONDS + WAIT))
until try_once; do
    if [ "$SECONDS" -ge "$deadline" ]; then
        say "comeback: NEEDS A PERSON: the board did not answer ssh within ${WAIT} s. Nothing here reboots, resets or retries: stop, report it, and a person checks the USB-C supply first (docs/notes/stm32mp257f-dk-bringup.md)"
        exit 4
    fi
    sleep 5
done

g() { sed -n "s/^$1=//p" "$TMP/out" | head -n1; }
boot_id=$(g boot_id)
kernel=$(g kernel)
boot_mount=$(g boot_mount)
boot_src=${boot_mount%% *}
boot_fstype=${boot_mount#* }
boot_type=$(g boot_type)
root_mount=$(g root_mount)
env_rc=$(g env_rc)
env_mp2s=$(g env_mp2s)
env_pfx=$(g env_pfx)

fails=()
if [ -z "$boot_id" ]; then
    fails+=("no boot id")
elif [ -n "$BEFORE" ] && [ "$boot_id" = "$BEFORE" ]; then
    fails+=("same boot id: no reboot happened")
fi
if [ -z "$boot_mount" ]; then
    fails+=("/boot not mounted")
else
    case $boot_src in
    /dev/mmcblk[0-9]*p[0-9]*) : ;;
    *) fails+=("/boot source '$boot_src' is not a partition") ;;
    esac
    [ "$boot_fstype" = ext4 ] || fails+=("/boot is '$boot_fstype', not ext4")
fi
[ "$boot_type" = SD ] ||
    fails+=("the disk /boot is on reads '${boot_type:-unreadable}', not SD (MMC is the eMMC, mmcblk0: the board must come back on the microSD)")
[ -n "$root_mount" ] || fails+=("/ not mounted")
case $env_rc in
0) : ;;
*) fails+=("fw_printenv did not answer (rc ${env_rc:-unreadable}): the variables are not taken as unset") ;;
esac
if [ -n "$env_mp2s" ]; then
    names=$(printf '%s\n' $env_mp2s | sed -n 's/^\(mp2s_[A-Za-z0-9_]*\)$/\1/p' | tr '\n' ' ')
    fails+=("mp2s_* left in the environment: ${names:-unknown}")
fi
case $env_pfx in
*/yakogl/standalone/*) fails+=("boot_prefixes still carries /yakogl/standalone/") ;;
esac

if [ ${#fails[@]} -gt 0 ]; then
    reasons=
    for f in "${fails[@]}"; do
        reasons=${reasons:+$reasons "; "}$f
    done
    say "comeback: FAIL $reasons"
    exit 1
fi
if [ -n "$BEFORE" ]; then
    nb="new boot, the boot id differs from the one before the reboot"
else
    nb="boot id not compared (no --before-boot-id)"
fi
say "comeback: ok OpenSTLinux kernel ${kernel:-unreadable}, /boot on $boot_src ($boot_fstype, disk type SD), / on ${root_mount%% *}, the environment holds no mp2s_* and no /yakogl/standalone/ prefix, $nb"
say "comeback: uptime $(g uptime) s"
exit 0
