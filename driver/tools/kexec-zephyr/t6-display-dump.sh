#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-display-dump.sh - read the NanoPC-T6's display registers while Armbian
# shows a picture on the first HDMI output (driver/docs/nanopc-t6-display.md,
# section 6, step D0). The result is the reference the Zephyr display
# bring-up is compared with.
#
#   driver/tools/kexec-zephyr/t6-display-dump.sh --plan                    # the read list, no board
#   BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/t6-display-dump.sh      # dry run
#   BOARD_HOST=root@<t6> T6_HAND=<name> T6_RECORD_DIR=<dir> \
#     driver/tools/kexec-zephyr/t6-display-dump.sh --go                    # the reads
#
# It only reads: registers with mmio_rd (opened read only) and the GRFs
# through their regmap files in debugfs. Without --go it checks the read
# list, reads the board's display state from sysfs and debugfs, prints what
# --go would do and reads no register. With --go it takes the board lock
# (board_lock.sh; it refuses, never waits, while anyone holds it), reads the
# state again, copies mmio_rd to /tmp/yakogl-display-dump on the board, makes
# the reads one list line at a time and removes what it copied.
#
# The display must be on, and stay on: a read of a block whose power domain
# or bus clock is off may hang the SoC's bus, and then only the power switch
# helps (so --go asks for T6_HAND). "On" is checked on the board before the
# first read and again in the same command as every read: the connector of
# fde80000.hdmi is connected and enabled, the power domains vop and vo1 are
# on, and the bus clocks hclk_vop, aclk_vop, pclk_hdmitx0 and pclk_hdptx0
# have users. If one of them does not hold, nothing more is read.
#
# The read list is checked before anything else, against the windows below:
# a line that reads a word outside them is refused. The windows leave out
# what must not be read (nanopc-t6-display.md, "Must NOT be read"): the HDMI
# controller's request words 0x40, 0x974 and 0xab0 and its I2C master
# 0xe0-0x118, the VOP's LUT RAM from 0x2000, the PHY outside the ranges its
# Linux driver reads, and every regmap file but the GRFs' (the controller's
# has no readable list and would read all of it). In the HDMI controller a
# window is a run of registers that answered on the board (2026-11-12), not
# an address range: the read of a register this controller does not have
# ends in a bus error. That is 0x10, which Linux's dw-hdmi-qp.h (v6.18) does
# not define, and also 0x804 and 0x80c, which it does define. So a new
# offset is added only after it was read alone on the board.
#
# The record: one directory per --go run under $T6_RECORD_DIR. record.txt
# (this script's lines), dump.txt (the registers as "ADDRESS WIDTH VALUE",
# with a header; the file that goes into the tree as
# driver/docs/nanopc-t6-display-dump-linux.txt) and files.txt (the mode, the
# planes, Linux's own VOP2 and PHY dumps, the clocks and the domains as
# Linux prints them; it stays in the record).
#
# Exit: 0 done; 2 usage; 3 refused, nothing was copied and no register was
# read; 4 a read did not return what was asked: stopped there, the record
# has the reads before it.
#
# Environment:
#   BOARD_HOST      root@<the NanoPC-T6> (not needed for --plan)
#   BOARD_SSH_OPTS  more ssh options, as words
#   T6_HAND         --go: the name of the person at the T6's power switch
#   T6_RECORD_DIR   --go: an existing directory outside the checkout
#   T6_DUMP_LIST    a file with another read list (the same check applies),
#                   to read a part again
#   MMIO_RD         a built mmio_rd for the board (default: built here with
#                   CROSS_GCC, default aarch64-linux-gnu-gcc)
#   BOARD_LOCK, LOCK_WAIT, HOLD_MAX  see board_lock.sh
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
STAGE=/tmp/yakogl-display-dump

die() { echo "t6-display-dump.sh: $*" >&2; exit 2; }
refuse() {
	echo "t6-display-dump.sh: REFUSED: $*" >&2
	echo "t6-display-dump.sh: nothing was copied and no register was read." >&2
	exit 3
}

GO=0 PLAN=0
for a in "$@"; do
	case "$a" in
	--go) GO=1 ;;
	--plan) PLAN=1 ;;
	*) die "unknown argument $a (usage: t6-display-dump.sh [--plan | --go])" ;;
	esac
done

