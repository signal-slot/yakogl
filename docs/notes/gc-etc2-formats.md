<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: ETC1, ETC2 and EAC as Mesa's etnaviv describes them, and the ETC1 question

Design note of plan #186, item etc2-note. It fixes, from Mesa's etnaviv driver,
the words a GC7000UL texture descriptor takes for every ETC1, ETC2 and EAC
format, argues that a valid ETC1 block decodes to the same texels under the
ETC2 decoder, decides what the ES 2.0 ETC1 path does, lists where
`src/vivante/{regs,texture}.rs` differ, and names what a board must confirm.
It is the source for the items etc2-ext-none and etc2-srgb-gc. No code changes
with it.

## 1. Sources

Mesa 26.2.1, tag `mesa-26.2.1` (`third_party/mesa-26.2.1/UPSTREAM`; MIT, so it
may be cited). **The etnaviv driver is not in `third_party/`**: the vendored
tree holds the V3D, PowerVR and Mali parts only. `docs/es3/gc-formats.md`
section 1 reads etnaviv from the same release's source tree in the deps cache,
and so does this note; a reviewer opens the same files in a checkout of tag
`mesa-26.2.1`. Paths are relative to its `src/`, with the letters of
gc-formats.md and five more:

* `F` `gallium/drivers/etnaviv/etnaviv_format.c`, `D`
  `gallium/drivers/etnaviv/etnaviv_texture_desc.c`, `S`
  `gallium/drivers/etnaviv/etnaviv_screen.c`, `X`
  `etnaviv/hw/common_3d.xml.h`;
* `N` `etnaviv/hw/state_3d.xml.h`, `E`
  `gallium/drivers/etnaviv/etnaviv_etc2.c`, `Y` `util/format/u_format.yaml`,
  `P` `util/format/u_format_parse.py`, `H`
  `etnaviv/hwdb/nxp/gc_feature_database.h` (the GC7000UL row, "gc7000ULN_v123",
  model 0x7000 revision 0x6204, is lines 18841-19733).

No GPL kernel source is used.

## 2. Mesa's words, format by format

etnaviv looks a format up by its **linear** twin (`util_format_linear`,
`F:278`), so the sRGB formats have no table row of their own: they take the
linear row and add the sRGB sampler bit (2.2). The rows are `F:237` and
`F:244-250` (`_T` = texture only, `F:69-76`).

| Gallium format | GL internal format | table `.tex` | CONFIG0.FORMAT | CONFIG1.FORMAT_EXT | sRGB | gate |
|---|---|---|---|---|---|---|
| `ETC1_RGB8` | `GL_ETC1_RGB8_OES` | `ETC1` = 0x1e (`F:237`, `X:65`) | 0x1e | 0 | no | `ETC1_TEXTURE_COMPRESSION` (`S:400-401`) |
| `ETC2_RGB8` | `COMPRESSED_RGB8_ETC2` | `EXT_NONE \| EXT_FORMAT` = 0 and bit 31 (`F:244`, `X:66`) | 0 | 0 | no | HALTI0 (`S:409-410`) |
| `ETC2_SRGB8` | `COMPRESSED_SRGB8_ETC2` | the row of `ETC2_RGB8` (`F:278`) | 0 | 0 | **yes** | HALTI0 (`S:406-407`) |
| `ETC2_RGB8A1` | `COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2` | `EXT_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 \| EXT_FORMAT` = 1 (`F:245`, `X:67`) | 0 | 1 | no | HALTI0 |
| `ETC2_SRGB8A1` | `COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2` | the row of `ETC2_RGB8A1` (`F:278`) | 0 | 1 | **yes** | HALTI0 |
| `ETC2_RGBA8` | `COMPRESSED_RGBA8_ETC2_EAC` | `EXT_RGBA8_ETC2_EAC \| EXT_FORMAT` = 2 (`F:246`, `X:68`) | 0 | 2 | no | HALTI0 |
| `ETC2_SRGBA8` | `COMPRESSED_SRGB8_ALPHA8_ETC2_EAC` | the row of `ETC2_RGBA8` (`F:278`) | 0 | 2 | **yes** | HALTI0 |
| `ETC2_R11_UNORM` | `COMPRESSED_R11_EAC` | `EXT_R11_EAC \| EXT_FORMAT` = 3 (`F:247`, `X:69`) | 0 | 3 | no | HALTI0 |
| `ETC2_R11_SNORM` | `COMPRESSED_SIGNED_R11_EAC` | `EXT_SIGNED_R11_EAC \| EXT_FORMAT` = 0x0d (`F:248`, `X:79`) | 0 | 0x0d | no | HALTI0 and HALTI1 (snorm, `S:416-417`) |
| `ETC2_RG11_UNORM` | `COMPRESSED_RG11_EAC` | `EXT_RG11_EAC \| EXT_FORMAT` = 4 (`F:249`, `X:70`) | 0 | 4 | no | HALTI0 |
| `ETC2_RG11_SNORM` | `COMPRESSED_SIGNED_RG11_EAC` | `EXT_SIGNED_RG11_EAC \| EXT_FORMAT` = 5 (`F:250`, `X:71`) | 0 | 5 | no | HALTI0 and HALTI1 |

