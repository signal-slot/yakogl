# sRGB render targets and sRGB texture sampling (ES 3.0): PowerVR, V3D 4.2, GC7000UL

Design for plan #17 (item `ubo-uniform`, the sRGB design). Nothing here is
implemented; section 9 lists the implementation items.

Why: `GL_SRGB8_ALPHA8` is stored and rendered as plain RGBA8 and no sampler
decodes sRGB (`src/gl/state.rs:105-107`, `src/gl/framebuffer.rs:557-566`,
`src/gl/texture.rs:4558-4559`, `docs/es3-sized-formats.md:98`).
`docs/es3-fbo-fragops-failure-map.md` §3 rank 1 puts about 1,106-1,111 AM62P
`fragment_ops.blend.fbo_srgb.*` failures on this, plus
`fbo.color.blend` srgb8_alpha8 (1), 24 `fragment_out` names, 32
`fbo.blit.conversion` names, and the sRGB cases of the `texture` group
(that count was not established; no per-case list for it exists in the tree).

## 0. Decisions in one table

| | PowerVR (AM62P, BXS-4-64) | V3D 4.2 (Pi 4) | GC7000UL (i.MX8MP) |
|---|---|---|---|
| render encode | PBE: `gamma` bit in `PBESTATE_REG_WORD0`, accumulator F16, memory U8U8U8U8 | TLB: RT output format `SRGB8_ALPHA8`, tile buffer 16F/64bpp, convert at TLB load/store | PE: one sRGB bit (RT0 in `PE_LOGIC_OP`, RT1+ in `PE_RT_CONFIG`), memory A8R8G8B8, needs HALTI3 |
| blend in linear | yes by construction: blend runs in the shader (`nir_lower_blend`) on the F16 linear accumulator; the PBE encodes on store | yes by construction: `BLEND_CFG` blends in the 16F tile buffer, sRGB conversion is at tile load/store | by the PE hardware: Mesa has no lowering anywhere, so Mesa relies on the PE decoding dst and encoding the result (inference from absence, see 6.2) |
| shader change | RT class "F16X4 + saturate" (new) | none for the colour write (it already ends in `vfpack` to the TLB); RT config only | none |
| sample decode | TEXSTATE `gamma` = ON | `TEXTURE_SHADER_STATE.sRGB` (v42) | sampler descriptor `SAMP_CTRL1` SRGB bit (descriptor path) |
| SRGB8 (RGB) | widen to RGBA8, alpha 255, same sRGB class | same | same |
| gating | none | none | `ETNA_FEATURE_HALTI3` for render, HALTI0 for sampling; the board's entry has both |

## 1. Sources and versions

Everything cited as `mesa:` is **Mesa 26.2.1**, tag `mesa-26.2.1`, commit
`da14d65e4499e66468094be52bff9ea0915a695e` (`third_party/mesa-26.2.1/UPSTREAM`).
Three trees were read, all 26.2.1:

* `third_party/mesa-26.2.1/` (vendored): `src/broadcom/cle/v3d_packet.xml`,
  `src/gallium/drivers/v3d/v3dx_format_table.c`.
* `$CCFARM_DEPS_DIR/mesa-26.2.1-caps/` (`VERSION` = 26.2.1): the gallium v3d
  and etnaviv drivers, `src/etnaviv/hwdb/nxp/gc_feature_database.h`. Its
  `v3dx_format_table.c` is byte-identical to the vendored one (`cmp`).
* `$CCFARM_DEPS_DIR/mesa-26.2.1-pvr/`: `src/imagination/{vulkan,pco,csbgen}`.
  It carries no `VERSION` file; it is the tree `docs/es3-pvr-formats.md`
  cites, and its `pvr_arch_job_common.c` lines match that document's.

Not available: `src/compiler/nir/nir_lower_blend.c` and the Broadcom
compiler (`src/broadcom/compiler/`) are in none of those trees, and a
`git clone` of Mesa was refused in the sandbox that wrote this. Two claims
below depend on them and are marked "not verified in source" (4.2, 5.3).
Section 10 lists them as the first things to check. No Mesa main checkout
was used; there is no `mesa-main` in the deps cache.

Paths below are relative to those trees; `pvr:` = the imagination tree,
`caps:` = the caps tree, `vend:` = the vendored tree.

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

## 3. What yakogl does today

