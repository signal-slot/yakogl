<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# V3D: 4x multisampling of several colour attachments and of ES 3.0 colour formats

Research, no code. `draw_call` (`src/v3d/backend.rs`, after `let target =
Target {..}`) refuses two shapes by name: "4x multisampling of several colour
attachments" and "4x multisampling of an ES 3.0 colour format". Both comments
say the GL core makes no such surface. That is true today, and for a reason
that sits in the GL core: an application's multisample renderbuffer never
reaches the back end as 4x. This note says what the GL core makes (1), how
Mesa's v3d keeps four samples for several render targets (RTs) and for wide
formats (2), what the job builder and `src/v3d` need (3), what the GL core
needs (4), and which dEQP rows each part should move (5). The work splits in
two: a V3D part that is useless alone (nothing hands it a 4x FBO) and a
GL-core part that is harmful alone (it would hand 4x FBOs to back ends that
do not draw them, GC7000UL and PowerVR, unless a capability gates it).

Line numbers: yakogl is main at `a8e79b52` and drifts, function names are the
address. Mesa is 26.2.1, paths relative to `src/gallium/drivers/v3d/` unless
they start with `src/`. The vendored `third_party/mesa-26.2.1` holds only the
format table; every other Mesa file below was read in the full 26.2.1 tree
(the deps cache's `mesa-pan/mesa-26.2.1`). Mesa is described in words, not
copied.

## (1) What the GL core makes today

- `GL_MAX_SAMPLES` is the constant 1 (`src/gl/state.rs`, `GL_MAX_SAMPLES => 1`).
  `glRenderbufferStorageMultisample` (`src/gl/framebuffer.rs`) gives
  `GL_INVALID_OPERATION` for `samples > 1` and, for 1, stores
  `rb.samples = 1`: one sample a pixel, "as plain storage, but a framebuffer
  of it has sample buffers" (`GlRenderbuffer::samples`, `src/gl/context.rs`).
  Integer formats with `samples > 0` are `GL_INVALID_OPERATION` (ES 3.0
  4.4.2.1), which is the spec's rule, not a limit: **ES 3.0 has no
  multisampled integer formats, so the V3D never has to keep or resolve
  integer samples** (see (2)).
- `GlesDraw.samples` is `default_samples` for the window and the literal `1`
  for every framebuffer object (`src/gl/draw.rs`, `d.samples = if
  c.bound_framebuffer == 0 { c.default_samples } else { 1 }`, in both the
  draw and the clear builders). Sample coverage, alpha-to-coverage and the
  sample mask (`sample_coverage_mask`, `needs_sample_kill`, the `samples <= 1`
  early out around `d.sample_mask`) read the same expression, so an FBO never
  gets them.
- `glBlitFramebuffer` (`src/gl/blit.rs`): a CPU copy. `fb_samples` reads the
  attachments' `rb.samples`; a multisample draw framebuffer or a multisample
  read with differing rectangles is `GL_INVALID_OPERATION`; a multisample read
  resolves only between identical rectangles of identical (linear) formats.
  Because the storage is one sample, "resolve" is a plain copy of the
  resolved pixels. This is the contract a real 4x renderbuffer can keep: if
  the back end always leaves the *resolved* image in `rb.mem` (3.2), blit,
  `glReadPixels` and sampling need no change.
- The 4x surface exists only as the window: `EGL_SAMPLES 4` configs
  (`default_samples == 4`, `msaa_on`, `v3d_debug_msaa`). Then `d.samples == 4`
  reaches the V3D, with `rts == 1`, RGBA8, not typed: the one shape the job
  builder supports.
- `glGetInternalformativ` answers one sample count (1), `GL_SAMPLES` of a
  complete FBO is its renderbuffers' count (0 or 1).

So the two V3D refusals cannot fire from an application today; they are
guards for the day (4) lands. Until then every multisample renderbuffer test
fails or is `NotSupported` in the GL core (5).

## (2) How Mesa's v3d keeps four samples per RT

Which resources are multisampled: the screen answers `is_format_supported`
for `sample_count == 4` only (`v3d_screen.c`: `sample_count > 1 &&
sample_count != V3D_MAX_SAMPLES` is false), `max_samples = 4` in the caps
(`v3d_screen.c`, `.max_samples = 4`). So GL_MAX_SAMPLES is 4 and a request
for 2 is rounded to 4. A job is multisampled when any colour, Z/S or blit
surface has `nr_samples > 1` (`v3d_job.c`, `v3d_get_job`:
`job->msaa = true` in each of the three attach branches), or when the
framebuffer state says `samples >= 1` (`v3d_get_job_for_fbo`).

**One bit for the whole job.** `multisample_mode_4x` is set in both
`TILE_BINNING_MODE_CFG` (`v3dx_draw.c`, `v3dX(start_binning)`) and the
rendering mode config (`v3dx_rcl.c`, :763-764). Every RT, the depth and the
stencil buffer of the job are then 4x. There is no per-RT choice; a mixed
attachment set (one multisample renderbuffer, one texture) is already
`GL_FRAMEBUFFER_INCOMPLETE_MULTISAMPLE` in the GL (`attached_samples`).

**Tile buffer size per RT and bpp.** `v3d_get_tile_buffer_size`
(`v3d_context.c`) takes, over the job's colour surfaces, the largest internal
bpp and the highest RT index in use, then `v3d_choose_tile_size`
(`src/broadcom/common/v3d_util.c`). For V3D 4.x (the Pi 4) the table index is
the sum of: 0/1/2 for 1/2/3-4 RTs (3 for 5+), +2 for 4x MSAA, +the maximum
internal bpp (0 for 32, 1 for 64, 2 for 128), into 64x64, 64x32, 32x32,
32x16, 16x16, 16x8, 8x8. The extremes on a Pi 4: one RGBA8 at 4x is 32x32;
a 64-bpp RT (sRGB8_A8, RGB10_A2, RGBA16F all store as 16F, bpp 64) at 4x is
32x16; four RTs, 4x and a 128-bpp one is 8x8, still valid (index 6 of 7).
The count of RTs matters through the TLB's 4 colour RTs of 64 bpp-class
space; Mesa has no further check on the V3D 4.x path, and V3D 7.x sums the
real bpp (`total_color_bpp * 4`), which does not apply to the Pi 4.
yakogl's `v3d_frame_tiling_gen` (`driver/src/v3d/v3d_frame.c`, lines
59-85) already has the same index sum with `msaa ? 2 : 0` and `max_bpp`, and
`v3d_job_end` calls `v3d_frame_tiling_bpp(.., job_rts, msaa, max_bpp)`. The
tile arithmetic needs no work for several RTs at 4x or for wide formats.