The gates are `gpu_supports_texture_format` (`S:395-451`); the GC7000UL sets
HALTI0 to HALTI5, so all are met (gc-formats.md section 1).

### 2.1 How the descriptor encodes the value

`etna_texture_desc_fill` (`D:140`) is the only writer on this core (halti >= 5):

* `CONFIG0 = COND(!ext && !astc, FORMAT(format)) | TYPE | ...` (`D:179`): a
  format with `EXT_FORMAT` leaves CONFIG0.FORMAT **zero**. CONFIG0.FORMAT is
  bits 13-17 (`N:1600-1602`).
* `CONFIG1 = COND(ext, FORMAT_EXT(format)) | ... | swizzle` (`D:183`):
  FORMAT_EXT is bits 0-5 (`N:1661-1663`); the mask drops `EXT_FORMAT` (bit 31).
* So **`EXT_NONE` is CONFIG0.FORMAT = 0 (`TEXTURE_FORMAT_NONE`, `X:43`) with
  CONFIG1.FORMAT_EXT = 0**: no format field set at all, which the hardware
  decodes as ETC2 RGB8 (`F:244`: "Extd. format NONE doubles as ETC2_RGB8").
  No other sampled format collides: every non-ext format has a non-zero
  CONFIG0.FORMAT (A4R4G4B4 is 5, `X:48`). The converse holds too: `EXT_FORMAT`
  never reaches the descriptor, so a class that is *undecoded* and left at
  zeros would read as ETC2 RGB8. `GcTexfmt::decoded` guards that today;
  etc2-ext-none must keep the guard.
* Compressed layout: `ADDRESSING_MODE_LINEAR` is set only for a non-compressed
  linear resource (`D:180-181`), so block formats take the same CONFIG0 type
  and addressing for ETC1 and ETC2.
* CONFIG2 is `0x00030000` plus sign bits for pure sint only (`D:187`): nothing
  extra for ETC.
* Mesa patches ETC2 blocks on the CPU (`E:34-41`, `etnaviv_transfer.c:165`)
  only for chips **without HALTI1**; the GC7000UL has it, so no ETC2 block is
  patched here.

### 2.2 The sRGB bit

`if (util_format_is_srgb(so->format)) sv->SAMP_CTRL1 |= ..._SRGB` (`D:246-247`);
`VIVS_NTE_DESCRIPTOR_SAMP_CTRL1_SRGB` = 0x4 (`N:1984`; our `GC_SAMP_CTRL1_SRGB`,
`regs.rs:751`). The format words do not change for the sRGB twin.
`texture_use_int_filter` returns false for any sRGB format (`F:317-318`), so
**INT_FILTER is off** for `ETC2_SRGB*`. It is on for `ETC1_RGB8`, `ETC2_RGB8`,
`ETC2_RGB8A1`, `ETC2_RGBA8`: a compressed format that is neither float nor
snorm counts as unorm (`P:408-417`). `ETC2_R11_UNORM` and `ETC2_RG11_UNORM` are
named off (`F:332-333`); the signed ones are snorm, so off.

### 2.3 Swizzle

`get_texture_swiz` (`F:346-384`) has two format rules, DXT1_RGB alpha-to-1 and
the R8 emulation, and **no ETC rule**: the descriptor carries the sampler
view's swizzle unchanged. The 1-fill of alpha for the RGB formats comes from
the format, not a swizzle word (the format description of `ETC1_RGB8` and
`ETC2_RGB8` is `[X, Y, Z, 1]`, `Y` `name: ETC2_RGB8`; R11 is `[X, 0, 0, 1]`,
RG11 `[X, Y, 0, 1]`). Our `no_alpha` swizzle of alpha to ONE for `ETC2_RGB`
(`texture.rs:228`) writes the same value the format gives: redundant, harmless.

### 2.4 The feature bits

