#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-verdict-test.sh - t6-verdict.sh over the fixtures in testdata/ (*.uart:
# driver/.gitignore ignores *.log): each case's exit code, verdict line and
# per-line statuses. Host only.
#
#   bash driver/tools/kexec-zephyr/t6-verdict-test.sh
#
# The fixtures are the predicted records (driver/docs/nanopc-t6-mali.md:
# Z1's measured t6_hello and t6_gpu lines, Z2's "The expected uart2 record" with
# P = 0x10027000 and B = 0x10037000), not board output. The Z1 stop fixtures
# (t6_mmu-z1stop, t6_fw-z1fail, t6_submit-z3-stop, and the crash route's
# t6_fw-z1-takeover-mask and -hard) and the crash route's success record
# (t6_gpu-crash-pass) carry Z1's record as
# tools/probes/nanopc_t6/common/t6_z1.c prints it: the host cc builds that file
# with common/test/t6_z1_test.c, and its lines must be the fixture's.
# triangle-cold-pass is the Zephyr part of a board record: the triangle image
# after a normal kexec, its first show the cold chain at 1920x1080
# (2026-11-24, a development run; the tree name in its build line is the
# placeholder of the other fixtures). triangle-adopt-pass is predicted from the
# code (examples/triangle, the board layer, the drivers crate), with the
# register values of a board record of the adopt after kexec --crash.
set -euo pipefail
export LC_ALL=C

HERE=$(cd "$(dirname "$0")" && pwd)
V=$HERE/t6-verdict.sh
D=$HERE/testdata
WORK=$(mktemp -d "${TMPDIR:-$HERE}/t6-verdict-test.XXXXXX")
trap 'rm -rf "$WORK"' EXIT

fails=0 runs=0

# check <name> <want rc> <want verdict line> <want "ok missing differs"> <probe> <log> [env...]
check() {
	local name=$1 want_rc=$2 want_v=$3 want_n=$4 probe=$5 log=$6
	shift 6
	local out rc=0 got_v got_n
	out=$(env "$@" bash "$V" "$probe" "$log" 2>&1) || rc=$?
	got_v=$(printf '%s\n' "$out" | tail -n 1)
	got_n="$(printf '%s\n' "$out" | grep -c '^ok ' || true) $(printf '%s\n' "$out" | grep -c '^missing ' || true) $(printf '%s\n' "$out" | grep -c '^differs ' || true)"
	runs=$((runs + 1))
	if [ "$rc" = "$want_rc" ] && [ "$got_v" = "$want_v" ] && [ "$got_n" = "$want_n" ]; then
		echo "ok   $name: rc $rc, $got_v, ok/missing/differs $got_n"
	else
		echo "FAIL $name: rc $rc (want $want_rc), '$got_v' (want '$want_v'), ok/missing/differs $got_n (want $want_n)"
		printf '%s\n' "$out" | grep -v '^ok ' | sed 's/^/     /'
		fails=$((fails + 1))
	fi
}

# the line counts: boot 12 + reset 1 + each probe's own
MMU=29 HELLO=27 GPU=34

check t6_mmu-pass 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$D/t6_mmu-pass.uart"
check t6_mmu-missing 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 1 0" t6_mmu "$D/t6_mmu-missing.uart"
check t6_mmu-differs 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 0 1" t6_mmu "$D/t6_mmu-differs.uart"

# the board's pins line (issue #11859): its words are recorded, an rc below
# 0 for the 5 V switch or for the HDMI0 pins fails the record
PINS='t6: vbus5v0_usb rc 0, GPIO4_B0 level 1: 0xfd5f8088 0x00000000 -> 0x00000000 0xfd5fc144 0x0000d7fd -> 0x0000d7fc 0xfec50000 0x00000000 -> 0x00000100 0xfec50008 0x00000020 -> 0x00000120; HDMI0 pins rc 0: 0xfd5f8090 0x00000000 -> 0x00000055 0xfd5fc148 0x00001557 -> 0x00001550'
sed "/^board: watchdog armed\$/i $PINS" "$D/t6_mmu-pass.uart" >"$WORK/pins.log"
check t6_mmu-pins 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$WORK/pins.log"
sed 's/^t6: vbus5v0_usb rc 0, GPIO4_B0 level 1: /t6: vbus5v0_usb rc -5, GPIO4_B0 level 0: /; s/-> 0x00000100 /-> 0x00000000 (differs) /' "$WORK/pins.log" >"$WORK/pins-vbus.log"
check t6_mmu-pins-vbus 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$MMU 0 1" t6_mmu "$WORK/pins-vbus.log"
sed 's/; HDMI0 pins rc 0: /; HDMI0 pins rc -5: /; s/-> 0x00000055 /-> 0x00000000 (differs) /' "$WORK/pins.log" >"$WORK/pins-hdmi.log"
check t6_mmu-pins-hdmi 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$MMU 0 1" t6_mmu "$WORK/pins-hdmi.log"

# the UART's CR LF, and NULs around the jump
sed 's/$/\r/' "$D/t6_mmu-pass.uart" | sed '1s/^/\x00\x00/' >"$WORK/crlf.log"
check t6_mmu-crlf 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$WORK/crlf.log"

# the PAs from nm (with and without 0x): equal passes, another differs on
# the pool line and every line derived from P
check t6_mmu-nm 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$D/t6_mmu-pass.uart" \
	T6_POOL_PA=0000000010027000 T6_BACKING_PA=0x10037000
check t6_mmu-nm-other 1 "VERDICT t6_mmu FAIL (6 of $MMU)" "$((MMU - 6)) 0 6" t6_mmu "$D/t6_mmu-pass.uart" \
	T6_POOL_PA=0x10017000 T6_BACKING_PA=0x10037000

# the identity map: a pool %p that is not its PA differs on that line only
sed 's/^t6_mmu: pool 0x10027000 /t6_mmu: pool 0xffff000010027000 /' "$D/t6_mmu-pass.uart" >"$WORK/va.log"
check t6_mmu-pool-va 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 0 1" t6_mmu "$WORK/va.log"

# FAULTADDRESS as found is predicted 0, not recorded
sed '/AS0 as found/s/FAULTADDRESS 0x0000000000000000/FAULTADDRESS 0x0000000004001000/' "$D/t6_mmu-pass.uart" >"$WORK/fa.log"
check t6_mmu-faultaddress 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 0 1" t6_mmu "$WORK/fa.log"

# a FAIL line from the probe is a differing PASS line, the lines it skips
# missing
sed -e '/the flush after the map/d' \
	-e 's/^t6_mmu: PASS: .*/t6_mmu: FAIL: stop 9 (the flush after the map stopped), refusal 3, detail 0x0000000000000304/' \
	"$D/t6_mmu-pass.uart" >"$WORK/fail.log"
check t6_mmu-probe-fail 1 "VERDICT t6_mmu FAIL (2 of $MMU)" "$((MMU - 2)) 1 1" t6_mmu "$WORK/fail.log"

# an empty record (the first Z1 jump's 0-byte uart.log)
: >"$WORK/empty.log"
check t6_mmu-empty 1 "VERDICT t6_mmu FAIL ($MMU of $MMU)" "0 $MMU 0" t6_mmu "$WORK/empty.log"

