<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# es2-final-expected: what each image or row of the final dEQP-GLES2 trees should read

Date: 2026-10-14. Written on the host at main `fa4751b6573f`. Nothing here ran
on a board. This file is the reference for the final sessions
(`am62p-es2-final-a`, `am62p-es2-final-b`, `imx8mp-es2-final`,
`pi4-es2-final-a`, `pi4-es2-final-b`) and for ccfarm's post-promote
`deqp-tree` stages. A session compares its per-image or per-row counters with
the tables below and writes any difference into its record by case name.
Modelled on `tools/cts/log/board-sessions/pi4-es2-r2-baseline.md`.

## 0. Part 1: the invariance fix at the tip

The PowerVR invariance fix is on main: `f9688bc5` (a sum on the way to an
`invariant` output is not fused into an `fmad`), merged by `e17fb6a7`. The test
file `tests/es2_invariance.rs` runs the twelve cases (`common_subexpression_0`
to `_3` at lowp, mediump and highp) with no `#[ignore]`. It needs a GPU feature
and `shaderc-online`; the bare `cargo test --test es2_invariance` does not
build. Run at main `fa4751b6573f`, one command per back end and per
ES level (`cargo test --no-default-features --features "<set>" --test
es2_invariance`):

| Feature set (`host-test,` and `shaderc-online` in all) | Result line |
|---|---|
| `gpu-powervr` | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `gpu-powervr,report-es2` | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `gpu-vivante` | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `gpu-vivante,report-es2` | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `gpu-v3d,no-shader-table` | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |
| `gpu-v3d,no-shader-table,report-es2` | `test result: ok. 12 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out` |

All nine PowerVR cases that failed on the board (cs_1/2/3 at three
precisions) pass on the host. The fix is still in at the tip.

## 1. How to read the tables

- **Cases**: the number of names of `tools/cts/gles2-mustpass.txt` that the
  row's filter selects. Counted by a throw-away script (the filter globs of
  the tree list, matched against the 17,165 names). Each board's rows select
  every name exactly once: 0 names twice, 0 names left out.
- **Begun**: the number of cases the image runs and the board prints in its
  own counter. It is larger than Cases where the filter also runs names that
  are not in the mustpass list (206 on the AM62P and the i.MX8MP, 202 on
  the Pi 4). Those extra cases pass in every record. A board's per-image
  counter is compared on this basis.
- **Expected final** (on the Begun basis): Pass / Fail / NotSupported (NS) /
  QualityWarning (QW) / CompatibilityWarning (CW). Fail is 0 in every row.
  dEQP counts QW and CW as a pass; the tally prints them apart and Pass does
  not include them.
- **NS and QW buckets** are the accepted ones of
  `docs/es2-cts-failures.md` section C. Any NS or QW case that is not in a
  bucket named in the row is a difference.
- **CW** is `flush_finish.flush` and `flush_finish.flush_wait`, dEQP's own
  timing check, which flips between Pass and CW between runs (section C,
  "QualityWarning"). Each one that warns moves one case from Pass to CW.
  The tables show Pass with both as Pass (CW 0), and the column says "0-2".
- **Fix on main**: a row whose reading changes because of a commit on main.
  Rows without a fix are expected equal to their last reading.
