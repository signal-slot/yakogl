<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali dEQP-GLES2 family `mixprec`: "mixes highp and mediump" and "varying load or conversion of a form no reference draws"

Date 2026-11-16. Plan #377 item `study-mixprec` (stage 5, issue #20, NanoPC-T6
Mali-G610). This is a study only: it changes no code and makes no references.
Read `docs/notes/mali-precision.md` (Mesa's width rule, part A),
`docs/notes/mali-mixed-precision.md` (the fp32/fp16 meeting points already
lowered) and `docs/notes/mali-fragcoord-mediump.md` first. Line numbers in our
tree are at commit 38ad30eb. Mesa citations are 26.2.1 (`G/` is
`src/compiler/glsl/`, `N/` is `src/compiler/nir/`, `B/` is
`src/panfrost/compiler/bifrost/`). dEQP shader text is from VK-GL-CTS main,
`modules/gles2/functional/es2fUniformApiTests.cpp` and
`es2fShaderLoopTests.cpp`, rebuilt from the generators (no host dEQP run; the
`measure` item gives the generated pairs).

The case counts are from `refusals.tsv` of
`tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/` (the tree in section M
of `docs/es2-cts-failures.md`). The site of each shape was found by compiling a
rebuilt shader through the Mali online compiler at 38ad30eb (feature set
`host-test,gpu-mali,shaderc-online,no-shader-table`). The probe was a
temporary unit test, and each refusal site briefly logged its own line.
Both were removed afterwards.

## 1. The two refusals and their sites

| Refusal (`MaliError`, `src/shaderc/online/mali/mod.rs`) | Cases | Message |
|---|---|---|
| `MixedPrecision` (mod.rs:146, text :427-429) | 192 | "a fragment shader that mixes highp and mediump or lowp float values (Mesa's fp32 and fp16)" |
| `FsVaryingLoad` (mod.rs:272, text :524-526) | 59 (41 NotSupported, 16 + 2 Fail) | "a fragment varying load or conversion of a form no reference draws (...)" |

Where `MixedPrecision` is raised (every site, so a later lowering knows which
one it moves):

- `mod.rs:1391`: a highp ES 3.00 colour output of a mediump shader.
- `mod.rs:1484`: `f.mixed` set by the walk, reported after selection. It is
  set in `FsPrecision::of` (mod.rs:1075-1126: a local, parameter, function
  result or block precision statement of the other width), at mod.rs:1448
  (a non-highp float uniform of a highp shader), and in `nir_fs.rs` at :1441
  and :2060-2067 (`mark_mixed`: gl_PointCoord, a lowp texel in a highp
  operation), :1832 (`widen` of an fp16 texel product), :2797-2803
  (`fp16_operand`: a highp varying lane in a mediump operation), :3125-3126,
  :4237 (a mediump varying of a highp shader) and :4363.
- `nir_fs.rs:2842` (`tex32`): a look-up read at both widths.
- `nir_fs.rs:4413-4415`: `alu16` refuses a form (its `FragCoordMediump`) on no
  gl_FragCoord read, reported as mixed.
- **`nir_fs.rs:4501-4503`**: the walk's "product or sum on an fp32 value of a
  mediump shader" arm (:4487-4506). The other operand must be fp32
  (`fp32_lane`, :1802), an fp32 select of constants (`select32`, :1499), an
  fp16 node or a constant. **Every probed dEQP shape of this family stops
  here.**
- `nir_fs.rs:8480` (`src32`): a node kind the fp32 selection has no source
  for.

Where `FsVaryingLoad` is raised:

- `nir_fs.rs:4548-4551`: `+ vec2(0.0, -0.0)`, zeros of both signs.
- `nir_fs.rs:6046` and `:6089` (`kept_whole`, :5982): a varying lane read
  raw by a node other than a cube look-up, or a colour lane of a kind not
  classified.
- `nir_fs.rs:6161` (`colour_keeps_whole`, :6123): a colour store beside a
  value of no measured kind.
- `nir_fs.rs:6987-6988` (`conversion_order`, :6946): one load's fp16 trees
  read lanes of two varyings.
- **`nir_fs.rs:7043-7044`** (`conversion_order`): Mesa's conversions are not
  the ones `convert` makes. The doc comment at :6926-6945 names the case: a
  pair Mesa makes with its odd lane low has no reference. **Every probed
  dEQP shape of this family stops here.**
- `nir_fs.rs:7842-7843`: a load that holds lanes it does not read, of a
  varying that is not kept whole.

## 2. Shapes and counts

### 2.1 `MixedPrecision` (192)

| Shape | Cases | Site |
|---|---|---|
| M1 `uniform_api`: products of compare-function results, all mediump | 186 | nir_fs.rs:4502 (probed: vec4, sampler2D) |
| M2 `shaders.conversions.vector_combine` (`float_int_to_vec2_fragment`, `vec2_ivec2_to_vec4_fragment`) | 2 | not probed |
| M3 `shaders.random.*.fragment` (`conditionals.27`, `exponential.91`, `scalar_conversion.27`, `trigonometric.91`) | 4 | not probed |

M1 by uniform type: `sampler2D_samplerCube` 56, `sampler2D` 36, `vec4` 25,
`vec3` 14, `vec2` 14, `mat4` 10, `mat4_mat2` 7, `mat3` 5, `mat2` 5, and
`uniform_api.random.N` 14. By stage: 100 `_fragment`, 72 `_both`, 14 random.
The groups are `info_query`, `value.initial.{get_uniform,render}`,
`value.assigned.{by_value,by_pointer,basic_array_assign_*,unused_uniforms}`,
over `basic`, `basic_array`, `basic_struct`, `struct_in_array`,
`array_in_struct` and `nested_structs_arrays`. **The M1 shaders mix no
precisions.** No uniform, local or parameter is highp, and there is no
`precision` statement. The refusal's text is wrong for them. The fragment
shader (es2fUniformApiTests.cpp, `generateFragmentSource` and
`writeUniformDefinitions`) is:

```glsl
varying mediump float v_vtxOut;
uniform mediump vec4 u_var;            /* or sampler2D, samplerCube, a struct, an array */
mediump float compare_float(mediump float a, mediump float b) { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec4(mediump vec4 a, mediump vec4 b)
    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
void main()
{
    mediump float result = v_vtxOut;
    result *= compare_vec4(u_var, vec4(...));   /* a sampler: compare_vec4(texture2D(u_var, vec2(0.0)), vec4(...)) */
    gl_FragColor = vec4(result, result, result, 1.0);
}
```

Our front end inlines the calls, so each `?:` reaches the walk as a 32-bit
`bcsel(flt16(..), 1.0, 0.0)` (`select32`, nir_fs.rs:1493-1503, the shape of
intbool_76_boolany, where the condition is a bool uniform of no precision).
The product of two such selects takes the fp32 arm. Then the third factor
(an fp32 product times a select) or `v_vtxOut * product` (a raw mediump
varying lane) is neither fp32, nor a select, nor fp16, so :4502 refuses it.

What Mesa does with the same source:

- lower_precision runs at compile, before inlining (G/glsl_parser_extras.cpp:
  2487-2491, then `opt_shader`). The `?:` is a `conditional_tmp` temporary of
  no precision, assigned constants in an `if` (G/ast_to_hir.cpp:1986-2006).
  Its assignments are UNKNOWN (G/lower_precision.cpp:295-311), so they stay
  fp32. That matches our `select32`.
- The caller's return temporary of a mediump function is made mediump
  (G/lower_precision.cpp:545-579, `visit_leave(ir_call)`). As a mediump
  temporary it is retyped to float16 (`lower_variables_visitor`,
  :1023-1060), and the assignment from the callee's fp32 value gets an
  `f2fmp` (:695-712). So each `compare_float(...)` result is fp16, and the
  products in `compare_vec4` and `result *= ...` are fp16 operations
  (SHOULD_LOWER on mediump operands, :232-245).
- In NIR, `f2f16(bcsel(a, #b, #c))` becomes `bcsel(a, f2f16(b), f2f16(c))`
  (N/nir_opt_algebraic.py:3149-3150, under `support_16bit_alu`), and
  `f2fmp(b2f32(a))` becomes `b2f16(a)` (:1980). So the selects should end up
  16-bit, on the 16-bit compare of `abs(a - b)` with 0.05 (an fp16 constant,
  `_mesa_float_to_half`, :739-746).

So the expected program is fp16 all through: `FADD.v2f16`/`FCMP.v2f16` per
lane, 16-bit selects of 1.0h/0.0h, an fp16 product chain with `v_vtxOut`,
and the fp16 colour. Expected, not read: none of our references draws a
compare result multiplied inside a mediump function's return. What
`nir_opt_vectorize` pairs and how the products associate (the factors'
order through GLSL `do_rebalance_tree`) must come from the reference.

Once :4502 accepts M1, the same programs may stop at a later gap. Two shapes
probed beside M1 show what follows it, and neither is in this family:
`result *= abs(u_var - 0.5) < 0.05 ? 1.0 : 0.0` written inline gives
"a gl_FragColor lane other than the varying's own lane or a constant"
(`ColourForm`), and a bare `vec4(result, result, result, 1.0)` of
`v_vtxOut` gives "a fragment shader operand of a form or in a place no
reference draws" (`FsOperandForm`). Both are the colourpair and misc
families' question.
The `_both` cases also need their vertex shader (the same compare
functions at highp-by-default; the unitype, uniarray and intval families
hold the vertex refusals). The struct and array cases also need the uniarray
family's layouts, and the samplerCube ones a cube look-up at (0,0,0) in a
mediump shader.

### 2.2 `FsVaryingLoad` (59)

| Shape | Cases | Site |
|---|---|---|
| V1 fragment loops whose unrolled `res = res.yzwx` leaves the colour a rotation of one mediump varying (`vec4(res.rgb, 1.0)`) | 54 | nir_fs.rs:7044 (probed: 3 iterations, break after 1) |
| V2 `shaders.return.conditional_return_{always,never}_fragment` | 2 | not probed |
| V3 `shaders.random`: `basic_expression.vertex.8`, `swizzle.fragment.41`, `trigonometric.vertex.60` | 3 | not probed |

V1 by group: `for_constant_iterations` 16, `while_constant_iterations` 15,
`do_while_constant_iterations` 15, and `{for,while,do_while}_{dynamic,uniform}_iterations` 8
(`infinite_with_unconditional_break_last`, `unconditional_break`). The
fragment shader (es2fShaderLoopTests.cpp, the generic and special cases) is:

```glsl
varying mediump vec4 v_coords;
void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    for (mediump int ndx = 0; ndx < 3; ndx++) { res = res.yzwx; }   /* or one of the special bodies */
    gl_FragColor = vec4(res.rgb, 1.0);
}
```

After unrolling, the colour is `vec4(c.w, c.x, c.y, 1.0)` for 3 iterations
and `vec4(c.y, c.z, c.w, 1.0)` for one (a break). The same colour written
without a loop is refused at the same line (probe), and 4 iterations (the
identity) compiles. So V1 is a varying-lane order problem, not a loop one.
The loops family's unrolling already gets the shader here. Mesa's order
(nir_fs.rs:6926-6945): the colour's own lanes lose their `f2fmp` at the link
(`f2f32(f2fmp(a))` is `a`, N/nir_opt_algebraic.py:1991).
`nir_lower_mediump_io` converts the stored colour just before the store
(B/bifrost_nir.c:913-915), in the colour's lane order, and
`nir_opt_vectorize` makes a pair at the first lane's place. For `(w, x, y,
1.0)` that is a conversion of lanes 3 and 0, then 1 with the constant. For
`(y, z, w, 1.0)` it is 1 and 2, then 3 with the constant. Our `convert`
pairs only aligned lanes with the even lane low, so :7043 refuses. Whether
Mesa makes `V2F32_TO_V2F16 r, w, x` (an unaligned pair, the "odd lane low"
form the comment names after swap_yx) or a `MKVEC` of two `FADD.f32 .h0`
conversions is not in any committed reference.
`tools/mali/testdata` has no swap_yx file. The reference decides.

The counts of the two V1 rotations per case are the `measure` item's: the
special cases (`sequence`, `nested_tricky_dataflow_2`, `vector_counter`,
`function_call_*`, ...) apply the swizzle a body-dependent number of times.
A case whose count is a multiple of 4 would already compile, so every refused
one is a non-identity rotation, (w,x,y), (z,w,x) or (y,z,w).

## 3. The pan_shc question

Every reference of this family must come from **the drm-shim driver**
(`tools/mali/drm_shim_ref.sh run`, which draws through `tools/mali/gl_ref.py`
on Mesa 26.2.1's panfrost GL driver). It cannot come from `pan_shc`. All
the shapes are mediump fragment shaders, and both refusals are about where
Mesa's GLSL path puts fp16 and its conversions. pan_shc's SPIR-V front end
has no precision lowering: glslang's RelaxedPrecision reaches only the IO
variables, and spirv_to_nir lowers no ALU
(`tools/mali/README-pan_shc.md`, "Known gaps of pan_shc", `mediump`). So its
bytes would be the fp32 program. Use pan_shc for nothing here except, if
wanted, a cross-check of a highp variant.

## 4. Lowering plan (one session, item `lower-mixprec`)

References first (item `ref-mixprec`, all drm-shim, each `.vert`/`.frag`
with `.trace`, `.stdout`, `.stderr` and `.expect.json` as the other
drm-shim cases are kept, listed in `tools/mali/testdata/cases.txt`). Use
`--uniform-values` so that the fp16 uniform block is in the trace:

| Reference | Source | Pins |
|---|---|---|
| `deqp_mixprec_cmpf` | `uniform_api` vec4 `_fragment` (the shader above, one `compare_float`) | the 16-bit select of a compare, `b2f16` vs `CSEL`, the fp16 constant 0.05 |
| `deqp_mixprec_cmpv4` | `uniform_api` vec4 `_fragment` (`compare_vec4`) | the product chain's order and pairing, the multiply with `v_vtxOut` |
| `deqp_mixprec_cmpv2`, `deqp_mixprec_cmpv3` | vec2 and vec3 | the odd lane counts of the chain |
| `deqp_mixprec_cmpmat2` | mat2 (`compare_mat2` over `compare_vec2`) | the matrix columns' uniform words (fp16 block) |
| `deqp_mixprec_cmptex` | sampler2D `_fragment` | the fp16 texel's lanes into the compares |
| `deqp_mixprec_rot3` | V1, `for_constant_iterations.basic_mediump_int_fragment` | the `(w, x, y, 1.0)` conversions |
| `deqp_mixprec_rot1` | V1, `for_constant_iterations.unconditional_break_fragment` | the `(y, z, w, 1.0)` conversions |
| `deqp_mixprec_rot2` | a 2-iteration rotation (hand-made from the same template) | the `(z, w, x, 1.0)` conversions, if the measure finds such a case |

The vertex shader for every fragment reference is the dEQP one
(`attribute highp vec4 a_position; varying mediump float v_vtxOut;
gl_Position = a_position; v_vtxOut = 1.0;` for `uniform_api`, and
`a_coords` to `v_coords` for the loops). The link's constant propagation
must not fold `v_vtxOut = 1.0` away unseen: that fold is the linked program's
real behaviour (`link::VaryingConsts`), so keep the dEQP VS as it is and
record the folded form.

Lowering, in this order, each byte-compared in a new
`tests/mali_deqp_mixprec.rs` (header feature set
`host-test,gpu-mali,shaderc-online,no-shader-table`):

1. **V1, the rotated colour** (54 cases, smallest change). In
   `conversion_order` (nir_fs.rs:6946-7050) and `convert`, add the
   conversion forms the rot references show: an unaligned pair (or two
   singles plus a `MKVEC`) at the first lane's place, in colour lane order.
   Keep :7043 for any other mismatch.
2. **M1, the compare-function chain** (186 cases). Lift the selects to fp16
   when they reach a mediump operation, which is what Mesa's return-temporary
   `f2fmp` plus N/nir_opt_algebraic.py:3149 do: in the :4487 arm (or before
   it), a `select32` whose consumer is a product with a mediump value (an
   fp16 node or a mediump varying lane) becomes a 16-bit select, and the
   product goes to `alu16`'s fp16 path, not the fp32 arm. The intbool_76
   rule (a sum of selects alone stays fp32, narrowed at the store) must
   still hold: its operands meet no mediump value. Fix the misleading
   message too. A remaining fp32/mediump meeting in a mediump shader with no
   highp declaration is not "mixes highp and mediump". Give it its own
   `MaliError` (with a new `GAPS` line) so the tally tells the two apart.
3. Re-run the probes of section 2 and record the next gap of M1's
   `_fragment` cases (expected: `ColourForm` or `FsOperandForm` for the
   `vec4(result, result, result, 1.0)` colour, the colourpair family).

M2, M3, V2 and V3 (11 cases) are left until the `measure` item supplies
their generated pairs. dEQP's random shaders cannot be rebuilt by hand. A
case among them that meets the same two sites joins steps 1 and 2.

If step 2 changes a glmark2 verdict (a mediump select of constants on a
compare in a glmark2 fragment shader), move
`tests/fixtures/mali_glmark2_verdicts.txt` forward in the same commit (plan
rule). Plan #367 holds the glmark2 control-flow and ARITH families, so check
its merged notes for a select rule of its own first.

## 5. What this does not settle

- Mesa's exact M1 and V1 programs: the references decide them, and the
  plan above names the alternatives.
- What M1 unblocks: 186 cases stop at :4502 first. How many then pass
  depends on the colour gap after it, and for `_both` on the vertex
  refusals.
- M2, M3, V2, V3: not probed (no generated source here).

## 6. References

Item `ref-mixprec`, 2026-11-16. Eight Mesa 26.2.1 drm-shim references,
`tools/mali/testdata/deqp_mixprec_<shape>.{vert,frag,expect.json}`, one per
shape of section 4's table except `rot2` (below). The tree had no
`tools/mali/testdata/deqp_shapes/` pair and no `tools/mali/deqp_ref.sh`, so
each pair is the dEQP-GLES2 text rebuilt from the generator at VK-GL-CTS
`93bca01861b0` (the revision of section M), one real case each. Neither stage
was changed: the vertex shader is the dEQP one (section 4).

The literals in the `uniform_api` shaders are the case's own values. The case
seeds `de::Random` with `deStringHash(getName()) ^ base seed`
(es2fUniformApiTests.cpp:1899; `getName()` is the leaf name, for example
`vec4_fragment`; the base seed is dEQP's default 0). It then draws
`getFloat(-10, 10)` per component, or `getFloat(0, 1)` four times for a
sampler's fill colour (:600-636). The literal is that value with two
decimals (:538-567, `de::floatToString`). The values were replayed off the
board in float32 (deRandom.c and deRandom.hpp transcribed). The same replay
gives random.3's first literals in `tests/gc_es2ctx_random3.rs` (seed
`deStringHash("3")`: `bvec2(true, true)`, `bvec3(true, true, false)`,
`vec4(0.97, 0.27, 0.21, 0.27)`). A leaf name gives the same values in every
group, so `info_query`, `by_value`, `by_pointer` and `basic_array_assign_*`
of one type and stage share this text. `value.initial.*` uses zeros
(`FEATURE_UNIFORMVALUE_ZERO`, :2103), so its literals are `0.00` and are not
covered here.

Each reference was made with the same command (work dir: Mesa 26.2.1 built
by `tools/mali/drm_shim_ref.sh build`, tarball sha256 as
`third_party/mesa-26.2.1/UPSTREAM`):

```sh
T=tools/mali/testdata/deqp_mixprec_<shape>
tools/mali/drm_shim_ref.sh run <work dir> $T.vert $T.frag <out>/deqp_mixprec_<shape> --uniform-values
tools/mali/pan_ref_diff.py extract <out>/deqp_mixprec_<shape>.trace --draw > $T.expect.json
```

Two runs gave the same `expect.json`. The `.trace`, `.stdout` and `.stderr`
files and the `cases.txt` lines are not committed. `cases.txt` is a phase 2
file. Section 4 asks for them, but the item delivers only the triples.

| Reference | Case (dEQP-GLES2.functional.) | Source (rebuilt) | Pins for `lower-mixprec` |
|---|---|---|---|
| `deqp_mixprec_cmpf` | `uniform_api.value.assigned.by_pointer.render.basic.float_fragment` | es2fUniformApiTests.cpp:1237-1261 (VS), :1263-1286 (FS), :1135-1204 (`compare_float`, :1154), :1206-1217; value -2.16 | one compare: `FADD.v2f16` of the fp16 uniform and the negated literal (FAU 0x4052 = 2.16h), `FCMP_OR.v2f16.lt.f1` against 0.05h (FAU 0x2a66). The compare gives 1.0h/0.0h itself. Colour `(r, r, r, 1.0)` is `IADD.v2u16 r.h00` plus `MKVEC (r.h0, 1.0h)`. 10 words. Today it stops at `ColourForm`, not `MixedPrecision` (as the dEQP row does) |
| `deqp_mixprec_cmpv2` | `...by_pointer.render.basic.vec2_fragment` | as above, `compare_vec2` :1157; `vec2(-0.35, 7.81)` | `FADD_IMM.v2f16` of both lanes with packed fp16 immediates, `FMAX.v2f16 \|x\|, \|y\|`, then one `FCMP_OR`. 11 words |
| `deqp_mixprec_cmpv3` | `...by_pointer.render.basic.vec3_fragment` | `compare_vec3` :1160; `vec3(-0.56, -6.37, 3.12)` | the odd lane: (x, y) by `FADD_IMM.v2f16`, z by `FADD.v2f16 u.h00` with a pushed -3.12h pair; `FMAX(\|x\|,\|y\|)`, then `FMAX(\|z\|, xy)`. 14 words |
| `deqp_mixprec_cmpv4` | `...by_pointer.render.basic.vec4_fragment` | `compare_vec4` :1163; `vec4(-1.12, -8.66, 1.74, -5.62)` | the chain's order: `FMAX(\|x\|,\|y\|)`, `FMAX(\|w\|,\|z\|)` (h11 first), `FMAX(zw, xy)`, one `FCMP_OR`. The fp16 uniform block: u2 holds (16, 17), (18, 19) as half pairs. No varying program (`secondary_offset` none). 14 words |
| `deqp_mixprec_cmpmat2` | `...by_pointer.render.basic.mat2_fragment` | `compare_mat2` :1166 over `compare_vec2`; `mat2(-7.48, 6.25, -0.95, 4.59)` | a matrix column per uniform word (column-major, fp16): `FMAX` per column, then across the columns. 14 words |
| `deqp_mixprec_cmptex` | `...by_pointer.render.basic.sampler2D_fragment` | `compare_vec4` over `texture2D(u_var, vec2(0.0))` (:1208-1212); fill colour `vec4(0.35, 0.03, 0.40, 0.10)` | `TEX_SINGLE....16.2d.computed` at (0, 0), an fp16 texel, then cmpv4's chain on its lanes. 19 words |
| `deqp_mixprec_rot3` | `shaders.loops.for_constant_iterations.basic_mediump_int_fragment` | es2fShaderLoopTests.cpp:284-492 (`numLoopIters` 3 :316, body :416, `for` :418-424, footer :452-454) | colour `(w, x, y, 1.0)`: `LD_VAR_BUF_IMM.f32 v4`, `FADD.f32 .h0` of w alone, `V2F32_TO_V2F16` of the aligned pair (x, y), then `MKVEC (w.h0, x.h0)`, `MKVEC (y.h1, 1.0h)`. 11 words |
| `deqp_mixprec_rot1` | `shaders.loops.for_constant_iterations.unconditional_break_fragment` | es2fShaderLoopTests.cpp:495-566 (`ui_*` :530), :656-659, `forLoopStr` :759-761, footer :1194-1196 | colour `(y, z, w, 1.0)`: `FADD.f32 .h0` of y alone, `V2F32_TO_V2F16` of (z, w), `MKVEC (y.h0, z.h0)`, `MKVEC (w.h1, 1.0h)`. 9 words |

What the references settle:

- **V1** (section 2.2): neither alternative. Mesa makes no unaligned
  `V2F32_TO_V2F16 r, w, x` and no pair at the colour's first lane. It
  converts the load's lanes the colour reads, in aligned pairs where both
  lanes are read ((x, y) for rot3, (z, w) for rot1) and the odd one alone
  (`FADD.f32 .h0`). Then it builds the colour's halves with `MKVEC.v2i16`,
  the constant alpha from a pushed 1.0h. So in `conversion_order`/`convert`
  the step is "convert per aligned lane pair of the load, then repack". It
  is not "pair at the first colour lane".
- **M1** (section 2.1): neither `b2f16` nor `CSEL`, and no product chain.
  `fmul(b2f(a), b2f(b))` becomes `b2f(iand(a, b))`
  (N/nir_opt_algebraic.py:1679), and `iand(flt(a, c), flt(b, c))` becomes
  `flt(fmax(a, b), c)` (:1159). So each `compare_vecN` is one fp16
  `FCMP_OR.lt.f1` of an `FMAX` tree of the absolute differences. `v_vtxOut =
  1.0` folds across the link (no varying load, and no varying program in the
  vertex shader), so `result` is the compare result itself. Section 4's step
  2 ("16-bit selects and an fp16 product chain") should be read as "fold the
  compare chain to the FMAX tree", with the tree order of cmpv4 and cmpmat2.
- The whole program is pinned, colour included, so `lower-mixprec` can match
  the M1 shaders' colour too. The `ColourForm`/`FsOperandForm` that
  section 2.1 expects after :4502 is part of the same programs here.

Not made:

- `deqp_mixprec_rot2`: no refused V1 case is a (z, w, x) rotation. From the
  generator, every one of the 54 rotates 1 or 3 times mod 4: the basic,
  `*_increment`, `single_statement`, `unconditional_continue`,
  `vector_counter`, `function_call_*` and `nested_tricky_dataflow_2` cases 3,
  and `unconditional_break`, `infinite_with_unconditional_break_last`,
  `single_iteration`, `sequence` (5), `nested_sequence` (9) and the while
  `conditional_body` (`while(i++ < 3) if (i < 2)`, once) 1. The cases that
  rotate twice (`conditional_break`, `mixed_break_continue`,
  `infinite_with_conditional_break`) are not in V1's refusal. Their
  constant-count variants have no line in `refusals.tsv` at all, so the
  (z, w, x) colour already compiles. Their uniform and dynamic variants stop
  first at a loop, control-flow or intval refusal. `measure` confirms the
  counts.
- M2, M3, V2, V3: no generated pairs yet (section 4). A reference for them
  waits for `measure`.
- The `_both` variants, struct, array and samplerCube cases: other families'
  vertex and layout refusals come first (section 2.1).

## 7. Lowering map

Item `map-mixprec` (plan #377), 2026-11-17, for `lower-mixprec`. Read at
the integration tip `bd421fef367d3b75995f4c0fb621832302d5e261`. Every file
line below is at that commit (sections 1 to 6 quote 38ad30eb: `nir_fs.rs`
moved by about +5 lines in the part this family uses). Status of the
evidence: the eight first refusals in 7.1 were run
(`cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_deqp_mixprec -- --include-ignored`:
all eight reference tests fail with exactly the pinned texts, and
`refusals_pinned` passes). **The code site of each refusal and "what comes
next" were read from the code, not run under a probe**: this session could
not add a log line to the compiler, so each site is a trace of the walk
over the reference source. A site the trace could not settle is marked
*(read)*. The first `lower-mixprec` run should confirm each with a
temporary `eprintln!` of `line!()` at the sites named here.

### 7.1 Per reference: refusal, site, what is behind it

Files are in `src/shaderc/online/mali/`. All refusals are fragment-side;
every reference's vertex shader compiles.

**Which compile refuses.** `tests/mali_ref::compile_pair` compiles the
fragment shader alone first (what glCompileShader does, `compile_lowered`
with `FsConsts::NONE`, mod.rs:1642), and returns that error. The refusals
below are that standalone compile, and so are the board's `refusals.tsv`
lines. In a real program the link then recompiles the fragment shader
with the vertex shader's constant lanes folded in (backend.rs:1152,
`codegen::compile_fs_for_vs`, mod.rs:1757), which is the form Mesa's
reference is made from (`v_vtxOut = 1.0` folds, no varying program: the
`cmp*` references have no `vs.varying` region and a null
`secondary_offset`). **Both forms must compile**, and only the folded one
can equal the reference.

| Ref | Today's first refusal (pinned) | Site | Behind it |
|---|---|---|---|
| `rot3` `(w,x,y,1.0)` | `FsVaryingLoad` | `nir_fs.rs:conversion_order` :7049 (`same` false, :7047) | below |
| `rot1` `(y,z,w,1.0)` | `FsVaryingLoad` | the same :7049 | below |
| `cmpv2`, `cmpv3`, `cmpv4`, `cmpmat2`, `cmptex` | `MixedPrecision` | `nir_fs.rs:walk_nodes` :4507, the `Fmul`/`Fadd` arm :4492-4511 (`ok`, :4505) | below |
| `cmpf` | `ColourForm` | `nir_fs.rs:vectorise` last arm :6924 *(read)* | below |

**rot3 and rot1.** The colour pair `(w, x)` is a `Leaf::Lanes` (:6875; the
varying is kept whole, so :6871 holds), `(y, 1.0)` a `Leaf::Mk`
(:6891-6898). `select` (:7798-7814) gives each lane the conversion form of
its leaf: `Lanes` and `Mk` are "alone" (`pair = false`), so
`conv_form[k]` is three single conversions and `ours` in
`conversion_order` (:7027-7044) is `(x) (y) (w)`. Mesa's order from the
colour's lanes (:7004-7025) is `[w, x, y]`, which groups as `(w)`,
`(x, y)`: two instructions. `nm != no`, so :7049. rot1: order `[y, z, w]`
groups `(y)`, `(z, w)`.
*If :7049 were lifted* nothing else refuses: `used` is lanes {x, y, w} and
the load is `whole` (:7847 passes), `alpha` is the constant 1.0h, and
`conversion` (:10295) and `convert` (:10235) emit what `conv_form` says.
The next defect is not a refusal but wrong bytes or a `Defect::IrIndex`:
`conv_of` (:10455), `mkvec` (:10460) and `mkvec_lanes` (:10466) read a
converted lane as `.h0` of the register of the lane's own index, and
`lv.conv[k][c]` is set only at a conversion's first lane (`conversion`).
The odd lane of a pair (`y` of rot3's `(x, y)`, `w` of rot1's `(z, w)`)
finds no register.

**cmpv2, cmpv3, cmpv4, cmpmat2, cmptex (standalone).** Each
`compare_float` is `bcsel(flt(|a - b|, 0.05), 1.0, 0.0)`. The walk makes
the compare an fp16 `Fcmp` mask (:4587-4610, `mask16`) and the `bcsel` of
the mask and the constants 1.0 / 0.0 a 32-bit `B2f` (the arm at :4736,
:4752; `select32`, :1504). A product of two such selects takes the fp32
arm (:4492, `select32(x) && select32(y)`), and a third and fourth factor
too (`fp32_lane`, :1807: a 32-bit `Fmul`). The last product, `result *=
chain` with `result = v_vtxOut`, reads a raw varying lane (`Kind::Input`,
32-bit) beside the fp32 chain. The arm is taken because `fp32_lane(chain)`,
and `ok(x)` (:4505) is false for the varying: :4507. (cmpv2's chain is one
product of two selects; cmptex's `a.x - b.x` are fp16 `Fadd`s of fp16
texel lanes, :4533, so its chain is the same.)
*If :4507 were lifted* (standalone, raw `v_vtxOut`): `op32` (:1424) widens
nothing and `f.mixed` stays false; `mask_colour` (:8528) is false (the
16-bit `Fadd`/`Fabs` nodes are neither compares nor `paired`), so
mod.rs:1471 takes `select` (:7669). Its colour pair `(r, r)` reaches
`vectorise` `(Fmul, Fmul)` (:6829), then `vectorise(B2f, B2f)`, which has
no arm and ends at :6924 `ColourForm`: the refusal `cmpf` already has.

**cmpv2 and the others (linked form).** With `v_vtxOut` the constant 1.0,
the walk meets `Fmul(1.0, chain)`: the :4492 arm again (`fp32_lane(chain)`),
not refused at :4507, and `op32` folds the `fmul` by 1.0. The colour is
the fp32 chain, and the next refusal is the same :6924. So the
`MixedPrecision` text goes from M1 only when the compare chain is
rewritten (7.3 step 2), and **the first refusal after step 2 alone is
`ColourForm` :6924, which belongs to steps 3 and 4**.

**cmpf.** Standalone: `Fmul(v_vtxOut, B2f)` has no `fp32_lane` operand and
is not two selects, so it is not the :4492 arm; it falls to the generic
fp16 `Fmul` (:4533, `Fs::op` :1101) with a 32-bit `B2f` operand and no
widen. The colour pair `(r, r)` is `lane_word`'s `None` (:7223: `b` is no
constant), then `vectorise` :6829 `(Fmul, Fmul)`, whose `vectorise(B2f,
B2f)` ends at :6924. The folded form (`r` is the `B2f` itself, rule
`IdentityFirst`, :4563) ends at the same line. The test's `#[ignore]`
reason names plan #367's `FRAGCOLOR_LANE` lowering first: the `(r, r)`
and `(r, 1.0)` words of a *computed* lane.

