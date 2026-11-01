<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2: shadow look-ups (sampler compare and compiler)

Design, for item `v3d-qpu-shadow`. Today the V3D online compiler refuses every
shadow look-up by name (`src/shaderc/online/v3d/prepare.rs:1335-1337`,
`NotYet("a shadow look-up")`), although the GL core already hands the compare
state to the back end (`GlesTextureBind::compare_op`, `src/backend.rs:863-871`).
The hardware needs two things, and they are independent: a compare function in
the sampler record, and one extra TMU write (the reference) in the look-up.
This note gives the words, the size question, the operands, the formats that
may be compared and the tests. The TMU words of the other ES 3.0 look-ups are in
`docs/notes/v3d-tmu-es3.md` and the memory layouts in
`docs/notes/v3d-tex-layout-es3.md`; neither is repeated here.

Sources. `third_party/mesa-26.2.1` holds only the packet XML and the format
table of Mesa's V3D (`UPSTREAM`, `src/broadcom/cle/v3d_packet.xml`,
`src/gallium/drivers/v3d/v3dx_format_table.c`). The compiler (`v3d_tex.c`) and
the state code (`v3dx_state.c`) are in the same 26.2.1 tarball, which the deps
cache holds extracted (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, the tree
`tools/v3d/README.md` builds `v3d_shc` in); their paths below are relative to
that tree and marked "(tree)". Line numbers are of Mesa 26.2.1 and of main at
`ff7b9882`, and drift in ours; the names are the address.

## (a) The sampler words

The SAMPLER_STATE record of V3D 4.2 is 24 bytes (`v3d_packet.xml:1709-1731`).
The compare lives in one 3-bit field, "Depth Compare Function", at bits 4-6 of
the record's first 32-bit word (start 4, size 3, `v3d_packet.xml:1728`), between the sRGB
disable bit (7) and the filter bits (0-3). Its enum is the usual eight
(`v3d_packet.xml:3-12`):

| field value | function | GL name | `compare_op` bits 0-2 |
|---|---|---|---|
| 0 | NEVER | `GL_NEVER` | 0 |
| 1 | LESS | `GL_LESS` | 1 |
| 2 | EQUAL | `GL_EQUAL` | 2 |
| 3 | LEQUAL | `GL_LEQUAL` | 3 |
| 4 | GREATER | `GL_GREATER` | 4 |
| 5 | NOTEQUAL | `GL_NOTEQUAL` | 5 |
| 6 | GEQUAL | `GL_GEQUAL` | 6 |
| 7 | ALWAYS | `GL_ALWAYS` | 7 |

The GL core builds `compare_op` as `func - GL_NEVER` for a function in
`GL_NEVER..=GL_ALWAYS` under `GL_COMPARE_REF_TO_TEXTURE`, else 16
(`src/gl/draw.rs:678-688`), with bit 3 added for a fixed-point depth
(`src/gl/draw.rs:685-688`). GL's function numbers run in the same order as the
V3D's, so bits 0-2 are the field value unchanged; no table is needed. This is
also what Mesa does: it stores the Gallium function as it is and writes NEVER
when the compare mode is off (`src/gallium/drivers/v3d/v3dx_state.c:598-600`
(tree)). So the mapping is:

- `compare_op & 16` (no compare): field 0 (NEVER), as Mesa's compare-off word.
  The shader of a non-shadow sampler never writes the reference, so it reads
  the depth itself (see (c)); a shadow look-up on such a texture is undefined
  in GLSL ES 3.00 (`src/gl/draw.rs:668-672` says the same) and reads 0 here.
- otherwise field = `compare_op & 7`; bit 3 does not reach the record (see
  (d), it is a reference clamp).

