<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the 188 dEQP-GLES3 shaders.texture_functions fails (SH-TEXFN), sub-causes and fix designs

Plan #400, item design-texfn. This is host-only reading at the integration tip
`5f781c49` (2026-11-28). There was no board run and no source change. Every
`file:line` below refers to that tree unless another commit is named.

The case list is cause SH-TEXFN of
`python3 -I tools/cts/am62p-notpass-causes.py docs/es3/am62p-gles3-notpass.tsv`
(188 lines, all `Fail`). They come from row 28 of
`tools/cts/am62p-gles3-tree.txt` (`shaders.texture_functions` + uniform_block
+ state_query + texture.compressed). The TSV header says that row was last
measured at build `ad2b5ed` (2026-10-13/14). Board-c ran row 28 again at
`472630df`, but that result is not in the tree at `5f781c49`. A board run may
change the counts below; the sub-cause rules in section 2 are meant to be
applied to the new list again.

dEQP source: `modules/gles3/functional/es3fShaderTextureFunctionTests.cpp` of
VK-GL-CTS `93bca01`, read with `git show` from the clone in the dependency
cache.

## 1. Result in short

| sub-cause | cases | what fails | status at `5f781c49` |
|---|---:|---|---|
| A-WRAP | 12 | 2D-array look-up without a mip chain, on the stacked image: REPEAT on T is lost | **fixed at `02c5c9af`** (native 2D array by default) |
| A-MIP | 39 | 2D-array look-up of a mip-mapped array, on the stacked image | **fixed at `02c5c9af`** |
| A-SHADOW | 5 | `sampler2DArrayShadow` look-up, on the stacked image | **fixed at `02c5c9af`** (look-up). The compare accuracy belongs to T-SHADOW. |
| V-WRAP | 19 | 3D look-up without a mip chain, on the grid of slices: REPEAT lost on every axis | open: native 3D is **off** by default (`PVR_NATIVE_3D_DEFAULT = 0`). Fix: section 4.4 |
| V-MIP | 67 | 3D look-up of a mip-mapped volume, on the grid: level 0 only and REPEAT lost | open, same fix |
| V-FETCH | 8 | `texelFetch(Offset)` on a 3D texture at level 1 or 2, on the grid: reads level 0 | open, same fix |
| TS-2D | 10 | `textureSize` of a 2D texture at `GL_TEXTURE_BASE_LEVEL` 1 | **fixed at `4c0bcd77`** |
| TS-CUBE | 10 | `textureSize` of a cube at base level 1 | open: the `src/gl` change of `docs/es3/shader-failure-map.md` section 7 |
| TS-ARRAY | 10 | `textureSize` of a 2D array at base level 1 | open: section 7's change **and** K4's (`docs/notes/pvr-npot-array-levels.md`) |
| TS-3D | 8 | `textureSize` of a 3D texture at base level 1 | open at the default. Fixed by `e90122b8` once native 3D is on (section 4.4) |
| **total** | **188** | | 66 fixed, 122 open |

What the changes since `ad2b5ed` do for this cause:

- **Native 2D arrays** (`02c5c9af`, default on, `src/shaderc/mod.rs:314`)
  fix A-WRAP, A-MIP and A-SHADOW (56 cases).
- **Native 3D** (`e179ad48`, `e90122b8`, plus the 3D forms of `f9744f26`,
  `92b93c23` and `1727ffe4`) covers every 3D form these cases use
  (`src/shaderc/online/powervr/select.rs:1526-1615`). But it is **not the
  default**: `PVR_NATIVE_3D_DEFAULT = 0` at `src/shaderc/mod.rs:342`. At
  the default, V-WRAP, V-MIP, V-FETCH and TS-3D (102 cases) still take the
  grid of slices.
- **K2** (`ecd9743e`, RGBA32F sampled as full floats) fixes **none** of
  the 188. No texture_functions spec uses `GL_RGBA32F`: the float specs are
  all `GL_RGBA16F` (`es3fShaderTextureFunctionTests.cpp`, lines 1820-1889).

Expected at the tip, if the code reading above holds on the board:

- With the default switches: 122 fail (102 3D + TS-CUBE + TS-ARRAY).
- With `DEQP_PVR_NATIVE_3D=1` (item board-n3d, rows 22-40, which hold
  row 28): 20 fail (TS-CUBE + TS-ARRAY).
- With the switch on and the two `src/gl` changes of section 5 applied: 0.

## 2. How the 188 split

### 2.1 The rule

`texturesize.*` goes by target: TS-2D, TS-CUBE, TS-ARRAY, TS-3D. Every other
case is a look-up on a `2darray` or `3d` sampler; none of the 188 is a 2D or
cube look-up. The look-ups are sorted by the dEQP `TextureSpec` they use
(`:1852-1889`):

- **A-SHADOW**: the sampler is `sampler2darrayshadow`.
- **A-WRAP**: another `2darray` case whose spec has one level
  (`tex2DArrayFixed/Float/Int/Uint`).
- **A-MIP**: another `2darray` case whose spec has 8 levels
  (`tex2DArrayMipmap*`).
- **V-FETCH**: a `3d` `texelfetch*` case.
- **V-WRAP**: another `3d` case whose spec has one level (`tex3DFixed/...`).
- **V-MIP**: another `3d` case whose spec has 7 levels (`tex3DMipmap*`).

The `_bias` forms of `textureoffset` use the one-level specs, so they are in
the WRAP rows. The `_bias` forms of `texture` and `textureproj` use the
mip-mapped ones.

### 2.2 By function and target

| function | 2D | cube | 2D array | 3D | total |
|---|---:|---:|---:|---:|---:|
| texture | | | 13 | 12 | 25 |
| textureOffset | | | 12 | 7 | 19 |
| textureProj | | | | 12 | 12 |
| textureProjOffset | | | | 4 | 4 |
| textureLod | | | 4 | 4 | 8 |
| textureLodOffset | | | 7 | 6 | 13 |
| textureProjLod | | | | 4 | 4 |
| textureProjLodOffset | | | | 5 | 5 |
| textureGrad | | | 10 | 8 | 18 |
| textureGradOffset | | | 10 | 8 | 18 |
| textureProjGrad | | | | 8 | 8 |
| textureProjGradOffset | | | | 8 | 8 |
| texelFetch | | | | 4 | 4 |
| texelFetchOffset | | | | 4 | 4 |
| textureSize | 10 | 10 | 10 | 8 | 38 |
| **total** | 10 | 10 | 66 | 102 | **188** |

dEQP has no projective form for 2D arrays.

### 2.3 By sampler type

| sampler | cases | | sampler | cases |
|---|---:|---|---|---:|
| sampler2darray_fixed | 16 | | sampler3d_fixed | 27 |
| sampler2darray_float | 16 | | sampler3d_float | 29 |
| isampler2darray | 14 | | isampler3d | 25 |
| usampler2darray | 13 | | usampler3d | 21 |
| sampler2darrayshadow | 7 | | (texturesize 2D, cube) | 20 |

The `texturesize` 2D and cube cases are 2 each of sampler2d_fixed,
sampler2d_float, isampler2d, usampler2d, sampler2dshadow, samplercube_fixed,
samplercube_float, isamplercube, usamplercube and samplercubeshadow. Fixed,
float, signed and unsigned samplers all fail in the same proportion, so no
sub-cause depends on the format.

### 2.4 By stage

| sub-cause | fragment | vertex |
|---|---:|---:|
| A-WRAP | 4 | 8 |
| A-MIP | 26 | 13 |
| A-SHADOW | 3 | 2 |
| V-WRAP | 4 | 15 |
| V-MIP | 44 | 23 |
| V-FETCH | 4 | 4 |
| TS-* | 19 | 19 |
| **total** | **104** | **84** |

The stage is not a cause. The vertex cases lean towards the WRAP rows only
because dEQP gives most vertex cases the one-level spec. `docs/notes/pvr-vertex-texture-es3.md`
found the same for texture.vertex: the vertex stage has no sampler defect of
its own.

## 3. What the pass/fail pattern shows

