/* SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Host OSAL: YakoGL on a Linux/POSIX host with no GPU. GPU memory is
 * malloc'd (with fake device VAs bumped per heap), submits complete at once,
 * registers are absent. What it is for: running the whole GL core, the PowerVR
 * backend and the offline shader table on the host under AddressSanitizer or
 * valgrind with a real application's call stream (tools: the no_std Slint
 * host harness pointed at this library as its libEGL). Nothing is rendered.
 *
 * A YAKOGL_GPU_V3D build (the Raspberry Pi 4) models the board's GPU memory
 * instead of the per-heap bump below: one carve-out of a size given at run
 * time, the Zephyr V3D OSAL's own allocator and free rule over it, and its
 * GPU address layout (the section "the V3D" below).
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
#if defined(YAKOGL_GPU_V3D)
#include <time.h>
#include "v3d_kick.h"
#include "v3d_job.h"       /* v3d_job_pack_skip */
#include "v3d_frees.h"
#include "v3d_granules.h"
#if defined(__SANITIZE_ADDRESS__)
#include <sanitizer/asan_interface.h>
#endif
#endif

static void l_log(const char *fmt, ...);   /* defined below; every section reports through it */

extern unsigned g_gl_error_log_limit;   /* gl/context.rs */

/* YAKOGL_ERROR_LOG: on the host there is no slow console to protect, and a
 * conformance run wants to see which entry point raised every error it
 * collects. */
static void error_log_from_env(void)
{
	const char *lim = getenv("YAKOGL_ERROR_LOG");

	if (lim && *lim)
		g_gl_error_log_limit = (unsigned)strtoul(lim, NULL, 0);
}

#if !defined(YAKOGL_GPU_V3D)

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
#if defined(YAKOGL_GPU_VIVANTE)
	uint64_t gc_budget;  /* YAKOGL_GC_CARVEOUT_BYTES: 0 = no limit */
	uint64_t gc_refused; /* allocations the budget refused */
#endif
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

#if defined(YAKOGL_GPU_VIVANTE)
/* YAKOGL_GC_CARVEOUT_BYTES: the GC7000's GPU memory as the board has it,
 * one carve-out every heap shares (src/osal/zephyr_gc.c: gc_render_alloc
 * ignores the heap base). An allocation that would take the live bytes of
 * all heaps together past it fails with -ENOMEM, as the board's pool does.
 * Unset or 0: no limit (each heap's own HEAP_RANGE_BYTES only). The board
 * also keeps its window and the panel-sized screen copy in the carve-out
 * (driver/zephyr/lib/board/imx8mp/board_imx8mp.c), and holds frees until
 * their kick completes; a caller models those by passing less than the
 * carve-out's size (tools/hostharness/glmark2/run.sh, GM2_GC_CARVEOUT). */
static void gc_budget_from_env(void)
{
	const char *b = getenv("YAKOGL_GC_CARVEOUT_BYTES");

	l.gc_budget = b && *b ? strtoull(b, NULL, 0) : 0;
	if (l.gc_budget)
		l_log("osal(linux): YAKOGL_GC_CARVEOUT_BYTES %llu: the heaps share this budget",
		      (unsigned long long)l.gc_budget);
}

/* The live bytes of every heap together. */
static uint64_t gc_live(void)
{
	uint64_t n = 0;
	int i;

	for (i = 0; i < MAX_HEAPS; i++)
		if (l.heap[i] != 0)
			n += l.heaps[i].used;
	return n;
}
#endif

