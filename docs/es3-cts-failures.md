<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 CTS failure inventory

dEQP-GLES3, the mustpass list `tools/cts/gles3-mustpass.txt` (42,810 cases),
on the AM62P (PowerVR). Written 2026-10-10 for plan #3 (OpenGL ES 3.0, item
`es3-inventory`) from the `396d667a` run, and modelled on
`docs/es2-cts-failures.md`. The root causes below keep that run's counts; the
section "State on `f2a13877`" says which of them have a fix on main and what the
latest tree reads. Appendix A (the vertex groups) and Appendix B (fbo,
fragment_ops and fragment_out by subgroup) were separate documents until
2026-10-11.

## State on `f2a13877` (2026-10-11)

The whole AM62P tree ran again on `f2a13877`, 28 images (`tools/cts/RESULTS.md`,
"AM62P: the whole dEQP-GLES3 tree on `f2a13877`"): **32,669 pass / 5,549 fail /
2,318 NotSupported / 185 QualityWarning / 2,089 without a result**. Failures by
group, against `c4ccc966` (2026-10-10) and the `396d667a` run below:

| group | `f2a13877` | `c4ccc966` | `396d667a` |
|---|---:|---:|---:|
| shaders | 412 | 1,603 | 2,231 |
| texture | 1,752 (+ 1,646 NotSupported) | 1,451 | 2,612 |
| fragment_ops | 1,109 | 1,111 | 1,111 |
| fbo | 872 | 1,000 | 1,116 |
| transform_feedback | 640 (+ 191 NotSupported) | 675 | 461 |
| fragment_out | 595 | 619 | 619 |
| attribute_location | 48 | 48 | 48 |
| vertex_arrays | 41 | 41 | 41 |
| state_query | 26 | 35 | 63 |
| rasterization | 21 | 21 | 21 |
| draw | 14 | 50 | 50 |
| implementation_limits | 11 | 9 | 16 |
| instanced | 6 | 6 | 6 |
| lifetime | 1 | 1 | (no result) |
| uniform_api | 1 | 1 | 27 |
| buffer | 0 | 300 | 0 |
| read_pixels | 0 | 24 | 24 |
| default_vertex_attrib | 0 | 16 | 16 |

