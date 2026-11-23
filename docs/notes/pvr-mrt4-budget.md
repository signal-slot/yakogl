# PowerVR: the four-target output-register budget and the over-budget answers

Status: design note (plan #368, item `budget-doc`). Read-only: no code is
changed here. The rule is `src/pvr/mrt_layout.rs`; today's answers are pinned
by `tests/es3_mrt4_budget_pvr.rs` and `tests/es3_mrt4_formats_pvr.rs`; the
change this note recommends is item `pvr-budget-fb` (section 5).

## 1. The budget rule

A BXS-4-64 pixel has four 32-bit USC output registers: the core has no
`eight_output_registers` (`docs/es3/backend-caps.md:128`).
`OUTPUT_REGS` is 4 (`src/pvr/mrt_layout.rs:65`). A colour target takes its
format's dwords (`surf_out_dwords`, `src/pvr/backend.rs:1040-1046`):

| dwords | ES 3.0 colour-renderable formats |
|---|---|
| 1 | RGBA8-class (RGBA8, RGB8, RG8, R8, RGB565, RGB5_A1, RGBA4, RGB10_A2), R16F; every integer layout of at most 32 bits (R8I/UI, RG8I/UI, RGBA8I/UI, R16I/UI, RG16I/UI, R32I/UI, RGB10_A2UI) |
| 2 | RGBA16F (RG16F and RGB16F are held as RGBA16F surfaces), SRGB8_ALPHA8 (its linear F16 halves), RGBA16I/UI, RG32I/UI |
| 4 | RGBA32I/UI |

The integer counts are `ShcIntRt::dwords`, `ceil(channels * bits / 32)`,
10-bit as one dword (`src/shaderc/mod.rs:253-256`). R32F, RG32F and RGBA32F
are colour-renderable only with GL_EXT_color_buffer_float (not ES 3.0 core;
plan #360) and would take 1, 2 and 4.

The walk is `walk` (`src/pvr/mrt_layout.rs:216-240`), reached as
`register_layout` (`:212`); it is the emit's rule of
`docs/notes/pvr-int-mixed-targets.md:250-265` (Mesa 26.2.1's
`pvr_mrt_alloc_from_buffer` without first-fit, see `:267-272` there):

1. Targets are placed in draw-buffer order at the next free register.
2. A target of two or more dwords starts at an even register
   (`first = (next + 1) & !1`, `:228`; a 64-bit PBE source).
3. It fits when `first + dwords <= 4` (`:229`). Registers are never
   back-filled: RGBA8, RGBA16F, RGBA8 leaves register 1 empty and the third
   target would start at register 4.
4. `register_layout` refuses the first target that does not fit with
   `LayoutError::OutputRegs { target, first, dwords }` (`:169-171`, `:233`).
   `layout` (`:204`) instead spills it and every later target to tile
   buffers where a program was captured (`program_for`, `:193-199`).

So the layouts that fit with up to four targets are exactly: four 1-dword
targets (registers 0-3); 2+2 (0, 2); 2+1+1 (0, 2, 3); 1+1+2 (0, 1, 2);
1+2 (0, 2); and the prefixes of these, down to a lone 4-dword target
(registers 0-3). Nothing else does.

## 2. The over-budget combinations

Enumerating widths {1, 2, 4} (a 1-dword, 2-dword and 4-dword format) through
the walk of section 1 gives 0 refused sequences of one target, 5 of two, 24
of three and 80 of four. "W / S" below is the plain sum of dwords and the
aligned span (the register one past the walk's last dword with the limit
ignored, holes counted); "refused at" is `OutputRegs`' (target index,
first register, dwords). The span, not the sum, is what the walk compares
with four.

**Two targets (5):** 1,4 (W/S 5 / 6; refused at 1, 2, 4); 2,4 (6 / 6; 1, 2,
4); 4,1 (5 / 5; 1, 4, 1); 4,2 (6 / 6; 1, 4, 2); 4,4 (8 / 8; 1, 4, 4).

**Three targets (24):**

| widths | W / S | refused at | widths | W / S | refused at |
|---|---|---|---|---|---|
| 1,1,4 | 6 / 6 | 2, 2, 4 | 2,4,1 | 7 / 7 | 1, 2, 4 |
| 1,2,1 | 4 / 5 | 2, 4, 1 | 2,4,2 | 8 / 8 | 1, 2, 4 |
| 1,2,2 | 5 / 6 | 2, 4, 2 | 2,4,4 | 10 / 10 | 1, 2, 4 |
| 1,2,4 | 7 / 8 | 2, 4, 4 | 4,1,1 | 6 / 6 | 1, 4, 1 |
| 1,4,1 | 6 / 7 | 1, 2, 4 | 4,1,2 | 7 / 8 | 1, 4, 1 |
| 1,4,2 | 7 / 8 | 1, 2, 4 | 4,1,4 | 9 / 10 | 1, 4, 1 |
| 1,4,4 | 9 / 10 | 1, 2, 4 | 4,2,1 | 7 / 7 | 1, 4, 2 |
| 2,1,2 | 5 / 6 | 2, 4, 2 | 4,2,2 | 8 / 8 | 1, 4, 2 |
| 2,1,4 | 7 / 8 | 2, 4, 4 | 4,2,4 | 10 / 10 | 1, 4, 2 |
| 2,2,1 | 5 / 5 | 2, 4, 1 | 4,4,1 | 9 / 9 | 1, 4, 4 |
| 2,2,2 | 6 / 6 | 2, 4, 2 | 4,4,2 | 10 / 10 | 1, 4, 4 |
| 2,2,4 | 8 / 8 | 2, 4, 4 | 4,4,4 | 12 / 12 | 1, 4, 4 |

**Four targets (80):** every sequence that is not a fit of section 1. The
test file pins fifteen of them, the ones over RGBA8 (1), RGBA16F (2),
RGBA32UI (4) and RGBA32F (4) (`OVER`,
`tests/es3_mrt4_budget_pvr.rs:916-932`; the test asserts `register_layout`
refuses each, `:957-980`). The registers column is W / S, and "today" is
with the host stand-in at four draw buffers (`HOST_DRAW_TARGETS`,
`src/backend.rs:930-939`; the product reports three,
`docs/es3/backend-caps.md:211`):

| combination | widths | W / S | today: status | today: draw | planned status / draw |
|---|---|---|---|---|---|
| RGBA8 x3 + RGBA16F | 1,1,1,2 | 5 / 6 | COMPLETE | NO_ERROR, 3 states emitted (`:917`) | UNSUPPORTED / INVALID_FRAMEBUFFER_OPERATION, no kick |
| RGBA8 x2 + RGBA16F + RGBA8 | 1,1,2,1 | 5 / 5 | COMPLETE | NO_ERROR, 3 emitted (`:918`) | same |
| RGBA8 + RGBA16F + RGBA8 x2 | 1,2,1,1 | 5 / 6 | COMPLETE | NO_ERROR, targets 0 and 2 (`:919`) | same |
| RGBA16F + RGBA8 x3 | 2,1,1,1 | 5 / 5 | COMPLETE | INVALID_OPERATION, mix refused by name (`:920`) | same |
| RGBA8 x2 + RGBA16F x2 | 1,1,2,2 | 6 / 6 | COMPLETE | NO_ERROR, 3 emitted (`:921`) | same |
| RGBA16F + RGBA8 + RGBA16F + RGBA8 | 2,1,2,1 | 6 / 7 | COMPLETE | INVALID_OPERATION (`:922`) | same |
| RGBA16F x2 + RGBA8 x2 | 2,2,1,1 | 6 / 6 | COMPLETE | INVALID_OPERATION (`:923`) | same |
| RGBA8 + RGBA16F x3 | 1,2,2,2 | 7 / 8 | COMPLETE | NO_ERROR, targets 0 and 2 (`:924`) | same |
| RGBA16F + RGBA8 x2 + RGBA16F | 2,1,1,2 | 6 / 6 | COMPLETE | INVALID_OPERATION (`:925`) | same |
| RGBA16F x3 + RGBA8 | 2,2,2,1 | 7 / 7 | COMPLETE | INVALID_OPERATION (`:926`) | same |
| RGBA16F x4 | 2,2,2,2 | 8 / 8 | COMPLETE | NO_ERROR, targets 0 and 2 (`:927`) | same |
| RGBA32UI x4 | 4,4,4,4 | 16 / 16 | COMPLETE | NO_ERROR, draw dropped (no variant), no kick (`:928`) | same |
| RGBA32UI + RGBA8 x3 | 4,1,1,1 | 7 / 7 | COMPLETE | NO_ERROR, dropped (`:929`) | same |
| RGBA8 x3 + RGBA32F | 1,1,1,4 | 7 / 8 | INCOMPLETE_ATTACHMENT, storage INVALID_ENUM (`:990-993`) | INVALID_FRAMEBUFFER_OPERATION, no kick (`:930`) | unchanged until #360 makes RGBA32F renderable, then UNSUPPORTED |
| RGBA32F + RGBA8 x3 | 4,1,1,1 | 7 / 7 | as above | as above (`:931`) | as above |

"Today: status" is `over_budget_framebuffer_statuses_today`
(`:986-1000`); "today: draw" is `over_budget_draws_today`
(`:1014-1044`). "Emitted" means the PBE states land on registers 0-3 and the
back end stops where the next target would pass register 3
(`src/pvr/backend.rs:9715-9740`): GL_NO_ERROR for a render that lacks
targets. "Planned" is the ignored test
`over_budget_combinations_answer_framebuffer_unsupported`
(`:1145-1157`). The fitting four-target rows are `FITS` (`:944-949`).

**Not pinned.** The test file has no row for the two- and three-target
sequences above, and the default product (three draw buffers) can reach only
those. The back end's rule today for them is not a budget answer: a last
attachment that overflows is emitted from a tile buffer when the compiler
spilled it and an `eot3_tb` program exists (`src/pvr/backend.rs:9725-9728`;
`layout` gives 1,2,1 and 2,2,2 `Eot3Tb`, `src/pvr/mrt_layout.rs:296-306`), and
an integer target in a mix is refused by name
(`docs/notes/pvr-int-mixed-targets.md:295-301`). The planned answer for
them is also UNSUPPORTED, except where section 4.3 keeps the spill.

## 3. Why the budget belongs to the framebuffer

`glCheckFramebufferStatus` answers GL_FRAMEBUFFER_UNSUPPORTED already for
an implementation limit (split depth/stencil planes,
`src/gl/framebuffer.rs:966-978`; a pitch the hardware cannot take,
`:915-917`). It has no output-register budget, so every over-budget frame is
COMPLETE and the failure surfaces later, differently per class (the table:
a short kick, a refusal by name, a dropped draw). The application is told
"complete" and gets undefined attachments.

## 4. The recommendation and why

1. **Answer GL_FRAMEBUFFER_UNSUPPORTED for every layout `register_layout`
   refuses**, and INVALID_FRAMEBUFFER_OPERATION for a draw or clear to such
   a frame (the standard consequence of a non-complete framebuffer). One
   rule, already written once and shared by the compiler, the back end and
   the unit tests (`docs/notes/pvr-int-mixed-targets.md:250-265` makes the
   emit's walk the single source): the framebuffer check asks the same
   function instead of keeping a second table. The ES 3.0 text allows it:
   UNSUPPORTED is the answer for a combination of internal formats the
   implementation cannot render together.
2. **Tile-buffer spill is the later back-end target, not the check.**
   `layout` spills the first target that does not fit and every later one
   (`src/pvr/mrt_layout.rs:19-25`); `eot3_tb`, `eot4_tb`, `eot4_tb2` and
   `eot4_tb3` are captured, two layouts are not (`:38-42`: two targets with
   target 1 spilled, three with 1 and 2 spilled, `LayoutError::NoProgram`),
   the programs were captured at one sample per pixel (`:44-46`), and the
   back end emits only the last attachment from a tile buffer today
   (`src/pvr/backend.rs:9725-9728`). Until the back end wires a layout, the
   honest answer is UNSUPPORTED; when a layout gains a wired program it
   leaves the refused set in that change, and the check uses `layout` for it.
3. **Do not regress the three-target spill.** With the product at three
   draw buffers, 1,2,1 and 2,2,2 draw today through `eot3_tb`. The check
   must ask `layout` (it accepts what has a program and a wired emit), not
   `register_layout`, for what the back end already spills, or the plan's
   own G-buffer-class frames turn UNSUPPORTED. Where the table says refused
   by `register_layout` but the back end spills, the check follows the
   back end.
4. **A mix the draw refuses is not a budget answer.** RGBA16F beside RGBA8
   is refused by name today because its key carries attachment 0's class
   (`tests/es3_mrt4_budget_pvr.rs:37-40`); the mixed-key design
   (`docs/notes/pvr-int-mixed-targets.md:100-150`) fixes that. 2,1,1 fits the
   budget (registers 0, 2, 3), so the check must not answer UNSUPPORTED for
   it because its draw fails today (`fitting_float_combinations_draw_every_target`,
   `:1183`, ignored).

Why the framebuffer and not the draw: the clears (`glClearBuffer*`) and
the read-back run the back end's own programs on the same registers and
would each need a refusal of their own; the framebuffer is the one place
every path passes. Why not "emit short": that is today's behaviour and it
leaves the missing targets undefined with no GL error. Mesa's Vulkan driver
makes the same split: the allocation fails at render-pass creation, not per
draw (`pvr_mrt_alloc_from_buffer`, `docs/notes/pvr-int-mixed-targets.md`
section 1).

## 5. What `pvr-budget-fb` changes

- `src/gl/framebuffer.rs`, `glCheckFramebufferStatus` (`:2250-2260`) /
  `check_status`: after the existing checks and before the final
  GL_FRAMEBUFFER_COMPLETE (`:994`), ask the back end whether the colour
  attachments, in draw-buffer order, have a layout; GL_FRAMEBUFFER_UNSUPPORTED
  when they do not. The question is a hook on the back-end table
  (`src/backend.rs`, owned by #354, so rebase onto it): the PowerVR answers
  from `surf_out_dwords` and `layout`/`register_layout`; the V3D and
  GC7000UL answer "fits". The default three-buffer build and report-es2 keep
  today's answers where the back end spills or has no budget.
- `tests/es3_mrt4_budget_pvr.rs`: `over_budget_framebuffer_statuses_today`
  (`:986-1000`) becomes the UNSUPPORTED pin, and
  `over_budget_combinations_answer_framebuffer_unsupported` (`:1145-1157`)
  loses its `#[ignore]`. `over_budget_draws_today` (`:1014-1044`) loses its
  `Emitted`, `MixRefused` and `NoVariant` arms (a draw to an UNSUPPORTED frame is
  INVALID_FRAMEBUFFER_OPERATION, no kick); the RGBA32F rows stay
  INCOMPLETE_ATTACHMENT until #360 (`:1199`). The fitting-row tests
  (`:957-980`, `:1047-1066`, `:1072-1115`) stay.
- `tests/es3_mrt4_formats_pvr.rs`: `four_pbe_states_reach_the_hardware`
  (`:861-882`) draws RG16F as attachment 3 beside three RGBA8 (1,1,1,2) and
  expects three states; that frame is over budget and becomes UNSUPPORTED.
  `completeness_and_surface_of_attachment_3` (`:829-851`) expects COMPLETE
  for every renderable row; the 2- and 4-dword rows (RG16F, RGB16F, RGBA16F,
  SRGB8_ALPHA8, RGBA16I/UI, RG32I/UI, RGBA32I/UI beside three RGBA8) now
  expect UNSUPPORTED, the 1-dword rows stay COMPLETE. The by-name refusal
  test `integer_and_srgb_attachment_3_are_refused_by_name_today`
  (`:885-902`) keeps only what is still reachable.
- Not changed: `src/pvr/mrt_layout.rs` (the arbiter), the compiler's
  variants, the tile-buffer programs, and the RC14 row of
  `docs/es3-cts-failures.md` (a later docs item, after #364).

## 6. Open

- Whether the hook is a bool or a status the core maps: recommend a bool
  ("fits"), so the core owns the token.
- Multisampled frames: the registers per pixel do not change with samples;
  the spill programs were captured at one sample (`src/pvr/mrt_layout.rs:44-46`), so
  a multisampled spill stays UNSUPPORTED even after a layout is wired.
- A board capture (`tools/pco_shc/README.md`) of the two missing tile-buffer
  layouts would turn them from UNSUPPORTED to drawn.
