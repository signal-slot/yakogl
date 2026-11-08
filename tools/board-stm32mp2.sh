#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board-stm32mp2.sh - the STM32MP257F-DK runner: a one-shot boot of a Zephyr
# image from the board's own U-Boot, off the microSD it boots from, and back
# into ST's Linux without a person (docs/notes/pre-study-stm32mp257f-dk.md
# section 6 item 1, the return trial; #1281 for what the board answered).
# Self-contained: it shares nothing with tools/board.sh but the board lock
# (driver/tools/kexec-zephyr/board_lock.sh) and the serial-bridge capture
# (tools/am62p_netboot_uart.py); its verdict line has board.sh's shape so
# the hook into board.sh (plan #322, board-gpu-identity) is a dispatch.
#
#   tools/board-stm32mp2.sh [--dry-run] status | install | uninstall | disarm
#   tools/board-stm32mp2.sh [--dry-run] probe <logdir>
#   tools/board-stm32mp2.sh [--dry-run] run <image> <logdir> [name]
#   tools/board-stm32mp2.sh env-edit hook|unhook|arm|disarm <env.bin> <out.bin> [token file]
#   tools/board-stm32mp2.sh tally <rundir>
#
# Why not tools/board.sh's uboot-mmc backend: that backend arms the one-shot
# with fw_setenv on the board, and this board's Linux has fw_printenv but no
# fw_setenv and no /etc/fw_env.config (#1281). The environment is edited on
# the host instead: the runner reads the card's `u-boot-env` partition with
# dd, edits the image byte for byte with tools/uboot_env.py and writes it
# back with dd, then proves the write with the board's own fw_printenv (it
# reads the partition with U-Boot's compiled-in layout: if it does not show
# the edit, U-Boot would not see it either, and the backup goes back at once).
#
# The U-Boot side is the standalone-boot design (docs/standalone-boot.md): a
# hook in front of the saved bootcmd, a one-shot variable U-Boot deletes and
# saves BEFORE it loads anything, the image loaded from the boot medium:
#   bootcmd      = "run yako_hook; <bootcmd as it was>"   (yako_bootcmd_prev keeps it)
#   yako_hook    : when yako_once is set, clear it, saveenv, run yako_mmc;
#                  a failed load or saveenv falls through to the saved bootcmd
#                  (ST's distro boot: extlinux, Linux)
#   yako_mmc     : ext4load mmc ${boot_instance}:${yako_part} ${yako_addr}
#                  ${yako_file}; dcache off; icache off; go ${yako_addr}
#   yako_addr    = 0xa0000000, the A35 image's link address
#                  (driver/boards/st/stm32mp257f_dk, zephyr,sram: 256 MiB)
#   yako_part    = the bootfs partition's number on the card (read from Linux
#                  at install: the partition mounted on /boot)
# `${boot_instance}` is the device number ST's U-Boot sets at boot for the
# medium it booted from (its boot_device/boot_instance variables, which the
# distro boot uses as mmc${boot_instance}); no device number is hard-coded
# and a load from a device that is not there only boots Linux. The image is
# staged on bootfs (/boot, 64 MiB ext4 with about 34 MiB free, #1281) under
# /boot/yakogl/, which is also where U-Boot reads it (ext4load of
# /yakogl/<file> on that partition). No default or "forever" form on this
# board: a boot that does not return to ST's Linux cuts the board off until
# a person is there (#1281), so every boot is a one-shot.
#
# The environment image: U-Boot's env/mmc.c puts a partition-named
# environment at the TOP of the partition, copy 1 at end - ENV_SIZE and the
# redundant copy 2 at end - 2 * ENV_SIZE; with ENV_SIZE 0x2000 and the 512 KiB
# u-boot-env partition that is 0x7e000 and 0x7c000, and 0x7e000 is where
# board-read found the CRC, flag byte and strings (the note, section 1).
# MP2_ENV_OFFSET, MP2_ENV_OFFSET2 and MP2_ENV_SIZE override; both copies'
# CRCs must check before any write, else the runner refuses.
#
# "install" prints the exact undo before it changes anything, keeps the
# pristine environment image on the host (MP2_BACKUP_DIR, default
# records/stm32mp257f-dk/uboot-env/<stamp>-u-boot-env.bin) and on the card
# (/boot/yakogl/backup/u-boot-env.bin), and records the sha256 of every file
# of /boot (boot-sha256.txt) so "status" and the return trial can show the
# stock boot files unchanged. "uninstall" puts bootcmd back and removes the
# yako_* variables; "disarm" clears a pending one-shot and staged images.
#
# "run" does, in order, and stops at the first failure:
#   1. checks the image: the AArch64 Image header, its footprint inside
#      0xa0000000..0xb0000000, and the watchdog marker "gc_wdog_arm: armed"
#      (driver/zephyr/lib/board/stm32mp2/stm32mp2_wdog.c: OP-TEE's arm,smc-wdt
#      armed by the image; BL2 has the IWDG running already, so
#      MP2_NO_WATCHDOG=1 overrides, deliberately);
#   2. takes the board lock and makes the pre-checks: ssh, no session from
#      another machine, the hook installed as this script writes it, no
#      one-shot pending, no leftover staged image, bootfs free space, and the
#      console (BOARD_UART: host:port of a serial bridge or a local device;
#      MP2_NO_CONSOLE=1 runs blind: the verdict is then the return alone);
#   3. stages the image as /boot/yakogl/once-<token>.bin and reads it back
#      by sha256 after dropping the page cache;
#   4. starts the console capture;
#   5. arms: yako_once and yako_file in one environment write, read back with
#      fw_printenv; from here any exit disarms until the reboot is issued;
#   6. reboots; waits for a NEW boot to answer ssh (uptime shorter than the
#      time since the reboot), <= RUN_BOUND + RETURN_BOUND s (defaults 120
#      and 180: the layer resets at CONFIG_YAKOGL_STM32MP2_LIFETIME_S, 20 s,
#      and Linux is back on the network about 35 s after a reset, #1281);
#   7. checks the one-shot is gone (else disarms it: rc 6), removes the
#      staged image, writes meta.txt, console.txt, INDEX.txt, redacts the
#      copies (host names -> <board-host>, IPv4 -> 192.0.2.n, the console
#      path or bridge -> <BOARD_UART>), prints the verdict line.
#
# The verdict line:
#   board-stm32mp2.sh stm32mp257f-dk <name> rc=<rc> verdict=<v> boot=uboot-sd <row|blind>
#       boot_verdict=<bv> result=<pass:<prog>|fail:<prog>|none> back_after=<s>s
# The image's end is the layer's "mp2: <why>: PSCI SYSTEM_RESET at <n> ms"
# (stm32mp2.c); its result the probe's own "<prog>: PASS: ..." or
# "<prog>: FAIL: ..." line (tools/probes/stm32mp257f_dk/mp2_identify).
# verdict=done: the board came back and the console has the end line and no
# FAIL; failed: a FAIL line; psci_returned: "PSCI SYSTEM_RESET returned"
# (the watchdog brought the board back); no_end_line: the board came back
# but the image's end was not seen (boot_verdict says how far it got:
# complete, image_started, uboot_go, not_started, no_lines); returned: a
# blind run (no console) whose board came back; not_back: it did not.
# boot_verdict is read from the console: U-Boot's "yakogl-mp2: go" line, the
# layer's "MP2:B console up" and hand-over lines, the end line.
#
# Exit codes: 0 done or returned; 2 usage; 3 refused before the board was
# changed; 4 the verdict is not done; 5 the board did not come back, or an
# environment write did not read back; 6 the one-shot was still set after
# the cycle (cleared).
#
# Environment (no default for a machine or an address; none is ever written
# to a file: the records and the messages are scrubbed):
#   BOARD_HOST       user@host of the board's Linux (root on ST's image)
#   BOARD_UART       host:port of the serial bridge of the A35 console, or a
#                    local serial device (115200); the console is a farm
#                    device attached to a node that need not be this one
#   MP2_NO_CONSOLE   1: run without a console (blind; never a row)
#   MP2_NO_WATCHDOG  1: accept an image without the marker (BL2's IWDG runs)
#   MP2_SUDO         prefix for root on the board ("" for root, "sudo -n ")
#   MP2_ENV_OFFSET, MP2_ENV_OFFSET2, MP2_ENV_SIZE  the environment's layout
#                    in its partition (0x7e000, 0x7c000, 0x2000; OFFSET2=none
#                    for a single copy)
#   MP2_BACKUP_DIR   where install keeps the pristine environment image
#   RUN_BOUND, RETURN_BOUND  seconds to the image's end and to Linux after it
#   BOARD_END_RE     extended regex of the image's end line (the layer's)
#   LOCK_WAIT, HOLD_MAX  see board_lock.sh
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
SELF="$HERE/board-stm32mp2.sh"
TAG="board-stm32mp2.sh stm32mp257f-dk"
BOARD=stm32mp257f-dk
BOOT=uboot-sd

