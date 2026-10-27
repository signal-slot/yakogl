#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# nanopc-t6.sh - kexec a Zephyr image into the NanoPC-T6 from Armbian
# (driver/docs/nanopc-t6-mali.md, "Z1"). Development only: a result counts
# only when the board's boot loader started the image.
#
#   BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/nanopc-t6.sh <zephyr.bin>          # dry run
#   BOARD_HOST=root@<t6> T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=<tty> \
#     driver/tools/kexec-zephyr/nanopc-t6.sh <zephyr.bin> --go [--no-linux-wdt]     # the jump
#
# Without --go it checks the image on this host, makes read-only checks on the
# board, prints exactly what --go would do, and changes nothing anywhere.
# With --go it takes the board lock (board_lock.sh; it refuses, never waits,
# while anyone holds it: see LOCK_WAIT), reads the board's state
# again under the lock, starts the record (below), copies the loader, mmio_rd
# and the image to /tmp/yakogl-kexec on the board and jumps; then it waits for
# the board to come back into Armbian, prints how long that took and
# completes the record.
#
# Linux's watchdog across the jump (the default; --no-linux-wdt turns it off,
# --linux-wdt is accepted and changes nothing): the jump's ssh command opens
# /dev/watchdog0 right before kexec_zephyr and keeps it open across the jump.
# Linux's dw_wdt then starts WDT0 with its own period (timeout 44 s: TOP 14,
# 44.7 s from the open), and nothing in Linux feeds it after the open: the
# reboot notifier's dw_wdt_stop cannot stop it (no reset line) and only marks
# it running (linux v6.18 watchdog_core.c:158-176, dw_wdt.c:285-297;
# kernel_kexec runs the notifiers with SYS_RESTART = SYS_DOWN,
# reboot.c:100-106). So it covers the image's pre-arm window (reset.S, the MMU
# setup, the early init hooks: nanopc_t6.c, "Order"), which nothing of the
# image's own covers; the image's arm then re-arms the running WDT0. It is
# the default because the first jump (2026-10-19 00:26, without it) stopped
# in that window and held the board until its power was cut. The command
# proves the watchdog runs instead of assuming it: after the open it reads
# WDT0's WDT_CR and WDT_TORR with mmio_rd, prints them ("linux-wdt: WDT0 CR
# ..."), and jumps only when WDT_EN (bit 0) reads 1; otherwise it writes the
# magic close and exits 98 without jumping. That WDT0's timeout resets this
# SoC back into Armbian is still unproven (GLB_RST_ST after such a reset
# says). If
# kexec_zephyr returns instead, the command writes the magic close 'V' and
# closes the device, and Linux keeps feeding WDT0 from then on (the board's
# nowayout is 0; watchdog_dev.c:942-990, the worker for a running watchdog
# nobody has open, :76-96).
#
# The record: one directory per --go run under $T6_RECORD_DIR, printed at the
# start and at the end. uart.log (the board's debug UART, T6_UART, read
# only, from before the jump until the board is back in Armbian),
# record.txt (this script's lines, the hand, the registers before and
# after), linux-before.dmesg-w (`dmesg -w` of the old
# Linux, from before the jump until the connection drops), linux-after.dmesg
# (`dmesg` of the Linux that came back). The registers, read with mmio_rd:
# GLB_RST_ST 0xfd7c0c04, GLB_RST_CON 0xfd7c0c10, WDT0 WDT_CR and WDT_TORR.
#
# Why not kexec-tools: Armbian 26.8.10 does not ship it (no `kexec`, no
# kexec-tools package; read 2026-10-18), and it could not do this anyway:
# its arm64 Image and ELF loaders put the kernel at the first 2 MiB aligned
# hole that fits (kexec-tools v2.0.31 kexec/arch/arm64/kexec-arm64.c:699-701,
# kexec-image-arm64.c:70, 101-102), and a Zephyr image runs only at its link
# address. kexec_zephyr (kexec_zephyr.c, beside this) makes the kexec_load
# system call with one segment at that address; the T6's kernel has
# CONFIG_KEXEC=y and kexec_load_disabled 0.
#
# The image must carry the board glue's marker (driver/zephyr/lib/board/
# nanopc_t6/nanopc_t6.c, t6_marker): "yakogl-wdt: nanopc_t6 WDT0 armed at
# boot; top N; lifetime_s L; alive_s A; proof P; load ADDR". An image without
# it has no watchdog of its own (Armbian leaves WDT0 stopped), so a hang would
# hold the board until someone cuts the power: refused, exit 3.
#
# Environment:
#   BOARD_HOST      user@host of the T6, root (kexec_load needs CAP_SYS_BOOT and
#                   /proc/iomem shows addresses to root only). Required.
#   BOARD_SSH_OPTS  extra ssh/scp options (e.g. -o UserKnownHostsFile=...).
#   LOCK_WAIT       --go: seconds to wait for the board lock (default 0: refuse
#                   at once, naming the holder). A jump never waits onto a
#                   later session: a waiting jump starts the moment the
#                   holder lets go, on a board someone may have been handed
#                   meanwhile (2026-10-19: a jump rebooted the T6 under a
#                   gl_test session handed over by chat). Whoever wants to
#                   queue sets LOCK_WAIT, knowing that.
#   KEXEC_ZEPHYR    a built kexec_zephyr; otherwise it is built here with
#                   CROSS_GCC (default aarch64-linux-gnu-gcc), freestanding.
#   T6_HAND         --go only, required: the name of the person at the T6's
#                   power switch for this run (if PSCI and WDT0 both fail, the
#                   board stays in Zephyr until the power is cut). It goes
#                   into the record.
#   T6_RECORD_DIR   --go only, required: an existing directory, outside any
#                   checkout, in which the run's record directory is made.
#   T6_UART         --go only, required: the host's tty on the T6's debug UART
#                   (UART2, 1500000 8N1; e.g. /dev/serial/by-id/usb-FTDI_...).
#                   READ ONLY: Armbian runs an autologin root shell on ttyS2,
#                   so any byte written to it runs on the board as root. The
#                   script sets it raw with no echo (stty, which writes no
#                   data) and only reads it with cat; it refuses when the tty
#                   is absent, already open, or the settings do not hold.
#   MMIO_RD         a built mmio_rd; otherwise it is built like kexec_zephyr.
#   T6_ALLOW_SESSIONS=1  --go even when `who` lists remote (pts, sshd) sessions
#                   on the board (the jump ends them; ask whoever holds them
#                   first). Console logins (tty1, ttyS2, seat0: Armbian's
#                   autologin) are printed, not refused.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
# The window nanopc_t6.dts links the image into (zephyr,sram).
WINDOW_BASE=$((0x10000000))
WINDOW_SIZE=$((16 << 20))
STAGE=/tmp/yakogl-kexec

