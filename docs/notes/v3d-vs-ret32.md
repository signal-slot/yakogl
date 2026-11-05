<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: vertex-stage look-ups of the 32-bit texture classes

Design, for the item `v3d-qpu-grad` (its first half). Nothing here is verified
on hardware or by a run: it is read from our tree at `eac7c391` and from the
Mesa 26.2.1 files vendored under `third_party/mesa-26.2.1`. Line numbers
drift; the function names are the address.

The 32-bit classes are R32F, RGBA32F, RGBA32UI and RGBA32I, and the depth
textures, which the GL core stores as R32F (`gles_texfmt_return_32`,
`src/backend.rs:253-255`). The TMU returns these at 32 bits a channel: TMU
Config Parameter 1 bit 0, "Output Type 32-bit"
(`third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml:1543`), and one
`ldtmu` per channel. A fragment look-up of such a texture is built end to end
(`LodKey::ret32`). A vertex-stage look-up of one is refused at the draw. The
refusal is the "vertex-stage look-ups of the 32-bit classes" in
`docs/es3-cts-failures.md` (RC2, "Left", the V3D row, line 127). The fix is
small and sits in four places; the work is the tests.

## Today

The chain, from the draw to the QPU word:

1. **GL core, the key of a stage.** `stage_lod_key` (`src/gl/draw.rs:1733`)
   builds a stage's key from its sensitivity: for each sampler binding of
   the stage's shader whose sensitivity has the `ret32` bit, it sets
   `k.ret32` when the bound texture's format is a 32-bit class
   (`:1747-1749`). It is called for both stages with their own sampler-unit
   tables (`draw_lod_key`, `:1805-1806`), because "the fragment and the
   vertex stage number their bindings apart". So the GL core already does
   the right thing for a vertex binding **if the vertex record's
   sensitivity carries `ret32`**.
2. **The sensitivity of a vertex record is empty of `ret32`.**
   `v3d_sensitivity` (`src/shaderc/online/backend.rs:2038-2046`) sets the
   `ret32` bit only `if fragment && sampler < 16`; for the vertex stage it
   gives the cube bindings and `mag_switch` only. The doc comment says the
   reason: "a vertex look-up of a depth texture is refused at the draw,
   `v3d::texture::refusal_for`" (`:1944-1947`), and a test pins "a vertex
   shader is sensitive to nothing on the V3D" for a vertex shader with no
   look-up (`:2903`). So `lod_sense[0]` has no `ret32` bit and
   `stage_lod_key` never sets it for the vertex stage.
3. **The variant and the link.** A vertex record's variant is built the
   generic way (`online_sampler_variant`, `:2260-2263`: `variant_of` with the
   key in `Facts::lod_key`; the V3D special case at `:2231` is the
   fragment/program record's). The link puts the vertex record's key in the
   program's cache key (`link_program`, `:1738`, bytes 112..120; the fragment's is `:1736`), passes
   it as `vs_key` to `link_sources` (`:1763`), and from there it reaches
   `compile_program_keyed` as `vs_key` (`src/shaderc/online/v3d/chain.rs:380`).
   On the V3D a vertex key rebuilds the whole program, because the program
   holds both stages' code (`fs_rebuilt`, `src/gl/draw.rs:1855-1857`). All of
   this is in place for `cube_noseam` and `mag_switch`.
4. **The compiler ignores the key's `ret32` in the vertex stage.**
   `compile_program_keyed` reads `vs_key.cube_noseam` only
   (`chain.rs:452-453`). `vertex()` (`chain.rs:258-264`), called for the
   vertex shader and for the coordinate shader (`:495-496`), takes no key.
   `select_vertex` (`src/shaderc/online/v3d/select.rs:2370`) builds its `Sel`
   with `ret32: 0` (`:2412`); the field's doc says "fragment stage only: the
   two stages number their bindings apart, and the key is the fragment's"
   (`:474-476`). `select_fragment` takes the ambient fragment key,
   `compile_lod_key().ret32` (`:2166`), which `link_program` set to
   `fr.lod_key` (`backend.rs:1759-1761`). So a vertex look-up always has
   P1 = `unit | 4` (`select.rs:934`), a two-word P0 mask
   (`words |= 1 << (lane / 2)`, `:910`) and the f16 unpack (`:1012-1016`).
