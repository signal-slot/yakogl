<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Bring-up record: STM32MP257F-DK, the first triangle

Opened 2026-11-12 from the host-side studies, main `fb51ce43` (plan #349,
roadmap stage 5, issues #7 and #1281; the pre-study's rows P6 and P7,
`docs/notes/pre-study-stm32mp257f-dk.md:902-903`). **No step below has run on
the board yet**: `tools/cts/RESULTS.md` has no DK row and `tools/cts/log/` has
no DK directory at this tip. Every value in this note is marked
*expected* (a host source gives it) or *derived* (a study's reasoning, cited);
a board item replaces a step's line in "Board verdicts" with the reading, and
the current results then move to `tools/cts/RESULTS.md` and
`tools/cts/STATUS.md` (that step waits for plan #336). This note keeps the
current state only — no per-session log (CLAUDE.md); the run records are the
runner's (`records/…/INDEX.txt`).

The port itself is settled by the pre-study: the same Rust Vivante kernel side
extends to this GPU (`driver/docs/vivante-kernel-rust.md`, "The STM32MP257F-DK's
GC8000", `:314-364`), the identity row is `DB_STM32MP257F`
(`driver/rust/src/gpu/vivante/hw.rs:107-135`), the power is SCMI over OP-TEE
then the RCC (`driver/rust/src/soc/stm32mp2/power.rs:8-16`, `rcc.rs:31-38`),
the board layer is `driver/zephyr/lib/board/stm32mp2/`, and the runner is
`tools/board-stm32mp2.sh`. What is open is only what the board must answer —
this note's steps are ordered to answer it.

## The run order

The pre-study's board order (sections 6 and 8, rows P6/P7): the return before
any GPU step (`docs/notes/pre-study-stm32mp257f-dk.md:775-779`), then power
and identity from our image (`:780-794`), then the old Vivante order
`gc_fence`, `gc_clear`, `gc_triangle`, then the GL step
(`:795-799`, after `docs/notes/imx8mp-gc-bringup.md:131-135`).

| step | image | runner call | done when |
| --- | --- | --- | --- |
| 1 return trial, ends | `mp2_hello` | `MP2_NO_WATCHDOG=1 tools/board-stm32mp2.sh run <img> <logdir> mp2_hello` | banner, `mp2: mp2_hello done: PSCI SYSTEM_RESET at <n> ms`, board back in ST's Linux: `verdict=done rc=0` |
| 2 return trial, hangs on purpose | `mp2_hello` with `CONFIG_MP2_HELLO_HANG=y` | `MP2_NO_WATCHDOG=1 … run --expect-watchdog <img> <logdir> mp2_hello_hang` | banner + hang line, no PSCI line, back over ssh: `verdict=watchdog_reset rc=0` |
| 3 layer + watchdog | `mp2_identify` | `… run … mp2_identify` | `gc_wdog_arm: armed …`, `mp2_identify: PASS: …`, `verdict=done` |
| 4 power + identity | `mp2_gpu` | `… run … mp2_gpu` | `mp2_gpu: verdict PASS model=0x8000 rev=0x6205 … intr=ok idle=ok` |
| 5 fence | `gc_fence` | `… run … gc_fence` | `gc_fence: fence 1 irqs 1 …`, `gc_fence: PASS` |
| 6 clear | `gc_clear` | `… run … gc_clear` | `gc_clear: 256x128 wrong pixels 0 …`, `gc_clear: PASS` |
| 7 triangle | `gc_triangle` | `… run … gc_triangle` | the five check lines, `gc_triangle: PASS (0 failing checks)` |
| 8 GL case 1 | `gl_test` with `CONFIG_GL_TEST_MP2_SET_HEAD=y` | `… run … gl_test_case1` | `gl_test: case 1 uniform: PASS`, `gl_test: all: 0 of 1 cases failed` |

`mp2_hello` carries no `gc_wdog_arm: armed` marker (no
`CONFIG_YAKOGL_STM32MP2_WDOG` in its `prj.conf`), so the runner refuses it
without `MP2_NO_WATCHDOG=1`, deliberately (`tools/board-stm32mp2.sh:448-461`,
marker `:191` = `driver/zephyr/lib/board/stm32mp2/stm32mp2_wdog.c:130`). BL2's
IWDG is what resets those runs (pre-study `:255-259`). Steps 3-8 images carry
the marker on their own (`app.conf` `CONFIG_YAKOGL_STM32MP2_WDOG=y`).

Setup once: `tools/board-stm32mp2.sh status`, then `install` (it prints the
exact undo before changing anything, `tools/board-stm32mp2.sh:61-70,712-715`);
`probe` runs the pre-checks without booting anything (`:15,875-877`). Every
boot is a one-shot that U-Boot clears before loading (`:30-51`): a boot that
does not return to ST's Linux cuts the board off until a person is there. The
U-Boot hook the runner installs touches only its own `yako_*` variables and
wraps `bootcmd` (`:31-39,214-217`); it never edits ST's `bootcmd` contents.
The start line is `go` after `dcache off; icache off` (`:215`), which is what
step 1's hand-over record reads (below). Before the return trial the
runner-safety-audit gaps are closed by `runner-fixes` (signal-safe undo,
first-use fixes); the board items run after it.

## Step 1-2: the return trial

Console shape (expected; `tools/probes/stm32mp257f_dk/mp2_hello/README.md:21-37`):
`MP2:A EL2`, `MP2:A1`, `MP2:B console up (USART2, polled)`, the
`mp2-handover:` line, the counter and watchdog lines, the Zephyr banner,
`mp2_hello: PASS: counted 5 s in … ms`, then the end line
`mp2: mp2_hello done: PSCI SYSTEM_RESET at … ms`
(`driver/zephyr/lib/board/stm32mp2/stm32mp2.c:156-158`; the runner's end
regex `tools/board-stm32mp2.sh:205`). After it TF-A resets the SoC and the
console shows TF-A, OP-TEE, U-Boot and ST's Linux again — the board answers
ssh from the new boot within `RUN_BOUND + RETURN_BOUND` (120 + 180 s; Linux is
back on the network about 35 s after a reset, #1281, `tools/board-stm32mp2.sh:99-102`).

The `mp2-handover:` line answers the pre-study's Q8 (hand-over state,
pre-study `:823`) and prints `CNTFRQ_EL0`, for which no reading from a CPU
exists yet (Q6, `:479-480`, `:821`). Its format is `stm32mp2.c:38-49`; with the
runner's `go` the expected values are *derived* from TF-A r3.1 / U-Boot r3.1:
EL2, SCTLR M/C/I = 0 (MMU and caches off), `x0 = 0` with `x1` non-null, `x2`
and `x3` indeterminate (`stm32mp2.c:43-45`, restated in commit `073db528`).
`cntfrq` is expected 40 MHz (board-read via U-Boot, expected for the CPU
register, pre-study `:479-480`, Q6 `:821`). `pc` is expected 0xa0000000, the
load address (`tools/board-stm32mp2.sh:189-190`).

Step 2 is the caught hang: the image prints
`mp2_hello: hanging on purpose with interrupts locked, the OP-TEE watchdog must reset`
(`tools/probes/stm32mp257f_dk/mp2_hello/src/main.c:34`) and spins with
interrupts locked, so the layer's 20 s lifetime timer cannot fire
(`stm32mp2.c:28-30,236-252`). BL2 armed the IWDG before hand-over and a hang
resets the board in about 32 s (pre-study `:255-258`); the runner's verdict is
`watchdog_reset` when the banner and the hang line are there, no PSCI line
follows, and Linux answers within `WDOG_BOUND` (120 s)
(`tools/board-stm32mp2.sh:108-121,544-552`). A `psci_reset_before_wdog`,
`no_hang` or `not_back` verdict means the board layer or the runner, not the
board, needs the fix. If the board does not come back, the runner prints
`NEEDS A PERSON` and never retries (`:136-139`): release the board and report
it (plan #322 rule).

## Step 3: the layer, OP-TEE's watchdog, the marker

`mp2_identify` arms the `arm,smc-wdt` (SMC fid 0xbc000000, timeout 32 s, a
PET every second until the 20 s lifetime;
`tools/probes/stm32mp257f_dk/mp2_identify/README.md:24-37`). Expected lines
(`README.md:43-62`), key ones:

- `gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0xbc000000, timeout 32 s, PET every 1000 ms until 20000 ms of uptime` (`stm32mp2_wdog.c:130-133`; the literal the runner checks in the image, `tools/board-stm32mp2.sh:85-88`);
- on refusal: `mp2: OP-TEE watchdog arming FAILED: <command> answered <code> …: resetting now` (`stm32mp2_wdog.c:123-129`) — that is a board-layer fix, not a verdict;
- `mp2_identify: PASS: watchdog armed (32 s), N PETs in … ms`.

Open question this step answers (*derived*, pre-study `:257-258`): Linux's
client calls INIT first and never ENABLE with 0; our probe sends SET_TIMEOUT
then ENABLE without INIT (`mp2_identify/README.md:29-34,67-70`). Whether
OP-TEE accepts that is the board's answer, and the point where the board layer
is fixed if it refuses.

## Step 4: power and identity (mp2_gpu)

Sequence and expected console shape:
`tools/probes/stm32mp257f_dk/mp2_gpu/README.md:10-28,59-93` (all
*expected*, nothing run). The order is galcore's, pinned by host tests
(`power.rs:8-16`: PERFORMANCE domain 1 level read/set/read-back, POWER domain
0 on/read-back, then `RCC_GPUCFGR` gate and reset, then `HI_IDLE_STATE`
(0x004); pre-study `:510-519`, `:605-617`). Lines to read:

- `mp2_gpu: optee smc 0xb2000009 … -> …`: the capability exchange; the reply
  is *derived* as 0x8074 from OP-TEE r3.1's source (pre-study `:540-552`), and
  this run is its confirming read;
- the session to the SCMI pseudo-TA (UUID a8cfe406-d4f5-4a2e-9f8d-a25dc754c099,
  invoke command 4, channel 0; pre-study `:577-604`): whether OP-TEE accepts a
  non-Linux client's `TEE_LOGIN_REE_KERNEL` is open (pre-study `:595-601`) —
  ACCESS_DENIED here is a finding, not a fault;
- every `scmi >` / `scmi <` line is logged before the request is sent, so a
  panic leaves the last request on the console (pre-study `:872-881`,
  `power.rs:24-26`);
- `mp2_gpu: rcc gpucfgr 0x00000002` (gate on, clocks running), `0x00000003`
  (reset held), back to `0x00000002` (`rcc.rs:31-38`; *open question*: the
  reset bit's polarity is unconfirmed from RM0457, `rcc.rs:18-21` — the run
  shows whether the reset then reaches idle);
- `mp2_gpu: identity …: GC8000 rev 0x6205 product 0x80003 …` and the field
  line: model 0x8000, revision 0x6205, product 0x80003, eco 0 were *read*
  under ST's Linux (#1281, pre-study `:132-133`); `streams=16` is the row's
  value (Mesa `st gc_feature_database.h` :4756 and Linux
  `etnaviv_hwdb.c:205`, cited as such at pre-study `:156`, `:379`;
  `hw.rs:110-112,128`); the **customer id has never been read from the board**:
  expected 0x15 from the row (pre-study `:134-135`, Q1 `:816`; the table
  does not match it: `hw.rs:108-109,120`), printed as `customer=0x…` — this run closes
  that part of Q1;
- `mp2_gpu: gpu intr_enbl wrote 0xffffffff read 0xffffffff, then 0`:
  `HI_INTR_ENBL` (0x014, `driver/rust/src/gpu/vivante/regs.rs:53`) must read
  back what was written — the i.MX8MP's aliveness instrument
  (pre-study `:786-792`, `imx8mp-gc-driver-notes.md:98`; the write and
  read-back are `hw.rs:431,435` — the pre-study's `hw.rs:379-383` moved at
  this tip);
- `mp2_gpu: reset: rcc 0x00000003 held, 0x00000002 released; idle 0x7fffffff after 1 reads`: the reset reaches idle (`mp2_gpu/README.md:86-89`);
- `mp2_gpu: verdict PASS model=0x8000 rev=0x6205 product=0x80003 eco=0x0 customer=0x… intr=ok idle=ok`.

Nothing at or above the GPU's offset 0x800 is read from the CPU — the crate
refuses it before the bus (`gc_power_mp2.c:19-20`; the finding it guards:
`driver/docs/vivante-kernel-rust.md:258-263`, two builds that read the
0x800+ state space rebooted the i.MX8MP). Wrong identity is refused with one
`gc_power: REFUSED …` line and a PSCI reset (rule `gc_power_mp2.c:14-15`,
line and reset `:54-59`).

## Step 5: gc_fence — the interrupt route and the fence

The run powers the GPU through the board layer (`driver/samples/zephyr/gc_fence/README.md:34-38`,
`gc_power_mp2.c:9-15`), then connects SPI 183 (INTID 215;
`gc_irq_mp2.c:41-51`, `app.overlay`). After the connect the image prints the
route line (`gc_irq_mp2.c:98-104`):

- **expected `route=ours`**: enable, priority and target read normally,
  because TF-A r3.1's BL31 leaves SPI 183 in group 1, non-secure
  (`plat/st/stm32mp2/bl31_plat_setup.c:151`, cited in commit `073db528`;
  `gc_irq_mp2.c:93-94`). The second line's `expect=1` can still print
  `mismatch`: from non-secure EL1 `GICD_IGROUPRn` may read RAZ, and then the
  pair is `mismatch` + `route=raz` (`gc_irq_mp2.c:11-13,76-90`, commit
  `073db528`'s message). `route=raz` (all of enable/prio/target read 0) means
  **the interrupt is not routed to us** — a group/routing problem in the
  firmware chain, the Q6 open part (pre-study `:821`), not a GPU fault;
  `dist-off`/`cpuif-off`/`disabled`/`other-cpu`/`masked` name the other ways
  (`gc_irq_mp2.c:76-90`).
- the fence itself: `gc_fence: fence 1 irqs 1 elapsed <us> us irq-latency <us> us (completed 1, wait 0)` then `gc_fence: PASS` (`gc_fence/README.md:44-47`). That `irqs 1` is trustworthy because only the interrupt handler moves the counter — a retired fence with `irqs 0` is one retired with no interrupt counted (`gc_fence/README.md:50-58`). A "no interrupt from the GPU" (`route=ours`, `pend=0` after a fence) vs "not routed" (`route=raz`) is exactly what the two lines tell apart (`gc_irq_mp2.c:17-19`).
- before the fence, reset and init print the `gc diag:` read-backs the
  galcore-init-diff study asked for (`hw::DIAG_REGS` in
  `driver/rust/src/gpu/vivante/hw.rs`; reads only, all below 0x800):
  `gc diag: reset-attempt n=<k> 004=<IDLE_STATE> 000=<CLOCK_CONTROL>` on
  every reset attempt, then `gc diag: after-reset 3a8=… 10c=…` and
  `gc diag: after-init 3a8=… 10c=…`. A fence that does not retire is read
  from them first: `3a8` bit 0 = 1 after reset or init is galcore's suspect 1
  (`_ResetGPU` releases the secure-AHB soft reset by writing bit 0 back to 0,
  and `gckHARDWARE_InitializeHardware` writes 0x3A8 = 0x2; our reset never
  clears bit 0 and our init keeps it), an attempt count above 1 or an
  `004` that is not idle shows a reset that settled late, and `10c` is the
  pulse eater galcore programs in both places.
- the idle loop's WAIT delay scales with the devicetree's assumed 800 MHz core clock, never read on this board (`app.overlay:8-13` comment; `driver/docs/vivante-kernel-rust.md:307-309`): latency only, not a fault.

## Step 6: gc_clear — the resolve engine

`gc_clear: 256x128 wrong pixels 0 (clear 1 0, clear 2 0, guard 0)` then
`gc_clear: PASS` (`gc_clear/README.md:46-51`): two clears with different
colours through the **resolve engine** (this family has no BLT engine —
`gcFEATURE_BIT_REG_BltEngine` 0 in both rows, `driver/samples/zephyr/gc_triangle/README.md:60-61`),
every pixel compared, two guard rows must keep their poison
(`driver/samples/zephyr/gc_clear/README.md:16-21`). This is the pre-study's "the first clear reads
back through the resolve engine" (pre-study `:802-803`).

## Step 7: gc_triangle — the three apexes

One Gouraud triangle, red top / green bottom-left / blue bottom-right, into a
256x128 linear target cleared to `0x00102030` with poisoned guard rows
(`gc_triangle/README.md:19-22`). The check is the i.MX8MP's three-apex
read-back (`docs/notes/imx8mp-gc-bringup.md:233-238` — the `gl_triangle_user`
lines: red apex, green left, blue right, window written by GPU, PASS;
pre-study `:803-804`), re-expressed as five named checks
(`gc_triangle/README.md:24-33`: red apex, green left, blue right, outside,
covered within 20% of the triangle's area). Expected shape
(`gc_triangle/README.md:97-106` — the pixels shown there "only show the
shape; they are not a measurement", `:121-122`):

```
gc_triangle: red apex   PASS (… ) = rrggbbaa
gc_triangle: green left PASS (…) = …
gc_triangle: blue right PASS (…) = …
gc_triangle: outside    PASS (background kept outside the box and in the guard rows)
gc_triangle: covered    PASS N pixels (8389..12583 expected)
gc_triangle: PASS (0 failing checks)
```

`gc_clear` passing while `gc_triangle` fails points at the 3D pipe
(`gc_triangle/README.md:13-15`). The record carries which checks failed
(`gc_triangle/README.md:125-134`). This run also exercises the 64 MiB `gpu-reserved`
carveout at 0xfa800000 with the CPU reading back GPU-written memory — the
coherence question Q14 (pre-study `:829`) has no source answer except this
read-back. `CONFIG_GC_TRIANGLE_LINEAR_OUT=y` adds a resolve-engine copy into
a 1280x720 linear scan-out-shaped buffer (`gc_triangle/README.md:54-91`): not needed for
the first run, useful groundwork for the display step (P8,
`docs/notes/stm32mp257f-dk-display.md`).

## Step 8: gl_test case 1 (the gc_gl_triangle step)

Build with `-DCONFIG_GL_TEST_MP2_SET_HEAD=y` so the run is case 1 alone, the
regression set's first case (`gl_test/README.md:141-162`; the set order starts
1, `driver/tools/pvrlab/runner/src/lib.rs:52-55`; the name "uniform",
`lib.rs:773`; the C body "case 1: fragment uniform",
`driver/tools/pvrlab/gl_test.c:1560` — the pre-study's `:1449` citation moved
at this tip). Expected lines: `gl_test: case 1 uniform: PASS` (line shape as
the runner's own tests print it, `lib.rs:901`) and the tally
`gl_test: all: 0 of 1 cases failed` (`gl_test/README.md:14-16`), with the
`yk-record:` line, kind CASE, on the console (`gl_test/README.md:164-171`). The
board conf raises the layer's lifetime to 100 s for gl_test
(`driver/samples/zephyr/gl_test/boards/stm32mp257f_dk_stm32mp257fxx_a35.conf:16`);
anything longer needs the runner's `RUN_BOUND` raised with it
(`gl_test/README.md:169-171`, `tools/board-stm32mp2.sh:206`). The ST profile's
expected differences (the line-width cap, Q11) are P9 work; case 1 is not a
line-width case (`docs/notes/pre-study-stm32mp257f-dk.md:826`; the
gl-regression-study's report is in the plan, not the tree).

Reading the run: the runner's verdict line
(`tools/board-stm32mp2.sh:122-133`) and `boot_verdict` (`:134-135`). The
`result=` field and the `MP2_HOLD_S` hold read the image's final line through
`result_of`, anchored at the line start: `<prog>: PASS: ` / `<prog>: FAIL: `
(`mp2_identify`, `mp2_hello`), `<prog>: PASS` / `<prog>: FAIL (...)` /
`<prog>: FAIL at step N: ...` (`gc_fence`, `gc_clear`, `gc_triangle`),
`mp2_gpu: verdict PASS|FAIL`, and `gl_test: all: N of M cases failed` (0 is
`pass:gl_test`, anything else `fail:gl_test`). Sub-lines (`gc_triangle:
linear: PASS ...`, `gl_test: case 1 ...: PASS`) are not results, so a
`gl_test` run that stops before its tally reads `result=none`. A FAIL turns
the verdict to `failed`.

## When it looks like a hardware fault: supply first

The DK's only supply is its USB-C cable (pre-study `:783-785` — "A result
that looks like a hardware fault is first checked against the supply"): a
brown-out can reboot or stall the board mid-run and reads like a GPU fault.
The discriminator is the boot you did not look at: **capture the console from
BL2**, which the runner already does — the capture starts before the reboot
(`tools/board-stm32mp2.sh:897-898`), so every record's `console.txt`/
`console.raw` holds the new boot's TF-A BL2 lines. ST's TF-A v2.10-r3.1 BL2
for stm32mp2 prints the SoC's reset reason at the start of every boot, as a
`NOTICE` (always on) after the board model and board info
(*confirmed by reading the source*; paths are relative to the tag
`v2.10-stm32mp-r3.1` of github.com/STMicroelectronics/arm-trusted-firmware):

```
Reset reason: <text> (0x<RCC_C1BOOTRSTSCLRR>).<boot>
```

- format: `NOTICE("Reset reason: %s (0x%x).%s\n", reason_str, rstsr, boot_str)`,
  `plat/st/stm32mp2/bl2_plat_setup.c:181-182`; called from BL2 at `:582`
  (function `print_reset_reason`, `:129-183`).
- `0x%x` is the raw `RCC_C1BOOTRSTSCLRR` (offset `0x420`,
  `include/drivers/st/stm32mp25_rcc.h:260`) read at `:132`; the bits are
  `:2289-2309`: `PORRSTF` 0, `BORRSTF` 1, `PADRSTF` 2, `HCSSRSTF` 3,
  `VCORERSTF` 4, `VCPURSTF` 5, `SYSC1RSTF` 6, `SYSC2RSTF` 7,
  `IWDG1SYSRSTF`..`IWDG5SYSRSTF` 8-12, `C1RSTF` 13, `STBYC1RSTF` 20,
  `D1STBYRSTF` 22. `IWDGXSYSRSTF` is the OR of the five IWDG flags (`:2311-2315`).
- `<boot>` is ` Cold boot.` when `RCC_BDCR.RTCCKEN` is 0, else ` Warm boot.`
  (`bl2_plat_setup.c:142-147`): a cold boot means the backup domain lost its
  RTC clock enable, which a real power loss does and a reset does not (an
  inference from the code, not checked on the board).
- the text is chosen in this order (`bl2_plat_setup.c:152-179`):

| `PADRSTF` | first flag found, in this order | text after `Reset reason: ` | class |
| --- | --- | --- | --- |
| set | `PORRSTF` | `Power-on reset (por_rstn)` (`:166`) | power-on / supply |
| set | `BORRSTF` | `Brownout reset (bor_rstn)` (`:168`) | brown-out / supply |
| set | `SYSC2RSTF` or `SYSC1RSTF` | `System reset (SYSRST)` (`:171`) | system reset |
| set | `HCSSRSTF` | `Clock failure on HSE` (`:173`) | clock |
| set | any `IWDGnSYSRSTF` | `IWDG system reset (iwdgX_out_rst)` (`:175`) | watchdog |
| set | none of those | `Pin reset from NRST` (`:177`) | pin reset |
| clear | `STBYC1RSTF` | `System exits from Standby for CA35` (`:154`) | not a reset |
| clear | `D1STBYRSTF` | `D1 domain exits from DStandby` (`:156`) | not a reset |
| clear | `VCPURSTF` | `System reset from VCPU monitor` (`:158`) | VCPU monitor |
| clear | `C1RSTF` | `CA35 reset by CM33 (C1RST)` (`:160`) | CM33 reset |
| clear | none | `Unidentified` (`:162`) | - |

  `VCORERSTF` is defined but has no text: with `PADRSTF` set and nothing
  else a VCORE reset reads `Pin reset from NRST` (read from the code; the
  board's value is unverified). The test order matters: with `POR` and `IWDG`
  both set the line says power-on.
- ST's U-Boot `v2023.10-stm32mp-r3.1` (github.com/STMicroelectronics/u-boot)
  prints **no** reset reason on stm32mp2: a search of `arch/arm/mach-stm32mp`,
  `board/st`, `cmd`, `drivers/watchdog` and `drivers/clk/stm32` for
  `reset reason`/`reset cause`/`RSTSCLRR`/`BOOTRST` finds nothing there (only
  other SoCs print `Reset cause:`, e.g. `arch/arm/mach-imx/cpu.c:245`); only
  the register header `include/stm32mp25_rcc.h` mentions the flags. The BL2
  line is the only source.
- BL2 arms IWDG1 for 32 s (`fdts/stm32mp257f-dk.dts:164-167`
  `timeout-sec = <32>`, `status = "okay"`; `stm32_iwdg_init`/`stm32_iwdg_refresh`
  at `plat/st/stm32mp2/bl2_plat_setup.c:536-540`), and a watchdog reset shows
  as the `IWDG system reset (iwdgX_out_rst)` text above. The `X` in the text
  is literal; only the raw word says which instance fired (bits 8-12).
  **Unverified**: whether OP-TEE's SMC watchdog timeout sets `IWDG1SYSRSTF` or
  `IWDG2SYSRSTF` (OP-TEE's sources are not in these two trees; only the
  `IWDG1`/`IWDG2` base-to-instance mapping is,
  `plat/st/stm32mp2/stm32mp2_private.c:646-656`). Either bit gives the same text.
- the PSCI reset our image issues: `SYSTEM_RESET` goes to `stm32_system_reset`
  (`plat/st/stm32mp2/stm32mp2_pm.c:1230-1239`, `INFO("BL31: System cold
  reset")`), which sends SCMI `SYSTEM_POWER_STATE_SET` with the cold-reset
  type (`plat/st/stm32mp2/scmi/scmi_common.c:527-535`); the SCMI server then
  resets the SoC. **Unverified**: which RCC flag that leaves (the server is
  not in these sources), so the next BL2 text may be `System reset (SYSRST)`
  or `Pin reset from NRST`; read it from the first return trial and record it
  here. It is not a power-on text unless the supply fell.

Only that line separates the causes:

| what happened | console signature |
| --- | --- |
| PSCI reset by the image (normal end) | BL2 `Reset reason: System reset (SYSRST) (0x...)` or `Pin reset from NRST` (**unverified** which; no power-on text); the end line `mp2: <why>: PSCI SYSTEM_RESET at <n> ms` immediately before the new boot (`stm32mp2.c:156-158`; the runner's `END_RE`, `board-stm32mp2.sh:205`) |
| lifetime reset (too long, not too long for the watchdog) | end line with `why = lifetime over` (`stm32mp2.c:236-239`) |
| fatal error | `mp2: fatal error <reason>`, then the end line (`stm32mp2.c:163-168`) |
| watchdog reset (a hang, or our own 32 s arm firing un-fed) | BL2 `Reset reason: IWDG system reset (iwdgX_out_rst) (0x...)`; no end line at all; the reset lands about 32 s after the stall — 32 s from BL2's IWDG, about 27 s after an OP-TEE panic (pre-study `:255-258`) |
| OP-TEE panic | the last logged request is the last line before the reset (every request is logged before it is sent, `power.rs:24-26`, pre-study `:872-881`); reset sooner, ~27 s |
| supply loss / brown-out | BL2 `Reset reason: Power-on reset (por_rstn) (0x...)` or `Reset reason: Brownout reset (bor_rstn) (0x...)`, usually with ` Cold boot.`; or the console shows no BL2 lines at all (board silent: `boot_verdict=not_started/no_lines`, `board-stm32mp2.sh:129-133`) |

So: read `console.txt` from the boot boundary backwards, find the end line or
its absence, then read BL2's line. A GPU step that fails twice, a board that
resets with no end line, or a record with no BL2 lines is checked against the
cable and the supply **before** any register-level theory — and before
blaming SCMI, the GIC or the GPU. The eMMC/microSD and partitions are guarded
by the runner (`dev_guard`, `board-stm32mp2.sh:341-354`); a supply fault is
the one failure the runner cannot undo for you.

## Board verdicts

Current state only; each board item replaces its row (verdict + date + the
runner record directory name) and folds the findings into the pre-study
(`prestudy-update`) and `tools/cts/` (after plan #336).

| step | board item | verdict |
| --- | --- | --- |
| 1 return trial, ends | return-trial | **not run yet** |
| 2 return trial, watchdog-caught hang | return-trial | **not run yet** |
| 3 layer + OP-TEE watchdog armed | board-smoke | **not run yet** |
| 4 power + identity (`mp2_gpu`) | gpu-identity | **not run yet** |
| 5 `gc_fence` | fence-clear | **not run yet** |
| 6 `gc_clear` | fence-clear | **not run yet** |
| 7 `gc_triangle` | triangle-run | **not run yet** |
| 8 `gl_test` case 1 (`gc_gl_triangle`) | gl-triangle-run | **not run yet** |

## What these runs answer

- Q1 customer id (step 4 prints it; expected 0x15, never read;
  pre-study `:134-136,816`);
- Q6 `CNTFRQ_EL0` from a CPU (steps 1-3 print `cntfrq=`; expected 40 MHz) and
  SPI 183's route to the A35 (step 5's `gc_irq` line) (pre-study `:479-480,821`);
- Q8 what U-Boot hands over at `go` (EL2, MMU/caches off, x0 = 0 with x1
  non-null; steps 1-4 print `mp2-handover:`) and whether the SMC interface to
  OP-TEE stays open (steps 3-4) (pre-study `:823`, `stm32mp2.c:43-45`);
- Q3 whether OP-TEE accepts our power requests without a panic: the
  capability reply (derived 0x8074), the SET_TIMEOUT-without-INIT question
  (step 3), the REE_KERNEL login question (step 4) (pre-study `:595-601,818`);
- the `RCC_GPUCFGR` reset bit's polarity, unconfirmed from RM0457
  (`rcc.rs:18-21`): step 4's reset-to-idle is the answer;
- Q14, the carveout's coherence with the A35 (step 7's CPU read-back of
  GPU-written memory; pre-study `:829`);
- whether the ST row behaves like the i.MX8MP's end to end — steps 5-8 are
  the fence, resolve-engine and 3D-pipe proof before any CTS row (pre-study
  `:801-807`).

Out of this note's order: the display (P8, LTDC → DSI → ADV7535 → HDMI) is
`docs/notes/stm32mp257f-dk-display.md` and its bring-up note; the gl_test
regression set and the dEQP-GLES2 sweep (P9, the Q11 line-width decision) and
sign-off through `tools/board.sh` (P10) follow after these eight steps.