static int l_init(void *platform_arg)
{
	(void)platform_arg;
	memset(&l, 0, sizeof(l));
	error_log_from_env();
#if defined(YAKOGL_GPU_VIVANTE)
	gc_budget_from_env();
#endif
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
#if defined(YAKOGL_GPU_VIVANTE)
	if (l.gc_budget)
		l_log("osal(linux): YAKOGL_GC_CARVEOUT_BYTES %llu: %llu allocation(s) refused",
		      (unsigned long long)l.gc_budget, (unsigned long long)l.gc_refused);
#endif
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
#if defined(YAKOGL_GPU_VIVANTE)
	if (l.gc_budget && gc_live() + rsz > l.gc_budget) {
		if (l.gc_refused++ < 16)
			l_log("osal(linux): out of the %llu-byte GC carve-out: %zu bytes asked, %llu live",
			      (unsigned long long)l.gc_budget, size, (unsigned long long)gc_live());
		return -12;
	}
#endif
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

#endif /* !YAKOGL_GPU_V3D */

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

#if !defined(YAKOGL_GPU_V3D)

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

#else /* YAKOGL_GPU_V3D */

/* ---- the V3D ---------------------------------------------------------------
 *
 * The Zephyr V3D OSAL (src/osal/zephyr_v3d.c) with the GPU taken out: the
 * same carve-out layout, allocator and free rule, so a host run runs out of
 * GPU memory where the Pi 4 would, and every submit completes inside
 * submit(). What is the board's, and kept here:
 *
 *   carve-out  one pool for every heap label; a 512 KiB page table and a
 *              scratch page at its front (the page table maps 512 MiB of
 *              GPU VA); the rest from GPU VA 1 MiB (V3D_VA_BASE) on
 *   gpu_alloc  256-byte granules, page-aligned from a page up, first fit
 *              (v3d_granules.h; small blocks from the top when asked)
 *   gpu_free   held while the backend records a job, at once otherwise
 *              (v3d_frees.h, OSAL_QUERY_V3D_JOB_OPEN)
 *   overflow   the binner's overflow blocks taken from the carve-out at init
 *   map_phys   a window at a fresh GPU VA past the carve-out, its CPU view
 *              the host address (the "physical" address a host window gives)
 *   query      the VPM the programs are linked for, and the settings the
 *              Zephyr OSAL answers from Kconfig, from the environment here
 *
 * What is not: no binner (OSAL_QUERY_V3D_BIN_POOL has no data), no TFU (the
 * board's default: every tiling on the CPU), no hang and no reset.
 *
 * The settings, read at the first init (each the Kconfig option of the
 * same name on Zephyr, zephyr/Kconfig, and its default):
 *   YAKOGL_V3D_CARVEOUT_BYTES        50331648 (a multiple of 8 KiB)
 *   YAKOGL_V3D_BIN_OVERFLOW_BLOCKS   2
 *   YAKOGL_V3D_CARVEOUT_TOP_BELOW    0
 *   YAKOGL_V3D_DRAWS_PER_JOB         1024
 *   YAKOGL_V3D_TILED_COPIES_BYTES    8388608
 *   YAKOGL_V3D_SPLIT_ZS              1
 *   YAKOGL_V3D_S8_STENCIL            0
 *   YAKOGL_V3D_PACK_SKIP             1
 * A board's .conf sets some of them (glmark2's rpi_4b.conf: the carve-out,
 * 100663296); tools/hostharness/glmark2/run.sh passes that value.
 */

#define PAGE               4096u
#define V3D_GRANULE        256u
#define V3D_VA_BASE        0x00100000u
#define V3D_VA_SPACE       0x20000000u
#define V3D_PT_BYTES       ((V3D_VA_SPACE / PAGE) * 4u)
#define V3D_OVERFLOW_BLOCK (256u * 1024u)
/* The VPM the online compiler links every program for
 * (src/shaderc/online/codegen.rs V3D_VPM_BYTES): the BCM2711's, which the
 * Zephyr OSAL reads from IDENT1. The backend refuses to start on another. */
#define V3D_VPM_BYTES      65536u
#define V3D_WINDOWS        32
#define V3D_OOM_SIZES      16

#if defined(__SANITIZE_ADDRESS__)
/* Inside the one carve-out AddressSanitizer would see no allocation's end:
 * the granules no allocation holds are poisoned, so a CPU access past an
 * allocation (or into a freed one) is reported as on a heap block. */
#define V3D_POISON(p, n)   ASAN_POISON_MEMORY_REGION((p), (n))
#define V3D_UNPOISON(p, n) ASAN_UNPOISON_MEMORY_REGION((p), (n))
#else
#define V3D_POISON(p, n)   ((void)(p), (void)(n))
#define V3D_UNPOISON(p, n) ((void)(p), (void)(n))
#endif

static struct {
	bool inited;
	uint8_t *carve;             /* the carve-out: page table, scratch page, heap */
	uint32_t carve_bytes;
	uint32_t granules;          /* carve_bytes / V3D_GRANULE */
	uint32_t *used_map;         /* the allocator's map, a bit a granule */
	struct v3d_granules gm;
	struct v3d_frees frees;
	uint32_t submitted;         /* also the newest completed fence */
	uint32_t window_va;         /* the next GPU VA for map_phys */
	struct {
		uint32_t va, span;
		uint8_t *cpu;
	} windows[V3D_WINDOWS];
	uint32_t nwindows;
	/* the settings (the comment above) */
	uint32_t overflow_blocks, top_below, draws_per_job, tiled_budget, split_zs, s8_stencil;
	/* the allocations refused, by size (the Zephyr OSAL's oom_line) */
	struct {
		uint32_t size, refused;
	} oom[V3D_OOM_SIZES + 1];
	uint32_t oom_total;
} v;

static uint32_t env_u32(const char *name, uint32_t def)
{
	const char *s = getenv(name);

	return s && *s ? (uint32_t)strtoul(s, NULL, 0) : def;
}

static bool v_in_carve(const void *p)
{
	return v.carve && p >= (const void *)v.carve && (const uint8_t *)p < v.carve + v.carve_bytes;
}

/* The CPU view of `n` bytes at a GPU address in the carve-out or in a window,
 * or NULL (zephyr_v3d.c cpu_of). */
static uint8_t *v_cpu_of(uint32_t va, uint32_t n)
{
	uint32_t i;

	if (va >= V3D_VA_BASE && (uint64_t)va - V3D_VA_BASE + n <= v.carve_bytes)
		return v.carve + (va - V3D_VA_BASE);
	for (i = 0; i < v.nwindows && i < V3D_WINDOWS; i++) {
		if (va >= v.windows[i].va && (uint64_t)va - v.windows[i].va + n <= v.windows[i].span)
			return v.windows[i].cpu + (va - v.windows[i].va);
	}
	return NULL;
}

static void v_release(const struct v3d_frees_run *r)
{
	v3d_granules_mark(&v.gm, r->granule, r->n, false);
	V3D_POISON(v.carve + (size_t)r->granule * V3D_GRANULE, (size_t)r->n * V3D_GRANULE);
}

/* Every submitted job has completed: what the frees held for them goes back. */
static void v_drain(void)
{
	v3d_frees_drain(&v.frees, v.submitted, v_release);
}

/* pending_frees.h's full-queue wait (taken only with
 * YAKOGL_PENDING_FREES_WAIT): a submitted job has completed already; the job
 * being recorded cannot complete before it is submitted. */
static int v_wait_pending(uint32_t fence, uint32_t *completed)
{
	if (fence > v.submitted)
		return -22;
	*completed = v.submitted;
	return 0;
}

static void v_report_pending(const char *what)
{
	char line[160];

	pf_format(&v.frees.q, line, sizeof(line));
	l_log("osal(linux): v3d %s%s", what, line);
}

/* "out of GPU memory" as the Zephyr OSAL prints it (its oom_line): the first
 * refusal of a size and its 10th, 100th, ... ones, so a host run's lines are
 * the board's lines. */
static void v_oom_line(size_t size)
{
	unsigned i = 0;
	uint32_t n, m;
	bool power_of_ten = true;

	while (i < V3D_OOM_SIZES && v.oom[i].refused && v.oom[i].size != (uint32_t)size)
		i++;
	v.oom[i].size = i < V3D_OOM_SIZES ? (uint32_t)size : 0;
	n = ++v.oom[i].refused;
	v.oom_total++;
	for (m = n; m > 1 && power_of_ten; m /= 10)
		power_of_ten = m % 10 == 0;
	if (!power_of_ten)
		return;
	if (i < V3D_OOM_SIZES)
		l_log("osal(linux): out of GPU memory for %u bytes (%u frees held; this size refused %u time%s)",
		      (unsigned)size, v.frees.q.n, n, n == 1 ? "" : "s");
	else
		l_log("osal(linux): out of GPU memory for %u bytes (%u frees held; sizes past the first %u "
		      "refused %u times)", (unsigned)size, v.frees.q.n, V3D_OOM_SIZES, n);
}

static int l_init(void *platform_arg)
{
	uint32_t carve, first, heap_off, i;

	(void)platform_arg;
	if (v.inited)
		return 0;   /* as on the board: the carve-out and its blocks stay */
	error_log_from_env();
	carve = env_u32("YAKOGL_V3D_CARVEOUT_BYTES", 50331648u);
	/* the Zephyr OSAL's build-time checks, at run time: whole 32-granule
	 * words of the map (so whole pages), room for the page table, and the
	 * carve-out inside the GPU VA space */
	if (carve % (32u * V3D_GRANULE) != 0 || carve <= V3D_PT_BYTES + 2 * PAGE ||
	    (uint64_t)V3D_VA_BASE + carve >= V3D_VA_SPACE) {
		l_log("osal(linux): YAKOGL_V3D_CARVEOUT_BYTES %u: not a multiple of %u bytes, or not between "
		      "%u and %u bytes", carve, 32u * V3D_GRANULE, V3D_PT_BYTES + 2 * PAGE,
		      V3D_VA_SPACE - V3D_VA_BASE);
		return -22;
	}
	v.overflow_blocks = env_u32("YAKOGL_V3D_BIN_OVERFLOW_BLOCKS", 2);
	v.top_below = env_u32("YAKOGL_V3D_CARVEOUT_TOP_BELOW", 0);
	v.draws_per_job = env_u32("YAKOGL_V3D_DRAWS_PER_JOB", 1024);
	v.tiled_budget = env_u32("YAKOGL_V3D_TILED_COPIES_BYTES", 8388608u);
	v.split_zs = env_u32("YAKOGL_V3D_SPLIT_ZS", 1) != 0;
	v.s8_stencil = env_u32("YAKOGL_V3D_S8_STENCIL", 0) != 0;
	/* the job builder reads it at each v3d_job_begin */
	v3d_job_pack_skip = env_u32("YAKOGL_V3D_PACK_SKIP", 1) != 0;

	v.granules = carve / V3D_GRANULE;
	v.carve = aligned_alloc(PAGE, carve);
	v.used_map = calloc(v.granules / 32, sizeof(uint32_t));
	if (!v.carve || !v.used_map) {
		free(v.carve);
		free(v.used_map);
		v.carve = NULL;
		v.used_map = NULL;
		l_log("osal(linux): no host memory for a %u-byte V3D carve-out", carve);
		return -12;
	}
	v.carve_bytes = carve;
	/* the page table and the scratch page at the front, never handed out */
	heap_off = V3D_PT_BYTES + PAGE;
	first = heap_off / V3D_GRANULE;
	V3D_POISON(v.carve, carve);
	v3d_granules_init(&v.gm, v.used_map, first, v.granules);
	v3d_granules_top_below(&v.gm, v.top_below / V3D_GRANULE);
	v.window_va = (V3D_VA_BASE + carve + PAGE - 1) & ~(PAGE - 1);
	/* the binner's overflow blocks: taken once, as the Zephyr OSAL does, so
	 * they are missing from the allocations as on the board */
	for (i = 0; i < v.overflow_blocks; i++) {
		int64_t g = v3d_granules_find(&v.gm, V3D_OVERFLOW_BLOCK / V3D_GRANULE, PAGE / V3D_GRANULE);

		if (g < 0) {
			l_log("osal(linux): the V3D carve-out cannot hold %u binner overflow blocks of %u KiB",
			      v.overflow_blocks, V3D_OVERFLOW_BLOCK / 1024);
			free(v.carve);
			free(v.used_map);
			memset(&v, 0, sizeof(v));
			return -12;
		}
		v3d_granules_mark(&v.gm, (uint32_t)g, V3D_OVERFLOW_BLOCK / V3D_GRANULE, true);
	}
	v.inited = true;
	l_log("osal(linux): v3d carve-out %u bytes, GPU VA 0x%x..0x%x, %u overflow blocks, draws a job %u, "
	      "tiled copies %u bytes, depth/stencil %s%s, packet skip %s",
	      carve, V3D_VA_BASE + heap_off, V3D_VA_BASE + carve, v.overflow_blocks, v.draws_per_job,
	      v.tiled_budget, v.split_zs ? "apart" : "in one buffer", v.s8_stencil ? " (S8)" : "",
	      v3d_job_pack_skip ? "on" : "off");
	return 0;
}

/* The carve-out's line: at shutdown and at process exit (one process a
 * glmark2 scene, so the scene's high-water). */
static void report_heaps(void)
{
	uint32_t longest = 0, runs = 0, held = 0, i;

	v3d_free_runs(v.used_map, v.gm.first, v.granules, &longest, &runs);
	for (i = 0; i < v.frees.q.n; i++)
		held += v.frees.q.e[i].p.n * V3D_GRANULE;
	l_log("osal(linux): v3d carve-out: %u bytes used at close, %u at most, of %u; frees held %u (%u bytes); "
	      "longest free run %u bytes (%u runs); allocations refused %u",
	      v.gm.used * V3D_GRANULE, v.gm.high * V3D_GRANULE, (v.granules - v.gm.first) * V3D_GRANULE,
	      v.frees.q.n, held, longest * V3D_GRANULE, runs, v.oom_total);
	v_report_pending("");
}

static void l_shutdown(void)
{
	if (!v.inited)
		return;
	report_heaps();
	free(v.carve);
	free(v.used_map);
	memset(&v, 0, sizeof(v));
}

__attribute__((destructor)) static void l_exit_report(void)
{
	if (v.inited)
		report_heaps();
}

static int l_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	uint32_t n, align, off;
	int64_t g;

	(void)heap_base;   /* one pool; the heap labels only name the request */
	(void)flags;
	if (!v.inited || !out || size == 0 || size > v.carve_bytes)
		return -22;
	n = (uint32_t)((size + V3D_GRANULE - 1) / V3D_GRANULE);
	/* page-aligned for anything a page or larger, granule-aligned below */
	align = size >= PAGE ? PAGE / V3D_GRANULE : 1;
	g = v3d_granules_place(&v.gm, n, align);
	if (g < 0) {
		v_drain();
		g = v3d_granules_place(&v.gm, n, align);
	}
	if (g < 0) {
		v_oom_line(size);
		return -12;
	}
	v3d_granules_mark(&v.gm, (uint32_t)g, n, true);
	off = (uint32_t)g * V3D_GRANULE;
	/* exactly the request, so a sanitizer sees an overrun of it */
	V3D_UNPOISON(v.carve + off, size);
	memset(v.carve + off, 0, size);
	out->cpu = v.carve + off;
	out->gpu = V3D_VA_BASE + off;
	out->phys = (uint64_t)(uintptr_t)out->cpu;
	out->size = size;
	out->impl = (void *)(uintptr_t)out->gpu;   /* non-NULL marks it live */
	return 0;
}