**Samples vs resolve, per RT.** `v3d_rcl_emit_stores` /`store_general`
(`v3dx_rcl.c`:127-170): a surface whose texture has `nr_samples > 1` is
stored `ALL_SAMPLES` (the samples, into the multisample resource); the
resolve is a second store of the same RT into a single-sample resource with
decimate `4X`. `load_general` (:60-110) loads a multisample surface with
`ALL_SAMPLES`, a single-sample one with `SAMPLE_0`. All of it is per RT
(`RENDER_TARGET_0 + i`, the loop in `v3d_rcl_emit_loads` over `nr_cbufs`),
nothing is special about RT 0 in Mesa. The multisample resource's layout is a
single-level UIF at twice the width and height of the surface, with the
format's own cpp (`v3d_resource.c`:612-642: `uif_top |= msaa`, the
`if (msaa)` branch of the slice loop doubles both sizes); the load/store
`output_image_format` is the RT's own (`v3d_get_rt_format(.., psurf->format)`,
not RGBA8), `r_b_swap` by `v3d_format_needs_tlb_rb_swap`.

**Integer formats cannot be resolved by averaging, and need not be.** The
TLB's `4X` decimate averages. Mesa takes the TLB resolve in a blit only when
`v3d_format_supports_tlb_resolve_and_blend` (`v3d_formats.c`:148-163)
holds: the internal type is `8` or `16F`, nothing else; for every other
internal type (`8I`, `8UI`, `16I`, `16UI`, `32I`, `32UI`, `32F`, ...) the
resolve falls to the blitter's shader path (`v3d_blit.c`:382-387 returns
false for the TLB path, and the generic `util_blitter` resolve draws it). The
same predicate decides soft blending (`v3dx_state.c`:517-525,
`v3d_fb_needs_soft_blend`). For yakogl this is an exclusion, not work: ES 3.0
4.4.2.1 refuses multisample integer renderbuffers (already in
`glRenderbufferStorageMultisample`), and the ES 3.0 multisample-renderable
colour formats are RGBA8, SRGB8_ALPHA8, RGB10_A2, RGB8, RGB565, RGBA4,
RGB5_A1, R8, RG8 (and, with EXT_color_buffer_float/half_float, R16F, RG16F,
RGBA16F, R11F_G11F_B10F). In yakogl's tables (`rt_internal`,
`driver/src/v3d/v3d_job.c`:151-) the first group is internal type `8` (the
GL core stores everything of 8 bits or fewer in RGBA8888, so those formats are
the existing RGBA8 case) and the sRGB8_A8, RGB10_A2 and 16F rows are `16F`
at 64 bpp: both resolvable by the TLB. R11F_G11F_B10F and the 32F rows have
no `rt_internal` row and stay refused (the 32F multisample formats are
`NotSupported` in dEQP anyway, and Mesa would resolve 32F by shader).

