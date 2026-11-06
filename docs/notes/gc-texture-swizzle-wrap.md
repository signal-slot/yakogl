<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: texture swizzle and wrap R as Mesa's etnaviv sets them

Design note of plan #310, item gc-sampler-note (part 1; part 2 is
`gc-shadow-compare.md`). The GC7000UL back end (`src/vivante`) ignores two
pieces of ES 3.0 sampler state that the GL core already hands it in
`GlesTextureBind` (`src/backend.rs:939` `swizzle`, `:907` `wrap_r`): it builds
CONFIG1's four swizzle fields from the format alone and it sends the S wrap
mode on the W axis. This note fixes the words Mesa writes for both, the
mapping from `GlesSwizzle` to `GC_SWIZZLE_*`, the exact lines to change, the
host test cases for the later item, and a HOST-ONLY dEQP baseline. **No code
changes with it, and nothing here was run on a board.**

## 1. Sources

Mesa 26.2.1, tag `mesa-26.2.1` (`third_party/mesa-26.2.1/UPSTREAM`). As in
`docs/es3/gc-formats.md` section 1 and `gc-etc2-formats.md` section 1, the
etnaviv driver is **not** in `third_party/`; it was read from the same
release's source tree in the deps cache, and a reviewer opens the same files in
a checkout of the tag. Paths are relative to Mesa's `src/`; the letters are
those of `gc-formats.md` plus three:

* `D` `gallium/drivers/etnaviv/etnaviv_texture_desc.c`, `F`
  `gallium/drivers/etnaviv/etnaviv_format.c`, `X`
  `etnaviv/hw/common_3d.xml.h`, `N` `etnaviv/hw/state_3d.xml.h` (the repo's
  `driver/src/vivante/hw/state_3d.xml.h` is an older cut that has no
  `NTE_DESCRIPTOR_SAMP_CTRL0` bits, so `N` is the only place they can be read);
* `T` `gallium/drivers/etnaviv/etnaviv_translate.h`.

Own code is cited at the tip of this note's baseline, `1949cd0c`. Two items
running at the same time edit `src/vivante/texture.rs` (etc2-srgb-desc) and
`src/backend.rs` (r32-core), so the **line numbers of those two files will
drift**; every one below is given with the function that holds it.

## 2. What Mesa writes

### 2.1 The swizzle: CONFIG1 of the texture descriptor

A HALTI5 GC7000 samples through the descriptor array (`NTE_DESCRIPTOR`,
`D:554` installs `etna_create_sampler_view_desc`). `etna_texture_desc_fill`
(`D:140`) builds the 256-byte descriptor once per sampler view:

* `D:150-152` `swiz = get_texture_swiz(view format, view swizzle_r, _g, _b,
  _a)`; the four swizzles are the **view's own**, i.e. what the gallium state
  tracker already composed from the GL texture's base format and
  `GL_TEXTURE_SWIZZLE_*` (Mesa core, `st_texture`/`util_format_compose_swizzles`,
  not etnaviv code). etnaviv takes the result as it is.
* `D:183-186` `DESC_SET(CONFIG1, ... | VIVS_TE_SAMPLER_CONFIG1_HALIGN(...) |
  swiz)`: the swizzle is OR-ed into CONFIG1, the same word that carries
  `FORMAT_EXT`, the array bit and HALIGN.
* The fields: `N:1660-1675` `VIVS_TE_SAMPLER_CONFIG1_SWIZZLE_R` bits 8-10,
  `_G` 12-14, `_B` 16-18, `_A` 20-22. The descriptor's CONFIG1 has the same
  layout as the legacy sampler's (`D` writes `VIVS_TE_SAMPLER_CONFIG1_*`
  names into the descriptor); `src/vivante/regs.rs:817-828`
  `texdesc_swizzle_r/g/b/a` are `(x & 7) << 8/12/16/20`, equal.
* The values: `X:117-122` `TEXTURE_SWIZZLE_RED 0, GREEN 1, BLUE 2, ALPHA 3,
  ZERO 4, ONE 5`. `F:372-378` pins `PIPE_SWIZZLE_X/Y/Z/W/0/1` to those six
  by `STATIC_ASSERT`, so a gallium swizzle goes in unchanged.
  `src/vivante/regs.rs:993-998` `GC_SWIZZLE_R..GC_SWIZZLE_1` are the same
  numbers.

`get_texture_swiz` (`F:346-384`) applies **two** format overrides to the
view's swizzle before packing it, and nothing else:

1. `F:356-362` for `PIPE_FORMAT_DXT1_RGB`: every component that reads W
   (alpha) is changed to ONE, because the hardware decodes DXT1 RGB and RGBA
   alike. (This is the S3TC counterpart of what `GcTexfmt::no_alpha` does for
   ETC2 RGB here; no DXT1 class exists in this driver.)
