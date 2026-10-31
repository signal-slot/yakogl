<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: RGB10A2 as its own class (render target, sampling, mipmaps)

Design for item `v3d-tex-rgb10a2`. Today the GL core stores a `GL_RGB10_A2`
texture and renderbuffer as RGBA8 on the V3D, two bits of alpha and two bits
of each colour channel lost (`GLES_TEXFMT_RGBA8` is the second entry of the
format's storage list, `src/gl/es3_formats.rs:124`). This note names the one
switch that moves it to its own class, every site that switch reaches, the
hardware words from Mesa 26.2.1, the mipmap path, and an order of commits in
which each commit leaves the tests green. No code changes here.

Line numbers are of main at `ff7b9882` and drift; the function names are the
address. Mesa lines are of `third_party/mesa-26.2.1/src/gallium/drivers/v3d/`
(only `v3dx_format_table.c/.h` and `v3d_format_table.h` are vendored there;
`v3dx_rcl.c`, `v3dx_tfu.c` and `v3d_blit.c` are not, see the last section).

## What is true today

- There is **no** `RGB10A2_RT` constant and no RGB10A2 bit in any render-target
  mask. The names that exist are `INTEGER_RT_FORMATS` (`src/backend.rs:455`)
  and `SRGB_RT` (`src/backend.rs:488`), and `rt_format_drawable`
  (`src/backend.rs:498`) answers yes for every surface that is neither integer
  nor sRGB, so a new surface value would be "drawable" on every back end
  unless a gate is added next to it.
- There is no `GLES_SURF_*` value for RGB10A2: the surfaces end at
  `GLES_SURF_RGB10_A2UI = 24` and `GLES_SURF_SRGB8_A8 = 25`
  (`src/backend.rs:344`, `:358`).
- The texture side is half there. The GL core has the class
  (`GLES_TEXFMT_RGB10A2 = 13`, `src/backend.rs:117`), its upload conversion
  (`src/gl/es3_formats.rs:633`) and its decode (`src/gl/tf_capture.rs:1146`);
  the V3D has the TMU row (`src/v3d/texture.rs:207`); only the bit in
  `V3D_SAMPLED_TEXFMTS` (`src/backend.rs:233`) is missing, and the doc comment
  above it (`:225-232`) says why.

## (a) The gate and every site it reaches

**The switch is one bit: `GLES_TEXFMT_RGB10A2` in `V3D_SAMPLED_TEXFMTS`**
(`src/backend.rs:233-247`), read through `sampled_texfmts()`
(`src/backend.rs:273`). `GL_RGB10_A2`'s storage list is `[RGB10A2, RGBA8]`
and `storage_in` takes the first entry the mask has (`src/gl/es3_formats.rs:183`),
so adding the bit changes the class of every new `GL_RGB10_A2` texture on the
V3D and nothing on the other back ends (their masks are separate constants,
`src/backend.rs:190`, `:211`). Everything below follows from that class, and
each of these sites still assumes "RGB10A2 is held as RGBA8":

| site | today | after the bit |
|---|---|---|
| `src/v3d/texture.rs:98` `refusal_for` (message at `:111`) | refuses the class by name | passes; reword the message to ETC2 only |
| `src/gl/es3_formats.rs:633` upload conversion | already packs the 10/10/10/2 word | reached for the first time on the V3D |
| `src/gl/framebuffer.rs:432` `attachment_storage`, the clause at `:489` | a non-RGBA8 es3 class with no integer surface is not an attachment | RGB10A2 would become "not renderable"; needs a branch that maps the class to the new surface (like `integer_texture_surface`, `:416`) |
| `src/gl/framebuffer.rs:579` `is_color_format`, `:2490` `is_renderable_format` | renderbuffer `GL_RGB10_A2` accepted, held as RGBA8 (`:609`) | the renderbuffer's surface must be the new one when the back end renders it, RGBA8 otherwise |
| `src/gl/framebuffer.rs:2020` renderbuffer bits, `:2129` `es3_texture_bits` | 8/8/8/8 for the renderbuffer; texture bits already say 10/10/10/2 for the class | renderbuffer bits become 10/10/10/2 under the same condition as the surface |
| `src/gl/texture.rs:4148` `glGenerateMipmap` | `INVALID_OPERATION` unless the class is the RGBA8 fallback | must accept the class (see (d)) |
| `src/gl/texture.rs:4221` back-end mipmap call | only `GLES_TEXFMT_RGBA8`, `es3_fmt == 0` | an RGB10A2 texture never reaches it (see (d)) |
| `src/gl/texture.rs:4260` the CPU box filter | RGBA8, sRGB, F16 and F32 texels only | no 10-bit path |
| `src/v3d/backend.rs:1761` `rt_texture` (`:1765`) | a texture rendered into must be `GLES_TEXFMT_RGBA8` | the render image of an RGB10A2 texture is a 4-byte image (the same `rt_layout`, `src/v3d/texture.rs:352`); the test needs the class and the TMU row must be the class's, not RGBA8's |
| `src/gl/tf_capture.rs:1146` | already reads the class | none |

The render-target half is gated separately, and **that is the gate that does
not exist yet**: a `RGB10A2_RT: bool` beside `SRGB_RT`, true for
`gpu-v3d` only, consulted by `rt_format_drawable` for the new surface (the
same shape as `SRGB_RT`, `src/backend.rs:499-505`, with a host-test override
like `HOST_SRGB_RT`, `:493`). Without it a PowerVR, GC7000 or Mali build would
claim to draw a surface it cannot. The sampling bit and `RGB10A2_RT` are two
switches; the item sets them in one commit (see (e)), because a texture whose
class is native but which is not an attachment would stop being a render
target, which the RGBA8 fallback is today (`src/gl/framebuffer.rs:488`).

## (b) The render-target words

Mesa's row for `R10G10B10A2_UNORM` is output format `RGB10_A2`, texture format
`RGB10_A2`, swizzle XYZW, return size 16 (`v3dx_format_table.c:71`;
the B10G10R10 and X2 variants at `:72-74` differ only in the swizzle). The
tile-buffer words are the ones Mesa derives from the output format:

| word | value | source |
|---|---|---|
| output image format | `V3D_OUTPUT_IMAGE_FORMAT_RGB10_A2` | `v3dx_format_table.c:71`, `:252` |
| internal type | 16F | `v3dx_format_table.c:252-259` (the case shared with sRGB, R11F_G11F_B10F and RGBA16F) |
| internal bpp | 64 | `v3dx_format_table.c:259-260` |
| clamp | NONE | the clamp rule is `v3dx_rcl.c`, which is not vendored; `src/v3d/state.rs:403-406` and `driver/src/v3d/v3d_job.c:149-155` record it as INT for pure integers, NORM for sRGB, NONE for the rest |
| bytes in memory | 4 | the word is one 32-bit store; `gles_surf_bytes_per_pixel` (`src/backend.rs:518`) must agree, a test pins `RtFormat::bytes` to it (`tests/es3_int_rt_v3d.rs:92`) |

Contrast with the existing unsigned-integer row, which differs in exactly the
type and clamp: `GLES_SURF_RGB10_A2UI` is output `RGB10_A2UI`, type 16UI, 64 bpp,
clamp INT (`v3dx_format_table.c:75`, `:282-285`; `src/v3d/state.rs:480`).

Where the words live, for the new surface (say `GLES_SURF_RGB10_A2`, value 26,
after `GLES_SURF_SRGB8_A8`):

1. `src/backend.rs`: the constant, `gles_surf_name` (`:412`),
   `gles_surf_bytes_per_pixel` (`:518`). It is **not** in the integer range
   `GLES_SURF_INT_FIRST..LAST` (`:345-346`), so `gles_surf_is_integer` stays
   false for it; the same for the sRGB value at 25.
2. `src/v3d/state.rs` `rt_format` (`:439`): one `row(...)`, 16F, 64 bpp, NONE,
   4. `typed` (`:497`) is true for it without an edit (it lists only the three
   ES 2.0 surfaces as untyped). The `rt_row` fallback (`:487`) stays RGBA8.
3. `driver/src/v3d/v3d_job.c` `rt_internal` (`:149`): its own table by output
   format, which today has no RGB10_A2 case (its comment says so); add one
   with the same 16F/64/NONE words. This is the C twin of (2); the host tests
   pin the two against each other.
4. `src/v3d/state.rs` `clear_words` (`:512`): the 16F arm packs four halves,
   two a word, and clamps only for the sRGB surface (`:522`). The RGB10_A2
   buffer is fixed point, so its clear colour must be clamped to [0, 1] too
   (ES 3.0 clears a fixed-point buffer with the clamped colour); widen the
   `surf == GLES_SURF_SRGB8_A8` test to the new surface. The words are then
   `[h0 | h1 << 16, h2 | h3 << 16, 0, 0]`, as the sRGB row's. Two words reach
   a 64-bpp tile buffer (`state.rs:508-511`). The TLB quantises to 10/10/10/2
   at the store; nothing quantises the clear on the host side.
5. `src/gl/framebuffer.rs`: the surface choice for the renderbuffer and the
   texture level, per (a).

The V3D blend and read-back see 16F values, so the 2-bit alpha is only
quantised at the store; a blend over a cleared 2-bit alpha reads the halves.
That matches the hardware and is what the dEQP cases measure; it is not
something the host can pin beyond the words above.

## (c) The TMU format and return size

- Texture data format `RGB10_A2` = 9 (`driver/src/v3d/v3d_tex.h:153`),
  swizzle XYZW, return size 16, from the same table row
  (`v3dx_format_table.c:71`). The Rust row is already
  `GLES_TEXFMT_RGB10A2 => (RGB10_A2, XYZW)` (`src/v3d/texture.rs:207`) and the
  C row `driver/src/v3d/v3d_tex.c:468`.
- Return size 16 means the 16-bit TMU output: not in the 32-bit set
  (`gles_texfmt_return_32`, `src/backend.rs:253-255` lists R32F, RGBA32F,
  RGBA32UI, RGBA32I), so `texture::return_32` is false and the program is keyed
  as for RGBA8. The sampler is `sampler2D` / `samplerCube` float; no
  integer-sampler key.
- Texels are 4 bytes (`gles_texfmt_bytes` falls through to 4,
  `src/backend.rs:289-301`), so the layout is RGBA8's (`texture::layout`,
  cpp 4) and the GL core's R-in-the-low-10-bits word
  (`src/backend.rs:110`) is the TMU's R10G10B10A2 order, which is why the
  class needs no repacking. `tests/es3_formats_v3d.rs:204` is the table of
  such rows (add `RGB10A2` there with type 9, XYZW, 4 bytes, not 32-bit).
