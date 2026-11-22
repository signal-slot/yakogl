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
# The safety core: the verbs, --dry-run, the person gate for "forever", the
# write guard (guard_path, dev_guard), the undo printed before anything
# changes, and the name checks. The payload (PAYLOAD_READY=1) is wired on it:
#
#   install    builds boot.scr.uimg with driver/tools/mp2-standalone/build.sh
#              (its sha256 is the last line of build.sh's output) and checks
#              it with tools/mp2s_uimg_check.py; checks both environment
#              copies' CRCs; records the sha256 of every file under /boot
#              (tools/mp2s_boot_manifest.py record --from, over a listing
#              taken by ssh: no tool is copied to the board) BEFORE anything
#              changes; then stages, under /boot/yakogl/standalone only,
#              boot-sha256.txt, u-boot-env.bin (the pristine u-boot-env
#              partition), boot.scr.uimg.sha256 and boot.scr.uimg, each as
#              NAME.part, then renamed, then read back by sha256 after the
#              page cache is dropped. Section 2: the script goes into the
#              stage directory, where the extra boot_prefixes entry finds it;
#              no stock file under /boot is written, renamed or replaced.
#              No environment write.
#   default    tools/mp2s_image_check.py on the image (the product marker
#              'mp2_wdtr: product refresh armed' for forever; for a count the
#              product marker or the board layer's 'gc_wdog_arm: armed';
#              MP2_NO_WATCHDOG=1 waives both), and a refusal of a
#              LIFETIME_S=0 product build under a count (its line 'no
#              lifetime of its own (product build)': it never resets, so the
#              count would never run out). Then, read-only: /boot on the SD
#              card, the staged script equal to this commit's build,
#              tools/mp2s_guard.py --for standalone, tools/mp2s_fwenv_check.py
#              (sent to the board's python3 on stdin), both environment
#              copies' CRCs and fw_printenv equal to the partition read,
#              tools/fwu_metadata.py (the booted bank accepted), the free
#              space. Then tools/mp2s_env_plan.py (its write guard) and
#              tools/mp2s_env_capacity.py compute and size the edit; the
#              staged default.bin; one fw_setenv -s - of mp2s_ver, mp2s_left,
#              mp2s_prefixes and boot_prefixes over the SD's u-boot-env;
#              the read-back (fw_printenv and the partition, through
#              mp2s_env_plan.py verify); on a mismatch the values before it
#              go back at once. Reboots nothing.
#   nodefault  the same reads; mp2s_env_plan.py nodefault against the default
#              record; one fw_setenv that puts boot_prefixes back and deletes
#              mp2s_*, read back; removes default.bin; compares /boot with the
#              install record when the manifest tool is there. Safe to repeat.
#   uninstall  nodefault, then removes the stage directory (and /boot/yakogl
#              when that is empty then), then requires
#              'mp2s-boot: same' between /boot and the install record.
#
# Everything sent to the board is built from fixed strings and from names that
# passed name_ok (^[A-Za-z0-9][A-Za-z0-9._-]*$) or a device regex: the remote
# command line is a root shell's, so a name is never trusted, and every path in
# it goes through printf %q as well. Environment values never go on a command
# line: fw_setenv reads them on stdin. "nodefault" and "uninstall" never take a
# name from a listing: the staged image's name is the fixed default.bin (the
# contract has no file-name variable in the environment).
#
# What may be written: files under /boot/yakogl/standalone on the card's bootfs
# (guard_path) and, through fw_setenv, the variables mp2s_ver, mp2s_left,
# mp2s_prefixes and boot_prefixes of the u-boot-env partition of the SD card
# /boot is on (dev_guard before each write, fwset_guard on every line). Never
# the eMMC (the disk whose type is MMC, whatever its number: on the DK the
# card is mmcblk0 on one boot and mmcblk2 on another), never fsbla1, fsbla2,
# metadata1, metadata2, fip-a, fip-b, vendorfs, rootfs or userfs; the FWU
# metadata partitions are only read. On a board without /etc/fw_env.config
# (the DK as shipped) the runner also writes two lines to
# /run/mp2s-fw_env.config on the board's tmpfs, for fw_printenv and fw_setenv
# -c, and removes the file when the run ends (fw_config).
#
# The host keeps, in MP2S_STATE_DIR (default records/stm32mp257f-dk/standalone,
# gitignored): boot-sha256.txt (the install record), u-boot-env.bin (the
# pristine partition) and default-prefixes.txt (the default record: the live
# boot_prefixes, byte for byte). The card keeps the first two in the stage
# directory, so a nodefault or uninstall from another host still has them.
#
# Helpers are looked up when the runner runs. tools/mp2s_boot_manifest.py
# (MP2S_BOOT_MANIFEST overrides the path) missing: a live install or uninstall
# refuses (rc 3) before anything is sent; --dry-run prints the plan with a
# 'skip:' line; nodefault leaves /boot uncompared and says so.
#
# Environment: BOARD_HOST (user@host of the board's Linux), BOARD_PERSON_PRESENT
# (1: a person is at the board), MP2_NO_WATCHDOG (1: accept an image without
# the watchdog marker), MP2_SUDO (prefix for root on the board, default
# "sudo -n "), MP2_KNOWN_HOSTS (known_hosts file for the board's host key),
# MP2S_STATE_DIR, MP2S_BOOT_MANIFEST, BOARD_LOCK and LOCK_WAIT (the board lock:
# by default the lock board_lock.sh names from BOARD_HOST; 600 s).
# No address, host or path of a board is kept in this file.
#
# Exit codes: 0 done; 2 usage; 3 refused before anything changed; 4 a change
# failed and was undone (the message says what is left); 5 NEEDS A PERSON (a
# read-back that could not be undone, or /boot differs from the install
# record); 129, 130, 143 cut off by HUP, INT, TERM (the message names the undo).
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
SELF="$HERE/board-stm32mp2-standalone.sh"
TAG="board-stm32mp2-standalone.sh stm32mp257f-dk"

