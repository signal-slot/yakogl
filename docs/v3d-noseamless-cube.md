<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: non-seamless cube sampling in ES 2.0 builds (design)

`c3e4146d`, 2026-10-07. The design and the host reproduction of
the Raspberry Pi 4's 25 remaining dEQP-GLES2 failures; `ec5ba4fb` (same
day) implements it. The Pi 4's whole tree at `a27d69c0` (2026-10-10) passed
19 of the 25 and failed 6 cube mipmap cases and 3 `texture.size.cube.15x15`
cases, all from one cause: the fragment stage's level of detail (section
"The level of detail"), fixed on the host by computing the level in the
shader (section "As implemented"). That fix ran on the board on `1cf946a9`
(2026-10-11): T3 304 / 304 and T4 69 / 69 pass, and the six rows whose
fragment cube look-ups changed pass 934 of 934 mustpass cases
(`tools/cts/RESULTS.md`, "Pi 4: gl_test on `1cf946a9` and the six cube
rows").

## The defect

25 cases fail on the Pi 4 under the conformance configuration
(`CONFIG_YAKOGL_REPORT_ES2=y`) and pass with it off (tools/cts/RESULTS.md,
"Reference-selection difference", 2026-09-25):

| Group | Cases | Look-up |
|---|---|---|
| `texture.vertex.cube.filtering.*` | 10 | vertex `textureCubeLod`, mag LINEAR with every min filter, wrap clamp / mirror / repeat |
| `texture.vertex.cube.wrap.*` | 8 | vertex `textureCubeLod`, LINEAR, every (wrap_s, wrap_t) pair but repeat_repeat |
| `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}` | 6 | fragment `textureCube`, implicit level (and a bias), LINEAR_MIPMAP_LINEAR / LINEAR_MIPMAP_NEAREST |
| `shaders.texture_functions.vertex.texturecubelod` | 1 | vertex `textureCubeLod`, LINEAR |

The cause is one rule. The V3D 4.2 filters every cube map seamlessly: a
LINEAR tap past a face's edge reads the neighbouring face (ES 3.0 3.8.9.1),
and the sampler's wrap modes are ignored. There is no bit to turn this off:
the texture state record (`V3dTexState`, `src/v3d/ffi.rs`) and the sampler
record have none, Mesa's v3d has no seamless control anywhere and does not
implement `VK_EXT_non_seamless_cube_map` (RESULTS.md's corroboration list).
dEQP chooses its reference by the context: `IsES3Compatible` is true when
`GL_MAJOR_VERSION` answers, and our ES 2.0-reporting images make it an
error, so every one of these cases is judged against the NON-seamless
reference of GL ES 2.0 3.7.5: the face is selected by the major axis, (s, t)
come from Table 3.21, and the face is then "an independent two-dimensional
texture" filtered with the sampler's wrap modes (`sampleLevelArray2D` on
`m_levels[face]`, tcuTexture.cpp). The 25 are exactly the cases whose
LINEAR taps reach a face edge: NEAREST and NEAREST_MIPMAP_NEAREST read the
selected face's texel either way, so they pass under both references.

The goal is zero failures, so the driver must sample cube maps
non-seamlessly in an ES 2.0 build, and keep the seamless hardware path in an
ES 3.0 build (which ES 3.0 requires and the seamless reference judges).

## The chosen emulation: the cube as a 2D array of six faces

On the V3D 4.2 the texture state record has no dimensionality. A cube's
record is a 2D-array record: `depth` = 6 layers and `array_stride_64` = the
per-face mip tree stride (`src/v3d/texture.rs` `state`, Mesa's
`v3dx_state.c:936-938` sets `image_depth` to the layer count for cubes and
arrays alike, and `v3d_resource.c` lays both out as `cube_map_stride`
layers). The LOOK-UP decides what the record is: a cube look-up writes T, R
and retires with TMUSCM; a 2D-array look-up writes T, the layer in TMUI and
retires with TMUS (implicit level) or TMUSLOD (explicit level), Mesa's
`v3d_tex.c` `is_array` write.

So the emulation changes no record, no job builder field and no GL-core
binding: in the ES 2.0 variant the compiler lowers `textureCube(s, d)` to

1. the direction normalised by its major axis (already done for every cube
   look-up: `prepare.rs`, `nir_normalize_cubemap_coords`), giving (x', y',
   z') with the largest component ±1;
2. the face and face coordinates of GL ES 2.0 Table 3.21, in the order of
   dEQP's `selectCubeFace` (`texref::select_cube_face`, which the C header
   tools/v3d/tex_filter_ref.h transcribes) so that ties go where the
   reference's go:
   - |x| largest: face = x' >= 0 ? 0 : 1, sc = x' >= 0 ? -z' : z', tc = -y'
   - |y| largest: face = y' >= 0 ? 2 : 3, sc = x', tc = y' >= 0 ? z' : -z'
   - |z| largest: face = z' >= 0 ? 4 : 5, sc = z' >= 0 ? x' : -x', tc = -y'
   - s = 0.5 sc + 0.5, t = 0.5 tc + 0.5 (ma = 1 after the normalisation)
3. a 2D-array look-up of the SAME sampler binding: TMUT = t, TMUI = face,
   TMUB = the explicit level (vertex stage, `textureCubeLod` and the
   implicit `textureCube`, which the front end gives as `Lod(0.0)`) or the
   bias (fragment `textureCube(s, d, bias)`), retired by TMUSLOD (explicit)
   or TMUS (implicit) with S = s; P0 and P1 as today (the unit, the word
   mask, the per-pixel mask enable, `ret32`), no P2.

The TMU then applies the sampler's wrap_s / wrap_t and its min / mag / mip
filters inside the one layer: non-seamless by construction, for every
filter, wrap and mipmap combination, with the hardware's own LINEAR weights
and level selection. About 20 scalar operations per look-up (3 FABS, 2
FMAX, 1 RCP already there; 4 comparisons, 6 selects, 2 FMUL+FADD).

The level of detail:

- Vertex stage: explicit, as today (TMUSLOD instead of TMUSCM + P2
  disable_autolod). `texturecubelod` and the 18 `texture.vertex.cube` cases
  are this path.
- Fragment stage, implicit or with a bias: the level is computed in the
  shader and the look-up retires with TMUSLOD (TMUB = the level). The TMU's
  own level cannot be used: it derives it from the quad's differences of
  the per-face (s, t) the shader writes, and in a quad whose four pixels do
  not all select one face the difference across the face jump is
  meaningless. Such quads are not rare: dEQP draws each face as its own
  quad whose corner directions lie exactly on the face's edges, so wherever
  the primitive's edge (or the viewport's) does not fall on an even pixel,
  the 2x2 quad holds helper pixels whose extrapolated direction is past the
  edge and selects the neighbouring face. On the Pi 4 at `a27d69c0` that
  gave the edge pixels of every `mipmap.cube` cell the smallest level's
  colour (6 cases) and, in `texture.size.cube.15x15_*` (min LINEAR, mag
  NEAREST, a magnified 15x15 face), a minification on the viewport's edge
  row and column: LINEAR where GL magnifies with NEAREST. On a 15-texel face
  LINEAR is up to 8.43 / 255 away from the NEAREST texel there, over the
  8-bit formats' threshold of 8 (1 + 7) and inside RGBA4444's 23: three
  cases fail and `15x15_rgba4444` passes; on the power-of-two faces the
  edge pixels sit at most 7/16 of a texel from a texel centre, 7.0 / 255,
  so they pass (a model of dEQP's comparison over the six faces' edge rows
  and columns, both sizes). The level is dEQP's
  `computeCubeLodFromDerivates` (glsTextureTestUtil.cpp), the exact LOD of
  ES 2.0 3.7.7 on the pixel's own face: with (ma, sc, tc) the direction's
  major, s and t components for that face (x: (x, z, y), y: (y, x, z), z:
  (z, x, y)), du/dx = W/2 (dsc/dx |ma| - sc |dma/dx|) / ma^2, likewise dv/dx
  and the y pair, and lambda = log2(max(|(du/dx, dv/dx)|, |(du/dy,
  dv/dy)|)); the direction's derivatives (FDX, FDY of the interpolated
  direction, `select.rs` dFdx / dFdy) are continuous across faces, so a
  pixel's level does not depend on which face its quad neighbours select.
  W is the face size, the texture's `QUNIFORM_TEXTURE_WIDTH` word (the job
  builder fills it from the binding, as Mesa's v3d_uniforms.c does for
  textureSize). The bias is added, and the level clamped to [-16, 16]
  (a constant direction gives log2(0)). Measured on dEQP's shader
  (`textureCube(u_cube, v_dir)`): the keyed fragment program grows from 86
  to 155 instructions (156 with a bias) and from 7 to 12 uniform words, at
  4 threads still; fragment variants only, the unkeyed program unchanged.