### 7.2 What Mesa emits, per reference (from the expect.json)

Instruction forms as `bir.rs` / `emit.rs` name them: `FADD.v2f16` is
`row(Alu::Fadd, 16, 2)` (`FADD_IMM.v2f16`, `Op::FaddImmV2f16`, comes from
it by `opt::fuse_add_imm` when one source is a constant pair and the other
`H01`); `FMAX.v2f16` is `row(Fmax, 16, 1)` (emit.rs:238, both sources
`.h00` / `.h11` of a pair register); `FCMP_OR.v2f16.lt.f1` is
`Op::FcmpOrV2f16 { cmpf: Lt, result: ResultType::F1 }`; `MKVEC.v2i16` is
`row(Vec2, 16, 2)`; `IADD.v2u16 r, x.h00, 0` is `Vec2(x, x)` after
`lower_mkvec_swz`.

- **rot3** (`(w, x, y, 1.0)`): `LD_VAR_BUF_IMM.f32.v4` (r0..r3);
  `FADD.f32 r2.h0, r3, 0x0.neg` (w alone: `row(F2f16, 16, 1)`);
  `V2F32_TO_V2F16 r0, r0, r1` (x, y: `row(F2f16, 16, 2)`, x low); `ATEST`
  on the constant alpha 1.0h; `MKVEC.v2i16 r1, r2.h0, r0.h0` (w, x);
  `MKVEC.v2i16 r2, r0.h1, u2.w0.h0` (y, 1.0h); the two `MOV` that make
  `BLEND`'s sources contiguous; 11 words. The lone conversion comes
  **first**, then the pair, at the pair's own first lane.
