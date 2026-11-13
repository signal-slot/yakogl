#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# board-stm32mp2-uboot.sh - the STM32MP257F-DK's typed U-Boot one-shot: boots
# a Zephyr image once from ST's own U-Boot (2023.10, after TF-A v2.10 and
# OP-TEE 4.0) and gives the board back to ST's Linux, with bootcmd and the
# stored U-Boot environment unchanged (docs/notes/stm32mp257f-dk-uboot-boot.md;
# plan #357, issue #1281).
#
#   tools/board-stm32mp2-uboot.sh [--dry-run] status
#   tools/board-stm32mp2-uboot.sh [--dry-run] stage <image>
#   tools/board-stm32mp2-uboot.sh [--dry-run] run <image> <logdir> [name]
#   tools/board-stm32mp2-uboot.sh [--dry-run] disarm
#   tools/board-stm32mp2-uboot.sh selftest    (tools/board_stm32mp2_uboot_selftest.sh, no board)
#
# The route is TYPED: nothing is armed in the stored environment. The runner
# stages a U-Boot script (driver/tools/uboot-mp2: mp2-mmc.cmd or mp2-net.cmd,
# built by its build.sh and read back with tools/uboot_script.py) and the
# image, reboots the board's Linux, and tools/mp2_uboot_console.py stops
# autoboot with a key and types ONE line at ST's prompt. The line loads the
# script to ${scriptaddr} (U-Boot's own variable, read live by U-Boot; the
# runner checks the stored value is set and clear of the go window, it never
# hard-codes an address), sources it, and falls through to "run bootcmd"
# whatever the script did; the script loads the image to 0xa0000000 (the
# image's link address; 0xa0000000..0xb0000000 is clear of U-Boot, the secure
# firmware and the distro loop's addresses: plan #357 go-window-overlap) and
# jumps there with "go". Every variable it sets is mp2z_* (and autoload, for
# the net route), in RAM: a reset forgets them. The runner never writes the
# environment, never names bootcmd as a target, never writes a partition or
# the eMMC (its only writes are files under /boot/yakogl/ on the card's
# bootfs, made from Linux, and files under the TFTP root on the host), never
# loads or unloads a kernel module and never touches tailscale or sshd. The
# staged files are named mp2z-<token>.scr and mp2z-<token>.bin, never a name
# U-Boot's distro loop looks for by itself.
#
# Backends (BOARD_BOOT):
#   uboot-mmc   (default) the script and the image under /boot/yakogl/ on the
#               bootfs partition of the microSD the board booted from, copied
#               over ssh and read back by sha256; U-Boot reads them with
#               "load mmc ${boot_instance}:<bootfs part>" (the medium ST's
#               U-Boot booted from). The typed line sizes the image first and
#               refuses one of 255 MiB or more (itest ... -lt 0xff00000).
#   uboot-tftp  the script and the image under <NETBOOT_TFTP_ROOT>/yakogl/,
#               served once by tools/tftp_oneshot.py (each file renamed
#               <name>.served after one transfer); the server must be running
#               and is proven with tools/uboot-netboot-lib.sh's nb_tftp_canary
#               before the board is touched (the library is sourced in a
#               subshell for that function alone, read-only use).
# On both, the image is refused on the host when it is 255 MiB or more (the
# window is 256 MiB), before anything is staged; the staged copies are those
# bytes (sha256 read back), so the bound holds for what U-Boot loads.
#
# "run" does, in order, and stops at the first failure:
#   1. checks the image on the host: the AArch64 Image header ("ARMd" at byte
#      56), image_size inside the window, the file below 255 MiB, the watchdog
#      marker "gc_wdog_arm: armed" (MP2_NO_WATCHDOG=1 overrides: BL2's IWDG,
#      about 32 s, is then the only bound of a hang); builds and reads back
#      the script;
#   2. takes the board lock; checks ssh, no session from another machine,
#      /boot on the SD card's bootfs, the console free (BOARD_UART, a probe),
#      no staged file left by an earlier run, the free space, and reads the
#      stored environment with fw_printenv (all variables): a "Bad CRC" is a
#      refusal (ST's env_check would store the RAM environment at the next
#      "run bootcmd"), scriptaddr must be set and clear of the window; records
#      fw_printenv and the sha256 of every stock file of /boot (the "before");
#   3. prints the UNDO, then stages the script and the image;
#   4. starts mp2_uboot_console.py typed (capture and the one line), then
#      reboots the board's Linux; BL2's IWDG ends a hang in about 32 s, the
#      image's own PSCI reset or watchdog ends a run;
#   5. waits for a NEW boot of Linux to answer ssh, <= RUN_BOUND +
#      RETURN_BOUND s (120 + 180); the old boot still answering
#      MP2_REBOOT_GRACE s (90) after the reboot is verdict=not_rebooted;
#   6. removes the staged files (read back gone);
#   7. records fw_printenv and the /boot sha256 again and prints both diffs
#      (expected empty);
#   8. prints the verdict line.
# A board that does not come back within the bound: verdict=not_back, rc 7,
# and a NEEDS A PERSON line. The runner never retries.
#
# The verdict line (board.sh's shape; a run from U-Boot is a row, never DEV):
#   board-stm32mp2-uboot.sh stm32mp257f-dk <name> rc=<rc> verdict=<v>
#       boot=<uboot-mmc|uboot-tftp> row boot_verdict=<scan> result=<pass:<prog>|fail:<prog>|none>
#       env_diff=<empty|changed|absent> boot_diff=<empty|changed|absent> back_after=<s>s|absent
# boot_verdict is mp2_uboot_console.py scan's (done, refused, watchdog_reset,
# not_stopped, no_go, not_started, hang). verdict=done: the scan is done, no
# FAIL line, both diffs empty; failed: a "<prog>: FAIL: " line; env_changed /
# boot_changed: a diff is not empty; else the scan's verdict; not_rebooted;
# not_back.
#
# Exit codes: 0 done; 2 usage; 3 refused before the board was changed; 4 the
# verdict is not done; 5 a board read or the cleanup failed; 6 the stored
# environment or a stock /boot file changed; 7 NEEDS A PERSON (the board did
# not come back); 129, 130, 143 ended by HUP, INT, TERM (after the undo; the
# state file beside the lock says what is left, "disarm" clears it).
#
# Environment (no default for a machine, an address or a console; none is
# written to a file: the records and the messages are scrubbed):
#   BOARD_HOST         user@host of the board's Linux (root on ST's image)
#   BOARD_UART         host:port of the A35 console's serial bridge, or a local
#                      serial device (run needs it: the typed line is the route)
#   BOARD_BOOT         uboot-mmc (default) | uboot-tftp
#   NETBOOT_SERVER     uboot-tftp: the TFTP server's address as U-Boot reaches it
#   NETBOOT_TFTP_ROOT  uboot-tftp: the root directory tools/tftp_oneshot.py serves
#   MP2_SUDO           prefix for root on the board ("" for root, "sudo -n ")
#   MP2_KNOWN_HOSTS    a known_hosts file of its own (read only)
#   MP2_NO_WATCHDOG    1: accept an image without the watchdog marker
#   RUN_BOUND, RETURN_BOUND, MP2_REBOOT_GRACE  seconds (120, 180, 90)
#   MP2U_POLL          seconds between the return polls (5)
#   MP2U_CONSOLE       the console helper's command (default python3 -I
#                      tools/mp2_uboot_console.py; the self-test's fake)
#   LOCK_WAIT, HOLD_MAX  see driver/tools/kexec-zephyr/board_lock.sh
#
# Helpers copied from tools/board-stm32mp2.sh (plan #349's file; it cannot be
# sourced, it dispatches when loaded): fgw, kill_fg, scrub, redact_words,
# redact_run, the strict ssh options, take_lock's use of board_lock.sh, the
# state file beside the lock, the /boot sha256 listing.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
REPO=$(cd "$HERE/.." && pwd)
SELF="$HERE/board-stm32mp2-uboot.sh"
NAME_TOOL=board-stm32mp2-uboot.sh
BOARD=stm32mp257f-dk
TAG="$NAME_TOOL $BOARD"

