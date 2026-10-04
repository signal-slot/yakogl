/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Host OSAL: YakoGL on a Linux/POSIX host with no GPU. GPU memory is
 * malloc'd (with fake device VAs bumped per heap), submits complete at once,
 * registers are absent. What it is for: running the whole GL core, the PowerVR
 * backend and the offline shader table on the host under AddressSanitizer or
 * valgrind with a real application's call stream (tools: the no_std Slint
 * host harness pointed at this library as its libEGL). Nothing is rendered.
 */

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>
#include <stdlib.h>
#include <string.h>
#include <stdarg.h>
#include <stdio.h>
#include <pthread.h>
#include <sys/syscall.h>
#include <unistd.h>

#include "gles_osal.h"
#include "osal_internal.h"
#include "osal_bounds.h"
#include "linux_heap.h"
#include "pvr_render.h"   /* the driver repository's src/, on the include path */
#if defined(YAKOGL_GPU_VIVANTE)
#include "gc_kick.h"
#endif

#define HEAP_GENERAL 0x8000000000ULL
#define HEAP_PDS     0xDA00000000ULL
#define HEAP_USC     0xE000000000ULL
#define PAGE         4096u
#define MAX_HEAPS    4

/* Each heap's fake device-VA range is finite (a code review finding:
 * "host fake VA: no overflow or reuse" -- the old per-heap bump had neither
 * a limit nor a way to give an address back). 256 MiB costs nothing until
 * touched (linux_heap.h only tracks offsets; l_gpu_alloc still malloc's
 * exactly `size` bytes per allocation) and is comfortably inside the gap
 * between any two heap bases above (HEAP_PDS - HEAP_GENERAL = 0x5A00000000,
 * far past 256 MiB), so heaps can never run into each other. */
#define HEAP_RANGE_BYTES (256ULL * 1024 * 1024)

static void l_log(const char *fmt, ...);   /* defined below; l_shutdown reports through it */

static struct {
	bool inited;
	uint64_t heap[MAX_HEAPS];           /* the bases the backend has allocated in */
	struct linux_heap heaps[MAX_HEAPS]; /* the free-list allocator per heap (linux_heap.h) */
	int64_t last_fence;   /* every submit completes immediately */
	uint64_t border_gpu;
	void *border_cpu;
	/* Where each live allocation lives, so a submit can find the CPU side
	 * of the stream it names and walk every block the stream points at.
	 * Only the dumps below need this. */
	struct {
		uint64_t gpu;
		void *cpu;
		size_t size;
	} map[64];
	unsigned maps;
} l;

/* Find the allocation a device address falls in, and the offset into it. */
static void *cpu_of(uint64_t gpu, size_t need)
{
	unsigned i;

	for (i = 0; i < l.maps; i++) {
		if (gpu >= l.map[i].gpu && gpu + need <= l.map[i].gpu + l.map[i].size)
			return (uint8_t *)l.map[i].cpu + (gpu - l.map[i].gpu);
	}
	return NULL;
}

/* Each backend chooses its own device-VA heap bases, so they are learned on
 * first use rather than listed here; the allocations of one heap still come
 * back in ascending order from its base, as they would on a device. */
static int heap_index(uint64_t base)
{
	int i;

	for (i = 0; i < MAX_HEAPS; i++) {
		if (l.heap[i] == base)
			return i;
		if (l.heap[i] == 0) {
			l.heap[i] = base;
			linux_heap_init(&l.heaps[i], HEAP_RANGE_BYTES);
			return i;
		}
	}
	return -1;
}

/* The heap a live `gpu` address belongs to, from the bases `heap_index`
 * learned: independent of the debug `l.map[]` (capped at 64 entries, "only
 * the dumps below need this"), so gpu_free can always give a range back to
 * its heap's free list even once more than 64 allocations are live. */
static int heap_index_of(uint64_t gpu)
{
	int i;

	for (i = 0; i < MAX_HEAPS; i++) {
		if (l.heap[i] != 0 && gpu >= l.heap[i] && gpu - l.heap[i] < HEAP_RANGE_BYTES)
			return i;
	}
	return -1;
}