# ---- 1. the read list and its check -----------------------------------------
# mem <block> <address> <words>: 32-bit words with mmio_rd.
# regmap <block> <base> <offset> <words>: words of the regmap file of the GRF
# at <base>. Offsets are those of Linux v6.18's headers and of
# nanopc-t6-display.md, section 1. The order is the order of the reads: what
# every boot leaves on first, the PMU1 CRU last.
builtin_list() {
	cat <<'EOF'
# PMU: idle request, ack and state; power gate; status; memory power; chain
# and memory status; repair status (pm-domains.c, the rk3588 offsets)
mem pmu 0xfd8d810c 7
mem pmu 0xfd8d814c 2
mem pmu 0xfd8d8180 2
mem pmu 0xfd8d81a0 2
mem pmu 0xfd8d81f0 4
mem pmu 0xfd8d8290 2
# CRU: V0PLL, CPLL, GPLL (PLL_CON 0-6 of each) and MODE_CON0
mem cru 0xfd7c0160 7
mem cru 0xfd7c01a0 7
mem cru 0xfd7c01c0 7
mem cru 0xfd7c0280 1
# CRU: CLKSEL_CON 110-115 and 128-136
mem cru 0xfd7c04b8 6
mem cru 0xfd7c0500 9
# CRU: CLKGATE_CON 1, 52-53, 55, 59-61, 72-74
mem cru 0xfd7c0804 1
mem cru 0xfd7c08d0 2
mem cru 0xfd7c08dc 1
mem cru 0xfd7c08ec 3
mem cru 0xfd7c0920 3
# CRU: SOFTRST_CON 52-53, 60-61, 72-73
mem cru 0xfd7c0ad0 2
mem cru 0xfd7c0af0 2
mem cru 0xfd7c0b20 2
# sys_grf SOC_CON2, SOC_CON7, SOC_STATUS1; vop_grf GRF_VOP_CON2; vo1_grf
# CON0-CON15; hdptxphy0_grf CON0 and STATUS
regmap sys_grf 0xfd58c000 0x308 1
regmap sys_grf 0xfd58c000 0x31c 1
regmap sys_grf 0xfd58c000 0x384 1
regmap vop_grf 0xfd5a4000 0x008 1
regmap vo1_grf 0xfd5a8000 0x000 16
regmap phy0_grf 0xfd5e0000 0x000 1
regmap phy0_grf 0xfd5e0000 0x080 1
# VOP2: SYS, OVL, video ports 0 and 1, DSP_CTRL of video ports 2 and 3
mem vop 0xfdd90000 64
mem vop 0xfdd90600 64
mem vop 0xfdd90c00 64
mem vop 0xfdd90d00 64
mem vop 0xfdd90e00 1
mem vop 0xfdd90f00 1
# VOP2 windows: Cluster0 and Cluster1 (with CLUSTER_CTRL at +0x100), the
# enable words of Cluster2 and Cluster3, Esmart0 and Esmart1, the enable
# words of Esmart2 and Esmart3
mem vop 0xfdd91000 65
mem vop 0xfdd91200 65
mem vop 0xfdd91400 1
mem vop 0xfdd91500 1
mem vop 0xfdd91600 1
mem vop 0xfdd91700 1
mem vop 0xfdd91800 60
mem vop 0xfdd91a00 60
mem vop 0xfdd91c10 1
mem vop 0xfdd91e10 1
# vop_mmu, both sets: DTE_ADDR, STATUS, COMMAND
mem vop_mmu 0xfdd97e00 3
mem vop_mmu 0xfdd97f00 3
# HDMI QP TX0: ids and time stamp, software disable and reset manager, timer
# base, CMU, video interface and monitor, link, HDCP2 bypass, packet
# scheduler, AVI contents, interrupt status and mask. Every word is a
# register dw-hdmi-qp.h defines and that answered on the board (0x804 and
# 0x80c did not)
mem hdmi0 0xfde80000 4
mem hdmi0 0xfde80014 3
mem hdmi0 0xfde80044 2
mem hdmi0 0xfde80050 3
mem hdmi0 0xfde80080 2
mem hdmi0 0xfde800a0 10
mem hdmi0 0xfde80180 1
mem hdmi0 0xfde80800 1
mem hdmi0 0xfde80808 1
mem hdmi0 0xfde80814 1
mem hdmi0 0xfde8081c 1
mem hdmi0 0xfde80880 8
mem hdmi0 0xfde808e0 1
mem hdmi0 0xfde80960 1
mem hdmi0 0xfde80968 1
mem hdmi0 0xfde80970 1
mem hdmi0 0xfde80a80 11
mem hdmi0 0xfde80ab4 2
mem hdmi0 0xfde80be0 8
mem hdmi0 0xfde83010 2
mem hdmi0 0xfde83020 2
# HDPTX PHY0: the ranges its Linux driver reads (CMN, SB, the lane top, the
# four lanes)
mem phy0 0xfed60000 168
mem phy0 0xfed60400 42
mem phy0 0xfed60800 42
mem phy0 0xfed60c00 46
mem phy0 0xfed61000 46
mem phy0 0xfed61400 46
mem phy0 0xfed61800 46
# PMU1 CRU: the HDPTX reference clock's select and gate, its resets
mem pmu1cru 0xfd7f0338 1
mem pmu1cru 0xfd7f0810 1
mem pmu1cru 0xfd7f0a0c 2
EOF
}

