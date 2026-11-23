#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# nanopc-t6.sh - kexec a Zephyr image into the NanoPC-T6 from Armbian
# (driver/docs/nanopc-t6-mali.md, "Z1"). Development only: a result counts
# only when the board's boot loader started the image.
#
#   BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/nanopc-t6.sh <zephyr.bin>          # dry run
#   BOARD_HOST=root@<t6> T6_HAND=<name> T6_RECORD_DIR=<dir> T6_UART=<tty> \
#     driver/tools/kexec-zephyr/nanopc-t6.sh <zephyr.bin> --go [--no-linux-wdt] [--crash]  # the jump
#
# Without --go it checks the image on this host, makes read-only checks on the
# board, prints exactly what --go would do, and changes nothing anywhere.
#
# Two routes (the dry run prints which one it would take):
#   normal (the default): kexec_load, then reboot(LINUX_REBOOT_CMD_KEXEC).
#           Linux runs device_shutdown() first, so the display is off when the
#           image starts (driver/docs/nanopc-t6-display.md, section 2(a)).
#           The window may lie in the crashkernel= reservation (the kernel
#           limits only crash segments to it, kernel/kexec_core.c:200-210)
#           while no crash image is loaded; that is read again under the lock.
#   --crash: kexec_load with KEXEC_ON_CRASH, then a panic (sysrq c) takes the
#           crash route: __crash_kexec runs no device_shutdown
#           (linux v6.18 kernel/crash_core.c:120-138, kernel/panic.c:504-509),
#           so the display Linux drives stays on for the image to adopt. The
#           segment must lie in the crashkernel= reservation
#           (kernel/kexec_core.c:200-210; kernel/kexec.c:31-37 for the entry),
#           and the kernel puts one 4 KiB control page in the reservation
#           outside the segment (kernel/kexec.c:65-71, kexec_core.c:395-450).
#           So --crash refuses unless /proc/iomem's "Crash kernel" range holds
#           the whole window and the image's segment plus that page; the
#           reservation is made with t6-crashkernel.sh, beside this. The GPU
#           and its firmware also stay running (section 2(a)). --crash with
#           --no-linux-wdt is refused (below).
#           The image adopts the modes of ADOPT_MODES below only, and Linux
#           runs the sink's preferred mode. So before the jump --crash runs
#           t6-kms-mode.py, beside this, on the board: it holds the mode VP0
#           runs when that is an adopted one (no size change: an HDMI
#           capture unit gives no picture after one), else it sets 1024x768,
#           on HDMI-A-1 as Linux's DRM master. The hold keeps fbdev from
#           changing the screen until the jump, and the jump ends Linux with
#           that mode in the hardware. Then the script reads VP0's mode back
#           from Linux (/sys/kernel/debug/dri/0/state) and jumps only when
#           it is the mode held. On every path that does not jump it stops the
#           holder by its exact pid, and the holder gives Linux its own mode
#           back (driver/docs/nanopc-t6-display.md, section 2(a), "The
#           mode before the jump"). So --crash refuses without python3 on
#           the board or without that debugfs file.
# With --go it takes the board lock (board_lock.sh; it refuses, never waits,
# while anyone holds it: see LOCK_WAIT), reads the board's state
# again under the lock, proves the UART path (see T6_UART), starts the
# record (below), copies the loader, mmio_rd
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
# reboot.c:100-106). So it covers the image's pre-arm window (reset.S up to
# the end of Zephyr's EL1 init: nanopc_t6.c, "Order"), which nothing of the
# image's own covers; the image's arm then re-arms the running WDT0. It is
# the default because the first jump (2026-10-19 00:26, without it) stopped
# in that window and held the board until its power was cut. The command
# proves the watchdog runs instead of assuming it: after the open it reads
# WDT0's WDT_CR and WDT_TORR with mmio_rd, prints them ("linux-wdt: WDT0 CR
# ..."), and jumps only when WDT_EN (bit 0) reads 1; otherwise it writes the
# magic close and exits 98 without jumping. A WDT0 timeout brings the SoC
# back through U-Boot into Armbian (measured 2026-10-19, driver/docs/
# nanopc-t6-mali.md, "The watchdog proof and the trap"). If
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
#                   Then, under the lock and before anything is copied, it
#                   proves the path: a temporary capture reads the tty, the
#                   board writes one marker line with a per-run nonce to its
#                   serial console (the last console=ttyS entry of its
#                   kernel command line; no such entry is refused), and the
#                   capture must read the line back within 5 s. Otherwise
#                   it refuses (exit 3) and names the bytes read and whether
#                   the board's write succeeded (2026-11-22: a jump with
#                   uart.log 0 bytes, the wire delivering nothing).
#   MMIO_RD         a built mmio_rd; otherwise it is built like kexec_zephyr.
#   T6_ALLOW_SESSIONS=1  --go even when `who` lists remote (pts, sshd) sessions
#                   on the board other than this script's own ssh command
#                   (who_remote.sh; the jump ends them; ask whoever holds them
#                   first). Console logins (tty1, ttyS2, seat0: Armbian's
#                   autologin) are printed, not refused.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
# The window nanopc_t6.dts links the image into (zephyr,sram).
# board_lock_selftest.sh, beside this, fails when these differ from it.
WINDOW_BASE=$((0x10000000))
WINDOW_SIZE=$((256 << 20))
STAGE=/tmp/yakogl-kexec
# The modes the image adopts (board::nanopc_t6_display::MODES; the crash
# route only), one per line: the name t6-kms-mode.py --mode takes, the
# clock in kHz, h and v in the order the DRM state prints them, and the
# sync flags (the flags' low six bits: 0xa both negative, 0x5 both
# positive, not interlaced). The first is the one set when VP0 runs none of
# them. board_lock_selftest.sh, beside this, fails when these differ from
# the Rust MODES.
ADOPT_MODES="1024x768 65000 1024 1048 1184 1344 768 771 777 806 0xa
1920x1080 148500 1920 2008 2052 2200 1080 1084 1089 1125 0x5"
# How long t6-kms-mode.py holds the mode at most: it gives it back by
# itself after that, also when this script is killed before it can stop it.
MODE_HOLD=600
# --go: how long the UART proof waits for the marker line the board wrote
# to its console (see T6_UART). At 1500000 baud the line takes well under
# a millisecond; the ssh command that writes it returns before the wait.
UART_PROOF_S=5