**What 64-bpp formats change.** Only the numbers: `internal_size = 4 << bpp`
for the clear colour words (`v3dx_draw.c`:1704, already `clear_words`), the
tile size above, and the multisample buffer's cpp (4 for sRGB8_A8 and
RGB10_A2, 2/4/8 for R16F/RG16F/RGBA16F). The kept samples are stored in the
RT's output format, so `v3d_tex_layout_msaa(w, h, cpp, ..)` takes the format's
cpp where yakogl passes the literal 4 (`ms_buf`). The sRGB conversion
happens in the TLB at the load and store (the tile buffer holds linear
halves), identically for the samples store and the resolve store.

## (3) What the job builder and `src/v3d` need (the V3D part)

Job builder, `driver/src/v3d/v3d_job.[ch]`:

1. Lift two guards in `v3d_job_begin`: `t->msaa && t->rts > 1` and
   `t->typed && t->msaa` (both set `j->broken`). Keep `msaa && !t->address`
   (a 4x depth-only framebuffer has no colour; `draw_call` also refuses it) and
   the `uif_*` render-image rule (no render image at 4x; a 4x texture is not
   a thing in ES 3.0).
2. Per-RT sample buffers. `struct v3d_job_target` has `ms_address`,
   `ms_memory_format`, `ms_height_in_ub` for RT 0 only, and `struct
   v3d_job_rt { address, stride }` for RT 1..3. Add the three fields to
   `v3d_job_rt` (appended; `rt[3]` is the last-but-one member, so mind the
   Rust mirror `src/v3d/ffi.rs` `struct v3d_job_target`, which must change in
   the same commit). Both are plain `repr(C)` structs; no field in the
   middle moves if the new ones go at the end of `v3d_job_rt` and the array
   is re-laid in the mirror.
3. `emit_tile_list`: at 4x, the load of RT n (`c->load_rt[n-1]`) comes from
   its sample buffer, `ALL_SAMPLES`, with `rt_fmt(&j->t, n)` (RT 0 today:
   RGBA8 literal in the 4x load and in the kept store, which is right only for
   the RGBA8 shape). The stores of RT n: the kept samples (`ALL_SAMPLES`,
   sample buffer) then the resolve (`4X`, raster target) as RT 0 does now
   (the `if (rt0 && j->t.msaa && j->t.ms_address)` block and the resolve
   below it), with `stores` counted for each. `st.r_b_swap` per RT (typed
   targets carry none today). The end-of-frame store flag (`stores == 0` ->
   NONE) is per-job and unchanged.
