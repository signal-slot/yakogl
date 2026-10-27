<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): texelFetch, textureOffset, integer samplers, an array layer beside a 2D look-up

Plan #153, item `study-texfetch`. It covers the five shapes the online compiler
still refuses and `texmore` (`docs/notes/mali-texture-lookups.md`) did not
record: `es300/texoffset.frag`, `es300/texfetch.frag`, `gltest/usamp.frag`
(`usampler2D`), `gltest/isamp.frag` (`isampler2D`, mediump-looking
`gl_FragCoord` in a highp shader) and `es300/layers2.frag` (a
`sampler2DArray` and a `sampler2D` in one shader). Nothing was run on the
board and no Rust changed. Citations `M:<path>:<line>` are Mesa 26.2.1,
`src/panfrost/compiler/` (`MESA_PAN_TREE`); `<path>:<line>` is our tree.

## References and the commands

`tools/mali/testdata/texfetch_{texoffset,texfetch,usamp,isamp,layers2}.{trace,stdout,stderr}`
are real drm-shim traces (gallium panfrost on panthor, G610 `PAN_GPU_ID=a867`,
`BIFROST_MESA_DEBUG=shaders,shaderdb`). One command each, from the repository
root, `SHIM` being the shim build of `drm_shim_ref.sh build` (section "Producing
the references" of `mali-texture-lookups.md`):

```sh
VS=tools/shaders/es300/cube.vert; OUT=tools/mali/testdata
tools/mali/drm_shim_ref.sh run $SHIM $VS tools/shaders/es300/texoffset.frag $OUT/texfetch_texoffset --uniform-values
tools/mali/drm_shim_ref.sh run $SHIM $VS tools/shaders/es300/texfetch.frag  $OUT/texfetch_texfetch  --uniform-values
tools/mali/drm_shim_ref.sh run $SHIM $VS tools/shaders/gltest/usamp.frag    $OUT/texfetch_usamp     --uniform-values
tools/mali/drm_shim_ref.sh run $SHIM $VS tools/shaders/gltest/isamp.frag    $OUT/texfetch_isamp     --uniform-values
tools/mali/drm_shim_ref.sh run $SHIM $VS tools/shaders/es300/layers2.frag   $OUT/texfetch_layers2   --uniform-values
```

`gl_ref.py` gained (existing behaviour unchanged: `texmore_46` regenerated
with only the host pointer values and the dump path in `.stderr` line 1
differing, as for any two runs): `usampler2D` / `isampler2D` samplers get a
2x2 RGBA8UI / RGBA8I texture (texels 1,2,3,4; NEAREST), and
`--uniform-values` sets `int` / `ivecN` uniforms (`16*(k+1)+c`, as integers).
The words do not depend on the values. `p` = (32, 33), `lvl` = 48 in
texfetch; `uv` = (32, 33), `level` = 48 in texoffset.

## Refusal sites (`src/shaderc/online/mali/nir_fs.rs`)

| Shader | Refusal | Site |
|---|---|---|
| texoffset | `TexKind`: `Op::Tex` with an offset / non-`Tex2D` `es100_tex()` | `:1944-1947` |
| layers2 | `TexKind` (sampler2DArray), then `TexCount` (two look-ups) | `:1944`, `:1949` |
| usamp | `TexKind`: `ret` is not `F32` | `:1944` |
| texfetch | `UniformType` (`ivec2 p`, `int lvl`), before the fetch itself (`:1944`) | `mod.rs:339` |
| isamp | mediump-typed `gl_FragCoord` use (`ivec2(gl_FragCoord.xy) / 32`), then the fetch (`:1944`) | as stated by the plan |

## 1. Which instruction Mesa picks

`va_lower_tex` (`M:pan_nir_lower_tex.c:802-972`) lowers every `nir_tex_instr`
to `TEX_SINGLE` (`tex`, `txl`, `txb`, `txd`) or `TEX_FETCH` (`txf`, integer
coordinates). The handle operand is two registers `[handle : narrow]`; the
staging operand is the coordinate registers, then the optional ones in the order
of `VA_TEX_SR_*` (`:790-799`): S, T, R, array layer, shadow compare, offset
word, LOD, gradient descriptor. The result is 4 words (`.32`), or the
`.r` single lane when only one is read (layers2's `shadows`).

| GLSL | Instruction | Flags (`.reserved.32.<dim>.<...>`) |
|---|---|---|
| `texture(s, uv)` | `TEX_SINGLE` | `.2d.computed` |
| `textureOffset(s, uv, ivec2 const)` | `TEX_SINGLE` | `.2d.computed`; offset in the handle's high word |
| `textureLodOffset(.., level, off)`, level run-time | `TEX_SINGLE` | `.2d.explicit`; LOD 8.8 word as the 3rd staging register |
| `textureLodOffset(.., 0.0, off)` | `TEX_SINGLE` | `.2d.zero`; no LOD register |
| `texture(sampler2DArray, vec3)` | `TEX_SINGLE` | `.2d.array_enable.computed`; layer is a plain float register |
| `texture(usampler2D, uv)` | `TEX_SINGLE` | `.2d.computed`, unsigned result registers |
| `texelFetch(s, p, lvl)`, `lvl` run-time | `TEX_FETCH` | `.2d.texel_offset`; offset word carries the level |
| `texelFetch(isampler2D, ivec2, 0)` | `TEX_FETCH` | `.2d`; level 0 in the handle's narrow word |

## 2. How the offset is packed

An offset is a 4-bit signed value per axis (`scalar_is_imm_i4`, `:870-895`).
When every component is a constant in -8..7 and the handle is narrow
(`!wide_indices`), the `u8vec4 <off_s, off_t, off_r_or_ms, lod>` is OR-ed into
the **high dword of the handle operand** (`narrow`, `:960`): nibble *i* is
component *i* (`narrow_offset |= imm_i4 << (i*4)`). The traces confirm it
(`texoffset`'s NIR handles `(0x20001800, 0xf)` and the FAU words):

| call | offset | handle high word |
|---|---|---|
| `textureOffset(.., ivec2(-1, 0))` | x = 0xF | `0x0000000F` (`u4.w0`) |
| `textureLodOffset(.., ivec2(0, -1))` | y = 0xF | `0x000000F0` (`u5.w1`) |
| `textureLodOffset(.., ivec2(1, -1))` | | `0x000000F1` (`u6.w0`) |
| `textureLodOffset(.., 0.0, ivec2(7, -8))` | | `0x00000087` (`u6.w1`) |

The low dword is the ordinary handle `0x20001800` (`u3.w1`), the same `TEX_HANDLE`
as in `texmore`. The high word is a pushed FAU word, `MOV`ed into the second
register of `[r4:r5]`; the offset costs no instruction.

`texelFetch` is the other case: a `txf` carries its level in byte 3 of the
same word, and a run-time level (or run-time offset) is not narrow, so Mesa
sets `texel_offset` and puts the packed word in a **staging** register
(`pack_32_4x8(i2i8(vec4(off_s, off_t, 0, min(lvl, 255))))`, `:896-915`).
`texfetch` shows it: `lvl` = `u3.w0` (48), `u3.w1` = 255 (`CSEL.u32.lt.discard
r0, u3.w0, u3.w1, ...` is `umin`), then `MKVEC.v2i8 r0, 0x0.b0, r0.b0, 0x0` /
`MKVEC.v2i8 r0, 0x0.b0, 0x0.b0, r0` shift it to byte 3, and the instruction is

```
TEX_FETCH.slot0.reserved.32.2d.texel_offset @r2:r3:r4:r5, @r2:r3:r4, [r6^:r7^]
```

with `r2:r3` = `p` (integers; the FAU words `u2 = 0x20, 0x21`), `r4` the packed
word and the handle `[u4.w0 = 0x20001800 : 0]`. The second fetch
(`p + ivec2(1,0)`) is `IADD_IMM.i32 r1, u2.w0, #0x1` first, and reuses the offset
register. A constant level of at most 15 and no offset is narrow (`:869-880`,
`narrow |= min(lod, 15) << 12`): `isamp`'s `texelFetch(.., 0)` has handle
`[u3.w1 : 0]`.

A run-time `textureLod` level (`texoffset`'s `level` = 48) is
`fsat_signed(level / 16) * 4096`, `F32_TO_S32.rtz`, `U16_TO_U32`
(`build_sfixed_8_8`): `FMA.f32.clamp_m1_1 r4, u3.w0, 1/16`, `FMA.f32 r4, r4,
4096`, `F32_TO_S32.rtz`, `U16_TO_U32 r4, r4.h0`, the third staging register
(`@r6:r7:r8`, `.explicit`). `0.0` is `.zero` and no register.

## 3. Integer registers

* `usampler2D` (`texfetch_usamp`): the texture descriptor format is
  **RGBA8UI** (`Format (v7): RGBA8UI RGBA`, swizzle 1672 = RGBA), the sampler
  NEAREST with `Mipmap Mode: Nearest`; the instruction is the ordinary
  `TEX_SINGLE.slot1.skip.reserved.32.2d.computed @r0:r1:r2:r3, @r0:r1, [r2:r3]`
  and the results are **unsigned integers in the destination registers**
  (the BIR dump prints `.u32`). The shader then converts: `U32_TO_F32` per
  lane, `FMA.f32 x, 1/255 (u2.w1 = 0x3B808081), 0x0.neg`. No other flag marks
  the type; it is the descriptor's format.
  The coordinate `v_xy * 0.5 + 0.5` is `FMA.f32 r, r, 0x38000000.h1, 0x38000000.h1`
  (0.5 as an fp16 half of an immediate; the same constant form `texmore_2` has).
* `isampler2D` (`texfetch_isamp`): descriptor **RGBA8I**; `TEX_FETCH ... .32.2d`
  returns signed integers sign-extended in 32-bit registers (RGBA8I has no
  fixed-function conversion here); `+ 128` is `IADD_IMM.i32 r, r, #0x80`, then
  `S32_TO_F32`, `FMA.f32 x, 1/255, 0x0.neg`.
* `gl_FragCoord.xy` as integers (isamp): the highp coordinate comes from the
  packed `r59` (`U16_TO_F32 r0, r59.h1`; `FADD_IMM #0.5`;
  `FMA.f32 r0, r0, u2.w0, u2.w1` with `u2 = 0x3F800000, 0` -- the y flip is
  the FAU scale and bias, and for this 64x64 target the identity plus 0)
  and `U16_TO_U32 r1, r59.h0`, `F32_TO_S32.rtz` for y. The signed `/ 32` is:
  `sign = min(max(v, -1), 1)` (`CSEL.s32.gt ... 0xFFFFFFFF` then
  `CSEL.s32.lt ... u3.w0` with `u3.w0 = 1`), `IABS`, `RSHIFT_OR ... 0x7060504.b11`
  (a shift by 5), `IMUL` by the sign. See `docs/notes/mali-highp-fragcoord.md`
  for the coordinate itself; this note adds only the signed division.
* The texture is read **after** the divides, so the two coordinate registers are
  `@r2:r3` and the result lands on `@r0:r1:r2:r3`, the handle `[r0^:r1^]`
  (`MOV.i32 r0, u3.w1 ; MOV.i32 r1, 0`) overwriting r0 and r1 (the allocator
  reuses them as the handle is `^` last-use).

## 4. layers2: an array layer next to a 2D look-up

`texture(shadows, vec3(v_xy, 1.0)).r` is
`TEX_SINGLE.slot1.skip.reserved.32.r.2d.array_enable.computed @r2, @r0:r1:r2, [r4^:r5^]`
(only `.r` is read, one result register, `step(bias, d)` becomes
`FCMP_OR.f32.ge.m1`); the layer is the constant `MOV.i32 r2, 0x3F800000`, a plain
float register, as in `texmore_206_array` (the sampler has
`Clamp integer array indices: true`, no rounding in the program). The second,
`texture(base, v_xy)`, is a `.2d.computed` 4-lane look-up with handle
`u4.w1 = 0x20011801` (index 1, `texmore_2`'s second handle) and slot2. The
fragment FAU is `u2..u3` = `tint` and `u4.w0` = `bias`, `u5.w0 = 0x20001800` the
array's handle, `u4.w1` the 2D one: handle words are pushed **in order of use**
after the declared uniforms, as `texmore` found.

## 5. Descriptors the frame needs (pinned to the dumps)

| Case | Table 4 (textures) | Table 3 (samplers) |
|---|---|---|
| texoffset, texfetch | 2D, `RGBA8 UNORM`, 2x2, `Levels 1`, swizzle 1672, row stride 0x400, slice stride 0 | Clamp S/T, `Minify/Magnify nearest`, `Mipmap Mode: None`, LOD 0..31.996 |
| usamp | 2D, `RGBA8UI RGBA`, same strides | as above but `Mipmap Mode: Nearest` (Mesa sets it for integer formats) |
| isamp | 2D, `RGBA8I RGBA`, same strides | as usamp |
| layers2 | index 0: 2D, `Array size: 3`, one plane per layer (`texfetch_layers2.trace:381-430`); index 1: 2D 2x2, `Array size: 1` | one per sampler, `Clamp integer array indices: true`, nearest, `Mipmap Mode: None` |

Each look-up's texture index equals its sampler index, in the order the linked
program reads them (`texmore_2`). Integer formats make the frame choose the
descriptor format from the **sampler type**, not only the GL internal format.

## 6. What would have to change

**`src/shaderc/online/mali` (the compiler)**
1. `nir_fs.rs:1944`: accept `Op::Tex` with `offset: Some(const ivec2)` and
   `TexLod::Lod`; the offset's 4-bit packing goes into a second FAU word per
   look-up (`TEX_HANDLE` low, the narrow word high), `Src::of(Idx::Const(..))`
   at `:3924` becoming `[handle, narrow]`; the run-time level uses the
   existing cube `sfixed_8_8` (`cube_desc` neighbour at `:1910`) and `.explicit`.
2. `ir::Op::Tex` for `usampler2D` / `isampler2D`: carry `ret: U32 / S32` to
   the lowering; the `TEX_SINGLE`/`TEX_FETCH` word is unchanged, the converts
   (`U32_TO_F32`, `S32_TO_F32`) and the `IADD_IMM` already lower in `select.rs`.
3. A new fetch form: `TEX_FETCH` with `texel_offset`, the packed word built by
   `umin(lvl, 255)` and `MKVEC.v2i8` (byte 3), integer `p` from FAU words,
   `p + ivec2(1,0)` as `IADD_IMM`; `IUNIFORM` (`ivec2`, `int`) in the uniform
   table (today `UniformType`).
4. `gl_FragCoord` as an integer: `F32_TO_S32.rtz` of the highp coordinate and
   the signed division by a constant power of two (`CSEL`/`IABS`/`RSHIFT_OR`/`IMUL`).
5. `layers2`: the array dimension in `TEX_SINGLE` (`array_enable`, third
   coordinate register), and a list of look-ups instead of `f.look_up`
   (`TexCount`, `texmore_2`).

**`src/mali` (the frame)**: textures of format RGBA8UI / RGBA8I from the
sampler type; a 2D array descriptor with one plane per layer; two texture and
sampler descriptors in tables 4 and 3 when two samplers are read; the FAU gets
the handle words (and the `0xFF` / `1/255` constants) after the uniforms.
`SampledTexture` (`frame.rs:1155`) is 2D RGBA8 / cube only.

Byte-compare against `texfetch_*` with `pan_ref_diff.py extract --draw`, then
two board sessions (not this item).
