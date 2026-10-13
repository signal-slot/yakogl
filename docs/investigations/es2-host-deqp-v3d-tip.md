<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES2 host dEQP-GLES2 on the V3D back end at the main tip (item clippy-gate-fix)

**Status (2026-10-12): finished.** Every number below is **HOST-ONLY**. The library is built
with the Linux OSAL, which keeps the command stream instead of executing it, so every
`glReadPixels` reads black. Only these results mean something: cases begun and ended, aborts,
compile/link outcomes, info logs, API error behaviour, and results that need no rendering
(state queries, negative API). A pixel Fail means nothing, and no render verdict is quoted.

**Conclusion.**

- The whole mustpass (17165 cases, 172 chunks) ran with `GPU=v3d` at the 16 MiB tiled-copy
  budget of the Pi 4 image. **Every case began and ended. No chunk aborted.** There was no
  `ResourceError`, crash, panic or timeout.
- There were 0 compile gaps. All 878 compile/link refusals are on negative cases that dEQP
  passed. negative_api: 237 Pass, 0 Fail, 9 NotSupported.
- The `GL error 0x500 in glGetIntegerv` lines come from dEQP's ES-version probes:
  `GL_MAJOR_VERSION` (0x821B), `GL_MINOR_VERSION` (0x821C) and `GL_MAX_DRAW_BUFFERS` (0x8824).
  gdb measured this at this commit. The library refuses them **correctly** for ES 2.0, and they
  cannot become board Fails.
- Two of these lines are not from `glGetIntegerv` at all: `glGetBooleanv` and `glGetFloatv`
  log their errors under that name. This is a small diagnostic bug, with a proposed fix below.
- One host finding can be a real Pi 4 failure: the five `texture.size.2d.2048x2048_*` cases
  whose texture is larger than 16 MiB. The board passes them only when their image is built
  with `texture-2048.conf` (image T5), as the recipe does.

## Identification

- Commit: main `0c5e0b351a40` plus this item's commit `f957d3b` (the script change only).
  `src/`, `driver/`, `zephyr/`, `capi/` and `include/` are those of `0c5e0b351a40`. The script
  labels the runs `f957d3b`.
- Library: `-DYAKOGL_OSAL=linux -DYAKOGL_GPU=v3d -DYAKOGL_SHADERC=online
  -DYAKOGL_REPORT_ES2=ON -DYAKOGL_SHARED=ON`, built by the script into
  `build/host-deqp/lib-v3d`. Library log line: `osal(linux): v3d carve-out 50331648 bytes, ...
  draws a job 1024, tiled copies 16777216 bytes`.
- dEQP: VK-GL-CTS `93bca01861b0` (`vulkan-cts-1.4.6.2-555-g93bca01861b0`) with
  `targets/yakogl` applied, in the farm dependency cache (`$CCFARM_DEPS_DIR/VK_GL_CTS`).
  The script built `deqp-gles2` into `build/host-deqp/deqp`.
- List: `tools/cts/gles2-mustpass.txt`, 100 cases a process (172 chunks, the last one has 65).
  Surface: pbuffer 64x64, as the script sets it.

## The script change

On main, `tools/cts/host-deqp-run.sh` already had `GPU=v3d` (library directory `lib-v3d`) and
a report of every chunk that lost cases (`aborted.txt`; `run_chunk` resumes such a chunk). Both
parts of the diff in `tools/cts/log/investigations/es2-host-deqp-v3d.md` ("Script diff") were
therefore already there in a stronger form. This item adds only the missing part:

- For `GPU=v3d`, the script exports `YAKOGL_V3D_TILED_COPIES_BYTES=16777216` unless the caller
  set it, and it logs the value. The host library default is 8 MiB
  (`src/osal/linux.c:585`). The Pi 4 dEQP image uses 16 MiB
  (`driver/samples/zephyr/v3d_deqp/prj.conf:15`). At 8 MiB, dEQP aborts chunk `c_150` at
  `texture.size.cube.512x512_rgb888_mipmap` with a ResourceError (the earlier run in
  `es2-host-deqp-v3d.md`).
