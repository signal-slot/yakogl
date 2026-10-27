<!-- SPDX-License-Identifier: Apache-2.0 -->
# dEQP-GLES3 shaders: the AM62P failures reproduced on the host

ES 3.0 plan, item `pvr-shadow`. The follow-up to `docs/es3/shader-failure-map.md`,
whose §3 left **645** of the AM62P's 2,231 `shaders.*` failures (main
`396d667a`, 2026-09-26) tied to no cause. Every failing case of the group is
given a cause here, from the per-case board data and a host run; the front
end's share turns out to be zero (§4).

Sources, each a file in the repository or a run on this host:

- **board**: `docs/es3/am62p-gles3-notpass.tsv`, the AM62P per-case list at
  build `ad2b5ed` (run 2026-10-13/14): every mustpass case whose result is
  not Pass, with dEQP's first message line. For `shaders.*` it holds 1,397
  names: 410 `Fail` and 987 `NoResult`.
- **host**: `MODULE=gles3 GPU=powervr tools/cts/host-deqp-run.sh
  dEQP-GLES3.functional.shaders.` at `70142d35` (2026-10-22), VK-GL-CTS
  `93bca01`: all 20,616 mustpass cases of the group began and ended
  (no aborted chunk). HOST-ONLY: the library keeps the command stream, so
  every pixel reads black and a render verdict means nothing. What is real
  is each case's compile and link outcome, our info log, and the GL errors
  dEQP's set-up code reports (an API verdict).
- **front end alone**: `tests/es3_shaderc_repro.rs` lowers the programs of
  the rows below through `compile_to_ir` (parser, sema, lower, fold) with no
  back end, then builds them on each GPU's online back end, and runs the IR
  on the CPU evaluator (eval.rs) where the case is arithmetic.

## 1. What the host run says about the group as a whole

| host verdict (PowerVR back end, `70142d35`) | cases |
|---|---:|
| Pass | 2,496 |
| QualityWarning (dEQP's "low-quality shader precision" on the 955 `builtin_functions.precision` executor programs: a pass) | 955 |
| Fail, a black pixel (meaningless here: "Fail" 10,975, "Image comparison failed" 5,428, "Result comparison failed" 360, "Got unexpected texture size" 38) | 16,801 |
| Fail, a compile refusal by the **back end** (the four buckets of §3) | 265 |
| Fail, a link refusal (64-byte reflection name, §3 row 11) | 2 |
| Fail, a GL API verdict (framebuffer set-up, §3 rows 8 and 13) | 69 |
| Fail, "Invalid builtin constant value": `builtin_variable.max_*` render the constant and read it back, black here; all 22 passed on the board | 22 |
| Fail, "Quick check fails on recipient" (`metamorphic`, a pixel check) | 6 |

**No front-end refusal on any case that should build.** `tools/cts/cts-refusals.py`
counts 2,298 compile refusals in 146 reasons over the run: 2,031 in 140
reasons on cases dEQP **passed**, that is negative tests the compiler was
right to refuse (`no implicit conversion` 796, `expected a name` 150,
`reserved for future use` 126, `no form of this texture look-up takes these
arguments` 116, ...), and 267 in 6 reasons on cases dEQP failed. Those six
all carry the back end's prefix, "the code generator refused this ... shader"
(four reasons, 265 cases), or the link's (two, the 64-byte name). Measured
twice: no case the board passed is refused on the host (the join of board
and host per case gives zero such rows), and no host case other than a pass
carries a `0:<line>:` (parser, sema, preprocessor or lowering) reason.

## 2. The 645 of `396d667a`, accounted for

`shader-failure-map.md` §3 attributed 1,586 of the 2,231 failures at
`396d667a` to a subgroup through three buckets (1,461 builtin_functions and
22 builtin_variable "Failed to set up framebuffer object", 103 derivate
"checkFramebufferStatus"), leaving 645. No per-case list of `396d667a` exists
(its logs were never committed), so those 645 cannot be named one by one.
The closest per-case list is **board** (`ad2b5ed`, 17 days later), and
every failing case of it is in a row of §3. Set against the three buckets:

- The three buckets at `ad2b5ed`: builtin_functions' "Failed to set up
  framebuffer object" bucket is gone (the GL half of the integer render
  targets landed, item int-rt-gl; of its 1,461 cases the `common` ones pass
  but round, roundEven and modf, rows 4 and 7, and the 987 `precision` ones
  ended `NoResult` on the image's `ResourceError`, row 13), builtin_variable's
  is gone (0 failures), and derivate's "checkFramebufferStatus" is down to
  15 (row 8).
- Failures at `ad2b5ed` outside those three buckets: 395 =
  texture_functions 297 + builtin_functions.common 40 + precision 24 +
  fragdepth 15 + derivate (other) 8 + qualification_order 7 + uniform_block 2 +
  metamorphic 2. Each is in a row of §3 with a cause.
- The other 645 − 395 = **250** of `396d667a`'s unattributed failures are
  **not reproduced**: they do not fail at `ad2b5ed` and cannot be named. The
  sources that mention them say what they were: the "Failed to compile
  shader program ... trunc" bucket (24 `operator.common_functions.trunc.*`,
  `RESULTS.md` R-bucket `:1512`; `trunc` lowers today, `tests/es3_shaderc_repro.rs`
  uses `Ftrunc` through `modf`), the `line_continuation` and `centroid`
  refusals since fixed (items pp-splice, pvr-centroid), and the
  texture-upload API errors (`GL_INVALID_OPERATION` / `GL_INVALID_ENUM` at
  gluTexture, R-bucket `:1506` and `:1514`) that item sized-formats-gl removed,
  which turned those texture_functions cases into the compile refusals of
  row 1 or the passes of today.

