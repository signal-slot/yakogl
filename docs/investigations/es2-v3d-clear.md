<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: scissored and masked `glClear` against dEQP-GLES2 (diagnosis)

Plan #4 item `es2-v3d-clear-diag`. Diagnosis only: no source file was changed.

Every claim is marked **[measured]** (a board result or a host run I did in
this session), **[read]** (I read the code or the CTS source; nothing ran), or
**[inferred]** (derived from the other two; could be wrong).

## 0. Result in one paragraph

The premise of the item ("nobody has confirmed glClear with scissor or masks on
V3D") does not hold. The V3D back end already draws every clear the TLB cannot
do as a clear quad, as Mesa's state tracker does, and the board has run all the
dEQP groups this item names:

- `functional.color_clear.*` 19/19 on the Pi 4 at `eddf6ab7` (2026-09-24, the
  first M8 session) [measured, `docs/online-shaderc/v3d-backend.md` "M8"].
- `functional.depth_stencil_clear.*` 11/11 at `3c193ccd` (run J) [measured,
  same file, "Evidence"].
- Sweep 30, the whole mustpass at `fe5e8e73` (2026-10-01): 17,075 pass, 25
  fail, 62 NotSupported, 3 QualityWarning. The 25 failures are all cube-map
  cases (`texture.*cube*`, `texturecubelod`); `color_clear`,
  `depth_stencil_clear`, and `fragment_ops` are not among the failures or the
  NotSupported rows [measured, `docs/es2-cts-failures.md` section A and B;
  RESULTS.md "Pi 4: sweep 30"].

So the expected verdict of every clear-dependent case today is **Pass**
(section 4). What is missing is not a fix but (a) a host test that pins the
behaviour that the board measured (`tests/v3d_clear.rs` does not exist), and
(b) a re-measurement at the integration tip: no Pi 4 run exists after
`fe5e8e73`, and the current branch has no history before the squash commit
`afb1633`, so I cannot list what changed in `src/v3d/` or `src/gl/draw.rs`
since then [inferred: small, the only later V3D-side commits visible are
`a138f73` logging, `b0a9c0c` draw-step timing, `2b219c5` depth/stencil slot
count]. `pi4-es2-r1a` / `pi4-es2-r1b` (running / ready) are that
re-measurement.

## 1. The cases and the state each clear runs under

Source: `$CCFARM_DEPS_DIR/VK-GL-CTS/modules/gles2/functional/`
[read]. The mustpass list is `tools/cts/gles2-mustpass.txt`.

### 1.1 `functional.color_clear.*` (19 cases, `es2fColorClearTest.cpp`)

One test body. Each iteration: N random clears of `GL_COLOR_BUFFER_BIT` only.
Per clear: `glScissor(x, y, w, h)` with `x, y` in `[-W, W)`, `w < W`, `h < H`
(the box may start outside the target, be empty, or be larger than what remains);
`glClearColor` random (alpha random only in `_rgba`); `glColorMask` random 4
bits. Scissor test is enabled once for the whole case when `scissor`. GL_DITHER
is left at its default (enabled); dEQP then allows one extra step of error.
Depth/stencil are not touched. The first clear is full-target with full mask
when `firstClearFull`. Result read with `glReadPixels` and compared with
the threshold of the pixel format (+1 with dither); only the channels each pixel
has "known" are compared (`pixelKnownChannelMask`).

| case | iters | clears/iter | alpha | scissor | colour mask | 1st full |
|---|---|---|---|---|---|---|
| single_rgb, single_rgba | 30 | 1-3 | no / yes | no | no | yes |
| multiple_rgb, multiple_rgba | 15 | 4-20 | no / yes | no | no | yes |
| long_rgb, long_rgba | 2 | 100-500 | no / yes | no | no | yes |
| subclears_rgb, subclears_rgba | 15 | 4-30 | no / yes | no | no | no |
| short_scissored_rgb | 30 | 2-4 | no | yes | no | yes |
| scissored_rgb, scissored_rgba | 15 | 4-30 | no / yes | yes | no | yes |
| masked_rgb, masked_rgba | 15 | 4-30 | no / yes | no | yes | yes |
| masked_scissored_rgb, masked_scissored_rgba | 15 | 4-30 | no / yes | yes | yes | yes |
| complex_rgb, complex_rgba | 15 | 5-50 | no / yes | yes | yes | **no** |
| long_masked_rgb, long_masked_rgba | 2 | 100-500 | no / yes | yes | yes | yes |

State combinations that occur: {full | scissored (incl. negative, empty,
oversize boxes)} x {mask 0xf | partial 1-14 | 0x0}. Mask 0 occurs in the masked
cases (1 in 16 clears). `complex_*` has no initial full clear, so the first
clear is itself a quad over an undefined-contents target; the reference marks
those channels "unknown" and skips them. 100-500 quads in one frame
(`long_masked_*`) test job capacity.

### 1.2 `functional.depth_stencil_clear.*` (11 cases, `es2fDepthStencilClearTests.cpp`)

First a full clear of colour red, depth 1.0, stencil 0 (mask all on, no
scissor). Then per clear: optionally `GL_SCISSOR_TEST` + `glScissor` with a box
inside the target (`1 <= w <= W`, always non-empty); `glClearDepthf` in
`[-0.2, 1.2]` (clamped by GL); `glClearStencil` random 32 bit; `glDepthMask`
random if `masked`; `glStencilMask` random 32 bit if `masked`. The mask is
`DEPTH`, `STENCIL` or both (the combined cases pick one of the three per
clear). Colour is never cleared in the loop. Then the buffers are visualised by
draws (32 depth steps through `GL_LESS` with colour mask B only; 32 stencil
steps through `GL_LEQUAL`, mask G only) and read back.

| case | iters | clears | buffers | scissor | masks |
|---|---|---|---|---|---|
| depth | 4 | 2 | D | no | no |
| depth_scissored | 4 | 16 | D | yes | no |
| depth_scissored_masked | 4 | 16 | D | yes | depth mask random |
| stencil | 4 | 2 | S | no | no |
| stencil_masked | 4 | 8 | S | no | stencil mask random |
| stencil_scissored | 4 | 16 | S | yes | no |
| stencil_scissored_masked | 4 | 16 | S | yes | stencil mask random |
| depth_stencil | 4 | 2 | D, S, D+S | no | no |
| depth_stencil_masked | 4 | 8 | D, S, D+S | no | both |
| depth_stencil_scissored | 4 | 16 | D, S, D+S | yes | no |
| depth_stencil_scissored_masked | 4 | 16 | D, S, D+S | yes | both |

The cases raise `NotSupported` when the target has no depth or stencil bits
(`:219`), which is not the case on the V3D window (D24S8 per window,
`draw.rs resolve_ds_storage`).

### 1.3 Clears inside `functional.fragment_ops.*` (1,923 cases)

Only the groups below issue a `glClear` whose state differs from "everything on,
no scissor"; blend (1,060), `interaction.basic_shader` (100), `depth.cmp_*`
(8) and `random` (100) clear with the default state or draw the background
instead [read; blend and basic_shader were not opened line by line, **[inferred]**
from the groups' use of the common `FragmentOpUtil` / `sglr` reference].

| group | cases | clear state |
|---|---|---|
| `fragment_ops.scissor.clear_color` / `clear_depth` / `clear_stencil` (`glsScissorTests.cpp` `ScissorClearCase`) | 3 | scissor box (0.1, 0.1, 0.8, 0.8 of the target) with `GL_SCISSOR_TEST` on; colour clear 0.1,0.6,0.1 (mask full); depth clear 0.0 with `GL_GREATER` test; stencil clear 123 (mask full) |
| `fragment_ops.depth_stencil.stencil_ops` (512), `stencil_depth_funcs` (81), `random` (25), `write_mask` (3) (`es2fDepthStencilTests.cpp render(clears)`) | 621 | scissored `glClear(COLOR \| STENCIL)` into cells, scissor test on during the clears, off after; colour mask default |
| `fragment_ops.stencil.*` (`es2fStencilTests.cpp`) | 17 | `StencilOp::clearStencil/clearDepth`: scissor on for each cell, `glClear(STENCIL)` or `glClear(DEPTH)` under the cell box; the initial `glClear(COLOR\|DEPTH\|STENCIL)` is unscissored. The `clear` case sets the high stencil bits (`target \| ~mask`) with `glStencilMask` narrower than the buffer, so it is a partial stencil mask |

The per-cell scissored depth or stencil clear in the middle of a frame that
has draws before it is the interesting V3D situation: the TLB clear applies at
the start of a frame, so it cannot be used after draws (section 2.3).

### 1.4 Other `glClear` users that are not state-dependent here

`fbo.render.color_clear.*` (28) clears an FBO colour (+ optional depth/stencil)
with default state. `dither.*` do not clear [read]. Their V3D behaviour is the
whole-target TLB clear (section 2.1) and is not in doubt.

## 2. How V3D implements `glClear` today

### 2.1 The GL core (shared by all back ends)

- `src/gl/state.rs: glClear` does not draw. It records a `PendingClear`
  (`src/gl/draw.rs:46`) with the clear values and a `ClearState`
  (`clear_state`: colour mask bits, depth mask, stencil write mask, scissor box
  or `None` when the test is off), taken at the call, not at the draw
  [read].
- `pending_clear_for` -> `settle_clear_state`: a `glClear` under different
  state than the pending one first emits the pending one
  (`emit_pending_clear`), so the box/mask of each clear is kept [read;
  host tests `a_sequence_of_scissored_clears_keeps_every_one_of_them`,
  `a_clear_carries_the_colour_mask_of_its_own_glclear` pass, measured below].
- `plan_clear` decides per buffer what survives: colour needs
  `mask != 0` and a colour buffer; depth needs `depth_mask` and a depth buffer;
  stencil needs `write_mask != 0` and a stencil buffer. A scissor box that holds
  the target is dropped; an empty box clears nothing (returns `None`); any other
  box sets `clear_quad = true` with `scissor_rect`. A stencil write mask other
  than 0xff also sets `clear_quad`. A colour mask other than 0xf does *not* set
  `clear_quad`; it is sent as `clear_color_mask` and is the back end's to apply
  (`draw.rs:1834 clear_color_mask`, `:1909 apply_clear_plan`) [read].
- `settle_quad_clear`: a pending clear that needs a quad is emitted before the
  next draw so that it keeps its place in order; a plain one is fused into the
  next draw (`do_clear*` in the draw) [read].
- `emit_pending_clear` builds a clear-only `GlesDraw` (`index_count = 0`) for
  flush/finish/FBO switch [read].

### 2.2 The V3D back end: which clears use the tile buffer, which a draw

`src/v3d/backend.rs: v3d_draw` (around `:4350-4415`) [read]:

```
quad_colour = state::colour_needs_quad(d)      // do_clear && (clear_quad || mask != 0xf)
tlb_colour  = d.do_clear && !quad_colour
tlb_depth/tlb_stencil = d.clear_quad ? (false,false) : (do_clear_depth, do_clear_stencil)
```

- TLB path: `begin_job`, then `j.clear.rgba8 = state::pack_rgba8(clear_rgba)`
  (the RCL's clear colour, written by the job builder), `j.clear.z`,
  `j.clear.stencil`. The tile buffer is cleared per tile at the start of the
  job, over whole tiles and every channel; nothing in the RCL restricts it to
  a box or a mask. A TLB clear is only legal for a job with no draws yet: an
  open job with `draws > 0` is flushed first, so the clear opens a new job
  (Mesa `v3d_tlb_clear`: `if (job->draw_calls_queued) buffers &= ~(load|store)`,
  `v3dx_draw.c`, mesa-26.2.1 caps tree [read]).
- GFXH-1461 (V3D 4.2): a depth-only or stencil-only TLB clear while the other
  half must be loaded is moved to the quad (`quad_depth`, `quad_stencil`,
  `backend.rs:4380-4385`) [read]; this is Mesa's rule too (`v3d_tlb_clear`,
  `devinfo->ver == 42`).
- Quad path: `draw_clear_quad` (`backend.rs:3944`) -> `state::clear_quad_draw`
  (`state.rs:148`) -> `state::job_state` -> `draw_in_job`, i.e. an ordinary
  draw of the clear program (`CLEAR_VS`/`CLEAR_FS`, built at `v3d_init`).
  The quad covers the whole target and keeps the clear's scissor
  (`scissor()` translation, y flip included), colour write mask =
  `clear_color_mask` as COLOR_WRITE_MASKS, depth test ALWAYS + write on, stencil
  ALWAYS/REPLACE on all outcomes with the clear's write mask and reference,
  blend/cull/offset/dither off, z at `2*depth-1` [read; same as Mesa
  `clear_with_quad`, `st_cb_clear.c:194-309`, which is not in this
  workspace's Mesa trees so I compared against the repo's own citations, not the
  source].
- `rts > 1` with a scissored or masked colour clear is refused by name
  (`backend.rs:4310`): ES 3 only, not reachable from GLES2 dEQP [read].
- Partial tiles: the quad is rasterised and scissored in pixels, so a box that
  cuts a tile only touches its pixels; the TLB is loaded for the other pixels
  of the tile because the job loads RT0 when it is not wholly cleared
  (`j.clear.load_rt0 = true` unless `tlb_colour`; test
  `a_masked_colour_clear_is_a_quad_and_depth_stays_in_the_tlb`) [read +
  host-checked].

### 2.3 State combinations

| # | state at `glClear` | GL core result | V3D path | status |
|---|---|---|---|---|
| 1 | colour, no scissor, mask 0xf | `do_clear`, no quad | TLB clear colour, new job if draws pending | [measured] color_clear.single/multiple/long/subclears, 19/19 at `eddf6ab7` |
| 2 | colour, scissor box inside the target | `clear_quad`, `scissor_rect` | quad, scissored, mask 0xf | [measured] color_clear.scissored_*, short_scissored (sweep 30) |
| 3 | colour, scissor box empty or entirely outside | `plan_clear` returns `None`; nothing emitted | nothing | [read] + host `a_clear_scissor_box_decides_between_nothing_the_background_and_a_quad` |
| 4 | colour, scissor box larger than the target or hanging off it | clipped by `scissor()` to the target | quad over the intersection | [measured] (color_clear draws these: `x in [-W,W)`, `w < W`) and [measured, this session] `scissor()` = the CTS rectangle for 122,880,000 pixels (section 3) |
| 5 | colour, partial mask, no scissor | `do_clear`, `clear_color_mask = m` | quad, whole target, `COLOR_WRITE_MASKS = m`; job loads RT0 | [measured] color_clear.masked_* (sweep 30); host `a_masked_colour_clear_is_a_quad_and_depth_stays_in_the_tlb` |
| 6 | colour, partial mask + scissor | `clear_quad`, mask in `clear_color_mask` | quad with both | [measured] masked_scissored, complex, long_masked |
| 7 | colour, mask 0 | `cp.color = false`; with no other buffer `plan_clear` returns `None` | nothing | [read], host `a_clear_of_a_fully_masked_buffer_does_nothing` |
| 8 | depth only, full mask, no scissor | `do_clear_depth` | TLB clear z, unless stencil half must be loaded (GFXH-1461) -> quad | [measured] depth_stencil_clear.depth |
| 9 | depth, scissor | `clear_quad` | quad: depth ALWAYS, write on, colour write mask 0 | [measured] depth_scissored, scissored_masked |
| 10 | depth, `glDepthMask(false)` | depth not cleared | nothing | [measured] depth_scissored_masked |
| 11 | stencil, mask 0xff | `do_clear_stencil` | TLB clear (or quad by GFXH-1461) | [measured] stencil |
| 12 | stencil, partial mask or scissor | `clear_quad`, `clear_stencil_write_mask` | quad: stencil REPLACE, write mask = the *glClear's* mask (not the draw's) | [measured] stencil_masked/scissored, gl_test 9 check 9 (fixed `66a1b5d2`, board run was pending when written: **not** confirmed on the board, [read] host test `a_fused_stencil_clear_uses_the_clears_write_mask`) |
| 13 | stencil mask 0 | not cleared | nothing | [read] host `a_stencil_clear_under_an_empty_mask_is_no_stencil_clear` |
| 14 | clear fused into a draw that has its own scissor/masks | the clear carries its own state (`ClearState`), the draw gets its own | quad uses the clear's box and masks (`clear_quad_draw` takes only target and clear values from `d`) | [read] host `the_clear_quad_takes_nothing_from_an_applications_draw`, `a_fused_clear_carries_its_glclear_state_whatever_changes_before_the_draw` |
| 15 | scissored clear in the middle of a frame (after draws) | `settle_quad_clear` emits it before the draw that follows | quad appended to the open job in order, no flush | [measured] fragment_ops.scissor / stencil / depth_stencil (sweep 30, F1 image pass) |
| 16 | full colour clear after draws | `do_clear` | job flushed, new job with TLB clear | [read] host `a_new_target_ends_the_job_and_a_clear_after_draws_starts_one` |
| 17 | scissored depth clear on a target whose depth an earlier job stored | quad reads depth (ALWAYS counts as a read, `zs_use`) | allowed when the target keeps depth (`ds_persist`); refused (`-EINVAL`, "reads depth an earlier job wrote") only on a target with no depth/stencil buffer | [read] host `a_depth_clear_quad_after_a_job_wrote_depth_is_refused`; unreachable from dEQP because `plan_clear` drops depth when the target has none [inferred] |
| 18 | clear on a 4x multisampled target with scissor/mask | quad with `samples = 4` | TLB clear or quad, as above | [inferred] same code; not measured on the Pi 4 (`multisample.*` are NotSupported there) |
| 19 | dithering enabled (default) | recorded (`c.dither`), not modelled | the job builder sets no dither mode (only the generated enum mentions it) | [read]; measured harmless: dEQP allows +1 step with dither and color_clear passes |

Comparison with the other back ends [read]:

- **PowerVR** (`src/pvr/backend.rs`): no per-channel colour mask in hardware, so
  the masked clear is a quad with a masked clear program (`CLEAR_MASKED`,
  `record_clear_quad`, `:2814`); background clear when not `clear_quad`
  (`:3520-3522`). `masked_clear_applied()` follows `pvr_debug_masked_clear`.
- **Vivante GC7000** (`src/vivante/backend.rs`): the mask goes into the pixel
  engine's component mask (`clear_components`), the quad's clip rectangle is the
  scissor only when `clear_quad` (`clear_clip_rect`, `:1028`). Measured on the
  i.MX8MP (gl_test 102).
- **V3D**: same split as the other two; the mask is the quad's
  COLOR_WRITE_MASKS (hardware has it) and the scissor is the job state's, which
  also applies to ordinary draws.

## 3. Host runs done for this item

1. [measured, this session] `cargo test --no-default-features --features
   host-test,gpu-v3d,shaderc-online,no-shader-table --lib clear`: **109 passed,
   0 failed, 1 ignored** (the ignore is an offline-table contract test, unrelated).
   It includes the V3D clear-quad tests in `src/v3d/backend_tests.rs`
   (`a_clear_quad_draws_every_buffer_with_the_clears_state`,
   `a_clear_quad_writes_only_the_buffers_it_clears`,
   `a_masked_colour_clear_is_a_quad_and_depth_stays_in_the_tlb`,
   `a_masked_clear_fused_into_a_draw_is_tlb_depth_stencil_then_the_quad_then_the_draw`,
   `a_partial_clear_whose_other_half_is_stored_is_drawn_by_the_quad`,
   `a_depth_clear_quad_after_a_job_wrote_depth_is_refused`) and the GL-core
   planning tests in `src/gl/draw.rs` (`a_clear_scissor_box_decides_...`,
   `a_stencil_clear_with_a_partial_write_mask_becomes_a_quad`, ...).
2. [measured, this session, throwaway test, not committed] a clear whose
   state is the colour_clear generator's (box `x,y` in `[-W,W)`, `w<W`, `h<H`,
   random 4-bit mask) on a 64x48 target, 20,000 boxes with `flip_y` false and
   again true: `state::scissor()` gives the same covered pixel set as the CTS
   reference rectangle arithmetic for all **122,880,000** pixels, **0
   mismatches**; 9,435 of 40,000 boxes were empty (reduced to `[0,0,0,0]`
   correctly); `colour_needs_quad` was true for the 40,000 boxes (all
   `clear_quad`).
