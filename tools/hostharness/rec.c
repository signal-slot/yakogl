/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * rec.c - the host harness's recording OSAL.
 *
 * A complete fake platform (all 17 struct osal_ops entries) that logs every
 * call the library makes into stdout, in order, together with hashes of the
 * data that crosses the interface, so two library builds driven by the same
 * harness program can be compared line by line. Nothing is rendered.
 *
 * Environment (all optional):
 *   HH_LAG=N        completed() trails submit() by N kicks; wait(f) completes f
 *                   (default 0: every submit completes at once)
 *   HH_FILL=0xNN    byte fresh allocations are filled with (default 0x00 like
 *                   src/osal/linux.c; 0xA5 models an uninitialised pool)
 *   HH_NOCPU=1      object storage (allocations made inside glBufferData,
 *                   glTexImage2D, eglCreatePbufferSurface, ...) has no CPU
 *                   mapping (cpu = NULL); gpu_write / gpu_read still work, so
 *                   the GL core's no-CPU-view paths run. HH_NOCPU=2: no
 *                   allocation has one (the backend cannot start then)
 *   HH_MAPPHYS=0|1|2  map_phys absent (like linux.c; window surfaces go through
 *                   gpu_alloc + mmu_map) | present without a CPU view | with one
 *   HH_NHASH=1      memory hashes treat 0x00 and 0xA5 alike (fill comparison)
 *   HH_BASE=1       allocate / free / read / write through src/osal/linux.c
 *                   (the linked platform OSAL) instead of the harness allocator
 *   HH_FAIL_AT=n, HH_FAIL_EVERY=k   make the n-th / every k-th (after
 *                   HH_FAIL_AFTER, default 48) gpu_alloc fail with -12.
 *                   The skip clears the library's own start-up, whose
 *                   allocations are not what these jobs mean to fail: the
 *                   PowerVR backend alone takes 67 of them and the V3D's 15,
 *                   and failing one there stops the backend instead of
 *                   exercising a GL object's error path (run.sh sets the
 *                   skip and the offsets per GPU).
 *   HH_WAIT_FAIL=k  every k-th wait() on a pending fence times out (-110)
 *   HH_VERBOSE=1    print every allocation's hash at every dump (default:
 *                   only the ones that changed since the last dump)
 *   HH_DUMP=n HH_DUMPTO=prefix   write all memory + the kick at submit n
 *   HH_CALLS=path   write per-entry-point call counts there at exit
 */
#define _GNU_SOURCE
#include <signal.h>
#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>

#include "gles_osal.h"
#include "osal_internal.h"
#include "pvr_render.h"
#if defined(YAKOGL_GPU_V3D)
#include "v3d_kick.h"
#endif
#include "hh.h"

#define HEAP_GENERAL 0x8000000000ULL
#define HEAP_PDS     0xDA00000000ULL
#define HEAP_USC     0xE000000000ULL
#if defined(YAKOGL_GPU_V3D)
/* The V3D backend's two heap labels (src/v3d/backend.rs V3D_HEAP_DATA,
 * V3D_HEAP_SHADER). Its addresses are 32-bit, so this allocator's own VAs
 * for them are two 32-bit ranges that never meet (the backend takes the
 * labels as names only: one pool on the board). */
#define V3D_HEAP_DATA      0x10000000ULL
#define V3D_HEAP_SHADER    0x20000000ULL
#define V3D_VA_DATA        0x10000000ULL
#define V3D_VA_DATA_END    0xC0000000ULL
#define V3D_VA_SHADER      0xD0000000ULL
#define V3D_VA_SHADER_END  0xFFFFF000ULL
#endif
#define PAGE         4096u
#define IMPL_TAG     0x4848000000000000ULL
#define MAPPHYS_VA   0x40000000ULL

enum { K_ALLOC, K_MAPPHYS, K_BORDER };

struct arec {
	uint64_t heap, gpu;
	size_t size;
	unsigned char *host;       /* backing store (NULL: map_phys of unknown memory) */
	void *cpu;                 /* what the library was given as mem->cpu */
	void *base_impl;           /* HH_BASE: linux.c's handle */
	int region;                /* >= 0: mmu_map'd / map_phys'd onto this region */
	size_t region_off;
	int own_region;            /* the region that stands for this allocation's pages */
	uint64_t lasth;
	int prev, next;            /* live list, in id order */
	unsigned kind : 2, live : 1, dumped : 1;
};

/* Memory a physical address can name: application buffers (hh_region_add)
 * and the pages of every live allocation (their phys is the host address,
 * like src/osal/linux.c, so a window surface on a pool buffer maps it). */
struct region { unsigned char *p; size_t n; char name[32]; int live, rec; uint64_t lasth; int dumped; };

static const struct osal_ops *base;
static struct osal_ops rec;
static struct arec *R;
static size_t nR, capR;
static int live_head = -1, live_tail = -1;
static struct region *G;
static int nG, capG;

static int lag, nocpu, mapphys, nhash, use_base, verbose;
static unsigned fill;
static long fail_at = -1, fail_every, fail_after = 48, fail_in, wait_fail;
static unsigned long n_alloc, n_free, n_submit, n_wait, n_init, n_log, errors;
static int64_t last_fence, done_fence;
static uint64_t bump[3], mp_bump;
static int border = -1;
static int inited;

