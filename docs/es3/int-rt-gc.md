<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: ES 3.0 integer colour render targets (int-rt-gc)

Item int-rt-gc of plan #19 (ES 3.0), on top of int-rt-gl (the GL core's
half: the nineteen integer formats as renderbuffer storage and FBO targets,
`glClearBuffer{i,ui}v`, integer `glReadPixels`, and a refusal by name of a
format a back end has not learned, `backend::rt_format_drawable`).

## 1. What the GC7000UL now draws

`backend::INTEGER_RT_FORMATS` is `GC_INTEGER_RT_FORMATS` on the GC7000
(`src/vivante/backend.rs`): **17 of the 19** integer surfaces, those of 1, 2,
4 and 8 bytes a pixel. The four of 8 bytes (RGBA16I/UI, RG32I/UI,
`GC_INT_RT_64BPP`) are drawn through the tiled shadow RGBA16F takes, as the
only draw buffer (below the table); two (RGBA32I/UI) the hardware has no
format for (section 2). Per surface, the pixel engine format (`pe_color_format`, written to `PE_COLOR_FORMAT`'s
`FORMAT_EXT` with `FORMAT_MASK`, or to `PE_RT_CONFIG_8`'s format for draw
buffers 1..3) and the fragment stage's output mode (`shader.rs`
`ps_color_output_mode`, `PS_CONTROL_EXT.OUTPUT_MODEn`):

| GL format | PE format (`state_3d.xml.h`) | output mode (`state_3d.xml.h`) | bytes |
|---|---|---|---|
| R8I / R8UI | `PE_FORMAT_R8I` 0x17 (`:104`) | `I8` 5 / `U8` 3 (`:135`, `:133`) | 1 |
| RG8I / RG8UI | `PE_FORMAT_G8R8I` 0x18 (`:105`) | `I8` / `U8` | 2 |
| RGBA8I / RGBA8UI | `PE_FORMAT_A8B8G8R8I` 0x19 (`:106`) | `I8` / `U8` | 4 |
| R16I / R16UI | `PE_FORMAT_R16I` 0x1a (`:107`) | `I16` 6 / `U16` 4 (`:136`, `:134`) | 2 |
| RG16I / RG16UI | `PE_FORMAT_G16R16I` 0x1b (`:108`) | `I16` / `U16` | 4 |
| RGBA16I / RGBA16UI (tiled shadow) | `PE_FORMAT_A16B16G16R16I` 0x1c (`:109`) | `I16` / `U16` | 8 |
| R32I / R32UI | `PE_FORMAT_R32F` 0x14 (`:101`) | `UIF32` 2 (`:132`) | 4 |
| RG32I / RG32UI (tiled shadow) | `PE_FORMAT_G32R32F` 0x15 (`:102`) | `UIF32` | 8 |
| RGB10_A2UI | `PE_FORMAT_A2B10G10R10UI` 0x1e (`:111`) | `A2B10G10R10UI` 1 (`:131`) | 4 |
| **RGBA32I / RGBA32UI** | **none**: refused, section 2 | | 16 |

**The 64-bit ones: the tiled shadow.** They were held back (item
int-rt-gc) while gl_test case 44 had the PE's 64-bit LINEAR write open. The
probe settled it (`src/vivante/wide_target.rs`, module doc; #6, `cba8b199`,
record `got 0x20000004 want 0x00c2073e`): the PE misplaces 64-bit pixels in a
LINEAR target (16-byte pixel pairs move inside a row, at every pitch), so a
draw into any 64-bit-pixel colour target renders into a 4x4-tiled shadow
instead, as Mesa only ever renders tiled targets on this core. `wanted()`
picks the shadow by pixel size (`gles_surf_bytes_per_pixel == 8`), not by
format, and its copies move whole 8-byte pixels, so RGBA16I/UI and RG32I/UI
take the path RGBA16F already draws through with nothing format-specific to
add: the shadow's page-aligned address in `PE_PIPE_COLOR_ADDR`, a
`PE_COLOR_STRIDE` of the width padded to 16 times 8, `PE_LOGIC_OP`
`SINGLE_BUFFER(2)` (Mesa's value for a tiled target above 16 bits a pixel,
`etnaviv_state.c:536-539`), no `SUPER_TILED` bit, the linear storage tiled in
before the first draw of a kick and untiled back when the kick retires.
`GC_INT_RT_64BPP` is now inside `GC_INTEGER_RT_FORMATS`.

The words, pinned by `tests/es3_int_rt_gc.rs` for a full-mask draw and for
`glClearBuffer{i,ui}v`: `PE_COLOR_FORMAT` `0x1c010f10` for RGBA16I/UI
(`FORMAT_EXT` `A16B16G16R16I` 0x1c, `FORMAT_MASK` bit 4, `COMPONENTS` 0xf,
`OVERWRITE`; bit 31, `FORMAT_EXT`'s own write mask, clear: the mask-bit
lesson of `VIVANTE_NOTES.md`, "RGBA16F targets", `etnaviv_state.c:282-283`)
and `0x15000710` for RG32I/UI (`G32R32F` 0x15; a target without alpha keeps A
out of `COMPONENTS` and so loses `OVERWRITE`, as every alpha-less target
does). Output modes `I16`/`U16` (RGBA16I/UI) and `UIF32` (RG32I/UI); no
saturate; `PE_ALPHA_CONFIG` 0 with `GL_BLEND` on (the GL core clears
`GlesDraw::blend` for an integer attachment 0).

