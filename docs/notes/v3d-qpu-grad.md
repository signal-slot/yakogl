<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D QPU compiler: textureGrad lowered to an explicit level

Design, for the item `v3d-qpu-grad`. Nothing here is verified on hardware or
by a run: it is read from Mesa 26.2.1 and from our compiler at `ff7b9882`
(line numbers drift; the function names are the address). The V3D 4.2 TMU
has no gradient operand, so a `textureGrad` look-up is turned into the
look-up we already emit for `textureLod`: the shader computes the level from
the two gradients and the texture size, and the TMU is given it as an
explicit level. This note gives the arithmetic, the QPU operations and
uniforms, the place in the compiler, the precision, and the tests.

Mesa file names are relative to `src/` of the Mesa 26.2.1 tree
(`third_party/mesa-26.2.1` holds only headers and the QPU tables; the
compiler sources are in the full tree `tools/v3d/README.md` builds `v3d_shc`
from). The TMU words themselves (P0/P1/P2, the TMUB / TMUSLOD / TMUSCM
writes) are not repeated here; `docs/notes/v3d-tmu-es3.md` (item
`notes-v3d-tmu`) is not on main yet, and this note only uses the words that
the explicit-level look-up already has.

## Today

`prepare.rs` (`src/shaderc/online/v3d/prepare.rs`) refuses every look-up whose
`TexLod` is `Grad` with `NotYet("a texture look-up with gradients")`
(`prepare.rs:1364`, the last arm of the `match lod` that starts at `:1353`).
The front end is complete: `builtin_call.rs:698-702` turns `textureGrad`,
`textureGradOffset`, `textureProjGrad` and `textureProjGradOffset` into
`TexLod::Grad { ddx, ddy }`, each gradient read with the identity swizzle at
the coordinate's base width (`ir.rs:158-162`): 2 lanes for `D2` and
`D2Array`, 3 for `D3` and `Cube`; a layer or a shadow reference is not part of
a gradient. `eval.rs:723-736` reads them the same way for the host model.

The explicit-level look-up is already emitted: `TexLod::Lod` gives
`is_lod = true` and the level as the third (or, for a cube or array, fourth)
packed operand (`prepare.rs:1363`, `:1426-1440`), and `select.rs` writes it to
TMUB and retires on TMUSLOD for a 2D look-up, on TMUSCM with P2's
`disable_autolod` for a cube (`select.rs:901-935`). So the whole change is in
`prepare.rs`: `Grad` becomes `Lod(computed level)`. `select.rs` does not
change.

## (a) What Mesa does

v3d asks `nir_lower_tex` for `lower_txd = true` (`broadcom/compiler/vir.c:801-808`,
inside `v3d_lower_nir`), so every `textureGrad` (NIR `txd`), of every
dimension, shadow or not, array or not, is rewritten by `lower_gradient`
(`compiler/nir/nir_lower_tex.c:827`; the dispatch at `:1780-1791`) to a `txl`
(explicit level) look-up, and no gradient operand ever reaches the TMU
emitter (`v3d_tex.c` has a `bias` and a `lod` source, no `ddx`/`ddy`). The
pass runs after the projective lowering (`lower_txp = ~0`, `vir.c:807`;
`project_src`, `nir_lower_tex.c:54-75`, divides only the coordinate and the
comparator, `:1590-1612`): the gradients of `textureProjGrad` are used as
given. Offsets, the comparator and the layer stay on the new `txl`; the
final rewrite only removes the two gradient sources, adds the level and
changes the op (`replace_gradient_with_lod`, `:660-675`).

**2D, 2D array, 3D, 1D** (`nir_lower_tex.c:827-880`):

1. `size` is `textureSize(s, 0)`, converted to float, cut to the gradient's
   width: 1 lane for 1D, 3 for 3D, 2 for everything else, an array's layer
   count included in none (`:838-854`).
2. Each gradient is scaled lane by lane by `size`: the derivatives of the
   normalised coordinate become derivatives in texels of level 0 (GL ES 3.0
   section 3.8.10, `u(x, y) = w_t * s(x, y)`, the formulas 3.17-3.19; the
   comment at `:856-860`).
