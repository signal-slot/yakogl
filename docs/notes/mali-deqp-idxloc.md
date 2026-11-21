<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall v10): the indexed-array cases behind the loop steps S2, S7 and S11, and their next refusal

**HOST-ONLY.** Plan #387 item idxloc-ref (issue #7, ROADMAP stage 5). This is
a study with references. Nothing under `src/` changed. It adds
`tools/mali/testdata/deqp_idxloc_*` (14 references), `tests/mali_deqp_idxloc.rs`
(it pins each reference's refusal) and this note. Every count is a compile or
link outcome on the host at tip `4714317e`, never a render verdict.

Read with: [`mali-loops-lowering.md`](mali-loops-lowering.md) (steps S2-fs, S2-vs,
S7 and S11; section 8 asks this question),
[`mali-deqp-loops.md`](mali-deqp-loops.md) sections 9.2-9.7 (the rows and the
`deqp_loops2_*` references), [`mali-indexed-locals.md`](mali-indexed-locals.md).

## 0. Result in brief

- **The 156 cases:** 146 dEQP `shaders.indexing.*` cases (fragment 84 on
  `LoopMediump`, vertex 62 on `LoopVertex`) and 10 other S7 cases (`discard` 4,
  `return` 2, `struct` 4). The 146 were measured again at the tip, case by
  case. They match the nine indexing rows of `mali-deqp-loops.md` 9.2 exactly
  (section 2).
- **None of the 156 reaches `IndexedLocal`.** The note of the loop steps
  expected `IndexedLocal` (`mali-loops-lowering.md` section 8). It is not the
  next refusal in any of the four measures of section 1. Some of these
  shaders read a local array with a run-time index (`arr[ui_zero]`, the
  `dd` reads). The fragment walk takes those reads today. In a mediump
  shader the next refusal is the arithmetic text, `Arith`.
- **The next refusal, fragment** (the loop folded by hand, U in section 1):
  49 compile (but section 4.3 shows they do not yet give Mesa's words), 23
  stop at `FragColorLane`, 10 at `Arith`, 2 at `VaryingForm`. A run-time
  guard that stays an `if` (G, S) stops all 84 at `ControlFlow`. So the
  guard selects must come from the loop lowering itself, not from the
  generic `if` walk.
- **The next refusal, vertex:** all 62 stop at `UniformType` ("a uniform
  other than a float, vec2, vec3, vec4 or mat4", the `mediump int` bound or
  index in the vertex shader) while the bound is read at run time. With
  the bound folded they stop at `ArithConstant` ("a constant product in a
  shape Mesa's algebraic rules move or fold", the `coords = coords * 0.5`
  chain). None compiles until the loop step, the vertex int uniform
  (intval-vs16) and the constant-product lowering have all landed.
- **Mesa's mechanism** (146 = 86 + 8 + 48 + 4):
  - 86 keep the array in registers, with no loop. 36 are folded (a store
    whose guard vanishes, no select). 50 are `ICMP_OR` + `MUX.v2i16`/`CSEL`
    ladders.
  - 8 use branches with no loop: the 4 uniform-array loads, and the 4
    dynamic reads of a varying array (`vs_idx_var_vec4_dd`'s fragment
    shader).
  - 48 keep a loop: the vector subscripts, with an `eq`/`lt` ladder inside
    the loop.
  - 4 put the array in thread-local scratch (`STORE.i16.force ...
    thread_local_pointer`) and keep the loop that reads it.

## 1. Method

- **The cases.** No `.qpa` of the measure is in the tree, and this host cannot
  reach the CTS source, so the shaders were rebuilt from the generator's
  templates (`es2fShaderIndexingTests.cpp`, VK-GL-CTS `93bca01`).
  - The rebuild (a scratch Python script under `$CONNECTED_DEPS_DIR/idxloc`,
    not committed) writes all 208 `shaders.indexing.*` cases whose write or
    read is a `dynamic_loop`. The 208 are: `tmp_array` 64 (with the
    40-element `const_write` form), `uniform_array` 8, `varying_array` 28,
    `vector_subscript` 66, `matrix_subscript` 42.
  - **27 rebuilt pairs are byte-identical to the verbatim `.qpa` copies
    already in `tools/mali/testdata`**: every `deqp_loops2_*idx*`,
    `deqp_loops_*idx_dyn`, `deqp_unif2_vs_int_uniform`,
    `deqp_varylink2_loop_{uniform,bcast}` and
    `deqp_shapes/loops_fs_kept_for_index`.
  - Three forms have no verbatim copy to check against: a vector's
    component write and read (`tmp.x = coords.x`), and float and vec3
    varying arrays. Each reference built from them says so in its
    `.license`.
- **The four measures**, per case, all with `tests/mali_ref`'s `compile_pair`
  (compile, relink, link) in a scratch test (not committed):
  - **orig**: the case's own pair. This gives the first refusal at the tip.
  - **U** ("unrolled"): every `for (int i = 0; i < ui_N; i++)` replaced by
    its `N` passes with the counter a literal (N is the array length,
    `ui_four` = 4). This is the loop after a lowering that folds the counter
    and drops the guards. It is right where Mesa's guards vanish, and a
    lower bound where Mesa keeps them.
  - **G** ("guarded"): Mesa's partial unroll by hand. Each pass sits in
    `if (k < ui_N) { ... }`, nested in the previous pass's `if`
    (`nir_opt_loop_unroll.c` `partial_unroll`, read).
  - **S** ("sequential"): the same guards, one after the other instead of
    nested.

  The refusing stage was told apart by compiling the vertex shader with a
  trivial fragment shader, then the other way round.
- **A throw-away build** (reverted, not committed) removed the mediump gate
  of `walk_kept_loop` (`nir_fs.rs:5182`, `if !prec.highp`), to see where the
  kept walk stops next for the fragment shapes Mesa keeps as loops.
- **References.** Mesa 26.2.1's drm-shim was built into
  `$CONNECTED_DEPS_DIR/mesa-shim` (`tools/mali/drm_shim_ref.sh build`; section
  1 of `mali-deqp-irindex.md`). Each reference was drawn with

  ```sh
  MESA_SHIM_WORK="$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/deqp_ref.sh \
      <case>.vert <case>.frag tools/mali/testdata/deqp_idxloc_<name> --instrument shim
  ```

  To check that build, four existing references were drawn again with it:
  `gm2_cf_ifelse_fs`, `deqp_loops2_fs_idx_tmp_float_dd`,
  `deqp_loops2_vs_idx_vec_w_dynsub` and `deqp_loops2_fs_idx_tmp_vec4_cd`. Each
  `expect.json` came out byte-equal to the committed one. Each reference
  reads Mesa's program from its `expect.json` disassembly: the instruction
  count, the backward `BRANCH*` (a kept loop), the forward branches,
  `thread_local_pointer` (scratch), and `MUX`/`CSEL` with
  `ICMP`/`FCMP` (ladders). The G310 needs nothing separate
  (`mali-g310-vs-g610.md`).

## 2. The 156 cases

| part | cases | how they were measured |
|---|---:|---|
| `shaders.indexing.*`, fragment, `LoopMediump` | 84 | rebuilt, measured case by case |
| `shaders.indexing.*`, vertex, `LoopVertex` | 62 | rebuilt, measured case by case |
| `discard.dynamic_loop_{always,uniform,dynamic,texture}` | 4 | `deqp_loops2_fs_disc_*` (the cases' own pairs) |
| `return.return_in_dynamic_loop_{dynamic,never}_fragment` | 2 | `deqp_loops2_fs_ret_*` |
| `struct.{local,uniform}.dynamic_loop_*` (S7's 4) | 4 | **not measured**: the four cases are not named in the loop notes, and their sources are not on this host. `deqp_loops2_fs_struct_local` (`struct.local.dynamic_loop_struct_array_fragment`, an S4 case: Mesa has no loop) was measured as the nearest shape |
| **sum** | **156** | |

The 146 per group (fragment / vertex): `tmp_array` 28 / 16, `matrix_subscript`
18 / 12, `uniform_array` 4 / 0, `varying_array` 4 / 16, `vector_subscript` 30
/ 18. These are the nine indexing rows of `mali-deqp-loops.md` 9.2 (at
`9ac31f30`), count for count. The other 62 of the 208 rebuilt cases stop at
another text before any loop at the tip. They are not in the 156 and belong
to other families:

| text | cases |
|---|---:|
| `UniformType16` | 10 |
| `UniformType` | 14 |
| `ConstOp` | 14 |
| `VaryLink` | 9 |
| `IntValue` | 5 |
| `Arith` | 5 |
| `UniformIndex` (uniform arrays in a vertex shader) | 4 |
| `PosVary` | 1 |

## 3. Shapes, Mesa's mechanism and the next refusal

Notation:

- In a case name, `w`/`r` is the write/read access: `s` static, `d`
  dynamic (`arr[ui_zero]`), `sl` static loop, `dl` dynamic loop (`i <
  ui_four`), `cd` the constant-written 40-element array.
- "Mesa" is read from the words of the references listed. `L` means a kept
  loop, `sel`/`cmp` the `MUX`/`CSEL` and `ICMP`/`FCMP` counts.
- U, G and S are the measures of section 1. "no-gate" is the throw-away
  build.

### Fragment (84)

| shape | cases | dEQP cases (`functional.shaders.indexing.`) | Mesa (references) | next refusal: U | G / S | no-gate |
|---|---:|---|---|---|---|---|
| **F1** loop write, constant reads: the guards vanish | 14 | `tmp_array.{float,vec2,vec3,vec4}_dynamic_loop_write_{static,static_loop}_read_fragment` 8, `matrix_subscript.{mat2,mat3,mat4}_dynamic_loop_write_{static,static_loop}_read_fragment` 6 | **folded**, registers, no select, no loop. `deqp_loops_fs_idx_dyn` 11 words, `deqp_idxloc_fs_tmp_vec2_dl_sl` 10, `deqp_idxloc_fs_mat3_dl_sl` 17 | compiles 8 (tmp), `FragColorLane` 6 (mat) | `ControlFlow` | `LoopForm` |
| **F2** dynamic-loop read (a guarded accumulation) | 25 | `tmp_array.*_{static,static_loop,dynamic_loop}_write_dynamic_loop_read_fragment` 12, `matrix_subscript.*_{static,static_loop,dynamic_loop}_write_dynamic_loop_read_fragment` 9, `varying_array.{float,vec2,vec3,vec4}_static_write_dynamic_loop_read` 4 | **registers, ladder**: per pass `ICMP_OR.s32.lt.m1 k, ui_four` with a `MUX.v2i16` against the old `res`, last `0 < ui_four` against 0, no loop (`deqp_idxloc_fs_tmp_vec3_sl_dl` words 10-27: 33 words, sel 12, cmp 5). Also `deqp_loops2_fs_idx_tmp_vec4_sd` 35, `deqp_loops2_fs_idx_tmp_float_dldl` 22, `deqp_loops2_fs_idx_mat4_sd` 43, `deqp_loops2_fs_idx_var_vec4_sd` 44, `deqp_idxloc_fs_var_float_sd` 23, `deqp_idxloc_fs_var_vec3_sd` 41 | compiles 12 (tmp 11, var 1), `FragColorLane` 11 (mat 9, tmp vec3 `sl_dl`, var vec3), `VaryingForm` 2 (var float, vec2) | `ControlFlow` | `ControlFlow` 18, `LoopForm` 7 |
| **F3** loop write, dynamic (uniform-index) read | 7 | `tmp_array.*_dynamic_loop_write_dynamic_read_fragment` 4, `matrix_subscript.*_dynamic_loop_write_dynamic_read_fragment` 3 | **registers, ladder** per read: `ICMP_OR.s32.lt.m1 ui_n, <k>` + `MUX.v2i16`, no `ui_four` (store guards vanish). `deqp_idxloc_fs_mat2_dd` 17 words (words 7-12), `deqp_loops2_fs_idx_tmp_float_dd` 38, `deqp_loops2_fs_idx_mat4_dd` 60 | `Arith` 7 | `ControlFlow` | `LoopForm` |
| **F4** uniform array, dynamic-loop read | 4 | `uniform_array.{float,vec2,vec3,vec4}_dynamic_loop_read_fragment` | **branches**: the guarded uniform loads stay nested `BRANCHZ.eq.reconverge` (6-7 forward branches), no loop. `deqp_idxloc_fs_uni_vec3` 35, `deqp_loops2_fs_idx_uni_{float,vec4}` 30/34 | compiles 4 | `ControlFlow` | `ControlFlow` |
| **F5** constant-written 40-element array, dynamic-loop read | 4 | `tmp_array.{float,vec2,vec3,vec4}_const_write_dynamic_loop_read_fragment` | **thread-local scratch, kept loop**: 40 `STORE.i16.force ... thread_local_pointer.w0, offset:2k`, one loop that loads `thread_local_pointer + 2i`. `deqp_idxloc_fs_tmp_float_cd` 98 words (43 scratch accesses), `deqp_loops2_fs_idx_tmp_vec4_cd` 141 | compiles 4 (not Mesa's: 7 words) | `ControlFlow` | `ControlFlow` |
| **F6** vector subscript in a kept loop | 30 | `vector_subscript.{vec2,vec3,vec4}_{direct,component,static_subscript,static_loop_subscript}_write_dynamic_loop_subscript_read_fragment` 12, `vector_subscript.*_dynamic_loop_subscript_write_{direct,component,static_subscript,dynamic_subscript,static_loop_subscript,dynamic_loop_subscript}_read_fragment` 18 | **kept loop with a ladder**: the write is `ICMP_OR.u32.eq` of the counter against byte constants into `MUX.v2i16`, the read an `lt` ladder. `deqp_idxloc_fs_vec2_dl_static` 22 (1 L), `deqp_idxloc_fs_vec3_component_dl` 23 (1 L), `deqp_loops2_fs_idx_vec_{w_direct,direct_w,static_w,w_dynsub}` 25-58 | compiles 21, `FragColorLane` 6 (vec3/vec4 `component`, `static_subscript`, `static_loop_subscript` writes), `Arith` 3 (`dynamic_subscript` read) | `ControlFlow` | `LoopForm` 18, `ControlFlow` 12 |

### Vertex (62)

The loop is in the vertex shader. Mesa's loop-free forms are the fragment
ones in the varying program, with the fp32 stores after them.

| shape | cases | dEQP cases | Mesa (references) | next refusal: U | G / S |
|---|---:|---|---|---|---|
| **V1** loop write, constant reads (folded) | 22 | `tmp_array.*_dynamic_loop_write_{static,static_loop}_read_vertex` 8, `varying_array.*_dynamic_loop_write_{static,static_loop}_read` 8, `matrix_subscript.*_dynamic_loop_write_{static,static_loop}_read_vertex` 6 | **folded**: `deqp_idxloc_vs_tmp_vec2_dl_sl` varying 23 words, `deqp_idxloc_vs_var_float_dl_s` 21 / fragment 12, `deqp_loops_vs_idx_dyn` 23. No select, no loop | `ArithConstant` 22 | `UniformType` |
| **V2** dynamic-loop write and dynamic-loop read | 11 | `tmp_array.*_dynamic_loop_write_dynamic_loop_read_vertex` 4, `varying_array.*_dynamic_loop_write_dynamic_loop_read` 4 (the fragment shader reads in a loop too), `matrix_subscript.*_dynamic_loop_write_dynamic_loop_read_vertex` 3 | **ladder**: `deqp_loops2_vs_idx_tmp_vec4_dldl` 50 (sel 14), `deqp_loops2_vs_idx_mat4_dldl` 58, `deqp_loops2_vs_idx_var_vec4_dldl` fragment 44 (sel 14) | `ArithConstant` 11 | `UniformType` |
| **V3** loop write, dynamic read | 11 | `tmp_array.*_dynamic_loop_write_dynamic_read_vertex` 4, `matrix_subscript.*_dynamic_loop_write_dynamic_read_vertex` 3, `varying_array.*_dynamic_loop_write_dynamic_read` 4 | **ladder** (tmp, mat: `deqp_idxloc_vs_mat2_dl_d` varying 30, sel 2; `deqp_loops2_vs_idx_tmp_vec4_dd` 73, sel 24). For a varying array the fragment shader's dynamic read is **branches** (`deqp_loops2_vs_idx_var_vec4_dd` fragment 114, 24 forward branches) | `ArithConstant` 11 | `UniformType` |
| **V4** vector subscript in a kept loop | 18 | `vector_subscript.{vec2,vec3,vec4}_dynamic_loop_subscript_write_*_read_vertex` | **kept loop with a ladder**: `deqp_idxloc_vs_vec3_dl_component` varying 33 (1 L), `deqp_loops2_vs_idx_vec_{w_direct,dldl,w_dynsub}` 36-61. `w_dynsub` also has a loop in `vs.position` (29 words) | `ArithConstant` 18 | `UniformType` |

The throw-away build changes only the fragment gate. The vertex cases stay
on `LoopVertex` in it.

### The other S7 cases (10)

| shape | cases | Mesa | next refusal: U | G / S |
|---|---:|---|---|---|
| `discard` in a kept loop | 4 | kept loop, 16-25 words (`deqp_loops2_fs_disc_*`) | `Discard` 3 ("a discard other than a highp fragment shader's `if (a < b) discard;`..."), `TexCoord` 1 (`dynamic_loop_texture`) | `Discard` 3, `ControlFlow` 1 |
| `return` in a kept loop | 2 | kept loop, 17/21 words (`deqp_loops2_fs_ret_*`) | `OperandForm` 2 ("a fragment shader operand of a form or in a place no reference draws") | `ControlFlow` |
| `struct` (S7's 4) | 4 | not drawn (section 2). The nearest, `deqp_loops2_fs_struct_local`, is a `CSEL.u32.eq` ladder, 22 words, no loop | proxy `FragColorLane` | `ControlFlow` |

These 10 index no local array, except the struct arrays. They are S7's
loop-control work, not an indexing step. They are listed for the 156 only.

## 4. What the measures say about the lowering

1. **`IndexedLocal` is not the next text.** U folds the counter in the source
   (`arr[0]`), so the front end sees constant indices. The S2/S7 lowering
   folds the counter in the walk instead. There the store is still an
   `ir::Op::StoreLocal` with `index: Some(v)` and `v` a constant. That fold
   (`mali-loops-lowering.md` S2-fs: "local arrays need the same fold") is
   what keeps `stored_lanes` (`nir_fs.rs:5118`) and the walks' indexed-local
   sites (`nir_fs.rs:5706`, `nir.rs:4182`, `:4620`) away. If the fold is
   missing, those sites give `IndexedLocal`. The measure shows that nothing
   **after** them refuses `IndexedLocal`.
2. **The guards.** The generic `if` walk refuses every guarded pass
   (`ControlFlow`), nested (G) or not (S). The partial unroll must emit
   Mesa's selects itself:
   - A guarded store becomes the stored value. The select's other arm is
     undefined, which is why F1 and V1 have no select.
   - A guarded accumulation becomes `ICMP_OR.s32.lt.m1 <k>, ui_four` and
     `MUX.v2i16` against the old value. Mesa's order runs from `4 <
     ui_four` (the pass past the array; its arm is a FAU constant) down to
     `0 < ui_four` (`deqp_idxloc_fs_tmp_vec3_sl_dl` words 10-27).
   - A guarded uniform-array load stays a branch (F4, `holds_look_up`).
3. **"Compiles" is not "Mesa's words" yet.** Of the 13 references whose case
   compiles under U:
   - `deqp_idxloc_fs_tmp_vec2_dl_sl` differs from Mesa only in the operand
     order of two `FMA.v2f16` (words 3 and 4: Mesa writes
     `FMA.v2f16 r1, 0x34000000.h11, r0, r1^`, the constant first; we write
     it second). That is the FMA operand placement of the constant-product
     and FMA families.
   - `deqp_loops_fs_idx_dyn` differs in the chain's fold: Mesa has 11
     words, we have 12.
   - The others miss the guard ladder or branches, so they are shorter than
     Mesa (as U predicts).
4. **Vertex order.** With the bound read at run time, the vertex `int`
   uniform comes first (`UniformType`, item intval-vs16's vs16-2). With it
   folded, the `coords * 0.5` chain comes first (`ArithConstant`,
   constprod K2-K5). Both stand between S2-vs/S11 and a compile.
5. **The kept walk.** Without its mediump gate it stops at `LoopForm` or
   `ControlFlow` for these shapes (F1 and F3 all `LoopForm`; F2, F4 and F5
   mostly `ControlFlow`). For F1-F4, which Mesa does not keep, that is the
   decision (A0) still saying "keep". For F6 it is the missing parts that
   S3 and S7 add. It never reaches `IndexedLocal`.

## 5. Proposed steps (one session each), ranked by cases

The forecast counts cases that leave the loop and indexing refusals, with the
refusal they meet next. It is HOST-ONLY and an upper bound: a case that
"compiles" under U still has to match Mesa's words (section 4.3), and the
board's pass count belongs to the board rerun.

| rank | step | cases | needs first | forecast |
|---:|---|---:|---|---|
| 1 | **IL-V** vertex partial unroll and vector loops (S2-vs 44 + S11 18) | 62 | loops-lower-2 (S8 for V4), intval-vs16 (vs16-2, the int uniform), constprod K2-K5 (`ArithConstant`) | 0 compile until all three have landed. Then unmeasured: the U chain stops at `ArithConstant` in all 62 |
| 2 | **IL-1** fragment partial unroll with the indexed-local fold and the guard ladder (S2-fs: F1, F2) | 39 | loops-lower-1's A0 (the decision) | 20 compile (F1 8, F2 12; byte-equal needs the FMA operand order of 4.3), 17 to `FragColorLane` (mat 15, tmp vec3 1, var vec3 1), 2 to `VaryingForm` |
| 3 | **IL-6** fragment vector subscripts in a kept loop (S7's `vector_subscript`: F6) | 30 | loops-lower-1's S3 (the mediump kept loop) | 21 compile, 6 to `FragColorLane`, 3 to `Arith` (the `dynamic_subscript` reads, as IL-3) |
| 4 | **IL-3** dynamic read of a loop-written array in a mediump fragment shader (F3) | 7 | IL-1 | 7 leave the loop text and stop at `Arith` (the `lt` ladder of `ICMP_OR` + `MUX.v2i16` per read). Lowering that ladder (the arith family's `bcsel`, A5) frees 7 + 3 (F6) |
| 5 | **IL-4** uniform-array loads under the guard (F4) | 4 | IL-1 | 4 compile (Mesa's form is branches, not U's straight code) |
| 6 | **IL-5** thread-local scratch arrays (F5) | 4 | S3 (kept loop) | 4. New mechanism (TLS stores and loads, the scratch size in the shader's descriptors); lowest |

Sum: 62 + 39 + 30 + 7 + 4 + 4 = 146. The 10 non-index S7 cases stay with S7
(section 3). Order of work: IL-1 with or after loops-lower-1's S2-fs, then
IL-3 and IL-4 (same walker), IL-6 with S7, IL-V after loops-lower-2,
vs16-2 and the constant-product steps. IL-5 last.

## 6. The references and the test

`tests/mali_deqp_idxloc.rs` pins the first refusal of each `deqp_idxloc_*`
pair at the tip (`refusals_pinned`). It checks that the list and the
directory agree (`references_listed`). It keeps the byte compares
`#[ignore]`d (`compares`). Four of the fourteen have a kept loop in Mesa
(`fs_tmp_float_cd`, `fs_vec2_dl_static`, `fs_vec3_component_dl`,
`vs_vec3_dl_component`): compare them with `CapEdit`.

| reference | case (`functional.shaders.indexing.`) | shape | Mesa words (pos / vary / frag) | refusal at the tip |
|---|---|---|---|---|
| `fs_tmp_vec2_dl_sl` | `tmp_array.vec2_dynamic_loop_write_static_loop_read_fragment` | F1 | 24 / 15 / 10 | LoopMediump |
| `fs_mat3_dl_sl` | `matrix_subscript.mat3_dynamic_loop_write_static_loop_read_fragment` | F1 | 24 / 15 / 17 | LoopMediump |
| `fs_tmp_vec3_sl_dl` | `tmp_array.vec3_static_loop_write_dynamic_loop_read_fragment` | F2 | 24 / 15 / 33 | LoopMediump |
| `fs_var_float_sd` | `varying_array.float_static_write_dynamic_loop_read` | F2 | 24 / 18 / 23 | LoopMediump |
| `fs_var_vec3_sd` | `varying_array.vec3_static_write_dynamic_loop_read` | F2 | 24 / 66 / 41 | LoopMediump |
| `fs_mat2_dd` | `matrix_subscript.mat2_dynamic_loop_write_dynamic_read_fragment` | F3 | 24 / 15 / 17 | LoopMediump |
| `fs_uni_vec3` | `uniform_array.vec3_dynamic_loop_read_fragment` | F4 | 24 / - / 35 | LoopMediump |
| `fs_tmp_float_cd` | `tmp_array.float_const_write_dynamic_loop_read_fragment` | F5 | 24 / - / 98 (1 L) | LoopMediump |
| `fs_vec2_dl_static` | `vector_subscript.vec2_dynamic_loop_subscript_write_static_subscript_read_fragment` | F6 | 24 / 15 / 22 (1 L) | LoopMediump |
| `fs_vec3_component_dl` | `vector_subscript.vec3_component_write_dynamic_loop_subscript_read_fragment` | F6 | 24 / 15 / 23 (1 L) | LoopMediump |
| `vs_tmp_vec2_dl_sl` | `tmp_array.vec2_dynamic_loop_write_static_loop_read_vertex` | V1 | 24 / 23 / 7 | LoopVertex |
| `vs_var_float_dl_s` | `varying_array.float_dynamic_loop_write_static_read` | V1 | 24 / 21 / 12 | LoopVertex |
| `vs_mat2_dl_d` | `matrix_subscript.mat2_dynamic_loop_write_dynamic_read_vertex` | V3 | 24 / 30 / 7 | LoopVertex |
| `vs_vec3_dl_component` | `vector_subscript.vec3_dynamic_loop_subscript_write_component_read_vertex` | V4 | 24 / 33 (1 L) / 8 | LoopVertex |

V2 has no new reference: `deqp_loops2_vs_idx_{tmp_vec4,mat4,var_vec4}_dldl`
cover its three groups.

## 7. Caveats

- The case set was measured at `4714317e`, not at `9ac31f30`. Its row counts
  equal 9.2's. Which cases make up a row is not proven equal, because 9.2's
  `measure.tsv` is not in the tree.
- U, G and S are source transforms, not the lowering. U is exact for F1 and
  V1, the forms where Mesa's guards vanish. For the others it is the data
  path alone: a case can still stop at whatever the lowering emits for the
  guards.
- Five of the rebuilt forms (section 1) have no verbatim `.qpa` copy to check
  against. A different dEQP line would change the reference's words, not
  the shape or the measured texts.
- The 4 S7 `struct` cases are counted, not measured (section 2).
