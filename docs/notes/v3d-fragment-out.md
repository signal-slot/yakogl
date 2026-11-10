<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: the Pi 4 `fragment_out.*` not-pass rows

Research, written before any code item. `docs/es3/pi4-gles3-fails.tsv` lists
158 `dEQP-GLES3.functional.fragment_out.*` rows that are not Pass on the
Pi 4 (images FR1 and FR2, run `0ff58329`, records in
`tools/cts/log/2026-10-31-pi4-gles3-a/FR1.console.txt` and `FR2.console.txt`).
This note groups them, names the cause of each group in today's code, says
which layer owns the fix, and designs the `src/v3d` part.

Already written, not repeated here: `docs/es3/srgb.md` (sRGB targets),
`docs/notes/v3d-int-clear.md` (the integer clear and the `f32_color_rb`
rule), `docs/notes/v3d-msaa-es3.md`, `docs/notes/v3d-mrt-sizes.md`.

Function names are the address; line numbers drift (yakogl at `434d2695`).
Mesa is 26.2.1. The vendored `third_party/mesa-26.2.1` holds only the v3d
format table; `v3d_program.c` and `nir_to_vir.c` were read in the full tree
of the deps cache (`mesa-pan/mesa-26.2.1`).

## What the test does (VK-GL-CTS `es3fFragmentOutputTests.cpp`)

- **Basic**: one 64x64 attachment per case; **array**: three attachments, a
  three-element output array; **random**: seeds 0-99, 2-4 attachments of
  128x64, each output a 30% chance of an array, each format drawn from the
  list of its class (the float list holds the fixed-point formats too).
- Every attachment is a **renderbuffer**
  (`glRenderbufferStorageMultisample` with 0 samples, then
  `glFramebufferRenderbuffer` at `GL_COLOR_ATTACHMENT0 + n`, then
  `GLU_EXPECT_NO_ERROR "After framebuffer setup"`).
- Float formats: RGBA32F, RGBA16F, R11F_G11F_B10F, RG32F, RG16F, R32F, R16F,
  and in the random list the fixed ones. A case whose format fails
  `glu::isSizedFormatColorRenderable` (which reads the context's extension
  string) throws **NotSupported "Unsupported attachment format"**.
- Judgment: dithering off, outputs clamped to the precision's range
  (lowp +-2, mediump +-16000, highp +-1e35), 32F compared exactly (0 ULP
  format term), half float 2^13 ULP of a float32 mantissa, R11F_G11F_B10F
  2^17/2^17/2^18. Integer and fixed formats compare exactly or to
  1/(2^bits-1) at most 8 bits.

The whole tsv set is the **float class** and nothing else. All 782 other
fragment_out rows Pass on the Pi 4: every integer and unsigned class
(`basic.int`, `basic.uint`, `array.int`, `array.uint`), every fixed format
(RGBA8, SRGB8_ALPHA8, RGB10_A2, RGBA4, ...), and the 16F float formats in
the basic group. The 115 + 43 failing rows below are therefore not mixed
integer/float-class trouble; they are 32-bit float and R11F_G11F_B10F
formats, one 16F format at attachment 1 and 2, and the fourth attachment.

## Groups, counts, causes, owners

Every group has the same shape of rows: 3 precisions (lowp, mediump, highp)
times 4 output types (`float`, `vec2`, `vec3`, `vec4`) is 12 rows per
format, 4 per precision.

