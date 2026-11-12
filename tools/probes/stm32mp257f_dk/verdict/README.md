<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dk_verdict: the DK's console log as P6/P7 verdicts

`dk_verdict.py` reads one USART2 console log of the STM32MP257F-DK (the
`console.txt` of `tools/board-stm32mp2.sh`, or any capture) and prints one
line per check. Python 3, standard library only. The board items
(return-trial, gpu-identity, fence-clear, triangle-run) quote its lines.

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
  if there is none.
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

`record` (samples only): the last `yk-record:` line must carry the magic
0x594b, kind 2 (APP) and status 1 (PASS) in the low byte of w1. A CHECK
record (kind 21) is a FAIL.

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
- `yk-record: %08x %08x %08x %08x` `driver/zephyr/lib/board/include/yakogl_record.h:21-22`
  (printed by `common/record.c:33`); kinds `yakogl_record.h` (APP 2, CHECK 21),
  status PASS 1 / FAIL 2 `:87-88`; the sample's result record is the verdict
  (`board.c:247-254`).

## Caveats

- `mp2_identify` prints no model/revision: only `mp2_gpu` does, so the plan's
  `CHIP_MODEL`/`CHIP_REV` check is `gpu-identity` on an mp2_gpu log.
- Q1/Q6/Q8 facts (CNTFRQ, customer id, what U-Boot hands over) are printed, not
  judged: read them off the `handover` and `gpu-identity` lines.
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
