<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK P9: the gl_test and dEQP-GLES2 procedure, and the Q11 line-width decision

Roadmap item P9 (`docs/notes/pre-study-stm32mp257f-dk.md:979`): the DK's
`gl_test` regression set, then its dEQP-GLES2 sweep diffed against the
i.MX8MP's failures, and the line-width decision Q11 (`:890`). The DK's first
triangle has run on the board (`docs/notes/stm32mp257f-dk-bringup.md:696`,
2026-11-23). **Status: a procedure and a recommendation. No DK run of either
suite exists yet — `tools/cts/RESULTS.md` has no DK row — and the Q11 decision
stays a recommendation until the board test below has run.**

Citations are `file:line` on the tree this note was written on. The VK-GL-CTS
facts (section 4) are from the CTS source on its `main` branch, read for this
note; the tree holds no copy, and the CTS has no line numbers in what was
read, so they are named by function.

## 1. The baseline

| What | Where |
|---|---|
| The i.MX8MP dEQP-GLES2 row: 29 entries, re-run at `0fbe61a3`, 17,371 cases begun, **17,318 Pass, 0 Fail, 34 NotSupported, 19 other (Quality/Compatibility warning)** | `tools/cts/RESULTS.md:33` |
| The 34 NotSupported, accepted on all three boards: `clip_control` 8, `compressedtexsubimage2d_*` 9, `multisampled_render_to_texture.readpixels` 1, `multisample.*` 16 | `tools/cts/RESULTS.md:36-40`, `tools/cts/es2-accepted.txt` (70 lines; `num_samples_line` at `:60`) |
| The per-group i.MX8MP results, one row per entry, last column the failing names | `tools/cts/imx8mp-full-tree-29460f96.results.tsv` (header at `:1`; row 27, the `rasterization.*` row, at `:29`) |
| The case list | `tools/cts/gles2-mustpass.txt` (17,165 lines); the DK's groups in `tools/cts/mp2-deqp-gles2.txt` (83 groups, the i.MX8MP's split and order, header `:4-9`), what it leaves out in `tools/cts/mp2-deqp-gles2-exclude.txt` (21 names pending Q11: 17 wide-line draws at `:20-36`, the four range queries at `:37-40`), checked by `tools/cts/mp2-deqp-list-check.sh` (prints: 83 groups, 21 excluded, 17,144 cases) |
| The regression set of `gl_test` | `SET_ORDER`, `driver/tools/pvrlab/runner/src/lib.rs:52-55` (50 cases); the i.MX8MP's `gl_test` column of its row at `tools/cts/RESULTS.md:84`: 49 of 49 run, case 16 skipped, cases 10 and 17 nondeterministic |

The "failures" of the i.MX8MP are therefore not zero Fail in the row but the
`QualityWarning`/`CompatibilityWarning` set that `mp2-p9-diff.py` treats as
non-failing (`tools/cts/mp2-p9-diff.py:15-16`) plus the five `rasterization.*`
names row 27 of the TSV lists as "other"
(`interpolation.basic.{line_strip, line_strip_wide, lines}`,
`interpolation.projected.{lines, lines_wide}`; the same family in
`docs/es2-cts-failures.md:59`). A DK run is expected to match the i.MX8MP's
*list* unless a DK-specific part differs: the profile's limits are equal
(`tools/cts/mp2-deqp-gles2-exclude.txt:8-14`), and the one difference that
reaches GLES2 is `wideline_triangle_emu`, which is Q11.

## 2. How a run fits the DK's run model

Every DK run is a one-shot from the board's own bootloader that hands the
board back to ST's Linux: the board layer powers the GPU, arms OP-TEE's
watchdog and feeds it until `CONFIG_YAKOGL_STM32MP2_LIFETIME_S` (default 20 s,
`driver/zephyr/lib/board/stm32mp2/Kconfig:19-21`); `board_finish()` ends with
the record and a PSCI SYSTEM_RESET into ST's Linux, and Linux is back on the
network about 35 s after a reset (`tools/board-stm32mp2.sh:190-192`). A hang
with interrupts locked, where neither the lifetime timer nor PSCI can fire, is
reset by the watchdog in about 32 s (`tools/board-stm32mp2.sh:200-205`;
`docs/notes/stm32mp257f-dk-uboot-boot.md` section 9). The two suites have
different lifetimes and different run counts.