## 3. Causes, by count

Cause words: **front end** (parse, sema, lower, preprocessor), **folding**,
**back-end refusal** (the code generator refuses; the host shows it),
**back-end miscompile / hardware** (compiles everywhere, wrong pixel on the
board, black on the host: not reproducible here), **GL core** (an API verdict
or a link decision). "host" is the PowerVR back end's verdict at `70142d35`;
GC7000UL and V3D columns come from `tests/es3_shaderc_repro.rs` where it
runs the same programs there.

| # | subgroup (child) | board (`ad2b5ed`) | host (`70142d35`) | cause | code site | fix or owner |
|---:|---|---:|---|---|---|---|
| 1 | texture_functions: texturegrad 35, texturegradoffset 28, textureprojgrad 26, textureprojgradoffset 26, texelfetchoffset 24, textureoffset 22, texture 18 (`isampler`/`usampler` on cube, 3D, array), texturelodoffset 15, texelfetch 8 (integer 3D/array), textureprojlodoffset 7, textureprojoffset 7, textureproj 6 (integer 3D), texturelod 4 (integer cube) | 226 Fail "Failed to compile shader program (glsShaderRenderCase.cpp:447)" | 226 refused, 121 fragment + 105 vertex: `Usc(Select(Unsupported("a texture fetch with an integer result on a cube, a 3D texture or an array, gradients, a projective cube or array, or an offset on a stacked texture, is not selected yet")))` | **back-end refusal** (PowerVR). The front end lowers every form: sema's `ES300_TEXTURES` and `builtin_call.rs` give `Op::Tex` with `TexLod::Grad`, a literal offset, a `Fetch` level | `src/shaderc/online/powervr/select.rs` (the message); `src/shaderc/online/texref.rs` | PowerVR: gradients (`TexLod::Grad`), offsets on stacked images, integer results from cube/3D/array. Items pvr-2darray (arrays, shadow) and formats-pvr (integer samplers) cover parts; the gradient and offset forms have no item |
| 2 | texture_functions: texture 13, texturelod 8, textureproj 6, textureprojlod 4, texelfetch 2 -- every one a `sampler3D` or `sampler2DArray`(`Shadow`) look-up with a float result | 33 Fail (no message: a wrong pixel) | compiled and linked; black | **back-end miscompile / hardware**: PowerVR 3D textures are a one-level grid of slices (`docs/es3-cts-failures.md` RC15) and 2D arrays one stacked image (RC6) | `src/pvr/` texture state; `powervr/select.rs` `vol_grid` | items pvr-2darray (running) and pvr-3d |
| 3 | texture_functions.texturesize (38: every sampler type, vertex and fragment) | 38 Fail "Got unexpected texture size" | compiled and linked; black ("Got unexpected texture size" here too, from the black pixel) | **back-end miscompile / hardware**: the size is read from the image-state words on the PowerVR (2D: width and height less one plus the base level, shifted by `lod`; arrays and cubes by dividing the stacked height; 3D: level 0 whatever `lod`). `tests/es3_shaderc_repro.rs` pins that each sampler type lowers to one `Op::TexSize` of its dimension. Which of the 36 sizes per case is wrong (`GL_TEXTURE_BASE_LEVEL`, the NPOT levels, 3D at `lod` > 0) needs a board log | `src/shaderc/online/powervr/select.rs:1206-1262` | PowerVR, with the 3D/array work (pvr-3d, pvr-2darray). GC7000UL refuses `textureSize` at a non-literal level ("not selected yet"); V3D refuses 3D and array sizes (`NotYet`) |
| 4 | builtin_functions.common.modf (24: float..vec4 x lowp/mediump/highp x vertex/fragment) | 24 Fail "Result comparison failed" | compiled and linked; black | **back-end miscompile / hardware** (PowerVR). The IR is exact: `out0 + out1` is `in0` to the bit and `out1` is `float(int(in0))` on the CPU evaluator over dEQP's highp range (`modf_is_exact_in_the_ir`). The case is the only one of `common` with two colour outputs (`out0`, `out1` at locations 0 and 1): the second draw buffer is the likely place | `src/shaderc/online/lower.rs` `modf` (correct); `src/pvr/` second PBE target | PowerVR MRT (RC14); int-rt-pvr |
| 5 | precision.int and precision.uint, the 24 fragment cases | 24 Fail "m_program->isOk()" | 24 refused: `Usc(Select(Unsupported("a register read that nothing writes")))` | **back-end refusal** (PowerVR): a scalar integer colour output whose value is an ALU result (`v_in0 + 1`, `int(f)`); a copy of the input, a constant or a uniform builds (`int_output_from_arithmetic_is_the_powervr_shape`). The 24 vertex cases build and passed on the board. The front end gives one `Colour` output at location 0 for all 48. GC7000UL and V3D build all 48 | `src/shaderc/online/powervr/select.rs:3567` (the live-in check over a lane the integer output tail reads) | int-rt-pvr |
| 6 | fragdepth.write 8, fragdepth.compare 7 | 15 Fail "Compile failed (es3fFragDepthTests.cpp:196 / :314)" | 15 refused: `Usc(FragmentOutputsNotBuilt)` | **back-end refusal**, on every GPU: `gl_FragDepth` lowers to an `IoKind::Depth` output (`fragdepth_writes_lower_to_a_depth_output`); the PowerVR chain builds colour outputs only, the GC7000UL says "this output is not selected yet", the V3D `Program(NotYet("gl_FragDepth"))` | `src/shaderc/online/powervr/chain.rs:516`; `vivante/`, `v3d/` by their messages | no item: `gl_FragDepth` on the three back ends |
| 7 | builtin_functions.common.round 8, roundeven 8 (highp) | 16 Fail "Result comparison failed" | compiled; black | **folding** (front end), fixed on the host at `65cf0125` (`tests/es3_shaderc_fold.rs`), after the board run; board unverified | `src/shaderc/online/fold.rs`, `lower.rs` | done; the next AM62P run checks it |
| 8 | derivate.*.texture.msaa4 (15) | 15 Fail "checkFramebufferStatus == COMPLETE (es3fShaderDerivateTests.cpp:757)" | the same API verdict | **GL core / back end**: `glRenderbufferStorageMultisample(4, GL_RGBA8)` then the framebuffer is incomplete (`docs/es3-cts-failures.md` RC8) | `src/gl/framebuffer.rs`, the PowerVR MSAA renderbuffer | RC8 (multisample), not a shader cause |
| 9 | derivate.{dfdx,dfdy,fwidth}.texture.basic 6 and dfdy.texture.float_{fastest,nicest} 2, all `float_highp` / `vec2_highp` | 8 Fail (no message: a wrong pixel) | compiled; black | **back-end miscompile / hardware**: the derivative of a texture-sampled value at highp; the mediump and lowp twins and the `linear`/`nicest`/`fastest` dfdx and fwidth twins passed | PowerVR `dFdx`/`dFdy` selection | PowerVR; a board log of the pixel difference first |
| 10 | qualification_order.variables.valid.*storage* (7) | 7 Fail "Image comparison failed" | compiled; black | **back-end** (PowerVR `centroid in`): every one of the seven declares `smooth centroid` (the `.test` file); the board image predates item pvr-centroid (`936fcd76`, `ab6fd2ce`), which builds centroid inputs as smooth; host-verified by `tests/es3_pvr_centroid.rs`, board unverified | `src/shaderc/online/powervr/chain.rs` | done on the host; the next AM62P run checks it |
| 11 | uniform_block.valid.long_instance_name_{vertex,fragment} (2) | 2 Fail "failed to link" | 2 link refusals: "the vertex uniform named 'aaa...' (63 a's) is as long as the reflection's 64-byte name field stores, so it may have been truncated" | **GL core / link decision**: a 1,024-character instance name; the reflection's name field is 64 bytes and the link refuses a name that fills it (yakogl task #47) rather than pair a truncated one | `src/shaderc/online/reflect.rs`, the link | decision (`docs/es3-cts-failures.md` "Reflection name of 63+ characters") |
| 12 | metamorphic.bubblesort_flag (2) | 2 Fail "Nondeterministic rendering" | compiled; "Quick check fails on recipient" (black) | **hardware / back end**: two draws of the same program differ on the board | PowerVR | not reproducible on the host; a board log |
| 13 | builtin_functions.precision (987) | 987 NoResult (image 7 stopped at `clamp.highp_fragment.vec3`, `ResourceError`, RC16) | all 987 ran: 819 QualityWarning, 114 Pass, 54 Fail "Failed to set up framebuffer object: GL_INVALID_ENUM (glsShaderExecUtil.cpp:670)" | the `NoResult` is RC16, fixed on the host by formats-pvr (the draw cut into pieces). The 54 are **GL core**: `matrixcompmult`, `outerproduct` and `transpose` with a four-column matrix output (`mat4`, `mat4x2`, `mat4x3`: 18 cases x 3) render one column per draw buffer and attach `GL_COLOR_ATTACHMENT3`, which the GL core refuses while the back end's `DRAW_TARGETS` is 3 (RC14). The other 933 are compile-clean; their values are for the board | `src/gl/framebuffer.rs` (attachment count), `src/backend.rs` `DRAW_TARGETS` | RC14 (four draw buffers on the PowerVR); the next AM62P run for the 933 |
| | **total** | **1,397** (410 Fail + 987 NoResult) | | | | |