# Z1 stopped in t6_mmu (stop 5, the record of 2026-11-11 13:13): the Z1 line
# differs, Z1's record follows it (8 lines judged, ok), Z2's 12 lines are
# missing and the FAIL line stands for PASS
Z1=8
check t6_mmu-z1stop 1 "VERDICT t6_mmu FAIL (14 of $((MMU + Z1)))" "$((MMU + Z1 - 14)) 12 2" t6_mmu "$D/t6_mmu-z1stop.uart"

# another probe's record
check t6_gpu-over-mmu 1 "VERDICT t6_gpu FAIL ($((GPU - 12)) of $GPU)" "12 $((GPU - 12)) 0" t6_gpu "$D/t6_mmu-pass.uart"

check t6_hello-pass 0 'VERDICT t6_hello PASS' "$HELLO 0 0" t6_hello "$D/t6_hello-pass.uart"
# an active priority left by Linux is recorded, not judged: the reset cleared
# it (issue #6366)
sed 's/ICC_AP1R0_EL1 0x00000000 ICC_BPR1_EL1 0x0 /ICC_AP1R0_EL1 0x00100000 ICC_BPR1_EL1 0x7 /' "$D/t6_hello-pass.uart" >"$WORK/ap1r.log"
check t6_hello-ap1r-active 0 'VERDICT t6_hello PASS' "$HELLO 0 0" t6_hello "$WORK/ap1r.log"
# an image without the clear, or one that found SRE clear: that line missing
sed 's/^t6: GICv3 CPU interface: ICC_AP1R0_EL1 .*/t6: GICv3 CPU interface: ICC_SRE_EL1 0x0, SRE clear: not touched/' "$D/t6_hello-pass.uart" >"$WORK/sre.log"
check t6_hello-no-ap1r 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 1 0" t6_hello "$WORK/sre.log"

# T6_ROUTE=uboot (U-Boot's go, t6_hello-uboot-pass: the predicted record,
# not board output): the T6:G line in place of the GIC EOImode line, so the
# count is the same. Each route's record judged as the other's names its
# lines: the go record as kexec misses the EOImode line and has a T6:G; the
# kexec record as uboot misses T6:G.
check t6_hello-uboot-pass 0 'VERDICT t6_hello PASS' "$HELLO 0 0" t6_hello "$D/t6_hello-uboot-pass.uart" T6_ROUTE=uboot
check t6_hello-uboot-as-kexec 1 "VERDICT t6_hello FAIL (2 of $HELLO)" "$((HELLO - 1)) 1 1" t6_hello "$D/t6_hello-uboot-pass.uart"
check t6_hello-kexec-as-uboot 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 1 0" t6_hello "$D/t6_hello-pass.uart" T6_ROUTE=uboot
# a T6:G line that names another range differs
sed 's/0x10000000-0x20000000 cleaned/0x10000000-0x10800000 cleaned/' "$D/t6_hello-uboot-pass.uart" >"$WORK/range.log"
check t6_hello-uboot-range 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/range.log" T6_ROUTE=uboot
# the MMU or a cache left on must never pass: T6:A's SCTLR_EL2 with M, C, I
# set (T6:A differs, and T6:G's read-back no longer equals it); T6:G's
# read-back with them set; a T6:G whose read-back is not T6:A's value
sed '/^T6:A EL2/s/SCTLR_EL2 0x0000000030c50830/SCTLR_EL2 0x0000000030c51835/' "$D/t6_hello-uboot-pass.uart" >"$WORK/a-on.log"
check t6_hello-uboot-t6a-mmu-on 1 "VERDICT t6_hello FAIL (2 of $HELLO)" "$((HELLO - 2)) 0 2" t6_hello "$WORK/a-on.log" T6_ROUTE=uboot
sed '/^T6:G/s/0x0000000030c51835 -> 0x0000000030c50830/0x0000000030c51835 -> 0x0000000030c51835/' "$D/t6_hello-uboot-pass.uart" >"$WORK/g-on.log"
check t6_hello-uboot-t6g-left-on 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/g-on.log" T6_ROUTE=uboot
sed '/^T6:A EL2/a T6:G go: SCTLR_EL2 0x0000000030c51835 -> 0x0000000030c50830 (MMU, D-cache, I-cache off); 0x10000000-0x20000000 cleaned and invalidated to PoC, TLB and I-cache invalidated' \
	"$D/t6_hello-pass.uart" >"$WORK/g-other.log"
check t6_hello-uboot-t6g-not-t6a 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/g-other.log" T6_ROUTE=uboot
# the same record on the kexec route: a T6:G line there differs
check t6_hello-kexec-with-t6g 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$HELLO 0 1" t6_hello "$WORK/g-other.log"
# T6:A's SCTLR_EL2 with the MMU on fails the kexec route too
sed '/^T6:A EL2/s/SCTLR_EL2 0x0000000030400800/SCTLR_EL2 0x0000000030400801/' "$D/t6_hello-pass.uart" >"$WORK/k-on.log"
check t6_hello-kexec-t6a-mmu-on 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/k-on.log"

# the GIC active state line (issue #10530). The fixtures' GICD_TYPER 0xf
# and GICR_TYPER 0 are of the right form, not a prediction: the verdict
# reads them as tokens, and checks the words written against them (1 +
# ITLinesNumber 15 = 16). INTIDs Linux left Active are recorded, not
# judged: the clear deactivated them
GA='t6: GIC active state: '
sed "s/^${GA}\(.*\) as found, 0 INTID(s) active (none)/${GA}\1 as found, 2 INTID(s) active (INTIDs 27 451)/; s/GICR_ISACTIVER0 0x00000000/GICR_ISACTIVER0 0x08000000/" \
	"$D/t6_hello-pass.uart" >"$WORK/active.log"
check t6_hello-gic-active-found 0 'VERDICT t6_hello PASS' "$HELLO 0 0" t6_hello "$WORK/active.log"
# an image without the clear: the line missing
grep -v "^$GA" "$D/t6_hello-pass.uart" >"$WORK/noactive.log"
check t6_hello-gic-active-missing 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 1 0" t6_hello "$WORK/noactive.log"
# an INTID still Active after the clear
sed "/^$GA/s/, 0 still active\$/, 1 still active/" "$D/t6_hello-pass.uart" >"$WORK/still.log"
check t6_hello-gic-active-still 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/still.log"
# no redistributor for CPU 0: the other form of the line
sed "s/^${GA}GICD_TYPER 0x0000000f .* as found, /${GA}GICD_TYPER 0x0000000f, no redistributor with affinity 0x00000000 in 4 frame(s): SGIs and PPIs not scanned, /; /^$GA/s/, 16 word(s)/, 15 word(s)/" \
	"$D/t6_hello-pass.uart" >"$WORK/nordist.log"
check t6_hello-gic-no-redistributor 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/nordist.log"
# the line twice: once per boot
sed "/^$GA/p" "$D/t6_hello-pass.uart" >"$WORK/twice.log"
check t6_hello-gic-active-twice 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/twice.log"
# one word fewer or more than GICD_TYPER's ITLinesNumber 15 announces
sed "/^$GA/s/, 16 word(s)/, 15 word(s)/" "$D/t6_hello-pass.uart" >"$WORK/words15.log"
check t6_hello-gic-words-fewer 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/words15.log"
sed "/^$GA/s/, 16 word(s)/, 17 word(s)/" "$D/t6_hello-pass.uart" >"$WORK/words17.log"
check t6_hello-gic-words-more 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/words17.log"
# the extended ranges count too: GICD_TYPER ESPI with ESPI_range 1 (2
# words) and GICR_TYPER PPInum 2 (2 words): 1 + 2 + 15 + 2 = 20
sed "/^$GA/s/GICD_TYPER 0x0000000f GICR_TYPER 0x0000000000000000 /GICD_TYPER 0x0800010f GICR_TYPER 0x0000000010000000 /" \
	"$D/t6_hello-pass.uart" >"$WORK/ext16.log"
