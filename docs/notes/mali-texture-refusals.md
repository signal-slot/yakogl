<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Mali (Valhall, v10): the texture refusals, Mesa's design per case

The texture refusals of issue #1385 (the NanoPC-T6's `gpu-mali` backend), one
section per item: `tex-vs` (vertex-shader look-ups, gl_test cases 113 and
197), `tex-coord-122` (the coordinate form of case 122) and `tex-depth-169`
(the depth texture of case 169, and the `depth_resolve` hook it needs), and
plan #153's `texcoord` (a computed coordinate, case 121 and femtovg's image
programs, section (d), which has its own citation rule). For
each: what Mesa 26.2.1's panfrost does on v10, where the change lands in our
tree, and which refusal it removes. It is a reading of Mesa's source and ours;
nothing here was run on the board. A fact only the board can settle is marked
**pin on the board**. The frame-side refusals (#1386) are
`docs/notes/mali-frame-refusals.md`; the general lowering (#1384) is
`driver/docs/nanopc-t6-mali.md`, "The general lowering (approved
2026-10-19)", and `docs/notes/mali-lowering-map.md`.

## Citations

- Mesa: `M:<path>:<line>`, `<path>` under `src/` of the Mesa 26.2.1 tree
  (`MESA_PAN_TREE`, `$CCFARM_DEPS_DIR/mesa-pan/mesa-26.2.1`, `VERSION`
  26.2.1). `third_party/mesa-26.2.1/` holds no gallium panfrost files, so it
  is not the tree cited.
- Ours: `<path>:<line>` from the repository root, at `main` 445c5653.
- PR #1389: `a0aff40b6:<path>:<line>` (the tip of `work/mali-row-align`).

Every citation is written with its whole path, never as a bare `:line`
continuing an earlier one, so each can be checked on its own. The compiler
read is Mesa's default Bifrost-BIR path: kraid runs only in a build with
`WITH_PANFROST_RUST` and a stage named in `PAN_USE_KRAID`, which defaults to
none (`M:panfrost/compiler/pan_compiler.c:49-50`,
`M:panfrost/compiler/pan_compiler.c:53-68`,
`M:panfrost/compiler/bifrost/bifrost_nir.c:1295-1298`). The RK3588's v10 GPU is
a CSF part (Panthor), so the job path is `pan_csf.c`.

## The four cases today

| case | gl_test | shaders | first refusal | corpus verdict |
|---|---|---|---|---|
| 113 `cubemap` | `driver/tools/pvrlab/gl_test.c:14118` | `tools/shaders/es100/cubemap.vert` + `tools/shaders/es100/cubelod.frag` | `Valhall: a texture look-up in a vertex shader is not lowered yet` (`MaliError::TexVertex`, `src/shaderc/online/mali/mod.rs:306`) | `tools/mali/corpus-verdicts.txt:26` (the FS is `ok`, `tools/mali/corpus-verdicts.txt:24`) |
| 197 `vtxgrid` | `driver/tools/pvrlab/gl_test.c:17013` | `tools/shaders/gltest/vtxgrid_vs.vert` + `tools/shaders/gltest/vtl_fs.frag` | VS: the same string (`tools/mali/corpus-verdicts.txt:270`); FS: `Valhall: a texture coordinate other than a vec2 varying read whole is not lowered yet` (`tools/mali/corpus-verdicts.txt:266`) | both refused |
| 122 `uniidxi` | `driver/tools/pvrlab/gl_test.c:13260` (`case_uniidx_tex(2)`) | passthrough VS + `tools/shaders/es100/uniidx_i.frag` | the coordinate string (`MaliError::TexCoord`, `src/shaderc/online/mali/mod.rs:311-313`) | `tools/mali/corpus-verdicts.txt:110` |
| 169 `dstex` | `driver/tools/pvrlab/gl_test.c:17606` | `tools/shaders/gl_test_es100/uv.vert` + `tools/shaders/gl_test_es100/texture.frag`, and `dsk` for step 2 (its VS `driver/tools/pvrlab/gl_test_shaders_es100.h:7733-7736`) | none from the compiler (`tools/mali/corpus-verdicts.txt:203`, `tools/mali/corpus-verdicts.txt:206`: `ok`); the backend refuses the texture: `NOT_BUILT_TEXTURE_DEPTH` (`src/mali/backend.rs:200`), then `NOT_BUILT_TEXTURE_FORMAT` for R32F (`src/mali/backend.rs:198-199`); and `depth_resolve: None` (`src/mali/backend.rs:2365`) | — |

Two facts the issue's grouping hides, both from the verdicts above:

- **197 is two items.** Its fragment shader samples at the constant
  `vec2(0.5, 0.5)` (`tools/shaders/gltest/vtl_fs.frag:6`), the very form of
  case 122. 197 needs `tex-vs` *and* `tex-coord-122`.
- **122 does not pass from its item.** Its next refusal is the run-time
  uniform index (section (b)).

## (a) Vertex-shader look-ups: cases 113 and 197

### What the cases draw

- **197** (`driver/tools/pvrlab/gl_test.c:16987-17013`, dEQP
  `texture.vertex.2d`'s draw): `v_color = texture2DLod(u_vtex, a_uv, u_lod)`
  (`tools/shaders/gltest/vtxgrid_vs.vert:9`), `u_lod` a uniform set to 0, 1, 2
  and 3 for the four quarters; a 16x16 `GL_RGB` chain of 5 levels,
  NEAREST_MIPMAP_NEAREST, CLAMP_TO_EDGE (`driver/tools/pvrlab/gl_test.c:17044-17064`);
  a 127x127-quad grid of client arrays and 96,774 client u16 indices,
  1,225,804 bytes of client data a draw
  (`driver/tools/pvrlab/gl_test.c:16992-16995`). The GL core stores `GL_RGB` as
  RGBA8 (`src/gl/texture.rs:5267`), which the Mali back end samples. The
  fragment shader's texture is on unit 1 and weighed out by `u_pick` 0
  (`driver/tools/pvrlab/gl_test.c:17037-17043`): the draw binds one texture a
  stage, on different units.
- **113** (`driver/tools/pvrlab/gl_test.c:14102-14116`): `v_col =
  textureCubeLod(u_s, u_dir, u_lod)` (`tools/shaders/es100/cubemap.vert:12`), a
  16x16 **cube** map of 5 levels, NEAREST_MIPMAP_NEAREST; 30 draws, one a
  (face, level). With the vertex refusal gone, the backend refuses the cube
  map: `NOT_BUILT_TEXTURE_TYPE` (`src/mali/backend.rs:201-202`, raised at
  `src/mali/backend.rs:1557`). 113 is therefore the second step: 197's 2D
  explicit-LOD look-up first, then the cube form and a cube descriptor.

### How Mesa binds a vertex-stage texture and sampler

Nothing is special for the vertex stage: the texture and sampler state is per
stage, and the vertex stage fills the same tables the fragment stage does.

1. **State per stage.** `panfrost_set_sampler_views` and
   `panfrost_bind_sampler_states` take a `mesa_shader_stage` and fill
   `ctx->sampler_views[shader]`, `ctx->sampler_view_count[shader]` and
   `ctx->samplers[shader]`, marking the stage dirty
   (`M:gallium/drivers/panfrost/pan_context.c:373-430`,
   `M:gallium/drivers/panfrost/pan_context.c:310-328`). The slots are the
   stage's own: our fragment rule, taken from the shim, is that Mesa numbers
   the samplers the program reads and binds the GL unit's texture at that
   index (`src/shaderc/online/mali/nir_fs.rs:50-54`). For 197 that predicts
   slot 0 in each stage although the GL units are 0 and 1; **pin with the
   shim** for the vertex stage.
2. **Descriptor arrays per stage.** `panfrost_update_shader_state(batch, st)`
   runs for the vertex and the fragment stage
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:3465-3466`) and, when the
   stage's textures, samplers or shader changed, emits `batch->textures[st]`
   and `batch->samplers[st]`
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:3171-3177`).
   - TEXTURE: `max(sampler_view_count, info.texture_count)` descriptors, each
     the view's pre-built one, a null texture for a hole or past the views
     (`M:gallium/drivers/panfrost/pan_cmdstream.c:1935-1987`). The shader's
     `texture_count` is `BITSET_LAST_BIT(textures_used)`, the count through
     the last slot used, not the number used
     (`M:panfrost/compiler/pan_compiler.c:305-306`).
   - SAMPLER: `sampler_count` descriptors, or with none bound one workaround
     sampler, "We always need at least 1 sampler for txf to work"
     (`M:gallium/drivers/panfrost/pan_cmdstream.c:2022-2047`).
3. **The resource table.** `panfrost_emit_resources(batch, stage)` builds one
   table per stage, 64-byte aligned, in the order of `enum
   panfrost_resource_table` (UBO, ATTRIBUTE, ATTRIBUTE_BUFFER, SAMPLER,
   TEXTURE, IMAGE, SSBO; `M:gallium/drivers/panfrost/pan_context.h:335-345`)
   (`M:gallium/drivers/panfrost/pan_cmdstream.h:260-308`). TEXTURE and SAMPLER
   are filled for every stage, the TEXTURE entry counting
   `sampler_view_count[stage]` and the SAMPLER entry `max(sampler_count, 1)`
   (`M:gallium/drivers/panfrost/pan_cmdstream.h:282-287`); the vertex stage
   adds ATTRIBUTE and ATTRIBUTE_BUFFER
   (`M:gallium/drivers/panfrost/pan_cmdstream.h:295-302`). An entry is
   `{address, size = count * pan_size(BUFFER)}`, nothing for a count of 0
   (`M:panfrost/lib/pan_encoder.h:267-279`); a `Buffer` is 8 words, 32 bytes
   (`M:panfrost/genxml/v10.xml:1056`).
4. **The job.** `csf_emit_shader_regs` puts a stage's resource table in
   register 0 (vertex) or 4 (fragment), its push uniforms in 8 / 12 and its
   shader in 16 / 20 (`M:gallium/drivers/panfrost/pan_csf.c:1215-1237`). The
   draw loads the vertex stage with the **position** shader, and the fragment
   stage if the fragment shader is needed
   (`M:gallium/drivers/panfrost/pan_csf.c:1454-1459`). The varying shader has
   no registers of its own: `cs_run_idvs(.., cs_shader_res_sel(0, 0, 1, 0),
   cs_shader_res_sel(2, 2, 2, 0), ..)` (`M:gallium/drivers/panfrost/pan_csf.c:1726-1727`;
   the arguments are srt, fau, spd, tsd, `M:panfrost/genxml/cs_builder.h:1497-1505`)
   selects SRT 0 and FAU 0, the vertex stage's, for the varying shader, and the
   builder asserts that selection (`M:panfrost/genxml/cs_builder.h:1604-1609`).
   **One vertex resource table, with its TEXTURE and SAMPLER entries, serves
   both IDVS shaders.**
5. **Descriptor contents.** The sampler view's TEXTURE descriptor is built
   once by `panfrost_create_sampler_view_bo`, whatever the stage
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:1720-1897`; its fields in
   section (c)). The SAMPLER is `panfrost_create_sampler_state`
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:199-260`).

