<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 2.0 CTS failure inventory

dEQP-GLES2, the mustpass list `tools/cts/gles2-mustpass.txt` (17,165 cases),
on the AM62P (PowerVR BXS-4-64), the i.MX8MP (Vivante GC7000UL) and the
Raspberry Pi 4 (V3D 4.2). This file says what is not Pass now and why that is
accepted. The numbers and the commit are in `tools/cts/RESULTS.md`. Earlier
rounds, their failures and their fixes are in git history.

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
| `multisample.{common_edge_big_quad, common_edge_small_quads, common_edge_viewport_quad, constancy_alpha_to_coverage, constancy_both, constancy_both_inverted, constancy_sample_coverage, constancy_sample_coverage_inverted, depth, num_samples_line, num_samples_polygon, proportionality_alpha_to_coverage, proportionality_sample_coverage, proportionality_sample_coverage_inverted, sample_coverage_invert, stencil}` | 16 | 0 (16 pass on the ms4 image; NotSupported at 1x) | 16 | 16 | acceptable: the default config has no multisample surface ("No multisample buffers"). The AM62P's 4x image runs them. The i.MX8MP can once its 4x MSAA works (#8); the Pi 4 has no 4x config | test harness (image config); hardware on the i.MX8MP (#8) |
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
