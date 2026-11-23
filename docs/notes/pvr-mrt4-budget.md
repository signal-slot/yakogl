# PowerVR four targets: the output-register budget and the over-budget answers

Plan #368 (RC14), item `budget-doc`. A read-only note: it changes no code.
It fixes what a four-target PowerVR BXS-4-64 frame may hold, lists the
combinations that do not fit, and records what the answer to them is today
and what it becomes. It is the note `tests/es3_mrt4_budget_pvr.rs:11-14`
refers to; the code items that follow are `pvr-budget-fb` (section 6) and
the spill work (section 5).

## 1. The budget rule

A pixel of the BXS-4-64 has **four 32-bit USC output registers** (no
`eight_output_registers`; `docs/es3/backend-caps.md:128`). Everything the
fragment shader writes to colour targets goes through them, and the
end-of-tile program's PBE emits read them back. The rule is
`register_layout` (`src/pvr/mrt_layout.rs:212`, walk at `:216-242`), the
same walk as the back end's emit and as Mesa 26.2.1's
`pvr_mrt_alloc_from_buffer` on a `pbe2_in_xe` core
(`src/pvr/mrt_layout.rs:50-58`, `docs/notes/pvr-int-mixed-targets.md:250-272`):

```
next = 0
for each target i, in emit order (attachment order):
    d = dwords of its format                 (1, 2 or 4)
    first = d >= 2 ? (next + 1) & !1 : next  (src/pvr/mrt_layout.rs:228)
    if first + d > 4: no layout              (:229, :232-233)
    place[i] = registers first .. first + d
    next = first + d
```

- A target of two or more dwords starts on an even register (a 64-bit PBE
  source, `src/pvr/mrt_layout.rs:15-16`). The odd register it skips is a
  hole that **nothing back-fills**: the walk only moves `next` forward
  (`:17-18`).
- The first target that does not fit refuses the whole layout
  (`LayoutError::OutputRegs { target, first, dwords }`, `:168-171`). The
  walk with spilling (`layout`, `:204`) would instead store that target and
  every later one to tile buffers (`:19-25`); `register_layout` is the
  variant with no tile buffer, which is all the product has wired.
- The dwords of a format are `surf_out_dwords` (`src/pvr/backend.rs:1040-1046`):
  the packed dwords of an integer layout `ceil(channels * bits / 32)`, with
  RGB10_A2UI as one (`src/shaderc/mod.rs:253-256`); two for RGBA16F (RG16F
  is kept as four halves) and for sRGB (its linear F16 accumulation); one
  for everything else.

| class | dwords | ES 3.0 colour-renderable formats |
|---|---|---|
| 1 | 1 | R8, RG8, RGB8, RGBA8, RGB565, RGBA4, RGB5_A1, RGB10_A2 (drawn as RGBA8, `docs/es3/backend-caps.md` section 3), R8I/UI, R16I/UI, R32I/UI, RG8I/UI, RG16I/UI, RGBA8I/UI, RGB10_A2UI; R32F where renderable |
| 2 | 2 | RGBA16F, RG16F, SRGB8_ALPHA8, RG32I/UI, RGBA16I/UI |
| 4 | 4 | RGBA32I/UI; RGBA32F where renderable (`tests/es3_mrt4_budget_pvr.rs:671-677` takes it as four) |

The 25 formats the GL core lets attachment 3 be are in
`tests/es3_mrt4_formats_pvr.rs:757-787` (R11F_G11F_B10F is not renderable
there). The floating-point 32-bit formats are not renderable in ES 3.0
without `GL_EXT_color_buffer_float` (plan #360; today RGBA32F is
`GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`,
`tests/es3_mrt4_budget_pvr.rs:1123`).

What fits (`tests/es3_mrt4_budget_pvr.rs:944-949`, `FITS`): RGBA8 x4 (r0-r3),
RGBA16F x2 (r0-1, r2-3), RGBA32F or any class-4 alone (r0-r3), RGBA16F +
RGBA8 x2 (r0-1, r2, r3). Over ES 3.0's minimum of four draw buffers the
budget is the whole constraint: four **class-1** targets, in any mix of
formats, always fit.

## 2. Over-budget combinations

