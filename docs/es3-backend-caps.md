<!-- SPDX-License-Identifier: Apache-2.0 -->
# ES 3.0 capabilities per back end: hardware, what yakogl reports, the gap

Plan #3 item `es3-caps` (host evidence). For every ES 3.0 requirement it
answers three questions per GPU: what the hardware does, what yakogl reports
today, and what is missing. The limit rows are enforced by
`tests/es3_caps.rs`, which queries every ES 3.0 `MAX_*` limit through the GL
API on the host in the `gpu-powervr`, `gpu-vivante` and `gpu-v3d` builds
(not under `report-es2`) and holds it against the ES 3.0.6 minimum; the
limits below the minimum are listed in the test's `known_gaps()` and in §3.

## 0. What this was checked against, and what could not be

**Sources.** The vendored `third_party/mesa-26.2.1` is only a subset (no
`v3d_screen.c`, no etnaviv, no `pvr_formats.c`), and the deps-cache copies
`mesa-26.2.1-src` / `mesa-hwdb` are empty. So the Mesa 26.2.1 tag
(`da14d65e`, the commit `third_party/mesa-26.2.1/UPSTREAM` pins) was cloned
sparse into `$CCFARM_DEPS_DIR/mesa-26.2.1-caps` (`src/gallium/drivers/etnaviv`,
`src/gallium/drivers/v3d`, `src/etnaviv/hwdb`, `src/imagination/vulkan`;
the rest comes from the vendored subset). **All citations are `path:line`
in Mesa 26.2.1** relative to its root, so they hold in either tree. `M/` is
Mesa 26.2.1's `src/` (`M/broadcom/...` = `src/broadcom/...`). `D/` is
`third_party/ti-img-rogue-ddk-25.2/`. Remaining **unverified** cells are
marked as such.

Two facts shape the whole document:

- **Hardware transform feedback exists only on the V3D.** etnaviv enables
  it only with `ETNA_FEATURE_HWTFB`, and the GC7000UL hwdb row has
  `gcFEATURE_BIT_HWTFB = 0`; the Imagination Vulkan driver has no
  transform-feedback code at all. PowerVR and GC7000UL need emulation.
- **Mesa's own Vulkan pvr driver reports ES 3.0-shaped limits** for the
  same silicon (4 colour attachments, 64 vertex-output / fragment-input
  components, 16 KiB uniform range, 256 layers), which are the numbers
  yakogl should aim for.

## 1. Hardware per GPU, with citations

### V3D 4.2 (Raspberry Pi 4)

