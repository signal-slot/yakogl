<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: 3D textures as the TPU's own type (`pvr_native_3d`)

Roadmap stage 4 (ES 3.0, host first), plans #147 and #748. Code:
`src/gl/native_vol.rs`, the `Sample`/`SampleZ { vol }` arms in
`src/shaderc/online/powervr/select.rs`, `pvr/backend.rs` (the
`GLES_TEXTYPE_3D_TWIDDLED` descriptor). Tests: `tests/es3_pvr_3d.rs`,
`gl::native_vol::tests`, `gl::draw::tests::a_native_3d_program_binds_*`,
`pvr::backend::tests::a_3d_descriptor_*`.

## The defect

A 3D texture was a grid of slices, sampled as one 2D image with level 0
only (`texture_alloc_storage_3d`; `Sel::vol_uv` picks the two slices around
the depth and mixes them). Two things follow, and they are why almost every
dEQP-GLES3 `texture.filtering.3d` and `texture.mipmap.3d` case fails on the
AM62P (`docs/notes/pvr-3d-mipmap-hang.md`, section 3):

- the TPU never sees more than level 0, so no mipmap filter applies;
- `vol_uv` clamps `(u, v)` into the slice and the slice index into
  `0..layers-1`, so REPEAT (and MIRRORED_REPEAT) is lost on all three axes.

## The choice: native 3D textures

Two ways were open: mip levels and wrap modes in the grid emulation, or the
TPU's `TEXTYPE_3D`. This is native 3D, for these reasons.

1. **The grid cannot give the TPU its mip chain.** Level l of a grid
   holds `depth >> l` slices in `ceil((depth >> l) / cols)` rows, but the
   TPU walks a STRIDE chain as `(gw >> l) x (gh >> l)` at a power-of-two
   arithmetic of its own, which is not where those grids are (the note's
   section 2: the walk "has no power-of-two chain the TPU could walk"). So
   the shader would have to pick the level itself and address the level's
   grid from a base the descriptor does not carry: a second descriptor per
   level, or the level offsets in a uniform plus a hand-written LOD from
   derivatives. That is a new sampling path in the shader for every form
   (implicit, bias, explicit, fetch, size) and for the vertex stage.
2. **Wrap modes in the shader are as much work, and slower.** Each tap
   needs an explicit `mod` of the coordinate into the slice, which breaks
   the TPU's bilinear footprint at the slice edge (the footprint no longer
   wraps; the shader would need four taps and its own weights, per slice,
   per level, twice for trilinear in `w`).
3. **The TPU does all of it for a real 3D image,** and Mesa's pvr does
   exactly this for `VK_IMAGE_TYPE_3D` (`PVR_MEMLAYOUT_3DTWIDDLED`,
   `pvr_image.c:58-59`; `TEXTYPE_3D`, `pvr_arch_tex_state.c:170-176`): the
   mip level, trilinear across slices, and the wrap mode per axis from the
   sampler (`addrmode_w`). The shader is one `smp` with `dmn` 3d, the form
   the native cube already uses.
4. **The precedent is in the tree.** `gl::native_cube` and `gl::native_array`
   keep the GL storage as it is and make a twiddled copy for the TPU,
   remade when the texels change, behind a compile-time switch. Native 3D is
   the same shape, so the storage (and every non-PowerVR backend, which reads
   the grid or the stack) is untouched.

Costs accepted: a copy of the whole volume with its chain (twiddling is
`O(texels)` on the CPU, repeated after every texel write or render into the
volume), and a layout that rests on Mesa's source rather than on a board run
yet (see "What is not verified").

## The layout (Mesa's, as `native_vol.rs` writes it)

- Level 0's physical extent is each axis rounded up to a power of two
  (`pvr_image_plane_init_physical_extent`, `pvr_image.c:83-92`); 3x7x5 is
  laid out as 4x8x8. The descriptor still carries the API size (word 0
  `width - 1`, `height - 1`; word 1 `depth - 1`), as Mesa's image view does.
- Level l is the extent halved l times, at least 1; it takes
  `align(w, 4) * align(h, 4) * align(d, 4)` texels (`pvr_image.c:146-171`,
  `extent_alignment` 4 for a 3D image); levels follow each other. The
  chain runs to 1x1x1 when more than one level is sampled, so a mip walk
  stays inside the allocation.
