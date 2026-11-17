#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_session.sh - the STM32MP257F-DK's standalone board session
# `default <image> 2` as one driver (docs/notes/stm32mp257f-dk-standalone.md,
# "Board session procedure"). It stops at the first failing step.
#
#   tools/mp2s_session.sh [--dry-run] IMAGE
#
# Steps, one line each (`mp2s-session: step <name>: ok`, or `FAIL`):
#   precheck  fw_printenv read from the board; mp2s_guard.py --for standalone;
#             mp2s_env_capacity.py on the two edits `default` makes;
#             mp2s_image_check.py IMAGE with the free space of /boot
#   capture   sha256 of every file under /boot, before
#   console   the passive console watch, started before anything reboots
#   runner    install, default IMAGE 2 (reboots nothing), then the reboot
#   back      the watch ends; mp2s_comeback.sh: OpenSTLinux answers again
#   record    mp2s_console_watch.py scan, mp2s_record_check.py
#   undo      nodefault, uninstall; env and /boot read again; mp2s_env_diff.py,
#             mp2s_guard.py --for oneshot, /boot byte for byte as before
# The last line is `mp2s-session: PASS` or `mp2s-session: FAIL <step>`.
# Stop rules (the note's step 8): nothing is retried, no reset resource is
# used, `forever` is never run. A failure before the reboot, or after the
# board answered again, runs nodefault and uninstall. A board that did not
# come back is NOT touched (it may be mid-boot): the output then says NEEDS A
# PERSON, with tools/mp2s_card_undo.sh as the last resort, and a person checks
# the USB-C supply first.
# Records go to records/stm32mp257f-dk/<YYYYMMDD-HHMM>-standalone-<sha8>/.
#
# Environment (nothing about a board is kept in this file):
#   BOARD_HOST   user@host of the board's Linux (the lease)
#   BOARD_UART   HOST:PORT or device of the console (passive reads only)
#   MP2S_SSH     ssh command (default: ssh -o BatchMode=yes -o ConnectTimeout=10)
#   MP2S_RUNNER  the runner (default: tools/board-stm32mp2-standalone.sh)
#   MP2S_WATCH   the console watch command (default: python3 -I
#                tools/mp2s_console_watch.py), called as
#                `watch UART RAW STAMPED --boots 2 --timeout S --linger 60`
#   MP2S_TIMEOUT watch timeout in seconds (default 900)
#   MP2S_BACK    seconds mp2s_comeback.sh waits for ssh (default 300)
#   MP2S_RECORDS records root (default: records/stm32mp257f-dk)
#   MP2_SUDO     root prefix on the board (default "sudo -n ")
# MP2_NO_WATCHDOG defaults to 1 for the runner: mp2_hello has no marker.
# BOARD_PERSON_PRESENT is removed from the environment of every child.
# Exit: 0 PASS, 1 FAIL <step>, 2 usage. Selftest: tools/mp2s_session_selftest.sh.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
P="mp2s-session:"
COUNT=2
USAGE="usage: tools/mp2s_session.sh [--dry-run] IMAGE"

DRY=0
IMG=
for a in "$@"; do
    case "$a" in
    --dry-run) DRY=1 ;;
    -*) echo "$P $USAGE" >&2; exit 2 ;;
    *) if [ -z "$IMG" ]; then IMG=$a; else echo "$P $USAGE" >&2; exit 2; fi ;;
    esac
done
[ -n "$IMG" ] || { echo "$P $USAGE" >&2; exit 2; }

read -ra SSH <<<"${MP2S_SSH:-ssh -o BatchMode=yes -o ConnectTimeout=10}"
read -ra RUNNER <<<"${MP2S_RUNNER:-$HERE/board-stm32mp2-standalone.sh}"
read -ra WATCH <<<"${MP2S_WATCH:-python3 -I $HERE/mp2s_console_watch.py}"
TIMEOUT=${MP2S_TIMEOUT:-900}
BACK=${MP2S_BACK:-300}
DEFAULT_SUDO='sudo -n '
SUDO=${MP2_SUDO-$DEFAULT_SUDO}
export MP2_NO_WATCHDOG=${MP2_NO_WATCHDOG:-1}
unset BOARD_PERSON_PRESENT

if [ "$DRY" = 1 ]; then
    BOARD_HOST=${BOARD_HOST:-user@BOARD}
    BOARD_UART=${BOARD_UART:-BOARD_UART}
else
    [ -n "${BOARD_HOST:-}" ] || { echo "$P set BOARD_HOST (from the lease; no default)" >&2; exit 2; }
    [ -n "${BOARD_UART:-}" ] || { echo "$P set BOARD_UART (from the lease; no default)" >&2; exit 2; }
    [ -r "$IMG" ] || { echo "$P cannot read $IMG" >&2; exit 2; }
fi
export BOARD_HOST

if [ "$DRY" = 1 ]; then SHA8=XXXXXXXX; else SHA8=$(sha256sum "$IMG" | cut -c1-8); fi
REC=${MP2S_RECORDS:-$ROOT/records/stm32mp257f-dk}/$(date +%Y%m%d-%H%M)-standalone-$SHA8
RAW=$REC/raw.log
STAMPED=$REC/console.log
WPID=
UNDO_OK=0   # 1: the board is in Linux with something staged, so a failure runs the undo

