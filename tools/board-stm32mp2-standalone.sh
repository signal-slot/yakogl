#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board-stm32mp2-standalone.sh - the STM32MP257F-DK's standalone boot from its
# own microSD (docs/notes/stm32mp257f-dk-standalone.md, section 7: the contract
# this runner follows; docs/standalone-boot.md for the Verdins' shape).
#
#   tools/board-stm32mp2-standalone.sh [--dry-run] status | install | uninstall
#   tools/board-stm32mp2-standalone.sh [--dry-run] default <image> <1-9|forever>
#   tools/board-stm32mp2-standalone.sh [--dry-run] nodefault
#   tools/board-stm32mp2-standalone.sh guard path <path> | guard name <name>
#   tools/board-stm32mp2-standalone.sh selftest   (tools/board_stm32mp2_standalone_selftest.sh)
#
# This file is the safety core: the verbs, --dry-run, the person gate for
# "forever", the write guard (guard_path, dev_guard), the undo printed before
# anything changes, and the name checks. The payload (the staged U-Boot script
# and the environment write of section 3) is not wired yet: PAYLOAD_READY=0
# makes every verb that would write the board refuse (rc 3) after its checks
# and its undo lines, and --dry-run shows what it would send. The verbs are
# the shape of tools/board.sh's Verdin verbs: "default" prints its exact undo
# first and reboots nothing, "forever" needs BOARD_PERSON_PRESENT=1, a count
# (1-9) brings the board back to Linux by itself, "nodefault" undoes any
# default from Linux.
#
# Everything sent to the board is built from fixed strings and from names that
# passed name_ok (^[A-Za-z0-9][A-Za-z0-9._-]*$): the remote command line is a
# root shell's, so a name is never trusted, and every path in it goes through
# printf %q as well. "nodefault" and "uninstall" never take a name from a
# listing: the staged image's name is the fixed default.bin (the contract has
# no file-name variable in the environment).
#
# What may be written: files under /boot/yakogl/standalone on the card's bootfs
# (guard_path) and, once the payload is wired, the u-boot-env partition of the
# SD card /boot is on (dev_guard). Never the eMMC (mmcblk0), never fsbla1,
# fsbla2, metadata1, metadata2, fip-a, fip-b, vendorfs, rootfs or userfs.
#
# Environment: BOARD_HOST (user@host of the board's Linux), BOARD_PERSON_PRESENT
# (1: a person is at the board), MP2_NO_WATCHDOG (1: accept an image without
# the watchdog marker), MP2_SUDO (prefix for root on the board, default
# "sudo -n "), MP2_KNOWN_HOSTS (known_hosts file for the board's host key).
# No address, host or path of a board is kept in this file.
#
# Exit codes: 0 done; 2 usage; 3 refused before anything changed.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
SELF="$HERE/board-stm32mp2-standalone.sh"
TAG="board-stm32mp2-standalone.sh stm32mp257f-dk"

PAYLOAD_READY=0
DRY=0
args=()
for a in "$@"; do
    case "$a" in
    --dry-run) DRY=1 ;;
    *) args+=("$a") ;;
    esac
