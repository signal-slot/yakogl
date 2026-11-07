<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: what the Pi 4 back end does with each ES 3.0 colour format

Research note, item `v3d-format-matrix-note` of plan #330. One table per
question, one row per ES 3.0 sized colour format, every cell with the
`file:line` that decides it. Nothing here is measured on a board: it is what
the code at main `434d2695` does, read against Mesa 26.2.1's V3D 4.2 tables.
No code changed.

**Sources of the format list.** ES 3.0.6 table 3.13 (sized internal formats:
48 colour rows) and its colour-renderable list (R8, RG8, RGB8, RGB565, RGBA4,
RGB5_A1, RGBA8, RGB10_A2, RGB10_A2UI, SRGB8_ALPHA8 and the 18 integer
formats), plus the half and full float formats the GL core also accepts as
render targets (`R16F`, `RG16F`, `RGB16F`, `RGBA16F`, `R32F`:
`src/gl/framebuffer.rs:602-634`), which ES 3.0 makes renderable only through
`EXT_color_buffer_float` / `_half_float`. The ten compressed ETC2/EAC names
of table 3.14 are in section 6; the depth and stencil formats are not colour
formats and are not covered.

**Line numbers** are of this commit and drift; the names are the address.
`Mesa` lines are of the full 26.2.1 tree in the deps cache
(`mesa-pan/mesa-26.2.1/src/gallium/drivers/v3d/`); only
`v3dx_format_table.c` (and two headers) are vendored under
`third_party/mesa-26.2.1/`, so `v3d_formats.c`, `v3dx_state.c`, `v3dx_emit.c`,
`v3dx_rcl.c` and `v3d_program.c` cites are of the cache. The sRGB, RGB10A2 and
mixed-size designs are not repeated: `docs/es3/srgb.md`,
`docs/notes/v3d-rgb10a2.md`, `docs/notes/v3d-tex-layout-es3.md`,
`docs/notes/v3d-int-clear.md`, `docs/notes/v3d-mrt-sizes.md`,
`docs/notes/v3d-msaa-es3.md`.

**Cell marks.** `drawn` works as written (host-tested or by construction, the
cite says which). `R "…"` is refused by name; the string is quoted.
`G<n>` is a gap: the code runs or is silent, and what it does differs from
ES 3.0 or from Mesa; the gap is listed in section 7. `M<n>` is a difference
from Mesa's table, section 5. `n/a` means the format cannot reach the cell
(not renderable, so no render, clear or blend).

## 1. The path a format takes

A format reaches the V3D in two shapes, and they are different tables.

- **A texture** is stored in a *class* (`GLES_TEXFMT_*`): the first class of
  the format's storage list that `V3D_SAMPLED_TEXFMTS` holds
  (`src/backend.rs:243-258`, `src/gl/es3_formats.rs:80-95` for the 35 ES 3.0
  formats, `src/gl/texture.rs:5741-5800` `sized_format` for the older ones).
  The TMU reads the class (`src/v3d/texture.rs:192-247` `tmu_format`).
- **A colour render target** is a *surface* (`GLES_SURF_*`): the renderbuffer
  surface of a sized format (`src/gl/framebuffer.rs:719-735`
  `renderbuffer_surf_format`) or the surface of the attached texture's class
  (`src/gl/framebuffer.rs:926-960` `fb_target_format_n`, guarded by
  `attachment_storage`, `:451-540`). The back end turns a surface into a tile
  buffer row (`src/v3d/state.rs:445-492` `rt_format`; the same rows in C at
  `driver/src/v3d/v3d_job.c:151-244` `rt_internal`).

`rt_format_drawable` (`src/backend.rs:531-552`) decides which surfaces the GL
core lets through: every integer surface (`INTEGER_RT_FORMATS`,
`src/backend.rs:485-503`, all 19 on the V3D), sRGB (`SRGB_RT`,
`src/backend.rs:518`, true), RGB10_A2 (the class's sampled bit, set on the
V3D, `src/backend.rs:248`). So no ES 3.0 surface is refused by
`rt_format_drawable` on the V3D today. Two statements in the plan's context
are stale for the same reason: the sRGB draw is not refused (`SRGB_RT`, the
row `state.rs:463`), and the RGB10A2 gate is open (`03f4bbd9`; the row
`state.rs:468`, host test `tests/es3_formats_v3d.rs:1334`).

## 2. Per format: texture class, render surface, attachment

`Class` is the texture class on the V3D (`src/gl/es3_formats.rs` storage list,
first entry in `V3D_SAMPLED_TEXFMTS`; older names `texture.rs:5749-5760`).
`RB surface` is `renderbuffer_surf_format` (`framebuffer.rs:719-735`). `Tex
attach` is whether a texture of the format can be a colour attachment
(`attachment_storage`, `framebuffer.rs:451-540`; the integer rule is the doc
at `:396-403`, `integer_texture_internalformat` `:404`).