| # | group | rows | status | cause (file:line) | fix lives in |
|---|-------|------|--------|-------------------|--------------|
| G1 | `basic.float.{r11f_g11f_b10f,r32f,rg32f,rgba32f}_*` (4 x 12) | 48 | NotSupported | `GL_EXT_color_buffer_float` is not in the extension string (`src/gl/state.rs` `base_extension_string!` lists `GL_EXT_color_buffer_half_float` only; `docs/es3-cts-failures.md` "Float targets: not claimed"), so dEQP's renderable query says no. Behind it: `is_color_format` (`src/gl/framebuffer.rs:602-632`) has no RG32F, RGBA32F or R11F_G11F_B10F, and `src/backend.rs` has no surface class for them (`GLES_SURF_*` ends at 26). R32F is in `is_color_format` and has a surface, but the program writes it as halves (G7 below) | src/gl (claim, surfaces, read-back), src/shaderc (32-bit write), src/v3d (rows) |
| G2 | `array.float.{r11f_g11f_b10f,r32f,rg32f,rgba32f}_*` (4 x 12) | 48 | NotSupported | the same four formats, same cause as G1 | as G1 |
| G3 | `random.{5,7,26,28,32,37,39,40,46,48,51,59,63,73,74,76,80,81,95}` | 19 | NotSupported | an attachment of one of the four G1 formats (the float list is 16 long, 4 of them are these) | as G1 |
| G4 | `array.float.rg16f_*` (12) | 12 | Fail, "Image comparison failed" | `fb_target_n` (`src/gl/framebuffer.rs:1232-1238`), the format of colour attachment 1.. handed to the back end, maps only GL_RGBA16F, GL_R16F, GL_R32F, the integers, sRGB and RGB10_A2; **GL_RG16F and GL_RGB16F fall to `_ => GLES_SURF_RGBA8888`**. Attachment 0 goes through `renderbuffer_surf_format` (`:719-721`, RG16F is RGBA16F there), so the basic cases pass. In an array case attachments 1 and 2 are 8-byte RGBA16F storage drawn as 4-byte RGBA8888 | src/gl (one match arm) |
| G5 | `random.{0,8,17,21,24,25,29,33,41,45,55,56,66,67,70,71,72,75,78,79,82,86,88,90,94,96}` | 26 | Fail, `glGetError() returned GL_INVALID_ENUM` at `es3fFragmentOutputTests.cpp:286` ("After framebuffer setup") | the random group takes 2-4 targets; the fourth is `GL_COLOR_ATTACHMENT3`, and `is_color_point` (`src/gl/framebuffer.rs:282`) accepts points below `max_color_attachments()` = `max_draw_buffers()` = `backend::draw_targets()` = **3** (`DRAW_TARGETS`, `src/backend.rs:763-780`, V3D arm 3, RC14 in `docs/es3-cts-failures.md`). Test `attachment past the cap` at `framebuffer.rs:4705` pins the error. About a third of 100 seeds draw four targets: 26 here, the rest of those are in G3 (NotSupported comes first) | src/backend.rs arm (V3D), then src/v3d for the 4th RT |
| G6 | `random.{11,27,31,43,64}` | 5 | Fail, "Image comparison failed" | **not confirmed.** The logs hold the verdict, not the formats the seed drew. Most likely G4: a 2-3 target case with RG16F at attachment 1 or 2, one of 16 float formats per target. Re-check after G4 is fixed; if any remains it is a new group | src/gl (as G4), to be confirmed |
| | total | **158** | | 48 + 48 + 19 + 12 + 26 + 5 | |

Per format, G1+G2 are 24 rows each of R11F_G11F_B10F, R32F, RG32F and
RGBA32F; per precision 8 rows of each (lowp, mediump, highp); per output
class all float. Mixed-class outputs (a float output next to an integer
one in one draw) are what the random group would produce; none of the 158
rows is one, and the passing random cases show the mixed draws work today.

### The R32F row is a second cause hiding behind G1

R32F is renderable in the GL and has a row in `state::rt_format`
(`src/v3d/state.rs:471`), so one would expect R32F rows to Pass once the
extension is claimed. They will not, for the highp rows. A float shader
output is always written as two `VFPACK` half pairs
(`src/shaderc/online/v3d/select.rs`, the colour loop of the fragment end,
`tlb_config_f16`), whatever the render target is. Mesa writes a target as
32-bit words (`TLB_TYPE_F32_COLOR`, one `MOV` per component to TLBU/TLB)
when the draw's key marks it `f32_color_rb`, and the key marks every
32-bit float format as well as the pure integer, 16-bit unorm and snorm
ones (`gallium/drivers/v3d/v3d_program.c:681-704`;
`broadcom/compiler/nir_to_vir.c:1953-2037`, the `f32_color_rb` branch).
Our integer path (`rt_int`, `tlb_config_32b`) is exactly that branch, chosen
by the shader's declared type, not by the target. A float output into a 32F
tile buffer therefore reaches it as f16: values beyond +-65504 become
infinity and the mantissa is 10 bits. dEQP's highp rows draw up to +-1e35
and compare 32F exactly. mediump and lowp (+-16000, +-2, 2^15 ULP slack)
probably pass; the highp rows (4 types per format and group) would not.

## How Mesa writes several outputs of mixed classes