DRY=0
args=()
for a in "$@"; do
    if [ "$a" = --dry-run ]; then DRY=1; else args+=("$a"); fi
done
set -- "${args[@]+"${args[@]}"}"
usage() { sed -n '14,18p' "$SELF" >&2; exit 2; }
[ $# -ge 1 ] || usage
VERB=$1
shift

# ---- the board's constants ----------------------------------------------------
YAKO_ADDR=0xa0000000              # the link address (stm32mp257f_dk a35_ddr_section)
LOAD_END=0xb0000000               # 256 MiB after it
MARKER='gc_wdog_arm: armed'       # stm32mp2_wdog.c:130
STAGE_DIR=/boot/yakogl            # on bootfs; U-Boot sees it as /yakogl
BACKUP_SUB=backup
ENV_LABEL=u-boot-env
ENV_OFF=${MP2_ENV_OFFSET:-0x7e000}
ENV_OFF2=${MP2_ENV_OFFSET2:-0x7c000}
ENV_SIZE=${MP2_ENV_SIZE:-0x2000}
SU=${MP2_SUDO-}
END_RE=${BOARD_END_RE:-'^mp2: .*: PSCI SYSTEM_RESET at [0-9]+ ms'}
RUN_BOUND=${RUN_BOUND:-120}
RETURN_BOUND=${RETURN_BOUND:-180}
HOLD_MAX=${HOLD_MAX:-$((RUN_BOUND + RETURN_BOUND + 300))}

# The U-Boot side, exactly as installed (fw_printenv shows them byte for
# byte; U-Boot's shell expands ${...} when the variable runs).
YAKO_HOOK='if test -n "${yako_once}"; then echo yakogl-mp2: once=${yako_once}; setenv yako_once; if saveenv; then echo yakogl-mp2: cleared; run yako_mmc; else echo yakogl-mp2: saveenv failed, booting Linux; fi; fi'
YAKO_MMC='if ext4load mmc ${boot_instance}:${yako_part} ${yako_addr} ${yako_file}; then echo yakogl-mp2: go ${yako_addr}; dcache off; icache off; go ${yako_addr}; fi; echo yakogl-mp2: mmc load failed, booting Linux'
HOOK_PREFIX='run yako_hook; '
OUR_VARS="yako_hook yako_mmc yako_addr yako_part yako_bootcmd_prev yako_once yako_file"

TMP=$(mktemp -d)
CAP=
trap 'rm -rf "$TMP"' EXIT

# ---- messages, scrubbed: no host, address or console path leaves this script ----
scrub() {
    local e=() pair s esc bh=${BOARD_HOST:-}
    for pair in "$bh|<BOARD_HOST>" "${BOARD_UART:-}|<BOARD_UART>" "${UART_REAL:-}|<BOARD_UART>" \
        "${bh#*@}|<board-host>" "${BHOSTNAME:-}|<board-host>"; do
        s=${pair%|*}
        [ -n "$s" ] || continue
        case "${pair##*|}" in "<board-host>") [ ${#s} -ge 6 ] || continue ;; esac
        esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
        e+=(-e "s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1${pair##*|}\\2/g"
            -e "s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1${pair##*|}\\2/g")
    done
    LC_ALL=C sed -E "${e[@]}" -e 's/(^|[^0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9.]|$)/\1<address>\3/g' \
        -e 's/(^|[^0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9.]|$)/\1<address>\3/g'
}
note() { echo "$TAG: $*" | scrub; }
die() { echo "$TAG: $1" | scrub >&2; exit "${2:-3}"; }
UART_REAL=
BHOSTNAME=
SSH_OPTS=(-o StrictHostKeyChecking=no -o ConnectTimeout=10 -o BatchMode=yes)
# b <command>: run on the board, bounded; in --dry-run print it instead.
b() {
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*"; return 0; fi
    { timeout 60 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "$@" 2>&1 1>&3 3>&- | scrub >&2; } 3>&1
}
# bget VAR CMD [DRY-VALUE]: a board read into VAR; in --dry-run VAR gets DRY-VALUE.
bget() {
    local __v=$1 __c=$2 __d=${3:-} __o __r=0
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ $__c"
        printf -v "$__v" '%s' "$__d"
        return 0
    fi
    __o=$(b "$__c") || __r=$?
    printf -v "$__v" '%s' "$__o"
    return "$__r"
}
# act <command...>: a host-side action with an effect; printed in --dry-run.
act() {
    if [ "$DRY" = 1 ]; then echo "DRY host\$ $*" | scrub; return 0; fi
    "$@" 2> >(scrub >&2)
}
need_board() {
    if [ "$DRY" = 1 ]; then BOARD_HOST=${BOARD_HOST:-user@BOARD}; return; fi
    : "${BOARD_HOST:?set BOARD_HOST=user@host of the board (ST Linux)}"
}
uart_is_bridge() { [[ ${BOARD_UART:-} =~ ^[^/:]+:[0-9]+$ ]]; }

# ---- the environment image, on the host -----------------------------------------------
env_tool() {
    if [ "$ENV_OFF2" = none ]; then
        python3 -I "$HERE/uboot_env.py" --size "$ENV_SIZE" --offset "$ENV_OFF" "$@"
    else
        python3 -I "$HERE/uboot_env.py" --redundant --size "$ENV_SIZE" --offset "$ENV_OFF" --offset2 "$ENV_OFF2" "$@"
    fi
}
# the byte range of the partition the copies occupy: ENV_LO..ENV_HI (dd writes only it)
env_range() {
    local lo=$((ENV_OFF)) hi=$((ENV_OFF + ENV_SIZE))
    if [ "$ENV_OFF2" != none ]; then
        [ $((ENV_OFF2)) -lt "$lo" ] && lo=$((ENV_OFF2))
        [ $((ENV_OFF2 + ENV_SIZE)) -gt "$hi" ] && hi=$((ENV_OFF2 + ENV_SIZE))
    fi
    [ $((ENV_OFF % 512)) = 0 ] || die "the environment's offset $ENV_OFF is not a multiple of 512 (dd writes whole sectors)" 2
    [ "$ENV_OFF2" = none ] || [ $((ENV_OFF2 % 512)) = 0 ] || die "the environment's second offset $ENV_OFF2 is not a multiple of 512 (dd writes whole sectors)" 2
    ENV_LO=$lo; ENV_HI=$hi
}
env_get() { env_tool print "$1" -n "$2" 2>/dev/null; }
# env_hook IN OUT PART: the hook installed into IN as OUT (bootcmd wrapped once;
# an installed hook of another version is brought to these strings, bootcmd
# untouched). EDIT_HOW says what it did: installed | upgraded | unchanged.
# (Not called in a $(...): a refusal must end the script, not a subshell.)
EDIT_HOW=
env_hook() {
    local in=$1 out=$2 part=$3 bc prev
    bc=$(env_get "$in" bootcmd) || die "the environment has no bootcmd"
    local vars=("yako_hook=$YAKO_HOOK" "yako_mmc=$YAKO_MMC" "yako_addr=$YAKO_ADDR" "yako_part=$part")
    if [ "${bc#"$HOOK_PREFIX"}" != "$bc" ]; then
        prev=$(env_get "$in" yako_bootcmd_prev)
        [ "$prev" = "${bc#"$HOOK_PREFIX"}" ] || die "bootcmd is hooked but yako_bootcmd_prev ('$prev') is not what follows the hook: not ours to touch"
        if [ "$(env_get "$in" yako_hook)" = "$YAKO_HOOK" ] && [ "$(env_get "$in" yako_mmc)" = "$YAKO_MMC" ] &&
            [ "$(env_get "$in" yako_addr)" = "$YAKO_ADDR" ] && [ "$(env_get "$in" yako_part)" = "$part" ]; then
            cp "$in" "$out"; EDIT_HOW=unchanged; return 0
        fi
        env_tool set "$in" "$out" "${vars[@]}" >/dev/null || die "uboot_env.py refused the edit" 2
        EDIT_HOW=upgraded; return 0
    fi
    case "$bc" in *yako*) die "bootcmd '$bc' names yako but does not start with '$HOOK_PREFIX': not ours to touch" ;; esac
    env_tool set "$in" "$out" "bootcmd=$HOOK_PREFIX$bc" "yako_bootcmd_prev=$bc" "${vars[@]}" >/dev/null ||
        die "uboot_env.py refused the edit" 2
    EDIT_HOW=installed
}
# env_unhook IN OUT: bootcmd back to yako_bootcmd_prev, every yako_* variable gone
env_unhook() {
    local in=$1 out=$2 bc prev
    bc=$(env_get "$in" bootcmd) || die "the environment has no bootcmd"
    if [ "${bc#"$HOOK_PREFIX"}" = "$bc" ]; then
        # not hooked: only the variables, if any
        # shellcheck disable=SC2086  # the names, one per word
        env_tool unset "$in" "$out" $OUR_VARS >/dev/null || die "uboot_env.py refused the edit" 2
        EDIT_HOW="not hooked"; return 0
    fi
    prev=$(env_get "$in" yako_bootcmd_prev)
    [ -n "$prev" ] || die "bootcmd is hooked but yako_bootcmd_prev is absent: restore bootcmd by hand ('${bc#"$HOOK_PREFIX"}')"
    env_tool set "$in" "$TMP/unhook.1" "bootcmd=$prev" >/dev/null || die "uboot_env.py refused the edit" 2
    # shellcheck disable=SC2086
    env_tool unset "$TMP/unhook.1" "$out" $OUR_VARS >/dev/null || die "uboot_env.py refused the edit" 2
    EDIT_HOW=unhooked
}
env_arm() { env_tool set "$1" "$2" "yako_once=$3" "yako_file=$4" >/dev/null || die "uboot_env.py refused the edit" 2; }
env_disarm() { env_tool unset "$1" "$2" yako_once yako_file >/dev/null || die "uboot_env.py refused the edit" 2; }

# ---- the environment partition, on the board ----------------------------------------
# where_env: ENV_DEV (the u-boot-env partition of the card Linux booted from)
# and BOOT_DEV, BOOT_PART (the bootfs partition and its number), from the
# mount of /boot. The eMMC carries the same partition labels (#1281), so the
# label is looked up on the booted card only, never system-wide.
where_env() {
    local m disk
    bget m 'set -- $(findmnt -n -v -o SOURCE,FSTYPE -T /boot); d=$1; p=${d%p*}; echo "$d $2 $p"; lsblk -rno PATH,PARTLABEL "$p"' \
        $'/dev/mmcblkDp8 ext4 /dev/mmcblkD\n/dev/mmcblkD \n/dev/mmcblkDp7 u-boot-env\n/dev/mmcblkDp8 bootfs' ||
        die "cannot read the mount of /boot and the card's partitions"
    read -r BOOT_DEV fstype disk <<<"$(head -1 <<<"$m")"
    [ "$fstype" = ext4 ] || die "/boot is '$fstype' on $BOOT_DEV, not ext4 (U-Boot's ext4load reads bootfs)"
    [[ $BOOT_DEV =~ p([0-9]+)$ ]] || die "/boot's source '$BOOT_DEV' is not a partition"
    BOOT_PART=${BASH_REMATCH[1]}
    ENV_DEV=$(awk -v l="$ENV_LABEL" '$2 == l { print $1 }' <<<"$m" | head -1)
    [ -n "$ENV_DEV" ] || die "no partition labelled $ENV_LABEL on $disk (the card /boot is on)"
    [ "$ENV_DEV" != "$BOOT_DEV" ] || die "the $ENV_LABEL partition is /boot's: refusing"
}
# env_read FILE: the whole partition into FILE (the host), both copies checked
env_read() {
    local sz
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ ${SU}dd if=$ENV_DEV bs=64k status=none   # > $1 (the whole $ENV_LABEL partition)"
        return 0
    fi
    timeout 120 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}dd if=$ENV_DEV bs=64k status=none" > "$1" ||
        die "reading $ENV_DEV failed"
    sz=$(stat -c %s "$1")
    [ "$sz" -ge "$ENV_HI" ] || die "$ENV_DEV is $sz bytes, shorter than the environment's end $ENV_HI (MP2_ENV_OFFSET*/SIZE)"
    env_tool check "$1" > "$TMP/env.check" 2>&1 ||
        die "the environment at $ENV_OFF/$ENV_OFF2 (size $ENV_SIZE) of $ENV_DEV does not check: $(tr '\n' ';' < "$TMP/env.check")"
}
# env_write FILE WHAT: the copies' range of FILE onto the partition, read back
# equal, then WHAT's variables shown by the board's fw_printenv (NAME=VALUE
# lines in $TMP/expect); on any mismatch the pristine image goes back.
env_write() {
    local file=$1 what=$2 got want
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ tail -c +$((ENV_LO + 1)) $file | head -c $((ENV_HI - ENV_LO)) | ssh BOARD '${SU}dd of=$ENV_DEV bs=512 seek=$((ENV_LO / 512)) conv=fsync status=none'"
        echo "DRY board\$ ${SU}dd if=$ENV_DEV bs=64k status=none | sha256sum   # must equal the host image"
        echo "DRY board\$ fw_printenv $(sed 's/=.*//' "$TMP/expect" | paste -sd' ' -)   # must show the edit ($what)"
        return 0
    fi
    tail -c +$((ENV_LO + 1)) "$file" | head -c $((ENV_HI - ENV_LO)) |
        timeout 120 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}dd of=$ENV_DEV bs=512 seek=$((ENV_LO / 512)) conv=fsync status=none" ||
        { env_restore; die "writing $ENV_DEV failed; the pristine image was written back" 5; }
    got=$(b "${SU}dd if=$ENV_DEV bs=64k status=none | sha256sum" | cut -d' ' -f1)
    want=$(sha256sum "$file" | cut -d' ' -f1)
    [ "$got" = "$want" ] || { env_restore; die "$ENV_DEV reads back as '$got', not $want; the pristine image was written back" 5; }
    got=$(b "${SU}fw_printenv $(sed 's/=.*//' "$TMP/expect" | paste -sd' ' -) 2>&1")
    while IFS= read -r want; do
        grep -qxF -- "$want" <<<"$got" || {
            env_restore
            die "fw_printenv on the board does not show '$want' after the write ($what): U-Boot's layout is not $ENV_OFF/$ENV_OFF2/$ENV_SIZE; the pristine image was written back (fw_printenv said: $(tr '\n' ';' <<<"$got"))" 5
        }
    done < "$TMP/expect"
}
PRISTINE=
env_restore() {
    [ -n "$PRISTINE" ] && [ -f "$PRISTINE" ] || return 0
    tail -c +$((ENV_LO + 1)) "$PRISTINE" | head -c $((ENV_HI - ENV_LO)) |
        timeout 120 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}dd of=$ENV_DEV bs=512 seek=$((ENV_LO / 512)) conv=fsync status=none" ||
        echo "$TAG: WARNING: writing the pristine environment back failed; the backup is $PRISTINE" | scrub >&2
}
# installed: INSTALLED = ok | absent | stale: <why>, from fw_printenv on the
# board (a dry run reads an installed hook; its DRY line goes to stderr so a
# caller's capture sees the word alone)
INSTALLED=
installed() {
    local out bc
    bget out "${SU}fw_printenv bootcmd $OUR_VARS 2>/dev/null" \
        "bootcmd=${HOOK_PREFIX}run bootcmd_stm32mp
yako_hook=$YAKO_HOOK
yako_mmc=$YAKO_MMC
yako_addr=$YAKO_ADDR
yako_part=8
yako_bootcmd_prev=run bootcmd_stm32mp" 1>&2 || true
    bc=$(sed -n 's/^bootcmd=//p' <<<"$out")
    if [ "${bc#"$HOOK_PREFIX"}" = "$bc" ]; then
        if grep -q '^yako_' <<<"$out"; then INSTALLED="stale: bootcmd has no hook but yako_* variables are set"; else INSTALLED=absent; fi
        return 0
    fi
    [ "$(sed -n 's/^yako_hook=//p' <<<"$out")" = "$YAKO_HOOK" ] || { INSTALLED="stale: yako_hook is another version's"; return 0; }
    [ "$(sed -n 's/^yako_mmc=//p' <<<"$out")" = "$YAKO_MMC" ] || { INSTALLED="stale: yako_mmc is another version's"; return 0; }
    [ "$(sed -n 's/^yako_addr=//p' <<<"$out")" = "$YAKO_ADDR" ] || { INSTALLED="stale: yako_addr is not $YAKO_ADDR"; return 0; }
    [ -n "$(sed -n 's/^yako_part=//p' <<<"$out")" ] || { INSTALLED="stale: yako_part is absent"; return 0; }
    INSTALLED=ok
}
fw_get() { bget "$1" "${SU}fw_printenv -n $2 2>/dev/null" "${3:-}" || true; }

