# Mali (Valhall, v10): a vertex cube look-up read as six array layers (`cube_noseam`)

Item cube-noseam of plan #373. It fixes the four dEQP-GLES2 vertex cube cases
that the R373 board record (`tools/cts/log/2026-11-21-t6-gles2-R373-a2482523/`)
still fails:

- `texture.vertex.cube.filtering.linear_mipmap_linear_{linear,nearest}_repeat`
- `texture.vertex.cube.wrap.repeat_repeat`
- `shaders.texture_functions.vertex.texturecubelod`

docs/es2-cts-failures.md section M.1 has them as rows 5 and 11. The cause is in
docs/notes/mali-cube-textures.md section (l) (check-rows5, `f4d444a5`). Our
descriptor words and vertex program are already Mesa's. The difference is in
the hardware: on a cube that is not seamless, the Mali texture unit does not
wrap a LINEAR tap inside the selected face. ES 2.0 3.7.5 and dEQP-GLES2's
reference do wrap it. That reference is non-seamless for a context without
`GL_MAJOR_VERSION` (`es2fVertexTextureTests.cpp:1154`), and it samples the one
face with the sampler's own wrap modes. Neither the seamless bit nor a
clamped R moved the board's result in the right direction (section (l),
cube-seam-probe).

## Not Mesa's: our variant

Mesa 26.2.1 has no such variant on the Mali. In its ES 3 context every cube
is seamless: `ctx->Texture.CubeMapSeamless = _mesa_is_gles3(ctx)`
(`src/mesa/main/version.c:690`), and st/mesa then clamps the three wrap modes
(`src/mesa/state_tracker/st_atom_sampler.c:84-94`). Under
`MESA_GLES_VERSION_OVERRIDE=2.0` it keeps the cube descriptor, with the words
we wrote before this item (the `es2` references of `tools/mali/testdata/vs_cube_edge*`).
So no drm-shim trace of the whole variant can exist. The fix
has the V3D's shape for the same dEQP rule (`docs/v3d-noseamless-cube.md`,
`LodKey::cube_noseam`): read the cube as the 2D array of its six faces, so
that the unit's own 2D wrap and filtering apply inside the face. The passing
`texture.vertex.2d.*repeat*` cases exercise exactly that.