The texture count rose because 1,447 more of its cases ran. The 2,089 without a
result: image 5's tail 558 (it aborted at a `lifetime` case before
transform-feedback capture existed: `lifetime` 48, `multisample` 64,
`negative_api` 312, `occlusion_query` 62, `pbo` 49, `polygon_offset` 20,
`prerequisite` 3), image 7's tail 1,397 (a `ResourceError`: a 24,384-vertex
draw does not fit the 1 MiB general arena), the two images that hang the GPU 83,
and 51 lost in capture. Every group not in the table has 0 failures on
`f2a13877` (the 5,549 are the listed rows' sum); for `occlusion_query`,
`multisample`, `negative_api`, `pbo` and `polygon_offset` that is because image
5's tail never ran. No per-case list of either later run is written down here.

Which root cause has a fix on main (each built on the host; "board" says
whether the `f2a13877` tree contains it):

| root cause | fix on main | board |
|---|---|---|
| RC1 integer and ES 3.0 colour targets | GL core `b9b2ac00`; PowerVR writes all nineteen integer formats `fc658e64` (merged in `a27d69c0`); the GC7000 and V3D refuse by name | in the tree: shaders fell from 2,231 to 412 |
| RC2 sized formats | GL core `888af5d0`, `e5c4a180`; PowerVR samples every class but `RGB10A2` `25267d17` (merged in `a27d69c0`) | in the tree |
| RC3 transform feedback | API `16ac8641`; capture on the CPU `896ca724` (merged in `c4dce9125`) | capture not in the tree (it landed after) |
| RC5 shadow look-ups | the online compiler takes the shadow, projective and 3D bias forms (merged in `a27d69c0`) | in the tree |
| RC6 PowerVR 2D arrays | none: designed in `docs/es3-pvr-2darray.md` | |
| RC7 sRGB | `3c433ed2` (merged in `c4dce9125`) | not in the tree |
| RC8 blit and MSAA | colour blit `39b18f9b` (merged in `c4dce9125`); depth/stencil blit and multisample storage: none | not in the tree |
| RC11 occlusion queries | `2241cd84`, `d9573ab9`, `dcac96cb`, `7526b084` (merged in `452d6c31`) | in the tree, but `occlusion_query` is in image 5's unrun tail |
| RC12 default_vertex_attrib | `8b3e7039` | in the tree: 16 -> 0 |
| RC13 compiler gaps | link rules `b88fea5e`, `03ff7a63`; line continuation `79fe1718`; `centroid` on the PowerVR and V3D `e5874527` | in the tree |
| RC14 MRT | `GL_MAX_DRAW_BUFFERS` is 3 on every GPU (`src/gl/context.rs` `MAX_DRAW_BUFFERS`); ES 3.0 asks for 4 | |

The RESULTS.md line numbers cited below are those of the day this was written;
the file has since moved by one line (section 1r now starts at line 1438).

## How to read the counts

The only full GLES3 run is AM62P, Zephyr, main `396d667a`, 2026-09-26:
**31,278 pass, 8,835 fail, 2,636 NotSupported, 5 QualityWarning, 56 with no
result** (all of `lifetime.*`). Source: `tools/cts/RESULTS.md` §1r (heading at
line 1437, group table lines 1457-1494, "8,835 failures by message" lines
1496-1516), and `tools/cts/log/status-2026-09-25-to-10-07.md`, "この先" item 2 (Japanese). The i.MX8MP
and the Pi 4 have **no GLES3 run**, so everything below is AM62P only.

There is **no per-case list**. The per-case session of 2026-10-08 got no console
output and wrote no TSV: its lease named the wrong UART port
(`tools/cts/log/investigations/am62p-console-empty.md`, section 8). The logs of the 396d667a
run were deleted with a worktree cleanup (RESULTS.md lines 1527-1529). So causes
are assigned by **message bucket** and by group, and every count carries a tag:

| tag | meaning |
|---|---|
| **exact** | the number is written in the cited source (a group row, a bucket row, a work-list row) or is a sum/difference of such numbers (the arithmetic is shown) |
| **bound** | a lower bound (`≥`), an upper bound (`≤`) or a range that the cited numbers allow; the true count is not recorded |
| **unclassified** | failures that belong to a group but to no known root cause; no number is invented for how they split |

Abbreviations for sources: **R-grp** = RESULTS.md §1r group table (line =
group's row); **R-msg** = RESULTS.md §1r failures-by-message table (the 16
largest of 78 messages; the other 62 buckets together are 8,835 − 7,530 =
**1,305** cases and are not listed anywhere in the repository); **R-row** =
the "AM62P GLES3 (§1r)" rows of the RESULTS.md work list (lines 989-993) and the
neighbouring "AM62P GLES3 (shadow/depth uploads)" rows (lines 994-997).

**Host reproduction.** `tools/cts/host-deqp-run.sh` and `tools/hostharness/`
run the library on the host with a recording platform layer: the command stream
is kept, nothing is rendered. So compile/link verdicts and GL errors (API
buckets) reproduce; picture verdicts do not and must never be quoted. In the
tables below "host: yes" means the cause is reproduced or pinned by a host test
(named), "host: API only" means the GL-layer half reproduces and the picture
half needs the board, "host: no" means it needs the GPU.

## Summary by group (reconciliation with 8,835)

Fail counts per dEQP group are **exact** (R-grp, `RESULTS.md:1457-1493`). They
sum to **8,835** (checked: 48 + 16 + 50 + 1,116 + 1,111 + 619 + 16 + 6 + 16 +
31 + 62 + 26 + 21 + 51 + 24 + 2,231 + 63 + 2,612 + 461 + 187 + 27 + 41 = 8,835;
all other groups have 0 failures). The last two columns split each group into
what a root cause below accounts for (**lower bound**, from exact buckets) and
the rest.

| group | fail (exact) | classified (≥) | unclassified (≤) | root causes (section) |
|---|---:|---:|---:|---|
| shaders | 2,231 | 1,483 | 748 | RC1 (1,483); rest: section "Unclassified" |
| texture | 2,612 | 1,378 | 1,234 | RC2 (736), RC5 (432 shadow), RC6 (210 2d_array) |
| fbo | 1,116 | 703 | 413 | RC1 (270), RC8 (≤ 252), RC9 (181) |
| fragment_ops | 1,111 | 1,106 | ≤ 5 | RC7 (1,106 to 1,111) |
| fragment_out | 619 | 531 | 88 | RC1 (531) |
| transform_feedback | 461 | 461 | 0 | RC3 (413), RC4 (48) |
| ubo | 187 | 0 | 187 | RC7b/unclassified (≤ 187 share of an image-compare bucket) |
| state_query | 63 | 63 | 0 | RC10 (29 TF + 34 limits and queries) |
| occlusion_query | 62 | 62 | 0 | RC11 |
| rasterizer_discard | 51 | 51 | 0 | RC3b |
| draw | 50 | 0 | 50 | unclassified |
| attribute_location | 48 | 0 | 48 | unclassified |
| vertex_arrays | 41 | 0 | 41 | unclassified |
| negative_api | 31 | 0 | 31 | unclassified (item gl-core-misc) |
| uniform_api | 27 | 0 | 27 | unclassified |
| pbo | 26 | 0 | 26 | unclassified (item gl-core-misc) |
| read_pixels | 24 | 0 | 24 | unclassified (item gl-core-misc) |
| rasterization | 21 | 0 | 21 | unclassified |
| default_vertex_attrib | 16 | 16 | 0 | RC12 |
| implementation_limits | 16 | 0 | 16 | unclassified (item gl-core-misc) |
| multisample | 16 | 0 | 16 | unclassified (MSAA, no item) |
| instanced | 6 | 0 | 6 | unclassified |
| **total** | **8,835** | **5,854** | **2,981** | |

Check: 1,483 + 1,378 + 703 + 1,106 + 531 + 461 + 63 + 62 + 51 + 16 = **5,854**;
8,835 − 5,854 = **2,981**; and the unclassified column adds up to the same
(748 + 1,234 + 413 + 5 + 88 + 187 + 50 + 48 + 41 + 31 + 27 + 26 + 24 + 21 + 16 +
16 + 6 = 2,981). So **at least 66.3 % (5,854 of 8,835) of the failures have a
named root cause; 2,981 (33.7 %) are unclassified.** The classified column uses
the *lower* bound of each range, so the unclassified column is an upper bound.

The per-image tallies of RESULTS.md (the image table after line 1520) are **not** used: their
fail figures do not reconcile with the group table (image 2 matches 1,905
mustpass names but reports 1,956 results; for fbo, fragment_ops and
fragment_out the per-image fails exceed the group totals by 38;
the record `tools/cts/log/investigations/es3-fbo-fragops-396d667a.md` §1 records this). The group table is
what the tally script produced and is the truth here. Texture's per-image fails
(images 19a-19g, 20) also add to more than the group's 2,612, for the same
reason (reading; not checked in detail).

NotSupported (2,636) and the 56 lifetime cases are **not** failures and are not in
8,835. They matter because some root causes below turn them into runs (see RC3,
RC13).

## Root causes

Layers: **GL core** = `src/gl/**`; **shaderc** = `src/shaderc/**` (the online
GLSL compiler, front end or a back-end code generator); **PowerVR** =
`src/pvr/**` (AM62P); **V3D** = `src/v3d/**` (Pi 4); **GC7000UL** =
`src/vivante/**` (i.MX8MP). The three back ends are separate code; only the
PowerVR one has been measured in dEQP-GLES3.

### RC1. Integer and ES 3.0 colour render targets (GL_INVALID_ENUM at framebuffer set-up)

| | |
|---|---|
| Layer | GL core (refused the formats) then PowerVR back end (integer fragment outputs through the PBE and their read-back). Same on V3D and GC7000UL, not measured |
| Groups and counts | shaders.builtin_functions 1,461 + builtin_variable 22 = **1,483 exact** (R-msg line 1501); fragment_out **531 exact** (R-msg line 1505: basic 240, array 240, random 51, R-row line 989); fbo **270 exact** (R-msg line 1510: "Got GL_INVALID_ENUM", fbo.blit / color / msaa). Total **2,284 exact** |
| First message | `Failed to set up framebuffer object: glGetError() returned GL_INVALID_ENUM at glsShaderExecUtil.cpp:670`; `After framebuffer setup: ... GL_INVALID_ENUM at es3fFragmentOutputTests.cpp`; `Got GL_INVALID_ENUM (es3fFboTestCase)` |
| Cause | dEQP's shader executor renders every output into integer renderbuffers (R32I .. RGBA32UI); `glRenderbufferStorage` refused every integer format and RGB10_A2 (`src/gl/framebuffer.rs` renderbuffer_storage_args_ok / is_renderable_format). Shown in code (R-msg line 1501). For fragment_out and fbo the same cause is a reading from the message and the case names: the fragment_out bucket of 240 equals the integer names plus the RGB10_A2 names exactly (Appendix B and its record) |
| Host | yes: `tests/es3_int_rt.rs` |
| Fixed by | `int-rt-gl` (GL core, `b9b2ac00`); `int-rt-pvr` (`fc658e64`, merged in `a27d69c0`). `int-rt-v3d`, `int-rt-gc`: not on main |
| Status | **Both halves on main for the PowerVR, and in the `f2a13877` tree**: shaders fail 412 there (2,231 here), fragment_out 595 (619). Left on the PowerVR: an integer attachment beside another, colour masks that keep some of an integer target's channels |

Shaders.derivate has a related bucket: `checkFramebufferStatus == COMPLETE
(es3fShaderDerivateTests)` **103 exact** (R-msg line 1516), "float render
targets". It is not counted in the 1,483 (different message). It is a float
colour target (RGBA32F or RGBA16F), which `GL_EXT_color_buffer_float` gates
(Appendix B: `src/gl/state.rs` lists only
`GL_EXT_color_buffer_half_float`). Layer: GL core. Plan item: **no item yet**
(`int-rt-gl` covers integer targets; whether it made float targets complete is
not checked). Host: yes (a completeness query). These 103 are inside the
748 unclassified shaders failures.

### RC2. ES 3.0 sized texture formats refused at upload; integer and packed-float textures

| | |
|---|---|
| Layer | GL core (upload, storage, conversion), then each back end (sampling the format; isampler/usampler) |
| Groups and counts | texture.specification **736 exact** (R-msg line 1503, R-row line 990). Related, not additive to the 736: "Texture upload failed: GL_INVALID_OPERATION (gluTexture)" **522 exact** (R-msg line 1506) in shaders.texture_functions (isampler/usampler) and texture.filtering / texture.format, which lie in the shaders and texture unclassified remainders |
| First message | `Runtime check failed: 'glGetError() == GL_NO_ERROR' at es3fTextureSpecificationTests.cpp` (e.g. basic_teximage2d.r11f_g11f_b10f_2d, basic_teximage3d.rg8ui_2d_array, basic_texsubimage2d.rgba32ui_cube) |
| Cause | the ES 3.0 internal formats were not accepted. Reading from the case names (R-row line 990) |
| Host | yes: `tests/es3_sized_formats.rs`; design `docs/es3-sized-formats.md` |
| Fixed by | `sized-formats-gl` (GL core, `888af5d0`, `e5c4a180`); `formats-pvr` (`25267d17`, merged in `a27d69c0`: every class but `RGB10A2`, `isampler2D` / `usampler2D`). `formats-v3d`, `formats-gc`: not on main |
| Status | **On main for the PowerVR and in the `f2a13877` tree.** Left on the PowerVR: integer cube / 3D / 2D-array samplers and `RGB10A2` as a class |

Two smaller texture.specification causes are *inside* the 736 (same message;
R-row lines 996-997, so they are not added to it):

* **NPOT mip levels ≥ 1** of 2D-array / 3D / texstorage textures: GL_INVALID_VALUE
  from level 1 up. **20 cases** (bound: the row names the groups, not a count
  of cases beyond "20"; it is a work-list figure, treated as exact for the
  row). Host: yes (57x44x5, "teximage3d level 1 28x22: 0x501"). Fixed by
  `tex3d-levels` (**merged**); the texels of those levels are kept by
  `tex3d-finish` (running). Not re-measured.
* **Depth uploads from a pixel unpack buffer**: teximage2d_depth_pbo and
  teximage3d_depth_pbo, **10 cases** (5 each, R-row line 997). Cause not
  investigated in the row. `tex3d-levels`' title includes "depth uploads from a
  PBO" (merged), so it is expected fixed; `oq-gc` (ready) is to trace it on the
  board. Not re-measured.

### RC3. Transform feedback is not implemented (varyings, capture), and rasterizer discard

| | |
|---|---|
| Layer | GL core (state, varyings, queries, discard) and the three back ends (the capture/write path) |
| Groups and counts | transform_feedback **413 exact** (R-msg line 1507: `glTransformFeedbackVaryings(): GL_INVALID_VALUE at gluShaderProgram.cpp`; `MAX_TRANSFORM_FEEDBACK_* = 0`). Also 421 NotSupported (R-grp line 1489: dEQP reads the limits and skips; not failures) and the **56 lifetime cases with no result**: the group's init links a program with interleaved TF of gl_Position and `TCU_CHECK(isOk())` ends the run (RESULTS.md §1r "Not measured") |
| Cause | shown in code: limits are 0 and the varyings call refuses (`docs/es3-transform-feedback.md` section on what the AM62P cost, citing `es3.rs`) |
| Host | API half: yes, `tests/es3_tf_api.rs`. Capture (the data written to the buffer): no, it needs the back end |
| Fixed by | `tf-api` (`16ac8641`): state, varyings, queries, discard. `tf-capture` (`896ca724`, merged in `c4dce9125`): the GL core runs the vertex shader on the CPU for every captured vertex (design `docs/es3-transform-feedback.md` section 4) |
| Status | **Both on main; the capture is not on a board yet.** On `f2a13877` (API only) transform_feedback fails 640 with 191 NotSupported. On the host, dEQP-GLES3 `transform_feedback` passes 882 / 882 and `lifetime` 56 / 56 (API and buffer verdicts) |

**RC3b. rasterizer_discard (51 exact, R-grp line 1482).** "Fail (no message)"
(part of the 133 bucket, R-msg line 1515): discard is recorded but not
honoured. Nine of every 17 cases are about the **clear**, and `tf-api` ships the
clear part (`docs/es3-transform-feedback.md` lines 42 and 595: "the 51 cases
can pass"). Layer: GL core. Host: yes. Fixed by `tf-api` (**merged**, not
re-measured; the draw-skipping cases need a back end: not checked).

### RC4. Compile failure with an empty info log (48)

| | |
|---|---|
| Layer | shaderc |
| Groups and counts | transform_feedback.interpolation 18 + random_full_array_capture 30 = **48 exact** (R-row lines 991 and 993); these 48 are part of the 461 of RC3, not extra (413 + 48 = 461 = the group's fails) |
| First message | `Compile failed: '' at es3fTransformFeedbackTests.cpp` |
| Cause | a stage returns a refusal with no text. The stage is not identified (R-row line 993: "find which stage returns an error with no text") |
| Host | yes (a compile of the generated shaders; `tools/cts/host-deqp-run.sh transform_feedback` reproduces the verdict) |
| Fixed by | `tf-infolog` (in review) |

### RC5. Shadow samplers: compare look-ups, depth textures

| | |
|---|---|
| Layer | PowerVR back end (the online code generator refuses the forms: "a texture fetch with a projective level or bias, a bias on a ND texture, a shadow ... is not selected yet") and GL core (depth texture formats) |
| Groups and counts | texture.shadow **432 exact**: 288 "Failed to compile shader program at glsTextureTestUtil.cpp" (R-row line 992) + 144 "Texture upload failed: GL_INVALID_ENUM at gluTexture.cpp" (depth formats). R-msg lines 1509 (292 = the 288 + 4 texture.mipmap 3D bias) and 1514 (159 = the 144 + 15 in shaders.texture_functions). The shadow forms still refused at 15dde808 are 29 + 6 in shaders.texture_functions (R-row line 995 area; "counted at a1f88596", a later commit, so a bound for the 396d667a run) |
| Cause | online code generator; and the depth formats at upload (R-row line 992) |
| Host | compile half: yes (a host compile shows the refusal; R-row 992 lists "a host repro of the refusal message" as NEXT, so it was not done when written). Picture half: no |
| Fixed by | `pvr-shadow` (todo) for the PowerVR look-ups and PCF. The depth upload half is GL core: depth client-memory uploads pass since commits eef7a9d2 / e60bb067 (R-row line 997); whether those are in the 396d667a numbers or on integration was not checked |

Not in the 8,835: at 15dde808 the 2D and cube shadow cases with a LINEAR or
mipmap filter were 180 **QualityWarning** (compare after filter; R-row line
994). They are warnings, not failures.

### RC6. 2D-array textures sampled as one tall image (PowerVR)

| | |
|---|---|
| Layer | PowerVR shader back end (the TPU filters and wraps across layers) |
| Groups and counts | texture.filtering.2d_array **210 exact** (R-row line 995; the per-image tally agrees, RESULTS.md line 1560 "0 / 210 / 12"). Also 108 of texture.shadow 2d_array and 2 texture_functions sampler2darrayshadow (R-row line 995): these are **inside** RC5's 432 and the shaders remainder, not additive |
| Cause | shown (design and board step 1 at 80daed91, R-row line 995); `docs/es3-pvr-2darray.md` |
| Host | the storage half: yes. The sampling: no (the TPU is not simulated) |
| Fixed by | `pvr-2darray` (todo). Design `pvr-2darray-design` merged. Related fbo cases: fbo.color.tex2darray (36 names) |

### RC7. sRGB blending (fragment_ops.blend.fbo_srgb)

| | |
|---|---|
| Layer | GL core + each back end (SRGB8_ALPHA8 is stored as plain RGBA8; no encode on write, no decode of the destination for blending) |
| Groups and counts | fragment_ops.blend.fbo_srgb **1,106 to 1,111 (bound)**, derived in the record `tools/cts/log/investigations/es3-fbo-fragops-396d667a.md` §2.2: the "Image compare failed" bucket is 1,293 (R-msg line 1502) and names fbo_srgb, ubo.single_nested_struct* and ubo.random; ubo fails 187 in all (R-grp line 1490), so fbo_srgb ≥ 1,293 − 187 = 1,106; fragment_ops fails 1,111 in all (R-grp line 1469), so fbo_srgb ≤ 1,111 |
| First message | `Image compare failed` |
| Host | no (picture verdict). The format handling half is a GL test |
| Fixed by | `srgb` (`3c433ed2`, merged in `c4dce9125`): the PowerVR encodes on write and blends in linear space; the GC7000 and V3D sample the class and refuse a draw into it by name. Design `docs/es3-srgb.md`. Not on a board yet |

**RC7b. ubo nested-struct and random (≤ 187, bound).** The other part of the
same 1,293 bucket. All of ubo's 187 failures *may* be in it; the split is not
recorded. Layer: unknown (GL core UBO layout or the shader back end; the host
test `tests/es3_ubo.rs` runs the ubo groups through the GL API, and
b0c0fa6b fixed `glGetUniform*` of non-square matrices). Plan item: **no item
yet**. Counted as unclassified.

### RC8. glBlitFramebuffer (was a stub); multisample renderbuffers

| | |
|---|---|
| Layer | GL core (`glBlitFramebuffer` always raises GL_INVALID_OPERATION, `src/gl/framebuffer.rs`; samples > 1 refused, `GL_MAX_SAMPLES` is 1) and the back ends |
| Groups and counts | "Got GL_INVALID_OPERATION (es3fFboTestCase)" **252 exact** in the bucket (R-msg line 1511), in fbo.blit, fbo.color, fbo.msaa; the three names are fbo subgroups only, so all 252 are in fbo's 1,116. Not split between blit and MSAA (Appendix B, ranks 2 and 3). The 16 multisample failures (R-grp line 1474, 48 NotSupported) are a separate **unclassified** group |
| Host | API half: yes. Picture half: no |
| Fixed by | the colour blit (`39b18f9b`, merged in `c4dce9125`; `src/gl/blit.rs`, a CPU copy shared by the three back ends; not on a board yet). Depth and stencil blits (item blit-ds) and multisample renderbuffers: **no fix yet**; Appendix B ranks 2 and 3 |

### RC9. fbo "Compile failed" (sglrGLContext), 181

| | |
|---|---|
| Layer | unknown: shaderc or a back end refusing the fbo test shaders (isampler/usampler, integer outputs, sampler2DArray, sampler3D) |
| Groups and counts | **181 exact** (R-msg line 1513): fbo.color, fbo.depth, fbo.render |
| First message | `Compile failed` (at sglrGLContext) |
| Cause | not split (R-msg: "not split yet"). A reading from the sources (Appendix B, rank 4): the integer samplers (`formats-*`), the array sampler (`pvr-2darray`) or the integer outputs (`int-rt-*`) |
| Host | yes (a compile). Not done |
| Fixed by | no single item: `formats-pvr` / `pvr-2darray` / `int-rt-pvr` are candidates. **Unconfirmed** |

### RC10. state_query (63 exact)

Source: RESULTS.md §1q (line 1415) from the earlier image at 9134800c; the
group total 63 is the same in R-grp (line 1487). All **exact**: transform
feedback 29; an ES 3.0 limit read as float or int64 18; 64-bit uniform limits 6;
compressed texture format list 3; fbo attachment queries 3; renderbuffer queries
3; other (shader.program_binary) 1. Layer: GL core. Host: yes (all of them are
API queries; the §1q text says the board adds no bucket). Fixed by `tf-api`
(merged; the 29 TF queries, **expected**, not re-measured) and `gl-core-misc`
(running; the rest). Commits before 396d667a on `main` fixed some of these on
the host (RESULTS.md work list lines 873-885: state_query 98 → 61 of 946 on the
host), so the 63 of the board may already be lower on integration.

### RC11. Occlusion queries (62 exact)

| | |
|---|---|
| Layer | GL core (query state machine) and each back end (a visibility counter) |
| Groups and counts | occlusion_query **62 exact** (R-grp line 1476; all 62 fail, `docs/es3-occlusion-query.md` line 16) |
| Host | the state machine: yes (with a fake back end); the counter: no |
| Fixed by | built on the three back ends (`2241cd84` GL core, `d9573ab9` PowerVR, `dcac96cb` GC7000, `7526b084` V3D; merged in `452d6c31`); design `docs/es3-occlusion-query.md`. In the `f2a13877` tree the group sits in image 5's unrun tail, so it has no board verdict yet |

### RC12. default_vertex_attrib int/uint current values (16 exact)

default_vertex_attrib **16 exact** (R-grp line 1462). Cause: pure-integer
current attribute values went through a float (the message of commit
a54bbdec, the `vertex-int-attribs` result). Layer: GL core
(`src/gl/vertex.rs`). Host: yes. Fixed by `vertex-int-attribs` (`8b3e7039`):
0 failures on `f2a13877`. Appendix A has the item's whole account. The other vertex groups (vertex_arrays 41, attribute_location
48, draw 50, instanced 6) are **unclassified**: the "Fail (no message)" bucket
(133 exact, R-msg line 1515) names attribute_location.mixed*, instanced,
rasterizer_discard, shaders.derivate and texture_functions without a split;
Appendix A classifies attribute_location, vertex_arrays and instanced case by
case.

### RC13. Shaders-group causes found on the host (PowerVR compiler)

The host corpus run (`docs/es3-shaderc-coverage.md`, 5,596 programs from 23
`data/gles3/shaders/*.test` files, measured on 3f78982e) lists what the online
compiler refuses or wrongly accepts. All counts below are **exact for that
corpus**, not for the AM62P failures: they are *programs* (a `both` case is two),
and the link to the board's 2,231 shaders failures is not established
(the `.test` files are one source of the shaders group; the C++-generated cases
are not in it). They show the compiler is not the large part: 3,436 of 3,461
programs that must build do on PowerVR; 2,128 of 2,135 that must be refused are.

| cause | layer | PowerVR / GC7000UL / V3D programs | fixed by |
|---|---|---:|---|
| Link: interpolation and uniform-block mismatches accepted | shaderc link | 7 / 7 / 7 | `link-iface` (**merged**; not re-measured) |
| Backslash-newline inside a token | shaderc preprocessor | 4 / 4 / 4 | `pp-splice` (**merged**; not re-measured) |
| `centroid in` fragment input | shaderc PowerVR and V3D | 9 / 0 / 9 | `pvr-centroid` (running) for PowerVR; V3D: no item |
| Indexed element passed as out/inout argument | shaderc PowerVR and V3D | 8 / 0 / 8 | no item yet (V3D: `v3d-shaderc-gaps` result on a branch, not on the tip) |
| `gl_MaxDrawBuffers` is 1 (two `constant_expressions.other.complex` programs) | GL core / limits | 2 / 2 / 2 | `mrt4` (todo) |
| Reflection name of 63+ characters | shaderc/GL core | 2 / 2 / 2 | no item yet |
| Matrix vertex attribute | V3D only | 0 / 0 / 132 | `v3d-shaderc-gaps` (done, result on branch) |
| `Ishl`, `Ftrunc` | V3D only | 0 / 0 / 2 (Ishl; Ftrunc: count not recorded) | `v3d-shaderc-gaps` |
| Large constant array register pressure | GC7000UL | 0 / 2 / 0 | no item yet |

The board's R-msg compile buckets that these belong to (not split): "Failed to
compile shader program (glsShaderRenderCase)" **189 exact** (shaders.operator
trunc, shaders.texture_functions sampler3D bias; R-msg line 1512), whose
sampler3D-bias part (and the 4 texture.mipmap 3D-bias cases of the 292 bucket)
is the same "bias on a ND texture" refusal as RC5.