/* Held for the job being recorded, given back at once otherwise
 * (v3d_frees.h). A window keeps its VA, as on the board. */
static int l_gpu_free(struct osal_gpu_mem *mem)
{
	int rc = 0;

	if (!mem)
		return 0;
	if (mem->impl && v_in_carve(mem->cpu)) {
		uint32_t off = (uint32_t)((uint8_t *)mem->cpu - v.carve);

		rc = v3d_frees_free(&v.frees, off / V3D_GRANULE,
				    (uint32_t)((mem->size + V3D_GRANULE - 1) / V3D_GRANULE),
				    (uint32_t)((mem->size + 1023) / 1024), v.submitted, v.submitted,
				    PF_WAIT(v_wait_pending), v_release);
		if (rc && v.frees.q.refused == 1)
			v_report_pending("a free refused, memory kept (the first; the rest are counted): ");
	}
	mem->cpu = NULL;
	mem->gpu = 0;
	mem->size = 0;
	mem->impl = NULL;
	return rc;
}

static int l_map_phys(uint64_t heap_base, uint64_t phys, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	uint64_t base, span;

	(void)heap_base;
	(void)flags;
	if (!v.inited || !out || size == 0)
		return -22;
	base = phys & ~(uint64_t)(PAGE - 1);
	span = ((phys - base) + size + PAGE - 1) & ~(uint64_t)(PAGE - 1);
	if ((uint64_t)v.window_va + span > V3D_VA_SPACE)
		return -12;
	out->cpu = (void *)(uintptr_t)phys;
	out->gpu = v.window_va + (phys - base);
	if (v.nwindows < V3D_WINDOWS) {
		v.windows[v.nwindows].va = v.window_va;
		v.windows[v.nwindows].span = (uint32_t)span;
		v.windows[v.nwindows].cpu = (uint8_t *)(uintptr_t)base;
		v.nwindows++;
	} else if (v.nwindows == V3D_WINDOWS) {
		v.nwindows++;   /* once */
		l_log("osal(linux): more than %u map_phys windows: the later ones have no CPU view", V3D_WINDOWS);
	}
	out->phys = phys;
	out->size = size;
	out->impl = (void *)(uintptr_t)out->gpu;
	v.window_va += (uint32_t)span;
	return 0;
}

