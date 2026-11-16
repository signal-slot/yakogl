#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
# Host self-test of decode.c: build it, decode the synthetic dumps in
# testdata/ (report-shaped fragments in the word forms of the
# driver/zephyr/lib/board/test mock vectors), check the decoded lines, and
# check that no absolute address reaches the output. Exit 0 when all checks
# pass, 1 on a failed check, 77 with "gap: CC" when no C compiler is found.
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/decode_selftest.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT

cc=${CC:-}
if [ -z "$cc" ]; then
	for c in cc gcc clang; do
		if command -v "$c" >/dev/null 2>&1; then cc=$c; break; fi
	done
fi
if [ -z "$cc" ] || ! command -v "$cc" >/dev/null 2>&1; then
	echo "gap: CC"
	exit 77
fi
export CC=$cc
bin=$(bash "$here/build.sh" "$work/decode") || { echo "decode_selftest: build failed"; exit 1; }

fails=0 checks=0
# has FILE REGEX: the decoded output of dump FILE contains a line matching REGEX
has() {
	checks=$((checks + 1))
	if ! grep -Eq -- "$2" "$work/$1.out"; then
		echo "FAIL: $1: no line matching: $2"
		fails=$((fails + 1))
	fi
}
# lacks FILE REGEX: no decoded output line of dump FILE matches REGEX
lacks() {
	checks=$((checks + 1))
	if grep -Eiq -- "$2" "$work/$1.out"; then
		echo "FAIL: $1: unexpected line matching: $2"
		fails=$((fails + 1))
	fi
}

for t in linux_ref_c mem_dlist probe_u3_d u3_notread u3_genunknown; do
	"$bin" <"$here/testdata/$t.txt" >"$work/$t.out" 2>"$work/$t.err" ||
		{ echo "FAIL: $t: decode exit $?"; fails=$((fails + 1)); }
	# the no-absolute-address rule: no hex number of 7+ digits anywhere in the
	# decoded output, and named here are the dumps' own block bases and plane
	# buffer addresses (register offsets print as at most 4 hex digits,
	# plane addresses as offsets "base+0x.." of at most 6)
	lacks "$t" '0x[0-9a-f]{7,}'
	lacks "$t" '3e000000|3e140000|3c000000|3c014000|107c|7c580000'
done

# linux_ref format: generation C from the version word; a running 1920x1080
# channel with no list in the dump, an idle channel, a channel not covered by
# the read; 1080p60 timing; VID_CTL past what linux_ref.sh reads
has linux_ref_c '^== linux_ref \(18 words\) =='
has linux_ref_c 'hvs: version 0x53 generation C, cxm_size 16384 words'
has linux_ref_c 'hvs ch0 \(\+0x0030\): enabled=yes running=yes mode=RUN empty=no frcnt=5 yline=123 size=1920x1080 head=3 active=3'
has linux_ref_c 'hvs ch1 \(\+0x0050\): enabled=no running=no mode=DISABLED empty=yes frcnt=0 yline=0 size=0x0 head=0 active=0'
has linux_ref_c 'hvs ch2: registers \(\+0x0070\.\.\) not in dump'
has linux_ref_c 'display list: not in dump'
has linux_ref_c 'pv0: running=yes enabled=yes clk_select=1 format=0 fifo_level=16\+0'
has linux_ref_c 'pv0 timing: 1920x1080 active, h total 2200 \(sync 44 bp 148 fp 88\), v total 1125 \(sync 5 bp 36 fp 4\)'
has linux_ref_c 'hdmi0 hd: VID_CTL \(\+0x0044\) not in dump'
lacks linux_ref_c 'plane '

# deeper mem.* read: ch0's list carried as HVS memory from +0x4000, walked
# from the word the channel's DL names; two planes; pixelvalve1 its own valve
has mem_dlist '^== linux_ref \(37 words\) =='
has mem_dlist 'hvs ch0 \(\+0x0030\): enabled=yes running=yes mode=RUN empty=no frcnt=1 yline=0 size=1280x720 head=3 active=3'
has mem_dlist 'display list \(memory \+0x4000, 22 words, from word 3\)'
has mem_dlist 'plane 0 \(word 3, 9 words\): RGBA8888 ARGB \(rgb32\) 1280x256 at 0,0 pitch 5120 span 1310720 bytes addr base\+0x0$'
has mem_dlist 'plane 1 \(word 12, 9 words\): RGBA8888 ARGB \(rgb32\) 1280x464 at 0,256 pitch 5120 span 2375680 bytes addr base\+0x140000$'
has mem_dlist 'end of list at word 21'
has mem_dlist 'hvs ch1: registers \(\+0x0050\.\.\) not in dump'
has mem_dlist 'pv0: not in dump'
has mem_dlist 'pv1: running=yes'
has mem_dlist 'pv1 timing: 1920x1080 active'
has mem_dlist 'hdmi0 hd: VID_CTL \(\+0x0044\) enable=yes'
lacks mem_dlist 'u3 '

