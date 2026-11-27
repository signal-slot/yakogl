<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D 4.2: the texture size uniform words 44-47 and the RET16_OF_32 split

What the code on this tip does with the four texture size uniform words —
the TMU width, height, depth and layer count a `textureSize` or a 3D
`textureGrad` asks the driver for — and with the return-size rule
`RET16_OF_32`. Words 46/47 landed at `298fe11c` (#1261), the size-only
split at `080240bc`. Read from our tree at `5fd5bf43`; Mesa is 26.2.1.
Line numbers drift; the names are the address.

Sources. `third_party/mesa-26.2.1/` holds the packet XML, the v3d format
table and `src/broadcom/`'s `cle/`, `clif/`, `qpu/`, `common/`, but not the
V3D driver's C: `v3d_uniforms.c` below is a path in the same 26.2.1 tree,
marked "(tree)" (`docs/notes/v3d-shadow.md` "Sources" says where it is).

## (a) What the online compiler emits

`src/shaderc/online/v3d/select.rs:93-99` names the four words with Mesa's
enum values: `QUNIFORM_TEXTURE_WIDTH` 44, `_HEIGHT` 45, `_DEPTH` 46,
`_ARRAY_SIZE` 47. `SOp::TexSizeWord { sampler, lane }` becomes contents
`WIDTH + lane` with `data` the sampler binding (`deferred_loads`,
`select.rs:2126-2132`).

`prepare.rs:1678-1703` (`Op::TexSize`): every size lane loads its word and
minifies it by the shader's level, `max(word >> lod, 1)` (Mesa's
`ntq_minify`). A `sampler3D`'s third lane is the depth word (44 + 2 = 46),
minified like the other two; a `sampler2DArray`'s third lane asks for lane
index 3 — word 47, the layer count — which is not minified ("Don't minify
the array size"). The interpreter run `texture_size_of_a_volume_and_an_array`
(`chain.rs:4602`) pins it: the volume's depth goes 8, 4, 1 at levels
0, 1 and 5, the array's count stays 8.

`textureGrad` emits the same words for another reason (`grad_lod`,
`prepare.rs:970-975`): the TMU has no gradient operand, so each gradient
lane is scaled by the size on its axis, a 3D sampler's third axis being
word 46. Those are the base level's sizes ((e)), so the lod the shader
computes is relative to the base level, as the TMU's Base Level is ((e)):
the base level is added once, by the TMU. A 2D array's layer count is not a gradient axis: word 47
comes only from `textureSize`.

## (b) How the back end fills the table

`struct v3d_job_tmu` (`driver/src/v3d/v3d_job.h:291-299`) is one sampler
binding: the two record addresses plus `width`, `height`, `depth`,
`array_size` — the values the four words read, 0 meaning "not given", such
a word refused (field doc `:283-290`; the word numbers `:276-281`). `texture_units`
(`src/v3d/backend.rs`) fills one entry per binding and per stand-in, for
every stage: the fragment table (`tmu`) and the vertex table (`vs_tmu`),
which a compute dispatch shares; the GL core fills both stages' bindings
from one loop. The four values come from `texture::size_words(b,
b.base_level)` (src/v3d/texture.rs):

- a complete texture: `b.width`, `b.height` and `max(b.layers, 1)`, the GL
  core's level-0 numbers of the stack (`src/gl/draw.rs`; the V3D takes them
  unwrapped, `native_layered_textures`), minified to the binding's base
  level ((e)). `depth` is the 3D texture's depth minified the same way,
  `array_size` the layer count of a `GLES_TEXTYPE_2D_ARRAY`, never
  minified, each 1 for any other type;
