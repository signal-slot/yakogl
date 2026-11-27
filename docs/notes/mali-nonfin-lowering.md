<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex non-finite constant refusal of the G310 run, what is left after the train, and Mesa's NaN word

**Status: design** (plan #387, item `nonfinite-design`; issue #20). Nothing
under `src/` changed. The references are
`tools/mali/testdata/deqp_nonfin_*` (seven pairs) and are pinned by
`tests/mali_deqp_nonfin.rs`. Measured HOST-ONLY at the integration tip
**`8133663ca5da`** and at the compiler train's result commit **`224cc3f77f4a`**
(item `intval-rest`, branch `connected/01a21c00-5315-…`, not landed yet).
Nothing in this note was run on a board. The board rows it starts from are the
Verdin iMX95 (Mali-G310) ones. Mesa paths are relative to `src/` of the
26.2.1 tree.

## 1. The text, the site, the cases

The text is `MaliError::ConstantFold` (`src/shaderc/online/mali/mod.rs:464`):
"a constant operation on or to a value that is not a finite normal float".

- **Board lines.** The consoles of the Verdin iMX95 whole-tree run
  (`tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e/console/*.log`,
  images from `e63ab28e`) hold **199** lines `refused this vertex shader:
  Mali(Valhall: a constant operation on or to …)`. Each is in a different
  case: Fail 126, NotSupported 73. The plan's figure of 196 lines came from
  another count; this note uses the 199 cases. The 48 fragment-shader lines of
  the same text are not this item's.
- **Groups of the 199.** `shaders.indexing` 126 (`tmp_array` 40,
  `varying_array` 32, `vector_subscript` 27, `matrix_subscript` 27),
  `shaders.random` 36 (`conditionals` 25, `all_features` 6, `comparison_ops`
  5), `light_amount` 16, `fragment_ops.interaction.basic_shader` 9,
  `shaders.constant_expressions` 9, `shaders.struct.uniform` 3.
- **Host measure.** `tools/mali/deqp-host-measure.sh --cts
  $CONNECTED_DEPS_DIR/VK-GL-CTS-host --cases <the 199> --slice
  tools/cts/log/2026-11-25-imx95-gles2-S4-e63ab28e`, run on the deqp-gles2
  that `deps-env.sh --build HOST_DEQP_CTS` built (VK-GL-CTS `93bca018`).
  All 199 cases began. At `8133663c`, **198 have this text first** and 1 has
  the constant-product text (`ArithConstant`).
- **Refusal sites.** A scratch probe (not committed) printed `file:line`, the
  op and the operand bits at every `MaliError::ConstantFold` in a copy of
  `nir.rs`/`nir_fs.rs` whose line numbers were left unchanged. It compiled
  each of the measure's 199 programs through `tests/mali_ref`'s
  `compile_pair`. At `8133663c` every one of the 198 stops in
  `Nir::alu_sized`'s all-constant branch:

| Site at `8133663c` | Op, operand bits | Cases | Groups |
|---|---|--:|---|
| `nir.rs:2358` ("the front end folds a constant expression" arm) | `Inot [0]`: the front end's `!false` of a loop's break and continue flags | 122 | `indexing` 110 (`varying_array` 32, `tmp_array` 32, `matrix_subscript` 25, `vector_subscript` 21), `random` 9, `light_amount` 2, `constant_expressions.operators` 1 |
| `nir.rs:2270` (the `plain` test: an int below 2^23 is a denormal as a float, `~0` a NaN) | `Icmp(Eq)` of int constants (`[0, 2]`, `[1, 1]`, `[0, 1]`, `[0, 26]`, `[fffffffc, fffffffc]`, ...): an unrolled counter and random int vectors | 69 | `random` 25, `indexing` 16, `light_amount` 14, `interaction.basic_shader` 8, `struct.uniform` 3, `constant_expressions` 3 |
| `nir.rs:2270` | `Inot [ffffffff]` | 5 | `constant_expressions.builtin_functions` 3, `random` 1, `interaction.basic_shader` 1 |
| `nir.rs:2270` | `Iabs [9]` | 2 | `constant_expressions.operators.math_int` |
| **Total** | | **198** | |

**No case meets a real infinity or NaN.** Every operand is an integer or
boolean bit pattern that the float test at `nir.rs:2268-2271` (`plain`,
`nir.rs:867`) or the integer arm at `nir.rs:2324-2358` refuses. The other
sites (`nir.rs:2275, :2318, :2386, :2936, :2941, :6175, :6902` and the
`nir_fs.rs` ones) are not reached. This is the same picture as
`docs/notes/mali-deqp-vsconst.md` (NanoPC-T6 cases, at `e65567ea`) and
mechanism A of `docs/notes/mali-deqp-cfold2.md`.

## 2. What the train does, and the residue

ConstantFold **step A** of `mali-deqp-cfold2.md` is in the train as commit
`e24a0d1d`. It adds `fold_int_alu` (`nir.rs:896` there), which sends every
32-bit integer and boolean op of constants (`Icmp`, `Inot`, `Ior`, `Ixor`,
`Iadd`, `Imul`, `Ineg`, `F2i`, `I2f`, `U2f`, `B2i`, `Isub`, `Iabs`) to
`nir::fold_int`, ahead of the float test (`nir.rs:2307` at `224cc3f7`).
It also folds `bcsel` of a constant condition. `mali-libm-fold.md`
(`mesa_fold.rs`) and `mali-constprod-lowering.md` plan nothing more for this
text in the vertex walk: their non-finite rule is "callers refuse non-finite
folds" (`src/shaderc/online/mesa_fold.rs:35`, `mali-libm-fold.md` section 5).

The same measure and probe at **`224cc3f7`** (the train's worktree; its
deqp-host-measure library built from that commit):

| First host refusal at `224cc3f7` | Cases | Groups |
|---|--:|---|
| **this text (`ConstantFold`)** | **0** | none; the probe sees no `ConstantFold` site at all |
| constant product (`ArithConstant`) | 107 | `indexing` 106, `random.conditionals` 1 |
| arithmetic (`Arithmetic`) | 39 | `random` 32, `interaction.basic_shader` 7 |
| a uniform past the FAU's 128 words (`UboFallback`) | 5 | `light_amount` (the 16-light ones) |
| other texts | 7 | `struct.uniform` 3 (2 `VaryingForm`, 1 `UniformPrecision`), `constant_expressions.operators` 2 (fs `IntValue`), `interaction` 1, `random` 1 |
| none: the pair compiles on the host | 41 | `light_amount` 11, `indexing` 20, `constant_expressions` 7, `random` 2, `interaction` 1 |

**The residue of the text after the train is 0 first-refusal cases.** That
does not mean no case can return to it. To see what a later walk would meet
once the texts in front are lowered, every one of the 199 programs was drawn
on Mesa 26.2.1's drm-shim with `NIR_DEBUG=print` (the shader after every
pass), and every `load_const` with an all-ones exponent was collected
(`fp32 & 0x7f800000`, `fp16 & 0x7c00`; the int masks `~0`, `-n` and
`0x7fffffff` of `fabs` were dropped by hand):

