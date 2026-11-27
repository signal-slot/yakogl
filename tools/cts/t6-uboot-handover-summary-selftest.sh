#!/bin/bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
#
#   tools/cts/t6-uboot-handover-summary-selftest.sh
#
# Host-only check of tools/cts/t6-uboot-handover-summary.py on synthetic
# records built in $TMPDIR: a prompt-reads answers.txt and a t6_gpu go
# record (image.log, verdict.txt), in the line formats of prompt-reads.py and
# of tools/probes/nanopc_t6/common/t6_z1.c. Covers keep, move and refuse
# (stops 9 and 10) of gpu_clock_set, the supply on and off, the WDT0 gates
# open and closed, PD_GPU on and off at the prompt, values not read, and the
# refusal of an rfc2217 URL, a /dev/tty path, a MAC and a 100.64/10 address.
# Exit 0: every check passed. No board.

set -u
HERE=$(cd "$(dirname "$0")" && pwd)
TOOL="$HERE/t6-uboot-handover-summary.py"
W=$(mktemp -d "${TMPDIR:-/tmp}/t6-handover-summary.XXXXXX") || exit 1
trap 'rm -rf "$W"' EXIT
fails=0 n=0
# 100.64/10 addresses are built from parts: the redaction scan flags them whole
HI=100
IP1="$HI.64.1.2" IP2="$HI.101.102.103" IP3="$HI.64.3.4"

ok() { n=$((n + 1)); echo "ok: $1"; }
bad() { n=$((n + 1)); fails=$((fails + 1)); echo "FAIL: $1"; }

OUT="" ERR="" RC=0
run() { OUT=$(python3 -I "$TOOL" "$@" 2>"$W/err"); RC=$?; ERR=$(cat "$W/err"); }
has() { # has LABEL TEXT: OUT contains the fixed string
	if grep -qF -- "$2" <<<"$OUT"; then ok "$1"; else bad "$1 (no '$2')"; fi
}
hasnt() {
	if grep -qF -- "$2" <<<"$OUT"; then bad "$1 ('$2' present)"; else ok "$1"; fi
}
rc_is() { if [ "$RC" = "$2" ]; then ok "$1"; else bad "$1 (exit $RC, want $2)"; fi; }

# reads GATES PD DIR: a prompt-reads record. GATES is the CLKGATE_CON(15)
# word, PD "on" or "off".
reads() {
	local gates=$1 pd=$2 d=$3 gate=0000ffff rep=fad40001
	if [ "$pd" = on ]; then gate=0000fffe; rep=fad40003; fi
	mkdir -p "$d"
	{
		echo "## version"; echo "U-Boot 2026.07-armbian"
		echo "## printenv"; echo "ethaddr=aa:bb:cc:dd:ee:ff"; echo "ipaddr=$IP1"
		echo "## md.l 0xfd8d814c 1"; echo "fd8d814c: $gate    ...."
		echo "## md.l 0xfd8d810c 1"; echo "fd8d810c: 0000e3cf    ...."
		echo "## md.l 0xfd8d8118 1"; echo "fd8d8118: 000a9fff    ...."
		echo "## md.l 0xfd8d8120 1"; echo "fd8d8120: 000a9fff    ...."
		echo "## md.l 0xfd8d8290 1"; echo "fd8d8290: $rep    ...."
		echo "## md.l 0xfd7c0578 1"; echo "fd7c0578: 00001f24    ...."
		echo "## md.l 0xfd7c057c 1"; echo "fd7c057c: 00000000    ...."
		echo "## md.l 0xfd7c0908 1"; echo "fd7c0908: 00000000    ...."
		echo "## md.l 0xfd7c0280 1"; echo "fd7c0280: 00000155    ...."
		echo "## md.l 0xfd7c01a0 7"
		echo "fd7c01a0: 000000fa 00000042 00000000 00000000    ...."
		echo "fd7c01b0: 00000000 00000000 00009c00    ...."
		echo "## md.l 0xfd7c01c0 7"
		echo "fd7c01c0: 000000fa 00000042 00000000 00000000    ...."
		echo "fd7c01d0: 00000000 00000000 00009c00    ...."
		echo "## md.l 0xfd7c083c 1"; echo "fd7c083c: $gates    ...."
		echo "## md.l 0xfd7c0c04 1"; echo "fd7c0c04: 00000003    ...."
		echo "## md.l 0xfd7c0c10 1"; echo "fd7c0c10: 0000000c    ...."
		if [ "$gates" = 00000000 ]; then
			echo "# WDT0's gates read open: WDT0's control and identity words are read"
			echo "## md.l 0xfeaf0000 2"; echo "feaf0000: 00000008 0000000e    ...."
			echo "## md.l 0xfeaf00f4 3"; echo "feaf00f4: 10000ac0 3131302a 44570120    ...."
		else
			echo "# WDT0's gates do not read open: WDT0 is not read"
		fi
		echo "## reset"
	} >"$d/answers.txt"
}