4. `v3d_job_end`: "a 4x job that loads RT 0 needs the kept samples" becomes
   per RT. A load with no buffer breaks the job (`j->broken`), the backend
   then starts the RT from the clear colour as it does for RT 0 today.
5. The rendering mode config and the per-RT `RENDER_TARGET_*` config words
   already carry type/bpp/clamp for typed targets (`rt_internal`, the loop at
   `v3d_job.c`:1267), and `multisample_mode_4x` is already set from `t->msaa`
   in both places. The tile-coordinates/`TILE_BINNING` path needs no change.
6. The fragment shader side: `rasterizer_oversample_mode` and the sample mask
   (`v3d_job.c`:562, :746) are per draw and RT-independent. Writing several
   RTs from one fragment shader at 4x is the same TLB write, per sample;
   nothing in the compiler's output differs (Mesa's `force_per_sample_msaa`,
   `v3dx_draw.c`:568, is a shader-feature switch, not an RT one).

`src/v3d/backend.rs`:

- `MsBuf` (`MS_BUFS = 2`, "the window surfaces a client multisamples") is
  keyed by the colour target's address alone and holds one RGBA8 layout. It
  becomes keyed by (target address, RT index), sized by the format's cpp
  (`v3d_tex_layout_msaa(w, h, cpp, ..)`), with room for the live set: a
  renderbuffer-based FBO with up to 4 multisample RTs plus the window's two.
  Eviction by least-recently-used already exists (`ms_buf`); at 4 RTs the
  pool must hold at least 4 or an FBO evicts its own RT within one job
  (`MS_BUFS >= 6`, or a per-job guarantee that the RTs of the running job
  are never the victim). Memory: a sample buffer is 4 x the pixels x cpp
  (UIF, twice each side): a 1024x1024 RGBA8 RT is 16 MiB, four 64-bpp RTs
  128 MiB, a 2048x2048 one 64 MiB per RGBA8 RT. dEQP's renderbuffers are
  small (a few hundred pixels a side); state the cost, decide a size cap
  (see the open points).
- `Target::msaa` stays `d.samples == 4`; delete the two refusals and the
  `rts > 1` clear refusal does not change (that one is the several-attachment
  clear item, another note). `Target::typed()` at 4x needs `rt_format` for RT 0
  as well as the extras: `rt_fmt` already serves both.
- Depth and stencil renderbuffers at 4x: `ZsBuf` already has an `msaa` flag
  keyed on `target.msaa`, loaded and stored `ALL_SAMPLES` (`zs_decimate`).
  What changes is only who owns the buffer: an FBO's depth renderbuffer is
  identified by `ds_id` while a window's follows the surface. Check `ds_persist`
  and the `used`/eviction logic with an FBO that outlives a job (host test).
- `rt_find`/render images are skipped at 4x already (`!msaa`).
- Invalid-after-swap (`MsBuf::valid`): for an FBO, the samples are
  undefined after `glInvalidateFramebuffer` and after a re-specification of
  the renderbuffer; the GL core already bumps `storage_gen`
  (`context::respecified`), which the back end must turn into "drop the
  sample buffer" (a changed `(w, h, format)` already frees it in `ms_buf`;
  a same-size respecification does not).
- Clear at 4x: the TLB clear writes all four samples; the scissored/masked
  quad draws at 4x as any draw. A several-RT 4x clear takes the same path as
  the non-4x several-RT clear (item `v3d-mrt-clear`): do the two in that
  order.

Host tests (V3D side, no GL core needed, with a hand-made `GlesDraw`
`samples = 4`): the `v3d_job` self-test that records the command lists and
asserts the load/store words (`driver/src/v3d/` tests, and the
`backend_tests.rs` job-level tests which already pin the 4x RGBA8 case); a
4x job with two RTs (RGBA8 + sRGB) pinning `multisample_mode_4x`, the tile
size 16x16 (`v3d_frame_tiling_bpp(.., rts 2, msaa, bpp 1)`: index
1+2+1 = 4), the load, the kept store and the resolve store of each RT, and
the resolve decimate 4X.

