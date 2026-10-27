<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Varyings computed from no attribute, and gl_FragColor lane shapes (Mali)

Study of nine cases of the 2026-10-23 sweep
(`0dce654a8:tools/mali/board-ref/2026-10-23-s6-discard/ours/03-panthor-gltest--sweep-1.log`)
that the online Mali compiler refuses:

| refusal | cases |
|---|---|
| `Valhall: a varying lane computed from no attribute (...) is not lowered yet` | 63 matattr, 78 matvary, 101 cubebias, 117 uniidxf, 181 fusemad, 191 varyfma, 199 es3trunc |
| `Valhall: a gl_FragColor lane other than the varying's own lane or a constant is not lowered yet` | 75 immneg, 185 kindcost |

Nothing here changes `src/`; this is Mesa 26.2.1's behaviour, recorded, and an
order to implement it in. The words quoted are the shim's
(`BIFROST_MESA_DEBUG=shaders,shaderdb`: NIR, then the final disassembly).

## How the references were made

```
# MESA_PAN_TREE=$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1; the drm-shim install is
# $CONNECTED_DEPS_DIR/mesa-shim (tools/mali/README-pan_shc.md; build-shim.sh there)
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/mali/testdata/varyconst_<n>.vert tools/mali/testdata/varyconst_<n>.frag \
    <out>/varyconst_<n> --uniform-values
python3 tools/mali/pan_ref_diff.py extract <out>/varyconst_<n>.trace --draw \
    > tools/mali/testdata/varyconst_<n>.expect.json
```

`<n>` is one of `63_matattr 75_immneg 78_matvary 101_cubebias 117_uniidxf
181_fusemad 185_kindcost_fmul 185_kindcost_maxneg 191_varyfma 199_es3trunc`.
Each `.vert`/`.frag` is a copy of the case's shaders (below). The
`.expect.json` holds the regions the shim draws: `vs.position`,
`vs.varying`, `fs.fragment` (a stage Mesa leaves with no varying has no
`vs.varying`; 75 and 185 draw none). The descriptors below were read from the
same `.trace` (the `Fragment resources resource table`, entry 1).

Host caveat: on this host `stdbuf -o0` in `drm_shim_ref.sh run` drops the
`LD_PRELOAD` of the shim (`eglInitialize failed`); the references were made
with a pass-through `stdbuf` first on `PATH`. The script is unchanged.
`gl_ref.py` parses ES 1.00 only: for 199 (`#version 300 es`, `in vec4 a_pos`) it
binds no attribute ("attributes (none)"), which does not change the programs.
For 117, `uniidx_d.frag` has an `int` uniform `--uniform-values` cannot set, so
the reference pairs `uniidx_f.vert` with `es100/matattr.frag` (same `v_color`
varying); the vertex words are what the case needs.

The shaders per case (gl_test.c in `driver/tools/pvrlab`, sources in
`tools/shaders`):

| case | vertex | fragment |
|---|---|---|
| 63 | `es100/matattr2.vert` (the first the case compiles; `matattr3/4.vert` are refused earlier by `Valhall: three or more additions ... do_rebalance_tree`) | `es100/matattr.frag` |
| 75 | `es100/immneg.vert` (ok) | `es100/immneg.frag` |
| 78 | `es100/matvary.vert` | `es100/matvary.frag` |
| 101 | `es100/cubebias.vert` | `es100/cubebias.frag` |
| 117 | `es100/uniidx_f.vert` | `es100/uniidx_d.frag` (reference: `matattr.frag`) |
| 181 | `gltest/fma_vs_fused.vert` | `gltest/fma_fs_varying.frag` |
| 185 | `gltest/position_only2.vert` | `gltest/kc_{fadd,fmul,maxmul,maxneg}_fs.frag` (reference: fmul, maxneg; `kc_mulneg` and `dep1` are other refusals) |
| 191 | `gltest/varyfma_vs.vert` | `gltest/varyfma_fs.frag` |
| 199 | `gltest/es3trunc_vs.vert` | `gltest/es3trunc_fs.frag` first (an arithmetic refusal), then `es3trunc_vsc.vert` + `es3trunc_vary_fs.frag`; reference: `es3trunc_vs.vert` + `es3trunc_vary_fs.frag` |

## What Mesa's link does (the part the seven cases share)

