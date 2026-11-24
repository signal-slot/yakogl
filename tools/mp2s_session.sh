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
# Steps, in order:
#   precheck  read-only: the runner's printenv (the active copy of the SD
#             card's u-boot-env, read from the partition) into
#             tools/mp2s_guard.py --for standalone;
#             tools/mp2s_image_check.py IMAGE against the free space of /boot;
#             the device /boot is on (a disk whose device/type says SD; no
#             mmcblk index means the eMMC), then, each when present:
#             tools/mp2s_bootfs_check.py and tools/fwu_metadata.py check on
#             the card's metadata1 and metadata2 (read, never written)
#   capture   the runner's printenv, the sha256 of every file under /boot and the boot id,
#             before; tools/mp2s_boot_manifest.py parses the listing
#   console   the passive console watch, started before anything can reboot;
#             one that ends at once (busy console) stops the session here
#   runner    install, default IMAGE 2, the read-back (tools/mp2s_env_diff.py
#             --expect-default 2), then the reboot (default reboots nothing)
#   console   the watch's end: anything but exit 0 is a FAIL
#   comeback  tools/mp2s_comeback.sh (else ssh answers within MP2S_BACK s)
#   record    tools/mp2s_record_check.py on the stamped log, and a login prompt
#             in the raw log
#   undo      the runner's nodefault, then uninstall
#   env       the runner's printenv again; tools/mp2s_env_diff.py BEFORE AFTER and
#             tools/mp2s_guard.py --for oneshot
#   manifest  the /boot listing again; tools/mp2s_boot_manifest.py
#             compare-records (else cmp)
# A helper that is not there prints `mp2s-session: skip tools/<name>: not
# present` and its check does not run (and is never reported as passed).
# Then one line per step (`mp2s-session: step <name>: ok|FAIL|not run`) and
# `mp2s-session: PASS` or `mp2s-session: FAIL <step>` last.
#
# fw_printenv is never used: the DK's Linux has no /etc/fw_env.config, so it
# stops with "Cannot initialize environment" (#1281). Every environment read
# is the runner's printenv verb, which reads the u-boot-env partition of the
# SD card /boot is on (found in lsblk of that card, never by partlabel).
#
# Stop rules (the note's step 8): nothing is retried, no reset resource is
# used, `forever` is never run. A failure while the board is in Linux with
# something staged runs nodefault and uninstall once. A board that may be
# mid-boot (the watch or the comeback failed) is NOT touched: no second power
# cycle, no reset resource, no other outside reset, no runner call, no ssh
# command of the driver after the failure. It prints one line
# `mp2s-session: NEEDS PERSON: <reason>` (also in session.txt), the card-in-a-PC
# undo to use (tools/mp2s_card_undo.sh --dry-run, then --yes, on the card's
# bootfs) and that the USB-C supply is checked first, then exits 3.
#
# Record: records/stm32mp257f-dk/<YYYYMMDD-HHMM>-standalone-<commit sha8>/
# (gitignored): env-before.txt, boot-before.txt, raw.log, console.log, the
# helpers' outputs, env-after.txt, boot-after.txt and session.txt.
#
# Environment (nothing about a board is kept in this file):
#   BOARD_HOST   the board's Linux for ssh, from the lease (the runner's and
#                tools/mp2s_comeback.sh's variable); no default
#   BOARD_UART   the console, HOST:PORT or a device, from the lease; read only
#   MP2S_SSH     the ssh command (default: ssh), given -o BatchMode=yes
#                -o ConnectTimeout=10; tools/mp2s_comeback.sh uses its own ssh
#   MP2S_RUNNER  the runner (default: tools/board-stm32mp2-standalone.sh)
#   MP2S_WATCH   the console watch (default: python3 -I
#                tools/mp2s_console_watch.py), called as `watch UART RAW
#                STAMPED --boots 2 --timeout S --linger 60`: exit 0 the
#                verdict ok, 1 another verdict (a timeout ends in one),
#                4 console busy or unreachable, 2 usage
#   MP2S_TIMEOUT watch timeout in seconds (default 900)
#   MP2S_BACK    seconds to wait for ssh after the run (default 300)
#   MP2S_WATCH_SETTLE  seconds the watch must stay up before the runner
#                (default 3); a watch that ends sooner is seen at once
#   MP2S_RECORDS records root (default: records/stm32mp257f-dk)
#   MP2_SUDO     root prefix on the board (default "sudo -n ")
# MP2_NO_WATCHDOG defaults to 1 for the runner: mp2_hello has no marker.
# BOARD_PERSON_PRESENT is removed from the environment: no `forever` here.
# Exit: 0 PASS, 1 FAIL <step>, 2 usage, 3 NEEDS PERSON (the board did not come
# back: the console watch or the comeback check failed; stopped, never retried). Selftest: tools/mp2s_session_selftest.sh.
set -uo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/.." && pwd)
P="mp2s-session:"
COUNT=2
USAGE="usage: tools/mp2s_session.sh [--dry-run] IMAGE"
LOGIN_RE='(^|[[:space:]])[[:alnum:]-]+ login: ?$'
STEPS="precheck capture console runner comeback record undo env manifest"

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

