<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): lowering the mediump fragment-shader uniform refusal (`FsUniform`) to Mesa's words

**Status: design** (plan #387, item `unifmp-design`). Nothing under `src/`
changed. The references are `tools/mali/testdata/deqp_unifmp_*` (12 pairs),
pinned by `tests/mali_deqp_unifmp.rs` (feature set
`--no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table`).
Everything here was measured **HOST-ONLY at the integration tip
`8133663ca5da`** (2026-11-27). Line numbers are of that commit. Nothing
was run on a board. The G310's shader words are the G610's
(`docs/notes/mali-g310-vs-g610.md`), so every reference is the G610
drm-shim's, and every step serves both the Verdin iMX95 and the NanoPC-T6.
Earlier studies of the same refusal: `docs/notes/mali-deqp-fsunif.md`
(sites at `7462e3d5`, eight references, `struct_basic` lowered) and
`docs/notes/mali-deqp-fsunif2.md` (five more, `select.rs:628`);
`tests/mali_deqp_fsunif3.rs` drew eight `uniform_api.random.*` programs.

## 1. The text and its sites

`MaliError::FsUniform` (`src/shaderc/online/mali/mod.rs:146`, message at
`mod.rs:437-439`):

> Valhall: a mediump fragment shader uniform other than a float, vec2, vec3,
> vec4, vec4 array or int read in its own fp16 lane pairs or as one lane, or
> one declared highp, is not lowered yet

It has 37 raises in `src/shaderc/online/mali/{mod,select,nir_fs,vec16}.rs`.
A scratch probe outside the commit (a copy of the tree in `$TMPDIR` where
every raise reports `file!():line!()` through a lazy `ok_or_else`, so no
line moved) compiled every program of the measured cases. Eight raises
end a compile:

| Site (tip `8133663c`) | Code | What it refuses |
|---|---|---|
| `select.rs:628` | `UboLayout::fs16`, the `_ =>` arm of the match on `u.gl_type` | a read uniform whose type no arm takes: an `int`, `bool`, `ivecN` or `bvecN` array (`array_len > 1`), a `mat2`/`mat3`/`mat4` array, the same as a struct member (a member is a row of the flattened list) |
| `nir_fs.rs:10805` | `lane_tree`, the fp16-block arm, `(Some(_), ..) \| (_, Some(_), ..)` | one half of the fp16 uniform block in a word whose other lane is not a uniform half: a varying's lane, a computed lane, a constant outside the colour's last word |
| `nir_fs.rs:10792` | `lane_tree`, the same arm, `(Some(d), Some(e))` | two halves of the fp16 block in one word that are neither one half twice nor a uniform's own aligned pair (`H01`): swapped, from two words, from two uniforms |
| `nir_fs.rs:10318` | `lane_tree`, the fp32-uniform arm, `(Some(d), Some(e))` | two fp32 uniform words (declared highp, or a struct's member) in one word that are not consecutive words of one load |
| `nir_fs.rs:10335` | the same arm, `_ =>` | an fp32 uniform word beside a lane that is no fp32 uniform word (and not the `Leaf::WideInt` colour pair) |
| `nir_fs.rs:10360` | `lane_tree`, `Leaf::IntWide` | two lanes of int uniforms' floats that are not consecutive words (one int in both lanes) |
| `mod.rs:1687` | `bool_colour` route, `reads_member_words_apart` | a mediump colour of booleans reading float struct-member words that are not consecutive |
| `mod.rs:1606` | `fp32_uniforms`, `1u64.checked_shl(k)` | an fp32 float uniform past the 64th row of the uniform list |

## 2. Measure

- **Board cases.** The text has **472 lines** in the console logs of the
  Verdin iMX95 (Mali-G310) dEQP-GLES2 tree, `tools/cts/log/2026-11-2{4,5}-imx95-gles2-*-e63ab28e/console/*.log`
  (the run the plan calls "2026-11-25, main a3d8d2de"; the slices' README
  and image build string say `e63ab28e`). The lines belong to **466 cases**
  (`state_query.shader.program_active_uniform_types` logs it 7 times), all
  in `tools/cts/gles2-mustpass.txt`. They are the 272 cases of the
  NanoPC-T6 slices `2026-11-24-t6-gles2-S{1..4}-8ff01055` and 194 more
  (mostly the `_both` variants of `uniform_api.value.*` and
  `shaders.invariance.*`).
- **Host measure at the tip:** `tools/mali/deqp-host-measure.sh --cts
  "$CONNECTED_DEPS_DIR/VK-GL-CTS-host" --jobs 4 --slice <the four T6
  8ff01055 slices> --cases <the 466 names>`. VK-GL-CTS `93bca018` with
  `cts-yakogl-target.patch`, a shallow blobless clone without the Vulkan
  and GL 4 data. 466 began. 456 refuse on the host:
  - **284** refuse with this text first, in the fragment shader;
  - **168** refuse first in the vertex shader with another text (96 "a
    uniform array", 42 arithmetic, 9 FMA, 9 constant product, 6 vertex
    loop, 3 rebalance, 3 varying lane from no attribute). Their fragment
    shader, compiled with a generated pass-through vertex shader that
    feeds its varyings, refuses with **this text** in all 168: 137 at
    `select.rs:628`, 31 at `nir_fs.rs:10805`;
  - 4 refuse with "a shape Mesa's algebraic rules rewrite"
    (`shaders.matrix.mul.uniform_{lowp,mediump}_mat{2,4}_float_fragment`):
    not this family any more.
  - The other 10 compile and link on the host: the highp matrix shapes
    (`fsunif`'s `mat32`) and `shaders.struct.uniform.{basic,array_member}_fragment`
    (lowered by `lower-fsunif` since `e63ab28e`), and
    `state_query.shader.program_active_uniform_types`. They should leave
    this text in the next board run.
- So **452 must-pass cases need this lowering**: 284 have it as their
  first refusal, 168 have it behind a vertex refusal that other families
  own.

### Cases per site (HOST-ONLY, tip `8133663c`)

| Site | fs first | behind a vs refusal | All | dEQP groups |
|---|--:|--:|--:|---|
| `select.rs:628` | 141 | 137 | **278** | `uniform_api.value` 224, `uniform_api.info_query` 32, `uniform_api.random` 22 |
| `nir_fs.rs:10805` | 72 | 31 | **103** | `shaders.random` 60, `shaders.invariance` 36 (`common_subexpression_*` fs, `loop_*`/`subexpression_precision_*` behind vs), `fragment_ops.interaction` 7 |
| `nir_fs.rs:10792` | 53 | 0 | **53** | `shaders.matrix` 28, `uniform_api.value` 16 (`basic_array.float`), `shaders.random` 7, `uniform_api.info_query` 2 |
| `nir_fs.rs:10318` | 8 | 0 | **8** | `shaders.matrix` 4 (highp `mat3`), `shaders.struct` 4 (`nested`, `struct_array`, `loop_struct_array`, `dynamic_loop_struct_array`) |
| `mod.rs:1687` | 6 | 0 | **6** | `shaders.linkage.uniform_struct_*` |
| `nir_fs.rs:10335` | 2 | 0 | **2** | `shaders.struct.uniform.sampler{,_array}_fragment` |
| `nir_fs.rs:10360` | 1 | 0 | **1** | `fragment_ops.interaction.basic_shader.88` |
| `mod.rs:1606` | 1 | 0 | **1** | `uniform_api.random.23` |
| **all** | **284** | **168** | **452** | |

## 3. The references

Each pair is a program of a case above, verbatim from the measure's `.qpa`
(the `.license` beside each shader names the case). Each was drawn with
`tools/mali/deqp_ref.sh <vert> <frag> tools/mali/testdata/deqp_unifmp_<shape>
--instrument shim` on the drm-shim that `tools/mali/drm_shim_ref.sh build`
made from the pinned `mesa-26.2.1.tar.xz` (`$CONNECTED_DEPS_DIR/mesa-shim`).
`--uniform-values` is on, except for `struct_sampler`, drawn with
`--zero-uniforms` because of its sampler member. Redrawing
`deqp_fsunif_mat4_add_float` on that shim gave a byte-equal expect.json.
All 12 refuse at the tip with the whole text above. The site of each is
the probe's.

The 12 cover the sites no earlier reference draws (`10318`, `10335`,
`10360`) and add matrix and random-expression forms of `10792` and `10805`.
`select.rs:628` already has 20 references refusing there at the tip
(`deqp_fsunif_{arr_bvec4,arr_ivec4,arr_mat4,struct_int_ivec4}`, the five
`deqp_fsunif2_*`, the eight `deqp_fsunif3_*`,
`deqp_unif2_fs_{bool,int,struct}_uniform`), so none was added for it.

| Reference | Case | Site | fs words | What Mesa emits |
|---|---|---|--:|---|
| `interaction_16` | `fragment_ops.interaction.basic_shader.16` | `10805` | 11 | `vec4(a, bool(d), float(d), float(a))`, `a` a mediump float uniform (FAU u2.w0 = `0x5000`): ATEST reads `u2.w0.h0` **in place**. The colour's words are `MKVEC.v2i16 r0, u2.w0.h0, r1.h0` (uniform half, then the compare's half) and `MKVEC.v2i16 r2, r0.h0, u2.w0.h0` (the varying narrowed by `FADD.f32 r0.h0, r0, -0`, then the uniform half). |
| `inv_mediump_cse2` | `shaders.invariance.mediump.common_subexpression_2` | `10805` | 15 | `vec4(u_color.r, u_color.g, blue, u_color.a)`: the first word is the uniform's aligned pair (`MOV.i32 r1, u2.w0`). The second is `MKVEC.v2i16 r2, r0.h0, u2.w1.h1` (computed lane, then the uniform's **high** half). ATEST reads `u2.w1.h1`. |
| `rsg_expr10` | `shaders.random.basic_expression.fragment.10` | `10805` | 18 | only the last colour store is live, and it reads one uniform, `a` (u2.w0), and a varying. `MKVEC.v2i16 r1, u2.w0.h0, u2.w1.h0` pairs `a`'s half with a half of a constant word (the shader's constants are FAU words u2.w1..u5). `MKVEC.v2i16 r1, u4.w0.h0, r1.h0` puts a constant's half beside the narrowed varying. `MOV.i32 r0, u2.w0`, then `FMA.v2f16 r0, u3.w0, r0.h00` replicates `a`'s half. |
| `mat4_sub_mat4` | `shaders.matrix.sub.uniform_mediump_mat4_mat4_fragment` | `10792` | 23 | both mat4s fp16 in the FAU, **interleaved by slot** (`bi_opt_reorder_push`): `u_in0` column words and `u_in1`'s share a 64-bit slot (u2 = `u_in0[0].xy`, `u_in1[0].xy`, ...). `res[k] = a - b` is `FADD.v2f16 r, uN.w0, uN.w1.neg` for a column's aligned pair, and `FADD.v2f16 r, uN.w0.h00, uN.w1.neg.h00` (or `.h11`) for a lane the swizzle reads alone. The sums recombine `.h00`/`.h11` of the results, then `MKVEC.v2i16`. |
| `vec4_mat4_mul` | `shaders.matrix.mul.uniform_mediump_vec4_mat4_fragment` | `10792` | 27 | `v_coords * u_in1`: the varying narrowed by `V2F32_TO_V2F16` per pair, then for each result lane a chain of four `FMA.v2f16 r, rV.hXX, uN.wM.hXX, acc.h00`: a column's halves read **one at a time** (`u2.w1.h11`, `u2.w1.h00`, ...). |
| `rsg_sconv12` | `shaders.random.scalar_conversion.fragment.12` | `10792` | 7 | `vec4(c, b, 0, a)`: `MKVEC.v2i16 r0, u2.w1.h0, u2.w0.h0` (two uniforms' halves in **reverse** word order), then `FMA.v2f16 r0, u3.w0, r0, -0` with the folded constant vector. The second colour word is the folded constant `MOV.i32 r1, u3.w1`. |
| `highp_mat3_mul` | `shaders.matrix.mul.uniform_highp_mat3_mat3_fragment` | `10318` | 48 | two highp `mat3`s as 18 fp32 FAU words (u2..u10; a column's three words not padded, the two matrices interleaved by slot). The product is 27 `FMA.f32`, a word moved to a register (`MOV.i32 r, uN.w1`) where an instruction would read two slots. Each colour lane is narrowed **alone** (`FADD.f32 r.h0, r, -0`), and `MKVEC.v2i16 r2, r0.h0, u11.w0.h0` pairs the last lane with the 1.0 constant. |
| `highp_vec3_mat3` | `shaders.matrix.mul.uniform_highp_vec3_mat3_fragment` | `10318` | 21 | a highp varying (`LD_VAR_BUF_IMM.f32 .v3`) times a highp `mat3` (u2..u6 fp32): 9 `FMA.f32`, three lanes narrowed alone, `MKVEC.v2i16` as above. |
| `struct_array` | `shaders.struct.uniform.struct_array_fragment` | `10318` | 14 | `uniform S s[3]` of `{mediump float a; mediump int b;}`: the struct members are fp32 (a struct member is not narrowed). Mesa pushes only the read words: `s[2].a` u4.w1, `s[1].a` u4.w0, `s[0].a` u3.w0, the ints u2.w0, u2.w1, u3.w1. `FADD.f32 r.h0, u4.w1, -0` and `FADD.f32 r.h0, u4.w0, -0` paired by `MKVEC.v2i16` (no `V2F32_TO_V2F16`: the lanes are in reverse word order), and `ISUB.s32`/`IADD.s32` then `S32_TO_F32` for the int lane. |
| `struct_nested` | `shaders.struct.uniform.nested_fragment` | `10318` | 13 | `vec4(s.a, s.b.b, s.b.a + s.c)`: `V2F32_TO_V2F16 r2, u3.w0, u3.w1` for the nested `vec2` (a consecutive pair), `FADD.f32 r1.h0, u4.w0, -0` for `s.a`, `IADD.s32` + `S32_TO_F32` + `FADD.f32 .h0` for the int sum, then `MKVEC.v2i16 r1, r1.h0, r2.h0` and `MKVEC.v2i16 r2, r2.h1, r0.h0`: the vec2's narrowed pair split across the colour's two words. |
| `struct_sampler` | `shaders.struct.uniform.sampler_fragment` | `10335` | 18 | `texture2D(s.c, v_coords.xy * s.b.xy + s.b.z)`: the fp32 members narrowed as pairs, **swapped** where the operand wants it (`V2F32_TO_V2F16 r1, u2.w1, u2.w0`), `MKVEC.v2i16 r3, r1.h0, r2.h0`, then one `FMA.v2f16 r0, r0, r3, r2.h11` and `F16_TO_F32` of the coordinate for `TEX_SINGLE`. Uniform words 0 (`--zero-uniforms`). |
| `interaction_88` | `fragment_ops.interaction.basic_shader.88` | `10360` | 8 | the colour's live part is `(a, a)` of a `uniform mediump int a` (u2.w0): `S32_TO_F32 r0, u2.w0`, `FADD.f32 r0.h0, r0, -0`, and `IADD.v2u16 r1, r0.h00, 0`, one half in both lanes. The first word is the constant `0x3C000000`. |

The earlier references that refuse at these sites at the tip, for each
step (sites by the same probe):

- `10805`: `deqp_intval3_rsg_swizzle12_fs`, `deqp_misc2_invariant_cse`;
- `10792`: `deqp_fsunif_mat4_add_float`, `deqp_fsunif_mat2_mul_mat2`,
  `deqp_uniarray_api_float`, `deqp_unif2_fs_other`;
- `mod.rs:1687`: `deqp_boolpix_lk_uniform_struct_vec4`,
  `deqp_boolpix_lk_uniform_struct_fragment_only_vec4`,
  `deqp_audit_o_linkage_uniform_struct_vec4`;
- `select.rs:628`: the 20 listed above.

## 4. What Mesa does, in general

- **Storage.** `lower_precision` (`compiler/glsl/lower_precision.cpp`)
  narrows a mediump or lowp float uniform that is not a struct member.
  The driver uploads it as fp16 halves, two components a 32-bit word,
  a column of a matrix starting a word. A highp uniform and a struct's
  float member stay fp32, one word a component. An integer or boolean
  is one 32-bit word a component (`caps->int16` is false in
  `gallium/drivers/panfrost/pan_screen.c`).
- **Push.** Mesa pushes only the words the program reads, in storage
  order. `bi_opt_reorder_push`
  (`panfrost/compiler/bifrost/bi_opt_push_ubo.c`) then moves two words
  one instruction reads into one 64-bit slot, because an instruction
  reads one FAU slot. That is the interleaving of `mat4_sub_mat4` and
  `highp_mat3_mul`. Where an instruction would still need two slots, the
  scheduler moves a word to a register first (`MOV.i32 r, uN.wM`).
- **fp16 lanes.** A Valhall instruction reads a FAU word with a half
  swizzle (`.h00`, `.h11`, `.h01`, `.h10`), so a single uniform half is
  read **in place** as an operand (`u2.w0.h0` in ATEST and MKVEC,
  `uN.wM.h11` in `FMA.v2f16`). Mesa never moves it to a register first.
  Two lanes that are not one word's aligned pair are combined with
  `MKVEC.v2i16 r, x.h0, y.h0`, uniform or not, in either order.
- **fp32 to fp16.** An fp32 value (a highp uniform word, a struct
  member, an int's `S32_TO_F32`) is narrowed by
  `V2F32_TO_V2F16 r, a, b` when two consecutive lanes of the result come
  from two values the instruction can read together (a register pair or
  one slot, in either order: `struct_sampler`'s `u2.w1, u2.w0`).
  Otherwise each lane is narrowed alone with `FADD.f32 r.h0, x, -0`
  (`F32_TO_F16`) and the halves are paired by `MKVEC.v2i16`.

## 5. Lowering steps, in order

The order is by the cases a step makes reachable. Each step is a change of
`lane_tree` (`nir_fs.rs`) or `UboLayout::fs16` (`select.rs`). The steps
are independent of each other, except that U1's programs reach the
`lane_tree` sites (U2-U5) once their layout exists
(`mali-deqp-fsunif2.md` section 5).

| Step | Site | Change | References that must become byte-equal |
|---|---|---|---|
| **U1** | `select.rs:628` | `fs16` rows for integer and boolean arrays (one 32-bit word a lane, `stride` the front end's), `mat2`/`mat3`/`mat4` arrays (the matrix row per element at its fp16 column layout), and the same as struct members and nested members (the flattened leaf rows `deqp_unif2_fs_struct_uniform` uses). The reads are integer compares that `intval`/`fsint` already lower, plus the fp16 `FMAX`/`FCMP` trees of `deqp_fsunif_arr_mat4`. | `deqp_fsunif_{arr_bvec4,arr_ivec4,arr_mat4,struct_int_ivec4}`, `deqp_fsunif2_*`, `deqp_fsunif3_*`, `deqp_unif2_fs_{bool,int,struct}_uniform` |
| **U2** | `nir_fs.rs:10805` | one fp16 uniform half beside any other lane: a new leaf that reads `uN.wM.h0`/`.h1` **as the operand** (no register move) and builds the word with `MKVEC.v2i16 <lane>.h0, uN.wM.hX` in Mesa's order (uniform first or second). ATEST reading a uniform half in place (`interaction_16`, `inv_mediump_cse2`). | `deqp_unifmp_{interaction_16,inv_mediump_cse2,rsg_expr10}`, `deqp_intval3_rsg_swizzle12_fs`, `deqp_misc2_invariant_cse` |
| **U3** | `nir_fs.rs:10792` | two fp16 uniform halves that are not an aligned pair: (a) the same half twice is `Swz::H00`/`H11` already; add (b) a half of one word and a half of another, either order, as `MKVEC.v2i16 uA.h0, uB.h0` (`rsg_sconv12`); (c) single halves as operands of `FADD/FMA.v2f16` with `.h00`/`.h11` (`vec4_mat4_mul`, `mat4_sub_mat4`, `mat4_add_float`), which needs the result lanes kept apart and recombined as Mesa does; (d) the slot reorder of `bi_opt_reorder_push` for two matrices read by one instruction (`mat4_sub_mat4`, `mat2_mul_mat2`). | `deqp_unifmp_{mat4_sub_mat4,vec4_mat4_mul,rsg_sconv12}`, `deqp_fsunif_{mat4_add_float,mat2_mul_mat2}`, `deqp_uniarray_api_float`, `deqp_unif2_fs_other` |
| **U4** | `nir_fs.rs:10318` | two fp32 uniform words that are not consecutive: each lane `FADD.f32 r.h0, uN.wM, -0` and `MKVEC.v2i16`; a consecutive pair in reverse order as `V2F32_TO_V2F16 r, u.w1, u.w0`. For the highp `mat3` shapes, also the fp32 FMA chain with its slot interleave and `MOV.i32` of the second slot's word. | `deqp_unifmp_{highp_mat3_mul,highp_vec3_mat3,struct_array,struct_nested}` |
| **U5** | `mod.rs:1687` | the `bool_colour` route with struct-member words read apart: lay the members out as the walk reads them (U4's narrowing) instead of refusing | `deqp_boolpix_lk_uniform_struct_{vec4,fragment_only_vec4}`, `deqp_audit_o_linkage_uniform_struct_vec4` |
| **U6** | `nir_fs.rs:10335` | an fp32 word beside a non-fp32 lane inside an fp16 operand: narrow it (pair or alone, U4) and MKVEC it with the other lane | `deqp_unifmp_struct_sampler` |
| **U7** | `nir_fs.rs:10360` | one int uniform's float in both lanes: `S32_TO_F32`, `FADD.f32 .h0`, `IADD.v2u16 r, r.h00, 0` | `deqp_unifmp_interaction_88` |
| (U8) | `mod.rs:1606` | more than 64 uniforms: a wider mask. Mesa 26.2.1 on the shim cannot draw `uniform_api.random.23` ("export failed", `mali-deqp-fsunif2.md` section 2), so there is no reference. Leave it refused. | none |

## 6. Forecast (HOST-ONLY)

"Unblocks" means that this text is no longer the case's first refusal in
the fragment shader. A case passes only if nothing else refuses after the
step and the board draws the right pixels. U1's programs can refuse at
U2-U4 sites once laid out. The numbers are upper bounds from the tip's
host measure.

| Step | fs first (this text is the case's first refusal) | Behind a vertex refusal (also needs that family) | dEQP groups |
|---|--:|--:|---|
| U1 | 141 | 137 (96 vertex "uniform array": `vsunifarr-design`; 41 vertex arithmetic: `arith-lower-2`) | `uniform_api.value` 224, `uniform_api.info_query` 32, `uniform_api.random` 22 |
| U2 | 72 | 31 (`shaders.invariance.*.loop_*`, `subexpression_precision_*`: vertex FMA 9, constant product 9, loop 6, rebalance 3, varying lane 3, arithmetic 1) | `shaders.random` 60, `shaders.invariance` 36, `fragment_ops.interaction` 7 |
| U3 | 53 | 0 | `shaders.matrix` 28, `uniform_api.value` 16, `shaders.random` 7, `uniform_api.info_query` 2 |
| U4 | 8 | 0 | `shaders.matrix` 4, `shaders.struct` 4 |
| U5 | 6 | 0 | `shaders.linkage` 6 |
| U6 | 2 | 0 | `shaders.struct` 2 |
| U7 | 1 | 0 | `fragment_ops.interaction` 1 |
| U8 | (1, not lowered) | 0 | `uniform_api.random` 1 |
| **all** | **283** (+1 U8) | **168** | |

U1, U2 and U3 together take 266 of the 284 first-refusal cases. U1 alone
is 141 first-refusal cases and 278 in all. It comes first, and it is the
step that also needs the vertex families to reach its other 137.

## 7. Caveats

- HOST-ONLY. The host library reads black pixels. Only compile and link
  outcomes are real.
- The 168 vertex-first cases' fragment sites come from pairing each
  fragment shader with a generated pass-through vertex shader (one
  highp attribute feeding each varying). A real vertex shader could
  change what the link propagates (a constant varying), but not a
  uniform's layout.
- The plan names the run "main a3d8d2de". Its records say `e63ab28e`, the
  commit this note counts from.
- The train `intval-rest` (commit `224cc3f7`, not reachable from this
  clone) changes `src/shaderc/online/mali/**`. The line numbers above are
  `8133663c`'s. Re-probe the sites after it lands. Its intval steps may
  move some `select.rs:628` integer reads to other texts.

## 8. Landed (plan #387, item `mali-train-3a`)

Re-probed at `5fd5bf43` with the recipe of section 1 (line numbers of that
commit): `select.rs:628` stops 8 references (every one a matrix array;
the int and bool arrays have had rows since plan #377), `nir_fs.rs:11122`
(the `10805` site) 6, `nir_fs.rs:11100` (the `10792` site) 4,
`nir_fs.rs:10616` 4, `mod.rs:1689` 3, `nir_fs.rs:10633` and `:10658` one each.

- **U1** (`select.rs`, `UboLayout::fs16`): a `mat2`, `mat3` or `mat4`
  array has rows, one a column, element after element (fp16: a `mat4`
  element 8 words, a `mat3`'s 6, a `mat2`'s 2; a struct member's fp32, one
  word a component). No reference becomes byte-equal by it: the 8 leave
  this text and stop at the next one. `deqp_fsunif_arr_mat4`,
  `deqp_fsunif3_random_{4,9,80}` stop at "a gl_FragColor lane other than
  the varying's own lane or a constant" (the compare chain of
  `docs/notes/mali-fclane-lowering.md`), `deqp_fsunif2_{struct_mat_array,
  struct_mat_nested,random_13}` and `deqp_fsunif3_random_81` at "a fragment
  colour of more than 255 vectorised operations and operands". The rows
  are checked against the FAU word indices of `deqp_fsunif_arr_mat4`
  (`u2.w0` to `u13.w1`) only: no reference of a matrix array compiles to
  the end yet.
