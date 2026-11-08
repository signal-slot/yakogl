#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board-stm32mp2.sh - the STM32MP257F-DK (ST's Linux on the microSD, ST's
# U-Boot 2023.10) as a board of tools/board.sh: the one-shot boot of a Zephyr
# A35 image from the board's own U-Boot off the microSD, the install that
# makes it possible, its exact undo, the run and its verdict. A library
# (board.sh sources it and calls the stm32mp2_* functions) that also runs
# standalone:
#
#   tools/board-stm32mp2.sh [--dry-run] install [CONF...]
#   tools/board-stm32mp2.sh [--dry-run] uninstall
#   tools/board-stm32mp2.sh [--dry-run] run <image> <logdir> [name]
#   tools/board-stm32mp2.sh [--dry-run] status
#   tools/board-stm32mp2.sh verdict <rundir>
#
# The board (issue #1281; docs/notes/pre-study-stm32mp257f-dk.md section 1
# "Boot chain" and "U-Boot's flow"): TF-A, OP-TEE, then ST's U-Boot, whose
# distro loop finds an extlinux configuration under /boot (the card's bootfs,
# a 64 MiB ext4 partition of the microSD, Linux's mmcblk1) and shows its menu
# ("Select the boot mode", "N:<tab>LABEL" lines, then "Enter choice: " with the
# file's TIMEOUT) before it boots the DEFAULT label, Linux, with booti. The
# board has fw_printenv but no fw_setenv, no fw_env.config, no remote reset,
# and the only way back from a boot that is not Linux is the board itself.
#
# The one-shot, in three parts, none of which writes the U-Boot environment
# or anything outside /boot:
#   install    appends one extra label, "yakogl" (KERNEL /yakogl/once.bin,
#              the DEFAULT label's FDT line), to every extlinux configuration
#              U-Boot could read (/boot/mmc<n>_extlinux/*.conf: ST's U-Boot
#              names the directory after its own mmc device number and the
#              file after the board or "extlinux", so every one gets it). The
#              DEFAULT, the TIMEOUT and every other byte stay as they are:
#              a boot that nobody types at is Linux, exactly as before. The
#              stock file is saved first, byte for byte, as
#              /boot/yakogl/stock/<dir>__<file> (its sha256 in
#              /boot/yakogl/stock/SHA256SUMS) and once more on this host
#              (STM32MP2_BACKUP_DIR, default records/stm32mp257f-dk/extlinux
#              of the main checkout). The undo is printed before the first
#              change.
#   run        stages the image as /boot/yakogl/once.bin (read back by sha256
#              after the page cache is dropped), opens the console, reboots
#              Linux, and when U-Boot's menu lists "N:<tab>yakogl" and asks
#              "Enter choice: ", types N once. U-Boot loads the image and
#              starts it with booti. The next boot, after the image's PSCI
#              SYSTEM_RESET (the board layer: driver/zephyr/lib/board/
#              stm32mp2) or the OP-TEE watchdog, gets no key and is Linux.
#              Nothing is typed before the prompt (a key during "Hit any key
#              to stop autoboot" would leave the board at U-Boot's prompt),
#              never twice, and never later than STM32MP2_ARM_S after the
#              start. A choice that is missed is a boot of Linux: the run
#              reports it, the board is back. The staged image is removed
#              once Linux is back.
#   uninstall  puts the stock files back from the saved copies (sha256 against
#              SHA256SUMS before and after) and removes /boot/yakogl.
#
# booti and the image's header. Zephyr's AArch64 image header has text_offset
# 0 and flag bit 3 (position independent) clear (zephyr/arch/arm64/core/
# header.S), and U-Boot's booti_setup then moves such an image to the start
# of DRAM (0x80000000 on this part, below System RAM) and starts it there,
# while the stm32mp257f_dk board links it at 0xa0000000
# (driver/boards/st/stm32mp257f_dk, zephyr,sram). So the staged copy is the
# image with text_offset = 0xa0000000 - 0x80000000 = 0x20000000: booti moves it
# to the link address and enters it there at EL2 with x0 the DTB. An image
# whose header already says that is staged as it is; one with any other
# text_offset or with bit 3 set is refused (it would run elsewhere).
# meta.txt records both sha256s (image_sha256, staged_sha256).
#
# "run" does, in order, and stops at the first failure:
#   1. refuses without BOARD_HOST and BOARD_UART (a character device or
#      host:port of a serial bridge), checks the image: the AArch64 header,
#      the footprint inside the 256 MiB window at 0xa0000000, the watchdog
#      line "board: watchdog armed" (the stm32mp2 board layer's; the brief's
#      rule: an image without it is refused; NETBOOT_NO_WATCHDOG=1 overrides,
#      deliberately), and the header (above);
#   2. takes the host lock (driver/tools/kexec-zephyr/board_lock.sh when it
#      is there) and the board lease (/var/tmp/yakogl-board.lease, the one
#      tools/board.sh's boards use, so two runners never take the board at
#      once), checks ssh, the boot_id, that /boot is an ext4 partition of
#      mmcblk1 (never the eMMC), its free space (twice the image), that the
#      label is installed in every configuration, that no staged image is
#      left from an earlier run, and that nobody holds the console;
#   3. stages the image and reads it back;
#   4. starts the console client (capture to console.raw/console.ts, the
#      typer) and checks it is running;
#   5. reboots Linux ("sync; systemctl reboot"; root logs in on this board,
#      so no sudo; a BOARD_HOST that is not root gets "sudo -n");
#   6. waits for a NEW boot of Linux to answer ssh (its uptime shorter than
#      the time since the reboot), up to RUN_BOUND + RETURN_BOUND seconds
#      (defaults 120 + 120; the sum is never below 90: a reset from Linux is
#      back in about 35 s, the rest covers U-Boot, the menu and the image).
#      Not back in time: the run stops, the staged image stays, and it says
#      that the board needs a person. It never retries, and no other run
#      should take the board until the owner says it is back (exit 5);
#   7. removes the staged image (read back absent), stops the capture,
#      derives console.txt, prints the verdict line and writes INDEX.txt.
#
# The verdict line has the shape of the other boards':
#   board.sh stm32mp257f-dk <name> rc=<rc> verdict=<v> boot=uboot-extlinux row
#       boot_verdict=<bv> result=<...> resets=absent record=absent
#       back_after=<s>s handover=<the mp2-handover line's values|absent>
# boot_verdict, from the console: no_console (nothing captured), no_menu (no
# "Enter choice" seen), label_not_offered (a menu without our label: not
# installed where U-Boot reads), not_chosen (offered, typed, but U-Boot did
# not take it: Linux booted), load_failed (chosen, no "Starting kernel"),
# no_zephyr (started, no Zephyr banner), no_end (banner, then nothing more),
# reset_without_end (the banner, then a new boot with no end line: the
# watchdog or a silent fault), complete (the end line, then a new boot),
# looped (our label was taken twice: the one-shot did not hold), not_back
# (Linux did not answer in time). verdict=done needs complete, Linux back,
# and a result: the sample's own PASS line, or the board layer's end line
# "mp2: <why>: PSCI SYSTEM_RESET" for a <why> that is neither a fatal error
# nor "lifetime over" (those are failed and lifetime_end). A sample's FAIL line
# is failed. Exit codes: 0 done; 2 usage; 3 refused before the board changed;
# 4 the verdict is not done; 5 the board did not come back (a person).
#
# Environment (no default for a machine, an address or a path of one):
#   BOARD_HOST         [user@]host of the board's Linux (the lease's; root)
#   BOARD_UART         the console: a character device on this host, or
#                      host:port of a serial bridge (a raw TCP stream, one
#                      client at a time); required by run, never written to
#                      a file
#   RUN_BOUND, RETURN_BOUND  seconds (above)
#   STM32MP2_ARM_S     the typer types nothing later than this after the
#                      start (default 180)
#   STM32MP2_BACKUP_DIR  where install keeps its copies of the stock files
#   BOARD_END_RE       extended regex of the image's last line (default: the
#                      board layer's PSCI reset line and a fatal error)
#   NETBOOT_NO_WATCHDOG  1: run an image without the watchdog line
#   LOCK_WAIT, HOLD_MAX  the lease (board_lock.sh's words)
# --dry-run prints every command that would touch the board or the console
# as "DRY board$ ..." / "DRY host$ ..." and runs none of them (reads
# included); the image is still checked on the host.
# tools/board_stm32mp2_selftest.sh runs every verb this way and the real path
# against a fake board.