die() { echo "nanopc-t6.sh: $*" >&2; exit 2; }
refuse() { echo "nanopc-t6.sh: REFUSED: $*" >&2; echo "nanopc-t6.sh: nothing was copied, loaded or changed." >&2; exit 3; }

IMG=
GO=0
LINUX_WDT=1
CRASH=0
for a in "$@"; do
	case "$a" in
	--go) GO=1 ;;
	--crash) CRASH=1 ;;
	--linux-wdt) LINUX_WDT=1 ;;
	--no-linux-wdt) LINUX_WDT=0 ;;
	-*) die "unknown option $a" ;;
	*) [ -z "$IMG" ] || die "one image only"; IMG=$a ;;
	esac
done
[ -n "$IMG" ] || die "usage: BOARD_HOST=root@<t6> $0 <zephyr.bin> [--go [--no-linux-wdt]] [--crash]"
[ -f "$IMG" ] || die "$IMG: no such file"
# The crash route needs Linux's watchdog: if the panic does not take the
# crash kexec, panic() ends in its timeout loop (linux v6.18 kernel/panic.c:
# 508-509, :557), which never resets the board when panic_timeout is 0;
# only WDT0, open and no longer fed, does.
if [ "$CRASH" = 1 ] && [ "$LINUX_WDT" = 0 ]; then
	echo "nanopc-t6.sh: REFUSED: --crash needs the Linux watchdog (drop --no-linux-wdt): if the crash kexec does not happen, only WDT0 brings the board back" >&2
	exit 3
fi
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