### RC14. MRT: only one draw buffer

When this was written `gl_MaxDrawBuffers` and `GL_MAX_DRAW_BUFFERS` were 1 in the
ES 3.0 build. They are 3 now on every GPU (`src/gl/context.rs` `MAX_DRAW_BUFFERS`
= `CAP_COLOR_ATTACHMENTS`); ES 3.0 asks for 4. Which of the board's
failures it causes is **not recorded**: the NotSupported groups
draw_buffers_indexed (76) and part of fragment_out (115) show the limit; the
4-target seeds of fragment_out.random are inside its 51+ integer failures. The
host proves two programs (RC13 table). Count: **no source**. Layer: GL core and
all three back ends. Fixed by `mrt4` (not on main).

## Unclassified (2,981)

These failures belong to a group but to no root cause above. **No count is
estimated** for how they split.

| group | unclassified (≤) | what the repository says |
|---|---:|---|
| texture | 1,234 | 2,612 − (736 + 432 + 210). Buckets that name texture groups: "Image verification failed" 298 (texture.filtering, mipmap), "Image comparison failed" 675 (shared), "Texture upload failed: INVALID_OPERATION" 522 (shared), 4 mipmap 3D-bias compile failures. 3D filtering (texture.filtering.3d): image tallies show most of its 153 names failing, probably the 3D levels (`tex3d-levels`, `tex3d-finish`) and 3D formats |
| shaders | 748 | 2,231 − 1,483. Includes derivate 103 (RC1 note), the 189 compile bucket (part), isampler/usampler texture_functions (part of 522), shadow forms 35 (RC5), linkage/preprocessor programs of RC13, shaders.random image failures |
| fbo | 413 | 1,116 − 703. In "Image comparison failed" 675 (shared), in the unlisted 62 buckets, or both |
| ubo | 187 | RC7b |
| fragment_out | 88 | 619 − 531 |
| draw 50, attribute_location 48, vertex_arrays 41 | 139 | vertex groups; attribute_location and vertex_arrays are classified in Appendix A (tasks T2 to T4); draw is 14 on `f2a13877` |
| negative_api 31, uniform_api 27, pbo 26, read_pixels 24, rasterization 21, implementation_limits 16, multisample 16, instanced 6 | 167 | `gl-core-misc` (running) takes negative_api, pbo, read_pixels, implementation_limits; `es3-rest-deqp-gap` (running) maps the rest; multisample is MSAA (RC8) with no item |
| fragment_ops | ≤ 5 | 1,111 − 1,106; in blend.default_framebuffer, scissor or stencil |