The GC7000UL row has `REG_ETC1TextureCompression` = 1 (`H:18961`; read by
`etnaviv/hwdb/etna_hwdb.c:48`, gated at `S:400-401`) and, **in the same row,
`TX_ETC2_COMPRESSION` = 0 (`H:19355`)**. Mesa never reads the second bit; the
ETC2 formats are gated by HALTI0 alone (`S:409-410`). That Mesa ships them for
this core and the 0 in the row do not agree on their face. Our ext 1 and 2
words (`ETC2_RGBA`, `ETC2_RGB_PT`) rest on the same assumption, so the board
run of section 6 is also what settles it.

## 3. ETC1 blocks under the ETC2 decoder

OpenGL ES 3.0 specification, appendix C (C.1 "ETC2 compressed texture image
formats", the RGB8 format), against the ETC1 format of
OES_compressed_ETC1_RGB8_texture:

1. The 64 bits have the same fields: the colour bits, the two 3-bit table
   codewords, the **diff bit** (bit 33), the flip bit (bit 32) and the 2x16
   pixel-index bits, at the same positions.
2. Diff bit 0 is the **individual mode** in both (two 4-bit colours expanded
   to 8 bits). Diff bit 1 is the **differential mode** in both (a 5-bit colour
   and a 3-bit signed delta per channel, each sum expanded to 8 bits).
3. The modifier tables, the flip split into two 2x4 or 4x2 sub-blocks and the
   per-texel `clamp(base + modifier, 0, 255)` are the same.
4. The one extension: in ETC1 a differential block whose base plus delta
   leaves 0..31 in R, G or B is invalid and the ETC1 specification leaves its
   result undefined. ETC2 gives those blocks meaning: overflow in R is T
   mode, in G is H mode, in B is planar mode.

So a **valid** ETC1 block (every differential sum in 0..31; the only blocks an
ETC1 encoder writes) never reaches the T, H or planar paths and decodes to the
same texels under the ETC2 RGB8 decoder; the steps are integer and identical,
so the equality is exact. In RGB8 (no punch-through) the diff bit stays the
diff bit: only RGB8A1 reads it as the "opaque" bit. A block ETC1 does not
define decodes to something under ETC2, which no conformant ES 2.0 content can
rely on. The CPU decoder `src/gl/etc1.rs:65-98` (dEQP's `decompressETC1Block`)
implements the ETC1 side of points 1-3.

This proves the specification, not the **hardware**: that the GC7000UL's
`EXT_NONE` decoder is exactly the specification on the ETC1-compatible modes.
Mesa relies on it for ETC2 RGB8 (`F:244`) and no ETC2 patch applies here
(2.1); section 6 is where it is measured.

## 4. Decision for the ES 2.0 ETC1 path

The GL core has **one** class, `GLES_TEXFMT_ETC2_RGB`, for both
`GL_ETC1_RGB8_OES` and `GL_COMPRESSED_RGB8_ETC2` (`src/gl/texture.rs:3376-3377`,
test `:6499`), 8 bytes per block. The GC7000UL back end does not decode ETC1 on
the CPU (`etc1_decoded_at_upload: false`, `src/vivante/backend.rs:3335`;
`src/gl/etc1.rs` is for a unit with no ETC1 at all). So the ES 2.0 ETC1 path
and ETC2 RGB8 are the same descriptor word today and will stay one.

**Decision: yes, the ES 2.0 ETC1 path changes, with the class.**
`GLES_TEXFMT_ETC2_RGB` moves from CONFIG0.FORMAT `ETC1` (0x1e) to Mesa's
`EXT_NONE` (CONFIG0.FORMAT 0, CONFIG1.FORMAT_EXT 0), and `GL_ETC1_RGB8_OES`
follows because it is the same class. Reasons: section 3 gives the same texels
for every valid ETC1 block, so ES 2.0 content is unchanged; keeping 0x1e for
the class gets the T, H and planar modes of ETC2 RGB8 wrong, and ES 3.0
content (and dEQP `texture.compressed.etc2*`) needs them. The alternative is a
second GL-core class so ES 2.0 keeps 0x1e: it costs a class bit (the sampled
masks are near 32, item mask-u64) and every GL-core ETC1 site for no texel
that differs. It is the **fallback only if the board shows EXT_NONE differs
from 0x1e on a valid ETC1 block** (section 6, check 2); this note is amended
then.

## 5. Mismatches with the words in `src/vivante/`

| # | Place | yakogl today | Mesa | Consequence |
|---|---|---|---|---|
| 1 | `texture.rs:160-164` | `GLES_TEXFMT_ETC2_RGB`: CONFIG0.FORMAT `GC_TEX_FORMAT_ETC1` (0x1e), ext 0 | `ETC2_RGB8`: format 0, ext 0 (`F:244`, `D:179,183`) | T/H/planar blocks decode wrong; etc2-ext-none |
| 2 | `regs.rs:879-882` | `ETC1` 0x1e, ext 0x01 and 0x02 only | also ext `NONE` 0x00 (`X:66`), R11 0x03, RG11 0x04, signed RG11 0x05, signed R11 0x0d (`X:69-71,79`) | no constants for `EXT_NONE` or EAC (regs.rs belongs to regs-words) |
| 3 | `texture.rs:129-131` | `ETC2_SRGB_RGB` / `_RGBA`: `decoded = false` | the linear row's words (format 0, ext 0 and 2) plus `SAMP_CTRL1.SRGB` (`F:278`, `D:246-247`) | refused today; etc2-srgb-gc |
| 4 | `texture.rs:398-399` (`samp_ctrl1`) | SRGB only for `GLES_TEXFMT_SRGB8_A8` | SRGB for every sRGB format incl. the compressed ones | the two classes must be added |
| 5 | `texture.rs:142-176` (`unorm_format`) | every class in it has `int_filter: true` | sRGB formats off (`F:317-318`) | the two sRGB classes need `int_filter: false`, as `SRGB8_A8` has (test `texture.rs:648`) |
| 6 | `texture.rs:165-172` | `ETC2_RGBA` ext 2, `ETC2_RGB_PT` ext 1, both `compressed` | same words (`F:245-246`) | none in the words. But the GL core cannot reach `_PT`: `compressed_format_info` (`gl/texture.rs:3374-3384`) has no `GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2`, and no sRGB twin class exists |
| 7 | `texture.rs:228` | `no_alpha` forces swizzle A to 1 for `ETC2_RGB` | none (the format gives alpha 1) | redundant, same value |
| 8 | `texture.rs` | no R11 / RG11 EAC class | ext 3, 4, 5, 0x0d; `int_filter` off (`F:332-333`) | out of this plan's items: no GL-core class (`compressed_block_bytes` knows the sizes only) |
| 9 | `GC_SAMPLED_TEXFMTS` (`src/backend.rs`) | linear ETC2 classes only | HALTI0 for sRGB and ext (`S:406-410`), met | etc2-srgb-gc adds the two sRGB bits |

Every other word agrees: ext 1 and 2, the field positions of 2.1 and
`GC_SAMP_CTRL1_SRGB` = 0x4.

## 6. What a board run must confirm

The host sees only upload and compile refusals; none of these is a host check.

1. **The decoder**: dEQP-GLES3 `texture.compressed.etc2*` (RGB8, SRGB8, RGB8A1,
   SRGB8A1, RGBA8; 2D, cube, 2D array, mipmapped), on the i.MX8MP, against the
   result before etc2-ext-none. The images contain T, H and planar blocks: the
   RGB8 cases that fail on 0x1e must pass with EXT_NONE. The same run settles
   the `TX_ETC2_COMPRESSION` = 0 doubt of 2.4: if the hardware had no ETC2
   decoder, the ext 1 and 2 cases would fail too, and the remedy would not be
   in this note.
2. **The ES 2.0 ETC1 path**: dEQP-GLES2 `texture.format.etc1_*` (2D pot and
   npot, mipmap, cube) and an ES 2.0 ETC1 case of glmark2 or `gl_test`
   (a `GL_ETC1_RGB8_OES` photograph, compared with the CPU decode of
   `etc1.rs`), before and after: pixel for pixel equal on valid blocks. A
   difference selects the fallback of section 4.
3. **sRGB** (etc2-srgb-gc): the sRGB cases of `texture.compressed.etc2*` give
   the decoded texels through `SAMP_CTRL1.SRGB` with INT_FILTER off, and a
   linear filter over an sRGB chain takes the curve before filtering.
4. **Neighbours**: `ETC2_RGBA` and `ETC2_RGB_PT` keep passing (words unchanged).

Host side, `tests/es3_formats_gc.rs` reads gc-formats.md section 2's table and
compares every row with the descriptor words the back end builds.

## 7. For the items after this one

* etc2-ext-none: one word in `gc_texfmt` (`ETC2_RGB`: CONFIG0.FORMAT 0), a
  constant `GC_TEX_FORMAT_EXT_NONE` = 0, the `decoded` guard kept; gc-formats.md
  row 75 and section 4 item 3 become done, and the row says the ES 2.0 ETC1
  path follows (section 4 here).
* etc2-srgb-gc: classes 24 and 25 get rows `ETC2_SRGB8` and `ETC2_SRGBA8`
  of section 2, `SAMP_CTRL1.SRGB`, `int_filter: false` and the mask bits.
* Neither needs the CPU block patch of `E`; HALTI1 is set.