- **No infinity in any pass of any of the 199 programs.** No `rcp(0)`, no
  `rsq(0)`, no overflowing built-in. (cfold2's mechanism C, the
  viewport-transform clamp, has its own cases elsewhere.)
- **One NaN: `nir_opt_undef`'s**, in **22** programs. Each is one case, all
  board NotSupported:
  - `tmp_array.{float,vec2,vec3,vec4}_{dynamic,static}_loop_write_dynamic_loop_read_vertex`
    (8): NaN in the VS.
  - `matrix_subscript.mat{2,3,4}_{dynamic,static}_loop_write_dynamic_loop_read_vertex`
    (6): NaN in the VS.
  - `varying_array.{float,vec2,vec3,vec4}_{dynamic,static}_loop_write_dynamic_loop_read`
    (8): NaN in the FS, seen as fp32 `0x7fc00000` in an early pass and
    fp16 `0x7e00` at the end.

  All 22 stop at the constant-product text at `224cc3f7`. The other dynamic
  cases (`dynamic_loop_write_static_loop_read`, `..._static_read`,
  `..._dynamic_read` without a loop, `vector_subscript`) have no NaN left:
  `nir_opt_remove_phis` drops the undef source of a guarded write.

So the **residue** this design lowers is the fp16 NaN of an unwritten array
element read through a dynamic loop: 22 cases, 14 in the vertex walk and
8 in the fragment walk. Each sits behind two other steps (section 4).

## 3. How Mesa encodes it

`nir_opt_undef` (`compiler/nir/nir_opt_undef.c:240-272`) replaces an undef
that a float ALU op reads with `nir_imm_floatN_t(b, NAN, bit_size)`; panfrost
does not set `disallow_undef_to_nan`. The element the dynamic read loop reaches
past the last written one, after `partial_unroll` by the trip count guessed
from `arr[4]` (`compiler/nir/nir_opt_loop_unroll.c:1079-1087`,
`nir_loop_analyze.c:400-466`), is such an undef. At mediump it is fp16
`0x7e00` (`NAN` rounded by `_mesa_float_to_half`: quiet, positive, payload
0).