read -ra SSH <<<"${MP2S_SSH:-ssh}"
SSH+=(-o BatchMode=yes -o ConnectTimeout=10)
read -ra RUNNER <<<"${MP2S_RUNNER:-$HERE/board-stm32mp2-standalone.sh}"
read -ra WATCH <<<"${MP2S_WATCH:-python3 -I $HERE/mp2s_console_watch.py}"
TIMEOUT=${MP2S_TIMEOUT:-900}
BACK=${MP2S_BACK:-300}
SETTLE=${MP2S_WATCH_SETTLE:-3}
for n in "$TIMEOUT" "$BACK" "$SETTLE"; do
    case $n in '' | *[!0-9]*) echo "$P MP2S_TIMEOUT, MP2S_BACK and MP2S_WATCH_SETTLE are whole seconds" >&2; exit 2 ;; esac
done
DEFAULT_SUDO='sudo -n '
SUDO=${MP2_SUDO-$DEFAULT_SUDO}
PY=(python3 -I)
export MP2_NO_WATCHDOG=${MP2_NO_WATCHDOG:-1}
unset BOARD_PERSON_PRESENT

if [ "$DRY" != 1 ]; then
    [ -n "${BOARD_HOST:-}" ] || { echo "$P set BOARD_HOST (from the lease; no default)" >&2; exit 2; }
    [ -n "${BOARD_UART:-}" ] || { echo "$P set BOARD_UART (from the lease; no default)" >&2; exit 2; }
    [ -r "$IMG" ] || { echo "$P cannot read $IMG" >&2; exit 2; }
    export BOARD_HOST
fi

SHA8=$(git -C "$ROOT" rev-parse --short=8 HEAD 2>/dev/null) || SHA8=00000000
REC=${MP2S_RECORDS:-$ROOT/records/stm32mp257f-dk}/$(date +%Y%m%d-%H%M)-standalone-$SHA8
RAW=$REC/raw.log
STAMPED=$REC/console.log
WPID=
NEEDS=      # the reason, once the board may be stranded (exit 3)
UNDO_OK=0   # 1: the board is in Linux with something staged: a failure runs the undo
declare -A ST
for s in $STEPS; do ST[$s]="not run"; done

