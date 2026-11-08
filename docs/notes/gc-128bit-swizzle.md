<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: GL texture swizzle, no_alpha and wrap R on two-plane RGBA32 textures

Design note for the flip of RGBA32F/UI/I into `GC_SAMPLED_TEXFMTS`
(item gc-wide128-flip) and for a compiler change after it. **No code changes
with it and nothing here ran on a board.** Cites: Mesa 26.2.1 etnaviv, read
only (`src/gallium/drivers/etnaviv/`; not vendored, `third_party/mesa-26.2.1/UPSTREAM`
names the tag), and our tree at `17e91019`. Siblings:
`gc-128bit-texture-sampling.md` (the planes, "What is left" caveats),
`gc-texture-swizzle-wrap.md` (the swizzle and wrap R change for every other
class; item gc-swizzle is making it in `src/vivante/texture.rs`).

## 1. The question

An RGBA32 texture is two G32R32 planes, sampled in two slots; the shader
joins `(a.x, a.y, b.x, b.y)`. A descriptor swizzle permutes only within the
four lanes **one plane returns**, and the join reads only `.xy` of each
plane. So the descriptors can express none of: a swizzle that moves a channel
across the pair (`(B,G,R,A)`: component 0 must come from plane 1), ZERO or
ONE on lanes 0-3 of the joined value (the join never reads a plane's
`.z/.w`), or `no_alpha`'s A = ONE (A is plane 1's `.y`, which is the G field
of plane 1's descriptor, not its A field).

## 2. What Mesa does

- **The descriptor is copied whole.** The companion is
  `memcpy(companion, buf, TEXTURE_DESC_SIZE)` plus new level addresses
  (`etnaviv_texture_desc.c:404-415`). The view's swizzle, from
  `get_texture_swiz(sv->base.format, swizzle_r..a)` (`:150-152`, ORed into
  CONFIG1 at `:183-186`), is therefore in **both** planes. `get_texture_swiz`
  (`etnaviv_format.c:346-384`) overrides only DXT1 RGB and R8; 128-bit formats
  are not special-cased there. `etnaviv_texture_state.c:195-197` (legacy path)
  is the same.
- **The shader key has no swizzle for them.** `etna_get_fs`/`etna_get_vs` put
  `tex_is_128bit` and `sampler_companion[]` into the key
  (`etnaviv_context.c:146-150, 198-202`). The `tex_swizzle` they fill
  (`:190-193`) is for `nir_lower_tex_shadow` only (`has_sample_tex_compare`,
  `etnaviv_nir_lower_texture.c:463-467`).
- **The join ignores it.** `lower_128bit_sampler`
  (`etnaviv_nir_lower_128bit.c:89-170`) clones the look-up onto the companion
  and builds `nir_vec4(tex.x, tex.y, clone.x, clone.y)` (`:161-165`); the
  gather path joins 16-bit halves (`:115-151`, `orig_comp < 2 ? sampler :
  companion`, `:117`).
- **Result.** A swizzle on an RGBA32 view is applied by the descriptor to
  each plane separately. `(G,R,B,A)` swaps x/y inside plane 0 (right for R/G) and inside plane 1 too (wrong: B and A must stay). `(B,G,R,A)` makes plane 0 `.x` read its `.z`, a lane the join discards. ZERO/ONE land in lanes the join
  drops or in the wrong plane's pair. Mesa has no handling and no guard; it
  is a latent defect there, not a design to copy. For `no_alpha` a GL_RGB32F
  view carries A = ONE, plane 1's `.w` field, which the join never reads, so
  Mesa returns the stored alpha. dEQP swizzle
  cases do not use 32-bit channels (all 83 are unorm 2D,
  `gc-texture-swizzle-wrap.md` section 4), so no Mesa-based expectation exists
  for ES 3.0 correctness here: the answer is ours to decide.

## 3. What our code does today (by reading; no scratch test was needed)

