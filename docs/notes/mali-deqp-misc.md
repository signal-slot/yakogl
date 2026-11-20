<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the small dEQP-GLES2 refusals (study)

**Status: study** (plan #377, item `study-misc`; issues #20, #7). This note
changes no code; its references are the `deqp_misc_*` of "References"
(item `ref-misc`) at the end. It covers the seven small compile
refusals of section M of `docs/es2-cts-failures.md` (whole tree at
`37305608`, 2026-11-15). For each one it gives where we refuse, which dEQP
cases hit it, what Mesa 26.2.1 does, and a verdict: **lower now** (in the one
session planned in "Plan" below) or **later**. State read: `faeb6962`.
Mesa paths are relative to `src/` of the 26.2.1 tree (`$MESA_PAN_TREE`).

| refusal (`MaliError`) | cases (Fail) | verdict |
|---|---|---|
| an invariant output (`Invariant`) | 12 | later: every case refuses again in another family |
| more than the split file's 32 work registers (`Spill`) | 2 | **lower now**, once the reference shows whether the fix is the full file or where the widenings go |
| more than 2048 IR values (`TooManyValues`) | 3 (VS 2, FS 1) | later |
| a comparison other than `<`, `<=`, `>`, `>=`, `==` (`CompareForm`) | 3 | **lower now**: highp `!=` |
| a texture result's lanes other than its own pairs (`TexLanes`) | 2 | later: waits on #367's TEX_COORD |
| a built-in input (`SpecialInput`) | 1 | **lower now**: mediump `gl_FragCoord.z` |
| a shape Mesa's algebraic rules rewrite (`AlgebraicForm`) | 6 | **lower now**: the factored constant sum |

## How this was measured

* The cases are the rows of `tools/cts/log/2026-11-15-t6-gles2-S{1..4}-37305608/refusals.tsv`
  whose text is one of the seven. The dEQP sources are VK-GL-CTS `93bca01`, the
  commit `DEQP_WORK` pins (`tools/cts/deps.toml`), read from GitHub:
  `es2fShaderInvarianceTests.cpp`, `es2fShaderMatrixTests.cpp`,
  `es2fShaderStructTests.cpp`, `es2fShaderBuiltinVarTests.cpp`,
  `glsShaderConstExprTests.cpp`, `glsShaderLibraryCase.cpp`, and
  `data/gles2/shaders/{preprocessor,qualification_order}.test`.
* **"Measured" below** means a scratch run on this host. A throwaway
  `tests/` file compiled the rebuilt pairs through `tests/mali_ref`
  `compile_pair` (feature set `host-test,gpu-mali,shaderc-online,no-shader-table`).
  A temporary `eprintln!` at each `Err(MaliError::…)` of the four refusals
  that share a message named the line that fired. A temporary switch skipped
  the `Invariant` check to see the next refusal. All of it was reverted and
  nothing was committed. The pairs were rebuilt from the C++ generators: they
  are close to dEQP's but not dEQP's bytes. The `measure` item's
  `tools/mali/testdata/deqp_shapes/` pairs are the real ones.
* **"pan_shc"** below means Mesa 26.2.1's compiler run through
  `tools/mali/glsl2pan.py` (`$MESA_PAN_TREE/build-pan`) on the rebuilt pairs.
  That is the SPIR-V path (glslang, `310 es`), not the GL path that
  `drm_shim_ref.sh` draws. The two differ on mediump: in the SPIR-V runs
  below, `gl_FragCoord` and the vertex shader's mediump uniforms stay fp32,
  whereas the GL path narrows them (`mali-fragcoord-mediump.md` section 3,
  `nir.rs:1032-1044`). So these runs show the shape of Mesa's rewrites, but
  are not references. No drm-shim build was on this host, so no GL-path
  word was recorded. Claims about the GL path's words are marked
  **inferred** and are left to `ref-misc`.

## 1. `Invariant`: "an invariant output (Mesa keeps its arithmetic exact, unfused)"

**Site.** `src/shaderc/online/mali/nir.rs:3309-3313` (`build`, the vertex walk):
after the walk, any `ir::Output` with `invariant` set (`src/shaderc/online/ir.rs:737-746`;
`invariant gl_Position`, `invariant varying`, `#pragma STDGL invariant(all)`)
refuses. Pinned by `tests.rs:1125`. The check comes after the walk, so an
earlier refusal of the same shader wins (the `common_subexpression_1` cases
refuse earlier, in another family).

**Cases (12 Fail).** `shaders.invariance.{highp,mediump,lowp}.common_subexpression_{0,2,3}`
(9), `shaders.preprocessor.pragmas.pragma_vertex`,
`shaders.qualification_order.variables.valid.invariant_storage` and
`…invariant_storage_precision`.

* The invariance cases draw two programs. Each declares `invariant gl_Position`
  and computes the same position. Program A also writes `v_unrelated` from
  shared subexpressions; program B writes `vec4(0.0)`. The fragment shader
  is `uniform mediump vec4 u_color; varying mediump vec4 v_unrelated;
  … gl_FragColor = vec4(u_color.r, u_color.g, dot(v_unrelated, vec4(1.0)), u_color.a);`.
* `pragma_vertex` puts `#pragma STDGL invariant(all)` in the vertex shader
  (`v_val = 1.0`). The two qualification-order cases declare
  `invariant varying lowp float x0` with `x0 = 1.0`.

**Mesa 26.2.1.**
* `propagate_invariance` marks every variable that feeds an invariant one
  as invariant. It runs first and last in the GLSL optimiser loop
  (`compiler/glsl/propagate_invariance.cpp:77-103`,
  `compiler/glsl/glsl_parser_extras.cpp:2597`, `:2612`).
* `do_rebalance_tree` skips an assignment to an invariant or precise
  variable (`compiler/glsl/opt_rebalance_tree.cpp:155-160`; also
  `mali-rebalance.md` line 49).
* `glsl_to_nir` builds such an assignment with `nir_fp_exact |
  nir_fp_preserve_nan | nir_fp_preserve_inf` (`compiler/glsl/glsl_to_nir.cpp:1899-1902`).
  So no `~` (inexact) rule of `nir_opt_algebraic.py` applies to it, and the
  late fusion of `fadd(contract)(fmul(a, b), c)` into `ffma`
  (`compiler/nir/nir_opt_algebraic.py:3814-3818`; panfrost asks for it,
  `panfrost/compiler/bifrost/bifrost_compile.h:128-131`) does not fire:
  an `FMUL` then an `FADD`, not an `FMA`.
* Whether the viewport transform that `nir_lower_viewport_transform` adds
  (`compiler/nir/nir_lower_viewport_transform.c:73-96`) stays unfused for an
  invariant `gl_Position` is **not established** for the GL path. The
  comment at `nir.rs:3309` says it does. pan_shc's SPIR-V path for
  `invariant gl_Position; … gl_Position = a_position;` shows the transform
  **fused**, word for word as without `invariant`: `FMA.f32 r0, r0^, u0.w0,
  u0.w1` after the `FRCP`/`FMAX`/`FMIN` clamp. The SPIR-V front end does not
  set the GLSL front end's exact flag, so a GL-path reference must settle
  it.

**Measured: what comes after.** With the check skipped, no case compiles:

| case | next refusal (family) |
|---|---|
| `common_subexpression_{0,2}`, program A (all precisions) | two varying lanes of equal value (varylink) |
| `common_subexpression_{0,2,3}`, program B; `_3` program A | a mediump fragment shader uniform other than … (unitype): the fragment shader's `u_color` lanes |
| `pragma_vertex`, `invariant_storage*` | a fragment shader operand of a form no reference draws (`FsOperandForm`, the colour of `x0 + x1`, or `vec4(v_val)`) |

The last row's fragment shaders were rebuilt from a summary of the `.test`
files, so that row is the weakest measurement here.

**Verdict: later.** Lowering `Invariant` unblocks no case on its own: every
case refuses again in another family (unitype, varylink, `FsOperandForm`).
The lowering would carry an exact flag from the invariant outputs back
through their operands, as `propagate_invariance` does. It would turn off
the inexact rules and the `FMA` fusion that the walk models for those
values. It would also need its own references: an exact `FMUL`+`FADD`
chain, and the viewport transform of an invariant `gl_Position`. Revisit it
after `lower-unitype` and `lower-varylink`. Then, first take the cheap
subset: an invariant output whose value has no float arithmetic (an
attribute copy, a constant; `pragma_vertex`, `invariant_storage*`). Even
that subset needs the viewport-transform reference first, because
`pragma_vertex` makes `gl_Position` invariant too.

## 2. `Spill`: "more than the split file's 32 work registers (the full file, spilling)"

**Site.** `src/shaderc/online/mali/ra.rs:696-697` (`allocate_in`): the one
LCRA solve runs with the split file's affinity `0xffff | 0xffff << 48`
(r0-r15, r48-r63; `ra.rs:592-594`). When it fails we refuse, and the
allocator returns `WORK_REGS_SPLIT` = 32 (`ra.rs:259`, `:730`). Pinned by
`tests.rs:4081`.

**Cases (2 Fail).** `shaders.matrix.mul.uniform_{lowp,mediump}_mat4_mat4_vertex`:

    attribute highp vec4 a_position;
    varying mediump vec4 v_color;
    uniform <p> mat4 u_in0;
    uniform <p> mat4 u_in1;
    void main (void) {
        gl_Position = a_position;
        <p> mat4 res = u_in0 * u_in1;
        v_color = vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0);
    }

