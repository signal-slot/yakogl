<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall general lowering: Mesa pass map (S2-S6) and the femtovg per-program needs

Date 2026-10-20, tip `db5701fb1`. Plan #152 item `notes-lowering`, for issue
#1384 (`gh` was not readable from the worker; the scope is
`driver/docs/nanopc-t6-mali.md`, "The general lowering (approved 2026-10-19)").
Host research only: no file under `src/` or `tools/` changed, no board run.

Citations are Mesa 26.2.1 (`MESA_PAN_TREE`, here
`$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`). `B/` is
`src/panfrost/compiler/bifrost/`, `N/` is `src/compiler/nir/`. Our files are
under `src/shaderc/online/mali/` as on main at `db5701fb1`; S1c
(`work/mali-lower-s1c`) moved the fragment side into `nir_fs.rs` (S1c landed;
b75d518c deleted `frag.rs`), so re-grep the names before relying on a line.

The target is the G610, arch 10: `pan_get_nir_shader_compiler_options` returns
`bifrost_nir_options_v9` for arch 9 and 10 (`src/panfrost/compiler/pan_compiler.c:93-97`),
`DEFINE_OPTIONS(.., 9, ..)` at `B/bifrost_compile.h:164`. Every
`arch < 11` / `arch >= 9` condition below is read for arch 10.

## Part 1. The transcription map

### 0. The order Mesa runs (what "in Mesa's order" means)

| Stage | Where | What |
|---|---|---|
| GL front-end optimisation loop | `gl_nir_opts`, `gl_nir_linker.c:45-130`; the repo's oracle copy is `src/panfrost/pan_shc/pan_shc.c:119-178` | contains **its own** peephole_select: `limit = 8, indirect_load_ok, expensive_alu_ok` (`pan_shc.c:156-161`), then a second one `limit = 0, discard_ok = true` (`:169-173`), then `nir_opt_loop_unroll` (`:174-177`) |
| `bifrost_preprocess_nir` | `B/bifrost_nir.c:400`, called from `pan_compiler.c:114` | `nir_lower_terminate_to_demote` for fragment (`:411-416`) |
| `bifrost_postprocess_nir` | `B/bifrost_nir.c:864`, called from `pan_compiler.c:139` | `nir_lower_frag_coord_to_pixel_coord` (`:910`); `nir_lower_flrp` (`:1083`); `nir_lower_bit_size` (`:1087`); `nir_lower_alu_width` (`:1088`); `nir_lower_phis_to_scalar` (`:1090`); then `bi_optimize_loop` |
| `bi_optimize_loop` | `B/bifrost_nir.c:202-281` | the fixed-point loop below |
| late NIR | `B/bifrost_nir.c:300-380` | algebraic, `pan_nir_lower_bool_to_bitsize` (`:349`), `bifrost_nir_lower_algebraic_late` (`:358`), `nir_opt_algebraic_late` loops (`:338`, `:365`), `nir_opt_sink`/`nir_opt_move` (`:380-381`) |
| NIR to BIR | `bifrost_compile.c` `emit_cf_list` (`:4428`) then `bi_opt_push_ubo` (`:4448`), `bi_opt_copy_prop`, `bi_opt_constant_fold`, `bi_opt_mod_prop_forward` (`:4457`) / `_backward`, `bi_opt_dce`, `bi_opt_cse`, `bi_opt_reorder_push`, `bi_lower_opt_instructions` (`:4477`), then for arch >= 9 `va_lower_isel`, `va_optimize` (`:4480+`) | |

`bi_optimize_loop` body, in order (all `B/bifrost_nir.c`): `nir_lower_alu_width` +
`nir_opt_vectorize` (`:228-229`), copy_prop/dce/cse, **peephole_select**
(`:234-239`), `nir_opt_idiv_const`, `nir_opt_algebraic` (`:241`), constant
folding (`:242`), `nir_opt_dead_cf` (`:244`), `nir_opt_loop` (`:247`),
`nir_opt_if` for arch >= 9 with
`optimize_phi_true_false | avoid_64bit_phis` (`:261-265`), `nir_opt_phi_to_bool`,
**`nir_opt_loop_unroll`** (`:268`), `nir_opt_remove_phis`, `nir_opt_undef`.
Loops until no progress.

Which of our files: the graph passes are `nir.rs` for the vertex stage today
(`build` `:709`, `late_rules` `:1010`, `fuse` `:1027`, `dce` `:1116`,
`opt_move` `:1193`) and `nir_fs.rs` for the fragment stage (`walk` `:254`,
`layout` `:453`, `select` `:768`); S1c puts the fragment stage on the same
graph. The new passes of S2-S6 are added next to `late_rules`/`fuse` in
`nir.rs`, not as new files, unless the S2 author splits them out.

### 1. `nir_opt_peephole_select` (step S5)

- **What it does.** A block `if (c) { A } else { B }` with one block per side
  and only "allowed" instructions becomes `A; B; phi -> bcsel(c, a, b)`
  (`N/nir_opt_peephole_select.c:505-602`, the rewrite at `:575-599`). Both sides
  execute unconditionally, so the sides must be speculation-safe.
- **Options as bifrost calls it** (`B/bifrost_nir.c:234-237`):
  `limit = 64, expensive_alu_ok = true`. `indirect_load_ok` and `discard_ok`
  are false (default). Effects:
  - ALU instructions are counted (`:250-310`); `mov`-like ops (`mov fneg ineg fabs iabs vec*`) and
    `fsat` are free; an `fmul` used only by an `fadd` is free (`is_only_used_by_fadd`,
    it will fuse); the total must be `<= 64` (`:548`).
  - Expensive ALU (`frcp frsq fsin fcos fexp2 flog2 fpow fdiv ...`, `:266-281`) is
    allowed because `expensive_alu_ok`.
  - Any intrinsic other than a short allow-list returns false (`:200-240`);
    `demote`/`terminate*` only if `discard_ok` (`:227-232`). **An `if`
    containing `discard` is therefore never flattened by bifrost's call: it
    stays a real `BRANCHZ` block (S6).** (The GL-front-end copy with
    `discard_ok` and `limit = 0` flattens only if the sides are bare moves.)
  - Per-`if` hints override: `selection_control_flatten` raises the limit,
    `dont_flatten` sets 0 (`get_options_for_if`, `:328-344`). GLSL ES 1.00
    carries none.
  - `nir_opt_collapse_if` (`:409`) runs first in each call and merges nested
    `if (a) { if (b) ...}` (`:529`).
  - A side that ends in `break`/loop jump or has a second block is not
    handled (`:536-538`).
- **Maps to.** A new graph pass in `nir.rs` working on the structured regions
  with phis that S1 introduced; its output is `bcsel` (an `Alu::Bcsel` row in
  `emit.rs`, section 4). There is no BIR counterpart: after the pass, a flat
  program needs no blocks.
- **Needed by.** S5. All `scissorMask` `if`s and the `glyphTextureType` `if`s
  of the femtovg shaders are candidates; whether each fits in 64 is
  **unmeasured**: S5 must count it with the shim reference (`pan_shc`) before
  claiming "peephole only" for a program. The `texType` `if`s of the image
  shaders reassign a whole `vec4`, small. Programs whose `if` survives (any
  `discard`) need S6's real blocks.