3. [limit] `tests/v3d_sim_mesa.rs` cannot run a clear. It is the QPU/TMU
   interpreter (`src/shaderc/online/v3d/sim.rs`) checked against Mesa's compiled
   shaders. There is no tile buffer, rasteriser, COLOR_WRITE_MASKS, scissor or
   RCL in it. The pixels of a V3D clear exist only on the board. What the host
   can pin is (a) the *decision* (TLB vs quad, per buffer), (b) the quad's
   state (`clear_quad_draw`, `job_state`) and (c) the scissor/mask geometry
   against the CTS reference (item 2).
4. [not done] Running dEQP on the host V3D: there is no host V3D renderer;
   `tools/cts/host-deqp-run.sh` runs the PowerVR and GC7000 back ends only
   (`es2-host-deqp`).

## 4. Expected dEQP verdict per case today

All **Pass** [measured at `fe5e8e73` for the Pi 4 (2026-10-01) and for
color_clear/depth_stencil_clear at `eddf6ab7`/`3c193ccd`; **[inferred]** at the
integration tip, which no Pi 4 run has covered; `pi4-es2-r1a` / `-r1b` are
that run].

| group | cases | expected |
|---|---|---|
| `functional.color_clear.*` | 19 | Pass |
| `functional.depth_stencil_clear.*` | 11 | Pass |
| `functional.fragment_ops.scissor.clear_*` | 3 | Pass |
| `functional.fragment_ops.depth_stencil.*` (clear cells) | 621 | Pass |
| `functional.fragment_ops.stencil.*` | 17 | Pass |
| `functional.fbo.render.color_clear.*` | 28 | Pass (not state-dependent) |

