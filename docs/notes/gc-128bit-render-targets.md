<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: 128-bit render targets as Mesa etnaviv handles them

What Mesa's etnaviv does with RGBA32I, RGBA32UI and RGBA32F colour
buffers on a HALTI5 GC7000, and what it means for `src/vivante/**`.
Read from Mesa 26.2.1 (tag `mesa-26.2.1`, commit
`da14d65e4499e66468094be52bff9ea0915a695e`, checked out whole; only
`third_party/mesa-26.2.1/UPSTREAM` is vendored). Paths are
`src/gallium/drivers/etnaviv/` unless they start with `src/etnaviv/`. The
core is the GC7000UL, model 0x7000, revision 0x6204
(`src/etnaviv/hwdb/nxp/gc_feature_database.h:18841-18842`, row
`gc7000ULN_v123`). Its row has `HalfFloatPipe` 1 (`:19026`) and Halti0 to
Halti5 all 1 (`:19038`, `:19058`, `:19127`, `:19152`, `:19157`, `:19172`).
Nothing here was run: it is Mesa's source, not a measurement of the
i.MX8MP.

This corrects `docs/es3/int-rt-gc.md` section 2, which says the hardware
has "no hardware format" and that the plane path is "not implemented
here". The first half is true of one PE format and not of the problem:
Mesa draws the 128-bit formats, on this core, as two 64-bit planes.

## (1) Does Mesa offer RGBA32I/UI/F as render targets here? Yes

`gpu_supports_render_format` takes the 128-bit formats before the generic
pure-integer and float test:

```
if (format_is_128bit(format))
   return VIV_FEATURE(screen, ETNA_FEATURE_HALF_FLOAT);   /* etnaviv_screen.c:501-503 */
```

It first needs `translate_pe_format(format) != ETNA_NO_MATCH`
(`etnaviv_screen.c:474-477`), which holds because the three formats have a
PE column, `G32R32F` (`etnaviv_format.c:229-230,233`). On this core
`HALF_FLOAT` is 1 (`gc_feature_database.h:19026`), so
`etna_screen_is_format_supported` with `PIPE_BIND_RENDER_TARGET` allows
all three (`etnaviv_screen.c:574-575`). The same format is also a sampler
view on HALTI5 (`:424-430`, integers need HALTI5, floats HALF_FLOAT).
`format_is_128bit` names exactly `R32G32B32A32_FLOAT`, `_SINT` and
`_UINT` (`etnaviv_format.h:86-96`).

The cost is half the render targets. The core has 8 (`num_rts`, HALTI2 or
later: `etnaviv_screen.c:774-777`) and the screen reports
`max_render_targets = num_rts / 2` = 4 whenever the core is HALTI2
(`etnaviv_screen.c:299-304`), "to reserve half of the available render
targets for emulation". The comment names float formats; the same
reserve serves the integers, since they use the same machinery.

## (2) How: two G32R32 planes, written as two MRT slots

There is no 128-bit PE format. The PE columns of the three rows are the
64-bit `G32R32F` for float and for both integer classes
(`etnaviv_format.c:229-230,233`, "emulated format"), PE enum value 0x15
(`src/etnaviv/hw/state_3d.xml.h:101`; ours `driver/src/vivante/hw/state_3d.xml.h:102`).
The integer classes also render as `G32R32F`, the same as
`R32G32_SINT/UINT` and `R32_SINT/UINT` do (`etnaviv_format.c:136-137,209-210`),
with the shader's bits stored raw. Mesa sets `COLOR_OUTPUT_MODE_UIF32`
for every 32-bit-component target (`etnaviv_translate.h:417-424`; enum
`state_3d.xml.h:131` in Mesa, 0x2, "UI or F32"), so an integer value goes
through the pipeline as its bit pattern.

**Resource layout.** The same two stacked planes as the 128-bit texture
(`docs/notes/gc-128bit-texture-sampling.md`). `setup_miptree` takes the
stride and layer stride of the 64-bit twin
(`translate_format_128bit_to_64bit`, `etnaviv_format.h:70-83`) and doubles
the level size (`etnaviv_resource.c:315-338`). Plane 1 (B,A) starts
half-way into the level, `size * depth / 2`
(`etna_resource_level_second_plane_offset`, `etnaviv_resource.h:91-94`).
The resource is always `ETNA_LAYOUT_TILED` (`etnaviv_resource.c:592-594`)
and never has tile status (`:165-167`).

