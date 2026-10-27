/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gles_osal.h - OS/platform abstraction for the portable PowerVR GL ES backend.
 *
 * This is the ONLY interface the GL core and PowerVR backend use to touch the
 * outside world. One implementation per platform (Zephyr, bare-metal, Linux,
 * PX5) provides these; the GL core links against the interface, never an OS API.
 *
 * Contract summary
 *  - GPU memory is physically-contiguous, CPU-mappable, and mapped into the GPU
 *    MMU at a caller-chosen device virtual address (heap-relative). The backend
 *    manages device-VA layout; the OSAL just backs it with pages and programs
 *    the MMU.
 *  - Submit is fire-and-forget into the firmware KCCB + doorbell; completion is a
 *    separate wait (poll/IRQ/fence) so the GL core can choose sync points.
 *  - Cache ops are explicit; the backend calls them where the render path needs
 *    them (the SLC is coherent between PBE and TPU, so RTT needs ordering, not a
 *    data-cache flush - see DESIGN.md).
 *
 * All functions are synchronous unless noted. Return 0 on success, negative
 * errno-style on failure, unless the signature says otherwise.
 */
#ifndef GLES_OSAL_H
#define GLES_OSAL_H

#include <stdint.h>
#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The longest line the core hands log(), NUL included (glog! in
 * src/osal/mod.rs formats into a buffer of this size; a test there pins the
 * two to each other). */
#define OSAL_LOG_LINE_MAX 256

/* A chunk of GPU-visible memory. cpu may be NULL if the platform cannot CPU-map
 * it (then the caller must use osal_gpu_write/read). gpu is the device VA the
 * backend chose; phys is informational. */
struct osal_gpu_mem {
	void     *cpu;   /* CPU-mapped address, or NULL */
	uint64_t  gpu;   /* GPU device virtual address */
	uint64_t  phys;  /* physical address (informational/debug) */
	size_t    size;  /* rounded-up allocation size in bytes */
	void     *impl;  /* platform bookkeeping handle */
};

/* Memory attributes for allocation/mapping. */
enum osal_mem_flags {
	OSAL_MEM_DATA      = 0,        /* normal GPU-readable/writable data */
	OSAL_MEM_CPU_CACHED = 1 << 0, /* CPU mapping is cached (needs flush/invd) */
	OSAL_MEM_GPU_RO    = 1 << 1,  /* GPU read-only hint */
	OSAL_MEM_UNCACHED  = 1 << 2,  /* CPU mapping uncached (coherent readback) */
	OSAL_MEM_GPU_EXEC  = 1 << 3,  /* the GPU executes it (shader code); ignored where all GPU memory is executable */
};

struct osal_ops {
	/* ---- lifecycle ---- */
	/* Power on, map registers, load FW, boot. Bare metal: the board must be
	 * registered first (gles_osal_baremetal.h), or init returns -EINVAL. */
	int  (*init)(void *platform_arg);
	void (*shutdown)(void);

	/* ---- GPU memory + MMU ---- *
	 * Allocate `size` bytes of GPU-visible memory in heap `heap_base` (a device-VA
	 * base the backend passes; the OSAL bump-allocates within it) and map it. */
	int  (*gpu_alloc)(uint64_t heap_base, size_t size, uint32_t flags,
			  struct osal_gpu_mem *out);
	/* Free `mem` (no-op for an empty one); `mem` is cleared either way.
	 * Returns 0 when the memory went back or is held until the kicks that
	 * may read it complete. <0 when the free was refused: the OSAL's
	 * pending-free queue was full and its bounded wait for the oldest kick
	 * failed (src/osal/pending_frees.h). The memory is then kept for good,
	 * counted by the OSAL; the caller counts and logs it too. */
	int  (*gpu_free)(struct osal_gpu_mem *mem);
	/* Map an explicit device VA -> phys range (for scan-out / imported buffers). */
	int  (*mmu_map)(uint64_t gpu_va, uint64_t phys, size_t size, uint32_t flags);
	/* Map physical memory the caller owns (a scan-out buffer) at a fresh
	 * device VA of heap `heap_base`, consuming no pool memory. out->cpu is
	 * NULL (the platform may not have a CPU view). Optional: without it the
	 * caller allocates a VA range with gpu_alloc and repoints it with mmu_map. */
	int  (*map_phys)(uint64_t heap_base, uint64_t phys, size_t size, uint32_t flags,
			 struct osal_gpu_mem *out);
	/* For platforms without a CPU mapping: bulk copy in/out of GPU memory.
	 * Copies `n` bytes between `src`/`dst` and [off, off + n) of `mem`.
	 * Returns -EINVAL and copies nothing when `mem` or mem->cpu is NULL, `src`/`dst` is
	 * NULL (also for n == 0), or the range leaves the allocation:
	 * off > mem->size || n > mem->size - off (a form in which off + n
	 * cannot wrap). n == 0 at off == mem->size is in range and copies
	 * nothing. Every OSAL makes this one check: src/osal/osal_bounds.h. */
	int  (*gpu_write)(struct osal_gpu_mem *mem, size_t off, const void *src, size_t n);
	int  (*gpu_read)(struct osal_gpu_mem *mem, size_t off, void *dst, size_t n);