The record's byte count does not change (24, `driver/src/v3d/v3d_tex.h:260`).
`v3d_tex_pack_sampler` leaves the field at 0 today (comment "depth compare NEVER
(0)", `driver/src/v3d/v3d_tex.c:628`); the item sets `p.depth_compare_function`
in the same struct it already fills (`v3d_tex.c:624-635`).

Direction. Nothing in the files read says "reference OP texel"; Mesa copies
the Gallium function, which the state tracker takes from GL (`GL_LESS`: 1
when Dref < Dt, ES 3.0.6 3.8.15.1). That the straight copy has the same
direction is an inference, not a cited fact (the state tracker is not in
the tree). The test plan (e) pins it on the interpreter and
with one board case, so nobody has to take it on trust.

## (b) Does `V3dSampler` grow?

Yes. `V3dSampler` (`src/v3d/ffi.rs:562-574`) is three `u32` wraps, four
`bool`s, three `f32` and a `u32`: 12 + 4 + 12 + 4 = 32 bytes with no padding,
and `ffi.rs:888-892` asserts the size 32 and the offsets of `min_nearest` (12),
`mip_none` (15), `min_lod` (16) and `border_color_mode` (28). It has no field
for the compare, and the sampler cache needs one: `sampler_for` finds a record
again by whole-struct equality (`src/v3d/backend.rs:5163-5185`), so two
bindings that differ only in compare must differ as keys, or the second one
would be given the first one's record.

Smallest change: add `compare: u32` (the field value 0-7) after
`border_color_mode`. The struct becomes 36 bytes, 4-aligned; changed together:

- `src/v3d/ffi.rs`: the field, the size assert (36) and an offset assert (32).
- `driver/src/v3d/v3d_tex.h:265-272`: the same field in `struct v3d_sampler`
  (the Rust struct mirrors it field for field); `v3d_tex_pack_sampler` checks
  `compare <= 7` and returns false otherwise, like its other fields
  (`v3d_tex.c:594-597`), and sets the packer field.
- `src/v3d/texture.rs:267-280` (`sampler`): `compare` from `b.compare_op`
  (`16 -> 0`, else `& 7`).
- `src/v3d/backend.rs:970-982`: the zero key of `sampler_keys` is a struct
  literal that lists every field; it needs the new one (or `..Default`). That
  literal is the only one in `src/` and `tests/`; the test files declare the C
  function with a `*const V3dSampler` and never build one
  (`grep -rn 'V3dSampler {' src tests`). C struct users in
  `tools/v3d/tex_check.c` use designated initialisers (`:376-401`, `:709`), so
  the new field defaults to 0 there and nothing breaks.
- the sampler cache holds 16 keys of 36 bytes instead of 32
  (`SAMPLER_CACHE`, `backend.rs:93`, `:906`): 64 more bytes of state, no
  other cost.

Alternative that avoids the size change: pack `compare` into the three
`bool`s' neighbour by turning two of the `bool`s into `u8` bit flags. It saves
four bytes and costs a rewrite of every reader of `min_nearest`/`mag_nearest`
(`texture.rs:267-280`, `v3d_tex.c:625-627`, the asserts, `tex_check.c`); not
worth it for a 16-entry cache. Take the 36-byte struct.

The uniform stream does not change. The compare is data in the sampler
record, which P1 points at (`TMU Config Parameter 1`, sampler state address in
bits 31..0, `v3d_packet.xml:1539-1544`; `v3d_tex_p1`, `v3d_tex.h:290`), so one
shader serves every compare function. No shader key is needed for the
function. (The fixed-depth clamp in (d) is the one exception, and it is
optional.)

## (c) The compiler side

What the GL front end gives. `Op::Tex { shadow: true, .. }` carries the
reference in the coordinate: lane 2 (`z`) for `D2`, lane 3 (`w`) for `Cube`
and `D2Array` (`src/shaderc/online/ir.rs:408-409`; the evaluator reads them the
same way, `src/shaderc/online/eval.rs:699-700`). `proj` divides every lane up
to the reference by `w`, so for a projective 2D shadow the reference is
divided too (`eval.rs:740-745`). The value is one `F32` lane
(`ir.rs:411-412`; `builtin_call.rs:747-749` sets `ret` to `F32` for a shadow
type whatever the sampler returns).

What Mesa emits. The reference is one more TMU register write, `TMUDREF`
(`handle_tex_src`, `nir_tex_src_comparator`, `src/broadcom/compiler/v3d_tex.c:
155-158` (tree); the address is `V3D_QPU_WADDR_TMUDREF = 38`,
`src/broadcom/qpu/qpu_instr.h:127`). There is no P1 or P2 bit for it: P2's
fields in 4.2 are the op, the offsets, the gather and coefficient modes,
sample number, autolod disable and LOD query (`v3d_packet.xml:1560-1573`), and
a regular shadow look-up leaves P2 at its default, so it needs no P2 word
unless it also has an offset, a gradient or a fixed level (the cases in
`docs/notes/v3d-tmu-es3.md`). Order: the coordinate writes T, R, I, then each
other source in the order NIR lists them, S last because S retires the
look-up (`v3d_tex.c:100-130`, `:370-380` (tree)); the DREF write is an ordinary
one among the earlier writes, so the order against bias or level does not
matter, only that it comes before S. Each write counts toward the 16-slot TMU
input FIFO (`get_required_tex_tmu_writes`, `v3d_tex.c:219-223`, `:264-270`
(tree)): our `select.rs` has the matching counter in the Tex arm and must add
one for the reference, or a 4-thread program can overflow the FIFO.

Per form (our write order, from `select.rs:864-935`, with DREF added before
the retiring write):

| form | writes before the retiring write | retiring write |
|---|---|---|
| 2D `sampler2DShadow` | T, [B], DREF | `TMUS` (`TMUSLOD` with an explicit level) |
| cube `samplerCubeShadow` | T, R, [B], DREF; P2 disable-autolod when the level is explicit | `TMUSCM` |
| 2D-array `sampler2DArrayShadow` | T, I (layer), [B], DREF | `TMUS` (`TMUSLOD` with a level) |
| cube through the no-seamless layers (`cube_noseam`) | T, I (the face), [B], DREF | `TMUS`; see the caveat below |

`[B]` is the bias or level (`TMUB`). The array row depends on the 2D-array
item (`v3d-tex-3d-array`, `v3d-qpu-array-3d`); the shadow item adds only DREF
to whatever write sequence the array item gives it.

Changes in our compiler (all in `src/shaderc/online/v3d/`):

1. `prepare.rs:1335-1337` stops refusing. The reference is `self.src(coord, 2)`
   for `D2` and `self.src(coord, 3)` for `Cube` (the same lanes `ir.rs`
   documents). For `proj` the existing multiply by the reciprocal of `q`
   (`prepare.rs:1367-1373`) must also be applied to the reference, as the
   evaluator does (`eval.rs:740-745`).
   For a cube, the reference is not divided by the major axis: only the
   direction is (`prepare.rs:1391-1420`), and the reference is a depth, not a
   direction lane.
2. `SOp::Tex` (`prepare.rs:103`) needs a way to carry the reference. It has
   `srcs: [S; 4]` of s, t, third, bias/level; a shadow look-up has up to five
   operands (s, t, third, bias/level, ref). Either widen `srcs` to five and
   make `tex_operands` (`prepare.rs:2265-2267`) take a `shadow: bool` (the
   readers are `prepare.rs:171`, `:2214`, `select.rs:837-841`), or add a
   `dref: Option<S>` field; the first keeps every consumer of `srcs` in one
   place. `tex_operands` is also what keeps the operand a value in `select.rs`
   (`as_value`, `:837-841`), so the reference is kept alive without further
   work.
3. `select.rs` writes `WADDR_TMUDREF = 38` (new constant next to
   `WADDR_TMUB`, `:123-126`) after T/R/I/B and before the retiring S write
   (`select.rs:905-935`). The sim already names the register in its header
   comment (`sim.rs:141`) but refuses it (`sim.rs:186-187`), and `chain.rs`
   reports a write to TMUDREF as a form this back end does not select
   (`chain.rs:804-808`): its check must accept address 38 inside a look-up's
   stretch, as it accepts TMUI and TMUB.
4. What comes back: one value. The depth formats are return size 32
   (`v3dx_format_table.c:149-156` (vendored), all rows `32, 1`), so the key is
   always `ret32` (P1 bit 0, `v3d_packet.xml:1543`) and one `ldtmu` returns
   the compared result as an f32 in word 0. Mesa sets the return-words mask
   from the channels the NIR def reads (`v3d_tex.c:250-252` (tree)), a shadow
   def has one component (the compare result), so `return_words` is 1 and our
   `words` mask is 1 (`select.rs:870-880`). `prepare.rs` pushes one `TexLane`
   lane 0 and puts the other three lanes of the `out` array to the same value
   or to `None` according to what the front end reads (a shadow value has
   one lane, `ir.rs:411-412`).
5. The 32-bit key. `backend.rs` of the online compiler derives `ret32` from
   the bound textures (`src/shaderc/online/backend.rs:2035-2047`); a depth
   texture is R32F in the GL core (`texture.rs:167-178`), so it is `ret32`
   already for a depth sampler. Nothing new, but the tests must cover the
   D24S8-buffer binding too, which is `ret32` through
   `depth_buffer_state` (`texture.rs:317-325`).
6. A vertex-stage shadow look-up has no derivatives: the existing
   `Lod(0.0)` fill (`prepare.rs:1358-1361`) applies and the form is
   `TMUB`+`TMUSLOD`+DREF; no other change.
7. `sim.rs` needs `TMUDREF` (take the lane values, queue them with the other
   coordinates) and a compare in its sampler entry
   (`texref::Sampler`, `src/shaderc/online/texref.rs:230-236`, has no compare
   field; `TexLookup::dref` already exists on the evaluator side,
   `eval.rs:721-722`, so the model in `texref.rs` should do the compare and
   both the evaluator and the sim call it). `sim.rs:186-187` stops listing
   DREF as refused.

Caveat, cube through the no-seamless layers. With `cube_noseam` the compiler
looks a cube up as a layer of a 2D-array record (`select.rs` writes TMUI for
`layer`, `prepare.rs:1426-1437`). A shadow look-up at a face edge then
compares within one face only, with no cross-edge taps: acceptable for
NEAREST and for GL's own PCF-less answer, wrong by a tap only for LINEAR at
the very edge (dEQP's `texture.shadow.cube` tolerates it, as the
QualityWarning rows for the 2D case show, `docs/es3-cts-failures.md:104`). An
ES 3.0 build keys cubes seamless (the `cube_noseam` bits are ES 2.0's,
`docs/v3d-noseamless-cube.md`), so the first version need only refuse
`shadow && layer` with its own name if it turns out to matter.

What the hardware returns for LINEAR. The hardware compares first and then
filters (percentage-closer), which is what GL specifies for a shadow sampler
(ES 3.0.6 3.8.15.1); the interpreter's model does the same by construction
(compare per texel, then the bilinear weights). Whether the TMU's fixed-point
weights make the LINEAR result differ from the float reference by a few ulps
is measured, not read: the `gl_test` 109 note for ordinary LINEAR
(`sim.rs:176-181`) applies, and dEQP's tolerance is what the check uses.

## (d) The depth formats that may be compared

The GL core keeps every depth texture as R32F texels; a texture a draw has
rendered into is read from the back end's D24S8 buffer instead
(`docs/notes/v3d-depth-storage.md`, `src/v3d/backend.rs:4184-4213`). So there
are two bindings to make comparable, and they are different TMU types:

| binding | TMU type today | compare? |
|---|---|---|
| depth buffer of a rendered texture (`depth_id != 0`) | DEPTH24_X8, value 24 (`texture.rs:317-325`; `v3d_packet.xml:357`) | yes: Mesa's row for a depth format (`v3dx_format_table.c:149-150`) |
| R32F texels (CPU-written, or resolved) | R32F (`texture.rs:167-178`) | not established: R32F is a colour format; Mesa never compares on it |

Mesa compares only on its depth types: DEPTH_COMP16 (21), DEPTH_COMP24 (22),
DEPTH_COMP32F (23) and DEPTH24_X8 (24) (`v3d_packet.xml:354-357`;
`v3dx_format_table.c:149-156`). Nothing in the sources says the TMU compares on
an R32F type, so the design does not rely on it: for a binding whose sampler is
a shadow sampler (`compare_op & 16 == 0`, or any depth binding), `tmu_format`
returns DEPTH_COMP32F for R32F depth texels. DEPTH_COMP32F is 32 bits of
float per texel, the same bytes as R32F, and Mesa's own row for `Z32_FLOAT` is
return size 32 with the swizzle XXXX (`v3dx_format_table.c:152`), the TMU
layout of R32F (`v3dx_format_table.c:91`). Changes:

- `driver/src/v3d/v3d_tex.h` and `src/v3d/ffi.rs:454` gain
  `V3D_TEXTURE_DATA_FORMAT_DEPTH_COMP32F = 23`, with its row in the C format
  table that `tools/v3d/tex_check.c` compares with Mesa's.
- `texture::tmu_format` (`texture.rs:164-178`) returns it for an R32F depth
  binding that a shadow look-up reads. The GL core's only flags for a depth
  texture are `compare_op` and `depth_luminance` (`src/backend.rs:863-880`);
  the item decides whether to give `GlesTextureBind` an explicit "is depth"
  byte (a table struct: grep `tests/` for its literals first) or to key the
  type by the draw's compiled `ret32`/shadow bits. Either way the type must
  agree with the shader: a DREF write on an R32F record is the case to avoid.
- The swizzle stays the GL core's `(d, 0, 0, 1)` / luminance, as
  `depth_buffer_state` already keeps it for DEPTH24_X8 (`texture.rs:317-325`).
  The compiler reads lane 0 only.

D16 and D24 need no type of their own: the GL core stores them as R32F (or in
the D24S8 buffer), which is why `compare_op` bit 3 exists. That bit asks for
the reference to be clamped to [0, 1]: ES 3.0.6 3.8.15.1 clamps Dref and Dt
for a fixed-point depth. The texel is already in range (unorm, or R32F texels
the core wrote from a 16/24-bit value). For the reference, Mesa writes the
comparator to DREF as it is (`v3d_tex.c:155-158` (tree)); whether the TMU
clamps it is not stated in the sources. Decide by test (e): if the hardware does
not clamp, the compiler clamps with an `fmax(0)`/`fmin(1)` pair before the DREF
write for a sampler keyed `fixed_depth`, a new bit mask beside `ret32` in
`LodKey` (`src/shaderc/online/backend.rs:2007`, `:2035-2047`). The cost is two
ALU instructions for the fixed-point shadow look-up. A D32F texture
(`GL_DEPTH_COMPONENT32F`, `GL_DEPTH32F_STENCIL8`) is never clamped.

Not comparable: stencil-only reads (`GL_STENCIL_INDEX8` as a texture is not an
ES 3.0 texture), and a depth texture that is also a render target of the
draw (a feedback loop, undefined in GL, refused elsewhere). A shadow sampler
of a colour texture is undefined in GLSL ES 3.00; the compare field is
whatever `compare_op` says and the format has no depth type, so it reads
garbage by contract; the item should not add a check for it.

## (e) Test plan

Host only; the board part is one `gl_test` case (below). Run line for the first
two files: `cargo test --no-default-features --features
host-test,gpu-v3d,shaderc-online,no-shader-table --test <name>`.

`tests/es3_shaderc_v3d.rs` (compile and link through `SHC_ONLINE`, the file's
`build`/`run_vs` helpers, `:24-56`, `:147`). One case per form, each with the
`#version 300 es` source of the dEQP case and checks on the compiled program:

1. `shadow_2d_builds`: `sampler2DShadow`, `texture(s, vec3(uv, ref))`;
   expect `build` to succeed (today `NotYet("a shadow look-up")`), a TMUDREF
   write between the T write and the TMUS retiring write, `wrtmuc` count 2
   (P0, P1; no P2), one `ldtmu`, and a `ret32` P1 (`data & 1`).
2. `shadow_2d_proj_builds`: `textureProj(s, vec4(uv, ref, q))`; expect the
   reference multiplied by `1/q` before DREF (the multiply count rises by one
   over test 1).
3. `shadow_2d_lod_and_bias`: `textureLod` (TMUB, TMUSLOD, DREF) and, in the
   fragment stage, `texture(s, c, bias)` (TMUB, TMUS, DREF).
4. `shadow_cube_builds`: `samplerCubeShadow`, `texture(s, vec4(dir, ref))`;
   expect T, R, DREF, TMUSCM and the direction divide, with the reference
   not divided (compare the instruction count with the non-shadow cube).
5. `shadow_2d_array_builds`: `sampler2DArrayShadow`, `texture(s, vec4(uv,
   layer, ref))`; expect T, I, DREF, TMUS. Depends on `v3d-qpu-array-3d`; if
   that is not in yet, mark the test `#[ignore]` with that item's id and keep
   it as the pin.
6. `shadow_vertex_builds`: a vertex shader's `textureLod` on a shadow sampler.
7. `shadow_fifo_threads`: a shader with four shadow look-ups in one stretch
   keeps its thread count (`tmu_writes > 16 / threads` lowers it, `v3d_tex.c:
   264-270` (tree)): the counter in `select.rs` must include DREF.
8. Refusals that remain are named: an `isampler` with `shadow` does not exist
   in GLSL (no case); a shadow look-up with an offset or `textureGrad` keeps
   the refusal of those forms and names them, not "a shadow look-up".

`tests/v3d_sim_mesa.rs` (the interpreter judged on Mesa's code) and the
interpreter's own `sim.rs` tests: this file runs only fixture programs from
Mesa (`tests/fixtures/v3d_mesa_programs.txt`, written by
`tools/v3d/sim_fixture.py`), so the shadow work enters in two steps:

- add one or two shadow pairs to the program corpus
  (`tools/v3d/qpu_corpus.py`, `tools/v3d/sim_fixture.py`) so that Mesa's
  `v3d_shc` compiles them, regenerate the fixture and the pinned counts in the
  same commit (the population pin of the file's header, `:24-28`), and run
  Mesa's code on the interpreter against the evaluator once TMUDREF is
  modelled. That is the "Mesa's code agrees with the model" gate, the same
  one the 2D look-ups passed. If `v3d_shc`'s output for the pair disagrees
  with the model, the model is wrong, not Mesa.
- the `sim.rs` unit tests (`:2146` is the pattern: `Env { textures, samplers,
  .. }`) get eight cases, one per compare function, over a 2x2 depth texture
  with texels 0.25, 0.5, 0.5, 0.75 and a reference of 0.5: NEAREST gives
  (NEVER 0, LESS 0/1, ...) per texel, which pins the direction (reference OP
  texel) once; and a LINEAR case where the compare happens before the filter
  (the answer is a fraction, not 0/1); plus a reference above 1 and below 0
  on a fixed-depth texture for the clamp decision of (d).

Records to compare with Mesa. Neither `tools/v3d/drm_shim_ref.sh run` nor
`tools/v3d/gl_ref.py` draws anything but `clear` and `triangle` on an ES 2.0
context, so:

- Sampler words: `tools/v3d/tex_check.c` already packs our sampler and Mesa's
  `V3D42_SAMPLER_STATE` side by side and compares the bytes (`sampler_case`,
  `:337-348`; the cases at `:376-410`, `:706-740`, `:830`, `:894-910`). Add
  eight cases, `compare` 0 to 7 with Mesa's `depth_compare_function` set to
  the same value, and one with a non-zero compare beside a non-zero LOD bias
  and clamp-to-edge wraps (nothing else moves). Run with
  `tools/v3d/tex_check.sh` (MESA_V3D_TREE unset).
- Texture types: a `state_case` row for 16x16 DEPTH_COMP32F and 16x16
  DEPTH24_X8, `part_records` (`tex_check.c:353-378`), so the type word (23, 24)
  and the 32-bit return are Mesa's.
- Programs: `tools/v3d/v3d_shc` (built in the reference tree by
  `tools/v3d/README.md`) compiles `sampler2DShadow`, `sampler2DArrayShadow` and
  `samplerCubeShadow` pairs; their QPU words are the reference for the
  instruction counts (`tools/v3d/qpu_counts_42.tsv`, another item,
  `qpu-ref-counts`, owns that file; this item adds its programs after it
  lands) and their uniform lists give the TMUDREF position. A drm-shim job
  with a shadow draw would need `gl_ref.py` to build a GLES 3.0 context and a
  depth texture with `GL_TEXTURE_COMPARE_MODE`: out of scope here, the sampler
  record bytes above are the only draw-time word that moves.
- Board: one `gl_test` case (the next free number after 205): a 4x4 depth
  texture (a D24S8 render and an R32F upload), two samplers (LESS and GEQUAL,
  one NEAREST and one LINEAR), the 1-channel readback as a colour; its host
  prediction must equal the board's. This is the only place the direction of
  the comparison and the R32F-versus-DEPTH_COMP32F question get a hardware
  answer; the host cannot.

dEQP: `dEQP-GLES3.functional.texture.shadow.*` (`docs/es3-cts-failures.md:104`,
`:146-154`) is the target; after the work, the V3D host prediction and the
Pi 4 image `T*` rows (`docs/es3/pi4-gles3-fails.tsv`) change; this item does
not edit those files.

## What the item must not forget

- Struct, C struct, asserts and the zero literal move together ((b)); a
  stale asserted size fails the build, a stale literal fails `cargo clippy`.
- The sampler record is the only compare word; do not put the function in
  P1/P2 or in the program.
- A compare key that differs makes a different record, so `sampler_for`
  needs no change besides the type ((b)).
- A table struct field was added at the cost of two green PRs and a red
  main (`CLAUDE.md`, "Two green pull requests"): after adding `compare`,
  `grep -rn 'V3dSampler {' src tests` again against the current main.
- Regenerate whatever a new `v3d_tex.h` row moves in the same commit
  (`tests/reference/*.rs`, `tests/fixtures/`).
