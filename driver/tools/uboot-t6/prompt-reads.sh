#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# prompt-reads.sh - reboot the NanoPC-T6 from Armbian, stop its U-Boot at
# the prompt, read the facts of docs/notes/nanopc-t6-uboot-handover.md there
# and let it boot Armbian again. Reads only: prompt-reads.py (beside this)
# holds the whole list of what can be entered at the prompt.
#
#   BOARD_HOST=root@<t6> driver/tools/uboot-t6/prompt-reads.sh                     # dry run
#   BOARD_HOST=root@<t6> T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=<tty> \
#     driver/tools/uboot-t6/prompt-reads.sh --go                                  # the run
#
# Without --go it makes read-only checks on the board, prints the commands
# and what --go would do, and changes nothing anywhere.
# With --go it takes the board lock (../kexec-zephyr/board_lock.sh; it
# refuses, never waits, while anyone holds it: see LOCK_WAIT), reads the
# board's state again under the lock, starts prompt-reads.py on the UART,
# waits until that listens, and only then reboots the board over ssh
# (`systemctl reboot`). prompt-reads.py stops the autoboot, enters the
# commands and `reset`. Then this waits for Armbian with a new boot_id.
#
# The UART is written to here, and Armbian logs root in on it by itself, so
# a byte at the wrong moment is typed into a root shell. prompt-reads.py's
# header says how that is excluded; its --self-test proves it without a
# board, and this script runs that self-test before it touches the board.
#
# The record: one directory per --go run under $T6_RECORD_DIR, printed at
# the start and at the end. record.txt (this script's lines), uart.log
# (every byte read), typed.log (every byte written, with its time),
# answers.txt (each command and its output). `printenv` prints the board's
# MAC addresses and serial number: answers.txt and uart.log stay in the
# record directory and are never copied into the tree, an issue or a pull
# request as they are.
#
# Exit: prompt-reads.py's code (0 done; 3 no autoboot line, nothing written;
# 4 the autoboot was missed, Linux booted; 5 stopped at the U-Boot prompt,
# the board stays there until its power is cut; 6 `reset` entered, no Linux
# login; 7 a command made U-Boot reset the board, Linux booted), 1 when the
# board did not come back into Armbian, 2 usage or a missing tool, 3 refused
# before anything was changed.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
K=$HERE/../kexec-zephyr
PY=$HERE/prompt-reads.py

die() { echo "prompt-reads.sh: $*" >&2; exit 2; }
refuse() {
	echo "REFUSED: $*" >&2
	echo "nothing was written to the UART and the board was not rebooted." >&2
	exit 3
}

GO=0
for a in "$@"; do
	case "$a" in
	--go) GO=1 ;;
	*) die "usage: prompt-reads.sh [--go] (see the header)" ;;
	esac
done
[ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=root@<the T6>"
# shellcheck disable=SC2206  # deliberately word-split
SSH_OPTS=(${BOARD_SSH_OPTS:-})
for t in ssh timeout python3 flock fuser stty sha256sum; do
	command -v "$t" >/dev/null || die "$t is not on this host"
done
echo "build $(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"

# ---- 1. the tool, without a board ------------------------------------------
selftest=$(python3 -I "$PY" --self-test 2>&1 | tail -1) || die "prompt-reads.py --self-test failed: $selftest"
case "$selftest" in
*", 0 failed") echo "$selftest" ;;
*) die "prompt-reads.py --self-test: $selftest" ;;
esac

