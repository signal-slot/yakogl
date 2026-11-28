<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 120 `dEQP-GLES3.functional.texture.units` fails (T-UNITS)

Plan #400, item `cause-units`. Investigation only: no file under `src/`,
`tests/` or `tools/` changes. Everything here is **HOST-ONLY**: no board
was touched, and the host library renders nothing (the Linux OSAL keeps the
command stream), so the host shows compile and bind facts, never a picture.
The board rerun of the tree row that holds `texture.units` measures the
outcome.

## 1. Summary

`texture.units` has no cause of its own. The group draws one quad whose
fragment shader averages N textures of random type and random format, so
each case fails exactly when one of its textures hits a defect that another
family already records. Replaying dEQP's random choices for all 159 mustpass
cases gives each failing row one of three causes, and predicts the
`ad2b5ed` outcome of every row (section 4):

| rows | message at `ad2b5ed` | cause | state at the tip |
|---|---|---|---|
| 103 | `Compile failed at sglrGLContext.cpp:863` | a sampler with an **integer result on a cube, a 2D array or a 3D texture** (`isamplerCube`, `usampler2DArray`, `isampler3D`, ...): the selector's last `ir::Op::Tex` refusal (RC2 with RC6, RC9, RC15 of `docs/es3-cts-failures.md`) | **fixed on the host**: all 159 shaders compile (section 5) |
| 16 | `Image comparison failed` | a **2D array or a 3D texture** with a float or normalized format, sampled from the stacked array (RC6) or the 3D slice grid (RC15, `docs/notes/pvr-3d-mipmap-hang.md`) | arrays: native by default since `02c5c9af`; 3D: native path exists, **default off** |
| 1 | `Image comparison failed` (`8_units.only_2d.7`) | a 2D **`GL_RGBA32F`** texture, then stored as halves; dEQP fills float textures with +-1e5, beyond the half maximum 65504. The same cause as `texture.format.sized.2d.rgba32f_{pot,npot}` | **fixed on the host** by `ecd9743e` (`tests/es3_rgba32f_pvr.rs`) |

So: nothing new to file. What can remain on the board after the rerun is the
cases that hold a 3D texture while `PVR_NATIVE_3D_DEFAULT` is 0 (52 of the
120 rows, section 6), plus whatever the first board pictures of integer
cube, array and 3D look-ups show.

## 2. What the test does

`modules/gles3/functional/es3fTextureUnitTests.cpp` of VK-GL-CTS at
`93bca01` (the commit `tools/cts/deps.toml` pins):

- `TextureUnitTests::init` (`:1200-1241`): four unit counts (2, 4, 8, and
  `GL_MAX_TEXTURE_IMAGE_UNITS`, which is 16 here: `CAP_TEXTURE_UNITS`,
  `src/gl/context.rs:149`) times five types (`only_2d`, `only_cube`,
  `only_2d_array`, `only_3d`, `mixed`) times ten cases, seeded
  `deUint32Hash(testNdx)`. The mustpass list keeps 159 of the 200: all 150
  of 2, 4 and 8 units, and nine `all_units` cases.
- `TextureUnitCase::init` (`:677-913`): the number of textures (the unit
  count, or up to two fewer so that a texture is bound to several units);
  per texture a type (random in `mixed`), an internal format out of **all 49
  sized colour formats** (`s_testSizedInternalFormats`, `:85-92`, integer
  and 32-bit float ones included), three wrap modes, and a mag and min
  filter (nearest only when the format is not filterable); then a
  two-colour grid in every level. Sizes: 2D 128x128, cube 256x256, 2D array
  64x64x4, 3D 32x32x32, each with its whole mip chain.
- `generateMultiTexFragmentShader` (`:132-181`): one sampler per unit, of
  the type and the result class of its texture (`sampler2D`, `isamplerCube`,
  `usampler3D`, ...), each look-up `texture(u_samplerN, vecK(u_transN *
  vec4(v_coord, 1.0, 1.0)))` scaled, biased and averaged.
- `iterate` (`:915-1002`): one quad in a 128x128 viewport, against the
  reference rasterizer with `tcu::fuzzyCompare`, threshold 0.001.

Nothing in it is specific to several units: no per-unit state other than
the binding is exercised. The 39 cases that passed at `ad2b5ed` (2D and
float or normalized cube textures only, up to 16 units) show that the
unit plumbing itself works.

## 3. The case families

`docs/es3/am62p-gles3-notpass.tsv`, the rows of build `ad2b5ed`
(`grep texture.units`, 120 lines):

