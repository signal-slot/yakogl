<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# gc-msaa-record: case 16's 4x probes without a console (defect #8)

Date: 2026-10-10. Host and cross-build work only; no board run was made.

**Status (2026-10-12): built, not run on a board.** Re-landed on main from
integration (`2e96561bd`, ccfarm `01a125bb-a6db-72c4-9778-3da9418c9e98`).
On main the C side was compiled at last: the i.MX8MP `gc_gl_test` image
(default; with `CONFIG_GC_GL_TEST_MSAA=y` as the sweep image
`imx8mp-gltest-build.sh` makes, and as case 16 alone),
the AM62P `gl_test` and the Pi 4 `v3d_gl_test` images build with
`-DYAKOGL_WERROR=ON`. The "Verification" section below is the state at
integration. No board has produced a page yet.

**Next arms (2026-10-12): 1, 3, 17 and 19.** Round 2 (arms 1 and 9) left
the scratch empty. The host comparison against Mesa's etnaviv
(`gc-msaa-addresses.md`) found the probe reads the surface the PE and RS are
given, and that arms 1 and 9 lack three things Mesa writes for every 4x
target on this core: the extra PS input and temporary and PE_LOGIC_OP
UNK24 = 5 (bit 1), and SINGLE_BUFFER 2 for the supertiled scratch (new bit 4).
Bits 2 and 3 are not Mesa's. The build and run commands for the four arms
are in `gc-msaa-addresses.md`, "The board A/B"; the pages are read as below.

## Why

The #8 A/B (the 4x MSAA step, "the `gc_debug_msaa` bit 3
A/B") reads its verdict from console lines: `x4: inside / outside / edge`,
`x4 scratch`, `x4 clear only`, `x4 draw only` and their `map` lines. A board
session without a serial capture only gets the SNVS result record: one CASE
record, whose w2/w3 are got/want of the *first bad check*. That is not enough
to tell which half (clear, draw, resolve) failed. This item lets the record
carry the same readings.

## The channel

The i.MX8MP result sink is one 16-byte record in the SNVS (LPGPR0..3), the
last record written before the SoC reset is the one read
(`driver/rust/src/board/record.rs`, `tools/board.sh` step 8). It cannot hold a
map of 256 chunks, so the readings are **pages of three words**, and a run
ends with one page chosen by the selector that `tools/board.sh` already
writes into the cleared record (`SEL=<n>` -> w1 = `0x53450000 | n`). This is
the existing kind PERF convention (`GC_RESULT_PERF_SEL | tag`, gc_result.h:
"selector ... runs the application and ends with page `page` of its
measurement instead of its usual record; w0's low byte = the selector's low
byte; w1..w3 = the page"). No new record kind was added: kinds 16, 17 and 19
are reserved on other branches, and PERF's status is n/a, so `board.sh` reads
such a run as `done` and a decode of it is `kind=PERF`.

One run = one boot = one page. Case 16 is deterministic enough that the pages
of several boots are read together; the decoder keeps the last page of each
tag.

## Layout

Selector `SEL = 0x1000 | tag` (decimal 4096 + tag), tag 0xf0..0xfd. The
record is `594b<12> <tag>`: w0 = `0x594B0C00 | tag` (kind PERF = 12), w1..w3
= the page's words. Defined once, in `driver/tools/pvrlab/gl_test.c`
(`gl_test_msaa_page`, `MSAA_REC_*`); the decoder is
`tools/yakogl-tools/src/msaarec.rs`, and a test there reads the constants out
of gl_test.c so the two cannot drift.

| tag | w1 | w2 | w3 |
|---|---|---|---|
| 0xf0 | inside pixel | outside pixel | edge pixel (each 0xAABBGGRR as read back, the `x4: inside ...` words) |
| 0xf1 | what was taken: 0x01 pixels, 0x02 scratch, 0x04 clear only, 0x08 draw only, 0x10 clear map, 0x20 draw map | scratch bytes | clear-only map chunks (low 16) \| draw-only map chunks << 16 |
| 0xf2 | scratch after the frame: non-zero dwords | first non-zero index | its value |
| 0xf3 | `clear only`: non-zero dwords | first index | value |
| 0xf4 | `draw only`: non-zero dwords | first index | value |
| 0xf8..0xfa | clear-only map, words 0..2 / 3..5 / 6..7 (+ 0) | | |
| 0xfb..0xfd | draw-only map, the same | | |

A map is one bit per 256-byte chunk, chunk i at bit i % 32 of word i / 32,
8 words = the first 256 chunks, which is the whole 64 KiB scratch of the
64x64 4x target. The console prints up to 1024 chunks; if a build ever
returns more, the decoder says only the first 256 are in the record.

The probes exist only in builds with `GL_TEST_VIVANTE` + `GL_TEST_KNOBS` (the
scratch and the maps); the pixel page exists in every build, and the x4 half
only on a board that offers a 4x EGL config (`CONFIG_GC_GL_TEST_MSAA=y`).
The probe state is reset when the 4x half has read its pixels; a run
without the 4x half has no page (the image logs it and, in the non-sweep
image, falls back to the CASE record; in the sweep image the SWEEP record
stays the verdict).

Console output is unchanged: the same `pr_info` lines, the same order.

## Image changes

- `driver/tools/pvrlab/gl_test.c`: `s_msaa_rec`, filled in `case_msaa_at`
  (4x half only) and `msaa_print_map`; exported `gl_test_msaa_page()`.
- `driver/samples/zephyr/gc_gl_test/src/main.c`: `msaa_page_tag()` and the
  first lines of `report_result()`; in the sweep image (the one
  `imx8mp-gltest-build.sh` builds) the page selector also runs case 16 alone,
  so no index arithmetic is needed.

## Reading it after a board run

`tools/board.sh imx8mp run` prints one verdict line per run with
`record=<w0,w1,w2,w3>` (hex, comma separated), and keeps the run's files
(a console log, if there is one, holds `yk-record:` lines). Feed the decoder
the words of every run, or a 16-byte nvmem dump, or a log:

```sh
yakogl-tools record msaa 594b0cf0 ffff00ff ff000000 ff800080 \
                         594b0cf1 0000003f 00010000 00460046   # w0 w1 w2 w3 per run
yakogl-tools record msaa --nvmem <16-byte nvmem dump> ...
yakogl-tools record msaa --log <console log> ...               # yk-record: lines
```

Output, in the console's words, e.g.

```
gl_test:   x4: inside 0xffff00ff outside 0xff000000 edge 0xff800080
gl_test:   x4 scratch: 65536 bytes, 8192 non-zero dwords, first [16] = 0x0000ffff
gl_test:   x4 clear only: 0 non-zero dwords of 16384, first [0] = 0x00000000
gl_test:   x4 clear only map, 70 chunks of 256 bytes:
gl_test:        0 ................................................................
gl_test:   x4 draw only: ...
```

A page that was not read is printed as `record: ... not read`, never as
zeros; a `clear only`/`draw only` that the run did not take is
`record: x4 clear only: not taken`. `yakogl-tools record <w0..w3>` still
decodes a single page generically (kind PERF, "experiment 30 page 0").

## Arm 1 and arm 9 (#8, step 3) with this record

Build one image per arm as in #8 (first case 0, so the selector in the
sweep image is the page selector, not a case index):