Which look-ups pay: only an ES 2.0 build (`crate::REPORT_ES2`), and only a
cube binding whose sampler state can differ: mag LINEAR or min LINEAR /
LINEAR_MIPMAP_* (`binding_filters` in `src/gl/draw.rs` already returns
these). NEAREST with NEAREST or NEAREST_MIPMAP_NEAREST reads the selected
face's texel under both rules and keeps the hardware cube path (the only
differences there are the tie rule at an exact face edge, which dEQP
avoids, and the LOD derivation, which is the same inside a face). The
sampler-state variant mechanism (`shaderc::LodKey`, `online::lodkey`,
`draw_lod_key`) selects the variant per draw from the state bound now, so
nothing is hooked on glTexParameter.

### A second rule the same cases need: the c = 0.5 switch

Six of the ten `filtering` cases (`nearest_mipmap_{nearest,linear}_linear_*`)
also depend on GL ES 2.0 3.7.8's magnification switch at lambda = 0.5 for
mag LINEAR with min NEAREST_MIPMAP_*. The AM62P and the i.MX8MP needed the
`mag_switch` remap for exactly these (RESULTS.md "Batch 107"); it is
compiled only in ES 2.0 builds and `sensitivity_of` returns it for every
GPU but the V3D (`backend.rs`: the V3D path returns `ret32` only). The TMU's
own switch is not known to be at 0.5, and under the seamless reference
(switch at 0) the question never arose on the Pi 4. The implementation must turn
`mag_switch_sensitivity` on for the V3D vertex stage too (the remap is
IR-level and already exists); if the board then shows these six passing,
the TMU switched at 0, and if they already passed before the remap, the
TMU's switch is at 0.5 and the remap is harmless. The host cannot tell.