- The unsigned-integer sibling already samples (`:208`, type 114, `:243`
  `format_is_integer`); RGB10A2 is not an integer format and must stay out of
  `format_is_integer`.
- Filtering: the class is filterable (unlike the integer classes,
  `es3_formats.rs:200`), so no completeness rule changes.

## (d) The mipmap path

Two paths exist for RGBA8 on the V3D and neither takes this class today.

1. **GPU: the TFU.** `generate_mipmap` (`src/v3d/backend.rs:2303`) builds a
   TFU mipmap job with the type `V3D_TFU_TTYPE_RGBA8` (`:2341`) and only for
   a texture a job rendered into (`rt_texture`, `:1761`, which requires
   `GLES_TEXFMT_RGBA8`). The job packer accepts five types (R8, RG8, RGBA8,
   R16F, R32F; `driver/src/v3d/v3d_tex.h:301-305`,
   `driver/src/v3d/v3d_tex.c:711-720`). The GL core calls it only when
   `t.texfmt == GLES_TEXFMT_RGBA8 && t.es3_fmt == 0` (`src/gl/texture.rs:4221`).
2. **CPU: the GL core's box filter** (`src/gl/texture.rs:4260` on), over RGBA8,
   sRGB, F16 and F32 texels. A packed 10-bit word has no branch; the
   comment at `:4144-4147` says so, and that is why the class is refused
   mipmaps today.