# What a mem line may read: <block> <first byte> <last byte>. A read must lie
# inside one window of its own block.
WINDOWS='
pmu 0xfd8d8000 0xfd8d8fff
cru 0xfd7c0000 0xfd7c0fff
pmu1cru 0xfd7f0000 0xfd7f0fff
vop 0xfdd90000 0xfdd91fff
vop_mmu 0xfdd97e00 0xfdd97e0b
vop_mmu 0xfdd97f00 0xfdd97f0b
hdmi0 0xfde80000 0xfde8000f
hdmi0 0xfde80014 0xfde8001f
hdmi0 0xfde80044 0xfde8004b
hdmi0 0xfde80050 0xfde8005b
hdmi0 0xfde80080 0xfde80087
hdmi0 0xfde800a0 0xfde800c7
hdmi0 0xfde80180 0xfde80183
hdmi0 0xfde80800 0xfde80803
hdmi0 0xfde80808 0xfde8080b
hdmi0 0xfde80814 0xfde80817
hdmi0 0xfde8081c 0xfde8081f
hdmi0 0xfde80880 0xfde8089f
hdmi0 0xfde808e0 0xfde808e3
hdmi0 0xfde80960 0xfde80963
hdmi0 0xfde80968 0xfde8096b
hdmi0 0xfde80970 0xfde80973
hdmi0 0xfde80a80 0xfde80aaf
hdmi0 0xfde80ab4 0xfde80abb
hdmi0 0xfde80be0 0xfde80bff
hdmi0 0xfde83010 0xfde83017
hdmi0 0xfde83020 0xfde83027
phy0 0xfed60000 0xfed6029f
phy0 0xfed60400 0xfed604a7
phy0 0xfed60800 0xfed608a7
phy0 0xfed60c00 0xfed60cb7
phy0 0xfed61000 0xfed610b7
phy0 0xfed61400 0xfed614b7
phy0 0xfed61800 0xfed618b7
'
# The regmap files a regmap line may read: <block> <base>, 4 KiB each.
REGMAPS='
sys_grf 0xfd58c000
vop_grf 0xfd5a4000
vo1_grf 0xfd5a8000
phy0_grf 0xfd5e0000
'

is_hex() { [[ "$1" =~ ^0x[0-9a-f]{1,8}$ ]]; }
is_count() { [[ "$1" =~ ^[1-9][0-9]{0,2}$ ]]; }

