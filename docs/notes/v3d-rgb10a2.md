<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: RGB10A2 as its own class (render target, sampling, mipmaps)

Design for item `v3d-tex-rgb10a2`. Today the GL core stores a `GL_RGB10_A2`
texture and renderbuffer as RGBA8 on the V3D: two bits of each colour channel
are lost, and alpha is widened from 2 to 8 bits (`GLES_TEXFMT_RGBA8` is the
second entry of the format's storage list, `src/gl/es3_formats.rs:124`). This
note names the one switch that moves it to its own class, every site that
switch reaches, the hardware words from Mesa 26.2.1, the mipmap path, and an
order of commits in which each commit leaves the gate green. No code changes
here.

Line numbers are of main at `6488dae8` and drift; the function names are the
address. Mesa lines are of `third_party/mesa-26.2.1/src/gallium/drivers/v3d/`
unless another path is given. Of that directory only `v3dx_format_table.c`,
`v3dx_format_table.h` and `v3d_format_table.h` are vendored; `v3dx_rcl.c`,
`v3dx_tfu.c`, `v3dx_draw.c` and `v3d_blit.c` are not (see (f)).

## What is true today

Steps 1 to 3 of (e) are on main; step 4 (the TFU) is not.

- **Step 1, the surface and its words** (`735e24e8`): `GLES_SURF_RGB10_A2` = 26
  with its name, 4 bytes a pixel and `rt_format_drawable` arm; the
  `V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2` = 3 constant with its
  `ffi_layout_check.c` assert; the `state::rt_format` row (16F, 64 bpp, clamp
  NONE); the `v3d_job.c` `rt_internal` case; the `clear_words` clamp to [0, 1].
- **Step 2, the GL core** (`089cf587`): `attachment_storage`, the renderbuffer
  surface and bits, `glGenerateMipmap` with the CPU box filter
  (`box_average_rgb10a2`), all keyed on `rt_format_drawable(GLES_SURF_RGB10_A2)`.
- **Step 3, the flip** (`03f4bbd9`): `GLES_TEXFMT_RGB10A2` is in
  `V3D_SAMPLED_TEXFMTS`. A `GL_RGB10_A2` texture, renderbuffer and attachment
  is the 10/10/10/2 class on the V3D; the PowerVR and GC7000 masks do not hold
  it and keep the RGBA8 fallback.
- **Tests on the V3D**: `tests/es3_int_rt_v3d.rs` (the row, clamp NONE, the
  clamped clear words), `tests/es3_formats_v3d.rs` (the TMU row, type 9 XYZW,
  16-bit return; and through the GL API over the real back end with the job
  builder faked: an attachment and a renderbuffer give output format 3, the
  job's target and clear words are the row's, an out-of-range clear colour
  arrives clamped, RGB10_A2 beside an RGBA8 attachment keeps each render
  target's output format and clear words, the CPU mipmap chain averages each
  field). The one thing the host cannot pin is the C `rt_internal` case
  (`v3d_job.c`) from the GL level, because the file fakes `v3d_job_*`; it is
  covered by `src/v3d/state.rs` and the `tests/v3d_*` job rows only for the
  surfaces they build.
- **Not done**: step 4. `rt_texture` (`src/v3d/backend.rs`) still requires
  `GLES_TEXFMT_RGBA8`, the TFU has no 10/10/10/2 input type
  (`V3D_TFU_TTYPE_*` are R8, RG8, RGBA8, R16F, R32F), and the GL core offers
  a chain to the back end only for RGBA8 (`src/gl/texture.rs`). The mipmap
  chain of the class is the CPU's. The TFU input-type value is in Mesa's
  `v3dx_tfu.c`, which is not vendored (see (f)).

The paragraphs below are the design as written before steps 1 to 3, kept
for the sites and the words they name; line numbers have drifted.

## (a) The gate and every site it reaches

**The gate is one bit: `GLES_TEXFMT_RGB10A2` in `V3D_SAMPLED_TEXFMTS`**
(`src/backend.rs:233-247`), read through `sampled_texfmts()`
(`src/backend.rs:273-286`). `GL_RGB10_A2`'s storage list is `[RGB10A2, RGBA8]`
(`src/gl/es3_formats.rs:124`), and `storage_in` takes the first entry the mask
holds (`src/gl/es3_formats.rs:183`). Setting the bit therefore changes the
class of every new `GL_RGB10_A2` texture on the V3D, and nothing on the other
back ends: their masks are separate constants (`PVR_SAMPLED_TEXFMTS`
`src/backend.rs:190`, `GC_SAMPLED_TEXFMTS` `:211`, Mali's own
`SAMPLED_TEXFMTS` `:281`).

The render-target side reads the **same** bit. No second switch is added.
For the new surface (`GLES_SURF_RGB10_A2`, see (b)), `rt_format_drawable`
answers `sampled_texfmts() & gles_texfmt_bit(GLES_TEXFMT_RGB10A2) != 0`
rather than its default yes. It sits beside the sRGB test at
`src/backend.rs:499-505`. The renderbuffer's surface choice in `src/gl`
calls `rt_format_drawable` too.
This works because every back end already treats the bit as "renders and
samples": the PowerVR keeps the class out of its mask precisely because it
cannot render it (`src/backend.rs:186-189`). A host test turns the gate on
with `TEST_EXTRA_SAMPLED` (`src/backend.rs:260`, applied at `:282-285`) and
hides it with `TEST_HIDDEN_SAMPLED` (`:265`), both already in use. No
`HOST_*` override like `HOST_SRGB_RT` (`:493`) is needed. The cost is that a
back end can never sample the class without rendering it. That is the rule
ES 3.0 sets for `GL_RGB10_A2` anyway (colour-renderable and filterable).

Each site below assumes today that RGB10A2 is held as RGBA8:

| site | today | after the bit |
|---|---|---|
| `src/v3d/texture.rs:98` `refusal_for` (message at `:111`) | refuses the class by name | passes it; the message names ETC2 only |
| `src/gl/es3_formats.rs:633` upload conversion | already packs the 10/10/10/2 word | reached on the V3D for the first time |
| `src/gl/framebuffer.rs:432` `attachment_storage`, the clause at `:487-489` | an es3 class other than RGBA8 that has no integer surface is no attachment | needs a branch that maps the class to `GLES_SURF_RGB10_A2`, the way `integer_texture_surface` (`:411`) maps the integer classes; otherwise the texture stops being a render target, which the RGBA8 fallback is today |
| `src/gl/framebuffer.rs:579` `is_color_format` (`GL_RGB10_A2` at `:609`), `:2490` `is_renderable_format`, `:3698` renderbuffer storage | renderbuffer `GL_RGB10_A2` is accepted and held as RGBA8 | its surface is `GLES_SURF_RGB10_A2` when `rt_format_drawable` says so, RGBA8 otherwise |
| `src/gl/framebuffer.rs:2017-2020` renderbuffer bits; `:2113` `es3_texture_bits` (`:2129`) | the renderbuffer says 8/8/8/8; the texture bits already say 10/10/10/2 for the class | the renderbuffer says 10/10/10/2 under the same condition as its surface |
| `src/gl/texture.rs:4181-4190`, in `glGenerateMipmap` (`:4141`) | `INVALID_OPERATION` unless the texture is in the RGBA8 fallback | accepts the class (see (d)) |
| `src/gl/texture.rs:4191-4196` `filtered`, and the CPU box filter from `:4310` | RGBA8, sRGB, F16 and F32 texels and the narrow L/A classes | needs a 10/10/10/2 arm (see (d)) |
| `src/gl/texture.rs:4264-4266`, the call to `backend::generate_mipmap` at `:4274` | only `GLES_TEXFMT_RGBA8` with `es3_fmt == 0` | not reached by the class (see (d)) |
| `src/v3d/backend.rs:1780` `rt_texture` (`:1784`) | a texture rendered into must be `GLES_TEXFMT_RGBA8` | left alone until the TFU commit. The class's render image is a 4-byte image like RGBA8's (`texture::rt_layout`, `src/v3d/texture.rs:374`), but sampling it through `rt_texture` would bind it with RGBA8's TMU row |
| `src/gl/tf_capture.rs:1146` | already decodes the class | none |

## (b) The render-target words

Mesa's row for `R10G10B10A2_UNORM` gives output format `RGB10_A2`, texture
format `RGB10_A2`, swizzle XYZW and return size 16 (`v3dx_format_table.c:71`).
The B10G10R10 and X2 variants on `:72-74` differ only in the swizzle. Mesa
derives the tile-buffer words from the output format:

| word | value | source |
|---|---|---|
| output image format | `V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2` = 3 | `v3dx_format_table.c:71`; the value at `src/broadcom/cle/v3d_packet.xml:230` |
| internal type | `V3D_INTERNAL_TYPE_16F` (6, `src/v3d/ffi.rs:301`) | `v3dx_format_table.c:250-259`: the case it shares with sRGB, R11F_G11F_B10F and RGBA16F |
| internal bpp | `V3D_INTERNAL_BPP_64` (1, `src/v3d/ffi.rs:306`) | `v3dx_format_table.c:260` |
| clamp | `V3D_RENDER_TARGET_CLAMP_NONE` (0, `src/v3d/ffi.rs:308`) | the rule is in `v3dx_rcl.c`, which is not vendored. The repository records it as INT for a pure integer format, NORM for sRGB, NONE for the rest (`src/v3d/state.rs:409-411`, `driver/src/v3d/v3d_job.c:146-147`), and the state unit test pins that rule for every surface (`src/v3d/state.rs:1055-1056`). RGB10_A2 is neither integer nor sRGB, so NONE |
| bytes a pixel in memory | 4 | the 10/10/10/2 word. `gles_surf_bytes_per_pixel` (`src/backend.rs:518`) must agree; `RtFormat::bytes` is pinned to it (`src/v3d/state.rs:427-429`, the test in `tests/es3_int_rt_v3d.rs:63`) |

The unsigned-integer row next to it differs in exactly the type and the
clamp. `GLES_SURF_RGB10_A2UI` is output `RGB10_A2UI` = 2, type 16UI, 64 bpp,
clamp INT (`v3dx_format_table.c:75`, `:282-285`; `src/v3d/state.rs:485`).

Where the words go, for a new surface `GLES_SURF_RGB10_A2` = 26 (after
`GLES_SURF_SRGB8_A8`):

1. `src/backend.rs`: the constant, `gles_surf_name` (`:412`) and
   `gles_surf_bytes_per_pixel` (`:518`). It lies outside the integer range
   `GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST` (`:345-346`), so
   `gles_surf_is_integer` stays false for it, as for the sRGB value 25.
   Also its arm in `rt_format_drawable` (see (a)).
2. `src/v3d/ffi.rs`: `V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2 = 3`, added to the
   list at `:271-280`. Its twin goes into the `_Static_assert` of
   `src/v3d/ffi_layout_check.c:173`, which pins the list against the
   generated packer.
3. `src/v3d/state.rs` `rt_format` (`:444`): one row with 16F, 64 bpp, NONE
   and 4 bytes. `typed` (`:502`) is already true for it, since it lists only
   the three ES 2.0 surfaces as untyped. The `rt_row` fallback (`:492`)
   stays RGBA8.
4. `driver/src/v3d/v3d_job.c` `rt_internal` (`:151`): the job builder's own
   table, keyed by output format. Its comment says it has no RGB10_A2 case
   (`:148-150`). Add the case with the same words (16F, 64, NONE), beside
   the sRGB case at `:155`. This is the C twin of (3).
5. `src/v3d/state.rs` `clear_words` (`:517`): the 16F arm (`:523-530`) packs
   four halves, two to a word, and clamps only for the sRGB surface (`:527`).
   The RGB10_A2 buffer is fixed point, and ES 3.0 clamps the clear colour of
   a fixed-point buffer to [0, 1], so widen the `surf == GLES_SURF_SRGB8_A8`
   test to the new surface. The words are then `[h0 | h1 << 16, h2 | h3 << 16,
   0, 0]`, like the sRGB row's; two words reach a 64-bpp tile buffer
   (`:514-516`). The store quantises to 10/10/10/2; nothing quantises the
   clear on the host side.
6. `src/gl/framebuffer.rs`: the surface choice for the renderbuffer and for
   the texture level, as listed in (a).

The tile buffer holds 16F values, so blending and the colour load see
halves, and the 2-bit alpha is quantised only at the store. That is how the
hardware works (`v3dx_format_table.c:250-259` puts the format in the 16F
case), and the host can pin nothing beyond the words above.

## (c) The TMU format and return size

- The texture data format is `RGB10_A2` = 9 (`driver/src/v3d/v3d_tex.h:153`,
  `src/v3d/ffi.rs:460`), with swizzle XYZW and return size 16, from the same
  Mesa row (`v3dx_format_table.c:71`). The Rust row
  `GLES_TEXFMT_RGB10A2 => (RGB10_A2, XYZW)` already exists
  (`src/v3d/texture.rs:207`, Mesa row quoted in its comment at `:191`), and so
  does the C row (`driver/src/v3d/v3d_tex.c:468`: type 9, 4 bytes, XYZW, 16).
- Return size 16 is the 16-bit TMU output. The class is not in the 32-bit set
  (`gles_texfmt_return_32`, `src/backend.rs:253-255`: R32F, RGBA32F, RGBA32UI,
  RGBA32I), so `texture::return_32` (`src/v3d/texture.rs:233`) is false and
  the program is keyed as for RGBA8. It is a float `sampler2D` /
  `samplerCube` with no integer-sampler key.
- Texels are 4 bytes (`gles_texfmt_bytes` falls through to 4,
  `src/backend.rs:289-301`), so the layout is RGBA8's (`texture::layout`,
  `src/v3d/texture.rs:351`, cpp 4). The GL core's word, R in the low 10 bits
  (`src/backend.rs:110`), is the bit order the TMU reads for R10G10B10A2, so
  the class needs no repacking.
