# GC7000UL: ES 3.0 integer colour render targets (int-rt-gc)

Item int-rt-gc of plan #19 (ES 3.0), on top of int-rt-gl (the GL core's
half: the nineteen integer formats as renderbuffer storage and FBO targets,
`glClearBuffer{i,ui}v`, integer `glReadPixels`, and a refusal by name of a
format a back end has not learned, `backend::rt_format_drawable`).

## 1. What the GC7000UL now draws

`backend::INTEGER_RT_FORMATS` is `GC_INTEGER_RT_FORMATS` on the GC7000
(`src/vivante/backend.rs`): **13 of the 19** integer surfaces, those of 1, 2
and 4 bytes a pixel. Four more (RGBA16I/UI, RG32I/UI, 8 bytes a pixel) are
mapped but **held back** (`GC_INT_RT_64BPP`, below the table), and two
(RGBA32I/UI) the hardware has no format for (section 2). Per surface, the
pixel engine format (`pe_color_format`, written to `PE_COLOR_FORMAT`'s
`FORMAT_EXT` with `FORMAT_MASK`, or to `PE_RT_CONFIG_8`'s format for draw
buffers 1..3) and the fragment stage's output mode (`shader.rs`
`ps_color_output_mode`, `PS_CONTROL_EXT.OUTPUT_MODEn`):

| GL format | PE format (`state_3d.xml.h`) | output mode (`state_3d.xml.h`) | bytes |
|---|---|---|---|
| R8I / R8UI | `PE_FORMAT_R8I` 0x17 (`:104`) | `I8` 5 / `U8` 3 (`:135`, `:133`) | 1 |
| RG8I / RG8UI | `PE_FORMAT_G8R8I` 0x18 (`:105`) | `I8` / `U8` | 2 |
| RGBA8I / RGBA8UI | `PE_FORMAT_A8B8G8R8I` 0x19 (`:106`) | `I8` / `U8` | 4 |
| R16I / R16UI | `PE_FORMAT_R16I` 0x1a (`:107`) | `I16` 6 / `U16` 4 (`:136`, `:134`) | 2 |
| RG16I / RG16UI | `PE_FORMAT_G16R16I` 0x1b (`:108`) | `I16` / `U16` | 4 |
| *RGBA16I / RGBA16UI* (held) | `PE_FORMAT_A16B16G16R16I` 0x1c (`:109`) | `I16` / `U16` | 8 |
| R32I / R32UI | `PE_FORMAT_R32F` 0x14 (`:101`) | `UIF32` 2 (`:132`) | 4 |
| *RG32I / RG32UI* (held) | `PE_FORMAT_G32R32F` 0x15 (`:102`) | `UIF32` | 8 |
| RGB10_A2UI | `PE_FORMAT_A2B10G10R10UI` 0x1e (`:111`) | `A2B10G10R10UI` 1 (`:131`) | 4 |
| **RGBA32I / RGBA32UI** | **none**: refused, section 2 | | 16 |

**Why the 64-bit ones are held back.** Not a hardware limit: the PE has
their formats. But this back end renders every target LINEAR, and the PE's
64-bit LINEAR write is the open question of gl_test case 44
(`src/vivante/VIVANTE_NOTES.md`, "RGBA16F targets": an RGBA16F target stores
pixel x's colour two texels off on the i.MX8MP, the AM62P passes the case;
Mesa never renders a linear target on this core, `LINEAR_PE` is off unless
the `linear_pe` debug flag asks). RGBA16I/UI and RG32I/UI are 64-bit pixels
on the same path, so drawing them would most likely store misplaced pixels:
the silent garbage the GL core's refusal exists to avoid. They are refused by
name like RGBA32; once case 44's layout probe settles the 64-bit target
(tiled, or a pitch rule), `GC_INT_RT_64BPP` goes into `GC_INTEGER_RT_FORMATS`
and the test's `drawn()` follows. RGBA16F itself is still drawn, as before
this item.

All line numbers are `driver/src/vivante/hw/state_3d.xml.h`, the rnndb
header etnaviv generates its own from (MIT; README there). The test reads
the values from that header by name, so a wrong constant in
`src/vivante/regs.rs` fails it.

