<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# The whole dEQP-GLES3 mustpass on the host, three GPUs (2026-10-12)

**Every number in this document is HOST-ONLY.** The library is built with the
LINUX OSAL. It records the command stream and does not execute it, so every
glReadPixels reads black. Every render verdict (a Pass or Fail decided by an
image, a buffer read back or a pixel count) means nothing and must never be
quoted. What is real: what the GL API returns (errors, queries, NotSupported
decided from our limits and formats), each shader's compile and link outcome
with our info log, and the library's own messages on stderr (a refused draw,
the GL call that set an error). See the header of `tools/cts/host-deqp-run.sh`.

The AM62P column is the board run of `tools/cts/RESULTS.md` §1r (main
`396d667a`, 2026-09-26). It is not HOST-ONLY. It is here to compare with, not to
add to. An earlier host run of the same list, from integration commit
`522c41f` (2026-10-09), is kept as a record in `tools/cts/log/es3-host-run.md`.
This run replaces its numbers for main.

## What ran

- Library and harness: main `9390731` (this branch's base; `src/` is not
  changed here), for each of `GPU=powervr`, `vivante` and `v3d`, gles3
  (`YAKOGL_REPORT_ES2=OFF`, the online compiler, `YAKOGL_OSAL=linux`).
- dEQP: VK-GL-CTS `93bca018` (the revision of §1r; deqp-gles3 prints
  `vulkan-cts-1.4.6.2-555-g93bca01861b0`), the checkout in
  `$CCFARM_DEPS_DIR/VK_GL_CTS` with the yakogl target. deqp-gles3 was built by
  the script into `build/host-deqp/deqp-gles3`.
- Case list: the union of `android/cts/main/gles3-main-*.txt`, **42,810
  cases**, the same list as §1r's mustpass (`tools/cts/gles3-mustpass.txt`).
  429 chunks of 100 cases, 20 chunks in parallel per GPU, a 256x256 pbuffer.
- About two minutes per GPU, the three GPUs at once on one 64-thread host.

```sh
CTS=$CCFARM_DEPS_DIR/VK_GL_CTS
for g in powervr vivante v3d; do
    tools/cts/host-deqp-run.sh --cts="$CTS" --gpu=$g --module=gles3 --jobs=20 \
        --out=build/host-deqp/full-$g > build/full-$g.log 2>&1
done
# the report again, from a finished run:
tools/cts/host-deqp-run.sh --cts="$CTS" --report=build/host-deqp/full-v3d
```

Each run directory holds `summary.txt` (`cts-summary.py --by-group 3
--by-message`: status totals, the group table, every failure message with its
count), `failures.tsv` (every failing case with its message), `aborted.txt`
(every chunk that needed more than one process) and the `.qpa` logs. The run
log (`build/full-$g.log`) holds `cts-refusals.py`'s work list. The tables
below fold the source line out of each message, as §1r does, and add the
library's stderr line for the case (`c_NNN.out`) where it names the cause.

No harness change was needed. The script's resume (a chunk goes on in a fresh
process after dEQP ends its run) handled every stop below, and every case
began.

## Begun against asked (HOST-ONLY)

| GPU | asked | began | never began | processes | chunks resumed | stops: ResourceError | stops: error in a case's deinit | exit 134 / 137 / timeout |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| powervr | 42,810 | 42,810 | 0 | 477 | 5 | 48 | 0 | none |
| vivante | 42,810 | 42,810 | 0 | 1,569 | 13 | 0 | 1,152 | none |
| v3d | 42,810 | 42,810 | 0 | 1,893 | 47 | 330 | 1,152 | none |

Every process ended with exit 0 or 1. **No chunk crashed (no exit 134, no
panic line), none was killed (137) and none reached the 600 s timeout**, on
any GPU. So there is no crashing chunk to list by case.

lifetime.\* (56 cases) began and passed its API checks on every GPU. On the
AM62P and in the 2026-10-09 host run its init died, because the group links a
program with transform feedback of gl_Position and transform feedback was not
implemented.

### The chunks that stopped early, by case

dEQP ends its whole run in two cases: a case ends with ResourceError, or a
case's deinit throws. Both end the process with exit 1, like a chunk with any
failing case. The script then goes on with the cases not begun yet. Each row
is one chunk. "stops" is how many processes stopped right after a case of
that group (the full list is in `aborted.txt`).

**PowerVR** (5 chunks, 48 ResourceError):

| chunk | processes | stops | cases the processes stopped after |
|---|---:|---:|---|
| c_103, c_105, c_108, c_110, c_111 | 9, 13, 13, 13, 5 | 8, 12, 12, 12, 4 | shaders.builtin_functions.precision.{clamp,faceforward,mix,refract,smoothstep}.\*.vec3/vec4 |

Cause, from the library: `pvr_backend: a draw (24384 vertices/indices) does
not fit an empty frame: the general arena (1048576 bytes) is too small; not
drawn`. The draw is refused, glGetError gives GL_OUT_OF_MEMORY and dEQP takes
that as a resource error. The 1 MiB arena is the back end's default
(`src/pvr/backend_tests.rs`,
`the_default_arenas_are_1024_and_768_kib_and_bound_a_draws_client_indices`),
not a host setting, so the board should refuse these draws too. A non-indexed
draw is not cut into pieces.