- The unsigned-integer sibling already samples (`src/v3d/texture.rs:208`,
  type 114; `format_is_integer` `:240-243`). RGB10A2 is not an integer format
  and must stay out of `format_is_integer`.
- The class is filterable (`Es3Format::filterable`,
  `src/gl/es3_formats.rs:200`; it is not one of the integer or 32-bit float
  classes), so no completeness rule changes.

## (d) The mipmap path

The V3D has two mipmap paths for RGBA8. Neither takes this class today.

1. **GPU, the TFU.** `v3d_generate_mipmap` (`src/v3d/backend.rs:2311`) calls
   `generate_mipmap` (`:2322`). That builds a TFU mipmap job of type
   `V3D_TFU_TTYPE_RGBA8` (`:2360`), and only for a texture a job rendered into
   (`rt_texture`, `:1780`, which requires `GLES_TEXFMT_RGBA8` at `:1784`). The
   job packer takes five types: R8, RG8, RGBA8, R16F and R32F
   (`driver/src/v3d/v3d_tex.h:301-305`, the switch at
   `driver/src/v3d/v3d_tex.c:710-721`). The GL core offers a chain to the back
   end only when `t.texfmt == GLES_TEXFMT_RGBA8 && t.es3_fmt == 0`
   (`src/gl/texture.rs:4264-4266`). With no TFU in the OSAL, or on any
   refusal, the call answers 0 and is counted by reason (`MipRefusal`,
   `src/v3d/backend.rs:2128`).