## Rejected options

**(a) Six 2D texture state records per cube binding, face select in the
shader.** The same shader as the chosen design, but six records per cube
sampler in the per-stage table of 16 units (`MAX_UNITS`), a job-builder
change (P0 relocation per face: binding -> six records), and a GL-core
change (unit bookkeeping for the extra records). Two cube samplers plus a
few 2D ones overflow the table. Nothing is gained over the 2D-array look-up,
which reads the record the cube already has.

**(b) Shader-side filtering with texel fetches.** Four TMUSF fetches per
level (eight for LINEAR_MIPMAP_LINEAR), the level computed in the shader for
every look-up including the fragment stage's implicit one, the wrap modes
emulated on integer indices, and f32 weights that differ from the TMU's
fixed-point ones (a cube and a 2D texture in one image would filter
differently; dEQP's LINEAR tolerance would absorb it, Slint's images might
not). Ten times the cost of the chosen design, TMUSF is not used by any of
our programs yet (`sim.rs` refuses it), and the vertex stage's output FIFO
budget (16 words a thread) is tight with eight look-ups in flight.

**(c) Wrap-mode emulation on the cube path.** Clamping or repeating (s, t)
before a TMUSCM look-up changes nothing: the hardware re-derives the face
from the direction and reads the neighbouring face for the tap past the
edge whatever the coordinates say.

**(d) Unconditional, every ES 2.0 cube look-up (no variant).** Simplest: no
key bit, no GL-core change, the lowering under `REPORT_ES2` only. Rejected
for cost on NEAREST cubes and because it moves the fragment stage's
implicit LOD onto the per-face derivatives for every cube in an ES 2.0
product image. Kept as the fallback if the variant plumbing for the vertex
stage proves larger than expected: the lowering itself is the same, only
the condition differs, so the implementation should write the lowering keyed by a
flag and decide the plumbing last.

**(e) Reporting ES 3.0 in the conformance image.** Makes dEQP use the
seamless reference and the 25 pass today; rejected by the owner (CLAUDE.md
keeps `REPORT_ES2` for conformance runs; a client that reads
`GL_MAJOR_VERSION` would believe it has ES 3).

## The code points

In the order the implementation should take them; each with its host evidence.

1. **`src/shaderc/online/v3d/prepare.rs`, `Op::Tex` with `dim == Cube`**
   (around line 1116). Under the variant condition, after the
   normalisation, emit the face selection (step 2) as `SOp::Alu` operations
   and push `SOp::Tex { cube: false, layer: Some(face), .. }` with `srcs =
   [s, t, face, level-or-bias]`. `SOp::Tex` (line 91) and `tex_operands`
   (line 2007) gain the layer operand: `2 + cube + layer + third`. Evidence:
   the chain tests' listing (`tmui`, `tmuslod`, no `tmuscm`) and
   tests/v3d_cube_noseamless.rs's ignored test.