2. `F:364-370` for `util_format_linear(fmt) == PIPE_FORMAT_R8_UNORM`
   (`F:340-344` `texture_format_needs_swiz`): R8 is emulated with the TE's L8,
   which returns (L, L, L, 1); a view component that reads Y or Z is changed to
   ZERO so that GL's (R, 0, 0, 1) holds. A component that reads X, W, 0 or 1 is
   kept: X is L, and the format's W is 1.

So **Mesa composes a view's swizzle with the format's by replacing the
channels the format cannot supply**, not by a general matrix product. For
every other texture the hardware format already returns GL's base-format
channels (RGBA8 (r,g,b,a); L8 (l,l,l,1); A8 (0,0,0,a); A8L8 (l,l,l,a)) and
the view's swizzle goes straight into the field. There is no other
composition in etnaviv, and none for the integer formats (`D:187-189` sets only
the CONFIG2 sign bits for them, `D:160` `sint`).

The legacy (non-descriptor) sampler writes the same value through
`etnaviv_texture_state.c` (`get_texture_swiz` is shared); GC7000UL has
`NTE_DESCRIPTOR`, so only `D` matters here.

### 2.2 Wrap R: W in SAMP_CTRL0

The wrap modes of a sampler state are in `NTE_DESCRIPTOR_SAMP_CTRL0`, written
per slot by `emit_desc_sampler_state` (`D:297-313`, the `SAMP_CTRL0(slot)`
store is `D:308`):

* `D:106-108` `UWRAP(translate_texture_wrapmode(ss->wrap_s)) | VWRAP(...wrap_t)
  | WWRAP(...ss->wrap_r)`: **W takes the sampler state's own `wrap_r`**, not
  S. `D:106-113` is `etna_create_sampler_state_desc`, so it is the gallium
  sampler state (the GL texture's, or the bound sampler object's, wrap_r).
* The fields: `N:1952-1960` `UWRAP` bits 0-2, `VWRAP` bits 3-5, `WWRAP` bits
  6-8 (`__MASK 0x1c0`, `__SHIFT 6`); `src/vivante/regs.rs:723-731`
  `samp_uwrap/vwrap/wwrap` are `x & 7`, `(x & 7) << 3`, `(x & 7) << 6`.
* The values: `T:164-190` `translate_texture_wrapmode`: REPEAT -> `X:107`
  `TEXTURE_WRAPMODE_REPEAT 0`, CLAMP and CLAMP_TO_EDGE -> `CLAMP_TO_EDGE 2`
  (`X:109`), MIRROR_REPEAT -> `MIRRORED_REPEAT 1` (`X:108`); the border and
  mirror-clamp modes fall back to those with an `XXX`. `X:110` has a
  `CLAMP_TO_BORDER 3` that no GL ES 3.0 mode needs. GL's three modes and the
  hardware's are the same three numbers (`src/vivante/texture.rs` `gc_wrap`
  and `GlesWrap`'s doc say so).
* `D:250-251`: a 1D sampler view masks VWRAP out and forces `REPEAT` there
  (`SAMP_CTRL0_MASK = ~VWRAP__MASK`, `SAMP_CTRL0 = VWRAP(REPEAT)`). No other
  view overrides a wrap field, and `D:316-331` `emit_desc_sampler_ctrl0`
  (`(ss->SAMP_CTRL0 & sv->SAMP_CTRL0_MASK) | sv->SAMP_CTRL0`) only adds
  INT_FILTER and, for a Z/S format with a linear mip filter, forces the MIP
  field to NEAREST (`D:324-328`).
* The target: `T:528-541` `translate_texture_target` returns
  `TEXTURE_TYPE_3D` for **both** `PIPE_TEXTURE_3D` and `PIPE_TEXTURE_2D_ARRAY`,
  and `D:162-172`, `D:183-185` tell an array apart with `CONFIG1.TEXTURE_ARRAY`. Mesa sets
  `WWRAP` from `wrap_r` for every target (2D, cube, array) alike; nothing in
  etnaviv makes the array case special.

Both swizzle and wrap are **sampler/view state, not shader state**: no
compiler change and no per-draw shader key is needed for either on a
non-128-bit texture.

## 3. What the back end does today

At `1949cd0c`, `src/vivante/texture.rs`:

* `format_words` (the function, ~`:244`), the `config1` expression
  (`:285-289`):
  `texdesc_swizzle_r(GC_SWIZZLE_R)`, `_g(if lum { R } else { G })`, `_b(if lum
  { R } else { B })`, `_a(if fmt.no_alpha || t.no_alpha != 0 { 1 } else { A })`
  with `lum = t.depth_luminance != 0` (`:284`). Nothing reads
  `t.swizzle`. `grep -rn 'wrap_r\|compare_op\|\.swizzle' src/vivante` finds no
  reader of `GlesTextureBind::{swizzle, wrap_r, compare_op}`.