| Capability | Hardware | Citation |
|---|---|---|
| Render targets | 4 (V3D < 7.1; 8 from 7.1) | `M/broadcom/common/v3d_limits.h:44` |
| MSAA | 4 samples max | `v3d_limits.h:41` |
| Texture / framebuffer size | 4096 (hardware could do 16384; Mesa caps at 13 mip levels to avoid a hang) | `v3d_limits.h:62-69` |
| Array layers | 2048 | `v3d_limits.h:77` |
| Varying inputs | 64 components per FS/VS/GS (16 vec4) | `v3d_limits.h:32-34` |
| Texture samplers | 24 | `v3d_limits.h:39` |
| Point size / line width | 512 / 32 | `v3d_limits.h:46-47` |
| Instance divisor | up to 0xffff | `v3d_limits.h:84` |
| Float render targets | R16F, RG16F, RGBA16F, R32F, RG32F, RGBA32F | `M/gallium/drivers/v3d/v3dx_format_table.c:90-101` |
| R11F_G11F_B10F | render + texture | `v3dx_format_table.c:146` |
| RGB9_E5 | texture only | `v3dx_format_table.c:147` |
| Integer render targets | R8/RG8/RGBA8 and 16-bit and 32-bit, I and UI | `v3dx_format_table.c:118-137` |
| RGB10_A2 / RGB10_A2UI | render + texture | `v3dx_format_table.c:71-75` |
| sRGB render | SRGB8_ALPHA8 (stored 16F, converted on tile load/store) | `v3dx_format_table.c:63,67,250-259` |
| SNORM (8-bit) | texture only (not colour-renderable, as in ES 3.0) | `v3dx_format_table.c:69,84,86` |
| Depth/stencil | D16, D32F, D24S8, S8; D32F_S8 emulated as Z32F + S8 | `v3dx_format_table.c:149-157` |
| ETC2/EAC | texture, all ten ES 3.0 formats (RGB8, sRGB8, RGB8A1, sRGB8A1, RGBA8, sRGBA8, R11, RG11 ±) | `v3dx_format_table.c:159-168` |
| Transform feedback | **hardware**: TF buffer, specs and "flush and count" packets, wait-for-TF, primitive-counts feedback | `M/broadcom/cle/v3d_packet.xml:440,560,700,724,730,735` |
| Occlusion queries | **hardware** counter | `v3d_packet.xml:822` |
| Primitive restart | **hardware** (index-list packets carry "Enable Primitive Restarts") | `v3d_packet.xml:565,580,595` |
| Stream-output caps | `max_stream_output_buffers = 4`, pause/resume | `src/gallium/drivers/v3d/v3d_screen.c:321,250` |
| Primitive restart / occlusion (pipe caps) | fixed index, hardware; non-fixed emulated | `v3d_screen.c:246-249` |
| MSAA | only `V3D_MAX_SAMPLES` (4) is a valid sample count; `texture_multisample` | `v3d_screen.c:239,423` |
| Shading language level | GLSL 3.30, **ESSL 3.10** | `v3d_screen.c:307,309` |
| Constant buffer 0 | 16 K floats = 64 KiB | `v3d_screen.c:164` |
| 3D / cube / array limits | `V3D_MAX_MIP_LEVELS`, `V3D_MAX_ARRAY_LAYERS` | `v3d_screen.c:328-330` |
| Render targets (caps) | `devinfo.max_render_targets` | `v3d_screen.c:332` |

### GC7000UL (i.MX8MP)

`H` = `src/etnaviv/hwdb/nxp/gc_feature_database.h`, the row for ChipID
0x7000 / revision 0x6204 / product 0x70003 (starts at line 18841: the
i.MX8MP row, as `src/vivante/VIVANTE_NOTES.md:13` records). `E` =
`src/gallium/drivers/etnaviv/etnaviv_screen.c`.

| Capability | Hardware | Citation |
|---|---|---|
| Generation | HALTI0..HALTI5 all set (HALTI5), 1 pixel pipe, 2 shader cores, 16 varyings, 0x140 constants | `H:19038,19058,19127,19152,19157,19172` (Halti0/1/2/3/4/5), `H:18851,18866` (cores, pixel pipes), `H:18876` (VaryingCount 0x10), `H:18853` (constants) |
| Render targets | `num_rts = 8` for HALTI2+, `max_render_targets = num_rts / 2 = 4` (half reserved for float emulation); independent blend with halti >= 5 | `E:774-781,299-306` |
| Integer / float colour render targets | pure-integer and float formats are renderable with HALTI2; R8 / R8I / R8UI need HALTI5; sRGB render needs HALTI3; 128-bit (RGBA32F/I/UI) render through the half-float pipe, integer 128-bit **sample** only on HALTI5 | `E:471-516` (render), `E:395-449` (sample) |
| Texture formats | sRGB / EXT formats HALTI0; snorm HALTI1; pure-integer and float HALTI2; ASTC off (`NO_ASTC = 1`); ETC1 hardware, ETC2 emulated in Mesa (`etnaviv_etc2.c`) | `E:395-449`, `H:19200`, `src/gallium/drivers/etnaviv/etnaviv_etc2.c` |
| Max texture size | 8192 with TEXTURE_8K, else 2048 | `E:970-971` |
| 3D / array textures | allowed from HALTI0; array layers = max texture size ("TODO: verify" in Mesa) | `E:283-284,287,378-392` |
| MSAA | feature present; 4x only (SmallMSAA = 1, so 2x is refused); etnaviv exposes `texture_multisample` only under a debug flag | `H:18958,19129`, `E:452-468,296` (the GL layer may still expose 4x renderbuffers: `texture_multisample` only gates multisample *textures*) |
| Transform feedback | **not in hardware**: `HWTFB = 0` in this row; etnaviv reports 0 stream-output buffers unless `ETNA_FEATURE_HWTFB` (a dEQP debug flag forces 4) | `H:19202`, `E:268` |
| Occlusion queries / conditional render | HALTI0 | `E:309-311` |
| Primitive restart (fixed index) | HALTI0 | `E:255` |
| Instancing | `vs_instanceid` and divisor need HALTI2 | `E:275-276` |
| Shadow samplers | `texture_shadow_map = true` | `E:281` |
| Uniform blocks | none in hardware: UBOs are advertised from HALTI2 and lowered to uniforms (`etnaviv_nir_lower_ubo_to_uniform.c`); `max_const_buffer0_size` 16384 "so state tracker enables UBOs" | `E:164-166,195-199` |
| Vertex-stage texture | reads only 4x4-tiled textures (repo finding) | `src/vivante/VIVANTE_NOTES.md:240-244` |
| Primitive modes | triangle strip only with BUG_FIXES8, line loop with LINE_LOOP | `E:335-346` |