# ---- the image (step 1) ----------------------------------------------------------
check_image() {
    local img=$1 magic max
    max=$((LOAD_END - YAKO_ADDR))
    [ -f "$img" ] || die "no image $img"
    magic=$(od -A n -t x1 -j 56 -N 4 "$img" | tr -d ' \n')
    [ "$magic" = 41524d64 ] || die "$img has no AArch64 Image header ('$magic' at byte 56); build with CONFIG_AARCH64_IMAGE_HEADER=y"
    footprint=$(od -A n -t u8 -j 16 -N 8 "$img" | tr -d ' ')
    { [ "$footprint" -gt 0 ] && [ "$footprint" -le "$max" ]; } ||
        die "image_size $footprint does not fit $YAKO_ADDR..$LOAD_END ($max bytes)"
    if ! grep -aqF "$MARKER" "$img"; then
        [ "${MP2_NO_WATCHDOG:-0}" = 1 ] ||
            die "REFUSED: $img does not contain \"$MARKER\": it would not arm OP-TEE's watchdog itself (BL2's IWDG is the only way back from a hang; MP2_NO_WATCHDOG=1 overrides, deliberately)"
        echo "$TAG: WARNING: MP2_NO_WATCHDOG=1: this image relies on the IWDG BL2 started" >&2
    fi
    IMG_SHA=$(sha256sum "$img" | cut -d' ' -f1)
    IMG_SIZE=$(stat -c %s "$img")
    IMG_ZEPHYR=$(grep -aoE 'Booting Zephyr OS build [0-9A-Za-z._+-]+' "$img" | head -1 | sed 's/.* build //')
    IMG_ZEPHYR=${IMG_ZEPHYR:-absent (no Zephyr banner in the image)}
}