/* ------------------------------------------------------------------ output */

int hh_printf(const char *fmt, ...)
{
	va_list ap;
	int r;

	va_start(ap, fmt);
	r = vprintf(fmt, ap);
	va_end(ap);
	return r;
}

static void err(const char *fmt, ...) __attribute__((format(printf, 1, 2)));
static void err(const char *fmt, ...)
{
	va_list ap;

	errors++;
	printf("HH-ERROR ");
	va_start(ap, fmt);
	vprintf(fmt, ap);
	va_end(ap);
	printf("\n");
}

unsigned hh_errors(void) { return (unsigned)errors; }

void hh_step(const char *fmt, ...)
{
	va_list ap;

	printf("== ");
	va_start(ap, fmt);
	vprintf(fmt, ap);
	va_end(ap);
	printf("\n");
}

/* ------------------------------------------------------------------ hashing */

static inline uint64_t mix(uint64_t h, uint64_t v)
{
	h ^= v;
	h *= 0x9E3779B97F4A7C15ull;
	return h ^ (h >> 29);
}

/* bytes equal to 0x00 or 0xA5 -> 0x00 (exact per-byte SWAR test) */
static inline uint64_t norm(uint64_t w)
{
	const uint64_t lo7 = 0x7F7F7F7F7F7F7F7Full;
	uint64_t y = w ^ 0xA5A5A5A5A5A5A5A5ull;
	uint64_t z = ~(((y & lo7) + lo7) | y | lo7);   /* 0x80 where y's byte is 0 */

	return w & ~((z >> 7) * 0xFF);
}

uint64_t hh_hash(const void *p, size_t n)
{
	const unsigned char *b = p;
	uint64_t h = mix(0x243F6A8885A308D3ull, n), w;
	size_t i = 0;

	if (!p)
		return 0;
	for (; i + 8 <= n; i += 8) {
		memcpy(&w, b + i, 8);
		h = mix(h, nhash ? norm(w) : w);
	}
	if (i < n) {
		w = 0;
		memcpy(&w, b + i, n - i);
		if (nhash) {
			uint64_t pad = 0;
			/* bytes past the end stay 0 either way */
			pad = norm(w);
			w = pad;
		}
		h = mix(h, w);
	}
	return mix(h, 0x13198A2E03707344ull);
}

/* ------------------------------------------------------------------ records */

static int rec_new(void)
{
	if (nR == capR) {
		capR = capR ? capR * 2 : 1024;
		R = realloc(R, capR * sizeof(*R));
		if (!R) {
			fprintf(stderr, "hostharness: out of memory for allocation records\n");
			printf("HH-ERROR out of memory for allocation records\n");
			fflush(stdout);
			abort();
		}
	}
	memset(&R[nR], 0, sizeof(R[nR]));
	R[nR].region = -1;
	R[nR].prev = live_tail;
	R[nR].next = -1;
	R[nR].live = 1;
	if (live_tail >= 0)
		R[live_tail].next = (int)nR;
	else
		live_head = (int)nR;
	live_tail = (int)nR;
	return (int)nR++;
}

static void rec_unlink(int i)
{
	struct arec *a = &R[i];

	if (a->prev >= 0) R[a->prev].next = a->next; else live_head = a->next;
	if (a->next >= 0) R[a->next].prev = a->prev; else live_tail = a->prev;
	a->live = 0;
}

static int rec_of(const struct osal_gpu_mem *m)
{
	if (!m)
		return -1;
	if (use_base) {
		for (int i = live_head; i >= 0; i = R[i].next)
			if (R[i].base_impl == m->impl && R[i].kind != K_MAPPHYS)
				return i;
		if (((uint64_t)(uintptr_t)m->impl & ~0xFFFFFFFFull) != IMPL_TAG)
			return -1;
	}
	uint64_t v = (uint64_t)(uintptr_t)m->impl;
	if ((v & ~0xFFFFFFFFull) != IMPL_TAG)
		return -1;
	v &= 0xFFFFFFFFull;
	if (v == 0 || v > nR)
		return -1;
	return (int)v - 1;
}

static int region_of(uint64_t addr, size_t *off)
{
	for (int g = 0; g < nG; g++)
		if (G[g].live && addr >= (uint64_t)(uintptr_t)G[g].p && addr < (uint64_t)(uintptr_t)G[g].p + G[g].n) {
			*off = addr - (uint64_t)(uintptr_t)G[g].p;
			return g;
		}
	return -1;
}

/* the bytes the GPU sees for a record */
static unsigned char *backing(const struct arec *a, size_t *n)
{
	if (a->region >= 0) {
		const struct region *g = &G[a->region];
		size_t room = g->n - a->region_off;

		if (!g->live) {
			/* the memory behind the mapping is gone */
			*n = 0;
			return NULL;
		}
		*n = a->size < room ? a->size : room;
		return g->p + a->region_off;
	}
	*n = a->host ? a->size : 0;
	return a->host;
}

static uint64_t rec_hash(const struct arec *a)
{
	size_t n;
	unsigned char *p = backing(a, &n);

	return p ? hh_hash(p, n) : 0;
}