- **It is a FAU word, never an immediate.** `0x7e00` is not in Valhall's
  immediate table. bi packs the pair as one push constant **`7e007e00`**
  (the fp16 NaN in both halves) and reads it as the second source of a
  `MUX.v2i16` whose mask is the `ICMP_OR.s32.lt.m1` of the constant index
  against `ui_four`. There is one `MUX.v2i16` per 16-bit lane pair of the
  element: 1 for float, vec2 and vec3; 2 for vec4 and mat3 columns.
- The FAU slot comes after the driver's words (viewport, varying offsets,
  `ui_four`) and the folded constant products. In the seven new references:

| Reference | dEQP case (`shaders.indexing.`) | Stage, FAU word, readers | Region words (pos / varying / fs) |
|---|---|---|---|
| `tmp_float_static_dynread` | `tmp_array.float_static_loop_write_dynamic_loop_read_vertex` | VS `u6.w1` = `7e007e00` (`u6.w0` = `30003000`, 0.125 twice), `MUX.v2i16 r3, r3^.h00, u6.w1, r5^.h00` | 24 / 34 / 7 |
| `tmp_vec3_static_dynread` | `tmp_array.vec3_static_loop_write_dynamic_loop_read_vertex` | VS `u6.w1` | 24 / 46 / 7 |
| `tmp_vec4_dyn` | `tmp_array.vec4_dynamic_loop_write_dynamic_loop_read_vertex` | VS `u6.w1`, two `MUX.v2i16` (lanes xy, zw) | 24 / 50 / 7 |
| `matrix_mat3_dyn` | `matrix_subscript.mat3_dynamic_loop_write_dynamic_loop_read_vertex` | VS `u6.w0` (`u6.w1` = 0), two `MUX.v2i16` | 24 / 44 / 7 |
| `matrix_mat4_static_dynread` | `matrix_subscript.mat4_static_loop_write_dynamic_loop_read_vertex` | VS `u6.w1` | 24 / 58 / 7 |
| `varying_float_static_dynread` | `varying_array.float_static_loop_write_dynamic_loop_read` | FS `u2.w1` (`u2.w0` = `ui_four`), one `MUX.v2i16` | 24 / 21 / 23 |
| `varying_vec4_dyn` | `varying_array.vec4_dynamic_loop_write_dynamic_loop_read` | FS `u2.w1`, two `MUX.v2i16` | 24 / 83 / 44 |

The three references of `mali-deqp-vsconst.md` (`deqp_vsconst_dynloop_tmp_array`,
`_dynloop_matrix_subscript`, `_dynloop_varying_array`) are the float/mat2
dynamic-write members of the same set and show the same word. All seven new
ones give the same words on the G310v2 (`tools/mali/g310_check.sh
'deqp_nonfin_*'`: 7 identical, 0 differ).

## 4. The steps

The NaN is the last step of the 22 cases. Two steps that other items own come
before it, and the train comes before those.

