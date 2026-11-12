#!/usr/bin/env bash
# SPDX-License-Identifier: Apache-2.0
# SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
# t6-verdict.sh - check a NanoPC-T6 Zephyr run's uart.log (the record
# nanopc-t6.sh --go writes) against the lines the probe must print.
#
#   driver/tools/kexec-zephyr/t6-verdict.sh <probe> <uart.log>
#   T6_POOL_PA=<hex> T6_BACKING_PA=<hex> t6-verdict.sh t6_mmu <uart.log>
#
# probe: t6_hello, t6_gpu, t6_mmu, t6_fw or t6_submit. The expected lines are in order: the
# early banner (T6:A .. T6:A3, board::nanopc_t6_early), the board glue's
# lines (T6:B, the marker, the GIC line, the arming, T6:C, "board: watchdog
# armed"; driver/zephyr/lib/board/nanopc_t6/nanopc_t6.c), the probe's own
# lines (tools/probes/nanopc_t6/<probe>/src/main.c) and the glue's reset
# line. Each template below is the printf format with its conversions
# replaced: a value driver/docs/nanopc-t6-mali.md predicts exactly is written
# out and compared exactly; one it gives as a range, "recorded" or not at all
# is a token, checked for presence and form only. Tokens:
#   {x8} {x16}  exactly 8 or 16 lower-case hex digits (%08x, Tx::hex 16)
#   {x}         one or more hex digits (%x, %llx, %p after its 0x)
#   {d}         a decimal number, optionally negative (%d, %u, %lld)
#   {s}         any non-empty text (%s)
#   {@NAME}     hex digits, captured into NAME (a later {=...} uses it)
#   {=NAME}     the value of NAME, compared exactly (derived by a hook)
#
# A hook (run after a line matched) may check what one template cannot, the
# equality of two values on the line: when it fails, the line "differs".
#
# Each expected line is looked for after the line the previous one was found
# on. A line carrying the template's leading text (or its prefix pattern) is
# the candidate: "ok" when the whole template matches it, "differs" when not;
# no candidate at all is "missing". One line each, with the expected and the
# seen text; then "VERDICT <probe> PASS" (exit 0) or "VERDICT <probe> FAIL
# (<n> of <m>)" (exit 1: n expected lines not ok, of m). Exit 2: usage.
#
# t6_mmu: P (t6_mmu_pool's PA) and B (t6_mmu_backing's) are taken from the
# "pool" line, or from T6_POOL_PA and T6_BACKING_PA when set (step 1 of the
# Z2 procedure: `nm zephyr.elf`; with or without 0x), which then must equal
# what the line prints. The image is identity mapped, so the line's %p and
# its PA must be the same number. The tables, walk, enable and PASS lines
# are compared against them.
#
# t6_mmu, t6_fw, t6_submit: when Z1's power-on stopped (t6_mmu's "Z1: rc"
# not 0, the "Z3: rc" line's step 3), Z1's record (table_z1_stop) is
# expected after the line that reports the stop; the verdict stays FAIL.
#
# t6_fw: JOB_INT_RAWSTAT must have bit 31 set and the GLB req and ack words
# agree on the acknowledged bits; hooks check both.
#
# t6_submit: Z3's lines as t6_fw's, then the Z4 record's eleven lines (Z4a ..
# Z4k, board::nanopc_t6_submit::SubmitUp). The status line (Z4a) pins step 0,
# refusal 0, detail 0, so a stopped run (wait timeout, Z3 stopped) differs
# there and on the fields it left at 0, and its FAIL line stands for PASS.
# Pinned from the first board run (issue #4515): the suspend sizes 25600 /
# 25600 and CS 0's ack 0x00000011. The record's timing (*_us), latest_flush
# and as0_tables_used (the board's file sizes Z3's tables: the model's 8 is
# not the board's) are tokens; so are, until the second board run measures
# them, extract after the wait, sync_updates, cleared and the drain's line
# and extract after the end (Z4j, Z4k). vm_tables_used, the heap chunk and
# the digest are pinned as the generated frame's record in nanopc_t6_submit
# pins them; the target's pixel counts are tokens whose sum must equal the
# pixels read.
set -euo pipefail
export LC_ALL=C

