<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: the colour-output split for 128-bit render targets (design)

The shader and draw-key half of drawing RGBA32I/UI (and F) colour
attachments as two G32R32F planes: the render-side analogue of the sampler
join already in the tree. The plane layout, the framebuffer expansion and the
board questions are in `docs/notes/gc-128bit-render-targets.md` (its section 4
item 3 is this note); the sampler half is `docs/notes/gc-128bit-texture-sampling.md`
and `src/shaderc/online/vivante/wide128.rs`. Mesa 26.2.1 etnaviv, read only
(tag `mesa-26.2.1`, commit `da14d65e4499e66468094be52bff9ea0915a695e`; Mesa is
MIT, cited here and not copied). Paths are `src/gallium/drivers/etnaviv/`.

**Nothing here is measured and nothing runs on a board.** It is Mesa's source
read against our tree at `b156c874`; no i.MX8MP draw of a 128-bit target has
been made, and `docs/notes/gc-128bit-render-targets.md` section 4 says a board
probe comes before any code. Every file:line below is Mesa's or ours at that
commit.

## (1) The pass

Mesa's `etna_nir_lower_128bit` (`etnaviv_nir_lower_128bit.c:172-214`) runs on a
fragment shader whose key has `has_128bit_rt` (`:181`). For each output
variable at `FRAG_RESULT_DATA0 + n`, `n < ETNA_MAX_128BIT_RTS` (4,
`etnaviv_internal.h:50-52`) and `rt_is_128bit` bit `n` set (`:190-203`), it
creates a companion output `etna_128bit_companion` at
`FRAG_RESULT_DATA0 + rt_companion[n]` (`:22-41`) and widens both types to a
vec4 of the same base (`:15-20,202`), so an integer output stays integer and
no conversion is inserted. Each store to the original output
(`lower_128bit_output`, `:43-87`) is replaced by `vec4(r,g,0,0)` to the
original and `vec4(b,a,0,0)` to the companion, each under its half of the
write mask (`wm & 3`, `wm >> 2`, `:76-79`); missing components read zero
(`:68-71`). Same bits, no float conversion: the PE then stores them raw
(`UIF32`, see the render-targets note section 2).

**Where the second output's index goes.** `rt_companion[n]` is assigned in
`etna_fb_expand_128bit` from `next_output`, which starts at
`max_render_targets` = 4 on this core and counts up per 128-bit buffer
(`etnaviv_state.c:141,153,159,163`). So the user's outputs keep 0-3, and the
B,A halves are outputs 4, 5, 6, 7, in the order the 128-bit attachments appear,
independent of which hardware slot the plane lands in. The compiler records
each output's register (`etnaviv_compiler_nir.c:125`,
`ps_color_out_reg[location - FRAG_RESULT_DATA0]`) and the link maps hardware
target to shader output through `ps_output_remap`: `fs->ps_color_out_reg[
ps_output_remap[i]]` for `i` in 0..8, packed four to `PS_OUTPUT_REG[0]` and
four to `PS_OUTPUT_REG[1]` (`etnaviv_shader.c:205-221`). The remap entry of a
plane slot is its `rt_output` (`etnaviv_state.c:383`).

**How it meets `PS_OUTPUT_REG` and `RT_COUNT` here.** Our fragment shader
carries output rows (`ShcVivIo`, `src/shaderc/mod.rs:1011`: `reg`, `slot`)
with `slot` = 4 + output index; `shader.rs:327-337` finds the row `want = 4 +
shift + slot` per hardware target and writes its register into byte `rt` of
`GC_PS_OUTPUT_REG`; a target with no row falls back to RT0's register
(`:315-319`). `GC_PS_OUTPUT_REG` is one word of four bytes today, which is
the four-target limit of `hw_targets`' `[u8; 4]`. The plane path needs:

- a second word (Mesa's `PS_OUTPUT_REG[1]`, targets 4-7; its register offset
  in `driver/src/vivante/hw/state_3d.xml.h` is to be read, not assumed) and
  the bank-8 state for RT4-RT7;
- the companion rows at `slot = 4 + 4 + k` (output 4 + k), one per 128-bit
  attachment in attachment order. The user's `want` for a 128-bit
  attachment's R,G plane is unchanged; the B,A plane's `want` is `8 + k`, and
  it does **not** move with `out_shift`: the companion index is a property
  of the 128-bit attachment's rank, not of its draw-buffer slot (Mesa's
  `next_output` ignores nulled buffers the same way);