say() { echo "$P $*"; }
# run CMD...: print it; run it unless --dry-run.
run() { echo "+ $*"; if [ "$DRY" = 1 ]; then return 0; fi; "$@"; }
# cap FILE CMD...: print it with its redirect; run it unless --dry-run.
cap() {
    local f=$1; shift
    echo "+ $* > $f"
    if [ "$DRY" = 1 ]; then return 0; fi
    "$@" > "$f"
}
capenv() { cap "$1" "${SSH[@]}" "$BOARD_HOST" "${SUDO}fw_printenv"; }
capman() { cap "$1" "${SSH[@]}" "$BOARD_HOST" "find /boot -type f -print0 | sort -z | xargs -0 ${SUDO}sha256sum"; }

undo() {
    say "undo: nodefault, uninstall"
    "${RUNNER[@]}" nodefault >/dev/null 2>&1
    "${RUNNER[@]}" uninstall >/dev/null 2>&1
}

fail() {
    say "step $1: FAIL"
    if [ -n "$WPID" ]; then kill "$WPID" 2>/dev/null; WPID=; fi
    if [ "$UNDO_OK" = 1 ]; then
        undo
    elif [ "$1" = back ]; then
        say "NEEDS A PERSON: the board did not come back; nothing was changed by this driver"
        say "check the USB-C supply first (BL2 'Reset reason:' in the raw log); last resort: tools/mp2s_card_undo.sh"
    fi
    say "FAIL $1"
    exit 1
}
ok() { say "step $1: ok"; }

if [ "$DRY" != 1 ]; then mkdir -p "$REC" || { echo "$P cannot create $REC" >&2; exit 2; }; fi
echo "# record directory: $REC"

# (1) precheck, read-only on the board: a refusal changes nothing
capenv "$REC/env-before.txt" || fail precheck
run python3 -I "$HERE/mp2s_guard.py" --for standalone --printenv "$REC/env-before.txt" || fail precheck
if [ "$DRY" = 1 ]; then
    PFX=PREFIXES
    FREE=FREE_BYTES
    echo "+ ${SSH[*]} \$BOARD_HOST df -B1 --output=avail /boot | tail -n 1"
else
    PFX=$(sed -n 's/^boot_prefixes=//p' "$REC/env-before.txt" | tail -n 1)
    FREE=$("${SSH[@]}" "$BOARD_HOST" "df -B1 --output=avail /boot | tail -n 1" | tr -d ' ')
    case $FREE in '' | *[!0-9]*) say "cannot read the free space of /boot"; fail precheck ;; esac
fi
run python3 -I "$HERE/mp2s_env_capacity.py" --text "$REC/env-before.txt" \
    mp2s_ver=1 mp2s_left=$COUNT "mp2s_prefixes=$PFX" "boot_prefixes=/yakogl/standalone/ $PFX" || fail precheck
run python3 -I "$HERE/mp2s_image_check.py" "$IMG" --free-bytes "$FREE" || fail precheck
ok precheck

# (2) capture
capman "$REC/boot-before.txt" || fail capture
ok capture

# (3) console watch, before anything can reboot
echo "+ ${WATCH[*]} watch \$BOARD_UART $RAW $STAMPED --boots $COUNT --timeout $TIMEOUT --linger 60 &"
if [ "$DRY" != 1 ]; then
    "${WATCH[@]}" watch "$BOARD_UART" "$RAW" "$STAMPED" --boots "$COUNT" \
        --timeout "$TIMEOUT" --linger 60 > "$REC/watch.out" 2>&1 &
    WPID=$!
fi
ok console

# (4) runner, then the reboot (default itself reboots nothing)
UNDO_OK=1
cap "$REC/install.txt" "${RUNNER[@]}" install || fail runner
cap "$REC/default.txt" "${RUNNER[@]}" default "$IMG" "$COUNT" || fail runner
echo "+ ${SSH[*]} \$BOARD_HOST ${SUDO}systemctl reboot"
if [ "$DRY" != 1 ]; then
    UNDO_OK=0   # from here the board may be mid-boot until it answers again
    "${SSH[@]}" "$BOARD_HOST" "${SUDO}systemctl reboot" || true
fi
ok runner

# (5) the watch ends, then OpenSTLinux must answer
if [ "$DRY" = 1 ]; then
    echo "+ wait for the watch (exit 0 ok, 1 another verdict, 4 console busy)"
else
    rc=0
    wait "$WPID" || rc=$?
    WPID=
    if [ "$rc" != 0 ]; then say "console watch exit $rc"; fail back; fi
fi
cap "$REC/comeback.txt" "$HERE/mp2s_comeback.sh" --wait "$BACK" || { cat "$REC/comeback.txt"; fail back; }
UNDO_OK=1
ok back

# (6) the record
run python3 -I "$HERE/mp2s_console_watch.py" scan "$STAMPED" --boots "$COUNT" || fail record
run python3 -I "$HERE/mp2s_record_check.py" "$STAMPED" --count "$COUNT" || fail record
ok record

# (7) undo, read again, compare
run "${RUNNER[@]}" nodefault || fail undo
run "${RUNNER[@]}" uninstall || fail undo
capenv "$REC/env-after.txt" || fail undo
capman "$REC/boot-after.txt" || fail undo
run python3 -I "$HERE/mp2s_env_diff.py" "$REC/env-before.txt" "$REC/env-after.txt" || fail undo
run python3 -I "$HERE/mp2s_guard.py" --for oneshot --printenv "$REC/env-after.txt" || fail undo
echo "+ cmp $REC/boot-before.txt $REC/boot-after.txt"
if [ "$DRY" != 1 ]; then cmp "$REC/boot-before.txt" "$REC/boot-after.txt" || fail undo; fi
ok undo

if [ "$DRY" = 1 ]; then say "dry-run: nothing was run"; exit 0; fi
say PASS