Check: 1,234 + 748 + 413 + 187 + 88 + 139 + 167 + 5 = **2,981**.

## Plan items against the causes

The table as it stood at `396d667a`, with each item's state on main
(2026-10-11):

| item | state on main | root causes | what remains |
|---|---|---|---|
| int-rt-gl | `b9b2ac00` | RC1 | |
| int-rt-pvr | `fc658e64` (`a27d69c0`) | RC1 | an integer attachment beside another; partial colour masks |
| int-rt-v3d / -gc | not on main | RC1 | all |
| sized-formats-gl | `888af5d0`, `e5c4a180` | RC2 | |
| formats-pvr | `25267d17` (`a27d69c0`) | RC2, part of RC9 | integer cube / 3D / 2D-array samplers, `RGB10A2` |
| formats-v3d / -gc | not on main | RC2 | all |
| tex3d-levels | `c40414b9` | RC2 (20 + 10) | |
| link-iface | `b88fea5e`, `03ff7a63` | RC13 (7) | |
| pp-splice | `79fe1718` | RC13 (4) | |
| tf-api | `16ac8641` | RC3 (API), RC3b, RC10 | |
| tf-capture | `896ca724` (`c4dce9125`) | RC3 (413) | the board; a vertex shader that samples a texture |
| tf-infolog | not found on main | RC4 | the empty info log (48) |
| mrt4 | not on main | RC14 | `GL_MAX_DRAW_BUFFERS` 4 |
| pvr-2darray | not on main (design `docs/es3-pvr-2darray.md`) | RC6 | all |
| pvr-shadow | the shadow forms compile (`a27d69c0`) | RC5 | the board's verdict |
| occlusion queries | `452d6c31` | RC11 | the board |
| gl-core-misc | not found under that name | RC10 (rest), negative_api, pbo, read_pixels, implementation_limits | read_pixels 0 and state_query 26 on `f2a13877` |
| srgb | `3c433ed2` (`c4dce9125`) | RC7 | the board; the GC7000's and V3D's sRGB targets |
| blit (colour) | `39b18f9b` (`c4dce9125`) | RC8 (part) | the board |
| blit-ds, MSAA | none | RC8 (rest) | all |
| pvr-centroid | `e5874527` | RC13 (9) | |
| vertex-int-attribs | `8b3e7039` | RC12 | Appendix A tasks T1 to T7 |
| **no item yet** | | RC7b (ubo ≤ 187), the float-target bucket (103), indexed out/inout arguments, long reflection names, multisample 16, uniform_api, rasterization 21, and the unclassified remainder | |

## What is still to measure

* A per-case list of a whole tree. The `f2a13877` tree's records are on the
  board runner (`records/am62p/deqp-gles3-f2a13877`); none of its per-case
  results is written down in the repository, so the unclassified rows stay
  unsplit.
