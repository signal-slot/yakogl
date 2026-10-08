# The whole dEQP-GLES3 mustpass on the host, three GPUs (2026-10-09)

**Every number in this document is HOST-ONLY.** The library is built with the
LINUX OSAL, which records the command stream and does not execute it: every
glReadPixels reads black, so every render verdict (Pass or Fail decided by an
image, a buffer read back or a pixel count) means nothing and must never be
quoted. What is real is what the GL API returns (errors, queries, NotSupported
decided from our limits and formats) and each shader's compile and link
outcome with our info log (see the header of `tools/cts/host-deqp-run.sh`).
The AM62P column is the board run of `tools/cts/RESULTS.md` §1r, which is not
HOST-ONLY; it is there to be compared with, not added to.

## What ran

- Library and harness: `522c41f` (this branch: `83dcdd5` plus the harness
  changes below), for each of `GPU=powervr`, `vivante` and `v3d`, gles3
  (`YAKOGL_REPORT_ES2=OFF`, the online compiler, `YAKOGL_OSAL=linux`).
- dEQP: VK-GL-CTS `93bca0186` (the revision of §1p/§1r) with
  `tools/cts/cts-yakogl-target.patch`, the checkout of `docs/es3-host-env.md`
  (`$CCFARM_DEPS_DIR/VK-GL-CTS`), deqp-gles3 built by the script into
  `build/host-deqp/deqp-gles3`.
