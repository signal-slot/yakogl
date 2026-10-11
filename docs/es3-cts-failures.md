<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 3.0 CTS failure inventory

dEQP-GLES3, the mustpass list `tools/cts/gles3-mustpass.txt` (42,810 cases),
on the AM62P (PowerVR). Written 2026-10-10 for plan #3 (OpenGL ES 3.0, item
`es3-inventory`). It is modelled on `docs/es2-cts-failures.md`. This is the
**first version**: it is built only from data already in the repository. Nothing
was run for it. Later items refine it (section "Pending inputs").

## How to read the counts

The only full GLES3 run is AM62P, Zephyr, main `396d667a`, 2026-09-26:
**31,278 pass, 8,835 fail, 2,636 NotSupported, 5 QualityWarning, 56 with no
result** (all of `lifetime.*`). Source: `tools/cts/RESULTS.md` §1r (heading at
line 1437, group table lines 1457-1494, "8,835 failures by message" lines
1496-1516), and `tools/cts/STATUS.md` lines 9 and 250 (Japanese). The i.MX8MP
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
`docs/es3-fbo-fragops-failure-map.md` §1 records this). The group table is
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
| Cause | dEQP's shader executor renders every output into integer renderbuffers (R32I .. RGBA32UI); `glRenderbufferStorage` refused every integer format and RGB10_A2 (`src/gl/framebuffer.rs` renderbuffer_storage_args_ok / is_renderable_format). Shown in code (R-msg line 1501). For fragment_out and fbo the same cause is a reading from the message and the case names: the fragment_out bucket of 240 equals the integer names plus the RGB10_A2 names exactly (`docs/es3-fbo-fragops-failure-map.md` §2.3) |
| Host | yes: `tests/es3_int_rt.rs` |
| Fixed by | `int-rt-gl` (**merged**, GL core). Then `int-rt-pvr`, `int-rt-v3d`, `int-rt-gc` (ready, not merged) for the integer fragment outputs |
| Status | **Fixed in GL core by a merged item, not re-measured on the board.** The board will move these cases past set-up; they cannot pass before `int-rt-pvr` (PowerVR still has no integer output path, `docs/es3-pvr-formats.md`). Expect the 2,284 to turn into a different, smaller failure set, not into passes |