2. **CPU, the GL core's box filter** (`src/gl/texture.rs:4191-4196` decides,
   the filter runs from `:4310`) takes RGBA8, sRGB, F16 and F32 texels and the
   narrow classes. It has no branch for a packed 10-bit word. The comment at
   `:4181-4184` says so, and that is why the class gets `INVALID_OPERATION`
   today (`:4186`).

Does the TFU take the format? Mesa's predicate
`v3dX(tfu_supports_tex_format)` (`v3dx_format_table.c:343-376`) lists
`RGB10_A2` among the formats it accepts both for a copy and for a mipmap (the
label at `:357`, in the case list `:348-369`). Only RGB9_E5 and the three
32-bit float formats are refused when the job is a mipmap (`:370-374`). So
the hardware can do it. What the vendored tree does **not** show is the TFU
input-type value that makes the unit average 10/10/10/2 fields, because
`v3dx_tfu.c` is not vendored. `V3D_TFU_TTYPE_RGBA8` averages byte by byte,
which would mix the 2-bit alpha with the neighbouring colour bits; it is
wrong for this class. Do not reuse it.

Mesa's other route, a shader blit in `v3d_blit.c` used when the TFU cannot
do the job, is not vendored. The YakoGL back end does not use it either: it
has no blit program for mipmaps. Here the CPU chain is the fallback.