# ---- the run directory ---------------------------------------------------------------
RD=
meta() { [ "$DRY" = 1 ] || printf '%s=%s\n' "$1" "$2" >> "$RD/meta.txt"; }
mget() { sed -n "s/^$1=//p" "${2:-$RD}/meta.txt" 2>/dev/null | tail -1; }
plain() { sed -E 's/^([0-9:]{8}|[0-9]{9,}\.[0-9]+) //' "$1" | tr -d '\0\r'; }
redact_words() { # redact_words DIR PLACEHOLDER STRING...
    local dir=$1 as=$2 f s esc files=()
    shift 2
    for f in "$dir"/*; do [ -f "$f" ] && [ "${f##*/}" != INDEX.txt ] && files+=("$f"); done
    [ ${#files[@]} -gt 0 ] || return 0
    for s in "$@"; do
        [ ${#s} -ge 6 ] || continue
        esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
        LC_ALL=C sed -i -E "s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1$as\\2/g;
            s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1$as\\2/g" "${files[@]}"
    done
}
redact_run() { # the copies never carry a host, an address or the console's path
    local dir=$1 f n=0 ip files=()
    for f in "$dir"/*; do [ -f "$f" ] && [ "${f##*/}" != INDEX.txt ] && files+=("$f"); done
    [ ${#files[@]} -gt 0 ] || return 0
    redact_words "$dir" "<BOARD_UART>" "${BOARD_UART:-}" "${UART_REAL:-}"
    if uart_is_bridge; then redact_words "$dir" "<uart-bridge>" "${BOARD_UART%:*}"; fi
    redact_words "$dir" "<board-host>" "${BHOSTNAME:-}" "${BOARD_HOST#*@}"
    while read -r ip; do
        n=$((n + 1))
        LC_ALL=C sed -i -E "s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g;
            s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g" "${files[@]}"
    done < <(LC_ALL=C grep -aohE '(^|[^0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9.]|$)' "${files[@]}" |
        LC_ALL=C grep -aoE '([0-9]{1,3}\.){3}[0-9]{1,3}' | grep -v '^192\.0\.2\.' | awk '!seen[$0]++')
}
console_from_ts() {
    [ -f "$RD/console.ts" ] || return 0
    python3 -I -c '
import re, sys, time
ts = re.compile(rb"^[0-9.]+ ?")
num = re.compile(rb"^[+-]?(?:\d+\.?\d*|\.\d+)(?:[eE][+-]?\d+)?")
out = sys.stdout.buffer
for row in sys.stdin.buffer:
    if row[-1:] == b"\n":
        row = row[:-1]
    m = num.match(row.lstrip(b" \t"))
    t = int(float(m.group())) if m else 0
    out.write(time.strftime("%H:%M:%S", time.localtime(t)).encode() + b" " + ts.sub(b"", row, 1) + b"\n")
' < "$RD/console.ts" > "$RD/console.txt"
}

# verdict_line DIR: V_LINE and V_RC from the run's files (meta.txt, console.txt)
verdict_line() {
    local dir=$1 name back t0 t1 after con bv res v rc cycle blind
    name=$(mget name "$dir"); back=$(mget back "$dir"); blind=$(mget blind "$dir")
    t0=$(mget t_reboot "$dir"); t1=$(mget t_back "$dir"); cycle=$(mget rc_cycle "$dir")
    if [ -n "$t0" ] && [ -n "$t1" ] && [ "$back" = 1 ]; then after="$((t1 - t0))s"; else after=absent; fi
    con=$dir/console.txt
    res=none
    if [ "$blind" = 1 ]; then
        bv=no_console
    elif [ ! -s "$con" ]; then
        bv=no_lines
    else
        local p
        p=$(plain "$con")
        if grep -qE "$END_RE" <<<"$p"; then bv=complete
        elif grep -qF 'PSCI SYSTEM_RESET returned' <<<"$p"; then bv=psci_returned
        elif grep -qE '^(MP2:B console up|mp2-handover: )' <<<"$p"; then bv=image_started
        elif grep -qF 'yakogl-mp2: go ' <<<"$p"; then bv=uboot_go
        else bv=not_started; fi
        local l
        l=$(grep -aoE '^[A-Za-z0-9_]+: (PASS|FAIL): ' <<<"$p" | tail -1)
        if [ -n "$l" ]; then
            case "$l" in *PASS*) res="pass:${l%%:*}" ;; *) res="fail:${l%%:*}" ;; esac
        fi
    fi
    if [ "$back" != 1 ]; then v=not_back; rc=5
    elif [ "$blind" = 1 ]; then v=returned; rc=0
    elif [ "${res%%:*}" = fail ]; then v=failed; rc=4
    elif [ "$bv" = psci_returned ]; then v=psci_returned; rc=4
    elif [ "$bv" = complete ]; then v=done; rc=0
    else v=no_end_line; rc=4; fi
    [ "$cycle" = 6 ] && [ "$rc" = 0 ] && rc=6
    local kind=row
    [ "$blind" = 1 ] && kind=blind
    V_LINE="$TAG $name rc=$rc verdict=$v boot=$BOOT $kind boot_verdict=$bv result=$res back_after=$after"
    V_RC=$rc
}
write_index() {
    local dir=$1 f
    {
        echo "# tools/board-stm32mp2.sh run record (records/README.md)"
        echo "runner: tools/board-stm32mp2.sh build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown)"
        echo "board: $BOARD"
        echo "boot: $BOOT (one-shot from the card's U-Boot)"
        echo "name: $(mget name "$dir")"
        echo "image: $(mget image "$dir") sha256 $(mget image_sha256 "$dir") size $(mget image_size "$dir") footprint $(mget footprint "$dir")"
        echo "image zephyr: $(mget image_zephyr "$dir")"
        echo "started: $(mget started "$dir")"
        echo "ended: $(mget ended "$dir")"
        echo "verdict: $V_LINE"
        echo "console lines of ours:"
        if [ -s "$dir/console.txt" ]; then
            plain "$dir/console.txt" | grep -aE '^(yakogl-mp2: |MP2:|mp2: |mp2-handover: |[A-Za-z0-9_]+: (PASS|FAIL): |\*\*\* Booting Zephyr)' | sed 's/^/  /'
        else
            echo "  none (no console: $(mget no_console_why "$dir"))"
        fi
        echo "files (sha256, lines):"
        for f in "$dir"/*; do
            { [ -f "$f" ] && [ "${f##*/}" != INDEX.txt ]; } || continue
            printf '%s  %s  %s\n' "$(sha256sum "$f" | cut -d' ' -f1)" "$(wc -l < "$f")" "${f##*/}"
        done
    } > "$dir/INDEX.txt"
}
finish() {
    meta ended "$(date '+%F %T')"
    redact_run "$RD"
    verdict_line "$RD"
    write_index "$RD"
    sed -n '/^console lines of ours:/,/^files/p' "$RD/INDEX.txt" | sed '$d'
    echo "$V_LINE"
    note "record $RD/INDEX.txt"
    exit "$V_RC"
}
stop_cap() { if [ -n "${CAP:-}" ]; then kill "$CAP" 2>/dev/null; wait "$CAP" 2>/dev/null; CAP=; fi; }

# ---- the lock and the pre-checks (step 2) ---------------------------------------------
take_lock() {
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ . driver/tools/kexec-zephyr/board_lock.sh   # HOLD_MAX=$HOLD_MAX, one lock per BOARD_HOST"
        return 0
    fi
    export HOLD_MAX
    # shellcheck source=/dev/null
    . "$REPO/driver/tools/kexec-zephyr/board_lock.sh"
}
common_prechecks() {
    local who mine others junk
    bget junk "${SU}true" || die "$BOARD_HOST does not answer (ssh key login${SU:+ and $SU})"
    bget BHOSTNAME 'uname -n' DRY-HOSTNAME || die "cannot read the board's host name"
    bget who 'who' '' || die "cannot run who on the board"
    mine=$(ip -o addr 2>/dev/null | awk '{sub(/\/.*/, "", $4); print $4}')
    # shellcheck disable=SC2086
    others=$(awk '/\(.*\)/ { a = $NF; gsub(/[()]/, "", a); print a }' <<<"$who" | sort -u |
        grep -vxF -f <(printf '%s\n' $mine) || true)
    [ -z "$others" ] || die "the board has sessions from other machines; ask before taking it:
$who"
    meta who_sessions "$(grep -c . <<<"$who")"
}
# console_check: BOARD_UART reachable and free (a bridge probed, a device
# opened); UART_REAL the device a path resolves to
console_check() {
    if uart_is_bridge; then
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ python3 tools/am62p_netboot_uart.py probe <BOARD_UART>   # must accept and hold"
        else
            python3 "$HERE/am62p_netboot_uart.py" probe "$BOARD_UART" 2> >(scrub >&2) ||
                die "the serial bridge BOARD_UART refused, is busy or unreachable (no reboot)"
        fi
        return 0
    fi
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ stty -F <BOARD_UART> 115200 cs8 -cstopb -parenb raw -echo"
        return 0
    fi
    [ -e "$BOARD_UART" ] || die "BOARD_UART is not a device on this host and not host:port of a bridge"
    UART_REAL=$(readlink -f "$BOARD_UART")
    stty -F "$BOARD_UART" 115200 cs8 -cstopb -parenb raw -echo 2> >(scrub >&2) || die "stty on BOARD_UART failed (no reboot)"
}
capture_start() {
    if uart_is_bridge; then
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ python3 tools/am62p_netboot_uart.py capture <BOARD_UART> <rundir>/console.raw <rundir>/console.ts &"
            return 0
        fi
        python3 "$HERE/am62p_netboot_uart.py" capture "$BOARD_UART" "$RD/console.raw" "$RD/console.ts" 2> >(scrub >&2) &
        CAP=$!
        sleep 3
        kill -0 "$CAP" 2>/dev/null || { CAP=; return 1; }
        meta console "serial bridge: console.raw, console.ts; console.txt derived"
        return 0
    fi
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ read <BOARD_UART> line by line into <rundir>/console.txt as 'HH:MM:SS line' &"
        return 0
    fi
    (
        exec 2>/dev/null
        while IFS= read -r line || [ -n "$line" ]; do
            printf '%(%H:%M:%S)T %s\n' -1 "${line//$'\r'/}"
        done < "$BOARD_UART" > "$RD/console.txt"
    ) &
    CAP=$!
    sleep 1
    meta console "serial device: console.txt"
}

# ---- status / install / uninstall / disarm -----------------------------------------
cmd_status() {
    need_board
    env_range
    where_env
    b "${SU}fw_printenv bootcmd bootdelay boot_targets $OUR_VARS 2>&1; echo; ls -l $STAGE_DIR $STAGE_DIR/$BACKUP_SUB 2>/dev/null; df -B1 --output=avail /boot | tail -1; cut -d' ' -f1 /proc/uptime"
    installed
    echo "install: $INSTALLED"
    echo "env partition: $ENV_DEV (copies at $ENV_OFF${ENV_OFF2:+, $ENV_OFF2}, size $ENV_SIZE); bootfs: $BOOT_DEV (U-Boot part $BOOT_PART)"
    if [ "$DRY" = 1 ]; then
        b "cd /boot && ${SU}sha256sum -c $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt --quiet   # the stock boot files against install's record"
    else
        local chk
        chk=$(b "test -f $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt && cd /boot && ${SU}sha256sum -c $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt --quiet 2>&1 && echo 'stock boot files: unchanged since install'")
        echo "${chk:-stock boot files: no record (not installed)}"
    fi
}
STAMP=$(date +%Y%m%d-%H%M%S)
backup_dir() { echo "${MP2_BACKUP_DIR:-$REPO/records/stm32mp257f-dk/uboot-env}"; }
cmd_install() {
    need_board
    env_range
    take_lock
    common_prechecks
    where_env
    local st bdir
    installed
    st=$INSTALLED
    bdir=$(backup_dir)
    PRISTINE="$bdir/$STAMP-u-boot-env.bin"
    env_read "$TMP/env.bin"
    # the undo, before anything changes
    echo "$TAG: UNDO, should anything below go wrong (the pristine environment image is kept as $PRISTINE and on the card as $STAGE_DIR/$BACKUP_SUB/u-boot-env.bin):"
    echo "  host\$ tail -c +$((ENV_LO + 1)) $PRISTINE | head -c $((ENV_HI - ENV_LO)) | ssh <BOARD_HOST> '${SU}dd of=$ENV_DEV bs=512 seek=$((ENV_LO / 512)) conv=fsync'"
    echo "  board\$ ${SU}rm -rf $STAGE_DIR"
    echo "  (or: tools/board-stm32mp2.sh uninstall)"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ mkdir -p $bdir && cp <env.bin> $PRISTINE"
        echo "DRY host\$ python3 tools/uboot_env.py ... set <env.bin> <env.new> 'bootcmd=${HOOK_PREFIX}<bootcmd>' 'yako_bootcmd_prev=<bootcmd>' 'yako_hook=$YAKO_HOOK' 'yako_mmc=$YAKO_MMC' yako_addr=$YAKO_ADDR yako_part=$BOOT_PART"
        printf 'yako_addr=%s\nyako_part=%s\n' "$YAKO_ADDR" "$BOOT_PART" > "$TMP/expect"
        b "${SU}mkdir -p $STAGE_DIR/$BACKUP_SUB && ${SU}cp <env.bin> $STAGE_DIR/$BACKUP_SUB/u-boot-env.bin && cd /boot && find . -path ./yakogl -prune -o -type f -print0 | sort -z | xargs -0 ${SU}sha256sum > $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt"
        env_write "<env.new>" "install"
        note "install (dry run): hook state read as '$st'; nothing was changed"
        return 0
    fi
    if [ "$st" = ok ]; then
        env_hook "$TMP/env.bin" "$TMP/env.new" "$BOOT_PART"
        [ "$EDIT_HOW" != unchanged ] || { note "installed already, byte for byte; nothing to do"; return 0; }
    fi
    mkdir -p "$bdir" || die "cannot create $bdir"
    cp "$TMP/env.bin" "$PRISTINE" || die "cannot keep the pristine image at $PRISTINE"
    chmod 600 "$PRISTINE"
    b "${SU}mkdir -p $STAGE_DIR/$BACKUP_SUB && test ! -e $STAGE_DIR/$BACKUP_SUB/u-boot-env.bin" ||
        die "$STAGE_DIR/$BACKUP_SUB/u-boot-env.bin exists on the card: an earlier install's pristine image; 'uninstall' first, or move it"
    timeout 120 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}sh -c 'cat > $STAGE_DIR/$BACKUP_SUB/u-boot-env.bin && sync'" < "$TMP/env.bin" ||
        die "cannot keep the pristine image on the card"
    b "cd /boot && find . -path ./yakogl -prune -o -type f -print0 | sort -z | xargs -0 ${SU}sha256sum > $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt && sync" ||
        die "cannot record the stock boot files' sha256"
    env_hook "$TMP/env.bin" "$TMP/env.new" "$BOOT_PART"
    {
        env_get "$TMP/env.new" bootcmd | sed 's/^/bootcmd=/'
        printf 'yako_hook=%s\nyako_mmc=%s\nyako_addr=%s\nyako_part=%s\n' "$YAKO_HOOK" "$YAKO_MMC" "$YAKO_ADDR" "$BOOT_PART"
    } > "$TMP/expect"
    env_write "$TMP/env.new" "install"
    note "$EDIT_HOW: U-Boot runs yako_hook before '$(env_get "$TMP/env.new" yako_bootcmd_prev)'; the one-shot loads from mmc \${boot_instance}:$BOOT_PART. Nothing was rebooted. Pristine image: $PRISTINE"
}
cmd_uninstall() {
    need_board
    env_range
    take_lock
    common_prechecks
    where_env
    local bdir
    bdir=$(backup_dir)
    PRISTINE="$bdir/$STAMP-u-boot-env.before-uninstall.bin"
    env_read "$TMP/env.bin"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ python3 tools/uboot_env.py ... set <env.bin> ... bootcmd=<yako_bootcmd_prev>; unset $OUR_VARS"
        printf 'bootcmd=<yako_bootcmd_prev>\n' > "$TMP/expect"
        env_write "<env.new>" "uninstall"
        b "${SU}rm -rf $STAGE_DIR"
        return 0
    fi
    mkdir -p "$bdir" && cp "$TMP/env.bin" "$PRISTINE" && chmod 600 "$PRISTINE" || die "cannot keep the image before the uninstall at $PRISTINE"
    env_unhook "$TMP/env.bin" "$TMP/env.new"
    printf 'bootcmd=%s\n' "$(env_get "$TMP/env.new" bootcmd)" > "$TMP/expect"
    env_write "$TMP/env.new" "uninstall"
    local left
    fw_get left yako_hook
    [ -z "$left" ] || die "yako_hook is still set after the uninstall" 5
    b "${SU}rm -rf $STAGE_DIR"
    note "$EDIT_HOW: bootcmd is '$(env_get "$TMP/env.new" bootcmd)' again, no yako_* variable, $STAGE_DIR removed. Nothing was rebooted."
}
cmd_disarm() {
    need_board
    env_range
    take_lock
    where_env
    local once
    fw_get once yako_once
    if [ -n "$once" ] || [ "$DRY" = 1 ]; then
        env_read "$TMP/env.bin"
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ python3 tools/uboot_env.py ... unset <env.bin> <env.new> yako_once yako_file"
            : > "$TMP/expect"
            env_write "<env.new>" "disarm"
        else
            env_disarm "$TMP/env.bin" "$TMP/env.new"
            : > "$TMP/expect"
            env_write "$TMP/env.new" "disarm"
            fw_get once yako_once
            [ -z "$once" ] || die "yako_once is still '$once'" 5
        fi
    fi
    b "${SU}rm -f $STAGE_DIR/once-*.bin; ls $STAGE_DIR 2>/dev/null"
    note "disarmed: no one-shot pending, no staged image"
}