**Framebuffer: one GL attachment becomes two hardware targets.**
`etna_fb_expand_128bit` (`etnaviv_state.c:135-165`) runs first in
`etna_set_framebuffer_state` (`:229`). For each 128-bit colour
buffer `i` it appends a copy of the same surface in the next free slot
(`:156-160`, `fb->cbufs[next_slot] = fb->cbufs[i]`, same resource, same
format), and records:

- `rt_offset[next_slot]` = the plane-1 offset (`:158`), added to the
  slot's PE address for both pixel pipes (`:277-278`);
- `rt_companion[i]` = `max_render_targets + k` (`:141,153`), the shader
  output index of the B,A half (4, 5, ... on this core);
- `rt_output[next_slot]` = that same index (`:159`); and
  `companion_src[next_slot] = i` (`:154`).

Each hardware slot is then programmed like any render target:

- PE format: slot 0 in `PE_COLOR_FORMAT` as an extended format
  (`fmt >= PE_FORMAT_R16F`, `:281-284`), slots 1 and up in
  `PE_RT_CONFIG[rt-1]` (`:329-331`), both `G32R32F`. Stride is the
  64-bit twin's (`:302`, `level->stride`). Super-tiled bits only if the
  layout is (`:288-296,332-336`); the 128-bit layout is plain TILED.
- Output mode: `translate_output_mode(surf->format, halti5) << (4*rt)` into
  `PS_CONTROL_EXT` (`:377`) = UIF32 for both planes
  (`etnaviv_translate.h:419-424`). Saturation is off (not unorm, `:364-366`).
- `RT_COUNT(rt - 1)` counts the planes (`:389-391`).
- `ps_output_remap[rt] = rt_output[i]` (`:383`): the shader's output index
  for the slot. `etna_link_shaders` turns it into `PS_OUTPUT_REG`,
  `fs->ps_color_out_reg[remap[i]]` (`etnaviv_shader.c:205-221`).
- TS: no tile status for either plane (`etnaviv_resource.c:165-167`;
  `etna_fb_rt_ts_mask`, `etnaviv_state.c:168-195`, skips a level without
  `ts_size`). `COLOR_TS_MODE` stays off for them.
- Single buffer: `PE_LOGIC_OP.SINGLE_BUFFER` is 2 for a tiled target on a
  SINGLE_BUFFER core, "one switch ... not a separate one per color
  buffer" (`:532-539`). So every target of the draw, the planes too,
  is tiled.
- Colour mask and blend: the companion has no blend state of its own.
  `etnaviv_blend.c:153-163` shifts the user's mask down, `(mask >> 2) & 3`,
  so the B,A bits mask the plane's R,G. Slot 0 takes the user's R,G mask
  from `rt[0]` as usual.

**Shader output.** `etna_nir_lower_128bit`
(`etnaviv_nir_lower_128bit.c:173-212`, run from
`etnaviv_compiler_nir.c:1300` after `etna_nir_lower_texture`) rewrites a
store to the 128-bit colour output `FRAG_RESULT_DATA0 + i`:
`lower_128bit_output` (`:44-87`) splits the vec4 into `vec4(r, g, 0, 0)`
stored to the original output (type forced to vec4 of the same base) and
`vec4(b, a, 0, 0)` stored to a new `etna_128bit_companion` output at
`FRAG_RESULT_DATA0 + rt_companion[i]` (`:21-42`), honouring the
write mask. The bits are kept: no int-to-float conversion
(integers keep their own base type). The shader is a variant: the key
carries `has_128bit_rt`, `rt_is_128bit` and `rt_companion[]`
(`etnaviv_shader.h:65,77-78`), set from the framebuffer
(`etnaviv_context.c:399-402`), and the key compare takes the slow path
when either is set (`etnaviv_shader.h:85-91`). A vertex-stage or
fragment-stage sampler of a 128-bit texture is the same family: the
companion look-up joined into a vec4 (`:90-170`; yakogl has this half,
`src/shaderc/online/vivante/wide128.rs`, `docs/notes/gc-128bit-texture-sampling.md`).

