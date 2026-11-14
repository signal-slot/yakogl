#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-crashkernel.sh - the NanoPC-T6's crashkernel= reservation for
# nanopc-t6.sh --crash (driver/docs/nanopc-t6-display.md, section 2(a)).
#
#   BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/t6-crashkernel.sh status
#   BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/t6-crashkernel.sh set
#   BOARD_HOST=root@<t6> driver/tools/kexec-zephyr/t6-crashkernel.sh undo
#
# status  reads only. The file's extraargs= line, what the running kernel was
#         given (/proc/cmdline), what it reserved (/proc/iomem's "Crash
#         kernel" line, /sys/kernel/kexec_crash_size, the dmesg lines of the
#         reservation and of CMA), and whether that reservation holds the
#         image window 0x10000000-0x1fffffff. Exit 0 when it does, 1 when not.
# set     adds crashkernel=256M@0x10000000 to the extraargs= line of
#         /boot/armbianEnv.txt (Armbian's boot script appends extraargs to the
#         kernel command line), or adds the line "extraargs=crashkernel=..."
#         at the end when the file has none. Before it writes, it keeps a copy
#         of the file and its sha256 in /root/yakogl-crashkernel/ on the board,
#         and after it writes it proves that every other line is unchanged.
#         The new file is written beside the old one and renamed over it, so
#         a power cut leaves the old file or the new one, never half of one.
# undo    puts the copy back and proves the file is byte for byte the one set
#         found (sha256 and cmp), then removes the copy. It refuses when the
#         file changed after set (someone else edited it; their edit would be
#         lost).
#
# Neither set nor undo reboots: the reservation is made (or dropped) by the
# next boot. Each prints what to run next. They take the board lock
# (board_lock.sh, LOCK_WAIT 0) so that they never run under a jump. Nothing
# else is touched: no other file of /boot, not the eMMC's loader area, no
# kernel module, no service.
#
# What Linux v6.18 does with crashkernel=256M@0x10000000 (read in the sources
# at the tag; facts with file:line):
#  - The last crashkernel= of the command line wins (kernel/crash_reserve.c:
#    219, :271). Without ':' it is the simple form size[@offset], both parsed
#    by memparse (K/M/G suffixes, 0x hex) (:153-173, :282-290).
#  - arm64 passes it to reserve_crashkernel_generic (arch/arm64/mm/init.c:
#    97-114). A non-zero offset is a fixed base: the only range searched is
#    [offset, offset + size) at CRASH_ALIGN = 2 MiB (kernel/crash_reserve.c:
#    401-412; arch/arm64/include/asm/crash_reserve.h:6). 0x10000000 is 2 MiB
#    aligned. If any byte of that range is already taken, there is no fallback:
#    the kernel logs "crashkernel reservation failed - memory is in use." and
#    reserves nothing (:418-421). On success it logs "crashkernel reserved:"
#    (:456) and /proc/iomem shows "Crash kernel" (:30, :470 or :528).
#  - What is taken before it: in arm64_memblock_init the initrd (init.c:
#    249-275), the kernel image (:282) and the device tree's reserved-memory
#    (:289); then in bootmem_init the CMA area (dma_contiguous_reserve, :327)
#    BEFORE the crash reservation (arch_reserve_crashkernel, :333).
#  - cma=256M (no @base) is placed by cma_alloc_mem below the limit
#    arm64_dma_phys_limit (kernel/dma/contiguous.c:211-252). On the T6 the RAM
#    starts below 4 GiB, so that limit is at most 4 GiB (arch/arm64/mm/init.c:
#    139-141); the bottom-up try above 4 GiB needs limit >= 4 GiB + size
#    (mm/cma.c:394) and is skipped; the allocation is memblock's default
#    top-down (mm/memblock.c:137, bottom_up = false; mm/cma.c:423): the
#    highest free 256 MiB below the limit, near the top of the bank
#    0x00200000-0xe143ffff. It can only reach 0x10000000 if no 256 MiB hole
#    is left above it. So CMA does not collide by the code; status prints the
#    measured "cma: Reserved ... at" line after the reboot to show it.
#  - The kernel's own placement: it runs where the boot loader put it (on the
#    board 0x02000000-0x0432ffff, read 2026-11-12), far below the window. The
#    initrd sits where U-Boot loaded it (ramdisk_addr_r) and is reserved
#    before the crash reservation (init.c:249-275): an initrd that grew past
#    0x10000000 would make the reservation fail, as would a reserved-memory
#    node added to the device tree there. Both show as the "memory is in use"
#    line, and status then says the window is not held.
#  - Panics and loads: nanopc-t6.sh --crash needs, in the reservation, the
#    image's segment and one 4 KiB control page after it (kernel/kexec.c:
#    65-71, kernel/kexec_core.c:395-450, :200-210).
#
# Environment:
#   BOARD_HOST      root@<the T6>. Required.
#   BOARD_SSH_OPTS  extra ssh options.
#   T6CK_ROOT       a directory the board's paths are read under, for
#                   board_lock_selftest.sh's fake board; empty on a real run.
set -euo pipefail

