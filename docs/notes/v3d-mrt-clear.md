<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: a scissored or masked colour clear of several colour attachments

Design, written before the code item `v3d-mrt-clear`, from reading only (no
board, no run), and, in section 0, what the item built. Before it,
`draw_call` refused the clear by name
(`src/v3d/backend.rs:6062`, "a scissored or masked colour clear of several
colour attachments") because the clear quad's programs wrote RT 0 only. The
single-attachment integer quad is [v3d-int-clear.md](v3d-int-clear.md); read
its "which clears take the quad" table first, this note does not repeat it.

## 0. What landed (`v3d-mrt-clear`)

The refusal is gone. The clear is **one quad from `clear_mrt.rs`** (the
alternative section 3 rejects, minus the per-RT mask it rejected it for),
not section 3's one quad per attachment:

- `draw_clear_quad` (`backend.rs`) classes every RT of the job
  (`mrt_pattern`): an RT `clear_target_mask` names (every one for glClear)
  and that has an address takes its format's class (`clear_class`:
  integer by signedness, R32F as `Uint`, section 5b; anything else
  `Float`); a GL_NONE draw buffer or an RT the clear does not name is
  `NotCleared` and gets no output. `clear_mrt_source` writes the program,
  `build_clear_program` checks each `u_color<n>` at its four dwords and
  that the compiled `rt_mask` is exactly the cleared set, and the fragment
  block is `clear_rgba`'s four words once per cleared RT (dwords `4k..4k+3`).
  A single-attachment job keeps `CLEAR_FS` / `CLEAR_FS_INT` byte for byte.
- One program per class pattern, built on first use and kept in
  `V3dState::clear_program_mrt` (`MRT_CLEAR_PROGRAMS` = 8 slots of 1.5 KiB;
  `Int` and `Uint` are one `uvec4` output, so one key: 3^4 patterns at
  most, too many to keep them all in static state). When the table is full
  the oldest slot is given up with no flush: the OSAL holds a freed block
  until every job that may read it has completed (`src/osal/v3d_frees.h`),
  and the job builder copied the program's record when the quad was
  recorded. The pattern costs a compile again when it returns.
- The colour mask is the job's one `COLOR_WRITE_MASKS` (section 2: ES 3.0
  has one mask for every draw buffer); no per-RT mask ABI change. The
  depth and stencil halves ride on the same quad. The unscissored, unmasked
  clear is the TLB's as before.
- Why one quad and not k: tests/es3_mrt_ds_clear_v3d.rs and
  tests/es3_fragops_scissor_v3d.rs pin "one job, one quad", and k quads cost
  k draws' fixed cost (section 3, "Cost") for the same pixels.

The host tests are `src/v3d/backend_tests.rs` (the mixed-class quad, one
RT of several, the job-wide mask, the TLB fast path, depth/stencil with it,
R32F and R16F, a GL_NONE slot, the program table and its eviction, a
program that did not build) and the un-ignored tests of
tests/es3_mrt_clear_v3d.rs, tests/es3_mrt_ds_clear_v3d.rs,
tests/es3_fragops_scissor_v3d.rs and tests/es3_four_rt_v3d.rs. The board
pixel check of section 5a (an RT the program leaves out keeps its bytes) is
still open; it is the Pi 4 rerun of FO6 and M1.
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
host tests above pin the recorded state. The gate is the
pull request's CI (`tools/check.sh`); the change touches `src/v3d/**` and
this note.

## 5. Open points, answered

Open points (a) and (b) of section 3, read in code. The vendored Mesa under
`third_party/mesa-26.2.1` holds only the packet XML
(`src/broadcom/cle/v3d_packet.xml`) and `v3dx_format_table.c`; the compiler
(`nir_to_vir.c`, `vir_emit_tlb_color_write`), `v3dx_rcl.c` and `v3dx_emit.c`
are **not** vendored. A Mesa line number below that is not in those two files
is the one yakogl's own comments cite, not a line read here. Point (c) stays
as section 3 says.

### (a) An RT the quad's program does not write keeps its tile-buffer value

**Answer: yes, by everything the code shows. No yakogl code writes or clears
an RT the program leaves out. A Pi 4 pixel check is still open.**

Read in code:

- *Mesa's side* (cited, not vendored): `vir_emit_tlb_color_write` is called
  per RT from `emit_frag_end` and emits nothing for an RT with no colour
  output (nir_to_vir.c:1952-1990, 2038-2141, as `select.rs:2302-2304` and
  section 2 cite them). No TLB write reaches that RT.