# The image's strings are read once and matched from here-strings: under
# pipefail, `strings | grep -q` is 141 (strings killed by SIGPIPE) when grep
# leaves at a match before strings has finished, which read as "no such line".
strs=$(strings -n 20 "$IMG")
markers=$(grep '^yakogl-wdt: nanopc_t6 ' <<<"$strs" || true)
[ -n "$markers" ] || refuse "$IMG carries no 'yakogl-wdt: nanopc_t6' marker: its watchdog is not armed at boot (an image for another board, or built without driver/zephyr/lib/board/nanopc_t6)"
[ "$(printf '%s\n' "$markers" | wc -l)" -eq 1 ] || refuse "$IMG carries more than one watchdog marker: $markers"
grep -qxF 'board: watchdog armed' <<<"$strs" || refuse "$IMG lacks the 'board: watchdog armed' line"
re='^yakogl-wdt: nanopc_t6 WDT0 armed at boot; top ([0-9]+); lifetime_s ([0-9]+); alive_s ([0-9]+); proof ([01]); load (0x[0-9a-fA-F]+)$'
[[ "$markers" =~ $re ]] || refuse "the marker does not parse: $markers"
top=${BASH_REMATCH[1]} lifetime=${BASH_REMATCH[2]} alive=${BASH_REMATCH[3]} proof=${BASH_REMATCH[4]} load=${BASH_REMATCH[5]}
[ "$top" -ge 11 ] && [ "$top" -le 15 ] || refuse "WDT0 TOP $top outside 11..15"
[ $((load)) -eq "$WINDOW_BASE" ] || refuse "the image is linked at $load, not at the window $(printf 0x%x "$WINDOW_BASE") (nanopc_t6.dts)"
[ "$memsz" -le "$WINDOW_SIZE" ] || refuse "the image needs $memsz bytes, the window has $WINDOW_SIZE"
# kexec_zephyr reads a file into a 64 MiB buffer (IMG_MAX) and refuses a
# larger one. A __nocache static lies in the file's span (Zephyr places the
# NOLOAD nocache section before the kernel objects' initialised sections,
# k_timer_area and the others), so such a file can be far larger than its
# code and data; the Mali OSAL's memory is .noinit and is not in the file.
[ "$file_size" -le $((64 << 20)) ] || refuse "$IMG is $file_size bytes; kexec_zephyr reads at most 64 MiB (IMG_MAX)"
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
# who_remote, defined on the board ahead of the commands that call it: the
# remote sessions without this ssh command's own (who_remote.sh).
WHO_REMOTE=$(cat "$HERE/who_remote.sh") || die "cannot read $HERE/who_remote.sh"
# vp0_mode, defined on the board the same way: VP0's lines of the DRM state
# on one line, "enable=1 active=1 mode: "1024x768": 60 65000 1024 1048 1184
# 1344 768 771 777 806 0x40 0xa" (linux v6.18 drm_atomic.c:443-455, the
# format DRM_MODE_FMT of include/drm/drm_modes.h:425-435: name, refresh,
# clock, h, v, type, flags). The CRTC's name is the port's
# (rockchip_drm_vop2.c:2457-2459). Empty when the file cannot be read.
# hdmi_out: HDMI-A-1's output_bpc, output_format and tmds_char_rate from the
# same file (drm_atomic.c:1198-1207), for the record.
VP0_FN=$(cat <<'EOF'
vp0_mode() {
	awk '/^crtc\[/ { a = ($2 == "video_port0"); next } /^[^ \t]/ { a = 0 }
		a && $1 ~ /^(enable|active)=/ { printf "%s ", $1 } a && $1 == "mode:" { print; exit }' \
		/sys/kernel/debug/dri/0/state 2>/dev/null | tr -s " \t" " "
}
hdmi_out() {
	awk '/^connector\[/ { a = ($2 == "HDMI-A-1"); next } /^[^ \t]/ { a = 0 }
		a && $1 ~ /^(output_bpc|output_format|tmds_char_rate)=/ { printf "%s ", $1 }' \
		/sys/kernel/debug/dri/0/state 2>/dev/null
}
EOF
)
# vp0_timing <a vp0_mode line>: "<clock> <h x4> <v x4> <flags>" when VP0
# is on, the flags' low six bits (both syncs' polarities, interlace, double
# scan); nothing when it is off or the line does not parse.
vp0_timing() {
	local re='^enable=1 active=1 mode: "[^"]*": [0-9]+ ([0-9]+( [0-9]+){8}) 0x[0-9a-f]+ 0x([0-9a-f]+)$'
	[[ "$1" =~ $re ]] || return 0
	printf '%s 0x%x\n' "${BASH_REMATCH[1]}" $((0x${BASH_REMATCH[3]} & 0x3f))
}
# adopt_name <a vp0_mode line>: the name of the adopted mode VP0 runs;
# nothing when it runs none of ADOPT_MODES (or is off, or unread).
adopt_name() {
	local t n rest
	t=$(vp0_timing "$1")
	[ -n "$t" ] || return 0
	while read -r n rest; do
		if [ "$rest" = "$t" ]; then
			echo "$n"
			return 0
		fi
	done <<<"$ADOPT_MODES"
}
# The mode the hold sets: VP0's own when adopted, else the first.
ADOPT_SET=${ADOPT_MODES%% *}
hold_mode() { local n; n=$(adopt_name "$1"); echo "${n:-$ADOPT_SET}"; }
facts=$(board "$WHO_REMOTE
$VP0_FN"'
set -u
printf "compatible=%s\n" "$(tr "\0" " " </proc/device-tree/compatible)"
printf "kernel=%s\n" "$(uname -r)"
printf "kexec_load_disabled=%s\n" "$(cat /proc/sys/kernel/kexec_load_disabled)"
printf "kexec_loaded=%s\n" "$(cat /sys/kernel/kexec_loaded)"
printf "config_kexec=%s\n" "$(grep -h "^CONFIG_KEXEC=" /boot/config-"$(uname -r)" 2>/dev/null || echo absent)"
printf "config_crash_dump=%s\n" "$(grep -h "^CONFIG_CRASH_DUMP=" /boot/config-"$(uname -r)" 2>/dev/null || echo absent)"
printf "kexec_crash_loaded=%s\n" "$(cat /sys/kernel/kexec_crash_loaded 2>/dev/null || echo absent)"
printf "kexec_crash_size=%s\n" "$(cat /sys/kernel/kexec_crash_size 2>/dev/null || echo absent)"
printf "sysrq_trigger=%s\n" "$(if [ -w /proc/sysrq-trigger ]; then echo writable; else echo absent; fi)"
printf "uid=%s\n" "$(id -u)"
printf "uptime=%s\n" "$(cut -d" " -f1 /proc/uptime)"
printf "system=%s\n" "$(systemctl is-system-running 2>/dev/null || true)"
printf "boot_id=%s\n" "$(cat /proc/sys/kernel/random/boot_id)"
for f in identity state timeout nowayout; do printf "wdt_%s=%s\n" "$f" "$(cat /sys/class/watchdog/watchdog0/$f 2>/dev/null || echo absent)"; done
printf "wdt_users=%s\n" "$(fuser /dev/watchdog /dev/watchdog0 2>/dev/null | tr -s " " || true)"
printf "gpu_users=%s\n" "$(fuser /dev/dri/* 2>/dev/null | tr -s " " || true)"
printf "who_remote=%s\n" "$(who_remote)"
printf "who_local=%s\n" "$(who | awk "\$2 !~ /^(pts|sshd)/ {print \$1\"@\"\$2}" | tr "\n" " ")"
printf "python3=%s\n" "$(command -v python3 || echo absent)"
printf "console=%s\n" "$(tr " " "\n" </proc/cmdline | sed -n "s/^console=\(ttyS[0-9]*\).*/\1/p" | tail -n 1)"
printf "vp0=%s\n" "$(vp0_mode)"
printf "gic_tables=%s\n" "$(dmesg | grep -oE "(LPI (property|pending) table|Devices|Interrupt Collections) @(0x)?[0-9a-f]+" | sed "s/.*@//; s/^0x//" | tr "\n" " ")"
echo "iomem<<"; cat /proc/iomem; echo ">>"') || refuse "cannot read the board's facts over ssh ($BOARD_HOST)"
fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p" | head -1; }

case "$(fact compatible)" in *friendlyarm,nanopc-t6*) ;; *) refuse "$BOARD_HOST is not a NanoPC-T6: compatible '$(fact compatible)'" ;; esac
[ "$(fact uid)" = 0 ] || refuse "$BOARD_HOST is not root (kexec_load needs CAP_SYS_BOOT; /proc/iomem hides addresses)"
[ "$(fact kexec_load_disabled)" = 0 ] || refuse "kexec_load_disabled is $(fact kexec_load_disabled)"
[ "$(fact config_kexec)" = CONFIG_KEXEC=y ] || refuse "the kernel $(fact kernel) has $(fact config_kexec)"
[ "$(fact kexec_loaded)" = 0 ] || refuse "a kexec image is already loaded (/sys/kernel/kexec_loaded 1): someone else is about to jump"