### How TEX is compiled, and where it lands in the two IDVS shaders

1. **Implicit LOD in a vertex shader.** `pan_preprocess_nir` runs
   `nir_lower_tex` with `lower_invalid_implicit_lod`
   (`M:panfrost/compiler/pan_compiler.c:118-129`), which turns a look-up with
   implicit derivatives in a stage without them into an explicit LOD of 0
   (`M:compiler/nir/nir_lower_tex.c:1807-1818`). So a VS `texture2D` becomes a
   `txl` at 0; `texture2DLod` and `textureCubeLod` are already `txl`.
2. **Handles.** The gallium pass gives the texture index
   `pan_res_handle(PAN_TABLE_TEXTURE, i)` and the sampler
   `pan_res_handle(PAN_TABLE_SAMPLER, j)`, sampler 0 by ABI for a look-up that
   needs none (`M:gallium/drivers/panfrost/pan_nir_lower_res_indices.c:124-156`);
   a handle is `table << 24 | index` (`M:panfrost/compiler/pan_compiler.h:626-633`).
   The index is the stage's own slot.
3. **The Valhall form.** On arch >= 9 `pan_nir_lower_tex` runs `va_lower_tex`
   on every tex (`M:panfrost/compiler/pan_nir_lower_tex.c:1109-1122`,
   `M:panfrost/compiler/pan_nir_lower_tex.c:1077-1107`,
   `M:panfrost/compiler/pan_nir_lower_tex.c:802-973`):
   - the handle: constant table/index pairs are one immediate,
     `tex_table << 27 | tex_index << 16 | samp_table << 11 | samp_index`
     (`M:panfrost/compiler/pan_nir_lower_tex.c:723-745`), our
     `TEX_HANDLE` (`src/shaderc/online/mali/nir_fs.rs:59`);
   - the coordinate: each component a staging register S, T (, R); a cube
     direction first becomes `build_cube_desc`'s two words, the face in the
     top 3 bits of S (`M:panfrost/compiler/pan_nir_lower_tex.c:824-833`,
     `M:panfrost/compiler/pan_nir_lower_tex.c:117-128`);
   - the LOD (not `txf`): a constant-zero LOD is `ZERO_LOD`, no staging
     register; any other explicit LOD is `EXPLICIT` with an LOD staging
     register holding signed 8.8 fixed point, `u2u32(f2i16(fsat_signed(lod *
     1/16) * 4096))`; no LOD, bias or gradient is `COMPUTED_LOD`
     (`M:panfrost/compiler/pan_nir_lower_tex.c:927-947`,
     `M:panfrost/compiler/pan_nir_lower_tex.c:136-141`). 197's `u_lod` is a
     uniform, so its look-up is `EXPLICIT` with a computed staging word; a
     VS never reaches `COMPUTED_LOD` (step 1);
   - the flags (`struct pan_va_tex_flags`,
     `M:panfrost/compiler/pan_nir.h:236-250`) go in `backend_flags`, the
     staging registers in `backend1` / `backend2`
     (`M:panfrost/compiler/pan_nir_lower_tex.c:949-970`).
4. **BIR.** `bi_emit_tex_valhall` collects the staging sources into one
   `COLLECT` and emits `TEX_SINGLE` for tex/txb/txl/txd (`TEX_FETCH` for txf),
   then splits the result into the components read
   (`M:panfrost/compiler/bifrost/bifrost_compile.c:3488-3574`). The register
   format is the destination type's (`bi_reg_fmt_for_nir(tex->dest_type)`,
   `M:panfrost/compiler/bifrost/bifrost_compile.c:3521`). Whether that type
   is 16 or 32 bits comes from the GLSL front end's precision lowering, not
   from this pass, so the shim reference of each vertex form decides it.
   Our fragment look-up returns fp16 (`f16: true`,
   `src/shaderc/online/mali/nir_fs.rs:960`); 197's vertex sampler is highp,
   and 113's `samplerCube` takes the vertex default, lowp.
5. **The two IDVS shaders.** On v10, a vertex shader with IDVS is compiled
   twice, `BI_IDVS_POSITION` then `BI_IDVS_VARYING`; v12+ compiles one `ALL`
   variant instead (`M:panfrost/compiler/bifrost/bifrost_nir.c:1299-1306`).
   Each variant inlines `load_shader_output_pan` with the position bits
   (`POSITION | ATTRIB`) or the varying bit and then runs constant folding,
   DCE, dead control flow and CSE to a fixed point
   (`M:panfrost/compiler/bifrost/bifrost_compile.c:4374-4397`): an
   instruction only the other variant's outputs need is removed. A TEX
   therefore lands in
   - **the varying shader only** when only varyings depend on it: cases 197
     (`v_color`) and 113 (`v_col`), whose `gl_Position` is an attribute;
   - **the position shader only** when only `gl_Position` (or the attribute
     outputs) depend on it, as in displacement mapping;
   - **both**, each with its own `TEX_SINGLE`, when both do.
   The texture and sampler tables are the shared vertex ones in every case.
   On v10 the varying shader is enabled when its binary is not empty,
   `secondary_enable = binary->size > offset`
   (`M:panfrost/compiler/bifrost/bifrost_compile.c:4707-4711`); the
   `outputs_written & ~(POS | EX_FIFO)` rule is the v12+ `ALL` variant's
   (`M:panfrost/compiler/bifrost/bifrost_compile.c:4716-4722`).

### Landing site in our code

