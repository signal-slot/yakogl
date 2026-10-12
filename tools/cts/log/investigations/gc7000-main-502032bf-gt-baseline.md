<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# gc_gl_test on the i.MX8MP at main `502032bf`: what the baseline's failures and skips are

Written 2026-10-12 on the host for the PR #129 A/B (plan item `gc7000-baseline-triage`). No board
was used. Each statement is marked **[record]** (read from a board record or runner output),
**[source]** (read from the code at `502032bf`) or **[inferred]**.

## Read this first: the records were not readable

The main-side run's own records (`records/imx8mp/20261012-2010-main-offline-GT-502032bf/...` and
`...-SL-...`, gitignored, on the node that ran the board, workspace `01a12fb3...`) **could not be
read here**: `fetch_from_farm` refused the path ("only paths of node-local values, dependency
caches and export_paths are lent"), and the directory is outside this agent's allowed
directories. So **this file quotes no line of that run**. What it states about the run is:

- the figures the plan gives (verdict `failed`; cases 46 and 44 failed; 39 run; 11 skipped, the
  first skip being case 16), and
- everything the code at `502032bf` and the earlier records on this board say about what such a
  line must mean.

Where a statement is a prediction of what the record shows, it says so. The first thing the
board-session item should do with the record is check the "Record lines to expect" block below
against it, line by line.

What was read:

| Thing | Where | Commit / sha |
|---|---|---|
| gc_gl_test sources | `driver/tools/pvrlab/gl_test.c`, `driver/samples/zephyr/gc_gl_test/src/main.c` | `502032bf` (the build's worktree file is byte-equal to `git show 502032bf:...`) |
| image config | `imx8mp-out/main-502032bf-offline/GT/zephyr/.config` | the built image, offline table |
| runner | `tools/board.sh imx8mp`, `run-main-baseline.sh GT <stamp>` | |
| an earlier run on this board, same sample and SNVS decoder | `records/imx8mp/pr133-066a2db8/20261012-1907-gt-066a2db8-1-c30d8bd6/INDEX.txt` (PR #133 image `066a2db8`, 2026-10-12 19:07) | image sha256 `c30d8bd64de6...` |
| table analysis | `tools/cts/log/investigations/gc7000-table-pr129.md` on branch `ccfarm/01a12f0e-27b3-7647-90ad-0944dc6633d1` (commit `2e347714`) | main `2fe4b8798b5f` vs PR `419b0159d2ea` |
| known defects | `docs/HANDOFF.md`, `tools/cts/RESULTS.md` ("i.MX8MP: gl_test case 44 on `f2a13877` still fails"), `tools/cts/STATUS.md` | |

## 1. The image

`gc_gl_test`, `GC_GL_TEST_CASE=7` (the regression set), offline table, built by
`build-main-offline.sh` with `-DCONFIG_YAKOGL_SHADERC_ONLINE=n -DCONFIG_YAKOGL_SHADERC_OFFLINE=y
-DCONFIG_GC_BOARD_WDOG_SELF_ARM=y`. Its `.config` [record: the image's own `.config`]:

```
CONFIG_GC_GL_TEST_CASE=7
CONFIG_GC_GL_TEST_DEADLINE_SECONDS=200
CONFIG_GC_GL_TEST_SKIP=""
# CONFIG_GC_GL_TEST_MSAA is not set
CONFIG_YAKOGL_SHADERC_OFFLINE=y
# CONFIG_YAKOGL_SHADERC_ONLINE is not set
```

An offline image has none of `GL_TEST_ES100`, `YAKOGL_SHADERC_ONLINE`,
`YAKOGL_PRECOMPILED_SHADERS`, so `gl_test.c` includes the default `gl_test_shaders.h` (GLSL 450 /
300 es) and `GL_TEST_ES100_SET` is not defined [source, `gl_test.c` 790-800].

## 2. What cases 46 and 44 are

Names are `case_names[]` in `gl_test.c` (line 8301-8304) [source].

| Case | Name | Function | Shaders (`link_program` calls) |
|---|---|---|---|
| 46 | `es3size` | `case_es3size()` | `gl_test_es300_cube_vs_src` + `gl_test_es300_texsize_fs_src` (`es300/texsize.frag`) |
| 44 | `es3hdr` | `case_es3hdr()` | `gl_test_es300_vid_vs_src` + `gl_test_es300_vcolor_fs_src` (clear/draw), `gl_test_es300_cube_vs_src` + `gl_test_es300_hdrsample_fs_src` (sampling) |

Case 46 draws a quad whose fragment shader reports `textureSize()` of a 2D texture and a
`sampler2DArray` and checks one pixel equals `rgba(8, 24, 8, 3)`. Case 44 clears and draws into an
RGBA16F render target, reads it back and checks four pixels; its check 3 is the GC7000 layout probe
(`hdr_layout_probe`, "Case 44's layout probe on the GC7000 (#6 item 3)").

In the regression-set walker, `order[]` has 50 entries and 46 sits at index 29, 44 at index 34
[source, `gl_test.c` 21822]. The walker's tally therefore names **first failure case 46 (index 29),
second failure case 44** [source, `set_tally_case`], which is the order the plan states.

## 3. How each fails

### Case 46 `es3size`: the shader is not in the offline table

[source + earlier analysis, **not a record of this run**] `es300/texsize.frag` is keyed in main's
GC7000 table by `sha256=dbda06a6e2f0...` (header comment of `tests/reference/vivante_blobs.rs`
line 39, and the rows at lines 16195 and 37252 of the file at `502032bf`), a text with an older
precision line; `gl_test_es300_texsize_fs_src` has since gained its `precision` line, so its key
differs. The offline back end matches the SHA-256 of the exact text given to `glShaderSource`; a
text with no row logs `offline shaderc: fragment shader not in offline table (sha256=...)` and the
link fails (`src/shaderc/offline.rs`). `case_es3size()` then returns `-1` at `if (!prog) return -1`
(`gl_test.c` 6892). The table analysis counted this as the **only** miss of the 40 cases of case 7
that run, on main (`gc7000-table-pr129.md`, section 3, "gc_gl_test: the default set"; measured
there by hashing the header's texts against both tables). The same stale key made case 46 fail on
the AM62P's table image until `0fe778f6` re-keyed it (HANDOFF line 205: "Case 46 `es3size`... passes
now; it was refused before the re-key"; RESULTS.md "AM62P ... `case 46` PASSED").

The image has no console, so a miss is **not visible** in the board record: the SNVS set record
carries counts and the first two failing case numbers, not a reason.

### Case 44 `es3hdr`: RGBA16F render target stores a texel at the wrong place

**[record, earlier run, not this one]** `tools/cts/RESULTS.md`, "i.MX8MP: gl_test case 44 on
`f2a13877` still fails (2026-10-11)": `gc_gl_test` with `CONFIG_GL_TEST_CASES="44"`, record
`records/imx8mp/gc44-f2a13877/20261011-0032-gc44-f2a13877-81722382`, verdict `failed`, `CASE`
record `594b010e 002c0302 40042814 40043904`:

> case 44 (`es3hdr`) check 3 status 2, got `0x40042814` want `0x40043904`.

Texel (5,0)'s colour is stored where (7,0)'s belongs in an 8-wide linear RGBA16F target (a pixel
engine / rasteriser placement, not the sampler; HANDOFF line 238). `466e1b00`'s two fixes did not
move it. `502032bf`'s `gl_test.c` has the layout probe built in, but HANDOFF's "next board run
(i.MX8MP step 6)" says it had not run on a board; so a **case-44 failure in a set run reads as a
count only**, and the probe's per-arm answer (got[31:24] arm bits, `0x40043904`-style want words,
decode in the comment above `hdr_probe_expect`) appears only in a single-case run.

## 4. Known or new

| Case | Status | Issue | Where |
|---|---|---|---|
| 44 `es3hdr` | **known, open** | **#6** (RGBA16F target, i.MX8MP step 6) | HANDOFF 48, 70, 238-244, 441-448; RESULTS.md "case 44 on `f2a13877` still fails"; STATUS.md 23, 44. On `a0f439d6` (2026-10-10) 47 of 48 run, "first failure case 44"; PR #133's `066a2db8` image (2026-10-12 19:07) read `passed 48 of 49 run; skipped 1, first skip case 16; failing 1: first case 44, second case none` [record, INDEX.txt of `20261012-1907-gt-066a2db8-1-c30d8bd6`]. |
| 46 `es3size` | **known cause, new on this board's offline arm** | the lag of the offline table, **#22** (GC7000 table, regenerated by PR #129) | The online pair has refused 46 for the sampler-precision text (RESULTS.md 10128, "refused: invalid shader (4.5.4)", 37/38) and the AM62P's table was re-keyed in `0fe778f6`. The GC7000's table was not re-keyed on main. #22's text names `matattr`, but `matattr` rows are current on main (`gc7000-table-pr129.md` section 2): the stale `texsize` row is the one that bites case 7. The earlier i.MX8MP set records (`47 / 48 run ... first failure case 44`, `a0f439d6`; `48 of 49`, `066a2db8`) show 46 passing or not run; why is not established here (their images' shader path was not checked). **No earlier i.MX8MP record names 46 as failing** [grep of `tools/cts/log/board-sessions/*imx8mp*` has no 46/`es3size` hit; RESULTS.md names it only for the online pair and the AM62P]. |

Neither is a regression of `502032bf`: the 44 failure is reproduced on `f2a13877`, `a0f439d6` and
(per the PR #133 record) `066a2db8`; the 46 failure is a table key mismatch that exists on `main`'s
table since the text changed.

## 5. Why 11 cases were skipped, "from case 16 on"

**Not a budget, not a time cap, not a recovery, not a crash.** Eleven skips are counted by
`set_tally_case` for a `-3` return; "from case 16 on" is `first skip case 16` in the record, the
**first** skip, not a contiguous range.

| Skips | Cases | Mechanism | Source |
|---|---|---|---|
| 1 | 16 `msaa` | `case_msaa()` runs its 1x half, finds no 4x EGL config because `CONFIG_GC_GL_TEST_MSAA` is not set, logs `case 16 msaa: 1x half done; SKIP the 4x half`, returns `-3`. The set counts it as a skip, not a pass or a failure. This is #8's closed state. | `gl_test.c` 3318-3351, 21071-21075; HANDOFF 56 |
| 10 | 181, 182, 183, 188, 189, 190, 191, 192, 197, 199 | `shader_skip_reason()`: "its own sources: a table build has no rows for them", for a build without `GL_TEST_ES100_SET`. These ten are the last entries of `order[]` before 21, 17, 18 (indices 37-46). | `gl_test.c` 808-840, 21834-21840 |

50 `order[]` entries - 11 skipped = **39 cases run**, matching the figure. The configured skip list
is empty (`CONFIG_GC_GL_TEST_SKIP=""`, so `case_skipped()` skips nothing), the suite deadline of
200 s is not what cut it short (a deadline would end the run with a `CASE UNFINISHED` record and
verdict `failed`, and the 21 s uptime of the comparable PR #133 run says the set finishes in seconds),
and a recovery or crash would show in `resets=` and in a `back_after`/uptime outside the usual
range. The plan's text said the failures were "46 and 44 out of 39 run": the prediction is
**37 passed of 39 run, skipped 11, first skip case 16, failing 2: first case 46, second case 44**.

The other cases that a table build skips by name (184, 185, 195, 198, 204) are not in `order[]`, so
they do not count in this set.

## 6. Do PR #129's rows touch these shaders?

From `gc7000-table-pr129.md`, section 3 (measured there by hashing the 192 header texts against
both tables), and the table diff there (218 rows added, 0 removed, 1 re-keyed):

| Case | Uses the offline table | Touched by PR #129's 218 rows? |
|---|---|---|
| 46 `es3size` | yes: `texsize.frag` fragment shader (the `cube` vertex shader is a hit) | **Yes, this is the one row of case 7 the PR changes.** The PR re-keys `es300/texsize.frag` (`dbda06a6...` -> `c2f6426a...`), adding the text's current key; code words, wiring and every other field of the row are identical to the old row. So on the PR arm the fragment shader is a table hit; on main it is a miss. |
| 44 `es3hdr` | yes: `vid.vert`, `vcolor.frag`, `cube.vert`, `hdrsample.frag`, all hits on both arms | **No.** All four are rows present, byte-identical, on both tables; none is among the 218 (the case 7 analysis found no new key reachable except texsize). The 44 failure is the RGBA16F pixel-engine layout (#6), not a compiled-shader defect, so a table change cannot fix it. |
| 16 (skipped) and the ten name-skipped | not applicable | Not run, so not in the A/B. |

Predicted table misses of the PR arm for the three apps are 0 for `gc_gl_test` case 7 (27 of 27
shaders hit) [`gc7000-table-pr129.md`], so the PR image should log no `not in offline table` line
at all. That is invisible without a console; the set counts are what the record gives.

Two caveats the analysis already carries: a source a case builds at run time with `snprintf` is not
a header name and is not seen; and the `texsize` row on the PR was generated by a different
`etna_shc` build (`1d52afefcb16`) than the original rows, and **never run on a board**.

## 7. Record lines to expect (a check on the baseline), and what a PR-side run must show

Expected main record (from the code; compare with the real one):

```
  CASE (1) seq N: set status 7 (set failed) passed 37 of 39 run; skipped 11, first skip case 16; failing 2: first case 46, second case 44
board.sh imx8mp <name> rc=4 verdict=failed boot=uboot-mmc ... resets=absent ...
```

(the PR #133 line for the same decoder: `passed 48 of 49 run; skipped 1, first skip case 16; failing 1:
first case 44, second case none`, verdict `failed`, `rc=4`, `boot_verdict=cycled`, `resets=absent`,
`uptime=21s`.)

If the real record shows anything else (a different pass count, a first skip other than 16, a third
failing case, `resets` > 0, `CASE UNFINISHED`), this file's section 5 is wrong and the baseline must
be reread before the A/B is judged.

The PR-side run (same image configuration, PR table, same board, same day) is judged against it
case by case. Because the set record gives counts and the first two failing cases only:

| Outcome | PR-side record | Reading |
|---|---|---|
| **no change** | `passed 37 of 39 run; skipped 11, first skip case 16; failing 2: first case 46, second case 44` | The table does not change case 46 at all: the new `texsize` row either does not reach the shader or yields the same failure. The table is not a regression and not a fix. |
| **fixed by the table** | `passed 38 of 39 run; skipped 11, first skip case 16; failing 1: first case 44, second case none` (case 46 passes, 44 still fails) | The re-keyed row serves case 46 and its code words are right. This is the one change the analysis predicts to be allowed. Check that no other count moved. |
| **regressed by the table** | any case other than 46 and 44 in the failing list, or `failing` has 46 with a *different* mode (only a single-case run `GC_GL_TEST_CASE=46` shows it: a `CASE` record `case 46 check N status 2 got ... want ...` where main's link failed before any check), or `skipped` differs from 11 (a case that used to run now skips, or a skip that now runs), or `resets` > 0, `UNFINISHED` | Not explained by the 218 rows unless the failing case's shader has a new row, which the analysis says is true of no case of case 7 but 46. A failing case 10 or 2 alone is not that: #9 / #25 fail about 1 in 3 or 4 boots on both arms (HANDOFF 245-252: case 10 4 of 32, case 2 0 of 32); repeat the image before blaming the table. |
| **case 44 passes** | `failing 1: first case 46` or no failures | Not attributable to PR #129's rows (none of case 44's shaders changes). Record it as an observation, repeat to see whether it is stable, and do not credit the table. |

Procedure note: if a PR-side run lands in "no change" and the question is *why* 46 still fails, a
single-case run (`CONFIG_GC_GL_TEST_CASES="46"`, a one-case image) gives a `CASE` record with the
check number and got/want words; the set record cannot.

For `gl_slint` the main baseline was `verdict=done`; the PR side counts as no change if it is
`done` as well (the 36 femtovg files are rows on both arms, the PR adds none).

## What this note did not do

No board, no read of the main-side records, no build, no GitHub comment. The texsize key was
re-hashed here [measured]: the text of `gl_test_es300_texsize_fs_src` in `gl_test_shaders.h` at
`502032bf` (quotes joined, escapes decoded) has sha256 `c2f6426ac35d6958...`, while main's table
keys `es300/texsize.frag` by `dbda06a6e2f0...` (header comment, line 39 of `vivante_blobs.rs`).
So the lookup misses on main; `c2f6426a...` is the key PR #129 adds.