Measured: `uniform_highp_mat4_mat4_vertex` compiles (32 registers) and
passes on the board. lowp and mediump refuse `Spill`. The difference is the
fp16 uniforms. A mediump or lowp `mat4` is fp16 in the block
(`compiler/glsl/lower_precision.cpp:1023-1060` with `fp16_const_buffers`,
`nir.rs:1032-1044`), and each of its 32 lanes is widened by its own
`F16_TO_F32` into a register (`nir.rs:2942-2950`). The highp operands stay
FAU reads. So 32 widened values plus the products are live at once.

**Mesa 26.2.1.** `bi_register_allocate`
(`panfrost/compiler/bifrost/bi_ra.c:1219-1318`):
1. An SSA pre-spill runs only when the register demand is above
   `BI_MAX_REGS` = 64 (`:1237-1267`).
2. Out of SSA, then one try in the split file
   (`bi_allocate_registers(.., false)`, `:1275-1285`). On success,
   `work_reg_count` = 32.
3. Otherwise the whole file, `BITFIELD64_MASK(64)` (`:435-438`,
   `:1287-1293`), with `work_reg_count` = `BI_MAX_REGS` (64). It spills a
   node only when the whole file fails too (`:1294-1305`).

`bi_make_affinity` without `split_file` keeps the end-of-file clamp for
vectors and drops the r16-r47 hole (`bi_ra.c:296-320`). Our
`ra.rs:530` `make_affinity(.., split)` already has that parameter. On the
driver side, `src/mali/program.rs:27` (`register_allocation`) already maps a
count above 32 to the half-thread allocation, for the varying program too
(`program.rs:164`).

**pan_shc (SPIR-V path), mediump case.** `work_reg_count` 32 and
`secondary_work_reg_count` 32. The varying program reads the uniforms
straight from the FAU as fp32 (`FMA.f32 r1, u17.w1, r0, 0x0.neg`, 64
`FMA`s, highest register r12). The SPIR-V path keeps the vertex shader's
mediump uniforms 32-bit, so this run does not show the GL path's fp16
block. It does show that Mesa reaches this program inside the split file
when it does not widen.

