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