- `RT_COUNT = hardware targets - 1`, planes counted
  (`etnaviv_state.c:389-391`; ours `shader.rs:344` `ps_control_rt_count(targets
  - 1)`), so an RGBA32UI attachment alone is `RT_COUNT(1)`;
- `PS_CONTROL_EXT` output mode per hardware target, `UIF32` for both planes
  (`shader.rs:352`'s loop, once the format of a plane slot is the 64-bit twin).

In our compiler the split is a rewrite of the module's colour store, the
fragment analogue of `wide128.rs`'s look-up join (a constructor's lane-masked
stores into a temporary, `assemble`): the store of output `n` becomes a store
of `.xy` to output `n` and of `.zw` (as `.xy`) to output `4 + k`, zero in the
unused lanes. The register allocator and `ps_color_out_reg` / `out[]` already
cope with several outputs (`mrt_targets` draws up to four today), so the new
rows come out of the existing output collection; whether eight outputs fit
its `SHC_VIV_MAX_IO`-sized table and the allocator's colour-output limits is
to be read in the first item below, not assumed.

## (2) The key

A second key beside `wide128::Planes` (`wide128.rs:46-60`: `mask` of units
and `companion[32]`). The render-side key is small: Mesa's three fields,
`has_128bit_rt`, `rt_is_128bit` and `rt_companion[]`
(`etnaviv_shader.h:65,77-78`, filled `etnaviv_context.c:399-402`), compared on
the slow path whenever either is set (`etnaviv_shader.h:85-91`). Ours:

```text
RtPlanes { mask: u8,            // bit n: colour output n is a 128-bit target
           companion: [u8; 4] } // output index of n's B,A half (4 + k)
```

Because the companion index is the attachment's rank among 128-bit ones, the
mask alone fixes it; `companion[]` is then derived and need not be hashed
separately (Mesa stores it because the context fills it once).
What the key must **not** contain: the hardware slot of each plane. That is
the framebuffer's, read by `hw_targets` and the bank-8 programming at draw
time, and is outside the shader.

**How the GL core sets it.** At draw time, from the bound framebuffer, next
to the sampler key: `wide128_select` (`src/gl/draw.rs:2102`, called at
`:4495`) computes the sampler `Planes` from the draw's textures, then asks
`w128_variant_for` for the slot keyed `(planes, lod)` (`:2177-2189`). The
render key is a function of `d.targets[..num_targets]` formats (a
`GLES_SURF_*` that is RGBA32I/UI, `d.targets[i].format`) and so is known where
`refuse_undrawable_targets` (`:2439`) already walks them. Recommendation: **one
cache, a larger key**, not a second cache:

- `ProgW128Variant` (`:222`) holds `planes: [Wide128Planes; 2]` and `lod`
  and compiles each stage that has a non-empty key. A render key only touches
  the fragment stage, so it is one more field `rt: RtPlanes`; the match at
  `:2187` adds `&& v.rt == rt`, and "never both empty" becomes "never all
  empty". A program drawn into a 128-bit target and sampling a 128-bit
  texture needs one variant with both splits, exactly as the sampler pass
  already combines per-unit splits; two caches would compile it twice and
  could not express the pair.
- `PROG_MAX_W128_VARIANTS` is 2 (`:210`); a program drawn into an RGBA32UI
  target and also to an RGBA8 one alternates two keys. Keep 2, count the
  evictions in the host test, raise to 3 only if a corpus program alternates
  three.
- The compile entry is the free function pair `wide128_planes` /
  `wide128_variant` (`src/shaderc/mod.rs:1610-1646`; `online/backend.rs:2176`
  chains the key into `record_key` as `"wide128-planes"` plus the key bytes,
  `:2202-2213`). The render key chains the same way (`"wide128-rt"` plus
  `mask`) so the compile cache and the on-disk record stay distinct per key,
  and the empty key leaves every existing shader byte-identical (the
  contract `wide128.rs:30-34` states for the sampler key).
- The two refusals the sampler path has carry over: a blend variant of the
  fragment shader (`:2133-2137`, `online/backend.rs:2195-2198`) cannot also be
  keyed. That is the front-facing twin, which a program with `gl_FrontFacing`
  needs; an integer-target draw with `gl_FrontFacing` is refused by name until
  the blend variant takes a key. A plain program is the common case.
- The key is empty for a program that writes no output of an integer 128-bit
  target: its output `n` is simply never stored, and the other planes keep
  Mesa's behaviour (an unwritten output is undefined, `shader.rs:313-314`).