DRY=0
args=()
for a in "$@"; do
    case "$a" in
    --dry-run) DRY=1 ;;
    *) args+=("$a") ;;
    esac
done
set -- "${args[@]+"${args[@]}"}"
usage() { sed -n '11,15p' "$SELF" >&2; exit 2; }
[ $# -ge 1 ] || usage
VERB=$1
shift

# ---- the board's constants --------------------------------------------------------
LOAD=0xa0000000                   # the image's link address (stm32mp257f_dk zephyr,sram)
LOAD_END=0xb0000000               # 256 MiB after it
MAX_IMG=$((255 * 1024 * 1024))    # refused at this size and above (0xff00000)
MAX_IMG_HEX=0xff00000
MARKER='gc_wdog_arm: armed'
STAGE_DIR=/boot/yakogl            # on bootfs; U-Boot sees it as /yakogl
PREFIX=mp2z-                      # every staged file's name starts with it
BOOT_LABEL=bootfs
EMMC_RE='^/dev/mmcblk0(p[0-9]+)?$'
UB_DIR="$REPO/driver/tools/uboot-mp2"
BOOT=${BOARD_BOOT:-uboot-mmc}
case "$BOOT" in uboot-mmc | uboot-tftp) ;; *) echo "$NAME_TOOL: BOARD_BOOT must be uboot-mmc or uboot-tftp, got '$BOOT'" >&2; exit 2 ;; esac
SU=${MP2_SUDO-}
RUN_BOUND=${RUN_BOUND:-120}
RETURN_BOUND=${RETURN_BOUND:-180}
REBOOT_GRACE=${MP2_REBOOT_GRACE:-90}
POLL=${MP2U_POLL:-5}
for v in RUN_BOUND RETURN_BOUND REBOOT_GRACE POLL; do
    case "${!v}" in '' | *[!0-9]*) echo "$NAME_TOOL: $v must be whole seconds, got '${!v}'" >&2; exit 2 ;; esac
done
WAIT_BOUND=$((RUN_BOUND + RETURN_BOUND))
STAGE_TIMEOUT=900
HOLD_MAX=${HOLD_MAX:-$((WAIT_BOUND + STAGE_TIMEOUT + 300))}
END_RE='^mp2: .*: PSCI SYSTEM_RESET at [0-9]+ ms'
if [ -n "${MP2U_CONSOLE:-}" ]; then CONSOLE=("$MP2U_CONSOLE"); else CONSOLE=(python3 -I "$HERE/mp2_uboot_console.py"); fi
SCAN=(python3 -I "$HERE/mp2_uboot_console.py" scan)

TMP=$(mktemp -d)
# ---- signals: a TERM, INT or HUP ends the script through on_exit at once ----------
# (as tools/board-stm32mp2.sh: bash runs a trap only after the foreground
# child ends, so every ssh and sleep runs in the background and is waited for)
FG=
on_signal() { echo "$NAME_TOOL: SIG$1: ending through the undo" >&2; exit "$2"; }
trap 'on_signal TERM 143' TERM
trap 'on_signal INT 130' INT
trap 'on_signal HUP 129' HUP
fgw() {
    local r=0
    "$@" 0<&0 9>&- &
    FG=$!
    wait "$FG" || r=$?
    FG=
    return "$r"
}
kill_fg() {
    [ -n "${FG:-}" ] || return 0
    pkill -TERM -P "$FG" 2>/dev/null
    kill -TERM "$FG" 2>/dev/null
    FG=
}
trap on_exit EXIT