* The `c4dce9125` rows on a board (transform feedback, blit, buffer and PBO,
  sRGB): `docs/HANDOFF.md`, "Next board session".
* The i.MX8MP and the Pi 4 have never run dEQP-GLES3 (the Pi 4 recipe is
  `docs/es3-board-recipe-pi4.md`).
* The shaders group's failure map on `396d667a` is the record
  `tools/cts/log/investigations/es3-shaders-396d667a.md`.
* Documents this file once expected, which were never written:
  `docs/es3-rest-failure-map.md`,
  `docs/es3-texture-failure-map.md`, `docs/es3-host-run.md`. The ES 3.0
  limits document `docs/es3-backend-caps.md` was reverted (`3f78982`).

## Assumptions made writing this

* "Exact" includes sums and differences of cited numbers; a range whose ends
  come from cited numbers is a "bound".
* Buckets are taken as disjoint (the 78 buckets sum to 8,835, RESULTS.md line
  1496), so RC1's three buckets are added. Overlaps are stated where a source
  counts the same cases twice (RC2, RC5/RC6, RC4/RC3).
* The 20 NPOT-level and 10 depth-PBO cases are taken as part of the 736
  (same message and same subgroup in R-row lines 996-997); this is a reading.
* `texture.shadow` is counted once, in RC5; the 2D-array cases in it are not
  added to RC6.
* The GC7000UL and V3D back ends are named only where a document in the
  repository does so; no GLES3 board result exists for them.

## Appendix A: the vertex groups

Folded from `docs/es3-vertex-gap.md` (item vertex-int-attribs, written on the
integration tip `2f69c7d1`; its line numbers are that tip's). It classifies
every dEQP-GLES3 failure of the vertex-attribute groups in the AM62P run on
`396d667a`, from that run's per-case UART records (gitignored, in the owner's
checkout under `records/am62p/`: `record_G3-img1-396d667a_*`, `img5`, `img18`,
`img25`); every count marked *exact* is a count of their `deqp:` lines. dEQP
source: VK-GL-CTS `93bca01`.

**Status (2026-10-11).** The item's fix is on main as `8b3e7039`:
`default_vertex_attrib` fails 16 on `c4ccc966` (before it) and 0 on
`f2a13877`. Tasks T2 to T6 below are open, and the board agrees: on
`f2a13877` `attribute_location` fails 48 (28 + 20 here), `vertex_arrays` 41
(20 + 21) and `instanced` 6 (`tools/cts/RESULTS.md`). The i.MX8MP and the Pi 4
columns are still predictions from the code.

A *current value* is what a vertex input reads while its array is disabled
(`glVertexAttrib*`); a *pure-integer* current value is one set with
`glVertexAttribI4*`; a *packed type* is `GL_INT_2_10_10_10_REV` or
`GL_UNSIGNED_INT_2_10_10_10_REV`.

### The counts and causes

| Subgroup (dEQP-GLES3.functional.) | Mustpass cases | AM62P fails | Root cause | Fixed by |
|---|---:|---|---|---|
| vertex_arrays.single_attribute.first.int2_10_10_10, .offset.int2_10_10_10, .strides.int2_10_10_10.buffer_* | 8 + 8 + 4 | **20, exact** (all of them; img25) | A packed array in a buffer is fetched in place as float32 words: `src/gl/draw.rs:818` `map_attrib_format` maps the packed types (and `GL_HALF_FLOAT`) to `GLES_ATTRIBF_FLOAT32`, so `draw.rs:3253` takes the array as native and never reaches `convert_attrib`; `draw.rs:3272` also sizes one element as components x 4 bytes. Client arrays (the `user_ptr` strides cases, normalize, output_types: 22 cases) are converted and pass. | not covered (GL core draw path); see task T2 |
| vertex_arrays.single_attribute.output_types.{byte,short,int}.components{2,3}_ivec4_*, .{unsigned_byte,unsigned_short,unsigned_int}.components{2,3}_uvec4_* | 24 | **21, exact** (img25; the 3 passing are unsigned components2 quads1) | An integer array narrower than its ivec4/uvec4 input gets its missing w filled with the float 1.0 (`draw.rs:868`, `attrib_row`): bits 0x3f800000, which the input reads as 1065353216. ES 3.0 2.8 fills (0, 0, 0, 1) in the input's type. | not covered (GL core draw path); task T3 |
| vertex_arrays, everything else | 920 | 0, exact | | |
| attribute_location.mixed.*, .mixed_time.{pre_attach,pre_link,reattach,relink} | 24 + 5 | **28, exact** (img1; mixed_time.post_link passes) | `glBindAttribLocation` is applied to an attribute that has `layout(location = N)`: `src/gl/program.rs:1545-1549` (`build_attrib_indices`) remaps the shader's location to the bound index whenever the name was bound. ES 3.0 2.12.3 and GLSL ES 3.00 4.3.8.1: the qualifier wins and the binding is ignored. dEQP binds `a_0` to 4 while the shader says 3 and expects 3; post_link passes because a binding made after the link has no effect until the next link. | not covered (GL core link); task T4 |
| attribute_location.mixed_max_attributes.* | 24 | **20, exact** (img1; mat3, mat3x2, mat3x3, mat3x4 pass) | The compiler places an `in` with no qualifier at the counter continuing past the previous explicit location (`src/shaderc/online/reflect.rs:471-485`). dEQP alternates `layout(location = L)` and bound attributes from the top location down, so for vec4 `a_0` (unqualified) takes 0 and `a_15` is `layout(location = 0)`: two inputs on one location, and `program.rs:1562` writes `attrib_app_index[0]` twice, so the bound name answers 0 instead of 15. The mat3 shapes fill 15 of 16 locations and never collide. ES 3.0 2.12.3: unqualified, unbound attributes get locations the linker chooses among those no qualifier uses. | not covered (shaderc reflection + GL core link); task T4 |
| attribute_location, everything else (bind*, layout*, mixed_hole, mixed_relink_hole, bind_time, mixed_time.post_link) | 221 | 0, exact | | |
| default_vertex_attrib.{int,ivec2,ivec3,ivec4}.vertex_attribi_4i{,v}, .{uint,uvec2,uvec3,uvec4}.vertex_attribi_4ui{,v} | 16 | **16, exact** (img1) | `glVertexAttribI4i`/`I4ui` stored the value converted to a float (`src/gl/vertex.rs`, the old lines 312-321), and the draw stages the current value's dwords as they are (`draw.rs:3208`), so an ivec4/uvec4 input read a float's bit pattern. | **this item**: the pure-integer value is stored as its bits (`vertex.rs:361`, `set_current_bits`) |
| default_vertex_attrib, float and matrix cases | 64 | 0, exact | | |
| instanced.{draw_arrays_instanced,draw_elements_instanced}.mixed.{2,4,20}_instances | 6 | **6, exact** (img5) | The mixed cases read `gl_InstanceID` and use divisors 2 and 1 (es3fInstancedRenderingTests.cpp:55-57, 598-602). The PowerVR takes divisor 1 only (`src/pvr/backend.rs:6630`, `max_instance_divisor`), so `draw.rs:4270` emulates the draw instance by instance, and `pvr/backend.rs:3927` refuses an emulated instance whose program reads `gl_InstanceID` (the PDS counts instances from 0 in every draw). i.MX8MP: the same refusal (`src/vivante/backend.rs:2167`, divisor 1 at `:2690`), so the same 6 are predicted to fail. Pi 4: the V3D takes any divisor (`src/v3d/backend.rs:4645`, 0xffff) and the draw stays on the GPU, predicted to pass. | back-end item, not covered; task T6 |
| instanced, everything else | 39 | 0, exact | | |
| negative_api.vertex_array.{draw_elements, draw_elements_incomplete_primitive, draw_elements_instanced, draw_elements_instanced_incomplete_primitive, draw_range_elements, draw_range_elements_incomplete_primitive} | 6 | **6, exact** (img5) | Each case ends with a transform-feedback section (es3fNegativeVertexArrayApiTests.cpp:430-462): it links a program with a captured varying, begins a capture and expects GL_NO_ERROR, then expects GL_INVALID_OPERATION from an indexed draw while the capture is active. At 396d667a transform feedback did not exist (every MAX_TRANSFORM_FEEDBACK_* was 0 and Begin was an error). | tf-api (merged): `tests/es3_tf_api.rs` `capture_is_refused_loudly` holds exactly this order; confirm on the board (board-am62p) |
| negative_api.vertex_array.{vertex_attrib_pointer, vertex_attrib_i_pointer, vertex_attrib_divisor, vertex_attribf, vertex_attribfv, vertex_attribi4, vertex_attribi4v, enable_vertex_attrib_array, disable_vertex_attrib_array, gen_vertex_arrays, bind_vertex_array, delete_vertex_arrays, draw_arrays*, draw_elements_invalid_program, draw_elements_instanced_invalid_program, draw_range_elements_invalid_program}, negative_api.state.{get_vertex_attribfv, get_vertex_attribiv, get_vertex_attribi_iv, get_vertex_attribi_uiv, get_vertex_attrib_pointerv, is_vertex_array} | 27 | 0, exact | | |
| state_query.shader.vertex_attrib_{size,type,stride,normalized,integer,enabled,divisor,array_buffer_binding,pointerv}, .current_vertex_attrib_{initial,float,int,uint,float_to_int}, state_query.integers.max_vertex_attribs_*, .vertex_array_binding_* | 21 | 0, exact (img18) | `current_vertex_attrib_float_to_int` accepts any integer from floor(x) to ceil(x) (es3fShaderStateQueryTests.cpp:62-74, 203-223), so the truncation the code did passed; the code now rounds to nearest as ES 3.0 6.1.2 says. `current_vertex_attrib_int`/`uint` use values below 2^24, which also round-tripped through the old float storage. | this item (rounding) |
| vertex_array_objects.*, default_vertex_array_object.* | 11 | 0, exact | | |