static int region_new(void *p, size_t n, const char *name, int recno)
{
	if (nG == capG) {
		capG = capG ? capG * 2 : 256;
		G = realloc(G, capG * sizeof(*G));
		if (!G) {
			printf("HH-ERROR out of memory for regions\n");
			fflush(stdout);
			abort();
		}
	}
	memset(&G[nG], 0, sizeof(G[nG]));
	G[nG].p = p;
	G[nG].n = n;
	G[nG].live = 1;
	G[nG].rec = recno;
	snprintf(G[nG].name, sizeof(G[nG].name), "%s", name);
	return nG++;
}

void hh_region_add(void *p, size_t n, const char *name)
{
	int g = region_new(p, n, name, -1);

	printf("region %d %s size=%zu\n", g, G[g].name, n);
}

static const char *region_name(int g)
{
	static char b[48];

	if (G[g].rec < 0)
		return G[g].name;
	snprintf(b, sizeof(b), "pages of #%d", G[g].rec + 1);
	return b;
}

void hh_region_del(void *p)
{
	for (int g = 0; g < nG; g++)
		if (G[g].live && G[g].p == p) {
			printf("region %d %s gone hash=%016llx\n", g, G[g].name, (unsigned long long)hh_hash(G[g].p, G[g].n));
			G[g].live = 0;
		}
}

void hh_fail_alloc_in(unsigned n) { fail_in = n; }

static void dump(const char *tag, int all)
{
	uint64_t acc = 0x6A09E667F3BCC908ull;
	unsigned long nlive = 0;

	for (int i = live_head; i >= 0; i = R[i].next) {
		struct arec *a = &R[i];
		uint64_t h = rec_hash(a);

		nlive++;
		if (a->region >= 0 && !G[a->region].live)
			err("#%d maps %s, which is gone", i + 1, region_name(a->region));
		if (all || verbose || !a->dumped || h != a->lasth)
			printf("  mem #%d gpu=%llx size=%zu hash=%016llx\n", i + 1, (unsigned long long)a->gpu, a->size,
			       (unsigned long long)h);
		a->lasth = h;
		a->dumped = 1;
		acc = mix(acc, h);
	}
	for (int g = 0; g < nG; g++) {
		uint64_t h;

		if (!G[g].live || G[g].rec >= 0)
			continue;
		h = hh_hash(G[g].p, G[g].n);
		if (all || verbose || !G[g].dumped || h != G[g].lasth)
			printf("  region %d %s hash=%016llx\n", g, G[g].name, (unsigned long long)h);
		G[g].lasth = h;
		G[g].dumped = 1;
		acc = mix(acc, h);
	}
	printf("  memhash[%s] live=%lu %016llx\n", tag, nlive, (unsigned long long)acc);
}

void hh_memdump(const char *tag) { dump(tag, 0); }

/* ------------------------------------------------------------------ ops */

static int r_init(void *arg)
{
	n_init++;
	printf("init #%lu arg=%s%s\n", n_init, arg ? "set" : "NULL", inited ? " (again: state kept)" : "");
	if (!inited) {
		inited = 1;
#if defined(YAKOGL_GPU_V3D)
		/* every mode: linux.c answers the V3D's settings (r_query) */
		if (base->init)
			return base->init(arg);
#else
		if (use_base && base->init)
			return base->init(arg);
#endif
	}
	return 0;
}

static void r_shutdown(void)
{
	printf("shutdown (ignored: state kept)\n");
}

static void place(struct osal_gpu_mem *out, int i)
{
	struct arec *a = &R[i];

	memset(out, 0, sizeof(*out));
	out->cpu = a->cpu;
	out->gpu = a->gpu;
	out->phys = 0;
	out->size = a->size;
	out->impl = use_base && a->kind != K_MAPPHYS ? a->base_impl : (void *)(uintptr_t)(IMPL_TAG | (uint64_t)(i + 1));
}