# ---- messages, scrubbed: no host, address or console path leaves this script -----
BHOSTNAME=
scrub() {
    local e=() pair s esc bh=${BOARD_HOST:-}
    for pair in "$bh|<BOARD_HOST>" "${BOARD_UART:-}|<BOARD_UART>" "${NETBOOT_SERVER:-}|<NETBOOT_SERVER>" \
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
undo() { echo "$TAG: UNDO: $*" | scrub; }   # printed before the action it undoes

SSH_OPTS=(-o StrictHostKeyChecking=yes -o UpdateHostKeys=no -o ConnectTimeout=10 -o BatchMode=yes)
[ -z "${MP2_KNOWN_HOSTS:-}" ] || SSH_OPTS+=(-o "UserKnownHostsFile=$MP2_KNOWN_HOSTS" -o GlobalKnownHostsFile=/dev/null)
b_ssh() { { timeout "${B_TIMEOUT:-60}" ssh "${SSH_OPTS[@]}" "$BOARD_HOST" "$@" 2>&1 1>&3 3>&- | scrub >&2; } 3>&1; }
# b <command>: run on the board, bounded; in --dry-run print it instead
b() {
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $*"; return 0; fi
    fgw b_ssh "$@"
}
# bget VAR CMD [DRY-VALUE]: a board read into VAR (through a file, not $(...))
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
# act <command...>: a host-side action with an effect; printed in --dry-run
act() {
    if [ "$DRY" = 1 ]; then echo "DRY host\$ $*" | scrub; return 0; fi
    "$@" 2> >(scrub >&2)
}
need_board() {
    if [ "$DRY" = 1 ]; then BOARD_HOST=${BOARD_HOST:-user@BOARD}; return; fi
    [ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=user@host of the board's Linux; it comes from the lease, there is no default" 2
}
need_tftp() {
    [ "$BOOT" = uboot-tftp ] || return 0
    if [ "$DRY" = 1 ]; then
        NETBOOT_SERVER=${NETBOOT_SERVER:-SERVER}; NETBOOT_TFTP_ROOT=${NETBOOT_TFTP_ROOT:-/TFTP_ROOT}
        return 0
    fi
    [ -n "${NETBOOT_SERVER:-}" ] || die "uboot-tftp: set NETBOOT_SERVER=the TFTP server's address as U-Boot reaches it" 2
    [ -n "${NETBOOT_TFTP_ROOT:-}" ] && [ -d "$NETBOOT_TFTP_ROOT" ] ||
        die "uboot-tftp: set NETBOOT_TFTP_ROOT=the directory tools/tftp_oneshot.py serves" 2
}
uart_is_bridge() { [[ ${BOARD_UART:-} =~ ^[^/:]+:[0-9]+$ ]]; }

# ---- the image (host only) ---------------------------------------------------------
IMG_SHA= IMG_SIZE= FOOTPRINT= IMG_ZEPHYR=
check_image() {
    local img=$1 magic max
    max=$((LOAD_END - LOAD))
    [ -f "$img" ] || die "no image $img"
    IMG_SIZE=$(stat -c %s "$img")
    # the size first: nothing else is read from an image this large
    [ "$IMG_SIZE" -lt "$MAX_IMG" ] ||
        die "REFUSED: $img is $IMG_SIZE bytes, 255 MiB ($MAX_IMG) or more: the 256 MiB window at $LOAD would not hold it with room to spare; nothing was staged"
    magic=$(od -A n -t x1 -j 56 -N 4 "$img" | tr -d ' \n')
    [ "$magic" = 41524d64 ] || die "$img has no AArch64 Image header ('$magic' at byte 56); build with CONFIG_AARCH64_IMAGE_HEADER=y"
    FOOTPRINT=$(od -A n -t u8 -j 16 -N 8 "$img" | tr -d ' ')
    { [ "$FOOTPRINT" -gt 0 ] && [ "$FOOTPRINT" -le "$max" ]; } ||
        die "image_size $FOOTPRINT does not fit $LOAD..$LOAD_END ($max bytes)"
    if ! grep -aqF "$MARKER" "$img"; then
        [ "${MP2_NO_WATCHDOG:-0}" = 1 ] ||
            die "REFUSED: $img does not contain \"$MARKER\": it would not arm OP-TEE's watchdog itself (MP2_NO_WATCHDOG=1 overrides, deliberately: BL2's IWDG is then the only bound of a hang)"
        echo "$TAG: WARNING: MP2_NO_WATCHDOG=1: this image relies on the IWDG BL2 started" >&2
    fi
    IMG_SHA=$(sha256sum "$img" | cut -d' ' -f1)
    IMG_ZEPHYR=$(grep -aoE 'Booting Zephyr OS build [0-9A-Za-z._+-]+' "$img" | head -1 | sed 's/.* build //')
    IMG_ZEPHYR=${IMG_ZEPHYR:-absent}
}
# build_script: driver/tools/uboot-mp2's script for this backend, built and
# read back (its payload byte for byte the merged .cmd). SCR, SCR_NAME
SCR= SCR_NAME=
build_script() {
    local src
    if [ "$BOOT" = uboot-mmc ]; then SCR_NAME=mp2-mmc; else SCR_NAME=mp2-net; fi
    src="$UB_DIR/$SCR_NAME.cmd"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ driver/tools/uboot-mp2/build.sh <tmp>/scr   # then tools/uboot_script.py check and extract: the payload is $SCR_NAME.cmd byte for byte"
        return 0
    fi
    [ -f "$src" ] || die "no $src"
    SCR="$TMP/scr/$SCR_NAME.scr"
    "$UB_DIR/build.sh" "$TMP/scr" > "$TMP/build.log" 2>&1 || die "driver/tools/uboot-mp2/build.sh failed: $(tr '\n' ';' < "$TMP/build.log")"
    python3 -I "$HERE/uboot_script.py" check "$SCR" > /dev/null 2>&1 || die "the built $SCR_NAME.scr does not check"
    python3 -I "$HERE/uboot_script.py" extract "$SCR" -o "$TMP/scr/payload" > /dev/null 2>&1 &&
        cmp -s "$TMP/scr/payload" "$src" || die "the built $SCR_NAME.scr does not carry $SCR_NAME.cmd byte for byte"
}

# ---- the typed line ------------------------------------------------------------------
# typed_line PART|SERVER TOKEN: the one line mp2_uboot_console.py types at the prompt.
# Only mp2z_* (and, net, autoload as the script does) are set, in RAM; the
# script loads to ${scriptaddr}; whatever happens, "run bootcmd" follows.
typed_line() {
    local where=$1 t=$2
    if [ "$BOOT" = uboot-mmc ]; then
        printf '%s' "setenv mp2z_part $where; setenv mp2z_file /yakogl/$PREFIX$t.bin; if ext4size mmc \${boot_instance}:$where /yakogl/$PREFIX$t.bin && itest \${filesize} -lt $MAX_IMG_HEX; then load mmc \${boot_instance}:$where \${scriptaddr} /yakogl/$PREFIX$t.scr && source \${scriptaddr}; else echo mp2z: refused: /yakogl/$PREFIX$t.bin missing or 255 MiB or more; fi; echo mp2z: back to bootcmd; run bootcmd"
    else
        printf '%s' "setenv mp2z_server $where; setenv mp2z_file yakogl/$PREFIX$t.bin; setenv autoload no; dhcp; tftpboot \${scriptaddr} $where:yakogl/$PREFIX$t.scr && source \${scriptaddr}; echo mp2z: back to bootcmd; run bootcmd"
    fi
}

# ---- the run directory and the record --------------------------------------------------
RD=
meta() { [ "$DRY" = 1 ] || [ -z "$RD" ] || printf '%s=%s\n' "$1" "$2" >> "$RD/meta.txt"; }
mget() { sed -n "s/^$1=//p" "${2:-$RD}/meta.txt" 2>/dev/null | tail -1; }
redact_words() { # redact_words DIR PLACEHOLDER STRING...
    local dir=$1 as=$2 f s esc files=()
    shift 2
    for f in "$dir"/*; do [ -f "$f" ] && files+=("$f"); done
    [ ${#files[@]} -gt 0 ] || return 0
    for s in "$@"; do
        [ ${#s} -ge 6 ] || continue
        esc=$(printf '%s' "$s" | sed 's/[][\\.*^$/|(){}+?]/\\&/g')
        LC_ALL=C sed -i -E "s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1$as\\2/g;
            s/(^|[^A-Za-z0-9_.-])$esc([^A-Za-z0-9_-]|$)/\\1$as\\2/g" "${files[@]}"
    done
}
# the copies never carry a host, an address, a MAC, a serial or the console's path
redact_run() {
    local dir=$1 f n=0 ip files=()
    for f in "$dir"/*; do [ -f "$f" ] && files+=("$f"); done
    [ ${#files[@]} -gt 0 ] || return 0
    LC_ALL=C sed -i -E 's/^([<>+ -]?)((eth[0-9]*addr|serial#|ipaddr|serverip|gatewayip|netmask|hostname|dnsip[0-9]*|board_id)=).*/\1\2<redacted>/' "${files[@]}"
    redact_words "$dir" "<BOARD_UART>" "${BOARD_UART:-}"
    if uart_is_bridge; then redact_words "$dir" "<uart-bridge>" "${BOARD_UART%:*}"; fi
    redact_words "$dir" "<board-host>" "${BHOSTNAME:-}" "${BOARD_HOST#*@}"
    redact_words "$dir" "<NETBOOT_SERVER>" "${NETBOOT_SERVER:-}"
    while read -r ip; do
        n=$((n + 1))
        LC_ALL=C sed -i -E "s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g;
            s/(^|[^0-9.])${ip//./\\.}([^0-9]|$)/\\1192.0.2.$n\\2/g" "${files[@]}"
    done < <(LC_ALL=C grep -aohE '(^|[^0-9.])([0-9]{1,3}\.){3}[0-9]{1,3}([^0-9.]|$)' "${files[@]}" |
        LC_ALL=C grep -aoE '([0-9]{1,3}\.){3}[0-9]{1,3}' | grep -v '^192\.0\.2\.' | awk '!seen[$0]++')
}

# ---- the lock and the state file beside it --------------------------------------------
LOCKED=0
LEFT=
take_lock() {
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ . driver/tools/kexec-zephyr/board_lock.sh   # HOLD_MAX=$HOLD_MAX, one lock per BOARD_HOST"
        return 0
    fi
    export HOLD_MAX
    # shellcheck source=/dev/null
    . "$REPO/driver/tools/kexec-zephyr/board_lock.sh"
    LOCKED=1
    LEFT=${BOARD_LOCK%.lock}.mp2u-left
    trap on_exit EXIT
}
left_write() { # left_write LINE...: written before the change it names
    meta left "$*"
    [ "$DRY" = 0 ] && [ -n "$LEFT" ] || return 0
    { echo "when=$(date '+%F %T')"; printf 'left=%s\n' "$@"; } > "$LEFT.new" && mv "$LEFT.new" "$LEFT"
}
left_clear() { [ "$DRY" = 0 ] && [ -n "$LEFT" ] && rm -f "$LEFT"; return 0; }
left_show() {
    [ "$DRY" = 0 ] && [ -n "$LEFT" ] && [ -s "$LEFT" ] || return 0
    { echo "$TAG: an earlier run left this record:"; sed 's/^/  /' "$LEFT"; } | scrub >&2
}

# ---- what a run owns, and its undo on exit ------------------------------------------------
TYPER=          # the console helper's pid
BOARD_FILES=()  # staged on the card (full paths), while there
TFTP_FILES=()   # staged in the TFTP root, while there
MADE_DIR=0      # 1: this run made /boot/yakogl
WENT=0          # 1: the board was seen to leave its Linux
BACK=0          # 1: a new boot answered
RUN_OWNS_LEFT=0 # 1: the board showed nothing of an earlier run: this run owns the state file
KEEP_LEFT=0     # 1: the state file names what a person must undo (not_back)
stop_typer() { if [ -n "${TYPER:-}" ]; then kill "$TYPER" 2>/dev/null; wait "$TYPER" 2>/dev/null; TYPER=; fi; }
remove_tftp() {
    local f
    [ ${#TFTP_FILES[@]} -gt 0 ] || return 0
    for f in "${TFTP_FILES[@]}"; do rm -f "$f" "$f.served" "$f.part"; done
    for f in "${TFTP_FILES[@]}"; do [ ! -e "$f" ] && [ ! -e "$f.served" ] || return 1; done
    TFTP_FILES=()
}
remove_board() { # the staged files off the card, read back gone
    local f list=() chk
    [ ${#BOARD_FILES[@]} -gt 0 ] || return 0
    for f in "${BOARD_FILES[@]}"; do list+=("$f" "$f.part"); done
    chk="${SU}rm -f ${list[*]} && ! ls ${list[*]} 2>/dev/null"
    [ "$MADE_DIR" = 1 ] && chk+=" && { ${SU}rmdir $STAGE_DIR 2>/dev/null; true; }"
    b "$chk" || return 1
    BOARD_FILES=()
}
on_exit() {
    local rc=$? left=()
    trap '' TERM INT HUP
    kill_fg
    stop_typer
    if [ "$DRY" = 0 ] && { [ "$VERB" = run ] || [ "$VERB" = stage ]; }; then
        if [ "$VERB" = run ] || [ "$rc" != 0 ]; then
            remove_tftp || left+=("TFTP root files ${TFTP_FILES[*]}")
            if [ ${#BOARD_FILES[@]} -gt 0 ]; then
                if { [ "$WENT" = 0 ] || [ "$BACK" = 1 ]; } && remove_board; then
                    meta cleanup "removed on exit"
                else
                    left+=("on the card: ${BOARD_FILES[*]}")
                fi
            fi
        fi
        if [ ${#left[@]} -gt 0 ]; then
            left_write "${left[@]}"
            { printf '%s: WARNING: left behind: %s\n' "$TAG" "${left[@]}"
              echo "$TAG: the undo, once the board answers: tools/$NAME_TOOL disarm"; } | scrub >&2
        elif [ "$VERB" = run ] && [ "$RUN_OWNS_LEFT" = 1 ] && [ "$KEEP_LEFT" = 0 ]; then
            left_clear
        fi
    fi
    if [ "$LOCKED" = 1 ] && [ "$DRY" = 0 ]; then
        [ -n "${BOARD_LOCK_WATCHDOG:-}" ] && kill "$BOARD_LOCK_WATCHDOG" 2>/dev/null
        : > "$BOARD_LOCK"
        [ -n "${_wd_log:-}" ] && [ -s "$_wd_log" ] && cat "$_wd_log" >&2
    fi
    rm -rf "$TMP"
    exit "$rc"
}

# ---- the pre-checks --------------------------------------------------------------------
BOOT_DEV= BOOT_PART= CARD_DISK=
prechecks() {
    local who mine others junk m fstype dtype label
    bget junk "${SU}true" || die "the board does not answer (ssh key login${SU:+ and $SU}; the host key must be known)"
    bget BHOSTNAME 'uname -n' DRY-HOSTNAME || die "cannot read the board's host name"
    bget who 'who' '' || die "cannot run who on the board"
    mine=$(ip -o addr 2>/dev/null | awk '{sub(/\/.*/, "", $4); print $4}')
    # shellcheck disable=SC2086
    others=$(awk '/\(.*\)/ { a = $NF; gsub(/[()]/, "", a); print a }' <<<"$who" | sort -u |
        grep -vxF -f <(printf '%s\n' $mine) || true)
    [ -z "$others" ] || die "the board has sessions from other machines; ask before taking it"
    # /boot: ext4 on a partition labelled bootfs of the SD card the board booted from
    bget m 'set -- $(findmnt -n -v -o SOURCE,FSTYPE -T /boot); d=$1; p=${d%p*}; echo "$d $2 $p $(cat /sys/block/${p##*/}/device/type 2>/dev/null) $(lsblk -rno PARTLABEL "$d" 2>/dev/null)"' \
        '/dev/mmcblkDp8 ext4 /dev/mmcblkD SD bootfs' || die "cannot read the mount of /boot"
    read -r BOOT_DEV fstype CARD_DISK dtype label <<<"$m"
    [ "$fstype" = ext4 ] || die "/boot is '$fstype', not ext4 (U-Boot's ext4size/ext4load read bootfs)"
    [[ $CARD_DISK =~ $EMMC_RE ]] && die "REFUSED: /boot is on the eMMC (mmcblk0): this runner stages on the microSD only"
    [ "$dtype" = SD ] || die "REFUSED: /boot's disk says '${dtype:-unreadable}', not SD: this runner stages on the microSD only"
    [ "$label" = "$BOOT_LABEL" ] || die "REFUSED: /boot's partition is labelled '${label:-nothing}', not $BOOT_LABEL"
    [[ $BOOT_DEV =~ p([0-9]+)$ ]] || die "/boot's source is not a partition"
    BOOT_PART=${BASH_REMATCH[1]}
}
# env_read FILE: fw_printenv, all variables, into FILE; refuses a bad CRC and
# a scriptaddr that is unset or not clear of the window
ENV_DRY='bootcmd=run bootcmd_stm32mp
bootdelay=1
scriptaddr=0x90100000'
env_read() {
    local out=$1 sa
    if [ "$DRY" = 1 ]; then
        echo "DRY board\$ ${SU}fw_printenv 2>&1   # > $out (every variable)"
        printf '%s\n' "$ENV_DRY" > "$TMP/env.dry"
        out=$TMP/env.dry
    else
        B_TIMEOUT=60 fgw b_ssh "${SU}fw_printenv 2>&1; echo \"fw_printenv:rc=\$?\"" > "$out" 2>/dev/null ||
            die "cannot read the environment (fw_printenv over ssh failed)" 5
        grep -qx 'fw_printenv:rc=0' "$out" || die "fw_printenv failed on the board: $(tr '\n' ';' < "$out" | cut -c1-200)" 5
        sed -i '/^fw_printenv:rc=/d' "$out"
    fi
    grep -qi 'bad crc' "$out" &&
        die "REFUSED: fw_printenv reports a bad CRC: U-Boot runs on its defaults and ST's env_check would store the RAM environment at the next 'run bootcmd'"
    grep -q '^bootcmd=' "$out" || die "the stored environment has no bootcmd: not ST's environment as expected"
    sa=$(sed -n 's/^scriptaddr=//p' "$out" | head -1)
    [[ $sa =~ ^(0x)?[0-9a-fA-F]+$ ]] || die "REFUSED: scriptaddr is '${sa:-unset}' in the stored environment: the typed line loads the script to \${scriptaddr}"
    [[ $sa == 0x* ]] || sa=0x$sa
    { [ $((sa + 0x100000)) -le $((LOAD)) ] || [ $((sa)) -ge $((LOAD_END)) ]; } ||
        die "REFUSED: scriptaddr $sa is inside or next to the go window $LOAD..$LOAD_END"
    SCRIPTADDR=$sa
}
SCRIPTADDR=
BOOT_LIST="cd /boot && find . -path ./yakogl -prune -o -type f -print0 | sort -z | xargs -0 ${SU}sha256sum"
boot_sha() { # boot_sha FILE: the sha256 of every stock file of /boot (yakogl/ excluded)
    if [ "$DRY" = 1 ]; then echo "DRY board\$ $BOOT_LIST   # > $1"; return 0; fi
    B_TIMEOUT=120 fgw b_ssh "$BOOT_LIST" > "$1" 2>/dev/null && [ -s "$1" ] || die "cannot list /boot's sha256" 5
}
console_probe() {
    if [ "$DRY" = 1 ]; then echo "DRY host\$ ${CONSOLE[*]##*/} probe <BOARD_UART>   # must be free"; return 0; fi
    fgw "${CONSOLE[@]}" probe "$BOARD_UART" 2> >(scrub >&2) || die "the console BOARD_UART is busy or unreachable (nothing was changed)"
}
our_leftovers() { # LEFTOVER: our staged files on the card, if any
    bget LEFTOVER "ls -d $STAGE_DIR/$PREFIX* 2>/dev/null; true" '' || die "cannot list $STAGE_DIR on the board" 5
}
tftp_canary() {
    [ "$BOOT" = uboot-tftp ] || return 0
    # read-only use of the library: sourced in a subshell for this one function
    ( NB_TAG=$TAG NB_RECORDS=records/stm32mp257f-dk/netboot NB_WATCHDOG="OP-TEE's watchdog" NB_MMC=- YAKO_ADDR=$LOAD
      # shellcheck source=/dev/null
      . "$HERE/uboot-netboot-lib.sh"
      nb_tftp_canary "$NETBOOT_SERVER" "$NETBOOT_TFTP_ROOT" "$TOKEN" ) 2>&1 | scrub
    [ "${PIPESTATUS[0]}" = 0 ] || die "the TFTP server does not serve NETBOOT_TFTP_ROOT (start tools/tftp_oneshot.py --root <root>)"
}

# ---- staging --------------------------------------------------------------------------
TOKEN=
stage_files() { # stage_files IMAGE: the script and the image, read back by sha256
    local img=$1 scr_sha got d f
    if [ "$DRY" = 1 ]; then scr_sha=SCRIPT_SHA256; else scr_sha=$(sha256sum "$SCR" | cut -d' ' -f1); fi
    if [ "$BOOT" = uboot-tftp ]; then
        d=$NETBOOT_TFTP_ROOT/yakogl
        undo "host\$ rm -f $d/$PREFIX$TOKEN.scr* $d/$PREFIX$TOKEN.bin*   (or: tools/$NAME_TOOL disarm)"
        TFTP_FILES=("$d/$PREFIX$TOKEN.scr" "$d/$PREFIX$TOKEN.bin")
        left_write "staging in the TFTP root: ${TFTP_FILES[*]}"
        act mkdir -p "$d" || die "cannot create $d"
        act cp "${SCR:-<tmp>/scr/mp2-net.scr}" "$d/$PREFIX$TOKEN.scr" || die "cannot stage the script"
        act cp "$img" "$d/$PREFIX$TOKEN.bin" || die "cannot stage the image"
        if [ "$DRY" = 1 ]; then echo "DRY host\$ sha256sum $d/$PREFIX$TOKEN.*   # must be the script's and the image's"; return 0; fi
        [ "$(sha256sum "$d/$PREFIX$TOKEN.scr" | cut -d' ' -f1)" = "$scr_sha" ] &&
            [ "$(sha256sum "$d/$PREFIX$TOKEN.bin" | cut -d' ' -f1)" = "$IMG_SHA" ] || die "the staged copies read back different"
        return 0
    fi
    undo "board\$ ${SU}rm -f $STAGE_DIR/$PREFIX$TOKEN.scr $STAGE_DIR/$PREFIX$TOKEN.bin $STAGE_DIR/$PREFIX$TOKEN.*.part   (or: tools/$NAME_TOOL disarm)"
    bget d "test -d $STAGE_DIR && echo had || echo new" new || die "cannot look at $STAGE_DIR"
    [ "$d" = new ] && MADE_DIR=1
    BOARD_FILES=("$STAGE_DIR/$PREFIX$TOKEN.scr" "$STAGE_DIR/$PREFIX$TOKEN.bin")
    left_write "staging on the card: ${BOARD_FILES[*]}"
    for f in scr bin; do
        local src=$img
        [ "$f" = scr ] && src=${SCR:-<tmp>/scr/mp2-mmc.scr}
        if [ "$DRY" = 1 ]; then
            echo "DRY board\$ ${SU}sh -c 'mkdir -p $STAGE_DIR && cat > $STAGE_DIR/$PREFIX$TOKEN.$f.part && sync && mv $STAGE_DIR/$PREFIX$TOKEN.$f.part $STAGE_DIR/$PREFIX$TOKEN.$f && sync' < $src"
        else
            fgw timeout "$STAGE_TIMEOUT" ssh "${SSH_OPTS[@]}" "$BOARD_HOST" \
                "${SU}sh -c 'mkdir -p $STAGE_DIR && cat > $STAGE_DIR/$PREFIX$TOKEN.$f.part && sync && mv $STAGE_DIR/$PREFIX$TOKEN.$f.part $STAGE_DIR/$PREFIX$TOKEN.$f && sync'" < "$src" ||
                die "copying $PREFIX$TOKEN.$f to $STAGE_DIR failed (removed on exit)"
        fi
    done
    bget got "echo 3 | ${SU}tee /proc/sys/vm/drop_caches >/dev/null && cd $STAGE_DIR && sha256sum $PREFIX$TOKEN.scr $PREFIX$TOKEN.bin" \
        "$scr_sha  $PREFIX$TOKEN.scr
$IMG_SHA  $PREFIX$TOKEN.bin" || got=
    [ "$DRY" = 1 ] && return 0
    printf '%s  %s\n%s  %s\n' "$scr_sha" "$PREFIX$TOKEN.scr" "$IMG_SHA" "$PREFIX$TOKEN.bin" | cmp -s - <(printf '%s\n' "$got") ||
        die "the card's copies read back different (removed on exit)"
}
make_token() { # make_token NAME
    TOKEN=$(printf '%s' "$1-${IMG_SHA:0:12}-$(date +%s)" | tr -c 'A-Za-z0-9_-' '_')
}
where_arg() { if [ "$BOOT" = uboot-mmc ]; then echo "$BOOT_PART"; else echo "$NETBOOT_SERVER"; fi; }

# ---- status / stage / disarm ------------------------------------------------------------
cmd_status() {
    need_board
    need_tftp
    prechecks
    env_read "$TMP/env.now"
    echo "boot: $BOOT; bootfs $BOOT_DEV (U-Boot part $BOOT_PART); scriptaddr $SCRIPTADDR; go window $LOAD..$LOAD_END"
    b "${SU}fw_printenv bootcmd bootdelay boot_targets scriptaddr 2>&1; ls -l $STAGE_DIR 2>/dev/null; df -B1 --output=avail /boot | tail -1; cut -d' ' -f1 /proc/uptime"
    our_leftovers
    if [ -n "$LEFTOVER" ]; then echo "staged files of this runner: $(tr '\n' ' ' <<<"$LEFTOVER")"; else echo "staged files of this runner: none on the card"; fi
    if [ "$BOOT" = uboot-tftp ]; then
        if [ "$DRY" = 1 ]; then echo "DRY host\$ ls $NETBOOT_TFTP_ROOT/yakogl/$PREFIX*"
        else echo "in the TFTP root: $(ls "$NETBOOT_TFTP_ROOT/yakogl/" 2>/dev/null | grep "^$PREFIX" | tr '\n' ' ')"; fi
    fi
    if [ "$DRY" = 0 ]; then
        # shellcheck source=/dev/null
        BOARD_LOCK_NO_ACQUIRE=1 . "$REPO/driver/tools/kexec-zephyr/board_lock.sh" && LEFT=${BOARD_LOCK%.lock}.mp2u-left
        if [ -n "$LEFT" ] && [ -s "$LEFT" ]; then left_show 2>&1; else echo "left behind: nothing recorded"; fi
    fi
    undo "of anything staged: tools/$NAME_TOOL disarm   # removes $STAGE_DIR/$PREFIX* (and the TFTP root's yakogl/$PREFIX*), nothing else"
    echo "the stored environment and bootcmd are never written by this runner"
}
cmd_stage() {
    [ $# -ge 1 ] || usage
    local img=$1
    check_image "$img"
    need_board
    need_tftp
    build_script
    take_lock
    prechecks
    env_read "$TMP/env.now"
    our_leftovers
    [ -z "$LEFTOVER" ] || die "$STAGE_DIR holds files of an earlier run: 'disarm' first"
    make_token "$(basename "$img" .bin | tr -c 'A-Za-z0-9_-' '_')"
    tftp_canary
    stage_files "$img"
    echo "staged; a person types at ST's prompt (autoboot stopped), once:"
    echo "  $(typed_line "$(where_arg)" "$TOKEN")" | scrub
    undo "tools/$NAME_TOOL disarm   # removes the staged files; nothing else was written"
}
cmd_disarm() {
    need_board
    take_lock
    left_show
    our_leftovers
    if [ -n "$LEFTOVER" ] || [ "$DRY" = 1 ]; then
        echo "$TAG: removing this runner's staged files: $(tr '\n' ' ' <<<"${LEFTOVER:-$STAGE_DIR/$PREFIX*}")" | scrub
        b "${SU}rm -f $STAGE_DIR/$PREFIX* && ! ls -d $STAGE_DIR/$PREFIX* 2>/dev/null && { ${SU}rmdir $STAGE_DIR 2>/dev/null; true; }" || die "cannot remove the staged files" 5
    fi
    if [ -n "${NETBOOT_TFTP_ROOT:-}" ] || [ "$DRY" = 1 ]; then
        local root=${NETBOOT_TFTP_ROOT:-/TFTP_ROOT}
        if [ "$DRY" = 1 ]; then echo "DRY host\$ rm -f $root/yakogl/$PREFIX*"
        else rm -f "$root/yakogl/$PREFIX"*; ! ls "$root/yakogl/$PREFIX"* >/dev/null 2>&1 || die "cannot remove the TFTP root's files" 5; fi
    fi
    left_clear
    note "disarmed: no staged file of this runner left (the environment was never written)"
}

# ---- run ------------------------------------------------------------------------------------
verdict_line() { # verdict_line DIR: V_LINE, V_RC
    local dir=$1 bv res v rc ed bd after l t0 t1
    bv=$(mget scan "$dir"); bv=${bv:-absent}
    res=none
    if [ -s "$dir/console.ts" ]; then
        l=$(tr -d '\0\r' < "$dir/console.ts" | sed -E 's/^[0-9.]+ //' | grep -aoE '^[A-Za-z0-9_]+: (PASS|FAIL): ' | tail -1)
        if [ -n "$l" ]; then case "$l" in *PASS*) res="pass:${l%%:*}" ;; *) res="fail:${l%%:*}" ;; esac; fi
    fi
    ed=$(mget env_diff "$dir"); ed=${ed:-absent}
    bd=$(mget boot_diff "$dir"); bd=${bd:-absent}
    t0=$(mget t_reboot "$dir"); t1=$(mget t_back "$dir")
    if [ "$(mget back "$dir")" = 1 ] && [ -n "$t0" ] && [ -n "$t1" ]; then after="$((t1 - t0))s"; else after=absent; fi
    if [ "$(mget rebooted "$dir")" = 0 ]; then v=not_rebooted; rc=4; after=absent
    elif [ "$(mget back "$dir")" != 1 ]; then v=not_back; rc=7
    elif [ "$ed" != empty ]; then v=env_changed; rc=6
    elif [ "$bd" != empty ]; then v=boot_changed; rc=6
    elif [ "$(mget cleanup "$dir")" != removed ]; then v=not_cleaned; rc=5
    elif [ "${res%%:*}" = fail ]; then v=failed; rc=4
    elif [ "$bv" = done ]; then v=done; rc=0
    else v=$bv; rc=4; fi
    V_LINE="$TAG $(mget name "$dir") rc=$rc verdict=$v boot=$BOOT row boot_verdict=$bv result=$res env_diff=$ed boot_diff=$bd back_after=$after"
    V_RC=$rc
}
cmd_run() {
    [ $# -ge 2 ] || usage
    local img=$1 logdir=$2 name=${3:-} line up el down=0 rcc="" st
    [ -n "$name" ] || name=$(basename "$img" .bin)
    [[ $name =~ ^[A-Za-z0-9_-]+$ ]] || die "name '$name': letters, digits, _ and - only" 2
    # 1. the image, on the host, before anything else
    check_image "$img"
    need_board
    need_tftp
    if [ -z "${BOARD_UART:-}" ]; then
        if [ "$DRY" = 1 ]; then BOARD_UART=BRIDGE:PORT
        else die "set BOARD_UART=host:port of the console's serial bridge, or a local serial device: the typed line is the route" 2; fi
    fi
    build_script
    RD="$logdir/$(date +%Y%m%d-%H%M%S)-$name-${IMG_SHA:0:8}"
    if [ "$DRY" = 1 ]; then echo "DRY host\$ mkdir -p $RD   # meta.txt, console.raw, console.ts, typed.log, env-*.txt, boot-sha256*.txt, *.diff"
    else
        [ -e "$RD" ] && die "$RD exists: a record is never overwritten"
        mkdir -p "$RD" || die "cannot create $RD"
    fi
    meta board "$BOARD"; meta boot "$BOOT"; meta name "$name"; meta kind row
    meta image "$(basename "$img")"; meta image_sha256 "$IMG_SHA"; meta image_size "$IMG_SIZE"
    meta footprint "$FOOTPRINT"; meta image_zephyr "$IMG_ZEPHYR"; meta script "$SCR_NAME"
    meta started "$(date '+%F %T')"; meta wait_bound "$WAIT_BOUND"
    note "$name: build $(git -C "$REPO" describe --always --dirty 2>/dev/null || echo unknown) boot $BOOT image sha256 ${IMG_SHA:0:16} size $IMG_SIZE footprint $FOOTPRINT"
    # 2. the lock, the pre-checks, the "before"
    console_probe
    take_lock
    left_show
    prechecks
    our_leftovers
    [ -z "$LEFTOVER" ] || die "$STAGE_DIR holds files of an earlier run that did not clean up: 'disarm' first"
    RUN_OWNS_LEFT=1
    bget st "df -B1 --output=avail /boot | tail -1" 600000000 || die "df on the board failed"
    st=${st// /}
    [[ $st =~ ^[0-9]+$ ]] && [ "$st" -ge $((2 * IMG_SIZE + 1048576)) ] || [ "$BOOT" = uboot-tftp ] ||
        die "/boot has '$st' bytes free, less than twice the image ($IMG_SIZE) and 1 MiB"
    make_token "$name"
    meta token "$TOKEN"
    tftp_canary
    env_read "${RD:-<rundir>}/env-before.txt"
    meta scriptaddr "$SCRIPTADDR"
    boot_sha "${RD:-<rundir>}/boot-sha256.txt"
    line=$(typed_line "$(where_arg)" "$TOKEN")
    # 3. the undo, then the staging
    undo "should this run be cut off, its exit removes what it staged while the board's Linux answers; what it cannot is recorded beside the lock and 'tools/$NAME_TOOL disarm' removes it. The environment is never written: there is nothing to put back in it."
    stage_files "$img"
    meta staged "$PREFIX$TOKEN.scr $PREFIX$TOKEN.bin, read back equal"
    # 4. the typed one-shot, then the reboot
    undo "of the typed line: nothing persistent (mp2z_* in RAM; it ends in 'run bootcmd'). A prompt left behind: type 'run bootcmd' or 'reset' at it (neither stores anything). A hang: BL2's IWDG resets the board in about 32 s."
    echo "$TAG: typed line: $line" | scrub
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ ${CONSOLE[*]##*/} typed <BOARD_UART> <rundir>/console.raw <rundir>/console.ts '<the typed line>' 2> <rundir>/typed.log &"
    else
        "${CONSOLE[@]}" typed "$BOARD_UART" "$RD/console.raw" "$RD/console.ts" "$line" 2> >(scrub > "$RD/typed.log") 9>&- &
        TYPER=$!
        fgw sleep 2
        kill -0 "$TYPER" 2>/dev/null || { TYPER=; die "the console helper ended at once (console busy or unreachable); the staged files are removed, nothing was rebooted"; }
    fi
    left_write "rebooting into the typed one-shot; staged: ${BOARD_FILES[*]}${TFTP_FILES[*]}"
    B_TIMEOUT=30 b "sync; ${SU}systemctl reboot" || true
    T_REBOOT=$(date +%s)
    meta t_reboot "$T_REBOOT"
    if [ "$DRY" = 1 ]; then
        echo "DRY host\$ wait until ssh answers from a new boot (uptime < time since the reboot), <= $WAIT_BOUND s; the old boot still answering after $REBOOT_GRACE s: not_rebooted; no answer: not_back, rc 7, NEEDS A PERSON, no retry"
        echo "DRY host\$ stop the console helper; python3 tools/mp2_uboot_console.py scan <rundir>/console.ts   # boot_verdict"
        if [ "$BOOT" = uboot-mmc ]; then
            b "${SU}rm -f $STAGE_DIR/$PREFIX$TOKEN.scr $STAGE_DIR/$PREFIX$TOKEN.bin ... && ! ls ...   # the staged files, read back gone"
        else
            echo "DRY host\$ rm -f $NETBOOT_TFTP_ROOT/yakogl/$PREFIX$TOKEN.*   # the staged files (and .served)"
        fi
        env_read "<rundir>/env-after.txt"
        boot_sha "<rundir>/boot-sha256.after.txt"
        echo "DRY host\$ diff <rundir>/env-before.txt <rundir>/env-after.txt   # must be empty"
        echo "DRY host\$ diff <rundir>/boot-sha256.txt <rundir>/boot-sha256.after.txt   # must be empty"
        echo "DRY host\$ the verdict line: $TAG $name rc=<rc> verdict=<v> boot=$BOOT row boot_verdict=<scan> result=<r> env_diff=<empty|changed> boot_diff=<empty|changed> back_after=<s>s"
        BOARD_FILES=(); TFTP_FILES=()
        exit 0
    fi
    # 5. wait for a new boot of Linux
    while :; do
        fgw sleep "$POLL"
        el=$(($(date +%s) - T_REBOOT))
        fgw timeout 8 ssh "${SSH_OPTS[@]}" -o ConnectTimeout=5 "$BOARD_HOST" 'cut -d. -f1 /proc/uptime' > "$TMP/up" 2>/dev/null
        up=$(cat "$TMP/up")
        if [[ $up =~ ^[0-9]+$ ]]; then
            if [ "$up" -lt "$el" ]; then WENT=1; BACK=1; break; fi
            if [ "$down" = 0 ] && [ "$el" -gt "$REBOOT_GRACE" ]; then rcc=not_rebooted; BACK=1; break; fi
        elif [ "$down" = 0 ]; then
            down=1; WENT=1; meta t_down "$(date +%s)"
        fi
        if [ "$el" -gt "$WAIT_BOUND" ]; then rcc=not_back; break; fi
    done
    meta t_back "$(date +%s)"
    fgw sleep 1
    stop_typer
    if [ -s "$RD/console.ts" ]; then
        st=$("${SCAN[@]}" "$RD/console.ts" 2>/dev/null | head -1)
        meta scan_line "$st"
        st=$(sed -n 's/.*verdict=\([a-z_]*\).*/\1/p' <<<"$st")
        meta scan "${st:-absent}"
    else
        meta scan no_lines
    fi
    if [ "$rcc" = not_back ]; then
        meta back 0
        meta cleanup "not done: Linux not back"
        left_write "the board did not come back; staged: ${BOARD_FILES[*]}${TFTP_FILES[*]}"
        remove_tftp && meta tftp_cleanup removed
        redact_run "$RD"
        verdict_line "$RD"
        echo "$V_LINE"
        echo "$TAG: NEEDS A PERSON: the board did not answer ssh from a new boot within ${WAIT_BOUND}s of the reboot. If it sits at ST's prompt, 'run bootcmd' or 'reset' there (neither stores anything); if it is dark, check the USB-C supply first. This runner never retries; once it is back, 'tools/$NAME_TOOL disarm' removes the staged files." | scrub
        BOARD_FILES=()   # recorded in the state file; nothing reaches the card now
        KEEP_LEFT=1
        exit "$V_RC"
    fi
    meta back 1
    [ "$rcc" = not_rebooted ] && meta rebooted 0
    # 6. the staged files go
    if remove_tftp && remove_board; then meta cleanup removed; else meta cleanup "NOT removed"; fi
    # 7. the "after", and the diffs
    env_read "$TMP/env-after.txt"
    boot_sha "$TMP/boot-sha256.after.txt"
    cp "$TMP/env-after.txt" "$RD/env-after.txt"; cp "$TMP/boot-sha256.after.txt" "$RD/boot-sha256.after.txt"
    if diff "$RD/env-before.txt" "$RD/env-after.txt" > "$RD/env.diff"; then meta env_diff empty; else meta env_diff changed; fi
    if diff "$RD/boot-sha256.txt" "$RD/boot-sha256.after.txt" > "$RD/boot.diff"; then meta boot_diff empty; else meta boot_diff changed; fi
    meta ended "$(date '+%F %T')"
    redact_run "$RD"
    echo "$TAG: fw_printenv before/after diff ($(mget env_diff)):"; sed 's/^/  /' "$RD/env.diff"
    echo "$TAG: /boot sha256 before/after diff ($(mget boot_diff)):"; sed 's/^/  /' "$RD/boot.diff"
    verdict_line "$RD"
    echo "$V_LINE"
    note "record $RD"
    exit "$V_RC"
}

case "$VERB" in
selftest) exec bash "$HERE/board_stm32mp2_uboot_selftest.sh" ;;
status) cmd_status "$@" ;;
stage) cmd_stage "$@" ;;
run) cmd_run "$@" ;;
disarm) cmd_disarm "$@" ;;
*) usage ;;
esac
