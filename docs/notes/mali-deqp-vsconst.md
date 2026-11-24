<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex-shader `ConstantFold` refusal at 8ff01055, its new shapes and Mesa 26.2.1 references

**Status: study** (plan #402, item `ref-vsconst`; issue #20). Nothing under
`src/` changed. The references are `tools/mali/testdata/deqp_vsconst_*`, and
`tests/mali_deqp_vsconst.rs` pins them. Measured HOST-ONLY at the integration
tip **`e65567ea3dc6`** (2026-11-24). Nothing in this note was checked on the
NanoPC-T6. Mesa paths are relative to `src/` of the 26.2.1 tree.

The refusal is `MaliError::ConstantFold` (`src/shaderc/online/mali/mod.rs:464`):
"a constant operation on or to a value that is not a finite normal float".
On the board (`docs/es2-cts-failures.md` section M) the vertex-shader text
grew from 114 Fail at `37305608` to 144 Fail plus 73 NotSupported at
`8ff01055`. `docs/notes/mali-deqp-cfold2.md` (plan #387, at `5cb80624`) drew
the shapes of that time. This note finds what has been added since then,
draws references for it, and names the earlier reference that covers each of
the other shapes. Plan #367's `tests/mali_nonfinite_const.rs` does not exist
at the tip, so it covers nothing here.

## 1. How this was measured

- **Cases:** the 259 cases whose `refusals.tsv` text in the slices
  `tools/cts/log/2026-11-24-t6-gles2-S{1..4}-8ff01055/` holds this refusal:
  226 vertex-shader and 33 fragment-shader rows.
- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS" --jobs 3 --slice <each of the four>
  --cases <the 259 names>`. `--filter` matches case names, not refusal text,
  so the list went in through `--cases`. VK-GL-CTS is at `93bca018` with
  `cts-yakogl-target.patch`, fetched shallow. Only the external sources that
  the configure step needs were fetched. All 259 cases began, and 3 processes
  exited 1.
  - 240 cases have a host refusal. **218 have this text first** (VS 188,
    FS 30), 20 have #367's arithmetic or fused multiply-add text first, and 2
    have other texts.
  - 19 cases compile on the host (10 of them Pass, 9 Fail on the board).
    The measure's own `not-reproduced.tsv` lists them; this note does not
    use them.
- **Refusal sites:** a scratch probe outside the commit (a detached worktree
  whose `nir.rs` and `nir_fs.rs` print `file!():line!()`, the op and the
  operand bits at every `MaliError::ConstantFold`, with line numbers
  unchanged) compiled every program in the measure's `programs.tsv` through
  `tests/mali_ref`'s `compile_pair`. Each of the 218 cases refuses at exactly
  one of three sites: `nir.rs:2325` (an operand that is not `plain`: an int
  bit pattern read as a float), `nir.rs:2412` (the arm "the front end folds
  a constant expression": `inot`, `iand`, `flog2`), and `nir_fs.rs:6999` (a
  `1/c` that is not exact in fp16). No case refuses at the `rcp`/`rsq` of
  0.0 sites (cfold2's mechanism C); issue #11328's viewport-transform clamp
  and constrnd's lowering took those.
- **References:** `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_vsconst_<shape> --instrument shim`, run on the
  drm-shim that `tools/mali/drm_shim_ref.sh build` made from the pinned
  `mesa-26.2.1.tar.xz` into `$CONNECTED_DEPS_DIR/mesa-shim`. The README's
  self-check (`gm2_cf_ifelse_vs`) was byte-equal. The pairs are verbatim
  from the measure's `.qpa` (the refusing program of the case). The
  `.license` beside each shader names the case.

## 2. The shapes at the tip (HOST-ONLY)

| Shape | Cases | Board Fail / NS | Stage, site, op (operand bits) | Reference |
|---|--:|---|---|---|
| **`dynloop`**: a `shaders.indexing` loop to a uniform bound (`for (int i = 0; i < ui_four; i++)`), both the write and the read (`tmp_array` 12, `varying_array` 12, `matrix_subscript` 9) | **33** | 0 / 33 | VS, `nir.rs:2412`, `Inot [0]` | **new**: `dynloop_tmp_array`, `dynloop_varying_array`, `dynloop_matrix_subscript` |
| **`mixloop`**: one loop to a uniform bound, the other static (`tmp_array` 8, `varying_array` 8, `matrix_subscript` 6, `vector_subscript` 3) | **25** | 0 / 25 | VS, `nir.rs:2412`, `Inot [0]` | **new**: `mixloop_tmp_array`, `mixloop_varying_array`, `mixloop_vector_subscript` |
| **`fsdiv_highp`**: `shaders.matrix.div.const_highp_mat{2,3,4}_mat*_fragment` | **3** | 3 / 0 | FS, `nir_fs.rs:6999`, `rcp` of `3f333333` (0.7), `3f99999a` (1.2) | **new**: `fsdiv_highp_mat2`, `fsdiv_highp_mat3` |
| `staticloop`: `shaders.indexing` static loops, which Mesa unrolls | 68 | 54 / 14 | VS, `nir.rs:2412` `Inot [0]` 52; `nir.rs:2325` `Icmp(Eq)` 16 | covered: `deqp_cfold2_zero_operand_icmp`, `_zero_operand_inot`, `_const_other_inot` |
| `random_int`: int and bool constants in `shaders.random.*` and `fragment_ops.interaction.basic_shader` | 35 | 35 / 0 | VS, `nir.rs:2325` `Icmp(Eq)` 26 (`[0, 2]`, `[1, 1]`, `[fffffffc, 1]`, ...), `Inot [ffffffff]` 2; `nir.rs:2412` `Inot [0]` 7 | covered: `deqp_cfold2_random_mixed_icmp`, `_random_mixed_inot`, `deqp_constrnd_int_bits_vs` |
| `light_amount`: the unrolled light loop | 16 | 16 / 0 | VS, `nir.rs:2325` `Icmp(Eq)` 14; `nir.rs:2412` `Inot [0]` 2 | covered: `deqp_intval5_light_amount_2_2`, `_0_16` |
| `constexpr_int`: `constant_expressions` int and bool | 9 | 9 / 0 | VS, `nir.rs:2325` `Icmp(Eq)` 3, `Inot [ffffffff]` 3, `Iabs [9]` 2; `nir.rs:2412` `Inot [0]` 1 | covered: `deqp_cfold2_const_call_other_bvec`, `_const_call_other_ivec`, `_const_other_iabs` |
| `struct_loop`: `shaders.struct.uniform.loop_struct_array_*` | 2 | 1 / 1 | VS, `nir.rs:2325`, `Icmp(Eq)` | covered: `deqp_cfold2_zero_operand_icmp` (the same unrolled counter) |
| `fs_log2`: `log2`, `log`, `pow` of constants | 12 | 12 / 0 | FS, `nir.rs:2412`, `Flog2` of `42c80000` (100.0), `3fd9999a` (1.7), `42287ae1` (42.12) | covered: `deqp_cfold2_const_call_transcendental_log2`, `_pow` |
| `fs_atan`: `atan` of constants | 4 | 4 / 0 | FS, `nir.rs:2412`, `Iand [40000000, 80000000]` | covered: `deqp_cfold2_const_call_transcendental_atan` |
| `fs_rcp16`: mediump and lowp `matrix.div.const_*` (6), `shaders.random` fragment (5) | 11 | 11 / 0 | FS, `nir_fs.rs:6999`, `rcp` of `0`, `3f333333`, `3f99999a`, `3f600000`, `bfa00000` | covered: `deqp_cfold2_zero_operand_rcp16`, `_const_other_rcp16`, `_random_mixed_rcp16` |
| **Total** | **218** | 145 / 73 | VS 188 / FS 30 | 8 new references |

Every case the board counted NotSupported is in `dynloop` or `mixloop`, except
14 in `staticloop` and 1 in `struct_loop`. These 58 loop cases account for
the growth since `37305608`. Before, they stopped at the vertex loop refusal
("a loop in a vertex shader"). The loop lowerings since then walk them, and
they first stop at the `inot` of a constant false. The front end makes that
`inot` for the loop's break and continue flags. The partner shaders
(the measure's `next_family`) of the two new VS shapes: 40 have no
refusal; 8 refuse with #367's arithmetic text, 4 with `mixprec`, 4 with
`loops` and 2 with #367's colour-lane text.

## 3. What Mesa makes of each value (the FAU words)

`nir_opt_constant_folding` (`compiler/nir/nir_opt_constant_folding.c`) folds
every op of section 2 whose operands are all constants. It runs in
`bi_optimize_loop` (`panfrost/compiler/bifrost/bifrost_nir.c:202-273`)
beside `nir_opt_loop_unroll`, `nir_opt_remove_phis` and `nir_opt_undef`. The GL
path sets no float controls, so the fold neither flushes denormals nor drops
infinities. A folded value that a program still reads lands in the FAU
(`u<n>.w<k>`) or in a lookup-table immediate. The words below are from the
`.expect.json` files.

### 3.1 `dynloop_*`: the loop removed, a guard per element, and fp16 NaN

- `inot(false)` folds to `true`, and so does each compare of the counter
  with a constant. No word of these is left.
- The loop has a uniform bound, but `arr[i]` indexes a `float arr[4]`.
  `nir_loop_analyze` guesses a trip count of 4 from the array access
  (`compiler/nir/nir_loop_analyze.c:400-466`, `find_array_access_via_induction`).
  `nir_opt_loop_unroll` then calls `partial_unroll` with it
  (`compiler/nir/nir_opt_loop_unroll.c:1079-1087`). The result is
  straight-line code: per element, one `ICMP_OR.s32.lt.m1` of the constant
  index against `ui_four` and one `MUX.v2i16`.
  - The constant indices 0..4 are not FAU words. They are bytes of the
    Valhall immediate table: `0x0`, `0x1000000.b3` (1), `0x3020100.b2` (2),
    `0x3020100.b3` (3) and `0x7060504.b0` (4).
- `ui_four` is FAU `u5.w1` in the VS of `dynloop_tmp_array` and
  `dynloop_matrix_subscript`, and `u2.w0` in the FS of
  `dynloop_varying_array`. Its word is `00000010`, the value that
  `gl_ref.py --uniform-values` sets; the driver fills it.
- An element that no iteration wrote is an undef that a float operation
  reads. `nir_opt_undef` replaces it with NaN
  (`compiler/nir/nir_opt_undef.c:262-272`). At mediump it is fp16 `0x7e00`,
  so the FAU word is **`7e007e00`** (`u6.w1` in `dynloop_tmp_array`; the VS
  of `dynloop_matrix_subscript`; the FS of `dynloop_varying_array`). This is
  a NaN that Mesa keeps in the program on purpose: the vertex walk's
  `plain` test (`nir.rs:2323-2325`) and the "NaN has no reference" rule of
  the fold (`nir.rs:2439-2441`) both refuse it today.
- The constant products of the write loop (`coords = coords * 0.5`) fold
  in fp16 after `nir_lower_mediump`: `0x38000000.h11` (0.5) and
  `0x34000000.h11` (0.25) are table immediates, and `u6.w0` =
  **`30003000`** (0.125 in both halves).

### 3.2 `mixloop_*`: a guarded write, a static read

- In `mixloop_tmp_array` (a guarded write, then a static loop reads all
  four), the undef of an unwritten element meets the written value in a
  phi. `nir_opt_remove_phis` drops the undef source, so no guard is left.
  The varying is `x + x/2 + x/4 + x/8` in fp16: `FMA.v2f16` with the
  0.5/0.25 table immediates and `u5.w1` = `30003000`. Its VS varying region
  is 23 words and has no `ICMP`.
- In `mixloop_varying_array`, the VS writes the varyings
  (`0x38000000.h11`, and `u5.w1` = `30003400`, which is 0.25 and 0.125). The
  FS sums all four with no compare (12 words).
- `mixloop_vector_subscript` is the one reference in which Mesa **keeps a
  loop**. `tmp[i]` of a `vec2` is a vector component, not an array access,
  so no trip count can be guessed. The read loop to `ui_two` stays:
  `ICMP_OR.s32.ge` against `u5.w1`, `BRANCHZ.reconverge ... offset:5`, and
  the back edge `BRANCHZ.eq.reconverge 0x0, offset:-7`. FAU `u6.w0` =
  `ffff0000` is the `MUX` mask of the unrolled static write loop. A
  lowering of this shape also needs the vertex-shader loop
  (plan #402 `lower-vsloop`).

### 3.3 `fsdiv_highp_*`: an fp32 quotient narrowed to the fp16 colour

- The colour is constant. The fragment program is 7 words: `ATEST` and two
  `MOV.i32` of FAU `u2.w0`, `u2.w1` into the fp16 `BLEND`.
- `fsdiv_highp_mat2`: **`4100bccd 3c003800`**, the lanes `bccd` (-1.2),
  `4100` (2.5), `3800` (0.5) and `3c00` (1.0). Lane 2 is `0.7/0.7 +
  0.4/-0.8`, folded at fp32 (`1.0 + -0.5`, exact) and rounded to fp16:
  `0x3800`. The mediump case folds `fmul(a, frcp(b))` at fp16 and gives
  `0x37fe` (`deqp_cfold2_const_other_rcp16`). So this is a different shape
  for the same text.
- `fsdiv_highp_mat3`: **`b639bcab 3c003db7`**, against the mediump case's
  `b638bcab 3c003db6`. A host recomputation of `res[0] + res[1] + res[2]`
  with fp32 quotients (either `a / b` or `a * (1 / b)`, rounded at fp32 after
  each op) and a final round to nearest even in fp16 gives exactly `bcab`,
  `b639`, `3db7`.
- Our fragment walk takes the quotient as mediump and refuses `rcp(0.7)`
  because the operand is not exact in fp16 (`nir_fs.rs:6993-7000`). The
  lowering must fold a highp constant quotient at fp32 and round once to
  fp16 at the colour.

## 4. For the lowering (plan #402 `lower-vsconst`)

1. **`inot` of a constant, and the int/bool constant ops, in the vertex
   walk** (`nir.rs:2412`, `:2325`). This is cfold2's step A: send them to
   `nir::fold_int` before the `plain` test. On its own it covers the
   `staticloop`, `random_int`, `light_amount`, `constexpr_int` and
   `struct_loop` shapes (130 cases, under the cfold2 references).
2. **The guessed-trip-count unroll of `dynloop`/`mixloop` (58 cases)**:
   after step 1 these reach the vertex loop. The byte-equal program is
   Mesa's partial unroll by the array length, with a compare and a `MUX` per
   element and the fp16 NaN `7e007e00` for an unwritten element. The fold
   must let that NaN through as a FAU word. `mixloop_vector_subscript` needs
   a real VS loop instead (`lower-vsloop`).
3. **`fsdiv_highp` (3 cases)**: the fp32 fold of a highp constant quotient
   in the fragment walk (section 3.3). The 11 `fs_rcp16` cases are cfold2's
   step D.
4. `fs_log2` and `fs_atan` (16) are cfold2's step B (a front-end fold with
   correctly rounded `log2f`, `powf`, `atanf`).

## 5. The test

`tests/mali_deqp_vsconst.rs`:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_vsconst`.
At `e65567ea`, 1 test passes (`refusals_pinned`) and 8 are ignored. Each
reference is `#[ignore]`d with the `ConstantFold` refusal quoted and pinned
in `REFUSED`. `refusals_pinned` fails when a `deqp_vsconst_*` reference is in
neither `REFUSED` nor `MATCHED`, or when a pinned refusal moves. With
`--ignored`, each of the 8 fails only because the pair is refused.

## 6. Issue comment text (for #20; agents do not post it)

> VS ConstantFold ("a constant operation on or to a value that is not a
> finite normal float"), HOST-ONLY at `e65567ea`: of the 259 board cases
> with this text at `8ff01055`, 218 still refuse with it first (VS 188,
> FS 30). A site probe puts each at one site: `nir.rs:2412` (`inot(0)`,
> `iand`, `flog2`) 136, `nir.rs:2325` (int bits as float) 68,
> `nir_fs.rs:6999` (inexact fp16 `rcp`) 14. The growth since `37305608` is
> 58 `shaders.indexing` loop cases to a uniform bound (`dynloop` 33,
> `mixloop` 25), all board NotSupported. They used to stop at the VS loop
> refusal and now reach the `inot` of the front end's loop flags. Mesa
> removes those loops: `partial_unroll` by the trip count guessed from the
> array size, one `ICMP_OR.lt ui_four` and `MUX` per element, and
> `nir_opt_undef`'s fp16 NaN `7e007e00` in FAU for an unwritten element.
> Only `vector_subscript` keeps a real loop. A third new shape is
> `matrix.div.const_highp_*` (3): Mesa folds at fp32 and rounds once to the
> fp16 colour (`4100bccd 3c003800`, where the mediump case gives `37fe`).
> 8 Mesa 26.2.1 references, `tools/mali/testdata/deqp_vsconst_*`, pinned
> by `tests/mali_deqp_vsconst.rs`. The other 157 cases are shapes that the
> `deqp_cfold2_*`, `deqp_constrnd_*` and `deqp_intval5_*` references
> already cover. Details: `docs/notes/mali-deqp-vsconst.md`.