5. **The back end refuses.** `texture_units` runs once per stage
   (`src/v3d/backend.rs:3694-3695`, with the vertex stage's two streams, the
   vertex and coordinate shaders', `:3687`). It folds the stage's P1 words
   into `ret32_units` (`:5290-5298`, `data & 1`, unit in bits 24..31), so
   here it finds 0, passes `ret32 = false` to `tex_state_for` (`:5307-5309`),
   and `texture::refusal_for` returns the "texture of 32-bit channels ...
   read by a look-up compiled for the 16-bit TMU output" refusal
   (`src/v3d/texture.rs:143-150`). The draw is refused as a whole
   (`Attempt::Refused`, `backend.rs:4115-4118`). The refusal is correct: the
   16-bit read of a 32-bit format is "not a measured behaviour"
   (`texture.rs:120-123`). The back end needs nothing to read a keyed vertex
   look-up: the fold is per stream and sees the coordinate shader's stream
   too (`:3687`), and the incomplete-texture branches already answer for
   `ret32` (`:5310`, `:5342`, `:5381`; `black_typed_state`).
6. **What is shared, so not changed.** The select path of a 32-bit look-up
   is stage-independent once `Sel::ret32` is set: the word mask (`:905-910`),
   the P1 bit (`:934`), the `(tex, words, ret32)` bookkeeping (`:983`,
   `:1001-1010`, `:1571-1580`) where lane `l` is word `l`, the integer
   classes taken raw (`:1008-1010`, no unpack), and `sim.rs` models P1 bit 0
   (`sim.rs:150-151`). A vertex look-up is always an explicit level
   (`TexLod` implicit becomes `Lod(0.0)` in the vertex stage, as
   `docs/notes/v3d-shadow.md` item 6 says of `prepare.rs:1358-1361`), so its
   retiring write is TMUSLOD (`select.rs:925-935`), or TMUSCM for a cube.

So three things are missing, one per link of the chain: the sensitivity of the
vertex record (2), the key reaching `select_vertex` (4), and the tests.

## (a) What Mesa does

Vendored: `src/gallium/drivers/v3d/v3dx_format_table.c` and
`src/broadcom/cle/v3d_packet.xml` under `third_party/mesa-26.2.1/`. The
compiler sources (`v3d_tex.c`, `v3d_program.c`, `nir_to_vir.c`) are **not**
vendored: `third_party/mesa-26.2.1/src/broadcom/` holds `qpu/`, `cle/`,
`clif/` and `common/` only. The compiler facts below are the ones this repo
already cites as "(tree)" (`docs/notes/v3d-shadow.md`, `src/shaderc/online/
v3d/select.rs` comments, `v3d_tex.c:250-252, 303-306, 317, 374-375`), plus
two statements of Mesa that are not yet checked in this repo and are marked
so. `tools/v3d/README.md` builds the full tree for `v3d_shc`.

1. **The return size is a property of the texture's format, per stage.** The
   gallium table gives each format a `return_size` and `return_channels` (the
   macro, `v3dx_format_table.c:37-45`): R32F `32, 1` (`:91`), RGBA32F `32, 4`
   (`:101`), R32I / R32UI `32, 1` (`:132-133`), RGBA32I / RGBA32UI `32, 4`
   (`:136-137`), and the depth formats `32, 1` (D24S8 as DEPTH24_X8, `:149-150`;
   D32F, `:152, :156`). The driver copies it into the shader key's per-sampler
   entry for each stage that samples (`v3d_setup_shared_key`, the `v3d_key`
   shared by the vertex, coordinate and fragment keys: *not checked here*,
   `v3d_program.c`). Mesa has no "fragment only" rule: the vertex shader of a
   program is keyed with the vertex stage's samplers.
2. **The look-up reads the key, not the stage.** `v3d40_vir_emit_tex`
   (`v3d_tex.c`) sets `output_type_32_bit` in P1 from the key's
   `return_size == 32` for the sampler, and limits the return-words mask to
   the channels the NIR def reads (`v3d_tex.c:250-252`). It tests the stage
   only for the per-pixel mask (`:317`, as `select.rs:928-931` says),
   for the implicit-derivative forms, which a vertex shader does not have,
   and for the final write (`txl`: TMUSLOD, `:374-375`). So a vertex
   look-up of an R32F sampler is P0 mask (the read channels), P1
   `output_type_32_bit | per_pixel_mask_enable`, T, B (the level), S; the
   same words as the fragment look-up of that sampler at an explicit level.
3. **The results.** With a 32-bit return each `ldtmu` returns one channel
   (4 for a vec4); with 16-bit, two per word, unpacked by
   `unpack_half_2x16_split` (`select.rs:1012-1016` is ours). Integer formats
   at 32 bits are the raw word (no `nir_lower_tex_packing_16` unpack, which
   applies to 16-bit returns only: `select.rs:1017-1030`).
4. **The TMU output FIFO.** The output FIFO holds `TMU_OUTPUT_WORDS` words
   (`select.rs:1871`, `16 / 4`; Mesa's `ntq_tmu_fifo_overflow`,
   `nir_to_vir.c:251-258`, *not checked here*). A vec4 of 32-bit words is 4
   words, a whole FIFO, where the f16 vec4 is 2. Our rule `queued +
   words.count_ones() > TMU_OUTPUT_WORDS` (`select.rs:922`) then flushes
   before a second look-up is requested, for the vertex stage as for the
   fragment's. Whether Mesa flushes at the same place in a vertex shader is
   one of the numbers to measure (c).
5. **What the reference tool cannot say today.** `tools/v3d/v3d_shc.c` zeroes
   every key (`:891-913`), so every sampler is return size 0 and P1 bit 0 is
   always clear in its output: there is no Mesa word for any 32-bit
   look-up, fragment or vertex, in `tests/fixtures/v3d_mesa_programs.txt`.
   `tests/v3d_sim_mesa.rs` has only a mutant of the bit (`P1 output type`,
   `:2056`). The reference needs a switch on the tool to set the sampler
   key's return size per stage; the item `pi4-fbo-render-probe` records the
   words and counts Mesa makes for R32F, RGBA32F, RGBA32UI and RGBA32I
   vertex look-ups, and this note's numbers wait for it.

## (b) The change

Four edits, in the order of the chain; none needs a new key field.

1. `src/shaderc/online/backend.rs`, `v3d_sensitivity`: set the `ret32` bit for
   a look-up in either stage, `sampler < 16`, which is the line
   `if fragment && sampler < 16` (`:2040`) without `fragment &&`. Update the
   doc comments of `sensitivity_of` (`:1944-1947`) and `v3d_sensitivity`
   (`:2016-2020`), which say fragment only, and the test comment at `:2903`
   (a vertex shader *without a look-up* is sensitive to nothing).
2. `src/shaderc/online/v3d/chain.rs`: `vertex()` (`:258`) takes the vertex
   key (or its `ret32`), `compile_program_keyed` passes `vs_key` at both
   calls (`:495-496`, the vertex and the coordinate shader: the same bits,
   since the driver loads one set of texture records for both).
3. `src/shaderc/online/v3d/select.rs`: `select_vertex` (`:2370`) takes the
   `ret32` mask and sets `Sel::ret32` (`:2412`) from it. **Not** from
   `compile_lod_key()`: that ambient key is the fragment's (the stages
   number their bindings apart), and reading it would key vertex binding 3
   by the fragment's binding 3. Fix the field doc (`:474-476`), keep the
   fragment's read at `:2166`.
