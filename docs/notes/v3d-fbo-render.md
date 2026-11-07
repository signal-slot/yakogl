<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: the Pi 4 `fbo.render.*` and `fbo.color.*` rows, by cause

Research, no code change. `docs/es3/pi4-gles3-fails.tsv` lists 511 not-pass
rows in `dEQP-GLES3.functional.fbo.render.*` (305) and `fbo.color.*` (206).
This note puts every one of them in exactly one set, says which note or
layer explains the set, and designs the `src/v3d` part of each cause that is
left. Three things in the data limit what can be said, and the sections
below say where each bites:

- **The rows are names and a status, nothing else.** The tsv has no failure
  text, and the consoles of the nine images are not in the tree
  (`tools/cts/log/` holds the FO2-SQ and S01-S32 runs only). Which half of a
  row failed (the framebuffer check, a GL error, the picture) is read here
  from the dEQP source and from a host replay, not from the board.
- **The run is `925dbca9` (2026-10-22), which is not an ancestor of main.**
  Main (`434d2695`) has since gained, among other things, the V3D integer
  clear (`e883be8f`), the sRGB render target, the GL core's integer texture
  attachments and 16 render images. A row can have moved without anyone
  having looked.
- **dEQP is VK-GL-CTS `main` (`c9835145`), shallow-cloned**; the board image
  is built from `93bca01` (`docs/es3/board-recipe-pi4.md`). The case names
  and the two test files read here are the same in both; a later change to a
  test body would not show.

Function names are the address; line numbers are of main at `434d2695` and
drift. Mesa is 26.2.1, `third_party/mesa-26.2.1` (the format table is
vendored; `v3dx_format_table.c:220-345` is read there).

## 1. The accounting

Every row is in one set; the sets are tested in the order A, B, C, E1, E2a,
E2b, E3, E4a, E4b (a row of a set is not counted again below it). D is the
three notes that explain no row of these two groups, with the reason.

