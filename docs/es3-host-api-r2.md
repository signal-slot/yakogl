<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# dEQP-GLES3 API-only groups on the host, second run (2026-10-14)

**Measured commit: `cd738b1ae7cf99166a6bdd86cf24dac04c4c4328`** (integration tip,
"ccfarm: merge ccfarm/01a13623-0a20-7578-bba9-a6ab4290251a (#533)"; `src/` is
not changed by this document). The first run is `docs/es3-host-run.md` (main
`9390731`, 2026-10-12). This one repeats the API-only groups on the PowerVR
(`gpu-powervr`), the GC7000 (`gpu-vivante`) and the V3D (`gpu-v3d`) and says,
for every case that still fails, why and who owns it.

**HOST-ONLY.** The library is built with the LINUX OSAL: it records the command
stream and does not run it, so every `glReadPixels` reads black. A verdict
decided by a picture is meaningless and is never quoted here as a pass or a
failure of the GPU. What is real: what the GL API returns (errors, queries,
limits), each shader's compile and link outcome with our info log, and the
library's stderr. Failures that are picture verdicts are counted apart in the
tables ("picture") and named by group in the failure sections.

## What ran

- Library and harness: `cd738b1`, `MODULE=gles3` (`YAKOGL_REPORT_ES2=OFF`, the
  online compiler, `YAKOGL_OSAL=linux`), once per GPU. The script
  `tools/cts/host-deqp-run.sh` supports all three GPUs, V3D included (it sets
  `YAKOGL_V3D_TILED_COPIES_BYTES=16777216` for `GPU=v3d`). It was not edited and
  needed no wrapper.
- dEQP: VK-GL-CTS `93bca018` (`vulkan-cts-1.4.6.2-555-g93bca01861b0`) with the
  yakogl target, the checkout in `$CCFARM_DEPS_DIR/VK_GL_CTS`. `deqp-gles3` was
  built by the script. `docs/es3-host-env.md`, named in the plan, does not exist
  on the tip; the environment is `tools/cts/README.md` section 2b and
  `docs/es3-host-run.md`.
- The groups are read out of the full gles3 mustpass run (42,810 cases, 429 chunks
  of 100, 20 chunks in parallel, a 256x256 pbuffer) rather than a run per group:
  the script takes one substring filter, and the whole list takes a few minutes
  per GPU once the libraries are built, so one run per GPU gave every group at
  one commit. The same three runs also cover every other group; only the
  groups of this item are reported here.

```sh
CTS=$CCFARM_DEPS_DIR/VK_GL_CTS
for g in powervr vivante v3d; do
    MODULE=gles3 tools/cts/host-deqp-run.sh --cts="$CTS" --gpu=$g --jobs=20 \
        --out=build/host-deqp/r2-$g
done
```

Single cases were run again to read the library's stderr, for example
`tools/cts/host-deqp-run.sh --cts="$CTS" --gpu=v3d --module=gles3 --jobs=1
--out=build/host-deqp/r2-one dEQP-GLES3.functional.state_query.shader.program_active_uniform_types`.
The per-case tables were read out of each run's `c_*.qpa` logs (status, message
and whether a picture comparison failed in the case's log).

### Begun against asked

| GPU | asked | began | never began | processes | exit 0 / exit 1 | crash (134 / 137 / timeout) |
|---|---:|---:|---:|---:|---|---|
| powervr | 42,810 | 42,810 | 0 | 477 | 62 / 415 | none |
| vivante | 42,810 | 42,810 | 0 | 1,569 | 54 / 1,515 | none |
| v3d | 42,810 | 42,810 | 0 | 1,890 | 55 / 1,835 | none |

