<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: wiring the depth sample, the code for each answer of the X2 depth-TPU session

A design note for plan #328, item depth-sample-gc (issue #1204). glmark2
`shadow` and `refract` lose an estimated 350 to 1000 ms a frame to the CPU
D24S8 to R32F depth resolve (`docs/notes/gc-slow-shadow-refract.md`, cause 1;
`docs/notes/gc-depth-resolve.md`). `docs/notes/gc-depth-sample.md` explains how
the texture unit could sample the D24S8 surface itself, and its board case
`dtpu` (item gt-depth-tpu, run by item x2-depth-tpu-run on the Armadillo-X2)
decides whether it can. This note writes out the code for each answer, so
depth-sample-gc can start as soon as the record arrives: the files, the
functions, the words, and the host test that proves the path without a board.

Nothing here was built or run, and no board was used. Every `path:line` is at
`723a75c8` (the integration tip this note was written on) unless a line says
otherwise. `gc-depth-sample.md`'s own line numbers are at `98653bb5` and are
not repeated here. Mesa citations use that note's letters (`F`, `D`, `X`, `N`,
...: Mesa 26.2.1 etnaviv, `$CONNECTED_DEPS_DIR/mesa-pan/mesa-26.2.1`).

## The three outcomes, read from case `dtpu`

`gc-depth-sample.md` section (a), "The board experiment", defines checks 3 to 7
and a verdict table. This note's three outcomes are those rows grouped by
the code they lead to:

| outcome | `dtpu` record | meaning | code (section) |
|---|---|---|---|
| **(a)** | check 4 passes (or 4 fails and 5 passes), check 7 passes | the TPU samples the row-linear D24S8 surface as `TEXTURE_FORMAT_D24X8` (0x11), LINEAR addressing, depth in X | (a): a depth view of the surface, no copy, no CPU resolve on the draw path |
| (a) with a stale cache | check 4 (or 5) passes, check 7 fails | as (a), but the texture cache serves old texels when nothing waits between the depth pass and the sample | (a) plus the flush of "(a).9" |
| **(b)** | checks 4 and 5 fail, check 6 passes | D24X8 does not read the linear surface, but the TPU addresses its bytes (the raw A8R8G8B8 view, P2, gives the PE's words) | (b): a copy on the GPU, in the stream, into a layout or format the TPU reads |
| **(c)** | check 6 fails (and so 4 and 5) | the LINEAR descriptor does not address the surface at all | (c): the CPU resolve stays; its three CPU fixes |

Check 3 is the reference (today's path). If check 3 fails, the record is
not a verdict: the case or the board is wrong. Rerun the session; this note
does not apply until check 3 passes.

## What holds for every outcome

### Today's path, call by call

1. glmark2 `shadow` (`src/scene-shadow.cpp:103-108`, 113, 146-150, glmark2
   `22c527cb`, the deps cache's `glmark2-src`) and `refract`
   (`src/scene-refract.cpp:195-200`, 214, 249-254) make a
   `GL_DEPTH_COMPONENT`/`GL_UNSIGNED_INT` 2D texture with one level, NEAREST
   min and mag, CLAMP_TO_EDGE in S and T. They attach it as the FBO's depth,
   draw depth into it, then bind the canvas and sample it. The shaders read
   `texture2D(ShadowMap, uv).x` (`data/shaders/shadow.frag`) and
   `texture2D(DistanceMap, uv)` (`data/shaders/light-refract.frag:25`) and
   compare in GLSL. **No sampler compare** (`GL_TEXTURE_COMPARE_MODE` stays
   `GL_NONE`, `src/gl/texture.rs:602`), so `compare_op` is `16 | 8`
   (`src/gl/draw.rs:678-687`).
2. The sampling draw: `build_common` (`src/gl/draw.rs:3359`) calls
   `resolve_sampled_depth` (`:3322`), which calls
   `texture::depth_for_draw` (`src/gl/texture.rs:837`) for each 2D sampler.
3. `depth_for_draw` submits the depth kick (`texture.rs:850`, `flush`, no
   wait) and asks `backend::samples_rendered_depth(id, w, h)`
   (`texture.rs:851`, `src/backend.rs:1858`). It asks only when the texture
   passes `depth_texture_rendered` (`texture.rs:745-750`: used, DEPTH
   format, R32F storage), is `GL_TEXTURE_2D`, was rendered at level 0, and has
   one level (`texture.rs:847`). glmark2's textures pass all four.
4. `samples_rendered_depth` has a V3D and a PowerVR arm only. Every other
   build answers false (`backend.rs:1867-1871`; the doc names the GC7000 at
   `:1848-1850`). So `depth_for_draw` calls `resolve_depth`
   (`texture.rs:698`): `flush` and `finish` (`:709`, every slot,
   `src/vivante/backend.rs:3197-3209`), `backend::depth_resolve` (`:729`) ->
   `gc_depth_resolve` (`vivante/backend.rs:2995`) -> `ds_cache::resolve_depth`
   (`src/vivante/ds_cache.rs:562`): `cache_invd` of the whole surface
   (`:598`), the byte-load loop (`:604-613`), then `cache_flush` of the texels
   (`texture.rs:732`).
5. The draw's binding: `tb.depth_id = depth_ids.of(ti)`
   (`src/gl/draw.rs:4018`), 0 today. `depth_luminance` (`:4026`,
   `texture_depth_luminance` `:664`) is 1 only on the ES 2.0 surface
   (`DEPTH_TEXTURE_EXTENSIONS`, `src/gl/state.rs:206`). `no_alpha` is 1 for
   `GL_DEPTH_COMPONENT` (`draw.rs:644-657`). The binding reaches
   `emit_textures` (`src/vivante/backend.rs:2163`) -> `emit_slot`
   (`src/vivante/texture.rs:480`) -> `build_descriptor` (`:306`), which
   describes the R32F texels: `gc_texfmt` -> `EXT_R32F` (`:143`), LINEAR
   (`:273`), swizzle (R, R or G, R or B, 1) (`:291-296`), `LINEAR_STRIDE` =
   `stride_texels * 4` (`:383`), LOD 0 = `base_gpu` (`:431-434`).

### What the GL core already does, and needs no change for

`depth_for_draw` and the `depth_id` contract (`GlesTextureBind::depth_id`,
`src/backend.rs:917-925`) exist for the V3D and the PowerVR. With a GC arm in
`samples_rendered_depth`, the GL core leaves the texels stale and puts the
id into the binding. Every CPU reader resolves first through
`settle_texture` (`texture.rs:900-933`) or `resolve_depth`: glReadPixels of
the depth, the copies, the blits, glGenerateMipmap and sub-image writes. The
CPU resolve therefore stays in every outcome, for the CPU readers, and
`depth_resolve: Some(gc_depth_resolve)` (`vivante/backend.rs:3385`) stays in
the table. **No outcome changes `src/gl/texture.rs` or `src/gl/draw.rs` in
logic.** Outcome (a) changes two doc comments (`texture.rs:679-684`,
`backend.rs:1848-1850`).

Texture completeness: a one-level NEAREST depth texture is complete
(`incomplete_reason`, `src/gl/draw.rs:2772`). An incomplete one never
reaches the depth path: its binding is the black texel before any descriptor
(`vivante/texture.rs:505-523`). The depth path must keep that order (below).

### Files, and who owns them now

depth-sample-gc's brief names `src/backend.rs`, `src/vivante/depth_sample.rs`
and `tests/gc_depth_sample_path.rs`. Outcome (a) also needs small edits in
four more files. Each has an owner today:

| file | edit | owner at this tip |
|---|---|---|
| `src/vivante/regs.rs` | one const, `GC_TEX_FORMAT_D24X8` | none (plan #310 edits texture formats nearby) |
| `src/vivante/mod.rs` | `pub mod depth_sample;` (if gt-depth-tpu did not add it) | gc-early-z (running), then gc-groundwork-mod |
| `src/vivante/ds_cache.rs` | two small functions (the view lookup, the sample stamp) | resolve-wide (running) |
| `src/vivante/texture.rs` | one `gc_texfmt` arm, one swizzle clause, one substitution in `emit_slot` | tiled-copy-rs (running); plan #310 |

So depth-sample-gc starts after resolve-wide, tiled-copy-rs and gc-early-z
are merged, rebased on the main of that day, and keeps each edit to the
lines below. The line numbers here move with those merges. The function
names are the anchors.

`src/vivante/depth_sample.rs` and `tests/gc_depth_sample.rs` do not exist at
this tip. Item gt-depth-tpu is writing them (the test knob
`gc_debug_set_depth_tpu(mode)` of `gc-depth-sample.md`, the descriptor for
modes 1 and 2, and its host test). depth-sample-gc builds on what gt-depth-tpu
merged: the knob's mode 1 descriptor **is** outcome (a)'s product
descriptor. The product path drops the knob test, and the knob stays for
gl_test. If gt-depth-tpu's module already has a function that builds the
substituted binding (below), depth-sample-gc calls that function and does not
write a second one.

## (a) The TPU samples D24S8 as D24X8: a depth view, no copy

The design mirrors the PowerVR's (`pvr_depth_sampled`, `src/pvr/backend.rs:4812`;
`depth_buffer_sampleable` `:4784`; `depth_buffer_binding` `:4851`, which turns
the binding into one with a backend-private format, `PVR_TEXFMT_DEPTH_BUFFER`
`:2143`, consumed at `:5412`). Ten steps.

**(a).1 `src/vivante/regs.rs`, beside `GC_TEX_FORMAT_ETC1` (`:899`).**

```rust
/// Mesa X:60, F:179-180 (X8Z24_UNORM, S8_UINT_Z24_UNORM): a CONFIG0 format.
pub const GC_TEX_FORMAT_D24X8: u32 = 0x11;
```

The `mesa_header` test (`regs.rs:1093-1141`) then checks it against
`driver/src/vivante/hw/common_3d.xml.h:61` (`TEXTURE_FORMAT_D24X8
0x00000011`). Nothing else is needed for the test to cover it.

**(a).2 `src/vivante/ds_cache.rs`: the view, and the stamp of a sample.**

```rust
/// Where the depth plane of GL object `id` lies, for a `w` x `h` texture
/// sampled from it: the surface's GPU address past `guard`, its row
/// stride in bytes and its rows. `None` unless the plane is Live (not a
/// clear value, not parked, not Undefined), single-sampled (`mine`'s
/// scale 1) and covers `w` x `h` (`w * 4 <= stride`, `h <= rows`). No wait,
/// and no CPU access: a pending kick is fine, since the GPU runs kicks in order.
pub fn depth_view(id: u32, w: u32, h: u32, guard: usize) -> Option<DepthView>;

/// A draw recorded in kick `kick` samples the depth plane of `id`: its
/// surface is not freed (`gone`) or given away (`victim`) until that
/// kick completed.
pub fn note_sampled(id: u32, kick: u32);
```

- The lookup is `resolve_depth`'s (`:580-581`): the same `mine` filter and the
  same `held` search, then `e.planes[DEPTH].is_live()`
  (`src/ds_retention.rs:85`). A plane whose `cleared_value()` is `Some`
  (`:587`, `ds_retention.rs:90`) answers `None`. Its memory holds no clear
  (the clear is kept as a value), so the CPU fill must write the texels.
- `DepthView { gpu: e.mem.gpu + guard, stride: e.stride, rows: e.rows }`.
  `Surface::stride`/`rows` are 64-aligned (`:83-85`). The allocation is
  `guard + stride * rows + guard` (`alloc`, `:626-629`), so the LOD 0 address
  is `mem.gpu + GC_DEPTH_GUARD_BYTES` (`vivante/backend.rs:89-92`, 0 outside
  the `vivante-depth-guard` build). This is the address the PE writes
  (`pe_pipe_depth_addr`, `vivante/backend.rs:1730-1731`).
- `note_sampled` sets `used_by = kick` on that surface. This is required.
  `used_by` is today only the last kick that **rendered** into the surface
  (`select`, `:384`). `gone` (`:410-433`) frees a surface whose planes are
  all Undefined once `used_by` has completed (`:421`). Take glmark2's teardown:
  it deletes the texture right after the last sampling draw is recorded, and
  `attachment_gone` frees the memory a pending kick still reads. The
  PowerVR has the same rule. `tiled_copy::lookup` does the same for its
  copies (`src/vivante/tiled_copy.rs:504`).
- Both are `pub`, not `#[cfg(test)]` (the test accessors `planes_of`,
  `memory_of`, `stride_of` at `:677-694` are `cfg(test)` and are not visible
  to `tests/`).

**(a).3 `src/vivante/depth_sample.rs`: the answer, the binding, the counts.**

```rust
/// A backend-private texel class: a binding that reads a depth plane in
/// place (never a GLES_TEXFMT_*, which end at 25, src/backend.rs:141).
pub const GC_TEXFMT_DEPTH_VIEW: u8 = 0xf5;

/// crate::backend::samples_rendered_depth's GC answer.
pub fn gc_depth_sampled(id: u32, w: u32, h: u32) -> bool {
    g().inited && id != 0 && enabled() && ds_cache::depth_view(id, w, h, GC_DEPTH_GUARD_BYTES).is_some()
}

/// The binding `emit_slot` builds the descriptor from, for a binding with
/// `depth_id != 0`: the texture's size, wraps and filters, the surface's
/// memory. `None`: the plane is gone (counted and logged once).
pub fn binding(t: &GlesTextureBind, kick: u32) -> Option<GlesTextureBind>;

/// (draws that sampled a depth plane in place, bindings refused), for the
/// exit line and the host test (the PowerVR's pvr_backend_depth_sample_stats).
pub fn stats() -> (u32, u32);
```

- `enabled()`: the product answer is on, unless a `test-knobs` build set the
  knob to 0 (the pattern of `narrow_la_on`, `vivante/backend.rs:446-459`).
  Knob 1 is the product path. Knob 2 (the raw 8888 view) stays a gl_test
  arm only.
- `binding` returns `GlesTextureBind { format: GC_TEXFMT_DEPTH_VIEW,
  type_: GLES_TEXTYPE_2D, layers: 1, base_gpu: view.gpu, stride_texels:
  view.stride / 4, bytes: view.stride * view.rows, mip_filter: 0,
  mip_levels: 1, ..*t }`. The wraps, filters, `depth_luminance`, `no_alpha`,
  `compare_op`, `unit` and `swizzle` are kept, and `width`/`height` stay the
  texture's. It calls `ds_cache::note_sampled(t.depth_id, kick)`.
  `gles_texfmt_bytes(0xf5)` is 4 (`src/backend.rs:290-302`, the default
  arm), so `build_descriptor` computes `LINEAR_STRIDE = view.stride`
  (`texture.rs:383`), `SLICE = stride * h` (`:398`) and LOD 0 =
  `view.gpu` (64-aligned already, `:433`) **with no change to
  `build_descriptor`**. `tiled_copy::lookup` (`:310`) finds no copy for
  it: one level, 2D, 4-byte (`wanted_vs`, `tiled_copy.rs:498-501`).
- The first sample logs one line (`a WxH depth texture is sampled from its
  depth surface (D24X8, stride S): no CPU resolve`). A refusal logs once and
  counts. The words follow the PowerVR's (`pvr/backend.rs:4854-4872`).

**(a).4 `src/backend.rs:1858-1872`: the GC arm.**

```rust
#[cfg(feature = "gpu-vivante")]
{
    crate::vivante::depth_sample::gc_depth_sampled(id, w, h)
}
#[cfg(not(any(feature = "gpu-v3d", feature = "gpu-powervr", feature = "gpu-vivante")))]
```

Update the doc (`:1844-1857`): the GC7000 joins the backends with an
answer, and the Mali keeps the resolve. The function reads backend state
only. The GL suite's mock tables never call `gc_init`, so `inited` is false
and the answer stays no there (the doc's point at `:1853-1857`).

`defers_depth_storage` (`:1883-1896`) gets **no** GC arm in the first
commit. With an arm, glmark2's 7.68 MB R32F store would never be allocated
(`gl/texture.rs:760-768`). But one refused sample would then fall back to
texels that do not exist, and the GL core allocates them only on a CPU access
(`ensure_storage`, `:785`). Add the arm in a second commit, with its own test
(a refused binding samples black and allocates nothing), once (a) has run on
the board.

**(a).5 `src/vivante/texture.rs`, `gc_texfmt` (`:96-147`): the class.**

```rust
GC_TEXFMT_DEPTH_VIEW => {
    f.format = GC_TEX_FORMAT_D24X8;
    f.no_alpha = true;     /* A reads 1 */
    /* int_filter stays false: no INT_FILTER for a Z/S format (F:320) */
}
```

`int_filter_wanted` (`:464-466`) then answers false through
`gc_texfmt(..).int_filter`. Its doc (`:461-463`, "the GL core stores a depth
texture in RGBA8") is stale, because the texels are R32F, and should say
so.

**(a).6 `texture.rs`, `format_words` (`:287-298`): the swizzle.** The
TPU's Y, Z and W for D24X8 are not in Mesa (`gc-depth-sample.md` (a)
point 6). So the depth view names the constants instead of passing G and B
through:

| surface | R | G | B | A |
|---|---|---|---|---|
| ES 2.0, `depth_luminance` 1 (d, d, d, 1) | `GC_SWIZZLE_R` | `GC_SWIZZLE_R` | `GC_SWIZZLE_R` | `GC_SWIZZLE_1` |
| ES 3.0, `depth_luminance` 0 (d, 0, 0, 1) | `GC_SWIZZLE_R` | `GC_SWIZZLE_0` | `GC_SWIZZLE_0` | `GC_SWIZZLE_1` |

(`GC_SWIZZLE_*`, `regs.rs:993-998`.) One clause:
`let zero_gb = t.format == GC_TEXFMT_DEPTH_VIEW && !lum;`, read in the G
and B arms. The R32F class needs nothing, because its format returns 0 in G
and B itself. If the GL texture swizzle (`GlesTextureBind::swizzle`) is
composed in `format_words` by then (gc-swizzle, plan #310), the depth view
composes it the same way. The table above is its "identity" row.

**(a).7 `texture.rs`, `emit_slot` (`:480-524`): the substitution.** After
the incomplete/undecoded substitution (`:505-523`) and before
`build_descriptor` (`:524`):

```rust
let view;
let t = if t.depth_id != 0 && t.incomplete == 0 {
    match super::depth_sample::binding(t, super::backend::gc_kick_id()) {
        Some(v) => { view = v; &view }
        None => &black_of(t),   /* the same 1x1 black as above, by name */
    }
} else { t };
```

Refuse to black, not to the stale R32F texels. The GL core left them stale
because of the answer (`gl/texture.rs:813-818`). The PowerVR samples black
for the same reason (`pvr/backend.rs:4851-4861`). The answer and the
binding read the same state, and nothing runs between them but the
draw's own `ds_cache::select` (`vivante/backend.rs:2826`). So a refusal here
is a defect to count, not a path.

**(a).8 The sampler word.** `samp_ctrl0` (`texture.rs:578-585`) is
unchanged: NEAREST min and mag (glmark2's), MIP NONE (`mip_filter` 0),
CLAMP_TO_EDGE, `UNK21`, and no `INT_FILTER` (from (a).5). `TX_CTRL` stays 0
(`:597`, no tile status). A LINEAR filter on the view is allowed, since D24X8
is a real depth value (`D:324-328` restricts only the mip filter), and
`mip_filter` is 0 anyway.

**(a).9 Compare, and what glmark2 maps onto.**

- glmark2: `compare_op = 16 | 8`, so no compare bits. The shader's `.x` reads
  R, the depth, as the R32F texel did. No shader changes, and no
  `src/shaderc` change.
- An ES 3.0 shadow sampler: the compiler refuses `sampler2DShadow` today
  (`docs/notes/gc-shadow-compare.md` section 3,
  `src/shaderc/online/select.rs` `ir::Op::Tex` arm), so no draw reaches a
  depth view with a compare. When gc-shadow-sampler lands
  (`gc-shadow-compare.md` 4.1: `GC_SAMP_COMPARE_ENABLE`,
  `samp_compare_func`, table `[7,2,4,0,3,5,1,6]`), its bits apply to the
  depth view unchanged. That note's open point ("does the TE compare a
  non-depth format") is moot for the view, because D24X8 is a depth format, as in
  Mesa's route. depth-sample-gc adds no compare code. Its test pins that
  SAMP_CTRL0 has no bit 17 and no bits 18-20 set.

**(a).10 The two variants of (a).**

- *Check 5 only (fewer than 24 bits).* Same code. glmark2's 0.1505 bias
  hides 16-bit depth. The exact readers are CPU readers (glReadPixels of a
  depth texture, the copies), and they still resolve on the CPU. Open an issue
  for dEQP's `texture.shadow` exactness. Do not gate the path on it.
- *Check 7 fails (stale texture cache).* Every draw already starts with
  `FLUSH COLOR|DEPTH|TEXTURE|SHADER_L1`, `TEXTUREVS`, a RA-PE stall and a TS
  flush (`emit_flushes`, `vivante/backend.rs:1790-1795`). The descriptor
  cache is flushed at each frame's start (`emit_context_init`, `:2213-2214`),
  and each slot is invalidated after its address write (`texture.rs:636-657`).
  If that still serves old depth, the fix is in `depth_sample`, with its own
  board check: when a draw has a depth view, write `FLUSH_DEPTH` and a FE-PE
  stall before the draw's state. If no flush cures check 7, take (b)'s copy
  (b.2), which writes the texels the TPU already reads coherently today.

**(a).11 What happens to `ds_cache.rs`.** The CPU resolve
(`resolve_depth`, `:562-615`, with resolve-wide's loads) stays for the CPU
readers and for every binding the answer refuses: a whole clear (`fill`),
a parked clear, scale 2, or a texture with levels. On glmark2's frame it is
no longer called. Its `cache_invd` (`:598`) and the GL core's `cache_flush`
(`gl/texture.rs:732`) go with it. The two new functions of (a).2 are the
only edit. `victim` (`:642-658`) needs nothing: a Live plane is never given
away (`may_go`, `:647`), and a sampled plane is Live.

### (a) Host test: `tests/gc_depth_sample_path.rs`

The real GL core, the real Vivante back end and the in-process compiler over
a stand-in OSAL. The harness is `tests/gc_npot_repeat.rs`'s: `on_gc7000`
(`:324-356`), `p_submit` keeping every stream (`:186-209`), `writes`
decoding LOAD_STATEs (`:271-294`), and the descriptor read back through
`read_gpu` (`:254-262`, `render_quad`'s `:422-441`). Gate:

```text
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_depth_sample_path
cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_depth_sample_path
```

The stand-in also counts `wait` calls, and every `cache_invd`/`cache_flush`
that touches the depth surface's allocation or the texture's texels. Each case runs glmark2's calls, not hand-built bindings:

1. **The shadow frame.** A `GL_DEPTH_COMPONENT`/`GL_UNSIGNED_INT` texture
   of 64x64, NEAREST, CLAMP_TO_EDGE, the FBO's only attachment.
   `glClear(GL_DEPTH_BUFFER_BIT)`, then a depth-only draw with depth test
   LESS (the kick K2). Bind framebuffer 0 and draw a quad with `shadow.frag`'s
   sample (`texture2D(s, uv).x` compared in GLSL), then `glFinish`. From the
   depth kick, read the surface address `A = pe_pipe_depth_addr(0)` and
   `GC_PE_DEPTH_STRIDE` `S`. From the sampling kick, decode slot 0 and
   assert, each field by number with the header line (as `gc_npot_repeat.rs`
   does):
   - CONFIG0: TYPE 2D, FORMAT (bits 13..17) = 0x11, ADDRESSING (20..21) = 3;
   - CONFIG1: FORMAT_EXT (0..5) = 0, HALIGN 0, swizzle (R, R, R, 1) under
     `report-es2` and (R, 0, 0, 1) without it;
   - `LINEAR_STRIDE == S`, `SIZE == 64 | 64 << 16`, BASELOD max 0,
     `LOD[0] == A`, `SLICE == S * 64`, LOD 1..15 zero;
   - SAMP_CTRL0: MIN and MAG NEAREST, MIP NONE, U/V CLAMP, UNK21 set,
     INT_FILTER (bit 23) clear, bits 17..20 clear;
   - the golden is A and S from the same run's stream, so it does not
     depend on the allocator.
2. **No wait, no CPU pass.** Across the sampling `glDrawArrays`, with the
   stand-in's fences completing only when `p_wait` is called for them (so
   the depth kick is still pending when the draw is built, and a wait is
   visible): zero `wait` calls, zero cache ranges over the surface or
   the texels, and `gc_depth_resolve` not entered
   (`depth_sample::stats().0` rose by 1). Today's code fails this case (one
   `finish` that waits, the surface's `cache_invd` and the texels'
   `cache_flush`), which shows the test can see the change.
3. **Padding.** 90x70: the surface is 128 words by 128 rows (as
   `tests/gc_depth_resolve.rs`'s note says), so `LINEAR_STRIDE = 512`,
   `SIZE = 90 | 70 << 16`. Also 1x1.
4. **Refusals, by answer.** Each case below answers no and takes the old
   route, so the descriptor is `EXT_R32F` at the texels' `base_gpu`:
   - a texture only cleared (the plane is a clear value);
   - a texture whose surface went to another pair, so its clear is parked
     (cycle `ds_cache::ENTRIES` other FBOs, as
     `depth_resolve_reads_the_linear_depth_as_floats` does in
     `src/vivante/backend_tests.rs`);
   - a texture with two levels;
   - a level-1 render;
   - with `test-knobs`, knob 0.
5. **CPU readers still resolve.** After case 1 without `glFinish`,
   `glReadPixels` of the depth (or `glCopyTexSubImage2D` from it) enters
   `gc_depth_resolve` once and gives the floats of the surface's words. Fill
   the surface through the stand-in's memory at A with
   `tests/gc_depth_resolve.rs`'s pattern before the read.
6. **Keep alive.** Record the sampling draw without a flush, then
   `glDeleteTextures` the depth texture: the stand-in's `gpu_free` is not
   called for A's allocation until the sampling kick's fence has completed.
   The stand-in's `p_completed` lags one fence for this case.
7. **The answer outside the backend.** A GL-core test on the mock table
   (`testutil`) still sees `samples_rendered_depth == false`.

What the host cannot show is whether the TPU returns the depth. That is
`dtpu`'s check 4, already in the record when this test is written.

### (a) The X2 measurement that confirms it

Item x2-gm-after, from `gc-slow-shadow-refract.md` "Confirms the real
fix": PERF page 4 v3 drops from about 15,000 KiB to 0 a frame and v2 drops by
1. Page 0 v0 rises to about 250 to 500 frames per 10 s scene (20 to 40 ms a
frame). Page 0 v2 drops well under 65,535.

## (b) The TPU reads the bytes, not D24X8 on a linear surface: a copy on the GPU

Record: checks 4 and 5 fail, check 6 passes. The TPU addresses the linear
surface (P2's raw A8R8G8B8 view returns the PE's words), but D24X8 does
not decode it in LINEAR mode. Mesa samples D24X8 only SUPER_TILED
(`gc-depth-sample.md` (a) point 3, `R:572-576`, `S:416`). So the cheapest
working path makes a supertiled copy on the GPU and samples that, which is
Mesa's own sample. Two forms, in this order:

### (b).1 Primary: an RS copy into a supertiled depth view

Steps (a).1, (a).4, (a).5, (a).7 and (a).8 hold as written. These change:

- **The copy.** `depth_sample.rs` keeps one supertiled buffer per sampled
  depth object: `stride * rows` bytes, the same 64-aligned shape as the
  surface (`ds_cache.rs:83-85`; a supertile is 64x64, `R:389-392`). It is
  keyed by (id, surface gen), freed on `attachment_gone`, and stamped with
  the kick that last read it. `depth_sample::prepare(gl)` runs in `draw_one`
  beside `tiled_copy::prepare` (`vivante/backend.rs:2873`). For each binding
  with `depth_id != 0`, it records into the open frame **before the draw**
  one RS copy: source = the surface (`ds_cache::depth_view`), LINEAR, stride
  `S`; destination = the buffer, `GC_RS_DEST_TILED`
  (`regs.rs:1043`), `RS_DEST_STRIDE = (S << 2) | TILING | SUPER_TILED_NEW`.
  That is the destination form of `emit_resolve`'s source
  (`vivante/backend.rs:1015-1017`; `state_3d.xml.h:1384`, `:1386` name the
  two bits, and `regs.rs` needs `GC_RS_DEST_STRIDE_TILING` and
  `_SUPER_TILED_NEW`). The format is `GC_RS_FORMAT_A8R8G8B8` both sides,
  Mesa's 4-byte "compatible" format (`etna_compatible_rs_format`,
  `RS:55-65`), so the 32 bits are copied as they are (Mesa's own depth RS
  format is `RS_FORMAT_D32`, `Tr:288-290`, `state_3d.xml.h:87`; either one
  copies raw words when not downsampling). The window is `S/4 x rows`, a
  multiple of 16 wide (`regs.rs:1028`'s rule), with flushes, stall,
  dither off, `SINGLE_BUFFER` around the kick: `emit_rs_copy`'s sequence
  (`vivante/backend.rs:938-960`) with the tiling bits. tiled-copy-rs is
  adding an RS tiling helper for terrain's vertex-stage maps
  (`tiled_copy.rs`, not on this tip). Use that helper rather than a third
  copy of the sequence. The copy is recorded inside `record_all`
  (`:2939-2949`), so an overflow rollback re-records it.
- **The descriptor.** `binding` points at the buffer. `format_words`
  (`texture.rs:251-299`) needs the view's layout: ADDRESSING TILED (0,
  `regs.rs:800`) and `HALIGN = TEXTURE_HALIGN_SUPER_TILED` (2,
  `common_3d.xml.h:126`; Mesa `D:186`, `R:392`). `LINEAR_STRIDE` stays
  `S`, which is Mesa's `levels[0].stride` for a 64-padded supertiled level.
  The tiled-layout flag goes into `GcTexfmt` or the view, not into
  `build_descriptor`'s `tiled` (`:364`), which means "the tiled copy of
  tiled_copy.rs".
- **No wait.** The RS reads the surface after the depth kick in ring order.
  The sampling draw reads the buffer after the RS copy in the same stream
  (`emit_flushes` and the RA-PE stall before the draw).
- **Board check (new, not in `dtpu`).** `dtpu` does not test D24X8 on a
  supertiled copy. Before (b).1 lands, the board must run one more arm:
  knob 3, "RS copy + D24X8 SUPER_TILED", checked as check 4 against P0. It goes in
  gt-depth-tpu's gl_test case (its files) and runs in a second X2 session.
  If it fails, take (b).2.
- **Cost.** One 7.68 MB read plus one 7.68 MB write by the RS a frame on the
  GPU, against the CPU's 5.76 M uncached loads. The RS rate on this part is
  unmeasured (`gc-tiled-targets.md`, "Order" step 0, item gt-tiled-probe).

A related variant has no copy: render depth SUPER_TILED
(`GC_PE_DEPTH_SUPER_TILED`, `regs.rs:423`, at `emit_pe_state`,
`vivante/backend.rs:1513-1521`) and sample in place. It is Mesa's layout, but
it moves every CPU reader of the surface (`resolve_depth`, `copy_plane`,
`seed_depth`) onto a supertile map (gc-layout's `layout.rs`), and the PE's
behaviour under SINGLE_BUFFER(1) with that bit is unmeasured
(`gc-msaa-addresses.md` row (E)). It belongs to tiled-wire-design's "tiled
depth" step, not to depth-sample-gc.

### (b).2 Fallback: a GPU resolve pass into the R32F texels

If D24X8 reads no layout the GPU can make, check 6 still says the raw
A8R8G8B8 view addresses the words. Then a quad pass on the GPU does the
CPU's work: it samples the surface as A8R8G8B8, identity swizzle,
`INT_FILTER` on, NEAREST (`gc-depth-sample.md` (b) route 1, `dtpu` knob 2).
It rebuilds `(a*65536 + r*256 + g) / 16777215` in the fragment program and
writes R32F into the texture's own storage.

- **Mechanism.** The GPU mipmap's (`src/vivante/mipmap.rs`:
  `record_level` `:129-206` through `backend::draw_one`, a linear target at
  the storage's address, a hand-made `GlesDraw`). The target format is
  `GLES_SURF_R32F` (`pe_color_format`, `vivante/backend.rs:313`:
  `GC_PE_FORMAT_R32F`, an ES 3.0 render format of this back end), the
  storage pitch is `stride_texels * 4`, and the source binding is
  `{ format: GLES_TEXFMT_RGBA8, base_gpu: view.gpu, stride_texels: S / 4,
  bgra: 1, .. }`. A8R8G8B8 is the format that `bgra` selects,
  `texture.rs:257-259`.
- **The shader.** A new internal fragment program beside
  `GC_INTERNAL_MIPGEN_PS` (`src/vivante/internal.rs:255`, generated:
  `tools/vivante/` and `internal/es/`, regenerated in the same commit,
  CLAUDE.md "Generated files"). Its body is the decode of
  `gc-depth-sample.md`'s case, highp, the result in `.r`.
- **The hook.** `depth_sample::prepare` in `gc_draw`
  (`vivante/backend.rs:2715`), before `draw_one(draw)`, for each binding
  with `depth_id != 0`. It records the pass (its own kick, as a mipmap
  level is), then clears `depth_id` in the binding it hands on: the draw
  samples the R32F texels, now current on the GPU. The GL core still holds
  the texels stale (it was told so), so a later CPU reader resolves on the
  CPU again and writes the same values. That is correct and redundant. A
  draw that samples is not a CPU reader, so glmark2's frame never pays it.
- **Cost.** One full-screen-sized pass (1.92 Mpx, about 7.5 ms at the 3.9 ns
  a pixel of `imx8mp-frame-floor.md` candidate 2, more with the decode's ALU)
  plus 7.68 MB written. No CPU loop, no `finish`.
- **Board check (new).** The R32F write of the decoded value against P0,
  in gt-depth-tpu's case (knob 4), in the second session.

### (b) `ds_cache.rs`, compare, glmark2

The same as (a): two functions (a).2, and the CPU resolve stays for CPU
readers. (b).1's view is D24X8, so compare works as in (a).9. (b).2 samples
R32F texels, so a compare there meets `gc-shadow-compare.md`'s open "TE
compare on EXT_R32F" question, as today. glmark2 needs no compare either
way.

### (b) Host test: `tests/gc_depth_sample_path.rs`, the same file

Cases 1, 3, 4, 5, 6 and 7 of (a), with these changes:

- **(b).1 golden stream.** In the sampling kick, before the draw's state: one
  RS copy, decoded register by register. `RS_CONFIG` = A8R8G8B8 both sides
  with `DEST_TILED`, no `SOURCE_TILED`, no downsample. `RS_SOURCE_STRIDE = S`.
  `RS_DEST_STRIDE = (S << 2) | 0x8000_0000 | 0x0800_0000`. Source address =
  A, destination = the buffer B, window `S/4 x rows`. Then `SINGLE_BUFFER`
  1, the kicker, and `SINGLE_BUFFER` 0. The descriptor is D24X8, TILED,
  `HALIGN` 2, `LOD[0] == B`, `LINEAR_STRIDE == S`. If ts-golden's Mesa
  golden-stream mode (`tools/vivante/`) can record Mesa's RS blit of a
  Z24S8 resource, compare these RS words with Mesa's.
- **(b).1 host model of the RS.** Fill the surface with
  `gc_depth_resolve.rs`'s pattern. Apply a software supertile of the copy:
  gc-layout's `layout.rs` map, Mesa's supertile order. Then decode every
  texel through the descriptor (TILED, HALIGN SUPER) and compare with
  `gc_depth_resolve`'s floats, bit for bit. This proves the address
  arithmetic, not the RS.
- **(b).2 golden.** One extra kick before the sampling kick: target R32F at
  the texels' `base_gpu`, the internal program's code address, slot 0 =
  A8R8G8B8 LINEAR at A with stride S, NEAREST, INT_FILTER set. The
  sampling draw's slot 0 is then `EXT_R32F` at `base_gpu`, as today.
- Case 2 becomes: zero `wait` calls, zero CPU ranges, `gc_depth_resolve`
  not entered.

## (c) Neither: the CPU resolve stays

Record: check 6 fails. The LINEAR descriptor does not address the surface
even as bytes, so no view and no in-stream copy is known to work.
depth-sample-gc then makes no code change. It closes with the record and a
comment on #1204, and `dtpu`'s knob stays test-only. Three CPU fixes
remain, each its own item of plan #328. They touch none of
depth-sample-gc's files except `ds_cache.rs`, which belongs to resolve-wide.

### (c).1 resolve-wide (running): the loads

`ds_cache::resolve_depth`'s loop (`ds_cache.rs:604-613`) reads each texel
with three `ldrb` on `aarch64-unknown-none` (`+strict-align`;
`gc-slow-shadow-refract.md` "The loop's code"). The fix reads aligned
`u32` (or 16-byte) words of the 64-byte-aligned surface. Host tests:
`tests/gc_depth_resolve.rs` stays bit-exact, and
`tests/gc_depth_resolve_loads.rs` (resolve-wide's) checks the target's code.
The same `as_chunks` form is in `copy_plane` (`:474`) and `seed_depth`
(`:533`), which are not per frame.

### (c).2 resolve-own-kick: wait for the depth kick, not every slot

- `src/gl/texture.rs:709`: `backend::kick_pending(kick) && (flush() != 0 ||
  finish() != 0)` becomes `flush()`, then `backend::wait_kick(kick)`
  (`src/backend.rs:1978-1981`), falling back to `finish()` when it is
  `None` (the contract the doc of `wait_kick` already gives).
- `src/vivante/backend.rs:3381`: `wait_kick: Some(gc_wait_kick)`. A new
  `gc_wait_kick(kick_id)` finds the frame whose `kick_id` matches (the
  search of `kick_pending`, `:3276-3281`) and `wait_frame`s it (`:3181-3190`).
  A kick older than every slot has completed. The open frame's kick is
  submitted by the `flush` before it. Then `retire_wide_targets`
  (`:3287-3289`), as `finish` does.
- Gain: refract's resolve no longer waits for its 10 mipmap kicks
  (`gc-slow-shadow-refract.md` cause 3). Shadow gains nothing (K1 and K2 are
  the only kicks).
- Host test: in `src/vivante/backend_tests.rs`, or a new
  `tests/gc_wait_kick.rs`, a stand-in whose fences complete on demand. Record
  a depth kick and three later kicks, call the GL core's `resolve_depth`
  path, and assert that `wait` was called for the depth kick's fence only.
  Also `gc_wait_kick` of the open kick after `flush`, and of a retired kick
  (no wait).

### (c).3 gc-cache-skip: no DC maintenance on the non-cacheable carve-out

- The two ranges of the frame: `ds_cache.rs:598` (`cache_invd`, the whole
  surface allocation) and `gl/texture.rs:732` (`cache_flush`, the texels).
- The fix is in the OSAL, where the mapping is known. `z_cache_flush` and
  `z_cache_invd` (`src/osal/zephyr_gc.c:359-369`) return at once for a range
  inside the carve-out, which is Normal non-cacheable for every user
  (`:154`; `z_gpu_write` already says so, `:343-345`). Do the same for
  `gc_render_cache_flush`/`_invd` (`driver/src/vivante/gc_render.c:461-469`)
  after fence-irq, which owns `gc_render.c`. The Rust callers
  (`src/osal/mod.rs:492`, `:505`) do not change.
- Host test: a C unit test of the range check in the Zephyr OSAL (a pure
  `in_carveout(base, size, p, n)` function, compiled for the host the way the
  OSAL's other host-built C is), or a Rust test over a stand-in OSAL that
  counts the calls reaching the cache ops.
- Gain: an estimated 2 to 25 ms a frame (unmeasured).

### (c) What else stays

`ds_cache.rs` keeps today's design. No `depth_view` and no `note_sampled`
are added, because nothing samples the surface. `src/backend.rs`,
`vivante/texture.rs` and `regs.rs` are unchanged. `samples_rendered_depth`'s doc
(`backend.rs:1848-1850`) and `gl/texture.rs:679-684` name the record that
answered no.

Even with all three fixes, shadow and refract keep a 1.92 M-texel CPU pass
a frame. The resolve-wide estimate is 50 to 350 ms left
(`gc-slow-shadow-refract.md` cause 1, "Fix, cheap"). The fill-side fixes of
`gc-tiled-targets.md` (TS-aware resolve, step 3) would then be the next
lever. Their gain shrinks to 0 in (a) and (b).

## What this note does not decide

- Whether the TPU returns the depth, how many bits it returns, and whether
  the texture cache is coherent: `dtpu` checks 4, 5 and 7, run by
  x2-depth-tpu-run.
- Whether D24X8 reads a supertiled RS copy ((b).1), and whether the RS copies
  D24S8 words unchanged as A8R8G8B8: a second board session, after
  gt-depth-tpu adds the arm.
- The RS's rate on this part, which sets (b).1's cost: gt-tiled-probe.
- How the depth view changes once depth gets a tile status or a tiled layout
  (tiled-wire-design, `gc-tiled-targets.md` steps 2 and 3). `depth_view`
  then has to report the layout, and refuse while a TS is not resolved (or
  bind it as Mesa's sampler TS, `T:79-159`). `gc-depth-sample.md` (c),
  "Fast clear / tile status", gives the rule.
