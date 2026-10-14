<!-- SPDX-License-Identifier: Apache-2.0 -->
# ES 3.0 capabilities per back end: hardware, what yakogl reports, the gap

For every ES 3.0 limit, per GPU: what the hardware can reach (from Mesa's
drivers for the same silicon), what yakogl reports today, and what is
missing. The limit rows are held by `tests/es3_caps.rs`.

History: the first version (b6004e1, item `es3-caps`) was vetoed and
reverted in 3f78982. The redo (item `caps-redo`, 46d5f80 and 231c5aa) was
never merged. This version (item `caps-land`, plan item id `pvr-shadow`)
lands it. §9 says why the first version was vetoed and how this one avoids
it. A later caps-redo branch (6853b41, written on 3603eb2) was merged into
this text afterwards: its rows that still hold were added with re-read
cites (V3D stream-output pause/resume, GC7000 primitive modes); its
transform feedback 0/0/0, `INTEGER_RT_FORMATS` = 0 everywhere and "ETC2
emulated on the GC7000" claims are stale or wrong at this tip and were
not taken (§1.2 ETC2 row, §2, §4).

## 0. How to read this

Every statement carries one of three labels.

| Label | Meaning |
|---|---|
| **[C]** | verified in yakogl's code at this tip (`path:line`, or a symbol name) |
| **[M]** | verified in Mesa **26.2.1** (tag `mesa-26.2.1`, commit `da14d65e4499e66468094be52bff9ea0915a695e`) at `path:line` |
| **[U]** | not verified here; the text says where the claim came from |

**Mesa tree.** All [M] cites are in the full Mesa 26.2.1 release tree (the
tarball `third_party/mesa-26.2.1/UPSTREAM` pins, by sha256). The repository
vendors a subset of the same release at `third_party/mesa-26.2.1/`
(v3d limits and packets, the v3d format table, the Imagination device
info, csbgen and hwdef files): a cite of a file in that subset reads the
same there. Path prefixes: `V/` = `src/broadcom/` or
`src/gallium/drivers/v3d/` (the file name says which), `E/` =
`src/gallium/drivers/etnaviv/`, `H/` = `src/etnaviv/hwdb/nxp/gc_feature_database.h`
(the i.MX8MP row), `P/` = `src/imagination/`, `PV/` = `src/imagination/vulkan/`.

The redo cited a newer Mesa (main at a9e75e34, 26.3.0-devel) for the files
the vendored subset lacks. Those cites were re-read in 26.2.1. What
exists only in newer Mesa is listed apart, in §10, with lines in Mesa main
at `776c1582808ce2923d5fdbd9a82b7c1c8b79c0bc` (26.3.0-devel, a shallow
clone in the deps cache); no row of §1-§5 rests on it.

**yakogl line numbers** are at this branch's base (main a63fcc5). The
test's yakogl references name symbols, not lines, so they do not go stale
when `state.rs` moves (its Mesa cites are lines of a fixed release).

**What a Mesa statement can and cannot say.** A hardware limit here is what
Mesa programs the part to do, not an independent measurement. Mesa's
*Vulkan* pvr numbers describe the same silicon, but they are a different
API's choices.

## 1. What the hardware reaches, per GPU

### 1.1 V3D 4.2 (Raspberry Pi 4), all [M]

| Capability | Hardware | Cite |
|---|---|---|
| Render targets | 4 on V3D < 7.1 (`ver < 71`); 8 above | `V/common/v3d_limits.h:44` |
| MSAA | 4 samples, the one multisample mode | `v3d_limits.h:41`; only 4 is valid: `V/gallium/drivers/v3d/v3d_screen.c:423-424` |
| Image / framebuffer size | image 4096 (`:62`, not per version); framebuffer 4096 for `ver < 71` (`:63`). Mesa stops at 13 mip levels because more hangs (`:65-69`) | `v3d_limits.h:62-69` |
| Array layers | 2048 | `v3d_limits.h:77` |
| Shader inputs | 64 components for FS, GS and VS inputs (= 16 vec4) | `v3d_limits.h:32-34` |
| Texture samplers | 24 a stage | `v3d_limits.h:39` |
| Point size / line width | 512 / 32 | `v3d_limits.h:46-47` |
| Attribute divisor | up to 0xffff | `v3d_limits.h:84` |
| Float colour RT | R16F, RG16F, RGBA16F, R32F, RG32F, RGBA32F | `V/gallium/drivers/v3d/v3dx_format_table.c:90,95,100,91,96,101` |
| R11F_G11F_B10F | render + sample | `v3dx_format_table.c:146` |
| RGB9_E5 | sample only (RT column `NO`) | `v3dx_format_table.c:147` |
| RGB10_A2 / RGB10_A2UI | render + sample | `v3dx_format_table.c:71,75` |
| Integer colour RT | R8/RG8/RGBA8, R16/RG16/RGBA16, R32/RG32/RGBA32, each I and UI: 18 rows, plus RGB10_A2UI | `v3dx_format_table.c:118-137,75` |
| 32-bit integer / float sample | same rows; return size 32 | `v3dx_format_table.c:132-137,91,96,101` |
| sRGB | RGBA8 sRGB: RT format `SRGB8_ALPHA8`; held in the tile buffer at 16F and converted at tile load/store | `v3dx_format_table.c:63,67,250-259` |
| SNORM 8-bit | sample only (RT `NO`), R8/RG8/RGBA8 | `v3dx_format_table.c:69,84,86` |
| Depth / stencil | D16, D32F, D24S8, S8 sample + render; D32F_S8 is a Z32F plus an S8 | `v3dx_format_table.c:149-157` |
| ETC2 / EAC | all ten ES 3.0 formats sample | `v3dx_format_table.c:159-168` |
| Filtering of 32-bit float and integer formats | the gallium table has no filtering column (6 columns, macro at `v3dx_format_table.c:37-45`). The Vulkan table has one, `false` for the 32F and integer formats (`src/broadcom/vulkan/v3dvx_formats.c:42`, e.g. `:134,142,179`). Matches ES 3.0: NEAREST only | as listed |
| ASTC rows | in the table (`v3dx_format_table.c:179-207`); whether the screen exposes them on 4.2 was not traced | [U] |
| Transform feedback | hardware: the wait, flush-and-count, buffer, specs, flush-data and primitive-counts packets; output spec struct | `V/cle/v3d_packet.xml:440,560,700,724,730,735,750-755` |
| Stream-output buffers | `max_stream_output_buffers = 4`; `stream_output_pause_resume` | `v3d_screen.c:321,250` |
| Occlusion queries | hardware counter packet; `occlusion_query` cap | `v3d_packet.xml:822`; `v3d_screen.c:249` |
| Primitive restart | flag on the indexed list packets; fixed-index cap | `v3d_packet.xml:565,580,595`; `v3d_screen.c:246-248` |
| Shading-language level | GLSL 3.30, ESSL 3.10 | `v3d_screen.c:307,309` |
| Constant buffer 0 | 16K floats = 64 KiB, 16 constant buffers | `v3d_screen.c:164-165` |
| Screen caps | 3D/cube levels `V3D_MAX_MIP_LEVELS` (13), array layers, render targets `devinfo.max_render_targets` (set at `src/broadcom/common/v3d_device_info.c:82`), varyings `V3D_MAX_FS_INPUTS / 4` (16), samplers | `v3d_screen.c:323,326-332,169-170` |