# ---- the board facts -----------------------------------------------------------
MP2_BOARD=stm32mp257f-dk
MP2_TAG="board.sh $MP2_BOARD"
MP2_BOOT=uboot-extlinux
MP2_LABEL=yakogl
MP2_DIR=/boot/yakogl
MP2_ONCE=$MP2_DIR/once.bin
MP2_STOCK=$MP2_DIR/stock
MP2_SUMS=$MP2_STOCK/SHA256SUMS
MP2_CONF_GLOB='/boot/mmc[0-9]_extlinux/*.conf'
MP2_BOOT_DEV_RE='^/dev/mmcblk1p[0-9]+$'   # the microSD's bootfs; the eMMC is another device
MP2_DRAM_BASE=0x80000000                  # booti's bi_dram[0].start on this part
MP2_YAKO_ADDR=0xa0000000                  # the link address (driver/boards/st/stm32mp257f_dk)
MP2_LOAD_END=0xb0000000                   # 256 MiB window (zephyr,sram)
MP2_TEXT_OFFSET=$((MP2_YAKO_ADDR - MP2_DRAM_BASE))
MP2_MARKER='board: watchdog armed'        # driver/zephyr/lib/board/common/board_internal.h BOARD_WDOG_ARMED_LINE
MP2_WATCHDOG="the OP-TEE SMC watchdog"
MP2_END_RE_DEFAULT='^mp2: .*: PSCI SYSTEM_RESET at [0-9]+ ms|ZEPHYR FATAL ERROR'
MP2_LEGACY_RE='^[A-Za-z0-9_]+: (PASS|FAIL)\b|^gl_test: all: |^deqp: summary: '
MP2_LEASE=/var/tmp/yakogl-board.lease
MP2_SSH_OPTS=(-o StrictHostKeyChecking=no -o ConnectTimeout=10 -o BatchMode=yes)

MP2_HERE=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
MP2_REPO=$(cd "$MP2_HERE/.." && pwd)
DRY=${DRY:-0}
MP2_TMP=
MP2_CAP=
MP2_LEASE_HELD=0
MP2_LEASE_ME="$(hostname -s 2>/dev/null || echo host):$$"
MP2_RD=
MP2_SU=

# ---- primitives -----------------------------------------------------------------
mp2_note() { echo "$MP2_TAG: $*"; }
mp2_die() { echo "$MP2_TAG: $1" >&2; exit "${2:-3}"; }

mp2_tmp() {
    [ -n "$MP2_TMP" ] && return 0
    MP2_TMP=$(mktemp -d)
}

# mp2_su: the prefix of a root command on the board: none when root logs in
mp2_su() {
    case "${BOARD_HOST:-}" in
    root@* | root) MP2_SU= ;;
    *) MP2_SU='sudo -n ' ;;
    esac
}

# mp2_b CMD: run CMD on the board, bounded; in --dry-run print it instead.
mp2_b() {
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*"; return 0; fi
    timeout 60 ssh "${MP2_SSH_OPTS[@]}" "$BOARD_HOST" "$@"
}
# mp2_bin CMD < FILE: CMD on the board with this stdin (an upload), longer bound
mp2_bin() {
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*   # stdin: the file"; return 0; fi
    timeout 900 ssh "${MP2_SSH_OPTS[@]}" "$BOARD_HOST" "$@"
}
# mp2_bget VAR CMD [DRY-VALUE]: a board read into VAR; printed in --dry-run
mp2_bget() {
    local __v=$1 __c=$2 __d=${3:-} __o __r=0
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ $__c"
        printf -v "$__v" '%s' "$__d"
        return 0
    fi
    __o=$(mp2_b "$__c") || __r=$?
    printf -v "$__v" '%s' "$__o"
    return "$__r"
}
# mp2_act CMD...: a host-side action with an effect; printed in --dry-run
mp2_act() {
    if [ "$DRY" = 1 ]; then echo "DRY host\$ $*"; return 0; fi
    "$@"
}

# mp2_uart_is_bridge: BOARD_UART is host:port of a serial bridge
mp2_uart_is_bridge() { [[ ${BOARD_UART:-} =~ ^[^/:]+:[0-9]+$ ]]; }