sed "/^$GA/s/, 16 word(s)/, 20 word(s)/" "$WORK/ext16.log" >"$WORK/ext20.log"
check t6_hello-gic-words-extended 0 'VERDICT t6_hello PASS' "$HELLO 0 0" t6_hello "$WORK/ext20.log"
check t6_hello-gic-words-extended-missed 1 "VERDICT t6_hello FAIL (1 of $HELLO)" "$((HELLO - 1)) 0 1" t6_hello "$WORK/ext16.log"
check t6_gpu-pass 0 'VERDICT t6_gpu PASS' "$GPU 0 0" t6_gpu "$D/t6_gpu-pass.uart"
# the crash kexec: PD_GPU handed over on, Z1 takes the GPU over (four lines)
TAKE=4
check t6_gpu-crash-pass 0 'VERDICT t6_gpu PASS' "$((GPU + TAKE)) 0 0" t6_gpu "$D/t6_gpu-crash-pass.uart"
# an image without the take-over on the crash route: its four lines missing
sed '/^t6_gpu: take-over: /d' "$D/t6_gpu-crash-pass.uart" >"$WORK/notake.log"
check t6_gpu-crash-no-take-over 1 "VERDICT t6_gpu FAIL ($TAKE of $((GPU + TAKE)))" "$GPU $TAKE 0" t6_gpu "$WORK/notake.log"
# the route is chosen by REPAIR_STATUS bit 1 on the hand-over line, as
# gpu_up chooses it: an off-route record whose PD_GPU read powered lacks the
# four take-over lines
sed 's/^\(t6_gpu: PD_GPU at hand-over: .* REPAIR_STATUS 0xfad4000\)1: off or idle$/\13: off or idle/' "$D/t6_gpu-pass.uart" >"$WORK/on.log"
check t6_gpu-on-without-take-over 1 "VERDICT t6_gpu FAIL ($TAKE of $((GPU + TAKE)))" "$GPU $TAKE 0" t6_gpu "$WORK/on.log"

# not_pass <name> <log> <ERE>: t6_gpu's verdict on <log> is FAIL (exit 1),
# and a line not ok names the line that <ERE> matches
not_pass() {
	local name=$1 log=$2 re=$3 out rc=0
	out=$(bash "$V" t6_gpu "$log" 2>&1) || rc=$?
	runs=$((runs + 1))
	if [ "$rc" = 1 ] && [[ $(printf '%s\n' "$out" | tail -n 1) == 'VERDICT t6_gpu FAIL ('* ]] &&
		printf '%s\n' "$out" | grep -v '^ok ' | grep -Eq "$re"; then
		echo "ok   $name: rc 1, not ok: $re"
	else
		echo "FAIL $name: rc $rc (want 1), no line not ok matching $re"
		printf '%s\n' "$out" | grep -v '^ok ' | sed 's/^/     /'
		fails=$((fails + 1))
	fi
}

# a take-over that did not go cleanly is no PASS: each altered crash record
# differs on the line that shows it
X="$D/t6_gpu-crash-pass.uart"
sed 's/MCU halt: MCU_STATUS 0 after 3 polls of 1 us: DISABLED$/MCU halt: MCU_STATUS 1 after 100001 polls of 1 us: not DISABLED in 100 ms, reset anyway/' "$X" >"$WORK/halt-timeout.log"
not_pass t6_gpu-take-over-halt-timed-out "$WORK/halt-timeout.log" '^differs .*seen "t6_gpu: take-over: MCU halt: MCU_STATUS 1 '
sed 's/MCU halt: MCU_STATUS 0 after/MCU halt: MCU_STATUS 3 after/' "$X" >"$WORK/halt-fatal.log"
not_pass t6_gpu-take-over-halt-fatal "$WORK/halt-fatal.log" '^differs .*seen "t6_gpu: take-over: MCU halt: MCU_STATUS 3 '
sed 's/^t6_gpu: take-over: soft reset:/t6_gpu: take-over: hard reset:/' "$X" >"$WORK/hard.log"
not_pass t6_gpu-take-over-hard-reset "$WORK/hard.log" '^differs .*seen "t6_gpu: take-over: hard reset:'
sed 's/then masks 0x00000000 0x00000000 0x00000000 MCU_STATUS 0$/then masks 0x00020103 0xffffffff 0x0000ffff MCU_STATUS 1/' "$X" >"$WORK/after.log"
not_pass t6_gpu-take-over-not-quiet-after-reset "$WORK/after.log" '^differs .*seen "t6_gpu: take-over: soft reset: .*MCU_STATUS 1"'
sed 's/MCU_STATUS 0$/MCU_STATUS 1/' "$X" >"$WORK/after-mcu.log"
not_pass t6_gpu-take-over-mcu-enabled-after-reset "$WORK/after-mcu.log" '^differs .*seen "t6_gpu: take-over: soft reset: .*MCU_STATUS 1"'
# the masks as found are not panthor's (the JOB mask reads the QoS PRIORITY
# word): a wrong map
sed 's/JOB_INT_MASK 0xffffffff MMU_INT_MASK 0x0000ffff/JOB_INT_MASK 0x80000000 MMU_INT_MASK 0x00000001/' "$X" >"$WORK/aliased.log"
not_pass t6_gpu-take-over-found-not-panthors "$WORK/aliased.log" '^differs .*seen "t6_gpu: take-over: found '
# a take-over line on the off route
sed '/^t6_gpu: PD_GPU after: /a t6_gpu: take-over: found GPU_INT_MASK 0x00000000 JOB_INT_MASK 0x00000000 MMU_INT_MASK 0x00000000, RAWSTAT 0x00000000 0x00000000 0x00000000, MCU_STATUS 0' \
	"$D/t6_gpu-pass.uart" >"$WORK/off-take.log"
not_pass t6_gpu-off-route-take-over-line "$WORK/off-take.log" '^differs  expected no line matching "\^t6_gpu: take-over: "'
# PD_GPU powered with its idle request set: t6_z1.c prints "off or idle",
# gpu_up takes it over, and the verdict judges the take-over lines (all ok)
sed 's/^\(t6_gpu: PD_GPU at hand-over: .* BUS_IDLE_REQ 0x000083c\)e\(.*\): on$/\1f\2: off or idle/' "$X" >"$WORK/idle.log"
out=$(bash "$V" t6_gpu "$WORK/idle.log" 2>&1) && rc=0 || rc=$?
runs=$((runs + 1))
n=$(printf '%s\n' "$out" | grep -c '^ok .*seen "t6_gpu: take-over: ' || true)
if [ "$rc" = 0 ] && [ "$n" = "$TAKE" ] && grep -q 'BUS_IDLE_REQ 0x000083cf .*: off or idle$' "$WORK/idle.log"; then
	echo "ok   t6_gpu-powered-idle: PASS, its $n take-over lines judged"
