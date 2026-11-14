<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 2.0 CTS failure inventory

dEQP-GLES2, the mustpass list `tools/cts/gles2-mustpass.txt` (17,165 cases),
on the AM62P (PowerVR BXS-4-64), the i.MX8MP (Vivante GC7000UL) and the
Raspberry Pi 4 (V3D 4.2). This file says what is not Pass now and why that is
accepted. The numbers and the commit are in `tools/cts/RESULTS.md`. Earlier
rounds, their failures and their fixes are in git history. The fourth back
end, the Mali-G610 of the NanoPC-T6 (stage 5), is section M: its first whole
tree, not yet at 0 Fail.

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

## M. Mali-G610 (NanoPC-T6): the first whole tree

The `gpu-mali` back end on the NanoPC-T6 (RK3588, Valhall v10), on the
board's own Armbian (kernel 6.18.55-current-rockchip64) through panthor, with
the `tools/mali/panthor-gles` pair, `tools/cts/mali-deqp-run.sh` over the 76
rows of `tools/cts/mali-gles2-tree.txt` (issue #20, ROADMAP stage 5, issue
#7). Seven slices, `tools/cts/log/<slice>/`, from four origin/main commits:
`a50d87b5` (S1, S1b, 2026-11-12), `1bd2074f` (S2, S2b), `4ecefc7b` (S3,
S3b), `9b5daae8` (S4, all 2026-11-13). `src/mali`, `src/shaderc/online/mali`
and `src/gl` are the same in `1bd2074f`, `4ecefc7b` and `9b5daae8`, and the
first two are also unchanged at origin/main `d1e77916`. S1 and S1b ran
`a50d87b5`'s, 26 commits to the Mali compiler and back end older. No slice
reruns another and
no case is Lost: every mustpass case has exactly one result
(`mali-deqp-tally.py --check-cover` exits 0 against origin/main
`d1e77916`).

The board: all 76 chunks ended `done`, with 0 panthor kernel lines, 0 resets,
ssh answering after every chunk and the same boot id before and after each
slice. No exit line reports a GPU fault or a job timeout. No case crashed,
hung or faulted the GPU.

The block below is `mali-deqp-tally.py --tally --markdown` of the seven
slices, and `--check-doc` keeps it equal to them. Its cause column reads
`refused:this` for every case of a chunk whose exit line names a refusal:
the tally takes the word after "refused" in `first_refused=compile: online
shaderc: the code generator refused this ... shader`. The table after the
block names the causes.

<!-- mali-deqp-tally begin -->

Slices: `2026-11-12-t6-gles2-S1-a50d87b5`, `2026-11-12-t6-gles2-S1b-a50d87b5`, `2026-11-13-t6-gles2-S2-1bd2074f`, `2026-11-13-t6-gles2-S2b-1bd2074f`, `2026-11-13-t6-gles2-S3-4ecefc7b`, `2026-11-13-t6-gles2-S3b-4ecefc7b`, `2026-11-13-t6-gles2-S4-9b5daae8`
(commits `1bd2074f`, `4ecefc7b`, `9b5daae8`, `a50d87b5`).

| Status | Cases |
| --- | ---: |
| Pass | 5260 |
| Fail | 10898 |
| NotSupported | 971 |
| QualityWarning | 3 |
| ResourceError | 33 |
| **Total** | **17165** |

| Status | Cause | Cases |
| --- | --- | ---: |
| Fail | refused:this | 10017 |
| NotSupported | refused:this | 970 |
| Fail | Image compare failed | 828 |
| ResourceError | refused:this | 33 |
| Fail | Image comparison failed | 32 |
| Fail | Got unexpected error | 21 |
| QualityWarning | refused:this | 3 |
| NotSupported | EXT_multisampled_render_to_texture is not supported at es#fMultisampledRenderToTextureTests.cpp:# | 1 |

<!-- mali-deqp-tally end -->

### M.1 The non-Pass cases by cause

11,905 cases are not Pass. A case's cause is what dEQP reports for it,
together with its chunk's `yakogl-mali-exit` line (`draws=`, `refused=`,
`first_refused=`). The library logs only the first refusal of a chunk by
name, so a case's own `MaliError` variant is not recorded. Owners: plan #309
holds every refusal of `src/mali` and `src/shaderc/online/mali` (#33 for the
compiler); "new" means no open issue holds the cause yet.

