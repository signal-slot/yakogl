<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES2 `shaders.fragdata.*`: what the cases check, what yakogl does, the board results

Item `es2-fragdata-guard` of plan #20, roadmap stage 1. Host reading and host tests only. No file
under `src/`, `tools/` or `Cargo.toml` is changed by this item. Every claim is marked:

- **[measured]**: run on the host in this session, at main `667e90a8` (2026-10-15),
  and in an earlier recheck at main `fa4751b` (2026-10-14).
- **[read]**: read from source or a committed record, not run.
- **[inferred]**: reasoning, not run.

The first record of this investigation is `tools/cts/log/investigations/es2-fragdata.md`. It was
written on 2026-10-09 against the integration line and landed on main (commit `4fa00f0a`) with a status
note. This file is the record at the current tree: the CTS decision with line cites, the yakogl path,
the host pin and the **measured** final board trees. The earlier one only predicted those trees.

## Status

**Nothing is left. All seven `shaders.fragdata.*` cases pass on all three boards in the final
dEQP-GLES2 trees. Host tests pin the behaviour, and none of them is `#[ignore]`.** The fix was `4e9397c6`
(`es2-gl-core`). Before it, `GL_MAX_DRAW_BUFFERS` answered 3 in an ES 2.0 context, so
`invalid_assign_to_1` was NotSupported on the AM62P and the Pi 4. On the i.MX8MP, a `valid_*` case was
NotSupported with "Dynamic indexing of gl_FragData[] not supported". No source change is needed, and
nothing goes to `es2-error-rules`.

## CTS source

VK-GL-CTS checkout in the machine's dependency cache (`$CCFARM_DEPS_DIR/VK-GL-CTS`, the tree the board
images build). Line numbers are that tree's **[read]**. The same code in upstream `main` sits a few
lines lower; see "Line numbers in upstream `main`" below.

The group is built in `ShaderFragDataTests::init`
(`modules/gles2/functional/es2fShaderFragDataTests.cpp:221-238`). It has three C++ cases
(`addChild` at cpp:223, 225, 227) and the four cases of `data/gles2/shaders/fragdata.test`, added at
cpp:236. There is no `invalid_assign_to_N` for N > 1. Only `invalid_assign_to_1` exists.

| Case | Source | What it checks |
|---|---|---|
| `valid_static_index` | cpp:223, `FragDataIndexingCase` | Fragment shader `gl_FragData[0] = v_color`. If the program fails, the case fails with `TCU_FAIL("Compile failed")` (cpp:170-173). Otherwise it clears red (cpp:178-179), draws a quad with colour green, then compares every pixel against green and passes or fails (cpp:193-201). |
| `valid_uniform_index` | cpp:225 | Same, with `gl_FragData[u_index]` (`uniform mediump int u_index`, cpp:118, 139). It sets `u_index` to 0 (cpp:183). If the program fails, the case is **NotSupported** "Dynamic indexing of gl_FragData[] not supported" (cpp:175). |
| `valid_dynamic_index` | cpp:227 | Same, with `gl_FragData[int(v_index)]`, `v_index` = 0. A failed program is NotSupported (cpp:175). |
| `invalid_assign_to_1` | fragdata.test:2-19 | `version 100 es`, `expect compile_fail`, `require only_glsl_es_100_support` (line 5), `require exactly_one_draw_buffer` (line 6). Fragment shader: `gl_FragData[1] = vec4(1.0)` (line 16). |
| `write_fragcolor_and_fragdata_simple` | fragdata.test:21-37 | `expect compile_fail`, no `require`. The shader writes both `gl_FragColor` and `gl_FragData[0]`. |
| `write_fragcolor_and_fragdata_static_if` | fragdata.test:39-57 | Same, with the writes in the two arms of `if (false)`. |
| `write_fragcolor_and_fragdata_unused_func` | fragdata.test:59-78 | Same, with the `gl_FragData[0]` write in a function `main` never calls. |

### How `invalid_assign_to_1` decides "exactly one draw buffer"

`ShaderLibraryCase` in `modules/glshared/glsShaderLibraryCase.cpp` decides this with two queries. It
does not test an extension string:

1. **Lines 1041-1042.** If `only_glsl_es_100_support` is required and `glu::IsES3Compatible(gl)` is
   true, the case returns `true`, which is a pass, without compiling anything. `IsES3Compatible`
   (`framework/opengl/gluContextInfo.cpp:64-73`) clears the error, calls
   `glGetIntegerv(GL_MAJOR_VERSION)` (line 70), and says "ES 3" if and only if the error is then
   `GL_NO_ERROR` (line 72).
2. **Lines 1044-1052.** `glGetIntegerv(GL_MAX_DRAW_BUFFERS, &maxDrawBuffers)` (line 1049). The case
   throws `NotSupportedError("Test requires exactly one draw buffer")` (line 1051) **only if** the call
   raised no error **and** returned more than 1 (line 1050). A `GL_INVALID_ENUM`, which is the
   unextended ES 2.0 answer as the comment at lines 1046-1047 says, or a value of 1, lets the case run.
3. **Lines 1174-1178.** `EXPECT_COMPILE_FAIL` passes only if a compile fails. If everything compiles
   but the link fails, the case fails ("compiled and link failed"). If everything compiles and links,
   it fails too.

The `write_fragcolor_and_fragdata_*` cases have no `require`, so neither query is made for them. They
go straight to step 3.

### Line numbers in upstream `main`

**[read]** from VK-GL-CTS `main` on raw.githubusercontent.com on 2026-10-14 (the files were fetched one
by one, not cloned, so no commit is pinned; the numbers are counted from the raw files and may be off
by one). Upstream has shifted lines by about 6. The code is the same as above.

| Step | Upstream `main` (2026-10-14) | Cache checkout (above) |
|---|---|---|
| The case | `data/gles2/shaders/fragdata.test:1-18` | fragdata.test:2-19 |
| ES 3 skip | `glsShaderLibraryCase.cpp:1047-1048`, `gluContextInfo.cpp:64-73` | 1041-1042, same |
| One draw buffer | `glsShaderLibraryCase.cpp:1050-1058` | 1044-1052 |
| Verdict | `glsShaderLibraryCase.cpp:1181-1185` | 1174-1178 |
| Other cases | `es2fShaderFragDataTests.cpp:220-237` (`init`), `:105-208` (`FragDataIndexingCase`), shaders `:136-144`, compile failure `:169-175`, draw `:184-185`, compare `:194-197`; `fragdata.test:20-77` | cpp:221-238 and the lines in the case table |

## What yakogl does (`report-es2` build, as the board images are built)

Read at `667e90a8` **[read]**. The host tests below confirm the outcome of each item **[measured]**.

- `glGetIntegerv(GL_MAJOR_VERSION)`: `GL_INVALID_ENUM` when `crate::REPORT_ES2` is set
  (`src/gl/state.rs:1207`). Step 1 therefore does not short-circuit the case.
- `glGetIntegerv(GL_MAX_DRAW_BUFFERS)`: `GL_INVALID_ENUM` with the value left untouched in
  `report-es2` (`src/gl/state.rs:1245`, comment at 1238-1244). Other builds answer
  `context::max_draw_buffers()` (line 1249), but then step 1 already passes the case. The extension
  string claims neither `GL_EXT_draw_buffers` nor `GL_NV_draw_buffers` (`state.rs:116`, `:1239`). So
  the case runs, as it should.
- Compile of `gl_FragData[1] = ...`: refused at lowering with "OpenGL ES 2.0 has one draw buffer, so
  only gl_FragData[0] exists" (`src/shaderc/online/lower.rs:2772`). `glCompileShader` reports failure,
  so step 3 passes the case.
- Compile of `gl_FragData[0]`, `[u_index]` and `[int(v_index)]`: these lower to the one colour store
  (`lower.rs:2750-2780`). Every index other than 0 is out of range with one draw buffer.
  `gl_MaxDrawBuffers` is 1 in `report-es2` (`lower.rs:12569`). Compile and link succeed, so the case
  reaches its draw.
- Both `gl_FragColor` and `gl_FragData` written: refused in semantic analysis with "a fragment shader
  writes either 'gl_FragColor' or 'gl_FragData', never both" (`src/shaderc/online/sema.rs:457`). The
  static-if and unused-function shapes are refused as well, because the rule is per shader and not per
  executed path.
- These sources are not in the offline shader table, so every back end compiles them with the online
  front end. The PowerVR, GC7000 and V3D builds all take the same path up to the back end **[inferred]**.

## Host pin: `tests/es2_fragdata.rs`

