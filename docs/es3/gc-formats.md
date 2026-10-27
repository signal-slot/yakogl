<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: the texture descriptor of every storage class (formats-gc)

Item formats-gc of plan #19 (ES 3.0). For each `GLES_TEXFMT_*` storage class
of `src/backend.rs`: the texture-engine (TE) format the GC7000UL's texture
descriptor names it with, the swizzle, whether this core samples it and from
which HALTI level, how an integer sampler reads it, sRGB and border handling,
and what Mesa's etnaviv does where the hardware has no direct format. The
code is `src/vivante/texture.rs` (`gc_texfmt`, `format_words`) and
`src/shaderc/online/vivante/int_tex.rs`; the test is `tests/es3_formats_gc.rs`,
which reads section 2's table out of this file and compares every row with the
descriptor words the back end builds and with the rnndb header's values.

## 1. Sources and the core

* **Mesa 26.2.1**, tag `mesa-26.2.1`, commit
  `da14d65e4499e66468094be52bff9ea0915a695e` (the tarball
  `third_party/mesa-26.2.1/UPSTREAM` names; the etnaviv driver is not
  vendored, it was read from the same release's source tree in the deps
  cache). Paths below are relative to its `src/`; `F` is
  `gallium/drivers/etnaviv/etnaviv_format.c`, `D` is
  `gallium/drivers/etnaviv/etnaviv_texture_desc.c`, `S` is
  `gallium/drivers/etnaviv/etnaviv_screen.c`, `X` is
  `etnaviv/hw/common_3d.xml.h`, `T` is `etnaviv/hw/texdesc_3d.xml.h`.