# go KIND DIR: a t6_gpu go record (the hand-over lines are t6_z1.c's, which t6_gpu
# links; t6_cold prints none of them). KIND: keep, move, supply-on (a move with
# the supply on at hand-over), refuse9, refuse10, bare (no hand-over lines).
go() {
	local kind=$1 d=$2 supply=off clk="" en="" stop=""
	mkdir -p "$d"
	case $kind in
	keep)
		supply=on
		clk="kept 1, CPLL 1500000000 Hz, wrote CLKSEL_CON(158) 0x00000000 (159) 0x00000000, now 0x00001f24 0x00000000: clk_gpu_src 300000000 Hz"
		en="not sent; POWER_EN0 0x0f BUCK1_ON_VSEL 0x1c after, waited 0 us" ;;
	move)
		clk="kept 0, CPLL 1500000000 Hz, wrote CLKSEL_CON(158) 0x00001f24 (159) 0x00000000, now 0x00001f24 0x00000000: clk_gpu_src 300000000 Hz"
		en="frame sent; POWER_EN0 0x0f BUCK1_ON_VSEL 0x1c after, waited 412 us" ;;
	supply-on)
		supply=on
		clk="kept 0, CPLL 1500000000 Hz, wrote CLKSEL_CON(158) 0x00001f24 (159) 0x00000003, now 0x00001f24 0x00000000: clk_gpu_src 300000000 Hz"
		en="not sent; POWER_EN0 0x0f BUCK1_ON_VSEL 0x1c after, waited 0 us" ;;
	refuse9)
		clk="kept 0, CPLL 1188000000 Hz, wrote CLKSEL_CON(158) 0x00000000 (159) 0x00000000, now 0x00001f24 0x00000000: clk_gpu_src 0 Hz"
		en="not sent; POWER_EN0 0x0e BUCK1_ON_VSEL 0x1c after, waited 0 us"
		stop="t6_gpu: Z1 stopped: stop 9 (the GPU clock is on no OPP rate and the CPLL is not 1500 MHz (nothing written)), refusal 0, detail 0x0000000046e3f0e0" ;;
	refuse10)
		clk="kept 0, CPLL 1500000000 Hz, wrote CLKSEL_CON(158) 0x00001f24 (159) 0x00000000, now 0x00001f04 0x00000000: clk_gpu_src 0 Hz"
		en="not sent; POWER_EN0 0x0e BUCK1_ON_VSEL 0x1c after, waited 0 us"
		stop="t6_gpu: Z1 stopped: stop 10 (the clock set did not read back), refusal 158, detail 0x0000000000001f04" ;;
	esac
	{
		echo "T6:A EL2 HCR_EL2 0x0000000080000020 -> 0x0000000080000000"
		echo "T6:A3 WDT0 arm rc 0 CR 0x00000009 period_ms 22369"
		echo "t6: WDT0 armed: CR 0x00000008 -> 0x00000009 (was stopped), TORR 0x00000000 -> 0x0000000d, CCVR 0x1ffffff3, PARAM_1 0x10000ac0, version 0x3131302a, flags 0x0: period 22369 ms (TOP 13), fed until 60 s of uptime, alive window 5 s"
		echo "*** Booting Zephyr OS build v4.2.0 ***"
		if [ "$kind" != bare ]; then
			echo "t6_gpu: PD_GPU at hand-over: PWR_GATE 0x0000ffff STATUS 0x00000000 BUS_IDLE_REQ 0x0000e3cf ACK 0x000a9fff ST 0x000a9fff REPAIR_STATUS 0xfad40001: off or idle"
			echo "t6_gpu: CRU at hand-over: CLKSEL_CON(158) 0x00001f24 CLKSEL_CON(159) 0x00000000 CLKGATE_CON(66) 0x00000000 MODE_CON0 0x00000155 PLL_CON 0x000000fa 0x00000042 0x00000000: clk_gpu_src 300000000 Hz"
			if [ "$supply" = on ]; then
				echo "t6_gpu: RK806 at hand-over: POWER_EN0 0x0f POWER_SLP_EN0 0xe0 BUCK1_CONFIG 0xa4 BUCK1_ON_VSEL 0x1c: vdd_gpu_s0 on, 675000 uV"
			else
				echo "t6_gpu: RK806 at hand-over: POWER_EN0 0x0e POWER_SLP_EN0 0xe0 BUCK1_CONFIG 0xa4 BUCK1_ON_VSEL 0x1c: vdd_gpu_s0 off, 675000 uV"
			fi
			echo "t6_gpu: clock set: $clk"
			echo "t6_gpu: vdd_gpu_s0 enable: $en"
			if [ -n "$stop" ]; then
				echo "$stop"
			else
				echo "t6_gpu: GPU PLL on: PLL_CON6 0x00009c00 at hand-over, was on 1, waited 0 us, PLL_CON6 0x00009c00"
				echo "t6_gpu: PD_GPU on: gates opened 0xf, was on 0, waited 3/2/2 us"
				echo "t6_gpu: PD_GPU after: PWR_GATE 0x0000fffe STATUS 0x00000000 BUS_IDLE_REQ 0x0000e3ce ACK 0x000a9ffe ST 0x000a9ffe REPAIR_STATUS 0xfad40003: on"
			fi
		fi
	} >"$d/image.log"
	if [ "$kind" = bare ]; then
		rm -f "$d/verdict.txt"
	elif [ -n "$stop" ]; then
		echo "VERDICT t6_gpu FAIL" >"$d/verdict.txt"
	else
		echo "VERDICT t6_gpu PASS" >"$d/verdict.txt"
	fi
}