HERE=$(cd "$(dirname "$0")" && pwd)
WINDOW_BASE=$((0x10000000))
WINDOW_SIZE=$((256 << 20))
TOKEN="crashkernel=256M@0x10000000"

die() { echo "t6-crashkernel.sh: $*" >&2; exit 2; }

CMD=${1:-}
case "$CMD" in status | set | undo) ;; *) die "usage: BOARD_HOST=root@<t6> $0 status|set|undo" ;; esac
[ $# -eq 1 ] || die "one command only"
: "${BOARD_HOST:?set BOARD_HOST=root@<the NanoPC-T6>}"
# shellcheck disable=SC2206  # options are words
SSH_OPTS=(${BOARD_SSH_OPTS:-})

# The part that runs on the board: POSIX sh, sent on stdin. $1 is the command;
# T6CK_ROOT and TOKEN come in front of it on the command line.
# shellcheck disable=SC2016  # expanded on the board
BOARD_SCRIPT='
set -u
R=${T6CK_ROOT:-}
F=$R/boot/armbianEnv.txt
S=$R/root/yakogl-crashkernel
# The live extraargs= line: U-Boot env import skips blanks before a name
# (lib/hashtable.c:902-904, v2026.07), so a line with leading blanks counts.
E="^[[:blank:]]*extraargs="
say() { echo "board: $*"; }
no() { echo "board: REFUSED: $*"; echo "board: nothing was changed."; exit 3; }
sha() { sha256sum "$1" | cut -d" " -f1; }
# The command line and armbianEnv.txt carry the root UUID (root=, rootdev=),
# which must not reach a log or an issue: of a line only the crashkernel= and
# cma= words are printed.
toks() { tr " \t" "\n\n" | grep -E "^(crashkernel|cma)=" | tr "\n" " "; }
lines_toks() { # <grep pattern> <file>: "N: <tokens>" for each matching line
	grep -n "$1" "$2" | while IFS= read -r l; do
		printf "%s: %s\n" "${l%%:*}" "$(printf "%s\n" "${l#*:}" | toks)"
	done
}
case "$(tr "\0" " " <"$R/proc/device-tree/compatible" 2>/dev/null)" in
*friendlyarm,nanopc-t6*) ;;
*) no "not a NanoPC-T6 (compatible $(tr "\0" " " <"$R/proc/device-tree/compatible" 2>/dev/null || echo unread))" ;;
esac
[ "$(id -u)" = 0 ] || [ -n "$R" ] || no "not root"
[ -f "$F" ] || no "$F: no such file"
case "$1" in
facts)
	rel=$(cat "$R/proc/sys/kernel/osrelease")
	echo "kernel=$rel"
	for c in CONFIG_CRASH_RESERVE CONFIG_CRASH_DUMP CONFIG_KEXEC; do
		echo "config=$(grep -h "^$c=" "$R/boot/config-$rel" 2>/dev/null || echo "$c absent")"
	done
	echo "file_sha=$(sha "$F")"
	lines_toks "$E" "$F" | sed "s/^/file_extraargs=line /"
	lines_toks "crashkernel=" "$F" | sed "s/^/file_crashkernel=line /"
	if [ -d "$S" ]; then echo "state=$(cat "$S/orig.sha256" 2>/dev/null) $(cat "$S/new.sha256" 2>/dev/null)"; fi
	echo "cmdline=$(toks <"$R/proc/cmdline")"
	echo "kexec_crash_size=$(cat "$R/sys/kernel/kexec_crash_size" 2>/dev/null || echo absent)"
	echo "kexec_crash_loaded=$(cat "$R/sys/kernel/kexec_crash_loaded" 2>/dev/null || echo absent)"
	grep -E "^ *[0-9a-f]+-[0-9a-f]+ : Crash kernel$" "$R/proc/iomem" | sed "s/^ */iomem=/"
	# The lines of the reservation (kernel/crash_reserve.c:418-421, :445-446,
	# :456) and of CMA; never "Kernel command line:", which holds root=.
	dmesg 2>/dev/null | grep -v "command line" |
		grep -E "crashkernel reserved|crashkernel reservation failed|cannot allocate crashkernel|crashkernel: |cma: Reserved" |
		sed "s/^/dmesg=/"
	;;