- A texel's index inside its level is `usclib_twiddle3d`
  (`pco/usclib/tex.cl:152-212`): 4x4x4 cubes in an interleave of their
  coordinate bits (y, then x, then z each round, an axis dropping out when
  its cubes are used up), 16 texels per slice inside a cube, 2x2 quads
  inside a slice. `native_vol::twiddle3d` is a transcription; the tests pin
  hand-worked values and that it is a bijection.
- Descriptor: `TEXTYPE_3D`, the twiddled `IMAGE_WORD1` (levels, `depth`,
  `mipmaps_present`), and `SAMPLER_WORD0.addrmode_w` from `GL_TEXTURE_WRAP_R`
  (it was a fixed CLAMP_TO_EDGE, since the third axis was the shader's).

A draw that samples one level (no mipmap filter, or a one-level chain) gets
a one-level copy described as one level, the rule the cube and the array
learned on the board (`3b634e98`). The GL core keeps `levels` at 1 for a 3D
grid, so `native_vol::sampled_levels` counts the defined levels from the
per-level record, capped by `GL_TEXTURE_MAX_LEVEL`, and the draw sets the
binding's `mip_levels` / `mip_filter` from it (after a sampler object's
filters are in force).

## The shader forms

With `pvr_native_3d` on: `texture`, `texture` with bias, `textureLod`,
`textureProj`, `texelFetch` (centre of the texel, level replaced) and
`textureSize` (width and height at the level, depth from the image word's
`depth + 1` shifted by the level). The record says `native_cube` bit 2; the
draw binds the copy only for such a program (`sh.native_cube & 4`). Not
native yet: gradients (`textureGrad`), offsets, integer and shadow 3D
samplers; they take the existing grid arms or the existing refusal.

## Switch and default

`pvr_native_3d` (`pvr_set_native_3d`, `CONFIG_DEQP_PVR_NATIVE_3D`), default
OFF, read when a shader compiles; the precompiled table is used only while it
stands at its default. Every `report-es2` answer is therefore unchanged:
nothing in the default path moved, and the ES 2.0 build has no 3D entry
points that reach it.

## What is not verified

No board ran this. The host tests pin the layout arithmetic, the record, the
descriptor words and the draw's binding. Open questions for the first AM62P
run (`texture.mipmap.3d`, `texture.filtering.3d`, one image per arm of
`CONFIG_DEQP_PVR_NATIVE_3D`):

- whether the TPU's per-level twiddle for an NPOT volume uses the physical
  (power-of-two) level extent, as `native_vol` assumes, or the API extent
  halved: they differ for a level whose API side rounds to a smaller cube
  count (63x57x67 at level 1). `3d.sizes` is the case that tells.
- `texelFetch` on a 3D image: the `+ 0.5` centre convention is the 2D and the
  array's; Mesa's 3D fetch path is not read here.
- the cost: dEQP's 64x64x64 cases copy 262,144 texels per level-0 write.

On a wrong answer set the switch to 0: the grid path is unchanged.

## Host dEQP of the 3D texture groups, native 3D off (2026-11-05)

Measured at `f3ec9117` (the HEAD of the workspace), GPU powervr, dEQP-GLES3,
VK-GL-CTS `93bca01` with the yakogl target, native 3D off (the default). The
commands, one per group, from the repository root:

```sh
tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3 --cts=<VK-GL-CTS> dEQP-GLES3.functional.texture.filtering.3d.
tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3 --cts=<VK-GL-CTS> --out=<dir> dEQP-GLES3.functional.texture.mipmap.3d.
```

| group | cases | Pass | Fail | NotSupported | QualityWarning | crash | wall | sum of case times |
|---|---|---|---|---|---|---|---|---|
| `texture.filtering.3d` | 153 | 0 | 141 | 12 | 0 | 0 | 4.6 s | 6.3 s |
| `texture.mipmap.3d` | 64 | 0 | 64 | 0 | 0 | 0 | 1.0 s | 1.0 s |