The process counts are `docs/es3-host-run.md`'s (477, 1,569, 1,893; the V3D has
3 fewer). Every case began, no process crashed and none timed out. **Crash is 0
in every table below.** Processes stop early only after a `ResourceError` or a
case-deinit error. Of the groups of this item, the V3D's `transform_feedback`
is the only one in that list (below); the PowerVR's stops are all in
`shaders.builtin_functions.precision` and the GC7000's and V3D's in
`fragment_ops.blend.fbo_srgb`, all outside the groups here.

## The groups and what is in them

Case names are `dEQP-GLES3.functional.<group>` except `info`
(`dEQP-GLES3.info`).

| item group | cases taken |
|---|---|
| negative_api, state_query, implementation_limits, buffer, occlusion_query, attribute_location | the whole group |
| info | `dEQP-GLES3.info.*` (6) |
| fbo.api, fbo.completeness | those two subgroups (20, 560) |
| sync | `fence_sync` (10), the only sync group of the list |
| transform_feedback | the whole group (882). dEQP's capture is read back through GL, and the capture runs on the CPU on every GPU (`docs/es3-transform-feedback.md` 4.1), so these verdicts are real on the host. There is no API-only subset to separate, so the whole group is the "API cases" |
| shaders compile/link | the subgroups whose verdict is the compile and link outcome: `linkage`, `negative`, `invalid_implicit_conversions`, `keywords.invalid_identifiers`, `keywords.keywords`, `keywords.reserved_keywords`, `preprocessor`, `qualification_order`, `large_constant_arrays`, `declarations`, `uniform_block.valid` and `.invalid` (1,703 cases). `keywords.allowed_keywords` (8) is not taken: it is a rendering test |

Some cases of the last row render after they compile (linkage, preprocessor,
qualification_order, large_constant_arrays). Their compile and link outcome is
real, because the case reached the picture comparison; a failure that ends in
"Image comparison failed" is the black read-back and is counted as a picture
verdict.

## Results per group and GPU

pass / fail / not supported / other. "picture" is the part of the fail count
that ended in a picture comparison (meaningless on the host); the rest of the
failures is real. "earlier" is `docs/es3-host-run.md` (p / f / ns / o); delta is
this run minus earlier. The first run reports only whole groups for `fbo` and
`shaders`, so `fbo.api`, `fbo.completeness` and `shaders compile/link` have no
earlier numbers by subgroup (not recorded); their whole group is shown for
context.

