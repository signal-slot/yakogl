<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): a second look-up, textureSize, 3D and 2D-array look-ups

Plan #153, item `texmore2` (retry of `study-texmore`); issue #1385/#1384
follow-ups for gl_test cases 2, 46, 48 and 206. This note is a reading of Mesa
26.2.1's panfrost source (`MESA_PAN_TREE`, citations `M:<path under src/>:<line>`)
and of our tree (`<path>:<line>` from the repository root). Nothing here was
run on the board.

**References.** Mesa 26.2.1's drm-shim build (gallium panfrost inside
`libgallium-26.2.1.so`, EGL surfaceless, GLES 2/3, the noop drm-shim) was built
from `mesa-26.2.1.tar.xz` (sha256 `c47e81bd...a4442`, as in
`third_party/mesa-26.2.1/UPSTREAM`) with the options of `drm_shim_ref.sh build`
plus `--pkg-config-path=/usr/lib/llvm/23/lib64/pkgconfig` (LLVMSPIRVLib lives
there). The references are `tools/mali/testdata/texmore_{2,46,48,206_vol,206_array}`
`.trace` (pandecode: programs, FAU, texture descriptors), `.stdout`, `.stderr`
(NIR and disassembly). Line numbers below are of the `.stderr` files' last
(final) fragment listing.

## Producing the references (the commands)

```sh
tools/mali/drm_shim_ref.sh build mesa-26.2.1.tar.xz $CONNECTED_DEPS_DIR/mesa-shim
# case 2: second look-up (ES 1.00; the files gl_test's texture.fs/uv.vs are)
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/shaders/gl_test_es100/uv.vert tools/shaders/gl_test_es100/texture2.frag \
    tools/mali/testdata/texmore_2 --uniform-values
# case 46: textureSize of a 2D texture and a 2D array (ES 3 mode, 8x8x3 array, 2x2 for the 2D one)
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/cube.vert tools/shaders/es300/texsize.frag \
    tools/mali/testdata/texmore_46 --uniform-values
# case 48: sampler3D (4x4x5, GL_LINEAR)
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/cube.vert tools/shaders/es300/vol.frag \
    tools/mali/testdata/texmore_48 --uniform-values
# case 206: the volume and the array over 3-level chains (--levels 3)
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/cube.vert tools/shaders/es300/vol.frag \
    tools/mali/testdata/texmore_206_vol --uniform-values --levels 3
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim tools/shaders/es300/cube.vert tools/shaders/es300/array.frag \
    tools/mali/testdata/texmore_206_array --uniform-values --levels 3
```

The words and disassembly come from the trace and
`.stderr` (`BIFROST_MESA_DEBUG=shaders,shaderdb`) as for every other
reference (`pan_ref_diff.py list <prefix>.trace`).

`gl_ref.py` ES 3 mode (this item): when either shader's first non-blank line
is `#version 300 es` it creates an EGL_CONTEXT_CLIENT_VERSION 3 context,
takes the vertex shader's `in` declarations as the attributes (declaration
order, as for `attribute`), and gives `sampler3D` a 4x4x5 RGBA8 volume
(case 48: LINEAR, WRAP_R clamped) and `sampler2DArray` an 8x8x3 array
(case 46: NEAREST); `--levels N` makes both an N-level chain at 16x16 (3D
depth 4 halving, array 2 layers) with NEAREST_MIPMAP_NEAREST (case 206).
A shader without that line takes the unchanged ES 2.0 path: no ES 2 output
moves. `--levels` is refused for an ES 1.00 program.

## Refusal sites (src/shaderc/online/mali)