# list_check <file>: prints the list without comments, or says which line is
# not allowed and returns 1.
list_check() {
	local kind block a b c rest lo hi w wb wlo whi ok n=0 words=0
	while read -r kind block a b c rest; do
		case "$kind" in '' | '#'*) continue ;; esac
		n=$((n + 1)) ok=0
		case "$kind" in
		mem)
			if [ -z "$c$rest" ] && is_hex "$a" && is_count "$b" && [ $((a % 4)) = 0 ]; then
				lo=$((a)) hi=$((a + 4 * b - 1))
				while read -r wb wlo whi; do
					[ "$wb" = "$block" ] && [ "$lo" -ge $((wlo)) ] && [ "$hi" -le $((whi)) ] && ok=1
				done <<<"$WINDOWS"
			fi
			w=$b
			;;
		regmap)
			if [ -z "$rest" ] && is_hex "$a" && is_hex "$b" && is_count "$c" && [ $((b % 4)) = 0 ] &&
				[ $((b + 4 * c)) -le $((0x1000)) ]; then
				while read -r wb wlo; do
					[ "$wb" = "$block" ] && [ "$((a))" = "$((wlo))" ] && ok=1
				done <<<"$REGMAPS"
			fi
			w=$c
			;;
		esac
		if [ "$ok" != 1 ]; then
			echo "not in the windows of the read list: '$kind $block $a $b${c:+ $c}${rest:+ $rest}'"
			return 1
		fi
		words=$((words + w))
		echo "$kind $block $a $b${c:+ $c}"
	done <"$1"
	[ "$n" -gt 0 ] || { echo "the read list is empty"; return 1; }
	echo "# $n lines, $words words"
}

tmp=$(mktemp -d)
cleanup() { rm -rf "$tmp"; }
trap cleanup EXIT
if [ -n "${T6_DUMP_LIST:-}" ]; then
	[ -r "$T6_DUMP_LIST" ] || die "cannot read T6_DUMP_LIST=$T6_DUMP_LIST"
	cp "$T6_DUMP_LIST" "$tmp/list.in"
else
	builtin_list >"$tmp/list.in"
fi
# A refused list has printed its lines up to the one that is not allowed; the
# reason is the last line.
LIST=$(list_check "$tmp/list.in") || refuse "$(tail -n 1 <<<"$LIST")"
BUILD=$(git -C "$HERE" describe --always --dirty 2>/dev/null || echo unknown)
echo "t6-display-dump.sh: build $BUILD"
if [ "$PLAN" = 1 ]; then
	printf '%s\n' "$LIST"
	exit 0
fi

# ---- 2. the board's display state, read only ---------------------------------
: "${BOARD_HOST:?set BOARD_HOST=root@<the NanoPC-T6>}"
# shellcheck disable=SC2206  # options are words
SSH_OPTS=(${BOARD_SSH_OPTS:-})
board() { timeout "${BOARD_TIMEOUT:-30}" ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" "$@" </dev/null; }

# display_off, defined on the board ahead of the commands that call it: one
# line for each thing that is not as a read needs it; nothing when the
# display is on.
DISPLAY_OFF=$(cat <<'EOF'
display_off() {
	conn=
	for c in /sys/class/drm/card*-HDMI-A-*; do
		case "$(readlink "$c/ddc" 2>/dev/null)" in */fde80000.hdmi/*) conn=$c ;; esac
	done
	if [ -z "$conn" ]; then
		echo "no connector of fde80000.hdmi"
	else
		[ "$(cat "$conn/status")" = connected ] || echo "${conn##*/} is not connected"
		[ "$(cat "$conn/enabled")" = enabled ] || echo "${conn##*/} is not enabled"
	fi
	for d in vop vo1; do
		[ "$(awk -v d="$d" '$1 == d { print $2; exit }' /sys/kernel/debug/pm_genpd/pm_genpd_summary 2>/dev/null)" = on ] ||
			echo "power domain $d is not on"
	done
	for k in hclk_vop aclk_vop pclk_hdmitx0 pclk_hdptx0; do
		n=$(cat "/sys/kernel/debug/clk/$k/clk_enable_count" 2>/dev/null)
		case "$n" in '' | 0 | *[!0-9]*) echo "clock $k has no user" ;; esac
	done
}
EOF
)
STATE_CMD=$(cat <<'EOF'
set -u
printf "compatible=%s\n" "$(tr "\0" " " </proc/device-tree/compatible)"
printf "uid=%s\n" "$(id -u)"
printf "uname=%s\n" "$(uname -srvm)"
for c in /sys/class/drm/card*-HDMI-A-*; do
	case "$(readlink "$c/ddc" 2>/dev/null)" in */fde80000.hdmi/*)
		printf "connector=%s\n" "${c##*/}"
		printf "edid_bytes=%s\n" "$(wc -c <"$c/edid")"
		;;
	esac