Whether the TFU takes the format: Mesa's own predicate
`v3dX(tfu_supports_tex_format)(tex_format, for_mipmap)` returns true for
`RGB10_A2` whether or not the call is for a mipmap (the case list
`v3dx_format_table.c:347-369`, the `RGB10_A2` label at `:357`); only RGB9_E5
and the three 32-bit float formats are excluded for a mipmap
(`:370-374`). So the hardware path exists. What is **not** known from the
vendored tree is the TFU job-type value that makes the unit average
10/10/10/2 fields: `v3dx_tfu.c` is not vendored, and `V3D_TFU_TTYPE_RGBA8`
(a byte-per-channel average) would average the 2-bit alpha with the neighbouring
colour bits and is wrong for this class. Do not reuse it.

Order of preference for the item:

1. **Ship sampling and rendering first; give the class a CPU chain** by adding
   an RGB10A2 arm to the box filter (unpack the four fields, average per field
   as unsigned normalised, repack), reached when the backend call refuses. This
   is slow but correct, needs no hardware word, and removes the
   `INVALID_OPERATION` at `src/gl/texture.rs:4148`. The check at
   `:4147-4152` becomes "the class, or the RGBA8 fallback".
2. **Then the TFU**, once the job-type value is read from Mesa's `v3dx_tfu.c`
   (needs `MESA_V3D_TREE` or a vendored-file addition, which belongs to
   `qpu-ref-counts`' glob and is out of this note) and measured byte-exact
   against the CPU chain with the host rig `tools/v3d/tfu_case_check.c`.
   Widen `rt_texture` and the type match in `generate_mipmap` and
   `v3d_tex_tfu_pack_mipmap`; keep the CPU chain as the refusal fallback
   (`MipRefusal`, `:2246`).

Mesa's blit/`v3d_blit.c` route (a shader-based generate, taken when the TFU
cannot) is not vendored and not used by the YakoGL back end, which has no
blit program for mipmaps; the CPU chain is the fallback here.