* GL core: `renderbuffer_surf_format` (`src/gl/framebuffer.rs:580-588`) sends
  `SRGB8_ALPHA8` to `GLES_SURF_RGBA8888`. The sized-format table stores
  `SRGB8_ALPHA8` and `SRGB8` as `GLES_TEXFMT_RGBA8`
  (`src/gl/texture.rs:4558-4559`).
  `GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING` already answers `GL_SRGB` for an
  `SRGB8_ALPHA8` **renderbuffer** (`framebuffer.rs:1701`, test `:4598-4600`);
  a texture-attached `SRGB8_ALPHA8` level is not checked here and must be
  (item S1).
* The only surface and texel classes are in `src/backend.rs`:
  `GLES_TEXFMT_*` 0-22 (`:84-126`), `GLES_SURF_*` 0-24 (`:205-244`;
  integer ones are the range `GLES_SURF_INT_FIRST..=LAST`),
  `gles_surf_bytes_per_pixel` (`:367`), `gles_surf_rt_class` (`:381-388`)
  and `gles_surf_texfmt` (`:391-399`). Drawability of a non-default surface is
  `rt_format_drawable` (`:354-365`), keyed by the per-build mask
  `INTEGER_RT_FORMATS` with a host-test override
  (`HOST_INTEGER_RT_FORMATS`, `:349`). That is the pattern the sRGB
  gate should copy.
* Back ends: PowerVR builds PBE words in `pbe_reg_words`
  (`src/pvr/backend.rs:731-767`, REG_WORD0/1) and `pbe_emit_words`
  (`:792-821`, STATE_WORD0/1); it has no gamma field set. Texture formats
  are `TexFormat` consts (`:865`, `TEXF_RGBA8`; chosen at `:3626-3629`).
  V3D: `src/v3d/state.rs:29-34` (`refusal`, `:27`) refuses every surface but
  RGBA8888/XRGB8888/RGB565; RT config lives in C, `driver/src/v3d/v3d_job.c`
  (`:162`, `:832-870`, `:917-948`, `:1028-1065`) and `v3d_frame.c:119-162`,
  all hard-wired to internal type 8 / BPP_32 / clamp NONE / output format
  RGBA8. Its texture state already carries an `srgb` field
  (`src/v3d/ffi.rs:457`, packed at `driver/src/v3d/v3d_tex.c:489`) that
  nothing sets. GC7000UL: `pe_color_format` (`src/vivante/backend.rs:263-272`)
  has no sRGB case; texture formats are `format_of`
  (`src/vivante/texture.rs:74-98`); `GC_PE_LOGIC_OP` is written at
  `backend.rs:1548-1551` with no sRGB bit; the sampler's `SAMP_CTRL1` at
  `texture.rs:385` carries `GC_SAMP_CTRL1_UNK1` only.
* Shader back ends know four render-target classes, `SHC_RT_RGBA8/F16X4/F16/F32`
  (`src/shaderc/mod.rs:107-110`), carried by the blend key
  (`src/gl/draw.rs:1065`, `shc_blend_key_rt(gles_surf_rt_class(..))`).
