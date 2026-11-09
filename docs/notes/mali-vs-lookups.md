<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall): the vertex look-ups of cases 101 and 149

**Status: case 101 and the compiler half of case 149 implemented
(vs-lookups, section 6); 149's frame half open** (plan #336, issue #33). The
sections below are the design as written at `60184fe5`. It maps the two vertex look-up refusals left in the sweep
(`tools/cts/log/2026-11-07-t6-step4a`) to the Mesa 26.2.1 / drm-shim
references, says where in `src/shaderc/online/mali` and `src/mali` each part is
made, and what is missing. It continues `notes/mali-vs-texture-lanes.md`
(case 149, step A landed in `606ce713`), `notes/mali-texture-lookups.md` and
`notes/mali-cube-textures.md` (the cube in the vertex stage, case 114); it
repeats none of their listings.

Locations are at `60184fe5` (`nir.rs` etc. are in `src/shaderc/online/mali/`);
section 5 records how they and the references were checked there.

## 1. What each case refuses on

| Case | Sweep text | Source of the refusal |
|---|---|---|
| 101 cubebias | `a vertex shader look-up at a coordinate with a constant lane` (`MaliError::TexVertexConstant`) | **not** `cubebias.vert/.frag` (the bias program, which has no vertex look-up and is byte-equal in `VARYCONST_POPULATION` "101", `src/shaderc/online/mali/tests.rs:4896`). gl_test case 101 links a second program, `gl_test_cubelod_{vs,fs}_src` = `tools/shaders/es100/cubelod.{vert,frag}` (`driver/tools/pvrlab/gl_test.c:22046`): `v_col = textureCubeLod(u_s, vec3(1.0, 0.0, 0.0), u_lod)`. Its direction is three constants: `src/shaderc/online/mali/nir.rs:2642-2644` |
| 149 vsunits | `a vertex shader texture look-up other than one 2D look-up at a level (a second one, a size query)` (`MaliError::TexVertex`) | sixteen `texture2DLod(u_sK, vec2(0.5), 0.0)`; the second look-up falls to the `IrOp::Tex { .. }` fallback `src/shaderc/online/mali/nir.rs:2668` because the first arm is guarded `if vs.sampler.is_none()` (`:2631`) |

## 2. Case 101: the constant-direction cube look-up

### 2.1 The references

* present: `varyconst_101_cubebias.{vert,frag,expect.json}` (the *bias* pair;
  its vertex shader is `v_dir = vec3(1.0, -a_pos.y, -a_pos.x)`, no look-up;
  its fragment has the cube look-up with a bias). Already byte-equal.
* present, the nearest vertex cubes: `case114_v.*` (direction
  `vec3(1.0, a_pos.y*0.95, a_pos.x*0.95)`: **one** constant lane, an explicit
  level `u_lod`), `cube_vs.*` (113: three uniform lanes). Their
  `TEX_SINGLE.slot1.reserved.32.cube.explicit @r0:r1:r2:r3, @r4:r5:r6, [r0:r1]`
  and the `CUBEFACE1/2_V9`, `CUBE_SSEL`, `CUBE_TSEL`, `FRCP`, two `.clamp_0_1`
  `FMA`, `MUX.i32.bit`, 8.8 level sequence are `Nir::cube_desc` (`src/shaderc/online/mali/nir.rs:1814`)
  and `sfixed_8_8` (`:1831`) and are compared in the tests of
  `notes/mali-cube-textures.md`.