## (e) Order of the commits

Each commit leaves `tools/check.sh --changed origin/main` green on its own;
the flip is last-but-one so that nothing is half on.

1. **Surface and words, gate off.** `GLES_SURF_RGB10_A2`, name and bytes;
   `RGB10A2_RT` (`false` everywhere, then the V3D value in commit 3);
   `rt_format` row; `v3d_job.c` `rt_internal` case; `clear_words` clamp.
   `rt_format_drawable` returns the const for the new surface. Tests that
   change: a new non-integer row test in `tests/es3_int_rt_v3d.rs` beside
   `integer_rows_follow_mesas_format_table` (`:61`; leave that test's 19
   integer rows alone, the new surface is not an integer one) and the
   clear-words test (`:122-142`); the unit test at `src/v3d/state.rs:1064`
   gains a case. Nothing existing changes, because no GL path produces the
   surface yet.
2. **GL core, still off.** The surface for the renderbuffer and the texture
   level, `attachment_storage`'s branch, the bits at `:2020`, the mipmap
   acceptance (CPU arm) and the CPU filter arm. Guarded by the mask bit and
   `RGB10A2_RT` both being false on every back end, so the existing tests
   hold; the new tests use `TEST_EXTRA_SAMPLED` and `HOST_*` overrides
   (`src/backend.rs:282-285`, `:493`) in `src/gl`'s unit tests, in the style of
   `src/gl/es3_format_tests.rs`. This commit is the risky one because it
   touches `src/gl`; keep it separate from any V3D edit.
