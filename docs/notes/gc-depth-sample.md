<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# GC7000: can the texture unit sample the D24S8 depth surface?

A read-only study. `ds_cache::resolve_depth` (`src/vivante/ds_cache.rs:477`)
turns the depth surface into R32F texels on the CPU, behind a `finish()`, once
a frame in glmark2 `shadow` and `refract` (`docs/notes/gc-depth-resolve.md`,
`docs/notes/imx8mp-glmark2-cpu.md` "Fix items, in order", item 2). The V3D
samples its depth buffer itself and says so through
`backend::samples_rendered_depth` (`src/backend.rs:1678`) and
`GlesTextureBind::depth_id`. This note asks whether the GC back end can do the
same. Nothing here was run on a board.

## Verdict: unknown, leaning yes by one route

**Unknown.** No TPU format that means "D24S8" is known on this part, and one
board experiment decides the rest. The surface *can* be addressed by the
texture unit (it is plain row-linear memory, see (a)); what is open is how its
words are read. There is a route that needs no new format: sample the words as
an 8888 texel and rebuild the 24-bit depth from the channels. It is a yes only
after the board check at the end of (a).

### Sources, and what is missing

`third_party/` holds no etnaviv source: the vendored tree is
`mesa-26.2.1/{docs,include,licenses,src}` with no `etnaviv` directory, and the
repository's own copies of etnaviv's register headers are only the values
`src/vivante/regs.rs` lists. So `etnaviv_texture.c`, `etnaviv_format.c` and
`common_3d.xml.h` could not be read for this study; every statement about them
below is either quoted from a comment in this tree that names the file, or
marked *from memory, unchecked*. Before any code relies on a value, read it
against Mesa's `src/gallium/drivers/etnaviv` at the commit
`src/vivante/VIVANTE_NOTES.md` pins (Mesa main `c8ce6c8bdfbb` is named in
`src/vivante/texture.rs:422`).

In-tree sources used:

- `src/vivante/regs.rs:850-866`: the TEXDESC format values the back end knows
  (`common_3d.xml.h TEXTURE_FORMAT_*`): A8 0x01, L8 0x02, A8L8 0x04,
  A8R8G8B8 0x07, X8R8G8B8 0x08, A8B8G8R8 0x09, X8B8G8R8 0x0a, R5G6B5 0x0b,
  ETC1 0x1e; extended set (CONFIG1, CONFIG0 format 0): R16F 0x07,
  A16B16G16R16F 0x09, R32F 0x0a, ETC2 0x01/0x02. **No D24S8, Z24 or depth
  value is among them.** *From memory, unchecked*: etnaviv has no depth entry
  in its texture format table that samples a depth buffer in place; it
  samples a depth texture through a copy (an RS resolve into a colour-format
  texture) and compares in the shader.
- `src/vivante/ds_cache.rs:459-476` (doc of `resolve_depth`) and
  `src/vivante/backend.rs:1455-1473`: the layout, measured on the i.MX8MP
  2026-09-30 (gl_test 169).
- `src/vivante/texture.rs:107-299` (`build_descriptor`), the descriptor the
  back end writes today.

## (a) Can the TPU read the surface, and as what

**Layout.** The PE writes the D24S8 surface **row-linear** at `Surface::stride`
(`ds_cache.rs:80-83`: `align(w, 64) * 4` bytes a row, `align(h, 64)` rows),
row 0 at GL's y 0. `PE_DEPTH_CONFIG` never sets `GC_PE_DEPTH_SUPER_TILED`
(`regs.rs:420`, `backend.rs:1465-1473`). It is not 4x4-tiled and not
supertiled, and it has no tile status: TS mode 0, so memory holds every pixel
(`ds_cache.rs:471-473`). Each word is `depth << 8 | stencil`: depth is bits
31..8, stencil bits 7..0 (`ds_cache.rs:512`, `>> 8`). That is exactly the
layout of a LINEAR texture (`GC_TEX_ADDRESSING_LINEAR`, the mode
`build_descriptor` already uses at `texture.rs:181-184` for every untiled
texture), so the addressing side needs nothing a normal RGBA8 texture does not.

**Format.** No documented depth format (above). Two readings of the words are
available from the values in `regs.rs`:

1. **ARGB8888 view, depth rebuilt in the shader (the route that needs no new
   format).** A little-endian word `D2 D1 D0 S` (depth byte 2, 1, 0 in bits
   31..24, 23..16, 15..8; stencil in 7..0) read with
   `GC_TEX_FORMAT_A8R8G8B8` (0x07, memory order B,G,R,A) gives
   B = stencil, G = depth bits 7..0, R = depth bits 15..8, A = depth bits
   23..16. With the descriptor's swizzle (`GC_SWIZZLE_*`, `regs.rs:868-873`)
   the three depth channels can be routed to R, G, B and the stencil dropped
   (A = `GC_SWIZZLE_1`). The texel then arrives as three unorm8 values
   `c = k / 255`. The depth the CPU resolve builds is
   `((word >> 8) as f64 / 16_777_215.0) as f32`
   (`ds_retention::unorm24_depth`), i.e. `(A*65536 + R*256 + G) / 16777215`
   with the three bytes as integers. A shader rebuilds it as
   `(round(255*a)*65536 + round(255*r)*256 + round(255*g)) / 16777215`: every
   intermediate is an integer below 2^24, exact in fp32, so the result equals
   the CPU's value to the last bit if the TPU returns `k / 255` correctly
   rounded (not measured). The filter must be NEAREST on the depth channels:
   a bilinear filter of the three bytes is not a bilinear filter of the depth
   (carry between bytes), and a shadow compare wants the nearest or a
   per-texel compare anyway. `SAMP_CTRL0 INT_FILTER`
   (`texture.rs:301-304`) is the 8-bit path; it stays on.
2. **A depth format** (*from memory, unchecked*): some Vivante parts
   document a depth sampler format; etnaviv's table is where it would show.
   Unknown whether the GC7000UL has one. If it does it replaces the shader
   rebuild with a plain `.r` read and the compare stays as the shader's own.

**Descriptor fields** (route 1, against `build_descriptor`, `texture.rs:107`):

| field | value | today's code |
|---|---|---|
| CONFIG0 `TYPE` | `GC_TEX_TYPE_2D` | `texture.rs:123-128` |
| CONFIG0 `FORMAT` | `GC_TEX_FORMAT_A8R8G8B8` (0x07) | the existing `pe_order` branch sets this value (`:176-178`) |
| CONFIG0 `ADDRESSING_MODE` | `GC_TEX_ADDRESSING_LINEAR` | `:181-184` |
| CONFIG1 swizzle | R<-A (depth 23..16), G<-R (15..8), B<-G (7..0), A<-1 | the existing swizzle fields, `:200-208`, with a new mapping |
| `LINEAR_STRIDE` | the surface's `stride` (a multiple of 256) | `:215-226` uses `stride_texels * bytes` |
| `SIZE` | the texture's width x height | `:227` |
| `LOG_SIZE_EXT`, `BASELOD` | one level: `levels = 1` | `:228-229` |
| `SLICE`, `3D_CONFIG` | stride * height, depth 1 | `:232-243` |
| LOD 0 address | surface GPU address **+ guard** (`GC_DEPTH_GUARD_BYTES`, `backend.rs:87-89`) | `:269-270` takes `t.base_gpu` |
| `SAMP_CTRL0` | NEAREST min and mag, CLAMP_TO_EDGE as the texture asks, `INT_FILTER` on | `texture.rs:301-311` |

The surface's width in memory is `stride / 4` words, wider than the texture
(`align64`): `SIZE` is the texture's, `LINEAR_STRIDE` is the surface's, the
same relation `stride_texels` has to `width` for a padded colour texture.

**Fast clear and tile status: none to resolve.** The GC back end never clears
with the RS engine: a glClear is a quad through the PE, depth test always,
write on (`VIVANTE_NOTES.md:109-110`), and there is no BLT engine
(`:1205`). `DISABLE_ZS` is set when nothing needs the depth tile status
(`backend.rs:1475-1477`). So no TS flush or resolve sits between the depth
pass and the sample. One real exception: a plane that holds **a whole clear
only** has no memory behind it (`ds_cache.rs:488-496`, the `fill` path: the
value is kept in `Plane::cleared_value`). That surface's words are not
written. It must answer "not sampleable" (the CPU resolve, which is cheap for
a constant, stays for it).

**The board check that decides the verdict** (a later item, not this one):
render the ramp of gl_test 169 into a depth texture, bind the surface with
the table above, and read it back through a trivial fragment shader that
writes the rebuilt depth to a colour target. Pass: every texel equals
`unorm24_depth` of the words, including the extremes `0x000000ff`,
`0xffffff00`, `0x80000000`. Also try the swizzle with the stencil kept (A8R8G8B8
with an identity swizzle) once, to read the stencil byte on its own and prove
the byte order above. Fail on any step: the verdict becomes no, and the CPU
resolve stays.

## (b) What would change in the GC back end

Mirror of the V3D: `v3d_depth_sampled` (`src/v3d/backend.rs:3557`),
`depth_buffer_state`/`depth_sample` (`:3600`), `GlesTextureBind::depth_id`.