die() { echo "nanopc-t6.sh: $*" >&2; exit 2; }
refuse() { echo "nanopc-t6.sh: REFUSED: $*" >&2; echo "nanopc-t6.sh: nothing was copied, loaded or changed." >&2; exit 3; }

IMG=
GO=0
LINUX_WDT=1
for a in "$@"; do
	case "$a" in
	--go) GO=1 ;;
	--linux-wdt) LINUX_WDT=1 ;;
	--no-linux-wdt) LINUX_WDT=0 ;;
	-*) die "unknown option $a" ;;
	*) [ -z "$IMG" ] || die "one image only"; IMG=$a ;;
	esac
done
[ -n "$IMG" ] || die "usage: BOARD_HOST=root@<t6> $0 <zephyr.bin> [--go [--no-linux-wdt]]"
[ -f "$IMG" ] || die "$IMG: no such file"
: "${BOARD_HOST:?set BOARD_HOST=root@<the NanoPC-T6>}"
# shellcheck disable=SC2206  # options are words
SSH_OPTS=(${BOARD_SSH_OPTS:-})

echo "nanopc-t6.sh: build $(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"

# ---- 1. the image, on this host --------------------------------------------
magic=$(od -A n -t x1 -j 56 -N 4 "$IMG" | tr -d ' \n')
[ "$magic" = 41524d64 ] || refuse "$IMG is not an AArch64 Image (no ARM\\x64 at byte 56: CONFIG_AARCH64_IMAGE_HEADER)"
file_size=$(stat -c %s "$IMG")
footprint=$(od -A n -t u8 -j 16 -N 8 "$IMG" | tr -d ' ')
[ "$footprint" -ge "$file_size" ] || footprint=$file_size
memsz=$(( (footprint + 4095) / 4096 * 4096 ))

