<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GL-core texture classes for R32I/UI, RG32I/UI and the narrow ES 3.0 formats

The design for three items of plan #186 (GC7000UL ES 3.0 host work):

* **mask-u64**: widen the sampled-class masks (`*_SAMPLED_TEXFMTS`,
  `sampled_texfmts()`, `gles_texfmt_bit`) from `u32` to `u64`. No behaviour
  change.
* **r32-rg32-classes**: give GL_R32I/UI and GL_RG32I/UI their own storage
  classes, which the GC7000UL texture engine (TE) samples as
  `TEXTURE_FORMAT_EXT_R32I` / `EXT_G32R32I`.
* **narrow-classes**: give GL_R8_SNORM, GL_RG8_SNORM, GL_R8I/UI, GL_RG8I/UI,
  GL_R16I/UI and GL_RG16I/UI their own classes, matching Mesa's `EXT_R8_SNORM`,
  `EXT_G8R8_SNORM`, `EXT_R8I`, `EXT_G8R8I`, `EXT_R16I` and `EXT_G16R16I` rows.
  GL_R8 and GL_RG8 (unsigned normalised) stay in RGBA8 (section 3.3).

The context is `docs/es3/gc-formats.md` section 4, items 4 and 5. Every
`file:line` below is at commit `794c0216` (the integration tip this note was
written on). A line number can move by the time an item lands; the function
or constant named next to it is the stable anchor.

The Mesa citations follow the convention of `docs/es3/gc-formats.md`
section 1. The source is Mesa 26.2.1, commit `da14d65e`, the release
`third_party/mesa-26.2.1/UPSTREAM` names. The etnaviv driver is **not**
vendored under `third_party/`. It was read from the same release's tree in the
farm's deps cache (`$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`). Paths are
relative to that tree's `src/`:

* `F` is `gallium/drivers/etnaviv/etnaviv_format.c`
* `D` is `gallium/drivers/etnaviv/etnaviv_texture_desc.c`
* `S` is `gallium/drivers/etnaviv/etnaviv_screen.c`
* `X` is `etnaviv/hw/common_3d.xml.h`

The in-tree copy of `X` is `driver/src/vivante/hw/common_3d.xml.h`. It has an
added SPDX line, so its line numbers are `X`'s plus one, and both are given.

## 1. Where the masks and the class numbers are used (what mask-u64 changes)

### 1.1 The definitions (src/backend.rs)