Every part the variant is built from is pinned against Mesa 26.2.1, cited as
`M:` = `src/` of the 26.2.1 tarball (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`):

- **The face sequence** is `build_cube_coord2_face`
  (`M:panfrost/compiler/pan_nir_lower_tex.c:68-101`): `CUBEFACE1`,
  `CUBEFACE2_V9`, `CUBE_SSEL`, `CUBE_TSEL`, `FRCP`, and the two saturated
  `FMA`s. It is the same sequence as the cube look-up's. Only
  `build_cube_desc`'s `MUX` (`:103-128`), which packs the face above bit 29
  of S, is left out.
- **The layer** is `u2f32(face >> 29)`. `CUBEFACE2_V9` leaves the face index
  in bits 29-31. The shift is Mesa's `RSHIFT_OR.i32 a, 0x1F1E1D1C.b11, 0x0`
  form: byte 1 of the small-constant word is 29.
- **The look-up** is `TEX_SINGLE.slot1.reserved.32.2d.array_enable.explicit`
  with four staging words in Mesa's order: S, T, the layer, then the 8.8
  level (`enum valhall_tex_sreg`, `M:panfrost/compiler/pan_nir_lower_tex.c:787-799`;
  `va_lower_tex` `:803-973`, the array index at `:835-849`). On v10 the
  layer is the coordinate's fp32 lane as it is, not rounded to an integer.
  Bifrost rounds it (`nir_f2u32_rtne`, `:520-532`); `va_lower_tex` does not.
  Reference: Mesa's own compiler (pan_shc) on a vertex
  `textureLod(sampler2DArray, vec3, float)`,
  `tools/mali/testdata/pan_tex_ref/vcube_layers.{vert,frag,pan_shc.disasm}`.
- **The texture descriptor** is a 2D array's
  (`pan_sampled_texture_emit`, `M:panfrost/lib/pan_texture.c:1272-1325`):
  `Dimension` 2D and `Array size` 6 (`pan_texture_get_array_size`,
  `:1199-1217`, with no cube division). The planes are the cube's 30, already
  layer-major and level-minor as a 2D array's are. The shim's
  texmore_206_array trace pins this form for a real 2D array
  (src/mali/frame_tex_tests.rs).
- **The sampler** keeps GL's S, T and R wraps. `Seamless Cube Map` is set, as
  on every sampler of a texture that is not a cube (`cso->seamless_cube_map`,
  `M:gallium/drivers/panfrost/pan_cmdstream.c:231`). It filters nothing on a
  2D array.

The `pan_shc` reference was made with:

```sh
glslangValidator -G -S vert tools/mali/testdata/pan_tex_ref/vcube_layers.vert -o v.spv
glslangValidator -G -S frag tools/mali/testdata/pan_tex_ref/vcube_layers.frag -o f.spv
$PAN_SHC v.spv f.spv > out.json 2> tools/mali/testdata/pan_tex_ref/vcube_layers.pan_shc.disasm
```

`PAN_SHC` is `tools/mali/build_pan_shc.sh`'s build
(`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1/build-pan/src/panfrost/pan_shc/pan_shc`),
and the glslang used was 16.2.0. `glsl2pan.py` is not used here, because it
rewrites GLSL ES 1.00 and ES 1.00 has no `sampler2DArray`.

## The mechanism

1. **The key: which bindings.** `online::backend::sensitivity_of`
   (src/shaderc/online/backend.rs, `mali_cube_bindings`) reports
   `LodKey::cube_noseam` for each cube binding below 15 that a Mali vertex
   stage reads. It does so only in a `report-es2` build, the board's
   dEQP-GLES2 library. An ES 3 context is judged against the seamless
   reference, which the hardware's cube path already meets.
2. **The key: which states.** The GL core's `stage_lod_key` (src/gl/draw.rs,
   unchanged) sets the bit only when the unit's filters can read past a face
   edge: mag LINEAR, or min LINEAR or LINEAR_MIPMAP_*
   (`cube_filters_see_seams`). It then asks for the sampler-state variant
   (`online_sampler_variant`), as it already does for `mag_switch`. The two
   keys combine.
3. **The compiler** (src/shaderc/online/mali). The vertex walk (`nir.rs`, the
   `IrOp::Tex` arm) reads `compile_lod_key().cube_noseam`. For a keyed cube
   binding it builds `Nir::cube_layer` (S, T and the layer) in place of
   `Nir::cube_desc`. The look-up is then a `Kind::VsTex { layers: true }`
   whose third source is a `Kind::VsTexLayer` node. A node has three
   sources, so that node carries the layer and the level, and it makes no
   code. Selection (`select.rs`) stages the four words and emits
   `TexDim::D2Array`. The record's `MaliShader::cube_layers` gets the
   look-up's descriptor-index bit. `cube_samplers` keeps it too, so an
   incomplete map still reads the black cube.
4. **The back end** (src/mali/backend.rs). `stage_texture` passes the
   binding's texture, or the black fallback cube, through `cube_as_layers`
   when its `cube_layers` bit is set. That sets `SampledTexture::array` on a
   six-face cube. `texture()` then writes `Dimension` 2D and `Array size` 6,
   and `sampler_with` keeps GL's wraps with `Seamless Cube Map` set. A
   texture that is not a six-face cube is left as it is.

Nothing changes without the key. A build without `report-es2` never asks for
it. The fragment stage is not keyed: `texture.filtering.cube.*` and
`texture.mipmap.cube.*` pass on the board with the cube path. A fragment
version would also need the level computed from the direction's
derivatives, as the V3D's does. The glmark2 corpus compiles every shader with
no key, so its verdicts do not move (`tests/mali_glmark2_corpus.rs`,
`tests/fixtures/mali_glmark2_verdicts.txt`).

## Tests

- `shaderc::online::mali::tests::a_keyed_vertex_cube_look_up_reads_the_face_as_an_array_layer`
  (src/shaderc/online/mali/tests.rs) compiles `pan_tex_ref/vcube.vert`
  three ways. With the key it has the face sequence, the shift and the
  conversion, no `MUX`, and a `.2d.array_enable.explicit` look-up of four
  staging registers, and the record has `cube_layers` 1. Without the key, or
  with a key of another binding, it is the plain `.cube.explicit` look-up.
- `shaderc::online::mali::tests::the_keyed_vertex_cube_look_up_is_mesa_s_array_look_up`
  checks that this `TEX_SINGLE` has the mnemonic and operand shapes of
  Mesa's in `vcube_layers.pan_shc.disasm`.
- `mali::frame_tex_tests::a_cube_read_as_six_layers_keeps_its_planes`
  (src/mali/frame_tex_tests.rs) builds the view from a cube. Its texture is
  the cube's with 2D and 6 layers, its 30 planes are the cube's, its ES 2.0
  sampler is the cube's with seamless set, and the frame takes it.
- GL core on the Mali mock, `report-es2`:
  - `tests/mali_cube_samplers_es2.rs`: the keyed states' vertex sampler is
    Mesa's ES 2.0 words with `Seamless Cube Map` set, and every other state
    keeps Mesa's ES 2.0 words.
  - `tests/mali_vs_cube_edge_gl.rs`: the four cases' vertex texture is
    Mesa's with 2D and 6 layers over the same planes, its sampler word 0 is
    Mesa's ES 2.0 word with bit 23 set, and the fragment stage keeps the
    cube.
  - `tests/mali_vcube_magswitch.rs`: the states with no remap form two
    programs, plain and `cube_noseam`, and the remap states carry both
    keys.
- Without `report-es2`, the same files and `tests/mali_cube_samplers.rs` and
  `tests/mali_vcube_lod.rs` check that nothing moved.

## Pin on the board

This has not been run on the board. The board-r373b item reruns the four
cases and the rest of `texture.vertex.cube.*` (the 2D-array view must not
lose a passing CLAMP or MIRROR state). Two points to watch:

- That the v10 unit takes the fp32 layer as Mesa stages it, which Mesa's
  passing 2D-array cases suggest.
- That `texturecubelod`, whose answer to the seamless bit differed from the
  other three cases (section (l)), passes too.