usage() { echo "usage: $0 {t6_hello|t6_gpu|t6_mmu|t6_fw|t6_submit} <uart.log>" >&2; exit 2; }
[ $# -eq 2 ] || usage
PROBE=$1
LOG=$2
[ -r "$LOG" ] || { echo "t6-verdict.sh: $LOG: not readable" >&2; exit 2; }

# ---- the log ----------------------------------------------------------------
# uart.log as the UART gave it: CR LF from the early banner and the console,
# maybe NULs or line noise around the jump and the resets.
mapfile -t LINES < <(tr -d '\r\000' <"$LOG")

# ---- the tables -------------------------------------------------------------
TPL=()  # the templates, in order
PFX=()  # an ERE that marks the candidate line ('' = the template's leading text)
HOOK=() # a function run after the line was found ('' = none)
declare -A V=() # captured and derived values

# e <template> [prefix ERE] [hook]
e() { TPL+=("$1"); PFX+=("${2:-}"); HOOK+=("${3:-}"); }

# The image's first lines (driver/rust/src/board/nanopc_t6_early.rs,
# banner_a_el2 .. banner_a3, printed raw on uart2) and the board glue's at
# PRE_KERNEL_1 99 (nanopc_t6.c, t6_wdt_report), for the board's Kconfig
# defaults: VHE exit on, TOP 13 (22369 ms), lifetime 60 s, alive 5 s, no
# proof, loaded at 0x10000000.
table_boot() {
	# banner_a_el2 with the VHE exit: HCR_EL2 found -> written (Z1 measured
	# 0x000000048800003a -> 0x0000000080000002; not a prediction for a probe)
	e 'T6:A EL2 HCR_EL2 0x{x16} -> 0x{x16} SCTLR_EL2 0x{x16} VBAR_EL2 0x{x16} -> 0x{x16}' 'T6:A EL'
	e 'T6:A1 EL1 init done, MMU off'
	e 'T6:A2 PRE_KERNEL_1, MMU on: arming WDT0'
	# banner_a3: rc 0, CR read back 0x9 (WDT_EN, RMOD), TOP 13's period
	e 'T6:A3 WDT0 arm rc 0 CR 0x00000009 period_ms 22369'
	e 'T6:B console up (uart2 through the ns16550 driver)'
	e 'yakogl-wdt: nanopc_t6 WDT0 armed at boot; top 13; lifetime_s 60; alive_s 5; proof 0; load 0x10000000'
	# printed when Linux left EOImode 1 (every kexec so far); the value is recorded
	e 't6: GICv3 CPU interface: ICC_CTLR_EL1 0x{x} had EOImode 1 (the kernel before us), now 0'
	# two printks, one line; as found depends on --no-linux-wdt (was stopped)
	# or not (was running): recorded. CR after is WDT_EN | RMOD.
	e 't6: WDT0 armed: CR 0x{x8} -> 0x00000009 (was {s}), TORR 0x{x8} -> 0x{x8}, CCVR 0x{x8}, PARAM_1 0x{x8}, version 0x{x8}, flags 0x{x}: period 22369 ms (TOP 13), fed until 60 s of uptime, alive window 5 s' \
		't6: WDT0 arm'
	e 'T6:C WDT0 armed, period 22369 ms'
	e 'board: watchdog armed'
}

# nanopc_t6.c, t6_reset_to_linux, the probe's last line.
table_reset() {
	e "t6: $1 done: PSCI SYSTEM_RESET at {d} ms ({d} feeds)"
}

# Z1, tools/probes/nanopc_t6/t6_hello/src/main.c; the doc's "Z1 PASS" record.
table_t6_hello() {
	table_boot
	e 't6_hello: build {s}'
	e 't6_hello: nanopc_t6, console uart2 at 1500000 baud, image at 0x10000000'
	e 't6_hello: WDT0 period 22369 ms, fed until 60000 ms of uptime'
	local i
	for i in 1 2 3 4 5 6 7 8 9 10; do
		e "t6_hello: $i s (uptime {d} ms)" "t6_hello: $i s \\(uptime"
	done
	e 't6_hello: PASS: counted 10 s in {d} ms' 't6_hello: (PASS|FAIL)'
	table_reset t6_hello
}

# Z1's second half, tools/probes/nanopc_t6/t6_gpu/src/main.c on the success
# path; the doc's "The GPU power-on", measured 2026-10-19: the identity
# registers exactly (panthor's probe line, value for value), what Linux
# hands over only as present except PD_GPU off and the supply on.
table_t6_gpu() {
	table_boot
	e 't6_gpu: build {s}'
	e 't6_gpu: WDT0 period 22369 ms, fed until 60000 ms of uptime'
	# on_handover
	e 't6_gpu: PD_GPU at hand-over: PWR_GATE 0x{x8} STATUS 0x{x8} BUS_IDLE_REQ 0x{x8} ACK 0x{x8} ST 0x{x8} REPAIR_STATUS 0x{x8}: off or idle'
	e 't6_gpu: CRU at hand-over: CLKSEL_CON(158) 0x{x8} CLKSEL_CON(159) 0x{x8} CLKGATE_CON(66) 0x{x8} MODE_CON0 0x{x8} PLL_CON 0x{x8} 0x{x8} 0x{x8}: clk_gpu_src {d} Hz'
	e 't6_gpu: SPI2 gates CLKGATE_CON(14) 0x{x8}, opened 0x{x}'
	e 't6_gpu: SPI2 as found: CTRLR0 0x{x8} CTRLR1 0x{x8} SSIENR 0x{x} SER 0x{x} BAUDR 0x{x} SR 0x{x8} RXFLR 0x{x} VERSION 0x{x8}'
	e 't6_gpu: RK806 at hand-over: POWER_EN0 0x{x} POWER_SLP_EN0 0x{x} BUCK1_CONFIG 0x{x} BUCK1_ON_VSEL 0x{x}: vdd_gpu_s0 on, {d} uV'
	e 't6_gpu: GPU PLL on: PLL_CON6 0x{x8} at hand-over, was on {d}, waited {d} us, PLL_CON6 0x{x8}'
	e 't6_gpu: PD_GPU on: gates opened 0x{x}, was on {d}, waited {d}/{d}/{d} us'
	e 't6_gpu: PD_GPU after: PWR_GATE 0x{x8} STATUS 0x{x8} BUS_IDLE_REQ 0x{x8} ACK 0x{x8} ST 0x{x8} REPAIR_STATUS 0x{x8}: on'
	local b
	for b in 0 1 2 3; do
		e "t6_gpu: qos_gpu_m$b: 0x{x8} 0x{x8} 0x{x8} 0x{x8} 0x{x8}"
	done
	e 't6_gpu: GPU after power-on: GPU_ID 0xa8670005 L2_FEATURES 0x07120306 TILER 0x00000809 MEM 0x00000301 MMU 0x00002830 AS_PRESENT 0x000000ff CSF_ID 0x040a0412'
	e 't6_gpu: GPU after power-on: SHADER_PRESENT 0x0000000000050005 TILER_PRESENT 0x{x16} L2_PRESENT 0x{x16} COHERENCY_FEATURES 0x{x8} GPU_STATUS 0x{x8} MCU_STATUS 0x{x8}'
	e 't6_gpu: soft reset: GPU_INT_RAWSTAT 0x{x8} before the clear, 0x{x8} after; RESET_COMPLETED after {d} polls of 1 us'
	e 't6_gpu: GPU after reset: GPU_ID 0xa8670005 L2_FEATURES 0x07120306 TILER 0x00000809 MEM 0x00000301 MMU 0x00002830 AS_PRESENT 0x000000ff CSF_ID 0x040a0412'
	e 't6_gpu: GPU after reset: SHADER_PRESENT 0x0000000000050005 TILER_PRESENT 0x{x16} L2_PRESENT 0x{x16} COHERENCY_FEATURES 0x{x8} GPU_STATUS 0x{x8} MCU_STATUS 0x{x8}'
	e 't6_gpu: L2 mask 0x{x}: L2_READY 0x{x} before L2_PWRON, 0x{x} after; waited {d}/{d} polls'
	e 't6_gpu: PASS: GPU_ID 0xa8670005, L2 ready, {d} ms' 't6_gpu: (PASS|FAIL)'
	table_reset t6_gpu
}

# a line of the log matches the ERE $1
seen_line() {
	local l
	for l in "${LINES[@]}"; do
		[[ $l =~ $1 ]] && return 0
	done
	return 1
}

# table_z1_stop <probe>: Z1's record as a probe after t6_gpu prints it when
# Z1's power-on stops (tools/probes/nanopc_t6/common/t6_z1.c,
# t6_z1_print_stop): the hand-over lines, the clock set, the supply enable
# and the stop line, which names the stop. A stopped run's values are not
# predicted: each line is checked for its form only, and a record without
# them (an image that does not print them) is named line by line. The lines
# of the steps the run reached (PLL, PD_GPU, identity, reset) depend on the
# stop and are not judged.
table_z1_stop() {
	local p=$1
	e "$p: PD_GPU at hand-over: PWR_GATE 0x{x8} STATUS 0x{x8} BUS_IDLE_REQ 0x{x8} ACK 0x{x8} ST 0x{x8} REPAIR_STATUS 0x{x8}: {s}"
	e "$p: CRU at hand-over: CLKSEL_CON(158) 0x{x8} CLKSEL_CON(159) 0x{x8} CLKGATE_CON(66) 0x{x8} MODE_CON0 0x{x8} PLL_CON 0x{x8} 0x{x8} 0x{x8}: clk_gpu_src {d} Hz"
	e "$p: SPI2 gates CLKGATE_CON(14) 0x{x8}, opened 0x{x}"
	e "$p: SPI2 as found: CTRLR0 0x{x8} CTRLR1 0x{x8} SSIENR 0x{x} SER 0x{x} BAUDR 0x{x} SR 0x{x8} RXFLR 0x{x} VERSION 0x{x8}"
	e "$p: RK806 at hand-over: POWER_EN0 0x{x} POWER_SLP_EN0 0x{x} BUCK1_CONFIG 0x{x} BUCK1_ON_VSEL 0x{x}: vdd_gpu_s0 {s}, {d} uV"
	e "$p: clock set: kept {d}, CPLL {d} Hz, wrote CLKSEL_CON(158) 0x{x8} (159) 0x{x8}, now 0x{x8} 0x{x8}: clk_gpu_src {d} Hz"
	e "$p: vdd_gpu_s0 enable: {s}; POWER_EN0 0x{x} BUCK1_ON_VSEL 0x{x} after, waited {d} us"
	e "$p: Z1 stopped: stop {d} ({s}), refusal {d}, detail 0x{x16}"
}

hex16() { printf '%016x' "$1"; }

# P and B (from the pool line or the environment) and what the doc's
# prediction derives from them: the table descriptors of the walk (each
# next table's PA | 0b11), the L3 page of VA 0x04001234 (B + 0x1000 |
# 0x0060000000000f47), and its PA.
mmu_derive() {
	local p=$((16#${V[P]:-0})) b=$((16#${V[B]:-0}))
	V[P16]=$(hex16 "$p")
	V[B16]=$(hex16 "$b")
	V[L0]=$(hex16 $((p + 0x1003)))
	V[L1]=$(hex16 $((p + 0x2003)))
	V[L2]=$(hex16 $((p + 0x3003)))
	V[L3]=$(hex16 $(((b + 0x1000) | 0x0060000000000f47)))
	V[WPA]=$(hex16 $((b + 0x1234)))
}

nm_hex() { local v=${1#0x}; v=${v#0X}; printf '%x' $((16#$v)); }

# the pool line: mmu_derive from the PAs, and each %p equal to its PA
# (the identity map)
mmu_pool() {
	mmu_derive
	[ $((16#${V[PV]})) -eq $((16#${V[P]})) ] && [ $((16#${V[BV]})) -eq $((16#${V[B]})) ]
}

# Z2, tools/probes/nanopc_t6/t6_mmu/src/main.c on the success path; the
# predicted lines are the doc's "Z2: AS0 and a cache flush" ("The expected
# uart2 record").
table_t6_mmu() {
	table_boot
	e 't6_mmu: build {s}'
	e 't6_mmu: WDT0 period 22369 ms, fed until 60000 ms of uptime'
	e 't6_mmu: Z1: rc 0 stop 0 refusal 0: GPU_ID 0xa8670005 MMU_FEATURES 0x00002830 AS_PRESENT 0x000000ff L2_READY 0x1'
	# Z1 stopped (rc not 0): its record follows
	if seen_line '^t6_mmu: Z1: rc -[0-9]+ '; then table_z1_stop t6_mmu; fi
	if [ -n "${T6_POOL_PA:-}" ] || [ -n "${T6_BACKING_PA:-}" ]; then
		: "${T6_POOL_PA:?T6_BACKING_PA needs T6_POOL_PA too}" "${T6_BACKING_PA:?T6_POOL_PA needs T6_BACKING_PA too}"
		V[P]=$(nm_hex "$T6_POOL_PA")
		V[B]=$(nm_hex "$T6_BACKING_PA")
		mmu_derive
		e 't6_mmu: pool 0x{=P} at PA 0x{=P} (16 pages), backing 0x{=B} at PA 0x{=B}'
	else
		e 't6_mmu: pool 0x{@PV} at PA 0x{@P} (16 pages), backing 0x{@BV} at PA 0x{@B}' '' mmu_pool
	fi
	# as found after Z1's soft reset: TRANSTAB, MEMATTR, STATUS, FAULTSTATUS,
	# FAULTADDRESS 0; TRANSCFG and LOCKADDR recorded
	e 't6_mmu: AS0 as found: TRANSTAB 0x0000000000000000 MEMATTR 0x0000000000000000 TRANSCFG 0x{x16} LOCKADDR 0x{x16} STATUS 0x00000000 FAULTSTATUS 0x00000000 FAULTADDRESS 0x0000000000000000'
	# GPU_INT_RAWSTAT: bit 17 clear, bits 9/10 may be set; the masks recorded
	e 't6_mmu: IRQ as found: MMU_INT_RAWSTAT 0x00000000 MMU_INT_MASK 0x{x8} GPU_INT_RAWSTAT 0x{x8} GPU_INT_MASK 0x{x8}'
	e 't6_mmu: flush round trip: GPU_CMD 0x00003304, GPU_INT_RAWSTAT 0x{x8} before the clear, 0x{x8} after; CLEAN_CACHES_COMPLETED after {d} polls of 1 us'
	e 't6_mmu: tables at PA 0x{=P16}: 4 of 16 pages used; backing PA 0x{=B16}'
	e 't6_mmu: walk VA 0x0000000004001234: L0 0x{=L0} L1 0x{=L1} L2 0x{=L2} L3 0x{=L3} (4 levels): PA 0x{=WPA}'
	e 't6_mmu: tables cleaned to the point of coherency: rc 0'
	e 't6_mmu: AS0 enable, the whole-VA flush: LOCKADDR 0x000000000000003f, LOCK polls {d}/{d}; GPU_CMD 0x00003304, GPU_INT_RAWSTAT 0x{x8} before the clear, 0x{x8} after, CLEAN_CACHES_COMPLETED after {d} polls of 1 us; UNLOCK polls {d}/{d}'
	e 't6_mmu: AS0 enable: TRANSTAB 0x{=P16} MEMATTR 0x8f8f8f8f8c4c8f4c TRANSCFG 0x00000000420001c6 written; UPDATE polls {d}/{d}'
	e 't6_mmu: AS0 after: TRANSTAB 0x{=P16} MEMATTR 0x8f8f8f8f8c4c8f4c TRANSCFG 0x00000000420001c6 LOCKADDR 0x000000000000003f STATUS 0x00000000 FAULTSTATUS 0x00000000 FAULTADDRESS 0x0000000000000000'
	# bit 16: AS0's commands completed (no fault: the fault bits are 15:0)
	e 't6_mmu: IRQ after: MMU_INT_RAWSTAT 0x00010000 MMU_INT_MASK 0x{x8} GPU_INT_RAWSTAT 0x{x8} GPU_INT_MASK 0x{x8}'
	e 't6_mmu: the flush after the map: LOCKADDR 0x000000000400000f, LOCK polls {d}/{d}; GPU_CMD 0x00000304, GPU_INT_RAWSTAT 0x{x8} before the clear, 0x{x8} after, CLEAN_CACHES_COMPLETED after {d} polls of 1 us; UNLOCK polls {d}/{d}'
	e 't6_mmu: PASS: AS0 on TRANSTAB 0x{=P16}, three cache flush round trips, {d} ms' 't6_mmu: (PASS|FAIL)'
	table_reset t6_mmu
}

# JOB_INT_RAWSTAT after the start: bit 31 (JOB_INT_GLOBAL_IF) set
fw_job_bit31() { (( 16#${V[J]} & 0x80000000 )); }

# the GLB req and ack words agree on the bits of acked (0x10e)
fw_req_ack() { (( ((16#${V[RQ]} ^ 16#${V[AK]}) & 0x10e) == 0 )); }

# table_fw_image <probe>: the lines every probe built on Z3 prints first, with
# its own name as the prefix: build, WDT0, image/sha256, the 7 sections, the pool.
table_fw_image() {
	local p=$1
	e "$p: build {s}"
	e "$p: WDT0 period 22369 ms, fed until 60000 ms of uptime"
	e "$p: image {d} bytes, 7 loaded sections, sha256 {s}, magic 0x{x8}"
	local i=0 va
	for va in 0x00400000:64 0x00410000:64 0x00000000:64 0x00800000:128 0x02000000:256 0x01000000:256 0x04000000:64; do
		e "$p: section $i: VA ${va%:*}, ${va#*:} KiB, backing 0x{x} at PA 0x{x}" "$p: section $i:"
		i=$((i + 1))
	done
	e "$p: pool 0x{x} at PA 0x{x} (16 pages)"
}

# table_z3_record <probe>: yd_t6_fw_up's "Z3: rc" line and print_record's 7
# lines, with the probe's name as the prefix.
table_z3_record() {
	local p=$1
	e "$p: Z3: rc 0 step 0, {d} ms"
	e "$p: Z3: image version 0x01050000, 7 sections loaded, 1 protected skipped, 9 table pages, 7 flushes"
	e "$p: Z3: GPU_ID 0xa8670005 SHADER_PRESENT 0x0000000000050005 MMU_FEATURES 0x00002830 shared PA 0x{x16} clean rc 0"
	e "$p: Z3: MCU_STATUS 0 before, 1 after, booted in {d} us, JOB_INT_RAWSTAT 0x{@J}" '' fw_job_bit31
	e "$p: Z3: GLB version 1.5.0 (0x01050000), features 0x00000000, instr 0x00000071, 8 groups stride 0xa0"
	e "$p: Z3: GLB input 0x04002000 output 0x04006000, CSG0 in 0x04003000 out 0x04007000 8 streams stride 0xc, CS0 in 0x04003400 out 0x04007200 features 0x{x8}"
	e "$p: Z3: CORE_EN 0x0000000000050005, poweroff 0x000000eb progress 0x00271000 idle 0x00000013, ack_irq_mask 0x0400050e"
	e "$p: Z3: init req 0x0000040e acked 0x0000010e in {d} us, PING acked in {d} us, GLB req 0x{@RQ} ack 0x{@AK}" '' fw_req_ack
}

# tools/probes/nanopc_t6/t6_fw/src/main.c on the success path: build, image,
# the 7 loaded sections (in fw::Image::sections() order), the pool, then
# yd_t6_fw_up's one call (Z1's power-on, Z2's AS0, the load, the boot): its
# "Z3: rc" line, print_record's 7 lines, PASS and the reset line. The
# predicted values are the doc's Z3 table (board::nanopc_t6_fw::FwUp): GLB
# version 0x01050000 (1.5.0), the PING acknowledged. The recorded ones (boot
# and acknowledge times, the shared section's PA, CS0's features) are tokens.
# A run that stops prints "t6_fw: Z3: stop ..." (differs, with its step) and
# "t6_fw: FAIL: Z3 step N (name)" in place of PASS (differs, naming the step).
table_t6_fw() {
	table_boot
	table_fw_image t6_fw
	table_z3_record t6_fw
	# Z1 stopped (step 3, YD_T6_FW_STEP_GPU): its record follows the stop line
	if seen_line '^t6_fw: Z3: rc -?[0-9]+ step 3,'; then table_z1_stop t6_fw; fi
	e 't6_fw: PASS' 't6_fw: (PASS|FAIL)'
	table_reset t6_fw
}

# the target's counts (printed in decimal) add up to the pixels read
z4_pixels() {
	local k
	for k in R B F O P; do [[ ${V[$k]} =~ ^[0-9]+$ ]] || return 1; done
	(( 10#${V[R]} + 10#${V[B]} + 10#${V[F]} + 10#${V[O]} == 10#${V[P]} ))
}

# tools/probes/nanopc_t6/t6_submit/src/main.c: Z3's lines as t6_fw's, then
# yd_t6_submit's record (SubmitUp, 424 bytes) in eleven lines, %016llx for the
# 64-bit words, %08x for the 32-bit words, %u for counts. The predicted values
# are the_generated_frame_s_record_is_as_predicted's, over the MCU model; the
# ones the first board run measured are pinned from it. A stopped run prints its record too (the
# fields after the stop at 0) and "t6_submit: FAIL: Z4 step N (name)".
table_t6_submit() {
	table_boot
	table_fw_image t6_submit
	table_z3_record t6_submit
	# Z1 stopped (Z3's step 3): its record follows the Z3 stop line
	if seen_line '^t6_submit: Z3: rc -?[0-9]+ step 3,'; then table_z1_stop t6_submit; fi
	e 't6_submit: Z4a: step 0 refusal 0 detail 0x0000000000000000' 't6_submit: Z4a:'
	e 't6_submit: Z4b: AS_PRESENT 0x000000ff L2_FEATURES 0x07120306 as0_tables_used {d} vm_tables_used 10 suspend_size 25600 protm_suspend_size 25600' 't6_submit: Z4b:'
	e 't6_submit: Z4c: heap ctx 0x0000800000015000 chunk 0x0000800000a00000 chunk_size 2097152 stride 64' 't6_submit: Z4c:'
	e 't6_submit: Z4d: CSG0 req 0x00000011 ack 0x00000011 CS0 req 0x00000511 ack 0x00000011, acked in {d}/{d} us' 't6_submit: Z4d:'
	# extract: the firmware publishes it after the stream's seqno (issue
	# #4515: 0 at the wait's end on the first run); its value at the wait's
	# end is not measured on the board yet as a rule: form only
	e 't6_submit: Z4e: insert 0x0000000000000080 extract 0x{x16} seqno 0x0000000000000001 sync_status 0x00000000, wait {d} us' 't6_submit: Z4e:'
	# sync_updates (Z4f) and cleared (Z4g) move with when the firmware
	# publishes extract: not measured on the board yet, form only
	e 't6_submit: Z4f: latest_flush 0x{x8} sync_updates {d} status_cmd_ptr 0x0000000000000000 info 0x0000000000000000' 't6_submit: Z4f:'
	e 't6_submit: Z4g: stop {d} us refusal 0, terminate {d} us refusal 0, cleared {d} {d} {d}' 't6_submit: Z4g:'
	e 't6_submit: Z4h: target red {@R} blue {@B} fill {@F} other {@O} of {@P} pixels' 't6_submit: Z4h:' z4_pixels
	e 't6_submit: Z4i: frame digest 0xa4db81a9d39bfdcc' 't6_submit: Z4i:'
	# sched::drained and extract after the group's end: not measured on the
	# board yet, form only (the commit that records the second run pins them)
	e 't6_submit: Z4j: drain {d} us refusal {d} extract 0x{x16} sync_updates {d} cs_ack 0x{x8}' 't6_submit: Z4j:'
	e 't6_submit: Z4k: extract after the end 0x{x16}' 't6_submit: Z4k:'
	e 't6_submit: PASS' 't6_submit: (PASS|FAIL)'
	table_reset t6_submit
}

case "$PROBE" in
t6_hello | t6_gpu | t6_mmu | t6_fw | t6_submit) "table_$PROBE" ;;
*) usage ;;
esac

# ---- the templates ----------------------------------------------------------
ere_quote() { printf '%s' "$1" | sed 's/[][\.*^$+?(){}|]/\\&/g'; }

# to_ere <template>: sets RE (the ERE, with one group per {@NAME}), CAPS
# (those names), SHOW (the template with each {=NAME} filled in) and LEAD
# (the text before the first token or value).
to_ere() {
	local t=$1 lit tok val
	RE= SHOW= CAPS=() LEAD=
	local first=1
	while [ -n "$t" ]; do
		if [[ $t == *'{'* ]]; then
			lit=${t%%\{*}
			t=${t#*\{}
			tok=${t%%\}*}
			t=${t#*\}}
		else
			lit=$t tok= t=
		fi
		RE+=$(ere_quote "$lit")
		SHOW+=$lit
		# the leading text, up to the first value: a line that has it but
		# other values is "differs", not "missing"
		[ "$first" = 0 ] || LEAD=$(printf '%s' "$lit" | sed -E 's/ (0x|[0-9]).*//')
		[ -n "$tok" ] || continue
		first=0
		case "$tok" in
		x8) RE+='[0-9a-f]{8}' SHOW+="{$tok}" ;;
		x16) RE+='[0-9a-f]{16}' SHOW+="{$tok}" ;;
		x) RE+='[0-9a-f]+' SHOW+="{$tok}" ;;
		d) RE+='-?[0-9]+' SHOW+="{$tok}" ;;
		s) RE+='.+' SHOW+="{$tok}" ;;
		@*) RE+='([0-9a-f]+)' SHOW+="{$tok}"; CAPS+=("${tok#@}") ;;
		=*)
			val=${V[${tok#=}]:-}
			if [ -n "$val" ]; then
				RE+=$(ere_quote "$val") SHOW+=$val
			else
				RE+='<unknown>' SHOW+="<${tok#=} unknown>"
			fi
			;;
		*) echo "t6-verdict.sh: bad token {$tok} in the $PROBE table" >&2; exit 2 ;;
		esac
	done
}

cur=0 bad=0 total=${#TPL[@]}
for ((i = 0; i < total; i++)); do
	to_ere "${TPL[i]}"
	pfx=${PFX[i]}
	[ -n "$pfx" ] || pfx=$(ere_quote "$LEAD")
	found=-1
	for ((j = cur; j < ${#LINES[@]}; j++)); do
		if [[ ${LINES[j]} =~ $pfx ]]; then
			found=$j
			break
		fi
	done
	if [ "$found" -lt 0 ]; then
		printf 'missing  expected "%s"; seen nothing after line %d\n' "$SHOW" "$cur"
		bad=$((bad + 1))
		continue
	fi
	seen=${LINES[found]}
	cur=$((found + 1))
	ok=0
	if [[ $seen =~ $RE$ ]]; then
		ok=1
		for ((k = 0; k < ${#CAPS[@]}; k++)); do
			V[${CAPS[k]}]=${BASH_REMATCH[k + 1]}
		done
		[ -z "${HOOK[i]}" ] || "${HOOK[i]}" || ok=0
	fi
	if [ "$ok" = 1 ]; then
		printf 'ok       expected "%s"; seen "%s" (line %d)\n' "$SHOW" "$seen" "$cur"
	else
		printf 'differs  expected "%s"; seen "%s" (line %d)\n' "$SHOW" "$seen" "$cur"
		bad=$((bad + 1))
	fi
done

# t6_fw, t6_submit: the stop line of a run that did not reach the end (the
# stop, the refusal and what it was about) is a differing line too
if [ "$PROBE" = t6_fw ] || [ "$PROBE" = t6_submit ]; then
	for ((j = 0; j < ${#LINES[@]}; j++)); do
		if [[ ${LINES[j]} == "$PROBE: Z3: stop "* ]]; then
			printf 'differs  expected no stop line; seen "%s" (line %d)\n' "${LINES[j]}" $((j + 1))
			bad=$((bad + 1))
		fi
	done
fi

if [ "$bad" -eq 0 ]; then
	echo "VERDICT $PROBE PASS"
	exit 0
fi
echo "VERDICT $PROBE FAIL ($bad of $total)"
exit 1