The shadow has two limits (`wide_target.rs`, "Not covered"). A draw with
**more than one colour target** cannot use it: the 8-RT bank's `RT_CONFIG`
has no tiled bit of its own and `SINGLE_BUFFER` switches every target, so
such a draw keeps the linear path, where case 44 measured the PE misplacing
64-bit pixels. `draw_one` therefore refuses (`-ENOTSUP`, `GL_INVALID_OPERATION`
from the draw, nothing reaching the PE) any draw that has one of the four
64-bit integer surfaces beside another draw buffer, first or later
(`a_64_bit_target_beside_another_refuses_the_draw`): a refusal, not misplaced
pixels. RGBA16F there is still drawn linear, as before this item. Drawing
them in an MRT draw takes a tiled multi-target path (every target tiled,
section 5); whether the GL core should report such a framebuffer
`GL_FRAMEBUFFER_UNSUPPORTED` instead of failing the draw is open for a
person. The design for lifting that refusal (every target of the draw on a
tiled shadow, the order of the steps, the questions only the board can
answer) is `docs/notes/gc-int64-mixed-targets.md`; it is not implemented.
And the **depth surface** of a draw into a shadow is switched by
`SINGLE_BUFFER(2)` too; its layout under (2) is unmeasured (logged once when
a draw reaches it), for the board (section 5).

All line numbers are `driver/src/vivante/hw/state_3d.xml.h`, the rnndb
header etnaviv generates its own from (MIT; README there). The test reads
the values from that header by name, so a wrong constant in
`src/vivante/regs.rs` fails it.

