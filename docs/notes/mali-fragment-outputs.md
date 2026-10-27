<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali: several fragment outputs (case 55) and the fragment operand shapes of cases 91 and 192

Study for plan #153 (the sweep refusals of `0dce654a8:tools/mali/board-ref/2026-10-23-s6-discard/`,
issue #1384). Mesa 26.2.1, `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`; `M/` below
is its `src/`. Every word quoted is from a reference recorded with
`tools/mali/drm_shim_ref.sh` (Mesa's panfrost on drm-shim, GPU id 0xa867):

| prefix in `tools/mali/testdata/` | program | extra `gl_ref.py` options |
|---|---|---|
| `fsout_55_es3mrt` | `es300/mrt2.{vert,frag}` (case 55; see below) | `--mrt 2` |
| `fsout_106_mrt3` | `es300/mrt3.{vert,frag}` (case 106, three outputs) | `--mrt 3` |
| `fsout_91_hdrscale` | `es100/hdrquad.vert` + `hdrscale.frag` (case 91) | |
| `fsout_91_hdrcolor` | `es100/hdrquad.vert` + `hdrcolor.frag` (case 91, compiles today) | `--blend-func ONE,ONE_MINUS_SRC_ALPHA` |
| `fsout_192_uni`, `fsout_192_coord` | `gltest/position_only.vert` + `predlive_fs_{uni,coord}.frag` (case 192) | `--uniform-values` |

Each has `.stdout`, `.stderr` (the NIR and the disassembly of both stages,
BIFROST_MESA_DEBUG=shaders), `.trace` (pandecode: programs, FAU, blend,
framebuffer and render target descriptors) and `.expect.json`
(`pan_ref_diff.py extract --draw`: the program bytes and FAU words).

`gl_ref.py` got `--mrt N` for this: an ES 3.0 context, N RGBA8 colour
attachments, `glDrawBuffers(0..N-1)`. (gl_test case 55 itself draws two
attachments without `glDrawBuffers`; Mesa then writes attachment 0 only, and
the reference wants both targets.) The second half of case 91 (blending into
an RGBA16F target) is not drawn: `gl_ref.py` has RGBA8 targets only.

**Why the first attempt had no references.** `drm_shim_ref.sh run` put the shim
on `LD_PRELOAD` and ran the draw under `stdbuf -o0`. The `stdbuf` of this host
(uutils coreutils, Ubuntu 26.04) *replaces* `LD_PRELOAD` with its own
`libstdbuf.so`, where GNU's appends: the shim never loaded into python and
`eglInitialize` failed (`failed to create dri2 screen`). Found by running
`gl_ref.py` without `stdbuf`, which draws. The script now puts the shim on
`LD_PRELOAD` inside `stdbuf` (`stdbuf -o0 sh -c 'LD_PRELOAD=shim:$LD_PRELOAD exec ...'`);
every other `run` works on such a host with it.

To record again, from the repository root (`W` = the `mesa-shim/build-shim.sh` install):

```sh
export MESA_PAN_TREE=$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1
W=$CONNECTED_DEPS_DIR/mesa-shim; T=tools/mali; D=$T/testdata
$T/drm_shim_ref.sh run $W tools/shaders/es300/mrt2.vert tools/shaders/es300/mrt2.frag $D/fsout_55_es3mrt --mrt 2
$T/drm_shim_ref.sh run $W tools/shaders/es300/mrt3.vert tools/shaders/es300/mrt3.frag $D/fsout_106_mrt3 --mrt 3
$T/drm_shim_ref.sh run $W tools/shaders/es100/hdrquad.vert tools/shaders/es100/hdrscale.frag $D/fsout_91_hdrscale
$T/drm_shim_ref.sh run $W tools/shaders/es100/hdrquad.vert tools/shaders/es100/hdrcolor.frag $D/fsout_91_hdrcolor --blend-func ONE,ONE_MINUS_SRC_ALPHA
$T/drm_shim_ref.sh run $W tools/shaders/gltest/position_only.vert tools/shaders/gltest/predlive_fs_uni.frag $D/fsout_192_uni --uniform-values
$T/drm_shim_ref.sh run $W tools/shaders/gltest/position_only.vert tools/shaders/gltest/predlive_fs_coord.frag $D/fsout_192_coord --uniform-values
$T/pan_ref_diff.py extract $D/fsout_<name>.trace --draw > $D/fsout_<name>.expect.json    # each
```

## Case 55 `es3mrt`: two outputs

**Shader.** `tools/shaders/es300/mrt2.frag` (= `gl_test_mrt_fs_src`): highp,
`layout(location = 0) out vec4 o_first; layout(location = 1) out vec4 o_second;`
`o_first = u_clear.first; o_second = u_clear.second;`. The `fsout_55_es3mrt`
reference was recorded from the shader case 55 linked before (#1509): the
same shape (a struct of two vec4 uniforms, two outputs), other names
(`ubuf.clearColor0/1`, `fragOutput0/1`). Record it again with the command
above when the shim is at hand. Case 106
(`es300/mrt3.frag`, three constant outputs) has the same refusal.

**Refusal site.** `src/shaderc/online/mali/nir_fs.rs:1882-1884` in `layout()`:
`m.outputs.iter().any(|o| o.kind != IoKind::Colour || o.location != 0)` returns
`MaliError::FragOutput` (text `mod.rs:422`; the vertex-side twin is `nir.rs:1424`).
The frame is not reached.

**Mesa's NIR** (`fsout_55_es3mrt.stderr:201-216`):

```
%0 = load_cumulative_coverage_pan
%2 = load_ubo(0, 0) vec4                      ; clearColor0
%4 = atest_pan(%0, %2.w)                      ; ONE atest, on output 0's alpha
%5 = load_blend_descriptor_pan (base=0)
     blend_pan(%4, %5, %2)   FRAG_RESULT_DATA0
%6 = load_blend_descriptor_pan (base=1)
%8 = load_ubo(0, 16) vec4                     ; clearColor1
     blend_pan(%4, %6, %8)   FRAG_RESULT_DATA1   ; the same %4
```

`pan_nir_lower_fs_outputs.c` (`M/panfrost/compiler/`) makes this: one `atest_pan`
(on the alpha of colour 0: "alpha values refer only to the alpha component of
the output linked to colour number zero"), then per written slot `i` a
`load_blend_descriptor_pan(base = i)` and `blend_pan(coverage, desc, colour,
location = FRAG_RESULT_DATA0 + i)`. `outputs_written: 4-5`. There is no
`ST_TILE` on Valhall: every render target ends in one `BLEND`, blended or not.

**The words to match** (`fsout_55_es3mrt.expect.json`, "fs.fragment"):

```
MOV.i32 r0, atest-param
ATEST   r60, r60, u3[1], r0                     ; u3[1] = clearColor0.w
MOV.i32 r2, u3 ; MOV.i32 r3, u3[1] ; MOV.i32 r0, u2 ; MOV.i32 r1, u2[1]
BLEND.f32 r0:r1:r2:r3, r60, blend_descriptor_0, blend_descriptor_0[1], _.h00, sr_count:4, sr_count_2:2   ; target:0x2
IADD_IMM.i32 r48, program_counter.w0, #0x8 ; BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
MOV.i32 r2, u5 ; MOV.i32 r3, u5[1] ; MOV.i32 r0, u4 ; MOV.i32 r1, u4[1]
BLEND.f32 r0:r1:r2:r3, r60, blend_descriptor_1, blend_descriptor_1[1], _.h00, sr_count:4, sr_count_2:2   ; .end, target:0x0
IADD_IMM.i32 r48, 0x0, #0x0 ; BRANCHZI.eq.absolute 0x0, blend_descriptor_1.w1
```

- Both `BLEND`s read the colour in **r0-r3** (it is copied again between them) and
  the one coverage word `r60` that `ATEST` wrote. Only the alpha comes straight
  from the uniform (`u3[1]`); the colour is `MOV`ed into place.
- Each `BLEND` is followed by an `IADD_IMM r48 / BRANCHZI` stub through the
  descriptor's second word (the blend-shader return); the last one ends the
  program. This is the single-RT `BLEND` of our code, repeated per target.
- FAU: `blend_descriptor_<rt>` is a 64-bit FAU slot, `BIR_FAU_BLEND_0 + rt`
  (`bifrost_compile.c:1626-1631`), after the uniform words; the uniforms are the
  8 words `u2..u5` (two vec4s).

**What the frame must build for an MRT framebuffer** (`fsout_55_es3mrt.trace`):

- **Blend array**: `Blend RT 0` and `Blend RT 1` (lines 533-598), consecutive 16-byte
  descriptors, identical but for `Fixed-Function: RT: <i>` (`Enable: true`, the
  src/zero opaque equation, `Color Mask: 15`, `Num Comps: 4`, RGBA8 conversion). Our
  `frame.rs:2847` hard-codes the `BLEND_DESC` pointer's count as 1
  (`tagged(blend.gpu, 1, ...)`); the tag is the count.
- **DCD1** `Render Target Mask: 0x3` (line 664; `0x7` for three, `fsout_106`); ours is
  `fs.meta.colour_outputs & 1` (`frame.rs:2942`).
- **Framebuffer descriptor**: `Render Target Count: 2` (line 709),
  `Color Buffer Allocation: 8192` (2 x 4096: 1024 pixels x 4 bytes per target; 12288
  for three), `Effective Tile Size: 1024` (unchanged by a second RGBA8 target).
- **Render target descriptors** (`Color Render Targets`, line 856): one per target in
  order; `Internal Buffer Offset` 0 for target 0, **4096** for target 1 (8192 for
  target 2), i.e. the previous offsets plus tile pixels x bytes per pixel. Each has
  its own writeback address/stride and clear colour (Mesa's are AFBC; ours are
  the single-RT frame's format, so only the offsets, addresses and clears are
  taken from this).
- Nothing else changes: one tiler context, the same jobs.

**Implementation order** (cases 55, 106):
1. Selector: `layout()` accepts `IoKind::Colour` at locations 0..7; the fragment
   program emits the `ATEST` on output 0's alpha and, per output, the colour copy
   to r0-r3 and one `BLEND` as above. Constant outputs (106) first: its `ATEST`
   takes the literal `0x3F800000` and the colours are `MOV`s of constants
   (`fsout_106_mrt3.trace`).
2. FAU: `blend_descriptor_<rt>` entries after the uniform words, one per output.
3. Frame: `BLEND_DESC` with `n` descriptors (`RT: i`), the mask, the RT count, the
   colour buffer allocation and the per-target offsets and RT descriptors
   (`Target` becomes up to 8); host tests against both traces.
4. Board: cases 55 and 106 x2 (a board item, not this study).

## Case 91 `hdrblend`: `texture2D(hdr, v_uv) * 0.25`

**Shader that refuses.** `tools/shaders/es100/hdrscale.frag`
(`corpus-verdicts.txt:43`; `hdrcolor.frag` is `ok`):

```glsl
precision mediump float;
uniform sampler2D hdr;
varying vec2 v_uv;
void main() { gl_FragColor = texture2D(hdr, v_uv) * 0.25; }
```

**Refusal site.** `src/shaderc/online/mali/nir_fs.rs:2298-2300` in `select()`:
`f.prods & !ADMITTED != 0` returns `MaliError::FsOperandForm` (`mod.rs:416`).
Reading `ADMITTED` (`nir_fs.rs:263-`), the refused production is a `Tex` result
as a product operand: its admitted places are the output and the two sum
places only. (From the source; our compiler was not run on it.)

**Mesa's NIR and words** (`fsout_91_hdrscale.stderr:294-364`):

```
%4 = (float16)tex ... 2D                       ; 16x4
%5 = load_const 0.25 (0x3400)
%6 = fmul %4.xy, %5.xx ; %7 = fmul %4.zw, %5.xx          ; 16x2 each
%10 = atest_pan(coverage, %7.y)
blend_pan(%10, desc, vec4 %6.x, %6.y, %7.x, %7.y)        ; src_type float16
```
```
r0:r1 = LD_VAR_BUF_IMM.f32.v2.f32.f32.center.store r61, index:0
r2 = MOV.i32 u2 ; r3 = MOV.i32 u256
r0:r1 = TEX_SINGLE.skip.rgba.2d.f16.computed_lod r0:r1, r2, r3, sr_count:2
FMA.v2f16 r0, r0^, 0x34000000.h11, 0x0.neg
FMA.v2f16 r1, r1^, 0x34000000.h11, 0x0.neg
ATEST r60, r60^, r1^.h1, atest_datum
BLEND.f16 r0:r1, r60, blend_descriptor_0, ..., sr_count:2, sr_count_2:2
```

**The rule in Mesa.** None: `fmul(tex, 0.25)` matches no algebraic rule (the
special constants are `-1.0` and `2.0`, `nir_opt_algebraic.py:296,313-315`). A
product is `FMA(a, b, -0.0)` on Valhall; in the mediump pair form it is
`FMA.v2f16 t, c, -0.0`, with the constant the *inline* `0x34000000` (0.25 in the high
half, `.h11` for both lanes). The texel words are the `TEX_SINGLE` destination
and are the FMA's **first** source in place (no copy): the two FMAs write r0 and r1
over it, `BLEND` takes `r0:r1` as they are (no `MKVEC`/`MOV`), and `ATEST` reads
`r1.h1`, the second FMA's high half.

**Matching.** Admit `prod(Tex, MulA)` in `ADMITTED` and let `select()` place a `Tex`
leaf as the FMA's A source, writing over its own register; the constant is the
inline half pair the `Const x Half` rows already draw. Then `hdrscale.frag` moves
to `ok` in `corpus-verdicts.txt`. The femtovg shaders that multiply a texel by a
colour (`fill_image*`, `filter_image*`) share the production and may need more.

**Implementation order**: (1) `ADMITTED` + the operand code; (2) a host test of the
program against `fsout_91_hdrscale.expect.json` (bytes and FAU); (3) the RGBA16F
target and blend of the case's second half are the frame's and need their own
reference (`gl_ref.py` has RGBA8 only; a float target's `blend.float_equation`
is `pan_cmdstream.c:392-394`).

## Case 192 `predlive`: `a = u_q * 3.0; ... r = a * 2.0`

**Shader that refuses.** `tools/shaders/gltest/predlive_fs_uni.frag`
(`corpus-verdicts.txt:250`; its `_coord` twin refuses with another text,
`FragCoordMediump`, `:249`, the mediump-`gl_FragCoord` item; words in `fsout_192_coord`):

```glsl
precision highp float;
uniform float u_x, u_q, u_a, u_b;
void main() {
  float a = u_q * 3.0;   float b = u_a + 0.125;
  float r, s;
  if (u_x > 0.0) { r = a * 2.0; s = b - a; } else { r = b * 0.5; s = a + b; }
  gl_FragColor = vec4(r, s, a, b);
}
```

**Refusal site.** `src/shaderc/online/mali/nir.rs:1155-1160`: the `Alu::Fmul if
self.user && consts[..2]... && (self.is_const_product(a) || self.is_const_product(b))`
arm (`is_const_product`, `:1212`) returns `MaliError::ArithConstant` (`mod.rs:359`);
its comment says Mesa folds the two constants "in an order not transcribed".
The sibling arms (`:1128-1130` fneg of fneg in a product, `:1141`, `:1167-1170`
ffma by +-1.0) do not apply here.

**Mesa's NIR** (`fsout_192_uni.stderr:236-262`):

```
%3  = fmul u_q, 3.0                ; a
%5  = fadd u_a, 0.125              ; b
%7  = fmul 6.0, u_q                ; then-arm r = a*2.0, FOLDED: 6.0 * u_q
%8  = fneg %3
%10 = fmul %5, 0.5                 ; else-arm r
%11 = flt_pan 0.0, u_x
%12 = bcsel_pan %11, %8, %3        ; s = b + (c ? -a : a)
%13 = fadd %5, %12
%14 = bcsel_pan %11, %7, %10       ; r
```

Mesa **does** fold `2.0 * (u_q * 3.0)` to `6.0 * u_q`, although `a` has other uses.

**The rules in Mesa** (`M/compiler/nir/nir_opt_algebraic.py`):
- The fold is the constant-reassociation rule at **line 2406**,
  `(('~fmul', '#a', ('fmul', b, '#c')), ('fmul', ('fmul', a, c), b))`: the `~` marks the
  contractable (not `exact`) form, the match is commutative, so
  `fmul(fmul(u_q, 3.0), 2.0)` gives `#a = 2.0`, `b = u_q`, `#c = 3.0`. It has **no
  `is_used_once`**, which is why `a` staying alive for `gl_FragColor.z` does not stop
  it. `fmul(a, c)` is constant-folded to 6.0 and is the **first** operand of the
  new product. The neighbour at 2394 (`is_used_once` inner product of two
  non-constants, one constant outside) does not match.
- `s = b - a` and `s = a + b` in the two arms become `b + bcsel(c, -a, a)`: the
  `bcsel(c, fadd(b, x), fadd(b, y)) -> fadd(b, bcsel(c, x, y))` rules of the `fadd`
  loop near `nir_opt_algebraic.py:4139-4142` (one arm `is_used_once`), after
  `b - a` is `fadd(b, fneg(a))`.
- No `ffma` is made: each product has several uses and the sums take the
  `bcsel` result.

**Mesa's words** (`fsout_192_uni.expect.json`, "fs.fragment"; `.h0` = low half of a register):

```
MOV.i32.discard r0, u2.w1                      ; u_q into a register
FMA.f32 r0, u4.w0, r0^, 0x0.neg               ; 6.0 * u_q   (u4.w0 = 6.0): the folded r
FADD_IMM.f32 r1, u3.w0, #0x3E000000            ; b = u_a + 0.125
FMA.f32 r2, r1, 0x38000000.h1, 0x0.neg         ; b * 0.5 (inline half constant)
FCMP_OR.f32.lt.m1 r3, 0x0, u2.w0, 0x0          ; u_x > 0.0
CSEL.u32.eq r0, r3, 0x0, r2^, r0^              ; r = c ? 6*u_q : 0.5*b
MOV.i32 r2, u3.w1 ; FMA.f32 r2, u2.w1, r2^, 0x0.neg      ; a = u_q * 3.0 (3.0 = u3.w1 into a register)
FADD.f32 r4, r2.neg, 0x0.neg ; CSEL.u32.eq r3, r3^, 0x0, r2, r4^ ; FADD.f32 r3, r1, r3^   ; s
FADD.f32 r3.h0 / r0.h0 / r1.h0 / r2.h0, x, 0x0.neg      ; f32 to f16 narrowing of s, r, b, a
MKVEC.v2i16 r0, r0^.h0, r3^.h0 ; MKVEC.v2i16 r2, r2^.h0, r1.h0
ATEST r60, r60^, r1^.h0 ; MOV.i32 r1, r2^ ; BLEND.f16 r0:r1, ... (+ return stub)
```

FAU: `u2 = (u_x, u_q)`, `u3 = (u_a, 3.0)`, `u4 = (6.0, 0)`; `u_b` is unused.
Operand placement is the one the existing rows draw: the first NIR source of a
product stays in the FAU and the second goes through a `MOV` into a register
(a constant and a uniform are two FAU slots; one per instruction):
`fmul(u_q, 3.0)` is `FMA u2.w1, r(3.0)`, `fmul(6.0, u_q)` is `FMA u4.w0, r(u_q)`.

**How `src/shaderc/online/mali` matches it.** Do not refuse; apply Mesa's rule in
the NIR reader (`nir.rs`, at this arm): for `Fmul` with a constant `#a` and the
other operand a `Fmul` of a value `b` and a constant `#c`, build `Fmul(Const(a*c), b)`
with the f32 product (round to nearest, as NIR's constant folding does), the
constant *first*, and keep the inner product for its other uses. Mesa's condition
is only the non-`exact` form (every shader here) and no use count. Leave the
`fneg(fneg)` and `ffma(+-1.0)` arms refusing. The `bcsel` hoist is a second rule;
check it with the same reference.

**Implementation order**: (1) replace the `ArithConstant` return of this arm by the
fold; (2) a host test: `predlive_fs_uni.frag` against `fsout_192_uni.expect.json`
(words and FAU); (3) the `fadd`/`bcsel` hoist if the byte comparison stops there;
(4) `_coord` is the mediump `gl_FragCoord` item (`fsout_192_coord`:
`U16_TO_F32 r59.h00`, `FCMP_OR.v2f16.lt.m1` against `u4`, `S16_TO_S32`); (5) case
192 on the board is a sweep case.

## Summary

| case | refusal site | what the reference shows | first step |
|---|---|---|---|
| 55 | `nir_fs.rs:1882-1884` (`FragOutput`); frame `frame.rs:2847`, `:2942` | one `ATEST` on output 0's alpha, per RT a colour copy to r0-r3 and a `BLEND` on `blend_descriptor_<rt>`; two blend descriptors (`RT: i`), mask 0x3, RT count 2, buffer offsets 0/4096 | accept locations 0..7, constant outputs (106) first |
| 91 | `nir_fs.rs:2298-2300` (`FsOperandForm`), (Tex, product) | `FMA.v2f16 r0, r0, 0x34000000.h11, -0.0` per texel pair, in place | admit `prod(Tex, MulA)` |
| 192 | `nir.rs:1155-1160` (`ArithConstant`), `is_const_product` | Mesa folds `2.0 * (u_q * 3.0)` to `6.0 * u_q` (`nir_opt_algebraic.py:2406`, no use-count condition) | do the fold instead of refusing |
