<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2: the vertex uniform-array refusal, the residue after unif2 M1 (vsunifarr-design)

Plan #387, item `vsunifarr-design`: a design and Mesa references, no compiler
change. Nothing under `src/` changed and no board ran. The references are
`tools/mali/testdata/deqp_vsuarr_*`, pinned by `tests/mali_deqp_vsuarr.rs`.

## 1. The text, its site, its cases

- **Text:** `MaliError::UniformArray`, "Valhall: a uniform array is not
  lowered yet" (`src/shaderc/online/mali/mod.rs:430`).
- **Tip:** integration tip **`8133663c`** (`8133663ca5da6042860053ff2c68484fdcaf7c9a`).
- **Site:** one, `nir::uniform_of` at **`src/shaderc/online/mali/nir.rs:1103`**
  (`return Err(MaliError::UniformArray)`, in the `if u.array_len > 1` arm at
  `:1096`). It is reached for an array that `nir::array_element`
  (`nir.rs:1141`) does not take. That function takes `GL_FLOAT`,
  `GL_FLOAT_VEC2`, `GL_FLOAT_VEC3` and `GL_FLOAT_VEC4` elements only. So
  every int, uint, bool, ivec, bvec and matrix array is refused here, whether
  it is a plain uniform or a struct's member (the front end flattens a struct
  to leaf rows, and a member array is one leaf row with `array_len` > 1).
  `nir_fs::uniform_extent` (`nir_fs.rs:8916`) calls the same function for the
  highp fragment walk. The fragment shaders of these cases stop earlier, at
  `FsUniform`.
- **At the train's tip:** with the compiler train `intval-rest` applied
  (`224cc3f7`, not yet on the tip) the same `return` is at `nir.rs:1143`. The
  train does not change `uniform_of` or `array_element`. The pins of
  `tests/mali_deqp_vsuarr.rs` pass at `224cc3f7` too: it was checked in a
  scratch worktree, 2 passed, 8 ignored.
- **Cases:** the Verdin iMX95 (Mali-G310) run of 2026-11-25 has **193** lines
  of this text (`tools/cts/log/2026-11-24-imx95-gles2-S1b-e63ab28e/console/T6S1R19..22.log`,
  images from `e63ab28e`, measured on the iMX95). That is one line per case,
  193 distinct cases, all board Fail and all refused in the vertex shader.
  They are **exactly** the 193 cases of the NanoPC-T6 slice
  `2026-11-24-t6-gles2-S1-8ff01055/refusals.tsv` with this text: `comm` of
  the two lists is empty. Group split: `uniform_api.value.assigned` 118,
  `value.initial` 36, `info_query` 20, `random` 19.
- **Indices:** every index in the 193 shaders is a constant. No case reads
  one of these arrays at a run-time index (`mali-deqp-vsuniarr.md` 2 measured
  this at `ab919063`; the case list has not changed since). So Mesa pushes
  every word that is read, and no reference here or in `deqp_vsuniarr_*`
  has an `LD_PKA` or a UBO load. The largest table is `nested_mat`'s 108 FAU
  words. **Dynamic indexing does not occur in this text.** Its addressing
  (`LD_PKA` from UBO 0) is the `UniformIndexed` family's
  (`mali-deqp-uniarray.md`, `uniarray_dyn_*`). No reference was drawn for it
  here: a synthetic shape would be no dEQP shape.

## 2. What the earlier work covers

Three earlier sets of references already pin this text's assigned cases:

- `deqp_unif2_arr_{bool,int,mat,struct}_array`: plan #387 `unif-ref`, with
  the design in `docs/notes/mali-unif2-lowering.md`.
- `deqp_vsuniarr_*` (8): plan #402, `docs/notes/mali-deqp-vsuniarr.md`.
- `deqp_vsuniarr2_*` (8, `uniform_api.random`): `docs/notes/mali-deqp-vsuniarr2.md`.

