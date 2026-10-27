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