The plan for the item:

1. **First a CPU chain.** Add an RGB10A2 arm to the box filter: unpack the
   four fields, average each as unsigned normalised with the filter's
   round-half-up, and repack. Add the class to `filtered`. Accept the class at
   `src/gl/texture.rs:4186`, so the test becomes "the class, or the RGBA8
   fallback". This is slow but correct, and needs no hardware word. The class
   never reaches the backend offer at `:4264`, so `rt_texture` and the TFU stay
   as they are.
2. **Then the TFU**, once the input-type value has been read from Mesa's
   `v3dx_tfu.c` (a built `MESA_V3D_TREE` or a Mesa source tarball, see (f))
   and checked byte for byte against the CPU chain with the host rig
   (`tools/v3d/tfu_case_check.c`, `tools/v3d/tfu_case.h`,
   `tests/v3d_tfu_host.rs`). That commit widens `rt_texture`, the type
   chosen in `generate_mipmap`, the packer's cpp switch and the GL core's
   offer condition. It keeps the CPU chain as the refusal fallback.

## (e) Order of the commits

Each commit leaves the CI gate (`tools/check.sh --changed`) green on its own.
The bit is set last but one, so nothing is half on. The rows that reach these
tests: `tests/es3_int_rt_v3d.rs`, `tests/es3_formats_v3d.rs` and
`tests/mipmap_offer_guard.rs` each have a `$V3D_CONFIG` row
(`tools/check.sh:1544-1545`, `:1551-1552`, `:1507-1508`). `src/**` unit tests
run under the V3D in `v3d_lib_tests` (`tools/check.sh:1424-1459`) and under
every `CONFIGS` entry (`tools/check.sh:109`, rows at `:1292-1310`). Those
entries also run the rest of `tests/`, `tests/es3_sized_formats.rs` and
`tests/es3_gl_core.rs` among them, with the PowerVR and GC7000 features only.