- **Compiler.** The vertex walk refuses every `Tex` / `TexSize`
  (`src/shaderc/online/mali/nir.rs:837`). The vertex variants are already
  selected per output with a dead-code pass, as Mesa's are:
  `select::vs_variant(.., 0)` for the position shader, `(.., 1)` for the
  varying one (`src/shaderc/online/mali/mod.rs:857`,
  `src/shaderc/online/mali/mod.rs:868`), each starting with
  `nir::dce(vs, Some(out))` (`src/shaderc/online/mali/select.rs:317-324`). A
  vertex `Kind::Tex` node in the shared graph (the fragment graph has one,
  `src/shaderc/online/mali/nir_fs.rs:395`) therefore lands in the right
  variant with no rule of its own. What is new against the fragment
  look-up (`tex`, `src/shaderc/online/mali/nir_fs.rs:953-966`):
  - the LOD mode: `Op::TexSingle` has `zero_lod` (ZERO_LOD) or nothing
    (COMPUTED_LOD) (`src/shaderc/online/mali/bir.rs:261-271`); `EXPLICIT`
    and its LOD staging word, built by Mesa's 8.8 conversion as NIR ALU,
    are missing;
  - the result width the shim shows (`f16: false` is the preload's 32-bit
    look-up, `src/shaderc/online/mali/preload.rs:62`), the coordinate as
    fp32 attribute lanes, no helper invocations;
  - for 113, the cube: `build_cube_desc`'s arithmetic, and the IR form. Our
    IR has `Es100Tex::Tex2DLod` (`src/shaderc/online/ir.rs:180-181`) and
    `Es100Tex::TexCube` (`src/shaderc/online/ir.rs:184-185`), but
    `es100_tex()` maps only `{Cube, Implicit}` to the cube
    (`src/shaderc/online/ir.rs:519`); `{Cube, Lod}`, `textureCubeLod`, falls
    to `None` (`src/shaderc/online/ir.rs:520`).
  The general lowering's steps S1-S6 do not list a vertex look-up
  (`driver/docs/nanopc-t6-mali.md:300-314`), so this is a step of its own
  after them, with one shim reference per form (2D `txl` at a uniform LOD,
  2D `txl` at 0, cube `txl`) as #1384 does for its rows.
- **Frame.** The vertex resource table has UBO, ATTRIBUTE, ATTRIBUTE_BUFFER
  and a one-entry SAMPLER, and **no TEXTURE entry**
  (`src/mali/frame.rs:2466-2470`); the fragment table has one when the draw
  samples (`src/mali/frame.rs:2471-2482`). The landing: a vertex texture
  built as `fs_texture` is (`src/mali/frame.rs:2440-2455`: a `GenericPlane`
  a level, then the `Texture`), a vertex sampler from it in
  `emit_stage_uniforms` (it takes `None` for the VS today,
  `src/mali/frame.rs:2433-2434`, `src/mali/frame.rs:2189-2195`), and
  `TABLE_TEXTURE` in `vs_res`.
- **Backend.** The draw's vertex binds are `GlesDraw::vs_textures` /
  `num_vs_textures` (`src/backend.rs:991-994`); the backend reads only the
  fragment ones (`sampled_bind`, `src/mali/backend.rs:1363-1374`), and the
  shader record holds one fragment sampler binding (`MaliShader::sampler`,
  `src/shaderc/online/mali/mod.rs:478-481`). The vertex record needs its
  own, and `DrawCall` a vertex `SampledTexture` beside `texture`
  (`src/mali/frame.rs:1246`).
- **Limits.** `VS_TEX_UNITS` stays 16 (`src/mali/backend.rs:99-108`, which
  says why it is not 0). Mesa gives every stage `PIPE_MAX_SAMPLERS`
  (`M:gallium/drivers/panfrost/pan_screen.c:593`), 32
  (`M:gallium/include/pipe/p_state.h:68`).
- **Refusal removed:** `Valhall: a texture look-up in a vertex shader is not
  lowered yet`, for the corpus rows `cubelod.vert`, `cubemap.vert`,
  `vtex_struct_sampler.vert`, `vtex_struct_sampler_array.vert`,
  `cubelodv.vert`, `vtl_vs.vert`, `vtl_vs_const.vert`, `vtl_vs_grid.vert` and
  `vtxgrid_vs.vert` (`tools/mali/corpus-verdicts.txt:25-26`,
  `tools/mali/corpus-verdicts.txt:119-120`, `tools/mali/corpus-verdicts.txt:129`,
  `tools/mali/corpus-verdicts.txt:267-270`), each moving to its next refusal
  or `ok` (the cube rows need the cube form). 197 then needs (b) for its
  fragment shader and meets the frame with 1.2 MB of client data a draw:
  **pin on the board** whether the Mali's frame arenas take it (the PowerVR
  needed the GL core to cut such a draw, `driver/tools/pvrlab/gl_test.c:16996-17000`).
  113 then needs the cube texture: Mesa's cube view is `dimension = CUBE`,
  `array_size = layers / 6` (`M:panfrost/lib/pan_texture.c:1198-1217`,
  `M:panfrost/lib/pan_texture.c:1299-1300`, `M:panfrost/lib/pan_texture.c:1316`).

## (b) Case 122, the coordinate form

### The shader and the form

`tools/shaders/es100/uniidx_i.frag` (case 122 `uniidxi`, a control for the
GC7000's texld-before-movar order, `driver/tools/pvrlab/gl_test.c:13244-13259`):

```glsl
precision mediump float;
uniform vec4 fcols[4];
uniform int fidx;
uniform sampler2D ftex;
void main() {
    vec4 t = texture2D(ftex, vec2(0.5));
    gl_FragColor = 0.5 * (fcols[fidx + int(t.w * 0.0)] + t);
}
```

The coordinate is `vec2(0.5)` (`tools/shaders/es100/uniidx_i.frag:8`): **a
constant**, both lanes 0.5, not a varying. The texture is 1x1 RGBA8 NEAREST
(`driver/tools/pvrlab/gl_test.c:13294-13296`). 197's fragment shader has the
same form, `texture2D(u_ftex, vec2(0.5, 0.5))`.

Our fragment look-up takes exactly one coordinate form, two lanes of one vec2
varying read whole, and refuses anything else with `MaliError::TexCoord`: in
the graph path's walk (`src/shaderc/online/mali/nir_fs.rs:595-604`) and its
varying layout (`src/shaderc/online/mali/nir_fs.rs:735-739`); the plan
path (`frag.rs` at da18ff68) was deleted in b75d518c. The graph path is what S1c made
the only one (`driver/docs/nanopc-t6-mali.md:296-298`).

### What Mesa does with it

`va_lower_tex` does not look at what the coordinate is: lane `i` of the
coordinate becomes staging register S + i, `nir_channel(coord, i)`
(`M:panfrost/compiler/pan_nir_lower_tex.c:831-832`). With no LOD source the
mode is `COMPUTED_LOD` (`M:panfrost/compiler/pan_nir_lower_tex.c:944-945`),
as for a varying coordinate, so the instruction and its flags are the
fragment `TEX_SINGLE` we build already. `bi_emit_tex_valhall` collects
whatever the staging defs are (`M:panfrost/compiler/bifrost/bifrost_compile.c:3508-3518`),
and a constant def is a `MOV.i32` of an immediate per 32-bit word, collected
(`M:panfrost/compiler/bifrost/bifrost_compile.c:2240-2256`). Only the
sources of the `COLLECT` differ from the varying case. There is no
constant-coordinate fast path on v10: `bi_lower_texs`, the TEXS path, is the
Bifrost (v6-v8) pass (`M:panfrost/compiler/pan_nir_lower_tex.c:203`,
`M:panfrost/compiler/pan_nir_lower_tex.c:1113-1120`).

Whether the BIR passes then fold the two equal `MOV`s into one register, and
where the scheduler puts them, is what the shim reference decides; nothing
here predicts those bytes.

### Does the general lowering's Tex row cover it?

The coordinate: yes, if the look-up takes its staging sources from the value
graph as Mesa does, any fp32 value (a constant, a varying lane, arithmetic),
instead of testing for "lanes 0 and 1 of one 2-component varying". Today
that look-up is not an `emit.rs` row: `emit.rs` is `bi_emit_alu`'s table
(`src/shaderc/online/mali/emit.rs:3-9`), and the look-up is `nir_fs.rs`'s
`tex` (`src/shaderc/online/mali/nir_fs.rs:953-966`). The landing is
therefore in `nir_fs.rs`: drop the coordinate test of the walk and the
layout, and collect the coordinate's two value lanes. `TexCoordShared` (a
coordinate lane also read converted in the colour,
`src/shaderc/online/mali/nir_fs.rs:815-818`) goes with the same change: the
staging register is a copy, so a lane read both ways is no conflict.

The shim references the row needs: a look-up at a constant coordinate and
one at a computed coordinate; the existing look-up references all use a
varying.

