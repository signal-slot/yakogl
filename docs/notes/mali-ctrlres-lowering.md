<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the `ControlFlow` and `DiscardForm` residue at `349f989f`: steps, Mesa references and order

**HOST-ONLY design** (plan #387, item `ctrlres-refs`; issues #7, #20).
Nothing under `src/` changed. Every dEQP-GLES2 case whose first refusal at the
integration tip **`349f989f5392`** (2026-11-28) is `MaliError::ControlFlow`
or `MaliError::DiscardForm` is put in exactly one step row below. The new
references are `tools/mali/testdata/deqp_ctrlres_*` (15 pairs), pinned by
`tests/mali_deqp_ctrlres.rs`. The counts are first refusals of the host
measure, never render verdicts. The G310 needs no separate work
([`mali-g310-vs-g610.md`](mali-g310-vs-g610.md)).

The steps are those of [`mali-deqp-ctrlflow.md`](mali-deqp-ctrlflow.md)
section 5 (CT1 steps 1-6). Step 1 is **cf-dead**, which item `cfdead-refs`
designs (`mali-cfdead-lowering.md`, in review, measured at `297d118c`). This
note does not redo it: it checks that cf-dead's 52 cases are still the same
at this tip. One new step, **CR1**, takes the fragment `if`s nested under a
run-time condition. The ctrlflow note's step 3 did not mention them.

## 1. How this was measured

- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out "$PWD/build/measure" --jobs 3
  --chunk 100` over the 10,597 board-refused cases of the slices
  `2026-11-15-t6-gles2-S{1..4}-37305608`, the cases of
  `mali-deqp-families.md` sections 18 and 19. VK-GL-CTS `93bca018` was
  fetched at that one commit, `external/fetch_sources.py` was run and
  `tools/cts/cts-yakogl-target.patch` was applied. All 10,597 cases began,
  106 chunk processes exited 1 and none crashed. **4,709** cases meet a host
  refusal. The first refusal is **`ControlFlow` for 102** (33 vs / 69 fs)
  and **`DiscardForm` for 20** (all fs). These are the counts of section
  19.1 at `e9033663` and of `cfdead-refs` at `297d118c`.
- **Tip caveat:** at `349f989f`, `capi/Cargo.toml` defines the feature
  `vivante-st` twice (lines 41 and 85, added by `75a7d901` and again by
  `110080eb`). Cargo refuses the workspace manifest ("duplicate key"),
  so no `cargo` command runs at the tip as it is. For this measure and for
  the test run, line 41 was removed in the working tree and **not
  committed**, so the measure's `commit` file says `349f989f-dirty`. The
  line is a feature-forward entry. It changes no compiler code, so the
  outcomes are those of `349f989f`.
- **Sites:** a scratch copy of the tip (a `git worktree` in `$TMPDIR`, not
  committed) printed `file:line` at each of the 31 `MaliError::ControlFlow`
  / `DiscardForm` constructions in `nir.rs`, `nir_fs.rs` and `vec16.rs`.
  Each case's one program (`programs/` of the measure) was compiled with
  `mali_ref::compile_pair` in a scratch test. All 122 reproduce their
  measured text, and **each fires exactly one site**. Line numbers are those
  of `349f989f`. They are one line off `mali-cfdead-lowering.md`'s for
  `nir.rs` (that note's `297d118c` `:4947` is `:4948` here) and drift with
  the files.
- **Mesa's class:** each of the 122 pairs was drawn on the drm-shim
  (`tools/mali/deqp_ref.sh --instrument shim`, not committed). The refusing
  stage's final disassembly (`vs.position` + `vs.varying`, or
  `fs.fragment`) is classed as `kept-branch` (a `BRANCH*` other than the
  blend tail's `BRANCHZI`), `discard`, `select` (`CSEL`/`MUX`) or `gone`.
  This is the rule of `mali-cfdead-lowering.md` section 3.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_ctrlres_<name> --instrument shim`, on the shim
  that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` (sha256 of `third_party/mesa-26.2.1/UPSTREAM`
  checked) into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's self-check
  (`gm2_cf_ifelse_vs`, `gm2_cf_ifelse_fs`) and a redraw of
  `deqp_ctrlflow_fs_vec_cond` were byte-equal. Each new reference is
  byte-equal to its first draw in the class pass. `.nir.sha` is from
  `tools/mali/nir_refs.sh 'deqp_ctrlres_*'`, and its rerun on
  `deqp_ctrlflow_fs_vec_cond` left that file unchanged. Each pair is
  verbatim from the measure's `programs/` (the `.qpa` text).
- **Coverage:** a case is "covered" when its pair, or its refusing stage's
  source, is byte-equal to a pinned reference's (SHA-256 against every
  `tools/mali/testdata` pair and `cfdead-refs`' `deqp_cfdead_*`), or when
  Mesa's words for it equal a reference's in every region (the discard
  wrappers, the `if_elseif_else` siblings). A reference was drawn for each
  shape with neither.

## 2. The per-step table at `349f989f`

"Cases" are first-refusal must-pass cases (Fail / NotSupported by the
board's status at `37305608`). For every case outside cf-dead, the measure's
`next_family` (the other stage's refusal) is empty, except
`random.conditionals.combined.66` (step 4, next `367:ARITH`). A later
refusal in the same stage cannot be seen before the step lands.

| Step | Mechanism (one line) | Cases (CF / DF; vs / fs) | Board | Top dEQP groups | Sites (`349f989f`) | Mesa | References to make byte-equal | Depends on |
|---|---|--:|---|---|---|---|---|---|
| **1 cf-dead** (`cfdead-refs`, not this item) | fold constant conditions, drop `if`s whose writes are dead (CD-a/CD-b) | **52** (47 / 5; 23 / 29) | 51 F, 1 NS | random.conditionals 31, fragment_ops.interaction 8, discard 5, linkage 4, random.all_features 3, operator.sequence 1 | fs `:5578` 12, `:5603` 3, `:5619` 7, `:5654` 2, `:2423` 5; vs `nir.rs:4969` 10, `:4979` 5, `:5223` 1, `:5244` 4, `:6220` 3 | gone | `deqp_cfdead_*` and the pinned ones of `mali-cfdead-lowering.md` 2 | `mesa_fold.rs` (the one folder) |
| **2 disc-forms** | `discard;` outside an `if` (`DISCARD.f32.eq 0x0, 0x0`), a discard under an integer compare (`ICMP_OR.s32` then `DISCARD.f32.ne`), one under a compare of a computed fp16 value (the `fadd` folded into the compare) | **12** (0 / 12; 0 / 12) | 12 F | discard 12 (`{basic,function,static_loop,function_static_loop}_{always,uniform,dynamic}`) | `:8616` 2 (always), `:2423` 2 (static-loop always), `:2425` 4 (uniform), `:11494` 4 (dynamic) | discard | `deqp_ctrlflow_disc_{basic_always,function_static_loop_always,basic_uniform,static_loop_uniform,basic_dynamic,function_dynamic}` (the other six cases have the words of these) | cf-dead first (`:2423`'s constant `if (false)` is CD-a's); the integer compare form of intval-rest step 1 (landed) |
| **3 fs-select32** | a mediump fragment `if` whose condition is an integer or vector compare, or whose sides read varyings, an int uniform or build booleans: `CSEL.u32.eq` of the 32-bit sources before the fp16 conversion, `MUX.v2i16` on a widened 16-bit mask | **11** (11 / 0; 0 / 11) | 11 F | random.conditionals 6 (fragment 14, 23, 28, 39, 51, 92), fragment_ops.interaction 3 (basic_shader 4, 41, 68), struct.local 2 | `:5578` 3, `:5603` 1, `:5619` 4, `:5654` 3 | select | `deqp_ctrlflow_fs_{vec_cond,float_cond_varying,struct_select}`; new `deqp_ctrlres_fs_{ivec4_eq_else_colour,ivec_uniform_eq,float_ne_const_nested,int_uniform_colours,int_uniform_dead_bvec,float_cond_bvec_else,struct_nested_select}` | cf-dead (shares `walk_if16`; `float_ne_const_nested`'s outer `if (bool(int(1)))` is CD-a's); the fragment integer compares and conversions (`ICMP_OR.u32/s32`, `F32_TO_S32`, `F16_TO_S32`, `S32_TO_F32`) of intval-rest steps 2-4 / `fsintres` / plan #402 `lower-fsint` |
| **CR1 fs-nested-select** (new) | an `if` under a run-time `if` of a mediump fragment shader (`else if` ladders included), flattened from the inside out as step 4 does for the vertex walk: inner `CSEL.u32`/`MUX.v2i16`, the masks combined by `LSHIFT_AND.i32/.v2i16` | **6** (6 / 0; 0 / 6) | 6 F | random.conditionals 6 (fragment 79, 82, 88, 94; combined 79, 88) | `:5603` 4, `:5578` 2 | select | `deqp_ctrlflow_fs_int_cond_colour` (step 3 in the ctrlflow note; its `else if` makes it CR1); new `deqp_ctrlres_fs_{ivec_nested,int_elseif_nested,int_nested_bool}` | step 3 (each case's outer or inner condition is an integer compare); cf-dead (`int_elseif_nested`'s empty then-side, dead locals) |
| **4 vs-nested-select** | a nested `if`, a side over `PEEPHOLE_LIMIT_GL` that Mesa still flattens, and a varying lane set on one side, in the vertex walk: `CSEL.u32.eq` from the inside out (the join's phi of the earlier value) | **10** (10 / 0; 10 / 0) | 10 F | random.conditionals 6 (vertex 1, 6, 79, 82; combined 30, 66), conditionals.if 3 (`if_elseif_vertex`, `if_elseif_else_vertex`, `mixed_if_elseif_else_vertex`), algorithm 1 (`hsl_to_rgb_vertex`) | `nir.rs:6220` 6 (nested under a kept `if`), `:4969` 2 (side over the limit), `:4948` 1 (depth), `:5223` 1 (an undef lane at the join) | select | `deqp_ctrlflow_vs_{elseif,nested_hsl,varying_select}`; new `deqp_ctrlres_vs_{nested_bool,nested_int_bool,nested_long,side_over_limit,one_side_lane}` | intval-vs16 (done): every new vertex reference has mediump uniforms, and `vs_nested_bool` / `vs_side_over_limit` compare at `FCMP_OR.v2f16` with `S16_TO_S32`; the vertex integer compares (`ICMP_OR.s32`, `F16_TO_S32`); cf-dead (moves `vs_const_cond` into its pass) |
| **5 fs-mask-logic** | `&&`/`||` and `gl_FrontFacing` conditions, the inlined side effects, the early-return flag of an unrolled one-pass loop: `LSHIFT_AND.v2i16` of masks, `MUX.v2i16` | **7** (7 / 0; 0 / 7) | 7 F | conditionals.if 4 (`complex_condition`, `compound_statement`, `sequence_condition`, `sequence_statements` `_vertex`), builtin_variable 1 (`frontfacing`), operator.sequence 1 (`affect_or`), return 1 (`return_in_static_loop_dynamic_fragment`) | `:5619` 4, `:5603` 2, `:5654` 1 | select | `deqp_ctrlflow_fs_{shaderlib_and,frontfacing,side_effect_or,return_static_loop}` | step 3 (the 32-bit select); `affect_ternary` (8th case at `5cb80624`) is now cf-dead's (Mesa's `FCMP_OR ... .f1`) |
| **loops** (the loop items, not this note) | the `if` or `continue` in a kept mediump loop's body, a discard in a dynamic loop, a loop whose body is only `continue` | **24** (21 / 3; 0 / 24) | 24 NS | loops.do_while_{uniform,dynamic}_iterations 12, loops.{for,while}_{uniform,dynamic}_iterations 8, discard 3 (`dynamic_loop_{always,uniform,dynamic}`), struct.local 1 (`nested_dynamic_loop_assignment`) | `:6759` 12 (6 `only_continue` gone, 6 `unconditional_continue` kept), `:5603` 8 (kept: `nested`, `nested_tricky_dataflow_{1,2}`, `mixed_break_continue`), `:2425` 2, `:5593` 1, `:5654` 1 | kept-branch 18, gone 6 | `deqp_loops2_fs_*` (19 pairs), `deqp_ctrlflow_fs_loop_break_continue`, `deqp_idxloc_fs_struct_nested_loop_assign`; `do_while_dynamic_iterations.nested_tricky_dataflow_1` has only its uniform sibling | the loop steps of `mali-loops2-lowering.md` (`loops2-refs`, train 4) |
| **Total** | | **122** (102 / 20) | 97 F, 25 NS | | | | | |

The ctrlflow note's step 6 (the four `do_while` body `if`s at `5cb80624`)
is now the **loops** row, which has grown to 24. Steps 2-5 plus CR1 are
**46** cases (40 at `5cb80624`: steps 2-5 were 12, 11, 9 and 8). Step 3
gains the inflow shapes (`basic_shader.4/41/68`, `fragment.23/39/92`,
`nested_conditional_assignment`) and gives three nested cases to CR1. Step
4 gains `vertex.6/79/82` and `combined.30/66`. Step 5 loses `affect_ternary`
to cf-dead.

## 3. The new references (`tests/mali_deqp_ctrlres.rs`)

All 15 are refused at the tip with the full `ControlFlow` text ("Valhall:
control flow (if, a loop, a return before the end) is not lowered yet"),
which `refusals_pinned` pins. Each byte-compare is `#[ignore]`d.
`LOWERED` and `DIFFERS` are empty. "Mesa" is the ops of the refusing stage
that come from the `if`. The FS column is the `fs.fragment` word count the
test checks.

| Reference | dEQP case (`dEQP-GLES2.functional.`) | Step | Site | Mesa (the `if`'s ops) | FS |
|---|---|---|---|---|--:|
| `fs_ivec4_eq_else_colour` | `fragment_ops.interaction.basic_shader.4` | 3 | `:5578` | `F32_TO_S32.rtz` of the lanes, `ICMP_OR.u32.eq` + `ICMP_AND.u32.eq` (the `ivec4 ==`), one `CSEL.u32.eq`, `MUX.v2i16` (the else side's colour) | 19 |
| `fs_ivec_uniform_eq` | `shaders.random.conditionals.fragment.23` | 3 | `:5578` | `ivec4 - ivec4(i, i, h, g) == f` of an `ivec4` uniform: three `ICMP_OR.u32.eq`, `ICMP_AND`, `LSHIFT_AND.i32`, four `CSEL.u32.eq` before `V2F32_TO_V2F16` | 41 |
| `fs_float_ne_const_nested` | `shaders.random.conditionals.fragment.39` | 3 | `:5578` | the outer `k.r != g + j` sets dead locals (CD-b); `if (bool(int(1))) if (6.25 * f <= e)` becomes `FCMP_OR.v2f16.ge`, `S16_TO_S32`, four `CSEL.u32.eq` | 26 |
| `fs_int_uniform_colours` | `fragment_ops.interaction.basic_shader.41` | 3 | `:5603` | `int(h) == int(g)` of an int uniform: `ICMP_OR.u32.eq`, `CSEL.u32.eq`, two `MUX.v2i16` of the per-side colours | 13 |
| `fs_int_uniform_dead_bvec` | `fragment_ops.interaction.basic_shader.68` | 3 | `:5654` | `1 < g`: `ICMP_OR.s32.lt`, two `MUX.v2i16` (the colour + varying side; the `bvec4` else side is dead) | 15 |
| `fs_float_cond_bvec_else` | `shaders.random.conditionals.fragment.92` | 3 | `:5619` | an fp32 compare (`FCMP_OR.f32.ge`) whose else side is `vec4(ivec2 == ivec2, int - int, ...)`: `F16_TO_S32`, `F32_TO_S32`, `ICMP_*`, `LSHIFT_OR.i32`, three `CSEL.u32.eq`, a `MUX.v2i16` | 37 |
| `fs_struct_nested_select` | `shaders.struct.local.nested_conditional_assignment_fragment` | 3 | `:5654` | `uf_one > 0.0`: `FCMP_OR.v2f16.lt`, `S16_TO_S32`, three `CSEL.u32.eq` of the struct's 32-bit members (int `ui_zero`, varyings) | 21 |
| `fs_ivec_nested` | `shaders.random.conditionals.fragment.82` | CR1 | `:5578` | `ivec` compares nested two deep, then three more `if`s. Only the last `if`'s `FCMP_OR.v2f16.ge`, `S16_TO_S32` and three `CSEL.u32.eq` are left: the nested `if`s write only dead locals | 22 |
| `fs_int_elseif_nested` | `shaders.random.conditionals.fragment.94` | CR1 | `:5578` | `if (d < int(d)) {} else if (bool(b)) ... else ...` and `if (g <= 7) if (f != e) ... else colour`: `ICMP_OR.s32.ge`, `FCMP_OR.v2f16.ne` and `FCMP_OR.f32.ne`, `LSHIFT_OR.i32`, `LSHIFT_AND.i32`, three `CSEL.u32.eq`, `MUX.v2i16` | 33 |
| `fs_int_nested_bool` | `shaders.random.conditionals.fragment.88` | CR1 | `:5603` | an int compare over a side of int locals and `bvec` lanes, then `if (bool(5.5 + e)) if (c)`: `ICMP_OR.s32.ge`, `FCMP_OR.v2f16.eq`, `LSHIFT_AND.v2i16` and `.i32`, two `CSEL.u32.eq`, two `MUX.v2i16` | 25 |
| `vs_side_over_limit` | `shaders.random.conditionals.combined.30` | 4 | `nir.rs:4969` | a side of more than `PEEPHOLE_LIMIT_GL` ALU ops that Mesa still flattens: `FCMP_OR.v2f16.lt` of a mediump uniform, `S16_TO_S32`, three `CSEL.u32.eq` | 9 |
| `vs_one_side_lane` | `shaders.random.conditionals.combined.66` | 4 | `nir.rs:5223` | `a = 0.25` on one side of an fp32 compare: `FCMP_OR.f32.lt`, one `CSEL.u32.eq` of the earlier value | 13 |
| `vs_nested_bool` | `shaders.random.conditionals.vertex.6` | 4 | `nir.rs:6220` | `if (bvec3(..).t) if (0 <= h + 5) .. else ..; else if (..)`: `FCMP_OR.v2f16.eq`, `S16_TO_S32`, `FCMP_OR.f32.ne`, `LSHIFT_AND.i32`, four `CSEL.u32.eq` | 7 |
| `vs_nested_int_bool` | `shaders.random.conditionals.vertex.79` | 4 | `nir.rs:6220` | `if (l <= int(k)) if (a) ..; else if (h > g * f)`: `F16_TO_S32`, `ICMP_OR.s32.lt`, `FCMP_OR.f32.lt`, `LSHIFT_AND.i32`, three `CSEL.u32.eq` | 7 |
| `vs_nested_long` | `shaders.random.conditionals.vertex.82` | 4 | `nir.rs:6220` | a nested `if`, then a second `if` whose two sides both store `v_color`: the nested one is dead in Mesa, and one `FCMP_OR.f32.ge` and four `CSEL.u32.eq` of the second are left | 7 |

Not drawn again, because the words or the refusing source are already
pinned: the six discard wrappers (their words equal those of
`disc_basic_{always,uniform,dynamic}`), the two `if_elseif_else` siblings
(= `vs_elseif` in every region), `complex_condition_vertex` (fragment words
= `fs_shaderlib_and`), `sequence_{condition,statements}_vertex` (=
`fs_shaderlib_and`), `fragment.14` (the `fs_float_cond_varying` shape),
`combined.79` (the fragment source of `fs_int_cond_colour`) and
`combined.88` (the fragment source of `fs_int_nested_bool`).
`do_while_dynamic_iterations.nested_tricky_dataflow_1` has no reference of
its own, only its uniform sibling. It belongs to the loop items, which can
draw it with the rest of `deqp_loops2_*`.

## 4. Recommended order, and the train

1. **cf-dead** (train 4 commit 1, `cfdead-refs`): it must land first. It
   rewrites the `if` before `walk_if16` and the vertex walk see it, and
   three of the residue references (`fs_float_ne_const_nested`,
   `fs_int_elseif_nested`, `vs_nested_long`) hold a CD-a/CD-b `if` beside
   the one that stays.
2. **Step 2, disc-forms** (12): it has no dependency left once cf-dead's
   constant `if (false)` fold is in. Its integer compare (`ICMP_OR.s32.lt
   ... u2.w0`) is the form intval-rest step 1 already emits. All 12 cases
   are `shaders.discard` with empty `next_family`, so the forecast is **12
   compiling**.
3. **Step 3, fs-select32** (11): after the fragment integer compares and
   conversions of intval-rest steps 2-4 / `fsintres` (plan #402
   `lower-fsint`). Forecast 11 first refusals lifted.
4. **CR1, fs-nested-select** (6): right after step 3, in the same commit
   series. It is step 3's select applied recursively, with the masks
   combined.
5. **Step 4, vs-nested-select** (10): it can run beside 3 (it touches
   `nir.rs`, not `nir_fs.rs`) once intval-vs16's mediump compares are in
   (done). Forecast 10 lifted, 9 compiling (`combined.66` meets `Arithmetic`
   next).
6. **Step 5, fs-mask-logic** (7): last. It needs step 3's 32-bit select and
   CR1's mask combine.

The **loops** row (24, all NotSupported) stays with the loop steps of train
4 (`loops2-refs`). This note gives it no step.

**Train placement:** steps 2, 3, CR1, 4 and 5 (46 cases) can make up one
**train 5** group, after train 4 (`mali-lowering-schedule.md`: trains in
series, train 4 holding cf-dead and the loops). Step 2 could also go at the
end of train 4, right after cf-dead, since it depends on nothing else. The
choice belongs to `schedule-refresh-4`. The 46 are under the 100-case line
on their own. Together with cf-dead's 52 and the loops' 24 they are
`ControlFlow`'s 102 and `DiscardForm`'s 20.

## 5. Caveats

- HOST-ONLY: a case that compiles on the host is not a board pass.
- Only the 10,597 board-refused cases of the slices are measured.
- The step of a case is read from its source and site. The Mesa class reads
  the final words only. CR1 versus step 3 is decided by whether a run-time
  `if` stays nested after cf-dead's folds: `fs_float_ne_const_nested` has
  a constant outer `if`, so it is step 3. In `fs_ivec_nested` (CR1) and
  `vs_nested_long` (step 4), Mesa's nested `if`s write only dead values
  (the class is `select` because of a later flat `if`). Once cf-dead's CD-b
  removes them, these two may need only the flat `if` the walks already
  take. The lowering commit should recheck them right after cf-dead.
- Site line numbers are `349f989f`'s.

## Appendix: every case, its step

`Board` is the NanoPC-T6 status at `37305608`. `Reference` is the pinned
reference with the same pair, the same refusing source, or the same words.

| Step | Case (`dEQP-GLES2.functional.`) | Board | Stage | Text | Site | Mesa | Reference |
|---|---|---|---|---|---|---|---|
| 2 | `shaders.discard.basic_always` | Fail | fs | DiscardForm | `nir_fs.rs:8616` | discard | `deqp_ctrlflow_disc_basic_always` |
| 2 | `shaders.discard.basic_dynamic` | Fail | fs | DiscardForm | `nir_fs.rs:11494` | discard | `deqp_ctrlflow_disc_basic_dynamic` |
| 2 | `shaders.discard.basic_uniform` | Fail | fs | DiscardForm | `nir_fs.rs:2425` | discard | `deqp_ctrlflow_disc_basic_uniform` |
| 2 | `shaders.discard.function_always` | Fail | fs | DiscardForm | `nir_fs.rs:8616` | discard | words = `deqp_ctrlflow_disc_basic_always` |
| 2 | `shaders.discard.function_dynamic` | Fail | fs | DiscardForm | `nir_fs.rs:11494` | discard | `deqp_ctrlflow_disc_function_dynamic` |
| 2 | `shaders.discard.function_static_loop_always` | Fail | fs | DiscardForm | `nir_fs.rs:2423` | discard | `deqp_ctrlflow_disc_function_static_loop_always` |
| 2 | `shaders.discard.function_static_loop_dynamic` | Fail | fs | DiscardForm | `nir_fs.rs:11494` | discard | words = `deqp_ctrlflow_disc_basic_dynamic` |
| 2 | `shaders.discard.function_static_loop_uniform` | Fail | fs | DiscardForm | `nir_fs.rs:2425` | discard | words = `deqp_ctrlflow_disc_basic_uniform` |
| 2 | `shaders.discard.function_uniform` | Fail | fs | DiscardForm | `nir_fs.rs:2425` | discard | words = `deqp_ctrlflow_disc_basic_uniform` |
| 2 | `shaders.discard.static_loop_always` | Fail | fs | DiscardForm | `nir_fs.rs:2423` | discard | words = `deqp_ctrlflow_disc_basic_always` |
| 2 | `shaders.discard.static_loop_dynamic` | Fail | fs | DiscardForm | `nir_fs.rs:11494` | discard | words = `deqp_ctrlflow_disc_basic_dynamic` |
| 2 | `shaders.discard.static_loop_uniform` | Fail | fs | DiscardForm | `nir_fs.rs:2425` | discard | `deqp_ctrlflow_disc_static_loop_uniform` |
| 3 | `fragment_ops.interaction.basic_shader.4` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | select | `deqp_ctrlres_fs_ivec4_eq_else_colour` |
| 3 | `fragment_ops.interaction.basic_shader.41` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | select | `deqp_ctrlres_fs_int_uniform_colours` |
| 3 | `fragment_ops.interaction.basic_shader.68` | Fail | fs | ControlFlow | `nir_fs.rs:5654` | select | `deqp_ctrlres_fs_int_uniform_dead_bvec` |
| 3 | `shaders.random.conditionals.fragment.14` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | sibling of `deqp_ctrlflow_fs_float_cond_varying` (side reads a varying) |
| 3 | `shaders.random.conditionals.fragment.23` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | select | `deqp_ctrlres_fs_ivec_uniform_eq` |
| 3 | `shaders.random.conditionals.fragment.28` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | `deqp_ctrlflow_fs_vec_cond` |
| 3 | `shaders.random.conditionals.fragment.39` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | select | `deqp_ctrlres_fs_float_ne_const_nested` |
| 3 | `shaders.random.conditionals.fragment.51` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | `deqp_ctrlflow_fs_float_cond_varying` |
| 3 | `shaders.random.conditionals.fragment.92` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | `deqp_ctrlres_fs_float_cond_bvec_else` |
| 3 | `shaders.struct.local.conditional_assignment_fragment` | Fail | fs | ControlFlow | `nir_fs.rs:5654` | select | `deqp_ctrlflow_fs_struct_select` |
| 3 | `shaders.struct.local.nested_conditional_assignment_fragment` | Fail | fs | ControlFlow | `nir_fs.rs:5654` | select | `deqp_ctrlres_fs_struct_nested_select` |
| 4 | `shaders.algorithm.hsl_to_rgb_vertex` | Fail | vs | ControlFlow | `nir.rs:4948` | select | `deqp_ctrlflow_vs_nested_hsl` |
| 4 | `shaders.conditionals.if.if_elseif_else_vertex` | Fail | vs | ControlFlow | `nir.rs:6220` | select | words = `deqp_ctrlflow_vs_elseif` |
| 4 | `shaders.conditionals.if.if_elseif_vertex` | Fail | vs | ControlFlow | `nir.rs:6220` | select | `deqp_ctrlflow_vs_elseif` |
| 4 | `shaders.conditionals.if.mixed_if_elseif_else_vertex` | Fail | vs | ControlFlow | `nir.rs:6220` | select | words = `deqp_ctrlflow_vs_elseif` |
| 4 | `shaders.random.conditionals.combined.30` | Fail | vs | ControlFlow | `nir.rs:4969` | select | `deqp_ctrlres_vs_side_over_limit` |
| 4 | `shaders.random.conditionals.combined.66` | Fail | vs | ControlFlow | `nir.rs:5223` | select | `deqp_ctrlres_vs_one_side_lane` |
| 4 | `shaders.random.conditionals.vertex.1` | Fail | vs | ControlFlow | `nir.rs:4969` | select | `deqp_ctrlflow_vs_varying_select` |
| 4 | `shaders.random.conditionals.vertex.6` | Fail | vs | ControlFlow | `nir.rs:6220` | select | `deqp_ctrlres_vs_nested_bool` |
| 4 | `shaders.random.conditionals.vertex.79` | Fail | vs | ControlFlow | `nir.rs:6220` | select | `deqp_ctrlres_vs_nested_int_bool` |
| 4 | `shaders.random.conditionals.vertex.82` | Fail | vs | ControlFlow | `nir.rs:6220` | select | `deqp_ctrlres_vs_nested_long` |
| 5 | `shaders.builtin_variable.frontfacing` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | select | `deqp_ctrlflow_fs_frontfacing` |
| 5 | `shaders.conditionals.if.complex_condition_vertex` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | fs words = `deqp_ctrlflow_fs_shaderlib_and` |
| 5 | `shaders.conditionals.if.compound_statement_vertex` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | `deqp_ctrlflow_fs_shaderlib_and` |
| 5 | `shaders.conditionals.if.sequence_condition_vertex` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | words = `deqp_ctrlflow_fs_shaderlib_and` |
| 5 | `shaders.conditionals.if.sequence_statements_vertex` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | select | words = `deqp_ctrlflow_fs_shaderlib_and` |
| 5 | `shaders.operator.sequence.side_effects.affect_or` | Fail | fs | ControlFlow | `nir_fs.rs:5654` | select | `deqp_ctrlflow_fs_side_effect_or` |
| 5 | `shaders.return.return_in_static_loop_dynamic_fragment` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | select | `deqp_ctrlflow_fs_return_static_loop` |
| CR1 | `shaders.random.conditionals.combined.79` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | select | fs source = `deqp_ctrlflow_fs_int_cond_colour` |
| CR1 | `shaders.random.conditionals.combined.88` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | select | fs source = `deqp_ctrlres_fs_int_nested_bool` |
| CR1 | `shaders.random.conditionals.fragment.79` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | select | `deqp_ctrlflow_fs_int_cond_colour` |
| CR1 | `shaders.random.conditionals.fragment.82` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | select | `deqp_ctrlres_fs_ivec_nested` |
| CR1 | `shaders.random.conditionals.fragment.88` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | select | `deqp_ctrlres_fs_int_nested_bool` |
| CR1 | `shaders.random.conditionals.fragment.94` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | select | `deqp_ctrlres_fs_int_elseif_nested` |
| cf-dead | `fragment_ops.interaction.basic_shader.24` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | gone | `deqp_ctrlflow_fs_float_cond_colour_read` |
| cf-dead | `fragment_ops.interaction.basic_shader.52` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `fragment_ops.interaction.basic_shader.53` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `fragment_ops.interaction.basic_shader.55` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `fragment_ops.interaction.basic_shader.58` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | gone | `deqp_cfdead_fs_self_compare` |
| cf-dead | `fragment_ops.interaction.basic_shader.61` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `fragment_ops.interaction.basic_shader.72` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `fragment_ops.interaction.basic_shader.80` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.discard.basic_never` | Fail | fs | DiscardForm | `nir_fs.rs:2423` | gone | `deqp_ctrlflow_disc_basic_never` |
| cf-dead | `shaders.discard.dynamic_loop_never` | NotSupported | fs | DiscardForm | `nir_fs.rs:2423` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.discard.function_never` | Fail | fs | DiscardForm | `nir_fs.rs:2423` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.discard.function_static_loop_never` | Fail | fs | DiscardForm | `nir_fs.rs:2423` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.discard.static_loop_never` | Fail | fs | DiscardForm | `nir_fs.rs:2423` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.linkage.varying_4` | Fail | fs | ControlFlow | `nir_fs.rs:5603` | gone | `deqp_posvary_unwritten_declared_dead_read` |
| cf-dead | `shaders.linkage.varying_5` | Fail | vs | ControlFlow | `nir.rs:5244` | gone | `deqp_ctrlflow_vs_varying_one_side` |
| cf-dead | `shaders.linkage.varying_6` | Fail | vs | ControlFlow | `nir.rs:5244` | gone | fs/vs source = `deqp_ctrlflow_vs_varying_one_side` |
| cf-dead | `shaders.linkage.varying_7` | Fail | vs | ControlFlow | `nir.rs:5244` | gone | fs/vs source = `deqp_ctrlflow_vs_varying_one_side` |
| cf-dead | `shaders.operator.sequence.side_effects.affect_ternary` | Fail | fs | ControlFlow | `nir_fs.rs:5654` | gone | `deqp_ctrlflow_fs_side_effect_ternary` |
| cf-dead | `shaders.random.all_features.vertex.10` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.all_features.vertex.57` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.all_features.vertex.65` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.14` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.22` | Fail | vs | ControlFlow | `nir.rs:4979` | gone | `deqp_cfdead_vs_const_nested` |
| cf-dead | `shaders.random.conditionals.combined.28` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | gone | fs/vs source = `deqp_ctrlflow_fs_vec_cond` |
| cf-dead | `shaders.random.conditionals.combined.33` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.34` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | `deqp_cfdead_vs_dead_local` |
| cf-dead | `shaders.random.conditionals.combined.41` | Fail | vs | ControlFlow | `nir.rs:5223` | gone | `deqp_ctrlflow_vs_nested_combined` |
| cf-dead | `shaders.random.conditionals.combined.42` | Fail | vs | ControlFlow | `nir.rs:4979` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.43` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.45` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | fs/vs source = `deqp_cfdead_fs_colour_stored_again` |
| cf-dead | `shaders.random.conditionals.combined.56` | Fail | vs | ControlFlow | `nir.rs:6220` | gone | `deqp_cfdead_vs_unread_nested` |
| cf-dead | `shaders.random.conditionals.combined.60` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.8` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.80` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | `deqp_intval3_rsg_condcomb80_fs` |
| cf-dead | `shaders.random.conditionals.combined.82` | Fail | vs | ControlFlow | `nir.rs:6220` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.combined.90` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | fs/vs source = `deqp_cfdead_fs_int_dead_local` |
| cf-dead | `shaders.random.conditionals.combined.92` | Fail | vs | ControlFlow | `nir.rs:4979` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.fragment.33` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.fragment.43` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.fragment.45` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | `deqp_cfdead_fs_colour_stored_again` |
| cf-dead | `shaders.random.conditionals.fragment.60` | Fail | fs | ControlFlow | `nir_fs.rs:5619` | gone | `deqp_cfdead_fs_float_cond_dead` |
| cf-dead | `shaders.random.conditionals.fragment.78` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.fragment.80` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | fs/vs source = `deqp_intval3_rsg_condcomb80_fs` |
| cf-dead | `shaders.random.conditionals.fragment.9` | Fail | fs | ControlFlow | `nir_fs.rs:5654` | gone | `deqp_fsint2_rnd_cond9` |
| cf-dead | `shaders.random.conditionals.fragment.90` | Fail | fs | ControlFlow | `nir_fs.rs:5578` | gone | `deqp_cfdead_fs_int_dead_local` |
| cf-dead | `shaders.random.conditionals.vertex.38` | Fail | vs | ControlFlow | `nir.rs:4979` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.vertex.59` | Fail | vs | ControlFlow | `nir.rs:5244` | gone | `deqp_ctrlflow_vs_varying_else_side` |
| cf-dead | `shaders.random.conditionals.vertex.60` | Fail | vs | ControlFlow | `nir.rs:6220` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.vertex.74` | Fail | vs | ControlFlow | `nir.rs:4979` | gone | `deqp_cfdead_vs_varying_overwritten` |
| cf-dead | `shaders.random.conditionals.vertex.90` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | `deqp_ctrlflow_vs_statement_side` |
| cf-dead | `shaders.random.conditionals.vertex.93` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| cf-dead | `shaders.random.conditionals.vertex.96` | Fail | vs | ControlFlow | `nir.rs:4969` | gone | cf-dead class (cfdead note 5) |
| loops | `shaders.discard.dynamic_loop_always` | NotSupported | fs | DiscardForm | `nir_fs.rs:2425` | kept-branch | `deqp_loops2_fs_disc_always` |
| loops | `shaders.discard.dynamic_loop_dynamic` | NotSupported | fs | DiscardForm | `nir_fs.rs:5593` | kept-branch | `deqp_loops2_fs_disc_dynamic` |
| loops | `shaders.discard.dynamic_loop_uniform` | NotSupported | fs | DiscardForm | `nir_fs.rs:2425` | kept-branch | `deqp_loops2_fs_disc_dyn` |
| loops | `shaders.loops.do_while_dynamic_iterations.mixed_break_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | `deqp_loops2_fs_do_dyn_mixed_break_continue` |
| loops | `shaders.loops.do_while_dynamic_iterations.nested_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | `deqp_loops2_fs_do_dyn_nested` |
| loops | `shaders.loops.do_while_dynamic_iterations.nested_tricky_dataflow_1_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | none; uniform sibling `deqp_loops2_fs_do_uni_nested_tricky_dataflow_1` |
| loops | `shaders.loops.do_while_dynamic_iterations.nested_tricky_dataflow_2_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | `deqp_loops2_fs_do_dyn_nested_tricky_dataflow_2` |
| loops | `shaders.loops.do_while_dynamic_iterations.only_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | gone | `deqp_loops2_fs_do_dyn_only_continue` |
| loops | `shaders.loops.do_while_dynamic_iterations.unconditional_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | kept-branch | `deqp_loops2_fs_do_dyn_unconditional_continue` |
| loops | `shaders.loops.do_while_uniform_iterations.mixed_break_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | `deqp_ctrlflow_fs_loop_break_continue` |
| loops | `shaders.loops.do_while_uniform_iterations.nested_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | `deqp_loops2_fs_do_uni_nested` |
| loops | `shaders.loops.do_while_uniform_iterations.nested_tricky_dataflow_1_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | `deqp_loops2_fs_do_uni_nested_tricky_dataflow_1` |
| loops | `shaders.loops.do_while_uniform_iterations.nested_tricky_dataflow_2_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5603` | kept-branch | `deqp_loops2_fs_do_uni_nested_tricky_dataflow_2` |
| loops | `shaders.loops.do_while_uniform_iterations.only_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | gone | `deqp_loops2_fs_do_uni_only_continue` |
| loops | `shaders.loops.do_while_uniform_iterations.unconditional_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | kept-branch | `deqp_loops2_fs_do_uni_unconditional_continue` |
| loops | `shaders.loops.for_dynamic_iterations.only_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | gone | `deqp_loops2_fs_for_dyn_only_continue` |
| loops | `shaders.loops.for_dynamic_iterations.unconditional_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | kept-branch | `deqp_loops2_fs_for_dyn_unconditional_continue` |
| loops | `shaders.loops.for_uniform_iterations.only_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | gone | `deqp_loops2_fs_for_uni_only_continue` |
| loops | `shaders.loops.for_uniform_iterations.unconditional_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | kept-branch | `deqp_loops2_fs_for_uni_unconditional_continue` |
| loops | `shaders.loops.while_dynamic_iterations.only_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | gone | `deqp_loops2_fs_wh_dyn_only_continue` |
| loops | `shaders.loops.while_dynamic_iterations.unconditional_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | kept-branch | `deqp_loops2_fs_wh_dyn_unconditional_continue` |
| loops | `shaders.loops.while_uniform_iterations.only_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | gone | `deqp_loops2_fs_wh_uni_only_continue` |
| loops | `shaders.loops.while_uniform_iterations.unconditional_continue_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:6759` | kept-branch | `deqp_loops2_fs_wh_uni_unconditional_continue` |
| loops | `shaders.struct.local.nested_dynamic_loop_assignment_fragment` | NotSupported | fs | ControlFlow | `nir_fs.rs:5654` | kept-branch | `deqp_idxloc_fs_struct_nested_loop_assign` |