# The window: inside one top-level "System RAM" range, overlapping no entry
# under it (the kernel image, reserved memory). The one entry it may overlap
# is the crashkernel= reservation ("Crash kernel"): the crash route needs it
# to hold the whole window (below), and the normal route may load into it,
# since the kernel limits only crash segments to it (kernel/kexec_core.c:
# 200-210), as long as no crash image is loaded there (below).
iomem=$(printf '%s\n' "$facts" | sed -n '/^iomem<<$/,/^>>$/p' | sed '1d;$d')
grep -qE '^ *[0-9a-f]*[1-9a-f][0-9a-f]*-' <<<"$iomem" || refuse "/proc/iomem shows no addresses (not root?)"
wb=$WINDOW_BASE we=$((WINDOW_BASE + WINDOW_SIZE - 1))
inside=0 parent_s=-1 parent_e=-1 clash= ck_s=-1 ck_e=-1
while IFS= read -r line; do
	[[ "$line" =~ ^(\ *)([0-9a-f]+)-([0-9a-f]+)\ :\ (.*)$ ]] || continue
	ind=${#BASH_REMATCH[1]} s=$((16#${BASH_REMATCH[2]})) e=$((16#${BASH_REMATCH[3]})) name=${BASH_REMATCH[4]}
	if [ "$ind" -eq 0 ]; then
		parent_s=$s parent_e=$e
		if [ "$name" = "System RAM" ] && [ "$s" -le "$wb" ] && [ "$e" -ge "$we" ]; then inside=1; fi
		if [ "$name" != "System RAM" ] && [ "$s" -le "$we" ] && [ "$e" -ge "$wb" ]; then clash="$clash; $line"; fi
	elif [ "$parent_s" -le "$wb" ] && [ "$parent_e" -ge "$we" ] && [ "$s" -le "$we" ] && [ "$e" -ge "$wb" ]; then
		if [ "$name" = "Crash kernel" ]; then
			ck_s=$s ck_e=$e ck_line=${line#"${line%%[! ]*}"}
			continue
		fi
		clash="$clash; ${line#"${line%%[! ]*}"}"
	fi
done <<<"$iomem"
[ "$inside" = 1 ] || refuse "the window $(printf '0x%x-0x%x' "$wb" "$we") is not inside one System RAM range of /proc/iomem"
[ -z "$clash" ] || refuse "the window overlaps:${clash#;}"
# The crash route: the reservation holds the whole window, and in it the
# image's segment (load .. load + memsz) plus the one 4 KiB control page the
# kernel places after it (kexec_core.c:395-450 takes the first free page of
# the reservation above the segment).
ck_lines=$(grep -E '^ *[0-9a-f]+-[0-9a-f]+ : Crash kernel$' <<<"$iomem" | sed 's/^ *//' | tr '\n' ';' | sed 's/;$//' || true)
if [ "$CRASH" = 1 ]; then
	[ "$(fact config_crash_dump)" = CONFIG_CRASH_DUMP=y ] ||
		refuse "--crash: the kernel $(fact kernel) has $(fact config_crash_dump), so it has no crash route"
	if [ "$ck_s" -lt 0 ] || [ "$ck_s" -gt "$wb" ] || [ "$ck_e" -lt "$we" ]; then
		refuse "--crash needs a crashkernel= reservation that holds the window $(printf '0x%08x-0x%08x' "$wb" "$we"); the board has: Crash kernel '${ck_lines:-none}' in /proc/iomem, kexec_crash_size $(fact kexec_crash_size). Make it with t6-crashkernel.sh set and reboot, or run without --crash"
	fi
	[ $((wb + memsz + 4096 - 1)) -le "$ck_e" ] ||
		refuse "--crash: the image's segment ($memsz bytes at $load) and the kernel's 4 KiB control page do not fit in the reservation $(printf '0x%x-0x%x' "$ck_s" "$ck_e")"
	# kexec_zephyr takes every link down before it writes the trigger.
	[ "$(fact sysrq_trigger)" = writable ] ||
		refuse "--crash: /proc/sysrq-trigger is $(fact sysrq_trigger) (not writable): the panic that takes the crash route could not be started"
	[ "$(fact kexec_crash_loaded)" = 0 ] ||
		refuse "--crash: a crash image is already loaded (/sys/kernel/kexec_crash_loaded '$(fact kexec_crash_loaded)': a kdump service, or another run); loading this one would replace it"
	# The mode before the jump: t6-kms-mode.py runs with the board's
	# python3, and the read-back needs VP0's lines of the DRM state.
	[ "$(fact python3)" != absent ] ||
		refuse "--crash: the board has no python3: t6-kms-mode.py, which sets the adopt's mode before the jump, cannot run"
	[ -n "$(fact vp0)" ] ||
		refuse "--crash: no video_port0 in /sys/kernel/debug/dri/0/state (debugfs not mounted, or no rockchip DRM): VP0's mode cannot be read back before the jump"
elif [ "$ck_s" -ge 0 ] && [ "$(fact kexec_crash_loaded)" != 0 ]; then
	# The normal route writes the window at the reboot, over the loaded
	# crash image's segment: a panic before that would enter a damaged image.
	refuse "a crash image is loaded in the crashkernel= reservation the window lies in (/sys/kernel/kexec_crash_loaded '$(fact kexec_crash_loaded)'); the normal route would write over it"
fi

# The GIC's ITS and LPI tables: the redistributors and ITSs Linux enabled keep
# them (EnableLPIs cannot be cleared again) and write the pending tables when an
# LPI arrives, after the jump too. Their addresses are only in Linux's log;
# when it names none (its ring wrapped), they are unknown, and so is whether
# the window is free of them.
gic=$(fact gic_tables)
gic_n=0
for t in $gic; do
	a=$((16#$t)) gic_n=$((gic_n + 1))
	if [ "$a" -le "$we" ] && [ $((a + 0x10000)) -gt "$wb" ]; then
		refuse "a GIC ITS/LPI table at 0x$t (64 KiB) is inside the window (Linux's dmesg)"
	fi
done
[ "$gic_n" -gt 0 ] || refuse "Linux's log names no GIC ITS/LPI table (its ring wrapped?): whether one lies in the window is unknown; reboot the board into Armbian so that dmesg names them again"
echo "board: $BOARD_HOST: $(fact compatible), Linux $(fact kernel), $(fact config_kexec), kexec_load_disabled 0, nothing loaded"
echo "board: up $(fact uptime) s, systemd '$(fact system)', boot_id $(fact boot_id)"
if [ "$CRASH" = 1 ]; then
	echo "board: window $(printf '0x%08x-0x%08x' "$wb" "$we") is inside System RAM and inside the crashkernel= reservation $(printf '0x%08x-0x%08x' "$ck_s" "$ck_e") (kexec_crash_size $(fact kexec_crash_size)), and overlaps no other /proc/iomem entry; no crash image is loaded"
	echo "route: crash (--crash): kexec_load(KEXEC_ON_CRASH), then a panic; no device_shutdown, so the display and the GPU stay as Linux left them"
	if [ -n "$(adopt_name "$(fact vp0)")" ]; then
		echo "board: VP0 runs an adopted mode, $(adopt_name "$(fact vp0)") ($(fact vp0)); --go holds it with t6-kms-mode.py, no size change"
	else
		echo "board: VP0 runs '$(fact vp0)', none of the adopted modes; --go sets $ADOPT_SET first with t6-kms-mode.py ($(fact python3))"
	fi
else
	if [ "$ck_s" -ge 0 ]; then
		echo "board: window $(printf '0x%08x-0x%08x' "$wb" "$we") is inside System RAM; the window lies in the crashkernel= reservation $ck_line, and no crash image is loaded; it overlaps no other /proc/iomem entry"
	else
		echo "board: window $(printf '0x%08x-0x%08x' "$wb" "$we") is inside System RAM and overlaps no /proc/iomem entry"
	fi
	echo "route: normal: kexec_load, then reboot(LINUX_REBOOT_CMD_KEXEC); device_shutdown runs, so the display is off when the image starts (--crash keeps it; crashkernel= reservation: ${ck_lines:-none})"
fi
echo "board: the $gic_n GIC ITS/LPI tables Linux's log names are outside the window ($(printf '%s' "$gic" | tr ' ' ',' | sed 's/,$//'))"
echo "board: WDT0 under Linux: $(fact wdt_identity), state $(fact wdt_state), timeout $(fact wdt_timeout) s, nowayout $(fact wdt_nowayout), open by '$(fact wdt_users)' (the image arms it itself either way)"
if [ "$LINUX_WDT" = 1 ]; then
	[ "$(fact wdt_identity)" != absent ] || refuse "linux-wdt: the board has no /sys/class/watchdog/watchdog0 (--no-linux-wdt jumps without it)"
	[ -z "$(fact wdt_users)" ] || refuse "linux-wdt: /dev/watchdog0 is already open by '$(fact wdt_users)'"
	echo "board: linux-wdt (default): /dev/watchdog0 is opened right before the jump, WDT_CR.WDT_EN is read back, and the jump happens only when it is 1: WDT0 then resets the board about $(fact wdt_timeout) s after the open unless the image re-arms it"
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
if [ -n "$(fact console)" ]; then
	echo "uart: the board's serial console (its kernel command line): /dev/$(fact console); --go has the board write one marker line to it and reads it back from T6_UART before it copies anything"
else
	echo "uart: the board's kernel command line has no console=ttyS entry: --go would refuse (no console to prove the UART path with)"
fi
echo "board: GPU device users '$(fact gpu_users)'; remote sessions '$(fact who_remote)', console logins '$(fact who_local)'"

# ---- 3. the plan ------------------------------------------------------------
open_wdt=
if [ "$LINUX_WDT" = 1 ]; then
	open_wdt="exec 3>/dev/watchdog0 (Linux's dw_wdt starts WDT0 for $(fact wdt_timeout) s), read WDT_CR/WDT_TORR (no jump unless WDT_EN is 1), then "
fi
crash_arg=
if [ "$CRASH" = 1 ]; then
	crash_arg=" --crash"
	how="= kexec_load(entry $load, 1 segment: $file_size bytes at $load, memsz $memsz,
       KEXEC_ON_CRASH: copied into the crashkernel= reservation at once), then
       /proc/sysrq-trigger opened, every device of rk805-pwrkey (the PMIC's
       power key) unbound, CPUs 1-7 offline (PSCI CPU_OFF), two checks: only
       CPU 0 online, every /proc/interrupts row with a handler on a chip the
       crash route was measured with (crash_irq.h; otherwise CPUs back, power
       key bound again, crash image unloaded, no trigger), sysrq enabled,
       sync, every network link but lo down, and a panic by
       /proc/sysrq-trigger 'c': __crash_kexec stops nothing but the CPU, runs
       no device_shutdown (VOP2, HDMI and the GPU keep running) and enters
       the image at EL2 with the MMU off; this ssh session ends there"
	reread="kexec_loaded, kexec_crash_loaded"
	copy_kms=", t6-kms-mode.py"
	mode_step="
     then the adopt's mode: python3 -I $STAGE/t6-kms-mode.py hold --mode $(hold_mode "$(fact vp0)") --max-hold $MODE_HOLD
     holds that mode on HDMI-A-1 as Linux's DRM master (at most $MODE_HOLD s):
     VP0's own mode when it is adopted (no size change), else $ADOPT_SET,
     decided on VP0's mode read again under the lock; VP0's mode is read back
     from /sys/kernel/debug/dri/0/state, and unless it is the mode held, the
     holder is stopped by its pid (Linux's own mode comes back) and there is
     no jump"
else
	copy_kms=
	mode_step=
	how="= kexec_load(entry $load, 1 segment: $file_size bytes at $load, memsz $memsz)
       then reboot(LINUX_REBOOT_CMD_KEXEC): Linux shuts its devices down (PCI bus
       mastering cleared), stops the other CPUs and enters the image at EL2 with
       the MMU off and its VHE HCR_EL2 (E2H set; the image's first code
       rewrites it); this ssh session ends there"
	reread="kexec_loaded"
	[ "$ck_s" -lt 0 ] || reread="kexec_loaded, kexec_crash_loaded"
fi
cat <<EOF
plan (--go):
  1. take the board lock (driver/tools/kexec-zephyr/board_lock.sh, per BOARD_HOST);
     read $reread, the GPU users, the remote sessions and boot_id again under
     it and decide on those values; refuse without T6_HAND, T6_RECORD_DIR and
     T6_UART; prove the UART path: the board writes one marker line to its
     console, and it must be read back from T6_UART within $UART_PROOF_S s
  2. the record: \$T6_RECORD_DIR/<date>-t6/, with the hand at the power switch;
     the UART (\$T6_UART, read only) into it until the board is back, and
     \`dmesg -w\` of the board until the jump
  3. copy kexec_zephyr, mmio_rd$copy_kms and $(basename "$IMG") to $BOARD_HOST:$STAGE/, then
     sync; read GLB_RST_ST, GLB_RST_CON, WDT0 CR and TORR into the record$mode_step
  4. on the board: ${open_wdt}$STAGE/kexec_zephyr $STAGE/zephyr.bin $load$crash_arg
     $how
  5. the image, on UART2 (1500000 baud, into uart.log): the raw early lines
     T6:A (first code, EL2 state), T6:A1 (EL1 init), T6:A2 and T6:A3 (MMU off,
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
proof_pid=
mode_pid=
mode_before=
mode_back=0
cleanup() {
	if [ -n "$dmesg_pid" ]; then kill "$dmesg_pid" 2>/dev/null || true; fi
	if [ -n "$uart_pid" ]; then kill "$uart_pid" 2>/dev/null || true; fi
	if [ -n "$proof_pid" ]; then kill "$proof_pid" 2>/dev/null || true; fi
	mode_stop || true
	rm -rf "$tmp"
}
trap cleanup EXIT
# The holder's stop, on the board: $1 its pid, $2 the boot_id of the Linux
# it runs in, $3 the stage. It kills only that pid, and only while it is
# still the holder in that Linux (its command line names the script and
# "hold"; argv[0] may be python3's full path), then waits up to 5 s for
# it to end (a zombie counts as ended) and prints the holder's own last line
# and VP0's mode.
MODE_STOP=$(cat <<'EOF'
p=$1 boot=$2 stage=$3
if [ "$(cat /proc/sys/kernel/random/boot_id)" != "$boot" ]; then
	echo "another Linux runs (boot_id changed): pid $p ended with the one that ran it"
	exit 0
fi
pstate() { sed 's/.*) //' "/proc/$p/stat" 2>/dev/null | cut -d" " -f1; }
cmd=$(tr "\0" " " <"/proc/$p/cmdline" 2>/dev/null)
case "$cmd" in
*" $stage/t6-kms-mode.py hold "*)
	kill -TERM "$p"
	i=0
	while [ -n "$(pstate)" ] && [ "$(pstate)" != Z ] && [ "$i" -lt 50 ]; do sleep 0.1; i=$((i + 1)); done
	if [ -n "$(pstate)" ] && [ "$(pstate)" != Z ]; then
		echo "pid $p did not end 5 s after SIGTERM"
	else
		echo "pid $p ended after SIGTERM"
	fi
	;;
*) echo "pid $p is not the holder any more (command line '$cmd'): nothing killed" ;;
esac
cat "$stage/kms-mode.log" 2>/dev/null
echo "vp0=$(vp0_mode)"
EOF
)
# mode_stop: stops the holder (once) and records what the board says, and
# whether VP0 runs Linux's own mode again (the mode it ran before the set).
# A read that gave nothing, now or before the set, says neither.
mode_stop() {
	[ -n "$mode_pid" ] || return 0
	local p=$mode_pid out now
	mode_pid=
	if [ "$mode_back" = 1 ]; then
		rec "mode: holder pid $p ended with the Linux that ran it (the board came back with a new boot_id)"
		return 0
	fi
	if ! out=$(board "$VP0_FN
set -- $p $old_boot $STAGE
$MODE_STOP"); then
		rec "mode: could not reach the board to stop holder pid $p; it gives the mode back by itself $MODE_HOLD s after its start" >&2
		return 0
	fi
	rec "mode: stop: $(grep -v '^vp0=' <<<"$out" | tr '\n' ' ')"
	now=$(sed -n 's/^vp0=//p' <<<"$out")
	if [ -z "$now" ] || [ -z "$mode_before" ]; then
		rec "mode: VP0 was not read (now '${now:-unread}', before the set '${mode_before:-unread}'): nothing says that Linux's own mode is back" >&2
	elif [ "$(vp0_timing "$now")" = "$(vp0_timing "$mode_before")" ] &&
		{ [ -n "$(vp0_timing "$now")" ] || [ "$now" = "$mode_before" ]; }; then
		rec "mode: Linux's own mode is back on VP0: $now"
	else
		rec "mode: VP0 does NOT run the mode it ran before the set: now '$now', before '$mode_before'" >&2
	fi
}
HOLD_MAX=${HOLD_MAX:-$((bound_s + 600))}
LOCK_WAIT=${LOCK_WAIT:-0}
# shellcheck source=board_lock.sh
. "$HERE/board_lock.sh"

# Under the lock, the state the jump depends on is read again: the facts
# above were read before the lock, and the wait for it can be LOCK_WAIT long
# when the caller set one.
fresh=$(board "$WHO_REMOTE"'
printf "kexec_loaded=%s\n" "$(cat /sys/kernel/kexec_loaded)"
printf "kexec_crash_loaded=%s\n" "$(cat /sys/kernel/kexec_crash_loaded 2>/dev/null || echo absent)"
printf "boot_id=%s\n" "$(cat /proc/sys/kernel/random/boot_id)"
printf "gpu_users=%s\n" "$(fuser /dev/dri/* 2>/dev/null | tr -s " " || true)"
printf "wdt_users=%s\n" "$(fuser /dev/watchdog /dev/watchdog0 2>/dev/null | tr -s " " || true)"
printf "who_remote=%s\n" "$(who_remote)"') ||
	refuse "cannot read the board's state again under the lock"
fresh_fact() { printf '%s\n' "$fresh" | sed -n "s/^$1=//p" | head -1; }
[ "$(fresh_fact kexec_loaded)" = 0 ] || refuse "under the lock: a kexec image is already loaded (/sys/kernel/kexec_loaded '$(fresh_fact kexec_loaded)')"
if { [ "$CRASH" = 1 ] || [ "$ck_s" -ge 0 ]; } && [ "$(fresh_fact kexec_crash_loaded)" != 0 ]; then
	refuse "under the lock: a crash image is already loaded (/sys/kernel/kexec_crash_loaded '$(fresh_fact kexec_crash_loaded)')"
fi
[ -z "$(fresh_fact gpu_users)" ] || refuse "under the lock: processes hold the GPU device: $(fresh_fact gpu_users)"
if [ -n "$(fresh_fact who_remote)" ] && [ "${T6_ALLOW_SESSIONS:-0}" != 1 ]; then
	refuse "under the lock: remote sessions on the board: $(fresh_fact who_remote)-- the jump ends them; ask whoever holds them, then T6_ALLOW_SESSIONS=1"
fi
if [ "$LINUX_WDT" = 1 ] && [ -n "$(fresh_fact wdt_users)" ]; then
	refuse "under the lock: linux-wdt: /dev/watchdog0 is open by '$(fresh_fact wdt_users)'"
fi
old_boot=$(fresh_fact boot_id)
[ -n "$old_boot" ] || refuse "under the lock: the board gave no boot_id"

# The UART path, proven before anything is copied or loaded: the run is
# judged from uart.log, so a wire that delivers nothing spends the jump for
# no verdict (2026-11-22: uart.log 0 bytes; a line the board wrote to its
# own console moved its tx counter, and the host read nothing). A temporary
# capture reads T6_UART the way the real one does (cat, read only); the
# board writes one marker line with this run's nonce to its serial console
# (the last console=ttyS entry of its kernel command line, from the facts);
# the capture must read it back within UART_PROOF_S seconds (above). The
# host never writes to T6_UART. The capture is stopped by its pid and
# waited for before anything else, so the real capture below is the only
# reader. Its bytes, the marker line among them, are not part of uart.log.
console=$(fact console)
[[ "$console" =~ ^ttyS[0-9]+$ ]] ||
	refuse "the board's kernel command line has no console=ttyS entry ('$console'): there is no serial console to write the UART proof's marker line to, so the path to $T6_UART cannot be proven"
now_ms() { echo $(($(date +%s%N) / 1000000)); }
secs() { printf '%d.%d' $(($1 / 1000)) $(($1 % 1000 / 100)); }
proof_log=$tmp/uart-proof.log
uart_dev=$(readlink -f "$T6_UART")
cat <"$T6_UART" >"$proof_log" &
proof_pid=$!
t_open=$(now_ms)
# The board's line is lost unless the tty is open when it arrives: wait
# until the capture holds it as its input.
opened=0
for _ in $(seq 50); do
	if [ "$(readlink "/proc/$proof_pid/fd/0" 2>/dev/null)" = "$uart_dev" ]; then opened=1; break; fi
	kill -0 "$proof_pid" 2>/dev/null || break
	sleep 0.1
done
proof_stop() {
	kill "$proof_pid" 2>/dev/null || true
	wait "$proof_pid" 2>/dev/null || true
	proof_pid=
}
if [ "$opened" = 0 ]; then
	proof_stop
	refuse "the UART proof's capture (cat <$T6_UART) did not open $T6_UART within 5 s; nothing was written to the board's console"
fi
marker="yakogl-uart-proof $(od -A n -N 8 -t x1 /dev/urandom | tr -d ' \n')"
t_write=$(now_ms)
write_rc=0
board "printf '\\r\\n%s\\r\\n' '$marker' >/dev/$console" >/dev/null 2>&1 || write_rc=$?
if [ "$write_rc" = 0 ]; then
	written="the board wrote one marker line to its console /dev/$console (ssh rc 0)"
else
	written="the ssh command that writes one marker line to the board's console /dev/$console failed (rc $write_rc), so the line may not have been written"
fi
got_ms=
deadline_ms=$(($(now_ms) + UART_PROOF_S * 1000))
while :; do
	if grep -qaF "$marker" "$proof_log"; then got_ms=$(($(now_ms) - t_write)); break; fi
	[ "$(now_ms)" -lt "$deadline_ms" ] || break
	kill -0 "$proof_pid" 2>/dev/null || break
	sleep 0.1
done
proof_stop
# A capture that ended by itself may have written the line after the last look.
if [ -z "$got_ms" ] && grep -qaF "$marker" "$proof_log"; then got_ms=$(($(now_ms) - t_write)); fi
open_ms=$(($(now_ms) - t_open))
proof_bytes=$(wc -c <"$proof_log")
if [ -z "$got_ms" ]; then
	if [ "$proof_bytes" = 0 ]; then
		refuse "UART path not proven: the capture of $T6_UART read 0 bytes in $(secs "$open_ms") s, and $written. Check first the wire between the board's debug header (UART2: its TX to the adapter's RX, and GND) and the host's adapter"
	fi
	refuse "UART path not proven: the capture of $T6_UART read $proof_bytes bytes in $(secs "$open_ms") s, but not the marker line, and $written. The bytes are not that line: check that T6_UART is this board's debug UART (not another board's console) and that it runs at 1500000 baud"
fi

REC=$T6_RECORD_DIR/$(date +%Y%m%d-%H%M%S)-t6
mkdir "$REC" || die "cannot make the record directory $REC"
rec() { printf '%s %s\n' "$(date '+%F %T')" "$*" | tee -a "$REC/record.txt"; }
echo "nanopc-t6.sh: record $REC"
rec "build $(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)"
rec "board $BOARD_HOST, boot_id $old_boot; hand at the power switch: $T6_HAND; linux_wdt $LINUX_WDT; route $([ "$CRASH" = 1 ] && echo "crash (reservation $(printf '0x%x-0x%x' "$ck_s" "$ck_e"))" || echo normal)"
rec "image $IMG, sha256 $(sha256sum "$IMG" | cut -d' ' -f1): $markers"
rec "bound: back in reset by $bound_s s after the jump at the latest"
rec "uart: proven before anything was copied: the marker line '$marker' the board wrote to /dev/$console was read back from $T6_UART $got_ms ms after the write began; that capture read $proof_bytes bytes in $(secs "$open_ms") s (not part of uart.log)"

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
# CRU's CLKSEL_CON(158), (159) and CLKGATE_CON(66), the CRU's MODE_CON0 and
# PLL_CON1 and PLL_CON6 of the four PLLs the GPU clock can run from (AUPLL,
# CPLL, GPLL, NPLL: CON1 bit 13 set is powered down, CON6 bit 15 is the
# lock; Linux powers down a PLL nothing uses), and the RK806's POWER_EN0,
# POWER_SLP_EN0, BUCK1_CONFIG and BUCK1_ON_VSEL from the regmap cache Linux
# keeps (debugfs). All always-on blocks: nothing behind PD_GPU is read.
read_gpu_power() {
	board "printf 'runtime %s; ' \$(cat /sys/bus/platform/devices/fb000000.gpu/power/runtime_status)
for a in 0xfd8d814c 0xfd8d810c 0xfd8d8118 0xfd8d8120 0xfd8d8290 0xfd7c0578 0xfd7c057c 0xfd7c0908; do $STAGE/mmio_rd \$a; done | tr '\n' ' '
printf 'pll '; for a in 0xfd7c0280 0xfd7c0184 0xfd7c0198 0xfd7c01a4 0xfd7c01b8 0xfd7c01c4 0xfd7c01d8 0xfd7c01e4 0xfd7c01f8; do $STAGE/mmio_rd \$a; done | tr '\n' ' '
printf 'rk806 '; grep -E '^(00|06|10|1a):' /sys/kernel/debug/regmap/spi2.0/registers | tr '\n' ' '"
}
board "rm -rf $STAGE && mkdir -p $STAGE"
bscp "${SSH_OPTS[@]}" "$LOADER" "$BOARD_HOST:$STAGE/kexec_zephyr"
stage_mmio
bscp "${SSH_OPTS[@]}" "$IMG" "$BOARD_HOST:$STAGE/zephyr.bin"
staged=("$IMG" "$LOADER")
staged_on_board="$STAGE/zephyr.bin $STAGE/kexec_zephyr"
if [ "$CRASH" = 1 ]; then
	bscp "${SSH_OPTS[@]}" "$HERE/t6-kms-mode.py" "$BOARD_HOST:$STAGE/t6-kms-mode.py"
	staged+=("$HERE/t6-kms-mode.py")
	staged_on_board+=" $STAGE/t6-kms-mode.py"
fi
want=$(sha256sum "${staged[@]}" | cut -d' ' -f1 | tr '\n' ' ')
have=$(board "sha256sum $staged_on_board | cut -d' ' -f1 | tr '\n' ' '; sync")
[ "$want" = "$have" ] || { rec "the copies on the board differ (want $want, have $have); not jumping" >&2; exit 1; }
regs=$(read_regs) || { rec "cannot read the registers with mmio_rd; not jumping" >&2; exit 1; }
rec "registers before (GLB_RST_ST, GLB_RST_CON, WDT0 CR, TORR): $regs"
rec "gpu power before: $(read_gpu_power || echo unread)"

# The adopt's mode (the crash route only; driver/docs/nanopc-t6-display.md,
# section 2(a), "The mode before the jump"). t6-kms-mode.py holds VP0's own
# mode when it is one of ADOPT_MODES, else it sets ADOPT_SET, and leaves a
# holder; its line names the holder's pid. Then VP0's mode is read back
# from Linux, and the jump goes on only when it is the mode held. Every
# path from here that does not jump stops the holder by that pid
# (mode_refuse, finish, and the EXIT trap behind them).
mode_refuse() {
	rec "mode: REFUSED: $*; not jumping" >&2
	mode_stop
	exit 1
}
if [ "$CRASH" = 1 ]; then
	mode_before=$(board "$VP0_FN
vp0_mode") || mode_before=
	rec "mode: VP0 before: ${mode_before:-unread}"
	held=$(hold_mode "$mode_before")
	if [ -n "$(adopt_name "$mode_before")" ]; then
		rec "mode: VP0 runs an adopted mode: hold $held, no size change"
	else
		rec "mode: VP0 runs none of the adopted modes: set $held"
	fi
	mode_rc=0
	mode_out=$(board "python3 -I $STAGE/t6-kms-mode.py hold --mode $held --max-hold $MODE_HOLD --log $STAGE/kms-mode.log" 2>&1) || mode_rc=$?
	rec "mode: $(tr '\n' ' ' <<<"$mode_out")(rc $mode_rc)"
	mode_re='; held by pid ([0-9]+) for at most [0-9]+ s$'
	if [[ "$mode_out" =~ $mode_re ]]; then mode_pid=${BASH_REMATCH[1]}; fi
	if [ "$mode_rc" != 0 ] || [ -z "$mode_pid" ]; then
		# Exit 3 (refused) and 1 (given back) leave no holder. Anything
		# else without a pid (ssh cut, a timeout) may have left one that
		# cannot be named: it ends by itself.
		case "$mode_pid:$mode_rc" in
		:1 | :3) ;;
		:*) rec "mode: no holder pid in the answer: if a holder runs, it gives the mode back by itself $MODE_HOLD s after its start" >&2 ;;
		esac
		mode_refuse "t6-kms-mode.py did not set and hold the adopt's mode (rc $mode_rc)"
	fi
	vp0=$(board "$VP0_FN
vp0_mode") || vp0=
	[ "$(adopt_name "$vp0")" = "$held" ] ||
		mode_refuse "VP0 does not run the mode held ($(grep "^$held " <<<"$ADOPT_MODES")) after the set: '${vp0:-unread}'"
	rec "mode: VP0 reads back '$vp0': the adopt's mode $held, held by pid $mode_pid across the jump"
	# For the record only: the adopt also compares VP0's DSP_CTRL with the
	# build's HDMI RGB value, which an output in another depth or format
	# changes (driver/docs/nanopc-t6-display.md, section 8, U15).
	rec "mode: HDMI-A-1 output (DRM state): $(board "$VP0_FN
hdmi_out" || true)"
fi

# What the jump command does when kexec_zephyr returns. On the normal route
# the board is as it was: the magic close stops WDT0 from resetting it
# (Linux feeds it again). On the crash route kexec_zephyr may have taken
# every link down before it returned, so the board may be unreachable: no
# magic close, and a process outside the ssh session (setsid) keeps WDT0
# open and unfed, so WDT0 resets the board. The open is held rather than
# left to the close: after a close without 'V' WDT0 stays active, and
# whether Linux then feeds it depends on the driver's maximum hardware
# heartbeat (linux v6.18 drivers/watchdog/watchdog_dev.c:95-96, :960-972).
if [ "$CRASH" = 1 ]; then
	return_wdt="setsid sleep 600 </dev/null >/dev/null 2>&1 &
echo \"linux-wdt: kexec_zephyr returned \$rc on the crash route: no magic close; WDT0 stays open, unfed, by pid \$! and resets the board\""
else
	return_wdt="printf V >&3; exec 3>&-
echo \"linux-wdt: kexec_zephyr returned \$rc: magic close written, Linux feeds WDT0 from now on\""
fi
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
$STAGE/kexec_zephyr $STAGE/zephyr.bin $load$crash_arg; rc=\$?
$return_wdt
exit \$rc"
else
	jump="sync; $STAGE/kexec_zephyr $STAGE/zephyr.bin $load$crash_arg"
fi
t0=$(date +%s)
rec "jumping (linux_wdt $LINUX_WDT, route $([ "$CRASH" = 1 ] && echo crash || echo normal))"
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
	mode_stop
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
		mode_back=1
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