Registers and the walk depend on the dword classes only, so the full set is
the sequences of classes {1, 2, 4}, length 2..4, that `register_layout`
refuses (a lone target always fits). Run over all of them: 4 of the 9
two-target sequences fit (1,1; 1,2; 2,1; 2,2), 3 of the 27 three-target
ones (1,1,1; 1,1,2; 2,1,1) and 1 of the 81 four-target ones (1,1,1,1);
**5 + 24 + 80 = 109 are refused**. By the target that does not fit:

| targets | refused | at target 1 | at target 2 | at target 3 |
|---|---|---|---|---|
| 2 | 5 | 5 | - | - |
| 3 | 24 | 15 | 9 | - |
| 4 | 80 | 45 | 27 | 8 |

"Registers" below is the sum of the dwords. A layout can be refused with
the sum at four: **1,2,1** (RGBA8, RGBA16F, RGBA8) needs `r0`, `r2-3`, `r4`,
because the hole at r1 is never filled (`src/pvr/mrt_layout.rs:228-233`).
Every refused sequence has the same planned answer, so the table lists the
15 four-target rows the test pins (formats RGBA8 `A8`, RGBA16F `F16`,
RGBA32UI `U4`, RGBA32F `F4`, `tests/es3_mrt4_budget_pvr.rs:663-666`).

### 2.2 The 15 pinned four-target rows

"Walk" is the target `register_layout` refuses and
where it would start (`first`). "Today" is
`tests/es3_mrt4_budget_pvr.rs:869-912` (`Today`) and `:916-932` (`OVER`):

- *drawn short*: `glCheckFramebufferStatus` answers COMPLETE, the draw
  kicks with `GL_NO_ERROR`, and the back end stops emitting where the next
  target would pass register 3; the `source_pos` list is what it emits;
