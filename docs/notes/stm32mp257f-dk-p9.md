<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# STM32MP257F-DK P9: the gl_test and dEQP-GLES2 procedure, and the Q11 line-width decision

Roadmap item P9 (`docs/notes/pre-study-stm32mp257f-dk.md:979`): the DK's
`gl_test` regression set, then its dEQP-GLES2 sweep diffed against the
i.MX8MP's failures, and the line-width decision Q11 (`:890`). The DK's first
triangle has run on the board (`docs/notes/stm32mp257f-dk-bringup.md:696`,
2026-11-23). **Status: both suites and the Q11 board test have run on the board
(2026-11-24), all from origin/main `a5f1dc8e`, and the gl_test set again on
2026-11-25 from origin/main `670913e1`, timed; results in section 6. Sections
1-5 are the procedure as written beforehand; section 6 corrects what the runs
showed.**

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
diff reports them `missing`; split it and rerun. The split is
`tools/cts/mp2-group-resplit.py` (its docstring has the formats): it takes the
group's case list or the sweep's `manifest.tsv`, the times and the missing
list `tools/cts/mp2-case-times.py` writes, puts the case that was running when
the run ended in a shard of its own, and cuts the rest to fit the 600 s
lifetime and the 16,384-byte case slot. It writes shards in
`mp2-deqp-shards.py`'s format, numbered from `--first-index` past the shards
already run.

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
- **The range query is answered from the profile, but only the i.MX8MP row
  is ever built.** `glGetFloatv`/`glGetIntegerv(GL_ALIASED_LINE_WIDTH_RANGE)`
  returns `ALIASED_LINE_WIDTH_RANGE` (`src/gl/state.rs:1046-1051`), which is
  `crate::vivante::backend::LINE_WIDTH_RANGE` under `gpu-vivante` and
  `[1.0, 16.0]` otherwise (`src/gl/state.rs:197-202`).
  `LINE_WIDTH_RANGE = line_width_range_of(super::profile::ACTIVE)`
  (`src/vivante/backend.rs:1037`; the helper `:1030-1032`) and
  `ACTIVE = &GC7000UL_IMX8MP` (`src/vivante/profile.rs:172`, documented at
  `:17-19`: "nothing carries the GPU's identity to the backend at run time
  yet, so the ST profile is data and its test, not a path"). So the query
  returns `[1.0, 16.0]` for the i.MX8MP row (`profile.rs:130`) and would
  return `[1.0, 1.0]` for the ST row (`:165`). **No cfg, feature or Kconfig
  option makes a DK build pick the ST row:** `ACTIVE` is a plain constant,
  and `grep ACTIVE src/` finds no `cfg` around it (the other readers are
  `backend.rs:1027`, `:3987`, `src/vivante/state.rs:14-15`). The DK image
  therefore reports 1 to 16 today. `wideline_triangle_emu` itself is still
  read by nothing outside `profile.rs` and its tests.
- **`glLineWidth` does not clamp.** It rejects `width <= 0` with
  `GL_INVALID_VALUE` and stores any other width as given
  (`src/gl/state.rs:789-797`, the store at `:796`); `GL_LINE_WIDTH` reads it
  back (`:1031`). Nothing compares it with the reported range.
- **Tests that pin the values:** `tests/vivante_profile.rs:33` (ST row
  `[1.0, 1.0]`) and `:58` (i.MX8MP row `[1.0, 16.0]`);
  `tests/vivante_st_profile.rs:18-24` (`line_width_range_of` per row) and
  `:28-29` (`ACTIVE == GC7000UL_IMX8MP`, `LINE_WIDTH_RANGE == [1.0, 16.0]`);
  `tests/es2_report_guard.rs:433` and `tests/es3_caps.rs:574` (the GL core
  reports `[1.0, 16.0]`; `tests/es3_caps.rs:361-363` is the spec bound,
  min at most 1 and max at least 1); and in `src/gl/state.rs` the unit test
  `line_and_point_size_ranges` (`:2622`, `:2638`) and
  `line_width_must_be_positive` (`:2574-2587`, which stores 2.5 unclamped).
  The pre-study still cites the old location (`:993-997`,
  `docs/notes/pre-study-stm32mp257f-dk.md:431`); it has moved.
- **Option A in the tree now means two things only:** clamping `glLineWidth`
  to the reported range, and making the DK build select the ST row as
  `ACTIVE`; the profile-to-query path already exists.
- The backend programs the wide-line registers on every draw, half the width:
  `GC_PA_LINE_WIDTH` (`src/vivante/backend.rs:1618`) and
  `GC_PA_WIDE_LINE_WIDTH0/1` (`:1686-1687`). Mesa's golden stream for the ST
  part writes `PA_LINE_WIDTH` 0.5, clamped, where the i.MX8MP writes 2.0
  (pre-study `:890`; `tools/vivante/README.md`, "Another part: the ST row"
  `:676`), so what the DK hardware draws for a programmed width above 1 is
  **unknown**: a board test.
- The DK build's wide-line items: case 10 `prims` (skipped by default,
  Kconfig `:180-187`), case 103 `maskdepth` (`driver/tools/pvrlab/gl_test.c:24052`, dispatched
  `:24270`; the run showed case 103 draws no line, the `glLineWidth(6.0f)` is
  the panel demo at `gl_test.c:24666`, section 6), and
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