say() { echo "$P $*"; }
show() { echo "+ $*"; }
# hrun CMD...: a host command, printed, run unless --dry-run
hrun() { show "$*"; [ "$DRY" = 1 ] && return 0; "$@"; }
# hcap FILE CMD...: a host command whose output goes to FILE (and the terminal)
hcap() {
    local f=$1 rc
    shift
    show "$* > $f"
    [ "$DRY" = 1 ] && return 0
    "$@" > "$f" 2>&1
    rc=$?
    cat "$f"
    return "$rc"
}
# bcap FILE CMD: a board command (one string) whose stdout goes to FILE
bcap() {
    show "ssh \$BOARD_HOST '$2' > $1"
    [ "$DRY" = 1 ] && return 0
    "${SSH[@]}" "$BOARD_HOST" "$2" > "$1" </dev/null
}
# rcap FILE: the runner's printenv (stdout only) into FILE
rcap() {
    show "${RUNNER[*]} printenv > $1"
    [ "$DRY" = 1 ] && return 0
    "${RUNNER[@]}" printenv > "$1"
}
# b CMD: a board command (one string), its stdout to ours
b() {
    show "ssh \$BOARD_HOST '$1'"
    [ "$DRY" = 1 ] && return 0
    "${SSH[@]}" "$BOARD_HOST" "$1" </dev/null
}
# have NAME: the helper tools/NAME is there; else the skip line
have() {
    test -x "$HERE/$1" && return 0
    say "skip tools/$1: not present"
    return 1
}

finish() {
    local s
    trap - EXIT
    if [ -n "$WPID" ]; then kill "$WPID" 2>/dev/null; wait "$WPID" 2>/dev/null; WPID=; fi
    {
        for s in $STEPS; do say "step $s: ${ST[$s]}"; done
        if [ -n "$NEEDS" ]; then
            say "NEEDS PERSON: $NEEDS"
            say "undo with the card in a PC: tools/mp2s_card_undo.sh --dry-run, then --yes, on the card's bootfs partition"
            say "check the USB-C supply first (BL2 'Reset reason:' in raw.log: tools/mp2s_reset_reason.py); no reset, no retry"
        fi
        say "$1"
    } | if [ "$DRY" = 1 ] || [ ! -d "$REC" ]; then cat; else tee "$REC/session.txt"; fi
    [ "$1" = PASS ] && exit 0
    [ -n "$NEEDS" ] && exit 3
    exit 1
}
undo_after_failure() {
    say "undo after the failure: nodefault, then uninstall (once, no retry)"
    "${RUNNER[@]}" nodefault && "${RUNNER[@]}" uninstall && { ST[undo]="ok (after a failure)"; return; }
    ST[undo]=FAIL
    say "the undo failed: a person runs the runner's nodefault and uninstall"
}
fail() {
    ST[$1]=FAIL
    if [ -n "$WPID" ]; then kill "$WPID" 2>/dev/null; wait "$WPID" 2>/dev/null; WPID=; fi
    if [ "$UNDO_OK" = 1 ]; then
        undo_after_failure
    elif [ "$1" = console ] || [ "$1" = comeback ]; then
        if [ "${2:-}" != idle ]; then
            NEEDS="the board did not come back (step $1 failed); it may be mid-boot, nothing more is sent to it"
        fi
    fi
    finish "FAIL $1"
}
trap 'finish "FAIL interrupted"' INT TERM
trap '[ -z "$WPID" ] || kill "$WPID" 2>/dev/null' EXIT

if [ "$DRY" = 1 ]; then
    say "dry-run: every command is printed, nothing is run, nothing is written"
else
    mkdir -p "$REC" || { echo "$P cannot create $REC" >&2; exit 2; }
fi
echo "# record directory: $REC"

# (1) precheck, read-only on the board: a refusal changes nothing
rcap "$REC/env-precheck.txt" || fail precheck
hrun "${PY[@]}" "$HERE/mp2s_guard.py" --for standalone --printenv "$REC/env-precheck.txt" || fail precheck
bcap "$REC/df.txt" "df -B1 --output=avail /boot | tail -n 1" || fail precheck
if [ "$DRY" = 1 ]; then
    FREE=FREE_BYTES
else
    FREE=$(tr -d ' \n' < "$REC/df.txt")
    case $FREE in '' | *[!0-9]*) say "cannot read the free space of /boot"; fail precheck ;; esac
fi
hrun "${PY[@]}" "$HERE/mp2s_image_check.py" "$IMG" --free-bytes "$FREE" || fail precheck
bcap "$REC/findmnt.txt" "findmnt -n -o SOURCE /boot" || fail precheck
bcap "$REC/lsblk.txt" "lsblk -P -o NAME,PKNAME,TYPE,FSTYPE,LABEL,PARTLABEL" || fail precheck
bcap "$REC/blkid.txt" "${SUDO}blkid" || fail precheck
if [ "$DRY" = 1 ]; then
    DISK=DISK