### 1.2 GC7000UL (i.MX8MP), all [M]

The hwdb row is the i.MX8MP one: `H:18841` (ChipID 0x7000, revision
0x6204, product 0x70003, EcoID 0x0, CustomerID 0x11; the entry opens at
`:18839-18840`). A second row with the same IDs and EcoID 0x1 starts at
`:19735`. In etnaviv, `VIV_FEATURE(screen, X)` is that row's bit and
`halti` is the highest HALTI bit set.

| Capability | Hardware | Cite |
|---|---|---|
| Generation | HALTI0..HALTI5 all 1; 2 shader cores, 1 pixel pipe, 0x140 constants, 16 varyings | `H:19038,19058,19127,19152,19157,19172` (Halti0-5); `H:18851,18866,18853,18876` |
| Render targets | `num_rts` = 8 for halti >= 2; with HALTI2 the cap is `num_rts / 2` = **4** (half kept for float emulation) | `E/etnaviv_screen.c:774-782,299-304` |
| Independent blend | halti >= 5 | `etnaviv_screen.c:305-306` |
| Colour RT formats (`gpu_supports_render_format`) | integer and float RTs with HALTI2; R8 / R8I / R8UI need HALTI5 (met); sRGB RT needs HALTI3 (met); 128-bit formats need HALF_FLOAT, and the row has HalfFloatPipe (`H:19026`) | `etnaviv_screen.c:470-515` (`:490-495,497-498,500-502,504-505`) |
| Sampled formats (`gpu_supports_texture_format`) | sRGB and EXT formats HALTI0; snorm HALTI1; integer and float HALTI2; 128-bit integer HALTI5; ASTC off (`NO_ASTC` = 1, `H:19200`) | `etnaviv_screen.c:394-449` (`:406-410,412-414,416-417,419-421,426-429`) |
| ETC2/EAC | hardware EXT formats in the format table (ETC2_RGB8, RGB8A1, RGBA8, R11, RG11, signed and unsigned), gated by HALTI0. Not emulated here: `etnaviv_etc2.c` patches ETC2 blocks only on parts without HALTI1 (`etna_etc2_needs_patching`), and this row has HALTI1 | `E/etnaviv_format.c:244-250`; `etnaviv_screen.c:409-410`; `E/etnaviv_etc2.c:33-56` (HALTI1 test at `:41-42`) |
| Half-float linear filtering | needs `HALF_FLOAT` | `etnaviv_screen.c:280` |
| Max texture / RT size | 8192 with `Texture8K` (set), RT 8192 with `Render8K` (set) | `etnaviv_screen.c:970-973`; `H:18986,18992` |
| 3D / array textures | from HALTI0; array layers = max texture size (comment "TODO: verify"); 3D levels = log2(size) | `etnaviv_screen.c:283-288` |
| MSAA | the `MSAA` bit is set and `SmallMSAA` is set, so 2x is refused and **4x** is the only mode (`gpu_supports_msaa`; only 1, 2 and 4 translate to a scale). Multisample *textures* are a debug-only cap (`texture_multisample = DBG_ENABLED(ETNA_DBG_DEQP)`) | `H:18958,19129`; `etnaviv_screen.c:451-468,296`; `E/etnaviv_translate.h:501-517` |
| Samplers | 16 fragment + 16 vertex from halti >= 1 | `etnaviv_screen.c:824-827` |
| Varyings | `max_varyings` = MIN3(`ETNA_NUM_VARYINGS` 16, hwdb `VaryingCount` 16, `max_vs_outputs - 1`); `max_vs_outputs` is 32 for halti >= 5, so 16. One output slot is the position (the comment there); yakogl reaches 15 for the fragment stage (`src/gl/context.rs:119-122` [C]) | `etnaviv_screen.c:963,965-968`; `E/etnaviv_internal.h:41`; `H:18876` |
| Transform feedback | **none on this part**: `HWTFB` = 0 in the row, and etnaviv reports `max_stream_output_buffers` 4 only with HWTFB or the `deqp` debug flag, so 0 here (64 / 64 components otherwise) | `H:19202`; `etnaviv_screen.c:267-269` |
| Occlusion queries, primitive restart (fixed index) | HALTI0 | `etnaviv_screen.c:309-311,254-255` |
| Instance id / divisor | HALTI2 | `etnaviv_screen.c:275-276` |
| Shadow samplers | `texture_shadow_map = true` | `etnaviv_screen.c:281` |
| Primitive modes | triangle strips only with `BUG_FIXES8`, line loops only with `LINE_LOOP`; the row sets both, so all ES primitive modes | `etnaviv_screen.c:341-345`; `H:19047,19078` |
| Uniform blocks | none in hardware; `max_const_buffer0_size` is 16384 "so state tracker enables UBOs", lowered to uniforms | `etnaviv_screen.c:195-199` |
| Shading-language level | etnaviv sets none; the gallium default is GLSL 1.20 | `src/gallium/auxiliary/util/u_screen.c:64-65` |
| Vertex-stage textures | read from 4x4-tiled copies only (yakogl finding) | `src/vivante/VIVANTE_NOTES.md:243-246` [C] |