R="$W/20261125-1000-reads-0123abcd"; reads 00000000 off "$R"
RC1="$W/20261125-1010-t6_gpu-keep-0123abcd"

# 1. keep, supply on
go keep "$RC1"; run "$R" "$RC1"
rc_is "keep: exit 0" 0
has "keep: names the choice" "gpu_clock_set took: keep"
has "keep: supply on" "vdd_gpu_s0 on, 675000 uV"
has "keep: enable not sent" "vdd_gpu_s0 enable: not sent"
has "keep: both record names" "records: prompt reads 20261125-1000-reads-0123abcd; go 20261125-1010-t6_gpu-keep-0123abcd"
hasnt "keep: no full path echoed" "$W"
has "keep: verdict" "verdict: VERDICT t6_gpu PASS"
has "reads: WDT0 gates open" "PCLK open, TCLK open -> open"
has "reads: WDT0 CR decoded" "WDT_CR 0x00000008 (WDT_EN 0, RMOD 0, RPL 2), WDT_TORR 0x0000000e (TOP 14)"
has "reads: WDT0 identity" "COMP_VERSION 0x3131302a"
has "reads: GLB_RST_CON raw" "0xfd7c0c10: 0x0000000c"
has "reads: GLB_RST_ST raw" "0xfd7c0c04: 0x00000003"
has "reads: CLKGATE_CON(15) word" "0xfd7c083c: 0x00000000"
has "reads: PD_GPU off" "PD_GPU at the prompt: off"
has "reads: QoS not read" "GPU QoS words: not read"
has "reads: CPLL words over two md lines" "000000fa 00000042 00000000 00000000 00000000 00000000 00009c00"

# 2. move, supply off
RC2="$W/20261125-1020-t6_gpu-move-0123abcd"; go move "$RC2"; run "$R" "$RC2"
rc_is "move: exit 0" 0
has "move: names the choice" "gpu_clock_set took: move: CLKSEL_CON(158) written, both words read back"
has "move: words written and read back" "words written: CLKSEL_CON(158) 0x00001f24, (159) 0x00000000; read back: 0x00001f24 0x00000000"
has "supply off: read" "vdd_gpu_s0 off, 675000 uV"
has "supply off: enable sent" "vdd_gpu_s0 enable: frame sent; POWER_EN0 0x0f BUCK1_ON_VSEL 0x1c after, waited 412 us"
has "move: PD_GPU after" "PD_GPU after: PWR_GATE 0x0000fffe"

# 2b. move with the supply on and a (159) write
RC2b="$W/20261125-1025-t6_gpu-supplyon-0123abcd"; go supply-on "$RC2b"; run "$R" "$RC2b"
has "move, supply on: both words written" "written and (159), both words read back"
has "supply on: read" "vdd_gpu_s0 on, 675000 uV"