| set | rows | what | explained by | owning layer |
|---|---|---|---|---|
| A | 63 | `NotSupported`: `r32f`, `rg32f`, `rgba32f`, `r11f_g11f_b10f` in every group (dEQP's `checkFormatSupport` wants `GL_EXT_color_buffer_float` for them, `es3fFboTestCase.cpp:268`, `es3fFboRenderTest.cpp:178`) | the GL core does not list the extension (`src/gl/state.rs:111` lists half-float only); `docs/es3-cts-failures.md` RC1 "Float targets" | `src/gl`, a decision; a `NotSupported` is not a `Fail` |
| B | 21 | `srgb8_alpha8` in every group | `docs/es3/srgb.md`; the V3D refused the sRGB draw at `925dbca9` (`state.rs` "an sRGB colour target is not written yet", `SRGB_RT` was PowerVR-only); main draws it and `gl_test` 204 passes on the Pi 4 at `67b27e26` (`tools/cts/log/2026-11-pi4-gltest/README.md`) | `src/v3d` (done on the host, #1234 / #1235) |
| C | 14 | `rgb10_a2` (not `rgb10_a2ui`): `shared_colorbuffer` 6, `shared_colorbuffer_clear` 2, `resize` 2, `recreate_color` 2, `tex2darray` 1, `tex3d` 1 | `docs/notes/v3d-rgb10a2.md` (held as RGBA8 today, the class is behind the gate) | `src/v3d` + `src/gl` |
| D | 0 | the 4x multisampling note, the different-size note, the several-attachment clear | neither source file calls `glDrawBuffers`, `glRenderbufferStorageMultisample` or attaches past `COLOR_ATTACHMENT0`; each case renders one single-sampled attachment of one size (a case's two framebuffers differ in size but are never bound together). `glBlitFramebuffer` (two `repeated_clear.blit` cases) runs on the CPU (`src/gl/blit.rs`) | `v3d-msaa-es3.md`, `v3d-mrt-sizes.md` and `v3d-mrt-clear` move 0 rows here |
| E1 | 61 | moved since the run: 19 integer `color.clear` (the scissored integer clear, refused at `925dbca9` as "a scissored or masked colour clear of an integer colour target", `CLEAR_FS_INT` now) and the 7 four-channel integer formats (`rgba8i`, `rgba8ui`, `rgba16i`, `rgba16ui`, `rgba32i`, `rgba32ui`, `rgb10_a2ui`) as texture attachments in six groups (42; the GL core had no integer texture attachments then, `framebuffer.rs` `integer_texture_internalformat`) | a Pi 4 rerun | none: rerun only |
| E2a | 96 | `r8i r8ui r16i r16ui r32i r32ui rg8i rg8ui rg16i rg16ui rg32i rg32ui` as **texture** attachments, 8 groups of 12 | section 3 (D1) | `src/gl` classes, then `src/v3d` mask bits, TMU rows and narrow surface |
| E2b | 12 | `rgb16f` as a texture attachment (`tex2d`, `texcube`, `blit.tex2d`, `sample`, `tex2darray`, `tex3d`, `resize`, `recreate_color`: 8; `shared_colorbuffer` 3; `shared_colorbuffer_clear` 1) | section 5 (D5) | `src/gl` |
| E3 | 34 | `tex2darray` 17 and `tex3d` 17 of the formats whose layered framebuffer is complete | section 4 (D2): the sampling of a 2D-array or 3D texture is refused twice | `src/shaderc` + `src/v3d` |
| E4a | 3 | `color.blend` `r8`, `rg8`, `rg16f` | section 6 (D4): the channels the format lacks are written | `src/gl` + `src/v3d` |
| E4b | 207 | every other `fbo.render.*` row: `recreate_color` 37, `resize` 62, `shared_colorbuffer` 63, `shared_colorbuffer_clear` 21, `recreate_depth_stencil` 11, `shared_depth_stencil` 10, `stencil_clear` 3 | section 7: **no cause found**; nothing the host sees refuses them | open |
| | **511** | = 206 `fbo.color` + 305 `fbo.render` | | |

The same by group (A B C E1 E2a E2b E3 E4a E4b, columns of the table above):

| group | rows | A | B | C | E1 | E2a | E2b | E3 | E4a | E4b |
|---|---|---|---|---|---|---|---|---|---|---|
| `color.blend` | 5 | 1 | 1 | | | | | | 3 | |
| `color.clear` | 24 | 4 | 1 | | 19 | | | | | |
| `color.repeated_clear.blit.rbo` | 5 | 4 | 1 | | | | | | | |
| `color.repeated_clear.blit.tex2d` | 25 | 4 | 1 | | 7 | 12 | 1 | | | |
| `color.repeated_clear.sample.tex2d` | 25 | 4 | 1 | | 7 | 12 | 1 | | | |
| `color.tex2d` | 25 | 4 | 1 | | 7 | 12 | 1 | | | |
| `color.texcube` | 25 | 4 | 1 | | 7 | 12 | 1 | | | |
| `color.tex2darray` | 36 | 4 | 1 | 1 | | 12 | 1 | 17 | | |
| `color.tex3d` | 36 | 4 | 1 | 1 | | 12 | 1 | 17 | | |
| **`fbo.color`** | **206** | 33 | 9 | 2 | 47 | 72 | 6 | 34 | 3 | |
| `render.recreate_color` | 69 | 8 | 2 | 2 | 7 | 12 | 1 | | | 37 |
| `render.recreate_depth_stencil` | 11 | | | | | | | | | 11 |
| `render.resize` | 94 | 8 | 2 | 2 | 7 | 12 | 1 | | | 62 |
| `render.shared_colorbuffer` | 84 | 6 | 6 | 6 | | | 3 | | | 63 |
| `render.shared_colorbuffer_clear` | 34 | 8 | 2 | 2 | | | 1 | | | 21 |
| `render.shared_depth_stencil` | 10 | | | | | | | | | 10 |
| `render.stencil_clear` | 3 | | | | | | | | | 3 |
| **`fbo.render`** | **305** | 30 | 12 | 12 | 14 | 24 | 6 | | | 207 |
| **both** | **511** | 63 | 21 | 14 | 61 | 96 | 12 | 34 | 3 | 207 |

(Check: 206 + 305 = 511; 63 + 21 + 14 + 61 + 96 + 12 + 34 + 3 + 207 = 511.)
Of the 448 `Fail` rows (511 less A) only B and C (35) are another note's;
the rest are E1 (61, expected to move on their own), the causes of D1, D2,
D4 and D5 (145 rows) and one unexplained set of 207.

What the groups are, from the sources (`es3fFboColorbufferTests.cpp`,
`es3fFboRenderTest.cpp`; both compare with `tcu::bilinearCompare`, threshold
at least 12 of 255 a channel, which fails on one wrong pixel; the ES 2.0
twin of `FboRenderCase` compares with `fuzzyCompare(0.05)`):

- `color.clear`: a renderbuffer 129x117 of the format, a full clear, 16
  scissored clears, `glReadPixels`. `color.tex2d`: two textures of 129x117
  and 99x128, a quad rendered into each from an RGBA8 texture, then both
  sampled on the window. `color.texcube`, `tex2darray`, `tex3d`: one
  framebuffer per face / layer / slice (`glFramebufferTexture2D` /
  `glFramebufferTextureLayer`), a render into 4 (cube) or all 5 of them, each
  face / layer sampled on the window with a `samplerCube`, `sampler2DArray`
  or `sampler3D` (`isampler*` / `usampler*` for the integer formats).
  `color.blend`: a grid drawn without, a gradient with `SRC_ALPHA,
  ONE_MINUS_SRC_ALPHA` (alpha `ZERO, ONE`) into a renderbuffer. `repeated_clear`:
  a 16x16 and a 64x64 `glTexStorage2D` texture (or renderbuffer), 16 clears of
  the small one, each copied into a cell of the large one by `glBlitFramebuffer`
  (CPU) or by a draw that samples it.
- `render.*` (308 cases in `FboRenderTestGroup::init`; `texsubimage` is
  `#if 0` in the ES 3.0 file and is not a case): a clear of the whole
  window, then 128x128 of work in one or two framebuffers that share a colour
  buffer, a depth/stencil buffer, are resized (128x128 to 64x32) or have a
  buffer deleted and made again under its name, then either `glReadPixels` of
  the renderbuffer or a draw of the texture on the window. The flat, texture
  and gradient shaders write `ivec4` / `uvec4` / `vec4` by format and scale
  and bias the colour into the format's range (so a narrow integer or fixed
  format is written in the channels it has, except in `shared_colorbuffer`,
  `shared_colorbuffer_clear` and `color.blend`, which write plain colours).
- No case attaches a mip level other than 0, and none of the 511 uses a
  second attachment (D above).

## 2. How the sets were found: a host replay, and its limit

The cases were replayed on the host through the real GL core and the real
V3D back end (`tests/v3d_lazy_raster_readers.rs`'s rig: the stand-in GPU, the
real `v3d_tex.c`, the online compiler) in a scratch test that is **not**
committed (a new test file needs a `tools/check.sh` row, which is #316's):
the rig's first 887 lines, a 128x128 canvas, and 14 case bodies (`color.clear`,
`tex2d`, `texcube`, `tex2darray`, `tex3d`, `blend`, `repeated_clear` x3,
`resize`, `recreate_color`, `recreate_depth_stencil`, `shared_colorbuffer`,
`shared_colorbuffer_clear`, `shared_depth_stencil`, `stencil_clear`) written
from the dEQP calls with ES 3.00 shaders of the right output and sampler
type per format, 36 formats, 547 cases. Per case it records the GL errors,
the framebuffer status of every framebuffer, the V3D's refusals by name and
the jobs the builder saw.

Two replays were run, of main and of `925dbca9` (the same test on the
`git archive` of that commit), and compared with the tsv:

| | tsv `Fail` rows (448) | tsv pass rows replayed (99) |
|---|---|---|
| `925dbca9`: ok | 222 | 99 |
| `925dbca9`: framebuffer incomplete (`0x8cd6`) | 164 | 0 |
| `925dbca9`: a GL error from a refused clear (`GL_OUT_OF_MEMORY`) | 35 | 0 |
| `925dbca9`: the sampling program refused (array / 3D) | 27 | 0 |
| main: ok / incomplete / program refused | 302 / 82 / 64 | 99 ok |

So the replay never flags a row that passed on the board (0 of 99), and at
the run's commit it flags 226 of the 448 failures with the refusals the code
had then (the integer clear's `GL_OUT_OF_MEMORY`, the integer textures'
incomplete attachment, the sRGB refusal). It is a model of the GL core and
the back end's *accept / refuse* decisions, not of pictures: the stand-in
GPU paints nothing a comparison could use, so a row it calls ok is a row
whose failure is a wrong picture, a GL error the replay did not issue, or an
order of CPU and GPU work the synchronous rig does not have.

## 3. D1. Narrow integer textures as render targets (E2a, 96 rows)

**What fails.** A texture of `GL_R8I .. GL_RG32UI` is an incomplete
attachment (`GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`; the replay says so for all
12 formats in all 8 groups, and `fbo.completeness` is unaffected). The gate is
the GL core's `integer_texture_internalformat` (`src/gl/framebuffer.rs:404`):
"Only the four-channel classes (RGBA8I .. RGB10_A2UI) qualify. `es3_formats`
widens an R or RG integer texture to a four-channel store, so the surface of
its internal format (`GLES_SURF_R8I` ..) has a different texel size than the
store: drawing it would write the wrong layout, and the channels the format
lacks would not read 0, 0, 1." The renderbuffer of the same formats is a
complete `GLES_SURF_R8I` etc. (`renderbuffer_surf_format`), which is why
`repeated_clear.blit.rbo`, `shared_*` and the rbo halves of `resize` and
`recreate_color` are not in this set.

**The owner is the GL core first.** `docs/notes/gl-texture-classes-es3.md`
already plans the way out: step 3 `narrow-classes` (classes 30-39: one- and
two-channel 8/16-bit integer classes, 2 and 4 bytes a texel; the R32/RG32
classes 26-29 are in the GL core since `d7d7357a`, "in no mask yet") and step 4
(map them in `integer_texture_surface`, `framebuffer.rs:~421`, so the texture
is a complete attachment where `rt_format_drawable` says the surface is
drawn). Nothing in `src/v3d` can be tried before those two GL-core steps:
the V3D draws and samples the wide class today.

**The `src/v3d` part, once the classes exist** (the V3D needs no wide-store
trick: Mesa has the narrow rows natively):

1. `V3D_SAMPLED_TEXFMTS` (`src/backend.rs`, the V3D mask) gets bits 26-29 and
   30-39 for the classes the V3D samples (all of them: Mesa's table has
   `R8I R8UI R16I R16UI R32I R32UI RG8I RG8UI RG16I RG16UI RG32I RG32UI` as
   `SWIZ_X001` / `SWIZ_XY01` with return size 16 for the 8- and 16-bit and 32
   (1 or 2 channels) for the 32-bit ones, `v3dx_format_table.c:118-134`).
2. `tmu_format` (`src/v3d/texture.rs:~200-246`) gets the rows: the data
   format of the class, swizzle `[R, 0, 0, 1]` for one channel and `[R, G, 0,
   1]` for two (the arms of `R16F` and `RGB565` are the model); a texture of
   these classes read through the sampler of its type needs no new look-up.
3. `gles_texfmt_return_32` (`src/backend.rs`) lists the four 32-bit classes
   (`R32UI`, `R32I`, `RG32UI`, `RG32I`) so the program is compiled for the
   32-bit TMU output (`LodKey::ret32`) and `texture::refusal_for`'s
   `return_32` check agrees.
4. The tiled copy (`copy_image_for`, `v3d_tex_layout(.., cpp, ..)`) takes
   `cpp` 1, 2, 4 and 8 from `gles_texfmt_bytes`; the TFU path is not used for
   integer classes today (`v3d-tfu-order.md`), so nothing there.
5. The render side needs no new row: `state::rt_format` has all twelve
   (`state.rs:472-483`, Mesa `v3dx_format_table.c:236-345`). `rt_texture`
   (`backend.rs:1794`) keeps these textures out of the render-image table
   (RGBA8 only), so a render into one stores the raster storage and the next
   sample takes the tiled copy of it, as every non-RGBA8 target does.
6. Tests (`src/v3d/backend_tests.rs`, `gl_rig`): an `R8I` texture attached,
   drawn into, sampled with an `isampler2D`: the job's `rt_format[0]` and
   `stride = pitch * 1`, the sampled record's data format and swizzle; and
   the replay's incomplete count for the 12 formats going to 0.

After D1 these rows meet the next sets: the 24 `tex2darray` / `tex3d` rows
fall into E3 (D2) and the 14 `resize` / `recreate_color` rows into E4b.

## 4. D2. Sampling a 2D-array or 3D texture (E3, 34 rows now; 24 more behind D1)

**What fails.** The framebuffer of a layer or a slice is complete and the
render into it is accepted (the replay: complete, no refusal, all 36
formats); the draw that samples the whole texture on the window never gets
that far, because the program does not compile and the texture is refused
at bind:

- `src/shaderc/online/v3d/prepare.rs:1335`: `Op::Tex` takes `TexDim::D2` and
  `Cube` only ("a texture look-up other than 2D or cube"); the array index
  (`TMUI`, `select.rs` constant, modelled in `sim.rs`) and the 3D
  coordinate (`TMUR`) are not written, and `Op::TexSize` of a 3D or array
  sampler is refused at `prepare.rs:1494`. This is **`src/shaderc`**,
  plan #192's.
- `src/v3d/texture.rs:126-136`: `refusal_for` answers "a 2D array or 3D texture
  is not sampled yet" for the two types. The note states what exists: the
  layout, the image and the record of both are in place (`layout`,
  `image_offset`, `state`, `tile`; `docs/notes/v3d-tex-layout-es3.md`), the
  look-ups are not.

**Design, the `src/v3d` half.** After the compiler writes `TMUI` / `TMUR`:
(1) the two arms of `refusal_for` return `None` for `GLES_TEXTYPE_2D_ARRAY`
(layers = the layer count) and `GLES_TEXTYPE_3D` (layers = level 0 depth),
with the existing size checks; the cube arm's square check does not apply;
(2) the texture state already carries `image_depth` and the array stride
(`v3d_tex.c` `array_stride_64`, `texture::state`), so the record needs no
new field; (3) a render into a layer is a plain raster target at the layer's
offset (`gl_level_layered`; `rt_texture` requires `layers == 1`, so no render
image), and the next sample re-tiles the whole texture (`copy_image_for`,
cost: one tiling per layer rendered, 5 for these cases); (4) `textureSize` is
the compiler's (`prepare.rs:1494`). **Tests:** a `gl_rig` replay of
`color.tex2darray.rgba8` / `tex3d.rgba8` with the refusal lifted:
`rig.jobs()` shows 5 render jobs with `address` = base + `layer * stride`,
then a window job whose TMU record has `image_depth` 5 and the layered
array stride; the sampling program compiles.

## 5. D5. `GL_RGB16F` as a texture attachment (E2b, 12 rows)

A `GL_RGB16F` texture is stored in the `RGBA16F` class and its `es3_fmt` is
the RGB16F entry, so `attachment_storage` refuses it
(`framebuffer.rs:508-512`: "an ES 3.0 format of es3_formats is no render
target ... `t.es3_fmt != 0 && t.texfmt != RGBA8`"), although the GL core
lists `GL_EXT_color_buffer_half_float` (`state.rs:111`, and
`dEQP`'s `getEnablingExtensions` takes it as sufficient for `GL_RGB16F`) and
the renderbuffer of the same format is complete. The replay: the 6 texture
groups, `shared_colorbuffer` x3 and `shared_colorbuffer_clear` x1 are
incomplete for `rgb16f` and nothing else of the 16F family (`rg16f`, `r16f`,
`rgba16f` textures are complete).

Owner: **`src/gl`**, one condition in `attachment_storage`: let a `RGBA16F`
texture whose format is `GL_RGB16F` through, with alpha bits 0
(`fb_target_alpha_bits_n` already answers 0 for the renderbuffer,
`framebuffer.rs:1006`). The `src/v3d` half is already there for the
renderbuffer: the draw is the `RGBA16F` surface with
`target_alpha_bits == 0`, so `Target.no_alpha` masks alpha out of the write
mask (`state.rs:670`), takes the destination alpha as 1 for the blend factors
(`state.rs:647`) and forces alpha 1 at the job's load
(`backend.rs:2683`, `V3dJobClear::force_alpha_1`). Whether `force_alpha_1`
is valid on a 64-bpp 16F render target (it is exercised only for the RGBA8
untyped path by the host tests) is one line to check in Mesa's
`v3dx_rcl.c` load configuration before the texture case is trusted; the
renderbuffer rows of `rgb16f` (`color.blend.rgb16f_src_over` passes today)
are the control.

## 6. D4. A format with channels the draw still writes (E4a, 3 rows)

`color.blend` fails for `r8`, `rg8` and `rg16f` and passes for `rgb8`,
`rgb565`, `rgba4`, `rgb5_a1`, `rgba8`, `rgba16f`, `r16f`, `rgb16f`. That
split is the formats **held in a wider store whose missing channels are not
alpha**. The GL core holds `GL_R8` and `GL_RG8` in RGBA8 and `GL_RG16F` in
RGBA16F; for them it passes `target_alpha_bits == 0`
(`framebuffer.rs:1006-1007`) and nothing else, and the V3D takes only alpha out
of the write mask (`state.rs:670`: `st.colormask &= 0x7`). The blend case's
first draw writes a grid and the second a gradient with every channel, so the
stored G (and B) of an `R8` buffer are not 0 and `glReadPixels` returns them
where ES 3.0 4.3.2 gives 0, 0, 1. `r16f` passes because it is its own surface
(`GLES_SURF_R16F`, only R is stored); `rgb8` because its missing channel is
alpha. The comment at `framebuffer.rs:998-1001` records the gap ("Their
green and blue read 0, which the PBE's swizzle could do as well and does not
yet"). The other 21 `r8` / `rg8` / `rg16f` rows of `shared_colorbuffer` and
`shared_colorbuffer_clear`, which write plain colours too, are exposed to the
same thing and are counted in E4b only because they would fail anyway.

**Design.** The V3D cannot tell `R8` from `RGB8` from what it gets (both
arrive as `GLES_SURF_RGBA8888` with alpha bits 0), so the information has to
come from the GL core: a field in `GlesDraw` for the channels the target has
(`target_channels`, 4 bits, set where `set_target_format` sets the alpha bits,
`src/gl/draw.rs:2093`), and in the V3D `st.colormask &= channels` next to the
alpha line (`state.rs:670`), the same mask for the clear quad
(`clear_quad_draw` copies the target's format and alpha bits, `state.rs:164`,
and would copy this) and the clear colour's missing channels set to 0 (and
alpha 1) in `clear_words`. A masked clear is a quad (`colour_needs_quad`), so
a plain `glClear` of an `R8` buffer would become a quad; the cheaper
alternative is zeroing the missing channels of the TLB clear colour and
keeping the whole-tile clear, which is right while nothing masks. GL-core
change first, V3D second.

## 7. E4b. The 207 `fbo.render` rows nothing explains

**The facts.**

- Every format of the group fails, `rgba8` included, and the failures are in
  all seven sub-groups. Of the 308 cases of the ES 3.0 group, three pass:
  `recreate_color.rbo_r16i`, `rbo_r16ui` and `rbo_r16f` (the tsv lacks them).
- The PowerVR on the AM62P passes the `rgba8` cases of the same group
  (`docs/es3/am62p-gles3-notpass.tsv` has none of them), so the test and the
  GL core are not what is wrong everywhere.
- The Pi 4 passes the ES 2.0 `fbo.render` of the same harness (17,126 cases,
  0 fail, `46f93767`, `tools/cts/RESULTS.md`), which compares with
  `fuzzyCompare(0.05)`. The ES 3.0 harness compares each pixel
  (`bilinearCompare`, at least 12 of 255), so a picture with one wrong row or
  column that ES 2.0 forgave fails here. `fbo.color` uses the same compare and
  passes for `rgba8`, so the compare alone is not the cause.
- The replay finds nothing: no refusal, no incomplete framebuffer, no GL error
  for these rows, at main or at `925dbca9`, and the jobs are the ones the
  code means (`shared_colorbuffer_clear.rbo_rgba8`: the window's clear job,
  then one 128x128 job on the renderbuffer with the TLB clear to blue and four
  clear-quad draws; the `tex2d` form adds the window job that samples).
- The `fbo.color` rows of the same formats pass (`clear`, `tex2d`, `texcube`,
  `repeated_clear` for `rgba8`, `rgba16f`, `r8`, `rg8`, `rgb565` ...), so
  the V3D draws, clears, samples and reads those formats right in
  isolation.

**What the rows have that `fbo.color` has not**, ranked by how much of the
207 each would explain and how cheap it is to rule out (none is measured):

1. **The window clear that opens every case** (`FboRenderCase::iterate`: a
   full clear of colour, depth and stencil of the default framebuffer, then
   the case). It is a job with a depth/stencil buffer and no draw, ended by
   the first draw into a framebuffer. The replay's jobs for it are plausible
   (a 128x128 job with a Z/S address and `zs_load` false), but the rig
   completes every job inside its submit; the board runs `flush_job` with at
   most two jobs outstanding (`queued` / `in_flight`) and the stamp aliasing
   of `open_alias`, which the earlier Pi 4 `fbo.render` failures were about
   (`backend_tests.rs`, "FBOR5", "S8FBOC"). Rule out first by running one case
   with the window clear removed (one board run).
2. **A shared or recreated attachment between framebuffers**
   (`shared_colorbuffer*`, `shared_depth_stencil`, `recreate_*`): 3 framebuffer
   objects on one colour image, a depth/stencil image owned by two, a name
   deleted and bound again. In the V3D these meet `Target` identity
   (`draw_call`, `g.j.target != target`), the per-`ds_id` `ZsBuf` table
   (`target_zs_bufs`, `backend.rs:2711`; a framebuffer past the 33 slots is
   refused with `GL_OUT_OF_MEMORY`; `DS_SLOTS` is 32), `rt_for_render` (`backend.rs:1966`) and
   `texture_released`. But `resize` (no sharing) and `stencil_clear` (one
   framebuffer) fail too, so this is at most part of it.
3. **The depth/stencil half**, for the 24 cases that attach a depth or
   stencil texture or a `STENCIL_INDEX8` / `DEPTH32F_STENCIL8` buffer
   (`recreate_depth_stencil` 11, `shared_depth_stencil` 10, `stencil_clear` 3, and
   the 22 depth variants of `resize`): the scissored stencil clear is a quad
   (`quad_stencil`, `backend.rs:6145-6150`, GFXH-1461 in `v3dx_draw.c`), a
   depth texture is read back through `seed_depth` / the depth resolve, and
   `seed_depth` refuses a stencil-only buffer with `-ENOTSUP`
   (`backend.rs:2752`, "a 4x or S8 buffer"). The replay says these draws are
   accepted; it cannot say the stencil result is right. `fbo.depth` and
   `fbo.stencil` (FboTestCase, flat shaders, no texture) pass on the Pi 4, so
   a defect would be in what `fbo.render` adds: sampling the depth texture,
   and the quad clear with scissor.
4. **Order within the 8 FB4 image**: the image runs `fbo.invalidate` (38 not
   pass) and `fbo.msaa` (66) first. A state one of them leaves (a refused
   4x draw, a `GL_OUT_OF_MEMORY`, a job left open) would break the rest
   of the image and none of the nine other images. Against it: three
   `render` cases pass in the middle of the group, and a refused draw in
   `fbo.color.clear` (image FB3, 19 of them) did not break its neighbours.
   Rule out by running `fbo.render.*` as an image of its own.

**What settles it, and who has it.** The 207 rows are one question to the
board, not to the code: the console record of an image holds each failing
case's message ("Image comparison failed", "Got unexpected error", a
framebuffer status) and, with `--deqp-log-images`, the two pictures. Plan
#320's rerun of FB4 should (a) keep the first message of each not-pass case in
the tsv (the am62p file has the column, the Pi 4 file does not), (b) run
`fbo.render.*` alone, and (c) keep the QPA images of five cases:
`shared_colorbuffer_clear.rbo_rgba8` (clears only, no draw, no texture, no
depth: the smallest), `resize.rbo_rgba8`, `recreate_color.tex2d_rgba8_depth_stencil_rbo_depth24_stencil8`,
`stencil_clear.tex2d_rgba8_stencil_rbo_stencil_index8` and
`shared_depth_stencil.tex2d_rgba8_depth_tex2d_depth_component24`. A wrong
picture says which of 1-3 it is within minutes; "Got unexpected error" says
a GL error the replay does not issue.

## 8. What each fix moves

| fix | rows | needs |
|---|---|---|
| a rerun on main (E1) | 61 expected to pass or join E4b (14 of them, the `render` ones, probably join it) | board |
| D1 narrow integer classes | up to 96 (then 24 meet D2 and 14 meet E4b) | `src/gl` classes 30-39 and `integer_texture_surface`, then `src/v3d` bits, TMU rows |
| D2 array / 3D sampling | 34 now, 58 with D1 | `src/shaderc` look-ups, then `src/v3d` refusal |
| D5 `rgb16f` texture | 12 | `src/gl`, one condition |
| D4 missing channels | 3 certain, 21 exposed | `src/gl` field, then `src/v3d` mask |
| B, C | 35 | `srgb.md`, `v3d-rgb10a2.md` |
| E4b | 207 | board data (section 7) |
| A | 63 `NotSupported` | a decision on `GL_EXT_color_buffer_float`; the V3D has rows for `R32F` only (`state.rs:471`): `RG32F` (32F, 64 bpp), `RGBA32F` (32F, 128 bpp) and `R11F_G11F_B10F` (16F, 64 bpp) are Mesa's (`v3dx_format_table.c:310-330`, `:250-262`) and would be three `rt_format` arms and three `GLES_SURF_*` values; the GL core stores `GL_RGBA32F` as halves (`sized_format`), so claiming the extension needs a real 32-bit store first |

Of the 511 rows the cause is known for 304 (A, B, C, E1, E2a, E2b, E3, E4a),
and of those 61 need no change.