3. `rho` is the larger of the two lengths: for one lane the larger absolute
   value, otherwise the larger of `sqrt(dot(dPdx, dPdx))` and
   `sqrt(dot(dPdy, dPdy))` (`:868-875`). This is the exact norm, not the
   cheaper `max(|dudx|, |dvdx|, ...)` form the ES spec also allows. A 3D
   texture uses a three-lane dot, with no extra weight on r.
4. The level is `log2(rho)` (`:878`). No bias term (the comment says GL
   state biases are ignored) and no clamp: not to `[0, max level]`, not for
   `rho = 0`. ES 3.0 has no `textureGrad` with a bias or a `min_lod`, so the
   `nir_tex_src_min_lod` clamp in `replace_gradient_with_lod`
   (`:666-671`) never applies to us. The clamp to the texture's levels, its
   `TEXTURE_MIN_LOD`/`MAX_LOD` and `BASE_LEVEL`, is the TMU's, from the
   sampler and texture state it already has; a negative level selects the
   magnified filter, as for `textureLod`.

**Cube** (`lower_gradient_cube_map`, `:681-823`). The direction's gradient
has three lanes and the face selection must come first, so the pass:

1. takes `size.x` of `textureSize(s, 0)` as `L`, the face width (cube faces
   are square);
2. picks the major axis by comparisons in a fixed order, z first, then y,
   else x (`:763-764`: `cond_z = |z| >= max(|x|, |y|)`, then `cond_y`), and
   rotates the direction and both gradients so that the major axis is last
   (`Q`, `dQdx`, `dQdy`, `:769-790`);
3. applies the quotient rule to get the derivative of `Q.xy / Q.z` along
   each screen axis, dropping the sign of `Q.z` (the sign does not change a
   length): `recip * (dQ.xy - Q.xy * recip * dQ.z)` (`:793-807`);
4. takes `M`, the larger of the two squared lengths (`:809`), and returns
   `-1 + 0.5 * log2(L * L * M)` (`:812-821`), which is
   `log2(L / 2 * sqrt(M))`: the face is `[-1, 1]` wide, so its width in
   normalised units is 2.

The `+/-1` normalisation of the direction by its major axis (which yakogl
does at `prepare.rs:1391-1403` before the look-up) is a separate step in the
gallium driver (`nir_normalize_cubemap_coords`); the gradients are not
normalised and are used as given.

## (b) QPU operations and uniforms

Names are yakogl's. `AluOp` is the kernel-operation enum prepare.rs works
in; the QPU operation each becomes is in `select.rs`.

