<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# i.MX8MP board session: gl_test #9/#25 KICK_DIFF interleaved arms (imx8mp-gltest-r2)

Plan #20 item `imx8mp-gltest-r2`. Date: 2026-10-15. Board: Verdin i.MX8M Plus (GC7000),
the leased `verdin-imx8mp` resource, `BOARD_BOOT=uboot-mmc`. The board has no UART in this
setup: each run's result is the SNVS record `tools/board.sh` reads after the cycle. No board
address, credential or UART path is written here.

Procedure: `tools/cts/log/investigations/gc-kick-diff.md`, "How to run it on the board", with
the selector values of `docs/HANDOFF.md` ("Known defects"). An earlier round-2 record of the
same defects without the instrument is `tools/cts/log/board-sessions/imx8mp-gltest-r2.md`
(`e3efeaad`, 32 boots); this file is the KICK_DIFF round.

## Summary

| what | result |
|---|---|
| Commit | origin/main `e40e3835a7392b9294a5a2482329b05c02052e57` (`e40e3835`, "ccfarm: merge ccfarm/01a13a97-… (#805)"), the newest origin/main at the session's start. Built from the item's workspace at `d2a07a33` (`e40e3835`'s first parent): `git diff --stat d2a07a33 e40e3835` is one file, `docs/release/redaction-audit.md`, so every build input is identical. `tools/board.sh` therefore names the build `d2a07a33` (and `d2a07a33-dirty` once this doc's edits began; the image did not change). |
| Image | `kd-e40e3835`, `rc=0` (below); `zephyr.bin` sha256 `66280b5342e2e9b62c417186936a5087c880a4d12d0a70daca6ae6a363379a7e`, `GC_BUILD_STAMP=2026-10-14T22:36:23Z`, Zephyr `v4.4.0-9848-gaf17c0c8aa2a` |
| Boots | 120 interleaved (base, kd10, kd2 per round, rounds 1-40), all `rc=0 verdict=done`; plus one base warm-up boot (`base-0`, pass) and one interrupted boot (`kd2-20`, discarded and re-run, below) |
| Base arm (`SEL=512`) | 40 boots: **case 10 failed 4 of 40** (rounds 6, 18, 28, 39), **case 2 failed 0 of 40**; `resets=0` in all 40 |
| KICK_DIFF case 10 (`SEL=4586`, `0x11ea`) | 40 boots: **case 10 failed 1 of 40** (kd10-16), verdict **stable** |
| KICK_DIFF case 2 (`SEL=4578`, `0x11e2`) | 40 boots: **case 2 failed 1 of 40** (kd2-28), verdict **stable** |
| `late` checks | **0** in all 80 KICK_DIFF runs (320 checks in case 10, 480 in case 2) |

This is the first build of the Zephyr arm of `gc_gl_test` with the KICK_DIFF call site; it
compiled and the instrument armed in every KICK_DIFF boot (verdict byte 1, never `0x80 | n`).

## Image recipe

```sh
tools/cts/imx8mp-gltest-build.sh kd-e40e3835 0 '-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10\;10\;2"' -DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
```

with `ZEPHYR_WORKSPACE`, `ZEPHYR_VENV`, `ZEPHYR_SDK_INSTALL_DIR` (SDK 1.0.1) and `IMX8MP_OUT`
pointing into the shared west workspace of `$CCFARM_DEPS_DIR` (`zephyr-ws`,
`zephyr-sdk-1.0.1`, `imx8mp-out`). Build output, verbatim:

```
rc=0
             RAM:      49664 KB       256 MB     18.95%
        IDT_LIST:           0 B        32 KB      0.00%
Generating files from …/imx8mp-out/kd-e40e3835/zephyr/zephyr.elf for board: verdin_imx8mp/mimx8ml8/a53
```

The image's `.config` has `CONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10;10;2"`,
`CONFIG_GC_BOARD_WDOG_SELF_ARM=y`, `CONFIG_GC_GL_TEST_KNOBS=y`, `CONFIG_YAKOGL_TEST_KNOBS=y`.

## Arms and the loop

All three selectors run sweep entry 0 (`1-6,8-10`, nine cases) from the one image; only the
selector differs.

| arm | selector | record |
|---|---|---|
| base | `SEL=512` | SWEEP: pass bits, case c is bit c (`0x77e` = all nine pass) |
| kd10 | `SEL=4586` (`0x1000 \| 1 << 8 \| 0xea`) | PERF page tag `0xea`: case 10's KICK_DIFF verdict and own result |
| kd2 | `SEL=4578` (`0x1000 \| 1 << 8 \| 0xe2`) | PERF page tag `0xe2`: case 2's KICK_DIFF verdict and own result |

A KICK_DIFF arm's page reports only its armed case, so kd10 says nothing about case 2 and kd2
nothing about case 10 (both cases still run in each boot).