static int do_alloc(uint64_t heap, size_t size, uint32_t flags, struct osal_gpu_mem *out, int kind)
{
#if defined(YAKOGL_GPU_V3D)
	static const uint64_t va[2] = { V3D_VA_DATA, V3D_VA_SHADER }, va_end[2] = { V3D_VA_DATA_END, V3D_VA_SHADER_END };
	int hi = heap == V3D_HEAP_DATA ? 0 : heap == V3D_HEAP_SHADER ? 1 : -1;
#else
	int hi = heap == HEAP_GENERAL ? 0 : heap == HEAP_PDS ? 1 : heap == HEAP_USC ? 2 : -1;
#endif
	int rc = 0, i;
	struct osal_gpu_mem bm;

	n_alloc++;
	if (!out) {
		err("gpu_alloc #%lu with out == NULL", n_alloc);
		return -22;
	}
	if ((long)n_alloc == fail_at || (fail_every && (long)n_alloc > fail_after && n_alloc % fail_every == 0) ||
	    (fail_in && --fail_in == 0))
		rc = -12;
	else if (hi < 0 || size == 0)
		rc = -22;
	else if (size > (1ull << 32))
		rc = -12;   /* a host run does not model more than 4 GiB of GPU memory */
#if defined(YAKOGL_GPU_V3D)
	/* nor more VA than the 32-bit range of the heap (VAs are never reused) */
	else if (!(use_base && kind != K_MAPPHYS) && va[hi] + bump[hi] + size > va_end[hi])
		rc = -12;
#endif
	if (rc) {
		printf("alloc n%lu heap=%llx size=%zu flags=%u -> %d\n", n_alloc, (unsigned long long)heap, size, flags, rc);
		return rc;
	}
	if (use_base && kind != K_MAPPHYS) {
		rc = base->gpu_alloc(heap, size, flags, &bm);
		if (rc) {
			printf("alloc n%lu heap=%llx size=%zu flags=%u -> %d (base)\n", n_alloc, (unsigned long long)heap, size, flags, rc);
			return rc;
		}
	}
	i = rec_new();
	R[i].kind = kind;
	R[i].heap = heap;
	R[i].size = size;
	if (use_base && kind != K_MAPPHYS) {
		R[i].host = bm.cpu;
		R[i].cpu = bm.cpu;
		R[i].gpu = bm.gpu;
		R[i].base_impl = bm.impl;
	} else {
		/* exactly the requested size, so AddressSanitizer sees an overrun
		 * of the request (the fake VA still advances by pages) */
		R[i].host = malloc(size);
		if (!R[i].host) {
			rec_unlink(i);
			printf("alloc n%lu heap=%llx size=%zu flags=%u -> -12 (host)\n", n_alloc, (unsigned long long)heap, size, flags);
			return -12;
		}
#if defined(YAKOGL_GPU_V3D)
		R[i].gpu = va[hi] + bump[hi];
#else
		R[i].gpu = heap + bump[hi];
#endif
		bump[hi] += (size + PAGE - 1) & ~(uint64_t)(PAGE - 1);
		R[i].cpu = (nocpu == 2 || (nocpu == 1 && hh_storage_call())) ? NULL : R[i].host;
	}
	memset(R[i].host, (int)fill, size);
	R[i].own_region = region_new(R[i].host, size, "", i);
	place(out, i);
	out->phys = (uint64_t)(uintptr_t)R[i].host;
	printf("alloc #%d n%lu heap=%llx size=%zu flags=%u%s -> 0 gpu=%llx%s\n", i + 1, n_alloc, (unsigned long long)heap,
	       size, flags, kind == K_BORDER ? " (border)" : "", (unsigned long long)R[i].gpu, R[i].cpu ? "" : " nocpu");
	return 0;
}

static int r_alloc(uint64_t heap, size_t size, uint32_t flags, struct osal_gpu_mem *out)
{
	return do_alloc(heap, size, flags, out, K_ALLOC);
}

static int r_free(struct osal_gpu_mem *m)
{
	int i, rc = 0;

	n_free++;
	if (!m) {
		printf("free NULL\n");
		return 0;
	}
	/* gles_osal.h: "Free `mem` (no-op for an empty one)"; zephyr.c does the same */
	if (!m->impl && !m->gpu && !m->size) {
		printf("free empty\n");
		return 0;
	}
	i = rec_of(m);
	if (i < 0 || !R[i].live) {
		err("free of an unknown or already freed allocation gpu=%llx size=%zu (#%d)", (unsigned long long)m->gpu,
		    m->size, i + 1);
		memset(m, 0, sizeof(*m));
		return 0;
	}
	if (R[i].gpu != m->gpu || R[i].size != m->size)
		err("free #%d with a changed descriptor: gpu=%llx size=%zu (allocated gpu=%llx size=%zu)", i + 1,
		    (unsigned long long)m->gpu, m->size, (unsigned long long)R[i].gpu, R[i].size);
	printf("free #%d gpu=%llx size=%zu hash=%016llx%s%s\n", i + 1, (unsigned long long)R[i].gpu, R[i].size,
	       (unsigned long long)rec_hash(&R[i]), R[i].kind == K_MAPPHYS ? " (map_phys)" : "",
	       done_fence < last_fence ? " (kicks in flight)" : "");
	rec_unlink(i);
	if (R[i].kind != K_MAPPHYS)
		G[R[i].own_region].live = 0;
	if (R[i].kind != K_MAPPHYS) {
		if (use_base) {
			struct osal_gpu_mem bm = *m;

			bm.impl = R[i].base_impl;
			rc = base->gpu_free(&bm); /* a refusal (pending_frees.h) goes to the caller */
		} else {
			free(R[i].host);
		}
	}
	memset(m, 0, sizeof(*m));
	return rc;
}

static int r_mmu_map(uint64_t va, uint64_t phys, size_t size, uint32_t fl)
{
	size_t roff = 0;
	int g = region_of(phys, &roff), hit = -1;

	for (int i = live_head; i >= 0; i = R[i].next)
		if (va >= R[i].gpu && va < R[i].gpu + R[i].size && R[i].kind != K_MAPPHYS) {
			hit = i;
			break;
		}
	printf("mmu_map va=%llx size=%zu fl=%u -> ", (unsigned long long)va, size, fl);
	if (hit >= 0)
		printf("#%d+%llu", hit + 1, (unsigned long long)(va - R[hit].gpu));
	else
		printf("(no allocation)");
	if (g >= 0)
		printf(" phys=%s+%zu\n", region_name(g), roff);
	else
		printf(" phys=(not a registered region)\n");
	if (hit < 0 || va != R[hit].gpu || size > R[hit].size)
		err("mmu_map of a range that is not a whole live allocation");
	if (g < 0)
		err("mmu_map of physical memory the harness does not know");
	else if (roff + size > G[g].n)
		err("mmu_map range exceeds %s: off %zu + %zu > %zu", region_name(g), roff, size, G[g].n);
	if (hit >= 0 && g >= 0) {
		R[hit].region = g;
		R[hit].region_off = roff;
	}
	return 0;
}