**Vivante (GC7000)** (13 chunks, 1,152 deinit errors):

| chunk | processes | stops | cases the processes stopped after |
|---|---:|---:|---|
| c_052 .. c_064 | 23, 100 x11, 30 | 23, 100 x11, 29 | fragment_ops.blend.fbo_srgb.\* (every case of the group) |

Cause, from the library: `draw refused: this back end cannot render into a
SRGB8_ALPHA8 colour attachment (draw buffer 0)`, then GL_INVALID_OPERATION in
`refuse_undrawable_targets`. The case does not check the error after its
draws, so its verdict is the picture ("Image compare failed"). The pending
error is found by `glBindRenderbuffer(GL_RENDERBUFFER, 0)` in the case's
deinit (es3fBlendTests.cpp:229). dEQP logs "Error in test case deinit, test
program will terminate." and ends the run. **On a board image this ends the
whole image after the first fbo_srgb case**; a GC7000 or V3D board recipe
must give fragment_ops.blend.fbo_srgb its own image, or one image per case,
until sRGB targets render.

**V3D** (47 chunks: the same 13 fbo_srgb chunks as the GC7000, 1,152 deinit
errors, plus 34 chunks with 330 ResourceError):

| chunk | processes | stops | cases the processes stopped after |
|---|---:|---:|---|
| c_052 .. c_064 | 23, 100 x11, 30 | 23, 100 x11, 29 | fragment_ops.blend.fbo_srgb.\* (as the GC7000) |
| c_036, c_040 | 4, 4 | 3, 3 | fbo.invalidate.format.{r16f,rg16f,rgba16f}, fbo.render.shared_colorbuffer_clear.tex2d_\*16f |
| c_074, c_078, c_081 | 10, 10, 2 | 9, 9, 1 | fragment_out.{array,basic}.float.\*16f\*, fragment_out.random.3 |
| c_088 | 5 | 4 | pbo.renderbuffer.{rg16f,rgba16f}_{clears,triangles} |
| c_131, c_132, c_133 | 6 each | 5 each | shaders.derivate.{dfdx,dfdy,fwidth}.\* |
| c_296 .. c_299 | 7, 5, 10, 9 | 6, 5, 9, 8 | shaders.texture_functions.{texture,texturelod,textureproj,textureprojlod,texturesize}.\* |
| c_313, c_314, c_317 | 7, 6, 13 | 7, 5, 12 | texture.filtering.{2d,cube}.\* |
| c_318, c_319, c_322, c_323 | 7, 11, 9, 5 | 6, 10, 8, 4 | texture.format.compressed.etc2_\*, texture.format.sized.{2d,cube}.\* |
| c_330 | 4 | 3 | texture.size.cube.512x512_\*_mipmap |
| c_343, c_344 | 2, 2 | 1, 1 | texture.units.{2_units.only_cube.5, 4_units.only_cube.5} |
| c_358, c_359 | 36, 37 | 36, 36 | texture.wrap.etc2_eac_rgba8.\*, texture.wrap.etc2_rgb8.\* |
| c_361 .. c_368, c_423 | 28, 34, 20, 19, 6, 6, 5, 4, 13 | 28, 33, 20, 18, 5, 5, 5, 3, 12 | transform_feedback.{array,basic_types,random_full_array_capture}.\* |

Every ResourceError is a draw or clear the V3D back end refuses, which the
context reports as GL_OUT_OF_MEMORY: "an R16F / RGBA16F colour target is not
written yet", "a texture format other than RGBA8, SRGB8_ALPHA8, L8, A8, LA8
and R32F (RGB565, half float, ETC2) is not sampled yet", "the texture state
does not fit its record", "a texture larger than the tiled-copy budget", and
for transform feedback (129) "the job builder refused the draw (a uniform,
program or primitive it does not write)".

## The groups (HOST-ONLY except the AM62P column)

p / f / ns / other = Pass / Fail / NotSupported / any other status. Other is
915 QualityWarning (all in shaders) and 48 ResourceError on the PowerVR, and
330 ResourceError on the V3D. No case is missing on any GPU. **The Pass and
Fail columns of the render groups mean nothing on the host.** Compare only
NotSupported, the API groups and the message tables below.