**gl_test: the regression set is one run.** The DK image takes
`GL_TEST_MP2_CASE` (default 7, the set; `driver/samples/zephyr/gl_test/Kconfig:171-173`)
minus `GL_TEST_MP2_SKIP` (default `"10"`: case 10 `prims`, whose 3 px wide
LINE_LOOP waits on Q11; `:180-187`); its board conf sets the lifetime to 100 s
(`driver/samples/zephyr/gl_test/boards/stm32mp257f_dk_stm32mp257fxx_a35.conf:16`)
and the runner waits `RUN_BOUND` (default 120 s, `tools/board-stm32mp2.sh:328`).
`GL_TEST_MP2_SET_HEAD` (`driver/samples/zephyr/gl_test/Kconfig:189-203`) is the
bring-up knob, one small case; `tools/cts/board-stage.sh stm32mp257f-dk common`
builds exactly that first-run image (case 1 alone, `tools/cts/README.md:367-372`)
and is not the sign-off build. Whether all 49 cases of the default set finish
inside 100 s is not known from main — time the first run, and if the set is cut,
raise the lifetime and `RUN_BOUND` together (the board conf's comment, `:13-16`).

**dEQP: one image, one patch slot per shard.**
`tools/cts/board-stage.sh stm32mp257f-dk deqp` builds the whole 29-row tree
image once (`tools/cts/README.md:375-384`: its own stage 1 `DEQP_STAGE1_DK`,
held to 34 MiB and to the `0xa0000000..0xb0000000` window). The DK has no
run-time selector; its image carries a patch slot
(`CONFIG_DEQP_CASE_SLOT=y`, `driver/samples/zephyr/deqp/boards/stm32mp257f_dk_stm32mp257fxx_a35.conf`,
lifetime 600 s at `:32`), which `tools/cts/mp2-deqp-slot.py` rewrites per run
(`--case-list`, `--filter` or `--selector`; the image prints
`deqp: case filter '<patterns>'` first, which `tools/cts/deqp-uart-tally.py:57`
reads; `driver/samples/zephyr/deqp/README.md:128-137`). No rebuild per shard.
The runner needs `RUN_BOUND=600` or more.

**The sweep** is `tools/cts/mp2-deqp-sweep.sh <image> <outdir>`
(header `:5-28`): it expands `mp2-deqp-gles2.txt` minus
`mp2-deqp-gles2-exclude.txt` to `cases.txt` (17,144 names), splits it with
`tools/cts/mp2-deqp-shards.py` (default budget 15,000 ms at a default
100 ms a case — conservative for a 600 s lifetime; run the first shard,
then re-plan with `--times` from the DK's own measured times), and per shard
writes the slot into `slot-NNNN.bin`, runs it, merges the run directory's
`console.txt` files into `shard-NNNN.log`, and ends with `mp2-p9-diff.py`
over all logs. A shard whose log already holds a `deqp: <Status> <case>` line
is skipped, and `--first`/`--last` bound the range: an interrupted sweep
resumes. A shard cut by the lifetime shows as cases with no result and the
diff reports them `missing`; split it and rerun.

**The runner** for every run is a one-shot of the board's own U-Boot; the
verdict line is a row, never DEV. The sweep's default `MP2_SWEEP_RUNNER` is
`tools/board-stm32mp2.sh` (stages the image on bootfs, arms
`yako_once`/`yako_file` in one environment write, reboots, verifies the arm is
gone, removes the image; `tools/board-stm32mp2.sh:171-201`). The typed route
that arms nothing in the stored environment is
`tools/board-stm32mp2-uboot.sh run <image> <logdir> [name]` (`:19`) —
`tools/board.sh stm32mp257f-dk run` sends it with `BOARD_BOOT=uboot-mmc`
(the default) or `uboot-tftp` (`tools/board.sh:20-22`, `:53-70`, `:313-328`;
`docs/notes/stm32mp257f-dk-uboot-boot.md`), and
`MP2_SWEEP_RUNNER="bash tools/board.sh stm32mp257f-dk"` runs the sweep on it.

## 3. Diffing the results

Both tools read UART logs; a DK run's log is what the runner's `<logdir>`
holds (the sweep already merges the consoles into the shard logs). Several
logs are merged and the last status of a case wins.

```sh
# dEQP-GLES2: every shard's log (or a results.tsv) against the i.MX8MP row
tools/cts/mp2-p9-diff.py <shard1.log> <shard2.log> ...
#   defaults: --baseline tools/cts/imx8mp-full-tree-29460f96.results.tsv
#             --accepted tools/cts/es2-accepted.txt
#             --mustpass tools/cts/gles2-mustpass.txt   (:91-93)
# gl_test: the regression set's log against the i.MX8MP cell of RESULTS.md
tools/cts/mp2-gltest-diff.py <gl_test.log> ...
#   defaults: --results tools/cts/RESULTS.md, --expected tools/cts/mp2-gltest-expected.txt  (:106-109)
```

`--report-only` prints the classes and exits 0 whatever they hold. Otherwise:

| Tool | Exit 0 | Exit 1 | Exit 2 |
|---|---|---|---|
| `mp2-p9-diff.py` | no `new` and no `missing` | `new` > 0 (fails on the DK only: a regression) or `missing` > 0 (a mustpass name of the run's filters with no result) | usage or input error |
| `mp2-gltest-diff.py` | no `new` and no `missing` | `new` > 0 (FAIL or TIMEOUT, not expected, not flaky on the i.MX8MP) or `missing` > 0 (a set case with no verdict) | usage or input error |

(`tools/cts/mp2-p9-diff.py:32-33`, `tools/cts/mp2-gltest-diff.py:30-31`; their
self-tests are `tools/cts/mp2_p9_diff_selftest.sh`,
`tools/cts/mp2_gltest_diff_selftest.sh`, `tools/cts/mp2_deqp_sweep_selftest.sh`.)
The last lines are `mp2-p9-diff: shared S, new N, fixed F, missing M` and
`mp2-gltest-diff: new N, flaky F, expected E, missing M`. `shared` is a failure
on both boards and is not a DK defect; `new` is the work list. A `results.tsv`
lists failing names only, so it cannot show `missing`.

`mp2-gltest-diff.py` has two more classes that matter here
(`tools/cts/mp2-gltest-diff.py:26-27`): `stale` (on the expected list but
passing on the DK) and `skipped` (skipped on the DK but not on the i.MX8MP, or
the other way). The default DK image skips case 10 by config while the i.MX8MP
runs it, so a default run reports case 10 under `skipped`, not `new`; the
expected file's entry for it (`tools/cts/mp2-gltest-expected.txt:9`) counts
when the set is run whole (`GL_TEST_MP2_SKIP=""`).

Reading `fixed` with Q11 in mind: a NotSupported counts as non-failing
(`tools/cts/mp2-p9-diff.py:15-16`), so under option A below the five
interpolation names that fail on the i.MX8MP would appear as `fixed` when the
DK reports them NotSupported. They are not fixed; read them as the
wide-line cases (section 4) before counting.

## 4. Q11: wide lines on the DK's WIDELINE_TRIANGLE_EMU core

### The facts in the tree

- The ST profile has `wideline_triangle_emu: true`
  (`src/vivante/profile.rs:152`, field documented `:61-64`) and
  `line_width_range: [1.0, 1.0]` (`:165`, with the comment that Mesa's range
  on an emulating core is kept "until a board test says what the wide-line
  registers do on this part"; the i.MX8MP's is `[1.0, 16.0]`, `:130`). Mesa
  reports 1.0 on such a core (Mesa `etnaviv_screen.c:366-368`, cited at
  `src/vivante/profile.rs:63`).
- **Nothing in `src/` reads that field** (the pins are
  `tests/vivante_profile.rs:33` and `:58`). The GL core hard-codes
  `GL_ALIASED_LINE_WIDTH_RANGE` as 1 to 16 (`src/gl/state.rs:1040-1044`; its
  test at `:2615` and `:2631`, and the report guards pin the same values in
  `tests/es2_report_guard.rs:433` and `tests/es3_caps.rs:574`) and
  `glLineWidth` stores any positive width unclamped (`:783-790`). (The
  pre-study still cites this as `:993-997`,
  `docs/notes/pre-study-stm32mp257f-dk.md:431`; it has moved.)
- The backend programs the wide-line registers on every draw, half the width:
  `GC_PA_LINE_WIDTH` (`src/vivante/backend.rs:1608`) and
  `GC_PA_WIDE_LINE_WIDTH0/1` (`:1676-1677`). Mesa's golden stream for the ST
  part writes `PA_LINE_WIDTH` 0.5, clamped, where the i.MX8MP writes 2.0
  (pre-study `:890`; `tools/vivante/README.md`, "Another part: the ST row"
  `:676`), so what the DK hardware draws for a programmed width above 1 is
  **unknown**: a board test.
- The DK build's wide-line items: case 10 `prims` (skipped by default,
  Kconfig `:180-187`), case 103 `maskdepth`, which draws at
  `glLineWidth(6.0f)` (`driver/tools/pvrlab/gl_test.c:23946`, `:23964`), and
  the flat ES 3.0 cases 231/233/235 — none of them in `SET_ORDER`; the image
  runs one asked for by number (`driver/samples/zephyr/gl_test/src/main_stm32mp2.c:24-26`).

### The dEQP-GLES2 wide-line cases

In `tools/cts/gles2-mustpass.txt` the cases that draw a line wider than 1 are
17 (all already listed "pending Q11" in
`tools/cts/mp2-deqp-gles2-exclude.txt:20-36`; the file lists 21 Q11 names, the
other four being the range queries at `:37-40`):

- `clipping.line.{long_wide_line_clip, wide_line_attrib_clip, wide_line_clip,
  wide_line_clip_viewport_center, wide_line_clip_viewport_corner,
  wide_line_z_clip, wide_line_z_clip_viewport_center,
  wide_line_z_clip_viewport_corner}` (mustpass `:122-129`, 8 names)
- `rasterization.interpolation.{basic,projected}.{line_loop_wide,
  line_strip_wide, lines_wide}` (block at `:3944-3957`, 6)
- `rasterization.primitives.{line_loop_wide, line_strip_wide, lines_wide}`
  (block at `:3963-3967`, 3)
- range queries, which draw nothing: `implementation_limits.aliased_line_width_range`
  (`:3576`) and `state_query.floats.aliased_line_width_range_{getboolean,
  getfloat,getinteger}` (`:14172-14174`). The `state_query` checks were not
  read for this note.

### What VK-GL-CTS checks (source read, `main`)

- `implementation_limits.aliased_line_width_range` is
  `LIMIT_CASE(..., GL_ALIASED_LINE_WIDTH_RANGE, FloatRange, FloatRange(1, 1))`
  in `es2fImplementationLimitTests.cpp`: the reported range must contain
  [1, 1]. **A reported [1, 1] passes**, as does 1 to 16.
- `rasterization.primitives.*_wide` and `interpolation.*_wide`
  (`es2fRasterizationTests.cpp`, `BaseLineCase`, `BaseRenderingCase::init`):
  the wide width is `s_wideSize` = 5.0 (explicit `5.0f` for the interpolation
  cases). `init` queries `GL_ALIASED_LINE_WIDTH_RANGE`; when 5.0 is outside it
  the case throws **`tcu::NotSupportedError`** ("Support for line width 5 is
  required.") and does not run. On the last iteration the case draws at the
  range's maximum. A multisampled wide-line comparison failure is a
  CompatibilityWarning, not a Fail.
- `clipping.line.*wide*` (`es2fClippingTests.cpp`): width 5.0, and
  `checkLineWidth` in `LineRenderTestCase::init` throws
  `NotSupportedError("Maximum line width is too low for this test")` when the
  range's maximum is below it. A multisampled failure is a
  CompatibilityWarning.

So the CTS treats a range below 5 as "not supported" and an *advertised* range
as a promise: with the present 1 to 16 the DK would **run** all 17 and Fail
them wherever the hardware draws a width-1 line.

### Option A: report the hardware's range from the profile

`GL_ALIASED_LINE_WIDTH_RANGE` reads `profile.line_width_range` (the ST row's
1 to 1) and `glLineWidth` clamps to the reported maximum so the backend never
programs what it reports as unavailable. Effect: the 17 cases become
NotSupported (the 34 NotSupported of section 1 grow to 51 on the DK); the four
range queries and `aliased_line_width_range` are unaffected by the CTS bound
quoted above (`[1,1]` contains `[1,1]`).

For: Mesa reports the same on this core, so the CTS result matches the
reference driver's; the change is a few lines in `src/gl/state.rs` and keeps
`line_width_range` as the single place a board test later moves; the CTS
accepts it (NotSupported is not a failure, `mp2-p9-diff.py` counts it as
non-failing); and the i.MX8MP stays `[1, 16]`, unchanged. Against: wide lines
are lost for applications on the DK; the 17 cases do not exercise the core.

### Option B: emulate wide lines

The backend draws a width-w line as a screen-space quad (two triangles) the
way the core's feature name says the driver must. Effect: the range can stay
at a real maximum and the 17 cases run; their pass or fail then depends on the
emulation matching dEQP's reference rasterization, including clipping at the
viewport and z, interpolated varyings (`wide_line_attrib_clip`,
`interpolation.*_wide`) and the line loop's and strip's joins.

For: wide lines work. Against: a new geometry path in `src/vivante/**` for
`GL_LINES`, `LINE_STRIP` and `LINE_LOOP` (vertex expansion with varyings,
clipping, flat shading of the wide flat cases 231/233/235), no measurement
says it is needed, Mesa does not do it, and even the i.MX8MP's native path
fails five `rasterization.*` interpolation cases (section 1), so the reference
for "correct" is itself not pass-clean. It is a feature, not a conformance fix.

### Recommendation: A, with a board test first

Take **option A**, in two steps. (1) On the DK, before changing anything, run
the board test the pre-study asks for (`:890`): build gl_test with
`-DCONFIG_GL_TEST_MP2_CASE=10` (and `=103` for a second width) with the
present 1 to 16 range, and read the console's `expect_px` verdicts — case 10's
frame 2 draws the LINE_LOOP at `glLineWidth(3.0f)`
(`driver/tools/pvrlab/gl_test.c:4337-4348`) and checks
`expect_px("loop: 3 px wide", 17, 32, 0xff00ff00, 0)` (`:4357`), which the
clear colour fails if the hardware draws width 1. (2) If the hardware draws
width 1 (the expected reading of `PA_LINE_WIDTH` clamped to 0.5), switch the
range to the profile and clamp `glLineWidth`. If the registers do draw wide
lines, set the ST row's `line_width_range` to what the measurement shows and A
is "report the hardware's range" with a larger range; no emulation is needed
either way. Reasons: Mesa's reference reports 1.0 on this core; the CTS passes
it; it is the smaller change and does not hide the 17 cases (they are named in
the exclude file and will be counted as NotSupported, not dropped); B is only
worth its cost if a product needs wide lines on the DK, which no item asks
for. The decision stays a recommendation until step (1) has run.

### gl_test expectations that change under A

- **Case 10 `prims`** (`SET_ORDER`, `driver/tools/pvrlab/runner/src/lib.rs:52`;
  `case_prims` at `driver/tools/pvrlab/gl_test.c:4242`): once step (2) lands,
  the image's `GL_TEST_MP2_SKIP="10"` can go away only if the case reads
  `GL_ALIASED_LINE_WIDTH_RANGE` and skips that one check at a range maximum of
  1, as `es3flat_wide_width` does (below). Until then the skip stays and
  `tools/cts/mp2-gltest-expected.txt:9` covers the full-set run.
- **Cases 231, 233, 235** (`flat_lines_wide`, `flat_line_strip_wide`,
  `flat_line_loop_wide`; `driver/tools/pvrlab/gl_test_es3_flat.inc:213, 219,
  228`) already adapt: `es3flat_wide_width` clamps 5 to the reported range
  (`:141-152`), and the header says a range maximum of 1 runs the case at
  width 1 (`:43-46`). Under A they should **pass at width 1** when asked for
  by number, and their entries in `tools/cts/mp2-gltest-expected.txt:11, :13,
  :15` become stale (the diff will say so under `stale`, which is why it
  reports them).
- **Cases 230, 232, 234** (`flat_lines`, `flat_line_strip`, `flat_line_loop`)
  draw at width 1 (`driver/tools/pvrlab/gl_test_es3_flat.inc:210, 216, 225`):
  their "pending Q11" entries in `mp2-gltest-expected.txt:10, :12, :14` have
  no wide-line reason in the source and should be dropped whatever A decides.
- Case 127 `lines` (`case_lines`, `driver/tools/pvrlab/gl_test.c:13725-13794`)
  and the flat cases' header comment (`driver/tools/pvrlab/gl_test.c:485-490`)
  do not draw above width 1.

Under B the expectations are the opposite: 10 and 231/233/235 keep their wide
checks and a failure of any is a defect in the emulation.

## 5. What to take away

- Run order: gl_test regression set (section 2, 3), then the dEQP shards.
- The 17 wide-line names stay out of the first sweep (exclude file) and come
  back after step (1) of the recommendation.
- Not verified: the DK's hardware behaviour for wide lines; whether the 49
  default gl_test cases finish inside the 100 s lifetime; the time per dEQP
  shard on the DK; the `state_query.floats.aliased_line_width_range_*` checks.
- A finished DK row replaces the current text in `tools/cts/RESULTS.md` and
  `tools/cts/STATUS.md` (CLAUDE.md) and gets an issue update on #1281 and #7;
  the Q11 decision goes back into the pre-study's Q11 row.
