<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The PowerVR fence wait: where a swap spends its time

Status: finished record, moved here from `driver/docs/pvr-fence-wait.md` on 2026-10-11. Measurements of 2026-10-03 and 10-04 on the AM62P. The defaults they led to (the 1 ms poll, the swap dropping depth/stencil) and the open knobs are driver/docs/pvr-render.md.

Written 2026-10-03 from the code at 5398fb14 (line numbers are that commit's) and
the glmark2-es2 run on the AM62P of 2026-10-03 23:09 (image 9d4f63c8, U-Boot
netboot; `build/glmark2-am62p-20261003-230937/.../console.ts` in the main
checkout; `driver/docs/glmark2.md` section 5c on branch `bench/glmark2`).

## What was measured

Six scenes with different fragment cost (conditionals with 0 steps, both
loop scenes, function low, ...) all print `swap 2016` or `swap 2017` us per
frame (`gl` 27-28, `show` 3, `frame` 2048-2049). effect2d (3x3) prints 7,016.
Other scenes print averages that are not near a whole millisecond (texture
2,240-2,258, bump normals 2,486, effect2d 5x5 10,910): those are a mix of
waits of different whole-millisecond lengths. The run has no GPU time per
kick ("gpu job times: absent").

## Where the swap waits

- `eglSwapBuffers` submits frame N, then waits for frame N-1
  (`src/gl/egl.rs:2059-2064`: `backend::flush()`, `backend::finish_previous()`).
- `pvr_backend_finish_previous` -> `wait_fence(last_fence - 1)`
  (`src/pvr/backend.rs:5993`, `:1666`). If the fence has already completed it
  returns without a wait.
- The OSAL: `z_wait` (`src/osal/zephyr.c:403`) -> `pvr_render_wait`
  (`driver/src/pvr_render.c:608`) -> `yd_pvr_render_wait` -> `wait_for`
  (`driver/rust/src/gpu/pvr/render.rs:1728`).
- `wait_for` loops once per millisecond (`render.rs:1734`): it reads the
  fragment UFO (the word the firmware writes with the UPDATE command queued
  after the 3D command, `render.rs:1294`), serves the FWCCB, checks the HWR
  counter, and then calls `busy_wait_us(1000)` (`render.rs:1827`). That hook is
  `k_busy_wait` (`driver/src/pvr_fw_boot.c:109-113`): the CPU spins.

So a wait ends on the first whole millisecond (from the start of the wait) at
or after the fence write, plus the cost of the reads (about 16 us here:
2,016 = 2 x 1,000 + 16, and 7,016 = 7 x 1,000 + 16).

What the six scenes say: frame N-1's fence was written between 1 and 2 ms
after the wait for it began, every frame. They do not say where in that window.
If the GPU had finished N-1 in less than 1 ms, `wait_fence` would often
return at once and the swap would be far below 2 ms. The finer poll below
measures where in the window the write happens.

The same 1 ms loop serves every other wait of the render service on the frame
path, all through `wait_for`: the geometry UFO ring throttle, "HWRT member in
flight", and "HWRT pair recycle".

## Interrupts: what exists today

- The device tree has the GPU's line: `interrupts = <GIC_SPI 241 IRQ_TYPE_LEVEL
  IRQ_DEFAULT_PRIORITY>` (`driver/dts/arm64/ti/ti_am62p54_a53.dtsi:156`). The
  source is TI's kernel device tree, quoted in
  `ab43a20c5:tools/cts/log/investigations/pvr-bringup/am62p-desk-research.md` section 1 (`GIC_SPI 241
  IRQ_TYPE_LEVEL_HIGH`).
- Nothing connects it. `IRQ_CONNECT` appears in `src/osal/zephyr_v3d.c`
  (the V3D) and in the shims `driver/drivers/gpu/gpu_vivante_gc_rust.c` and the
  display drivers. The PowerVR driver has no ISR.
- The firmware's OS0 event is acknowledged once, after the firmware boots
  (`driver/src/pvr_fw_boot.c:671-674`). The Rust side has the same ack,
  `rgxlayer::ack_irq` (`driver/rust/src/gpu/pvr/rgxlayer.rs:360`, the DDK's
  `RGXAckIrqDedicated`): `RGX_CR_IRQ_OS0_EVENT_STATUS` 0x0BD8 and
  `RGX_CR_IRQ_OS0_EVENT_CLEAR` 0x0BE8, bit 0 `SOURCE_EN` (`rgxlayer.rs:140-143`,
  from the DDK's `rgx_cr_defs_km.h`).
- What the firmware raises on a completion: the mainline Linux driver
  (drm/imagination, GPL-2.0, read for facts only) takes one interrupt line for
  the firmware. On each interrupt its threaded handler processes the FWCCB,
  wakes the KCCB waiters and processes the active queues, which signals the
  job fences (`drivers/gpu/drm/imagination/pvr_device.c`,
  `pvr_device_irq_thread_handler`, about lines 210-230). For a RISC-V
  firmware core, pending and clear are `ROGUE_CR_IRQ_OS0_EVENT_STATUS` /
  `_CLEAR` with `SOURCE_EN` (`pvr_fw_riscv.c`, about lines 108-116). It
  writes no GPU-side enable register; it clears pending events before it
  requests the line (`pvr_device.c`, about line 271). Read 2026-10-03 from
  torvalds/linux master.
- Not known: that driver runs the mainline firmware. We run the TI/Img DDK
  25.2 firmware. Nobody has measured whether that firmware raises the OS0
  event after every fragment UFO write on this board.

## The options and the floor each gives

| | The wait ends at | CPU during the wait | Status |
|---|---|---|---|
| today | fence write rounded up to the next whole ms from the start of the wait, + ~16 us | spins (`k_busy_wait`) | measured (above) |
| (a) finer poll, 20 us | fence write + at most 20 us + one 4-byte invalidate and read per step | spins | knob `pvr-fine-wait` / `CONFIG_SAFEUI_GPU_PVR_FINE_WAIT`, default off |
| (b) interrupt | fence write + interrupt latency (GIC, Zephyr ISR, `k_sem` wake-up; not measured on this board) | sleeps | not built (below) |

Prediction for (a), to be checked on the board: the six scenes at 2,016-2,017
us drop to the real completion time, somewhere in 1,000-2,000 us. They stop
being equal to each other, and no scene's average sits on a whole millisecond
+ 16 us any more. If they still sit at about 2,0xx us, the 2 ms floor is the
GPU or the firmware (the kick path), not the poll, and the item moves there.
The timeout is unchanged: the loop still counts whole milliseconds, and the
host test pins that a fence that never moves times out after exactly
`timeout_ms` of busy wait.

(b) over (a): at most 20 us less per wait (about 1% of a 2 ms frame). Its
real gain is that the CPU sleeps instead of spinning. It also needs a
periodic wake (for example `k_sem_take` with a 1 ms timeout), because the
FWCCB service, the re-post and the HWR check must still run during a long
wait.

## What (b) needs before it can be built

1. The hookup. `IRQ_CONNECT` is a Zephyr C macro; the crate has no ISR hookup
   of its own (`driver/rust/Cargo.toml`, header). Our pattern is a thin C shim
   that connects a Rust ISR (`gpu_vivante_gc_rust.c`). For the PowerVR, that is
   an edit of `driver/src/pvr_bringup.c` (connect and enable SPI 241, a
   `k_sem`), plus a new hook in `struct yd_pvr_km_ops` for "wait for the
   event, with a timeout". The ISR body in Rust is `ack_irq` + give.
2. The measurement: does the DDK 25.2 firmware raise OS0 after each fragment
   UFO write? A probe answers it: clear the event before the kick, then read
   0x0BD8 when the wait sees the fence. Count "event with completion" against
   "completion without event" over a gl_test run. The clear is a register
   write, so the probe ships opt-in, like this knob.

Not done here: (1) adds C, and (2) is a board measurement.

## The fine-wait run, and the floor it left (2026-10-04)

AM62P from U-Boot, both arms differing only by
`CONFIG_SAFEUI_GPU_PVR_FINE_WAIT`, the lead's runs (rows and the per-scene
table: `tools/cts/RESULTS.md`, "AM62P: the PowerVR fence poll, 1 ms against
20 us").

- gl_test (images from 78c4c208): both arms print `gl_test: all: 1 of 38 cases
  failed (37 passed; 0 timed out; first failure case 48 at index 31)`, with
  no recovery, timeout or re-post line. Case 48 is the ES 3 case that fails
  on main too.
- glmark2 (images from e7906077): score 303 -> 316, 0 GPU resets in both. The
  six light scenes went from 2,016-2,017 us per swap to 1,909-1,933 us. The
  fine arm had one re-post (the firmware stall the 20 ms re-post works
  around). Images without the knob show it too, so one occurrence does not
  attribute it.

The reading: the quantisation is gone, but the six scenes fell by ~85-110
us, not by up to 1 ms. The poll explained ~100 us of the 2.0 ms. Frame N-1's
fence is written ~1.9 ms after the swap of N begins, in every light scene
alike. That remaining ~1.9 ms is the kick path or the GPU, and it is the
next item. The knob stays default off; it can land with these rows.

## The kick path (line numbers at the commit that adds this section)

A swap (`src/gl/egl.rs` `eglSwapBuffers`) does `backend::flush()` then
`backend::finish_previous()`:

1. `flush` (`src/pvr/backend.rs:5469`) closes the frame's streams, cleans the
   used part of each arena (`:5881-5883`) and calls the OSAL's submit (`:5924`)
   -> `z_submit` (`src/osal/zephyr.c:359`) -> `pvr_render_submit`
   (`driver/src/pvr_render.c:587`) -> `render::submit`
   (`driver/rust/src/gpu/pvr/render.rs:1193`).
2. `submit` picks the HWRT dataset (`hwrt_get`, `:573`; it can wait for the
   member's previous kick, "HWRT member in flight") and writes the geometry
   command plus an UPDATE of the geometry UFO (`:1233`). It writes the
   fragment command behind a FENCE on that geometry UFO (`:1299`) and an UPDATE
   of the fragment UFO (`:1304`).
3. The pre-kick MMU cache invalidate (`mmu_cache_sync`, `:969`) is posted only
   when a mapping changed (`mmu_gate`, on by default, `pvr_render.c:248`). Its
   acknowledgement is awaited in 50 us steps (`mmu_ack`, on, `:247`).
   Measured: after an idle GPU the acknowledgement comes ActivePMLatency
   minus ~1.5 ms late, 4.10 ms median at the 5 ms default (commit ff062a35,
   `records/am62p/pm-latency`).
4. One COMBINED_TA_3D_KICK is posted to the KCCB (`:1348`). `kccb_post` writes
   the slot, the write offset and rings the doorbell (MTS schedule and
   read-back, `ccb.rs:104-110`).
5. The firmware runs the geometry phase and writes the geometry UFO, then
   the fragment phase (it waits on that UFO), then writes the fragment UFO.
6. `finish_previous` waits for N-1's fragment UFO (`wait_for`, `:1762`).

What is measured about the parts:

- Post -> geometry fence 120-140 us, post -> fragment fence 220-230 us, for
  64x64 kicks posted to an idle GPU (`mc_probe_in_order`, 10 us poll,
  `records/am62p/pm-latency/mmucache_lines.txt`, arms with `mmu_ack=0`). There
  is no 800x600 measurement; the 600x600 probe arms in the same run never
  printed one.
- The CPU part of a swap: when the previous kick is already complete at the
  swap (scenes whose GL calls are long), the swap prints 143 us
  (build use-vbo=false), 41 (buffer), 33 (ideas), 10 (terrain). So flush +
  submit cost 10-150 us, and the rest of the light scenes' 1.9 ms is the wait.
- Fragment cost adds on top of the floor: conditionals with 5 fragment steps
  is +412 us over 0 steps (2,321 against 1,909), function medium +393. If the
  floor were a latency the GPU's work could hide under, a small extra shader
  cost would not show. It shows 1:1, so whatever takes the 1.9 ms runs in
  series with the fragment work.

The structure that fits: swap N posts N and waits for N-1, which was posted
one frame (~1.95 ms) earlier and completes ~1.9 ms into the wait. So
post -> fragment fence is ~3.8 ms, two kicks are in flight, and one kick
retires every ~1.9 ms. Something takes ~1.9 ms per 800x600 kick, serially.

## The instrument (feature `pvr-kick-times`)

`CONFIG_SAFEUI_GPU_PVR_KICK_TIMES=y` (default off) stamps each kick with the
ARM generic timer (`driver/rust/src/gpu/pvr/kick_times.rs`):

- submit entry;
- the MMU invalidate's span, and whether one was posted;
- the post;
- the first host read that saw the geometry UFO and the fragment UFO, each
  with the last read that missed it (the error bar);
- the first wait that asked for it.

The UFOs of every kick in flight are read at each of `wait_for`'s reads, so
the wait for N-1 also times N's geometry. `pvr_render_last_kick_us()` returns
the last completed kick. After each swap, glmark2 prints, per scene:
`pvr last completed kick per frame: submit, mmu (syncs), kick->geom err
(bounded of), kick->frag err (bounded of), frag->frag, wait`. It prints
"absent" with the reason when the driver library was built without it or
when no kick completed. A value whose error bar is above 100 us is an upper
bound, and the line counts how many were bounded.

How to read it, for kick N:

- `frag->frag` is the retire interval (F_N - F_{N-1}).
- `kick->frag - kick->geom` is F_N - G_N. If it is larger than `frag->frag`,
  N's geometry finished before N-1's fragment phase did: the geometry phase
  overlaps, and the fragment phases are serial. If it is smaller, N's
  geometry waited for N-1's fragment phase.
- `submit - mmu` is the host's own submit cost.

What it cannot split: inside the fragment phase, firmware time against GPU
work. That needs the firmware's own timestamps (the DDK's HWPerf events, or
the 3D start). This instrument does not read them.

## Prediction for the first run with it (glmark2, light scenes, fine wait on)

- `submit` 20-60 us, `mmu` 0-10 us with `syncs 0`: a steady scene maps
  nothing new. A sync per frame would cost ~4 ms (the measured ack delay),
  more than the whole 1.9 ms frame.
- `frag->frag` ~1,930 us, the frame period, bounded in nearly all frames.
- `wait` ~1,880 us; `kick->frag` ~3,800 us (two periods).
- `kick->geom` 100-300 us, bounded: the geometry phase of N runs while N-1's
  fragment phase does, so `kick->frag - kick->geom` (~3,500 us) is larger
  than `frag->frag`.
- So the split: firmware and kick latency up to the geometry fence ~0.1-0.3
  ms, overlapped. The ~1.9 ms floor is the fragment phase of one 800x600
  kick: GPU work, or firmware time inside that phase, which this instrument
  cannot separate.
- If instead `kick->geom` comes out near 1.9 ms, the firmware runs N's
  geometry only after N-1's fragment phase. The floor would then be one whole
  kick per frame, geometry included, and the item moves to the firmware's
  scheduling.

## The kick-times run: geometry is late, and the pipeline does not overlap as predicted (2026-10-04)

kt-fine (`263aa966`, e92a7f20, fine wait on, kick times on), AM62P from
U-Boot, the lead's run: score 316 (as without the instrument), 0 GPU resets.
Console: `build/fwkt-am62p-20261004-004719/kt-fine/` in the main checkout.
In the scenes below, every fence time is bounded within 100 us in all but at
most one frame per scene (high-poly, not in the table: three). us per frame, means over the frames that completed a
kick:

| scene | swap | submit | kick->geom | kick->frag | wait | frag->frag |
|---|---:|---:|---:|---:|---:|---:|
| conditionals 0:0 | 1,911 | 6 | 1,087 | 3,844 | 1,898 | 2,421 (*) |
| conditionals 0:5 | 1,911 | 6 | 1,088 | 3,856 | 1,898 | 1,954 |
| function low | 1,936 | 6 | 1,090 | 3,905 | 1,922 | 1,979 |
| loop false | 1,932 | 6 | 1,179 | 3,899 | 1,918 | 1,977 |
| texture nearest | 2,128 | 7 | 861 | 4,305 | 2,113 | 2,189 |
| use-vbo=true | 2,327 | 6 | 1,525 | 4,696 | 2,313 | 2,381 |
| conditionals 5:0 | 2,319 | 6 | 1,235 | 4,673 | 2,305 | 2,364 |
| loop uniform=true | 4,264 | 6 | 1,827 | 8,577 | 4,251 | 4,325 |
| terrain (CPU-bound) | 11 | 5 | 145 | 866 | 864 | 54,683 |

(*) An instrument defect, not a GPU time: the first frame of a scene measures
from the previous scene's last kick, across the scene's setup.
(2,421 - 1,954) x 5,069 frames is 2.4 s. The other scenes have short setups.
Fixed in the glmark2 reader in the commit after this one. Read frag->frag
from conditionals 0:5 onwards.

One re-post (`not moved at 20 ms, re-posted 1 kick(s)`) in the run, the
known firmware stall. `syncs` is 0-2 per scene and `mmu` 0: the MMU invalidate is not in the frame.
`submit` is 5-7 us (desktop shadow excepted, below).

### What it says

For the light scenes, kick N is posted at P. N-1's fragment fence is seen at
P + ~1.90 ms (`wait`), N's geometry fence at P + ~1.09 ms, and N's fragment
fence at P + ~3.85 ms = N-1's + ~1.95 ms (`frag->frag`). So:

- N's geometry phase overlaps N-1's fragment phase. It ends ~0.8 ms before
  N-1's fragment fence. But it ends ~1.1 ms after the post, where my
  prediction said 0.1-0.3 ms. The prediction failed on that number.
- One kick retires per ~1.95 ms, and the fragment phases are back to back.
  If a fragment phase starts when the previous one ends, one 800x600 light
  fragment phase takes ~1.95 ms, while the geometry of the next kick runs
  slowly beside it.
- Fragment cost still adds 1:1 (conditionals 5:0 +410 us frag->frag). The
  geometry time grows with the scene's fragment load too (texture 861,
  use-vbo 1,525, gouraud 1,930, cel 2,392, high-poly 3,858). For a trivial
  geometry load that is not the geometry's own work: the geometry phase is
  slowed or held while the fragment phase runs.

What it does not say: the alone cost of a light-scene kick. Terrain's kick
(145 / 866 us, GPU idle between kicks) is not that kick. Terrain posts ~6
kicks per frame (374 over 62), most into its 256x256 and 512x512
framebuffers, and the last one completed at the swap is not shown to be an
800x600 one. So "an 800x600 trivial kick costs ~0.9 ms alone" is not
measured.

### The firmware facts (DDK 25.2, read for the ABI; `~/pvr-lab/ti-img-rogue-driver-25.2`)

- One render context holds two firmware common contexts, one per data
  master (`RGXFWIF_FWRENDERCONTEXT.sTAContext`, `.s3DContext`,
  `include/rogue/rgx_fwif_km.h:454-455`), each with its own client CCB. The
  firmware schedules the geometry and fragment data masters independently.
  A 3D waits only for what its CCB fences.
- Our kicks carry no dependency across frames. The geometry CCB holds
  [GEOM][UPDATE geometry UFO = N]. The fragment CCB holds [FENCE geometry UFO
  = N][3D][UPDATE fragment UFO = N] (`render.rs` `submit`, at the GEOM and
  FRAG blocks). Nothing in kick N names N-1's fragment fence. So "submit N
  without a dependency on N-1" is what we already do; there is nothing to
  remove.
- The swap's wait for N-1 comes after N is posted (`egl.rs`: flush, then
  finish_previous). It cannot hold N's geometry.
- DM overlap: the host sets `RGXFWIF_INICFG_DISABLE_DM_OVERLAP` only on the
  `DisableDMOverlap` apphint (default 0, `config_kernel.h:60`), or when
  `SLC_SIZE_IN_KILOBYTES <= 2` (`rgxsrvinit.c:562-568`). This core has 16
  (`rgxconfig_km_36.V.104.796.h:94`). The flag's PDump text is "Disable DM
  overlap (except TA during SPM)" (`rgxfwutils.c:1536-1538`). Pipeline
  roadblocks need `PIPELINED_DATAMASTERS_VERSION`, which this core does not
  have (`rgxsrvinit.c:551-559`). `TRY_OVERLAPPING_DM_PIPELINES_EN` is never
  set by the host (only its PDump comment, `rgxfwutils.c:1511-1512`). Our
  flags are the DDK's: system 0x00020000 (context-switch profile medium,
  captured from a running DDK; `fwinit.rs`), OS 0x0F. So the firmware is
  told the same as under the DDK: overlap allowed.
- Not in the kernel source: what the firmware does inside the fragment phase
  (its own scheduling, any context store, the PM's work). That needs the
  firmware's timestamps (HWPerf, `RGXFWIF_INICFG_HWPERF_EN`), which we do not
  enable or read.

### The next experiment, one knob: one kick in flight

The cheapest discriminating knob is in the YakoGL backend, not the driver:
the swap waits for N itself instead of N-1 (`finish` instead of
`finish_previous` after the flush). Then every kick starts on an idle GPU,
and the kick-times line gives the alone cost of exactly the light-scene
800x600 kick. That is the number the terrain row cannot give. A second render
context only makes sense after it: it changes how two kicks share the
firmware, and it is worth building only if the alone cost is well below the
1.95 ms period.

Prediction for the light scenes with it: `kick->geom` 120-200 us (as the
64x64 probe and the idle terrain kick), `kick->frag` 0.8-1.2 ms, and the
frame ~1.0-1.3 ms, faster than the 1.94 ms of now. That would mean overlap
costs more than it saves: the geometry of N and the fragment phase of N-1
slow each other down. The next step would then be the firmware's
timestamps, or a second context.

The other outcome: if `kick->frag` alone is ~1.9 ms, one light 800x600
fragment phase is ~1.7 ms of GPU (or firmware) time on its own. Overlap is
then not the cost, and the floor is the fragment phase itself (its tile
count, the colour and depth stores).

### desktop shadow's submit of 2,038 us

The likely reading, from the code, not measured: `submit` includes
`hwrt_get`'s wait for the pair member's previous kick ("HWRT member in
flight", `render.rs` `hwrt_get`: two renders of one size may be in flight,
no more). desktop shadow posts ~3 kicks per frame (3,962 over 1,316), so the
third render of a size waits for the first. desktop blur (11 kicks) is
CPU-bound (gl 13,955 us), and its kicks complete before they are reused.

## One kick in flight: the fragment phase is the floor (2026-10-04)

`fw-glmark2-am62p-one` (`256969fe`, d2ba18b2: fine wait, kick times, and
`PVR_BACKEND_OPT_ONE_IN_FLIGHT=1`, printed at boot), AM62P from U-Boot, the
lead's run. Run dir: `build/fwone-am62p-20261004-012110/` in the main
checkout (named `-012115` in the lead's message). Score 288 (316 with
overlap), 0 GPU resets, one re-post (the known stall). Each kick alone on an
idle GPU, every fence bounded:

| scene | kick->geom | kick->frag | frame |
|---|---:|---:|---:|
| conditionals 0:0 | 249 | 2,081 | 2,123 |
| conditionals 0:5 | 266 | 2,094 | 2,136 |
| conditionals 5:0 | 249 | 2,528 | 2,570 |
| function low | 249 | 2,105 | 2,147 |
| loop false | 305 | 2,156 | 2,199 |
| texture nearest | 174 | 2,299 | 2,352 |

The prediction split. The geometry alone is 170-310 us (held). The fragment
fence comes 2.1-2.5 ms after the kick even alone, so the "else" branch holds:
the fragment phase of one light 800x600 kick is ~1.8-1.9 ms by itself
(kick->frag - kick->geom). Overlap was buying ~0.2 ms, not costing it.

## What the fragment phase spends ~1.85 ms on: the candidates, ranked

What one light-scene kick moves, from the state the backend emits (`flush`
and `frame_begin_with`, `src/pvr/backend.rs`). The window is 800x600 XRGB8888
with D24S8: "Surface Config: buf=32 r=8 g=8 b=8 a=8 depth=24 stencil=8" in
the console. glmark2 clears colour and depth every frame.

| traffic per kick | bytes | why |
|---|---:|---|
| colour load | 0 | the frame begins with a full glClear: `frame_loads_color = !bg_color` is false, no fbload quad |
| depth/stencil load | 1,945,600 (corrected, e4cb0903) | depth is cleared, so `load_depth` = false. But glmark2 clears colour and depth only (`canvas-generic.cpp:103`), so `init_stencil` is false and `load_stencil` is true: the ISP loads the packed buffer for the stencil every frame. This row said 0 until the test of e4cb0903 showed `SLOADEN` set. |
| colour store (PBE, end of tile) | 1,920,000 | 800 x 600 x 4 |
| depth/stencil store (ZLS) | 1,945,600 | `isp_zlsctl`: `zstoreen`, `sstoreen`, `forcezstore` always on; 50 x 38 ISP tiles (the extent aligned to 32-pixel ZLS tiles) x 256 px x 4 B |
| extra full-screen object | 480,000 px | `record_frame_quad` with `init_depth`: the depth/stencil-init quad, colour masked, the one-dword clear FS |

So a light kick moves 5.8 MB: 3.87 MB written and 1.95 MB read. Two thirds of it is the depth/stencil store and load. It is
dead: EGL 1.5 section 3.10.1 makes the window's depth/stencil undefined after
the swap. The V3D backend drops it (`v3d_surface_swapped`: `zs_discard`). The
PowerVR backend has no `surface_swapped` hook (`surface_swapped: None` in its
table). The init quad is there only so that the stored buffer is defined: its
comment says the ZLS store writes 0 for pixels no object covered (gl_test
case 13). Without the store, it has no job either.

Ranked:

1. **The depth/stencil store, plus the quad that exists for it.** Half the
   written bytes, and one of the two full-screen objects per frame. 3.87 MB
   in 1.83 ms is 2.1 GB/s. If the fragment phase is write-bandwidth bound
   (not measured on this board), the store is ~0.9 ms of it.
2. **Per-tile and per-object ISP work.** 1,900 ISP tiles, ~0.96 us per tile
   at 1.83 ms. Each tile processes two full-screen objects (the init quad and
   the scene). This cannot be separated from (1) without a knob, and the knob
   for (1) removes part of it.
3. **The fragment shader.** Alone, conditionals with 5 fragment steps is +447
   us over 0 steps (2,528 against 2,081). The 0-step shader is part of the
   ~1.83 ms, but the steps show a full-screen shader costs hundreds of us,
   not the whole phase.
4. **Firmware per-kick fragment overhead.** Bounded small: a 64x64 kick from
   idle goes post -> fragment fence in 220-230 us, geometry -> fragment ~100 us
   (`records/am62p/pm-latency`). That includes the fragment DM kick, the
   end-of-tile program and the PBE flush. The 1.83 ms grows with the target
   area, not per kick. HWPerf would give the exact share; it is not needed to
   rank this last.

### The knob for (1): the swap drops the window's depth/stencil

A `surface_swapped` hook for the PowerVR backend, opt-in
(`PVR_BACKEND_OPT_SWAP_DROPS_ZS`), as the V3D's: when the frame being swapped
targets the window surface, its kick does not store depth/stencil (`zstoreen`,
`sstoreen`, `forcezstore` off), and the buffer is marked not valid, so the
next frame initialises it instead of loading it. Conformant (EGL 1.5 3.10.1).
It does not remove the init quad: the quad is recorded at the frame's first
draw, before the swap is known. A second knob would skip it for a target
whose previous frame was dropped.

Prediction, glmark2 light scenes:

- With one kick in flight: kick->frag 2.08 -> 1.2-1.4 ms.
- With overlap (the default): the frame 1.94 -> 1.1-1.3 ms, score 316 ->
  ~400.
- That holds if the fragment phase is bound by its memory writes. If
  kick->frag moves by less than 0.1 ms, memory is not the limit. Then the
  per-object work (2) is next, tested by the second knob.
- gl_test must stay 37/38: no case reads depth after a swap. The frames the
  GL core splits are not swaps and keep their store.

## The verdict: memory traffic bounds the fragment phase (2026-10-04)

e4cb0903's four images, AM62P from U-Boot, the lead's runs. Run dir:
`build/fwzs-am62p-20261004-014255/` in the main checkout (`-013800` in the
lead's message). Rows and the per-scene table: `tools/cts/RESULTS.md`, "AM62P:
the swap drops the window's depth/stencil".

- gl_test: 37/38 in both arms, word for word (case 48, the ES 3 case).
- glmark2 score 315 (off) -> 550 (on). The light scenes go from 1.95-2.36 ms
  per frame to 0.88-1.29 ms. conditionals 0:0 goes 1,953 -> 879 us, texture
  nearest 2,178 -> 919. With the knob, frag->frag equals the frame (879, 919),
  so the fragment phase of a light kick is ~0.88 ms, down from ~1.95.
  kick->geom also falls (1,099 -> 640 us in conditionals 0:0): the geometry
  phase overlaps a shorter fragment phase.
- The prediction held with room: frame 1.94 -> 0.9-1.3 ms predicted, 0.88-1.29
  measured; score 400+ predicted, 550 measured.

A two-point fit, cost = bytes / bandwidth + fixed, over the light kick before
(5.8 MB, ~1.83 ms) and after (1.92 MB, the colour store only, ~0.88 ms): ~4.1
GB/s and ~0.41 ms fixed per kick. Two points determine a line exactly: this is
a reading, not a measurement of either term. The ~0.41 ms holds the init quad,
the scene's ISP and shader work, and the firmware's fragment overhead (the
64x64 probe bounds that one at ~0.1 ms).

## The default: what could tell the difference before the flip

The spec, EGL 1.5 section 3.10.1 (read from the registry's PDF, 2026-10-04):
"The contents of ancillary buffers are always undefined after calling
eglSwapBuffers. The contents of the color buffer are undefined if the value of
the EGL_SWAP_BEHAVIOR attribute of surface is not EGL_BUFFER_PRESERVED. ...
EGL_SWAP_BEHAVIOR applies only to the color buffer. EGL 1.5 has no way to
specify or query whether or not ancillary buffers are preserved, and
applications should not rely on this behavior." The 2010-03-03 change log:
"Relax language in section 3.10.1 to allow ancillary buffer contents to be
undefined after swap, regardless of the value of EGL_SWAP_BEHAVIOR". So
`EGL_BUFFER_PRESERVED`, which our surfaces report by default, does not oblige
us to keep depth/stencil. That corrects the premise in the request.

The same paragraph: "If surface is a single-buffered window, pixmap, or
pbuffer surface, eglSwapBuffers has no effect." Our `eglSwapBuffers` calls
`surface_swapped` for every surface, pbuffers included (`src/gl/egl.rs`), and
the V3D backend already drops a pbuffer's depth/stencil at a swap. "Always
undefined" allows that; "has no effect" reads against it. The conservative
default is window surfaces only. That is a change in `egl.rs` (pass the
surface type, or call the hook for windows only), and it moves the V3D too.
It is a decision for the lead, not made here.

What could tell the difference:

- dEQP-GLES2, as run here: nothing. dEQP swaps only window surfaces
  (`framework/egl/egluGLContextFactory.cpp:540`, `RenderContext::postIterate`:
  `if (m_window)`), and our runs use `--deqp-surface-type=pbuffer`
  (`driver/samples/zephyr/deqp/src/main.cpp:134`, `tools/cts/cts-run.sh`).
  No dEQP-GLES2 case calls eglSwapBuffers in our configuration. A green
  dEQP-GLES2 run says nothing about this knob either way.
- dEQP-EGL: of the modules that swap (swap_buffers, buffer_age,
  preserving_swap, partial_update, swap_buffers_with_damage, native color and
  coord mapping, wide_color, get_frame_timestamps), none enables a depth or
  stencil test (a grep of `modules/egl/*.cpp` for `GL_DEPTH_TEST` /
  `GL_STENCIL_TEST`; the search finds them in teglRenderTests.cpp, which uses
  glFinish, not a swap). They need a window surface, which YakoGL's EGL does
  not offer the dEQP platform.
- What does exercise it: gl_test cases 15-18 (pbuffer swaps) and 21 (window
  swap). All read colour only. Our own frames that are flushed without a swap
  (GL-split frames, glFinish) keep their store (e4cb0903's test, kick 2).
- What is missing, and should land with the flip: a gl_test case with pixel
  read-back that draws with a depth test into a window across a swap without
  clearing depth. Undefined contents cannot be asserted, so it asserts our
  defined behaviour: after a dropped swap, the next frame starts from the
  clear values, not from garbage. Plus the same in a split frame, which must
  keep its depth.


## The default (the lead's decision, 2026-10-04)

- `eglSwapBuffers` tells the backend of window swaps only (fcaa9f39). A
  pbuffer keeps its depth/stencil across a swap, on the V3D too.
- gl_test cases 172 `swapzs` and 173 `splitzs` (b25c9ebc, renumbered from 170/171) assert each
  backend's behaviour across a window swap, and that a split frame keeps
  its depth.
- `PVR_BACKEND_OPT_SWAP_DROPS_ZS` is on by default (`pvr_swap_drops_zs`,
  `PVR_SWAP_DROPS_ZS_DEFAULT` = 1). The opt-outs are
  `CONFIG_GLMARK2_PVR_KEEP_ZS` and `CONFIG_GL_TEST_PVR_KEEP_ZS`.
- `PVR_BACKEND_OPT_SKIP_INIT_QUAD` stayed opt-in until its measurement, then was removed (below).

## The init-quad knob: nearly free, and its condition was unsound (2026-10-04)

10d9b083's pair, AM62P from U-Boot, the lead's run. Run dir:
`build/fwquad-am62p-20261004-022645/` in the main checkout (`-022649` in the
lead's message).
- zs2 (`b5218d6f`, the same-commit control): score 550.
- quad (`762cf0c4`, + `PVR_BACKEND_OPT_SKIP_INIT_QUAD`): score 556.
- Both "29 scenes ran, 2 failed, 2 unsupported", one re-post each, env diff 0.

Per scene, us per frame (both consoles, the same 29 scenes in the same order):

| scene | frame zs2 | frame quad | change |
|---|---:|---:|---:|
| [build] use-vbo=false | 9886 | 9867 | -19 |
| [build] use-vbo=true | 1050 | 1033 | -17 |
| [texture] texture-filter=nearest | 919 | 910 | -9 |
| [texture] texture-filter=linear | 936 | 928 | -8 |
| [texture] texture-filter=mipmap | 1056 | 1040 | -16 |
| [shading] shading=gouraud | 1274 | 1261 | -13 |
| [shading] shading=blinn-phong-inf | 1683 | 1653 | -30 |
| [shading] shading=phong | 2173 | 2142 | -31 |
| [shading] shading=cel | 2473 | 2448 | -25 |
| [bump] bump-render=high-poly | 3129 | 3089 | -40 |
| [bump] bump-render=normals | 1224 | 1215 | -9 |
| [bump] bump-render=height | 1451 | 1440 | -11 |
| [effect2d] kernel=0,1,0;1,-4,1;0,1,0; | 6747 | 6746 | -1 |
| [effect2d] kernel=1,1,1,1,1;1,1,1,1,1;1,1,1,1,1; | 10733 | 10732 | -1 |
| [pulsar] light=false:quads=5:texture=false | 1163 | 1136 | -27 |
| [desktop] blur-radius=5:effect=blur:passes=1:separable=true:windows=4 | 17940 | 17940 | +0 |
| [desktop] effect=shadow:windows=4 | 6368 | 6366 | -2 |
| [buffer] columns=200:interleave=false:update-dispersion=0.9:update-fraction=0.5:update-method=subdata | 16836 | 16781 | -55 |
| [ideas] speed=duration | 4241 | 4293 | +52 |
| [jellyfish] <default> | 7039 | 7030 | -9 |
| [terrain] <default> | 162926 | 162563 | -363 |
| [conditionals] fragment-steps=0:vertex-steps=0 | 879 | 862 | -17 |
| [conditionals] fragment-steps=5:vertex-steps=0 | 1783 | 1766 | -17 |
| [conditionals] fragment-steps=0:vertex-steps=5 | 891 | 874 | -17 |
| [function] fragment-complexity=low:fragment-steps=5 | 1223 | 1203 | -20 |
| [function] fragment-complexity=medium:fragment-steps=5 | 1768 | 1751 | -17 |
| [loop] fragment-loop=false:fragment-steps=5:vertex-steps=5 | 1289 | 1293 | +4 |
| [loop] fragment-steps=5:fragment-uniform=false:vertex-steps=5 | 1289 | 1294 | +5 |
| [loop] fragment-steps=5:fragment-uniform=true:vertex-steps=5 | 3818 | 3804 | -14 |

The light scenes gain 8-40 us (conditionals 0:0 879 -> 862). A full-screen,
colour-masked depth/stencil object costs ~2% of a light frame: the
prediction's "under 0.03 ms" branch. The score moves +6 (550 -> 556), not to
580-650.

The hazard fired: `I: pvr_backend: init quad skipped, but the frame was stored
without a swap: its uncovered depth/stencil hold 0` (02:36:25, right after
pulsar's frame line, at the start of desktop blur). The frame:
- glmark2's main loop clears the window (`canvas-generic.cpp:103`, colour
  and depth). Our GL core records that as a pending clear.
- `SceneDesktop::draw` (`scene-desktop.cpp:976`) then binds its desktop FBO
  (`priv_->desktop.clear()`).
- The bind submits the window's pending clear as a window kick of its own,
  without a swap. That kick stores.
- The knob had skipped the quad when the frame began, because the previous
  swap (pulsar's last) had dropped the buffer.

The line prints once per run, but the condition holds in every desktop
frame, so it likely fired there every frame. The counter
(`init_quad_skipped_stored`) was never printed, so how often is not
measured.

This is a defect of the knob's condition, not a cost. Whether a frame ends in
a swap is known only at its flush, and the quad is recorded at its first
draw. No condition on the frame's start is right for an application that
changes target mid-frame. A correct form would decide at flush time (patch
the quad out of the recorded stream), and ~2% does not pay for it.

Decision (the lead's reading, agreed): removed, f6897390 (a revert of
10d9b083).

## The ~0.41 ms fixed per light kick: what it is, and how to tell the parts apart

What is measured:
- With the window's depth/stencil gone, a light kick's fragment phase is
  ~0.88 ms. That is the colour store (1.92 MB), the init quad (8-40 us, above),
  the scene's ISP and shading, and the firmware.
- The fragment shader is now in plain view. conditionals 5:0 against 0:0 is
  +904 us (1,783 against 879). With the depth/stencil traffic still there,
  the same pair differed by +412 (2,321 against 1,909): the shading then
  overlapped the memory traffic.
- A 64x64 kick from idle goes geometry -> fragment fence in ~100 us
  (`records/am62p/pm-latency`). That bounds the firmware's per-kick fragment
  work, plus a 16-tile walk, at ~0.1 ms.
- The two-point fit's ~0.41 ms is a reading of a line through two points, not
  a measurement of either term.

The candidates, by what they scale with:
- **Firmware per kick** (the 3D kick, the end-of-render, the UFO update):
  fixed per kick, whatever the size.
- **The ISP's per-tile walk** (1,900 ISP tiles at 800x600, with the scene's
  objects and the background object): scales with the tile count, not with
  bytes.
- **The PBE store**: scales with bytes written (pixels x bytes per pixel).
- **Shading**: scales with shaded pixels x the shader's cost (the steps
  above).

Telling them apart without new code: a window-size sweep of one light scene
with one kick in flight and kick times on. Use CONFIG_GLMARK2_SIZE 200x150,
400x300, 800x600 and 1600x900 (the board window allows up to 8,100 KiB),
`CONFIG_GLMARK2_ARGS="-b conditionals:fragment-steps=0:vertex-steps=0"`,
`CONFIG_GLMARK2_PVR_ONE_KICK_IN_FLIGHT=y`, `CONFIG_SAFEUI_GPU_PVR_KICK_TIMES=y`,
fine wait. kick->frag minus kick->geom against the tile count T gives:
- the intercept: firmware and per-kick fixed work;
- the slope: per tile, which is the walk plus the store plus shading.

Prediction: intercept ~0.1 ms (the 64x64 bound), slope ~0.4 us per tile (about
0.25 us of store at ~4.1 GB/s for 1 KiB per tile, the rest walk and shading).
So ~0.3 ms at 400x300 and ~2.4 ms at 1600x900.

Splitting the slope needs one knob: a measurement-only option that makes the
end-of-tile program emit no PBE write (the picture is wrong by design; it is
never a product setting). At a fixed size, its difference is the PBE store's
share, and the rest of the slope is the walk and shading. The firmware's own
split (when the 3D starts and ends inside the kick) needs its HWPerf
timestamps (`RGXFWIF_INICFG_HWPERF_EN` and the HWPerf buffer), a firmware-ABI
instrument; it is not needed for the sweep.

## The window-size sweep: ~0.12 ms per kick, ~0.37 us per tile (2026-10-04)

ae4d84e2's four images, AM62P from U-Boot, the lead's run. Run dir:
`build/sweep-am62p-20261004-025233/` in the main checkout (`-025237` in the
lead's message). One install, env diff 0, every window "shown yes" (the DSS
took 1600x900), "GPU resets 0" (read as absent at this tree).
`conditionals 0:0` alone (`-b conditionals:fragment-steps=0:vertex-steps=0`),
one kick in flight, kick times, fine wait. Every fence is bounded (err 20 us
in all frames).

| window | ISP tiles | kick->geom | kick->frag | fragment phase (frag - geom) | frag->frag | frame | score |
|---|---:|---:|---:|---:|---:|---:|---:|
| 200x150 | 130 | 203 | 357 | 154 | 398 | 403 | 2478 |
| 400x300 | 475 | 211 | 505 | 294 | 546 | 552 | 1808 |
| 800x600 | 1,900 | 249 | 1,086 | 837 | 1,127 | 1,138 | 877 |
| 1600x900 | 5,700 | 369 | 2,568 | 2,199 | 2,610 | 2,635 | 379 |

Least squares over the four points (my fit):

- **Fragment phase = 121 us + 0.366 us per tile** (R^2 0.9997). Residuals
  -14, 0, +21, -7 us: a slight bend, so the line is a good summary, not a law.
  The first three points alone give 107 us + 0.385 us per tile.
- **Geometry phase (kick->geom) = 196 us + 0.030 us per tile** (R^2 0.998).
  It grows with the window, not only with the scene: 203 -> 369 us for the
  same geometry. Likely the TA's per-tile region work: the region headers
  and tail pointers of the larger tile grid (the HWRT `rgn` and macrotile
  sizes scale with the tiles). The 30 ns per tile is that reading, not
  measured apart.
- frag->frag - kick->frag is a constant ~41 us: the host's turn between two
  kicks (gl 27 + show 3 + submit, the wait's last poll). `submit` itself
  grows 8 -> 29 us with the window (the arena cleaning of larger per-frame
  streams), still small.

The prediction (0.1 ms + 0.4 us per tile: 150 / 290 / 860 / 2,380) held.
The slope is a little lower at the top (2,199 measured at 5,700 tiles).

The verdict:

- The fixed part of a light kick's fragment phase is ~0.1 ms: the firmware's
  per-kick work plus a small-target floor (the 64x64 probe's ~0.1 ms says
  the same).
- Everything else is per tile, at ~0.37 us. For a light scene, the window
  size is the lever a client has.
- At 800x600 both boards are now in the same place: the Pi 4's 842 and the
  AM62P's 877 (sweep image; 550 with all 29 scenes).
- The cluster's 1920x720 panel is 5,400 tiles: by the fit, ~2.1 ms of
  fragment phase per light kick, plus the geometry, so ~2.4 ms per frame
  before the scene's own cost.

## Splitting the 0.37 us per tile: the store's share

The proposed knob was a measurement-only end-of-tile program with no PBE
write. Not built: it needs a hardware fact we do not have, and guessing a
register is not done here.

- Mesa 26.2.1's `csbgen/rogue/pbestate.xml` has
  `PBESTATE_STATE_WORD1.emptytile` (bit 6; our `csbgen.rs`), but nothing in
  Mesa's `src/imagination` uses it, so what it does is not documented.
- The only other no-write forms would be an end-of-tile program with zero
  emits (our end-of-tile programs are captured, one per attachment count
  1..3, `pvr_render_static`), or a PBE state that points nowhere. Both are
  register guesses.
- The DDK's user-mode driver, which would show a no-emit end of tile, is not
  in the vendored kernel source.

The same split without a new register: an RGB565 window. At the same tile
count the PBE writes half the bytes (2 B a pixel; gl_test case 21 renders an
RGB565 window on all three GPUs, the PBE packing to 5/6/5).

It needs:
- the board window in RGB565 (`board_window`'s format is XRGB8888 today,
  `board_am62p.c`; a board Kconfig);
- glmark2 picking the 565 EGL config
  (`CONFIG_GLMARK2_ARGS` `--visual-config red=5:green=6:blue=5:alpha=0`);
- the DSS may refuse to show it ("shown no"), which does not change the GPU
  times.

Prediction at 800x600 with one kick in flight: the store is ~0.25 us per tile
at ~4.1 GB/s for 1 KiB, so 565 saves ~0.12 us per tile, and the fragment phase
goes 837 -> ~600 us. If it moves by less than ~50 us, the store is not where
the per-tile time goes, and the walk and shading are.

## RGB565 against XRGB8888: the store is a small share (2026-10-04)

196d4286's pair, AM62P from U-Boot, the lead's run. Run dir:
`build/fw565-am62p-20261004-030504/` in the main checkout (`-030509` in the
lead's message). One install, env diff 0. 800x600, conditionals 0:0 alone,
one kick in flight, kick times, fine wait.

| arm | window line | shown | kick->geom | kick->frag | fragment phase | frag->frag | frame | score |
|---|---|---|---:|---:|---:|---:|---:|---:|
| x8888 (`725009c0`) | `window 800x600, stride 3200, 4 bytes a pixel` | yes | 249 | 1,086 | 837 | 1,128 | 1,139 | 877 |
| r565 (`e26ccc94`) | `window 800x600, stride 1600, 2 bytes a pixel` | no, by design | 244 | 1,027 | 783 | 1,065 | 1,076 | 928 |

- The instrument check: the control reproduces the sweep's 800x600 point
  exactly (geometry 249, fragment phase 837, ae4d84e2's image 4 hours earlier).
- "shown no" on the 565 arm is the design (`board_show` returns -ENOTSUP for
  a 565 window: `board_show = -134 (the window is not on the screen)`). Its
  `show` cost is 0 us against 3. The fragment phase is GPU time and does not
  include it.

The verdict: halving the bytes the PBE writes saved 54 us of 837 (6.5%). That
is at the "under ~50 us" line, and far from the predicted ~600 us. The
prediction failed on the store's share.
- Bound: 960,000 B fewer saved 54 us, so at most ~0.028 us per tile for
  512 B. The whole 1 KiB colour store, if linear, is at most ~0.06 us per
  tile, ~108 us a frame.
- As a bandwidth, 1.92 MB in ~108 us would be ~18 GB/s, above the LPDDR4's
  peak. So the store mostly overlaps the work of later tiles. Its marginal
  cost is small, not its traffic.
- That also reads the earlier result differently. Dropping the
  depth/stencil saved ~1 ms for 3.9 MB, but half of that was a LOAD (the
  stencil). A ZLS load must land before the tile's ISP work starts; a PBE
  store is written behind it. The ~4.1 GB/s of the two-point fit was a
  mixed number, mostly the load's latency. It is not the store's bandwidth.
- What remains of the ~0.37 us per tile is ~0.3 us of tile walk, per-tile
  end-of-tile work and shading.

## Separating the remaining ~0.3 us per tile

What the frame is now (light glmark2 frame, defaults: the window's
depth/stencil dropped at the swap):
- the ISP background object (the clear);
- the colour-masked depth/stencil-init quad (the frame's depth was dropped,
  so it is initialised; ~17 us a frame, the init-quad pair);
- the scene's mesh (conditionals: SceneGrid's 32x32 grid, every pixel
  shaded by the conditionals fragment program);
- the end-of-tile program and the PBE store, once per tile.

Levers that need no register guess, on existing paths:

1. **No scene object: glmark2's `clear` scene** (`-b clear`). The pinned
   glmark2 registers `SceneClear` (`scene-collection.h:79`, compiled in our
   CMakeLists). It draws nothing of its own, so its frames are the main
   loop's glClear (colour and depth) and the swap. Same window, same tiles,
   same init quad and store; no scene mesh, no scene shading.
   - Fragment phase(clear) is the per-tile floor: walk, end-of-tile, store,
     init quad.
   - conditionals 0:0 minus clear is the scene object's ISP work plus its
     shading.
   - Prediction at 800x600, one kick in flight: ~0.25-0.40 ms
     (0.1 ms per kick + 0.08-0.15 us per tile), so the grid and its shading
     are ~450-600 us of the 837.
2. **Shading only: fragment steps at fixed geometry.** conditionals
   fragment-steps 0 and 5 alone at 800x600: same mesh, same objects, more
   ALU per pixel.
   - With overlap the pair differed by +904 us (879 against 1,783); one
     kick in flight gives the per-step cost cleanly (~180 us a step, ~0.09
     us per tile a step).
   - The 0-step program's own cost sits in (1)'s difference minus the
     mesh's ISP work. That cannot be split further without a "no shading"
     form of the mesh.
3. **Not a clean lever: a frame without glClear.** In our backend a frame
   that does not clear its colour records the colour-LOAD quad (fbload: a
   full-screen texture fetch of the previous frame). That adds a full-screen
   object and a 1.92 MB read; it removes nothing.

The objects-per-tile count itself was already varied once: removing the
init quad, one full-screen colour-masked object, saved 8-40 us a frame
(~0.009 us per tile at 1,900 tiles). So the ISP's per-object work for a
depth-only quad is small. The scene object's cost is mostly its shading,
which (1) and (2) measure.

## The object levers: clear alone, and the shading per step (2026-10-04)

ac1f8006's pair, AM62P from U-Boot, the lead's run (run dir
`build/fwobj-am62p-20261004-032735/`, as the lead named it). 800x600, one kick
in flight, kick times, fine wait, both shown, env diff 0.

| scene | kick->geom | kick->frag | fragment phase | frame | score |
|---|---:|---:|---:|---:|---:|
| `clear` (no scene object) | 170 | 665 | 495 | 697 | 1432 |
| conditionals 0:0 (the x8888 control) | 249 | 1,086 | 837 | 1,139 | 877 |
| conditionals 5:0 | 248 | 2,003 | 1,755 | 2,064 | 484 |

- The shading per step held: +918 us for 5 steps over 0, ~0.097 us per tile
  per step (predicted ~1.7 ms, measured 1.755).
- The clear-only floor failed: predicted 0.25-0.40 ms, measured 495 us
  (~0.26 us per tile, ~1 ns per pixel).
- So a frame with no scene object still costs most of a full-screen pass.

## Is there a cheaper clear? Mesa clears as we do

The request assumed our glClear is a full-screen quad through the pixel pipe
and Mesa's is shader-less. Read from the sources, neither holds:

- **Our colour clear is the ISP background object.** `frame_begin_with` takes
  a full glClear of colour as `bg_color` (no quad). The background object is
  `pds_bgnd` (`CR_PDS_BGRND0/1/3`): "clear FS kick + colour DMA", the clear
  values in `usc_clear_register` (`src/pvr/backend.rs`, flush).
- **Mesa's clear is the same mechanism.** A render pass's LOAD_OP_CLEAR is a
  `pvr_load_op`: a PDS program that DMAs the clear values to shared registers
  plus a USC fragment program (`usc_frag_prog_bo`, `vulkan/pvr_mrt.h`),
  installed as the background object (`pvr_pds_bgnd_pack_state`,
  `vulkan/pvr_arch_cmd_buffer.c:998-1022`). Mesa's render path has no
  shader-less colour clear. (Mesa 26.2.1 full tree,
  `~/pvr-lab/mesa-build/mesa-26.2.1/src/imagination`, MIT.)
- So the proposed knob (emit the clear as the background object instead of a
  quad) is what we already do. The 837 -> ~440 us prediction has no mechanism
  behind it.

Where we do differ from Mesa, from the same sources:
- **`ISP_BGOBJVALS.enablebgtag`**: ours is `false` (`isp_bgobjvals`); Mesa's
  is `job->enable_bg_tag` (`pvr_arch_job_render.c:1126`), true whenever the
  render has a colour load op (`pvr_perform_start_of_render_clears`,
  `pvr_arch_cmd_buffer.c:3433`).
- **The depth/stencil-init quad**: we add it to every frame that initialises
  depth/stencil, because the ZLS store wrote 0 for pixels no object covered
  (gl_test case 13, silicon, with `enablebgtag` false). Mesa, for a
  full-screen tile-aligned single-layer render, clears depth/stencil with no
  quad. It sets `ISP_BGOBJDEPTH` to the clear value and stores
  (`d_store`/`s_store`, `pvr_arch_cmd_buffer.c:1849-1900`; "If this is
  single-layer fullscreen, we already do the clears in
  pvr_sub_cmd_gfx_job_init()", `:3356`).
- A reading, not measured: with the background tag on, the background object
  is a tagged object in every pixel, so its depth is what the ZLS stores where
  nothing else covered the pixel. That is why Mesa needs no quad, and why ours
  did with the tag off.

What that is worth:
- In the light glmark2 scenes the init quad is hidden behind the scene
  (deferred shading shades only what is visible): removing it saved 8-40 us
  (the init-quad pair). So this is not the lever for the 837 us.
- It is a correctness simplification with a measurable test: gl_test 13
  (zls), 172/173 and the set.
- In a clear-only frame the quad may be the visible full-screen object.
  How much of the 495 us it is cannot be split from the background object's
  own pass without the knob.

What the 837 us is, as far as measured (800x600, one kick in flight):
- ~0.1 ms per kick, fixed;
- at most ~0.06 us per tile for the colour store (the RGB565 pair);
- ~0.097 us per tile per fragment step;
- the clear-only frame at 495 us, which holds the fixed part, the
  background/init-quad pass, the end-of-tile work and the store;
- the scene's grid at 0 steps adds 342 us on top (837 - 495; deferred
  shading hides the background where the grid covers, so this is not simply
  additive).

The remaining question is what one full-screen pass costs per pixel on this
GPU (~1 ns, about 1 Gpx/s, where the BXS-4-64's nominal rate is 4 pixels a
clock at 800 MHz). Its own counters or the firmware's HWPerf would answer
that, and neither is read today.

Possible next knob (not built; your call):
- `ISP_BGOBJVALS.enablebgtag = 1` plus no depth/stencil-init quad for a
  full-screen depth/stencil clear, as Mesa does. A field Mesa sets on this
  core for this purpose, not a guess.
- Opt-in, judged first by gl_test 13 (the case that showed the 0 store),
  172, 173 and the set with read-back.
- Prediction: light scenes -10 to -40 us (the hidden quad). The clear-only
  scene drops by the init quad's visible pass, if the background object's
  pass is cheaper.
