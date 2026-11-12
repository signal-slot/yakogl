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
#   tools/board-stm32mp2.sh [--dry-run] status | install [image] | uninstall | disarm
#   tools/board-stm32mp2.sh [--dry-run] probe <logdir>
#   tools/board-stm32mp2.sh [--dry-run] run [--expect-watchdog] <image> <logdir> [name]
#   tools/board-stm32mp2.sh env-edit hook|unhook|arm|disarm <env.bin> <out.bin> [token file]
#   tools/board-stm32mp2.sh tally <rundir>
#   tools/board-stm32mp2.sh selftest          (tools/board_stm32mp2_selftest.sh, no board)
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
#                  bootcmd is ST's variable, not the runner's: the runner never
#                  writes it. "install" writes the yako_* variables alone and,
#                  while bootcmd does not begin with "run yako_hook; ", stops
#                  with NEEDS A PERSON (rc 7) and the exact line a person who
#                  allows it types at U-Boot's prompt; "run" refuses until
#                  then. "uninstall" likewise asks the person to put bootcmd
#                  back before it removes the hook's variables.
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
# stock boot files unchanged (boot.scr.uimg, the extlinux files, Image.gz and
# the device trees are never written: the one-shot is a variable, not a
# label). With an image, "install" checks it first, as "run" does, and
# refuses before anything changes. "uninstall" checks the stock boot files
# against boot-sha256.txt and removes the yako_* variables once bootcmd no
# longer names the hook; "disarm" clears a pending one-shot and staged images
# (once-*.bin and a cut-off copy's once-*.bin.part), whatever is left of a run
# that did not finish, and is safe to repeat.
#
# Undo that survives the run being cut off: a TERM (the lock's watchdog at
# HOLD_MAX), INT or HUP ends the run through its EXIT trap at once, because
# every ssh and sleep runs in the background and is waited for (bash holds a
# trapped signal back until a foreground child ends, and without a TERM trap
# it dies without running the EXIT trap at all). The trap writes the pre-arm
# environment back while the one-shot is still the runner's, removes a staged
# image the board has not booted, and stops the console capture. Before each
# change the runner writes what it is about to leave behind to a state file
# beside the board lock (<lock>.mp2-left, the lock's hashed name: no
# address), so a SIGKILL that runs no trap still leaves the record; the next
# run, "status" and "disarm" print it, and "disarm" clears it with the
# board's state. The run's meta.txt carries the same "left=" lines.
#
# What the runner may write, enforced before every write (dev_guard): the
# card's u-boot-env partition (the two copies' bytes, nothing else of it, and
# of the variables only its own yako_*) and files under /boot/yakogl on the
# card's bootfs. The card is the disk /boot is mounted from, and it must say
# it is an SD card (/sys/block/<disk>/device/type "SD"): an eMMC says "MMC"
# whatever its number. It refuses any device of mmcblk0 as well, any device
# that is not a partition of that card, and any partition labelled fsbla1,
# fsbla2, metadata1, metadata2, fip-a, fip-b, vendorfs, rootfs or userfs, and
# the partition it writes must carry the label it expects (u-boot-env,
# bootfs). It never repartitions, loads or unloads a module, or touches
# tailscale or sshd: every command it sends is in this file, and the
# self-test's fake board logs them. ssh checks the board's host key and
# never records one (StrictHostKeyChecking=yes, UpdateHostKeys=no): the key
# must be known before the first use (MP2_KNOWN_HOSTS names a file of its
# own); ~/.ssh/known_hosts is never written.
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
#      prints the UNDO of steps 3 and 5 ('disarm') before either;
#   3. stages the image as /boot/yakogl/once-<token>.bin and reads it back
#      by sha256 after dropping the page cache;
#   4. starts the console capture;
#   5. arms: yako_once and yako_file in one environment write, read back with
#      fw_printenv; from here any exit disarms (the pre-arm image written
#      back) until the board is seen to leave its Linux;
#   6. reboots; waits for a NEW boot to answer ssh (uptime shorter than the
#      time since the reboot), <= RUN_BOUND + RETURN_BOUND s (defaults 120
#      and 180: the layer resets at CONFIG_YAKOGL_STM32MP2_LIFETIME_S, 20 s,
#      and Linux is back on the network about 35 s after a reset, #1281);
#      the old boot still answering MP2_REBOOT_GRACE s (90) after the reboot
#      means the reboot did not happen: the runner disarms, removes the image
#      and says verdict=not_rebooted (rc 4), never NEEDS A PERSON;
#   7. checks the one-shot is gone (else disarms it: rc 6), removes the
#      staged image, writes meta.txt, console.txt, INDEX.txt, redacts the
#      copies (host names -> <board-host>, IPv4 -> 192.0.2.n, the console
#      path or bridge -> <BOARD_UART>), prints the verdict line.
#
# "run --expect-watchdog" is the return trial of a run that hangs on purpose
# (tools/probes/stm32mp257f_dk/mp2_hello built with CONFIG_MP2_HELLO_HANG=y:
# interrupts locked, so neither the layer's lifetime timer nor a PSCI reset
# can fire; OP-TEE's watchdog must reset the board in about 32 s). The wait is
# then WDOG_BOUND s (default 120) from the reboot. The verdict is
# verdict=watchdog_reset (rc 0) when the console has the banner and the hang
# line "mp2_hello: hanging on purpose", no PSCI SYSTEM_RESET line followed
# (neither "at <n> ms" nor "returned"), and the board came back to Linux over
# ssh within the bound; boot_verdict is hung. Otherwise: psci_reset_before_wdog
# (the layer or the image reset the board itself, rc 4), no_hang (the banner
# but no hang line, rc 4), no_end_line (rc 4, no banner), not_back (rc 5). An
# image that does not arm the watchdog itself needs MP2_NO_WATCHDOG=1 (BL2's
# IWDG is what resets it), as for any run. The record carries expect_wdog=1.
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
# blind run (no console) whose board came back; not_back: it did not;
# not_rebooted: the board's Linux never went away (disarmed, rc 4).
# boot_verdict is read from the console: U-Boot's "yakogl-mp2: go" line, the
# layer's "MP2:B console up" and hand-over lines, the end line.
# After a not_back verdict the runner prints one more line, "NEEDS A PERSON":
# the board did not answer within the wait, nothing resets it from outside,
# and the runner never retries; the board is released and reported, and no
# other item takes it until its owner says it is back (plan #322).
#
# Exit codes: 0 done or returned; 2 usage; 3 refused before the board was
# changed; 4 the verdict is not done; 5 the board did not come back, or an
# environment write did not read back; 6 the one-shot was still set after
# the cycle (cleared); 7 NEEDS A PERSON for bootcmd (install, uninstall,
# run: bootcmd is ST's and the runner never changes it); 129, 130, 143 ended
# by HUP, INT, TERM (after the undo; the state file says what is left).
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
#   WDOG_BOUND       with --expect-watchdog: seconds from the reboot to Linux
#                    back (default 120)
#   MP2_HOLD_S       opt-in, unset = no hold (output unchanged): seconds to
#                    hold after the image's "<prog>: PASS"/"FAIL" line, so an
#                    observer (the KVM) can take a screenshot of a display
#                    probe. When the line shows on the console the runner
#                    prints "hold: result seen at <UTC ISO time>, holding <s>s",
#                    waits that long without polling ssh, and goes on. The
#                    runner cannot keep the image running: a display probe
#                    must keep scanning out at least MP2_HOLD_S after its
#                    result line, before it returns or resets. The bound to
#                    Linux (RUN_BOUND + RETURN_BOUND, or WDOG_BOUND) is
#                    extended by MP2_HOLD_S, so a hold does not turn a board
#                    that is back into not_back; wait_bound in meta.txt
#                    carries the extended value, hold_s the hold. The verdict
#                    line is the same.
#   BOARD_END_RE     extended regex of the image's end line (the layer's)
#   MP2_KNOWN_HOSTS  a known_hosts file of its own for the board's host key
#                    (read only; default ssh's own files, read only too)
#   MP2_REBOOT_GRACE seconds after the reboot command by which the old boot
#                    must have stopped answering (90)
#   LOCK_WAIT, HOLD_MAX  see board_lock.sh; HOLD_MAX defaults to the wait
#                    bound (with the hold) + 900 s of staging + 300 s, so the
#                    lock's watchdog never ends a run inside its own bounds
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
SELF="$HERE/board-stm32mp2.sh"
TAG="board-stm32mp2.sh stm32mp257f-dk"
BOARD=stm32mp257f-dk
BOOT=uboot-sd