* **committed by vs-lookups: the reference for `es100/cubelod.vert` with
  `es100/cubelod.frag`** (a cube direction of three constants), as
  `tools/mali/testdata/vslookup_101_cubelod.{vert,frag,expect.json}` (the two
  sources byte copies of `tools/shaders/es100/cubelod.{vert,frag}`):

      cp tools/shaders/es100/cubelod.vert tools/mali/testdata/vslookup_101_cubelod.vert
      cp tools/shaders/es100/cubelod.frag tools/mali/testdata/vslookup_101_cubelod.frag
      tools/mali/drm_shim_ref.sh run <work> tools/mali/testdata/vslookup_101_cubelod.vert \
          tools/mali/testdata/vslookup_101_cubelod.frag target/vslk/vslookup_101_cubelod --uniform-values
      python3 -I tools/mali/pan_ref_diff.py extract target/vslk/vslookup_101_cubelod.trace --draw \
          > tools/mali/testdata/vslookup_101_cubelod.expect.json

  `<work>` is the `drm_shim_ref.sh build` directory of Mesa 26.2.1
  (section 5 says how it was made on a host without the tarball). The
  regions it must hold: `vs.position` 24 instructions, `vs.varying` 38,
  `fs.fragment` 7; work registers 32 each; preload `0x3800000000000000`,
  `0x0800000000000000`, `0x3000000000000000`; secondary offset 256.

### 2.2 What the reference shows (measured, section 5)