else
    SRC=$(tr -d ' \n' < "$REC/findmnt.txt")
    [[ $SRC =~ ^/dev/(mmcblk[0-9]+)p[0-9]+$ ]] || { say "/boot is on '$SRC', not a partition of an mmcblk disk"; fail precheck; }
    DISK=${BASH_REMATCH[1]}
fi
bcap "$REC/disk-type.txt" "cat /sys/block/$DISK/device/type" || fail precheck
if [ "$DRY" != 1 ] && [ "$(tr -d ' \n' < "$REC/disk-type.txt")" != SD ]; then
    say "/boot's disk is not the microSD (sysfs type is not SD)"
    fail precheck
fi
if have mp2s_bootfs_check.py; then
    hrun "${PY[@]}" "$HERE/mp2s_bootfs_check.py" --findmnt "$REC/findmnt.txt" --lsblk "$REC/lsblk.txt" \
        --blkid "$REC/blkid.txt" --disk-type "$REC/disk-type.txt" || fail precheck
fi
if have fwu_metadata.py; then
    for m in metadata1 metadata2; do
        if [ "$DRY" = 1 ]; then
            dev=PART_$m
        else
            dev=$(sed -n "s/^NAME=\"\([^\"]*\)\" PKNAME=\"$DISK\" .* PARTLABEL=\"$m\"\$/\1/p" "$REC/lsblk.txt" | head -n 1)
            [[ $dev =~ ^${DISK}p[0-9]+$ ]] || { say "no $m partition on $DISK"; fail precheck; }
        fi
        bcap "$REC/fwu-$m.bin" "${SUDO}head -c 1048576 -- /dev/$dev" || fail precheck
    done
    hrun "${PY[@]}" "$HERE/fwu_metadata.py" check "$REC/fwu-metadata1.bin" "$REC/fwu-metadata2.bin" || fail precheck
fi
ST[precheck]=ok

# (2) capture, before
rcap "$REC/env-before.txt" || fail capture
# find -exec, not xargs -0: the DK's xargs is BusyBox's (no -0), and that
# pipeline lists nothing with exit 0; an empty listing is a failure here
bcap "$REC/boot-before.txt" "cd /boot && find . -type f -exec ${SUDO}sha256sum -- {} + | LC_ALL=C sort -k 2" || fail capture
if [ "$DRY" != 1 ] && ! grep -q '  \./boot\.scr\.uimg$' "$REC/boot-before.txt"; then say "the /boot listing is empty or has no ./boot.scr.uimg"; fail capture; fi
bcap "$REC/boot-id-before.txt" "cat /proc/sys/kernel/random/boot_id" || fail capture
if have mp2s_boot_manifest.py; then
    hrun "${PY[@]}" "$HERE/mp2s_boot_manifest.py" record --from "$REC/boot-before.txt" -o "$REC/manifest-before.txt" || fail capture
fi
ST[capture]=ok

# (3) the console watch, before anything can reboot
show "${WATCH[*]} watch \$BOARD_UART $RAW $STAMPED --boots $COUNT --timeout $TIMEOUT --linger 60 > $REC/console-verdict.txt &"
if [ "$DRY" != 1 ]; then
    "${WATCH[@]}" watch "$BOARD_UART" "$RAW" "$STAMPED" --boots "$COUNT" \
        --timeout "$TIMEOUT" --linger 60 > "$REC/console-verdict.txt" 2>&1 &
    WPID=$!
    # it must stay up the whole settle time; one that ends is seen at once
    end=$((SECONDS + SETTLE))
    while [ "$SECONDS" -lt "$end" ] && kill -0 "$WPID" 2>/dev/null; do sleep 0.2; done
    if ! kill -0 "$WPID" 2>/dev/null; then
        rc=0
        wait "$WPID" || rc=$?
        WPID=
        cat "$REC/console-verdict.txt"
        say "the console watch ended before the run (exit $rc): nothing was changed"
        fail console idle
    fi