4. `src/gl/draw.rs`: none, `stage_lod_key` is stage-agnostic (`:1747`); the
   doc at `:1728-1732` already says so. `src/v3d/backend.rs` and
   `src/v3d/texture.rs`: none (Today, 5). The `refusal_for` text stays: it
   now fires only for a draw whose vertex stage is not keyed (a variant that
   failed to build, `lod_variant_for`'s fallback to the program's own
   shaders, `src/gl/draw.rs:1862-1866`), which is the right answer.
5. `docs/es3-cts-failures.md` (RC2 "Left", `:127`) and
   `docs/es3/backend-caps.md` (the 32-bit sampled rows, `:258`): drop the
   vertex clause when the item lands.

Caveats the edits must keep:

* The key is by **binding** and a vertex binding and a fragment binding with
  the same number are different textures: in `link_program`'s cache key the
  two stage keys are separate words (`:1736-1738`); nothing is merged.
* The program holds a vertex key bit per vertex binding below 16 that the
  vertex shader looks up, whether or not the bound texture is a 32-bit class
  (`stage_lod_key` clears the bit for a 16-bit texture). A shader whose
  vertex stage samples an RGBA8 texture is unchanged: key 0, no variant.
  A vertex stage that now has a sensitivity makes `lod_sense` non-zero for
  programs that had `[0, 0]`; `draw_lod_key` then builds a key per draw
  (cheap: it reads the bound formats), `lod_variant_for` finds the key 0
  as "no variant".
