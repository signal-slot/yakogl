#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# prompt-go.sh - start a Zephyr image on the NanoPC-T6 from its U-Boot
# prompt: reboot the board from Armbian, stop its U-Boot, load the image
# from the eMMC (staged under /boot/yakogl by stage.sh beside this), check
# it, `go`, record the image's UART output and verdict, and wait for
# Armbian to come back (README.md, "The route"; issue #4627).
#
#   BOARD_HOST=root@<t6> driver/tools/uboot-t6/prompt-go.sh <zephyr.bin> <probe>          # dry run
#   BOARD_HOST=root@<t6> T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=<tty> \
#     driver/tools/uboot-t6/prompt-go.sh <zephyr.bin> <probe> --go                      # the run
#   ... T6_UART=rfc2217://<host>:<port> ... --go     # the UART on a network serial adapter
#
# probe: t6_hello, t6_gpu, t6_mmu, t6_fw, t6_submit, t6_cold or triangle
# (t6-verdict.sh's table for the record). t6_display is not taken: it
# adopts the picture Linux leaves after kexec --crash, and U-Boot leaves
# none.
#
# Without --go it checks the image on this host, checks over ssh (reads
# only) that the board runs Armbian and holds the same bytes as
# /boot/yakogl/<name of zephyr.bin>, prints the commands and what each
# answer must be, and changes nothing anywhere.
# With --go it checks T6_HAND, T6_RECORD_DIR and T6_UART, runs prompt-go.py's
# self-test (a fake board on a pty), takes the board lock (../kexec-zephyr/board_lock.sh; it
# refuses, never waits, while anyone holds it: see LOCK_WAIT), reads the
# board's state again under the lock, starts prompt-go.py on the UART,
# waits until that listens, and only then reboots the board over ssh
# (`systemctl reboot`). prompt-go.py stops the autoboot, enters the
# commands one at a time, each answer checked before the next, and `go`;
# after `go` nothing is written to the UART. The image prints its record
# and resets the board at its end (PSCI, or its WDT0); U-Boot's
# `bootflow scan -lb` then boots Armbian. This waits for Armbian with a
# new boot_id.
#
# From `go` until the image arms WDT0 (T6:A3) nothing resets the board if
# the image stops: U-Boot runs no watchdog and Linux's is gone. A stop
# there holds the board until its power is cut, hence T6_HAND.
#
# The record: one directory per --go run under $T6_RECORD_DIR, printed at
# the start and at the end. record.txt (this script's lines), uart.log
# (every byte read), typed.log (every byte written, with its time),
# answers.txt (each command and its answer), image.log (the UART from `go`
# to Linux's login), verdict.txt (t6-verdict.sh, T6_ROUTE=uboot). When the
# login never came on the UART, uart.log ends with a "####" line that says
# where the stream went silent.
#
# Exit: prompt-go.py's code (0 the verdict PASS and Armbian back; 12 no
# autoboot line after the reboot, nothing written; 4 the autoboot was
# missed, Linux booted;
# 5 stopped at the U-Boot prompt, the board stays there until its power is
# cut; 6 refused at the prompt, `reset` entered, and no sign of Linux; 7 a
# command made U-Boot reset the board; 8 an answer differed: no `go`,
# `reset` entered, Armbian back; 9 `go` not answered; 10 the image
# started, and no sign of Linux: the board needs T6_HAND; 11 the verdict
# FAIL, Armbian back). "No sign of Linux" is no login on the UART and no
# new boot_id over ssh: when the UART shows no login (its stream can stop in
# the kernel's output, issue #11874) but ssh finds Armbian with a new
# boot_id, the exit is the code of the login seen (6 becomes 8; 10 becomes
# 0 or 11 by the verdict) and record.txt says so. 1 when the board did
# not come back into Armbian otherwise, 2 usage or a missing tool, 3
# refused before anything was changed (the board was not rebooted: 3 has
# only this meaning here). With T6_UART=rfc2217://<host>:<port>: 40 when
# t6-uart-rfc2217.py could not open the session or set the line (this
# script did not start), 41 when that tool failed during the run.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
K=$HERE/../kexec-zephyr
PY=$HERE/prompt-go.py
TOOL=prompt-go
# The window nanopc_t6.dts links the image into (zephyr,sram).
# ../kexec-zephyr/board_lock_selftest.sh fails when these differ from it.
WINDOW_BASE=$((0x10000000))
WINDOW_SIZE=$((256 << 20))
STAGE_DIR=/boot/yakogl

