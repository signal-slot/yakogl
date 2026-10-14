<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# tf-sampler: transform-feedback capture evaluates texture look-ups, 2026-10-14, branch of plan #144 item tf-sampler (from cd738b1)

A capturing draw whose vertex shader samples a texture is now evaluated
instead of refused. The CPU evaluator (`src/shaderc/online/eval.rs`) has a
texel hook, the `TexelSource` trait and `run_in_with_textures`; the capture
(`src/gl/tf_capture.rs`, `GlTexels`) answers it from the GL texture storage,
read through `texture.rs`' layout without changing that file. Host only; no
board run.

## State of main

- ES 3.0 table, transform feedback row: the capture now also evaluates
  `texture`, `textureLod`, `textureOffset`, `textureProj`, `texelFetch`,
  `texelFetchOffset` and `textureSize` on 2D, cube, 3D and 2D array samplers
  (float, `isampler*`, `usampler*`), with NEAREST/LINEAR and the four mipmap
  filters, REPEAT / CLAMP_TO_EDGE / MIRRORED_REPEAT, seamless cube edges,
  MIN_LOD / MAX_LOD / MAX_LEVEL, sampler objects and the texture swizzle.
  Every storage class of the original tables and of `es3_formats.rs` is
  decoded (RGBA8, sRGB, L8/A8/LA8, RGB565, RGBA16F, R16F, R32F, RGBA32F,
  R11G11B10F, RGB9E5, RGB10A2, RGBA8_SNORM and the integer classes), and a
  depth texture reads as red. Tests: `tests/es3_tf_sampler.rs` (15),
  `src/gl/tf_capture_tests.rs` (6 new, and `a_vertex_shader_that_samples_is_refused`
  replaced by one that captures), `eval.rs` (2 new). Green on gpu-powervr,
  gpu-vivante and gpu-v3d, with and without `report-es2`, together with
  `es3_tf_api` and `es2_report_guard`; the full suites of
  `host-test,gpu-powervr,shaderc-online`, `host-test,gpu-vivante,shaderc-online`,
  `host-test,gpu-powervr,shaderc-online,report-es2` and the V3D library
  tests pass.

## Open work

- Item 4, **tf-capture**: replace "Left: a vertex shader that samples a
  texture refuses its capturing draw;" with "Left:". Still refused, each by
  name in the log: a shadow look-up, `textureGrad`, a compressed texture
  (its format named), a texture with GL_TEXTURE_BASE_LEVEL other than 0 (the
  draw does not apply it either), a sampler whose component type does not
  match the texture's, storage with no CPU view.
- Stale text this item could not edit (outside its files), for the fold or
  the next item that owns them:
  - `src/GL_CORE_NOTES.md` lines 394 and 634: the capture no longer refuses
    a vertex shader that samples; the deviations are now the refusals listed
    above.
  - `docs/es3-transform-feedback.md` section 4.1 "Vertex textures" (line
    228): the capture reads the GL storage through `TexelSource`; section
    5.7's refusal is narrowed to the list above.
- What each GPU's code generator lets reach a capture (measured with
  `tests/es3_tf_sampler.rs -- --nocapture`, which prints every shader it
  skips): the capture is the same on the three GPUs, but a program the code
  generator or the link refuses never draws.
  - PowerVR refuses `texelFetchOffset`, `textureGrad` and an integer result
    from a 2D array (and cube, 3D).
  - GC7000 refuses `textureOffset`, `texelFetchOffset`, `textureProj`,
    `textureGrad`, shadow samplers, integer results, and `textureSize` at a
    level other than a literal 0.
  - V3D refuses 3D and 2D array samplers, `texelFetch`, offsets,
    `textureGrad`, shadow samplers and integer results.

  `texelFetchOffset` and an integer 2D array reach no capture on any GPU
  yet; `the_texel_source_reads_what_no_code_generator_selects_yet` in
  `tf_capture_tests.rs` asks the texel source for them directly. These are
  code-generator items, not capture items.

## Known defects

- Opened (no issue number yet): on the PowerVR, a `sampler2DArray` that a
  vertex shader reads only through `textureSize` has no uniform location
  (`glGetUniformLocation` answers -1), so the application cannot set its
  unit. `src/shaderc/online/powervr/chain.rs` keeps only the sampler rows a
  `Tex` op samples (`row_sampled`). The test works around it with an extra
  `texture()` read (`texture_size_of_each_target`).
- Known differences between the capture and the draw, by design: the capture
  applies the texture swizzle and GL_TEXTURE_MIN_LOD/MAX_LOD, which the draw
  does not yet (`GlTexture` comment "the draw does not apply them yet"); it
  filters cube maps seamlessly (ES 3.0 3.8.9.1); it computes a 3D texture's
  chain length from max(w, h, d) where `draw.rs` `mip_levels_needed` uses
  w and h. A sampler object's WRAP_R and LOD range are not exposed by
  `es3.rs` (`sampler_state` returns min, mag, wrap S, wrap T), so the
  capture uses the texture's, as the draw does.
