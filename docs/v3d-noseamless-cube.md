<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Pi 4: non-seamless cube sampling in ES 2.0 builds (design)

`c3e4146d`, 2026-10-07. The design and the host reproduction of
the Raspberry Pi 4's 25 remaining dEQP-GLES2 failures; `ec5ba4fb` (same
day) implements it. Status: implemented and verified on the host; the
Pi 4 run is open (section "As implemented").

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
- Fragment stage, implicit: the TMU derives the level from the quad's
  derivatives of the per-face (s, t) it is given. Inside a face this is the
  cube's own LOD (ES 2.0 3.7.7 applied to the projected coordinates, which
  is what dEQP's `computeCubeLodBoundsFromDerivates` computes). In a quad
  whose four pixels do not all select the same face, the derivative of (s,
  t) across the face jump is wrong and the level too coarse on those pixels:
  the classic non-seamless seam band, which every ES 2.0 GPU with per-face
  sampling had. dEQP's `mipmap.cube` cases draw each face as its own quad
  with the face's corner directions, so no quad straddles a face. Fallback,
  if a board run shows a band failure: compute the level in the shader from
  FDX / FDY of the direction (the V3D has both, `select.rs` dFdx / dFdy)
  with the projected-derivative formula dEQP uses, add the bias, and retire
  with TMUSLOD: exact per pixel, ~25 more operations, fragment variants only.
- A bias (`textureCube(s, d, b)`): TMUB = b, TMUS retire, as a 2D bias
  look-up today.

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
   **docs/HANDOFF.md** #12; tools/cts/RESULTS.md's "Reference-selection
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
  FTOIN of the face.
- The seam band of the fragment stage's implicit LOD (section "The level of
  detail"), and the TMU's switch between magnification and minification
  (0 or 0.5). Both are the board recipe's.
- LINEAR weights: the model's are f32; the board measured the TMU's
  (gl_test case 109). dEQP's tolerance covers the difference.

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
- Not done: a 16th cube sampler of a stage stays seamless (bit 15), and
  the fragment-stage FDX / FDY level fallback is not written (board-gated,
  above).

### Host evidence

- tests/v3d_cube_noseamless.rs (both builds, 7 tests): every wrap pair on
  eight directions over all six faces, the CLAMP / MIRROR / REPEAT edge
  case against the seamless value, explicit levels (-1, 0.25, 1, 1.5) with
  LINEAR_MIPMAP_LINEAR / NEAREST_MIPMAP_LINEAR / LINEAR on a 3-level cube,
  a projected direction (`a_dir.xyz / a_dir.w`), a fragment look-up with a
  bias (TMUB, TMUI, TMUS) on a one-level texture (the interpreter cannot
  derive an implicit level, so bias on a mipmapped cube is the board's), the
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
- v3d/backend_tests.rs (the black stand-in test): the 1x1 cube record has
  `depth` 6, the layout's layer stride is 64 bytes (the C derives the
  record's `array_stride_64` = 1 from it; that test module stubs the
  derivation, so the field itself is not asserted) and six black faces sit at
  that stride, so a TMUI look-up of an incomplete cube reads black.

### Pi 4 verification (one session; ES 2.0 images: v3d_deqp's prj.conf has `CONFIG_YAKOGL_REPORT_ES2=y`)

Where the 25 cases are, in tools/cts/pi4-full-tree.txt (row = its 0-based position
among the non-comment lines, the `TREE_ROWS` number):

| Image (row) | Filter | Cases | The cube cases in it |
|---|---|---|---|
| **T1** (19) | `texture.vertex.*` among others | 181 | 18: `texture.vertex.cube.filtering.*` 10, `texture.vertex.cube.wrap.*` 8 |
| **T3** (21) | `texture.mipmap.*`, `texture.specification.*` | 304 | 6: `texture.mipmap.cube.{basic,bias,projected}.{linear_linear,linear_nearest}` |
| **TF** (24) | `shaders.texture_functions.*` | 17 | 1: `shaders.texture_functions.vertex.texturecubelod` |

Build the three images (ZEPHYR_WS, DEQP_WORK, ZEPHYR_SDK_INSTALL_DIR as the
header of tools/cts/pi4-deqp-images.sh says; `$IMG` must not exist yet) and
run them, either with the tree runner, which waits for each image's
`<label>.built` line and tallies at the end:

    tools/cts/pi4-deqp-images.sh tools/cts/pi4-full-tree.txt $IMG T1 T3 TF
    TREE_IMAGE_DIR=$IMG TREE_ROWS=19,21,24 tools/board.sh pi4 tree tools/cts/pi4-full-tree.txt $OUT
    tools/board.sh pi4 tally $OUT

or image by image (`<image>` is `$IMG/<LABEL>/zephyr/zephyr.bin`):

    tools/board.sh pi4 run $IMG/T1/zephyr/zephyr.bin $OUT/T1     # likewise T3, TF
    tools/board.sh pi4 tally $OUT/T1                              # likewise T3, TF

Expected: the 25 cases above are Pass (dEQP's "Passed"), and nothing else
moves: T1, T3 and TF each equal their sweep-30 tally with those cases moved
from Fail to Pass; for the whole tree (all 29 images,
`TREE_PREV=<sweep 30's tree> tools/board.sh pi4 tree ...` diffs against it)
17,100 pass, 0 fail, 62 not supported, 3 quality warnings. Then `v3d_gl_test`
unchanged, 21/21 (12/12 on the older record): its cube cases use NEAREST,
which the GL core never keys.

Reading a failure:

- All 25 fail: the TMU did not take TMUI as a float face index (the
  assumption above). Add one FTOIN of the face in `prepare.rs`
  (`Walk::cube_face`'s caller), rebuild, run T1 `texture.vertex.cube.wrap.clamp_clamp` alone.
- Only the six `texture.vertex.cube.filtering.nearest_mipmap_{nearest,linear}_linear_*`
  fail: the c = 0.5 switch is wrong for the TMU. The remap is wired
  (`LodKey::mag_switch` in the vertex key); check the vertex key reaches the
  link (`V3D_VARIANTS_BUILT` in the back end's print grows) before
  suspecting the remap itself.
- Only the six `texture.mipmap.cube.*` fail, with invalid pixels along face
  edges in the QPA: the fragment stage's implicit level needs the FDX / FDY
  fallback (section "The level of detail"); write it in `prepare.rs` for
  fragment variants only.