| Case | Shader | Refusal | Site |
|---|---|---|---|
| 2 | texture2.frag, two `texture2D` in one FS | lowered (plan #154 texture-2, section 1); `TexCount` now names only a look-up beside another in a form a lone one takes | `nir_fs.rs` `Fs::look_ups`, `select`'s `lone` |
| 46 | texsize.frag, `textureSize(sampler2D)`, `textureSize(sampler2DArray)` | `MaliError::TexKind` | `nir_fs.rs:1689` (`ir::Op::TexSize`) |
| 48 | vol.frag, `texture`/`textureLod`/`texelFetch` of `sampler3D` and `textureSize` | `TexKind` | `nir_fs.rs:1664` (`es100_tex()` is not `Tex2D`; a 3D `Op::Tex` has no ES 1.00 form, `ir.rs:510`) and `:1689` |
| 206 | vol.frag and array.frag at levels 0..2 | `TexKind` | as 48; array.frag adds `Op::Tex` with a 2D-array dim, `ir.rs:959` |

The one look-up the lowering handles takes the texture and sampler index 0
and handle `TEX_HANDLE` = texture table 4, sampler table 3, index 0
(`nir_fs.rs:93-102`; `M:panfrost/compiler/pan_nir_lower_tex.c:723-745`,
`va_tex_handle`). The coordinate must be a whole vec2 varying or two
constants (`:1669-1683`, `TexCoord`).

## 1. Two look-ups in one shader (case 2)

`texmore_2` (texture2.frag, mediump, `0.5 * texture2D(u_tex0, v) + 0.25 *
texture2D(u_tex1, v)`; gl_ref binds a 2x2 texture at each unit). The final
listing (`texmore_2.stderr`, 16 instrs, 256 bytes):

```
LD_VAR_BUF_IMM.f32.slot0.v2.src_f32.center.store @r0:r1, r61^, index:0x0
MOV.i32 r2, u2.w0 ; MOV.i32.wait0 r3, 0x0
TEX_SINGLE.slot1.skip.reserved.16.rgb.2d.computed @r2:r3, @r0:r1, [r2^:r3^]
MOV.i32 r4, u2.w1 ; MOV.i32 r5, 0x0
TEX_SINGLE.slot2.skip.reserved.16.rgb.2d.computed.wait2 @r0:r1, @r0:r1, [r4^:r5^]
FMA.v2f16.discard r0, r0, 0x34000000.h11, 0x0.neg      (* 0.25, the second result)
FMA.v2f16.wait1 r1, r1.h00, 0x34000000.h11, 0x0.neg
FMA.v2f16 r0, r2^, 0x38000000.h11, r0^                 (+ 0.5 * the first)
FMA.v2f16.wait0126 r1, r3^.h00, 0x38000000.h11, r1^.h00
ATEST.wait0 @r60, r60^, 0x3F800000, atest_datum.w0
MKVEC.v2i16.wait r1, r1^.h0, u3.w0.h0 ; BLEND.slot0.v4.f16.end ...
```

- **Indices and handles.** The samplers are numbered in the order the linked
  program reads them: `u_tex0` is 0, `u_tex1` is 1. The handle of look-up
  *i* is the dword `(4<<27)|(i<<16)|(3<<11)|i` (`M:panfrost/compiler/
  pan_nir_lower_tex.c:723-745`), and it is a **pushed uniform**, not an
  immediate: the Fragment FAU of the trace is `u2 = 20001800 20011801`
  (`texmore_2.trace:520-522`), i.e. `TEX_HANDLE` for i = 0 and 1, read as
  `u2.w0` / `u2.w1` and moved into the first register of the 2-register
  `[handle:0]` operand (`MOV.i32 r2, u2.w0; MOV.i32 r3, 0x0`). Ours has the
  one handle as `TEX_HANDLE` (`nir_fs.rs:98-102`); a second handle is a second FAU
  word of the fragment's table, in order of use.
- **Descriptor table.** Two texture descriptors and two sampler
  descriptors in tables 4 and 3, index *i* = unit of the *i*-th read sampler.
- **Staging.** The coordinate pair `@r0:r1` (the `LD_VAR_BUF_IMM` result)
  feeds both messages; the first result `@r2:r3` is on a different pair,
  the second reuses `@r0:r1` (the coordinate pair is dead after the second
  message); `.wait2`/`.wait1` order the scoreboard slots: the first TEX is
  slot1, the second slot2, the FMAs wait on them (`.wait1`, `.wait0126`).
  The two look-ups are `rgb` (`skip`, 3 lanes, `.16` half-float result).
  The same coordinates are read by both: no second interpolation.
- **Implementation order.** (1) `Fs::look_up` from one to a list
  (`nir_fs.rs:1667`'s `TexCount` lifts), each with `TEX_HANDLE | i`
  and the FAU word; (2) `SELECT` pushes the handle words as uniforms after the
  declared ones (check `texmore_2.trace:520-522` against our FAU table); (3) the
  scheduler gives the second TEX slot2 and RA reuses the coordinate pair;
  (4) the frame binds two texture and two sampler descriptors (`src/mali`);
  (5) byte-compare to `texmore_2` words (`pan_ref_diff.py diff ... --draw`).

### What landed (plan #154 texture-2)

Steps (1), (2), (3) and (5); step (4), the frame, is open.

- `Fs::look_ups` (`nir_fs.rs`, up to `MAX_LOOK_UPS` = 64) holds every
  look-up the walk meets, an unrolled loop's once per pass. `layout`
  sets each one's texels read (`lanes`: the colour reaches them), its
  coordinate's load, and its descriptor index: the rank of its sampler's
  binding among the bindings the program reads (texmore_2 `u_tex0` 0,
  `u_tex1` 1; sel_t_second `u_tex1` alone 0; one sampler read twice keeps
  one index). The handle is `tex_handle(index)`, an immediate that the
  constant passes push as a FAU word in order of first use, as before:
  nothing else was needed for (2) and (3). The look-up's lane mask is
  trimmed to the last texel read (`nir_opt_shrink_vectors`: texmore_2's
  `.rgb`).
- The record (`MaliShader::samplers`) lists the bindings at descriptor
  indices 0, 1, ..; `MaliShader::sampler` stays the first of them, which
  is all `src/mali` binds today. **Until the frame binds `samplers[i]` at
  descriptor index `i` of the fragment texture and sampler tables, a
  program with two samplers reads a descriptor the frame did not write**
  (case 2, the femtovg glyph programs on the board).
- The mediump colour word of a lane alone and a constant (texmore_2's
  `(b, 1.0)`): `lane_word`/`lane_tree` build the lane as one-lane nodes
  (`FMA.v2f16 r1, r1.h00, ..`, the fusion as `vectorise`'s) and the word
  `MKVEC.v2i16 x.h0, <half>.h0`. Only sums and products of texels and
  constants take it; the other trees of that shape (case 75's uniform
  lane) stay `ColourForm` for plan #154 varyconst
  (docs/notes/mali-varying-constants.md, item 5).
- Several look-ups are taken in a mediump shader only at varyings or
  constants, and in a highp one only when none is a cube: `TexCount`.
- Byte-equal: `texmore_2` (case 2, both stages), the six femtovg
  `fill_image*.glyph*` programs, and, through the operand forms case 2
  admits (`Tex` at `MulA`/`FmaA`, `Op` at `FmaC`), case 91's
  `fsout_91_hdrscale`. One word is pinned, not equal
  (`tests.rs` `FEMTOVG_PINNED`): fill_image_gradient.glyph's glyph
  look-up, where Mesa clears `skip` and we set it. The shim's BIR listing
  of the pair (`drm_shim_ref.sh run` of `fvg_fill_image_gradient_glyph.*`,
  not committed) shows why: `bi_add_branch_compare_values`
  (`M:panfrost/compiler/bifrost/bi_helper_invocations.c`) marks
  `src[1].value` of each `BRANCHZ.i16` without testing its type, and that
  source is the zero constant, by then the FAU immediate `u256`, so SSA
  value 256 is marked. In this program `%256` is the second word of the
  glyph look-up's `SPLIT` (`%255, %256, %257, %258 = SPLIT.i32 %254`), so
  its result counts as needed by helpers. It is a collision of value
  numbers, not a data path, and it hits any program with a branch whose
  value 256 feeds a look-up's result; ours are not numbered as Mesa's (NIR
  def indices, then emission temporaries), so it is pinned, not
  reproduced. The bit only lets the look-up skip helper lanes, so the
  image is the same.
- femtovg's `filter_image` (49 look-ups) is no longer refused for the
  look-ups: it stops at the block table (`BlockTableFull`; Mesa keeps the
  24 guarded passes of `if (i >= sampleCount) break;` as nested branches,
  about 75 blocks, 25 `if`s, 949 instructions). Measured with the caps
  raised (64 or 128 blocks, 32 `if`s, 1280 BIR instructions, 1536 values
  and NIR nodes): the walk's loop unrolling then refuses it
  (`LoopForm`), and the guarded passes kept as branches are refused after
  the peephole select too. It needs the run-time loop work (plan #154
  loops-rt) and larger tables, not texture work.

## 2. textureSize (case 46)

`texmore_46` (texsize.frag; 37 instrs). Mesa does **not** use a sysval or a
uniform: `va_lower_tex_query` (`M:panfrost/compiler/pan_nir_lower_tex.c:
1043-1075`) with `pan_nir_load_va_tex_size` (`M:panfrost/compiler/pan_nir.h:
105-140`) reads the texture *descriptor* through the handle. For the 2D
texture (`@r0:r1`) and, after a `BRANCHZ` on a null texture, for the array:

```
LD_PKA.i64.unsigned.slot0.wait0 @r0:r1, 0x0, u2.w0     descriptor 0, bytes 0..7
U16_TO_U32.discard r2, r1.h0 ; U16_TO_U32 r1, r1^.h1   width-1, height-1 (halfwords 2, 3)
IADD_IMM.i32 r2, r2^, #0x1 ; IADD_IMM.i32 r1, r1^, #0x1
ICMP_OR.v2u16.eq.m1 r0, r0^.h00, 0x0, 0x0 ; S16_TO_S32 r0, r0^.h0   null test (halfword 0 == 0)
CSEL.u32.eq r2, r0, 0x0, r2^, 0x0 ; CSEL.u32.eq r0, r0^, 0x0, r1^, 0x0   0 when null
IADD_IMM.i32 r1, 0x0, #0x20                            descriptor 1 (the array)
LD_PKA.i64.unsigned.slot1.wait1 @r4:r5, r1^, u2.w0
... BRANCHZ.eq.reconverge ...: U16_TO_U32 r1, r5^.h1 ; IADD_IMM r3, 0x0, #0x38 ;
    LD_PKA.i16.unsigned.slot2.wait2 @r3, r3^, u2.w0 ; U16_TO_U32 ; +1 ; +1
S32_TO_F32 ; FMA.f32 rN, rN^, u2.w1, 0x0.neg           x * (1/255), u2.w1 = 0x3B808081
```

(`texmore_46.stderr` lines 400-438.) `LD_PKA` loads from a base given by the pushed word `u2.w0` plus the byte
offset in the register. The Fragment FAU of `texmore_46.trace:469-472` is
`u2 = 3E000004 3B808081`: `u2.w0 = 0x3E000004` is a **sysval word, not a handle**
(it differs from `TEX_HANDLE`; what it is -- the texture table's address
bits -- is to be read from `pan_nir_lower_tex.c` `va_tex_handle`/
`pan_nir_load_va_desc` and the FAU of the draw), and `u2.w1 = 0x3B808081` is
1/255. Offsets seen: 0 (descriptor 0, bytes 0..7: width-1, height-1), `0x20`
(descriptor 1 -- a descriptor is `0x20` bytes, `Size: 0x20` in the trace --
bytes 0..7), and `0x38` = `0x20 + 0x18`, the array's layer count - 1 at byte
24 of descriptor 1 (the shader reads `a.y` and `a.z`: height and layers; its
width is not used but is loaded). The `lod` argument is dropped
(`lower_txs_lod`, `M:panfrost/compiler/pan_compiler.c:120`). Each integer is
converted with `S32_TO_F32` and multiplied by `u2.w1`: `/ 255.0` is a multiply
by the pushed `1/255` (`0x3B808081`), not a division.

The frame needs nothing beyond the texture descriptors already needed for a
look-up (they are read in place): `Width-1`, `Height-1` in the first 8 bytes,
the array size at byte `6*4` (the `0x38` offset above, as the array is the
second texture: descriptor index 1, `0x20`), the 3D depth at byte `7*4`.

Implementation order: (1) `ir::Op::TexSize` (`nir_fs.rs:1689`) lowers to
`LD_PKA` of the descriptor and the unpack/`+1`/null-select above, for D2 (two
lanes) and D2Array (three: width, height, layers); (2) the handle for each
texture is a pushed FAU word; (3) byte-compare to `texmore_46`.

## 3. sampler3D and sampler2DArray look-ups (cases 48, 206)

Descriptors in the traces (`Texture:` dumps): `texmore_48` is `Dimension: 3D`,
`Width 4`, `Height 4`, `Depth 5`, `Levels 1`, `Array size 1`, `Format RGBA8
UNORM`, `Texel interleave true`, one plane (`Size 0x1400`, `Row stride 0x400`,
`Slice stride 0x400`); `texmore_206_vol` is 16x16x4, `Levels 3`, three planes
(one per level, 16, 8, 4 wide) with the same strides; `texmore_46`'s array is
`Dimension: 2D`, `Array size: 3`, one plane per layer (`Slice stride 0`);
`texmore_206_array` is `Array size 2`, `Levels 3`, **six planes in layer-major
order** (layer 0 levels 0..2, then layer 1 levels 0..2). Samplers: nearest
and clamp as set by `gl_ref.py`.

The three look-ups of vol.frag (`texmore_48.stderr:451-492`, 40 instrs):

```
TEX_SINGLE.slot0.skip.reserved.32.r.3d.computed.discard @r4, @r4:r5:r6, [r8^:r9^]   texture()
TEX_SINGLE.slot1.skip.reserved.32.r.3d.zero      @r1, @r6:r7:r8, [r10^:r11^]         textureLod(.., 0.0)
TEX_FETCH.slot2.reserved.32.b.3d                 @r2, @r6:r7:r8, [r2^:r3^]           texelFetch(.., 0)
```

(the handle `u3.w0` here is `0x20001800` = `TEX_HANDLE` (`texmore_48.trace:411`); the Fragment FAU
`u2 = 42000000 3E000004` (`texmore_48.trace:410`) is `u2.w0` = `zz` = 32.0 (the Z
coordinate, `MOV.i32 r6, u2.w0`) and `u2.w1` = `0x3E000004`, the base word of
the `LD_PKA` descriptor loads (`LD_PKA.i16 ... u2.w1`, the word that is `u2.w0`
in case 46); 1/255 is not pushed in this case; the
coordinate triple is S, T, Z in consecutive registers; `.r`/`.b` is the single
lane read (`skip` and the lane select, 32-bit); `.zero` is the LOD mode for a
constant 0 level, `.computed` the derivative one; `texelFetch` has no LOD mode
(its level 0 is implicit in `TEX_FETCH`).) The size query is an
`LD_PKA.i16` at byte `0x1C` (= `7*4`, the depth minus one) with the same null
branch as in section 2 (`texmore_48.stderr:451-461`). The `texelFetch` Z is
`F32_TO_S32.rtz (zz * float(sz.z))` (`S32_TO_F32 r0, r0^; FMA.f32 r3, u2.w0, r0,
0x0.neg; F32_TO_S32.rtz r3, r3^`, lines 465-467) computed before the TEX_FETCH. The
1/255 of the output is built in a register: `IADD_IMM.i32 r3, 0x0, #0x3B808081`
then `FMA.f32.wait0126 r3, r0^, r3^, 0x0.neg` (lines 486-487). array.frag (`texmore_206_array.stderr:433-473`, mediump; the `layer` uniform is
the pushed half `u2.w0.h0`):

```
TEX_FETCH.slot2.reserved.16.a.2d.array_enable @r1, @r6:r7:r8, [r4^:r5^]     texelFetch: coordinates S, T, layer (F16_TO_S32.rtz of `layer`)
TEX_SINGLE.slot0.skip.reserved.16.rgb.2d.array_enable.computed.wait02 @r2:r3, @r4:r5:r6, [r2^:r3^]   texture(): S, T, layer (F16_TO_F32 of `layer`; no rounding in the program)
```

So on v10 the layer of a `texture()` look-up is a plain coordinate register
(`array_enable`; `M:panfrost/compiler/pan_nir_lower_tex.c:835-849`; the RTNE
and `narrow` forms are v11+); the `texelFetch` layer is an integer register.
Case 206's three levels of the vol shader run through `computed` LOD with the
sampler's NEAREST_MIPMAP_NEAREST (a different draw viewport per level: the
shader is the same; only the descriptors/sampler differ).

Implementation order: (1) accept `Op::Tex` with D3 / D2Array and
`Op::TexSize` for them at `nir_fs.rs:1664`/`:1689`; (2) `TEX_SINGLE`/`TEX_FETCH`
operands: 3 coordinate registers, the dimension (`3d`, `2d` + `array_enable`),
the LOD mode (`computed`, `zero`; fetch), the 16/32 result width and lane
select from the precision; (3) the descriptors above in `src/mali/frame.rs`
(3D with the slice stride, array with per-layer planes in layer-major
order); (4) byte-compare `texmore_48`, `texmore_206_vol`, `texmore_206_array`,
`texmore_46`; (5) board runs of 46, 48, 206 (not this item).

## Caveat

The `texmore_*` references were drawn with the gl_ref.py default texel data and
`--uniform-values` (so `zz` and `layer` are 32 and the pushed values are
`16*(k+1)+c`); the words do not depend on texel data. Case 46's 2D texture is
2x2 here (gl_test: 8x24) and case 206's array level shapes follow gl_test's
(16x16 x 2 layers, 3 levels) -- the descriptors, not the program, differ.
