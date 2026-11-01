<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: the integer clear-quad program for a scissored or masked clear

Design, written before the code item `v3d-int-clear`. Today
`draw_call` refuses a scissored or masked colour clear of an integer colour
target by name (`src/v3d/backend.rs:5989-5991`, "a scissored or masked colour
clear of an integer colour target"), because the clear quad's program
(`CLEAR_FS`, `src/v3d/backend.rs:802-803`) writes `gl_FragColor` as f16 pairs,
which an integer render target cannot take. This note says which clears take
the quad, what the quad's program and uniforms are, how the mask and the
scissor reach the hardware, what the TLB writes for each integer class, and
how to test it. The change itself is one program, one selection and one
removed refusal; no new hardware word is needed.

Line numbers: yakogl is main at `ff7b9882` and drifts, the function names are
the address. Mesa is 26.2.1, paths relative to the Mesa tree. The vendored
`third_party/mesa-26.2.1` holds only the format table of the v3d driver
(`src/gallium/drivers/v3d/v3dx_format_table.c`); every other Mesa file below
was read in the full 26.2.1 tarball tree (the deps cache's `mesa-pan`
checkout, the same tag). Mesa is described in words here, not copied.

## (a) Which clears take the quad, which keep the TLB clear

The split is decided in two places and stays as it is; only the refusal
between them goes.

- GL core (`src/gl/draw.rs: plan_clear`): a clear whose scissor box does not
  hold the whole target, or whose stencil write mask is partial, is marked
  `GlesDraw::clear_quad`; a colour mask that is neither all on nor all off
  travels as `clear_color_mask` (`apply_clear_plan`). A colour mask of 0
  clears no colour.
- V3D back end (`src/v3d/state.rs: colour_needs_quad`, line 89): the colour
  clear is the quad's when `clear_quad` is set or the mask is not `0xf`;
  otherwise it is the TLB's clear (`tlb_colour` in `draw_call`, which writes
  `clear_words(format, clear_rgba)` into the job's CLEAR_COLORS).

Mesa's rule is the same in shape. The state tracker sends a colour buffer to
the driver's `clear` hook only when the screen can do the clear itself: a
scissored one needs `caps.clear_scissored`, a masked one `caps.clear_masked`
(`src/mesa/state_tracker/st_cb_clear.c:439-446`, `can_scissor_clear` set from
the cap at `st_manager.c:1072`). The v3d screen sets neither cap (no hit for
either in `src/gallium/drivers/v3d/`), so every scissored or masked colour
clear is drawn by `clear_with_quad` (`st_cb_clear.c:194`, called from the end
of `st_Clear`, :396). The TLB clear is Mesa's `v3d_tlb_clear`
(`src/gallium/drivers/v3d/v3dx_draw.c:1661`), taken by `v3d_clear` (:1801) for
what remains; it covers whole tiles and every channel (it sets
`scissor.disabled`, :1793). `v3d_clear` falls back to the blitter's draw
(`v3d_draw_clear`, :1634; `util_blitter_clear`,
`src/gallium/auxiliary/util/u_blitter.c:1630`) only for buffers a queued draw
already used (:1668-1673), which yakogl already handles by giving the TLB
clear a job of its own (`draw_call`: flush when `j.draws > 0`).

Cases, for an integer RT 0 (`gles_surf_is_integer`, `GLES_SURF_R8I` ..
`GLES_SURF_RGB10_A2UI`, `src/backend.rs:345-346`):

| clear | today | after |
|---|---|---|
| whole target, mask 0xf, no scissor (`glClearBuffer{i,ui}v`, the CTS's `fbo.render` clears) | TLB | TLB, unchanged |
| scissor box inside the target | refused | quad |
| scissor box holding the whole target | TLB (the GL core drops it) | TLB, unchanged |
| any colour mask other than 0xf and 0 | refused | quad (see the refinement below) |
| scissor and mask | refused | quad |
| clear fused into a draw (`do_clear` with `index_count != 0`) | refused if either | the quad first, then the draw (`draw_call` already orders them) |
| depth or stencil clear beside a colour clear | refused with it | one quad, `QuadBuffers {colour, depth, stencil}` as for RGBA8 |
| several attachments (`rts > 1`) | refused (`backend.rs:5995`) | still refused, same line: the quad writes RT 0 only |
| 4x (`msaa && typed`) | refused (`backend.rs:5982`) | still refused |

One refinement is open, and the code item decides it: Mesa compares the mask
with the channels the format *has* (`surf_colormask`,
`st_cb_clear.c:436-442`), so `glColorMask(true,false,false,false)` on `R32I`
is a whole-channel clear and stays the TLB's. `colour_needs_quad` compares
with `0xf`, and `tests/v3d_clear.rs: colour_needs_quad_is_the_tlb_decision`
pins that for RGBA8. The proposal is to leave `colour_needs_quad` alone and
add the format-aware test for integer targets only (`channels` from
`GlesIntLayout`, `src/backend.rs:373`): it keeps RGBA8 byte for byte, and an
`R32I` target with a mask of R alone then costs no quad. It is an
optimisation, not a need: with it absent the quad still gives the right
pixels (the masked-off channels do not exist). Pin whichever is chosen.

A caveat that is not new: the quad draws RT 0 and ignores
`clear_target_mask`. With `rts == 1` the mask is 0 or bit 0 for a clear that
reaches the quad, except the "all draw buffers GL_NONE" value 0x80
(`clear_target_mask`, `src/gl/draw.rs:2572-2587`), for which `do_clear` is
already false (`cp.color` and the mask). The item should assert that, not
rely on it.

## (b) Where the clear value comes from, and its bit layout

The value is `GlesDraw::clear_rgba`, four `f32` that hold the raw integers
of `glClearBuffer{i,ui}v` as bit patterns (`clear_color_integer`,
`src/gl/es3.rs:1355-1368`: `bits.map(f32::from_bits)`). `draw_clear_quad`
(`src/v3d/backend.rs:5481`) already hands that array to
`state::clear_quad_draw` as the fragment uniform block, four dwords
(`state.rs:175-176`). That is all the data path the integer quad needs: the
block is read as `u32`, never as a float. Care: `f32::from_bits` of a
signalling-NaN pattern (an `ivec4` such as 0x7f800001) is preserved by a
plain move on both host and Cortex-A72, but any arithmetic on `clear_rgba`
would not preserve it; `apply_clear_plan` does arithmetic only for non-integer
formats (`draw.rs:2331`, guarded by `!gles_surf_is_integer`). The quad path
must copy the four words (`to_bits`) and do nothing else with them. A test
should use 0x7f800001 and 0xffffffff.

Mesa does the same: the clear colour is uploaded as a four-float constant
buffer without conversion (`set_clearcolor_fs`, `st_cb_clear.c:110-124`; the
blitter's clear does the same with `color->f`, `u_blitter.c:1592` in
`util_blitter_clear_custom`, :1566) and the clear shader loads that vec4 as
`float32` and stores it to the colour output (`st_nir_make_clearcolor_shader`,
`src/mesa/state_tracker/st_nir_builtins.c:138-156`). The words reach the TLB
unchanged because the v3d key writes integer formats as 32-bit words
(section d). Mesa sends all four words to every format, whatever its width.

Layout in the uniform block, by class. The block is always four dwords, word
n = clear value of channel n as 32 raw bits; the program writes them all and
the tile buffer narrows (section d). What a class *keeps* of each word:

| class (internal type, `state::rt_format`) | formats | kept from each word | tile-buffer clear word, for comparison (`clear_words`) |
|---|---|---|---|
| 8I / 8UI | R8I, R8UI, RG8I, RG8UI, RGBA8I, RGBA8UI | low 8 bits of the first 1, 2 or 4 words | one word, byte n = low byte of word n |
| 16I / 16UI | R16\*, RG16\*, RGBA16\* | low 16 bits of the first 1, 2 or 4 words | two words, two 16-bit halves each |
| 32I / 32UI | R32\*, RG32\*, RGBA32\* | the whole word, 1, 2 or 4 of them | the words as they are |
| 16UI packed (RGB10_A2UI) | RGB10_A2UI | low 16 bits of each of four words in the tile buffer; the store packs 10/10/10/2 | as 16UI, two words |

An R-only format has green, blue and alpha words in the block that nothing
reads; an RG one has blue and alpha. The block is the same four words for I
and UI: signedness is the program's declaration only, and the 32-bit TLB
write does not carry it (section d).

One program serves every integer class. The proposal is a second built-in
fragment shader beside `CLEAR_FS`, in ES 3.00 (the online compiler already
compiles it, `tests/es3_int_rt_v3d.rs: run_fs`), with the same vertex shader
and the same one uniform at dwords 0..3:

```glsl
#version 300 es
precision highp int;
uniform uvec4 u_color;
out uvec4 o;
void main() { o = u_color; }
```

`build_clear_program` (`backend.rs:3856`) checks the uniform's name and slot
(`u_color`, `dword_off == 0`, `dwords == 4`, :3894-3899) and the single
attribute; the integer program passes the same checks, so
`ClearProgram`/`ClearProg` gain a second instance (`clear_program_int`) and
`draw_clear_quad` picks it by `gles_surf_is_integer(target.format)`. A
failure to build it leaves the refusal in place for integer targets with the
message of `clear_program` (`backend.rs:3817-3821`), and RGBA8 is untouched.
The compiler writes an `out uvec4` as four words, `MOV` to TLBU with the
32-bit config for the first and to TLB for the rest
(`src/shaderc/online/v3d/select.rs:2262-2268`, `tlb_config_32b` :1784); a
vec4 declaration is Mesa's component count too (`nir_to_vir.c:1960-1976`
takes the count from the output variable). Using `uvec4` for the signed
classes is deliberate: the words go to the TLB as they are and a signed
declaration would make no difference to the code; if the item prefers an
`ivec4` variant for symmetry, it costs a second program and adds no
behaviour.

## (c) The write mask and the scissor

Both are state of the quad's draw, as for RGBA8, and need no new code in the
job builder.

- **Write mask.** `clear_quad_draw` sets `color_write = b.colour` and
  `color_mask = clear_color_mask & 0xf` (`state.rs:181-182`); `job_state`
  copies it to `V3dJobState::colormask` (`state.rs:648`), and the job builder
  emits COLOR_WRITE_MASKS with the mask complemented, four bits a render
  target (`driver/src/v3d/v3d_job.c:689-694`). Mesa makes the same word:
  `translate_colormask` returns `~colormask & 0xf` (the red/blue swap only for
  a swapped RT, which an integer format never is,
  `src/gallium/drivers/v3d/v3dx_emit.c:83-92`), emitted per render target at
  :572-583; the clear's blend state gets the GL mask per draw buffer
  (`st_cb_clear.c:239`). The mask is bit 0 R .. bit 3 A. The `target_alpha_bits
  == 0` trim of `job_state` (`state.rs:655-657`) does not apply to integer
  targets (their `target_alpha_bits` is the format's own; check that it is not
  zero for `R32I`, whose alpha does not exist: the trim would then clear the
  A bit of a mask that never mattered, which is harmless).
  Blending is off in the quad (`clear_quad_draw` starts from `GlesDraw::ZERO`),
  as ES 3.0 4.1.8 requires of integer buffers anyway.
- **Scissor.** `clear_quad_draw(.., scissored = d.clear_quad, ..)` copies
  the clear's box when it is a `clear_quad` (`state.rs:203-205`);
  `job_state` turns it into the V3D's clip window with the y flip for a
  window (`state::scissor`, :261-290, `v3d_frame.c`/`v3d_job.c` CLIP_WINDOW).
  Mesa's v3d does it the same way, by narrowing the clip window to the
  scissor (`v3dx_emit.c:238-262`) and, in the job, recording scissor rects to
  cull tiles (:273-282). The quad covers the whole target; the clip window
  trims it. An empty box is all zero, the clip window is empty and nothing is
  written (a box of zero width or height is already dropped in the GL core,
  `plan_clear`: `box_[2] <= 0 || box_[3] <= 0`). Mesa's quad covers the
  scissor rectangle instead (`clear_with_quad`, `st_cb_clear.c:204-207`, x/y
  from `_Xmin.._Ymax`); the result is the same pixels.
- **What does not apply.** Depth test off (ALWAYS, no write unless the quad
  clears depth), stencil off unless the quad clears it, no cull, viewport the
  whole target (`state.rs:177-192`); alpha-to-coverage and the sample mask
  are zero in `GlesDraw::ZERO`.

## (d) What the TLB write looks like for each integer class

On V3D 4.2 there is one 32-bit TLB colour type for float and integer words
alike, and the render target's internal type, not the write, says what the
words are.

1. **The key.** Mesa marks a colour buffer as a 32-bit-word target
   (`f32_color_rb`) for a 32-bit float format, for 16-bit unorm and snorm, and
   for every pure unsigned or pure signed integer format
   (`src/gallium/drivers/v3d/v3d_program.c:681-704`; the pure-integer cases
   are :700-704).
2. **The write.** For such an RT the fragment program does not pack halves.
   It writes the config word to TLBU with the first channel and then each
   remaining channel to TLB, one word per component of the shader's output
   (`src/broadcom/compiler/nir_to_vir.c:1974-1977` sets the 32-bit type and
   "size minus one"; :2005-2021 emits the moves). The config is
   `0xffffff00 | per-pixel sample mode | (7 - rt) << 3 | TLB_TYPE_F32_COLOR |
   (components - 1)` (:1963-1968, type constants :1929-1938). `TLB_TYPE_I32_COLOR`
   is defined (:1930) and never written. For RT 0 and four components that is
   `0xffffff3f`, pinned by `tests/es3_int_rt_v3d.rs:
   an_ivec4_output_writes_four_raw_words` against `select.rs: tlb_config_32b`
   (`select.rs:1784-1792`); this program's TLB sequence is exactly that
   test's four words.
3. **The narrowing.** The tile buffer holds the target's internal type
   (`v3dX(get_internal_type_bpp_for_output_format)`,
   `v3dx_format_table.c:220-341`; yakogl: `state::rt_format`), and the render
   target's clamp is INT for a pure integer format
   (`src/gallium/drivers/v3d/v3dx_rcl.c:397-408`, V3D 4.2). The words are
   narrowed by the hardware when they enter the tile buffer.

Per class, the TLB sees (RT 0, the quad writing four components in every
case, as Mesa's vec4 clear shader does):

| class | TLB writes | tile buffer (internal type, bpp) | `v3dx_format_table.c` row | store |
|---|---|---|---|---|
| R8I, R8UI | TLBU conf `0xffffff3f` + 3 TLB words | 8I / 8UI, 32 bpp | :118-119 | R8I / R8UI, one byte |
| RG8I, RG8UI | same | 8I / 8UI, 32 bpp | :120-121 | RG8\* |
| RGBA8I, RGBA8UI | same | 8I / 8UI, 32 bpp | :122-123 | RGBA8\* |
| R16I, R16UI, RG16I, RG16UI | same | 16I / 16UI, 32 bpp | :125-128 | 16-bit channels |
| RGBA16I, RGBA16UI | same | 16I / 16UI, 64 bpp | :129-130 | RGBA16\* |
| R32I, R32UI | same | 32I / 32UI, 32 bpp | :132-133 | R32\* |
| RG32I, RG32UI | same | 32I / 32UI, 64 bpp | :134-135 | RG32\* |
| RGBA32I, RGBA32UI | same | 32I / 32UI, 128 bpp | :136-137 | RGBA32\* |
| RGB10_A2UI | same | 16UI, 64 bpp (`src/gallium/drivers/v3d/v3dx_format_table.c:75`, :282-285) | :75 | packed 10/10/10/2 |

The row numbers are of the file as read; the table in
`tests/es3_int_rt_v3d.rs: integer_rows_follow_mesas_format_table` already
holds them keyed by name. The hardware config word is the same for every row;
the class is entirely in the render-target configuration the job builder
already emits (`state::rt_format`, `v3d_job.c: rt_internal`).

Behaviour on out-of-range values differs between the two paths and is the
one measurement the item owes. The TLB clear copies the low bits of each word
(`v3dx_draw.c:1735-1769`, masks of 0xff and 0xffff, and Mesa's own comment
that "clamping is not performed on the clear color", :1702-1712); the quad's
words pass the RT's INT clamp on entry. So `glClearBufferiv` of 300 to
`R8I` is 44 by the TLB and, if the clamp saturates, 127 by the quad. The ES
3.0 text (4.2.3) leaves integer conversion of an out-of-range clear to the
implementation, and the CTS clears with in-range values; the item should
record what the host reference and the board give and not promise either.
The same holds for `RGB10_A2UI` (values above 1023 or alpha above 3 reach the
store's packer after a 16-bit clamp). In-range values are identical on both
paths.

## (e) Test plan

Run: `cargo test --no-default-features --features
host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_clear --test
es3_int_rt_v3d` (the test command of each file's header), and the back
end's own `src/v3d/backend_tests.rs`.

**`src/v3d/backend_tests.rs`** (not a file the item may own: it is inside
`src/v3d/**`, which the code item changes): replace
`what_a_typed_job_cannot_take_is_refused_by_name`'s two integer asserts
(`backend_tests.rs:5929-5937`) by the cases below; the 4x, tile-pool and
several-attachment refusals stay.

**`tests/es3_int_rt_v3d.rs`** (the compiler's side and the per-class words):
1. The integer clear program compiles, links, and passes
   `build_clear_program`'s uniform check: `u_color` at dwords 0..3, one
   attribute, `rt_int` bit 0, `rt_comps[0] == 4`.
2. `run_fs` on the program with a uniform block of `[1, 0xfffffffe,
   0x7f800001, 0xffffffff]` gives `[(Some(0xffff_ff3f), 1), (None,
   0xffff_fffe), (None, 0x7f80_0001), (None, 0xffff_ffff)]`: every word as it
   is, signalling NaN pattern and all-ones included. (The existing `run_fs`
   builds its uniform vector from `u.contents == 0`; the program's uniform is
   a block value, so the test needs the block as its input, as the helper of
   `backend_tests.rs` does with `fs_block`.)
3. For each of the 20 integer surfaces, `clear_words(surf, bits)` equals what
   the narrowing of the four quad words would give (the table in (b)):
   one test over `integer_surfaces()` that builds the expected tile-buffer
   word from the kept bits by class. This pins the quad and the TLB clear to
   one value for in-range input; a second asserts the out-of-range
   difference is documented, not hidden (it may only be filled in after the
   host record).

**`tests/v3d_clear.rs`** (the decision and the quad's state, no hardware):
4. `colour_needs_quad_is_the_tlb_decision` gains integer targets: whole
   target and full mask is the TLB; scissored, masked, and both are the
   quad, for one 8-, 16-, 32-bit and the RGB10_A2UI surface; with the
   refinement chosen in (a), `R32I` with mask `0b0001` is the TLB.
5. The quad draw for an integer target (extending
   `quad_state_for_each_buffer_combination`): the draw is the integer
   program's, `fs_uniform_dwords == 4` and the block holds the clear's bits
   unchanged (compare `to_bits` of `clear_rgba`, with 0x7f800001 among them),
   `st.colormask` is the GL mask, `st.scissor` and the min/max are
   `state::scissor`'s, for flipped (window) and unflipped (FBO) targets.
6. The cts-style random boxes of `cts_box` (x, y in [-W, W), may be empty or
   overrun) over an integer target: the pixels the quad's clip window leaves
   in (`in_job_scissor`) are exactly the CTS rectangle (`in_box`), for
   `R32I` and `RGBA8UI`, so the quad never clears a pixel outside GL's box.
7. A masked integer clear on a multi-channel format (`RGBA8I`, mask
   `0b0101`): the channels off keep the target's previous contents and the
   channels on take the clear value; the test uses the same model the file's
   `v3d_clear` function uses for RGBA8 (`tests/v3d_clear.rs:561`) extended to
   per-channel integer words.
8. Order: a fused clear (`do_clear` plus an integer draw) records the quad
   before the draw, the quad's `colormask` is the clear's and the draw's is
   its own (the integer twin of `masked_fused_clear_keeps_its_own_masks`,
   `v3d_clear.rs:385`).
9. Refusals that stay: `rts > 1` with a quad, 4x with an integer format; and
   one that is new: if the integer program failed to build, the refusal names
   it (`clear_program` text), not the old message.

**Host records to compare** (leave `MESA_V3D_TREE` unset: the vendored
reference build, `tools/v3d/README.md`):
- `tools/v3d/drm_shim_ref.sh run <work dir> <case> <out prefix> 42` runs
  Mesa's v3d on the drm-shim with `V3D_DEBUG=clif` and writes the control
  lists Mesa submits. Its cases today are `clear` and `triangle`
  (`tools/v3d/gl_ref.py`), on an ES 2.0 RGBA8 FBO; neither is an integer
  clear. The record the item needs is a new `gl_ref.py` case,
  `clear_int_scissor`: an ES 3.0 context, an `R32UI` (then `RGBA8I`,
  `RG16UI`) FBO of 64x64, `glScissor(8, 4, 32, 20)`, then
  `glClearBufferuiv(GL_COLOR, 0, {1, 0xfffffffe, 0x7f800001, 0xffffffff})`;
  and a masked one, `glColorMask(1,0,1,0)` on `RGBA8I`. The CLIF compares
  these fields with the back end's job: the render target's internal type,
  bpp and clamp in the RCL/TILE_RENDERING_MODE_CFG (INT clamp), the
  COLOR_WRITE_MASKS word, the CLIP_WINDOW, the fragment shader's TLBU config
  `0xffffff3f` and the uniform stream (four words as written), and **no**
  CLEAR_COLORS for the cleared RT in the quad case (the quad replaces the TLB
  clear), but one in the whole-target case.
  This is `tools/v3d/**`, which the item `qpu-ref-counts` holds today; the
  case therefore is a follow-up for after it, and the item can compare the
  CLIF it gets by hand with `tools/v3d/cl_golden.c`'s decoder until then.
- `v3d_shc` (`tools/v3d/v3d_shc.c`, built by the same script) for the
  program's code size: Mesa's clear shader for the integer RT against the
  integer program's, instruction for instruction (`qpu_counts_42.tsv` holds
  the format; a one-output pass-through of a uniform is a handful of
  instructions and the compiler's choice is the item's to record).
- `gl_ref.py` cannot read pixels (the shim runs no job,
  `drm_shim_ref.sh`), so pixel truth is the board: `gl_test` 200 (integer
  render targets, `pi4-gltest`) must gain, or be followed by, a scissored and
  a masked integer clear, and the dEQP images `fbo.render.*` / `color_clear`
  with integer attachments are the board check. Nothing here is verified on
  hardware yet.

## Risks the tests do not close

- **Out-of-range clears** (section d): the two paths may differ; unmeasured.
- **RGB10_A2UI** through a 16UI tile buffer and the packing store: the same
  16-bit clamp question, plus whether the packer saturates or truncates.
- **Programs for several RTs** stay refused; a clear of draw buffer 1 alone
  with a scissor on an MRT framebuffer needs a program variant keyed by the RT
  index (config bits `(7 - rt) << 3`), which is the `docs/online-shaderc/
  v3d-backend.md` "Multiple render targets" item, not this one.
- **The first draw of a frame** pays one program upload for the second
  program (`init_clear_program` at init, `backend.rs:3838`): a few hundred
  bytes of QPU code in the carve-out, built once.
