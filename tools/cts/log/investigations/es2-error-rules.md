<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# ES 2.0 error rules: what a `report-es2` build still accepts

Static audit for plan #4 item `es2-error-rules-diag`, written 2026-10-09 at
the integration tip `334d4e6`. **Nothing was built or run for this file.** It
is read from `src/gl/` and from the dEQP-GLES2 sources in
`VK-GL-CTS/modules/{gles2,glshared}` (the checkout in `<deps>`).
`es2-host-deqp` measures; `es2-error-rules` fixes. ES 2.0 section numbers
are those of the ES 2.0.25 specification as I know them, not re-read here;
where the repository already cites a rule I say so.

## Result in five lines

1. `report-es2` is one `const`, `crate::REPORT_ES2` (`src/lib.rs:89`), and
   `src/gl/` uses it at about 40 sites. The ES 3.0 surface is **mostly not
   gated**: 8 buffer targets, 9 usages, the ES 3.0 `glEnable`/`glGet*` names,
   `glPixelStorei`, `glBlendEquation` MIN/MAX, vertex types, index type
   `GL_UNSIGNED_INT`, `GL_READ_/DRAW_FRAMEBUFFER`, `GL_DEPTH_STENCIL_ATTACHMENT`,
   `glTexParameter` names and 3D/array texture targets are taken in a
   `report-es2` build (table A, rows 1 to 44).
2. What *is* gated is the part dEQP-GLES2 reaches: `GL_MAJOR_VERSION` and
   friends, `GL_MAX_DRAW_BUFFERS`/`GL_DRAW_BUFFERi`, `GL_DEPTH_COMPONENT24`
   renderbuffers, the sized float/sRGB texture names, integer formats,
   float/half-float unsized types, depth-texture rules, `SRC_ALPHA_SATURATE`
   as a destination factor, `glUniformMatrix*(transpose)`, the attachment
   queries. Table B walks every format dEQP iterates.
3. **No `negative_api` case reaches an ungated value.** The six
   `es2fNegative*ApiTests.cpp` files use these ES 3.0-era tokens and no
   others: `GL_MAJOR_VERSION` (gated), `GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE`
   (inside the ES 3.0 branch that `GL_MAJOR_VERSION` selects away), and the
   ES 2.0 set. Every other bad enum is `-1` or `0`, which is refused in both
   builds. So the ungated rows of table A cannot fail a dEQP-GLES2 case;
   they are conformance debt the CTS does not test.