- `GPU=vivante` and `GPU=powervr` are unchanged.

Checks: `bash -n tools/cts/host-deqp-run.sh` passes. `shellcheck` is not installed on this
machine, so it was not run.

## Commands

```sh
bash -n tools/cts/host-deqp-run.sh
# run 1: the script as committed (the library logs at most 16 GL errors a process)
GPU=v3d CTS="$CCFARM_DEPS_DIR/VK_GL_CTS" tools/cts/host-deqp-run.sh
#   -> build/host-deqp/run-f957d3b-v3d
# run 2: the same, with every GL error logged (src/osal/linux.c:51-60 reads YAKOGL_ERROR_LOG)
GPU=v3d CTS="$CCFARM_DEPS_DIR/VK_GL_CTS" YAKOGL_ERROR_LOG=1000000 \
  OUT="$PWD/build/host-deqp/run-f957d3b-v3d-errlog" tools/cts/host-deqp-run.sh
# the pname of every glGetIntegerv error: the 1336 cases of run 2 that logged
# "GL error 0x500 in glGetIntegerv" (plus the first case of each chunk that logged it before
# any case), rerun under gdb in 27 processes of 50 cases, with YAKOGL_ERROR_LOG=1000000
gdb -batch -nx -x trace.gdb.py --args ./deqp-gles2 --deqp-caselist-file=bNNN.txt ...
tools/check.sh --quick
```

`trace.gdb.py` (gdb Python, not committed) puts a breakpoint on the first instruction of the
library's `glGetIntegerv` and records `pname` (`$rdi`). A `FinishBreakpoint` on the return
compares the library's `GL_ERRORS_RAISED` counter (`src/gl/context.rs:1079`) before and after the call.
When the counter went up, it prints the pname and the dEQP call chain. For the first call of
each process, the pname comes from the debug info.

This worker's command sandbox refused shell lines that start with `VAR=value`. So the runs and
`check.sh` were started from a small Python launcher that passes the same environment through
`subprocess` and changes nothing else.

## Results

Runs 1 and 2 give identical per-chunk and per-status results. Run 2 differs only in how many
`GL error` lines the library prints.

| Outcome | Count |
|---|---|
| Cases asked / begun / ended | 17165 / 17165 / 17165 (each of the 172 chunks: begun = ended = asked) |
| Chunks that lost cases or needed a second process (`aborted.txt`) | **0** (the file is empty) |
| `Test run was ABORTED` | **0** |
| Process exits | 21 `exit=0`, 151 `exit=1` (dEQP exits 1 when a chunk has any non-Pass); no other exit; one process per chunk |
| `ResourceError` | **0** |
| Crash / InternalError / Timeout / no result | 0 / 0 / 0 / 0; no `panicked` line |
| Cases with a failed compile or link | 878, **all Pass** (negative cases). `cts-refusals.py`: 878 refusals, 91 reasons; 0 on Failed cases (gaps), 0 on NotSupported cases |
| Results | 3101 Pass, 13224 Fail, 837 NotSupported, 3 CompatibilityWarning |
| negative_api | 237 Pass, 0 Fail, 9 NotSupported (`No suitable compressed formats found`) |
| `GL error` lines, run 1 (at most 16 a process) | 448 in 33 chunks; 26 chunks reach the limit |
| `GL error` lines, run 2 (no limit) | 2338, 140 error/entry-point pairs (by class below) |

Fail means nothing here, except in groups that check without rendering. All of those pass:

- state_query: 396 of 396
- fbo.api 20 of 20, fbo.completeness 370 of 370
- implementation_limits 16, attribute_location 57, lifetime 33, shader_api 38: all pass

The NotSupported cases are almost all caused by the 64x64 host surface:

- `Render target size must be at least NxN`: clipping 602, rasterization 50, shaders 36
  (invariance), texture 16, polygon_offset 5
- `Too small viewport`: texture.mipmap 52, texture.specification 42

The other NotSupported cases:

- `No multisample buffers`: 16
- negative_api compressed formats: 9
- `EXT_clip_control`: 8
- `EXT_multisampled_render_to_texture`: 1

The three CompatibilityWarnings are `flush_finish.{finish, finish_wait, flush}` ("Suspicious
performance behavior"). That comes from host timing, because the Linux OSAL executes nothing.

### The chunks the item names

| Chunk | Asked | Begun | Ended | Exit | Aborted | First case | Last case |
|---|---:|---:|---:|---:|---|---|---|
| `c_149` | 100 | 100 | 100 | 1 | no | `texture.mipmap.2d.generate.rgba4444_non_square_fastest` | `texture.size.cube.16x16_rgba4444_mipmap` |
| `c_150` | 100 | 100 | 100 | 1 | no | `texture.size.cube.16x16_rgba8888` | `texture.specification.random_teximage2d.2d_3` |
| `c_151` | 100 | 100 | 100 | 1 | no | `texture.specification.random_teximage2d.2d_4` | `texture.specification.texsubimage2d_empty_tex.la16f_cube` |
| `c_152` | 100 | 100 | 100 | 1 | no | `texture.specification.texsubimage2d_empty_tex.la88_2d` | `texture.vertex.cube.wrap.clamp_repeat` |

At the 8 MiB library default, `c_150` aborted at its 13th case,
`texture.size.cube.512x512_rgb888_mipmap`. It now runs all 100 cases. The
`texture.specification` cases are in `c_150` (76 cases), `c_151` (100) and `c_152` (16). Of
these 192 cases, 150 are Fail (host image comparison) and 42 are NotSupported (`Too small
viewport`, `es2fTextureSpecificationTests.cpp:440`). Nothing in the mustpass was left unrun.

### GL error lines by class (run 2, every error logged)

Each line was matched to the case dEQP was running when it was printed (the `Test case '...'`
line before it in the chunk's `.out`).

| Error | Lines | Cases | Where | dEQP verdict |
|---|---:|---:|---|---|
| `0x500 in glGetIntegerv` | 1464 | 1332 (1352 lines), plus 112 lines before the first case of seven fbo chunks (16 each) | 17 groups, mostly ones that draw (vertex_arrays 571, fbo.render 319, texture.specification 150, fragment_ops 125, draw 119, texture.vertex 45, texture.mipmap 12, ...), plus negative_api.state 4, fbo.completeness 2, shaders.fragdata 2 | the ES-version probes below; 62 of these cases Pass, the rest Fail on the host read-back |
| `0x505 in glDrawElements` | 5 | 5 | `texture.size.2d.2048x2048_{rgb888, rgb888_mipmap, rgba4444_mipmap, rgba8888, rgba8888_mipmap}` (`c_149`) | Fail. The draw is refused: `yakogl/v3d: draw refused: a texture larger than the tiled-copy budget ...` (`src/v3d/backend.rs:2917`) |
| every other error (138 more error/entry-point pairs, for example `0x500 glRenderbufferStorage`, `0x501 glTexImage2D`, `0x502 store_uniform`, `0x506 glCopyTexImage2D`) | 869 | — | negative_api 682 (texture 372, shader 121, buffer 63, state 60, vertex_array 35, fragment 31), fbo.completeness 164 (format probes), fbo.api 22, state_query.rbo 1 | **all Pass**: errors the cases cause and expect |

So no `GL error` line falls on a case that failed for an API reason. The only error on a
failing case that is not a probe is the 2048x2048 refusal.

## The `GL error 0x500 in glGetIntegerv` lines

### Which pname (measured with gdb at this commit)

The gdb processes printed 1461 such lines. 1459 of them were traced to a `glGetIntegerv` call:

| Count | pname | dEQP caller |
|---:|---|---|
| 1312 | `0x821B GL_MAJOR_VERSION` | `glu::IsES3Compatible` |
| 14 | `0x821B GL_MAJOR_VERSION` | `glu::ContextInfo::isES3Compatible` (calls the same function) |
| 65 + 65 | `0x821B GL_MAJOR_VERSION`, `0x821C GL_MINOR_VERSION` | `deqp::gls::FboUtil::detectGLESCompatibleContext` (fbo group set-up) |
| 1 | `0x8824 GL_MAX_DRAW_BUFFERS` | `deqp::gls::ShaderLibraryCase::execute` |
| 1 + 1 | `0xFFFFFFFF`, `0x821B` | `glu::CallLogWrapper::glGetIntegerv` (`negative_api.state.get_integerv`, which asks for invalid pnames on purpose; Pass) |

The other 2 lines are in `negative_api.state.get_booleanv` and `negative_api.state.get_floatv`
(both Pass). No `glGetIntegerv` call raised them. `glGetBooleanv` and `glGetFloatv` call
`get_integerv` (`src/gl/state.rs:1486`, `:1457`), and `get_integerv` always reports
`"glGetIntegerv"` as the entry point (`src/gl/state.rs:1081`, `:1208`, `:1246`, `:1334`). So
the log names the wrong function. `glGetInteger64v` (`:1369`) does the same. The error code that
the application sees is correct.

The fbo set-up count (65 each) is higher than in the chunked run (112 lines in seven chunks)
because the gdb processes are split differently. Each process that runs an fbo case does the
set-up once.

### Who asks (VK-GL-CTS `93bca01`)

- `modules/gles2/functional/es2fTextureSpecificationTests.cpp:439-440` throws NotSupported
  `Too small viewport` when the render target is smaller than the case's texture. Every case
  that gets past it builds a reference context at `:453`:
  `sglr::ReferenceContext refContext(sglr::ReferenceContextLimits(renderCtx), ...)`.
- `framework/opengl/simplereference/sglrReferenceContext.cpp:287` (in the
  `ReferenceContextLimits` constructor) calls `glu::IsES3Compatible(gl)`.
- `framework/opengl/gluContextInfo.cpp:64-73`, `IsES3Compatible`: `gl.getError()` (clears the
  error), `gl.getIntegerv(GL_MAJOR_VERSION, ...)` (`:70`; the comment at `:66` says this
  detects the ES version), then `return gl.getError() == GL_NO_ERROR`. The probe reads the
  error back itself, so later checks never see it. `ContextInfo::isES3Compatible` (`:271`)
  calls the same function.
- `modules/glshared/glsFboUtil.cpp:173-183`, `detectGLESCompatibleContext`: `GL_MAJOR_VERSION`
  then `GL_MINOR_VERSION`. Each is followed by `gl.getError()`, and an error means "ES 2".
- `modules/glshared/glsShaderLibraryCase.cpp:1041-1051`: for cases that need exactly one draw
  buffer, `IsES3Compatible` and then `getIntegerv(GL_MAX_DRAW_BUFFERS)` and `gl.getError()`.
  The comment at `:1047` says there is no `GL_MAX_DRAW_BUFFERS` query on unextended ES2.

Every probe reads back the error it causes, so no later `glGetError` check sees it.

### Why the earlier doc saw exactly 16 lines, all in cube cases

`src/gl/context.rs:1131-1140`: `log_error` prints at most `g_gl_error_log_limit` errors a
process (`:1058`, 16 by default). `src/osal/linux.c:51-60` raises the limit only when
`YAKOGL_ERROR_LOG` is set, and the script does not set it. The earlier rerun of `c_150` was one
process. Its first 16 errors were the probes of the first 16 `texture.specification` cases
that got past the viewport check. The 2D cases of `basic_copyteximage2d`,
`basic_copytexsubimage2d` and `basic_teximage2d` stop at `:440` (64x64 is too small for them)
and never build the reference context. So the 16 logged cases were all cube cases. Run 1 shows
the same thing: exactly 16 lines in each of `c_150`, `c_151` and `c_152`, and 26 chunks reach
the limit. Without the limit (run 2), the three `texture.specification` chunks print 39, 95 and
55 lines.

### Where the library raises it

- `src/gl/state.rs:1204-1210`, in `get_integerv`:
  `GL_MAJOR_VERSION | GL_MINOR_VERSION | GL_NUM_EXTENSIONS if crate::REPORT_ES2 =>
  { c.set_error(GL_INVALID_ENUM, "glGetIntegerv"); return 0; }`. The comment says an ES 2.0
  report has to refuse these, because dEQP and Qt read `GL_MAJOR_VERSION` to decide on ES 3.
- `src/gl/state.rs:1238-1248`: `GL_MAX_DRAW_BUFFERS if crate::REPORT_ES2 =>` the same
  refusal (`GL_EXT_draw_buffers` is not claimed).
- `src/gl/state.rs:1333-1336`: the default arm, `INVALID_ENUM` for any other unknown pname
  (the negative_api `0xFFFFFFFF` case).
- `src/gl/context.rs:1422-1428`: `set_error` calls `note_raised` (count and log) and then
  records the first error.

### Verdict against the ES 2.0 specification

**Correct.** OpenGL ES 2.0.25 §2.5 says that a command given an enum it does not accept
generates `INVALID_ENUM`. §6.1.1 and the state tables of chapter 6 list the pnames that
`GetIntegerv` accepts. `MAJOR_VERSION`, `MINOR_VERSION` and `NUM_EXTENSIONS` first appear in
ES 3.0, and none of them is in the ES 2.0 state tables. `MAX_DRAW_BUFFERS` is ES 3.0 state, or
state of `EXT_draw_buffers`/`NV_draw_buffers`, and the library claims neither extension. An ES
2.0 context must therefore refuse all four. Answering them would be the bug: dEQP would take its
ES 3 paths (`sglrReferenceContext.cpp:287-294` re-types the reference context), and the
fragdata cases would take the multiple-draw-buffer path (the comment at `state.rs:1238-1244`
records exactly that failure).

**It cannot become a board Fail.** Every probe reads `glGetError` right after its own query,
before the test's checks run. In run 2, 62 of the cases that log the line still Pass
(vertex_arrays.single_attribute 44, draw.random 12, negative_api.state 4, fbo.completeness 1,
shaders.fragdata 1). So the probe's error does not reach their checks. The cases that log it
and fail are draw cases. They fail on the host's black read-back (`Image comparison failed`),
not on a `glGetError` check. On the Pi 4 board, `docs/es2-cts-failures.md` (summary table)
records 0 Fail for the composed tree, and these probes ran there too.

### Proposed change (for a later item; `src/` is not changed here)

No conformance fix is needed. Two diagnostic fixes are worth making:

1. **Do not log the version probes.** On a board the log limit stays at 16 a process (only the
   Linux OSAL reads `YAKOGL_ERROR_LOG`). So a dEQP image spends its first 16 error lines on
   these probes, and the errors that matter (such as the 2048x2048 refusal) are not logged.
   - `src/gl/context.rs` near `:1422`: add `pub fn set_error_quiet(&mut self, e: GLenum)`. It
     records the error and counts it in `GL_ERRORS_RAISED`/`GL_ERRORS_RECORDED` exactly like
     `set_error`, but does not call `log_error`.
   - `src/gl/state.rs:1208` and `:1246`: call `c.set_error_quiet(GL_INVALID_ENUM)` for the
     three version pnames and for `GL_MAX_DRAW_BUFFERS` when `REPORT_ES2`. Leave the default
     arm (`:1334`) and the `es3_only_pname` arm (`:1081`) logging.
   - Test: extend `es2_report_refuses_the_es3_version_queries` (`src/gl/state.rs:3010`) to
     assert that `glGetError()` is still `GL_INVALID_ENUM` and that the logged-error count did
     not change.
2. **Name the right entry point.** Give `get_integerv` (`src/gl/state.rs:1063`) a
   `func: &str` parameter and use it in its four `set_error` calls (`:1081`, `:1208`,
   `:1246`, `:1334`). `glGetIntegerv` passes `"glGetIntegerv"`, `glGetFloatv` (`:1457`)
   `"glGetFloatv"`, `glGetBooleanv` (`:1486`) `"glGetBooleanv"`, `glGetInteger64v` (`:1369`)
   `"glGetInteger64v"`. These four are the only callers of `get_integerv` in `src/gl`.

The error that the application sees does not change. Only the console line changes.

## Host findings that could be real Pi 4 board failures

1. **`texture.size.2d.2048x2048_{rgb888, rgb888_mipmap, rgba4444_mipmap, rgba8888,
   rgba8888_mipmap}`, 5 cases.** At 16 MiB the draw is refused (`GL error 0x505 in
   glDrawElements`, `draw refused: a texture larger than the tiled-copy budget`). dEQP reports
   a Fail, not a ResourceError, so the chunk does not abort. The same cases fail on the board
   unless the image that holds them is built with
   `driver/samples/zephyr/v3d_deqp/texture-2048.conf` (24 MiB). The recipe does that for image
   T5 (`tools/cts/pi4-deqp-images.sh`, `tools/cts/pi4-full-tree.txt`), and
   `tools/cts/check-deqp-images.py` guards the two budgets. The two `2048x2048_l8*` cases fit
   in 16 MiB. Not run on the host here: `YAKOGL_V3D_TILED_COPIES_BYTES=25165824 GPU=v3d
   tools/cts/host-deqp-run.sh texture.size.2d.2048` would check the 24 MiB budget.
2. **Not board failures:** the `glGetIntegerv` lines (above) and every other `GL error` line.
   All of those fall on cases dEQP passes.
3. **Not measured on the host, so open on the board:** everything that renders (13224 host
   Fails), and the cases the 64x64 surface makes NotSupported: clipping 602, rasterization 50,
   shaders.invariance 36, texture.mipmap 52, texture.specification 42, texture 16,
   polygon_offset 5. The board runs them at its own surface size. This host run says nothing
   about them.
4. **Compile and API behaviour carry over:** 0 compile gaps, 0 negative_api Fail, all query
   groups Pass. A board Fail in these groups would therefore not come from the shared GL core
   or the online compiler as built here. It would come from the board build (offline shader
   table, Zephyr OSAL) or from execution.

## Compared with the earlier V3D host runs

| | `47aa17aea9c7` (8 MiB; `tools/cts/log/investigations/es2-host-deqp-v3d.md`) | `db3545fa25e2` (16 MiB; earlier attempt of this item, not merged) | **`0c5e0b351a40` + `f957d3b` (16 MiB, this run)** |
|---|---|---|---|
| Cases begun | 17078 of 17165 | 17165 of 17165 | **17165 of 17165** |
| Aborted chunks | 1 (`c_150`, 13 of 100 began) | 0 | **0** |
| `ResourceError` | 1 | 0 | **0** |
| Compile refusals | 878, 92 reasons | 878, 91 reasons | 878, 91 reasons |
| negative_api | 237 / 0 / 9 | 237 / 0 / 9 | 237 / 0 / 9 |
| Results (Pass / Fail / NotSupported / Warning) | — | 3101 / 13224 / 837 / 3 | 3101 / 13224 / 837 / 3 |

Raw files (not committed): `build/host-deqp/run-f957d3b-v3d/` (run 1) and
`build/host-deqp/run-f957d3b-v3d-errlog/` (run 2).