**negative_api** (312 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 301 | 7 | 0 | 4 | 0 | 299 / 9 / 4 / 0 | +2 / -2 / 0 / 0 |
| vivante | 300 | 8 | 0 | 4 | 0 | 300 / 8 / 4 / 0 | 0 |
| v3d | 300 | 8 | 0 | 4 | 0 | 300 / 8 / 4 / 0 | 0 |

**state_query** (946 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 912 | 25 | 0 | 9 | 0 | 912 / 25 / 9 / 0 | 0 |
| vivante | 916 | 21 | 0 | 9 | 0 | 916 / 21 / 9 / 0 | 0 |
| v3d | 909 | 28 | 0 | 9 | 0 | 909 / 28 / 9 / 0 | 0 |

**implementation_limits** (49 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 38 | 11 | 0 | 0 | 0 | 38 / 11 / 0 / 0 | 0 |
| vivante | 41 | 8 | 0 | 0 | 0 | 41 / 8 / 0 / 0 | 0 |
| v3d | 38 | 11 | 0 | 0 | 0 | 38 / 11 / 0 / 0 | 0 |

**info** (6 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 6 | 0 | 0 | 0 | 0 | 6 / 0 / 0 / 0 | 0 |
| vivante | 6 | 0 | 0 | 0 | 0 | 6 / 0 / 0 / 0 | 0 |
| v3d | 6 | 0 | 0 | 0 | 0 | 6 / 0 / 0 / 0 | 0 |

**buffer** (695 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 168 | 527 | 527 | 0 | 0 | 168 / 527 / 0 / 0 | 0 |
| vivante | 168 | 527 | 527 | 0 | 0 | 168 / 527 / 0 / 0 | 0 |
| v3d | 168 | 527 | 527 | 0 | 0 | 168 / 527 / 0 / 0 | 0 |

**fbo.api** (20 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 20 | 0 | 0 | 0 | 0 | not recorded | |
| vivante | 20 | 0 | 0 | 0 | 0 | not recorded | |
| v3d | 20 | 0 | 0 | 0 | 0 | not recorded | |

**fbo.completeness** (560 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 497 | 56 | 0 | 7 | 0 | not recorded | |
| vivante | 442 | 111 | 0 | 7 | 0 | not recorded | |
| v3d | 419 | 134 | 0 | 7 | 0 | not recorded | |

The whole `fbo` group (2,028 cases) is 517 / 1,256 / 255 / 0 (powervr), 462 /
1,311 / 255 / 0 (vivante) and 439 / 1,328 / 255 / 6 (v3d), identical to the first
run on every GPU (delta 0). 927 / 642 / 511 of its failures are picture verdicts.

**fence_sync** (the sync group, 10 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 10 | 0 | 0 | 0 | 0 | 10 / 0 / 0 / 0 | 0 |
| vivante | 10 | 0 | 0 | 0 | 0 | 10 / 0 / 0 / 0 | 0 |
| v3d | 10 | 0 | 0 | 0 | 0 | 10 / 0 / 0 / 0 | 0 |

**transform_feedback** (882 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 882 | 0 | 0 | 0 | 0 | 882 / 0 / 0 / 0 | 0 |
| vivante | 880 | 2 | 0 | 0 | 0 | 880 / 2 / 0 / 0 | 0 |
| v3d | 740 | 1 | 0 | 12 | 129 ResourceError | 740 / 1 / 12 / 129 | 0 |

**occlusion_query** (62 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 62 | 0 | 0 | 0 | 0 | 62 / 0 / 0 / 0 | 0 |
| vivante | 62 | 0 | 0 | 0 | 0 | 62 / 0 / 0 / 0 | 0 |
| v3d | 62 | 0 | 0 | 0 | 0 | 62 / 0 / 0 / 0 | 0 |

The occlusion_query pass is an API pass (query objects, state, result
availability). The host cannot count samples, so it says nothing about the
visibility counters (`docs/es3-occlusion-query.md`).

**attribute_location** (274 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 226 | 48 | 0 | 0 | 0 | 226 / 48 / 0 / 0 | 0 |
| vivante | 226 | 48 | 0 | 0 | 0 | 226 / 48 / 0 / 0 | 0 |
| v3d | 226 | 48 | 0 | 0 | 0 | 226 / 48 / 0 / 0 | 0 |

**shaders compile/link** (1,703 cases)

| GPU | pass | fail | picture | not supported | other | earlier | delta |
|---|---:|---:|---:|---:|---|---|---|
| powervr | 1,309 | 394 | 392 | 0 | 0 | not recorded | |
| vivante | 1,309 | 394 | 390 | 0 | 0 | not recorded | |
| v3d | 1,309 | 394 | 392 | 0 | 0 | not recorded | |

The whole `shaders` group (20,616 cases) is identical to the first run on every
GPU: 2,488 / 17,165 / 0 / 963 (powervr), 2,241 / 18,375 / 0 / 0 (vivante), 2,248
/ 18,325 / 0 / 43 (v3d).

### What changed since the first run

- **PowerVR negative_api: two failures fewer** (9 to 7). `docs/es3-host-run.md`
  has no per-case list, so which two cases passed now is not known. Ten commits
  between `9390731` and `cd738b1` touch `src/gl` or `src/pvr` (tex3d-finish and
  the report-es2 error rules among them); none is tied to a case here.
- Everything else in the groups of this item is **identical** to the first run on
  every GPU, in count. The state of these groups did not move in two days; no
  item of the plan so far has changed an API-only verdict.
- Outside these groups, the PowerVR whole-run totals are 8,109 pass (+2) and the
  V3D's 327 ResourceError (-3) and 32,677 fail (+3) against 330 and 32,674. These
  V3D numbers are in groups this item does not cover and were not examined.

## Every remaining failure, with its cause and owner

Owners are named as follows. **RCn** is a root-cause row of
`docs/es3-cts-failures.md`. **Appendix A** is that file's vertex appendix. An
item named there but not found on the tip (`mrt4`, `gl-core-misc`,
`formats-gc`, `formats-v3d`) is written as "item x, not on the tip". Plan #18
and #19 are not described anywhere in the repository, so an owner there could not
be read; a failure with no row in `docs/es3-cts-failures.md` and no plan #144
item is **unowned**. `P / G / V` = PowerVR / GC7000 / V3D.

### negative_api (P 7, G 8, V 8, plus 4 not supported)

| case | GPUs | cause | owner |
|---|---|---|---|
| buffer.blit_framebuffer | P | Integer colour textures (RGBA32UI, RGBA32I) are not framebuffer-complete (`attachment_storage` refuses an `es3_fmt` texture whose class is not RGBA8, `src/gl/framebuffer.rs:435`), so `glBlitFramebuffer` returns GL_INVALID_FRAMEBUFFER_OPERATION where dEQP expects GL_INVALID_OPERATION for a read/draw format mismatch. G and V pass only because their `glTexImage2D` refuses these formats and the pending error satisfies the check | RC1, integer textures as render targets: no item |
| buffer.blit_framebuffer_multisample | P G V | `glRenderbufferStorageMultisample(4 samples)` raises GL_INVALID_OPERATION, because `GL_MAX_SAMPLES` is 1 (`src/gl/framebuffer.rs:3077`); the pending error shows at the next `glGetError` | RC8, multisample: no item |
| buffer.read_pixels_fbo_format_mismatch | P G V | the same multisample refusal (the case needs a 4-sample renderbuffer) | RC8: no item |
| buffer.clear_bufferiv, buffer.clear_bufferuiv | G V | `glTexImage2D(GL_RGBA32I / RGBA32UI)` is refused by the back end ("samples none of its storage classes"), so the error is pending when dEQP expects GL_NO_ERROR | RC2, item formats-gc / formats-v3d, not on the tip |
| shader.uniformui_incompatible_type, shader.uniformuiv_incompatible_type | P G V | `glUniform4ui` on an `ivec4` is accepted: the unsigned setter loads an int uniform on purpose, for Qt RHI (`src/gl/program.rs:2070`, `setter_accepts`) | a documented deviation; ES 3.0 says GL_INVALID_OPERATION. Unowned |
| texture.texsubimage2d, texture.texsubimage3d | P G V | `glTexSubImage2D/3D(GL_RGB, GL_FLOAT)` into an RGBA8 texture is accepted; the format/type/internal-format combination should be GL_INVALID_OPERATION | unowned (`src/gl/texture.rs`) |
| fragment.blend_equationi, blend_equation_separatei, blend_funci, blend_func_separatei | P G V (not supported) | `GL_EXT_draw_buffers_indexed` is not advertised; not an ES 3.0 core feature | excused; draw_buffers_indexed stays NotSupported (76 cases in `docs/es3-host-run.md`) |

### state_query (P 25, G 21, V 28, plus 9 not supported)

| cases | GPUs | cause | owner |
|---|---|---|---|
| integers.compressed_texture_formats_{getinteger, getinteger64, getfloat} | P G V | `GL_NUM_COMPRESSED_TEXTURE_FORMATS` is 3 (`src/gl/state.rs:1172`); ES 3.0 lists the ten ETC2/EAC formats | RC10, item gl-core-misc, not on the tip |
| integers.max_color_attachments_{getinteger64, getfloat}, max_draw_buffers_{getinteger64, getfloat} (4) | P G V | the limit is 3 (`CAP_COLOR_ATTACHMENTS`, `src/gl/context.rs:139`), ES 3.0 asks for 4 | RC14, item mrt4, not on the tip |
| integers.max_samples_{getinteger64, getfloat} (2) | P G V | `GL_MAX_SAMPLES` is 1 (`src/gl/state.rs:1251`), ES 3.0 asks for 4 | RC8: no item |
| integers.max_fragment_input_components, max_varying_components, max_varying_vectors, each {getinteger64, getfloat} (6) | P V | 32 / 32 / 8 reported, 60 / 60 / 15 needed (`src/gl/state.rs:1200-1220`, the back end's varying budget). The GC7000 reports 60 / 60 / 15 and passes | unowned (back-end varying budget) |
| integers.max_vertex_output_components_{getinteger64, getfloat} (2) | P G V | 32 on P and V, 60 on G; 64 needed | unowned |
| integers64.max_combined_fragment_uniform_components, max_combined_vertex_uniform_components, each {getinteger, getfloat} (4); max_uniform_block_size_{getinteger, getfloat} (2) | P G V | 1,536 reported against 19,968 needed; the block size is 6,144 against 16,384 (`src/gl/state.rs:1230-1232`, `MAX_UNIFORM_DWORDS`) | unowned (back-end uniform budget) |
| rbo.renderbuffer_component_size_color | P G V | `GL_RENDERBUFFER_{RED..ALPHA}_SIZE` is a flat 8 for every non-integer, non-float colour format (`src/gl/framebuffer.rs:2421`); dEQP wants the format's own sizes (R8 has no green, RGB565 is 5/6/5, RGBA4 is 4, RGB10_A2 is 10/2). From the code and the case table; not stepped to the failing line | unowned (RC10 names renderbuffer queries; item gl-core-misc, not on the tip) |
| shader.program_binary | P G V | `GL_PROGRAM_BINARY_RETRIEVABLE_HINT` is always read back as 0 after `glProgramParameteri(..., 1)` (`src/gl/program.rs:2728`) | RC10 ("other (shader.program_binary) 1"), item gl-core-misc, not on the tip |
| fbo.framebuffer_attachment_component_type, fbo.framebuffer_attachment_x_size_texture | G V | `glTexStorage2D(GL_RGBA8I / GL_RG32UI)` is refused by the back end, so the attachment sizes read 0 | RC2, formats-gc / formats-v3d, not on the tip |
| shader.program_active_uniform_types | V | the program does not link: `NotYet("textureSize of a 3D or array sampler")` (re-run alone on the V3D) | V3D shader compiler: unowned |
| indexed.* (9) | P G V (not supported) | `GL_EXT_draw_buffers_indexed` | excused, as above |

### implementation_limits (P 11, G 8, V 11)

Each case reports the limit and the minimum.

| case | P | G | V | minimum | owner |
|---|---|---|---|---|---|
| max_color_attachments, max_draw_buffers | 3 | 3 | 3 | 4 | RC14, item mrt4, not on the tip |
| max_fragment_input_components, max_varying_components | 32 | pass | 32 | 60 | unowned |
| max_varying_vectors | 8 | pass | 8 | 15 | unowned |
| max_vertex_output_components | 32 | 60 | 32 | 64 | unowned |
| max_combined_fragment_uniform_components | 1,536 | 1,536 | 1,536 | 50,048 | unowned |
| max_combined_vertex_uniform_components | 1,536 | 1,536 | 1,536 | 50,176 | unowned |
| max_uniform_block_size | 6,144 | 6,144 | 6,144 | 16,384 | unowned |
| num_compressed_texture_formats | 3 | 3 | 3 | 10 | RC10, gl-core-misc, not on the tip |
| compressed_texture_formats | missing GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, SRGB8_ALPHA8_ETC2_EAC (and others) | same | same | all ten | RC10, gl-core-misc, not on the tip |

### buffer (527 failures per GPU, all picture verdicts)

Every failing case ends in "Image comparison failed" or "Buffer verification
failed": dEQP checks the buffer contents by drawing from it and reading the
picture back, and the host reads black. Not measurable. The failing cases are
`buffer.copy.basic` 56 of 56, `copy.single_buffer` 56 of 56, `copy.subrange` 7 of
7, `map.read_write` 88 of 120, `map.write` 144 of 176, `write.basic` 72 of 72,
`write.basic_subdata` 72 of 72, `write.partial_specify` 13 of 13,
`write.random` 10 of 10 and `write.recreate_store` 9 of 9. `map.read` passes
104 of 104 (the mapping API is exercised and is correct). Owner: none; needs a
board (the AM62P passed all 695, `docs/es3-host-run.md`).

### fbo.completeness (P 56, G 111, V 134, plus 7 not supported)

| cases | GPUs | cause | owner |
|---|---|---|---|
| attachment_combinations.none_{rbo,tex}_{none,rbo,tex}_{none,rbo,tex} where the case is complete (14: none_rbo_none_none, none_rbo_none_rbo, none_rbo_none_tex, none_rbo_rbo_none, none_rbo_rbo_rbo, none_rbo_tex_none, none_rbo_tex_tex, and the same seven with tex) | P G V | a framebuffer with a colour attachment only on `COLOR_ATTACHMENT1` is GL_FRAMEBUFFER_UNSUPPORTED (`src/gl/framebuffer.rs:721-728`, "colour only on attachments 1.. stays unsupported"); ES 3.0 4.4.4 makes it complete. dEQP log: "Received GL_FRAMEBUFFER_UNSUPPORTED" | unowned (`es3-cts-failures.md` Appendix B: "no bucket names it") |
| attachment_combinations.{none,rbo,tex}_{none,rbo,tex}_{rbo,tex}_{tex,rbo} with different depth and stencil images (14: none_none_rbo_tex, none_none_tex_rbo, rbo_none_rbo_tex, rbo_none_tex_rbo, rbo_rbo_rbo_tex, rbo_rbo_tex_rbo, rbo_tex_rbo_tex, rbo_tex_tex_rbo, tex_none_rbo_tex, tex_none_tex_rbo, tex_rbo_rbo_tex, tex_rbo_tex_rbo, tex_tex_rbo_tex, tex_tex_tex_rbo) | P G V | `check_status` never compares the depth and the stencil attachment: "Depth and stencil attachments are not the same image" should give GL_FRAMEBUFFER_UNSUPPORTED, the status is COMPLETE | unowned |
| size.distinct | P G V | depth and colour of different sizes give GL_FRAMEBUFFER_INCOMPLETE_DIMENSIONS (`src/gl/framebuffer.rs:690-707`); ES 3.0 4.4.4.2 says an ES 3.0 implementation cannot generate that token (the colour loop already skips it for ES 3.0 only) | unowned |
| layer.3d_64_15 | P | a 3D texture attachment is incomplete on the PowerVR: its slices are tiled in a grid (`grid_cols > 1`, `src/gl/framebuffer.rs:436`; pinned by `a_3d_texture_attachment_is_incomplete`, line 7217). G and V complete it | unowned (RC6 designs 2D-array sampling, not 3D render targets) |
| renderable.renderbuffer.color0.r32f; renderable.texture.color0.{r32f, rgba32f} | P G V | R32F and RGBA32F count as colour-renderable (`is_color_format`, `src/gl/framebuffer.rs:530`; the texture class passes `attachment_storage`). `GL_EXT_color_buffer_float` is not advertised, so dEQP expects incomplete | RC1's float-target note (the 103 derivate cases), "no item yet" |
| renderable.texture.depth.{depth_component_unsigned_int, depth_component_unsigned_short, depth_stencil_unsigned_int_24_8}, renderable.texture.stencil.depth_stencil_unsigned_int_24_8 | P G V | the unsized `DEPTH_COMPONENT` / `DEPTH_STENCIL` texture forms (the ES 2.0 `GL_OES_depth_texture` ones) are accepted in the ES 3.0 build; ES 3.0 allows only the sized forms and dEQP expects an error or incomplete | unowned (`src/gl/texture.rs`, gate on `!REPORT_ES2`) |
| renderable.texture.color0.{r8i, r8ui, r16i, r16ui, r32i, r32ui, rg8i, rg8ui, rg16i, rg16ui, rg32i, rg32ui, rgb10_a2ui, rgba8i, rgba8ui, rgba16i, rgba16ui, rgba32i, rgba32ui} (19) | P | integer textures are not render targets (`src/gl/framebuffer.rs:435`: a texture of an `es3_fmt` class other than RGBA8 is refused). Integer renderbuffers are complete | RC1: integer renderbuffers done (`int-rt-gl`, `int-rt-pvr`); integer textures as targets: no item (`es3-cts-failures.md:684-685`) |
| renderable.texture.{color0, depth, stencil}.{the 25 integer names (r8i .. rgba32ui, rgb10_a2ui, rgb8i, rgb16i, rgb32i ...)} (25 per row, 75) | G | `glTexImage2D` refuses the integer formats at the back end ("samples none of its storage classes"), so dEQP sees GL_INVALID_OPERATION during FBO set-up | RC2, formats-gc, not on the tip |
| the same 75, plus 7 more in the colour row (r11f_g11f_b10f, r8_snorm, rg8_snorm, rgb8_snorm, rgba8_snorm, rgb9_e5, rg32f) and 8 in each of the depth and stencil rows (those seven and rgb16f) = 98 | V | the same, and the V3D also refuses the snorm, packed-float, RG32F and (in the depth and stencil rows) RGB16F formats at upload | RC2, formats-v3d, not on the tip |
| samples.{rbo1_rbo2_rbo0, rbo1_rbo2_rbo4, rbo2_none_none, rbo2_rbo1_none, rbo2_rbo2_none, rbo2_rbo2_rbo0, rbo2_tex_none} | P G V (not supported) | dEQP reports "Unsupported number of samples": a 2- or 4-sample renderbuffer is refused (`GL_MAX_SAMPLES` is 1) | RC8: no item |

### transform_feedback (P 0, G 2, V 1 fail + 12 not supported + 129 ResourceError)

| cases | GPUs | cause | owner |
|---|---|---|---|
| random_full_array_capture.interleaved.points.3, random_full_array_capture.separate.triangles.8 | G | "Compile failed: ''": the vertex shader is refused with `Abi(LoadBalancingTempsUnmeasured { num_out: 15, temps: 17, temps_max: 16 })` (read in separate.triangles.8's own log) | GC7000 code generator (`src/shaderc/online/vivante`): unowned. `es3-cts-failures.md` RC4 read this as "empty info log" |
| 13 random_full_array_capture cases: 12 not supported (interleaved.lines.2, .lines.4, .points.1, .points.3, .points.5, .triangles.1 and six more) and the Link failed one (interleaved.triangles.8) | V | the V3D refuses the program, found by re-running `random_full_array_capture.interleaved.` and `.separate.` alone: 8 `NotYet("a flat varying past input 31")` and 5 `NotYet("a centroid varying past input 31")`. dEQP reports 12 of them as "Capturing arrays is not supported" (NotSupported) and one as Link failed | V3D shader compiler: unowned |
| 129 ResourceError: `array.*` 99, `basic_types.*` 18, `random_full_array_capture.*` 12 (all three primitives, interleaved and separate), e.g. array.interleaved.lines.highp_ivec4 | V | `yakogl/v3d: draw refused: the job builder refused the draw (a uniform, program or primitive it does not write)`, then `glDrawArrays was not recorded (-22)` and GL_OUT_OF_MEMORY. By type, of the 12 cases of each type in `array` and `basic_types`: vec4, ivec4 and uvec4 fail 3, mat3, mat3x4 and mat4x3 fail 6, mat4 fails 12. A ResourceError ends the dEQP process, so a board image would stop at each one (9 V3D chunks stop in this group: c_361..c_368 and c_423) | V3D back end: unowned |

### attribute_location (48 per GPU)

`mixed.*` 24 of 24, `mixed_max_attributes.*` 20 of 24 (mat3, mat3x2, mat3x3,
mat3x4 pass) and `mixed_time.{pre_attach, pre_link, reattach, relink}` 4: no
picture comparison; dEQP's own check of the attribute locations fails. The same
48 as the AM62P, and the same two causes as `es3-cts-failures.md` Appendix A:
(1) `glBindAttribLocation` is applied to an attribute that has
`layout(location = N)` (`build_attrib_indices`, `src/gl/program.rs:1545-1548`; the
qualifier should win), 28 cases; (2) the compiler places an unqualified `in` at
the counter after the last explicit location (`src/shaderc/online/reflect.rs`),
so it collides with a `layout(location)` input, 20 cases. Owner: Appendix A task
T4, no item. Not fixed since the first run.

### shaders compile/link (P 394, G 394, V 394)

| cases | GPUs | cause | owner |
|---|---|---|---|
| 392 (P, V) / 390 (G) cases of `linkage` (85), `preprocessor` (276), `qualification_order` (15) and `large_constant_arrays` (16; 14 on G) | P G V | the programs compile and link; the verdict is a picture ("Image comparison failed"). Not measurable | none; needs a board |
| large_constant_arrays.indexing.float_128_vertex, vec4_128_vertex | G | the vertex shader is refused with `Abi(LoadBalancingTempsUnmeasured { num_out: 1, temps: 35, temps_max: 21 })`: a 128-element constant array indexed by a variable is unrolled into temporaries | plan #144 item **gc-const-array** (relative addressing in the constant file) |
| uniform_block.valid.long_instance_name_{fragment, vertex} | P G V | the link fails: a 63-character uniform name fills the reflection's 64-byte name field | RC13: "Reflection name of 63+ characters": no item yet |

## Reading these numbers

- The API-only groups are nearly finished on the host. The groups that fail
  because of a limit or a feature are 3 families: the **limits** the library
  reports under ES 3.0's minimums (colour attachments and draw buffers 3 against
  4; `MAX_SAMPLES` 1 against 4; varyings 32 / 8 against 60 / 15 on the PowerVR and
  V3D; uniform components 1,536 against about 50,000; 3 compressed formats against
  10), **multisample renderbuffers** (negative_api 2, state_query 2,
  fbo.completeness 7 not supported) and **integer and packed formats on the
  GC7000 and V3D** (negative_api 2, state_query 2, fbo.completeness 75 and 98).
- The 527 `buffer` failures and the 392 picture failures of the compile/link row
  are the black read-back. They are the only large numbers here and none of them
  is a defect that the host can show.
- No case was owned by a plan #18/#19 item that could be read from the repository.
  The unowned rows are a candidate list for follow-up items: the
  `COLOR_ATTACHMENT1`-only and depth/stencil-image completeness rules
  (28 cases, `check_status`), the unsized depth texture forms, `size.distinct`,
  `texsubimage` format/type validation, renderbuffer component sizes, the
  program-binary hint, and the V3D / GC7000 varying and uniform budgets.

## What is not measured here

- No board. A pass here says the GL layer answers as dEQP expects; a render or
  buffer read-back needs the AM62P, i.MX8MP or Pi 4.
- The first run kept no per-case list, so the delta is by count. The rows that
  are subgroups (`fbo.api`, `fbo.completeness`, `shaders compile/link`) have no
  earlier numbers to compare with.
- The `*.qpa` logs and the per-case JSON used for this document are in
  `build/host-deqp/r2-{powervr,vivante,v3d}` and are not committed.
