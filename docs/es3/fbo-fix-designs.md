# Fix designs for the AM62P dEQP-GLES3 fbo fails

Plan #400, item fbo-rest-design. This file gives a fix design for every cause of
the `fbo.*` Fail lines of `docs/es3/am62p-gles3-notpass.tsv`. Each design names
the case count, the cause with file:line, the change and its files, the owner and
a host test. Two causes already have fix items of their own, `fbo-rint` and
`fbo-msaablit`. They are listed with their counts so that the counts add up, but
they are not designed again here.

Source of the clusters: the done triage item tri-fbo (plan #400). Its branch
changes no file, so its clusters exist only in its report. That report has seven
clusters, which sum to 155. The missing 12 lines are the R/RG integer formats of
`fbo.color.tex3d` (section 5). This file counts every line from the TSV itself.
The file:line references are to the integration tip `bdd2ba1c`. Between the build
the board ran (`b25997e4`) and that tip, `src/pvr/backend.rs`,
`src/gl/{blit,draw,framebuffer,native_cube,native_vol}.rs` changed only in
d243aefe, 253dd427 (base/max level) and ecd9743e (RGBA32F). None of those three
touches the paths cited here.

The run: all `fbo.*` cases are in tree rows 1 and 2 (`tools/cts/am62p-gles3-tree.txt`).
Both rows ran at build `b25997e4` on 2026-11-22, the TSV's rows 0-10. Nothing in
this file ran on a board, and dEQP did not run on the host: the VK-GL-CTS cache
on this machine has the data only, not the sources. Where a design says
"suspected", the first step of its fix item is the named host test or board A/B,
which tells the suspects apart before any change is made.

## 0. Reconciliation with the TSV

`awk -F'\t' '$1=="Fail" && $2 ~ /functional\.fbo\./' docs/es3/am62p-gles3-notpass.tsv | wc -l`
gives **167** (the other 247 `fbo.*` lines are NotSupported and are not in scope).

| § | Cause | Lines | Status | Fix item / owner |
|---|---|---:|---|---|
| 1 | R/RG integer colour textures and renderbuffers (stored RGBA, sampled RGBA) | 72 | traced | **fbo-rint** (existing), src/pvr |
| 2 | 4x MSAA depth/stencil resolve blit refused (`GL_INVALID_OPERATION`) | 12 | traced | **fbo-msaablit** (existing), src/pvr |
| 3 | Cube faces rendered through an FBO, then sampled by a native-cube program | 20 | suspected | new: `pvr-fbo-cube`, src/pvr (+ named src/gl) |
| 4 | 3D slices rendered through an FBO, then sampled through the slice grid, non-integer formats | 12 | suspected | new: `pvr-fbo-tex3d`, src/shaderc (switch) / src/shaderc/online/powervr |
| 5 | 3D slices of R/RG integer formats: causes 1 and 4 together | 12 | traced + suspected | fbo-rint, then `pvr-fbo-tex3d` |
| 6 | R8 / RG8 / RG16F targets: lanes the format lacks are stored and read back | 25 | traced | new: `pvr-narrow-targets` (adopts adc1abfb), src/pvr + named src/gl |
| 7 | `invalidate.*.unbind_blit_msaa_*`: a 4x blit that carries a depth plane | 4 | suspected | joins fbo-msaablit, plus a test of its own |
| 8 | `fbo.depth.depth_{test,write}_clamp`: a per-fragment `gl_FragDepth` | 10 | suspected | joins the `shaders.fragdepth` cause (fix-shpix-design), src/shaderc/online/powervr / src/pvr |
| | **Total** | **167** | | |

72 + 12 + 20 + 12 + 12 + 25 + 4 + 10 = 167.

Line lists, by TSV case name (prefix `dEQP-GLES3.functional.fbo.`):

- §1 (72): `color.tex2d.{r,rg}{8,16,32}{i,ui}` 12, `color.tex2darray.*` same 12,
  `color.texcube.*` same 12, `invalidate.format.*` same 12,
  `render.recreate_color.tex2d_*_depth_stencil_rbo_depth24_stencil8` same 12,
  `render.resize.tex2d_*` same 12.
- §2 (12): `msaa.{2,4}_samples.{depth24_stencil8, depth32f_stencil8,
  depth_component16, depth_component24, depth_component32f, stencil_index8}`.
- §3 (20): `color.texcube.{r8, r16f, rg8, rg16f, rgb8, rgb565, rgb16f, rgb5_a1,
  rgb10_a2, rgb10_a2ui, rgba4, rgba8, rgba8i, rgba8ui, rgba16f, rgba16i, rgba16ui,
  rgba32i, rgba32ui, srgb8_alpha8}`.
- §4 (12): `color.tex3d.{r8, rg8, rg16f, rgb8, rgb565, rgb16f, rgb5_a1, rgb10_a2,
  rgba4, rgba8, rgba16f, srgb8_alpha8}`.
- §5 (12): `color.tex3d.{r,rg}{8,16,32}{i,ui}`.
- §6 (25): `blit.default_framebuffer.r8_{linear_out_of_bounds,linear_scale,nearest_scale}_blit_from_default` 3,
  `color.blend.{r8,rg8}_src_over` 2,
  `render.shared_colorbuffer.{rbo_r8, rbo_rg8, tex2d_r8, tex2d_rg8, tex2d_rg16f}{,_depth_rbo_depth24_stencil8,_depth_stencil_rbo_depth24_stencil8}` 15,
  `render.shared_colorbuffer_clear.{rbo_r8, rbo_rg8, tex2d_r8, tex2d_rg8, tex2d_rg16f}` 5.
- §7 (4): `invalidate.{sub,whole}.unbind_blit_msaa_{color,stencil}`.
- §8 (10): `depth.depth_test_clamp.*` 5, `depth.depth_write_clamp.*` 5 (all five depth formats).

The 12 `color.texcube` R/RG integer lines (in §1) and the 12 §5 lines each have
two causes: R/RG integer plus cube (§3), and R/RG integer plus 3D (§4). They are
counted once, in the row named above (§1 as tri-fbo counted them; §5 is the row
tri-fbo's clusters left out). They pass only after both fixes have landed. This
matches the board picture: `color.tex2darray` fails only its 12 R/RG integer
formats, while `color.texcube` (32) and `color.tex3d` (24) fail every format.

## 1. R/RG integer colour attachments (72) -- existing item fbo-rint

Cause (tri-fbo, traced): an R/RG integer texture is kept in the 4-channel integer
store and sampled as RGBA (`src/pvr/backend.rs`, `es3_tex_format` /
`build_tex_desc`, around line 1304), so G/B/A return what the shader wrote
rather than 0, 0, 1. The design, the change and the host test are fbo-rint's.
Nothing is added here.

## 2. 4x MSAA depth/stencil resolve blit (12) -- existing item fbo-msaablit

Cause (tri-fbo, traced; the host part file shows the same 12 lines as
`gl-error` in `docs/es3/host-gles3-part-powervr.tsv`): the blit's destination is
a renderbuffer that no kick has stored into. `pvr_backend_ds_write` answers 1
("nothing held", `src/pvr/backend.rs:5909`) to the empty-rectangle probe of
`dst_writable` (`src/gl/blit.rs:1058-1067`). The GL core then refuses the whole
blit with `GL_INVALID_OPERATION` (`src/gl/blit.rs:1418-1421`,
`REFUSED_DS_UNHELD`). The design and the test are fbo-msaablit's. Section 7
depends on it.

## 3. Cube faces rendered through an FBO (20) -- new item `pvr-fbo-cube`

**What the case does.** It specifies the six faces with `glTexImage2D(..., NULL)`,
attaches each face in turn with `glFramebufferTexture2D`, draws into it, then
binds framebuffer 0 and draws six quads that sample the cube (a `samplerCube`
program, online-compiled). Every format fails, float and integer included. The
same case on a 2D array (`color.tex2darray`) passes for every non-R/RG-integer
format. So the fault is between "rendered into the cube's storage" and "sampled
as a cube", not in the format path.

**Path at the tip.** `PVR_NATIVE_CUBE_DEFAULT` is 1 (`src/shaderc/mod.rs:282`),
so the program samples the TPU's own cube, through the twiddled copy that
`gl::native_cube::copy` makes (`src/gl/native_cube.rs:181-283`). The draw asks
whether the copy is stale (`src/gl/draw.rs:4939-4960`). The doc comment of
`src/gl/native_cube.rs:4` still says "default OFF", which is out of date.

**Suspects, in the order the test checks them:**

1. `src/gl/draw.rs:4944-4945`: when the render into the cube is the open kick
   (`rendered == backend::kick_id()`), or `backend::finish()` fails, or `copy`
   returns `None` (`copyable`, `src/gl/native_cube.rs:144-146`), `tb` keeps the
   **stacked** linear image (`GLES_TEXTYPE_CUBE`). But the program was compiled
   to sample the native cube (`sh.native_cube & 1`). That native-cube program
   then reads a linear level-major image as a twiddled face-major one, which
   matches "every format wrong". On the 2D array the same kind of mismatch
   cannot happen: its native arm is off by default
   (`PVR_NATIVE_ARRAY_DEFAULT` 0, `src/shaderc/mod.rs:311`), so program and
   image are both stacked.
2. The copy is taken but holds texels from before the render: the target-use
   stamp of the face kicks (`src/gl/framebuffer.rs:195-212`,
   `stamp_target_use`) is not what `tb.target_kick` reads, or the CPU copy reads
   `t.mem` before the last face's kick has been waited for.
3. The copy is right but its descriptor is not, for a rendered texture: the
   `CUBE_TWIDDLED_PROBE` arm (`src/pvr/backend.rs:6331-6380`), with
   `pe_order`/`bgra` (`src/gl/draw.rs:4918`) and the one-level `single` rule
   (`src/gl/native_cube.rs:166-180`).

**The test first** (new file `tests/pvr_fbo_cube_faces.rs`, features
`host-test,gpu-powervr,shaderc-online`). It makes a 64x64 cube with NULL faces
in RGBA8, RGBA16F and RGBA32I. It renders each face through an FBO with a
clear and a flat-colour draw, writing each face's texels on the CPU where the
host has no GPU, the way the existing host tests stand in for the pixel engine.
It then binds framebuffer 0 and draws with an online-compiled `samplerCube`
program. It asserts:
(a) the bind that reaches the back end is `GLES_TEXTYPE_CUBE_TWIDDLED_PROBE`
whenever the program's record has `native_cube & 1` (never a stacked image
under a native program);
(b) `native_cube::copies()` went up by one after the six face renders;
(c) face k, texel (x, y) of the copy at `twiddle_index` equals what face k's
render left in the stacked storage;
(d) the texture-state words for each format, against the words the existing
CPU-uploaded cube tests pin.

**Change, by which assertion fails:**

- (a) fails: when a native-cube program cannot get a copy, the draw must
  not hand it the stacked image. The src/pvr-side change is in
  `src/pvr/backend.rs`: refuse such a draw by name (`-ENOTSUP`, the
  refusal convention of `pvr_backend_ds_write`) instead of describing a stacked
  image with a cube textype. The proper fix is in **src/gl and is only named
  here**: at `src/gl/draw.rs:4944`, close the open kick (`backend::flush`) when
  `rendered == backend::kick_id()` and the frame being recorded is **not** the
  one sampling (render to an FBO, then draw to another target, is not a feedback
  loop). Then the copy is made.
- (b) or (c) fails: the stamp or the wait. That is src/gl (`framebuffer.rs`
  `stamp_target_use` for a cube-face attachment, or `native_cube::copy`'s
  `cache_invd` ordering), named only.
- (d) fails: `src/pvr/backend.rs` `build_tex_desc`, the
  `CUBE_TWIDDLED_PROBE` arm. That is the src/pvr fix, with the test as its
  pin.

**Board A/B** (with board-n3d or the next AM62P session):
`dEQP-GLES3.functional.fbo.color.texcube.*` with `DEQP_PVR_NATIVE_CUBE=0`
(`driver/samples/zephyr/deqp/Kconfig:92`). If the 20 lines pass with the stacked
cube, the fault is in the native path (suspects 1-3). If they still fail, it is
in the render-to-face storage, which the GC7000UL shares (compare its
`fbo.color.texcube` in `docs/es3/imx8mp-gles3-notpass.tsv`).

Owner: src/pvr (refusal, descriptor). The src/gl change at `draw.rs:4944`, if
the test points there, goes in the report of the item that makes it.

## 4. 3D slices rendered through an FBO, non-integer formats (12) -- new item `pvr-fbo-tex3d`

**What the case does.** It renders into each slice of a 3D texture with
`glFramebufferTextureLayer`, then samples the volume with a `sampler3D` program
at each slice's depth. Every format fails. `color.tex2darray`, which renders
layers the same way, passes.

**Path at the tip.** `PVR_NATIVE_3D_DEFAULT` is 0 (`src/shaderc/mod.rs:339`).
The volume is a slice grid (`GlTexture::grid_cols`,
`src/gl/context.rs:437-444`). The render target of slice z is
`texture::grid_slice_target` (`src/gl/texture.rs:1764`, used at
`src/gl/framebuffer.rs:609-610`; pinned by
`a_3d_grid_slice_is_a_render_target_at_the_grids_pitch`,
`src/gl/framebuffer.rs:7984`). The program samples the grid as one 2D image
through the online compiler's 3D grid arm (`src/shaderc/online/powervr`).
CPU-uploaded volumes fail broadly on the same board too: `texture.*.3d`,
`texture.vertex.3d`, `texture.units` 3D (RC15, `docs/es3-cts-failures.md`), with
#1285's note `docs/notes/pvr-3d-mipmap-hang.md`. So the suspect is the grid
sampling itself (RC15), not the render-to-slice.

**Change.** Use the native 3D path: `PVR_NATIVE_3D_DEFAULT = 1`
(`src/shaderc/mod.rs:339`). That decision belongs to `n3d-default-decide`, and
its pinned-file impact was measured by `n3d-flip-impact`. Under native 3D, the
draw makes `native_vol::copy` with the render's `target_kick`
(`src/gl/draw.rs:5025-5030`). So this item adds no change of its own until the
board A/B below says the grid is at fault. If the A/B passes with native 3D,
the 12 lines move with the flip. If it fails both ways, the render-to-slice is
at fault: `grid_slice_target`, or `native_vol::copy` reading the slice from
the grid at a pitch other than the one it was rendered at
(`texture::grid_level_pitch`, `src/gl/framebuffer.rs:1343-1344`). Those are
src/gl, named only.

**Host test** (new file `tests/pvr_fbo_tex3d_slices.rs`, features
`host-test,gpu-powervr,shaderc-online`). A 32x32x4 RGBA8 volume. Render slice z
through `glFramebufferTextureLayer`, writing the texels on the CPU at
`grid_slice_target`'s offset and pitch. Then draw with a `sampler3D` program,
once with `pvr_set_native_3d(0)` and once with 1, and assert:
(a) grid arm: the address and stride words of the bound image are the
grid's (`grid_cols` columns), and the online compiler's slice select for
r = (z + 0.5) / 4 lands on column z % 2, row z / 2;
(b) native arm: the copy is remade after the render (its stamp moves), and
copy texel (x, y, z) equals the grid's slice z texel (x, y).

**Board A/B:** add `fbo.color.tex3d.*` to board-n3d (`DEQP_PVR_NATIVE_3D=1`), as
tri-fbo's handoff already asks.

Owner: src/shaderc (the switch, `n3d-default-decide`); src/shaderc/online/powervr
if the grid arm's slice select is wrong.

## 5. 3D slices of R/RG integer formats (12) -- fbo-rint, then `pvr-fbo-tex3d`

These are `color.tex3d.{r,rg}{8,16,32}{i,ui}`. They have §1's cause (an R/RG
integer store sampled as RGBA) and §4's cause (the volume path). Neither fix
alone moves them.