Row sums: 226 + 33 + 38 + 24 + 24 + 15 + 16 + 15 + 8 + 7 + 2 + 2 + 987 = 1,397.

## 4. The front end's share, and what this item changed

- **Front-end causes found: none.** Every program of the 410 board failures
  parses, type-checks and lowers (rows 1-12 compile on the host or are
  refused by a back end by name), and the host run refuses no mustpass
  program that should build. The one front-end cause in the group's
  history, `round`/`roundEven` at highp (row 7), was fixed by `65cf0125`
  before this item. So there is no front-end fix in this change, and
  `tests/fixtures/` did not move.
- **`tests/es3_shaderc_repro.rs`** pins what the classification rests on,
  so a front-end regression would show up as a front-end refusal rather
  than hide behind a back end's: the eight `gl_FragDepth` writers lower to
  a `Depth` output; the 48 precision programs lower to one integer colour
  output; `modf` is exact on the IR for dEQP's inputs and lowers to two
  colour outputs; `textureSize` lowers to a `TexSize` of the right dimension
  for ten sampler types; and on each back end a refusal of these programs
  is the code generator's, never a parse or type error. It runs in the
  `host-test,gpu-powervr,shaderc-online` and `gpu-vivante` rows of
  `tools/check.sh` (and by hand with the V3D configuration).