static int r_map_phys(uint64_t heap, uint64_t phys, size_t size, uint32_t fl, struct osal_gpu_mem *out)
{
	size_t roff = 0;
	int g = region_of(phys, &roff), i;

	if (!out) {
		err("map_phys with out == NULL");
		return -22;
	}
	if (size == 0) {
		printf("map_phys heap=%llx size=0 -> -22\n", (unsigned long long)heap);
		return -22;
	}
	i = rec_new();
	R[i].kind = K_MAPPHYS;
	R[i].heap = heap;
	R[i].size = size;
	R[i].gpu = heap + MAPPHYS_VA + mp_bump;
	mp_bump += (size + PAGE - 1) & ~(uint64_t)(PAGE - 1);
	if (g >= 0) {
		R[i].region = g;
		R[i].region_off = roff;
		R[i].host = G[g].p + roff;
		if (roff + size > G[g].n)
			err("map_phys range exceeds %s: off %zu + %zu > %zu", region_name(g), roff, size, G[g].n);
	} else {
		err("map_phys of physical memory the harness does not know");
	}
	R[i].cpu = mapphys == 2 && !nocpu ? R[i].host : NULL;
	place(out, i);
	out->phys = phys;
	printf("map_phys #%d heap=%llx size=%zu fl=%u -> 0 gpu=%llx phys=%s%s\n", i + 1, (unsigned long long)heap, size, fl,
	       (unsigned long long)R[i].gpu, g >= 0 ? region_name(g) : "?", R[i].cpu ? " cpu" : " nocpu");
	return 0;
}

static int io_check(const char *op, struct osal_gpu_mem *m, size_t off, size_t n, int *ip, unsigned char **pp)
{
	int i = rec_of(m);
	size_t bn;

	if (i < 0 || !R[i].live) {
		err("%s on an unknown or freed allocation gpu=%llx off=%zu n=%zu", op, m ? (unsigned long long)m->gpu : 0ull,
		    off, n);
		return -22;
	}
	*pp = backing(&R[i], &bn);
	*ip = i;
	if (!*pp || off > R[i].size || n > R[i].size - off || off + n > bn) {
		err("%s #%d off=%zu n=%zu outside the %zu-byte allocation", op, i + 1, off, n, R[i].size);
		return -22;
	}
	return 0;
}

static int r_gpu_write(struct osal_gpu_mem *m, size_t off, const void *src, size_t n)
{
	int i = -1, rc;
	unsigned char *p = NULL;

	rc = io_check("gpu_write", m, off, n, &i, &p);
	/* a rejected length is not the size of the caller's buffer: no hash */
	printf("gpu_write #%d off=%zu n=%zu data=%016llx -> %d\n", i + 1, off, n,
	       (unsigned long long)(src && !rc ? hh_hash(src, n) : 0), rc);
	if (rc)
		return rc;
	memcpy(p + off, src, n);
	return 0;
}

static int r_gpu_read(struct osal_gpu_mem *m, size_t off, void *dst, size_t n)
{
	int i = -1, rc;
	unsigned char *p = NULL;

	rc = io_check("gpu_read", m, off, n, &i, &p);
	if (!rc)
		memcpy(dst, p + off, n);
	printf("gpu_read #%d off=%zu n=%zu data=%016llx -> %d\n", i + 1, off, n,
	       (unsigned long long)(rc ? 0 : hh_hash(dst, n)), rc);
	return rc;
}