fi

# (4) the runner: install, default IMAGE 2, the read-back, then the reboot
UNDO_OK=1
echo "# the runner gets MP2_NO_WATCHDOG=$MP2_NO_WATCHDOG and no BOARD_PERSON_PRESENT"
hcap "$REC/install.txt" "${RUNNER[@]}" install || fail runner
hcap "$REC/default.txt" "${RUNNER[@]}" default "$IMG" "$COUNT" || fail runner
rcap "$REC/env-default.txt" || fail runner
hrun "${PY[@]}" "$HERE/mp2s_env_diff.py" "$REC/env-before.txt" "$REC/env-default.txt" --expect-default "$COUNT" || fail runner
UNDO_OK=0   # from here the board may be mid-boot until it answers again
b "${SUDO}systemctl reboot"
ST[runner]=ok

# (5) the watch's end, OpenSTLinux back, the record
show "wait for the console watch (0: verdict ok)"
if [ "$DRY" != 1 ]; then
    rc=0
    wait "$WPID" || rc=$?
    WPID=
    cat "$REC/console-verdict.txt"
    if [ "$rc" != 0 ]; then say "console watch exit $rc"; fail console; fi
fi
ST[console]=ok
if have mp2s_comeback.sh; then
    if [ "$DRY" = 1 ]; then BID=BOOT_ID_BEFORE; else BID=$(tr -d ' \n' < "$REC/boot-id-before.txt"); fi
    hcap "$REC/comeback.txt" "$HERE/mp2s_comeback.sh" --wait "$BACK" --before-boot-id "$BID" || fail comeback
else
    show "ssh \$BOARD_HOST true   # until it answers, at most $BACK s"
    if [ "$DRY" != 1 ]; then
        end=$((SECONDS + BACK))
        until "${SSH[@]}" "$BOARD_HOST" true </dev/null; do
            [ "$SECONDS" -lt "$end" ] || { say "the board did not answer ssh within $BACK s"; fail comeback; }
            sleep 5
        done
    fi
fi
UNDO_OK=1
ST[comeback]=ok
hcap "$REC/record-verdict.txt" "${PY[@]}" "$HERE/mp2s_record_check.py" "$STAMPED" --count "$COUNT" || fail record
show "tr -d '\\r' < $RAW | grep -aEq '$LOGIN_RE'"
if [ "$DRY" != 1 ]; then
    tr -d '\r' < "$RAW" | grep -aEq "$LOGIN_RE" || { say "no login prompt in raw.log"; fail record; }
fi
ST[record]=ok

# (6) undo, read again, compare
hcap "$REC/nodefault.txt" "${RUNNER[@]}" nodefault || { UNDO_OK=0; fail undo; }
hcap "$REC/uninstall.txt" "${RUNNER[@]}" uninstall || { UNDO_OK=0; fail undo; }
UNDO_OK=0
ST[undo]=ok
rcap "$REC/env-after.txt" || fail env
hcap "$REC/env-diff.txt" "${PY[@]}" "$HERE/mp2s_env_diff.py" "$REC/env-before.txt" "$REC/env-after.txt" || fail env
hrun "${PY[@]}" "$HERE/mp2s_guard.py" --for oneshot --printenv "$REC/env-after.txt" || fail env
ST[env]=ok
bcap "$REC/boot-after.txt" "cd /boot && find . -type f -exec ${SUDO}sha256sum -- {} + | LC_ALL=C sort -k 2" || fail manifest
if have mp2s_boot_manifest.py; then
    hcap "$REC/boot-compare.txt" "${PY[@]}" "$HERE/mp2s_boot_manifest.py" compare-records \
        "$REC/boot-before.txt" "$REC/boot-after.txt" || fail manifest
else
    hrun cmp "$REC/boot-before.txt" "$REC/boot-after.txt" || fail manifest
fi
ST[manifest]=ok

if [ "$DRY" = 1 ]; then
    for s in $STEPS; do say "step $s: not run (dry-run)"; done
    say "dry-run: nothing was run"
    exit 0
fi
finish PASS