extern unsigned g_gl_error_log_limit;   /* gl/context.rs */
/* vivante/backend.rs; absent from a PowerVR or V3D library, hence weak. */
extern void gc_debug_set_vs_units16(int on) __attribute__((weak));

static int l_init(void *platform_arg)
{
	const char *lim = getenv("YAKOGL_ERROR_LOG");
	const char *vsu = getenv("YAKOGL_GC_VS_UNITS16");

	(void)platform_arg;
	memset(&l, 0, sizeof(l));
	/* On the host there is no slow console to protect, and a conformance
	 * run wants to see which entry point raised every error it collects. */
	if (lim && *lim)
		g_gl_error_log_limit = (unsigned)strtoul(lim, NULL, 0);
	/* YAKOGL_GC_VS_UNITS16=0|1: the GC7000's vertex texture unit count,
	 * 8 or 16 (vivante/backend.rs gc_debug_vs_units16); unset keeps the
	 * library's default. Only a Vivante library has the switch. */
	if (vsu && (vsu[0] == '0' || vsu[0] == '1') && vsu[1] == '\0' && gc_debug_set_vs_units16)
		gc_debug_set_vs_units16(vsu[0] - '0');
	l.inited = true;
	return 0;
}

/* The heaps' lines ("heap B: U bytes used at close, H at most, of R (L
 * leaked)"): at shutdown and at process exit. */
static void report_heaps(void)
{
	int i;

	for (i = 0; i < MAX_HEAPS; i++) {
		if (l.heap[i] == 0)
			continue;
		l_log("osal(linux): heap 0x%llx: %llu bytes used at close, %llu at most, of %llu (%llu leaked)",
		      (unsigned long long)l.heap[i], (unsigned long long)l.heaps[i].used,
		      (unsigned long long)l.heaps[i].high, (unsigned long long)HEAP_RANGE_BYTES,
		      (unsigned long long)l.heaps[i].leaked);
	}
}

static void l_shutdown(void)
{
	report_heaps();
	free(l.border_cpu);
	memset(&l, 0, sizeof(l));
}

/* Process exit: the counters, printed only (a destructor, so a client that
 * never reaches the shutdown op still ends its run with them). This is not
 * the shutdown op: shutdown tears the OSAL down (zeroes the heaps) and
 * the product never calls it, because eglTerminate keeps the backend and its
 * GPU memory for the next eglInitialize. This report frees nothing. Silent
 * when the OSAL never came up or shutdown already printed the line. */
__attribute__((destructor)) static void l_exit_report(void)
{
	if (l.inited)
		report_heaps();
}

static int l_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	int hi = heap_index(heap_base);
	size_t rsz = (size + PAGE - 1) & ~(size_t)(PAGE - 1);
	int64_t off;
	void *cpu;

	(void)flags;
	if (!l.inited || !out || size == 0 || hi < 0)
		return -22;
	/* The heap's finite fake-VA range, first fit over its free list, else a
	 * checked bump (linux_heap.h); -1 is this heap's HEAP_RANGE_BYTES
	 * exhausted, reported as -ENOMEM rather than wrapping (R-25). */
	off = linux_heap_alloc(&l.heaps[hi], rsz);
	if (off < 0)
		return -12;
	/* Exactly the requested size (the capacity the GL core asked for), so a
	 * sanitizer sees an overrun of the request, not of the page it would be
	 * rounded to on the target; the fake device VA still advances by pages. */
	cpu = aligned_alloc(64, (size + 63) & ~(size_t)63);
	if (!cpu) {
		linux_heap_free(&l.heaps[hi], (uint64_t)off, rsz);
		return -12;
	}
	memset(cpu, 0, size);
	out->cpu = cpu;
	out->gpu = heap_base + (uint64_t)off;
	out->phys = (uint64_t)(uintptr_t)cpu;
	out->size = size;
	out->impl = cpu;
	if (l.maps < sizeof(l.map) / sizeof(l.map[0])) {
		l.map[l.maps].gpu = out->gpu;
		l.map[l.maps].cpu = cpu;
		l.map[l.maps].size = size;
		l.maps++;
	}
	return 0;
}