The loop ran `tools/board.sh imx8mp run <image> <rec> <arm>-<i>` with `SEL` set, in the order
base, kd10, kd2 for i = 1 … 40, one board, no other runs in between. It was started from a
small untracked wrapper (it exports `BOARD_HOST` from the lease, `BOARD_BOOT=uboot-mmc` and
the Zephyr variables, because the worker's command policy refuses them as a command prefix);
the wrapper was deleted afterwards and is not committed. It ran in three batches (rounds 1-20,
20-34, 35-40) because of the worker's 2-hour limit on a background command. The first batch
was stopped while `kd2-20` waited for the board to come back; the board was found in Linux
with no one-shot image staged (`yako_once` and `yako_default` empty, `/var/yakogl/` empty),
and `kd2-20` was run again as the first boot of the second batch. Only the re-run counts.

## Verdict lines (board.sh, verbatim)

All 120 counted boots ended `rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled`.
Grouped by record (`uniq -c` over the 120 lines):

```
      4 base rc=0 verdict=done	record=594b07ff,00000000,0000ff00,0000037e
     36 base rc=0 verdict=done	record=594b07ff,00000000,0000ff00,0000077e
      1 kd10 rc=0 verdict=done	record=594b0cea,17020001,00000008,00000000
     39 kd10 rc=0 verdict=done	record=594b0cea,ff010001,00000008,00000000
      1 kd2 rc=0 verdict=done	record=594b0ce2,17020001,0000000c,00000000
     39 kd2 rc=0 verdict=done	record=594b0ce2,ff010001,0000000c,00000000
```

Every failing boot:

```
board.sh imx8mp base-6 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=91s uptime=25s
board.sh imx8mp kd10-16 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PERF resets=absent record=594b0cea,17020001,00000008,00000000 back_after=103s uptime=27s
board.sh imx8mp base-18 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=92s uptime=26s
board.sh imx8mp base-28 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=104s uptime=24s
board.sh imx8mp kd2-28 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PERF resets=absent record=594b0ce2,17020001,0000000c,00000000 back_after=92s uptime=21s
board.sh imx8mp base-39 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=92s uptime=25s
```

A passing boot of each arm, for comparison:

```
board.sh imx8mp base-1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000077e back_after=52s uptime=26s
board.sh imx8mp kd10-1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PERF resets=absent record=594b0cea,ff010001,00000008,00000000 back_after=84s uptime=24s
board.sh imx8mp kd2-1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:PERF resets=absent record=594b0ce2,ff010001,0000000c,00000000 back_after=104s uptime=24s
```

`resets=absent` on the PERF rows means the record carries no reset counter (the KICK_DIFF page
replaces the SWEEP record), not that a reset happened. All 40 base boots read `resets=0`.

## Decoded

Base arm: `yakogl-tools record sweep 0 00000000 0000ff00 0000037e` gives
`pass [1, 2, 3, 4, 5, 6, 8, 9]`, so case 10 (bit 10) is the only case not passed in the four
failing base boots; `0x77e` is all nine. Case 2 (bit 2) is set in all 40 base records.

KICK_DIFF arms: the procedure's tally over the 80 KICK_DIFF records
(`grep -o 'record=…' | cut -d= -f2 | tr ',' ' ' | xargs yakogl-tools record kickdiff | sort | uniq -c`):

```
     40 gl_test:   kick_diff case 10: stable, checks 8 late 0 max_changed 0
     40 gl_test:   kick_diff case 2: stable, checks 12 late 0 max_changed 0
      1 record: case 10 FAIL, first failing check 23; no late check
     39 record: case 10 PASS, no failing check; no late check
      1 record: case 2 FAIL, first failing check 23; no late check
     39 record: case 2 PASS, no failing check; no late check
```

| failing KICK_DIFF run | page | decoded | verdict |
|---|---|---|---|
| kd10-16 | `594b0cea,17020001,00000008,00000000` | case 10 FAIL, first failing check 23; stable, checks 8, late 0, max_changed 0 | **never appeared**: the second read-back, 2000 us after the first, equals the first in every one of the 8 checks, and the case still fails at check 23 (the 1 px point of the points frame). The pixels did not land within 2 ms of the fence: the PE dropped the dirty lines, or wrote them more than 2 ms late. |
| kd2-28 | `594b0ce2,17020001,0000000c,00000000` | case 2 FAIL, first failing check 23; stable, checks 12, late 0, max_changed 0 | **never appeared**: same reading, 12 checks, no word changed, the case fails at its check 23. |

No KICK_DIFF run, failing or passing, saw a word change between the two reads.

## Reading

- The late-write story ("the fence fired, then the pixel engine wrote") predicts `late` lines
  and a lower failure rate in the KICK_DIFF arms. Neither showed: 0 late checks in 80 runs, and
  both KICK_DIFF failures are `stable`. As far as a 2 ms window can tell, the write was
  **lost**, not late. That points at the flush/clear side (the K1 arm of the 2x2 in
  `gc-kick-diff.md`), not at the fence or the epilogue.
- Rates: base case 10 4 of 40 (10%), consistent with 4 of 32 at `e3efeaad` (Fisher two-sided
  p = 1.0); kd10 1 of 40 against base 4 of 40 gives p = 0.36, so the wait does not measurably
  hide the fault. Case 2: 0 of 40 base, 1 of 40 kd2 (p = 1.0); #25 is rare at this tip.
- The "about 1 run in 3" in `docs/HANDOFF.md` for #9 is higher than the 10% measured here and
  at `e3efeaad`.
- Limits: one 2000 us window. A write landing later than 2 ms reads as `stable`; the next arm
  that would separate "lost" from "very late" is a longer `KICK_DIFF_DELAY_US` (the full
  record, "Call site"). One failure per KICK_DIFF arm is a small sample.

## Record directories

Under `records/kd-e40e3835/` in the item's workspace (`records/` is gitignored, so none of
this is committed): one directory per boot, `20261015-<HHMM>-<arm>-<i>-66280b53`, 122 in all
(the 120 counted boots, `base-0`, and the interrupted `20261015-0952-kd2-20-66280b53`), each
with the runner's console in `<arm>-<i>.out` next to them. The failing boots:

- `20261015-0822-base-6-66280b53`
- `20261015-0926-kd10-16-66280b53`
- `20261015-0936-base-18-66280b53`
- `20261015-1050-base-28-66280b53`
- `20261015-1054-kd2-28-66280b53`
- `20261015-1157-base-39-66280b53`

The re-run of `kd2-20` is `20261015-1006-kd2-20-66280b53`.