The other failures of negative_api (25) and state_query (63) in those records
are transform feedback, queries, textures, framebuffers and limits, outside
this item (the root causes above).

Sum of the vertex-related failures: 20 + 21 + 28 + 20 + 16 + 6 + 6 = 117. Of
them 16 are fixed here, 6 wait for a board confirmation of tf-api, and 95
need files this item does not own (tasks T2-T6 below).

### What this item changed in `src/gl/vertex.rs`

1. **Pure-integer current values are exact.** `glVertexAttribI4{i,ui}{,v}`
   store the four 32-bit values as they are, in the float slots of
   `GlContext::current_attrib` (`set_current_bits`). That is the convention
   the draw already uses for integer arrays (`draw.rs` `attrib_component`
   widens an integer to 32 bits "whose bits travel in the float slot") and
   for the per-instance values it writes into `current_attrib` itself
   (`draw.rs:4190` `instance_attrib_value`). The draw stages the dwords
   unchanged (`draw.rs:3208`), so an ivec4/uvec4 input now reads i32::MIN to
   u32::MAX exactly. `glGetVertexAttribI{i,ui}v(GL_CURRENT_VERTEX_ATTRIB)`
   return those bits. Fixes the 16 default_vertex_attrib cases (to be
   confirmed on the board).
2. **`glGetVertexAttribiv(GL_CURRENT_VERTEX_ATTRIB)` rounds** the float value
   to the nearest integer, halves away from zero, saturating (ES 3.0 6.1.2).
   It truncated before.
3. **The ES 2.0 report answers ES 2.0.** With `report-es2`,
   `glVertexAttribPointer` says GL_INVALID_ENUM for `GL_HALF_FLOAT` and the
   two packed types (ES 2.0 has none of them; `GL_OES_vertex_half_float` and
   `GL_OES_vertex_type_10_10_10_2` are not claimed, and `GL_HALF_FLOAT_OES`
   0x8D61 was never accepted), and the four getters say GL_INVALID_ENUM for
   `GL_VERTEX_ATTRIB_ARRAY_INTEGER` and `GL_VERTEX_ATTRIB_ARRAY_DIVISOR` (no
   instanced-arrays extension is claimed). Before, both builds accepted them.
   The ES 3.0 entry points (`glVertexAttribIPointer`, `glVertexAttribI4*`,
   `glGetVertexAttribI*`, `glVertexAttribDivisor`) stay exported and
   functional in the ES 2.0 build, as every other ES 3.0 entry point does
   there (`src/gl/egl.rs` has no per-report filter and `src/gl/es3.rs` gates
   behaviour, not symbols); hiding them would need `egl.rs` and `capi/`.
   dEQP-GLES2 never uses these types or pnames (its negative state cases pass
   pname -1), so the ES 2.0 CTS answer does not move.

Not changed, because the code was already right (held by
`tests/es3_vertex_api.rs`): index, size and stride checks (GL_INVALID_VALUE),
unknown types (GL_INVALID_ENUM), `glVertexAttribIPointer` taking the six
integer types only, packed types with size other than 4
(GL_INVALID_OPERATION), a client pointer with a vertex array object bound
(GL_INVALID_OPERATION), `glVertexAttribDivisor` (index check only; any
divisor value is legal), the stored integer flag and divisor, and the getters'
GL_INVALID_ENUM for unknown pnames.

**A limit of the fix.** The context records a current value's dwords but not
whether a float or an integer command set them (`GlContext::current_attrib` is
`[[GLfloat; 4]; 16]`, `src/gl/context.rs:821`; that file belongs to item
gl-core-misc). ES 3.0 2.7 and 6.1.14 leave the mixed reads undefined (a float
input or `glGetVertexAttribfv/iv` on a pure-integer value, an integer input or
`glGetVertexAttribI*v` on a float value, the initial (0, 0, 0, 1) included),
and here they return the stored bits reinterpreted. Mesa converts instead.
dEQP exercises only the defined pairs. Task T1 adds the type tag.

### Back-end support still missing

- **PowerVR, GC7000UL: divisors above 1 together with `gl_InstanceID`.**
  Both back ends take divisor 1 only; the GL core emulates other divisors
  one instance per draw and the back ends then refuse a program that reads
  `gl_InstanceID` (`pvr/backend.rs:3927`, `vivante/backend.rs:2167`). The
  6 instanced.mixed cases need either the vertex fetch stepping by the
  divisor (Mesa pvr programs the PDS instance step; etnaviv has no divisor
  on GC7000 and emulates in the driver) or `gl_InstanceID` fed as a per-draw
  constant in the emulated path. Not in the plan: task T6.
- **GC7000UL: integer arrays in a buffer.** The Vivante back end fetches
  byte, short and fixed arrays in place (`vivante/backend.rs:2646`,
  `native_attrib_formats`), and `draw.rs:3253` does not look at
  `GlVertexAttrib::integer` before choosing the in-place fetch, so a
  `glVertexAttribIPointer` byte or short array in a buffer would be fetched
  as a float array on the i.MX8MP. The PowerVR (`native_attrib_formats` 0)
  and the V3D (float32 only) convert such arrays on the CPU, so the AM62P
  passed the integer `output_types.*.components4_*` cases. Untested on the
  i.MX8MP (no GLES3 run yet); task T2 covers the check.
- **Nothing is missing for the integer current values**: every back end
  receives the staged 16 bytes and fetches them with stride 0.

### Tasks for later items

- **T1, typed current value** (`src/gl/context.rs`, then `vertex.rs`): keep
  `current_attrib` for the ABI (`src/abi_layout.rs:592` pins its offset) and
  add a per-attribute tag (float, int, uint) reset with the other attribute
  state at `context.rs:1462`; `glGetVertexAttribfv/iv` then convert a
  pure-integer value and `glGetVertexAttribI*v` convert a float one, as Mesa
  does. The draw does not need the tag.
- **T2, ES 3.0 array formats in a buffer** (`src/gl/draw.rs`):
  `map_attrib_format` (`draw.rs:818`) must not call the packed types and
  `GL_HALF_FLOAT` float32; they, and any array with `integer` set on a back
  end whose `native_attrib_formats` covers the type, must go through
  `convert_attrib`. The element size at `draw.rs:3272` must be
  `attrib_elem_bytes`. Fixes the 20 vertex_arrays packed cases on the AM62P
  and the i.MX8MP integer-array hazard above.
- **T3, the integer fill** (`draw.rs:868`, `attrib_row`): the w of a
  narrower integer array is the integer 1, not the float 1.0. Fixes 21
  vertex_arrays.output_types cases.
- **T4, layout locations at link** (`src/gl/program.rs`,
  `src/shaderc/online/reflect.rs`, `src/shaderc/online/codegen.rs:979`,
  `program.rs:1353`): (a) `build_attrib_indices` must ignore a binding for an
  attribute with an explicit layout location; (b) unqualified attributes must
  be placed at link among the locations no qualifier uses (the reflection's
  running counter at `reflect.rs:471-485` collides with a later qualifier,
  and the deferred `layout_at_link` record drops the declared location at
  `codegen.rs:979` / `program.rs:1353`). Fixes 48 attribute_location cases.
  These verdicts are API-level and reproducible on the host with the command
  below.
- **T5, confirm tf-api on the board** (board-am62p): the 6
  negative_api.vertex_array indexed-draw cases.
- **T6, instancing with any divisor on the PowerVR and the GC7000UL** (back
  ends): the 6 instanced.mixed cases; also predicted on the i.MX8MP.
- **T7, the i.MX8MP and Pi 4 columns**: the first dEQP-GLES3 run of each
  (board-imx8mp, board-pi4) turns the predictions above into counts.

### Host measurement to take (HOST-ONLY: API verdicts only)