die() { echo "prompt-go.sh: $*" >&2; exit 2; }
refuse() {
	echo "REFUSED: $*" >&2
	echo "nothing was written to the UART and the board was not rebooted." >&2
	exit 3
}

GO=0
IMG=
PROBE=
for a in "$@"; do
	case "$a" in
	--go) GO=1 ;;
	-*) die "unknown option $a" ;;
	*)
		if [ -z "$IMG" ]; then IMG=$a
		elif [ -z "$PROBE" ]; then PROBE=$a
		else die "usage: prompt-go.sh <zephyr.bin> <probe> [--go] (see the header)"; fi
		;;
	esac
done
[ -n "$IMG" ] && [ -n "$PROBE" ] || die "usage: prompt-go.sh <zephyr.bin> <probe> [--go] (see the header)"
case "$PROBE" in t6_hello | t6_gpu | t6_mmu | t6_fw | t6_submit | t6_cold | triangle) ;; *) die "probe $PROBE: not one of t6-verdict.sh's" ;; esac
[ -f "$IMG" ] || die "$IMG: no such file"
[ -n "${BOARD_HOST:-}" ] || die "set BOARD_HOST=root@<the T6>"
# T6_UART=rfc2217://<host>:<port> (the debug UART on a network serial
# adapter): --go runs again through ../kexec-zephyr/t6-uart-rfc2217.py,
# which opens the session, sets 1500000 8N1 (the adapter's answers must say
# so, else exit 40 and nothing is done) and hands this script a pty as
# T6_UART (and T6_UART_RFC2217=1). Every check of T6_UART below then runs on
# the pty. The address is never printed or recorded.
if [[ ${T6_UART:-} == rfc2217://* ]] && [ "$GO" = 1 ]; then
	[ -z "${T6_UART_RFC2217:-}" ] || die "T6_UART is still an rfc2217:// address inside t6-uart-rfc2217.py"
	exec python3 -I "$K/t6-uart-rfc2217.py" run -- bash "$HERE/$(basename "$0")" "$@"
fi
# shellcheck disable=SC2206  # deliberately word-split
SSH_OPTS=(${BOARD_SSH_OPTS:-})
for t in ssh timeout python3 flock fuser stty sha256sum od strings; do
	command -v "$t" >/dev/null || die "$t is not on this host"
done
echo "build $(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"

# ---- 1. the image, on this host --------------------------------------------
NAME=$(basename "$IMG")
[[ "$NAME" =~ ^[A-Za-z0-9_][A-Za-z0-9._-]*\.bin$ ]] || refuse "$NAME: the staged name must be letters, digits, '.', '_' or '-', ending in .bin"
magic=$(od -A n -t x1 -j 56 -N 4 "$IMG" | tr -d ' \n')
[ "$magic" = 41524d64 ] || refuse "$IMG is not an AArch64 Image (no ARM\\x64 at byte 56)"
size=$(stat -c %s "$IMG")
footprint=$(od -A n -t u8 -j 16 -N 8 "$IMG" | tr -d ' ')
[ "$footprint" -ge "$size" ] || footprint=$size
[ "$footprint" -le "$WINDOW_SIZE" ] || refuse "the image needs $footprint bytes, the window has $WINDOW_SIZE"
strs=$(strings -n 20 "$IMG")
markers=$(grep '^yakogl-wdt: nanopc_t6 ' <<<"$strs" || true)
re='^yakogl-wdt: nanopc_t6 WDT0 armed at boot; top ([0-9]+); lifetime_s ([0-9]+); alive_s ([0-9]+); proof ([01]); load (0x[0-9a-fA-F]+)$'
[[ "$markers" =~ $re ]] || refuse "$IMG carries no single 'yakogl-wdt: nanopc_t6' marker: its watchdog is not armed at boot"
top=${BASH_REMATCH[1]} lifetime=${BASH_REMATCH[2]} proof=${BASH_REMATCH[4]} load=${BASH_REMATCH[5]}
[ $((load)) -eq "$WINDOW_BASE" ] || refuse "the image is linked at $load, not at the window $(printf 0x%x "$WINDOW_BASE")"
grep -qxF 'board: watchdog armed' <<<"$strs" || refuse "$IMG lacks the 'board: watchdog armed' line"
# The go route's entry (board::nanopc_t6_go) prints T6:G; an image built
# before it would run on U-Boot's caches.
grep -qF 'T6:G go: SCTLR_EL2' <<<"$(strings -n 8 "$IMG")" || refuse "$IMG has no T6:G line: its entry does not turn off the MMU and caches U-Boot's go leaves on"
[ "$top" -ge 11 ] && [ "$top" -le 15 ] || refuse "WDT0 TOP $top outside 11..15"
period_s=$(( ((1 << (16 + top)) + 23999999) / 24000000 ))
if [ "$proof" = 1 ]; then image_wait=$((period_s + 10)); else image_wait=$((lifetime + period_s)); fi
# U-Boot, its prompt and the load take well under a minute.
image_wait=$((image_wait + 60))
crc=$(python3 -I -c 'import sys, zlib; print("%08x" % zlib.crc32(open(sys.argv[1], "rb").read()))' "$IMG")
sha=$(sha256sum "$IMG" | cut -d' ' -f1)
echo "image: $IMG, $size bytes, crc32 $crc, sha256 $sha, linked at $load"
echo "image: WDT0 TOP $top, lifetime $lifetime s, proof $proof: the image ends by $image_wait s after go at the latest"

# ---- 2. the board, read only -----------------------------------------------
# shellcheck source=prompt-lib.sh
. "$HERE/prompt-lib.sh"
WHO_REMOTE=$(cat "$K/who_remote.sh") || die "cannot read $K/who_remote.sh"
STATE="$T6_STATE_COMMON"'
printf "boot_dev=%s\n" "$(findmnt -n -o SOURCE --target /boot 2>/dev/null || true)"
printf "staged=%s\n" "$(sha256sum '"$STAGE_DIR/$NAME"' 2>/dev/null | cut -d" " -f1)"'
facts=$(board "$WHO_REMOTE$STATE") || refuse "cannot read the board's facts over ssh ($BOARD_HOST)"
fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p" | head -1; }

case "$(fact compatible)" in *friendlyarm,nanopc-t6*) ;; *) refuse "$BOARD_HOST is not a NanoPC-T6: compatible '$(fact compatible)'" ;; esac
[ "$(fact uid)" = 0 ] || refuse "$BOARD_HOST is not root (the reboot needs it)"
[ "$(fact kexec_loaded)" = 0 ] || refuse "a kexec image is loaded (/sys/kernel/kexec_loaded 1): someone else is about to jump"
[ "$(fact staged)" = "$sha" ] ||
	refuse "$STAGE_DIR/$NAME on the board is '$(fact staged)', not this image ($sha): stage it first (stage.sh stage $IMG)"