- **rot1** (`(y, z, w, 1.0)`): `FADD.f32 r0.h0, r1, 0x0.neg` (y alone,
  first), `V2F32_TO_V2F16 r1, r2, r3` (z low, w high), `ATEST`, `MKVEC r0,
  r0.h0, r1.h0` (y, z), `MKVEC r1, r1.h1, 1.0h` (w, 1.0); 9 words.
  So the colour's lanes group into the load's aligned lane pairs where
  both lanes of the pair are colour lanes, whatever their leaf form; the
  odd lane is read as `.h1` of the pair's register; a lane whose partner
  is not read stays alone. These groups are what `conversion_order`'s
  `mesa` array already computes (:7015-7025, the `c ^ 1` pairing); only
  `ours` disagrees.
- **cmpf**: `FADD.v2f16 r0, u2.w0.h00, u2.w1` (the fp16 uniform and the
  literal -2.16, FAU `40524052`), `FCMP_OR.v2f16.lt.f1 r0, r0.abs.h00,
  u3.w0` against 0.05h (FAU `2a662a66`), `ATEST`, `IADD.v2u16 r1, r0.h00,
  0` (the `(r, r)` word), `MKVEC.v2i16 r2, r0.h0, u3.w1.h0` (`(r, 1.0)`),
  the two `MOV`, `BLEND`; 10 words. No `FMAX`, no `b2f`: the compare's
  `.f1` result is the 1.0h / 0.0h itself.
