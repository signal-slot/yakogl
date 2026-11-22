# PowerVR four targets: the output-register budget and the over-budget answers

Status: design note, read-only; nothing here is implemented. Plan #368
(RC14, `docs/es3-cts-failures.md` RC14). It records the recommendation of
the earlier read-only item pvr-budget-note, which until now existed only in
a report, and what the item pvr-budget-fb changes.

## 1. The budget rule

A BXS-4-64 pixel has four 32-bit USC output registers (no
`eight_output_registers`; `docs/es3/backend-caps.md:128`) and the PBE emit
reads each colour target from them. `register_layout`
(`src/pvr/mrt_layout.rs:212`, `walk` `:216-240`) is the rule, over up to
`MAX_TARGETS` = 4 targets (`:68`) in emit order:

- A target takes its format's dwords (`surf_out_dwords`,
  `src/pvr/backend.rs:1040-1046`):
  - **1**: RGBA8-class (RGBA8, RGB10_A2, R8, RG8, RGB8, RGB565, RGBA4,
    RGB5_A1, R32F) and every integer layout of at most 32 bits (R8/16/32I/UI,
    RG8I/UI, RG16I/UI, RGBA8I/UI, RGB10_A2UI), by `ShcIntRt::dwords`
    (`src/shaderc/mod.rs:253-256`);
  - **2**: RGBA16F, RG16F (kept as four halves), SRGB8_ALPHA8 (F16 source,
    `backend.rs:1043`), RG32I/UI, RGBA16I/UI;
  - **4**: RGBA32I/UI and RGBA32F.
- It starts at the next free register, an even one if it takes two or more
  (`first = (next + 1) & !1`, `mrt_layout.rs:226`; `pbe2_in_xe`: a 64- or
  128-bit source sits on an even register).
- It fits if `first + dwords <= OUTPUT_REGS` (4, `:65`, `:229`). Registers
  are never back-filled, and once one target is refused every later one is
  refused too (`spilled == 0`, `:229`).
- `register_layout` (no spill) returns `LayoutError::OutputRegs { target,
  first, dwords }` (`:171`, `:233`) for the first target that does not fit.
  `layout` would instead spill that target and all later ones to tile
  buffers (`:235-237`), but only the captured programs `eot3_tb` and
  `eot4_tb`..`eot4_tb3` exist (`program_for`, `:193-201`) and the back end
  does not use them yet.

Mesa's Vulkan driver allocates the same registers on this core
(`pvr_mrt_alloc_from_buffer`, `docs/notes/pvr-int-mixed-targets.md:57-65`),
first fit where yakogl's walk is sequential
(`pvr-int-mixed-targets.md:268-272`).

## 2. Over-budget combinations

### 2.1 By dword class

Over the classes 1, 2 and 4, every sequence of one to four targets (the
walk of `mrt_layout.rs:216-240` run over all 3 + 9 + 27 + 81 = 120 of
them): **11 fit, 109 do not** (5 of two targets, 24 of three, 80 of four).
The 11: `1`, `2`, `4`; `1 1`, `1 2`, `2 1`, `2 2`; `1 1 1`, `1 1 2`,
`2 1 1`; `1 1 1 1`. For the others the first refused target is:

| first refused target | sequences (classes in emit order) |
|---|---|
| target 1 | any `4 x` (5, 6 or 8 registers), `1 4`, `2 4` (pair at 2, four needed), and their longer extensions |
| target 2 | `1 1 4`, `1 2 x`, `2 1 2`, `2 1 4`, `2 2 x` (x any class that does not fit after the pair at 2 or 4), and their extensions |
| target 3 | `1 1 1 2`, `1 1 1 4`, `1 1 2 1`, `1 1 2 2`, `1 1 2 4`, `2 1 1 1`, `2 1 1 2`, `2 1 1 4` |

(`1 2 1` places the pair at 2..3 and refuses the third; the full list is
reproduced by the walk, not copied here.)

### 2.2 The pinned rows