Residual failure risks, none measured as failing [inferred]:

1. Regression since `fe5e8e73`: the V3D files changed after the sweep include
   `2b219c5` (depth/stencil retention: 32 slots), the part of the back end
   that decides whether a scissored depth/stencil clear can read the earlier
   job's buffer (state 17). Re-measure `depth_stencil_clear` and
   `fragment_ops.depth_stencil` first.
2. Job capacity in `long_masked_*` (100-500 quads in one frame): passes
   today; a change to the BCL/data arena sizes would show here first. The
   backend flushes a full job and continues
   (`record` -> `Attempt::NoMemory`/`Full` handling) [read].
3. gl_test 9 check 9 (stencil write mask of a fused clear) was fixed in
   `66a1b5d2` and was "pending a board run" in `v3d-backend.md`; the dEQP
   `stencil_*masked` cases cover the same rule and pass, but the gl_test itself
   is not confirmed on the board in the documents I read.

## 5. Fix proposal

There is no defect to fix. The proposal is tests, plus one decision.

### 5.1 `tests/v3d_clear.rs` (new, host, `#![cfg(all(feature = "gpu-v3d", feature = "host-test"))]`)

`yakogl::v3d::state` and `yakogl::backend::GlesDraw` are public, so an
integration test can call `state::scissor`, `state::colour_needs_quad`,
`state::clear_quad_draw` and `state::job_state`. It cannot reach the fake job
builder in `src/v3d/backend_tests.rs` (that is linked into the lib's test
binary only); tests that need the recorded jobs stay there.