2. **`src/shaderc/online/v3d/select.rs`, `SOp::Tex` emission** (line 855):
   for `layer`, write TMUI (`WADDR_TMUI = 36`, add it next to `WADDR_TMUR`)
   after TMUT and before TMUB, no P2 word, retire TMUSLOD when `lod` else
   TMUS. The operand counting at line 829 and the `tex_operands` users at
   prepare.rs 157 / 1954 follow. `chain.rs` `first_tmu_memory_access` (line
   781, run on every program by `check_all`) reports every TMU write that is
   not TMUS / TMUT / TMUR / TMUB / TMUSCM / TMUSLOD, so TMUI (36) must join
   its `lookup` list or every emulated program fails the chain tests.
   Evidence: `chain.rs` tests (a listing pin like
   `a_ret32_binding_reads_one_32_bit_word_per_channel`), and the
   interpreter: `sim.rs` models TMUI as of this item (a 2D-array look-up on
   a cube texture reads the face with the sampler's wrap modes;
   `a_cube_lookup_is_seamless_and_an_array_lookup_of_its_face_is_not`).
3. **The variant key, `src/shaderc/mod.rs` `LodKey`.** A new field
   `cube_noseam: u32` (bindings 0..14) in bits 48..62 of `bits()` /
   `from_bits()`, which means narrowing `mag_switch` to bits 32..47 (it keys
   bindings below `CAP_SAMPLERS` = 16 only, `stage_lod_key`). Binding 15 of
   a stage cannot be keyed for the cube rule; a 16th cube sampler in one
   stage would stay seamless. The ABI is one `uint64_t`; its C side is in
   `include/` (grep `lod_key`). Evidence: `the_key_packs_and_unpacks` style
   test in `shaderc::mod` (`LodKey { cube_noseam: 1 }.bits() == 1 << 48`).
4. **The GL core, `src/gl/draw.rs` `stage_lod_key`** (line 1391): for a
   `GL_SAMPLER_CUBE` binding in `sense.cube_noseam`, set the key bit when
   `mag == GL_LINEAR || matches!(min, GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST |
   GL_LINEAR_MIPMAP_LINEAR)`. `binding_filters` already returns min and mag
   (a sampler object's over the texture's). Evidence: the existing
   `stage_lod_key` tests (line 7951 on) extended with a cube binding and the
   four filter cases.
5. **The compiler's sensitivity and variant, `src/shaderc/online/backend.rs`.**
   - `sensitivity_of` (line 1486): the V3D branch returns `ret32` for the
     fragment program record only. Add, under `crate::REPORT_ES2`: the cube
     look-ups' bindings (`Op::Tex { dim: Cube, sampler }`, sampler < 15) as
     `cube_noseam`, for BOTH stages, and `mag_switch_sensitivity` for the
     vertex stage (section above). A vertex shader record is then sensitive
     on the V3D, which `a_v3d_program_has_a_ret32_variant`'s "a vertex
     shader is sensitive to nothing" assertion pins today: change the pin.
   - `online_sampler_variant` (line 1622): the V3D refuses every key bit but
     `ret32`; accept `cube_noseam` and `mag_switch`, and compile a VERTEX
     variant too (`variant_of(SHC_STAGE_VERTEX, ...)` as the generic path
     does).
   - `link_program` (line 1352): the link compiles the whole program with
     ONE key, the fragment record's (`set_compile_lod_key(fr.lod_key)` at
     line 1385, read in `select.rs` 2046 and in prepare for the lowering).
     The vertex stage needs its own: either `set_compile_lod_key` takes the
     pair and `compile_lod_key()` answers per stage (`Facts` carrying
     `[u64; 2]`), or the chain's `compile_program` takes the two keys as
     arguments and threads them into `prepare_vertex` / `prepare_fragment`.
     The second keeps the atomics as they are and is what the interpreter
     tests want (a key passed to `compile_program`).
   - `lodkey::remap_mag_switch`: apply it in the V3D chain for the vertex
     module when the vertex key has `mag_switch` bits (the PowerVR and
     GC7000 chains show where).
   Evidence: `a_v3d_program_has_a_ret32_variant`'s pattern for a
   `cube_noseam` key: the variant's VS stream has no P2 word, its listing
   has `tmui`; `backend.rs` 2320-2330 pins.
6. **`src/v3d/backend.rs`, the job builder.** Nothing to change: the record
   and sampler of the binding are the cube's (`tex_state_for`,
   `sampler_for`); `texture_units` resolves P0/P1 by unit as before. Check
   only that `cube_units_of` (line 3811: which units sample as cubes, for
   the black stand-in) stays right: a unit sampled by the emulation is
   still a `GL_SAMPLER_CUBE` uniform and the 1x1 black cube stand-in is a
   six-layer record too, so a 2D-array look-up on it reads black. One
   backend test: the stand-in's record has `depth` 6 and an array stride.
