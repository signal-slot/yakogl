<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Valhall mixed highp/mediump fragment code, fp16-inexact colour constants, folded constant varyings

Date 2026-10-25. Plan #153 item `study-mixprec`, for issue #1384. It builds on
`docs/notes/mali-precision.md` and `docs/notes/mali-highp-fragcoord.md` (read
them first; not edited here). Citations are Mesa 26.2.1; `G/` is
`src/compiler/glsl/`, `N/` is `src/compiler/nir/`, `B/` is
`src/panfrost/compiler/bifrost/`. The words quoted are **read from the
references** `tools/mali/testdata/mixprec_*.expect.json` (Mesa 26.2.1's GL
driver on drm-shim, GPU id 0xa867) and from the `.stderr` of the same runs
(NIR and disassembly, `BIFROST_MESA_DEBUG=shaders,shaderdb`), not derived.
Committed: `.vert`, `.frag`, `.expect.json` and the run's `.stderr` (the
NIR and disassembly this note quotes); the `.trace`/`.stdout` are dropped.
Statements about Mesa source below were read from the 26.2.1 tree, not
traced through a debugger; the ones not confirmed by a reference say so.

The three refusals (`tools/mali/corpus-verdicts.txt`, `mod.rs` `MaliError`):

| Program | Refusal | Raised at |
|---|---|---|
| `es300/hdrsample.frag` | `MixedPrecision` | `mod.rs:1149` (a highp output of a mediump shader), `:1194` (`f.mixed`), `nir_fs.rs:3188` |
| `es100/hello.frag` | `Fp16Inexact` | `nir_fs.rs:2460, 2620, 2653, 2788, 3127, 3573`, `nir.rs:1223` |
| `es100/varyings15.frag` | `ConstantFold` | `nir_fs.rs:544, 559`, `nir.rs:1241-1282` |

## Recording

```
W=$CONNECTED_DEPS_DIR/scratch-mixprec   # a copy of the shim install: symlinks to
                                         # mesa-shim/inst/lib64/*.so* plus
                                         # panthor_dri.so -> libgallium-26.2.1.so, dri -> .
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/mixprec_hdrsample.vert    tools/mali/testdata/mixprec_hdrsample.frag    tools/mali/testdata/mixprec_hdrsample    --uniform-values
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/mixprec_hello.vert        tools/mali/testdata/mixprec_hello.frag        tools/mali/testdata/mixprec_hello        --uniform-values
tools/mali/drm_shim_ref.sh run $W tools/mali/testdata/mixprec_varyings15.vert   tools/mali/testdata/mixprec_varyings15.frag   tools/mali/testdata/mixprec_varyings15   --uniform-values
python3 -I tools/mali/pan_ref_diff.py extract tools/mali/testdata/mixprec_<n>.trace --draw > tools/mali/testdata/mixprec_<n>.expect.json
```

`hdrsample` is `#version 300 es`, so `gl_ref.py` selects its ES 3 mode by
itself; its vertex shader is `es300/cube.vert` (what `gl_test.c` links with it:
`v_xy = pos`). `hello.vert` is `es100/hello.vert`, the PowerVR SDK matrix
shader (mediump `transformationMatrix`, as `prec_19`), so the vertex words of
`mixprec_hello` equal `prec_19`'s; the reference of interest is the fragment
shader. `varyings15.vert` is the case's own vertex shader (fifteen constant
varyings). The shim reports Mali-G610 MC4; the 2x2 RGBA8 texture of `hdr` is
`gl_ref.py`'s. Scratch install note: the cached install lacks
`lib64/panthor_dri.so` (see `mali-precision.md`).

## 1. hdrsample: mixed fp32 and fp16 (`mixprec_hdrsample`)

`fragColor = texture(hdr, v_xy * 0.5 + 0.5) * 0.25;` with `precision highp
float` but a `sampler2D` that has no precision qualifier (ES 3.00 default
**lowp**, the NIR says `lowp sampler2D hdr`). Three precision regions:

- coordinate: highp, fp32 (`v_xy * 0.5 + 0.5`);
- the look-up result and the `* 0.25`: lowp, so **fp16**;
- the output `fragColor`: highp, fp32.