1. **The surface and its words, gate off.** Add `GLES_SURF_RGB10_A2`, its
   name and bytes, and its `rt_format_drawable` arm (false everywhere,
   because no mask has the bit). Add the `ffi.rs` constant and its
   `ffi_layout_check.c` assert, the `rt_format` row, the `v3d_job.c`
   `rt_internal` case and the `clear_words` clamp. Tests: the new row in
   `src/v3d/state.rs` `every_colour_surface_has_mesas_row` (`:1020`; the
   array length at `:1022` grows from 26 to 27) and in
   `clear_words_pack_as_the_tlb_clear_does` (`:1069`). The new surface goes
   beside the float rows in `tests/es3_int_rt_v3d.rs`
   `float_rows_and_the_es2_formats` (`:113`), with clamp NONE. A clear case
   with an out-of-range colour, clamped, goes in
   `clear_words_pack_per_internal_type` (`:137`). Leave
   `integer_rows_follow_mesas_format_table` (`:63`, 19 integer rows) alone:
   the new surface is not an integer one. No existing assertion changes,
   because no GL path produces the surface yet.
2. **The GL core, still off.** The surface for the renderbuffer and for the
   texture level, `attachment_storage`'s branch, the renderbuffer bits, and
   the `glGenerateMipmap` acceptance with the CPU filter arm, all keyed on the
   class or on `rt_format_drawable`. All are off, since no mask holds the bit.
   New tests in `src/gl` turn the class on with `Sampled::also`
   (`src/gl/es3_format_tests.rs:33`, which sets `TEST_EXTRA_SAMPLED`), in the
   style of that file. They cover a 10/10/10/2 renderbuffer, a complete
   framebuffer with the texture attached, and a mipmap chain averaged field by
   field. This commit touches `src/gl`, so its gate run is the full `--local`
   set (CLAUDE.md); keep it free of V3D edits.