- `format_words` (`src/vivante/texture.rs:253-299`) builds CONFIG1 from the
  format alone: `texdesc_swizzle_r(R)`, `_g(G)`, `_b(B)`,
  `_a(ONE if fmt.no_alpha || t.no_alpha != 0 else A)` (`:293-296`); it never
  reads `t.swizzle`. A 128-bit binding takes the plane format (`:252`) and
  the same fields. Plane 1 is a copy but for the level addresses (`:303-307`,
  `emit_slot`). So today: the user's swizzle is **ignored** (identity wins);
  `no_alpha` writes A = ONE into both descriptors, **harmlessly** (the field
  affects lane `.w`, which the join does not read). The result for an
  RGBA32 binding with `swizzle = (B,G,R,A)` is the unswizzled texel, silently.
  That is also what item gc-swizzle's `descriptor_swizzle` leaves for
  `is_wide128` (`gc-texture-swizzle-wrap.md` section 4, "identity only").
- The join (`src/shaderc/online/vivante/wide128.rs:135-243`) has no notion
  of a swizzle: per split look-up it emits `tex`, `StoreLocal mask 0b0011`,
  `tex` on the companion, `StoreLocal mask 0b1100` with `swizzle [0,1,0,1]`
  (`:213-222`), `LoadLocal` (`EXTRA = 4`, `:127`).
- The key is `Planes { mask, companion[] }` only (`:47-54`), built by
  `wide128_planes(sense, &companion)` (`src/shaderc/online/backend.rs:2150-2155`)
  from the companions `wide128_companions` computes per draw
  (`src/gl/draw.rs:2040-2075`, called in `wide128_select`, `:2119-2125`); the
  compile cache key hashes `mask` and `companion[]` (`backend.rs:2202-2212`),
  and `ProgW128Variant::planes` (`draw.rs:225`) is compared by value
  (`PartialEq`, `wide128.rs:47`), so a new field in `Planes` is part of the
  variant identity for free.
- `no_alpha` for an RGB integer texture is **not set** by the GL core:
  `texture_has_no_alpha` (`src/gl/draw.rs:694-705`) tests `t.format`, the
  *upload* format (`src/gl/texture.rs:2790`), against `GL_RGB`, `GL_RG`,
  `GL_RED`, ... but not `GL_RGB_INTEGER`/`GL_RG_INTEGER`/`GL_RED_INTEGER`. A
  `GL_RGB32I` texture uploaded with `GL_RGB_INTEGER` therefore binds
  `no_alpha = 0`; its alpha is whatever the storage holds. GL_RGB32F with
  `GL_RGB` does bind `no_alpha = 1`. (Unverified by a test; section 7 pins it.)
  RGB32UI/I and RG32UI/I are stored as RGBA32I/UI classes (`es3_formats.rs:
  155-156, 162`), so they reach this path.
- **Wrap R.** `samp_ctrl0` is computed once in `emit_slot` (`texture.rs:
  570-577`) and written to the binding's slot and, unchanged, to the
  companion (`:592-594`, `write_sampler`). Whatever item gc-swizzle makes the
  W field (`wwrap`), the companion gets the same word: nothing to add for
  planes. 3D RGBA32 textures use TYPE_3D descriptors whose plane layout is
  `second_plane_offset(t, l) = layers * ...` (`tiled_copy.rs`), already per
  volume; the wrap logic is plane-blind.
- **Integer samplers.** `wide128::split_planes` runs **before**
  `int_tex::float_typed_lookups` (`int_tex.rs:27-45`), which only retypes
  `Op::Tex` results `I32/U32 -> F32` (the register holds the integers
  verbatim). A post-join value built by the join therefore reaches the
  selector as a local of the look-up's type; anything the join adds as a
  constant must be built in that type (below).
  `int_filter_wanted(t)` (`texture.rs:467`) reads `gc_texfmt(t.format)`, whose
  128-bit rows are `int_filter: false` (`:104`), and `wide128_plane_format`
  sets no INT_FILTER either (`:167-177`): the integer planes use NEAREST /
  no INT_FILTER, which GL requires for an integer texture anyway.

## 4. Where the swizzle must be applied: decision

Options considered:

1. **Descriptors only (as today).** Cannot express the cases (section 1); the
   swizzle is silently ignored. A wrong picture with no message. Rejected as
   the final state; it is the interim state only if the flip is guarded
   (section 5).