4. The one dEQP-GLES2 area that does exercise ES 3.0-only *formats* is
   `fbo.completeness.renderable.*` (table B). By the code every row is
   handled, which agrees with the board record: the last full trees have
   `fbo` at 711 of 711 except the one `depth_component24` case, fixed on the
   host in `4e9397c` and not yet run on a board. HANDOFF.md ("Not done:
   ... `GL_DEPTH_COMPONENT32F` ... still accepted") is row 36 here.
5. Fix: one predicate file `src/gl/es2_rules.rs` and a guard
   `if crate::REPORT_ES2 && !es2_rules::x(v)` at each entry point
   (section "Fix proposal"). It is a `const` branch, not a `cfg`, so the ES 3.0
   build compiles to the code it has today. Cost: host tests that use ES 3.0
   enums need the `#[cfg(not(feature = "report-es2"))]` the file already uses.

## How `report-es2` is wired

| Where | What |
|---|---|
| `Cargo.toml:69` | feature `report-es2 = []`, off by default for cargo |
| `src/lib.rs:89` | `pub const REPORT_ES2: bool = cfg!(feature = "report-es2")` |
| CMake `-DYAKOGL_REPORT_ES2`, Kconfig `CONFIG_YAKOGL_REPORT_ES2`, `make REPORT_ES2=1` | **default ON** for the C and Zephyr builds (`src/lib.rs:66-71`, `CMakeLists.txt:85`) |
| `tools/check.sh:58-68, 326, 1202, 1387` | the report-es2 twins of the host test sets |
| `src/gl/state.rs:818-819` | `GL_VERSION`, `GL_SHADING_LANGUAGE_VERSION` |
| `src/gl/state.rs:187, 195` | `NPOT_EXTENSION`, `DEPTH_TEXTURE_EXTENSIONS` = `REPORT_ES2`; they add the last three names of `ALL_EXTENSION_NAMES` (`state.rs:40-180`) |
| `src/shaderc/online/pp.rs:250` and `codegen.rs`, `sema.rs`, `lower.rs`, `backend.rs` | GLSL ES 3.00 sources are refused; tests that compile them are `#[cfg(not(feature = "report-es2"))]` |
| `src/gl/{state,framebuffer,texture,program}.rs` | the API-surface gates audited below |

Two styles exist: `crate::REPORT_ES2` as a runtime-const `if`
(`state.rs:604, 1087, 1125, 1155`, `texture.rs:1799, 1861, 1938, 1962, 2428`,
`framebuffer.rs:507, 575, 1689, 2001, 2123`, `program.rs:2210`) and
`#[cfg(feature = "report-es2")]` on tests and a few items.
The `if` style is the one to continue (see proposal).

### The extensions the ES 2.0 surface advertises (the baseline for "valid")

`GL_EXTENSIONS` is the same list in both builds plus, in `report-es2` only,
`GL_OES_texture_npot`, `GL_OES_depth_texture`, `GL_OES_packed_depth_stencil`
(`state.rs:40-180`): `GL_OES_EGL_image`, `GL_OES_standard_derivatives`,
`GL_OES_compressed_ETC1_RGB8_texture`, `GL_OES_compressed_ETC2_RGB8_texture`,
`GL_OES_compressed_ETC2_RGBA8_texture`, `GL_OES_vertex_array_object`,
`GL_OES_mapbuffer`, `GL_OES_fbo_render_mipmap`,
`GL_EXT_texture_format_BGRA8888`, `GL_OES_rgb8_rgba8`,
`GL_OES_required_internalformat`, `GL_EXT_texture_rg`,
`GL_OES_texture_half_float`, `GL_EXT_color_buffer_half_float`,
`GL_NV_fbo_color_attachments`, `GL_EXT_debug_marker`.

**Not advertised**, so what they add is ES 3.0-only here: `GL_OES_depth24`,
`GL_OES_depth32`, `GL_OES_texture_float`, `GL_EXT_color_buffer_float`,
`GL_EXT_sRGB`, `GL_EXT_sRGB_write_control`, `GL_NV_packed_float`,
`GL_EXT_texture_type_2_10_10_10_REV`, `GL_OES_element_index_uint`,
`GL_EXT_blend_minmax`, `GL_EXT_draw_buffers`, `GL_NV_draw_buffers`,
`GL_EXT_instanced_arrays` / `GL_ANGLE_instanced_arrays`,
`GL_OES_vertex_half_float`, `GL_OES_vertex_type_10_10_10_2`,
`GL_OES_texture_3D`, `GL_EXT_unpack_subimage`, `GL_NV_pack_subimage`,
`GL_EXT_texture_storage`, `GL_OES_get_program_binary`,
`GL_NV_read_buffer`, `GL_NV_framebuffer_blit` / `GL_ANGLE_framebuffer_blit`,
`GL_EXT_map_buffer_range`, `GL_NV_copy_buffer`, `GL_EXT_occlusion_query_boolean`.

(`GL_OES_vertex_array_object` makes `glGenVertexArraysOES` and friends valid;
the unsuffixed `glGenVertexArrays` etc. in `vao.rs:170-196` are plain
forwards. The OES mapbuffer entry points restrict targets to
`ARRAY_BUFFER`/`ELEMENT_ARRAY_BUFFER` already, `buffer.rs:813-853`.)

## What dEQP-GLES2 reaches

Token census of `es2fNegative{Buffer,Fragment,Shader,State,Texture,VertexArray}ApiTests.cpp`
(5,534 lines, 246 cases): the `GL_*` names used are only ES 2.0 core, the
PVRTC/S3TC/palette extension names the texture cases look up, and the two
ES 3.0-era names above. Enum arguments that must fail are `-1`, `0`,
`GL_TRIANGLES` (as a capability, `NegativeStateApi:604`), `GL_FUNC_ADD` (as a
blend factor, `state.rs:2500` test) and `GL_FRAMEBUFFER` (as a
renderbuffer target and as a renderbuffer attachment target, `NegativeBuffer:392`,
`FboApiTest:330`). The ES 3.0 entry points (`glVertexAttribIPointer`,
`glVertexAttribDivisor`, `glDraw*Instanced`, `glBlitFramebuffer`,
`glDrawBuffers`, `glReadBuffer`, `glBindBufferBase`, `glMapBufferRange`,
`glCopyBufferSubData`, `glTexImage3D`, `glGenVertexArrays`) do not appear in
`modules/gles2` at all. `glsVertexArrayTests.cpp:514, 553`
(`glVertexAttribIPointer`) is in the shared ES 3.0 path only.

Cases that can see a difference between ES 2.0 and ES 3.0 behaviour, and what
each expects of a `report-es2` build:

| # | dEQP case(s) | Source | ES 3.0-only value it can hit | Expected | report-es2 today |
|---|---|---|---|---|---|
| D1 | `negative_api.state.get_framebuffer_attachment_parameteriv` | `es2fNegativeStateApiTests.cpp:496-572` (probe `:503`) | `glGetIntegerv(GL_MAJOR_VERSION)`: ES 2.0 must raise `GL_INVALID_ENUM` (that selects the ES 2.0 branch `:531-549`: `OBJECT_NAME` of an empty point is `GL_INVALID_ENUM`; `TEXTURE_LEVEL` of a renderbuffer is `GL_INVALID_ENUM`) | `GL_INVALID_ENUM`, then the ES 2.0 branch | refused, `state.rs:1087-1090`; empty point `framebuffer.rs` about `:1750-1800`. Matches |
| D2 | `negative_api.buffer.renderbuffer_storage` | `es2fNegativeBufferApiTests.cpp:384-430`, `:399` | `GL_RGBA` as renderbuffer format (unsized; not a renderbuffer format in 2.0 or 3.0) | `GL_INVALID_ENUM` | `framebuffer.rs:500-511` drops `GL_RGBA`, `:2018`. Matches |
| D3 | `fbo.completeness.renderable.{renderbuffer,texture}.{color0,depth,stencil}.*` (about 230 + 226 cases in ES 3.0; the ES 2.0 count is the DB size) | `es2fFboCompletenessTests.cpp:56-198, 354`; iteration `glsFboCompletenessTests.cpp:885-915`; verdict `glsFboUtil.cpp:560-612, 454-516`; `glsFboCompletenessTests.cpp:530-585` | every format of the shared DB, valid or not: see table B | an invalid format: error (`INVALID_ENUM` for a renderbuffer; `INVALID_ENUM`/`INVALID_VALUE`/`INVALID_OPERATION` for a texture) **or** `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`; never `GL_FRAMEBUFFER_COMPLETE` | table B: all handled |
| D4 | `fbo.completeness.attachment_combinations.*`, `fbo.api.*` | `glsFboCompletenessTests.cpp:951-970`, `es2fFboApiTest.cpp:279-471` | only ES 2.0 tokens | per the cases | not format-sensitive |
| D5 | `shaders.fragdata.*` | `es2fShaderFragDataTests.cpp:175` | `GL_MAX_DRAW_BUFFERS` must be `GL_INVALID_ENUM` | | refused `state.rs:1125`, `4e9397c` |
| D6 | all `negative_api.*` taking `-1`/`0` | the six files | none | `GL_INVALID_ENUM` etc. | passes by the `_ =>` arm of each match (host harness: `tests/negative_api`, `GL_CORE_NOTES.md` says 222 of 246 pass with `report-es2`; the other 24 wait on the run-time compiler and are unrelated to enum rules) |

## Table A: ES 3.0-only values a `report-es2` build accepts, by entry point

"Today" is what the code does in a `report-es2` build. **accepted** means the
ES 3.0 value is taken; **refused** means gated already. "dEQP" is the
`negative_api` or other GLES2 case that checks it; "none" means the token
census above found no case that passes that value. ES 2.0 rules are the
2.0.25 text as I know it (section numbers unchecked; OES/EXT texts for the
advertised extensions).

### Buffers (`src/gl/buffer.rs`)

| # | Entry point | Value | ES 2.0 rule | Today | dEQP | Expected |
|---|---|---|---|---|---|---|
| 1 | `glBindBuffer`, `glBufferData`, `glBufferSubData`, `glGetBufferParameteriv`, `glGetBufferPointerv`, `glUnmapBuffer`, `glFlushMappedBufferRange`, `glMapBufferRange`, `glCopyBufferSubData` through `binding_for_target` (`buffer.rs:58-70`, used by `:290, 91`) | `GL_PIXEL_PACK_BUFFER`, `GL_PIXEL_UNPACK_BUFFER`, `GL_COPY_READ_BUFFER`, `GL_COPY_WRITE_BUFFER`, `GL_UNIFORM_BUFFER`, `GL_TRANSFORM_FEEDBACK_BUFFER` | 2.9: target is `ARRAY_BUFFER` or `ELEMENT_ARRAY_BUFFER`, else `GL_INVALID_ENUM` | accepted | none (`bind_buffer` `NegativeBuffer:53` and `buffer_data` `:71` use `-1`) | `GL_INVALID_ENUM` |
| 2 | `glBufferData` (`buffer.rs:316-326`, `is_buffer_usage` `:179-194`) | `GL_STREAM_READ`, `GL_STREAM_COPY`, `GL_STATIC_READ`, `GL_STATIC_COPY`, `GL_DYNAMIC_READ`, `GL_DYNAMIC_COPY` | 2.9: `STREAM_DRAW`, `STATIC_DRAW`, `DYNAMIC_DRAW` | accepted | none (`buffer_data` `NegativeBuffer:71-100` uses `-1`) | `GL_INVALID_ENUM` |
| 3 | `glGetBufferParameteriv` (`buffer.rs:549-559`) | `GL_BUFFER_ACCESS_FLAGS`, `GL_BUFFER_MAP_OFFSET`, `GL_BUFFER_MAP_LENGTH` (`GL_BUFFER_MAPPED` and `GL_BUFFER_ACCESS_OES` are `GL_OES_mapbuffer`: valid) | 6.1.? `BUFFER_SIZE`, `BUFFER_USAGE` (+ OES) | accepted | `get_buffer_parameteriv` `NegativeStateApi:470` uses `-1` | `GL_INVALID_ENUM` |
| 4 | `glGetBufferPointerv` (`buffer.rs:611`), `glMapBufferRange` (`:699`), `glUnmapBuffer` (`:881`), `glFlushMappedBufferRange` (`:856`), `glCopyBufferSubData` (`:643`) | the entry points themselves | not in 2.0 (OES variants exist and are restricted, `:813-853`) | exported and working | none | not an enum rule; leave callable, gate only the targets (row 1) |
| 5 | `glDeleteBuffers` (`buffer.rs:459-516`) | resets ES 3.0 bindings | 2.9 | harmless | none | none |

### Vertex arrays (`src/gl/vertex.rs`, `vao.rs`, `draw.rs`)

| # | Entry point | Value | ES 2.0 rule | Today | dEQP | Expected |
|---|---|---|---|---|---|---|
| 6 | `glVertexAttribPointer` (`vertex.rs:41-55`) | `GL_INT`, `GL_UNSIGNED_INT`, `GL_HALF_FLOAT` (0x140B), `GL_INT_2_10_10_10_REV`, `GL_UNSIGNED_INT_2_10_10_10_REV` | 2.8: `BYTE`, `UNSIGNED_BYTE`, `SHORT`, `UNSIGNED_SHORT`, `FIXED`, `FLOAT` | accepted (the packed pair needs size 4 else `GL_INVALID_OPERATION`) | `vertex_attrib_pointer` `NegativeVertexArray:77` uses type `0` | `GL_INVALID_ENUM` |
| 7 | `glVertexAttribIPointer` (`vertex.rs:77-110`) | the entry point | not in 2.0 | exported, working | none | no enum rule; if kept exported, add the row-6 style refusal for `report-es2` only if a C client could resolve it (see "Open questions") |
| 8 | `glVertexAttribDivisor` (`vertex.rs:115`), `glDrawArraysInstanced` (`draw.rs:4246`), `glDrawElementsInstanced` (`:4252`) | the entry points | `GL_EXT_instanced_arrays` not advertised | exported, working | none | as row 7 |
| 9 | `glGetVertexAttribfv/iv` (`vertex.rs:232-253`) | `GL_VERTEX_ATTRIB_ARRAY_INTEGER`, `GL_VERTEX_ATTRIB_ARRAY_DIVISOR` | 6.1.?: the eight 2.0 pnames | accepted | `get_vertex_attribfv/iv` `NegativeStateApi:415, 433` use `-1` | `GL_INVALID_ENUM` |
| 10 | `glGetVertexAttribIiv/Iuiv`, `glVertexAttribI4*` (`vertex.rs:312-380`) | entry points | not in 2.0 | exported | none | as row 7 |
| 11 | `glDrawElements` (`draw.rs:3794`), `glDrawElementsInstanced` (`:4261`), `glDrawRangeElements` (`:4268`) | `GL_UNSIGNED_INT` index type | 2.8: `UNSIGNED_BYTE`, `UNSIGNED_SHORT`; `UNSIGNED_INT` only with `GL_OES_element_index_uint`, not advertised | accepted | `draw_elements` `NegativeVertexArray:209` | `GL_INVALID_ENUM` |
| 12 | `glDrawElements` (`draw.rs:3780-`, `primitive_restart`) | `GL_PRIMITIVE_RESTART_FIXED_INDEX` enabled (row 14) | not in 2.0 | follows row 14 | none | follows row 14 |
| 13 | `glGenVertexArrays`, `glBindVertexArray`, `glDeleteVertexArrays`, `glIsVertexArray` (`vao.rs:170-196`) | unsuffixed names | `GL_OES_vertex_array_object` is advertised; the OES names are the ES 2.0 ones | forward to OES | none | none (kept; no enum) |

### State, capabilities, queries (`src/gl/state.rs`, `program.rs`)

| # | Entry point | Value | ES 2.0 rule | Today | dEQP | Expected |
|---|---|---|---|---|---|---|
| 14 | `glEnable`/`glDisable` (`set_cap` `state.rs:418-440`) | `GL_RASTERIZER_DISCARD` (`:437`), `GL_PRIMITIVE_RESTART_FIXED_INDEX` (`:434`) | 4.1 and 6.1.1 Table 6.?: `BLEND`, `CULL_FACE`, `DEPTH_TEST`, `DITHER`, `POLYGON_OFFSET_FILL`, `SAMPLE_ALPHA_TO_COVERAGE`, `SAMPLE_COVERAGE`, `SCISSOR_TEST`, `STENCIL_TEST` | accepted | `enable`/`disable` `NegativeStateApi:67, 73` use `-1` | `GL_INVALID_ENUM` |
| 15 | `glIsEnabled` (`is_enabled` `state.rs:538-553`) | the same two | as row 14 | accepted | `get_is_enabled` `NegativeStateApi:604` uses `-1`, `GL_TRIANGLES` | `GL_INVALID_ENUM` |
| 16 | `glGetBooleanv/Integerv/Floatv/Integer64v` (`get_integerv` `state.rs:991-992`) | the same two as pnames | 6.1.1: the cap pnames are those of row 14 | accepted | `get_*v` `NegativeStateApi:82-96` use `-1` | `GL_INVALID_ENUM` |
| 17 | `glBlendEquation`, `glBlendEquationSeparate` (`state.rs:638`) | `GL_MIN`, `GL_MAX` | 4.1.8: `FUNC_ADD`, `FUNC_SUBTRACT`, `FUNC_REVERSE_SUBTRACT` (`GL_EXT_blend_minmax` not advertised) | accepted | `blend_equation(_separate)` `NegativeFragment:138, 145` use `-1`; the text lists the three | `GL_INVALID_ENUM` |
| 18 | `glBlendFunc(Separate)` (`valid_blend_factor` `state.rs:586-607`) | `GL_SRC_ALPHA_SATURATE` as a destination | 4.1.8: source only | **refused** (`:604`, host test `:2518`) | `blend_func(_separate)` `NegativeFragment:154, 167` | `GL_INVALID_ENUM` |
| 19 | `glPixelStorei` (`state.rs:499-522`) | `GL_UNPACK_ROW_LENGTH`, `_SKIP_ROWS`, `_SKIP_PIXELS`, `_SKIP_IMAGES`, `_IMAGE_HEIGHT`, `GL_PACK_ROW_LENGTH`, `_SKIP_ROWS`, `_SKIP_PIXELS` | 3.6.1 Table 3.1: `PACK_ALIGNMENT`, `UNPACK_ALIGNMENT` | accepted and honoured | none (the pixel-store cases are in the state-query group, `es2fIntegerStateQueryTests`, which query the two ES 2.0 names) | `GL_INVALID_ENUM` |
| 20 | `get_integerv` ES 3.0 pnames (`state.rs:996-1003, 1031-1042, 1094-1114, 1129-1189`) | `GL_COPY_READ/WRITE_BUFFER_BINDING`, `GL_PIXEL_PACK/UNPACK_BUFFER_BINDING`, `GL_UNIFORM_BUFFER_BINDING`, `GL_SAMPLER_BINDING`, `GL_TRANSFORM_FEEDBACK_BINDING`, the eight pixel-store names (row 19), `GL_MAX_VARYING_COMPONENTS`, `GL_MAX_VERTEX_OUTPUT_COMPONENTS`, `GL_MAX_FRAGMENT_INPUT_COMPONENTS`, `GL_MAX_*_UNIFORM_BLOCKS`, `GL_MAX_COMBINED_UNIFORM_BLOCKS`, `GL_MAX_UNIFORM_BUFFER_BINDINGS`, `GL_MAX_UNIFORM_BLOCK_SIZE`, `GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT`, `GL_MAX_COMBINED_*_UNIFORM_COMPONENTS`, `GL_MAX_SAMPLES`, `GL_NUM_PROGRAM_BINARY_FORMATS`, `GL_PROGRAM_BINARY_FORMATS`, `GL_MAX_ARRAY_TEXTURE_LAYERS`, `GL_MAX_3D_TEXTURE_SIZE`, `GL_MAX_ELEMENT_INDEX`, `GL_MAX_ELEMENTS_VERTICES/INDICES`, `GL_MAX_TEXTURE_LOD_BIAS`, `GL_MAX_TRANSFORM_FEEDBACK_*`, `GL_TRANSFORM_FEEDBACK_ACTIVE/PAUSED`, `GL_MIN/MAX_PROGRAM_TEXEL_OFFSET`, `GL_MAX_SERVER_WAIT_TIMEOUT`, `GL_READ_FRAMEBUFFER_BINDING`, `GL_READ_BUFFER`, `GL_TEXTURE_BINDING_2D_ARRAY`, `GL_TEXTURE_BINDING_3D`, `0x90EB` (ES 3.1) | Table 6.? holds the ES 2.0 set only | accepted (`GL_MAX_DRAW_BUFFERS`, `GL_DRAW_BUFFERi`, `GL_MAJOR/MINOR_VERSION`, `GL_NUM_EXTENSIONS` are **refused**, `:1087, 1125, 1155`) | none; `NegativeStateApi:82-96` use `-1`. The ES 2.0 `state_query` cases query ES 2.0 pnames and are unaffected | `GL_INVALID_ENUM` |
| 21 | `glGetProgramiv` (`program.rs:2672-2677`) | `GL_PROGRAM_BINARY_LENGTH`, `GL_PROGRAM_BINARY_RETRIEVABLE_HINT`, `GL_ACTIVE_UNIFORM_BLOCKS`, `GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH` | 6.1.8: `DELETE_STATUS`, `LINK_STATUS`, `VALIDATE_STATUS`, `INFO_LOG_LENGTH`, `ATTACHED_SHADERS`, `ACTIVE_ATTRIBUTES`, `ACTIVE_ATTRIBUTE_MAX_LENGTH`, `ACTIVE_UNIFORMS`, `ACTIVE_UNIFORM_MAX_LENGTH` | accepted | `get_programiv` `NegativeStateApi:260` uses `-1` | `GL_INVALID_ENUM` |
| 22 | `glGetString` / `glGetStringi` (`state.rs:795-844`) | `GL_VERSION`, `GL_SHADING_LANGUAGE_VERSION` | ES 2.0 strings | **refused** (`:818-819`). `glGetStringi` is an ES 3.0 entry point | `get_string` `NegativeStateApi:103` | `GL_INVALID_ENUM` for `0` |
| 23 | `glHint` (`state.rs:459-476`) | `GL_FRAGMENT_SHADER_DERIVATIVE_HINT` | valid with `GL_OES_standard_derivatives` (advertised) | valid | `hint` `NegativeStateApi:615` | none |

### Textures (`src/gl/texture.rs`)

| # | Entry point | Value | ES 2.0 rule | Today | dEQP | Expected |
|---|---|---|---|---|---|---|
| 24 | `glBindTexture` (`texture.rs:800`), `glGenerateMipmap`, `glTexParameter*`, `glGetTexParameter*` through `binding_for` | `GL_TEXTURE_3D`, `GL_TEXTURE_2D_ARRAY` | 3.7.? targets `TEXTURE_2D`, `TEXTURE_CUBE_MAP` | accepted | `bindtexture_invalid_target` `NegativeTexture:144` uses `0`; `generatemipmap_*` use `-1` | `GL_INVALID_ENUM` |
| 25 | `glTexParameter*` (`tex_parameter` `texture.rs:3486-3536`, `valid_tex_param` `:3455`) | `GL_TEXTURE_WRAP_R`, `_BASE_LEVEL`, `_MAX_LEVEL`, `_MIN_LOD`, `_MAX_LOD`, `_COMPARE_MODE`, `_COMPARE_FUNC`, `_SWIZZLE_R/G/B/A`, and the ES 3.0 enum values `GL_MIRRORED_REPEAT` is 2.0 core: valid | 3.7.4: `MIN_FILTER`, `MAG_FILTER`, `WRAP_S`, `WRAP_T` | **setting accepted** (reading is gated, `:3638-3648`) | `tex_parameter*` `NegativeTexture` cases use `-1` | `GL_INVALID_ENUM` |
| 26 | `glGetTexParameter*` (`texture.rs:3628-3637`) | `GL_TEXTURE_IMMUTABLE_FORMAT`, `GL_TEXTURE_IMMUTABLE_LEVELS` | not in 2.0 | accepted (the rest of the ES 3.0 names are gated by `es3`) | `get_tex_parameterfv/iv` `NegativeStateApi:311, 323` use `-1` | `GL_INVALID_ENUM` |
| 27 | `glTexImage2D`, `glTexSubImage2D`, `glCopyTex*` pixel `type` (`is_pixel_type_enum` `texture.rs:440-461`, `check_format_type` `:1806`) | `GL_BYTE`, `GL_SHORT`, `GL_INT`, `GL_UNSIGNED_INT` (non-depth), `GL_HALF_FLOAT` (0x140B), `GL_UNSIGNED_INT_2_10_10_10_REV`, `GL_UNSIGNED_INT_10F_11F_11F_REV`, `GL_UNSIGNED_INT_5_9_9_9_REV`, `GL_FLOAT_32_UNSIGNED_INT_24_8_REV` | 3.7.1 Table 3.2/3.3: an unknown type is `GL_INVALID_ENUM`; `GL_FLOAT` needs `GL_OES_texture_float` | known enum, so the *wrong error*: `GL_INVALID_OPERATION` (`valid_format_type` `:1684-1693` has no pair), and for `GL_FLOAT`/`GL_HALF_FLOAT` `:1962` also `GL_INVALID_OPERATION` | `teximage2d_invalid_format` `NegativeTexture:1470-1482` uses `0`; `:1735, 1737` use ES 2.0 packed types | `GL_INVALID_ENUM` (the fbo verifier accepts any of the three for textures, `glsFboUtil.cpp:588-596`) |
| 28 | `glTexImage2D` (`texture.rs:1938-1946`) sized internal formats | `GL_SRGB8`, `GL_SRGB8_ALPHA8`, `GL_RGBA16F`, `GL_RGBA32F`, `GL_R16F`, `GL_R32F`, `GL_RG16F`, `GL_RG32F` | needs `GL_EXT_sRGB`, `GL_OES_texture_float`, `GL_OES_texture_half_float` (the last is advertised, but gives *unsized* RGBA/RGB + `HALF_FLOAT_OES`) | **refused** `GL_INVALID_VALUE` | `fbo.completeness.renderable.texture.*` (D3) | error or incomplete |
| 29 | the same call, other ES 3.0 sized names | `GL_RGB16F`, `GL_RGB32F`, `GL_R11F_G11F_B10F`, `GL_RGB10_A2`, `GL_RGB10`, `GL_RGB9_E5`, the integer sized formats, `GL_STENCIL_INDEX8` | not names in 2.0 | **refused** (`sized_format` returns `None`, `:1947`, `:4388-4436`) | D3 | |
| 30 | `glTexImage2D` format `*_INTEGER` | `GL_RED_INTEGER` etc. | not names | **refused** `GL_INVALID_ENUM` (`es2_integer_format` `:1798`, used `:1907, 2276`) | | |
| 31 | `glTexImage2D` depth names (`depth_image_error` `texture.rs:1855-1879`) | `GL_DEPTH_COMPONENT24`, `GL_DEPTH_COMPONENT32F`, `GL_DEPTH32F_STENCIL8`; `GL_FLOAT` depth; non-2D target | `GL_OES_depth_texture`/`packed_depth_stencil` rules | **refused** `GL_INVALID_VALUE`/`GL_INVALID_OPERATION` | D3 depth rows | |
| 32 | `glCopyTexImage2D` (`texture.rs:2417-2433`) | depth/stencil internal formats | 3.7.2: `GL_INVALID_ENUM` | **refused** `GL_INVALID_ENUM` (ES 3.0: `INVALID_OPERATION`) | `copyteximage2d_*` `NegativeTexture:793-1106` | `GL_INVALID_ENUM` |
| 33 | `glTexImage3D`, `glTexSubImage3D`, `glCompressedTexImage3D`, `glCopyTexSubImage3D`, `glTexStorage2D/3D` (`texture.rs:3850-4558`) | the entry points | not in 2.0 | exported | none | no enum rule |

### Framebuffers and renderbuffers (`src/gl/framebuffer.rs`)

| # | Entry point | Value | ES 2.0 rule | Today | dEQP | Expected |
|---|---|---|---|---|---|---|
| 34 | `glBindFramebuffer` (`framebuffer.rs:1244-1286`), `glCheckFramebufferStatus` (`:1515`), `glFramebufferTexture2D` (`:1343`), `glFramebufferRenderbuffer` (`:1485`), `glFramebufferTextureLayer` (`:1423`), `glGetFramebufferAttachmentParameteriv` (`:1685`) through `is_fb_target` (`:283`) | `GL_READ_FRAMEBUFFER`, `GL_DRAW_FRAMEBUFFER` | 4.4.1, 4.4.3, 4.4.5: target is `GL_FRAMEBUFFER` | accepted | `bind_framebuffer` `NegativeBuffer:225` and `check_framebuffer_status` `:241` use `-1` | `GL_INVALID_ENUM` |
| 35 | `glFramebufferTexture2D`, `glFramebufferRenderbuffer`, `glGetFramebufferAttachmentParameteriv` (`is_attachment_point` `framebuffer.rs:275-281`, `:1731`) | `GL_DEPTH_STENCIL_ATTACHMENT` (0x821A) | 4.4.3: `COLOR_ATTACHMENT0`, `DEPTH_ATTACHMENT`, `STENCIL_ATTACHMENT` (+ `COLOR_ATTACHMENTi_NV`, advertised) | accepted: sets depth and stencil together (`set_attachment` `:290`) | `framebuffer_renderbuffer` `NegativeBuffer:274`, `framebuffer_texture2d` `:315` use `-1` | `GL_INVALID_ENUM` |
| 36 | `glRenderbufferStorage` (`framebuffer.rs:2018`, `is_renderable_format` `:1990`, `is_depth_format` `:533`, `is_color_format` `:466`) | `GL_DEPTH_COMPONENT32F`, `GL_DEPTH32F_STENCIL8`, `GL_R32F` | 4.4.2: `RGBA4`, `RGB5_A1`, `RGB565`, `DEPTH_COMPONENT16`, `STENCIL_INDEX8` (+ advertised: `DEPTH24_STENCIL8_OES`, `RGB8_OES`, `RGBA8_OES`, `R8_EXT`, `RG8_EXT`, `RGBA16F_EXT` family, `BGRA8_EXT`) | accepted. This is the HANDOFF "not done" item | not in the DB of D3 (`glsFboCompletenessTests.cpp:56-122` has `DEPTH_COMPONENT32`, not `32F`), so no case iterates them | `GL_INVALID_ENUM` |
| 37 | the same call | `GL_DEPTH_COMPONENT24` | `GL_OES_depth24`, not advertised | **refused** (`renderbuffer_format_advertised` `:2000`, `4e9397c`) | D3 `renderbuffer.depth.depth_component24` | `GL_INVALID_ENUM` |
| 38 | the same call | `GL_SRGB8_ALPHA8` | `GL_EXT_sRGB`, not advertised | **refused** (`:507`) | D3 | `GL_INVALID_ENUM` |
| 39 | the same call | `GL_RGB10_A2`, `GL_RGB10`, `GL_SRGB8`, `GL_R11F_G11F_B10F`, `GL_DEPTH_COMPONENT32`, `GL_RGBA32F`, `GL_RGB16F`... integer formats | not names / not advertised | **refused** (not in `is_color_format`/`is_depth_format`) | D3 | `GL_INVALID_ENUM` |
| 40 | `glGetRenderbufferParameteriv` (`framebuffer.rs:2089-2145`) | `GL_RENDERBUFFER_SAMPLES` | not in 2.0 | **refused** (`:2123`) | `get_renderbuffer_parameteriv` `NegativeStateApi:574` | `GL_INVALID_ENUM` |
| 41 | `glGetFramebufferAttachmentParameteriv` (`framebuffer.rs:1661-1850`) | `GL_FRAMEBUFFER_ATTACHMENT_{COMPONENT_TYPE,COLOR_ENCODING,*_SIZE,TEXTURE_LAYER}`, default-framebuffer queries, empty-point `OBJECT_NAME` | 4.4.5 / 6.1.? | **refused** via `es3 = !REPORT_ES2` (`:1689, 1701`, and the `es3` arms through about `:1840`) | D1, `fbo.api.attachment_query_*` (`FboApiTest:362-402, 455-471`) | per D1 |
| 42 | `glReadPixels` (`framebuffer.rs:2151-`, `read_pixels` `:2278`) | `GL_FLOAT` type from a float target | 4.3.1: only the implementation-chosen pair and `RGBA/UNSIGNED_BYTE` | accepted for a float colour attachment (`:2291`) | `read_pixels(_format_mismatch)` `NegativeBuffer:161, 189` use ES 2.0 types | `GL_INVALID_OPERATION` is correct for known-but-wrong pairs; an ES 3.0-only type enum should be `GL_INVALID_ENUM` (same as row 27) |
| 43 | `glDrawBuffers`, `glReadBuffer`, `glBlitFramebuffer`, `glInvalidate*`, `glRenderbufferStorageMultisample`, `glGetInternalformativ` (`framebuffer.rs:2589-2874`) | entry points | not in 2.0 (draw buffers: `GL_EXT_draw_buffers`, not claimed) | exported, ES 3.0 rules (`GL_CORE_NOTES.md` "Draw buffers") | none | no enum rule |
| 44 | ES 3.0 objects: queries, transform feedback, samplers, sync, `glBindBufferBase/Range`, `glClearBuffer*`, `glProgramBinary` (`src/gl/es3.rs`) | entry points | not in 2.0 | exported | none | no enum rule |

## Table B: the formats dEQP-GLES2 iterates, against the advertised set

The DB is `s_esExtFormats` (`glsFboCompletenessTests.cpp:207-296`) plus
`s_es2Formats` and `s_es2ExtFormats` (`es2fFboCompletenessTests.cpp:43-198`).
`ctxFormats` = core + the entries whose extension string the context lists;
`allFormats` = everything. Each case builds an FBO on one `allFormats` entry
at one attachment point; the verdict is `glsFboUtil.cpp:560-612` (errors) and
`:466-516` (status). A format missing from `ctxFormats` must be refused
with an error, **or** make the FBO `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`;
`GL_FRAMEBUFFER_COMPLETE` fails. The `DEQP_gles3_core_compatible` entries
are off in a context whose `GL_MAJOR_VERSION` query fails
(`glsFboUtil.cpp:166-183`; row D1), which is why that query has to stay
refused.

| Format (DB entry) | Valid in ctx? | yakogl `report-es2`, renderbuffer | yakogl `report-es2`, texture | OK? |
|---|---|---|---|---|
| `RGBA4`, `RGB5_A1`, `RGB565`, `DEPTH_COMPONENT16`, `STENCIL_INDEX8` | core | taken | taken (`GL_OES_required_internalformat`) | yes |
| `RGBA8`, `RGB8` | `GL_OES_rgb8_rgba8` (listed) | taken | taken | yes |
| `R8`, `RG8`, `(RED|RG, UNSIGNED_BYTE)` | `GL_EXT_texture_rg` (listed) | taken | taken | yes |
| `RGBA16F`, `RGB16F`, `RG16F`, `R16F` (RBO) | half-float pair (listed) | taken | **refused** `INVALID_VALUE` (`texture.rs:1941`); not texture-valid in the DB either | yes |
| `(RGBA|RGB, HALF_FLOAT_OES)`, `(RED|RG, HALF_FLOAT_OES)` | listed | `GL_RGBA` etc. refused as RBO | taken | yes |
| `BGRA`, `BGRA8_EXT` | `GL_EXT_texture_format_BGRA8888` (listed) | taken | taken | yes |
| `DEPTH24_STENCIL8`, `(DEPTH_STENCIL, UNSIGNED_INT_24_8)`, `(DEPTH_COMPONENT, UNSIGNED_SHORT|UNSIGNED_INT)` | depth-texture pair (listed in `report-es2`) | taken | taken | yes |
| `DEPTH_COMPONENT24` | `GL_OES_depth24`, **not** listed | **refused** `INVALID_ENUM` | **refused** `INVALID_VALUE` (`texture.rs:1861`) | yes (since `4e9397c`, not board-run) |
| `DEPTH_COMPONENT32` (0x81A7) | `GL_OES_depth32`, not listed | refused (`is_depth_format`) | refused (`sized_format` `None`) | yes |
| `SRGB8_ALPHA8` | `GL_EXT_sRGB`/`write_control`, not listed | refused (`:507`) | refused `INVALID_VALUE` | yes |
| `SRGB8`, `(SRGB, UNSIGNED_BYTE)`, `(SRGB_ALPHA, UNSIGNED_BYTE)` | `GL_NV_sRGB_formats`/`GL_EXT_sRGB`, not listed | refused | `SRGB8`: `INVALID_VALUE`; `SRGB`/`SRGB_ALPHA` formats are not pixel-format enums: `INVALID_ENUM` | yes |
| `R11F_G11F_B10F`, `(RGB, UNSIGNED_INT_10F_11F_11F_REV)` | `GL_NV_packed_float`, not listed | refused | `INVALID_VALUE` / `INVALID_OPERATION` | yes |
| `RGB10_A2`, `RGB10`, `(RGBA|RGB, UNSIGNED_INT_2_10_10_10_REV)` | `GL_EXT_texture_type_2_10_10_10_REV`, not listed | refused | `INVALID_VALUE` / `INVALID_OPERATION` | yes |
| `(RGBA|RGB|RED|RG, FLOAT)` | `GL_OES_texture_float`, not listed | n/a | `INVALID_OPERATION` (`texture.rs:1962`) | yes |
| `SR8_EXT`, `SRG8_EXT` | not listed | refused | `INVALID_VALUE` | yes |

Every row is "yes" by the code. A format accepted but *not* in the DB cannot
fail a case, because the case is only built from DB entries: table A row 36
(`DEPTH_COMPONENT32F`, `DEPTH32F_STENCIL8`, `R32F` renderbuffers) is wrong by
the spec and invisible to dEQP-GLES2.

## Cases that name ES 3.0-only values: summary

| Value | Cases | Expected | `report-es2` |
|---|---|---|---|
| `GL_MAJOR_VERSION` (and `GL_MINOR_VERSION`, probed by `glsFboUtil.cpp:174-183`) | D1, D3 (all rows through `DEQP_gles3_core_compatible`) | `GL_INVALID_ENUM` | refused |
| `GL_MAX_DRAW_BUFFERS`, `GL_DRAW_BUFFERi` | D5 `shaders.fragdata.*` | `GL_INVALID_ENUM` | refused |
| `GL_FRAMEBUFFER_ATTACHMENT_COMPONENT_TYPE` | D1 (only after `GL_MAJOR_VERSION` succeeds) | not reached | n/a |
| the formats of table B | D3 | error or incomplete | handled |
| everything in table A not marked refused | none | | accepted |

## Fix proposal (for `es2-error-rules`)

**Scope decision first.** The ungated rows cannot fail a dEQP-GLES2 case, so
the reason to fix them is spec conformance, and the plan's zero-failure goal
does not depend on it. Two things do: `es2-host-deqp` must confirm D3 with a
real run (this file predicts all-pass for the DB), and the HANDOFF "not done"
item (row 36) is cheap. Suggested order: rows 36, 14-16, 1-2, 34-35, 6, 11,
17, 19, 20-21, 24-27; leave 4, 7, 8, 10, 33, 43, 44 (entry points, no enum).

**Files and functions.**

1. New `src/gl/es2_rules.rs` (declare in `src/gl/mod.rs`). Pure `const fn`
   predicates returning "is this value in ES 2.0 plus the advertised
   extensions", each with a doc line naming the spec section, so one file
   lists the ES 2.0 surface and the advertised-extension baseline above is
   stated once:
   `buffer_target`, `buffer_usage`, `buffer_param`, `vertex_attrib_type`,
   `attrib_query_pname`, `index_type`, `capability`, `pixel_store_pname`,
   `blend_equation_mode`, `get_pname`, `program_pname`, `texture_target`,
   `tex_parameter_pname`, `pixel_type`, `framebuffer_target`,
   `attachment_point`, `renderbuffer_format`.
   `renderbuffer_format` generalises `renderbuffer_format_advertised`
   (`framebuffer.rs:2000`) and reads the extension list through
   `state::extension_listed` as that function does, so a GPU that later lists
   `GL_OES_depth24` or `GL_EXT_color_buffer_float` needs no code change.
2. Guard at each entry point, in the existing style:
   `if crate::REPORT_ES2 && !es2_rules::buffer_target(target) { c.set_error(GL_INVALID_ENUM, FN); return; }`.
   Sites: `buffer.rs` `binding_for_target` callers (`glBindBuffer` `:290`,
   `bound_buffer_for_target` `:90`), `glBufferData` (`:323`);
   `vertex.rs:41` and `get_attrib_int` (`:232`); `draw.rs:3794, 4261`;
   `state.rs` `set_cap` (`:418`), `is_enabled` (`:538`, so `glIsEnabled` and
   the `get_integerv` arm at `:991`), `glBlendEquationSeparate` (`:638`),
   `glPixelStorei` (`:499`), `get_integerv` (extend the `:1087` and `:1125`
   pattern, one arm listing rows 20's names), `program.rs:2672-2677`;
   `texture.rs` `glBindTexture` (`:800`), `binding_for` (`glGenerateMipmap`,
   `tex_parameter`, `get_tex_parameter`), `valid_tex_param`/`tex_parameter`
   (`:3455, 3486`), `get_tex_parameter` immutable names (`:3628`),
   `check_format_type` (`:1806`, so an ES 3.0-only type is `INVALID_ENUM`
   rather than `INVALID_OPERATION`); `framebuffer.rs` `is_fb_target` (`:283`),
   `is_attachment_point` (`:275`), `renderbuffer_storage_args_ok` (`:2018`).
   `is_fb_target` and `is_attachment_point` are called with the ES 3.0 values
   from inside the file by `as_read` (`framebuffer.rs:1303`), which re-enters
   an entry point with `GL_FRAMEBUFFER`; those internal calls stay valid, but a
   `GL_READ_FRAMEBUFFER` target has to be refused *before* `as_read` runs
   (`:1329, 1412, 1471, 1517, 1669`).
3. Do **not** gate on `#[cfg(feature = "report-es2")]` in product code: the
   `if crate::REPORT_ES2 && ...` form is a `const` branch that the optimiser
   removes in the ES 3.0 build, keeps one source for both, and lets the
   predicates be unit-tested in both builds (`es2_rules::tests` asserts the
   ES 2.0 set once). `cfg` stays for tests.
4. Tests: one host test per row group asserting
   `if REPORT_ES2 { GL_INVALID_ENUM } else { GL_NO_ERROR }` (the shape of
   `state.rs:2524`), naming the dEQP case where one exists. Existing tests
   that call ES 3.0 enums through the public entry points will fail under
   `report-es2` and need `#[cfg(not(feature = "report-es2"))]`: find them
   with `cargo test --no-default-features --features host-test,gpu-powervr,report-es2`
   (`tools/check.sh:58`) after the first guard lands; expect them in
   `buffer.rs`, `es3.rs`, `state.rs` (the `:2985-3030` uniform-block asserts
   already call `geti1(GL_MAX_VERTEX_UNIFORM_BLOCKS)` unconditionally),
   `texture.rs` and `framebuffer.rs`.

**Gating on `report-es2` so the ES 3.0 builds keep working.** Every guard is
`crate::REPORT_ES2 && ...`; with the feature off, `REPORT_ES2` is the
constant `false` and the guard disappears. Nothing in the ES 3.0 path moves.

**Risks to check before merging.**

- *C/Zephyr builds default to `report-es2` ON* (`CMakeLists.txt:85`). Anything
  in `driver/` that calls an ES 3.0 value through a default build changes
  behaviour: `driver/tools/pvrlab/gl_test.c` has 38 lines using
  `GL_UNIFORM_BUFFER`, `glBindBufferBase`, `GL_RASTERIZER_DISCARD`,
  `GL_READ/DRAW_FRAMEBUFFER`, `glDrawBuffers`, `glVertexAttribDivisor`,
  `GL_UNSIGNED_INT`, `GL_COPY_READ_BUFFER`, `GL_TEXTURE_3D`,
  `GL_UNPACK_ROW_LENGTH`. Its ES 3.0 cases already SKIP on an ES 2.0 library
  (header comment, lines 365, 381, 609); read each case's skip test before
  gating, or a case that used to run will start failing instead of skipping.
  Board gl_test runs are part of `am62p-es2-r1`, `imx8mp-gltest-r2`.
- The extension list is the oracle: any value an advertised extension adds
  must stay (row 3's `GL_BUFFER_MAPPED`, row 23, the half-float and `_NV`
  colour attachments, `GL_OES_vertex_array_object`).
- `glGetIntegerv` returns 0 values and writes nothing on an unknown pname
  (`state.rs:1205`); a new refusal must keep that.

**Open questions (assumed, not decided).** Should ES 3.0-only *entry points*
return an error in a `report-es2` build? I assumed no: a real ES 2.0 library
does not export them, a client never resolves them, and an unconditional
`GL_INVALID_OPERATION` from `glDrawArraysInstanced` would hide behaviour
Qt-on-ES2 never asks for. Rows 4, 7, 8, 10, 33, 43, 44 are therefore not
proposed.

## Handoff notes for the other items

- `es2-host-deqp`: run `fbo.completeness.renderable.*`,
  `fbo.completeness.attachment_combinations.*`, `fbo.api.*`,
  `negative_api.*` and `state_query.*` on the `report-es2` host build; this
  file predicts zero failures there. A failure in D3 names a row of table B
  that the code read got wrong. Compare against table A only by adding
  probe calls: no dEQP case reaches those values.
- `es2-error-rules`: implement the proposal above; each fix is invisible to
  dEQP-GLES2, so prove it with host tests, not with a CTS delta.
- Not verified by this audit: the ES 2.0.25 section numbers in the "rule"
  columns marked `?`; the runtime result of any row (nothing was run).

## Status

Item `es2-error-rules` (vertex part). Pinned by `tests/es2_error_rules_vertex.rs`,
whose header has the per-row reasons.

| Rows | State | File |
|---|---|---|
| 6 | done: `GL_INT` and `GL_UNSIGNED_INT` are now refused under `report-es2` (the other four types already were). `GL_HALF_FLOAT_OES` stays refused: `GL_OES_vertex_half_float` is not advertised and the draw path cannot convert it | `src/gl/vertex.rs` |
| 9 | done at the base commit (`get_attrib_int`); now covered by a test | `src/gl/vertex.rs` |
| 7, 8, 10 | decided: no change (entry points, no enum; see "Open questions") | - |
| 13 | decided: no change (`GL_OES_vertex_array_object` is advertised) | `src/gl/vao.rs` |

Still open, no item yet (hardening; host `negative_api` is already clean):

- buffer rows (1-5): `src/gl/buffer.rs`
- framebuffer rows: `src/gl/framebuffer.rs`
- state rows (14-22 and the state-query rows): `src/gl/state.rs`, `src/gl/program.rs` (row 21)
- draw rows (8's draw calls, 11, 12): `src/gl/draw.rs`
- texture rows: `src/gl/texture.rs`
- the shared `src/gl/es2_rules.rs` of the proposal, if wanted once more than one file needs the same predicate