With `x = 1.0`, `y = z = 0.0` the NIR holds one `@cubeface_pan` intrinsic over
`load_const`s (`32x2 %2 = @cubeface_pan (%0 (0x3f800000), %1 (0x0), %1 (0x0))`
in the shim's print): `Alu::is_intrinsic` ops are never folded
(`src/shaderc/online/mali/nir.rs:1849`, `!op.is_intrinsic()`; the comment at
`:2635-2640`: "the intrinsics fold no constant"), so the sequence is
`case114_v`'s with every lane an operand. The three points this note left to
the reference:

1. **Every constant is an inline immediate; no `MOV` first.** `CUBEFACE1 r0,
   0x3F800000, 0x0, 0x0`, `CUBEFACE2_V9 r1, 0x3F800000, 0x0, 0x0`,
   `CUBE_SSEL r2, 0x0, 0x3F800000, r1`, `CUBE_TSEL r3, 0x0, 0x0, r1`.
2. **The FAU table**: the eleven position/varying words (sysvals, `0xC7000000`,
   `0x47000000`, the store offsets 4, 8, 12), then `u_lod` (uniform dword 0),
   `0x1FFFFFFF` (the face mask of `MUX.i32.bit`), `0x3D800000` (1/16),
   `0x45800000` (4096), the handle `0x20001800`: 16 words. `1.0` and `0.5`
   do not join it (`0x3F800000` an immediate, `0x38000000.h1` from the table
   of constants).
3. **The varying is flat**: the vertex stores each texel word after the exact
   `FMA.f32 rK, rK^, 0x0, rK^` (four of them after the `TEX_SINGLE`), and
   the fragment is the 7-word `LD_VAR_BUF_IMM.f32.slot0.v4.src_flat32` copy.
   This is `src/shaderc/online/mali/nir.rs:2846-2872` (every stored lane that
   reads no attribute) and the convergent lane's flat promotion
   (`:2692`, `Nir::convergent` `:1702`; the store loop's comment `:2767-2775`),
   measured before in `unitype_116v`.

### 2.3 Where it is produced

* **Guard.** `src/shaderc/online/mali/nir.rs:2642-2644`: `if [x, y, z].iter().all(|v| matches!(v,
  Val::Const(_))) { return Err(MaliError::TexVertexConstant); }` is the only
  thing that stops the case. **Measured at `60184fe5`: with it deleted the
  linked pair is byte-equal to the reference above, every region, the facts
  and the FAU words** (`check_linked`, section 5). `cube_desc` pushes
  `Alu::Cubeface1`/`Cubeface2`/`CubeSsel`/`CubeTsel` nodes with `Val::Const`
  sources, the select (`src/shaderc/online/mali/select.rs:648`, `nir::Kind::VsTex { explicit, cube }`)
  and `emit.rs` put each in an instruction, and the constants become inline
  immediates by `fau.rs` as Mesa's do.
* **`select.rs`, `emit.rs`, `fau.rs`, `link.rs`: no change** (measured).
* **Frame side**: the draw binds one vertex cube texture (`vs_texture`,
  `src/mali/backend.rs:1674`, as case 114's, which passes on the board:
  `tools/cts/log/2026-11-07-t6-step4a/sweep-1.verdicts.txt`); the texture is the
  same RGBA8 cube map with levels as case 114's. Nothing new.

### 2.4 Tests

1. `a_cube_direction_of_constants_is_byte_equal_to_mesa_26_2_1`:
   `check_linked(td!("vslookup_101_cubelod.vert"), td!("vslookup_101_cubelod.frag"),
   td!("vslookup_101_cubelod.expect.json"))`, `v.info.sampler == Some(0)`, and
   the varying program's FAU tail `[Uniform { dword: 0 }, Constant(0x1fff_ffff),
   Constant(0x3d80_0000), Constant(0x4580_0000), Constant(0x2000_1800)]`; a
   byte-copy assertion that the two sources are `tools/shaders/es100/cubelod.*`
   (as `the_corpus_kept_branch_source_is_the_reference`).
2. `MaliError::TexVertexConstant` is deleted (no ES 1.00 source reaches it
   once the guard goes; a cube direction lane is never `Val::Undef`), in the
   same commit as the test: its variant and doc
   (`src/shaderc/online/mali/mod.rs:182-185`), `GAPS` (`:301`, 62 entries
   become 61; the entry `:322`), its text (`:421-423`), the reachable list in
   `unreachable_from_es100` (`src/shaderc/online/mali/tests.rs:1024`), the gap
   entry and its comment (`:1146-1149`), and the assertion in
   `a_cube_direction_from_an_attribute_is_divergent` (`:2322-2324`, its doc
   `:2306-2307`); the arm's comment `src/shaderc/online/mali/nir.rs:2635-2640`
   loses "A direction of constants alone is a shape no reference draws".
3. `tools/mali/corpus-verdicts.txt:26` (`es100/cubelod.vert`) becomes `ok`
   (`tools/mali/corpus-verdicts.sh --write`, same commit; the gate compares
   the bytes).
4. The sweep's case 101 is `cubebias` then `cubelod`: the bias program already
   runs (plan #309); the case passes once this one does.

## 3. Case 149: the second look-up

### 3.1 The references (present)

`tools/mali/testdata/case149_vsunits.{vert,frag,expect.json}` (the case's
program verbatim: `vsunits_vs_src(16)`, sixteen `texture2DLod(u_sK, c, 0.0)`
weighted by `u_w[K/4].xyzw` into `v_c`) and `case149_vstexconst{,_s}.*` (one
constant lane, step A). All three regenerate byte for byte at `60184fe5`
(section 5). `vs.varying` of `case149_vsunits` is 162 words with
`work_reg_count` 32, preload `0x0800000000000000`:

```
MOV.i32 r0, u13.w1 ; MOV.i32 r1, u13.w1                ; c = (0.5, 0.5): two moves of FAU word 27
MOV.i32 r2, u14.w0 ; MOV.i32 r3, 0x0                    ; handle 0 (FAU word 28), sampler word
TEX_SINGLE.slot0.reserved.32.2d.zero @r0:r1:r2:r3, @r0:r1, [r2^:r3^]
... the same four instructions at r4..r7 for look-up 1 (u14.w1): slot1, .wait1 ...
FMA.f32 r4, u6.w0, r4^, 0x0.neg  (x4)                   ; w1 * t1 (the first product, as a -0 FMA)
FMA.f32 r0, u5.w1, r0^, r4^      (x4)                   ; w0 * t0 + (w1 * t1)
... TEX slot2/slot0/slot1/.. ; FMA.f32 rK, u?.w?, r4^, rK^ per look-up and lane
FMA.f32 r0..r3, rK^, 0x0, rK^                           ; the exact ffma(x, 0.0, x): v_c is a flat varying
```

FAU: words 11..26 the sixteen weights, 27 the `0.5`, 28..43 the handles
`0x20001800 + K * 0x10001`. The `fs.fragment` is the usual
`LD_VAR_BUF_IMM.f32.v4.src_flat32` copy.

Two smaller shapes were drawn for this note (section 5; not committed) and
settle the two rules the design below needs:

* **rank**: a vertex shader declaring `u_s0`, `u_s1`, `u_s2` and reading
  `texture2DLod(u_s2, c, 0.0)` then `texture2DLod(u_s0, c, 0.0)` gets
  `textures_used 0x3`; the first look-up's handle is `0x20011801` (index 1),
  the second's `0x20001800` (index 0), FAU words 14, 15 in first-use order.
  The index is the sampler's rank by binding among the samplers the program
  reads, the fragment's rule (`src/shaderc/online/mali/nir_fs.rs:5350-5363`).
* **same sampler**: `u_w.x * texture2DLod(u_s0, c, 0.0) + u_w.y *
  texture2DLod(u_s0, c, 0.0)` is **one** `TEX_SINGLE` (`nir_opt_cse`), then
  `FADD.f32 r4, u5.w1, r4^` and four `FMA.f32 rK, rK^, r4, 0x0.neg`
  (`a*t + b*t` factored to `(a + b)*t`).

### 3.2 Which parts already exist

* the handle: `nir_fs::tex_handle(index)` (`src/shaderc/online/mali/nir_fs.rs:241`) is
  `(4<<27) | (index<<16) | (3<<11) | index` = `0x20001800 + index*0x10001`, the
  reference's table. Only the vertex stage's `select.rs` still uses the
  constant `TEX_HANDLE` (index 0, `src/shaderc/online/mali/select.rs:680`).
* the weights: `u_w[4]` read at constant indices is a 64-byte uniform array
  (`uniform_of`, `src/shaderc/online/mali/nir.rs:951`: `array_len > 1` of a `vec4` array, `:961`), each
  element a `Kind::Uniform` that is a FAU word (`uniarray`); the products and
  sums are `Fmul`/`Fadd`, and `fuse` (`src/shaderc/online/mali/nir.rs:3060`)
  gives the listing's order. **Measured at `60184fe5`** (section 5): with the
  arm's guard dropped and each look-up given its own handle the program is
  byte-equal, so the `fuse` order needs nothing.
* the exact `ffma(x, 0.0, x)`: `src/shaderc/online/mali/nir.rs:2846-2872` (the lanes read no
  attribute); the flat promotion `src/shaderc/online/mali/nir.rs:2692`.
* the look-up arm itself (`src/shaderc/online/mali/nir.rs:2622-2667`), incl.
  two constant lanes (`unitype_100`) and one (`case149_vstexconst*`, step A).
* the `TEX_SINGLE` `slot0/1/2` rotation and the `.wait` marks are the
  scheduler's; measured equal for sixteen look-ups.
* the store offsets: with all sixteen handles one value (`TEX_HANDLE`) the
  varying program keeps `IADD_IMM.i32 r0, 0x0, #0x4` (and `#0x8`, `#0xC`) where
  Mesa reads `u4.w0`, `u4.w1`, `u5.w0`: `fau.rs`'s promotion threshold
  (`lower_constants`, `src/shaderc/online/mali/fau.rs:674-704`, the count of
  the `max_amount`-th histogram entry in slot order) moves with the histogram.
  With sixteen distinct handles the histogram is Mesa's and the offsets are FAU
  words again (measured): nothing to do in `fau.rs`, but a test that pins only
  the handles would miss it, so compare the whole program.

### 3.3 What has to change, in order

Each step names its file, and whether it depends on vs-abs-if
(`notes/mali-vs-abs-if.md`, running) or vs-cmp-clamp
(`notes/mali-vs-compare-clamp.md`, merged in `aa854bd2`). **Neither changes
what these steps do**: vs-cmp-clamp only added `IrOp::Alu` arms (already in the
tree, the reason this note's look-up arm moved by +144 lines from `17e91019`), and
vs-abs-if's arms take no `IrOp::Tex` (its `IrNode::If` refuses an arm with a
look-up). The dependency is **textual**: vs-abs-if's step 1 moves `build`'s
per-instruction loop (`src/shaderc/online/mali/nir.rs:2335-2675` today, in `build`, `:2315`) into a
`fn walk_nodes(&mut VsWalk, ..)` with `vs` a field of `VsWalk`, so the look-up
arm edited below sits in that function after it lands, and every `nir.rs` line
here past `:2335` moves. Rebase on it and find the arm by its pattern
(`dim: dim @ (TexDim::D2 | TexDim::Cube)`).

**Order: case 101 first (section 2.3, one deleted `if`, independent of 149).
Then 149's compiler half and frame half in one item (step 4 says why).**

1. **`nir.rs`: the look-up carries its sampler binding** (no dependency).
   * `Kind::VsTex { explicit, cube }` (`src/shaderc/online/mali/nir.rs:426-429`)
     gains `sampler: u8` (the GL binding, `IrOp::Tex { sampler, .. }`); the two
     pushes (`:2656`, `:2659`) set it. Every other match is `VsTex { .. }` or
     `VsTex { explicit: .., .. }` (`:1284`, `:1679`, `:2148`, `:2150`, `:2891`)
     and needs no edit.
   * the arm guard `if vs.sampler.is_none()` (`:2631`) and its assignment
     `vs.sampler = Some(sampler)` (`:2632`) go.
   * **CSE of a look-up**: `Nir::push`/`push_node` (`:1743-1763`) append
     without CSE (only `Nir::alu`, `:1802`, and `Nir::load`, `:1767`, CSE), so
     two look-ups at one sampler and one coordinate stay two `TEX_SINGLE`
     where Mesa has one (section 3.1, "same sampler"), and the factoring
     `a*t + b*t` -> `(a + b)*t` that follows in Mesa is not in `fuse`. Either
     make the arm CSE an equal `VsTex` node (same kind, sampler and sources)
     **and** add the factoring with that reference committed, or keep that
     shape refused: a second look-up whose `(sampler, coordinate, level)`
     equals an earlier one returns `MaliError::TexVertex`. The second is the
     smaller step and case 149 does not need the first.
   * after `dce` (the clear at `:2889-2893`, "a look-up no output reads binds
     nothing") compute each live `VsTex`'s descriptor index, its sampler's
     rank by binding among the live look-ups' samplers, the fragment's code at
     `src/shaderc/online/mali/nir_fs.rs:5350-5363`, and keep it in the node
     (an `index: u8` beside `sampler`, set there) for `select.rs`.
     `Nir::sampler: Option<u8>` (`:706`, init `:1085`) becomes the bounded list
     of `(index, sampler)` pairs, as `Fs::bound_samplers`
     (`src/shaderc/online/mali/nir_fs.rs:2602-2606`).
   * the fallback arm (`:2668`) keeps `IrOp::TexSize` and every other shape.
     After this step no ES 1.00 source reaches `TexVertex` (measured with the
     guard dropped: two samplers, `texture2DProjLod` and an implicit-level
     `texture2D` in a vertex shader all compile; seventeen samplers are the
     front end's `Rejected` "the samplers need more than the 16 texture bindings
     a stage has"), unless the same-sampler shape above is kept refused: then
     that shape is the gap table's source for it. Otherwise `TexVertex` moves
     to the `Some(..)` side of `unreachable_from_es100`
     (`src/shaderc/online/mali/tests.rs:1023`) with that reason.
2. **`select.rs`: the handle** (a `select.rs` row; no dependency on
   vs-abs-if or vs-cmp-clamp; textual overlap with isa-transc, in review, which
   edits `select.rs` elsewhere): in the `nir::Kind::VsTex` arm
   (`src/shaderc/online/mali/select.rs:648`) the handle
   `Src::of(Idx::Const(super::nir_fs::TEX_HANDLE))` (`:680`) becomes
   `Src::of(Idx::Const(super::nir_fs::tex_handle(u32::from(index))))`. The
   FAU order (constants in first-use order, a handle where its look-up is
   first met) is `fau.rs`'s as it is: measured byte-equal for case 149 with
   this change. No `emit.rs` row: `TEX_SINGLE` and its handle operand exist.
3. **`mod.rs`: the stage record** (no dependency). `src/shaderc/online/mali/mod.rs:1246`
   (`sc.info.sampler = vs.sampler;`) fills `sc.samplers`/`sc.nsamplers`
   (`:930`) from the vertex list as `:1395-1402` does for the fragment, and
   sets `sc.info.sampler` to the first; `MaliShader::samplers` (`:648`, "a
   fragment shader's ... empty for a vertex shader") and `sampler` (`:642`)
   change their docs to both stages. The record copy (`:1508`) is shared.
4. **`src/mali`: the frame binds one vertex texture today** (no dependency on
   the two items; `src/mali/**` is this plan's).
   * `src/mali/backend.rs:1689` `sampler_bindings(d, vertex)` returns, for the
     vertex stage, `r.sampler.as_slice()` (`:1694-1695`, `if vertex ||
     r.samplers.is_empty()`): **one** binding; `sampled_texture(d, true, ..)`
     (`:2000`) takes `.first()` and `DrawCall::vs_texture` is an
     `Option<SampledTexture>` (`src/mali/frame.rs:1564`, filled at
     `src/mali/backend.rs:1674`). The fragment stage is already an array:
     `fs_textures` (`:1982`) builds up to `frame::MAX_STAGE_TEXTURES` (16,
     `src/mali/frame.rs:1254`) textures, one per rank, and
     `DrawCall::textures: &[SampledTexture]`.
   * `frame.rs` builds the vertex tables for one: `texture_descriptors(desc,
     &[tex])` (`:3219-3220`), `emit_stage_uniforms(.., d.vs_texture.as_slice())`
     (`:3223-3224`; it already takes a slice: it makes `max(len, 1)` sampler
     descriptors, `:2843`), and the resource entries `resource(.., TABLE_SAMPLER,
     vs_sampler.gpu, 1)` (`:3252`) and `TABLE_TEXTURE ... 1` (`:3254`) carry a
     hard-coded count of 1; the fragment's are `fs_count = textures.len().max(1)`
     (`:3237`). The validity check `vs.samples_texture != d.vs_texture.is_some()`
     (`:2394`, `ShaderMeta::samples_texture` from `record.sampler.is_some()`,
     `src/mali/program.rs:138`), the bind loop `:2398`, and
     `Layout::{vs_sampler, vs_texture, vs_plane}` (`:1602`, `:1608-1609`, one
     block each; the fragment's are one block holding all) follow.
   * So the frame work is the fragment's, mirrored: `vs_texture` becomes a
     slice (`&'a [SampledTexture]`), `sampler_bindings` drops `vertex ||`,
     `sampled_texture` becomes `vs_textures` after `fs_textures`,
     `texture_descriptors(desc, d.vs_textures)`, both counts `len().max(1)`,
     the check `vs.samples_texture == !d.vs_textures.is_empty()`. The
     `texture_descriptors` function and the `Texture`/`Sampler` array packing
     (`:2936-2958`, `:2843-2849`) are shared and need no change. Every literal
     of `DrawCall { .. }` changes with it: `grep -rl 'DrawCall {' src tests`
     (nineteen `src/mali/frame_*_tests.rs` files carry `vs_texture: None`;
     `CLAUDE.md`'s struct-literal warning applies).
   * **Why with steps 1-3, not after them**: with the compiler half alone the
     draw is not refused. The record names sixteen bindings, the frame binds
     the first, and look-ups 1..15 carry handles past the vertex tables' count
     of 1: a descriptor read out of the table on the GPU (a fault or garbage,
     the hang-guard's territory), not a `REFUSED`. If the frame half has to
     land later, the compiler half must come with a frame-side refusal
     (`FrameError::NotBuilt`, as `texture_of` uses, when the vertex record
     names more than one binding) so that the sweep's 149 moves from
     `REFUSED compile` to `REFUSED draw`, never to a fault.
5. **The board reference of the descriptors is missing.** The drm-shim records
   programs, not command streams (`notes/mali-vs-texture-lanes.md` §4 step B
   item 3 lists what a board capture must pin and no such record exists under
   `tools/cts/log/`): the vertex resource-table entries for 16 bindings, the 16
   texture and sampler descriptors, their stride and order (descriptor K at
   index K is what the handles `0x20001800 + K*0x10001` imply), and whether
   Mesa emits all bound units or only the used ones (the shim's
   `textures_used 0x3` of the rank draw says the program reads two; the
   tables are the board's to show). The fragment mirror above
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
   (words 27..43), and `v.info.samplers == [0, 1, .., 15]`. Add
   `td!("case149_vsunits.vert")` to `vs_reference_sources()`
   (`src/shaderc/online/mali/tests.rs:2095`, the `case149_*` line `:2112`).
   This test fails at `60184fe5` with the guard alone dropped (the FAU words
   and the store offsets, section 3.2) and passes with steps 1-2.
2. `vertex_samplers_are_ranked_by_binding_is_byte_equal_to_mesa_26_2_1`: the
   rank draw of section 3.1 committed as
   `tools/mali/testdata/vslookup_149_rank.{vert,frag,expect.json}` (its vertex
   shader below, `case149_vsunits.frag` as its fragment, `--uniform-values`);
   handles `0x20011801` then `0x20001800`, `samplers == [0, 2]`.

       attribute vec4 a_pos;
       uniform sampler2D u_s0;
       uniform sampler2D u_s1;
       uniform sampler2D u_s2;
       uniform vec4 u_w;
       varying vec4 v_c;
       void main() {
           gl_Position = a_pos;
           vec2 c = vec2(0.5);
           v_c = u_w.x * texture2DLod(u_s2, c, 0.0) + u_w.y * texture2DLod(u_s0, c, 0.0);
       }

3. The gap table: the entry `src/shaderc/online/mali/tests.rs:1145` (two
   samplers -> `TexVertex`) goes; if the same-sampler shape stays refused
   (step 1) it is the new entry (`texture2DLod(s, c, 0.0)` twice ->
   `TexVertex`), else `TexVertex` moves to `unreachable_from_es100`.
4. `src/mali`: a `frame_tex_tests.rs` case with two vertex textures: two texture
   and two sampler descriptors, both table counts 2, the validity check, and
   `Layout` addresses (the existing vertex tests
   `a_vertex_shader_s_texture_is_the_board_s`, `src/mali/frame_tex_tests.rs:695`,
   and `a_cube_map_s_texture_is_the_board_s`, `:823`, show the shape). Byte
   comparison with the board is the board item's.
5. `tools/mali/corpus-verdicts.txt` has no line for `case149_vsunits` (a
   sweep case, not a corpus shader; `notes/mali-vs-texture-lanes.md` §4).

## 4. Order and what is blocked on what

101 needs its reference committed (section 2.1) and one deleted `if`; it is
independent of 149 and of vs-abs-if/vs-cmp-clamp except for the `nir.rs` line
numbers. 149's compiler half (`nir.rs`, `select.rs`, `mod.rs`) is checked
against references that exist; its frame half is mechanical against the
fragment stage, but its Mesa-side descriptor words need the board session.
Land 149's two halves together (or the compiler half with the frame-side
refusal of section 3.3 step 4), never the compiler half alone.

## 5. Checked at `60184fe5`

* **References.** Every `file:line` above was re-read on `60184fe5`; the
  `nir.rs` lines from the look-up arm on had moved by +144 from `17e91019`
  (vs-cmp-clamp's arms; `fuse` `:2916` is `:3060`), `backend.rs` by +5, and
  `GLES_MAX_VS_TEX_BINDS` is `src/backend.rs:686` (it was `:658`). The flat store conversion was cited
  as `:2712-2730` and is the block `:2846-2872`; the `DrawCall` literals are in
  nineteen files, not twelve. Corrected in place.
* **The shim.** Mesa 26.2.1 was built with `tools/mali/drm_shim_ref.sh build`
  into `$CONNECTED_DEPS_DIR/mesa-shim/work`. The tarball could not be
  downloaded on this host; it was made from the pan_shc tree already there
  (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`, extracted from the checked
  tarball by `build_pan_shc.sh`) less its build directory, with
  `tools/mali/mesa-pan_shc.patch` reversed, so its sha256 is not the one in
  `third_party/mesa-26.2.1/UPSTREAM`. The three committed references below
  regenerating byte for byte is the check that the build is Mesa's.
* **Reruns, as documented** (`drm_shim_ref.sh run <work> <vs> <fs> <out>`,
  then `pan_ref_diff.py extract <out>.trace --draw`, `cmp` with the committed
  file): `case149_vsunits`, `case149_vstexconst`, `case149_vstexconst_s`:
  **byte-identical**. The words of section 3.1's listing (handles `u14.w0`,
  `u14.w1`, `u15.w0`, .., the `0x0.neg` first product, the four exact `FMA`s)
  match the trace's disassembly. `es100/cubelod` (`--uniform-values`, `u_lod`
  = 32) drawn for the first time: section 2.2's words, 38-instruction
  `vs.varying`. The rank and same-sampler draws of section 3.1 were made the
  same way. No trace or expect is committed.
* **The next refusal, with the look-up refusals disabled** (a throwaway
  change: the `TexVertexConstant` check and the `if vs.sampler.is_none()`
  guard deleted, then the pairs through `compile_linked`'s steps,
  `vs_constants`, `compile_fs_for_vs`, `compile_vs_for_reads`):
  * **case 101: none.** Both stages compile and `check_linked` against the new
    reference passes (words, work registers, preload, FAU values). The frame
    path is case 114's (`PASS` in the last sweep).
  * **case 149: none in the compiler.** Both stages compile (`work_reg_count`
    32, 163 words), but every look-up reads handle 0 and the FAU table is 29
    words: `check_linked` fails at `vs.position: FAU words` (the 16 handles
    missing) and the varying program differs in 15 handle moves and the three
    store offsets (section 3.2). With a further throwaway handle per sampler
    (`Kind::VsTex` carrying the binding, `select.rs` building
    `tex_handle(binding)`; rank = binding in this program) `check_linked`
    passes, every region byte-equal. The next stop is therefore the frame,
    and it is not a refusal: one vertex binding is built where sixteen are
    read (section 3.3 step 4).

## 6. What vs-lookups landed

* **References** (Mesa 26.2.1 drm-shim, section 2.1's commands, both with
  `--uniform-values`): `tools/mali/testdata/vslookup_101_cubelod.*` and
  `vslookup_149_rank.*` (section 3.4 item 2's vertex shader, case 149's
  fragment). They were drawn on a shim built from the pan_shc tree as in
  section 5, which reproduced `case149_vsunits.expect.json` byte for byte,
  and drawn again on a shim built from the release tarball itself (its
  sha256 the one in `third_party/mesa-26.2.1/UPSTREAM`): both equal to the
  committed files but for the `"trace"` name.
* **Case 101**: the `TexVertexConstant` guard and the gap are gone (section
  2.4 items 1-3); `a_cube_direction_of_constants_is_byte_equal_to_mesa_26_2_1`.
* **Case 149, compiler half** (section 3.3 steps 1-3): `Kind::VsTex` carries
  `sampler` and `index`; after DCE `nir::build` sets each index to the
  sampler's rank by binding among the live look-ups' samplers;
  `Nir::bound_samplers` replaces `Nir::sampler`; `select.rs` takes
  `tex_handle(index)`; `mod.rs` fills `MaliShader::samplers` for a vertex
  shader as for a fragment shader. Tests
  `sixteen_vertex_look_ups_are_byte_equal_to_mesa_26_2_1` and
  `vertex_samplers_are_ranked_by_binding_is_byte_equal_to_mesa_26_2_1`.
* **Same sampler, same coordinate and level** (step 1's choice): kept
  refused, `MaliError::TexVertex`, which is the gap table's source for it
  (`w.x * texture2DLod(s, p.xy, 0.0) + w.y * texture2DLod(s, p.xy, 0.0)`).
  The same sampler at two different coordinates compiles (two
  `TEX_SINGLE` of one handle); no reference draws it.
* **Frame half: not done** (step 4's fallback). `src/mali/backend.rs`
  `sampled_texture` returns `NOT_BUILT_VERTEX_TEXTURES` ("a vertex shader that
  reads more than one texture") when the vertex record names more than one
  binding, so case 149 moves from `REFUSED compile` to `REFUSED draw`, not to
  a descriptor read past the vertex tables. `sampler_bindings` now reads a
  vertex record's `samplers` list too (one binding: as before). Step 4's
  mirror of the fragment tables (`DrawCall::vs_textures`, the counts, the
  literals) and step 5's board capture remain.