### 1.3 BXS-4-64 (AM62P, BVNC 36.53.104.796), all [M]

The device info and the packet XML describe the part; Mesa's *Vulkan*
driver (`PV/`) adds what one API chose to expose on it.

| Capability | Hardware | Cite |
|---|---|---|
| Identity | BVNC 36.53.104.796; 1 cluster, 1 raster pipe, 16x16 tiles; ASTC flag set | `P/common/device_info/bxs-4-64.h:131-137,95-96,99-100,36` |
| Colour outputs | 8 PBE emit registers (`PVR_MAX_COLOR_ATTACHMENTS = PVR_NUM_PBE_EMIT_REGS`) | `P/common/pvr_limits.h:35`; `P/include/hwdef/rogue_hw_defs.h:154` |
| USC output registers per pixel | **4** without `eight_output_registers`, 8 with (`rogue_get_max_output_regs_per_pixel`). BXS-4-64 has no `eight_output_registers`; `usc_min_output_registers_per_pix = 2` is a minimum only. So four 32-bit registers a pixel: four RGBA8, or two 64-bit, or one 128-bit source (64/128-bit sources start on an even register) | `P/include/hwdef/rogue_hw_utils.h:189-195`; `PV/pvr_arch_mrt.c:61`; `bxs-4-64.h:77,104`; yakogl's tool agrees: `tools/pco_shc/pco_shc.c:555-562` [C] |
| MSAA | `max_multisample = 4`, `isp_samples_per_pixel = 4` | `bxs-4-64.h:47,90,92` |
| PBE pack modes | U8, S8, U16, S16, F16, U32, S32, F32 in 1/2/3/4 channels; R5G6B5, A1R5G5B5, A4R4G4B4, A2R10B10G10, A2F10F10F10, F11F11F10, SE9995 | `P/csbgen/rogue/pbestate.xml:60-122` |
| Texture formats | the same widths and signs, F11F11F10, SE9995, A2R10B10G10, depth/stencil forms; compressed ETC2_RGB, ETC2A_RGBA, ETC2_PUNCHTHROUGHA, EAC R11 / RG11 signed and unsigned, BC1-7 | `texstate.xml:95-192,195-231` |
| Mesa's per-format bind flags (V vertex, T sampler, R render target, I storage) | integer R8/RG8/RGBA8/R16/RG16/RGBA16/R32/RG32/RGBA32 `UINT` and `SINT` are `VTRI`; **R10G10B10A2_UINT `VTRI`**; R16/R32 float and RGBA16F/RGBA32F `VTRI`; **R11G11B10_FLOAT `_TRI`**; **R9G9B9E5 `VT__`** (not renderable); SNORM 8-bit `VTRI`; RGBA8 sRGB `_TR_`; R10G10B10A2_UNORM `VTRI` | `PV/pvr_arch_formats.c:93-149,119,150,151,92,96,102,105,115` |
| ETC2 / EAC (all ten ES 3.0 formats) | map to `ETC2_RGB`, `ETC2_PUNCHTHROUGHA`, `ETC2A_RGBA`, `EAC_R11/RG11` signed and unsigned; `textureCompressionETC2 = true` | `PV/pvr_arch_formats.c:158-167`; `PV/pvr_physical_device.c:253` |
| sRGB | `gamma` / `twocomp_gamma` fields on texture and PBE state | `texstate.xml:234-237,258-262,287-288`; `pbestate.xml:183,267-268` |
| Image size | texture state: width/height 14 bits (MAX_SIZE 16383), depth 11 bits (MAX_SIZE 2047); types 1D, 2D, 3D, CUBE (arrays are depth > 1 of a 2D type). The render size limit in z is 2048 (`rogue_get_render_size_max_z`). Mesa's Vulkan driver exposes 4096 for 1D/2D/cube and **256** for 3D and array layers as device limits; 2048 only in per-format image properties | `texstate.xml:248-256,275-277,309-311`; `rogue_hw_utils.h:265-273`; `PV/pvr_physical_device.c:620-624`; `PV/pvr_formats.c:680-713` |
| Compare (shadow) sampler | `cmp_mode`, 3 bits | `texstate.xml:52-61,344` |
| Primitive restart | cut index, 32 bits, in the VDM state words | `P/csbgen/rogue/vdm.xml:152-171` (fields at `:154,162,170`) |
| Instancing | `instance_count` and `start_instance` in the index-list packets; `max_instances_per_pds_task = 32` | `vdm.xml:208,231,243`; `bxs-4-64.h:46,91` |
| Occlusion queries | hardware visibility results (`ROGUE_OQUERY_ALIGN`); Mesa's Vulkan pvr reports `occlusionQueryPrecise = false` (non-precise: enough for ANY_SAMPLES_PASSED) | `rogue_hw_defs.h:51`; `PV/pvr_physical_device.c:256` |
| Transform feedback | **none**: no packet or flag in the device info; Mesa's Vulkan pvr exposes no transform-feedback feature (it only sets `transformFeedbackPreservesProvokingVertex = false`, `:454`, and the triangle-fan one, `:844`). The only stream-out code is the PDS program a render context stores the TA state with (`PV/pvr_arch_job_context.c:88-155`); the DDK has the VDM context-store `STREAM_OUT` registers (`third_party/ti-img-rogue-ddk-25.2/hwdefs/rogue/km/rgx_cr_defs_km.h:1774-1778` [C]) | as listed |
| Mesa Vulkan limits for this silicon | maxColorAttachments 4; maxFragmentOutputAttachments 4; maxVertexOutputComponents 64; maxFragmentInputComponents 64; maxUniformBufferRange 16 KiB; 12 uniform buffers a stage; 16 samplers a stage; framebuffer and sampled sample counts 1 and 4; anisotropy 16 | `PV/pvr_physical_device.c:739,678,655,677,626,635,634,735,740,704` |
| Point size / line width | 1..511 / 1..16 (what Vulkan reports) | `P/common/pvr_limits.h:44,48` |

## 2. What yakogl reports today, with the code behind it (all [C])

`glGetIntegerv` is `get_integerv` in `src/gl/state.rs`; the constants are
in `src/gl/context.rs`. "Per GPU" means a `cfg(feature = "gpu-…")` constant.