| group | cases | AM62P §1r p / f / ns / other | powervr p / f / ns / other | vivante p / f / ns / other | v3d p / f / ns / other |
|---|---:|---|---|---|---|
| attribute_location | 274 | 226 / 48 / 0 / 0 | 226 / 48 / 0 / 0 | 226 / 48 / 0 / 0 | 226 / 48 / 0 / 0 |
| buffer | 695 | 695 / 0 / 0 / 0 | 168 / 527 / 0 / 0 | 168 / 527 / 0 / 0 | 168 / 527 / 0 / 0 |
| clipping | 602 | 602 / 0 / 0 / 0 | 0 / 602 / 0 / 0 | 0 / 602 / 0 / 0 | 0 / 602 / 0 / 0 |
| color_clear | 19 | 19 / 0 / 0 / 0 | 0 / 19 / 0 / 0 | 0 / 19 / 0 / 0 | 0 / 19 / 0 / 0 |
| default_vertex_array_object | 1 | 1 / 0 / 0 / 0 | 1 / 0 / 0 / 0 | 1 / 0 / 0 / 0 | 1 / 0 / 0 / 0 |
| default_vertex_attrib | 80 | 64 / 16 / 0 / 0 | 0 / 80 / 0 / 0 | 0 / 80 / 0 / 0 | 0 / 80 / 0 / 0 |
| depth_stencil_clear | 10 | 10 / 0 / 0 / 0 | 0 / 10 / 0 / 0 | 0 / 10 / 0 / 0 | 0 / 10 / 0 / 0 |
| dither | 20 | 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 | 0 / 20 / 0 / 0 | 0 / 20 / 0 / 0 |
| draw | 322 | 272 / 50 / 0 / 0 | 2 / 320 / 0 / 0 | 2 / 320 / 0 / 0 | 2 / 320 / 0 / 0 |
| draw_buffers_indexed | 76 | 0 / 0 / 76 / 0 | 0 / 0 / 76 / 0 | 0 / 0 / 76 / 0 | 0 / 0 / 76 / 0 |
| fbo | 2028 | 657 / 1116 / 255 / 0 | 517 / 1256 / 255 / 0 | 462 / 1311 / 255 / 0 | 439 / 1328 / 255 / 6 |
| fence_sync | 10 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 |
| fragment_ops | 3170 | 2059 / 1111 / 0 / 0 | 91 / 3079 / 0 / 0 | 89 / 3081 / 0 / 0 | 89 / 3081 / 0 / 0 |
| fragment_out | 940 | 206 / 619 / 115 / 0 | 0 / 825 / 115 / 0 | 0 / 825 / 115 / 0 | 0 / 806 / 115 / 19 |
| implementation_limits | 49 | 33 / 16 / 0 / 0 | 38 / 11 / 0 / 0 | 41 / 8 / 0 / 0 | 38 / 11 / 0 / 0 |
| instanced | 45 | 39 / 6 / 0 / 0 | 0 / 45 / 0 / 0 | 0 / 45 / 0 / 0 | 0 / 45 / 0 / 0 |
| lifetime | 56 | 0 / 0 / 0 / 0 (56 no result) | 56 / 0 / 0 / 0 | 56 / 0 / 0 / 0 | 56 / 0 / 0 / 0 |
| multisample | 64 | 0 / 16 / 48 / 0 | 9 / 7 / 48 / 0 | 9 / 7 / 48 / 0 | 9 / 7 / 48 / 0 |
| negative_api | 312 | 277 / 31 / 4 / 0 | 299 / 9 / 4 / 0 | 300 / 8 / 4 / 0 | 300 / 8 / 4 / 0 |
| occlusion_query | 62 | 0 / 62 / 0 / 0 | 62 / 0 / 0 / 0 | 62 / 0 / 0 / 0 | 62 / 0 / 0 / 0 |
| pbo | 49 | 17 / 26 / 6 / 0 | 43 / 0 / 6 / 0 | 15 / 28 / 6 / 0 | 11 / 28 / 6 / 4 |
| polygon_offset | 20 | 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 | 0 / 20 / 0 / 0 | 0 / 20 / 0 / 0 |
| prerequisite | 3 | 3 / 0 / 0 / 0 | 2 / 1 / 0 / 0 | 2 / 1 / 0 / 0 | 2 / 1 / 0 / 0 |
| primitive_restart | 504 | 504 / 0 / 0 / 0 | 504 / 0 / 0 / 0 | 504 / 0 / 0 / 0 | 504 / 0 / 0 / 0 |
| rasterization | 100 | 62 / 21 / 12 / 5 | 34 / 54 / 12 / 0 | 34 / 54 / 12 / 0 | 34 / 54 / 12 / 0 |
| rasterizer_discard | 51 | 0 / 51 / 0 / 0 | 0 / 51 / 0 / 0 | 0 / 51 / 0 / 0 | 0 / 51 / 0 / 0 |
| read_pixels | 48 | 24 / 24 / 0 / 0 | 0 / 48 / 0 / 0 | 0 / 48 / 0 / 0 | 0 / 48 / 0 / 0 |
| samplers | 42 | 42 / 0 / 0 / 0 | 42 / 0 / 0 / 0 | 42 / 0 / 0 / 0 | 28 / 14 / 0 / 0 |
| shader_api | 52 | 38 / 0 / 14 / 0 | 38 / 0 / 14 / 0 | 38 / 0 / 14 / 0 | 38 / 0 / 14 / 0 |
| shaders | 20616 | 18385 / 2231 / 0 / 0 | 2488 / 17165 / 0 / 963 | 2241 / 18375 / 0 / 0 | 2248 / 18325 / 0 / 43 |
| state_query | 946 | 874 / 63 / 9 / 0 | 912 / 25 / 9 / 0 | 916 / 21 / 9 / 0 | 909 / 28 / 9 / 0 |
| texture | 5105 | 835 / 2612 / 1658 / 0 | 22 / 3425 / 1658 / 0 | 19 / 3428 / 1658 / 0 | 22 / 3296 / 1658 / 129 |
| transform_feedback | 882 | 0 / 461 / 421 / 0 | 882 / 0 / 0 / 0 | 880 / 2 / 0 / 0 | 740 / 1 / 12 / 129 |
| ubo | 2355 | 2168 / 187 / 0 / 0 | 0 / 2355 / 0 / 0 | 0 / 2355 / 0 / 0 | 0 / 2355 / 0 / 0 |
| uniform_api | 2181 | 2136 / 27 / 18 / 0 | 1605 / 576 / 0 / 0 | 1604 / 577 / 0 / 0 | 1605 / 576 / 0 / 0 |
| vertex_array_objects | 10 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 |
| vertex_arrays | 1005 | 964 / 41 / 0 / 0 | 40 / 965 / 0 / 0 | 40 / 965 / 0 / 0 | 40 / 965 / 0 / 0 |
| info | 6 | 6 / 0 / 0 / 0 | 6 / 0 / 0 / 0 | 6 / 0 / 0 / 0 | 6 / 0 / 0 / 0 |
| TOTAL | 42810 | **31278 / 8835 / 2636 / 5** (+56 no result) | **8107 / 31543 / 2197 / 963** | **7777 / 32836 / 2197 / 0** | **7597 / 32674 / 2209 / 330** |

