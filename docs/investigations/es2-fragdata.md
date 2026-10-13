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

These are the CTS lines that the record cites, **[read]** from the record. This session could not
fetch VK-GL-CTS again because the network was refused, so the line numbers are those of the record's
checkout.

| Step | CTS source | What happens |
|---|---|---|
| The case | `data/gles2/shaders/fragdata.test:2-19` | `invalid_assign_to_1`: `require only_glsl_es_100_support`, `require exactly_one_draw_buffer`, `expect compile_fail`. The fragment shader is `gl_FragData[1] = vec4(1.0)`. |
| ES 3 skip | `modules/glshared/glsShaderLibraryCase.cpp:1041-1042`, `framework/opengl/gluContextInfo.cpp:64-73` | If `glGetIntegerv(GL_MAJOR_VERSION)` raises no error, the context counts as ES 3 and the case returns a pass without compiling anything. |
| One draw buffer | `glsShaderLibraryCase.cpp:1044-1052` (the throw is at line 1051) | `glGetIntegerv(GL_MAX_DRAW_BUFFERS, &n)`. The case is NotSupported ("Test requires exactly one draw buffer") only when the query raises **no error and** `n > 1`. An error (`GL_INVALID_ENUM`) or `n <= 1` lets the case run. The case does not check the extension string. |
| Verdict | `glsShaderLibraryCase.cpp:1174-1178` | `compile_fail` passes when a stage fails to compile. |
| Other cases | `modules/gles2/functional/es2fShaderFragDataTests.cpp:221-238` (the group), `:106-209` (`FragDataIndexingCase`), `fragdata.test:21-78` | `valid_{static,uniform,dynamic}_index` compile `gl_FragData[0 / u_index / int(v_index)]`, draw a green quad and compare the pixels. `write_fragcolor_and_fragdata_*` must fail to compile. Neither group makes the two queries. There is no `invalid_assign_to_N` for N > 1. |

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
