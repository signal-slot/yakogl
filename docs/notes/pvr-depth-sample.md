<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# PowerVR: a depth texture sampled from its depth buffer (glmark2 shadow, #40)

What the code relies on when a draw samples a depth texture a kick rendered
into: the TPU reads the ISP's depth/stencil buffer itself, and the CPU
resolve of that buffer into the texture's R32F texels (`src/gl/texture.rs`
`resolve_depth`, `pvr_backend_depth_resolve`) runs only for a CPU read or
write of the texels. Cited from `src/pvr/backend.rs` (`pvr_depth_sampled`,
`depth_buffer_binding`, `TEXF_X8U24`) and `src/backend.rs`
(`samples_rendered_depth`).

## The cost it removes

glmark2 `shadow` at 800x600 on the AM62P ran at 353 ms a frame (score 461,
`tools/cts/RESULTS.md` before this change) while 30 of 33 scenes ran at
real-time rates. The scene renders the horse's depth into a 1600x1200
GL_DEPTH_COMPONENT texture (twice its canvas), then draws the ground
sampling that texture with NEAREST and compares in the shader. On the host
harness `docs/notes/pvr-refract-draw.md` had already attributed 95 to 97%
of the scene's `glDrawArrays` time to one step, `pvr_backend_depth_resolve`:
before the ground draw the GL core waited for the depth pass's kick and
converted the 1600x1200 packed D24S8 words the ISP stored (1.92 Mpx) into
R32F texels, one closure call and one f64 divide a texel, on the CPU, every
frame. On the board that conversion reads GPU memory the CPU had to
invalidate first and writes 7.68 MB of texels it then flushes; the A53 took
the whole frame for it. The GPU's own work for the frame is a few
milliseconds (gl_test case 215 prints it beside the wall time).

The reproducer is gl_test case 215 `shadowmap` (`driver/tools/pvrlab/gl_test.c`):
the scene's two passes on a 1600x1200 depth texture, 20 frames, ms a frame
and the depth map's Mpx/s, the last frame's picture checked at four points.

## The design (Mesa's)

Mesa's PowerVR Vulkan driver has no depth-resolve step: a depth/stencil
image is an ordinary image the ISP stores to and the TPU samples, and the
format work is one table row, `FORMAT_DEPTH_STENCIL(Z24_UNORM_S8_UINT,
ST8U24, X8U24, U8X24)` (`src/imagination/vulkan/pvr_arch_formats.c:156`,
Mesa 26.2.1): the TPU format of the whole image is ST8U24, of a view on the
depth aspect X8U24 (the 24-bit unorm in bits 23..0 as red, the top byte
ignored), of a view on the stencil aspect U8X24. `pvr_arch_pack_tex_state`
(`pvr_arch_tex_state.c`) takes a linear 2D view as TEXTYPE_STRIDE with
`stride` the row pitch in texels less one, `alpha_msb` from
`vk_format_is_alpha_on_msb` (false for Z24X8), and the pack info from
`setup_pck_info`: `pco_pipe_to_pck_format` has no pack format for Z24X8
("no pack format", 0x1f) and the format is unorm (0x100), so 0x11f.

YakoGL's depth/stencil buffer is the ISP's ZLS store: packed D24S8
(`ZLS_FORMAT_TYPE` 24BITINT), laid out linearly at the ZLS-tile-aligned
width (32 pixels), rows top-down, which is exactly the layout
`pvr_backend_depth_resolve` reads (confirmed on the AM62P on 2026-10-01 by
gl_test 169, the rendered ramp read back exactly). So the buffer is a
STRIDE image of X8U24 texels with stride the aligned width, and the TPU
returns the same 24-bit depth the resolve converts: a draw reads the same
depth either way.

## What the code does