| Step | What | Owner | Cases (HOST forecast, upper bound) | dEQP groups | First reference to un-ignore |
|---|---|---|--:|---|---|
| **NF0** | ConstantFold step A: integer and boolean constant ops through `nir::fold_int` before the float test | the train (`intval-rest`, commit `e24a0d1d`) | **199** leave this text (0 of 199 refuse with it first at `224cc3f7`); **41** compile on the host | `light_amount` 11, `indexing` static loops 20, `constant_expressions` 7, `random` 2, `interaction` 1 | (cfold2's, in the train) |
| NF1 | the unrolled `coords = coords * 0.5` chain: a constant times a product with a constant, folded to `30003000` / `30003400` | constprod (`mali-lowering-schedule.md` K2, "literal times a product"; assumed from the source shape, not probed) | 107 reach their next text; 48 static `indexing` cases may compile | `indexing.{tmp_array, varying_array, vector_subscript, matrix_subscript}` static 48, dynamic 58, `random.conditionals` 1 | `deqp_cfold2_zero_operand_inot`, `deqp_cfold2_const_other_inot` |
| NF2 | Mesa's `partial_unroll` of a dynamic loop over a constant-size array (guessed trip count, one `ICMP_OR.lt` and `MUX` per element) in the vertex walk and the fragment walk | loops (`mali-loops-lowering.md` S2; schedule LS2, S2-fs `loops-lower-1`, S2-vs `loops-lower-2`) | 58 dynamic `indexing` cases reach the guard and select; 36 of them need no NaN after it | `indexing.*dynamic_loop*`, `*_dynamic_read*` | `deqp_vsconst_mixloop_tmp_array` |
| **NF3** | **this design: the undef NaN as a FAU constant** (below) | the item that lowers this family after NF1 and NF2 | **22** (VS 14, FS 8) | `indexing.tmp_array` 8, `indexing.matrix_subscript` 6, `indexing.varying_array` 8: `{dynamic,static}_loop_write_dynamic_loop_read` | `deqp_nonfin_tmp_float_static_dynread`, then `deqp_nonfin_varying_float_static_dynread` |

NF3, the lowering:

1. **Where the undef is.** When NF2's partial unroll reads an element that no
   iteration wrote, the value is the walk's `Val::Undef`, not a constant.
   Where a mediump float lane of it reaches a select (the `bcsel` of the
   guard), make it the constant fp16 `0x7e00`, as `nir_opt_undef` does. It
   must be the undef's own lane: an undef that an integer op reads is 0 in
   Mesa (`nir_opt_undef.c:273-274`), and no reference here draws that.
2. **Do not open the fold to NaN.** The NaN enters as an operand of the
   select only. `plain` (`nir.rs:867`) and the result test
   (`nir.rs:2384-2387` at `8133663c`, `:2436-2439` at `224cc3f7`) keep
   refusing a NaN that an arithmetic fold would read or make. In Mesa,
   `nir_opt_undef` replaces the undef with NaN *so that* the float ops that
   consume it fold away. Our walk has to give the same words: a select is
   the only reader the references show.
3. **FAU word.** The 16-bit pair packer puts `0x7e00` into both halves of one
   push word (`7e007e00`). It takes the slot after the folded product
   constants: `u6.w1` in the VS when there is a `30003000`, `u6.w0` when
   there is none (`matrix_mat3_dyn`), and `u2.w1` in the FS after `ui_four`.
   The `MUX.v2i16`'s second source is that word, as is or `.h00`.
4. **fp32.** No case of the 199 has an fp32 undef NaN (`0x7fc00000`) left
   in its final program; the varying_array FS shows it only before
   `lower_mediump`. Refuse it by name until a reference draws one.

Forecast: NF0 alone takes all **199** G310 vertex lines of this text off the
board (HOST-ONLY; at the next G310 rerun after the train lands they should
show as the texts of section 2's table). NF3 is worth **22** cases, all
board NotSupported today, and only after NF1 and NF2. It is below the plan's
100-case line on its own, so it can follow NF2 in the same item rather than
take one of its own.

## 5. The test

`tests/mali_deqp_nonfin.rs`:
`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_nonfin`.
At `8133663c` and at `224cc3f7`: 2 pass, 7 ignored.

- `references_load` checks that every `deqp_nonfin_*` reference is listed, has
  its three regions, has `7e007e00` in the FAU of the stage named in
  `REFUSED`, and has a `MUX.v2i16`.
- `refusals_pinned` accepts the tip's `ConstantFold` text or the train's
  `ArithConstant` text, so it is green in either merge order. It fails when a
  pair compiles, which means: remove the `#[ignore]` and move the name to
  `MATCHED`. It also fails when a pair moves to a third text. When NF1 lands,
  replace the train text with the next one.

## 6. How this was measured (scratch, not committed)

- deqp-gles2 for the host: `tools/cts/deps-env.sh --build HOST_DEQP_CTS`,
  then `tools/mali/deqp-host-measure.sh --stage build`. The `224cc3f7`
  measure ran in a detached worktree of that commit, with its own library and
  the same deqp-gles2.
- Mesa: `tools/mali/drm_shim_ref.sh build mesa-26.2.1.tar.xz
  $CONNECTED_DEPS_DIR/mesa-shim` (`LLVM_DIR=/usr/lib/llvm-21`). The
  README-deqp_ref.md self-check (`gm2_cf_ifelse_vs`, `_fs`) was byte-equal.
  References: `tools/mali/deqp_ref.sh <vert> <frag>
  tools/mali/testdata/deqp_nonfin_<shape> --instrument shim`, the sources
  verbatim from the measure's `programs/`.
- The probe: every `Err(MaliError::ConstantFold)` and
  `.ok_or(MaliError::ConstantFold)` in `nir.rs` and `nir_fs.rs` was replaced
  in place by a block that prints `file!():line!()` (and the op and the
  constants at the three `alu_sized` sites). A scratch test then fed the
  measure's `programs/<id>.{vert,frag}` to `mali_ref::compile_pair`.