- **cmpv2**: `FADD_IMM.v2f16 r0, u2.w0, #0xC7CF359A` (both differences, the
  constants packed), `FMAX.v2f16 r0, r0.abs.h00, r0.abs.h11`,
  `FCMP_OR.v2f16.lt.f1 r0, r0.h00, u2.w1`, then cmpf's colour; 11 words.
- **cmpv3**: `FADD_IMM.v2f16 r0, u2.w0, #imm` (x, y); z alone: `MOV.i32 r1,
  u3.w0` (the pushed -3.12h pair) and `FADD.v2f16 r1, u2.w1.h00, r1`;
  `FMAX r0, r0.abs.h00, r0.abs.h11` (x, y); `FMAX r0, r1.abs.h00, r0.h00`
  (z first, then the (x, y) maximum); `FCMP_OR.lt.f1`; colour; 14 words.
- **cmpv4**: two `FADD_IMM.v2f16` ((x, y) and (z, w)); `FMAX r0, r0.abs.h00,
  r0.abs.h11` (x, y); `FMAX r1, r1.abs.h11, r1.abs.h00` (**w, z**: the
  high half first); `FMAX r0, r1.h00, r0.h00` (the (z, w) maximum first);
  `FCMP_OR.lt.f1 r0, r0.h00, u3.w0`; colour; 14 words. The tree is
  GLSL's: `do_rebalance_tree` (glsl.rs already ports it for products)
  makes `(x*y)*(z*w)` of the four-factor product, and NIR's
  `iand(flt(a, c), flt(b, c))` to `flt(fmax(a, b), c)`
  (nir_opt_algebraic.py:1159) with `fmul(b2f(a), b2f(b))` to
  `b2f(iand(a, b))` (:1679) turn it into this.