# 3. refuse, stop 9 (nothing written)
RC3="$W/20261125-1030-t6_gpu-refuse9-0123abcd"; go refuse9 "$RC3"; run "$R" "$RC3"
rc_is "refuse 9: exit 0" 0
has "refuse 9: names the choice" "gpu_clock_set took: refuse: NoTableRate, nothing written"
has "refuse 9: the CPLL rate" "CPLL 1188000000 Hz;"
has "refuse 9: stop line" "image stop: stop 9"
has "refuse 9: verdict FAIL" "verdict: VERDICT t6_gpu FAIL"
has "refuse 9: PD_GPU after not read" "PD_GPU after: not read"

# 4. refuse, stop 10 (written, did not read back)
RC4="$W/20261125-1040-t6_gpu-refuse10-0123abcd"; go refuse10 "$RC4"; run "$R" "$RC4"
has "refuse 10: names the choice" "gpu_clock_set took: refuse: the clock set did not read back"
has "refuse 10: written and read back" "written: CLKSEL_CON(158) 0x00001f24, (159) 0x00000000; read back: 0x00001f04 0x00000000"

# 5. PD_GPU on and gates closed at the prompt
R2="$W/20261125-1050-reads-0123abcd"; reads 00000003 on "$R2"; run "$R2" "$RC1"
has "reads: PD_GPU on" "PD_GPU at the prompt: on"
has "reads: QoS words listed" "qos@0xfdf35000: not read"
has "reads: gates closed" "PCLK gated, TCLK gated -> closed"
has "reads: WDT0 not read" "WDT_CR / WDT_TORR: not read (gates closed: WDT0 is not read)"
has "reads: WDT0 identity not read" "COMP_PARAM_1 / COMP_VERSION / COMP_TYPE: not read"

# 6. values missing: a bare image.log, an answers.txt without md answers
RB="$W/20261125-1060-t6_gpu-bare-0123abcd"; go bare "$RB"
mkdir -p "$W/emptyreads"; printf '## version\nU-Boot\n' >"$W/emptyreads/answers.txt"
run "$W/emptyreads" "$RB"
rc_is "missing values: exit 0" 0
has "missing: clock not read" "gpu_clock_set: not read"
has "missing: PD_GPU not read" "PD_GPU at the prompt: not read"
has "missing: verdict not read" "verdict: not read"
has "missing: GLB_RST_CON not read" "0xfd7c0c10: not read"
has "missing: supply not read" "RK806 at hand-over: not read"
hasnt "missing: nothing decoded as keep or move" "gpu_clock_set took"

# 7. files instead of directories
run "$R/answers.txt" "$RC2/image.log"
rc_is "files given: exit 0" 0
has "files given: picks verdict.txt next to image.log" "verdict: VERDICT t6_gpu PASS"

# 8. redaction refusals: each kind, in image.log; nothing may be echoed
i=0
for kind in "rfc2217://192.0.2.9:4001/x" "/dev/ttyUSB7" "aa:bb:cc:dd:ee:0f" "$IP2"; do
	RX="$W/20261125-107$i-t6_gpu-move-0123abcd"; go move "$RX"
	echo "console: $kind" >>"$RX/image.log"
	run "$R" "$RX"
	rc_is "redaction ($kind): exit 3" 3
	if [ -z "$OUT" ] && ! grep -qF -- "$kind" <<<"$ERR"; then ok "redaction ($kind): nothing echoed"; else bad "redaction ($kind): echoed"; fi
	i=$((i + 1))
done
# a MAC inside a md.l answer, an address in verdict.txt, an address in a record name
RM="$W/reads-mac"; reads 00000000 off "$RM"
sed -i 's/^## reset/## md.l 0xfd7c0c10 1\nfd7c0c10: aa:bb:cc:dd:ee:0f\n## reset/' "$RM/answers.txt"
run "$RM" "$RC1"; rc_is "redaction: MAC in a md.l answer: exit 3" 3
RV="$W/20261125-1080-t6_gpu-move-0123abcd"; go move "$RV"; echo "VERDICT t6_gpu PASS via /dev/ttyS0" >>"$RV/verdict.txt"
run "$R" "$RV"; rc_is "redaction: /dev/tty in verdict.txt: exit 3" 3
RN="$W/$IP3-t6_cold"; go move "$RN"; run "$R" "$RN"
rc_is "redaction: address in a record name: exit 3" 3
# printenv holds the board's own MAC and IP in answers.txt: never read, never echoed.
run "$R" "$RC1"
rc_is "printenv's MAC and IP in answers.txt do not refuse" 0
hasnt "printenv's MAC is not echoed" "aa:bb"
hasnt "printenv's IP is not echoed" "$IP1"

