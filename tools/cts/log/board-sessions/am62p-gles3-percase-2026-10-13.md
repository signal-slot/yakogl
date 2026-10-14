<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->

# AM62P board session: dEQP-GLES3 per case, and the depth-PBO trace (2026-10-13)

Item oq-gc of plan #19 (ccfarm `01a13578-d134-74b9-a507-0227e984ec7b`).
Board: TI AM62P on the Toradex Verdin (PowerVR BXS-4-64, BVNC 36.53.104.796).
Build: `ad2b5ed`: main `e51de0af` (the merge of PR #433, 2026-10-13, the
snapshot this item was given) plus two docs-only commits of this item; main
`a63fcc5b` (2026-10-13 23:19) has the same `src/`, `driver/` and `capi/`. Boot:
`uboot-mmc`. Board time: 2026-10-13 23:46 to 2026-10-14 02:19.

**Status: done, with 998 mustpass cases not run** (section "Not run").

## Result

Of the 42,810 mustpass cases (`tools/cts/gles3-mustpass.txt`), 41,812 have a
result:

| | cases |
|---|---:|
| Pass | 36,552 |
| Fail | 2,880 |
| NotSupported | 2,195 |
| QualityWarning | 185 |
| no result | 998 |

The previous full tree, main `f2a13877` (2026-10-11, `tools/cts/RESULTS.md`
"AM62P: the whole dEQP-GLES3 tree on `f2a13877`"), had 32,669 pass, 5,549
fail, 2,318 NotSupported, 185 QualityWarning and 2,089 without a result.
Fail by group, `f2a13877` -> this run:

| group | fail on `f2a13877` | fail here |
|---|---:|---:|
| fragment_ops | 1,109 | 0 |
| transform_feedback | 640 (+ 191 NotSupported) | 0 (+ 0) |
| fbo | 872 | 367 |
| texture | 1,752 (+ 1,646 NotSupported) | 1,351 (+ 1,656) |
| fragment_out | 595 | 583 |
| shaders | 412 | 410 |
| state_query | 26 | 25 |
| rasterization | 21 | 11 |
| the other groups | 122 | 133 |

Between the two builds main gained `c4dce9125` (transform feedback on the
CPU, colour `glBlitFramebuffer` on the CPU, buffer mapping and PBO fixes, sRGB
on the PowerVR), which RESULTS.md lists as not yet run on a board; this is its
first board run. Which of its parts moved which group is not measured here. `negative_api`, `occlusion_query`, `pbo`, `polygon_offset`, `multisample`
and `lifetime` have results this time (image 5 no longer aborts in
`lifetime`).

Every case that is not Pass is listed, one per line, with its first message, in
`am62p-gles3-percase-2026-10-13.notpass.tsv` (next to this file).

### Per group

`info` is the 6 `dEQP-GLES3.info.*` cases.

| group | pass | fail | NotSupported | QualityWarning | no result |
|---|---:|---:|---:|---:|---:|
| attribute_location | 226 | 48 | 0 | 0 | 0 |
| buffer | 695 | 0 | 0 | 0 | 0 |
| clipping | 602 | 0 | 0 | 0 | 0 |
| color_clear | 19 | 0 | 0 | 0 | 0 |
| default_vertex_array_object | 1 | 0 | 0 | 0 | 0 |
| default_vertex_attrib | 80 | 0 | 0 | 0 | 0 |
| depth_stencil_clear | 10 | 0 | 0 | 0 | 0 |
| dither | 20 | 0 | 0 | 0 | 0 |
| draw | 308 | 14 | 0 | 0 | 0 |
| draw_buffers_indexed | 0 | 0 | 76 | 0 | 0 |
| fbo | 1,406 | 367 | 255 | 0 | 0 |
| fence_sync | 10 | 0 | 0 | 0 | 0 |
| fragment_ops | 3,170 | 0 | 0 | 0 | 0 |
| fragment_out | 242 | 583 | 115 | 0 | 0 |
| implementation_limits | 38 | 11 | 0 | 0 | 0 |
| info | 6 | 0 | 0 | 0 | 0 |
| instanced | 39 | 6 | 0 | 0 | 0 |
| lifetime | 55 | 1 | 0 | 0 | 0 |
| multisample | 12 | 4 | 48 | 0 | 0 |
| negative_api | 301 | 7 | 4 | 0 | 0 |
| occlusion_query | 62 | 0 | 0 | 0 | 0 |
| pbo | 43 | 0 | 6 | 0 | 0 |
| polygon_offset | 20 | 0 | 0 | 0 | 0 |
| prerequisite | 3 | 0 | 0 | 0 | 0 |
| primitive_restart | 504 | 0 | 0 | 0 | 0 |
| rasterization | 72 | 11 | 12 | 5 | 0 |
| rasterizer_discard | 51 | 0 | 0 | 0 | 0 |
| read_pixels | 48 | 0 | 0 | 0 | 0 |
| samplers | 42 | 0 | 0 | 0 | 0 |
| shader_api | 38 | 0 | 14 | 0 | 0 |
| shaders | 19,219 | 410 | 0 | 0 | 987 |
| state_query | 912 | 25 | 9 | 0 | 0 |
| texture | 1,907 | 1,351 | 1,656 | 180 | 11 |
| transform_feedback | 882 | 0 | 0 | 0 | 0 |
| ubo | 2,355 | 0 | 0 | 0 | 0 |
| uniform_api | 2,180 | 1 | 0 | 0 | 0 |
| vertex_array_objects | 10 | 0 | 0 | 0 | 0 |
| vertex_arrays | 964 | 41 | 0 | 0 | 0 |
| **total** | **36,552** | **2,880** | **2,195** | **185** | **998** |