- **cmpmat2**: column 0's `FADD_IMM` + `FMAX(|.h00|, |.h11|)`, column 1's,
  then `FMAX r0, r0.h00, r1.h00` (column 0 first: the opposite operand
  order to cmpv4); `FCMP_OR.lt.f1`; colour; 14 words.
- **cmptex**: `MOV r0, 0; MOV r1, 0; MOV r2, u2.w0; MOV r3, 0`;
  `TEX_SINGLE.16.2d.computed @r0:r1` at (0, 0) (the existing `tex`
  emission); the texel words go through cmpv4's chain (`FADD_IMM.v2f16 r0,
  r0, #imm` on the texel register itself); 19 words.

### 7.3 Ordered lowering steps

Counts are the families note's section 9: `fs_uniform_compare_harness`
186 Fail (FS side only: 100 `_fragment`, 72 `_both`, 14
`uniform_api.random`; the `_both` cases also need their vertex shader,
other families), `fs_varying_rgb_one` 54 (13 Fail, 41 NotSupported), and 11
further cases with no reference (M2 2, M3 4, V2 2, V3 3). "Unblocks" means
the fragment compile no longer refuses there; a dEQP verdict also needs
the board draw.

**Step 0 (harness, before step 2).** The `cmp*` references cannot be
byte-compared by `tests/mali_ref::compile_pair`: it compiles the fragment
shader alone, with `v_vtxOut` a varying (a `vs.varying` region and a
varying load the reference lacks), like the intval note's `rsg_constbool_vs`
(`mali-deqp-intval.md`, risks (e)). Compare them with the unit tests'
`check_linked` (`tests.rs:525` `compile_linked`: `fs_constants`,
`compile_fs_for_vs`, `compile_vs_for_reads`) added to
`src/shaderc/online/mali/tests.rs`, and keep `tests/mali_deqp_mixprec.rs`'s
`cmp*` tests `#[ignore]`d with that reason (their pin still names the
standalone refusal, which moves with step 2). Add a unit assertion that the
standalone compile of each `cmp*` fragment shader no longer refuses (that
is what glCompileShader, and so the dEQP case, needs). `rot1` and `rot3`
need none of this (no constant varying) and compare in
`tests/mali_deqp_mixprec.rs` as they are.