else
	echo "FAIL t6_gpu-powered-idle: rc $rc, $n take-over lines ok (want $TAKE)"
	printf '%s\n' "$out" | grep -v '^ok ' | sed 's/^/     /'
	fails=$((fails + 1))
fi

# t6_fw: boot 12 + build, WDT0, image, 7 sections, pool, Z3's rc line and 7
# record lines, PASS, reset = 33
FW=33
check t6_fw-pass 0 'VERDICT t6_fw PASS' "$FW 0 0" t6_fw "$D/t6_fw-pass.uart"
check t6_fw-missing 1 "VERDICT t6_fw FAIL (1 of $FW)" "$((FW - 1)) 1 0" t6_fw "$D/t6_fw-missing.uart"
# Z1 refused (step 3; NPLL did not lock, stop 4): the rc line differs, so does
# the stop line, the FAIL line stands for PASS (differs, it names the step)
# and the Z3 record never came; Z1's record follows the stop line (8 ok)
check t6_fw-z1fail 1 "VERDICT t6_fw FAIL (10 of $((FW + Z1)))" "$((FW + Z1 - 9)) 7 3" t6_fw "$D/t6_fw-z1fail.uart"
# the crash route's take-over refused (stop 14): judged as any Z1 stop; the
# record's take-over lines show only the words read (a mask stuck: the found
# line; neither reset: found, cleared, the MCU halt that timed out)
for m in mask hard; do
	check "t6_fw-z1-takeover-$m" 1 "VERDICT t6_fw FAIL (10 of $((FW + Z1)))" "$((FW + Z1 - 9)) 7 3" t6_fw "$D/t6_fw-z1-takeover-$m.uart"
done
# the same stop from an image that does not print Z1's record: its 8 lines
# are named missing
sed '/^t6_fw: Z3: stop /,/^t6_fw: Z1 stopped: /{/^t6_fw: Z3: stop /!d}' "$D/t6_fw-z1fail.uart" >"$WORK/z1bare.log"
check t6_fw-z1-no-record 1 "VERDICT t6_fw FAIL (18 of $((FW + Z1)))" "$((FW - 9)) 15 3" t6_fw "$WORK/z1bare.log"
# stops at boot with no GLB version (step 5, VersionZero): the rc line, the
# version, the interface and the configuration lines differ or are missing
check t6_fw-boot-stop 1 "VERDICT t6_fw FAIL (7 of $FW)" "$((FW - 6)) 2 5" t6_fw "$D/t6_fw-boot-stop.uart"
# the PING not acked (step 5, PingTimeout): the rc line, the last record line
# (acked 0xe, NOT acked) and the FAIL line differ, with the stop line
check t6_fw-ping-lost 1 "VERDICT t6_fw FAIL (4 of $FW)" "$((FW - 3)) 0 4" t6_fw "$D/t6_fw-ping-lost.uart"
# JOB_INT_RAWSTAT without bit 31 differs (hook)
sed 's/JOB_INT_RAWSTAT 0x80000000/JOB_INT_RAWSTAT 0x00000000/' "$D/t6_fw-pass.uart" >"$WORK/job.log"
check t6_fw-job-bit 1 "VERDICT t6_fw FAIL (1 of $FW)" "$((FW - 1)) 0 1" t6_fw "$WORK/job.log"
# GLB version 1.4.0 differs on that line only
sed 's/GLB version 1.5.0 (0x01050000)/GLB version 1.4.0 (0x01040000)/' "$D/t6_fw-pass.uart" >"$WORK/ver.log"
check t6_fw-version 1 "VERDICT t6_fw FAIL (1 of $FW)" "$((FW - 1)) 0 1" t6_fw "$WORK/ver.log"

# t6_submit: t6_fw's 33 lines (the Z3 record) + 11 Z4 lines = 44. The Z4 lines
# are the model's records (board::nanopc_t6_submit's
# the_fixtures_z4_lines_are_the_model_s_records); a stopped run prints its
# record too and is judged by its status fields.
SUB=44
check t6_submit-pass 0 'VERDICT t6_submit PASS' "$SUB 0 0" t6_submit "$D/t6_submit-pass.uart"
# wait timeout (step 14, refusal 19): Z4a, Z4e (extract and seqno 0), Z4j and
# Z4k (the drain did not run: extract 0, ack 0) and the FAIL line differ
check t6_submit-wait-timeout 1 "VERDICT t6_submit FAIL (5 of $SUB)" "$((SUB - 5)) 0 5" t6_submit "$D/t6_submit-wait-timeout.uart"
# Z3 stopped (Z1 refused, stop 5): the rc line, Z4a..Z4e, Z4j and Z4k (zeros),
# the FAIL line and the stop line differ, the 7 record lines are missing;
# Z1's record follows the stop line (8 ok)
check t6_submit-z3-stop 1 "VERDICT t6_submit FAIL (17 of $((SUB + Z1)))" "$((28 + Z1)) 7 10" t6_submit "$D/t6_submit-z3-stop.uart"
# cut after Z4f: Z4g .. Z4k, PASS and the reset line missing
check t6_submit-missing 1 "VERDICT t6_submit FAIL (7 of $SUB)" "$((SUB - 7)) 7 0" t6_submit "$D/t6_submit-missing.uart"
# the watchdog reset the probe during the Z4 wait: the capture ends after the
# last Z3 line with a fresh boot banner. Z4a..Z4k, PASS and the reset line are
# missing, and the first missing line names Z4a
check t6_submit-z4-reset 1 "VERDICT t6_submit FAIL (13 of $SUB)" "$((SUB - 13)) 13 0" t6_submit "$D/t6_submit-z4-reset.uart"
if [ "$(bash "$V" t6_submit "$D/t6_submit-z4-reset.uart" 2>&1 | grep -m1 '^missing ' | grep -c 't6_submit: Z4a:')" = 1 ]; then
	echo "ok   t6_submit-z4-reset: the first missing line is Z4a"
else
	echo "FAIL t6_submit-z4-reset: the first missing line is not Z4a"
	fails=$((fails + 1))
fi
runs=$((runs + 1))
# the target's counts must add up to the pixels read
sed 's/other 0 of 4096 pixels/other 0 of 4095 pixels/' "$D/t6_submit-pass.uart" >"$WORK/pix.log"
check t6_submit-pixels 1 "VERDICT t6_submit FAIL (1 of $SUB)" "$((SUB - 1)) 0 1" t6_submit "$WORK/pix.log"
# a Z3 stop line is a differing line for t6_submit as for t6_fw
sed '/Z3: rc 0/a t6_submit: Z3: stop 5 refusal 5 detail 0x0000000000000000 acked 0x00000000' "$D/t6_submit-pass.uart" >"$WORK/stop.log"
check t6_submit-stop-line 1 "VERDICT t6_submit FAIL (1 of $SUB)" "$SUB 0 1" t6_submit "$WORK/stop.log"