# 10. UART2 prompt-read lines, the GIC wake line and the first-feed line.
# SYNTHETIC fixtures: made-up lines in the formats of prompt-reads.py's
# uart2_decode(), aarch64.rs gic_active_reset's line and nanopc_t6.c's feeder.
RU="$W/20261125-1100-reads-0123abcd"; reads 00000000 off "$RU"
{
	echo "# SYNTHETIC: UART2 notes as prompt-reads.py writes them, not a measurement"
	echo "# UART2 LCR 0x00000003: 8N1, DLAB clear"
	echo "# UART2 USR 0x00000006: BUSY 0, TFNF 1, TFE 1, RFNE 0, RFF 0"
	echo "# UART2 clock select: CLKSEL_CON(43) 0x00000090, (44) 0x00010003, (45) 0x00000001: source cpll / 5 * 1 / 3"
	echo "# UART2 input clock 100000000 Hz; divisor 1 gives 6250000 baud (clk / 16); the PLL mode is not applied"
} >>"$RU/answers.txt"
RG="$W/20261125-1110-t6_gpu-gic-0123abcd"; go keep "$RG"
{
	echo "# SYNTHETIC lines: GIC and feeder, not a measurement"
	echo "t6: GIC active state: GICD_TYPER 0x0000ff25 GICR_TYPER 0x0000000000000000 GICR_ISACTIVER0 0x08000000 as found, 1 INTID(s) active (INTIDs 27), 16 word(s) written ICACTIVER, 0 still active"
	echo "yakogl-wdt: nanopc_t6 first feed at 2005 ms"
} >>"$RG/image.log"
run "$RU" "$RG"
rc_is "new lines: exit 0" 0
has "uart2: LCR line" "UART2 LCR 0x00000003: 8N1, DLAB clear"
has "uart2: clock input line" "UART2 input clock 100000000 Hz; divisor 1 gives 6250000 baud"
has "gic: line echoed" "GIC active state: GICD_TYPER 0x0000ff25"
has "gic: found" "a redistributor matched"
has "feed: first feed" "first feed at 2005 ms"
hasnt "feed: not absent" "feeder timer never ticked"
# the new lines come after the old ones
if [ "$(grep -n 'verdict:' <<<"$OUT" | head -1 | cut -d: -f1)" -lt "$(grep -n 'UART2 at the prompt' <<<"$OUT" | head -1 | cut -d: -f1)" ]; then ok "new lines are appended after the verdict"; else bad "new lines are appended after the verdict"; fi
# no redistributor, no first feed
RH="$W/20261125-1120-t6_gpu-nogic-0123abcd"; go keep "$RH"
echo "t6: GIC active state: GICD_TYPER 0x0000ff25, no redistributor with affinity 0x00000000 in 1 frame(s): SGIs and PPIs not scanned, 0 INTID(s) active (none), 15 word(s) written ICACTIVER, 0 still active" >>"$RH/image.log"
run "$R" "$RH"
has "gic: no redistributor" "no redistributor matched"
has "first feed absent" "first feed: absent"
has "first feed absent: says why" "the feeder timer never ticked"
has "uart2 absent: not read" "UART2 at the prompt: not read"
# nothing at all: the GIC line is not read
run "$R" "$RB"
has "gic absent: not read" "GIC active state line: not read"
has "first feed absent on a bare log" "first feed: absent"

