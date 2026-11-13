<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dk_verdict: the DK's console log as P6/P7 verdicts

`dk_verdict.py` reads one USART2 console log of the STM32MP257F-DK (the
`console.txt` of `tools/board-stm32mp2.sh`, or any capture) and prints one
line per check. Python 3, standard library only. The board items
(return-trial, gpu-identity, fence-clear, triangle-run, gl-triangle-run)
quote its lines.

```
tools/probes/stm32mp257f_dk/verdict/dk_verdict.py [--expect CHECK,...] [--only CHECK,...] [LOGFILE]
tools/probes/stm32mp257f_dk/verdict/selftest.sh [--regen]
```

Without `LOGFILE` it reads stdin. Without flags a check runs when its program
shows in the log (so a `gc_fence` log does not fail `gpu-identity`).
`--expect` forces checks on top of that: a program missing from the log is
then a FAIL. `--only` runs exactly the named checks (the self-test uses it).

Output, e.g. `verdict return-trial banner=PASS psci-reset=PASS`, then
`N checks, M failed`. Exit 0: all PASS; 1: a FAIL, or no check applied
(an unknown log is never a pass); 2: usage. A field is `PASS`/`FAIL`, or
`PASS:<value>`/`FAIL:<value>` when the reading matters (`FAIL:absent` = the
line is not in the log), or a plain reading (`customer=0x...`, `el=2`).

## What is read

- The log from the last `MP2:A EL<n>` line on (the last run); the whole log
  if there is none. `supply` is the exception: it reads the whole capture,
  because BL2 prints its line before the image starts as well as after it.
- A leading runner stamp (`HH:MM:SS ` or `<secs>.<frac> `, the `plain()` of
  `tools/board-stm32mp2.sh:493`), CR and NUL are dropped, and so is a Zephyr
  log prefix (`[00:00:01.000,000] <inf> gpu_gc: `).
- Lines must start at column 0 after that.

## Checks and the lines they parse