# ---- probe / run -------------------------------------------------------------------
disarm_on_exit() {
    local rc=$?
    stop_cap
    if [ "${ARMED:-0}" = 1 ] && [ "$DRY" = 0 ]; then
        note "exit before the reboot: disarming the one-shot" >&2
        env_read "$TMP/env.x" 2>/dev/null && env_disarm "$TMP/env.x" "$TMP/env.y" && : > "$TMP/expect" && env_write "$TMP/env.y" "disarm" ||
            echo "$TAG: WARNING: the one-shot may still be armed: run 'disarm'" >&2
    fi
    if [ -n "${STAGED:-}" ] && [ "${REBOOTED:-0}" = 0 ] && [ "$DRY" = 0 ]; then
        b "${SU}rm -f $STAGED" || echo "$TAG: WARNING: $STAGED was not removed" >&2
    fi
    rm -rf "$TMP"
    exit "$rc"
}
cmd_run() {
    local sha8 st once left free up el rcc="" token file
    PROBE=0
    if [ "$VERB" = probe ]; then
        [ $# -ge 1 ] || usage
        PROBE=1; IMG=; LOGDIR=$1; NAME=probe
    else
        [ $# -ge 2 ] || usage
        IMG=$1; LOGDIR=$2; NAME=${3:-$(basename "$IMG" .bin)}
    fi
    [[ $NAME =~ ^[A-Za-z0-9_-]+$ ]] || die "name '$NAME': letters, digits, _ and - only" 2
    need_board
    env_range
    BLIND=0
    if [ -z "${BOARD_UART:-}" ]; then
        if [ "${MP2_NO_CONSOLE:-0}" = 1 ]; then BLIND=1
        elif [ "$DRY" = 1 ]; then BOARD_UART=BRIDGE:PORT
        else die "set BOARD_UART=host:port of the console's serial bridge, or a local serial device (MP2_NO_CONSOLE=1 runs blind: the verdict is the return alone, never a row)" 2; fi
    fi
    if [ "$PROBE" = 1 ]; then
        IMG_SHA=none; IMG_SIZE=0; footprint=none; sha8=probe; IMG_ZEPHYR="absent (probe: no image)"
    else
        check_image "$IMG"
        sha8=${IMG_SHA:0:8}
    fi
    RD="$LOGDIR/$(date +%Y%m%d-%H%M)-$NAME-$sha8"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ mkdir -p $RD   # meta.txt, console.txt, INDEX.txt"
    else
        [ -e "$RD" ] && die "$RD exists: a record is never overwritten"
        mkdir -p "$RD" || die "cannot create $RD"
    fi
    meta board "$BOARD"; meta boot "$BOOT"; meta name "$NAME"; meta probe "$PROBE"; meta blind "$BLIND"
    [ "$BLIND" = 1 ] && meta no_console_why "MP2_NO_CONSOLE=1"
    meta image "${IMG:+$(basename "$IMG")}"; meta image_sha256 "$IMG_SHA"; meta image_size "$IMG_SIZE"
    meta footprint "${footprint:-none}"; meta image_zephyr "$IMG_ZEPHYR"
    meta started "$(date '+%F %T')"
    note "$NAME: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown) boot $BOOT image sha256 ${IMG_SHA:0:16} footprint ${footprint:-none}"
    [ "$BLIND" = 1 ] || console_check
    take_lock
    bget st 'systemctl is-system-running --wait >/dev/null 2>&1; cut -d" " -f1 /proc/uptime' 100 || die "the board's Linux does not answer"
    common_prechecks
    where_env
    installed
    st=$INSTALLED
    [ "$st" = ok ] || die "the U-Boot hook is not installed as this script writes it ($st): 'install' first"
    fw_get once yako_once
    [ -z "$once" ] || die "a one-shot is already pending (yako_once=$once); find its owner, or 'disarm'"
    bget left "ls $STAGE_DIR/once-*.bin 2>/dev/null   # must print nothing" ''
    [ -z "$left" ] || die "$STAGE_DIR holds a one-shot image of a run that did not clean up ($left): look at it, or 'disarm'"
    bget free "df -B1 --output=avail /boot | tail -1" 30000000 || die "df on the board failed"
    free=${free// /}
    [[ $free =~ ^[0-9]+$ ]] || die "cannot read the free space of /boot ('$free')"
    [ "$PROBE" = 1 ] || [ "$free" -ge $((2 * IMG_SIZE)) ] || die "/boot has $free bytes free, less than twice the image ($IMG_SIZE)"
    if [ "$PROBE" = 1 ]; then
        note "probe: the pre-checks passed (hook $st, bootfs $BOOT_DEV part $BOOT_PART, env $ENV_DEV); no image ran, the board was not rebooted"
        exit 0
    fi

    # 3. stage, read back
    token=$(printf '%s' "$NAME-${IMG_SHA:0:16}-$(date +%s)" | tr -c 'A-Za-z0-9._-' '_')
    file="once-$token.bin"
    meta token "$token"
    ARMED=0; REBOOTED=0; STAGED=
    trap disarm_on_exit EXIT
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ ${SU}sh -c 'mkdir -p $STAGE_DIR && cat > $STAGE_DIR/$file.part && sync && mv $STAGE_DIR/$file.part $STAGE_DIR/$file && sync' < $IMG"
    else
        timeout 900 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}sh -c 'mkdir -p $STAGE_DIR && cat > $STAGE_DIR/$file.part && sync && mv $STAGE_DIR/$file.part $STAGE_DIR/$file && sync'" < "$IMG" ||
            { b "${SU}rm -f $STAGE_DIR/$file.part $STAGE_DIR/$file"; die "copying the image to $STAGE_DIR/$file failed"; }
    fi
    STAGED=$STAGE_DIR/$file
    bget st "echo 3 | ${SU}tee /proc/sys/vm/drop_caches >/dev/null && sha256sum $STAGE_DIR/$file" "$IMG_SHA  $STAGE_DIR/$file"
    st=${st%% *}
    [ "$st" = "$IMG_SHA" ] || die "the card's copy $STAGE_DIR/$file reads back as '$st', not $IMG_SHA (removed)"
    meta staged "$STAGE_DIR/$file on bootfs, read back equal"

    # 4. the capture, before anything can print
    if [ "$BLIND" = 0 ]; then capture_start || die "the console capture ended at once (bridge busy or unreachable; no reboot)"; fi

    # 5. arm: one environment write, read back by fw_printenv
    env_read "$TMP/env.bin"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ python3 tools/uboot_env.py ... set <env.bin> <env.new> yako_once=$token yako_file=/yakogl/$file"
    else
        env_arm "$TMP/env.bin" "$TMP/env.new" "$token" "/yakogl/$file"
        ARMED=1
    fi
    printf 'yako_once=%s\nyako_file=/yakogl/%s\n' "$token" "$file" > "$TMP/expect"
    if [ "$DRY" = 1 ]; then env_write "<env.new>" "arm"; else env_write "$TMP/env.new" "arm"; fi
    meta t_arm "$(date +%s)"

    # 6. reboot; the ssh session ends with it
    b "sync; ${SU}systemctl reboot" || true
    ARMED=0   # the board owns the one-shot now; U-Boot clears it
    REBOOTED=1
    T_REBOOT=$(date +%s)
    meta t_reboot "$T_REBOOT"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ wait until ssh answers from a new boot (uptime < time since the reboot), <= $((RUN_BOUND + RETURN_BOUND)) s"
        b "${SU}fw_printenv -n yako_once   # must print nothing"
        b "${SU}rm -f $STAGE_DIR/$file"
        echo "DRY host\$ the verdict line from the run directory"
        exit 0
    fi
    while :; do
        sleep 5
        el=$(($(date +%s) - T_REBOOT))
        up=$(timeout 8 ssh "${SSH_OPTS[@]}" -o ConnectTimeout=5 "$BOARD_HOST" 'cut -d. -f1 /proc/uptime' 2>/dev/null)
        if [ -n "$up" ] && [ "$up" -lt "$el" ]; then break; fi
        if [ "$el" -gt $((RUN_BOUND + RETURN_BOUND)) ]; then rcc=5; break; fi
    done
    [ -n "$up" ] && meta up "$up"
    meta t_back "$(date +%s)"

    # 7. the one-shot must be gone; the staged image removed; the verdict
    if [ "$rcc" != 5 ]; then
        meta back 1
        fw_get left yako_once
        if [ -n "$left" ]; then
            note "yako_once is still '$left' after the cycle: disarming" >&2
            env_read "$TMP/env.x" && env_disarm "$TMP/env.x" "$TMP/env.y" && : > "$TMP/expect" && env_write "$TMP/env.y" "disarm"
            rcc=6
        fi
        if b "${SU}rm -f $STAGED && ! test -e $STAGED"; then
            meta cleanup "$STAGED removed (read back)"
        else
            meta cleanup "$STAGED NOT removed"
            note "warning: $STAGED was not removed" >&2
        fi
    else
        meta back 0
        meta cleanup "not done: Linux not back ($STAGED left on the card)"
    fi
    [ -n "$rcc" ] && meta rc_cycle "$rcc"
    stop_cap
    console_from_ts
    finish
}

# ---- env-edit (the host-side edit alone, on files) and tally -------------------------------
cmd_env_edit() {
    [ $# -ge 3 ] || usage
    local what=$1 in=$2 out=$3
    env_range
    [ -f "$in" ] || die "no environment image $in" 2
    env_tool check "$in" > "$TMP/env.check" 2>&1 || die "$in does not check: $(tr '\n' ';' < "$TMP/env.check")" 2
    case "$what" in
    hook) env_hook "$in" "$out" "${4:-8}"; echo "$EDIT_HOW" ;;
    unhook) env_unhook "$in" "$out"; echo "$EDIT_HOW" ;;
    arm) [ $# -ge 5 ] || usage; env_arm "$in" "$out" "$4" "$5"; echo armed ;;
    disarm) env_disarm "$in" "$out"; echo disarmed ;;
    *) usage ;;
    esac
}
cmd_tally() {
    [ $# -ge 1 ] || usage
    local dir=$1
    [ -f "$dir/meta.txt" ] || die "$dir has no meta.txt" 2
    verdict_line "$dir"
    echo "$V_LINE"
    exit "$V_RC"
}

case "$VERB" in
status) cmd_status "$@" ;;
install) cmd_install "$@" ;;
uninstall) cmd_uninstall "$@" ;;
disarm) cmd_disarm "$@" ;;
probe | run) cmd_run "$@" ;;
env-edit) cmd_env_edit "$@" ;;
tally) cmd_tally "$@" ;;
*) usage ;;
esac