* `emit_slot`, the wrap closure and `samp_ctrl0` (`:539-575`):
  `uwrap = gc_wrap(t.wrap_s, t.width <= 1)`, `vwrap = gc_wrap(t.wrap_t,
  t.height <= 1)`, and `samp_ctrl0 = samp_uwrap(uwrap) | samp_vwrap(vwrap) |
  samp_wwrap(uwrap) | ...` (`:570`).

`tests/gc_npot_repeat.rs:533-536` **pins** `WWRAP == UWRAP` for the 2D and cube
shapes it draws ("WWRAP follows UWRAP in the back end ... a 2D or cube lookup
has no third coordinate, so it is not a GL-visible state"). Keep that for
those shapes (see 5.4): it is evidence about the board, since every textured
board case so far ran with W = S.

`docs/es3-cts-failures.md:70` and RC18 (`:357-370`) already list both gaps for
the GC7000UL ("`texdesc_swizzle_*` and `samp_wwrap`").

## 4. The mapping `GlesSwizzle` -> `GC_SWIZZLE_*`

`GlesTextureBind::swizzle` (`src/backend.rs:939`) is in the texture's **own
base-format terms**: R, G, B, A are the components the base-format rule gives
before any swizzle, so an L8's G is L and an RGB texture's A is 1, "whatever
the storage holds". `GlesSwizzle` (`src/backend.rs:955-962`): 0 `SAME`
(itself), 1 `RED`, 2 `GREEN`, 3 `BLUE`, 4 `ALPHA`, 5 `ZERO`, 6 `ONE`.
`swizzle_source(k, s)` (`src/backend.rs:968`) turns a component into the
channel it reads: `SAME` -> `k`, `RED..ONE` -> `s - 1`, i.e. **0-3 a channel,
4 the constant zero, 5 the constant one**. Those last two numbers are exactly
`GC_SWIZZLE_0 = 4` and `GC_SWIZZLE_1 = 5` (`regs.rs:997-998`), and 0-3 are
`GC_SWIZZLE_R..A`; no table is needed. The V3D back end already composes it
this way (`src/v3d/texture.rs:300` `tmu_swizzle`, citing
`util_format_compose_swizzles`).

**Composition with what the descriptor does today.** Let `base[c]` be the
`GC_SWIZZLE_*` the descriptor's CONFIG1 uses today for base-format channel
`c` (it is the format's contribution):

```
base[0] = GC_SWIZZLE_R
base[1] = if lum { R } else { G }
base[2] = if lum { R } else { B }
base[3] = if fmt.no_alpha || t.no_alpha != 0 { GC_SWIZZLE_1 } else { A }
```

Then, for component `k`:

```
hw[k] = match swizzle_source(k, t.swizzle[k]) {
    c @ 0..=3 => base[c],          // the channel as the format supplies it
    c         => c as u32,         // 4 -> GC_SWIZZLE_0, 5 -> GC_SWIZZLE_1
}
```

With the all-zero (identity) swizzle `hw == base`, byte for byte today's CONFIG1
(`swizzle_source(k, SAME) = k`). This is what Mesa does for every case in 2.1
except R8 and DXT1, and it gives the same answer for them:

| texture / storage | what the TE returns | `base` | note |
|---|---|---|---|
| RGBA8, RGBA16F, ... (`no_alpha` 0) | (r,g,b,a) | R,G,B,A | identity row |
| GL_RGB, GL_RG, GL_RED on RGBA8 storage, ETC2 RGB (`fmt.no_alpha`) | (r,g,b,x) | R,G,B,**1** | a view that reads ALPHA gets 1, as GL says (alpha base channel is 1); one that reads GREEN/BLUE of a GL_RED/GL_RG texture reads the storage's G/B, which the GL core stores as 0 (`src/backend.rs` "missing ones stored as (0,0,0,1)") |
| L8 / A8 / LA8 narrow (`GC_TEX_FORMAT_L8/A8/A8L8`) | (l,l,l,1), (0,0,0,a), (l,l,l,a) | R,G,B,A (+1 for alpha when `no_alpha`) | the hardware format carries the replication, as Mesa's L8_UNORM/A8_UNORM/L8A8_UNORM rows do with an identity swizzle |
| depth, `depth_luminance` | (d,0,0,1) | R,R,R,1 | green and blue read red; a user swizzle on top reads these |
| the integer classes | (r,g,b,a) integer | as RGBA8 | see "integer" below |
| 128-bit planes | per plane (r,g) | see "128-bit" below | **identity only** |

**The narrow R8/RG8 classes (item narrow-core/narrow-classes).** When they exist,
R8 would be the TE's L8 returning (l,l,l,1) with GL base (R,0,0,1): `base` must
be `[R, 0, 0, 1]` for it, which is exactly Mesa's `F:364-370` rule (a view
component that reads Y or Z gets ZERO). The clean way is a `swz: [u32; 4]`
field in `GcTexfmt` that `base` reads in place of the `lum`/`no_alpha`
expressions, so that adding a class is one row, not one more `if`. RG8 on an
A8L8-like format would be `[R, G, 0, 1]` the same way. The swizzle change
below should therefore build `base` through one small function that those
items extend (see 5.1).

**Integer formats.** `D:187-189` shows Mesa passes the view's swizzle as it is
for them; the existing alpha-ONE for an RGB integer texture already relies on
it (`texture.rs` comment at `format_words`). Whether the TE's constant ONE
returns the integer 1 or the float 1.0 (`0x3f800000`) for an integer format is
**not established anywhere in this tree or in Mesa's text**; GL wants the integer
1. The first board check for the swizzle item is an RGBA8UI texture with
swizzle `(ONE, ZERO, ...)` read through a `usampler2D`. If it returns the
float pattern, the compiler must fix it (an integer `texture()` result already
goes through `src/shaderc/online/vivante/int_tex.rs`).

**The two-plane 128-bit path (`wide128_plane_format`, `texture.rs:~154`; the
descriptor words `:245`, `:300`).** Mesa copies the descriptor whole for the
companion (`D:404-415` `memcpy(companion, buf, TEXTURE_DESC_SIZE)`), so a
swizzle goes into **both** planes unchanged, and the shader join takes the x,y
of plane 0 and the x,y of the companion (`etnaviv_nir_lower_128bit.c:90-160`:
`orig_comp < 2 ? sampler : companion`). A plane's descriptor swizzle
therefore permutes *within* that plane's (r,g) pair, for both planes at once:
a view swizzle `(G,R,B,A)` would swap R/G and also swap B/A, which is wrong for
the other plane, and any swizzle that reads a channel of the other plane (`BLUE`
on component 0) cannot be done by the descriptor at all. Mesa does not guard
against it. **Decision for the implementation: a 128-bit binding keeps the
identity swizzle in both planes (`hw = base`, the user's swizzle ignored), as it
does today**; the swizzle on a 128-bit class is then applied by nobody until
the plane join (`src/shaderc/online/vivante/wide128.rs`) learns a swizzle, which
needs the per-draw key (`wide128-shader`) to carry it. No dEQP swizzle case
uses a 32-bit-per-channel texture (all 83 are unorm 2D: alpha, luminance,
luminance_alpha, R8, RG8, RGB8, RGBA8; `es3fTextureSwizzleTests.cpp:256-262`),
so this costs no CTS case. Say so in the code comment, and put it in
`GlesTextureBind::swizzle`'s doc as a back end's known limit.

**Shadow (compare) look-ups.** `tf_capture.rs:~679` records the GL rule that
the swizzle does not apply to a comparison. Whether the TE swizzles the depth
before it compares is not read from Mesa (Mesa's lowering path takes
`tex_swizzle` in `nir_lower_tex_shadow`, `etnaviv_nir_lower_texture.c:463-467`,
for the shader route only). The implementation writes the identity swizzle for a
binding whose `(compare_op & 16) == 0` (a compare is on); the shadow item
(`gc-shadow-compare.md`) owns that line.

## 5. The exact change

### 5.1 Swizzle: `src/vivante/texture.rs`, `format_words` (`:284-289`)

Replace the `lum` / `texdesc_swizzle_*` lines of `config1` with a function that
builds `hw` and use its four values:

```rust
/// The swizzle the descriptor writes: the format's channels (`base`) and the
/// binding's GL_TEXTURE_SWIZZLE_* composed as Mesa's view swizzle is
/// (etnaviv_format.c:346-384, etnaviv_texture_desc.c:150-152).
pub fn descriptor_swizzle(fmt: &GcTexfmt, t: &GlesTextureBind) -> [u32; 4] {
    let lum = t.depth_luminance != 0;
    let base = [
        GC_SWIZZLE_R,
        if lum { GC_SWIZZLE_R } else { GC_SWIZZLE_G },
        if lum { GC_SWIZZLE_R } else { GC_SWIZZLE_B },
        if fmt.no_alpha || t.no_alpha != 0 { GC_SWIZZLE_1 } else { GC_SWIZZLE_A },
    ];
    if is_wide128(t.format) || (t.compare_op & 16) == 0 {
        return base; // 4: the planes' swizzle is not the user's; a compare is not swizzled
    }
    core::array::from_fn(|k| match swizzle_source(k, t.swizzle[k]) {
        c @ 0..=3 => base[c],
        c => c as u32, // GC_SWIZZLE_0 = 4, GC_SWIZZLE_1 = 5
    })
}
```

and `config1 = texdesc_format_ext(...) | texdesc_swizzle_r(s[0]) | _g(s[1]) |
_b(s[2]) | _a(s[3]) | ...`. Three notes:

* A `const _: () = assert!(GC_SWIZZLE_0 == 4 && GC_SWIZZLE_1 == 5)` next to it
  states what the `c => c as u32` arm relies on. `use crate::backend::swizzle_source`.
* `(compare_op & 16) == 0` is the "compare is on" test of
  `GlesTextureBind::compare_op` (`src/backend.rs:895`: 16 = no compare). A zeroed
  `compare_op` (0, which the host tests' `bind()` leaves) means GL_NEVER *with*
  compare on; the rule `(& 16) == 0` would then take the identity branch for
  every hand-built bind in `src/vivante/backend_tests.rs`. That is harmless for
  the identity swizzle those tests expect, but the **new tests must set
  `compare_op = 16`** to reach the swizzle (the GL core always does).
  Drop this clause from `descriptor_swizzle` if the shadow item has not landed
  by then; add it with the shadow item.
* No other file changes: `build_descriptor` calls `format_words`
  (`texture.rs:~300`), the companion plane is a copy, and the tiled copy path
  (`tiled_copy.rs`) moves texels, not channels. `gc_texfmt` and the
  `GcTexfmt` struct are unchanged unless the narrow R8/RG8 items add `swz`.

### 5.2 Wrap R: `src/vivante/texture.rs`, `emit_slot` (`:539-575`)

After `vwrap`:

```rust
/* W: the third axis of a 3D texture. A 2D, cube or array lookup has no
 * third coordinate; those keep S there as before (tests/gc_npot_repeat.rs
 * pins it, and every textured board case ran with it). A depth of one is
 * one texel, so clamp is exact, as for S and T. */
let wwrap = if t.type_ == GLES_TEXTYPE_3D { gc_wrap(t.wrap_r, t.layers <= 1) } else { uwrap };
```

and `samp_wwrap(wwrap)` at `:570`. `t.layers` is a 3D texture's depth
(`build_descriptor` uses `t.layers.max(1)` as `depth` for volumes, ~`:333`;
`format_words` maps `GLES_TEXTYPE_3D` to `GC_TEX_TYPE_3D`).

The incomplete-texture stand-in (`texture.rs:~510-520`, the 1x1 black texel
with `wrap_s/t = CLAMP_TO_EDGE` and the rest `ZERO`) has `type_ =
GLES_TEXTYPE_2D`, so it stays on `uwrap`.

### 5.3 The one-texel rule for R (question d)

`gc_wrap(w, one_texel)` (`:539-547`) returns CLAMP when the axis is one texel,
because a 1-wide axis repeated or mirrored "would sample outside the image"
and gl_test case 24 (a 1x1 texture with the default REPEAT) came out black.
For R the same rule applies to a **3D** binding whose depth is 1
(`t.layers <= 1`): the cause is the hardware's wrap on a one-texel axis, which
does not depend on which axis it is; mip levels below level 0 have depth
`max(1, d >> l)`, but a depth-1 level 0 has no deeper level, so testing level 0
(as S/T test `t.width`/`t.height`) is sufficient. It does **not** apply to a
2D, cube or 2D-array binding: they have no R coordinate to wrap, `wwrap` is
`uwrap` there (already one-texel-clamped for a 1-wide texture), and a 2D
array's layer is selected, not wrapped (ES 3.0 clamps the layer index to [0, layers-1]), so it is the one place where a hardware W = REPEAT might wrap a
layer index. Mesa sets `wrap_r` from the GL state on an array too (2.2); the
current back end sets it from S (usually REPEAT), and `texture.filtering.2d_array`
has not been judged by a render on this part. If the board shows layers
wrapping, the array's `wwrap` becomes `GC_TEX_WRAP_CLAMP_TO_EDGE`; the first
board run of the item should print the 2D-array cases with both values.

### 5.4 Files

Edit: `src/vivante/texture.rs` (the two spots above, plus the doc comment on
`format_words`), `src/backend.rs` (a sentence in `GlesTextureBind::swizzle`'s
doc: a 128-bit class ignores it on the GC7000). Add `tests/es3_sampler_state_gc.rs`.
`tests/gc_npot_repeat.rs` must still pass unchanged (2D and cube keep
`WWRAP == UWRAP`). Docs: `docs/es3-cts-failures.md:70` and RC18
(`:357-370`), `docs/es3/backend-caps.md` (GC7000UL column of the swizzle and
wrap R rows). No generated file moves (`src/vivante/internal.rs` is the
shader-table output; this change touches no shader), no `YakoglBackend` /
table-struct field is added, so the `grep -rl 'YakoglBackend {' tests src` rule
of CLAUDE.md has nothing to recheck.

## 6. Host test cases for `tests/es3_sampler_state_gc.rs`

Style of `tests/es3_formats_gc.rs` and `tests/es3_wide128_planes_gc.rs`: the
pure ones call `format_words(&bind, false)` / the sampler-word function;
the stream ones go through the GL API on the stand-in OSAL, draw, and decode the
descriptor and the `NTE_DESCRIPTOR_SAMP_CTRL0(slot)` store out of the
submitted words with the harness of `es3_wide128_planes_gc.rs` (`writes`,
`descriptor`, `draw`, `:292-425`; an integration test cannot share
`src/vivante/backend_tests.rs`'s private `Rig`, which is where
`a_texture_with_no_alpha_samples_alpha_one` (`:2939`) decodes a
descriptor field with `(config1 >> 20) & 7`). Factoring `samp_ctrl0` into a
`pub fn sampler_ctrl0(t: &GlesTextureBind) -> u32` (the expression at
`texture.rs:568-574` plus `int_filter_wanted`) makes the wrap cases pure; if the
item prefers not to, they are the stream cases below. Because `driver/src/vivante/hw/state_3d.xml.h` lacks the
`NTE_DESCRIPTOR_SAMP_CTRL0` bits (2.1), field positions are pinned as
`tests/gc_npot_repeat.rs:86-108` pins them, by number and with the Mesa line in
the comment (`N:1952-1960`).

Swizzle (CONFIG1 bits 8/12/16/20, 3 bits each):

1. **Identity.** An all-zero `swizzle` gives today's word for RGBA8 (R,G,B,A),
   GL_RGB (`no_alpha`: A = ONE), a narrow L8 (`no_alpha`), ETC2 RGB
   (`fmt.no_alpha`), `depth_luminance` (R,R,R,ONE), RGBA8UI, RGBA16I, RGBA16F,
   R32F: pin the five words (these are the "byte for byte" regression).
2. **Every value on every component.** RGBA8, each of the four components set
   to each of RED, GREEN, BLUE, ALPHA, ZERO, ONE (24 cases): the field equals
   `[0,1,2,3,4,5]` for those values, and the other three fields stay identity.
   Also `SAME` and an explicit own channel give the same word.
3. **The dEQP set.** The nine `swizzleCases` of `es3fTextureSwizzleTests.cpp:287-295`
   (`all_red .. one_one_red_green`) on RGBA8: `bgra` = (B,G,R,A), `abgr` =
   (A,B,G,R), `one_one_red_green` = (1,1,R,G).
4. **no_alpha composes.** GL_RGB (`no_alpha = 1`): `swizzle = (ALPHA, ...)`
   reads `GC_SWIZZLE_1` (the base alpha is 1); `swizzle[3] = RED` reads
   `GC_SWIZZLE_R` (the override is of the *base* channel, not the output slot);
   `swizzle[3] = ZERO` is 0; `swizzle[0] = BLUE` of a GL_RG texture reads
   storage B.
5. **depth_luminance composes.** `depth_luminance = 1`, `swizzle = (SAME, BLUE,
   GREEN, ALPHA)`: (R, R, R, 1) with green/blue swapped is still (R,R,R,1);
   `swizzle[0] = ALPHA` reads `GC_SWIZZLE_1`.
6. **Narrow L8/A8/LA8** (`GLES_TEXFMT_L8/A8/LA8`): the format words are
   untouched (`GC_TEX_FORMAT_L8/A8/A8L8`), a swizzle changes only CONFIG1's
   swizzle bits.
7. **Integer and other classes keep their words.** For RGBA8UI, RGBA8I,
   RGBA16UI, RGBA16I, RGB10A2UI, RGBA8_SNORM, R11G11B10F, RGB9E5, RGBA16F, R16F,
   R32F, ETC2 RGB/RGBA/PT: with a non-identity swizzle CONFIG0, CONFIG2 and every
   CONFIG1 bit except 8-22 equal the identity word.
8. **128-bit planes ignore it.** RGBA32F/UI/I (`TEST_EXTRA_SAMPLED` is
   `#[cfg(test)]`, so as in `es3_wide128_planes_gc.rs` the pure `format_words`
   call is used; the stream version waits for the wide128-flip): a non-identity
   swizzle gives the same CONFIG1 as the identity (`R`,`G` fields identity), and
   the two plane words are equal.
9. **Compare on = identity** (only once the shadow item has landed):
   `compare_op = 2` (LESS) with a swizzle: identity word; `compare_op = 16`:
   the swizzled word.
10. **Constants by name.** `GC_SWIZZLE_R..1` equal `X:117-122` (as
    `es3_wide128_planes_gc.rs` reads `rnndb` from `common_3d.xml.h`: the
    `TEXTURE_SWIZZLE_*` defines are there), and `swizzle_source(k, GLES_SWIZZLE_ZERO)
    == GC_SWIZZLE_0 as usize`, `ONE == GC_SWIZZLE_1`.
11. **GL end to end.** `glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R/G/B/A, ...)`
    on a 4x4 RGBA8 texture, a draw, and the CONFIG1 of the slot's descriptor read
    from the stream: (BLUE, GREEN, RED, ALPHA) lands as that word; a second draw
    after `glTexParameteri(... SWIZZLE_A, GL_ONE)` shows the new word (the
    descriptor is rebuilt per draw); a sampler object does not touch it (the
    swizzle is texture state, ES 3.0.6 3.8.2).

Wrap R (SAMP_CTRL0 bits 0-2 U, 3-5 V, 6-8 W):

12. **3D follows `wrap_r`.** `type_ = GLES_TEXTYPE_3D`, `layers = 4`, nine
    combinations of (S, T, R) over {REPEAT, MIRRORED_REPEAT, CLAMP_TO_EDGE}:
    U, V, W are each the hardware number of their own GL mode (`0, 1, 2`).
13. **A depth of one clamps R.** `layers = 1`, `wrap_r = REPEAT`/`MIRRORED_REPEAT`:
    W = 2; `layers = 2`: W = the mode. S and T keep their own one-texel rule
    (`width = 1` with REPEAT: U = 2).
14. **Non-3D keep W = U.** 2D, cube and 2D-array binds with `wrap_r` different
    from `wrap_s`: W equals U (the pinned behaviour of `gc_npot_repeat.rs`),
    including the 1-wide texture (both 2).
15. **The rest of SAMP_CTRL0 does not move.** For the same binds the bits outside
    0-8 equal the word before the change (MIN/MIP/MAG, UNK21, INT_FILTER).
16. **GL end to end.** A 4x4x4 `GL_TEXTURE_3D` texture with
    `glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, GL_MIRRORED_REPEAT)`
    and `WRAP_S = CLAMP_TO_EDGE`: the sampler register reads U = 2, W = 1; a
    sampler object (`glSamplerParameteri(..., GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE)`)
    bound to the unit overrides the texture's `wrap_r` (the draw's texture loop in
    `gl/draw.rs` swaps the sampler object's wraps in), W = 2.

Cases 12-16 need 3D sampling to reach the back end at all: the GC mask
(`GC_SAMPLED_TEXFMTS`) is about texel formats, not targets, and the GL core
stores 3D textures for it (RC15 lists 3D as a V3D matter), so the GL
end-to-end cases run; if a build refuses a 3D texture, the pure cases still do.

Board checks (not host): (a) a 3D texture with wrap R = MIRRORED_REPEAT sampled
past the end of the depth (dEQP `texture.wrap.*` 3D variants and
`texture.filtering.3d`); (b) the integer ONE/ZERO question (4); (c) the 2D-array
W question (5.3); (d) swizzle on L8/A8/LA8 narrow storage against RGBA8 storage
(dEQP `texture.swizzle.*` rows `luminance`, `alpha`, `luminance_alpha`).

## 7. HOST-ONLY baseline

The host library cannot show a picture (the Linux stand-in OSAL keeps the
command stream; every `glReadPixels` reads black), so **only upload and
compile refusals mean anything here**; every "Fail" with an image message
below is a picture verdict and is *not* a result. Label: HOST-ONLY.

Run: `tools/cts/host-deqp-run.sh --gpu=vivante --module=gles3
--cts=/home/orion/connected-deps/VK-GL-CTS --out=<dir outside the tree> <filter>`
at commit **`1949cd0c`** (`git describe`: the tree was clean when the library
was built; the per-run label later reads `-dirty` because this note was
being written), library built with `YAKOGL_GPU=vivante`, `YAKOGL_SHADERC=online`,
`YAKOGL_REPORT_ES2=OFF`, 256x256 pbuffer; dEQP from VK-GL-CTS `93bca0186` (the
revision `tools/cts/README.md` pins) with the yakogl target applied
(`tools/cts/cts-yakogl-target.patch`, uncommitted in that checkout, which was
fetched into `$CONNECTED_DEPS_DIR/VK-GL-CTS` on this node because it was not
there). Logs were kept outside the working tree (the task's `$TMPDIR`,
`host/run-<filter>/`, with each chunk's `.qpa`). The case lists are the union
of `android/cts/main/gles3-main-*.txt` of that checkout. The main tip at the
time of writing, `68e7e129`, differs from `1949cd0c` in no file of
`src/vivante`, `src/shaderc`, `src/backend.rs`, `src/gl/draw.rs` or
`tests/gc_npot_repeat.rs` (`git diff --stat` empty), so every line cited here
holds there too.

| filter | cases | began | compile refusals | upload refusals (`Texture upload failed`) | NotSupported | picture verdicts (not results) |
|---|---|---|---|---|---|---|
| `functional.texture.swizzle.` | 83 | 83 | 0 | 0 | 0 | 70 Fail / 13 Pass |
| `functional.texture.shadow.` | 432 | 432 | **432** | 0 | 0 | 0 (all 432 stop at the compile) |
| `functional.texture.filtering.3d.` | 153 | 153 | 0 | 0 | 12 | 141 Fail |
| `functional.texture.vertex.` | 174 | 174 | 0 | 0 | 0 | 174 Fail |
| `functional.texture.wrap.` | 1,440 | 1,440 | 0 | 0 | 1,296 | 144 Fail |
| total | 2,282 | 2,282 | 432 | 0 | 1,308 | 529 |

Where the numbers come from:

* **Compile refusals, `shadow`: 432 of 432**, all one reason, at the same
  place the local experiment hit (`gc-shadow-compare.md` section 3):
  `online shaderc: the code generator refused this fragment shader:
  Select(Unsupported("an ES 3.00 texture look-up (a shadow sampler, an integer
  result, a gradient, an offset or a cube fetch) is not selected yet"))`, source
  `src/shaderc/online/select.rs:2686-2691`; dEQP's message is `Failed to compile
  shader program at glsTextureTestUtil.cpp:411`. The group is 3 targets (2d,
  cube, 2d_array) x 6 filters x 8 compare functions x 3 formats
  (`depth_component16`, `depth_component32f`, `depth24_stencil8`,
  `es3fTextureShadowTests.cpp:1035-1057`).
* **`swizzle`, `filtering.3d`, `vertex`, `wrap`: no compile and no upload
  refusal.** The GC7000UL accepts the textures and compiles the shaders; what is
  wrong with swizzle and wrap R is a *picture* (the swizzle and the W wrap are
  descriptor words, section 3), which the host cannot see. The whole baseline
  of those gaps is therefore **0 refusals**: a later item's host verdict for
  them is the unit tests of section 6, not this run, and the board's dEQP run
  (#28) is the only render verdict.
* **NotSupported, `filtering.3d` 12**: `GL_EXT_texture_sRGB_R8` (6) and
  `GL_EXT_texture_sRGB_RG8` (6) are not offered (`es3fTextureFilteringTests.cpp:75,78`).
  **`wrap` 1,296**: ASTC 1,008 (`ASTC not supported`) and the EAC/ETC2
  sRGB/punch-through/R11/RG11 formats 288 (`Compressed texture format not
  supported at gluTexture.cpp:142`); `etc1`, `etc2_rgb8`, `etc2_eac_rgba8`
  and `rgba8` run (36 cases each, `Image verification failed` is a picture).
* **What `texture.wrap` covers.** It is S and T of 2D textures only (36
  combinations per format, `texture.wrap.rgba8.<s>_<t>_<filter>_<pot|npot>`);
  **wrap R is exercised by `texture.filtering.3d.combinations.*`**
  (`<min>_<mag>_<s>_<t>_<r>`, 27 R/S/T combinations for `linear_linear`,
  `es3fTextureFilteringTests.cpp:1775-1792`), by `texture.vertex.3d` (57
  cases) and the 3D cases of `texture.mipmap` and `texture.specification`, which
  set `GL_TEXTURE_WRAP_R` too.
* **`vertex` 174** (2d 39, cube 39, 2d_array 39, 3d 57): picture verdicts only;
  vertex-stage look-ups compile.


## 8. Implementation items, as briefs

Two small items; both edit `src/vivante/texture.rs`, so they follow
`etc2-srgb-desc` (running) and can be one item or two.

**gc-swizzle** (host work only; no board item).
* Files: `src/vivante/texture.rs` (`format_words`: add `descriptor_swizzle`,
  use it for the four `texdesc_swizzle_*` terms; the exact code is in 5.1),
  `src/backend.rs` (one sentence in the `swizzle` doc: a 128-bit class ignores it
  on the GC7000), `tests/es3_sampler_state_gc.rs` (cases 1-8, 10, 11 of section 6),
  docs: `docs/es3-cts-failures.md` (`:70` and RC18 `:357-370`),
  `docs/es3/backend-caps.md` (GC7000UL column), `docs/es3/gc-formats.md` if it
  lists the swizzle fields.
* Words: CONFIG1 bits 8-10 / 12-14 / 16-18 / 20-22 (`regs.rs:817-828`), values
  `GC_SWIZZLE_R..1` = `X:117-122`; nothing else moves. No `GcTexfmt`,
  `YakoglBackend` or other table field is added; no generated file moves.
* Host check: `tools/check.sh --changed origin/main` against the current main tip;
  `cargo test --features host-test,gpu-vivante,shaderc-online --test
  es3_sampler_state_gc` and the in-crate `src/vivante/backend_tests.rs`
  (`a_texture_with_no_alpha_samples_alpha_one`, `a_luminance_depth_texture_...`,
  `narrow_luminance_alpha_textures_...`), which pin today's identity words and
  must still pass unchanged.
* Host dEQP: no change expected (0 refusals before and after, section 7); say so.
  Board: `texture.swizzle.*` (83) and the integer ONE/ZERO check (4).

**gc-wrap-r**.
* Files: `src/vivante/texture.rs` (`emit_slot`: `wwrap`, 5.2),
  `tests/es3_sampler_state_gc.rs` (cases 12-16), docs as above. Optional pure
  helper `pub fn sampler_ctrl0(t)` so the wrap cases need no stream.
* Words: SAMP_CTRL0 bits 6-8 (`regs.rs:729`, `N:1958-1960`); 2D, cube and
  2D-array keep W = U (`tests/gc_npot_repeat.rs:533-536` must stay green).
* Board: `texture.filtering.3d.combinations.*` (27 R/S/T cases) and the
  2D-array layer question (5.3).
* Not in scope here and listed for the planner: the 128-bit classes' swizzle
  (needs the `wide128-shader` key to carry it); the narrow R8/RG8 `swz` row
  (`narrow-classes`).