- an incomplete texture: 1 / 1 / 1 / 1 with the black stand-in record, GL
  ES 2.0 3.7.10 (`texture_units`' doc, `backend.rs:5955-5969`);
- a stand-in (black) unit — one the stream names with a P0/P1 or size word
  but no binding names — also 1 / 1 / 1 / 1 (the second loop of `texture_units`, `:6020-6070`): GL samples an
  unbound sampler as (0, 0, 0, 1).

## (c) How the job builder writes them

`write_uniforms` (`driver/src/v3d/v3d_job.c:413-519`) switches over the
stream; the width/height case (`:475-487`) and the depth/array case
(`:488-500`) look the binding up through `find_tmu` by the word's `data`
(the binding itself, not shifted like P0/P1's unit) and write the field.
A binding the draw did not give, and a field of 0, set `bad_field`, as does
the `default:` (`:512-514`; the case for 46/47 is `:488-500`) for every word the builder does not write.
`bad_field` rolls both lists back and answers `V3D_JOB_REFUSED`
(`:1090-1096`), which `src/v3d/backend.rs:4004-4006` maps to
`Attempt::Unsupported`, "the job builder refused the draw (a uniform,
program or primitive it does not write)" — GL_INVALID_OPERATION. The Rust
side never fills 0, so that branch is the C check's:
`tools/v3d/job_check.c:897-929` wants the four words over a two-binding
table to come out `9, 1, 1, 5` (binding 1's depth 9, binding 0's layer
count 5: "FS depth/array word") and must answer both "a depth word for a
binding the draw did not give" and "for a binding given without it" REFUSED,
nothing left. The size words have the same pair at `:864-896`.

## (d) RET16_OF_32, split by whether a look-up reads the unit

`texture::refusal_for` (`src/v3d/texture.rs:423-452`) refuses a bound unit
whose format return size (`return_32`, `:582-584`: R32F, RGBA32F, RGBA32UI,
RGBA32I and the depth textures) disagrees with the return size the program
was compiled for (TMU Config P1 bit 0): the 32-bit format read at 16 is
`RET16_OF_32` (`:403-405`, a backend gap, `GAPS` at `:411`), the other way
`RET32_OF_16`.

The split is in `texture_units`. A unit a look-up reads is named by a P0/P1
word, and the rule above stands for it. A unit that only the size words
name (a `textureSize` lowers to no `Op::Tex`, so no P0/P1 word names it)
has no TMU output to agree with, and `texture_units` checks it against its
own format instead (`read`, `backend.rs:5999-6004`), which always agrees: a size-only
sampler over a 32-bit texture draws; `black_typed_state` keeps the same
test for an incomplete texture read by a 32-bit look-up (`black_typed_state`, `:6090-6093`).

Tests, one point each: `tests/es3_tex_words_v3d.rs` (which shapes carry
46/47, the C cases, the refusing `default:`; control
`controls_without_words_46_47_are_not_refused_that_way`),
`tests/es3_texsize3d_v3d.rs` (the stream per shape;
`the_job_builder_has_cases_for_depth_and_array_size`), and
`tests/v3d_tex_array.rs` (`the_job_tmu_carries_the_depth_and_the_layer_count`:
the entry of (b) — (4, 4, 1, 5) for a 4x4x5 array, (4, 4, 5, 1) for a volume).
`tests/es3_shadow_texsize_v3d.rs` is the 2D and cube half of the exemption;
`tests/es3_shadow_array_texsize_v3d.rs` the array half: a size-only
`sampler2DArrayShadow` of a R32F depth array draws in both stages and its
stream carries word 47 (`the_array_size_program_also_carries_word_47`), a
16-bit look-up stays refused
(`a_16_bit_look_up_of_a_32_bit_depth_array_is_still_refused_ret16_of_32`),
and so do the sampled-and-queried and sampling-only units
(`a_sampled_and_queried_array_shadow_unit_of_a_32_bit_depth_array_keeps_ret16_of_32`).
`tests/es3_sampling_r32f_ret16_v3d.rs`: the core keys a sampling look-up
32-bit from the bound format (`stage_lod_key`, `src/gl/draw.rs`), so a
consistent draw never meets the 16-bit key; its predicate-level pin and
RGBA8 control are there too.

## (e) The base level

The words and the TMU record both start at `GL_TEXTURE_BASE_LEVEL`, as
Mesa's do. The GL core hands the effective base level in
`GlesTextureBind::base_level` (`draw::bind_base_level`: an immutable
texture's clamped to its last level, a mutable texture's 0 when the level
is not defined), and completeness and the required levels start there
(`mip_levels_needed`, `docs/notes/gles3-texture-completeness.md`).

- Words 44-46: `size_words` minifies width, height and a 3D depth by that
  level, `max(x >> base, 1)`, as Mesa's `get_texture_size` minifies by the
  sampler view's `u.tex.first_level` (`v3d_uniforms.c:63-72` (tree)).
  Word 47, the layer count, is never minified (`:73-79`). An incomplete
  texture and a stand-in unit keep 1, 1, 1, 1.
- The TMU record: `tex_state_for` builds it with `texture::state_at` at
  `min(base, max_level)`, `max_level` being the last level of the image
  the draw samples, and keeps Max Level at that last level: Mesa's
  `v3d_setup_texture_shader_state` programs Base Level = `first_level`,
  Max Level = `last_level`, both absolute (v3dx_state.c (tree)).
  `TexKey`, the key a job finds a packed record again under, carries the
  base level, so a draw at another base level of the same storage packs
  its own record; a kept tiled copy is keyed without it (`TexKey::image`),
  the texels being the same.
- A `textureLod`, `texelFetch` or `textureGrad` lod is relative to the
  TMU's Base Level, which the shader never adds: the base level is applied
  once.

The limit left: without a mipmap min filter the back end lays out and
copies level 0 only (`texture::levels_sampled`), so the record's base
level clamps to 0 and a non-mipmapped draw at `GL_TEXTURE_BASE_LEVEL` > 0
samples level 0 while its size words are the base level's. GL samples the
base level there. The base-level tests use mipmapped filters.
