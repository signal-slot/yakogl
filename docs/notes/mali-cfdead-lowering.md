<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): cf-dead and the reordered `gl_FragColor` lanes: design and Mesa references

**HOST-ONLY design** (plan #387, item `cfdead-refs`; issues #7, #20). This
note covers the two steps that train 2 (`arith-lower-2`) left open with no
description: **cf-dead** and **the reordered lanes**
(`mali-lowering-schedule.md` section 5.5a, commits 1 and 2 of train 4). It
gives each step's shape in Mesa 26.2.1's words, the lowering, the dEQP-GLES2
groups and a host forecast. Nothing under `src/` changed. The new
references are `tools/mali/testdata/deqp_cfdead_*` (12 pairs), pinned by
`tests/mali_deqp_cfdead.rs`. Measured at the tip **`297d118c`**
(2026-11-27). Counts are first-refusal cases of the host measure, never a
render verdict. The G310 needs no separate work
([`mali-g310-vs-g610.md`](mali-g310-vs-g610.md)).

Where the two steps were described before:

- **cf-dead**: `mali-deqp-ctrlflow.md` section 5, step 1 ("fold an `if`
  whose condition is a constant to its taken side, and drop an `if` whose
  sides write only locals, varyings or outputs nothing reads after it"; 45
  cases at `5cb80624`). Schedule section 5.5a, row "cf-dead". Section 19 of
  `mali-deqp-families.md` (row 22) calls it landed in train 2, but the tip
  has no such code and the handoff lists it as open. The measure below
  confirms it is open: every case it covers still refuses at the tip.
- **The reordered lanes**: schedule 5.5a says no note describes them, and
  asks this item to decide what they are. The decision is in section 1.2.

## 1. What the two steps are

### 1.1 cf-dead

The `if` disappears in Mesa's output: no `CSEL`, `MUX`, kept `BRANCHZ` or
`DISCARD` is left of it (the class `gone` of `mali-deqp-ctrlflow.md` section
1). Two rules remove an `if`, and a case often needs both:

- **CD-a, the condition folds to a constant.** The condition is a literal,
  a `const` global, or an identity of one value: `int c = d; if (d >= c)`
  (`ige a, a`), `if (d != d)` of an `ivec4` uniform (`ine a, a` per lane), or
  `if (-2 + 5 > int(9) - 7)`. The `if` becomes its taken side. An
  `if (false) discard;` becomes nothing (the `*_never` discards).
- **CD-b, nothing reads what the `if` writes.** Each side writes only
  locals no later statement reads, the colour (which is stored again,
  unconditionally, before the end), a varying (stored again later in the
  vertex shader, or read by no fragment shader once the link has run), or
  expression statements with no effect (`(f) + v_color.q >= e.s;`). The
  `if` is dropped.

### 1.2 The reordered lanes (the decision)

**Definition.** A colour word whose two fp16 halves are lanes of **one
varying load** in another order than that load's own pairs: not (x, y) or
(z, w), not the swapped (y, x) or (w, z), and not a load the link keeps
whole. Examples are (y, z), (z, x) and (x of column 0, x of column 1) of a
`mat2` varying. The compiler refuses this at **`nir_fs.rs:11057`**, the
`else { return Err(MaliError::ColourForm) }` of `vectorise`'s input-lane
arm, under the comment "Another swizzle of a compacted load has no
reference". At `a7260420` this was `vectorise`'s 8735, the first site of
`mali-fclane-lowering.md` section 3. That note's step 1 (mechanism A, the
highp lanes) has since moved the highp `op_pair_*` shapes off it, so what is
left there is the mediump and lowp lanes.

Why this is cf-dead's companion: once cf-dead folds their `if`s, cf-dead
cases stop here. Two of the eight new `if` references were folded by hand
(section 4) and stop at exactly this site: `fs_float_cond_dead`'s
`gl_FragColor = (i.ragb)` and `vs_const_nested`'s fragment
`gl_FragColor = j.bgra`. By a scan of the sources, 15 of cf-dead's 52 cases
hold a swizzled varying like these.