The file is already on main (`a1189436`, `53fe3973`, `46d0c715`). This item re-ran it at the tip and
pointed its header at this file. It has two layers:

- The top level compiles the case shaders with the front end (`compile_to_ir`):
  `valid_static_index`, `valid_uniform_index`, `valid_dynamic_index`, `invalid_assign_to_1`,
  `write_fragcolor_and_fragdata` (all three shapes), and `gl_max_draw_buffers_is_one_in_the_es2_report`.
- `mod gl_api` builds only with `report-es2`. It makes a host GL context with a do-nothing OSAL and
  back end, as `tests/es2_error_rules_state.rs` does, and replays each case through the public GL entry
  points:
  - `invalid_assign_to_1_requirements_let_the_case_run` (`tests/es2_fragdata.rs:372`): the two
    queries of steps 1-2 (both `GL_INVALID_ENUM`) and no draw-buffer extension in the extension string.
  - `invalid_assign_to_1` (`:400`) and `write_fragcolor_and_fragdata`: the `compile_fail` verdict of
    step 3.
  - `valid_*`: compile and link, clear, `u_index = 0`, the indexed quad draw with no GL error and
    6 indices reaching the back end, then `glReadPixels`.

  The host back end draws nothing, so the green-pixel check stays a board result.

```text
cargo test --no-default-features \
    --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_fragdata
cargo test --no-default-features \
    --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_fragdata
cargo test --no-default-features \
    --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_fragdata
```

Results at `667e90a8` and at main `fa4751b` **[measured]**: 12 passed, 0 failed, 0 ignored in each of
the three builds. At `fa4751b`, `gl::state::tests::draw_buffer_queries_are_es3_only` passed as well
(PowerVR, report-es2). No `[[test]]` entry or `required-features` is needed: the file is
`#![cfg(feature = "shaderc-online")]`, and `mod gl_api` is gated on `report-es2`.

## Board results

These are the final whole-tree records, all built with `report-es2`. A tree records counts per entry,
not per case, so a case counts as passed when its entry has 0 fail and 0 NotSupported **[read]**.

| Board | Record | Entry holding `shaders.fragdata.*` | Result for the 7 cases |
|---|---|---|---|
| AM62P (PowerVR) | `docs/board-sessions/am62p-es2-final-r2.md`, origin/main `63efed8e` | image 1 (`tools/cts/am62p-full-tree.txt:3`): 1512 / 1512 pass | **Pass**. The record names `invalid_assign_to_1` as no longer NotSupported, and its 18 NotSupported cases are clip_control 8, multisampled_render_to_texture 1 and compressedtexsubimage2d 9. |
| i.MX8MP (GC7000) | `docs/board-sessions/imx8mp-es2-final-r2.md`, origin/main `c345ab74` | entry 2, "shaders (20 groups)" (`tools/cts/imx8mp-full-tree.txt:8`): 1512 / 1512, 0 notsup | **Pass**. All 34 NotSupported results are in entries 22 and 28. |
| Pi 4 (V3D) | `docs/board-sessions/pi4-es2-final-r2.md`, origin/main `d56e79e5` | row D (`tools/cts/pi4-full-tree.txt:18`): 305 pass / 0 fail / 0 NotSupported | **Pass**. |

Earlier board runs agree: the AM62P passed the case at `32778b57`, the i.MX8MP run at `32778b57` had 0
NotSupported in the shaders tail, and the Pi 4 passed at `a27d69c0` (`docs/es2-cts-failures.md`,
"Status after plan #1") **[read]**.

Later readings do not touch this group **[read]**: the i.MX8MP tree at `835cf9cc` (2026-10-14) and the
Pi 4 rows 0-14 at `20a48791` have 46 failures, all `GL_BGRA_EXT` renderbuffer cases in the `fbo` entry
(`docs/es2-cts-failures.md`, Summary), not in the shaders entry.

The prediction for the next trees at any later tip is still Pass on all three boards **[inferred]**,
unless an image is built **without** `report-es2`. In that case step 1 passes `invalid_assign_to_1`
without compiling anything, and `valid_uniform_index` / `valid_dynamic_index` see
`GL_MAX_DRAW_BUFFERS` > 1. The first thing to check on a regression is `CONFIG_DEQP_REPORT_ES2=y` /
`REPORT_ES2=1` in the image, not `src/`.

## Source change needed

None. Nothing goes to `es2-error-rules` from this group.