* A vertex key rebuilds the whole V3D program (`fs_rebuilt`): one variant
  slot per (vertex key, fragment key) pair, up to `PROG_MAX_LOD_VARIANTS`.
* The vertex stage's shader memory: `ret32` makes a look-up take more
  registers (4 words instead of 2) and, for a program that already presses
  the vertex stage, a retry at the next `Pressure` step
  (`PRESSURES`, `chain.rs:231`; `under_pressure`, `:241`). This is the existing path.

## (c) Test plan

Host-only, in the gates that already run: `tests/es3_shaderc_v3d.rs` (the
online compiler through `SHC_ONLINE`, `cargo test ... --test es3_shaderc_v3d`
as its header says) and `tests/v3d_sim_mesa.rs` (Mesa's code on our
interpreter, a `gpu-v3d` row).

`src/` unit tests, next to the code they pin (the compiler's own tests are
the first gate; they are not in the contract of this note, they are the
revert checks of the edits):

* `a_ret32_binding_reads_one_32_bit_word_per_channel` (`chain.rs:3508`) gets
  a vertex twin: the same program with the look-up in the vertex shader
  (`texture2DLod(s, p.xy, 0.0)`, ES 1.00), compiled with a vertex key
  `ret32 = 1`: P0 `0xf`, P1 `0x5` in `vs.uniforms` and in `cs.uniforms`, four
  `ldtmu`, no `.l` / `.h` in the vertex code; with key 0 the old `0x3` /
  `0x4`; with key bit 2 at a binding it does not read, unchanged. Revert
  checks: `Sel::ret32` left 0; the coordinate shader not keyed.
* `a_v3d_program_has_a_ret32_variant` (`src/shaderc/online/backend.rs:2879`)
  gets the vertex half: the vertex record's `online_lod_sensitivity` is
  `ret32: 1` for a vertex shader with a look-up at binding 0 and 0 without,
  and the variant built for `LodKey { ret32: 1 }` of the vertex record links to
  a program whose `vs` stream has P1 `0x5`.
* `src/v3d/backend_tests.rs` has the fragment pair of the draw:
  `a_depth_textures_look_up_must_take_the_32_bit_output` (`:9007`) and
  `ret32_program` (`:8715`); add the vertex draw: an R32F (and an RGBA32F)
  texture in `GlesDraw::vs_textures`, a program whose `vs` stream has P1
  bit 0, accepted with the record's type R32F / RGBA32F; the same texture with
  a 16-bit vertex stream refused with the existing text. A vertex draw
  through the coordinate stream: the binning shader's stream carries the same
  bit.

`tests/es3_shaderc_v3d.rs` (compile and link through `glCompileShader` /
`glLinkProgram`, the file's `build`):

* `vertex_ret32_builds`: a GLSL ES 3.00 vertex shader `textureLod` (and
  `texelFetch`, which is also an explicit-level form) on `sampler2D`,
  `isampler2D`, `usampler2D` and a `sampler2DShadow`, links with a
  fragment shader; the program's vertex key is 0 at link (the key comes
  with the draw), so these assert **no regression** of the build, and the
  keyed variant is asserted through `online_sampler_variant` in the `src/`
  tests above.
* `vertex_ret32_two_look_ups`: two 32-bit vec4 look-ups in one vertex shader
  (the FIFO rule of (a) 4): builds, and the first look-up's four
  `ldtmu` come before the second's P0 write.