1. Before a draw that samples a depth texture whose rendered depth is newer
   than its texels, the GL core asks `backend::samples_rendered_depth(id, w,
   h)` after submitting the kick that rendered it (`gl::texture::depth_for_draw`,
   as the V3D already did). The PowerVR answers (`pvr_depth_sampled`) yes
   when the single-sampled buffer whose depth plane `id` owns holds depth a
   submitted kick stored (`valid`, the plane `LIVE`) at the texture's size
   and is not the frame still being recorded. A plane that is a whole clear
   alone keeps the CPU path (the resolve writes the clear value; the ISP may
   not have stored it).
2. The GL core resolves nothing and gives the draw's binding `depth_id`.
   `depth_buffer_binding` turns that binding into a `PVR_TEXFMT_DEPTH_BUFFER`
   one: the buffer's address, stride the aligned width, one level, the API
   extent, the texture's own sampler state; `build_tex_desc` packs it as
   X8U24, (r, 0, 0, 1), or (r, r, r, 1) under GL_OES_depth_texture, pack
   info 0x11f. A binding whose buffer is not there to sample (a check, not a
   path: `ds_make_room` never gives up a buffer holding contents) samples the
   black texel and says so once.
3. No wait: the GPU runs kicks in order, so the kick that samples the
   buffer reads it after the one that stored it (`flush`'s comment; the
   same rule every render-to-texture here relies on). The texels stay stale
   until a CPU read or write resolves them (`settle_texture`), as before.
4. The texture's R32F storage is deferred (`pvr_defers_depth_storage`, as
   the V3D's `docs/notes/v3d-depth-storage.md`): glmark2's 1600x1200 map
   never allocates its 7.68 MB of texels unless something reads them.

The exit line counts the draws that sampled a buffer and the refusals
(`ds_report`: "depth sampled from the buffer N (refused M)").

## The TPU state words

The descriptor `build_tex_desc_lod` fills for a `PVR_TEXFMT_DEPTH_BUFFER`
binding (`src/pvr/backend.rs`: the `t.format == PVR_TEXFMT_DEPTH_BUFFER`
branch, `TEXF_X8U24`, `tex_image_word0`, `tex_stride_image_word1`,
`tex_desc_sampler_lod`), for glmark2's 1600x1200 depth map. 1600 is a
multiple of the 32-pixel ZLS tile, so the aligned width is 1600 and the
buffer is 1600 x 1216 words.

Citations. Header: `texstate.xml` is `third_party/mesa-26.2.1/src/
imagination/csbgen/rogue/texstate.xml` (line numbers are that file's);
`src/pvr/csbgen.rs` is generated from it and gives
`ROGUE_TEXSTATE_FORMAT_X8U24 = 21` (line 871). Mesa: the pvr Vulkan driver
is not vendored, so the tag `mesa-26.2.1` was fetched shallow
(gitlab.freedesktop.org/mesa/mesa); `tex_state.c` below is
`src/imagination/vulkan/pvr_arch_tex_state.c`, `formats.c`
`pvr_arch_formats.c`, `sampler.c` `pvr_arch_sampler.c`, and the format
helpers are `src/util/format/u_format.yaml`, `u_format_parse.py`,
`src/vulkan/util/vk_format.h`, `src/imagination/common/pvr_iface.h` and
`src/imagination/pco/pco.c`. The sampler is the texture's own; the sampler
words below are glmark2 shadow's (NEAREST both ways, CLAMP_TO_EDGE, no
mipmap, no compare mode: the compare is in the shader).

A descriptor is 14 dwords: `w[0..1]` IMAGE_WORD0, `w[2..3]` the STRIDE
IMAGE_WORD1, `w[4..7]` the image meta, `w[8..9]` SAMPLER_WORD0, `w[10..11]`
SAMPLER_WORD1, `w[12..13]` the sampler meta.

### IMAGE_WORD0 (`w[0..1]`, texstate.xml:272-290)

The red form (r, 0, 0, 1) is `0x04af18fc_a8002d84`. The luminance form
(r, r, r, 1) differs in swiz1 and swiz2 only: `0x04af18fc_a8000084`.