The run needs the VK-GL-CTS checkout of `tools/cts/README.md` section 2b
(`<VK-GL-CTS>`) and a working `cargo` and `cmake`:

```sh
export CTS="<VK-GL-CTS>"
for g in powervr vivante; do
  for f in attribute_location. negative_api.vertex_array negative_api.state.get_vertex_attrib \
           state_query.shader.vertex_attrib state_query.shader.current_vertex_attrib; do
    GPU=$g MODULE=gles3 OUT=build/host-deqp/vtx-$g-${f//[.\/]/_} \
      tools/cts/host-deqp-run.sh "dEQP-GLES3.functional.$f"
  done
done
```

Expected on the integration tip plus this item: attribute_location mixed*
48 fails until T4; negative_api.vertex_array and negative_api.state
get_vertex_attrib* 0 fails; state_query.shader vertex_attrib* and
current_vertex_attrib* 0 fails. default_vertex_attrib, instanced and
vertex_arrays are render verdicts and mean nothing on the host.

### ES 2.0 no-regression

The ES 2.0 build (`report-es2`) changes only in what it refuses: the ES 3.0
array types and pnames above. dEQP-GLES2 does not use them, so its
state_query (396/396) and negative_api (237 pass, 9 not supported) rows are
predicted unchanged. `tests/es3_vertex_api.rs` runs its argument,
current-value and getter tests in the ES 2.0 build too, and
`es2_build_refuses_the_es3_types_and_pnames` pins the refusals; the unit
tests in `vertex.rs` branch on `crate::REPORT_ES2` the same way.
`glGetVertexAttribiv(GL_CURRENT_VERTEX_ATTRIB)` now rounds instead of
truncating in both builds; dEQP-GLES2's conversion case accepts both.

## Appendix B: fbo, fragment_ops and fragment_out by subgroup

Folded from `docs/es3-fbo-fragops-failure-map.md` (written 2026-10-08,
`53c7ba34`). The case counts are mustpass names; the failure counts are the
`396d667a` run's unless a row names `c4ccc966`. How each count was derived,
and the figures that do not reconcile, are in the record
`tools/cts/log/investigations/es3-fbo-fragops-396d667a.md`. On `f2a13877`
the groups read fbo 872, fragment_ops 1,109, fragment_out 595 failures
(`tools/cts/RESULTS.md`); no per-subgroup split of that run is written down.
"Plan item" names the item expected to fix the subgroup. Format classes:
**I** = the 19 integer formats, **A2** = rgb10_a2, **F32** = the 4 formats
that need `GL_EXT_color_buffer_float` (not listed, so dEQP reports
NotSupported), **F16** = r16f rg16f rgba16f, **ok** = the 8 ES 2.0-class
formats. `FB` = `es3fFboTestCase.cpp`, `FBU` = `es3fFboTestUtil.cpp`.

### fbo (2,028 cases)

