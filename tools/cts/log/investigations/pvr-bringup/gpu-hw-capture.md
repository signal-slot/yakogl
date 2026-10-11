<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# Ground-truth GPU render/transfer on the Verdin AM62P (BXS-4-64)

Status: finished record, moved here from `driver/docs/gpu-recipes/gpu-hw-capture.md` on 2026-10-11. How the vendor stack was observed on 2026-09-07 during bring-up. The driver no longer depends on any capture (docs/source-form.md).

Bring-up note from 2026-09-07, cut to its method and conclusions. The original
quoted the DDK firmware trace and raw trace words from the vendor stack; those
excerpts are not reproduced here.

Board: Verdin AM62P, Torizon Linux (kernel 6.6.x), GPU BVNC 36.53.104.796
(BXS-4-64), vendor DDK 25.2, `/dev/dri/renderD128` (+ `card1` = pvr,
`card0` = tidss).

## (A) How a reference run was observed

The Mesa/GLES route did not work on that board image (the PVRDRI handshake
between the container's Mesa and the DDK support library failed and Mesa fell
back to llvmpipe). The DDK's own native self-test binaries talk to the kernel
through `libsrv_um` without Mesa or EGL, and the kernel exposes a firmware
trace and debug dump through debugfs:

1. Get a root shell in the host namespace (privileged container plus
   `nsenter -t 1 ...`).
2. Enable firmware trace logging with
   `pvrdebug -log trace -loggroups main,mts,rtd,pow,hwr,pm,spm,misc`.
3. Run a real workload: `/usr/bin/rgx_triangle_test` (TA geometry + 3D to the
   DSI panel), `rgx_blit_test` (the transfer-queue blit, "TQ_3D"),
   `rgx_compute_test`, `rgx_xd_blit_test`, `rgx_twiddling_test`.
4. Immediately read `/sys/kernel/debug/pvr/gpu00/firmware_trace` (decoded
   kick-level trace) and `debug_dump`, and run `pvrdebug -dd` / `-dds` for the
   register-level 3D debug info. The firmware powers the core off about 100 ms
   after idle, after which `debug_dump` reports "RGX power is down"; to see
   live ISP/PBE/TDM registers run `pvrdebug -dd` in a tight loop while a long
   workload runs.

Other debugfs nodes: `firmware_mappings`, `power_timing_stats`,
`utilisation_stats`, `/sys/kernel/debug/pvr/{version,status,driver_stats}` and
the writable `apphint/` knobs (`EnableLogGroup`, `FirmwareLogType`,
`HWPerfFWFilter`). For how to enable and decode the trace from our own driver
see `gpu-fwtrace-enable.md`.

What did not work: headless GLES via Mesa/GBM on `renderD128`, kmscube and
es2gears (all llvmpipe); `pvr_memory_test` (a CPU/DMA memcpy bandwidth test, it
never kicks the GPU); a stock `graphics-tests` container (no DDK). No DDK or
Mesa source is on the board.

## (B) What distinguishes a working fill from the first hand-built one

A real transfer-queue fill or blit is not a lone 3D (fragment) kick. It is a
paired, fence-chained GEOM(TA) + 3D submission that lays down geometry into
per-tile primitive lists. The PBE only emits for tiles that contain rasterised
primitives, so a 3D-only kick over empty tiles completes cleanly and writes
nothing: it submits, completes, signals its UFO, raises no MMU fault, and
`PIXELBE_END_RENDER` never sets. That was the symptom of the first hand-built
fill.

The observed order of one reference blit (and the same for the triangle test):

1. A GEOM command (data master 2) and a 3D command (data master 3) are queued,
   each on its own firmware context.
2. The 3D command blocks on a UFO check against the value the TA will write.
3. The firmware loads the parameter-buffer freelist and the render-target data
   (HWRTData) on a hardware context, and lists the TA buffers (VHeap, TPC) and
   the TE region-header base. The tiling engine then writes per-tile lists.
4. The TA kick runs and finishes, and updates the UFO; the 3D check is
   satisfied.
5. The 3D phase updates tiles-in-flight (dusts, partition mask, ISP control),
   a USC pixel/EOT shader runs per tile, the 3D kick runs, and the firmware
   reports it finished with a successful HWRTData state.

Why a bare 3D kick emits nothing: with no TA phase the tiling engine never
writes region headers, every tile's primitive list is empty, the 3D phase takes
the `process_empty_tiles` path (background only, clear registers), the ISP has
nothing to rasterise and the end-of-render emit never fires. Static register
values matching Mesa are necessary but not sufficient: the emit is driven by the
tiled primitive lists and the UFO-ordered TA to 3D dependency, not by the 3D
register block alone.

What the working 3D kick has that an empty one lacks:

- valid HWRTData shared between the TA and 3D contexts, carrying VHeap, TPC and
  the TE region-header base;
- a loaded parameter-buffer freelist that the TA grows;
- a UFO fence: the 3D waits on the value the TA writes;
- non-empty tiles: at least one primitive tiled by the TA, so the USC tile count
  is above zero and the ISP and PBE process and flush each tile.

## (C) The fix for a hand-built fill

Make the fill a GEOM(TA) + 3D pair:

1. Kick the TA first with one screen-covering primitive and a trivial vertex
   program, with an HWRTData shared with the 3D kick, a parameter-buffer
   freelist, and VHeap and TPC buffers.
2. Kick the 3D fenced on the TA by UFO, referencing the same HWRTData.
3. Reach the ISP/PBE setup on the non-empty-tile path, with the partition mask
   matching the active dusts and a USC pixel/EOT program bound that writes the
   colour (the working clear still runs a per-tile USC shader).

A pure clear with no geometry would have to drive the ISP background object so
that tiles are still processed and flushed rather than routed to
`process_empty_tiles`; the shipped reference path (the DDK self-tests and Mesa)
uses real geometry. Mirroring the TA + 3D + UFO structure is the reliable fix.
