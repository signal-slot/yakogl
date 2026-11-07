<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# AM62P: the transfer queue for the PowerVR mipmap chain (path A, a study)

Written 2026-11-07 from the code at main `aaf8e53e` (line numbers are that
commit's) and the vendored headers, read only. No code and no board run.
It answers the "path A" row of `am62p-pvr-mipmap-kicks.md` ("The paths",
"What decides it", "Open"): can a transfer-queue (TQ) job, or one transfer
submit carrying every level's blit, make a level of `glGenerateMipmap`
cheaper than the render kick it costs today?

**Verdict: needs the board experiment, and the code leans no-go.** On this
core a transfer job is a 3D data-master job like the fragment half of a kick,
so it can only save what the geometry half and its hand-over cost, and
nobody has measured that share. The experiment below measures it on the
existing render context, with no transfer context, and its record value
decides whether the transfer context is worth building.

## The cost being attacked

- glmark2 terrain on the AM62P: 83.06 ms a frame at `6a7021ec` with every
  level on the GPU (`tools/cts/RESULTS.md`, Common set; the path B rows of
  `am62p-pvr-mipmap-kicks.md`, "Measured"). Five chains a frame, one kick a
  level above 0 (`mip_chain_gpu`, `src/pvr/backend.rs:6241`; one
  `record_mip_level` a level, `:6268-6278`, a frame of its own with one
  full-screen quad, `:6138-6202`).
- gl_test case 216 `mipchains` (terrain's four chains, 20 frames, glFinish
  each; `driver/tools/pvrlab/gl_test.c:20012`): 46.15 ms a frame, 40 kicks
  a frame, GPU TA|3D busy 7.75 ms a frame (main `61ad38a1` plus the kick
  count, `tools/cts/RESULTS.md:300-305`). So a queued kick costs about
  1.15 ms of wall time, of which **at most 0.19 ms** is the GPU's hardware
  busy time (7.75 / 40, the TA's and the 3D's busy time together). About
  0.96 ms a kick is outside the hardware: host time, our waits, or the
  firmware's per-job work.
- The levels share one target size (level 1's, `backend.rs:6124-6131`,
  `:6269`), so a chain does not recycle HWRT datasets; it can wait for its
  own members, below.

A transfer job helps only if the 0.96 ms is mostly in the part of a kick
that a transfer job does not have.

## What the firmware ABI needs for a transfer context and its jobs

From the DDK 25.2 kernel headers in `third_party/ti-img-rogue-ddk-25.2`:

- **The data masters of this core.** The DM numbers are GP 0, then 1 for
  the 2D DM "when `RGX_FEATURE_TLA` defined" or the TDM "when
  `RGX_FEATURE_FASTRENDER_DM` defined", GEOM 2, 3D 3, CDM 4
  (`include/rgx_common.h:97-105`; only one of TLA and FASTRENDER_DM can be
  present, `:69-73`). The BXS-4-64's feature list
  (`hwdefs/rogue/km/configs/rgxconfig_km_36.V.104.796.h:57-112`) defines
  **neither** `RGX_FEATURE_TLA` nor `RGX_FEATURE_FASTRENDER_DM`. Mesa's
  device info for the core says the same: `bxs-4-64.h` in
  `third_party/mesa-26.2.1/src/imagination/common/device_info/` has no
  `tla` and no `fastrender_dm` feature. So this core has no 2D DM and no
  TDM: its only transfer path is **TQ3D**.
- **The context.** `RGXFWIF_FWTRANSFERCONTEXT` is one
  `RGXFWIF_FWCOMMONCONTEXT sTQContext`, "Firmware context for TQ3D"
  (`include/rogue/rgx_fwif_km.h:477-482`), beside the render context's two
  (`sTAContext`, `s3DContext`, `:454-455`). The common context is the one
  `ctx_init` already fills for the render context
  (`include/rogue/rgx_fwif_shared.h:1063-1131`: CCB control and base
  `:1065`, the suspend state `:1070`, the memory context `:1101`, the DM
  `:1122`, and the DM-specific union whose transfer member
  `RGX_CONTEXT_DATA_TRANSFER`, `:1047-1055`, holds only TRP checksums).
  So a transfer context is a second copy of what
  `driver/rust/src/gpu/pvr/render.rs:578-599` writes for the 3D context: a
  client CCB and its control, a context state, the same firmware memory
  context, a server context ID, `RANGE_CHECK_DM` = 3 (`DM_3D`).
- **The command.** `RGXFWIF_CCB_CMD_TYPE_TQ_3D`, "3D DM command for TQ
  operation" (`rgx_fwif_km.h:1319-1321`), next to `_3D` (`:1322`), `_TQ_TDM`
  and `_TQ_2D` (`:1331-1338`, not on this core). Its payload is not in the
  vendored kernel headers: the DDK keeps the client command structs in its
  user-mode headers, which we do not have (the same gap `render.rs:19-24`
  states for the render commands, whose words past `CMDTA3D_SHARED` are a
  DDK capture). Mesa's layout for the mainline firmware is
  `CMD_TRANSFER`, 43 dwords
  (`third_party/mesa-26.2.1/src/imagination/csbgen/rogue/kmd_stream.xml:223-248`):
  the background PDS words, `isp_mtile_base`, the PBE words, the USC
  clear registers, `isp_mtile_size`, `isp_render_origin`, `isp_ctl`,
  `isp_aa`, the end-of-tile PDS words, `isp_render`, `isp_rgn`. That is a
  3D job whose ISP input (region headers and primitive blocks) is built by
  the host, not by a geometry phase. The DDK 25.2 firmware's body is
  expected to carry the same registers in its own order; the order is not
  known without a capture.
- **The parameter heap.** The DDK reserves `RGX_TQ3DPARAMETERS_HEAP_BASE`
  0xE4_0000_0000, 16 GiB, "aligned to match `RGX_CR_ISP_PIXEL_BASE`"
  (`include/rogue/rgxheapconfig.h:200-205`): the host-built ISP input of a
  TQ3D job lives in a heap the driver maps today for nothing
  (`driver/rust/src/gpu/pvr/fwif.rs:58-73` lists the heaps we map).
- **The programs.** The TQ USC code has a reserved region in the USC heap
  (`RGXFWIF_KM_USC_TQ_SHADER_CODE_OFFSET_BYTES`, 512 KiB,
  `rgx_fwif_shared.h:1356-1358`). The end-of-tile programs exist already:
  `driver/src/pvr_usc_eot_blob.h:12-18` is `pvr_usc_eot()` with "the
  transfer context's props (`pvr_arch_job_context.c`
  `pvr_transfer_eot_shaders_init`)", which the render path uses today. The
  pixel program that samples the source is ours already (`mipgen.frag`,
  `PVR_INTERNAL_MIPGEN`), but a transfer job feeds it through the
  background object (`pds_bgnd*`), not through a draw's PDS state.