done
printf "mode=%s\n" "$(awk '/^crtc\[/ { a = 0 } $1 == "active=1" { a = 1 } a && $1 == "mode:" { $1 = ""; print; exit }' /sys/kernel/debug/dri/0/state 2>/dev/null)"
echo "off<<"; display_off; echo ">>"
EOF
)
state=$(board "$DISPLAY_OFF
$STATE_CMD") || refuse "cannot read the board's state over ssh ($BOARD_HOST)"
fact() { printf '%s\n' "$1" | sed -n "s/^$2=//p" | head -1; }
off_lines() { printf '%s\n' "$1" | sed -n '/^off<<$/,/^>>$/p' | sed '1d;$d'; }
state_check() { # <state> <prefix of the refusal>
	local off
	case "$(fact "$1" compatible)" in
	*friendlyarm,nanopc-t6*) ;;
	*) refuse "$2$BOARD_HOST is not a NanoPC-T6: compatible '$(fact "$1" compatible)'" ;;
	esac
	[ "$(fact "$1" uid)" = 0 ] || refuse "$2the ssh login is not root (uid $(fact "$1" uid)): debugfs and /dev/mem need it"
	off=$(off_lines "$1")
	[ -z "$off" ] || refuse "$2the display is not on: ${off//$'\n'/; } -- plug the display into the first HDMI output, switch it on, wake the console"
}
state_check "$state" ""
echo "board: $(fact "$state" uname)"
echo "board: $(fact "$state" connector) (fde80000.hdmi) connected and enabled; EDID $(fact "$state" edid_bytes) bytes; mode$(fact "$state" mode)"
echo "board: power domains vop and vo1 on; hclk_vop, aclk_vop, pclk_hdmitx0, pclk_hdptx0 have users"
cat <<EOF
plan (--go):
  1. take the board lock; read the display state again
  2. copy mmio_rd to $STAGE on the board
  3. make the reads, one line of the list per ssh command, each after the
     display state check; stop at the first that does not return its words:
     ${LIST##*# }
  4. copy Linux's own view (mode, planes, VOP2 and PHY dumps, clocks,
     domains) into files.txt; remove $STAGE
EOF
if [ "$GO" != 1 ]; then
	echo "dry run: nothing was copied and no register was read (add --go to read)"
	exit 0
fi

# ---- 3. --go ------------------------------------------------------------------
[ -n "${T6_HAND:-}" ] || refuse "--go needs T6_HAND=<the name of the person at the T6's power switch for this run>"
{ [ -n "${T6_RECORD_DIR:-}" ] && [ -d "$T6_RECORD_DIR" ]; } ||
	refuse "--go needs T6_RECORD_DIR=<an existing directory outside the checkout> for the run's record"
MMIO=${MMIO_RD:-}
if [ -z "$MMIO" ]; then
	MMIO=$tmp/mmio_rd
	"${CROSS_GCC:-aarch64-linux-gnu-gcc}" -O2 -static -nostdlib -ffreestanding -fno-stack-protector \
		-o "$MMIO" "$HERE/mmio_rd.c" || die "cannot build mmio_rd"
fi
[ -r "$MMIO" ] || die "cannot read mmio_rd ($MMIO)"

# Before the lock: board_lock.sh chains the EXIT trap it finds.
staged=0
cleanup() {
	[ "$staged" = 0 ] || board "rm -f $STAGE/mmio_rd; rmdir $STAGE" || echo "t6-display-dump.sh: $STAGE is left on the board" >&2
	rm -rf "$tmp"
}
HOLD_MAX=${HOLD_MAX:-900}
LOCK_WAIT=${LOCK_WAIT:-0}
# shellcheck source=board_lock.sh
. "$HERE/board_lock.sh"

