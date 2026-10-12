<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (old PR #63, ccfarm `01a129a1-fafd-7219-805b-089e9f458ce7`, last commit `285f1a1c2`). Written on 2026-10-11 against the integration tip `e3efeaad`, not on main; the board runs it describes were made then and are not repeated here. The text is as written; only file paths are changed to main's layout. `gc-kick-diff.md` reaches main with PR #118; `imx8mp-plan1.md` is not on main (it stayed on the integration branch).
# i.MX8MP gl_test round 2: #8 4x probes decoded; #9/#25 measured without the instrument

Plan #20 item `imx8mp-gltest-r2`. Date: 2026-10-11. Source: integration tip
`e3efeaadccd4345a2634f960ebdc97f80aeef28e` (`e3efeaad`). Board: Verdin i.MX8M
Plus (GC7000), leased resource `verdin-imx8mp`, `BOARD_BOOT=uboot-mmc`. The
board has no UART: every result is the SNVS record `tools/board.sh` reads.
The runner printed `build 63d162f` on each run: that is `e3efeaad` plus this
document's own earlier commits, which change no build input.

## Summary

| what | result |
|---|---|
| #8, case 16 with the 4x probes, arms 1 and 9 | **check 4 `x4 inside` fails in both arms: got `0x00000000`, want `0xffff00ff`.** The decoded probes are identical in the two arms: inside, outside and edge pixel all `0x00000000`; the 64 KiB scratch has 0 non-zero dwords; `clear only` and `draw only` also have 0 non-zero dwords and empty maps. Neither the clear nor the draw writes the 4x scratch. Bit 3 changes nothing. |
| #9/#25, KICK_DIFF instrument | **not run.** Nothing in the tree calls it (see below), so there are no instrumented boots and no KICK_DIFF output. |
| #9/#25, same image without the instrument, 32 boots | case 10 failed in **4 of 32** boots (base-5, -15, -23, -26); case 2 in 0 of 32. |

## Build

The committed `tools/cts/imx8mp-gltest-build.sh` worked as it is
(`rc=0` on every build); no local workaround was needed and nothing was
changed in the tree. Each build needs `-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`, or
`tools/board.sh` refuses the image (`imx8mp-plan1.md`). The workspace and SDK
are the shared ones in `$CCFARM_DEPS_DIR`.

```sh
W=-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y
tools/cts/imx8mp-gltest-build.sh msaa1-e3efeaad 0 $W -DCONFIG_GC_GL_TEST_MSAA=y -DCONFIG_GC_GL_TEST_MSAA_BITS=1
tools/cts/imx8mp-gltest-build.sh msaa9-e3efeaad 0 $W -DCONFIG_GC_GL_TEST_MSAA=y -DCONFIG_GC_GL_TEST_MSAA_BITS=9
tools/cts/imx8mp-gltest-build.sh gc10-base-tip  0 '-DCONFIG_GC_GL_TEST_SWEEP_LISTS="1-6,8-10\;10\;2"' $W   # \; keeps the separators through cmake
```

The MSAA record code in `gl_test.c` and `main.c` (`gc-msaa-record.md`, which
had never compiled it) builds and runs.

## Run commands

`tools/board.sh` needs `BOARD_HOST` (the leased resource's ssh login),
`BOARD_BOOT=uboot-mmc` and `SEL`; a small wrapper outside the tree exported
them and called, one boot per call:

```sh
SEL=16                    tools/board.sh imx8mp run <msaa image> <rec> msaa<bits>-case   # the CASE record
SEL=$((0x1000 + 0x<tag>)) tools/board.sh imx8mp run <msaa image> <rec> msaa<bits>-f<tag> # one PERF page; tag f0..f4, f8..fd
SEL=512                   tools/board.sh imx8mp run gc10-base-tip/zephyr/zephyr.bin <rec> base-<i>
yakogl-tools record msaa <w0 w1 w2 w3 of each page run>
```

Process note: capturing the runner's output through a pipe held the pipe
open for about 14 minutes after each run (a process the runner leaves behind
keeps it); redirecting the output to a file made a boot take 1.5 to 2
minutes. Only timing was affected; the records are the runner's own.

## #8: case 16 with the 4x probes

Case records (`SEL=16`):

```
board.sh imx8mp msaa1-case rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00100402,00000000,ffff00ff back_after=34s uptime=25s
board.sh imx8mp msaa9-case rc=4 verdict=failed boot=uboot-mmc row boot_verdict=cycled result=record:CASE resets=absent record=594b010e,00100402,00000000,ffff00ff back_after=98s uptime=26s
```

Decoded: `CASE (1) seq 14: case 16 check 4 status 2 (check failed) got 0x00000000
want 0xffff00ff`, the same words as `imx8mp-plan1.md` step 3 at `b20e4d11`.
Checks 1 to 3 (4x config, `EGL_SAMPLES`, `GL_SAMPLES`) pass; check 4 is
`x4 inside`.

Page runs (`record=` words, all `rc=0 verdict=done result=record:PERF`; the
arm 9 pages equal the arm 1 pages word for word):

| page (tag) | what | w0,w1,w2,w3 (arm 1 = arm 9) |
|---|---|---|
| 0xf0 | inside / outside / edge pixel | `594b0cf0,00000000,00000000,00000000` |
| 0xf1 | taken / scratch bytes / map chunks | `594b0cf1,0000003f,00010000,01000100` |
| 0xf2 | scratch after the frame | `594b0cf2,00000000,ffffffff,00000000` |
| 0xf3 | `clear only` | `594b0cf3,00000000,ffffffff,00000000` |
| 0xf4 | `draw only` | `594b0cf4,00000000,ffffffff,00000000` |
| 0xf8..0xfa | clear-only map | `594b0cf8..fa,00000000,00000000,00000000` |
| 0xfb..0xfd | draw-only map | `594b0cfb..fd,00000000,00000000,00000000` |

(`ffffffff` in w2 is the page's "no non-zero dword" index.) Decoder output,
`yakogl-tools record msaa` on the arm 1 words (arm 9 identical):

```
gl_test:   x4: inside 0x00000000 outside 0x00000000 edge 0x00000000
gl_test:   x4 scratch: 65536 bytes, 0 non-zero dwords, first [4294967295] = 0x00000000
gl_test:   x4 clear only: 0 non-zero dwords of 16384, first [4294967295] = 0x00000000
gl_test:   x4 clear only map, 256 chunks of 256 bytes:      (all 4 rows of 64 chunks: '.')
gl_test:   x4 draw only: 0 non-zero dwords of 16384, first [4294967295] = 0x00000000
gl_test:   x4 draw only map, 256 chunks of 256 bytes:       (all 4 rows of 64 chunks: '.')
```

Page 0xf1 w1 = `0x3f`: all six probes were taken (pixels, scratch, clear
only, draw only, both maps). Check-4 outcome: **fail in both arms**.

Reading against the table in `gc-msaa-record.md` / HANDOFF:

- Arm 1 expected `inside` `0xff000000` with a non-zero scratch and a clear
  map in the top-left quarter. Seen: a blank target and an **empty** scratch.
  Arm 9 expected a pass with a clear map over the whole scratch. Seen: the
  arm 1 result unchanged.
- The scratch is empty after the full frame, **and** `clear only` (the clear
  without the draw) and `draw only` also leave it empty. HANDOFF reads "empty
  scratch" as the draw (PE supertiling); here the clear leaves it empty too,
  so the point is wider: nothing the frame does reaches the 4x scratch that
  the probes read. That is consistent with a wrong scratch address or
  placement in the probe, a PE that is not given the 4x surface, or a
  write-back that never lands. These probes cannot tell which; they rule out
  "the resolve reads a good scratch and mis-tiles it" (the scratch is not
  good) and "bit 3 fixes the scale" (arm 9 = arm 1).
- Not run: arms 11 and 13 (only if arm 9 passes).

## #9/#25: instrument not run, measurement without it

`tools/cts/log/investigations/gc-kick-diff.md` leaves the harness call to this item,
and no code makes it: `grep -ri kick_diff` over the C, header, Kconfig, CMake
and conf files finds nothing; only `src/vivante/kick.rs` and
`tests/gc_kick_diff.rs` use it. Running it needs:

1. `driver/tools/pvrlab/gl_test.c`: arm before cases 10 and 2, call
   `gc_kick_diff_after_fence` after `glFinish`, keep `gc_kick_diff_stats`.
2. `driver/samples/zephyr/gc_gl_test/src/main.c`: a record for the stats (a
   PERF page chosen by `SEL`, or the sweep's spare words). The board has no
   console, so the `glog!` lines `KICK_DIFF stable|late` have no sink.
3. A Kconfig/CMake switch that builds `yakogl` with `test-knobs` for the
   gl_test image only; `imx8mp-gltest-build.sh` passes none.

This item may change no other file, so none of it was done. To have data, the
`gc10-base-tip` image (lists `1-6,8-10`, `10`, `2`; `SEL=512` = the first
list, cases 1-6 and 8-10) was booted 32 times. Pass word `0x77e` = all nine
cases passed, `0x37e` = case 10 (#9) failed, `0x77a` = case 2 (#25) failed.
Every run had w1 = 0 (no lost work, resets or fence timeouts) and rc=0.

| boot | w3 | case 10 | case 2 |
|---|---|---|---|
| base-1 to base-4 | `0000077e` | pass | pass |
| base-5 | `0000037e` | **FAIL** | pass |
| base-6 to base-14 | `0000077e` | pass | pass |
| base-15 | `0000037e` | **FAIL** | pass |
| base-16 to base-22 | `0000077e` | pass | pass |
| base-23 | `0000037e` | **FAIL** | pass |
| base-24, base-25 | `0000077e` | pass | pass |
| base-26 | `0000037e` | **FAIL** | pass |
| base-27 to base-32 | `0000077e` | pass | pass |

Verdict lines of the four failing boots (all runs have this shape; the
passing ones differ only in w3 and the timings):

```
board.sh imx8mp base-5  rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=100s uptime=19s
board.sh imx8mp base-15 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=102s uptime=21s
board.sh imx8mp base-23 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=101s uptime=19s
board.sh imx8mp base-26 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000037e back_after=98s uptime=29s
```

The 28 passing boots, for example:
`board.sh imx8mp base-1 rc=0 verdict=done boot=uboot-mmc row boot_verdict=cycled result=record:SWEEP resets=0 record=594b07ff,00000000,0000ff00,0000077e back_after=38s uptime=18s`.

Three of the four failures have a short `uptime` (19 to 21 s; base-26 has
29 s) and so do 6 of the 28 passes; I did not find a relation and do not
claim one.

## Conclusions

- **#8:** still open, and now localised further. Case 16's 4x half fails at
  check 4 with the same words as before. The probes show an empty 4x scratch
  after the whole frame and after the clear alone, so the problem is before
  the resolve: the clear and the draw both fail to reach the surface the
  probes read, or the probes look in the wrong place. Bit 3 (arm 9) has no
  effect. Next step: check that the 4x colour surface address given to the
  PE and RS is the one the probe reads (`gc_debug_msaa` code path in
  `src/vivante`), then the supertile setup. The MSAA record channel itself
  works on the board.
- **#9/#25:** no KICK_DIFF data, so none of the hypotheses in
  `gc-kick-diff.md` / `gc-case10-flake.md` (late write vs lost write; the
  flush race a1; K1) is supported or ruled out. What the 32 boots add is the
  rate: **case 10 fails in 4 of 32 (12.5 %) at the tip, case 2 in 0 of 32.**
  That matches `b20e4d11` (4 of 40 base, `imx8mp-plan1.md`) and is far below
  the 14 of 30 of the older measurement, and below the 2 of 3 at `334d4e65`;
  at 12 % the 2-of-3 was a lucky draw, not a regression, and
  `c86adc9` / `cc78859` need no bisect on this evidence. A 10-boot arm
  separates 0 from 5 failures, not 1 from 2, so any A/B of K1 against base
  needs about 40 boots per arm at this rate.

## Left to do

1. A driver item wires KICK_DIFF (points 1 to 3 above); then 30 or more
   interleaved boots of base, base+kd, and, once K1 exists, k1 and k1+kd, at
   about 40 boots per arm.
2. An #8 item: where the 4x surface is allocated and addressed.