- **The kick and the fences.** One `RGXFWIF_KCCB_CMD_KICK`
  (`rgx_fwif_km.h:906`) whose `RGXFWIF_KCCB_CMD_KICK_DATA`
  (`:584-598`) names the transfer common context and the CCB's new write
  offset (`ui32CWoffUpdate`, `:587`): the same kick `render.rs:1600-1618`
  builds for each half of a render (`kick_data`), posted by `kccb_post`
  (`driver/rust/src/gpu/pvr/ccb.rs:68`). Ordering against the render
  context is by UFO `FENCE`/`UPDATE` commands in each CCB, as the render's
  3D fences its geometry today (`render.rs:1576-1584`).
- **The HWPerf events exist.** `RGX_HWPERF_HW_3DTQKICK` "3D TQ started"
  and `RGX_HWPERF_HW_3DTQFINISHED` (`include/rgx_hwperf_common.h:106,120`)
  beside `RGX_HWPERF_HW_3DKICK` (`:107`): the firmware tells a TQ3D job
  from a render's 3D, but both are 3D-DM work.

Mesa's transfer job code (`pvr_job_transfer.c`, the ISP primitive blocks,
`pvr_arch_job_context.c`'s transfer context) is **not** vendored:
`third_party/mesa-26.2.1/src/imagination` holds the device info, csbgen and
the PDS generators only. Vendoring it is a `third_party/` change, which
CLAUDE.md reserves.

