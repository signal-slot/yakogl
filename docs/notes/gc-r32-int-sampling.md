<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000UL: sampling R32I/UI and RG32I/UI (design, item gc-r32-sample-note)

Plan #366. `docs/es3-cts-failures.md` RC2's "Left" row still lists, for the
GC7000UL, "R32I/UI and RG32I/UI (hardware formats, no GL-core class)". On the
tree this note was written against (`d9d65cff`) that clause is **out of
date**: the classes, the descriptor rows and the mask bits exist. The note
records what is in the tree, the Mesa side it follows, what is really left
(little, and none of it a host code gap), and how the RGBA32 flip touches it.

## 1. Mesa

Mesa 26.2.1 (tag `mesa-26.2.1`, `da14d65e4499e66468094be52bff9ea0915a695e`;
the etnaviv driver is not vendored, `docs/es3/gc-formats.md` section 1 names
it). Paths are relative to `src/`; `F` = `gallium/drivers/etnaviv/etnaviv_format.c`,
`D` = `gallium/drivers/etnaviv/etnaviv_texture_desc.c`, `S` =
`gallium/drivers/etnaviv/etnaviv_screen.c`.

Read and verified in this item from a sparse clone of the tag
(`$CONNECTED_DEPS_DIR`, `mesa-etnaviv-26.2.1`): `translate_texture_format`
(`F:275-294`) takes the table's `.tex`, then on `halti >= 5` replaces it
(`F:286-291`). The table's own rows are the *float* extended formats
(`VT(R32_SINT, FLOAT, EXT_R32F | EXT_FORMAT, R32F)`, `F:136-137`;
`R32G32_*`: `EXT_G32R32F`, `F:209-210`), so without the override an integer
texture would be read as float.

| class | Mesa table row | HALTI5 override | hardware format (value) | sign |
|---|---|---|---|---|
| R32UI / R32I | `PIPE_FORMAT_R32_UINT` / `_SINT` (`EXT_R32F`), `F:136-137` | `translate_texture_format` replaces the format by `TEXTURE_FORMAT_EXT_R32I`, `F:286-291` | `EXT_R32I` = 0x23 | one word for both signs: CONFIG2 `0x00030000` plus a sign bit only for 8- and 16-bit channels, `D:187-189` |
| RG32UI / RG32I | `PIPE_FORMAT_R32G32_UINT` / `_SINT`, `F:209-210` (`EXT_G32R32F`) | `F:289-290` | `EXT_G32R32I` = 0x24 | as above |

- Support gate: `gpu_supports_texture_format`, the 32-bit integer formats need
  HALTI5 (`S:423-429`); the i.MX8MP GC7000UL has HALTI0-5.
- Filtering: no `INT_FILTER` (`F:296-338`: none of the classes is unorm); GL
  only allows NEAREST on integer textures anyway (`Es3Format::filterable`,
  `src/gl/es3_formats.rs:214`).
- Swizzle: Mesa composes the format's own swizzle with the sampler view's
  (`swiz` is ORed into CONFIG1 at `D:186`). In the tree
  the format's own word is the identity for these classes
  (`tests/es3_r32_gc.rs:96-120`) and the GL texture swizzle is composed on top
  (`840ea7ec`, `docs/notes/gc-texture-swizzle-wrap.md`). Whether the TE returns
  G/B = 0 and A = 1 for the absent channels of an integer texel, or whatever
  the plane holds, is not shown by anything on the host; see section 4, item 6.

## 2. What is in the tree

- Storage classes: `GLES_TEXFMT_R32UI`, `_R32I`, `_RG32UI`, `_RG32I`
  (`src/backend.rs:147-150`, class masks stay below 64 at `:166`).
- GL-core mapping: each format lists its own class first, the RGBA32 class as
  the fallback (`src/gl/es3_formats.rs:89-92`, `:153-160`), so `storage_in(mask)`
  picks the own class when the mask holds it, the RGBA32 class otherwise.
  Upload packs the texels at `src/gl/es3_formats.rs:694-699` (4 or 8 bytes).
- Mask: all four are in `GC_SAMPLED_TEXFMTS` (`src/backend.rs:236-257`,
  `b5e04b14`; descriptor rows first, `c9bcc5fe`, `e40e9418`).