# t6_display (step D2): the line count is the table's, read from the pass
# run (every line ok), not kept by hand here
DISP=$(bash "$V" t6_display "$D/t6_display-pass.uart" | grep -c '^ok ' || true)
check t6_display-pass 0 'VERDICT t6_display PASS' "$DISP 0 0" t6_display "$D/t6_display-pass.uart"
# the hooks: a pixel clock off by 7.7 %, a PHY PLL not locked, a controller
# clock monitor not locked each differ on that line only
sed 's/pixel clock 65002 kHz/pixel clock 60000 kHz/' "$D/t6_display-pass.uart" >"$WORK/pclk.log"
check t6_display-pclk 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/pclk.log"
sed 's/PHY0 GRF STATUS 0x0000000e/PHY0 GRF STATUS 0x00000006/' "$D/t6_display-pass.uart" >"$WORK/phy.log"
check t6_display-phy 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/phy.log"
sed 's/CMU_STATUS 0x00000255/CMU_STATUS 0x00000254/' "$D/t6_display-pass.uart" >"$WORK/cmu.log"
check t6_display-cmu 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/cmu.log"
# Esmart0 reading another address after the hold differs
sed '/^t6_display: VOP OVL_PORT_SEL/s/YRGB_MST 0x10012000/YRGB_MST 0x0ad17000/' "$D/t6_display-pass.uart" >"$WORK/mst.log"
check t6_display-mst 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/mst.log"
# the overlay selects written must read back: a port select that moved
# during the hold differs on the status line, a layer select on the shown
# line differs there
sed '/^t6_display: VOP OVL_PORT_SEL/s/OVL_PORT_SEL 0xa0547880/OVL_PORT_SEL 0xa0547873/' "$D/t6_display-pass.uart" >"$WORK/ops.log"
check t6_display-port-sel 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/ops.log"
sed '/^t6_display: shown:/s/OVL_LAYER_SEL 0x76513042/OVL_LAYER_SEL 0x76513240/' "$D/t6_display-pass.uart" >"$WORK/ols.log"
check t6_display-layer-sel 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/ols.log"
# an underflow in the frame check differs
sed 's/frame start 1, underflow 0,/frame start 1, underflow 1,/' "$D/t6_display-pass.uart" >"$WORK/under.log"
check t6_display-underflow 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/under.log"
# the adopt refused (VP0 in STANDBY): the five step lines, the hold, the
# three held lines and the seven status lines are missing; the adopt line
# and the FAIL line in place of PASS differ
sed -e '/^t6_display: take-over/,/^t6_display: HDMI0 VIDEO_MONITOR/d' \
	-e 's/^t6_display: PASS: .*/t6_display: FAIL: adopt stop 4/' "$D/t6_display-pass.uart" |
	sed '/^t6_display: FAIL: adopt/i t6_display: adopt rc -5 stop 4 (VP0 is in STANDBY: no picture to adopt), refusal 0, index 0, detail 0x8000000f' >"$WORK/standby.log"
check t6_display-standby 1 "VERDICT t6_display FAIL (18 of $DISP)" "$((DISP - 18)) 16 2" t6_display "$WORK/standby.log"

# t6_display on Linux's 1920x1080 (the adopt keeps it; the buffer is still
# the probe's 1024x768): the pass record with what differs in that mode,
# from the dump of 2026-11-23 (driver/docs/
# nanopc-t6-display-dump-linux-1920x1080.txt): HDMI0 alone in DSP_IF_EN,
# VP0's timing words, Linux's overlay selects and those the adopt writes,
# the vop_mmu DTE, the pixel clock of 2200 x 1125 at 60.006 Hz, the totals,
# the HPD word, the CMU counts and the monitor words. The 1024x768 pixel
# clock on it differs, and so do 1024x768's monitor words: the mode is taken
# from the "found VP0 timing" line only.
D1080=$WORK/t6_display-1920x1080-pass.log
sed -e 's/DSP_IF_EN 0x00040028/DSP_IF_EN 0x00000008/g' \
	-e 's/found VP0 timing 0x05400088 0x01280528 0x03260006 0x00230323 OVL_LAYER_SEL 0x76513240 OVL_PORT_SEL 0xa0547873/found VP0 timing 0x0898002c 0x00c00840 0x04650005 0x00290461 OVL_LAYER_SEL 0x76543210 OVL_PORT_SEL 0xa0507873/' \
	-e 's/DTE_ADDR 0x0ad17100 0x0ad17100/DTE_ADDR 0x02cad100 0x02cad100/' \
	-e 's/0x76513042/0x76543012/g' -e 's/0xa0547880/0xa0507880/g' \
	-e 's/pixel clock 65002 kHz/pixel clock 148514 kHz/' \
	-e 's/HTOTAL_HS_END 0x05400088 VTOTAL_VS_END 0x03260006/HTOTAL_HS_END 0x0898002c VTOTAL_VS_END 0x04650005/' \
	-e 's/SOC_STATUS1 0x001da240/SOC_STATUS1 0x001d6240/' \
	-e 's/VIDQPCLK_FREQ 16250 LINKQPCLK_FREQ 16249/VIDQPCLK_FREQ 35471 LINKQPCLK_FREQ 36863/' \
	-e 's/0x00880018 0x014000a0 0x05400400 0x00060003 0x0026001d 0x03260300/0x002c0058 0x01180094 0x08980780 0x00050004 0x002d0024 0x04650438/' \
	"$D/t6_display-pass.uart" >"$D1080"
if [ "$(diff "$D/t6_display-pass.uart" "$D1080" | grep -c '^>')" = 11 ]; then
	echo "ok   t6_display-1920x1080: 11 lines of the pass record changed"
else
	echo "FAIL t6_display-1920x1080: not 11 lines of the pass record changed (a sed above no longer matches)"
	fails=$((fails + 1))
fi
runs=$((runs + 1))
check t6_display-1920x1080-pass 0 'VERDICT t6_display PASS' "$DISP 0 0" t6_display "$D1080"
sed 's/pixel clock 148514 kHz/pixel clock 65002 kHz/' "$D1080" >"$WORK/pclk1080.log"
check t6_display-1920x1080-pclk 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/pclk1080.log"
sed 's/0x002c0058 0x01180094 0x08980780 0x00050004 0x002d0024 0x04650438/0x00880018 0x014000a0 0x05400400 0x00060003 0x0026001d 0x03260300/' "$D1080" >"$WORK/mon1080.log"
check t6_display-1920x1080-monitor 1 "VERDICT t6_display FAIL (1 of $DISP)" "$((DISP - 1)) 0 1" t6_display "$WORK/mon1080.log"
# a mode that is neither: judged against 1024x768, so the timing line, the
# status line's totals, the pixel clock and the monitor words differ
sed -e 's/found VP0 timing 0x0898002c/found VP0 timing 0x06720028/' "$D1080" >"$WORK/other.log"
check t6_display-other-mode 1 "VERDICT t6_display FAIL (4 of $DISP)" "$((DISP - 4)) 0 4" t6_display "$WORK/other.log"

# t6_cold (step D8): the line counts are the tables', read from the passing
# records (a run to the end, a run stopped after stage 4 as asked)
COLD=$(bash "$V" t6_cold "$D/t6_cold-pass.uart" | grep -c '^ok ' || true)
COLD4=$(bash "$V" t6_cold "$D/t6_cold-stop4.uart" | grep -c '^ok ' || true)
check t6_cold-pass 0 'VERDICT t6_cold PASS' "$COLD 0 0" t6_cold "$D/t6_cold-pass.uart"
check t6_cold-stop4 0 'VERDICT t6_cold PASS' "$COLD4 0 0" t6_cold "$D/t6_cold-stop4.uart"
# a stop after VOP: the picture is up, so the hold and the read-backs
# follow (their overlay selects checked for form: no report line names
# them), and STOPPED in place of PASS
sed -e 's/stop after 0 (run to the end)/stop after 7 (VOP)/' -e '/^t6 display: stage 8 VIDEO$/,/^t6_cold: chain rc/d' \
	-e 's/^t6_cold: PASS: .*/t6_cold: STOPPED: after stage 7 (VOP) as asked/' "$D/t6_cold-pass.uart" |
	sed '/^t6_cold: holding/i t6 display: stopped after stage 7 VOP as asked\nt6_cold: chain rc -140 in 380 ms: stage 7 (VOP), error 1 (stopped as asked), refusal 0, index 0, detail 0x00000000' >"$WORK/cold7.log"
