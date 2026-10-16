<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# sRGB render targets and sRGB texture sampling (ES 3.0): PowerVR, V3D 4.2, GC7000UL

`docs/HANDOFF.md`, open work 4, srgb. Built on the host: the GL core's
sRGB storage class and surface, the CPU codec, the shader back end's sRGB
render-target class and the PowerVR's PBE and TPU gamma (render and
sample); the GC7000UL's and the V3D's sampler decode; `glBlitFramebuffer`'s
sRGB decode and encode (3.2). Not built: the GC7000UL's and the V3D's sRGB
render targets (a draw into one is refused by name there), compressed sRGB
textures. Nothing is measured on a board yet: gl_test 204 `srgb` is the
first check (section 10). Section 9 is the state per item.

Why: `GL_SRGB8_ALPHA8` was stored and rendered as plain RGBA8 and no
sampler decoded sRGB. On the AM62P dEQP-GLES3 tree at `c4ccc966`
(`records/am62p/deqp-gles3-c4ccc966/`, tally with
`tools/cts/deqp-uart-tally.py --mustpass tools/cts/gles3-mustpass.txt`) the
whole `fragment_ops.blend.fbo_srgb.*` group failed, 1,106 of 1,152
("Image compare failed": the blend ran on the encoded bytes; the 46 that
passed are blends whose result does not depend on the destination), and
with it these sRGB cases:

| group | fail | failure text | cause, and what answers it |
|---|---|---|---|
| `texture.specification` srgb8 / srgb8_alpha8 | 34 + 2 | Image comparison failed | no decode on sampling: TPU gamma |
| `texture.filtering.{2d,2d_array}.formats.srgb8_alpha8_*` | 12 | Image verification failed | no decode before the filter: TPU gamma |
| `fragment_out.basic.fixed.srgb8_alpha8_*` | 12 | Image comparison failed | no encode on write: PBE gamma |
| `fragment_out.array.fixed.srgb8_alpha8_*` | 12 | Image comparison failed | several sRGB attachments: refused by name (section 4.4) |
| `fbo.render.*srgb8_alpha8*` | 6 | Image comparison failed | no encode: PBE gamma |
| `fbo.color.{clear,tex3d,texcube,blend}` srgb8_alpha8 | 4 | Image comparison failed | encode and decode |
| `fbo.completeness.renderable.texture.color0.srgb8` | 1 | complete, expected incomplete | SRGB8 is texture-only: now incomplete |
| `fbo.blit.*srgb*`, `fbo.msaa.*srgb*`, `fbo.color.repeated_clear.*srgb*` | 30 + 2 + 3 | GL_INVALID_OPERATION / INVALID_ENUM at es3fFboTestCase.cpp | `glBlitFramebuffer` was not implemented; `src/gl/blit.rs` now copies colour, decoding an sRGB read buffer and encoding an sRGB draw buffer (3.2); the multisample blit (`fbo.msaa`) is the blit item's |

`texture.format.sized.*srgb*` (32) and `texture.filtering.{3d,cube}.formats.srgb*`
(36) were NoResult (their images did not reach those cases),
`pbo.renderbuffer.srgb8_alpha8_*` (2) likewise; `texture.compressed` sRGB
(112) and the `srgb_r8` / `srgb_rg8` names (EXT_texture_sRGB_R8/RG8, not
claimed) are NotSupported.

## 0. Decisions in one table

| | PowerVR (AM62P, BXS-4-64) | V3D 4.2 (Pi 4) | GC7000UL (i.MX8MP) |
|---|---|---|---|
| render encode | PBE: `gamma` bit in `PBESTATE_REG_WORD0`, accumulator F16, memory U8U8U8U8 | TLB: RT output format `SRGB8_ALPHA8`, tile buffer 16F/64bpp, convert at TLB load/store | PE: one sRGB bit (RT0 in `PE_LOGIC_OP`, RT1+ in `PE_RT_CONFIG`), memory A8R8G8B8, needs HALTI3 |
| blend in linear | yes by construction: blend runs in the shader (`nir_lower_blend`) on the F16 linear accumulator; the PBE encodes on store | yes by construction: `BLEND_CFG` blends in the 16F tile buffer, sRGB conversion is at tile load/store | by the PE hardware: Mesa has no lowering anywhere, so Mesa relies on the PE decoding dst and encoding the result (inference from absence, see 6.2) |
| shader change | RT class "F16X4 + saturate" (new) | none for the colour write (it already ends in `vfpack` to the TLB); RT config only | none |
| sample decode | TEXSTATE `gamma` = ON | `TEXTURE_SHADER_STATE.sRGB` (v42) | sampler descriptor `SAMP_CTRL1` SRGB bit (descriptor path) |
| SRGB8 (RGB) | widen to RGBA8, alpha 255, same sRGB class | same | same |
| gating | none | none | `ETNA_FEATURE_HALTI3` for render, HALTI0 for sampling; the board's entry has both |
| built | render and sample | sample; render refused by name | sample; render refused by name |

## 1. Sources and versions

Everything cited as `mesa:` is **Mesa 26.2.1**, tag `mesa-26.2.1`, commit
`da14d65e4499e66468094be52bff9ea0915a695e` (`third_party/mesa-26.2.1/UPSTREAM`).
Three trees were read, all 26.2.1:

* `third_party/mesa-26.2.1/` (vendored): `src/broadcom/cle/v3d_packet.xml`,
  `src/gallium/drivers/v3d/v3dx_format_table.c`.
* `<mesa-26.2.1-caps>/` (`VERSION` = 26.2.1): the gallium v3d
  and etnaviv drivers, `src/etnaviv/hwdb/nxp/gc_feature_database.h`. Its
  `v3dx_format_table.c` is byte-identical to the vendored one (`cmp`).
* `<mesa-26.2.1-pvr>/`: `src/imagination/{vulkan,pco,csbgen}`.
  It carries no `VERSION` file; it is the tree `docs/es3/pvr-formats.md`
  cites, and its `pvr_arch_job_common.c` lines match that document's.

A fourth tree, `<mesa-26.2.1-full>/` (`git clone --depth 1
--branch mesa-26.2.1`, HEAD `da14d65e4499e66468094be52bff9ea0915a695e`), was
cloned for the two files the others lack: `src/compiler/nir/nir_lower_blend.c`
and `src/broadcom/compiler/` (`full:` below; 4.2, 5.3). No Mesa main checkout
was used: the tag is the version yakogl vendors, and every line cited is from
it.

Paths below are relative to those trees; `pvr:` = the imagination tree,
`caps:` = the caps tree, `vend:` = the vendored tree, `full:` = the full clone.

## 2. The GL rules the design must meet (ES 3.0)

(From the ES 3.0.6 specification, §4.1.7-4.1.8 and §3.8.16; the spec is not
in the tree, so section numbers are from memory.)

1. A colour buffer whose attachment has `GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING`
   = `GL_SRGB` (renderbuffer or texture level of `SRGB8_ALPHA8`) is **always**
   treated as sRGB: there is no `GL_FRAMEBUFFER_SRGB` enable in ES 3.0
   (`GL_EXT_sRGB_write_control` is not claimed, `src/gl/consts.rs:1477`).
2. With blending off, the fragment colour (RGB) is converted linear -> sRGB
   when written; alpha is never converted.
3. With blending on, the destination RGB is converted sRGB -> linear, the
   blend is done in linear space, and the result RGB is converted back.
   Constant colour, source and factors are linear. dEQP
   (`es3fBlendTests.cpp:316-331`, per the failure map) builds its reference
   exactly this way.
4. Clear: `glClearColor` / `glClearBufferfv` values are linear and are
   converted on write like any fragment colour.
5. `glReadPixels` of an sRGB buffer returns the stored (encoded) bytes.
   `glBlitFramebuffer` between an sRGB and a linear buffer converts
   (decode source, encode destination); between two sRGB buffers it copies the
   bits (etnaviv does the same, 6.4).
6. Sampling an sRGB texture decodes RGB (alpha untouched) **before**
   filtering.
7. `SRGB8` (RGB, no alpha) is texture-only in ES 3.0 (not colour
   renderable); `SRGB8_ALPHA8` is both. `COMPRESSED_SRGB8_*_ETC2` /
   `*_EAC` are texture-only sRGB too.
8. `GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING`: `GL_SRGB` for sRGB images,
   `GL_LINEAR` otherwise (including the default framebuffer, section 8).

## 3. What yakogl does

### 3.1 The classes (`src/backend.rs`)