Wall is from the case list to the last chunk log with the default 2 parallel
processes of a 4-core host (the library and dEQP were already built). Every
Fail is "Image verification failed": this is the Linux OSAL, which keeps the
command stream and never runs it, so `glReadPixels` reads black and no render
verdict here means anything (the script's HOST-ONLY rule). The 12
NotSupported are all in `filtering.3d` (not looked into further).

Slowest cases (dEQP's own TestDuration):

| case | time |
|---|---|
| `filtering.3d.formats.rgba4_linear_mipmap_linear` | 0.149 s |
| `filtering.3d.formats.rgba8_snorm_linear_mipmap_linear` | 0.126 s |
| `filtering.3d.formats.r11f_g11f_b10f_linear_mipmap_linear` | 0.103 s |
| `filtering.3d.formats.rgb9_e5_linear_mipmap_linear` | 0.101 s |
| `filtering.3d.formats.rgba8_snorm_linear` | 0.094 s |
| `mipmap.3d.projected.linear_linear_clamp` | 0.036 s |
| `mipmap.3d.projected.linear_linear_mirror` | 0.036 s |
| `mipmap.3d.projected.linear_linear_repeat` | 0.034 s |

What this does and does not show for issue #2 (images 20 and 21): the host
does not reproduce the 9 to 19 s per `linear_mipmap_linear` case. Here the
slowest case is 0.15 s, the black readback is the likely reason: it probably makes
dEQP's picture comparison fail at once (not verified). The
9 to 19 s that the note above attributes to dEQP checking each wrong picture
is the cost of a picture that is wrong but not black, so it needs a host that
renders (the native-3D-on arm, `host-3d`) or the board. This measurement is
only the host baseline: zero crashes, all 217 cases began, no chunk was
resumed.

## Host dEQP of the 3D texture groups, native 3D on (2026-11-05)

Measured at `e179ad48` (the HEAD of the workspace), GPU powervr, dEQP-GLES3,
VK-GL-CTS `93bca01` with the yakogl target, native 3D on through the host
knob `YAKOGL_PVR_NATIVE_3D=1` (`host-deqp-run.sh --native-3d=1`; the off-arm
above is the default, same script without the flag). The commands, one per
group, from the repository root:

```sh
tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3 --native-3d=1 --cts=<VK-GL-CTS> --out=<dir> dEQP-GLES3.functional.texture.filtering.3d.
tools/cts/host-deqp-run.sh --gpu=powervr --module=gles3 --native-3d=1 --cts=<VK-GL-CTS> --out=<dir> dEQP-GLES3.functional.texture.mipmap.3d.
```

| group | cases | Pass | Fail | NotSupported | crash | wall | sum of case times |
|---|---|---|---|---|---|---|---|
| `texture.filtering.3d` | 153 | 0 | 141 | 12 | 0 | 9 s | 8.7 s |
| `texture.mipmap.3d` | 64 | 0 | 64 | 0 | 0 | 3 s | 1.3 s |

Wall is the whole script call (library and dEQP already built, 2 parallel
processes), so it includes process start-up; the off-arm's wall was counted
from the case list to the last chunk log and is not comparable to the second.
Every Fail is "Image verification failed" (the Linux OSAL never runs the
command stream, so `glReadPixels` reads black: HOST-ONLY, no render verdict
means anything). All 217 cases began, no refusal carried a `0:<line>:` reason.

The 3D `linear_mipmap_linear` cases that fail, all with the first message
"Image verification failed" (9 `formats`, 4 `sizes`):

- `filtering.3d.formats.{r11f_g11f_b10f,rgb10_a2,rgb565,rgb5_a1,rgb9_e5,rgba4,rgba8,rgba8_snorm,srgb8_alpha8}_linear_mipmap_linear`
- `filtering.3d.sizes.{128x32x64,32x64x16,3x7x5,4x8x8}_linear_mipmap_linear`

`mipmap.3d` has no case named `linear_mipmap_linear`; its 64 cases (affine,
projected, ... by filter and wrap) all fail the same way.

Against the off-arm: the tally is identical (141/12 and 64/64, zero Pass) and
the sum of case times is about 40% higher in `filtering.3d` (8.7 s against
6.3 s), so on the host the switch changes neither the verdicts nor the
runtime in a way that matters; only the board, which renders, can show
whether the picture is now right.