3. **The flip.** Add `GLES_TEXFMT_RGB10A2` to `V3D_SAMPLED_TEXFMTS`, reword
   `src/v3d/texture.rs:111` and the doc comment at `src/backend.rs:230-232`.
   Tests that change, and why:
   - `tests/es3_formats_v3d.rs` `the_sampled_mask_is_the_rows` (`:231`). Its
     doc at `:226-229` and its refused list at `:237` name RGB10A2, and the
     assertion at `:236` says the mask is exactly the rows. Move RGB10A2 into
     `ROWS` (`:195`; the array goes from 16 to 17 rows: type 9, XYZW, 4 bytes,
     not 32-bit, Mesa `:71`), and drop it from the refused list, which leaves
     the three ETC2 classes. `each_class_is_mesas_row_in_rust_and_in_c`
     (`:249`) and `each_classs_record_carries_its_type_and_swizzle` (`:276`)
     then cover it from `ROWS`. Remove the separate tail at `:266-268`
     ("RGB10A2's row is there ... though the class is not sampled").
   - `tests/es3_sized_formats.rs`, the V3D block at `:232-239`. Change the
     assertion at `:239` from `GLES_TEXFMT_RGBA8` to `GLES_TEXFMT_RGB10A2`.
     Drop the `.filter(|e| e.name != "GL_RGB10_A2")` from the loop at `:235`,
     so the loop asserts every format's first class on the V3D. Reword the
     comment at `:232-233` ("every class natively ... but RGB10_A2's, as the
     PowerVR") to "every class natively". The PowerVR lines (`:186-197`: the
     comment, the filtered loop at `:189` and RGBA8 at `:197`) and the GC
     line (`:210`, RGBA8) stay as they are, because their masks do not
     change.
   - `src/gl/es3_format_tests.rs:242-248` ("falls back to RGBA8 on every back
     end") runs under `Sampled::none_new()` (set at `:206`), which hides
     every class from 11 up, RGB10A2 included (`:39-41`). It stays true and
     needs no edit.
   - `src/gl/es3_format_tests.rs:414-424`
     (`generate_mipmap_follows_renderable_and_filterable`) runs with no class
     hidden (`Sampled::also(0)`, `:421`). Under the V3D `--lib` row the
     texture is now in the class, and `glGenerateMipmap` must still be
     `GL_NO_ERROR`. Commit 2's acceptance guarantees that. Reword the doc at
     `:414` ("stored as RGBA8") to "in its class or as RGBA8".
   - `tests/es3_gl_core.rs:350-380` (the renderbuffer sizes, `[8; 4]` at
     `:376`) and `:483` run only in `CONFIGS` (PowerVR and GC7000 features),
     where the gate is off, so they stay. Do not add `gpu-v3d` to them in
     this item.
   - `tests/es3_tf_sampler.rs:757-760` already accepts either class.
   - `docs/es3/backend-caps.md` (the RGB10_A2 rows at `:71` and `:241`),
     `docs/es3-cts-failures.md` (RC2 "Left", `:127`, and the V3D "Done since"
     at `:126`, also `:56`) and `docs/es3/sized-formats.md` (`:60-71`, `:96`,
     `:106`) change in the same commit.
4. **The TFU, if measured.** The input type, `rt_texture`'s class test, the
   packer's cpp switch, the GL core's offer condition
   (`src/gl/texture.rs:4264-4266`), and a byte-exact host test against the
   CPU chain. Tests only gain rows: `tests/mipmap_offer_guard.rs` pins
   RGBA8 chains (its module doc, `:1-16`) and gets an RGB10A2 sibling rather
   than a changed row.

No commit of this item moves `tests/reference/*.rs` or `tests/fixtures/`.
Those goldens build RGBA8 jobs, and the new `rt_internal` case is reached
only by a job with the new surface. If a gate row says otherwise, regenerate
in that commit (CLAUDE.md).

## (f) Checking the words against Mesa with `MESA_V3D_TREE` unset

With `MESA_V3D_TREE` unset, the gate builds what it reads from Mesa out of
the vendored tree (`tools/v3d/tex_check.sh:10-14`, `tools/v3d/README.md`
lines 100-107, through `tools/v3d/mesa_ref.sh` into `build/v3d-check/mesa/`).

- **Texture words.** `tools/v3d/tex_check.sh` builds
  `driver/src/v3d/v3d_tex.c` against Mesa's own format table and compares
  type, swizzle and return size by format name. The mapping from name to
  pipe format is `tools/v3d/tex_check_mesa_fmt.c:41`, which already has
  `"RGB10A2"` → `PIPE_FORMAT_R10G10B10A2_UNORM`. The gate runs it as a row
  (`tools/check.sh:2116-2132`, the call at `:2126`), so the words in (c)
  are checked today, before any code change. Run `tools/v3d/tex_check.sh` by hand, or that row.
- **Render-target words.** The output, internal type and bpp come from
  Mesa's switch (`v3dx_format_table.c:220-341`). The repository encodes
  them by hand with citations (`src/v3d/state.rs:444`, `tests/es3_int_rt_v3d.rs`).
  The enum values are checked against Mesa's generated packer by
  `src/v3d/ffi_layout_check.c` (gate row at `tools/check.sh:1762`), which
  covers the new constant once it is added there. For a byte-exact check of
  a whole job, use `tools/v3d/drm_shim_ref.sh`. That script does not read
  `MESA_V3D_TREE`. Its `build` form takes a Mesa 26.2.1 tarball and a work
  directory (`tools/v3d/drm_shim_ref.sh:11-15`), and its `run` form draws a
  `tools/v3d/gl_ref.py` case on the drm-shim with `V3D_DEBUG=clif`
  (`:16-20`). Pass GPU id 42 for the Pi 4, since 71 is the default. The two
  cases today, `clear` and `triangle`, draw into an RGBA texture in a GLES 2
  context (`tools/v3d/gl_ref.py:85-108`). An RGB10_A2 case needs a GLES 3
  context and a `GL_RGB10_A2` texture. That is a `tools/v3d` change, made in
  the item's commit 1 or later. The CLIF it writes shows Mesa's
  render-target (`TILE_RENDERING_MODE_CFG_RENDER_TARGET_PART1`) and
  clear-colour words for the case, to diff against what `v3d_job.c` emits.
- **The clamp and the TFU type** live in `v3dx_rcl.c` and `v3dx_tfu.c`,
  which the vendored tree does not hold. The clamp can be read from the CLIF
  of that drm-shim case. The TFU type can be read from the source in the
  tarball `drm_shim_ref.sh build` extracts, or from a built
  `MESA_V3D_TREE`, or measured on the board.