| Query | Reports (ES 3.0 build) | Cite |
|---|---|---|
| MAX_COLOR_ATTACHMENTS | 3, every GPU | `state.rs:1250` ← `context.rs:139` |
| MAX_DRAW_BUFFERS | 3 | `state.rs:1249` ← `context.rs:152` |
| MAX_SAMPLES | 1 | `state.rs:1251` |
| MAX_3D_TEXTURE_SIZE, MAX_ARRAY_TEXTURE_LAYERS | 256, 256 | `state.rs:1253-1254` |
| MAX_TEXTURE_SIZE, MAX_CUBE_MAP_TEXTURE_SIZE, MAX_RENDERBUFFER_SIZE | 4096 each | `state.rs:1190-1192` |
| MAX_VIEWPORT_DIMS | 4096 x 4096 | `state.rs:279,1180-1183` |
| MAX_VERTEX_ATTRIBS | 16 | `state.rs:1184` ← `context.rs:78` |
| MAX_TEXTURE_IMAGE_UNITS, MAX_COMBINED_TEXTURE_IMAGE_UNITS, MAX_VERTEX_TEXTURE_IMAGE_UNITS | 16, 32, 16 | `state.rs:1185-1189` ← `context.rs:95,107`, `backend.rs:571,582,602` |
| MAX_VERTEX/FRAGMENT_UNIFORM_VECTORS, MAX_VERTEX/FRAGMENT_UNIFORM_COMPONENTS | 384 / 1536 | `state.rs:1198-1199,1214-1215` ← `context.rs:83,89-90` |
| MAX_VARYING_VECTORS / MAX_VARYING_COMPONENTS | GC7000 15 / 60; PowerVR 8 / 32; V3D 8 / 32 | `state.rs:1200,1218` ← `context.rs:124,127,133` |
| MAX_VERTEX_OUTPUT_COMPONENTS, MAX_FRAGMENT_INPUT_COMPONENTS | GC7000 60, 60; PowerVR 32, 32; V3D 32, 32 | `state.rs:1219-1220` ← `context.rs:158-159` |
| MAX_VERTEX/FRAGMENT_UNIFORM_BLOCKS, MAX_COMBINED_UNIFORM_BLOCKS | 12, 12, 24 | `state.rs:1227-1228` ← `program.rs:69` |
| MAX_UNIFORM_BUFFER_BINDINGS | 24 | `state.rs:1229` ← `context.rs:168` |
| MAX_UNIFORM_BLOCK_SIZE | 6144 (= 1536 x 4) | `state.rs:1230` |
| UNIFORM_BUFFER_OFFSET_ALIGNMENT | 4 | `state.rs:1231` |
| MAX_COMBINED_VERTEX/FRAGMENT_UNIFORM_COMPONENTS | 1536 (no block share) | `state.rs:1232-1234` |
| MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS / SEPARATE_ATTRIBS / SEPARATE_COMPONENTS | **64 / 4 / 64**. SEPARATE_COMPONENTS is set to the interleaved limit, not ES 3.0's 4, because the capture runs on the CPU (`gl/tf_capture.rs`) and has no hardware ceiling | `state.rs:1259-1267` ← `tf.rs:32,35,43` (the reason is the comment at `tf.rs:36-42`) |
| NUM_COMPRESSED_TEXTURE_FORMATS | 3 (ETC1, RGB8_ETC2, RGBA8_ETC2_EAC) | `state.rs:1172-1177` |
| MAX_ELEMENT_INDEX | 0xFFFFFF | `state.rs:1255` |
| MIN / MAX_PROGRAM_TEXEL_OFFSET, MAX_TEXTURE_LOD_BIAS, MAX_SERVER_WAIT_TIMEOUT | -8, 7, 2, 0 | `state.rs:1273-1274,1257,1275` ← `context.rs:162-163` |
| ALIASED_LINE_WIDTH_RANGE | 1..16 | `state.rs:956-960` |
| ALIASED_POINT_SIZE_RANGE | 1..511 PowerVR, 1..64 GC7000, 1..512 V3D | `state.rs:961-965` ← `state.rs:307-313` |

**Under `report-es2`** (the ES 2.0 report a conformance image uses):

- The ES 3.0-only pnames are `GL_INVALID_ENUM`. `es3_only_pname`
  (`state.rs:1010-1058`) lists them, and `get_integerv` refuses them
  before it answers (`state.rs:1080-1083`). From the table above these are
  MAX_SAMPLES, MAX_3D_TEXTURE_SIZE, MAX_ARRAY_TEXTURE_LAYERS,
  MAX_ELEMENT_INDEX, MAX_TEXTURE_LOD_BIAS, MAX_SERVER_WAIT_TIMEOUT,
  MIN/MAX_PROGRAM_TEXEL_OFFSET, all the *_UNIFORM_COMPONENTS, the uniform
  block and binding limits, UNIFORM_BUFFER_OFFSET_ALIGNMENT,
  MAX_VARYING_COMPONENTS, MAX_VERTEX_OUTPUT_COMPONENTS,
  MAX_FRAGMENT_INPUT_COMPONENTS, and the three
  **MAX_TRANSFORM_FEEDBACK_*** limits. (The `if crate::REPORT_ES2 { 0 }`
  branches of the transform feedback arms at `state.rs:1259-1267` are
  never reached: the refusal at `:1080` comes first.) This refusal came
  with 1bad575 (2026-10-11, the ES 2.0 error rules, state/program tranche).
- GL_MAJOR_VERSION, GL_MINOR_VERSION, GL_NUM_EXTENSIONS (`state.rs:1207-1210`)
  and GL_MAX_DRAW_BUFFERS (`state.rs:1245-1248`) are `GL_INVALID_ENUM`
  through their own arms.
- The ES 2.0 limits keep the values in the table: MAX_VERTEX_ATTRIBS, the
  UNIFORM_VECTORS pair, MAX_VARYING_VECTORS, the three TEXTURE_IMAGE_UNITS,
  the three texture/renderbuffer sizes, NUM_COMPRESSED_TEXTURE_FORMATS,
  MAX_VIEWPORT_DIMS and the two ranges.