- **Before/after host counts**: unchanged, 20,616 cases, 265 back-end
  refusals, 2 link refusals, 0 front-end refusals on expected-build cases,
  at `70142d35` before and after (no compiler change).

## 5. Left for other items (the causes above, by owner)

1. PowerVR texture look-ups (row 1, 226): gradients, offsets on stacked
   images, integer results on cube/3D/array. The largest shaders bucket on
   the board. Parts belong to pvr-2darray and formats-pvr; `textureGrad*`
   and the offset forms on 2D arrays and 3D have no item.
2. PowerVR 3D and 2D-array sampling (rows 2 and 3, 71): pvr-2darray, pvr-3d.
3. PowerVR integer scalar output from an ALU result (row 5, 24): int-rt-pvr.
4. `gl_FragDepth` on all three back ends (row 6, 15): no item yet.
5. The second and fourth draw buffer on the PowerVR (rows 4 and 13, 24 + 54):
   RC14.
6. Board logs needed, not reproducible on the host: rows 9 and 12 (10
   cases), and the per-size detail of row 3.
7. Fixed on the host, unverified on the board: rows 7 and 10 (23 cases), and
   the 933 `builtin_functions.precision` values of row 13.
8. A decision: the 64-byte reflection name (row 11, 2).

The GC7000UL and V3D host runs of the group were not repeated here (item
host-gles3-run holds them); their differences from the PowerVR on these
programs are noted per row from `tests/es3_shaderc_repro.rs`.