| # | Cause | Status | Cases | Where (largest groups) | Owner |
|---|---|---|---:|---|---|
| 1 | The online compiler refused a shader. dEQP: `Failed to compile shader (program)`, `Compile failed`, `could not build program`, `expected shaders to compile and link properly, but failed to compile.`, `Failed to retrieve uniform location`, ...; the chunk's exit line names a `MaliError` (M.2) | Fail | 9,851 | `shaders.operator` 2,576, `shaders.random` 1,765, `uniform_api` 1,267, `shaders.swizzles` 648, `vertex_arrays` 571, `shaders.matrix` 527, `shaders.conversions` 470, `shaders.constant_expressions` 411, `shaders.preprocessor` 260, `shaders.indexing` 186, `shaders.functions` 171, `texture.filtering` 112, `texture.specification` 106, `draw` 101, `fragment_ops.interaction` 97 | plan #309, #33 |
| 2 | The same refusal, which dEQP reports NotSupported because GLSL ES 1.00 Appendix A makes the feature optional (`Loop type is not supported`, `Shader is not supported`, `Dynamic loops not supported`, `Dynamic indexing not supported`, `Full GLSL ES 100 is not supported`) | NotSupported | 937 | `shaders.loops` 527, `shaders.indexing` 367, `shaders.struct` 24 | plan #309, #33 |
| 3 | A blend shader refused at the draw (`mali frame: a blend shader other than a constant colour's ... is not built yet`, `NOT_BUILT_BLEND_SHADER`); dEQP: `Image compare failed` (the draw is missing) | Fail | 828 | `fragment_ops.blend` (232 pass) | plan #309 |
| 4 | A GL error, a query value or missing pixels in a chunk whose exit line counts refusals: `GL_INVALID_OPERATION` at a draw (a refused draw is `-ENOTSUP`, `src/gl/draw.rs` `submit_draw`, or the program did not link), `GL error`, `Got invalid error`, `got invalid value`, `expected GL_TRUE`, `Image comparison failed` | Fail | 164 | `fragment_ops.random` 100, `light_amount` 18, `negative_api` 14, `default_vertex_attrib` 8 (`mat2`, `mat3`), `shaders.random` 8, `state_query` 7, `shaders.operator` 6, `fragment_ops.interaction` 2, `shaders.texture_functions` 1 | plan #309 by the chunk's refusals; each case unproven |
| 5 | A depth or stencil object refused at the draw (`mali: a depth or stencil object drawn in another framebuffer's depth/stencil pair is not built yet`); dEQP: `Got unexpected error` 21, `Image comparison failed` 14 | Fail | 35 | `fbo.render.{recreate_depthbuffer,recreate_stencilbuffer,shared_depthbuffer}.*_depth_component16_stencil_index8` | plan #309 |
| 6 | A draw failed with `GL_OUT_OF_MEMORY`: `backend::draw` returned an error other than `-ENOTSUP` (`-ENOMEM`, `-EINVAL` or `-EIO`, `src/mali/backend.rs`); not a refusal | ResourceError | 33 | `shaders.matrix` 24 (`*_vertex`), `shaders.random` 6, `shaders.conditionals` 2, `fragment_ops.interaction` 1 | new |
| 7 | Wrong pixels from a vertex-shader cube look-up, with no refusal in the chunk (row `T6S1R17`: `draws=1260 refused=0`); dEQP: `Image comparison failed` | Fail | 16 | `texture.vertex.cube.filtering.*` 10 (mip filters), `texture.vertex.cube.wrap.{clamp,mirror}_*` 6 | new |
| 8 | Wrong pixels that no refusal of the chunk explains: `fbo.render.stencil_clear.rbo_rgb565_{stencil_index8,depth_component16_stencil_index8}` 2 (`Image comparison failed`), `texture.mipmap.cube.bias.linear_{linear,nearest}` 2 (`Image verification failed`) | Fail | 4 | `fbo.render`, `texture.mipmap` | new |
| 9 | A feature the context does not offer: `EXT_clip_control` 8, no multisample buffers 16, `EXT_multisampled_render_to_texture` 1, no compressed format with sub-image updates 9: the 34 of section C | NotSupported | 34 | `clip_control`, `multisample`, `negative_api.texture` | accepted (section C) |
| 10 | `rasterization.interpolation.basic.lines`, `.projected.lines`, `.projected.lines_wide`: the Pi 4's three of section C | QualityWarning | 3 | `rasterization` | accepted (section C) |
| | **Total** | | **11,905** | Fail 10,898 (1, 3, 4, 5, 7, 8), NotSupported 971 (2, 9), ResourceError 33, QualityWarning 3 | |