2. **Refuse non-identity swizzle on RGBA32 bindings.** Honest but a
   conformance hole (ES 3.0 allows swizzle on every sized format; dEQP has
   none on 32-bit channels, so no CTS case would show it, yet a program that
   uses `GL_TEXTURE_SWIZZLE_*` on an RGBA32F texture, as GPGPU-style code
   does, would draw nothing). Usable **as the interim guard** (section 5).
3. **Identity in both descriptors + the swizzle in the per-draw shader key,
   applied by the join.** *Decided.* The join already builds the four lanes
   from per-lane sources of two look-ups, so a lane source is free to be a
   different lane of either plane or a constant: no new arithmetic.
   `no_alpha` is the same mechanism (lane 3 source = constant ONE). The
   descriptors stay identity (so the plane's own `.x/.y` are the stored
   words, which is all the join reads), so there is no per-plane
   permutation to get wrong. Wrap R stays in the sampler registers, which
   both slots already share.

Why the key and not the program: the swizzle is texture state of the binding
at the draw, like `wide128_sampled` itself; the same `usampler2D` names
different textures at different draws (`gc-128bit-texture-sampling.md`
"What is left" 1). It costs a variant per distinct swizzle per unit; a
non-identity swizzle on an RGBA32 texture is rare, and the cache of
`w128_var` (`PROG_MAX_W128_VARIANTS`) already bounds variants.

**The composed swizzle for a keyed unit** (computed from the binding, same
table as the other back ends, `swizzle_source` in `src/backend.rs:983`):

```
base[c] = c for c in 0..=2;  base[3] = ONE if t.no_alpha != 0 else 3
sw[k]   = match swizzle_source(k, t.swizzle[k]) { c @ 0..=3 => base[c],
                                                  4 => ZERO, 5 => ONE }
```

(`base` of a 128-bit class has no `depth_luminance`/`fmt.no_alpha` terms.)
The identity RGBA32 binding is `[0,1,2,3]`: the key then carries nothing
extra and the code is byte-identical to today's. A `no_alpha` binding is
`[0,1,2,ONE]`: it **does** take the constant (a variant), so an RGB32F
texture returns alpha 1.0 whatever the storage's alpha lane holds. That is
GL's rule, and for float it equals what the core stores; for RGB32I/UI it
makes the answer right even though the core's `no_alpha` test misses the
`*_INTEGER` upload formats (section 3, to be fixed in the core separately:
src/gl/draw.rs:694).

**Descriptors:** `descriptor_swizzle` (gc-swizzle) must return the identity
`(R,G,B,A)` for `is_wide128` **regardless of `no_alpha`** (today A = ONE is
written but is unread; making it pure identity removes the one place a stray
value could ever matter). Both planes are the same words but the level
addresses (unchanged).

## 5. The exact changes

### 5.1 gc-wide128-flip (`src/backend.rs`, `tests/es3_wide128_gc.rs`)

The flip must not make a non-identity swizzle silently wrong. The compiler
change (5.2) is a separate, later item held by plan #328's files; so the flip
lands with an **interim guard**, removed by that item:

1. `src/backend.rs`: next to `swizzle_source` add
   ```rust
   /// What a 128-bit (two-plane) binding's look-up returns in each lane:
   /// a lane of the joined (R,G,B,A) 0..=3, 4 zero, 5 one. no_alpha: lane 3
   /// reads one. Identity: [0,1,2,3].
   pub const fn wide128_lane_sources(swizzle: [u8; 4], no_alpha: bool) -> [u8; 4]
   ```
   built exactly as the formula in section 4 (a `const fn`, a loop; no
   dependency on the vivante module, so `src/gl/draw.rs` and the shaderc
   plumbing can call it with the `texture.rs` held).
2. Interim guard in `wide128_select` (`src/gl/draw.rs:2102-2140`): a keyed
   binding whose lane sources are not `[0,1,2,3]` makes the function return
   `Err("a 128-bit texture with a texture swizzle is not joined with it
   yet")`; the caller already turns an `Err` into `note_draw_refused` and a
   dropped draw with `GL_NO_ERROR` (`draw.rs:4494-4498`, as it does for a
   blend variant). `no_alpha` alone is **not** refused: ONE is only needed
   when the stored alpha is not 1, see below. This is the same by-name
   refusal pattern the planes already use ("no companion sampler slot").
   Since `draw.rs` is the GL core's, it is a small change of its own
   (`gc-wide128-flip` may do it, or a prior item "gc-wide128-key" that also
   threads the swizzle, section 5.2 step 1).