`wide128_sensitivity` (`shaderc::wide128_sensitivity`, asked once per link)
has a render analogue that is trivial: which colour outputs the fragment
shader declares (`out[]` slots). A draw whose fragment shader has fewer
outputs than the attachment index needs no variant for that attachment.

The sampler key and this key must not disagree on the planes' memory: the
render key only changes which register is stored where; the texture planes
are two stacked halves of one level (`gc-128bit-texture-sampling.md`, the
render-targets note section 2 "Resource layout"), and feedback (sampling the
attachment being drawn) is already undefined behaviour in GL.

## (3) The hardware-target count

A GL framebuffer holds up to `GLES_MAX_TARGETS` = 4 attachments
(`src/backend.rs:746`), and a 128-bit one takes two hardware targets, so a
draw has up to 8 (four attachments, all 128-bit; Mesa asserts the same,
`etnaviv_state.c:150`, and reports 4 attachments, `etnaviv_screen.c:299-304`).
Today's code stops at 4 in three places, each separate:

- `mrt_targets` clamps `num_targets` to 4 (`src/vivante/backend.rs:505`) and
  `hw_targets` returns `([u8; 4], usize)` (`:521`). The planes need
  `[u8; 8]` slots plus a per-slot "plane" tag (0 = R,G or an ordinary
  target, 1 = B,A half) and the plane offset
  (`etna_resource_level_second_plane_offset`, `etnaviv_resource.h:91-94`;
  ours `tiled_copy.rs` `second_plane_offset`). The hardware targets of a draw
  are then the ordinary targets plus one per 128-bit attachment, in Mesa's
  order: user slots first, planes appended (`etnaviv_state.c:140,156-160`).
- `GlesDraw::num_targets`, `DRAW_TARGETS` and the GL core's arrays count
  **attachments**, not hardware targets, and need not change for 128-bit
  support: `DRAW_TARGETS` stays 3 until a board draws four (plan #335 item 5
  keeps it; this note does not move it). A 128-bit attachment's two hardware
  targets are a back-end fact. Consequence: with `DRAW_TARGETS` at 3, the
  most the GL core hands over is three attachments, i.e. up to six hardware
  targets, and four attachments (eight) wait for the same board check as the
  fourth plain target.
- `GC_PS_OUTPUT_REG` and the bank writes in `shader.rs:310-360` index
  `slots[..targets]` with `rt` as the byte index; beyond 4, byte `rt - 4` of
  the second word.

The PE's own limits (8 targets in the 8-RT bank, `pe_rt_addr_8_pipe`,
`pe_rt_config_8`) are in the render-targets note section 4 item 1; this note
adds nothing to them. What is **not** known without a board: that the bank
accepts RT4-RT7 on the GC7000UL as it does RT1-RT3 (Mesa's `num_rts` is 8 on
HALTI2+, `etnaviv_screen.c:774-777`, a statement about the database and not a
measurement of this core).

## (4) Colour mask, blending, `glClearBuffer` per plane

**Colour mask.** Plane 0 takes the user's R,G bits; plane 1 takes
`(mask >> 2) & 3`, i.e. B,A in the plane's R,G lanes
(`etnaviv_blend.c:153-163`, via `companion_src`; full-overwrite recomputed from
the shifted mask). Ours: `GlesDraw::color_mask` (`src/backend.rs:1172`) is
one 4-bit mask per draw today; the plane target's mask is derived at
programming time from the source attachment's mask, so the `GlesDraw` stays
unchanged. Note the pass above also masks by the store's write mask
(`wm & 3`, `wm >> 2`): a store that writes only `.z` stores nothing to the
R,G output and `.x` of the companion. Both layers are needed: the colour
mask is a per-draw PE state, the write mask is a per-shader fact.