`tests/es3_mrt4_budget_pvr.rs` pins the four-target combinations of RGBA8
(class 1), RGBA16F (2), RGBA32UI and RGBA32F (4) as `OVER` (15 rows,
`:916-932`) and `FITS` (4, `:944-949`); `regs_of` (`:671-677`) is the class
map; `the_standin_offers_four_targets_and_the_table_matches_the_layout_rule`
(`:956-983`) asserts every `OVER` row is `LayoutError::OutputRegs`. "Today"
is the `Today` enum (`:869-905`) held by `over_budget_framebuffer_statuses_today`
(`:985-1011`) and `over_budget_draws_today` (`:1013-1044`). "Registers" is
what the walk reaches before refusing (`first + dwords` of the refused
target):

| combination | dwords | registers at refusal | today: status; draw | planned |
|---|---|---|---|---|
| RGBA8 x3 + RGBA16F | 1 1 1 2 | 4+2 = 6 (pair at 4) | COMPLETE; short kick, PBE 0 1 2 | UNSUPPORTED |
| RGBA8 x2 + RGBA16F + RGBA8 | 1 1 2 1 | 4+1 = 5 | COMPLETE; short, 0 1 2 | UNSUPPORTED |
| RGBA8 + RGBA16F + RGBA8 x2 | 1 2 1 1 | 4+1 = 5 | COMPLETE; short, 0 2 | UNSUPPORTED |
| RGBA16F + RGBA8 x3 | 2 1 1 1 | 4+1 = 5 | COMPLETE; INVALID_OPERATION, no kick | UNSUPPORTED |
| RGBA8 x2 + RGBA16F x2 | 1 1 2 2 | 4+2 = 6 | COMPLETE; short, 0 1 2 | UNSUPPORTED |
| RGBA16F + RGBA8 + RGBA16F + RGBA8 | 2 1 2 1 | 4+2 = 6 | COMPLETE; INVALID_OPERATION | UNSUPPORTED |
| RGBA16F x2 + RGBA8 x2 | 2 2 1 1 | 4+1 = 5 | COMPLETE; INVALID_OPERATION | UNSUPPORTED |
| RGBA8 + RGBA16F x3 | 1 2 2 2 | 4+2 = 6 | COMPLETE; short, 0 2 | UNSUPPORTED |
| RGBA16F + RGBA8 x2 + RGBA16F | 2 1 1 2 | 4+2 = 6 | COMPLETE; INVALID_OPERATION | UNSUPPORTED |
| RGBA16F x3 + RGBA8 | 2 2 2 1 | 4+2 = 6 (third pair) | COMPLETE; INVALID_OPERATION | UNSUPPORTED |
| RGBA16F x4 | 2 2 2 2 | 4+2 = 6 (third pair) | COMPLETE; short, 0 2 | UNSUPPORTED |
| RGBA32UI x4 | 4 4 4 4 | 4+4 = 8 (target 1) | COMPLETE; no kick, no error (no variant) | UNSUPPORTED |
| RGBA32UI + RGBA8 x3 | 4 1 1 1 | 4+1 = 5 (target 1) | COMPLETE; no kick, no error (no variant) | UNSUPPORTED |
| RGBA8 x3 + RGBA32F | 1 1 1 4 | 4+4 = 8 (target 3) | INCOMPLETE_ATTACHMENT (not renderable, plan #360); draw INVALID_FRAMEBUFFER_OPERATION | UNSUPPORTED once #360 makes RGBA32F renderable |
| RGBA32F + RGBA8 x3 | 4 1 1 1 | 4+1 = 5 (target 1) | as above | as above |

The 13 rows that draw "short" or are refused by name are all
`GL_FRAMEBUFFER_COMPLETE` today (`tests/es3_mrt4_budget_pvr.rs:36-45`: the
core has no budget, `src/gl/framebuffer.rs` answers UNSUPPORTED only for
pitch and depth-stencil planes). "Short" is the worst answer: `GL_NO_ERROR`
and a kick that silently drops the targets past register 3; no register past
3 is ever written (`over_budget_emits_stay_within_four_output_registers`,
`:1046-1069`). The fitting combinations are the `FITS` rows: RGBA8 x4 at
0 1 2 3, RGBA16F x2 at 0 2, RGBA32F alone at 0, RGBA16F + RGBA8 x2 at 0 2 3.

The 94 class sequences of 2.1 that are not among these 15 are derived from
the walk only; no test runs them.

## 3. Recommendation

1. **Answer `GL_FRAMEBUFFER_UNSUPPORTED` from `glCheckFramebufferStatus`
   for every layout `register_layout` refuses**; a draw into such a frame is
   then the ordinary `GL_INVALID_FRAMEBUFFER_OPERATION` with no kick. ES 3.0
   4.4.4.2 lets an implementation refuse a format combination with
   UNSUPPORTED, and the core already does for separate depth and stencil
   images (`src/gl/framebuffer.rs:968-977`). It is the honest answer: the
   hardware has no register for the target, and "complete, then drop the
   target" returns a wrong image. The budget is a property of the frame, so
   it belongs in the status, not in the draw.
2. **Do not spill yet; make spill the later back-end target.** The
   captured tile-buffer programs (`mrt_layout.rs:30-41`) cover only one
   sample per pixel (`:49-51`) and `pvr-int-mixed-targets.md:609-611` names
   tile buffers as what would lift the four-register limit. When the back
   end wires them, the check relaxes from `register_layout` to `layout`: a
   frame is UNSUPPORTED only where `layout` returns `NoProgram` or `Dwords`
   (`:45-48`), or the render is multisampled.
3. **The product keeps reporting 3 draw targets until a board draws four**
   (`docs/es3/backend-caps.md:211`), so the check changes nothing for the
   default build.
4. **Integer mixes are consistent with it.** An integer target beside
   others is already refused by name past four registers
   (`pvr-int-mixed-targets.md:296-298`, `:487`; the refusal is a draw error
   today); the budget check turns the same condition into the status.
5. **Open: dEQP's view of UNSUPPORTED.** Whether `fragment_out`,
   `draw_buffers_indexed` and `implementation_limits` fail or skip a case
   whose framebuffer is UNSUPPORTED is the open item deqp-unsupported; this
   note does not settle it. The ES 3.0 text permits it, and the alternative
   (a draw-time error on a complete frame) is not better for any caller.

## 4. What pvr-budget-fb changes

- `src/gl/framebuffer.rs`, `glCheckFramebufferStatus` (tail,
  `:960-995`): after the existing checks, return `GL_FRAMEBUFFER_UNSUPPORTED`
  when the draw-buffer colour attachments' dwords in emit order are refused
  by the budget. It is a back-end question like `DEPTH_ONLY_TARGETS`
  (`:981-989`): a hook next to `backend::draw_targets()`, a no-op on V3D and
  GC7000UL, and it reads the attachments the draw would emit (draw-buffer
  order, GL_NONE gaps skipped), not every attachment.
- `tests/es3_mrt4_budget_pvr.rs`: `over_budget_framebuffer_statuses_today`
  (`:985-1011`) flips to UNSUPPORTED for the 13 renderable rows and keeps
  INCOMPLETE_ATTACHMENT for the two RGBA32F rows until #360;
  `over_budget_draws_today` (`:1013-1044`) loses its `Emitted`,
  `MixRefused` and `NoVariant` families (all become
  INVALID_FRAMEBUFFER_OPERATION, no kick); the `#[ignore]` on
  `over_budget_combinations_answer_framebuffer_unsupported`
  (`:1144-1160`) comes off and the test is the pin. The table test
  (`:956-983`) should extend to all 109 class sequences.
- `tests/es3_mrt4_formats_pvr.rs`: rows that build a four-target frame over
  four registers (the RG16F row, `:786`, flagged "does not fit beside three
  RGBA8 targets", and any sRGB or integer row) expect UNSUPPORTED instead of
  a short draw.
- Not changed: `src/pvr/mrt_layout.rs`, `src/pvr/backend.rs` (the emit's
  `PVR_BACKEND_MAX_OUT_REGS` clamp stays as the backstop), the product's
  draw-target count.