All are drawn from the cases' own `.qpa` sources.

`mali-unif2-lowering.md` designs part of the lowering:

| unif2 step | What it lowers | Shapes of the 193 it covers |
|---|---|---|
| **U3** (M1) | `array_element` for `GL_INT`, `GL_UNSIGNED_INT`, `GL_BOOL` and their vectors: 32-bit words, an element a word per component, both stages (`UboLayout::fs16` for the fragment shader) | `basic_array.{bool,bvec4,int,ivec4}*`, `multiple_basic_array.*` (its float array is lowered already, its `ivec3[]`/`bvec2[]` are U3's), and the int/bool-only `random` cases |
| **U5** (M3) | struct leaf rows, member arrays of int/bool/ivec4/bvec4 | `unused_uniforms.bool_bvec4*` |
| **U8** (M6) | one fp16 `mat3` array element | `random.10` (= `deqp_unif2_arr_mat_array`) |
| U6 | the vertex bvec/bool compare chain (`vs_reduce`) | the compare words of the bool shapes after U3 |

The unif2 note designs these steps from the **assigned** references only: each
element is compared with a non-zero constant. Three things are left for this
note.

1. **Matrix arrays** other than U8's one mat3 element: plain `mediump`
   `mat2[]`, `mat4[]` (fp16 in Mesa's block), and struct-member `mat2[]`,
   `mat4[]` (fp32 words, narrowed in the shader).
2. **The zero compares** of the `value.initial` cases. Mesa folds them into
   one reduction and one compare (section 3). That is a different word
   sequence from every assigned reference, and no reference showed it before
   this item.
3. **The `random` cases that also declare a sampler** (6, 24, 37, 61). Their
   vertex shaders also need a texture: `TexVertex` / texmisc, not designed
   here.

## 3. Mesa's words for the residue (the new references)

The eight new references are the `value.initial` programs (`_vertex` cases)
of the eight shapes with a zero form. Each vertex shader was derived from the
verbatim `.qpa` source of its assigned sibling (`deqp_vsuniarr_*`,
`deqp_unif2_arr_*`) by the generator's own zero rule. In VK-GL-CTS `93bca01`,
`es2fUniformApiTests.cpp` `generateZeroVarValue` and `shaderVarValueStr`
write a float as `de::floatToString(0.0f, 2)` = `0.00`, an int as `0` and a
bool as `false`. The `_both`, `get_uniform` and `api_*` siblings have the same
vertex shader. The fragment shader is the vertex case's, the varying alone.
The `.license` beside each shader says so.

They were drawn with `tools/mali/deqp_ref.sh <vert> <frag> <prefix>
--instrument shim`, on the Mesa 26.2.1 drm-shim that `drm_shim_ref.sh build`
made from the pinned tarball (sha256 `c47e81bd…4442`, equal to
`third_party/mesa-26.2.1/UPSTREAM`) in `$CONNECTED_DEPS_DIR/mesa-shim`. The
README's self-check (`gm2_cf_ifelse_vs`, `_fs`) was byte-equal, and a redraw
of `deqp_vsuniarr_mat4` was equal except for the `trace` name. All eight
were drawn twice with identical results. `tools/mali/g310_check.sh
'deqp_vsuarr_*'` reports 8 identical, 0 differ, so the words serve the G310
too. Every reference has `vs.position` 24 words and `fs.fragment` 10. FAU
words 0 to 10 are the viewport and varying words, and the uniforms start at
word 11 (`u5.w1`).

| Reference | `vs.varying` words (the assigned sibling's) | FAU | Mesa's words |
|---|--:|--:|---|
| `init_bool` | 7 (`arr_bool_array`: see unif2 1.2) | 14 | `MOV.i32 r0, u5.w1`; `LSHIFT_OR.i32 r0, u6.w0, 0x0.b00, r0^`; `LSHIFT_OR.i32 r0, r0^, 0x0.b00, u6.w1`; `CSEL.u32.eq r0, r0^, 0x0, 0x3F800000, 0x0`; `FMA.f32` |
| `init_int` | 8 | 14 | `MOV.i32 r0, u5.w1`; `CSEL.u32.gt r0, u6.w0, r0^, u6.w0, r0^` (umax); again with `u6.w1`; `ICMP_OR.u32.eq.m1 r0, r0^, 0x0, 0x0`; `CSEL.u32.eq r0, r0^, 0x0, 0x0, 0x3F800000` |
| `init_bvec4` | 18 (20) | 24 | three `MOV.i32` + `LSHIFT_OR.i32` trees of four words each, **regrouped across elements** (r0 = words 11, 12, 18, 17; r1 = 13, 14, 20, 19), then joined by `LSHIFT_OR.i32`; one `CSEL.u32.eq` |
| `init_ivec4` | 19 (29) | 24 | the same tree with `CSEL.u32.gt` (umax) in place of `LSHIFT_OR`; one `ICMP_OR.u32.eq.m1 r0, r0^, 0x0, 0x0` |
| `init_mat4` | 53 (77) | 36 | fp16, 24 words (11 to 34). **No `FADD_IMM`**: `FMAX.v2f16 rX, uN.abs.h00, uN.abs.h11` per word (the second of a pair swapped `.h11, .h00`), a balanced `FMAX.v2f16` tree, one `FCMP_OR.v2f16.lt.m1 r0, r0^.h00, u17.w1, 0x0` (0.05 = `2a662a66` at FAU 35), `S16_TO_S32`, `CSEL.u32.eq` |
| `init_multi` | 34 (43) | 30 | the three fp16 floats `FMAX`ed with `.abs` and one `FCMP_OR.v2f16.lt.m1`; per ivec3 element a umax tree, then `ICMP_OR.u32.eq.m1 … 0x0, 0x0`; per bvec2 element `LSHIFT_OR.i32` then `LSHIFT_OR.i32 r2, 0x0, 0x0.b00, r2^.not`; all joined by `LSHIFT_AND.i32`. The words read are 11 to 28, in first-use order (floats 11, 12, 25; ivec3 13 to 18 and 26 to 28; bvec2 19 to 24) |
| `init_struct_mat` | 165 (205) | 92 | 80 fp32 words (11 to 90): 40 `MOV.i32` + `V2F32_TO_V2F16` pairs, then the zero tree of `init_mat4` (79 `FMAX.v2f16`), one `FCMP_OR.v2f16.lt.m1` with `u45.w1` |
| `init_nested_mat` | 197 (245) | 108 | 96 fp32 words (11 to 106): 48 narrowing pairs, 95 `FMAX.v2f16`, one compare with `u53.w1` |

The rules behind the zero forms are `nir_opt_algebraic.py` of Mesa 26.2.1:

- `:1381`/`:1383`: `iand(ieq(a, 0), ieq(b, 0))` → `ieq(umax(a, b), 0)`
  when `umax` is not lowered (panfrost; the `ior` form is for back ends
  without `umax`). That is the int `CSEL.u32.gt` tree.
- For bools, `a == false` is `inot(a)`, and the conjunction of `inot`s is
  `inot(ior(...))`. That is the `LSHIFT_OR.i32` tree and a 1.0-on-zero
  `CSEL.u32.eq` (`init_bool`, `init_bvec4`), or an explicit `.not` where the
  mask is joined with others (`init_multi`).
- `:1159`/`:1163`: `iand(flt(a, c), flt(b, c))` → `flt(fmax(a, b), c)`. With
  `a - 0.0` folded to `a`, `abs(a) < 0.05` for every lane becomes one `fmax`
  tree of `fabs` and a single `flt`. The assigned matrices' `FADD_IMM` of the
  negated constants is gone.

The push order is Mesa's first-use order (`bi_opt_reorder_push`), which
`fau.rs` already models (`push_ubo`, the comment at `fau.rs:102-112`). In
`init_multi` and `init_bvec4` that order follows the reassociated trees and
not the declaration. A lowering that builds the trees in Mesa's order gets
the FAU order for free. One that builds them in source order gets both the
instructions and the FAU indices wrong.

## 4. Steps (one session each, after the train and after unif2 U3)

The vertex shaders' first refusal is this text in every step. A step's
**text** count is the number of cases whose vertex shader then compiles
(the host measure no longer reports this text). Its **pass** count is the
part that can then pass on host-compile terms: the `_vertex` cases, whose
fragment shader is the varying alone. A `_both` case then stops at its
fragment shader's refusal: `FsUniform` for an int/bool/matrix array (unif2
U3's fragment half, or the next step for matrices), followed by the
intval-rest compare and fclane lane words. `random` cases are counted in
**text** only, because their fragment shaders were not all read.

| Step | What | Prerequisite | References that turn byte-equal | dEQP groups | Text | Pass |
|---|---|---|---|---|--:|--:|
| (U3) | unif2's int/bool/ivec/bvec array element rule, assigned compares | intval-vs16 (merged), unif2 U1/U2 | `deqp_unif2_arr_{bool,int}_array`, `deqp_vsuniarr_{bvec4,ivec4,multi}`, `deqp_vsuniarr2_{ivec3,bvec3}_part` | `uniform_api.value.assigned.*.{basic_array,multiple_basic_array}.*`, `info_query.{basic_array,multiple_basic_array}.*` (bool, bvec4, int, ivec4), `random.66,67,76,77` | 106 | 51 |
| **VA1** | zero-compare reduction in the vertex walk: an `iand` chain of `ieq(x, 0)` / `inot(b)` / `flt(fabs(x), c)` is built as Mesa's single `umax` / `ior` / `fmax` tree and one compare, leaves in Mesa's reassociated order (`init_bvec4`: across elements). Site: the vertex compare-product chain (`nir::vs_reduce`, `nir.rs:4417`, the same place U6 extends) and the int/bool compare words of intval step 3 | U3 (the arrays load) | `init_bool`, `init_int`, `init_bvec4`, `init_ivec4`, `init_multi` | `uniform_api.value.initial.{render,get_uniform}.{basic_array.{bool,bvec4,int,ivec4},multiple_basic_array}.*` | 24 | 12 |
| **VA2** | plain mediump matrix arrays: `array_element` takes `GL_FLOAT_MAT2`/`MAT3`/`MAT4` with Mesa's fp16 element stride (mat2 2 words, mat3 6 (U8's xy / z+pad columns), mat4 8); `fp16_uniform`'s array guard admits them. This generalises **U8**, so do it in its place | U3 (array rows in `UboLayout::vs`) | `deqp_vsuniarr_mat4`, `deqp_unif2_arr_mat_array`, `deqp_vsuniarr2_{mat3_part,mat4_mixed}`, then `init_mat4` with VA1 | `uniform_api.*.basic_array.mat4*`, `random.10,17,53,71,75,99` | 22 (+4 with VA1) | 8 (+2) |
| **VA3** | struct-member matrix arrays: fp32 words in declaration order, each column pair narrowed (`MOV.i32` + `V2F32_TO_V2F16`, as `vs_mediump_struct`), a member array of `mat2`/`mat4` and the nested `ms2[k].mss*`; only the words read are pushed (`unused_mat2`) | **U5** (struct leaf rows), VA2 (the matrix element rule) | `deqp_vsuniarr_{struct_mat,nested_mat,unused_mat2}`, `deqp_vsuniarr2_{struct_mat4_arr,mat2_struct,struct_vec3_mat2,struct_nested}`, then `init_struct_mat`, `init_nested_mat` with VA1 | `uniform_api.*.{array_in_struct,nested_structs_arrays}.mat4_mat2_*`, `unused_uniforms.mat4_mat2_*`, `random.15,57,74,83,94` | 21 (+8 with VA1) | 8 (+4) |
| (U5) | unif2's struct member arrays of bool/bvec4 | U3 | `deqp_vsuniarr_unused_bvec4` | `uniform_api.*.unused_uniforms.bool_bvec4_*` | 4 | 2 |
| not here | the `random` cases that also declare a sampler (a texture lookup in the vertex shader: `TexVertex`, texmisc design) | VA2/VA3 and texmisc | none | `random.6,24,37,61` | 4 | 0 |
| **Total** | | | | | **193** | **87** |

Check: 106 + 24 + 26 + 29 + 4 + 4 = 193. The pass column is the 87
`_vertex` cases (`value.*`, `info_query`) of the 174 named-shape cases. The
other 87 are `_both`, and they also need the fragment half of the same
uniforms.

Step order for the schedule (`schedule-refresh-2`): **U3 → VA1 → VA2 (with
U8) → U5 → VA3**. VA1 is the largest residue (24 text, plus 12 more through
VA2 and VA3) and changes only the compare chain. VA2 needs no struct work.
VA3 waits on U5's leaf rows. Each step removes its `#[ignore]`s and moves
the references from `REFUSED` to `MATCHED` in `tests/mali_deqp_vsuarr.rs`,
`tests/mali_deqp_vsuniarr.rs` and `tests/mali_deqp_vsuniarr2.rs`.

How these counts were made: the 193 are the G310 log's case list (it is
identical to the T6 slice's). Each case was put in a step by its case name
(the `uniform_api` group and the shape in its name, `value.initial` → VA1).
The 19 `random` cases were assigned from the shapes `mali-deqp-vsuniarr.md` 2
and `mali-deqp-vsuniarr2.md` 1 list for them (`random.83` from the
"struct members ... arrays" list, unread here). This is a host forecast and
an upper bound. It assumes the compare words of intval step 3 / U6 for the
int/bool compares, and the fp16 `FMAX` / `FCMP_OR.v2f16` words that
`deqp_unitype_vsapi_mat2` already pins for the matrices. It was not run
through `deqp-host-measure.sh` (no host dEQP build here). The board rerun
after the steps decides.

## 5. Risks and open points

- **The unif2 forecast:** `mali-lowering-schedule.md` 4c credits U3 with
  137 of this text's cases ("Q17 56"). By case name, U3's assigned shapes are
  106 of the 193. The initial (zero) cases of the same arrays (24) need VA1
  as well, and the matrices (26 + 29) need VA2/VA3. With U3 alone the text
  stays at about 87, not 56.
- `init_bvec4`'s regrouping across elements is the reassociation of a long
  `iand` chain (`nir_opt_reassociate` / the `iand` tree that
  `nir_opt_algebraic` builds). Only the reference tells the leaf order, so
  VA1's implementation should be checked against `init_bvec4` and
  `init_ivec4` first.
- The struct references carry gl_ref.py's member values (16 (k + 1) + c).
  The plain arrays' words are 0 (gl_ref.py sets no plain array element;
  `README-deqp_ref.md`, "Struct uniforms"). The words of these references
  do not depend on the values (every compare constant is 0, and 0.05 is an
  FAU constant), so this is no limit here.
