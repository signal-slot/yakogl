# YakoGL OSAL — implementation notes

The OSAL (layer 5b, `gles_osal.h`) is the only place the GL core and the GPU
backend touch the platform. One C translation unit per platform implements
`struct osal_ops`; `mod.rs` (Rust, part of the crate `yakogl`) provides the single
`g_osal` pointer and selection glue. The OSALs stay C on purpose: they are thin glue
over C APIs (Zephyr macros and inlines, board hook pointers, a variadic `log`) that
stable Rust cannot define.

Zephyr has one such translation unit per GPU, because the driver underneath is
a different driver: `zephyr.c` for the PowerVR BXS-4-64 and `zephyr_gc.c` for
the Vivante GC7000UL. Both define `osal_zephyr_ops`, the module build compiles
exactly one of them (`CONFIG_YAKOGL_GPU_*`), and nothing above the OSAL knows
which.

## Files (all under `yakogl/src/osal/`)

| File              | Role                                                           |
|-------------------|---------------------------------------------------------------|
| `osal_internal.h` | Private backend↔OSAL contract, the C view: `struct osal_kick_desc`, the Zephyr init arg, GPU-MMU L0 PTE flags, and the `g_osal` selection decls. Not part of the public interface. Its Rust mirror is `mod.rs` / `render.rs`; the generated layout pins (`src/abi_layout.rs` / `src/abi_layout_check.c`, `yakogl-tools abi-layout`) fail the build when the two drift. |
| `zephyr.c`        | Zephyr backend for the PowerVR — wraps the silicon-proven bring-up driver. |
| `zephyr_gc.c`     | Zephyr backend for the Vivante GC7000UL — wraps the driver's render service (`src/vivante/gc_render.h`). Compiled instead of `zephyr.c` when `CONFIG_YAKOGL_GPU_VIVANTE=y`. |
| `baremetal.c`     | Bare-metal backend skeleton — direct MMIO + static carveout; proves the portability seam (2nd backend, same interface). The board registers its addresses and hooks first (`include/gles_osal_baremetal.h`, `osal_bm_board_config()`); init refuses without them (see below). |
| `linux.c`         | Host backend with no GPU: malloc'd GPU memory, fake device VAs per heap, submits complete at once. Runs the GL core + PowerVR backend + shader table on a Linux host under ASan/valgrind with a real application's call stream (`-DYAKOGL_OSAL=linux`); the host harness `tools/hostharness/` links it (its recording OSAL can allocate through it). Renders nothing. |
| `osal_bounds.h`   | `osal_gpu_copy_ok()`: the one argument check every OSAL's `gpu_write` / `gpu_read` makes before its `memcpy` (NULL `mem`, `mem->cpu` or buffer, and `off > size || n > size - off`, a form in which `off + n` cannot wrap). The contract is in `gles_osal.h`; the host checks it with `tools/osal/osal_bounds_check.c` (a `tools/check.sh` row). |
| `mod.rs`          | `struct osal_ops` / `struct osal_gpu_mem` as `#[repr(C)]` (`OsalOps`, `OsalGpuMem`); defines `g_osal` (initial value = the ops table of the OSAL the cargo feature `osal-zephyr` / `osal-baremetal` / `osal-linux` names; null with none, then the application calls `osal_set_ops()` first) + `osal_set_ops()`; the wrappers the core uses and the `glog!` macro (`g_osal->log("%s", line)`). |
| `render.rs`       | `#[repr(C)]` mirror of the driver's `pvr_render.h` types (`struct pvr_render_kick`, `struct pvr_render_static`): the kick descriptor `submit` takes and what `query` fills. |

