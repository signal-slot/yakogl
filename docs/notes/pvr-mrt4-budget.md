<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR BXS-4-64: the four-target output-register budget and the over-budget answers

Design note of plan #368, item `budget-doc`, for the item that follows it
(`pvr-budget-fb`). Read-only: no code changes here. It writes out the
output-register rule of `src/pvr/mrt_layout.rs`, enumerates every
combination of ES 3.0 colour-renderable formats of up to four targets that
fails it, records the answer each gives today (what
`tests/es3_mrt4_budget_pvr.rs` and `tests/es3_mrt4_formats_pvr.rs` pin),
and recommends the answer it should give: `GL_FRAMEBUFFER_UNSUPPORTED`,
the `pvr-budget-note` handoff (issue #6237) already held as an
`#[ignore]`d defect. Line numbers are of commit `38132404`; grep the
quoted name when they drift.

Related notes: `docs/notes/mrt4-v3d-pvr.md` (the plan's design note; its
section 2.3, `docs/notes/mrt4-v3d-pvr.md:292-320`, gives the same budget
rule against Mesa's allocator), and `docs/notes/pvr-int-mixed-targets.md`
(the per-target/mixed key that draws integer targets beside other
classes). RC14 is `docs/es3-cts-failures.md:489-500`: it accounts for
`implementation_limits.max_draw_buffers` and `max_color_attachments`, the
76 `draw_buffers_indexed` NotSupported rows and part of `fragment_out`.
Its row still names `PVR_RENDER_MAX_RT` 3 in `driver/src/pvr_render.h`;
the render layer is at 4 now (`:55-59`), so the row needs an update —
that is a later docs item after #364, not this note's.

## 1. The budget rule

A BXS-4-64 pixel has **four 32-bit USC output registers**: the core has
no `eight_output_registers` (`docs/es3/backend-caps.md:128`).
`OUTPUT_REGS` is 4 (`src/pvr/mrt_layout.rs:65`), and the back end emits
against the same count (`PVR_BACKEND_MAX_OUT_REGS`,
`src/pvr/backend.rs:538`). A colour target takes its format's dwords
(`surf_out_dwords`, `src/pvr/backend.rs:1040-1046`):

| dwords | ES 3.0 colour-renderable formats (this build: `src/gl/framebuffer.rs:667-707` and `:801-803`) |
|---|---|
| 1 | the RGBA8 class: RGBA8, RGB8, RG8, R8, RGB565, RGBA4, RGB5_A1, RGB10_A2 (held as RGBA8 today, `tests/es3_mrt4_formats_pvr.rs:760-762`); R16F, R32F; every integer layout of at most 32 bits: R8I/UI, RG8I/UI, RGBA8I/UI, R16I/UI, RG16I/UI, R32I/UI, RGB10_A2UI |
| 2 | RGBA16F, RGB16F, RG16F (all kept as RGBA16F surfaces, `tests/es3_mrt4_formats_pvr.rs:782-786`); SRGB8_ALPHA8 (its linear F16 halves); RGBA16I/UI, RG32I/UI |
| 4 | RGBA32I/UI |

The integer counts are `ShcIntRt::dwords`, `ceil(channels * bits / 32)`
with the 10-bit layout as one dword (`src/shaderc/mod.rs:253-256`). No
renderable format is 3-dword wide (`src/pvr/mrt_layout.rs:291-293` pins
that a 3-dword layout walks; no format reaches it). R32F, RG32F, RGBA32F
and R11F_G11F_B10F are colour-renderable only with
GL_EXT_color_buffer_float (`src/gl/framebuffer.rs:692-700`; plan #360),
and would take 1, 2, 4 and 1.

The rule is `walk` (`src/pvr/mrt_layout.rs:216-242`), reached as
`layout` (`:204`, with tile buffers) and as `register_layout` (`:212`,
registers only; the module docs `:50-58`):

1. Targets are placed in draw-buffer order at the next free register.
2. A target of two or more dwords starts at an even one
   (`first = (next + 1) & !1`, `:228`): a 64- or 128-bit PBE source on a
   `pbe2_in_xe` core (`docs/notes/pvr-int-mixed-targets.md:64-67`).
3. It fits when `first + dwords <= 4` (`:229`). Registers are never
   back-filled: RGBA8, RGBA16F, RGBA8 leaves register 1 empty and the
   third target would start at register 4 (`:17`; the walk tests
   `:295-306`).
4. `register_layout` refuses the first target that does not fit with
   `LayoutError::OutputRegs { target, first, dwords }` (`:168-171`,
   `:233`). `layout` instead spills that target and every later one to
   tile buffers (`:19-25`) when a captured end-of-tile program covers the
   spill (`program_for`, `:193-200`); the spills with no captured program
   — two targets with target 1 spilled, three targets with targets 1 and
   2 spilled — are `LayoutError::NoProgram` (`:38-44`, tests `:379-389`).
5. Where both succeed they give the same layout.

The end-of-tile programs already cover four emits: `eot1`..`eot4` and
`eot4_tb`, `eot4_tb2`, `eot4_tb3` (`src/pvr/mrt_layout.rs:31-37`;
`PVR_RENDER_EOT_MAX_RT` 4, `driver/src/pvr_render.h:61-62`; `MAX_TARGETS`
4 asserted equal to `GLES_MAX_TARGETS`, `src/pvr/mrt_layout.rs:68-69`,
`src/backend.rs:882`). What the budget refuses is not a missing program:
it is a set of formats whose registers do not fit in four.

The layouts that **fit** with up to four targets are exactly (registers,
aligned walk): one target of 1, 2 or 4 dwords (registers 0..d); 1+1 (0,
1); 1+2 and 2+1 (0, 2); 2+2 (0, 2); 1+1+1 (0, 1, 2); 1+1+2 (0, 1, 2);
2+1+1 (0, 2, 3); and 1+1+1+1 (0, 1, 2, 3). Nothing else. Over the width
sequences {1, 2, 4} to length 4 that is 11 fits of 120 sequences: 0
refused of one target, 5 of two, 24 of three, 80 of four. The module's
own test sweeps every 1..4-dword list against the captured programs
(`src/pvr/mrt_layout.rs:404-453`).

Today's product reports three draw buffers (`DRAW_TARGETS`,
`src/backend.rs:901-919`; `docs/es3/backend-caps.md:211`), and a host
test stands in four through `HOST_DRAW_TARGETS` (`src/backend.rs:921-926`,
`tests/es3_mrt4_budget_pvr.rs:626`). "Four targets" below means the
stand-in; the two- and three-target rows are reachable on the product
today.

## 2. The over-budget combinations

"W/S" is the plain sum of the dwords and the **span**: the aligned walk
continued past the limit, holes counted (what `:229` compares with four
is each target's aligned start plus its dwords; the sum misses a hole,
the span counts it). "Refused at" is `OutputRegs`' (target index, first
register, dwords). These tables are derived from `register_layout`
directly; the four-target one matches the fifteen `OVER` rows the test
pins (`tests/es3_mrt4_budget_pvr.rs:916-932`).

### 2.1 Two targets (5 refused)

| widths | W/S | refused at | `layout` gives |
|---|---|---|---|
| 1, 4 | 5 / 6 | 1, 2, 4 | 1 spill: no program (`src/pvr/mrt_layout.rs:379-389`) |
| 2, 4 | 6 / 6 | 1, 2, 4 | 1 spill: no program |
| 4, 1 | 5 / 5 | 1, 4, 1 | 1 spill: no program |
| 4, 2 | 6 / 6 | 1, 4, 2 | 1 spill: no program |
| 4, 4 | 8 / 8 | 1, 4, 4 | 1 spill: no program |

Every one of these frames is `GL_FRAMEBUFFER_COMPLETE` today
(`src/gl/framebuffer.rs:888-995` asks no register question). The draw
answers by class family, not by budget: an integer target beside a
non-integer one is refused by name with the four-registers line
(`REFUSE_FOUR_REGISTERS`, `src/pvr/target_keys.rs:21-23`, checked at
`:67`; the worked refusals in `docs/notes/pvr-int-mixed-targets.md:293-294`);
two targets of one 4-dword integer layout keep the one-layout key and the
compiler builds no variant (`chain::target_layout` is `None`,
`src/shaderc/online/powervr/chain.rs:1470-1488`, refused at `:1019`; the
`modf` `vec3` / `vec4` cases stay refused,
`docs/notes/pvr-int-mixed-targets.md:299-301`). The planned answer for
all five is `GL_FRAMEBUFFER_UNSUPPORTED`: the spills have no captured
program, so nothing draws over them.

### 2.2 Three targets (24 refused)

| widths | W/S | refused at | `layout` gives |
|---|---|---|---|
| 1, 1, 4 | 6 / 6 | 2, 2, 4 | 1 spill: `eot3_tb` |
| 1, 2, 1 | 4 / 5 | 2, 4, 1 | 1 spill: `eot3_tb` |
| 1, 2, 2 | 5 / 6 | 2, 4, 2 | 1 spill: `eot3_tb` |
| 1, 2, 4 | 7 / 8 | 2, 4, 4 | 1 spill: `eot3_tb` |
| 1, 4, 1 | 6 / 7 | 1, 2, 4 | 2 spills: no program |
| 1, 4, 2 | 7 / 8 | 1, 2, 4 | 2 spills: no program |
| 1, 4, 4 | 9 / 10 | 1, 2, 4 | 2 spills: no program |
| 2, 1, 2 | 5 / 6 | 2, 4, 2 | 1 spill: `eot3_tb` |
| 2, 1, 4 | 7 / 8 | 2, 4, 4 | 1 spill: `eot3_tb` |
| 2, 2, 1 | 5 / 5 | 2, 4, 1 | 1 spill: `eot3_tb` |
| 2, 2, 2 | 6 / 6 | 2, 4, 2 | 1 spill: `eot3_tb` |
| 2, 2, 4 | 8 / 8 | 2, 4, 4 | 1 spill: `eot3_tb` |

| widths | W/S | refused at | `layout` gives |
|---|---|---|---|
| 2, 4, 1 | 7 / 7 | 1, 2, 4 | 2 spills: no program |
| 2, 4, 2 | 8 / 8 | 1, 2, 4 | 2 spills: no program |
| 2, 4, 4 | 10 / 10 | 1, 2, 4 | 2 spills: no program |
| 4, 1, 1 | 6 / 6 | 1, 4, 1 | 2 spills: no program |
| 4, 1, 2 | 7 / 8 | 1, 4, 1 | 2 spills: no program |
| 4, 1, 4 | 9 / 10 | 1, 4, 1 | 2 spills: no program |
| 4, 2, 1 | 7 / 7 | 1, 4, 2 | 2 spills: no program |
| 4, 2, 2 | 8 / 8 | 1, 4, 2 | 2 spills: no program |
| 4, 2, 4 | 10 / 10 | 1, 4, 2 | 2 spills: no program |
| 4, 4, 1 | 9 / 9 | 1, 4, 4 | 2 spills: no program |
| 4, 4, 2 | 10 / 10 | 1, 4, 4 | 2 spills: no program |
| 4, 4, 4 | 12 / 12 | 1, 4, 4 | 2 spills: no program |

None of these 24 has a status or draw pin (no test names them; the
product reaches them at three targets). Their answers today are not
budget answers: the frames are COMPLETE, nine of the 24 spill the last
target where `layout` finds `eot3_tb` (the rule test pins 2,2,2 and 1,2,1
to `Eot3Tb`, `src/pvr/mrt_layout.rs:295-306`), and the back end emits a
spilled **last** attachment from the tile buffer when the program record
names the buffer — pinned end to end for three RGBA16F targets
(`tests/pvr_f16_mrt_gl.rs:793-825`: one kick of three, the third PBE
state with `source_pos` 0, the `eot3_tb` program) — and stops emitting
short without one (`an_attachment_past_the_pixel_output_registers_is_not_emitted`,
`src/pvr/backend_tests.rs:3451-3474`; the emit's walk and its
last-attachment exception, `src/pvr/backend.rs:9718-9742`). The 15
two-spill layouts have no program, so the compiler builds no variant and
the draw is dropped (section 2.1's family text). The planned answer for
all 24 is `GL_FRAMEBUFFER_UNSUPPORTED` — except where section 4.3 keeps
the spill.

### 2.3 Four targets: the pinned rows

The test file pins fifteen format combinations over RGBA8 (1 dword),
RGBA16F (2), RGBA32UI (4) and RGBA32F (4) as `OVER`
(`tests/es3_mrt4_budget_pvr.rs:916-932`; the arbiter test asserts
`register_layout` refuses each by name and places the fitting rows,
`:956-980`). "Status" is `glCheckFramebufferStatus`, "draw" is the
triangle of `over_budget_draws_today` (`:1013-1040`; today-answers, the
`Today` enum at `:868-900`); "planned" is the ignored
`over_budget_combinations_answer_framebuffer_unsupported` (`:1144-1155`):

| combination | widths | W/S | refused at | today: status | today: draw | planned |
|---|---|---|---|---|---|---|
| RGBA8 x3 + RGBA16F | 1,1,1,2 | 5 / 6 | 3, 4, 2 | COMPLETE | NO_ERROR, 3 states at 0,1,2 | UNSUPPORTED / IFBO, no kick |
| RGBA8 x2 + RGBA16F + RGBA8 | 1,1,2,1 | 5 / 5 | 3, 4, 1 | COMPLETE | NO_ERROR, 3 states at 0,1,2 | same |
| RGBA8 + RGBA16F + RGBA8 x2 | 1,2,1,1 | 5 / 6 | 2, 4, 1 | COMPLETE | NO_ERROR, states at 0,2 | same |
| RGBA16F + RGBA8 x3 | 2,1,1,1 | 5 / 5 | 3, 4, 1 | COMPLETE | INVALID_OPERATION, no kick | same |
| RGBA8 x2 + RGBA16F x2 | 1,1,2,2 | 6 / 6 | 3, 4, 2 | COMPLETE | NO_ERROR, 3 states at 0,1,2 | same |
| RGBA16F + RGBA8 + RGBA16F + RGBA8 | 2,1,2,1 | 6 / 7 | 2, 4, 2 | COMPLETE | INVALID_OPERATION, no kick | same |
| RGBA16F x2 + RGBA8 x2 | 2,2,1,1 | 6 / 6 | 2, 4, 1 | COMPLETE | INVALID_OPERATION, no kick | same |
| RGBA8 + RGBA16F x3 | 1,2,2,2 | 7 / 8 | 2, 4, 2 | COMPLETE | NO_ERROR, states at 0,2 | same |
| RGBA16F + RGBA8 x2 + RGBA16F | 2,1,1,2 | 6 / 6 | 3, 4, 2 | COMPLETE | INVALID_OPERATION, no kick | same |
| RGBA16F x3 + RGBA8 | 2,2,2,1 | 7 / 7 | 2, 4, 2 | COMPLETE | INVALID_OPERATION, no kick | same |
| RGBA16F x4 | 2,2,2,2 | 8 / 8 | 2, 4, 2 | COMPLETE | NO_ERROR, states at 0,2 | same |
| RGBA32UI x4 | 4,4,4,4 | 16 / 16 | 1, 4, 4 | COMPLETE | no GL error, no kick (no variant) | same |
| RGBA32UI + RGBA8 x3 | 4,1,1,1 | 7 / 7 | 1, 4, 1 | COMPLETE | no GL error, no kick (no variant) | same |
| RGBA8 x3 + RGBA32F | 1,1,1,4 | 7 / 8 | 3, 4, 4 | INCOMPLETE_ATTACHMENT (storage INVALID_ENUM) | INVALID_FRAMEBUFFER_OPERATION, no kick | as today until #360 renders RGBA32F, then UNSUPPORTED |
| RGBA32F + RGBA8 x3 | 4,1,1,1 | 7 / 7 | 1, 4, 1 | as above | as above | as above |

IFBO is `GL_INVALID_FRAMEBUFFER_OPERATION`, the standard consequence of a
non-complete framebuffer. "No variant" rows are the online compiler
building no fragment variant for the class/list and the GL core dropping
the draw counted, with no GL error (`:877-884` and its pin at
`:1026-1029`). The "3 states" / "states at 0,2" rows are the back end's
emit stopping where the next target would pass register 3
(`src/pvr/backend.rs:9731-9741`); no emitted source passes register 3
anywhere (pinned, `:1046-1065`). Every fitting combination draws today:
RGBA8 x4 draws four targets with per-buffer clears reaching all four
(`:1161-1177`), and the fitting rows are `FITS` (`:944-949`): RGBA8 x4
(0,1,2,3), RGBA16F x2 (0,2), RGBA16F + RGBA8 x2 (0,2,3), and RGBA32F
alone (0..3) — the last blocked on #360, not on the budget
(`rgba32f_alone_is_not_renderable_today`, `:1122-1134`).

**Not pinned.** The 5+24 sequences of sections 2.1-2.2 have no status or
draw test, and the 80 refused four-target width sequences exceed what the
fifteen pinned combinations sample: the pinned thirteen float rows and two
integer rows are the ones the draw families make observable today.

## 3. Why the budget belongs to glCheckFramebufferStatus

`glCheckFramebufferStatus` (`src/gl/framebuffer.rs:2236-2261`,
`check_status` `:888-995`) already answers `GL_FRAMEBUFFER_UNSUPPORTED`
as the implementation-limit token: a pitch the hardware cannot take
(`:915-917`) and split depth/stencil planes (`:971-978`). It has no
output-register question, so every over-budget frame is COMPLETE and the
failure surfaces later — a short kick (rows 1-3, 5, 8, 11), a refusal by
name (4, 6, 7, 9, 10), a dropped draw (12, 13) — with the application
told "complete" and getting undefined attachments past the cut
(`src/pvr/backend.rs:9731-9741` logs once and stores nothing). ES 3.0
4.4.4.2 lists the combination of internal formats as exactly the cause
UNSUPPORTED names; the token is already in use here for two limits.

Why the framebuffer and not the draw: the clears (`glClearBuffer*`) and
the read-back run the back end's own programs over the same registers
(`src/pvr/backend.rs:2335-2350` re-walks the rule for attachment
positions; the per-buffer clears already honour the budget registers,
`tests/es3_mrt4_budget_pvr.rs:1093-1116`), so each path would need its
own refusal; the framebuffer is the one place every path passes. Mesa
makes the same split: the output registers are allocated once per render
target set, not per draw (`pvr_alloc_mrt` / `pvr_mixed_out_layout`'s
quoted Mesa source, `docs/notes/pvr-int-mixed-targets.md:58-67`), and
its Vulkan driver advertises 4 colour attachments on this core
(`docs/es3/backend-caps.md:211`).

## 4. The recommendation and why

1. **Answer `GL_FRAMEBUFFER_UNSUPPORTED` for every layout
   `register_layout` refuses, and `GL_INVALID_FRAMEBUFFER_OPERATION` for
   a draw or clear to such a frame.** One rule, already written once and
   shared by the compiler (`chain::target_layout`,
   `src/shaderc/online/powervr/chain.rs:1470-1488`), the back end's
   acceptance (`src/pvr/target_keys.rs:51-76`) and the emit
   (`src/pvr/backend.rs:9718-9742`): the framebuffer check should ask the
   same function instead of keeping a second table
   (`docs/notes/pvr-int-mixed-targets.md:250-264` states it as one rule
   for all three). The pinned planned test asserts exactly this
   (`tests/es3_mrt4_budget_pvr.rs:1144-1155`).
2. **Tile-buffer spill is the later back-end target, not the check.**
   `layout` spills the first target that does not fit and every later one
   (`src/pvr/mrt_layout.rs:19-25`); the programs `eot3_tb`, `eot4_tb`,
   `eot4_tb2`, `eot4_tb3` are captured (`:31-37`), two layouts are not
   (`:38-44`), and the programs were captured at one sample per pixel
   (`:46-48`); the back end today emits only the last attachment from a
   tile buffer (`src/pvr/backend.rs:9725-9730`). Until the back end wires
   a layout, the honest answer is UNSUPPORTED; when a layout gains a
   wired emit it leaves the refused set in that change (the planned
   test's own words, `tests/es3_mrt4_budget_pvr.rs:1145`), and the check
   follows.
3. **Do not regress the wired three-target spill.** Three RGBA16F targets
   (2,2,2 — the Quick 3D G-buffer class) draw today with the third from
   tile buffer 0 (`tests/pvr_f16_mrt_gl.rs:793-825`,
   `src/pvr/backend_tests.rs:3476-3510`; the rule at
   `src/pvr/mrt_layout.rs:295-306`). The check must ask the back end what
   it draws — `layout`'s acceptance (spilled-with-a-captured-program plus
   the emit wiring), not bare `register_layout` — or frames the back end
   already spills turn UNSUPPORTED. Where `register_layout` refuses but
   the back end spills and emits, the check follows the back end.
4. **A mix the draw refuses is not a budget answer.** RGBA16F beside
   RGBA8 is refused today because the key carries attachment 0's class
   (`tests/es3_mrt4_budget_pvr.rs:886-894`, `tests/pvr_f16_mrt_gl.rs:855`
   and the `#[ignore]`d #388 float-mix fix at `:1182-1194`); the
   mixed-key design (`docs/notes/pvr-int-mixed-targets.md:100-148`)
   fixes the integer half and leaves room for the fourth slot at bits
   [26:30] (`:186-187`). 2,1,1 — RGBA16F + RGBA8 x2 — fits the budget
   (registers 0, 2, 3) and draws short today; the check must not answer
   UNSUPPORTED for it because its draw fails (`FITS` `:948` stays
   COMPLETE).

## 5. What `pvr-budget-fb` changes

- `src/gl/framebuffer.rs`, `glCheckFramebufferStatus` (`:2236-2261`) /
  `check_status`: after the existing checks and before the final
  `GL_FRAMEBUFFER_COMPLETE` (`:994`), ask the back end whether the colour
  attachments, in draw-buffer order, have a layout it draws; answer
  `GL_FRAMEBUFFER_UNSUPPORTED` when they do not. The question is a hook
  on the back-end table (`src/backend.rs` — owned by #354, so rebase onto
  it; `src/gl/framebuffer.rs` is also in plan #364's remaining items).
  The PowerVR answers from `surf_out_dwords` (`src/pvr/backend.rs:1040-1046`)
  plus `layout`'s acceptance (section 4.3); the V3D and the GC7000UL
  answer "fits" (their counts are 4-capable, `src/backend.rs:884-900`).
  A build at three draw buffers keeps today's answers: the hook only
  refuses lists that pass four registers, and everything the back end
  spills and emits today stays COMPLETE.
- `tests/es3_mrt4_budget_pvr.rs` (run:
  `--no-default-features --features host-test,gpu-powervr,shaderc-online
  --test es3_mrt4_budget_pvr`, header `:67`):
  `over_budget_framebuffer_statuses_today` (`:985-1001`) becomes the
  UNSUPPORTED pin for the thirteen renderable rows and keeps the two
  NotRenderable rows as today (`:991-994`); `over_budget_draws_today`
  (`:1013-1040`) loses its `Emitted`, `MixRefused` and `NoVariant` arms —
  a draw to a refused frame is IFBO with no kick, as the planned test
  already asserts (`:1150-1153`); `over_budget_combinations_answer_framebuffer_unsupported`
  (`:1144-1155`) loses its `#[ignore]`, skipping its two RGBA32F rows
  until #360 makes them renderable. The fitting-row tests stay as they
  are: `FITS`/`OVER` as arbiter (`:956-980`), the in-range check
  (`:1046-1065`), `four_rgba8_targets_reach_the_hardware` (`:1161-1177`),
  the fitting clear test (`:1093-1116`), and the #388 / #360 defects
  (`:1182-1194`, `:1198-1212`).
- `tests/es3_mrt4_formats_pvr.rs`: `ROWS` (`:757-790`) is one format as
  attachment 3 beside three RGBA8 targets (widths 1,1,1,d). Its 2-dword
  rows — SRGB8_ALPHA8 (`:759`) and RG16F (`:782-786`, kept as RGBA16F,
  refused at 3, 4, 2) — are over budget: `completeness_and_surface_of_attachment_3`
  (`:828-849`) expects UNSUPPORTED for them; `four_pbe_states_reach_the_hardware`
  (`:860-880`) no longer covers them (its `!one_reg` arm, `:875-878`,
  goes); `integer_and_srgb_attachment_3_are_refused_by_name_today`
  (`:884-898`) loses only the sRGB row (the frame is no longer complete)
  and **keeps all thirteen `RefusedInteger` rows** — 1-dword targets fit
  the budget, and their refusal is the mixed key's three-slot capacity
  (`src/pvr/target_keys.rs:19`), not the budget. `R11F_G11F_B10F` stays
  INCOMPLETE_ATTACHMENT (`:781`, `:833-836`). If the item adds the wider
  renderable names that are not in the table today (RGBA16F, RGBA16I/UI,
  RG32I/UI, RGBA32I/UI beside three RGBA8), each is over budget and
  every such new row expects UNSUPPORTED.
- Not changed: `src/pvr/mrt_layout.rs` (the arbiter), the compiler's
  variants and the tile-buffer programs (`tests/es3_pvr_mrt4.rs:449`,
  `:477`), and the RC14 row of `docs/es3-cts-failures.md` (a later docs
  item, after #364).

## 6. Open

- Whether the hook is a bool ("this back end draws this target list") or
  a status the core maps: a bool, so the core owns the token.
- Multisampled frames: the registers per pixel do not change with
  samples, but the spill programs were captured at one sample per pixel
  (`src/pvr/mrt_layout.rs:46-48`), so a multisampled spill stays
  UNSUPPORTED even after a single-sample layout is wired.
- A board capture (`tools/pco_shc/`) of the two missing tile-buffer
  layouts (two targets with target 1 spilled, three targets with targets
  1 and 2 spilled, `src/pvr/mrt_layout.rs:38-44`; today `eot2_tb` is a
  three-emit program the rule never asks for, `:41-44`) would turn those
  spills from refused to drawable, and they would leave the UNSUPPORTED
  set per section 4.2.