Why these, and why the same PE format for both signs: the enum has one
integer format per channel layout (`:104-111`), and the output mode carries
the sign. The output-mode rule is etnaviv's `translate_output_mode`
(`etnaviv_translate.h:446`, already cited in `shader.rs` for the R32F arm):
32-bit channels are `UIF32` before anything else; a pure integer format with
10-bit channels, or any integer format on a pre-HALTI5 core, is
`A2B10G10R10UI`; otherwise `I8`/`I16` for signed and `U8`/`U16` for unsigned
by channel width. This core is HALTI5 (`regs.rs`: model 0x7000 rev 0x6204;
the caps doc's hwdb row has Halti0..5 set).

**The 32-bit-channel formats.** The enum has no 32-bit integer PE format,
only `R32F` and `G32R32F`; `UIF32` is the mode whose name says it stores
"UI or F32": the register's 32 bits as they are. So R32I/UI and RG32I/UI are
drawn as R32F / G32R32F under `UIF32`, blending off (ES 3.0 4.1.8; the GL
core clears `GlesDraw::blend` for an integer target) and no saturate. This is
inferred from the enum and the mode, not traced through etnaviv's format table
(section 4): **for the board to confirm**.

**Memory order.** The rnndb names list channels from the high bits down, so R
is the low byte, half or word of every format above: the R,G,B,A packed
order the GL core keeps an integer surface in (`backend::gles_surf_int_layout`,
`gles_surf_bytes_per_pixel`, which the test checks against each PE format's
size). No swap on read-back (`rt_rgba8888_is_bgra` is about RGBA8888 only).

**The rest of the draw.** `PS_CONTROL.SATURATE_RTn` was set for every
surface of the RGBA8 shader class, which the integer surfaces are too (they
have no float variant); it is now cleared for them, as etnaviv saturates
`util_format_is_unorm` formats only (the "checked and identical" note in
`VIVANTE_NOTES.md`, "Two rules we had written as constants"). Dithering is
off for every draw already (`PE_DITHER` all ones).

**Blending, per draw buffer.** The GL core turns blending off for a draw
whose attachment 0 is integer (`GlesDraw::blend`). Draw buffers 1..3 took
RT0's blend word (`PE_HALTI5_RT_ALPHA_CONFIG`) whatever their format, so an
integer buffer beside a blended RGBA8 one would have been blended; it now
gets 0 there (and the OVERWRITE hint), RGBA8 beside it keeps its blend. The
reverse case, an integer attachment 0 with a unorm draw buffer 1 and
GL_BLEND on, leaves the unorm buffer unblended: that is the GL core's single
`blend` flag, not the back end's (handoff).

**Clears.** `glClearBuffer{i,ui}v` is the clear quad as every clear is. The
GL core puts the four raw 32-bit values in `GlesDraw::clear_rgba`'s bits; the
quad's shader is one `mov` of its constant to the colour output; the target's
output mode is programmed from the target's format, not the program, so the
clear stores the integers. Tested: the constant window holds the four values
bit for bit, and the clear's `PE_COLOR_FORMAT` / `PS_CONTROL_EXT` are the
table's.

**Shader outputs, unconverted.** The online compiler (`src/shaderc/online/
vivante`) already keeps an `ivec4`/`uvec4` output in the integer register it
computed: no conversion is emitted on the way out. `tests/es3_int_rt_gc.rs`
runs the encoded programs in the interpreter of `tests/common` and checks the
output register's bits (a `uvec4` past 2^31, an `ivec4` wrapping to
`INT_MIN`), that integer arithmetic emits no `i2f`/`f2i`, that a float made
integer by `int()` is converted exactly once (the shader's own `f2i`), and that
a uniform moved to the output is one `mov`. No compiler change was needed, so
`src/vivante/internal.rs` is unchanged.

## 2. What the hardware cannot render: RGBA32I and RGBA32UI

The PE format enum has **no 128-bit format of any kind**
(`state_3d.xml.h:89-113`: the widest are the 64-bit `A16B16G16R16F`,
`G32R32F`, `A16B16G16R16I`). Mesa's etnaviv renders a 128-bit format as two
planes, which is why it keeps half of the core's eight render targets back:
`max_render_targets = num_rts / 2` (`etnaviv_screen.c:330` at Mesa 4550a8d,
quoted in `src/vivante/VIVANTE_NOTES.md`, "Draw buffers"; at 26.2.1 the same
cap is `etnaviv_screen.c:299-304,774-782`, and `gpu_supports_render_format`
gates 128-bit formats on `HALF_FLOAT`, `:500-505`, both as cited by the
es3-caps item). That two-plane path (two PE targets per GL attachment, the
shader's output split across two registers) is not implemented here.

So RGBA32I and RGBA32UI are **refused by name, never drawn as garbage**: their
bits are not in `GC_INTEGER_RT_FORMATS`, the GL core's
`refuse_undrawable_targets` (`src/gl/draw.rs`) logs `draw refused: this back
end cannot render into a RGBA32UI colour attachment` once and the draw or
clear sets `GL_INVALID_OPERATION`, and `draw_one` refuses such a target with
`-ENOTSUP` as well should anything reach it. The test checks both: the error
and that no `PE_COLOR_FORMAT` was written.

ES 3.0 lists RGBA32I/UI as required colour-renderable formats, so the
conformant answer is `GL_FRAMEBUFFER_UNSUPPORTED` from
`glCheckFramebufferStatus` (an implementation may refuse a combination of
internal formats, ES 3.0 4.4.4.2), not an error at the draw;
`glGetInternalformativ` cannot refuse a required format. The framebuffer is
still reported complete today, because turning a non-drawable surface into
`FRAMEBUFFER_UNSUPPORTED` is a GL-core change (`src/gl/framebuffer.rs`) that
moves the V3D's and the GC7000's answers together. It is left to a GL-core
item (handoff).

## 3. The other ES 3.0 colour-renderable formats

| GL format | how the GL core stores it | PE format used | PE format the hardware has |
|---|---|---|---|
| R8, RG8 | RGBA8 (`framebuffer.rs` `is_color_format`) | `A8R8G8B8` | `PE_FORMAT_R8` 0x23, `G8R8` 0x1f (`:113`, `:112`) |
| RGB10_A2 | RGBA8 (ES 3.0 build) | `A8R8G8B8` | `PE_FORMAT_A2B10G10R10` 0x16 (`:103`) |
| RG16F | RGBA16F | `A16B16G16R16F` | `PE_FORMAT_G16R16F` 0x12 (`:99`) |
| R11F_G11F_B10F | not a colour-renderable name in the GL core | | `PE_FORMAT_B10G11R11F` 0x1d (`:110`) |
| RG32F | not a colour-renderable name | | `PE_FORMAT_G32R32F` 0x15 (`:102`) |
| RGBA32F | not a colour-renderable name | | none (128-bit, section 2) |
| SRGB8_ALPHA8 | `GLES_SURF_SRGB8_A8`, refused (`SRGB_RT` false) | | docs/es3-srgb.md |

ES 3.0 requires none of R11F_G11F_B10F, RG32F or RGBA32F to be
colour-renderable (they are only with EXT_color_buffer_float). The narrow
native formats (R8, G8R8, A2B10G10R10, G16R16F) would save memory but need
`GLES_SURF_*` surfaces of their own in the GL core first; the wider storage
draws the same values.

## 4. Sources, and what was not available

* `driver/src/vivante/hw/state_3d.xml.h` (vendored rnndb header): every PE
  format and output-mode value above.
* `etnaviv_translate.h:446` (`translate_output_mode`): the 32-bit and
  non-integer arms were already transcribed in `shader.rs` by the state audit
  (2026-09-22). The integer arms were written down from that function **without
  a Mesa tree at hand**: this worker had no network and no etnaviv checkout
  (the vendored `third_party/mesa-26.2.1` carries the V3D and PowerVR drivers
  only). The mode values themselves are the header's. `etnaviv_format.c`'s
  per-format PE column (which PE format etnaviv gives R32_UINT and friends) and
  `etnaviv_emit.c`'s RT emission were not re-read for this item; the
  register-by-register MRT emission was checked against them by the MRT work
  (`VIVANTE_NOTES.md`, "Draw buffers").
* The hwdb (`gc_feature_database.h`) and `etnaviv_screen.c` lines are the
  es3-caps item's citations (branch `ccfarm/01a11ddf-...`,
  `docs/es3-backend-caps.md` §1.2, not yet on the integration branch).

## 5. For the board (i.MX8MP)

Nothing here has run on the GPU. What a session should check, cheapest first:

1. dEQP-GLES3 `functional.fragment_out.basic.int.*` / `.uint.*` for the 8-
   and 16-bit formats and RGB10_A2UI: the integer modes and PE formats.
2. The same for `r32i`/`r32ui`: confirms or refutes the R32F + `UIF32`
   inference (section 1). A wrong answer shows as wrong values, not a hang;
   if it fails, take the two bits out of `GC_INTEGER_RT_FORMATS` and they are
   refused by name like RGBA32.
3. gl_test case 44's layout probe (64-bit LINEAR targets); only after it
   reads right, `GC_INT_RT_64BPP` in and the `rgba16i`/`rg32i` cases.
4. `functional.color_clear.*int*` and the `shaders.builtin_functions` groups
   that render into integer renderbuffers (es3-inventory's bucket for
   int-rt-gl): the clear quad's raw constants.
5. `rgba32i`/`rgba32ui` cases: refused (`GL_INVALID_OPERATION`, the log line),
   so they fail rather than pass; they stop failing only with section 2's
   two-plane path or with `FRAMEBUFFER_UNSUPPORTED` (NotSupported).
