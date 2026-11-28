<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the `Defect(IrIndex)` cases of the Verdin iMX95 run, at the integration tip

**HOST-ONLY.** Plan #387 item `irdefect-design` (issue #7, ROADMAP stage 5).
Tip **`6e9e515c`**; every line number in this note is that commit's. No
compiler change: this note, the references `tools/mali/testdata/deqp_irdef_*`
and their test `tests/mali_deqp_irdef.rs` are the whole change. It continues
`mali-deqp-irindex.md` (causes 5.1 to 5.8 at `670913e1`) and
`mali-deqp-irindex23.md`; it does not edit them.

The text: `Valhall back end defect: an invariant of the pipeline did not hold
(IrIndex)` (`MaliError::Defect(Defect::IrIndex)`). It is not a refusal by
name: a pass looked up a lane, a load or a value an earlier pass should have
made, and did not find it.

## 1. Method

- **Board list.** The six slices of the Verdin iMX95 (Mali-G310) whole-tree
  run at `e63ab28e` (`tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e/refusals.tsv`)
  have **216** cases whose text is `Defect(IrIndex)`: 206 refused in the
  fragment shader, 10 through a program link (the eight
  `constant_expressions.builtin_functions.trigonometry.{acos,asin}_*_vertex`,
  `preprocessor.basic.null_directive_vertex` and
  `preprocessor.predefined_macros.file_vertex`). The plan's count, 207 lines,
  was not reproduced from these files; this note uses the 216.
- **Host measure** (4 cores, CTS VK-GL-CTS `93bca018` with
  `cts-yakogl-target.patch`, the `HOST_DEQP_CTS` tree of `tools/cts/deps.toml`):

  ```sh
  tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" \
      --out "$TMPDIR/measure" --jobs 3 --chunk 40 \
      --slice tools/cts/log/2026-11-24-imx95-gles2-S1a-e63ab28e ... (the six slices) \
      --cases <the 216 case names>
  ```

  The tip's `capi/Cargo.toml` names `vivante-st` twice (lines 41 and 85), a
  duplicate key that stops cargo 1.99 from loading the workspace. The second
  entry was removed in the working tree for the measure and the tests and
  restored before the commit (outside this item's paths; reported to the
  planner).
- **Sites.** Each of the 56 programs with a host refusal was compiled through
  `tests/mali_ref::compile_pair` in a scratch worktree of the tip, not
  committed. There every `Defect::IrIndex` construction was made lazy
  (`ok_or` to `ok_or_else`, the eager `let defect = ...` of `src32`,
  `motion.rs`, `vec16.rs` and five `nir_fs.rs` functions removed) and
  recorded its `Location` and a backtrace, so the last record of a refused
  compile is its site. `eprintln` at the sites printed the lanes and loads
  named below.
- **References.** Mesa 26.2.1's G610 drm-shim
  (`tools/mali/drm_shim_ref.sh build "$CONNECTED_DEPS_DIR/downloads/mesa-26.2.1.tar.xz" "$CONNECTED_DEPS_DIR/mesa-shim"`),
  each pair verbatim from the measure's `programs/`:

  ```sh
  tools/mali/deqp_ref.sh <case>.vert <case>.frag tools/mali/testdata/deqp_irdef_<shape> --instrument shim
  tools/mali/nir_refs.sh --build-dir "$CONNECTED_DEPS_DIR/mesa-shim" 'deqp_irdef_*'
  ```

  The G310 needs no reference of its own (`mali-g310-vs-g610.md`).

## 2. Result: 216 board cases, 42 still `Defect(IrIndex)` on the host

| At `6e9e515c` (host) | Cases |
|---|--:|
| compile and link (no refusal on the host) | 160 |
| `Defect(IrIndex)`, four root causes (section 3) | **42** |
| mixed precision (`a fragment shader that mixes highp and mediump ...`) | 7 |
| algebraic rewrite (`a shape Mesa's algebraic rules rewrite without a reference ...`) | 7 |
| **Total** | **216** |

