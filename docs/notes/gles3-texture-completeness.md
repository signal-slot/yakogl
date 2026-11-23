<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GLES 3.0 texture completeness with GL_TEXTURE_BASE_LEVEL and GL_TEXTURE_MAX_LEVEL

The rules the planned `basecompl` change implements in `src/gl/draw.rs`,
with where each comes from. Spec is OpenGL ES 3.0.6, 3.8.13 "Texture
Completeness" and 3.8.10 "Texture Minification" (the mipmapping rules, with
the level-of-detail clamps to the base and max levels). Mesa is 26.2.1,
`src/mesa/main/texobj.c` (`_mesa_test_texobj_completeness`) and the state
tracker `src/mesa/state_tracker/st_atom_texture.c` (the sampler view). None of those exist under `third_party/mesa-26.2.1/` (it holds
`src/broadcom`, `src/gallium` pieces, `src/imagination`, `src/panfrost`,
`src/util`, `src/compiler`, `src/c11`, no `src/mesa`); they are cited by
their upstream tree path. They were not re-read for this note (no copy is
vendored); check the function names against the 26.2.1 tree before relying
on a detail. Line numbers
drift; the names are the address.

## What the tip does today

`src/gl/draw.rs` ignores `GL_TEXTURE_BASE_LEVEL` in its completeness check.
`incomplete_reason` (`draw.rs:3630`) tests level 0: `level_defined & 1 == 0`
(`draw.rs:3637`); the chain it demands under a mipmap filter is
`(1 << needed) - 1` from bit 0 (`draw.rs:3661`) and a cube's faces are
checked from level 1 up (`draw.rs:3669`); `mip_levels_needed`
(`draw.rs:3618`) counts from level 0 and says so in its comment, "the base
level is level 0 here: the draw does not apply GL_TEXTURE_BASE_LEVEL
(`GlTexture::base_level`)". The field exists (`src/gl/context.rs:534`) and
is only read by the native-volume and transform-feedback code.

## The rules

Write B for the base level, M for `GL_TEXTURE_MAX_LEVEL`, p for the last
level of the chain the base image's size allows (`floor(log2(max side)) +
B`, the depth counting for a 3D texture), and L for an immutable texture's
level count.

1. **Mutable: completeness starts at B.** The "levels" of 3.8.13 are
   numbered from the base level: the base image must exist, and with a
   mipmap min filter every level B..q with q = min(p, M) must exist, with
   the size the chain gives (half the previous, floor, minimum 1) and the
   base's internal format. Level 0 is not tested when B > 0.
2. **A base level without an image is incomplete**, whatever the filter
   (the array of base images is the first thing 3.8.13 asks for). Zero
   width, height or depth counts as no image.
3. **M < B** is incomplete **only with a mipmap min filter**. 3.8.13 gives
   q = min(p, M) < B and the required set is empty or negative, which the
   spec words as "levelBase to q must be consistent", i.e. incomplete.
   With NEAREST or LINEAR the texture samples the base level alone
   (3.8.10: the filter reads the base level, the max only clamps the
   mipmap lod range), as Mesa does (`texobj.c`: the non-mipmap
   branch tests only `BaseLevel`, `_MaxLevel` is not consulted) and as
   dEQP's reference sampler does (`tcuTexture`, `Sampler`-with-no-mip
   reads `baseLevel`).
4. **Immutable (glTexStorage*): clamp, not test.** 3.8.13 (and 3.8.4 for
   the immutable-format clamp): base = clamp(B, 0, L-1), max = clamp(M,
   base, L-1). So an immutable texture with B >= L reads its last level,
   and with M < B reads the base level; neither is incomplete by itself.
   Completeness of an immutable texture is then only the format/filter
   rules (integer or 32-bit float needs NEAREST) and a cube's face rule.
5. **A cube map is complete only when its faces are complete from the
   base level**: all six faces hold level B (square, one size, one format),
   and with a mipmap filter every face holds B..q. The check at the tip
   starts the face loop at level 1 (`level_faces[1..levels]`), which is
   the off-by-one the `basecompldry` dry run found: with B > 0 the loop
   must start at B (`B.max(1)` is wrong in both directions; level B, even
   when B = 0, is covered by the base-image rule, so the loop is
   `B+1..=q` after a `level_faces[B] == 0x3f` test). Level 0 faces are
   irrelevant when B > 0.
6. **A texture whose base is out of range is incomplete**: B above the
   last level the base image's size can have for a mutable texture is
   not an error of the parameter (`glTexParameter` accepts any B >= 0)
   but no image exists at B, so rule 2 applies. (Immutable textures clamp
   by rule 4 instead.)

## What Mesa hands the driver

`_mesa_test_texobj_completeness` sets `t->_BaseComplete` and
`t->_MipmapComplete`; `_mesa_update_texture_object`/the finalize step
computes `t->_MaxLevel = min(MaxLevel, BaseLevel + chain(base image) - 1)`
and, for an immutable texture, `min(_MaxLevel, Immutable levels - 1)` (with
`BaseLevel` clamped as rule 4). The state tracker
(`src/mesa/state_tracker/st_atom_texture.c`, `st_create_texture_sampler_view`
and `get_texture_format`/`st_finalize_texture`) then makes the sampler view
with `u.tex.first_level = BaseLevel` and `u.tex.last_level = min(_MaxLevel,
storage last level)` (also clamped by the resource's `last_level`). The
V3D driver's `v3dx_state.c` turns that pair into the TMU's base level and
max level, and `v3d_uniforms.c` writes words 44-47 from `first_level`
(`docs/notes/v3d-texture-size-words.md` (e) and `basefix`).

## For the implementation

* `mip_levels_needed` and `incomplete_reason` take B (and the immutable
  clamp of rule 4) and test bits `B..B+n` of `level_defined`, `level_bad`
  and `level_faces`, instead of `0..n`.
* The same B and q go to the backend (`GlesTextureBind::base_level`,
  `basefield`), which is what the TMU base level and the size words need.
* Pins that flip with it are the `GL_TEXTURE_BASE_LEVEL` pins in `tests/`
  (`flipmap`, `flipmap2` list them).