One TLB write sequence per colour buffer in RT order, the class chosen by
that buffer's key bit, not by the others (`vir_emit_tlb_color_write` is
called for rt 0..7, the function above `emit_frag_end`):

- f16 target: `VFPACK` pairs, config `F16 | swap | vec size`, first word to
  TLBU unless the config is the default (RT 0, 3-4 components).
- f32 target (32F, pure integer, 16-bit norm, snorm): one `MOV` per
  component, config `TLB_TYPE_F32_COLOR | (n - 1)`, first word to TLBU.
  16-bit norm and snorm also add `norm_16` / `snorm` key bits that
  pre-scale the value; the 32F formats need none.

Our program already does this per RT for the integer case
(`select.rs`, `p.rt_int`, `p.rt_comps`), so a draw of an integer RT next to
a float RT is correct today (the 782 passing rows, and
`tests/es3_int_rt_v3d.rs`). What is missing is a third arm keyed by the
target, not the shader.

The tile buffer: a render target's internal bpp is 32, 64 or 128 and the
common config carries the maximum. The job builder already picks the tile
size from RT count and maximum bpp the way `v3d_choose_tile_size` does for
V3D 4.2 (`driver/src/v3d/v3d_frame.c:85`: index 0 for one RT, +1 for two,
+2 for three or four, +2 for 128 bpp). Three RGBA32F targets give index 4,
16x16 tiles, inside Mesa's table. No `src/v3d` code decides it.

## Design for the `src/v3d` part

Four pieces; the first two are `src/v3d` only, the third reaches a shaderc
entry, the fourth the constant in `src/backend.rs`. None may be merged
before its GL half, otherwise no draw reaches them.

1. **Rows** (`state::rt_format`, `src/v3d/state.rs`, and the output-format
   constants in `src/v3d/ffi.rs`, pinned by `ffi_layout_check.c`): three new
   surfaces, in the order the GL half numbers them.
   - R11F_G11F_B10F: `V3D_OUTPUT_IMAGE_FORMAT_R11F_G11F_B10F` (8), type
     `16F`, `64` bpp, clamp `NONE`, 4 bytes (`v3dx_format_table.c:146`, bpp
     `:253`, the same tile-buffer case as RGB10_A2 and RGBA16F).
   - RGBA32F: format 9, type `32F`, `128` bpp, clamp `NONE`, 16 bytes
     (`:101`, `:319-322`).
   - RG32F: format 10, type `32F`, `64` bpp, clamp `NONE`, 8 bytes
     (`:96`, `:323-326`).
   `clear_words` already keys on the internal type (`16F` packs halves,
   `32F` hands the words as they are), so the TLB clear needs no new arm;
   the unit test list at `state.rs` `clear_words_pack_as_the_tlb_clear_does`
   gets the three rows. R11F_G11F_B10F clears as halves, as RGBA16F, and
   the TLB packs at the store. `GlesDraw::clear_rgba` for it is a float
   colour clamped nowhere (ES 3.0 4.2.3: a float buffer's clear is not
   clamped).
2. **Blend and masks.** None of fragment_out blends. Mesa (and the ES
   3.0 + `EXT_color_buffer_float` text) do not require blending of 32F
   targets (that is `EXT_float_blend`); `state::job_state` should refuse
   by name "blending into a 32-bit float colour attachment" for G1's
   surfaces and leave the 16F ones as they are. Colour write masks work on
   the tile buffer per channel for every type.
