#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# Builds decode.c and decodes the synthetic dumps in testdata/ (built from the
# hvs6_dlist and pv_hdmi mock vectors), checking the decoded lines and that
# no absolute address appears in the output. Exit 0 when all checks pass.
here=$(cd "$(dirname "$0")" && pwd)
work=$(mktemp -d "${TMPDIR:-/tmp}/decode_selftest.XXXXXX") || exit 1
trap 'rm -rf "$work"' EXIT
bin=$(bash "$here/build.sh" "$work/decode") || { echo "decode_selftest: build failed"; exit 1; }

fails=0 checks=0
# has FILE REGEX [DESC]: the output of the dump FILE contains a line matching
has() {
	checks=$((checks + 1))
	if ! grep -Eq -- "$2" "$work/$1.out"; then
		echo "FAIL: $1: no line matching: $2"
		fails=$((fails + 1))
	fi
}
lacks() {
	checks=$((checks + 1))
	if grep -Eiq -- "$2" "$work/$1.out"; then
		echo "FAIL: $1: unexpected line matching: $2"
		fails=$((fails + 1))
	fi
}

for t in linux_ref_c probe_u3_d mem_dlist_d2; do
	"$bin" <"$here/testdata/$t.txt" >"$work/$t.out" 2>"$work/$t.err" ||
		{ echo "FAIL: $t: decode exit $?"; fails=$((fails + 1)); }
	# no absolute addresses: no hex number of 7+ digits, none of the test's
	# buffer addresses, no 0x10_7c block base
	lacks "$t" '0x[0-9a-f]{7,}'
	lacks "$t" '3e000000|3e140000|107c|7c580000'
done

# linux_ref format, generation C, 1080p
has linux_ref_c '^== linux_ref '
has linux_ref_c 'hvs: version 0x53 generation C, cxm_size 16384 words'
has linux_ref_c 'hvs ch0 \(\+0x0030\): enabled=yes running=yes mode=RUN empty=no frcnt=5 yline=123 size=1920x1080 head=3 active=3'
has linux_ref_c 'hvs ch1 \(\+0x0050\): enabled=no running=no mode=DISABLED empty=yes'
has linux_ref_c 'display list: not in dump'
has linux_ref_c 'hvs ch2: registers \(\+0x0070\.\.\) not in dump'
has linux_ref_c 'pv0: running=yes enabled=yes clk_select=1 format=0 fifo_level=16\+0'
has linux_ref_c 'pv0 timing: 1920x1080 active, h total 2200 \(sync 44 bp 148 fp 88\), v total 1125 \(sync 5 bp 36 fp 4\)'
has linux_ref_c 'hdmi0: HD_VID_CTL \(\+0x0044\) not in dump'

# probe snapshot lines, generation D, unity XRGB8888 plane from u3 dlist
has probe_u3_d '^== u3 '
has probe_u3_d 'generation D'
has probe_u3_d 'hvs ch0 \(\+0x0100\): enabled=yes running=yes mode=RUN'
has probe_u3_d 'hvs ch1: registers \(\+0x0140\.\.\) not in dump'
has probe_u3_d 'plane 0 \(word 0, 9 words\): RGBA8888 ARGB \(rgb32\) 1920x1080 at 0,0 pitch 7680 span 8294400 bytes addr base\+0x0$'
has probe_u3_d 'hdmi0: hdmi_vid_ctl named line \(\+0x0038\) enable=yes'
has probe_u3_d 'pv0 timing: 1920x1080'

# D2 console words, mem lines with display-list memory, two planes
has mem_dlist_d2 '^== D2 before '
has mem_dlist_d2 '^== D2 after '
has mem_dlist_d2 'hvs: version 0x00 generation unknown'
has mem_dlist_d2 'pv1: running=yes'
has mem_dlist_d2 'pv1 timing: 1920x1080 active'
has mem_dlist_d2 'display list \(memory \+0x4000, 22 words, from word 3\)'
has mem_dlist_d2 'plane 0 \(word 3, 9 words\): RGBA8888 ARGB \(rgb32\) 1280x256 at 0,0 pitch 5120 span 1310720 bytes addr base\+0x0$'
has mem_dlist_d2 'plane 1 \(word 12, 9 words\): RGBA8888 ARGB \(rgb32\) 1280x464 at 0,256 pitch 5120 span 2375680 bytes addr base\+0x140000$'
has mem_dlist_d2 'end of list at word 21'
has mem_dlist_d2 'hdmi0: HD_VID_CTL \(\+0x0044\) enable=yes'

# input without register words fails
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