/* The kick checked as the Zephyr OSAL checks it, plus the lists' memory
 * (which the board's MMU would fault on), the tile state zeroed, and the
 * job complete at once. */
static int64_t l_submit(const void *kick_desc, size_t kick_desc_len)
{
	const struct v3d_kick *k = kick_desc;
	uint8_t *ts;

	if (!v.inited || !k || kick_desc_len < sizeof(*k))
		return -22;
	if (k->magic != V3D_KICK_MAGIC) {
		l_log("osal(linux): kick magic 0x%08x is not a V3D kick", k->magic);
		return -22;
	}
	ts = v_cpu_of(k->tile_state, k->tile_state_size);
	if (!ts || k->bcl_end <= k->bcl_start || k->rcl_end <= k->rcl_start ||
	    !v_cpu_of(k->bcl_start, k->bcl_end - k->bcl_start) || !v_cpu_of(k->rcl_start, k->rcl_end - k->rcl_start) ||
	    !v_cpu_of(k->tile_alloc, k->tile_alloc_size)) {
		l_log("osal(linux): a kick with bad lists or tile state (BCL 0x%x..0x%x, RCL 0x%x..0x%x, tile alloc "
		      "0x%x+%u, tile state 0x%x+%u)", k->bcl_start, k->bcl_end, k->rcl_start, k->rcl_end,
		      k->tile_alloc, k->tile_alloc_size, k->tile_state, k->tile_state_size);
		return -22;
	}
	memset(ts, 0, k->tile_state_size);
	v.submitted++;
	v_drain();
	return (int64_t)v.submitted;
}