The vertex shader runs as two programs on Valhall IDVS, a *position* shader
(`gl_Position`, 4 `STORE ... istream`) and a *varying* shader (the other
outputs, `STORE ... estream`). What the varying shader stores is decided by the
link with the fragment shader (`nir_opt_varyings`), before either is compiled:

1. **A constant lane is not a varying.** The consumer's read is replaced by
   the constant and the producer's store is deleted; the remaining lanes
   are compacted into a smaller varying.
2. **A lane the fragment shader does not read is not stored.**
3. **A lane computed from uniforms alone** (no attribute behind it) *stays* a
   varying, but is promoted to `INTERP_MODE_FLAT`; the vertex shader's store
   is `ffma(x, 0.0, x)` (`exact, preserve:inf,nan`) of the value. That is
   what the refusal says Mesa "moves or rewrites": here it rewrites, it does
   not move.
4. A lane computed from an attribute (63's `a_xf[0][0]`, 101's `-a_pos.y`)
   stays smooth, stored as is (`mov`, or `FADD.f32 x.neg, -0.0` for a
   negation).

### Cases 63, 78, 101: a constant lane is dropped

63 `v_color = vec4(a_xf[0][0], a_xf[1][0], 0.5, 1.0)`:

```
decl_var shader_out ... mediump vec2 v_color (VARYING_SLOT_VAR0.xy, 1, 0)   // was vec4
decl_var shader_in INTERP_MODE_SMOOTH none mediump vec2 VARYING_SLOT_VAR0   // fragment
varying shader: 2 stores, lane 0 = a_xf[0].x (GENERIC1) at offset 0, lane 1 = a_xf[1].x (GENERIC2) at 4;
                the loads are issued in the opposite order (`attribute_index:2` first)
fragment NIR:  %9 = vec4 %3.x, %3.y, %8 (0x3800 = 0.5), %5 (0x3c00 = 1.0)
```

The fragment words: `LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store.wait0 @r0:r1, r61^, index:0x0`,
`V2F32_TO_V2F16 r0, r0^, r1^`, `ATEST`, `MOV.i32 r1, u2.w0`, `BLEND.slot1.v4.f16.end`.
The blue/alpha word is the FAU word `u2` (`0x3C003800` in the trace's
fragment FAU: 0.5 and 1.0 as f16), not a varying. Position shader: 24 instructions; varying shader: 8.

Descriptors (fragment resource table, entry 1): one Attribute, `Vertex packet`,
`RG32F RGB1`, `Table 61`, `Offset 1024`, attribute stride 8, packet stride 24
(`Stride 0x180008`); entry 0 is a Null descriptor and a Buffer of 0x10 bytes.
`Varying allocation: 8`.

78 `v_m = mat2(a_color.xyzw); v_after = vec4(0.5)`, the fragment reads
`v_m[0].x, v_m[1].y, v_after.z`:

```
decl_var shader_out ... mediump vec4 packed:v_m[0],v_m[1] (VARYING_SLOT_VAR0.xyzw, 1, 0)
outputs_written: 0,32                        // v_after is gone
fragment NIR:  %10 = vec4 %3, %4, %9 (0x3800 = 0.5), %6 (0x3c00)   // %3 = f2f16 %2.x, %4 = f2f16 %2.w
```

The mat2 is one `vec4` slot (the columns `packed`), `v_after`'s `0.5` is a
constant of the fragment shader, not slot 1. The varying shader stores all four
lanes of `a_color` (`LD_ATTR_IMM.v4`, four `STORE`s, `Varying allocation: 16`),
the fragment loads `v4` and uses lanes 0 and 3:
`FADD.f32.discard r0.h0, r0^, 0x0.neg` / `FADD.f32 r1.h0, r3^, 0x0.neg` (the
f32 -> f16 conversion is an add of `-0.0` writing the low half),
`MKVEC.v2i16 r0, r0^.h0, r1^.h0`, `MOV.i32 r1, u2.w0` (the `0.5, 1.0` word),
`BLEND`. Descriptor: `RGBA32F RGBA`, attribute stride 16, packet stride 32.