## 2. The pinned references at the tip

The sites were measured as `mali-deqp-ctrlflow.md` section 6a measured
them. A temporary `eprintln!(file!(), line!())` was put at every
`MaliError::ControlFlow`, `ColourForm` and `DiscardForm` construction in
`nir.rs`, `nir_fs.rs` and `vec16.rs`, and the pairs were compiled with
`mali_ref::compile_pair` in a scratch test. Both were reverted and nothing
was committed. Each case fires exactly one site. Sites are `nir_fs.rs`
unless `nir.rs` is named, with the line numbers of `297d118c`.

These references already pin cf-dead shapes. They stay with their own tests,
and this item drew none of them again:

| Reference (test) | dEQP case (`dEQP-GLES2.functional.`) | Text | Site | Rule |
|---|---|---|---|---|
| `deqp_ctrlflow_fs_int_cond` (`mali_deqp_ctrlflow.rs`) | `fragment_ops.interaction.basic_shader.69` | compiles, byte-equal (`ir_live`) | - | CD-b, landed |
| `deqp_ctrlflow_vs_nested` (`mali_deqp_ctrlflow.rs`) | `shaders.random.conditionals.vertex.10` | compiles, byte-equal (`vs_const_cond`) | - | CD-a, landed |
| `deqp_ctrlflow_fs_float_cond_colour_read` | `fragment_ops.interaction.basic_shader.24` | ControlFlow | `:5619` | CD-b (colour stored again) |
| `deqp_ctrlflow_fs_side_effect_ternary` | `shaders.operator.sequence.side_effects.affect_ternary` | ControlFlow | `:5654` | CD-b (Mesa's `FCMP_OR ... .f1` result, no select) |
| `deqp_ctrlflow_vs_nested_combined` | `shaders.random.conditionals.combined.41` | ControlFlow | `nir.rs:5201` | CD-b |
| `deqp_ctrlflow_vs_statement_side` | `shaders.random.conditionals.vertex.90` | ControlFlow | `nir.rs:4947` | CD-b (statement side) |
| `deqp_ctrlflow_vs_varying_one_side` | `shaders.linkage.varying_5` | ControlFlow | `nir.rs:5222` | CD-b (varying no fragment shader reads) |
| `deqp_ctrlflow_vs_varying_else_side` | `shaders.random.conditionals.vertex.59` | ControlFlow | `nir.rs:5222` | CD-b |
| `deqp_ctrlflow_disc_basic_never` | `shaders.discard.basic_never` | DiscardForm | `:2423` | CD-a (`if (false) discard;`) |
| **`deqp_posvary_unwritten_declared_dead_read`** (`mali_deqp_posvary.rs`) | `shaders.linkage.varying_4` | ControlFlow | **`:5603`** | CD-a/undef: `if (u_false)` picks `vec4(var)` of a varying no vertex shader writes, or `vec4(1.0)`. Mesa's `nir_opt_undef` takes the defined side, and the colours disagree at `:5603` |
| `deqp_intval3_rsg_condcomb80_fs` (`mali_deqp_intval3.rs`) | `shaders.random.conditionals.combined.80` | ControlFlow | `:5578` | CD-a (`d != d` of an `ivec4`) |
| `deqp_fsint2_rnd_cond9` (`mali_deqp_fsint2.rs`) | `shaders.random.conditionals.fragment.9` | ControlFlow | `:5654` | CD-b |

Two references already pin reordered-lanes shapes at `:11057`:
`deqp_fclane_vec3_local` (`mali_deqp_fclane.rs`,
`indexing.matrix_subscript.mat3_static_loop_write_static_loop_read_fragment`)
and `deqp_idxloc_fs_mat3_dl_sl` (`mali_deqp_idxloc.rs`, the
`mat3_dynamic_loop_write_static_loop_read` case). Both are mat3 shapes of
step RL2.

The other `mali-deqp-ctrlflow.md` references are not cf-dead. Mesa keeps a
select or a branch for them (steps 2 to 6 of that note), and at this tip
they refuse at `:5603` (`fs_frontfacing`, `fs_int_cond_colour`,
`fs_loop_break_continue`, `fs_return_static_loop`), `:5619` (`fs_vec_cond`,
`fs_shaderlib_and`, `fs_float_cond_varying`), `:5654` (`fs_side_effect_or`,
`fs_struct_select`), `nir.rs:4926` (`vs_nested_hsl`), `nir.rs:4947`
(`vs_varying_select`), `nir.rs:6198` (`vs_elseif`), and at the discard sites
`:2423`, `:2425`, `:8616` and `:11494`.

## 3. How this was measured

- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out build/measure --jobs 3 --chunk 100`
  over the 10,597 board-refused cases of the slices
  `2026-11-15-t6-gles2-S{1..4}-37305608`. These are the cases of
  `mali-deqp-families.md` sections 18 and 19, all in
  `tools/cts/gles2-mustpass.txt`. VK-GL-CTS is `93bca018` (one commit
  fetched, `external/fetch_sources.py`, `cts-yakogl-target.patch`). All
  10,597 cases began, and 106 chunk processes exited 1. **4,709** cases meet
  a host refusal. The first refusal is `ControlFlow` for 102 of them,
  `DiscardForm` for 20 and `ColourForm` for 943. These are the same
  `ControlFlow` and `DiscardForm` counts as section 19 at `e9033663`, with
  `ColourForm` up from 926.
- **Sites:** each of the 1,065 refusing pairs (`programs/` of the measure,
  one program per case) was compiled through the instrument of section 2.
  Every case reproduces its measured text and fires one site.
- **Mesa's class:** each of the 122 `ControlFlow`/`DiscardForm` pairs was
  drawn on the drm-shim (`deqp_ref.sh --instrument shim`, not committed).
  The refusing stage's disassembly is classed `kept-branch` (a `BRANCHZ` or
  `BRANCH` other than the blend epilogue's `BRANCHZI`), `discard`, `select`
  (`CSEL`/`MUX`) or `gone`.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_cfdead_<name> --instrument shim`, on the shim
  that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's
  self-check (`gm2_cf_ifelse_fs`) is byte-equal. The eight `if` pairs are
  byte-equal to their first draw, and the four lane pairs were drawn twice
  with equal results. `.nir.sha` comes from `tools/mali/nir_refs.sh
  'deqp_cfdead_*'`. Each pair is verbatim from the measure's `.qpa`.
- **No duplicate pair:** the SHA-256 of each new `.vert` + `.frag` was
  compared with every pair in `tools/mali/testdata`, and none is equal. The
  shared parts are listed in the test's header: the `v_color` fragment
  shader of the `vertex.*` cases, and `vs_dead_local`'s fragment shader,
  which is also `deqp_constprod_chainlit_fs`'s. Mesa's words for
  `vs_unread_nested` and `vs_varying_overwritten` are equal to
  `deqp_ctrlflow_fs_int_cond`'s in every region: the varyings are gone,
  leaving the position program and a 7-word uniform colour. They pin new
  **vertex** shapes, at sites `nir.rs:6198` and `:4957` that no other
  reference reaches with a `gone` result. They add no new words.

## 4. The references (`tests/mali_deqp_cfdead.rs`)

Each is pinned REFUSED with its full first refusal (`refusals_pinned`),
with an `#[ignore]`d byte-compare (`compares`). Texts:
**ControlFlow** is "Valhall: control flow (if, a loop, a return before the
end) is not lowered yet". **ColourForm** is "Valhall: a gl_FragColor lane
other than the varying's own lane or a constant is not lowered yet".