Every 2D-array and 3D case in the mustpass list fails except 24 (192 names
on those targets, less the 24, is 168; adding the 20 2D and cube
`texturesize` cases gives 188). The 24 that pass are all `texelFetch` or
`texelFetchOffset`:

- **All 16 on 2D arrays pass**, at levels 0 to 3 (`lod` 0, 1, 2, 3,
  `:2875-2884`, `:2908-2917`). So the stacked array's storage, its POT chain
  (128x128x4, 8 levels) and the stacked fetch arm (`src/shaderc/online/powervr/select.rs:1466-1506`)
  were right. What failed was the filtered look-up.
- **On 3D textures, only the 8 at level 0 pass** (`sampler3d_fixed`,
  `usampler3d`: `lod` 0). The 8 at level 1 (`sampler3d_float`) and level 2
  (`isampler3d`) fail. That is V-FETCH: the grid's fetch reads level 0
  whatever the level.

Every dEQP sampler here is `REPEAT_GL` on all three axes
(`samplerNearestNoMipmap` ... `samplerTexelFetch`, `:1786-1816`). The
look-up coordinates leave [0, 1]:

- 2D array: `(-1.2, -0.4, -0.5)` to `(1.5, 2.3, 3.5)`.
- 3D: `(-1.2, -1.4, 0.1)` to `(1.5, 2.3, 2.3)`.
- `textureProj` on 3D: `(0.9, 1.05, -0.08) / -0.75` and its far corner.

So a path that clamps a coordinate instead of wrapping it fails every
filtered case, with or without mipmaps. Fetches carry integer coordinates
and no wrap, so they are not affected. All 188 first messages are `Fail`
(an image comparison, `glsShaderRenderCase`) or "Got unexpected texture
size". None is a compile or link refusal, so at `ad2b5ed` every look-up was
compiled and drew a wrong picture.

## 4. Each sub-cause

### 4.1 A-WRAP (12) and A-MIP (39): fixed at `02c5c9af`

**Cause at `ad2b5ed`.** A `sampler2DArray` was the stacked 2D image (layers
one above another), sampled through `array_uv` (`src/shaderc/online/powervr/select.rs:4210-4262`;
stacked look-up arm `:1812-1847`). That function:

- rounds the layer as GL does;
- clamps `v` into `[0.5 / h, 1 - 0.5 / h]` of level 0's layer height;
- maps `v` into the layer.

So REPEAT on T is lost for every case: `y` runs from -0.4 to 2.3. At
levels above 0 the bilinear footprint also reaches into the next layer,
because the clamp is level 0's half texel (`docs/es3/pvr-2darray.md`
section 1).

**Why `02c5c9af` fixes it.** `PVR_NATIVE_ARRAY_DEFAULT` is 1
(`src/shaderc/mod.rs:314`). A 2D-array look-up is now Mesa pco's form
(`src/shaderc/online/powervr/select.rs:1397-1460`):

- an ordinary 2D `smp` over one layer, reached through the texture address
  override (`tao`, `array_layer_addr` `:4270`);
- the TPU wraps S and T within the layer, picks the level and filters,
  under the sampler's own address modes;
- offsets are the lookup word (`:1438-1442`), gradients are
  `explicit_gradients` (`:1426-1429`), integer samplers use the 2D
  `smp_type`.

The record sets bit 1, and the draw binds the twiddled layer-major copy
(`src/gl/draw.rs:5141-5158`). dEQP's arrays are 128x128x4 (POT), so the GL
core holds the whole chain and the copy carries it. K4, the NPOT array
limit, does not apply here (`docs/notes/pvr-npot-array-levels.md`
section 1).

**Host evidence at the tip** (nothing renders on the host; these are words,
records and acceptance):

- `tests/es3_pvr_texfn.rs`: every texture_functions look-up shape compiles,
  links and draws with each switch on and off.
- `tests/pvr_array_native_cases.rs`: a 2D-array look-up at a level above 0
  binds the twiddled copy and reads its layer, in both stages.
- `tests/es3_pvr_2darray.rs`: a 64x32x3, 7-level array reads each LOD from
  its own level.

