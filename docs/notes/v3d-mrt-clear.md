<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: a scissored or masked colour clear of several colour attachments

Design, written before the code item `v3d-mrt-clear`, from reading only (no
board, no run). Today `draw_call` refuses the clear by name
(`src/v3d/backend.rs:6062`, "a scissored or masked colour clear of several
colour attachments") because the clear quad's programs write RT 0 only. The
single-attachment integer quad is [v3d-int-clear.md](v3d-int-clear.md); read
its "which clears take the quad" table first, this note does not repeat it.
Attachments of different sizes are [v3d-mrt-sizes.md](v3d-mrt-sizes.md) (a
refusal that stays), 4x of several attachments is
[v3d-msaa-es3.md](v3d-msaa-es3.md) (stays refused, `backend.rs:6048-6050`).

Line numbers are main at `9af85cd5` and drift; the names are the address.
Mesa is 26.2.1. `third_party/mesa-26.2.1` holds only the v3d format table;
the other Mesa files cited were read in the full 26.2.1 tree (the deps
cache's `mesa-pan`, same tag). Paths below are relative to Mesa's `src/`.

## 1. What the GL core hands the back end

A glClear or glClearBuffer* becomes one `GlesDraw` with `index_count` 0, or is
fused into the next draw. The fields, `src/backend.rs` (`GlesDraw`):

| field | line | meaning for a clear |
|---|---|---|
| `do_clear` | 1032 | the colour clear is pending |
| `clear_rgba` | 1052 | four values; for an integer target the raw 32-bit words as `f32` bit patterns |
| `clear_quad` | 1069 | the clear is scissored (or stencil-masked): a quad, not the background object |
| `clear_target_mask` | 1098 | bit n = `targets[n]` takes the colour clear; 0 = every target (glClear) |
| `targets[]`, `num_targets` | 1136-1137 | colour attachments; `[0]` mirrors `target_*`; `targets[0].out_shift` (811-825) is the dropped leading draw buffers |
| `scissor`, `scissor_rect` | 1174-1175 | the clear's own box, set only when `clear_quad` |
| `color_mask` | 1172 | the *draw's* mask, not the clear's |
| `clear_color_mask` | 1199 | the clear's glColorMask (bit 0 R .. 3 A), never 0 with `do_clear` |
| `target_format`, `targets[n].format` | 1030, 811 | per attachment surface format, so the class (float/integer) is per attachment |

Where they are set, `src/gl/draw.rs`:

- `plan_clear` (2267) decides quad or not: a box that does not hold the target
  sets `quad` and `scissor_rect`; `clear_color_mask` comes from
  `clear_color_mask` (2252), the GL state at the glClear (ES 3.0 has one mask
  for every draw buffer, no glColorMaski).
- `apply_clear_plan` (2312) writes `do_clear`, `clear_color_mask`,
  `clear_target_mask` (through `dense_target_mask`, 2198, which shifts the
  bits down by `out_shift`, so bit n is RT n of the job) and `clear_rgba`.
- `clear_target_mask` (2572) and `with_clear_target`: glClearBuffer{f,i,ui}v
  and glClearBufferfi name one draw buffer, so the mask has one bit, and
  `clear_rgba` holds that buffer's class of value (`src/gl/es3.rs:1385`). A
  glClear leaves the mask 0 (all draw buffers, or the non-NONE ones).

Consequences the design uses: (a) **one `GlesDraw` carries one colour value**
and one colour mask for every attachment it clears. (b) A clear that reaches
several attachments with *different* classes is only a glClear, whose value
for an integer buffer is undefined by ES 3.0 4.2.3, so any bit pattern is
right there; glClearBuffer, whose value is defined, is always one attachment.
(c) Only `clear_target_mask` says which RTs the clear writes.

## 2. The V3D's per-RT state

**Render-target config.** Per RT the RCL carries a TLB render-target config
with the internal type and bpp and the clear colour
(`gallium/drivers/v3d/v3dx_rcl.c: v3dX(emit_rcl)`, the loop at 795-915, with
`v3d_setup_render_target` 456/488 and `v3dX(clamp_for_format_and_type)` 397).
Type and bpp come from the format table
(`v3dx_format_table.c` via `v3d_format_get_internal_type_and_bpp`); yakogl's
copy is `state::rt_format` (`src/v3d/state.rs:445`), kept per RT by
`Target::formats()` (`backend.rs:791`). The job's tile size depends on the
*largest* bpp of all RTs (`maximum_bpp_of_all_render_targets`,
`v3dx_rcl.c:768`; `v3d_context.c:267-300`; yakogl `Target::max_bpp`,
`backend.rs:802`). The clear colour is per RT, in CLEAR_COLORS
(`v3dx_rcl.c:836-890`) - this is what the TLB clear of a whole target fills
(`backend.rs:6154-6178`, `clear_words` per RT, honouring `clear_target_mask`).
The tile buffer therefore holds each RT in *its own* type, whatever the others
are: mixed integer and float RTs in one job are fine (the int-RT draws
already do this).

**COLOR_WRITE_MASKS.** One packet, four bits per RT, a set bit *disables* the
channel; Mesa builds it per RT from `blend->rt[rt].colormask` (independent
blend) or RT 0's (`gallium/drivers/v3d/v3dx_emit.c:571-585`,
`translate_colormask` 83-91, which also swaps R and B for `swap_color_rb`).
yakogl emits **one mask for all RTs**: `state::job_state` fills one
`colormask` (`state.rs:662`, `color_write ? color_mask & 0xf : 0`) and
`driver/src/v3d/v3d_job.c:698-704` replicates `~mask & 0xf` into the four
nibbles (`V3dJobState.colormask`, `ffi.rs:366`). That matches the clear: the
clear has one mask for every attachment (section 1). No per-RT mask is needed
unless an RT the clear does *not* name would be written by the quad (below).

**How a quad writes an RT.** The compiler writes the TLB once per RT that the
fragment shader outputs and the key's `cbufs` includes, each with a config word
carrying the RT number (`(7 - rt) << TLB_RENDER_TARGET_SHIFT`) and the type:
16-bit pairs (`TLB_TYPE_F16_COLOR`) for a float output, 32-bit
(`TLB_TYPE_F32_COLOR`, vec size 1-4) for `f32_color_rb` and for integer
outputs (`broadcom/compiler/nir_to_vir.c: vir_emit_tlb_color_write`
1952-1990, `emit_frag_end` 2038-2141). The tile buffer narrows the written
words to the RT's internal type; the TLB type carries no signedness
(this is why `CLEAR_FS_INT` serves every integer class,
`backend.rs:816-833`). An RT the shader does not output is not written at all
(`vir_emit_tlb_color_write` returns for it); it keeps the TLB's loaded or
cleared value. The fragment shader's `rt_mask` (`fs.v3d.fs.rt_mask`, checked
at `backend.rs:6090`) is yakogl's record of which RTs a program writes; a
program that writes only location n sets bit n.