- MAX_COLOR_ATTACHMENTS answers 3: its token is
  GL_MAX_COLOR_ATTACHMENTS_NV of GL_NV_fbo_color_attachments, which the
  ES 2.0 list claims (`state.rs:111-123`).

`es2_report_answers_are_unchanged` in the test pins all four groups, and
`tests/es2_report_guard.rs` (`es3_only_queries_are_invalid_enum`,
`draw_buffer_queries_are_invalid_enum`) pins the refusals as well.

## 3. ES 3.0 minimum by minimum, per GPU

"Hw" is the ceiling from §1; "Reports" is §2 (the ES 3.0 build). `=`
means the minimum is met.

| ES 3.0 minimum (spec value) | PowerVR BXS-4-64 | GC7000UL | V3D 4.2 |
|---|---|---|---|
| MAX_DRAW_BUFFERS, MAX_COLOR_ATTACHMENTS ≥ 4 | Hw 8 PBE emit registers, four 32-bit output registers a pixel: four RGBA8-class outputs fit, wider formats take pairs (`rogue_hw_utils.h:189-195`); Mesa Vulkan exposes 4 (`pvr_physical_device.c:739`). **Reports 3.** `VIVANTE_NOTES.md:1136-1137` plans to keep the PowerVR at 3, below the minimum, which the hardware does not require for ≤32-bit formats | Hw 4 (`etnaviv_screen.c:299-304,774-782`). **Reports 3**; plan 3 → 4 (`VIVANTE_NOTES.md:1136-1137`) | Hw 4 (`v3d_limits.h:44`; `v3d_screen.c:332`). **Reports 3** (mrt4) |
| MAX_3D_TEXTURE_SIZE ≥ 256; MAX_ARRAY_TEXTURE_LAYERS ≥ 256 | Hw 11-bit depth (`texstate.xml:310`), render z 2048 (`rogue_hw_utils.h:265-273`); Mesa Vulkan's device limits are 256 / 256 (`pvr_physical_device.c:622,624`). Reports 256/256 = | Hw layers = max texture size (8192), "TODO: verify" (`etnaviv_screen.c:283-288`). Reports 256/256 = | Hw 2048 layers (`v3d_limits.h:77`). Reports 256/256 = |
| MAX_SAMPLES ≥ 4 | Hw 4 (`bxs-4-64.h:92`); Vulkan sample counts 1, 4 (`pvr_physical_device.c:735,740`). **Reports 1** | Hw 4x only (`etnaviv_screen.c:451-468`, `H:19129`); multisample textures debug-only in Mesa (`:296`). **Reports 1** | Hw 4 (`v3d_limits.h:41`; `v3d_screen.c:423-424`). **Reports 1** |
| MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS ≥ 64, SEPARATE_ATTRIBS ≥ 4, SEPARATE_COMPONENTS ≥ 4 | Hw none (§1.3). **Reports 64 / 4 / 64 =** (the capture runs on the CPU, `gl/tf_capture.rs`) | Hw none (`HWTFB` = 0, `H:19202`); Mesa 26.2.1 offers 0 buffers here (`etnaviv_screen.c:267`). **Reports 64 / 4 / 64 =** (CPU capture) | Hw yes, 4 buffers (packets; `v3d_screen.c:321`). **Reports 64 / 4 / 64 =** (CPU capture) |
| MAX_VERTEX_UNIFORM_BLOCKS ≥ 12, FRAGMENT ≥ 12, COMBINED ≥ 24, BINDINGS ≥ 24 | no UBO hardware relied on: blocks are flattened into the default block ([C] the comment at `state.rs:1221-1226`). Reports 12/12/24/24 = | same lowering (etnaviv lowers UBOs to uniforms too, `etnaviv_screen.c:195-199`); = | same lowering; = |
| MAX_UNIFORM_BLOCK_SIZE ≥ 16384 | **Reports 6144** (yakogl's budget, not a hardware cap) | same | same |
| MAX_COMBINED_VERTEX / FRAGMENT_UNIFORM_COMPONENTS ≥ 50176 / 50048 | **Reports 1536** | same | same |
| MAX_VERTEX / FRAGMENT_UNIFORM_COMPONENTS ≥ 1024 / 896 | 1536 = | 1536 = | 1536 = |
| MAX_VARYING_VECTORS ≥ 15, MAX_VARYING_COMPONENTS ≥ 60, MAX_FRAGMENT_INPUT_COMPONENTS ≥ 60 | Mesa Vulkan: 64 out / 64 in components (`pvr_physical_device.c:655,677`). **Reports 8 / 32 / 32** ("until the USC's limit is measured", `context.rs:125`) | Hw 16 varyings, one slot is the position (`etnaviv_screen.c:965-968`, `H:18876`). Reports 15 / 60 / 60 = | Hw 64 input components (`v3d_limits.h:32-34`; `v3d_screen.c:323`). **Reports 8 / 32 / 32** ("raised with the board run", `context.rs:128-131`) |
| MAX_VERTEX_OUTPUT_COMPONENTS ≥ 64 | **Reports 32**; Mesa Vulkan 64 | **Reports 60** (15 x 4); whether 64 is reachable with 16 output rows, one of them the position, is [U] | **Reports 32**; Hw 64 |
| MAX_TEXTURE_SIZE, MAX_CUBE_MAP_TEXTURE_SIZE, MAX_RENDERBUFFER_SIZE ≥ 2048 | Hw state field 16384 (`texstate.xml:276`); Mesa Vulkan 4096 (`pvr_physical_device.c:620-623`). Reports 4096 = | Hw 8192 (`etnaviv_screen.c:970-973`, `H:18986,18992`). Reports 4096 = | Hw 4096 (`v3d_limits.h:62-63`). 4096 = |
| MAX_TEXTURE_IMAGE_UNITS ≥ 16, VERTEX ≥ 16, COMBINED ≥ 32 | Mesa Vulkan 16 samplers a stage (`pvr_physical_device.c:634`). 16/16/32 = | Hw 16 + 16 (`etnaviv_screen.c:824-827`). 16/16/32 = | Hw 24 (`v3d_limits.h:39`). 16/16/32 = |
| NUM_COMPRESSED_TEXTURE_FORMATS ≥ 10 | Hw has all ten (`texstate.xml:195-231`; `pvr_arch_formats.c:158-167`). **Reports 3** | ETC2/EAC are hardware EXT formats from HALTI0 (`etnaviv_format.c:244-250`). **Reports 3** | All ten (`v3dx_format_table.c:159-168`). **Reports 3** |
| MAX_ELEMENT_INDEX ≥ 2²⁴-1; MAX_TEXTURE_LOD_BIAS ≥ 2; texel offset -8..7; MAX_VERTEX_ATTRIBS ≥ 16 | = (exactly the minimum for the first three) | = | = |
| Line width / point size ranges ≥ 1 | 16 / 511 = | 16 / 64 = | 16 / 512 = (hw 32 / 512, `v3d_limits.h:46-47`) |

## 4. Integer, float and sRGB render targets and sampling

**Render targets.** yakogl's colour surfaces are `GLES_SURF_*` ([C]
`src/backend.rs:259-264` for the 6 original ones, `:278-298` for the 19
integer ones, `:310` for sRGB). A back end says which integer surfaces it
draws in `INTEGER_RT_FORMATS` (`backend.rs:404-417`) and whether it draws
sRGB in `SRGB_RT` (`:432`). The GL core accepts every one of these
renderbuffer names (ES 3.0 requires them) and refuses a draw or clear into
one the back end does not draw, by name (`rt_format_drawable`, `:442-460`).