| field | bits | value | header | Mesa 26.2.1 | verdict |
|---|---|---|---|---|---|
| `textype` | 0..2 | STRIDE = 4 | texstate.xml:248-256, field :289 | tex_state.c:140-145: a linear 2D view is STRIDE | matches |
| `gamma` | 3 | 0 (OFF) | :288 | tex_state.c:207-228: sRGB formats only | matches |
| `swiz3` | 5..7 | SRC_ONE = 4 | :286; enum :239-246 | see "Swizzle" | matches |
| `swiz2` | 8..10 | SRC_ZERO = 5 (luminance: SRCCHAN_0 = 0) | :285 | see "Swizzle" | red form matches; luminance is GL's, not Mesa's |
| `swiz1` | 11..13 | SRC_ZERO = 5 (luminance: SRCCHAN_0 = 0) | :284 | see "Swizzle" | as swiz2 |
| `swiz0` | 14..16 | SRCCHAN_0 = 0 (r) | :283 | see "Swizzle" | matches |
| `minlod` | 17..26 | 0 | :282 | not set (tex_state.c:139-234) | matches |
| `texformat` | 27..33 | X8U24 = 21 | :95-193 (`X8U24` :117); csbgen.rs:871 | tex_state.c:190-191 takes `pvr_arch_get_tex_format_aspect(format, aspect)` (formats.c:312-326), the row's `depth_tex_format`; formats.c:153 `FORMAT_DEPTH_STENCIL(Z24X8_UNORM, X8U24, X8U24, INVALID)` and :156 `(Z24_UNORM_S8_UINT, ST8U24, X8U24, U8X24)` both give X8U24 for the depth aspect | matches |
| `width` | 34..47 | 1599 (API width - 1) | :275 | tex_state.c:230 `extent.width - 1` | matches |
| `height` | 48..61 | 1199 (API height - 1) | :274 | tex_state.c:231-233 | matches |
| `smpcnt` | 62..63 | 0 | :273 | tex_state.c:197 `util_logbase2(sample_count)` = 0 | matches (a binding is made for `samples <= 1` only, `depth_buffer_sampleable`) |

Width and height are the API extent, not the aligned one: the stride alone
reaches the padding (a 1000x700 texture in a 1024x704 buffer has width 999,
height 699, stride 1023).

### STRIDE_IMAGE_WORD1 (`w[2..3]`, texstate.xml:315-332)

For the buffer address `0x80_0000_2000` of the unit test:
`0x10200000_0800063f`.

| field | bits | value | header | Mesa 26.2.1 | verdict |
|---|---|---|---|---|---|
| `stride` | 0..14 | 1599 (aligned width - 1, in texels) | :331 | tex_state.c:245-247 `info->stride - 1` | matches in form; the pitch itself is the ISP's, see "What is not confirmed" |
| `mipmaps_present` | 15 | 0 | :330 | tex_state.c:249 | matches (one level) |
| `texaddr` | 16..53 | address >> 2 | :329 (shift 2) | tex_state.c:251 | matches |
| `tpu_image_state_v2_compression_mode` | 54..56 | 0 (NONE) | :322 | tex_state.c:266-269: TPU mode for compressed formats only | matches |
| `alpha_msb` | 59 | 0 | :317 | tex_state.c:253-254 `vk_format_is_alpha_on_msb`; vk_format.h:214-224 is true only for RGB or SRGB colourspace with alpha in W, Z24 is `ZS` | matches |
| `num_mip_levels` | 60..63 | 1 | :316 | tex_state.c:248 `info->mip_levels` | matches |

### Image meta (`w[4..7]`)

| dword | Mesa name | value | Mesa 26.2.1 | verdict |
|---|---|---|---|---|
| `w[4]` | `PCO_IMAGE_META_LAYER_SIZE` | 1600 x 1200 x 4 = 7,680,000 (`0x753000`) | tex_state.c:318 `info->layer_size` | not Mesa's value for a depth image (Mesa gives the image's layer pitch, the aligned size); read only to step layers of an array, never for a 2D view: unobservable here |
| `w[5]`, `w[6]` | `BUFFER_ELEMS`, `Z_SLICE` | 0 | tex_state.c:319-320 | read only by buffer textures and 2D views of 3D: unobservable |
| `w[7]` | `PCO_IMAGE_META_PCK_INFO` | `0x11f` | tex_state.c:321, `setup_pck_info` :84-112 | matches, below |