static int l_wait(int64_t fence, int64_t timeout_us)
{
	(void)timeout_us;
	return (!v.inited || fence <= 0 || fence > (int64_t)v.submitted) ? -22 : 0;
}

static int64_t l_completed(void)
{
	return v.inited ? (int64_t)v.submitted : 0;
}

static int answer_u32(void *out, size_t out_sz, uint32_t value)
{
	if (out_sz != sizeof(uint32_t))
		return -22;
	*(uint32_t *)out = value;
	return 0;
}

static int l_query(uint32_t what, void *out, size_t out_sz)
{
	if (!v.inited || !out)
		return -22;
	switch (what) {
	case OSAL_QUERY_V3D_VPM_BYTES:
		return answer_u32(out, out_sz, V3D_VPM_BYTES);
	case OSAL_QUERY_V3D_CPU_OF_GPU: {
		struct osal_v3d_cpu_of *q = out;

		if (out_sz != sizeof(*q) || q->gpu > UINT32_MAX || q->bytes > UINT32_MAX)
			return -22;
		q->cpu = v_cpu_of((uint32_t)q->gpu, (uint32_t)q->bytes);
		return q->cpu ? 0 : -22;
	}
	case OSAL_QUERY_V3D_JOB_OPEN:
		if (out_sz != sizeof(uint32_t))
			return -22;
		v.frees.job_open = *(const uint32_t *)out != 0;
		if (!v.frees.job_open)
			v_drain();
		return 0;
	case OSAL_QUERY_V3D_SPLIT_ZS:
		return answer_u32(out, out_sz, v.split_zs);
	case OSAL_QUERY_V3D_S8_STENCIL:
		return answer_u32(out, out_sz, v.s8_stencil);
	case OSAL_QUERY_V3D_TILED_BUDGET:
		return answer_u32(out, out_sz, v.tiled_budget);
	case OSAL_QUERY_V3D_DRAWS_PER_JOB:
		return answer_u32(out, out_sz, v.draws_per_job);
	case OSAL_QUERY_V3D_BIN_POOL:
		return -61;   /* -ENODATA: no binner ran (the board's answer before its first job) */
	case OSAL_QUERY_CLOCK_NS: {
		struct timespec t;

		if (out_sz != sizeof(uint64_t) || clock_gettime(CLOCK_MONOTONIC, &t) != 0)
			return -22;
		*(uint64_t *)out = (uint64_t)t.tv_sec * 1000000000u + (uint64_t)t.tv_nsec;
		return 0;
	}
	default:
		return -95;   /* no TFU (the board's default), no draw-step totals */
	}
}

#endif /* YAKOGL_GPU_V3D */

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
#if defined(YAKOGL_GPU_V3D)
	/* the board's way: a window at a VA of its own, no pool memory */
	.mmu_map     = NULL,
	.map_phys    = l_map_phys,
#else
	.mmu_map     = l_mmu_map,
	.map_phys    = NULL,
#endif
	.gpu_write   = l_gpu_write,
	.gpu_read    = l_gpu_read,
	.cache_flush = l_cache_flush,
	.cache_invd  = l_cache_invd,
	.submit      = l_submit,
	.wait        = l_wait,
	.completed   = l_completed,
#if defined(YAKOGL_GPU_V3D)
	.reg_read    = NULL,
	.reg_write   = NULL,
#else
	.reg_read    = l_reg_read,
	.reg_write   = l_reg_write,
#endif
	.query       = l_query,
	.log         = l_log,
	/* no vertical sync here: presentation is immediate */
	.wait_vblank = NULL,
	.lock        = l_lock,
	.unlock      = l_unlock,
	.thread_id   = l_thread_id,
};