Build files: `yakogl/CMakeLists.txt` (host/Linux/generic → `libyakogl.a`),
`yakogl/zephyr/{CMakeLists.txt,Kconfig,module.yml}` (Zephyr module,
`CONFIG_YAKOGL`), `yakogl/baremetal.mk` (cross-compile → `libyakogl.a`). Each runs
cargo for the crate with the matching `osal-*` feature and compiles the one OSAL TU
(plus the layout pins `src/abi_layout_check.c` and `src/pvr/ffi_layout_check.c` and, outside Zephyr, the driver's command
generators). CMake and `baremetal.mk` append the C objects to the cargo static library
with `ar`; Zephyr links the cargo static library next to the `zephyr_library` that
holds `zephyr.c`.

## The submit/wait contract (`struct osal_kick_desc`)

`gles_osal.h` `submit()` takes an opaque `const void *kick_desc`. The concrete
shape is `struct osal_kick_desc` (in `osal_internal.h`) = the driver's
`struct pvr_render_kick` (`src/pvr_render.h`): the per-render register blocks the
backend generates (VDM stream base, PPP control, ISP/PBE/background-object
registers). The Zephyr OSAL hands it to the driver's render service
(`pvr_render_submit`), which owns the firmware objects (render context, client
CCBs, freelists, HWRT datasets per target size) and the KCCB kick + UFO fence;
`wait()` polls that fence (`pvr_render_wait`). No FWIF type crosses the OSAL.

On a `YAKOGL_GPU_VIVANTE` build the descriptor is `struct gc_kick`
(`src/vivante/gc_kick.h`) instead: the GPU address and length of a command
stream. The GC7000 has no firmware, so there is nothing to hand a register
block to — the OSAL gives the stream to `gc_render_submit()`, which appends the
hardware's epilogue (cache flushes, the FE/PE semaphore pair, the `EVENT`),
allocates the event and returns a monotonic fence.

## How the Zephyr OSAL maps onto the driver

| osal op        | Driver plumbing wrapped                                        | where |
|----------------|----------------------------------------------------------------|-------|
| `init`         | GPU already powered/booted by the pvr DT driver (`pvr_bringup.c` → `pvr_fw_boot`); `pvr_render_init()` builds the render service objects once | `src/pvr_render.c` |
| `gpu_alloc`    | `pvr_render_alloc()`: page-aligned render-pool span, mapped at the next free VA of the heap named by the base VA (GENERAL / PDS / USC) | `src/pvr_render.c`, `pvr_gpu_bo_alloc` (`src/pvr_drv.h`) |
| `gpu_free`     | Deferred: the block joins a pending list tagged with the kick being recorded (`s_last_submitted + 1`) and goes back to the render pool's free list (`pvr_render_free`) once a fence at or past that kick has completed (`drain_pending_frees`, run from `wait` and `completed`). A GL object freed while the GPU still reads it is therefore safe. | `src/pvr_render.c` |
| `mmu_map`      | `pvr_mmu_map` for an explicit VA→phys range                    | `src/pvr_drv.h` |
| `map_phys`     | `pvr_render_map_phys`: caller-owned pages (scan-out) at a fresh heap VA, no pool memory | `src/pvr_render.h` |
| `gpu_write`    | `memcpy` into the CPU-mapped carveout + `sys_cache_data_flush_range` | — |
| `gpu_read`     | `sys_cache_data_invd_range` + `memcpy` out                     | — |
| `cache_flush`  | `sys_cache_data_flush_range`                                   | — |
| `cache_invd`   | `sys_cache_data_invd_range`                                    | — |
| `submit`       | `pvr_render_submit(kick)`: GEOM + FRAG client-CCB commands, MMU cache flush, two KCCB kicks, returns the fence value | `src/pvr_render.c` |
| `wait`         | `pvr_render_wait(fence, ms)`: UFO poll + FWCCB servicing + HWR detection | `src/pvr_render.c` |
| `completed`    | `pvr_render_completed()`: the highest fence the GPU has finished (the fragment UFO), without blocking. The backend uses it to reuse a frame set only after its kick is done, and to drain pending frees. Optional: `NULL` means "nothing is known to be complete before `wait` says so" (bare metal). | `src/pvr_render.c` |
| `query`        | `pvr_render_static_get()`: EOT program, border table, heap bases | `src/pvr_render.h` |
| `reg_read/write` | `sys_read32/sys_write32` on the register base recorded at boot | `pvr_regs()` |
| `log`          | `vsnprintk` → `LOG_INF`                                        | — |

