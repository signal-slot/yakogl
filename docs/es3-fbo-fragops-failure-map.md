<!-- SPDX-License-Identifier: Apache-2.0 -->
# dEQP-GLES3 fbo, fragment_ops and fragment_out: failure map

Written 2026-10-08 (`53c7ba34`) as input for the ES 3.0 work of
`docs/HANDOFF.md`, open work 4. Documentation only: nothing
here was run; every number is either a count of names in
`tools/cts/gles3-mustpass.txt`, a figure copied from the AM62P run, or
arithmetic on those two (the arithmetic is shown).

The blit rows (`fbo.blit.*`, `fbo.color.repeated_clear`, `fbo.invalidate.sub`
/ `whole`, `fragment_ops.scissor`) are the exception: they carry the per-case
results of the AM62P GLES3 tree at `c4ccc966`
(`records/am62p/deqp-gles3-c4ccc966`, tallied with `deqp-uart-tally.py
--mustpass tools/cts/gles3-mustpass.txt`) and what the host shows since
`glBlitFramebuffer` was built (`src/gl/blit.rs`). "Fixed on the host" means
the unit tests pass; no board has run it.

The run is the only GLES3 run there is: AM62P, main `396d667a`, 2026-09-26,
`tools/cts/RESULTS.md` §1r (heading at line 1437), summarised in
`tools/cts/log/status-2026-09-25-to-10-07.md`, "この先" item 2 (Japanese). The i.MX8MP and the Pi 4 have no
GLES3 run. Abbreviations used for sources:

- **R-group** = the per-group table, `tools/cts/RESULTS.md:1467` (fbo),
  `:1469` (fragment_ops), `:1470` (fragment_out).
- **R-bucket** = the "8,835 failures by message" table, `RESULTS.md:1499-1516`
  (only the 16 largest of 78 messages are in the repository; the other 62
  together are 8,835 − 7,530 = 1,305 cases and are not listed anywhere).
- **R-img** = the per-image table, `RESULTS.md:1552-1556` (images 2-5).
- **dEQP** = `$CCFARM_DEPS_DIR/VK-GL-CTS/modules/gles3/functional/` (pinned
  93bca01), **glu** = `.../framework/opengl/gluTextureUtil.cpp`.
- **Hardware caps document**: `docs/es3-backend-caps.md` (plan item `es3-caps`) was vetoed and reverted (3f78982e) and is not on `ccfarm/integration`. Nothing here is sourced from it; hardware statements below cite `docs/es3-pvr-formats.md` (**PVRF**) or are marked unverified.
- **PVRF** =
  `docs/es3-pvr-formats.md`.
- Code is cited at the checkout this file was written on (`src/gl/...`);
  `396d667a` has the same functions (checked: `glBlitFramebuffer: not
  implemented` and the `REPORT_ES2 && f == GL_SRGB8_ALPHA8` test are both in
  `src/gl/framebuffer.rs` at `396d667a`).

## 1. What is known, and what is not

| | cases | pass | fail | not supported | source |
|---|---:|---:|---:|---:|---|
| fbo | 2,028 | 657 | 1,116 | 255 | R-group `RESULTS.md:1467` |
| fragment_ops | 3,170 | 2,059 | 1,111 | 0 | `RESULTS.md:1469` |
| fragment_out | 940 | 206 | 619 | 115 | `RESULTS.md:1470` |
| **total** | 6,138 | 2,922 | **2,846** | 370 | |

The mustpass case counts equal pass + fail + n/s in every row (2,028 = 657 +
1,116 + 255; 3,170 = 2,059 + 1,111; 940 = 206 + 619 + 115).

**There is no per-case or per-subgroup result for these groups in the
repository or the deps cache.** The logs the run was tallied from
(`build/g3-396d667a/logs/uart-img*-396d667a.log`, `tally-final-with197.txt`) lived
in a worktree that was cleaned up (`RESULTS.md:1527-1529`); `tools/cts/testdata`
holds only parser fixtures and `imx8mp-full-tree-29460f96.results.tsv` is a
GLES2 table. What the repository does record, and all this map can use:

1. the group totals above;
2. the first-failure message buckets, with the groups each one reached
   (`RESULTS.md:1499-1516`), and for one bucket a split by subgroup
   (`RESULTS.md` Work-list row, "fragment_out 531 ... basic 240, array 240,
   random 51", the `AM62P GLES3 (§1r) | 1,483` row near line 989);
3. the per-image tallies for images 2-5, which cover these groups together
   with a few others (`RESULTS.md:1553-1556`).

So the "AM62P fails" column below is **"not recorded" unless a figure is
derived from one of those, with the derivation stated.** Where a subgroup has
no figure, the case population by name (a count of mustpass names, not a
failure estimate) and the bucket that probably holds it are given instead.
The per-subgroup figures come from a fresh AM62P run or a host run (see §6).

Two figures do not reconcile and are left as they are: image 2's patterns
(`draw.*` .. `fbo.depth.*`) match 1,905 mustpass names (322 draw + 76
draw_buffers_indexed + 1,507 fbo) but the row reports 1,956 results
(697 + 997 + 262, `RESULTS.md:1553`); and the fail figures of images 2-5 (997 + 204 + 1,106 + 784 = 3,091, `RESULTS.md:1553-1556`),
less draw's 50 and the 157 failures of the non-fbo groups in image 5
(implementation_limits 16, instanced 6, multisample 16, negative_api 31,
occlusion_query 62, pbo 26), are 2,884, not the 2,846 of the group table.
The cause was not recorded. The group table is what the tally script produced and is used as
the truth below.

## 2. The subgroup table

Columns: **subgroup** | **mustpass cases** (counted from
`tools/cts/gles3-mustpass.txt`) | **AM62P fails** | **bucket** | **dEQP source**
| **needs** | **plan item**. "Plan item" is the item expected to fix the
subgroup; `none` = no item in plan #3 covers it (§4); a chain `a, then b` means
the first fixes the first error and the second is then reached. `FB` =
`es3fFboTestCase.cpp`, `FBU` = `es3fFboTestUtil.cpp`. The first block of
numbers in a subgroup's "needs" is a count of mustpass names by format class,
not a failure count.

Format classes used below (names in the mustpass, per row of 36 colour formats
in `fbo.color.*`): **I** = the 19 integer formats (r8i r8ui r16i r16ui r32i
r32ui rg8i rg8ui rg16i rg16ui rg32i rg32ui rgba8i rgba8ui rgba16i rgba16ui
rgba32i rgba32ui rgb10_a2ui); **A2** = rgb10_a2; **F32** = the 4 formats that
need `GL_EXT_color_buffer_float` (r32f rg32f rgba32f r11f_g11f_b10f, glu
`getRenderableBitsES3`, `gluTextureUtil.cpp:1516-1522`), which yakogl does not
list (`src/gl/state.rs:109` lists only `GL_EXT_color_buffer_half_float`), so
dEQP reports them NotSupported; **F16** = r16f rg16f rgba16f (renderable with
the half-float extension, `:1526-1531`); **rgb16f** (1; not in glu's
renderable list); **ok** = the 8 ES 2.0-class formats (r8 rg8 rgb8 rgba8 rgb565
rgb5_a1 rgba4 srgb8_alpha8). 19 + 1 + 4 + 3 + 1 + 8 = 36.

### 2.1 fbo (2,028)

| subgroup | cases | AM62P fails | bucket (R-bucket) | dEQP source | needs | plan item |
|---|---:|---|---|---|---|---|
| fbo.api | 20 | not recorded for 396d667a (an earlier GLES3 image passed 20 / 20, `RESULTS.md:1408`; that is not the §1r run) | – | `es3fFboApiTests.cpp` | – | – |
| fbo.blit.conversion | 436 | 324 at `c4ccc966` (`records/am62p/deqp-gles3-c4ccc966`): "Got GL_INVALID_OPERATION" 302 (the stub), "Got GL_INVALID_ENUM" 23 (an RGB10_A2 rbo, taken since `468528a`); 112 n/s (F32 / R11F_G11F_B10F) | – | `es3fFramebufferBlitTests.cpp` (blit between two renderbuffers of different formats: 255 float/normalised pairs incl. F32, 81 signed-int, 100 unsigned-int pairs) | format conversion int→int / float→float, integer rbo for 181 cases, sRGB8_ALPHA8 as source and destination (32 names) | **fixed on the host** (blit: `src/gl/blit.rs`, a CPU copy with the conversions of ES 3.0.6 4.3.3). The 32 `srgb8_alpha8` names: the sRGB class decodes the read buffer and encodes the draw buffer there (`decode_float` / `encode_float`, item srgb, `docs/es3-srgb.md` 3.2) |
| fbo.blit.default_framebuffer | 106 | 84 at `c4ccc966`: "GL_INVALID_OPERATION" (the stub) 84; 22 n/s | – | same file | `glBlitFramebuffer` to/from the window surface (stored top-down), LINEAR scale, out-of-bounds | **fixed on the host** (blit) |
| fbo.blit.depth_stencil | 28 | 16 at `c4ccc966`: "GL_INVALID_OPERATION" 16; 12 n/s (separate depth and stencil) | – | same | blit of D16, D24, D32F, D24S8, D32F_S8, S8 (depth-only, stencil-only, scaled) | **none**: the blit validates them and refuses the copy with GL_INVALID_OPERATION; it needs a depth/stencil copy in each back end (the planes live in the back ends' own buffers, `src/ds_retention.rs`) |
| fbo.blit.rect | 40 | 40 at `c4ccc966`: "GL_INVALID_OPERATION" (the stub) | – | same | blit with reversed rectangles, scaling, clamping, NEAREST / LINEAR, NEAREST consistency | **fixed on the host** (blit) |
| fbo.color.blend | 14 | not recorded | INVALID_ENUM 270 (A2), Compile failed 181, Image comparison failed 675 | `es3fFboColorbufferTests.cpp` (blend on a colour rbo) | A2 (1), F32 (1), sRGB blend in linear space (1), others ok | int-rt-gl (A2 rbo); sRGB part: **none** (§3, item 1) |
| fbo.color.clear | 36 | not recorded | INVALID_ENUM 270 for I (19) + A2 (1); F32 (4) n/s | same | `glClearBufferiv/uiv` and integer `glReadPixels` for I; classes above | int-rt-gl, int-rt-pvr / -v3d / -gc |
| fbo.color.repeated_clear | 108 | `blit` 64 at `c4ccc966` (62 "Got GL_INVALID_OPERATION", the stub; rgb10_a2 INVALID_ENUM, rgb16f tex2d incomplete), 8 n/s | Image comparison failed 675 | same | `blit` 72 (clear, blit to a second fbo), `sample` 36 (clear, sample as a texture) | blit part **fixed on the host** (blit; the `tex2d` integer names also need integer textures as render targets); sample part not known |
| fbo.color.tex2d | 36 | not recorded | INVALID_ENUM 270 / Compile failed 181 / Image comparison failed 675 | same + `FBU` shaders (`genTexFragmentShader`, `FBU:260`: `isampler2D` / `usampler2D` and `ivec4` / `uvec4` outputs for I) | texture attached to an fbo in each class; integer texture upload, integer sampling and integer render | int-rt-gl, formats-pvr / -v3d / -gc, int-rt-pvr / -v3d / -gc |
| fbo.color.tex2darray | 36 | not recorded | same | same + `Texture2DArrayShader` (`FBU:587`) | `glFramebufferTextureLayer` on an array, `sampler2DArray` sampling (PowerVR samples an array as one tall image, PowerVR shader back end; see plan item `pvr-2darray`, not re-verified here) | int-rt-gl, pvr-2darray, formats-*, int-rt-* |
| fbo.color.tex3d | 36 | not recorded | same | same + a 3D shader | `glFramebufferTextureLayer` on a 3D texture, `sampler3D` | int-rt-gl, formats-*, int-rt-* |
| fbo.color.texcube | 36 | not recorded | same | same + `TextureCubeShader` (`FBU:455`) | cube face attachments in each class | int-rt-gl, formats-*, int-rt-* |
| fbo.completeness.attachment_combinations | 81 | not recorded | – | `es3fFboCompletenessTests.cpp` | colour / depth / stencil texture or rbo combinations | – (no bucket names it) |
| fbo.completeness.layer | 8 | not recorded | – | same | `glFramebufferTextureLayer` layer / size completeness on 2D-array and 3D | tex3d-levels (host gap: NPOT mips ≥ 1), else none |
| fbo.completeness.renderable.renderbuffer | 230 | not recorded | not named in R-bucket | same + glsFboCompletenessTests | 76 colour rows, 77 depth rows, 77 stencil rows; a format ES 3.0 makes renderable must give a complete fbo, others must not | int-rt-gl for the I and A2 colour rows (they are refused today, `src/gl/framebuffer.rs:455-478`); the rest not known |
| fbo.completeness.renderable.texture | 226 | not recorded | not named in R-bucket | same | 72 colour rows, 77 depth, 77 stencil, as texture attachments | sized-formats-gl first (the texture must exist), then int-rt-gl |
| fbo.completeness.samples | 13 | not recorded | INVALID_OPERATION 252 | same | `glRenderbufferStorageMultisample` with 1, 2, 4 samples and mixed fbos; refused above 1 (`src/gl/framebuffer.rs:2587`, `GL_MAX_SAMPLES` is 1) | **none** (MSAA) |
| fbo.completeness.size | 2 | not recorded | – | same | `distinct` size attachments (complete in ES 3.0), `zero` | none (small; check) |
| fbo.depth | 15 | not recorded | Compile failed 181 | `es3fFboDepthbufferTests.cpp` (basic, depth_test_clamp, depth_write_clamp ×5) | D32F and D24 / D16 rbo and textures, float depth written by the shader | **none** (depth-format part; shader part reading: formats-pvr) |
| fbo.invalidate.default | 25 | not recorded | – | `es3fFboInvalidateTests.cpp` | `glInvalidateFramebuffer` on the default fbo; validation-only is correct (`src/gl/framebuffer.rs:2633`: "colour is always kept, so an invalidate is a no-op") | – (reading: pass) |
| fbo.invalidate.format | 41 | not recorded | INVALID_ENUM 270 (I 19 + A2 1), Image comparison failed 675 | same | one case per format (35 colour: I 19, A2 1, F32 4, F16 3, ok 8; plus 5 depth, 1 stencil) | int-rt-gl |
| fbo.invalidate.sub | 19 | 8 at `c4ccc966`: `unbind_blit*` 4 ("GL_INVALID_OPERATION", the stub), `unbind_blit_msaa*` 4 (incomplete: no multisample storage) | – | same | `glInvalidateSubFramebuffer`, blit | blit part **fixed on the host**; MSAA part **none** |
| fbo.invalidate.target | 27 | not recorded | – | same | all `GL_FRAMEBUFFER` / `DRAW` / `READ` target combinations, validation | – (reading: pass) |
| fbo.invalidate.whole | 19 | 11 at `c4ccc966`: the same 4 + 4 blit cases, and 3 `unbind_read_color*` "Image comparison failed" | – | same | as `sub` | blit part **fixed on the host**; MSAA part **none**; `unbind_read_color*` not known |
| fbo.msaa.2_samples, 4_samples, 8_samples | 22 + 22 + 22 | not recorded | INVALID_OPERATION 252 (storage refused above 1 sample; resolve via blit) | `es3fFboMultisampleTests.cpp:276-330, 372-404` | multisample rbo of each format (21 colour + depth / stencil formats) at 2 / 4 / 8 samples, then `glBlitFramebuffer` resolve (`:180, :330`); the hardware sample counts of the three GPUs are unverified here | **none** (MSAA + blit) |
| fbo.msaa.renderbuffer_resize | 4 | not recorded | INVALID_OPERATION 252 | same | multisample → non-multisample and back | **none** (MSAA + blit) |
| fbo.multiview | 3 | 0 (reading: n/s, in the 255 of `RESULTS.md:1467`) | – | `es3fMultiviewTests.cpp` | `GL_OVR_multiview` samples_1 / 2 / 4 | – (extension absent: n/s is right) |
| fbo.render.recreate_color | 72 | not recorded | Image comparison failed 675 / Compile failed 181 | `es3fFboRenderTest.cpp` | rbo / texture colour in the ES 3.0 formats, repeated recreation | not known (reading: int-rt-gl for the I / A2 names) |
| fbo.render.recreate_depth_stencil | 11 | not recorded | same | same | D24S8, D32F_S8 recreate | not known |
| fbo.render.resize | 94 | not recorded | same | same | resize of rbo / texture in 3 colour + depth cases | not known |
| fbo.render.shared_colorbuffer | 84 | not recorded | same | same | one colour buffer shared by two fbos | not known |
| fbo.render.shared_colorbuffer_clear | 34 | not recorded | same | same | clears of a shared colour buffer | not known |
| fbo.render.shared_depth_stencil | 10 | not recorded | same | same | D24S8 shared | not known |
| fbo.render.stencil_clear | 3 | not recorded | same | same | stencil clear | not known |
| fbo.stencil | 9 | not recorded | – | `es3fFboStencilbufferTests.cpp` (attach 4, basic 5) | stencil rbo / texture | not known |
| **fbo total** | **2,028** | **1,116** | | | | |

What bounds the fbo figure: of the 1,116 failures, at least 270 + 252 + 181 =
**703** are in the three buckets whose group list names only fbo subgroups
(`RESULTS.md:1510, 1511, 1513`: INVALID_ENUM 270 and INVALID_OPERATION 252 in
blit / color / msaa; Compile failed 181 in color / depth / render). The other
**413** are in "Image comparison failed" (675, shared with fragment_out and four
other groups, `RESULTS.md:1504`), in the unlisted 62 buckets, or both; their
split is not recorded.

### 2.2 fragment_ops (3,170)

| subgroup | cases | AM62P fails | bucket | dEQP source | needs | plan item |
|---|---:|---|---|---|---|---|
| fragment_ops.blend.default_framebuffer.equation_src_func_dst_func | 677 | not recorded; the 5 failures that §2.2 puts outside fbo_srgb sit in blend.default_framebuffer, scissor or stencil | – | `es3fBlendTests.cpp` | blend factors on the window surface; MIN / MAX are accepted (`src/gl/state.rs:637`) | – (reading: pass; ES 2.0 blend 1,060 / 1,060 pass, `STATUS.md:165`) |
| …default_framebuffer.rgb_equation_alpha_equation | 25 | as above | – | same | ES 3.0 MIN / MAX separate RGB / alpha equations | – (reading: pass; **MIN / MAX in the three back ends not checked here**) |
| …default_framebuffer.rgb_func_alpha_func | 450 | as above | – | same | | – |
| fragment_ops.blend.fbo_srgb.equation_src_func_dst_func | 677 | **1,106 to 1,111 over the three fbo_srgb rows** (derived below) | Image compare failed 1,293 | `es3fBlendTests.cpp:199-206` (`GL_SRGB8_ALPHA8` rbo of 2× viewport), `:316-321` (reference converted with `sRGBToLinear`) | blend in an SRGB8_ALPHA8 colour buffer: the destination is decoded, the blend result encoded | **none** (§3, item 1) |
| …fbo_srgb.rgb_equation_alpha_equation | 25 | (in the 1,106-1,111) | same | same | | **none** |
| …fbo_srgb.rgb_func_alpha_func | 450 | (in the 1,106-1,111) | same | same | | **none** |
| fragment_ops.depth | 8 | 0 if the image-4 reading holds (below) | – | `es3fDepthTests.cpp` | `cmp_*` ×8 | – |
| fragment_ops.depth_stencil | 621 | same | – | `es3fDepthStencilTests.cpp` (random 25, stencil_depth_funcs 81, stencil_ops 512, write_mask 3) | | – |
| fragment_ops.interaction | 94 | same | – | `es3fFragOpInteractionTests.cpp` (basic_shader) | | – |
| fragment_ops.random | 100 | same | – | `es3fRandomFragmentOpTests.cpp` | | – |
| fragment_ops.scissor | 26 | `framebuffer_blit_*` 3 at `c4ccc966` ("GL_INVALID_OPERATION", the stub); the rest not recorded | – | `es3fScissorTests.cpp` (clear_int_buffer and clear_uint_buffer are 2 of the names; blit ×3 `framebuffer_blit_*`) | `glClearBufferiv/uiv` on an integer buffer (only the `clear_*_buffer` names), `glBlitFramebuffer` into the window under a scissor box for the 3 blit names | `framebuffer_blit_*` (3): **fixed on the host** (the scissor box cuts the blit's destination); the rest – |
| fragment_ops.stencil | 17 | not recorded (≤ 5 with the scissor row) | – | `es3fStencilTests.cpp` | | – |
| **fragment_ops total** | **3,170** | **1,111** | | | | |

Derivation of the fbo_srgb figure. The "Image compare failed" bucket is 1,293
cases and names fbo_srgb, `ubo.single_nested_struct*` and `ubo.random` as its
groups (`RESULTS.md:1502`). ubo has 187 failures in total (`RESULTS.md:1490`), so the ubo share is at most 187 and fbo_srgb has **at least
1,293 − 187 = 1,106** failures. fragment_ops has 1,111 failures in all, so
fbo_srgb has **at most 1,111**. Of fbo_srgb's 1,152 cases, 41 to 46 pass.
Image 4 (`fragment_ops.blend.fbo_srgb.*` .. `fragment_ops.random.*`, 1,975
names) reports 1,106 failures (`RESULTS.md:1555`): if the per-image figure is
the final tally's, the 1,106 are exactly the fbo_srgb floor and **fbo_srgb is
1,106 with depth, depth_stencil, interaction and random all passing**, and the
remaining 5 fragment_ops failures sit in `blend.default_framebuffer` (image 3)
or in `scissor` / `stencil` (image 5), with no record of which.

Cause (code, not a log): the PowerVR / host surface has "no sRGB encode or
decode anywhere" — `GL_SRGB8_ALPHA8` "maps to plain RGBA8" (`src/gl/state.rs:105-107`;
`src/gl/framebuffer.rs:490-497`). dEQP renders into an SRGB8_ALPHA8 rbo and
compares against a reference blended in linear space and converted back
(`es3fBlendTests.cpp:316-331`), so blending in the encoded space fails the
comparison in almost every case. Hardware: PowerVR's PBE has a `gamma` bit and
the accumulator is F16, so sRGB writes encode (PVRF §2.1 rows SRGB8 /
SRGB8_ALPHA8, `:113-114, :282-290`). V3D and GC7000UL sRGB support is
unverified here.

Whether blending then happens in linear space on each back end is **not
established** and is the work of `fbo-fragops`.

### 2.3 fragment_out (940)

| subgroup | cases | AM62P fails | bucket | dEQP source | needs | plan item |
|---|---:|---|---|---|---|---|
| fragment_out.basic.float | 84 | not recorded; fbo setup error counts below | n/s for F32 (rgba32f, r11f_g11f_b10f, rg32f, r32f: 4 formats × 12 = 48); F16 (3 × 12 = 36) pass or image-fail | `es3fFragmentOutputTests.cpp:1062-1112` (`requiredFloatFormats`, 7 formats × 3 precisions × float / vec2 / vec3 / vec4) | – | – (n/s is right while `GL_EXT_color_buffer_float` is unlisted) |
| fragment_out.basic.fixed | 108 | not recorded | INVALID_ENUM 531 for rgb10_a2 (12) | `:1063-1065, :1114-1143` (9 formats: rgba8, srgb8_alpha8, rgb10_a2, rgba4, rgb5_a1, rgb8, rgb565, rg8, r8) | RGB10_A2 rbo (12 names) | int-rt-gl / int-rt-* (ES 3.0 colour render-target formats); sRGB8_ALPHA8 output (12 names) writes encoded: **none** (§3, item 1) |
| fragment_out.basic.int | 108 | all 108 (derived below) | INVALID_ENUM 531 | `:1066-1067, :1145-1174` (9 integer formats × 12) | integer rbo, `ivec` fragment outputs, integer `glReadPixels` | int-rt-gl, int-rt-pvr, int-rt-v3d, int-rt-gc |
| fragment_out.basic.uint | 120 | all 120 | INVALID_ENUM 531 | `:1068, :1176-1205` (10 formats incl. RGB10_A2UI × 12) | as int | int-rt-gl, int-rt-pvr / -v3d / -gc |
| fragment_out.array.float | 84 | not recorded | F32 n/s 48 | `:1209-1280` (`numTargets = 3` at `:1215`) | array output `out vec4 o[3]`, 3 draw buffers (met: `CAP_COLOR_ATTACHMENTS` = 3, `src/gl/context.rs:139`) | – |
| fragment_out.array.fixed | 108 | not recorded | INVALID_ENUM for rgb10_a2 (12) | same | 3 draw buffers of one format | int-rt-gl, int-rt-*; sRGB: **none** |
| fragment_out.array.int | 108 | all 108 | INVALID_ENUM 531 | `:1282-1314` | as basic.int, with 3 targets | int-rt-gl, int-rt-* |
| fragment_out.array.uint | 120 | all 120 | INVALID_ENUM 531 | `:1316-1346` | as basic.uint | int-rt-gl, int-rt-* |
| fragment_out.random | 100 | not recorded; ≥ 51 | INVALID_ENUM 531 (51 of the bucket) | `:1008-1060`, `:1346-1353` (`createRandomCase(..., 2, 4, seed)`: 2 to 4 targets, random float / int / uint formats) | integer or A2 targets (the 51); cases with 4 targets need `MAX_DRAW_BUFFERS` 4 (3 today, `CAP_COLOR_ATTACHMENTS`, `src/gl/context.rs:139`); some are n/s for F32 | int-rt-gl, int-rt-*, mrt4 (for the 4-target seeds; which seeds have four is not recorded) |
| **fragment_out total** | **940** | **619** | | | | |

Derivation of the integer-output rows. The INVALID_ENUM bucket "After
framebuffer setup" is 531 cases: basic 240, array 240, random 51
(`RESULTS.md` Work-list row `AM62P GLES3 (§1r) | 1,483`, near line 989).
Basic and array each have 108 `int` + 120 `uint` = 228 integer cases plus 12
`rgb10_a2` cases in `fixed` = **240**: the bucket's 240 equals exactly the
integer names plus the RGB10_A2 names, and nothing else (the F32 names are
n/s, the sRGB, RGB565, RGBA4, RGB5_A1, RGB8, RG8, R8 and F16 names have
renderable formats). That is consistent with the cause shown in code:
`is_color_format` has no integer format, no RGB10_A2 and no R11F / RG32F /
RGBA32F (`src/gl/framebuffer.rs:455-478`), so `glRenderbufferStorage` raises
GL_INVALID_ENUM. n/s adds up too: F32 is 4 formats × 12 names × 2 groups = 96,
and 19 of the 100 random cases make 115 (`RESULTS.md:1470`); the 19 is a
subtraction, not a record.

The remaining **619 − 531 = 88** fragment_out failures are not accounted for.
They are in the "Image comparison failed" bucket (675, which names
fragment_out) or the unlisted buckets. The population they can come from: the
passing-class names (basic and array fixed 96 + float 36, each ×2 = 264 names)
and the 49 non-INVALID_ENUM random cases; the pass count (206) says at most
264 + 49 − 88 of them pass.

## 3. The ranked list: subgroups no plan item covers (`none`)

Ranked by AM62P failures. A "fail count" is shown only where §2 derives one;
otherwise the count is the **ceiling from the bucket** (the largest number of
failures the recorded evidence allows) or "not recorded", and the rank among
those uses the mustpass cases that depend on the missing function. This is
the scope for item `fbo-fragops`.

| rank | subgroup(s) | fails | suspected cause (code evidence) | needs |
|---:|---|---|---|---|
| 1 | fragment_ops.blend.fbo_srgb.* (1,152 cases) | **1,106 – 1,111** (derived, §2.2) | no sRGB encode / decode: SRGB8_ALPHA8 is stored as plain RGBA8 (`src/gl/state.rs:105`, `src/gl/framebuffer.rs:490`); dEQP's reference blends in linear space | sRGB write encode and destination decode for blending in each back end (PVRF `:113-114, :282-290`); a decision whether this is done in the GL layer (convert in the shader) or the colour-buffer format. Also fixes `fbo.color.blend` srgb8_alpha8, the sRGB names of `fragment_out.*.fixed` (24) and `fbo.blit.conversion.*srgb8_alpha8*` (32 names) |
| 2 | fbo.blit.depth_stencil (16 at `c4ccc966`) | 16 | the colour blit is built (`src/gl/blit.rs`: every class pair, NEAREST / LINEAR, scaling, flips, cut rectangles, scissor, the window as source and destination; fixed on the host, the AM62P rows of `tools/cts/am62p-gles3-tree.txt` 1, 2 and 4 to rerun); a depth or stencil copy is validated and then refused with GL_INVALID_OPERATION, said once in the log | a depth/stencil copy in each back end: the planes live in the back ends' buffers (`src/ds_retention.rs`; the PowerVR's packed D24S8 rows in its ZLS layout, `pvr_backend_depth_resolve`), so a back-end entry that copies between two objects' planes (or writes one from the CPU) and keeps their plane state |
| 3 | fbo.msaa.* (70) + fbo.completeness.samples (13) + the 8 MSAA blit cases of fbo.invalidate | ≤ 252 (same bucket; not split) | multisample renderbuffers are refused above one sample (`src/gl/framebuffer.rs:2587`), `MAX_SAMPLES` is 1 on all three back ends (`glRenderbufferStorageMultisample` raises GL_INVALID_OPERATION above 1, `src/gl/framebuffer.rs:2587`; no plan item covers MSAA); resolve needs the blit of rank 2 | `glRenderbufferStorageMultisample` with 2 / 4 samples, resolve into a single-sample rbo, `glGetInternalformativ` sample counts; hardware sample counts per GPU: unverified (4x already exists in the AM62P 4x-MSAA image config `tools/cts/am62p-ms4-tree.txt`, not through the ES 3.0 rbo API). Case count: 91 |
| 4 | fbo.depth (15), fbo.render.* (308), fbo.stencil (9), fbo.color.tex2darray / tex3d (72) | part of "Compile failed 181" (`RESULTS.md:1513`: color, depth, render) and "Image comparison failed 675"; **no split recorded** | reading from the sources: the fbo test shaders (`FBU:260`, `:455`, `:587`) use `isampler` / `usampler`, integer outputs and `sampler2DArray` / `sampler3D`; a "Compile failed" is the online compiler or the back end refusing one of them. Not a **none** if it is the integer samplers (formats-pvr / -v3d / -gc) or the array sampler (pvr-2darray) | measure first (host run or a rerun with the `glu` program log); then assign |
| 5 | fbo.depth float-depth and D32F cases | not recorded | `GL_DEPTH_COMPONENT32F` and `DEPTH32F_STENCIL8` as rbo / texture and as blit source (hardware support unverified here) | the depth-format list of `is_depth_format`, not read here |
| 6 | fbo.completeness.layer (8), fbo.completeness.size (2) | not recorded | layer attach rules on 3D / array, `size.distinct` | small; read the cases |
| – | the 88 unexplained fragment_out failures | 88 (derived) | not a cause: unaccounted (§2.3) | rerun fragment_out.basic.fixed / float and array.fixed / float, and read the "Image comparison failed" cases |

Ranking note: rank 1 is the largest. Rank 2 is the 16 depth/stencil blit
cases the colour blit leaves; rank 3 (91 names) needs multisample storage
first, and its resolves then go through the colour blit.

## 4. Plan items and what each takes out of these groups

Counts are mustpass **names that depend on the item** (not failures fixed).
Failures fixed need the per-subgroup run of §6.

| item | takes out | names in these three groups | evidence |
|---|---|---:|---|
| int-rt-gl | rbo / texture colour targets of the integer formats and RGB10_A2 / RGB10_A2UI, `glClearBuffer{i,ui}v`, integer `glReadPixels`, `glGetError` after fbo setup | fragment_out basic + array 456 (I) + 24 (A2) + random ≥ 51; fbo.color 5 × 20 = 100 (I 19 + A2 1 per row); fbo.invalidate.format 20; fbo.blit.conversion integer pairs 181 | `RESULTS.md:1499-1516`; `src/gl/framebuffer.rs:455-478` |
| int-rt-pvr / -v3d / -gc | integer fragment outputs and PBE / TLB / PE formats | same names as int-rt-gl, after it | PVRF (PowerVR); V3D and GC7000UL unverified |
| sized-formats-gl, formats-pvr / -v3d / -gc | texture upload and sampling of the ES 3.0 sized formats for the texture-attached cases | fbo.color.tex2d / tex2darray / tex3d / texcube (144, the I / F / A2 rows), fbo.completeness.renderable.texture colour rows (72) | PVRF (PowerVR); V3D and GC7000UL unverified |
| mrt4 | `MAX_DRAW_BUFFERS` 4 | the 4-target seeds of fragment_out.random (count not recorded); the 3-target `array` cases already fit | `src/gl/context.rs:139`, `es3fFragmentOutputTests.cpp:1215` |
| pvr-2darray | array textures sampled as arrays | fbo.color.tex2darray (36) | plan item text; not re-verified |
| tex3d-levels | NPOT array / 3D levels ≥ 1 | fbo.completeness.layer (8) if the cases use NPOT mips (not checked) | `RESULTS.md:996` |
| blit (`src/gl/blit.rs`) | the colour half of §3's former rank 2 | 701 names depend on `glBlitFramebuffer` (blit 610 + repeated_clear.blit 72 + invalidate 16 + scissor 3); the depth/stencil 28 of them are not taken out | the unit tests of `src/gl/blit.rs`, gl_test 202 `blitconv` |
| none → fbo-fragops | §3 ranks 1-3 | 1,152 + 28 + 91 | §3 |

## 5. Hardware facts that bound the `none` items

Only PowerVR has a hardware document in the repository (PVRF); the V3D and
GC7000UL statements the first draft made came from the withdrawn caps document
and are **unverified** and removed.

- **sRGB render (PowerVR)**: the PBE has a `gamma` bit and the accumulators are
  F16 (PVRF `:113-114`, `:282-290`). V3D and GC7000UL: unverified.
- **MSAA**: the repository's only evidence is the GL layer (`GL_MAX_SAMPLES`
  1, `src/gl/framebuffer.rs:2587`) and the AM62P 4x-MSAA image config
  `tools/cts/am62p-ms4-tree.txt`. Hardware sample counts per GPU and how dEQP
  treats a count above `GL_MAX_SAMPLES` for the 2x and 8x `fbo.msaa` names
  (`es3fFboMultisampleTests.cpp:372-404`) were not established.