* Host: the host back ends draw nothing (`HOST_INTEGER_RT_FORMATS` doc,
  `src/backend.rs:341-343`). Host tests are therefore GL-state tests against
  a fake back end, register-pack tests (`src/{pvr,v3d,vivante}/backend_tests.rs`)
  and shader-compile tests; pixel results exist only on boards.

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
  the PBE encodes it. (`docs/es3-pvr-formats.md:506` already records that
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
  bit on that load (4.3). yakogl's frame load is
  `PVR_INTERNAL_FBLOAD_F16X4` (`src/pvr/backend.rs:1718`, `FBLOAD_RT`); for sRGB
  it needs the same program with a texture state built with gamma ON.
  **Not verified in source:** how Mesa's `nir_lower_blend` treats an sRGB
  render-target format (it may add its own decode/encode, which would
  double-convert on top of the PBE gamma). `nir_lower_blend.c` is not in the
  trees. The yakogl port should be read for `srgb`; if it has no sRGB branch
  the design above stands; a board test (section 10, B1) settles it.

### 4.3 Texture side: TPU gamma

* `pvr_arch_pack_tex_state` (`pvr:vulkan/pvr_arch_tex_state.c:207-226`): for an
  sRGB format `word0.gamma = ROGUE_TEXSTATE_GAMMA_ON` (`:226`); only two-component
  formats use `twocomp_gamma` (`:209-224`). Fields: `texstate.xml:234-237`
  (`GAMMA`), `:258-262` (`TWOCOMP_GAMMA`), `:287-288` (word0 `gamma` bit 3).
  The texel format is plain `U8U8U8U8` (same `FORMAT` row as above).
* The TPU returns decoded floats; alpha is not gamma'd for four-channel
  formats. Whether the TPU decodes **before** bilinear filtering is not
  stated in any file in the tree (Vulkan requires it, so Mesa relies on it);
  board check B2.
* Compressed sRGB (ETC2): same gamma bit (`docs/es3-pvr-formats.md:406`).
* `SRGB8` (RGB): Mesa has no three-component 8-bit sRGB format on PVR (no
  `R8G8B8_SRGB` row in `pvr_arch_formats.c`), and neither the PBE nor the TPU
  has one; yakogl already widens to RGBA8 with alpha 255
  (`src/gl/texture.rs:4559`), so SRGB8 is the SRGB8_ALPHA8 sampling class
  with an opaque alpha. It is never a render target (ES 3.0).

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
(5.1) does. **Not verified in source:** Mesa's own colour-write lowering for
a 16F RT (`src/broadcom/compiler/`, not in the trees); the vfpack ending is
the evidence from yakogl's compiler, whose RGBA8 path is already
board-validated on the Pi 4. B3 checks the first sRGB triangle.

### 5.4 Texture side: TMU

* v4.2 `TEXTURE_SHADER_STATE` has an `sRGB` bit (`vend:.../v3d_packet.xml:1627`,
  struct `:1592`, `max_ver="42"`); v7.1 replaced it with `Transfer func`
  (`:1633` struct, `:1699` value `Transfer Func sRGB`). Mesa sets it from
  `util_format_is_srgb(cso->format)` (`caps:.../v3dx_state.c:1023-1028`).
  The TMU format is the plain `RGBA8` row (`v3dx_format_table.c:63-68`).
* `Sampler State.sRGB Disable` (`v3d_packet.xml:1726`, struct `:1709`) is the
  `GL_EXT_texture_sRGB_decode` switch; yakogl leaves it 0 (`v3d_tex.c:553`
  comment "sRGB decode on"), correct for ES 3.0.
* yakogl already plumbs the bit: `V3dTexState.srgb` (`src/v3d/ffi.rs:457`),
  packed at `driver/src/v3d/v3d_tex.c:489`; `src/v3d/texture.rs` never sets
  it. Texel ordering and the swizzle rows (`tmu_format`, `:160-181`) stay
  those of RGBA8; `SRGB8` uses the SRGB8_ALPHA8 row with alpha stored 255
  (Mesa's `R8G8B8X8_SRGB` is `RGBA8` + `SWIZ_XYZ1`, `:68`; yakogl widens in
  the GL core instead, which gives the same texels).
* TMU decode-before-filter is by construction (the bit is on the texture
  state, the filter runs in the TMU after format conversion); no source
  statement in the tree. B2 covers it.

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
* **The bit values are not in the trees.** `VIVS_PE_LOGIC_OP_SRGB` and
  `RT_CONFIG_SRGB` come from etnaviv's generated `state_3d.xml.h`, which
  neither vendored nor deps trees contain (yakogl's `regs.rs` ports only the
  registers it uses). They must be taken from the rnndb / `state_3d.xml.h` of
  the same Mesa tag or read from an etnaviv register capture on the board;
  do **not** guess them.
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
  (`etnaviv_format.c:314-318`); yakogl's own rule at `src/vivante/texture.rs:294`
  ("UNORM, non-sRGB ...") must keep excluding them.
* `SRGB8`: etnaviv has `R8G8B8X8_SRGB` (X8 formats, `etnaviv_translate.h:258,303`);
  yakogl widens in the GL core (alpha 255) and samples the SRGB8_ALPHA8 row.

### 6.4 Resolve and copy

`etnaviv_blt.c:862-894`: the BLT sets `src.srgb` / `dest.srgb` only **across
the sRGB<->linear boundary** ("a same-encoding copy is bit-preserving and a
decode+encode roundtrip would only lose precision"), config bits at `:162`
and `:177`. yakogl's MSAA resolve and tiled copies of an sRGB target are
same-encoding and must not set them.

## 7. Cross-cutting: what the GL core must know

* The GL core cannot encode on the CPU on the draw path, because pixels are
  produced on the GPU. It must (a) put sRGB into the surface/texel **class**
  so a back end can set the hardware bits, (b) refuse (INVALID_OPERATION
  incomplete framebuffer) when the back end cannot render the class, as it
  does for integers (`rt_format_drawable`), and (c) keep the ES 2.0 build
  unchanged: the ES 2.0 report refuses the format already
  (`framebuffer.rs:563`, `texture.rs:2026`), so the new classes are only
  reachable with `REPORT_ES2=0`.
* Clear values: the GL core passes `GlesDraw::clear_rgba` (floats, linear,
  `src/backend.rs:701`) unchanged for sRGB targets. PowerVR and V3D convert
  in hardware from linear F16 (4.1, 5.1); GC7000UL's clear writes the RGBA8
  bytes, so for that back end the **PE does not encode a clear that is a
  memory fill**: if the Vivante clear is a fast clear / RS fill of the packed
  colour, the packed colour must be sRGB-encoded on the CPU (appendix);
  if it is a drawn quad through the PE, the PE encodes it. Check which path
  `src/vivante` uses per clear before choosing (item S5).
* `glReadPixels` of the target: bytes are returned as stored; no change.
* `glBlitFramebuffer` (stub today, `framebuffer.rs:2823` per the failure map)
  is a separate item; its sRGB part needs the CPU codec (appendix) or the
  BLT flags (6.4).
* `glGenerateMipmap` on an `SRGB8_ALPHA8` texture: the box filter should
  average in linear space (decode, average, encode); the GL core's mip
  generation is CPU code (`src/gl/texture.rs`), so it needs the codec.
* Compressed sRGB (ETC2 `SRGB8*`, EAC): the GL core decodes ETC2 to RGBA8 on
  the CPU at upload for back ends without hardware ETC2
  (`texture.rs:2925` area); the decoded texels are sRGB-encoded bytes and
  must be tagged with the sRGB class, not RGBA8, to get the sampler's decode.

## 8. The window surface

It is never sRGB in ES 3.0. `GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING` on
`GL_BACK` is `GL_LINEAR`; the surface is `GLES_SURF_RGBA8888` /
`XRGB8888` / `RGB565`. `eglCreateWindowSurface(EGL_GL_COLORSPACE_SRGB)`
needs `EGL_KHR_gl_colorspace`, which yakogl does not claim:
`src/gl/egl.rs:1375` returns `EGL_BAD_MATCH` ("no sRGB encoding") and the
test at `:4790` pins it. Keep that until a default-framebuffer sRGB is wanted;
do not let a `GL_SRGB8_ALPHA8` renderbuffer's class reach the window-surface
paths (`default_format`, `egl.rs:2402`).

## 9. Implementation items

Order matters: S1 and S2 first (no board needed), then per back end.
`REPORT_ES2=0` only in every item; each item adds an ES 2.0 no-regression
check (the existing `es2-guard` test plus a build with `--features report-es2`).
Files owned by running items (`state.rs`, `framebuffer.rs`, `texture.rs`)
must be edited after `gl-core-misc` and `tex3d-finish` merge.

**S1 - classes and the GL core.**
`src/backend.rs`: new `GLES_SURF_SRGB8_A8 = 25` (outside the integer range
`GLES_SURF_INT_FIRST..=LAST`; add `gles_surf_is_srgb`), new
`GLES_TEXFMT_SRGB8_A8 = 23`; extend `gles_surf_bytes_per_pixel` (4),
`gles_surf_rt_class` (new `SHC_RT_SRGB = 4`, see S3), `gles_surf_texfmt`; a
per-build `SRGB_RT_FORMATS` mask + `HOST_SRGB_RT_FORMATS` host override beside
`INTEGER_RT_FORMATS`, consulted by `rt_format_drawable`.
`src/gl/framebuffer.rs`: `renderbuffer_surf_format`, the texture-attachment
surface format, `GL_FRAMEBUFFER_ATTACHMENT_COLOR_ENCODING` for a texture level
of `SRGB8_ALPHA8` (renderbuffer already done, `:1701`).
`src/gl/texture.rs`: `sized_format` maps `SRGB8_ALPHA8`/`SRGB8` to
`GLES_TEXFMT_SRGB8_A8` (RGB widened with alpha 255), the 3D/array/cube
paths follow; ETC2 sRGB decode tags the class (section 7).
Host tests: new `tests/es3_srgb.rs` (fake back end with
`HOST_SRGB_RT_FORMATS`): rbo `SRGB8_ALPHA8` -> `d.target_format ==
GLES_SURF_SRGB8_A8`; completeness fails without the back end bit; texture
attachment encoding is `GL_SRGB`; `SRGB8` is not colour-renderable; upload
of `SRGB8` stores alpha 255 in the sRGB class; default framebuffer encoding is
`GL_LINEAR`; a `report-es2` build still refuses the format.

**S2 - CPU codec.** New `src/gl/srgb.rs` (appendix). Host tests: all 256
decode values against the ES 3.0 formula in f64, encode(decode(i)) == i for
all i, monotone, alpha untouched, NaN -> 0; used by S5 and later blit and
mip-generation work.

**S3 - shader back end class.** `src/shaderc/mod.rs`: `SHC_RT_SRGB`;
`src/gl/draw.rs:1065` blend key; `src/shaderc/online/powervr/blend.rs`: the
class is read/written as F16X4 and the source is saturated (pvfio `:100`),
and the fragment end packs F16X4 (as for RGBA16F). V3D and Vivante map the
class to their RGBA8 shapes (no shader change). Host tests: a shader-compile
test per back end (extend `tests/online_lowering.rs` pattern, new file
`tests/es3_srgb_shaders.rs`): the same blending fragment shader compiles for
`SHC_RT_SRGB` to the F16X4 variant's words plus a saturate on PowerVR, and to
the RGBA8 words on V3D/Vivante. Also check `blend.rs` for any sRGB branch
(4.2 caveat).

**S4 - PowerVR render and sample.**
`src/pvr/backend.rs`: `pbe_reg_words` sets `gamma: true` when the target is
`GLES_SURF_SRGB8_A8` (REG_WORD0 bit 4); `pbe_emit_words`: for the sRGB class
`source_format = F16_PER_CHANNEL`, `packmode = U8U8U8U8`, `norm = true`
(`pvr:vulkan/pvr_arch_formats.c:213` and docs/es3-pvr-formats.md row SRGB8_ALPHA8),
`source_pos` as for the two-register F16X4 output; `FBLOAD_RT` and
`CLEAR_MASKED` use the F16X4 programs for the class, with the fbload texture
built with gamma ON; the clear uniform is the linear float colour; `TexFormat`:
`TEXF_SRGB8_A8` = `TEXF_RGBA8` with `gamma` ON in
`tex_image_word0`, chosen at the `GLES_TEXFMT_RGBA8` branch
(`:3626-3629`). Host tests in `src/pvr/backend_tests.rs`
(+ `backend_csbgen_tests.rs` for the layout): REG_WORD0 bit 4 set only for the
sRGB class and clear for every other class (ES 2.0 words byte-identical);
STATE_WORD1 fields for the class; TEXSTATE word0 bit 3 set only for the
sRGB texel class; clear/fbload program selection.

**S5 - GC7000UL render and sample.**
Obtain `VIVS_PE_LOGIC_OP_SRGB`, `RT_CONFIG_SRGB`, `SAMP_CTRL1_SRGB` from
`state_3d.xml.h` (6.1) and add to `src/vivante/regs.rs`.
`src/vivante/backend.rs`: `pe_color_format` returns A8R8G8B8 for the class
(as RGBA8888 today) and the sRGB flag is ORed into `GC_PE_LOGIC_OP` (RT0,
`:1548-1551`) and `pe_rt_config_8` (RT1+, `:1519`), only if `HALTI3` is in
the device features (it is on this chip); the resolve/tiled-copy paths do not
set it (6.4); clear: per section 7 (CPU-encode if the clear is a memory
fill). `src/vivante/texture.rs`: `format_of` maps the texel class to the
RGBA8 row and `SAMP_CTRL1_SRGB` is set at `:385`; the integer-filter rule at
`:294` keeps excluding it. Host tests in `src/vivante/backend_tests.rs`:
register images (pattern `reg(GC_PE_LOGIC_OP)`, `:3787`) for sRGB vs RGBA8
RT0/RT1; the descriptor word for an sRGB texture; the ES 2.0 images
unchanged.

**S6 - V3D render and sample.**
`src/v3d/state.rs:29-34` accepts the class; `driver/src/v3d/v3d_job.c` RT
config uses internal type 16F, bpp 64, clamp NORM, load/store/MS-store image
format `SRGB8_ALPHA8` for an RT of the class, and
`maximum_bpp_of_all_render_targets` = 64 (`:162`, `:1028`); the clear colour
is the linear float as `R16G16B16A16_FLOAT` (5.1) instead of
`c->rgba8` (`:1041-1044`; `v3d_frame.c` for the first frame); MRT check
`backend.rs:4234` allows the class on RT1+. `src/v3d/texture.rs`: `refusal_for`
(`:116`) accepts `GLES_TEXFMT_SRGB8_A8`, `tmu_format` maps it to the RGBA8
row, `state` sets `srgb = true`. Host tests: `src/v3d/backend_tests.rs`
(packed RT config packets: type/bpp/clamp/format fields, 64 bpp bin
config), `V3dTexState.srgb` (texture bit only for the class),
`tools/v3d/tex_check.c` comparison against Mesa's table, ES 2.0
packets byte-identical.

**S7 - board verification (after S3-S6).** `board-am62p`, `board-pi4`,
`board-imx8mp`: the checks of section 10, then
`dEQP-GLES3.functional.fragment_ops.blend.fbo_srgb.*`,
`fbo.color.blend.srgb8_alpha8`, `fragment_out.*srgb*`, and the texture sRGB
cases.

## 10. Open points to settle on a board (all small)

* **B1** - blend in linear on each chip: draw `GL_FUNC_ADD` ONE/ONE of a
  0.2 grey (linear) over a cleared 0.2 grey into an `SRGB8_ALPHA8` rbo;
  expect the stored byte `encode(0.4)` = 170, not `2*encode(0.2)` = 248 (a
  blend done on the encoded bytes). PowerVR: also whether `nir_lower_blend`
  (not in the trees) adds a second conversion (4.2). GC7000UL: whether the
  PE decodes the destination (6.2).
* **B2** - TPU/TMU/TX filter after decode: a 2x1 sRGB texture (0, 255)
  sampled `GL_LINEAR` at the centre and written to an RGBA8 target; expect
  decode-then-average (0.5, byte 128), not average-then-decode (0.214, byte 55).
* **B3** - first sRGB triangle on V3D 4.2 (the vfpack ending, 5.3).
* **B4** - `PE_LOGIC_OP_SRGB` and friends bit values for GC7000UL from the
  etnaviv headers, and a register capture of an etnaviv sRGB draw if the
  header is not available.
* Mesa checks that could not be done here: `nir_lower_blend.c`'s sRGB
  branch; `src/broadcom/compiler` colour-write lowering for 16F RTs. A
  `git clone --depth 1 --branch mesa-26.2.1` into `$CCFARM_DEPS_DIR` settles
  both; the clone was refused in the sandbox that wrote this document.

## Appendix: a CPU sRGB codec (exact 8-bit, ES 3.0 formula)

Needed later for: `glBlitFramebuffer` between sRGB and linear buffers,
`glGenerateMipmap` of sRGB textures, a GC7000UL memory-fill clear,
software ETC2 sRGB decode, and as the host test oracle.

* Decode (8-bit -> linear float), ES 3.0 §3.8.16 / EXT_sRGB:
  `c = i/255; lin = c <= 0.04045 ? c/12.92 : ((c+0.055)/1.055)^2.4`.
* Encode (linear float -> 8-bit): clamp to [0,1] (NaN -> 0);
  `s = lin <= 0.0031308 ? 12.92*lin : 1.055*lin^(1/2.4) - 0.055`; result
  `round(s*255)`. For determinism build a 256-entry decode table (f32) and
  implement encode by comparing against the 255 midpoints between consecutive
  decoded values (nearest in the *linear* domain), or against
  `(k+0.5)/255` encoded; pick one and test it against the f64 formula for
  all 256 values and a dense sweep of floats. The spec allows hardware
  tolerance (dEQP compares with a threshold), so the codec's job is to be
  exact and monotone, not to match any GPU bit for bit.
* Alpha is never converted. An sRGB-to-sRGB copy copies bits (no round trip).
* `pow` is not in `core`: the table must be built with a `const fn`-safe
  series, an integer-only method, or a build-time generator; check whether the
  crate already has `libm` before adding one (not looked at here).