# mp2_need_board [uart]: BOARD_HOST (and BOARD_UART) from the environment
# only; --dry-run stands in placeholders. A BOARD_UART that is neither a
# character device nor host:port is refused before anything else.
mp2_need_board() {
    if [ "$DRY" = 1 ]; then
        BOARD_HOST=${BOARD_HOST:-root@BOARD}
        [ "${1:-}" = uart ] && BOARD_UART=${BOARD_UART:-/dev/UART}
    fi
    [ -n "${BOARD_HOST:-}" ] || mp2_die "REFUSED: set BOARD_HOST=[user@]host of the board's Linux (from the lease, never a file)"
    mp2_su
    [ "${1:-}" = uart ] || return 0
    [ -n "${BOARD_UART:-}" ] || mp2_die "REFUSED: set BOARD_UART=the console's character device on this host, or host:port of its serial bridge (from the lease, never a file)"
    if ! mp2_uart_is_bridge; then
        if [ "$DRY" = 1 ]; then
            [[ $BOARD_UART == /dev/* ]] || mp2_die "REFUSED: BOARD_UART='$BOARD_UART' is neither a character device nor host:port"
        else
            [ -c "$BOARD_UART" ] || mp2_die "REFUSED: BOARD_UART='$BOARD_UART' is neither a character device nor host:port"
        fi
    fi
}

# mp2_meta KEY VALUE: one measured fact of the run (meta.txt; the last wins)
mp2_meta() {
    [ "$DRY" = 1 ] && return 0
    [ -n "$MP2_RD" ] || return 0
    printf '%s=%s\n' "$1" "$2" >> "$MP2_RD/meta.txt"
}
mp2_mget() { sed -n "s/^$1=//p" "${2:-$MP2_RD}/meta.txt" 2>/dev/null | tail -1; }

# ---- the image (run, step 1) ----------------------------------------------------
# mp2_check_image IMG: the header, the footprint, the watchdog line; sets
# IMG_SHA IMG_SIZE IMG_TEXT_OFFSET IMG_FLAGS IMG_FOOTPRINT IMG_ZEPHYR, and
# STAGE_FILE (the booti form, in MP2_TMP) with STAGE_SHA.
mp2_check_image() {
    local img=$1 magic max flags toff footprint
    max=$((MP2_LOAD_END - MP2_YAKO_ADDR))
    [ -f "$img" ] || mp2_die "no image $img"
    magic=$(od -A n -t x1 -j 56 -N 4 "$img" | tr -d ' \n')
    [ "$magic" = 41524d64 ] || mp2_die "$img has no AArch64 Image header ('$magic' at byte 56); build with CONFIG_AARCH64_IMAGE_HEADER=y"
    toff=$(od -A n -t u8 -j 8 -N 8 "$img" | tr -d ' ')
    footprint=$(od -A n -t u8 -j 16 -N 8 "$img" | tr -d ' ')
    flags=$(od -A n -t u8 -j 24 -N 8 "$img" | tr -d ' ')
    { [ "$footprint" -gt 0 ] && [ "$footprint" -le "$max" ]; } ||
        mp2_die "image_size $footprint does not fit $MP2_YAKO_ADDR..$MP2_LOAD_END ($max bytes)"
    if ! grep -aqF "$MP2_MARKER" "$img"; then
        [ "${NETBOOT_NO_WATCHDOG:-0}" = 1 ] ||
            mp2_die "REFUSED: $img does not contain \"$MP2_MARKER\": it would not arm $MP2_WATCHDOG, and a hang would hold the board until a person resets it (NETBOOT_NO_WATCHDOG=1 overrides, deliberately)"
        mp2_note "WARNING: NETBOOT_NO_WATCHDOG=1: this image has no way back if it hangs" >&2
    fi
    IMG_SHA=$(sha256sum "$img" | cut -d' ' -f1)
    IMG_SIZE=$(stat -c %s "$img")
    IMG_TEXT_OFFSET=$toff; IMG_FLAGS=$flags; IMG_FOOTPRINT=$footprint
    IMG_ZEPHYR=$(grep -aoE 'Booting Zephyr OS build [0-9A-Za-z._+-]+' "$img" | head -1 | sed 's/.* build //')
    IMG_ZEPHYR=${IMG_ZEPHYR:-absent (no Zephyr banner in the image)}
    # the booti form (the file header above)
    mp2_tmp
    STAGE_FILE=$MP2_TMP/once.bin
    if [ $((flags & 8)) -ne 0 ]; then
        mp2_die "REFUSED: the image's header has flag bit 3 (position independent) set: booti would start it at kernel_addr_r, not at $MP2_YAKO_ADDR where it is linked"
    elif [ "$toff" = "$MP2_TEXT_OFFSET" ]; then
        cp "$img" "$STAGE_FILE"
        IMG_BOOTI="as built (text_offset $(printf '0x%x' "$toff"))"
    elif [ "$toff" = 0 ]; then
        { head -c 8 "$img"; printf '\x00\x00\x00\x20\x00\x00\x00\x00'; tail -c +17 "$img"; } > "$STAGE_FILE"
        IMG_BOOTI="text_offset 0 -> $(printf '0x%x' "$MP2_TEXT_OFFSET") (booti moves it from kernel_addr_r to $MP2_YAKO_ADDR)"
    else
        mp2_die "REFUSED: the image's header has text_offset $(printf '0x%x' "$toff"): booti would start it at $(printf '0x%x' $((MP2_DRAM_BASE + toff))), not at $MP2_YAKO_ADDR where it is linked"
    fi
    [ "$(stat -c %s "$STAGE_FILE")" = "$IMG_SIZE" ] || mp2_die "the booti form has another size than the image"
    STAGE_SHA=$(sha256sum "$STAGE_FILE" | cut -d' ' -f1)
}

# ---- the lease (tools/board.sh's, the same file and format) ---------------------
mp2_lease_try() {
    mp2_b "L=$MP2_LEASE; me='$MP2_LEASE_ME'; row='${MP2_NAME:-install}'; hold=${HOLD_MAX:-900}
now=\$(date +%s)
if ! mkdir \$L.d 2>/dev/null; then
    age=\$(( now - \$(stat -c %Y \$L.d 2>/dev/null || echo \$now) ))
    [ \$age -gt 60 ] && rmdir \$L.d 2>/dev/null
    mkdir \$L.d 2>/dev/null || { echo 'holder=(the lease is being written)'; exit 3; }
fi
rc=0
if [ -f \$L ]; then
    h=\$(sed -n 's/^holder=//p' \$L); e=\$(sed -n 's/^expires=//p' \$L)
    if [ \"\$h\" != \"\$me\" ] && [ \"\${e:-0}\" -gt \$now ]; then cat \$L; rc=3; fi
    [ \$rc = 0 ] && [ \"\$h\" != \"\$me\" ] && echo \"taken over: \$(tr '\\n' ' ' < \$L)\"
fi
if [ \$rc = 0 ]; then
    printf 'holder=%s\\nrow=%s\\nexpires=%s\\n' \"\$me\" \"\$row\" \$((now + hold)) > \$L.new && mv \$L.new \$L &&
        grep -qx \"holder=\$me\" \$L || rc=4
fi
rmdir \$L.d
exit \$rc"
}

mp2_lease_take() {
    local out rc deadline
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ . driver/tools/kexec-zephyr/board_lock.sh   # HOLD_MAX=${HOLD_MAX:-900}, one lock per BOARD_HOST"
        echo "DRY board\$ take $MP2_LEASE   # holder=<host>:<pid>, expires now + HOLD_MAX on the board's clock; wait while another holder's lease runs"
        return 0
    fi
    if [ -f "$MP2_REPO/driver/tools/kexec-zephyr/board_lock.sh" ]; then
        # shellcheck source=/dev/null
        . "$MP2_REPO/driver/tools/kexec-zephyr/board_lock.sh"
    fi
    deadline=$(( $(date +%s) + ${LOCK_WAIT:-2700} ))
    while :; do
        rc=0
        out=$(mp2_lease_try 2>&1) || rc=$?
        if [ "$rc" = 0 ]; then
            MP2_LEASE_HELD=1
            [ -n "$out" ] && mp2_note "board lease: $out"
            return 0
        fi
        [ "$(date +%s)" -lt "$deadline" ] ||
            mp2_die "the board is leased to another run and LOCK_WAIT ran out: $(tr '\n' ' ' <<<"$out")"
        if [ "$rc" = 3 ]; then
            mp2_note "board lease held by another run, waiting: $(tr '\n' ' ' <<<"$out")" >&2
        else
            mp2_note "board lease: the board did not answer (rc $rc), retrying" >&2
        fi
        sleep 20
    done
}

mp2_lease_release() {
    [ "$MP2_LEASE_HELD" = 1 ] || return 0
    mp2_b "grep -qx 'holder=$MP2_LEASE_ME' $MP2_LEASE 2>/dev/null && rm -f $MP2_LEASE" >/dev/null 2>&1
    MP2_LEASE_HELD=0
}

mp2_stop_cap() {
    if [ -n "$MP2_CAP" ]; then kill "$MP2_CAP" 2>/dev/null; wait "$MP2_CAP" 2>/dev/null; MP2_CAP=; fi
}

mp2_on_exit() {
    mp2_stop_cap
    mp2_lease_release
    [ -n "$MP2_TMP" ] && rm -rf "$MP2_TMP"
    [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null
    return 0
}

# ---- the board's view: /boot, the configurations, the install state -------------
# mp2_boot_where: /boot is an ext4 partition of the microSD (mmcblk1); sets
# BOOT_DEV and BOOT_FREE
mp2_boot_where() {
    local m
    mp2_bget m 'findmnt -n -o SOURCE,FSTYPE -T /boot' '/dev/mmcblk1p8 ext4' || mp2_die "cannot read the mount of /boot on the board"
    BOOT_DEV=${m%% *}
    [[ $BOOT_DEV =~ $MP2_BOOT_DEV_RE ]] || mp2_die "REFUSED: /boot is '$m', not an mmcblk1 partition (the microSD's bootfs): nothing else is ever written"
    [ "${m##* }" = ext4 ] || mp2_die "REFUSED: /boot is '$m', not ext4 (U-Boot reads the bootfs with its ext4 driver)"
    mp2_bget BOOT_FREE 'df -B1 --output=avail /boot | tail -1' 35000000 || mp2_die "df on the board failed"
    BOOT_FREE=${BOOT_FREE// /}
    [[ $BOOT_FREE =~ ^[0-9]+$ ]] || mp2_die "cannot read the free space of /boot ('$BOOT_FREE')"
}

# mp2_confs [CONF...]: the extlinux configurations, one per line (CONFS);
# the given ones, else every /boot/mmc<n>_extlinux/*.conf on the card
mp2_confs() {
    local c
    if [ $# -gt 0 ]; then
        for c in "$@"; do
            [[ $c == /boot/*.conf ]] || mp2_die "REFUSED: '$c' is not a /boot/...conf file (only extlinux configurations under /boot are touched)"
        done
        CONFS=$(printf '%s\n' "$@")
        return 0
    fi
    mp2_bget CONFS "ls $MP2_CONF_GLOB 2>/dev/null" $'/boot/mmc0_extlinux/extlinux.conf\n/boot/mmc0_extlinux/stm32mp257f-dk_extlinux.conf\n/boot/mmc1_extlinux/extlinux.conf\n/boot/mmc1_extlinux/stm32mp257f-dk_extlinux.conf' ||
        mp2_die "cannot list $MP2_CONF_GLOB on the board"
    [ -n "$CONFS" ] || mp2_die "REFUSED: no $MP2_CONF_GLOB on the card: this is not the layout the install was designed for (#1281)"
    while IFS= read -r c; do
        [ -z "$c" ] || [[ $c =~ ^/boot/mmc[0-9]_extlinux/[^/]+\.conf$ ]] || mp2_die "REFUSED: the board listed '$c' for $MP2_CONF_GLOB, which is not a file this install knows"
    done <<<"$CONFS"
}

# mp2_stock_name CONF: the saved copy's name: mmc0_extlinux__extlinux.conf
mp2_stock_name() { local c=${1#/boot/}; echo "${c//\//__}"; }

# the dry run's stand-in for a stock configuration
MP2_DRY_CONF='# Generic Distro Configuration file generated by OpenEmbedded
menu title Select the boot mode
TIMEOUT 20
DEFAULT stm32mp257f-dk-sdcard
LABEL stm32mp257f-dk-sdcard
	KERNEL /Image.gz
	FDT /stm32mp257f-dk.dtb
	APPEND root=PARTUUID=<the rootfs> rootwait rw console=ttySTM0,115200'

# mp2_conf_check CONTENT: refuse a configuration the one-shot cannot use:
# no LABEL, no DEFAULT, no TIMEOUT above 0 (the menu would wait forever at
# "Enter choice" after our label is added), or a yakogl label already there.
mp2_conf_check() {
    local content=$1 t
    grep -qiE '^[[:space:]]*LABEL[[:space:]]+' <<<"$content" || { echo "no LABEL line"; return 1; }
    grep -qiE '^[[:space:]]*DEFAULT[[:space:]]+[^[:space:]]' <<<"$content" || { echo "no DEFAULT line"; return 1; }
    t=$(sed -nE 's/^[[:space:]]*[Tt][Ii][Mm][Ee][Oo][Uu][Tt][[:space:]]+([0-9]+).*/\1/p' <<<"$content" | tail -1)
    { [ -n "$t" ] && [ "$t" -gt 0 ]; } || { echo "no TIMEOUT above 0 (the menu would wait forever)"; return 1; }
    echo ok
}

# mp2_label_block CONTENT: the lines install appends: never the DEFAULT, the
# DEFAULT label's FDT (or FDTDIR) line reused so the image gets the board's
# DTB in x0 as Linux does
mp2_label_block() {
    local content=$1 dflt fdt
    dflt=$(sed -nE 's/^[[:space:]]*[Dd][Ee][Ff][Aa][Uu][Ll][Tt][[:space:]]+([^[:space:]]+).*/\1/p' <<<"$content" | tail -1)
    # (no IGNORECASE: that is gawk's alone, and a gate host may have mawk)
    fdt=$(awk -v d="$dflt" '
        BEGIN { inl = 0; done = 0 }
        /^[[:space:]]*[Ll][Aa][Bb][Ee][Ll][[:space:]]+/ { n = $2; inl = (n == d) }
        inl && /^[[:space:]]*[Ff][Dd][Tt]([Dd][Ii][Rr])?[[:space:]]+[^[:space:]]/ && !done { sub(/^[[:space:]]+/, ""); print; done = 1 }' <<<"$content")
    printf '\n# yakogl one-shot (tools/board-stm32mp2.sh install): never the DEFAULT; the runner types its\n'
    printf '# menu number once at "Enter choice:" for one boot of /yakogl/once.bin, then this boot is Linux again\n'
    printf 'LABEL %s\n\tKERNEL /yakogl/once.bin\n' "$MP2_LABEL"
    [ -n "$fdt" ] && printf '\t%s\n' "$fdt"
    return 0
}

# mp2_has_label CONTENT: our label is in it
mp2_has_label() { grep -qE "^[[:space:]]*[Ll][Aa][Bb][Ee][Ll][[:space:]]+$MP2_LABEL([[:space:]]|$)" <<<"$1"; }

# mp2_check_install: "ok" (the label in every configuration, the stock copies
# and SHA256SUMS there), "absent" (nothing of ours), else the reason
# ("partial: ..." or "foreign: ..."); --dry-run reads nothing
mp2_check_install() {
    local sums c content have=0 lack=0 n=0 missing=""
    if [ "$DRY" = 1 ]; then echo "not read (dry run)"; return 0; fi
    mp2_confs
    sums=$(mp2_b "cat $MP2_SUMS 2>/dev/null")
    while IFS= read -r c; do
        [ -n "$c" ] || continue
        n=$((n + 1))
        content=$(mp2_b "cat $c") || { echo "foreign: cannot read $c"; return 0; }
        if mp2_has_label "$content"; then have=$((have + 1)); else lack=$((lack + 1)); fi
        grep -qF "  $c" <<<"$sums" || missing+=" $c"
    done <<<"$CONFS"
    if [ "$have" = 0 ] && [ -z "$sums" ]; then
        if mp2_b "test -e $MP2_DIR"; then echo "partial: $MP2_DIR exists with no label and no SHA256SUMS"; else echo absent; fi
        return 0
    fi
    [ "$lack" = 0 ] || { echo "partial: the label is in $have of $n configurations"; return 0; }
    [ -n "$sums" ] || { echo "partial: the label is in every configuration but $MP2_SUMS is missing"; return 0; }
    [ -z "$missing" ] || { echo "partial: no stock copy recorded for$missing"; return 0; }
    mp2_b "cd $MP2_STOCK && sha256sum -c --quiet SHA256SUMS.stock 2>/dev/null" || { echo "foreign: a stock copy differs from SHA256SUMS.stock"; return 0; }
    echo ok
}

# ---- install / uninstall ----------------------------------------------------------
# mp2_backup_dir: this host's copies of the stock files (never in git by
# default: records/ of the main checkout, as the netboot env backups)
mp2_backup_dir() {
    local main
    main=$(cd "$(git -C "$MP2_REPO" rev-parse --path-format=absolute --git-common-dir 2>/dev/null || echo "$MP2_REPO/.git")/.." && pwd)
    echo "${STM32MP2_BACKUP_DIR:-$main/records/$MP2_BOARD/extlinux}"
}

# mp2_put CONF CONTENT-FILE: CONTENT-FILE becomes CONF on the board (a .part
# beside it, then mv: the same file system), synced, read back by sha256
mp2_put() {
    local dst=$1 src=$2 want got
    want=$(sha256sum "$src" | cut -d' ' -f1)
    mp2_bin "${MP2_SU}sh -c 'cat > $dst.part && sync && mv $dst.part $dst && sync'" < "$src" ||
        { mp2_b "${MP2_SU}rm -f $dst.part"; return 1; }
    mp2_bget got "sha256sum $dst" "$want  $dst" || return 1
    [ "${got%% *}" = "$want" ] || { mp2_note "$dst reads back as '${got%% *}', not $want" >&2; return 1; }
}

stm32mp2_install() {
    local st c name content block new bdir stamp first=1 sums="" sums_stock="" n=0
    mp2_need_board
    mp2_tmp
    trap mp2_on_exit EXIT
    bdir=$(mp2_backup_dir)
    stamp=$(date +%Y%m%d-%H%M%S)
    # the exact undo, before the first board command of any kind
    mp2_note "UNDO, from Linux: tools/board-stm32mp2.sh uninstall   # each configuration ($MP2_CONF_GLOB${*:+, here: $*}) gets its stock bytes back from $MP2_STOCK/<dir>__<file> (sha256 against $MP2_SUMS), then $MP2_DIR goes"
    mp2_note "UNDO, by hand on the board: for each configuration: cp $MP2_STOCK/<dir>__<file> /boot/<dir>/<file> && sync; then rm -r $MP2_DIR"
    mp2_note "UNDO, without Linux (a person): the microSD in a PC; on its bootfs (the ext4 partition mounted at /boot) copy yakogl/stock/<dir>__<file> over each <dir>/<file>, remove yakogl/"
    mp2_note "The U-Boot environment is not written by this install, so there is nothing of it to undo; the DEFAULT label and the TIMEOUT stay as they are"
    mp2_note "copies of the stock files also go to $bdir/before-install-$stamp/"
    st=$(mp2_check_install)
    case "$st" in
    ok) mp2_note "already installed (the $MP2_LABEL label is in every configuration; stock copies in $MP2_STOCK)"; exit 0 ;;
    absent | "not read (dry run)") ;;
    *) mp2_die "the board has a partial or foreign install ($st); look at it with 'status', then 'uninstall' or fix it by hand" ;;
    esac
    mp2_confs "$@"
    while IFS= read -r c; do
        [ -n "$c" ] || continue
        mp2_note "UNDO of $c, by hand: cp $MP2_STOCK/$(mp2_stock_name "$c") $c && sync"
    done <<<"$CONFS"
    mp2_act mkdir -p "$bdir/before-install-$stamp"
    [ "$DRY" = 1 ] || chmod 700 "$bdir" "$bdir/before-install-$stamp"
    mp2_lease_take
    mp2_boot_where
    # every configuration is read and checked before the first write
    while IFS= read -r c; do
        [ -n "$c" ] || continue
        n=$((n + 1))
        name=$(mp2_stock_name "$c")
        mp2_bget content "cat $c" "$MP2_DRY_CONF" || mp2_die "cannot read $c"
        if mp2_has_label "$content"; then mp2_die "$c already has a $MP2_LABEL label but the install is not complete ($st): fix it by hand"; fi
        st=$(mp2_conf_check "$content") || mp2_die "REFUSED: $c: $st; nothing was changed"
        printf '%s\n' "$content" > "$MP2_TMP/stock-$name"
        # the stock bytes exactly: cat loses no byte but a trailing newline is
        # added by printf; take the board's own bytes for the copy
        if [ "$DRY" = 0 ]; then mp2_b "cat $c" > "$MP2_TMP/stock-$name" || mp2_die "cannot read $c"; fi
        block=$(mp2_label_block "$content")
        { cat "$MP2_TMP/stock-$name"; [ -n "$(tail -c 1 "$MP2_TMP/stock-$name")" ] && echo; printf '%s\n' "$block"; } > "$MP2_TMP/new-$name"
        sums_stock+="$(sha256sum "$MP2_TMP/stock-$name" | cut -d' ' -f1)  $name"$'\n'
        sums+="$(sha256sum "$MP2_TMP/stock-$name" | cut -d' ' -f1)  $c"$'\n'
        if [ "$first" = 1 ]; then
            mp2_note "the label block appended to each configuration:"
            sed 's/^/    /' <<<"$block"
            first=0
        fi
    done <<<"$CONFS"
    [ "$n" -gt 0 ] || mp2_die "no configuration to install into"
    [ "$BOOT_FREE" -ge $((1024 * 1024)) ] || mp2_die "/boot has $BOOT_FREE bytes free, less than 1 MiB for the copies"
    # the stock copies first (board and host), then SHA256SUMS, then the new files
    mp2_act cp "$MP2_TMP"/stock-* "$bdir/before-install-$stamp/"
    mp2_b "${MP2_SU}install -d -m 755 $MP2_STOCK" || mp2_die "cannot create $MP2_STOCK"
    while IFS= read -r c; do
        [ -n "$c" ] || continue
        name=$(mp2_stock_name "$c")
        mp2_b "${MP2_SU}cp -p $c $MP2_STOCK/$name && sync && cmp $c $MP2_STOCK/$name" || mp2_die "saving $c as $MP2_STOCK/$name failed (nothing else changed)"
    done <<<"$CONFS"
    printf '%s' "$sums_stock" > "$MP2_TMP/SHA256SUMS.stock"
    printf '%s' "$sums" > "$MP2_TMP/SHA256SUMS"
    mp2_put "$MP2_STOCK/SHA256SUMS.stock" "$MP2_TMP/SHA256SUMS.stock" || mp2_die "writing $MP2_STOCK/SHA256SUMS.stock failed (the configurations are unchanged)" 5
    mp2_put "$MP2_SUMS" "$MP2_TMP/SHA256SUMS" || mp2_die "writing $MP2_SUMS failed (the configurations are unchanged)" 5
    while IFS= read -r c; do
        [ -n "$c" ] || continue
        name=$(mp2_stock_name "$c")
        mp2_put "$c" "$MP2_TMP/new-$name" || mp2_die "writing $c failed: put it back by hand: cp $MP2_STOCK/$name $c" 5
        mp2_note "$c: the $MP2_LABEL label appended (stock copy $MP2_STOCK/$name, sha256 $(sha256sum "$MP2_TMP/stock-$name" | cut -c1-16)...)"
    done <<<"$CONFS"
    if [ "$DRY" = 1 ]; then mp2_note "(dry run) nothing was written"; exit 0; fi
    st=$(mp2_check_install)
    [ "$st" = ok ] || mp2_die "read-back after install: $st" 5
    mp2_note "installed in $n configuration(s); the U-Boot environment untouched; the default boot is Linux as before"
}

