#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# mp2s_session.sh - the STM32MP257F-DK's standalone board session
# `default <image> 2` as one driver (docs/notes/stm32mp257f-dk-standalone.md,
# "Board run" steps 2-7). It stops at the first failing step.
#
#   tools/mp2s_session.sh [--dry-run] IMAGE
#
# Steps, one summary line each (`mp2s-session: step <name>: ok|FAIL`):
#   precheck  tools/mp2s_guard.py --for standalone on the env read from the
#             board, tools/mp2s_image_check.py on IMAGE; each helper not merged
#             yet (found with test -x) prints `mp2s-session: skip <tool>: not
#             present` and the session goes on
#   capture   the /boot manifest before (the U-Boot env is read in precheck)
#   console   the passive console logger, started before anything reboots
#   runner    $MP2S_RUNNER default IMAGE 2, then the reboot over ssh
#   record    wait for the logger, tools/mp2s_record_check.py on its raw log
#   env       nodefault, uninstall, env and manifest again, mp2s_env_diff.py
#   manifest  /boot byte for byte as before
# The last line is `mp2s-session: PASS` or `mp2s-session: FAIL <step>`. After a
# failure past the capture, nodefault and uninstall are still run.
# Records go to records/stm32mp257f-dk/<YYYYMMDD-HHMM>-standalone-<sha8>/.
#
# Environment (nothing about a board is kept in this file):
#   BOARD_HOST   user@host of the board's Linux (what the runner reads too)
#   BOARD_UART   HOST:PORT or device of the console (passive reads only)
#   MP2S_SSH     ssh command (default: ssh)
#   MP2S_RUNNER  the runner (default: tools/board-stm32mp2-standalone.sh)
#   MP2S_WATCH   the console logger command (default: python3 -I
#                tools/mp2s_console_watch.py); called as
#                `watch UART RAW STAMPED --boots N --timeout S --linger 60`
#   MP2S_TIMEOUT logger timeout in seconds (default 900)
#   MP2S_RECORDS records root (default: records/stm32mp257f-dk)
#   MP2_SUDO     root prefix on the board (default "sudo -n ")
# MP2_NO_WATCHDOG defaults to 1 for the runner: mp2_hello has no marker.
#
# Helpers whose interface is assumed (found by test -x, skipped when absent):
#   tools/mp2s_bootfs_check.py, mp2s_fwenv_check.py, fwu_metadata.py  DIR
#   tools/mp2s_comeback.sh DIR (after the run), tools/mp2s_boot_manifest.py
#   compare BEFORE AFTER. DIR holds env-before.txt and boot-before.txt.
# Exit: 0 PASS, 1 FAIL <step>, 2 usage.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
P="mp2s-session:"
COUNT=2

DRY=0
IMG=
for a in "$@"; do
    case "$a" in
    --dry-run) DRY=1 ;;
    -*) echo "$P usage: tools/mp2s_session.sh [--dry-run] IMAGE" >&2; exit 2 ;;
    *) if [ -z "$IMG" ]; then IMG=$a; else echo "$P usage: one IMAGE only" >&2; exit 2; fi ;;
    esac
done
[ -n "$IMG" ] || { echo "$P usage: tools/mp2s_session.sh [--dry-run] IMAGE" >&2; exit 2; }

read -ra SSH <<<"${MP2S_SSH:-ssh}"
read -ra RUNNER <<<"${MP2S_RUNNER:-$HERE/board-stm32mp2-standalone.sh}"
read -ra WATCH <<<"${MP2S_WATCH:-python3 -I $HERE/mp2s_console_watch.py}"
TIMEOUT=${MP2S_TIMEOUT:-900}
SUDO=${MP2_SUDO-"sudo -n "}
export MP2_NO_WATCHDOG=${MP2_NO_WATCHDOG:-1}

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
REACHED_BOARD=0

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
envcmd() { printf '%sfw_printenv' "$SUDO"; }
mancmd() { echo "find /boot -type f -print0 | sort -z | xargs -0 ${SUDO}sha256sum"; }
capenv() { cap "$1" "${SSH[@]}" "$BOARD_HOST" "$(envcmd)"; }
capman() { cap "$1" "${SSH[@]}" "$BOARD_HOST" "$(mancmd)"; }