- Case list: the union of `android/cts/main/gles3-main-*.txt`, **42,810 cases**
  (the same list as §1r's mustpass), 429 chunks of 100, 6 chunks in parallel
  per GPU, a 256x256 pbuffer.
- About 14 to 18 minutes per GPU, the three GPUs at once on one 22-thread host.

```sh
CTS=$CCFARM_DEPS_DIR/VK-GL-CTS
for g in powervr vivante v3d; do
    tools/cts/host-deqp-run.sh --cts="$CTS" --gpu=$g --module=gles3 --jobs=6 \
        --out=build/host-deqp/full-$g > build/full-$g.log 2>&1
done
# the report again, from a finished run:
tools/cts/host-deqp-run.sh --cts="$CTS" --report=build/host-deqp/full-v3d
```

Each run directory holds `summary.txt` (`cts-summary.py --by-group 3
--by-message`: status totals, the group table, every failure message with its
count), `failures.tsv` (every failing case with its message), `aborted.txt`
(below) and the `.qpa` logs. The refusal work list is the part of the run log
that `cts-refusals.py` writes.

## Begun against asked (HOST-ONLY)

| GPU | asked | began | never began | processes | chunks resumed | ResourceError |
|---|---:|---:|---:|---:|---:|---:|
| powervr | 42,810 | 42,754 | 56 | 486 | 1 | 0 |
| vivante | 42,810 | 42,754 | 56 | 486 | 1 | 0 |
| v3d | 42,810 | 42,754 | 56 | 667 | 22 | 183 |

The 56 never began are all of `lifetime.*`, on every GPU, for the cause §1r
gives: the group's init links a program with transform feedback of
gl_Position (es3fLifetimeTests.cpp:97, `TCU_CHECK(isOk())`), transform
feedback is not implemented, and dEQP ends the process with die(). On the host
that is exit 137, between two cases, so no case is open when the process ends.

### The chunks that stopped early, by case

The only exit 134/137 chunk, on all three GPUs, is the lifetime one:

| GPU | chunk | exits | began | where each process stopped |
|---|---|---|---:|---|
| all three | c_083 | 137 x57, 0 | 44 of 100 | after `instanced.types.vec4` (the next case, `lifetime.attach.deleted_input.renderbuffer_framebuffer`, never began); then 56 processes each died before its first case, one `lifetime.*` case skipped each time; the 57th ran the 15 cases after `lifetime.*` (`multisample.*`, to `multisample.default_framebuffer.sample_coverage_invert`) |

No process crashed in a case (no "in <case>" stop, no panic line) and none
reached the 600 s timeout, on any GPU.

On the V3D, 22 more chunks were resumed (exit 1 each time, 238 extra
processes). Each stop is a case that ended with **ResourceError**, which ends
dEQP's whole run. The V3D back end refuses a draw it cannot record ("draw
refused: an R16F colour target is not written yet", "the pending clear was
not recorded (-22)") and the context reports GL_OUT_OF_MEMORY, which dEQP
takes as a resource error. The chunks (`aborted.txt` lists every stop):

| chunk | processes | the cases the processes stopped after (groups) |
|---|---:|---|
| c_036 | 4 | fbo.invalidate.format.{r16f,rg16f,rgba16f}, fbo.invalidate.target |
| c_040 | 4 | fbo.render.shared_colorbuffer_clear.tex2d_{r16f,rg16f,rgba16f}, fbo.render.shared_depth_stencil |
| c_074 | 10 | fragment_out.array.float.{r16f,rg16f,rgba16f}_* (9), fragment_out.array.int |
| c_078 | 10 | fragment_out.basic.float.{r16f,rg16f,rgba16f}_* (9), rgba32f_lowp_float |
| c_081 | 2 | fragment_out.random.3, .67 |
| c_088 | 5 | pbo.renderbuffer.{rg16f,rgba16f}_{clears,triangles}, primitive_restart |
| c_131, c_132, c_133 | 6 each | shaders.derivate.{dfdx,dfdy,fwidth}.texture.basic.* |
| c_296 .. c_299 | 7, 5, 10, 13 | shaders.texture_functions.{texture,texturelod,textureproj,texturesize}.* |
| c_313, c_317 | 7 each | texture.filtering.{2d,cube}.formats.*16f* |
| c_318, c_319, c_322 | 7, 7, 9 | texture.format.compressed.etc2_*, texture.format.sized.{2d,cube}.*16f* |
| c_330 | 4 | texture.size.cube.512x512_* |
| c_358, c_359 | 36 each | texture.wrap.etc2_eac_rgba8.*, texture.wrap.etc2_rgb8.* |

Before this branch's harness fix those 22 chunks lost every case after their
first ResourceError: the first full V3D run began 41,464 of 42,810 and showed
no chunk as stopped (below).

## The groups (HOST-ONLY except the AM62P column)

p / f / ns / other / none = Pass / Fail / NotSupported / any other status
(V3D: 183 ResourceError and 7 QualityWarning) / never began. **The Pass and
Fail columns of the render groups mean nothing on the host**; compare only
NotSupported, the API groups and the refusal list below.

| group | cases | AM62P §1r p / f / ns / other | powervr p / f / ns / other / none | vivante p / f / ns / other / none | v3d p / f / ns / other / none |
|---|---:|---|---|---|---|
| attribute_location | 274 | 226 / 48 / 0 / 0 | 226 / 48 / 0 / 0 / 0 | 226 / 48 / 0 / 0 / 0 | 189 / 85 / 0 / 0 / 0 |
| buffer | 695 | 695 / 0 / 0 / 0 | 42 / 653 / 0 / 0 / 0 | 42 / 653 / 0 / 0 / 0 | 42 / 653 / 0 / 0 / 0 |
| clipping | 602 | 602 / 0 / 0 / 0 | 0 / 602 / 0 / 0 / 0 | 0 / 602 / 0 / 0 / 0 | 0 / 602 / 0 / 0 / 0 |
| color_clear | 19 | 19 / 0 / 0 / 0 | 0 / 19 / 0 / 0 / 0 | 0 / 19 / 0 / 0 / 0 | 0 / 19 / 0 / 0 / 0 |
| default_vertex_array_object | 1 | 1 / 0 / 0 / 0 | 1 / 0 / 0 / 0 / 0 | 1 / 0 / 0 / 0 / 0 | 1 / 0 / 0 / 0 / 0 |
| default_vertex_attrib | 80 | 64 / 16 / 0 / 0 | 0 / 80 / 0 / 0 / 0 | 0 / 80 / 0 / 0 / 0 | 0 / 80 / 0 / 0 / 0 |
| depth_stencil_clear | 10 | 10 / 0 / 0 / 0 | 0 / 10 / 0 / 0 / 0 | 0 / 10 / 0 / 0 / 0 | 0 / 10 / 0 / 0 / 0 |
| dither | 20 | 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 / 0 |
| draw | 322 | 272 / 50 / 0 / 0 | 2 / 320 / 0 / 0 / 0 | 2 / 320 / 0 / 0 / 0 | 2 / 320 / 0 / 0 / 0 |
| draw_buffers_indexed | 76 | 0 / 0 / 76 / 0 | 0 / 0 / 76 / 0 / 0 | 0 / 0 / 76 / 0 / 0 | 0 / 0 / 76 / 0 / 0 |
| fbo | 2028 | 657 / 1116 / 255 / 0 | 373 / 1400 / 255 / 0 / 0 | 374 / 1399 / 255 / 0 / 0 | 374 / 1393 / 255 / 6 / 0 |
| fence_sync | 10 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 / 0 | 10 / 0 / 0 / 0 / 0 | 10 / 0 / 0 / 0 / 0 |
| fragment_ops | 3170 | 2059 / 1111 / 0 / 0 | 89 / 3081 / 0 / 0 / 0 | 89 / 3081 / 0 / 0 / 0 | 89 / 3081 / 0 / 0 / 0 |
| fragment_out | 940 | 206 / 619 / 115 / 0 | 0 / 825 / 115 / 0 / 0 | 0 / 825 / 115 / 0 / 0 | 0 / 806 / 115 / 19 / 0 |
| implementation_limits | 49 | 33 / 16 / 0 / 0 | 35 / 14 / 0 / 0 / 0 | 38 / 11 / 0 / 0 / 0 | 35 / 14 / 0 / 0 / 0 |
| instanced | 45 | 39 / 6 / 0 / 0 | 0 / 45 / 0 / 0 / 0 | 0 / 45 / 0 / 0 / 0 | 0 / 45 / 0 / 0 / 0 |
| lifetime | 56 | 0 / 0 / 0 / 0 (56 no result) | 0 / 0 / 0 / 0 / 56 | 0 / 0 / 0 / 0 / 56 | 0 / 0 / 0 / 0 / 56 |
| multisample | 64 | 0 / 16 / 48 / 0 | 0 / 16 / 48 / 0 / 0 | 0 / 16 / 48 / 0 / 0 | 0 / 16 / 48 / 0 / 0 |
| negative_api | 312 | 277 / 31 / 4 / 0 | 268 / 40 / 4 / 0 / 0 | 270 / 38 / 4 / 0 / 0 | 270 / 38 / 4 / 0 / 0 |
| occlusion_query | 62 | 0 / 62 / 0 / 0 | 0 / 62 / 0 / 0 / 0 | 0 / 62 / 0 / 0 / 0 | 0 / 62 / 0 / 0 / 0 |
| pbo | 49 | 17 / 26 / 6 / 0 | 0 / 43 / 6 / 0 / 0 | 0 / 43 / 6 / 0 / 0 | 0 / 39 / 6 / 4 / 0 |
| polygon_offset | 20 | 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 / 0 | 0 / 20 / 0 / 0 / 0 |
| prerequisite | 3 | 3 / 0 / 0 / 0 | 2 / 1 / 0 / 0 / 0 | 2 / 1 / 0 / 0 / 0 | 2 / 1 / 0 / 0 / 0 |
| primitive_restart | 504 | 504 / 0 / 0 / 0 | 504 / 0 / 0 / 0 / 0 | 504 / 0 / 0 / 0 / 0 | 504 / 0 / 0 / 0 / 0 |
| rasterization | 100 | 62 / 21 / 12 / 5 | 29 / 59 / 12 / 0 / 0 | 29 / 59 / 12 / 0 / 0 | 29 / 59 / 12 / 0 / 0 |
| rasterizer_discard | 51 | 0 / 51 / 0 / 0 | 0 / 51 / 0 / 0 / 0 | 0 / 51 / 0 / 0 / 0 | 0 / 51 / 0 / 0 / 0 |
| read_pixels | 48 | 24 / 24 / 0 / 0 | 0 / 48 / 0 / 0 / 0 | 0 / 48 / 0 / 0 / 0 | 0 / 48 / 0 / 0 / 0 |
| samplers | 42 | 42 / 0 / 0 / 0 | 42 / 0 / 0 / 0 / 0 | 42 / 0 / 0 / 0 / 0 | 28 / 14 / 0 / 0 / 0 |
| shader_api | 52 | 38 / 0 / 14 / 0 | 38 / 0 / 14 / 0 / 0 | 38 / 0 / 14 / 0 / 0 | 38 / 0 / 14 / 0 / 0 |
| shaders | 20616 | 18385 / 2231 / 0 / 0 | 2238 / 18378 / 0 / 0 / 0 | 2231 / 18385 / 0 / 0 / 0 | 2238 / 18324 / 0 / 54 / 0 |
| state_query | 946 | 874 / 63 / 9 / 0 | 872 / 65 / 9 / 0 / 0 | 878 / 59 / 9 / 0 / 0 | 871 / 66 / 9 / 0 / 0 |
| texture | 5105 | 835 / 2612 / 1658 / 0 | 16 / 3431 / 1658 / 0 / 0 | 16 / 3431 / 1658 / 0 / 0 | 16 / 3324 / 1658 / 107 / 0 |
| transform_feedback | 882 | 0 / 461 / 421 / 0 | 0 / 461 / 421 / 0 / 0 | 0 / 442 / 440 / 0 / 0 | 0 / 441 / 441 / 0 / 0 |
| ubo | 2355 | 2168 / 187 / 0 / 0 | 0 / 2355 / 0 / 0 / 0 | 0 / 2355 / 0 / 0 / 0 | 0 / 2355 / 0 / 0 / 0 |
| uniform_api | 2181 | 2136 / 27 / 18 / 0 | 1581 / 600 / 0 / 0 / 0 | 1580 / 601 / 0 / 0 / 0 | 1581 / 600 / 0 / 0 / 0 |
| vertex_array_objects | 10 | 10 / 0 / 0 / 0 | 10 / 0 / 0 / 0 / 0 | 10 / 0 / 0 / 0 / 0 | 10 / 0 / 0 / 0 / 0 |
| vertex_arrays | 1005 | 964 / 41 / 0 / 0 | 40 / 965 / 0 / 0 / 0 | 40 / 965 / 0 / 0 / 0 | 40 / 965 / 0 / 0 / 0 |
| info | 6 | 6 / 0 / 0 / 0 | 6 / 0 / 0 / 0 / 0 | 6 / 0 / 0 / 0 / 0 | 6 / 0 / 0 / 0 / 0 |
| TOTAL | 42810 | **31278 / 8835 / 2636 / 5** | **6424 / 33712 / 2618 / 0 / 56** | **6428 / 33689 / 2637 / 0 / 56** | **6375 / 33551 / 2638 / 190 / 56** |

What the columns can say:

- **NotSupported agrees with the AM62P.** The PowerVR host run has 2,618
  against §1r's 2,636. Group by group the counts are equal (texture 1,658, fbo
  255, transform_feedback 421, fragment_out 115, multisample 48, ...) except
  uniform_api: 18 on the board, 0 here. The Vivante and V3D runs differ only in
  transform_feedback (440 and 441): the PowerVR refuses 48 of its programs at
  compile time ("Compile failed", es3fTransformFeedbackTests.cpp:1043,
  random_full_array_capture 30 and interpolation 18), the GC7000 2 and the V3D
  none, so on those two the cases go on to dEQP's limit check (19 and 20 more
  NotSupported) or to glTransformFeedbackVaryings (27 and 28 more failures).