stm32mp2_uninstall() {
    local st sums line sha c name cur stock n=0
    mp2_need_board
    mp2_tmp
    trap mp2_on_exit EXIT
    st=$(mp2_check_install)
    if [ "$st" = absent ]; then mp2_note "not installed"; exit 0; fi
    mp2_lease_take
    mp2_boot_where
    mp2_bget sums "cat $MP2_SUMS" "0000000000000000000000000000000000000000000000000000000000000000  /boot/mmc0_extlinux/extlinux.conf" ||
        mp2_die "cannot read $MP2_SUMS: restore the configurations by hand from $MP2_STOCK (or this host's copies under $(mp2_backup_dir))"
    # first every file is checked (the stock copy's sha256, and that the
    # current file is the stock one or ours), then every file is restored:
    # a refusal leaves the card as it was
    local todo=""
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        sha=${line%%  *}; c=${line#*  }
        name=$(mp2_stock_name "$c")
        [[ $c == /boot/*.conf ]] || mp2_die "REFUSED: $MP2_SUMS names '$c', which is not a /boot/...conf file"
        mp2_bget stock "sha256sum $MP2_STOCK/$name" "$sha  $MP2_STOCK/$name" || mp2_die "no stock copy $MP2_STOCK/$name: restore $c by hand from this host's copies under $(mp2_backup_dir)"
        [ "${stock%% *}" = "$sha" ] || mp2_die "REFUSED: the stock copy $MP2_STOCK/$name reads sha256 '${stock%% *}', not the recorded $sha: restore $c by hand from this host's copies under $(mp2_backup_dir)"
        mp2_bget cur "sha256sum $c 2>/dev/null" "ffff  $c"
        if [ "${cur%% *}" = "$sha" ]; then
            mp2_note "$c is already the stock file"
        else
            if [ "$DRY" = 0 ]; then
                mp2_b "cat $c" > "$MP2_TMP/cur-$name" 2>/dev/null || :
                mp2_has_label "$(cat "$MP2_TMP/cur-$name" 2>/dev/null)" ||
                    mp2_die "REFUSED: $c has no $MP2_LABEL label and is not the stock file either: someone changed it since the install; a person decides (the stock copy is $MP2_STOCK/$name); nothing was changed"
            fi
            todo+="$sha  $c"$'\n'
        fi
        n=$((n + 1))
    done <<<"$sums"
    while IFS= read -r line; do
        [ -n "$line" ] || continue
        sha=${line%%  *}; c=${line#*  }
        name=$(mp2_stock_name "$c")
        mp2_b "${MP2_SU}sh -c 'cp $MP2_STOCK/$name $c.part && sync && mv $c.part $c && sync'" || mp2_die "restoring $c failed" 5
        mp2_bget cur "sha256sum $c" "$sha  $c" || mp2_die "cannot read back $c" 5
        [ "${cur%% *}" = "$sha" ] || mp2_die "$c reads back as '${cur%% *}', not the stock $sha" 5
        mp2_note "$c restored byte for byte (sha256 ${sha:0:16}...)"
    done <<<"$todo"
    mp2_b "${MP2_SU}rm -rf $MP2_DIR && sync && test ! -e $MP2_DIR" || mp2_die "removing $MP2_DIR failed" 5
    if [ "$DRY" = 1 ]; then mp2_note "(dry run) nothing was written"; exit 0; fi
    st=$(mp2_check_install)
    [ "$st" = absent ] || mp2_die "read-back after uninstall: $st" 5
    mp2_note "uninstalled: $n configuration(s) are the stock files again, $MP2_DIR removed"
}

# ---- status -----------------------------------------------------------------------
stm32mp2_status() {
    mp2_need_board
    mp2_tmp
    trap mp2_on_exit EXIT
    mp2_b "echo 'board lease ($MP2_LEASE):'; cat $MP2_LEASE 2>/dev/null || echo none; echo \"board clock: \$(date +%s)\""
    mp2_b "cat /proc/sys/kernel/random/boot_id; cut -d. -f1 /proc/uptime; findmnt -n -o SOURCE,FSTYPE,TARGET -T /boot; df -B1 --output=avail /boot | tail -1; ls -l $MP2_DIR $MP2_STOCK 2>/dev/null; cat $MP2_SUMS 2>/dev/null; grep -c '^[[:space:]]*[Ll][Aa][Bb][Ee][Ll][[:space:]]*$MP2_LABEL\$' $MP2_CONF_GLOB 2>/dev/null"
    echo "install: $(mp2_check_install)"
    if [ -n "${BOARD_UART:-}" ]; then echo "console: set (not printed)"; else echo "console: BOARD_UART unset"; fi
}

# ---- the console client: capture and the typed choice ------------------------------
# One program for both forms of BOARD_UART (a character device, or host:port
# of a serial bridge), written to MP2_TMP at run time. Its text is below;
# python3 -I runs it. The scan of a capture is the same program.
mp2_console_py() {
    cat <<'PYEOF'
"""tools/board-stm32mp2.sh's console client (written at run time; see the
shell file's header). Modes:
  capture SRC RAW TS EVENTS [--choose LABEL] [--arm-s N] [--baud B]
     SRC a character device (set to 8N1 raw at B baud, default 115200) or
     host:port (a raw TCP serial bridge; "busy" in its first bytes is exit 4).
     Bytes go to RAW unchanged and every completed line to TS as
     "<unix time> <line>" (CR and NUL removed). With --choose the typer runs:
     when a menu line "N:<tab>LABEL" has been seen and "Enter choice: "
     follows, it sends "N\r" once, within --arm-s seconds of the start (180),
     and never again. Events go to EVENTS with the time. Runs until SIGTERM
     or the source closes.
  scan TS [--label LABEL] [--end-re RE]
     The phases of one cycle and the boot verdict (the shell header's
     words), as "key=value" words on one line; exit 0 when complete.
"""
import os
import re
import select
import signal
import socket
import sys
import termios
import time

ENTER = b"Enter choice: "
MP2_LINE_RE = re.compile(r"mp2-handover: v=1 (.*)$")


def clean(b):
    return b.replace(b"\r", b"").replace(b"\0", b"").decode("utf-8", "replace")


def open_source(src, baud):
    if re.match(r"^[^/:]+:[0-9]+$", src):
        host, port = src.rsplit(":", 1)
        s = socket.create_connection((host, int(port)), timeout=10)
        s.setblocking(False)
        return s.fileno(), s, True
    fd = os.open(src, os.O_RDWR | os.O_NOCTTY | os.O_NONBLOCK)
    try:
        attr = termios.tcgetattr(fd)
        rate = getattr(termios, "B%d" % baud)
        attr[0] = 0                    # iflag
        attr[1] = 0                    # oflag
        attr[2] = termios.CS8 | termios.CREAD | termios.CLOCAL  # cflag: 8N1, no flow control
        attr[3] = 0                    # lflag: raw, no echo
        attr[4] = rate
        attr[5] = rate
        attr[6][termios.VMIN] = 0
        attr[6][termios.VTIME] = 0
        termios.tcsetattr(fd, termios.TCSANOW, attr)
    except termios.error:
        pass                           # a pty in the self-test: the line speed is nobody's
    return fd, None, False


def capture(args):
    src, raw_path, ts_path, ev_path = args[:4]
    label, arm_s, baud = None, 180.0, 115200
    rest = args[4:]
    while rest:
        if rest[0] == "--choose":
            label = rest[1]; rest = rest[2:]
        elif rest[0] == "--arm-s":
            arm_s = float(rest[1]); rest = rest[2:]
        elif rest[0] == "--baud":
            baud = int(rest[1]); rest = rest[2:]
        else:
            print("capture: unknown option %s" % rest[0], file=sys.stderr)
            return 2
    try:
        fd, sock, is_net = open_source(src, baud)
    except OSError as e:
        print("%.3f console: cannot open the console: %s" % (time.time(), e.strerror or e), file=sys.stderr)
        return 4

    def on_term(signum, frame):
        raise SystemExit(0)
    signal.signal(signal.SIGTERM, on_term)

    menu_re = re.compile(r"^\s*(\d+):\s*%s\s*$" % re.escape(label)) if label else None
    choice_re = re.compile(r"^\s*(\d+):\s*%s\s*$" % re.escape(label)) if label else None
    t0 = time.time()
    typed = False
    taken = False
    num = None
    since = b""          # bytes since the last "Enter choice"
    pending = b""
    head = b""
    with open(raw_path, "ab", buffering=0) as raw, open(ts_path, "a", buffering=1) as ts, \
            open(ev_path, "a", buffering=1) as ev:
        def event(msg):
            ev.write("%.3f %s\n" % (time.time(), msg))
        event("console open (%s), typer %s" % ("bridge" if is_net else "device",
                                                ("armed for %s, %.0f s" % (label, arm_s)) if label else "off"))
        while True:
            r, _, _ = select.select([fd], [], [], 0.02)
            if not r:
                continue
            try:
                chunk = sock.recv(4096) if is_net else os.read(fd, 4096)
            except BlockingIOError:
                continue
            except OSError as e:
                event("console error: %s" % e)
                return 0
            if not chunk:
                event("console closed")
                return 0
            now = time.time()
            raw.write(chunk)
            if is_net and now - t0 < 3 and len(head) < 256:
                head += chunk
                if b"busy" in head:
                    event("the bridge is busy (another client holds it)")
                    return 4
            pending += chunk
            while b"\n" in pending:
                ln, pending = pending.split(b"\n", 1)
                text = clean(ln)
                ts.write("%.3f %s\n" % (now, text))
                if menu_re and not typed:
                    m = menu_re.match(text)
                    if m:
                        num = m.group(1)
                        event("menu offers %s as %s" % (label, num))
                if choice_re and typed and not taken and choice_re.match(text):
                    taken = True
                    event("U-Boot took the choice: %s" % text.strip())
            since += chunk
            if label and ENTER in since:
                since = b""
                if typed:
                    event("Enter choice again (a later boot): nothing typed, the default boots")
                elif now - t0 > arm_s:
                    event("Enter choice after %.0f s: past --arm-s, nothing typed" % (now - t0))
                    typed = True
                elif num is None:
                    event("Enter choice without a %s line in the menu: nothing typed (not installed where U-Boot reads?)" % label)
                    typed = True
                else:
                    data = (num + "\r").encode()
                    try:
                        if is_net:
                            sock.sendall(data)
                        else:
                            os.write(fd, data)
                    except OSError as e:
                        event("typing %s failed: %s" % (num, e))
                        return 0
                    typed = True
                    event("typed %s at Enter choice" % num)


def scan(args):
    ts_path = args[0]
    label, end_re = "yakogl", r"^mp2: .*: PSCI SYSTEM_RESET at [0-9]+ ms|ZEPHYR FATAL ERROR"
    rest = args[1:]
    while rest:
        if rest[0] == "--label":
            label = rest[1]; rest = rest[2:]
        elif rest[0] == "--end-re":
            end_re = rest[1]; rest = rest[2:]
        else:
            print("scan: unknown option %s" % rest[0], file=sys.stderr)
            return 2
    end_rx = re.compile(end_re)
    menu_rx = re.compile(r"^\s*(\d+):\s*%s\s*$" % re.escape(label))
    lines = []
    try:
        with open(ts_path, "rb") as f:
            for row in f:
                row = row.rstrip(b"\n")
                m = re.match(rb"^([0-9.]+) ?", row)
                t = float(m.group(1)) if m else 0.0
                lines.append((t, clean(row[m.end():] if m else row)))
    except OSError:
        lines = []
    ph = {}
    nmenu = nchosen = 0
    handover = None
    end_line = None
    boot_banners = 0
    prev = ""
    for i, (t, text) in enumerate(lines):
        s = text.strip()
        if "Enter choice" in text:
            nmenu += 1
            ph.setdefault("menu", t)
            # the default taken on the timeout prints on the same line
            # ("Enter choice: 1:<tab>linux"); a typed choice echoes its digits,
            # and U-Boot prints the label on the next line
            if menu_rx.match(text.split("Enter choice:", 1)[1]):
                ph.setdefault("chosen", t)
                nchosen += 1
        elif menu_rx.match(text):
            if "Enter choice" in prev:
                ph.setdefault("chosen", t)
                nchosen += 1
            else:
                ph.setdefault("offered", t)
        if s:
            prev = text
        if "chosen" in ph and "go" not in ph and s.startswith("Starting kernel"):
            ph["go"] = t
        if "go" in ph and "started" not in ph and (s.startswith("MP2:A") or "Booting Zephyr OS" in s):
            ph["started"] = t
        if "go" in ph and "zephyr" not in ph and "Booting Zephyr OS" in s:
            ph["zephyr"] = t
        if "go" in ph and "armed" not in ph and "board: watchdog armed" in s:
            ph["armed"] = t
        m = MP2_LINE_RE.search(s)
        if m and "go" in ph and handover is None:
            handover = m.group(1).strip()
        if "started" in ph and "end" not in ph and end_rx.search(s):
            ph["end"] = t
            end_line = s
        if "started" in ph and "reset" not in ph and (s.startswith("NOTICE:  CPU:") or s.startswith("U-Boot 20") or s.startswith("NOTICE:  BL2:")):
            ph["reset"] = t
        if "reset" in ph and "linux" not in ph and s.startswith("Starting kernel"):
            ph["linux"] = t
        if s.startswith("U-Boot 20"):
            boot_banners += 1
    if not lines:
        bv = "no_console"
    elif nchosen >= 2:
        bv = "looped"
    elif "menu" not in ph:
        bv = "no_menu"
    elif "offered" not in ph:
        bv = "label_not_offered"
    elif "chosen" not in ph:
        bv = "not_chosen"
    elif "go" not in ph:
        bv = "load_failed"
    elif "started" not in ph:
        bv = "no_zephyr"
    elif "end" in ph and "reset" in ph:
        bv = "complete"
    elif "end" in ph:
        bv = "end_no_reset"
    elif "reset" in ph:
        bv = "reset_without_end"
    else:
        bv = "no_end"
    words = ["verdict=%s" % bv]
    first = lines[0][0] if lines else 0.0
    for k in ("menu", "offered", "chosen", "go", "started", "zephyr", "armed", "end", "reset", "linux"):
        if k in ph:
            words.append("%s=%.1fs" % (k, ph[k] - first))
    if handover:
        words.append("handover=%s" % handover.replace(" ", ","))
    if end_line:
        words.append("end_line=%s" % end_line.replace(" ", "_"))
    print(" ".join(words))
    return 0 if bv == "complete" else 1


def main(argv):
    if len(argv) >= 5 and argv[0] == "capture":
        return capture(argv[1:])
    if len(argv) >= 2 and argv[0] == "scan":
        return scan(argv[1:])
    print(__doc__, file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
PYEOF
}

# mp2_console_tool: the client's path (written once per process)
mp2_console_tool() {
    mp2_tmp
    [ -f "$MP2_TMP/mp2_console.py" ] || mp2_console_py > "$MP2_TMP/mp2_console.py"
    echo "$MP2_TMP/mp2_console.py"
}

# mp2_uart_free: nobody holds the console (a device: every /proc/<pid>/fd;
# a bridge: it must accept a connection and not answer busy)
mp2_uart_free() {
    local uart p f holders=
    if mp2_uart_is_bridge; then
        if [ "$DRY" = 1 ]; then
            echo "DRY host\$ python3 tools/am62p_netboot_uart.py probe $BOARD_UART   # the bridge must accept and not be busy"
            return 0
        fi
        python3 -I "$MP2_HERE/am62p_netboot_uart.py" probe "$BOARD_UART" || mp2_die "BOARD_UART bridge busy, refused or unreachable (rc $?)"
        return 0
    fi
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ scan every /proc/<pid>/fd for $BOARD_UART (nobody may hold it)"
        return 0
    fi
    uart=$(readlink -f "$BOARD_UART")
    for p in /proc/[0-9]*; do
        for f in "$p"/fd/*; do
            [ "$(readlink "$f" 2>/dev/null)" = "$uart" ] &&
                holders+=" ${p#/proc/}($(tr '\0' ' ' < "$p/cmdline" 2>/dev/null | cut -c1-60))"
        done
    done
    [ -z "$holders" ] || mp2_die "$uart is held by:$holders"
}

# mp2_capture_start: the console client into the run directory, before the
# reboot; MP2_CAP its PID. Returns 1 when it ended at once.
mp2_capture_start() {
    local tool
    tool=$(mp2_console_tool)
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ python3 -I <mp2_console.py> capture $BOARD_UART <rundir>/console.raw <rundir>/console.ts <rundir>/console.events --choose $MP2_LABEL --arm-s ${STM32MP2_ARM_S:-180} &   # types the menu number once at 'Enter choice: '"
        return 0
    fi
    python3 -I "$tool" capture "$BOARD_UART" "$MP2_RD/console.raw" "$MP2_RD/console.ts" "$MP2_RD/console.events" \
        --choose "$MP2_LABEL" --arm-s "${STM32MP2_ARM_S:-180}" 2>> "$MP2_RD/console.events" &
    MP2_CAP=$!
    sleep 2
    if ! kill -0 "$MP2_CAP" 2>/dev/null; then
        wait "$MP2_CAP" 2>/dev/null
        MP2_CAP=
        return 1
    fi
    return 0
}

# mp2_console_txt: console.txt ("HH:MM:SS line") from console.ts
mp2_console_txt() {
    [ -f "$MP2_RD/console.ts" ] || return 0
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
' < "$MP2_RD/console.ts" > "$MP2_RD/console.txt"
}

# mp2_redact DIR STRING...: no address in the copies (tools/board.sh's rule):
# IPv4 outside 192.0.2.0/24 -> 192.0.2.<n>, MAC -> 02:00:00:00:00:<nn>, each
# STRING of six characters or more -> <board-host>
mp2_redact() {
    local dir=$1 f n=0 ip mac s esc
    shift
    local files=()
    for f in "$dir"/*; do [ -f "$f" ] && [ "${f##*/}" != INDEX.txt ] && files+=("$f"); done
    [ ${#files[@]} -gt 0 ] || return 0
    for s in "$@"; do
        [ ${#s} -ge 6 ] || continue
        esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
        LC_ALL=C sed -i -E "s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1<board-host>\\2/g;
            s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1<board-host>\\2/g" "${files[@]}"
    done
    while read -r ip; do
        n=$((n + 1))
        LC_ALL=C sed -i -E "s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g;
            s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g" "${files[@]}"
    done < <(LC_ALL=C grep -aohE '(^|[^0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9.]|$)' "${files[@]}" |
        LC_ALL=C grep -aoE '([0-9]{1,3}\.){3}[0-9]{1,3}' | grep -v '^192\.0\.2\.' | awk '!seen[$0]++')
    n=0
    while read -r mac; do
        n=$((n + 1))
        LC_ALL=C sed -i "s/$mac/02:00:00:00:00:$(printf '%02x' "$n")/g" "${files[@]}"
    done < <(LC_ALL=C grep -aohE '([0-9a-fA-F]{2}:){5}[0-9a-fA-F]{2}' "${files[@]}" |
        grep -v '^02:00:00:00:00:' | awk '!seen[$0]++')
}

# ---- the verdict --------------------------------------------------------------------
# stm32mp2_verdict DIR: the verdict of a run from its files alone (run and
# the tally). Sets V_LINE V_RC V_VERDICT V_SCAN.
stm32mp2_verdict() {
    local dir=$1 name rcc scan bv verdict="" result=none ba=? end_line why legacy ho tool
    name=$(mp2_mget name "$dir"); rcc=$(mp2_mget rc_cycle "$dir")
    tool=$(mp2_console_tool)
    scan=$(python3 -I "$tool" scan "$dir/console.ts" --label "$MP2_LABEL" --end-re "${BOARD_END_RE:-$MP2_END_RE_DEFAULT}" 2>/dev/null)
    bv=$(sed -n 's/.*verdict=\([a-z_]*\).*/\1/p' <<<"$scan"); bv=${bv:-scan_failed}
    ho=$(sed -n 's/.*handover=\([^ ]*\).*/\1/p' <<<"$scan"); ho=${ho:-absent}
    end_line=$(sed -n 's/.*end_line=\([^ ]*\).*/\1/p' <<<"$scan" | tr '_' ' ')
    if [ "$(mp2_mget back "$dir")" != 1 ]; then
        bv=not_back; rcc=${rcc:-5}
    fi
    # the result: the sample's own line, else the layer's end line
    if [ -s "$dir/console.ts" ]; then
        legacy=$(python3 -I -c '
import re, sys
rx = re.compile(sys.argv[2])
last = ""
for row in open(sys.argv[1], "rb"):
    t = row.decode("utf-8", "replace").rstrip("\n")
    t = re.sub(r"^[0-9.]+ ?", "", t)
    if rx.search(t):
        last = t
print(last)' "$dir/console.ts" "$MP2_LEGACY_RE" 2>/dev/null)
    fi
    case "$bv" in
    complete)
        if [ -n "${legacy:-}" ]; then
            case "$legacy" in
            *": PASS"*) result=legacy:pass; verdict=done ;;
            *": FAIL"*) result=legacy:fail; verdict=failed ;;
            "deqp: summary: "*) result=legacy:deqp_summary; verdict=done ;;
            "gl_test: all: "*) result=legacy:gl_test_all; verdict=done ;;
            *) result=legacy:other; verdict=done ;;
            esac
        fi
        if [ -z "$verdict" ]; then
            case "$end_line" in
            "mp2: fatal error: "*) result=end:fatal; verdict=failed ;;
            "mp2: lifetime over: "*) result=end:lifetime; verdict=lifetime_end ;;
            mp2:*": PSCI SYSTEM_RESET at"*) why=${end_line#mp2: }; why=${why%%: PSCI*}; result="end:${why// /_}"; verdict=done ;;
            *"ZEPHYR FATAL ERROR"*) result=end:fatal; verdict=failed ;;
            *) result=none; verdict=no_record ;;
            esac
        fi
        ;;
    *) verdict=$bv ;;
    esac
    if [ -n "$rcc" ]; then V_RC=$rcc
    elif [ "$verdict" = done ]; then V_RC=0
    else V_RC=4; fi
    if [ -n "$(mp2_mget t_back "$dir")" ] && [ -n "$(mp2_mget t_reboot "$dir")" ]; then
        ba=$(($(mp2_mget t_back "$dir") - $(mp2_mget t_reboot "$dir")))
    fi
    V_VERDICT=$verdict
    V_SCAN=$scan
    V_LINE="board.sh $MP2_BOARD $name rc=$V_RC verdict=$verdict boot=$MP2_BOOT row boot_verdict=$bv result=$result resets=absent record=absent back_after=${ba}s handover=$ho"
    return 0
}