* The values themselves are the rnndb header the back end is written from,
  `driver/src/vivante/hw/common_3d.xml.h` (identical to `X` but for the added
  SPDX first line, so its line numbers are `X`'s plus one).
* **The core**: GC7000UL, model 0x7000 revision 0x6204 (i.MX8MP), the hwdb row
  `etnaviv/hwdb/nxp/gc_feature_database.h:18841`, with **HALTI0 to HALTI5**
  all set (`docs/es3/backend-caps.md` §1.2). Every HALTI gate of
  `gpu_supports_texture_format` (`S:394-449`) is therefore met: sRGB and the
  extended (`EXT_FORMAT`) formats need HALTI0 (`S:406-410`), snorm HALTI1
  (`S:416-417`), pure integer and float HALTI2 (`S:419-421`), the 128-bit
  integer formats HALTI5 and the 128-bit float ones HALF_FLOAT
  (`S:423-429`). This core takes the descriptor path (halti >= 5,
  `D:139-207`), not the TE_SAMPLER registers (`etnaviv_texture_state.c`).

How etnaviv writes the format: `translate_texture_format` (`F:275-294`)
returns the table's `.tex` (`F:86-273`); a value with `EXT_FORMAT` goes into
`TEXDESC_CONFIG1.FORMAT_EXT` with `CONFIG0.FORMAT` 0, any other into
`CONFIG0.FORMAT` (`D:179-186`). The swizzle is the view's, in
`CONFIG1.SWIZZLE_*` (`get_texture_swiz`, `F:346-384`): identity for every
format of ours (the gallium view of a GL texture is identity; only R8_UNORM,
emulated with L8, and DXT1_RGB patch it). A pure signed-integer format with
8- or 16-bit channels also sets `CONFIG2.SIGNED_INT8` / `SIGNED_INT16`
(`D:187-189`; `X:165-166`). `SAMP_CTRL0.INT_FILTER` is set for a unorm,
non-sRGB format that is not R10G10B10A2 or ETC2 R11/RG11, outside 3D, without
anisotropy (`texture_use_int_filter`, `F:296-338`). sRGB is
`SAMP_CTRL1.SRGB` on the linear twin's format (`D:246-247`; `F:278`).

## 2. The table

Columns: the storage class; the TE format by its rnndb name (`TEXTURE_FORMAT_*`
in CONFIG0, `TEXTURE_FORMAT_EXT_*` in CONFIG1, `none` where the descriptor
decodes nothing); the swizzle the descriptor writes for a texture that has
alpha (four of `R G B A 0 1`); CONFIG2's sign bit; INT_FILTER; whether the
GC7000UL samples the format and the HALTI gate (`S:`); whether the class is
in `GC_SAMPLED_TEXFMTS` (`src/backend.rs`, the mask the GL core stores
natively by; the linear ETC block classes are not in any back end's mask, the GL
core's compressed upload picks them, and the sRGB ones are in the PowerVR's alone); the integer sampler; sRGB and border; Mesa's fallback; Mesa's
row.

Every row: no border. ES 3.0 has no `CLAMP_TO_BORDER` (that is ES 3.2 /
EXT_texture_border_clamp); etnaviv maps it to CLAMP_TO_EDGE ("XXX",
`gallium/drivers/etnaviv/etnaviv_translate.h:173-174`) and never writes the
descriptor's border words (`T:170,194-200`; no reference in `D`). The GL
core's `no_alpha` (a GL_RGB / GL_LUMINANCE base) turns the alpha swizzle into
`1`, and `depth_luminance` the green and blue into `R`, for every class
(`texture.rs` `format_words`); the column is the swizzle without them.

| class | TE format | swizzle | CONFIG2 | INT_FILTER | GC7000UL samples it | in mask | integer sampler | sRGB, border | Mesa's fallback | Mesa |
|---|---|---|---|---|---|---|---|---|---|---|
| `GLES_TEXFMT_RGBA8` | `TEXTURE_FORMAT_A8B8G8R8` | RGBA | - | yes | yes (no gate) | yes | - | linear; none | - (R,G,B,A bytes: A8B8G8R8; a texture the PE wrote, B,G,R,A, is A8R8G8B8, which is Mesa's own choice for its PE-order resources, `F:155,417-425`) | `F:155,164`; `X:52` |
| `GLES_TEXFMT_ETC2_RGB` | `TEXTURE_FORMAT_ETC1` | RGB1 | - | yes | yes (HALTI0) | no (block) | - | linear; none | Mesa samples ETC2_RGB8 as `TEXTURE_FORMAT_EXT_NONE` with `EXT_FORMAT` ("Extd. format NONE doubles as ETC2_RGB8"), ETC1 as `TEXTURE_FORMAT_ETC1`; ours decodes ETC1 for both (section 4, item 3) | `F:237,244`; `X:65-66` |
| `GLES_TEXFMT_ETC2_RGBA` | `TEXTURE_FORMAT_EXT_RGBA8_ETC2_EAC` | RGBA | - | yes | yes (HALTI0) | no (block) | - | linear; none | - | `F:246`; `X:68` |
| `GLES_TEXFMT_ETC2_RGB_PT` | `TEXTURE_FORMAT_EXT_RGB8_PUNCHTHROUGH_ALPHA1_ETC2` | RGBA | - | yes | yes (HALTI0) | no (block) | - | linear; none | - | `F:245`; `X:67` |
| `GLES_TEXFMT_ETC2_SRGB_RGB` | none | RGB1 | - | no | not here (only the PowerVR's mask has the compressed sRGB classes, `docs/es3/srgb.md` 4.3) | no | - | sRGB; none | - (etnaviv samples it as the ETC2 block format with `SAMP_CTRL1.SRGB`; not built) | `F:237`; `X:65-66` |
| `GLES_TEXFMT_ETC2_SRGB_RGBA` | none | RGBA | - | no | not here (as the row above) | no | - | sRGB; none | - (as the row above) | `F:246`; `X:68` |
| `GLES_TEXFMT_RGB565` | `TEXTURE_FORMAT_R5G6B5` | RGBA | - | yes | yes (no gate) | yes | - | linear; none | - (B5G6R5_UNORM, red in the high bits as GL's 5_6_5) | `F:114`; `X:54` |
| `GLES_TEXFMT_RGBA16F` | `TEXTURE_FORMAT_EXT_A16B16G16R16F` | RGBA | - | no | yes (HALTI2) | yes | - | linear; none | - | `F:205`; `S:419-421`; `X:75` |
| `GLES_TEXFMT_R16F` | `TEXTURE_FORMAT_EXT_R16F` | RGBA | - | no | yes (HALTI2) | yes | - | linear; none | - (the TE fills G, B with 0 and A with 1) | `F:106`; `X:73` |
| `GLES_TEXFMT_R32F` | `TEXTURE_FORMAT_EXT_R32F` | RGBA | - | no | yes (HALTI2) | yes | - | linear; none | - | `F:140`; `X:76` |
| `GLES_TEXFMT_L8` | `TEXTURE_FORMAT_L8` | RGBA | - | yes | yes (no gate) | yes | - | linear; none | - (L8_UNORM; Mesa's R8_UNORM is L8 with G, B swizzled to 0, `F:364-370`) | `F:96,88`; `X:45` |
| `GLES_TEXFMT_A8` | `TEXTURE_FORMAT_A8` | RGBA | - | yes | yes (no gate) | yes | - | linear; none | - | `F:95`; `X:44` |
| `GLES_TEXFMT_LA8` | `TEXTURE_FORMAT_A8L8` | RGBA | - | yes | yes (no gate) | yes | - | linear; none | - | `F:111`; `X:47` |
| `GLES_TEXFMT_R11G11B10F` | `TEXTURE_FORMAT_EXT_B10G11R11F` | RGBA | - | no | yes (HALTI2: float) | yes | - | linear; none | - (R in the low 11 bits, GL's 10F_11F_11F_REV word) | `F:187`; `S:419-421`; `X:91` |
| `GLES_TEXFMT_RGB9E5` | `TEXTURE_FORMAT_E5B9G9R9` | RGBA | - | no | yes (HALTI2: float) | yes | - | linear; none | - (a CONFIG0 format, not an extended one) | `F:186`; `S:419-421`; `X:64` |
| `GLES_TEXFMT_RGB10A2` | `TEXTURE_FORMAT_EXT_A2B10G10R10` | RGBA | - | no | yes (HALTI0) | no | - | linear; none | - (no INT_FILTER by name, `F:330-331`) | `F:167`; `S:409-410`; `X:78` |
| `GLES_TEXFMT_RGB10A2UI` | `TEXTURE_FORMAT_EXT_A2B10G10R10UI` | RGBA | - | no | yes (HALTI2: integer) | yes | `usampler*`: the raw 10/10/10/2-bit integers | linear; none | - | `F:170`; `S:419-421`; `X:92` |
| `GLES_TEXFMT_RGBA8_SNORM` | `TEXTURE_FORMAT_EXT_A8B8G8R8_SNORM` | RGBA | - | no | yes (HALTI1: snorm) | yes | - | linear; none | - (R8/RG8 snorm have their own EXT formats, `F:89,119`; the GL core widens them to RGBA) | `F:156`; `S:416-417`; `X:83` |
| `GLES_TEXFMT_RGBA8UI` | `TEXTURE_FORMAT_EXT_A8B8G8R8I` | RGBA | - | no | yes (HALTI2: integer) | yes | `usampler*`: the bytes, zero-extended | linear; none | - (one format for both signs; R8/RG8 have EXT_R8I/EXT_G8R8I, `F:90-91,120-121`) | `F:159`; `D:187-189`; `X:87` |
| `GLES_TEXFMT_RGBA8I` | `TEXTURE_FORMAT_EXT_A8B8G8R8I` | RGBA | `SIGNED_INT8` | no | yes (HALTI2: integer) | yes | `isampler*`: the bytes, sign-extended by CONFIG2 | linear; none | - | `F:160`; `D:187-189`; `X:87,165` |
| `GLES_TEXFMT_RGBA16UI` | `TEXTURE_FORMAT_EXT_A16B16G16R16I` | RGBA | - | no | yes (HALTI2: integer) | yes | `usampler*`: the halves, zero-extended | linear; none | - (R16/RG16 have EXT_R16I/EXT_G16R16I, `F:102-103,147-148`) | `F:201`; `D:187-189`; `X:90` |
| `GLES_TEXFMT_RGBA16I` | `TEXTURE_FORMAT_EXT_A16B16G16R16I` | RGBA | `SIGNED_INT16` | no | yes (HALTI2: integer) | yes | `isampler*`: the halves, sign-extended by CONFIG2 | linear; none | - | `F:202`; `D:187-189`; `X:90,166` |
| `GLES_TEXFMT_RGBA32UI` | none | RGBA | - | no | yes, as two planes (HALTI5) | no | not here (section 3) | linear; none | two G32R32I planes (R,G then B,A), a companion sampler slot and the shader's two look-ups joined (`etnaviv_nir_lower_128bit.c:89-170`; `etnaviv_texture_state.c:286-296,516-557`); R32UI and RG32UI are EXT_R32I / EXT_G32R32I on HALTI5 (`F:286-291`) | `F:229`; `S:423-429`; `X:96` |
| `GLES_TEXFMT_RGBA32I` | none | RGBA | - | no | yes, as two planes (HALTI5) | no | not here (section 3) | linear; none | as RGBA32UI | `F:230`; `S:423-429`; `X:96` |
| `GLES_TEXFMT_RGBA32F` | none | RGBA | - | no | yes, as two planes (HALF_FLOAT) | no | - | linear; none | two G32R32F planes, as RGBA32UI; the GL core stores RGBA32F / RGB32F / RG32F in RGBA16F on this back end meanwhile (`docs/es3/sized-formats.md`) | `F:233`; `S:423-429`; `X:77` |
| `GLES_TEXFMT_SRGB8_A8` | `TEXTURE_FORMAT_A8B8G8R8` | RGBA | - | no | yes (HALTI0) | yes | - | `SAMP_CTRL1.SRGB`; none | - (the RGBA8 row of the linear twin, `F:278`; no INT_FILTER for sRGB, `F:317-318`) | `D:246-247`; `S:406-407` |

## 3. Integer samplers

**The look-up is the float one.** etnaviv emits an `isampler*`/`usampler*`
look-up as the same `texld` / `texldl` / `texldb` / `txf` it emits for a
float sampler, with nothing that names the result type
(`gallium/drivers/etnaviv/etnaviv_compiler_nir_emit.c:206-247`, called from
`etnaviv_compiler_nir.c:538-570`), and its texture lowering
(`etnaviv_nir_lower_texture.c:449-499`) has no integer case: the TE returns
the texel's integers in the destination register, sign-extended for the
signed formats by CONFIG2 (`D:187-189`).

The online compiler did the opposite: its selector (`src/shaderc/online/
select.rs`, `ir::Op::es100_tex`, `three_lane_lookup`, `fetch_level`) matched
only a float result and refused every integer look-up by name ("an ES 3.00
texture look-up (... an integer result ...) is not selected yet"). Now
`vivante::int_tex::float_typed_lookups` runs on the GC7000's module before
selection (`codegen.rs` `compile_vivante`) and gives every non-shadow integer
look-up the float result type, so it selects as the float shape of the same
dimension and level: `texture`, `textureLod`, `texture` with a bias,
`textureProj` and `texelFetch` on 2D, 3D, 2D array and (but `texelFetch`)
cube. The value's consumers are unchanged and read the register as the
integers it holds: `vec4(texture(u, uv))` is the front end's own `u2f`, an
integer output is a plain move. A look-up the float path does not select
either (an offset, a gradient, a cube `texelFetch`) is still refused by name.

**Filtering.** ES 3.0 3.8.13 makes an integer texture complete only with
NEAREST filters (the GL core, `docs/es3/sized-formats.md`); `INT_FILTER` is
off for every integer class (`F:313-315`: unorm only), and the descriptor
carries no other filter state.

**The 128-bit classes are not decoded.** The TE has no 128-bit format. Mesa
splits RGBA32F/I/UI into two G32R32 planes in its own resource layout, binds a
companion sampler slot to the second plane, and rewrites each look-up into two
whose R,G halves it joins (`etnaviv_nir_lower_128bit.c:89-170`, keyed per
draw on `tex_is_128bit`). Here the GL core keeps the 16-byte texels
interleaved, the compiler has no per-draw key for the sampler's format, and a
`usampler2D` names an RGBA8UI and an RGBA32UI texture alike, so the same
program would need both shapes: not attempted. `gc_texfmt` reports them as
not decoded; none is in the mask, so the GL core never stores one here, and
a binding of one that arrived anyway would sample the back end's black texel,
with a log line naming the class (`emit_slot`), not texels read as another
format.

## 4. Status and what is left

Built (host only; nothing here has run on the GPU):

1. The descriptor decodes R11G11B10F, RGB9E5, RGB10A2, RGB10A2UI,
   RGBA8_SNORM, RGBA8UI/I and RGBA16UI/I (section 2), in dEQP case-count
   order (the 16- and 8-bit integer formats first, about 205 mustpass cases
   each in `texture.format` + `texture.specification`; snorm 101; RGB10_A2UI,
   R11F_G11F_B10F, RGB9_E5 and RGB10_A2 about 25 each). A 16-bit integer
   texture has 8-byte texels and is sampled from the page-aligned tiled copy
   as RGBA16F is (`tiled_copy.rs` `wide_texels`, by size).
2. Integer look-ups select (section 3).

**In `GC_SAMPLED_TEXFMTS`** (`src/backend.rs`): the eight classes above are
in the mask, so the GL core stores them natively on this back end;
`GLES_TEXFMT_RGB10A2` stays out for the reason `PVR_SAMPLED_TEXFMTS` gives.
The test fails when the mask and the `in mask` column disagree. Still unsupported
(refused or kept in a fallback class): RGBA32F/UI/I, R32I/UI, RG32I/UI, ETC2.

Left:

3. `GLES_TEXFMT_ETC2_RGB` is decoded as `TEXTURE_FORMAT_ETC1`, where Mesa uses
   the extended `EXT_NONE` (`F:244`). An ETC1 block decodes the same either way,
   but ETC2's T, H and planar modes only with Mesa's; the ETC1 choice is older
   than this item and ES 2.0's ETC1 textures go through it, so it is a board
   question, not changed here (dEQP-GLES3 `texture.compressed.etc2*`).
4. RGBA32F/I/UI as two planes (section 3), and R32I/UI, RG32I/UI as their own
   classes: the TE samples them as `EXT_R32I` / `EXT_G32R32I` (`F:286-291`),
   but the GL core has no class for them and stores them in RGBA32UI/I.
5. R8, RG8 and the R/RG snorm and integer formats in their own narrow
   classes (Mesa's EXT_R8 / G8R8 / R8I / G8R8I / R16I / G16R16I rows): memory
   only, the RGBA classes sample the same values.

## 5. Host dEQP (GPU=vivante, HOST-ONLY)

`tools/cts/host-deqp-run.sh --gpu=vivante --module=gles3`, filters
`functional.texture.format.` and `functional.texture.specification.`. The
host's Linux OSAL reads every pixel black, so a render verdict means nothing;
what the run shows is which cases get past upload and compile.

Run 2026-10-22, library at `c206c2aa` (this item) and with the one-line
compiler hook taken out for "before" (the descriptor change cannot move a host
outcome while the mask is unchanged), VK-GL-CTS `93bca01`, surface 256x256.
"Upload" counts the cases ending in GL_INVALID_OPERATION at `glTexImage*`
(the GL core's refusal by name), "compile" those ending in a refused shader;
the rest of the failures are image comparisons against a black read-back.

| run | `texture.format` (546) | `texture.specification` (1,163) |
|---|---|---|
| before (base `70142d35` behaviour) | 498 fail, 48 n/s; upload 200, compile 0, image 298 | 6 pass, 1,157 fail; upload 448 (`es3fTextureSpecificationTests.cpp:220`), compile 0, image 706, fbo 3 |
| after (this item, mask unchanged) | **identical** (the failure lists compare byte for byte) | **identical** |
| the eight classes in `GC_SAMPLED_TEXFMTS` (measured on the commit that added them, HOST-ONLY) | 498 fail, 48 n/s; upload 64, compile 0, image 434 | 6 pass, 1,157 fail; upload 141, compile 0, image 1,013, fbo 3 |
| the same mask without this item's compiler pass | upload 64, **compile 136** (`glsTextureTestUtil.cpp:411`), image 298 | upload 141, **compile 307** (`sglrGLContext.cpp:863`), image 706 |

Read: with the eight classes in the mask, 136 + 307 = 443 cases get past upload and
compile that the integer sampler refusal would otherwise stop, and 204 of the
205 remaining upload refusals (64 + 141) are the 32-bit integer formats
(R/RG/RGB/RGBA 32I/UI, the RGBA32UI/I class, section 3); the other is
`specification.random_teximage2d.cube_7`, not traced. Whether those cases then pass
is the board's to say (verdin-imx8mp; no GPU result exists for any of this).