3. `tests/es3_wide128_gc.rs` (it drives `yakogl::shaderc` free functions,
   `:1-25`):
   - the flip pins: `GC_SAMPLED_TEXFMTS` holds the three classes; a plain
     RGBA32F/I/UI draw is not refused and keys one companion;
   - **guard:** RGBA32UI with `swizzle = [BLUE, GREEN, RED, ALPHA]`, then
     `[SAME, SAME, SAME, ONE]`, then `[ZERO; 4]` each yields the refusal
     text and no draw; the identity (`[SAME; 4]`) and `[RED, GREEN, BLUE,
     ALPHA]` (explicit identity) do not;
   - **no_alpha:** a `GL_RGB32F` texture (`no_alpha = 1`) is not refused and
     keys the same variant as RGBA32F (the interim, because float storage
     alpha is 1: `src/backend.rs` "missing ones stored as (0,0,0,1)"); a
     `GL_RGB32I`/`UI` texture needs the stored alpha to be 1: pin what the
     core stores (the test reads the level texels after `glTexImage2D(...
     GL_RGB_INTEGER ...)`), and if it is 0, either fix `texture_has_no_alpha`
     (add the three `*_INTEGER` bases, src/gl/draw.rs:694; own item) before
     the flip or refuse the case here. Do not flip with that open.
   - **descriptors:** the plane words for a swizzled binding are the
     identity in both planes (a pure `format_words` call with
     `TEST_EXTRA_SAMPLED`, as `es3_wide128_planes_gc.rs`).
   - **wrap R:** a 3D RGBA32F binding with `wrap_r != wrap_s` (after
     gc-swizzle): the slot's and the companion's SAMP_CTRL0 have W = the
     mode of R, bits 0-2 and 3-5 their own (decode from the stream the way
     `es3_wide128_planes_gc.rs` decodes the descriptor pair).
   - **integer sampler:** a `usampler2D` draw of RGBA32UI compiles under the
     key (the retype of `int_tex` runs after the join) and the code has two
     `Tex` and no float conversion; a `isampler` the same.

### 5.2 The compiler item (`src/shaderc/online/vivante/wide128.rs`, held by #328) and its plumbing

Order: (1) plumbing, (2) the pass, (3) remove the guard. Steps 1 and 3 touch
`src/gl/draw.rs` and `src/shaderc/online/backend.rs`, which no listed plan
holds; step 2 waits for #328's release of `src/shaderc/online/vivante/**`.

1. **Key** (`wide128.rs:47-54`): add
   `pub swizzle: [[u8; 4]; UNITS]` (lane sources; 128 bytes; `Copy`, still
   `PartialEq`), `NONE` all identity, `Planes::with(unit, companion)` unchanged
   (identity) and a new `with_swizzle(unit, companion, sw)`;
   `swizzle_of(unit) -> [u8; 4]`. `wide128_planes` keeps its signature (all
   existing callers and tests keep passing) and a sibling
   `wide128_planes_swizzled(sense, &companion, &sources)` fills it;
   `wide128_companions` (`draw.rs:2040`) gains a sibling returning the lane
   sources from the same bound lists and the same `wide` predicate;
   `wide128_select` passes both. The compile key
   (`backend.rs:2202-2212`) hashes the swizzle **only for units whose sources
   are not identity** (an extra `b"wide128-swizzle"` chained hash after the
   existing one, absent for identity), so every existing key and shader
   cache entry is unchanged. `w128_variant_for` already matches by
   `planes ==`.