1. **The answer.** `backend::samples_rendered_depth` (`src/backend.rs:1678-1688`)
   is `cfg(gpu-v3d)` only: add a `gpu-vivante` arm calling a new
   `vivante::backend::gc_depth_sampled(id, w, h)`. The GL core's caller
   (`gl/texture.rs:780`, `depth_for_draw`) needs no change: it already asks,
   submits the kick (`flush`, no wait) and puts `depth_id` into the binding.
   The comment at `backend.rs:1670-1672` and `gl/texture.rs:680-684` ("not
   tried on either board") are then updated.
2. **A lookup in the cache.** New `ds_cache::sampleable(id, w, h) ->
   Option<(gpu_addr, stride)>`, next to `resolve_depth` (`ds_cache.rs:477`),
   which finds the surface the same way (`mine(...)`, `:493-496`) and answers
   `Some` only when: the depth plane is **Live** (not
   `cleared_value()`, not parked), `scale == 1` (the `mine` filter), the key's
   size covers `w` x `h`, and `stride >= w * 4`. It reports the surface's GPU
   address plus `GC_DEPTH_GUARD_BYTES` (resolve reads `all[guard..]`, `:518`)
   and `Surface::stride` (`:83`). It must not wait: pending kicks are fine,
   they are ordered before the draw in the same ring (the GL core flushed
   first); only the CPU read of `resolve_depth` needs `finish`.
3. **The descriptor.** `build_descriptor` (`vivante/texture.rs:107`) and
   `emit_slot` (`:314`): when `t.depth_id != 0`, take the surface from
   `ds_cache::sampleable` and write the table above instead of the texture's
   own `base_gpu` (`:269`), `stride_texels` (`:215-226`) and format
   (`format_of`, `:74-105`). The `lum`/`depth_luminance` swizzle (`:200`)
   must be reconciled: the OES_depth_texture luminance mapping becomes a
   second stage after the rebuild. A refusal (`sampleable` -> `None`) has to
   be visible to the draw: the GL core already left the texels stale on the
   strength of the answer (`gl/texture.rs:745-753`), so a refusal at descriptor
   time must fail the draw with a named log (as `depth_sample` does for the
   V3D, `v3d/backend.rs:3592-3600`), not sample a stale copy.
4. **The shader.** Route 1 needs the rebuild in the fragment (and vertex)
   program that samples the texture. The shader table (`src/shaderc`,
   the online compiler, `docs/online-shaderc/`) compiles the app's
   `texture2D(shadowmap, uv).r` as a plain sample. A variant key bit "this
   sampler is a packed depth view" must lower the sample to the three-channel
   sample plus the rebuild above. That is the largest part and it is in
   `src/shaderc`, a different area from `src/vivante`; route 2 (a real depth
   format) needs none of it. Alternative without shader work: **a GPU
   resolve pass**, a full-target quad (the mechanism of `vivante/mipmap.rs`,
   `GC_INTERNAL_MIPGEN_PS` in `vivante/internal.rs`) that samples the surface
   as above and writes the rebuilt depth into the texture's R32F storage
   (needs R32F as a PE colour target: unmeasured). It keeps `depth_id == 0`,
   so shaderc and `build_descriptor` are untouched, drops the CPU loop and
   the `finish`, and costs one full-screen quad. Choose it first; the
   per-sample rebuild is the second step.
5. **Mirror of the V3D's bookkeeping.** `GlesTextureBind::depth_id` reaches the
   backend as `abi_layout.rs:255` (offset 52); `abi_layout` and `backend.h:207`
   need no edit. `backend.rs:3084` (`depth_resolve: Some(gc_depth_resolve)`)
   stays: the CPU readers still resolve (below).
6. **Keep alive.** The depth surface must not be recycled (`victim`,
   `ds_cache.rs:557`) while a later draw samples it: `used_by` already
   records the latest kick that used it, and `victim` skips surfaces with
   live planes, so nothing new is needed for a live plane.

## (c) Risks

- **Fast clear / tile status.** None on this core (above); the exception is the
  cleared-only plane with no memory. If a later change adds RS clears or a
  tile status to depth, `sampleable` must refuse until a TS resolve ran.
- **Cache coherence.** The PE writes depth through the depth cache; the TPU
  reads through the texture cache. The back end flushes
  `COLOR|DEPTH|TEXTURE|SHADER_L1` before each draw (`VIVANTE_NOTES.md:1200`),
  which covers a depth pass followed by a sampling draw in one submit; a
  descriptor INVALIDATE follows the address writes (`texture.rs:446-453`).
  Not measured for depth specifically.
- **Precision.** Route 1 is exact in fp32 *if* the TPU returns `k/255`
  correctly rounded; a TPU that returns a truncated or approximate value
  breaks the integer recovery by one in the low byte. The fix is `round`, and
  the board check above compares all 2^24 steps along the ramp. The
  CPU path's 24-bit unorm to f32 (`k / 16777215` as f64 then f32) is the
  reference; a rebuild in fp32 divides by a float, one rounding, equal to it
  for every k (checked exhaustively by the unorm24 item, not here).
  Filtering: LINEAR on the three bytes is wrong (carry); the descriptor must
  force NEAREST, but GL's texture may ask LINEAR (glmark2 shadow uses
  `GL_NEAREST`? unchecked): a LINEAR request needs the resolve instead.