markers=$(strings -n 20 "$IMG" | grep '^yakogl-wdt: nanopc_t6 ' || true)
[ -n "$markers" ] || refuse "$IMG carries no 'yakogl-wdt: nanopc_t6' marker: its watchdog is not armed at boot (an image for another board, or built without driver/zephyr/lib/board/nanopc_t6)"
[ "$(printf '%s\n' "$markers" | wc -l)" -eq 1 ] || refuse "$IMG carries more than one watchdog marker: $markers"
strings -n 20 "$IMG" | grep -qxF 'board: watchdog armed' || refuse "$IMG lacks the 'board: watchdog armed' line"
re='^yakogl-wdt: nanopc_t6 WDT0 armed at boot; top ([0-9]+); lifetime_s ([0-9]+); alive_s ([0-9]+); proof ([01]); load (0x[0-9a-fA-F]+)$'
[[ "$markers" =~ $re ]] || refuse "the marker does not parse: $markers"
top=${BASH_REMATCH[1]} lifetime=${BASH_REMATCH[2]} alive=${BASH_REMATCH[3]} proof=${BASH_REMATCH[4]} load=${BASH_REMATCH[5]}
[ "$top" -ge 11 ] && [ "$top" -le 15 ] || refuse "WDT0 TOP $top outside 11..15"
[ $((load)) -eq "$WINDOW_BASE" ] || refuse "the image is linked at $load, not at the window $(printf 0x%x "$WINDOW_BASE") (nanopc_t6.dts)"
[ "$memsz" -le "$WINDOW_SIZE" ] || refuse "the image needs $memsz bytes, the window has $WINDOW_SIZE"
period_ms=$(( (1 << (16 + top)) * 1000 / 24000000 ))
if [ "$proof" = 1 ]; then
	bound_s=$(( (period_ms + 10000 + 999) / 1000 ))
	echo "image: PROOF build: WDT0 is never fed; it must reset the board ~$((period_ms / 1000)) s after boot, the PSCI backstop at $bound_s s"
else
	bound_s=$(( lifetime + (period_ms + 999) / 1000 ))
fi
echo "image: $IMG, $file_size bytes, RAM footprint $footprint bytes (segment $memsz), linked at $load"
if [ "$proof" = 1 ]; then
	echo "image: WDT0 TOP $top = $period_ms ms period, never fed: back in reset by $bound_s s after the jump at the latest"
else
	echo "image: WDT0 TOP $top = $period_ms ms period, fed until $lifetime s of uptime (alive window $alive s): back in reset by $bound_s s after the jump at the latest"
fi

# ---- 2. the board, read only -----------------------------------------------
board() { timeout 30 ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" "$@" </dev/null; }
facts=$(board 'set -u
printf "compatible=%s\n" "$(tr "\0" " " </proc/device-tree/compatible)"
printf "kernel=%s\n" "$(uname -r)"
printf "kexec_load_disabled=%s\n" "$(cat /proc/sys/kernel/kexec_load_disabled)"
printf "kexec_loaded=%s\n" "$(cat /sys/kernel/kexec_loaded)"
printf "config_kexec=%s\n" "$(grep -h "^CONFIG_KEXEC=" /boot/config-"$(uname -r)" 2>/dev/null || echo absent)"
printf "uid=%s\n" "$(id -u)"
printf "uptime=%s\n" "$(cut -d" " -f1 /proc/uptime)"
printf "system=%s\n" "$(systemctl is-system-running 2>/dev/null || true)"
printf "boot_id=%s\n" "$(cat /proc/sys/kernel/random/boot_id)"
for f in identity state timeout nowayout; do printf "wdt_%s=%s\n" "$f" "$(cat /sys/class/watchdog/watchdog0/$f 2>/dev/null || echo absent)"; done
printf "wdt_users=%s\n" "$(fuser /dev/watchdog /dev/watchdog0 2>/dev/null | tr -s " " || true)"
printf "gpu_users=%s\n" "$(fuser /dev/dri/* 2>/dev/null | tr -s " " || true)"
printf "who_remote=%s\n" "$(who | awk "\$2 ~ /^(pts|sshd)/ {print \$1\"@\"\$2\"(\"\$NF\")\"}" | tr "\n" " ")"
printf "who_local=%s\n" "$(who | awk "\$2 !~ /^(pts|sshd)/ {print \$1\"@\"\$2}" | tr "\n" " ")"
printf "gic_tables=%s\n" "$(dmesg | grep -oE "(LPI (property|pending) table|Devices|Interrupt Collections) @(0x)?[0-9a-f]+" | sed "s/.*@//; s/^0x//" | tr "\n" " ")"
echo "iomem<<"; cat /proc/iomem; echo ">>"') || refuse "cannot read the board's facts over ssh ($BOARD_HOST)"
fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p" | head -1; }

case "$(fact compatible)" in *friendlyarm,nanopc-t6*) ;; *) refuse "$BOARD_HOST is not a NanoPC-T6: compatible '$(fact compatible)'" ;; esac
[ "$(fact uid)" = 0 ] || refuse "$BOARD_HOST is not root (kexec_load needs CAP_SYS_BOOT; /proc/iomem hides addresses)"
[ "$(fact kexec_load_disabled)" = 0 ] || refuse "kexec_load_disabled is $(fact kexec_load_disabled)"
[ "$(fact config_kexec)" = CONFIG_KEXEC=y ] || refuse "the kernel $(fact kernel) has $(fact config_kexec)"
[ "$(fact kexec_loaded)" = 0 ] || refuse "a kexec image is already loaded (/sys/kernel/kexec_loaded 1): someone else is about to jump"