/* cache ops get CPU pointers: report them relative to the allocation */
static void cache_op(const char *op, void *cpu, size_t n)
{
	unsigned char *c = cpu;
	int g;
	size_t roff;

	if (!cpu) {
		err("%s(NULL, %zu)", op, n);
		return;
	}
	/* The MOST RECENT live allocation that contains the pointer. Window
	 * surfaces and pixmap images map caller-owned pages, and the GL core
	 * never releases those mappings (egl.rs destroy_surface: no unmap exists
	 * in the OSAL yet), so a second window over the same buffer leaves two
	 * live allocations at one CPU address. Taking the first one found
	 * blamed the old, smaller mapping for the new one's invalidate and
	 * reported "runs past the end" for an access that stayed inside its own
	 * buffer (scen.win, mode l0mpc). The newest mapping is the one in use. */
	int hit = -1;

	for (int i = live_head; i >= 0; i = R[i].next) {
		unsigned char *b = R[i].cpu;

		if (!b || c < b || c >= b + R[i].size)
			continue;
		if (hit < 0 || i > hit)
			hit = i;
	}
	if (hit >= 0) {
		unsigned char *b = R[hit].cpu;

		printf("%s #%d gpu=%llx off=%zu n=%zu\n", op, hit + 1, (unsigned long long)R[hit].gpu, (size_t)(c - b), n);
		if (n > R[hit].size - (size_t)(c - b))
			err("%s #%d off=%zu n=%zu runs past the end of the %zu-byte allocation", op, hit + 1,
			    (size_t)(c - b), n, R[hit].size);
		return;
	}
	g = region_of((uint64_t)(uintptr_t)cpu, &roff);
	if (g >= 0) {
		printf("%s %s off=%zu n=%zu\n", op, region_name(g), roff, n);
		if (n > G[g].n - roff)
			err("%s runs past the end of %s", op, region_name(g));
		return;
	}
	for (size_t i = nR; i-- > 0;) {
		unsigned char *b = R[i].cpu;

		if (!R[i].live && b && c >= b && c < b + R[i].size && R[i].kind == K_ALLOC) {
			err("%s n=%zu inside FREED allocation #%zu (off %zu)", op, n, i + 1, (size_t)(c - b));
			return;
		}
	}
	err("%s n=%zu on memory that is not a live allocation", op, n);
}

static void r_cache_flush(void *cpu, size_t n) { cache_op("cache_flush", cpu, n); }
static void r_cache_invd(void *cpu, size_t n) { cache_op("cache_invd", cpu, n); }

#if defined(YAKOGL_GPU_V3D)
static void decode_kick(const struct v3d_kick *k)
{
	printf("  kick v3d magic=%08x bcl=%x..%x rcl=%x..%x tile_alloc=%x+%u tile_state=%x+%u\n", k->magic,
	       k->bcl_start, k->bcl_end, k->rcl_start, k->rcl_end, k->tile_alloc, k->tile_alloc_size,
	       k->tile_state, k->tile_state_size);
	if (k->magic != V3D_KICK_MAGIC)
		err("submit: kick magic %08x is not V3D_KICK_MAGIC", k->magic);
}
#else
static void decode_kick(const struct pvr_render_kick *k)
{
	const struct pvr_render_frag_regs *f = &k->frag;

	printf("  kick %ux%u samples=%u vdm=%llx ppp_ctrl=%x te_psg=%x\n", k->width, k->height, k->samples,
	       (unsigned long long)k->geom.vdm_ctrl_stream_base, k->geom.ppp_ctrl, k->geom.te_psg);
	printf("  kick usc_pix_out=%x clear=%x,%x,%x,%x bgobjdepth=%x bgobjvals=%x isp_aa=%x isp_ctl=%x\n",
	       f->usc_pixel_output_ctrl, f->usc_clear_register[0], f->usc_clear_register[1], f->usc_clear_register[2],
	       f->usc_clear_register[3], f->isp_bgobjdepth, f->isp_bgobjvals, f->isp_aa, f->isp_ctl);
	printf("  kick eot_info=%x eot_data=%x num_rt=%u", f->event_pixel_pds_info, f->event_pixel_pds_data,
	       f->num_rt);
	/* one PBE register pair per colour attachment (the driver's pbe_word is
	 * [PVR_RENDER_MAX_RT][2]; printing the row itself would print a host
	 * address, which ASLR changes between two runs of the same build) */
	for (unsigned rt = 0; rt < (f->num_rt ? f->num_rt : 1u) && rt < PVR_RENDER_MAX_RT; rt++)
		printf(" pbe[%u]=%llx,%llx", rt, (unsigned long long)f->pbe_word[rt][0],
		       (unsigned long long)f->pbe_word[rt][1]);
	printf(" pds_bgnd=%llx,%llx,%llx\n", (unsigned long long)f->pds_bgnd[0],
	       (unsigned long long)f->pds_bgnd[1], (unsigned long long)f->pds_bgnd[2]);
	printf("  kick dbias=%llx zlsctl=%llx zload=%llx sload=%llx flags=%x", (unsigned long long)f->isp_dbias_base,
	       (unsigned long long)f->isp_zlsctl, (unsigned long long)f->isp_zload_store_base,
	       (unsigned long long)f->isp_stencil_load_store_base, f->cmd_flags);
#ifdef HH_KICK_SCISSOR
	printf(" scissor=%llx", (unsigned long long)f->isp_scissor_base);
#endif
	printf("\n");
}
#endif

static void dump_to_files(const void *k, size_t len)
{
	const char *to = getenv("HH_DUMPTO");
	char fn[512];
	FILE *f;

	if (!to)
		return;
	for (int i = live_head; i >= 0; i = R[i].next) {
		size_t n;
		unsigned char *p = backing(&R[i], &n);

		if (!p)
			continue;
		snprintf(fn, sizeof(fn), "%s.%llx.bin", to, (unsigned long long)R[i].gpu);
		if ((f = fopen(fn, "wb"))) {
			fwrite(p, 1, n, f);
			fclose(f);
		}
	}
	snprintf(fn, sizeof(fn), "%s.kick.bin", to);
	if ((f = fopen(fn, "wb"))) {
		fwrite(k, 1, len, f);
		fclose(f);
	}
}