101 `v_dir = vec3(1.0, -a_pos.y, -a_pos.x)`: the varying is `vec2`
(`-a_pos.y, -a_pos.x`); the fragment's `1.0` is the first source of
`CUBEFACE1 r2, 0x3F800000, r0, r1`. Varying shader: `LD_ATTR_IMM.v4`,
`FADD.f32 r1, r1^.neg, 0x0.neg`, `FADD.f32 r0, r0^.neg, 0x0.neg`, two stores
(9 instructions). Descriptor `RG32F RGB1`, stride 8/24. The fragment is a cube
look-up with a bias (`TEX_SINGLE ... cube.computed_bias`): its refusal
(`a texture look-up other than texture2D`, tools/mali/corpus-verdicts.txt) is
the next one for case 101, and belongs to the cube items.

### Cases 117, 181, 191, 199: a uniform-only lane is a flat varying

117 `v_color = u_k` (mediump vec4). Varying shader (18 instructions, 8
uniforms: the FAU holds the viewport, the four attribute-less `u_k` words and
the `STORE` offsets):

```
%2 = ffma %1.x, 0.0, %1.x // exact, preserve:inf,nan      (and .y .z .w)
FMA.f32 r2, u5.w1, 0x0, u5.w1      STORE.i32.estream.slot1 @r2, [r0:r1], offset:0
```

Four lanes, four `FMA`s, four `STORE`s at offsets 0, 4, 8, 12 (`MOV.i32 r2,
u4.w0; MOV.i32 r3, 0x0; SHADDX.s64`). Fragment: `decl_var shader_in
INTERP_MODE_FLAT`, `@load_var_buf_flat_pan`, words
`LD_VAR_BUF_IMM.f32.slot0.v4.src_flat32.center.store.wait0 @r0:r1:r2:r3, 0x0, index:0x0`
(the address operand is `0x0`, not `r61`), two `V2F32_TO_V2F16`, `ATEST`
with `r1.h1`, `BLEND`. Descriptor: `RGBA32UI RGBA` (flat is an integer
format), stride 16/32, `Table 61`, `Offset 1024`.

181 `v = vec4(u_x - 2.0*u_q, u_x - u_a*u_b, 2.0*u_q + u_b, 1.0)`: the three
computed lanes are folded in the **vertex** shader (`ffma(-2.0, u_q, u_x)`,
`ffma(-u_a, u_b, u_x)`, `ffma(2.0, u_q, u_b)`), each then wrapped in
`ffma(x, 0.0, x)`; the `1.0` is a fragment constant (item 1), so the varying
is `mediump vec3`, flat. Words (varying shader, 18 instrs):
`MOV.i32 r0, u5.w1; FMA.f32 r1, 0x40000000.neg, u6.w0, r0; MOV.i32 r2, u6.w1;
FMA.f32 r0, u7.w0.neg, r2, r0^; FMA.f32 r2, 0x40000000, u6.w0, r2^;
FMA.f32 r1, r1^, 0x0, r1^; FMA.f32 r0, r0^, 0x0, r0^; FMA.f32 r2, r2^, 0x0, r2^`
then three stores. Fragment: `LD_VAR_BUF_IMM.f32.slot0.v3.src_flat32...`,
`V2F32_TO_V2F16 r0, r0^, r1^`, `FADD.f32 r1.h0, r2^, 0x0.neg`,
`MKVEC.v2i16 r1, r1^.h0, u2.w0.h0` (the `1.0`), `BLEND`. Descriptor
`RGB32UI RGB1`, stride 12/28, `Varying allocation: 12`.

191 `v = vec4(u_x, u_q, u_a, u_b)`, the fragment (`varyfma_fs.frag`) reads
`v.x, v.y`: the varying is `highp vec2` flat (lanes z, w are gone), two
`FMA`+`STORE`s (8 instructions), descriptor `RG32UI RGB1`, stride 8/24. The
fragment's arithmetic runs on the loaded words: `LD_VAR_BUF_IMM...v2.src_flat32`,
`FMA.f32.discard r2, r1, 0x38000000.h1, 0x34000000.h1`,
`FMA.f32 r1, r0.neg, r1^, 0x3F800000`, `FMA.f32 r0, r0^, 0x40000000,
0x3F800000.neg`, three `FADD.f32 rN.h0, rN^, 0x0.neg`, `MKVEC.v2i16` x2,
`BLEND` (13 instructions). (`0x38000000.h1` is the FAU constant 0.5 as a half
in the high half of a word: `.h1` selects the half.)

