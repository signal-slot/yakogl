<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the dEQP-GLES2 arithmetic refusals (ARITH): shapes, Mesa references and a lowering map

**HOST-ONLY.** Plan #387 item `arith-ref` (a study and references: no change under
`src/`). Every case count in this note comes from `deqp-gles2` run on a host against
the Mali back end's C pair on the Linux OSAL (`tools/mali/deqp-host-measure.sh`, the
method of `mali-deqp-families.md`): nothing is drawn, and what is real is each shader's
compile and link outcome. A board status quoted here (Fail, NotSupported) is the
NanoPC-T6's own, from the slices `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608`.

- Tip: `9ac31f30` (integration tip of plan #387 at the time of the run).
- CTS: VK-GL-CTS `93bca01861b0e3ef3c387027a9791e6d065f900c` with
  `cts-yakogl-target.patch` applied, its external sources fetched (`$CONNECTED_DEPS_DIR/VK-GL-CTS`).
- Cases: the 10,597 cases of the four slices whose board refusal is a compile refusal;
  all began on the host, 6,196 still end with a compile refusal at the tip (4,401
  compile now: the work of plans #367, #373 and #377 since `37305608`).
- Command (4 cores): `tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS"
  --out <dir> --jobs 3`; its `measure.tsv` rows with host family `367:ARITH` are the
  **1627** cases of this note. Per case the refusing shader's source is in `<dir>/src/`.
- References: Mesa 26.2.1 (`tools/mali/drm_shim_ref.sh build` of the tarball of
  `third_party/mesa-26.2.1/UPSTREAM`, in `$CONNECTED_DEPS_DIR/mesa-shim`), drawn with
  `tools/mali/deqp_ref.sh --instrument shim` (= `drm_shim_ref.sh run ... --uniform-values`,
  then `pan_ref_diff.py extract --draw`). Checked first by redrawing `deqp_intval_opfs_not`:
  every byte of its `expect.json` but the `trace` name is equal.

## 1. What was counted

The three texts of the family (`MaliError::Arithmetic`, `FsOperandForm`, `Rebalance`,
`src/shaderc/online/mali/mod.rs`), as the FIRST compile refusal of a case (the
first refusing shader of the case's first refusing program):

| Text | FS | VS | Total |
|---|---:|---:|---:|
| `arithmetic other than +, -, * and a dot or matrix product (in a fragment shader: +, -, * and abs)` | 889 | 574 | 1463 |
| `a fragment shader operand of a form or in a place no reference draws` | 152 | 0 | 152 |
| `three or more additions or multiplications in a chain` | 4 | 8 | 12 |
| **Total** | **1045** | **582** | **1627** |

Board status of the 1627: 1564 Fail, 63 NotSupported. At `37305608` the plan counted 1,581
(FS 721 + 59 NotSupported, VS 651); the tip has 46 more (most plausibly cases whose
earlier refusal, `CONST_FOLD`, `intval` or `LOCALS`, a later plan lifted, now reaching an op;
the cause was not studied). dEQP groups: operator 643, random 368, uniform_api.value 136, swizzles 93, matrix 83, constant_expressions 74, conversions 61, indexing 60, uniform_api.info_query 22, uniform_api.random 21, functions 14, fragment_ops.interaction 11, builtin_variable 9, struct 9, return 6, algorithm 3, invariance 3, texture_functions 2, attribute_location.bind 1, attribute_location.bind_time 1, flush_finish.finish 1, flush_finish.finish_wait 1, flush_finish.flush 1, flush_finish.flush_wait 1, flush_finish.wait 1, linkage 1, misc 1.

How the shapes were found. The refusal text names no op. A scratch copy of the tree
(a git worktree outside the repository, never committed) recorded, at every site that
raises one of the three texts, the source line and the IR op the walk was at, and
`mali_ref::compile_pair` compiled the 1,627 refusing pairs in-process (a second, 1 s run):
its stage and text equal `measure.tsv`'s for all 1,627 cases. The **site** below is
that line, the **op** the front end's `AluOp` the walk had reached (not always the op the
shader's author wrote: the front end lowers `/` to `frcp`, `acos` to a polynomial of
`fsign`, `sqrt` to `frcp(frsq)`, ...). The first refusal hides the next ones: the
counts are the cases a step MOVES, an upper bound of those it finishes (see section 5).

## 2. The shapes, ranked by the cases of the step that lifts them

A **shape** is one dEQP operation in one stage (the case's built-in or operator, or for
the generated programs the op the walk refuses first). Precision: the dEQP suites run
each op at `lowp`, `mediump` and `highp`; Mesa 26.2.1 compiles `lowp` exactly as `mediump`
(fp16, checked byte for byte on `sin` fs, `max` vs and `sign` fs: the three pairs' words
are equal), so the tables count them together (`mid`). **Steps** group the shapes by the
site that refuses them, i.e. by the lowering code that has to change.

| Step | Stage and site | Cases | Fail / NS | Operator-suite cases | In constant folds | Owned elsewhere | Shapes (cases) |
|---|---|---:|---|---:|---:|---:|---|
| 1 vs-transc | VS op arm (nir.rs:5099): `sin cos exp exp2 log log2 pow ceil sign` | 240 | 240 / 0 | 100 | 41 | 0 | random.fexp2 46, random.flog2 24, random.fsin 22, constexpr.flog2 12, op.angle.cos 12, op.angle.sin 12 |
| 2 fs-div | FS `Frcp` arm (nir_fs.rs:6016): `/`, `mod`, `smoothstep`, `atan`, matrix `/` | 187 | 187 / 0 | 115 | 0 | 0 | matrix.div 54, op.binary.div_assign 42, op.binary.div 30, op.common.smoothstep 21, random.frcp 15, op.common.mod 14 |
| 3 fs-transc | FS `sin cos log2` arm and `exp2` arm (nir_fs.rs:6175, 6212) | 187 | 187 / 0 | 60 | 0 | 0 | random.flog2 41, random.fsin 35, random.fexp2 33, random.fcos 13, op.angle.cos 12, op.angle.sin 12 |
| 4 fs-pairs | FS pair trees (nir_fs.rs:9807, 9345, 9012, 8391, `FsOperandForm`) | 152 | 152 / 0 | 69 | 0 | 0 | random.storeoutput 31, matrix.storeout 29, other.storeoutput 19, op.unary.incdec 12, op.binary.add 8, op.binary.add_assign 8 |
| 5 minmax | `min max clamp` (nir_fs.rs:6048, nir.rs `vs_clamp` :3571) | 133 | 133 / 0 | 126 | 0 | 0 | op.common.clamp 42, op.common.max 42, op.common.min 42, other.fmax 5, other.fmin 2 |
| 6 fs-select | FS `Bcsel` on a non-constant condition (nir_fs.rs:6151) | 120 | 57 / 63 | 52 | 0 | 0 | indexing.dynamic_select 60, op.selection.select 28, other.bcsel 8, op.angle.atan2 8, op.geometric.faceforward 8, op.geometric.refract 8 |
| 7 fs-round | FS generic arm (nir_fs.rs:6291): `floor`, `!=` of floats, `sign` | 247 | 247 / 0 | 61 | 0 | 146 | isok_int_round 146, random.fne 19, other.ffloor 16, op.binary.not_equal 12, op.common.floor 12, op.common.sign 12 |
| 8 vs-reduce | VS `vs_reduce`, `any`/`all` and `==`/`!=` of vectors (nir.rs:3571-3711) | 265 | 265 / 0 | 48 | 6 | 179 | uniform_api.bool_compare 179, random.bany 22, op.binary.equal 21, op.binary.not_equal 21, other.bany 4, other.ball 4 |
| 9 fs-sqrt | FS `sqrt`, `inversesqrt`, `ceil` (nir_fs.rs:6259, 5938) | 56 | 56 / 0 | 12 | 0 | 0 | random.fsqrt 34, op.common.ceil 12, random.frsq 10 |
| 10 constfold | FS highp constant ops (nir_fs.rs:3493, 3495) | 28 | 28 / 0 | 0 | 27 | 0 | constexpr.fsin 9, constexpr.fexp2 8, constexpr.fcos 4, constexpr.ball 3, constexpr.bany 3, other.ball 1 |
| 11 chain | `do_rebalance_tree` chains (glsl.rs:1255-1379, `Rebalance`) | 12 | 12 / 0 | 0 | 0 | 0 | random.chain 7, other.x 5 |
| **Total** | | **1627** | 1564 / 63 | 643 | 74 | 325 | |

"In constant folds" are the `shaders.constant_expressions` cases (`const float x =
sin(...)`): Mesa folds them away entirely (their references hold no op), so the step
there is the front end's constant fold (the `constop` work), not a lowering.
"Owned elsewhere" is `isok_int_round` (section 3, `intval-reslib`) and
`uniform_api.bool_compare` (`bvec == bvec` of uniform words, the words of
`mali-deqp-intval.md`'s bool uniforms): the refusal text is ARITH but the lowering is the
integer/boolean family's.

### 2.1 Every shape, with its reference

One row per (shape, stage); the sum of the Cases column is 1627. `Refs` names the
references `tools/mali/testdata/deqp_arith_<shape>_<stage>_<precision>.{vert,frag,expect.json}`
(`<precision>` is `highp`, `mediump` (and `lowp`) or `none` where the shader holds no
precision of its own), `-` where a shape has none (a bucket under 3 cases outside the
operator suites, a random bucket under 10: the pair is a generated program, `Steps` say what
refuses it).

| Shape | Stage | Cases | Precision (hi / mid+low / none) | Text | Step | dEQP groups | Refs |
|---|---|---:|---|---|---|---|---|
| `uniform_api.bool_compare` | vs | 179 | 0 / 0 / 179 | ARITH | 8 | uniform_api.value 136, uniform_api.info_query 22, uniform_api.random 21 | none |
| `isok_int_round` | fs | 146 | 0 / 146 / 0 | ARITH | 7 | swizzles 93, conversions 53 | mediump |
| `indexing.dynamic_select` | fs | 60 | 0 / 0 / 60 | ARITH | 6 | indexing 60 | none |
| `matrix.div` | fs | 54 | 18 / 36 / 0 | ARITH | 2 | matrix 54 | highp, mediump |
| `random.fexp2` | vs | 46 | 0 / 0 / 46 | ARITH | 1 | random 46 | none |
| `op.binary.div_assign` | fs | 42 | 14 / 28 / 0 | ARITH | 2 | operator 42 | highp, mediump |
| `random.flog2` | fs | 41 | 0 / 41 / 0 | ARITH | 3 | random 41 | mediump |
| `random.fsin` | fs | 35 | 0 / 35 / 0 | ARITH | 3 | random 35 | mediump |
| `random.fsqrt` | fs | 34 | 0 / 34 / 0 | ARITH | 9 | random 34 | mediump |
| `random.fexp2` | fs | 33 | 0 / 33 / 0 | ARITH | 3 | random 33 | mediump |
| `random.storeoutput` | fs | 31 | 0 / 31 / 0 | OPERAND | 4 | random 31 | mediump |
| `op.binary.div` | fs | 30 | 10 / 20 / 0 | ARITH | 2 | operator 30 | highp, mediump |
| `matrix.storeout` | fs | 29 | 11 / 18 / 0 | OPERAND | 4 | matrix 29 | highp, mediump |
| `op.selection.select` | fs | 28 | 8 / 16 / 4 | ARITH | 6 | operator 28 | highp, mediump, none |
| `random.flog2` | vs | 24 | 0 / 0 / 24 | ARITH | 1 | random 24 | none |
| `random.bany` | vs | 22 | 0 / 0 / 22 | ARITH | 8 | random 22 | none |
| `random.fsin` | vs | 22 | 0 / 0 / 22 | ARITH | 1 | random 22 | none |
| `op.binary.equal` | vs | 21 | 6 / 12 / 3 | ARITH | 8 | operator 21 | highp, mediump, none |
| `op.binary.not_equal` | vs | 21 | 6 / 12 / 3 | ARITH | 8 | operator 21 | highp, mediump, none |
| `op.common.clamp` | fs | 21 | 7 / 14 / 0 | ARITH | 5 | operator 21 | highp, mediump |
| `op.common.clamp` | vs | 21 | 7 / 14 / 0 | ARITH | 5 | operator 21 | highp, mediump |
| `op.common.max` | fs | 21 | 7 / 14 / 0 | ARITH | 5 | operator 21 | highp, mediump |
| `op.common.max` | vs | 21 | 7 / 14 / 0 | ARITH | 5 | operator 21 | highp, mediump |
| `op.common.min` | fs | 21 | 7 / 14 / 0 | ARITH | 5 | operator 21 | highp, mediump |
| `op.common.min` | vs | 21 | 7 / 14 / 0 | ARITH | 5 | operator 21 | highp, mediump |
| `op.common.smoothstep` | fs | 21 | 7 / 14 / 0 | ARITH | 2 | operator 21 | highp, mediump |
| `other.storeoutput` | fs | 19 | 0 / 10 / 9 | OPERAND | 4 | conversions 8, return 6, fragment_ops.interaction 2 | mediump, none |
| `random.fne` | fs | 19 | 0 / 19 / 0 | ARITH | 7 | random 19 | mediump |
| `other.ffloor` | fs | 16 | 0 / 2 / 14 | ARITH | 7 | builtin_variable 8, struct 6, functions 2 | none |
| `random.frcp` | fs | 15 | 0 / 15 / 0 | ARITH | 2 | random 15 | mediump |
| `op.common.mod` | fs | 14 | 7 / 7 / 0 | ARITH | 2 | operator 14 | highp, mediump |
| `random.fcos` | fs | 13 | 0 / 13 / 0 | ARITH | 3 | random 13 | mediump |
| `constexpr.flog2` | vs | 12 | 12 / 0 / 0 | ARITH | 1 | constant_expressions 12 | highp |
| `op.angle.cos` | fs | 12 | 4 / 8 / 0 | ARITH | 3 | operator 12 | highp, mediump |
| `op.angle.cos` | vs | 12 | 4 / 8 / 0 | ARITH | 1 | operator 12 | highp, mediump |
| `op.angle.sin` | fs | 12 | 4 / 8 / 0 | ARITH | 3 | operator 12 | highp, mediump |
| `op.angle.sin` | vs | 12 | 4 / 8 / 0 | ARITH | 1 | operator 12 | highp, mediump |
| `op.angle.tan` | fs | 12 | 4 / 8 / 0 | ARITH | 3 | operator 12 | highp, mediump |
| `op.angle.tan` | vs | 12 | 4 / 8 / 0 | ARITH | 1 | operator 12 | highp, mediump |
| `op.binary.not_equal` | fs | 12 | 4 / 8 / 0 | ARITH | 7 | operator 12 | highp, mediump |
| `op.common.ceil` | fs | 12 | 4 / 8 / 0 | ARITH | 9 | operator 12 | highp, mediump |
| `op.common.ceil` | vs | 12 | 4 / 8 / 0 | ARITH | 1 | operator 12 | highp, mediump |
| `op.common.floor` | fs | 12 | 4 / 8 / 0 | ARITH | 7 | operator 12 | highp, mediump |
| `op.common.sign` | fs | 12 | 4 / 8 / 0 | ARITH | 7 | operator 12 | highp, mediump |
| `op.common.sign` | vs | 12 | 4 / 8 / 0 | ARITH | 1 | operator 12 | highp, mediump |
| `op.unary.incdec` | fs | 12 | 0 / 12 / 0 | OPERAND | 4 | operator 12 | mediump |
| `random.frsq` | fs | 10 | 0 / 10 / 0 | ARITH | 9 | random 10 | mediump |
| `constexpr.fsin` | fs | 9 | 9 / 0 / 0 | ARITH | 10 | constant_expressions 9 | highp |
| `constexpr.fsin` | vs | 9 | 9 / 0 / 0 | ARITH | 1 | constant_expressions 9 | highp |
| `op.float.notEqual` | fs | 9 | 3 / 6 / 0 | ARITH | 7 | operator 9 | highp, mediump |
| `constexpr.fexp2` | fs | 8 | 8 / 0 / 0 | ARITH | 10 | constant_expressions 8 | highp |
| `constexpr.fexp2` | vs | 8 | 8 / 0 / 0 | ARITH | 1 | constant_expressions 8 | highp |
| `op.angle.acos` | fs | 8 | 4 / 4 / 0 | ARITH | 7 | operator 8 | highp, mediump |
| `op.angle.asin` | fs | 8 | 4 / 4 / 0 | ARITH | 7 | operator 8 | highp, mediump |
| `op.angle.atan` | fs | 8 | 4 / 4 / 0 | ARITH | 2 | operator 8 | highp, mediump |
| `op.angle.atan2` | fs | 8 | 4 / 4 / 0 | ARITH | 6 | operator 8 | highp, mediump |
| `op.binary.add` | fs | 8 | 0 / 8 / 0 | OPERAND | 4 | operator 8 | mediump |
| `op.binary.add_assign` | fs | 8 | 0 / 8 / 0 | OPERAND | 4 | operator 8 | mediump |
| `op.binary.sub` | fs | 8 | 0 / 8 / 0 | OPERAND | 4 | operator 8 | mediump |
| `op.binary.sub_assign` | fs | 8 | 0 / 8 / 0 | OPERAND | 4 | operator 8 | mediump |
| `op.exponential.exp` | vs | 8 | 4 / 4 / 0 | ARITH | 1 | operator 8 | highp, mediump |
| `op.exponential.exp2` | vs | 8 | 4 / 4 / 0 | ARITH | 1 | operator 8 | highp, mediump |
| `op.exponential.log` | fs | 8 | 4 / 4 / 0 | ARITH | 3 | operator 8 | highp, mediump |
| `op.exponential.log` | vs | 8 | 4 / 4 / 0 | ARITH | 1 | operator 8 | highp, mediump |
| `op.exponential.log2` | fs | 8 | 4 / 4 / 0 | ARITH | 3 | operator 8 | highp, mediump |
| `op.exponential.log2` | vs | 8 | 4 / 4 / 0 | ARITH | 1 | operator 8 | highp, mediump |
| `op.exponential.pow` | fs | 8 | 4 / 4 / 0 | ARITH | 3 | operator 8 | highp, mediump |
| `op.exponential.pow` | vs | 8 | 4 / 4 / 0 | ARITH | 1 | operator 8 | highp, mediump |
| `op.geometric.faceforward` | fs | 8 | 4 / 4 / 0 | ARITH | 6 | operator 8 | highp, mediump |
| `op.geometric.refract` | fs | 8 | 4 / 4 / 0 | ARITH | 6 | operator 8 | highp, mediump |
| `other.bcsel` | fs | 8 | 0 / 4 / 4 | ARITH | 6 | functions 4, struct 3, algorithm 1 | mediump, none |
| `random.fcos` | vs | 7 | 0 / 0 / 7 | ARITH | 1 | random 7 | - |
| `op.binary.mul` | fs | 6 | 0 / 6 / 0 | OPERAND | 4 | operator 6 | mediump |
| `other.fmax` | vs | 5 | 0 / 5 / 0 | ARITH | 5 | functions 5 | mediump |
| `other.fsin` | fs | 5 | 0 / 0 / 5 | ARITH | 3 | flush_finish.finish 1, flush_finish.finish_wait 1, flush_finish.flush 1 | none |
| `constexpr.fceil` | vs | 4 | 4 / 0 / 0 | ARITH | 1 | constant_expressions 4 | highp |
| `constexpr.fcos` | fs | 4 | 4 / 0 / 0 | ARITH | 10 | constant_expressions 4 | highp |
| `constexpr.fcos` | vs | 4 | 4 / 0 / 0 | ARITH | 1 | constant_expressions 4 | highp |
| `constexpr.fsign` | vs | 4 | 4 / 0 / 0 | ARITH | 1 | constant_expressions 4 | highp |
| `op.binary.mul_assign` | fs | 4 | 0 / 4 / 0 | OPERAND | 4 | operator 4 | mediump |
| `op.common.mix` | fs | 4 | 0 / 4 / 0 | OPERAND | 4 | operator 4 | mediump |
| `op.exponential.exp2` | fs | 4 | 2 / 2 / 0 | OPERAND | 4 | operator 4 | highp, mediump |
| `other.ball` | vs | 4 | 0 / 0 / 4 | ARITH | 8 | fragment_ops.interaction 4 | none |
| `other.bany` | vs | 4 | 0 / 0 / 4 | ARITH | 8 | fragment_ops.interaction 4 | none |
| `other.x` | vs | 4 | 1 / 3 / 0 | CHAIN | 11 | invariance 3, functions 1 | mediump |
| `random.chain` | vs | 4 | 0 / 0 / 4 | CHAIN | 11 | random 4 | - |
| `constexpr.ball` | fs | 3 | 3 / 0 / 0 | ARITH | 10 | constant_expressions 3 | highp |
| `constexpr.ball` | vs | 3 | 3 / 0 / 0 | ARITH | 8 | constant_expressions 3 | highp |
| `constexpr.bany` | fs | 3 | 3 / 0 / 0 | ARITH | 10 | constant_expressions 3 | highp |
| `constexpr.bany` | vs | 3 | 3 / 0 / 0 | ARITH | 8 | constant_expressions 3 | highp |
| `op.bool.all` | vs | 3 | 0 / 0 / 3 | ARITH | 8 | operator 3 | none |
| `op.bool.any` | vs | 3 | 0 / 0 / 3 | ARITH | 8 | operator 3 | none |
| `other.frcp` | fs | 3 | 0 / 0 / 3 | ARITH | 2 | texture_functions 2, builtin_variable 1 | none |
| `random.chain` | fs | 3 | 0 / 3 / 0 | CHAIN | 11 | random 3 | - |
| `random.fsign` | fs | 3 | 0 / 3 / 0 | ARITH | 7 | random 3 | - |
| `random.storelocal` | fs | 3 | 0 / 3 / 0 | OPERAND | 4 | random 3 | - |
| `op.common.abs` | fs | 2 | 0 / 2 / 0 | OPERAND | 4 | operator 2 | mediump |
| `op.geometric.reflect` | fs | 2 | 1 / 1 / 0 | OPERAND | 4 | operator 2 | highp, mediump |
| `op.unary.unary_minus` | fs | 2 | 0 / 2 / 0 | OPERAND | 4 | operator 2 | mediump |
| `other.fne` | fs | 2 | 0 / 2 / 0 | ARITH | 7 | fragment_ops.interaction 1, functions 1 | - |
| `random.ball` | vs | 2 | 0 / 0 / 2 | ARITH | 8 | random 2 | - |
| `op.geometric.cross` | fs | 1 | 0 / 1 / 0 | OPERAND | 4 | operator 1 | mediump |
| `other.ball` | fs | 1 | 1 / 0 / 0 | ARITH | 10 | misc 1 | - |
| `other.fmin` | fs | 1 | 0 / 0 / 1 | ARITH | 5 | algorithm 1 | - |
| `other.fmin` | vs | 1 | 0 / 0 / 1 | ARITH | 5 | algorithm 1 | - |
| `other.x` | fs | 1 | 0 / 1 / 0 | CHAIN | 11 | functions 1 | - |
| `random.imul` | fs | 1 | 0 / 1 / 0 | OPERAND | 4 | random 1 | - |
| **Total** | | **1627** | | | | | |

Reference count: 144 (`tests/mali_deqp_arith.rs`, one test each).


## 3. Mesa 26.2.1's lowering per op

From the references' own listings (`BIFROST_MESA_DEBUG=shaders`, the final Valhall
disassembly; the mnemonics below leave out the attribute loads, the stores and the
`ATEST`/`BLEND` tail). `mid` is `mediump` and `lowp` (fp16 in lane pairs: an operand is
narrowed by `FADD.f32 r.h0, x, 0x0.neg` or `V2F32_TO_V2F16`, the result widened by
`U16_TO_U32` and the colour's `FMA.v2f16 .. 0x38000000.h11` for `color * 0.5 + 0.5`);
`hi` is `highp`. Words, FAU and registers are in each `expect.json`.

| Op | Mesa's sequence | Reference |
|---|---|---|
| `a / b`, `a /= b`, `mat / float` | `FRCP.f32 b`, `FMA.f32 a, rcp, -0` (hi); mid: `FRCP.f16` + `FMA.v2f16`; the matrix form one `FRCP` for the scalar and one `FMA` a lane | `op_binary_div_*`, `op_binary_div_assign_*`, `matrix_div_*` |
| `mod(x, y)` | `FRCP`, `FMA` (x * rcp y), `FROUND.<f32\|v2f16>.rtn`, `FMA` (x - y * floor): 4 ops at either width | `op_common_mod_*` |
| `smoothstep(e0, e1, x)` | `FRCP` (of e1 - e0), `FMA.<f32\|v2f16>.clamp_0_1` (t), then `FMA` x3 (t * t * (3 - 2t)) | `op_common_smoothstep_*` |
| `floor`, `ceil` | one `FROUND.f32.rtn` / `.rtp` (hi); mid `FROUND.v2f16.rtn` / `.rtp` of the narrowed operand | `op_common_floor_*`, `op_common_ceil_*` |
| `sign` | two `FCMP_OR.<f32\|v2f16>.lt.f1` (x < 0 and 0 < x as a float 0.0 / 1.0) and `FADD` of the two (the second `.neg`) | `op_common_sign_*` |
| `min`, `max` | `FMIN` / `FMAX.f32` (hi, in either stage); mid `.v2f16` on the narrowed operands; `clamp` is `FMAX` then `FMIN` | `op_common_{min,max,clamp}_*` |
| `a != b` (float) | `FCMP_OR.f32.ne.m1` and `MUX.v2i16` to the colour's pair (hi and mid alike) | `op_binary_not_equal_fs_*`, `op_float_notequal_fs_*` |
| `c ? a : b` | mid `FCMP_OR.v2f16.lt.m1` + `MUX.v2i16`; hi `FCMP_OR.v2f16.lt.m1` + `S16_TO_S32` + `CSEL.u32.eq` | `op_selection_select_fs_*` |
| `sin`, `cos` | the same fp32 sequence at either precision (a mediump operand is `F16_TO_F32` first): `FMA.f32`, `FADD_IMM.f32 #0xC9400000`, `FMA.f32`, `FSIN_TABLE.u6`, `FCOS_TABLE.u6`, `FMA_RSCALE.f32`, `FMA.f32`, `FMA.f32.clamp_m1_1`, `FADD.f32` (`mali-transcendentals.md`) | `op_angle_{sin,cos}_*` |
| `tan` | the `sin`/`cos` sequence once, a second `FMA.f32` + `.clamp_m1_1` + `FADD.f32` for the cosine, `FRCP.f32` (mid `FRCP.f16`) and one `FMA` | `op_angle_tan_*` |
| `exp2` | `FMA_RSCALE.f32`, `F32_TO_S32`, `FEXP.f32`; `exp` one `FMA.f32` (x * log2 e) before it | `op_exponential_exp2_*`, `op_exponential_exp_*` |
| `log2` | `FREXPE.f32.log`, `S32_TO_F32`, `FADD_LSCALE.f32`, `FLOGD.f32`, `FMA.f32`; `log` one more `FMA` (by ln 2) | `op_exponential_log2_*`, `op_exponential_log_*` |
| `pow(x, y)` | the `log2` row (5 ops), `FMA_RSCALE.f32` (y * log2 x in the scale slot), `F32_TO_S32`, `FEXP.f32` | `op_exponential_pow_*` |
| `acos`, `asin` | fp32 at **both** precisions (the mid listing is the hi one): `FMA.f32` x3, `FADD.f32`, `FRSQ.f32` + `FRCP.f32` (the `sqrt`), `FMA.f32`, `FCMP_OR.f32.lt.f1` x2 (the `sign`), `FADD.f32`; `asin` adds `FMA.f32` x4, `FCMP_OR.f32.lt.m1`, `CSEL.u32.eq` | `op_angle_{acos,asin}_fs_*` |
| `atan`, `atan2` | hi: `FRCP.f32`, `FCMP_OR.f32.ge.m1` + `CSEL.u32.eq` (the |x| > 1 fold), 6 `FMA.f32` (the polynomial), `CSEL`, `FMA`, `LSHIFT_AND.i32` x2 + `LSHIFT_OR.i32` (the sign bit), `FADD.f32`; mid: the fp16 form (`FRCP.f16`, `FMA.v2f16`, `MUX.v2i16`, `LSHIFT_*.v2i16`); `atan2` adds the quadrant fix (`FCMP_OR.ge/eq.m1` + `CSEL`, a second `FRCP`) | `op_angle_{atan,atan2}_fs_*` |
| `faceforward`, `refract` | `faceforward`: `FMA.f32` (dot), `FCMP_OR.f32.lt.m1`, `CSEL.u32.eq`, `FADD`; `refract`: 4 `FMA` (dot), `FRSQ.f32` + `FRCP.f32` (the `sqrt`), 3 `FMA`, `FCMP_OR.f32.lt.m1` + `CSEL.u32.eq` (the k < 0 select); mid the same in fp16 pairs with `FRSQ.f16`/`FRCP.f16` | `op_geometric_{faceforward,refract}_*` |
| `reflect`, `cross`, `mix` | pure `FMA` (`.v2f16` pairs at mid) and `MKVEC.v2i16` chains | `op_geometric_{reflect,cross}_*`, `op_common_mix_*` |
| `+= -= *= ++ -- -x abs` of varyings (mid) | `V2F32_TO_V2F16` / `FADD.f32 .h0` narrowing, `FADD.v2f16` / `FMA.v2f16` pairs, `MKVEC.v2i16` | `op_binary_{add,sub,mul}*_fs_mediump`, `op_unary_*`, `op_common_abs_*` |
| `any`, `all`, `==`, `!=` of vectors (VS) | mediump attributes at fp16: narrow (`FADD.f32 .h0`, `V2F32_TO_V2F16`), `FCMP_OR.v2f16.lt.m1` per pair, `ICMP_OR.v2u16.eq/ne.m1`, `LSHIFT_AND/OR.v2i16`, `S16_TO_S32`, `CSEL.u32.eq`; `any` / `all` of `lessThan` `FMAX` / `FMIN.v2f16` of the pairs; ivecs `F32_TO_S32.rtz`, `ICMP_OR.u32.eq.m1`, `ICMP_AND.u32.eq.m1`, `CSEL` | `op_binary_{equal,not_equal}_vs_*`, `op_bool_{any,all}_vs_none`, `other_{bany,ball}_vs_none` |
| bool uniform compare (VS) | integer words: `LSHIFT_OR.i32`, `LSHIFT_AND.i32`, `CSEL.u32.eq`, `FMA.f32` (the `*=` of the result) | `uniform_api_bool_compare_vs_none` |
| a constant expression (`const float x = sin(1.0)`) | no op: Mesa folds it, the listing holds `MOV.i32` / `MUX.v2i16` of the folded constants | `constexpr_*` |

`sqrt` and `inversesqrt` (step 9) are `FRSQ` (+ `FRCP` for `sqrt`) as in `mali-alu-ops.md`;
the random shapes' references (`random_*`) are generated programs: their listing is the
sum of the ops they hold and is not tabulated.

## 4. The lowering map, in steps of one session

Rank: the cases a step moves, net of the cases another item owns (`intval-reslib`'s
`isok_int_round`, `intval-rest`'s bool-uniform compare). `nir_fs.rs` is the fragment walk
(`walk_nodes`; the mediump arm, `alu32` for fp32, the pair trees from line 9000);
`nir.rs` the vertex walk (`walk`, the op arm from `IrOp::Alu(a, s)` at line 4619,
`vs_clamp`, `vs_reduce`). Every step byte-compares its references of the table above
(they are the tests `tests/mali_deqp_arith.rs` un-ignores) and regenerates nothing:
the references are fixed. The first-refusal count is an upper bound of the cases a step
finishes: the operator-suite column of section 2 (a single built-in over a pass-through
colour) is the likely floor of those it finishes, the random programs finish only when
every op of the program is lowered.

| # | Step | Moves | Net | dEQP groups | Where | What |
|---|---|---:|---:|---|---|---|
| 1 | **vs-transc** | 240 | 240 | operator.{angle,exponential,common} 100, random 99, constant_expressions 41 (folds) | `nir.rs` op arm at `_ => Err(Arithmetic)` (5099) | `Fsin`, `Fcos`, `Flog2`, `Fexp2` (and `exp`, `log`, `pow`, `tan` that the front end builds of them), `Fceil`, `Fsign` of an fp32 attribute lane, as the vertex stage's `Alu` nodes the fragment walk already builds (`Alu::Fsin`..`Alu::Fexp2`, rows of `mali-transcendentals.md`): references `op_angle_{sin,cos,tan}_vs_*`, `op_exponential_*_vs_*`, `op_common_{ceil,sign}_vs_*`, `random_{fexp2,flog2,fsin}_vs_none`. The constant-expression share (41) is the front end's fold, not this step |
| 2 | **fs-div** | 187 | 187 | operator.{binary,common,angle} 115, matrix 54, random 15 | `nir_fs.rs` `Frcp` arm (6016: only `1.0 / constant`) | `a / b` as `FRCP` + `FMA` (fp32; fp16 `FRCP.f16` + `FMA.v2f16` on lane pairs) of a varying or computed divisor; then `mod` (`FROUND.rtn` + `FMA`), `smoothstep` (`FMA.clamp_0_1`), the `atan` hi/mid forms are the next steps of the same arm: references `op_binary_div*`, `matrix_div_*`, `op_common_{mod,smoothstep}_*`, `op_angle_atan*`, `other_frcp_fs_none` |
| 3 | **fs-transc** | 187 | 187 | random 122, operator.{angle,exponential} 60 | `nir_fs.rs` `Fsin|Fcos|Flog2` arm (6175: only a uniform lane) and the `exp2` arm (6212) | the same ops of a varying's lane or of a computed lane (`F16_TO_F32` first at mid, `FADD.f32 .h0` to narrow): references `op_angle_{sin,cos,tan}_fs_*`, `op_exponential_*_fs_*`, `random_{flog2,fsin,fexp2,fcos}_fs_mediump` |
| 4 | **fs-pairs** | 152 | 152 | operator 69, random 35, matrix 29 | `nir_fs.rs` `ADMITTED` products (9807) and the pair-tree arms 8391, 9012, 9345 (`FsOperandForm`) | fp16 pair trees of `+= -= *= ++ -- -x abs mix reflect` over varyings (and a matrix `+`): widen the `Prod` set the pair builder admits; Mesa's words are the `FADD.v2f16` / `FMA.v2f16` / `MKVEC.v2i16` of the table: references `op_binary_{add,sub,mul}*_fs_mediump`, `op_unary_*`, `op_common_{abs,mix}_*`, `op_geometric_{reflect,cross}_*`, `matrix_storeout_*`, `random_storeoutput_fs_mediump`, `other_storeoutput_*` |
| 5 | **minmax** | 133 | 133 | operator.common 126, functions 5 | `nir_fs.rs` `Fmax|Fmin` arm (6048: needs a constant bound) and `nir.rs` `vs_clamp` (3571) | `min`, `max`, `clamp` of two computed values or of a varying and an attribute: `FMIN/FMAX.f32` and `.v2f16` (the rows' presence in the ISA table is to be checked): references `op_common_{min,max,clamp}_*` |
| 6 | **fs-select** | 120 | 120 | indexing 60, operator.selection 28, geometric 16, angle 8 | `nir_fs.rs` `Bcsel` arm (6151: constant arms only) | `Bcsel` on a computed condition with computed arms: `FCMP_OR.v2f16.lt.m1` + `MUX.v2i16` (mid) or `CSEL.u32.eq` (hi), and the dynamic vector subscript's `ICMP_OR` + `MUX` chain (`mali-alu-ops.md`, intloop_dynidx): references `op_selection_select_fs_*`, `indexing_dynamic_select_fs_none`, `other_bcsel_*`, then `faceforward`, `refract`, `atan2` |
| 7 | **fs-round** | 247 (101) | 101 | operator 61, random 19, builtin_variable 8, struct 6 | `nir_fs.rs` generic arm (6291) | `floor` (`FROUND.rtn`, hi and `.v2f16`), `!=` of floats (`FCMP_OR.f32.ne.m1` + `MUX`), `sign` (two `FCMP_OR.lt.f1` + `FADD`) of a varying. The 146 `isok_int_round` cases (`ivec4(floor(a + 0.5)) == b` of the shader library's `isOk`, 93 `swizzles` + 53 `conversions`) are `intval-reslib`'s: they stop here too, so the order between the two items is to be agreed: references `op_common_{floor,sign}_fs_*`, `op_binary_not_equal_fs_*`, `op_float_notequal_fs_*`, `other_ffloor_fs_none`, `random_fne_fs_mediump`, `isok_int_round_fs_mediump` |
| 8 | **vs-reduce** | 265 (86) | 86 | operator 48, random 24 | `nir.rs` `vs_reduce` (3686, 3711), `vs_clamp` (3618) | the vertex `any` / `all` of lessThan lanes and `==` / `!=` of vectors at fp16 (`FCMP_OR.v2f16.lt.m1`, `ICMP_OR.v2u16`, `LSHIFT_*.v2i16`, `S16_TO_S32`, `CSEL`): the `mixprec` vertex side (`mali-deqp-intval.md` section 8 names the same `opvs_not_equal_bvec3`); the 179 bool-uniform compares are `intval-rest`'s: references `op_binary_{equal,not_equal}_vs_*`, `op_bool_{any,all}_vs_none`, `other_{bany,ball}_vs_none`, `random_bany_vs_none`, `uniform_api_bool_compare_vs_none` |
| 9 | **fs-sqrt** | 56 | 56 | random 44, operator.common 12 | `nir_fs.rs` `Frsq|Fsqrt` arm (6259: a computed mediump lane only) and `Fceil` (5938) | `sqrt` / `inversesqrt` of a varying's lane or a highp lane (`FRSQ.f32` + `FRCP.f32`), `ceil` (`FROUND.f32.rtp`, `.v2f16.rtp`): references `op_common_ceil_fs_*`, `random_frsq_fs_mediump` (no `sqrt` reference: its 34 cases are random programs) |
| 10 | **constfold** | 28 | 28 | constant_expressions 27, misc 1 | `nir_fs.rs` `alu32` constant arms (3493, 3495) and the VS folds of step 1 | `sin`, `cos`, `exp2`, `any`, `all` of constants folded by the front end (Mesa leaves no op: `constexpr_*` listings are `MOV`/`MUX` of constants): `constop`'s fold, with its non-finite rules |
| 11 | **chain** | 12 | 12 | random 7, invariance 3, functions 2 | `glsl.rs` `Rebalance` (1255-1379) | the `do_rebalance_tree` forms the glmark2 references (`mali-rebalance.md`) do not draw; references `other_x_vs_mediump` (a mediump vertex chain), the invariance cases (`fract` of a chain) have none |

The sum of the Moves column of the shapes is 1,627 = the section 2 total. Step 7's 247 and step 8's 265
include the 146 and 179 other-item cases of the net column.

### 4.1 What a session of each step does

Each step is "one session" of the earlier Mali plans: find the arm, make the walk build the
`Alu` node Mesa builds (`emit.rs` has rows for `Frcp` at 32 and 16 bits, `Ffloor` and `Fceil` at
both widths and `Ftrunc` at 32; the transcendentals' nodes (`Alu::Fsin` ...) are the
fragment walk's; a missing row is added with its `isa/` table entry and a unit test), byte-compare the references
of the step, move the passing ones to `LOWERED` and un-ignore their tests in
`tests/mali_deqp_arith.rs` (its `refusals_pinned` forces both), and run the host
forecast: `tools/mali/deqp-host-measure.sh --filter` on the step's dEQP group. The first
refusal that remains in the cases of the step is the next step's input; a case's next
refusal is in the `next_family` column of `measure.tsv`.

Cautions found while drawing the references:

- `lowp` and `mediump` are one shape (words equal), so a mediump lowering serves both;
  `highp`-named fragment cases refuse at the same walk arm as the `mediump` ones (the
  varying the case reads is `mediump` or its colour is stored `mediump`: the walk's `prec`
  is the colour's, not the operation's). The tables count by the case name's precision;
  the Mesa words differ by precision only where the table says so (the `acos`/`asin`
  fp32 form is the same at both).
- `acos`, `asin`, `atan`, `atan2`, `refract` and `faceforward` refuse first at an
  op the front end made of them (`fsign`, `frcp`, `bcsel`), so their cases appear under
  steps 7, 2 and 6 (`acos`/`asin` 16 under fs-round's `Fsign`, `atan` 8 under fs-div's
  `Frcp`, `atan2`/`faceforward`/`refract` 24 under fs-select): the shapes need
  all the steps their listing names (the `acos` sequence holds `FRSQ`, `FRCP`, `FCMP_OR`).
- The vertex stage's `lowp`/`mediump` ops read mediump attributes at fp16 in Mesa
  (the `mixprec` family's vertex side), fp32 in the vertex walk: steps 1, 5 and 8 may land as
  `DIFFERS` until that is done (the `deqp_intval_opvs_logical_and` case of
  `mali-deqp-intval.md`).

## 5. What this study does not say

- The counts are the FIRST refusal of a case. No lowering is forecast here beyond that:
  the repository's rule (no change under `src/`) and the cost of a host run per step
  put the finishing forecast (`deqp-host-measure.sh` after each step) in each step's
  own session.
- The 4,401 cases of the slices that compile at the tip are not in the table, nor the
  cases of other families whose NEXT refusal is one of the three texts (the
  `next_family` column of `measure.tsv` of the families note counts 72
  `367:FRAGCOLOR_LANE` and 21 `unitype` partners behind these cases).
- A reference is a dEQP case's shader pair as dEQP writes it (the first case of the
  bucket whose type is `float`, else the first): the other types of an op (`vec2`..`vec4`,
  `ivec`, `bvec`) have the same ops per lane and are not drawn separately; the shapes with
  under 3 cases outside the operator suites, and random buckets under 10, have none
  (list in section 2.1, Refs `-`).
- The dEQP random programs (368 cases) are listed by the op the walk refused first,
  which is a property of the generator's draw, not a shape of its own.

## 6. Reproduce

```sh
tools/mali/deqp-host-measure.sh --cts "$CONNECTED_DEPS_DIR/VK-GL-CTS" --out "$TMPDIR/measure" --jobs 3
# the refusing cases: host family 367:ARITH in $TMPDIR/measure/measure.tsv, sources in src/
# a reference, per shape:
tools/mali/deqp_ref.sh "$TMPDIR/measure/src/<case>.vert" "$TMPDIR/measure/src/<case>.frag" \
    tools/mali/testdata/deqp_arith_<shape>_<stage>_<precision> --instrument shim
cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_arith
```

The 144 pairs are the `.vert` / `.frag` beside each `expect.json` verbatim from the
dEQP log of the host run (a case's `.qpa` shader sources).
