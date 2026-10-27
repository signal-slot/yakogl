/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * osal_internal.h - private contract shared between the OSAL backends
 * (src/osal) and the PowerVR command backend (src/pvr).
 *
 * This is NOT part of the public portability interface (gles_osal.h); it holds
 * the things the backends and the OSAL must agree on but which the opaque
 * public interface deliberately hides:
 *
 *   1. The shape of the "kick descriptor" gles_osal.h's submit() takes as
 *      `const void *kick_desc`, which is per GPU: `struct pvr_render_kick`
 *      from the driver's render service (src/pvr_render.h), the per-render
 *      register blocks the PVR backend generates, or `struct gc_kick`
 *      (src/vivante/gc_kick.h), a GC7000 command stream and its event id, on
 *      a YAKOGL_GPU_VIVANTE build. The OSAL forwards it to the driver; it
 *      never interprets the contents itself.
 *
 *   2. The OSAL_QUERY_* ids for gles_osal.h's query() op.
 *
 *   3. The g_osal selection glue (osal_set_ops + the per-platform registrars).
 */
#ifndef OSAL_INTERNAL_H
#define OSAL_INTERNAL_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include "gles_osal.h"

#ifdef __cplusplus
extern "C" {
#endif

/* ---- submit/wait contract (backend <-> OSAL) ------------------------- *
 * submit(kick_desc, kick_desc_len): kick_desc points at the GPU's kick
 * descriptor (above) and kick_desc_len is its size. The returned fence stands
 * for that submit's completion; wait() polls it (and services firmware
 * requests meanwhile on the PowerVR). One render in flight at a time for
 * bring-up. */

/* ---- query() ids ------------------------------------------------------ */
/* out = struct pvr_render_static (src/pvr_render.h): the EOT program's DOUTU
 * control, the border colour table VA and the device-VA heap bases. */
#define OSAL_QUERY_RENDER_STATIC 1u
/* out = uint32_t: the V3D's VPM in bytes, as the OSAL identified it
 * (IDENT1.VPM_SIZE * 8192, driver/src/v3d/v3d_hw.c v3d_identify). The V3D
 * backend's init compares it with the size the online compiler links programs
 * for (codegen::V3D_VPM_BYTES) and refuses to start on another. */
#define OSAL_QUERY_V3D_VPM_BYTES 2u
/* in/out = struct osal_v3d_cpu_of: the CPU view of `bytes` of V3D memory at
 * GPU address `gpu` (the caller fills both; the OSAL fills `cpu`), or
 * -EINVAL when the range is not wholly in the OSAL's memory. The V3D backend
 * reads a texture's texels through it to tile them (the TMU does not sample
 * the GL core's linear levels). */
#define OSAL_QUERY_V3D_CPU_OF_GPU 3u
struct osal_v3d_cpu_of {
	uint64_t gpu;
	uint64_t bytes;
	void *cpu;
};
/* out = struct osal_v3d_bin_pool: the binner's tile-memory pool as the last
 * job that binned without error left it, the registers read raw right after
 * its binning completed (V3D_PTB_0_BPCA/BPCS, V3D_CLE_0_CT0QMA/QMS; driver/src/
 * v3d/v3d_regs.h), next to the pool the job was given. -ENODATA before the
 * first such job. The V3D backend derives the pool's use from them and
 * prints its high-water (docs/online-shaderc/v3d-backend.md, "M6 job
 * arenas"). */
#define OSAL_QUERY_V3D_BIN_POOL 4u
/* out = uint32_t: the draws one V3D job is sized for
 * (CONFIG_YAKOGL_V3D_DRAWS_PER_JOB on Zephyr); the backend derives its job
 * arenas from it, and uses its default without an answer. */
#define OSAL_QUERY_V3D_DRAWS_PER_JOB 5u
/* out = uint32_t: the bytes of tiled texture copies the V3D backend may keep
 * across jobs (CONFIG_YAKOGL_V3D_TILED_COPIES_BYTES on Zephyr); the backend
 * uses its default without an answer. */
#define OSAL_QUERY_V3D_TILED_BUDGET 6u
/* in = uint32_t (passed through the out pointer): 1 when the V3D backend
 * opens a job that may reference GPU memory, 0 when it has none open. The
 * OSAL then gives freed memory back at once when no job is open, and holds it
 * for the job being recorded otherwise (src/osal/v3d_frees.h). An OSAL that
 * does not answer keeps holding every free for the next job. */
#define OSAL_QUERY_V3D_JOB_OPEN 7u
/* out = uint32_t: 1 when the V3D backend keeps depth and stencil apart, each
 * by its own renderbuffer (CONFIG_YAKOGL_V3D_SPLIT_ZS on Zephyr; the
 * backend's v3d_split_zs), read at init; 0 or no answer keeps them in one
 * buffer. */
#define OSAL_QUERY_V3D_SPLIT_ZS 8u
/* in = uint32_t (passed through the out pointer): 1 when the GC7000 backend
 * opens a frame (starts recording a command stream that may reference GPU
 * memory), 0 once it has submitted it. The OSAL then holds a free for the
 * frame being recorded (submitted + 1) only while one is open, and otherwise
 * only for the last submitted kick, which may still be running
 * (src/osal/zephyr_gc.c). An OSAL that does not answer holds every free for
 * the next kick, which never comes while nothing draws: the i.MX8MP's dEQP
 * texture.size ran out of the carveout that way (tools/cts/RESULTS.md). */
#define OSAL_QUERY_GC_FRAME_OPEN 9u
/* out = uint64_t: a monotonic clock in nanoseconds (every Zephyr OSAL: the
 * 64-bit cycle counter, zephyr_timing.c). The V3D backend times its CPU
 * texture tiling with it (v3d_tiling_time), the GL core and the GC7000
 * backend the steps of a draw (src/draw_steps.rs); without an answer each
 * counts its time as not measured, never as zero. */
#define OSAL_QUERY_CLOCK_NS 10u
/* out = uint32_t: the smallest tiled image, in bytes, the V3D backend hands
 * to the texture formatting unit instead of tiling it on the CPU
 * (CONFIG_YAKOGL_V3D_TFU_MIN_BYTES on Zephyr), read at init. Answered only
 * where the OSAL runs TFU jobs (CONFIG_YAKOGL_V3D_TFU); no answer, or 0,
 * keeps every tiling on the CPU. */
#define OSAL_QUERY_V3D_TFU_MIN_BYTES 11u
/* in/out = struct osal_v3d_tfu: run one TFU job with these registers
 * (driver/src/v3d v3d_tfu_run, polled) and return 0 with its result in `rc`
 * and its time in `us`; -ENOTSUP where the OSAL runs no TFU jobs. */
#define OSAL_QUERY_V3D_TFU_RUN 12u
/* out = uint32_t: 1 when the V3D backend keeps each stencil half in an S8
 * buffer (1 byte a pixel) instead of a D24S8 one (CONFIG_YAKOGL_V3D_S8_STENCIL
 * on Zephyr; the backend's V3dState::s8_stencil), read at init, with the
 * split halves only; 0 or no answer keeps D24S8. */
#define OSAL_QUERY_V3D_S8_STENCIL 13u
/* in = struct osal_draw_stats (passed through the out pointer): the time one
 * timed scope of the library spent in each step of a draw
 * (src/draw_steps.rs: the GL core's build of a draw, a backend's draw),
 * added to the OSAL's totals, which an application reads with
 * osal_draw_stats. -ENOTSUP where the OSAL keeps none. */
#define OSAL_QUERY_DRAW_STATS 14u
/* The Mali-G610's two (a gpu-mali OSAL only): out = struct osal_mali_gpu,
 * the device's counts, and out = struct osal_mali_heap, the tiler heap the
 * OSAL made. submit() takes a struct osal_mali_kick on that build. The Rust
 * side is src/osal/mali.rs; abi_layout_check.c pins both. */
#define OSAL_QUERY_MALI_GPU 15u
#define OSAL_QUERY_MALI_TILER_HEAP 16u
/* in/out = struct osal_v3d_tfu_queue: run one TFU job after the job of fence
 * `after` (0: none) without the caller waiting for that job, as Mesa's v3d
 * queues its TFU ioctl with an in-sync on the render's out-sync. The OSAL
 * runs one job at a time: with a job in flight the TFU job is kept and run
 * when the OSAL finishes that job (wait(), completed() or the next submit),
 * after the render and before its fence completes, so a wait on `fence` or a
 * later submit comes after the TFU; `fence` is then that job's fence and
 * tfu.rc 0. With none in flight it runs within the call: `fence` 0 and
 * tfu.rc its result. Returns -EBUSY when the OSAL holds as many queued jobs
 * as it keeps (the caller waits for the job in flight and asks again),
 * -ENOTSUP where it runs no TFU jobs. A queued job that fails is the OSAL's
 * to log and recover from; the job's fence keeps the render's result. */
#define OSAL_QUERY_V3D_TFU_QUEUE 17u
/* The steps, in src/draw_steps.rs Step order; record.rs numbers them from
 * CHECK_RUN_STEP_GL_PROGRAM. */
#define OSAL_DRAW_STEPS 12u
struct osal_draw_stats {
	uint64_t ns[OSAL_DRAW_STEPS];
	uint32_t untimed; /* laps the clock could not time: not measured, not zero */
	uint32_t reserved;
};
struct osal_v3d_tfu {
	uint32_t icfg, iia, iis, ica, iua, ioa, ios, coef0;
	int32_t rc;
	uint32_t us;
};
struct osal_v3d_tfu_queue {
	struct osal_v3d_tfu tfu; /* the job; out: rc and us when it ran in the call */
	int64_t after;           /* in: the fence of the job it runs after, 0: none */
	int64_t fence;           /* out: the fence that completes after it, 0: it ran */
};
struct osal_mali_gpu {
	uint32_t gpu_id;         /* GPU_ID */
	uint32_t tiler_features; /* TILER_FEATURES (bits 11:8 max_levels) */
	uint32_t cs_reg_count;   /* registers of one command stream */
	uint32_t unpreserved_cs_reg_count; /* of those, the kernel's */
};
struct osal_mali_heap {
	uint64_t ctx_gpu;         /* the heap context HEAP_SET names */
	uint64_t first_chunk_gpu; /* the first chunk */
	uint32_t chunk_size;
	uint32_t reserved;
};
struct osal_mali_kick {
	uint64_t stream_gpu;   /* the stream's first word, 64-byte aligned */
	uint32_t stream_bytes; /* a non-zero multiple of 8 */
	uint32_t reserved;
};
struct osal_v3d_bin_pool {
	uint32_t jobs;          /* jobs binned without error so far */
	uint32_t pool, pool_size; /* what the job gave CT0QMA/CT0QMS */
	uint32_t bpca, bpcs;    /* V3D_PTB_0_BPCA/BPCS after binning */
	uint32_t qma, qms;      /* V3D_CLE_0_CT0QMA/QMS after binning */
	/* the binner's overflow pool: blocks this job took (each time the
	 * pool ran out, OUTOMEM), the blocks there are, and their size. With
	 * overflow_used > 0, BPCA points into the last block taken, not into
	 * the pool. */
	uint32_t overflow_used, overflow_blocks, overflow_block_size;
};

/* ---- Zephyr init argument ------------------------------------------- *
 *
 * Passed as gles_osal.h init()'s `platform_arg` on Zephyr. The pvr bring-up
 * driver (src/pvr_bringup.c) has already powered the GPU, mapped the register
 * block and booted the firmware (pvr_fw_boot) at DT init; init() only brings
 * up the driver's render service on top. Pass NULL for the defaults.
 */
struct osal_zephyr_init {
	uintptr_t regs;          /* mapped GPU register base (0 = as recorded at boot) */
};

/* ---- Zephyr init argument, V3D (Raspberry Pi 4) ------------------------ *
 *
 * Passed as gles_osal.h init()'s `platform_arg` -- the EGL native display --
 * by an application on the Raspberry Pi 4 (src/osal/zephyr_v3d.c). The V3D's
 * clock is a VideoCore firmware clock switched through the property mailbox,
 * which is board support the OSAL does not own, so the application hands the
 * switch over (driver/src/v3d/v3d.h, struct v3d_clock_ops). NULL, or a NULL
 * `clock.set`, is refused: the power-on sequence needs it.
 */
struct osal_zephyr_v3d_init {
	int (*clock_set)(void *ctx, bool on);
	void *clock_ctx;
};
/* The platform the V3D OSAL uses when eglGetDisplay() gets none
 * (EGL_DEFAULT_DISPLAY): NULL by default (weak), the firmware clock switch
 * when the board layer is in the image (driver/zephyr/lib/board/rpi4), so a
 * sample written for every board passes no board argument. */
const struct osal_zephyr_v3d_init *osal_zephyr_v3d_default_platform(void);

/* The V3D OSAL's job record, for a sample that prints one at the end of a
 * run (v3d_gl_test's frozen record): the command-list registers after the
 * first job that completed and at the first that did not (or "none"), the
 * number of jobs that timed out, and UIFCFG as read at init. printk only. */
void osal_v3d_diag_print(void);
/* The Zephyr PowerVR (zephyr.c) and GC7000 (zephyr_gc.c) OSALs' close
 * record, for a sample that prints one at the end of a run (gl_test,
 * gc_gl_test): the pending-free queue's line, "pending frees: high-water
 * H/CAP, waits W (opt-in O), refused R (...)", as the V3D's
 * osal_v3d_diag_print prints it. It only prints: the OSAL stays up (the
 * shutdown op prints the same line but also tears the OSAL down, and the
 * product never calls it while the backend keeps its memory). printk. */
void osal_zephyr_diag_print(void);

/* The GC7000 OSAL's waits for kicks so far (zephyr_gc.c): how many, how many
 * timed out (the 2 s bound), and the time spent in them and in the longest
 * one. Read around a frame to tell GPU time from CPU time. */
void osal_gc_wait_stats(uint32_t *waits, uint32_t *timeouts, uint64_t *wait_us, uint64_t *max_us);
/* The GC7000 OSAL's time inside submits (which block while every event is in
 * flight), and the driver's GPU busy time and completed kicks
 * (gc_render_perf), so far. */
void osal_gc_gpu_stats(uint64_t *submit_us, uint64_t *gpu_busy_us, uint32_t *kicks);
/* The library's glog_once! counts (src/osal/mod.rs) into buf[cap],
 * NUL-terminated: "log: C causes logged once, R repeats not printed". Each
 * Zephyr OSAL's close print prints it. */
void yakogl_log_repeats_report(char *buf, size_t cap);
/* The draw-step totals the library reported so far (OSAL_QUERY_DRAW_STATS;
 * every Zephyr OSAL, zephyr_timing.c): ns per step and the laps it could not
 * time, either pointer may be NULL. Read around a frame and take the
 * difference. False when nothing has been reported (no clock, or no draw
 * yet): not measured, never zero. */
bool osal_draw_stats(uint64_t ns[OSAL_DRAW_STEPS], uint32_t *untimed);
/* The answers zephyr_timing.c gives for each Zephyr OSAL's query op:
 * OSAL_QUERY_CLOCK_NS and OSAL_QUERY_DRAW_STATS; -ENOTSUP for any other. */
int osal_zephyr_timing_query(uint32_t what, void *out, size_t out_sz);
/* One line naming this OSAL's GPU memory now, NUL-terminated into
 * buf[n]: what is allocated, the frees it holds, and how large an
 * allocation can still succeed. Each Zephyr OSAL its own wording and
 * prefix ("osal(v3d) mem:", "osal(zephyr) mem:", "osal(gc) mem:"); a value
 * an OSAL cannot read is written "absent", never 0. For a sample that
 * prints it at intervals (glmark2's "glmark2-mem:" line at every scene
 * change). Only formats: no allocation, no printk. */
void osal_mem_line(char *buf, size_t n);
/* Jobs whose binning ran the overflow pool dry (-ENOMEM, "binner out of
 * memory"), since init: for gl_test case 124 binovf's control. */
uint32_t osal_v3d_bin_oom_jobs(void);

/* ---- GPU-MMU L0 (leaf) PTE flags ------------------------------------ *
 * Mirrors pvr_fw_boot.c:1201-1207. Plain GL data/texture/vertex buffers use
 * SLC_BYPASS only (matches Mesa PVR_BO_ALLOC_FLAG_GPU_UNCACHED); PM_META_PROTECT
 * is reserved for PM freelist backing (not allocated through this GL path). */
#define OSAL_L0_SLC_BYPASS       0x0000000000000008ULL
#define OSAL_L0_PM_META_PROTECT  0x4000000000000000ULL
#define OSAL_L0_FLAGS_DATA       OSAL_L0_SLC_BYPASS

/* ---- g_osal selection glue (defined in src/osal/mod.rs) ---------------------- */
void osal_set_ops(const struct osal_ops *ops);

/* Per-platform ops tables + registrars (each defined in its own TU). Only the
 * ones compiled in are linked; the CMake/Kconfig picks exactly one backend. */
#if defined(YAKOGL_OSAL_ZEPHYR)
extern const struct osal_ops osal_zephyr_ops;
void osal_zephyr_register(void);
#endif
#if defined(YAKOGL_OSAL_BAREMETAL)
extern const struct osal_ops osal_baremetal_ops;
void osal_baremetal_register(void);
#endif
#if defined(YAKOGL_OSAL_LINUX)
extern const struct osal_ops osal_linux_ops;
void osal_linux_register(void);
#endif

#ifdef __cplusplus
}
#endif

#endif /* OSAL_INTERNAL_H */