	/* ---- cache maintenance (no-op where coherent) ---- */
	void (*cache_flush)(void *cpu, size_t n);   /* CPU writes -> memory (before GPU reads) */
	void (*cache_invd)(void *cpu, size_t n);    /* memory -> CPU (before CPU reads GPU writes) */

	/* ---- submit + completion ---- *
	 * Submit a prepared render (the backend has written all command streams to GPU
	 * memory and passes the kick descriptor). Returns a fence id (>=0) or negative. */
	int64_t (*submit)(const void *kick_desc, size_t kick_desc_len);
	/* Wait for a submitted fence to complete (timeout_us < 0 = forever). Returns 0
	 * on completion, >0 on timeout, <0 on error/HWR. */
	int  (*wait)(int64_t fence, int64_t timeout_us);
	/* The highest fence that has completed, without waiting (0: none yet).
	 * The backend keeps kicks in flight and needs this to know which of its
	 * per-kick resources and which GL objects are free to be rewritten. */
	int64_t (*completed)(void);

	/* ---- registers (optional; for backends that poke CRs directly) ---- */
	uint32_t (*reg_read)(uint32_t off);
	void     (*reg_write)(uint32_t off, uint32_t val);

	/* ---- platform queries ---- *
	 * Fetch a platform-owned object the GPU backend needs (e.g. the driver's
	 * static render objects). `what` is an OSAL_QUERY_* id (osal_internal.h),
	 * `out` a caller buffer of `out_sz` bytes. Returns 0, or -ENOTSUP if the
	 * platform has no such object. */
	int  (*query)(uint32_t what, void *out, size_t out_sz);

	/* ---- diagnostics ---- *
	 * One line per call, no trailing newline. A line the core formats is at
	 * most OSAL_LOG_LINE_MAX - 1 bytes; an OSAL that formats into a buffer of
	 * its own sizes it OSAL_LOG_LINE_MAX (src/osal/osal_log.h), so no core
	 * line is cut. */
	void (*log)(const char *fmt, ...);

	/* ---- presentation ---- *
	 * Block until the display has begun a new scan-out period, so that
	 * eglSwapBuffers can honour the swap interval EGL 1.5 3.10.3 gives it
	 * (the default is 1: one vertical period per swap). Returns 0 when a
	 * period passed, -ETIMEDOUT, or -ENOTSUP.
	 *
	 * NULL on a platform with no vertical sync, and then presentation is
	 * immediate and eglSwapInterval is accepted and ignored, which is what
	 * every YakoGL platform did before this op existed. Without it an
	 * application paces itself against nothing: it renders as fast as the
	 * GPU allows into a surface the display is scanning out, so what
	 * reaches the panel is whatever a scan-out happens to catch, and
	 * animation that advances per frame runs at the frame rate rather than
	 * at a rate anyone chose. */
	int (*wait_vblank)(int64_t timeout_us);