check t6_cold-stop7 0 'VERDICT t6_cold PASS' "$((COLD - 8)) 0 0" t6_cold "$WORK/cold7.log"
# a hang in stage 4 (WDT0 resets the board): the record ends with that
# stage's line; the first line missing is stage 5's, and every line after it
sed '/^t6 display: stage 5 DCLK$/,/^t6: t6_cold done/d' "$D/t6_cold-pass.uart" >"$WORK/hang.log"
check t6_cold-hang 1 "VERDICT t6_cold FAIL (26 of $COLD)" "$((COLD - 26)) 26 0" t6_cold "$WORK/hang.log"
first=$(bash "$V" t6_cold "$WORK/hang.log" | grep -v '^ok ' | sed -n 1p || true)
runs=$((runs + 1))
if [[ $first == 'missing  expected "t6 display: stage 5 DCLK";'* ]]; then
	echo "ok   t6_cold-hang: the first line missing names stage 5"
else
	echo "FAIL t6_cold-hang: the first line not ok is not stage 5's: $first"
	fails=$((fails + 1))
fi
# the "ends" line names where a record without the chain's rc line
# stopped: inside stage 8 VIDEO (its line is the last), inside the reads
# before the chain, and inside stage 4 (the hang above)
# cold_ends <name> <log> <where> [probe]
cold_ends() {
	local got probe=${4:-t6_cold} rc='t6_cold: chain rc'
	[ "$probe" = t6_cold ] || rc='board: chain rc'
	got=$(bash "$V" "$probe" "$2" | grep '^ends ' || true)
	runs=$((runs + 1))
	if [ "$got" = "ends     no \"$rc\" line: the run stopped inside $3" ]; then
		echo "ok   $1: $got"
	else
		echo "FAIL $1: '$got' (want the run stopped inside $3)"
		fails=$((fails + 1))
	fi
}
cold_ends t6_cold-ends-in-phy "$WORK/hang.log" 'stage 4 PHY'
sed '/^t6 display: up, /,/^t6: t6_cold done/d' "$D/t6_cold-pass.uart" >"$WORK/hang8.log"
check t6_cold-hang-video 1 "VERDICT t6_cold FAIL (22 of $COLD)" "$((COLD - 22)) 22 0" t6_cold "$WORK/hang8.log"
cold_ends t6_cold-ends-in-video "$WORK/hang8.log" 'stage 8 VIDEO'
sed '/^t6 display: before: hdmi-tx0 /,/^t6: t6_cold done/d' "$D/t6_cold-pass.uart" >"$WORK/hangbefore.log"
check t6_cold-hang-before 1 "VERDICT t6_cold FAIL (34 of $COLD)" "$((COLD - 34)) 34 0" t6_cold "$WORK/hangbefore.log"
cold_ends t6_cold-ends-in-before "$WORK/hangbefore.log" \
	'the reads before the chain, after "t6 display: before: vop not readable: Gated(Clk { part: Cru, gate: Gate { con: 52, bit: 9 } })"'
# a record that has its rc line has no "ends" line
runs=$((runs + 1))
if bash "$V" t6_cold "$D/t6_cold-pass.uart" | grep -q '^ends '; then
	echo "FAIL t6_cold-pass: an ends line in a record that has the chain's rc"
	fails=$((fails + 1))
else
	echo "ok   t6_cold-pass: no ends line"
fi
# stage 4 failed: the chain's rc and the FAIL line differ, and so does the
# failure line, which no record may have (it counts as a third, beyond the
# table's lines); the stages after it, the report, the hold and the
# read-backs are missing
sed -e '/^t6 display: stage 5 DCLK$/,/^t6_cold: PASS: /d' "$D/t6_cold-pass.uart" |
	sed '/^t6: t6_cold done/i t6 display: stage 4 PHY failed: Phy(Refused { why: LockTimeout, found: 6 })\nt6_cold: chain rc -5 in 170 ms: stage 4 (PHY), error 3 (the PHY), refusal 4, index 0, detail 0x00000006\nt6_cold: FAIL: stage 4 (PHY) failed' >"$WORK/phyfail.log"
check t6_cold-phy-failed 1 "VERDICT t6_cold FAIL (26 of $COLD)" "$((COLD - 25)) 23 3" t6_cold "$WORK/phyfail.log"
# a stage line lost in the middle is missing on its own
sed '/^t6 display: stage 3 GRF$/d' "$D/t6_cold-pass.uart" >"$WORK/nogrf.log"
check t6_cold-no-stage-3 1 "VERDICT t6_cold FAIL (1 of $COLD)" "$((COLD - 1)) 1 0" t6_cold "$WORK/nogrf.log"
# each pinned or derived value differs on its own line: another controller,
# a pixel clock off by 7.7 %, Esmart0 on another address after the hold, an
# overlay select that moved during the hold
sed 's/CORE_ID 0x48515458/CORE_ID 0x48515459/' "$D/t6_cold-pass.uart" >"$WORK/core.log"
check t6_cold-core-id 1 "VERDICT t6_cold FAIL (1 of $COLD)" "$((COLD - 1)) 0 1" t6_cold "$WORK/core.log"
sed 's/pixel clock 65002 kHz/pixel clock 60000 kHz/' "$D/t6_cold-pass.uart" >"$WORK/cpclk.log"
check t6_cold-pclk 1 "VERDICT t6_cold FAIL (1 of $COLD)" "$((COLD - 1)) 0 1" t6_cold "$WORK/cpclk.log"
sed '/^t6_cold: VOP OVL_PORT_SEL/s/YRGB_MST 0x1001d000/YRGB_MST 0x1001e000/' "$D/t6_cold-pass.uart" >"$WORK/cmst.log"
check t6_cold-mst 1 "VERDICT t6_cold FAIL (1 of $COLD)" "$((COLD - 1)) 0 1" t6_cold "$WORK/cmst.log"
sed '/^t6_cold: VOP OVL_PORT_SEL/s/OVL_PORT_SEL 0x80007880/OVL_PORT_SEL 0x80007873/' "$D/t6_cold-pass.uart" >"$WORK/cops.log"
check t6_cold-port-sel 1 "VERDICT t6_cold FAIL (1 of $COLD)" "$((COLD - 1)) 0 1" t6_cold "$WORK/cops.log"
# the mode (T6_COLD_MODE): a build with the option prints its mode after the
# options line, one line more than the records above, which are of a build
# before it. The 1920x1080 record is the 1024x768 one with what the mode
# changes: the size on the "up" line, VP0's totals, the pixel clock and the
# monitor words; the frame buffer and REGION0_VIR stay 1024x768.
sed '/^t6_cold: options: /a t6_cold: mode 1024x768' "$D/t6_cold-pass.uart" >"$WORK/cmode.log"
check t6_cold-mode-line 0 'VERDICT t6_cold PASS' "$((COLD + 1)) 0 0" t6_cold "$WORK/cmode.log"
sed -e '/^t6_cold: options: /a t6_cold: mode 1920x1080' -e 's/^t6 display: up, 1024x768 at /t6 display: up, 1920x1080 at /' \
	-e 's/pixel clock 65002 kHz/pixel clock 148504 kHz/' \
	-e 's/HTOTAL_HS_END 0x05400088 VTOTAL_VS_END 0x03260006/HTOTAL_HS_END 0x0898002c VTOTAL_VS_END 0x04650005/' \
	-e 's/0x00880018 0x014000a0 0x05400400 0x00060003 0x0026001d 0x03260300/0x002c0058 0x01180094 0x08980780 0x00050004 0x002d0024 0x04650438/' \
	"$D/t6_cold-pass.uart" >"$WORK/c1080.log"