mp2_write_index() {
    local dir=$1 f
    {
        echo "# tools/board-stm32mp2.sh run record (tools/board.sh's shape; records/README.md)"
        echo "runner: tools/board-stm32mp2.sh build $(git -C "$MP2_REPO" describe --always --dirty 2>/dev/null || echo unknown)"
        echo "board: $MP2_BOARD"
        echo "boot: $MP2_BOOT (a row)"
        echo "name: $(mp2_mget name "$dir")"
        echo "image: $(mp2_mget image "$dir") sha256 $(mp2_mget image_sha256 "$dir") size $(mp2_mget image_size "$dir") footprint $(mp2_mget footprint "$dir")"
        echo "staged: $(mp2_mget staged "$dir")"
        echo "image zephyr: $(mp2_mget image_zephyr "$dir")"
        echo "started: $(mp2_mget started "$dir")"
        echo "ended: $(mp2_mget ended "$dir")"
        echo "verdict: $V_LINE"
        echo "phase scan: $V_SCAN"
        echo "records:"
        echo "  none (the record is the console: the hand-over line and the end line above)"
        echo "files (sha256, lines):"
        for f in "$dir"/*; do
            { [ -f "$f" ] && [ "${f##*/}" != INDEX.txt ]; } || continue
            printf '%s  %s  %s\n' "$(sha256sum "$f" | cut -d' ' -f1)" "$(wc -l < "$f")" "${f##*/}"
        done
    } > "$dir/INDEX.txt"
}