- Not checked on any board. G310 = G610 holds for the eight references
  (`g310_check.sh`).

## 6. Landed: U3's residue and VA1 (plan #387, item `mali-train-3b`)

HOST-ONLY; nothing here ran on a board. The references were redrawn on Mesa
26.2.1's G610 drm-shim (`$CONNECTED_DEPS_DIR/mesa-shim`) with
`NIR_DEBUG=print`, which prints the shader after every pass; the rules below
are read from that, not guessed from the words.

- **A vector's `==`** (`nir.rs` `vs_reduce`): `ball_iequalN` is cut into its
  lanes last lane first (`lower_reduction`'s reverse order,
  `nir_lower_alu_width.c:228`): `iand(iand(iand(w, z), y), x)`. Of booleans
  a lane expected true is the word and one expected false its `inot`, and a
  run of `inot`s is one `ior` negated once (De Morgan). Of integers against
  zeros it is one `umax` chain and one `ieq`; against non-zero constants the
  `ieq`s' `iand` chain. A constant of zero and non-zero int lanes is refused
  (no reference).
- **Section 3's "regrouped across elements" is not a regrouping.** The
  leaves of `init_bvec4` and `init_ivec4` are each element's own chain, w
  and z first. The FAU slots look regrouped because `bi_opt_reorder_push`
  gives the first slots to the word pairs one instruction reads (w and z of
  each element) and the unpaired words (x, y) the rest. `fau.rs` already
  models that pass, so a tree built in Mesa's order gets the slots.