- *mix refused*: COMPLETE, `GL_INVALID_OPERATION`, no kick (RGBA16F key's
  program lays every output out as RGBA16F's);
- *no variant*: COMPLETE, no kick and **no GL error**: the online compiler
  has no integer variant for the layout; the draw is counted and logged as
  a missing variant;
- *not renderable*: `GL_INVALID_ENUM` building the storage, status
  `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`, draw
  `GL_INVALID_FRAMEBUFFER_OPERATION`.

| # | targets | classes | regs | walk (refused target @ first reg) | today | planned |
|---|---|---|---|---|---|---|
| 1 | RGBA8 x3 + RGBA16F | 1,1,1,2 | 5 | 3 @ r4 | drawn short, emits r0 r1 r2 | UNSUPPORTED |
| 2 | RGBA8 x2 + RGBA16F + RGBA8 | 1,1,2,1 | 5 | 3 @ r4 | drawn short, emits r0 r1 r2 | UNSUPPORTED |
| 3 | RGBA8 + RGBA16F + RGBA8 x2 | 1,2,1,1 | 5 | 2 @ r4 | drawn short, emits r0 r2 | UNSUPPORTED |
| 4 | RGBA16F + RGBA8 x3 | 2,1,1,1 | 5 | 3 @ r4 | mix refused | UNSUPPORTED |
| 5 | RGBA8 x2 + RGBA16F x2 | 1,1,2,2 | 6 | 3 @ r4 | drawn short, emits r0 r1 r2 | UNSUPPORTED |
| 6 | RGBA16F + RGBA8 + RGBA16F + RGBA8 | 2,1,2,1 | 6 | 2 @ r4 | mix refused | UNSUPPORTED |
| 7 | RGBA16F x2 + RGBA8 x2 | 2,2,1,1 | 6 | 2 @ r4 | mix refused | UNSUPPORTED |
| 8 | RGBA8 + RGBA16F x3 | 1,2,2,2 | 7 | 2 @ r4 | drawn short, emits r0 r2 | UNSUPPORTED |
| 9 | RGBA16F + RGBA8 x2 + RGBA16F | 2,1,1,2 | 6 | 3 @ r4 | mix refused | UNSUPPORTED |
| 10 | RGBA16F x3 + RGBA8 | 2,2,2,1 | 7 | 2 @ r4 | mix refused | UNSUPPORTED |
| 11 | RGBA16F x4 | 2,2,2,2 | 8 | 2 @ r4 | drawn short, emits r0 r2 | UNSUPPORTED |
| 12 | RGBA32UI x4 | 4,4,4,4 | 16 | 1 @ r4 | no variant | UNSUPPORTED |
| 13 | RGBA32UI + RGBA8 x3 | 4,1,1,1 | 7 | 1 @ r4 | no variant | UNSUPPORTED |
| 14 | RGBA8 x3 + RGBA32F | 1,1,1,4 | 7 | 3 @ r4 | not renderable (#360) | UNSUPPORTED once renderable |
| 15 | RGBA32F + RGBA8 x3 | 4,1,1,1 | 7 | 1 @ r4 | not renderable (#360) | UNSUPPORTED once renderable |

Rows 14 and 15 have the registers of 13: they are `register_layout`-refused
and `tests/es3_mrt4_budget_pvr.rs:986-1011` pins them as above until plan
#360 makes RGBA32F renderable; the ignored recommendation test iterates all
15 (`:1146-1158`) and so needs the two RGBA32F rows held back until then.

### 2.3 What the other formats do (`tests/es3_mrt4_formats_pvr.rs`)

That file draws three RGBA8 attachments and one of every other format as
attachment 3 (`:757-787`), so each class-2 row is the 1,1,1,2 shape of row 1
and a class-1 row always fits:

- RG16F (`GLES_SURF_RGBA16F`) is the over-budget one that **draws today**
  (`one_reg` false, `:786`): three targets, attachment 3 dropped
  (`:861-882`, "the fourth pair is zero"); planned UNSUPPORTED.
- SRGB8_ALPHA8 is class 2 and refused by name beside RGBA8 today
  (`RefusedSrgb`, `:745`, test ignored at `:989-995`); its ignored test
  asserts four emitted states, which the budget makes UNSUPPORTED instead.
- The integer rows (`RefusedInteger`, `:743`) are class 1 and fit; they
  are refused today for the key, not the budget
  (`docs/notes/pvr-int-mixed-targets.md`, which makes them draw).
- Every other class-1 row draws and stays COMPLETE.

## 3. Today's answer

No row answers `GL_FRAMEBUFFER_UNSUPPORTED`: `check_status`
(`src/gl/framebuffer.rs:888`) answers UNSUPPORTED only for the slot pitch
(`:916`), separate depth and stencil images (`:977`) and a missing colour
attachment 0 (`:992`); it has no budget. The over-budget frames therefore
complete and the PowerVR back end is where they are cut short, refused by
name, or never keyed (section 2.2). `over_budget_framebuffer_statuses_today`
and `over_budget_draws_today` pin this
(`tests/es3_mrt4_budget_pvr.rs:986-1042`), and
`over_budget_emits_stay_within_four_output_registers` (`:1047`) pins that no
kick names a register past r3.

The silent rows are the harm: rows 1, 2, 3, 5, 8 and 11 and RG16F as
attachment 3 are complete and `GL_NO_ERROR`, and the output to the dropped
targets is lost.

## 4. The recommendation

**Answer `GL_FRAMEBUFFER_UNSUPPORTED` for every layout `register_layout`
refuses, from `glCheckFramebufferStatus`, with a draw into such a frame
answering `GL_INVALID_FRAMEBUFFER_OPERATION` and no kick.**

Why:

1. **It is what ES 3.0 allows.** 4.4.4.2 lets an implementation report
   UNSUPPORTED for "an implementation-dependent set of restrictions", and
   the GL core already uses it for slot pitch and depth/stencil
   (section 3). The ES 3.0 minimum is *four draw buffers*, not every
   format combination of them; `docs/es3/backend-caps.md:211` and `:310`
   give the hardware's four 32-bit registers as the limit, within which
   four RGBA8-class outputs fit.
2. **The hardware offers no better without new work.** The wider answer is
   to spill the later targets to tile buffers (`layout`, the programs
   `eot3_tb`, `eot4_tb`, `eot4_tb2`, `eot4_tb3`,
   `src/pvr/mrt_layout.rs:27-44`), which would make 2,2,2,2 and
   4,1,1,1 work. That needs the shader to store through tile buffers, the
   multisample cases the captures do not cover (`:46-48`), and two layouts
   with no captured program (`:38-44`). Those are board-measured steps
   of their own. Until then these combinations cannot be drawn
   correctly, so the only honest answer is the incomplete-framebuffer one;
   drawing three of four silently (section 3) is a wrong answer that no
   conformance run can tell from a pass.
3. **It keeps the mixed-target design's rule.** `docs/notes/pvr-int-mixed-targets.md:268-270`
   already refuses a mix that needs a tile buffer ("a mixed program never
   spills a target to a tile buffer, so a mix that needs one is refused")
   and leaves tile buffers as the later work that lifts the limit
   (`:609`, section 7). UNSUPPORTED at status time is that refusal
   moved to where a conformant application can see it, before it draws.
4. **Spill replaces it layout by layout.** When a spill program is wired
   for a layout, that layout leaves the refused set; the status check is
   the same function and stays correct for the rest. The spilled targets
   are the last ones (`src/pvr/mrt_layout.rs:19-25`), so the planned spill
   makes the *later back-end target* the tile-buffer one, which is what
   the earlier read-only report recommended; it is the follow-up, not part
   of the check.

Not recommended: dropping the extra targets (today's answer) and lowering
`DRAW_TARGETS` to three (it fails ES 3.0's minimum of four on a GPU that has
four registers for the RGBA8 case).

## 5. What the planned answer needs from the back end

The check must use the **same function the emit uses**, so that the status
and the draw cannot disagree: `register_layout` over the dwords of the
attachments in emit order. The GL core cannot call `src/pvr/mrt_layout.rs`
(the back ends are separate modules); it would ask the back end through a
hook (a proposal; none exists yet) that answers `Ok` for the back ends
without a register budget and `register_layout(...).is_ok()` for the PowerVR. Which attachments count
is a point the item settles by reading `frame_attachment` (empty
`GL_NONE` slots are dense in the emit: `docs/notes/pvr-int-mixed-targets.md:195-200`);
this note assumes attachments in index order with the empty ones skipped.
The V3D (four TLB targets) and the GC7000UL are assumed to have no such
budget, so the hook would be the identity there.

## 6. What `pvr-budget-fb` changes

| file | change |
|---|---|
| `src/gl/framebuffer.rs` | `check_status` (`:888`) / `glCheckFramebufferStatus` (`:2237`): after the per-attachment checks (`:902-925`), answer `GL_FRAMEBUFFER_UNSUPPORTED` when the back end's budget hook refuses the colour attachments' layout. Report-es2 and default product builds keep their three-target answers (item 5 of plan #368). |
| `tests/es3_mrt4_budget_pvr.rs` | `over_budget_framebuffer_statuses_today` (`:986`) pins UNSUPPORTED for rows 1-13; `over_budget_draws_today` (`:1014`) pins `GL_INVALID_FRAMEBUFFER_OPERATION` and no kick for them, so the `Today::Emitted`, `NoVariant` and `MixRefused` arms and `over_budget_emits_stay_within_four_output_registers` (`:1047`) go; the ignored `over_budget_combinations_answer_framebuffer_unsupported` (`:1145-1158`) loses its `#[ignore]` and covers rows 1-13, rows 14-15 staying `NotRenderable` until plan #360. The FITS rows (`:944-949`) and `four_rgba8_targets_reach_the_hardware` (`:1162`) are unchanged. |
| `tests/es3_mrt4_formats_pvr.rs` | the class-2 attachment-3 rows (RG16F `:786`, SRGB8_ALPHA8 `:759`) become UNSUPPORTED in `completeness_and_surface_of_attachment_3` (`:829`) and drop out of `four_pbe_states_reach_the_hardware` (`:861`), which then draws only class-1 rows and no longer needs `one_reg`; `srgb_attachment_3_beside_rgba8_draws` (`:989`) is rewritten to assert UNSUPPORTED, and `read_of_integer_and_srgb_attachment_3` (`:1000`) drops the sRGB row. |

The back end's own over-budget paths (the short emit, the by-name
`GL_INVALID_OPERATION`, `src/pvr/backend.rs`) stay as a second line of
defence for a frame that reaches it without the status check (a draw
whose framebuffer was not checked), and are not changed here.

## 7. Board verdict lines

None: the budget is decided from the host tables and no board answers it.
The board runs of plan #368 flip `DRAW_TARGETS` to four and read the
four-target `gl_test` case and the dEQP rows of `docs/notes/mrt4-deqp-cases.md`.