DRY=0
EXPECT_WDOG=0
args=()
for a in "$@"; do
    case "$a" in
    --dry-run) DRY=1 ;;
    --expect-watchdog) EXPECT_WDOG=1 ;;
    *) args+=("$a") ;;
    esac
done
set -- "${args[@]+"${args[@]}"}"
usage() { sed -n '14,19p' "$SELF" >&2; exit 2; }
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
BOOT_LABEL=bootfs
# never written, whatever the arguments: the boot chain's partitions and the
# file systems of ST's image (the card's layout, #1281), and the eMMC as a whole
NEVER_LABELS="fsbla1 fsbla2 metadata1 metadata2 fip-a fip-b vendorfs rootfs userfs"
EMMC_RE='^/dev/mmcblk0(p[0-9]+)?$'
PART_RE='^/dev/mmcblk[^/ ]+p[0-9]+$'   # a partition of an mmcblk device (the dry run's is mmcblkD)
ENV_OFF=${MP2_ENV_OFFSET:-0x7e000}
ENV_OFF2=${MP2_ENV_OFFSET2:-0x7c000}
ENV_SIZE=${MP2_ENV_SIZE:-0x2000}
SU=${MP2_SUDO-}
END_RE=${BOARD_END_RE:-'^mp2: .*: PSCI SYSTEM_RESET at [0-9]+ ms'}
RUN_BOUND=${RUN_BOUND:-120}
RETURN_BOUND=${RETURN_BOUND:-180}
WDOG_BOUND=${WDOG_BOUND:-120}
if [ "$EXPECT_WDOG" = 1 ]; then WAIT_BOUND=$WDOG_BOUND; else WAIT_BOUND=$((RUN_BOUND + RETURN_BOUND)); fi
HOLD_S=${MP2_HOLD_S:-}
case "$HOLD_S" in
'') ;;
*[!0-9]*) echo "board-stm32mp2.sh: MP2_HOLD_S must be whole seconds, got '$HOLD_S'" >&2; exit 2 ;;
*) WAIT_BOUND=$((WAIT_BOUND + HOLD_S)) ;;
esac
REBOOT_GRACE=${MP2_REBOOT_GRACE:-90}
case "$REBOOT_GRACE" in '' | *[!0-9]*) echo "board-stm32mp2.sh: MP2_REBOOT_GRACE must be whole seconds, got '$REBOOT_GRACE'" >&2; exit 2 ;; esac
STAGE_TIMEOUT=900                 # the image's copy to the card
# the lock's watchdog must not fire inside the run's own bounds: the copy,
# the wait (with a hold) and 300 s for the checks around them
HOLD_MAX=${HOLD_MAX:-$((WAIT_BOUND + STAGE_TIMEOUT + 300))}

# The U-Boot side, exactly as installed (fw_printenv shows them byte for
# byte; U-Boot's shell expands ${...} when the variable runs).
YAKO_HOOK='if test -n "${yako_once}"; then echo yakogl-mp2: once=${yako_once}; setenv yako_once; if saveenv; then echo yakogl-mp2: cleared; run yako_mmc; else echo yakogl-mp2: saveenv failed, booting Linux; fi; fi'
YAKO_MMC='if ext4load mmc ${boot_instance}:${yako_part} ${yako_addr} ${yako_file}; then echo yakogl-mp2: go ${yako_addr}; dcache off; icache off; go ${yako_addr}; fi; echo yakogl-mp2: mmc load failed, booting Linux'
HOOK_PREFIX='run yako_hook; '
OUR_VARS="yako_hook yako_mmc yako_addr yako_part yako_bootcmd_prev yako_once yako_file"