- *yakogl's code generator*: `select.rs:2312` skips every RT whose bit is
  clear in `p.rt_mask` (`continue`), before both write paths (32-bit
  `:2315-2325`, f16 pair `:2327-2335`). `rt_mask` is built in
  `prepare.rs:2372-2376`: bit n is set only if a lane of colour output n was
  assigned. The zero constants `prepare.rs:2377-2387` gives an unwritten RT's
  roots are, by that comment, never read: the write loop does not reach them.
  A program with `rt_mask == 0` writes only the pass-through Z
  (`select.rs:2289`). `clear_mrt.rs` emits no output for a `NotCleared` RT,
  so its `rt_mask()` (`clear_mrt.rs:139-141`) is the cleared set exactly.
- *The TLB config*: a TLBU config names the RT of the writes after it
  (`tlb_config_f16`, `tlb_config_32b`, `select.rs:1810`, `:1839`,
  `(7 - rt) << 3`). Only a float RT 0 goes without one (the TLB's default,
  `:2329-2331`); a program whose first written RT is not RT 0 always has a
  config, so a quad that clears RT 2 only does not write RT 0 through the
  default.
- *The job builder* does not touch an unwritten RT. `v3d_job.c` loads each RT
  that has a buffer and `load_rt` set (`:1066`; RT 0 at `:1023-1052`), stores
  each RT that has a buffer (`:1128-1163`), and arms a clear colour only for
  an RT with `load_rt` false (`:1261-1277`). `backend.rs:6120-6173` unsets
  `load_rt` only on the TLB path, `tlb_colour = d.do_clear && !quad_colour`;
  a scissored or masked clear has `quad_colour` true, so the defaults
  `load_rt0: true`, `load_rt: [true; 3]` (`backend.rs:957`, `:973`) stand and
  every RT is loaded. The unwritten RT is loaded, left alone and stored
  back. `CLEAR_TILE_BUFFERS` (`v3d_job.c:1205`, `clear_all_render_targets`;
  `v3d_packet.xml:480-483`) ends the tile list for the next tile and is
  not part of the quad.
- *Job-wide state*: `COLOR_WRITE_MASKS` (`v3d_job.c:698-706`,
  `v3d_packet.xml:804-806`: one 32-bit word, four disable bits per RT) gates
  channels of an RT only where a write reaches it. A blend over the quad
  (`v3d_job.c:612-629`, `BLEND_CFG` with `render_target_mask = 0xf`) likewise
  acts on writes only; the code item's quad is drawn with the clear's own
  state, so check its `blend` is off in test 1.
- *Existing tests* touching an unwritten RT record the configuration only:
  `a_none_draw_buffer_is_configured_and_never_given_an_address`
  (`backend_tests.rs:3710`, `program_writing(0b101)` on three targets, RT 1
  not written) and `a_colour_output_with_no_attachment_slot_is_refused`
  (`:3736`). Neither says the unwritten RT's bytes survive.

Needs a board pixel check (the host has no TLB model): a job of 2 or 3
attachments whose quad writes RT 1 only leaves RT 0 and RT 2 equal to what was
loaded, bit for bit, inside and outside the scissor; and that an RT of an
integer internal type (8I, 32UI), loaded and not written, is not disturbed by
a TLBU config naming another RT (`0xffffff37` for RT 1). The host tests pin
the recorded state (section 4, tests 1 and 2: `rt_mask == 1 << n`, `load_rt*`
true); the issue carries the board check.

