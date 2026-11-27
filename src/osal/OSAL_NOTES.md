<!-- SPDX-License-Identifier: Apache-2.0 -->
<!-- SPDX-FileCopyrightText: 2026 Signal Slot, Inc. -->
# YakoGL OSAL — implementation notes

The OSAL (layer 5b, `include/gles_osal.h`) is the only place the GL core and the GPU
backend touch the platform. One C translation unit per platform implements
`struct osal_ops`; `src/osal/mod.rs` (Rust, part of the crate `yakogl`) provides the single
`g_osal` pointer and selection glue. The OSALs stay C on purpose: they are thin glue
over C APIs (Zephyr macros and inlines, board hook pointers, a variadic `log`) that
stable Rust cannot define.

Zephyr has one such translation unit per GPU, because the driver underneath is
a different driver: `zephyr.c` for the PowerVR BXS-4-64, `zephyr_gc.c` for
the Vivante GC7000UL, `zephyr_v3d.c` for the Broadcom V3D 4.2,
`zephyr_v3d71.c` for the Broadcom V3D 7.1 and `zephyr_mali.c` for the Arm
Mali (G610, and the G310 built only). All define `osal_zephyr_ops`, the
module build compiles exactly one of them (chosen by the Kconfig symbol of
the GPU, `zephyr/Kconfig`; the V3D 7.1's symbol `CONFIG_YAKOGL_GPU_V3D71`
is set on top of the V3D 4.2's), and nothing
above the OSAL knows which. Two more files serve them all:
`zephyr_timing.c` (always) and `zephyr_shaderc_scratch.c` (with the
in-process compiler, `CONFIG_YAKOGL_SHADERC_ONLINE`).

## Files (all under `src/osal/`)

| File              | Role                                                           |
|-------------------|---------------------------------------------------------------|
| `osal_internal.h` | Private backend↔OSAL contract, the C view: the kick descriptor's shape per GPU (see below), the `OSAL_QUERY_*` ids and the structs they fill, the Zephyr init args (`struct osal_zephyr_init`, `osal_zephyr_v3d_init`, `osal_zephyr_mali_init`), the run-end reports, the GPU-MMU L0 PTE flags, the weak `yakogl_zephyr_present`, and the `g_osal` selection decls. Not part of the public interface. Its Rust mirror is `mod.rs` / `render.rs` / `mali.rs`; the generated layout pins (`src/abi_layout.rs` / `src/abi_layout_check.c`, `yakogl-tools abi-layout`) fail the build when the two drift. |
| `zephyr.c`        | Zephyr backend for the PowerVR — wraps the silicon-proven bring-up driver. |
| `zephyr_gc.c`     | Zephyr backend for the Vivante GC7000UL — wraps the driver's render service (`driver/src/vivante/gc_render.h`). Compiled instead of `zephyr.c` for the Vivante Kconfig symbol (`zephyr/CMakeLists.txt`). |
| `zephyr_v3d.c`, `zephyr_v3d.h` | Zephyr backend for the Broadcom V3D 4.2 (Raspberry Pi 4). The V3D has no firmware, no ring and no Zephyr driver, so this file is the whole render service, on the driver's OS-free steps (`driver/src/v3d/v3d_hw.c`). `zephyr_v3d.h` holds the instruments it offers a V3D sample (memory use, free runs, held frees). Compiled for the V3D 4.2's Kconfig symbol (`zephyr/CMakeLists.txt`). |
| `zephyr_v3d71.c`, `zephyr_v3d71.h` | The same for the V3D 7.1 (Raspberry Pi 5, BCM2712), on the BCM2712 path of `driver/src/v3d/v3d_hw.c`; it shares the 4.2 OSAL's allocator and free rule (`v3d_granules.h`, `v3d_frees.h`) and instruments (`zephyr_v3d.h`), and refuses init unless the PM watchdog is armed. `zephyr_v3d71.h` adds the 7.1 TFU job. Compiled instead of `zephyr_v3d.c` for `CONFIG_YAKOGL_GPU_V3D71`. |
| `zephyr_mali.c`, `zephyr_mali_board.h` | Zephyr backend for the Arm Mali (the NanoPC-T6's Mali-G610; the Verdin iMX95's Mali-G310, built and host-tested only). The OSAL's logic is Rust in the drivers crate (`driver/rust/src/board/nanopc_t6_osal.rs`, the `yd_t6_osal_*` entries: the GPU and its CSF firmware brought up, one group with one stream, a carve-out, each kick one job on the ring); this file holds the image's GPU memory, the log and the table. What depends on the SoC (register mappings, power-on, the watchdog) is the board's half, declared in `zephyr_mali_board.h` and implemented once per board under `driver/zephyr/lib/board/`. Compiled for `CONFIG_YAKOGL_GPU_MALI`. |
| `zephyr_shaderc_scratch.c` | The memory the in-process shader compiler works in: one static region, lent once at boot, sized by `CONFIG_YAKOGL_SHADERC_SCRATCH_BYTES`. Linked beside any Zephyr OSAL when the compiler is built in. |
| `zephyr_timing.c` | The time answers a Zephyr OSAL gives (`OSAL_QUERY_CLOCK_NS`, `OSAL_QUERY_DRAW_STATS`, through `osal_zephyr_timing_query()`), in one place so the OSALs cannot drift apart. |
| `baremetal.c`     | Bare-metal backend skeleton — direct MMIO + static carveout; proves the portability seam (2nd backend, same interface). The board registers its addresses and hooks first (`include/gles_osal_baremetal.h`, `osal_bm_board_config()`); init refuses without them (see below). |
| `linux.c`         | Host backend with no GPU: malloc'd GPU memory, fake device VAs per heap, submits complete at once. Runs the GL core, a GPU backend (PowerVR, Vivante, or the V3D with a model of the Pi 4's carve-out and its Zephyr OSAL's allocator and free rule) and the shader compiler on a Linux host under ASan/valgrind with a real application's call stream (`-DYAKOGL_OSAL=linux`, `-DYAKOGL_GPU=powervr|vivante|v3d`); the host harness `tools/hostharness/` links it (its recording OSAL can allocate through it). Renders nothing. |
| `osal_bounds.h`   | `osal_gpu_copy_ok()`: the one argument check every OSAL that has a `gpu_write` / `gpu_read` (all but the Mali's, whose two are NULL) makes before its `memcpy` (NULL `mem`, `mem->cpu` or buffer, and `off > size || n > size - off`, a form in which `off + n` cannot wrap). The contract is in `include/gles_osal.h`; the host checks it with `tools/osal/osal_bounds_check.c`. |
| `osal_log.h`      | The one line buffer (`OSAL_LOG_LINE_MAX`) a Zephyr OSAL's `log` formats into before it prints, so a core line reaches the log whole. Host check: `tools/osal/osal_log_check.c`. |
| `pending_frees.h` | The queue of frees held until the kick that may read them completes, and its rule on a full queue (wait for the oldest kick, or refuse and count). Included by `zephyr.c`, and through `gc_frees.h` and `v3d_frees.h` used by `zephyr_gc.c`, `zephyr_v3d.c`, `zephyr_v3d71.c` and `linux.c`'s V3D model. Host check: `tools/osal/pending_frees_check.c`. |
| `gc_frees.h`, `gc_cache_range.h` | The GC7000 OSAL's free rule (which fence a free waits for) and its "is this range inside the non-cacheable carve-out" test. Host checks: `tools/gc/osal_frees_check.c`, `tools/osal/gc_cache_range_check.c`. |
| `v3d_frees.h`, `v3d_granules.h` | The V3D OSALs' free rule and their carve-out allocator (a granule bitmap, first fit, small blocks from the top opt-in). Host checks: `tools/v3d/osal_frees_check.c`, `tools/v3d/osal_granules_check.c`. |
| `linux_heap.h`    | `linux.c`'s fake device-VA allocator: a finite range per heap, first fit over a coalescing free list. Host check: `tools/osal/linux_heap_check.c`. |
| `mod.rs`          | `struct osal_ops` / `struct osal_gpu_mem` as `#[repr(C)]` (`OsalOps`, `OsalGpuMem`); defines `g_osal` (initial value = the ops table of the OSAL the cargo feature `osal-zephyr` / `osal-baremetal` / `osal-linux` names; null with none or under `host-test`, then the application calls `osal_set_ops()` first) + `osal_set_ops()`; the wrappers the core uses and the `glog!` macro (formats with `core::fmt`, then `log_str()` calls `g_osal->log("%s", line)`). |
| `render.rs`       | `#[repr(C)]` mirror of the driver's `driver/src/pvr_render.h` types (`struct pvr_render_kick`, `struct pvr_render_static`): the kick descriptor `submit` takes and what `query` fills. |
| `mali.rs`         | The Mali's part of the contract (`gpu-mali`): the kick descriptor `submit` takes (`OsalMaliKick`: a command stream's GPU VA and length) and what `query` fills (`OSAL_QUERY_MALI_GPU`: GPU_ID, TILER_FEATURES, the CSIF register counts; `OSAL_QUERY_MALI_TILER_HEAP`: the heap context, first chunk and chunk size), and what `query` is told (`OSAL_QUERY_MALI_FRAME_OPEN`: whether the backend records a frame; without one the OSAL holds a free only for the last submitted kick, as `OSAL_QUERY_GC_FRAME_OPEN`). The C mirrors (`struct osal_mali_gpu`, `osal_mali_heap`, `osal_mali_kick`) sit in `osal_internal.h` with the ids, pinned by `src/abi_layout_check.c` / `src/abi_layout.rs`. A development OSAL over the Linux panthor driver is `tools/mali/panthor/src/osal.rs`. Buffers are `gpu_alloc`'s (`OSAL_MEM_GPU_EXEC` for shader code), each a CPU mapping at a GPU VA. |

Build files: `CMakeLists.txt` (host/Linux/generic → `libyakogl.a`; options
`YAKOGL_OSAL=zephyr|baremetal|linux`, default `linux`, and
`YAKOGL_GPU=powervr|vivante|v3d`, where `v3d` needs the Linux OSAL),
`zephyr/CMakeLists.txt`, `zephyr/Kconfig` and `zephyr/module.yml` (the Zephyr
module, Kconfig symbol YAKOGL), `baremetal.mk` (cross-compile → `libyakogl.a`,
`GPU=powervr|vivante`). Each has cargo build the archive crate `capi/` (the
`yakogl` crate with the matching `osal-*` and `gpu-*` features, plus a panic
handler) and compiles the one OSAL TU (plus the layout pins
`src/abi_layout_check.c` and the GPU's FFI pins, e.g.
`src/pvr/ffi_layout_check.c`, and, for the PowerVR outside Zephyr, the driver's command
generators; for the V3D (CMake, Linux OSAL) its job builder). CMake and `baremetal.mk` post-process the cargo static library
with `capi/assemble.cmake`, which appends the C objects with `ar`; Zephyr
links the Rust archive next to the Zephyr library that holds the selected
OSAL. With `YAKOGL_RUST_LINKAGE=external` (`RUST_LINKAGE=external` in
`baremetal.mk`) only the C parts are built and the application links the
crate into its own Rust static library.

## The submit/wait contract (the kick descriptor)

`include/gles_osal.h` `submit()` takes an opaque `const void *kick_desc`. Its
concrete shape is per GPU, agreed between the backend and the OSAL in
`osal_internal.h`; the OSAL forwards it and never interprets it.

On a PowerVR build it is the driver's `struct pvr_render_kick`
(`driver/src/pvr_render.h`, mirrored in `render.rs`): the per-render register
blocks the backend generates (VDM stream base, PPP control,
ISP/PBE/background-object registers). The Zephyr OSAL hands it to the
driver's render service (`pvr_render_submit`), which owns the firmware
objects (render context, client CCBs, freelists, HWRT datasets per target
size) and the KCCB kick + UFO fence; `wait()` polls that fence
(`pvr_render_wait`). No FWIF type crosses the OSAL.

On a `YAKOGL_GPU_VIVANTE` build the descriptor is `struct gc_kick`
(`src/vivante/gc_kick.h`) instead: a command stream in GPU memory. The GC7000
has no firmware, so there is nothing to hand a register block to — the OSAL
gives the stream to `gc_render_submit()`, which appends the hardware's
epilogue (cache flushes, the FE/PE semaphore pair, the EVENT), allocates
the event and returns a monotonic fence.

The V3D OSALs take `struct v3d_kick` (`src/v3d/v3d_kick.h`), and the Mali
OSAL takes `struct osal_mali_kick` (`osal_internal.h`, `mali.rs`).

## How the Zephyr OSAL maps onto the driver

| osal op        | Driver plumbing wrapped                                        | where |
|----------------|----------------------------------------------------------------|-------|
| `init`         | GPU already powered/booted by the pvr DT driver (`driver/src/pvr_bringup.c` → `pvr_fw_boot`); `pvr_render_init()` builds the render service objects once | `driver/src/pvr_render.c` |
| `gpu_alloc`    | `pvr_render_alloc()`: page-aligned render-pool span, mapped at the next free VA of the heap named by the base VA (GENERAL / PDS / USC) | `driver/src/pvr_render.c`, the driver's `driver/src/pvr_drv.h` |
| `gpu_free`     | Deferred (`pending_frees.h`): the block is held until a fence at or past the newest kick that may read it has completed, then goes back to the render pool's free list (`pvr_render_free`, from `drain_pending_frees`, run from `wait` and `completed`). With `CONFIG_YAKOGL_PVR_POOL_RECLAIM` (on by default) that kick is the one being recorded while a frame is open, else the last one submitted (`pvr_backend_free_fence`), and a refused allocation waits for the last kick and retries; without it, always the kick being recorded (`s_last_submitted + 1`). A GL object freed while the GPU still reads it is therefore safe. | `driver/src/pvr_render.c` |
| `mmu_map`      | `pvr_mmu_map` for an explicit VA→phys range                    | `driver/src/pvr_drv.h` |
| `map_phys`     | `pvr_render_map_phys`: caller-owned pages (scan-out) at a fresh heap VA, no pool memory | `driver/src/pvr_render.h` |
| `gpu_write`    | `memcpy` into the CPU-mapped carveout + `sys_cache_data_flush_range` | — |
| `gpu_read`     | `sys_cache_data_invd_range` + `memcpy` out                     | — |
| `cache_flush`  | `sys_cache_data_flush_range`                                   | — |
| `cache_invd`   | `sys_cache_data_invd_range`                                    | — |
| `submit`       | `pvr_render_submit(kick)`: GEOM + FRAG client-CCB commands, MMU cache flush, two KCCB kicks, returns the fence value | `driver/src/pvr_render.c` |
| `wait`         | `pvr_render_wait(fence, ms)`: UFO poll + FWCCB servicing + HWR detection; `-ETIMEDOUT` becomes the contract's `1` | `driver/src/pvr_render.c` |
| `completed`    | `pvr_render_completed()`: the highest fence the GPU has finished (the fragment UFO), without blocking. The backend uses it to reuse a frame set only after its kick is done, and to drain pending frees. Optional: `NULL` means "nothing is known to be complete before `wait` says so" (bare metal). | `driver/src/pvr_render.c` |
| `query`        | `OSAL_QUERY_RENDER_STATIC` → `pvr_render_static_get()` (EOT program, border table, heap bases); `OSAL_QUERY_WAIT_SITE` → `pvr_render_wait_site()`; the clock and draw stats → `zephyr_timing.c` | `driver/src/pvr_render.h` |
| `reg_read/write` | `sys_read32/sys_write32` on the register base recorded at boot | `pvr_regs()` |
| `log`          | `osal_log_format()` (`osal_log.h`, `vsnprintk`) → `LOG_INF`    | — |
| `wait_vblank`  | `yakogl_zephyr_wait_vblank()`, the board layer's; the weak default returns `-ENOTSUP` (no vertical sync) | — |
| `present`, `display_busy` | `yakogl_zephyr_present()`, `yakogl_zephyr_display_busy()`: the board layer's, weak (see below) | — |

### The Vivante variant (`zephyr_gc.c`)

| osal op | Driver plumbing wrapped (`driver/src/vivante/gc_render.h`) |
|---------|------------------------------------------------------|
| `init` | `gc_render_init(DEVICE_DT_GET_ONE(vivante_gc))`: hardware init, MMU v2 page tables, the FE's idle loop, the interrupt. Idempotent. |
| `gpu_alloc` | `gc_render_alloc(size, align, ...)` after draining the frees whose kicks have completed. The alignment is 64 bytes, the strictest the hardware asks for (command buffers and shader code); with `CONFIG_YAKOGL_GC_ALLOC_PAGE_ALIGN` (a measurement, off by default) a block of a page or more starts on a page. `heap_base` is ignored: the carveout is identity-mapped, so an allocation's address is its physical address. |
| `gpu_free` | **Deferred** (`gc_frees.h`, on `pending_frees.h`): the block is held until the newest kick that may read it has completed — the kick being recorded while the backend records a frame (`OSAL_QUERY_GC_FRAME_OPEN`), else the last kick submitted; a block whose kick has already completed goes back at once. A held block goes straight to the pool (`gc_render_free_retired`), since it has waited for its own kick. The pool has a real free list, so memory handed back early would be handed straight out again — to a caller that writes it while the GPU is still reading it. |
| `mmu_map` | **absent.** The service maps at a VA it chooses; `map_phys` is the entry point, and `src/gl/egl.rs` prefers it anyway. |
| `map_phys` | `gc_render_map_phys()`. A range already inside the carveout comes back with its identity VA and the carveout's own CPU view — which is what `glReadPixels` on a window surface reads through. |
| `gpu_write` / `gpu_read` / `cache_*` | `memcpy` + `gc_render_cache_flush` / `gc_render_cache_invd`, skipped for a range inside the carveout (`gc_cache_range.h`: its mapping is Normal non-cacheable, so maintenance there only costs time) and kept for anything else. |
| `submit` | `gc_render_submit(&(struct gc_submit){...}, &fence)` with `GC_SUBMIT_FLUSH_ALL`: the backend does not say which caches its frame dirtied, and a missing flush shows a stale picture rather than an error. |
| `wait` | `gc_render_wait(fence, ms)`. `timeout_us < 0` becomes a bounded 2 s, not forever: a board booted by kexec has no way back except its own reset, so a stalled GPU has to become a failed sample. `-ETIMEDOUT` becomes the contract's `1`; the driver has already dumped the hardware state. |
| `completed` | `gc_render_completed()`. |
| `query` | `OSAL_QUERY_GC_FRAME_OPEN` (the backend tells the OSAL whether it records a frame) and the clock and draw stats (`zephyr_timing.c`); anything else is `-ENOTSUP` (`OSAL_QUERY_RENDER_STATIC` describes PowerVR firmware objects). |
| `reg_read/write` | **absent.** This backend writes a command stream and never touches a register. |

## Locks and thread ids

`lock` / `unlock` are a `pthread_mutex_t` in `linux.c`. The library takes the pair around each EGL entry point and nowhere
else, so the mutex does not have to be recursive - and must not be relied on
to be. It does not make a second thread safe: one thread issues every GL and
EGL call of the process (`include/gles_osal.h`, "threads"), no GL entry point takes
the mutex, and the library keeps no per-thread state.

`thread_id` is how the library catches a second thread. The Linux OSAL
(`linux.c`) returns the kernel thread id, kept in a `__thread` variable after the first
`gettid` (one TLS load per call, no system call); every Zephyr OSAL returns
`k_current_get()` folded to 32 bits; `baremetal.c` leaves it NULL (one flow of
control, nothing to tell apart). The Zephyr OSALs and `baremetal.c` leave
`lock` / `unlock` NULL, which costs those platforms nothing.

## Presenting on Zephyr

`present` is the display's "show this buffer", the OSAL's counterpart of a
sample's `board_show()`. On Zephyr the op is `yakogl_zephyr_present()`, which
the board library defines over `board_show()`. The OSALs declare it weak
(`osal_internal.h`), so an image without the board library has no `present`;
the Linux and bare-metal OSALs have none. With it EGL double-buffers its
window surfaces (`src/gl/egl.rs`, `EglSurface::bufs`): the swap shows the
previous frame's buffer and the next frame goes into one the display has
given up, so no application has to show anything itself, and no frame is
shown before it is complete. `display_busy` (also the board layer's, weak)
tells EGL which window buffers a display whose show does not wait may still
scan; without it EGL keeps its ring of three buffers.

## The counters at the end of a run: the report is not shutdown

Every OSAL that defers frees keeps counters a run should end with: the
pending-free queue's line ("pending frees: high-water H/CAP, waits W (opt-in
O), refused R ...", `pending_frees.h`) and, in `linux.c`, the heaps' lines.
Two different things print them:

- **shutdown** (the ops table's `shutdown`) tears the OSAL down: it prints,
  then zeroes the heaps (`linux.c`) or marks the OSAL down so the next `init`
  runs again (the Zephyr OSALs). The product never calls it: `eglTerminate`
  keeps the backend and its GPU memory for the next `eglInitialize`
  (the PowerVR and Vivante backends' init return early when already up), so a
  shutdown there would leave the backend drawing through memory that is gone.
- **the report** only prints and changes nothing. Zephyr:
  `osal_zephyr_diag_print()` (`zephyr.c`, `zephyr_gc.c`),
  `osal_v3d_diag_print()` (`zephyr_v3d.c`, `zephyr_v3d71.c`) and
  `osal_mali_diag_print()` (`zephyr_mali.c`), called at the end of a run by
  the gl_test and dEQP samples (`driver/samples/zephyr/gl_test/src/main_*.c`,
  `driver/samples/zephyr/deqp/src/`) and, for the PowerVR, by the board
  library's finish step (`driver/zephyr/lib/board/common/board.c`).
  Linux: a destructor in `linux.c` prints the same lines at process exit,
  when the OSAL came up and shutdown has not already printed them.

## errno numbers at the C boundaries

A negative errno is a number, and the number for a name depends on the C
library. The ones our code uses that differ (Zephyr's minimal libc and
picolibc agree with each other, zephyr af17c0c8aa2a; Linux is
asm-generic/errno.h, and glibc's `ENOTSUP` is Linux's EOPNOTSUPP):

| name | Zephyr (minimal, picolibc) | Linux |
|---|---:|---:|
| ESHUTDOWN | 110 | 108 |
| `ETIMEDOUT` | 116 | 110 |
| EMSGSIZE | 122 | 90 |
| `ENOTSUP` | 134 | 95 |
| ECANCELED | 140 | 125 |

So Zephyr's ESHUTDOWN is Linux's ETIMEDOUT. The names below 35 and
`ENODATA` (61) are the same in both.

**The rule: a Rust crate uses the numbering of the C library on the other
side of its boundary, and a number is never translated in between.** There
is one table file, `driver/rust/src/errno_numbers.rs`, with both numberings
(ZEPHYR, LINUX) and each value's header line. Where a number crosses:

1. **The driver crate (`driver/rust`) to C.** `yakogl_drivers::errno` is
   ZEPHYR, its only C library. Its C callers (`driver/src/*.c`,
   `src/osal/zephyr*.c`, the Zephyr samples) compare against errno.h, and
   in a Zephyr image both sides are Zephyr's (for example `zephyr.c`
   `z_wait`: `rc == -ETIMEDOUT` on `pvr_render_wait`'s result). A host
   program that links the crate under glibc translates in its own shim, as
   `tools/pvr/km/record_c.c` does by pinning `ETIMEDOUT` to 116.
2. **The C OSAL to the YakoGL crate (`src/`).** The crate's `util::E*` come
   from `errno_numbers::numbering(osal-zephyr)` (`src/util.rs`): ZEPHYR
   for the Zephyr OSALs, LINUX for `linux.c` (glibc) and for
   `baremetal.c`, whose literal -95 for `ENOTSUP` is LINUX's. So
   `src/gl/egl.rs` recognises a Zephyr OSAL's "no vsync" (`-ENOTSUP`, -134),
   and `src/gl/draw.rs` tells the backend gap (`-ENOTSUP`,
   GL_INVALID_OPERATION) from a lost draw (GL_OUT_OF_MEMORY). The host
   test zephyr_osal_reads_zephyr_numbers (`src/util.rs`) asserts what an
   `osal-zephyr` build gets: `ETIMEDOUT` is 116, not 110, and `ENOTSUP` is
   134.
3. **Logs and records to the tools.** The tools print rc values as numbers
   and name none. Read a Zephyr console's -110 as ESHUTDOWN (the driver's
   stopped render service) and -116 as `ETIMEDOUT`.

## Status

Real (works against the headers; compiles clean under `-Wall -Wextra`):
- The whole `osal_ops` surface for Zephyr, `linux.c` and bare metal, and the
  `mod.rs` glue.
- The Zephyr OSALs drive the GPUs on silicon, built as a Zephyr module
  (gl_test, dEQP, glmark2: `tools/cts/RESULTS.md`). The PowerVR one forwards
  `init` -> `pvr_render_init()`, `gpu_alloc` -> `pvr_render_alloc()` (heap
  chosen by the base VA), `submit`/`wait` -> `pvr_render_submit/wait()`,
  `query` -> `pvr_render_static_get()`; the driver exports it needs live in
  `driver/src/pvr_drv.h` and `driver/src/pvr_render.h`, and `pvr_fw_boot()`
  does not render its self-test when built with `-DPVR_NO_SELFTEST_RENDER`
  (the driver's Zephyr build sets it). `zephyr.c` compiles against the
  Zephyr cache/kernel/sys_io/log APIs of the pinned Zephyr.
- CMake and `baremetal.mk` produce `libyakogl.a` (the crate's static
  library with the OSAL and the other C objects appended); the Zephyr module
  links the same crate.

On the host the port is checked two ways: unit tests on the strict mocks of
`src/testutil.rs` (`cargo test --no-default-features --features host-test,gpu-powervr`), which model
this contract — a CPU view and device memory kept apart, synchronised only by
`cache_flush` / `cache_invd` (both recorded, ranges checked against the live
allocation), fresh memory filled with 0xA5, allocations that can come without a CPU
mapping, injectable `gpu_alloc` / `gpu_write` / `gpu_read` failures, double and foreign
frees reported, and a kick model that catches a draw whose inputs move before its kick
completes; and the host differential harness (`tools/hostharness/`, whose README lists
what is and is not compared): `driver/tools/pvrlab/gl_test.c`, API sweeps and hostile
sequences over a recording OSAL (every `struct osal_ops` entry, GPU memory hashes,
decoded kicks) and an interposer on the exported entry points, with
AddressSanitizer on the library itself. Nothing is rendered. The
harness's recording OSAL (`tools/hostharness/rec.c`) is a complete platform of its own;
its `HH_BASE=1` mode allocates through `linux.c`. Small OSAL parts written as
plain C headers (`osal_bounds.h`, `osal_log.h`, `pending_frees.h`,
`gc_frees.h`, `gc_cache_range.h`, `v3d_frees.h`, `v3d_granules.h`,
`linux_heap.h`) are checked directly by the host programs under
`tools/osal/`, `tools/gc/` and `tools/v3d/` named in the table above. The
host harness that once compiled `zephyr.c` in a Linux kernel module through
a Zephyr API shim is retired (`driver/tools/pvrlab/README.md`).

Fresh GPU storage: the Zephyr OSALs hand out pool memory with the previous
user's contents, while `linux.c` and `baremetal.c` zero each allocation. GL
leaves new storage undefined, but a leak between UI elements does not belong
in a stack, so the core clears fresh storage before the application can
observe it (`src/gl/context.rs`); reused storage is not cleared, so
steady-state streaming pays nothing.

Bare-metal fails closed. The OSAL has no address and no hook of its own: the
board fills a `struct osal_bm_board` (`include/gles_osal_baremetal.h`: register
base, carveout physical base, CPU address and size, the render pool's offset in
it, and the `boot`, `mmu_map`, `cache_clean` and `cache_invalidate` hooks) and
calls `osal_bm_board_config()` before the first GL/EGL call. Init checks the
struct before it uses any of it: no board, or a required field left 0 / NULL
(also an unaligned carveout, a pool offset not below the size, a carveout that
wraps), returns -EINVAL with one log line `yakogl baremetal: init refused:
<field>`, and no address is touched. Every other op refuses before init
(`reg_read` gives 0 and `reg_write` is dropped, each with a log line). There
are no default addresses and no do-nothing hooks. No board in this
repository uses the bare-metal OSAL; the host test
`tools/osal/baremetal_config_check.c` links it unchanged against a fake
board. Its allocator is a bump over the carveout that never takes memory
back, so it has no pending-free queue. `submit`, `wait` and `query` still
return -ENOTSUP: the render service is not ported (that port adds a
KCCB-post hook). `baremetal.mk` builds it for the PowerVR or the Vivante
backend.

## Open points in the public header

`struct osal_ops` and `struct osal_gpu_mem` are sufficient to implement the
OSAL. Observations:
- `include/gles_osal.h` has no unmap or mmu_unmap op: an allocator that hands
  a VA range to a different mapping would need one.
- The opaque `kick_desc` requires an out-of-band shape agreement per GPU —
  provided by `osal_internal.h` (see the contract section above).
- `osal_gpu_mem.impl` is the platform's handle: the GL core reads a NULL
  `impl` as "no memory" (`include/gles_osal.h`), so an OSAL sets it
  non-NULL for a live allocation or mapping and clears it on free.

## Build invocations

```sh
# Needs cargo; cross builds also `rustup target add aarch64-unknown-none`.
# Host / Linux / generic (standalone) → libyakogl.a
cmake -S yakogl -B build/gles -DYAKOGL_OSAL=baremetal   # or zephyr|linux (default linux)
      # -DYAKOGL_GPU=powervr|vivante|v3d (default powervr; v3d with linux only)
      # cross: -DYAKOGL_RUST_TARGET=aarch64-unknown-none   (default: the host)
cmake --build build/gles                                  # → build/gles/libyakogl.a

# Zephyr module (CONFIG_YAKOGL): add yakogl to the module list, e.g.
west build ... -- -DZEPHYR_EXTRA_MODULES=".../yakogl" -DCONFIG_YAKOGL=y
# (pick the GPU with CONFIG_YAKOGL_GPU_*; for the PowerVR the pvr driver,
#  CONFIG_YAKOGL_DRV_GPU_PVR, must be in the same build for submit/mmu symbols;
#  Rust target aarch64-unknown-none by default, hard float: CONFIG_FPU=y and
#  CONFIG_FPU_SHARING=y; override with -DYAKOGL_RUST_TARGET=<triple>)


# Bare-metal cross build → libyakogl.a
# (RUST_TARGET=aarch64-unknown-none and CARGO_PROFILE=release are the defaults)
# The board's addresses and hooks are given at run time: osal_bm_board_config().
make -f yakogl/baremetal.mk CROSS_COMPILE=aarch64-none-elf- [GPU=powervr|vivante] BUILD=build/bm
```