**Clear.** Not by one operation. Without tile status the RS clears plane
0 with `(g << 32 | r)` and plane 1 with `(a << 32 | b)`, a
`VIVS_GL_FLUSH_CACHE` and a PE-to-RA stall between the two
(`etnaviv_rs.c:399-419`, plane offset `:414-416`). On a BLT core, two
`emit_blt_clearimage`, the second at the plane offset
(`etnaviv_blt.c:452-465`). The GC7000UL's BLT use is not read here:
`wide_target.rs` records no BLT engine on this core.

**Blit and copy.** The BLT path maps the format to its 64-bit twin and
copies twice (`etnaviv_blt.c:783-784,954-960`). The RS blit has no plane
code (`etnaviv_rs.c` names `format_is_128bit` only in the clear).

**CPU access.** A mapped 128-bit resource is tiled and untiled by
`etna_texture_tile_rgba32f` / `etna_texture_untile_rgba32f`, two 8-byte
planes in one pass (`etnaviv_tiling.c:136-158`, from
`etnaviv_transfer.c:235-243,489-496`).

## (3) The gate that refuses it

None, for this core. The only refusals are `HALF_FLOAT` missing
(`etnaviv_screen.c:501-503`), a missing PE format
(`etnaviv_screen.c:474-477`), and the attachment count: a 128-bit
colour buffer takes two of the eight slots, `etna_fb_expand_128bit` has
`assert(next_slot < PIPE_MAX_COLOR_BUFS)` (`etnaviv_state.c:150`) and
the screen reports 4 GL attachments (`etnaviv_screen.c:299-304`), so a
framebuffer of four attachments, all 128-bit, fills the eight slots. The
assert is the only driver check: the cap of four attachments is what keeps
a framebuffer within it.

## (4) What this means for yakogl

**"Refused by name" is not upstream's behaviour.** It is the state of our
back end, not a hardware limit. `src/vivante/backend.rs:301-304,330-332`
and `docs/es3/int-rt-gc.md` section 2 should say "not implemented: Mesa
draws it as two G32R32 planes" when the work lands. RGBA32F is not
affected today (ES 3.0 does not require it as colour-renderable without
`EXT_color_buffer_float`).

**What works to our advantage.** The planes use words the back end has:
`PE_FORMAT_G32R32F` (`GC_PE_FORMAT_G32R32F`, already used for RG32I/UI,
`backend.rs:pe_color_format` line 307), `COLOR_OUTPUT_MODE_UIF32` (`regs.rs`
`GC_PS_OUTPUT_MODE_UIF32`), the 8-RT bank (`regs.rs` `pe_rt_addr_8_pipe`,
`pe_rt_config_8`), and the tiled shadow (`wide_target.rs`). The
texture-plane layout of `gc-128bit-texture-sampling.md` is the render
layout too. Our `PE_*` constants equal Mesa's for every `PE_FORMAT_*`,
`COLOR_OUTPUT_MODE_*` and `TEXTURE_FORMAT_EXT_*` name (`diff` of the
`#define` lines of `driver/src/vivante/hw/state_3d.xml.h` and
`common_3d.xml.h` against `src/etnaviv/hw/` of 26.2.1: no difference).

**Work a later item needs** (none of it done here):

1. `src/vivante/backend.rs`: remove `GC_INT_RT_128BPP` from the refusal
   (`:351`, `GC_INTEGER_RT_FORMATS` `:340`, `pe_color_format` `:307`, the
   `draw_one` check near `:2776-2800`). `mrt_targets` (`:501`) clamps to
   four hardware targets and `hw_targets` (`:517`) returns `[u8; 4]`: a
   128-bit attachment takes two, so the clamp must reach the full 8-RT
   bank (an attachment count of up to four still gives up to eight
   targets). The planes are `PE_COLOR_FORMAT` for slot 0 and
   `pe_rt_config_8(rt)` and `pe_rt_addr_8_pipe(rt, pipe)` beyond it, the
   second at plane offset.