| file:line | what | mask-u64 |
|---|---|---|
| `src/backend.rs:83-141` | `GLES_TEXFMT_*` (`u8`, 0..=25) | unchanged: class numbers stay `u8` |
| `src/backend.rs:150-152` | `gles_texfmt_bit(f) -> u32 { 1u32 << f }` | `-> u64 { 1u64 << f }` |
| `src/backend.rs:154-163` | `const fn texfmts(&[u8]) -> u32` | `-> u64` (the body's `let mut m = 0` follows) |
| `src/backend.rs:166` | `const BASE_SAMPLED_TEXFMTS: u32` | `u64` |
| `src/backend.rs:190` | `pub const PVR_SAMPLED_TEXFMTS: u32` | `u64` (the value does not change) |
| `src/backend.rs:211` | `pub const GC_SAMPLED_TEXFMTS: u32` | `u64` |
| `src/backend.rs:233` | `pub const V3D_SAMPLED_TEXFMTS: u32` | `u64` |
| `src/backend.rs:260` | `#[cfg(test)] TEST_EXTRA_SAMPLED: AtomicU32` | `AtomicU64` (host tests only) |
| `src/backend.rs:265` | `#[cfg(test)] TEST_HIDDEN_SAMPLED: AtomicU32` | `AtomicU64` |
| `src/backend.rs:273-286` | `pub fn sampled_texfmts() -> u32`, one arm per GPU feature | `-> u64`; the Mali arm (`:281`) becomes `crate::mali::backend::SAMPLED_TEXFMTS as u64` (see 1.2) |
| `src/backend.rs:289-301` | `gles_texfmt_bytes(f) -> u32`: 16 / 8 / 2 / 1, **default 4** | unchanged by mask-u64; the class items add their arms (section 2) |
| `src/backend.rs:145-147`, `:253-255`, `:549-559` | `gles_texfmt_is_rgba8_bytes`, `gles_texfmt_return_32`, `gles_surf_texfmt` | unchanged; no new class is RGBA8 bytes, 32-bit return (a V3D key) or a surface's sampled form |

mask-u64 should also add a compile-time check that the highest class number
is below 64 (`const _: () = assert!(GLES_TEXFMT_<last> < 64);`), next to
`gles_texfmt_bit`. Today `1u32 << 32` would overflow silently in a release
`const`, and the narrow classes reach bit 39.

### 1.2 Every use of the masks (the full output of `grep -rn 'SAMPLED_TEXFMTS' src tests`)

| file:line | use | mask-u64 |
|---|---|---|
| `src/backend.rs:166, 190, 211, 233` | the four definitions | as 1.1 |
| `src/backend.rs:275, 277, 279, 281` | `sampled_texfmts()` picks one per feature | as 1.1 |
| `src/backend.rs:1792` | comment ("The Mali samples nothing yet") | none |
| `src/mali/backend.rs:96` | `pub const SAMPLED_TEXFMTS: u32 = 0` | **leave it `u32`** and widen it at `src/backend.rs:281` with `as u64`. This file belongs to plans #154/#156 (Mali), and the value is 0 |
| `src/vivante/texture.rs:408` | comment in `emit_slot` | none |
| `src/gl/state.rs:3299` | doc comment on a test | none |
| `src/v3d/texture.rs:109` | `V3D_SAMPLED_TEXFMTS & gles_texfmt_bit(b.format) == 0` (`refusal_for`) | **none**: both sides become `u64` together, so the V3D file (plan #155) needs no edit |
| `tests/es3_formats_v3d.rs:4` | comment | none |
| `tests/es3_formats_v3d.rs:232, 235` | `fold(0u32, \|m, ..\| m \| gles_texfmt_bit(..))` | **`0u32` -> `0u64`** (or `0`). This file belongs to plan #155; it is the one unavoidable token change there |
| `tests/es3_formats_v3d.rs:236` | `assert_eq!(V3D_SAMPLED_TEXFMTS, rows \| narrow)` | follows the folds |
| `tests/es3_sized_formats.rs:19-20` | `const MASKS: [(&str, u32); 3]` | `u64` (plan #155's file) |
| `tests/es3_sized_formats.rs:23-24` | `fn all_classes() -> u32`, fold over `R11G11B10F..=RGBA32F` | `-> u64`; the range is extended by the class items (section 5) |
| `tests/es3_sized_formats.rs:176, 188, 201, 226, 234, 241, 246, 540` | `storage_in(mask)`, mask arithmetic, `.fold(0, ...)` | none (inferred) |
| `tests/es3_sized_formats.rs:568` | `let refused = \|mask: u32\| ...` | `u64` |
| `tests/es3_sized_formats.rs:571-576` | refusal lists per GPU | none for mask-u64; the class items change the GC list (section 5) |
| `tests/es3_formats_gc.rs:20, 197` | comments | none |
| `tests/es3_formats_gc.rs:200-205` | `GC_SAMPLED_TEXFMTS & gles_texfmt_bit(id) != 0` against the doc's `in mask` column | none |

The other users of the mask functions, from `grep -rn 'sampled_texfmts\|gles_texfmt_bit\|TEST_EXTRA_SAMPLED\|TEST_HIDDEN_SAMPLED'`:

| file:line | use | mask-u64 |
|---|---|---|
| `src/gl/es3_formats.rs:183-185` | `pub fn storage_in(&self, sampled: u32)` | **`sampled: u64`** (the one GL-core signature) |
| `src/gl/es3_formats.rs:188-190` | `storage_here()` = `storage_in(sampled_texfmts())` | none |
| `src/gl/texture.rs:3406` | `stored_compressed_format`: `sampled_texfmts() & gles_texfmt_bit(fmt) == 0` | none (inferred). `src/gl/texture.rs` is held by wide128-backend, and mask-u64 needs no edit in it |
| `src/gl/es3_format_tests.rs:27` | `const ALL_NEW: u32 = (!0 << 11) \| (1 << GLES_TEXFMT_RGBA16F)` | `u64`. `!0u64 << 11` also sets the bits of the classes that do not exist yet. That is harmless, because `storage_in` only probes the classes a format lists. It also means the class items need no edit here |
| `src/gl/es3_format_tests.rs:33` | `fn also(mask: u32)` | `u64` |
| `src/gl/es3_format_tests.rs:40` | `TEST_HIDDEN_SAMPLED.store(!0 << 11, ..)` | none (inferred `u64`; hides every class from 11 up) |
| `src/gl/es3_format_tests.rs:228` | `sampled_texfmts() & gles_texfmt_bit(..)` | none |
| `src/gl/framebuffer.rs:8163-8166` | `.fold(0, \|m, f\| m \| gles_texfmt_bit(f))` stored into `TEST_EXTRA_SAMPLED` | none (inferred). `src/gl/framebuffer.rs` belongs to plan #155 |
| `tests/es3_caps.rs:564-595` | `(sampled >> i) & 1`, a name list asserted to be `GLES_TEXFMT_ETC2_SRGB_RGBA + 1` long | none. The assertion stays true while new classes are numbered after 25, but they are not printed until the list grows (optional) |
| `tests/es3_caps.rs:598`, `tests/es3_srgb.rs:375` | `sampled & gles_texfmt_bit(..) != 0` | none |

### 1.3 FFI, C headers, arrays sized by the class count

* **No mask crosses FFI.** `GlesTextureBind::format` (`src/backend.rs:833-834`)
  and `GlTexture::texfmt` (`src/gl/context.rs:394`, mirrored as `uint8_t texfmt`
  at `src/gl_context.h:100`) stay `u8`, and their layout
  (`tools/yakogl-tools/src/abilayout.rs`) does not change.
* `src/backend.h:81-91` (`enum gles_texfmt`, `gles_texfmt_bytes`) has stopped
  at `GLES_TEXFMT_R32F` (7) since the ES 3.0 classes were added. It was never
  extended, and no C file switches on a class above 7 (`tools/vivante/cs_ours.c`,
  `tools/hostharness/*.c`, `tests/backend_h_heap_base.c` include it for other
  things). None of the three items touches it.
* `driver/src/v3d/v3d_tex.h:149-177` names the classes in comments only. Its
  `enum v3d_tex_format_id` is the V3D's own numbering.
* **Private class numbers in the same `u8`:** `src/pvr/backend.rs:2093-2102`
  (`PVR_TEXFMT_RAW8..RAW128` = `0xf0..0xf4`, `PVR_TEXFMT_DEPTH_BUFFER` = `0xf5`).
  The new classes (26..39) do not collide. `gles_texfmt_bit` is never called on
  them (it would overflow for `u32` and `u64` alike).
* **Arrays and lists sized by the class count:**
  * `tests/es3_formats_gc.rs:50-77` (`CLASSES: [(&str, u8); 26]`). Its test at
    `:136-146` parses every `pub const GLES_TEXFMT_` line of `src/backend.rs`
    and requires the list to equal it, and `:147-` requires one row per class
    in `docs/es3/gc-formats.md` section 2.
  * `tests/es3_caps.rs:565-591` (the name list above).
  * `src/gl/es3_formats.rs:91` (`FORMATS: [Es3Format; 35]`) is sized by
    formats, not classes, and its length does not change.

  mask-u64 changes none of these. Each class item extends the first one
  (section 5).

## 2. The new classes

### 2.1 Numbers, sizes, formats

Classes are numbered after `GLES_TEXFMT_ETC2_SRGB_RGBA` (25). Each `pub const`
must stay on one line: `tests/es3_formats_gc.rs:140` parses them. The `UI` and
`I` twins are separate classes, following the existing RGBA8UI/RGBA8I pair.
On the GC7000 they share one TE format and differ only in CONFIG2's sign bit
(32-bit: no bit at all). In the GL core the sign drives `tf_capture::decode`'s
sign extension and the integer surface a texture attachment maps to
(`framebuffer.rs` `integer_texture_surface`).

| # | class | bytes | components stored | internal formats stored in it (native) | fallback where the mask lacks it | item |
|---|---|---|---|---|---|---|
| 26 | `GLES_TEXFMT_R32UI` | 4 | R as u32 | GL_R32UI | `GLES_TEXFMT_RGBA32UI` | r32-rg32-classes |
| 27 | `GLES_TEXFMT_R32I` | 4 | R as i32 | GL_R32I | `GLES_TEXFMT_RGBA32I` | r32-rg32-classes |
| 28 | `GLES_TEXFMT_RG32UI` | 8 | R, G as u32 | GL_RG32UI | `GLES_TEXFMT_RGBA32UI` | r32-rg32-classes |
| 29 | `GLES_TEXFMT_RG32I` | 8 | R, G as i32 | GL_RG32I | `GLES_TEXFMT_RGBA32I` | r32-rg32-classes |
| 30 | `GLES_TEXFMT_R8_SNORM` | 1 | R as s8 | GL_R8_SNORM | `GLES_TEXFMT_RGBA8_SNORM`, then `RGBA16F` | narrow-classes |
| 31 | `GLES_TEXFMT_RG8_SNORM` | 2 | R, G as s8 | GL_RG8_SNORM | `GLES_TEXFMT_RGBA8_SNORM`, then `RGBA16F` | narrow-classes |
| 32 | `GLES_TEXFMT_R8UI` | 1 | R as u8 | GL_R8UI | `GLES_TEXFMT_RGBA8UI` | narrow-classes |
| 33 | `GLES_TEXFMT_R8I` | 1 | R as s8 | GL_R8I | `GLES_TEXFMT_RGBA8I` | narrow-classes |
| 34 | `GLES_TEXFMT_RG8UI` | 2 | R, G as u8 | GL_RG8UI | `GLES_TEXFMT_RGBA8UI` | narrow-classes |
| 35 | `GLES_TEXFMT_RG8I` | 2 | R, G as s8 | GL_RG8I | `GLES_TEXFMT_RGBA8I` | narrow-classes |
| 36 | `GLES_TEXFMT_R16UI` | 2 | R as u16 | GL_R16UI | `GLES_TEXFMT_RGBA16UI` | narrow-classes |
| 37 | `GLES_TEXFMT_R16I` | 2 | R as s16 | GL_R16I | `GLES_TEXFMT_RGBA16I` | narrow-classes |
| 38 | `GLES_TEXFMT_RG16UI` | 4 | R, G as u16 | GL_RG16UI | `GLES_TEXFMT_RGBA16UI` | narrow-classes |
| 39 | `GLES_TEXFMT_RG16I` | 4 | R, G as s16 | GL_RG16I | `GLES_TEXFMT_RGBA16I` | narrow-classes |
| (40) | `GLES_TEXFMT_R8` | 1 | R as u8 | (GL_R8, GL_RED) | `GLES_TEXFMT_RGBA8` | not declared (3.3) |
| (41) | `GLES_TEXFMT_RG8` | 2 | R, G as u8 | (GL_RG8, GL_RG) | `GLES_TEXFMT_RGBA8` | not declared (3.3) |

Notes on the table:

* **Not in any class item:** the RGB integer formats (GL_RGB8UI .. GL_RGB32I),
  GL_RGB8_SNORM and the float R/RG formats. The TE has no 3-component format
  (`F:126-131`, `F:190-196`, `F:217-224`: `V_` rows, `tex` NONE). GL_RG32F stays
  with RGBA32F/RGBA16F (wide128 work), and GL_R32F already has its class.
* **Classes 26-29 fit in a `u32` mask** (bits 26-29), so r32-rg32-classes does
  not strictly need mask-u64. Classes 30-39 do not.
* **`gles_texfmt_bytes` (`src/backend.rs:289-301`) gets an arm for every class
  whose size is not 4.** Without one it would answer 4 (the default) and every
  layout would be wrong:
  * 8: RG32UI/I
  * 2: RG8_SNORM, RG8UI/I, R16UI/I
  * 1: R8_SNORM, R8UI/I

  R32UI/I and RG16UI/I are 4 already, but list them anyway so a test can pin
  every class.

### 2.2 How a format chooses its class (src/gl/es3_formats.rs)

* The per-format storage lists are `src/gl/es3_formats.rs:80-88` (`SNORM_ST`,
  `U8_ST` .. `I32_ST`), used by `FORMATS` (`:91-159`). Each class item replaces
  the shared list of the formats it moves with a list of their own, **native
  class first, today's class second**. For example:
  * `R32U_ST = &[GLES_TEXFMT_R32UI, GLES_TEXFMT_RGBA32UI]` for GL_R32UI
    (`:139`).
  * `R8SN_ST = &[GLES_TEXFMT_R8_SNORM, GLES_TEXFMT_RGBA8_SNORM, GLES_TEXFMT_RGBA16F]`
    for GL_R8_SNORM (`:92`).

  The RGB and RGBA formats keep `U8_ST` .. `I32_ST` and `SNORM_ST`.
* `storage_in` (`:183-185`) takes the first listed class the mask has. A back
  end whose mask lacks the new class therefore gets the second entry, which
  is exactly today's class (section 3). A back end that samples neither still
  refuses the format by name (`src/gl/texture.rs:5743-5750`,
  `es3_storage_or_refuse`).
* The class is decided once, at specification (`src/gl/texture.rs:5629-5690`
  `sized_format`, `:5684-5690` for the es3 formats; `:5801-5814` `texfmt_of`),
  and recorded in `GlTexture::texfmt`. Sub-image calls use the recorded class.
* **An integer format now has two classes, both integer.**
  `tests/es3_sized_formats.rs:179-182` asserts "an integer format has one class".
  It must become "every class of an integer format is an integer class, and
  the format is stored natively or refused, never in a float class" (section 5).

### 2.3 Upload (CPU conversion)

All texel uploads of these formats go through `es3_formats::convert_row` /
`encode` (`src/gl/es3_formats.rs:603-666`, `:673-`). They are called by:

* `src/gl/texture.rs:2184-2228` for glTexImage*/glTexSubImage*/glTexStorage*,
  2D, 3D, arrays and PBO sources alike. It sizes rows by `gles_texfmt_bytes`
  through `storage_bytes` (`es3_formats.rs:596-598`), and its 64-texel chunk
  buffer of 16-byte texels covers every new class.

`encode` gets one arm per new class, after `masked(t, fmt.comps, 1)` has
already set the missing components to (0, 0, 0, 1):

| class | encode |
|---|---|
| R32UI, R32I | `put32(dst, 0, ints[0])` |
| RG32UI, RG32I | `put32(dst, 0, ints[0]); put32(dst, 1, ints[1])` |
| R16UI, R16I | `put16(dst, 0, ints[0] as u16)` |
| RG16UI, RG16I | `put16` of `ints[0]`, `ints[1]` |
| R8UI, R8I | `dst[0] = ints[0] as u8` |
| RG8UI, RG8I | `dst[0..2]` from `ints[0..2]` |
| R8_SNORM | `dst[0] = snorm8(fl[0])` |
| RG8_SNORM | `dst[0..2]` from `snorm8(fl[0..2])` |

The bits written are the low bytes of what the RGBA class writes for the same
texel today. The test can therefore compare the native bytes with a slice of
the RGBA bytes (section 5).

Unchanged and still refusing:

* glCopyTexImage/glCopyTexSubImage into an integer or snorm image is refused
  before any texel moves (`src/gl/texture.rs:5724-5733`, `es3_copy_compatible`).
* glGenerateMipmap on any es3_formats texture but RGB10_A2-in-RGBA8 is
  GL_INVALID_OPERATION (`src/gl/texture.rs:4147-4151`).

### 2.4 Read-back and CPU readers

ES 3.0 has no glGetTexImage. A texture's texels are read back only by these
paths:

1. **glReadPixels / glBlitFramebuffer from a texture attachment.**
   `attachment_storage` (`src/gl/framebuffer.rs:479-490`) accepts a texture
   only in RGBA8, sRGB, the float trio or an integer class that
   `integer_texture_surface` (`:411-423`) maps to a surface, and that
   `integer_texture_internalformat` (`:403-408`) confirms is the surface of
   the format's own name. That surface is what glReadPixels reads
   (`fb_target_format_n`, `:899-919`).

   The new classes are not in `integer_texture_surface`. An R/RG integer
   texture therefore stays an incomplete attachment, **as it is today**: the
   RGBA store's surface is not the name's surface, which is what `:396-402`
   explains. Mapping the native classes to their surfaces is the step that
   makes such a texture a render target and readable:
   * R32UI/I -> `GLES_SURF_R32UI/I`
   * RG32UI/I -> `GLES_SURF_RG32UI/I`
   * R8UI .. RG16I -> the matching `GLES_SURF_*`

   The packed integer surfaces have exactly the native classes' layout
   (`src/backend.rs:314-346`). That step is a separate follow-up (section 4,
   step 4), because it edits `src/gl/framebuffer.rs` (plan #155) and changes
   completeness.
2. **Attachment queries** (`GL_FRAMEBUFFER_ATTACHMENT_*_SIZE`,
   `src/gl/framebuffer.rs:2113-2145`, `es3_texture_bits`). The integer arm
   matches the class: RGBA8UI/I -> 8, RGBA16UI/I -> 16, `_ => 32`.
   * R32/RG32 classes: the default (32) is already right. No change.
   * Narrow integer classes: **must** be added (R8UI/I, RG8UI/I -> `first(8)`;
     R16UI/I, RG16UI/I -> `first(16)`). Without them, a GC build reports 32 for
     GL_R8UI, which `src/gl/framebuffer.rs:8157-8207` checks and dEQP-GLES3
     `state_query.fbo` asks. Snorm reads `first(8)` for any class and is
     unaffected.
3. **Transform-feedback capture's CPU texel fetch** (`src/gl/tf_capture.rs:1113-1163`,
   `decode`). A class without an arm makes `bound_of` refuse the capture by
   name (`:1335-1337`). Each item adds its classes' arms:
   * `Texel::I` for the integers, with I8/I16 sign-extended as the RGBA rows
     do, and the missing components 0, 0, 1.
   * `Texel::F([max(c / 127, -1), ..., 0, 1])` for the snorm pair.
4. **CPU mip filters, the alpha scan, EGL image export** (`src/gl/texture.rs:4153-4157`,
   `:4266`, `:4384`; `src/gl/draw.rs:3216-3253`; `src/gl/egl.rs:2672`). They
   act on RGBA8/float/narrow-LA classes only. None can see a new class: no
   mipmap for these formats, and no EGL image of them.

## 3. Back ends that do not sample a new class

The rule: **a back end whose mask lacks the new class stores the format in
exactly the class it stores it in today.** It sees no new class number, so
its texture code is not touched.

* **PowerVR** (`PVR_SAMPLED_TEXFMTS`, `src/backend.rs:190-206`). It samples
  every RGBA integer, RGBA8_SNORM and RGBA32 class, so GL_R32UI stays
  RGBA32UI, GL_R8UI stays RGBA8UI, and so on. `es3_tex_format`
  (`src/pvr/backend.rs:1250-1266`) and its caller (`:4980`) are unchanged. A
  new class would hit `_ => return None` there, but none reaches it.
* **V3D** (`V3D_SAMPLED_TEXFMTS`, `src/backend.rs:233-246`). It has the same
  fallbacks as the PowerVR. `refusal_for` (`src/v3d/texture.rs:109-113`)
  refuses by name any class not in its mask before `tmu_format`'s
  `_ => RGBA8` default (`:217`) could misread one.
* **Mali** (`src/mali/backend.rs:96`, mask 0). Every es3_formats format is
  refused at upload today and stays refused (no class of the list is
  sampled). `texel_format` (`src/mali/backend.rs:1733-1780`) is not reached.
* **GC7000UL** before its item flips the bit: the same. mask-u64 changes no
  bit. A class item adds the class to `GC_SAMPLED_TEXFMTS` in the same commit
  as its `gc_texfmt` arms.

  The trap is `gc_texfmt`'s default (`src/vivante/texture.rs:137` ->
  `unorm_format`, `:175` `_ => A8B8G8R8`): a class that reaches the GC with no
  arm is sampled **silently** as RGBA8, not refused. Each item must therefore
  add the arm first and pin it with a test that iterates every declared
  class. `tests/es3_formats_gc.rs`'s row-per-class test does that once the
  rows exist.

The tests that pin "the same on the other back ends" are in section 5.

### 3.1 The fallback for host tests

`TEST_EXTRA_SAMPLED` / `TEST_HIDDEN_SAMPLED` still let a host test stand in
for any mask:

* `Sampled::none_new()` (`src/gl/es3_format_tests.rs:39-42`) hides every class
  from 11 up, so the formats fall back to the base classes or are refused, as
  today.
* `ALL_NEW` (`:27`) selects the native classes.

### 3.2 GC7000UL render targets

Native R32/RG32 or narrow integer textures do not change which **render
targets** the GC draws. Those are renderbuffers and RGBA integer textures,
`GC_INTEGER_RT_FORMATS` (`src/vivante/backend.rs:341-344`), until the
follow-up of 2.4 item 1. For GL_R32I/UI, which the GC refuses at upload today,
the visible change is this: a dEQP `fbo.completeness` case that attaches one
moves from "refused at glTexStorage" to "incomplete attachment". It was a
failure before and is still one, until the follow-up.

### 3.3 Why GL_R8 and GL_RG8 stay in RGBA8

Mesa samples R8_UNORM as `TEXTURE_FORMAT_L8` with green and blue swizzled to
0 (`F:88`, `F:341-343`, `F:364-370`) and RG8 as `EXT_G8R8` (`F:118`). The
descriptor words are known (2.5).

The GL core, however, stores GL_R8/GL_RG8 (and GL_EXT_texture_rg's GL_RED/GL_RG)
outside es3_formats, in `sized_format` (`src/gl/texture.rs:5642-5643`, `:5658`).
They are **colour-renderable** and **mipmapped**, and every path that makes
them so is keyed on `GLES_TEXFMT_RGBA8`:

* `attachment_storage` (`src/gl/framebuffer.rs:479-490`)
* `fb_target_format_n` (`:911-918`)
* the CPU mip filter (`src/gl/texture.rs:4153-4157`)
* `texture_copy_from_fb` (`src/gl/texture.rs:2461-`)
* the blit's texture surface (`src/gl/blit.rs:175-181`)

A narrow R8 store would make every R8 texture an incomplete attachment, a
regression in `fbo.*` cases that pass the upload today. It would also lose its
generated mipmaps, and there is no `GLES_SURF_R8` / `GLES_SURF_RG8` to render
into.

narrow-classes therefore leaves GL_R8/GL_RG8 in RGBA8 and does **not**
declare classes 40/41. Taking them needs an R8 and an RG8 colour surface on
every back end (Mesa's PE `R8` / `G8R8`, `F:88`, `F:118`) and is a new item,
not part of this plan. The memory saved would be the only gain:
`docs/es3/gc-formats.md` section 4 item 5 says the RGBA classes sample the same
values.

## 4. GC7000UL descriptor words

Every GC7000UL HALTI gate is met: HALTI0 to HALTI5 (`docs/es3/gc-formats.md`
section 1).

* An `EXT_FORMAT` value goes into `TEXDESC_CONFIG1.FORMAT_EXT` with
  `CONFIG0.FORMAT` 0 (`D:179-186`).
* CONFIG2 gets `SIGNED_INT8` / `SIGNED_INT16` for a pure-sint format with 8- or
  16-bit channels, and nothing for 32-bit (`D:160`, `D:187-189`).
* INT_FILTER is off for every class below: it is unorm only, `F:296-338`.
* The swizzle is identity: `get_texture_swiz`, `F:346-384`, patches only
  R8_UNORM and DXT1.
* The gates:
  * `EXT_FORMAT`: HALTI0 (`S:409-410`)
  * snorm: HALTI1 (`S:416-417`)
  * pure integer: HALTI2 (`S:419-421`)
  * the R32/RG32 integer override: `halti >= 5` (`F:286-291`)

The `GC_TEX_FORMAT_EXT_*` constants are not in `src/vivante/regs.rs` at
`794c0216`, which defines only those listed at `regs.rs:868-893`. The
regs-words item (in review, branch `connected/01a198d2-4bbf-7244-8144-7262e9395c38`)
adds every one below, each with its `X` line, under the names in the "regs.rs
name" column. Use those once it lands. Until then the values are `X`'s.

| class | TE word (CONFIG1.FORMAT_EXT unless noted) | value | CONFIG2 | swizzle | Mesa row | header `X` (in-tree +1) | regs.rs name |
|---|---|---|---|---|---|---|---|
| R32UI | `TEXTURE_FORMAT_EXT_R32I` | 0x23 | - | identity | R32_UINT: table `EXT_R32F` (`F:137`), replaced on HALTI5 (`F:286-288`) | `X:95` (`:96`) | `GC_TEX_FORMAT_EXT_R32I` |
| R32I | `TEXTURE_FORMAT_EXT_R32I` | 0x23 | - (32-bit: no sign bit, `D:187-189`) | identity | R32_SINT: `F:136`, `F:286-288` | `X:95` | as above |
| RG32UI | `TEXTURE_FORMAT_EXT_G32R32I` | 0x24 | - | identity | R32G32_UINT: table `EXT_G32R32F` (`F:209`), replaced on HALTI5 (`F:289-290`) | `X:96` (`:97`) | `GC_TEX_FORMAT_EXT_G32R32I` |
| RG32I | `TEXTURE_FORMAT_EXT_G32R32I` | 0x24 | - | identity | R32G32_SINT: `F:210`, `F:289-290` | `X:96` | as above |
| R8_SNORM | `TEXTURE_FORMAT_EXT_R8_SNORM` | 0x0e | - | identity | `F:89` | `X:80` (`:81`) | `GC_TEX_FORMAT_EXT_R8_SNORM` |
| RG8_SNORM | `TEXTURE_FORMAT_EXT_G8R8_SNORM` | 0x0f | - | identity | `F:119` | `X:81` (`:82`) | `GC_TEX_FORMAT_EXT_G8R8_SNORM` |
| R8UI | `TEXTURE_FORMAT_EXT_R8I` | 0x15 | - | identity | `F:90` | `X:85` (`:86`) | `GC_TEX_FORMAT_EXT_R8I` |
| R8I | `TEXTURE_FORMAT_EXT_R8I` | 0x15 | `SIGNED_INT8` 0x00040000 | identity | `F:91`; `D:187-188` | `X:85`; `X:165` (`:166`) | `GC_TEX_FORMAT_EXT_R8I`, `GC_TEXDESC_CONFIG2_SIGNED_INT8` (`regs.rs:841`) |
| RG8UI | `TEXTURE_FORMAT_EXT_G8R8I` | 0x16 | - | identity | `F:120` | `X:86` (`:87`) | `GC_TEX_FORMAT_EXT_G8R8I` |
| RG8I | `TEXTURE_FORMAT_EXT_G8R8I` | 0x16 | `SIGNED_INT8` | identity | `F:121`; `D:187-188` | `X:86`; `X:165` | as above |
| R16UI | `TEXTURE_FORMAT_EXT_R16I` | 0x18 | - | identity | `F:102` | `X:88` (`:89`) | `GC_TEX_FORMAT_EXT_R16I` |
| R16I | `TEXTURE_FORMAT_EXT_R16I` | 0x18 | `SIGNED_INT16` 0x00080000 | identity | `F:103`; `D:187,189` | `X:88`; `X:166` (`:167`) | `GC_TEX_FORMAT_EXT_R16I`, `GC_TEXDESC_CONFIG2_SIGNED_INT16` (`regs.rs:842`) |
| RG16UI | `TEXTURE_FORMAT_EXT_G16R16I` | 0x19 | - | identity | `F:147` | `X:89` (`:90`) | `GC_TEX_FORMAT_EXT_G16R16I` |
| RG16I | `TEXTURE_FORMAT_EXT_G16R16I` | 0x19 | `SIGNED_INT16` | identity | `F:148`; `D:187,189` | `X:89`; `X:166` | as above |
| (R8, not declared) | `TEXTURE_FORMAT_L8` in **CONFIG0** | 0x02 | - | R 0 0 A (G, B -> 0) | `F:88`; `F:364-370` | `X:45` (`:46`) | `GC_TEX_FORMAT_L8` (`regs.rs:869`) |
| (RG8, not declared) | `TEXTURE_FORMAT_EXT_G8R8` | 0x06 | - | identity | `F:118` | `X:72` (`:73`) | `GC_TEX_FORMAT_EXT_G8R8` |

Notes for the code (`src/vivante/texture.rs`):

* **`gc_texfmt`** (`:96-139`) gets one arm per class:
  * `format_ext` as in the table, `config2` for the three signed narrow
    pairs, `int_filter: false`, `decoded: true`, `no_alpha: false`.
  * Mesa writes the identity swizzle and leaves the missing channels to the
    TE. `format_words` (`:183-230`) already writes identity plus the GL core's
    `no_alpha` / `depth_luminance` (`:224-228`), and needs no change.
  * The signed I and unsigned UI arms differ only in `config2`, as the RGBA8
    pair does today (`:117-121`).
* **The tiled copy** picks a page-aligned 4x4-tiled copy for 8-byte texels by
  size (`src/vivante/tiled_copy.rs:135-137`, `wide_texels`). RG32UI/I gets the
  RGBA16 classes' path without an edit.
  * The 1- and 2-byte classes take the linear path that the ES 2.0 L8/A8/LA8
    classes already use (same `gles_texfmt_bytes`-driven stride, `:309-311`).
  * wide128-backend is changing `tiled_copy.rs`. Check the 8-byte rule still
    holds after it lands.
* **The integer look-up** needs no compiler change. `int_tex::float_typed_lookups`
  (`src/shaderc/online/vivante/int_tex.rs`) selects every integer look-up as
  the float one, and the TE returns the raw integers, sign-extended by CONFIG2
  (`docs/es3/gc-formats.md` section 3). It does that for the new classes as
  for the RGBA ones.
* **Unmeasured (board):** the values the TE returns for the components a
  narrow class lacks. GL needs (r, 0, 0, 1) and (r, g, 0, 1), with integer 1
  for an integer sampler.
  * `docs/es3/gc-formats.md` records the TE filling G, B with 0 and A with 1
    for `EXT_R16F`.
  * For `EXT_R8I` / `EXT_R16I` / `EXT_R32I` and their two-channel twins it is
    Mesa's assumption (identity swizzle), not a measurement here.
  * If a board run shows float 1.0 bits in alpha, the fix is CONFIG1
    `SWIZZLE_A = ONE` per class in `format_words`. That is a board item for
    the i.MX8MP run, not a host one.

## 5. Order and test plan

### Step 1: mask-u64 (no behaviour change)

* **Touches:**
  * `src/backend.rs:150-166, 190, 211, 233, 260, 265, 273-286`
  * `src/gl/es3_formats.rs:183`
  * `src/gl/es3_format_tests.rs:27, 33`
  * `tests/es3_sized_formats.rs:19, 23, 568`
  * `tests/es3_formats_v3d.rs:232, 235`
* **Must not touch:** `src/mali/backend.rs` (widen at `src/backend.rs:281`),
  `src/v3d/**`, `src/pvr/**`, `src/gl/texture.rs` and `src/gl/framebuffer.rs`.
  None of them needs an edit (1.2).
* **Overlap with plan #155:** `tests/es3_sized_formats.rs` and
  `tests/es3_formats_v3d.rs` are plan #155's files. The edits are type tokens
  only. Land it as one small commit so connected can order it against #155.
* **Tests:**
  * Add a `src/backend.rs` unit test that the four masks have no bit at or
    above `GLES_TEXFMT_ETC2_SRGB_RGBA + 1`, and that their values equal the
    `u32` values before the change (pin them as literals). That proves the
    change moved no class.
  * Add the `< 64` const assertion (1.1).
  * Run `tools/check.sh --changed origin/main`. It reaches every row, because
    `src/backend.rs` is in scope `all`; use `--local` if needed (CLAUDE.md).
* **Host dEQP:** `tools/cts/host-deqp-run.sh --gpu=vivante --module=gles3`
  (`texture.format`, `texture.specification`). The failure lists must be
  byte-identical to the baseline of gc-host-baseline2.

### Step 2: r32-rg32-classes (classes 26-29, after mask-u64)

It could land without step 1, because bits 26-29 fit in a `u32`. Keep the
order anyway, so `src/backend.rs` is edited by one item at a time.

* **Touches:**
  * `src/backend.rs`: four `pub const`, `gles_texfmt_bytes` (RG32 = 8),
    `GC_SAMPLED_TEXFMTS` += the four, and its doc comment, which today reads
    "Not ... R32I/UI, RG32I/UI".
  * `src/gl/es3_formats.rs`: four storage lists and the `encode` arms.
  * `src/gl/tf_capture.rs`: the `decode` arms.
  * `src/vivante/texture.rs`: the `gc_texfmt` arms. It is held by
    wide128-backend; wait for it.
  * `tests/es3_formats_gc.rs`: `CLASSES` grows to 30. Held by wide128-backend.
  * `docs/es3/gc-formats.md`: four section-2 rows (11 columns, Mesa cell
    citing `F:`/`X:`), and section 4 item 4 updated. Held by gc-host-baseline2.
  * `tests/es3_sized_formats.rs`: plan #155's file, see below.
  * No `src/gl/framebuffer.rs` edit: `es3_texture_bits`' default 32 is right.
* **Host tests** (`tests/es3_formats_gc.rs`):
  1. `storage_in(GC_SAMPLED_TEXFMTS)` of GL_R32UI/R32I/RG32UI/RG32I is the new
     class.
  2. The same four on `PVR_SAMPLED_TEXFMTS` and `V3D_SAMPLED_TEXFMTS` are still
     RGBA32UI/I, and on Mali's mask still `None`.
  3. `gc_texfmt` gives CONFIG1 FORMAT_EXT 0x23 / 0x24 and CONFIG2 0 for both
     signs (compare with the header value, as the existing rows do).
  4. `encode`'s bytes equal the first 4 / 8 bytes of the RGBA32 encoding of
     the same texel.
  5. `gles_texfmt_bytes` of every declared class, pinned in one table.
  6. `tf_capture::decode` of each class.
* **`tests/es3_sized_formats.rs`:**
  * `:179-182` (one class per integer format) becomes "all classes integer".
  * `:189-191` and `:235-237` (`Some(e.storage[0])` on PowerVR/V3D) become
    `Some(*e.storage.last())`, since their native class is now the fallback.
  * `:211`: GL_R32UI on the GC is now `Some(GLES_TEXFMT_R32UI)`.
  * `:574-576`: the GC refusal list shrinks to GL_RGB32UI, GL_RGB32I,
    GL_RGBA32UI, GL_RGBA32I.
  * `all_classes` (`:23-24`) includes 26-29.
* **Host dEQP:** in `texture.format` and `texture.specification`, the upload
  refusals of the R32 and RG32 integer formats disappear (part of the 204
  32-bit refusals in `docs/es3/gc-formats.md` section 5), and no new compile
  refusal appears. Render verdicts on the host mean nothing.

### Step 3: narrow-classes (classes 30-39, after mask-u64; independent of step 2 but numbered after it)

* **Touches:** the same files as step 2, plus `src/gl/framebuffer.rs:2120-2125`
  (`es3_texture_bits`: the narrow integer arms, 2.4 item 2). That last file
  belongs to plan #155; keep it as a separate small commit.
* **Host tests:** the same as step 2, for ten classes:
  1. Pin the CONFIG2 sign bit per signed pair.
  2. `encode`'s bytes are a prefix of the RGBA class's per texel.
  3. `src/gl/framebuffer.rs:8157-8207`'s attachment bits still read 8 / 16
     under a GC build.
  4. PowerVR/V3D storage is unchanged for all ten formats.
  5. A test iterating every `GLES_TEXFMT_*` through `gc_texfmt` asserts that no
     class falls to the `_ => A8B8G8R8` default except RGBA8 itself, ETC2's
     and the narrow LA classes (section 3).
* **Host dEQP:** the outcome lists are unchanged. These formats were stored
  (as RGBA) and sampled before. This is memory only, and a change in the
  lists is a regression.

### Step 4 (follow-up, not one of the three items): R/RG integer textures as render targets

Map the native classes in `integer_texture_surface`
(`src/gl/framebuffer.rs:411-423`), so a GL_R8I .. GL_RG32UI texture is a
complete attachment where `rt_format_drawable` says the surface is drawn.

* On the GC that is every one but RG32I/UI, which wait for `GC_INT_RT_64BPP`
  (int-rt-64bpp).
* On the other back ends nothing changes: they never store the classes.

It moves dEQP `fbo.completeness.renderable.texture.color0.*` and the
`fragment_out` texture cases, and it needs a board run for the GC's writes
into a texture store. It is in plan #155's `framebuffer.rs`.

### Board items (for the issue, not this plan's host work)

The missing-channel fill of `EXT_R8I`, `EXT_R16I`, `EXT_R32I`, the G..R
twins and the narrow snorm formats (section 4). Also the 1- and 2-byte linear
row stride of the new classes on the GC7000UL.