Mesa's fragment words (`fs.fragment`, the stderr's final disassembly):

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0
FMA.f32 r0, r0^, 0x38000000.h1, 0x38000000.h1       ; x*0.5+0.5, 0.5 as the high half of 0x38000000
FMA.f32 r1, r1^, 0x38000000.h1, 0x38000000.h1
MOV.i32 r2, u2.w0                                    ; texture descriptor / sampler
MOV.i32 r3, 0x0
TEX_SINGLE.slot1.skip.reserved.16.2d.computed.wait1 @r0:r1, @r0:r1, [r2^:r3^]   ; f16 result (.f16 in the BIR)
FMA.v2f16.discard r0, r0^, 0x34000000.h11, 0x0.neg  ; (r,g) * 0.25: the multiply is FMA.v2f16 ..., -0.0
FMA.v2f16 r1, r1^, 0x34000000.h11, 0x0.neg          ; (b,a) * 0.25
F16_TO_F32 r2, r0.h0
F16_TO_F32 r3, r0^.h1
F16_TO_F32 r4, r1.h0
F16_TO_F32.wait0126 r5, r1^.h1
ATEST.wait0 @r60, r60^, r5, atest_datum.w0           ; alpha (fp32) feeds ATEST
MOV.i32 r0, r2^ ; MOV.i32 r1, r3^ ; MOV.i32 r2, r4^ ; MOV.i32.wait r3, r5^
BLEND.slot2.v4.f32.end @r0:r1:r2:r3, blend_descriptor_0.w0, r60, target:0x0
IADD_IMM.i32 r48, 0x0, #0x0
BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

20 instructions, 9 registers, 3 uniforms. Both 0.5 and 0.25 are
**inline instruction immediates** (`0x38000000.h1`, `0x34000000.h11`: fp16 in
the named half or both halves of a 32-bit immediate), not FAU words.
`expect.json` has the exact words. Observations (all in the NIR of the stderr):

1. The look-up is typed `(float16)tex`: the texture instruction's destination
   is fp16 (`TEX_SINGLE ... .f16`), not an f2f16 after an fp32 result. The
   coordinate (`backend1`) stays fp32.
2. The multiply is `fmul` on `16x2` pairs: `%10 = fmul %8.xy, 0.25.xx`,
   `%11 = fmul %8.zw, ...` (two `FMA.v2f16`, **each a vec2 of halves, never a
   scalar fp16 op**), the constant `0.25` is a `16`-bit `load_const (0x3400)`.
3. **The conversions are only at the boundary** back to fp32 for the highp
   output: four `nir_op_f2f32` on the lanes (`%12..%15`), compiled to four
   `F16_TO_F32` with `.h0`/`.h1` selecting the half of a result register.
   Because `fragColor` is highp the blend is the **fp32** form
   (`BLEND.v4.f32`, `src_type=float32`) fed through `MOV.i32` copies of the four
   converted registers into the contiguous `r0:r3` (a `COLLECT`); the fp32
   alpha `r5` also feeds `ATEST`.
4. The scheduler's register assignment and `.wait` flags are Mesa's (RA and
   `bi_schedule`); our `pressure_schedule` must be checked against them
   byte for byte as for `prec_*`.

### Where Mesa inserts the conversions

Two stages, GLSL IR then NIR:

- **GLSL IR, `lower_precision` (`G/lower_precision.cpp:1355`, called from
  `st_glsl_to_nir`'s link-time preparation when the screen has
  `fp16`/`glsl_16bit_consts`: Panfrost sets `caps->fp16 = caps->glsl_16bit_consts`
  and `fp16_const_buffers = arch >= 6`, `gallium/drivers/panfrost/pan_screen.c:585-588`).**
  `find_lowerable_rvalues_visitor` (`:184`) marks an expression *lowerable*
  when every operand is: `handle_precision` (`:295-312`) says
  mediump/lowp -> `SHOULD_LOWER`, highp -> `CANT_LOWER`, none -> `UNKNOWN`
  (constants and untyped temporaries follow their parent, `visit(ir_constant)`
  `:336`, `visit_leave(ir_assignment)` `:585`). A texture result takes its
  *sampler's* precision (`visit_enter(ir_texture)` `:384`), hence lowp here.
  `lower_precision_visitor::handle_rvalue` (`:724`) then rewrites a lowerable
  tree to 16-bit types: a dereference gets `convert_precision(false, ...)`
  (`:683`, `ir_unop_f2fmp`), a constant is converted in place with
  `_mesa_float_to_half` (`:744`). The **up-conversion** at the edge where a
  lowered value meets a highp consumer (the assignment to `fragColor`) is
  `convert_precision(true, ...)` = `ir_unop_f162f` (`:693-694`).
  `lower_variables_visitor` (`:978-1230`) retypes the variables whose declared
  precision is medium/low (outputs and `in` deref's included) and
  `can_lower_type` (`:153`) refuses non-float types except bool and samplers.
- **NIR.** `ir_unop_f2fmp` becomes `nir_op_f2fmp` (`glsl_to_nir.cpp`), `f162f`
  becomes `f2f32`. `nir_opt_algebraic`'s `late_optimizations`
  (`N/nir_opt_algebraic.py:4164-4199`) then removes a conversion pair around an
  operation (`f2f32(op(f2fmp(a), f2fmp(b))) -> op(a, b)`, for `fadd fmul ffma
  fmax fmin ... feq flt`) because the arithmetic at fp32 is cheaper than fp16
  plus conversions **unless** the operands are already 16-bit values
  (`preserve_mediump` unset on Panfrost), and finally rewrites the remaining
  `*2*mp` into the concrete `f2f16`/`i2i16`... (`:4193-4199`).
  `nir_lower_mediump_io` (`N/nir_lower_mediump.c:37`) exists for mediump
  inputs/outputs; Panfrost's `pan_shader.c:588` passes `false /* lower_mediump */`
  to the shader-cache path, and the fp16 colour output is produced by the
  `blend_pan` source type (below), not by it.
- **Bifrost back end.** `bi_emit_alu` (`B/bifrost_compile.c`): `nir_op_f2f32`
  at `:3073` (`bi_f16_to_f32_to`), `nir_op_f2f16`/`_rtz`/`_rtne` at `:2759-2790`
  (narrowing: `bi_v2f32_to_v2f16`, or `FADD.f32 rN.h0, rN, -0.0` pairs merged with
  `MKVEC.v2i16`; see `mali-highp-fragcoord.md`). The fragment output
  (`store_output`/`blend_pan`) takes `src_type=float16` when the output
  variable is mediump (and not highp-converted), else float32; the
  `BLEND.v4.f16` vs `.v4.f32` and the register count (`sr_count`) follow
  (`mixprec_hello` below is the fp16 form: `BLEND.slot0.v4.f16 @r0:r1`).

### What `src/shaderc/online/mali` must do

Today `f.mixed`/`MixedPrecision` rejects any value of the other width. To match
bytes it needs:

1. a per-value width in the fragment graph (`Val`): lowp/mediump regions
   carry the pair form (`vec2` of halves in one register), highp the fp32 form;
2. the texture look-up with the sampler's declared precision (default lowp)
   emitting `TEX_SINGLE ... .f16` and two result registers of packed halves
   (`TEX_SINGLE.rgba.2d.f16.computed_lod`, `sr_count:2`) - the existing fp16
   texture path (`ctl_tex_mediump.frag`) already does this for an fp16 output;
3. `Fmul` with a constant as `FMA.v2f16 x, c, -0.0` per half-pair (`select.rs`
   has the mediump form) with the constant as a **packed pair** `0x34000000.h11`;
4. the boundary: `F16_TO_F32 rd, rs.h0/.h1` per lane (the `Op::F16ToF32` row of
   `mali-precision.md`, 'implementation order for case 19'), the four results
   `MOV`'d into a contiguous source register block, `BLEND` in fp32 form with
   `sr_count:4` (the existing highp blend), fp32 alpha to `ATEST`;
5. rule: a conversion exists only where a highp consumer reads a lowered
   producer; Mesa's `late_optimizations` removes a conversion pair that wraps
   an operation whose operands are lowered values of the *same* expression, so
   no `F16_TO_F32` is followed by `F32_TO_F16`.

## 2. hello: a colour constant fp16 cannot hold (`mixprec_hello`)

```glsl
void main (void) { gl_FragColor = vec4(1.0, 1.0, 0.66 ,1.0); }
```

`gl_FragColor` is mediump (`gl_FragData[0]` is declared `mediump vec4` in the
NIR: `G/builtin_variables.cpp` sets mediump for the ES 1.00 fragment colour),
so the constant is lowered. NIR (`stderr`):

```
16x4 %4 = load_const (0x3c00, 0x3c00, 0x3948, 0x3c00)
         = (1.000000, 1.000000, 0.660156, 1.000000)
@blend_pan (%2, %3, %4) (io location=FRAG_RESULT_DATA0 slots=1, src_type=float16)
```

and the words (`fs.fragment`):

```
NOP.wait0126
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
MOV.i32.discard r0, u2.w0
MOV.i32.wait r1, u2.w1
BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
IADD_IMM.i32 r48, 0x0, #0x0
BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

FAU words 4-5: `3c003c00 3c003948` (`expect.json` carries them). The colour is
**one rounded fp16 per lane, read as the packed halves of two FAU words**
(lane z = `0x3948`, which is 0.66 rounded to the nearest fp16: 0.66015625;
the error 1.6e-4 is below the 8-bit target's step). Rule and source:

- The constant is converted by `lower_precision_visitor::handle_rvalue`
  (`G/lower_precision.cpp:739-746`) with `_mesa_float_to_half`
  (`src/util/half_float.h:62`; the slow path `src/util/half_float.c:57-120`):
  **round to nearest even**, no exactness test (`new_exp < -14` rounds to a
  subnormal or zero: `_mesa_lroundevenf((1<<24) * |x|)`; `new_exp > 15` ->
  infinity; fp32 denormal -> signed zero; NaN keeps its top mantissa bits, a
  mantissa that shifts to 0 becomes 1). On x86-64 with F16C the hardware
  `vcvtps2ph` is used (round mode nearest-even by the immediate in
  `half_float.h:64-80`), the same result.
- `lower_variables_visitor` (`:1000-1010`) does the same for constant
  initialisers of lowered variables.
- The back end gets a 16-bit `load_const`; the two lanes pairs become the
  32-bit words `3c003c00` (x,y) and `3c003948` (z,w), which sit in the FAU
  (`u2.w0`, `u2.w1`) and are `MOV`ed into the blend source registers (read
  from the words; the packing function itself was not traced).

Our code: `f16_exact` (`select.rs:43`) returns `None` unless the fp32 is exactly
an fp16; `f16_rtne` (`select.rs:78`) rounds to nearest even but also returns
`None` outside `[-14, 15]` exponents. To match Mesa on this input it is enough
that the colour-constant path (`nir_fs.rs:2460` is already
`if round { f16_rtne(bits) } else { f16_exact(bits) }`) uses `f16_rtne` for
the **lowered constants of a mediump fragment shader** and writes the pair as
the FAU words above; 0.66 -> `0x3948` is `f16_rtne`'s result (exponent -1, the
mantissa 0.32 * 1024 = 327.68 -> 328 = `0x148`, plus exponent bits `0x3800` =
`0x3948`). Subnormals and overflow stay refused until a reference exists
(Mesa's rules above; denormals round to a half subnormal, large values to
infinity: not recorded).

The vertex shader of the pair is `prec_19`'s (fp16 matrix) and is out of this
refusal.

## 3. varyings15: folded constants (`mixprec_varyings15`)

The fragment shader checks fifteen `varying vec4` against
constants (`abs(a - b) < 0.004`, `all(lessThan(...))`), `bad` selected with
`if`; the vertex shader writes constants `vec4(1,17,33,49)/64` etc. from no
attribute. Mesa's output is **a constant shader**:

```
NIR (fs): 16x4 %4 = load_const (0x0000, 0x3c00, 0x0000, 0x3c00) = (0, 1, 0, 1)
          @blend_pan (%2, %3, %4) (src_type=float16)
words:    NOP.wait0126
          ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
          MOV.i32.discard r0, 0x3C000000
          MOV.i32.wait r1, 0x3C000000
          BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
          IADD_IMM.i32 r48, 0x0, #0x0
          BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

7 instructions, 4 registers, 0 uniforms: green `(0, 1, 0, 1)` as halves
`0x3C000000` twice (x=0 and y=1.0 in one word, z=0 and w=1.0 in the next).
The vertex shader is `a_pos` only: 24 instructions, **no varying
outputs at all** (the `MESA_SHADER_VARYING` variant is empty, `0 instrs`, and the
NIR shows `outputs: 1`, only `gl_Position`).

So nothing "non-finite" is computed on the GPU: every one of the fifteen
varyings is replaced by its constant before code generation and the whole
fragment program folds, `all(lessThan(abs(c - c), 0.004))` -> true, `bad`
remains 0.0, the `?:` picks the green. The mechanism:

- **Front end, GLSL IR** (`G/`): `do_constant_folding` / `ir_constant`
  evaluation folds `/64.0`, the vector constructors, `abs`, `lessThan`, `all`
  in the vertex shader; no operation reaches NIR. Note `15.0/16.0` etc. are
  computed in fp32 host arithmetic; inf/NaN/denormals follow IEEE (no
  flush) since `nir_opt_constant_folding` and
  `constant_expressions` (`N/nir_constant_expressions.py`) evaluate with the
  shader's `float_controls_execution_mode` (`nir_opt_constant_folding.c:52`);
  there is no case in this program that produces one.
- **Link, `nir_opt_varyings_bulk`** (`G/gl_nir_linker.c:1524`, called by the
  GL linker for every pipeline): a vertex output that is a constant is
  **propagated into the consumer and removed from the interface**
  (`INTERP_MODE` flat/smooth makes no difference for a constant). This is why
  the fragment shader's inputs vanish and why `outputs` in the vertex NIR is
  only the position. After it the fragment shader contains only constants and
  `nir_opt_constant_folding` (`B/bifrost_compile.c:4394`, in the
  optimisation loop) plus `nir_opt_algebraic` finish the fold.
- The result is typed fp16 because `gl_FragColor` is mediump: the `bcsel`
  of `vec4(0,1,0,1)` / `vec4(1,0,bad,1)` folds to the former, whose `lower_precision`
  conversion (`_mesa_float_to_half`) is exact.

Our refusal: `MaliError::ConstantFold` is raised by constant-vs-constant
`Fmul`/`Fadd` in the fragment graph (`nir_fs.rs:544, 559`) and by the vertex
`nir.rs:1241-1282`, because our NIR build does not fold. The vertex shader of
this program also hits the 'varying lane computed from no attribute' refusal
(`corpus-verdicts.txt:128`, `linkconst`/`study-varyconst`, #1384); the two are
the same cut: when Mesa's link moves a constant varying into the fragment
shader, **the fragment compile must be the one of the program with those
varyings substituted**. To match byte for byte:

1. a constant-folding pass over the fragment graph that evaluates `Fadd`,
   `Fmul`, `Fabs`, `Fneg`, comparisons, `all`/`any` and `select` on constants
   with fp32 IEEE semantics (no flush; inf/NaN legal; the results here are all
   finite normal), after substituting the vertex shader's constant output lanes
   (`linkconst` already propagates them in the online backend);
2. a graph whose every fragment output is constant then takes the existing
   constant-colour path (as `mixprec_hello`/`prec_19`'s fragment shader):
   `ATEST`, two `MOV.i32` of the packed halves (here the immediate
   `0x3C000000` inline, since a word that fits the instruction immediate
   is not put in the FAU: `uniforms used 0`), `BLEND.v4.f16`;
3. the vertex program loses its fifteen varying stores (the variant is
   empty, `MESA_SHADER_VARYING 0 instrs`) and the draw's varying descriptors
   follow.

Denormals, infinities and NaN: the reference contains none (no value of this
program leaves the finite normal range), so the rules for them are **only
from the sources**: constant folding in NIR runs the C operation on the host
(`nir_eval_const_opcode`) with denormals preserved unless the shader's
`float_controls_execution_mode` asks for flush; a result that is inf/NaN is
written as such (the 16-bit narrowing of `_mesa_float_to_half` above maps
inf -> `0x7c00`, NaN -> `0x7c00 | (m>>13 or 1)`, a denormal to zero). Until a
reference exists `ConstantFold` should keep refusing a fold whose result or
operand is not finite normal; this program does not need that case.

## Summary of byte rules read from the references

| Rule | Mesa | Reference |
|---|---|---|
| lowp/mediump region -> pairs of halves (`FMA.v2f16 x, c, -0.0`) | `lower_precision.cpp` | hdrsample |
| result of `texture()` is the sampler's precision (default lowp) -> `TEX_SINGLE ... .f16` | `visit_enter(ir_texture)` `:384` | hdrsample |
| fp16 -> fp32 only at the edge of a highp consumer: one `F16_TO_F32 .h0/.h1` per lane, fp32 `BLEND` | `convert_precision(true)`; `bifrost_compile.c:3073` | hdrsample |
| fp16-inexact constant: round to nearest even, pair packed in two FAU words | `lower_precision.cpp:744`, `half_float.c:57` | hello (`3c003c00 3c003948`) |
| constant varyings folded into the fragment shader; constant colour program of 7 instructions | `gl_nir_linker.c:1524` (`nir_opt_varyings_bulk`), `nir_opt_constant_folding` | varyings15 |

Open points (no reference): a lowered constant outside the fp16 normal range
(subnormal, overflow, NaN); mixed regions with a highp *vec3*/*scalar* tail
(pairs of an odd lane count); `if`/`?:` between a lowered and an fp32 value;
a lowered varying (`mediump` `in`, `nir_lower_mediump_io`).