static int64_t r_submit(const void *k, size_t len)
{
	char tag[32];

#if defined(YAKOGL_GPU_V3D)
	const size_t need = sizeof(struct v3d_kick);
#else
	const size_t need = sizeof(struct pvr_render_kick);
#endif

	printf("submit #%lu len=%zu", n_submit, len);
	if (!k || len < need) {
		printf(" -> -22\n");
		err("submit with a short or missing kick descriptor (len %zu < %zu)", len, need);
		return -22;
	}
	printf(" kick=%016llx inflight=%lld\n", (unsigned long long)hh_hash(k, len), (long long)(last_fence - done_fence));
	decode_kick(k);
#if defined(YAKOGL_GPU_V3D)
	/* HH_BASE: linux.c's carve-out holds the lists; its submit checks them
	 * as the board's OSAL would and gives back what its frees held for the
	 * job */
	if (use_base) {
		int64_t bf = base->submit(k, len);

		if (bf < 0) {
			printf("  -> %lld (base)\n", (long long)bf);
			return bf;
		}
	}
#endif
	snprintf(tag, sizeof(tag), "submit#%lu", n_submit);
	dump(tag, 0);
	if (getenv("HH_DUMP") && strtoul(getenv("HH_DUMP"), NULL, 0) == n_submit)
		dump_to_files(k, len);
	n_submit++;
	last_fence++;
	if (last_fence - lag > done_fence)
		done_fence = last_fence - lag;
	printf("  -> fence %lld (completed %lld)\n", (long long)last_fence, (long long)done_fence);
	return last_fence;
}

static int r_wait(int64_t fence, int64_t timeout_us)
{
	int rc = (fence <= 0 || fence > last_fence) ? -22 : 0;

	n_wait++;
	/* a render that does not finish in time (only while one is pending) */
	if (!rc && wait_fail && fence > done_fence && n_wait % wait_fail == 0)
		rc = -110;
	if (!rc && fence > done_fence)
		done_fence = fence;
	printf("wait %lld t=%lld -> %d (completed %lld of %lld)\n", (long long)fence, (long long)timeout_us, rc,
	       (long long)done_fence, (long long)last_fence);
	return rc;
}

static int64_t r_completed(void)
{
	printf("completed -> %lld\n", (long long)done_fence);
	return done_fence;
}

static uint32_t r_reg_read(uint32_t off)
{
	printf("reg_read %x -> 0\n", off);
	return 0;
}

static void r_reg_write(uint32_t off, uint32_t v)
{
	printf("reg_write %x=%x\n", off, v);
}

static int r_query(uint32_t what, void *out, size_t sz)
{
	int rc;

	/* HH_BASE too: the border page then comes from linux.c's allocator
	 * through do_alloc, so record numbers and VAs match the other modes */
	if (!out) {
		rc = -22;
#if defined(YAKOGL_GPU_V3D)
	} else if (what == OSAL_QUERY_V3D_CPU_OF_GPU) {
		/* from this allocator's records (in HH_BASE too, whose records hold
		 * linux.c's addresses): the host copy, which exists even where the
		 * library was given no CPU view (HH_NOCPU) */
		struct osal_v3d_cpu_of *q = out;
		int j;

		rc = -22;
		if (sz == sizeof(*q)) {
			q->cpu = NULL;
			for (j = live_head; j >= 0; j = R[j].next) {
				if (R[j].host && q->gpu >= R[j].gpu && q->gpu - R[j].gpu + q->bytes <= R[j].size) {
					q->cpu = R[j].host + (q->gpu - R[j].gpu);
					rc = 0;
					break;
				}
			}
		}
	} else if (what == OSAL_QUERY_CLOCK_NS) {
		rc = -95;   /* no clock: a time in the log would differ between runs */
	} else if (what == OSAL_QUERY_V3D_JOB_OPEN && !use_base) {
		rc = 0;     /* frees are this allocator's: nothing held */
	} else if (what != OSAL_QUERY_RENDER_STATIC) {
		/* the V3D's settings (VPM, draws a job, ...), JOB_OPEN under
		 * HH_BASE: linux.c's answers */
		rc = base->query ? base->query(what, out, sz) : -95;
#endif
	} else if (what != OSAL_QUERY_RENDER_STATIC) {
		rc = -95;
	} else if (sz < sizeof(struct pvr_render_static)) {
		rc = -22;
	} else {
		struct pvr_render_static *st = out;
		struct osal_gpu_mem m;

		rc = 0;
		if (border < 0) {
			rc = do_alloc(HEAP_GENERAL, PAGE, OSAL_MEM_DATA, &m, K_BORDER);
			if (!rc)
				border = rec_of(&m);
		}
		if (!rc) {
			memset(st, 0, sizeof(*st));
			st->border_gpu = R[border].gpu;
			st->heap_base[PVR_RENDER_HEAP_GENERAL] = HEAP_GENERAL;
			st->heap_base[PVR_RENDER_HEAP_PDS] = HEAP_PDS;
			st->heap_base[PVR_RENDER_HEAP_USC] = HEAP_USC;
		}
	}
	printf("query %u size=%zu -> %d out=%016llx\n", what, sz, rc, (unsigned long long)(rc || !out ? 0 : hh_hash(out, sz)));
	return rc;
}