| Format | Class (texture store) | RB surface | Tex attach |
|---|---|---|---|
| R8 | RGBA8 (`texture.rs:5754`) | RGBA8888 (`framebuffer.rs:731`) | drawn; surface RGBA8888 (`:926-960`), alpha bits 0 (`:1002`); G, B not zeroed, G1 |
| RG8 | RGBA8 (`texture.rs:5755`) | RGBA8888 | drawn, as R8; G1 |
| RGB8 | RGBA8 (`texture.rs:5753`) | RGBA8888 | drawn; alpha bits 0 (`:1002`) |
| RGB565 | RGBA8 (`texture.rs:5753`) | RGBA8888 (not the RGB565 surface, which is the default framebuffer's only, `framebuffer.rs:731`) | drawn |
| RGBA4 | RGBA8 (`texture.rs:5752`) | RGBA8888; alpha bits 4 (`framebuffer.rs:1009`) | drawn; M3 |
| RGB5_A1 | RGBA8 (`texture.rs:5752`) | RGBA8888; alpha bits 1 (`framebuffer.rs:1008`) | drawn; M3 |
| RGBA8 | RGBA8 (`texture.rs:5752`) | RGBA8888 | drawn (also the only class with a render image, `v3d/backend.rs:1794-1805` `rt_texture`) |
| SRGB8_ALPHA8 | SRGB8_A8 (`texture.rs:5750`) | SRGB8_A8 (`framebuffer.rs:726`) | drawn (`framebuffer.rs:944`, `fb_target_format_n`) |
| SRGB8 | SRGB8_A8, alpha 255 (`texture.rs:5751`) | n/a (not renderable: `framebuffer.rs:505-507` refuses `t.format == GL_RGB` of the class; no RB name) | n/a: incomplete |
| RGB10_A2 | RGB10A2 (`es3_formats.rs:121-129`) | RGB10_A2 (`framebuffer.rs:729`) | drawn (`framebuffer.rs:425-426`, `:535`); `tests/es3_formats_v3d.rs:1334` |
| RGB10_A2UI | RGB10A2UI (`es3_formats.rs:130-138`) | RGB10_A2UI (`framebuffer.rs:659`) | drawn (`framebuffer.rs:439`) |
| R8I, R8UI, RG8I, RG8UI | RGBA8I / RGBA8UI (`es3_formats.rs:139-150`) | own surface (`framebuffer.rs:641-658`) | n/a for R/RG: incomplete, G2 (`framebuffer.rs:396-403`) |
| RGBA8I, RGBA8UI | RGBA8I / RGBA8UI | own surface | drawn (`framebuffer.rs:433-434`) |
| R16I, R16UI, RG16I, RG16UI | RGBA16I / RGBA16UI | own surface | incomplete, G2 |
| RGBA16I, RGBA16UI | RGBA16I / RGBA16UI | own surface | drawn (`framebuffer.rs:435-436`) |
| R32I, R32UI | RGBA32I / RGBA32UI, because classes 27/26 are in no mask (`es3_formats.rs:89-92`, `backend.rs:243-258`); M7 | own surface | incomplete, G2 |
| RG32I, RG32UI | RGBA32I / RGBA32UI (classes 29/28 in no mask); M7 | own surface | incomplete, G2 |
| RGBA32I, RGBA32UI | RGBA32I / RGBA32UI | own surface | drawn (`framebuffer.rs:437-438`) |
| RGB8I, RGB8UI, RGB16I, RGB16UI, RGB32I, RGB32UI | RGBA8I.. RGBA32UI (`es3_formats.rs:151-156`) | n/a (no RGB integer surface, not renderable in ES 3.0) | n/a: incomplete |
| R16F | R16F (`texture.rs:5758`) | R16F (`framebuffer.rs:722`) | drawn (`framebuffer.rs:532`) |
| RG16F | RGBA16F (`texture.rs:5760`) | RGBA16F (`framebuffer.rs:721`) | drawn as RGBA16F; G1 |
| RGB16F | RGBA16F (`es3_formats.rs:118`) | RGBA16F (`framebuffer.rs:721`) | drawn as RGBA16F; G1 |
| RGBA16F | RGBA16F (`texture.rs:5756`) | RGBA16F | drawn (`framebuffer.rs:531`) |
| R32F | R32F (`texture.rs:5759`) | R32F (`framebuffer.rs:723`) | drawn (`framebuffer.rs:533`); M1 |
| RG32F, RGB32F | RGBA32F (`es3_formats.rs:119-120`, `F32_ST` `:82`) | n/a: `is_color_format` lacks them, `GL_INVALID_ENUM` (`framebuffer.rs:2571`) | n/a: texture is not a render class, incomplete |
| RGBA32F | RGBA16F, stored as halves (`texture.rs:5757`, `rgba32f` flag `:5869`); M6 | n/a, as RG32F | n/a: judged as `GL_RGBA32F`, not renderable (`framebuffer.rs:526-530`) |
| R11F_G11F_B10F | R11G11B10F (`es3_formats.rs:100-108`) | n/a (not a colour-renderable name here; Mesa has a row, M5) | n/a |
| RGB9_E5 | RGB9E5 (`es3_formats.rs:109-117`) | n/a (not renderable in ES 3.0 either) | n/a |
| R8_SNORM, RG8_SNORM, RGB8_SNORM, RGBA8_SNORM | RGBA8_SNORM (`es3_formats.rs:96-99`, `SNORM_ST` `:80`) | n/a (not renderable) | n/a |

## 3. Per render surface: the tile buffer, clear, blend, read-back

One row per `GLES_SURF_*` the table in section 2 can reach. `Output / type /
bpp / bytes` are the `rt_format` row (`state.rs`); `clamp` is its fifth field.
The Mesa columns are `v3dx_format_table.c` (the `rt` column of the pipe
format of the same layout) and `get_internal_type_bpp_for_output_format`
(`v3dx_format_table.c:220-341` of the vendored file); `clamp` is
`v3dX(clamp_for_format_and_type)`, V3D 4.2 (`v3dx_rcl.c:396-407`).

**Clear.** *TLB* is the whole-target, all-channel clear
(`state::clear_words`, `state.rs:523-545`, put into the job's CLEAR_COLORS at
`v3d/backend.rs:6151-6170`). *Quad* is the scissored or masked one: the
float program `CLEAR_FS` (`v3d/backend.rs:816-818`, `gl_FragColor`, two f16
pairs to the TLB) or, for an integer RT 0, `CLEAR_FS_INT`
(`backend.rs:831-832`, four raw 32-bit words), chosen at `backend.rs:5546`
(`gles_surf_is_integer(target.format)`); design in `docs/notes/v3d-int-clear.md`.
Beside several attachments a quad clear is `R "a scissored or masked colour
clear of several colour attachments"` (`backend.rs:6063`), for every row.

**Blend.** The TLB blends in the tile buffer's type
(`state.rs:646-662`, `v3d_job.c:615-632`). Integer surfaces take no blend:
`d.blend = c.blend && !gles_surf_is_integer(...)` (`src/gl/draw.rs:3433`,
ES 3.0 4.1.8). One BLEND_CFG serves every RT, enabled for RT 0 only (G3).

**Read-back** is `glReadPixels` (`src/gl/framebuffer.rs:2969-3148`): float
surfaces by `read_float_row` (`:3184`, RGBA/UNSIGNED_BYTE clamped or
RGBA/FLOAT), integer ones by `read_int_row` (`:3255`, RGBA_INTEGER with
INT / UNSIGNED_INT only, `:2997-2999`), every other surface as raw bytes
(`:3105-3122`) with the RGB565 expansion and the BGRA swap after
(`:3123-3145`).

| Surface (formats) | Output / type / bpp / bytes / clamp | Mesa rt / type / bpp / clamp | Clear: TLB / quad | Blend | Read-back | vs Mesa |
|---|---|---|---|---|---|---|
| RGBA8888 (R8, RG8, RGB8, RGB565, RGBA4, RGB5_A1, RGBA8) | RGBA8 / 8 / 32 / 4 / none (`state.rs:436-442`) | RGBA8 (R8, RG8, ABGR4444, ABGR1555, BGR565 for the narrow ones, `v3dx_format_table.c:65,77,79,81,83,85`) / 8 / 32 / none | TLB one RGBA8 word (`clear_words` `:544`, the `_` arm); quad float `CLEAR_FS` (drawn) | TLB, 8-bit (drawn; `v3d_format_supports_tlb_resolve_and_blend` true for type 8, `v3d_formats.c:148-163`) | raw bytes (drawn). R8/RG8: green and blue as drawn, not 0, G1; alpha bits 0 -> alpha 1 (`state.rs:670-671`, `backend.rs:2683`) | type and bpp equal for all seven; output format and bytes differ for R8, RG8 (Mesa R8/RG8, 1 or 2 bytes), RGBA4, RGB5_A1 (Mesa 2 bytes, 4/5-bit quantised): M3 |
| RGB565 (default framebuffer only) | BGR565 / 8 / 32 / 2 / none (`state.rs:457`) | BGR565 / 8 / 32 / none (`v3dx_format_table.c:81`) | TLB, same 8-bit word (`state.rs:544`); quad float | TLB | expanded to RGBA8 (`framebuffer.rs:3123-3134`) | equal |
| SRGB8_A8 (SRGB8_ALPHA8) | SRGB8_ALPHA8 / 16F / 64 / 4 / norm (`state.rs:463`) | SRGB8_ALPHA8 / 16F / 64 / norm (`v3dx_format_table.c:250-261`, R8G8B8A8_SRGB `:67`) | TLB linear halves, clamped to [0, 1] (`state.rs:529-539`); quad float | TLB in linear, 16F (`docs/es3/srgb.md` 5.2); host tests `tests/es3_srgb.rs` (owned by `v3d-srgb-draw`, not read here) | raw stored (encoded) bytes (`framebuffer.rs:3105`); `docs/es3/srgb.md` | equal |
| RGB10_A2 (RGB10_A2) | RGB10_A2 / 16F / 64 / 4 / none (`state.rs:468`) | RGB10_A2 / 16F / 64 / none (`:71`, `:250-261`) | TLB halves clamped to [0, 1] (`state.rs:529-539`); quad float, colour not clamped, G4 | TLB, 16F (`docs/notes/v3d-rgb10a2.md:137`) | G5: raw 4 bytes of the 10/10/10/2 word as RGBA8 (`framebuffer.rs:3105`), and RGBA/UNSIGNED_INT_2_10_10_10_REV is not accepted (`:2999`) | equal |
| RGBA16F (RGBA16F, RGB16F, RG16F) | RGBA16F / 16F / 64 / 8 / none (`state.rs:469`) | RGBA16F / 16F / 64 / none (`v3dx_format_table.c:100`, `:250-261`); RG16F is its own row (`:95`) | TLB halves (`clear_words` `:529-539`, unclamped); quad float | TLB, 16F | `read_float_row` (`:3220-3228`) all four halves; RGB16F alpha and RG16F blue/alpha as stored, G1 | type and bpp equal; RG16F bytes 8 against Mesa's 4: M4 |
| R16F | R16F / 16F / 64 / 2 / none (`state.rs:470`) | R16F / 16F / 64 / none (`:90`, `:263-270`) | TLB halves; quad float | TLB, 16F | `read_float_row` (`:3224`) red, others 0,0,1 | equal |
| R32F | R32F / 32F / 32 / 4 / none (`state.rs:471`) | R32F / 32F / 32 / none (`:91`) | TLB four words as they are (`state.rs:543`, exact); quad float program writes f16, M1 | TLB blend over a 32F tile buffer, M2 | `read_float_row` (`:3226`) | equal row; M1, M2 |
| R8I | R8I / 8I / 32 / 1 / int (`state.rs:472`) | R8I / 8I / 32 / int (`:118`, `:236-240`) | TLB one word, low byte of each raw word (`state.rs:526-528`); quad `CLEAR_FS_INT` | none (`draw.rs:3433`) | `read_int_row` (`framebuffer.rs:3255`), RGBA_INTEGER + INT | equal |
| R8UI | R8UI / 8UI / 32 / 1 / int (`:473`) | R8UI / 8UI / 32 / int (`:119`) | as R8I | none | RGBA_INTEGER + UNSIGNED_INT | equal |
| RG8I, RG8UI | RG8I / RG8UI, 8I / 8UI, 32, 2 (`:478-479`) | RG8I / RG8UI, 8I / 8UI, 32 (`:120-121`) | as R8I | none | as above | equal |
| RGBA8I, RGBA8UI | RGBA8I / RGBA8UI, 8I / 8UI, 32, 4 (`:484-485`) | same, 32 (`:122-123`) | as R8I | none | as above | equal |
| R16I, R16UI, RG16I, RG16UI | R16I.. , 16I / 16UI, **32**, 2 or 4 (`:474-475`, `:480-481`) | same, 32 (`:125-130`, `:272-291`) | TLB two words, low 16 bits of each (`state.rs:540-542`); quad `CLEAR_FS_INT` | none | as above | equal |
| RGBA16I, RGBA16UI | RGBA16I / RGBA16UI, 16I / 16UI, 64, 8 (`:486-487`) | same, 64 (`:129-130`, `:272-275`, `:282-285`) | as above | none | as above | equal |
| R32I, R32UI | R32I / R32UI, 32I / 32UI, 32, 4 (`:476-477`) | same, 32 (`:132-133`, `:293-318`) | TLB the words as they are (`state.rs:543`); quad `CLEAR_FS_INT` | none | as above | equal |
| RG32I, RG32UI | RG32I / RG32UI, 32I / 32UI, 64, 8 (`:482-483`) | same, 64 | as R32I | none | as above | equal |
| RGBA32I, RGBA32UI | RGBA32I / RGBA32UI, 32I / 32UI, 128, 16 (`:488-489`) | same, 128 | as R32I | none | as above | equal |
| RGB10_A2UI | RGB10_A2UI / 16UI / 64 / 4 / int (`:491`) | RGB10_A2UI / 16UI / 64 (`:75`, `:282-285`) | TLB as 16UI, two words; quad `CLEAR_FS_INT` | none | `read_int_row` packed word split to four (`framebuffer.rs:3249-3252`) | equal |
| Formats with no render surface: SRGB8, the RGB integers, R8_SNORM.., RG32F, RGB32F, RGBA32F, R11F_G11F_B10F, RGB9_E5 | n/a | R11F_G11F_B10F has a row (16F / 64, `:146`, `:250-261`); RG32F and RGBA32F rows are 32F / 64 and 32F / 128 (`:96`, `:101`, `:319-326`); the others have none | n/a | n/a | n/a | M5 |

The integer rows are pinned by `rt_format` host tests in `src/v3d/state.rs`
and, for the clear quad of one 8-bit, one 16-bit and one 32-bit class
(scissored, masked, both, and the whole-target TLB case), by
`tests/es3_int_rt_v3d.rs:746-807`. `typed` jobs (every row but the first two)
refuse at the job builder what the tables lack
(`driver/src/v3d/v3d_job.c:310-324`), and 4x is refused for all of them
(`R "4x multisampling of an ES 3.0 colour format"`, `v3d/backend.rs:6057`;
design `docs/notes/v3d-msaa-es3.md`). A typed job whose smaller tiles
(64- and 128-bpp tile buffers, `v3d_frame_tiling_bpp`) outnumber what the
pools hold is `R "a 64- or 128-bpp colour target with more tiles than the job
pools hold"` (`backend.rs:2612-2614`, limit `MAX_JOB_TILES = 64 * 64`,
`:440`). Which target sizes that is per surface is arithmetic of the tiling
function, not enumerated here; it concerns the 64-bpp surfaces (SRGB8_A8,
RGB10_A2, RGBA16F, R16F, RG32I/UI, RGBA16I/UI, RGB10_A2UI) and the 128-bpp
ones (RGBA32I/UI) only.

## 4. Per texture class: the TMU

`TMU type` is `V3D_TEXTURE_DATA_FORMAT_*` and `swizzle` the record's, both from
`tmu_format` (`src/v3d/texture.rs:192-247`, the rows listed in its comment at
`:211-229`). `Ret` is the TMU output size (`gles_texfmt_return_32`,
`src/backend.rs:264-266`, `texture.rs:262-264`): 32-bit for R32F, RGBA32F,
RGBA32UI, RGBA32I, else 16-bit. `Sampler` is the GLSL class the class is read
by (`src/backend.rs:231-242` doc of the mask; integer classes only by
`isampler2D`/`usampler2D`/`isamplerCube`/`usamplerCube`, NEAREST only,
`docs/es3/sized-formats.md`). The look-up shapes the class cannot be read with
(2D array, 3D, `texelFetch`, offsets, shadow) are refused by shape, not by
format: `R "a 2D array or 3D texture is not sampled yet"`
(`texture.rs:136`); layout of the layered images is
`docs/notes/v3d-tex-layout-es3.md`.

| Class (formats that store in it) | TMU type | Swizzle | Ret | Sampler | Mesa row (type, swizzle, ret) | vs Mesa |
|---|---|---|---|---|---|---|
| RGBA8 (R8, RG8, RGB8, RGB565, RGBA4, RGB5_A1, RGBA8) | RGBA8 (`texture.rs:246`, the `_` arm) | XYZW (`:246`) | 16 | sampler2D / Cube | R8 `X001`, RG8 `XY01`, RGBA8 `XYZW`, RGB565 `XYZ1`, RGBA4, RGB5_A1 (`v3dx_format_table.c:65,77,79,81,83,85`) | same texels as sampled (the GL core stores `(r,0,0,1)` etc. in the RGBA8 bytes); bytes 4 against 1 / 2: M3 |
| SRGB8_A8 (SRGB8_ALPHA8, SRGB8) | RGBA8 + the record's sRGB bit (`texture.rs:359`) | XYZW | 16 | sampler2D / Cube | `R8G8B8A8_SRGB` RGBA8 `XYZW` (`:67`); `R8G8B8X8_SRGB` `XYZ1` for SRGB8 (`:68`) | equal (SRGB8: alpha stored 255 where Mesa swizzles 1; same samples) |
| RGB565 EGLImage only | RGB565 (`texture.rs:231`) | XYZ1 | 16 | sampler2D | B5G6R5 `XYZ1` (`:81`) | equal |
| R16F | R16F (`texture.rs:230`) | `X001` | 16 | sampler2D | R16F `X001` 16 (`:90`) | equal |
| RGBA16F (RG16F, RGB16F, RGBA16F) | RGBA16F (`texture.rs:234`) | XYZW | 16 | sampler2D | RGBA16F `XYZW` 16 (`:100`); RG16F `XY01` (`:95`) | same samples; M4 |
| R32F (R32F; the GL core's depth textures) | R32F (`texture.rs:205-206`) | `X001`, or `XXX1` for a depth texture | 32 | sampler2D | R32F `X001` 32 (`:91`) | equal |
| RGBA32F (RG32F, RGB32F) | RGBA32F (`texture.rs:235`) | XYZW | 32 | sampler2D, NEAREST (`f32_texels`) | RGBA32F `XYZW` 32 (`:101`), RG32F `XY01` (`:96`) | same samples; GL_RGBA32F does not reach this class: M6 |
| R11G11B10F (R11F_G11F_B10F) | R11F_G11F_B10F (`texture.rs:232`) | XYZ1 | 16 | sampler2D | `XYZ1` 16 (`:146`) | equal |
| RGB9E5 (RGB9_E5) | RGB9_E5 (`texture.rs:233`) | XYZ1 | 16 | sampler2D | `XYZ1` 16 (`:147`) | equal |
| RGB10A2 (RGB10_A2) | RGB10_A2 (`texture.rs:236`) | XYZW | 16 | sampler2D; own class since `03f4bbd9` | `XYZW` 16 (`:71`) | equal. Stale text: `texture.rs:139-141` and `backend.rs:231-242` still say the class stays in RGBA8, S1 |
| RGB10A2UI (RGB10_A2UI) | RGB10_A2UI (`texture.rs:237`) | XYZW | 16 | usampler2D | `XYZW` 16 (`:75`) | equal |
| RGBA8_SNORM (R8_, RG8_, RGB8_, RGBA8_SNORM) | RGBA8_SNORM (`texture.rs:238`) | XYZW | 16 | sampler2D | RGBA8_SNORM `XYZW` (`:69`); R8 `X001` (`:84`), RG8 `XY01` (`:86`) snorm | same samples; bytes 4 against 1 / 2 / 4: M3 |
| RGBA8I, RGBA8UI (R8I.. RGBA8UI, RGB8I, RGB8UI) | RGBA8I / RGBA8UI (`:239-240`) | XYZW | 16 | isampler2D / usampler2D (+ Cube) | `X001` / `XY01` / `XYZW` 16 (`:118-123`) | same samples; bytes 4 against 1 / 2 |
| RGBA16I, RGBA16UI (R16.., RG16.., RGB16.., RGBA16..) | RGBA16I / RGBA16UI (`:241-242`) | XYZW | 16 | i/usampler2D | `X001` / `XY01` / `XYZW` 16 (`:125-130`) | same samples; bytes 8 against 2 / 4 |
| RGBA32I, RGBA32UI (R32.., RG32.., RGB32.., RGBA32..) | RGBA32I / RGBA32UI (`:243-244`) | XYZW | 32 | i/usampler2D | `X001` / `XY01` / `XYZW` 32 (`:132-137`) | same samples; bytes 16 against 4 / 8: M7 |
| ETC2 (4 names) | not sampled | n/a | n/a | n/a | rows exist (`:159-168` of the vendored file) | `R "a texel format the V3D does not sample (ETC2, or the RGB10A2 class the GL core keeps in RGBA8 here)"` (`texture.rs:139-141`): M8 |

Swizzle composition with `GL_TEXTURE_SWIZZLE_*` is `tmu_swizzle`
(`texture.rs:300`), as Mesa composes a view's (`v3dx_state.c:1031-1034`).
`return_32` and the program's `LodKey::ret32` must agree
(`texture.rs:145-151`): a mismatch is `R "a look-up compiled for the 32-bit
TMU output reads a texture of a 16-bit format"` or the 16-bit twin.

## 5. Where yakogl's row differs from Mesa's

Hardware-facing rows (internal type, bpp, clamp, output format, TMU type,
return size) of every format yakogl can render or sample agree with Mesa's
V3D 4.2 tables, line by line, except these. Swizzles agree wherever the class
is the same; the classes that fold narrow formats into four channels read the
same samples through `XYZW`.

- **M1 R32F colour is written as f16.** Mesa sets `f32_color_rb` for a 32-bit
  float colour buffer (`v3d_program.c:683-686`) and writes it with the 32-bit
  TLB config. yakogl's V3D program is "not keyed by the target's class, so a
  float output always writes f16 pairs" (`src/shaderc/online/v3d/select.rs:
  1818-1830`, `:2305-2325`, `Prepared::rt_int` `prepare.rs:217`). A float
  shader or the clear quad into an R32F target reaches the 32F tile buffer
  with half the precision; the TLB clear is exact (`state.rs:543`).
- **M2 Blend into a 32F tile buffer.** Mesa blends in the shader when any
  colour buffer's internal type is neither 8 nor 16F
  (`v3d_format_supports_tlb_resolve_and_blend`, `v3d_formats.c:148-163`;
  `v3d_fb_needs_soft_blend`, `v3dx_state.c:515-525`). yakogl excludes only
  integer surfaces (`draw.rs:3433`) and programs the TLB blend for R32F.
- **M3 8-bit-or-fewer formats are all RGBA8.** Mesa renders R8, RG8,
  ABGR4444, ABGR1555 and BGR565 (`v3dx_format_table.c:77,79,81,83,85`); yakogl
  stores and renders every one as RGBA8888 (`framebuffer.rs:731`,
  `texture.rs:5752-5755`). Internal type 8 / bpp 32 is the same, so tile
  size and blend are unchanged; what differs is the memory (4 bytes), the
  4- and 5-bit quantisation of RGBA4 and RGB5_A1 (`state.rs:663-669` says
  they render as RGBA8, the GL core quantises the clear) and that the
  channels a narrow format lacks are stored (G1).
- **M4 Two-channel half floats are four.** RG16F stores as RGBA16F (8 bytes;
  Mesa's RG16F row is 4, `:95`); the hardware type and bpp are equal.
- **M5 Mesa renders formats yakogl calls not renderable.** `R11F_G11F_B10F`
  (16F, 64 bpp, `:146`), `RG32F` (32F / 64, `:96`) and `RGBA32F` (32F / 128,
  `:101`) have rows in Mesa; the GL core has no surface for them
  (`framebuffer.rs:602-634`). They are `EXT_color_buffer_float` formats, not
  ES 3.0 core ones.
- **M6 GL_RGBA32F is held as halves although the V3D samples RGBA32F.**
  `sized_format` maps `GL_RGBA32F` to the RGBA16F class
  (`texture.rs:5757`); `RG32F` and `RGB32F` reach the RGBA32F class through
  `es3_formats` (`:119-120`). A `GL_RGBA32F` texture on the V3D loses 13
  mantissa bits and the range above 65504, where Mesa's
  R32G32B32A32_FLOAT is RGBA32F 32-bit (`:101`) and the TMU row and the
  `ret32` plumbing exist here already.
- **M7 R32I/UI and RG32I/UI use the four-channel class.** Classes 26-29 exist
  in the GL core (`backend.rs:147-150`) and Mesa has R32 / RG32 rows
  (`:132-135`) with return size 32; the V3D mask omits them
  (`backend.rs:243-258`) so a one-channel 32-bit integer texture takes 16 bytes
  a texel. Samples are equal.
- **M8 ETC2 is not sampled.** Mesa has TMU rows for ETC2 RGB8, RGB8A1, RGBA8
  EAC, R11 and RG11 (`:159-168` of the vendored file). yakogl stores the
  linear ETC2 names everywhere (`texture.rs:3403-3412`) and refuses the draw
  that samples one; the two sRGB names are not stored at all on the V3D
  (`texture.rs:3406`: `sampled_texfmts()` has no bit), so their upload is
  refused. R11/RG11 EAC and the punch-through names have no GL-core class.
  The V3D `etc1_decoded_at_upload` comment (`v3d/backend.rs:6426-6429`) is
  right about ETC1 (no Mesa row) but ETC2 does have one.

## 6. ETC2 / EAC (table 3.14)

| Name | GL-core store | Draw that samples it |
|---|---|---|
| COMPRESSED_RGB8_ETC2 | ETC2_RGB class (`texture.rs:3377`) | `R "a texel format the V3D does not sample (ETC2, ...)"` (`texture.rs:139-141`) |
| COMPRESSED_RGBA8_ETC2_EAC | ETC2_RGBA (`:3378`) | same |
| COMPRESSED_SRGB8_ETC2, COMPRESSED_SRGB8_ALPHA8_ETC2_EAC | not stored on the V3D (`:3403-3412`) | the upload is refused |
| COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2, SRGB8_PUNCHTHROUGH_ALPHA1_ETC2, R11_EAC, SIGNED_R11_EAC, RG11_EAC, SIGNED_RG11_EAC | no class (`compressed_format_info`, `:3374-3384`) | the upload is refused |
| GL_ETC1_RGB8_OES | decoded to RGBA8 at upload (`texture.rs:3935-3938`, `v3d/backend.rs:6429`) | drawn as RGBA8 |

## 7. Gaps (code runs or is silent; ES 3.0 or Mesa differs)

- **G1 Channels a format lacks are stored.** `R8`, `RG8`, and the RGB/RG/R
  half-float surfaces keep green, blue (and alpha) as drawn: the colour mask
  trims only alpha, `colormask &= 0x7` when the target has no alpha
  (`state.rs:670-671`, bits from `framebuffer.rs:996-1011`). ES 3.0 reads
  `(r, 0, 0, 1)` / `(r, g, 0, 1)`. The comment at `framebuffer.rs:996-1001`
  says the same of the PBE's swizzle. Pinned by no V3D test; whether the
  dEQP cases compare the unwritten channels is not known from the code.
- **G2 R / RG / RGB integer textures cannot be attached.** Only the
  four-channel integer classes are render targets
  (`framebuffer.rs:396-403`): `GL_R8I`..`GL_RG32UI` textures are incomplete
  while their renderbuffers draw. dEQP `fbo.render.texture.*` and
  `fragment_out` on the integer R / RG formats reach it.
- **G3 One blend for one render target.** BLEND_ENABLES is `mask = 1`
  (`v3d_job.c:621-623`) and BLEND_CFG's mask is 0xf; Mesa enables every RT
  when RT 0 blends without independent blend (`v3dx_state.c:151-155`,
  `v3dx_emit.c:528-563`) and splits BLEND_CFG between RTs with and without
  alpha (`:540-558`, `blend_dst_alpha_one`). yakogl takes the alpha-less
  rule from RT 0 only (`state.rs:647`). A blended draw to RT 1..3 is
  unblended, and a framebuffer of an alpha-less and an alpha-bearing RT
  blends one of them with the wrong destination-alpha factor.
- **G4 The clear quad does not clamp a fixed-point clear colour.** The TLB
  path clamps for sRGB and RGB10_A2 (`state.rs:529-539`); the quad hands
  `clear_rgba` to the float program (`backend.rs:5546-5552`), and `glClearColor`
  keeps its argument unclamped (`src/gl/state.rs:390`, test `:1901`). The sRGB
  RT clamps what enters the TLB (`clamp` NORM, `state.rs:463`), so a quad
  clear of sRGB is covered; the RGB10_A2 row has clamp NONE (`state.rs:468`),
  so a scissored clear of it to a colour outside [0, 1] sends that colour to a
  16F tile buffer packed to 10 bits at the store, and what the pack does with
  an out-of-range half is not shown by anything in the tree.
- **G5 RGB10_A2 read-back.** `glReadPixels` of the 10/10/10/2 surface copies
  the four bytes as RGBA/UNSIGNED_BYTE (`framebuffer.rs:3105-3122`; no case
  for `GLES_SURF_RGB10_A2` among the row decoders `:3123-3145`), so the pixel
  is the packed word cut into bytes, not the 8-bit-converted colour; and the
  format ES 3.0 reports for it, RGBA with `UNSIGNED_INT_2_10_10_10_REV`, is
  `GL_INVALID_OPERATION` (`:2997-3000`, `implementation_color_read`
  `:1707-1730`). No test reads it (`tests/es3_formats_v3d.rs:1334` checks
  the job's output format only).
- **G6 Unwritten channels of RGB16F/RG16F read-back** are the stored halves
  (`framebuffer.rs:3220-3228`); same cause as G1, listed apart because the
  decoder, not the mask, would fix it.

## 8. Stale text (not defects)

- **S1** `src/backend.rs:231-242` ("Not ... `GLES_TEXFMT_RGB10A2`, which the TMU
  decodes too ... stays in RGBA8") and the refusal string at
  `src/v3d/texture.rs:139-141` ("or the RGB10A2 class the GL core keeps in
  RGBA8 here") describe the gate before `03f4bbd9` opened it; the class is in
  the mask at `backend.rs:248`. The string is a histogram key
  (`state::Refusal`), so changing it is a fixture-visible edit.
- **S2** The context of plan #330 lists the sRGB draw and RGB10A2 sampling as
  refused. Neither is, on this commit (section 1).

## 9. What is host-tested

`tests/es3_int_rt_v3d.rs` (integer RT rows and the quad clear, 3 formats),
`tests/es3_formats_v3d.rs` (RGB10_A2 sample record type 9 and attach, float
RT completeness and clears for RGBA16F, R16F, R32F, RGB565 / R16F / RGBA16F
sampling), `tests/es3_srgb.rs` (sRGB, `v3d-srgb-draw`), `src/v3d/state.rs`
unit tests (`rt_format`, `clear_words`), `src/v3d/texture.rs` unit tests
(`tmu_format` against `v3d_tex.c`, `the_srgb_class_is_the_rgba8_row_with_the_tmu_decode`
`:719`), `src/v3d/backend_tests.rs`. Nothing reads RGB10_A2 or the
narrow-format channels back (G1, G5), nothing blends into R32F or beside
another RT (G3, M2), and nothing draws a float shader into R32F and
compares bits (M1). No board has run any of the ES 3.0 classes
(`docs/es3-cts-failures.md:56`).