**Verdict: lower now, but only after its GL-path reference.** The open
question is whether Mesa's GL path, with its 32 `F16_TO_F32` widenings,
needs the whole file or stays at 32.
* If the drm-shim reference has `secondary_work_reg_count` 64, the
  lowering is the whole-file retry: when the split solve fails, solve again
  with default affinity `!0` and `make_affinity(.., false)`, and return 64
  from `allocate_in`. Keep `Spill` when the whole file fails too: we do not
  model the SSA pre-spill or `bi_spill_register`.
* If it has 32, the gap is where we put the widenings. Ours are all live at
  once; Mesa's would sit next to their uses (scheduling or sinking, to be
  read off the words). The whole-file retry would then compile, but with
  words that are not Mesa's.

Either way the work is small, and the highp twin shows that the rest of
the program already compiles. Corpus check: no
`tools/mali/corpus-verdicts.txt` or `tests/fixtures/mali_glmark2_verdicts.txt`
line refuses `Spill` today, so no pinned verdict moves.

## 3. `TooManyValues`: "a shader of more than 2048 IR values (or 1536 instructions before selection)"

**Sites.** `nir.rs:2849-2850` (vertex walk: `m.instrs.len() > MAX_VALUES`,
2048, `nir.rs:63`), `nir_fs.rs:3040-3041` (fragment walk, the same
bound), `nir.rs:1877-1878` and `:3885-3886` (more than `MAX_NODES` = 1536
nodes, `nir.rs:60`). The fragment side has further fixed tables
(`nir_fs.rs:5629`, `:8657`, `:8677`, `:9511`, `:9606`).

**Cases (3 Fail).** `uniform_api.random.3` and `.79` (vertex),
`uniform_api.random.23` (fragment). These shaders are generated from a seed
in `es2fUniformApiTests.cpp`: many uniforms of random types (structs,
arrays, `bool`/`int` vectors, samplers), each read and compared. Their
sources are not in the tree. Which of the sites above fires is for the
`measure` item's pairs to show.

**Mesa 26.2.1.** No such bound: NIR and the bifrost IR are dynamic.

**Verdict: later.** The tables are sized for the no-std scratch arena.
Raising them costs memory on every board for three cases. Those cases are
also dense in the shapes the unitype, uniarray and intval families refuse
(non-float uniform types, uniform arrays, integer and boolean values), so a
larger table alone would only move them to another refusal. Reconsider once
those families are lowered, with the `measure` item's sizes for these three
pairs.

## 4. `CompareForm`: "a comparison other than <, <=, >, >= and == of floats and < of ints"

**Site.** `nir_fs.rs:2509-2511`, the highp `alu32` arm
`AluOp::Fne | AluOp::Ine => Err(MaliError::CompareForm)` (the only raiser).
Pinned by `tests.rs:1243`. Design and measurements:
`docs/notes/mali-compare-forms.md`.

**Cases (3 Fail).** `shaders.constant_expressions.builtin_functions.vector_relational.notEqual_vec{2,3,4}_fragment`.
The `_vertex` twins fail on other refusals. The shader is
`glsShaderConstExprTests.cpp`'s template inside the shader-library
fragment-only wrapper (`glsShaderLibraryCase.cpp`): `precision highp float;
precision highp int; uniform bool ref_out0; bool out0;`, then
`const bool cval = notEqual(…)<.y|.z|.w>; out0 = cval; bool RES =
isOk(out0, ref_out0); gl_FragColor = vec4(RES, RES, RES, 1.0);`.

Measured: `cval` folds in the front end. The `!=` that is left comes from
the comparison against the `bool` uniform `ref_out0`. It refuses
`CompareForm` whether `cval` is true or false.

**Mesa 26.2.1.**
* A bool uniform is loaded as 32 bits and converted with `b2b1`
  (`compiler/nir/nir_lower_io.c:544-550`). **Inferred:** after the algebraic
  passes, this is `ine(u, 0)` or `ieq(u, 0)` depending on `cval`.
* `bi_translate_cmpf` maps `ine`/`fneu` to `FCMP_OR`/`ICMP_OR` `.ne` with the
  `m1` result (`mali-compare-forms.md` "What Mesa emits", measured there:
  `fne_hp`, `ine0_hp` byte-equal on the tip with step 1 applied).
* A GL bool uniform holds `~0` for true (`mesa/state_tracker/st_extensions.c:1356`,
  `UniformBooleanTrue`). This matters for the board run, not the words.

**Verdict: lower now.** This is step 1 of `mali-compare-forms.md`: the highp
`Fne` and `Ine` arm built as `Fcmp(Cmpf::Ne)` / `Icmp(Cmpf::Ne)` with the
`m1` result, and its emit rows (step 3 there). That note measured it
byte-equal, but its `cmp_*` references were never committed
(`tools/mali/testdata/` has no `cmp_fne*`), so `ref-misc` makes them. The
mediump `!=` (step 4 there) is not needed by these cases. Corpus check:
`tools/shaders/es100/sharedint.frag` refuses `CompareForm`. After the
lowering it will refuse something else (its `sin`, per that note), and
`corpus-verdicts.txt` must be rewritten in the same commit.

## 5. `TexLanes`: "a texture result's lanes other than its own pairs in order (a swizzle, a lane alone)"