**Board confirmation**: row 28 at `472630df` (board-c), which includes
`02c5c9af`.

**Caveat (vertex `texture()`).** The native arm turns an implicit level
into `lodm` bias 0 with `pplod` (`:1435-1437`), as pco does. The vertex
stage has no derivatives, so it reads level 0, which is dEQP's reference
for a vertex `texture()`. The T-VERTEX host evidence covers vertex
`textureLod`, not vertex `texture()`. If the board shows the 8 vertex
A-WRAP cases (`texture` and `textureOffset`, the only vertex array forms
with an implicit level) still failing, this is the first thing to look at.

### 4.2 A-SHADOW (5): fixed at `02c5c9af` (the look-up)

The cases are `texture.sampler2darrayshadow_fragment` and
`texturegrad`/`texturegradoffset.sampler2darrayshadow_{fragment,vertex}`.
They use a 128x128x4 `DEPTH_COMPONENT16` array with 8 levels,
NEAREST_MIPMAP_NEAREST and `COMPARE_REF_TO_TEXTURE` LESS (`:1863-1866`,
`:1807-1810`).

At `ad2b5ed` the plain depth look-up inside the shadow arm went through
`array_uv`, so it had the A-* cause. At the tip, the shadow arm
(`src/shaderc/online/powervr/select.rs:1931-1984`) samples the depth through the native-array arm, and
its PCF taps use the layer's address (`:1961-1965`). `pcf_taps` takes
`textureGrad`'s gradients for the level (`pcf_lambda`). So the
wrap and level cause is gone.

What is left for these 5 is the compare itself: PCF weights and the
NEAREST_MIPMAP_NEAREST level pick against dEQP's reference. That is
T-SHADOW's, the subject of item design-shadow. If the board still fails
them, they move to T-SHADOW's sub-cause there, not back here.

### 4.3 TS-2D (10): fixed at `4c0bcd77`