### What it does not fix: 122 stays refused

Once the coordinate is taken, the shader's next operation is
`fcols[fidx + int(...)]`, a uniform read at a run-time index. The fragment
siblings that read `fcols[fidx]` (`uniidx_a` to `_d`, `_g`, `_h`, `_j`) are
refused with `Valhall: a uniform read at a run-time index is not lowered
yet` (`MaliError::UniformIndexed`, `src/shaderc/online/mali/nir_fs.rs:379`,
`src/shaderc/online/mali/mod.rs:281`; `tools/mali/corpus-verdicts.txt:102-105`,
`tools/mali/corpus-verdicts.txt:108-109`, `tools/mali/corpus-verdicts.txt:111`).
So 122 moves to that string, not to PASS. The run-time index is outside
S1-S6, which read the uniform array at constant indices only
(`driver/docs/nanopc-t6-mali.md:239-240`; the uniform block's fallback is
named there as `LD_BUF`, `driver/docs/nanopc-t6-mali.md:280-281`).
Recommendation: #1385 owns 122's coordinate refusal; 122's PASS belongs to
the item that takes the run-time uniform index, with the other `uniidx`
cases.

## (c) The depth texture: case 169

### What the case needs

`case_dstex` (`driver/tools/pvrlab/gl_test.c:17606-17753`;
`GL_OES_depth_texture` and `GL_OES_packed_depth_stencil`), three steps, each
read back (`driver/tools/pvrlab/gl_test.c:17551-17574`):

1. a 4x1 `GL_DEPTH_COMPONENT` / `GL_UNSIGNED_SHORT` texture uploaded by the
   CPU (0, 1/3, 2/3, 1) and sampled NEAREST over the window, expected
   `(d, d, d, 1)` on ES 2.0 and `(d, 0, 0, 1)` on ES 3.0
   (`driver/tools/pvrlab/gl_test.c:17638-17660`);
2. one `GL_DEPTH_STENCIL` / `GL_UNSIGNED_INT_24_8` texture as the depth and
   the stencil attachment of an FBO, rendered into with stencil and depth
   tests (`driver/tools/pvrlab/gl_test.c:17662-17723`);
3. that texture sampled over the window: the depth the draws left
   (`driver/tools/pvrlab/gl_test.c:17725-17752`).

The GL core stores every depth texture as **R32F** texels
(`src/gl/texture.rs:5305-5309`). The Mali backend refuses the draw that
samples one, in this order: `depth_luminance != 0`
(`src/mali/backend.rs:1558`), a texel format it has no row for
(`texel_format` has RGBA8, sRGB8_A8 and the ETC2 formats,
`src/mali/backend.rs:1394-1429`; raised at `src/mali/backend.rs:1559`). And
`depth_resolve` is `None` (`src/mali/backend.rs:2365`), so step 2's depth
never reaches the texels.

### The `depth_resolve` contract (`YakoglBackend`)

The entry is `src/backend.rs:1351-1354`, the C form `src/backend.h:603`, the
offset checked at 160 (`src/abi_layout.rs:386`); the GL core calls it through
`backend::depth_resolve` (`src/backend.rs:1785-1804`):

```
depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32
```

- **`id`**: the GL object whose depth the backend keeps, the
  `GlesDraw::depth_id` form: the serial with bit 30 for a texture, without
  it for a renderbuffer (`ds_object_id`, `src/gl/context.rs:488-490`).
- **`dst`**: `h` rows of `pitch` floats, `w <= pitch`
  (`src/backend.rs:1796-1797`).
- **Callers.** `texture::resolve_depth` with a depth texture's level: `w`,
  `h` the level's size, `pitch = level_pitch_at(level)`
  (`src/gl/texture.rs:695-731`, the call at `src/gl/texture.rs:721`); it runs
  before a draw samples the texture, unless the backend samples its buffer
  itself (`depth_for_draw`, `src/gl/texture.rs:769-787`), and before the CPU
  writes it. And the depth blit, for a depth **renderbuffer** source, into a
  scratch of `w x h` floats at `pitch = w`
  (`src/gl/blit.rs:853-869`, the id at `src/gl/blit.rs:770`).
- **When.** Every kick that rendered the depth has completed: the GL core
  flushes and finishes first (`src/gl/texture.rs:706-709`). A backend may
  still answer `-EBUSY` for a buffer an open frame draws into, as a defect
  check.
- **Return.** `0`: written; the GL core flushes the CPU cache over the
  bytes, stamps the texels and counts the level
  (`src/gl/texture.rs:722-726`). `1`: the backend holds nothing for `id`
  (never written, or undefined); the texels stay. A negative errno: logged,
  the texels stay (`src/gl/texture.rs:727-728`). A table without the entry
  answers 1 (`src/backend.rs:1798-1804`).
- **Values.** Depth in [0, 1] as the R32F texel holds it; a 24-bit unorm
  is `(word & 0xffffff) / 16777215` (`unorm24_depth`,
  `src/ds_retention.rs:133-137`), and `write_depth_texels` writes a
  closure's value per texel, `None` leaving it
  (`src/ds_retention.rs:139-154`).
- **One way only.** Depth the CPU writes into a texture is never loaded into
  the backend's buffer (`src/gl/texture.rs:692-694`).
- **The direct path** is separate: `backend::samples_rendered_depth` lets a
  backend sample its own buffer instead (`src/backend.rs:1651-1688`); only
  the V3D answers yes, and its doc names the Mali as keeping the CPU
  resolve (`src/backend.rs:1667-1669`).

### How the other back ends implement it

All three: 1 for an uninitialised backend, `id` 0, a null `dst` or `w >
pitch`; the plane looked up by `id`; a plane that holds only a clear fills
its value everywhere; a multisampled buffer answers 1; the source is checked
against its allocation, `-EINVAL` past it; texels past the buffer are left.

| back end | entry | what it reads |
|---|---|---|
| V3D | `v3d_depth_resolve`, `src/v3d/backend.rs:4243-4291` (doc `src/v3d/backend.rs:4213-4242`) | the live zs buffer whose depth `id` owns (`ZsBuf::owners[0]`): `Cleared(v)` fills `v`; `Stored` and single-sampled: the D24S8 buffer in UIF without the XOR, detiled by `v3d_tex_load_box` into the rows, then each word `(word >> 8) / 16777215`, depth in bits 31..8; anything else 1. Draws sample the buffer directly instead (`v3d_depth_sampled`, `src/v3d/backend.rs:3717`), so this is the CPU's path alone. |
| Vivante (GC7000) | `gc_depth_resolve`, `src/vivante/backend.rs:2719-2725` -> `ds_cache::resolve_depth`, `src/vivante/ds_cache.rs:477-530` (doc `src/vivante/ds_cache.rs:460-476`) | the plane keyed by `id` at scale 1 (or a parked clear); `-EBUSY` while a pending kick uses the surface; the surface's D24S8 words **row-linear at its stride**, depth in bits 31..8, a row at a time; measured on the i.MX8MP (`docs/notes/gc-depth-resolve.md`) |
| PowerVR | `pvr_backend_depth_resolve`, `src/pvr/backend.rs:4177-4225` (doc `src/pvr/backend.rs:4159-4175`) | `-EBUSY` if the open frame's zs is `id`'s; a cleared or parked plane fills its value; a live single-sampled plane is the ISP's packed D24S8, **depth in bits 23..0**, linear at the ZLS-tile-aligned width `aw`, read through `unorm24_depth` |
| Mali | `depth_resolve: None`, `src/mali/backend.rs:2365` | nothing yet |

### `mali_depth_resolve`: the shape

The Mali backend has the PowerVR's bookkeeping already: `g.planes`, one
`PlaneEntry { id, gen, aspect, plane, ds_id }` a plane
(`src/mali/backend.rs:398-409`), made live or cleared when a frame is
submitted (`src/mali/backend.rs:1948-1958`), and `g.zsbufs`, one `ZsBuf {
ds_id, w, h, mem }` an attachment pair (`src/mali/backend.rs:386-396`),
allocated `4 * w * h` bytes by `zs_buffer`
(`src/mali/backend.rs:1769-1774`) with `row_stride = 4 * w`
(`src/mali/backend.rs:1915-1920`). The buffer is **linear D24S8, depth in
the low 24 bits, stencil in the high byte** (`ZsAttachment`,
`src/mali/frame.rs:996-1008`; Mesa's `Z24_UNORM_S8_UINT` is
`MALI_ZS_FORMAT_D24S8`, `M:panfrost/lib/pan_desc.c:188-189`): the PowerVR's
word layout, not the V3D's or the GC7000's. So the function is the
PowerVR's with the Mali's buffer:

- the `PlaneEntry` with `id` and aspect 0: a cleared value fills it;
  `is_live()` reads the `ZsBuf` whose `ds_id` is the entry's; no entry or
  no buffer answers 1;
- the words: `min(w, buf.w)` x `min(h, buf.h)` of them, row `y` at `y * 4 *
  buf.w`, each through `unorm24_depth`; no detile; the cache invalidated
  over the bytes read first, as the others do;
- `-EINVAL` for a buffer shorter than `4 * w * h`; `-EBUSY` when the open
  frame (`g.open`) still draws into that pair's buffer, a defect check (the
  GL core submits and waits first);
- 1 for a multisampled buffer; the Mali refuses 4x with depth/stencil today
  (#1386), so none exists yet;
- **pin on the board** that row 0 of our buffer is GL's y 0 for an FBO, as
  the V3D notes for its own (`src/v3d/backend.rs:4227-4228`): step 3 of 169
  reads the ramp and the top quarter at known rows. Mesa's own buffer has
  that order (below, "What the recording shows", fact 6), but that is Mesa's
  frame setup, not ours: ours stays open until a run of our frame reads it.

### How Mesa on v10 samples a depth attachment

1. **The view's format.** `panfrost_create_sampler_view_bo` keeps a depth
   view's depth format: `Z32_FLOAT_S8X24_UINT` reads its `Z32_FLOAT` part,
   `X32_S8X24_UINT` the separate stencil resource
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:1732-1739`). On v7+ the
   format table maps `Z16_UNORM` to `Z16_UNORM`, `Z24_UNORM_S8_UINT` and
   `Z24X8_UNORM` to `Z24X8_UNORM`, `Z32_FLOAT` to `Z32`, component order
   RGBA, flags `_T_Z__` (`M:panfrost/lib/pan_format.c:584-588`;
   `M:panfrost/lib/pan_format.c:140`); on v9+ the interchange formats
   `Z32_FLOAT_S8X24_UINT` to `R32F` GRBA and `X24S8_UINT`, `S8_UINT` to `S8`
   GRBA (`M:panfrost/lib/pan_format.c:590-602`). A colour `R32_FLOAT` is
   `R32F` with R001 (`M:panfrost/lib/pan_format.c:451`).
2. **The swizzle.** v7+ has no `_RRRR` order: for a depth or stencil view
   the driver composes the view's swizzle over `.XXXX`
   (`M:gallium/drivers/panfrost/pan_cmdstream.c:1843-1846`,
   `M:panfrost/lib/pan_texture.c:1146-1163`). The GL depth mode is part of
   the view's swizzle: `GL_LUMINANCE` is X X X 1 and `GL_RED` X 0 0 1
   (`M:mesa/main/teximage.c:792-828`), composed into the texture object's
   swizzle (`M:mesa/main/texobj.c:310-311`), `GL_LUMINANCE` the default
   outside desktop core profiles (`M:mesa/main/texobj.c:395`). Our
   `depth_luminance` (`src/backend.rs:806-811`) is exactly that choice; no
   shader change is involved.
3. **The descriptor.** Linear, 16x16 u-interleaved and 64K-interleaved
   images use a `GENERIC_PLANE`: clump ordering from the modifier,
   `clump_format = pan_clump_format(format)`, size, pointer, row stride,
   slice stride, extent (`M:panfrost/lib/pan_texture.c:425-459`, the
   modifiers asserted at `M:panfrost/lib/pan_texture.c:441-443`). Z16, Z24X8
   and Z32 are not in `special_clump_formats` and take the raw clump of
   their block size, `RAW16` / `RAW32`; X24S8 and S8 have their own
   (`M:panfrost/lib/pan_texture.c:191-241`,
   `M:panfrost/lib/pan_texture.c:303-312`). AFBC images use `AFBC_PLANE`
   (`M:panfrost/lib/pan_texture.c:651`). The `TEXTURE` header: dimension,
   format, width, height, sample count, swizzle, `texel_interleave =
   modifier != LINEAR || compressed` on v9+, levels, array size, the payload
   pointer, LOD range (`M:panfrost/lib/pan_texture.c:1271-1325`,
   `texel_interleave` at `M:panfrost/lib/pan_texture.c:1309-1311`; the v10
   layouts at `M:panfrost/genxml/v10.xml:1327`,
   `M:panfrost/genxml/v10.xml:1358`, `M:panfrost/genxml/v10.xml:1404`).
4. **AFBC or linear.** `panfrost_best_modifier` takes AFBC when
   `panfrost_should_afbc` holds, else 16x16 u-interleaved when it should
   tile, else linear (`M:gallium/drivers/panfrost/pan_resource.c:821-834`).
   Depth/stencil and sampler bindings are AFBC-able; only v7 and older
   exclude `Z32_FLOAT` (`M:gallium/drivers/panfrost/pan_resource.c:590-619`),
   and `Z24_UNORM_S8_UINT`, `Z24X8_UNORM` and `Z32_FLOAT` have an AFBC mode
   (`M:panfrost/lib/pan_afbc.h:590-592`, `M:panfrost/lib/pan_afbc.h:612-623`).
   The source reading was that on a device with AFBC (`dev->has_afbc`)
   Mesa's depth attachment is AFBC by default. **The recording says
   otherwise for this case**: the attachment is linear (below, fact 1). Ours
   is linear by construction (`src/mali/frame.rs:996-1008`), the frame
   descriptors pinned to Mesa's linear case, so here there is no
   difference.
5. **Our precedent.** The Z/S preload already samples our linear D24S8
   buffer as a texture: depth as `Z24X8_UNORM` with the `RAW32` clump,
   stencil as `S8` with the `X24S8` clump, one `GenericPlane` over the buffer
   at its row stride, swizzle 0 (`src/mali/frame.rs:2074-2150`). The
   descriptor for sampling our own depth buffer is therefore known; only the
   row-stride rule below limits it.

### What the recording shows (`mesa-dstex.pandecode`)

Read from `0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/mesa/mesa-dstex.pandecode`
(9149 lines; Mesa 25.0.7, Mali-G610, ES 3.1, `mesa-dstex.stdout`), with the
case source `0dce654a8:tools/mali/board-ref/2026-10-20-mesa-refs/cases.c:899-1020` and
its `README.md`. Line numbers are of the pandecode file. The three GL frames
are three dump contexts (`run-dstex.txt`: `ctx-0.0000..0002`), concatenated:
frame 1 (lines 1-1960) samples the CPU-uploaded ramp; frame 2 (1960-5870)
renders into the depth-stencil texture; frame 3 (5870-9149) samples it.

1. **Format and storage of the attachment.** Frame 2's `ZS CRC Extension`
   (3672-3693): `ZS Write Format: D24S8` (3673), `ZS Block Format: Linear`
   (3674), `ZS MSAA: Single` (3675), `ZS Writeback Base: 0x7fffffec0000`
   (3685), `ZS Writeback Row Stride: 256` (3686), `ZS Writeback Surface
   Stride: 0` (3687), `ZS AFBC Body Offset: 0` (3688), `CRC Base: 0x0` and
   `CRC Row Stride: 0` (no CRC). The frame word: `Z Internal Format: D24`
   (3551), `Z Write Enable: true` (3552), `Z Clear: 1.000000` (3560), `Z
   Preload Enable` and `Z Unload Enable: false` (3553-3554), `Has ZS CRC
   Extension: true` (3555). So: **linear D24S8, 4 bytes a texel, row stride
   256 = 4 x 64, 64 rows, 16384 bytes**, not AFBC and not u-interleaved. The
   same layout as ours (`ZsAttachment`, `src/mali/frame.rs:996-1008`). Why
   Mesa did not pick AFBC (the source reading above expected it) is not in
   the recording. The open fact is its choice for a larger texture: **a
   Mesa run of the `dstex` mode with `TW`/`TH` of 256 or more would settle
   it** (not run).
2. **The stencil and the word layout.** `S Write Format: XXX: INVALID`
   (3677), `S Block Format: No Write` (3678), `S Write Enable: false` (3548)
   and `S Writeback Row Stride: 0` (3690), yet the stencil reference 1 is in
   the memory: the packed D24S8 write puts it in the **high byte**. The
   buffer dump of `0x7fffffec0000` (4963-) reads, bytes little-endian, row 0
   `0x00020000, 0x00040000, ... 0x00800000` (4965-4980: depth in bits 23..0,
   high byte 0) and row 1 (offset `0x100`, 4981) beginning `0x01040000`, a
   high byte 1 where the upper-left triangle's stencil was written. This is
   the `unorm24_depth` layout (`word & 0xffffff`) the shape above assumes.
3. **No resolve, no copy.** Mesa samples the attachment directly. Frame 3's
   `Texture` (6116-6172) has `Pointer: 0x7fffffec0000` (6168), the very
   address of frame 2's `ZS Writeback Base` (3685). There is no blit, copy
   or resolve job between the frames (three dump contexts for three GL
   frames; frame 3's own `ZS CRC Extension`, 6644-6661, is the window's
   `0x7ffffffe0000` with `Z Write Enable: false`, 6600). The texture storage
   is one allocation serving as the attachment and the texture: option B's
   shape, in Mesa.
4. **The texture descriptor** (frame 3, 6116-6172). `Format (v7): Z24X8
   UNORM RGBA` (6122), `Width: 64`, `Height: 64`, `Swizzle: 2560 (RRR1)`
   (6125), `Texel interleave: false` (6126), `Levels: 1`, `Sample count: 1`
   (6130), `Dimension: 2D` (6119), `Plane type: Generic` (6137), `Clump
   ordering: Linear` (6165), `Clump format: RAW32` (6166), `Size: 16384`
   (6167), `Row stride: 256` (6169), `Slice stride: 16384` (6170). The AFBC,
   ASTC and AFRC members the printer shows under the plane are the union's
   other views, not set. The format is the one the Z/S preload already
   builds from our buffer (`Z24X8_UNORM`, `RAW32`,
   `src/mali/frame.rs:2074-2150`). `2560` is `RRR1`: the GL luminance depth
   mode composed over `.XXXX` (item 2 above), so the shader reads `(d, d, d,
   1)` and the read-backs are `0xff808080` etc. (`mesa-dstex.stdout`).
   **Mesa's context is ES 3.1 and still returns the luminance form**, so the
   case's `(d, 0, 0, 1)` rule for non-ES 2 fails on Mesa itself (checks 2-4
   and 9-12 MISMATCH on the channels only; every depth is right: `0x00,
   0x55, 0xaa, 0xff` and `0x80, 0x48, 0x4a, 0xb5`): Mesa is no reference for
   the channels. Frame 1's ramp is `Z16 UNORM RGBA` (352), `RRR1` (355),
   `RAW16` (396), `Size: 64` (397), `Row stride: 64` (399): a 4 x 1 texture
   of 8 bytes padded to a 64-byte linear row, the rule #1389 gives our rows.
5. **The sampler** (frame 3, 6086-6111, the fragment table's entry 3, 6082).
   `Minify nearest: true` (6098), `Magnify nearest: true` (6099), `Mipmap
   Mode: None` (6101), `Compare Function: Never` (6103), `Wrap Mode S/T/R:
   Repeat` (6089-6091; the case sets CLAMP_TO_EDGE only on the ramp
   texture), `Normalized Coordinates: true` (6096), `Maximum LOD
   31.996094`. `Never` is the descriptor's no-compare value (the case sets no
   `GL_TEXTURE_COMPARE_MODE`): **no depth compare state is involved**, so
   nothing in `GlesTextureBind` has to carry one. Frame 1's sampler (316-335)
   is the same. Frame 2's draws bind a stale `Z16 UNORM RGBA` texture
   (2593-2650, 3174-3230) in their resource table although their shader
   samples nothing; it has no bearing.
6. **Row order.** Frame 2's blue quad has window depth 0 at the GL
   bottom-left corner (`dsk_quad(.., -1, 1)`, `cases.c:991-993`) and rises
   to 0.5 along the bottom row. Buffer row 0 holds `0x020000 .. 0x800000`
   (item 2), so **row 0 of Mesa's depth buffer is GL's y 0 (the bottom
   row)**, rows ascending, as the case's `dst_ramp_byte` reads it. The
   stencil triangle shows in row 1 at x 0 only, the same orientation.

**Open after the recording**, each with the run that would settle it:

- Mesa's choice of AFBC or linear for a depth texture of another size
  (fact 1): the `dstex` mode of `mref-local.sh` at 256 x 256 or larger. Not
  needed for our plan (ours is linear).
- Mesa's row stride for a depth texture whose width is not a multiple of 16
  (64 wide gives 256, already a multiple of 64): the same mode at, say, 40 x
  40. It decides whether Mesa pads the attachment's rows for sampling; for
  ours, option B's 64-byte limit stands until it is answered.
- Our own buffer's row order (fact 6 is Mesa's): the Mali board run of 169
  once `mali_depth_resolve` exists (the case's three read-backs).

### Landing site

Two ways to give a draw the depth, as on the V3D:

- **A, the contract's own (smallest): resolve into R32F texels.**
  `mali_depth_resolve` as above, `depth_resolve: Some(..)` in the table
  (offset 160 unchanged); an R32F row in `texel_format`: `R32F` with the
  R001 order, `RAW32`, 4-byte texels, as Mesa's colour `R32_FLOAT`; and the
  `depth_luminance` refusal (`src/mali/backend.rs:1558`) replaced by the
  view swizzle, X X X 1 when it is 1, else the R001 order's own. Step 1 of
  169 needs only the R32F row; steps 2 and 3 need the hook. Whether R32F
  joins `SAMPLED_TEXFMTS` (`src/mali/backend.rs:84-87`) is a separate
  choice: the mask decides the storage of ES 3.0 sized formats
  (`src/backend.rs:221-227`, used at `src/gl/es3_formats.rs:189`), and the
  depth formats are R32F whatever it says (`src/gl/texture.rs:5305-5309`).
- **B, later (performance): sample the zs buffer.** `samples_rendered_depth`
  answering for the Mali (`src/backend.rs:1678-1688`), and the backend
  building the preload's `Z24X8_UNORM` descriptor over the buffer for a
  binding with `GlesTextureBind::depth_id` (`src/backend.rs:821-829`). The
  limit: the texture unit reads a linear row stride only at a multiple of
  64 bytes (`TEXTURE_ROW_ALIGN`, `src/mali/backend.rs:219-228`), and the zs
  buffer's stride is `4 * w`, so only widths that are a multiple of 16 could
  be sampled in place; the buffer would have to be allocated at a 64-byte
  stride first (the frame's ZS descriptors take the stride,
  `src/mali/frame.rs:1667`). Not needed for 169.

**Landed 2026-10-23 (A):** `mali_depth_resolve` over the pair's linear
D24S8 buffer, the R32F row of `texel_format` (`R32F` in the R001 order,
which v9+ spells `RGB1`, `RAW32`) and the luminance swizzle `RRR1` for
`depth_luminance` (src/mali/backend.rs; the descriptors against Mesa's
frames 1 and 3 in src/mali/frame_tex_tests.rs `check_depth`, the excerpt
tools/mali/board-dump/dstex-2026-10-20.txt). The board fact is settled:
row 0 of our buffer is GL's y 0 (case 169's checks 9-12 pass on the T6,
0dce654a8:tools/mali/board-ref/2026-10-23-tex-169). Our ES 3.0 surface reads
(d, 0, 0, 1) through the identity swizzle over `R001`; `RRR1` is bound
only by a `report-es2` build.

**After the recording: the plan stands.** Nothing in it moves A. The
recording confirms the descriptor option B would build over our own buffer
(`Z24X8_UNORM`, `RAW32`, linear, row stride 256 = `4 * w` for a 64-wide
buffer, swizzle `RRR1`; facts 3 and 4), and that Mesa samples in place with
no resolve. B stays later, limited by the 64-byte stride rule (a width that
is a multiple of 16) as written above. The sampler needs no compare state
(fact 5). A's R32F row keeps the `R001` / `RAW32` form for the texels the CPU
resolve writes; the luminance form `(d, d, d, 1)` is the view swizzle for
`depth_luminance`, as Mesa's `RRR1`.

**Refusals removed** (A): `mali: a depth texture sampled as luminance is not
built yet` (`NOT_BUILT_TEXTURE_DEPTH`) and, for R32F textures, `mali: a
texture stored other than RGBA8, sRGB8_A8 or ETC2 (RGB8, RGBA8, RGB8A1) is
not built yet` (`NOT_BUILT_TEXTURE_FORMAT`); `depth_resolve: None` becomes
the function.

### Does PR #1389 (64-byte linear row alignment) change any of this?

Read from its diff, `git diff b914d5699 a0aff40b6 -- src` (the merge base
with main and the tip of `work/mali-row-align`); not run.

- It adds `YakoglBackend::linear_row_align` (`a0aff40b6:src/backend.rs:1388-1402`),
  64 on the Mali, and the GL core then lays every linear row it allocates
  at that multiple: texture levels through `chain_layout_npot`'s chain pitch
  (`a0aff40b6:src/gl/texture.rs:1495-1501`), compressed raster rows,
  renderbuffers and surfaces. A depth texture's R32F level pitch becomes a
  multiple of 16 floats.
- **The hook is unchanged.** `depth_resolve` is handed the level pitch the
  GL core chose (`level_pitch_at`), and a `mali_depth_resolve` written to
  the contract (`h` rows of `pitch` floats) needs nothing from #1389.
- **What it changes for 169's sampling.** On main a texture whose rows are
  not a multiple of 64 bytes is sampled from a copy in the frame's data
  arena (`needs_copy`, `src/mali/backend.rs:1475-1478`); step 1's 4x1 R32F
  texture has 16-byte rows and would be copied. With #1389 it is sampled in
  place, and copies remain only for imported images (`row_copies`). Both
  are correct. An R32F row reads its pitch from `t.stride_texels` in
  `level_at` (`src/mali/backend.rs:1452-1454`), the branch #1389 leaves as
  it is (it rewrites the block-format branch,
  `src/mali/backend.rs:1455-1458` on main). So #1389 changes no design here;
  building the depth work on main after it lands, as the plan says, only
  avoids editing `level_at` and `needs_copy` on both sides.
- **The zs buffer** is the backend's own allocation, not the GL core's: #1389
  leaves it at `4 * w` (option B's limit stands).

## (d) A computed coordinate: case 121 and femtovg `fill_image*` / `filter_image*`

The study for plan #153's `texcoord` item (#1385, #1384). Nothing in `src/`
was changed for it.

**Citations in this section.** Mesa as `M:` above. Our code is named by file
and symbol (a function, a match arm, an enum variant), **never by line**, so
the anchors stay true while the lowering items move the lines; the state
read is main 71c586bc. The corpus is quoted by shader path and verdict
string, not by row number, for the same reason. Every Mesa instruction
quoted is copied from a committed `.expect.json`, named with it.

### The refusal and the rows

`Valhall: a texture coordinate other than a vec2 varying read whole or two
constants is not lowered yet` (`MaliError::TexCoord`). It is raised in
`src/shaderc/online/mali/nir_fs.rs`, `walk_nodes`, the `ir::Op::Tex` arm,
which classifies the coordinate's two lanes into `TexCoord::Varying` (lanes 0
and 1 of one 2-component varying) or `TexCoord::Const` (two constants) and
refuses everything else; `layout` raises it again when the varying is not the
first two lanes of its load. At main it is the verdict, in
`tools/mali/corpus-verdicts.txt`, of exactly these 18 shaders:

- `tools/shaders/es100/uniidx_h.frag` (case 121 `uniidxh`);
- `tools/shaders/femtovg/fill_image{,.glyph,.glyph.nodiscard,.nodiscard,.nodiscard.noscissor}.frag`;
- `tools/shaders/femtovg/fill_image_gradient{,.glyph,.glyph.nodiscard,.nodiscard,.nodiscard.noscissor}.frag`;
- `tools/shaders/femtovg/fill_image_gradient_conic{,.glyph,.glyph.nodiscard,.nodiscard,.nodiscard.noscissor}.frag`;
- `tools/shaders/femtovg/filter_image{,.nodiscard}.frag`.

These 17 femtovg rows are every femtovg/fvgtest row that is not `ok` at
main (`grep -E '^tools/shaders/(femtovg|fvgtest)/'
tools/mali/corpus-verdicts.txt | grep -v ': ok$'`), all with this string.
None refuses with `FusedMultiplyAdd` any more: the `ffma` of `mix()` was
lowered by dc82c41c.

**The references.** Every femtovg program has its drm-shim reference
already: `tools/mali/testdata/fvg_fill_image*.{vert,frag,expect.json}` (15)
and `fvg_filter_image{,_nodiscard}.*` (2); nothing was missing, nothing was
generated. Case 121's pair is new: `tools/mali/testdata/case121.frag` is
`tools/shaders/es100/uniidx_h.frag` byte for byte, `case121.vert` the
statements of `gl_test_passthrough_vs_src` (its comment reworded) (`driver/tools/pvrlab/gl_test_shaders_es100.h`,
the vertex shader `case_uniidx_tex` links for 120, 121 and 122). It was drawn
on Mesa 26.2.1's drm-shim **without** `--uniform-values` (`gl_ref.py` sets
float uniforms only; the index `fidx` is an int, so it stays 0 as in the
`uniidx_*` rows of `tools/mali/testdata/cases.txt`):

```
tools/mali/drm_shim_ref.sh run $CONNECTED_DEPS_DIR/mesa-shim \
    tools/mali/testdata/case121.vert tools/mali/testdata/case121.frag /tmp/case121
python3 -I tools/mali/pan_ref_diff.py extract /tmp/case121.trace --draw \
    > tools/mali/testdata/case121.expect.json
```

### What Mesa emits: femtovg `fill_image` (highp)

`tools/mali/testdata/fvg_fill_image.frag`: `vec2 pt = (paintMat * vec3(fpos,
1.0)).xy / extent; texture2D(tex, pt)`. `fvg_fill_image.expect.json`,
`fs.fragment`, the first 14 instructions:

```
LD_VAR_BUF_IMM.f32.slot0.v4.src_f32.center.store.wait0 @r0:r1:r2:r3, r61^, index:0x0
FMA.f32 r4, u5.w0, r2, u5.w1
FMA.f32 r4, u9.w1, r3, r4^
FRCP.f32 r5, u13.w0
FMA.f32 r4, r4^, r5^, 0x0.neg                   ; pt.y = (paintMat * (fpos, 1)).y / extent.y
FMA.f32 r5, u4.w0, r2, u4.w1
FMA.f32 r5, u9.w0, r3, r5^
FRCP.f32 r6, u12.w1
FMA.f32 r5, r5^, r6^, 0x0.neg                   ; pt.x
MOV.i32 r6, r5^                                 ; staging S
MOV.i32 r7, r4^                                 ; staging T
MOV.i32 r4, u15.w0
MOV.i32 r5, 0x0                                 ; the handle pair
TEX_SINGLE.slot1.skip.reserved.32.2d.computed.discard @r4:r5:r6:r7, @r6:r7, [r4^:r5^]
```

- **The staging is a register pair, whatever computed it.** `va_lower_tex`
  makes lane `i` of the coordinate staging word `S + i`, a value of the graph
  (`M:panfrost/compiler/pan_nir_lower_tex.c:831-832`); with no level it is
  `COMPUTED_LOD` (`M:panfrost/compiler/pan_nir_lower_tex.c:944-945`), the same instruction and flags as at a
  varying coordinate. The two `MOV.i32` are the `COLLECT` of the two values
  (`bi_lower_vector`): the chains end in `r5`, `r4`, not in the consecutive
  pair the TEX reads.
- **The destination overlaps the staging** (`@r4:r5:r6:r7` written, `@r6:r7`
  read): the staging is read first, so the allocator may reuse it.
- **The varying is loaded whole, in registers.** `.v4`: the packed slot
  (`ftcoord` in `r0:r1`, `fpos` in `r2:r3`), the coordinate computed from
  `r2:r3`. No varying-texture fusion: the `VAR_TEX_*` instructions are unused
  on Valhall (`docs/notes/mali-cube-textures.md`, section (g)).
- **The precision of the texel.** `.32`: a highp shader reads a 32-bit
  texel (`f16` false in our `Op::TexSingle`, which `tex` derives from the
  look-up node's `bits`).
- `skip` and `.discard`, the helper flags, come from `flow.rs`
  (`analyze_helper_requirements`, the helper terminate) as for every
  computed look-up.

### What Mesa emits: case 121 (mediump)

`vec4 c = fcols[fidx]; gl_FragColor = 0.5 * (c + texture2D(ftex, vec2(c.w *
0.5)));`. `case121.expect.json`, `fs.fragment`, whole:

```
MOV.i32 r0, 0x3020100
LSHIFT_OR.i32 r0, u2.w0, r0^.b33, 0x0
LD_PKA.i64.unsigned.slot0.wait0 @r0:r1, r0^, 0x0     ; fcols[fidx], four fp16 lanes
FMA.v2f16 r2, r1.h11, 0x38000000.h11, 0x0.neg       ; c.w * 0.5, in fp16
F16_TO_F32 r2, r2^.h0                               ; the coordinate to fp32
MOV.i32 r3, r2                                      ; staging T = S: one value twice
MOV.i32 r4, u2.w1
MOV.i32 r5, 0x0                                     ; the handle pair
TEX_SINGLE.slot1.skip.reserved.16.2d.computed.wait1 @r2:r3, @r2:r3, [r4^:r5^]
FADD.v2f16.discard r0, r0^, r2^
FADD.v2f16 r1, r1^, r3^
FMA.v2f16 r0, 0x38000000.h11, r0^, 0x0.neg
FMA.v2f16.wait0126 r1, 0x38000000.h11, r1^, 0x0.neg
ATEST.wait @r60, r60^, r1.h1, atest_datum.w0
BLEND.slot2.v4.f16.end @r0:r1, blend_descriptor_0.w0, r60, target:0x0
IADD_IMM.i32 r48, 0x0, #0x0
BRANCHZI.eq.absolute 0x0, blend_descriptor_0.w1
```

- The coordinate is computed in fp16 (`FMA.v2f16`), converted once
  (`F16_TO_F32`), and the one value is both staging words: the `COLLECT` of
  `(v, v)` is a single `MOV.i32` into the neighbour register, the value
  itself already sitting in the first.
- The texel is `.16`: a mediump shader's look-up, as `sel_t_tex`'s.
- The indexed read is case 122's (`LSHIFT_OR` / `LD_PKA.i64`,
  `docs/notes/mali-uniform-index.md`). At main it is lowered for a mediump
  shader (`walk_nodes`'s `ir::Op::LoadUniformIdx` arm, the `!prec.highp`
  guard); `uniidx_h.frag` passes it and stops at the coordinate, which is why
  its verdict is `TexCoord`, not `UniformIndexed`. So case 121 needs only
  this item's coordinate on the compiler side.

### What `texcoord` needs in our code (not done here)

1. **`nir_fs.rs`, `walk_nodes`, the `ir::Op::Tex` arm.** Keep the
   `TexKind` and `TexCount` checks; replace the `_ => return
   Err(MaliError::TexCoord)` fallback of the coordinate match by a third
   form, `TexCoord::Computed(s, t)` holding the two lane values the arm
   already has (`lane(vals, coord, 0)`, `lane(vals, coord, 1)`). The
   `Varying` and `Const` forms stay as they are: their bytes are pinned
   (`sel_t_tex`, `tex_coord_const`).
2. **`nir_fs.rs`, `layout`.** The `read |= 0b11` for a `TexCoord::Varying`
   and the final check that the coordinate is the first two lanes of one load
   stay for that form. For `Computed`, the two values are roots like the
   colour's: the `reaches` walk over them marks the varying lanes the
   coordinate reads (`fpos`), and nothing else special is needed.
3. **`nir_fs.rs`, the two selectors.** The coordinate's two values must be
   selected before the TEX event, in the order Mesa's listing has them
   (ahead of the TEX, after the loads they read; case 121's after its UBO
   event): in `select_highp` (femtovg) as fp32 values (`src32`), in
   `select` (case 121) as a lane of the fp16 pair tree converted with
   `F16_TO_F32`. `FMA.f32`, `FRCP.f32` and `FMA.v2f16` are rows already;
   **`F16_TO_F32` is not**: the ISA table has it (`isa::v10::Op::F16ToF32`
   in `src/shaderc/online/mali/isa/v10.rs`), `bir.rs` has no `Op` for it, so
   it needs a BIR op, its source info and a packer arm (the
   `mali-alu-ops.md` transcendental rows need the same op). The check `MaliError::TexCoordShared` in
   `select` (a varying coordinate lane also read converted by the colour)
   applies only to the `Varying` form: with a computed coordinate the
   staging is a copy and a lane read both ways is no conflict.
4. **`nir_fs.rs`, `tex`.** Its `staging` match gains the `Computed` arm:
   the two selected sources into the same `Op::Collect`; `sr_count` 2,
   `LodMode::Computed`, `TexDim::D2` unchanged. `ra.rs`'s `lower_vector`
   turns the `Collect` into one `MOV.i32` per source; case 121's single
   `MOV.i32 r3, r2` needs the allocator to put the value in the collect's
   first register and the self-move to go, which is what the byte-equal check
   on `case121.expect.json` settles.
5. **What stays refused after it.** The corpus records the first refusal
   only; the references show the next one for 8 of the 17 femtovg programs:
   the six `*glyph*` programs read two textures (`tex` and `glyphtex`, 2
   `TEX_SINGLE` in each reference) and `filter_image{,_nodiscard}` reads one
   49 times (the blur loop unrolled: 49 `TEX_SINGLE`), and the arm refuses a
   second look-up with `MaliError::TexCount` (`f.look_up` is a single
   `TexLookup`). The nine programs with one look-up (`fill_image`,
   `fill_image_gradient`, `fill_image_gradient_conic`, each plain,
   `nodiscard` and `nodiscard_noscissor`) and case 121 are what this item
   can bring to `ok`.
6. **The checks.** Byte-equal to the nine single-look-up `fvg_*`
   references and `case121.expect.json` (`fs.fragment`) through the
   testdata tests in `src/shaderc/online/mali/tests.rs`; then
   `tools/mali/corpus-verdicts.sh`, which must move those ten rows to `ok` and
   the other eight to `TexCount` (never backwards); then case 121 on the board
   (x2) and the sweep. `src/mali` needs nothing: the texture and sampler
   tables are those of any fragment look-up, the coordinate does not reach
   the descriptors.

## Summary per item

| item | Mesa v10 mechanism | landing site | refusal removed | then |
|---|---|---|---|---|
| tex-vs (113, 197) | per-stage TEXTURE and SAMPLER descriptors in the vertex resource table, shared by both IDVS shaders; `TEX_SINGLE` with `ZERO_LOD` or `EXPLICIT` 8.8 LOD, kept in the variant whose outputs need it; cube through `build_cube_desc` | the vertex walk (`src/shaderc/online/mali/nir.rs:837`) on the graph, the variants' DCE as it is; `EXPLICIT` in `Op::TexSingle`; a vertex texture, sampler and `TABLE_TEXTURE` in `vs_res` (`src/mali/frame.rs:2466-2470`); the vertex binds read from `GlesDraw::vs_textures` | `a texture look-up in a vertex shader` | 197: (b) for its FS, then the 1.2 MB draw; 113: the cube form and `NOT_BUILT_TEXTURE_TYPE` |
| tex-coord-122 | the coordinate's lanes are plain staging sources, a constant a `MOV` of an immediate; `COMPUTED_LOD` | `nir_fs.rs`'s look-up takes its coordinate lanes from the value graph | `a texture coordinate other than a vec2 varying read whole` (also 197's FS) | 122: `a uniform read at a run-time index` |
| texcoord (121, femtovg `fill_image*`; section (d)) | the coordinate's lanes are staging words whatever computed them, `COLLECT`ed, `COMPUTED_LOD`; no `VAR_TEX` fusion on v10 | `nir_fs.rs`: a `TexCoord::Computed` form in `walk_nodes`'s `ir::Op::Tex` arm, its values as roots in `layout`, selected before the TEX in `select` / `select_highp`, collected in `tex`; a BIR `F16_TO_F32` for the mediump coordinate | `a texture coordinate other than a vec2 varying read whole or two constants` (18 rows) | 121 and 9 femtovg programs `ok`; the 6 `*glyph*` and 2 `filter_image*` programs to `TexCount` |
| tex-depth-169 | depth formats `Z24X8_UNORM` / `Z32`, swizzle composed over `.XXXX`, the GL depth mode a view swizzle; `GENERIC_PLANE`; in the recording the attachment is linear D24S8 (stride 256) and the draw samples it in place as `Z24X8_UNORM` `RAW32` `RRR1` (`mesa-dstex.pandecode:3672-3690`, `6116-6172`) | `mali_depth_resolve` over the linear D24S8 zs buffer (depth low 24 bits), an R32F `texel_format` row, `depth_luminance` as the view swizzle | `a depth texture sampled as luminance`, R32F's `texture stored other than`, `depth_resolve: None` | board: our buffer's row order (step 3); Mesa's is GL y 0 first; open for other sizes: AFBC choice, stride at widths not a multiple of 16 |