# probe snapshot lines: generation D, so the D-group channel offsets are used
# and the C-group words in the same table are not; the list from the u3 dlist
# capture - the probe read it twice, the capture reset at [0], so the decoded
# list is the read after the mailbox allocation (one plane, pitch 7680), not
# the read before it (two planes, pitch 10240)
has probe_u3_d '^== u3 \(60 words\) =='
has probe_u3_d 'hvs: version 0x54 generation D, cxm_size 16384 words'
has probe_u3_d 'hvs ch0 \(\+0x0100\): enabled=yes running=yes mode=RUN empty=no frcnt=5 yline=123 size=1920x1080 head=0 active=0'
has probe_u3_d 'display list \(u3 dlist, 9 words, from index 0\)'
has probe_u3_d 'plane 0 \(word 0, 9 words\): RGBA8888 ARGB \(rgb32\) 1920x1080 at 0,0 pitch 7680 span 8294400 bytes addr base\+0x0$'
has probe_u3_d 'list continues past the dump at word 9'
has probe_u3_d 'hvs ch1 \(\+0x0140\): enabled=no running=no mode=DISABLED empty=no'
has probe_u3_d 'hvs ch2 \(\+0x0180\): enabled=no'
has probe_u3_d 'pv0: running=yes enabled=yes clk_select=1 format=0 fifo_level=16\+0'
has probe_u3_d 'pv0 timing: 1920x1080'
has probe_u3_d 'hdmi0 hd: VID_CTL \(\+0x0044\) enable=yes'
has probe_u3_d 'hdmi0 hd: hdmi1 VID_CTL \(\+0x0048\) enable=no'
lacks probe_u3_d '19 words'
lacks probe_u3_d 'plane 1 '
lacks probe_u3_d 'hact'

# blocks the probe could not read report as reading all ones, and are not
# decoded: no channel enabled from a 0 (an idle HVS was read here, its words
# are not all ones), no valve line, no HDMI enable
has u3_notread '^== u3 \(60 words\) =='
has u3_notread 'hvs ch0 \(\+0x0100\): enabled=no running=no'
has u3_notread 'pv0: control and v_control read as all ones \(the block was not read\)'
has u3_notread 'hdmi0 hd: VID_CTL \(\+0x0044\) reads as all ones'
has u3_notread 'hdmi0 hd: hdmi1 VID_CTL \(\+0x0048\) reads as all ones'
lacks u3_notread 'ffffffff'
lacks u3_notread 'enabled=yes'
lacks u3_notread 'clk_select'
lacks u3_notread 'enable='
lacks u3_notread 'fifo_level'
lacks u3_notread 'timing:'

# a version naming neither stepping: main.c's own rule, no channel offsets are
# safe and no list is measured; the valve and hdmi words still decode
has u3_genunknown '^== u3 \(60 words\) =='
has u3_genunknown 'hvs: version 0x55 generation unknown, cxm_size 16384 words'
has u3_genunknown 'pv0: running=yes'
has u3_genunknown 'pv0 timing: 1920x1080 active'
has u3_genunknown 'hdmi0 hd: VID_CTL \(\+0x0044\) enable=yes'
has u3_genunknown 'hdmi0 hd: hdmi1 VID_CTL \(\+0x0048\) enable=no'
lacks u3_genunknown 'hvs ch'
lacks u3_genunknown 'display list'
lacks u3_genunknown 'plane '

# a line with a log prefix parses (console captures carry one)
printf 'rpi5_display: u3 reg hvs version +0x0000=0x00000054\n' | "$bin" >"$work/prefix.out" 2>&1 ||
	{ echo "FAIL: prefix: decode exit $?"; fails=$((fails + 1)); }
has prefix 'generation D'

# an all-ones version word names the whole HVS unread: no channel is decoded
printf 'mem.hvs[0x0]: 0xffffffff\n' | "$bin" >"$work/allones.out" 2>&1 ||
	{ echo "FAIL: allones: decode exit $?"; fails=$((fails + 1)); }
has allones 'version reads as all ones \(the block was not read\); channels not decoded'
lacks allones 'ch0'

# a valve whose timing registers name an active size zero says so
printf 'mem.pixelvalve0[0x0]: 0x00000005\nmem.pixelvalve0[0x4]: 0x00000000\nmem.pixelvalve0[0xc]: 0x00000000\nmem.pixelvalve0[0x10]: 0x00000000\nmem.pixelvalve0[0x14]: 0x00000000\nmem.pixelvalve0[0x18]: 0x00000000\n' |
	"$bin" >"$work/zeroguard.out" 2>&1 ||
	{ echo "FAIL: zeroguard: decode exit $?"; fails=$((fails + 1)); }
has zeroguard 'pv0 timing: not set up \(active size zero\)'

# input without register words is refused, not answered with an empty decode
if echo "nothing here" | "$bin" >/dev/null 2>&1; then
	echo "FAIL: empty input accepted"
	fails=$((fails + 1))
fi
checks=$((checks + 1))

if [ "$fails" -ne 0 ]; then
	echo "decode_selftest: $fails of $checks checks failed"
	exit 1
fi
echo "decode_selftest: $checks of $checks checks, 0 failed"