### BXS-4-64 (AM62P, BVNC 36.53.104.796)

| Capability | Hardware | Citation |
|---|---|---|
| Identity | Rogue arch, 1 cluster, 1 raster pipe, 16x16 tiles | `M/imagination/common/device_info/bxs-4-64.h:31-33,95-96,99-100` |
| Colour outputs | `PVR_MAX_COLOR_ATTACHMENTS = 8` (PBE emit regs); only 2 USC output registers per pixel (`usc_min_output_registers_per_pix`, no `eight_output_registers`); the project assumes 4 x 32-bit outputs per pixel | `M/imagination/common/pvr_limits.h:35`, `bxs-4-64.h:77,104`, `M/include/hwdef/rogue_hw_defs.h:154`, `tools/pco_shc/pco_shc.c:555-562` |
| MSAA | `max_multisample = 4`, `isp_samples_per_pixel = 4` | `bxs-4-64.h:47,90,92` |
| PBE pack modes | U8..F32 in 1-4 channels, signed and unsigned, F16, F11F11F10, SE9995, A2R10B10G10, RGB565 etc. | `M/imagination/csbgen/rogue/pbestate.xml:60-122` |
| Texture formats | integer U/S 8/16/32 1-4 channels, F16/F32, F11F11F10, SE9995, depth/stencil (X8U24, ST8U24, ...), ETC2/EAC (ETC2_RGB, ETC2A_RGBA, punch-through, EAC_R11, EAC_RG11 ±), ASTC | `csbgen/rogue/texstate.xml:95-231`; `bxs-4-64.h:36` |
| sRGB | `GAMMA` bit plus `TWOCOMP_GAMMA` | `texstate.xml:234-262` |
| Image size | 14-bit width/height (16384), 11-bit depth/array (2048), 3D and cube types | `texstate.xml:248-256,274-277,309-311` |
| Compare (shadow) sampler | `cmp_mode` with all eight functions | `texstate.xml:52-61` |
| Primitive restart | hardware cut index (`cut_index_enable`, 32-bit `cut_index`); the ES 3.0 *fixed* index needs a per-index-type value programmed | `csbgen/rogue/vdm.xml:154,162,172-174` |
| Instancing | `instance_count`, `start_instance` in the index-list packets; `max_instances_per_pds_task = 32` | `vdm.xml:204-243`, `bxs-4-64.h:46,91` |
| Occlusion queries | hardware visibility results are implied (`ROGUE_OQUERY_ALIGN = 16`); the query path itself is unverified | `rogue_hw_defs.h:51` |
| Transform feedback | **none**: the Imagination Vulkan driver in Mesa 26.2.1 has no transform-feedback code (no file under `src/imagination/vulkan` mentions it), no stream-out flag in `bxs-4-64.h`, and the only DDK mention is the VDM context-store `STREAM_OUT` registers (context-switch state). Must be emulated (tf-design item) | `D/hwdefs/rogue/km/rgx_cr_defs_km.h:1774-1813` |
| Mesa Vulkan limits for this silicon | colour attachments 4, fragment outputs 4, vertex-output and fragment-input components 64, uniform-buffer range 16 KiB, 12 per-stage UBOs, 3D extent 256, array layers 256, ETC2 on, framebuffer/sampled colour samples 1 and 4, non-precise occlusion only | `src/imagination/vulkan/pvr_physical_device.c:739,678,655,677,626,635,622,624,253,735,740,256` |
| ETC2 formats | ETC2_RGB8, SRGB8, RGB8A1 map to hardware `ETC2_RGB` / `ETC2_PUNCHTHROUGHA` | `src/imagination/vulkan/pvr_formats.c:158-160` |
| Float / integer PBE | `SINT8/16/32`, `R16G16B16A16_SFLOAT`, `R32_SFLOAT`, 128-bit `R32G32B32A32_SINT/SFLOAT` and `B10G11R11` handled in the format switch | `pvr_formats.c:70-84,180-213,422-440` |
| Push constants / uniform alignment | 128 B push constants; Vulkan's UBO offset alignment 4 (a Vulkan figure, not a GL one) | `pvr_limits.h:41,71` |
| Point size / line width | 1..511 / 1..16 | `pvr_limits.h:43-49` |