Change: none of its own. fbo-rint's change has to reach the 3D descriptor as
well as the 2D one: the R/RG lane swizzle must be applied in the grid arm and in
the `GLES_TEXTYPE_3D_TWIDDLED` arm of `build_tex_desc`
(`src/pvr/backend.rs:6455-6500`), not only for 2D. The 3D path is §4's.

Host test: extend §4's `tests/pvr_fbo_tex3d_slices.rs` with an RG16I volume.
Assert that the 3D descriptor's swizzle reads lanes B and A as 0 and 1 on both
arms, which is the assertion fbo-rint pins for 2D.

Owner: src/pvr (fbo-rint's swizzle), then §4's.

## 6. R8 / RG8 / RG16F targets keep lanes the format lacks (25) -- new item `pvr-narrow-targets`

**Cause (traced).** GL_R8, GL_RG8 and an R/RG byte texture are kept in an
RGBA8888 store, and GL_RG16F in RGBA16F's four halves. The PBE swizzle
(`src/pvr/backend.rs:907-929`, `pbe_swizzle`) stores the fragment's G and B (and
A, except for attachment 0's `target_alpha_bits == 0`,
`src/pvr/backend.rs:10044` and `:10088`) into lanes the format does not have.
`glReadPixels` of an RGBA8888 store hands the bytes back as they are
(`src/gl/framebuffer.rs:3259`, `read_pixels`; its float and integer paths
already force the missing channels). So a clear, a blend or a second
framebuffer sharing the colour buffer reads non-zero G/B where ES 3.0 4.3.2
says 0, 0, 1. `shared_colorbuffer_clear` fails without any blend, which puts
the fault in the stored lanes, not the blend equation. The
`blit.default_framebuffer.r8_*` lines blit the RGBA window into an R8 target,
which writes the same extra lanes.