- **Blit**: a CPU copy in the GL core (`src/gl/blit.rs`), on the three GPUs
  alike; no hardware blit path is used or documented.
- **Integer targets (PowerVR)**: PBE signed/unsigned pack modes exist (PVRF
  integer sections); V3D and GC7000UL: unverified.

## 6. What to do next (for es3-inventory and fbo-fragops)

1. **Get per-subgroup results.** The cheapest source is the next AM62P GLES3
   tree run (`tools/cts/am62p-gles3-tree.txt` images 2 to 5 cover exactly
   these groups) with `deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt`
   keeping the per-case lines (the previous logs were not kept). For the API
   buckets (INVALID_ENUM, INVALID_OPERATION, completeness) a host run of the
   dEQP-GLES3 fbo and fragment_out groups (plan item `host-gles3-run`)
   reproduces them without a board; blend and image buckets need a board.
2. **Split the 413 + 88 unexplained fbo / fragment_out failures** and the 62
   unlisted buckets (1,305 cases) before ranking beyond §3 rank 3.
3. **fbo-fragops scope**, in order: (a) fbo_srgb (§3 rank 1, 1,106+ failures,
   independent of the other items); (b) the depth/stencil half of
   `glBlitFramebuffer` (rank 2; the colour half is built); (c) multisample
   renderbuffers and resolve (rank 3); (d) re-measure the remaining fbo
   buckets after int-rt-* and formats-* merge.
4. After int-rt-gl lands, the INVALID_ENUM bucket (270 + 531) should collapse;
   the ES 2.0 guard test (`es2-guard`) must still show the same answers for
   `GL_SRGB8_ALPHA8` and `GL_RGB10_A2` on `report-es2`
   (`src/gl/framebuffer.rs:490` is a REPORT_ES2 special case).

## 7. Assumptions made writing this

- RGB10_A2 / RGB10_A2UI are assigned to the `int-rt-*` items (item titles
  say "integer and ES 3.0 colour render-target formats"); `int-rt-gl`'s own
  title mentions integer targets only.
- Subgroups whose bucket the repository does not name are marked "not known"
  or "reading", never given a count.
- "Reading: pass" for fbo.api, invalidate.default / target and depth, stencil,
  interaction, random subgroups rests on the totals (no bucket names them) and,
  for fragment_ops, on the image-4 arithmetic of §2.2; neither is a per-case
  record.