199 (ES 3.00, highp `out vec4 v`, `v = vec4(u_x, u_q, u_a, u_b)`): like 117 at
fp32. Fragment: `blend_pan ... src_type=float32`, `LD_VAR_BUF_IMM...v4.src_flat32`,
`ATEST ... r3`, `BLEND.slot1.v4.f32.end @r0:r1:r2:r3` (5 instructions, no
conversion). Descriptor `RGBA32UI RGBA`, stride 16/32, `Varying allocation: 16`.

Rule (what the compiler must do when a stored lane has no attribute behind it):

* `Val::Const` lane: delete it from the stored set, and compact the
  varying (items 63, 78, 101, 181); the fragment reads the constant (a
  `vec4` of lanes with `Val::Const` in `colour`, already supported).
* A lane with no attribute (uniform-only, 117/181/191/199): flat, the store
  is `ffma(x, 0.0, x)`, i.e. one `FMA.f32 rN, <x>, 0x0, <x>` per lane, with
  the lane's own arithmetic folded first (181).

The stores keep the varying's lane order (63: lane 0 then lane 1) even when
the attribute loads are issued in another order.

### Next refusals for these cases

Established from the corpus verdicts (`tools/mali/corpus-verdicts.txt`) of the
other stage of each pair, not by compiling the changed vertex programs (they
do not compile yet):

| case | after the varying is lowered |
|---|---|
| 63 | `matattr2.vert` has two `fadd`s on one `off`, `matattr3/4.vert` the `do_rebalance_tree` refusal; the fragment `matattr.frag` is ok |
| 78 | fragment `matvary.frag` ok |
| 101 | the fragment: `a texture look-up other than texture2D(sampler2D, coordinate) (a bias, ...)` (`cubebias.frag`) |
| 117 | `uniidx_d.frag`: ok (corpus `uniidx_d.frag: ok`) |
| 181 | `fma_fs_varying.frag` ok; the case also compiles `fma_fs_fused.frag` (ok) and the controls: `fma_vs_ctrl.vert` refused as `arithmetic other than +, -, *`, which stops the case after the fused half |
| 191 | `varyfma_fs.frag` ok |
| 199 | the case compiles `es3trunc_fs.frag` first (`arithmetic other than +, -, *`) and `es3trunc_vsc.vert` (same) |

## gl_FragColor lane shapes (cases 75, 185)

`MaliError::ColourForm` is raised at 12 sites of `src/shaderc/online/mali/nir_fs.rs`
(`op` `:496`, `neg` `:534`, `abs` `:545`, `op32` `:709`, `:1579`, `layout` `:1887`,
`vectorise` `:2199` and `:2235`, `:2320`, `word_of` `:2557` and `:2565`, `:2871`).
The two cases hit these (read from the code against the references; the
refusal was not re-run under a debugger):