1. `scissor_is_the_cts_rectangle`: promote the throwaway of section 3 (item 2):
   for `flip_y` in {false, true}, boxes drawn from the color_clear generator
   (negative origin, oversize, empty, zero-width), assert the covered pixel set
   of `state::scissor` equals the CTS rectangle; empty boxes map to
   `[0,0,0,0]`. Pins the y flip and the clipping the quad relies on.
2. `colour_needs_quad_is_the_tlb_decision`: truth table over
   `{do_clear, clear_quad, clear_color_mask in 0..=15}`: quad iff
   `do_clear && (clear_quad || mask != 0xf)`; masks 0xf / no scissor is the TLB.
3. `quad_state_for_each_buffer_combination`: `clear_quad_draw` with
   `QuadBuffers` over the 8 combinations of colour/depth/stencil, scissored
   and not: colour mask is the clear's or 0; depth test ALWAYS + write only with
   depth; stencil REPLACE x3 with the clear's write mask, both faces; no blend
   or cull; viewport the target; `scissor` off when `scissored == false`.
   (needs a `ShcShader` for `ClearProg`; use `ShcShader::default()` if it
   exists, otherwise leave this one in `backend_tests.rs`).
4. `depth_clear_quad_z`: `clear_quad_vertices(d)` z = `2d-1` for the CTS
   depth values `-0.2..1.2` clamped to `[0,1]` (the GL core clamps; the test
   uses clamped values) and the transformed depth equals `d` after `viewport()`.