### The largest buckets (Fail and QualityWarning)

Grouped by subgroup and first message (numbers in the message folded to N).
This is a sort of the messages, not a root-cause analysis; that is the job of
`docs/es3-cts-failures.md` (plan item 1).

| cases | outcome | subgroup | first message | examples |
|---:|---|---|---|---|
| 323 | Fail | texture.specification | Compile failed at sglrGLContext.cpp | basic_teximage2d.r16i_cube, r16ui_cube |
| 276 | Fail | fragment_out.array | Image comparison failed | array.fixed.srgb8_alpha8_highp_float |
| 244 | Fail | texture.filtering | Image verification failed | 2d_array.combinations.linear_linear_clamp_mirror |
| 228 | Fail | fragment_out.basic | Image comparison failed | basic.int.r16i_highp_int |
| 226 | Fail | shaders.texture_functions | Failed to compile shader program at glsShaderRenderCase.cpp | texelfetch.isampler2darray_fragment |
| 180 | QualityWarning | texture.shadow | Low-quality result | 2d.linear.equal_depth24_stencil8 |
| 150 | Fail | texture.format | Failed to compile shader program at glsTextureTestUtil.cpp | sized.2d_array.r16i_npot |
| 144 | Fail | texture.specification | Image comparison failed | basic_teximage2d.rgba32f_2d |
| 108 | Fail | texture.shadow | Image verification failed | 2d_array.linear.equal_depth24_stencil8 |
| 103 | Fail | texture.units | Compile failed at sglrGLContext.cpp | 2_units.mixed.0 |
| 89 | Fail | texture.vertex | Image comparison failed | 2d_array.filtering.linear_linear_mirror |
| 85 | Fail | texture.mipmap | Image verification failed | 2d.base_level.linear_linear |
| 80 | Fail | texture.swizzle | Image comparison failed | multi_channel.alpha_abgr |
| 75 | Fail | fbo.color | Framebuffer is not complete: GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT | repeated_clear.blit.tex2d.r16i |
| 57 | Fail | fbo.color | Compile failed at sglrGLContext.cpp | tex2darray.r16i |
| 44 | Fail | fbo.render | Framebuffer is not complete: GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT | recreate_color.tex2d_r16i_depth_stencil_rbo_depth24_stencil8 |
| 40 | Fail | fragment_out.random | Image comparison failed | random.1 |
| 40 | Fail | shaders.builtin_functions | Result comparison failed | common.modf.float_highp_fragment |
| 38 | Fail | shaders.texture_functions | Got unexpected texture size | texturesize.isampler2d_fragment |
| 35 | Fail | fbo.completeness | Framebuffer checked is incomplete, expected complete | attachment_combinations.none_rbo_none_none |
| 34 | Fail | fbo.msaa | Got GL_INVALID_OPERATION at esNfFboTestCase.cpp | 2_samples.depth24_stencil8 |

Most of the large Fail buckets name an integer format (`r16i`, `isampler`, ...)
or a 2D array: the two gaps RESULTS.md section 1r already names (integer
colour targets and samplers; 2D-array sampling). The largest NotSupported
buckets are ASTC (1,274 in texture.wrap and texture.compressed) and other
compressed formats (288 in texture.wrap).

## Findings beyond the counts