# helper TOOL ARGS...: skip line when TOOL is not executable, else run it.
helper() {
    local tool=$1; shift
    if ! test -x "$HERE/$tool"; then say "skip $tool: not present"; return 0; fi
    run "$HERE/$tool" "$@"
}

cleanup() {
    REACHED_BOARD=0
    if [ -n "$WPID" ]; then kill "$WPID" 2>/dev/null; fi
    say "cleanup: nodefault, uninstall"
    "${RUNNER[@]}" nodefault >/dev/null 2>&1
    "${RUNNER[@]}" uninstall >/dev/null 2>&1
}

fail() {
    say "step $1: FAIL"
    if [ "$REACHED_BOARD" = 1 ]; then cleanup; fi
    say "FAIL $1"
    exit 1
}
ok() { say "step $1: ok"; }

if [ "$DRY" != 1 ]; then mkdir -p "$REC" || { echo "$P cannot create $REC" >&2; exit 2; }; fi
echo "# record directory: $REC"

# (1) precheck
capenv "$REC/env-before.txt" || fail precheck
run python3 -I "$HERE/mp2s_guard.py" --for standalone --printenv "$REC/env-before.txt" || fail precheck
run python3 -I "$HERE/mp2s_image_check.py" "$IMG" || fail precheck
for t in mp2s_bootfs_check.py mp2s_fwenv_check.py fwu_metadata.py; do
    helper "$t" "$REC" || fail precheck
done
ok precheck

# (2) capture
capman "$REC/boot-before.txt" || fail capture
ok capture

# (3) console logger
REACHED_BOARD=1
echo "+ ${WATCH[*]} watch \$BOARD_UART $RAW $STAMPED --boots $COUNT --timeout $TIMEOUT --linger 60 &"
if [ "$DRY" != 1 ]; then
    "${WATCH[@]}" watch "$BOARD_UART" "$RAW" "$STAMPED" --boots "$COUNT" \
        --timeout "$TIMEOUT" --linger 60 > "$REC/watch.out" 2>&1 &
    WPID=$!
fi
ok console-start

# (4) runner, then the reboot: default itself reboots nothing
cap "$REC/default.txt" "${RUNNER[@]}" default "$IMG" "$COUNT" || fail runner
echo "+ ${SSH[*]} \$BOARD_HOST ${SUDO}systemctl reboot"
if [ "$DRY" != 1 ]; then "${SSH[@]}" "$BOARD_HOST" "${SUDO}systemctl reboot" || true; fi
ok runner

# (5) wait for the logger, then the record
if [ "$DRY" = 1 ]; then
    echo "+ wait for the logger (exit 0 matched, 4 timeout, 2 error)"
else
    rc=0
    wait "$WPID" || rc=$?
    WPID=
    if [ "$rc" != 0 ]; then say "console logger exit $rc"; fail console; fi
fi
ok console
run python3 -I "$HERE/mp2s_record_check.py" "$RAW" --count "$COUNT" || fail record
ok record

# (6) undo, capture again, compare
run "${RUNNER[@]}" nodefault || fail env
run "${RUNNER[@]}" uninstall || fail env
REACHED_BOARD=0
capenv "$REC/env-after.txt" || fail env
capman "$REC/boot-after.txt" || fail env
run python3 -I "$HERE/mp2s_env_diff.py" "$REC/env-before.txt" "$REC/env-after.txt" || fail env
ok env
helper mp2s_comeback.sh "$REC" || fail env
if [ "$DRY" = 1 ]; then
    echo "+ cmp $REC/boot-before.txt $REC/boot-after.txt"
else
    cmp "$REC/boot-before.txt" "$REC/boot-after.txt" || fail manifest
fi
helper mp2s_boot_manifest.py compare "$REC/boot-before.txt" "$REC/boot-after.txt" || fail manifest
ok manifest

if [ "$DRY" = 1 ]; then say "dry-run: nothing was run"; exit 0; fi
say PASS
