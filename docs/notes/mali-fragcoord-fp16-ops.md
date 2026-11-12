<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: case 218 shadowcmp, a mediump ES 3.00 shader's arithmetic on the highp gl_FragCoord

**Design and implementation, fs-fragcoord16** (plan #336, issue #33).
Mesa 26.2.1 is `MESA_PAN_TREE` (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`);
`G/` is `src/compiler/glsl/`, `N/` `src/compiler/nir/`, `B/`
`src/panfrost/compiler/bifrost/`, line numbers that tree's. Everything about
Mesa's output is *measured* on the drm-shim build of
`tools/mali/drm_shim_ref.sh` (`$CONNECTED_DEPS_DIR/mesa-shim`): the
reference `tools/mali/testdata/fsarith_218_shadowcmp.{vert,frag,expect.json}`
(drawn twice, byte-equal), its final NIR and disassembly
(`BIFROST_MESA_DEBUG=shaders`, the `.stderr` of the run) and its GLSL IR
after the precision pass (`MESA_GLSL=dump`, a second run of the same
pair). Our side is `src/shaderc/online/mali/nir_fs.rs` at this commit; the
names cited are functions, re-grep them before relying on a line.

This note continues mali-fragcoord-mediump.md (the mediump **ES 1.00**
read, narrowed at the load) and mali-highp-fragcoord.md (the highp read of
a highp shader). Case 218 is the third shape: a **mediump ES 3.00** shader,
whose `gl_FragCoord` is highp (`G/builtin_variables.cpp:1333-1335`,
`frag_coord_precision`), so the arithmetic on it is an island of fp32 in an
fp16 shader, the mirror image of `Fs::alu16`'s fp16 island in a highp one.

## 1. The case and the refusal

`driver/tools/pvrlab/gl_test.c` `case_shadowcmp` (the comment at the case
list: "a depth texture compared by sampler2DShadow"). The fragment shader,
a byte copy in `tools/mali/testdata/fsarith_218_shadowcmp.frag`:

```
#version 300 es
precision mediump float;
uniform mediump sampler2DShadow u_near;
uniform mediump sampler2DShadow u_lin;
out vec4 o_color;
void main() {
    float right = step(32.0, gl_FragCoord.x);
    vec2 b = vec2(floor((gl_FragCoord.x - 32.0 * right) / 8.0), floor(gl_FragCoord.y / 16.0));
    float n = texture(u_near, vec3((b + 0.5) / 4.0, 0.5));
    float l = texture(u_lin, vec3((b + 1.0) / 4.0, 0.5));
    float r = right > 0.5 ? l : n;
    o_color = vec4(r, r, r, 1.0);
}
```

The vertex shader is the ES 3.00 pass-through `in vec2 a_pos; gl_Position =
vec4(a_pos, 0.0, 1.0)` (`.vert`). The case uploads a 4x4
`DEPTH_COMPONENT32F` texture, binds it to units 0 and 1 with two sampler
objects (`GL_COMPARE_REF_TO_TEXTURE`; `GL_LESS` with `GL_NEAREST`, `GL_GEQUAL`
with `GL_LINEAR`) and draws blocks of 8x16 pixels, each sampling one texel's
centre (left half) or corner (right half).

Before this item the sweep's verdict (`tools/cts/log/2026-11-07-t6-step4a`,
`sweep-*.verdicts.txt`) was `REFUSED compile: ... fp16 arithmetic on
gl_FragCoord other than on x and y as a pair, a compare of one lane, or floor
and fract of x and y is not lowered yet` (`MaliError::FragCoordMediump`): the
walk's `LoadInput { FragCoord }` arm had no case for a mediump shader whose
built-in is highp (`prec.frag_coord_highp` and not `prec.highp`). Behind it
stood the `sampler2DShadow` look-up, which no arm of the Mali back end took
(`Op::Tex { shadow: true }` fell to `TexKind` through `es100_tex`).

## 2. The reference

`tools/mali/drm_shim_ref.sh run <mesa-shim> fsarith_218_shadowcmp.vert
fsarith_218_shadowcmp.frag <out>` then `tools/mali/pan_ref_diff.py extract
<out>.trace --draw`. `tools/mali/gl_ref.py` learnt `sampler2DShadow` for it: the
n-th sampler takes unit n with gl_test's 4x4 `DEPTH_COMPONENT32F` texture
under `GL_COMPARE_REF_TO_TEXTURE`, as case 218 sets it: the first `GL_LESS`
and nearest, the second `GL_GEQUAL` and linear (the samplers' words are
`shadowcmp_218.trace`'s, pinned by `src/mali/frame_tex_tests.rs`
`shadow_samplers_are_case_218_s`: Mesa writes the compare function flipped,
`Greater` and `Lequal`). The texture's kind does
not reach the program (st/mesa lowers a shadow sampler bound to a non-depth
texture only for ARB programs, `src/mesa/state_tracker/st_program.c:1239`,
`st_atom_shader.c:170-180`); the **units** do: the two look-ups read the
handles `0x20001800` and `0x20011801` (texture 0 / sampler 0, texture 1 /
sampler 1, `tex_handle`), which two samplers both left at unit 0 would not.

| Region | Words | `work_reg_count` | `preload` | FAU |
|---|---|---|---|---|
| `vs.position` | 19 (the ES 3.00 position store; no varying program, `secondary_offset` null) | 32 | `0x3800000000000000` | the viewport pair, `0x3f000000`, the three `istream` offsets |
| `fs.fragment` | 45 | 32 | `0x3800000000000000` (r59: the pixel position) | below |

The fragment FAU (`u0`..`u7`, two words each): `u0`, `u1` zero; `u2` =
`(3f800000, 00000000)` the `gl_FbWposYTransform` pair of a 64x64 FBO draw
(`(1.0, 0.0)`, UBO 0 words 0-1, the default block's only member); then the
constants in order of first use, `u3` = `(32.0, -0.0)`, `u4` = `(-32.0,
0.125)`, `u5` = `(0.0625, 0.5)`, `u6` = the two texture handles, `u7` =
`(3c003c00, 0)`, the fp16 1.0 pair of the colour's alpha. Mesa's shader-db
line: `45 instrs, 12 registers used, 8 uniforms used`.

The 45 words (the `fs.fragment` text of the `.expect.json`):

```
U16_TO_F32 r0, r59.h0                      ; gl_FragCoord.x, y: the preloaded pixel position
U16_TO_F32 r1, r59^.h1
FADD_IMM.f32 r0, r0^, #0x3F000000          ; + 0.5 (integer-centre caps, nir_lower_wpos_ytransform)
FADD_IMM.f32 r1, r1^, #0x3F000000
FMA.f32 r1, r1^, u2.w0, u2.w1              ; y' = (y + 0.5) * T.x + T.y
FCMP_OR.f32.ge.m1 r2, r0, u3.w0, 0x0       ; c = x >= 32.0 (step's compare)
MOV.i32 r3, u4.w0                          ; -32.0 (two FAU slots in one CSEL: one is moved)
CSEL.u32.eq r3, r2, 0x0, u3.w1, r3^        ; c ? -32.0 : -0.0  (= -(32.0 * right))
FADD.f32 r0, r0^, r3^                      ; x - 32.0 * right
FMA.f32 r0, r0^, u4.w1, 0x0.neg            ; / 8.0 as * 0.125
FROUND.f32.rtn r0, r0^                     ; floor
FMA.f32 r1, r1^, u5.w0, 0x0.neg            ; y' / 16.0 as * 0.0625
FROUND.f32.rtn r1, r1^                     ; floor
FADD.f32 r0.h0, r0^, 0x0.neg               ; f2f16: b.x
FADD.f32 r1.h0, r1^, 0x0.neg               ; f2f16: b.y
MKVEC.v2i16 r0, r0^.h0, r1^.h0             ; vec2 b
FADD_IMM.v2f16 r1, r0, #0x38003800         ; b + 0.5 (fp16, both lanes)
FMA.v2f16 r1, r1^, 0x34000000.h11, 0x0.neg ; * 0.25
F16_TO_F32 r3, r1.h0                       ; the coordinate back to fp32 ...
F16_TO_F32 r1, r1^.h1
MOV.i32 r4, r3^                            ; ... into the staging registers
MOV.i32 r5, r1^
MOV.i32 r6, u5.w1                          ; the reference 0.5 (fp32)
MOV.i32 r8, u6.w0                          ; the handle of texture 0 / sampler 0
MOV.i32 r9, 0x0
TEX_SINGLE.slot0.skip.reserved.16.r.2d.shadow.computed @r1, @r4:r5:r6, [r8^:r9^]
FADD_IMM.v2f16 r0, r0^, #0x3C003C00        ; b + 1.0
FMA.v2f16 r0, r0^, 0x34000000.h11, 0x0.neg ; * 0.25
F16_TO_F32 r3, r0.h0
F16_TO_F32 r0, r0^.h1
MOV.i32 r4, r3^
MOV.i32 r5, r0^
MOV.i32 r6, u5.w1
MOV.i32 r8, u6.w1                          ; the handle of texture 1 / sampler 1
MOV.i32 r9, 0x0
TEX_SINGLE.slot1.skip.reserved.16.r.2d.shadow.computed.wait01 @r0, @r4:r5:r6, [r8^:r9^]
MUX.v2i16.wait0126 r0, r1^.h00, r0^.h00, r2^.h00   ; c ? l : n (16-bit select on the mask's low half)
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
IADD.v2u16.discard r1, r0.h00, 0x0         ; (r, r)
MKVEC.v2i16 r2, r0^.h0, u7.w0.h0           ; (r, 1.0)
MOV.i32 r0, r1^
MOV.i32.wait r1, r2^
BLEND.slot2.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
IADD_IMM.i32 r48, 0x0, #0x0
BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

## 3. What Mesa does

### 3.1 GLSL: where the fp32 ends and the fp16 begins

`lower_precision` (`G/lower_precision.cpp`) decides per rvalue tree.
`handle_precision` (`:295-312`): highp is `CANT_LOWER`, mediump and lowp
`SHOULD_LOWER`, none (a literal) `UNKNOWN`. `pop_stack_entry` (`:215-282`)
combines a child's state into its parent's (`:235-247`: a child that cannot
be lowered makes the parent so) and, when the parent cannot be lowered,
makes the parent's lowerable children roots of their own lowered subtrees
(`:250-279`, `add_lowerable_children`). The measured GLSL IR of the fragment
shader after the pass (`MESA_GLSL=dump`, abridged, `f2fmp` is fp32 to fp16,
`f162f` the way back):

```
(declare (location=19 sys highp ) vec4 gl_FragCoord)
(declare (mediump ) float16_t right)   (declare (mediump ) f16vec2 b)
(declare (temporary highp ) vec2 vec_ctor)
right = f2fmp(b2f(gl_FragCoord.x >= 32.0))
vec_ctor.x = floor((gl_FragCoord.x - f162f(32.0h * right)) / 8.0)
vec_ctor.y = floor(gl_FragCoord.y / 16.0)
b = f2fmp(vec_ctor)
vec_ctor@4 = (((b + 0.5h) / 4.0h).xy, f2fmp(0.5))       ; (mediump ) f16vec3
n = tex float16_t u_near (f162f vec_ctor@4).xy ... (f162f vec_ctor@4).z
l = tex float16_t u_lin  (...)                           ; the same of (b + 1.0h) / 4.0h
if (0.5h < right) conditional_tmp = l else conditional_tmp = n
o_color = f162f(f16vec4(r, r, r, f2fmp(1.0)))
```

So:

* **The built-in's tree is fp32**: `step`'s compare, `b2f`, the
  subtraction, the divisions and both `floor`s stay 32-bit, because a
  `gl_FragCoord` leaf is `CANT_LOWER` and taints every operation up to the
  assignment.
* **A lowerable subtree inside it is still lowered**: `32.0 * right`, both
  operands mediump (a literal takes the other's precision), is computed at
  fp16 and widened back (`f162f`) for the subtraction. This is `:250-279`:
  the product was queued under its `CANT_LOWER` parent and added as a root
  when the parent was popped.
* **The narrowing is at the variable's assignment**, not at the
  constructor: `vec_ctor` is a *highp* temporary (a constructor's precision
  is its arguments', GLSL ES 3.00 4.5.2), so `b = f2fmp(vec_ctor)` converts
  both lanes after both floors. `right` is narrowed at its own assignment.
* **What is mediump is fp16**: `(b + 0.5h) / 4.0h`, the colour vector, the
  `0.5h < right` compare. A `texture()` coordinate and reference are
  converted to fp32 at the call (`f162f`; the coordinate is fp32 on
  Bifrost/Valhall whatever the precision), its result is the sampler's
  precision, mediump, `float16_t`.

### 3.2 NIR: the rules that shape the words

The final NIR of the run (`fsarith_218.stderr`, the fragment shader), with
the rule behind each line that is not a straight translation:

```
16x2 %0 = @load_pixel_coord                 ; nir_lower_frag_coord_to_pixel_coord, B/bifrost_nir.c:904-910
32  %4 = fadd (u2f32 %0.x), 0.5             ; nir_lower_wpos_ytransform (mali-highp-fragcoord.md 2.1)
32  %8 = ffma (fadd (u2f32 %0.y), 0.5), %7.x, %7.y
32  %12 = fge_pan %4, 32.0                  ; step: inot(flt(x, e)) -> fge(x, e)
32  %13 = bcsel_pan %12, -32.0, -0.0        ; see below
32  %14 = fadd %4, %13
32  %17 = ffloor (fmul %14, 0.125)          ; fdiv by a constant -> fmul by its reciprocal (lower_fdiv, folded)
32  %20 = ffloor (fmul %8, 0.0625)
16  %21 = f2f16 %17 ;  16 %22 = f2f16 %20   ; the f2fmp at b's assignment
16x2 %24 = vec2 %21, %22                    ; stays a vector: 16-bit ALU is width 2 (B/bifrost_nir.c:176-179)
16x2 %25 = fadd %24, 0.5h.xx ;  16x2 %27 = fmul %25, 0.25h.xx
32x3 %30 = vec3 (f2f32 %27.x), (f2f32 %27.y), 0.5
16  %32 = (float16)tex %31 (handle 0x20001800), %30, backend_flags=0x30, 2D   ; the comparator in the vec3's z
...
16  %41 = i2i16 %12                         ; pan_nir_lower_bool_to_bitsize: the 32-bit mask for a 16-bit select
16  %42 = bcsel_pan %41, %40, %32
16x4 %46 = vec4 %42, %42, %42, 1.0h  ;  @blend_pan (.., src_type=float16)
```

* `%13`: `32.0h * right` is `fmul(32.0h, f2fmp(b2f32(c)))`; `f2fmp(b2f32(a))`
  is `b2f16(a)` (`N/nir_opt_algebraic.py:1980`); `fmul(b2f(a), #b)` is
  `bcsel(a, b, 0.0)` (`:1695`) because the product's one user is the
  `f162f`, not an `fadd` (the `is_not_only_used_by_fadd` guard that keeps a
  highp shader's `fmul(b2f, k)` for the fma fusion, s4_b2f); the conversion
  of a select of two constants is a select of the converted constants
  (measured; the bcsel is 32-bit with `-32.0`, `-0.0`); `fsub(a, b)` is
  `fadd(a, fneg(b))` (`:2379`) and `fneg` of a select of two constants used
  once is the select of the negated constants (`:1126`).
* `%41`/`%42`: `0.5h < right` is `flt(0.5h, b2f16(c))`; bifrost lowers
  `b2f16` to `bcsel_pan(c, 1.0h, 0.0h)` (`B/bifrost_nir_algebraic.py:128`),
  a compare of a constant against a select of constants is the select of the
  two folded compares (`N/nir_opt_algebraic.py:1022-1039`, the
  `for op in ['ine', ..., 'flt', 'fge', ...]` loop) and `bcsel(a, true,
  false)` is `a` (`:1918`): the select tests `step`'s own mask.
* The texture: `bi_emit_tex_valhall` puts the comparator after the
  coordinate in the staging registers (`sr_count:3`) and sets the
  `.shadow` modifier (`compare_enable`); `.16.r`: a mediump sampler's one
  texel at fp16; `.skip`: no derivative reads the result
  (`bi_analyze_helper_requirements`), the level is still `computed` from the
  quad's derivatives, so the program runs helper threads.

### 3.3 The Valhall words, by emitter

| Words | Emitter (Mesa) | Ours |
|---|---|---|
| `U16_TO_F32` x2, `FADD_IMM.f32` x2, `FMA.f32 .. u2` | the position (mali-highp-fragcoord.md 2.1 step 6) | `Fs::frag_coord` |
| `FCMP_OR.f32.ge.m1` | `fge_pan` | `Fs::lane32`, the `Bnot` arm (`inverse_compare`) |
| `MOV.i32 r3, u4.w0` + `CSEL.u32.eq r3, r2, 0x0, u3.w1, r3^` | `bcsel_pan` of two FAU constants: `va_lower_isel` makes the `CSEL`, `va_repair_fau` moves one of the two FAU slots to a register | `Fs::lane32`'s `Fmul` and `Fneg` arms (section 4), fau.rs `repair_fau` |
| `FADD.f32`, `FMA.f32 .. 0x0.neg`, `FROUND.f32.rtn` | fp32 `fadd`, `fmul`, `ffloor` | `Fs::alu32` through `op32` (the vertex stage's rows) |
| `FADD.f32 rN.h0, rN^, 0x0.neg` x2 | `f2f16` of a scalar (`bi_f32_to_f16_to`) | the `StoreLocal` arm's narrowing |
| `MKVEC.v2i16`, `FADD_IMM.v2f16`, `FMA.v2f16 .. 0x34000000.h11` | the 16x2 `vec2`, `fadd`, `fmul` | `Fs::alu16` (`vec16`, the `Fadd` and `Fmul` arms) |
| `F16_TO_F32` x2, `MOV.i32 r6, u5.w1`, the handle `MOV`s, `TEX_SINGLE .. .shadow` | `f2f32`, the `vec3`'s constant, `bi_emit_tex_valhall` | `tex_highp` (three staging words), `tex` (`TexSingle { shadow }`), pack.rs `compare_enable` |
| `MUX.v2i16 .., r2^.h00` | the 16-bit `bcsel_pan` of `i2i16(mask)` | the walk's `Bcsel` arm, `select_highp`'s `.h00` on a 32-bit mask |
| `IADD.v2u16 .. h00`, `MKVEC.v2i16 .., u7.w0.h0`, `ATEST 0x3F800000`, `BLEND.v4.f16` | the colour `(r, r, r, 1.0h)` | `select_highp`'s colour words (`lower_mkvec_swz` makes the `IADD.v2u16` of a repeated half) |

## 4. What we lower

The shader keeps its mediump route (`FsPrecision { highp: false,
frag_coord_highp: true }`) and the walk builds the island on the graph the
highp machinery already has. All in `nir_fs.rs` unless named:

1. **The read.** `LoadInput { FragCoord }` of a mediump ES 3.00 shader is
   `Fs::frag_coord`: the fp32 lanes of mali-highp-fragcoord.md 2.1 (the
   same nodes a highp shader builds). z and w stay `Kind::FragCoordZw`,
   refused where the colour reads them. The ES 1.00 case keeps
   `Fs::frag_coord16`.
2. **The island.** `Fs::fc32(v)`: `v` is one of those lanes or a 32-bit
   operation reading one, through any depth (`reaches_fc`); only when the
   shader is mediump *and* the built-in highp. An `Alu` with such an operand
   is built by `Fs::alu32` (the highp shader's lanes, NIR's rules), its fp16
   operands widened by `op32` as a highp shader's are. Three rules joined
   `Fs::lane32` for it:
   * `Bnot`/`Inot` of a compare used once is the inverse compare
     (`inverse_compare`): `step`'s `!(x < e)` is `fge`. This arm is not
     gated on the shader's precision: it is what the highp
     es300/layersunused.frag (`tint * step(bias, v_xy.x)`) needed, now
     byte-equal to `fsarith_layersunused.expect.json` (tests.rs
     `es300_layersunused_step_is_byte_equal_to_mesa_26_2_1`; the corpus
     verdict moved from `Arithmetic` to `ok`).
   * in a **mediump** shader, `fmul(#k, b2f(c))` is `bcsel(c, k, 0.0)`
     (the fp16 detour of 3.2: the product is lowered and widened, so NIR's
     rule fires with no fma fusion to stop it). A highp shader's product
     keeps `b2f_products`' rule (bcsel unless only an `fadd` uses it).
   * `fneg` of a select of two constants used once is the select of the
     negated constants (`:1126`).
   * `flt(#a, b2f(c))` with `a` in (0, 1) is `c` (3.2's `%41`): the select
     then tests the 32-bit mask.
3. **The narrowing.** A `StoreLocal` to a declared variable (the front
   end's `VarDecl::name` is not empty) of an island float
   (`Fs::fc32_float`: the lane itself or float arithmetic on it, not a
   compare's mask nor a `b2f`, which have no precision) makes an `F2f16`
   node (`FADD.f32 rN.h0`) at the store. A store to a constructor's
   temporary (an unnamed local) keeps the fp32 lanes, so `vec2 b =
   vec2(floor(..), floor(..))` narrows at `b`, after both floors, as 3.1
   shows and the word order needs.
4. **The fp16 vectors.** An `Alu` whose operands are narrowed island values
   or the vectors made of them (`Fs::narrowed`) and constants goes through
   `Fs::alu16`: `vec16` makes the `vec2` (`MKVEC.v2i16`) of two halves, the
   operation is one node of two lanes. `alu16` learnt a constant in both
   lanes of a two-lane `fadd` (one fp16 half, `FADD_IMM.v2f16 #0x38003800`
   once `fuse_add_imm` runs) and a narrowed operand of a two-lane `fmul`
   (beside the texels it took before; the constant `0x3400` is the
   table's `0x34000000.h11`).
5. **The shadow look-up.** `Op::Tex { dim: D2, shadow: true, lod: Implicit
   }` of a mediump shader whose coordinate lanes are fp16 and whose
   reference is a constant: a `Kind::Tex` node of three sources and one
   lane, `TexLookup { shadow: true, coord: Computed }`. `tex_highp` stages
   three words (the two lanes widened by `F2f32` rows, the constant as a
   pushed FAU word), `tex` sets `TexSingle { shadow }` and pack.rs writes
   `compare_enable` (bir.rs, pack.rs, preload.rs, select.rs: the new field).
   `tex_mask` gives `.r` from the one lane the colour reads.
6. **The select.** `Bcsel` whose condition is an island value and whose
   sides are fp16 values or fp16 texels is a 16-bit `Bcsel` node;
   `select_highp` gives a 32-bit mask the `.h00` swizzle (`i2i16`), the
   mirror of its `i2i32` for a 16-bit mask under a 32-bit select.
7. **The route.** `nir_fs::frag32(f)` (a mediump shader with the highp
   read) sends the program to `select_highp` beside `guarded16` and
   `kept16` (mod.rs `compile_stage`), with the fp16 uniform block
   (`UboLayout::fs16`, which already places the transform pair). No
   `vectorize16`: the island's fp16 nodes are vectors from the walk.

The FAU order (section 2) falls out of the existing constant lowering (by
first use) and `repair_fau` (the `MOV.i32 r3, u4.w0` of a `CSEL` that would
read two FAU slots).

## 5. What stays refused, and what has no reference

| Shape | Verdict | Why |
|---|---|---|
| a shadow look-up whose reference is computed (`vec3(uv, b.x)`) | `TexKind` | the staging word would be a converted lane; no reference (pinned in the test) |
| a highp shader's `sampler2DShadow`, `samplerCubeShadow`, `sampler2DArrayShadow`, `textureProj`/`textureLod` with a shadow sampler | `TexKind` | `es100_tex` takes no shadow form; no reference (the highp one is pinned) |
| gl_FragCoord.z or .w read in a mediump ES 3.00 shader | `FragCoordZw` where read | a highp shader's z is the `frag_z` special varying (mali-fragcoord-mediump.md 5); its narrowing has no reference |
| an island operation `alu16` has no form for (two different constants in a pair, an odd lane count, `fract`/`floor` of the fp16 vector...) | `FragCoordMediump` | no reference; the message names the forms it takes |
| a `bcsel` on the island's mask with sides other than fp16 values or texels | `Arithmetic` | no reference |
| a component assignment to a declared mediump vector, `b.x = floor(..)` | compiles (narrowed at each store, Mesa's `f2fmp` at each assignment) | no reference draws the order; the sweep has no such case |
| an fp32 `b2f` of the island read by fp16 arithmetic other than a product with a constant | `MixedPrecision` or `Arithmetic` | `f2fmp(b2f32)` is `b2f16` in NIR; no reference |

## 6. Checks

* `src/shaderc/online/mali/tests.rs`
  `case_218_shadowcmp_is_byte_equal_to_mesa_26_2_1`: both IDVS programs
  and the fragment program through the link (`check_linked_for`), every
  fact (registers, preload, the FAU table), helper threads on, two
  `.shadow.` look-ups, and the two refusals of section 5's first rows.
  `es300_layersunused_step_is_byte_equal_to_mesa_26_2_1` pins the side
  effect of the `Bnot` arm.
* `tools/mali/corpus-verdicts.txt`: one line moved forward
  (`es300/layersunused.frag: ok`); no other verdict changed. The ES 1.00
  mediump programs that read gl_FragCoord (prec_84, tri_fragcoord,
  fragcoord.frag) keep their words: `fc32` is off when the built-in is
  mediump.
* `tools/mali/sweep_passsets.py --delta tools/cts/log`: exit 0 (no record
  changed here).
* Not run: the board. The sweep's case 218 also needs the frame side it did
  not reach before: the sampler descriptor with the GL sampler object's
  compare mode and function (`src/mali/frame.rs` writes `compare_function:
  Func::NEVER`, "no compare", for every sampler today) and the
  `DEPTH_COMPONENT32F` texture's storage (the GL core keeps depth textures
  as R32F, which the Mali back end stores). Those are `src/mali` (plan
  #336's board-sweep item or a new frame item), not this item's.