## (4) What the GL core needs (the GL-core part)

All of it is `src/gl` and `src/backend.rs`, which this note's item leaves
read-only; it is the second issue draft.

- A backend capability `max_samples` (0 = single-sampled only) in the
  `YakoglBackend` table (grep `tests/` for `YakoglBackend {` literals when
  adding a field, CLAUDE.md), 4 for the V3D once (3) is built, absent for the
  PowerVR and GC7000UL. `GL_MAX_SAMPLES` (`state.rs`) and the
  `glRenderbufferStorageMultisample` limit read it. Both ES 3.0 builds on
  other GPUs keep 1.
- `glRenderbufferStorageMultisample`: accept `1 < samples <= max`, round up to
  4 (ES 3.0 4.4.2.1: the count is "at least" the request), store 4 in
  `rb.samples`; the format must be multisample-renderable (colour formats of
  (2) and the depth/stencil ones; not the integer ones, not 32F/R11F). Allocate
  the resolved storage as today (`rb.mem`); the back end owns the samples.
- `GL_SAMPLES` of `glGetInternalformativ` (`NUM_SAMPLE_COUNTS` 1, `SAMPLES`
  4 for those formats, 0 counts for integer ones) and of a framebuffer
  (`framebuffer_samples`) follow.
- `attached_samples`/`check_status`: the rule exists
  (`INCOMPLETE_MULTISAMPLE`); with real counts the depth/stencil attachments
  must agree with the colour ones, and a texture mixed with a 4x renderbuffer
  stays incomplete.
- `GlesDraw.samples` = `framebuffer_samples` (2 places, 3382 and 2459 region
  of `draw.rs`); the sample-coverage / alpha-to-coverage code (`draw.rs`
  `sample_coverage_mask`, :1453 and :2109 region) then applies to FBOs through
  the same field. The clear builders set `samples` too.
- Resolve: `glBlitFramebuffer` read from a 4x FBO to a single-sample one. The
  CPU copy is correct *if the 4x job has stored the resolved image into
  `rb.mem`* before the blit: it needs a flush of the pending job of the read
  renderbuffer (the same mechanism `glReadPixels` uses) and the "resolved
  pixels are the content" rule above. `fb_samples` already makes a 4x draw
  framebuffer `GL_INVALID_OPERATION`. Not needed: a blit *into* a 4x
  framebuffer (the spec refuses it).
- `glInvalidateFramebuffer` on a 4x FBO drops the back end's samples
  (new back-end entry or a bump of `storage_gen`).
- A gate: with `max_samples = 0` nothing changes for the PowerVR and
  GC7000UL, so the GL core part can land and be gated per GPU. Host test:
  `GlesDraw.samples == 4` for an FBO on a back end with the capability, `1`
  on one without.

The default framebuffer's multisample rows (`multisample.default_framebuffer`)
are the EGL config, not these two parts (see (5)).

## (5) dEQP rows each part should move

From `docs/es3/pi4-gles3-fails.tsv` (read only; counts by status):