| | PowerVR | GC7000UL | V3D 4.2 |
|---|---|---|---|
| Integer RT (R8I … RGBA32UI, RGB10_A2UI; 19) | **yakogl: all 19 drawable**, as the one colour attachment of a draw ([C] `backend.rs:413-416`; `src/pvr/backend.rs:868` `pbe_int_packmode`). Hw: all 19 are `R` formats in Mesa's table, R10G10B10A2_UINT too [M] `pvr_arch_formats.c:93-148,119`; pack modes `pbestate.xml:60-122` | **yakogl: 0 drawable** ([C] `backend.rs:405-408`). Hw: integer RTs with HALTI2, R8I/R8UI with HALTI5 (both met); 128-bit RGBA32I/UI need HALF_FLOAT (set) [M] `etnaviv_screen.c:490-505` | **yakogl: 0 drawable** ([C] `backend.rs:409-412`). Hw: all 19 [M] `v3dx_format_table.c:118-137,75` |
| Float RT: RGBA16F, R16F, R32F | yakogl draws them: `pbe_emit_words` [C] `src/pvr/backend.rs:918`. Hw has F16, F16x4, F32 [M] `pvr_arch_formats.c:124,137,140` | yakogl draws them [C] `src/vivante/backend.rs:267-269`. Hw: HALTI2 (met), 128-bit with HALF_FLOAT [M] `etnaviv_screen.c:500-505`, `H:19026` | **yakogl refuses** all three [C] `src/v3d/state.rs:31-33`. Hw has all six [M] `v3dx_format_table.c:90-101` |
| RG16F / RGB16F, RG32F, RGBA32F, R11F_G11F_B10F, RGB10_A2 | `is_color_format` [C] `src/gl/framebuffer.rs:506-536` lists RGBA16F, RGB16F, RG16F, R16F and R32F, and RGB10_A2 (ES 3.0 build only, stored as RGBA8: no back end has a 10-bit target). RG32F, RGBA32F and R11F_G11F_B10F are not colour-renderable names today. Hw: RG32F, RGBA32F, R11F_G11F_B10F and RGB10_A2 are `R` formats [M] `pvr_arch_formats.c:143,149,150,115` | same GL-side gap. Hw: float RTs with HALTI2; whether R11F_G11F_B10F and RGB10_A2 render was not traced through `gpu_supports_render_format` [U] | same GL-side gap. Hw renders all four [M] `v3dx_format_table.c:71,96,101,146` |
| sRGB render (SRGB8_ALPHA8) | **yakogl draws it**: `SRGB_RT` is true on the PowerVR [C] `backend.rs:432` (the PBE's gamma on store, docs/es3-srgb.md). Hw: RGBA8 sRGB is `_TR_` [M] `pvr_arch_formats.c:105`; PBE gamma `pbestate.xml:267-268` | refused by name (`SRGB_RT` false). Hw: sRGB RT needs HALTI3 (met) [M] `etnaviv_screen.c:497-498` | refused by name. Hw does it [M] `v3dx_format_table.c:63,67,250-259` |

**Sampling.** The GL core stores an ES 3.0 sized texture in the first
storage class (`GLES_TEXFMT_*`, [C] `src/backend.rs:84-133`) that the build
samples (`sampled_texfmts`, `:227-238`); a format with none is refused by
name. The masks today ([C] `backend.rs:158-209`; `cargo test --test
es3_caps -- --nocapture` prints them per build):

| Class | PowerVR (`PVR_SAMPLED_TEXFMTS`, `:181-195`) | GC7000UL (`GC_SAMPLED_TEXFMTS`, `:198`) | V3D 4.2 (`V3D_SAMPLED_TEXFMTS`, `:202-209`) |
|---|---|---|---|
| RGBA8, L8, A8, LA8, R32F | sampled | sampled | sampled |
| SRGB8_A8 | sampled | sampled | sampled |
| RGB565, RGBA16F, R16F | sampled | sampled | **not sampled** ([C] `v3d/texture.rs:117-123` refuses them). Hw has them [M] `v3dx_format_table.c:81,100,90` |
| R11F_G11F_B10F, RGB9_E5, RGB10_A2UI, RGBA8_SNORM | sampled | not sampled. Hw: snorm with HALTI1, the rest per the format table [M] `etnaviv_screen.c:394-449` (the table was not walked row by row) | not sampled. Hw has all four [M] `v3dx_format_table.c:146,147,75,69` |
| RGB10_A2 | **not sampled** on purpose: the descriptor decodes it, but ES 3.0 makes GL_RGB10_A2 renderable and mipmappable, so it stays in RGBA8 (two bits lost) until the PowerVR renders the class ([C] `backend.rs:177-180`) | not sampled | not sampled |
| RGBA8UI/I, RGBA16UI/I, RGBA32UI/I (integer samplers) | sampled (an `isampler2D` / `usampler2D`; other integer sampler dimensions are refused by name at compile time, [C] `backend.rs:172-175`) | not sampled. Hw: integer sampling with HALTI2, 128-bit integer with HALTI5, both met [M] `etnaviv_screen.c:419-429` | not sampled. Hw has all [M] `v3dx_format_table.c:122-123,129-130,136-137` |
| RGBA32F | sampled | not sampled | not sampled. Hw has it [M] `v3dx_format_table.c:101` |
| ETC2/EAC (`ETC2_RGB`, `ETC2_RGBA`, `ETC2_RGB_PT`) | outside this mask: compressed uploads are kept as blocks (`compressed_format_info`, [C] `src/gl/texture.rs:2999-3006`, ETC1 and RGB8_ETC2 as `ETC2_RGB`, RGBA8_ETC2_EAC as `ETC2_RGBA`); the other seven ES 3.0 formats are not accepted | same | `v3d/texture.rs:117-123` refuses ETC2 today ([C]). Hw has all ten [M] `v3dx_format_table.c:159-168` |

32-bit float and integer *filtering*: ES 3.0 requires NEAREST only for
those, and yakogl enforces completeness accordingly ([C]
`docs/es3-sized-formats.md`, "Rules implemented").

## 5. Gap list (the `known_gaps()` of the test, plus the functional gaps)

Limits below the ES 3.0 minimum today. `cargo test --test es3_caps --
--nocapture` prints one `GAP` line per entry for the build, and `CLOSED`
for an entry that now meets its minimum.

| Limit | spec | PowerVR | GC7000UL | V3D | owner item |
|---|---|---|---|---|---|
| MAX_COLOR_ATTACHMENTS, MAX_DRAW_BUFFERS | 4 | 3 | 3 | 3 | mrt4 |
| MAX_SAMPLES | 4 | 1 | 1 | 1 | none in the plan |
| MAX_UNIFORM_BLOCK_SIZE | 16384 | 6144 | 6144 | 6144 | ubo-uniform |
| MAX_COMBINED_*_UNIFORM_COMPONENTS | 50048 / 50176 | 1536 | 1536 | 1536 | ubo-uniform |
| NUM_COMPRESSED_TEXTURE_FORMATS | 10 | 3 | 3 | 3 | formats-* |
| MAX_VARYING_VECTORS / _COMPONENTS / FRAGMENT_INPUT | 15 / 60 / 60 | 8 / 32 / 32 | ok (15 / 60 / 60) | 8 / 32 / 32 | USC measurement; V3D board run |
| MAX_VERTEX_OUTPUT_COMPONENTS | 64 | 32 | 60 | 32 | same (GC7000: 15 x 4) |

The three transform feedback limits are **not** gaps: 64 / 4 / 64 meet ES
3.0's 64 / 4 / 4 on every GPU (§2).

Features below ES 3.0 that no limit query shows: integer render targets on
the GC7000 and the V3D (int-rt-gc, int-rt-v3d); the sized sampling classes
on the GC7000 and the V3D (formats-gc, formats-v3d); V3D float render
targets and RGB565 / half-float sampling; sRGB rendering on the GC7000 and
the V3D; the RG32F / RGBA32F / R11F_G11F_B10F colour-renderable names and a
real 10-bit RGB10_A2 target on every GPU; MSAA render targets; the seven
other ETC2/EAC formats.

**Real hardware limits, not just work to do.**
- Transform feedback is hardware only on the V3D [M]. The PowerVR and the
  GC7000UL have none, and Mesa 26.2.1 offers none on either (§1.2, §1.3).
  yakogl's CPU capture covers all three; a GPU path on the V3D is an
  option, not a requirement.
- V3D 4.2: SNORM and RGB9_E5 are sample-only [M] (in ES 3.0 they are not
  colour-renderable either); 4 render targets and 4x MSAA are the caps [M].
- PowerVR: four 32-bit output registers a pixel (§1.3), so four ≤32-bit
  attachments fit, but three RGBA16F, or one RGBA32F plus others, do not.
  ES 3.0 only asks for four of the smallest kind.
  `tools/pco_shc/pco_shc.c:555-562` makes the driver drop the attachments
  that do not fit.
- GC7000UL: 4 render targets with HALTI2, because etnaviv keeps half of
  `num_rts` for float emulation [M]; 16 varyings with one slot for the
  position [M].

## 6. Other findings

- `src/gl/context.rs:141-152` says every draw buffer is written and that
  the GC7000 and V3D draw four targets, while `CAP_COLOR_ATTACHMENTS` is 3
  on all three GPUs ([C] `:139`). mrt4 reconciles them.
  `src/vivante/VIVANTE_NOTES.md:1136-1137` plans "CAP_COLOR_ATTACHMENTS
  3 -> 4, per GPU (the PowerVR stays 3)", which leaves the PowerVR below
  ES 3.0's minimum of 4.
- The V3D back end already accepts a 4-sample draw (`src/v3d/state.rs:36-39`,
  V3D's one multisample mode), while the GL core reports MAX_SAMPLES 1 on
  every GPU ([C] `state.rs:1251`).
- `GL_MAX_ELEMENT_INDEX` is exactly the minimum, 2²⁴-1 (`state.rs:1255`).
- `MAX_VIEWPORT_DIMS` has no minimum in Table 6.32 (only "as large as the
  visible surface"); the b6004e1 test required it to be ≥ MAX_TEXTURE_SIZE,
  which the spec does not say. The test prints it as a note.

## 7. The test

`tests/es3_caps.rs` creates a context on the host (a minimal OSAL and a
back end with no draw path; nothing in `src/` changes). Per build:

- without `report-es2`:
  - `every_es3_limit_meets_the_spec_minimum_or_is_a_listed_gap`: 35
    integer limits, LOD bias, server wait timeout and the two ranges. A
    limit below its minimum must be in `known_gaps()`; a listed one that
    now meets it prints `CLOSED` and passes. It also checks that
    glGetInteger64v agrees with glGetIntegerv, and that SEPARATE_COMPONENTS
    equals INTERLEAVED_COMPONENTS (the CPU capture's choice, §2).
  - `reported_limits_do_not_exceed_the_hardware_ceilings`: the ceilings of
    §1 with their Mesa `file:line`. A limit raised past a hardware ceiling
    fails here on purpose.
  - `print_the_format_support_this_build_reports`: the sampled classes and
    integer surfaces of §4, with two sanity asserts.
- with `report-es2`: `es2_report_answers_are_unchanged` pins the four
  groups of §2's "Under report-es2" per GPU. It fails if ES 3.0 work moves
  an ES 2.0 answer or answers an ES 3.0-only query in the ES 2.0 report.
  The varying work changes MAX_VARYING_VECTORS, which is an ES 2.0 limit:
  that item must update this pin on purpose.

No test is `#[ignore]`d: the test asserts only what yakogl does today, and
everything below the minimum is a listed gap, not a failing assert.

`src/lib.rs` refuses a build with no GPU feature, so there is no "default
features" run; the GPU rows of `tools/check.sh` are the builds.

Results on this branch (host, x86_64): `cargo test --no-fail-fast
--no-default-features --features <F> --test es3_caps --test es2_report_guard`

| Features `<F>` | es3_caps | es2_report_guard |
|---|---|---|
| `host-test,gpu-powervr` | 3 passed | 0 (report-es2 only) |
| `host-test,gpu-powervr,report-es2` | 1 passed | 13 passed, 1 ignored (not this item's) |
| `host-test,gpu-vivante` | 3 passed | 0 (report-es2 only) |
| `host-test,gpu-vivante,shaderc-online,report-es2` | 1 passed | 14 passed, 1 ignored |
| `host-test,gpu-v3d,shaderc-online,no-shader-table` | 3 passed | 0 (report-es2 only) |
| `host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2` | 1 passed | 14 passed, 1 ignored |

## 8. Handoff: the `tools/check.sh` rows to add (not added here; gate-green owns the file)

`cargo test` in the PowerVR and GC7000 rows of `CONFIGS` already runs every
test file, so `es3_caps` runs there with and without `report-es2`. The V3D
rows name their test files one by one. Add these next to the `es3_int_rt`
lines of the two V3D blocks (`tools/check.sh:1232-1233` and `:1260-1261`
at a63fcc5), in the same `cargo_test` form:

```sh
# The ES 3.0 limits against the spec minimum and the hardware ceilings
# (docs/es3-backend-caps.md); the ES 2.0 report below pins its answers.
step "test    $V3D_CONFIG (tests/es3_caps.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_CONFIG" --test es3_caps
```

```sh
step "test    $V3D_ES2_CONFIG (tests/es3_caps.rs)" \
  cargo_test --no-fail-fast --no-default-features --features "$V3D_ES2_CONFIG" --test es3_caps
```

Items that change a limit (mrt4, ubo-uniform, the varying work, formats-*)
edit `known_gaps()` or `ceilings()` in the same change, or leave the
`CLOSED` line to the next reader.

## 9. Why the first version was reverted, and what is different now

The revert commit (3f78982) records only "Vetoed by Claude's confirming
review after it was merged without a consultation"; the review text was
not kept. The redo (46d5f80) lists what was wrong in b6004e1, and the
review of the earlier attempt of this item (round 1 to 3) added the rest:

1. **A `tools/check.sh` edit.** b6004e1 added a V3D row to `check.sh`,
   which the item did not own. This version does not touch `check.sh`; §8
   gives the rows to gate-green.
2. **A wrong report-es2 claim, and a test cfg'd out of the ES 2.0 build.**
   b6004e1 said every ES 3.0 limit was `GL_INVALID_ENUM` under
   `report-es2`, and ran nothing there. At its base that was wrong (only
   the version queries and MAX_DRAW_BUFFERS were refused). Since 1bad575
   it is true for the ES 3.0-only pnames. This version states the current
   behaviour with lines (§2) and runs a report-es2 test that pins both the
   refusals and the ES 2.0 answers.
3. **A check the spec does not make**: MAX_VIEWPORT_DIMS ≥ MAX_TEXTURE_SIZE.
   Now a printed note (§6).
4. **Unverified Mesa claims.** Every hardware statement now has a Mesa
   26.2.1 `file:line`, re-read in this change, or an explicit [U].
5. **The earlier attempt of this item** (2faebed) was rejected for stale
   facts: it said transform feedback reported 0/0/0, cited `state.rs`
   lines from an older tip with a wrong offset note, and cited old
   `check.sh` lines. Every yakogl line here is re-read at a63fcc5, the
   transform feedback limits are 64 / 4 / 64 (§2, not a gap), and the test
   names symbols instead of lines.

The ES 2.0 answers do not change: this change adds a document and a test,
and touches no `src/` file and not `tools/check.sh`.

## 10. Still unverified, and what only newer Mesa has

- ASTC exposure on V3D 4.2 (the format table has the rows; the screen's
  gate was not traced).
- etnaviv: the per-format texture table (`etnaviv_format.c`) was read for
  ETC2 only, not row by row for each ES 3.0 sized format; packed-float and
  10-bit render targets (R11F_G11F_B10F, RGB10_A2) on the pixel engine;
  whether MAX_VERTEX_OUTPUT_COMPONENTS 64 is reachable with 16 output rows
  and a position slot.
- Newer Mesa only (not in 26.2.1, so not used in §1-§5; checked in Mesa
  main 776c1582, 26.3.0-devel). The redo (231c5aa) reported these from
  main at a9e75e34:
  - etnaviv reports 4 stream-output buffers for `HWTFB || halti >= 2`
    (`etnaviv_screen.c:300-301`) and has `etnaviv_nir_lower_xfb.c`: the
    vertex-shader emulation of transform feedback. A design reference for
    a GPU path on the GC7000UL;
  - `ETNA_MAX_SAMPLES` 4 (`etnaviv_internal.h:47`), and multisample
    textures need the texture descriptors (`etnaviv_screen.c:330-331`);
  - pvr's Vulkan device takes 3D size and array layers from
    `pvr_get_texture_extent_max_z` and `rogue_get_render_size_max_z`
    (`pvr_physical_device.c:665,667`) instead of 256.

  None of these changes a yakogl number.
- The PowerVR query path (`pvr_arch_cmd_query.c`), and what the stream-out
  store program (`pvr_arch_job_context.c:88-155`) means for a GPU
  transform feedback path.
- What the hardware can do that Mesa does not use.