* `GLES_TEXFMT_SRGB8_A8` (23): the texel class of `SRGB8_ALPHA8` and
  `SRGB8` (widened with alpha 255, its binding `no_alpha`). Its bytes are
  RGBA8's (`gles_texfmt_is_rgba8_bytes`), so every CPU path that moves RGBA8
  texels moves these unchanged (upload, sub-image, copy, read-back); what
  differs is the sampler's decode and glGenerateMipmap (3.3). All three back
  ends sample it (`PVR_SAMPLED_TEXFMTS`, `GC_SAMPLED_TEXFMTS`,
  `V3D_SAMPLED_TEXFMTS`), so `texture.rs` `sized_format` stores the two
  names in it whatever the GPU. The ES 2.0 report keeps RGBA8 (glTexImage2D
  refuses the names there, glTexStorage2D's store stays as it was).
* `GLES_SURF_SRGB8_A8` (25, outside the integer range): the surface of an
  `SRGB8_ALPHA8` renderbuffer or texture level. RGBA8888's bytes, red,
  green and blue encoded, alpha linear; four bytes a pixel; sampled back as
  `GLES_TEXFMT_SRGB8_A8` (`gles_surf_texfmt`: the frame load decodes);
  shader class `SHC_RT_SRGB` (`gles_surf_rt_class`); not a float surface
  (`gles_surf_is_float`, which the read-back's float path asks instead of
  the class).
* `SRGB_RT` (true on the PowerVR only; host tests stand in with
  `HOST_SRGB_RT`): whether the back end encodes. `rt_format_drawable`
  answers it for the sRGB surface, so on the GC7000UL and the V3D a draw or
  clear into one is refused by name with GL_INVALID_OPERATION
  (`draw.rs` `refuse_undrawable_targets`, the integer formats' path)
  rather than storing linear values as if they were encoded.

### 3.2 The GL core

* `framebuffer.rs`: `renderbuffer_surf_format` and `fb_target_n` give
  `GLES_SURF_SRGB8_A8` for an `SRGB8_ALPHA8` renderbuffer (ES 3.0 report
  only: `is_renderbuffer_color_format` refuses the name under ES 2.0);
  `attachment_storage` takes a texture level of the sRGB class as
  `GL_SRGB8_ALPHA8` and refuses an `SRGB8` level (texture-only, table 3.13:
  `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT`); `fb_target_format` maps the class
  to the surface; `texture_bits` answers `GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING`
  = `GL_SRGB` for a texture level as the renderbuffer row already did. The
  default framebuffer stays `GL_LINEAR` (section 8).
* The draw: the clear colour (`GlesDraw::clear_rgba`) reaches the back end
  linear and unclamped, as for every surface; the back end clamps (unsigned
  normalised) and encodes.
* glReadPixels returns the stored (encoded) bytes, through the RGBA8 path.
* glBlitFramebuffer (`src/gl/blit.rs`, a CPU copy): an sRGB read buffer's
  red, green and blue are decoded to linear before any filtering
  (`decode_float`), an sRGB draw buffer's encoded after it
  (`encode_float`), alpha a linear byte; between two sRGB buffers a NEAREST
  copy keeps every byte (the codec's encode inverts its decode). Texture
  levels and renderbuffers alike. Host test
  `gl::blit::tests::srgb_buffers_decode_as_the_source_and_encode_as_the_destination`.
* glCopyTex{Sub}Image copies the read buffer's stored bytes into the sRGB
  texel bytes unconverted, as it copies RGBA8; a copy between an sRGB and a
  linear buffer converting is not built (no failing dEQP case asks for it).

### 3.3 The CPU codec (`src/gl/srgb.rs`)

`decode(u8) -> f32` and `encode(f32) -> u8`, the ES 3.0 formulas (appendix),
from two tables made at compile time in f64 (`core` has no `powf`; a Newton
fifth root gives x^2.4 = x^2 (x^2)^(1/5)): the 256 decoded values, and the
255 linear values where the encoded value crosses k + 1/2, which make the
encode an exact nearest-byte search. Host tests: all 256 decodes against the
formula, `encode(decode(i)) == i`, a sweep of 2^20 + 1 values against the
formula in f64, NaN and the ends. glGenerateMipmap of an sRGB texture
averages red, green and blue through it (decode, average, encode; alpha
linear), so a 2x2 of black and white is 188, not RGBA8's 128.

### 3.4 The shader back end's sRGB class (`src/shaderc/mod.rs`)

`SHC_RT_SRGB` (4) is not a value of the blend key's 2-bit class field
[30:31], and the 32-bit key has no free bit. It is carried on codes no
other key has: a one-channel class (F16 or F32) has no green, blue or alpha,
so its key keeps red's mask bit only, and the alpha mask bit [29] together
with a one-channel class is the sRGB tag. `shc_blend_key_target(class,
disabled)`:

* RGBA8, F16X4: [26:29] the four mask bits, [30:31] the class (every such
  key as it always was, so every table row keeps its key);
* F16, F32: [26] red's mask bit, [30:31] the class;
* sRGB: [26:28] red, green and blue's mask bits, [29] set, [30:31] F16 plus
  alpha's mask bit.

`shc_blend_key_class` and `shc_blend_key_masked` read them back; the GL
core (`draw.rs` `current_blend_key`) and the precompiled list
(`tools/precompile/src/list.rs`) build keys through it. The key of the
"stencil only, shader discards" ZERO/ONE variant carries the target's class
now too (it was class 0 for every target, which packs an F16X4 or sRGB
pixel as one dword of bytes). No table row has an sRGB variant (glsl2usc
and pco_shc have no such output class): a table-only image refuses the
draw by name, as for the integer variants; the online compiler builds it.

## 4. PowerVR (Rogue BXS-4-64)

### 4.1 Render side: PBE gamma, F16 accumulator

* Format table: sRGB 8-bit colour is `FORMAT(R8G8B8A8_SRGB, U8U8U8U8, F16)` =
  (PBE pack mode `U8U8U8U8`, accumulator `F16`)
  (`pvr:vulkan/pvr_arch_formats.c:213`; B8G8R8A8 `:215`, A8B8G8R8 `:220`);
  the texture/render flags are `_TR_` (`:105,107,112`: texture + render, **no**
  vertex use).
* `pvr_arch_pbe_get_src_format_and_gamma` (`pvr:vulkan/pvr_arch_job_common.c:39-80`):
  the sRGB branch (`:56-60`) turns the gamma request on
  (`PVR_PBE_GAMMA_ENABLED`) and selects `SOURCE_FORMAT_F16_PER_CHANNEL`.
  Integer and 32-bit-component formats take the 8-per-channel branch first
  (`:51-53`), so sRGB is not caught there.
* The gamma bit lives in **`PBESTATE_REG_WORD0`**, not in STATE_WORD0/1:
  `gamma` bit 4, `twocomp_gamma` bit 5 (`pvr:csbgen/rogue/pbestate.xml:245`
  for the struct, `:267-268` for the fields); Mesa sets it at
  `pvr:vulkan/pvr_arch_job_common.c:163-171` (`reg.gamma = true` at `:167`;
  `twocomp_gamma` only for two-component formats). So the change is in
  yakogl's `pbe_reg_words` (REG_WORD0), and `pbe_emit_words` changes only in
  `source_format` / `norm` / `packmode` (4.3).
* What the shader writes: `to_pbe_format` (`pvr:pco/pco_nir_pvfio.c:86-123`)
  maps both sRGB 8-bit formats to `R16G16B16A16_FLOAT` and first applies
  `nir_fsat` to the colour (`:97-101`). So the USC output is four **linear,
  saturated halves** (two 32-bit registers, as for RGBA16F), and the PBE
  gamma-encodes on the way to the U8U8U8U8 memory format.
* Clear: Mesa overrides the pack mode to `F16F16F16F16` for sRGB when it
  packs a clear colour for the PBE (`pvr:vulkan/pvr_arch_job_transfer.c:2372-2384`,
  the override at `:2382-2384`); the clear colour is passed **linear** and
  the PBE encodes it. (`docs/es3/pvr-formats.md:506` already records that
  `glClearBufferfv` on SRGB8_ALPHA8 passes the float colour.)

### 4.2 Blending in linear space

Blending is not a PBE function on this GPU. It is lowered into the fragment
shader: `pvr_init_fs_blend` (`pvr:vulkan/pvr_arch_pipeline.c:2229-2268`)
fills `nir_lower_blend_options` with the render target's pipe format
(`:2243`, `data->fs.output_formats[location]`) and `pco_nir.c:970` runs
`nir_lower_blend`. yakogl's own port is `src/shaderc/online/powervr/blend.rs`
("Derived from Mesa ... nir_lower_blend.c"), which reads the pixel from the
tile buffer, blends in float and writes it, per render-target class.

Consequence, from the data path rather than from a test:

* The USC's tile buffer holds the colour in the **accumulator format**
  (F16 for sRGB, 4.1), and the encode happens once, in the PBE, at the end
  of the tile. So a blend variant that treats the target as the F16X4 class
  (read F16, blend, write F16) blends in linear space with no per-draw
  decode/encode ALU. That is why the class for an sRGB target should be the
  existing F16X4 variant plus the saturate that `pvfio` adds (`:100`), not
  the RGBA8 variant (which unpacks `u8888` and would blend the encoded
  bytes, which is today's failure).
* What the tile buffer contains **at the start of a tile** matters: a
  render that loads the previous colour (no clear, or a second pass) loads
  it through the TPU as a texture, and decoding needs the TEXSTATE gamma
  bit on that load (4.3). yakogl's frame load for the sRGB surface is
  `PVR_INTERNAL_FBLOAD_F16X4` (`FBLOAD_RT`, RGBA16F's) over a texture of the
  sRGB class, whose state has gamma ON (4.4).
  `nir_lower_blend` adds **no** sRGB conversion of its own: the only
  `srgb` tests in `full:src/compiler/nir/nir_lower_blend.c` are in
  `nir_color_logicop` (`:270-275`, `:1047`), which skips float and sRGB
  targets for logic ops. So there is no double conversion on top of the PBE
  gamma, and the shader blends whatever the tile buffer holds (linear F16).
  The same applies to `full:src/broadcom/compiler/v3d_nir_lower_logic_ops.c:352-366`
  on V3D. yakogl's `blend.rs` has no sRGB conversion either: its sRGB class
  reads and writes the halves (4.4); gl_test 204 checks 2 and 4 confirm on
  the board.

### 4.3 Texture side: TPU gamma

* `pvr_arch_pack_tex_state` (`pvr:vulkan/pvr_arch_tex_state.c:207-226`): for an
  sRGB format `word0.gamma = ROGUE_TEXSTATE_GAMMA_ON` (`:226`); only two-component
  formats use `twocomp_gamma` (`:209-224`). Fields: `texstate.xml:234-237`
  (`GAMMA`), `:258-262` (`TWOCOMP_GAMMA`), `:287-288` (word0 `gamma` bit 3).
  The texel format is plain `U8U8U8U8` (same `FORMAT` row as above).
* The TPU returns decoded floats; alpha is not gamma'd for four-channel
  formats. Whether the TPU decodes **before** bilinear filtering is not
  stated in any file in the tree (Vulkan requires it, so Mesa relies on it);
  gl_test 204 check 6.
* Compressed sRGB (ETC2): same gamma bit (`docs/es3/pvr-formats.md:406`);
  not built (the GL core has no compressed sRGB class).
* `SRGB8` (RGB): Mesa has no three-component 8-bit sRGB format on PVR (no
  `R8G8B8_SRGB` row in `pvr_arch_formats.c`), and neither the PBE nor the TPU
  has one; yakogl widens to RGBA8 with alpha 255 (`texture.rs`
  `sized_format`), so SRGB8 is the SRGB8_ALPHA8 sampling class with an
  opaque alpha. It is never a render target (ES 3.0).

### 4.4 What yakogl's PowerVR back end does

* `pbe_reg_words` sets REG_WORD0 `gamma` (bit 4) for `GLES_SURF_SRGB8_A8`
  and for nothing else: the RGBA8888 register words plus that bit.
  `pbe_emit_words`: `source_format` F16_PER_CHANNEL, `norm` set, `packmode`
  U8U8U8U8 -- RGBA8888's state word but for the source format.
  `surf_out_dwords` is 2 (four halves, as RGBA16F), so the MRT layout and
  the background object's program (`bg_kick2_off`) are RGBA16F's.
* The fragment shader: the key's sRGB class (3.4). `blend.rs` treats it as
  RGBA16F for the pixel (read and write two dwords of halves, `Blend::wide`)
  and as RGBA8 for the source (clamped, `Blend::unorm`), and clamps the
  blended colour before the pack (`Blend::clamps_result`, four `fsat`; a
  plain replace writes the clamped source and needs none): pco's `nir_fsat`
  before the F16 pack of an sRGB format, and what keeps a value past 1 out
  of the accumulation the next blend reads. `assemble.rs` packs it as
  F16X4; the record says two outputs (`chain.rs`).
* The back end's own programs take the RGBA16F forms (`pixel_class`): the
  frame load (`FBLOAD_RT[0]`, its texture `gles_surf_texfmt` = the sRGB
  class, so the TPU decodes), the clear-attach program of two dwords, the
  masked clears' F16X4 forms. The clear colour is clamped to [0, 1]
  (`clear_colour`; NaN to 0) and packed as halves (`clear_uniform`); the
  surface clears by quad (`clears_by_quad`, as the float classes: the
  background object's untouched pixels were never measured for an F16
  accumulation).
* Textures: `TEXF_SRGB8_A8` is `TEXF_RGBA8` with `gamma` (TEXSTATE_IMAGE_WORD0
  bit 3; `TexFormat::gamma`, packed by `tex_image_word0`, so the native
  cube's twiddled word has it too); pack info `PVR_PCK_FORMAT_INVALID |
  UNORM` (0x11f), Mesa's setup_pck_info for R8G8B8A8_SRGB.
* Refused by name: an sRGB attachment beside another colour attachment
  (the several-output path packs RGBA8 bytes per output, which the PBE would
  take for halves) -- the `fragment_out.array.fixed.srgb8_alpha8_*` cases.
* Host tests: `srgb_pbe_words_are_mesas`,
  `an_srgb_texture_is_the_rgba8_descriptor_with_the_tpu_gamma`,
  `an_srgb_clear_is_the_clamped_linear_colour_in_halves`
  (`src/pvr/backend.rs`); the frame, load, masked clear and MRT refusal in
  `src/pvr/backend_tests.rs`; the blend tail in the byte model,
  `tests/usc_limits.rs` `an_srgb_target_blends_clamped_linear_halves_on_the_host`
  (case 91's shader keyed for the sRGB class: ONE/ONE, over, reverse
  subtract and replace written as GL's clamped linear blend, and those
  halves' encoding through `gl::srgb` as the bytes stored).

## 5. V3D 4.2 (Raspberry Pi 4)

### 5.1 Render side: TLB internal type 16F, convert at load/store

* Format table: `FORMAT(R8G8B8A8_SRGB, SRGB8_ALPHA8, RGBA8, SWIZ_XYZW, ...)`
  (`vend:src/gallium/drivers/v3d/v3dx_format_table.c:67`; B8G8R8A8 `:63`;
  the X8 variants `:64,68` use swizzle `XYZ1`). Column 2 is the **output
  image format** (TLB load/store format), column 3 the TMU format (plain
  `RGBA8`). The enum has both `srgb8_alpha8` = 0 and `srgb` = 1
  (`vend:src/broadcom/cle/v3d_packet.xml:227-228`); Mesa maps an
  RGB-without-alpha sRGB target to `SRGB8_ALPHA8` too (X8 variants), never to `srgb`.
* Internal type/bpp: `v3dx_format_table.c:249-259`: SRGB8_ALPHA8 (and `srgb`)
  are **`V3D_INTERNAL_TYPE_16F`, `V3D_INTERNAL_BPP_64`**, with the comment
  "sRGB RTs are stored in the tile buffer at 16F, and the conversion to sRGB
  happens at tilebuffer load/store" (`:255-258`). This is the Mesa statement
  that the TLB encodes on store and decodes on load.
* Clamp: `clamp_for_format_and_type` (`caps:src/gallium/drivers/v3d/v3dx_rcl.c:396-407`,
  v4.2): an sRGB format gets `V3D_RENDER_TARGET_CLAMP_NORM` (`:400-401`;
  enum `v3d_packet.xml:147-152`, "norm" = [0,1] for f16). (v7.1 has the
  equivalent `16f_clamp_norm`, `v3dx_rcl.c:436-438`; out of scope here.)
  Clamping applies to colour entering the TLB "and after blending", but not to
  the clear colour (`caps:.../v3dx_draw.c:1705-1710`).
* Clear colour: the TLB clear value is written in the **internal** format
  (`v3dx_draw.c:1739-1743`: `V3D_INTERNAL_TYPE_16F` packs the (clamped,
  R/B-swapped) float colour as `R16G16B16A16_FLOAT`). So an sRGB clear is the
  **linear** float colour packed as halves; no CPU encode.
* `maximum_bpp_of_all_render_targets` must become 64 when any RT is 64bpp
  (yakogl hard-codes BPP_32: `driver/src/v3d/v3d_job.c:162`, `:1028`); this
  halves the tile buffer capacity, so the bin tile size follows. Same change
  RGBA16F render targets need (`int-rt-v3d`).

### 5.2 Blending in linear space

V3D blends in fixed function in the TLB: `v3d_emit_blend` emits `BLEND_CFG`
per render target (`caps:.../v3dx_emit.c:105-122`) with no sRGB handling in
the driver at all. The tile buffer for the RT is 16F *linear* (5.1), so the
blend is linear and the load/store conversion supplies decode of the
destination (load) and encode of the result (store). The one-line source for
that is the `:255-258` comment. There is no shader-side sRGB lowering in
the files we have.

### 5.3 Fragment shader

yakogl's V3D fragment shaders end in `vfpack ... -> TLB` (two halves per
TLB write; `src/shaderc/online/v3d/select.rs:2193-2194`), which is how an
8-bit and a 16F RT are fed alike (the TLB converts to the RT's internal type).
So the shader needs **no change** for sRGB; only the RT configuration
(5.1) does. Mesa agrees: `vir_emit_tlb_color_write`
(`full:src/broadcom/compiler/nir_to_vir.c:1953-2018`) writes `F16` colour
(`TLB_TYPE_F16_COLOR`, `vir_VFPACK_dest` of r,g then b,a; `:1972-1979`,
`:2004-2013`) for every render target except those with `f32_color_rb` set
(32-bit float formats, `:1968-1971`). An sRGB RT is not one of those, so it
takes the same vfpack path as RGBA8, which yakogl already emits. B3 checks the
first sRGB triangle on the board.

### 5.4 Texture side: TMU

* v4.2 `TEXTURE_SHADER_STATE` has an `sRGB` bit (`vend:.../v3d_packet.xml:1627`,
  struct `:1592`, `max_ver="42"`); v7.1 replaced it with `Transfer func`
  (`:1633` struct, `:1699` value `Transfer Func sRGB`). Mesa sets it from
  `util_format_is_srgb(cso->format)` (`caps:.../v3dx_state.c:1023-1028`).
  The TMU format is the plain `RGBA8` row (`v3dx_format_table.c:63-68`).
* `Sampler State.sRGB Disable` (`v3d_packet.xml:1726`, struct `:1709`) is the
  `GL_EXT_texture_sRGB_decode` switch; yakogl leaves it 0 (`v3d_tex.c:553`
  comment "sRGB decode on"), correct for ES 3.0.
* Built: `src/v3d/texture.rs` `state` sets `V3dTexState.srgb` for the sRGB
  class (packed at `driver/src/v3d/v3d_tex.c:489`), `refusal` accepts the
  class, `tmu_format` takes the RGBA8 row for it (host test
  `the_srgb_class_is_the_rgba8_row_with_the_tmu_decode`). Texel ordering and the swizzle rows (`tmu_format`, `:160-181`) stay
  those of RGBA8; `SRGB8` uses the SRGB8_ALPHA8 row with alpha stored 255
  (Mesa's `R8G8B8X8_SRGB` is `RGBA8` + `SWIZ_XYZ1`, `:68`; yakogl widens in
  the GL core instead, which gives the same texels).
* TMU decode-before-filter is by construction (the bit is on the texture
  state, the filter runs in the TMU after format conversion); no source
  statement in the tree. gl_test 204 check 6 covers it.
* The render side (5.1) is not built: `SRGB_RT` is false on the V3D, and a
  draw into the sRGB surface is refused by name in the GL core before it
  reaches `src/v3d/state.rs`.

## 6. GC7000UL (Vivante, i.MX8MP)

All from etnaviv; `caps:` = `mesa-26.2.1-caps/src/gallium/drivers/etnaviv/`.

### 6.1 Render side: PE encodes in hardware, no shader lowering

* The sRGB render format is the **linear one plus a flag**:
  `translate_pe_format` does `fmt = util_format_linear(fmt)` first
  (`etnaviv_format.c:389`, `:403` for the rb-swap query), so `R8G8B8A8_SRGB`
  / `B8G8R8A8_SRGB` share the PE format of `A8R8G8B8`. The BLT / RS format
  tables list the sRGB names beside the linear ones
  (`etnaviv_translate.h:258-262,303-307,334-340`).
* The flag: **RT0** sets `VIVS_PE_LOGIC_OP_SRGB` in `PE_LOGIC_OP`
  (`etnaviv_state.c:326-327`, merged into the register at `:540`); **RT1+**
  set `RT_CONFIG_SRGB` in `PE_RT_CONFIG[rt-1]` (`:330-333`). yakogl has
  both registers: `GC_PE_LOGIC_OP` is written at `src/vivante/backend.rs:1548-1551`
  and `pe_rt_config_8(rt)` at `:1519`.
* Gate: a render target of an sRGB format needs `ETNA_FEATURE_HALTI3`
  (`etnaviv_screen.c:497-498`). The board's chip is in Mesa's feature DB as
  `gc7000ULN_v123`, ChipID 0x7000, revision 0x6204, product 0x70003
  (`mesa-26.2.1-caps/src/etnaviv/hwdb/nxp/gc_feature_database.h:18839-18843`,
  entry head; the repo's own record agrees, `src/vivante/VIVANTE_NOTES.md:12-13`).
  Its `gcFEATURE_BIT_REG_Halti3` is 1 (`:19152`; Halti0 `:19038`, Halti5
  `:19172`); a second ECO-1 entry with the same ids (`:19735`) has the same.
  So the sRGB render gate is open on this board.
* **The bit values** are in `full:src/etnaviv/hw/state_3d.xml.h` (not in the
  vendored or caps trees; yakogl's `regs.rs` ports only the registers it
  uses): `VIVS_PE_LOGIC_OP_SRGB` = `0x80000000` **with its write-enable
  `VIVS_PE_LOGIC_OP_SRGB_MASK` = `0x40000000`** (`:1148-1149`; so setting
  needs `0xc0000000` and an explicit clear needs the mask alone, like
  the register's other `_MASK` bits), `RT_CONFIG_SRGB` = `0x40000000`
  (`:151`, for `PE_RT_CONFIG[rt-1]`), `VIVS_NTE_DESCRIPTOR_SAMP_CTRL1_SRGB` =
  `0x00000004` (`:1984`), legacy `VIVS_TE_SAMPLER_LOG_SIZE_SRGB` =
  `0x80000000` (`:1631`). yakogl's `GC_PE_LOGIC_OP_BASE` (`regs.rs:534`) must
  be checked for which mask bits it already carries.
* No shader work: `etnaviv_compiler*.c` and `etnaviv_blend.c` contain no
  "srgb" (grep: none). The conversion is entirely in the PE.

### 6.2 Blending in linear space

Same finding, stated carefully: `etnaviv_blend.c:177-183` programs
`PE_ALPHA_CONFIG` factors and enables with no format dependence, and nothing
in the driver decodes the destination or encodes the result in a shader, so
Mesa depends on the PE doing "decode dst, blend, encode" when the sRGB bit
is set. That is an inference from the absence of any lowering, backed by
etnaviv passing GL ES 3.0 sRGB blend tests in Mesa CI; no file in the tree
states it. Board check B1 is the confirmation, and if the PE only encodes, the
fallback is a shader-side blend like PowerVR's (the Vivante back end has no
blend lowering today, `src/shaderc/online/vivante/` has none), so this would be
a larger item; keep it as the contingency, not the plan.

### 6.3 Texture side

* Texel format: `translate_texture_format` also starts from
  `util_format_linear` (`etnaviv_format.c:278`): the plain RGBA8 texel row.
* The decode bit: on the HALTI5 descriptor path (which yakogl uses,
  `regs.rs:693`, `texture.rs:385`) it is `VIVS_NTE_DESCRIPTOR_SAMP_CTRL1_SRGB`
  (`etnaviv_texture_desc.c:246-247`); on the legacy sampler path it is
  `VIVS_TE_SAMPLER_LOG_SIZE_SRGB` (`etnaviv_texture_state.c:271`, not ASTC).
  Sampling needs `HALTI0` (`etnaviv_screen.c:406-407`).
* sRGB textures cannot use the integer-filter fast path
  (`etnaviv_format.c:314-318`).
* Built: `src/vivante/texture.rs` samples the sRGB class through the
  A8B8G8R8 row (`format_of`'s default), `samp_ctrl1` adds
  `GC_SAMP_CTRL1_SRGB` (0x4, `regs.rs`) to Mesa's UNK1 for it, and
  `int_filter_wanted` leaves it out (host tests
  `an_srgb_sampler_decodes_and_takes_no_int_filter`,
  `the_srgb_class_is_the_rgba8_texel_row_with_the_decode_bit`). The render
  side (6.1, 6.2) is not built: `SRGB_RT` is false on the GC7000UL and a
  draw into the sRGB surface is refused by name in the GL core; B1 (the PE's
  blend in linear space) and B4 (the `PE_LOGIC_OP` write) wait for it.
* `SRGB8`: etnaviv has `R8G8B8X8_SRGB` (X8 formats, `etnaviv_translate.h:258,303`);
  yakogl widens in the GL core (alpha 255) and samples the SRGB8_ALPHA8 row.

### 6.4 Resolve and copy

`etnaviv_blt.c:862-894`: the BLT sets `src.srgb` / `dest.srgb` only **across
the sRGB<->linear boundary** ("a same-encoding copy is bit-preserving and a
decode+encode roundtrip would only lose precision"), config bits at `:162`
and `:177`. yakogl's MSAA resolve and tiled copies of an sRGB target are
same-encoding and must not set them.

## 7. What the GL core does not do yet

* Compressed sRGB (`COMPRESSED_SRGB8_*_ETC2`, `*_EAC`): no class; the
  `texture.compressed` sRGB cases stay NotSupported. The PowerVR's TPU
  decodes them with the same gamma bit (4.3).
* glCopyTex{Sub}Image between an sRGB and a linear buffer copies the stored
  bytes (3.2).
* `GL_EXT_sRGB` / `GL_EXT_sRGB_write_control` are not claimed: the ES 2.0
  report has no sRGB format (`glTexImage2D` GL_INVALID_VALUE,
  `glRenderbufferStorage` GL_INVALID_ENUM, unchanged), and in ES 3.0 an sRGB
  buffer is always encoded (no `GL_FRAMEBUFFER_SRGB` switch).

## 8. The window surface

It is never sRGB in ES 3.0. `GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING` on
`GL_BACK` is `GL_LINEAR`; the surface is `GLES_SURF_RGBA8888` /
`XRGB8888` / `RGB565`. `eglCreateWindowSurface(EGL_GL_COLORSPACE_SRGB)`
needs `EGL_KHR_gl_colorspace`, which yakogl does not claim:
`src/gl/egl.rs:1375` returns `EGL_BAD_MATCH` ("no sRGB encoding") and the
test at `:4790` pins it. Keep that until a default-framebuffer sRGB is wanted;
do not let a `GL_SRGB8_ALPHA8` renderbuffer's class reach the window-surface
paths (`default_format`, `egl.rs:2402`).

## 9. Implementation state

**S1, classes and the GL core: built** (3.1, 3.2). Host tests:
`tests/es3_srgb.rs` (the GL API with a stand-in back end that stores a clear
as one that encodes must: completeness, the encoding queries, SRGB8
texture-only, the clear read back encoded, the refusal where `SRGB_RT` is
false, every back end sampling the class; its ES 2.0 test that the report
has no sRGB format), `gl::framebuffer` `srgb_colour_buffers_are_the_srgb_surface_and_read_back_their_bytes`,
`gl::texture` `srgb_textures_keep_their_bytes_in_the_srgb_class`,
`tex_storage_sized_formats_map_to_their_unpack_pair`, `gl::draw`
`an_srgb_target_keys_the_srgb_class_and_clears_linear`.

**S2, CPU codec: built** (3.3); glGenerateMipmap uses it
(`generate_mipmap_of_an_srgb_texture_averages_in_linear_space`).

**S3, the shader back end's class: built** for the PowerVR (3.4, 4.4):
`shaderc::key_tests`, the byte-model test in `tests/usc_limits.rs`, and
`gl::draw` `a_draw_into_an_srgb_target_runs_the_srgb_variant` (the online
compiler's variant reached through a draw). The GC7000UL and the V3D blend
in fixed function and have no key (their render side is S5, S6).

**S4, PowerVR render and sample: built** (4.4).

**S5, GC7000UL: sample built** (6.3); render not built (6.1-6.2: the
`PE_LOGIC_OP` / `PE_RT_CONFIG` sRGB bits, the clear path, HALTI3), refused
by name meanwhile.

**S6, V3D: sample built** (5.4); render not built (5.1: internal type 16F,
64 bpp and the tile size that follows, clamp NORM, the output image format,
the clear as halves -- the same change RGBA16F render targets need),
refused by name meanwhile.

**S7, board verification:** gl_test 204 `srgb` (section 10), then on the
AM62P the dEQP-GLES3 images that hold the cases of the table at the top
(`tools/cts/am62p-gles3-tree.txt`, `TREE_ROWS` 0-based): 3
(`fragment_ops.blend.fbo_srgb`), 1 and 2 (`fbo.color`, `fbo.completeness`,
`fbo.render`), 4 (`fragment_out`, `pbo`), 18 (`texture.filtering.2d`,
`2d_array`), 19 (`texture.filtering.3d.formats`), 21
(`texture.filtering.cube`, `texture.format`) and 22
(`texture.specification`).

## 10. Board checks: gl_test 204 `srgb`

`driver/tools/pvrlab/gl_test.c` case 204 (ES 3.0 context, the online
compiler's image):

1. an 8x8 `SRGB8_ALPHA8` renderbuffer cleared to linear 0.2: 124 124 124 255
   (the PBE encodes the clear);
2. its left half blended ONE/ONE with 0.2: 170 (**B1**, blend in linear
   space; a blend of the encoded bytes reads 248);
3. its right half blended SRC_ALPHA/ONE_MINUS_SRC_ALPHA with (1, 0, 0.5,
   0.5): 203 89 160 191 (encoded bytes: 190 62 156);
4. a frame with no clear blends 0.1 over the left half: 188 (the frame load
   decodes 170 to 0.402; a load of the encoded bytes reads 227);
5. an sRGB texel (188, 124, 0, 128) sampled NEAREST into the RGBA8 window:
   128 51 0 128 (decoded, alpha linear);
6. a black/white sRGB pair sampled LINEAR at 0.516: 131 (**B2**, decode
   before the filter; filter then decode reads 58).

Checks 1-4 are the PowerVR's (the other GPUs refuse an sRGB target by name),
5-6 every GPU's. The tolerances (1 for the clear and the texel, 2 for a
blend, 3 for the filter) are what the F16 accumulation, the encode table
and the filter's fraction can move a byte; a result past them is a finding.
**B3** (the V3D's first sRGB triangle) and **B4** (the GC7000UL's
`PE_LOGIC_OP` write) belong to the render sides not built. Settled in
source: `nir_lower_blend` has no sRGB blend branch (4.2) and the V3D's
colour write is a plain F16 vfpack (5.3).

## Appendix: the ES 3.0 formulas

* Decode (8-bit to linear), ES 3.0.6 3.8.16 (eq. 3.24) / EXT_sRGB:
  `c = i/255; lin = c <= 0.04045 ? c/12.92 : ((c+0.055)/1.055)^2.4`.
* Encode (linear to 8-bit), ES 3.0.6 4.1.8: clamp to [0, 1] (NaN to 0);
  `s = lin <= 0.0031308 ? 12.92*lin : 1.055*lin^(1/2.4) - 0.055`; result
  `round(s*255)`.
* Alpha is never converted. An sRGB-to-sRGB copy copies bits.
* `src/gl/srgb.rs` is exact against the f64 formula for every byte and
  monotone (3.3). The GPUs need not match it bit for bit (dEQP compares
  with a threshold); it is the CPU's codec and the host tests' oracle.