TMP=$(mktemp -d)
CAP=
# ---- signals: a TERM, INT or HUP ends the script through on_exit at once -----------
# bash runs a trap only when the foreground child (or a $(...)) has ended,
# and with no TERM trap at all it dies without running the EXIT trap. So the
# signals are trapped, and every ssh and sleep goes through fgw: in the
# background, waited for, killed by on_exit.
FG=
SIGNALLED=
on_signal() { SIGNALLED=$1; echo "board-stm32mp2.sh: SIG$1: ending through the undo" >&2; exit "$2"; }
trap 'on_signal TERM 143' TERM
trap 'on_signal INT 130' INT
trap 'on_signal HUP 129' HUP
# fgw CMD...: CMD in the background (stdin kept: 0<&0), waited for; without
# the lock's fd 9, so nothing it leaves behind holds the board lock
fgw() {
    local r=0
    "$@" 0<&0 9>&- &
    FG=$!
    wait "$FG" || r=$?
    FG=
    return "$r"
}
ENV_PUSHING=0  # 1 while env_push writes the partition: never cut in half
kill_fg() {
    [ -n "${FG:-}" ] || return 0
    if [ "$ENV_PUSHING" = 1 ]; then
        echo "board-stm32mp2.sh: waiting for the environment write in flight (bounded by its own timeout)" >&2
        wait "$FG" 2>/dev/null
        FG=
        return 0
    fi
    pkill -TERM -P "$FG" 2>/dev/null
    kill -TERM "$FG" 2>/dev/null
    FG=
}
trap on_exit EXIT

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
# the host key is checked and never recorded: ~/.ssh/known_hosts is not
# written (a board whose key is unknown is refused before any change)
SSH_OPTS=(-o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=10 -o BatchMode=yes)
[ -z "${MP2_KNOWN_HOSTS:-}" ] || SSH_OPTS+=(-o "UserKnownHostsFile=$MP2_KNOWN_HOSTS" -o GlobalKnownHostsFile=/dev/null)
# mp2_ssh SECONDS ARGS...: ssh to the board, bounded, in the background and
# waited for (not "bssh": board_lock.sh defines that one when it is sourced)
mp2_ssh() { local t=$1; shift; fgw timeout "$t" ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "$@"; }
# b <command>: run on the board, bounded; in --dry-run print it instead.
b_ssh() { { timeout 60 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "$@" 2>&1 1>&3 3>&- | scrub >&2; } 3>&1; }
b() {
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*"; return 0; fi
    fgw b_ssh "$@"
}
# bget VAR CMD [DRY-VALUE]: a board read into VAR; in --dry-run VAR gets DRY-VALUE.
# (through a file, not $(...): a signal is not held back while it runs)
bget() {
    local __v=$1 __c=$2 __d=${3:-} __r=0
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ $__c"
        printf -v "$__v" '%s' "$__d"
        return 0
    fi
    b "$__c" > "$TMP/bget.out" || __r=$?
    printf -v "$__v" '%s' "$(cat "$TMP/bget.out")"
    return "$__r"
}
# act <command...>: a host-side action with an effect; printed in --dry-run.
act() {
    if [ "$DRY" = 1 ]; then echo "DRY host\$ $*" | scrub; return 0; fi
    "$@" 2> >(scrub >&2)
}
need_board() {
    if [ "$DRY" = 1 ]; then BOARD_HOST=${BOARD_HOST:-user@BOARD}; return; fi
    [ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=user@host of the board's Linux (root on ST's image); it comes from the lease, there is no default" 2
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
# env_hook IN OUT PART: the hook's own variables installed into IN as OUT;
# bootcmd is ST's and is never written (an installed hook of another version
# is brought to these strings). yako_bootcmd_prev keeps bootcmd as it is
# without the hook, for the person who puts it back. EDIT_HOW says what it
# did: installed | upgraded | unchanged; HOOKED is 1 when bootcmd already
# begins with the hook (a person put it there), else 0: the one-shot cannot
# run until a person does (bootcmd_line prints what they type).
# (Not called in a $(...): a refusal must end the script, not a subshell.)
EDIT_HOW=
HOOKED=0
BOOTCMD_BARE=
env_hook() {
    local in=$1 out=$2 part=$3 bc prev
    bc=$(env_get "$in" bootcmd) || die "the environment has no bootcmd"
    if [ "${bc#"$HOOK_PREFIX"}" != "$bc" ]; then
        HOOKED=1; BOOTCMD_BARE=${bc#"$HOOK_PREFIX"}
    else
        case "$bc" in *yako*) die "bootcmd '$bc' names yako but does not start with '$HOOK_PREFIX': not ours to touch" ;; esac
        HOOKED=0; BOOTCMD_BARE=$bc
    fi
    prev=$(env_get "$in" yako_bootcmd_prev) || prev=
    # this runner always writes yako_bootcmd_prev with its hook: a hook without it is someone else's
    if [ -z "$prev" ] && env_get "$in" yako_hook >/dev/null; then
        die "yako_hook is set but yako_bootcmd_prev is not: another origin's hook, not ours to touch"
    fi
    [ -z "$prev" ] || [ "$prev" = "$BOOTCMD_BARE" ] ||
        die "yako_bootcmd_prev ('$prev') is not bootcmd without the hook ('$BOOTCMD_BARE'): not ours to touch"
    local vars=("yako_hook=$YAKO_HOOK" "yako_mmc=$YAKO_MMC" "yako_addr=$YAKO_ADDR" "yako_part=$part" "yako_bootcmd_prev=$BOOTCMD_BARE")
    if [ "$(env_get "$in" yako_hook)" = "$YAKO_HOOK" ] && [ "$(env_get "$in" yako_mmc)" = "$YAKO_MMC" ] &&
        [ "$(env_get "$in" yako_addr)" = "$YAKO_ADDR" ] && [ "$(env_get "$in" yako_part)" = "$part" ] && [ "$prev" = "$BOOTCMD_BARE" ]; then
        cp "$in" "$out"; EDIT_HOW=unchanged; return 0
    fi
    if env_get "$in" yako_hook >/dev/null; then EDIT_HOW=upgraded; else EDIT_HOW=installed; fi
    env_tool set "$in" "$out" "${vars[@]}" >/dev/null || die "uboot_env.py refused the edit" 2
}
# bootcmd_line hook|unhook: what a person types at U-Boot's prompt (autoboot
# stopped) to put the hook in front of bootcmd, or to take it away again
bootcmd_line() {
    case "$1" in
    hook) echo "setenv bootcmd '$HOOK_PREFIX$BOOTCMD_BARE'; saveenv" ;;
    unhook) echo "setenv bootcmd '$BOOTCMD_BARE'; saveenv" ;;
    esac
}
# env_unhook IN OUT: every yako_* variable gone, once bootcmd no longer names
# the hook; while it does (HOOKED=1), only the one-shot (yako_once,
# yako_file) goes and the hook stays, inert without yako_once, until a person
# puts bootcmd back (bootcmd_line unhook). bootcmd is never written.
env_unhook() {
    local in=$1 out=$2 bc
    bc=$(env_get "$in" bootcmd) || die "the environment has no bootcmd"
    if [ "${bc#"$HOOK_PREFIX"}" != "$bc" ]; then
        HOOKED=1; BOOTCMD_BARE=${bc#"$HOOK_PREFIX"}
        env_tool unset "$in" "$out" yako_once yako_file >/dev/null || die "uboot_env.py refused the edit" 2
        EDIT_HOW="bootcmd still hooked"; return 0
    fi
    HOOKED=0; BOOTCMD_BARE=$bc
    # shellcheck disable=SC2086  # the names, one per word
    env_tool unset "$in" "$out" $OUR_VARS >/dev/null || die "uboot_env.py refused the edit" 2
    EDIT_HOW=unhooked
}
env_arm() { env_tool set "$1" "$2" "yako_once=$3" "yako_file=$4" >/dev/null || die "uboot_env.py refused the edit" 2; }
env_disarm() { env_tool unset "$1" "$2" yako_once yako_file >/dev/null || die "uboot_env.py refused the edit" 2; }

# ---- the environment partition, on the board ----------------------------------------
# where_env: ENV_DEV (the u-boot-env partition of the card Linux booted from)
# and BOOT_DEV, BOOT_PART (the bootfs partition and its number), from the
# mount of /boot. The eMMC carries the same partition labels (#1281), so the
# label is looked up on the booted card only, never system-wide.
PART_TABLE=   # "PATH PARTLABEL" lines of the booted card (lsblk), for dev_guard
CARD_DISK=    # the booted card (/dev/mmcblkN), an SD card by its sysfs type
# dev_guard DEV LABEL: DEV may be written only if it is not the eMMC, is a
# partition of CARD_DISK, is not one of NEVER_LABELS, and carries LABEL in
# PART_TABLE. Called where a device is chosen and again in front of every dd
# that writes one; it is pure (the table, the card and the patterns), so the
# self-test runs it on its own.
dev_guard() {
    local dev=$1 want=$2 label l
    [[ $dev =~ $EMMC_RE ]] && die "REFUSED: $dev is the eMMC (mmcblk0): never written, its partitions carry the card's labels (#1281)"
    [[ $dev =~ $PART_RE ]] || die "REFUSED: $dev is not a partition of an mmcblk device"
    { [ -n "$CARD_DISK" ] && [ "${dev%p*}" = "$CARD_DISK" ]; } ||
        die "REFUSED: $dev is not a partition of the SD card /boot is on (${CARD_DISK:-none found})"
    label=$(awk -v d="$dev" '$1 == d { print $2 }' <<<"$PART_TABLE" | head -1)
    for l in $NEVER_LABELS; do
        [ "$label" != "$l" ] || die "REFUSED: $dev is labelled $label: never written"
    done
    [ "$label" = "$want" ] || die "REFUSED: $dev is labelled '${label:-nothing}', not $want: not a device this runner writes"
}
where_env() {
    local m disk dtype
    bget m 'set -- $(findmnt -n -v -o SOURCE,FSTYPE -T /boot); d=$1; p=${d%p*}; echo "$d $2 $p $(cat /sys/block/${p##*/}/device/type 2>/dev/null)"; lsblk -rno PATH,PARTLABEL "$p"' \
        $'/dev/mmcblkDp8 ext4 /dev/mmcblkD SD\n/dev/mmcblkD \n/dev/mmcblkDp7 u-boot-env\n/dev/mmcblkDp8 bootfs' ||
        die "cannot read the mount of /boot and the card's partitions"
    read -r BOOT_DEV fstype disk dtype <<<"$(head -1 <<<"$m")"
    PART_TABLE=$(tail -n +2 <<<"$m")
    [ "$fstype" = ext4 ] || die "/boot is '$fstype' on $BOOT_DEV, not ext4 (U-Boot's ext4load reads bootfs)"
    [[ $disk =~ $EMMC_RE ]] && die "REFUSED: /boot is on $BOOT_DEV of the eMMC (mmcblk0): this runner writes the microSD the board booted from, never the eMMC"
    # the number alone does not name the eMMC (probe order can change it):
    # the MMC core's own word does, "SD" for a card, "MMC" for an eMMC
    [ "$dtype" = SD ] ||
        die "REFUSED: /boot is on $disk, whose /sys/block/${disk##*/}/device/type is '${dtype:-unreadable}', not SD: this runner writes the microSD the board booted from, never the eMMC"
    CARD_DISK=$disk
    [[ $BOOT_DEV =~ p([0-9]+)$ ]] || die "/boot's source '$BOOT_DEV' is not a partition"
    BOOT_PART=${BASH_REMATCH[1]}
    dev_guard "$BOOT_DEV" "$BOOT_LABEL"
    ENV_DEV=$(awk -v l="$ENV_LABEL" '$2 == l { print $1 }' <<<"$m" | head -1)
    [ -n "$ENV_DEV" ] || die "no partition labelled $ENV_LABEL on $disk (the card /boot is on)"
    [ "$ENV_DEV" != "$BOOT_DEV" ] || die "the $ENV_LABEL partition is /boot's: refusing"
    [ "${ENV_DEV%p*}" = "${BOOT_DEV%p*}" ] || die "REFUSED: $ENV_DEV is not on $disk, the card /boot is on"
    dev_guard "$ENV_DEV" "$ENV_LABEL"
}
# env_read FILE: the whole partition into FILE (the host), both copies checked
env_read() {
    local sz
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ ${SU}dd if=$ENV_DEV bs=64k status=none   # > $1 (the whole $ENV_LABEL partition)"
        return 0
    fi
    mp2_ssh 120 "${SU}dd if=$ENV_DEV bs=64k status=none" > "$1" ||
        die "reading $ENV_DEV failed"
    sz=$(stat -c %s "$1")
    [ "$sz" -ge "$ENV_HI" ] || die "$ENV_DEV is $sz bytes, shorter than the environment's end $ENV_HI (MP2_ENV_OFFSET*/SIZE)"
    env_tool check "$1" > "$TMP/env.check" 2>&1 ||
        die "the environment at $ENV_OFF/$ENV_OFF2 (size $ENV_SIZE) of $ENV_DEV does not check: $(tr '\n' ';' < "$TMP/env.check")"
}
# env_push FILE: the copies' range of FILE onto the partition (every write of
# it goes through here, run by fgw)
env_push() {
    tail -c +$((ENV_LO + 1)) "$1" | head -c $((ENV_HI - ENV_LO)) |
        timeout 120 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}dd of=$ENV_DEV bs=512 seek=$((ENV_LO / 512)) conv=fsync status=none"
}
# env_write FILE WHAT: the copies' range of FILE onto the partition, read back
# equal, then WHAT's variables shown by the board's fw_printenv (NAME=VALUE
# lines in $TMP/expect); on any mismatch the pristine image goes back.
env_write() {
    local file=$1 what=$2 got want
    dev_guard "$ENV_DEV" "$ENV_LABEL"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ tail -c +$((ENV_LO + 1)) $file | head -c $((ENV_HI - ENV_LO)) | ssh BOARD '${SU}dd of=$ENV_DEV bs=512 seek=$((ENV_LO / 512)) conv=fsync status=none'"
        echo "DRY board\$ ${SU}dd if=$ENV_DEV bs=64k status=none | sha256sum   # must equal the host image"
        echo "DRY board\$ fw_printenv $(sed 's/=.*//' "$TMP/expect" | paste -sd' ' -)   # must show the edit ($what)"
        return 0
    fi
    ENV_PUSHING=1
    fgw env_push "$file" ||
        { ENV_PUSHING=0; env_restore; die "writing $ENV_DEV failed; the pristine image was written back" 5; }
    ENV_PUSHING=0
    bget got "${SU}dd if=$ENV_DEV bs=64k status=none | sha256sum" || got=
    got=${got%% *}
    want=$(sha256sum "$file" | cut -d' ' -f1)
    [ "$got" = "$want" ] || { env_restore; die "$ENV_DEV reads back as '$got', not $want; the pristine image was written back" 5; }
    bget got "${SU}fw_printenv $(sed 's/=.*//' "$TMP/expect" | paste -sd' ' -) 2>&1" || true
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
    dev_guard "$ENV_DEV" "$ENV_LABEL"
    local r=0
    ENV_PUSHING=1
    fgw env_push "$PRISTINE" || r=$?
    ENV_PUSHING=0
    [ "$r" = 0 ] ||
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
    if ! grep -q '^yako_' <<<"$out"; then
        INSTALLED=absent
        [ "${bc#"$HOOK_PREFIX"}" = "$bc" ] || INSTALLED="stale: bootcmd begins with the hook but no yako_* variable is set"
        return 0
    fi
    [ "$(sed -n 's/^yako_hook=//p' <<<"$out")" = "$YAKO_HOOK" ] || { INSTALLED="stale: yako_hook is another version's"; return 0; }
    [ "$(sed -n 's/^yako_mmc=//p' <<<"$out")" = "$YAKO_MMC" ] || { INSTALLED="stale: yako_mmc is another version's"; return 0; }
    [ "$(sed -n 's/^yako_addr=//p' <<<"$out")" = "$YAKO_ADDR" ] || { INSTALLED="stale: yako_addr is not $YAKO_ADDR"; return 0; }
    [ -n "$(sed -n 's/^yako_part=//p' <<<"$out")" ] || { INSTALLED="stale: yako_part is absent"; return 0; }
    if [ "${bc#"$HOOK_PREFIX"}" = "$bc" ]; then
        BOOTCMD_BARE=$bc
        INSTALLED="needs a person: bootcmd does not begin with '$HOOK_PREFIX' (ST's variable; this runner never changes it)"
        return 0
    fi
    INSTALLED=ok
}
# needs_person_bootcmd hook|unhook: the stop when only bootcmd is in the way (rc 7)
needs_person_bootcmd() {
    local why
    case "$1" in
    hook) why="U-Boot runs the one-shot only if bootcmd begins with '$HOOK_PREFIX', and bootcmd is ST's variable, which this runner never changes. The hook's own variables are in place and nothing else was changed. A person who allows the change stops autoboot and types at U-Boot's prompt:
    $(bootcmd_line hook)