**Sites.** `nir_fs.rs:6845-6848` (`vectorise`: a pair holding a texel lane
must be that look-up's lanes `2k`, `2k+1`), `nir_fs.rs:8357-8365`
(`word_of`, the colour word of an fp16 look-up), and `nir_fs.rs:7704-7709`
(texels read with a gap). Pinned by `tests.rs:1203`.

**Cases (2 Fail).** `shaders.struct.uniform.sampler_fragment` and
`sampler_array_fragment`:

    varying mediump vec4 v_coords;
    struct S { mediump float a; mediump vec3 b; sampler2D c; };
    uniform S s;            // sampler_array: uniform S s[2]; s[1].c, s[0].b, s[1].b.z, s[0].a
    void main (void) {
        gl_FragColor = vec4(texture2D(s.c, v_coords.xy * s.b.xy + s.b.z).rgb, s.a);
    }

Measured: the refusal is at `nir_fs.rs:6848`. The colour's second word pairs
the texel's `b` with the struct member `s.a`, a texel lane alone. A plain
shader with the same colour, `vec4(texture2D(t, v.xy).rgb, a)` with a vec4
varying, refuses `TexCoord` instead ("a texture coordinate other than a vec2
varying read whole").

**Mesa 26.2.1.** A 16-bit `vec4` is collected two halves to a word with
`MKVEC.v2i16` (`bi_make_vec_to`,
`panfrost/compiler/bifrost/bifrost_compile.c:814-847`; `bi_lower_mkvec_swz.c:184-185`).
The texel's `.h0` and the converted member sit in one word. Our closest
reference is case 42's `vec4(t.rgb, f.a)` of two array look-ups
(`MKVEC.v2i16 r3^.h0, r1.h0`, `nir_fs.rs:8361-8364`). A struct member stays
fp32 in the block (`nir.rs:1036-1040`), so `s.a` is converted first
(**inferred**).

**Verdict: later.** The coordinate `v_coords.xy * s.b.xy + s.b.z` is a
computed mediump coordinate, which is #367's TEX_COORD family. The plain
shader shows that a `.xy` of a vec4 varying already refuses `TexCoord`. So
lowering the texel/uniform word alone would only move these two cases to
`TexCoord`. Also, `tests/fixtures/mali_glmark2_verdicts.txt` line 22
(terrain fs `d697`) refuses `TexLanes`, so this refusal is shared with
#367's glmark2 work. Take it up after #367's TEX_COORD lowering, as one
reference of the `.rgb` + struct-member word.

## 6. `SpecialInput`: "a built-in input (gl_FragCoord, gl_FrontFacing, gl_PointCoord, gl_VertexID, gl_InstanceID)"

**Sites.** `nir_fs.rs:6327-6333` (a root reaching `gl_FragCoord`'s w, or a
mediump shader's z: `frag_zw`), `nir_fs.rs:4287` (any other `LoadInput`,
e.g. `gl_PointCoord`), and `nir.rs:2924` (vertex). Pinned by `tests.rs:1223`.

**Case (1 Fail).** `shaders.builtin_variable.fragcoord_xyz`:

    uniform mediump vec3 u_scale;    // (1/width, 1/height, 1.0)
    void main (void) { gl_FragColor = vec4(gl_FragCoord.xyz*u_scale, 1.0); }

The fragment shader has no `precision` statement: `gl_FragCoord` is
mediump in ES 1.00, so the product is fp16. Measured: the refusal is at
`nir_fs.rs:6331`. The same shader at highp compiles today (32 registers).
x and y in a mediump shader are already lowered (`prec_84`,
`mali-fragcoord-mediump.md`); only z is missing.

**Mesa 26.2.1.**
* `nir_lower_frag_coord_to_pixel_coord` splits `gl_FragCoord` into
  `u2f32(load_pixel_coord)` for x and y and `load_frag_coord_z` /
  `load_frag_coord_w` (`compiler/nir/nir_lower_frag_coord_to_pixel_coord.c:20-28`).
* `pan_nir_lower_var_special_pan` turns z into `load_var_special_pan` of
  `BI_VARYING_NAME_FRAG_Z` with a pixel barycentric
  (`panfrost/compiler/pan_nir_lower_var_special.c:12-38`). That is
  `LD_VAR_SPECIAL` of `frag_z`, already drawn for highp
  (`mali-fragcoord-mediump.md` section 5).
* In an ES 1.00 shader, `lower_precision` converts at the read
  (`mali-fragcoord-mediump.md` section 3). **Inferred:** z is
  `LD_VAR_SPECIAL` (32-bit), then `F32_TO_F16`, then the product with
  `u_scale.z` (an fp16 uniform half), paired with the constant `1.0`.

**pan_shc (SPIR-V path, so fp32 throughout).** x and y are
`U16_TO_F32` of `r59.h0`/`.h1` plus 0.5, with the y flip by `FMA`. z is
`LD_VAR_SPECIAL.f32.center … index:0x3` (`frag_z`), then
`FMA.f32 r2, r2^, u4.w0, 0x0.neg`, then a conversion (`FADD.f32 r2.h0`).
The second colour word is `MKVEC.v2i16 r1, r2^.h0, u4.w1.h0`: z with the
pushed constant 1.0. In the GL path the shader is mediump, so x, y and z are
converted at the read and the product is fp16 (**inferred**, as above).

**Verdict: lower now.** One case and one reference. The highp `frag_z` load
exists, the x/y pair exists, and the new part is the conversion of z and its
place in the second colour word. `w` (`load_frag_coord_w`) stays refused; no
case in this list needs it.

## 7. `AlgebraicForm`: "a shape Mesa's algebraic rules rewrite without a reference (max(x, 0.0), clamp(x, -1.0, 1.0), a negated float(bool) in a product)"

**Sites.**
* `nir_fs.rs:1103-1108` (`Fs::op`): `fadd(fmul(a, b), fmul(a, c))` of two
  constants `b`, `c` is lowered only when `b + c == 1.0`. Any other sum
  refuses. `factor` is at `nir_fs.rs:1121-1128`.
* The other raisers are `nir_fs.rs:2235` (a negated `float(bool)` in a
  product, `nir_opt_algebraic.py:1696`), `nir_fs.rs:7422` and `:7530` (a
  product with 2.0 in a pair or coordinate), and `nir_fs.rs:10110`
  (`fsat_signed` / `fclamp_pos`, `compiler/nir/nir_opcodes.py:1816-1817`).
  Pinned by `tests.rs:1244-1245`.

**Cases (6 Fail).** `shaders.matrix.mul.const_{lowp,mediump,highp}_{mat2,mat4}_float_fragment`:

    const <p> mat2 in0 = mat2(0.6, -1.0, 0.7, 0.4);        // mat4: translationMatrix(0.2, -0.3, 0.15)
    varying <p> float v_coords;
    void main (void) {
        <p> mat2 res = in0 * v_coords;
        gl_FragColor = vec4(res[0][0], res[1][0], res[0][1]+res[1][1], 1.0);
        // mat4: vec4(res[0].xyz+res[1].yzw+res[2].zwx+res[3].wxy, 1.0)
    }

Measured: all six refuse at `nir_fs.rs:1107`, at all three precisions. The
mat2 colour's z is `v*(-1.0) + v*0.4` (sum -0.6). In the mat4 colour, the
identity part's lanes add products with 1.0 (sum 2.0). The `mat3` twins
refuse `FsOperandForm` (another family), so they are not counted here.

**Mesa 26.2.1.**
* `~fadd(fmul(a, b), fmul(a, c))` becomes `fmul(a, fadd(b, c))`
  (`compiler/nir/nir_opt_algebraic.py:246`), and constant folding makes
  `b + c` one constant. In a mediump shader that sum is folded in fp16
  (`lower_precision` narrowed the constants first).
* Other rules involved: `fmul(a, -1.0)` → `fneg(a)` (`:296`),
  `fadd(a, a)` → `fmul(a, 2.0)` (`:274`), and panfrost's late
  `fmul(a, 2.0)` → `fadd(a, a)`
  (`panfrost/compiler/bifrost/bifrost_nir_algebraic.py:31`).
* **pan_shc (SPIR-V path; the arithmetic here is fp32, then converted).**
  Every colour lane is **one** product of `v` with a folded constant:
  * mat2: `FMA.f32 r1, u2.w0, r0, 0x0.neg`, the same for `u2.w1` and `u3.w0`
    (three pushed constants: 0.6, 0.7, and the z lane's -1.0 + 0.4).
  * mat4: x is `FMA.f32 r1, r0, 0x44000000.h1, 0x0.neg`. That is
    `v * 4.0` (fp16 0x4400 widened from the table): the four 1.0 products
    factored into one sum. y and z are `v * u2.w0`, `v * u2.w1`.

  Then `FADD.f32 rN.h0` conversions, and `MKVEC.v2i16` pairs the colour,
  the last pair with the pushed 1.0. So the factoring of `:246` with the
  sum folded is what Mesa does to these lanes. No `fadd(v, v)` survives:
  the 2.0 never stands alone. The GL path's mediump form (fp16 products,
  sums folded in fp16) is **inferred** and is what `ref-misc` records.

**Verdict: lower now, in two steps.** These six cases consist entirely of
this shape. Step A: mat2. Generalise `nir_fs.rs:1103-1108` to any sum
whose result is exact in the shader's precision (`f16_exact` in a mediump
shader, as `:1083-1094` does for products), built as one `fmul(a, k)` and
then run through the existing constant-product rules (`k` = 1.0 → `a`,
-1.0 → `fneg`). Step B: mat4. The SPIR-V run folds a four-term sum (rebalanced by GLSL)
into one `v * 4.0`. So step B extends the factoring to a chain of more than
two products of the same `a`, and must check that no intermediate
`fmul(a, 2.0)` reaches the `nir_fs.rs:7422` / `:7530` refusal. lowp
is mediump in Mesa (both fp16), so one mediump and one highp reference per
matrix size are enough.

## Plan: one session (`ref-misc`, then `lower-misc`)

Order by cost, each step its own commit with its test.

1. **`ref-misc`** (no code): with `tools/mali/deqp_ref.sh` (or
   `drm_shim_ref.sh run`) on the `measure` item's `deqp_shapes` pairs, make
   these references:
   * `deqp_misc_cmp_notequal_vec2` (the dEQP pair), `deqp_misc_cmp_fne_hp`
     and `deqp_misc_cmp_ine0_hp` (`mali-compare-forms.md`'s small shaders,
     never committed);
   * `deqp_misc_spill_{lowp,mediump}_mat4_mat4`;
   * `deqp_misc_fragcoord_xyz`;
   * `deqp_misc_constmat_{mat2,mat4}_{mediump,highp}`;
   * `deqp_misc_invariant_position`: `invariant gl_Position = a;` alone. It
     settles the viewport-transform question in section 1 for the later
     step.
2. **`lower-misc`**, steps in this order:
   1. `CompareForm`: the highp `Fne`/`Ine` arm (`nir_fs.rs:2511`) and its
      emit rows. Rewrite `corpus-verdicts.txt` (`sharedint.frag` moves).
   2. `Spill`: read `secondary_work_reg_count` of the
      `deqp_misc_spill_*` references first.
      * At 64: the whole-file retry in `ra.rs` (`allocate_in`), returning 64.
      * At 32: place the `F16_TO_F32` widenings as Mesa's words do.

      Keep `tests.rs:4081`'s refusal pin for a program that fails the whole
      file too, or change its shader so it still refuses.
   3. `SpecialInput`: the mediump z (`nir_fs.rs:6327-6333`), with
      `F32_TO_F16` of the `frag_z` load.
   4. `AlgebraicForm` step A (mat2), then step B (mat4) if the references
      allow.

   Test: `tests/mali_deqp_misc.rs` (header feature set
   `host-test,gpu-mali,shaderc-online,no-shader-table`). It includes
   `mod mali_ref;` and byte-compares each reference with
   `mali_ref::assert_matches`. It keeps `assert_refused` pins for the
   later ones: `Invariant` (`invariance` pair), `TexLanes` (`struct.uniform.sampler`
   pair), `TooManyValues`.

**Expected effect.** At most 3 + 2 + 1 + 6 = 12 Fail cases leave these
refusals. Whether they then pass is not established: the next refusal of
each was not measured, because a lowered step is needed to see past it.
`lower-misc` should rerun these cases through `compile_pair` and report the
next refusal for any that still refuse. Left for later: `Invariant` (12),
`TexLanes` (2), `TooManyValues` (3).

## References (item `ref-misc`, 2026-11-16)

Eleven Mesa 26.2.1 drm-shim references (the GL path; none through the
SPIR-V `pan_shc`), `tools/mali/testdata/deqp_misc_<name>.{vert,frag,expect.json}`,
one per reference the Plan above lists. Each `.vert`/`.frag` from dEQP's text
has a `.license` sidecar (The Khronos Group Inc.) naming the case and how the
text was obtained; the three small shaders of our own carry a comment, as the
`gm2_*` ones do.

**Build.** `drm_shim_ref.sh build` (through `deqp_ref.sh`) into
`$CONNECTED_DEPS_DIR/mesa-shim`, from `mesa-26.2.1.tar.xz` fetched into
`$CONNECTED_DEPS_DIR/mesa-tarball/` (sha256 `c47e81bd…a4442`, the
`tarball-sha256` of `third_party/mesa-26.2.1/UPSTREAM`), `LLVM_DIR=/usr/lib/llvm-21`.
Check of the build: `README-deqp_ref.md`'s self-check, `gm2_cf_ifelse_vs` and
`gm2_cf_ifelse_fs` remade byte for byte (`cmp` silent). `cmp_fne_hp` and
`cmp_ine0_hp` also give the fragment word hashes that `mali-compare-forms.md`
measured on another build (`79b6d6b8…`, `fdbe94b2…`). Every draw printed
`glGetError 0x0`.

**Command.** For each reference except `cmp_notequal_vec2`:

```sh
LLVM_DIR=/usr/lib/llvm-21 tools/mali/deqp_ref.sh tools/mali/testdata/deqp_misc_<name>.vert \
    tools/mali/testdata/deqp_misc_<name>.frag <scratch>/deqp_misc_<name> --instrument shim
# then the sources were checked equal (cmp) and <scratch>/deqp_misc_<name>.expect.json copied in
```

That is `drm_shim_ref.sh run ... --prim triangles --uniform-values`, then
`pan_ref_diff.py extract <trace> --draw`. `cmp_notequal_vec2` has a `bool`
uniform, and `gl_ref.py --uniform-values` refuses one ("does not set a bool
uniform"), so it is the same two steps without `--uniform-values`:

```sh
tools/mali/drm_shim_ref.sh run "$CONNECTED_DEPS_DIR/mesa-shim" tools/mali/testdata/deqp_misc_cmp_notequal_vec2.vert \
    tools/mali/testdata/deqp_misc_cmp_notequal_vec2.frag <scratch>/deqp_misc_cmp_notequal_vec2 --prim triangles
python3 -I tools/mali/pan_ref_diff.py extract <scratch>/deqp_misc_cmp_notequal_vec2.trace --draw \
    > tools/mali/testdata/deqp_misc_cmp_notequal_vec2.expect.json
```

`ref_out0` is then 0 (false); dEQP sets it true. The words do not depend on the
value; the FAU word is the uniform the driver fills.

**Sources.** "verbatim" is the `tools/mali/testdata/deqp_shapes/misc_*` pair
(the `.qpa` of `deqp-host-measure.sh`, VK-GL-CTS `93bca01`). "rebuilt" is that
pair edited as `modules/gles2/functional/es2fShaderMatrixTests.cpp` at
`93bca01` generates the named case (`git show 93bca01:<file>` of a blobless
clone in `$CONNECTED_DEPS_DIR/vk-gl-cts`). The edits:
- the case's precision on its inputs (`:946`, `:952`, `:973-974`: the
  attribute `a_coords` and the varying `v_coords` take the input's precision;
  `:983` the uniforms) and on `res` (input 0's precision, `:899`, `:1079-1080`);
- the mat4 constant `s_constInMat4[0]` = `tcu::translationMatrix(Vec3(0.2f,
  -0.3f, 0.15f))` (`:86`). `writeMatrixConstructor<4, 4>` (`:988`, `:1012`)
  writes it column by column, `", "` between values, each with
  `de::floatToString(x, 1)` (`:831-848`), so `0.15f` prints `0.2`. The
  declaration order (the `const` from input 0, then input 1's `varying`, a
  blank line, `void main (void)`) is `:943-1031`;
- the mat4 colour of `genGLSLMatToVec3Reduction` (`:1190`; mat2 `:1184`).

The rebuilt texts are dEQP's generator output by reading, not a captured
`.qpa`; `measure`'s pairs confirm them only for the verbatim ones.

| Reference | Case | Source | What Mesa does | Pins for `lower-misc` |
|---|---|---|---|---|
| `cmp_notequal_vec2` | `shaders.constant_expressions.builtin_functions.vector_relational.notEqual_vec2_fragment` | verbatim `misc_comparison_form` | 10 words, **no compare**: `MOV.i32 r0, u2.w0` (the bool uniform is the 32-bit mask itself), `MUX.v2i16 r0, 0x0, 0x3C000000.h11, r0^.h00`, `IADD.v2u16 r1, r0.h00, 0x0`, `MKVEC.v2i16 r2, r0^.h0, u2.w1.h0` (FAU `0x3c003c00`) | the case does **not** need the `Ine` arm: `cval` is true, so `out0 == ref_out0` is `ref_out0` itself and Mesa drops the `ine(u, 0)`. Our `CompareForm` here comes from the bool uniform's `ine(u, 0)` (section 4); the step is to read a bool uniform as its own mask, then the `(RES, RES, RES, 1.0)` colour |
| `cmp_fne_hp` | none (`mali-compare-forms.md`'s `fne_hp`, with its `tc.vert`) | our text, as that note's table | 9 words: `FCMP_OR.f32.ne.m1 r0, r0^, r1^, 0x0` after a `.v2` load, then `MUX`/`ATEST`/`U16_TO_U32`/`MOV`/`BLEND` | the highp `Fne` arm, byte for byte (that note's step 1); no dEQP case of this list needs it, `bool(float)` cases do (`mali-alu-ops.md`) |
| `cmp_ine0_hp` | none (`ine0_hp`, `tc.vert`, `--uniform-values`: `m` = 16) | our text | 8 words: `ICMP_OR.u32.ne.m1.discard r0, u2.w0, 0x0, 0x0` | the highp `Ine` arm; `sharedint.frag`'s `u_p.mode != 0` is this compare |
| `spill_lowp_mat4_mat4` | `shaders.matrix.mul.uniform_lowp_mat4_mat4_vertex` | verbatim `misc_work_registers` | varying program 71 words, **`work_reg_count` 32** (the secondary program), highest register r8: the product is fp16, `FMA.v2f16` on the uniforms' fp16 FAU halves (`u5.w1`, `u6.w0.h00`, ...), sums by `FADD.v2f16`, then three `F16_TO_F32` of the results only | **neither register fix of section 2**: Mesa needs no whole file and places no widenings, because it widens nothing. The step is the mediump vertex `mat4 * mat4` in fp16 (`FMA.v2f16` with FAU half operands), which then fits the split file. Keep `Spill` and its pin `tests.rs:4081` for a program that is still over 32 |
| `spill_mediump_mat4_mat4` | `...uniform_mediump_mat4_mat4_vertex` | rebuilt: `lowp` → `mediump` on `u_in0`, `u_in1`, `res` | every region equal to `spill_lowp_mat4_mat4`'s (words, registers, preload, FAU) | lowp and mediump are one shape in Mesa |
| `fragcoord_xyz` | `shaders.builtin_variable.fragcoord_xyz` | verbatim `misc_builtin_input` | 17 words: x, y `U16_TO_F32` of `r59.h0`/`.h1` + 0.5 (y flipped by `FMA.f32 .., u2.w0, u2.w1`), converted by `FADD.f32 rN.h0, rN^, 0x0.neg`; z `LD_VAR_SPECIAL.f32.center.clobber.slot0 @r2, r61^, index:0x3`, converted the same way (**not** `F32_TO_F16`); `MKVEC` x/y, `FMA.v2f16 r0, r0^, u3.w0` (u_scale.xy), `FMA.v2f16 r1, r2^.h00, u3.w1.h00`, `MKVEC.v2i16 r1, r1^.h0, u4.w0.h0` (pushed 1.0); preload `0x3800000000000000` | the mediump z: the highp `frag_z` load, then the f32→f16 `FADD` (as x, y), then an fp16 product. The Plan's step 3 said `F32_TO_F16`; Mesa's word is the `FADD.f32 .h0` |
| `constmat_mat2_highp` | `shaders.matrix.mul.const_highp_mat2_float_fragment` | verbatim `misc_algebraic_rewrite` | 12 words: x `FMA.f32 r1, u2.w0, r0, 0x0.neg` (0.6), y `FADD.f32 r0.h0, r0^.neg, 0x0.neg` (the `-1.0` product is a `fneg` folded into the conversion), z `FMA.f32 r2, r0, u2.w1` with FAU `0x3f8ccccd` (1.1 = 0.7 + 0.4, one product); FAU `3f19999a 3f8ccccd 3c003c00` | step A: the sum `0.7 + 0.4` folded to one constant in fp32, and `-1.0` → `fneg`. dEQP's in0 is `mat2(0.6, 0.7, -1.0, 0.4)` (column-major), so the z sum is 1.1, not section 7's -0.6 |
| `constmat_mat2_mediump` | `...const_mediump_mat2_float_fragment` | rebuilt: `highp` → `mediump` (inputs, `in0`, `res`) | 13 words: `v` converted first (`FADD.f32 r0.h0`), then `FMA.v2f16 r1, u2.w0, r0.h00` (`0x38cd38cd`, 0.6 twice), `FADD.v2f16 r2, r0.neg.h00, 0x0.neg`, `FMA.v2f16 r0, r0^.h00, u2.w1` (`0x3c663c66`, 1.1 folded in fp16) | the mediump step A: fp16 products after one conversion, the folded sum an fp16 constant pushed twice in a word |
| `constmat_mat4_highp` | `...const_highp_mat4_float_fragment` | rebuilt (mat4 constant and colour) | 15 words: x `FMA.f32 r1, r0, 0x44000000.h1, 0x0.neg` (4.0, the four 1.0 products folded to one), y `v * 0x3e4ccccd` (0.2), z `v * 0xbe99999a` (-0.3) | step B: the four-term chain of products of `v` is one product with 4.0, an inline fp16 constant (`0x4400` of the table), as the SPIR-V run in section 7 showed |
| `constmat_mat4_mediump` | `...const_mediump_mat4_float_fragment` | rebuilt | 11 words: `v` converted, then **one** `FMA.v2f16 r1, r0.h00, u2.w0` with FAU `0x32664400` (halves 4.0, 0.2: x and y in one instruction), `FMA.v2f16 r0, r0^.h00, u2.w1` (`0xb4cdb4cd`, -0.3) | the mediump step B: the folded 4.0 is a FAU half next to 0.2 (not the inline `0x4400`), and x and y are one vector product |
| `invariant_position` | none (section 1's open question: `invariant gl_Position; gl_Position = a_pos;`, a constant mediump colour) | our text | position program 24 words, **every field equal** to `fragcoord_xyz`'s non-invariant `gl_Position = a_position` program: the viewport transform stays fused, `FMA.f32 r0, r0^, u0.w0, u0.w1` | answers section 1: on the GL path an invariant `gl_Position` that is a copy keeps the fused transform, so the cheap subset (`pragma_vertex`, `invariant_storage*`) needs no unfused viewport transform. An invariant output with arithmetic (the `common_subexpression_*` cases) is not covered by this reference |

What these references change in the Plan above, for `lower-misc`:
1. `CompareForm`: the three `notEqual_vec*_fragment` cases are the bool uniform
   read as a mask (`cmp_notequal_vec2`), not the `Ine` arm. The `Fne`/`Ine`
   arm (`cmp_fne_hp`, `cmp_ine0_hp`) is still what `sharedint.frag` needs.
   Only `_vec2` was drawn; `_vec3` and `_vec4` take another lane of the same
   folded `cval` and are expected to be the same program (**inferred**).
2. `Spill`: the answer is "32", but not because of where widenings go: Mesa's
   mediump/lowp vertex `mat4 * mat4` is fp16 arithmetic on FAU halves. The
   step is that fp16 product, not a register-allocator change.
3. `SpecialInput`: the conversion of z is `FADD.f32 rN.h0, rN^, 0x0.neg`.
4. `AlgebraicForm`: as the Plan, with the mediump forms above.

## What `lower-mixprec` did

Item `lower-mixprec` (plan #377), 2026-11-20, on the integration tip
98969940, after its mixprec step (`mali-deqp-mixprec.md` 8.1). Host only.
The steps follow the Plan's order. `tests/mali_deqp_misc.rs` byte-compares
each reference with `mali_ref::assert_matches`.

**Byte-equal now (5 of 11):**
- `cmp_fne_hp`, `cmp_ine0_hp`: already byte-equal at the tip (the highp
  `Fne`/`Ine` arm had landed).
- `cmp_notequal_vec2`: in a highp shader, the `i2b` of a `bool` uniform is
  the uniform's word itself, and `ieq(b, true)` of such a word is `b`
  (`alu32`'s lane arms, `bool_uniform32` in nir_fs.rs; the mediump arm
  already had both rules). dEQP's `isOk(true, ref_out0)` becomes the word,
  with no compare. The test uses `assert_matches_unset`, because the
  reference was drawn without `--uniform-values` (this note's References).
- `fragcoord_xyz`: in a mediump ES 1.00 shader, `gl_FragCoord.z` is the
  `FRAG_Z` special varying's fp32 lane, read the way a varying lane is
  (`frag_coord16`; `lane_tree`'s varying arm takes `FRAG_Z`). The colour
  narrows it with its own `FADD.f32 .h0`. Mesa narrows
  `vec4(u2f32(pixel_coord), z, w)` lane by lane, so the select's events
  place z's `LD_VAR_SPECIAL` and conversion after the x and y narrowing
  and before their `MKVEC` word (the `FRAG` event is split into its lanes
  and `FRAG_WORD`). As the reference showed, z's conversion is the
  `FADD.f32 .h0`, not `F32_TO_F16`. w stays refused.
- `invariant_position`: an invariant output whose every store is a copy
  (an attribute, a uniform or a constant, directly or through locals) has
  no arithmetic to keep exact. `build` (nir.rs) no longer refuses it
  (`invariant_copies`), and the program is the non-invariant one, with
  the viewport transform fused. The unit pin `tests.rs` keeps `Invariant`
  with `gl_Position = p * p`. This is the "cheap subset" of section 1
  (`pragma_vertex`, `invariant_storage*`). Those cases still refuse
  elsewhere (section 1's table); this step was not measured on them.

**Still refused (6), pinned in `REFUSALS`:**
- `spill_{lowp,mediump}_mat4_mat4` (`Spill`): not attempted. The reference
  is a 71-word varying program of `FMA.v2f16` on fp16 FAU halves, with
  swizzles such as `u7.w1.h11, u7.w0.h11` and a negated-zero accumulator
  (`r2.neg` of `MOV.i32 r2, 0x0`), then `F16_TO_F32` and `FMA.f32 r, r,
  0x0, r` of the three results. The vertex back end (nir.rs) has no fp16
  arithmetic. This is a vertex fp16 path of its own, with its vectorisation
  and order to be read off the drm-shim's NIR, not one step.
- `constmat_{mat2,mat4}_{highp,mediump}` (`AlgebraicForm`): tried and
  reverted. Generalising `Fs::op`'s factoring (`fadd(fmul(a, b), fmul(a,
  c))` to one `fmul(a, b + c)` folded at the shader's precision, and
  `fadd(fmul(a, b), a)`, :248) moves past this refusal, but nothing is
  byte-equal behind it:
  * `mat2_mediump` then compiles in 11 words against 13. Mesa shrinks each
    column's product to the lane still read once the factoring took the
    other (`nir_opt_shrink_vectors`), so `res[1][0]` is a scalar `fmul(v,
    -1.0)`, an fneg (`FADD.v2f16 r2, r0.neg.h00, 0x0.neg`), and `res[0][0]`
    is `FMA.v2f16 r1, u2.w0, r0.h00` with `0.6` in both halves. Our walk
    applies `Rule::Negate` only when both lanes of the column pair are -1.0,
    so it pairs x and y as one `FMA` with the constants `(0.6, -1.0)`.
    Matching needs the per-lane rules after a column lane dies, which no
    reference had pinned before.
  * `mat2_highp` then refuses `MixedPrecision`, and `mat4_*` refuse
    `ColourForm`. These were not followed further.

  A compile that is not byte-equal would go against this compiler's rule
  of refusing what no reference draws, so the step is left out and the
  refusal stays `AlgebraicForm`.

**Checks** (feature set `host-test,gpu-mali,shaderc-online,no-shader-table`):
every `tests/mali_deqp_*.rs` passes; no test of another family that was
ignored passes now; the `shaderc::online::mali` unit tests pass, including
`the_corpus_verdicts_are_pinned` (`tools/mali/corpus-verdicts.txt` does not
move). `tests/mali_glmark2_corpus.rs` gives the same verdicts with and
without these changes (`run.sh` corpus of glmark2 22c527cb, written into a
scratch copy with `MALI_GLMARK2_WRITE=1`, equal to the committed fixture).