`GL_ALIASED_LINE_WIDTH_RANGE` already reads `profile.line_width_range`
(`src/gl/state.rs:197-202`, `src/vivante/backend.rs:1037`); with the ST row
`ACTIVE` it gives 1 to 1, and `glLineWidth` clamps to the reported maximum so the backend never
programs what it reports as unavailable. Effect: the 17 cases become
NotSupported (the 34 NotSupported of section 1 grow to 51 on the DK); the four
range queries and `aliased_line_width_range` are unaffected by the CTS bound
quoted above (`[1,1]` contains `[1,1]`).

For: Mesa reports the same on this core, so the CTS result matches the
reference driver's; the change is the clamp in `glLineWidth` (`src/gl/state.rs:789`) plus the DK build making the ST row `ACTIVE`, and it keeps
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
  by number, and their entries in `tools/cts/mp2-gltest-expected.txt:10, :11,
  :12` become stale (the diff will say so under `stale`, which is why it
  reports them).
- **Cases 230, 232, 234** (`flat_lines`, `flat_line_strip`, `flat_line_loop`)
  draw at width 1 (`driver/tools/pvrlab/gl_test_es3_flat.inc:210, 216, 225`):
  their "pending Q11" entries had no wide-line reason in the source and were
  dropped from `mp2-gltest-expected.txt` ("tools/cts: mp2-gltest-expected
  drops the width-1 flat cases 230, 232, 234").
- Case 127 `lines` (`case_lines`, `driver/tools/pvrlab/gl_test.c:13725-13794`)
  and the flat cases' header comment (`driver/tools/pvrlab/gl_test.c:485-490`)
  do not draw above width 1.

Under B the expectations are the opposite: 10 and 231/233/235 keep their wide
checks and a failure of any is a defect in the emulation.

## 5. What to take away

- Run order: gl_test regression set (section 2, 3), then the dEQP shards.
- The 17 wide-line names stay out of the first sweep (exclude file) and come
  back after step (1) of the recommendation.
- Not verified when this was written: the DK's hardware behaviour for wide
  lines; whether the 49 default gl_test cases finish inside the 100 s
  lifetime (answered in section 6: 14.4 s of 100 s for all 50); the time per
  dEQP shard on the DK; the `state_query.floats.aliased_line_width_range_*`
  checks.
- A finished DK row replaces the current text in `tools/cts/RESULTS.md` and
  `tools/cts/STATUS.md` (CLAUDE.md) and gets an issue update on #1281 and #7;
  the Q11 decision goes back into the pre-study's Q11 row.

## 6. Results on the board (2026-11-24, built from `a5f1dc8e`)

Every result below was built from `a5f1dc8e` (origin/main), except the
gl_test set rerun, which was built from origin/main `670913e1`. Records are
gitignored under `records/stm32mp257f-dk/` and are cited by name. Every run is
judged on the serial console. The console has no time stamps, so a run's time
comes from the image's own lines: `gl_test: the suite took <n> ms` and the
layer's `PSCI SYSTEM_RESET at <n> ms` of uptime. After the runs, `u-boot-env`
and `/boot` were back byte for byte.

### gl_test

The P9 gl_test result, like the dEQP sweep below, was built from `a5f1dc8e`
(records `20261124-1038-gl_test_set-a5f1dc8e` and
`20261124-1044-gl_test_all-a5f1dc8e`; diff line below, built from `a5f1dc8e`). The regression set (50 cases,
case 10 included) was rerun on 2026-11-25 from an origin/main `670913e1` image
(record `20261125-1325-gltest-set-670913e1`) as a confirmation: the result is
the same as the `a5f1dc8e` run. The `a5f1dc8e` diff line:

`mp2-gltest-diff: new 5, flaky 0, expected 0, missing 0` (back on Linux after 70 s of 100 s lifetime)

The rerun's result:
44 PASS, 5 FAIL (cases 6, 11, 47, 182, 189) and 1 SKIP (case 16). Case 10
passed, and its check 11 (`loop: 3 px wide`) is included in that pass.
`mp2-gltest-shards.py --skip "" --budget-ms 90000` plans one shard, and that
shard ran in one run:

`mp2-gltest-diff: new 5, flaky 0, expected 0, missing 0` (shard 0 of 1, 50 cases: 14.4 s of 100 s; `gl_test: the suite took 11431 ms`, `PSCI SYSTEM_RESET at 14407 ms`, no `mp2: lifetime` line, back on Linux after 70 s)

There was no hang or refusal in the Vivante back end: `draws refused 0`,
fences 173 of 173. The section 2 question is answered: the whole set fits
the 100 s lifetime with 85 s to spare, so the lifetime and `RUN_BOUND` stay as
they are. The per-case table and the failing checks are in the bring-up
note, "Board verdicts".

#### The set at `dd6cfa02` (2026-11-26, gltest-board)

main had no set log under `tools/cts/log/`, so plan #405 ran the shard
again, built from origin/main `dd6cfa02`. The log is in
`tools/cts/log/2026-11-mp2-gltest-p9/set/`. The plan is still 1 shard of 50
cases. It ran three times: the shard image (`GL_TEST_MP2_CASES` from
`shard-0000.conf`) twice, and the default-walk image once. The time
against `CONFIG_YAKOGL_STM32MP2_LIFETIME_S=100`:

| run | record | suite | PSCI reset at | of the lifetime | back on Linux | failed |
| --- | --- | --- | --- | --- | --- | --- |
| shard 0 of 1 | `20261126-0119-gltest_set-46b5cec3` | 11,415 ms | 14,404 ms | 14 % (no `mp2: lifetime` line) | 57 s | 3: 47, 182, 189 |
| shard 0 of 1, rerun | `20261126-0123-gltest_set_rerun-46b5cec3` | 11,419 ms | 14,408 ms | 14 % | 70 s | 5: 6, 11, 47, 182, 189 |
| default walk | `20261126-0124-gltest_set_default-f0b0e969` | 11,414 ms | 14,387 ms | 14 % | 57 s | 3: 47, 182, 189 |

`mp2-gltest-diff: new 3, flaky 0, expected 0, missing 0` (built from `dd6cfa02`, shard 0 of 1: 14.4 s of 100 s; new 47, 182, 189; stale 0)

The rerun's line was `mp2-gltest-diff: new 5, flaky 0, expected 0, missing 0`
(14.4 s of 100 s). Cases 6 and 11 were not fixed: the same `zephyr.bin`
failed them in the rerun. Each run lands in one of two states, and the wrong
pixels repeat exactly within each state. In state A, 6 and 11 pass and the
errors of 47, 182 and 189 are smaller. State B is the five failures of every
earlier run. The bring-up note, "Board verdicts", has the pixel values. The
cause is open, and it is not fixed here. There was no hang or refusal
(`draws refused 0`, fences 173 of 173).
`tools/cts/mp2-gltest-expected.txt` is unchanged: Q11 is decided (option
A), and the list has no stale entry.

The one-case image now prints its tally (bring-up note, step 8). Case 1
alone, built from origin/main `dd6cfa02` with
`-DCONFIG_GL_TEST_MP2_SET_HEAD=y`, record
`20261126-0118-gl_test_case1_tally-d3a5c59e`, log
`tools/cts/log/2026-11-mp2-gltest-p9/tally/`, gave `gl_test: all: 0 of 1
cases failed (1 passed; ...)` and `dk_verdict.py` `tally=PASS:0/1`. The PSCI
reset came at 3,312 ms of the 100 s lifetime (3.3 s of 100 s), and the board
was back on Linux after 60 s.

### dEQP-GLES2 sweep

Record `20261124-1857-deqp-gles2-a5f1dc8e`, built from `a5f1dc8e` with
`tools/cts/board-stage.sh stm32mp257f-dk deqp` (stage 1 from VK-GL-CTS
`93bca01`). 83 groups minus the 21 excluded names: 17,144 listed cases plus 223
extra cases the patterns also ran, 17,367 results: **Pass 15,790, Fail 1,508
(1,496 new, 12 shared), NotSupported 34, QualityWarning 35.**

`mp2-p9-diff: shared 12, new 1496, fixed 5, missing 0` (built from `a5f1dc8e`).

- Six group-prefix pattern shards (450 s at 150 ms a case), because a flat list
  does not fit the 16 KiB slot.
- Shard 5 ran 643 s, past the 600 s lifetime, and overflowed the 1 MiB console.
  It was re-split into 5a, 5b, 5c and a tail, with 5x as a rerun.
- Per run (Pass/Fail/NotSupported/QualityWarning): 0: 2812/0/8/0; 1:
  2890/68/26/3; 2: 2869/224/0/0; 3: 2481/460/0/0; 4: 2589/260/0/0; 5a:
  286/318/0/30; 5x: 142/178/0/0; 5b: 841/0/0/0; 5c: 865/0/0/0.

Failure classes (observed, not root-caused):

- (a) Shader groups, all new, mostly fragment (in `operator`, 375 fragment
  against 93 vertex). Fail counts: struct 3, builtin_variable 1, functions 13,
  constant_expressions 35, matrix 39, swizzles 45, conversions 9, indexing 44,
  random 345, binary_operator 153, unary_operator 110, common_functions 103,
  trig 22, geometric 16, exponential 29, selection 3, sequence 32.
- (b) Every colour-texture case fails: filtering.2d 136, cube 112, size 80,
  wrap 58, format 32, mipmap 78 Fail (66 new). Only a8 passes.
- (c) `fragment_ops.interaction`: 10 new failures.
- NotSupported: negative_api compressedtexsubimage2d 9, multisample 16,
  clip_control 8, msrtt 1.

### dEQP-GLES2 list-shard sweep at `66fcaa3a` (six slices, in progress)

The sweep is run again from a flat case list. It uses one image from origin/main
`66fcaa3a` for all six slices, `mp2-deqp-shards.py --budget-ms 12500`,
138 shards, a 600 s lifetime and `RUN_BOUND=700`. The case list, the shard
to slice map, the times and the shard logs are in
`tools/cts/log/2026-11-mp2-gles2-p9/`. The line below is
`mp2-p9-diff.py --report-only` over every slice that has landed so far
(`tools/cts/mp2-p9-notes-check.py` c3 compares the two). Each slice
replaces it.

- Slice 1 (shards 0..22, 2,875 cases, 2026-11-25): 2,855 Pass, 12 Fail,
  8 NotSupported (`clip_control.*`), no hang, no refusal, no run cut. The
  12 Fail are `fbo.render.color.mix_{,npot_}tex2d_{rgb,rgba,bgra}{,_depth_component16}`
  (shard 10), colour-texture class (b). Each run took 4.1 to 19.6 s of
  image time. Shard 13 is the exception at 159.5 s (`flush_finish.*`),
  26.6 % of the lifetime.
- Slice 2 (shards 23..45, 2,875 cases, 2026-11-26): 2,741 Pass, 105 Fail,
  26 NotSupported, 3 QualityWarning, every shard returned with all 125 of its
  cases: no hang, no refusal and no lifetime cut-off. The 26 NotSupported are
  `multisample.*` (16), `multisampled_render_to_texture.readpixels` (1) and
  `negative_api.texture.compressedtexsubimage2d_*` (9), the same names the
  i.MX8MP reports. The 3 QualityWarning are the `rasterization.interpolation`
  line names the i.MX8MP fails: they are the cumulative diff's 3 `fixed`. The
  105 Fail are all new against the i.MX8MP: class (a) 91
  (`constant_expressions` 40, `indexing` 27, `functions` 13, `conversions` 9,
  `algorithm` 1, `builtin_variable` 1) and class (c) 14
  (`fragment_ops.interaction.basic_shader`, shards 26 and 27). Each run took
  3.5 to 17.6 s of image time, at most 2.9 % of the lifetime.
- Slice 3 (shards 46..68, 2,875 cases, 2026-11-26): 2,718 Pass, 157 Fail,
  0 NotSupported, 0 QualityWarning, 0 other, every shard returned with all
  125 of its cases: no hang, no refusal and no lifetime cut-off, and at most
  3.8 % of the lifetime (4.7 to 23.0 s of image time). The 157 Fail are all
  class (a): `binary_operator` 70, `indexing` 43, `matrix` 30,
  `angle_and_trigonometry` 14.
- Slice 4 (shards 69..91, 2,875 cases, 2026-11-27): 2,614 Pass, 261 Fail,
  0 NotSupported, 0 QualityWarning, 0 other, every shard returned with all
  125 of its cases: no hang, no refusal and no lifetime cut-off, and at most
  3.2 % of the lifetime (4.4 to 19.1 s of image time, shard 77). The 261 Fail
  are all class (a): `common_functions` 65, `unary_operator` 58,
  `binary_operator` 32, `shaders.random` 46, `exponential` 22, `sequence` 21,
  `geometric` 14, `selection` 3 — 229 `*_fragment` and `random.all_features.fragment.*`
  cases, 30 vec3/vec4 `*_vertex` cases and 2 `random.basic_expression.combined`
  fragment cases.
- Slice 5 (shards 92..114, 2,875 cases, 2026-11-27): 2,497 Pass, 378 Fail,
  0 NotSupported, 0 QualityWarning, 0 other, every shard returned with all
  125 of its cases (`deqp: summary: 125 cases begun, 125 with a result, 0
  without`, `yakogl: draws refused 0` in all 23): no hang, no refusal and no
  lifetime cut-off, and at most 1.6 % of the lifetime (3.8 to 9.6 s of image
  time, shard 92). Shard 92 gave 49 Fail in its run, not compared with
  another run.

`mp2-p9-diff: shared 0, new 913, fixed 3, missing 0, excluded 21`

This line covers slices 1-5 (shards 0-114, image `66fcaa3a`) and is replaced as later slices land.
(The same tool over s5 alone reads `new 378, missing 14269`: slice 5's logs are
console excerpts with no `deqp: case filter` line, so the run's scope cannot be
narrowed to its cases. It is a scope artifact, not cases without a result: every
slice-5 log ends `125 cases begun, 125 with a result, 0 without`.)

Slice 5's 378 new Fails, triaged by the failure classes of the `a5f1dc8e`
sweep above ((a) fragment arithmetic, (b) colour-texture sampling, (c)
`fragment_ops.interaction`, and unclassified):

- class (a) 365: `shaders.random` 300 (`basic_expression` 73,
  `comparison_ops` 19 of which 7 `comparison_ops.vertex`, `conditionals` 32,
  `exponential` 34, `scalar_conversion` 68, `swizzle` 14, `trigonometric` 60;
  fragment and `combined` variants), `shaders.swizzles` 63 (`vector_swizzles`
  `mediump_ivec3`/`mediump_ivec4` `*_fragment`) and `shaders.struct` 2
  (`uniform.loop_nested_struct_array_fragment`,
  `uniform.dynamic_loop_nested_struct_array_fragment`, `Fail (Fail)`,
  shard 107);
- class (b) 13: `shaders.random.texture.fragment.*`;
- class (c) 0; unclassified 0.

Cumulative over slices 1-5: class (a) 874, class (b) 25 (slice 1's 12 `fbo`
and slice 5's 13 `shaders.random` texture cases), class (c) 14 (slice 2),
unclassified 0 — 913, the line's new count. The cumulative diff lists no
accepted name that changed state, and the cumulative is the number to read:
slice 1 alone listed 26 accepted names as `NotSupported -> NoResult` only
because slice 1 had not run them yet, and slice 2 ran all 26 as
`NotSupported`.

Against the forecast for slice 5 — the `a5f1dc8e` group counts minus what
slices 1-4 already found, for the three groups whose remaining cases sit in
slice 5's shards (`shaders.random` 345 − 46 = 299, `shaders.swizzles` 45,
`shaders.struct` 3: 347 in all) — slice 5 found 378, with the groups 10 or
more off being `shaders.random` 313 against 299 (+14) and `shaders.swizzles`
63 against 45 (+18); `shaders.struct` 2 against 3 is 1 under. In slices 1-5
as a whole these two groups are already over their `a5f1dc8e` counts (359
against 345, 63 against 45), while the groups fully run in slices 1-4 mostly
found less than forecast (`shaders.operator.binary_operator` 102 against 153,
`shaders.operator.unary_operator` 58 against 110,
`shaders.operator.common_functions` 65 against 103, `shaders.matrix` 30
against 39, `shaders.operator.sequence` 21 against 32) and some found more
(`shaders.indexing` 70 against 44, `shaders.constant_expressions` 40 against
35, `fragment_ops.interaction` 14 against 10, and 12 `fbo.render.color.mix`
and 1 `shaders.algorithm` name that the `a5f1dc8e` build passed). What the
forecast still owes belongs to slice 6, where the class (b)
texture groups sit whole: `texture.filtering.2d` 136, `texture.filtering.cube`
112, `texture.size` 80, `texture.wrap` 58, `texture.format` 32,
`texture.mipmap` 66 new (484 of the 1,496), plus 12 shared mipmap names.

### Q11: the measured line width

Q11 board runs, all at `a5f1dc8e`, each case alone with `GL_TEST_MP2_SKIP=""`, all pass:

- `20261124-1344-q11_case10-0eca6ffc`: 32/32. Check 11, `loop: 3 px wide (17,32) ... ok`: at width 3 the hardware draws at least 2 px, so a range of [1,1] is wrong.
- `20261124-1346-q11_case103-68d6cda3`: case 103 draws no line. The `glLineWidth(6.0f)` is the panel demo at `gl_test.c:24666`; section 4's claim that case 103 draws at width 6 is corrected.
- `20261124-1347-q11_case230-3a6f407d`, `20261124-1348-q11_case231-ac0e64b0`, `20261124-1349-q11_case232-527afbc8`, `20261124-1350-q11_case233-995cefe7`, `20261124-1351-q11_case234-4ca2280f`, `20261124-1352-q11_case235-21609b50`: the flat line cases 230-235; width 5 is at least 3 px wide.

The dEQP Q11 shard (17 wide-line names at widths 1..16): 15 Pass, 2
QualityWarning. The 4 range queries pass under either range.

The Q11 board test was repeated on 2026-11-25 with case 249 `linewidth`, which
reads back the drawn width. All six images were built from origin/main
`00cb1c8e` (`-DCONFIG_GL_TEST_MP2_CASE=<N>`, `GL_TEST_MP2_SKIP=""`). Each ran
alone as a one-shot from the DK's U-Boot and was judged on the serial console.
All six ended `result=pass:gl_test`, with `draws refused 0` and a PSCI reset
3.0 to 3.4 s into the 100 s lifetime. The board was back on Linux after 45 to
61 s.

- `20261125-1540-q11-be8802a9` (case 249): `GL_ALIASED_LINE_WIDTH_RANGE 1.00 16.00`;
  `req 1 h 1 v 1`, `req 2 h 2 v 2`, `req 3 h 3 v 3`, `req 4 h 4 v 4`,
  `req 5 h 5 v 5`, `req 6 h 6 v 6`, `req 8 h 8 v 8`, `req 16 h 16 v 16`. The
  horizontal and vertical runs equal the requested width at every width up to 16.
- `20261125-1551-q11_case10-46cd3ecb`: 32/32, check 11 `loop: 3 px wide (17,32) ... ok`.
- `20261125-1553-q11_case231-32c86ede` (27 checks), `20261125-1555-q11_case233-df07b287`
  (10) and `20261125-1557-q11_case235-e5f63fca` (12): every +-1 px check at width 5 is ok.
- `20261125-1558-q11_case103-8cc3364f`: the control, which draws no line, 2/2.

Q11 measured width: record `20261125-1540-q11-be8802a9`, a line drawn at glLineWidth 16 is 16 px wide, and at 1, 2, 3, 4, 5, 6 and 8 as wide as requested.

**Decision: option A (the profile's range), no emulation, with the range
[1, 16].** The ST row's `line_width_range: [1.0, 1.0]`
(`src/vivante/profile.rs:165`) is wrong for this hardware and should be the
i.MX8MP's `[1.0, 16.0]`. The DK image already reports 1 to 16 (`ACTIVE` is the
i.MX8MP row). Two things stay open, as a follow-up and not in this note: the
`glLineWidth` clamp (`src/gl/state.rs:789`) and the ST row as the DK build's
`ACTIVE`. At [1, 16], the 21 names in the exclude file are expected to run as
in the `a5f1dc8e` Q11 shard. They stay listed until that follow-up lands.

### Cause of the DK-only failures (source study, not yet board-confirmed)

A source study of `src/shaderc/online/vivante` and `src/vivante` against
Mesa's etnaviv, plus the arithmetic below, found a likely cause. Nothing here
was run on the board.

**The rule.** A literal encoded directly in an instruction source (an f20
immediate, `build::Src::imm_f32`, `src/shaderc/online/vivante/build.rs:127`:
the value's top 20 bits, so `0.5` is payload `0x3f000`) is read wrong in
lane z (blue) by about half the threads, in two-pixel columns. The payload
comes back with bits 8-10 set (`| 0x700`):

| written | payload | read as |
| --- | --- | --- |
| 0.5 | `0x3f000` | `0x3f700` = 0.9375 |
| 1.0 | `0x3f800` | `0x3ff00` = 1.875 |
| 8.0 | `0x41000` | `0x41700` = 15.0 |

On the i.MX8MP the same instructions read right, so the cause is the
GCNANOULTRA31_VIP2 core, not the compile. The earlier DK session
(`dk-ab-session`) ran with clock gating on and off and got the same wrong
values: clock gating is not the cause. That session also saw the
half-of-the-pixels, two-pixel-column pattern in dEQP-GLES2 fragment
arithmetic and in the colour-texture cases, which are the bulk of the 1,496
new Fail (failure classes (a) and (b) above).

**Each failing case against the rule.** The arithmetic uses the shaders in
`driver/tools/pvrlab/gl_test_shaders.h`; "got" and "want" are the checks in
the bring-up note, "Board verdicts". The pixel's byte order is
`0xAABBGGRR`, so `0xff5a3050` is R `0x50`, G `0x30`, B `0x5a`. "Matches"
below means only that the number follows from the rule; the compiled
instructions were not read.

- **189 `mulone`: matches, both checks.** Only blue is wrong; red, green
  and alpha are right.
  - Check 2 (`M * vec3(0.5)`): blue is `(u_b - u_q + u_x) * 0.5 =
    0.375 * 0.5 = 0.1875` (`0x30`). With the literal 0.5 read as 0.9375 it is
    `0.375 * 0.9375 = 0.3516`, `0x5a`: the board read `0x5a`.
  - Check 3 (the third column, `(u_b * 1.0) * u_q * 8.0`): `0.25 * 0.125 *
    8.0 = 0.25` (`0x40`). With 8.0 read as 15.0 it is `0.46875`, `0x78`: the
    board read `0x78`. (Both ratios are 1.875.)
  - Check 1 (`M * vec3(0.5, 0.5, 1.0)`) passes. The rule does not say
    whether it should: the study did not read the compiled blue lane, and a
    sampled pixel can fall in the half of the threads that read right.
- **6 `fbo` (checks 1 and 6) and 11 `persist` (check 2): consistent, not
  shown.** The apex pixel (31,12) reads blue `0x12` for a wanted `0x09`.
  The fragment shader is `o_color = v_color * vec4(v_uv, 1.0, 1.0)`, so
  blue is `v_color.b * 1.0`, a literal 1.0 in lane z. Red and green are
  right. A 1.0 read as 1.875 gives `0x09 * 1.875 = 16.9`, so `0x11` or `0x12`
  only if the unrounded blue is about 9.5 of 255 or more. The board read
  exactly `0x12`, twice the wanted value. That fits at the edge, and the
  study cannot say more without the compiled instruction. A 1.0 multiply may
  also have been folded away. Cases 6 and 11 pass in state A (the first
  `dd6cfa02` run and the default walk), which fits a fault that hits about
  half the threads, but it is not shown that this is the same fault.
- **47 `es3inst` (checks 4 and 9): a different stage.** Instance 3's colour
  is `icol * 0.5` with `icol = (1,1,1,1)` in the **vertex** shader, and
  blue is wrong (`0xb4`, `0xf1`; `0x93`, `0xa9` in state A; want `0x80`);
  red and green are right. A 0.5 read as 0.9375 gives at most `0xef`, and the
  varying is interpolated, so a vertex whose lane z reads wrong and one that
  reads right would give values between `0x80` and `0xef`. `0xb4`, `0x93`
  and `0xa9` fit that. `0xf1` (241) is 2 above `0xef` (239) and does not
  follow from the rule. At `670913e1` and `a5f1dc8e` alpha was right
  (`0x80`); in state B at `dd6cfa02` it reads `0x99` and `0xb5`, and alpha is
  lane w, not z. Partial match: the lane-z error fits; `0xf1` and the later
  alpha excess do not.
- **182 `fsat` (check 2): a different stage, partial.** The failing check is
  the vertex-shader quad; the fragment-shader quad passes. Blue is
  `max(min(u_a, 1.0), 0.0) * 0.5 + 0.25 = 0.75` (`0xbf`) and the board read
  `0xfa` (`0xfa` in state B, `0xd5` in state A). A literal 0.5 read as
  0.9375 gives a vertex value up to `1.0 * 0.9375 + 0.25 = 1.1875`, and an
  interpolated mix of that and 0.75 clamps near `0xfa`. So the direction
  fits and the size is not shown. At `a5f1dc8e` alpha was right; in state B at `dd6cfa02` it is
  `0x7e` for a wanted `0x73`. Alpha is lane w, so the rule does not explain it.

So the rule reproduces case 189 to the byte. For 6 and 11 it fits at the
edge. For 47 and 182 it explains the lane-z excess only if the wrong read
also happens in the vertex shader, which the study found only by arithmetic:
the "two-pixel columns" the dk-ab-session saw are a fragment-stage pattern.
The alpha excess in state B (47 and 182) is not explained, and it may be a
second fault or the same one in another lane. This is a likely cause, not a
shown one.

**Immediate sites to look at.** `literal_src`
(`src/shaderc/online/select.rs:2176`) already is the one place where a
`ConstF32` becomes an f20 immediate (`:2094`), so the gate goes there. The
other sites that build one directly, checked in the tree:

- `src/shaderc/online/select.rs:2265` (the zero, `Src::imm_f32(0.0)`),
  `:2586` (a constant integer `lod`, through `imm_f32`), `:2863` and `:2898`
  (the `imm` closures).
- `src/shaderc/online/vivante/builtins.rs:300` (`B2f`, the 1.0) and `:316`
  (`F2b`, the 0.0); `:960` (the lowering of a `ConstF32`). `:1080` is a test,
  not a site.
- Integer immediates (`imm_s20`, `imm_u20`, `build.rs:136` and `:144`) share
  the 20-bit encoding in a source field. Whether lane z reads them wrong is
  not known: the study found no failing case that uses one. They are listed
  for the confirming run, not claimed.

**The proposed fix.** A field in `src/vivante/profile.rs`'s `ChipProfile`
(say `reads_lane_z_literals`) says whether the core reads lane-z literal
sources right. It is `true` on the i.MX8MP row, so that output is unchanged,
and `false` on the ST row. `src/vivante/backend.rs` reads it and passes it to
the compile. When it is `false`, `literal_src` and the direct sites above
move the constant to a uniform (`const_file_src`) instead of an immediate.
Owners: plan #397 owns `src/vivante`, plan #400 owns `src/shaderc`. The DK
image still builds with the i.MX8MP row as `ACTIVE`
(`src/vivante/profile.rs:172`), so the fix reaches the DK only once the
`gpu-vivante-st` plumbing exists (`dk-st-profile-study`).

**The confirming run.** From a `gpu-vivante-st` image with the gate, run
gl_test cases 6, 11, 47, 182 and 189 alone (`GL_TEST_MP2_CASES`), then a few
dEQP-GLES2 fragment-arithmetic names (for example `dEQP-GLES2.functional.
shaders.operator.binary_operator.*` and `...filtering.2d.*` colour cases).
If the rule is right, 189 passes and so do the colour-texture and arithmetic
names; 47 and 182 will show whether the vertex stage is hit. Anything that
still fails (the state-B alpha excess is the first candidate) is a second
cause.