7. **tests/v3d_cube_noseamless.rs.** Remove the `#[ignore]`; make
   `the_compiled_cube_lookup_is_seamless_today` `#[cfg(not(feature =
   "report-es2"))]` (the ES 3.0 build keeps the hardware path) and have it
   assert `tmuscm`; the ES 2.0 test asserts `tmui` + `tmuslod`, no `tmuscm`.
   The test compiles through `chain::compile_program` with no key today
   (the program's own code); with the key threaded (point 5) it passes the
   `cube_noseam` key for binding 0 so the variant, not the base program, is
   what runs.
8. **docs/online-shaderc/v3d-backend.md**: a section pointing here;
   **issue #12**; tools/cts/RESULTS.md's "Reference-selection
   difference" paragraph gets its closing line after the board run (the
   docs-sync item).

What the LodKey bit costs at run time: one extra vertex and fragment
compile per (program, cube sampler state) combination the first time a
draw asks (`lod_variant_for` keeps `PROG_MAX_LOD_VARIANTS` per program), and
the `V3D_VARIANTS_BUILT` counter the back end prints already.

## What the host can and cannot show

Can (this item's evidence, all green in `host-test,gpu-v3d,shaderc-online,
no-shader-table` with and without `report-es2`):

- `texref.rs`: the sampler-state model. `wrap_index` is dEQP's `wrap`
  (`the_wrap_modes_are_deqps`); the seamless tap past an edge reads the
  neighbouring face's edge column at the same row and inverts
  `project_to_face` (`a_seamless_tap_past_an_edge_reads_the_neighbouring_face`);
  a cube look-up is seamless and a layer look-up of its face wraps within
  the face (`a_cube_lookup_is_seamless_and_a_layer_lookup_wraps_within_the_face`);
  mipmap-LINEAR blends (`a_mipmap_linear_filter_blends_two_levels`).
- `sim.rs`: TMUI, the sampler by P1's index, LINEAR, and the two paths on
  one cube texture (`a_cube_lookup_is_seamless_and_an_array_lookup_of_its_face_is_not`):
  the emulation's output (TMUI + TMUS on the cube record) yields the
  non-seamless value for CLAMP, REPEAT and MIRROR, and a TMUSCM look-up the
  seamless one whatever the wrap.
- tests/v3d_cube_noseamless.rs: our compiler's own `textureCube` programs on
  the interpreter, against references computed in the test (not by the
  model). As designed it pinned the seamless value of the unkeyed program
  and carried an ignored ES 2.0 target test that failed on the seamless
  value; both were replaced by `ec5ba4fb`, see "As implemented /
  Host evidence" below for what the file holds now.

Cannot:

- The TMU's reading of TMUI on a sampled look-up (float, rounded; or an
  integer). ASSUMED float, as the other coordinates of a sampled look-up
  are and as Mesa's `is_array` write passes the NIR float layer with no
  rounding pass (`nir_lower_tex`'s `lower_array_layer_round_even` is for
  hardware that cannot). If the board says integer, the lowering adds one
  FTOIN of the face. (The Pi 4 answered: float; 19 of the 25 passed at
  `a27d69c0`.)
- LINEAR weights: the model's are f32; the board measured the TMU's
  (gl_test case 109). dEQP's tolerance covers the difference.
- The V3D's quad lane layout for FDX / FDY: the interpreter models them
  only on an assumed layout (`sim::Env::quads`, off by default), which the
  fragment tests' inputs follow; a level of detail depends only on the two
  derivative vectors' lengths, not on which direction is which nor on the
  signs.
- The TMU's switch between magnification and minification on an explicit
  level in the fragment stage. The emulated fragment look-ups now give the
  TMU an explicit level, so mag LINEAR with min NEAREST_MIPMAP_* (GL's
  c = 0.5, `texture.filtering.cube.nearest_mipmap_*_linear_*`, image T2)
  rests on the TMU's explicit-level switch. That switch is the one
  `texture.vertex.2d.filtering.nearest_mipmap_*_linear_*` pass with on the
  Pi 4 (explicit levels, no `mag_switch` remap on the V3D), so no remap is
  added; T2 is the board's check.

## Test plan

Host, with the fix (all in the two V3D configurations; `tools/check.sh`
runs them: the `--lib` rows for texref.rs and sim.rs, and a
`--test v3d_cube_noseamless` row in each V3D build beside the
`v3d_sim_mesa` rows):

| What | Where | Expect |
|---|---|---|
| Filtering: mag LINEAR / NEAREST, min each of the six, at the edge direction | tests/v3d_cube_noseamless.rs (grow `linear` into a table over `Filter` x `MinFilter`) | ES 2.0 build: `non_seamless_reference`; ES 3.0 build: `seamless_reference`; NEAREST / NEAREST_MIPMAP_NEAREST equal under both |
| Wrap: the nine (wrap_s, wrap_t) pairs, directions past the s edge and past the t edge | the same test, a second direction (1, -0.875, 0) for the t edge | CLAMP and MIRROR repeat the edge column / row, REPEAT takes the opposite one |
| Mipmap: a 3-level cube, `textureCubeLod` at 0.25, 1.0, 1.5 with LINEAR_MIPMAP_LINEAR and NEAREST_MIPMAP_LINEAR | the same test; the model's `levels` blend | the per-face bilinear of each level blended by frac(lod); `Outcome::levels` names both levels |
| Vertex-stage explicit LOD | `textureCubeLod(s, d, u_lod)` as `lodkey.rs` VS_CUBE; the ES 2.0 listing | `tmui`, `tmub`, `tmuslod`, no P2 word (contents 0, 0x00f00002) |
| The variant condition | `stage_lod_key` tests in draw.rs | key bit set for LINEAR mag or min, clear for NEAREST / NEAREST_MIPMAP_NEAREST and for a `sampler2D` |
| The ES 3.0 build keeps the hardware path | the ES 3.0 test | `tmuscm`, seamless value, `sensitivity_of` has no `cube_noseam` |
| Nothing else moved | `v3d_sim_mesa` both builds, `--lib` 2638+ | counts as before |

Board (one session, `tools/board.sh`, the Pi 4; exact lines for the
handoff after the fix):

1. The TMUI probe first: a `v3d_gl_test` case (or the dEQP image's
   `texture.vertex.cube.wrap.clamp_clamp` alone) built with REPORT_ES2.
   Pass means the TMU took the float face index; a picture with every face
   black or one face everywhere means integer, and the FTOIN goes in.
2. `tools/cts/pi4-deqp-images.sh` with the 25 names (RESULTS.md's list at
   its line 14991) in one image, REPORT_ES2 on: expect `done: 25 passed`.
   If the six `nearest_mipmap_*_linear_*` fail alone, the `mag_switch`
   remap is missing from the vertex stage (section above); if the six
   `mipmap.cube` fail alone with invalid pixels along face edges in the
   QPA, the implicit-LOD fallback (FDX / FDY) is needed.
3. The same 25 with REPORT_ES2 off: expect `25 passed` still (the
   seamless path unchanged).
4. `v3d_gl_test` 21/21 with REPORT_ES2 on and off (no regression in the
   cube cases of the GL suite: a skybox-like cube with NEAREST must be
   untouched).
5. The full Pi 4 tree (tools/cts/pi4-full-tree.txt, 29 images) at the end:
   17,100 pass, 0 fail, the 62 not supported and 3 quality warnings as
   sweep 30.

## Assumptions recorded

- TMUI takes a float layer on a sampled look-up (above).
- The TMU's magnification/minification switch is at lambda <= 0 (the
  model's rule); the `mag_switch` remap moves GL's 0.5 cases.
- A corner tap of a seamless LINEAR look-up is the mean of the other three
  taps in the model (dEQP's convention, ES 3.0's recommendation); the
  hardware's rule is not known, so no test samples a corner.

## As implemented (`ec5ba4fb`)

The design's code points 1-5 and 7, with these decisions:

- **Key**: `LodKey::cube_noseam` (bits 48..62, bindings 0..14); `mag_switch`
  narrowed to bits 32..47 (bindings 0..15, all the GL core ever keys).
  `LodKey` gained a field, so the six `LodKey { .. }` literals of
  backend.rs's tests were extended; the C ABI is one `uint64_t` as before.
- **Compiler**: `prepare.rs` lowers a keyed cube `Op::Tex` to
  `SOp::Tex { layer: true }` with the face and (s, t) of `Walk::cube_face`
  (dEQP's `selectCubeFace` order, ties included, on the un-normalised
  |x|, |y|, |z|); `select.rs` writes TMUI (`WADDR_TMUI` = 36) after TMUT and
  before TMUB, retires TMUSLOD (explicit level) or TMUS, no P2 word;
  `chain.rs` lists TMUI among the look-up writes of its TMU check. The
  unkeyed path pushes the same operations in the same order as before.
- **Two keys**: `chain::compile_program_keyed(vs, fs, prim, target, vs_key,
  fs_key, a)`; `compile_program` is it with no vertex key and the compile's
  own (`shaderc::compile_lod_key`) fragment key. `codegen::link_sources`
  takes the vertex key (`ProgramKey::vs_lod`), applies
  `lodkey::remap_mag_switch` to the vertex module with its `mag_switch`, and
  `backend::link_program` builds the vertex record's key (`vr.lod_key`) into
  the program's cache key. The vertex key reaches `link_program` because the
  GL core links the vertex VARIANT (`lod_variant_for`) with the fragment one.
- **Sensitivity** (`backend::v3d_sensitivity`, ES 2.0 builds only): the
  fragment program record keeps `ret32` and gains the fragment cube
  bindings; a vertex record is sensitive to its cube bindings and, of
  those, to the explicit-level ones for `mag_switch` (cube only: 2D vertex
  textures keep the hardware's switch, which they pass today).
  `online_sampler_variant` accepts `cube_noseam` on the program record;
  `mag_switch` stays refused there (it is the vertex record's key).
- **GL core** (`gl/draw.rs`): `stage_lod_key` sets the bit for a
  `GL_SAMPLER_CUBE` binding by `cube_filters_see_seams` (mag LINEAR, or min
  LINEAR / LINEAR_MIPMAP_*); on the V3D a vertex key alone now rebuilds the
  variant's program too (`fs_rebuilt`: the program holds both stages'
  code, so the link and the code upload run for a vertex key as for a
  fragment one).
- **Fragment level** (after the Pi 4's `a27d69c0` tree): `prepare.rs`
  `Walk::cube_lod` computes a keyed fragment look-up's level from FDX / FDY
  of the direction on the selected face (section "The level of detail"),
  adds the bias, clamps it (`clamp_lod`) and retires with TMUSLOD, so a
  fragment variant writes TMUT, TMUI, TMUB and TMUSLOD; the vertex stage is
  unchanged. The face size is the texture's `QUNIFORM_TEXTURE_WIDTH` word:
  `struct v3d_job_tmu` carries each binding's base-level width and height
  (`V3dJobTmu`, filled by `texture_units`, 1 x 1 for the black stand-in,
  which a unit named only by a size word also gets), and v3d_job.c writes
  `V3D_QUNIFORM_TEXTURE_WIDTH` / `_HEIGHT` from it (a binding not given, or
  one without its size, is a bad field). This also ends the by-name refusal
  of textureSize on the V3D.
- Not done: a 16th cube sampler of a stage stays seamless (bit 15).

### Host evidence

- tests/v3d_cube_noseamless.rs (both builds, 8 tests): every wrap pair on
  eight directions over all six faces, the CLAMP / MIRROR / REPEAT edge
  case against the seamless value, explicit levels (-1, 0.25, 1, 1.5) with
  LINEAR_MIPMAP_LINEAR / NEAREST_MIPMAP_LINEAR / LINEAR on a 3-level cube,
  a projected direction (`a_dir.xyz / a_dir.w`), the fragment stage's level
  (`a_fragment_lookup_takes_each_pixels_level_from_the_direction`: four
  quads, one inside +X, one straddling its edge with the left column on +Z
  as dEQP's helper pixels are, one on +Z where the major axis varies, on a
  5-level cube of one colour per level and face, implicit and biased (to
  below zero and past the chain), LINEAR_MIPMAP_LINEAR and
  LINEAR_MIPMAP_NEAREST, every lane against dEQP's
  `computeCubeLodFromDerivates` computed in the test;
  `a_fragment_lookup_with_a_constant_direction_magnifies`: the clamp), the
  stage isolation, and `an_unkeyed_program_is_unchanged`: FNV-1a hashes of
  the code and uniform streams of four cube programs (vertex textureCube,
  fragment bias, vertex textureCubeLod, cube + 2D) taken at 0252e9d, before
  the change, equal after it, with and without `report-es2` and with a key
  naming a binding the program has no cube at. Every reference is computed
  in the test (Table 3.21, dEQP's `sampleLinear2D`, `wrap`).
- backend.rs `a_v3d_cube_program_has_a_non_seamless_variant` (both builds):
  the sensitivities (none in an ES 3 build), the variants of both stages
  linking into a program that has `tmui` where the base has `tmuscm`.
- draw.rs `the_cube_key_follows_the_filters`.
- backend.rs `a_vertex_key_alone_relinks_the_base_program`: the draw of the 18
  `texture.vertex.cube` cases (key [k, 0]: the vertex variant linked to the
  retained base program record) gives a program of its own with `tmui` and
  no `tmuscm`, the base untouched, the records balanced.
- v3d/backend_tests.rs `a_program_reading_a_texture_size_gets_the_bindings_size`:
  the size words draw, the table carries the binding's size and the
  stand-in's 1 x 1; tools/v3d/job_check.c part 5: the words read back from
  the indirect buffer, the two refusals.
- v3d/backend_tests.rs (the black stand-in test): the 1x1 cube record has
  `depth` 6, the layout's layer stride is 64 bytes (the C derives the
  record's `array_stride_64` = 1 from it; that test module stubs the
  derivation, so the field itself is not asserted) and six black faces sit at
  that stride, so a TMUI look-up of an incomplete cube reads black.

### Pi 4 verification (ES 2.0 images: v3d_deqp's prj.conf has `CONFIG_YAKOGL_REPORT_ES2=y`)

**The whole tree at `a27d69c0`** (2026-10-10, records
`records/pi4/deqp-a27d69c0/full/`): 17,083 pass / 9 fail / 34 NotSupported.
Of the 25, the 18 `texture.vertex.cube` cases (T1) and
`texture_functions.vertex.texturecubelod` (TF) pass: the TMU takes TMUI as a
float face index, and the vertex stage's explicit level with the
`mag_switch` remap is right. The nine failures:

| Image (row) | Case | dEQP's result |
|---|---|---|
| **T3** (21) | `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}` (6) | Fail, "Image verification failed" |
| **T4** (22) | `texture.size.cube.15x15_{l8,rgb888,rgba8888}` (3) | Fail, "Image comparison failed" (`15x15_rgba4444` passes) |

Both are the fragment stage's level of detail (section "The level of
detail"): the mipmap.cube `nearest_*` variants, which are not keyed (mag
and min NEAREST within a level) and take the hardware cube look-up, pass,
and the size cases passed at sweep 30, before the emulation keyed their
min LINEAR. Fixed on the host by the fragment level (section "As
implemented").

**The run that confirms the fix** (rows are 0-based positions among the
non-comment lines of tools/cts/pi4-full-tree.txt, the `TREE_ROWS` numbers):
T3 (21) and T4 (22) are the nine; T2 (20, `texture.filtering.*`: every
`filtering.cube` case with a LINEAR filter now takes the computed level,
mag LINEAR with min NEAREST_MIPMAP_* among them), TF (24, the fragment
`texturecube*` cases), RT (11, `shaders.random.texture.*`, whose cube
look-ups are fragment ones) and S5f (16) are the fragment cube look-ups
that changed and must not move.

    tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt $IMG T2 T3 T4 TF RT S5f
    TREE_IMAGE_DIR=$IMG TREE_ROWS=11,16,20,21,22,24 tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt $OUT
    tools/board.sh pi4 tally $OUT

Expected: the nine pass and nothing else moves (each image equal to its
`a27d69c0` tally with those cases moved from Fail to Pass); the whole tree
then 17,092 pass / 0 fail / 34 NotSupported. Measured on `1cf946a9`: the nine
pass and nothing else moved. With the 34 cases the `a27d69c0` capture lost,
which pass in a rerun of F1, the tree reads 17,126 pass / 0 fail / 34
NotSupported / 5 warnings (`tools/cts/RESULTS.md`, "Pi 4: the whole dEQP-GLES2
tree, 0 fail"). `v3d_gl_test` is unchanged:
it is an ES 3.0 image (`CONFIG_YAKOGL_REPORT_ES2=n`), which never keys
the emulation.

Reading a failure:

- T3 or T4 still fail with the edge pixels wrong: check that the draw
  keyed the binding (`V3D_VARIANTS_BUILT` grows) and that the fragment
  variant's listing has `fdx`, `fdy`, `tmuslod` (tests/v3d_cube_noseamless.rs
  pins it on the host).
- A draw refused or a job failed with a bad field: the size word
  (`QUNIFORM_TEXTURE_WIDTH`) found no size in the job builder's table
  (`texture_units`, v3d_job.c).
- Only T2's `filtering.cube.nearest_mipmap_*_linear_*` move to Fail: the
  TMU's switch on an explicit fragment level is not GL's c = 0.5 for them;
  give the fragment record a `mag_switch` key for cube bindings and apply
  `lodkey`'s remap to the computed level, as the vertex stage does.