5. `masked_fused_clear_keeps_its_own_masks`: a `GlesDraw` with both a clear
   (`clear_color_mask`, `clear_stencil_write_mask`) and a draw state (different
   `color_mask`, `stencil_write_mask`, `scissor`): the quad takes the clear's
   values, never the draw's.
6. `cts_color_clear_reference`: a software model of the V3D semantics (quad =
   scissor box, per-channel mask; TLB = whole target) driven by the colour_clear
   generator (seeded like the CTS: `deInt32Hash(iter)` need not be reproduced;
   any seed) and compared with the CTS reference algorithm
   (`pixelKnownChannelMask`, span walk) for all 19 case parameter sets. This
   checks that "TLB clear for full/unmasked, quad otherwise" yields the CTS
   reference image, *given* the hardware does what the model says; the board
   tests the second half.
7. For the GL core (`plan_clear`) the existing `src/gl/draw.rs` tests already
   cover the box/mask matrix; add only the depth_stencil_clear matrix
   (random depth mask x scissor x {D, S, D+S}) as a table test there, not in
   `tests/v3d_clear.rs`, because `plan_clear` is private.

### 5.2 Tests that stay in `src/v3d/backend_tests.rs`

- a 500-quad frame (`long_masked`): 500 `clear_quad` draws fill a job, the
  backend flushes and continues; the recorded draw count and the flush
  count; the colour is loaded (`load_rt0`) in the continuation job.