- **Trap.** The count is taken after `nir_opt_algebraic`, `fsat` folding and
  `fmul`/`fadd` fusion; counting our IR before those gives a wrong answer
  near 64.
- **Measured (S5, 2026-10-22).** Two passes are transcribed
  (`nir.rs peephole_select`, run from `nir_fs.rs select_highp` after
  `dead_cf`): the GL front end's (`pan_shc.c:156-161`: limit 8, a uniform
  read allowed since it is still a `load_deref` there) and bifrost's (limit
  64; by then a uniform read is `load_ubo`, which
  `block_check_for_allowed_instrs` refuses: an `if` whose side reads a
  uniform is flattened by the first pass or not at all). A varying is read
  at the shader start (`nir_lower_io_to_temporaries`), so it never blocks;
  a lane already read in a dominating block is CSE'd and not counted
  (`nir_opt_cse` runs before each pass); a texture look-up refuses. The
  count: `fill_color.nodiscard`'s scissor `if` and `fvgtest_cover`'s are
  **real blocks** in the shim's bytes (two `BRANCHZ` each: both sides read
  `frag[]`, and run past 8); `stencil.nodiscard`'s `if` is dead
  (`nir_opt_dead_cf`, nothing after it reads its locals) and dropped;
  `fill_color.nodiscard.noscissor` has none. The five pan_shc references
  `tools/mali/testdata/s5_*.frag` pin the forms: one and twelve
  operations over varyings flatten to `CSEL` (peep_small, peep_big), nine
  min/max over uniforms stay blocks (if_noelse: `BRANCHZ` to the join and
  no jump; if_nested: four blocks a level; phi_const: a phi of two
  constants moved with `IADD_IMM`, the join's phis in the reverse of the
  variables' first-reference order, `nir_phi_builder_finish` inserting
  each at its block's start). `nir_opt_collapse_if` is not transcribed: no
  program of the corpus needed it (a nested `if` with nothing between the
  two conditions).

### 2. `nir_opt_loop_unroll` (step S6)

- **What it does.** Replaces a loop by its unrolled body when the trip count
  is bounded and cost fits: `check_unrolling_restrictions`
  (`N/nir_opt_loop_unroll.c:899-931`): unroll if
  `trip_count <= max_unroll_iterations` and
  `instr_cost * trip_count <= max_iter * LOOP_UNROLL_LIMIT`
  (`LOOP_UNROLL_LIMIT = 26`, `:36`).
- **Options** (`DEFINE_OPTIONS`, `B/bifrost_compile.h:140-144`):
  `max_unroll_iterations = 32` (so cost limit 832), `force_indirect_unrolling =
  shader_in | shader_out | function_temp`, `force_indirect_unrolling_sampler =
  true`; no `_aggressive`/`_fp64` limits. Called with no arguments at
  `B/bifrost_nir.c:268` (`nir_opt_loop_unroll(shader)`, `N/:1196`).
- **Shapes** (`process_loops`, `N/:990-1160`): `do {..} while(false)` wrapper
  (`wrapper_unroll :547`); known trip count (`simple_unroll :105`); **two
  terminators** where one is the counting limit and the other unknown
  (`complex_unroll :338`, taken at `:1125-1149`); one unknown terminator
  (`complex_unroll_single_terminator :466`, `:1151`); loops with a nested loop
  are skipped unless they break after one iteration (`:1096`).
- **The femtovg loop** (`filter_image*.frag`): `for (float i = 1.0; i <= 24.0;
  i += 1.) { if (i >= sampleCount) break; ... }`. NIR sees two terminators:
  `i <= 24` (limiting, trip count 24 <= 32) and `i >= sampleCount` (uniform
  derived, unknown), so it takes `complex_unroll` (`:1125`): 24 copies of the
  body, each guarded by the break test. Each copy has 2 `texture2D`, so the
  unrolled program has 1 + 48 look-ups and 24 guarded regions; the break tests
  are `if`s with a `bcsel`-able tail only if the sides are small. **Cost
  check**: settled, see "Gaps" 4: Mesa unrolls it (24 guarded copies, no
  back-edge), so S6 needs no `emit_loop`.
- **Maps to.** A graph pass in `nir.rs` (needs a loop-analysis helper:
  trip count from the constant start/step/limit compare; transcribe
  `nir_loop_analyze.c`'s `find_trip_count` for the float-induction case).
  Float induction variables (`i += 1.`) are the one non-obvious input.
- **Needed by.** S6 only (the two `filter_image` programs).
- **Caps to watch.** `bir.rs:26-28` `MAX_INSTRS = 192`, `MAX_NODES = 192`; the
  unrolled body is about 940 instructions, 5 times the cap (see "Gaps" 5). That
  is a limit of our arenas, not of Mesa; raise it in the S6 item or the program
  stays refused.

### 3. `bi_opt_push_ubo` and `PAN_MAX_PUSH` (step S3)

- **PAN_MAX_PUSH** is 128 words (`src/panfrost/compiler/pan_compiler.h:137`),
  `info->fau.max = PAN_MAX_PUSH` (`B/bifrost_nir.c:1291`); `pan_fau_available`
  is `max - count` (`pan_compiler.h:175-179`). Our `FAU_MAX_WORDS = 128`
  (`fau.rs:43`) is already the same constant. `frag[14]` is 56 words.
- **`bi_is_pushable_ubo`** (`B/bi_opt_push_ubo.c:21-31`): a `LOAD_UBO` with
  constant offset (`src[0]`), constant ubo index (`src[1]`), the ubo in
  `inputs->fau.pushable_ubos`, offset 4-byte aligned. Anything else keeps the
  UBO uploaded (`ubo_mask` bit, `:166-175`).
- **`bi_analyze_ranges`** (`:51-92`) records, per ubo, the word ranges read;
  **`bi_pick_ubo`** (`:121-146`) pushes from the *last* ubo first, in
  increasing word order, stopping when `pan_fau_available < range` (`:126`);
  **`bi_opt_push_ubo`** (`:150-205`) replaces each pushed load by
  `bi_collect_i32` of `bi_fau(BIR_FAU_UNIFORM | idx, hi)` (`:188-203`).
  Fragment: `bi_pick_blend_constants` first (`:95-118`).
- **`bi_opt_reorder_push`** (`:306+`, called `bifrost_compile.c:4473`) then
  reorders pushed words to pair adjacent ones (FAU is read in 64-bit pairs).
  The byte-equality of the FAU table depends on it.
- **Our uniform path today** feeds `fs_uniform` (`nir_fs.rs:719`) and
  `FauWord::{Uniform, UniformHalves}` (`fau.rs:21-38`), whose fragment
  refusal is "a fragment shader uniform other than one mediump vec4 read in its
  own lane pairs" (`mod.rs:286`, the verdict of `fvgtest_inner`). `fau.rs`
  already derives from `bi_opt_push_ubo.c` (its header). S3 generalises
  `UniformHalves` (mediump pairs) to fp32 `Uniform { dword }` words for the
  fragment stage and lifts the `UniformArray`/`UniformIndexed` refusals
  (`mod.rs:280-281`) for constant indices only.
- **Fallback.** `frag[14]` fits; a uniform block past 128 words uses `LD_BUF`
  (`mod.rs:289`), out of scope for femtovg.
- **Needed by.** S3 (`fvgtest_inner`, case 30), and every later step.
- **Caveat found on the way.** `lower_uniforms_to_ubo = true`
  (`B/bifrost_compile.h:135`): Mesa turns *all* default-block uniforms
  (including `viewSize`, the samplers' parameters) into one UBO 0 read, plus
  the sysval UBO; the push order puts the sysval UBO (last ubo number) first.
  Check the descriptor/FAU order against the shim dump, not by reasoning.

### 4. `bi_emit_alu` (step S4; `B/bifrost_compile.c:2621-3400`)

`bi_emit_alu` is a `switch (instr->op)` on already-lowered NIR. Many GLSL
operations never reach it: Mesa lowers them in NIR first. That is the main
trap of S4: a row exists only for the *post-lowering* op. Valhall's rows,
for the ops the design lists:

| GLSL / IR op | What NIR does first | `bi_emit_alu` case (line) | BIR | Our `emit.rs` `ROWS` | Step |
|---|---|---|---|---|---|
| `fmin` | none | `nir_op_fmin` (`:3298`) | `FMIN.f32` | present (`Fmin 32 1`) | S4 |
| `fmax` | `fmax(a, 0)` becomes `fclamp_pos`, `fmin(fmax(a,-1),1)` becomes `fsat_signed` (`bifrost_nir_algebraic.py:34-36`) | `nir_op_fmax` (`:3302`); clamps `:2881-2897` | `FMAX.f32`; `FCLAMP` + `.clamp` | present (`Fmax 32 1`); `FCLAMP` not | S4 |
| `clamp(x,0,1)` | `fsat` (algebraic) | `nir_op_fsat` (`:2881`) | `FCLAMP.f32` clamp 0..1; lowered to `FADD x, -0.0` + clamp by `bi_lower_opt_instructions` unless fused | missing: `Fsat` | S4 |
| `ffma` / `mix` | `flrp` lowered (`lower_flrp32`, `B/bifrost_compile.h:82-86`, `nir_lower_flrp :1083`) to `ffma`/`fadd`/`fmul`; `ffma_weak` to `ffma` (`N/nir_opt_algebraic.py:422-423`) | `nir_op_ffma` (`:2869`) | `FMA.f32` | present (`Ffma 32 1`) | S4 |
| `fdot2` | `nir_lower_alu_width` -> `lower_fdot` (`N/nir_lower_alu_width.c:161-190`, switch `:362-367`): a chain `fmul` then `ffma_weak` per component, **reverse component order** unless `nir_fp_exact` (`:166`); `nir_prefers_fmad` is false (`nir.h:7448`), so `ffma` not `fmul`+`fadd` | none (no `fdot2` case exists) | `FMA`, `FMA`... | needs `Fmul`/`Ffma` only; **the lowering rule is new** (graph pass) | S4 |
| `flt` | `bifrost_nir.c`'s `pan_nir_lower_bool_to_bitsize` makes it `flt_pan` with a 32-bit bool (`pan_nir_lower_bool_to_bitsize.c:96-111`, `CASE_CMP :89-93`) | `nir_op_flt_pan` (`:3267-3272`) via `bi_translate_cmpf` (`:2583-2606`) | `FCMP.f32 .lt`, result type M1 | missing: `Flt` | S4 |
| `fle` | no `fle` opcode in NIR (`N/nir_opcodes.py:926-931`: only `flt fge feq fneu`); GLSL `<=` arrives as `fge` with swapped sources (`glsl_to_nir.cpp:2488`) | `nir_op_fge_pan` (`:3270`) | `FCMP.f32 .ge` | missing: `Fge` (our IR's `AluOp::Fle`, `ir.rs:258`, must be swapped into `fge`) | S4 |
| `feq` / `fne` | `fneu` is the `!=` op; `feq`/`fneu` set `preserve_nan` (`glsl_to_nir.cpp:2495-2512`) | `feq_pan`, `fneu_pan` (`:3267-3269`) | `FCMP.f32 .eq/.ne` | missing | S4 (atan2 uses `feq`/`flt`) |
| `fsqrt` | `lower_fsqrt`: `fsqrt(a)` becomes `frcp(frsq(a))` (`N/nir_opt_algebraic.py:1908`; option `B/bifrost_compile.h:93`) | `nir_op_frsq` (`:3391`), `nir_op_frcp` (`:3398`) | `FRSQ.f32` then `FRCP.f32`; Valhall has no `BIFROST_NO_FP32_TRANSCENDENTALS` quirk, so a single instruction each | `Frcp` present; `Frsq` missing | S4 |
| `frcp` (also `a / b`) | `lower_fdiv` (`N/nir_opt_algebraic.py:1905`): `fdiv(a,b)` -> `fmul(a, frcp(b))` | `nir_op_frcp` (`:3398`) | `FRCP.f32` | present | S4 |
| `ffract` | arch < 11: `lower_ffract` (`DEFINE_OPTIONS :86`) -> `fsub(a, ffloor(a))` (`N/nir_opt_algebraic.py:545-546`); arch >= 11 keeps it and lowers late (`bifrost_nir_algebraic.py:92-93`) | `ffloor` -> `nir_op_ffloor` (`:3275-3295`) | `FROUND.f32 .rtn`, then `FADD a, -floor` | missing: `Ffloor`; `fsub` is `fadd(a, fneg(b))` | S4 |
| `fceil` | none | same `fround` case (`:3275-3295`) | `FROUND .rtp` | missing | S4 (filter_image's `ceil`) |
| `fsign` | `lower_fsign` (`DEFINE_OPTIONS :95`): `fsub(b2f(flt(0,a)), b2f(flt(a,0)))` (`N/nir_opt_algebraic.py:2818`), the `b2f`s then become `bcsel` | none | `FCMP` x2 + `MUX` x2 + `FADD` | missing | S4 (via atan2) |
| `b2f` | late algebraic: `b2f32(a)` becomes `bcsel_pan(a, 1.0, 0.0)` (`bifrost_nir_algebraic.py:115-129`, the four forms incl. `inot`) | `nir_op_bcsel_pan` (`:2967`) | `MUX.i32 .int_zero` (sources reordered: `bi_mux_to(dst, s2, s1, s0, ..)`) | missing | S4 |
| `bcsel` (float/int) | `pan_nir_lower_bool_to_bitsize` -> `bcsel_pan` (`:167-174`) | `nir_op_bcsel_pan` (`:2967-2970`) | `MUX.i32 .int_zero` | missing: `Bcsel` | S4 (S5 makes it the `if` result) |
| int compare `ieq ine ilt ige ult uge` | `*_pan` forms (`CASE_CMP`) | `nir_op_i*_pan` (`:3253-3265`) | `ICMP.s32/u32 .eq ..`, `BI_RESULT_TYPE_M1` | missing | S4 (`texType == 1`) |
| `f2i` (`int(frag[10].z)`) | none on arch 10 | `nir_op_f2i32` (`:3100-3115`) | `F32_TO_S32` (+ round) | missing | S4 |
| `i2f` | none for 32-bit on arch 10 | `nir_op_i2f32` (`:3193`) | `S32_TO_F32` | missing | only if a program needs it |
| `b2i` | none | `nir_op_b2i32` (`:3246-3251`) | `MUX` 0/1 | missing | only if needed |
| `discard` | `nir_lower_terminate_to_demote` (`B/bifrost_nir.c:411-416`); `discard_is_demote = true` (`bifrost_compile.h:151`); GLSL `discard` is `nir_discard_if(cond)` or `nir_discard()` (`glsl_to_nir.cpp:812-825`) | an *intrinsic*, not an ALU op: `demote_if` (`:1976-1986`) -> `DISCARD.b32`; `demote` -> `DISCARD.f32 0,0 .eq` (`:1989-1991`) | see below | missing | S6 |

Conventions that matter for byte equality:

- `fmul` is `FMA a, b, -0.0` (`:2873-2875`); `fadd` is `FADD` (`:2877`);
  `fneg`/`fabs` are `FABSNEG` pseudo-ops (`:2899-2905`) folded into users by
  `bi_opt_mod_prop_forward` or lowered to `FADD x, -0.0` (section 6). `emit.rs`
  already has these (`Fadd Fmul Ffma Frcp Fmax Fmin Fneg` 32-bit, `ROWS`
  `emit.rs:32-49`); S4 adds rows only when a reference draws them (the
  design's rule), one shim reference per op (item `shim-refs`).
- `fmul(a, 2.0)` becomes `fadd(a, a)` (`bifrost_nir_algebraic.py:30`).
- Comparison results are `-1`/`0` masks (`BI_RESULT_TYPE_M1`); `bcsel` tests
  the mask against zero (`MUX .int_zero`).
- `discard` fusing: `bi_opt_mod_prop_forward` rewrites
  `DISCARD.b32(FCMP.f32(x, y))` into `DISCARD.f32(x, y, cmpf)` when the compare
  is not `GTLT`-class (`B/bi_opt_mod_props.c:111-160`, call `:190-198`); a
  `DISCARD.b32` left over becomes `DISCARD.f32 x, 0 .ne`
  (`bi_lower_opt_instruction_helper`, `:501-518`). So `if (a < b) discard;`
  is **one** `DISCARD.f32 .lt` after the passes, not a `FCMP` + `DISCARD.b32`.
- `fdot2` reverse order and `lower_fdot` are the biggest bytes-risk: the
  chain is `fma(b.y*a.y .. )` starting from the last component; check it
  against `fill_color`'s `scissorMask`-free case first (it has no dot), so the
  first reference that exercises it is a gradient program.

Our mapping: `emit.rs` (the table and `emit`), the lowerings in `nir.rs`
(`late_rules :1010`). The tuple `(op, bit size)` of the design is
`Row { op: Alu, bits, width }` (`emit.rs:21`). `Alu` (`nir.rs:70`) needs new
variants: `Fsat Fmin..` (exists), `Ffloor`, `Frsq`, `Fcmp(cmpf)`, `Icmp(cmpf, signed)`,
`F2i32`, `Bcsel`.

### 5. `emit_cf_list`, `emit_if`, `emit_loop` (steps S5 for blocks, S6)

- **`emit_cf_list`** (`B/bifrost_compile.c:3772-3803`): for each node of the
  list: a NIR block -> `emit_block` (`:3667-3695`, creates or reuses
  `ctx->after_block`, emits instructions); an `if` -> `emit_if`; a loop ->
  `emit_loop`.
- **`emit_if`** (`:3697-3736`): in the current block, emit
  `BRANCHZ.i16 cond.h0, #0, .eq` (`bi_branchz_i16(.., bi_half(cond, false),
  bi_zero(), BI_CMPF_EQ)`, `:3703-3705`), emit the then list, the else list
  (fall-through order: then first), create the after block; patch the branch
  target to the *else* block; append `JUMP` (to after) at the end of the then
  block; successors: `before -> {else, then}`, `end_then -> after`,
  `end_else -> after`. The condition is a 16-bit-sized mask read through its
  low half.
- **`emit_loop`** (`:3738-3770`): `continue_block` (the header, `loop_header
  = true`) and `break_block` are created up front; body emitted; `JUMP` back
  to the header; `break`/`continue` are `JUMP` to those blocks
  (`bi_emit_jump :85-100`, `unconditional_jumps` set).
- **Phis**: `bi_emit_phi` / `bi_emit_phis_deferred` (`:3577-3625`, call
  `:4429`); a phi becomes `PHI` instructions resolved by the register
  allocator (`bi_ra*.c`, SSA based on arch >= 9), not moves in predecessors.
- **Our side.** `bir.rs`'s `Prog` is one flat instruction list
  (`bir.rs:590`, `MAX_INSTRS = 192`). S5 needs a block structure (list of
  blocks with successors) and `Op::Branchz`/`Op::Jump`/`Op::Phi`; then
  `ra.rs` (liveness `live_update :111`, LCRA `Lcra :219`,
  `allocate :328`), `sched.rs` (`pressure_schedule :166` is per whole
  program: must become per block, as Mesa's `bi_pressure_schedule.c` is) and
  `flow.rs` (`assign_slots :81`, `insert_flow_control_nops :135`,
  `merge_flow :245`, `mark_last :359`, `analyze_helper_requirements :39`,
  `preload_mask :399`) must work across blocks. `flow.rs`'s scoreboard state
  must be reset or merged at block boundaries.
- **Needed by.** S5 (any `if` that peephole does not flatten: none of
  femtovg's, if the counts fit 64) and S6 (every `if (..) discard;`, since
  discard is never flattened, §1). **Decision for the S5 author**: if every
  non-discard femtovg `if` flattens, real blocks are needed first by S6 and
  S5's PR may stop at "peephole"; the design says "then real blocks", so
  S5 builds blocks and proves them with a hand-written shader that has an
  `if` that cannot flatten (e.g. one containing a texture look-up, which
  `block_check_for_allowed_instrs` refuses, `N/nir_opt_peephole_select.c:200-240`).
- **Done (S5, 2026-10-22).** The scissor `if`s do not flatten (§1), so the
  blocks were needed for femtovg itself. `bir.rs` holds a block table
  (`MAX_BLOCKS` 32, four predecessors, `Op::Branchz`, `Op::Phi`;
  `emit_if`'s order: `BRANCHZ.eq` to the else side, the then side, the
  jump as `BRANCHZ` on a zero constant, `bi_lower_branch`; the else side;
  `bi_opt_control_flow` folds a branch to the next block). `sched.rs`
  schedules a block at a time with a phi-aware SSA liveness, `ra.rs`
  takes the phis out of SSA (a register per phi, `IADD_IMM` for a
  constant source) and allocates with the interference of each block from
  its live-out, `flow.rs` walks the blocks in layout order for the
  scoreboard, `RECONVERGE`, `END` and the last-use marks. The board
  record `0dce654a8:tools/mali/board-ref/2026-10-22-s5-if/` shows the first 40 words
  of `fill_color.nodiscard` byte-equal to the board's Mesa 25.0.7 (branches
  and offsets included) once the FAU operands are moved past the blend
  constants.

### 6. `bi_opt_mod_prop_forward`, `bi_lower_opt_instructions`, `va_fuse_add_imm` (all steps; already in `opt.rs`)

| Mesa | What it does | Ours |
|---|---|---|
| `bi_opt_mod_prop_forward` (`B/bi_opt_mod_props.c:184-225`) | in program order, a source defined by `FABSNEG` of the user's size takes the `FABSNEG`'s source with `.abs/.neg` composed when `bi_takes_fabs`/`bi_takes_fneg` allow (`:23`, `:50`); first fuses `DISCARD.b32(FCMP)` (`:190-198`, `bi_fuse_discard_fcmp :111`) and small-int to f32 conversions (`bi_fuse_small_int_to_f32 :163`) | `opt.rs:107` `mod_prop_forward` (fabs/neg part only). **S4 adds** the small-int conversion (`int(x)` of a compare) if a program shows it; **S6 adds** the discard fusion |
| `bi_opt_mod_prop_backward` (same file) | folds `FCLAMP` into the producer's `.clamp` (`bi_optimizer_clamp :259`, `bi_opt_mod_prop_backward :407`) | **absent**; needed by S4 as soon as `fsat` appears (every `clamp(x,0,1)` in femtovg) |
| `bi_lower_opt_instructions` (`:521-529`, helper `:484-518`) | `FABSNEG`/`FCLAMP` left over become `FADD x, -0.0` (+clamp); `DISCARD.b32` left over becomes `DISCARD.f32 x, 0 .ne` | `opt.rs:133` `lower_opt_instructions` (FABSNEG only). **S4** adds `FCLAMP`; **S6** adds `DISCARD.b32` |
| `va_fuse_add_imm` (`B/valhall/va_optimize.c:67-110`, driven by `va_optimize :391`) | `FADD/IADD` with a constant source and an unmodified other source becomes `FADD_IMM`/`IADD_IMM`; a `MOV.i32 #c` becomes `IADD_IMM 0, #c` (`va_lower_mov_imm :55-64`); `.neg` flips the sign bit of the immediate | `opt.rs:150` `fuse_add_imm` (FADD only). Integer forms are S4's if the int compare path shows `IADD` |
| `va_optimize_forward` (`:350`, `va_fuse_cmp :220`) | on Valhall fuses a compare with its `bcsel`/`MUX` user into `CSEL` | **absent**; the byte-equal check on the first `bcsel` program (S4/S5) decides if it is needed |
| `bi_opt_copy_prop`, `bi_opt_constant_fold`, `bi_opt_cse`, `bi_opt_dce`, `bi_opt_reorder_push` | between `push_ubo` and `lower_opt` | `opt.rs:184,229` `cse`, `dce`; constant fold and copy-prop are not in `opt.rs` |

### 7. `nir_lower_alu_width` / `nir_opt_vectorize` for fp16 pairs (not needed by S2-S6)

- `bi_vectorize_filter` (`B/bifrost_nir.c:106-181`) tells both passes the
  width: phis 2 for 16-bit (`:109-117`); non-ALU 0 (leave alone); boolean-reduce
  ops 1; `frcp frsq ishl ishr ushr extract_* insert_u16` stay scalar (`:146-153`);
  `f2f16 u2f16 i2f16` 2 on arch < 11 (`:154-160`); everything else
  `max(1, 32 / bit_size)` of the larger of dest and src0 (`:176-179`), bool 1.
  So fp32 ALU is width 1 (scalarised) and fp16 is width 2 (paired).
- `nir_lower_alu_width` (`N/nir_lower_alu_width.c:203-435`, entry `:437`) splits
  wider-than-target ops, also the dot products (`lower_fdot`, above);
  `nir_opt_vectorize` (`N/nir_opt_vectorize.c:689-722`) merges two equal
  scalar ops into one vec2 op where the callback allows; `vectorize_vec2_16bit
  = true` in the options.
- **Not needed.** Every femtovg fragment shader is `precision highp float`, so
  the fp16 pairing (what `nir_fs.rs::vectorise` :846 does today for
  mediump) is untouched by S2-S6; precision per value is the "later shared
  item" of the design. What S2-S6 **do** need from these passes is the
  scalarisation side: `lower_alu_width` with target width 1 for fp32, the
  `lower_fdot` expansion and `nir_lower_phis_to_scalar` (`:1090`) for the
  `if` phis of S5. In the graph "one value per lane" form of S1 this is
  already the representation; the work is `lower_fdot` and the phi handling.

### 8. Steps against passes (summary)

| Step | Mesa passes to transcribe | Our files |
|---|---|---|
| S2 highp fp32 fragment arithmetic | fp32 rows `Fadd Fmul Ffma` (exist), width-1 `lower_alu_width`, `fneg/fabs` forwarding, `lower_opt_instructions` | `emit.rs`, `opt.rs`, `nir.rs`; the fragment varying read (`nir_fs.rs:536 index`) at 32 bits; check mediump regression populations stay byte-equal |
| S3 uniform block to FAU | `bi_opt_push_ubo`, `reorder_push`, `lower_uniforms_to_ubo` | `fau.rs`, `nir.rs` (`Kind` load of the block, `uniform_of :314`), `mod.rs` refusals `:280-286` |
| S4 ALU rows | section 4 table (all ops without `if`/`discard`), `fsat`/`FCLAMP` backward propagation, `lower_fdot`, `lower_fdiv`, `lower_fsqrt`, `lower_ffract`, `lower_flrp`, `b2f` -> `bcsel_pan` | `emit.rs ROWS`, `nir.rs late_rules`, `opt.rs` |
| S5 `if` | `peephole_select` (+`collapse_if`), `emit_if`, phis, block-aware `ra.rs`/`sched.rs`/`flow.rs` | `nir.rs`, `bir.rs`, `ra.rs`, `sched.rs`, `flow.rs` |
| S6 discard, loop | `demote_if` -> `DISCARD`, `bi_fuse_discard_fcmp`, `loop_unroll` (complex_unroll), helper-invocation analysis | `emit.rs`, `opt.rs`, `nir.rs`, `flow.rs:39 analyze_helper_requirements` |

## Part 2. femtovg and fvgtest programs

42 programs: 35 `femtovg/*.frag`, `femtovg/main.vert`, six
`tools/shaders/fvgtest_*.frag`. All 41 fragment programs declare
`precision highp float`; the femtovg ones are one source
(`fill_color.nodiscard.noscissor.frag` and its siblings are the same file
under different `#define`s, `femtovg/PROVENANCE.md`).

**How the features were counted.** By reading the shared source and its
`#define` combinations (`SELECT_SHADER`, `ENABLE_GLYPH_TEXTURE`,
`NO_DISCARD`, `NO_SCISSOR`); `cpp` was not runnable on the worker, so the
counts are from the source, not from a preprocessor run (re-count with
`cpp -P` before relying on a count of 1). "`if` live" is the number of `if`s
left after dead code is removed (`nir_opt_dce` + `nir_opt_dead_cf`,
`bifrost_nir.c:244`): `scissorMask`'s `if` is dead whenever the result is not
multiplied in (stencil without glyph; filter_image; texture copy, which
returns before it); "(src)" is the source count. The design text's "up to 6
`if`s" is reproduced only for `filter_image.frag` (discard, scissor, 2
`texType`, the `break` and the `for` condition's break); my source count is 5
for the image/glyph variants and 4 live for `filter_image.frag`. The design's
"40 programs read `frag[]`" is 39 here (35 femtovg + `fvgtest_cover`,
`_discard`, `_discard2`, `_inner`); the 40th is not in this tree.

Features per shader type (names in the "Ops" column):

- **stroke** (all but texture copy): `fabs fneg fmul fadd fmin fmin`, varying
  `ftcoord`, `frag[10].x`.
- **scissor** (`!NO_SCISSOR`): `frag[0..2]` (mat3), `frag[8]`, `frag[12].w`;
  `flt` for `scissorRadius > 0`, `if`, `fabs fmax fmin fdot2 fsqrt fsat fneg`
  and `ffma`/`fmul` for the mat3 product.
- **discard** (`!NO_DISCARD`): `frag[10].y`, `flt`, `discard`.
- **gradient** (`SELECT 0`): `frag[3..5,6,7,9]`, `frcp` (the `/ feather`), `fsat`,
  `mix` -> `ffma`, the dither: **`gl_FragCoord`**, `fdot2`, `ffract` x2.
- **image gradient** (`3`): as gradient, `mix` replaced by `texture2D(tex, vec2(d, 0))`.
- **conic** (`8`): `atan(y, x)` (expanded by `builtin_call.rs` from
  `nir_atan2`: `fabs fmax fmin frcp ffma flt bcsel fneg`, `fsign`/`feq` forms
  possible), `ffract`, `frag[13].x`, dither (`gl_FragCoord`, `ffract`), `mix`.
- **image conic** (`9`): conic with the texture instead of `mix`.
- **image** (`1`): `frag[3..5,6,9]`, `frcp` (`/ extent`), `texture2D`, `f2i32` +
  `ieq` (`int(frag[10].z)`), 2 `if`s (`texType`), `fmul`.
- **texture copy** (`6`): `texture2D(tex, ftcoord)`, 2 `texType` `if`s, no
  stroke, no discard.
- **filter** (`4`): `fceil`, `frcp`, `for` x24 with `break`, 1 + 48
  `texture2D`, 2 `texType` `if`s.
- **glyph** (`ENABLE_GLYPH_TEXTURE`): a second `texture2D(glyphtex, ftcoord)`,
  1 `if` on `int(frag[11].x)`, `fmul` by `scissor`.
- **stencil** (`2`): constant `vec4(1)`.
- **color** (`5`): `frag[6]`, `fmul`.

Step key: S2 = highp arithmetic, S3 = fragment uniform block to FAU, S4 = ALU
rows, S5 = `if`, S6 = `discard` and the loop. **Tex** = the texture slice
(the issue #1385 items and the "Tex row" of #1384; today's refusals are one
look-up per fragment shader, a coordinate that is a varying read whole, a
result read in its own pairs; `mod.rs:306-321`), which is independent of S2-S6
and not scheduled by the design; a texture program is `ok` at the *later* of
its S-step and Tex. `\*` = needs `gl_FragCoord` (`mod.rs:326`, "a built-in
input ... is not lowered yet"): **not in the design's op list**, see the
report. Mesa side: `nir_lower_frag_coord_to_pixel_coord` (`B/bifrost_nir.c:910`)
then `load_pixel_coord` (`bifrost_compile.c:1945`) = `MOV.i32` of the preloaded
`BI_PRELOAD_POSITION_XY`; the register preload is not in `preload.rs` (that is
the framebuffer-preload programs), see `mali-highp-fragcoord.md` 2.2.

Verdict key (from `tools/mali/corpus-verdicts.txt`): **arith** = "Valhall:
arithmetic other than +, -, * and a dot or matrix product (in a fragment
shader: +, -, * and abs) is not lowered yet" (it is the *first* refusal met, so
the later refusals of the same program are not visible yet); **uniform** =
"a fragment shader uniform other than one mediump vec4 read in its own lane
pairs"; **ok**.

Common to every row: highp = yes. Columns: `frag[]` words read; `if` live
(src); loop; discard; ops beyond `+ - *`; tex = look-ups; `ok at`.

### femtovg (36 rows)

| Program | Verdict now | `frag[]` | `if` live (src) | loop | discard | ops | tex | ok at |
|---|---|---|---|---|---|---|---|---|
| `main.vert` | ok | no | 0 | - | - | (vertex stage; `viewSize` uniform) | 0 | ok (regression guard) |
| `fill_color.nodiscard.noscissor` | arith | 6, 10 | 0 (0) | - | - | stroke | 0 | **S4** (28 IR instrs; the S4 acceptance program) |
| `fill_color.nodiscard` | arith | 0-2,6,8,10,12 | 1 (1) | - | - | stroke, scissor | 0 | **S5** (if peephole fits 64) |
| `fill_color` | arith | + 10.y | 2 (2) | - | yes | stroke, scissor, `flt` discard | 0 | **S6** |
| `fill_color.glyph.nodiscard` | arith | + 11.x | 2 (2) | - | - | + glyph `if` | 1 | S5 + Tex |
| `fill_color.glyph` | arith | + 10.y, 11.x | 3 (3) | - | yes | | 1 | S6 + Tex |
| `stencil.nodiscard` | arith | 10.x | 0 (1, dead) | - | - | none live (const output) | 0 | **S5** (needs the dead `if` dropped; S4 if dead-CF removal is in S2-S4) |
| `stencil` | arith | 10.x, 10.y | 1 (2) | - | yes | `flt`, discard | 0 | **S6** |
| `fill_gradient.nodiscard.noscissor` | arith | 3-7,9,10.x | 0 (0) | - | - | stroke, gradient (`fabs fmax fmin fdot2 fsqrt frcp fsat ffma ffract`) | 0 | **S4\*** (gl_FragCoord) |
| `fill_gradient.nodiscard` | arith | + scissor | 1 (1) | - | - | | 0 | S5\* |
| `fill_gradient` | arith | + 10.y | 2 (2) | - | yes | | 0 | S6\* |
| `fill_gradient.glyph.nodiscard` | arith | + 11.x | 2 (2) | - | - | | 1 | S5\* + Tex |
| `fill_gradient.glyph` | arith | + 10.y | 3 (3) | - | yes | | 1 | S6\* + Tex |
| `fill_gradient_conic.nodiscard.noscissor` | arith | 3-5,6,7,10.x,13.x | 0 (0) | - | - | stroke, atan2 (`flt bcsel frcp ffma fabs fmax fmin fneg`), `ffract` x2, `mix` | 0 | **S4\*** |
| `fill_gradient_conic.nodiscard` | arith | + scissor | 1 (1) | - | - | | 0 | S5\* |
| `fill_gradient_conic` | arith | + 10.y | 2 (2) | - | yes | | 0 | S6\* |
| `fill_gradient_conic.glyph.nodiscard` | arith | + 11.x | 2 (2) | - | - | | 1 | S5\* + Tex |
| `fill_gradient_conic.glyph` | arith | + 10.y | 3 (3) | - | yes | | 1 | S6\* + Tex |
| `fill_image.nodiscard.noscissor` | arith | 3-6,9,10 | 2 (2) | - | - | `frcp`, `f2i32`, `ieq` x2, `bcsel`/`if` (texType) | 1 | S5 + Tex |
| `fill_image.nodiscard` | arith | + scissor | 3 (3) | - | - | | 1 | S5 + Tex |
| `fill_image` | arith | + 10.y | 4 (4) | - | yes | | 1 | S6 + Tex |
| `fill_image.glyph.nodiscard` | arith | + 11.x | 4 (4) | - | - | | 2 | S5 + Tex (two look-ups) |
| `fill_image.glyph` | arith | + 10.y | 5 (5) | - | yes | | 2 | S6 + Tex (two look-ups) |
| `fill_image_gradient.nodiscard.noscissor` | arith | 3-5,9,10.x | 0 (0) | - | - | gradient ops, `vec2(d,0)` coordinate | 1 | **S4\*** + Tex (coordinate form, #1385 case 122) |
| `fill_image_gradient.nodiscard` | arith | + scissor | 1 (1) | - | - | | 1 | S5\* + Tex |
| `fill_image_gradient` | arith | + 10.y | 2 (2) | - | yes | | 1 | S6\* + Tex |
| `fill_image_gradient.glyph.nodiscard` | arith | + 11.x | 2 (2) | - | - | | 2 | S5\* + Tex |
| `fill_image_gradient.glyph` | arith | + 10.y | 3 (3) | - | yes | | 2 | S6\* + Tex |
| `fill_image_gradient_conic.nodiscard.noscissor` | arith | 3-5,10.x,13.x | 0 (0) | - | - | conic ops, `vec2(d,0)` | 1 | **S4\*** + Tex |
| `fill_image_gradient_conic.nodiscard` | arith | + scissor | 1 (1) | - | - | | 1 | S5\* + Tex |
| `fill_image_gradient_conic` | arith | + 10.y | 2 (2) | - | yes | | 1 | S6\* + Tex |
| `fill_image_gradient_conic.glyph.nodiscard` | arith | + 11.x | 2 (2) | - | - | | 2 | S5\* + Tex |
| `fill_image_gradient_conic.glyph` | arith | + 10.y | 3 (3) | - | yes | | 2 | S6\* + Tex |
| `texture_copy_unclipped` | arith | 6, 10.z | 2 (3, one dead) | - | - | `f2i32`, `ieq` x2, `fmul` | 1 | S5 + Tex (needs the dead scissor `if` dropped) |
| `filter_image.nodiscard` | arith | 9.xy, 10.z, 11.yzw, 12.xyz | 3 + break (3 + break) | `for` 24, two terminators | - | `fceil`, `frcp`, `fmul`, `f2i32`, `ieq` | 1 + 48 | **S6** (unroll) + Tex (49 look-ups; arena caps) |
| `filter_image` | arith | + 10.y | 4 + break | as above | yes | | 1 + 48 | S6 + Tex |

### fvgtest (6 rows)

| Program | Verdict now | `frag[]` | `if` live | loop | discard | ops | tex | ok at |
|---|---|---|---|---|---|---|---|---|
| `fvgtest_green` | ok | no | 0 | - | - | constant output | 0 | ok (S2 regression guard) |
| `fvgtest_ftcoord` | ok | no | 0 | - | - | varying copy (`ftcoord`) | 0 | ok (S2 regression guard, named in the design) |
| `fvgtest_inner` | uniform | 6 | 0 | - | - | none | 0 | **S3** (case 30) |
| `fvgtest_cover` | arith | 0-2,8,10.x,12.w | 1 (scissor) | - | - | stroke, scissor (`flt fabs fmax fmin fdot2 fsqrt fsat fneg ffma`) | 0 | **S5** (peephole fits 64 is unmeasured; the shader's own purpose is to cover those ops, so S4's per-op references can reuse it) |
| `fvgtest_discard` | arith | 0-2,6,8,10.xy,12.w | 2 | - | yes | stroke, scissor, `flt` discard | 0 | **S6** |
| `fvgtest_discard2` | arith | 6 | 1 (the discard) | - | yes | `flt` (`ftcoord.y < -100.0`), discard | 0 | **S6** (smallest discard program: use it for the first `DISCARD.f32` reference) |

### What the table says about the order

- After S2: only `fvgtest_green`/`ftcoord` (already ok). No femtovg program
  can pass before S4, because even `fill_color.nodiscard.noscissor` multiplies
  by `strokeAlpha` (`fmin`, `fabs`).
- After S3: `fvgtest_inner`.
- After S4: `fill_color.nodiscard.noscissor` only is certain. The three
  non-texture `*.nodiscard.noscissor` gradient/conic programs wait on
  `gl_FragCoord` as well (see report); the image ones on Tex.
- After S5: the `.nodiscard` family without texture (`fill_color`, `gradient`,
  `conic`, `stencil`), `fvgtest_cover`, and, with Tex, the image ones.
- After S6: everything with `discard`, plus `filter_image*`.

## Gaps this note found in the design

1. **`gl_FragCoord`** is used by every gradient/conic shader (the dither,
   `ditherGradient(gl_FragCoord.xy)`) and is refused today (`mod.rs:326`).
   The design's op list does not name it. It is a preload read
   (`BI_PRELOAD_POSITION_XY`, `bifrost_compile.c:1945`) plus an integer to
   float conversion (`nir_lower_frag_coord_to_pixel_coord` yields `u2f32`
   only, of the 16-bit lanes; there is no `i2f32`), so S4 needs a
   16-bit-source `U2f32` row (`U16_TO_F32`; the row needs a source size).
   The register preload needs no new entry: `preload.rs` builds the
   framebuffer preload programs, not register preload, and `select.rs`
   already makes the `MOV.i32` from `Idx::Reg(59)`. Corrected per
   `mali-highp-fragcoord.md` (sections 1 step 4, 2.1, 2.2 and "Open points"
   8). Without it 20 of the 35 femtovg
   fragment programs (all gradient and conic variants, with and without
   the texture) stay refused after S6.
2. **`ceil`** (`fceil`, `filter_image`) and `fround`-family rows are not in
   the design's list; `ffract` implies `ffloor`.
3. **`fsub`** is not a row: it is `fadd(a, fneg(b))` (algebraic), fine.
4. **Cost of the `filter_image` unroll: Mesa unrolls it, in both variants**
   (`fvg_filter_image` with the `discard`, `fvg_filter_image_nodiscard`). The
   test is `check_unrolling_restrictions`
   (`N/nir_opt_loop_unroll.c:899-931`): unroll if `instr_cost * trip_count <=
   max_unroll_iterations * LOOP_UNROLL_LIMIT` (`:924-928`; 32 * 26 = 832, `:36`
   and `B/bifrost_compile.h:140`) and `trip_count <= 32`. `trip_count` is 24
   (`i <= 24.0`, step 1.0). The `instr_cost` is the NIR sum of
   `instr_cost` (`N/nir_loop_analyze.c:79-135`, summed at `:1477`): 1 per ALU
   op, intrinsic and `tex`, 0 for `mov`/`vec`, for a constant-after-unrolling
   op and for an `fmul` only used by an `fadd`; 832 / 24 leaves 34 per
   iteration. Two terminators (`i <= 24`, `i >= sampleCount`) take
   `complex_unroll` (`:1125-1149`). Mesa's decision is **unrolled**, shown by
   the shim driver's bytes, which run Mesa's whole GL path
   (`tools/mali/testdata/fvg_filter_image.expect.json` and
   `fvg_filter_image_nodiscard.expect.json`, region `fs.fragment`; read with
   a count of the `text` lines):

   | variant | `fs.fragment` instructions | `TEX_SINGLE` | `BRANCHZ` | back-edge |
   |---|---|---|---|---|
   | `filter_image` | 949 | 49 (1 + 24 x 2) | 48 (24 break guards + 24 one-word skips) | none (all offsets forward) |
   | `filter_image.nodiscard` | 943 | 49 | 48 | none |

   The only difference is the one `DISCARD.f32.lt` of the stroke test. Every
   `BRANCHZ.eq.reconverge` has a forward offset (the guards step 33-38 words
   over a copy of the body); there is no `BRANCHZ`/`JUMP` that goes back, so
   no loop remains. The emitted count per iteration (33-38 words between guards, about 35) is
   not the NIR `instr_cost` (which counts `mov`s and fused `fmul`s as 0), so
   it does not bound it; I did not compute `instr_cost` itself, the decision
   rests on the bytes. `pan_shc` confirms it. Run with
   `PAN_SHC=$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1/build-pan/src/panfrost/pan_shc/pan_shc
   tools/mali/glsl2pan.py fvg_filter_image.vert fvg_filter_image.frag`
   (and the `_nodiscard` pair), the stderr disassembly has 49 `TEX_SINGLE`,
   48 `BRANCHZ.eq.reconverge` plus the blend `BRANCHZI` (49 matches of
   `BRANCHZ`), no `offset:-` (no backward branch), and one `DISCARD` in
   `filter_image` and none in the nodiscard variant (`grep -c`). Its
   SPIR-V front end gives other offsets (910 for the first guard against 888)
   and so other word counts; the unroll is the same. The shim reference and
   `pan_shc` share the compiler, so the unrolled result holds
   for both. **S6 therefore needs the unroll (`complex_unroll`) and not
   `emit_loop`**, and the 24 guards are the `if`s of S5 with a `reconverge`
   branch each.
5. **Arena caps** `bir.rs:26-28` (`MAX_INSTRS = 192`, `MAX_NODES = 192`) do
   not hold the unrolled filter. Count: the Mesa reference has 949 (943
   without the discard) final instructions, of which 309 are `MOV` (the
   final `MOV`s include moves the register allocator and the vector
   packing add, so they need not exist in the BIR before allocation). Without any `MOV` it is still 640
   (634): 467 `FMA`, 49 `TEX_SINGLE`, 48 `BRANCHZ`, 24 `FCMP_OR`, 21
   `IADD_IMM`, 9 `FADD`, 3 `FRCP`, 2 `FMIN`, 2 `ICMP_OR`, 2 `MKVEC`, 6
   `CSEL`, 1 `FROUND`, 1 `F32_TO_S32`, 1 `DISCARD`, 1 `ATEST`, 1 `BLEND`,
   1 `BRANCHZI`, 1 `LD_VAR_BUF_IMM`. 949 / 192 = 4.9; 640 / 192 = 3.3, so the
   unrolled body is between 3.3 and 5 times `MAX_INSTRS` however the `MOV`s
   are counted. Per iteration about 35 words x 24 = 840, plus about 100 of
   prologue and epilogue, is 949. `MAX_NODES` is the same order (almost every
   instruction defines a value). S6 raises both to at least 1024 and checks
   what `preload.rs:53` (`alloc_filled(MAX_INSTRS, Ins::NOP)`) and the
   scratch arena then cost, or builds the body per `if` region. The BIR
   count cannot be taken from the reference exactly; S6 sets the number
   from the first byte-equal run.
6. **Two peephole calls** (limit 8 in the GL front end, 64 in bifrost): a
   `pan_shc` reference already includes the first; our graph runs both in
   that order or the byte-equal check fails near the counts.
7. **Texture is unscheduled** in the S2-S6 chain although 24 programs
   need it; #1385's "Tex row" lands where the plan puts it.
8. **The link's code motion** (found by S4, 2026-10-21). The GL linker runs
   `nir_opt_varyings` on every program (`gl_nir_lower_optimize_varyings`,
   gl_nir_linker.c:1477-1525), whose backward inter-shader code motion
   (nir_opt_varyings.c:4497-4807) moves a fragment shader's arithmetic on
   a varying that commutes with interpolation (`fadd`, `fneg`, `fmul`/`ffma`
   by a convergent value, `can_move_alu_across_interp` :3734-3793) into the
   vertex shader, as far as the farthest movable use post-dominator
   (nir_use_dominance.c). femtovg's `strokeMask` term `ftcoord.x * 2.0 -
   1.0` is computed by the vertex shader of every program that reads it
   (fvg_fill_color_nodiscard_noscissor.expect.json's varying program holds
   `FMA.f32 r0, r0, 2.0, -1.0`; fill_gradient, fill_image, fvgtest_cover
   the same). So a program's vertex shader depends on the fragment
   shader's code, not only on its reads: `motion.rs` finds the moves on
   the fragment graph (highp shaders only: a mediump shader's inputs are
   converted first, which stops every chain), the fragment record carries
   them, and the vertex shader is built for them (`compile_vs_for_reads`,
   the backend's `FsReads` key). Two facts the references pin beyond the
   pass's text: a uniform is not convergent for these programs (the
   gradient's `paintMat * vec3(fpos, 1.0)` stays in the fragment shader),
   and a post-dominator whose only users are the colour's stores is not
   moved (s4_fneg's `-v.x`); the rule as read gives no such condition, and
   the next item that meets a chain the measured rule does not cover
   (a lane stored directly after an affine chain, a chain over two
   varyings) must take a shim reference first. Two more facts, measured
   on pan_shc (2026-10-22, tools/mali/testdata/pan_shc-out/s4_motion_*):
   a dot product is never moved and keeps every lane of its varying (the
   linker sees one vector-sourced `fdot2`, which the rule refuses and
   whose source is the whole vector; the back end lowers it to the `fmul`
   and `ffma` chain only afterwards, so the lowered chain in `nir_fs.rs`
   is marked unmovable by its IR origin and the varying read whole), and
   `(v.x + v.y) * v.z` moves the sum once into `v.x`'s lane with `v.y` no
   longer read (the loads a moved post-dominator post-dominated are dead,
   :4440-4494: the fragment graph counts only the users the colour still
   reaches). The record's moved-expression encoding numbers `fadd`,
   `fmul`, `fneg`, `ffma` (motion.rs `OPS`, an operation only appended);
   no ES 1.00 source makes a moved `ffma` today (the linker runs before
   the late fusion), so `OPS`' `ffma` is pinned by motion.rs's own
   round-trip tests, not by a program.