1. **V1: the rotated colour's conversions.** Turns byte-equal: `rot1`,
   `rot3`. Unblocks 54 cases (13 Fail + 41 NotSupported). Functions
   (`nir_fs.rs`): `select` :7798-7815 (derive `conv_form` from the `mesa`
   grouping instead of the leaf kind), `conversion_order` :6951 (return the
   grouping; keep :6993 (two varyings), and keep refusing as today when a
   pair's first lane is odd (swap_yx, no reference) or any lane of the
   pair is a tree lane (`tree[k]`, :6969): `fsload_mk_pair` and
   `fsload_disc_col` have no reference), `conversion` :10295 and `convert`
   :10235 (the pair at the even lane, the other lane read through the
   pair's register), `conv_of` :10455, `mkvec` :10460, `mkvec_lanes`
   :10466, `operand`'s `Splat` arm (:10474) and `select`'s ATEST arm
   :8132 (read a lane as `.h0` or `.h1` of its group's register). The
   existing references that must stay byte-equal: `fsconv_cpq`,
   `fsconv_discw`, `fsconv_alpha3`, `intloop_ifloop` (`(z, w, x, 1.0)`: the
   pair first, then x), `fsload_*`, `varyconst_78` (`Lanes`). Remove the
   two `#[ignore]` and the two `PINNED` rows in the same commit.
   **glmark2 / #367:** no glmark2 verdict has this text
   (`tests/fixtures/mali_glmark2_verdicts.txt` has no `fragment varying
   load` line); `tools/mali/corpus-verdicts.txt:121`
   (`tools/shaders/es100/uniidx_j.frag`) has it and may move: re-run its
   writer. The change reaches every mediump fragment shader that loads a
   varying, so the glmark2 `ok` rows are in scope for a byte change
   (`tests/mali_glmark2_corpus.rs` pins verdicts only; the byte-compare is
   the references above). #367's `FRAGCOLOR_LANE` and fragment `ARITH`
   items edit `vectorise` / `word_of` near the same arms
   (`mali-varying-constants.md`, "gl_FragColor lane shapes"); re-read
   `conversion_order` on the latest main first. Same file as
   `lower-intval-1` and `lower-loops-1`. Rewrite the doc comment of
   `conversion_order` (:6931-6950), which names the old rule.

2. **M1: fold the compare chain (the walk).** Alone it turns nothing
   byte-equal (steps 3 and 4 finish the colour); it removes the
   `MixedPrecision` refusal of the 186. Functions: `walk_nodes` (:4094):
   the `Bcsel` arm :4736 and the `Fmul` arm :4492-4511, which must not
   take a product of selects of fp16 compares as an fp32 chain in a
   shader with no fp32 value (the `intbool_76_boolany` and `intbig_case79`
   shapes keep it). Add Mesa's two rules at node level: `fmul(b2f(a),
   b2f(b))` is `b2f(iand(a, b))` (`Fs::mask_op` :1532: two `mask16`s make
   a 16-bit `Iand`) and `iand(flt(x, c), flt(y, c))` is `flt(fmax(x, y),
   c)` for 16-bit compares of one constant `c` (a rewrite in `mask_op`,
   with a 16-bit `Alu::Fmax`). The last `result *= ...` with a raw mediump
   varying (the standalone form) is the generic fp16 `Fmul` of the varying
   and a 16-bit `B2f` (Mesa's `f2fmp(b2f32(a))` is `b2f16(a)`,
   nir_opt_algebraic.py:1980; `row(B2f, 16, 1)` exists, emit.rs:67); with
   the constant 1.0 it is the identity (`IdentityFirst`). :4505's `ok` and
   `select32` (:1504) stay for the fp32 shapes. **Overlap:** `mask_op` and
   `int_lane` are where `lower-intval-1` adds its arms
   (`mali-deqp-intval.md`); the `Fmul`/`Fadd` arms are #367's fragment
   `ARITH` and `INT_VALUE` ground (`mali-int-loops.md`: `tri_int` needs the
   same fp32-beside-fp16 colour). Keep the `intbool_76` and
   `mixprec_tests.rs` verdicts as they are. glmark2: a glmark2 fragment
   shader whose colour is a mediump select of a compare would move; none
   is `MixedPrecision` in the fixture, so a changed `ok` row would be
   new: compare `tests/fixtures/mali_glmark2_verdicts.txt` and
   `tools/mali/corpus-verdicts.txt` after the change and move them in the
   same commit.

3. **M1: the pair graph for the tree and the colour words.** Turns
   byte-equal, with steps 2 and 4: `cmpv2`, `cmpv3`, `cmpv4`, `cmpmat2`,
   `cmptex`, and `cmpf` (which also needs #367's `FRAGCOLOR_LANE` words:
   check whether it landed, else build the `(r, r)` and `(r, 1.0)` words
   here). Functions: `vectorise` :6751 (a new arm for `Fmax` of the two
   halves of one pair operation; the `FADD_IMM` pairs of uniform and texel
   pairs the `(Fadd, Fadd)` arm :6806 and the texel arm :6851 already
   make), `lane_tree` :7368, `lane_word` :7222 and `lone_tree` :7246 (the
   colour's `(r, 1.0)` and the `(r, r)` word of a 16-bit compare lane),
   `late_rules` :7622 if the `FMAX` order needs one, `op` :10507 (a scalar
   `Fmax` reads `.h00` / `.h11` of its pair operands; `lone` and `splat`
   there do not cover a half of a vector operation), `ADMITTED` and
   `Pairs::note` :6719 (a production not admitted is `FsOperandForm`,
   :7706). The operand order is the reference's (7.2): cmpv3 z first,
   cmpv4 (w, z) then (z, w) before (x, y), cmpmat2 column 0 first. Confirm
   that glsl.rs's `do_rebalance_tree` gives that tree and no `Rebalance`
   refusal stands first. The vertex side and `mask_colour` (:8528) are not
   touched: the chain is fp16 nodes, and `select` is the path (mod.rs:1476).
   Reference coverage: vec2 14, vec3 14, vec4 25, mat2 5, sampler2D 36 = 94
   of the 186 (no float case is in the 186: `cmpf`'s text is `ColourForm`,
   so it is counted elsewhere). **No reference covers** mat3 5, mat4 10,
   mat4_mat2 7, sampler2D_samplerCube 56 (two samplers, one a cube) and the
   14 `uniform_api.random` cases: 92 cases that only the same code reaches.
   Ask for `cmpmat4` and `cmpcube` references before claiming them; the
   cube one is the largest.

4. **M1: the 16-bit `.f1` fold (`opt.rs`).** Function
   `opt::mod_prop_backward` (opt.rs:253-300): its doc says Mesa folds a
   16-bit compare and its `MUX.v2i16` into `FCMP.v2f16 .f1` "with a
   swizzled mask composed into the compare's sources", and that this port's
   compares are 32-bit only. The `cmp*` references need it
   (`FCMP_OR.v2f16.lt.f1`, no `MUX`): `MuxV2i16 0, 1.0h, mask.h00` on a
   `FcmpOrV2f16` result. It touches every fragment program that makes a
   16-bit `b2f` of a compare, so it can change glmark2 words (verdicts do
   not move). Run the existing references that hold `.f1` / `MUX.v2i16`
   (`fbload_predif_fs_coord`, `s4_flt`, `s4_b2f`, `cflow_68`, `cflow_74`,
   `intbool_76`).

5. **The misleading text (optional, last).** M1's shaders mix nothing,
   and the refusal text says they do. After steps 2-4 no M1 shape reaches
   :4507; the line stays reachable for a real mix (a highp varying beside
   an fp32 select). Give it its own `MaliError` (a variant in mod.rs:146's
   list, :343's table and :427's text, with a `GAPS` line) so the tally
   tells the two apart. The pins in `tests/mali_deqp_*.rs`,
   `tools/cts/mali-family-refusals.txt` and the `refusals.tsv` readers
   (`mali-deqp-tally.py`, owned by plan #373) name the old text: **do not add
   a variant unless those move in the same commit**. No case unblocked.

M2, M3, V2, V3 (11 cases) have no reference and no generated pair in
`tools/mali/testdata/deqp_shapes/` beyond the four `mixprec_*` pairs
there (7.1's shapes). Leave them to a later reference.

### 7.4 Overlap with plan #367 and the glmark2 verdicts

- **FRAGCOLOR_LANE** (`mali-varying-constants.md`, "gl_FragColor lane
  shapes", the `(x, x)` and `(x, 1.0)` words): `cmpf` and the colour of
  every `cmp*` is `vec4(r, r, r, 1.0)` of a *computed* lane (`IADD.v2u16
  r.h00, 0`, `MKVEC r.h0, 1.0h`). That is #367's item, and the `cmpf` test
  says so in its `#[ignore]` reason. Step 3 builds the words here only if
  #367 has not.
- **Fragment ARITH / INT_VALUE** (`mali-int-loops.md`,
  `mali-deqp-intval.md`): the `Fmul`/`Fadd` arm at :4492 and `mask_op` are
  shared ground; the fp32 select beside an fp16 colour (`tri_int`) must keep
  refusing.
- **Varying link rules** (`mali-varying-constants.md`,
  `mali-deqp-varylink.md`): the `cmp*` pairs depend on the link's
  `v_vtxOut = 1.0` fold (`FsConsts`); step 0's `check_linked` takes the
  path varylink's does, and `lower-varylink` changes `fs_constants` /
  `VaryingConsts` (link.rs, mod.rs:1679-1740). The folded reference has no
  varying program; if `lower-varylink` lands first, re-read.
- **glmark2 verdicts** (`tests/fixtures/mali_glmark2_verdicts.txt`, compared
  with `tools/mali/corpus-verdicts.txt`): the fixture has no
  `MixedPrecision` and no `FsVaryingLoad` line; 8 of its 97 lines are the
  `ColourForm` text of `FRAGCOLOR_LANE` (:15-90). Steps 1, 3 and 4 edit code
  that passing glmark2 fragment shaders run through, so re-run the
  corpus writer (`MALI_GLMARK2_WRITE=1`) once per step and move the fixture
  only if a verdict changes. `corpus-verdicts.txt:121` (`uniidx_j.frag`)
  is the one line with the `FsVaryingLoad` text; step 1 may lift it.
- **Tests that pin what this map changes:**
  `src/shaderc/online/mali/mixprec_tests.rs` (three `MixedPrecision`
  verdicts, all on a highp shader: they stay) and
  `tests/mali_deqp_mixprec.rs` (`PINNED` and the `#[ignore]`s; item
  `map-misc` re-pins the eight family tests, so read the file at the
  latest main).

## 8. What `lower-mixprec` did

Item `lower-mixprec` (plan #377), 2026-11-18, on main 5bd67e35 (after
`lower-intval-1`, ab9cde96, and `constop`, 0fd576ea). Host only; the board
is not run.

- **Byte-equal to Mesa 26.2.1** (`tests/mali_deqp_mixprec.rs`, `MATCHED`):
  all eight. The first session (below) made `cmpf`, `cmpv2`, `rot1` and
  `rot3` byte-equal; the second (8.1) made `cmpv3`, `cmpv4`, `cmptex` and
  `cmpmat2` byte-equal, so `DIFFERS` is empty and no test is ignored.
- **Step 0.** Step 0 was not needed: `mali_ref::compile_pair` compiles the
  `cmp*` pairs with the vertex shader's constant `v_vtxOut` folded, which is
  the reference's program. The shader objects with `v_vtxOut` still a
  varying (glCompileShader's view) compile:
  `the_compare_chain_shader_objects_compile` in `tests.rs`.
- **Step 1 (V1).** In `conversion_order`, an aligned pair of colour lanes
  that no tree reads is one `V2F32_TO_V2F16`. The even lane is low, and the
  odd lane is read as `.h1` of the pair's register (`Leaves::conv_hi`,
  `conv_lane`). A pair whose first lane is odd, or that a tree reads, is
  still refused. This makes `fsload_mk_pair` byte-equal, so it moves from
  the refusal list to the colour-conversion test in `tests.rs`, and
  `fsload_disc_col` remains the `FsVaryingLoad` probe. In
  `tests/mali_deqp_constop.rs`, `loops_for_basic_mediump_float_fs` becomes
  byte-equal. `loops_while_basic_highp_float_fs` moves on to the
  `MixedPrecision` refusal, which is a real mix.
- **Steps 2 and 3 (M1).** These are in `walk_nodes`, `Fs::cmp_select` and
  `Fs::and_masks`. `fmul(b2f(a), b2f(b))` becomes `b2f(iand(a, b))`, and
  `iand(flt(x, c), flt(y, c))` becomes `flt(fmax(x, y), c)`. A product with
  another mediump value becomes the fp16 `fmul` of a `b2f16`; with the
  standalone varying, it becomes `bcsel(a, v, 0.0)`. In the pair graph,
  `lane_tree` and `lone_tree` take the `b2f16` of one compare, its select of
  a lane and 0.0, and `fabs`. `mates` pairs two consecutive dwords of an
  fp16 uniform. A constant-`v_vtxOut` program whose colour is all `b2f`
  lanes or constants is sent by `lower-intval-1`'s `bool_colour` to
  `select_highp` instead (seen for `cmpv2`: `vectorize16` pairs the
  differences and their `fabs`).
- **Step 4** (the 16-bit `.f1` fold in `opt::mod_prop_backward`) landed
  with `lower-intval-1` and is used as is.
- **emit.rs.** A one-lane fp16 `FMAX` now keeps a source's high half
  (`lone`, as `FADD` and `FMIN` already do). Before this change the row
  forced `.h00`. Through `bool_colour`, `cmpv2`'s `FMAX` reads the two
  halves of the paired difference (`r0.abs.h00, r0.abs.h11`), and before
  this change it read the low half twice.
- **Step 5** (a separate text for M1) is not done: no M1 reference reaches
  the `MixedPrecision` site now.
- `tools/mali/corpus-verdicts.txt` did not move (`uniidx_j.frag` keeps its
  `FsVaryingLoad` text). No glmark2 verdict moves:
  `tests/mali_glmark2_corpus.rs` over a corpus that `run.sh` made from
  glmark2 22c527cb reports the same 21 stale rows with and without this
  change. Those rows are main's own fixture lag, so the fixture is not
  regenerated here. The dEQP cases this
  unblocks were not measured here; the next whole-tree or host measure
  counts them.

### 8.1 Second session: `cmpv3`, `cmpv4`, `cmptex`, `cmpmat2`

Item `lower-mixprec` again (plan #377), 2026-11-20, on the integration tip
98969940. Host only. The order was read from Mesa itself: the drm-shim of
`tools/mali/drm_shim_ref.sh build` (into `$CONNECTED_DEPS_DIR/mesa-shim`)
run on each reference's pair with `NIR_DEBUG=print_fs`, which prints the
shader after every NIR pass with the pass's name (in `<out>.stdout`).

- **No rebalance.** Section 7.2 said `cmpv4`'s tree is GLSL's rebalanced
  `(x*y)*(z*w)`. It is not: the first fragment shader NIR prints holds the
  left chain `((x*y)*z)*w` of `compare_vec4`'s return. The order comes from
  `nir_opt_algebraic`. Its first run makes `x*y` the `b2f(iand(x, y))`
  (nir_opt_algebraic.py:1679), but a product whose other operand is still
  a `fmul` matches only `fmul(b2f(a), b)`, so `(x*y)*z` is `bcsel(z,
  x*y, 0.0)` and `.. * w` is `bcsel(w, .., 0.0)`. The next runs fold
  `bcsel(w, bcsel(z, v, 0), 0)` to `bcsel(iand(w, z), v, 0)`, then
  `iand(flt(a, c), flt(b, c))` to `flt(fmax(a, b), c)`: `fmax(fmax(w, z),
  fmax(x, y))` (`cmpv4`, `cmptex`) and `fmax(z, fmax(x, y))` (`cmpv3`). A
  product of two products (`cmpmat2`'s two `compare_vec2` calls) waits
  until both are `b2f`s, so it keeps the source order: `fmax(column 0,
  column 1)`. This is `Fs::cmp_product` (nir_fs.rs), called by the walk's
  compare-product arm in place of a bare `and_masks`. It keeps a small table
  (`Fs::cmp_chain`) of the `b2f` nodes it made, with the leaves folded so
  far and the base product's mask. The `fmax(z, xy)` node a three-leaf step
  makes is left to the dead code when a fourth leaf follows. A chain past
  four leaves has no reference; it folds each new leaf in front of the
  ones before.
- **cmpmat2's order** (`call_returns` in nir_fs.rs, called by
  `order_keys`). `order_keys` places every `?:` of a statement before the
  statement (GLSL's `conditional_tmp`), and the products after. Column 0's
  product `compare_vec2(a[0], b[0])` is an inlined call's whole return,
  which in GLSL is its own statement (`retval = a * b`) before the second
  call's parameter assignments. Mesa's NIR has column 0's `fadd`, `fabs`
  and `fmax` before column 1's. The rule: an operation of a statement
  whose operands are all such placed `?:` trees, and after which a separate
  statement's store (the next call's parameter copy) stands before a
  further `?:` of the same statement, takes the `?:` trees' place, by its
  index. `compare_vec4`'s products have no store in between and stay after
  every call.
- **cmptex's pairing** (`mates` in nir_fs.rs). Two scalar fp16 operations
  whose sources are lanes `2k` and `2k + 1` of one fp16 look-up's texel
  join as one `.v2f16` operation, like two halves of one uniform word. The
  differences become `FADD_IMM.v2f16 r0, r0^, #..` on the texel registers.
- Checks, feature set `host-test,gpu-mali,shaderc-online,no-shader-table`:
  every `tests/mali_deqp_*.rs` passes, no test of another family that was
  ignored passes now, and the `shaderc::online::mali` unit tests pass (300).
  `tests/mali_glmark2_corpus.rs` over a corpus that `run.sh` made from
  glmark2 22c527cb gives the same verdicts with and without this change.
  Written with `MALI_GLMARK2_WRITE=1` into a scratch copy, both equal the
  committed fixture line for line. `tools/mali/corpus-verdicts.txt` does
  not move (`the_corpus_verdicts_are_pinned` passes).
- **Left:** the `MixedPrecision` text (step 5) and M2, M3, V2, V3 (no
  reference), as before. The 92 `compare_*` cases with no reference (mat3,
  mat4, mat4_mat2, the cube sampler, `uniform_api.random`) go through the
  same code. Their chains are longer (`compare_mat3` is three `compare_vec3`
  products), so a reference is still needed before claiming them.