2. **Pass** (`split_planes`, `:135-243`): generalise the four inserted ops.
   For a unit with sources `sw`, lanes `k` whose `sw[k] < 2` are written from
   the first look-up (`Src { val: lo, swizzle: per-lane sw[k] }`, mask = those
   lanes); lanes with `2 <= sw[k] < 4` from the companion look-up
   (`swizzle: sw[k] - 2`); lanes with `sw[k] >= 4` by one constant store
   (below). The identity unit is exactly today's sequence (masks `0b0011`
   and `0b1100`, `swizzle [0,1,0,1]`), so identity compiles are byte for
   byte today's (`EXTRA` becomes a per-unit count: 4 for identity, +2 for
   a constant store; `new_id`, `comps` and `locals` follow it). A look-up
   none of whose lanes reads a plane need not emit that look-up's `Tex`
   (an all-ZERO swizzle is two stores of constants and one `Tex` is also
   unneeded: keep the first for the dependency on `ret`'s shape; a later
   optimisation, not required).
   **Constants are typed by the look-up's `ret`:** `ScalarType::F32` gets
   `Op::ConstF32` with `0.0`/`1.0` in the lane, `I32`/`U32` get
   `Op::ConstI32` with `0`/`1` (GL's integer zero and one); a float `1.0`
   bit pattern in an integer sampler is the bug to avoid. `int_tex` runs
   after and retypes only `Op::Tex`, so a constant is left alone.
   Refusals stay as they are (shadow, gradient, offset: `:153-166`).
3. **Remove the guard** of 5.1 in `wide128_select`.

### 5.3 Host tests that pin 5.2

In `src/shaderc/online/vivante/wide128.rs`'s tests (or `backend.rs`
`wide128_tests`, `:4434`) and `tests/es3_wide128_gc.rs`:

- identity key: output module equal (instruction list, locals) to the
  unswizzled pass's, and the compile-cache key equal to today's;
- `(B,G,R,A)`: lane 0 reads the companion's `.x`, lane 2 reads the first
  look-up's `.x`; `(A,A,A,A)`, `(ZERO,ONE,ZERO,ONE)` on `sampler2D`
  (`ConstF32`) and on `usampler2D`/`isampler2D` (`ConstI32` 0/1, never
  `0x3f800000`); `no_alpha`-only `[0,1,2,ONE]`;
- the nine dEQP swizzle patterns of `es3fTextureSwizzleTests.cpp:287-295`
  (as `gc-texture-swizzle-wrap.md` section 6, case 3) through the key, as a
  table: each lane's source in the emitted instruction;
- two units, different swizzles in one draw: independent; two draws with
  the same program and different swizzle on one unit give two variants
  (`w128_var` count 2), the same swizzle one;
- vertex stage: a vertex texture of RGBA32F with a swizzle in the vertex
  key only (the vertex stage's `Planes` has its own lane sources);
- `textureSize` and `texelFetch` on a keyed swizzle unit: size unchanged,
  `texelFetch` swizzled like `texture`;
- step 3: the guard text is gone from `wide128_select`, and the §5.1 guard
  tests turn into "compiles with the swizzle" tests.

## 6. Wrap R and the integer path, stated once

- **Wrap R** needs nothing for planes: `samp_ctrl0` is one value written to
  both slots (section 3). The 3D RGBA32 case follows gc-swizzle's
  `wwrap = wrap_r` for `GLES_TEXTYPE_3D`.
- **Integer samplers:** the join's two `Tex` keep their I32/U32 type; the
  swizzle constants are built in that type; `int_tex` retypes the `Tex` ops
  afterwards and leaves constants. INT_FILTER stays off. Whether the TE
  returns an integer `1` for a descriptor-level ONE (the open question of
  `gc-texture-swizzle-wrap.md` section 4, "Integer formats") does not arise
  for planes: the descriptors are identity and the constants are the
  compiler's.
- **Board checks** (not host; none exist yet): an RGBA32UI texture with
  `(B,G,R,A)` and `(ONE,ZERO,...)` read through a `usampler2D`; an RGB32F
  texture's alpha. Both go with the board session that first draws a
  two-plane texture.

## 7. What is left

1. `texture_has_no_alpha` misses `GL_RGB_INTEGER`/`GL_RG_INTEGER`/
   `GL_RED_INTEGER` uploads (section 3): a core item, own test; until it
   lands the key above does not see `no_alpha` for RGB32I/UI.
2. The interim guard is removed with the pass change; until then a draw of a
   swizzled RGBA32 texture is dropped by name, never drawn wrong.
3. Gathers on a keyed unit still need Mesa's 16-bit-halves lowering
   (`gc-128bit-texture-sampling.md` "What is left" 1); the swizzle applies to
   the joined gather like any look-up, after the join.
4. Nothing is measured on the board.