**Blending.** Integer render targets are not blended: ES 3.0 4.1.3 says blend
and logic op apply to fixed-point and floating-point buffers only, and
`GL_RGBA32I/UI` are neither. The GL core and back end are expected to pass the draw
without blend state for an integer attachment (not re-checked here), and the plane targets inherit
that: no blend setup for either plane. Mesa gives the companion no blend
state of its own at all (`etnaviv_blend.c:153-163`). RGBA32F, which is blendable
under `EXT_color_buffer_float`, is **not** covered by this note (ES 3.0 does
not require it) and would need blending on two independent planes, which for
a per-channel equation is correct plane by plane, except that
`GL_SRC_ALPHA`/`DST_ALPHA` factors and constant colour alpha cross planes
(alpha lives on plane 1, the factor applies to plane 0's R,G): that is the
reason to keep RGBA32F out of the first items.

**`glClear` and `glClearBuffer`.** Mesa clears the planes separately: two RS
clears with plane 0 `(g << 32) | r` and plane 1 `(a << 32) | b`, a cache flush
and a PE-to-RA stall between (`etnaviv_rs.c:399-419`), or two BLT clears
(`etnaviv_blt.c:452-465`). Ours: `clear_takes` (`src/vivante/backend.rs:495`)
and `GlesDraw::clear_target_mask` choose which targets a clear covers
(bit `n` = target `n`, GL core has applied `out_shift`); `clear_color_mask`
(`src/backend.rs:1199`) is the clear's own channel mask. A clear is a draw
of the clear quad with the colour in the target's output; with a 128-bit
target the clear shader has no variant, so the clear **must** take the plane
route as a clear of two targets: the same value split `(r,g)` / `(b,a)` into
two registers is a clear shader writing both outputs. Two ways:

1. The clear quad draws with the `RtPlanes` key as if it were a user draw: the
   clear shader is a program like any other and the variant cache above
   serves it. `clear_takes` extends to "the planes of an attachment follow
   its target bit" (bit `n` selects the pair), and `clear_color_mask`
   splits as the colour mask does. One mechanism for clear and draw.
2. A CPU/shadow clear of the tiled shadow (`wide_target.rs`), as the shadow
   already holds the pixels; no second variant of the clear shader. That is
   what `docs/notes/gc-128bit-render-targets.md` item 2 plans for the shadow's
   16-byte split and join, and it covers `glClearBuffer*` for free.

Recommendation: 2 for the first working draw (smaller, host-testable), 1
when the clear must be a GPU operation (a clear fused into the frame). The
choice does not touch the shader pass.

## (5) The host tests each step needs

All host tests; none needs a board. The existing rig is
`src/vivante/backend_tests.rs` (`rig.submit(&d).reg(...)`, e.g. the MRT
PS_OUTPUT_REG tests at `:3689-3808`) and `tests/es3_int_rt_gc.rs`.

1. **The pass** (`src/shaderc/online/vivante/`): a module storing
   `out vec4`/`uvec4`/`ivec4` to output 0 and 1, key `{mask 0b01}`: output 0
   gets `(r,g)`, output 4 gets `(b,a)`, output 1 is untouched; write masks
   `.x`, `.zw`, `.yz` split as `wm & 3` / `wm >> 2`; a scalar store and a
   `vec3` store zero-extend; integer type survives (no int-to-float);
   an empty key returns the module bit-identical (the golden test the
   sampler pass has); a key naming an output the shader does not write is a
   no-op. The unit tests live in the pass's file, as `wide128.rs` has them.
2. **The output rows and PS_OUTPUT_REG** (`shader.rs`, `backend_tests.rs`):
   with a hand-built fragment shader of outputs 0 and 4, one 128-bit
   attachment: `GC_PS_OUTPUT_REG` bytes 0 and 1 name the two registers,
   `RT_COUNT(1)`, `PS_CONTROL_EXT` both planes `UIF32`; with two 128-bit and
   one plain attachment, five targets: the second word of `PS_OUTPUT_REG`
   exists and byte 0 of it is the fifth register; a null middle draw buffer
   and `out_shift` do not move the companion output.
3. **The key** (`src/gl/draw.rs`, `wide128_draw_state` analogue
   `rt_planes_draw_state`): drawing into an RGBA32UI attachment gives
   `RtPlanes { mask: 1 }` for the fragment stage and an empty vertex key; a
   second draw to RGBA8 gives the empty key and the program's own shader
   (the variant slot is reused, not the plain shader replaced); sampling a
   128-bit texture and drawing into a 128-bit attachment gives one variant
   with both keys; the third alternation evicts the oldest; a blend-variant
   program is refused by name.
4. **The count** (`mrt_targets`, `hw_targets`): `[u8; 8]` with plane tags: one
   128-bit, one plain, one 128-bit in slots 0,1,2 gives hardware targets
   `[0, 1, 2, 0p, 2p]`, `RT_COUNT(4)`, plane offsets on the right two; four
   attachments all 128-bit are eight targets and are refused (or run) as
   `DRAW_TARGETS` says; `GlesDraw` literals are grepped in `tests/` and `src/`
   if a field is added (`grep -rl 'GlesDraw {' tests src`).
5. **Mask and clear**: `(mask >> 2) & 3` on the plane for each of the 16 user
   masks; a `glClearBuffer` of attachment 1 in a three-attachment framebuffer
   clears planes `1` and its companion and nothing else (`clear_target_mask`);
   the tiled-shadow 16-byte split and join round trip
   (`wide_target.rs`, plan #335 gc-rt128-planes).
6. **End to end on the host back end** (`tests/es3_int_rt_gc.rs`): the
   refusal tests of RGBA32I/UI become draws whose expected words are the
   register writes above. This test is the one that flips the refusal and so
   waits for the board probe.

## (6) Who owns each file today

Per the plan #335 text of this item's assignment; ownership moves and
connected keeps overlapping files apart, so a later item reads it again.

| File | Owner today | For this design |
|---|---|---|
| `src/shaderc/online/vivante/**` | #328 | the pass beside `wide128.rs`, its key |
| `src/shaderc/online/backend.rs`, `src/shaderc/mod.rs` | free of the plans named in the assignment; check at the time | the variant entry and the record-key chain |
| `src/vivante/backend.rs` | #328 and #333 | `mrt_targets`, `hw_targets`, `clear_takes`, plane programming |
| `src/vivante/shader.rs` | #328 | `PS_OUTPUT_REG` second word, `RT_COUNT`, output mode |
| `src/vivante/wide_target.rs` | gc-rt128-planes (#335) | the shadow's split and join |
| `src/gl/draw.rs` | no plan named; `wide128_select` was the 313 file | the render key, the cache field, `refuse_undrawable_targets` |
| `src/backend.rs` | #328, gc-flip-* items | `rt_format_drawable`, `DRAW_TARGETS` (stays 3) |
| `tests/es3_int_rt_gc.rs` | not listed; the other `tests/**` are gc-flip-survey's | the flip test |
| `docs/es3/int-rt-gc.md` | gc-docs (#335) | the line "no plane path" |
| `docs/notes/gc-128bit-render-targets.md` | gc-rt128-planes | not touched here |

## Sequence of reviewable items

Each is one commit with its host test and `tools/check.sh --changed
origin/main` green; none changes behaviour until the last, because the empty
key and the unchanged `rt_format_drawable` keep every existing draw
byte-identical (the gate compares generated shader bytes).

1. **The pass, inert** (`src/shaderc/online/vivante/`, a new `wide128_rt.rs`
   beside `wide128.rs`): the colour-output split under `RtPlanes`, set per
   compile like `set_compile_planes`, test 1. Also reads the output-table
   and allocator limits at 8 outputs; if they bind, that is its own item.
2. **The compiler entry and record key** (`src/shaderc/online/backend.rs`,
   `src/shaderc/mod.rs`): `rt_planes_variant`, the key chained into the record
   key, an empty key identical to today's; test: the same source, two keys,
   two distinct records, the empty key the original bytes.
3. **The GL core's key** (`src/gl/draw.rs`): the render key from
   `d.targets`, the `rt` field in `ProgW128Variant`, the eviction and the
   refusals; test 3. Inert while `rt_format_drawable` refuses the format:
   the host test sets the key through a test hook, as
   `wide128_draw_state` does for the sampler key.
4. **The back end's target list** (`src/vivante/backend.rs`, `shader.rs`):
   `hw_targets` with eight slots and plane tags, the second `PS_OUTPUT_REG`
   word, `RT_COUNT`, output mode, the plane colour mask; tests 2, 4, 5.
   Behind the same refusal; the host test hands the back end a `GlesDraw`
   with the format and reads the words.
5. **Clear** (item 4 option 2 first): the shadow's join and split from
   `gc-rt128-planes`, the clear through the shadow; test 5.
6. **The board probe** (plan #316, not an agent task): one RGBA32UI
   attachment, constant colour above 2^24 and negative, read back untiled
   (the render-targets note, "Suggested first step"). Its outcome decides item 7.
7. **The flip**: `rt_format_drawable` and `refuse_undrawable_targets` let
   RGBA32I/UI through, `int-rt-gc.md` and the RESULTS row change, test 6.
   Only after 6 is recorded on the issue.

Items 1-5 can be written without the board and reviewed on the host; items
1 and 2 touch only the compiler, item 3 only the GL core, item 4 only the GC
back end, so they can land in any order that their file owners allow. If the
probe fails the refusal stays and items 1-5 remain inert code with their
tests, the state `67b27e26` leaves the texture planes in.