fresh=$(board "$DISPLAY_OFF
$STATE_CMD") || refuse "cannot read the board's state again under the lock"
state_check "$fresh" "under the lock: "

REC=$T6_RECORD_DIR/$(date +%Y%m%d-%H%M%S)-t6-display-dump
mkdir "$REC" || die "cannot make the record directory $REC"
rec() { printf '%s %s\n' "$(date '+%F %T')" "$*" | tee -a "$REC/record.txt"; }
rec "record $REC"
rec "build $BUILD; hand $T6_HAND; list ${T6_DUMP_LIST:-built in}, ${LIST##*# }"
rec "board $(fact "$fresh" uname)"

staged=1
{ board "mkdir -p $STAGE" && bscp "${SSH_OPTS[@]}" "$MMIO" "$BOARD_HOST:$STAGE/mmio_rd"; } ||
	{ rec "STOPPED: cannot copy mmio_rd to the board"; exit 4; }

# The reads. Every command checks the display state first, on the board, and
# reads nothing when it is off.
GUARD="$DISPLAY_OFF
off=\$(display_off); [ -z \"\$off\" ] || { echo \"display off: \$off\"; exit 9; }"
stop() { rec "STOPPED at '$1': $2"; rec "the reads before it are in $REC/dump.body"; exit 4; }
: >"$REC/dump.body"
while read -r kind block a b c; do
	case "$kind" in '#'*) continue ;; esac
	line="$kind $block $a $b${c:+ $c}"
	case "$kind" in
	mem)
		base=0 first=$((a)) words=$b
		out=$(board "$GUARD
$STAGE/mmio_rd $a $b") || stop "$line" "the command failed (${out:-no output})"
		;;
	regmap)
		base=$((a)) first=$((b)) words=$c
		out=$(board "$GUARD
set -- /sys/kernel/debug/regmap/*${a#0x}*/registers
[ \$# = 1 ] && [ -r \"\$1\" ] || { echo \"no regmap file of $a\"; exit 8; }
n=\$(head -1 \"\$1\" | wc -c)
dd if=\"\$1\" bs=\"\$n\" skip=$((b / 4)) count=$c 2>/dev/null") || stop "$line" "the command failed (${out:-no output})"
		;;
	esac
	# Each word is on its own line, "<address or offset>: <value>", in order.
	i=0
	while read -r la lv; do
		la=${la%:}
		[ "$kind" = mem ] || { la=0x$la lv=0x$lv; }
		{ is_hex "$la" && is_hex "$lv" && [ "$((la))" = $((first + 4 * i)) ]; } ||
			stop "$line" "word $i came back as '$la $lv'"
		printf '0x%08x 32 0x%08x\n' $((base + la)) $((lv)) >>"$REC/dump.body"
		i=$((i + 1))
	done <<<"$out"
	[ "$i" = "$words" ] || stop "$line" "$i of $words words came back"
	rec "read $line"
done <<<"$LIST"

BOARD_TIMEOUT=60 board "$GUARD
"'for c in /sys/class/drm/card*-HDMI-A-*; do echo "== $c: $(cat "$c/status") $(cat "$c/enabled")"; cat "$c/modes"; done
for f in /sys/kernel/debug/dri/0/state /sys/kernel/debug/dri/0/vop2/summary /sys/kernel/debug/dri/0/vop2/regs \
	/sys/kernel/debug/pm_genpd/pm_genpd_summary /sys/kernel/debug/regmap/fed60000.phy/registers; do
	echo "== $f"; cat "$f"
done
echo "== clk_summary (display)"
grep -E " (hclk_vop|aclk_vop|dclk_vop[0-3]|pclk_hdmitx0|clk_hdmitx0_ref|pclk_hdptx0|pclk_vo1grf|pll_v0pll|pll_gpll|pll_cpll) " /sys/kernel/debug/clk/clk_summary
echo "== dmesg (display)"
dmesg | grep -iE "hdmi|hdptx|vop|drm"' >"$REC/files.txt" 2>&1 ||
	rec "files.txt is not complete (the registers above are)"

{
	cat <<EOF
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# The NanoPC-T6's display registers under Linux, read by
# driver/tools/kexec-zephyr/t6-display-dump.sh on
# $(date -u '+%F %T') UTC while the picture was on the first HDMI output.
#
# $(fact "$fresh" uname)
# Connector $(fact "$fresh" connector) (fde80000.hdmi) connected and enabled; EDID $(fact "$fresh" edid_bytes) bytes.
# Mode (drm_mode line of the active CRTC):$(fact "$fresh" mode)
# Power domains vop and vo1 on; hclk_vop, aclk_vop, pclk_hdmitx0, pclk_hdptx0 enabled.
# List: ${T6_DUMP_LIST:+another list, }${LIST##*# }.
#
# Format: ADDRESS WIDTH VALUE (hex, 32, hex), in the order of the read list.
# A GRF word is the address of its register; it was read through the GRF's
# regmap file.
EOF
	cat "$REC/dump.body"
} >"$REC/dump.txt"
rm "$REC/dump.body"
rec "done: $(grep -vc '^#' "$REC/dump.txt") words in $REC/dump.txt"