- The 6,400 Pass are API and compile cases (state_query, negative_api,
  uniform_api, shaders.* negative and compile-only cases, primitive_restart's
  API checks, ...) plus whatever a black read-back happens to satisfy. They are
  not a pass rate.

## Failures by message, mapped onto §1r (HOST-ONLY)

The per-message tally is `summary.txt`'s "failures by message", dEQP's own
detail string. The host's strings carry dEQP's "glGetError() returned" and
source line; §1r folded them. A §1r message whose source line differs between
cases is the sum of the host rows for that file. §1r is the PowerVR board, so
compare its column with powervr first.

### §1r's sixteen largest messages

| §1r cases | §1r message | powervr | vivante | v3d | reading |
|---:|---|---:|---:|---:|---|
| 1,483 | Failed to set up framebuffer object: GL_INVALID_ENUM (glsShaderExecUtil.cpp:670) | **1,483** | 1,561 | 639 | SAME on PowerVR (integer renderbuffers refused). The V3D refuses more of those programs at compile time first (below) |
| 1,293 | Image compare failed | 4,570 | 4,565 | 4,569 | render verdict: not measurable on the host |
| 736 | Runtime check failed: glGetError() == GL_NO_ERROR (es3fTextureSpecificationTests) | 720 | 720 | 720 | same bucket (texture.specification, all at line 220); 16 fewer, not split |
| 675 | Image comparison failed | 7,931 (+1,254 "Image comparison failed.") | 7,965 (+1,254) | 7,054 (+1,254) | render verdict: not measurable |
| 531 | After framebuffer setup: GL_INVALID_ENUM (es3fFragmentOutputTests) | **531** | 557 | 124 | SAME on PowerVR |
| 522 | Texture upload failed: GL_INVALID_OPERATION (gluTexture) | **522** (lines 188, 755, 633, 418: 168 + 134 + 119 + 101) | 522 | 522 | SAME on every GPU (integer and packed-float formats) |
| 413 | glTransformFeedbackVaryings(): GL_INVALID_VALUE | **413** | 440 | 441 | SAME on PowerVR; on the other two, programs the PowerVR refuses at compile time reach the call (above) |
| 298 | Image verification failed | 1,084 | 656 | 337 | render verdict: not measurable |
| 292 | Failed to compile shader program (glsTextureTestUtil) | 4 | 432 | 765 | FEWER on the PowerVR: the 432 texture.shadow cases now compile (they end in "Image verification failed", a render verdict); the GC7000 and V3D still refuse shadow look-ups (refusal list) |
| 270 | Got GL_INVALID_ENUM (es3fFboTestCase) | **270** | 270 | 270 | SAME on every GPU (fbo.blit 204, fbo.color 64, fbo.msaa 2) |
| 252 | Got GL_INVALID_OPERATION (es3fFboTestCase) | **252** | 252 | 177 | SAME (fbo.blit 193, fbo.msaa 32, fbo.color 27); on the V3D 83 of these cases get GL_OUT_OF_MEMORY instead (host-only row below) |
| 189 | Failed to compile shader program (glsShaderRenderCase) | 187 | 178 | 4,216 | same bucket on PowerVR (modf's "lowering into several groups", 3D bias); the V3D's integer-op refusals |
| 181 | Compile failed (sglrGLContext) | **181** | 181 | 347 | SAME on PowerVR |
| 159 | Texture upload failed: GL_INVALID_ENUM (gluTexture) | 0 | 0 | 0 | GONE on the host: depth texture formats upload. Whether that is a change since 396d667a or a host-path difference is not separated |
| 133 | Fail (no message) | 11,010 | 11,019 | 6,940 | render verdict: not measurable |
| 103 | checkFramebufferStatus == COMPLETE (es3fShaderDerivateTests) | **103** | 103 | 103 | SAME on every GPU (float render targets) |

### API messages below §1r's cut (its other 62 messages are not in RESULTS.md)

These are real on the host. A row whose group failed on the AM62P is marked
"§1r group"; one whose group passed there is **host-only/new**.

| powervr | vivante | v3d | message | groups | mapping |
|---:|---:|---:|---|---|---|
| 343 | 343 | 343 | glUnmapBuffer: GL_INVALID_ENUM (es3fBufferMapTests, glsBufferTestUtil: 9 source lines) | buffer.map | **host-only/new**: buffer passed 695/695 on the AM62P. Cause below |
| 17 | 17 | 13 | glUnmapBuffer(GL_PIXEL_PACK_BUFFER): GL_INVALID_ENUM (es3fPixelBufferObjectTests:584) | pbo.renderbuffer, pbo.native | **host-only/new**, the same cause |
| 96 / 60 / 5 | 96 / 60 / 5 | 96 / 60 / 5 | Expected no error but got GL_INVALID_OPERATION / _ENUM / _VALUE | fbo.completeness | §1r group (fbo) |
| 75 | 75 | 75 | GL_INVALID_OPERATION (es3fFboTestCase:90) | fbo.blit 67, fbo.invalidate 8 | §1r group (fbo) |
| 62 | 62 | 62 | Occlusion query started: GL_INVALID_OPERATION | occlusion_query | §1r group: all 62 failed on the AM62P too (docs/es3-occlusion-query.md) |
| 54 | 52 | 52 | Got invalid error | negative_api (all 40 of its PowerVR failures), state_query 14 | §1r groups (negative_api failed 31 there, 40 / 38 / 38 here; not split) |
| 50 + 18 | 50 + 18 | 50 + 18 | Texture upload failed: GL_INVALID_VALUE (gluTexture) | texture.filtering, texture.format | §1r group (texture) |
| 139 + 8 | 139 + 8 | 139 + 8 | Set {2d,cube,2d array,3d} texture image data: GL_INVALID_OPERATION + GL_INVALID_VALUE | texture.units | §1r group (texture) |
| 26 + 24 | 26 + 24 | 26 + 24 | glRenderbufferStorage: GL_INVALID_ENUM (es3fPixelBufferObjectTests, es3fReadPixelsTests RGBA32I/UI) | pbo.renderbuffer, read_pixels | §1r groups (pbo, read_pixels) |
| 40 | 64 | 16 | Post framebuffer setup: GL_INVALID_ENUM (es3fShaderPrecisionTests) | shaders.precision | §1r group (shaders) |
| 39 | 28 | 28 | Framebuffer is not complete: INCOMPLETE_ATTACHMENT (es3fFboTestCase) | fbo.invalidate, fbo.color | §1r group (fbo) |
| 23 / 16 | 23 / 15 | 23 / 15 | Framebuffer checked as complete, expected incomplete / the reverse | fbo.completeness | §1r group (fbo) |
| 20 | 20 | 20 | glDrawArrays(): GL_INVALID_OPERATION (glsVertexArrayTests) | vertex_arrays.single_attribute | §1r group (41 there) |
| 16 | 16 | 16 | glBlitFramebuffer: GL_INVALID_OPERATION (es3fMultisampleTests) | multisample.fbo_max_samples | §1r group: the 16 multisample failures |
| 12 | 12 | 12 | blit: GL_INVALID_OPERATION (es3fRasterizationTests) | rasterization.fbo | §1r group |
| 15 / 14 / 11 / 4 | 12 / 11 / 11 / 4 | 15 / 14 / 11 / 4 | Got invalid integer / float / boolean value, expected GL_TRUE | state_query | §1r group (63 there; 65 / 59 / 66 here) |
| 0 | 0 | 83 + 7 | Got GL_OUT_OF_MEMORY (es3fFboTestCase:154, :90) | fbo.blit 66, fbo.color 24 | **host-only/new (V3D)**: refused draws reported as GL_OUT_OF_MEMORY |
| 0 | 0 | 183 | ResourceError, every one GL_OUT_OF_MEMORY | texture.wrap 72, shaders.texture_functions 32, texture.format 20, fragment_out 19, shaders.derivate 15, texture.filtering 12, fbo.invalidate/render 6, pbo.renderbuffer 4, texture.size 3 | **host-only/new (V3D)**, the same cause; each one also ended its process (the resumed chunks above) |

**The buffer.map row, read in the code.** dEQP's ES loader replaces the
core `glMapBuffer`, `glUnmapBuffer` and `glGetBufferPointerv` with the
`...OES` entry points whenever the extension string lists `GL_OES_mapbuffer`
(VK-GL-CTS `framework/opengl/wrapper/glwInitExtES.inl:179-184`), also in an
ES 3.0 context. Our `glUnmapBufferOES` accepts only the extension's ES 2.0
targets (`src/gl/buffer.rs` `oes_target`: GL_ARRAY_BUFFER and
GL_ELEMENT_ARRAY_BUFFER) and sets GL_INVALID_ENUM for any other, so every
`buffer.map` case that maps GL_COPY_READ_BUFFER, GL_PIXEL_PACK_BUFFER, ... fails at
the unmap. GL_OES_mapbuffer landed on 2026-10-04 (RESULTS.md, "GL_OES_mapbuffer,
gl_test case 176"), after §1r's 396d667a, so this is a regression the board
would show too, not a host artefact. Not fixed here: `oes_map_errors` pins the
current reading of the extension, and choosing between accepting every ES 3.0
target in the OES entry points and not advertising the extension on an ES 3.0
context is the GL core owner's call.

The rest of the message list (render verdicts: "Fail", "Image comparison
failed", "Buffer verification failed" 344, "Got faulty pixels" 622, "Got
unexpected values", "Incorrect rasterization", ...) is in each
`summary.txt` and is not measurable here.

## Compile refusals (`cts-refusals.py`, HOST-ONLY)

| GPU | refusals | on cases dEQP failed (the work list) | reasons | on cases dEQP passed (negative tests, correct) |
|---|---:|---:|---:|---:|
| powervr | 2,650 | 617 | 11 | 2,033 in 138 reasons |
| vivante | 2,901 | 883 | 15 | 2,018 in 137 reasons |
| v3d | 9,432 | 7,414 | 47 | 2,018 in 137 reasons |

The correct refusals are the same front end on every GPU (796 "no implicit
conversion in GLSL ES 1.00", 150 "expected a name", 126 "reserved for future
use", ...). The work lists are the back ends:

**PowerVR** (§1r rows 9 and 12 are these refusals on the board):

| cases | reason | mapping |
|---:|---|---|
| 277 + 85 | Usc(Select(Unsupported("a texture fetch with a projective level or bias, a bias on a 3D texture, a shadow compare, an integer result or gradients, or an offset on a stacked texture, ..."))) fragment + vertex | §1r rows 9/12 and the integer-format sampling (fbo.color.repeated_clear.sample.*i, texture_functions isampler/usampler); shadow compare is no longer the cause in texture.shadow |
| 59 + 59 | Usc(Build(Unsupported("lowering into several groups"))) | §1r row 12 (shaders.builtin_functions.common.modf) |
| 57 | Usc(FragmentInputsNotBuilt), also on 15 PASSING cases | host-only/new as a row (shaders.linkage.varying.interpolation.centroid, rules.differing_interpolation_*): one message, two causes |
| 50 | Usc(Select(Unsupported("a register read that nothing writes"))) | host-only/new (fragment_out.random.*) |
| 25 | Usc(FragmentOutputsNotBuilt) | host-only/new (fbo.depth.depth_test_clamp.*: gl_FragDepth) |
| 2 | Usc(Select(Unsupported("exact gradients without the W plane"))) | host-only/new (texture_functions sampler2dshadow/samplercubeshadow) |
| 1 + 1 | a uniform named with 63 'a's, as long as the reflection's 64-byte name field | host-only/new (uniform_block.valid.long_instance_name_*) |
| 1 | Usc(Build(Unsupported("the shader does not fit the buffer"))) | host-only/new (uniform_api.random.83) |

**Vivante (GC7000)**: 715 + 92 "an ES 3.00 texture look-up (a shadow sampler,
an integer result, a gradient, an offset or a cube fetch) is not selected
yet"; 25 "this output is not selected yet" (gl_FragDepth); 19 + 19
"textureSize at a level other than a literal 0"; 4 + 1 + 1 constant registers
over 320 (ubo, uniform_api.random); 4 LoadBalancingTempsUnmeasured
(large_constant_arrays, transform_feedback.random); 1 ValueTooWide("SRC2_REG");
the two long uniform names. No AM62P counterpart: host-only/new for this GPU.

**V3D** (no AM62P counterpart: host-only/new for this GPU), folded by family:

| cases | reason |
|---:|---|
| 3,427 | NotYet(an integer or trunc ALU op): Inot 1,880, Ishl 518, Ushr 256, Ishr 252, Imax 158, Umax 112, Ftrunc 100, Imin 56, Umin 56, Ult 27, Ule 12 |
| 1,465 | Prepare(Undefined(ValueId(n))), 23 values of n: a value used before it is defined in the program the back end is given (fragment_out.*, builtin_functions.common/precision) |
| 1,052 | NotYet("an attribute location with no declaration") |
| 888 | NotYet("a texture look-up other than 2D or cube") |
| 319 | NotYet("a shadow look-up") |
| 133 | NotYet("an integer texture look-up") |
| 46 / 25 / 20 / 18 / 16 | NotYet: an offset / gl_FragDepth / gradients / textureSize of 3D or array / centroid varying |
| 2 + 1 | Select(OutOfScratch); the scratch arena (ubo.random.all_per_block_buffers.28) |

## Harness changes made for this run

Each of these was found by the run itself:

1. **A ResourceError lost the rest of its chunk, unreported.** dEQP ends its
   run on a ResourceError with exit 1, the status of any chunk with a failing
   case, so the first full V3D run (`b88def6`'s parent `9b247f0`) began 41,464
   of 42,810 and listed no stopped chunk. `run_chunk` now goes on in a fresh
   process with the cases not yet begun (`c_NNN.r<k>.qpa`); a process that
   begins none of them (lifetime's init die()) skips the first one
   (`c_NNN.skipped`). The report (`aborted.txt`) lists every chunk that needed
   more than one process or lost cases, with each process's exit and where
   it stopped, by case. Before, the lifetime chunk read "exit=137" with
   no case and took the 15 multisample cases after `lifetime.*` with it.
2. **A 64x64 surface made about 2,000 cases NotSupported before they compiled
   anything**: all 602 of clipping ("Render target size must be at least
   200x200"), 1,221 "Too small viewport" (texture.specification,
   texture.mipmap), 90 "Viewport size 128x128 required" and more. gles3 now
   runs at 256x256, as the Android mustpass does; gles2 stays at 64x64 so its
   earlier host runs stay comparable. Texture's NotSupported went from 2,895 to
   1,658, the AM62P's number.
3. **The tables the doc needs, from committed tools**: `cts-summary.py
   --by-message N` (cts-selftest check 19), and `host-deqp-run.sh` writes
   `summary.txt`, `failures.tsv` and `aborted.txt` into the run directory.
   `--jobs=` and `--report=DIR` join the option list (the farm sandbox blocks
   environment-prefixed commands).