| subgroup | cases | AM62P fails | bucket (R-bucket) | dEQP source | needs | plan item |
|---|---:|---|---|---|---|---|
| fbo.api | 20 | not recorded for 396d667a (an earlier GLES3 image passed 20 / 20, `RESULTS.md:1408`; that is not the §1r run) | – | `es3fFboApiTests.cpp` | – | – |
| fbo.blit.conversion | 436 | 324 at `c4ccc966` (`records/am62p/deqp-gles3-c4ccc966`): "Got GL_INVALID_OPERATION" 302 (the stub), "Got GL_INVALID_ENUM" 23 (an RGB10_A2 rbo, taken since `468528a`); 112 n/s (F32 / R11F_G11F_B10F) | – | `es3fFramebufferBlitTests.cpp` (blit between two renderbuffers of different formats: 255 float/normalised pairs incl. F32, 81 signed-int, 100 unsigned-int pairs) | format conversion int→int / float→float, integer rbo for 181 cases, sRGB8_ALPHA8 as source and destination (32 names) | **built** (`c4dce9125`, host only; blit: `src/gl/blit.rs`, a CPU copy with the conversions of ES 3.0.6 4.3.3). The 32 `srgb8_alpha8` names: the sRGB class decodes the read buffer and encodes the draw buffer there (`decode_float` / `encode_float`, item srgb, `docs/es3-srgb.md` 3.2) |
| fbo.blit.default_framebuffer | 106 | 84 at `c4ccc966`: "GL_INVALID_OPERATION" (the stub) 84; 22 n/s | – | same file | `glBlitFramebuffer` to/from the window surface (stored top-down), LINEAR scale, out-of-bounds | **built** (`c4dce9125`, host only; blit) |
| fbo.blit.depth_stencil | 28 | 16 at `c4ccc966`: "GL_INVALID_OPERATION" 16; 12 n/s (separate depth and stencil) | – | same | blit of D16, D24, D32F, D24S8, D32F_S8, S8 (depth-only, stencil-only, scaled) | **none**: the blit validates them and refuses the copy with GL_INVALID_OPERATION; it needs a depth/stencil copy in each back end (the planes live in the back ends' own buffers, `src/ds_retention.rs`) |
| fbo.blit.rect | 40 | 40 at `c4ccc966`: "GL_INVALID_OPERATION" (the stub) | – | same | blit with reversed rectangles, scaling, clamping, NEAREST / LINEAR, NEAREST consistency | **built** (`c4dce9125`, host only; blit) |
| fbo.color.blend | 14 | not recorded | INVALID_ENUM 270 (A2), Compile failed 181, Image comparison failed 675 | `es3fFboColorbufferTests.cpp` (blend on a colour rbo) | A2 (1), F32 (1), sRGB blend in linear space (1), others ok | int-rt-gl (A2 rbo); sRGB part: srgb (built, `c4dce9125`, host only) |
| fbo.color.clear | 36 | not recorded | INVALID_ENUM 270 for I (19) + A2 (1); F32 (4) n/s | same | `glClearBufferiv/uiv` and integer `glReadPixels` for I; classes above | int-rt-gl, int-rt-pvr / -v3d / -gc |
| fbo.color.repeated_clear | 108 | `blit` 64 at `c4ccc966` (62 "Got GL_INVALID_OPERATION", the stub; rgb10_a2 INVALID_ENUM, rgb16f tex2d incomplete), 8 n/s | Image comparison failed 675 | same | `blit` 72 (clear, blit to a second fbo), `sample` 36 (clear, sample as a texture) | blit part **built** (`c4dce9125`, host only; blit; the `tex2d` integer names also need integer textures as render targets); sample part not known |
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
| fbo.invalidate.sub | 19 | 8 at `c4ccc966`: `unbind_blit*` 4 ("GL_INVALID_OPERATION", the stub), `unbind_blit_msaa*` 4 (incomplete: no multisample storage) | – | same | `glInvalidateSubFramebuffer`, blit | blit part **built** (`c4dce9125`, host only); MSAA part **none** |
| fbo.invalidate.target | 27 | not recorded | – | same | all `GL_FRAMEBUFFER` / `DRAW` / `READ` target combinations, validation | – (reading: pass) |
| fbo.invalidate.whole | 19 | 11 at `c4ccc966`: the same 4 + 4 blit cases, and 3 `unbind_read_color*` "Image comparison failed" | – | same | as `sub` | blit part **built** (`c4dce9125`, host only); MSAA part **none**; `unbind_read_color*` not known |
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

### fragment_ops (3,170 cases)

| subgroup | cases | AM62P fails | bucket | dEQP source | needs | plan item |
|---|---:|---|---|---|---|---|
| fragment_ops.blend.default_framebuffer.equation_src_func_dst_func | 677 | not recorded; the 5 failures that the record puts outside fbo_srgb sit in blend.default_framebuffer, scissor or stencil | – | `es3fBlendTests.cpp` | blend factors on the window surface; MIN / MAX are accepted (`src/gl/state.rs:637`) | – (reading: pass; ES 2.0 blend 1,060 / 1,060 pass, `STATUS.md:165`) |
| …default_framebuffer.rgb_equation_alpha_equation | 25 | as above | – | same | ES 3.0 MIN / MAX separate RGB / alpha equations | – (reading: pass; **MIN / MAX in the three back ends not checked here**) |
| …default_framebuffer.rgb_func_alpha_func | 450 | as above | – | same | | – |
| fragment_ops.blend.fbo_srgb.equation_src_func_dst_func | 677 | **1,106 to 1,111 over the three fbo_srgb rows** (derived in the record named above) | Image compare failed 1,293 | `es3fBlendTests.cpp:199-206` (`GL_SRGB8_ALPHA8` rbo of 2× viewport), `:316-321` (reference converted with `sRGBToLinear`) | blend in an SRGB8_ALPHA8 colour buffer: the destination is decoded, the blend result encoded | srgb (built, `c4dce9125`, host only) |
| …fbo_srgb.rgb_equation_alpha_equation | 25 | (in the 1,106-1,111) | same | same | | srgb (built, `c4dce9125`, host only) |
| …fbo_srgb.rgb_func_alpha_func | 450 | (in the 1,106-1,111) | same | same | | srgb (built, `c4dce9125`, host only) |
| fragment_ops.depth | 8 | 0 if the image-4 reading holds (below) | – | `es3fDepthTests.cpp` | `cmp_*` ×8 | – |
| fragment_ops.depth_stencil | 621 | same | – | `es3fDepthStencilTests.cpp` (random 25, stencil_depth_funcs 81, stencil_ops 512, write_mask 3) | | – |
| fragment_ops.interaction | 94 | same | – | `es3fFragOpInteractionTests.cpp` (basic_shader) | | – |
| fragment_ops.random | 100 | same | – | `es3fRandomFragmentOpTests.cpp` | | – |
| fragment_ops.scissor | 26 | `framebuffer_blit_*` 3 at `c4ccc966` ("GL_INVALID_OPERATION", the stub); the rest not recorded | – | `es3fScissorTests.cpp` (clear_int_buffer and clear_uint_buffer are 2 of the names; blit ×3 `framebuffer_blit_*`) | `glClearBufferiv/uiv` on an integer buffer (only the `clear_*_buffer` names), `glBlitFramebuffer` into the window under a scissor box for the 3 blit names | `framebuffer_blit_*` (3): **built** (`c4dce9125`, host only; the scissor box cuts the blit's destination); the rest – |
| fragment_ops.stencil | 17 | not recorded (≤ 5 with the scissor row) | – | `es3fStencilTests.cpp` | | – |
| **fragment_ops total** | **3,170** | **1,111** | | | | |

### fragment_out (940 cases)

| subgroup | cases | AM62P fails | bucket | dEQP source | needs | plan item |
|---|---:|---|---|---|---|---|
| fragment_out.basic.float | 84 | not recorded; fbo setup error counts below | n/s for F32 (rgba32f, r11f_g11f_b10f, rg32f, r32f: 4 formats × 12 = 48); F16 (3 × 12 = 36) pass or image-fail | `es3fFragmentOutputTests.cpp:1062-1112` (`requiredFloatFormats`, 7 formats × 3 precisions × float / vec2 / vec3 / vec4) | – | – (n/s is right while `GL_EXT_color_buffer_float` is unlisted) |
| fragment_out.basic.fixed | 108 | not recorded | INVALID_ENUM 531 for rgb10_a2 (12) | `:1063-1065, :1114-1143` (9 formats: rgba8, srgb8_alpha8, rgb10_a2, rgba4, rgb5_a1, rgb8, rgb565, rg8, r8) | RGB10_A2 rbo (12 names) | int-rt-gl / int-rt-* (ES 3.0 colour render-target formats); sRGB8_ALPHA8 output (12 names) writes encoded: srgb (built, `c4dce9125`, host only) |
| fragment_out.basic.int | 108 | all 108 (derived in the record named above) | INVALID_ENUM 531 | `:1066-1067, :1145-1174` (9 integer formats × 12) | integer rbo, `ivec` fragment outputs, integer `glReadPixels` | int-rt-gl, int-rt-pvr, int-rt-v3d, int-rt-gc |
| fragment_out.basic.uint | 120 | all 120 | INVALID_ENUM 531 | `:1068, :1176-1205` (10 formats incl. RGB10_A2UI × 12) | as int | int-rt-gl, int-rt-pvr / -v3d / -gc |
| fragment_out.array.float | 84 | not recorded | F32 n/s 48 | `:1209-1280` (`numTargets = 3` at `:1215`) | array output `out vec4 o[3]`, 3 draw buffers (met: `CAP_COLOR_ATTACHMENTS` = 3, `src/gl/context.rs:139`) | – |
| fragment_out.array.fixed | 108 | not recorded | INVALID_ENUM for rgb10_a2 (12) | same | 3 draw buffers of one format | int-rt-gl, int-rt-*; sRGB: srgb (built; an sRGB attachment beside another is refused by name) |
| fragment_out.array.int | 108 | all 108 | INVALID_ENUM 531 | `:1282-1314` | as basic.int, with 3 targets | int-rt-gl, int-rt-* |
| fragment_out.array.uint | 120 | all 120 | INVALID_ENUM 531 | `:1316-1346` | as basic.uint | int-rt-gl, int-rt-* |
| fragment_out.random | 100 | not recorded; ≥ 51 | INVALID_ENUM 531 (51 of the bucket) | `:1008-1060`, `:1346-1353` (`createRandomCase(..., 2, 4, seed)`: 2 to 4 targets, random float / int / uint formats) | integer or A2 targets (the 51); cases with 4 targets need `MAX_DRAW_BUFFERS` 4 (3 today, `CAP_COLOR_ATTACHMENTS`, `src/gl/context.rs:139`); some are n/s for F32 | int-rt-gl, int-rt-*, mrt4 (for the 4-target seeds; which seeds have four is not recorded) |
| **fragment_out total** | **940** | **619** | | | | |

### Ranked: what no other item covers

Ranked by AM62P failures at `396d667a` (rank 1 now has an item).

| rank | subgroup(s) | fails | suspected cause (code evidence) | needs |
|---:|---|---|---|---|
| 1 | fragment_ops.blend.fbo_srgb.* (1,152 cases) | **1,106 – 1,111** at `396d667a` (derived, the record); fragment_ops fails 1,109 on `f2a13877`, before the sRGB work landed | **built** since `c4dce9125` (host only): `SRGB8_ALPHA8` is its own class, the PowerVR's PBE encodes and the shader blends in linear space (`docs/es3-srgb.md`). Before it the class was stored as plain RGBA8 and dEQP's reference blends in linear space | sRGB write encode and destination decode for blending in each back end (PVRF `:113-114, :282-290`); a decision whether this is done in the GL layer (convert in the shader) or the colour-buffer format. Also fixes `fbo.color.blend` srgb8_alpha8, the sRGB names of `fragment_out.*.fixed` (24) and `fbo.blit.conversion.*srgb8_alpha8*` (32 names) |
| 2 | fbo.blit.depth_stencil (16 at `c4ccc966`) | 16 | the colour blit is built (`src/gl/blit.rs`: every class pair, NEAREST / LINEAR, scaling, flips, cut rectangles, scissor, the window as source and destination; fixed on the host, the AM62P rows of `tools/cts/am62p-gles3-tree.txt` 1, 2 and 4 to rerun); a depth or stencil copy is validated and then refused with GL_INVALID_OPERATION, said once in the log | a depth/stencil copy in each back end: the planes live in the back ends' buffers (`src/ds_retention.rs`; the PowerVR's packed D24S8 rows in its ZLS layout, `pvr_backend_depth_resolve`), so a back-end entry that copies between two objects' planes (or writes one from the CPU) and keeps their plane state |
| 3 | fbo.msaa.* (70) + fbo.completeness.samples (13) + the 8 MSAA blit cases of fbo.invalidate | ≤ 252 (same bucket; not split) | multisample renderbuffers are refused above one sample (`src/gl/framebuffer.rs:2587`), `MAX_SAMPLES` is 1 on all three back ends (`glRenderbufferStorageMultisample` raises GL_INVALID_OPERATION above 1, `src/gl/framebuffer.rs:2587`; no plan item covers MSAA); resolve needs the blit of rank 2 | `glRenderbufferStorageMultisample` with 2 / 4 samples, resolve into a single-sample rbo, `glGetInternalformativ` sample counts; hardware sample counts per GPU: unverified (4x already exists in the AM62P 4x-MSAA image config `tools/cts/am62p-ms4-tree.txt`, not through the ES 3.0 rbo API). Case count: 91 |
| 4 | fbo.depth (15), fbo.render.* (308), fbo.stencil (9), fbo.color.tex2darray / tex3d (72) | part of "Compile failed 181" (`RESULTS.md:1513`: color, depth, render) and "Image comparison failed 675"; **no split recorded** | reading from the sources: the fbo test shaders (`FBU:260`, `:455`, `:587`) use `isampler` / `usampler`, integer outputs and `sampler2DArray` / `sampler3D`; a "Compile failed" is the online compiler or the back end refusing one of them. Not a **none** if it is the integer samplers (formats-pvr / -v3d / -gc) or the array sampler (pvr-2darray) | measure first (host run or a rerun with the `glu` program log); then assign |
| 5 | fbo.depth float-depth and D32F cases | not recorded | `GL_DEPTH_COMPONENT32F` and `DEPTH32F_STENCIL8` as rbo / texture and as blit source (hardware support unverified here) | the depth-format list of `is_depth_format`, not read here |
| 6 | fbo.completeness.layer (8), fbo.completeness.size (2) | not recorded | layer attach rules on 3D / array, `size.distinct` | small; read the cases |
| – | the 88 unexplained fragment_out failures | 88 (derived) | not a cause: unaccounted (the record) | rerun fragment_out.basic.fixed / float and array.fixed / float, and read the "Image comparison failed" cases |

