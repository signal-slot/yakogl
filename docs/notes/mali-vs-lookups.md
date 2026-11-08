<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex look-ups of cases 101 and 149

**Status: design, implementation in vs-lookups** (plan #336, issue #33). This
note changes no code. It maps the two vertex look-up refusals left in the sweep
(`tools/cts/log/2026-11-07-t6-step4a`) to the Mesa 26.2.1 / drm-shim
references, says where in `src/shaderc/online/mali` and `src/mali` each part is
made, and what is missing. It continues `notes/mali-vs-texture-lanes.md`
(case 149, step A landed in `606ce713`), `notes/mali-texture-lookups.md` and
`notes/mali-cube-textures.md` (the cube in the vertex stage, case 114); it
repeats none of their listings.

Locations are at `17e91019` (`nir.rs` etc. are in `src/shaderc/online/mali/`).

## 1. What each case refuses on

| Case | Sweep text | Source of the refusal |
|---|---|---|
| 101 cubebias | `a vertex shader look-up at a coordinate with a constant lane` (`MaliError::TexVertexConstant`) | **not** `cubebias.vert/.frag` (the bias program, which has no vertex look-up and is byte-equal in `VARYCONST_POPULATION` "101"). gl_test case 101 links a second program, `gl_test_cubelod_{vs,fs}_src` = `tools/shaders/es100/cubelod.{vert,frag}` (`driver/tools/pvrlab/gl_test.c:22046`): `v_col = textureCubeLod(u_s, vec3(1.0, 0.0, 0.0), u_lod)`. Its direction is three constants: `nir.rs:2498` |
| 149 vsunits | `a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query)` (`MaliError::TexVertex`) | sixteen `texture2DLod(u_sK, vec2(0.5), 0.0)`; the second look-up falls to the `IrOp::Tex { .. }` fallback `nir.rs:2524` because the first arm is guarded `if vs.sampler.is_none()` (`:2487`) |

## 2. Case 101: the constant-direction cube look-up

### 2.1 The references, and the one that is missing

* present: `varyconst_101_cubebias.{vert,frag,expect.json}` (the *bias* pair;
  its vertex shader is `v_dir = vec3(1.0, -a_pos.y, -a_pos.x)`, no look-up;
  its fragment has the cube look-up with a bias). Already byte-equal.
* present, the nearest vertex cubes: `case114_v.*` (direction
  `vec3(1.0, a_pos.y*0.95, a_pos.x*0.95)`: **one** constant lane, an explicit
  level `u_lod`), `cube_vs.*` (113: three uniform lanes). Their
  `TEX_SINGLE.slot1.reserved.32.cube.explicit @r0:r1:r2:r3, @r4:r5:r6, [r0:r1]`
  and the `CUBEFACE1/2_V9`, `CUBE_SSEL`, `CUBE_TSEL`, `FRCP`, two `.clamp_0_1`
  `FMA`, `MUX.i32.bit`, 8.8 level sequence are `Nir::cube_desc` (`nir.rs:1814`)
  and `sfixed_8_8` (`:1831`) and are compared in the tests of
  `notes/mali-cube-textures.md`.
* **missing: a reference for `es100/cubelod.vert` with `es100/cubelod.frag`**
  (a cube direction of three constants). `notes/mali-cube-textures.md` says "no
  reference draws it" and the gap table keeps it refused
  (`tests.rs:1148`, `:2261`). Draw it, as `case114_v` was drawn:

      tools/mali/drm_shim_ref.sh run <work> tools/shaders/es100/cubelod.vert \
          tools/shaders/es100/cubelod.frag target/vslk/cubelod --uniform-values
      python3 -I tools/mali/pan_ref_diff.py extract target/vslk/cubelod.trace --draw \
          > tools/mali/testdata/vslk_101_cubelod.expect.json

  (with the two sources copied to `vslk_101_cubelod.{vert,frag}`; `<work>` is
  the `drm_shim_ref.sh build` directory of Mesa 26.2.1: none is built on the
  machine this note was written on, `$CONNECTED_DEPS_DIR` holds the Mesa 26.2.1
  source tree only). Until it exists nothing below is byte-checkable and the
  walk's guard must stay.

### 2.2 What to expect, and what to read from the reference

With `x = 1.0`, `y = z = 0.0` the NIR holds `cube_face_*` ALU intrinsics over
`load_const`s: `Alu::is_intrinsic` ops are never folded
(`nir.rs:1849`, `!op.is_intrinsic()`; the comment at `:2494`: "the intrinsics
fold no constant"), so the sequence is `case114_v`'s with every lane an operand:
`CUBEFACE1 r2, 0x3F800000, 0x0, 0x0` and so on. What the reference must show
(the three unknowns the implementer reads off it, none guessed here):

1. which of the three constants of each `CUBEFACE*`/`CUBE_*SEL` instruction is
   an inline immediate and which a `MOV` first (an instruction takes one
   FAU/immediate word and the constant pass `va_lower_constants` moves the rest;
   `case114_v` has one constant, `0x3F800000`, inline; zero is an inline
   immediate `0x0`);
2. the FAU table (the words after the sysvals: the level uniform, the 1/16
   and 4096 level constants, the `0.5`s, the handle, and whether `1.0` joins
   them);
3. the varying: `v_col` is a texel of a convergent look-up (a uniform level, a
   constant direction), so the lane is flat: `LD_VAR_BUF_IMM.v4.src_flat32` in
   the fragment, and the vertex stores each texel word after the exact
   `FMA.f32 r, r, 0x0, r` (`nir.rs:2712-2730` converts every lane that reads
   no attribute: the tex lanes included; the flat promotion of a convergent
   look-up lane is `nir.rs:2548`, `Nir::convergent` `:1702`, measured in
   `unitype_116v`). `case114_v`'s lanes are attribute-derived, so smooth with
   no conversion; this shader has none.

### 2.3 Where it is produced

* **Guard.** `nir.rs:2497-2499`: `if [x, y, z].iter().all(|v| matches!(v,
  Val::Const(_))) { return Err(MaliError::TexVertexConstant); }` is the only
  thing that stops the case; drop it once the reference holds. Nothing else in
  the arm depends on a lane being computed: `cube_desc` pushes
  `Alu::Cubeface1`/`Cubeface2`/`CubeSsel`/`CubeTsel` nodes with `Val::Const`
  sources, the select (`select.rs:648`, `nir::Kind::VsTex { cube }`) and
  `emit.rs` put each in an instruction, and the constants become FAU words or
  inline immediates by `fau.rs`.
* **The cube sources, if the reference differs.** If an instruction gets a
  second constant that cannot be an operand, the register moves are `fau.rs`'s
  (the same path `unitype_100`'s two-constant 2D coordinate takes: `MOV.i32 r2,
  u6.w0` / `MOV.i32 r3, u6.w0`); no new pass is expected, but a case where
  `bi_` does something else (a `CUBEFACE1` of three constants folded by `bi`'s
  `bi_opt_constant_fold`) would be a new rule and its own reference.
* **`select.rs`, `emit.rs`, `link.rs`**: no change expected. The fragment
  program is the plain 7-word `LD_VAR_BUF_IMM` flat copy
  (`es100/cubelod.frag` is `ok` today, `corpus-verdicts.txt`).
* **Frame side**: the draw binds one vertex cube texture (`vs_texture`,
  `backend.rs:1669`, as case 114's); nothing new for a single look-up.

### 2.4 Tests

1. `a_cube_direction_of_constants_is_byte_equal_to_mesa_26_2_1` with the new
   reference, `check_linked` of the pair; its `vs.varying` FAU tail.
2. The gap table entry `tests.rs:1148` (`MaliError::TexVertexConstant`) and
   `tests.rs:2261` turn into the passing test; `es100/cubelod.vert` in
   `tools/mali/corpus-verdicts.txt` becomes `ok`; `TexVertexConstant` and its
   `mod.rs:185`/`:321`/`:421` text are deleted or kept as the guard of a still
   refused shape (a cube direction with a `Val::Undef` lane has none), in the
   same commit as the test (`tests.rs:1023` lists the variants that need a
   reference).
3. The sweep's case 101 is `cubebias` then `cubelod`: the bias program already
   runs (plan #309); the case passes once this one does.

## 3. Case 149: the second look-up

### 3.1 The references (present)

`tools/mali/testdata/case149_vsunits.{vert,frag,expect.json}` (the case's
program verbatim: `vsunits_vs_src(16)`, sixteen `texture2DLod(u_sK, c, 0.0)`
weighted by `u_w[K/4].xyzw` into `v_c`) and `case149_vstexconst{,_s}.*` (one
constant lane, step A). `vs.varying` of `case149_vsunits` is 162 words with
`work_reg_count` 32, preload `0x0800000000000000`:

```
MOV.i32 r0, u13.w1 ; MOV.i32 r1, u13.w1                ; c = (0.5, 0.5): two moves of FAU word 27
MOV.i32 r2, u14.w0 ; MOV.i32 r3, 0x0                    ; handle 0 (FAU word 28), sampler word
TEX_SINGLE.slot0.reserved.32.2d.zero @r0:r1:r2:r3, @r0:r1, [r2^:r3^]
... the same four instructions at r4..r7 for look-up 1: slot1, .wait1 ...
FMA.f32 r4, u6.w0, r4^, 0x0.neg  (x4)                   ; w1 * t1 (the first product, as a -0 FMA)
FMA.f32 r0, u5.w1, r0^, r4^      (x4)                   ; w0 * t0 + (w1 * t1)
... TEX slot2/slot0/slot1/.. ; FMA.f32 rK, u?.w?, r4^, rK^ per look-up and lane
FMA.f32 r0..r3, rK^, 0x0, rK^                           ; the exact ffma(x, 0.0, x): v_c is a flat varying
```

FAU: words 11..26 the sixteen weights, 27 the `0.5`, 28..43 the handles
`0x20001800 + K * 0x10001`. The `fs.fragment` is the usual
`LD_VAR_BUF_IMM.f32.v4.src_flat32` copy.

### 3.2 Which parts already exist

* the handle: `nir_fs::tex_handle(index)` (`nir_fs.rs:241`) is
  `(4<<27) | (index<<16) | (3<<11) | index` = `0x20001800 + index*0x10001`, the
  reference's table. Only the vertex stage's `select.rs` still uses the
  constant `TEX_HANDLE` (index 0, `select.rs:680`).
* the weights: `u_w[4]` read at constant indices is a 64-byte uniform array
  (`uniform_of`, `nir.rs:951`: `array_len > 1` of a `vec4` array), each
  element a `Kind::Uniform` that is a FAU word (`uniarray`); the products and
  sums are `Fmul`/`Fadd`, and `fuse` (`nir.rs:2916`) reproduces the listing's
  order: `r = 0.0 + w0*t0` folds to the product (`Alu::Fadd` with a zero), then
  `fadd(prod0, prod1)` tries its first source first, giving `ffma(w0, t0,
  prod1)`, then each further `fadd(acc, prodK)` gives `ffma(wK, tK, acc)`.
  This was derived from `fuse`'s rule order, not run.
* the exact `ffma(x, 0.0, x)`: `nir.rs:2712-2730` (the lanes read no
  attribute); the flat promotion `nir.rs:2548`.
* the look-up arm itself, incl. two constant lanes (`unitype_100`) and one
  (`case149_vstexconst*`, step A).
* the `TEX_SINGLE` `slot0/1/2` rotation and the `.wait` marks are the
  scheduler's (the fragment `texmore` references have several look-ups
  already); nothing is chosen by the lowering.

### 3.3 What has to change, in order

1. **`nir.rs`: one sampler becomes a list.**
   * `Nir::sampler: Option<u8>` (`nir.rs:706`, init `:1085`) and its clear at
     the end of `build` (`:2747-2749`, "a look-up no output reads binds
     nothing") become a bounded list of sampler bindings, each at its *rank* in
     the program (the fragment's rule: `Fs::bound_samplers`, `nir_fs.rs:2602`,
     "the sampler's rank among the samplers the program reads"; for case 149
     rank = binding, no reference separates them).
   * the arm guard `if vs.sampler.is_none()` (`:2487`) goes: the look-up takes
     the rank of its own `sampler` (`IrOp::Tex { sampler, .. }`), adding it
     if new; a seventeenth distinct sampler is `MaliError::TexVertex` (the
     limit is `GLES_MAX_VS_TEX_BINDS` = 16, `src/backend.rs:658`). A second
     look-up at the *same* sampler shares the binding. The fallback arm
     `:2524` keeps `TexSize` and every other shape.
   * `Kind::VsTex { explicit, cube }` (`nir.rs:426`) carries the rank
     (`index: u8`), so the CSE of two look-ups at different samplers does not
     merge them, and `select.rs:648` builds `Src::of(Idx::Const(tex_handle(
     index)))` in place of `TEX_HANDLE`. FAU order: constants in first-use
     order, a handle where the look-up is first met (word 28.. in the reference:
     after the `0.5`).
   * `mod.rs:1246` (`sc.info.sampler = vs.sampler;`) fills the stage's list.
     `MaliShader::sampler: Option<u8>` (`mod.rs:642`) and `samplers: &'k [u8]`
     (`:648`, "a fragment shader's") become one list for both stages, with the
     fragment's readers (`backend.rs:1684 sampler_bindings`) unchanged in
     meaning.
2. **`src/mali`: does the frame side already bind a second vertex sampler? No.**
   Read in `src/mali/backend.rs` and `frame.rs`:
   * `backend.rs:1684 sampler_bindings(d, vertex)` returns, for the vertex
     stage, `r.sampler.as_slice()`: **one** binding; `sampled_texture(d, true,
     ..)` (`:1952`) takes `.first()` and `DrawCall::vs_texture` is an
     `Option<SampledTexture>` (`frame.rs:1564`, filled at `backend.rs:1669`).
     The fragment stage is already an array: `fs_textures` (`:1936`) builds up
     to `frame::MAX_STAGE_TEXTURES` (16, `frame.rs:1254`) textures, one per
     rank, and `DrawCall::textures: &[SampledTexture]`.
   * `frame.rs` builds the vertex tables for one: `texture_descriptors(desc,
     &[tex])` (`:3219`), `emit_stage_uniforms(.., d.vs_texture.as_slice())`
     (`:3223`; it already takes a slice: it makes `max(len, 1)` sampler
     descriptors, `:2843`), and the resource entries `resource(.., TABLE_SAMPLER,
     vs_sampler.gpu, 1)` (`:3252`) and `TABLE_TEXTURE ... 1` (`:3254`) carry a
     hard-coded count of 1; the fragment's are `fs_count = textures.len().max(1)`
     (`:3237`). The validity check `vs.samples_texture != d.vs_texture.is_some()`
     (`:2394`) and `Layout::{vs_texture, vs_plane, vs_sampler}` (`:1602-1608`,
     one block each; the fragment's are one block holding all) follow.
   * So the frame work is the fragment's, mirrored: `vs_texture` becomes a
     slice (`&'a [SampledTexture]`), `sampler_bindings` returns the vertex
     list, `sampled_texture` becomes `vs_textures` after `fs_textures`,
     `texture_descriptors(desc, d.vs_textures)`, both counts `len().max(1)`,
     the check `vs.samples_texture == !d.vs_textures.is_empty()`. The
     `texture_descriptors` function and the `Texture`/`Sampler` array packing
     (`:2936-2960`, `:2843-2848`) are shared and need no change. Every literal
     of `DrawCall { .. }` changes with it: `grep -rl 'DrawCall {' src tests`
     (twelve `src/mali/frame_*_tests.rs` files carry `vs_texture: None`;
     `CLAUDE.md`'s struct-literal warning applies).
3. **The board reference of the descriptors is missing.** The drm-shim records
   programs, not command streams (`notes/mali-vs-texture-lanes.md` §4 step B
   item 3 lists what a board capture must pin and no such record exists under
   `tools/cts/log/`): the vertex resource-table entries for 16 bindings, the 16
   texture and sampler descriptors, their stride and order (descriptor K at
   index K is what the handles `0x20001800 + K*0x10001` imply), and whether
   Mesa emits all bound units or only the used ones. The fragment mirror above
   follows `panfrost_emit_sampler_descriptors` /
   `panfrost_emit_texture_descriptors` (Mesa 26.2.1 `pan_cmdstream.c`, per
   stage; the fragment's tables are board-measured, the `texmore` pairs) and
   is the design; a vertex-side board dump (`tools/mali/mesa-board-ref.sh`
   through `tools/mali/panthor/run-on-board.sh`, a board session this plan's
   `board-sweep` item holds) confirms it. The design here does not need the
   capture to be *written*, but `frame-` tests pinned to Mesa's words cannot be
   byte-exact without it.

### 3.4 Tests

1. `sixteen_vertex_look_ups_are_byte_equal_to_mesa_26_2_1`: `check_linked`
   (both programs and the fragment) of `case149_vsunits`; the FAU tail
   `[Constant(0x3f00_0000)]` then the sixteen handles `tex_handle(0..16)`
   (words 27..43). Add `case149_vsunits.vert` to `vs_reference_sources()`
   (`tests.rs:2094`).
2. The gap table: `tests.rs:1144` (two samplers -> `TexVertex`) becomes seventeen
   distinct samplers -> `TexVertex`; `textureSize` in a vertex shader stays
   `TexVertex`.
3. A rank test: two look-ups at one sampler give one entry; two at different
   samplers, ranks 0 and 1 and handles `0x20001800`, `0x20011801`.
4. `src/mali`: a `frame_tex_tests.rs` case with two vertex textures: two texture
   and two sampler descriptors, both table counts 2, the validity check, and
   `Layout` addresses (the existing vertex test at `frame_tex_tests.rs:736`,
   `:872`, `:971` shows the shape). Byte comparison with the board is the
   board item's.
5. `tools/mali/corpus-verdicts.txt` has no line for `case149_vsunits` (a
   sweep case, not a corpus shader; `notes/mali-vs-texture-lanes.md` §4).

## 4. Order and what is blocked on what

101 needs one reference (section 2.1) and then one deleted `if`; it is
independent of 149. 149's compiler half (`nir.rs`, `select.rs`, `mod.rs`) is
checked against a reference that exists; its frame half is mechanical against
the fragment stage but its Mesa-side descriptor words need the board session.
Run 149's compiler half first (the sweep's refusal moves from `TexVertex` to
the next one), the frame half second.