check t6_cold-1920x1080 0 'VERDICT t6_cold PASS' "$((COLD + 1)) 0 0" t6_cold "$WORK/c1080.log"
# each of the mode's values differs on its own line
sed 's/^t6 display: up, 1920x1080 at /t6 display: up, 1024x768 at /' "$WORK/c1080.log" >"$WORK/c1080up.log"
check t6_cold-1920x1080-up 1 "VERDICT t6_cold FAIL (1 of $((COLD + 1)))" "$COLD 0 1" t6_cold "$WORK/c1080up.log"
sed 's/pixel clock 148504 kHz/pixel clock 65002 kHz/' "$WORK/c1080.log" >"$WORK/c1080pclk.log"
check t6_cold-1920x1080-pclk 1 "VERDICT t6_cold FAIL (1 of $((COLD + 1)))" "$COLD 0 1" t6_cold "$WORK/c1080pclk.log"
# a build that says 1920x1080 and programs 1024x768: the "up" line, the
# totals, the pixel clock and the monitor words differ
sed '/^t6_cold: options: /a t6_cold: mode 1920x1080' "$D/t6_cold-pass.uart" >"$WORK/csays.log"
check t6_cold-says-1920x1080 1 "VERDICT t6_cold FAIL (4 of $((COLD + 1)))" "$((COLD - 3)) 0 4" t6_cold "$WORK/csays.log"
# a mode line naming neither mode differs (the rest is judged as 1024x768)
sed '/^t6_cold: options: /a t6_cold: mode 1280x720' "$D/t6_cold-pass.uart" >"$WORK/cother.log"
check t6_cold-other-mode 1 "VERDICT t6_cold FAIL (1 of $((COLD + 1)))" "$COLD 0 1" t6_cold "$WORK/cother.log"

# triangle (examples/triangle on the board layer): the first show by the
# cold chain (the kexec route) and by the adopt (kexec --crash)
TRI=$(bash "$V" triangle "$D/triangle-cold-pass.uart" | grep -c '^ok ' || true)
TRIA=$(bash "$V" triangle "$D/triangle-adopt-pass.uart" | grep -c '^ok ' || true)
check triangle-cold-pass 0 'VERDICT triangle PASS' "$TRI 0 0" triangle "$D/triangle-cold-pass.uart"
check triangle-adopt-pass 0 'VERDICT triangle PASS' "$TRIA 0 0" triangle "$D/triangle-adopt-pass.uart"
# the progress records are not judged
sed '/^yk-record: 594b05/d' "$D/triangle-cold-pass.uart" >"$WORK/tnoprog.log"
check triangle-no-progress-records 0 'VERDICT triangle PASS' "$TRI 0 0" triangle "$WORK/tnoprog.log"
# the mode is the one the board's first-show line names: a 1024x768 build
# passes, and a chain that brought up another mode than the board says
# differs on the chain's "up" line
sed -e 's/cold start at 1920x1080, rc 0/cold start at 1024x768, rc 0/' -e 's/^t6 display: up, 1920x1080 at /t6 display: up, 1024x768 at /' \
	"$D/triangle-cold-pass.uart" >"$WORK/t768.log"
check triangle-1024x768 0 'VERDICT triangle PASS' "$TRI 0 0" triangle "$WORK/t768.log"
sed 's/^t6 display: up, 1920x1080 at /t6 display: up, 1024x768 at /' "$D/triangle-cold-pass.uart" >"$WORK/tup.log"
check triangle-mode-up 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 0 1" triangle "$WORK/tup.log"
# the chain shows the window: another address in YRGB_MST differs
sed '/^t6 display: VIDEO: /s/YRGB_MST 0x[0-9a-f]*/YRGB_MST 0x19440000/' "$D/triangle-cold-pass.uart" >"$WORK/tmst.log"
check triangle-cold-mst 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 0 1" triangle "$WORK/tmst.log"
# the cold start is taken after the adopt's stops 1, 2 and 4 only, and the
# board's adopt line names the same stop
sed 's/no picture to adopt (stop 1)/no picture to adopt (stop 6)/' "$D/triangle-cold-pass.uart" >"$WORK/tstop.log"
check triangle-stop-6 1 "VERDICT triangle FAIL (2 of $TRI)" "$((TRI - 2)) 0 2" triangle "$WORK/tstop.log"
sed -e 's/no picture to adopt (stop 1)/no picture to adopt (stop 4)/' \
	-e 's/^board: adopt rc -19 stop 1 (PD_VOP is not powered or not out of idle)/board: adopt rc -19 stop 4 (VP0 is in STANDBY: no picture to adopt)/' \
	"$D/triangle-cold-pass.uart" >"$WORK/tstop4.log"
check triangle-stop-4 0 'VERDICT triangle PASS' "$TRI 0 0" triangle "$WORK/tstop4.log"
# a hang inside stage 4: every later line is missing, and the "ends" line
# names the stage
sed '/^t6 display: stage 5 DCLK$/,/^t6: board: run over/d' "$D/triangle-cold-pass.uart" >"$WORK/thang.log"
check triangle-hang 1 "VERDICT triangle FAIL (33 of $TRI)" "$((TRI - 33)) 33 0" triangle "$WORK/thang.log"
cold_ends triangle-ends-in-phy "$WORK/thang.log" 'stage 4 PHY' triangle
# a stage that fails: its failure line, the rc of the first show and of the
# chain, and the sample's line that the window is not on the screen
sed '/^t6 display: stage 5 DCLK$/,/^t6 display: VIDEO: /d' "$D/triangle-cold-pass.uart" |
	sed -e '/^board: found PD_VOP /i t6 display: stage 4 PHY failed: Phy(Refused { why: LockTimeout, found: 6 })' \
		-e 's/^board: first show: no picture was left running: cold start at 1920x1080, rc 0$/board: first show: no picture was left running: cold start at 1920x1080, rc -5/' \
		-e 's/^board: chain rc 0 in [0-9]* ms: stage 0 (none), error 0 (none), refusal 0, index 0, detail 0x00000000$/board: chain rc -5 in 170 ms: stage 4 (PHY), error 3 (the PHY), refusal 4, index 0, detail 0x00000006\ntriangle: board_show = -5 (the window is not on the screen)/' \
		>"$WORK/tphy.log"