3. **The float-to-32F write** (the R32F row of the section above, and
   RG32F/RGBA32F). The fragment program needs the second arm of
   `vir_emit_tlb_color_write` per render target, keyed by the target. The
   back end already has a draw-time program variant mechanism for samplers
   (`v3d_sampler_variants`, `crate::shaderc::online::backend`; the
   high-water mark logged in `src/v3d/backend.rs` around
   `variants_logged`), and `draw_call` is where the targets' formats are
   known (`extra_targets` returns them). The design: a `u8` mask
   `f32_rt` (bit n set when `rt_format(target n).internal_type` is `32F`,
   and bit set for slot 0 from `d.target_format`), passed with the program
   to the variant builder; a program with the mask 0 is the program of
   today and nothing already passing moves. Selection and the new arm are
   `src/shaderc/online/v3d` (owner: the shaderc plan); the back end side is
   computing the mask and asking for the variant, and refusing by name
   ("a variant of the program for 32-bit float targets could not be
   built") when the builder says no, as the sampler variants do.
   An alternative with no variant: write every float output of a program
   as 32-bit words and let a half-float target take them. It is not
   available: on V3D 4.2 `TLB_TYPE_F32_COLOR` into an 8-bit or 16F
   internal type is the integer narrowing, not a float conversion, which
   is why Mesa keys by format.
4. **Four draw buffers.** `DRAW_TARGETS` for the V3D arm 3 -> 4
   (`src/backend.rs`, RC14). The V3D side already takes four: `extra_targets`
   walks up to `GLES_MAX_TARGETS` (4), `Target::rts` is a `u8`, the compiler
   writes four (`MAX_RTS`), the job builder sizes the tile for four. The
   refusals a four-target draw can reach and that need a look on the host
   before the arm moves: "a colour output with no attachment slot"
   (`draw_call`, rt_mask against `rts`), "4x multisampling of several colour
   attachments", and "colour attachments of different sizes"
   (`docs/notes/v3d-mrt-sizes.md`); "a scissored or masked colour clear of
   several colour attachments" is gone (`v3d-mrt-clear`: one quad from
   `src/v3d/clear_mrt.rs`, up to its four RTs).
   dEQP's random cases clear nothing and use one size, so only the first
   matters for G5.

### What is not a `src/v3d` change

- The extension string, the three surface classes, the
  `is_color_format` / `renderbuffer_surf_format` / `fb_target_n` arms, the
  float read-back of the new classes (`read_float_row`,
  `src/gl/framebuffer.rs` around `:3220`, which knows RGBA16F and R16F and
  treats anything else as one 32-bit float), `glClearBuffer` on them, and
  the renderbuffer storage size in `renderbuffer_storage_args_ok`: all
  `src/gl` (plan #186 holds it).
- RGBA32F texture storage is halves today (`texture.rs`
  `GL_RGBA32F => ... GLES_TEXFMT_RGBA16F, stored as halves`). fragment_out
  uses renderbuffers only, so it does not need it; `fbo.render.*` on
  RGBA32F textures does, and so does anything that reads a rendered RGBA32F
  back as a texture. Keep the renderbuffer class independent of the
  texture's.
- The shader variant arm itself: `src/shaderc` (plan #192).

## Order and rows moved

| step | change | layer | rows it can move |
|------|--------|-------|------------------|
| 1 | RG16F / RGB16F at attachment 1.. | src/gl, one arm | G4 12, perhaps G6 5 |
| 2 | `DRAW_TARGETS` 4 for the V3D | src/backend.rs, host test first | G5 26 (and RC14's 76 `draw_buffers_indexed`, not counted here) |
| 3 | claim `GL_EXT_color_buffer_float` with R32F, RG32F, RGBA32F, R11F_G11F_B10F: surfaces, rows, read-back, clear | src/gl + src/v3d rows | G1+G2+G3 up to 115, 32F highp rows need step 4 |
| 4 | the f32 write keyed by target | src/shaderc + src/v3d mask | the highp rows of R32F, RG32F, RGBA32F: 3 formats x 4 types x 2 groups = 24, plus some of G3 |

Claim the extension (step 3) only when its formats render: claiming moves
every NotSupported row to a Pass or a Fail, and `fbo.completeness`,
`shaders.derivate` (15 on an incomplete RGBA32F/RGBA16F framebuffer) and
`fbo.render` rows change with it. Re-run the three dEQP groups on the Pi 4
after the claim (plan #320).

## Host tests

All through `GlesDraw` and the recorded job (`src/v3d/backend_tests.rs`,
`tests/es3_int_rt_v3d.rs` style), no board:

- `rt_format` rows: RG32F 10/32F/B64, RGBA32F 9/32F/B128, R11F 8/16F/B64;
  `clear_words` of each.
- A draw into RGBA32F + RG32F + R11F: the job's RT config, the maximum
  bpp 128, tile 16x16 (3 targets), the TLB config words of the fragment
  program (`f32_rt` mask 0b011 writes `TLB_TYPE_F32_COLOR` words for RT 0
  and 1, `VFPACK` for RT 2).
- A draw of a 32F float next to an `ivec4` and an RGBA8 target: all three
  classes in one program.
- A four-target draw (the arm at 4) with `rt_mask` 0b1111.

Open and not measurable on the host: the TLB's rounding of a 32F value
into an 11F/11F/10F word (dEQP allows 2^17 ULP, so any rounding passes);
a board run belongs to plan #320.