static void r_log(const char *fmt, ...)
{
	char buf[4096];
	va_list ap;
	size_t n;

	va_start(ap, fmt);
	vsnprintf(buf, sizeof(buf), fmt, ap);
	va_end(ap);
	n = strlen(buf);
	while (n && buf[n - 1] == '\n')
		buf[--n] = 0;
	/* a multi-line message keeps its number on every line */
	for (char *line = buf, *nl;; line = nl + 1) {
		nl = strchr(line, '\n');
		if (nl)
			*nl = 0;
		printf("log #%lu%s ", n_log, line == buf ? ":" : "+");
		for (const unsigned char *c = (const unsigned char *)line; *c; c++) {
			if (*c >= 0x20 && *c < 0x7f)
				putchar(*c);
			else
				printf("\\x%02x", *c);
		}
		putchar('\n');
		if (!nl)
			break;
	}
	n_log++;
}

/* ------------------------------------------------------------------ setup */

extern uint64_t hh_calls[];
extern const unsigned hh_nnames;
extern const char *const hh_names[];

static void at_end(void)
{
	const char *cf = getenv("HH_CALLS");

	dump("end", 1);
	printf("hh-end allocs=%lu frees=%lu live=%lld submits=%lu waits=%lu inits=%lu logs=%lu errors=%lu\n", n_alloc,
	       n_free, (long long)({ long long c = 0; for (int i = live_head; i >= 0; i = R[i].next) c++; c; }), n_submit,
	       n_wait, n_init, n_log, errors);
	fflush(stdout);
	if (cf) {
		FILE *f = fopen(cf, "w");

		if (f) {
			for (unsigned i = 0; i < hh_nnames; i++)
				fprintf(f, "%s %llu\n", hh_names[i], (unsigned long long)hh_calls[i]);
			fclose(f);
		}
	}
}

static void on_signal(int sig)
{
	printf("hh-crash signal %d\n", sig);
	fflush(stdout);
	signal(sig, SIG_DFL);
	raise(sig);
}

void __sanitizer_set_death_callback(void (*cb)(void)) __attribute__((weak));
static void on_sanitizer_death(void)
{
	printf("hh-crash sanitizer\n");
	fflush(stdout);
}

static long envl(const char *n, long def)
{
	const char *e = getenv(n);

	return e && *e ? strtol(e, NULL, 0) : def;
}

void hh_install(void)
{
	static char obuf[1 << 20];

	setvbuf(stdout, obuf, _IOFBF, sizeof(obuf));
	lag = (int)envl("HH_LAG", 0);
	fill = (unsigned)envl("HH_FILL", 0) & 0xff;
	nocpu = (int)envl("HH_NOCPU", 0);
	mapphys = (int)envl("HH_MAPPHYS", 0);
	nhash = (int)envl("HH_NHASH", 0);
	use_base = (int)envl("HH_BASE", 0);
	verbose = (int)envl("HH_VERBOSE", 0);
	fail_at = envl("HH_FAIL_AT", -1);
	fail_every = envl("HH_FAIL_EVERY", 0);
	fail_after = envl("HH_FAIL_AFTER", 48);
	wait_fail = envl("HH_WAIT_FAIL", 0);
	if (use_base && nocpu) {
		fprintf(stderr, "hostharness: HH_BASE=1 cannot model HH_NOCPU=1\n");
		exit(2);
	}
	printf("hh-config lag=%d fill=0x%02x nocpu=%d mapphys=%d nhash=%d base=%d\n", lag, fill, nocpu, mapphys, nhash,
	       use_base);
	base = g_osal;
	if (!base) {
		printf("HH-ERROR the library has no default OSAL\n");
		exit(2);
	}
	rec.init = r_init;
	rec.shutdown = r_shutdown;
	rec.gpu_alloc = r_alloc;
	rec.gpu_free = r_free;
	rec.mmu_map = r_mmu_map;
	rec.map_phys = mapphys ? r_map_phys : NULL;
	rec.gpu_write = r_gpu_write;
	rec.gpu_read = r_gpu_read;
	rec.cache_flush = r_cache_flush;
	rec.cache_invd = r_cache_invd;
	rec.submit = r_submit;
	rec.wait = r_wait;
	rec.completed = r_completed;
	rec.reg_read = r_reg_read;
	rec.reg_write = r_reg_write;
	rec.query = r_query;
	rec.log = r_log;
	osal_set_ops(&rec);
	atexit(at_end);
	signal(SIGABRT, on_signal);
	signal(SIGSEGV, on_signal);
	signal(SIGILL, on_signal);
	signal(SIGBUS, on_signal);
	signal(SIGFPE, on_signal);
	if (__sanitizer_set_death_callback)
		__sanitizer_set_death_callback(on_sanitizer_death);
}

/* the driver's platform hooks gl_test.c references (no GPU here) */
unsigned long pvr_regs(void) { return 0; }
int pvr_render_ta3d(unsigned long r) { (void)r; return 0; }
void pvr_render_perf_read(uint32_t out[7]) { memset(out, 0, 7 * sizeof(out[0])); }
void pvr_render_fl_counters(uint32_t out[9]) { memset(out, 0, 9 * sizeof(out[0])); }