### The Vivante variant (`zephyr_gc.c`)

| osal op | Driver plumbing wrapped (`src/vivante/gc_render.h`) |
|---------|------------------------------------------------------|
| `init` | `gc_render_init(DEVICE_DT_GET_ONE(vivante_gc))`: hardware init, MMU v2 page tables, the FE's idle loop, the interrupt. Idempotent. |
| `gpu_alloc` | `gc_render_alloc(size, 64, ...)`. 64 is the strictest alignment the hardware asks for (command buffers and shader code), so one value covers every request. `heap_base` is ignored: the carveout is identity-mapped, so an allocation's address is its physical address. |
| `gpu_free` | `gc_render_free()`, **deferred**: the block is held on a pending list tagged with the sequence of the kick being recorded, and goes back to the pool only once a fence at or beyond it has completed (drained on every free and every submit). The pool has a real free list, so memory handed back early is handed straight out again — to a caller that writes it while the GPU is still reading it. This is the same list `zephyr.c` keeps for the PowerVR. |
| `mmu_map` | **absent.** The service maps at a VA it chooses; `map_phys` is the entry point, and `gl_egl.c` prefers it anyway. |
| `map_phys` | `gc_render_map_phys()`. A range already inside the carveout comes back with its identity VA and the carveout's own CPU view — which is what `glReadPixels` on a window surface reads through. |
| `gpu_write` / `gpu_read` | `memcpy` + `gc_render_cache_flush` / `_invd` (no-ops on the carveout's Normal non-cacheable mapping, correct if a cached buffer is ever handed in). |
| `submit` | `gc_render_submit(&(struct gc_submit){...}, &fence)` with `GC_SUBMIT_FLUSH_ALL`: the backend does not say which caches its frame dirtied, and a missing flush shows a stale picture rather than an error. |
| `wait` | `gc_render_wait(fence, ms)`. `timeout_us < 0` becomes a bounded 2 s, not forever: a board booted by kexec has no way back except its own reset, so a stalled GPU has to become a failed sample. `-ETIMEDOUT` becomes the contract's `1`; the driver has already dumped the hardware state. |
| `completed` | `gc_render_completed()`. |
| `query` | `-ENOTSUP`. `OSAL_QUERY_RENDER_STATIC` describes PowerVR firmware objects. |
| `reg_read/write` | **absent.** This backend writes a command stream and never touches a register. |

## Locks and thread ids on the Linux OSAL

`lock` / `unlock` are a `pthread_mutex_t` in `linux.c`. The library takes the pair around each EGL entry point and nowhere
else, so the mutex does not have to be recursive - and must not be relied on
to be. It does not make a second thread safe: one thread issues every GL and
EGL call of the process (`gles_osal.h`, "threads"), no GL entry point takes
the mutex, and the library keeps no per-thread state.

`thread_id` is how the library catches a second thread. The Linux OSAL
(`linux.c`) returns the kernel thread id, kept in a `__thread` variable after the first
`gettid` (one TLS load per call, no system call); the three Zephyr OSALs return
`k_current_get()` folded to 32 bits; `baremetal.c` leaves it NULL (one flow of
control, nothing to tell apart). `zephyr.c` and `baremetal.c` leave `lock` /
`unlock` NULL, which costs those platforms nothing.

## The counters at the end of a run: the report is not shutdown

Every OSAL keeps counters a run should end with: the pending-free queue's
line ("pending frees: high-water H/CAP, waits W (opt-in O), refused R ...",
`pending_frees.h`) and, in `linux.c`, the heaps' lines. Two different things
print them:

- **shutdown** (the ops table's `shutdown`) tears the OSAL down: it prints,
  then zeroes the heaps (`linux.c`) or marks the OSAL down so the next `init`
  runs again (the Zephyr OSALs). The product never calls it: `eglTerminate`
  keeps the backend and its GPU memory for the next `eglInitialize`
  (`pvr_backend_init` / `gc_init` return early when already up), so a
  shutdown there would leave the backend drawing through memory that is gone.
- **the report** only prints and changes nothing. Zephyr:
  `osal_zephyr_diag_print()` (`zephyr.c`, `zephyr_gc.c`) and
  `osal_v3d_diag_print()` (`zephyr_v3d.c`), called by the sample at the end
  of its run (gl_test, gc_gl_test, v3d_gl_test). Linux: a
  destructor in `linux.c` prints the same lines at
  process exit, prefixed "at exit:", when the OSAL came up.

## errno numbers at the C boundaries

A negative errno is a number, and the number for a name depends on the C
library. The ones our code uses that differ (Zephyr's minimal libc and
picolibc agree with each other, zephyr af17c0c8aa2a; Linux is
`asm-generic/errno.h`, and glibc's `ENOTSUP` is `EOPNOTSUPP`):

| name | Zephyr (minimal, picolibc) | Linux |
|---|---:|---:|
| `ESHUTDOWN` | 110 | 108 |
| `ETIMEDOUT` | 116 | 110 |
| `EMSGSIZE` | 122 | 90 |
| `ENOTSUP` | 134 | 95 |
| `ECANCELED` | 140 | 125 |

So Zephyr's `ESHUTDOWN` is Linux's `ETIMEDOUT`. The names below 35 and
`ENODATA` (61) are the same in both.

Where a number crosses from one side to the other, and which numbering each
side assumes (2026-10-04, main 8b53557f):

1. **The driver crate (`driver/rust`) to C.** `yakogl_drivers::errno` is
   Zephyr's numbering, written as constants. Its C callers (`driver/src/*.c`,
   `src/osal/zephyr*.c`, the Zephyr samples) compare against `errno.h`. In a
   Zephyr image both sides are Zephyr's, so they agree (for example
   `zephyr.c` `z_wait`: `rc == -ETIMEDOUT` on `pvr_render_wait`'s result).
   - The host harness `tools/pvr/km/record_c.c` links the crate under glibc
     and pins `ETIMEDOUT` to 116 by hand.
   - No Linux kernel module links the crate, so the Linux reading of a
     crate errno is latent, not live.
2. **The C OSAL to the YakoGL crate (`src/`).** `src/util.rs` has ONE set
   of constants, and it is Linux's (`ENOTSUP` 95, `ETIMEDOUT` 110), whatever
   the OSAL.
   - `linux.c` (glibc) agrees with it.
   - `baremetal.c` returns the literal -95.
   - The three Zephyr OSALs return Zephyr's numbers. They do not agree, and
     this is live: `zephyr.c`'s default `yakogl_zephyr_wait_vblank` returns
     -134, and `egl.rs` `eglSwapBuffers` tests `== -util::ENOTSUP` (-95).
     So on a Zephyr board without a vertical sync, "no vsync" is never
     seen. The loop then calls the hook `swap_interval` times instead of
     stopping.
   - A Zephyr OSAL's -134 that reaches `draw.rs` is reported as
     `GL_OUT_OF_MEMORY`, not as the backend gap (`GL_INVALID_OPERATION`).
   - The YakoGL crate's own -110 (`pvr/backend.rs`, "the fence did not
     complete") is printed in a Zephyr console, where -110 is `ESHUTDOWN`.
     The same number is what the driver's stopped render service returns
     on the freelist-grow branch (`-errno::ESHUTDOWN`). One printed value,
     two causes.
3. **Logs and records to the tools.** The tools print rc values as numbers
   and name none.

**The rule: a Rust crate uses the numbering of the C library on the other
side of its boundary, and a number is never translated in between.** There
is one table file, `driver/rust/src/errno_numbers.rs`, with both numberings
(`ZEPHYR`, `LINUX`) and each value's header line. Then:

- The driver crate's `errno` is `ZEPHYR`, its only C library. A Linux module
  that links it again must translate in its own shim, as `record_c.c` does.
- The YakoGL crate's `util::E*` comes from `errno_numbers::numbering(osal-zephyr)`:
  `ZEPHYR` for the Zephyr OSALs, `LINUX` for `linux`.
  `baremetal.c`'s literal -95 is `LINUX`'s, so `osal-baremetal` stays on
  `LINUX`.

The host test `zephyr_osal_reads_zephyr_numbers` (`src/util.rs`) asserts
what an `osal-zephyr` build gets: `ETIMEDOUT` is Zephyr's 116, not 110
(Zephyr's `ESHUTDOWN`), and `ENOTSUP` is 134. Before this change the YakoGL
crate had one numbering for every OSAL (`numbering` ignoring its argument),
and the test fails on exactly that (the known negative).

## Real vs TODO

Real (works against the headers; compiles clean under `-Wall -Wextra`):
- The whole `osal_ops` surface for both Zephyr and bare-metal, `mod.rs` glue.
- Zephyr `gpu_alloc/free/mmu_map/gpu_write/gpu_read/cache_*/reg_*/log`, the
  bump allocator (per-heap VA + shared carveout offset), and the submit/wait
  logic — all faithful to the proven path.
- CMake and `baremetal.mk` produce `libyakogl.a` (the crate's static library: portable
  core + PVR backend + shaderc offline, with the OSAL and the other C objects
  appended); the Zephyr module links the same crate. `zephyr.c` compiles against the
  Zephyr cache/kernel/sys_io/log APIs, each confirmed present in the pinned Zephyr.

Integration (done): the Zephyr OSAL forwards `init` -> `pvr_render_init()`,
`gpu_alloc` -> `pvr_render_alloc()` (the driver's bump allocator, heap chosen by
the base VA), `submit`/`wait` -> `pvr_render_submit/wait()`, `query` ->
`pvr_render_static_get()`; the driver exports it needs live in `src/pvr_drv.h`,
and `pvr_fw_boot()` no longer renders when built with `-DPVR_NO_SELFTEST_RENDER`.
Verified on silicon (with the C core) through the Linux harness (`tools/pvrlab`,
`GLES=1`), which compiles `zephyr.c` through its Zephyr API shim. That harness
compiled YakoGL's `.c` files directly (symlinks + Kbuild); with the core in Rust it
has to link the cargo-built static library instead, which is not done, and the Rust
library has not been run on silicon. Nor has the Zephyr module build itself been run:
the crate compiles for `aarch64-unknown-none` with the `osal-zephyr` feature, but the
`west build` that puts `zephyr.c`, the driver module and the cargo archive together is
untested. On the host the port is checked two ways: unit tests on the strict mocks of
`src/testutil.rs` (`cargo test --no-default-features --features host-test,gpu-powervr`), which model
this contract — a CPU view and device memory kept apart, synchronised only by
`cache_flush` / `cache_invd` (both recorded, ranges checked against the live
allocation), fresh memory filled with 0xA5, allocations that can come without a CPU
mapping, injectable `gpu_alloc` / `gpu_write` / `gpu_read` failures, double and foreign
frees reported, and a kick model that catches a draw whose inputs move before its kick
completes; and the host differential harness (`tools/hostharness/`, whose README lists
what is and is not compared): gl_test.c, API sweeps and hostile sequences over a
recording OSAL (all 17 operations, GPU memory hashes, decoded kicks) and an interposer
on all 222 exported entry points, in twelve platform modes, dev and release, with
AddressSanitizer on the library itself. Nothing is rendered. The
harness's recording OSAL (`tools/hostharness/rec.c`) is a complete platform of its own;
its `HH_BASE=1` mode allocates through `linux.c`.

One OSAL-visible behaviour deliberately differs from the C library: the core now clears
fresh GPU storage before the application can observe it, because `zephyr.c` and
`baremetal.c` hand out pool memory with the previous user's contents while `linux.c`
zeroes it (GL leaves the contents undefined, but the leak between UI elements does not
belong in a SafeUI stack). Reused storage is not cleared, so steady-state streaming pays
nothing. In a harness log against the C this shows up as one extra `cache_flush` per new
allocation. The same defect is still open upstream.

Bare-metal fails closed. The OSAL has no address and no hook of its own: the
board fills a `struct osal_bm_board` (`include/gles_osal_baremetal.h`: register
base, carveout physical base, CPU address and size, the render pool's offset in
it, and the `boot`, `mmu_map`, `cache_clean` and `cache_invalidate` hooks) and
calls `osal_bm_board_config()` before the first GL/EGL call. Init checks the
struct before it uses any of it: no board, or a required field left 0 / NULL
(also an unaligned carveout, a pool offset not below the size, a carveout that
wraps), returns -EINVAL with one log line `yakogl baremetal: init refused:
<field>`, and no address is touched. Every other op refuses before init
(`reg_read` gives 0 and `reg_write` is dropped, each with a log line). Until
2026-09-29 the hooks were weak do-nothing symbols and the addresses were
`OSAL_BM_*` defaults (0xFD000000, 0xC0000000), so a board that forgot them
initialised against those addresses (review finding #4). No board in this
repository uses the bare-metal OSAL; the host test
`tools/osal/baremetal_config_check.c` (a `tools/check.sh` row) links it
unchanged against a fake board. `submit`, `wait` and `query` still return
-ENOTSUP: the render service is not ported (that port adds a KCCB-post hook).

## Missing / suggested header fields

No blocking gaps — `struct osal_ops`, `struct osal_gpu_mem`, and
`struct shc_shader`/`gles_draw` are sufficient to implement the OSAL. Observations
(no edits made to the shared headers, per task):
- `gles_osal.h` has no `unmap`/`mmu_unmap`; fine for the bump allocator, needed
  once a reclaiming allocator lands.
- The opaque `kick_desc` requires an out-of-band shape agreement — provided here
  as `struct osal_kick_desc` in `osal_internal.h` (see the contract note above).
- `osal_gpu_mem.impl` is unused by the current bump allocator (left NULL); it is
  the natural handle for a future arena/free-list.

## Build invocations

```sh
# Needs cargo; cross builds also `rustup target add aarch64-unknown-none`.
# Host / Linux / generic (standalone) → libyakogl.a
cmake -S yakogl -B build/gles -DYAKOGL_OSAL=baremetal   # or zephyr|linux
      # cross: -DYAKOGL_RUST_TARGET=aarch64-unknown-none   (default: the host)
cmake --build build/gles                                  # → build/gles/libyakogl.a

# Zephyr module (CONFIG_YAKOGL): add yakogl to the module list, e.g.
west build ... -- -DZEPHYR_EXTRA_MODULES=".../yakogl" -DCONFIG_YAKOGL=y
# (the pvr driver, src/pvr_*.c, must be in the same build for submit/mmu symbols;
#  Rust target aarch64-unknown-none by default, hard float: CONFIG_FPU=y and
#  CONFIG_FPU_SHARING=y; override with -DYAKOGL_RUST_TARGET=<triple>)


# Bare-metal cross build → libyakogl.a
# (RUST_TARGET=aarch64-unknown-none and CARGO_PROFILE=release are the defaults)
# The board's addresses and hooks are given at run time: osal_bm_board_config().
make -f yakogl/baremetal.mk CROSS_COMPILE=aarch64-none-elf- BUILD=build/bm
```