### M.2 The refusals by the chunk's first refusal

Causes 1 and 2 (10,788 cases) by the `MaliError` that the exit line of
their chunk names first (`src/shaderc/online/mali/mod.rs`). Every one is a
variant of the compiler, so they are plan #309's. A chunk holds one to 400
cases, and its later refusals may be other variants, so the counts say where
each variant stops a chunk first, not how many shaders it refuses.

| `MaliError` | Fail | NotSupported | Cases |
|---|---:|---:|---:|
| `IntValue` (an integer or boolean value) | 4,860 | 321 | 5,181 |
| `Arithmetic` | 1,646 | 24 | 1,670 |
| `ColourForm` (a gl_FragColor lane other than the varying's own or a constant) | 616 | 120 | 736 |
| `ConstantFold` (a constant operation on a value that is not a finite normal float) | 708 | 0 | 708 |
| `VaryingDuplicate` (two varying lanes of equal value) | 594 | 8 | 602 |
| `VaryingNotFromAttribute` (a varying lane computed from no attribute) | 304 | 0 | 304 |
| `AlgebraicForm` | 239 | 0 | 239 |
| `PositionLane` (a gl_Position lane left unwritten) | 0 | 208 | 208 |
| `NegZeroShared` | 196 | 0 | 196 |
| `TexKind` | 193 | 0 | 193 |
| `UniformPrecision` (a vertex shader uniform that is not highp) | 175 | 0 | 175 |
| `TooManyLocals` (more than 32 local variables) | 37 | 112 | 149 |
| `LoopMediump` | 37 | 87 | 124 |
| `ArithConstant` | 92 | 0 | 92 |
| `LoopVertex` | 16 | 48 | 64 |
| `LoopForm` | 50 | 9 | 59 |
| `Invariant` | 36 | 0 | 36 |
| `MixedPrecision` (#4496) | 28 | 0 | 28 |
| `ColourZeroHalf` (a colour pair of a varying lane and 0.0) | 24 | 0 | 24 |
| **Total** | **9,851** | **937** | **10,788** |

The 208 fragment loops with a run-time bound that
`docs/notes/mali-gles2-forecast.md` section 3 counts as hang-risks are row
`T6S3R08`. All 208 are NotSupported, refused at link (`PositionLane` first,
`refused=210`, `draws=0`). None reached the GPU, so the hang risk is still
untested.

### M.3 Still open

- Causes 6, 7 and 8 are new. Cause 4 needs the refusal of each case before
  its cases can be held by plan #309 or opened as defects of the GL core.
- The library names only the first refusal of a chunk. A per-case refusal
  line would let the tally split causes 1, 2 and 4 by variant.
- 0 Fail is the stage 5 criterion. The tree is rerun once plan #309's
  refusals move, from one origin/main commit, and replaces this section.