/* No pending-free queue here: every submit completes inside submit(), so no
 * kick can still read a freed block and it goes back at once. Never refused. */
static int l_gpu_free(struct osal_gpu_mem *mem)
{
	unsigned i;
	int hi;

	if (!mem)
		return 0;
	hi = heap_index_of(mem->gpu);
	if (hi >= 0) {
		/* the same page rounding l_gpu_alloc gave the fake VA, recovered from
		 * `size` (out->size is the exact request, not the rounded one -- see
		 * the comment in l_gpu_alloc) rather than stored a second time */
		uint64_t rsz = ((uint64_t)mem->size + PAGE - 1) & ~(uint64_t)(PAGE - 1);
		linux_heap_free(&l.heaps[hi], mem->gpu - l.heap[hi], rsz);
	}
	for (i = 0; i < l.maps; i++) {
		if (l.map[i].cpu == mem->impl) {
			l.map[i] = l.map[--l.maps];
			break;
		}
	}
	free(mem->impl);
	memset(mem, 0, sizeof(*mem));
	return 0;
}

static int l_mmu_map(uint64_t gpu_va, uint64_t phys, size_t size, uint32_t flags)
{
	(void)gpu_va; (void)phys; (void)size; (void)flags;
	return 0;   /* no MMU: a window buffer is never written here */
}

static int l_gpu_write(struct osal_gpu_mem *mem, size_t off, const void *src, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, src, n))
		return -22;
	memcpy((uint8_t *)mem->cpu + off, src, n);
	return 0;
}

static int l_gpu_read(struct osal_gpu_mem *mem, size_t off, void *dst, size_t n)
{
	if (!osal_gpu_copy_ok(mem, off, dst, n))
		return -22;
	memcpy(dst, (const uint8_t *)mem->cpu + off, n);
	return 0;
}

static void l_cache_flush(void *cpu, size_t n) { (void)cpu; (void)n; }
static void l_cache_invd(void *cpu, size_t n)  { (void)cpu; (void)n; }

static int64_t l_submit(const void *kick_desc, size_t kick_desc_len)
{
#if defined(YAKOGL_GPU_VIVANTE)
	const struct gc_kick *k = kick_desc;

	if (!l.inited || !kick_desc || kick_desc_len < sizeof(*k) ||
	    k->magic != GC_KICK_MAGIC)
		return -22;
	/*
	 * The command stream, kept instead of run. Named by YAKOGL_CS_DUMP, in
	 * the same shape the patched Mesa drm-shim writes (a byte length, then
	 * the stream), so tools/vivante/cs_decode.py reads either and the two
	 * can be diffed. That diff is what settles a question about what this
	 * part is told, rather than a fourth guess at a register.
	 */
	{
		const char *path = getenv("YAKOGL_CS_DUMP");
		const void *stream = path ? cpu_of(k->stream_gpu, k->stream_bytes) : NULL;

		if (stream) {
			FILE *f = fopen(path, "ab");

			if (f) {
				uint32_t len = k->stream_bytes;

				fwrite(&len, sizeof(len), 1, f);
				fwrite(stream, 1, len, f);
				fclose(f);
			}
		}
	}
	/*
	 * The blocks the stream only points at -- texture descriptors, shader
	 * binaries, vertex and index buffers -- never appear in the stream, so
	 * a stream diff cannot see them and a wrong descriptor hides behind a
	 * matching set of registers. Named by YAKOGL_MEM_DUMP, in the format
	 * the patched drm-shim writes for ETNA_SHIM_MEM_DUMP: per allocation a
	 * device address, a size, a zero, the bytes, padded to 8.
	 */
	{
		const char *path = getenv("YAKOGL_MEM_DUMP");
		FILE *f = path ? fopen(path, "ab") : NULL;

		if (f) {
			static const uint8_t zeros[8] = { 0 };
			unsigned i;

			for (i = 0; i < l.maps; i++) {
				uint64_t addr = l.map[i].gpu;
				uint32_t size = (uint32_t)l.map[i].size, zero = 0;
				size_t tail = (8u - (size % 8u)) % 8u;

				fwrite(&addr, sizeof(addr), 1, f);
				fwrite(&size, sizeof(size), 1, f);
				fwrite(&zero, sizeof(zero), 1, f);
				fwrite(l.map[i].cpu, 1, size, f);
				if (tail)
					fwrite(zeros, 1, tail, f);
			}
			fclose(f);
		}
	}
#else
	if (!l.inited || !kick_desc || kick_desc_len < sizeof(struct pvr_render_kick))
		return -22;
#endif
	return ++l.last_fence;
}