Pack info `0x11f`: bits 0..4 the pack format `PVR_PCK_FORMAT_INVALID`
(`0b11111`, pvr_iface.h:132), bit 8 UNORM (pvr_iface.h:140-142), the SPLIT
(5), SCALE (6) and ROUNDZERO (7) bits clear. Mesa: `pco_pipe_to_pck_format`
(pco.c:387-461) has no case for `Z24X8_UNORM` or `Z24_UNORM_S8_UINT`, so it
returns `~0` and `setup_pck_info` turns that into `PVR_PCK_FORMAT_INVALID`
(tex_state.c:95-96); `util_format_is_unorm` is true for a name containing
`_UNORM` (u_format_parse.py:414-418), which both names do (tex_state.c:108).
Matches. A shader reads the pack info for a shadow compare's clamp and for
formatless image stores only; a plain `texture2D` does not depend on it.

### SAMPLER_WORD0 (`w[8..9]`, texstate.xml:342-361), glmark2 shadow

Value `0x0200_2400_0000_0fff`.

| field | bits | value | Mesa 26.2.1 (sampler.c) | verdict |
|---|---|---|---|---|
| `dadjust` | 0..12 | 4095 (`ZERO_UINT`, bias 0) | :149-153 | matches |
| `minlod` | 13..22 | 0 | :181-183 | matches |
| `maxlod` | 23..32 | 0 (one level) | :185-187 | form matches; Mesa's is the application's `maxLod`, YakoGL's own rule for a one-level image |
| `anisoctl` | 33..35 | 0 (DISABLED) | :155-165 | matches |
| `magfilter`, `minfilter` | 36..37, 38..39 | POINT = 0 | :131-132 | matches |
| `mipfilter` | 40 | 0 | :134-135 | matches |
| `addrmode_u`, `addrmode_v` | 41..43, 44..46 | CLAMP_TO_EDGE = 2 | :137-140 | matches |
| `bordercolor_index` | 50..55 | 0 | :189 (a border table entry) | differs, harmlessly: CLAMP_TO_EDGE reads no border colour |
| `addrmode_w` | 56..58 | CLAMP_TO_EDGE = 2 | :141-142 | matches for a 2D image (Mesa copies the third axis; YakoGL fixes it for 2D) |

SAMPLER_WORD1 (`w[10..11]`, texstate.xml:363-367) is 0 (`cemedge_dontfilter`
is set only for the native cube under ES 2.0). The sampler meta `w[12]` is
the compare op: 0 without GL_TEXTURE_COMPARE_MODE, Mesa's
`VK_COMPARE_OP_NEVER` = 0 without compare (sampler.c:112-118): matches.
`w[13]` is YakoGL's own PCF flag word, 0 for NEAREST over one level (no Mesa
counterpart).

### Swizzle: `pvr_get_format_swizzle_for_tpu`