# The window: inside one top-level "System RAM" range, overlapping no entry
# under it (the kernel image, reserved memory).
iomem=$(printf '%s\n' "$facts" | sed -n '/^iomem<<$/,/^>>$/p' | sed '1d;$d')
printf '%s\n' "$iomem" | grep -qE '^ *[0-9a-f]*[1-9a-f][0-9a-f]*-' || refuse "/proc/iomem shows no addresses (not root?)"
wb=$WINDOW_BASE we=$((WINDOW_BASE + WINDOW_SIZE - 1))
inside=0 parent_s=-1 parent_e=-1 clash=
while IFS= read -r line; do
	[[ "$line" =~ ^(\ *)([0-9a-f]+)-([0-9a-f]+)\ :\ (.*)$ ]] || continue
	ind=${#BASH_REMATCH[1]} s=$((16#${BASH_REMATCH[2]})) e=$((16#${BASH_REMATCH[3]})) name=${BASH_REMATCH[4]}
	if [ "$ind" -eq 0 ]; then
		parent_s=$s parent_e=$e
		if [ "$name" = "System RAM" ] && [ "$s" -le "$wb" ] && [ "$e" -ge "$we" ]; then inside=1; fi
		if [ "$name" != "System RAM" ] && [ "$s" -le "$we" ] && [ "$e" -ge "$wb" ]; then clash="$clash; $line"; fi
	elif [ "$parent_s" -le "$wb" ] && [ "$parent_e" -ge "$we" ] && [ "$s" -le "$we" ] && [ "$e" -ge "$wb" ]; then
		clash="$clash; ${line#"${line%%[! ]*}"}"
	fi
done <<<"$iomem"
[ "$inside" = 1 ] || refuse "the window $(printf '0x%x-0x%x' "$wb" "$we") is not inside one System RAM range of /proc/iomem"
[ -z "$clash" ] || refuse "the window overlaps:${clash#;}"

# The GIC's ITS and LPI tables: the redistributors and ITSs Linux enabled keep
# them (EnableLPIs cannot be cleared again) and write the pending tables when an
# LPI arrives, after the jump too. Their addresses are only in Linux's log.
gic=$(fact gic_tables)
gic_n=0
for t in $gic; do
	a=$((16#$t)) gic_n=$((gic_n + 1))
	if [ "$a" -le "$we" ] && [ $((a + 0x10000)) -gt "$wb" ]; then
		refuse "a GIC ITS/LPI table at 0x$t (64 KiB) is inside the window (Linux's dmesg)"
	fi
done
echo "board: $BOARD_HOST: $(fact compatible), Linux $(fact kernel), $(fact config_kexec), kexec_load_disabled 0, nothing loaded"
echo "board: up $(fact uptime) s, systemd '$(fact system)', boot_id $(fact boot_id)"
echo "board: window $(printf '0x%08x-0x%08x' "$wb" "$we") is inside System RAM and overlaps no /proc/iomem entry"
if [ "$gic_n" -gt 0 ]; then
	echo "board: the $gic_n GIC ITS/LPI tables Linux's log names are outside the window ($(printf '%s' "$gic" | tr ' ' ',' | sed 's/,$//'))"
else
	echo "board: Linux's log names no GIC ITS/LPI table (the ring wrapped?): not checked"
fi
echo "board: WDT0 under Linux: $(fact wdt_identity), state $(fact wdt_state), timeout $(fact wdt_timeout) s, nowayout $(fact wdt_nowayout), open by '$(fact wdt_users)' (the image arms it itself either way)"
if [ "$LINUX_WDT" = 1 ]; then
	[ "$(fact wdt_identity)" != absent ] || refuse "linux-wdt: the board has no /sys/class/watchdog/watchdog0 (--no-linux-wdt jumps without it)"
	[ -z "$(fact wdt_users)" ] || refuse "linux-wdt: /dev/watchdog0 is already open by '$(fact wdt_users)'"
	echo "board: linux-wdt (default): /dev/watchdog0 is opened right before the jump, WDT_CR.WDT_EN is read back, and the jump happens only when it is 1: WDT0 then resets the board about $(fact wdt_timeout) s after the open unless the image re-arms it (that its reset brings the SoC back is unproven)"
else
	echo "board: --no-linux-wdt: nothing covers the image's pre-arm window; a hang there holds the board until its power is cut"
fi
if [ -n "${T6_UART:-}" ]; then
	if [ -c "$T6_UART" ]; then
		echo "uart: T6_UART $T6_UART is a character device (not opened in a dry run; --go reads it only)"
	else
		echo "uart: T6_UART $T6_UART is absent: --go would refuse"
	fi
else
	echo "uart: T6_UART is not set: --go would refuse"
fi
echo "board: GPU device users '$(fact gpu_users)'; remote sessions '$(fact who_remote)', console logins '$(fact who_local)'"

# ---- 3. the plan ------------------------------------------------------------
open_wdt=
if [ "$LINUX_WDT" = 1 ]; then
	open_wdt="exec 3>/dev/watchdog0 (Linux's dw_wdt starts WDT0 for $(fact wdt_timeout) s), read WDT_CR/WDT_TORR (no jump unless WDT_EN is 1), then "
fi
cat <<EOF
plan (--go):
  1. take the board lock (driver/tools/kexec-zephyr/board_lock.sh, per BOARD_HOST);
     read kexec_loaded, the GPU users, the remote sessions and boot_id again under
     it and decide on those values; refuse without T6_HAND, T6_RECORD_DIR and
     T6_UART
  2. the record: \$T6_RECORD_DIR/<date>-t6/, with the hand at the power switch;
     the UART (\$T6_UART, read only) into it until the board is back, and
     \`dmesg -w\` of the board until the jump
  3. copy kexec_zephyr, mmio_rd and $(basename "$IMG") to $BOARD_HOST:$STAGE/, then
     sync; read GLB_RST_ST, GLB_RST_CON, WDT0 CR and TORR into the record
  4. on the board: ${open_wdt}$STAGE/kexec_zephyr $STAGE/zephyr.bin $load
     = kexec_load(entry $load, 1 segment: $file_size bytes at $load, memsz $memsz)
       then reboot(LINUX_REBOOT_CMD_KEXEC): Linux shuts its devices down (PCI bus
       mastering cleared), stops the other CPUs and enters the image at EL2 with
       the MMU off and its VHE HCR_EL2 (E2H set; the image's first code
       rewrites it); this ssh session ends there
  5. the image, on UART2 (1500000 baud, into uart.log): the raw early lines
     T6:A (first code, EL2 state), T6:A1 (EL1 init), T6:A2 and T6:A3 (MMU on,
     WDT0 armed), then through the console T6:B, the marker, "t6: WDT0 armed ...",
     T6:C, "board: watchdog armed", then the sample (T6:X: an exception taken
     to EL2, the image halted); it ends with PSCI SYSTEM_RESET, or WDT0 resets
     the board by $bound_s s
  6. here: wait for ssh and a new boot_id (up to $((bound_s + 180)) s), print the
     time; save the new Linux's dmesg and the four registers to the record
EOF
if [ "$GO" = 0 ]; then
	[ "$LINUX_WDT" = 0 ] || echo "dry run: the watchdog is opened only with --go"
	echo "dry run: nothing was copied, loaded or changed (add --go to jump)"
	exit 0
fi

# ---- 4. the jump ------------------------------------------------------------
[ -n "${T6_HAND:-}" ] || refuse "--go needs T6_HAND=<the name of the person at the T6's power switch for this run>"
{ [ -n "${T6_RECORD_DIR:-}" ] && [ -d "$T6_RECORD_DIR" ]; } ||
	refuse "--go needs T6_RECORD_DIR=<an existing directory outside the checkout> for the run's record"
[ -n "${T6_UART:-}" ] || refuse "--go needs T6_UART=<the host tty on the T6's debug UART> (read only)"
[ -c "$T6_UART" ] && [ -r "$T6_UART" ] || refuse "T6_UART $T6_UART is not a readable character device"
if fuser "$T6_UART" >/dev/null 2>&1; then
	refuse "T6_UART $T6_UART is already open (pids $(fuser "$T6_UART" 2>/dev/null | tr -s " " | cut -c1-80)): a second reader would split the bytes; stop the other capture first"
fi
# raw, no echo: with echo on, the tty would send every received byte back
# to the board's root shell.
UART_STTY=(1500000 raw -echo -icrnl -ixon cs8 -cstopb -parenb clocal)
stty -F "$T6_UART" "${UART_STTY[@]}" || refuse "stty on $T6_UART failed"
uart_mode=$(stty -F "$T6_UART" -a | tr "\n;" "  ") || refuse "stty -a on $T6_UART failed"
for w in 'speed 1500000' ' -echo ' ' -icrnl ' ' -ixon ' ' cs8 ' ' -cstopb ' ' -parenb ' ' clocal ' ' -opost ' ' -icanon '; do
	[[ " $uart_mode " == *"$w"* ]] || refuse "$T6_UART does not read back '${w# }' after stty: $uart_mode"
done
# Before the lock: board_lock.sh chains the EXIT trap it finds.
tmp=$(mktemp -d)
dmesg_pid=
uart_pid=
cleanup() {
	if [ -n "$dmesg_pid" ]; then kill "$dmesg_pid" 2>/dev/null || true; fi
	if [ -n "$uart_pid" ]; then kill "$uart_pid" 2>/dev/null || true; fi
	rm -rf "$tmp"
}
trap cleanup EXIT
HOLD_MAX=${HOLD_MAX:-$((bound_s + 600))}
LOCK_WAIT=${LOCK_WAIT:-0}
# shellcheck source=board_lock.sh
. "$HERE/board_lock.sh"

# Under the lock, the state the jump depends on is read again: the facts
# above were read before the lock, and the wait for it can be LOCK_WAIT long
# when the caller set one.
fresh=$(board 'printf "kexec_loaded=%s\n" "$(cat /sys/kernel/kexec_loaded)"
printf "boot_id=%s\n" "$(cat /proc/sys/kernel/random/boot_id)"
printf "gpu_users=%s\n" "$(fuser /dev/dri/* 2>/dev/null | tr -s " " || true)"
printf "wdt_users=%s\n" "$(fuser /dev/watchdog /dev/watchdog0 2>/dev/null | tr -s " " || true)"
printf "who_remote=%s\n" "$(who | awk "\$2 ~ /^(pts|sshd)/ {print \$1\"@\"\$2\"(\"\$NF\")\"}" | tr "\n" " ")"') ||
	refuse "cannot read the board's state again under the lock"
fresh_fact() { printf '%s\n' "$fresh" | sed -n "s/^$1=//p" | head -1; }
[ "$(fresh_fact kexec_loaded)" = 0 ] || refuse "under the lock: a kexec image is already loaded (/sys/kernel/kexec_loaded '$(fresh_fact kexec_loaded)')"
[ -z "$(fresh_fact gpu_users)" ] || refuse "under the lock: processes hold the GPU device: $(fresh_fact gpu_users)"
if [ -n "$(fresh_fact who_remote)" ] && [ "${T6_ALLOW_SESSIONS:-0}" != 1 ]; then
	refuse "under the lock: remote sessions on the board: $(fresh_fact who_remote)-- the jump ends them; ask whoever holds them, then T6_ALLOW_SESSIONS=1"
fi
if [ "$LINUX_WDT" = 1 ] && [ -n "$(fresh_fact wdt_users)" ]; then
	refuse "under the lock: linux-wdt: /dev/watchdog0 is open by '$(fresh_fact wdt_users)'"
fi
old_boot=$(fresh_fact boot_id)
[ -n "$old_boot" ] || refuse "under the lock: the board gave no boot_id"

REC=$T6_RECORD_DIR/$(date +%Y%m%d-%H%M%S)-t6
mkdir "$REC" || die "cannot make the record directory $REC"
rec() { printf '%s %s\n' "$(date '+%F %T')" "$*" | tee -a "$REC/record.txt"; }
echo "nanopc-t6.sh: record $REC"
rec "build $(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"
rec "board $BOARD_HOST, boot_id $old_boot; hand at the power switch: $T6_HAND; linux_wdt $LINUX_WDT"
rec "image $IMG, sha256 $(sha256sum "$IMG" | cut -d' ' -f1): $markers"
rec "bound: back in reset by $bound_s s after the jump at the latest"

build_tool() {
	"${CROSS_GCC:-aarch64-linux-gnu-gcc}" -O2 -static -nostdlib -ffreestanding -fno-stack-protector \
		-o "$tmp/$1" "$HERE/$1.c" && printf '%s\n' "$tmp/$1"
}
LOADER=${KEXEC_ZEPHYR:-}
[ -n "$LOADER" ] || LOADER=$(build_tool kexec_zephyr) || die "cannot build kexec_zephyr"
MMIO=${MMIO_RD:-}
[ -n "$MMIO" ] || MMIO=$(build_tool mmio_rd) || die "cannot build mmio_rd"

# The UART, read only (cat opens it O_RDONLY), from before the jump until
# the board is back in Armbian.
cat <"$T6_UART" >"$REC/uart.log" &
uart_pid=$!
rec "UART capture started (pid $uart_pid, read only): $T6_UART -> $REC/uart.log"

# dmesg -w of the old Linux, from here until the jump drops the connection
# (ServerAlive ends it a few seconds after the board stops answering).
timeout $((bound_s + 300)) ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=3 \
	-o ServerAliveCountMax=2 "$BOARD_HOST" 'dmesg -w' </dev/null >"$REC/linux-before.dmesg-w" 2>&1 &
dmesg_pid=$!
rec "dmesg -w capture started (pid $dmesg_pid): $REC/linux-before.dmesg-w"

stage_mmio() {
	board "mkdir -p $STAGE" && bscp "${SSH_OPTS[@]}" "$MMIO" "$BOARD_HOST:$STAGE/mmio_rd"
}
# GLB_RST_ST, GLB_RST_CON, then WDT0's WDT_CR and WDT_TORR. Never WDT_EOI
# (0x14): a read of it clears the interrupt.
read_regs() {
	board "$STAGE/mmio_rd 0xfd7c0c04 && $STAGE/mmio_rd 0xfd7c0c10 && $STAGE/mmio_rd 0xfeaf0000 2" | tr '\n' ' '
}
# What the GPU's power looks like to Linux (driver/docs/nanopc-t6-mali.md,
# "Z1"): the runtime PM state, the PMU's PD_GPU registers (PWR_GATE 0x14c,
# BUS_IDLE_REQ 0x10c, _ACK 0x118, _ST 0x120, REPAIR_STATUS 0x290), the
# CRU's CLKSEL_CON(158), (159) and CLKGATE_CON(66), and the RK806's
# POWER_EN0, POWER_SLP_EN0, BUCK1_CONFIG and BUCK1_ON_VSEL from the regmap
# cache Linux keeps (debugfs). All always-on blocks: nothing behind PD_GPU
# is read.
read_gpu_power() {
	board "printf 'runtime %s; ' \$(cat /sys/bus/platform/devices/fb000000.gpu/power/runtime_status)
for a in 0xfd8d814c 0xfd8d810c 0xfd8d8118 0xfd8d8120 0xfd8d8290 0xfd7c0578 0xfd7c057c 0xfd7c0908; do $STAGE/mmio_rd \$a; done | tr '\n' ' '
printf 'rk806 '; grep -E '^(00|06|10|1a):' /sys/kernel/debug/regmap/spi2.0/registers | tr '\n' ' '"
}
board "rm -rf $STAGE && mkdir -p $STAGE"
bscp "${SSH_OPTS[@]}" "$LOADER" "$BOARD_HOST:$STAGE/kexec_zephyr"
stage_mmio
bscp "${SSH_OPTS[@]}" "$IMG" "$BOARD_HOST:$STAGE/zephyr.bin"
want=$(sha256sum "$IMG" "$LOADER" | cut -d' ' -f1 | tr '\n' ' ')
have=$(board "sha256sum $STAGE/zephyr.bin $STAGE/kexec_zephyr | cut -d' ' -f1 | tr '\n' ' '; sync")
[ "$want" = "$have" ] || { rec "the copies on the board differ (want $want, have $have); not jumping" >&2; exit 1; }
regs=$(read_regs) || { rec "cannot read the registers with mmio_rd; not jumping" >&2; exit 1; }
rec "registers before (GLB_RST_ST, GLB_RST_CON, WDT0 CR, TORR): $regs"
rec "gpu power before: $(read_gpu_power || echo unread)"

if [ "$LINUX_WDT" = 1 ]; then
	# WDT_CR (0xfeaf0000) and WDT_TORR (+4) after the open, as mmio_rd
	# prints them ("0xfeaf0000: 0x00000009"); WDT_EN is bit 0. The sleep
	# lets sshd send these lines before the kexec ends the connection.
	jump="sync; exec 3>/dev/watchdog0 || { echo 'linux-wdt: cannot open /dev/watchdog0'; exit 97; }
echo \"linux-wdt: /dev/watchdog0 open: state \$(cat /sys/class/watchdog/watchdog0/state), timeout \$(cat /sys/class/watchdog/watchdog0/timeout) s\"
regs=\$($STAGE/mmio_rd 0xfeaf0000 2 | tr '\\n' ' ')
cr=\$(printf '%s\\n' \"\$regs\" | sed -n 's/.*0xfeaf0000: \\(0x[0-9a-f]*\\).*/\\1/p')
torr=\$(printf '%s\\n' \"\$regs\" | sed -n 's/.*0xfeaf0004: \\(0x[0-9a-f]*\\).*/\\1/p')
if [ -z \"\$cr\" ] || [ \$(( cr & 1 )) -ne 1 ]; then
  echo \"linux-wdt: WDT0 CR \${cr:-unread} TORR \${torr:-unread} after the open: WDT_EN is not 1, not jumping (\$regs)\"
  printf V >&3; exec 3>&-; exit 98
fi
echo \"linux-wdt: WDT0 CR \$cr TORR \$torr after the open: WDT_EN 1, jumping\"
sleep 1
$STAGE/kexec_zephyr $STAGE/zephyr.bin $load; rc=\$?
printf V >&3; exec 3>&-
echo \"linux-wdt: kexec_zephyr returned \$rc: magic close written, Linux feeds WDT0 from now on\"
exit \$rc"
else
	jump="sync; $STAGE/kexec_zephyr $STAGE/zephyr.bin $load"
fi
t0=$(date +%s)
rec "jumping (linux_wdt $LINUX_WDT)"
jump_rc=0
timeout 30 ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 -o ServerAliveInterval=3 \
	-o ServerAliveCountMax=2 "$BOARD_HOST" "$jump" </dev/null >"$tmp/jump.out" 2>&1 || jump_rc=$?
cat "$tmp/jump.out" >>"$REC/record.txt"
refused_on_board=0
if [ "$LINUX_WDT" = 1 ]; then
	# The proof is the board's own line, read back from what arrived.
	proof=$(grep -m1 '^linux-wdt: WDT0 CR ' "$tmp/jump.out" || true)
	if [ "$jump_rc" = 98 ]; then
		rec "linux-wdt: REFUSED on the board, no jump: $proof"
		refused_on_board=1
	elif [[ "$proof" == *"WDT_EN 1, jumping"* ]]; then
		rec "linux-wdt: WDT_EN proof (the board's read after the open, before kexec_zephyr): $proof"
	else
		rec "linux-wdt: no WDT_EN proof line arrived (ssh rc $jump_rc): whether WDT0 ran at the jump is unknown"
	fi
fi

# The capture ends by itself when the old Linux goes; it is stopped here in
# any case, by its own pid.
finish() {
	if [ -n "$dmesg_pid" ]; then
		kill "$dmesg_pid" 2>/dev/null || true
		wait "$dmesg_pid" 2>/dev/null || true
		dmesg_pid=
	fi
	if [ -n "$uart_pid" ]; then
		kill "$uart_pid" 2>/dev/null || true
		wait "$uart_pid" 2>/dev/null || true
		uart_pid=
	fi
	rec "linux-before.dmesg-w: $(wc -l <"$REC/linux-before.dmesg-w") lines; uart.log: $(wc -c <"$REC/uart.log") bytes"
	echo "nanopc-t6.sh: record $REC"
}
if [ "$refused_on_board" = 1 ]; then
	finish
	exit 1
fi
deadline=$((t0 + bound_s + 180))
while [ "$(date +%s)" -lt "$deadline" ]; do
	sleep 5
	now_boot=$(board 'cat /proc/sys/kernel/random/boot_id' 2>/dev/null || true)
	if [ -n "$now_boot" ] && [ "$now_boot" != "$old_boot" ]; then
		rec "back in Armbian $(( $(date +%s) - t0 )) s after the jump (boot_id $now_boot, up $(board 'cut -d" " -f1 /proc/uptime') s)"
		# The reset cause first, before anything else runs on the board
		# (the boot cleared /tmp/yakogl-kexec, so mmio_rd is staged again).
		if stage_mmio && regs=$(read_regs); then
			rec "registers after (GLB_RST_ST, GLB_RST_CON, WDT0 CR, TORR): $regs"
			rec "gpu power after: $(read_gpu_power || echo unread)"
		else
			rec "could not read the registers after the return"
		fi
		rec "watchdog0 bootstatus after: $(board 'cat /sys/class/watchdog/watchdog0/bootstatus' 2>/dev/null || echo unread)"
		board dmesg >"$REC/linux-after.dmesg" 2>&1 || rec "could not save the new Linux's dmesg"
		rec "linux-after.dmesg: $(wc -l <"$REC/linux-after.dmesg") lines"
		finish
		exit 0
	fi
	# The old Linux answers for a few seconds while it shuts down.
	if [ -n "$now_boot" ] && [ $(( $(date +%s) - t0 )) -ge 60 ]; then
		rec "the same Linux still runs 60 s after the jump (boot_id unchanged): the jump did not happen" >&2
		finish
		exit 1
	fi
done
rec "no Armbian $((deadline - t0)) s after the jump: the board needs $T6_HAND at its power switch" >&2
finish
exit 1