`4c0bcd77` ("pvr: a 2D texture with GL_TEXTURE_BASE_LEVEL above 0 samples
and sizes from the base level"). `pvr::texture::bind_lod` (`src/pvr/texture.rs:97-103`)
now passes the bind's base level for a plain 2D bind, and
`rebase_stride_bind` (`:160`) restates the linear image from it.
`textureSize` then answers level `base + lod` (the compiler's `tex_size_2d`,
`src/shaderc/online/powervr/select.rs:3980`). Host test: `tests/pvr_texsize_base_level.rs`, which
checks dEQP's three "with lod base" rows for sampler2D and isampler2D.

### 4.4 V-WRAP (19), V-MIP (67), V-FETCH (8), TS-3D (8): one fix, the native 3D default

**Cause (unchanged at the default).** A 3D texture is a grid of slices
sampled as one 2D image (`docs/notes/pvr-3d-native.md`, "The defect"):

- Look-ups (`src/shaderc/online/powervr/select.rs:1678-1760`, `vol_uv` `:4434`) mix the two slices
  around `c.z`. `(u, v)` is clamped into the slice and the slice index into
  `0..layers-1`, so REPEAT is lost on every axis. That is V-WRAP. The TPU
  never sees more than level 0, so no mipmap filter, bias, explicit level
  or gradient selects a level. That is V-MIP. It is the same cause as K5
  (`docs/es3/texture-failure-map.md` 2.2.1) and the 3D rows of T-FILTER and
  T-MIPMAP.
- `texelFetch(Offset)` (`src/shaderc/online/powervr/select.rs:1616-1660`) addresses level 0 "whatever
  `lod`": the replaced level is `Imm(0.0)`, `:1658`. That is V-FETCH.
- `textureSize` (`src/shaderc/online/powervr/select.rs:2093-2110`) halves level 0's slice size `lod`
  times. Since `1727ffe4` this is right at every level from base 0 (the
  grid rows of `tests/es3_pvr_texsize.rs`). But the GL core does not apply
  `GL_TEXTURE_BASE_LEVEL` to the grid: `bind_lod` returns
  `TexLod::DEFAULT` for `GLES_TEXTYPE_3D`, `src/pvr/texture.rs:98-100`.
  So dEQP's three "with lod base" rows (`:1408-1411`) answer level `lod`'s
  size instead of level `lod + 1`'s. That is TS-3D. A case fails if any one
  row is wrong.

**What native 3D does at the tip** (`pvr_set_native_3d(1)`;
`CONFIG_DEQP_PVR_NATIVE_3D=1` on the board):

- **Look-ups** (`src/shaderc/online/powervr/select.rs:1526-1552`): one `smp` with `dmn` 3d over a
  `TEXTYPE_3D` twiddled copy, so the TPU picks the level and wraps every
  axis. `SAMPLER_WORD0.addrmode_w` comes from `GL_TEXTURE_WRAP_R`
  (`src/pvr/backend.rs:7278`). The arm takes every return type (F32, I32,
  U32) and every level form (implicit, bias, `Lod`, `Grad` through
  `explicit_gradients(.., 3)`), `proj` (`tex_coords(.., proj)`) and a
  constant offset with all three fields (`lookup_word_3d`). That covers all
  12 look-up functions of sections 2.2 and V-WRAP and V-MIP.
- **Fetch** (`src/shaderc/online/powervr/select.rs:1564-1593`): fetches at the texel's centre at the
  given level, and adds the offset first. That covers V-FETCH.
- **Size** (`src/shaderc/online/powervr/select.rs:1599-1615`): reads the copy's image word. The copy
  is the volume from the base level (`gl::native_vol::view`,
  `src/gl/native_vol.rs:188-213`, from `e90122b8`), with the base level's
  extent even when a level is not held. Its descriptor has baselevel 0.
  So `textureSize(s, lod)` is level `base + lod`'s size: (6, 1, 1),
  (8, 4, 2) and (16, 16, 16) for the three rows. That covers TS-3D.
- **Bind** (`src/gl/draw.rs:5211-5233`): binds the copy for a program whose
  record sets bit 2, with `mip_levels` from `native_vol::sampled_levels`
  (7 for dEQP's `tex3DMipmap*`, 1 for `tex3DFixed`).

**Fix design.**

1. Set `PVR_NATIVE_3D_DEFAULT` to 1 (`src/shaderc/mod.rs:342`).
2. Update its doc comment (`:329-337`, "default OFF until a board run says
   otherwise") and the `DEQP_PVR_NATIVE_3D` help text in
   `driver/samples/zephyr/deqp/Kconfig` (the "-1 leaves the library's
   default (off ...)" line).
3. `src/shaderc/precompiled.rs:165` uses the table only while the switch
   is at its default. Flipping the default moves the pinned references and
   fixtures that item n3d-flip-impact listed. They are regenerated in the
   same commit (CLAUDE.md, generated files).

The flip lives in `src/shaderc/mod.rs`, outside `src/pvr/**` and
`src/shaderc/online/powervr/**`. Whether to flip is item n3d-default-decide's
decision, and the measurement that settles it is item board-n3d (rows 22-40
with `DEQP_PVR_NATIVE_3D=1`; row 28 is these cases). Fixing the grid
instead is not designed: `docs/notes/pvr-3d-native.md` ("The choice") gives
the reasons the grid cannot give the TPU its chain or its wrap.

Open points for the board arm (from `docs/notes/pvr-3d-native.md`, "What is
not verified"):

- The fetch's `+ 0.5` centre on all three axes has not been checked
  against Mesa's 3D `txf`. If the 8 V-FETCH cases alone fail with the
  switch on, this is the place to look.
- The per-level twiddle of a POT volume is not in doubt here: dEQP's
  volumes are 64x32x32, POT.

**Host test to add** (`tests/pvr_texfn_3d_default.rs`; features
`host-test,gpu-powervr,shaderc-online`). It uses the GL-API harness of
`tests/pvr_native3d_sampler_words.rs`: a stand-in back end that packs the
bind with `pvr::backend::texture_descriptor`. The switch stays at its
**default** (no `pvr_set_native_3d` call).

1. For each 3D shape dEQP uses (`texture`, with bias, `textureOffset`,
   `textureProj`, `textureProjOffset`, `textureLod`, `textureLodOffset`,
   `textureProjLod`, `textureProjLodOffset`, `textureGrad`,
   `textureGradOffset`, `textureProjGrad`, `textureProjGradOffset`,
   `texelFetch`, `texelFetchOffset`, `textureSize`) and for each of
   `sampler3D`, `isampler3D` and `usampler3D`, in both stages: the record
   says `native_cube & 4`. The decoded group is one `smp` with `dmn` 3d,
   not the grid's two samples and mix.
2. A draw with dEQP's `tex3DMipmapFixed` (64x32x32 RGBA8, 7 levels,
   LINEAR_MIPMAP_NEAREST, REPEAT) binds `GLES_TEXTYPE_3D_TWIDDLED` with 7
   levels, a mipmap filter and `addrmode_w` REPEAT. `tex3DFixed` (1 level,
   LINEAR) binds 1 level with REPEAT on all three axes.
3. dEQP's three "with lod base" rows on a `sampler3D` (glTexStorage3D,
   `lod + 2` levels, NEAREST, `GL_TEXTURE_BASE_LEVEL` 1): the descriptor's
   extent is level 1's. `textureSize` in the byte model (`tests/usc_exec`,
   as `tests/es3_pvr_texsize.rs` runs it) answers (6, 1, 1), (8, 4, 2) and
   (16, 16, 16).

Parts 1-3 fail at `5f781c49` and pass after the flip. A byte-model check
that `texelFetch(p, 1)` reads level 1's texel needs a 3D volume model in
`tests/usc_exec`, which today models `dmn` 3d as the native cube
(`tests/usc_exec/mod.rs:960-963`). That model is a follow-up, not part of
this test.

### 4.5 TS-CUBE (10): the `src/gl` change of shader-failure-map.md section 7

**Cause.** The native cube is the default (`src/shaderc/mod.rs:282`). A
textureSize-only program sets record bit 0. Under NEAREST the draw asks for
a one-level copy (`src/gl/draw.rs:5115-5117`, `single`), and
`native_cube::copy` (`src/gl/native_cube.rs:181`) copies **level 0**. The
bind's extent is level 0's (`src/gl/draw.rs:5122-5124`). The back end names a base
level only for a mip-mapped copy, so `textureSize` at base level 1 answers
level `lod`'s size, not level `lod + 1`'s. Only dEQP's `64x64x64, lod 1,
base 1` row reaches a cube (`docs/es3/shader-failure-map.md` 7.2), and it
expects (16, 16).

**Fix design.** The patch of `docs/es3/shader-failure-map.md` section 7.4,
unchanged:

- `native_cube::copy_from(t, target_kick, single, first)`: the one-level
  copy holds level `first`, and `copy` becomes `copy_from(.., 0)`.
- In `src/gl/draw.rs`, the native-cube site passes
  `first = bind_base_level(t)` when `single`, sets the extent to that
  level's and the base level to 0. A stacked one-level cube or array is
  restated by `rebase_stacked_bind`.

That patch was written against `bdd2ba1c`. At `5f781c49` the sites have
moved: the cube copy is `src/gl/draw.rs:5115-5125`, the array copy
`src/gl/draw.rs:5146-5156`, and `tb.base_level = bind_base_level(t)` is
`src/gl/draw.rs:5240`. Its hunk headers name the places, so it applies by place.
Host test: section 7.6 there (part 1, the native cube 64x64 RGBA8 at base
level 1; part 2, the sizes in `tests/es3_pvr_texsize.rs`).

### 4.6 TS-ARRAY (10): section 7 **and** K4

**Cause.** This is the same as the cube's, on the native array, which is the
default since `02c5c9af`. `native_array::copy`
(`src/gl/native_array.rs:176`) copies level 0 for a one-level draw
(`src/gl/draw.rs:5146-5148`), so `textureSize` is one level too big in all three
"with lod base" rows.

**Finding.** Section 7 alone does not fix the `100x31x18, lod 3, base 1`
row, which expects (6, 1, 18). That row is a NPOT array. With native arrays
on, the GL core keeps one level of a NPOT array (`npot_mip_chains`,
`src/gl/texture.rs:1308-1310`, false for more than one layer; cause K4,
`docs/notes/pvr-npot-array-levels.md` sections 1-2), so
`level_held(t, 1)` (`src/gl/texture.rs:1767-1773`) is false. Section 7.4's
`native_array::copy_from` then falls back to `first = 0`, and the bind's
extent is `t.width >> 0`. The answer is level 3's (12, 3, 18), not
(6, 1, 18). Only section 7's *stacked* arm, `rebase_stacked_bind`, keeps
"the extent alone" for a level the storage does not hold. That arm is not
taken while native arrays are the default.

**Fix design.** Apply both `src/gl` changes:

1. K4, `docs/notes/pvr-npot-array-levels.md` section 4 (or section 5 if the
   K3 cube patch has landed first). With it, the NPOT array holds its chain
   and `level_held(t, 1)` is true.
2. Section 7.4 of `docs/es3/shader-failure-map.md`. With K4 in, its
   `copy_from(.., first = 1)` copies level 1 (50x15x18), and `textureSize`
   at `lod` 3 is (6, 1, 18).

Order: K4 first, or both in one commit. If section 7 lands alone, 8 of the
10 TS-ARRAY rows' textures are right but every case still fails on the NPOT
row. There is an alternative, for when K4 is delayed: in section 7.4's
native-array branch, set `tb.width`/`tb.height` from `bind_base_level(t)`
even when `copy_from` returns `first = 0`. That is not recommended. It
describes a level-0 copy with a smaller extent, which is wrong for any
look-up of that texture, not only for `textureSize`.

**Host test.**

- Section 7.6 part 1, "native array 128x64x32" and "stacked array
  100x31x18" (there).
- K4's `tests/pvr_npot_array_chain.rs` (its section 6).
- One more row in section 7.6 part 1: a **native** 100x31x18 RGBA8 array,
  glTexStorage3D with 5 levels, NEAREST, base level 1. The bind is
  `GLES_TEXTYPE_2D_ARRAY_TWIDDLED`, `width 50`, `height 15`, 18 layers,
  `base_level 0`. Part 2 in `tests/es3_pvr_texsize.rs` answers (6, 1, 18).
  This row fails with section 7 alone and passes with K4 + section 7.

## 5. `src/gl` changes, named exactly

No sub-cause needs a change in `src/pvr/**`, `src/shaderc/online/powervr/**`
or `driver/src/pvr/*`. The open ones need:

| change | where it is designed | sub-causes | cases |
|---|---|---|---:|
| the one-level cube/array bind restated from `GL_TEXTURE_BASE_LEVEL`: `gl::native_cube::copy_from`, `gl::native_array::copy_from`, `gl::draw::rebase_stacked_bind` and the three call-site edits | `docs/es3/shader-failure-map.md` section 7.4 (item texfn-gl-design) | TS-CUBE, TS-ARRAY | 20 |
| a NPOT 2D array keeps every level (`npot_mip_chains` and the storage paths) | `docs/notes/pvr-npot-array-levels.md` section 4 / 5 (item k4-array-design) | TS-ARRAY (its 100x31x18 row) | (the same 10) |

The 3D sub-causes need one line outside `src/gl`:
`src/shaderc/mod.rs:342`, `PVR_NATIVE_3D_DEFAULT` 0 -> 1, with its
regenerated pins (section 4.4).

## 6. Not verified

- Nothing here ran on a board. "Fixed at" means the default path at the
  tip compiles and binds the form that answers the case, as read in the code
  and pinned by the host tests named. The pictures are the board's to show.
- The counts are `ad2b5ed`'s. Board-c's row 28 at `472630df` should show
  A-WRAP, A-MIP, A-SHADOW and TS-2D (66) gone and the other 122 still
  failing. A case that does not follow this needs a new look, not a
  re-label.
- No host dEQP run informs the pictures: the Linux OSAL never runs the
  command stream (`docs/notes/pvr-3d-native.md`, the host dEQP sections).
