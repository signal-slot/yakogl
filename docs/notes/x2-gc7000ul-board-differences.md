<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL on the Armadillo-X2 against the Verdin i.MX8MP: what can change a dEQP outcome

Plan #372 (items x2diff, x2note). From the code only: no board was used and
nothing here was measured. Citations are at `c29a5ffe`, the X2 campaign
commit; each was rechecked against origin/main (`1d602ba3`). The code behind
them is the same there except `tools/board.sh` and
`driver/tools/kexec-zephyr/README.md`, whose lines moved (shifts at the end).

Since 2026-11-27 the GC7000UL results of stage 4 are measured on the
Armadillo-X2 (the Verdin i.MX8MP is off the farm). The SoC and the GPU are the
same; the way the image is entered, the memory around it and what the record
keeps are not. This note lists each difference, what it can do to a verdict
or to the end of a run, and how a tally classifies a result that it could
explain. Everything the X2 run does differently is in the crash-route runner
`tools/board.sh x2` (`kexec-dev` only, so every run is a development run:
`tools/board.sh:487-498`).

## The differences

**D1. The GPU is entered as Linux left it.** The X2 image is started by the
crash-route kexec out of Armadillo Base OS; galcore is built into that
kernel and "stops with the rest of Linux" (`driver/boards/atmark/armadillo_x2/
armadillo_x2_mimx8ml8_a53.dts:19-21`). No device shutdown runs on this route
(`driver/tools/kexec-zephyr/kexec_zephyr.c:372-377`). The Verdin's U-Boot
route never had a Linux GPU state; its kexec route has the same exposure, but
the campaign used U-Boot. The image's own bring-up then rewrites what it finds:
`power_on_marked` sets the four clock roots and two gates and powers GPUMIX and
GPU3D, and it records which it had to change (`roots_written`, `gates_opened`,
`was_down`, `adb_no_ack`: `driver/rust/src/gpu/vivante/power.rs:80-90,
121-163`). **The report is thrown away**: `yd_gc_power_on` returns `Ok(_) => 0`
(`driver/rust/src/ffi/vivante.rs:59-62`). So a run cannot say whether Linux
left the GPU running.
*Effect:* a hang or a wrong result in the first case or first entry after the
jump that a second jump does not repeat.
*Tally:* "X2 state", unproven until a power-report line exists (a candidate
item: log the report, optionally a cold-start option).

**D2. Clocks and power domains.** Both boards use the same registers and the
same devicetree figure, 800 MHz (X2 dts `:76-87`; Verdin
`driver/boards/toradex/verdin_imx8mp/verdin_imx8mp_mimx8ml8_a53.dts:89-99`),
and both bring-ups write the same roots, so a difference is the state Linux
left (D1), not the code. The root and gate writes are conditional
(`ccm::set_root` and `gate_on` return whether they wrote), so a root Linux
already set to another source or rate can survive only if the write is judged
unneeded; that comparison is not in this note's reach.
*Effect:* timing, or a hang at an unusual clock.
*Tally:* as D1; it is the same unlogged state.

**D3. Memory map, and the 256 MiB window.** The image's RAM is the same 256
MiB at `0x60000000` on both (`armadillo_x2...a53.dts:58-60`; Verdin dts
`:45-46`), and the same 160 MiB dEQP heap (`driver/samples/zephyr/deqp/boards/
armadillo_x2_mimx8ml8_a53.conf:48`; Verdin conf from `:37`). The X2 window is a
`crashkernel=256M@0x60000000` reservation inside Linux's own range
(X2 dts `:8-13`), checked by `board.sh` against `LOAD_END=0x70000000`
(`tools/board.sh:496`). What differs is the GPU carveout: 128 MiB at
`0x80000000`, galcore's `gcContMem`, no-map for Linux (X2 dts `:14-21,68-71`),
against the Verdin's 64 MiB at `0x58000000` (Verdin dts `:10,58-61`). The kernel
virtual space is the 512 MiB the conf sets (`...a53.conf:50-54`).
*Effect:* an out-of-memory outcome (`GL_OUT_OF_MEMORY`, a heap `bad_alloc`
that ends the process with a non-Pass and no FAULT record) differs between the
boards for the same case.
*Tally:* a board difference, not a GL or compiler fix. A case that fails with
out of memory on one board and passes on the other is classed D3 and listed
once; it does not start a driver issue.

**D4. Cache and MMU.** Entry is the same on both routes: the arm64 kexec path
stops the other CPUs, turns the MMU and caches off and jumps with x0..x3 = 0,
"the same state U-Boot's `go` gives" (`kexec_zephyr.c:10-12`). The code shows
no difference. The one thing it cannot show is what Linux's last panic path
leaves in the caches; it is not measurable from the tree.
*Effect:* none found.
*Tally:* never a stated cause. Only a result that reproduces on one board and
not on a second run of it may be put here, and then as "unexplained board
difference", not as D4.