| check | program | PASS needs |
|---|---|---|
| `return-trial` | mp2_hello, mp2_identify, mp2_gpu | `banner`: `MP2:A EL<n>`, `MP2:A1`, `MP2:B console up`, `mp2_*: build`; `psci-reset`: the end line, and none of `PSCI SYSTEM_RESET returned`, `fatal error`, `lifetime over` |
| `hang-trial` | mp2_hello built with `CONFIG_MP2_HELLO_HANG` | `banner`, `hang-line`, and after the hang line no end line, no `returned`, no `PASS:` (the OP-TEE watchdog did the reset) |
| `handover` | all | `fields`: `v=1` with el (1 or 2), m, c, i, x0, x1, pc, cntfrq (non-zero), mpidr and psci (non-zero) all numbers; the values are printed. `v=1 missing` FAILs |
| `identify-wdog` | mp2_identify | `armed`: the `gc_wdog_arm: armed` line and no `arming FAILED`; `pass-line`: `PASS: watchdog armed`, no `FAIL:` |
| `gpu-identity` | mp2_gpu | `model=0x8000`, `rev=0x6205`; `customer` is printed (Q6), not judged |
| `gpu-intr` | mp2_gpu | `readback`: wrote and read `0xffffffff`; `intr=ok` in the verdict line |
| `gpu-idle` | mp2_gpu | `reset-idle` line present (the idle word is shown), `idle=ok` in a `verdict PASS` line |
| `gc-irq` | gc_fence, gc_clear | after `irq <n> connected`, the `gc_irq:` line has `route=ours`; `route=raz` (or any other) FAILs and is named; `igroupr` shows `ok`/`mismatch` (`mismatch` FAILs) |
| `gc-fence` | gc_fence | `model` 0x8000/0x6205; `retired`: `wait 0`; `irq`: `irqs` > 0; `final`: `gc_fence: PASS` and no `gc_fence: FAIL`; `record` |
| `gc-clear` | gc_clear | `model`; `readback`: `wrong pixels 0`; `final`; `record` |
| `gc-triangle` | gc_triangle | `model`; `apex-red`, `apex-green`, `apex-blue`, `outside`, `covered` all PASS; `final`: `PASS (0 failing checks)`; `record`. Lines with a prefix (`linear: ...`) are not read; the sample's own `FAIL` lines still fail `final` |
| `gl-case1` | gl_test with `GL_TEST_MP2_SET_HEAD=y` (case 1, "uniform", alone) | `case1`: the last-run log has `gl_test: case 1 uniform: PASS` and no `FAIL`/`TIMEOUT` line for it (a `TIMEOUT` is named, an absent line is `FAIL:absent`); `tally`: the last `gl_test: all:` line is `0 of 1 cases failed` (the counts are shown, `0/1`); `final`: `gl_test: PASS` and no `gl_test: FAIL` line; `record`: kind CASE, status 1 |
| `supply` | BL2 (ST's TF-A, every boot in the capture) | every `Reset reason: <text> (0x<word>). Cold/Warm boot.` line, oldest first, as `boot1=... boot2=...`: `<PASS|FAIL>:<Warm|Cold>:<flags>`, the RCC flags decoded from the raw word in bit order (`None` when the word has none). `PORRSTF`, `BORRSTF` or a cold boot FAILs and names it (`ColdBoot` names a cold boot with neither flag); a PSCI (`System reset (SYSRST)`, `Pin reset from NRST`), watchdog (`IWDG system reset`) or standby boot is not supply evidence and PASSes with its decoded flags |

`record` (samples only): the last `yk-record:` line must carry the magic
0x594b, the check's kind - 2 (APP) for the `gc_*` samples, 1 (CASE) for
gl_test - and status 1 (PASS) in the low byte of w1. A CHECK record
(kind 21) is a FAIL.

## Line formats and where they come from

Probe lines (`tools/probes/stm32mp257f_dk/`):

- `MP2:A EL<n>` `stm32mp2_entry.S:116,130` (board layer
  `driver/zephyr/lib/board/stm32mp2/`); `MP2:A1` `stm32mp2.c:175`;
  `MP2:B console up (USART2, polled)` `stm32mp2.c:219`.
- `mp2-handover: v=1 el=.. sctlr=.. m= c= i= x0= x1= x2= x3= pc= cntfrq= mpidr= psci=`
  `stm32mp2.c:204`; `mp2-handover: v=1 missing` `stm32mp2.c:200`; the field
  list is `mp2_hello/README.md` ("What it prints").
- End line `mp2: <why>: PSCI SYSTEM_RESET at <n> ms` `stm32mp2.c:156`, which is
  also `END_RE` of the runner (`tools/board-stm32mp2.sh:219`);
  `mp2: PSCI SYSTEM_RESET returned <rc>: halted` `stm32mp2.c:158`;
  `mp2: fatal error <n>` `stm32mp2.c:166`; `lifetime over` is the `<why>` at
  `stm32mp2.c:239`.
- `mp2_hello: build ...` `mp2_hello/src/main.c:27`;
  `mp2_hello: hanging on purpose with interrupts locked, ...`
  `mp2_hello/src/main.c:34` (the runner greps the same text,
  `tools/board-stm32mp2.sh:555`); `mp2_hello: PASS: counted` `main.c:45`.
- `gc_wdog_arm: armed OP-TEE arm,smc-wdt fid 0x.., timeout <n> s, ...`
  `stm32mp2_wdog.c:130`; `mp2: OP-TEE watchdog arming FAILED: ...`
  `stm32mp2_wdog.c:123`; `mp2_identify: build` `mp2_identify/src/main.c:47`;
  `mp2_identify: PASS: watchdog armed (<n> s), <n> PETs in <n> ms`
  `mp2_identify/src/main.c:74`, `FAIL:` `:78`.
- mp2_gpu (`mp2_gpu/src/main.c`): `build` `:79`;
  `mp2_gpu: model=0x.. rev=0x.. product=0x.. eco=0x.. customer=0x.. date=..`
  `:116`; `mp2_gpu: reset: rcc .. held, .. released; idle 0x.. after <n> reads`
  `:139`; `mp2_gpu: verdict PASS model= rev= product= eco= customer= intr=ok idle=ok`
  `:143`; `mp2_gpu: verdict FAIL <what> rc=.. kind=.. step=.. a=.. b=..` `:62`
  (panic: `:55`).
- `mp2_gpu: gpu intr_enbl wrote 0x.. read 0x.., then 0` is a Rust log line
  (`driver/rust/src/soc/stm32mp2/gpu_ffi.rs:509`) that `mp2_gpu/src/main.c:49`
  prints with the `mp2_gpu: ` prefix; shown in `mp2_gpu/README.md` too.

Samples (`driver/samples/zephyr/<name>/src/main.c`, `NAME` = the directory):

- `irq <n> connected` is `LOG_INF` at `driver/drivers/gpu/gpu_vivante_gc_rust.c:419`,
  followed at once by `gc_irq: spi= intid= ... route=<ours|raz|dist-off|cpuif-off|disabled|other-cpu|masked>`
  (`gc_irq_mp2.c:98`; the route cases `:74-89`) and
  `gc_irq: igroupr=0x.. (GICD_IGROUPR<n>) bit23=<0|1> expect=1 <ok|mismatch>`
  (`gc_irq_mp2.c:103`).
- Every sample: `<NAME>: GPU model 0x.. revision 0x.. product 0x.. customer 0x.. eco <n>`
  (gc_fence `:92`, gc_clear `:152`, gc_triangle `:368`); `<NAME>: FAIL at step <n>: <rc>`
  (`:72`, `:77`, `:112`); the closing `<NAME>: PASS` / `FAIL rc=<n>`
  from `board_finish` (`driver/zephyr/lib/board/common/board.c:239,244`).
- gc_fence: `gc_fence: fence <n> irqs <n> elapsed <n> us irq-latency <n> us (completed <n>, wait <rc>)`
  `gc_fence/src/main.c:136`.
- gc_clear: `gc_clear: <W>x<H> wrong pixels <n> (clear 1 <n>, clear 2 <n>, guard <n>)`
  `gc_clear/src/main.c:201`.
- gc_triangle (`gc_triangle/src/main.c`): `gc_triangle: <what> <PASS|FAIL> (x,y) = rrggbbaa`
  `:167` with `red apex`, `green left`, `blue right` (`:192-200`);
  `outside     PASS|FAIL` `:237,242`; `covered     PASS|FAIL <n> pixels` `:251`;
  final `gc_triangle: PASS (0 failing checks)` `:444`, `FAIL (<n> failing checks)` `:439`.
- gl_test case lines (the walk is the pvrlab runner): `gl_test: case N <name>:
  PASS|FAIL|TIMEOUT` `driver/tools/pvrlab/runner/src/lib.rs:397-405` and the
  tally `gl_test: all: <fails> of <total> cases failed (…); precompiled hits
  <n>, compiler runs <n>` `lib.rs:411-424`, both printed through
  `gl_test_log`/`pr_info` (`gl_test.c:24564-24567`); the harness lines
  `gl_test: the first N case(s) of the regression set, from case N`
  `gl_test/src/main_stm32mp2.c:118`, `gl_test: case N on the STM32MP257F-DK's
  GPU (lifetime N ms)` `:121-122`, `gl_test: start (case=…)` `gl_test.c:24574`,
  `gl_test: the suite took <n> ms` `main_stm32mp2.c:144`; the sample's own
  `gl_test: FAIL: …` lines `:126,134`. The end line of a sample run is
  `mp2: board: run over: PSCI SYSTEM_RESET at <n> ms`
  (`driver/zephyr/lib/board/stm32mp2/board_stm32mp2.c:53` → `stm32mp2.c:156`).
- `yk-record: %08x %08x %08x %08x` `driver/zephyr/lib/board/include/yakogl_record.h:21-22`
  (printed by `common/record.c:33`); kinds `yakogl_record.h` (CASE 1 `:25`,
  APP 2, CHECK 21), status PASS 1 / FAIL 2 `:87-88`; the sample's result
  record is the verdict (`board.c:247-254`); gl_test writes its own kind CASE
  record in `report()` (`main_stm32mp2.c:71-93`, a single pass case with
  w1 = `case << 16 | 255 << 8 | 1`, CASE_PASS `yakogl_record.h:79`) and
  `board_finish` overwrites a result record only when the GPU was reset
  (`board.c:249`), so a clean gl_test run's last record is kind CASE.

Boot lines (the `supply` check; ST's TF-A v2.10-stm32mp-r3.1, captured from
before the reboot by the runner):

- `Reset reason: <text> (0x<raw RCC_C1BOOTRSTSCLRR>). Cold boot.` or
  `. Warm boot.`, printed as a `NOTICE` at the start of every boot
  (`plat/st/stm32mp2/bl2_plat_setup.c:181-182`, function
  `print_reset_reason` `:129-183`); Cold/Warm chosen by `RCC_BDCR.RTCCKEN`
  `:142-147`; the text chosen from the flags in the order `:152-179`; the
  flag bits `include/drivers/st/stm32mp25_rcc.h:2289-2309`
  (`PORRSTF` 0, `BORRSTF` 1, `PADRSTF` 2, `HCSSRSTF` 3, `VCORERSTF` 4,
  `VCPURSTF` 5, `SYSC1RSTF` 6, `SYSC2RSTF` 7, `IWDG1SYSRSTF`..`IWDG5SYSRSTF`
  8-12, `C1RSTF` 13, `STBYC1RSTF` 20, `D1STBYRSTF` 22). ST's U-Boot prints
  no reset reason on stm32mp2, so this is the only source. The meanings the
  check follows are `docs/notes/stm32mp257f-dk-bringup.md`, "When it looks
  like a hardware fault: supply first". The `NOTICE:  ` prefix is not
  stripped; the check searches inside the line.

## Caveats

- `mp2_identify` prints no model/revision: only `mp2_gpu` does, so the plan's
  `CHIP_MODEL`/`CHIP_REV` check is `gpu-identity` on an mp2_gpu log.
- Q1/Q6/Q8 facts (CNTFRQ, customer id, what U-Boot hands over) are printed, not
  judged: read them off the `handover` and `gpu-identity` lines.
- `supply` reads the whole capture, and fails every cold boot: a P6/P7 run
  never asks for a cold boot (per the bring-up note a PSCI or watchdog reset
  keeps the backup domain's RTC clock, so `Cold boot.` means power loss - and
  the board's only supply is its USB-C cable). A session that power-cycles
  on purpose (a baseline observation) is not read with this check. The check
  decodes the raw word only; it does not compare the decoded flags with the
  printed `<text>`.
- `gl-case1` requires `0 of 1`: a head of more than one case or the whole
  regression set is P9 work, not this check.
- The test logs in `testdata/` are synthetic, built from the formats above with
  placeholder values; a `.want` file next to each is the exact expected output.
  `selftest.sh --regen` rewrites them; read the diff before committing.
- A format change in a source above changes what this tool reads: update the
  tool, the logs and this file together.

## Self-test

`selftest.sh` runs each `testdata/<check>.<case>.log` (a pass and a fail case
per check, plus a runner-stamped log) against its `.want`, checks the exit
codes, stdin, auto mode and the unknown-log case, and ends with
`N checks, 0 failed`.
