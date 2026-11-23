<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 2.0 CTS failure inventory

dEQP-GLES2, the mustpass list `tools/cts/gles2-mustpass.txt` (17,165 cases),
on the AM62P (PowerVR BXS-4-64), the i.MX8MP (Vivante GC7000UL) and the
Raspberry Pi 4 (V3D 4.2). This file says what is not Pass now and why that is
accepted. The numbers and the commit are in `tools/cts/RESULTS.md`. Earlier
rounds, their failures and their fixes are in git history. The fourth back
end, the Mali-G610 of the NanoPC-T6 (stage 5), is section M: its whole tree,
not yet at 0 Fail.

## Summary

Stage 1 is met: one main commit, `e9901869a`, reads **0 Fail** on the three
boards (2026-10-15/16, issue #870).

| Board | Pass | Fail | NotSupported | Warnings |
|---|---:|---:|---:|---|
| AM62P (17 images at 1x, the ms4 image for `multisample.*`) | 17,140 | 0 | 18 | 5 QualityWarning, 2 CompatibilityWarning |
| i.MX8MP (29 entries, 17,371 cases begun) | 17,318 | 0 | 34 | 19 QualityWarning |
| Pi 4 (29 rows) | 17,126 | 0 | 34 | 3 QualityWarning, 2 CompatibilityWarning |

Every NotSupported and warning case is in a bucket of section C. A new
failure on any board is a regression: add it here with the case name, the
commit and the cause.

`shaders.fragdata.invalid_assign_to_1` passes on every board since the ES 2.0
context reports one draw buffer (`gl_MaxDrawBuffers = 1`); it is listed in no
group of section C.

## C. NotSupported and QualityWarning buckets, with verdicts

A NotSupported result is not a dEQP failure. A bucket is acceptable when the
hardware or the ES 2.0 profile does not offer the feature. It would be "to
fix" if our code, not the profile, decided it. `tools/cts/es2-accepted.txt`
names every accepted NotSupported case; `tools/cts/tree-diff.py` reads it.

### NotSupported

| Bucket | Cases | AM62P | i.MX8MP | Pi 4 | Verdict | Class |
|---|---:|---:|---:|---:|---|---|
| `negative_api.texture.compressedtexsubimage2d_{invalid_size, invalid_target, level_max_cube, level_max_tex2d, neg_level_cube, neg_level_tex2d, neg_offset, neg_wdt_hgt, offset_allowed}` | 9 | 9 | 9 | 9 | acceptable: the test needs a compressed format that allows sub-image updates; ES 2.0 core requires none ("No suitable compressed formats found") | driver (profile) |
| `clip_control.{depth_mode_one_to_one, depth_mode_zero_to_one, errors, face_culling, initial, modify_get, origin, viewport_bounds}` | 8 | 8 | 8 | 8 | acceptable: `EXT_clip_control` is an optional extension we do not claim | driver (profile) |
| `multisample.{common_edge_big_quad, common_edge_small_quads, common_edge_viewport_quad, constancy_alpha_to_coverage, constancy_both, constancy_both_inverted, constancy_sample_coverage, constancy_sample_coverage_inverted, depth, num_samples_line, num_samples_polygon, proportionality_alpha_to_coverage, proportionality_sample_coverage, proportionality_sample_coverage_inverted, sample_coverage_invert, stencil}` | 16 | 0 (16 pass on the ms4 image; NotSupported at 1x) | 16 | 16 | acceptable: the default config has no multisample surface ("No multisample buffers"). The AM62P's 4x image runs them. The i.MX8MP can once its 4x MSAA works (#4); the Pi 4 has no 4x config | test harness (image config); hardware on the i.MX8MP (#4) |
| `multisampled_render_to_texture.readpixels` | 1 | 1 | 1 | 1 | acceptable: `EXT_multisampled_render_to_texture` is not claimed | driver (profile) |

Totals: 34 on each board at 1x (9 + 8 + 16 + 1). The AM62P's combined count
is 18, because its ms4 image replaces the 16 `multisample.*` cases.

### QualityWarning and CompatibilityWarning

dEQP counts a QualityWarning as a pass ("Pass with a quality warning"). Every
bucket below is acceptable.

| Bucket | Cases | Boards | Verdict | Class |
|---|---:|---|---|---|
| `texture.mipmap.cube.generate.{a8,l8,la88,rgb565,rgb888,rgba4444,rgba5551,rgba8888}_{fastest,nicest}` except `l8_nicest` and `la88_nicest` | 14 | i.MX8MP | GC7000 behaviour: one corner pixel of a level-3 face reads a coarser level in the texture unit. NXP's own driver warns on the same 14 cases and faces; 8-bit chains are pixel-identical to it | hardware-reference |
| `rasterization.interpolation.basic.{line_strip, line_strip_wide, lines}` and `.projected.{lines, lines_wide}` | 5 | AM62P, i.MX8MP | major-axis weights of lines; NXP's driver gives the same names on the narrow ones, images within 1 LSB | hardware-reference |
| `rasterization.interpolation.basic.lines`, `.projected.lines`, `.projected.lines_wide` | 3 | Pi 4 | the same family: line interpolation | hardware-reference |
| `flush_finish.{flush,flush_wait}` (CompatibilityWarning) | 2 | AM62P, Pi 4 | dEQP's own timing check; it flips between runs (0 to 2) | timing |

At 4x MSAA the AM62P also shows 9 CompatibilityWarnings in `rasterization`
(wide lines). That is a second configuration, not part of the whole-tree
count.

## H. GL_OES_texture_npot: the dEQP-GLES2 cases it touches

Every ES 2.0 build lists `GL_OES_texture_npot` in `GL_EXTENSIONS` on the
three GPUs (`src/gl/state.rs`, `NPOT_EXTENSION`). Its rules replace ES 2.0's:
a non-power-of-two texture may have a mip chain and wrap with `GL_REPEAT` or
`GL_MIRRORED_REPEAT`, `glGenerateMipmap` of one succeeds, and a level above 0
that is not a power of two is no error (`src/gl/texture.rs`; `NPOT_RESTRICTED`
is false in every build). glmark2's terrain needs this. The 18 cases below
run and must Pass: none throws NotSupported on this extension. All 18 pass
on the three boards at `e9901869a`.

### H.1 How each GPU holds the chain (host-tested)

| GPU | NPOT chain | Host tests |
|---|---|---|
| V3D (Pi 4) | whole chain; level i is max(w >> i, 1) x max(h >> i, 1), packed at its own pitch; the tiled copy follows Mesa's `v3d_setup_slices` (levels 0 and 1 at their size, the rest padded from the power-of-two level 1); `tools/v3d/tex_check.c` compares it with Mesa | `v3d::backend::backend_tests::a_generated_npot_chain_is_sampled_whole` |
| GC7000 (i.MX8MP) | whole chain at one 800-texel pitch, each level on 64 bytes; the descriptor names each level's address | `vivante::backend::backend_tests::a_generated_npot_chain_is_named_level_by_level` |
| PowerVR (AM62P) | a 2D texture is laid out as the chain of its power-of-two extent (`chain_layout_npot`, `pot_chain`); the TPU finds each level with its own arithmetic. A cube keeps its base level and samples level 0 under a mip filter | `gl::draw::tests::a_generated_npot_chain_is_complete_with_repeat`, `a_powervr_npot_cube_keeps_its_base_level`, `tests/pvr_npot_levels.rs` |

The GC7000 samples an NPOT texture with any wrap mode only when
`chipMinorFeatures1_NON_POWER_OF_TWO` is set (etnaviv's rule). The part's own
`minor_features[1]` word was never read from the board. NXP's feature
database answers for it: both entries matching the part's ids (ChipID
0x7000, ChipRevision 0x6204, ProductID 0x70003, CustomerID 0x11, both named
`gc7000ULN_v123`) set the bit (Mesa 26.2.1,
`src/etnaviv/hwdb/nxp/gc_feature_database.h`). It is recorded as
`src/vivante/backend.rs`'s `NON_POWER_OF_TWO` and pinned by
`tests/vivante_npot.rs`. The board's REPEAT cases pass, which agrees.

### H.2 What the CTS checks

Read in VK-GL-CTS `modules/gles2/functional/`:
`es2fTextureCompletenessTests.cpp` (CC) and `es2fNegativeTextureApiTests.cpp`
(NT). No other file of `modules/gles2` reads the extension.

| Where | What |
|---|---|
| CC:291-309 | `Incomplete2DSizeCase`: the relaxing extensions are `GL_OES_texture_npot` or `GL_NV_texture_npot_2D_mipmap` |
| CC:332-334 | each `glTexImage2D` expects `GL_INVALID_VALUE` only with **no** relaxing extension, level > 0 and an NPOT level; otherwise `GL_NO_ERROR` |
| CC:338-343 | only an NPOT **base** with the extension listed changes the expected colour to blue; otherwise black |
| CC:517-523, CC:920-926 | the wrap cases (2D, cube): extension listed, expected colour blue |
| CC:671, 701-703 | `IncompleteCubeSizeCase`: the error rule of CC:332-334; the expected colour stays black |
| NT:1341-1349 | `generatemipmap_npot_wdt_hgt`: extension listed, the case logs a line and returns; `es2fApiCase.cpp:54` set the result to Pass before the body |

Each completeness case draws one quad with a solid blue texture over a 64x64
viewport, and every pixel must equal the expected colour exactly.

### H.3 The 18 cases

| Case (`dEQP-GLES2.functional.` ...) | With the extension listed | Why yakogl passes it |
|---|---|---|
| `texture.completeness.2d.` `npot_t_repeat`, `npot_s_repeat`, `npot_all_repeat`, `npot_mirrored_repeat`, `repeat_width_npot`, `repeat_height_npot` (6) | runs; expects **blue** | an NPOT texture with REPEAT is complete (`incomplete_reason` has no NPOT arm) |
| `texture.completeness.cube.` `npot_t_repeat`, `npot_s_repeat`, `npot_all_repeat`, `npot_mirrored_repeat` (4) | runs; expects **blue**. The case binds its texture to `GL_TEXTURE_2D` and sets the images on the default cube texture (name 0) | the default cube object takes images and is sampled |
| `texture.completeness.2d.npot_size` | runs; a complete 255x255 chain, NEAREST_MIPMAP_NEAREST, REPEAT; no error at any level, expects **blue**. The only case that samples a mipmapped NPOT texture | the chain is accepted and complete |
| `texture.completeness.2d.` `npot_size_level_0`, `npot_size_level_1` (2) | run; no error; expect **black** | a level of the wrong size makes the texture incomplete |
| `texture.completeness.cube.` `npot_size_level_0`, `npot_size_level_1`, `npot_size_level_0_pos_x`, `npot_size_level_1_neg_x` (4) | run; no error; expect **black** | the same, per face |
| `negative_api.texture.generatemipmap_npot_wdt_hgt` | returns early; **Pass** | nothing to do |

The seven "expect black" cases also need the upload to succeed: a build that
returned `GL_INVALID_VALUE` for an NPOT level (the ES 2.0 rule without the
extension) would fail them. So the extension and the rules change together,
never in one place only. Other `*_npot` cases (`texture.format.*_npot`,
`texture.filtering.{2d,cube}.*_clamp_*_npot`, `texture.wrap.clamp_clamp_*_npot_*`,
`texture.size.2d.65x63_*`, `texture.size.cube.15x15_*`, `fbo.render.*npot*`)
use one level and CLAMP_TO_EDGE and do not read the extension list.

`tests/es2_npot.rs` replays the 18 on the host, one test per case named after
it (dots as underscores), on the PowerVR, GC7000 and V3D back end shapes. It
asserts `GL_NO_ERROR` after every call and whether the bind the back end
receives is complete (blue) or incomplete (black); it reads no pixels. It
needs `shaderc-online`:

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_npot
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_npot
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test es2_npot
cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test es2_npot
```

### H.4 Board results

All 18 pass on the three boards at `e9901869a`: none is among the failures
or the NotSupported cases of any tree, and the i.MX8MP's entry 10
(`texture.completeness.*`) reads 40 / 40 Pass. So the GC7000 honours REPEAT
and MIRRORED_REPEAT on an NPOT size, and the PowerVR samples the 255x255
chain of `2d.npot_size` correctly. The cube cases sample a solid-colour
texture, so they would pass a wrong wrap as well; `tests/gc_npot_repeat.rs`
checks the GC7000 descriptor on the host for that half.

### H.5 Still open

- The PowerVR's level arithmetic for an NPOT 2D chain is exercised on the
  board only by `2d.npot_size` (a 63x63 level 2 at a 64-texel pitch) and
  glmark2's terrain. It passes at `e9901869a`; `tests/pvr_npot_levels.rs`
  pins the layout on the host (`docs/notes/pvr-npot-levels.md`). A change to
  the PowerVR texture layout must keep both.
- One stale comment: the doc of
  `generate_mipmap_of_a_npot_texture_follows_the_reported_version` in
  `src/gl/texture.rs` calls the case's outcome "predicted". It is not: the
  case returns early and is Pass (H.2).
- The `NPOT_RESTRICTED` branches of the tests in `src/gl/draw.rs` and
  `src/gl/texture.rs` never take the restricted side in any build. They are
  revert checks, not stale text.

## M. Mali-G610 (NanoPC-T6): the whole tree at `8ff01055`

The `gpu-mali` back end on the NanoPC-T6 (RK3588, Valhall v10), on the
board's own Armbian (kernel 6.18.55-current-rockchip64) through panthor, with
the `tools/mali/panthor-gles` pair, `tools/cts/mali-deqp-run.sh` over the 76
rows of `tools/cts/mali-gles2-tree.txt` (issue #20, ROADMAP stage 5, issue
#7). Four slices, `tools/cts/log/2026-11-24-t6-gles2-S<n>-8ff01055/`, all
from one origin/main commit, `8ff01055`, run on 2026-11-24 in one boot of the
board. Every mustpass case has exactly one result (`mali-deqp-tally.py
--check-cover` exits 0). No case is Lost.

The board: all 76 chunks ended `done`, with 0 panthor kernel lines, 0 resets,
ssh answering after every chunk and the same boot id before and after each
slice. No exit line reports a GPU fault or a job timeout. No case crashed,
hung or faulted the GPU.

The block below is `mali-deqp-tally.py --tally --markdown` of the four
slices. The gate keeps it equal to them: the `tools/check.sh` row
`es2-cts-failures.md's Mali block equals the tally of its slices` runs
`--check-doc docs/es2-cts-failures.md --log tools/cts/log`. Each slice has a
`refusals.tsv`: per case, the first shader or program info log in which the
library refuses. So a cause `refused, <stage> shader: <why>` is the case's
own compile or link refusal, the text of its `MaliError`
(`src/shaderc/online/mali/mod.rs`). Any other cause is dEQP's result text
with numbers and names stripped. A refusal text longer than the info log
dEQP reads ends cut.

<!-- mali-deqp-tally begin -->

Slices: `2026-11-24-t6-gles2-S1-8ff01055`, `2026-11-24-t6-gles2-S2-8ff01055`, `2026-11-24-t6-gles2-S3-8ff01055`, `2026-11-24-t6-gles2-S4-8ff01055`
(commits `8ff01055`).

| Status | Cases |
| --- | ---: |
| Pass | 10950 |
| Fail | 5706 |
| NotSupported | 506 |
| QualityWarning | 3 |
| **Total** | **17165** |

| Status | Cause | Cases |
| --- | --- | ---: |
| Fail | refused, fragment shader: Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet | 865 |
| Fail | refused, fragment shader: Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet | 763 |
| Fail | refused, vertex shader: Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet | 603 |
| Fail | refused, vertex shader: Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet | 371 |
| Fail | Image comparison failed | 306 |
| Fail | refused, vertex shader: Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is no | 299 |
| Fail | refused, fragment shader: Valhall: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, is not lowered | 271 |
| Fail | refused, fragment shader: Valhall back end defect: an invariant of the pipeline did not hold (IrIndex) | 206 |
| Fail | refused, vertex shader: Valhall: a uniform array is not lowered yet | 193 |
| Fail | refused, fragment shader: Valhall: an integer or boolean value is not lowered yet | 188 |
| Fail | refused, fragment shader: Valhall: a texture coordinate other than a vec2 varying read whole or two constants is not lowered yet | 182 |
| Fail | refused, vertex shader: Valhall: a loop in a vertex shader is not lowered yet | 176 |
| Fail | refused, fragment shader: Valhall: a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16) is not lowered yet | 168 |
| Fail | refused, fragment shader: Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform first) is not lowered yet | 152 |
| Fail | refused, vertex shader: Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet | 144 |
| Fail | refused, vertex shader: Valhall: a fused multiply-add other than a matrix product's (a built-in function's) is not lowered yet | 142 |
| NotSupported | refused, vertex shader: Valhall: a uniform other than a float, vec2, vec3, vec4 or mat4 is not lowered yet | 92 |
| NotSupported | refused, fragment shader: Valhall: a loop Mesa keeps (a run-time bound) in a mediump fragment shader (its fp16 vectors carried across the back edge) is not lowered yet | 87 |
| Fail | refused, fragment shader: Valhall: a constant product in a shape Mesa's algebraic rules move or fold (a constant times a product with a constant, a negation of a negation, a constant in a fused multiply-add) is | 86 |
| NotSupported | refused, fragment shader: Valhall: arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs) is not lowered yet | 78 |
| NotSupported | refused, vertex shader: Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet | 73 |
| Fail | refused, vertex shader: Valhall: a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member, is not lowered yet | 61 |
| Fail | refused, vertex shader: Valhall: a vertex shader texture look-up other than 2D or cube look-ups at a level (a size query), or its fp16 texels read by a mediump operation other than a product or sum, is not lower | 60 |
| Fail | refused, fragment shader: Valhall: a fragment varying load or conversion of a form no reference draws (a colour lane of unmeasured precision beside a mediump varying, an aligned pair converted apart, lanes conve | 50 |
| Fail | refused, fragment shader: Valhall: fp16 arithmetic on gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor and fract of x and y is not lowered yet | 43 |
| Fail | refused, fragment shader: Valhall: a texture look-up other than texture2D(sampler2D, coordinate) (a bias, a level, a projection, a cube map, a size query) is not lowered yet | 42 |
| Fail | refused, vertex shader: Valhall: control flow (if, a loop, a return before the end) is not lowered yet | 41 |
| Fail | refused, fragment shader: Valhall: two varying lanes of equal value (Mesa's link merges them) is not lowered yet | 40 |
| Fail | refused, fragment shader: Valhall: a constant operation on or to a value that is not a finite normal float is not lowered yet | 33 |
| Fail | refused, fragment shader: Valhall: control flow (if, a loop, a return before the end) is not lowered yet | 32 |
| Fail | refused, vertex shader: Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet | 31 |
| NotSupported | refused, fragment shader: Valhall: control flow (if, a loop, a return before the end) is not lowered yet | 27 |
| NotSupported | refused, fragment shader: Valhall: a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside | 23 |
| NotSupported | refused, fragment shader: Valhall: an integer or boolean value is not lowered yet | 22 |
| Fail | refused, vertex shader: Valhall: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), is not lowered yet | 20 |
| NotSupported | refused, vertex shader: Valhall: a loop in a vertex shader is not lowered yet | 20 |
| Fail | refused, fragment shader: Valhall: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) i | 16 |
| NotSupported | No multisample buffers | 16 |
| Fail | refused, fragment shader: Valhall: a -0.0 that a comparison reads through a variable, or that another value reads too (Mesa's rules make it 0.0 there), is not lowered yet | 14 |
| NotSupported | refused, vertex shader: Valhall: a vertex shader uniform that is not highp (Mesa's fp16 uniform), other than a float, vec2, vec3, vec4 or mat4 that is neither an array nor a struct member, is not lowered yet | 12 |
| Fail | Failed to compile shader | 10 |
| NotSupported | refused, fragment shader: Valhall: a break or continue behind a run-time condition in a loop a mediump fragment shader unrolls (Mesa's ladder of fp16 selects) is not lowered yet | 10 |
| Fail | refused, fragment shader: Valhall back end defect: an invariant of the pipeline did not hold (LaneCycle) | 9 |
| Fail | refused, vertex shader: Valhall: a vertex shader whose attribute numbering waits for a fold Mesa's back end makes after it numbers the locations, of a form the walk does not model, is not lowered yet | 9 |
| NotSupported | No suitable compressed formats found | 9 |
| NotSupported | refused, vertex shader: Valhall: two varying lanes of equal value (Mesa's link merges them) is not lowered yet | 9 |
| Fail | Got invalid error | 8 |
| Fail | glDrawElements(GL_TRIANGLES, DE_LENGTH_OF_ARRAY(indices), GL_UNSIGNED_SHORT, &indices[#]): glGetError() returned GL_INVALID_OPERATION at es#fUniformApiTests.cpp:# | 8 |
| Fail | refused, vertex shader: Valhall: three or more additions or multiplications in a chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered yet | 8 |
| Fail | refused, vertex shader: the Valhall back end of a vertex shader at the link's attribute locations is not lowered yet | 8 |
| NotSupported | Required extension EXT_clip_control is not supported | 8 |
| Fail | Program link failed: <name> at glsAttributeLocationTests.cpp:# | 7 |
| Fail | refused, fragment shader: Valhall: a fragment colour of more than 255 vectorised operations and operands is not lowered yet | 7 |
| Fail | refused, vertex shader: Valhall back end defect: an invariant of the pipeline did not hold (Encode) | 6 |
| Fail | refused, vertex shader: Valhall: a gl_Position lane left unwritten is not lowered yet | 6 |
| NotSupported | refused, vertex shader: Valhall: a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform, is not lo | 6 |
| NotSupported | refused, fragment shader: Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet | 5 |
| Fail | got invalid value | 4 |
| Fail | refused, fragment shader: Valhall: three or more additions or multiplications in a chain (GLSL's do_rebalance_tree reassociates such a tree) is not lowered yet | 4 |
| NotSupported | refused, fragment shader: Valhall: a discard other than a highp fragment shader's `if (a < b) discard;` with no else (an unconditional one, one among other statements, one under another if, a mediump shader's) i | 4 |
| Fail | expected GL_TRUE | 3 |
| Fail | refused, fragment shader: Valhall: a fragment shader operand of a form or in a place no reference draws (a lane alone first in a sum, a uniform fi | 3 |
| QualityWarning | Interpolation was calculated using projected coordinateds | 3 |
| Fail | refused, vertex shader: Valhall: a shader of more than 2048 IR values (or 1536 instructions before selection) is not lowered yet | 2 |
| NotSupported | refused, fragment shader: Valhall: a uniform read at a run-time index other than a float array's element or a mediump fragment shader's vec4 array first in the block read once, indexed by an int uniform, is not | 2 |
| Fail | Fail | 1 |
| Fail | Fail, expected LINK_STATUS to be TRUE.: <name> at es#fShaderApiTests.cpp:# | 1 |
| Fail | Runtime check failed: <name> at es#fShaderApiTests.cpp:# | 1 |
| Fail | refused, fragment shader: Valhall: a constant fp16 does not hold (past its normal range, or a highp colour or point size constant not exactly) is | 1 |
| Fail | refused, fragment shader: Valhall: a loop past Mesa's unroll limits or with two run-time breaks, or a kept loop with more than two breaks, a continue other than the step's at its top or the colour written inside | 1 |
| Fail | refused, fragment shader: Valhall: a texture result's lanes other than its own pairs in order (a swizzle, a lane alone) is not lowered yet | 1 |
| NotSupported | EXT_multisampled_render_to_texture is not supported at es#fMultisampledRenderToTextureTests.cpp:# | 1 |
| NotSupported | refused, fragment shader: Valhall: a mediump fragment shader uniform other than a float, vec2, vec3, vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or one declared highp, is not lowered | 1 |
| NotSupported | refused, vertex shader: Valhall: a varying lane computed from no attribute (a constant or a uniform alone, which Mesa's link moves or rewrites) is not lowered yet | 1 |

<!-- mali-deqp-tally end -->

### M.1 The non-Pass cases

6,215 cases are not Pass: Fail 5,706, NotSupported 506, QualityWarning 3. In
the tree before this one (`37305608` with 25 rows rerun at `a2482523`, M.2)
9,812 were not Pass.

**5,829 have a compile or link refusal in their own log** (Fail 5,357,
NotSupported 472). The block lists every refusal text with its count; that
list, by size, is the work order of the compiler (issue #11251: the general
front half, which takes the place of the per-shape rules these texts name).
The NotSupported ones are refusals of a feature that GLSL ES 1.00 Appendix A
makes optional, which dEQP then reports as NotSupported (`Loop type is not
supported`, `Dynamic indexing not supported`, ...). 9 Pass cases log a
refusal too: `shader_api.shader_source.*`, which read back the source and
never draw.

**386 have no refusal in their log** (Fail 349, NotSupported 34,
QualityWarning 3). In the tree before they were 84: the compiler now builds
shaders it refused then, and 314 cases that were refused now reach a draw, a
link or a comparison and fail there (rows 16 to 18). No row below is a case
that passed in the tree before. The "Cases" column is the count now; "Was"
is the count in the tree before. Rows 1 to 15 keep the numbers of the tree
before (tests and notes cite them); M.3 has the history of the fixed ones.

| # | Cause | Status | Was | Cases | Where | Fix and owner |
|---|---|---|---:|---:|---|---|
| 1 | A blend shader refused at the draw | Fail | 0 | 0 | `fragment_ops.blend` (1,060 Pass) | fixed, `961cbf77` (M.3) |
| 2 | Missing or wrong pixels and `GL error` in chunks whose exit line counts refused draws | Fail | 0 | 0 | `fragment_ops.random` (100 Pass) | fixed with row 1 (M.3) |
| 3 | A depth or stencil object drawn in another framebuffer's pair refused at the draw | Fail | 0 | 0 | `fbo.render.{recreate_depthbuffer,recreate_stencilbuffer,shared_depthbuffer}.*` (70 Pass) | fixed, `e918e015` (M.3) |
| 4 | A draw failed with `GL_OUT_OF_MEMORY` (a counting defect in the frame's varying check) | ResourceError | 0 | 0 | `shaders.matrix`, `shaders.random`, `shaders.conditionals`, `fragment_ops.interaction` | fixed, `b2585cf2` (M.3) |
| 5 | Wrong pixels from a vertex-shader cube look-up with `GL_REPEAT` and a `LINEAR` filter near a face edge, no refusal; dEQP: `Image comparison failed` | Fail | 3 | 3 | `texture.vertex.cube.filtering.linear_mipmap_linear_{linear,nearest}_repeat`, `.wrap.repeat_repeat` | open: issue #7489 |
| 6 | An incomplete cube map refused at the draw | Fail | 0 | 0 | `texture.completeness.cube.*` (21 Pass) | fixed, `c50f2c41` (M.3) |
| 7 | An error code other than the one the case expects (`Got invalid error`), in cases that build a program from dEQP's smallest shader pair first; the vertex shader `void main (void) { gl_Position = vec4(0.0); }` does not compile and the cases print no info log | Fail | 20 | 8 | `negative_api.shader.{get_active_attrib,get_active_uniform,get_attrib_location,get_uniform_location}` 4, `negative_api.vertex_array.{draw_arrays,draw_elements}{,_incomplete_primitive}` 4 | 12 Pass now (`negative_api.shader.uniform*` 10, `negative_api.state.get_uniform{fv,iv}` 2); the 8: issue #11328 |
| 8 | `GL_INVALID_OPERATION` at a draw (an unread attribute between read ones, a matrix column beyond column 0) | Fail | 0 | 0 | `default_vertex_attrib.{mat2,mat3}`, `shaders.random`, `shaders.operator`, `fragment_ops.interaction` | fixed, `24c04082` (M.3) |
| 9 | Wrong pixels from a mipmapped cube map | Fail | 0 | 0 | `texture.mipmap.cube.{basic,bias,projected}.linear_*` (28 Pass) | fixed, `0afaf24d`, `0c3a7729` (M.3) |
| 10 | A query value: `current_program_binding_*` 3 (`expected GL_TRUE`), `state_query.shader.{delete_status,program_active_uniform_types,program_validate_status,shader_compile_status}` 4 (`got invalid value`; the log of `delete_status` reads compile status 0 for the vertex shader of row 7) | Fail | 7 | 7 | `state_query` | as row 7: issue #11328 |
| 11 | `shader_api.compile_link.{compile_vertex_shader,link_vertex_fragment}` 2 (`checkCompileStatus` fails on the vertex shader of row 7); `shaders.texture_functions.vertex.texturecubelod` 1 (`Fail`, wrong pixels) | Fail | 3 | 3 | `shader_api`, `shaders.texture_functions` | `compile_link`: issue #11328; `texturecubelod`: as row 5, issue #7489 |
| 12 | A feature the context does not offer: `EXT_clip_control` 8, no multisample buffers 16, `EXT_multisampled_render_to_texture` 1, no compressed format with sub-image updates 9: the 34 of section C | NotSupported | 34 | 34 | `clip_control`, `multisample`, `multisampled_render_to_texture`, `negative_api.texture` | accepted (section C) |
| 13 | `rasterization.interpolation.basic.lines`, `.projected.lines`, `.projected.lines_wide`: the Pi 4's three of section C | QualityWarning | 3 | 3 | `rasterization` | accepted (section C) |
| 14 | Wrong pixels after a colour attachment without alpha is rebound next to a stencil buffer (`Image comparison failed`); the four passed at `37305608` and fail since `a2482523` | Fail | 4 | 4 | `fbo.render.recreate_colorbuffer.rebind_{rbo_rgb565,tex2d_rgb}_{stencil_index8,depth_component16_stencil_index8}` | open: issue #11329 (`e918e015` and `0c3a7729` are the commits to read first) |
| 15 | `Failed to compile shader`: the program's link is refused, `a Mali link whose stages place a varying at different offsets is not lowered yet` (`src/shaderc/online/mali/link.rs`). The text is in the case's log; `refusals.tsv` does not record it, because the run tool's extractor looks for the word `refused` only | Fail | 10 | 10 | `shaders.random.{basic_expression,comparison_ops,scalar_conversion,conditionals,swizzle}.fragment.*` | open: issue #11309 (the link, and the extractor) |
| 16 | Wrong pixels from a shader with a bool or bvec value, no refusal (`Image comparison failed`); every one was a compiler refusal in the tree before. `swizzles` 162 (vertex 108, fragment 54: every bvec swizzle case that compiles), `conversions` 111 (to bool or bvec 101, to ivec 10), `functions` 14, `linkage.uniform_struct*` 12 | Fail | 0 | 299 | `shaders.swizzles.vector_swizzles.mediump_bvec{2,3,4}_*`, `shaders.conversions`, `shaders.functions`, `shaders.linkage` | open: issue #11306 (cause not yet proven) |
| 17 | A draw refused: `a vertex shader that reads more than one texture is not built yet` (`src/mali/backend.rs`); dEQP: `GL_INVALID_OPERATION` at `glDrawElements` | Fail | 0 | 8 | `uniform_api.value.assigned.{by_pointer,by_value}.render.{array_in_struct,basic_struct,nested_structs_arrays,struct_in_array}.sampler2D_samplerCube_vertex` | open: issue #11308 |
| 18 | `Program link failed`: `attribute 'a_8' has location 16, beyond the 16 vertex inputs`. The shader declares 32 attributes and reads 16; the 16 under `if (0 != 0)` are not active and need no location | Fail | 0 | 7 | `attribute_location.bind_aliasing.max_inactive_{float,vec2,vec3,vec4,mat2,mat3,mat4}` | open: issue #11307 |
| | **Total** | | **84** | **386** | | |

Rows 5, 7, 10, 11 and 14 are the same cases as in the tree before, except
the 12 of row 7 that pass. Rows 16 to 18 are cases the compiler refused
before: each is nearer to Pass than a refusal, and each is a defect to fix
on its own issue.

### M.2 Against the earlier trees

**The tree before** (the four slices of `37305608` of 2026-11-15 with 25
rows replaced by the rerun `2026-11-21-t6-gles2-R373-a2482523`, kept under
`tools/cts/log/`) read 7,353 Pass, 8,836 Fail, 973 NotSupported, 3
QualityWarning. Case by case:

- **3,597 cases now pass** (3,374 from Fail, 223 from NotSupported): 2,652
  of `shaders` (`operator` 1,233, `preprocessor` 256, `swizzles` 252, `loops`
  235, `conversions` 190, `indexing` 119, `functions` 105,
  `constant_expressions` 99, `constants` 49, `scoping` 32, `matrix` 30,
  `linkage` 21, `random` 14, others 17), 571 of `vertex_arrays` (the whole
  group), 242 of `uniform_api`, 101 of `draw` (the whole group), 16 of
  `negative_api`, 7 of `attribute_location`, 3 of `state_query`, 3 of
  `fragment_ops`, 2 of `light_amount`.
- **No case that passed fails.**
- 244 refused cases moved from NotSupported to Fail (`shaders.loops` 240,
  `shaders.return` 4), with a refusal in both trees. dEQP asks the context
  whether an optional loop form is supported (`glu::ContextInfo`, a small
  shader of dEQP's own per form) and reports a refused case of an
  unsupported form as NotSupported (`es2fShaderLoopTests.cpp`). The forms
  read as supported now, so the same refusal is a Fail. These cases count
  against the 0 Fail criterion until their loops are lowered.

**The first whole tree** (seven slices of 2026-11-12/13, commits `a50d87b5`
to `9b5daae8`, kept under `tools/cts/log/`) read 5,260 Pass, 10,898 Fail,
971 NotSupported, 33 ResourceError. Case by case:

- **5,700 cases now pass** (5,446 from Fail, 221 from NotSupported, 33 from
  ResourceError).
- **10 cases that passed do not:**
  - 7 with wrong pixels and no refusal: the 4 of row 14 and the 3 of row 5;
  - 3 refused now where the first tree's commit compiled them:
    `shaders.operator.sequence.no_side_effects.{lowp,mediump}_vec4_fragment`
    by "a gl_FragColor lane other than the varying's own lane or a
    constant", and `shaders.return.conditional_return_always_fragment` by "a
    fragment varying load or conversion of a form no reference draws". A
    refusal there is the compiler's rule (words equal to Mesa's, or a named
    refusal); the three come back when the two forms are lowered with a
    reference.

### M.3 Landed on the host, and what the rerun showed

Plan #373's causes 3 to 8 (table rows 1 to 6, 8, 9 and 11) have host fixes.
This is the history of the tree before the current one: the board rerun of
origin/main `a2482523` (`2026-11-21-t6-gles2-R373-a2482523`), whose figures
M.2 gives. The block and M.1 hold the tree at `8ff01055`. Each entry gives the root cause, the
commit that landed the fix, the host test that pins it, and what was
expected from the host pins on the Mali mock (no pixels of a board).
"Expected" is that forecast; the **Rerun** lines at the end are the board's
result, case by case.

- **Row 1, blend (cause 3):** not a missing mechanism. The first tree's
  build refused every blend equation that needs a blend shader (`a blend
  shader other than a constant colour's ... is not built yet`). `961cbf77`
  builds blend shaders of any equation into RGBA8 at 1x/4x with Mesa's words
  (`src/mali/blend.rs`, `blend_nir.rs`, `blend_nir_rules.rs`);
  `can_fixed_function` / `needs_shader` equal Mesa's rule (`e33ef7b1`,
  `blend_choice_tests`). Pins: `tests/mali_blend_states.rs` (every
  `fragment_ops.blend` state through the GL core on the mock, none refused)
  and `src/mali/frame_blend_tests.rs` (`b5f882ea`, blended draws in the
  frame). Expected: the 828 draw their quads; a case that still fails shows a
  pixel difference, not a refusal. Blend shaders for RGB565, XRGB8888 and
  RGBA16F targets stay refused and are not in this tree. Design:
  `docs/notes/mali-blend-shaders.md`.
- **Row 2, `fragment_ops.random` (cause 4):** the library named only the
  chunk's first refusal, so the refused case was unknown. The host replay
  found the same cause as row 1 (the blend shader refusal), so the 100 follow
  row 1. `fc195a8d` writes one `yakogl-mali-refusal: <errno> <reason>` line
  per refused draw (a repeated reason once); `b48f3b49`
  (`tools/cts/mali-deqp-run.sh`) records `refusals/<label>.txt`, and
  `4cb466e4` / `b950cb89` make `mali-deqp-tally.py --per-case` split by it.
  Pin: `tests/mali_fragment_random.rs` (`07952272`: the states as drawn, none
  refused; a reduced version, the dEQP generator's seeds are not reproduced).
  Expected: most of the 100 pass with row 1; any that stays is named by its
  own refusal line (the stdout order of that line is for the rerun to check;
  `docs/notes/mali-frame-refusals.md`).
- **Row 3, depth/stencil objects shared between framebuffers (cause 5):** two
  defects. 21 cases (`shared_*`, `recreate_*.no_rebind_*`) were refused
  because a plane drawn in another pair's buffer was not built; 14
  (`recreate_*.rebind_*`) drew wrong pixels because the scissored stencil
  clear held past `glDeleteRenderbuffers` also wrote the clear colour.
  `e918e015` takes the plane into the frame's own pair's buffer and writes
  colour only when the clear says so. Pins: `tests/mali_zs_sharing.rs`
  (`4cc135c1`) and `src/mali/backend_tests.rs`. Expected: the 35 draw
  unrefused; their pixels are compared on the board only. The ES 2.0 core
  reports the D16 + S8 pair complete where Mesa's ES 3 context says
  unsupported, so the back end draws it. Design:
  `docs/notes/mali-zs-sharing.md`.
- **Row 4, `GL_OUT_OF_MEMORY` (cause 6):** a counting defect, not memory.
  `StageMeta::of` counted the placeholder rows of vertex varyings that fold to
  constants, so the frame's varying check failed with `-EINVAL`, reported as
  `GL_OUT_OF_MEMORY`. `b2585cf2` (`src/mali/program.rs`) counts only rows
  with components. Pins: `tests/mali_draw_oom.rs` (`69cee8fe`) and
  `src/mali/program_tests.rs`. Expected: the 33 return no error; the fragment
  shader must output the folded constants as Mesa's does, which only the
  board checks. Design: `docs/notes/mali-draw-limits.md`.
- **Row 5, vertex-shader cube look-ups (cause 7):** two causes, no refusal.
  The sampler wrote Mesa's ES 3 cube words (seamless, S/T/R clamped) in an ES
  2.0 build, where Mesa is not seamless and keeps GL's wraps: `0afaf24d`
  fixes the 10 `wrap.*` and `linear_mipmap_linear_*` cases. The other six
  (`nearest_mipmap_*_linear_*`) need ES 2.0's magnification switch at level of
  detail 0.5 where the Valhall unit switches at 0: `ea40c2b7`
  (`src/shaderc/online/mali/mod.rs`, report-es2 builds only). Pins:
  `tests/mali_cube_samplers.rs` (`ce63b181`), `tests/mali_vcube_magswitch.rs`
  (`90a000ac`, its fix cases run in a report-es2 build) and the lib tests
  `vertex_cube_samplers_are_mesa_s` and
  `a_keyed_vertex_cube_look_up_switches_to_magnification_at_one_half`.
  Expected: the 16 pass. Design: `docs/notes/mali-cube-textures.md` (i), (k).
- **Row 6, an incomplete cube map:** the draw was refused because GL's black
  texel was bound only as a 2D image. `c50f2c41` binds Mesa's fallback, six
  1x1 black faces, so the look-up reads (0, 0, 0, 1) and the refusal is gone.
  Pin: `tests/mali_cube_complete.rs` (GL errors and `draws_refused` only; the
  descriptor is in `src/mali/backend_tests.rs`). Expected: the 16
  `texture.completeness.cube.*` draw with no `GL_INVALID_OPERATION`; their
  pixels are for the board.
- **Row 8, `GL_INVALID_OPERATION` at a draw:** the `default_vertex_attrib`
  `mat2`/`mat3`, `shaders.operator.sequence` and the vertex cases of
  `shaders.random` and `fragment_ops.interaction` read attributes that are
  not one run from location 0 (a matrix column beyond column 0, an unread
  attribute between read ones) and were refused as
  `NOT_BUILT_ATTRIBUTE_RECORDS` (`-ENOTSUP`). `24c04082` finds the record by
  column range (`aeba68f8` already numbered the attributes as Mesa does).
  Pin: `tests/mali_attr_gap.rs`. Expected: those cases draw; the 23 are not
  all proven to be this cause (`shaders.random` `vertex.96` and others may
  stay), and the rerun's per-case refusal line names the rest. Design:
  `docs/notes/mali-attribute-records.md`.
- **Row 9, mipmapped cube (`texture.mipmap.cube.*.linear_*`):** the fragment
  stage's cube sampler had the same ES 3 words as row 5 (seamless, clamped
  wraps). `0afaf24d` made the sampler follow the context for every stage;
  `0c3a7729` (misc8-fix) pins it for the fragment stage in the lib test
  `fragment_cube_samplers_follow_the_context`, beside
  `tests/mali_cube_samplers.rs`. Expected: the 6 (`basic`, `bias`,
  `projected`) pass. There is no ES 2.0 drm-shim trace (`MESA_PAN_TREE` has
  no drm-shim build); the ES 2.0 words follow Mesa's sources, so the board is
  the first pixel check.
- **Row 11, `stencil_clear.rbo_rgb565_*` 2:** the GL core keeps an RGB565
  renderbuffer in RGBA8 storage and the back end ignored `target_alpha_bits`,
  so the case's draws wrote an alpha other than 1. `0c3a7729` takes alpha out
  of the draw's mask when the layout stores alpha and the GL format has none.
  Pins: `tests/mali_rgb565_stencil.rs` (`239b538d`) and
  `src/mali/frame_formats_tests.rs`, whose descriptors equal Mesa's draws
  field for field. Expected: the 2 pass. Design:
  `docs/notes/mali-frame-formats.md`.
- **Rows 7 and 10 wait.** Their cases and the two `shader_api.compile_link`
  cases of row 11 wait for the stub vertex shader constant fix of the online
  Mali compiler (plan #373, apierr-fix); no fix has landed for them and the
  rerun is expected to read them as in the table. `texturecubelod` (row 11)
  has the row 5 sampler fix and is first checked on the board.
- Rows 12 and 13 are accepted (section C). Until the rerun, rows 7 and 10 and
  the rest of row 8 need each case's own refusal line before an owner can
  hold them.
- **Rerun (`a2482523`, 25 rows, 5,944 cases: Pass 3,625, Fail 2,295,
  NotSupported 24; at `37305608` the same rows read Pass 1,728, Fail 4,159,
  NotSupported 24, ResourceError 33):** every forecast above held on
  the board except the entries named here. Row 1: all 1,060
  `fragment_ops.blend` cases Pass, 828 of them new. Row 2: the 100
  `fragment_ops.random` Pass (none was refused; no `yakogl-mali-refusal:`
  line at all). Row 3: the 70 `recreate_*` and `shared_*` Pass. Row 4: the 33
  Pass (0 ResourceError left). Row 6: the 21 `completeness.cube` Pass. Row 8:
  22 of 23 Pass, the other is a compiler refusal. Row 9: the 28
  `mipmap.cube` Pass. Row 11: the 14 `stencil_clear` Pass.
  **Not as forecast:** row 5 passes 42 of 45 `texture.vertex.cube` (the 16
  fixed, 3 others that passed before now fail: `linear_mipmap_linear_*_repeat`
  and `wrap.repeat_repeat`); `texturecubelod` still fails; the 4
  `recreate_colorbuffer.rebind_*` cases of `rgb565` and `tex2d_rgb` with a
  stencil attachment, which passed, now fail (M.1 row 14). 9 fragment
  `shaders.random` cases read `Failed to compile shader` with no refusal text
  (row 15). No other case moved from Pass to non-Pass.
- 0 Fail is the stage 5 criterion. After that rerun 8,836 Fail remained,
  8,789 of them with a compiler refusal. The tree at `8ff01055` (the block
  and M.1) has 5,706 Fail, 5,357 of them with a compiler refusal. The tree
  is rerun from one origin/main commit as the refusals move, and replaces
  the block, M.1 and M.2.