Mesa's `pvr_get_format_swizzle_for_tpu` (pvr_formats.c:346-369) returns
(X, NONE, NONE, NONE) for any depth or stencil format ("hardware can only
access either depth or stencil at once ... in the first channel"). Through
`pvr_get_hw_swizzle` (tex_state.c:40-64) that is (SRCCHAN_0, SRC_ZERO,
SRC_ZERO, SRC_ONE): the (r, 0, 0, 1) the binding emits. Its reach is
narrower than its name:

- In 26.2.1 it has one caller, `pvr_arch_border.c:358` (the border colour's
  re-swizzle for a mismatched pair of formats). A sampled image view does
  not use it: `pvr_arch_image.c:166-191` composes the pipe format's own
  `util_format_description()->swizzle` with the view's component mapping.
  So the words are checked against what Mesa's view code produces, and
  the helper agrees with that for the depth-only format.
- That view swizzle for `Z24X8_UNORM` is (X, _, _, _) (u_format.yaml:641-646)
  = (SRCCHAN_0, ZERO, ZERO, ONE): identical to the binding. For
  `Z24_UNORM_S8_UINT` it is (X, Y, _, _) (u_format.yaml:617-622) =
  (SRCCHAN_0, SRCCHAN_1, ZERO, ONE): a Vulkan depth view of a D24S8 image
  reads channel 1 into green, which X8U24 returns as whatever the TPU gives
  for a format with one data channel; the binding forces 0. A GLES 2.0
  depth texture is (d, 0, 0, 1), so YakoGL's form is the GL one and nothing
  here samples green. A difference from Mesa's D24S8 view, not a defect.
- The luminance form (r, r, r, 1) (`TEXF_X8U24_LUMINANCE`) is
  GL_OES_depth_texture's; no Vulkan view asks for it. It is the swizzle of
  `TEXF_R32F_LUMINANCE` on the CPU-resolved path.

### What is not confirmed

- The bit layout of the word as the TPU reads it: bits 23..0 as the
  24-bit unorm, 31..24 ignored. Mesa's table names X8U24 for the depth
  aspect of Z24_UNORM_S8_UINT (formats.c:156) and the PBE side lists
  `X8U24` and `ST8U24` pack modes (pbestate.xml:105-106), but no vendored
  header spells the bit layout out. The board measurement (shadow's
  pictures equal to the recorded column, gl_test 215's four points) is the
  evidence.
- That the ZLS store is a linear pitch of the aligned width at ZLSCTL's
  "no twiddling" setting: YakoGL's fact (`pvr_backend_depth_resolve`'s
  layout; gl_test 169 read a ramp back exactly), not Mesa's.
- That the TPU's 24-bit to float conversion equals the CPU resolve's
  `ds_retention::unorm24_depth` to the last bit. glmark2 compares NEAREST
  depths against a bias in the shader and the pictures agree; no
  bit-exact comparison of a sample against the resolve exists (the
  experiment below does not make one either).

No word was found that differs from Mesa in a way that changes what the TPU
reads for a 2D NEAREST sample of a depth texture. The differences (green
channel of Mesa's D24S8 view, `bordercolor_index`, `maxlod`, `layer_size`)
are listed above and none touches that read.

## Tests

- `src/pvr/backend.rs` `a_depth_buffer_binding_is_mesas_x8u24_stride_image`:
  the descriptor words against Mesa's values.
- `tests/depth_resolve_pvr_backend.rs`: glmark2's shadow frame on the real
  backend over a stand-in GPU: every frame's ground samples the buffer that
  holds its own depth (with the canvas kick in flight too), no resolve, the
  storage never allocated; a CPU write resolves first; a GL_LESS pass after a
  glTexSubImage2D still loads the uploaded depth.
- gl_test 215 on the board: the picture at four points, and the figure.

## Board measurement (AM62P, 2026-10-27)

gl_test case 215, 20 frames of the two passes on a 1600x1200 depth map, the
same picture at its four points in both runs:

| image | ms a frame | depth map Mpx/s | GPU busy a frame (TA\|3D) |
|---|---:|---:|---:|
| main `c5dcf5e8` plus the case alone | 355.20 | 5.4 | 6.35 ms |
| with this design (`c999f2f7`) | 27.75 | 69.1 | 6.37 ms |

The GPU's work is the same 6.4 ms; the 349 ms that went were the CPU
resolve. What remains above the GPU time (about 21 ms) is the two kicks'
submit and fence round trips with a glFinish each frame, not the texture.

glmark2 at 800x600 (`tools/cts/RESULTS.md`, the AM62P row): shadow 353.23
to 10.64 ms a frame (33x), refract 375.53 to 37.03 ms (10x: its depth
texture takes the same path), score 461 to 467, the other 31 scenes
unchanged within the run-to-run spread, every picture probe of shadow equal
to the recorded column, 0 GL errors, 0 resets, 2026 draws a run sampled a
buffer and none was refused.

## Re-checking on the board

What would confirm the words above on current main. It needs the AM62P
(`resource:verdin-am62p` and `resource:serial-am62p`, leased through
connected, shared with other work) and an image built from an `origin/main`
commit (`<sha8>` its first eight hexadecimal digits), never from a working
tree. Records go to the gitignored, redacted
`records/am62p/<YYYYMMDD-HHMM>-<name>-<sha8>/` (for example
`20261126-1030-depth-words-<sha8>`; board addresses and credentials stay in
the environment).

1. gl_test case 215 `shadowmap` alone, from that commit: an image built with
   `GL_TEST_CASES="215"` (`driver/samples/zephyr/gl_test/Kconfig`,
   `tools/cts/common-images.sh` passes it as `CONFIG_GL_TEST_CASES` for
   `am62p`), as the board stage builds the GT image of the common set.
2. `tools/cts/board-stage.sh am62p common` from the same commit: the GT
   image (gl_test, its default case list) and the GM image (glmark2-es2,
   every default scene), the records in `records/am62p/common-<sha8>`,
   moved or copied under the dated name above. Case 215 is in the GT image
   only if the default case list carries it; if it does not, step 1 is its
   run.

Read these lines in the console and records:

| what | where | expect |
|---|---|---|
| `gl_test: case 215 shadowmap: PASS` | the gl_test console (`common-tally.py`'s `CASE_LINE_RE`) | PASS (five checks after the setup: the two programs linked, no GL error after setup, framebuffer status, no GL error after the frames, the four points) |
| `gl_test: shadowmap: 20 frames, <ms> ms a frame (1600x1200 depth map, <Mpx/s> Mpx/s)` | the same | 27.75 ms a frame at `c999f2f7`; the figure on current main is the number to put beside it, with the GPU busy time a frame from the same line (6.37 ms at `c999f2f7`); a frame near 355 ms means the CPU resolve is back |
| `depth sampled from the buffer N (refused M)` | the exit line, `ds_report` (`src/pvr/backend.rs`) | N greater than 0 and M = 0: 2026 draws a glmark2 run at the AM62P measurement; N = 0 means no draw sampled the buffer, M > 0 means a binding named a buffer that was not there |
| `pvr_backend: a 1600x1200 depth texture is sampled from its depth buffer (X8U24, stride 1600): no CPU resolve` | the console, once a run (`depth_buffer_binding`) | present; confirms the stride this note derives |
| glmark2 `shadow` and `refract` ms a frame | the GM console, `tools/cts/glmark2-ms.py --cells` over it | shadow near 10.6 ms, refract near 35-37 ms (6a7021ec, `tools/cts/RESULTS.md`); 353 and 375 ms mean the CPU path |
| the picture probes of shadow | `common-tally.py`'s glmark2 picture rows against `tools/cts/glmark2-pictures.txt`, column `am62p` | `c 9d9d9dff, tl 000000ff, tr 000000ff, bl 4d4d4dff, br 4c4c4cff; crc16 3229`, black no (refract: `c 3f1c1aff`, crc16 3049, black no); a different probe at c, bl or br after the ground draw means the TPU read other depth than the resolve did: the format word or the stride is wrong |
| 0 GL errors, 0 resets | the exit line and the console | as the measurement |

A sampled word read wrongly shows in the picture before it shows anywhere
else: a wrong FORMAT or pitch gives a ground that is lit or shadowed
everywhere (all-white or all-black c, bl, br), a wrong swizzle gives the
same, a wrong address a TPU fault (a `gc`-style fault line and a reset).
If the probes match and the ms a frame is down, the words are confirmed on
this commit as far as a 2D NEAREST read goes. They do not confirm the last
bit of the conversion (see "What is not confirmed"): a draw that compared
samples with `pvr_backend_depth_resolve`'s texels would, and no such case
exists.
