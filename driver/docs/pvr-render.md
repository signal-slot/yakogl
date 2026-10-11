<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# The PowerVR render service: the pool and the fence wait

What the render service does today with GPU memory and with the frame fence,
and the decisions behind its defaults. The measurements that led here are
two logs: `tools/cts/log/investigations/pvr-render-pool.md` (the render
pool, AM62P, 2026-10-03 to 10-05) and
`tools/cts/log/investigations/pvr-fence-wait.md` (where a swap spends its
time, 2026-10-03 and 10-04). Code comments cite those logs by section name.

The code: `driver/rust/src/gpu/pvr/` (`pool.rs`, `mem.rs`, `freelist.rs`,
`render.rs`, `kick_times.rs`), `driver/src/pvr_render.c`, the Zephyr OSAL
`src/osal/zephyr.c` and the back end `src/pvr/backend.rs`.

## The render pool

**Layout.** The carveout is the board's reserved-memory region (64 MiB at
0x97800000 on the Verdin AM62P). The render pool is its tail after the
firmware heap and the page tables. Each heap (GENERAL, PDS, USC, PM) has its
own device-VA cursor; one physical cursor is shared by all heaps.

**Who allocates.** The parameter-buffer freelists, the per-size HWRT buffers
and the render context's VDM stream take memory from the bump at init. GL
objects (buffers, textures, renderbuffers, program code and uniforms, the
back end's frame-set arenas and depth/stencil buffers) go through
`gpu_alloc` to `pvr_render_alloc`: the free lists first, then the bump.

**Who frees.** The GL core frees on delete. The OSAL defers each free until
a fence (`src/osal/pending_frees.h`). The pool keeps a free list per heap
(`gpu::pvr::pool`): best fit on allocation, merging with both neighbours on
free. Two blocks merge only when they are adjacent in both VA and CPU space.

**The top region.** A GENERAL request larger than `YD_PVR_POOL_TOP_ABOVE`
(256 KiB, set in `pvr_fw_boot.c` `km_setup`) is placed from the carveout's
end downward (`mem.rs` `bo_alloc_top`). Everything else comes from the bump,
which stops at the top region's floor, so the two ends share one gap. A free
that reaches the floor moves it up at once, so a scene's large blocks go back
into the gap in one piece. Without it, small long-lived blocks split the free
space and glmark2's refract could not get its 7600 KiB depth buffer.
`-DPVR_POOL_TOP_ABOVE=0` builds the old pool, for an A/B run.

**Reclaim (`CONFIG_YAKOGL_PVR_POOL_RECLAIM`, default y).** A free made while
no frame is open waits only for the last submitted kick. An allocation the
pool refuses first takes back what completed kicks held and tries again; if
it is still refused, it waits for the submitted kicks and tries once more.
Without it, glmark2's shadow depth texture was refused beside 23932 KiB of
held frees (AM62P, 2026-10-04; gl_test 178 `heldfree` reproduces it).

**Frame sets (`CONFIG_YAKOGL_PVR_FRAME_SETS`, default 4).** Each set is a
GENERAL arena, a PDS arena, a 64 KiB VDM stream and small tables. At the
defaults (4 x (1024 KiB GENERAL + 768 KiB PDS)) they use 7488 KiB of the
pool. A frame that overflows an arena is split; a draw that does not fit an
empty frame is refused, naming the arena.

**Opt-in, unmeasured as a default:**

- `CONFIG_SAFEUI_GPU_PVR_FL_GROW_ON_DEMAND`: the global freelists keep one
  grow's pages in reserve instead of four and refill from the pool. It gives
  back 2 x 3 MiB. What the firmware does after a refused grow is unmeasured.
- `CONFIG_YAKOGL_PVR_POOL_OWNERS`: a measurement; prints the live pool
  memory by owner at run end.

**What a run prints.** At exhaustion and at the end of a run: the bump
high-water, the free bytes and largest free block per heap, the bytes held
in the pending-free queue, and the top region ("top T KiB now, H KiB
high-water"). "N KiB left" in "render pool exhausted" is the gap above the
cursor, not free memory.

**Open.** A large buffer from scattered pages through the GPU MMU would
remove the need for one contiguous range (the log's "design, large buffers
from scattered pages"; not built). The carveout's size has no Kconfig; the
driver takes the memory-region's size.

## The fence wait

**The poll.** The render service reads the fragment UFO once per millisecond
by default (`gpu::pvr::render::WAIT_POLL_US` = 1000), so every wait ends on a
whole millisecond. `CONFIG_SAFEUI_GPU_PVR_FINE_WAIT` (yakogl-drivers feature
`pvr-fine-wait`) reads it every 20 us. It stays opt-in until gl_test and
glmark2 have run with it on the AM62P. The GPU's interrupt is in the device
tree (GIC SPI 241) but not connected; the PowerVR driver has no ISR.

**The instrument.** `CONFIG_SAFEUI_GPU_PVR_KICK_TIMES` (feature
`pvr-kick-times`) stamps each kick: submit, the MMU invalidate, the post,
the first read that saw the geometry and fragment fences, and the wait. The
glmark2 sample prints them.

**One kick in flight.** `PVR_BACKEND_OPT_ONE_IN_FLIGHT` (`src/pvr/backend.rs`)
waits for the most recent kick too. It measures the fragment phase alone; it
is not a default.

**The swap drops depth/stencil (the lead's decision, 2026-10-04).**

- EGL 1.5 section 3.10.1: "The contents of ancillary buffers are always
  undefined after calling eglSwapBuffers." `EGL_BUFFER_PRESERVED` applies to
  the colour buffer only.
- `eglSwapBuffers` tells the back end of window swaps only. A pbuffer keeps
  its depth/stencil across a swap, on every GPU.
- `PVR_BACKEND_OPT_SWAP_DROPS_ZS` is on by default
  (`PVR_SWAP_DROPS_ZS_DEFAULT` = 1). The opt-outs are
  `CONFIG_GLMARK2_PVR_KEEP_ZS` and `CONFIG_GL_TEST_PVR_KEEP_ZS`.
- gl_test 172 `swapzs` and 173 `splitzs` assert each back end's behaviour
  across a window swap, and that a frame split without a swap keeps its
  depth.
- Why: the fragment phase of a light glmark2 kick is bound by memory
  traffic. Dropping the depth/stencil store took it from ~1.95 ms to
  ~0.88 ms on the AM62P, and the glmark2 score from 315 to 550 (gl_test 37/38
  in both arms).

**The init quad.** A frame that initialises depth/stencil draws a
full-screen quad, because with `ISP_BGOBJVALS.enablebgtag` off the ZLS store
wrote 0 where no object covered a pixel (gl_test 13). Skipping it saved only
8-40 us per light scene, and the knob was removed.

**Open.** Mesa sets `enablebgtag` whenever the render has a colour load op,
and clears depth/stencil of a full-screen single-layer render with no quad.
The same on our side is a possible next knob: opt-in, judged first by gl_test
13, 172 and 173 and the set. Not built. What one full-screen pass costs per
pixel (about 1 ns measured, against a nominal 4 pixels a clock at 800 MHz)
needs the GPU's counters or the firmware's HWPerf, and neither is read today.