| group | cases in mustpass | Compile failed | Image comparison failed | pass |
|---|---|---|---|---|
| `2_units.only_2d` | 10 | 0 | 0 | 10 |
| `2_units.only_cube` | 10 | 5 | 0 | 5 |
| `2_units.only_2d_array` | 10 | 6 | 4 | 0 |
| `2_units.only_3d` | 10 | 6 | 4 | 0 |
| `2_units.mixed` | 10 | 5 | 4 | 1 |
| `4_units.only_2d` | 10 | 0 | 0 | 10 |
| `4_units.only_cube` | 10 | 8 | 0 | 2 |
| `4_units.only_2d_array` | 10 | 9 | 1 | 0 |
| `4_units.only_3d` | 10 | 9 | 1 | 0 |
| `4_units.mixed` | 10 | 8 | 2 | 0 |
| `8_units.only_2d` | 10 | 0 | 1 | 9 |
| `8_units.only_cube` | 10 | 10 | 0 | 0 |
| `8_units.only_2d_array` | 10 | 10 | 0 | 0 |
| `8_units.only_3d` | 10 | 10 | 0 | 0 |
| `8_units.mixed` | 10 | 10 | 0 | 0 |
| `all_units.*` | 9 | 7 | 0 | 2 (`only_2d.0`, `.1`) |
| total | 159 | 103 | 17 | 39 |

The compile share grows with the unit count only because more textures
make an integer format on a non-2D type more likely (25 of the 49 formats
are integer).

## 4. Evidence: dEQP's choices replayed

A scratch Python script (not committed) replayed `TextureUnitCase::init`'s
random stream: `deRandom_init` and `deRandom_getUint32`
(`framework/delibs/debase/deRandom.c:35-59`), `getFloat`, `getInt` and
`shuffle` (`framework/delibs/decpp/deRandom.hpp:127-134`, `:202-214`),
`deInt32Hash` (`framework/delibs/debase/deInt32.h:601-611`), the filterable
list (`glu::isGLInternalColorFormatFilterable`,
`framework/opengl/gluTextureUtil.cpp:879-`), and one `getUint32` per grid
colour (six for a cube). It printed type, format, wraps and filters per
texture and the unit assignment per case.

**The rule.** Predict "compile" when a case has an integer format on a
cube, array or 3D texture; else "image" when it has an array or a 3D
texture; else "pass". Against the TSV, over the 159 mustpass cases:

```text
predicted compile, recorded Compile failed          103
predicted image,   recorded Image comparison failed  16
predicted pass,    recorded pass                     39
predicted pass,    recorded Image comparison failed   1   8_units.only_2d.7
```

158 of 159 rows; no compile row without such a sampler, no passing row
with an array or a 3D texture. The 103 compile rows by the types that carry
the integer format: 3D 34, 2D array 29, cube 27, array and 3D 7, array and
cube 3, 3D and cube 2, all three 1. The 16 image rows: 9 hold arrays and
no 3D texture, 7 hold a 3D texture.

**The representative cases** (unit order, then the textures):

```text
2_units.mixed.0    (Compile failed)   units->tex [1, 0]
   tex0 2d RGBA4, REPEAT/MIRROR, mag N, min LINEAR_MIPMAP_LINEAR
   tex1 3d R32I,  MIRROR/REPEAT/MIRROR, mag N, min NEAREST_MIPMAP_NEAREST   -> isampler3D
2_units.mixed.3    (Image comparison failed)   units->tex [1, 0]
   tex0 cube R8_SNORM, mag N, min NEAREST_MIPMAP_LINEAR
   tex1 2darray RG16F, mag L, min N                                         -> sampler2DArray
8_units.only_2d.7  (Image comparison failed)   units->tex [5, 2, 1, 3, 6, 6, 4, 0]
   tex0 RG16I  tex1 RGB16UI  tex2 RG32I  tex3 RGB9_E5 (L, LMN)  tex4 RG16I
   tex5 RGB16F (L, LMN)  tex6 RGBA32F (REPEAT/CLAMP, N, N), bound to units 4 and 5
```

**The seven rows `docs/es3/texture-host-repro.md` section 7 left
unexplained** (it had no CTS source): `2_units.mixed.3` (array RG16F),
`.5` (3D RG8_SNORM, cube RGBA32F), `.6` (array SRGB8_ALPHA8, cube RGB32F),
`.8` (array RGBA8_SNORM on both units), `4_units.mixed.5` (the textures of
`2_units.mixed.5`), `4_units.mixed.8` (arrays RGBA8_SNORM and RGB9_E5) all
hold an array or a 3D texture: RC6 and RC15, like the `only_2d_array` and
`only_3d` rows. That leaves `8_units.only_2d.7`.

**`8_units.only_2d.7` is RGBA32F.** Among the 2D textures of every passing
case, each format of `only_2d.7` except one occurs: RG16I, RGB16UI, RG32I,
RGB9_E5 and RGB16F pass elsewhere; **RGBA32F as a 2D texture occurs in no
passing case** (R32F and RGB32F 2D textures do, in `4_units.only_2d.2`,
`.4`, `8_units.only_2d.2`, `.4`). The same run records
`texture.format.sized.2d.rgba32f_pot`, `rgba32f_npot` and
`texture.specification.basic_teximage2d.rgba32f_2d` as `Image comparison
failed`. At `ad2b5ed` the GL core stored `GL_RGBA32F` as halves; dEQP's grid
colours for a float format are `valueMin`/`valueMax` = -1e5/+1e5
(`tcu::getTextureFormatInfo`), which a half cannot hold.

