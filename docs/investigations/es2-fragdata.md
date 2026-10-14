<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# dEQP-GLES2 `shaders.fragdata.*`: rechecked at main `fa4751b`

Item `es2-fragdata-guard` (plan #4, stage 1), host only. No file under `src/` changes, and no board
was used. Each claim is marked:

- **[measured]**: run on the host in this session, at main `fa4751b` (2026-10-14).
- **[read]**: read from source or a committed record, not run.

The full record is `tools/cts/log/investigations/es2-fragdata.md` (it landed from the integration line,
commit `4fa00f0a`). It covers what every case checks, the yakogl path and the two test layers. The host
pin is `tests/es2_fragdata.rs`. Both are already on main. This file only rechecks them at the current
tree. It does not copy the record.

## How the CTS case decides "exactly one draw buffer"

**[read]** from VK-GL-CTS `main` on raw.githubusercontent.com on 2026-10-14 (the files were fetched one
by one, not cloned, so no commit is pinned). The line numbers are counted from the raw files and may be
off by one. Upstream has shifted lines by about 6 since the checkout that the full record used (for
example, the record cites `glsShaderLibraryCase.cpp:1041-1052`). The code itself is the same as the
record describes.

| Step | CTS source (upstream `main`, 2026-10-14) | What happens |
|---|---|---|
| The case | `data/gles2/shaders/fragdata.test:1-18` | `invalid_assign_to_1`: `require only_glsl_es_100_support`, `require exactly_one_draw_buffer`, `expect compile_fail`. The fragment shader is `gl_FragData[1] = vec4(1.0)`. |
| ES 3 skip | `modules/glshared/glsShaderLibraryCase.cpp:1047-1048`, `framework/opengl/gluContextInfo.cpp:64-73` | `if (isCapabilityRequired(CAPABILITY_ONLY_GLSL_ES_100_SUPPORT, m_spec) && glu::IsES3Compatible(gl)) return true;`. `IsES3Compatible` clears the error, calls `getIntegerv(GL_MAJOR_VERSION)` and returns `getError() == GL_NO_ERROR`. If that query succeeds, the case passes without compiling anything. |
| One draw buffer | `glsShaderLibraryCase.cpp:1050-1058` | `getIntegerv(GL_MAX_DRAW_BUFFERS, &maxDrawBuffers)`, then `if ((gl.getError() == GL_NO_ERROR) && (maxDrawBuffers > 1)) throw tcu::NotSupportedError("Test requires exactly one draw buffer")`. An error (`GL_INVALID_ENUM`, the unextended ES 2.0 answer) or `n <= 1` lets the case run. The case does not check the extension string. |
| Verdict | `glsShaderLibraryCase.cpp:1181-1185` | For `EXPECT_COMPILE_FAIL`, any stage that fails to compile is a pass. If everything compiled, the result is a fail ("compiled correctly"), or a quality warning when the link failed. |
| Other cases | `modules/gles2/functional/es2fShaderFragDataTests.cpp:220-237` (`ShaderFragDataTests::init`), `:105-208` (`FragDataIndexingCase`), `fragdata.test:20-77` | `valid_{static,uniform,dynamic}_index` compile `gl_FragData[0 / u_index / int(v_index)]` (`:136-144`). A failed compile is Fail for the static index and NotSupported for the other two (`:169-175`). The case then draws a quad (`:184-185`), reads the pixels and requires them green (`:194-197`). `write_fragcolor_and_fragdata_*` must fail to compile. Neither group makes the two queries. There is no `invalid_assign_to_N` for N > 1. |

## What yakogl does at `fa4751b` (report-es2)

- **[read]** `src/gl/state.rs:1207`: `GL_MAJOR_VERSION` raises `GL_INVALID_ENUM` under `REPORT_ES2`,
  so the case is not skipped as ES 3.
- **[read]** `src/gl/state.rs:1245`: `GL_MAX_DRAW_BUFFERS` raises `GL_INVALID_ENUM` under `REPORT_ES2`.
  The context does not claim `GL_EXT_draw_buffers` or `GL_NV_draw_buffers` (`state.rs:116`, `:1239`),
  so the case is not NotSupported.
- **[read]** `src/shaderc/online/lower.rs:2772` refuses `gl_FragData[1]` with "OpenGL ES 2.0 has one
  draw buffer, so only gl_FragData[0] exists". `gl_MaxDrawBuffers` is 1 under `REPORT_ES2`
  (`lower.rs:12569`).
- **[measured]** `tests/es2_fragdata.rs`: 12 tests pass, 0 fail, 0 ignored, in each of the three board
  configurations:

  ```text
  cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_fragdata
  cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_fragdata
  cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_fragdata
  ```

  The two tests that pin the decision are `gl_api::invalid_assign_to_1_requirements_let_the_case_run`
  (`tests/es2_fragdata.rs:369`) and `gl_api::invalid_assign_to_1` (`:397`).
- **[measured]** `gl::state::tests::draw_buffer_queries_are_es3_only` passes (PowerVR, report-es2).

## Result per board

| Board | Predicted | Basis |
|---|---|---|
| AM62P (PowerVR) | Pass | **[read]** It passed on the board at `32778b57` (`docs/es2-cts-failures.md`, "Status after plan #1"). The two queries and the compile above are the same at `fa4751b`. |
| i.MX8MP (GC7000) | Pass | **[read]** The board run at `32778b57` had 0 NotSupported in the shaders tail (same table). |
| Pi 4 (V3D) | Pass | **[read]** It passed on the board at `a27d69c0` (same table). |

For the three `valid_*` cases, the host checks the compile, the link, that no GL error is raised and
that the draw reaches the back end. The green-pixel compare happens only on a board.

A board can still report NotSupported or an early pass for one reason: an image that was built without
`report-es2`. Such an image answers both queries, so the case passes at the ES 3 skip and proves
nothing. Check the build flag (`REPORT_ES2=1` / `CONFIG_YAKOGL_REPORT_ES2`) before you change anything
in `src/`.

## Source change needed

None. There is nothing to pass to `es2-error-rules` from this item.