	/* Show a finished colour buffer on the platform's display: `mem` is a
	 * buffer the library allocated or mapped through this table (a window
	 * surface's buffer), `width` x `height` pixels of `stride_bytes` a row
	 * in the native window's `format` (GLES_NATIVE_FORMAT_*, plus its
	 * GLES_NATIVE_WINDOW_* bits). The call may wait for the display to
	 * take it (a vertical period); it returns 0 when the buffer is shown,
	 * 1 when it is shown and the call waited for a vertical period (the
	 * swap counts that period against its swap interval, so interval 1
	 * waits one period, not two), -ENOTSUP when this platform cannot show
	 * it, or another negative errno. The buffer stays the display's until the next present, so the
	 * library does not draw into it again before that.
	 *
	 * With it a window surface is double-buffered (EGL_BACK_BUFFER, EGL
	 * 1.5 3.5.1): eglSwapBuffers presents the previous frame's buffer and
	 * draws the next frame into one the display is not scanning, so an
	 * application gets a complete picture and CPU/GPU overlap without
	 * code of its own. NULL: a window surface is single-buffered
	 * (EGL_SINGLE_BUFFER), rendered in place in the native window, and
	 * shown by the application after the swap (the samples' board_show). */
	int (*present)(const struct osal_gpu_mem *mem, uint32_t width, uint32_t height,
		       uint32_t stride_bytes, uint32_t format);

	/* ---- threads ---- *
	 * The contract: ONE thread issues every GL and EGL call of the process.
	 * That thread may create any number of contexts and make them current
	 * in turn. The library keeps no per-thread state and does not lock its
	 * tables for other threads, so these are NOT supported: a call from a
	 * second thread, and two calls at the same time. (An application that
	 * needs another thread hands its work to the GL thread.)
	 *
	 * thread_id() lets the library detect misuse. It returns a number that
	 * names the calling thread: non-zero, the same on every call from one
	 * thread, different for two live threads. 0 means "cannot tell", and
	 * NULL is the same as always 0. The thread that makes a context current
	 * owns the library until it releases it (eglMakeCurrent with
	 * EGL_NO_CONTEXT, or eglTerminate). While it does, a call from any other
	 * thread is refused and changes nothing: an EGL entry point returns its
	 * failure value (EGL_FALSE, EGL_NO_*) and that thread's eglGetError
	 * returns EGL_BAD_ACCESS; a GL entry point does nothing and that
	 * thread's glGetError returns GL_INVALID_OPERATION. The first refusal is
	 * logged once through log(). The owner's state and errors are not
	 * touched. Nothing is checked while no context is current, or where
	 * thread_id is NULL or returns 0: there a second thread is still a
	 * contract breach, only an undetected one. The call costs one compare on
	 * the entry path while nothing is current, and one thread_id() call per
	 * GL/EGL call while a context is, so it must be cheap (a thread-local
	 * read, not a system call).
	 *
	 * lock() and unlock() are an optional mutex the library takes around
	 * each EGL entry point (not recursive; both or neither, a table with one
	 * of the two is taken as having neither). They do NOT make a second
	 * thread safe: no GL entry point takes them, and the GL and EGL state is
	 * shared by every context. The Linux OSALs supply them; the Zephyr and
	 * bare-metal ones leave them NULL. */
	void (*lock)(void);
	void (*unlock)(void);
	uint32_t (*thread_id)(void);

	/* ---- presentation, continued ---- *
	 * Which of these buffers the display may still scan: `phys` holds the
	 * physical addresses (osal_gpu_mem.phys) of a window surface's `n`
	 * colour buffers (n <= 32); bit k of the result is set when the display
	 * may be scanning phys[k] now or may still start to (a buffer `present`
	 * was given and the display has not yet replaced it). The answer may
	 * be too large, never too small: a buffer whose bit is clear is not
	 * read by the display until it is presented again. It never waits.
	 *
	 * For a display whose `present` does not wait (the AM62P's DSS latches
	 * a new address at its next frame end and a later present replaces an
	 * unlatched one): with it the library keeps four buffers and draws the
	 * next frame into one that is neither being drawn nor busy, so a swap
	 * never waits and the panel never scans a buffer being drawn. NULL:
	 * `present` waits, or the display does not scan the buffers, and the
	 * library rotates three buffers as before. */
	uint32_t (*display_busy)(const uint64_t *phys, size_t n);
};

/* The active platform ops. A platform's OSAL translation unit defines this (or a
 * registration call sets it) before any GL call. */
extern const struct osal_ops *g_osal;

/* Convenience wrappers (null-safe-ish; the core uses these). */
static inline int osal_gpu_alloc(uint64_t heap, size_t sz, uint32_t fl,
				 struct osal_gpu_mem *o)
{ return g_osal->gpu_alloc(heap, sz, fl, o); }

#ifdef __cplusplus
}
#endif

#endif /* GLES_OSAL_H */