## 2. ES 3.0 requirement by requirement

"Reports" is what the build answers today (the host test for limits;
the source for the rest). Gap codes: **OK** = meets ES 3.0; **GAP** = below
or missing (listed in §3); **?** = needs the missing Mesa file.

| # | ES 3.0 requirement | PowerVR (hw / reports / gap) | GC7000UL (hw / reports / gap) | V3D 4.2 (hw / reports / gap) |
|---|---|---|---|---|
| 1 | MAX_DRAW_BUFFERS / MAX_COLOR_ATTACHMENTS >= 4 | Mesa pvr exposes 4 (pvr_physical_device.c:739) from 8 PBE regs / **3** (`CAP_COLOR_ATTACHMENTS`, gl/context.rs:150) / GAP 3 -> 4 (mrt4) | 4 (`E:299-306`) / **3** / GAP (mrt4; backend already programs up to four, `mrt_targets`) | 4 (v3d_limits.h:44) / **3** / GAP (mrt4) |
| 2 | Integer colour render targets (R8I..RGBA32UI, RGB10_A2UI) | PBE has S/U 8/16/32 pack modes (pbestate.xml:60-122) / not renderbuffer-capable: `is_color_format` lacks them (gl/framebuffer.rs:455-478); only `glGetInternalformativ` knows the names / GAP (int-rt-gl, int-rt-pvr) | pure-integer and float RTs from HALTI2 (`E:471-516`), R8I/R8UI need HALTI5 (met) / same GL-core gap / GAP (int-rt-gc) | all integer RTs (v3dx_format_table.c:118-137) / same GL-core gap / GAP (int-rt-v3d) |
| 3 | Sized formats of table 3.13: texture-filterable | integer, F16/F32, F11F11F10, SE9995 texture formats exist (texstate.xml:95-192) / `sized_format` (gl/texture.rs:4326) accepts only RGBA8/RGB8/R8/RG8/SRGB*, R/RG/RGBA 16F/32F (32F stored as halves), depth; no SNORM, RGB10_A2, R11F_G11F_B10F, RGB9_E5, integer, RGB8/RGBA8 ETC2 sized names beyond the RGB/RGBA ETC2 pair / GAP (sized-formats-gl, formats-pvr) | snorm (HALTI1), sRGB (HALTI0), int/float (HALTI2) all met (`E:395-449`); no ASTC, ETC2 emulated / same / GAP (formats-gc) | texture formats all present (v3dx_format_table.c:63-168) / same / GAP (formats-v3d) |
| 4 | Colour-renderable sized formats | RGBA8, SRGB8_ALPHA8, RGBA16F, R16F, RG16F, R32F, RGB565, RGBA4, RGB5_A1, R8, RG8 reported renderable (framebuffer.rs:455-478); missing RGB10_A2, R11F_G11F_B10F (ES 3.0 via ext only), RG32F/RGBA32F (ext), integer | same / GAP vs integer rows | same / GAP |
| 5 | 3D and 2D-array textures; MAX_3D_TEXTURE_SIZE >= 256, MAX_ARRAY_TEXTURE_LAYERS >= 256 | 3D/array types exist, 2048 layers (texstate.xml:248-256,309-311) / 256 and 256 / OK; the PowerVR *shader* samples an array as one tall image (pvr-2darray item) – functional gap, not a limit | supported from HALTI0 (`E:378-392`) / 256, 256 / OK | 2048 layers (v3d_limits.h:77) / 256, 256 / OK |
| 6 | Shadow samplers (`sampler2DShadow`, compare mode/func) | `cmp_mode` in the sampler word (texstate.xml:52-61) / compare state tracked (gl/es3.rs:146, 608); no per-texel compare (PCF) in the PowerVR shader back end / GAP (pvr-shadow) | `texture_shadow_map` (`E:281`) / compare state tracked / check in the vivante shader item | v3d has it through the TMU (v3d_screen.c cap not separately cited) / ? |
| 7 | Transform feedback (MAX_TRANSFORM_FEEDBACK_* >= 64/4/4) | **none** (no TF in Mesa's pvr) → emulate / **0, 0, 0** / GAP (tf-design, tf-api, tf-capture) | **none**: `HWTFB = 0` in the hwdb row (`H:19202`) → emulate / 0 / GAP | **hardware**, 4 buffers (v3d_screen.c:321; v3d_packet.xml:724-735,560,700) / 0 / GAP |
| 8 | Occlusion queries (ANY_SAMPLES_PASSED[_CONSERVATIVE]) | non-precise occlusion only (pvr_physical_device.c:256), enough for ANY_SAMPLES_PASSED; `ROGUE_OQUERY_ALIGN` (rogue_hw_defs.h:51) / targets accepted by the API (gl/es3.rs:241-244), result not drawn from hardware / GAP (occlusion-query) | HALTI0 (`E:309`; `etnaviv_query_acc_occlusion.c`) / same / GAP | hardware counter (v3d_packet.xml:822) / same / GAP |
| 9 | MSAA 4x (MAX_SAMPLES >= 4) | 4 (bxs-4-64.h:47,90; pvr_physical_device.c:735) / **1** / GAP | 4x only (`H:18958,19129`, `E:452-468`) / **1** / GAP | 4 (v3d_limits.h:41; v3d_screen.c:423) / **1** / GAP |
| 10 | sRGB render targets | GAMMA bit in the PBE/texture state (texstate.xml:234-262) / SRGB8_ALPHA8 accepted as a renderbuffer format but "maps to plain RGBA8" (state.rs extension notes; no encode/decode) / GAP (functional) | unverified / same / GAP | SRGB8_ALPHA8 render with conversion on tile load/store (v3dx_format_table.c:63,67,250-259) / same / GAP |
| 11 | Primitive restart (fixed index) | cut index in VDM (vdm.xml:154,162,172-174) / supported in the draw path (gl/draw.rs:3830, 3926) / OK | HALTI0 fixed index (`E:255`) / supported (shared GL-core) / OK | packet flag (v3d_packet.xml:565,580,595) / same / OK |
| 12 | Instancing (divisor, gl_InstanceID) | `instance_count` (vdm.xml:204-243) / `max_instance_divisor` in the backend table / OK | instance id and divisor need HALTI2 (`E:275-276`) / OK | divisor up to 0xffff (v3d_limits.h:84) / OK | 
| 13 | Uniform blocks (MAX_UNIFORM_BLOCK_SIZE >= 16384, 12 + 12 blocks, 24 bindings) | no UBO hardware; blocks are lowered into the default uniform block / 6144, 12, 12, 24 / GAP in size (ubo-uniform) | same lowering / 6144 / GAP | same lowering / 6144 / GAP |
| 14 | Vertex uniform components >= 1024, fragment >= 896 | n/a / 1536 each / OK | 1536 / OK | 1536 / OK |
| 15 | MAX_VARYING_VECTORS >= 15, MAX_VERTEX_OUTPUT_COMPONENTS >= 64, MAX_FRAGMENT_INPUT_COMPONENTS >= 60 | Mesa pvr: 64 / 64 components (pvr_physical_device.c:655,677) / 8, 32, 32 / GAP | 16 varyings (`H:18876`), minus one slot for position (`E:965-967`) / 15, 60, 60 / GAP only VERTEX_OUTPUT (60 < 64) | 16 vec4 (v3d_limits.h:32-34) / 8, 32, 32 / GAP (raise after a board run) |
| 16 | NUM_COMPRESSED_TEXTURE_FORMATS >= 10 (ETC2/EAC) | ETC2/EAC formats in hw (texstate.xml:195-231) / 3 (ETC1, ETC2_RGB, ETC2_RGBA) / GAP | ETC1 hw, ETC2 emulated (`etnaviv_etc2.c`) / 3 / GAP | all ten (v3dx_format_table.c:159-168) / 3 / GAP |
| 17 | Texture/renderbuffer size >= 2048, cube >= 2048 | 16384 (texstate.xml:275) / 4096 / OK | ? / 4096 / OK | 4096 (v3d_limits.h:62) / 4096 / OK |
| 18 | MAX_COMBINED_TEXTURE_IMAGE_UNITS >= 32, TEXTURE_IMAGE_UNITS >= 16, VERTEX >= 16 | – / 32, 16, 16 / OK | – / 32, 16, 16 / OK | 24 samplers in hw (v3d_limits.h:39) / 32, 16, 16 / OK |
| 19 | MAX_ELEMENT_INDEX >= 2^24-1, PROGRAM_TEXEL_OFFSET -8..7, LOD bias 2, vertex attribs 16 | – / OK / OK | OK | OK |

## 3. Gap list (the `known_gaps()` of `tests/es3_caps.rs` plus the functional gaps)

Limits below the ES 3.0 minimum **today**, per build (`tests/es3_caps.rs`
prints them as `GAP` lines; run it with `-- --nocapture`):

| Limit | spec | PowerVR | GC7000UL | V3D | owner item |
|---|---|---|---|---|---|
| MAX_COLOR_ATTACHMENTS, MAX_DRAW_BUFFERS | 4 | 3 | 3 | 3 | mrt4 |
| MAX_SAMPLES | 4 | 1 | 1 | 1 | (new: MSAA item; not in the plan) |
| MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS / SEPARATE_ATTRIBS / SEPARATE_COMPONENTS | 64 / 4 / 4 | 0 | 0 | 0 | tf-api |
| MAX_UNIFORM_BLOCK_SIZE | 16384 | 6144 | 6144 | 6144 | ubo-uniform |
| MAX_COMBINED_FRAGMENT / VERTEX_UNIFORM_COMPONENTS | 50048 / 50176 | 1536 | 1536 | 1536 | ubo-uniform (derived from the block size) |
| NUM_COMPRESSED_TEXTURE_FORMATS | 10 | 3 | 3 | 3 | formats-* (ETC2/EAC R11/RG11/RGB8A1 sized names) |
| MAX_VARYING_VECTORS | 15 | 8 | ok (15) | 8 | usc measurement / V3D board run |
| MAX_VARYING_COMPONENTS | 60 | 32 | ok | 32 | same |
| MAX_FRAGMENT_INPUT_COMPONENTS | 60 | 32 | ok | 32 | same |
| MAX_VERTEX_OUTPUT_COMPONENTS | 64 | 32 | 60 | 32 | same (GC7000: position takes one of 16 io rows; 15 x 4 = 60 < 64) |

Not limits, but features below ES 3.0 (no query reports them): integer colour
targets (row 2); the table 3.13 sized formats (rows 3, 4); PowerVR 2D-array
sampling and shadow PCF (rows 5, 6); sRGB encode/decode on render and sample
(row 10); transform feedback and occlusion results (rows 7, 8); MSAA (row 9).

**Hardware that is a real limit, not just work to do:**
- Transform feedback is hardware only on the V3D. PowerVR (no TF in Mesa's
  pvr) and GC7000UL (`HWTFB = 0`) must emulate it (tf-design).
- PowerVR: two USC output registers per pixel bound MRT with wide formats
  (4 x RGBA32UI cannot be 4 colour outputs; ES 3.0 only guarantees 4 outputs,
  not wide ones, but the allocation must then refuse rather than miscompile).
- V3D 4.2: SNORM, RGB9_E5 are texture-only in hardware (matches ES 3.0, where
  they are not colour-renderable); 32-bit float *filtering* needs
  OES_texture_float_linear and is not an ES 3.0 requirement.
- GC7000UL: the per-format render/sample rules are in `E:395-516`
  (HALTI levels, all met); the per-format hardware *encodings* are in
  `etnaviv_format.c` / `etnaviv_translate.h`, which the formats-gc and
  int-rt-gc items should read next (`mesa-26.2.1-caps` in the deps cache).
  etnaviv has no ASTC here and emulates ETC2 (`etnaviv_etc2.c`).

## 4. Test

`tests/es3_caps.rs` creates a context on the host (a minimal OSAL and a
backend table with no draw path; no change to `src/`), queries 35 integer
limits, `MAX_TEXTURE_LOD_BIAS`, `MAX_SERVER_WAIT_TIMEOUT`, the line-width,
point-size and viewport ranges, checks `glGetInteger64v` agrees with
`glGetIntegerv`, and compares each with the ES 3.0.6 Table 6.32-6.34 minimum
(`AtLeast`, or `AtMost` for `MIN_PROGRAM_TEXEL_OFFSET` and
`UNIFORM_BUFFER_OFFSET_ALIGNMENT`). A limit below the minimum must be in
`known_gaps()`; one that is not fails. A listed limit that now meets the
minimum prints `CLOSED ... remove it from known_gaps()` and passes, so the
item that closes a gap may delete the list entry in its own change.

Run: `cargo test --no-default-features --features host-test,gpu-powervr --test es3_caps -- --nocapture`
(also `gpu-vivante`, and `gpu-v3d,shaderc-online,no-shader-table`). The file
is cfg'd out under `report-es2`, where these queries are `GL_INVALID_ENUM`.

## 5. Other findings

- `src/gl/context.rs:140-152` comments say GC7000 and V3D draw four targets
  and "all of them" are written, while `CAP_COLOR_ATTACHMENTS` is 3 on all
  three GPUs (and `src/vivante/VIVANTE_NOTES.md:1130` plans 3 -> 4 per GPU).
  The comment and the constant disagree; mrt4 should reconcile them.
- `GL_MAX_ELEMENT_INDEX` is 0xFFFFFF (2^24 − 1), exactly the minimum.

## 6. Still unverified

- V3D shadow-sampler, uniform-block and MAX_UNIFORM_BLOCK_SIZE rows (the
  v3d compiler and `v3d_uniforms.c` are in the sparse Mesa tree
  `$CCFARM_DEPS_DIR/mesa-26.2.1-caps`, not read yet).
- The PowerVR *per-format* texture/PBE flags beyond `pvr_formats.c:70-440`
  (the sparse tree has `pvr_arch_formats.c` too) and the PowerVR query
  implementation (`pvr_arch_cmd_query.c`).
- The BXS-4-64 USC output-register limit for wide MRT formats (the project
  assumes 4 x 32-bit outputs: `tools/pco_shc/pco_shc.c:555-562`;
  `pvr_arch_mrt.c` in the sparse tree decides it for Vulkan).
- Everything about what the hardware does that Mesa does not use.