| need | prepare.rs form | QPU |
|---|---|---|
| the texture size, per lane | `SOp::TexSizeWord { sampler, lane }` then `AluOp::U2f` (as `cube_lod`, `prepare.rs:872-873`) | a uniform read of `QUNIFORM_TEXTURE_WIDTH + lane` with the sampler as data (`select.rs:1989-1991`); Mesa's `ntq_emit_txs` reads the same uniform (`broadcom/compiler/nir_to_vir.c:915-956`, the contents at `:929-934`) |
| scale, sum, max | `AluOp::Fmul`, `Fadd`, `Fmax` | FMUL, FADD, FMAX |
| length | `AluOp::Fsqrt` (or the fold below) | RSQRT then RECIP (`select.rs:1470-1474`, Mesa's `lower_fsqrt`) |
| the level | `AluOp::Flog2`, then an `Fmul` by 0.5 if the sqrt is folded | LOG (`select.rs:1469`) |
| a cube's axis choice | `AluOp::Bcsel` / `Fabs` / `Fmax`, or `CubeFace::is_x`, `is_y` | the same as `cube_lod` already selects |
| the look-up | unchanged: `SOp::Tex { lod: true, .. }` | TMUB write, TMUSLOD (2D) or P2 + TMUSCM (cube), `select.rs:901-935` |

Uniform kinds: `QUNIFORM_TEXTURE_WIDTH` lane 0 (width), 1 (height), and for a
3D texture lane 2 (depth: Mesa reads `WIDTH + 2`, `nir_to_vir.c:929-934`).
For a 2D array the size has only width and height in the gradient; the
`QUNIFORM_TEXTURE_ARRAY_SIZE` word of the third lane (`:930-931`) is not read.
No new uniform kind, no new TMU config word, and no extra texture-state field:
`GlesTextureBind` is not touched. The depth lane needs `TexSizeWord` to be
filled for a 3D texture, which is item `v3d-tex-3d-array`/`v3d-qpu-array-3d`'s
(today `Op::TexSize` refuses 3D and array, `prepare.rs:1460`); a `Grad` on a
3D or 2D-array sampler therefore needs that item's texture state before it can
be measured, but the arithmetic below does not depend on it.

Size: 2D, with Mesa's shape: 2 uniform reads, 2 `U2f`, 4 `Fmul` to scale,
2 `Fmul` + 1 `Fadd` per dot product (V3D has no fused multiply-add, so
`fdot` is mul/mul/add), 2 `Fsqrt` (4 SFU operations), 1 `Fmax`, 1 `Flog2`:
about 20 QPU operations plus the SFU waits (an estimate, to be replaced by
the `v3d_shc` count of a minimal program).

An allowed shortcut, and the one the cube path already takes
(`prepare.rs:877-880`, `0.5 * log2(rho^2)`): `log2(max(sqrt(a), sqrt(b)))` is
`0.5 * log2(max(a, b))`. It replaces four SFU operations by one `Fmul` and
is more precise (one approximation, not two or three); the price is an
instruction count below Mesa's, so it will not match a `qpu_counts_42.tsv`
row by equality (see (e)). Recommended: the fold for 2D, 3D and cube alike,
with the comparison to Mesa as "not more than". Open for the item: whether to
clamp the result with `clamp_lod` (`prepare.rs:884-890`, `[-16, 16]`, two more
operations). Mesa does not; `cube_lod` does because a constant direction
makes `log2(0)`. A gradient of zero (a constant coordinate) gives the same
`-inf` here. What the TMU makes of `-inf` on TMUB is not measured: take the
clamp unless the sim and Mesa agree that `-inf` is read as level 0.

## (c) Where it fits in prepare.rs

One place: the `Op::Tex` arm, `prepare.rs:1327`. After the `match lod`
(`:1353-1364`) replaces its last arm:

- `ir::TexLod::Grad { ddx, ddy }` computes `level` with a new method
  `grad_lod(a, sampler, dim, ddx, ddy) -> S` (next to `cube_lod`, `:829`,
  and `clamp_lod`, `:884`) and yields `(Some(level), true)`: the same shape
  as `Lod`, so the later code (the packed operands, `SOp::Tex { lod: true }`,
  `:1428-1440`) is untouched.
- The gradients are read with `self.src(Src::id(ddx), lane)`, one lane at a
  time (`eval.rs:728` reads them as `Src::id`); the lanes are the identity
  swizzle at the gradient's width (`ir.rs:158-160`).
- 2D, 2D array, 3D: `grad_lod` is steps 1-4 of (a). `dim` selects the lane
  count (2, 2, 3) and the size lanes (0..2). It does not read the layer or
  the shadow reference.
- Cube: the existing `cube_lod` (`:829-880`) already is the cube arithmetic
  of (a), written as dEQP's `computeCubeLodFromDerivates`; Mesa's and its
  formulas are the same up to the sign handling (`-1 + 0.5 log2(L^2 M)` =
  `0.5 log2((L/2)^2 M)`, `:871-879`). Split it in two: the part that takes
  the three-lane direction `d`, the two three-lane derivatives `dd[0]`,
  `dd[1]`, the major-axis flags and `inv`, and the wrapper that today fills
  `dd` with `Fddx`/`Fddy` of `d` (`:861-866`). `Grad` calls the first with
  the user's gradients. The flags come from `cube_face` (`:752`, whose ties
  are `texref::select_cube_face`'s, `x` before `y` before `z`). Note that
  Mesa's tie-break order is z, then y, then x (`nir_lower_tex.c:763-764`); a
  tie is a direction on a cube edge, where both faces give the same
  derivative length (the divided derivative is continuous across the edge),
  so the order does not change the level, but a test should have an edge
  direction.
- The cube branch that today adds a level for the no-seam case
  (`:1412-1419`, only when `!is_lod`) stays; with `Grad` the level exists
  already and `is_lod` is true, so that branch is skipped, and the face
  rewrite beneath it still happens (the layer, `:1420-1424`).
- Projective forms (`textureProjGrad*`): the gradients are not divided by
  `q` (Mesa, (a)), and `prepare.rs:1367-1372` divides only `s` and `t`
  (a projected cube is already refused, `:1332-1334`). Nothing to add.
- Offsets: unchanged. The offset refusal (`:1347-1349`) is the item
  `v3d-qpu-fetch-offset`'s; `Grad` composes with it. Shadow
  (`:1335-1337`) is `v3d-qpu-shadow`'s; the gradient lowering is the same for a
  shadow sampler (Mesa lowers all `txd`), so the order of the three items is
  free.
- Vertex stage: a gradient look-up is legal in a vertex shader (dEQP has
  `_vertex` forms of every case) and needs nothing from the derivatives, so
  the same code serves both stages. No `Fddx` is involved.
- `eval.rs` has the host model for `TexLevel::Grad` already (`:723-736`) and
  `sim.rs` models TMUB+TMUSLOD; the sim does not need the gradients.

The new operations all exist in `AluOp` and `select.rs` (`U2f`, `Fmul`,
`Fadd`, `Fmax`, `Fsqrt`, `Flog2`, `Bcsel`, `Fabs`; `Fddx` is not used).

## (d) Precision

The bound is dEQP's, not the hardware's. The 2D formula above is exact in
real arithmetic; the QPU's LOG, RSQRT and RECIP are SFU approximations, which
Mesa has too (its `fsqrt` is the same two operations). What the host knows:

- `texref.rs:148-150` pins `LOD_BITS = 8`, dEQP's lod precision for the
  mipmap cases (`es2fTextureMipmapTests.cpp:330-340`), and `texref.rs:395`
  turns it into the width of the band around a level boundary in which the
  model answers "within dEQP's LOD precision of a level boundary" and a
  mismatch is not counted (`sim.rs:2235`). So a level within `2^-8` of an
  integer, or of a filter switch at 0, may land on either side.
- dEQP's `texturegrad` cases (`functional.shaders.texture_functions.
  texturegrad*`, 35, with `texturegradoffset` 28, `textureprojgrad` 26 and
  `textureprojgradoffset` 26 in `docs/es3/shader-failure-map.md:216`) verify
  by `TexLookupVerifier` and accept any result a legal level within its
  `lodBits` and its derivative precision (`derivateBits`) would give; the
  reference code is not in this tree, so the exact numbers are to be read from the dEQP source when the item starts and
  entered into the test as constants, not guessed. `docs/es3/shader-host-repro.md:98`
  and `docs/es3/am62p-gles3-notpass.tsv:1770-1792` list the case names (the
  `sampler2d`, `2darray`, `3d`, `cube` and `isampler` forms, fragment and
  vertex, with the shadow forms).
- The ES spec allows any `rho` between the largest component of the two
  gradients and the sum of their absolute values (GL ES 3.0.6
  section 3.8.10); the exact norm Mesa uses, and the fold of (b), are both
  inside it. (Quoted from memory of the spec, not
  checked in this tree.)
- An integer sampler (`isampler`, `usampler`) selects its level the same
  way; the TMU reads the texels raw, and the result has no filter, so the
  level only picks the level.

The cube path's existing check is a good model: `tests/v3d_cube_noseamless.rs:393`
(`deqp_cube_lod`, in `f64`) and `:525-560` compare every lane's computed
level against dEQP's formula with the bias added.

## (e) Test plan

1. `tests/es3_shaderc_v3d.rs` (it builds through `SHC_ONLINE` like
   glCompileShader, `build`, `:39`; `run_vs`, `:147`, runs a vertex shader in
   the QPU simulator and reads back the result): a build test for each form
   in the ES 3.0 set, all fragment, and a vertex-stage one for `sampler2D`
   and `samplerCube`: `textureGrad` on `sampler2D`, `sampler2DArray`,
   `sampler3D`, `samplerCube`, with `isampler2D`, and `textureProjGrad` on a
   `sampler2D`. Today each must fail with the message at `prepare.rs:1364`;
   after, `build` returns `Ok`. 3D and array rows need the item
   `v3d-qpu-array-3d`; mark them `#[ignore]` with that reason until then.
2. A value test for a vertex shader (no derivatives, so `run_vs` computes it
   with known gradients): the same texture, level 0 of 3 or more, a
   gradient scaled to give `rho = 2^k` for `k = 0, 1, 2`, expecting the
   level `k` and the level-`k` texel. Cases: a gradient in x only, in y
   only (the larger of the two chosen), both equal, zero (`-inf`, the clamp
   question of (b), expecting level 0), a negative lane (the sign must not
   matter), and a texture of non-square size (`5x3`, so a swapped
   width/height shows).
3. `tests/v3d_sim_mesa.rs`: the pair harness compares our compiled program
   with Mesa's `v3d_shc` output by running both in the simulator on the same
   uniforms and textures (`texture(u, cube, mip)`, `:744`, `TEXTURE_SIZE`
   5, and the mipmapped one). Add pairs that read `textureGrad`; the
   mipmapped texture (`mip = true`) is the one that makes the level visible
   (each level's alpha names the level, `:728-735`). Mesa's output is the
   oracle for the arithmetic, so the pair needs the gradients as uniforms
   (a `uniform vec2 gx, gy`), not constants, or Mesa folds the level away
   at compile time. The harness skips a level within dEQP's LOD precision of
   a boundary (`sim.rs:2235`); choose gradients that avoid the boundary
   or let the skip count them.
4. A cube row for the same: three gradient lanes, a direction with a large
   major axis (to see the normalisation not applied to the gradient), a
   direction on an edge, and each of the three major axes.
5. Instruction counts: `tools/v3d/qpu_counts_42.tsv` has one row per corpus
   pair (vertex shader, fragment shader, then the fs, vs and cs counts). Add
   the Grad programs to the corpus (`tools/v3d/qpu_corpus.py` regenerates the
   file; tools/v3d belongs to item `qpu-ref-counts` while it runs: ask for
   the rows there) and compare our fragment shader's count with the
   `v3d_shc` row of the same program, built with `MESA_V3D_TREE` unset (the
   vendored reference build, `tools/v3d/README.md`; `tools/v3d/drm_shim_ref.sh
   build` makes it). Expected: ours not more than Mesa's with the fold,
   equal or within a few without it (the scheduling of the SFU waits
   differs). Record the numbers in the item's commit message.
6. The gate: `tools/check.sh --changed origin/main` against the current
   main tip. The change reaches `src/shaderc/online/v3d/**`, so the rows of
   the round-trip corpus, `qpu_corpus_*.txt`, and the golden programs run;
   a golden that changes is regenerated in the same commit.

## What the note leaves open

- The clamp of the level, and what the TMU does with `-inf` (b).
- The numbers of dEQP's `lodBits` / `derivateBits` for the `texturegrad`
  cases (d); they are to be read from the dEQP source of the CTS build.
- Which sampler-state fields the size words follow (the base level, a
  `TEXTURE_MAX_LEVEL` cut): Mesa's `textureSize(s, 0)` is the size of the
  texture's level 0 from the driver's uniform; ours is `TexSizeWord` of the
  same name (`select.rs:1989`) and follows the existing `textureSize`.
- Nothing here has run on the Pi 4.

## Index line for docs/README.md

`- [notes/v3d-qpu-grad.md](notes/v3d-qpu-grad.md): textureGrad on the V3D 4.2 lowered to an explicit level: Mesa's formula, the QPU operations and uniforms, where it fits in prepare.rs, precision, test plan`