What the columns can say:

- **NotSupported: 2,197 on the PowerVR and GC7000 against §1r's 2,636.** The
  439 fewer are transform_feedback (421 on the board, 0 here: transform
  feedback is implemented now) and uniform_api (18 on the board, 0 here).
  Every other group has the board's count. The V3D has 12 more, all in
  transform_feedback: dEQP reports "Capturing arrays is not supported
  (undefined in specification)" (es3fTransformFeedbackTests.cpp:1038) when
  the link fails, and the V3D refuses those 12 programs at link ("a flat /
  centroid varying past input 31"). So they are V3D refusals that read as
  NotSupported.
- Groups the board failed now pass on the host on every GPU: occlusion_query
  (62), lifetime (56) and transform_feedback (882 / 880 / 740). The
  transform_feedback verdicts are real even on the host: the GL core captures
  on the CPU on every GPU (docs/es3-transform-feedback.md §4.1), so the values
  dEQP reads back do not come from the GPU. The occlusion_query Pass is not
  checked here to be more than an API pass. negative_api is 9 / 8 / 8 failures against 31 on
  the board, state_query 25 / 21 / 28 against 63.
- The 7,600 to 8,100 Pass are API and compile cases plus whatever a black
  read-back happens to satisfy. They are not a pass rate.

## Failures by message, mapped onto §1r (HOST-ONLY)

The message is the first failure line of each case, dEQP's own detail
string, with the source line folded (`(file)` instead of `at file:NNN`). The
cause column quotes the library's stderr for those cases where it names one.
Mapping labels:

- **§1r row N**: the same bucket as row N of §1r's table of its sixteen
  largest messages (RESULTS.md, 1r).
- **§1r group**: the message is below §1r's cut, but its group failed on the
  AM62P.
- **new (GPU)**: a back-end refusal of that GPU. §1r is the PowerVR board, so
  a GC7000 or V3D refusal cannot be in it.
- **new**: a bucket that is not in §1r and whose cause is shared code or the
  PowerVR.

None of the buckets below is host-only in the sense of a host artefact: each
is our GL layer or a back end refusing something, which the board does too.

### §1r's sixteen largest messages

| §1r cases | §1r message | powervr | vivante | v3d | reading |
|---:|---|---:|---:|---:|---|
| 1,483 | Failed to set up framebuffer object: GL_INVALID_ENUM (glsShaderExecUtil) | 54 | 54 | 18 | **§1r row 1, mostly gone**: integer renderbuffers are made now. The rest is a new cause: 54 builtin_functions.precision.{matrixcompmult,outerproduct,transpose} mat4 cases need four colour attachments, and `CAP_COLOR_ATTACHMENTS` (src/gl/context.rs) is 3 on every GPU, so glFramebufferRenderbuffer(GL_COLOR_ATTACHMENT3) is GL_INVALID_ENUM. ES 3.0 asks for at least 4 (implementation_limits.max_color_attachments and max_draw_buffers fail on every GPU: "Reported: 3") |
| (1,483) | the same cases, next error: Error in draw: GL_INVALID_OPERATION (glsShaderExecUtil) | 0 | 1,507 | 705 | **new (GC7000, V3D)**: these cases now reach the draw, and the GC7000 and V3D back ends refuse integer colour targets ("draw refused: this back end cannot render into a R32UI colour attachment") |
| 1,293 | Image compare failed | 4,570 | 4,565 | 4,569 | render verdict, not measurable. But on the GC7000 and V3D, 1,107 of them (fragment_ops.blend.fbo_srgb) carry a real refusal: **new (GC7000, V3D)**, "cannot render into a SRGB8_ALPHA8 colour attachment" (and the deinit stop above) |
| 736 | Runtime check failed: glGetError() == GL_NO_ERROR (es3fTextureSpecificationTests) | 49 | 472 | 618 | **§1r row 3**. PowerVR: 687 fewer, 6 of the rest are glTexSubImage3D GL_INVALID_OPERATION. GC7000 and V3D: their back ends refuse the formats at upload ("GL_RG8I is not supported: this back end samples none of its storage classes", RG8_SNORM, ...) |
| 675 | Image comparison failed | 9,612 (+1,254 "Image comparison failed.") | 8,576 (+1,254) | 7,540 (+1,254) | render verdict, not measurable |
| 531 | After framebuffer setup: GL_INVALID_ENUM (es3fFragmentOutputTests) | 13 | 26 | 1 | **§1r row 5, mostly gone**. The rest are fragment_out.random cases with a 4th colour attachment (GL_INVALID_ENUM in glFramebufferRenderbuffer): the attachment cap of 3, as row 1 |
| (531) | the same cases, next error: glDrawElements: GL_INVALID_OPERATION (es3fFragmentOutputTests) | 0 | 531 | 123 | **new (GC7000, V3D)**: integer and sRGB colour targets refused at the draw |
| 522 | Texture upload failed: GL_INVALID_OPERATION (gluTexture) | 0 | 370 | 514 | **§1r row 6: gone on the PowerVR**. GC7000 and V3D: their back ends refuse integer formats at upload (GL_RGBA8UI, GL_RGBA8I, ...: "samples none of its storage classes") |
| 413 | glTransformFeedbackVaryings(): GL_INVALID_VALUE | 0 | 0 | 0 | **§1r row 7: gone** (transform feedback is implemented) |
| 298 | Image verification failed | 1,228 | 796 | 337 | render verdict. The PowerVR count includes the 432 texture.shadow cases, which compile now |
| 292 | Failed to compile shader program (glsTextureTestUtil) | 150 | 432 | 825 | **§1r row 9**. PowerVR: texture.shadow compiles now; the 150 are texture.format integer look-ups on a cube, 3D or array ("a texture fetch with an integer result on a cube, a 3D texture or an array ... is not selected"). GC7000: texture.shadow 432 ("an ES 3.00 texture look-up (a shadow sampler, ...) is not selected yet"). V3D: "a texture look-up other than 2D or cube" 537, "a shadow look-up" 288 |
| 270 | Got GL_INVALID_ENUM (es3fFboTestCase) | 0 | 0 | 0 | **§1r row 10: gone** |
| 252 | Got GL_INVALID_OPERATION (es3fFboTestCase) | 34 | 308 | 307 | **§1r row 11**. PowerVR: fbo.msaa 34, every format at 2 and 4 samples: glRenderbufferStorageMultisample sets GL_INVALID_OPERATION (the case's GL log shows it right after that call). GC7000 and V3D: fbo.blit 210 / 207 and fbo.color 64 / 66 as well, integer and sRGB targets refused at the draw. V3D also has 95 "Got GL_OUT_OF_MEMORY" (below) |
| 189 | Failed to compile shader program (glsShaderRenderCase) | 226 | 178 | 194 | **§1r row 12**, all shaders.texture_functions. PowerVR: integer look-ups on a cube, 3D or array texture (121 fragment, 105 vertex). GC7000: the ES 3.00 look-up refusal. V3D: non-2D/cube 97, offsets 46, shadow 31 |
| 181 | Compile failed (sglrGLContext) | 468 | 181 | 397 | **§1r row 13, more on the PowerVR**: texture.specification 298 and texture.units 103 upload integer textures now and fail one step later, at the integer look-up on a 3D, array or cube texture (458); fbo.depth 10 is gl_FragDepth (`Usc(FragmentOutputsNotBuilt)`). GC7000: fbo.color / render / depth, the ES 3.00 look-up refusal. V3D: non-2D/cube 254, integer look-up 133, gl_FragDepth 10 |
| 159 | Texture upload failed: GL_INVALID_ENUM (gluTexture) | 0 | 0 | 0 | **§1r row 14: gone** (depth texture formats upload) |
| 133 | Fail (no message) | 11,216 | 11,019 | 10,969 | render verdict, not measurable |
| 103 | checkFramebufferStatus == COMPLETE (es3fShaderDerivateTests) | 15 | 15 | 15 | **§1r row 16**, 88 fewer. PowerVR and V3D: GL_INVALID_OPERATION in glRenderbufferStorageMultisample |

The transform-feedback row of §1r's work list (RESULTS.md: "Compile failed:
'' at es3fTransformFeedbackTests.cpp", 48 on the board, read there as a
compiler refusal with an empty info log) is 0 / 2 / 0 here. On the GC7000 the
two cases' vertex shader info log does hold the reason
(`Abi(LoadBalancingTempsUnmeasured { num_out: 14, temps: 17, temps_max: 16
})`). The empty `''` in dEQP's message is the program's link log, so the
empty string is dEQP's message format and not a silent compiler, at least
for these two.

### Every other API and compile message (below §1r's cut)

These are real on the host. powervr / vivante / v3d are case counts.

| powervr | vivante | v3d | message | groups | cause (library) | mapping |
|---:|---:|---:|---|---|---|---|
| 0 | 0 | 572 | Shader compilation failed (glsBuiltinPrecisionTests) | shaders.builtin_functions.precision | `Program(Prepare(Undefined(ValueId(n))))` | new (V3D) |
| 26 | 0 | 634 | Compile failed (es3fFragmentOutputTests) | fragment_out | PowerVR: `Usc(Select(Unsupported("a register read that nothing writes")))` (random); V3D: `Prepare(Undefined(..))` | §1r group (fragment_out); new (V3D) |
| 1 | 7 | 247 | Compile failed | uniform_api.random, ubo, shaders.builtin_functions (V3D 244) | PowerVR: "the shader does not fit the buffer"; GC7000: constant registers over 320; V3D: `Prepare(Undefined(..))` 244, `Select(OutOfScratch)` 2 | §1r groups; new (GC7000, V3D) |
| 24 | 0 | 48 | Runtime check failed: 'm_program->isOk()' (es3fShaderPrecisionTests) | shaders.precision | PowerVR: "a register read that nothing writes"; V3D: `Prepare(Undefined(..))` | §1r group (shaders); new (V3D) |
| 0 | 0 | 22 | Compile failed (es3fShaderBuiltinVarTests) | shaders.builtin_variable | `Prepare(Undefined(..))` | new (V3D) |
| 0 | 0 | 96 | Failed to compile shader (es3fVertexTextureTests) | texture.vertex | "a texture look-up other than 2D or cube" | new (V3D) |
| 0 | 0 | 14 | Failed to compile shaders (glsSamplerObjectTest) | samplers.{single,multi}_tex_3d | "a texture look-up other than 2D or cube" | new (V3D): samplers passed 42/42 on the AM62P |
| 0 | 38 | 18 | Shader failed | shaders.texture_functions | GC7000: "textureSize at a level other than a literal 0"; V3D: "textureSize of a 3D or array sampler" | new (GC7000, V3D) |
| 0 | 26 | 0 | Failed to compile shader | shaders.random | the GC7000's ES 3.00 look-up refusal | new (GC7000) |
| 15 | 15 | 15 | Compile failed (es3fFragDepthTests) | shaders.fragdepth | gl_FragDepth: PowerVR `Usc(FragmentOutputsNotBuilt)`, GC7000 "this output is not selected yet", V3D `NotYet("gl_FragDepth")` | §1r group (shaders) |
| 0 | 2 | 0 | expected shaders to compile and link properly, but failed to compile. | shaders.large_constant_arrays | `Abi(LoadBalancingTempsUnmeasured)` | new (GC7000) |
| 2 | 2 | 2 | expected shaders to compile and link properly, but failed to link. | shaders.uniform_block.valid.long_instance_name_* | a 63-character uniform name fills the reflection's 64-byte name field | §1r group (shaders) |
| 0 | 2 | 1 | Compile failed: '' / Link failed: '' (es3fTransformFeedbackTests) | transform_feedback.random_full_array_capture | GC7000 `LoadBalancingTempsUnmeasured`; V3D "a flat varying past input 31" | §1r row of the TF work list (48 on the board), new causes |
| 0 | 0 | 129 | ResourceError: render: GL_OUT_OF_MEMORY (es3fTransformFeedbackTests) | transform_feedback.{array,basic_types,random_full_array_capture} | "the job builder refused the draw (a uniform, program or primitive it does not write)" | new (V3D) |
| 48 | 0 | 0 | ResourceError: Error in draw: GL_OUT_OF_MEMORY (glsShaderExecUtil) | shaders.builtin_functions.precision vec3/vec4 | the PowerVR's 1 MiB general arena (above) | new (PowerVR); whether the board reached these draws in §1r is not separated |
| 0 | 0 | 95 + 11 | Got GL_OUT_OF_MEMORY / GL_OUT_OF_MEMORY (es3fFboTestCase) | fbo.blit 82, fbo.color 24 | V3D draws refused (RGBA16F targets, unsampled formats) | new (V3D) |
| 0 | 0 | 201 | ResourceError, GL_OUT_OF_MEMORY at texturing state / draw / glReadPixels / glCreateProgram / glClearBufferfv / glDrawArrays / TexStorage (14 messages) | texture.wrap 72, texture.format 28, shaders.texture_functions 28, fragment_out 19, texture.filtering 24, shaders.derivate 15, fbo 6, pbo 4, texture.size 3, texture.units 2 | V3D draws and clears refused (above) | new (V3D) |
| 0 | 40 + 30 + 31 + 29 | 44 + 35 + 35 + 32 | Set {3d, 2d array, cube map, 2d} texture image data: GL_INVALID_OPERATION (es3fTextureUnitTests) | texture.units | the back end refuses the format at upload (R8UI, R32I, RG32I, RGB8_SNORM, ...) | §1r group (texture); new (GC7000, V3D) as a cause |
| 102 | 33 | 27 | Framebuffer is not complete: INCOMPLETE_ATTACHMENT (es3fFboTestCase) | fbo.color, fbo.invalidate | PowerVR: GL_INVALID_OPERATION in glRenderbufferStorageMultisample; GC7000 and V3D: glTexImage2D refusals | §1r group (fbo) |
| 44 | 6 | 1 | Framebuffer is not complete: INCOMPLETE_ATTACHMENT (es3fFboRenderTest) | fbo.render | (no stderr line) | §1r group (fbo) |
| 0 | 0 | 5 | GL_INVALID_OPERATION: '' (es3fFboRenderTest) | fbo.render | "GL_RGB16F is not supported" | new (V3D) |
| 0 | 75 | 98 | Expected no error but got GL_INVALID_OPERATION | fbo.completeness | glTexImage2D refusals (R16I, R16UI, R11F_G11F_B10F, ...) | §1r group (fbo); new (GC7000, V3D) as a cause |
| 35 | 15 | 15 | Framebuffer checked is incomplete, expected complete | fbo.completeness | | §1r group (fbo) |
| 21 | 21 | 21 | Framebuffer checked as complete, expected incomplete | fbo.completeness | | §1r group (fbo) |
| 22 | 24 | 24 | GL_INVALID_OPERATION (es3fFboTestCase) | fbo.blit 16, fbo.invalidate 6, fbo.color 2 | glBlitFramebuffer: "depth/stencil blits are not implemented" and other blit refusals | §1r group (fbo) |
| 0 | 88 | 88 | Draw: GL_INVALID_OPERATION (es3fShaderDerivateTests) | shaders.derivate | integer / float targets refused at the draw | new (GC7000, V3D) |
| 0 | 64 | 16 | After render: GL_INVALID_OPERATION (es3fShaderPrecisionTests) | shaders.precision | integer targets refused at the draw | new (GC7000, V3D) |
| 0 | 11 | 61 | Got unexpected error | fbo.render | GC7000: sRGB target refused; V3D: R16F target and the pending clear refused | new (GC7000, V3D) |
| 0 | 14 + 12 + 12 + 7 + 6 + 1 | 14 + 12 + 12 + 7 + 6 + 1 | glGenFramebuffers / glDrawArrays / glPixelStorei / glClearBufferuiv / glClearBufferiv / glClear: GL_INVALID_OPERATION (es3fPixelBufferObjectTests, es3fReadPixelsTests) | pbo.renderbuffer, read_pixels | an earlier refused draw into an integer or sRGB target leaves the error pending; the next checked call reports it | new (GC7000, V3D) |
| 0 | 2 | 2 | glGetError() returned GL_INVALID_OPERATION (glsScissorTests) | fragment_ops.scissor | integer targets refused at the draw | new (GC7000, V3D) |
| 0 | 2 | 2 | glCreateProgram(): GL_INVALID_OPERATION (gluShaderProgram) | fbo.invalidate, fbo.render | a pending error from an sRGB draw refusal | new (GC7000, V3D) |
| 0 | 3 | 0 | Runtime check failed: checkFramebufferStatus == COMPLETE (es3fTextureSpecificationTests) | texture.specification | | §1r group (texture) |
| 38 | 0 | 12 | Got unexpected texture size | shaders.texture_functions | V3D: glTexStorage2D refuses RGBA8I / RGBA8UI | §1r group (shaders) |
| 20 | 20 | 20 | glDrawArrays(): GL_INVALID_OPERATION (glsVertexArrayTests) | vertex_arrays.single_attribute | "attribute 0 fetches ... out of range" (`build_common`) | §1r group (vertex_arrays, 41 there) |
| 1 | 1 | 1 | glDrawElementsInstanced(): GL_INVALID_OPERATION (glsDrawTest) | draw.random | the same range check | §1r group (draw) |
| 9 | 10 | 10 | Got invalid error | negative_api, state_query.fbo | GC7000 / V3D: glTexImage2D RGBA32I refusals | §1r group (negative_api, 31 there) |
| 12 / 11 | 9 / 8 | 12 / 11 | Got invalid integer / float value | state_query.integers, integers64 | | §1r group (state_query, 63 there) |
| 2 | 2 | 3 | got invalid value | state_query.rbo, state_query.shader | | §1r group (state_query) |
| 11 | 8 | 11 | Requirement not satisfied | implementation_limits (max_color_attachments, max_draw_buffers, compressed_texture_formats, uniform components, ...) | | §1r group (16 there) |
| 4 | 4 | 4 | Runtime check failed: compareThreshold (es3fFboMultisampleTests) | fbo.msaa | | §1r group (fbo) |

Not measurable on the host (render verdicts), with their counts on
powervr / vivante / v3d: "Fail" 11,216 / 11,019 / 10,969, "Image comparison
failed" 9,612 / 8,576 / 7,540, "Image compare failed" 4,570 / 4,565 / 4,569,
"Image comparison failed." 1,254 each, "Image verification failed" 1,228 /
796 / 337, "Got faulty pixels" 622 each, "Failed" 582 each (uniform_api.value
549: the value is checked by rendering), "Buffer verification failed" 518
each, "Result comparison failed" 360 / 0 / 0 (shader executor read-back),
"Got unexpected values" 80 each, "Incorrect rasterization" 22, "Invalid
builtin constant value" 22 / 0 / 0, "Found invalid pixel values" 16,
"Incorrect rendering" 12, "Incorrect rendering result" 10, "Quick check fails
on recipient" 6 / 6 / 5, "Found invalid pixels" 4.

The earlier host record (`tools/cts/log/es3-host-run.md`, 522c41f) had a
host-only row for buffer.map (glUnmapBufferOES refused the ES 3.0 targets,
343 + 17 cases). It is gone on main: no case fails at glUnmapBuffer.

## Compile refusals (`cts-refusals.py`, HOST-ONLY)

| GPU | refusals | on cases dEQP failed (the work list) | reasons | on cases dEQP passed (negative tests, correct) |
|---|---:|---:|---:|---:|
| powervr | 2,943 | 912 | 7 | 2,031 in 140 reasons |
| vivante | 2,914 | 883 | 15 | 2,031 in 140 reasons |
| v3d | 5,128 | 3,085 (+12 on NotSupported cases) | 38 | 2,031 in 140 reasons |

The correct refusals are the same front end on every GPU (796 "no implicit
conversion in GLSL ES 1.00", 150 "expected a name", 126 "reserved for future
use", ...). The work lists are the back ends:

**PowerVR** (no ES 3.00 front-end refusal is on a failing case):

| cases | reason | mapping |
|---:|---|---|
| 729 + 105 | `Usc(Select(Unsupported("a texture fetch with an integer result on a cube, a 3D texture or an array, gradients, a projective cube or array, or an offset on a stacked texture, is not selected ...")))` fragment + vertex | §1r rows 9, 12, 13: the integer textures that upload now, sampled on a cube, 3D or array |
| 50 | `Usc(Select(Unsupported("a register read that nothing writes")))` | new (fragment_out.random, shaders.precision) |
| 25 | `Usc(FragmentOutputsNotBuilt)` | new (gl_FragDepth: fbo.depth.depth_test_clamp, shaders.fragdepth) |
| 1 + 1 | a uniform named with 63 'a's, as long as the reflection's 64-byte name field | new (uniform_block.valid.long_instance_name_*) |
| 1 | `Usc(Build(Unsupported("the shader does not fit the buffer")))` | new (uniform_api.random.83) |

The earlier host run's PowerVR rows "lowering into several groups" (modf, 118)
and `Usc(FragmentInputsNotBuilt)` (57) are gone.

**Vivante (GC7000)**: 715 + 92 "an ES 3.00 texture look-up (a shadow sampler,
an integer result, a gradient, an offset or a cube fetch) is not selected
yet"; 25 "this output is not selected yet" (gl_FragDepth); 19 + 19
"textureSize at a level other than a literal 0"; 4 + 1 + 1 constant
registers over 320; 4 `LoadBalancingTempsUnmeasured`; 1
`ValueTooWide("SRC2_REG")`; the two long uniform names. New (GC7000): no
AM62P counterpart.

**V3D** (the V3D compiles at link, "the code generator refused this program
(vertex + fragment)"). New (V3D): no AM62P counterpart.

| cases | reason |
|---:|---|
| 1,520 | `Prepare(Undefined(ValueId(n)))`, 25 values of n: a value used before it is defined in the program the back end is given (builtin_functions, fragment_out, precision, builtin_variable) |
| 998 | `NotYet("a texture look-up other than 2D or cube")` |
| 319 | `NotYet("a shadow look-up")` |
| 133 | `NotYet("an integer texture look-up")` |
| 46 / 25 / 20 / 18 | NotYet: an offset / gl_FragDepth / gradients / textureSize of a 3D or array sampler |
| 8 + 5 | NotYet: a flat / centroid varying past input 31 (12 of them read as NotSupported, above) |
| 2 + 1 | `Select(OutOfScratch)`; the scratch arena |

The earlier host run's largest V3D row, `NotYet` on integer and trunc ALU ops
(3,427 cases), is gone.

## What this says for the next items

- **Colour attachments: 3, ES 3.0 asks for 4.** One constant
  (`CAP_COLOR_ATTACHMENTS`, src/gl/context.rs) is behind the 54 + 13 / 54 + 26
  / 18 + 1 framebuffer-setup failures and two implementation_limits cases on
  every GPU.
- **GC7000 and V3D: integer and sRGB colour targets.** Draws into them are
  refused, which is the largest real bucket on both (1,507 / 705 in the
  shader executor alone). The sRGB refusal also ends a board image after one
  case (the deinit stop above).
- **GC7000 and V3D: integer, snorm and packed-float sampling formats** are
  refused at upload (texture.specification 472 / 618, gluTexture 370 / 514,
  texture.units).
- **V3D: `Prepare(Undefined(..))`** (1,520) and non-2D/cube look-ups (998).
- **V3D: draws refused as GL_OUT_OF_MEMORY** end dEQP's run (330
  ResourceError). A board image would stop at each one.
- **PowerVR: integer look-ups on a cube, 3D or array texture** (834
  refusals); the 1 MiB general arena for large non-indexed draws (48).