set)
	rel=$(cat "$R/proc/sys/kernel/osrelease")
	for c in CONFIG_CRASH_RESERVE CONFIG_CRASH_DUMP CONFIG_KEXEC; do
		grep -qx "$c=y" "$R/boot/config-$rel" 2>/dev/null || no "the kernel $rel lacks $c=y: it would ignore the reservation"
	done
	if [ -d "$S" ]; then
		if [ -f "$S/new.sha256" ] && [ "$(sha "$F")" = "$(cat "$S/new.sha256")" ]; then
			say "already set by this tool ($F sha256 $(sha "$F")); nothing to do"
			exit 0
		fi
		no "$S exists but $F is not the file set wrote: run status, then undo or ask who changed it"
	fi
	if grep -q "$(printf "\r")" "$F"; then no "$F has CR line ends"; fi
	[ -z "$(tail -c 1 "$F")" ] || no "$F does not end with a newline"
	n=$(grep -c "$E" "$F" || true)
	[ "$n" -le 1 ] || no "$F has $n extraargs= lines"
	if grep -q "crashkernel=" "$F"; then
		no "$F already has a crashkernel= (line: tokens) $(lines_toks "crashkernel=" "$F" | tr "\n" " ")(the last one on the command line wins; this tool adds only its own)"
	fi
	if [ "$n" = 1 ]; then
		old=$(grep "$E" "$F")
		case "$old" in *=) new="$old$TOKEN" ;; *) new="$old $TOKEN" ;; esac
	else
		old="(none)"
		new="extraargs=$TOKEN"
	fi
	mkdir "$S" || no "cannot make $S"
	cp -p "$F" "$S/armbianEnv.txt.orig" || no "cannot copy $F"
	sha "$F" >"$S/orig.sha256"
	[ "$(sha "$S/armbianEnv.txt.orig")" = "$(cat "$S/orig.sha256")" ] || no "the copy differs from $F"
	if [ "$n" = 1 ]; then
		awk -v new="$new" "/^[ \t]*extraargs=/ { print new; next } { print }" "$F" >"$F.yakogl-new"
	else
		{ cat "$F"; printf "%s\n" "$new"; } >"$F.yakogl-new"
	fi
	# Every line but extraargs= is the old file, and the line is the new one.
	if ! { [ "$(grep -v "$E" "$F")" = "$(grep -v "$E" "$F.yakogl-new")" ] &&
		[ "$(grep "$E" "$F.yakogl-new")" = "$new" ] &&
		[ "$(wc -l <"$F.yakogl-new")" -eq $(( $(wc -l <"$F") + 1 - n )) ]; }; then
		rm -f "$F.yakogl-new"; rm -rf "$S"
		no "the new file would change more than the extraargs= line"
	fi
	chmod --reference="$F" "$F.yakogl-new" 2>/dev/null || true
	sync
	mv "$F.yakogl-new" "$F" || no "cannot rename the new file over $F"
	sync
	sha "$F" >"$S/new.sha256"
	sync
	say "extraargs before (crashkernel=, cma= words): $(printf "%s\n" "$old" | toks)"
	say "extraargs after  (crashkernel=, cma= words): $(printf "%s\n" "$new" | toks)"
	say "$F sha256 $(cat "$S/orig.sha256") -> $(cat "$S/new.sha256"); the copy for undo is $S/armbianEnv.txt.orig"
	;;