then 'install' again shows it; until then 'run' refuses." ;;
    unhook) why="bootcmd still begins with '$HOOK_PREFIX', and bootcmd is ST's variable, which this runner never changes. The one-shot is cleared; the hook's variables stay (inert without yako_once) so bootcmd still runs what it names. A person stops autoboot and types at U-Boot's prompt:
    $(bootcmd_line unhook)
then 'uninstall' again removes the rest." ;;
    esac
    echo "$TAG: NEEDS A PERSON: $why" | scrub >&2
    exit 7
}
# fw_get VAR NAME [DRY]: the variable's value, "" when it is not set; a board
# that does not answer is not "not set" (rc 5)
fw_get() {
    local __g
    bget __g "${SU}fw_printenv -n $2 2>/dev/null; echo \"fw_get:ok\"" "${3:-}
fw_get:ok" || true
    [ "${__g##*fw_get:ok}" = "" ] && [ "$__g" != "${__g%fw_get:ok}" ] ||
        die "cannot read $2 from the board (fw_printenv over ssh failed): not taken as unset" 5
    __g=${__g%fw_get:ok}
    printf -v "$1" '%s' "${__g%$'\n'}"
}

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
# (no run directory outside run/probe: install, uninstall and disarm keep no record)
meta() { [ "$DRY" = 1 ] || [ -z "$RD" ] || printf '%s=%s\n' "$1" "$2" >> "$RD/meta.txt"; }
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
    local dir=$1 name back t0 t1 after con bv res v rc cycle blind expect
    expect=$(mget expect_wdog "$dir")
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
            grep -qF 'mp2_hello: hanging on purpose' <<<"$p" && [ "$expect" = 1 ] && bv=hung
        elif grep -qF 'yakogl-mp2: go ' <<<"$p"; then bv=uboot_go
        else bv=not_started; fi
        local l
        l=$(grep -aoE '^[A-Za-z0-9_]+: (PASS|FAIL): ' <<<"$p" | tail -1)
        if [ -n "$l" ]; then
            case "$l" in *PASS*) res="pass:${l%%:*}" ;; *) res="fail:${l%%:*}" ;; esac
        fi
    fi
    if [ "$(mget rebooted "$dir")" = 0 ]; then v=not_rebooted; rc=4; after=absent
    elif [ "$back" != 1 ]; then v=not_back; rc=5
    elif [ "$blind" = 1 ]; then v=returned; rc=0
    elif [ "${res%%:*}" = fail ]; then v=failed; rc=4
    elif [ "$bv" = psci_returned ]; then v=psci_returned; rc=4
    elif [ "$expect" = 1 ] && [ "$bv" = hung ]; then v=watchdog_reset; rc=0
    elif [ "$expect" = 1 ] && [ "$bv" = complete ]; then v=psci_reset_before_wdog; rc=4
    elif [ "$expect" = 1 ] && [ "$bv" = image_started ]; then v=no_hang; rc=4
    elif [ "$bv" = complete ]; then v=done; rc=0
    else v=no_end_line; rc=4; fi
    [ "$cycle" = 6 ] && [ "$rc" = 0 ] && rc=6
    local kind=row
    [ "$blind" = 1 ] && kind=blind
    V_LINE="$TAG $name rc=$rc verdict=$v boot=$BOOT $kind boot_verdict=$bv result=$res back_after=$after"
    V_RC=$rc
}
# the line after a not_back verdict (run and tally): what the plan asks of a
# board that did not come back
needs_person() {
    local dir=$1 bound
    [ "$V_RC" = 5 ] || return 0
    bound=$(mget wait_bound "$dir")
    bound=${bound:+$bound s}
    echo "$TAG: NEEDS A PERSON: the board did not answer ssh from a new boot within ${bound:-the wait} after the reboot (the image $(mget name "$dir"), its staged copy left on the card). Nothing resets it from outside and this runner never retries: release the board, report it, and let no other item take it until its owner says it is back."
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
            plain "$dir/console.txt" | grep -aE '^(yakogl-mp2: |MP2:|mp2: |mp2-handover: |[A-Za-z0-9_]+: (PASS|FAIL): |mp2_hello: hanging on purpose|\*\*\* Booting Zephyr)' | sed 's/^/  /'
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
    needs_person "$RD"
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
    # on_exit does what the lock's own trap did (the watchdog ended, the
    # record emptied, a watchdog's lines shown), after the undo
    LOCKED=1
    LEFT=${BOARD_LOCK%.lock}.mp2-left
    trap on_exit EXIT
}
# lock_path: LEFT without taking the lock (status reads the record only)
lock_path() {
    [ "$DRY" = 0 ] || return 0
    # shellcheck source=/dev/null
    BOARD_LOCK_NO_ACQUIRE=1 . "$REPO/driver/tools/kexec-zephyr/board_lock.sh" || return 0
    LEFT=${BOARD_LOCK%.lock}.mp2-left
}
# ---- what a run would leave behind, recorded before each change -------------------
LOCKED=0
LEFT=          # <lock>.mp2-left
RUN_OWNS_LEFT=0
left_write() { # left_write LINE...: written before the change it names
    meta left "$*"
    [ "$DRY" = 0 ] && [ -n "$LEFT" ] || return 0
    { echo "when=$(date '+%F %T')"; echo "run=${RD:-none}"; printf 'left=%s\n' "$@"; } > "$LEFT.new" && mv "$LEFT.new" "$LEFT"
}
left_clear() {
    [ "$DRY" = 0 ] && [ -n "$LEFT" ] || return 0
    rm -f "$LEFT"
}
left_show() { # an earlier run's record, if any (to stderr)
    [ "$DRY" = 0 ] && [ -n "$LEFT" ] && [ -s "$LEFT" ] || return 0
    { echo "$TAG: an earlier run left this record ($LEFT):"; sed 's/^/  /' "$LEFT"; } | scrub >&2
}
# on_exit: every verb ends here (the EXIT trap; a signal arrives through exit)
on_exit() {
    local rc=$?
    # a second signal does not cut the undo short (the watchdog's KILL still does)
    trap '' TERM INT HUP
    kill_fg
    stop_cap
    if [ "$VERB" = run ] && [ "$DRY" = 0 ]; then run_undo; fi
    if [ "$LOCKED" = 1 ] && [ "$DRY" = 0 ]; then
        [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null
        : > "$BOARD_LOCK"
        [ -n "${_wd_log:-}" ] && [ -s "$_wd_log" ] && cat "$_wd_log" >&2
    fi
    rm -rf "$TMP"
    exit "$rc"
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
# hold_result_seen: the image's "<prog>: PASS|FAIL: " line is on the console
# so far (console.txt for a device, console.ts for a bridge; either may carry
# a timestamp before the line)
hold_result_seen() {
    local f
    for f in "$RD/console.txt" "$RD/console.ts"; do
        [ -f "$f" ] && tr -d '\0\r' < "$f" | grep -aqE '(^|[0-9. ])[A-Za-z0-9_]+: (PASS|FAIL): ' && return 0
    done
    return 1
}
capture_start() {
    if uart_is_bridge; then
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ python3 tools/am62p_netboot_uart.py capture <BOARD_UART> <rundir>/console.raw <rundir>/console.ts &"
            return 0
        fi
        python3 "$HERE/am62p_netboot_uart.py" capture "$BOARD_UART" "$RD/console.raw" "$RD/console.ts" 2> >(scrub >&2) &
        CAP=$!
        fgw sleep 3
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
    fgw sleep 1
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
    [ "${INSTALLED#needs a person}" = "$INSTALLED" ] || echo "  a person types at U-Boot's prompt: $(bootcmd_line hook)"
    echo "env partition: $ENV_DEV (copies at $ENV_OFF${ENV_OFF2:+, $ENV_OFF2}, size $ENV_SIZE); bootfs: $BOOT_DEV (U-Boot part $BOOT_PART, card $CARD_DISK)"
    lock_path
    if [ -n "$LEFT" ] && [ -s "$LEFT" ]; then left_show 2>&1; else echo "left behind: nothing recorded"; fi
    if [ "$DRY" = 1 ]; then
        b "cd /boot && ${SU}sha256sum -c $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt --quiet   # the stock boot files against install's record"
    else
        local chk
        bget chk "test -f $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt && cd /boot && ${SU}sha256sum -c $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt --quiet 2>&1 && echo 'stock boot files: unchanged since install'" || true
        echo "${chk:-stock boot files: no record (not installed)}"
    fi
}
STAMP=$(date +%Y%m%d-%H%M%S)
backup_dir() { echo "${MP2_BACKUP_DIR:-$REPO/records/stm32mp257f-dk/uboot-env}"; }
cmd_install() {
    # with an image: checked first, as run checks it; a refusal changes nothing
    [ $# -eq 0 ] || check_image "$1"
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
        echo "DRY host\$ python3 tools/uboot_env.py ... set <env.bin> <env.new> 'yako_hook=$YAKO_HOOK' 'yako_mmc=$YAKO_MMC' yako_addr=$YAKO_ADDR yako_part=$BOOT_PART 'yako_bootcmd_prev=<bootcmd without the hook>'   # bootcmd is ST's: never written"
        printf 'yako_addr=%s\nyako_part=%s\n' "$YAKO_ADDR" "$BOOT_PART" > "$TMP/expect"
        b "${SU}mkdir -p $STAGE_DIR/$BACKUP_SUB && ${SU}cp <env.bin> $STAGE_DIR/$BACKUP_SUB/u-boot-env.bin && cd /boot && find . -path ./yakogl -prune -o -type f -print0 | sort -z | xargs -0 ${SU}sha256sum > $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt"
        env_write "<env.new>" "install"
        note "install (dry run): hook state read as '$st'; nothing was changed"
        return 0
    fi
    env_hook "$TMP/env.bin" "$TMP/env.new" "$BOOT_PART"
    if [ "$EDIT_HOW" = unchanged ]; then
        [ "$HOOKED" = 1 ] || needs_person_bootcmd hook
        note "installed already, byte for byte; nothing to do"
        return 0
    fi
    mkdir -p "$bdir" || die "cannot create $bdir"
    cp "$TMP/env.bin" "$PRISTINE" || die "cannot keep the pristine image at $PRISTINE"
    chmod 600 "$PRISTINE"
    local first
    bget first "${SU}mkdir -p $STAGE_DIR/$BACKUP_SUB && if test -e $STAGE_DIR/$BACKUP_SUB/u-boot-env.bin; then echo kept; else echo first; fi" ||
        die "cannot create $STAGE_DIR/$BACKUP_SUB on the card"
    if [ "$first" = first ]; then
        fgw timeout 120 ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}sh -c 'cat > $STAGE_DIR/$BACKUP_SUB/u-boot-env.bin && sync'" < "$TMP/env.bin" ||
            die "cannot keep the pristine image on the card"
        b "cd /boot && find . -path ./yakogl -prune -o -type f -print0 | sort -z | xargs -0 ${SU}sha256sum > $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt && sync" ||
            die "cannot record the stock boot files' sha256"
    else
        note "the card keeps the first install's pristine image and stock-file record ($STAGE_DIR/$BACKUP_SUB)"
    fi
    printf 'yako_hook=%s\nyako_mmc=%s\nyako_addr=%s\nyako_part=%s\nyako_bootcmd_prev=%s\n' "$YAKO_HOOK" "$YAKO_MMC" "$YAKO_ADDR" "$BOOT_PART" "$BOOTCMD_BARE" > "$TMP/expect"
    env_write "$TMP/env.new" "install"
    [ "$HOOKED" = 1 ] || needs_person_bootcmd hook
    note "$EDIT_HOW: U-Boot runs yako_hook before '$BOOTCMD_BARE'; the one-shot loads from mmc \${boot_instance}:$BOOT_PART. bootcmd was not written. Nothing was rebooted. Pristine image: $PRISTINE"
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
    # the undo, before anything changes
    echo "$TAG: UNDO, should the uninstall go wrong (the environment image as it is now is kept as $PRISTINE):"
    echo "  host\$ tail -c +$((ENV_LO + 1)) $PRISTINE | head -c $((ENV_HI - ENV_LO)) | ssh <BOARD_HOST> '${SU}dd of=$ENV_DEV bs=512 seek=$((ENV_LO / 512)) conv=fsync'"
    echo "  (or: tools/board-stm32mp2.sh install)"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ mkdir -p $bdir && cp <env.bin> $PRISTINE"
        echo "DRY host\$ python3 tools/uboot_env.py ... unset <env.bin> <env.new> $OUR_VARS   # bootcmd is ST's: never written; while it begins with the hook, only yako_once yako_file"
        : > "$TMP/expect"
        env_write "<env.new>" "uninstall"
        b "cd /boot && ${SU}sha256sum -c $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt --quiet   # the stock boot files as install found them, before the record goes"
        b "${SU}rm -rf $STAGE_DIR"
        return 0
    fi
    mkdir -p "$bdir" && cp "$TMP/env.bin" "$PRISTINE" && chmod 600 "$PRISTINE" || die "cannot keep the image before the uninstall at $PRISTINE"
    env_unhook "$TMP/env.bin" "$TMP/env.new"
    : > "$TMP/expect"
    if cmp -s "$TMP/env.bin" "$TMP/env.new"; then
        note "the environment holds nothing to remove; not written"
    else
        env_write "$TMP/env.new" "uninstall"
    fi
    if [ "$HOOKED" = 1 ]; then
        b "${SU}rm -f $STAGE_DIR/once-*.bin $STAGE_DIR/once-*.bin.part" || true
        needs_person_bootcmd unhook
    fi
    local left
    fw_get left yako_hook
    [ -z "$left" ] || die "yako_hook is still set after the uninstall" 5
    # the stock boot files as install recorded them, before the record goes
    local chk
    bget chk "if test -f $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt; then cd /boot && ${SU}sha256sum -c $STAGE_DIR/$BACKUP_SUB/boot-sha256.txt --quiet 2>&1 && echo unchanged; else echo norecord; fi" || true
    case "$chk" in
    unchanged) note "stock boot files: unchanged since install" ;;
    norecord) note "stock boot files: no record on the card (nothing to compare)" ;;
    *) die "the stock boot files differ from install's record: $STAGE_DIR is kept for a person to look at ($(tr '\n' ';' <<<"$chk"))" 5 ;;
    esac
    b "${SU}rm -rf $STAGE_DIR"
    note "$EDIT_HOW: no yako_* variable, bootcmd '$BOOTCMD_BARE' (not written), $STAGE_DIR removed. Nothing was rebooted."
}
# disarm: the undo of whatever a run left (yako_once, yako_file, staged
# images, a cut-off copy), from the board's own state; safe to repeat
cmd_disarm() {
    need_board
    env_range
    take_lock
    where_env
    left_show
    local once file rest
    fw_get once yako_once
    fw_get file yako_file
    if [ -n "$once$file" ] || [ "$DRY" = 1 ]; then
        echo "$TAG: disarming: yako_once='$once' yako_file='$file' (the runner's own variables; nothing else is written)"
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
    b "${SU}rm -f $STAGE_DIR/once-*.bin $STAGE_DIR/once-*.bin.part" || die "cannot remove the staged images"
    bget rest "ls $STAGE_DIR/once-* 2>/dev/null   # must print nothing" '' || true
    [ -z "$rest" ] || die "staged images are still on the card: $rest" 5
    left_clear
    note "disarmed: no one-shot pending, no staged image, no record left"
}