| group | rows | now | needs |
|---|---|---|---|
| `fbo.msaa.2_samples.*` | 22 | 17 Fail, 5 NotSupported (r11f_g11f_b10f, r32f, rg32f, rgba16f, rgba32f) | GL core (4), V3D (3): 17 can pass; the 5 float ones need the 16F/32F rows and EXT_color_buffer_float, 32F stay NotSupported |
| `fbo.msaa.4_samples.*` | 22 | the same 17 + 5 | the same |
| `fbo.msaa.8_samples.*` | 22 | NotSupported | none: `max_samples = 4` keeps them NotSupported |
| `fbo.completeness.samples.*` | 7 | NotSupported | GL core only (the cases need a count above 1) |
| `fbo.invalidate.{whole,sub}.unbind_blit_msaa_*` | 8 | Fail | GL core + V3D (4x draw, resolve blit, invalidate); depth/stencil variants need 4x Z/S |
| `multisample.fbo_4_samples.*` | 16 | NotSupported | GL core + V3D (`GL_MAX_SAMPLES >= 4` makes them run; sample-coverage, alpha-to-coverage and edge cases are only as good as the V3D's 4x draw) |
| `multisample.fbo_8_samples.*` | 16 | NotSupported | none |
| `multisample.fbo_max_samples.*` | 4 | Fail | GL core + V3D (they run at 4 samples; today at 1) |
| `rasterization.fbo.rbo_multisample_4.*` | 12 | NotSupported | GL core + V3D |
| `rasterization.fbo.rbo_multisample_max.*` | 2 (`lines_wide`) | Fail | GL core + V3D |
| `state_query.integers.max_samples_getfloat/_getinteger64` | 2 | Fail | GL core alone (a `GL_MAX_SAMPLES` the cases accept; they read a value the spec requires >= 4 for ES 3.0 *in the dEQP check*: re-measure) |
| `negative_api.buffer.blit_framebuffer_multisample` | 1 | Fail | GL core (it needs a 4x-capable error path); re-measure |
| `multisample.default_framebuffer.*` | 16 | NotSupported | neither part: the M1 image's EGL config has no 4x (see `msaa_on`, `v3d_debug_msaa`); a config switch of the dEQP image, plan #320's |

Totals, counting only rows that can plausibly pass: 34 `fbo.msaa` 2- and
4-sample Fail rows (some, the float and depth/stencil ones, depend on the
sub-items above), 7 completeness, 8 invalidate, 20 `multisample.fbo_*`
(4 and max), 14 `rasterization.fbo.rbo_multisample_*`: about 83 rows move
with both parts, 2 to 3 (`max_samples_*`, maybe `blit_framebuffer_multisample`)
with the GL-core part alone. The 10 float `NotSupported` rows of `fbo.msaa`
and the 8-sample and default-framebuffer rows do not move. Why the 2- and
4-sample cases are `Fail` while the 8-sample ones are `NotSupported` is read
from the status, not measured: the requests above `GL_MAX_SAMPLES` 1 get
`GL_INVALID_OPERATION`; the 8-sample cases check the limit first. A board
run (plan #320) settles it.

Order inside the V3D part: (a) the several-RT clear (`v3d-mrt-clear`), (b)
the sRGB and RGB10A2 render rows (the `v3d-srgb-draw` and `v3d-rgb10a2-side`
items; `extra_targets` still refuses an sRGB attachment past RT 0), (c) the
job builder's per-RT sample buffers and the two removed refusals. The GL-core
part comes last and is the switch: with `max_samples = 0` it is a no-op, so
the V3D part can be reviewed and tested alone and the GL core flipped in one
small commit.

## Open points (to settle before the code)

- Size cap for a multisample renderbuffer: the sample buffer is 4x the
  surface per RT; 2048x2048 four-RT 64-bpp is 512 MiB. A cap (refuse to
  report `GL_MAX_SAMPLES` per size, or fail `glRenderbufferStorageMultisample`
  with `GL_OUT_OF_MEMORY`) is a GL-core decision; the back end falls back to
  "samples not kept" (`ms_buf` returns `None`) when the allocation fails, which
  is correct only if the GL core never loads from it (an FBO that is drawn
  into across jobs without clearing).
- Whether 2 samples are reported as 4 (Mesa does) or `GL_SAMPLES` stays the
  request (`GL_RENDERBUFFER_SAMPLES` must be at least the request; dEQP
  checks it).
- `glBlitFramebuffer` between a 4x read and a differently-sized draw is the
  spec's `INVALID_OPERATION`; a 4x to 4x blit is also refused (draw
  framebuffer has sample buffers), so no sample-to-sample copy is needed.
- Depth/stencil renderbuffer multisample is in `fbo.msaa` (6 rows each of 2
  and 4 samples); the `ZsBuf` side exists for the window, the FBO side
  (ownership, persistence across jobs) needs its own test.