echo "board: NanoPC-T6, up $(fact uptime) s, system $(fact system), boot_id $(fact boot_id)"
# U-Boot loads from mmc 0:1, the eMMC's ext4 root (README.md): printed here;
# a /boot elsewhere makes ext4size refuse at the prompt (exit 8).
echo "board: $STAGE_DIR/$NAME is this image (sha256), /boot on $(fact boot_dev)"
echo "board: GPU users '$(fact gpu_users)', remote sessions '$(fact who_remote)'"

echo "at the U-Boot prompt, one at a time, each answer checked:"
python3 -I "$PY" plan "$NAME" "$size" "$crc" | sed 's/^/  /'

if [ "$GO" != 1 ]; then
	echo "dry run: nothing was written to the UART and the board was not rebooted."
	echo "--go would: take the board lock, read the state above again, start prompt-go.py on T6_UART,"
	echo "  reboot the board over ssh (systemctl reboot), stop the autoboot with one space, enter the"
	echo "  commands above, go, record the image's output and its verdict ($PROBE, T6_ROUTE=uboot),"
	echo "  then wait for Armbian with a new boot_id."
	uart_dry
	[ -z "$(fact gpu_users)" ] || echo "--go would REFUSE now: processes hold the GPU device: $(fact gpu_users)"
	if [ -n "$(fact who_remote)" ] && [ "${T6_ALLOW_SESSIONS:-0}" != 1 ]; then
		echo "--go would REFUSE now: remote sessions on the board: $(fact who_remote)"
	fi
	exit 0
fi