- **The product of the compare results** (`nir.rs` `vs_chain_step`,
  `vs_mask_tree`). `v = 1.0; v *= compare_a(..); v *= compare_b(..)` is, in
  Mesa's NIR, a nest of selects: each `fmul(v, b2f(c))` becomes
  `bcsel(c, v, 0.0)`, the last statement's outermost, and
  `bcsel(a, bcsel(b, x, 0.0), 0.0)` becomes `bcsel(iand(a, b), x, 0.0)`.
  `nir_opt_algebraic` visits the last instruction first and does not visit
  a select it has just made again, so each run joins the selects in pairs
  from the outermost, and the next run joins the pairs. In the first run
  the second factor joins the first at once, and the third joins that pair
  when the count is odd. Nine factors (`init_multi`) give
  `iand(iand(iand(c9, c8), iand(c7, c6)), iand(iand(c5, c4), iand(c3, iand(c2, c1))))`.
  Each `iand` then takes nir_opt_algebraic's own rules: two fp16 `flt`s of
  one constant are `flt(fmax(a, b), c)`, two zero tests
  `ieq(umax(a, b), 0)`, two negations the `inot` of their `ior`.
- **`inot` moves like a compare.** `nir_alu_instr_is_comparison` takes
  `inot` (`nir.c:3313`), and `nir_opt_move` runs twice on a vertex shader
  (before and after `pan_nir_lower_bool_to_bitsize`), so an `inot` an `iand`
  reads beside a compare ends up after that compare.