static int l_wait(int64_t fence, int64_t timeout_us)
{
	(void)timeout_us;
	return (!l.inited || fence <= 0 || fence > l.last_fence) ? -22 : 0;
}

static int64_t l_completed(void)
{
	return l.last_fence;   /* every submit completes at once */
}

static int l_query(uint32_t what, void *out, size_t out_sz)
{
	if (!l.inited || !out)
		return -22;
	if (what == OSAL_QUERY_RENDER_STATIC) {
		struct pvr_render_static *st = out;
		struct osal_gpu_mem m;

		if (out_sz < sizeof(*st))
			return -22;
		memset(st, 0, sizeof(*st));
		if (!l.border_cpu) {
			if (l_gpu_alloc(HEAP_GENERAL, PAGE, OSAL_MEM_DATA, &m) != 0)
				return -12;
			l.border_cpu = m.cpu;
			l.border_gpu = m.gpu;
		}
		st->border_gpu = l.border_gpu;
		st->heap_base[PVR_RENDER_HEAP_GENERAL] = HEAP_GENERAL;
		st->heap_base[PVR_RENDER_HEAP_PDS] = HEAP_PDS;
		st->heap_base[PVR_RENDER_HEAP_USC] = HEAP_USC;
		return 0;
	}
	return -95;
}

static uint32_t l_reg_read(uint32_t off) { (void)off; return 0; }
static void l_reg_write(uint32_t off, uint32_t v) { (void)off; (void)v; }

/* lock/unlock: the optional EGL entry-point mutex (gles_osal.h, "threads").
 * It does not make a second thread safe: one thread issues every GL and EGL
 * call, and thread_id below is how the library catches a second one.
 * PTHREAD_MUTEX_INITIALIZER needs no run-time init. */
static pthread_mutex_t l_mutex = PTHREAD_MUTEX_INITIALIZER;

static void l_lock(void) { pthread_mutex_lock(&l_mutex); }
static void l_unlock(void) { pthread_mutex_unlock(&l_mutex); }

/* thread_id (gles_osal.h, "threads"): the kernel's thread id, read once
 * per thread. gettid(2) is a system call and the library asks on every GL
 * call while a context is current, so the answer is kept in a thread-local
 * (one TLS load after the first call). A thread id is never 0. */
static uint32_t l_thread_id(void)
{
	static __thread uint32_t tid;

	if (tid == 0)
		tid = (uint32_t)syscall(SYS_gettid);
	return tid;
}

static void l_log(const char *fmt, ...)
{
	va_list ap;

	va_start(ap, fmt);
	vfprintf(stderr, fmt, ap);
	va_end(ap);
	fputc('\n', stderr);
}

const struct osal_ops osal_linux_ops = {
	.init        = l_init,
	.shutdown    = l_shutdown,
	.gpu_alloc   = l_gpu_alloc,
	.gpu_free    = l_gpu_free,
	.mmu_map     = l_mmu_map,
	.map_phys    = NULL,
	.gpu_write   = l_gpu_write,
	.gpu_read    = l_gpu_read,
	.cache_flush = l_cache_flush,
	.cache_invd  = l_cache_invd,
	.submit      = l_submit,
	.wait        = l_wait,
	.completed   = l_completed,
	.reg_read    = l_reg_read,
	.reg_write   = l_reg_write,
	.query       = l_query,
	.log         = l_log,
	/* no vertical sync here: presentation is immediate */
	.wait_vblank = NULL,
	.lock        = l_lock,
	.unlock      = l_unlock,
	.thread_id   = l_thread_id,
};