```sh
for bits in 1 9; do
  $SW/tools/cts/imx8mp-gltest-build.sh msaa$bits-$HEAD8 0 \
    -DCONFIG_GC_GL_TEST_MSAA=y -DCONFIG_GC_GL_TEST_MSAA_BITS=$bits
done
```

Run the case's own record first (the PASS/FAIL verdict and the first bad
check; no selector is the CASE record: `SEL=16` in the sweep image), then the
pages. Pages 0xf0..0xf4 and 0xf8..0xfd are nine boots per arm; the first
five (0xf0..0xf4) are enough for the table below, the map pages only
when a quadrant pattern has to be read:

```sh
for bits in 1 9; do
  img=$IMX8MP_OUT/msaa$bits-$HEAD8/zephyr/zephyr.bin
  SEL=16 $SW/tools/board.sh imx8mp run $img <rec>/msaa msaa$bits-case
  for tag in f0 f1 f2 f3 f4 f8 f9 fa fb fc fd; do
    SEL=$((0x1000 + 0x$tag)) $SW/tools/board.sh imx8mp run $img <rec>/msaa msaa$bits-p$tag
  done
  # then: yakogl-tools record msaa <the record= words of that arm's page runs, w0 w1 w2 w3 each,
  # commas turned into spaces>
done
```

Compare with the table of #8:

| arm | read in `record msaa` | expected |
|---|---|---|
| 1 | `x4: inside` | 0xff000000; scratch non-zero; `clear only map` `#` in the top-left quarter only |
| 9 | `x4: inside / outside / edge` | 0xffff00ff / 0xff000000 / 0xff800080 (+-2); the clear map covers the whole scratch |

Arm 9 failing: blank target with a full scratch map -> resolve; empty
scratch -> draw; scrambled in 64x64 or 4x4 blocks -> supertile bits;
half or double scale -> window size or stride (#8). Arms 11 and 13 only
if arm 9 passes. A fence timeout or fault in a run gives no page: the record
is then the usual breadcrumb/FAULT/UNFINISHED, and `dmesg` on the Linux side
is next.

## Verification (2026-10-10)

- `yakogl-tools` tests: 105 pass, 5 of them the decoder's (hand-made
  records: full run, missing page, no scratch, last page wins, and the
  constants against gl_test.c). `cargo fmt --check` and `cargo clippy
  -p yakogl-tools --all-targets -D warnings` are clean.
- gl_test.c and the sample's main.c were **not compiled**: the Zephyr cross
  build (`imx8mp-gltest-build.sh`, workspace `zephyr-ws` + SDK 1.0.1 in
  `$CCFARM_DEPS_DIR`) and a plain `gcc -fsyntax-only` were refused by this
  worker's sandbox ("requires approval"), so the C change is unproven. The
  edits are small and local; the first board-session build of
  `imx8mp-gltest-r2` is the proof, and a build error there is this item's.
- `tools/check.sh --quick` did not finish here (the second run was killed,
  exit 137); the first run's failures were `cargo fmt` (fixed since),
  clippy warnings in msaarec.rs (fixed since) and one `board.sh dry-run
  matrix` check ("207 checks, 1 failed") in code this item does not touch;
  not shown to fail on the base commit.