- depth_stencil_clear sequence: a scissored depth quad then a scissored
  stencil quad then a full TLB clear in one job order (the recorded order of
  quad, quad, TLB-flush).

### 5.3 Decision for the owner / for the plan

- `es2-v3d-clear` (ready, "scissored and masked glClear drawn correctly") has
  nothing to implement. Proposed: shrink it to section 5.1 and 5.2, which
  is host test work, no `src/` change; or drop it and fold 5.1/5.2 into a
  later cleanup. Either way it should not hold a Pi 4 slot.
- The only thing that can still fail on the board here is a regression since
  `fe5e8e73`; `pi4-es2-r1a/b` find it. If `depth_stencil_clear` or
  `fragment_ops.depth_stencil` fail there, the first suspect is the depth
  retention change `2b219c5` (state 17), at `src/v3d/backend.rs`:
  `zs_use`/`record` ("reads depth an earlier job wrote") and `begin_job`
  (`z_valid`, `zs_load`), not the clear quad.

## 6. Claims I could not check

- Mesa's `st_cb_clear.c` is not in any Mesa tree here (`mesa-26.2.1-src` is
  empty; `mesa-26.2.1-caps` holds `v3dx_draw.c`). Mesa's state-tracker rules
  are quoted from `src/v3d/state.rs` and `v3d-backend.md`, not re-read.
- No Pi 4 result after `fe5e8e73`; the repository's history before the squash
  commit `afb1633` is not in this clone, so "unchanged since" cannot be shown.
- The dEQP reference-image tolerances on the Pi 4 window format
  (`getColorThreshold`): the passes are measured, the thresholds are not
  re-derived.