check triangle-phy-failed 1 "VERDICT triangle FAIL (16 of $TRI)" "$((TRI - 14)) 12 4" triangle "$WORK/tphy.log"
# the check: a wrong pixel fails its count line, the check line and the
# sample's last line, and the hold is the failed run's
sed -e 's/inside wrong 0 /inside wrong 12 /' -e 's/^triangle: check PASS .*/triangle: check FAIL: inside pixels wrong (code 1)/' \
	-e 's/^triangle: PASS$/triangle: FAIL rc=-74/' -e 's/^board: holding 60 s/board: holding 40 s/' \
	"$D/triangle-cold-pass.uart" >"$WORK/tcheck.log"
check triangle-check-fail 1 "VERDICT triangle FAIL (4 of $TRI)" "$((TRI - 4)) 0 4" triangle "$WORK/tcheck.log"
# the three counts add up to the window
sed 's/= 194526 inside/= 194527 inside/' "$D/triangle-cold-pass.uart" >"$WORK/tsum.log"
check triangle-pixel-sum 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 0 1" triangle "$WORK/tsum.log"
sed 's/guard rows: 0 of 8192/guard rows: 3 of 8192/' "$D/triangle-cold-pass.uart" >"$WORK/tguard.log"
check triangle-guard-rows 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 0 1" triangle "$WORK/tguard.log"
# the PICTURE record carries the picture line's words
sed 's/^yk-record: 594b0407 ff403f80 /yk-record: 594b0407 ff403f81 /' "$D/triangle-cold-pass.uart" >"$WORK/trec.log"
check triangle-record-words 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 0 1" triangle "$WORK/trec.log"
sed '/^yk-record: 594b04/d' "$D/triangle-cold-pass.uart" >"$WORK/tnorec.log"
check triangle-no-picture-record 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 1 0" triangle "$WORK/tnorec.log"
sed 's/^yakogl: draws refused 0$/yakogl: draws refused 2/' "$D/triangle-cold-pass.uart" >"$WORK/tref.log"
check triangle-draws-refused 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 0 1" triangle "$WORK/tref.log"
sed 's/^board: the record screen stays 20 s$/board: no record screen: the display is not up (rc -19)/' "$D/triangle-cold-pass.uart" >"$WORK/tscr.log"
check triangle-no-record-screen 1 "VERDICT triangle FAIL (1 of $TRI)" "$((TRI - 1)) 0 1" triangle "$WORK/tscr.log"
# a fatal error line anywhere fails the record
sed '/^triangle: frame /a board: fatal error 4: halted, WDT0 resets the SoC within 22369 ms' "$D/triangle-cold-pass.uart" >"$WORK/tfatal.log"
check triangle-fatal 1 "VERDICT triangle FAIL (1 of $TRI)" "$TRI 0 1" triangle "$WORK/tfatal.log"
# the adopt route: the window's address on Esmart0 and in the read-back,
# a vop_mmu that did not stop, the rc of the first show
sed '/^board: shown: /s/YRGB_MST 0x1943f000/YRGB_MST 0x19440000/' "$D/triangle-adopt-pass.uart" >"$WORK/tamst.log"
check triangle-adopt-mst 1 "VERDICT triangle FAIL (1 of $TRIA)" "$((TRIA - 1)) 0 1" triangle "$WORK/tamst.log"
sed '/^board: vop_mmu: /s/stall time-out 0/stall time-out 1/' "$D/triangle-adopt-pass.uart" >"$WORK/tastall.log"
check triangle-adopt-stall 1 "VERDICT triangle FAIL (1 of $TRIA)" "$((TRIA - 1)) 0 1" triangle "$WORK/tastall.log"
sed 's/^board: first show: a picture was left running: adopt, rc 0$/board: first show: a picture was left running: adopt, rc -5/' "$D/triangle-adopt-pass.uart" >"$WORK/tarc.log"
check triangle-adopt-rc 1 "VERDICT triangle FAIL (1 of $TRIA)" "$((TRIA - 1)) 0 1" triangle "$WORK/tarc.log"
# no "ends" line for a record that has the chain's rc, nor on the adopt route
for f in "$D/triangle-cold-pass.uart" "$D/triangle-adopt-pass.uart"; do
	runs=$((runs + 1))
	if bash "$V" triangle "$f" | grep -q '^ends '; then
		echo "FAIL ${f##*/}: an ends line in a record that did not stop inside the chain"
		fails=$((fails + 1))
	else
		echo "ok   ${f##*/}: no ends line"
	fi
done

# Z1's record in the stop fixtures is what t6_z1.c prints (host cc, the
# probes' printk as printf): a fixture cannot drift from the code
P=$(cd "$HERE/../../.." && pwd)/tools/probes/nanopc_t6/common
if ! command -v cc >/dev/null 2>&1; then
	echo "FAIL t6_z1.c host build: no cc on PATH"
	fails=$((fails + 1))
elif ! cc -std=c11 -Wall -Wextra -Werror -I"$P/test" -I"$P" -I"$HERE/../../rust/include" \
	"$P/t6_z1.c" "$P/test/t6_z1_test.c" -o "$WORK/t6_z1_test"; then
	echo "FAIL t6_z1.c host build"
	fails=$((fails + 1))
else
	for c in t6_mmu:pd:t6_mmu-z1stop t6_fw:pll:t6_fw-z1fail t6_submit:pd:t6_submit-z3-stop \
		t6_fw:mask:t6_fw-z1-takeover-mask t6_fw:hard:t6_fw-z1-takeover-hard \
		t6_gpu:crash:t6_gpu-crash-pass; do
		IFS=: read -r probe stop fx <<<"$c"
		runs=$((runs + 1))
		"$WORK/t6_z1_test" "$probe" "$stop" >"$WORK/z1.want"
		# a stop record ends with its stop line, a run-through with the L2 line
		sed -n "/^$probe: PD_GPU at hand-over/,/^$probe: \(Z1 stopped\|L2 mask\)/p" "$D/$fx.uart" >"$WORK/z1.got"
		if [ -s "$WORK/z1.want" ] && cmp -s "$WORK/z1.want" "$WORK/z1.got"; then
			echo "ok   $fx: Z1's record is t6_z1.c's ($(wc -l <"$WORK/z1.want") lines)"
		else
			echo "FAIL $fx: Z1's record is not what t6_z1.c prints"
			diff "$WORK/z1.want" "$WORK/z1.got" | sed 's/^/     /' || true
			fails=$((fails + 1))
		fi
	done
fi

# usage
rc=0
bash "$V" t6_nonesuch "$D/t6_mmu-pass.uart" >/dev/null 2>&1 || rc=$?
runs=$((runs + 1))
if [ "$rc" = 2 ]; then echo "ok   unknown probe: rc 2"; else echo "FAIL unknown probe: rc $rc (want 2)"; fails=$((fails + 1)); fi
rc=0
T6_ROUTE=tftp bash "$V" t6_hello "$D/t6_hello-pass.uart" >/dev/null 2>&1 || rc=$?
runs=$((runs + 1))
if [ "$rc" = 2 ]; then echo "ok   unknown route: rc 2"; else echo "FAIL unknown route: rc $rc (want 2)"; fails=$((fails + 1)); fi

if [ "$fails" -eq 0 ]; then
	echo "t6-verdict-test: $runs ok"
	exit 0
fi
echo "t6-verdict-test: $fails of $runs FAILED"
exit 1