# ---- 2. the board, read only -----------------------------------------------
board() { timeout 30 ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" "$@" </dev/null; }
# who_remote, defined on the board ahead of the commands that call it: the
# remote sessions without this ssh command's own (who_remote.sh).
WHO_REMOTE=$(cat "$K/who_remote.sh") || die "cannot read $K/who_remote.sh"
STATE='
printf "compatible=%s\n" "$(tr "\0" " " </proc/device-tree/compatible)"
printf "uid=%s\n" "$(id -u)"
printf "uptime=%s\n" "$(cut -d" " -f1 /proc/uptime)"
printf "system=%s\n" "$(systemctl is-system-running 2>/dev/null || true)"
printf "boot_id=%s\n" "$(cat /proc/sys/kernel/random/boot_id)"
printf "kexec_loaded=%s\n" "$(cat /sys/kernel/kexec_loaded)"
printf "gpu_users=%s\n" "$(fuser /dev/dri/* 2>/dev/null | tr -s " " || true)"
printf "who_remote=%s\n" "$(who_remote)"
printf "console_login=%s\n" "$(systemctl is-active serial-getty@ttyS2.service 2>/dev/null || true)"'
facts=$(board "$WHO_REMOTE$STATE") || refuse "cannot read the board's facts over ssh ($BOARD_HOST)"
fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p" | head -1; }

case "$(fact compatible)" in *friendlyarm,nanopc-t6*) ;; *) refuse "$BOARD_HOST is not a NanoPC-T6: compatible '$(fact compatible)'" ;; esac
[ "$(fact uid)" = 0 ] || refuse "$BOARD_HOST is not root (the reboot needs it)"
[ "$(fact kexec_loaded)" = 0 ] || refuse "a kexec image is loaded (/sys/kernel/kexec_loaded 1): someone else is about to jump"
echo "board: NanoPC-T6, up $(fact uptime) s, system $(fact system), boot_id $(fact boot_id)"
echo "board: GPU users '$(fact gpu_users)', remote sessions '$(fact who_remote)', login on the UART: $(fact console_login)"

echo "at the U-Boot prompt, in this order (reads only):"
python3 -I "$PY" plan | sed 's/^/  /'

if [ "$GO" != 1 ]; then
	echo "dry run: nothing was written to the UART and the board was not rebooted."
	echo "--go would: take the board lock, read the state above again, start prompt-reads.py on T6_UART,"
	echo "  reboot the board over ssh (systemctl reboot), stop the autoboot with one space, enter the"
	echo "  commands above and reset, then wait for Armbian with a new boot_id."
	[ -z "$(fact gpu_users)" ] || echo "--go would REFUSE now: processes hold the GPU device: $(fact gpu_users)"
	if [ -n "$(fact who_remote)" ] && [ "${T6_ALLOW_SESSIONS:-0}" != 1 ]; then
		echo "--go would REFUSE now: remote sessions on the board: $(fact who_remote)"
	fi
	exit 0
fi

# ---- 3. the run -------------------------------------------------------------
[ -n "${T6_HAND:-}" ] || refuse "--go needs T6_HAND=<who can cut the board's power>: a run that stops at the U-Boot prompt (exit 5) leaves the board there"
{ [ -n "${T6_RECORD_DIR:-}" ] && [ -d "$T6_RECORD_DIR" ]; } ||
	refuse "--go needs T6_RECORD_DIR=<an existing directory outside the checkout> for the run's record"
[ -n "${T6_UART:-}" ] || refuse "--go needs T6_UART=<the host tty on the T6's debug UART>"
[ -c "$T6_UART" ] && [ -r "$T6_UART" ] && [ -w "$T6_UART" ] ||
	refuse "T6_UART $T6_UART is not a character device this user can read and write"
if fuser "$T6_UART" >/dev/null 2>&1; then
	refuse "T6_UART $T6_UART is already open (pids $(fuser "$T6_UART" 2>/dev/null | tr -s " " | cut -c1-80)): a second reader would split the bytes; stop the other capture first"
fi
# raw, no echo, before prompt-reads.py opens it: with echo on, the tty would
# send every received byte back to the board (nanopc-t6.sh does the same).
UART_STTY=(1500000 raw -echo -icrnl -ixon cs8 -cstopb -parenb clocal)
stty -F "$T6_UART" "${UART_STTY[@]}" || refuse "stty on $T6_UART failed"
uart_mode=$(stty -F "$T6_UART" -a | tr "\n;" "  ") || refuse "stty -a on $T6_UART failed"
for w in 'speed 1500000' ' -echo ' ' -icrnl ' ' -ixon ' ' cs8 ' ' -cstopb ' ' -parenb ' ' clocal ' ' -opost ' ' -icanon '; do
	[[ " $uart_mode " == *"$w"* ]] || refuse "$T6_UART does not read back '${w# }' after stty: $uart_mode"
done

# Before the lock: board_lock.sh chains the EXIT trap it finds.
tmp=$(mktemp -d)
py_pid=
cleanup() {
	if [ -n "$py_pid" ]; then kill "$py_pid" 2>/dev/null || true; fi
	rm -rf "$tmp"
}
trap cleanup EXIT
# prompt-reads.py's own bounds: 240 s to the autoboot line, 30 s a command
# (about 30 commands, each answered in well under a second), 180 s to the
# login; then up to 180 s here for ssh.
HOLD_MAX=${HOLD_MAX:-1200}
LOCK_WAIT=${LOCK_WAIT:-0}
# shellcheck source=../kexec-zephyr/board_lock.sh
. "$K/board_lock.sh"