1. **Image 7 aborts on an arena limit** (HANDOFF "Known defects", "AM62P
   GLES3 image 7"; unchanged). At
   `shaders.builtin_functions.precision.clamp.highp_fragment.vec3` the
   PowerVR back end refused a draw of 24,384 vertices: "a draw ... does not
   fit an empty frame: the general arena (1048576 bytes) is too small; not
   drawn". glDrawArrays raised `GL_OUT_OF_MEMORY`, dEQP reported
   ResourceError, and dEQP stops the whole run on ResourceError ("Test run
   was ABORTED!"). This is why the rest of `builtin_functions.precision`
   (987 cases) has no result. The groups after it in image 7 ran in an extra
   image R7 (`builtin_variable`, `conditionals`, `constant_expressions`: 410
   mustpass cases, which had no result on `f2a13877`).
2. **The 4 resets of image 11 are `shaders.metamorphic.bubblesort_flag`.**
   HANDOFF notes that image 11 needs 4 resets. They come from variant_1
   (3 resets) and variant_2 (1 reset): `FWCCB: CONTEXT_RESET ctxid=3
   reason=1 dm=3`. Both cases Fail, the run goes on, and image 11 ends
   `verdict=done_with_resets resets=4`.
3. **The "GLES3 hang" of images 20 and 21 (HANDOFF "Known defects") is, for
   image 20, the 600 s image lifetime, not a GPU hang.** The image's RTI
   feeder runs for 600 s ("fed for 600 s"). In image 20 the two non-mustpass
   cases `bgra8_linear_mipmap_linear` and `bgra_linear_mipmap_linear` take
   about 4.5 minutes each, so the 14th case,
   `r11f_g11f_b10f_linear_mipmap_linear`, starts at 549 s and the RTI resets
   the board while it runs: the same stop at the same case as on
   `f2a13877`. Run alone (image F20S1), that case completes in 158 s with a
   result (Fail) and no reset line. Image 21's case
   (`3d.sizes.3x7x5_linear_mipmap_linear`) did not run alone (its image F21S
   was not started), so it is not shown for image 21; image 21's own run
   here was cut by a worker restart while `32x64x16_linear_mipmap_linear`
   was running.
4. **texture.filtering.3d is too slow for one image.** Every
   `*_linear_mipmap_linear` case of `3d.formats` and `3d.sizes` is slow (the
   4 that ran took 158, 271, 271 and 272 s), and the filter `3d.formats.*`
   also runs the non-mustpass `bgra*` cases (9 minutes for two of them).
   Image 20 (`3d.formats.*`) hit the 600 s watchdog (`verdict=no_reset`,
   Linux back just after the 603 s bound). Image 21 (`3d.sizes.*`) stopped
   in a `*_linear_mipmap_linear` case too, and the board had rebooted when
   the run was looked at again. Both were split by mustpass names (section
   "Images"). Of the 86 mustpass 3D filtering cases that ran, 76 failed
   (`Image verification failed`) and 10 were NotSupported; none passed.
5. **The depth-PBO cases pass.** See "Depth-PBO trace".

## Depth-PBO trace

`texture.specification.teximage2d_depth_pbo.*` and `teximage3d_depth_pbo.*`
(5 each) failed at `396d667a` with `glGetError() == GL_NO_ERROR`
(RESULTS.md line 998, "Not investigated"). At `ad2b5ed` **all 10 pass**, both
in the sweep image (image 23) and in the trace image PBO1.

The trace image is image 23's code with temporary, uncommitted logging (never
in git): one line at each glTexImage2D/3D call (target, level, internal
format, size, format, type, offset, bound unpack buffer, unpack state, the
held error), one at each PIXEL_UNPACK_BUFFER resolve (`pixel_buffer_resolve`:
buffer size, CPU view, mapped, offset, bytes), and one at every `set_error`.
Each case makes one upload from the buffer at offset 0. The buffer is exactly
as large as the upload, has a CPU view and is not mapped. **No GL error is
raised at all**:

| case | call | internal format / format / type | buffer size = bytes read |
|---|---|---|---:|
| 2d depth24_stencil8 | glTexImage2D 64x128 | 0x88f0 / 0x84f9 / 0x84fa | 32,768 |
| 2d depth32f_stencil8 | glTexImage2D 64x128 | 0x8cad / 0x84f9 / 0x8dad | 65,536 |
| 2d depth_component16 | glTexImage2D 64x128 | 0x81a5 / 0x1902 / 0x1403 | 16,384 |
| 2d depth_component24 | glTexImage2D 64x128 | 0x81a6 / 0x1902 / 0x1405 | 32,768 |
| 2d depth_component32f | glTexImage2D 64x128 | 0x8cac / 0x1902 / 0x1406 | 32,768 |
| 3d depth24_stencil8_2d_array | glTexImage3D 32x64x8 | 0x88f0 / 0x84f9 / 0x84fa | 65,536 |
| 3d depth32f_stencil8_2d_array | glTexImage3D 32x64x8 | 0x8cad / 0x84f9 / 0x8dad | 131,072 |
| 3d depth_component16_2d_array | glTexImage3D 32x64x8 | 0x81a5 / 0x1902 / 0x1403 | 32,768 |
| 3d depth_component24_2d_array | glTexImage3D 32x64x8 | 0x81a6 / 0x1902 / 0x1405 | 65,536 |
| 3d depth_component32f_2d_array | glTexImage3D 32x64x8 | 0x8cac / 0x1902 / 0x1406 | 65,536 |

Not identified: which commit between `396d667a` and `ad2b5ed` fixed it.
`396d667a` is not in this clone, so no bisect was possible here. The RESULTS.md
row can be closed with this record as its evidence.

## Images

All from `ad2b5ed` with a clean tree, `DEQP_MODULE=gles3`
(`-DCONFIG_DEQP_REPORT_ES2=n -DCONFIG_YAKOGL_REPORT_ES2=n`), and the
`check-deqp-images.py --gles3` check passed for each. Images 1 to 28 are the
rows of `tools/cts/am62p-gles3-tree.txt`. The extra images are labelled:

| image | filter | why |
|---|---|---|
| F20A | the 60 mustpass names of `texture.filtering.3d.formats` except `*_linear_mipmap_linear` | image 20 hit the watchdog |
| F21A | the 22 mustpass names of `texture.filtering.3d.sizes` except `*_linear_mipmap_linear` | image 21 is as slow as image 20 |
| F20S1 to F20S4 | one slow `3d.formats.*_linear_mipmap_linear` case each (r11f_g11f_b10f, rgb10_a2, rgb565, rgb5_a1) | up to 4.5 min per case |
| R7 | `shaders.builtin_variable.*`, `conditionals.*`, `constant_expressions.*` | image 7 aborted before them |
| PBO1 | the 10 depth-PBO cases, with the temporary trace logging | the depth-PBO trace |

Verdicts (each run's `INDEX.txt`): every image ended `verdict=done` except:

- image 11: `verdict=done_with_resets resets=4` (finding 2; every case has a
  result);
- image 9: the first run was `done_lossy lost=53` (the capture dropped 53
  result lines); the second run was `done` and is the one counted;
- image 20: `verdict=no_reset` (finding 3); its cases are counted from F20A
  and F20S1 to F20S4 instead;
- image 21: its run was cut by a worker restart before a verdict; its cases
  are counted from F21A.

No other run had a GPU reset line.

## Not run

998 mustpass cases have no result:

- 987 in `shaders.builtin_functions.precision` (finding 1). Running them needs
  the arena limit fixed, or one image per case after the aborting one.
- 11 slow `texture.filtering.3d` cases: `3d.formats.*_linear_mipmap_linear`
  for 7 formats (images F20S5 to F20S11) and `3d.sizes.*_linear_mipmap_linear`
  for 4 sizes (F21S1 to F21S4). The images were built; they were not started
  because of the 2.5-hour limit on board time. Each needs its own image
  (up to about 4.5 min of run time each).

## Console

The lease named `usb-serial-1` port 35932, which carries mcu_uart0 (ttyS3).
Port 35933 of the same bridge carries main_uart0 (ttyS2). A write to ttyS2
over ssh arrived clean on 35933 (the probe table is in
`farm-console-blocked-2026-10.md`, row 2026-10-13). This session used
`BOARD_UART=<bridge>:35933`.

## Inputs

- Firmware: the board's `/lib/firmware/rgx.fw.36.53.104.796`, 229,376 B,
  sha256 `32c8b3d7730a523b…` (the same on the board and in the copy), turned
  into `driver/src/fw/pvr_fw_blob.h` by `driver/tools/mkfwblob.sh` (not
  committed).
- Environment: the values of `pi4-deqp-stage1-gles3.md` ("Environment"),
  all under the farm's dependency cache; VK-GL-CTS at `93bca018`, Zephyr
  `af17c0c8aa2a`.
- Stage 1: `MODULE=gles3 BOARD=verdin_am62p/am62p54/a53
  tools/cts/cts-build-zephyr.sh`, `WORK=<deps>/deqp-stage1-am62p-gles3/work`,
  rc 0 (`libdeqp-gles3-package.a` 132,960 B).
- Images: `DEQP_MODULE=gles3 tools/cts/am62p-deqp-images.sh
  tools/cts/am62p-gles3-tree.txt <out> ad2b5edf`: 28 of 28 `rc=0`.
- Board: `BOARD_BOOT=uboot-mmc BOARD_MUSTPASS=tools/cts/gles3-mustpass.txt
  RETURN_BOUND=300 tools/board.sh am62p tree tools/cts/am62p-gles3-tree.txt
  <rec>`, one row at a time, with `tools/board.sh am62p disarm` before each
  row (after image 20, a run left its staged image behind and the next row
  was refused).
- Records: `<deps>/oq-gc-20261013/rec/{full,split,pbo}` on the worker
  (console.txt, console.raw, INDEX.txt per run); the partial runs of images
  9, 20 and 21 are in `rec/partial` and `rec/refused`.