2. `src/vivante/wide_target.rs`: it is single-target today
   (`wanted(gl, single, straight)` at `:137`, "Not covered: a draw with
   more than one colour target"). A 128-bit draw has at least two. The
   PE's single `SINGLE_BUFFER` switch tiles every target of the draw
   (`etnaviv_state.c:532-539`), so a draw that mixes a 128-bit target
   with a 32-bit one needs every target on a tiled shadow, not only the
   wide ones. This is the main piece: the shadow must serve more than one
   target, and its tile and untile (`bind`, `prepare`, `retire`, `:155-272`)
   must split a 16-byte pixel into the two 8-byte plane halves (R,G and
   B,A) and join them back. `tiled_copy.rs` (`second_plane_offset`,
   `:165`) already does this for textures; reuse it.
3. Shader side, `src/shaderc/online/vivante/` and the draw key: a new
   pass beside `wide128.rs`, the analogue of `lower_128bit_output`:
   the colour output of a 128-bit attachment becomes two outputs,
   `(r,g)` and `(b,a)`, the second at output index `4 + k`. The existing
   `wide128::Planes` key is for the sampler side; the render side needs a
   second key (`rt_is_128bit`, companion index), compiled as a variant
   by the same mechanism (`wide128-shc-entry`: free functions in
   `src/shaderc/mod.rs`, no field in `ShcBackend`), chosen at draw time
   by the GL core (`src/gl/draw.rs`, plan 313's file). `shader.rs:327-337`
   looks the output row up by `4 + shift + slot` for `PS_OUTPUT_REG`;
   Mesa's equivalent is `ps_output_remap` (`etnaviv_shader.c:205-221`).
4. Colour mask: plane 1's mask is `(mask >> 2) & 3`
   (`etnaviv_blend.c:153-163`); our MRT code programs mask and blend per
   target from the draw (`backend.rs`, `mrt_targets` comment).
5. Clear and read-back: two plane clears with a flush between
   (`etnaviv_rs.c:399-419`); our `glClear` of an integer target
   (`backend.rs`) and `glReadPixels` through `retire_wide_targets` need
   the 16-byte join.
6. GL core: `refuse_undrawable_targets` in `src/gl/draw.rs` and
   `rt_format_drawable` in `src/backend.rs` hold the name list; both are
   other plans' files (313, 308).
7. Tests: `tests/es3_int_rt_gc.rs` (the refusal tests become draw tests;
   the PE_COLOR_FORMAT/PE_RT_CONFIG words, `PS_OUTPUT_REG`, the
   plane address offset), `docs/es3/int-rt-gc.md`.

**Open questions that need the board** (Mesa's source answers none):
whether `G32R32F` with `UIF32` keeps all 32 bits of a negative or large
integer on this GC7000UL in an MRT (Mesa relies on it; the 64-bit
`RG32I/UI` target of `int-rt-64bpp` is the single-target evidence); whether
the 8-RT bank's second target honours the same 4x4 tiling that slot 0 does
under `SINGLE_BUFFER(2)` (our `wide_target.rs` header: "the 8-RT bank's
RT_CONFIG has no tiled bit of its own"); and whether the PE misplaces
64-bit pixels in a tiled MRT as it does in a LINEAR one (gl_test case 44).
Mesa 26.2.1's 128-bit path is recent (copyright Igalia 2025,
`etnaviv_nir_lower_128bit.c:2`), and this note has no board record from an
i.MX8MP that it works there.

**Suggested first step.** A board probe before any code: one RGBA32UI
attachment drawn as two `G32R32F` tiled targets with `UIF32`, a constant
colour with values above 2^24 and negative values, read back untiled
(item 2's shadow with two planes), compared bit for bit. If it matches,
items 1 to 3 above are a sequence of three small items; if it does not,
the refusal stays and the cause (UIF32 on an MRT plane, the second
target's tiling) is the finding.

## Implementation sequence

Can a two-plane or shadow path like RGBA16I/UI and RG32I/UI's 64-bit
tiled shadow (`0087345f`) take RGBA32I/UI? On paper yes, and it is Mesa's
own: each plane is a 64-bit `G32R32F` target with `UIF32`, the very
format RG32I/UI already draws, and the shadow's 4x4 tiling serves both
planes if the shadow holds two of them half and half. No hardware word is
new. On the host nothing blocks the pure part (the split and join, below,
are done and tested); what blocks the draw is that it cannot be judged
without the board (does `UIF32` keep 32 bits on a plane of an MRT, is the
8-RT bank's second slot tiled under `SINGLE_BUFFER(2)`: the open questions
above), and that every file the draw needs belongs to another plan:
`backend.rs` (#328, #333), the shader key in `src/gl/draw.rs`,
`src/shaderc/online/vivante/` (#328). So the refusal by name stays until
the probe below says yes.

Done (inert, `src/vivante/wide_target.rs`): `split_planes` and
`join_planes`, a linear image of 16-byte pixels into two 4x4-tiled 8-byte
planes (R,G then B,A) and back, with `plane_offset` / `planes_bytes`. The
plane offset is `tiled_copy::second_plane_offset`, the texture side's rule,
called and not copied (`tile_planes` there is private, so the per-pixel
copy is `wide_target`'s own, over its own `tiled_offset`). Mesa's
`etna_texture_tile_rgba32f` / untile (`etnaviv_tiling.c:137-158`). Nothing
in the draw path calls them.

Step 2 is done too (inert, `src/vivante/wide_target.rs`, described below).
The rest, in order. Step 0 and 7 need the board; 1 and 3 to 6 are
host-testable.

0. **Board probe** (below). If the bits do not match, stop: the finding is
   the cause, the refusal stays.
1. `src/vivante/backend.rs` (#328, #333): `GC_INT_RT_128BPP` out of the
   refusal (`pe_color_format`, `GC_INTEGER_RT_FORMATS`, the `draw_one`
   check); `mrt_targets` / `hw_targets` clamp from 4 to the 8-RT bank (up
   to four attachments give up to eight targets, a 128-bit one takes two).
   Plane address = shadow gpu + `plane_offset`, in `pe_rt_addr_8_pipe` for
   both pipes. What it calls of step 2, in place of today's one-target
   `wanted` / `prepare` / `bind` (which stay, unchanged, for the live
   RGBA16F, RGBA16I/UI and RG32I/UI draw):
   - a `wide_target::Target` per GL attachment of the draw (GPU address,
     CPU view, size, pitch in bytes, bytes a pixel). `target_of(gl)` builds
     attachment 0's from `target_*`; the others need a CPU view and a pitch
     that `GlesDrawTarget` does not carry today (`gpu`, `w`, `h`, `format`
     only), so step 1 also needs those from the GL core (`src/backend.rs`,
     `src/gl/`), or the back end's own map of target storage.
   - `wanted_targets(&targets, straight)`: every target has a CPU view and
     2, 4, 8 or 16 bytes a pixel, at least one is 64 or 128 bits, at most
     `MAX_TARGETS` (4). For one 64-bit target it answers as `wanted`.
   - `prepare_targets(&targets, finish)` before the draw records anything
     (non-zero: refuse the draw), then `bind_targets(&targets, kick)` while
     recording: one `Bound { gpu, plane1, stride }` per target, in order.
     A 128-bit target's `gpu` is plane 0 (R,G) and `plane1` plane 1 (B,A),
     the two hardware slots; `plane1` is 0 for any other target. `stride`
     is PE_COLOR_STRIDE / the PE_RT_CONFIG stride (the 8-byte plane's for a
     128-bit target). `None`: a target has no shadow, nothing was marked.
   - `retire` and `released` as today; they serve every entry.
2. Done (inert), `src/vivante/wide_target.rs`: the multi-target shadow
   above. `Key` carries the pixel size; a 128-bit entry is `planes_bytes`
   of the padded size, `tile_in` calls `split_planes`, `retire` calls
   `join_planes`; other sizes tile 4x4 at their own bytes a pixel (a 32-bit
   or 16-bit target beside a 128-bit one, since SINGLE_BUFFER tiles every
   target of the draw). `prepare_targets` never evicts a shadow of the same
   draw for another (it lets the kicks complete instead). Unit tests in
   the module pin the one-target path's entries and bytes, round-trip a
   128-bit target bit for bit (negative words, words above 2^24), and check
   a mixed 128-bit + 32-bit draw's sizes, plane offsets and strides. No
   draw reaches the new functions: `backend.rs` refuses
   `GC_INT_RT_128BPP` by name and calls only the one-target ones.
3. Shader output split, `src/shaderc/online/vivante/` beside `wide128.rs`
   (#328): the analogue of `lower_128bit_output`, `(r,g)` to the output and
   `(b,a)` to output `4 + k`; and its draw key (`rt_is_128bit`, companion
   index) in `src/gl/draw.rs` (GL core), compiled as a variant like the
   sampler side's `wide128::Planes`. `shader.rs` `PS_OUTPUT_REG` follows
   the remap.
4. Colour mask: plane 1 takes `(mask >> 2) & 3`
   (`etnaviv_blend.c:153-163`), in the per-target mask words of
   `backend.rs`.
5. Clear and read-back: `glClear` as two plane clears with a flush and a
   PE-to-RA stall between (`etnaviv_rs.c:399-419`), or by a draw;
   `glReadPixels` joins through `retire` (step 2).
6. GL-core name lists: `refuse_undrawable_targets` in `src/gl/draw.rs` and
   `rt_format_drawable` in `src/backend.rs`.
7. Tests and docs: `tests/es3_int_rt_gc.rs` (the refusal tests become draw
   tests: PE_COLOR_FORMAT / PE_RT_CONFIG words, `PS_OUTPUT_REG`, plane
   address), `docs/es3/int-rt-gc.md` section 2 (gc-docs), then a board run
   of dEQP `fbo.completeness` and `renderable.texture.*32*`.

### The board probe: a gl_test case

One RGBA32UI attachment as two `G32R32F` tiled targets with `UIF32`, a
constant colour with values above 2^24 and negative ones, read back
untiled, compared bit for bit. Hand-written in
`driver/samples/zephyr/gl_test/src` (#328's directory for now), built for
the i.MX8MP / X2, not run here. Raw command stream, so it does not depend
on steps 1 to 6:

- One page-aligned allocation of 1024 bytes, zero-filled and flushed: two
  planes of 16 x 4 pixels of 8 bytes (`tw` 16, stride 128), plane 0 at +0,
  plane 1 at +512 (`wide_target::plane_offset(16, 4)`).
- Slot 0 in `PE_COLOR_FORMAT` = `G32R32F`, address +0. Slot 1 in the 8-RT
  bank (`PE_RT_CONFIG`, `PE_RT_ADDR`, both pipes) = `G32R32F`, address
  +512. `RT_COUNT` 2. `PS_CONTROL_EXT` output mode `UIF32` for both.
  `PE_LOGIC_OP.SINGLE_BUFFER` = 2. No tile status, blend or saturation;
  colour mask all on.
- A fragment shader writing raw bits: output 0 = `(R, G)`, output 1 =
  `(B, A)` (`PS_OUTPUT_REG` slot 1 reads output 1), no int-to-float
  conversion. One 16 x 4 rectangle, viewport 16 x 4.
- Colour (R,G,B,A) = `0x80000000`, `0xFFFFFFFF`, `0x01000001` (2^24 + 1),
  `0xDEADBEEF`. Repeat with `0x7FFFFFFF`, `0`, `0xFFFFFF01`, `0x12345678`.
- A second draw with colour mask `R|B` only (plane 0 `0b01`, plane 1 `0b01`
  as step 4 will program it) over the first result with a different
  constant: G and A (word 1 of each plane) must keep the first draw's bits.

Read back: cache invalidate after the PE-to-RA stall, then untile both
planes on the CPU with `join_planes`' rule (`tile = (y/4)*(tw/4) + x/4`,
texel at `(tile*16 + (y%4)*4 + x%4) * 8`).

Expected output (all 64 pixels equal):

```
gc rt128 probe: 64/64 pixels bit-exact
  plane0 px(0,0)  = 80000000 ffffffff  want 80000000 ffffffff
  plane1 px(0,0)  = 01000001 deadbeef  want 01000001 deadbeef
  plane0 px(15,3) = 80000000 ffffffff  want 80000000 ffffffff
  plane1 px(15,3) = 01000001 deadbeef  want 01000001 deadbeef
```

Anything else is the finding: which word changed, at which pixel. Pixels
moved in 16-byte pairs is gl_test 44's PE fault again (tiling not honoured
by the slot); `01000001` read as `01000000` is a 24-bit path (UIF32 not
raw on an MRT plane).