## 5. Evidence: the tip on the host

Commit `52434422` (the integration tip). Note: at this commit
`capi/Cargo.toml` names the feature `vivante-st` twice (lines 41 and 85)
and cargo refuses the workspace (`error: duplicate key`); the runs below
were made with the second line removed in the working tree and put back
afterwards. That is not this item's defect; it is reported to the planner.

**All 159 fragment shaders compile.** The replay script wrote dEQP's
fragment shader for each mustpass case (the text of
`generateMultiTexFragmentShader`, sampler types from the replayed
formats). A scratch test (deleted, not committed) ran each through the
PowerVR online chain as `tests/es3_pvr_2darray.rs`'s `compile_with` does
(`lower::compile_to_ir`, then `powervr::chain::compile_fragment`), with
`pvr_native_3d` at 0 (the default) and at 1:

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test <scratch> -- --nocapture

native_3d=0 2_units.mixed.0: compiles, 454 dwords
native_3d=0 2_units.mixed.3: compiles, 356 dwords
native_3d=0 8_units.only_2d.7: compiles, 1130 dwords
native_3d=0 all_units.mixed.0: compiles, 4372 dwords
native_3d=0: 159 compile, 0 refused of 159
native_3d=1 2_units.mixed.0: compiles, 290 dwords
native_3d=1 2_units.mixed.3: compiles, 356 dwords
native_3d=1 8_units.only_2d.7: compiles, 1130 dwords
native_3d=1 all_units.mixed.0: compiles, 3070 dwords
native_3d=1: 159 compile, 0 refused of 159
```

This agrees with the host dEQP run of `docs/es3/texture-host-repro.md`
section 7 (tip `e72ae815`: no compile or link refusal in `texture.units.`).
The refusing arm is now a catch-all no ES 3.00 function reaches
(`src/shaderc/online/powervr/select.rs:1986`).

**RGBA32F holds full floats.**

```text
cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_rgba32f_pvr

test rgba16f_control_stays_halves ... ok
test rgba32f_native_cube_copy_carries_full_floats ... ok
test rgba32f_samples_full_floats ... ok
test rgba32f_without_the_extension_is_not_a_colour_attachment ... ok
test result: ok. 4 passed; 0 failed
```

`rgba32f_samples_full_floats` uploads 1e5 and -1e5 and finds them in the
bound `GLES_TEXFMT_RGBA32F` texels.

## 6. Code locations and what remains

| cause | where | commit |
|---|---|---|
| integer result on cube, array, 3D | `src/shaderc/online/powervr/select.rs` (the `ir::Op::Tex` arms; catch-all at `:1986`) | integer volumes `e179ad48`, offsets `f9744f26`, gradients `92b93c23` |
| 2D arrays | `PVR_NATIVE_ARRAY_DEFAULT = 1`, `src/shaderc/mod.rs:314`; `docs/es3/pvr-2darray.md` | `02c5c9af` |
| RGBA32F as halves | `rgba32f_own_class`, `src/gl/texture.rs:6618`; `TEXF_RGBA32F`, `src/pvr/backend.rs:1418` | `ecd9743e`, tests `6c262deb` |
| 3D slice grid | `PVR_NATIVE_3D_DEFAULT = 0`, `src/shaderc/mod.rs:342`; `docs/notes/pvr-3d-native.md`, `docs/notes/pvr-3d-mipmap-hang.md` | native path `e179ad48`, `e90122b8`; not the default |

**Proposed fix.** No change of its own for `texture.units`:

1. The 68 failing rows without a 3D texture (27 integer cube only, 31 with
   an array and no 3D, 9 float arrays, 1 RGBA32F) need no further code: the
   rerun at a commit that holds `02c5c9af` and `ecd9743e` should move them.
   Unverified on hardware: integer cube, integer array and RGBA32F pictures
   have host tests only (`tests/es3_pvr_int_mixed_gl.rs`,
   `tests/es3_pvr_2darray.rs`, `tests/es3_rgba32f_pvr.rs`).
2. The 52 failing rows that hold a 3D texture (45 compile, 7 image) are
   expected to stay `Image comparison failed` while 3D textures are the
   slice grid: it shows level 0 only and clamps R, and these cases choose
   mipmap min filters and REPEAT or MIRRORED_REPEAT on R at random. Their
   fix is the native 3D default, `PVR_NATIVE_3D_DEFAULT` in
   `src/shaderc/mod.rs` (with the board image's `DEQP_PVR_NATIVE_3D` arm as
   the A/B before the flip) -- the decision of #1285 and plan #400's
   `board-n3d` item, not a change for this family.
3. If a row without a 3D texture still fails on the board, replay its
   textures (section 4) and look at the one format/type pair no passing
   case holds; that names the family to send it to.

This is a forecast from the replay and the host, not a measurement.