- Descriptor: `gc_texfmt` rows `src/vivante/texture.rs:135-136`
  (`EXT_R32I` / `EXT_G32R32I`, `decoded`, no `no_alpha`, no `int_filter`);
  `format_words` writes the word into CONFIG1's FORMAT_EXT field. A texel is
  4 or 8 bytes: an R32 texel is sampled from the linear storage as the GL core
  holds it, an RG32 texel is an 8-byte "wide" texel taken from the page-aligned
  tiled copy as RGBA16F's is (`src/vivante/tiled_copy.rs` `wide_texels`;
  `tests/es3_r32_gc.rs:138-145`).
- Integer samplers: `isampler*` / `usampler*` select on the GC through
  `src/shaderc/online/vivante/int_tex.rs` (`docs/es3/gc-formats.md` section 3).
- Render side (not this note's subject): the four classes are also colour
  attachments (`src/gl/framebuffer.rs:460-463`, `:671-678`;
  `src/vivante/backend.rs:346-347`; `tests/es3_int32_rt_gc.rs`, `docs/es3/int-rt-gc.md`).
- Tests: `tests/es3_r32_gc.rs` (descriptor words, swizzle composition, texel
  sizes, upload and layout through the GL API on the stand-in OSAL, the GL
  core's storage of the eight 32-bit integer formats), `tests/es3_formats_gc.rs`
  (the rows of `gc-formats.md` section 2 against the descriptor and the rnndb
  header), `tests/es3_tex_swizzle_gc.rs`.
- Docs that already say so: `docs/es3/gc-formats.md` section 2 rows and
  section 4, `docs/es3/sized-formats.md` lines about `b5e04b14`.

So of the three things the task lists as "missing" (a storage class, the
sampled-mask bits, the descriptor and integer-sampler path), none is missing.

## 3. What is left

Nothing for the host except two documentation corrections and optional hardening:

1. **RC2's GC7000UL clause is stale.** `docs/es3-cts-failures.md:151` ("R32I/UI
   and RG32I/UI (hardware formats, no GL-core class)") and the "refuses or
   keeps in a fallback class" wording at `:75` should say the four classes are
   host-done (`b5e04b14`, `tests/es3_r32_gc.rs`) with the i.MX8MP run pending.
   That file belongs to plans #356, #361 and gc-w128-deqp-list; the planner
   hands the edit to gc-w128-rc2 (item list below).
2. **`docs/es3/gc-formats.md:187-199`** is current but says "Left: 4. RGBA32F/I/UI
   ... R32I/UI and RG32I/UI are their own classes"; fine as is.
3. **Board run**: no GPU result exists for the four classes (descriptor words,
   the ES 3.0 `texture.format` / `texture.specification` cases of R32/RG32
   I/UI, the absent-channel values). This is the same i.MX8MP session as the
   RGBA32 flip's (`gc-w128-gltest`, issue #24).
4. **Host-only unknowns**: absent G/B/A of an integer texel (section 1), and
   the CTS counts of `docs/es3/gc-formats.md` section 5, which predate
   `b5e04b14` ("it was not repeated here"). A host dEQP re-run
   (`tools/cts/host-deqp-run.sh --gpu=vivante --module=gles3`, filters
   `functional.texture.format.` and `.specification.`) would give the new
   upload/compile split; `tools/cts/**` belongs to gc-w128-deqp-list.

## 4. Interaction with the RGBA32 flip

- Order dependence is none: the R32/RG32 classes were opened first. The flip
  only adds `RGBA32UI/I/F` to the same mask.
- Storage fallback lists (`src/gl/es3_formats.rs:89-92`): `R32U_ST` etc. end in
  the RGBA32 class. After the flip, R32/RG32 still take their own class (listed
  first and in the mask); nothing re-routes them. `RGB32UI/I` use `U32_ST` /
  `I32_ST` (`:165-166`), which are RGBA32-only, so they move from "refused"
  to the two-plane class with the flip (that is gc-w128-rgb32-alpha's no_alpha
  question, not this note's).
- Guard `is_wide128` (`src/vivante/texture.rs`) is false for the four classes
  (`tests/es3_r32_gc.rs:104`); the two-plane key, companion slot and swizzle
  refusal (`docs/notes/gc-128bit-swizzle.md`) therefore never apply to them.
  The R32/RG32 classes accept every GL swizzle through the ordinary descriptor
  composition; a pin that says so for RG32 (G in R, 0/1 lanes) is cheap
  insurance against the flip's guard being written too broadly.
- The two-plane `EXT_G32R32I` descriptor and the RG32I class share the format
  word and the sampled texel width; a copy-size test
  (`tests/es3_r32_gc.rs:138`) already tells a plane from a RG32 texture by
  `copy_bpp` (8 against 16), the flip's tests should not reuse that assertion
  for RGBA32.
- `GLES_TEXFMT_RGBA32F` flips the RGBA16F fallback of RGBA32F/RGB32F/RG32F
  (`docs/es3/sized-formats.md`); RG32F and R32F have no integer analogue here
  (R32F is its own class, RG32F still has none and will store as RGBA32F).

## 5. Ordered item list

Ordered as they can be done; none needs code in this plan.

1. **gc-r32-rc2-text** (docs): update RC2's GC7000UL clause and the `:75` row
   of `docs/es3-cts-failures.md` to "R32I/UI and RG32I/UI: host-done
   (`b5e04b14`), board run pending". Files: `docs/es3-cts-failures.md` only,
   after #356 docs-rc and #361 docs-rc1 merge; fold into gc-w128-rc2. No test.
2. **gc-r32-pins-flip** (host test, optional): in the new
   `tests/es3_wide128_gc.rs` (gc-w128-tests) assert that after the flip the
   four classes' `storage_in(GC_SAMPLED_TEXFMTS)` is still their own class
   (not RGBA32), that `is_wide128` is false for them and that a GL swizzle that
   the RGBA32 guard refuses (a channel moved across the planes, ZERO/ONE on a
   lane) is accepted on an RG32I binding. Files: `tests/es3_wide128_gc.rs`
   (or one more test in `tests/es3_r32_gc.rs`, which no other item owns).
3. **gc-r32-host-dEQP** (measurement, no code): repeat `gc-formats.md`
   section 5's run with the four classes in the mask and record the counts in
   that section (`docs/es3/gc-formats.md`); `tools/cts/**` is another item's.
   Test: the run itself (HOST-ONLY; the read-back is black).
4. **gc-r32-gltest** (board, outside the host plan): a `gl_test` case that
   uploads RG32I texels, samples with `isampler2D`, writes the values to a
   RGBA8UI target and reads them back; with it the swizzle of the absent
   channels is settled. Files: `gl_test` (as gc-w128-gltest does for RGBA32),
   `docs/es3/gc-formats.md` rows (the `no` column of "sampled on this core").
5. **gc-r32-narrow** (separate, listed in `gc-formats.md` section 4 item 5):
   R8/RG8/R16 and snorm in their own narrow classes; memory only, not needed
   for R32/RG32.
6. **gc-r32-swizzle-observe** (follows 4): if the board returns something other
   than G=B=0, A=1 for R32I/RG32I, give the four classes a `format_words`
   swizzle that fills them (`src/vivante/texture.rs` rows `:135-136`, the
   `no_alpha` / swizzle composition) with a pin in `tests/es3_r32_gc.rs`
   (`the_gl_swizzle_composes_with_the_identity`).

## 6. Host tests each item needs

| item | test | feature set (header of the test) |
|---|---|---|
| 1 | none (docs) | - |
| 2 | `tests/es3_wide128_gc.rs`; existing `tests/es3_r32_gc.rs` must stay green | `--no-default-features --features host-test,gpu-vivante,shaderc-online` |
| 3 | `tools/cts/host-deqp-run.sh --gpu=vivante` | - |
| 4 | board run, a `gl_test` case | - |
| 6 | `tests/es3_r32_gc.rs`, `tests/es3_formats_gc.rs` | as item 2 |

## 7. Assumptions

- The task's premise (no class, no mask bit) was found false; the note
  documents the tree rather than a design for something not yet built.