3. **The flip.** Add `GLES_TEXFMT_RGB10A2` to `V3D_SAMPLED_TEXFMTS`, set
   `RGB10A2_RT` for `gpu-v3d`, reword `texture.rs:111`, update the doc comment
   at `src/backend.rs:225-232`, and widen `rt_texture` (`:1765`). Tests that
   change, and why:
   - `tests/es3_formats_v3d.rs` `the_sampled_mask_is_the_rows` (`:227-241`):
     it lists RGB10A2 among the refused names and says the mask is the rows;
     RGB10A2 moves into `ROWS` (`:204` style) and out of the refused list,
     leaving the three ETC2 classes.
   - the same file's `each_class_is_mesas_row_in_rust_and_in_c` tail
     (`:266-268`) states "RGB10A2's row is there, though the class is not
     sampled"; it becomes an ordinary row of the loop above it.
   - `tests/es3_sized_formats.rs` (`:189-222`): the PowerVR and GC lines for
     `GL_RGB10_A2` stay RGBA8 (their masks are unchanged); add a V3D line
     asserting `GLES_TEXFMT_RGB10A2`, and update the comment at `:187-188`.
   - `src/gl/es3_format_tests.rs:242` ("falls back to RGBA8 on every back
     end") and `:414` (mipmap of the fallback) run on the host mask, which does
     not contain the bit; they stay, with their comments narrowed.
   - `tests/es3_gl_core.rs:350` ("held as RGBA8 ... the eight bits it really
     has"): a host build without `gpu-v3d` keeps the fallback and still passes;
     a `gpu-v3d` test of it needs 10/10/10/2. Grep for the size queries in
     that file and in `tests/es2_error_rules_fbo.rs` before the flip.
   - regenerate `docs/es3/backend-caps.md` and `docs/es3-cts-failures.md` rows
     for RGB10A2 (RC2 "left on the V3D"), in the same commit.
4. **The TFU, if measured.** The job type, `rt_texture`'s class test, the
   packer's cpp table and a byte-exact host test against the CPU chain
   (`tools/v3d/tfu_case.h`). Tests change only by gaining rows.

## (f) Checking the words against Mesa, `MESA_V3D_TREE` unset

With `MESA_V3D_TREE` unset the gate builds what it reads from the vendored
tree (`tools/v3d/README.md`, `tools/v3d/tex_check.sh:6-12`).

- **Texture words.** `tools/v3d/tex_check.sh` compiles
  `driver/src/v3d/v3d_tex.c` against Mesa's own `v3dx_format_table.c`
  (`tools/v3d/tex_check_mesa_fmt.c`) and compares type, swizzle and return
  size per format name. The `"RGB10A2"` name is already a row
  (`tests/es3_formats_v3d.rs:268` reads it); the rows it checks after commit 3
  are the ones in (c). Run it, or the gate row that runs it.
- **Render-target words.** The `rt` column and the internal type and bpp are
  Mesa's switch (`v3dx_format_table.c:220-341`); `tests/es3_int_rt_v3d.rs`
  encodes them by hand with citations. For the new surface add the row to the
  test and, if a byte-exact cross-check is wanted, a case in
  `tools/v3d/drm_shim_ref.sh run` (it runs `tools/v3d/gl_ref.py` for the
  `clear` and `triangle` cases on an RGBA8 framebuffer object only; an
  RGB10_A2 renderbuffer case is a new `gl_ref.py` case and a golden, which is
  a `tools/v3d` change and so belongs to a later item once `qpu-ref-counts`
  has released that glob). The CLIF dump it writes shows the render-target
  and clear-colour words of Mesa's job for that case, to diff against
  `v3d_job.c`'s.
- **Clamp and TFU words** are in `v3dx_rcl.c` and `v3dx_tfu.c`, which the
  vendored tree does not hold; the clamp can be read from the CLIF of the
  case above, the TFU type only from the source (needs a tree with those two
  files) or the board.
- The generated files `tests/reference/*.rs` and `tests/fixtures/` change in
  the commit that moves their bytes (CLAUDE.md); commit 1's `v3d_job.c` case
  does not move them unless a golden job builds the new surface.