* **185** `gl_FragColor = vec4(x, x, x, 1.0)` of a 50-step chain on uniforms
  (`u_a..u_d`, mediump out of highp arithmetic): `word_of` `nir_fs.rs:2557`,
  `_ if a == b => return Err(MaliError::ColourForm)` ("one conversion of a lane
  twice: not drawn by a reference"). Mesa draws it (`varyconst_185_kindcost_*`):

  ```
  %50 = f2f16 %49                 // one conversion
  %55 = vec4 %50, %50, %50, %52 (0x3c00)
  FADD.f32.wait0126 r0.h0, r0^, 0x0.neg        // the conversion: x -> half, low half of r0
  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
  IADD.v2u16 r1, r0.h00, 0x0                   // the pair (x, x): .h00 duplicates the half
  MKVEC.v2i16 r2, r0^.h0, u3.w1.h0             // the pair (x, 1.0): u3.w1 is the 1.0 half
  MOV.i32 r0, r1^                              // the BLEND source registers contiguous
  MOV.i32.wait r1, r2^
  BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
  ```

  The same tail for `kc_maxneg` (`MKVEC.v2i16 r2, r0^.h0, u3.w0.h0`, the 1.0
  half at `u3.w0`: the FAU slot of the constant moves with the uniform count).
  The chain itself is `FMA.f32 r0, rA, rB, 0x0.neg` per `fmul` (`kc_fmul`) and
  `FMAX.f32 r0, r0^.neg, u2.w1` per `fneg`+`fmax` (`kc_maxneg`), which the
  compiler lowers already (the `kc_mulneg` form is a separate refusal).
  58 / 57 instructions; two `MOV`s join the pairs.

  The shape: a lane used twice in a word is `IADD.v2u16 rN, <lane>.h00, 0x0`,
  the word (lane, constant 1.0) is `MKVEC.v2i16 rN, <lane>.h0, <FAU 1.0>.h0`.

* **75** `vec4(u_k.x - 0.5, u_k.y - 1.0, 1.0 - u_k.z, 1.0)` (mediump): Mesa
  vectorises the first pair into one `v2f16` add of the uniform's own pair and
  a constant pair, and the second pair as a scalar add plus the constant:

  ```
  16x2 %3 = fadd %1.xy, %2 (-0.500000, -1.000000)
  16   %4 = fneg %1.z ;  %6 = fadd %5 (1.0), %4
  FADD_IMM.v2f16.discard r0, u2.w0, #0xBC00B800        // x, y in one word
  FADD.v2f16.wait0126 r1, 0x3C000000.h11, u2.w1.neg.h00 // z (the h11 of a constant word is 1.0)
  ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
  MKVEC.v2i16.wait r1, r1^.h0, u3.w0.h0                // (z, 1.0)
  BLEND.slot0.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
  ```

  The second pair `(fadd(1.0, -u.z), Const(1.0))` has an `alu_of` lane and a
  constant, which `vectorise` (`nir_fs.rs:2135`) does not take: its last arm
  returns `ColourForm` at `:2235` (the arm before it, `(None, None, Const,
  _)`, needs a constant first). The first pair needs `FADD_IMM.v2f16` (an
  immediate pair of constants as the second source; `#0xBC00B800` is
  (-1.0, -0.5): y in the high half). Mesa's own rule for the second pair is
  the scalar `FADD.v2f16` (both halves computed, the high half of `u2.w1.neg.h00`
  duplicated) then `MKVEC.v2i16` of the low half with the 1.0 word.

  Descriptors: neither case has a varying; the fragment resource table's
  entry 1 is empty (`Address: 0x0, Size: 0x0`). The uniform words are in the
  FAU of the fragment (`u2`, `u3` here).

## Shared lowerings and an order

1. **Constant lanes dropped from the stored set** (63, 78, 101, 181). The
   varying's component count, the descriptor (`RG32F RGB1`, `RGBA32F RGBA`,
   `RGB32F`) and the packet stride (16 + the attribute stride) follow the
   remaining lanes; the fragment keeps reading the constant. Smallest
   change: `nir.rs:1582`, `Val::Const(_) => skip the store` plus the
   varying's `comps` in `link.rs`. Unlocks 78 and 63 (and 101 up to its
   texture refusal).
2. **A lane the fragment never reads is not stored** (191: z, w; 63: none;
   same mask as item 1, from the fragment's `inputs_read`-per-lane). Needed
   before 191 compiles.
3. **A uniform-only lane is a flat varying** (117, 181, 191, 199):
   `FMA.f32 r, x, 0x0, x` per lane and `INTERP_MODE_FLAT` in the fragment
   (`LD_VAR_BUF_IMM...flat32`, address operand `0x0`, descriptor format `*UI`
   instead of `*F`). Replaces `nir.rs:1588-1590`. The vertex lane's own
   arithmetic (181's folded `ffma`s) goes through the existing vertex
   lowering. fp32 fragment (199) needs no conversion.
4. **The `(x, x)` word and the `(x, 1.0)` word of the mediump colour**
   (185): `IADD.v2u16 rN, r.h00, 0x0` and `MKVEC.v2i16 rN, r.h0, <FAU 1.0>.h0`,
   followed by the two `MOV`s that make the source registers contiguous.
   Replaces `nir_fs.rs:2557`.
5. **`FADD_IMM.v2f16` for a pair with a constant pair, and the scalar
   `FADD.v2f16` pair plus `MKVEC` for the other** (75): `nir_fs.rs:2235`.

Order: 1 and 2 (one mask), then 3, then 4, then 5. 1+2 clear 63 and 78
(host-only references: the `varyconst_63_matattr`, `_78_matvary` words), 3
clears 117, 181 (to the control half), 191 and 199 (to their fragment arithmetic),
4 clears 185, 5 clears 75. 101 waits on the cube look-up with a bias.
The byte-equal check for each is `pan_ref_diff.py diff` against the
`varyconst_*.expect.json` it names (`vs.varying`, `fs.fragment`).