"After folding" is a scratch check that was not committed. Each `if` of the
reference was folded or dropped by hand in the source, as CD-a or CD-b
would, and the folded pair was compiled at the tip. Where it compiled, it
was compared with `mali_ref::differences` against the reference of the
**unfolded** pair.

| Reference | dEQP case | Text | Site | Rule | After folding |
|---|---|---|---|---|---|
| `fs_int_dead_local` | `shaders.random.conditionals.fragment.90` | ControlFlow | `:5578` | CD-b: an `ivec4 ==` condition, the side sets locals only | **byte-equal** (0 differences) |
| `fs_colour_stored_again` | `shaders.random.conditionals.fragment.45` | ControlFlow | `:5578` | CD-b: `if`/`else if`, the colour stored again after | `FsOperandForm` |
| `fs_self_compare` | `fragment_ops.interaction.basic_shader.58` | ControlFlow | `:5603` | CD-a: `int c = d; if (d >= c)` is true | **byte-equal** |
| `fs_float_cond_dead` | `shaders.random.conditionals.fragment.60` | ControlFlow | `:5619` | CD-b: a 16-bit compare under a `bool`, the side sets a global `vec4` nothing reads after | `ColourForm` at `:11057` (`i.ragb`): step RL1 |
| `vs_const_nested` | `shaders.random.conditionals.combined.22` | ControlFlow | `nir.rs:4957` | CD-a: `if (-2 + 5 > int(9) - 7) if (1.375 * 1.25 <= n * float(false) + 0.001)`, both constant (`n` is 6.5) | the fragment shader's `j.bgra`: `ColourForm` at `:11057`, step RL1 |
| `vs_dead_local` | `shaders.random.conditionals.combined.34` | ControlFlow | `nir.rs:4947` | CD-b: a side past `PEEPHOLE_LIMIT_GL` writing a local and a global stored again after it; a second `if` writing the varying `b`, which the fragment shader reads only in an `if` that stores the colour unchanged | `Arithmetic` |
| `vs_varying_overwritten` | `shaders.random.conditionals.vertex.74` | ControlFlow | `nir.rs:4957` | CD-b: a nested `if` writing `v_color`, stored again three times after it | **byte-equal** |
| `vs_unread_nested` | `shaders.random.conditionals.combined.56` | ControlFlow | `nir.rs:6198` | CD-b: a nested `if` writing varyings the fragment shader reads only under its own dead `if` | **byte-equal** |
| `rl_add_vec4` | `shaders.operator.binary_operator.add.mediump_vec4_fragment` | ColourForm | `:11057` | RL1: `v_in0.yzwx + v_in1.wzyx` | - |
| `rl_minus_vec4` | `shaders.operator.unary_operator.minus.mediump_vec4_fragment` | ColourForm | `:11057` | RL1: `-v_in0.yzwx` (with the operator's `* 0.5 + 0.5`) | - |
| `rl_swizzle` | `shaders.random.swizzle.fragment.35` | ColourForm | `:11057` | RL1: two float varyings of one packed load, in reverse order | - |
| `rl_mat2_sub` | `shaders.matrix.sub.dynamic_mediump_mat2_mat2_fragment` | ColourForm | `:11057` | RL2: `vec4(res[0][0], res[1][0], ..)` of a `mat2` varying (lanes 0 and 2 of one load) | - |

Four of the eight `if` pairs are byte-equal once their `if`s are gone. So
for them cf-dead needs no new instruction form: the walk gives Mesa's words
once the `if` is not in its input.

## 5. Step CD (cf-dead): one commit, train 4 commit 1

**What Mesa emits.** There is no trace of the `if`. `fs_self_compare` is the
7-word uniform-colour fragment shader (`MOV.i32 r0, 0x34000000`,
`MOV.i32 r1, u2.w0`, `BLEND`). `fs_int_dead_local` is the 10-word colour of
the statement before the `if` (`FADD.f32 .h0` of the varying, `MKVEC.v2i16`
with the uniform half, `FMA.v2f16`). In `vs_varying_overwritten` and
`vs_unread_nested` the vertex varying program is gone, leaving only the
24-word position shader. This is GLSL's `opt_dead_code` and
`opt_constant_folding`, then NIR's `nir_opt_dead_cf`, `nir_opt_dce` and the
link's `nir_remove_unused_varyings` / `nir_opt_varyings`. They run before
`nir_opt_peephole_select` would make a select, so nothing reaches the back
end.

**The lowering.** It is one pass over the front end's IR, before the walks
(`nir.rs` `vs_steps`/`vs_side` and `nir_fs.rs` `walk_nodes`/`walk_if16`
read its output unchanged). It runs to a fixed point:

1. **CD-a:** fold each `IrNode::If` condition with the one constant folder
   (`mesa_fold.rs`, shared with constop/negzero; no second folder). Add the
   integer identities of `nir_opt_algebraic.py:1704-1707` that the cases
   use: `ieq a, a` and `ige a, a` are true, and `ilt a, a` and `ine a, a`
   are false (GLSL's `<=` and `>` are `ige` and `ilt` with the operands
   swapped, so they fold too). A float
   compare of a value with itself is not folded here: `feq a, a` is not
   true for NaN, so Mesa's exact rule needs a reference first. Replace a
   folded `if` by its taken side. `nir.rs`'s `vs_const_cond` is this rule
   for the vertex walk today: move it into the pass and leave none in the
   walk. A `discard` under a folded `false` goes with its side.
2. **CD-b:** compute, after each `if`, the values read before the end:
   locals, the colour (dead when every path stores it again before the
   end) and, in the vertex shader, each varying. A varying is dead when it
   is stored again before the end, or when the linked fragment shader does
   not read it (`compile_vs_linked` has the fragment shader's reads; a
   vertex shader compiled with no fragment shader keeps every varying
   live). Drop an `if` whose sides write only dead values and hold no
   `discard`, look-up with side effects, or loop. `nir_fs.rs` `ir_live` is
   this rule for one case today (`fs_int_cond`): generalise it into the
   pass.
3. **The undef side** (`deqp_posvary_unwritten_declared_dead_read`, at
   `:5603`) is CD-a's companion. A side that reads a varying no vertex
   shader writes is `nir_opt_undef`'s, and Mesa keeps the other side.
   `walk_if16`'s `agree` already takes a lane-wise `Val::Undef`, yet the
   case still refuses at `:5603`. The likely cause, to confirm when the
   step starts (the instrument shows only the site), is that `vec4(var)`
   of the undef float reaches the colour as a converted node and not as a
   `Val::Undef` lane. The fix is to map the conversion of an undef to the
   undef in the pass, and let the `if` fold to its defined side.

**Sites it removes:** `nir_fs.rs:5578`, `:5603`, `:5619` and `:5654` (the
`gone` cases only) and `:2423` (the `*_never` discards). `nir.rs:4947`,
`:4957`, `:5201`, `:5222` and `:6198` (the `gone` cases only). The sites
stay for the select and branch cases of `mali-deqp-ctrlflow.md` steps 2 to 6.

**Cases (host forecast).** 52 must-pass cases have a first refusal Mesa
leaves no trace of: 47 `ControlFlow` and 5 `DiscardForm`. 51 are Fail and
1 NotSupported (`discard.dynamic_loop_never`).

| Site | Cases |
|---|--:|
| `nir_fs.rs:5578` | 12 |
| `nir_fs.rs:5603` | 3 |
| `nir_fs.rs:5619` | 7 |
| `nir_fs.rs:5654` | 2 |
| `nir_fs.rs:2423` (discard) | 5 |
| `nir.rs:4947` | 10 |
| `nir.rs:4957` | 5 |
| `nir.rs:5201` | 1 |
| `nir.rs:5222` | 4 |
| `nir.rs:6198` | 3 |

- dEQP groups: `shaders.random.conditionals` 31 (vertex, fragment and
  combined), `fragment_ops.interaction.basic_shader` 8, `shaders.discard`
  5 (`basic_never`, `function_never`, `static_loop_never`,
  `function_static_loop_never`, `dynamic_loop_never`), `shaders.linkage` 4
  (`varying_4` to `varying_7`), `shaders.random.all_features` 3,
  `shaders.operator.sequence` 1 (`affect_ternary`).
- What each case meets next: the measure's `next_family` (the other
  stage's refusal) is empty for 42 of the 52. The other 10 meet
  `ControlFlow` again (5), the colour lane (2), `Arithmetic`, `ConstantFold`
  and constop (1 each). A refusal later in the same stage cannot be seen
  before the step. Of the eight references folded by hand, 4 are
  byte-equal, 2 meet RL1, 1 meets `FsOperandForm` and 1 meets
  `Arithmetic`. **Forecast: 52 first refusals lifted, at most 42 cases
  compiling.** The 15 of the 52 that hold a swizzled varying likely need
  RL1 too. The 6 `shaders.loops.*_iterations.only_continue_fragment` cases
  (NotSupported, `:6759`) are `gone` in Mesa too, because their loop body
  is only `continue`. They are the loop steps' cases (`loops2-refs`), not
  this step's.
- **Byte-compare after the commit:** `mali_deqp_cfdead` (`fs_*`, `vs_*`
  move to `MATCHED`), and `mali_deqp_ctrlflow`'s eight cf-dead references
  together with the others of section 2: `posvary_unwritten_declared_dead_read`,
  `intval3_rsg_condcomb80_fs`, `fsint2_rnd_cond9`. Any of these that then
  meets another text keeps its `REFUSALS` entry with the new text.

## 6. Step RL (the reordered lanes): train 4 commit 2

**What Mesa emits.** The varying is loaded whole (`LD_VAR_BUF_IMM.v4`) and
converted **in its own pairs**: `V2F32_TO_V2F16 r0, r0^, r1^` (x, y) and
`r1, r2^, r3^` (z, w). The order is never changed at the conversion. The
operations then read halves by source swizzle, one result lane each where
the lanes cross words, and `MKVEC.v2i16` packs the results' low halves into
the colour words:

- `rl_add_vec4`: four `FADD.v2f16`, for example `r4 = r0.h11 + r3.h11`
  (v_in0.y + v_in1.w) and `r3 = r1.h00 + r3.h00` (v_in0.z + v_in1.z). Then
  `MKVEC.v2i16 r2, r4^.h0, r3^.h0` and `MKVEC.v2i16 r1, r1^.h0, r0^.h0`.
  `ATEST` reads `r0.h0`, the alpha lane.
- `rl_minus_vec4`: `FMA.v2f16 r0, r0^.h10, 0x38000000.h11, 0x0.neg` (the
  swapped pair as one operand, `.h10`), the second word in order, then
  `MKVEC.v2i16 r2, r0.h0, r1.h0` and `r0, r1^.h1, r0^.h1` (one half of
  each word). The `- x * 0.5 + 0.5` is `FADD.v2f16 .neg` of the packed words.
- `rl_swizzle`: two packed float varyings, each read by `.h00`/`.h11`
  broadcast in `FMA.v2f16` with the uniform halves, and `MKVEC.v2i16` of
  the results.
- `rl_mat2_sub`: `FADD.v2f16` of each converted column pair with the
  uniform words, then `MKVEC.v2i16 r0, r0^.h0, r1^.h0` (x of column 0, x of
  column 1) and the sum `r0.h11 + r1.h11` beside the constant alpha
  (`MKVEC.v2i16 r1, r2^.h0, u3.w0.h0`).

**The lowering** (`nir_fs.rs` `vectorise` and `select`'s `conv_form`):

1. At `:11057`, instead of `Err(ColourForm)`, build
   `Leaf::Pack { x, y }` of two lone lanes. Each lane is
   `Leaf::Half { k, h: i / 2, swz: H00 | H11 }` of its own pair's
   conversion, made by `lone_node` as the existing `varying_merged` arm
   does. This is that arm without its merged-lane condition. The load stays
   converted in its own pairs, so `conv_form` records the pair, never a
   lone conversion, for every lane of the load.
2. An operation whose two operands are such packs is computed per result
   lane as `.v2f16` on the broadcast halves. `select` then emits Mesa's
   `MKVEC.v2i16` of the results' `.h0`. An operand that is a swapped pair
   of one word is `.h10` (`Leaf::Half` with `Swz::H10`, which already
   exists) and keeps the pair operation (`rl_minus_vec4`'s first `FMA`).
3. The `ATEST` source is the alpha's half wherever the pack put it
   (`r0.h0` in `rl_add_vec4`, `r0.h1` in `rl_minus_vec4`), as `select`
   reads it for the existing packs.
4. Which result Mesa packs first (`MKVEC r2, r4^.h0, r3^.h0`, with the
   (y, z) lanes' results in the register order of the `FADD`s) is the
   scheduler's order of section 2.1 of `mali-fclane-lowering.md`. Check it
   against all four references before generalising.

**Cases (host forecast).** 195 must-pass cases have their first refusal at
`:11057` (191 Fail, 4 NotSupported). They split in two:

- **RL1, a swizzled varying as an operand or as the colour** (111, all
  Fail): `shaders.operator.binary_operator` 48, `common_functions` 27,
  `unary_operator` 20, `sequence` 4, `geometric` 4, `angle_and_trigonometry`
  2, `shaders.random.swizzle` 3, `shaders.struct.local` 1,
  `random.conditionals` 1, `fragment_ops.interaction` 1. The proxy forecast
  recompiles each pair with every swizzle of a varying removed
  (`v_in0.yzwx` becomes `v_in0`, which is what the walk sees once the lane
  order is lowered; scratch, not committed). **62 compile**:
  binary_operator 30, unary_operator 20, common_functions 4, geometric 3,
  angle_and_trigonometry 2, sequence 2, random.swizzle 1. The rest meet
  `FsOperandForm` (30), the colour lane at `:11179` (11, a constant before
  a lane), the link's "varying at different offsets" (4), `MixedPrecision`
  (3) and `:11057` again (1).
- **RL2, a matrix varying's columns or an indexed vector** (84; 80 Fail, 4
  NotSupported): `shaders.matrix` 46 (`dynamic_*_mat2/mat3/mat4_*` of every
  operator), `shaders.indexing` 26 (`vector_subscript`, `matrix_subscript`,
  `varying_array`), `shaders.random` 8, `shaders.struct` 3,
  `fragment_ops.interaction` 1. This needs the same `Pack` across columns
  of one load (`rl_mat2_sub`). No source proxy exists, because removing
  the indexing changes the lanes, so there is no count before the step.
  The pinned `deqp_fclane_vec3_local` and `deqp_idxloc_fs_mat3_dl_sl` are
  RL2's mat3 references.
- **Byte-compare after the commit:** `mali_deqp_cfdead` (`rl_*` to
  `MATCHED`), `mali_deqp_fclane` (`vec3_local`), `mali_deqp_idxloc`
  (`fs_mat3_dl_sl`), and `mali_deqp_fclane`/`_mixcol`/`_colourpair`
  unchanged (the `varying_merged` arm must give the same words).

## 7. Order and overlaps

- CD first, then RL. They touch different functions: CD a new pass before
  the walks, plus `nir.rs`'s `vs_const_cond` moved into it. RL touches
  `vectorise` and `select`. 15 of CD's cases need RL afterwards.
- RL edits `vectorise`, as do fix C, load_mat3, mat4_pair_unread and step
  4a's residue (schedule 5.5a). Rebase RL after train 3c's O4-O8, which
  also edit `vectorise`.
- CD's constant folder is `mesa_fold.rs`. It must be the only folder, as
  constop/negzero/cfold require.

## 8. Caveats

- HOST-ONLY: a case that compiles on the host is not a board pass.
- Only the 10,597 board-refused cases of the slices are measured.
- The CD-a/CD-b split of a case is read from its source. The `gone` class
  reads Mesa's final words only, so the two rules are counted together.
- The RL1 proxy is an upper bound on what the step lifts alone (a removed
  swizzle can also hide an operand form), and it is not a byte-compare.
- The site line numbers are `297d118c`'s and drift with the files.