**What Mesa does for the clear.** Mesa's v3d advertises neither
`clear_scissored` nor `clear_masked`, so the state tracker routes every
scissored or masked colour clear to `clear_with_quad`
(`mesa/state_tracker/st_cb_clear.c: st_Clear` 405-470,
`clear_with_quad` 194; per-buffer routing 424-446). The quad: blend state with
`independent_blend_enable = num_buffers > 1` and **`blend.rt[i].colormask`
only for the draw buffers named in the clear** (`st_cb_clear.c:223-244`; an RT
not named has colormask 0, i.e. not written); one shader, `st_nir_make_clearcolor_shader`
(`mesa/state_tracker/st_nir_builtins.c:135-157`), that stores one vec4 uniform
to `FRAG_RESULT_COLOR`, which `nir_lower_fragcolor` fans out to every
colour buffer (`gallium/drivers/v3d/v3d_program.c:364`). So Mesa uses **one
quad for all named attachments**, selecting RTs with the per-RT colour mask
(hence the independent-blend mask packet), and does not handle mixed
integer/float in that one quad: integer buffers use `glClearBuffer*` and
`st_ClearBuffer`'s own single-buffer path. A clear that a draw already used
(`job->draw_calls_queued`) cannot be a TLB clear and falls to
`v3d_draw_clear` (`gallium/drivers/v3d/v3dx_draw.c: v3d_tlb_clear` 1661,
`v3d_draw_clear` 1634, `v3d_clear` 1801), the blitter's quad
(`gallium/auxiliary/util/u_blitter.c: util_blitter_clear`). `v3d_blit.c`
(`v3d_tlb_blit`) is not part of the clear path; it needs no change.