# Under the lock, the state the reboot depends on is read again.
fresh=$(board "$WHO_REMOTE$STATE") || refuse "cannot read the board's state again under the lock"
fresh_fact() { printf '%s\n' "$fresh" | sed -n "s/^$1=//p" | head -1; }
[ "$(fresh_fact kexec_loaded)" = 0 ] || refuse "under the lock: a kexec image is loaded (/sys/kernel/kexec_loaded '$(fresh_fact kexec_loaded)')"
[ -z "$(fresh_fact gpu_users)" ] || refuse "under the lock: processes hold the GPU device: $(fresh_fact gpu_users)"
if [ -n "$(fresh_fact who_remote)" ] && [ "${T6_ALLOW_SESSIONS:-0}" != 1 ]; then
	refuse "under the lock: remote sessions on the board: $(fresh_fact who_remote)-- the reboot ends them; ask whoever holds them, then T6_ALLOW_SESSIONS=1"
fi
old_boot=$(fresh_fact boot_id)
[ -n "$old_boot" ] || refuse "under the lock: the board gave no boot_id"

REC=$T6_RECORD_DIR/$(date +%Y%m%d-%H%M%S)-t6-uboot-reads
mkdir "$REC" || die "cannot make the record directory $REC"
rec() { printf '%s %s\n' "$(date '+%F %T')" "$*" | tee -a "$REC/record.txt"; }
echo "prompt-reads.sh: record $REC"
rec "build $(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"
rec "prompt-reads.py sha256 $(sha256sum "$PY" | cut -d' ' -f1); $selftest"
rec "board boot_id $old_boot, up $(fresh_fact uptime) s; hand at the power switch: $T6_HAND"

python3 -I "$PY" run "$T6_UART" "$REC" >"$tmp/py.out" 2>&1 &
py_pid=$!
n=0
until grep -q 'listening, nothing written' "$tmp/py.out" 2>/dev/null; do
	n=$((n + 1))
	if [ "$n" -gt 50 ] || ! kill -0 "$py_pid" 2>/dev/null; then
		sed 's/^/  /' "$tmp/py.out" >&2
		rec "prompt-reads.py did not start listening: the board was not rebooted" >&2
		exit 3
	fi
	sleep 0.2
done
rec "prompt-reads.py listens on the UART (pid $py_pid, nothing written)"

t0=$(date +%s)
rec "rebooting the board over ssh (systemctl reboot)"
# The connection may drop before the command's status comes back.
board 'systemctl reboot' >/dev/null 2>&1 || true

py_rc=0
wait "$py_pid" || py_rc=$?
py_pid=
while IFS= read -r line; do rec "$line"; done <"$tmp/py.out"
rec "prompt-reads.py exit $py_rc, $(( $(date +%s) - t0 )) s after the reboot command; answers.txt $(grep -c '^## ' "$REC/answers.txt" 2>/dev/null || echo 0) command(s)"

# Whatever the verdict, the board is looked for: a command that made U-Boot
# reset the board ends in Armbian too.
deadline=$(( $(date +%s) + 180 ))
while [ "$(date +%s)" -lt "$deadline" ]; do
	now_boot=$(board 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null || true)
	if [ -n "$now_boot" ] && [ "$now_boot" != "$old_boot" ]; then
		rec "back in Armbian $(( $(date +%s) - t0 )) s after the reboot command (boot_id $now_boot, up $(board 'cut -d" " -f1 /proc/uptime') s)"
		echo "prompt-reads.sh: record $REC"
		exit "$py_rc"
	fi
	sleep 5
done
if [ -n "${now_boot:-}" ]; then
	rec "the same Linux still runs (boot_id unchanged): the reboot did not happen" >&2
elif [ "$py_rc" = 5 ]; then
	rec "the board is at its U-Boot prompt and stays there: it needs $T6_HAND at its power switch" >&2
	echo "prompt-reads.sh: record $REC"
	exit 5
else
	rec "no Armbian $(( $(date +%s) - t0 )) s after the reboot command: the board needs $T6_HAND at its power switch" >&2
fi
echo "prompt-reads.sh: record $REC"
exit 1