# ---- run ----------------------------------------------------------------------------
stm32mp2_run() {
    local img logdir sha8 st who kx rcc="" el up="" t0 got leftover bound ret
    [ $# -ge 2 ] || mp2_die "usage: run <image> <logdir> [name]" 2
    img=$1; logdir=$2; MP2_NAME=${3:-$(basename "$img" .bin)}
    [[ $MP2_NAME =~ ^[A-Za-z0-9_-]+$ ]] || mp2_die "name '$MP2_NAME': letters, digits, _ and - only" 2
    mp2_need_board uart
    mp2_tmp
    trap mp2_on_exit EXIT
    # 1. the image
    mp2_check_image "$img"
    sha8=${IMG_SHA:0:8}
    bound=${RUN_BOUND:-120}; ret=${RETURN_BOUND:-120}
    if [ $((bound + ret)) -lt 90 ] && [ "${STM32MP2_SELFTEST:-0}" != 1 ]; then
        mp2_die "RUN_BOUND + RETURN_BOUND is $((bound + ret)) s: the wait for Linux after a reset is never below 90 s on this board" 2
    fi
    HOLD_MAX=${HOLD_MAX:-$((bound + ret + 300))}
    MP2_RD="$logdir/$(date +%Y%m%d-%H%M)-$MP2_NAME-$sha8"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ mkdir -p $MP2_RD   # meta.txt, console.raw, console.ts, console.events, console.txt, INDEX.txt"
    else
        [ -e "$MP2_RD" ] && mp2_die "$MP2_RD exists: a record is never overwritten"
        mkdir -p "$MP2_RD" || mp2_die "cannot create $MP2_RD"
    fi
    mp2_meta board "$MP2_BOARD"; mp2_meta boot "$MP2_BOOT"; mp2_meta name "$MP2_NAME"
    mp2_meta image "$(basename "$img")"; mp2_meta image_sha256 "$IMG_SHA"; mp2_meta image_size "$IMG_SIZE"
    mp2_meta footprint "$IMG_FOOTPRINT"; mp2_meta image_zephyr "$IMG_ZEPHYR"
    mp2_meta image_header "text_offset $(printf '0x%x' "$IMG_TEXT_OFFSET") flags $(printf '0x%x' "$IMG_FLAGS")"
    mp2_meta booti_form "$IMG_BOOTI"; mp2_meta staged_sha256 "$STAGE_SHA"
    mp2_meta started "$(date '+%F %T')"
    mp2_note "$MP2_NAME: build $(git -C "$MP2_REPO" describe --always --dirty 2>/dev/null || echo unknown) boot $MP2_BOOT (row) image sha256 ${IMG_SHA:0:16} footprint $IMG_FOOTPRINT"
    mp2_note "$MP2_NAME: booti form: $IMG_BOOTI; staged sha256 ${STAGE_SHA:0:16}; zephyr: $IMG_ZEPHYR"
    # 2. the console is free, the lock and the lease, the pre-checks
    mp2_uart_free
    mp2_lease_take
    mp2_bget st 'true' || mp2_die "$BOARD_HOST does not answer (ssh key login)"
    mp2_bget BOOT0 'cat /proc/sys/kernel/random/boot_id' DRY-BOOT-ID || mp2_die "cannot read the board's boot_id"
    mp2_bget BHOSTNAME 'uname -n' DRY-HOSTNAME || mp2_die "cannot read the board's host name"
    mp2_bget who 'who' '' || mp2_die "cannot run who on the board"
    mp2_meta who_sessions "$(grep -c . <<<"$who")"
    mp2_boot_where
    [ "$BOOT_FREE" -ge $((2 * IMG_SIZE)) ] || mp2_die "/boot has $BOOT_FREE bytes free, less than twice the image ($IMG_SIZE)"
    mp2_meta boot_where "/boot = $BOOT_DEV ext4, $BOOT_FREE bytes free"
    if [ "$DRY" = 1 ]; then
        mp2_b "cat $MP2_SUMS; grep -l '^[[:space:]]*[Ll][Aa][Bb][Ee][Ll][[:space:]]*$MP2_LABEL\$' $MP2_CONF_GLOB   # the label must be in every configuration"
        mp2_b "ls $MP2_DIR/once*.bin 2>/dev/null   # must print nothing"
    else
        st=$(mp2_check_install)
        [ "$st" = ok ] || mp2_die "the $MP2_LABEL label is not installed as tools/board-stm32mp2.sh writes it ($st; 'install' first)"
        leftover=$(mp2_b "ls $MP2_DIR/once*.bin 2>/dev/null")
        [ -z "$leftover" ] || mp2_die "$MP2_DIR holds an image of a run that did not clean up ($leftover): look at it, then 'uninstall' and 'install' (or remove it by hand)"
    fi
    # 3. stage, read back after the page cache is dropped
    mp2_bin "${MP2_SU}sh -c 'cat > $MP2_ONCE.part && sync && mv $MP2_ONCE.part $MP2_ONCE && sync'" < "$STAGE_FILE" ||
        { mp2_b "${MP2_SU}rm -f $MP2_ONCE.part $MP2_ONCE"; mp2_die "copying the image to $MP2_ONCE failed"; }
    mp2_bget got "echo 3 | ${MP2_SU}tee /proc/sys/vm/drop_caches >/dev/null; sha256sum $MP2_ONCE" "$STAGE_SHA  $MP2_ONCE"
    got=${got%% *}
    [ "$got" = "$STAGE_SHA" ] || { mp2_b "${MP2_SU}rm -f $MP2_ONCE"; mp2_die "$MP2_ONCE reads back as '$got', not $STAGE_SHA (removed; no reboot)"; }
    mp2_meta staged "$MP2_ONCE on the microSD's bootfs, read back equal (sha256 $STAGE_SHA)"
    # 4. the console, before anything can print
    mp2_capture_start || { mp2_b "${MP2_SU}rm -f $MP2_ONCE"; mp2_die "the console client ended at once (device or bridge busy, unreachable, or refused); staged image removed, no reboot. $(tail -1 "$MP2_RD/console.events" 2>/dev/null)"; }
    # 5. reboot; the ssh session ends with it
    mp2_b "sync; ${MP2_SU}systemctl reboot" || true
    t0=$(date +%s)
    mp2_meta t_reboot "$t0"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ wait until ssh answers from a new boot (uptime < time since the reboot, boot_id changed), <= $((bound + ret)) s; else: the board needs a person, no retry"
        mp2_b "${MP2_SU}rm -f $MP2_ONCE && test ! -e $MP2_ONCE"
        echo "DRY host\$ stop the console client; console.txt from console.ts; the verdict line from the run directory"
        exit 0
    fi
    # 6. a NEW boot answers
    while :; do
        sleep 5
        el=$(($(date +%s) - t0))
        up=$(timeout 8 ssh "${MP2_SSH_OPTS[@]}" -o ConnectTimeout=5 "$BOARD_HOST" 'cut -d. -f1 /proc/uptime; cat /proc/sys/kernel/random/boot_id' 2>/dev/null)
        kx=${up#*$'\n'}; up=${up%%$'\n'*}
        if [ -n "$up" ] && [ "$up" -lt "$el" ] 2>/dev/null && [ "$kx" != "$BOOT0" ]; then break; fi
        if [ "$el" -gt $((bound + ret)) ]; then rcc=5; break; fi
    done
    mp2_meta t_back "$(date +%s)"
    if [ "$rcc" = 5 ]; then
        mp2_meta back 0
        mp2_meta cleanup "not done: Linux not back ($MP2_ONCE left on the card)"
        mp2_note "the board did not answer from a new boot within $((bound + ret)) s of the reboot: it has no remote reset, so THE BOARD NEEDS A PERSON. Nothing is retried, and no other run may take it until the owner says it is back. $MP2_ONCE is still on the card; the next boot is Linux (the DEFAULT label) if the board is power-cycled." >&2
    else
        mp2_meta back 1
        mp2_meta up "$up"
        # 7. the staged image goes, read back absent
        if mp2_b "${MP2_SU}rm -f $MP2_ONCE && test ! -e $MP2_ONCE"; then
            mp2_meta cleanup "$MP2_ONCE removed (read back)"
        else
            mp2_meta cleanup "$MP2_ONCE NOT removed"
            mp2_note "warning: $MP2_ONCE was not removed" >&2
        fi
    fi
    [ -n "$rcc" ] && mp2_meta rc_cycle "$rcc"
    # the capture: the last lines of the new boot are still arriving
    [ "$rcc" = 5 ] || sleep 3
    mp2_stop_cap
    mp2_console_txt
    mp2_meta ended "$(date '+%F %T')"
    mp2_redact "$MP2_RD" "${BHOSTNAME:-}" "${BOARD_HOST#*@}"
    stm32mp2_verdict "$MP2_RD"
    mp2_write_index "$MP2_RD"
    if [ -s "$MP2_RD/console.txt" ]; then
        sed -E 's/^[0-9:]{8} //' "$MP2_RD/console.txt" | tr -d '\0\r' |
            grep -aE 'Enter choice|Starting kernel|Moving Image|MP2:|mp2-handover|Booting Zephyr|board: watchdog armed|: (PASS|FAIL)|PSCI SYSTEM_RESET|ZEPHYR FATAL|yk-record: ' |
            sed 's/^/  /'
    fi
    [ -f "$MP2_RD/console.events" ] && sed 's/^/  event: /' "$MP2_RD/console.events"
    echo "$V_LINE"
    mp2_note "record $MP2_RD/INDEX.txt"
    [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null
    exit "$V_RC"
}

# ---- standalone ----------------------------------------------------------------------
mp2_usage() { sed -n '12,16p' "${BASH_SOURCE[0]}" >&2; exit 2; }

if [ "${BASH_SOURCE[0]}" = "$0" ]; then
    set -uo pipefail
    mp2_args=()
    for mp2_a in "$@"; do
        if [ "$mp2_a" = --dry-run ]; then DRY=1; else mp2_args+=("$mp2_a"); fi
    done
    set -- "${mp2_args[@]+"${mp2_args[@]}"}"
    [ $# -ge 1 ] || mp2_usage
    mp2_verb=$1; shift
    case "$mp2_verb" in
    install) stm32mp2_install "$@" ;;
    uninstall) [ $# -eq 0 ] || mp2_usage; stm32mp2_uninstall ;;
    run) stm32mp2_run "$@" ;;
    status) [ $# -eq 0 ] || mp2_usage; stm32mp2_status ;;
    verdict)
        [ $# -eq 1 ] || mp2_usage
        mp2_tmp; trap mp2_on_exit EXIT
        stm32mp2_verdict "$1"; echo "$V_LINE"; exit "$V_RC" ;;
    *) mp2_usage ;;
    esac
fi