PAYLOAD_READY=1
DRY=0
args=()
for a in "$@"; do
    case "$a" in
    --dry-run) DRY=1 ;;
    *) args+=("$a") ;;
    esac
done
set -- "${args[@]+"${args[@]}"}"
usage() { sed -n '9,13p' "$SELF" >&2; exit 2; }
[ $# -ge 1 ] || usage
VERB=$1
shift

STAGE_DIR=/boot/yakogl/standalone
STAGED=default.bin                 # the one name the script loads: fixed
NAME_RE='^[A-Za-z0-9][A-Za-z0-9._-]*$'
MARKER='gc_wdog_arm: armed'                         # the board layer's watchdog
PRODUCT_MARKER='mp2_wdtr: product refresh armed'    # mp2_product's refresh (section 6)
NOLIFE_MARKER='no lifetime of its own (product build)'   # mp2_product with LIFETIME_S=0
NEVER_LABELS="fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b vendorfs rootfs userfs"
PART_RE='^/dev/mmcblk[0-9]+p[0-9]+$'
OUR_FILES="boot.scr.uimg boot.scr.uimg.sha256 boot-sha256.txt u-boot-env.bin default.bin"
ENV_NAMES="mp2s_ver mp2s_left mp2s_prefixes boot_prefixes"   # all fw_setenv may name
ENV_SIZE=0x2000
FWCFG_PATH=/run/mp2s-fw_env.config   # the runner's own fw_env.config, on the board's tmpfs
FWCFG=0                              # 1: FWCFG_PATH was written (cleanup removes it)
FWOPT=                               # " -c FWCFG_PATH" then, for fw_printenv and fw_setenv
SU=${MP2_SUDO-sudo -n }
PY=(python3 -I)
MANIFEST=${MP2S_BOOT_MANIFEST:-$HERE/mp2s_boot_manifest.py}
STATE=${MP2S_STATE_DIR:-$REPO/records/stm32mp257f-dk/standalone}
# what --dry-run assumes the board answers (no board is asked)
DRY_TABLE=$'/dev/mmcblk1p1 fsbla1\n/dev/mmcblk1p2 fsbla2\n/dev/mmcblk1p3 metadata1\n/dev/mmcblk1p4 metadata2\n/dev/mmcblk1p5 fip-a\n/dev/mmcblk1p6 fip-b\n/dev/mmcblk1p7 u-boot-env\n/dev/mmcblk1p8 bootfs'

SSH_OPTS=(-o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=10 -o BatchMode=yes)
[ -z "${MP2_KNOWN_HOSTS:-}" ] || SSH_OPTS+=(-o "UserKnownHostsFile=$MP2_KNOWN_HOSTS" -o GlobalKnownHostsFile=/dev/null)

note() { echo "$TAG: $*"; }
die() { echo "$TAG: $1" >&2; exit "${2:-3}"; }
skip() { echo "skip: $*"; }

# ---- temp files, the cut-off message ---------------------------------------------------
T=
CUT="nothing was changed"     # what a signal leaves behind, set before each change
cleanup() {
    local rc=$?
    trap '' HUP INT TERM
    if [ "$FWCFG" = 1 ]; then
        timeout 30 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}rm -f -- $FWCFG_PATH" </dev/null >/dev/null 2>&1 ||
            echo "$TAG: could not remove $FWCFG_PATH from the board's tmpfs (it is gone at the next boot)" >&2
    fi
    [ -z "$T" ] || rm -rf "$T"
    exit "$rc"
}
cutoff() { trap '' HUP INT TERM; echo "$TAG: cut off: $CUT" >&2; exit "$1"; }
trap cleanup EXIT
trap 'cutoff 129' HUP
trap 'cutoff 130' INT
trap 'cutoff 143' TERM
mktmp() { [ -n "$T" ] || T=$(mktemp -d) || die "no temporary directory"; }

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
# dev_guard DEV LABEL TABLE CARD: DEV may be written only if it is a partition
# of CARD (the disk /boot is on, whose type board_facts read as SD: the eMMC
# says MMC and is never CARD, whatever its number), is not a never-written
# label, and TABLE ("PATH PARTLABEL" lines) says it carries LABEL. Pure.
dev_guard() {
    local dev=$1 want=$2 table=$3 card=$4 label l
    [[ $dev =~ $PART_RE ]] || die "REFUSED: $dev is not a partition of an mmcblk device"
    { [ -n "$card" ] && [ "${dev%p*}" = "$card" ]; } || die "REFUSED: $dev is not a partition of the SD card /boot is on (${card:-none found})"
    label=$(awk -v d="$dev" '$1 == d { print $2 }' <<<"$table" | head -1)
    for l in $NEVER_LABELS; do
        [ "$label" != "$l" ] || die "REFUSED: $dev is labelled $label: never written"
    done
    [ "$label" = "$want" ] || die "REFUSED: $dev is labelled '${label:-nothing}', not $want"
}
# read_guard DEV LABEL: a partition that is only read: of the card (so not of
# the eMMC), labelled LABEL in TABLE
read_guard() {
    local dev=$1 want=$2
    { [[ $dev =~ $PART_RE ]] && [ -n "$CARD" ] && [ "${dev%p*}" = "$CARD" ]; } || die "REFUSED: '$dev' is not a partition of the SD card"
    [ "$(awk -v d="$dev" '$1 == d { print $2 }' <<<"$TABLE" | head -1)" = "$want" ] || die "REFUSED: $dev is not labelled $want"
}
# fwset_guard FILE: every line of a fw_setenv script names one of ENV_NAMES
fwset_guard() {
    local l n ok
    [ -s "$1" ] || die "REFUSED: an empty environment write"
    while IFS= read -r l || [ -n "$l" ]; do
        n=${l%%=*}
        ok=0
        for v in $ENV_NAMES; do [ "$n" = "$v" ] && [ "$l" != "$n" ] && ok=1; done
        [ "$ok" = 1 ] || die "REFUSED: the environment write would name '$n': only $ENV_NAMES are written"
    done <"$1"
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
# bget VAR DRYVALUE CMD: a read; its output goes into VAR
bget() {
    local _v=$1 _d=$2 _o
    shift 2
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*"; printf -v "$_v" '%s' "$_d"; return 0; fi
    _o=$(b "$@") || return 1
    printf -v "$_v" '%s' "$_o"
}
# bfile OUT CMD: a read into the host file OUT
bfile() {
    local out=$1
    shift
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*   # > host:${out##*/}"; return 0; fi
    b "$@" >"$out"
}
# host CMD: a host-side check (dry-run: printed, its input is the board's)
host() {
    local s
    if [ "$DRY" = 1 ]; then
        s=${*//"$REPO"\//}
        [ -z "$T" ] || s=${s//"$T"\//}
        echo "DRY host\$ $s"
        return 0
    fi
    "$@"
}
# board_change CMD: the one place a changing command is sent
board_change() {
    if [ "$DRY" = 0 ] && [ "$PAYLOAD_READY" != 1 ]; then
        die "the payload (U-Boot script, environment write) is not wired yet: nothing was changed (PAYLOAD_READY=0)"
    fi
    b "$@"
}
# change_in FILE WHAT CMD: a change that reads FILE on its stdin
change_in() {
    local in=$1 what=$2
    shift 2
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*   # < $what"; return 0; fi
    board_change "$@" <"$in"
}
# board_rm NAME...: remove staged files; every path is checked and quoted
board_rm() {
    local p q=
    for p in "$@"; do
        guard_path "$STAGE_DIR/$p"
        q+=" $(printf '%q' "$STAGE_DIR/$p")"
    done
    board_change "${SU}rm -f --$q"
}
# board_rm_stage: the stage directory, then its parent if install made it and
# it is empty now (rmdir removes nothing else: tools/board-stm32mp2.sh keeps
# files of its own in /boot/yakogl, and then it stays). Without this an empty
# /boot/yakogl was left on the card (board run, 2026-11-22).
board_rm_stage() {
    guard_path "$STAGE_DIR"
    [ "${STAGE_DIR%/*}" = /boot/yakogl ] || die "REFUSED: the stage directory's parent is not /boot/yakogl"
    board_change "${SU}rm -rf -- $(printf '%q' "$STAGE_DIR") && { ${SU}rmdir -- $(printf '%q' "${STAGE_DIR%/*}") 2>/dev/null || true; }"
}

# take_lock: one run per board at a time, the lock board_lock.sh uses for the
# same BOARD_HOST, so board-stm32mp2.sh and this runner never overlap
take_lock() {
    local dir lock
    if [ "$DRY" = 1 ]; then echo "DRY host\$ flock the board lock (one per BOARD_HOST)"; return 0; fi
    lock=${BOARD_LOCK:-}
    if [ -z "$lock" ]; then
        dir=/run/user/$(id -u)/yakogl-locks
        mkdir -p -m 700 "$dir" 2>/dev/null || die "cannot use $dir for the board lock: nothing was changed"
        lock=$dir/board-$(printf '%s' "$BOARD_HOST" | sha256sum | cut -c1-12).lock
    fi
    exec 9>>"$lock" || die "cannot open the board lock $lock: nothing was changed"
    flock -w "${LOCK_WAIT:-600}" 9 || die "the board is busy (its lock was held for ${LOCK_WAIT:-600} s): nothing was changed"
    { echo "pid=$$"; echo "started=$(date '+%F %T')"; echo "cmd=board-stm32mp2-standalone.sh $VERB"; } >"$lock"
}

# board_facts: /boot on a partition of an SD card (not the eMMC), its
# partition table, the card's u-boot-env (CARD, TABLE, ENVDEV). The eMMC is
# known by its type (MMC), never by its number: on the DK the card's number
# changes from boot to boot and can be the lowest one (board run, 2026-11-22;
# tools/board-stm32mp2.sh found the same). Every run asks again.
board_facts() {
    local junk kind
    bget junk ok "${SU}true" || die "$BOARD_HOST does not answer (ssh key login${SU:+ and $SU})"
    bget BOOTDEV /dev/mmcblk1p8 "findmnt -n -o SOURCE /boot" || die "cannot read the device /boot is mounted from"
    [[ $BOOTDEV =~ $PART_RE ]] || die "REFUSED: /boot is on '$BOOTDEV', not a partition of an mmcblk device"
    CARD=${BOOTDEV%p*}
    bget kind SD "cat -- $(printf '%q' "/sys/block/${CARD#/dev/}/device/type")" || die "cannot read the type of $CARD"
    [ "$kind" = SD ] || die "REFUSED: $CARD says '$kind', not SD: /boot is not on the microSD (MMC is the eMMC: never used)"
    bget TABLE "$DRY_TABLE" "lsblk -rno PATH,PARTLABEL -- $(printf '%q' "$CARD")" || die "cannot read the partitions of $CARD"
    dev_guard "$BOOTDEV" bootfs "$TABLE" "$CARD"
    ENVDEV=$(awk '$2 == "u-boot-env" { print $1 }' <<<"$TABLE" | head -1)
    dev_guard "$ENVDEV" u-boot-env "$TABLE" "$CARD"
}

# stage_list: the names in the stage directory (STAGE_NAMES), each one ours
stage_list() {
    local n ok o
    bget STAGE_NAMES "${1:-}" "ls -A -- $(printf '%q' "$STAGE_DIR") 2>/dev/null; true" || die "cannot list $STAGE_DIR"
    for n in $STAGE_NAMES; do
        ok=0
        for o in $OUR_FILES; do [ "$n" = "$o" ] || [ "$n" = "$o.part" ] && ok=1; done
        [ "$n" = boot.scr.uimg.off ] && ok=1
        [ "$ok" = 1 ] || die "REFUSED: $STAGE_DIR holds '$n', which is not one of ours: a person looks first"
    done
}
staged() { [[ " ${STAGE_NAMES//$'\n'/ } " == *" $1 "* ]]; }

# build_script: this commit's boot.scr.uimg in $T/build, checked (SCRSUM)
build_script() {
    local out
    mktmp
    out=$("$REPO/driver/tools/mp2-standalone/build.sh" "$T/build" 2>&1) || die "build.sh failed: $out"
    SCRSUM=$(tail -n 1 <<<"$out" | sed -n 's/.* sha256 \([0-9a-f]\{64\}\) (via .*/\1/p')
    [ -n "$SCRSUM" ] && [ "$SCRSUM" = "$(sha256sum <"$T/build/boot.scr.uimg" | cut -d' ' -f1)" ] || die "build.sh's last line does not name the sha256 of its boot.scr.uimg: $out"
    out=$("${PY[@]}" "$HERE/mp2s_uimg_check.py" "$T/build/boot.scr.uimg" 2>&1) || die "REFUSED: tools/mp2s_uimg_check.py: $out"
    note "script: boot.scr.uimg sha256 $SCRSUM (driver/tools/mp2-standalone/build.sh, checked by mp2s_uimg_check.py)"
}

# fw_config SIZE: how fw_printenv and fw_setenv find the environment. A board
# with /etc/fw_env.config: as it is (fwenv_check judges that file). The DK as
# shipped has none (a bare fw_printenv stops with "Cannot initialize
# environment"; ST's /etc/fw_env.config.mmc names /dev/disk/by-partlabel/
# u-boot-env, which the eMMC's partition answers to as well, and the same
# offset twice): the runner then writes its own two lines, both copies of
# ENVDEV (SIZE bytes) by its device node, to FWCFG_PATH on the board's tmpfs
# and hands it over with -c (fwenv_check judges that one). Nothing is written
# to the rootfs or the card for it; cleanup removes the file.
fw_config() {
    local have fs sz=$1
    [ "$FWCFG" = 1 ] && return 0
    bget have yes "test -e /etc/fw_env.config && echo yes || echo no" || die "cannot ask the board for /etc/fw_env.config"
    [ "$have" = yes ] && return 0
    [ "$have" = no ] || die "REFUSED: the board's answer about /etc/fw_env.config reads '$have'"
    bget fs tmpfs "findmnt -n -o FSTYPE -T ${FWCFG_PATH%/*}" || die "cannot read the file system of ${FWCFG_PATH%/*}"
    [ "$fs" = tmpfs ] || die "REFUSED: ${FWCFG_PATH%/*} on the board is '$fs', not a tmpfs: the runner writes no fw_env.config to a disk"
    dev_guard "$ENVDEV" u-boot-env "$TABLE" "$CARD"
    printf '%s 0x%x 0x%x\n' "$ENVDEV" $((sz - ENV_SIZE)) $((ENV_SIZE)) "$ENVDEV" $((sz - 2 * ENV_SIZE)) $((ENV_SIZE)) >"$T/fw_env.config"
    FWCFG=1
    b "${SU}tee -- $FWCFG_PATH >/dev/null" <"$T/fw_env.config" || die "cannot write $FWCFG_PATH on the board's tmpfs"
    FWOPT=" -c $FWCFG_PATH"
    note "the board has no /etc/fw_env.config: fw_printenv and fw_setenv get -c $FWCFG_PATH (the board's tmpfs; both copies of $ENVDEV; removed when this run ends; nothing on the rootfs or the card is written for it)"
}
# read_env: the partition ($T/part.bin), both CRCs valid, then fw_printenv
# ($T/env-before.txt), which must read the partition read here (ENVLAY)
read_env() {
    local sz out
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ test -e /etc/fw_env.config   # a board without it: fw_printenv and fw_setenv get -c $FWCFG_PATH, the runner's own two lines on the board's tmpfs"
        bfile "$T/env-before.txt" "${SU}fw_printenv"
        bfile "$T/part.bin" "${SU}head -c 1048576 -- $(printf '%q' "$ENVDEV")"
        ENVLAY=(--redundant --size "$ENV_SIZE" --offset 0x7e000 --offset2 0x7c000)
        host "${PY[@]}" "$HERE/uboot_env.py" "${ENVLAY[@]}" check part.bin
        return 0
    fi
    bfile "$T/part.bin" "${SU}head -c 1048576 -- $(printf '%q' "$ENVDEV")" || die "cannot read $ENVDEV"
    sz=$(stat -c %s "$T/part.bin")
    { [ "$sz" -ge $((2 * ENV_SIZE)) ] && [ "$sz" -lt 1048576 ]; } || die "REFUSED: $ENVDEV is $sz bytes: not the u-boot-env partition this runner knows"
    ENVLAY=(--redundant --size "$ENV_SIZE" --offset $((sz - ENV_SIZE)) --offset2 $((sz - 2 * ENV_SIZE)))
    out=$("${PY[@]}" "$HERE/uboot_env.py" "${ENVLAY[@]}" check "$T/part.bin" 2>&1) || die "REFUSED: an environment copy's CRC fails on $ENVDEV: $out"
    "${PY[@]}" "$HERE/uboot_env.py" "${ENVLAY[@]}" print "$T/part.bin" | sort >"$T/part.txt" || die "cannot print $ENVDEV's environment"
    fw_config "$sz"
    bfile "$T/env-before.txt" "${SU}fw_printenv$FWOPT" || die "fw_printenv failed on the board"
    sort "$T/env-before.txt" | cmp -s - "$T/part.txt" || die "REFUSED: fw_printenv does not show the environment of $ENVDEV: fw_env.config does not point at the card's u-boot-env"
}
# env_val NAME FILE: NAME's value in a fw_printenv text
env_val() { sed -n "s/^$1=//p" "$2" | head -1; }

# fwenv_check: fw_env.config reaches only the card's u-boot-env (the tool
# goes to the board's python3 on stdin; nothing is copied there)
fwenv_check() {
    local out cfg=
    if [ "$DRY" = 1 ]; then echo "DRY board\$ python3 -I - --root /   # < tools/mp2s_fwenv_check.py"; return 0; fi
    [ "$FWCFG" = 1 ] && cfg=" --config ${FWCFG_PATH#/}"
    out=$(b "python3 -I - --root /$cfg" <"$HERE/mp2s_fwenv_check.py" 2>&1) || die "REFUSED: tools/mp2s_fwenv_check.py on the board: $out"
    note "$out"
}
# fwu_check: ST's firmware update is not in its trial state (both metadata
# copies read, never written)
fwu_check() {
    local d1 d2 out
    d1=$(awk '$2 == "metadata1" { print $1 }' <<<"$TABLE" | head -1)
    d2=$(awk '$2 == "metadata2" { print $1 }' <<<"$TABLE" | head -1)
    read_guard "$d1" metadata1
    read_guard "$d2" metadata2
    bfile "$T/fwu1.bin" "${SU}head -c 1048576 -- $(printf '%q' "$d1")" || die "cannot read $d1"
    bfile "$T/fwu2.bin" "${SU}head -c 1048576 -- $(printf '%q' "$d2")" || die "cannot read $d2"
    if [ "$DRY" = 1 ]; then host "${PY[@]}" "$HERE/fwu_metadata.py" check fwu1.bin fwu2.bin; return 0; fi
    out=$("${PY[@]}" "$HERE/fwu_metadata.py" check "$T/fwu1.bin" "$T/fwu2.bin" 2>&1) || die "REFUSED: $out: N image boots would spend ST's firmware-update trial"
    note "$out"
}

# boot_listing OUT: the sha256 listing of every file under /boot, taken on the
# board, made a manifest on the host
boot_listing() {
    # a read loop, not xargs -0: the DK's xargs is BusyBox's, without -0. A
    # name sha256sum cannot open (one with a newline) fails the listing.
    bfile "$T/listing.txt" "${SU}sh -c 'cd /boot && find . -type f | LC_ALL=C sort | while IFS= read -r f; do sha256sum \"\$f\" || exit 1; done'" || die "cannot list /boot"
    if [ "$DRY" = 1 ]; then host "${PY[@]}" "$MANIFEST" record --from listing.txt -o "${1##*/}"; return 0; fi
    "${PY[@]}" "$MANIFEST" record --from "$T/listing.txt" -o "$1" || die "mp2s_boot_manifest.py could not read the /boot listing"
}
manifest_there() {
    [ -f "$MANIFEST" ] && return 0
    [ "$DRY" = 1 ] || die "mp2s_boot_manifest.py missing ($MANIFEST): the /boot record cannot be taken or compared: nothing was changed"
    skip "mp2s_boot_manifest.py missing (a live $VERB refuses with rc 3)"
}
# install_record OUT: the install record, from the host, else the card
install_record() {
    if [ "$DRY" = 1 ]; then echo "DRY host\$ cp state/boot-sha256.txt, else the card's $STAGE_DIR/boot-sha256.txt"; return 0; fi
    if [ -s "$STATE/boot-sha256.txt" ]; then cp -- "$STATE/boot-sha256.txt" "$1"; return 0; fi
    bfile "$1" "cat -- $(printf '%q' "$STAGE_DIR/boot-sha256.txt") 2>/dev/null" && [ -s "$1" ]
}

# stage_file SRC NAME KIND: SRC to STAGE_DIR/NAME through NAME.part and a
# rename, then read back by sha256 from the card (the page cache dropped);
# KIND install also makes the stage directory. A mismatch removes the file.
stage_file() {
    local src=$1 name=$2 kind=$3 p q qp pre= sum got
    p="$STAGE_DIR/$name"
    guard_path "$p"
    guard_path "$p.part"
    q=$(printf '%q' "$p")
    qp=$(printf '%q' "$p.part")
    [ "$kind" = install ] && pre="${SU}mkdir -p -- $(printf '%q' "$STAGE_DIR") && "
    change_in "$src" "${src##*/}" "${pre}${SU}tee -- $qp >/dev/null && ${SU}sync && ${SU}mv -- $qp $q && ${SU}sync"
    [ "$DRY" = 1 ] && sum=dry || sum=$(sha256sum <"$src" | cut -d' ' -f1)
    bget got "$sum" "${SU}sh -c 'echo 3 > /proc/sys/vm/drop_caches' && sha256sum -- $q" || got=
    got=${got%% *}
    if [ "$got" != "$sum" ]; then
        board_rm "$name" "$name.part"
        die "$name did not read back from the card (sha256 ${got:-none}, want $sum): removed again" 4
    fi
}

# env_write SCRIPT WHAT: the one fw_setenv over the SD card's u-boot-env
env_write() {
    fwset_guard "$1"
    dev_guard "$ENVDEV" u-boot-env "$TABLE" "$CARD"
    change_in "$1" "$2" "${SU}fw_setenv$FWOPT -s -"
}
# env_readback EXPECTED_IMAGE VERB: fw_printenv and the partition after the
# write equal what mp2s_env_plan.py computed; else the values before it go
# back (rc 4), or NEEDS A PERSON (rc 5)
env_readback() {
    local want=$1 verb=$2 v out
    bfile "$T/env-after.txt" "${SU}fw_printenv$FWOPT" || true
    bfile "$T/part-after.bin" "${SU}head -c 1048576 -- $(printf '%q' "$ENVDEV")" || true
    if [ "$DRY" = 1 ]; then host "${PY[@]}" "$HERE/mp2s_env_plan.py" verify part.bin part-after.bin --expect "$verb"; return 0; fi
    "${PY[@]}" "$HERE/uboot_env.py" "${ENVLAY[@]}" print "$want" | sort >"$T/want.txt"
    if sort "$T/env-after.txt" | cmp -s - "$T/want.txt" &&
        out=$("${PY[@]}" "$HERE/mp2s_env_plan.py" verify "$T/part.bin" "$T/part-after.bin" --expect "$verb" "${ENVLAY[@]}" 2>&1); then
        note "read back: $out"
        return 0
    fi
    note "the environment write did not read back as planned: the values before it go back now"
    : >"$T/restore.txt"
    for v in $ENV_NAMES; do
        if grep -q "^$v=" "$T/env-before.txt"; then grep -m1 "^$v=" "$T/env-before.txt"; else echo "$v="; fi
    done >>"$T/restore.txt"
    env_write "$T/restore.txt" "the values before the write"
    bfile "$T/env-restored.txt" "${SU}fw_printenv$FWOPT" || true
    if sort "$T/env-restored.txt" | cmp -s - <(sort "$T/env-before.txt"); then
        die "$verb: the environment did not read back; it is as it was before, nothing of it is left changed" 4
    fi
    echo "$TAG: NEEDS A PERSON: the environment is neither as planned nor as before. Before it was:" >&2
    grep -E '^(mp2s_ver|mp2s_left|mp2s_prefixes|boot_prefixes)=' "$T/env-before.txt" >&2
    exit 5
}

# boot_compare RECORD WANT: /boot now against the install record; WANT is
# 'same' (uninstall) or 'allowed' (same, or differences under the stage dir)
boot_compare() {
    local rec=$1 want=$2 out rc
    boot_listing "$T/boot-now.txt"
    if [ "$DRY" = 1 ]; then host "${PY[@]}" "$MANIFEST" compare-records boot-sha256.txt boot-now.txt; return 0; fi
    out=$("${PY[@]}" "$MANIFEST" compare-records "$rec" "$T/boot-now.txt" 2>&1)
    rc=$?
    echo "$out"
    if [ "$rc" = 0 ] && { grep -q '^mp2s-boot: same:' <<<"$out" || { [ "$want" = allowed ] && grep -q '^mp2s-boot: allowed:' <<<"$out"; }; }; then
        return 0
    fi
    die "NEEDS A PERSON: /boot is not as the install record says (above); ST's files are never repaired by this runner" 5
}

# ---- the image -------------------------------------------------------------------------
check_image() {
    local img=$1 n=$2 out
    [ -f "$img" ] || die "no such image: $img" 2
    if [ "$n" = forever ] && [ "${MP2_NO_WATCHDOG:-0}" != 1 ]; then
        out=$("${PY[@]}" "$HERE/mp2s_image_check.py" "$img" --require-marker "$PRODUCT_MARKER" 2>&1) ||
            die "REFUSED: tools/mp2s_image_check.py: $(grep refused <<<"$out" || echo "$out"). 'forever' needs the product watchdog refresh ('$PRODUCT_MARKER', section 6), else BL2's IWDG resets the image in about 32 s at every boot. MP2_NO_WATCHDOG=1 accepts that on purpose"
    else
        out=$("${PY[@]}" "$HERE/mp2s_image_check.py" "$img" 2>&1) || die "REFUSED: tools/mp2s_image_check.py: $(grep refused <<<"$out" || echo "$out")"
    fi
    if [ "$n" != forever ]; then
        if [ "${MP2_NO_WATCHDOG:-0}" != 1 ] && ! grep -aqF -e "$MARKER" -e "$PRODUCT_MARKER" "$img"; then
            die "image has no watchdog marker '$MARKER' or '$PRODUCT_MARKER': BL2's IWDG would reset it in about 32 s. MP2_NO_WATCHDOG=1 accepts that on purpose"
        fi
        if grep -aqF -e "$NOLIFE_MARKER" "$img"; then
            die "REFUSED: the image is a LIFETIME_S=0 product build ('$NOLIFE_MARKER'): it never resets, so a count of $n would never run out. Use 'forever' with a person, or a build with a lifetime"
        fi
    fi
}

# ---- the undo, printed before anything changes ----------------------------------------
undo_default() {
    note "UNDO, from Linux (before the reboot, or once Linux is back): tools/board-stm32mp2-standalone.sh nodefault"
    if [ "$1" = forever ]; then
        note "UNDO, without Linux (a person at the U-Boot prompt): setenv mp2s_left 0; saveenv; boot"
        note "UNDO, with no console (a person): the card in a PC: tools/mp2s_card_undo.sh --yes <bootfs partition> (renames yakogl/standalone/boot.scr.uimg to boot.scr.uimg.off), then nodefault from Linux"
    else
        note "UNDO, without Linux: none needed: after $1 boots the script restores boot_prefixes and boots Linux"
    fi
}

# ---- verbs ---------------------------------------------------------------------------------
cmd_status() {
    [ $# -eq 0 ] || usage
    need_board
    b "ls -l -- $(printf '%q' "$STAGE_DIR")"
    if [ "$DRY" = 1 ]; then b "${SU}fw_printenv mp2s_ver mp2s_left mp2s_prefixes boot_prefixes"; return 0; fi
    mktmp
    board_facts
    read_env
    grep -E '^(mp2s_ver|mp2s_left|mp2s_prefixes|boot_prefixes)=' "$T/env-before.txt"
}
cmd_install() {
    [ $# -eq 0 ] || usage
    need_board
    manifest_there
    mktmp
    build_script
    take_lock
    note "UNDO: tools/board-stm32mp2-standalone.sh uninstall"
    board_facts
    stage_list
    if [ "$DRY" = 0 ] && [ -n "$STAGE_NAMES" ]; then
        local got=
        staged boot.scr.uimg && bget got "" "sha256sum -- $(printf '%q' "$STAGE_DIR/boot.scr.uimg")"
        if [ "${got%% *}" = "$SCRSUM" ] && staged boot-sha256.txt && staged u-boot-env.bin; then
            note "installed already, with this commit's script: nothing to do"
            return 0
        fi
        die "REFUSED: $STAGE_DIR holds files already (${STAGE_NAMES//$'\n'/ }): uninstall first"
    fi
    read_env
    boot_listing "$T/boot-sha256.txt"
    if [ "$DRY" = 0 ]; then
        mkdir -p -- "$STATE" || die "cannot make $STATE: nothing was changed"
        cp -- "$T/boot-sha256.txt" "$STATE/boot-sha256.txt" && cp -- "$T/part.bin" "$STATE/u-boot-env.bin" || die "cannot keep the install record in $STATE: nothing was changed"
        echo "$SCRSUM  boot.scr.uimg" >"$T/boot.scr.uimg.sha256"
    fi
    CUT="the stage directory may be partly written: tools/board-stm32mp2-standalone.sh uninstall"
    stage_file "$T/boot-sha256.txt" boot-sha256.txt install
    stage_file "$T/part.bin" u-boot-env.bin install
    stage_file "$T/boot.scr.uimg.sha256" boot.scr.uimg.sha256 install
    stage_file "$T/build/boot.scr.uimg" boot.scr.uimg install
    note "installed: $STAGE_DIR holds the script, its sha256, the pristine environment and the /boot record; the environment is unchanged."
}
# do_nodefault: the environment back and default.bin removed (nodefault and
# uninstall); the undo line is printed by the caller first
do_nodefault() {
    local rec
    read_env
    rec="$T/default-prefixes.txt"
    if [ "$DRY" = 0 ] && ! grep -q '^mp2s_' "$T/env-before.txt" && [[ $(env_val boot_prefixes "$T/env-before.txt") != /yakogl/standalone/* ]]; then
        note "no default is set in the environment: no environment write"
        rm -f -- "$STATE/default-prefixes.txt"   # the count ran out by itself: its record is spent
    else
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ the default record: state/default-prefixes.txt, else boot_prefixes of the pristine u-boot-env.bin"
        elif [ -f "$STATE/default-prefixes.txt" ]; then
            cp -- "$STATE/default-prefixes.txt" "$rec"
        else
            if [ -s "$STATE/u-boot-env.bin" ]; then cp -- "$STATE/u-boot-env.bin" "$T/pristine.bin"; else
                bfile "$T/pristine.bin" "cat -- $(printf '%q' "$STAGE_DIR/u-boot-env.bin")" || die "REFUSED: no default record on this host ($STATE) and no u-boot-env.bin on the card: a person compares boot_prefixes"
            fi
            printf '%s' "$("${PY[@]}" "$HERE/uboot_env.py" "${ENVLAY[@]}" print -n "$T/pristine.bin" boot_prefixes)" >"$rec" || die "REFUSED: the pristine u-boot-env.bin has no boot_prefixes"
            note "no default record on this host: boot_prefixes of the pristine u-boot-env.bin is the record"
        fi
        host "${PY[@]}" "$HERE/mp2s_env_plan.py" nodefault "$T/part.bin" "$T/part-planned.bin" --record "$rec" "${ENVLAY[@]}" | sed "s/^/  /" || die "REFUSED by mp2s_env_plan.py nodefault (above): nothing was changed"
        if [ "$DRY" = 1 ]; then
            printf '%s\n' 'boot_prefixes=<the record>' mp2s_ver= mp2s_left= mp2s_prefixes= >"$T/nodefault.txt"
        else
            : >"$T/nodefault.txt"
            [[ $(env_val boot_prefixes "$T/env-before.txt") == /yakogl/standalone/* ]] && printf 'boot_prefixes=%s\n' "$(cat "$rec")" >>"$T/nodefault.txt"
            for v in mp2s_ver mp2s_left mp2s_prefixes; do
                grep -q "^$v=" "$T/env-before.txt" && echo "$v=" >>"$T/nodefault.txt"
            done
        fi
        CUT="the environment write may be cut short: tools/board-stm32mp2-standalone.sh nodefault again"
        env_write "$T/nodefault.txt" "boot_prefixes back to the default record; deletes mp2s_ver, mp2s_left and mp2s_prefixes"
        env_readback "$T/part-planned.bin" nodefault
        [ "$DRY" = 1 ] || rm -f -- "$STATE/default-prefixes.txt"
    fi
    board_rm "$STAGED" "$STAGED.part"
}
cmd_nodefault() {
    [ $# -eq 0 ] || usage
    need_board
    mktmp
    take_lock
    note "nothing to undo: nodefault only removes"
    board_facts
    do_nodefault
    if [ -f "$MANIFEST" ]; then
        if install_record "$T/install-record.txt"; then
            boot_compare "$T/install-record.txt" allowed
        else
            note "no install record (host or card): /boot not compared"
        fi
    else
        skip "mp2s_boot_manifest.py missing: /boot not compared with the install record"
    fi
    note "no default: ST's extlinux boots Linux again"
}
cmd_uninstall() {
    [ $# -eq 0 ] || usage
    need_board
    manifest_there
    mktmp
    take_lock
    note "UNDO: tools/board-stm32mp2-standalone.sh install"
    board_facts
    stage_list
    install_record "$T/install-record.txt" || die "REFUSED: no install record on this host ($STATE) or on the card: /boot could not be compared; a person looks first"
    note "nothing to undo: nodefault only removes"
    do_nodefault
    CUT="the stage directory may be partly removed: tools/board-stm32mp2-standalone.sh uninstall again"
    board_rm_stage
    boot_compare "$T/install-record.txt" same
    [ "$DRY" = 1 ] || rm -f -- "$STATE/boot-sha256.txt" "$STATE/u-boot-env.bin"
    note "uninstalled: /boot is as the install record says. With no console and no Linux, the undo is tools/mp2s_card_undo.sh with the card in a PC"
}
cmd_default() {
    [ $# -eq 2 ] || usage
    local img=$1 n=$2 cur free got out
    case "$n" in
    [1-9] | forever) ;;
    *) die "boots must be 1-9 or forever, not '$n'" 2 ;;
    esac
    if [ "$n" = forever ] && [ "${BOARD_PERSON_PRESENT:-0}" != 1 ]; then
        die "'forever' makes the image every boot's: Linux comes back only through a person (docs/standalone-boot.md). Set BOARD_PERSON_PRESENT=1 when one is at the board, or give a boot count (1-9): the board then returns to Linux by itself." 2
    fi
    check_image "$img" "$n"
    need_board
    mktmp
    build_script
    undo_default "$n"
    take_lock
    board_facts
    stage_list
    { staged boot.scr.uimg && ! staged boot.scr.uimg.off; } || [ "$DRY" = 1 ] || die "REFUSED: not installed (no $STAGE_DIR/boot.scr.uimg, or the card undo renamed it): install first"
    bget got "$SCRSUM" "sha256sum -- $(printf '%q' "$STAGE_DIR/boot.scr.uimg")" || die "cannot read the staged script"
    [ "${got%% *}" = "$SCRSUM" ] || die "REFUSED: the staged boot.scr.uimg is not this commit's build: uninstall and install again"
    read_env
    if [ "$DRY" = 0 ]; then
        grep -q '^mp2s_left=' "$T/env-before.txt" && die "REFUSED: a default is already set ($(grep -m1 '^mp2s_left=' "$T/env-before.txt")): nodefault first"
        out=$("${PY[@]}" "$HERE/mp2s_guard.py" --for standalone --printenv "$T/env-before.txt" 2>&1) || die "REFUSED: tools/mp2s_guard.py: $out"
    else
        host "${PY[@]}" "$HERE/mp2s_guard.py" --for standalone --printenv env-before.txt
    fi
    fwenv_check
    fwu_check
    bget free 1000000000 "df -B1 --output=avail /boot | tail -n 1" || die "cannot read bootfs's free space"
    free=${free//[[:space:]]/}
    [[ $free =~ ^[0-9]+$ ]] || die "REFUSED: bootfs's free space reads '$free'"
    out=$("${PY[@]}" "$HERE/mp2s_image_check.py" "$img" --free-bytes "$free" 2>&1) || die "REFUSED: tools/mp2s_image_check.py: $(grep refused <<<"$out" || echo "$out")"
    host "${PY[@]}" "$HERE/mp2s_env_plan.py" default "$T/part.bin" "$T/part-planned.bin" --count "$n" --record "$T/default-prefixes.txt" "${ENVLAY[@]}" | sed "s/^/  /" || die "REFUSED by mp2s_env_plan.py default (above): nothing was changed"
    if [ "$DRY" = 0 ]; then
        cur=$(cat "$T/default-prefixes.txt")
        [ -n "$cur" ] && [[ $cur != *$'\n'* ]] && [ "$cur" = "$(env_val boot_prefixes "$T/env-before.txt")" ] || die "REFUSED: boot_prefixes is empty or not one line: a person looks first"
    else
        cur='<the live boot_prefixes>'
    fi
    host "${PY[@]}" "$HERE/mp2s_env_capacity.py" --image "$T/part.bin" --size "${ENVLAY[2]}" --offset "${ENVLAY[4]}" --offset2 "${ENVLAY[6]}" \
        mp2s_ver=1 "mp2s_left=$n" "mp2s_prefixes=$cur" "boot_prefixes=/yakogl/standalone/ $cur" | sed "s/^/  /" || die "REFUSED by mp2s_env_capacity.py: the edit does not fit both environment copies"
    printf '%s\n' mp2s_ver=1 "mp2s_left=$n" "mp2s_prefixes=$cur" "boot_prefixes=/yakogl/standalone/ $cur" >"$T/default.txt"
    if [ "$DRY" = 0 ]; then
        mkdir -p -- "$STATE" && cp -- "$T/default-prefixes.txt" "$STATE/default-prefixes.txt" || die "cannot keep the default record in $STATE: nothing was changed"
        note "the default record: boot_prefixes=$cur (kept in $STATE/default-prefixes.txt)"
    fi
    CUT="default.bin may be staged, the environment is not written: harmless; tools/board-stm32mp2-standalone.sh nodefault removes it"
    stage_file "$img" "$STAGED" default
    CUT="the environment write may be done: tools/board-stm32mp2-standalone.sh nodefault (from Linux), or, without Linux, the count runs out by itself"
    env_write "$T/default.txt" "$(paste -sd ' ' "$T/default.txt")"
    env_readback "$T/part-planned.bin" default
    if [ "$n" = forever ]; then note "default set forever; nothing was rebooted."; else note "default set for $n boots; nothing was rebooted."; fi
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