`tests/v3d_sim_mesa.rs`, and the Mesa programs it compares against
(`v3d_shc`, `tools/v3d/v3d_shc.c`, `tests/fixtures/v3d_mesa_programs.txt` by
`tools/v3d/sim_fixture.py`):

* The fixture's stage entries need the return size of each sampler
  (`v3d_shc.c` gets a `--vs-ret32 MASK` / `--fs-ret32 MASK` switch that sets the
  per-stage `key.sampler[i].return_size = 32`, `return_channels` 1 or 4 by
  the format of the program's name; default 0, so no existing word moves).
  Mesa programs to add, one vertex + fragment pair each, vertex stage
  `textureLod` at binding 0 and a constant-lane coordinate form (as
  `mali-vs-texture-lanes` for the Mali):
  `vsfetch_r32f` (`sampler2D`, 1 channel read: P0 `0x1`),
  `vsfetch_rgba32f` (`sampler2D`, vec4 read: P0 `0xf`),
  `vsfetch_rgba32ui` (`usampler2D`), `vsfetch_rgba32i` (`isampler2D`),
  a two-look-up vertex shader of R32F and RGBA32F on different units, and a
  vertex shader with a 16-bit look-up beside a 32-bit one (the per-sampler
  mask). Their source goes to `tools/shaders/` beside the other `vs` look-up
  shaders; the fixture is regenerated in the same commit (the generated
  files rule).
* `the_interpreter_runs_mesas_code_as_the_ir_says` (`:1815` region) then runs
  Mesa's vertex and coordinate code with 32-bit textures
  (`sim::Texture` already holds 32-bit channels, `sim.rs:303-306`) against
  the IR evaluator; `tmu_mutants` (`:1963`) already picks the vertex shader
  when only it samples, so the `P1 output type` mutant (`:2056`) becomes
  visible on a vertex look-up as soon as the fixture has one; pin its count.
* The comparison our compiler's output against Mesa's: instruction counts of
  the new pairs next to `tools/v3d/qpu_counts_42.tsv` and the word-for-word
  equality or the named difference, as the other texture forms are recorded.

Also, off-host and not part of this item: a gl_test case with a vertex
texture of R32F (a displacement-map shape, ES 3.0 vertex texture fetch of a
float texture) on the Pi 4, once the host tests pass; the dEQP group is
`texture.vertex` (`docs/es3-cts-failures.md`, 89 failures, mostly 3D).

## What the note leaves open

* The two Mesa statements marked "not checked here" in (a) 1 (the key is
  filled per stage with the format's return size) and (a) 2 (`output_type_32_bit`
  from the key) are read from memory of `v3d_program.c` / `v3d_tex.c`; the
  tree is not vendored. `pi4-fbo-render-probe` builds `v3d_shc` and shows the
  words; if P1 of a vertex look-up differs from `unit | 4 | 1` the edit (b) 3
  follows the words.
* The FIFO rule for the vertex stage ((a) 4) is unmeasured; the
  vertex shader of Mesa may flush before a second 4-word look-up where ours
  does not, or the other way round.
* Whether a 32-bit vertex look-up at a **linear** filter is ever asked for:
  ES 3.0 requires NEAREST for these classes (`docs/es3/backend-caps.md:261`),
  and the GL core enforces it; the sampler word for a vertex binding is the
  same `sampler_for` as the fragment's. Nothing is done for LINEAR on an
  incomplete 32-bit texture (a texture the GL core calls incomplete reads
  the black stand-in, `black_typed_state`).
* `textureGrad` in the vertex stage lowers to an explicit level
  (`docs/notes/v3d-qpu-grad.md`); it composes with this note's edit with no
  change of its own (the level is a TMUB write in either size), and its
  tests should use the keyed vertex shape of (c).
* A shader that samples one binding at two different return sizes cannot
  occur (a sampler has one texture). A texture unit bound for both stages
  with different binding numbers is two keys by construction ((b) caveats).
* No hardware: a board result (gl_test or dEQP `texture.vertex`) would
  confirm that the TMU output order for 32-bit words in a vertex shader is
  the fragment's.