**D5. The watchdog, and what the run waits for.** The image arms WDOG1 at 60 s
and feeds it for `CONFIG_GC_BOARD_WDOG_SECONDS=600` of uptime, then stops, so
a hung image is reset within 60 s after that (`driver/zephyr/lib/board/imx8mp/
gc_wdog_arm.c:15-25,63-75`; X2 conf `:24-27`, Verdin conf `:23-24`: the same).
The Verdin's U-Boot route bounded a run at `RUN_BOUND` 600
(`tools/board.sh:273-277`, header). The X2 route waits up to 1500 s after the
jump for the ping to be lost, to return and for ssh (`tools/board.sh:2543-2551`
`kexec_wait_back`; `run_kexec` `RUN_BOUND` 1500, `RETURN_BOUND` 600, `:2295`).
The one clean X2 jump came back 26 s after the jump with U-Boot reporting
"reset cause: watchdog", but the image reset the SoC itself, so **WDOG1
expiring on a hung X2 image has not been seen**; the proof image
(`CONFIG_GC_BOARD_WDOG_PROOF`) was never jumped
(`driver/tools/kexec-zephyr/README.md:421-429`). The loader checks no
watchdog on this SoC (`kexec_zephyr.c:23-24`).
*Effect:* a hang ends, if the watchdog works, about 600-660 s after the jump
plus Linux's boot, inside the 1500 s cap; if it does not, the board stays
hung, the run ends at 1500 s with no return, and the record holds the last
write.
*Tally:* a hang end is "watchdog end" only after the first X2 hang has proved
WDOG1; until then a no-return is "X2 watchdog unproven", never a GPU-driver
cause by itself. The first hang of the campaign is that proof.

**D6. The three-word record.** U-Boot keeps its boot count in LPGPR0, so the X2
image stores w0..w2 in LPGPR1..3 and drops w3 (`driver/boards/atmark/
armadillo_x2/Kconfig.defconfig:19`; `deqp_snvs.c:14-22`; README `:356-383`;
`tools/board.sh:1427`, "the X2 stores three words"). What is lost: the tally's
cases begun and the first failing name check (`deqp_snvs.c:255-259`), the
DEQP_CASES GPU reset and timeout counters and the first failure, and the
DEQP_FAILS pages hold four names, not six (`deqp_snvs.c:95-98,207-222`). So a
row's record cannot say that a case began and never ended, and "no resets"
is not stated.
*Effect:* a hang, a crash or a reset is invisible in the counts.
*Tally:* two checks stand in for the missing word (derived from the code, not
run):
1. *Sequence byte.* Past 32 cases the tally record is rewritten at every case
   begin and every case end, each with `++s_seq` in w0 (`deqp_snvs.c:226-227,
   238-259`): so `seq = begun + ended`, `ended = pass + fail + nsup + other`
   from w1/w2, and `seq - 2*ended` is 0 on a clean end and 1 when a case began
   without an end (compare modulo 256; parity survives the wrap). Up to 32
   cases the tag byte of DEQP_CASES is `s_begun` itself (`:251`).
2. *Expected begun.* The per-entry count of cases of the entry comes from the
   mustpass list (`tools/cts/gles3-mustpass.txt`), so `begun` short of that
   count marks an early end.
Neither exists in a tool yet. Until one does, do them by hand, and treat a
"none = 0" (no resets, no timeouts) on an X2 row with extras as unproven, not
as zero. A row read without them is classed "X2 record, begun unknown".

**D7. The console.** The X2 dEQP image has its console on, UART2 on the
board's USB adapter (`...a53.conf:9-11,34`), and `tools/cts/deqp-uart-tally.py`
names the failing cases from it; the Verdin image keeps it off by default so
the SNVS record is the only result (Verdin conf `:7-11,28`). A console print
is slow work inside the case loop.
*Effect:* run time; no verdict.
*Tally:* the SNVS record stays the authority; console names and record agree
or the row is flagged.

## How a tally classifies a result

For each case or end in an X2 row:

1. If the host's HOST-ONLY not-pass list at `c29a5ffe` (`hgc29`) names the case,
   it is *host-predicted*; its cause is the cause-table row, and no X2
   difference is claimed.
2. Else, if the end is a no-return, a watchdog end, an out-of-memory text, a
   begun-without-result by the sequence check (D6) or a first-case-only
   failure that a second jump does not repeat, it is an *X2 difference*, named
   with its D number (D5, D3, D6, D1). It is never entered in the GPU-driver
   cause table, and an out-of-memory difference is a board difference, not a
   fix (D3).
3. Else it is *X2-board-only*: a candidate GPU-driver cause. It becomes
   "X2-only" only with the control of the same board at the older commit
   `9bc82915`: GL, Vivante and the compiler changed a lot between that and
   `c29a5ffe`, so the Verdin figures cannot be used as the control (the Verdin
   is away, and the code moved). A failure that the control also shows is
   not new at `c29a5ffe`; one that only the new commit shows is a code
   change, not the board.
4. A row with "none = 0" extras on a three-word record is unproven (D6), and
   no cause is drawn from the absence of a reset.

## Candidate work, not done here

- A power-report log (and an optional cold-start option) in
  `driver/rust/src/ffi/vivante.rs`: removes D1 and D2 from the unproven.
- A board item for the `9bc82915` X2 control of the cases only the X2 fails.
- The cold-GPU start on the X2 is not available: the direct boot
  (`docs/notes/x2-direct-boot.md`) has not been run.

## Line shifts at origin/main

The citations are at `c29a5ffe`. At origin/main `tools/board.sh` moved: the
header RUN_BOUND text 276 -> 306, the x2 block 487-498 -> 529-540,
`REC0` 1427 -> 1482, `run_kexec` 2295 -> 2350, `kexec_wait_back`
2543-2551 -> 2611-2619. `driver/tools/kexec-zephyr/README.md` moved 12 lines
down (356-383 -> 368-395, 421-429 -> 433-441). The rest is unchanged.