The 160 are the shapes the trains fixed since `e63ab28e` (`65a9ba11`: the
unread lane of a highp pair, `pair_hi_unread` / `pair_lo_unread`; `85ef7ea6`:
`b2f16_pair`, `val16_abs_fold`): `shaders.swizzles.vector_swizzles` 108,
`shaders.conversions` 29 (`vector_to_scalar` 12, `vector_to_vector` 9,
`vector_combine` 4, `matrix_combine` 4), `constant_expressions...trigonometry`
8, `shaders.matrix` 8 (the `dynamic_highp_mat4_mat4` and `_assign` forms, 4, and
`{add,sub}.dynamic_{lowp,mediump}_mat3_float`, 4), `preprocessor` 4,
`functions.datatypes` 2, `state_query.shader` 1. Whether their words are
Mesa's is the compiled audit's question, not this note's.

The 14 named refusals are the ones `mali-deqp-irindex.md` 5 already lists as
past the defect: the six `highp_vec3_fragment` compares
(`deqp_irindex_load_vec3_cmp`) and `loops.for_uniform_iterations.basic_highp_float_fragment`
(mixed precision); `matrix.mul.{dynamic,uniform}_*_mat3_float` and
`matrix.mul.dynamic_{lowp,mediump}_mat4_float` (algebraic rewrite,
`deqp_irindex_mat3_uniform_conv`'s text).

## 3. The four root causes

| Group | Cases | Site (last `IrIndex`) | Root cause | Fix (file, function) | References |
|---|--:|---|---|---|---|
| **R1** | 20 | `nir_fs.rs:10146` (`index`, no `lane_of`) from `split_lanes` `:10491`, `split_sum`, `lane_tree` `:11820` | the mediump lane tree pairs a vector operation's two lanes when one reads a varying lane nothing reads | `nir_fs.rs` `lane_tree` (`:11816-11824`): pair only two live lanes, else each lane its own tree; `split_lanes` `:10491` answers `Ok(false)` | `deqp_irdef_mat4_{mat4,addassign,preinc,postdec,float_lowp}`; `deqp_irindex_mat4_pair_unread`; `deqp_irindex3_*` |
| **R2** | 18 | `nir_fs.rs:13491` (`src32`, `lv.raw` of load 1) from `select_highp`'s node walk `:15511` | `move_loads` does not see a two-lane `F2f16` read its second lane, so the compact `mat3` load holding it sinks past the conversion | `nir_fs.rs` `move_loads` (`reads`, `:13376-13377`): the node walk's arity rule (`:15508`) | `deqp_irdef_mat3_{vertex,combine,linkage,datatypes}`; `deqp_irindex_load_mat3` |
| **R3** | 2 | `nir_fs.rs:13498` (`src32`, a 16-bit ALU node's `val`) from `select_highp` `:15746` | the colour lanes `c` are read again from the stores after the passes only for `multi16`; `vectorize16` replaced a one-lane `B2f` by a lane of a two-lane one, and `c` still names the dead node | `nir_fs.rs` `select_highp` (`:15067-15072`): refresh `c` when `pairs16` too | `deqp_irdef_not_bvec{3,4}` |
| **R4** | 2 | `nir_fs.rs:16572` (`operand`, `P::Op` / `Leaf::IntWide` `val`) from `op`, `select` | an int uniform pair under `fneg` has no value (`mali-deqp-irindex.md` 5.8) | `nir_fs.rs` `op` (operand set-up): build an `IntWide` operand's value as the colour root does | `deqp_irindex2_op_val` (no new reference) |
| **Total** | **42** | | | | |

### R1: an unread lane paired in the mediump lane tree (20 cases)

- **Cases.** `shaders.matrix.{add,sub}.dynamic_{lowp,mediump}_mat4_float_fragment`
  (4), `{add,sub}.dynamic_{lowp,mediump}_mat4_mat4_fragment` (4),
  `{add,sub}_assign.dynamic_{lowp,mediump}_mat4_fragment` (4),
  `{pre,post}_{increment,decrement}.dynamic_{lowp,mediump}_mat4_fragment`
  (8). Their highp `mat4_mat4` and `_assign` siblings (4 of the 216) compile
  on the host.
- **Shape.** `gl_FragColor = vec4(res[0].xyz + res[1].yzw + res[2].zwx + res[3].wxy, 1.0)`
  of a mediump or lowp `mat4` (`v_mat4 + v_coords`, `v_mat4 + u_in1`,
  `res += u_in1`, `++tmpValue`, `tmpValue--`). Column 2 is read at `z, w, x`,
  column 1 of the `mat4_mat4` forms at `y, z, w`: one lane of an aligned pair
  is never read (`DIAG loc=2 c=1 lane_of=[Some((2,0)), None, Some((2,2)),
  Some((2,3))]`; `loc=1 c=0 lane_of=[None, ..]`).
- **Root cause.** `lane_tree` takes a lane of a vector operation
  (`Fs::pair_lane`: `nir_lower_alu_width` keeps the `fadd`'s lanes in pairs)
  and asks `split_sum`, then `pair_node`, about the aligned pair
  `(lanes[2h], lanes[2h+1])`, here `(res[2].x, res[2].y)`. The varying layout
  gives `lane_of` only to the lanes the colour reads, so `res[2].y`'s input
  lane `v_mat4[2].y` has no slot. `split_lanes` (`:10491`) looks both lanes
  up with `?`, and the "no slot" error is the defect, where the question "is
  this a split sum" should answer no. With `split_lanes` answering `Ok(false)`
  (checked in the scratch copy) the same lookup fails one level down, in
  `vectorise`'s input-lane arms (`:11014-11015`, the `Lanes` arm `:11076`,
  the whole-load arm `:11085-11087`, through `pair_node` `:11791`), always
  for the same unread lane: the tree should not have paired it.
- **What Mesa does** (`deqp_irindex_mat4_pair_unread`, the same shape at
  mediump): column 2 is loaded whole (`LD_VAR_BUF_IMM ... slot2.v4 @r4:r5:r6:r7`),
  its lane x converted alone (`FADD.f32 r4.h0, r4^, 0x0.neg`), its `z, w` as a
  pair (`V2F32_TO_V2F16 r5, r6^, r7^`), and the sum's lane x is a one-lane
  operation (`FADD.v2f16 r4, r4^.h00, r8.h00`). Mesa's DCE removed the dead
  lane before `nir_opt_vectorize`, so there is no pair to make.
- **Fix.** `lane_tree` (`nir_fs.rs:11816-11824`): take the pair branch only
  when both lanes are live (every input lane each one reads has a `lane_of`
  slot, as `convert_inputs16` counts live readers since `65a9ba11`), else
  fall through to the lane's own tree. `split_lanes` (`:10491`) answers
  `Ok(false)` for a lane without a slot, so the question cannot raise the
  defect again. Check the words against the five `deqp_irdef_mat4_*`
  references, `deqp_irindex_mat4_pair_unread` and the three hand-written
  `deqp_irindex3_*`.
- **Once fixed:** not measured (the scratch trial stopped at the
  `vectorise` arms). Whether the mediump forms then compile byte-equal or
  meet the mixed-precision or operand-form text is open.

### R2: a compact `mat3` load sunk past the conversion that reads it (18 cases)

- **Cases.** `shaders.conversions.matrix_combine.*_to_mat3_vertex` (6) and
  `vec3_vec3_vec3_to_mat3_fragment` (1), `matrix_to_matrix.{mat2_to_mat3,mat3_to_mat3,mat4_to_mat3}_vertex`
  (3) and `{mat3_to_mat2,mat3_to_mat3,mat3_to_mat4}_fragment` (3),
  `scalar_to_matrix.{bool,float,int}_to_mat3_vertex` (3),
  `functions.datatypes.float_mat3_fragment` (1), `linkage.varying_type_mat3`
  (1). The twelve `_vertex` cases share one fragment shader (`varying mat3
  out0` against a `mat3` uniform by `isOk`): `deqp_irdef_mat3_vertex`.
- **Root cause.** The `mat3` varying is laid out as three compact loads of
  4, 4 and 1 lanes (`lane_of[1] = [(0,3), (1,0), (1,1)]`: column 1 is load
  0's lane 3 and load 1's lanes 0 and 1). Column 1's `x, y` become one
  two-lane `F2f16` (`src: [Lane(1,0), Lane(1,1)]`, width 2,
  `V2F32_TO_V2F16`), whose first source is in load 0 and second in load 1.
  `select_highp` gives that node arity 2 (`:15508`). `move_loads`, which
  sinks each load to its first reader (`nir_opt_move`), reads only
  `op.arity()` = 1 source (`reads`, `:13377`), so it does not see the
  conversion read load 1 and moves load 1's event past it. The node walk then
  converts a lane of a load not yet emitted (`DIAG raw=[[true; 4], [false;
  4], [false; 4]]` at `id=53 F2f16 width 2`). Not `deferred`: the three loads
  have `deferred=false`. At `670913e1` the same cases failed at the same
  `lv.raw` site (`mali-deqp-irindex.md` 5.3), there read as "never emitted".
- **Fix.** `move_loads` (`nir_fs.rs:13376-13377`): count a two-lane `F2f16`'s
  two sources, the rule of `select_highp`'s walk (`:15508`). Checked in the
  scratch copy: the four `deqp_irdef_mat3_*` references then **compile, and
  differ** from Mesa's words.
- **Once fixed: `DIFFERS`.** `deqp_irdef_mat3_vertex` is 34 words, Mesa's
  30. Mesa converts each compact load by aligned pairs (`V2F32_TO_V2F16 r1,
  r2^, r3^` of load 0's `z, w`), and builds the column that spans two loads
  with `MKVEC.v2i16 r1, r1^.h1, r3.h0`; ours converts load 0's `z` alone and
  pairs `(load0.w, load1.x)` across the loads (`V2F32_TO_V2F16 r2, r3^, r4^`).
  The `all(lessThanEqual(diff, vec3(eps)))` also differs: Mesa reduces the
  `FMAX` tree to one lane and makes one `FCMP_OR.v2f16.ge.f1`; ours makes two
  `FCMP_OR.v2f16.ge.m1` and `LSHIFT_AND` the masks. Both are lowering steps
  past the defect (the vectoriser's compact-load pairs and the `all` of a
  `vec3` compare), for the item that fixes R2.

### R3: the colour lanes not refreshed after `vectorize16` (2 cases)

- **Cases.** `shaders.operator.bool_compare.not.bvec3_fragment` and
  `bvec4_fragment`. They were in `b2f16_pair` (`mali-deqp-irindex.md`); the
  `85ef7ea6` fix left these two at a different site.
- **Shape.** `vec3(not(greaterThan(v_in0.zxy, vec3(0.0))))` into
  `color.xyz` (no default float precision in the fragment shader, a mediump
  varying: the highp walk with fp16 lanes).
- **Root cause.** `select_highp` makes the colour's stores from `c =
  f.colour` (`:14979-14992`), runs the passes, and `vectorize16` (`:15049`)
  pairs the fp16 `B2f`s: node 8 `B2f` width 2 over the pair compare replaces
  node 9 (`B2f` of `Lane(5, 1)`), and the stores now read `Lane(8, 0)` and
  `Lane(8, 1)`. `c` is read again from the stores' sources only `if
  multi16(f)` (`:15069-15073`), so `c[2]` still names node 9, which is in no
  order and has no value. The colour words then read it (`src32` from
  `:15746`).
- **Fix.** `select_highp` (`nir_fs.rs:15069`): `if multi16(f) || pairs16`.
  Checked in the scratch copy: **`deqp_irdef_not_bvec3` and `not_bvec4` are
  then byte-equal with Mesa** (13 and 12 words). The other references of this
  note keep their outcome with the change.

### R4: an int uniform pair under `fneg` (2 cases)

- **Cases.** `shaders.random.scalar_conversion.fragment.56` and
  `combined.56` (one fragment shader).
- **Root cause and fix.** Unchanged from `mali-deqp-irindex.md` 5.8 and
  `mali-deqp-irindex23.md`: the site is `operand`'s `| P::Op(..) =>
  val[x].ok_or(Defect::IrIndex)?` arm, now `nir_fs.rs:16572`, from `op` and
  `select`. The reference is `deqp_irindex2_op_val`
  (`tests/mali_deqp_irindex2.rs`), the case's own pair; no new one is drawn.

## 4. The references and their test

`tests/mali_deqp_irdef.rs` pins the eleven references:

| Reference | dEQP case (verbatim pair) | Group | `fs.fragment` words | At the tip | With the group's fix (scratch) |
|---|---|---|--:|---|---|
| `mat3_vertex` | `conversions.matrix_to_matrix.mat3_to_mat3_vertex` | R2 | 30 | IrIndex | compiles, differs |
| `mat3_combine` | `conversions.matrix_combine.vec3_vec3_vec3_to_mat3_fragment` | R2 | 30 | IrIndex | compiles, differs |
| `mat3_linkage` | `linkage.varying_type_mat3` | R2 | 30 | IrIndex | compiles, differs |
| `mat3_datatypes` | `functions.datatypes.float_mat3_fragment` | R2 | 29 | IrIndex | compiles, differs |
| `mat4_mat4` | `matrix.add.dynamic_mediump_mat4_mat4_fragment` | R1 | 35 | IrIndex | not measured |
| `mat4_addassign` | `matrix.add_assign.dynamic_mediump_mat4_fragment` | R1 | 35 | IrIndex | not measured |
| `mat4_preinc` | `matrix.pre_increment.dynamic_mediump_mat4_fragment` | R1 | 38 | IrIndex | not measured |
| `mat4_postdec` | `matrix.post_decrement.dynamic_mediump_mat4_fragment` | R1 | 48 | IrIndex | not measured |
| `mat4_float_lowp` | `matrix.add.dynamic_lowp_mat4_float_fragment` | R1 | 37 | IrIndex | not measured |
| `not_bvec3` | `operator.bool_compare.not.bvec3_fragment` | R3 | 13 | IrIndex | byte-equal |
| `not_bvec4` | `operator.bool_compare.not.bvec4_fragment` | R3 | 12 | IrIndex | byte-equal |

`refusals_pinned` (not ignored) holds each reference's whole refusal text at
the tip; each byte-compare is `#[ignore]`d with its group. The fix of a group
moves its references to `LOWERED` or `DIFFERS` in the same commit, drops the
`#[ignore]`s and updates this note; `refusals_pinned` fails until it does.
The older references of the same causes stay where they are:
`deqp_irindex_mat4_pair_unread` and `deqp_irindex3_*` (R1),
`deqp_irindex_load_mat3` (R2, `mat3_to_mat3_fragment`), `deqp_irindex2_op_val`
(R4).

## 5. Order of work

By case count: R1 (20), R2 (18), then R3 and R4 (2 each). R3 is one line and
is byte-equal with it, so it can go first in any train. R2's defect fix is
one line too but leaves `DIFFERS`: it should land with the compact-load
pairing and the `vec3` `all` that make its words Mesa's, or the 18 cases move
from a defect to compiled-not-Mesa's, which the project's rule does not
accept. Every fix site is in `src/shaderc/online/mali/nir_fs.rs`, held by
the compiler trains (3b, 3c, 4).

## 6. Caveats

- HOST-ONLY: compile and link outcomes, never a render verdict.
- The sites were read with every `IrIndex` construction made lazy; the
  "with the fix" columns are scratch trials, not committed changes.
- R1's fix is a direction checked against the Mesa reference's words, not run
  to a compile.
- The 216 board cases are from `e63ab28e`; the host measure is at `6e9e515c`.
  The lowering items in review (plans #367, #377, #402) were not applied.