## Does a TQ3D job run on the fragment data master's queue?

**Yes, in the sense that matters.** It is a command for the 3D data master
(`rgx_fwif_km.h:1319-1321`; `RGXFWIF_FWTRANSFERCONTEXT` exists only "for
TQ3D", `:480`), and this core has no other transfer DM (above). It has its
own client CCB, so it is a separate queue in the firmware's scheduler, but it
runs on the same 3D hardware, one 3D job at a time, interleaved with the
render context's fragment phases by the firmware (by priority, at job
boundaries; the context-switch profile is the DDK's, `fwinit.rs`, see
`pvr-fence-wait.md` "The firmware facts").

So a TQ3D job keeps the 3D half of a kick: the firmware's 3D take-up, the
ISP and the end-of-tile flush. It drops: the geometry command and its
VDM/PPP/TE work, the HWRT dataset and its PM state, the fragment phase's
`FENCE` on the geometry UFO, and the `hwrt_get` and UFO-ring waits on the
host (below). Whether that is a gain depends on where the 0.96 ms a kick
sits, which no record says.

## What the host spends between kicks today (code)

The 9 kicks of a chain are queued, not waited for (`backend.rs:6279-6286`),
but `submit` can wait on two limits, both through `wait_for`
(`render.rs:2327`), whose poll is 1 ms unless `pvr-fine-wait` is built
(`render.rs:2026-2028`; Kconfig `YAKOGL_DRV_GPU_PVR_FINE_WAIT`, default n):

- "HWRT member in flight": one size has at most `HWRT_MEMBERS` = 4 datasets
  (`render.rs:85`); the fifth kick of a size waits for the oldest member's
  last kick (`render.rs:749-772`). A chain is 9 kicks of one size.
- "geometry UFO ring throttle": at most `GEOM_UFO_SLOTS` = 16 kicks in
  flight (`render.rs:54`, `:1394-1398`). Case 216 posts 40 a frame.

A TQ3D job uses neither (no HWRT, and its own UFO words), so these waits are
part of what path A would remove, but they are also removable without it.
With four or more kicks queued they should not starve the GPU unless the
1 ms quantum drains the queue; the experiment's arm 0 tells.

## One submit carrying N level blits, with ordering

- **One submit, N jobs: yes, by the ABI.** The firmware consumes a client
  CCB up to the write offset a KICK carries (`ui32CWoffUpdate`,
  `rgx_fwif_km.h:587`), so N `TQ_3D` commands written back to back go out
  with one `KCCB_CMD_KICK`. The host's saving is N - 1 KCCB posts; the
  measured `submit` of a whole render kick is 5-7 us (`pvr-fence-wait.md`,
  kick-times table), so the host gains at most about 50 us a chain. Any
  real gain would be the firmware's, if it pays a per-KCCB-command cost.
- **Ordering between levels: yes, as today.** Commands of one client CCB run
  in order, and the 3D DM runs one job at a time. Today's chain already
  relies on that: 9 render 3D commands in one fragment CCB, no fence between
  levels, and the pictures are right (case 216 PASS, column `am62p` of
  `tools/cts/glmark2-pictures.txt`). An explicit `UPDATE` after level k and
  a `FENCE` on it before level k+1 (32 bytes a level) makes it independent
  of that.