# 11. CNTFRQ_EL0 0 stop, the banner value, and the GIC wake timeout.
# SYNTHETIC fixtures in the formats of nanopc_t6_early.rs, nanopc_t6_mali.c
# and aarch64.rs gic_active_reset.
RZ="$W/20261125-1130-t6_cold-cntfrq0-0123abcd"; go keep "$RZ"
{
	echo "# SYNTHETIC lines: not a measurement"
	echo "t6: CNTFRQ_EL0 0: no time base, the osal and firmware entries refuse it"
} >>"$RZ/image.log"
run "$R" "$RZ"
has "cntfrq 0 stop: reported" "cntfrq: 0 at entry (image refused)"
RZB="$W/20261125-1131-t6_cold-cntfrqb-0123abcd"; go keep "$RZB"
{
	echo "T6:A EL2 HCR_EL2 0x0000000488000038 -> 0x0000000080000000 SCTLR_EL2 0x0000000030c50838 VBAR_EL2 0x0000000000200800 -> 0x0000000010008000 CNTFRQ_EL0 1000000000"
} >>"$RZB/image.log"
run "$R" "$RZB"
has "cntfrq banner value" "cntfrq: 1000000000 at entry"
hasnt "cntfrq banner value: not refused" "image refused"
RZC="$W/20261125-1132-t6_cold-cntfrqc-0123abcd"; go keep "$RZC"
{
	echo "T6:A EL2 HCR_EL2 0x0000000488000038 -> 0x0000000080000000 SCTLR_EL2 0x0000000030c50838 VBAR_EL2 0x0000000000200800 -> 0x0000000010008000 CNTFRQ_EL0 0"
	echo "t6: CNTFRQ_EL0 0: no time base, the osal and firmware entries refuse it"
} >>"$RZC/image.log"
run "$R" "$RZC"
has "cntfrq banner 0 and stop" "cntfrq: 0 at entry (image refused)"
RGT="$W/20261125-1133-t6_cold-gictimeout-0123abcd"; go keep "$RGT"
echo "t6: GIC active state: GICD_TYPER 0x0000000f, gic: redistributor at frame 2 did not wake (ChildrenAsleep still set): SGIs and PPIs not scanned, 1 INTID(s) active (INTIDs 71), 15 word(s) written ICACTIVER, 0 still active" >>"$RGT/image.log"
run "$R" "$RGT"
has "gic timeout: frame" "redistributor at frame 2 did not wake (ChildrenAsleep still set)"
has "gic timeout: apart from no match" "not 'no redistributor matched'"
hasnt "gic timeout: not reported as matched" "a redistributor matched"
hasnt "gic timeout: not the no-match line" "the wake timed out or the affinity has no frame"

# 12. a t6_cold-only record has no hand-over lines: not read, with the hint;
# t6_cold plus t6_gpu takes the hand-over from t6_gpu and says so.
RCold="$W/20261125-1130-t6_cold-0123abcd"; mkdir -p "$RCold"
{
	echo "T6:A3 WDT0 arm rc 0 CR 0x00000009 period_ms 22369"
	echo "*** Booting Zephyr OS build v4.2.0 ***"
	echo "t6_cold: display chain: VOP2 found, scan-out adopted"
} >"$RCold/image.log"
echo "VERDICT t6_cold PASS" >"$RCold/verdict.txt"
run "$R" "$RCold"
rc_is "t6_cold only: exit 0" 0
has "t6_cold only: clock not read" "gpu_clock_set: not read"
has "t6_cold only: RK806 not read" "RK806 at hand-over: not read"
has "t6_cold only: PD_GPU at hand-over not read" "PD_GPU at hand-over: not read"
has "t6_cold only: CRU not read" "CRU at hand-over: not read"
has "t6_cold only: hint names t6_gpu" "pass the t6_gpu go record"
has "t6_cold only: WDT0 arm still read" "rc 0, CR 0x00000009, period 22369 ms"
hasnt "t6_cold only: single record has no source tags" "[from "
run "$R" "$RCold" "$RC2"
rc_is "two records: exit 0" 0
has "two records: both names" "go 20261125-1130-t6_cold-0123abcd, 20261125-1020-t6_gpu-move-0123abcd"
has "two records: clock from t6_gpu" "gpu_clock_set took: move: CLKSEL_CON(158) written, both words read back [from 20261125-1020-t6_gpu-move-0123abcd]"
has "two records: PD_GPU from t6_gpu" "PD_GPU at hand-over: PWR_GATE 0x0000ffff"
has "two records: WDT0 arm from t6_cold" "rc 0, CR 0x00000009, period 22369 ms [from 20261125-1130-t6_cold-0123abcd]"
has "two records: verdict of each" "verdict (20261125-1130-t6_cold-0123abcd): VERDICT t6_cold PASS"
has "two records: second verdict" "verdict (20261125-1020-t6_gpu-move-0123abcd): VERDICT t6_gpu PASS"
hasnt "two records: no hint" "hint:"
run "$R" "$RCold" "$W/nowhere"; rc_is "second go record unreadable: exit 4" 4

# 9. usage and unreadable
run; rc_is "no arguments: exit 2" 2
run "$W/nowhere" "$RC1"; rc_is "no answers.txt: exit 4" 4
mkdir -p "$W/noimg"; run "$R" "$W/noimg"; rc_is "no image.log: exit 4" 4

echo "t6-uboot-handover-summary selftest: $n checks, $fails failed"
[ "$fails" = 0 ] && [ "$n" -gt 0 ]