- **Refused by name, not compiled:** a chain of more than three factors
  that holds a scalar `bool == false` (its `inot` exists before the product
  is a select, and `bcsel(inot(a), b, c)` swaps the arms: another join
  order, `init_bool`'s) or a factor that is itself a product
  (`compare_vec4`'s `compare_float(..) * ..`, which Mesa makes one condition
  in a later run); and any such chain that also holds a vector's `==`.
  `deqp_vsuniarr2_bvec3_part` and `deqp_uniarray_struct` are these. Before
  this step `deqp_uniarray_struct`'s vertex shader compiled to words that
  were not Mesa's (hidden behind its fragment shader's refusal).

References lifted: `deqp_vsuarr_init_{bvec4,ivec4,multi}`,
`deqp_vsuniarr_{bvec4,ivec4,multi,unused_bvec4}`,
`deqp_vs16ref_uapi_{bvec3,bvec4}_mix`. Vertex shaders that now compile to
Mesa's words while the pair still stops at its fragment shader (measured with
the reference's vertex shader beside a fragment shader that reads the varying
alone): `deqp_unif2_vs_struct_uniform`, `deqp_vsuniarr2_ivec3_part`,
`deqp_uniarray_api_float` (its three `compare_float` statements were
`fmax(fmax(F0, F1), F2)` before; Mesa's is `fmax(F2, fmax(F1, F0))`).