## 3. Recommended design

Keep the split of v3d-int-clear.md. Change only what the quad writes:

1. **Which RTs.** `rt_set = clear_target_mask` when non-zero, else all
   `target.rts.max(1)` RTs (the same rule as the TLB loop at `backend.rs:6159`).
   An RT whose `targets[n].gpu == 0` (a GL_NONE draw buffer, `extra_targets`
   5971) is configured and never stored: write it harmlessly or skip it.
2. **One quad per written attachment**, each with a program that writes
   *only* location n, in the class of that attachment:
   `layout(location = n) out vec4 o;` (float) and
   `layout(location = n) out uvec4 o;` (`CLEAR_FS_INT`'s shape). Both are
   cheap GLSL ES 3.00 variants of `CLEAR_FS` / `CLEAR_FS_INT`
   (`backend.rs:816-833`), built lazily and kept like `clear_program` /
   `clear_program_int` (`backend.rs:3853-3869`): at most 4 locations x 2
   classes = 8, and the existing two are location 0. `clear_quad_draw`
   (`state.rs:154`) already takes the program, so it needs a location
   argument only for `q.fs` selection; the uniform (`u_color`, four dwords)
   is unchanged. Because the program writes only RT n, every other RT is
   untouched by construction, scissor and mask included, and the single
   job-wide `colormask` (section 2) is correct for each quad. Depth and
   stencil, if the same clear has them, go in the first quad only
   (`QuadBuffers`), the others carry `colour` alone.
3. **The refusal goes**: `backend.rs:6062` is removed and `draw_clear_quad` loops over the written RTs
   (class from `target.formats()[n]`, `gles_surf_is_integer`). The integer
   program for RT n is chosen on the format of RT n, so a mixed glClear (float RT 0,
   integer RT 1) is two quads with the same four words: the float
   program gets the value as written, the integer one gets its bits (undefined
   by spec, so no conversion is wanted). The `rt_mask >> slots` check
   (`backend.rs:6090`) is bypassed for these quads (they use the quad's own
   program; `draw_in_job` takes it from `p.fs`), but each quad's RT n must be
   below `target.rts`.
4. **The unscissored, unmasked clear is unchanged**: `colour_needs_quad` false
   and no `clear_quad` is the TLB clear (`backend.rs:6122, 6154-6178`), one
   CLEAR_COLORS per RT, no quad, no draw. It already handles
   `clear_target_mask` and mixed formats through `clear_words`.

**Cost.** Jobs: 0 extra. The quads join the job the clear runs in (a quad is
a draw, `draw_in_job`), so the clear costs the attachments' count in quads, not
jobs; the TLB-clear job rule (flush if `j.draws > 0`) does not apply to a
quad. Quads: k for k written attachments (1 for glClearBuffer, up to 3 for
glClear: `GLES_MAX_TARGETS`). A quad costs a draw's fixed cost,
[v3d-draw-fixed-cost.md](v3d-draw-fixed-cost.md). A later optimisation
(do not do it first): glClear over several *float* RTs with the same mask is
Mesa's one quad - a program writing locations 0..k-1 of one class, one
variant per k; it saves k-1 draws and is a pure performance change.

**Alternative rejected: one mixed-class quad with a per-RT mask**
(Mesa's shape). It needs one `COLOR_WRITE_MASKS` nibble per RT (an extension
of `V3dJobState`, `ffi.rs:366` and `v3d_job.c:698`, a C-driver ABI change)
plus programs for every class/RT pattern. More surface, same pixels.

**Open points for the code item.** (a) That an RT the quad's shader does not
output keeps its value in the tile buffer (read from `vir_emit_tlb_color_write`;
Mesa's own draws to fewer outputs than cbufs rely on it, but no yakogl test
records it): pin it with the test in section 4 and put the board check in
the issue. (b) A float RT of a 32-bit format (R32F/RGBA32F) wants the `f32`
TLB config; the float `CLEAR_FS` is `mediump` (f16 pairs). Check
`select.rs:1818` / `rt_format().clamp` for how a draw to such an RT is
compiled and give the float variant the same treatment if it differs. (c)
The sRGB RT (no `rt_format` row, `extra_targets` refuses it) is not in this
change.

## 4. Host tests (`src/v3d/backend_tests.rs`)

Use `mrt_draw` (3657) and `program_writing`, the `Rig`, `clear_only`,
`assert_is_the_quad` and `writes_32bit_words` helpers already there
(2282-2600, 3640-3830); set `targets[n].format` with `GLES_SURF_*`.

1. **Mixed integer/float scissored clear.** 3 targets: RGBA8, RGBA8I, RGBA8
   (or R32F); `do_clear`, `clear_quad`, `scissor`/`scissor_rect`, no
   `clear_target_mask`. Assert: draw returns 0 (the refusal log
   "...several colour attachments" count is 0); exactly 3 recorded quads, in
   one job (`rts == 3`); each quad's `fs_code` is the float program for RT 0
   and 2 and the integer one for RT 1 (`writes_32bit_words`); each carries
   the scissor rectangle (`st.scissor*`, as the test at 2560); each quad's
   program writes only its own location (`fs.v3d.fs.rt_mask == 1 << n`);
   no TLB clear colour is armed for the quad's RTs (`clear.load_rt*` true).
2. **glClearBuffer of one attachment of several.** `clear_target_mask = 0b010`,
   scissored, integer RT 1: one quad, rt_mask `0b010`, the integer program,
   the four raw words in the uniform; RT 0 and RT 2 untouched. Add the same
   with RT 2 float and RT 0 integer, so the program choice is per RT.
3. **Per-attachment masked clear.** `clear_color_mask = 0b0101`, no scissor
   (a background clear that the mask turns into quads, `colour_needs_quad`),
   three targets: every quad's `st.colormask == 0b0101` (one job-wide mask,
   section 2), `st.scissor` false; the same with `clear_quad` and the mask
   (both), and with the clear fused into a draw (`index_count != 0`): the
   quads first, then the draw with its own `color_mask`, one job.
4. **Unscissored fast path.** `do_clear`, mask `0xf`, no `clear_quad`, three
   targets of mixed class: no quad (`rig.draws()` empty), no refusal, the
   job's `clear.clear_rt[n]` holds `clear_words(format_n, clear_rgba)` for
   every RT and `load_rt*` is false; with `clear_target_mask = 0b100` only
   RT 2's colour is armed and RT 0/1 are loaded. (Most of this exists
   already; the test pins that the new loop did not touch it.)
5. **Still refused.** 4x of several attachments, different sizes, an sRGB
   attachment: unchanged names. Replace the third refusal of
   `attachments_the_job_builder_cannot_take_are_refused_by_name` (3812),
   which asserts this refusal today, with test 1's expectation.
6. **Depth and stencil with a several-attachment colour clear**: the depth/stencil
   bits are in the first quad only (`QuadBuffers`), the later quads colour only.

A pixel-level check (that an unnamed RT keeps its value) is a board item; the
host tests above pin the recorded state. Gate with
`tools/check.sh --changed origin/main`; the change touches `src/v3d/**` and
this note.