done
set -- "${args[@]+"${args[@]}"}"
usage() { sed -n '9,15p' "$SELF" >&2; exit 2; }
[ $# -ge 1 ] || usage
VERB=$1
shift

STAGE_DIR=/boot/yakogl/standalone
STAGED=default.bin                 # the one name the script loads: fixed
NAME_RE='^[A-Za-z0-9][A-Za-z0-9._-]*$'
LOAD_MAX=$((256 * 1024 * 1024))    # the 256 MiB window at 0xa0000000
MARKER='gc_wdog_arm: armed'
NEVER_LABELS="fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b vendorfs rootfs userfs"
EMMC_RE='^/dev/mmcblk0(p[0-9]+)?$'
PART_RE='^/dev/mmcblk[0-9]+p[0-9]+$'
SU=${MP2_SUDO-sudo -n }

SSH_OPTS=(-o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=10 -o BatchMode=yes)
[ -z "${MP2_KNOWN_HOSTS:-}" ] || SSH_OPTS+=(-o "UserKnownHostsFile=$MP2_KNOWN_HOSTS" -o GlobalKnownHostsFile=/dev/null)

note() { echo "$TAG: $*"; }
die() { echo "$TAG: $1" >&2; exit "${2:-3}"; }

# ---- the write guard ---------------------------------------------------------------
# name_ok NAME: one plain file name: no slash, space, newline, quote or shell
# or U-Boot metacharacter. (The regex's $ would let a trailing newline
# through, so the newline is refused on its own.)
name_ok() { [[ $1 =~ $NAME_RE ]] && [[ $1 != *$'\n'* ]] && [ "$1" != . ] && [ "$1" != .. ]; }
# guard_path P: P is STAGE_DIR or STAGE_DIR/<name>, <name> passing name_ok;
# every remote rm, mv and tee goes through it first
guard_path() {
    local p=$1 n
    [ "$p" = "$STAGE_DIR" ] && return 0
    [[ $p == "$STAGE_DIR"/* ]] || die "REFUSED: path is not $STAGE_DIR or a file in it"
    n=${p#"$STAGE_DIR"/}
    name_ok "$n" || die "REFUSED: not a plain file name ([A-Za-z0-9][A-Za-z0-9._-]*) in $STAGE_DIR"
}
# dev_guard DEV LABEL TABLE CARD: DEV may be written only if it is not the
# eMMC, is a partition of CARD, is not a never-written label, and TABLE
# ("PATH PARTLABEL" lines) says it carries LABEL. Pure.
dev_guard() {
    local dev=$1 want=$2 table=$3 card=$4 label l
    [[ $dev =~ $EMMC_RE ]] && die "REFUSED: $dev is the eMMC (mmcblk0): never written"
    [[ $dev =~ $PART_RE ]] || die "REFUSED: $dev is not a partition of an mmcblk device"
    { [ -n "$card" ] && [ "${dev%p*}" = "$card" ]; } || die "REFUSED: $dev is not a partition of the SD card /boot is on (${card:-none found})"
    label=$(awk -v d="$dev" '$1 == d { print $2 }' <<<"$table" | head -1)
    for l in $NEVER_LABELS; do
        [ "$label" != "$l" ] || die "REFUSED: $dev is labelled $label: never written"
    done
    [ "$label" = "$want" ] || die "REFUSED: $dev is labelled '${label:-nothing}', not $want"
}

# ---- the board -----------------------------------------------------------------------
need_board() {
    if [ "$DRY" = 1 ]; then BOARD_HOST=${BOARD_HOST:-user@BOARD}; return; fi
    [ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=user@host of the board's Linux; it comes from the lease, there is no default" 2
}
# b CMD: run CMD on the board (dry-run: print it). CMD is built here from
# fixed strings and %q-quoted, checked names only.
b() {
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*"; return 0; fi
    timeout 60 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "$@"
}
# board_change CMD: the one place a changing command is sent; refuses until
# the payload is wired
board_change() {
    if [ "$DRY" = 0 ] && [ "$PAYLOAD_READY" != 1 ]; then
        die "the payload (U-Boot script, environment write) is not wired yet: nothing was changed (PAYLOAD_READY=0)"
    fi
    b "$@"
}
# board_rm NAME: remove one staged file; the path is checked and quoted
board_rm() {
    local p="$STAGE_DIR/$1"
    guard_path "$p"
    board_change "${SU}rm -f -- $(printf '%q' "$p")"
}
board_rm_stage() {
    guard_path "$STAGE_DIR"
    board_change "${SU}rm -rf -- $(printf '%q' "$STAGE_DIR")"
}

# ---- the image -------------------------------------------------------------------------
check_image() {
    local img=$1 size magic
    [ -f "$img" ] || die "no such image: $img" 2
    size=$(stat -c %s "$img")
    [ "$size" -gt 64 ] || die "image too small ($size bytes)" 2
    [ "$size" -le "$LOAD_MAX" ] || die "image of $size bytes does not fit the 256 MiB window at 0xa0000000"
    magic=$(dd if="$img" bs=1 skip=56 count=4 status=none | od -An -c | tr -d ' ')
    [ "$magic" = 'ARMd' ] || die "image has no AArch64 Image header (magic at byte 56)"
    if [ "${MP2_NO_WATCHDOG:-0}" != 1 ] && ! grep -aq "$MARKER" "$img"; then
        die "image has no watchdog marker '$MARKER': BL2's IWDG would reset it in about 32 s. MP2_NO_WATCHDOG=1 accepts that on purpose"
    fi
}

# ---- the undo, printed before anything changes ----------------------------------------
undo_default() {
    note "UNDO, from Linux (before the reboot, or once Linux is back): tools/board-stm32mp2-standalone.sh nodefault"
    if [ "$1" = forever ]; then
        note "UNDO, without Linux (a person at the U-Boot prompt): setenv mp2s_left 0; saveenv; boot"
        note "UNDO, with no console (a person): the card in a PC, delete yakogl/standalone/boot.scr.uimg from bootfs"
    else
        note "UNDO, without Linux: none needed: after $1 boots the script restores boot_prefixes and boots Linux"
    fi
}

# ---- verbs ---------------------------------------------------------------------------------
cmd_status() {
    [ $# -eq 0 ] || usage
    need_board
    b "ls -l -- $(printf '%q' "$STAGE_DIR")"
    b "${SU}fw_printenv mp2s_ver mp2s_left mp2s_prefixes boot_prefixes"
}
cmd_install() {
    [ $# -eq 0 ] || usage
    need_board
    note "UNDO: tools/board-stm32mp2-standalone.sh uninstall"
    guard_path "$STAGE_DIR/boot.scr.uimg"
    board_change "${SU}mkdir -p -- $(printf '%q' "$STAGE_DIR")"
}
cmd_uninstall() {
    [ $# -eq 0 ] || usage
    need_board
    note "UNDO: tools/board-stm32mp2-standalone.sh install"
    b "${SU}fw_printenv -n mp2s_left   # a default set refuses: 'nodefault' first"
    board_rm_stage
}
cmd_default() {
    [ $# -eq 2 ] || usage
    local img=$1 n=$2 p
    case "$n" in
    [1-9] | forever) ;;
    *) die "boots must be 1-9 or forever, not '$n'" 2 ;;
    esac
    if [ "$n" = forever ] && [ "${BOARD_PERSON_PRESENT:-0}" != 1 ]; then
        die "'forever' makes the image every boot's: Linux comes back only through a person (docs/standalone-boot.md). Set BOARD_PERSON_PRESENT=1 when one is at the board, or give a boot count (1-9): the board then returns to Linux by itself." 2
    fi
    check_image "$img"
    need_board
    undo_default "$n"
    b "${SU}fw_printenv mp2s_left   # a default already set refuses"
    p="$STAGE_DIR/$STAGED"
    guard_path "$p.part"
    guard_path "$p"
    board_change "${SU}tee -- $(printf '%q' "$p.part") >/dev/null && ${SU}mv -- $(printf '%q' "$p.part") $(printf '%q' "$p")   # < image"
    board_change "${SU}fw_setenv -s -   # mp2s_ver=1 mp2s_left=$n mp2s_prefixes boot_prefixes: one write"
    note "default set for $n boots; nothing was rebooted."
}
cmd_nodefault() {
    [ $# -eq 0 ] || usage
    need_board
    note "nothing to undo: nodefault only removes"
    board_change "${SU}fw_setenv -s -   # boot_prefixes back to mp2s_prefixes, mp2s_* deleted: one write"
    board_rm "$STAGED"
    note "no default: ST's extlinux boots Linux again"
}
cmd_guard() {
    case "${1:-}" in
    path) [ $# -eq 2 ] || usage; guard_path "$2"; echo ok ;;
    name) [ $# -eq 2 ] || usage; name_ok "$2" || die "REFUSED: bad name"; echo ok ;;
    dev) [ $# -eq 3 ] || usage; dev_guard "$2" "$3" "${GUARD_TABLE:-}" "${GUARD_CARD:-}"; echo ok ;;
    *) usage ;;
    esac
}

case "$VERB" in
status) cmd_status "$@" ;;
install) cmd_install "$@" ;;
uninstall) cmd_uninstall "$@" ;;
default) cmd_default "$@" ;;
nodefault) cmd_nodefault "$@" ;;
guard) cmd_guard "$@" ;;
selftest) exec "$HERE/board_stm32mp2_standalone_selftest.sh" ;;
*) usage ;;
esac