# ---- probe / run -------------------------------------------------------------------
ARMED=0         # 1: the one-shot is set and the board's Linux has not gone away
STAGED=         # the staged image (and its .part) while it is on the card
STAGED_OURS=0   # 1: the board has not booted since it was staged (on_exit removes it)
REBOOT_SENT=0
WENT=0          # 1: the board was seen to leave its Linux (silent, or a new boot)
BACK=0          # 1: the board's Linux answered after the reboot (or never left)
TOKEN=
# undo_arm: the pre-arm environment back and the one-shot read back unset (in
# a subshell: a refusal ends it, not the caller)
undo_arm() {
    [ -f "$TMP/env.prearm" ] || exit 1
    : > "$TMP/expect"
    env_write "$TMP/env.prearm" "disarm"
    local o
    fw_get o yako_once
    [ -z "$o" ]
}
# remove_staged: the staged image (and a cut-off copy) off the card, read back
remove_staged() {
    [ -n "$STAGED" ] || return 0
    b "${SU}rm -f $STAGED $STAGED.part && ! test -e $STAGED && ! test -e $STAGED.part" || return 1
    STAGED=
}
# run_undo (on_exit, a real run): what the run still owns goes back; what it
# cannot undo is written to the state file and named
run_undo() {
    local left=()
    if [ "$ARMED" = 1 ]; then
        note "ending while the one-shot is still the runner's: writing the pre-arm environment back" >&2
        if ( undo_arm ); then ARMED=0; meta disarmed "on exit"; else left+=("the one-shot may still be armed (yako_once=${TOKEN:-?})"); fi
    fi
    if [ -n "$STAGED" ]; then
        if [ "$STAGED_OURS" = 1 ] && remove_staged; then
            meta cleanup "removed on exit"
        else
            left+=("$STAGED (or its .part) on the card")
        fi
    fi
    [ "$WENT" = 1 ] && [ "$BACK" = 0 ] &&
        left+=("the board was rebooted and is not back: U-Boot clears yako_once before it loads; 'disarm' clears it if it is still set")
    [ "$RUN_OWNS_LEFT" = 1 ] || { [ ${#left[@]} = 0 ] || echo "$TAG: WARNING: left behind: ${left[*]}" | scrub >&2; return 0; }
    if [ ${#left[@]} = 0 ]; then
        left_clear
        return 0
    fi
    left_write "${left[@]}"
    { printf '%s: WARNING: left behind: %s\n' "$TAG" "${left[@]}"
      echo "$TAG: the undo, once the board answers: tools/board-stm32mp2.sh disarm (record: ${LEFT:-none})"; } | scrub >&2
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
    [ "$EXPECT_WDOG" = 1 ] && meta expect_wdog 1
    note "$NAME: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown) boot $BOOT image sha256 ${IMG_SHA:0:16} footprint ${footprint:-none}"
    [ "$BLIND" = 1 ] || console_check
    take_lock
    # after the lock: on_exit (the EXIT trap again) does the lock's own
    # cleanup after run_undo
    bget st 'systemctl is-system-running --wait >/dev/null 2>&1; cut -d" " -f1 /proc/uptime' 100 || die "the board's Linux does not answer (ssh needs the board's host key known beforehand: it is never recorded)"
    left_show
    common_prechecks
    where_env
    installed
    st=$INSTALLED
    [ "${st#needs a person}" = "$st" ] || needs_person_bootcmd hook
    [ "$st" = ok ] || die "the U-Boot hook is not installed as this script writes it ($st): 'install' first"
    fw_get once yako_once
    [ -z "$once" ] || die "a one-shot is already pending (yako_once=$once); find its owner, or 'disarm'"
    bget left "ls $STAGE_DIR/once-* 2>/dev/null; true   # must print nothing" '' || die "cannot list $STAGE_DIR on the board"
    [ -z "$left" ] || die "$STAGE_DIR holds a one-shot image (or a cut-off copy) of a run that did not clean up ($left): look at it, or 'disarm'"
    # the board shows nothing left: from here this run owns the state file
    if [ -n "$LEFT" ] && [ -s "$LEFT" ]; then note "the board shows none of that record's state: it is cleared" >&2; fi
    RUN_OWNS_LEFT=1
    left_clear
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
    TOKEN=$token
    file="once-$token.bin"
    meta token "$token"
    meta wait_bound "$WAIT_BOUND"
    # the undo, before anything changes
    echo "$TAG: UNDO, should this run be cut off (its exit does it itself while the board's Linux answers; what it could not do is recorded in ${LEFT:-<lock>.mp2-left}):"
    echo "  tools/board-stm32mp2.sh disarm   # unsets yako_once and yako_file, removes $STAGE_DIR/once-*.bin and once-*.bin.part"
    STAGED=$STAGE_DIR/$file
    STAGED_OURS=1
    left_write "staging $STAGED"
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ ${SU}sh -c 'mkdir -p $STAGE_DIR && cat > $STAGE_DIR/$file.part && sync && mv $STAGE_DIR/$file.part $STAGE_DIR/$file && sync' < $IMG"
    else
        fgw timeout "$STAGE_TIMEOUT" ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "${SU}sh -c 'mkdir -p $STAGE_DIR && cat > $STAGE_DIR/$file.part && sync && mv $STAGE_DIR/$file.part $STAGE_DIR/$file && sync'" < "$IMG" ||
            die "copying the image to $STAGE_DIR/$file failed"
    fi
    bget st "echo 3 | ${SU}tee /proc/sys/vm/drop_caches >/dev/null && sha256sum $STAGE_DIR/$file" "$IMG_SHA  $STAGE_DIR/$file" || st=
    st=${st%% *}
    [ "$st" = "$IMG_SHA" ] || die "the card's copy $STAGE_DIR/$file reads back as '$st', not $IMG_SHA (removed on exit)"
    meta staged "$STAGE_DIR/$file on bootfs, read back equal"

    # 4. the capture, before anything can print
    if [ "$BLIND" = 0 ]; then capture_start || die "the console capture ended at once (bridge busy or unreachable; no reboot)"; fi

    # 5. arm: one environment write, read back by fw_printenv; the image read
    # just before is the undo (on_exit writes it back while ARMED is 1)
    env_read "$TMP/env.bin"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ python3 tools/uboot_env.py ... set <env.bin> <env.new> yako_once=$token yako_file=/yakogl/$file"
    else
        cp "$TMP/env.bin" "$TMP/env.prearm"
        env_arm "$TMP/env.bin" "$TMP/env.new" "$token" "/yakogl/$file"
        left_write "armed yako_once=$token" "$STAGED on the card"
        ARMED=1
    fi
    printf 'yako_once=%s\nyako_file=/yakogl/%s\n' "$token" "$file" > "$TMP/expect"
    if [ "$DRY" = 1 ]; then env_write "<env.new>" "arm"; else env_write "$TMP/env.new" "arm"; fi
    meta t_arm "$(date +%s)"

    # 6. reboot; the ssh session ends with it. The one-shot stays the
    # runner's (ARMED) until the board is seen to leave its Linux: a reboot
    # that did not happen is disarmed, not reported as a board that is gone.
    left_write "rebooting with yako_once=$token armed (U-Boot clears it before it loads)" "$STAGED on the card"
    REBOOT_SENT=1
    b "sync; ${SU}systemctl reboot" || true
    T_REBOOT=$(date +%s)
    meta t_reboot "$T_REBOOT"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ wait until ssh answers from a new boot (uptime < time since the reboot), <= $WAIT_BOUND s; the old boot still answering after min($REBOOT_GRACE, $WAIT_BOUND) s: not_rebooted, disarmed"
        b "${SU}fw_printenv -n yako_once   # must print nothing"
        b "${SU}rm -f $STAGE_DIR/$file"
        echo "DRY host\$ the verdict line from the run directory"
        exit 0
    fi
    HELD=0
    local down=0
    while :; do
        fgw sleep 5
        if [ -n "$HOLD_S" ] && [ "$HELD" = 0 ] && hold_result_seen; then
            HELD=1
            echo "hold: result seen at $(date -u +%Y-%m-%dT%H:%M:%SZ), holding ${HOLD_S}s"
            meta hold_s "$HOLD_S"
            fgw sleep "$HOLD_S"
        fi
        el=$(($(date +%s) - T_REBOOT))
        fgw timeout 8 ssh "${SSH_OPTS[@]}" -o ConnectTimeout=5 "$BOARD_HOST" 'cut -d. -f1 /proc/uptime' > "$TMP/up" 2>/dev/null
        up=$(cat "$TMP/up")
        if [[ $up =~ ^[0-9]+$ ]]; then
            if [ "$up" -lt "$el" ]; then WENT=1; ARMED=0; STAGED_OURS=0; break; fi
            # the boot before the reboot still answers
            if [ "$down" = 0 ] && { [ "$el" -gt "$REBOOT_GRACE" ] || [ "$el" -gt "$WAIT_BOUND" ]; }; then rcc=not_rebooted; break; fi
        else
            up=
            if [ "$down" = 0 ]; then
                # it went away: the one-shot is U-Boot's now
                down=1; WENT=1; ARMED=0; STAGED_OURS=0
                meta t_down "$(date +%s)"
            fi
        fi
        if [ "$el" -gt "$WAIT_BOUND" ]; then rcc=5; break; fi
    done
    [ -n "$up" ] && meta up "$up"
    meta t_back "$(date +%s)"

    # 7. the one-shot must be gone; the staged image removed; the verdict
    if [ "$rcc" = not_rebooted ]; then
        BACK=1
        meta back 1
        meta rebooted 0
        note "the board's Linux still answered from the boot before the reboot ${el}s after it (uptime ${up}s): the reboot did not happen; writing the pre-arm environment back" >&2
        if ( undo_arm ); then ARMED=0; meta disarmed "not rebooted"; else note "WARNING: the disarm did not read back" >&2; fi
        STAGED_OURS=1
        if remove_staged; then meta cleanup "removed (not rebooted)"; else meta cleanup "NOT removed"; fi
        rcc=
    elif [ "$rcc" != 5 ]; then
        BACK=1
        meta back 1
        local once_left=
        if ( fw_get once_left yako_once; printf '%s' "$once_left" > "$TMP/once.left" ); then
            once_left=$(cat "$TMP/once.left")
        else
            once_left="unreadable"
        fi
        if [ -n "$once_left" ]; then
            note "yako_once is '$once_left' after the cycle: disarming" >&2
            if ( env_read "$TMP/env.x" && env_disarm "$TMP/env.x" "$TMP/env.y" && : > "$TMP/expect" && env_write "$TMP/env.y" "disarm" ); then
                meta disarmed "after the cycle"
            else
                note "WARNING: the disarm after the cycle failed" >&2
                ARMED=1   # on_exit tries once more (the pre-arm image) and records what is left
            fi
            rcc=6
        fi
        if remove_staged; then
            meta cleanup "$STAGE_DIR/$file removed (read back)"
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
    hook)
        env_hook "$in" "$out" "${4:-8}"
        if [ "$HOOKED" = 1 ]; then echo "$EDIT_HOW"; else echo "$EDIT_HOW; bootcmd not hooked: $(bootcmd_line hook)"; fi ;;
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
    needs_person "$dir"
    exit "$V_RC"
}

case "$VERB" in
selftest) exec bash "$HERE/board_stm32mp2_selftest.sh" ;;
status) cmd_status "$@" ;;
install) cmd_install "$@" ;;
uninstall) cmd_uninstall "$@" ;;
disarm) cmd_disarm "$@" ;;
probe | run) cmd_run "$@" ;;
env-edit) cmd_env_edit "$@" ;;
tally) cmd_tally "$@" ;;
*) usage ;;
esac