**The code item must:** change nothing in the generator or the job builder;
build one program per written RT (`rt_mask == 1 << n`), keep `load_rt*` true
on the quad path, and assert both in test 1. If the board ever shows an
unwritten RT changing, the fallback is a per-RT disable nibble in
`COLOR_WRITE_MASKS` for the RTs the clear does not name (the packet carries
one per RT; `wm.mask` repeats RT 0's today, `v3d_job.c:703-705`). The code
gives no reason to build it now.

### (b) The float clear program and a 32-bit float attachment

**Answer: `mediump` against `highp` changes nothing, and the word size comes
from the output's declared type only. A float output to an R32F attachment is
written as f16 pairs, which loses precision and range. The code item must
class an R32F attachment as `Uint` (raw 32-bit words): no `highp`, no per-RT
key.**

Read in code:

- *The precision qualifier is ignored.* `CLEAR_FS` is `mediump`
  (`backend.rs:816-817`), `clear_mrt.rs` writes `highp` (`clear_mrt.rs:192`).
  No non-test code under `src/shaderc/online/v3d/` reads a precision (the
  only `mediump` matches are test sources, e.g. `chain.rs:1114`), so both
  compile to the same writes.
- *The word size is the declared type.* `prepare.rs:2398-2406` sets `rt_int`
  for an output `integer_output` (`:2424-2426`: `int`, `uint`, `ivec`,
  `uvec`) accepts; `select.rs:2316-2325` then writes one 32-bit word per
  declared component under `tlb_config_32b` (`:1839`); every other output is
  two `VFPACK` half pairs under `tlb_config_f16` (`:2327-2335`). The
  generator never sees the attachment's format: "the V3D program is not keyed
  by the target's class, so a float output always writes f16 pairs"
  (`select.rs:1826-1838`). There is no per-RT 32-bit shader key in yakogl.
  `rt_format().clamp` (`state.rs:445`) is not one: it is the RCL's per-RT
  clamp (`v3d_job.c:1254-1285`, `TILE_RENDERING_MODE_CFG_COLOR`), not a write
  width.
- *Mesa differs*, as `docs/notes/v3d-format-matrix.md` §5 M1 records: it
  keys `f32_color_rb` for a 32-bit float colour buffer (`v3d_program.c:683-686`,
  cited) and writes `TLB_TYPE_F32_COLOR`. The R32F row is
  `v3dx_format_table.c:91` (`FORMAT(R32_FLOAT, R32F, R32F, SWIZ_X001, 32, 1)`).
- *Who is affected*: `state.rs:445-480` has one 32F row, R32F. RG32F and
  RGBA32F have no `rt_format` row and `extra_targets` refuses them
  (`backend.rs:5979-5981`). The 16F rows (R16F, RGBA16F, sRGB, RGB10_A2) hold
  f16 exactly. R32F is the only attachment where the float program's word
  size matters.
- *What an f16 write costs on R32F*: the word size is not wrong (no spill into
  the next pixel or RT; the tile buffer converts), but the value has an
  11-bit significand and a range of +-65504 (more becomes infinity). A
  `glClearBufferfv` of 1.0e10 on a scissored R32F attachment would not read
  back as given. The TLB clear is exact (`clear_words`, `state.rs:523`, its
  test `state.rs:1110`), so only the quad is lossy.

**The code item must:** leave `CLEAR_FS` and `highp` alone and add no key.
In the class choice for `clear_mrt_source` an R32F attachment is `Uint`
(`clear_mrt.rs:50-53` already says so): `layout(location = n) out uvec4`, four
32-bit words that are the float's own bits. `GlesDraw::clear_rgba` already
holds f32 bit patterns (`clear_mrt.rs:46-48`), so the uniform words need no
conversion. The class is per RT from the format:
`rt_format(f).internal_type == V3D_INTERNAL_TYPE_32F` gives `Uint`, an integer
internal type gives `Int`/`Uint`, anything else `Float`. Four words to a
one-channel 32-bpp RT is what `CLEAR_FS_INT` already does for R32I and R8I
(`backend.rs:831`). The existing one-target float quad keeps writing f16 to an
R32F RT 0; that is the same loss (M1), not this item's.

Tests that pin it today: `tests/v3d_clear_mrt.rs` pins the source and TLB
writes of each class (`float_then_int` `:170`, `int_float_uint` `:177`,
`float_kept_int` `:187`, `kept_first_then_float_and_uint` `:211`;
`expected` `:106` gives f16 pairs for `Float`, 32-bit words for `Uint`), but it
takes the class as an input, so it cannot tell that R32F was mapped to
`Uint`. `a_scissored_integer_clear_is_the_integer_programs_quad`
(`backend_tests.rs:2533`) and `writes_32bit_words` (`:2514`, RT 0's
`0xffffff3f`) pin the one-target integer program. No test names an R32F quad.

**Test the code item should add** (`backend_tests.rs`, beside test 1 of
section 4): targets `[RGBA8888, R32F]` and `[R32F, RGBA8I]`, scissored
`clear_quad`, `clear_rgba = [1.0e10, 0.0, 0.0, 1.0]`. Assert that the R32F
RT's quad carries the 32-bit config for its RT number (RT 1 `0xffffff37`,
RT 0 `0xffffff3f` = `TLB_CONFIG_32B_RT0_VEC4`) and that the fragment block
holds `1.0e10f32.to_bits()` (an f16 write would hold infinity), while the
RGBA8 RT keeps the f16 pair config. A case with an R16F attachment pins that
a half-float attachment stays `Float`.
