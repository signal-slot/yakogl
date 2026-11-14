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

# the line counts: boot 10 + reset 1 + each probe's own
MMU=27 HELLO=25 GPU=32

check t6_mmu-pass 0 'VERDICT t6_mmu PASS' "$MMU 0 0" t6_mmu "$D/t6_mmu-pass.uart"
check t6_mmu-missing 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 1 0" t6_mmu "$D/t6_mmu-missing.uart"
check t6_mmu-differs 1 "VERDICT t6_mmu FAIL (1 of $MMU)" "$((MMU - 1)) 0 1" t6_mmu "$D/t6_mmu-differs.uart"

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
check t6_gpu-over-mmu 1 "VERDICT t6_gpu FAIL ($((GPU - 10)) of $GPU)" "10 $((GPU - 10)) 0" t6_gpu "$D/t6_mmu-pass.uart"

check t6_hello-pass 0 'VERDICT t6_hello PASS' "$HELLO 0 0" t6_hello "$D/t6_hello-pass.uart"
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

# t6_fw: boot 10 + build, WDT0, image, 7 sections, pool, Z3's rc line and 7
# record lines, PASS, reset = 31
FW=31
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

# t6_submit: t6_fw's 31 lines (the Z3 record) + 11 Z4 lines = 42. The Z4 lines
# are the model's records (board::nanopc_t6_submit's
# the_fixtures_z4_lines_are_the_model_s_records); a stopped run prints its
# record too and is judged by its status fields.
SUB=42
check t6_submit-pass 0 'VERDICT t6_submit PASS' "$SUB 0 0" t6_submit "$D/t6_submit-pass.uart"
# wait timeout (step 14, refusal 19): Z4a, Z4e (extract and seqno 0), Z4j and
# Z4k (the drain did not run: extract 0, ack 0) and the FAIL line differ
check t6_submit-wait-timeout 1 "VERDICT t6_submit FAIL (5 of $SUB)" "$((SUB - 5)) 0 5" t6_submit "$D/t6_submit-wait-timeout.uart"
# Z3 stopped (Z1 refused, stop 5): the rc line, Z4a..Z4e, Z4j and Z4k (zeros),
# the FAIL line and the stop line differ, the 7 record lines are missing;
# Z1's record follows the stop line (8 ok)
check t6_submit-z3-stop 1 "VERDICT t6_submit FAIL (17 of $((SUB + Z1)))" "$((26 + Z1)) 7 10" t6_submit "$D/t6_submit-z3-stop.uart"
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

if [ "$fails" -eq 0 ]; then
	echo "t6-verdict-test: $runs ok"
	exit 0
fi
echo "t6-verdict-test: $fails of $runs FAILED"
exit 1