Shaders.derivate has a related bucket: `checkFramebufferStatus == COMPLETE
(es3fShaderDerivateTests)` **103 exact** (R-msg line 1516), "float render
targets". It is not counted in the 1,483 (different message). It is a float
colour target (RGBA32F or RGBA16F), which `GL_EXT_color_buffer_float` gates
(`docs/es3-fbo-fragops-failure-map.md` §2: `src/gl/state.rs` lists only
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
| Fixed by | `sized-formats-gl` (**merged**, GL core). `formats-pvr` (todo), `formats-v3d`, `formats-gc` (todo) for sampling |
| Status | **Upload side fixed by a merged item, not re-measured.** Sampling of the new classes is not done on any back end (`docs/es3-sized-formats.md` "Storage and the back end's mask"); the 522 integer-sampler cases need `formats-*` |

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
| Fixed by | `tf-api` (**merged**, commit 16ac8641): state, varyings, queries, discard. `tf-capture` (todo) for the data path (design `docs/es3-transform-feedback.md`; `tf-design` merged) |
| Status | **API half fixed by a merged item, not re-measured.** The 413 cannot pass before `tf-capture`; with `tf-api` alone they become "wrong buffer contents" failures (the design discusses this: `docs/es3-transform-feedback.md` around line 628 says non-zero limits without capture would turn the 413 + 421 cases into wrong-buffer failures) and the 421 NotSupported and 56 lifetime cases start to run |

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
| Groups and counts | fragment_ops.blend.fbo_srgb **1,106 to 1,111 (bound)**, derived in `docs/es3-fbo-fragops-failure-map.md` §2.2: the "Image compare failed" bucket is 1,293 (R-msg line 1502) and names fbo_srgb, ubo.single_nested_struct* and ubo.random; ubo fails 187 in all (R-grp line 1490), so fbo_srgb ≥ 1,293 − 187 = 1,106; fragment_ops fails 1,111 in all (R-grp line 1469), so fbo_srgb ≤ 1,111 |
| First message | `Image compare failed` |
| Host | no (picture verdict). The format handling half is a GL test |
| Fixed by | `fbo-fragops` (todo; scope in the map §3 rank 1). Design `docs/es3-srgb.md` (`ubo-uniform` item, merged) |

**RC7b. ubo nested-struct and random (≤ 187, bound).** The other part of the
same 1,293 bucket. All of ubo's 187 failures *may* be in it; the split is not
recorded. Layer: unknown (GL core UBO layout or the shader back end; the host
test `tests/es3_ubo.rs` runs the ubo groups through the GL API, and
b0c0fa6b fixed `glGetUniform*` of non-square matrices). Plan item: **no item
yet**. Counted as unclassified.

### RC8. glBlitFramebuffer is a stub; multisample renderbuffers

| | |
|---|---|
| Layer | GL core (`glBlitFramebuffer` always raises GL_INVALID_OPERATION, `src/gl/framebuffer.rs`; samples > 1 refused, `GL_MAX_SAMPLES` is 1) and the back ends |
| Groups and counts | "Got GL_INVALID_OPERATION (es3fFboTestCase)" **252 exact** in the bucket (R-msg line 1511), in fbo.blit, fbo.color, fbo.msaa; the three names are fbo subgroups only, so all 252 are in fbo's 1,116. Not split between blit and MSAA (`docs/es3-fbo-fragops-failure-map.md` §3 ranks 2 and 3). The 16 multisample failures (R-grp line 1474, 48 NotSupported) are a separate **unclassified** group |
| Host | API half: yes. Picture half: no |
| Fixed by | **no item yet** for the blit and MSAA; `fbo-fragops` (todo) is the item for "the largest remaining root cause" (it is sRGB first, per the map). `imx8mp-gles3-finish` (ready) adds a pure CPU reference blit module that is not wired to `glBlitFramebuffer` |

### RC9. fbo "Compile failed" (sglrGLContext), 181

| | |
|---|---|
| Layer | unknown: shaderc or a back end refusing the fbo test shaders (isampler/usampler, integer outputs, sampler2DArray, sampler3D) |
| Groups and counts | **181 exact** (R-msg line 1513): fbo.color, fbo.depth, fbo.render |
| First message | `Compile failed` (at sglrGLContext) |
| Cause | not split (R-msg: "not split yet"). A reading from the sources (`docs/es3-fbo-fragops-failure-map.md` §3 rank 4): the integer samplers (`formats-*`), the array sampler (`pvr-2darray`) or the integer outputs (`int-rt-*`) |
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
| Fixed by | `oq-gl` (in review), `occlusion-query` (todo, PowerVR), `oq-v3d` (todo, V3D and GC7000UL); designs `occlusion-design` merged (`docs/es3-occlusion-query.md`) |

### RC12. default_vertex_attrib int/uint current values (16 exact)

default_vertex_attrib **16 exact** (R-grp line 1462). Cause: pure-integer
current attribute values went through a float (the message of commit
a54bbdec, the `vertex-int-attribs` result). Layer: GL core
(`src/gl/vertex.rs`). Host: yes. Fixed by `vertex-int-attribs` (in review, not
merged). The commit message names `docs/es3-vertex-gap.md`, which is not on the
integration tip. The other vertex groups (vertex_arrays 41, attribute_location
48, draw 50, instanced 6) are **unclassified**: the "Fail (no message)" bucket
(133 exact, R-msg line 1515) names attribute_location.mixed*, instanced,
rasterizer_discard, shaders.derivate and texture_functions without a split.

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

`gl_MaxDrawBuffers` and `GL_MAX_DRAW_BUFFERS` are 1 in the ES 3.0 build on all
three back ends (the 4 required by ES 3.0 are missing). Which of the board's
failures it causes is **not recorded**: the NotSupported groups
draw_buffers_indexed (76) and part of fragment_out (115) show the limit; the
4-target seeds of fragment_out.random are inside its 51+ integer failures. The
host proves two programs (RC13 table). Count: **no source**. Layer: GL core and
all three back ends. Fixed by `mrt4` (todo). Note
`docs/es3-fbo-fragops-failure-map.md` says `CAP_COLOR_ATTACHMENTS` is 3
(`src/gl/context.rs:139`) while `docs/es3-shaderc-coverage.md` says the
front end folds 1; the two were not reconciled here.

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
| draw 50, attribute_location 48, vertex_arrays 41 | 139 | vertex groups; item `vertex-int-attribs` classified them on the host, outcome in `docs/es3-vertex-gap.md` (not yet on the tip) |
| negative_api 31, uniform_api 27, pbo 26, read_pixels 24, rasterization 21, implementation_limits 16, multisample 16, instanced 6 | 167 | `gl-core-misc` (running) takes negative_api, pbo, read_pixels, implementation_limits; `es3-rest-deqp-gap` (running) maps the rest; multisample is MSAA (RC8) with no item |
| fragment_ops | ≤ 5 | 1,111 − 1,106; in blend.default_framebuffer, scissor or stencil |

Check: 1,234 + 748 + 413 + 187 + 88 + 139 + 167 + 5 = **2,981**.

## Plan items against the causes

| item | state | root causes | what remains |
|---|---|---|---|
| int-rt-gl | merged | RC1 | board re-measure after int-rt-pvr |
| sized-formats-gl | merged | RC2 | sampling in the back ends |
| tex3d-levels | merged | RC2 (20 + 10) | board re-measure; tex3d-finish (running) keeps the texels |
| link-iface | merged | RC13 (7) | board re-measure |
| pp-splice | merged | RC13 (4) | board re-measure |
| tf-api | merged | RC3 (API), RC3b, RC10 | tf-capture for data; board re-measure |
| tf-infolog | review | RC4 | merge |
| tf-capture | todo | RC3 (413) | everything |
| int-rt-pvr / -v3d / -gc | ready | RC1 | all |
| formats-pvr / -v3d / -gc | todo | RC2 (sampling), part of RC9 | all |
| mrt4 | todo | RC14 | all |
| pvr-2darray | todo | RC6 | all |
| pvr-shadow | todo | RC5 | all |
| oq-gl / occlusion-query / oq-v3d | review / todo / todo | RC11 | all |
| gl-core-misc | running | RC10 (rest), negative_api, pbo, read_pixels, implementation_limits | its result |
| fbo-fragops | todo | RC7, RC8, RC9 | all |
| pvr-centroid | running | RC13 (9) | its result |
| vertex-int-attribs | review | RC12 | merge; a small `src/gl/vertex.rs` follow-up is planned once an id frees up |
| **no item yet** | | RC7b (ubo ≤ 187), RC8 (blit, MSAA), the float-target bucket (103), indexed out/inout arguments, V3D `centroid`, long reflection names, multisample 16, uniform_api 27, rasterization 21, and the unclassified remainder | |

### Merged, not re-measured on the board

`int-rt-gl`, `sized-formats-gl`, `tex3d-levels`, `link-iface`, `pp-splice` and
`tf-api` have landed on `ccfarm/integration` (checked in the git log and the
`tests/es3_*.rs` files: es3_int_rt, es3_sized_formats, es3_tf_api,
es3_link_iface, es3_shaderc_pp, es3_ubo). Their effect on dEQP numbers is
**host-tested only**. The numbers in this file are all those of 396d667a;
nothing here is a board result at a later commit. The first board run after
them is `board-am62p`; `oq-gc` and `am62p-gles3-percase` (no data: the
console was on the wrong UART port) are the sessions meant to produce the per-case list.

## Pending inputs

These will refine this file; none was used here.

* `docs/es3-shader-failure-map.md` (item `es3-shader-deqp-gap`, in review): the
  shaders group (2,231) mapped to buckets. Will split the 748.
* `docs/es3-rest-failure-map.md` (item `es3-rest-deqp-gap`, running): the groups
  other than shaders, texture, fbo, fragment_ops, fragment_out. Will split the
  small-group rows (draw, attribute_location, vertex_arrays, negative_api,
  uniform_api, pbo, read_pixels, rasterization, implementation_limits,
  multisample, instanced).
* `docs/es3-texture-failure-map.md` (item `es3-texture-deqp-gap`: the result is
  on a branch, not on the integration tip): will split the 1,234 of texture.
* `docs/es3-host-run.md` (item `host-gles3-run`, in review): the host
  full-mustpass run; will turn the "host: yes" claims into measured counts for the
  API and compile buckets.
* `tools/cts/log/board-sessions/am62p-gles3-r2*.md`: the per-case board list at the
  integration tip. It is the only source that can give per-case counts and
  check the "merged, not re-measured" rows.
* `docs/es3-backend-caps.md` (item `caps-redo`, result on a branch; the
  first `es3-caps` merge was reverted): V3D 4.2 and GC7000UL caps. Nothing here
  depends on it; V3D and GC7000UL hardware statements are marked unverified or
  absent.

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