undo)
	[ -f "$S/armbianEnv.txt.orig" ] && [ -f "$S/orig.sha256" ] && [ -f "$S/new.sha256" ] ||
		no "no copy from set in $S: nothing to undo"
	orig=$(cat "$S/orig.sha256")
	[ "$(sha "$S/armbianEnv.txt.orig")" = "$orig" ] || no "the copy $S/armbianEnv.txt.orig is not the file set found (sha256 differs)"
	now=$(sha "$F")
	if [ "$now" != "$orig" ]; then
		[ "$now" = "$(cat "$S/new.sha256")" ] ||
			no "$F changed after set (sha256 $now, set wrote $(cat "$S/new.sha256")): restoring would drop that edit"
		cp -p "$S/armbianEnv.txt.orig" "$F.yakogl-new" || no "cannot copy back"
		sync
		mv "$F.yakogl-new" "$F" || no "cannot rename the copy over $F"
		sync
	fi
	if [ "$(sha "$F")" = "$orig" ] && cmp -s "$F" "$S/armbianEnv.txt.orig"; then
		say "$F is byte for byte the file set found (sha256 $orig, cmp equal)"
		rm -rf "$S"
		sync
	else
		echo "board: FAILED: $F is not the file set found; the copy stays in $S"
		exit 1
	fi
	;;
esac
'

if [ -n "${T6CK_ROOT:-}" ]; then rootq=$(printf '%q' "$T6CK_ROOT"); else rootq=; fi
board() { # <command>
	timeout 60 ssh "${SSH_OPTS[@]}" -o BatchMode=yes -o ConnectTimeout=10 "$BOARD_HOST" \
		"T6CK_ROOT=$rootq TOKEN=$TOKEN sh -s $1" <<<"$BOARD_SCRIPT"
}

if [ "$CMD" != status ]; then
	HOLD_MAX=${HOLD_MAX:-300}
	LOCK_WAIT=${LOCK_WAIT:-0}
	# shellcheck source=board_lock.sh
	. "$HERE/board_lock.sh"
	rc=0
	board "$CMD" || rc=$?
	[ "$rc" = 0 ] || { echo "t6-crashkernel.sh: $CMD did not complete (rc $rc)" >&2; exit "$rc"; }
	if [ "$CMD" = set ]; then
		echo "next: reboot the board (the reservation is made at boot), then: $0 status"
	else
		echo "next: reboot the board (the reservation is dropped at boot), then: $0 status"
	fi
	exit 0
fi

facts=$(board facts) || { echo "$facts"; die "cannot read the board"; }
fact() { printf '%s\n' "$facts" | sed -n "s/^$1=//p"; }
echo "kernel: $(fact kernel); $(fact config | tr '\n' ' ')"
echo "file: /boot/armbianEnv.txt sha256 $(fact file_sha); extraargs line: $(fact file_extraargs | tr '\n' ' ')"
echo "file: crashkernel= in it: $(fact file_crashkernel | tr '\n' ' ')"
if [ -n "$(fact state)" ]; then
	echo "file: set by this tool (orig, new sha256: $(fact state)); undo restores the orig"
else
	echo "file: no copy from set on the board"
fi
echo "running kernel: command line (crashkernel=, cma= words) '$(fact cmdline)', kexec_crash_size $(fact kexec_crash_size), kexec_crash_loaded $(fact kexec_crash_loaded)"
echo "running kernel: /proc/iomem Crash kernel: $(fact iomem | tr '\n' ';')"
fact dmesg | sed 's/^/running kernel: dmesg: /'
held=0
while IFS= read -r l; do
	[[ "$l" =~ ^([0-9a-f]+)-([0-9a-f]+)\ :\ Crash\ kernel$ ]] || continue
	s=$((16#${BASH_REMATCH[1]})) e=$((16#${BASH_REMATCH[2]}))
	if [ "$s" -le "$WINDOW_BASE" ] && [ "$e" -ge $((WINDOW_BASE + WINDOW_SIZE - 1)) ]; then held=1; fi
done <<<"$(fact iomem)"
# The crashkernel= words the running kernel was given (cmdline also has cma=).
ck_cmd=$(fact cmdline | tr ' ' '\n' | grep '^crashkernel=' | tr '\n' ' ' || true)
win=$(printf '0x%08x-0x%08x' "$WINDOW_BASE" $((WINDOW_BASE + WINDOW_SIZE - 1)))
if [ "$held" = 1 ]; then
	echo "status: the window $win is inside the reservation: nanopc-t6.sh --crash can run"
	exit 0
fi
if [ -n "$(fact file_crashkernel)" ] && [ -z "$ck_cmd" ]; then
	echo "status: the window $win is NOT reserved; the file has crashkernel= but the running kernel was not given it: reboot"
elif [ -n "$ck_cmd" ]; then
	echo "status: the window $win is NOT reserved although the kernel was given '$ck_cmd': see the dmesg lines above (\"memory is in use\" = something was placed there first)"
else
	echo "status: the window $win is NOT reserved (no crashkernel=): $0 set, then reboot"
fi
exit 1