- **Ordering against the render context needs new fences in both
  directions.** Level 0 is rendered by a render kick, so the first TQ job
  must `FENCE` on that kick's fragment UFO, and the next render that samples
  the chain must `FENCE` on the transfer's last `UPDATE`. Two traps in the
  code: the fragment UFO is one word every kick overwrites
  (`render.rs:559`, `:1583-1584`), and the firmware's UFO check is an
  equality test (`render.rs:546-549`), so a transfer job fencing on
  "fragment UFO == seq" can miss its value when a later render moves the
  word first and then never run. It needs a slot ring per kick, as the
  geometry UFO has (`render.rs:546-555`, `geom_ufo_slot`). And today's 3D
  command carries one `FENCE` (`render.rs:1579`); a render that samples a
  TQ-built chain needs a second UFO in it.

## The smallest board experiment

It measures what a TQ3D job could cost against a level's kick **without** a
transfer context: the 3D half of the same level kicks, on the render
context, with the instrument that already exists (`pvr-kick-times`,
`driver/rust/src/gpu/pvr/kick_times.rs`; `pvr-fence-wait.md`, "The
instrument").

**Arm 0, configuration only (no code).** gl_test case 216 on two images
from one main commit, one with `CONFIG_YAKOGL_DRV_GPU_PVR_FINE_WAIT=y`.
Record value: `gl_test: mipchains: ... ms a frame` and the `per frame: TA
busy, 3D busy, TA|3D` line of each.
- If the frame falls by 15 ms or more (0.4 ms a kick of 40), the per-kick
  cost is our host's 1 ms waits (`hwrt_get`, the UFO ring), not the
  firmware's: fix the waits, and path A stops here (no-go).
- Also read `TA busy` and `3D busy` apart (case 216 prints them,
  `gl_test.c:20166-20169`; the RESULTS row gives only their union). They
  bound the hardware share of each half.

**Arm 1, an experiment build (not for main).** One chain alone, per kick:
1. `driver/rust/src/gpu/pvr/kick_times.rs`: `KT_RING` 8 -> 16 (`:31`), so
   a chain of 9 and its pass fit, and a reader that copies the ring out
   (today only the last completed kick is exported, `last_kick_us`,
   `:282`).
2. `driver/src/pvr_render.c`/`.h` and `driver/rust/include/yakogl_drivers.h`:
   the C wrapper for that reader (beside `pvr_render_last_kick_us`,
   `pvr_render.h:216`).
3. `driver/tools/pvrlab/gl_test.c`: a case beside 216 that renders one
   800x600 RGBA8 texture, calls glGenerateMipmap once, glFinish, and prints
   each kick's `t_posted`, `t_geom`, `t_frag` (with their error bars) over
   20 repetitions; images with `PVR_KICK_TIMES=y` and `PVR_FINE_WAIT=y`
   (20 us reads, so every stamp is bounded).

Record value, per level kick k (the mean over levels 2..9 and the 20
repetitions):
- `period` = t_frag(k) - t_frag(k-1), the retire interval (about 1.15 ms
  if the queued cost holds);
- `frag3d` = t_frag(k) - max(t_geom(k), t_frag(k-1)), the 3D half once both
  its inputs are there: the stand-in for one TQ3D job, which has the same
  3D take-up, ISP and end-of-tile work;
- `geom` = t_geom(k) - t_posted(k).

Decision:
- `frag3d` >= 0.8 x `period`: the cost is the 3D DM's own per-job work,
  which a TQ3D job keeps. **No-go** for path A; the lever is the cost of a
  3D job itself (firmware time, HWPerf `3DKICK`/`3DFINISHED`).
- `frag3d` <= 0.5 x `period`: the geometry half and its hand-over are most
  of a level. **Go** to arm 2: 45 kicks a frame at (period - frag3d) each
  is the prize (at 0.6 ms, about 27 of terrain's 83 ms).
- In between: arm 2 only if 45 x (period - frag3d) is 15 ms or more.

**Arm 2, the real job (only after a go).** One TQ3D job's cost measured
directly: a transfer context (one common context, CCB, state), the
TQ3DPARAMETERS heap mapped, and one `TQ_3D` command replayed from a DDK
capture (below) with our addresses, kicked once with N = 1 and once with
N = 9 jobs in one KICK. Record value: post to `UPDATE` for N = 1, and the
per-job interval for N = 9, against arm 1's `period`.

## The work and the risks of the whole path

Work, after a go from arm 1 (an estimate):
1. The TQ_3D command layout of DDK 25.2: a capture from TI's Linux DDK on
   the AM62P running a blit (the render commands were captured the same
   way, `render.rs:19-24`). One board session; the risk is that the lab's
   Linux image is not runnable any more.
2. `driver/rust/src/gpu/pvr/render.rs`: the transfer context
   (`ctx_init`'s second half for one more common context), `submit_tq`, a
   slot ring for the fragment UFO, the second `FENCE` in the 3D command,
   teardown, and `wait_for`'s re-post and HWR paths for a second context
   (`last_kick` knows one context). `fwif.rs`/`mmu.rs`/`mem.rs`: the
   TQ3DPARAMETERS heap. About 600-900 lines with host tests (the `tools/pvr/km`
   scenario and `render.rs`'s mock).
3. The host-built ISP input for a rectangle (region headers and one
   primitive block a level), and the background PDS program that runs the
   sampling USC program: Mesa's `pvr_job_transfer.c` is the reference and
   is not vendored (a `third_party/` decision first).
4. `src/pvr/backend.rs` `mip_chain_gpu` on the TQ, `src/osal/render.rs` and
   `driver/src/pvr_render.*` for the new submit, the fences between render
   and transfer, `src/pvr/backend_tests.rs` rigs.
5. Board: the capture, arm 2, then glmark2 with pictures against column
   `am62p`.

Two to three weeks of one agent, three or more board sessions. Risks: the
unvendored layouts (a wrong word hangs the 3D DM; the 20 ms re-post and HWR
paths then meet a context they do not know); the UFO equality trap above;
the picture (our mipgen sample through the background object must stay
the same bilinear sample, `am62p-pvr-mipmap-kicks.md` "Which steps are
exact"); and the gain itself, which arm 1 bounds before any of this.

Ownership: every file of steps 2 and 4 is held by another plan while this
one runs. `src/pvr/backend.rs`, `src/pvr/backend_tests.rs`,
`src/pvr/internal.rs`, `driver/src/pvr_render.*`, `driver/rust/src/gpu/pvr/**`,
`src/osal/render.rs`, `tools/check.sh` belong to plan #325 (PowerVR ES 3.0
host work); `src/pvr/**` (partial render) and `driver/rust/src/gpu/pvr/**`
also to plan #301. Arm 1 touches `driver/rust/src/gpu/pvr/kick_times.rs`
and `driver/src/pvr_render.*` (both #325's and #301's) and
`driver/tools/pvrlab/gl_test.c` (no plan holds it), so it is an experiment
branch built for one board session, not a change for main.

## Not path A, found on the way

- The core has `RGX_FEATURE_COMPUTE` (`rgxconfig_km_36.V.104.796.h:59`) and
  the DDK knows a "Compute context used for transfers"
  (`RGX_CONTEXT_FLAG_CDM_TRANSFER`, `rgx_common.h:234-235`). A compute
  dispatch per level runs on the CDM, a different data master from the 3D,
  with no tiling and no HWRT. Its per-job firmware cost is as unknown as the
  TQ's, and our USC encoder has no compute programs. Not studied further.
- "One submit for the chain" does not need a transfer context: N geometry
  commands and N fragment trios in the render context's two CCBs go out
  with one `COMBINED_TA_3D_KICK` (`render.rs:1621-1628`) as well, but each
  needs its own HWRT member in flight, and `HWRT_MEMBERS` is 4.