# ---- 3. the run -------------------------------------------------------------
[ -n "${T6_HAND:-}" ] || refuse "--go needs T6_HAND=<who can cut the board's power>: an image that stops before it arms WDT0, or a run that stops at the U-Boot prompt, leaves the board there"
{ [ -n "${T6_RECORD_DIR:-}" ] && [ -d "$T6_RECORD_DIR" ]; } ||
	refuse "--go needs T6_RECORD_DIR=<an existing directory outside the checkout> for the run's record"
uart_ready
# The tool, without a board, before anything reaches it.
selftest=$(python3 -I "$PY" --self-test 2>&1 | tail -1) || die "prompt-go.py --self-test failed: $selftest"
case "$selftest" in
*", 0 failed") echo "$selftest" ;;
*) die "prompt-go.py --self-test: $selftest" ;;
esac

# Before the lock: board_lock.sh chains the EXIT trap it finds.
tmp=$(mktemp -d)
py_pid=
cleanup() {
	if [ -n "$py_pid" ]; then kill "$py_pid" 2>/dev/null || true; fi
	rm -rf "$tmp"
}
trap cleanup EXIT
# prompt-go.py's own bounds: 240 s to the autoboot line, a few minutes for
# the commands, image_wait for the image and 180 s for Linux; then up to
# 180 s here for ssh.
HOLD_MAX=${HOLD_MAX:-$((image_wait + 1200))}
LOCK_WAIT=${LOCK_WAIT:-0}
# shellcheck source=../kexec-zephyr/board_lock.sh
. "$K/board_lock.sh"

fresh=$(board "$WHO_REMOTE$STATE") || refuse "cannot read the board's state again under the lock"
fresh_fact() { printf '%s\n' "$fresh" | sed -n "s/^$1=//p" | head -1; }
[ "$(fresh_fact kexec_loaded)" = 0 ] || refuse "under the lock: a kexec image is loaded (/sys/kernel/kexec_loaded '$(fresh_fact kexec_loaded)')"
[ "$(fresh_fact staged)" = "$sha" ] || refuse "under the lock: $STAGE_DIR/$NAME is no longer this image"
[ -z "$(fresh_fact gpu_users)" ] || refuse "under the lock: processes hold the GPU device: $(fresh_fact gpu_users)"
if [ -n "$(fresh_fact who_remote)" ] && [ "${T6_ALLOW_SESSIONS:-0}" != 1 ]; then
	refuse "under the lock: remote sessions on the board: $(fresh_fact who_remote)-- the reboot ends them; ask whoever holds them, then T6_ALLOW_SESSIONS=1"
fi
old_boot=$(fresh_fact boot_id)
[ -n "$old_boot" ] || refuse "under the lock: the board gave no boot_id"

REC=$T6_RECORD_DIR/$(date +%Y%m%d-%H%M%S)-t6-uboot-go
mkdir "$REC" || die "cannot make the record directory $REC"
rec() { printf '%s %s\n' "$(date '+%F %T')" "$*" | tee -a "$REC/record.txt"; }
echo "prompt-go.sh: record $REC"
rec "build $(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"
rec "prompt-go.py sha256 $(sha256sum "$PY" | cut -d' ' -f1); $selftest"
rec "image $IMG as $STAGE_DIR/$NAME, $size bytes, crc32 $crc, sha256 $sha: $markers"
rec "probe $PROBE; the image ends by $image_wait s after go at the latest"
rec "board boot_id $old_boot, up $(fresh_fact uptime) s; hand at the power switch: $T6_HAND"
uart_record

python3 -I "$PY" run "$T6_UART" "$REC" "$NAME" "$size" "$crc" "$PROBE" "$image_wait" >"$tmp/py.out" 2>&1 &
py_pid=$!
wait_listening
reboot_and_wait_tool
rec "prompt-go.py exit $py_rc, $(( $(date +%s) - t0 )) s after the reboot command"
[ ! -f "$REC/verdict.txt" ] || rec "verdict: $(tail -n 1 "$REC/verdict.txt")"
# 6 and 10 say only that the UART showed no Linux login; Armbian back over
# ssh gives the code of the login seen: 8 after a refusal, and after the
# image 0 or 11 by its verdict (t6-verdict.sh's last line, "VERDICT <probe>
# PASS" exactly when it exits 0).
seen=11
[ "$(tail -n 1 "$REC/verdict.txt" 2>/dev/null)" != "VERDICT $PROBE PASS" ] || seen=0
exit_when_back 6:8 10:$seen