- Last readings come from `docs/es2-cts-failures.md` (Summary, section A,
  "Round 1"), `tools/cts/RESULTS.md`, `tools/cts/STATUS.md` and the board
  records `docs/board-sessions/*-r2.md`. **Those records are newer than the
  doc's tables:** `docs/es2-cts-failures.md` and `STATUS.md` do not mention
  the three whole-tree records of 2026-10-12 below (they say "no board has a
  recorded final tree"; `es2-docs-r1` holds the doc). The records are on main.

Whole-tree records used (all on main, all 2026-10-12):

| Board | Commit | Record | Reading |
|---|---|---|---|
| AM62P | `63efed8e` | `docs/board-sessions/am62p-es2-final-r2.md` | 1x tree 17,125 pass / 0 / 34 NS / 5 QW / 1 CW; ms4 16 / 0; composed 17,141 / 0 / 18 / 5 / 1 |
| i.MX8MP | `c345ab74` | `docs/board-sessions/imx8mp-es2-final-r2.md` | `TALLY (29 entries): begun 17371 passed 17318 failed 0 notsup 34 other 19 noresult 0` |
| Pi 4 | `d56e79e5` | `docs/board-sessions/pi4-es2-final-r2.md` | 17,127 pass / 0 / 34 NS / 3 QW / 1 CW |

So each board already read 0 failures on a main commit. The final sessions
confirm it on the final tip, one commit for the whole tree.

## 2. AM62P (PowerVR BXS-4-64): `tools/cts/am62p-full-tree.txt`, 17 images, and `am62p-ms4-tree.txt`

Readings: `32778b57` is the whole tree of 2026-10-10 (`RESULTS.md`, "AM62P: the
whole dEQP-GLES2 tree on origin/main `32778b57`: 54 fail"; Summary of
`docs/es2-cts-failures.md`). `63efed8e` is the whole tree of 2026-10-12
(`docs/board-sessions/am62p-es2-final-r2.md`). Both read Pass / Fail / NS /
warnings on the Begun basis, except image 1 at `32778b57`, which the RESULTS
table gives on the mustpass basis (1,370 / 9 of 1,379; 1,512 begun).

| Image | Filter (`dEQP-GLES2.` dropped) | Cases | Begun | Last reading `32778b57` | Last reading `63efed8e` | Expected final (Pass / Fail / NS / QW / CW) | Fix on main |
|---:|---|---:|---:|---|---|---|---|
| 1 | `functional.shaders.{struct, keywords, scoping, linkage, conditionals, return, qualification_order, ...}` (20 filters) | 1,379 | 1,512 | 1,370 / **9** / 0 / 0 (mustpass basis) | 1,512 / 0 / 0 / 0 | 1,512 / 0 / 0 / 0 / 0 | **9 `shaders.invariance.{lowp,mediump,highp}.common_subexpression_{1,2,3}`: Fail to Pass, `f9688bc5`, merged by `e17fb6a7`.** Image 1 alone at `e17fb6a7`: 1,379 pass / 0 fail on the mustpass list (`RESULTS.md`, "AM62P: image 1 on `e17fb6a7`"). Also `shaders.fragdata.invalid_assign_to_1` (NS to Pass, `4e9397c`) |
| 2 | `functional.shaders.{matrix, invalid_implicit_conversions}.*` | 994 | 994 | 994 / 0 / 0 / 0 | 994 / 0 / 0 / 0 | 994 / 0 / 0 / 0 / 0 | none |
| 3 | `functional.shaders.{swizzles, conversions}.*` | 1,172 | 1,172 | 1,172 / 0 / 0 / 0 | 1,172 / 0 / 0 / 0 | 1,172 / 0 / 0 / 0 / 0 | none |
| 4 | `functional.shaders.{indexing, preprocessor}.*` | 1,084 | 1,100 | 1,100 / 0 / 0 / 0 | 1,100 / 0 / 0 / 0 | 1,100 / 0 / 0 / 0 / 0 | none |
| 5 | `functional.shaders.{loops, random}.*` | 2,523 | 2,525 | 2,525 / 0 / 0 / 0 | 2,525 / 0 / 0 / 0 | 2,525 / 0 / 0 / 0 / 0 | none |
| 6 | `functional.shaders.operator.binary_operator.*` | 1,318 | 1,318 | 1,318 / 0 / 0 / 0 | 1,318 / 0 / 0 / 0 | 1,318 / 0 / 0 / 0 / 0 | none |
| 7 | `functional.shaders.operator.{unary_operator, common_functions, ...}.*` (10 filters) | 1,631 | 1,631 | 1,631 / 0 / 0 / 0 | 1,631 / 0 / 0 / 0 | 1,631 / 0 / 0 / 0 / 0 | none |
| 8 | `functional.texture.{completeness, filtering, format}.*` | 324 | 324 | 324 / 0 / 0 / 0 | 324 / 0 / 0 / 0 | 324 / 0 / 0 / 0 / 0 | none |
| 9 | `functional.texture.{mipmap, specification, vertex, wrap}.*` | 409 | 452 | 407 / **45** / 0 / 0 | 452 / 0 / 0 / 0 | 452 / 0 / 0 / 0 / 0 | **45 `texture.vertex.2d.{filtering (36), wrap (9)}.*`: Fail to Pass, `a6fc6f14`** (an indexed draw too big for the GENERAL arena is drawn in pieces of its index list; gl_test 197 `vtxgrid`). `63efed8e` read 452 / 452 |
| 10 | `functional.texture.size.*` | 74 | 80 | 80 / 0 / 0 / 0 | 80 / 0 / 0 / 0 | 80 / 0 / 0 / 0 / 0 | none |
| 11 | `functional.fragment_ops.blend.*` | 1,060 | 1,060 | 1,060 / 0 / 0 / 0 | 1,060 / 0 / 0 / 0 | 1,060 / 0 / 0 / 0 / 0 | none |
| 12 | `functional.fragment_ops.{depth, stencil, depth_stencil, random, interaction, scissor, ...}.*` (7 filters) | 882 | 882 | 882 / 0 / 0 / 0 | 882 / 0 / 0 / 0 | 882 / 0 / 0 / 0 / 0 | none |
| 13 | `functional.uniform_api.*` | 1,267 | 1,267 | 1,267 / 0 / 0 / 0 | 1,267 / 0 / 0 / 0 | 1,267 / 0 / 0 / 0 / 0 | none |
| 14 | `functional.fbo.*` | 709 | 711 | 711 / 0 / 0 / 0 | 711 / 0 / 0 / 0 | 711 / 0 / 0 / 0 / 0 | none (the 27 `fbo.completeness.attachment_combinations.*_tex` pass since `a3856a06`, section C) |
| 15 | `functional.{clipping, vertex_arrays}.*` | 1,173 | 1,173 | 1,173 / 0 / 0 / 0 | 1,173 / 0 / 0 / 0 | 1,173 / 0 / 0 / 0 / 0 | none |
| 16 | `functional.{state_query, negative_api, draw}.*` | 743 | 743 | 734 / 0 / 9 / 0 | 734 / 0 / 9 / 0 | 734 / 0 / **9** / 0 / 0 | none. NS: bucket `negative_api.texture.compressedtexsubimage2d_{invalid_size, invalid_target, level_max_cube, level_max_tex2d, neg_level_cube, neg_level_tex2d, neg_offset, neg_wdt_hgt, offset_allowed}` |
| 17 | `info.*`, `functional.{attribute_location, rasterization, buffer, default_vertex_attrib, shader_api, lifetime, depth_range, flush_finish, multisample, ...}.*` (20 filters) | 423 | 427 | 395 / 0 / 25 / 7 (5 QW + 2 CW) | 396 / 0 / 25 / 6 (5 QW + 1 CW) | 397 / 0 / **25** / **5** / 0-2 | none. NS 25: bucket `clip_control.{depth_mode_one_to_one, depth_mode_zero_to_one, errors, face_culling, initial, modify_get, origin, viewport_bounds}` (8), `multisample.*` (16, the 1x config, see ms4 below), `multisampled_render_to_texture.readpixels` (1). QW 5: bucket `rasterization.interpolation.basic.{line_strip, line_strip_wide, lines}` and `.projected.{lines, lines_wide}`. CW: `flush_finish.{flush, flush_wait}` |
| | **sum** | **17,165** | **17,371** | | | **17,332 / 0 / 34 / 5 / 0-2** (CW 0) | |

The image 17 filter list is the tree file's line 17; it is cut here. Which
bucket falls in which image: `compressedtexsubimage2d` in 16; `clip_control`,
`multisample`, `multisampled_render_to_texture.readpixels`, the five
interpolation cases and `flush_finish` in 17 (checked by script against the
filter globs).

### The ms4 image (`tools/cts/am62p-ms4-tree.txt`, `DEQP_GL_CONFIG_NAME=rgba8888d24s8ms4`)

| Image | Filter | Cases | Begun | Last reading | Expected final (Pass / Fail / NS / QW / CW) | Fix on main |
|---|---|---:|---:|---|---|---|
| MS4 | `functional.multisample.*` | 16 | 16 | `32778b57`: 14 / **2** / 0 / 0; `452d6c31`: 16 / 0 (HANDOFF; `RESULTS.md`); `63efed8e`: 16 / 0 / 0 / 0 (`am62p-es2-final-r2.md`) | 16 / 0 / 0 / 0 / 0 | **2 `multisample.constancy_alpha_to_coverage`, `multisample.constancy_both_inverted`: Fail to Pass, `a4f64950`** (a frame uploads a PDS program it already holds once, so dEQP's 1024-draw 4x frame is one kick) |

### AM62P totals

The 17 rows add up to 17,165 cases. Result buckets, each summing to 17,165:

| Basis | Pass | Fail | NS | QW | CW | Sum |
|---|---:|---:|---:|---:|---:|---:|
| 17 images at 1x, mustpass list (CW 0) | 17,126 | 0 | 34 | 5 | 0 | 17,165 |
| 17 images at 1x, Begun (the 206 extra cases pass) | 17,332 | 0 | 34 | 5 | 0 | 17,371 |
| 1x tree with the ms4 image in place of the 16 multisample NS (mustpass list) | 17,142 | 0 | 18 | 5 | 0 | 17,165 |

NS 34 = 9 + 8 + 1 + 16; with the ms4 image 18 = 9 + 8 + 1. A flush_finish case
that reads CW moves 1 from Pass to CW, at most 2 (17,140 / 0 / 18 / 5 / 2).
The last reading is `63efed8e`: 17,141 / 0 / 18 / 5 / 1 (composed), which is
this with one CW. The pass count 17,141 of `d60b9d76` (RESULTS section 1z2)
had 19 NS because `shaders.fragdata.invalid_assign_to_1` was NS then.

## 3. i.MX8MP (Vivante GC7000): `tools/cts/imx8mp-full-tree.txt`, 29 entries

Entry number = the selector `i` (0-based), as in the results file
`tools/cts/imx8mp-full-tree-29460f96.results.tsv`. The last reading is the same
at four commits: `d8583f20` (2026-10-01, the tsv), `32778b57` (2026-10-10,
`RESULTS.md`, "i.MX8MP: the whole dEQP-GLES2 tree on Zephyr at origin/main
`32778b57`": 29 of 29 entries equal), `334d4e65`
(`tools/cts/log/board-sessions/imx8mp-es2-r1.md`) and `c345ab74`
(`docs/board-sessions/imx8mp-es2-final-r2.md`: "No entry changed"). Columns of
the reading: Pass / Fail / NS / other (QW), Begun basis. No GPU reset, no entry
without a result.

| Entry | Filter (`dEQP-GLES2.` dropped) | Cases | Begun | Last reading (four commits) | Expected final (Pass / Fail / NS / QW) | Fix on main |
|---:|---|---:|---:|---|---|---|
| 0 | `info.*, functional.prerequisite.*` | 9 | 9 | 9 / 0 / 0 / 0 | 9 / 0 / 0 / 0 | none |
| 1 | `functional.color_clear.*` | 19 | 19 | 19 / 0 / 0 / 0 | 19 / 0 / 0 / 0 | none |
| 2 | `functional.shaders.{struct, keywords, scoping, ...}.*` (20 filters) | 1,379 | 1,512 | 1,512 / 0 / 0 / 0 | 1,512 / 0 / 0 / 0 | `shaders.fragdata.invalid_assign_to_1`: `4e9397c`, board-read since `32778b57` (was 1,511 + 1 NS at `72d16864`). The PowerVR invariance fix does not touch this board |
| 3 | `functional.shaders.{matrix, invalid_implicit_conversions}.*` | 994 | 994 | 994 / 0 / 0 / 0 | 994 / 0 / 0 / 0 | none |
| 4 | `functional.shaders.{swizzles, conversions}.*` | 1,172 | 1,172 | 1,172 / 0 / 0 / 0 | 1,172 / 0 / 0 / 0 | none |
| 5 | `functional.shaders.{indexing, preprocessor}.*` | 1,084 | 1,100 | 1,100 / 0 / 0 / 0 | 1,100 / 0 / 0 / 0 | none |
| 6 | `functional.shaders.loops.*` | 625 | 625 | 625 / 0 / 0 / 0 | 625 / 0 / 0 / 0 | none |
| 7 | `functional.shaders.random.*` | 1,898 | 1,900 | 1,900 / 0 / 0 / 0 | 1,900 / 0 / 0 / 0 | none |
| 8 | `functional.shaders.operator.binary_operator.*` | 1,318 | 1,318 | 1,318 / 0 / 0 / 0 | 1,318 / 0 / 0 / 0 | none |
| 9 | `functional.shaders.operator.{unary_operator, common_functions, ...}.*` (10 filters) | 1,631 | 1,631 | 1,631 / 0 / 0 / 0 | 1,631 / 0 / 0 / 0 | none |
| 10 | `functional.texture.completeness.*` | 40 | 40 | 40 / 0 / 0 / 0 | 40 / 0 / 0 / 0 | none |
| 11 | `functional.texture.filtering.2d.*` | 136 | 136 | 136 / 0 / 0 / 0 | 136 / 0 / 0 / 0 | none |
| 12 | `functional.texture.filtering.cube.*` | 112 | 112 | 112 / 0 / 0 / 0 | 112 / 0 / 0 / 0 | none |
| 13 | `functional.texture.format.*` | 36 | 36 | 36 / 0 / 0 / 0 | 36 / 0 / 0 / 0 | none |
| 14 | `functional.texture.mipmap.*` | 112 | 112 | 98 / 0 / 0 / 14 | 98 / 0 / 0 / **14** | none. QW 14: bucket `texture.mipmap.cube.generate.{a8, l8, la88, rgb565, rgb888, rgba4444, rgba5551, rgba8888}_{fastest, nicest}` except `l8_nicest` and `la88_nicest` (GC7000 behaviour) |
| 15 | `functional.texture.size.*` | 74 | 80 | 80 / 0 / 0 / 0 | 80 / 0 / 0 / 0 | none |
| 16 | `functional.texture.specification.*` | 192 | 192 | 192 / 0 / 0 / 0 | 192 / 0 / 0 / 0 | none |
| 17 | `functional.texture.vertex.*` | 90 | 90 | 90 / 0 / 0 / 0 | 90 / 0 / 0 / 0 | none (the PowerVR `a6fc6f14` is not on this board's path) |
| 18 | `functional.texture.wrap.*` | 15 | 58 | 58 / 0 / 0 / 0 | 58 / 0 / 0 / 0 | none |
| 19 | `functional.fragment_ops.blend.*` | 1,060 | 1,060 | 1,060 / 0 / 0 / 0 | 1,060 / 0 / 0 / 0 | none |
| 20 | `functional.fragment_ops.{depth, stencil, depth_stencil, random, interaction, scissor}.*` | 863 | 863 | 863 / 0 / 0 / 0 | 863 / 0 / 0 / 0 | none |
| 21 | `functional.fbo.*` | 709 | 711 | 711 / 0 / 0 / 0 | 711 / 0 / 0 / 0 | `fbo.completeness.renderable.renderbuffer.depth.depth_component24`: `4e9397c`, board-read since `32778b57` (was 710 + 1 Fail at `72d16864`) |
| 22 | `functional.negative_api.*` | 246 | 246 | 237 / 0 / 9 / 0 | 237 / 0 / **9** / 0 | none. NS 9: bucket `negative_api.texture.compressedtexsubimage2d_*` |
| 23 | `functional.state_query.*` | 396 | 396 | 396 / 0 / 0 / 0 | 396 / 0 / 0 / 0 | none |
| 24 | `functional.uniform_api.*` | 1,267 | 1,267 | 1,267 / 0 / 0 / 0 | 1,267 / 0 / 0 / 0 | none |
| 25 | `functional.clipping.*` | 602 | 602 | 602 / 0 / 0 / 0 | 602 / 0 / 0 / 0 | none |
| 26 | `functional.vertex_arrays.*` | 571 | 571 | 571 / 0 / 0 / 0 | 571 / 0 / 0 / 0 | none |
| 27 | `functional.{draw, attribute_location, rasterization, buffer}.*` | 259 | 259 | 254 / 0 / 0 / 5 | 254 / 0 / 0 / **5** | none. QW 5: bucket `rasterization.interpolation.basic.{line_strip, line_strip_wide, lines}` and `.projected.{lines, lines_wide}` |
| 28 | `functional.{default_vertex_attrib, shader_api, lifetime, depth_range, dither, light_amount, multisample, implementation_limits, flush_finish, ...}.*` (15 filters) | 256 | 260 | 235 / 0 / 25 / 0 | 235 / 0 / **25** / 0 | none. NS 25: bucket `clip_control.*` (8), `multisample.*` (16, no 4x config; i.MX8MP #8), `multisampled_render_to_texture.readpixels` (1). `flush_finish.*` has not warned on this board; a CW would be a new reading |
| | **sum** | **17,165** | **17,371** | 17,318 / 0 / 34 / 19 | **17,318 / 0 / 34 / 19** | |

### i.MX8MP totals

The 29 rows add up to 17,165 cases.

| Basis | Pass | Fail | NS | QW | Sum |
|---|---:|---:|---:|---:|---:|
| Begun (the board's TALLY line) | 17,318 | 0 | 34 | 19 | 17,371 |
| Mustpass list (the 206 extra cases all pass) | 17,112 | 0 | 34 | 19 | 17,165 |

`docs/es2-cts-failures.md` gives the mustpass reading as a range (17,112 to
17,116 pass, 30 to 34 NS) because the board record names the NS cases only by
count. All 34 NS names are mustpass names (9 + 8 + 16 + 1, counted against the
filters; entry 28's 4 extra cases are not among them), and 17,371 - 17,165 =
206 = 17,318 - 17,112, so the expected mustpass reading is the single value
17,112 / 0 / 34 / 19. The final session takes the 34 names from the
`DEQP_FAILS` pages, if its image prints them; if it cannot name them it keeps
the range as the doc has it.

## 4. Raspberry Pi 4 (V3D 4.2): `tools/cts/pi4-full-tree.txt`, 29 rows

Row = 0-based position among the non-comment lines (the `TREE_ROWS` number).
Readings: `fe5e8e73` (2026-10-01, sweep 30, the table of `pi4-es2-r2-baseline.md`
section 1; `RESULTS.md`; Begun = its "Cases run"); `d56e79e5` (2026-10-12,
`docs/board-sessions/pi4-es2-final-r2.md`). In between, `a27d69c0`
(2026-10-10, section A of the doc: 9 Fail, 34 cases lost by the capture) and
`a25712f2` (2026-10-11, `docs/board-sessions/pi4-es2-tip-r2.md`: 17,118 / 9 /
34 / 3 / 1), whose 9 Fail are the same 9 cases as at `a27d69c0` and pass at
`d56e79e5`. Columns: Pass / Fail / NS / QW (CW).

| Row | Label | Filter (`dEQP-GLES2.` dropped) | Cases | Begun | `fe5e8e73` | `d56e79e5` | Expected final (Pass / Fail / NS / QW / CW) | Fix on main |
|---:|---|---|---:|---:|---|---|---|---|
| 0 | AL | `functional.attribute_location.*` | 57 | 57 | 57 / 0 / 0 / 0 | 57 / 0 / 0 / 0 | 57 / 0 / 0 / 0 / 0 | none |
| 1 | D | `functional.shaders.{algorithm, builtin_variable, conditionals, ...}.*` (13 filters) | 292 | 305 | 304 / 0 / 1 / 0 | 305 / 0 / 0 / 0 | 305 / 0 / 0 / 0 / 0 | `shaders.fragdata.invalid_assign_to_1`: NS to Pass, `4e9397c` (Pass at `a27d69c0`, `a25712f2`, `d56e79e5`) |
| 2 | E | `functional.shaders.{constants, keywords, scoping, struct}.*` | 352 | 453 | 453 / 0 / 0 / 0 | 453 / 0 / 0 / 0 | 453 / 0 / 0 / 0 / 0 | none |
| 3 | F1 | `functional.fragment_ops.*` | 1,923 | 1,923 | 1,923 / 0 / 0 / 0 | 1,923 / 0 / 0 / 0 | 1,923 / 0 / 0 / 0 / 0 | none. The capture can drop lines (console bridge): a run marked `done_lossy` or with Lost cases is a rerun, not a failure |
| 4 | FB1 | `functional.fbo.{api, completeness}.*` | 390 | 392 | 365 / 0 / 27 / 0 | 392 / 0 / 0 / 0 | 392 / 0 / 0 / 0 / 0 | 27 `fbo.completeness.attachment_combinations.*_tex`: NS to Pass. No commit in this history: the V3D has claimed `GL_OES_depth_texture` since 2026-10-03, before the import `afb1633` |
| 5 | FB2 | `functional.fbo.render.*` | 319 | 319 | 319 / 0 / 0 / 0 | 319 / 0 / 0 / 0 | 319 / 0 / 0 / 0 / 0 | none |
| 6 | FF | `functional.flush_finish.*` | 5 | 5 | 5 / 0 / 0 / 0 | 4 / 0 / 0 / 0 (CW 1) | 5 / 0 / 0 / 0 / 0-2 | none. CW bucket `flush_finish.{flush, flush_wait}` (timing check) |
| 7 | H | `functional.{rasterization, clipping}.*` | 654 | 654 | 651 / 0 / 0 / 3 | 651 / 0 / 0 / 3 | 651 / 0 / 0 / **3** / 0 | none. QW 3: bucket `rasterization.interpolation.basic.lines`, `.projected.lines`, `.projected.lines_wide` |
| 8 | LA | `functional.light_amount.*` | 19 | 19 | 19 / 0 / 0 / 0 | 19 / 0 / 0 / 0 | 19 / 0 / 0 / 0 / 0 | none |
| 9 | MISC | `functional.{depth_stencil_clear, polygon_offset, multisample, ...}.*` (4 filters) | 70 | 74 | 58 / 0 / 16 / 0 | 58 / 0 / 16 / 0 | 58 / 0 / **16** / 0 / 0 | none. NS 16: bucket `multisample.*` (no 4x config on the Pi 4) |
| 10 | NS | `functional.{negative_api, state_query}.*` | 642 | 642 | 633 / 0 / 9 / 0 | 633 / 0 / 9 / 0 | 633 / 0 / **9** / 0 / 0 | none. NS 9: bucket `negative_api.texture.compressedtexsubimage2d_*` |
| 11 | RT | `functional.shaders.random.texture.*` | 200 | 200 | 200 / 0 / 0 / 0 | 200 / 0 / 0 / 0 | 200 / 0 / 0 / 0 / 0 | none (fragment cube look-ups changed by `9b24ea2`; 200 / 200 on `1cf946a9`) |
| 12 | S1 | `functional.shaders.operator.*` | 2,949 | 2,949 | 2,949 / 0 / 0 / 0 | 2,949 / 0 / 0 / 0 | 2,949 / 0 / 0 / 0 / 0 | none |
| 13 | S2 | `functional.shaders.{matrix, swizzles, conversions}.*` | 1,910 | 1,910 | 1,910 / 0 / 0 / 0 | 1,910 / 0 / 0 / 0 | 1,910 / 0 / 0 / 0 / 0 | none |
| 14 | S3 | `functional.shaders.{loops, indexing, preprocessor, ...}.*` (4 filters) | 2,171 | 2,188 | 2,188 / 0 / 0 / 0 | 2,188 / 0 / 0 / 0 | 2,188 / 0 / 0 / 0 / 0 | none |
| 15 | S4 | `functional.shaders.{functions, invalid_implicit_conversions, random.basic_expression, ...}.*` (9 filters) | 2,012 | 2,027 | 2,027 / 0 / 0 / 0 | 2,027 / 0 / 0 / 0 | 2,027 / 0 / 0 / 0 / 0 | none |
| 16 | S5f | `functional.shaders.random.all_features.fragment.*` | 98 | 100 | 100 / 0 / 0 / 0 | 100 / 0 / 0 / 0 | 100 / 0 / 0 / 0 / 0 | none (cube fragment look-ups: `9b24ea2`; 100 / 100 on `1cf946a9`) |
| 17 | S5v | `functional.shaders.random.all_features.vertex.*` | 100 | 100 | 100 / 0 / 0 / 0 | 100 / 0 / 0 / 0 | 100 / 0 / 0 / 0 / 0 | none |
| 18 | SMALL | `info.*, functional.prerequisite.*, functional.color_clear.*, ...` (10 filters) | 227 | 227 | 227 / 0 / 0 / 0 | 227 / 0 / 0 / 0 | 227 / 0 / 0 / 0 / 0 | none |
| 19 | T1 | `functional.texture.{completeness, format, wrap, vertex}.*` | 181 | 224 | 206 / **18** / 0 / 0 | 224 / 0 / 0 / 0 | 224 / 0 / 0 / 0 / 0 | **18 `texture.vertex.cube.{filtering (10), wrap (8)}.*`: Fail to Pass, `ec5ba4f`, `53ef6d7`** (non-seamless cube emulation in ES 2.0 builds) |
| 20 | T2 | `functional.texture.filtering.*` | 248 | 248 | 248 / 0 / 0 / 0 | 248 / 0 / 0 / 0 | 248 / 0 / 0 / 0 / 0 | none (cube fragment look-ups: `9b24ea2`; 248 / 248 on `1cf946a9`) |
| 21 | T3 | `functional.texture.{mipmap, specification}.*` | 304 | 304 | 298 / **6** / 0 / 0 | 304 / 0 / 0 / 0 | 304 / 0 / 0 / 0 / 0 | **6 `texture.mipmap.cube.{basic, bias, projected}.{linear_linear, linear_nearest}`: Fail to Pass, `9b24ea2`** (the fragment variant takes dEQP's cube level from FDX / FDY of the direction); still Fail at `a27d69c0` and `a25712f2` (298 / 6). 304 / 304 on `1cf946a9` |
| 22 | T4 | `functional.texture.size.2d.1024x1024_*, ...` (26 filters) | 67 | 69 | 69 / 0 / 0 / 0 | 69 / 0 / 0 / 0 | 69 / 0 / 0 / 0 / 0 | **3 `texture.size.cube.15x15_{l8, rgb888, rgba8888}`: Pass at `fe5e8e73`, Fail at `a27d69c0` and `a25712f2` (66 / 3), Pass again with `9b24ea2`** (the same fragment-level fix; section A). The row must not read 66 / 3 again. 69 / 69 on `1cf946a9` |
| 23 | T5 | `functional.texture.size.2d.2048x2048_*, ...` (7 filters) | 7 | 7 | 7 / 0 / 0 / 0 | 7 / 0 / 0 / 0 | 7 / 0 / 0 / 0 / 0 | none. Needs `texture-2048.conf` (the tree file's note); a run that stops in this row leaves the board off the network for some minutes |
| 24 | TF | `functional.shaders.texture_functions.*` | 17 | 20 | 19 / **1** / 0 / 0 | 20 / 0 / 0 / 0 | 20 / 0 / 0 / 0 / 0 | **`shaders.texture_functions.vertex.texturecubelod`: Fail to Pass, `ec5ba4f`, `53ef6d7`** (20 / 20 on `1cf946a9`) |
| 25 | U1 | `functional.uniform_api.*` | 1,267 | 1,267 | 1,267 / 0 / 0 / 0 | 1,267 / 0 / 0 / 0 | 1,267 / 0 / 0 / 0 / 0 | none |
| 26 | VA | `functional.vertex_arrays.*` | 571 | 571 | 571 / 0 / 0 / 0 | 571 / 0 / 0 / 0 | 571 / 0 / 0 / 0 / 0 | none |
| 27 | DR | `functional.draw.*` | 101 | 101 | 101 / 0 / 0 / 0 | 101 / 0 / 0 / 0 | 101 / 0 / 0 / 0 / 0 | none |
| 28 | XT | `functional.{clip_control, debug_marker, multisampled_render_to_texture}.*` | 12 | 12 | 3 / 0 / 9 / 0 | 3 / 0 / 9 / 0 | 3 / 0 / **9** / 0 / 0 | none. NS 9: bucket `clip_control.*` (8) and `multisampled_render_to_texture.readpixels` (1) |
| | **sum** | | **17,165** | **17,367** | 17,277 / 25 / 62 / 3 | 17,329 / 0 / 34 / 3 / 1 | **17,330 / 0 / 34 / 3 / 0-2** | |

All three sum columns are on the Begun basis (17,367 cases run): 17,277 + 25 +
62 + 3 = 17,367 at `fe5e8e73`; 17,329 + 34 + 3 + 1 = 17,367 at `d56e79e5`.
The mustpass-list tally of `d56e79e5` (17,127 / 0 / 34 / 3 / 1) is 202 lower
in Pass, the 202 cases outside the list.

### Pi 4 totals

The 29 rows add up to 17,165 cases.

| Basis | Pass | Fail | NS | QW | CW | Sum |
|---|---:|---:|---:|---:|---:|---:|
| Mustpass list (CW 0) | 17,128 | 0 | 34 | 3 | 0 | 17,165 |
| Begun (the 202 extra cases pass) | 17,330 | 0 | 34 | 3 | 0 | 17,367 |

NS 34 = 16 (row 9) + 9 (row 10) + 9 (row 28). A flush_finish case that warns
moves 1 from Pass to CW, at most 2 (17,126 / 0 / 34 / 3 / 2). The last
reading, `d56e79e5`, is 17,127 / 0 / 34 / 3 / 1 (one CW). `fe5e8e73` was
17,075 / 25 / 62 / 3; the 52 expected changes of the baseline file (25 cube
Fail, 27 `*_tex` NS) are all in the table above, plus `fragdata` and the 3
`size.cube.15x15` cases.

## 5. What the final sessions do with this file

1. Run the whole tree of the board at one commit of main (or read the
   post-promote `deqp-tree` stage's tally).
2. For each image or row, compare the counters with "Expected final". A row
   that equals it needs no note. A row that differs is news: name each
   differing case.
3. The whole-tree total must read the "Totals" table of the board: Fail 0,
   NS and QW exactly the buckets named in the rows, the sum 17,165.
4. The rows with a "Fix on main" cell are the ones the final session closes:
   AM62P image 1 (`f9688bc5`, `e17fb6a7`), image 9 (`a6fc6f14`), ms4
   (`a4f64950`); Pi 4 rows 19, 21, 22, 24 (`ec5ba4f`, `53ef6d7`, `9b24ea2`);
   row 1 and AM62P image 1 / i.MX8MP entry 2 for `fragdata` and i.MX8MP
   entry 21 (`4e9397c`); Pi 4 row 4 (no commit).
5. A lossy capture (Lost cases, `done_lossy`) or a scan failure is a rerun of
   that image or row, not a failure.
6. Do not copy these expectations into a record as measured results.