- **Stencil byte.** Part of the same word; with the swizzle above it is
  dropped. A stencil-only or combined texture is not this path.
- **The GL core's CPU readers of the texels.** With sampling in place the
  texels are *stale* until a CPU access. Every reader already goes through
  `settle_texture`/`resolve_depth` (`gl/texture.rs:740-752`, as for the V3D):
  glReadPixels of the depth, glCopyTexSubImage, glBlit, glGenerateMipmap of
  the texture, a glTexSubImage that keeps the rendered rest, tf capture
  (`gl/tf_capture.rs:1332`). They call `backend::depth_resolve`
  (`gc_depth_resolve`) and must keep working on the same surface; so the
  surface may not be reclaimed between the sampling draw and a later CPU
  read. `ds_cache::gone` (`:387`) frees a surface only when nothing holds a
  live plane, which a sampled one does until its object is deleted.
- **Size.** A surface is `align64` in both directions and one cache entry per
  (depth, stencil) pair; a texture whose key does not match (a stencil
  attachment that differs) is a different surface: `sampleable` answers none.
- **Multisample / scale != 1:** refused by the `mine` filter.
- **Shader cost.** The rebuild is 3 sample channels and a few ALU ops per
  depth lookup; shadow's fragment shader samples once. Against 2 ms of CPU
  resolve a frame on the host and a 7.68 MB write, not a regression.

## (d) Draft host test (for the fix item)

`tests/gc_depth_sample.rs`, with the stand-in OSAL and the submit model of
`tests/gc_mipmap_gpu.rs` (host memory per allocation, `submit` decoding each
draw's LOAD_STATEs). Gate:

```text
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]
```

Steps:

1. Init the real back end (`gc_init`, `yakogl_backend_vivante`) over the
   stand-in OSAL. Draw once into a depth texture's pair through
   `GlesDraw { depth_id: id, ... }` with a depth attachment (as
   `pair_draw`/`ds_key` of `backend_tests.rs` do), and fill the surface's
   words through `ds_cache::memory_of(key)` with the pattern of
   `tests/gc_depth_resolve.rs`: xorshift words plus `0x000000ff`,
   `0xffffff00`, `0x80000000`, at the surface's own stride.
2. **The answer.** `gc_depth_sampled(id, w, h)` is true for a Live plane of
   that size and false for: another id, a larger size, an
   `id` whose plane is a whole clear, and a parked clear. With no GPU
   backend the shared `backend::samples_rendered_depth` stays false.
3. **The descriptor.** Draw with a binding `GlesTextureBind { depth_id: id,
   width: w, height: h, ..}` (as `gl::draw::texture_bind_2d` builds one) and,
   in the OSAL's `submit`, decode the texture descriptor
   (`GC_TEXDESC_CONFIG0`, `CONFIG1`, `LINEAR_STRIDE`, `SIZE`, LOD 0 address)
   and assert: format 0x07, LINEAR addressing, the swizzle table above,
   `LINEAR_STRIDE == ds_cache::stride_of(key)`, `SIZE == (w, h)`, LOD 0
   address `== surface gpu + GC_DEPTH_GUARD_BYTES`, one level.
4. **The values.** The model of the fragment program (as `gc_mipmap_gpu.rs`
   runs the mipgen quad) reads each texel of the surface through the
   descriptor and rebuilds `(a*65536 + r*256 + g) / 16777215` with the bytes
   from the model's unorm8 read (`c/255` rounded back); assert it equals
   `gc_depth_resolve`'s output for the same surface, **bit for bit** at every
   texel, for `w` x `h` of 64x64, 90x70 (padding), and 1x1.
5. **Refusals.** The draw after the surface is recycled (cycle
   `ds_cache::ENTRIES` other pairs, as `depth_resolve_reads_the_linear_depth_as_floats`
   does) fails with `-ENOTSUP`/logged, and samples nothing stale.
6. **No wait.** Count the OSAL's `finish`/wait calls across
   `samples_rendered_depth` + the draw: zero (the CPU resolve's `finish` is
   what this removes).

What it cannot show: whether the texture unit returns what the model returns.
That is the board check at the end of (a).
