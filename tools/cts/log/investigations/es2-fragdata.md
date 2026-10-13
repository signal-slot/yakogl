<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

> Record landed from `ccfarm/integration` (old PR #57, ccfarm `01a11d2b-2352-71e9-be21-aa7de80b4320`, commit `4fa00f0ae`). Written on 2026-10-09 against the integration line (base `9f47da77`), not on main; the text is as written, only file paths are changed to main's layout. Status (2026-10-12): `shaders.fragdata.invalid_assign_to_1` passes on all three boards on main builds (AM62P and i.MX8MP `32778b57`, Pi 4 `a27d69c0`); the fix is `4e9397c6`, which both lines share. `tests/es2_fragdata.rs` is the host pin.
# dEQP-GLES2 `shaders.fragdata.*`: what the cases check, and what yakogl does

Item `es2-fragdata-guard` (plan #4, stage 1). Host reading and host tests only;
no board was used and nothing under `src/` changed. The tests are
`tests/es2_fragdata.rs`:

```text
cargo test --no-default-features \
    --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_fragdata
cargo test --no-default-features \
    --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_fragdata
cargo test --no-default-features \
    --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_fragdata
```

(2026-10-12, at main `9390731`: 12 pass in each of the three `report-es2`
builds; 5 pass without `report-es2`, where only the front-end tests are
built.) CTS source: the
`VK-GL-CTS` checkout in the machine's dependency cache; line numbers below are
that tree's.

## What each case checks

The group `fragdata` is built in `ShaderFragDataTests::init`
(`modules/gles2/functional/es2fShaderFragDataTests.cpp:221-238`): three C++
cases, then the four cases of `data/gles2/shaders/fragdata.test`.

| Case | Source | Checks |
|---|---|---|
| `valid_static_index` / `valid_uniform_index` / `valid_dynamic_index` | cpp:223-228, `FragDataIndexingCase` cpp:106-209 | Compile `gl_FragData[0]`, `gl_FragData[u_index]` or `gl_FragData[int(v_index)]` `= v_color` (cpp:115-148). A failed compile is `Fail` for the static index (cpp:172-173) and `NotSupported` for the other two (cpp:175). Then clear red, draw a quad with `u_index = 0`, `v_index = 0`, colour green, and require every pixel green (cpp:178-198). |
| `invalid_assign_to_1` | fragdata.test:2-19 | `version 100 es`, `expect compile_fail`, `require only_glsl_es_100_support`, `require exactly_one_draw_buffer`; the fragment shader is `gl_FragData[1] = vec4(1.0)`. |
| `write_fragcolor_and_fragdata_simple` / `_static_if` / `_unused_func` | fragdata.test:21-78 | `expect compile_fail`, no `require`: a shader that writes both `gl_FragColor` and `gl_FragData[0]` must not compile, also when the writes sit in the two arms of `if (false)` or one write is in a function `main` never calls. |

There is no `invalid_assign_to_N` for N > 1: only `invalid_assign_to_1`
exists.

### How `invalid_assign_to_1` decides "exactly one draw buffer"

`ShaderLibraryCase::execute`, `modules/glshared/glsShaderLibraryCase.cpp`:

1. **lines 1041-1042**: `isCapabilityRequired(ONLY_GLSL_ES_100_SUPPORT) &&
   glu::IsES3Compatible(gl)` returns `true` (a pass) at once.
   `IsES3Compatible` (`framework/opengl/gluContextInfo.cpp:64-73`) clears the
   error, calls `glGetIntegerv(GL_MAJOR_VERSION)` and says "ES 3" iff the
   error is `GL_NO_ERROR`. So **a build that answers `GL_MAJOR_VERSION`
   passes this case without compiling anything**; only a `report-es2` build
   (where that query is `GL_INVALID_ENUM`) goes on.
2. **lines 1044-1052**: `glGetIntegerv(GL_MAX_DRAW_BUFFERS, &n)`. If the
   call raises **no error and `n > 1`** the case throws `NotSupportedError
   ("Test requires exactly one draw buffer")` (line 1051). If the query
   raises an error (`GL_INVALID_ENUM`, the ES 2.0 answer with no extension) or
   `n <= 1`, the case runs. There is no extension-string test.
3. The shaders are compiled and linked; `compile_fail` passes if compile or
   link fails (the rest of `execute`, after line 1052).

The `write_fragcolor_and_fragdata_*` cases carry no `require`, so neither the
version query nor the draw-buffer query is made.

## What yakogl does (report-es2 build)

- `glGetIntegerv(GL_MAJOR_VERSION)`: `GL_INVALID_ENUM` in `report-es2`
  (Cargo.toml feature comment). The context is therefore taken for ES 2.0.
- `glGetIntegerv(GL_MAX_DRAW_BUFFERS)`: `GL_INVALID_ENUM`, value untouched
  (`src/gl/state.rs:1245-1248`; in-crate test `draw_buffer_queries_are_es3_only`,
  run: passes; host test `gl_api::invalid_assign_to_1_requirements_let_the_case_run`).
  Neither `GL_EXT_draw_buffers` nor `GL_NV_draw_buffers` is claimed, so
  nothing brings the query back. The case therefore runs rather than reporting NotSupported.
  Without `report-es2` the query answers `MAX_DRAW_BUFFERS` (3), but then the
  version query also answers and step 1 passes the case.
- Shader: `gl_FragData[1] = ...` is refused at lowering with "OpenGL ES 2.0 has
  one draw buffer, so only gl_FragData[0] exists"
  (`src/shaderc/online/lower.rs:2768-2774`), so `glCompileShader` fails and
  the case passes. `gl_FragData[0]`, `[u_index]` and `[int(v_index)]` lower to
  the one colour store (`lower.rs:2750-2780`), and `gl_MaxDrawBuffers` is 1 in
  `report-es2` (`lower.rs:12569`). The three `write_fragcolor_and_fragdata_*`
  shaders are refused too (measured; all three tests pass).
- The offline-table path is not involved: these sources are not in the
  table, so a GPU back end with `shaderc-online` compiles them (PowerVR,
  GC7000, V3D all use the same front end; only the front end is under test).

The host tests have two layers. The top of `tests/es2_fragdata.rs` compiles
the case shaders with the front end (`compile_to_ir`). `mod gl_api` makes a
host context (a do-nothing OSAL and back end, as `tests/es2_error_rules_state.rs`
does) and replays each case through the public GL entry points in a
`report-es2` + `shaderc-online` build: the two queries of the shader library
and the extension string, `glCompileShader`/`glLinkProgram` with the
`compile_fail` verdict of `glsShaderLibraryCase.cpp:1174-1178`, and for
`valid_*` the clear, `u_index = 0`, the indexed quad draw (no GL error, the
draw reaches the back end with 6 indices) and `glReadPixels`. The pixel check
of `valid_*` (green quad) is a back-end matter: the host back end draws
nothing, so it is not replayed here.

## Prediction per board

| Board | Predicted result | Basis |
|---|---|---|
| AM62P (PowerVR) | Pass (recorded, `docs/es2-cts-failures.md:77`, tree `32778b57`) | Both queries refused, shader refused. The earlier NotSupported was measured on a build older than commit `4e9397c` (`es2-gl-core`), which is an ancestor of the integration tip; `GL_MAX_DRAW_BUFFERS` answered 3 then. |
| i.MX8MP (GC7000) | Pass | Same front end and queries. `docs/es2-cts-failures.md:76` records the fix confirmed on the board (tree `32778b57`, 0 NotSupported). |
| Pi 4 (V3D) | Pass (recorded, `docs/es2-cts-failures.md:77`, tree `a27d69c0`) | Same as the AM62P. |

The `valid_*` and `write_fragcolor_*` cases are not listed as failing on any
board in `docs/es2-cts-failures.md` and the front end behaves as the case
needs. Still a prediction until the round-1 board trees (am62p-es2-r1,
imx8mp-es2-r1, pi4-es2-r1a/b) are read: one thing a board could change is a
build made **without** `report-es2` that is not an ES 2.0 report (it passes at
step 1 and is not a signal either way).

## Source change needed

None for this group. If a board still shows NotSupported, the first thing to
check is that the dEQP image was built with `report-es2` (`REPORT_ES2=1` /
`CONFIG_YAKOGL_REPORT_ES2`), not a change in `src/gl/state.rs`. Nothing to
hand to `es2-error-rules` from this item.