**The fix exists, on unmerged branches, outside this plan:**

- `adc1abfb` "pvr: R and RG targets keep GL's missing channels" (issue #313,
  family F5; branch `connected/01a210fb-8032-73ba-9e41-bfa17473d960`). The PBE
  puts `SWIZ.ZERO` in a missing colour lane and `SWIZ.ONE` in a missing alpha
  lane, per attachment slot (`pbe_reg_words_missing`, `frame_missing`), for the
  background-object clear, the clear quad and every draw. Test:
  `tests/pvr_fbo_channels.rs`.
- `cde7a137` "gl: glreadpixels reads the channels a narrow colour attachment
  lacks as 0, 0, 1" (branch `connected/01a2131a-4e0e-7192-8562-969aaba94dce`),
  the read-back half. It is written for the GC7000UL but sits in the shared GL
  core.

**The src/gl part, named only (plan #397 owns src/gl).** adc1abfb needs the GL
core to tell the back end which channels each draw target lacks: the new
`framebuffer::fb_target_missing_channels_n`, filled per slot in `draw.rs`
`resolve_targets` into the new `GlesDraw::target_missing_channels` (bit 0 R ..
bit 3 A). That field is appended to `struct gles_draw` at offset 3604, in the
padding after `depth_texels_pitch`, with `sizeof` still 3608, so
`src/backend.h`, `src/backend.rs`, `src/abi_layout.rs` and
`src/abi_layout_check.c` change too. The read-back fix is `cde7a137`'s
`src/gl/framebuffer.rs` `read_pixels` change. The PowerVR back end cannot do
this alone: `GlesDraw` carries only the surface class
(`GLES_SURF_RGBA8888`) and `target_alpha_bits`, not that the internal format
is R8.

**src/pvr part:** adc1abfb's `src/pvr/backend.rs` hunk, `pbe_swizzle` and its
callers, as it stands on that branch.

**Host test:** `tests/pvr_fbo_channels.rs` from adc1abfb, which covers R8, RG8,
RG16F, RGB8 and RGB16F as renderbuffer and texture, cleared and cleared+drawn,
and pins REG_WORD0's lanes and the read-back. Add one case for the
blit-from-window shape (§6's 3 `blit.default_framebuffer.r8_*` lines): blit an
RGBA8 window into an R8 texture attachment, and assert that the read-back G, B,
A are 0, 0, 255.

Owner: src/pvr (PBE swizzle) plus the named src/gl change. The item should
rebase and land adc1abfb, with cde7a137, rather than write the fix again.

## 7. `invalidate.*.unbind_blit_msaa_{color,stencil}` (4) -- joins fbo-msaablit

**What sets them apart.** The single-sample siblings
(`invalidate.{sub,whole}.unbind_blit_{color,depth,stencil,...}`) pass, and the
4x colour resolve (`fbo.msaa.*` colour formats) passes. The case renders into a
4x FBO with an RGBA8 colour and a DEPTH24_STENCIL8 renderbuffer. It invalidates
the named attachment, binds framebuffer 0 and blits **the buffers it did not
invalidate** from that FBO to the window. So `msaa_color` blits depth and
stencil, and `msaa_stencil` blits colour and depth. Either way the blit carries
a 4x depth plane. dEQP's source was not available on this machine, so this
sequence is from memory; the fix item checks it against VK-GL-CTS
`modules/gles3/functional/es3fFboInvalidateTests.cpp`
(`InvalidateFboUnbindBlitCase`) first.

**Cause (suspected):** the same refusal arms as §2, at `src/gl/blit.rs:1391-1399`
(`refused`, whose arm skips the colour too: "nothing is copied, the colour
neither") or `:1418-1421`. That would be a 4x source plane (`depth_source` /
`stencil_resolve`) or a window plane `pvr_backend_ds_write` will not write
(`src/pvr/backend.rs:5909`: 1, nothing held; `:5912`: `-ENOTSUP` for
`s.samples > 1`). `glInvalidateFramebuffer` itself is validation only
(`src/gl/framebuffer.rs:3795-3830`), so it cannot cause the wrong picture. The
case does not read `glGetError`, so the refusal shows as "Image comparison
failed" and not as §2's error line.

**Change:** fbo-msaablit's, extended to the window as the destination. A blit of
the depth plane from a 4x renderbuffer into the window's held depth buffer must
pass the `ds_write` probe (`src/pvr/backend.rs:5884-5935`) and copy
`ds_sample0`'s values (d5231f8a). If the window's plane is not held yet,
initialise it as `ds_write` already does for an unstored buffer, instead of
answering 1.

**Host test** (new file `tests/pvr_invalidate_unbind_blit_msaa.rs`, features
`host-test,gpu-powervr,shaderc-online`). It runs both sequences on a window
context whose config has D24S8, for `glInvalidateFramebuffer` and for
`glInvalidateSubFramebuffer`, and asserts that `glGetError()` after the blit is
`GL_NO_ERROR`, that the colour copy happened (`msaa_stencil`), and that the
window's depth/stencil read back through `depth_resolve` / `stencil_resolve`
holds the FBO's sample-0 values (`msaa_color`). If fbo-msaablit's change makes
this test pass, the 4 lines go with that item. If not, the window-destination
arm above is the change.

Owner: src/pvr (`pvr_backend_ds_write`). The GL core's refusal rule stays as it
is.

## 8. `fbo.depth.depth_{test,write}_clamp` (10) -- joins the `shaders.fragdepth` cause

**What sets them apart.** `fbo.depth.basic.*` (5, the depth from the vertex)
passes for all five formats. The clamp cases write `gl_FragDepth` per fragment
(a gradient that leaves [0, 1]) and then depth-test against it. Every format
fails. The same build fails 11 of 17 `shaders.fragdepth` cases: every
`dynamic`, `*_conditional_write`, `uniform_loop_write`, `write_in_function` and
`fragcoord_z`, while `const`, `uniform` and `no_write` pass. So the fault is a
`gl_FragDepth` that varies per fragment or is written through control flow, not
the [0, 1] clamp.

**Path at the tip.** The chain clamps the depth with `Fsat`
(`src/shaderc/online/powervr/chain.rs:408-440`, `clamp_depth_output`) and emits
`depthf` (`assemble.rs`). The back end runs the program
`PVR_PPP_PASSTYPE_DEPTH_FEEDBACK` (`src/pvr/backend.rs:9097-9100`;
`docs/notes/gl-fragdepth.md` section 4).

**Cause and change:** those of the `shaders.fragdepth` family, which
fix-shpix-design owns (`docs/es3/shader-host-repro.md`). Whatever it finds in
the `depthf` source register, the feedback pass or the ISP depth-format words,
these 10 lines move with it. Change: none of its own. If that design closes
`shaders.fragdepth` and these 10 still fail, check the depth-format words of a
DEPTH_FEEDBACK pass into D16 and D32F (the ISP's ZLS format for a fed-back
depth), in `src/pvr/backend.rs`.

**Host test** (new file `tests/pvr_fbo_depth_clamp.rs`, features
`host-test,gpu-powervr,shaderc-online`). It compiles the dEQP gradient depth
writer (`gl_FragDepth = mix(u_min, u_max, v_coord.x)` with min -1 and max 2) and
asserts that `writes_depth` is set, that the program's last `depthf` takes the
`Fsat` result of the per-fragment value (not a hoisted constant), and that the
draw words carry DEPTH_FEEDBACK for each of the five depth formats, against an
FBO with an RGBA8 colour (no z-only frame refusal). Where the
`tests/es3_pvr_fragdepth.rs` helpers fit, they are reused, not copied.

Owner: src/shaderc/online/powervr (the depth source), src/pvr (pass and ZLS
words).

## 9. Not changed by these designs

- `docs/es3/sized-formats.md` (tri-fbo's intended home for the triage, which never
  landed) is not edited here.
- No line of the 167 is expected to have changed between `b25997e4` and the tip
  (tri-fbo: every relevant PowerVR change was already in that build). board-a's
  rows 1-2 at `31cdaefc` are the next measurement.