Why these, and why the same PE format for both signs: the enum has one
integer format per channel layout (`:104-111`), and the output mode carries
the sign. The output-mode rule is etnaviv's `translate_output_mode`
(`etnaviv_translate.h:446`, already cited in `shader.rs` for the R32F arm):
32-bit channels are `UIF32` before anything else; a pure integer format with
10-bit channels, or any integer format on a pre-HALTI5 core, is
`A2B10G10R10UI`; otherwise `I8`/`I16` for signed and `U8`/`U16` for unsigned
by channel width. This core is HALTI5 (`regs.rs`: model 0x7000 rev 0x6204;
the caps doc's hwdb row has Halti0..5 set).

**The 32-bit-channel formats.** The enum has no 32-bit integer PE format,
only `R32F` and `G32R32F`; `UIF32` is the mode whose name says it stores
"UI or F32": the register's 32 bits as they are. So R32I/UI and RG32I/UI are
drawn as R32F / G32R32F under `UIF32`, blending off (ES 3.0 4.1.8; the GL
core clears `GlesDraw::blend` for an integer target) and no saturate. This is
inferred from the enum and the mode, not traced through etnaviv's format table
(section 4): **for the board to confirm**.

**Memory order.** The rnndb names list channels from the high bits down, so R
is the low byte, half or word of every format above: the R,G,B,A packed
order the GL core keeps an integer surface in (`backend::gles_surf_int_layout`,
`gles_surf_bytes_per_pixel`, which the test checks against each PE format's
size). No swap on read-back (`rt_rgba8888_is_bgra` is about RGBA8888 only).

**The rest of the draw.** `PS_CONTROL.SATURATE_RTn` was set for every
surface of the RGBA8 shader class, which the integer surfaces are too (they
have no float variant); it is now cleared for them, as etnaviv saturates
`util_format_is_unorm` formats only (the "checked and identical" note in
`VIVANTE_NOTES.md`, "Two rules we had written as constants"). Dithering is
off for every draw already (`PE_DITHER` all ones).

**Blending, per draw buffer.** The GL core turns blending off for a draw
whose attachment 0 is integer (`GlesDraw::blend`). Draw buffers 1..3 took
RT0's blend word (`PE_HALTI5_RT_ALPHA_CONFIG`) whatever their format, so an
integer buffer beside a blended RGBA8 one would have been blended; it now
gets 0 there (and the OVERWRITE hint), RGBA8 beside it keeps its blend. The
reverse case, an integer attachment 0 with a unorm draw buffer 1 and
GL_BLEND on, leaves the unorm buffer unblended: that is the GL core's single
`blend` flag, not the back end's (handoff).

**Clears.** `glClearBuffer{i,ui}v` is the clear quad as every clear is. The
GL core puts the four raw 32-bit values in `GlesDraw::clear_rgba`'s bits; the
quad's shader is one `mov` of its constant to the colour output; the target's
output mode is programmed from the target's format, not the program, so the
clear stores the integers. Tested: the constant window holds the four values
bit for bit, and the clear's `PE_COLOR_FORMAT` / `PS_CONTROL_EXT` are the
table's.

**Shader outputs, unconverted.** The online compiler (`src/shaderc/online/
vivante`) already keeps an `ivec4`/`uvec4` output in the integer register it
computed: no conversion is emitted on the way out. `tests/es3_int_rt_gc.rs`
runs the encoded programs in the interpreter of `tests/common` and checks the
output register's bits (a `uvec4` past 2^31, an `ivec4` wrapping to
`INT_MIN`), that integer arithmetic emits no `i2f`/`f2i`, that a float made
integer by `int()` is converted exactly once (the shader's own `f2i`), and that
a uniform moved to the output is one `mov`. No compiler change was needed, so
`src/vivante/internal.rs` is unchanged.

## 2. What the hardware cannot render: RGBA32I and RGBA32UI

The PE format enum has **no 128-bit format of any kind**
(`state_3d.xml.h:89-113`: the widest are the 64-bit `A16B16G16R16F`,
`G32R32F`, `A16B16G16R16I`). Mesa's etnaviv renders a 128-bit format as two
planes, which is why it keeps half of the core's eight render targets back:
`max_render_targets = num_rts / 2` (`etnaviv_screen.c:330` at Mesa 4550a8d,
quoted in `src/vivante/VIVANTE_NOTES.md`, "Draw buffers"; at 26.2.1 the same
cap is `etnaviv_screen.c:299-304,774-782`, and `gpu_supports_render_format`
gates 128-bit formats on `HALF_FLOAT`, `:500-505`, both as cited by the
es3-caps item). That two-plane path (two PE targets per GL attachment, the
shader's output split across two registers) is not implemented here.

So RGBA32I and RGBA32UI are **refused by name, never drawn as garbage**: their
bits are not in `GC_INTEGER_RT_FORMATS`, the GL core's
`refuse_undrawable_targets` (`src/gl/draw.rs`) logs `draw refused: this back
end cannot render into a RGBA32UI colour attachment` once and the draw or
clear sets `GL_INVALID_OPERATION`, and `draw_one` refuses such a target with
`-ENOTSUP` as well should anything reach it. The test checks both: the error
and that no `PE_COLOR_FORMAT` was written.

ES 3.0 lists RGBA32I/UI as required colour-renderable formats, so the
conformant answer is `GL_FRAMEBUFFER_UNSUPPORTED` from
`glCheckFramebufferStatus` (an implementation may refuse a combination of
internal formats, ES 3.0 4.4.4.2), not an error at the draw;
`glGetInternalformativ` cannot refuse a required format. The framebuffer is
still reported complete today, because turning a non-drawable surface into
`FRAMEBUFFER_UNSUPPORTED` is a GL-core change (`src/gl/framebuffer.rs`) that
moves the V3D's and the GC7000's answers together. It is left to a GL-core
item (handoff).

The plane path is in the tree as inert code and notes; **no draw uses it, and
RGBA32I/UI stay refused by name as above.** `src/vivante/wide_target.rs` has
the pure parts: `split_planes` and `join_planes` (a linear image of 16-byte
pixels into two 4x4-tiled 8-byte planes, R,G then B,A, and back, with
`plane_offset` and `planes_bytes`), `plane_masks` and `plane_clears` (the two
planes' colour write masks and 64-bit clear words), and a shadow that holds
several targets (`wanted_targets`, `prepare_targets`, `bind_targets`), a
128-bit one as two planes. `backend.rs` calls none of the new functions. The
notes are `docs/notes/gc-128bit-render-targets.md` (the layout, the
implementation sequence and the board probe),
`docs/notes/gc-128bit-rt-shader.md` (the shader's colour-output split, the
render-side analogue of the sampler join) and
`docs/notes/gc-int64-mixed-targets.md` (the multi-target shadow's
draw-side design). What is left, in the sequence's order: a board probe
(one RGBA32UI attachment as two `G32R32F` tiled targets with `UIF32`; if the
bits do not match, the refusal stays and the cause is the finding); then, in
`backend.rs`, taking `GC_INT_RT_128BPP` out of the refusal and drawing the
planes through the multi-target shadow; the shader output split in
`src/shaderc/online/vivante/` and its draw key in `src/gl/draw.rs`; the plane
masks and clears in the back end; the GL-core name lists
(`refuse_undrawable_targets`, `rt_format_drawable`); tests that turn the
refusal tests into draw tests; and a board run of dEQP `fbo.completeness`
and `renderable.texture.*32*`.

## 3. The other ES 3.0 colour-renderable formats

| GL format | how the GL core stores it | PE format used | PE format the hardware has |
|---|---|---|---|
| R8, RG8 | RGBA8 (`framebuffer.rs` `is_color_format`) | `A8R8G8B8` | `PE_FORMAT_R8` 0x23, `G8R8` 0x1f (`:113`, `:112`) |
| RGB10_A2 | RGBA8 (ES 3.0 build) | `A8R8G8B8` | `PE_FORMAT_A2B10G10R10` 0x16 (`:103`) |
| RG16F | RGBA16F | `A16B16G16R16F` | `PE_FORMAT_G16R16F` 0x12 (`:99`) |
| R11F_G11F_B10F | not a colour-renderable name in the GL core | | `PE_FORMAT_B10G11R11F` 0x1d (`:110`) |
| RG32F | not a colour-renderable name | | `PE_FORMAT_G32R32F` 0x15 (`:102`) |
| RGBA32F | not a colour-renderable name | | none (128-bit, section 2) |
| SRGB8_ALPHA8 | `GLES_SURF_SRGB8_A8`, drawn: the A8R8G8B8 format with the PE sRGB bit | | docs/es3/srgb.md |

ES 3.0 requires none of R11F_G11F_B10F, RG32F or RGBA32F to be
colour-renderable (they are only with EXT_color_buffer_float). The narrow
native formats (R8, G8R8, A2B10G10R10, G16R16F) would save memory but need
`GLES_SURF_*` surfaces of their own in the GL core first; the wider storage
draws the same values.

## 4. Sources, and what was not available

* `driver/src/vivante/hw/state_3d.xml.h` (vendored rnndb header): every PE
  format and output-mode value above.
* `etnaviv_translate.h:446` (`translate_output_mode`): the 32-bit and
  non-integer arms were already transcribed in `shader.rs` by the state audit
  (2026-09-22). The integer arms were written down from that function **without
  a Mesa tree at hand**: this worker had no network and no etnaviv checkout
  (the vendored `third_party/mesa-26.2.1` carries the V3D and PowerVR drivers
  only). The mode values themselves are the header's. `etnaviv_format.c`'s
  per-format PE column (which PE format etnaviv gives R32_UINT and friends) and
  `etnaviv_emit.c`'s RT emission were not re-read for this item; the
  register-by-register MRT emission was checked against them by the MRT work
  (`VIVANTE_NOTES.md`, "Draw buffers").
* The hwdb (`gc_feature_database.h`) and `etnaviv_screen.c` lines are the
  es3-caps item's citations (branch `ccfarm/01a11ddf-...`,
  `docs/es3/backend-caps.md` §1.2, not yet on the integration branch).

## 5. For the board (i.MX8MP)

Nothing here has run on the GPU. What a session should check, cheapest first:

1. dEQP-GLES3 `functional.fragment_out.basic.int.*` / `.uint.*` for the 8-
   and 16-bit formats and RGB10_A2UI: the integer modes and PE formats.
2. The same for `r32i`/`r32ui`: confirms or refutes the R32F + `UIF32`
   inference (section 1). A wrong answer shows as wrong values, not a hang;
   if it fails, take the two bits out of `GC_INTEGER_RT_FORMATS` and they are
   refused by name like RGBA32.
3. The four 64-bit integer targets through the tiled shadow (section 1):
   a. `functional.fragment_out.basic.int.rgba16i_*`, `.uint.rgba16ui_*`,
      `.int.rg32i_*`, `.uint.rg32ui_*` (one target: the shadow path);
   b. `functional.fragment_out.array.*` and `.random.*` cases that put one of
      them beside another target: refused by `draw_one` (the linear
      multi-target path misplaces 64-bit pixels, case 44), so they fail
      with `GL_INVALID_OPERATION`, not wrong pixels; a tiled multi-target
      path (every target of the draw tiled) is what would draw them;
   c. `functional.color_clear.*` on an `rgba16ui`/`rg32i` renderbuffer, and
      `fbo.color.clear.rgba16i`/`rg32ui` style cases: the clear quad into the
      shadow, then read back after `glFinish` (the untile on retire);
   d. one such draw with a depth buffer attached
      (`fbo.render.*` with depth on an `rgba16ui` target, or gl_test 44 with
      depth): the depth surface under `SINGLE_BUFFER(2)`;
   e. `fbo.blit.conversion.*` into and out of the four (60 cases that drew
      nothing before this item, below): the blit's draw into the shadow
      and the read of a 64-bit integer source.

   Host reach of item 3 (`tools/cts/host-deqp-run.sh --gpu=vivante
   --module=gles3`, VK-GL-CTS `93bca018`, the `functional.fbo.` and
   `functional.fragment_out.` filters; HOST-ONLY: the Linux OSAL reads
   black, so only GL-error and incomplete-framebuffer failures count, never
   an image verdict). Before is this tree with `GC_INT_RT_64BPP` taken out
   of `GC_INTEGER_RT_FORMATS` again:

   | filter (cases) | GL-error/incomplete failures, before → after | of them naming rgba16i/ui or rg32i/ui |
   |---|---|---|
   | `fbo` (2,028) | 390 → 294 | 128 → 32 |
   | `fragment_out` (940) | 198 → 150 | 96 → 48 |

   The 48 `fragment_out.basic.{int,uint}` cases of the four draw; the 48
   `fragment_out.array.*` ones of them (several targets of the format) are
   refused by `draw_one`, as item 3b says. Of `fbo`, 96 cases draw now
   (`color`, `render`, `blit`, `completeness`, `invalidate` on the four);
   the 32 left were the RGBA32I/UI blits (8) and RG32I/UI *texture*
   attachments (24: the GL core had no RG32I/UI texture class then), not the
   render target. The attachments have a class and surface rows since
   `b5e04b14` and `6c5b004f` (R32I/UI and RG32I/UI; host test
   `tests/es3_int32_rt_gc.rs`); the table was not re-measured. The RGBA32I/UI
   blits stay refused (section 2).
4. `functional.color_clear.*int*` and the `shaders.builtin_functions` groups
   that render into integer renderbuffers (es3-inventory's bucket for
   int-rt-gl): the clear quad's raw constants.
4a. R32I/UI and RG32I/UI as **texture** colour attachments
   (`fbo.completeness.renderable.texture.color0.r32i` and the like,
   `renderable.texture.*32*`): the classes are in `GC_SAMPLED_TEXFMTS` and
   `integer_texture_surface` has their rows, with the 64-bit RG32I/UI through
   the tiled shadow; host-tested only (`tests/es3_int32_rt_gc.rs`).
4b. A fourth colour target: `DRAW_TARGETS` stays 3, the back end's fourth
   output is host-tested (`tests/es3_mrt4_gc.rs`), and the board check is
   `docs/notes/gc-mrt4-board-check.md`. A 64-bit integer target beside any
   other is still refused (item 3b); RGBA32I/UI would take two PE targets,
   which makes a fourth target matter for them
   (`docs/notes/gc-128bit-render-targets.md`).
5. `rgba32i`/`rgba32ui` cases: refused (`GL_INVALID_OPERATION`, the log line),
   so they fail rather than pass; they stop failing only with section 2's
   two-plane path or with `FRAMEBUFFER_UNSUPPORTED` (NotSupported). Lifting
   the refusal by the plane path needs the board probe and the remaining
   steps of `docs/notes/gc-128bit-render-targets.md` (implementation
   sequence), the shader output split of `docs/notes/gc-128bit-rt-shader.md`,
   and the multi-target shadow of `docs/notes/gc-int64-mixed-targets.md`;
   section 2 lists them.

## 6. PowerVR (AM62P): fix designs for the fragment_out.array and fragment_out.random fails

Item fix-fragout-design of plan #400. The clusters come from the
tri-fragout triage (its report, 2026-11-25; probes at `31cdaefc`). The cases
are the `fragment_out` lines of `docs/es3/am62p-gles3-notpass.tsv` (tree row
13, ad2b5ed). Line numbers below are at `bdd2ba1c`. This section is a design:
nothing in it is implemented or run. Every host test named here is new and
uses the feature set `--no-default-features --features
host-test,gpu-powervr,shaderc-online`, the one in the headers of
`tests/pvr_f16_mrt_gl.rs` and `tests/es3_pvr_fragout_array.rs`. The GL-level
tests follow the pattern of `tests/pvr_f16_mrt_gl.rs`: renderbuffers through
the GL core, `glDrawElements` into the real `pvr_backend_draw`, then the
recorded end-of-tile (PBE) words are read back.

The triage's forecast at the tip, without any of the fixes below: 121 Fail
(`array` 72, `random` 49) and 115 NotSupported (no
`GL_EXT_color_buffer_float`; allowed). Cluster A,
`array.float.{rg16f,rgba16f}` (24), is fixed by `f3409705` and `c558647b`
(host tests `tests/es3_pvr_fragout_array.rs` and `tests/pvr_f16_mrt_gl.rs`),
so it has no entry here.

### 6.0 A GL-core change shared by B and E1 (named, not made)

The draws of clusters B and E1 are dropped with **no GL error**. The variant
compile fails, and `src/gl/draw.rs:1759-1765` logs `no fragment shader
variant for blend key 0x…` once, marks the variant `missing` and returns
`Err`. Later draws take the `missing` arm at `src/gl/draw.rs:1714`. The draw
then returns without setting an error, so dEQP sees no error and reports
"Image comparison failed".

The change: where the draw path receives that `Err`, set
`GL_INVALID_OPERATION`, as a back-end refusal (`-ENOTSUP`) does. It makes no
case pass. It turns a silent drop into a named failure, so a host dEQP run
shows these cases as GL-error failures. `src/gl/**` belongs to plan #397, so
the change is not made here. Fixes B1, B2 and D2/E1 below make the compile
succeed, so they do not wait on it.

### 6.1 Cluster B1: `array.{int,uint}.{rg32i,rg32ui,rgba16i,rgba16ui}` (48 Fail)

**Cause.** dEQP's array case writes three outputs into three attachments of
one 2-dword integer format. That is 6 dwords, past the pixel's four output
registers.

- The GL core keys the draw with the one-layout integer key
  `shc_blend_key_int_rt`. Every attachment has the same layout, so
  `shc_blend_key_mixed` returns `None` (`src/target_key.rs:140`,
  `all_same_int`).
- The compiler packs every output into registers and refuses past four:
  `src/shaderc/online/powervr/chain.rs:1470-1471` (`pack_int_output`,
  "integer colour outputs past the pixel's four output registers").
- If the shader did compile, the back end would refuse it too:
  `src/pvr/backend.rs:6772-6775` (`int_targets_drawn`,
  `REFUSE_ONE_LAYOUT_PAST_FOUR`).

The layout itself already has a program. `mrt_layout::layout(&[2,2,2])` is
r0-1, r2-3 and tile buffer 0: shape `(3,1)`, program `eot3_tb`
(`src/pvr/mrt_layout.rs:193-199`). The emit loop already handles a spilled
last attachment (`src/pvr/backend.rs:9790-9797`, `eot_doutu_src0_tb_n[3]`).

**Change** (`src/shaderc/online/powervr/chain.rs`, `src/pvr/backend.rs`):

1. `chain.rs`, `build_fragment_with` (`:790-812`): add an integer one-layout
   arm beside the `replace_class` arm. It applies when all of these hold:
   - `ShcIntRt::of_key(blend_key)` is `Some(rt)`, with no colour mask;
   - the shader writes `n > 1` consecutive `FragColor` locations;
   - `n <= MAX_PACK_TARGETS` and `n * rt.dwords() > 4`.

   The draw then becomes the per-target list `[TargetPack::Int(rt); n]` with
   blend key 0. The per-target path lays it out with the spill
   (`target_layout`, `chain.rs:1700`) and stores target 2 to tile buffer 0
   (`TargetOut::Packed`, `chain.rs:1243`). `pack_int_output` still serves
   `n * dwords <= 4`, so no record pinned today changes.
2. `backend.rs`, `int_targets_drawn` (`:6772-6780`): take the dword count
   from `mrt_layout::layout(&[per; n])` (`output_regs`, `tile_buffers`). The
   program check also asserts `fs.num_tile_buffers == lay.tile_buffers`. A
   layout with no program (`Err`) keeps `REFUSE_ONE_LAYOUT_PAST_FOUR`.
3. The clear of a spilled attachment. dEQP's executor clears every
   attachment with `glClearBuffer{i,ui}v` in the same frame as the draw.
   Today `mrt_clear_regs` (`backend.rs:2134`) returns 0 past four registers,
   and `attachment_out_reg` (`:2336`) returns `None` for a spilled
   attachment. So the clear of attachment 2 is either refused or goes through
   `clear_by_target_quads`, which draws one quad per attachment, each with
   its own program into registers. Let the host test below decide which. If
   the clear is refused, make `clear_by_target_quads` the path for a list
   with a spilled attachment: each quad draws one attachment as attachment 0
   of its own emit, so the clear never spills.

**Host test.** `tests/pvr_fragout_int_spill.rs` (new):

- `three_rg32ui_outputs_spill_the_third`: compiles a fragment shader with
  three `uvec2` outputs under `shc_blend_key_int_rt(RG32UI)`. It asserts that
  the record has `num_outputs == 4` and `num_tile_buffers == 1`, one
  tile-buffer store and four pixel-output writes.
- `three_rgba16i_attachments_draw`: builds an FBO of three RGBA16I
  renderbuffers, clears each with `glClearBufferiv` and draws with
  `glDrawElements`. It asserts:
  - `glGetError() == GL_NO_ERROR`, and no `draw refused` log line;
  - the kick's end-of-tile `doutu` equals `eot_doutu_src0_tb_n[3]`;
  - three PBE emits, with the RGBA16I `source_format` and packmode.
- A negative test: three RGBA32UI attachments stay at
  `REFUSE_ONE_LAYOUT_PAST_FOUR` until B2.

Before the change the compile refuses, the draw sets no error and no kick is
recorded, so the test fails; after the change it passes.

**src/gl:** none.

**Expected:** the 48 cases pass if the board draws a spilled integer target
the way the host records it. `eot3_tb` with an integer spill has not run on
the AM62P; so far only RGBA16F has used it (`f3409705`).

**Overlap with pvr-modf-mrt.** This is the same code with a different shape.
modf needs `[4,4]`, shape `(2,1)`, which also needs `eot2_tb1` uploaded as
`tb_n[2]` (modf-mrt-locate's design, steps a-d). Steps 1 and 2 here are its
steps c and d, generalised from `n == 2` to `n <= 3`. Implement them once, in
pvr-modf-mrt, with `n` up to `MAX_PACK_TARGETS`, and add this section's
GL-level test there. The alternative is to run B1 right after pvr-modf-mrt,
since both hold `chain.rs` and `backend.rs`.

### 6.2 Cluster B2: `array.{int,uint}.{rgba32i,rgba32ui}` (24 Fail)

**Cause.** As in B1, but with three 4-dword attachments: 12 dwords, layout
`[4,4,4]` = r0-3 plus tile buffers 0 and 1, shape `(3,2)`. Besides B1's two
refusals (`chain.rs:1471`, `backend.rs:6774`), three more places have no
program for this shape:

- `mrt_layout::program_for` (`src/pvr/mrt_layout.rs:193-199`) has no `(3,2)`
  arm;
- `target_layout` returns `None` for `(2,1)` and `(3,2)` (`chain.rs:1717-1718`);
- the emit loop lets only the last attachment come from a tile buffer
  (`backend.rs:9790-9797`).

The program exists: `pvr_usc_eot3_tb12_words` (`capture_usc eot 3 1 2`) is
captured and generated (`driver/src/pvr_usc_eot_blob.h:26`, `:112`) but never
uploaded.

**Change** (`tools/pco_shc/gen_usc_blobs.py`, then the generated
`driver/src/pvr_usc_eot_blob.h`; `driver/src/pvr_render.c`; `src/pvr/**`;
`src/shaderc/online/powervr/chain.rs`):

1. Upload `eot3_tb12` into a new `PvrRenderStatic` slot,
   `eot_doutu_src0_tb3_k[2]` with its PDS info, shaped like `tb4_k`. This is
   an ABI change: `src/abi_layout.rs`, `src/abi_layout_check.c` and the size
   pins move in the same commit.
2. `mrt_layout.rs`: add `EotProgram::Eot3Tb12` and a `(3,2)` arm in
   `program_for`, with `spilled_mask` `0b110` and `doutu` and `pds_info` from
   the new slot. The test that pins `(3,2)` as `NoProgram` flips.
3. `chain.rs:1717`: drop `(3,2)` from the refusal. Leave `(2,1)` to
   pvr-modf-mrt.
4. `backend.rs` emit loop (`:9784-9812`): instead of the "only the last
   attachment spills" rule, pick the program from `mrt_layout::layout` of the
   attachments' `surf_out_dwords` (`:1041`), and emit each spilled attachment
   from its tile buffer.

**Host test.** In `tests/pvr_fragout_int_spill.rs`:

- `three_rgba32ui_outputs_spill_two`: the record has `num_outputs` 4,
  `num_tile_buffers` 2 and two tile-buffer stores.
- `three_rgba32i_attachments_draw`: at GL level as in B1. No error, `doutu`
  equal to the new slot's, three PBE emits, and attachments 1 and 2 emitted
  from tile buffers 0 and 1.
- `cargo test --lib pvr::mrt_layout` covers step 2.

**src/gl:** none.

**Overlap:** modf-mrt-locate left `(3,2)` refused because nothing it fixes
needs it; B2 is the case that does. Its caveat applies here too: a continued
frame cannot reload a spilled attachment (`attachment_out_reg`,
`backend.rs:2336`). dEQP draws and reads back in one frame.

### 6.3 Cluster C: `random` seeds with four draw buffers (26: 0 8 17 21 24 25 29 33 41 45 55 56 66 67 70 71 72 75 78 79 82 86 88 90 94 96)

**Cause.** The seed asks for four draw buffers, but the PowerVR reports
`GL_MAX_DRAW_BUFFERS` 3 (`DRAW_TARGETS` 3 under `gpu-powervr`,
`src/backend.rs:901-913`; `src/gl/context.rs:231`). So `glDrawBuffers` fails
and the case fails at `:286`: INVALID_ENUM at the tip, "Compile failed" at
ad2b5ed, when the shader's fourth output was refused first.

Even with four draw buffers the mixes would still be refused:

- A mix has more targets than `SHC_MIXED_MAX_TARGETS` (3,
  `src/target_key.rs:18`). The back end refuses it with `REFUSE_MIX_TARGETS`
  (`src/pvr/target_keys.rs:56-57`), and the GL core's `draw_target_keys`
  (`src/gl/target_keys.rs`) returns `None` at `n == SHC_MIXED_MAX_TARGETS`.
- The 32-bit mixed key cannot carry a fourth slot. Slots are 5 bits at
  `10 + 5 * i` (`src/target_key.rs:137`), so slot 3 would cover bit 25, the
  mixed tag (`:21`).

**Change**, in this order:

1. Four draw buffers on the PowerVR: `DRAW_TARGETS` 4 under `gpu-powervr`
   (`src/backend.rs:910-913`, which is not `src/gl`). The decision belongs
   to tri-limits and to the board check of `e329c01a` (four colour targets);
   the AM62P images do not set `CONFIG_PVR_DEQP_MRT4` today. On its own, this
   step moves the 26 cases from INVALID_ENUM to the back end's
   `REFUSE_MIX_TARGETS` (INVALID_OPERATION at `:810`), or to a draw for a
   same-class list.
2. Four-target mixes in the 64-bit multi key
   (`docs/notes/pvr-blend-multi.md`, sections 3.2-3.4). `shc_blend_key_multi`
   and its decoder already exist (`src/target_key.rs:243-265`). So does the
   compiler's `build_fragment_multi` (`chain.rs:687`), which has no caller
   outside `chain.rs`. In the back end:
   - `draw_target_keys` in `src/pvr/target_keys.rs` runs over
     `SHC_MULTI_SLOTS` (4);
   - `int_targets_drawn` accepts a multi-keyed record;
   - the emit loop changes as in B2 step 4, because a four-target mix spills
     (`program_for` `(4,1..=3)` exists).
3. Seeds 17, 67, 86 and 96 also need D1. All seeds except 24, 70 and 94 pass
   four registers, so they also need the spill of 6.4.

**Needs src/gl** (step 2; named, not made). This is the multi key's interface
from `pvr-blend-multi.md` 3.4:

- `ProgFsVariant::blend_key` and the program cache key become `u64`, and so
  does `blend_variant`'s key parameter (`src/gl/draw.rs`,
  `src/shaderc/mod.rs`);
- `current_blend_key` (`src/gl/draw.rs:1470-1483`) returns
  `shc_blend_key_multi` when four targets are present or no legacy key
  expresses the draw (the rule in 3.3);
- `draw_target_keys` in `src/gl/target_keys.rs` is sized by
  `SHC_MULTI_SLOTS`;
- `ShcShader::blend_key_hi` is added to the ABI (`src/abi_layout.rs`,
  `src/abi_layout_check.c`, `include/gles_shaderc.h`).

**Host test.**

- Step 1: `tests/pvr_draw_buffers4.rs`. `glGetIntegerv(GL_MAX_DRAW_BUFFERS)`
  returns 4, and `glDrawBuffers` of four attachments sets no error. This
  moves pins of 3 like those in `tests/es3_caps.rs`; grep `tests/` for them.
- Step 2: `tests/pvr_multi_key_mix4.rs`, for the formats of seeds 24, 70 and
  94 (re-derived with dEQP's generator, as the triage did). The GL core's key
  has the multi tag, the compile succeeds, and the draw sets no error and
  records four PBE emits, each with its attachment's packing.

### 6.4 Clusters D2 and E1: a mix past four registers (D2: random 9 11 18 31 50 53 62 85 89 92 97; E1: random 6 42 43 52 87 93 98)

These are one cause with two symptoms.

**Cause.** The draw mixes an integer target with another target, and their
registers pass four. `shc_mixed_out_layout` (`src/target_key.rs:90-115`)
returns `None` at `:105` instead of spilling, so `shc_blend_key_mixed`
returns `None` (`:140`).

- D2: attachment 0's own key compiles (a 1-dword target), so the draw reaches
  the back end. `draw_target_keys` refuses it (`src/pvr/target_keys.rs:66-67`,
  `REFUSE_FOUR_REGISTERS`): INVALID_OPERATION at `:810`.
- E1: with no mixed key, the GL core falls back to attachment 0's key
  (`src/gl/draw.rs:1480-1491`). Attachment 0 is an integer format of 2 or more
  dwords, so the compiler packs every output under it and refuses
  (`chain.rs:1470-1471`). The draw is dropped silently (6.0).
- Seeds 50, 85 and 97 pass four registers only because of the alignment
  gap: a 2-dword target after a 1-dword one starts at the next even register
  (`:101-103`).

**Change** (`src/target_key.rs`, `src/pvr/target_keys.rs`,
`src/pvr/backend.rs`; `chain.rs` already lays out mixes with spills):

1. `src/target_key.rs`: add `shc_mixed_out_places(targets) ->
   Option<([Place; 3], output_regs, tile_buffers)>`. It follows the
   `mrt_layout`/`target_layout` rule: each target goes in registers while it
   fits, then it and every later target go to the next tile buffer. It
   returns `None` only for a shape with no captured program: `(2,1)` and
   `(3,2)`, until B2 and pvr-modf-mrt wire them.
   - `shc_blend_key_mixed` and `src/pvr/target_keys.rs:66` call it.
   - `shc_mixed_out_layout` keeps its signature and meaning ("fits in
     registers"). So the GL core's test at `src/gl/draw.rs:8501` and the
     tests at `src/pvr/backend_tests.rs:6116` and `:6183` do not change.
2. `src/pvr/backend.rs`, the mix arm of `int_targets_drawn` (`:6788-6790`):
   take the span and tile buffers from step 1, and add `num_tile_buffers` to
   the record check. `attachment_out_reg` (`:2336`) and the emit loop follow
   B2 step 4, so a spilled mix target is emitted from its tile buffer.
3. The alignment gap (seeds 50, 85, 97) stays unfilled: with step 1 these
   seeds spill instead. Filling the gap (putting a 1-dword target into it)
   would change the emit order and the frame load (`attachment_out_reg`)
   together, so it is not proposed.

The GL core already keys through `shc_blend_key_mixed` (`draw_mixed_key` in
`src/gl/target_keys.rs`). So once step 1 accepts these lists, the GL core
keys these draws mixed with **no src/gl change**. Only the PowerVR uses the
mixed key: its compiler builds it and its back end reads it (`grep -rl
shc_blend_key_mixed src` finds only `src/gl`, `src/pvr` and
`src/target_key.rs`). So accepting more lists changes nothing for the other
back ends.

**Host test.** `tests/pvr_mixed_key_spill.rs` (new). It covers each D2/E1
seed's format list (from dEQP's generator), including at least one list with
attachment 0 RG32UI, one with RGBA8 + RGBA32I, and seed 50's gap list. For
each list it asserts:

- the GL core's key is `shc_blend_key_mixed` of the list, not attachment 0's
  key;
- the compile succeeds with `num_tile_buffers` 1;
- the draw sets no error and logs neither `draw refused` nor `no fragment
  shader variant`;
- the spilled attachment's PBE emit reads the tile buffer.

`src/target_key.rs` gets unit tests for `shc_mixed_out_places`: every list of
up to three keys goes to registers and then spills, and the function returns
`None` exactly for `(2,1)` and `(3,2)`. The change moves the pins in
`tests/pvr_target_keys_refusals.rs` and `tests/pvr_mixed_key_spec.rs` that
assert `REFUSE_FOUR_REGISTERS` for these shapes (grep for them).

**Overlap with fbo-rint:** a seed with an R or RG integer attachment passes
only once fbo-rint's R/RG integer fix (render and read back) is in as well.

### 6.5 Cluster D1: SRGB8_ALPHA8 beside an integer attachment (random 1, 64, 69; with four targets also 17, 67, 86, 96)

**Cause.** `src/pvr/target_keys.rs:53-54` refuses the draw (`REFUSE_SRGB`):
INVALID_OPERATION at `:810`. An sRGB format has no per-target key:
`gles_surf_target_key` returns `None` for it (`src/target_key.rs:65`), and
the multi key reserves the sRGB slot codes 0x04, 0x08 and 0x0C
(`multi_slot_ok`, `src/target_key.rs`, just above `:243`).

**Change** (`src/target_key.rs`, `src/pvr/target_keys.rs`,
`src/shaderc/online/powervr/chain.rs`):

- Give SRGB8_ALPHA8 slot code 0x04, an RGBA8 class that encodes, in both the
  mixed and the multi key.
- The compiler encodes that slot's colour from linear to sRGB in the program
  before packing it as RGBA8 bytes. The all-sRGB path already does this
  (`srgb_targets_as_rgba8`, `src/pvr/backend.rs:4308`): the attachment is
  stored as RGBA8 whose bytes are already encoded.
- `draw_target_keys` drops the `REFUSE_SRGB` arm.

The GL core gets the key through `gles_surf_target_key` and its own
`draw_target_keys` (which applies `?` to it). So there is **no src/gl
change** for up to three targets; four targets need the change in 6.3.

**Host test.** `tests/pvr_mixed_key_srgb.rs`, on RGBA32UI + SRGB8_ALPHA8
(seed 1's list). It asserts:

- slot 1 of the key is 0x04;
- the compiled record encodes slot 1: a pinned linear 0.5 gives byte 0xbc;
- the draw sets no error;
- the PBE emit of attachment 1 is RGBA8 with no sRGB conversion bit.

### 6.6 Clusters D3 and E3: float-only mixes of different classes (D3: random 65, RGBA16F + R8; E3: random 3, R8 + R16F)

**Cause.** No key describes a float-only mix. `shc_blend_key_mixed` requires
an integer target (`src/target_key.rs:140`, `!any_int`), so the GL core keys
the draw with attachment 0's class.

- D3 (attachment 0 RGBA16F): the compiler lays out every output as RGBA16F
  (`replace_class`, `chain.rs:727`, `:797-808`). The back end sees a
  per-target float program beside another class and refuses it
  (`src/pvr/backend.rs:6916-6931`, "several colour outputs of one float class
  beside an attachment of another class"): INVALID_OPERATION.
- E3 (attachment 0 RGBA8): the RGBA8 key is not a `replace_class` key, so
  `per_target_float_class` (`backend.rs:6822-6830`) returns `None` and the
  refusal above does not run. The program writes output 1 as RGBA8 bytes,
  but the PBE of the R16F attachment reads register 1 as F16 (packmode 51,
  `source_format` 0, probed). The result is a **silent wrong picture**.

**Step 1** (`src/pvr/backend.rs` only): turn E3 into a named refusal. The
case still fails, but with an error. Beside the `per_target_float_class`
check (`:6916`), refuse by name any draw where all of these hold:

- `ntargets > 1`, with no integer attachment and no mixed or multi key;
- the attachments' `gles_surf_rt_class` values differ;
- the program is keyed for one class, RGBA8 included.

The refusal uses the same log line and returns `-ENOTSUP`.

**Step 2** (draws these cases): the multi key of 6.3 step 2. By 3.3's rule, a
draw of non-uniform class that no legacy key expresses gets a multi key.
Dropping `!any_int` from the 32-bit mixed key instead is not proposed: the GL
core would then key float-only mixes as mixed for every back end
(`src/gl/draw.rs:1480`), and the V3D and GC7000 compilers do not accept that
key.

**Needs src/gl** for step 2: the same multi-key interface as 6.3 (named
there). Step 1 needs no src/gl change.

**Host test.**

- Step 1: `tests/pvr_float_mix_refused.rs`. It uses R8 + R16F renderbuffers
  (seed 3's list) and a two-output shader, drawn with `glDrawElements`.
  - Before the change, the draw records a kick whose PBE 1 is F16 while the
    program packs RGBA8.
  - After it, `glGetError() == GL_INVALID_OPERATION`, the log line appears
    once and no kick is recorded.
  - Same-class lists (RGBA16F + RGBA16F, RGBA8 + RGBA8) still draw.
- Step 2: in `tests/pvr_multi_key_mix4.rs` (6.3), R8 + R16F and RGBA16F + R8
  draw with a multi key and per-attachment packing.

### 6.7 Summary

| cluster | cases | files | src/gl | host test (new) |
|---|---|---|---|---|
| B1 | 48 | `chain.rs`, `src/pvr/backend.rs` | none (6.0 optional) | `tests/pvr_fragout_int_spill.rs` |
| B2 | 24 | `gen_usc_blobs.py`, blob header, `pvr_render.c`, ABI, `mrt_layout.rs`, `chain.rs`, `backend.rs` | none | `tests/pvr_fragout_int_spill.rs` |
| C | 26 | `src/backend.rs` (`DRAW_TARGETS`), `src/pvr/target_keys.rs`, `backend.rs` | multi key (6.3) | `tests/pvr_draw_buffers4.rs`, `tests/pvr_multi_key_mix4.rs` |
| D1 | 3 (+4) | `src/target_key.rs`, `src/pvr/target_keys.rs`, `chain.rs` | none | `tests/pvr_mixed_key_srgb.rs` |
| D2 + E1 | 11 + 7 | `src/target_key.rs`, `src/pvr/target_keys.rs`, `backend.rs` | none (6.0 optional) | `tests/pvr_mixed_key_spill.rs` |
| D3 + E3 | 1 + 1 | `backend.rs` (step 1); multi key (step 2) | step 2: multi key | `tests/pvr_float_mix_refused.rs` |

Suggested order:

1. B1, together with pvr-modf-mrt (same functions).
2. 6.4, which shares the emit-loop change with B2.
3. B2.
4. D1.
5. 6.6 step 1.
6. The multi key (6.3 step 2 and 6.6 step 2). This is the only part that
   needs `src/gl`.

None of this is proven on the board. Each fix needs an AM62P run of row 13
(`fragment_out`).
