/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * baremetal.c - bare-metal OSAL backend skeleton for YakoGL on PowerVR
 * BXS-4-64 (no OS: direct MMIO, a static DDR carveout, and a spin-wait).
 *
 * This is the SECOND backend behind the same gles_osal.h interface - it exists
 * to prove the portability seam is real: the portable GL core + PVR backend
 * compile and link against this without change. It is a skeleton: the pieces
 * that genuinely need board/SoC knowledge (register base, carveout base, GPU
 * MMU programming, firmware boot, cache maintenance instructions) are marked
 * TODO(board). The structure mirrors the Zephyr backend so a real port fills in
 * the blanks rather than redesigning.
 *
 * The board registers itself before init (include/gles_osal_baremetal.h:
 * osal_bm_board_config()): register base, carveout, and the hooks for boot,
 * GPU-MMU mapping and cache maintenance. There is no default address and no
 * do-nothing hook: init refuses, and touches no address, until a complete
 * board is registered.
 */

#include <stdbool.h>
#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdarg.h>
#include <stdio.h>

#include "gles_osal.h"
#include "gles_osal_baremetal.h"
#include "osal_internal.h"
#include "osal_bounds.h"

/* ---- MMIO --------------------------------------------------------- */
static inline uint32_t mmio_r32(uintptr_t addr)
{
	uint32_t v = *(volatile uint32_t *)addr;
	__asm__ volatile("" ::: "memory");
	return v;
}
static inline void mmio_w32(uintptr_t addr, uint32_t v)
{
	*(volatile uint32_t *)addr = v;
	__asm__ volatile("" ::: "memory");
}

/* ---- backend state ------------------------------------------------ */
static struct {
	int       configured;      /* osal_bm_board_config() stored `board` */
	struct osal_bm_board board;
	int       inited;
	uintptr_t regs;
	uint64_t  carveout_phys;
	void     *carveout_cpu;
	size_t    carveout_size;
	size_t    render_next;
	struct { uint64_t base, next_off; } heaps[8];
	unsigned  num_heaps;
	int64_t   fence_seq;
} b;

#define OSAL_PAGE 0x1000u

/* n rounded up to a whole page, or false when that does not fit a size_t
 * (a size near SIZE_MAX would round to 0). */
static bool bm_page_round(size_t n, size_t *out)
{
	if (n > SIZE_MAX - (OSAL_PAGE - 1)) {
		return false;
	}
	*out = (n + (OSAL_PAGE - 1)) & ~(size_t)(OSAL_PAGE - 1);
	return true;
}

static uint64_t *bm_heap_bump(uint64_t heap_base)
{
	for (unsigned i = 0; i < b.num_heaps; i++) {
		if (b.heaps[i].base == heap_base) {
			return &b.heaps[i].next_off;
		}
	}
	if (b.num_heaps >= (unsigned)(sizeof(b.heaps) / sizeof(b.heaps[0]))) {
		return NULL;
	}
	b.heaps[b.num_heaps].base = heap_base;
	b.heaps[b.num_heaps].next_off = 0;
	return &b.heaps[b.num_heaps++].next_off;
}

static void bm_log(const char *fmt, ...);

/* ---- board registration ------------------------------------------- */
int osal_bm_board_config(const struct osal_bm_board *board)
{
	if (!board) {
		return -22; /* -EINVAL */
	}
	if (b.inited) {
		return -16; /* -EBUSY */
	}
	b.board = *board;
	b.configured = 1;
	return 0;
}

/* Why the registered board cannot be used, or NULL when it can. Reads only
 * the struct: no address in it is touched. */
static const char *bm_board_refusal(void)
{
	const struct osal_bm_board *c = &b.board;

	if (!b.configured) {
		return "no board registered (call osal_bm_board_config() before init)";
	}
	if (c->reg_base == 0) {
		return "reg_base is 0";
	}
	if (c->carveout_phys == 0) {
		return "carveout_phys is 0";
	}
	if ((c->carveout_phys & (OSAL_PAGE - 1)) != 0) {
		return "carveout_phys is not 4 KiB aligned";
	}
	if (c->carveout_cpu == NULL) {
		return "carveout_cpu is NULL";
	}
	if (c->carveout_size == 0) {
		return "carveout_size is 0";
	}
	if (c->carveout_size > UINT64_MAX - c->carveout_phys ||
	    c->carveout_size > UINTPTR_MAX - (uintptr_t)c->carveout_cpu) {
		return "the carveout wraps the address space";
	}
	if (c->render_pool_off >= c->carveout_size) {
		return "render_pool_off is not below carveout_size";
	}
	if (!c->boot) {
		return "boot hook is NULL";
	}
	if (!c->mmu_map) {
		return "mmu_map hook is NULL";
	}
	if (!c->cache_clean) {
		return "cache_clean hook is NULL";
	}
	if (!c->cache_invalidate) {
		return "cache_invalidate hook is NULL";
	}
	return NULL;
}

/* ---- lifecycle ---------------------------------------------------- */
static int bm_init(void *platform_arg)
{
	(void)platform_arg; /* the board comes from osal_bm_board_config() */

	const char *why = bm_board_refusal();
	if (why) {
		bm_log("yakogl baremetal: init refused: %s\n", why);
		return -22; /* -EINVAL */
	}

	b.regs = b.board.reg_base;
	b.carveout_phys = b.board.carveout_phys;
	b.carveout_cpu = b.board.carveout_cpu;
	b.carveout_size = b.board.carveout_size;
	b.render_next = b.board.render_pool_off;
	b.num_heaps = 0;
	b.fence_seq = 0;

	int rc = b.board.boot(b.regs);
	if (rc < 0) {
		return rc;
	}
	b.inited = 1;
	return 0;
}

static void bm_shutdown(void)
{
	b.inited = 0;
}

/* ---- GPU memory + MMU --------------------------------------------- */
static int bm_gpu_alloc(uint64_t heap_base, size_t size, uint32_t flags,
			struct osal_gpu_mem *out)
{
	if (!b.inited || !out) {
		return -22; /* -EINVAL */
	}
	uint64_t *bump = bm_heap_bump(heap_base);
	if (!bump) {
		return -12; /* -ENOMEM */
	}
	(void)flags;

	size_t msz, co, hb;
	if (!bm_page_round(size, &msz) || !bm_page_round(b.render_next, &co) ||
	    !bm_page_round((size_t)*bump, &hb)) {
		return -12; /* -ENOMEM */
	}
	if (co > b.carveout_size || msz > b.carveout_size - co) {
		return -12; /* -ENOMEM */
	}
	b.render_next = co + msz;
	*bump = hb + msz;

	uint64_t phys = b.carveout_phys + co;
	void *cpu = (uint8_t *)b.carveout_cpu + co;
	uint64_t gpu = heap_base + hb;

	memset(cpu, 0, msz);
	b.board.mmu_map(gpu, phys, msz, OSAL_L0_FLAGS_DATA);

	out->cpu  = cpu;
	out->gpu  = gpu;
	out->phys = phys;
	out->size = msz;
	/* The GL core reads impl == NULL as "no storage" (R-10), so the
	 * allocation's handle is its CPU address, as linux.c does: never NULL,
	 * because init refused a NULL or wrapping carveout. */
	out->impl = cpu;
	return 0;
}

/* No pending-free queue here: the carveout is a bump allocator that never
 * takes memory back (and bare metal submits nothing yet), so there is
 * nothing to defer. Never refused. The descriptor is cleared, impl
 * included, as the other OSALs do: it no longer names storage. */
static int bm_gpu_free(struct osal_gpu_mem *mem)
{
	if (mem) {
		memset(mem, 0, sizeof(*mem));
	}
	return 0;
}

/* Physical memory the caller owns at a fresh heap VA: no carveout pages. */
static int bm_map_phys(uint64_t heap_base, uint64_t phys, size_t size, uint32_t flags,
		       struct osal_gpu_mem *out)
{
	uint64_t *bump = b.inited ? bm_heap_bump(heap_base) : NULL;
	size_t msz, hb;

	(void)flags;
	if (!bump || !out || size == 0 || (phys & (OSAL_PAGE - 1)) != 0 ||
	    !bm_page_round(size, &msz) || !bm_page_round((size_t)*bump, &hb)) {
		return -22; /* -EINVAL */
	}
	*bump = hb + msz;
	b.board.mmu_map(heap_base + hb, phys, msz, OSAL_L0_FLAGS_DATA);
	out->cpu  = NULL;
	out->gpu  = heap_base + hb;
	out->phys = phys;
	out->size = msz;
	out->impl = (void *)1;
	return 0;
}

static int bm_mmu_map(uint64_t gpu_va, uint64_t phys, size_t size, uint32_t flags)
{
	size_t msz;

	(void)flags;
	if (!b.inited || !bm_page_round(size, &msz)) {
		return -22; /* -EINVAL */
	}
	b.board.mmu_map(gpu_va, phys, msz, OSAL_L0_FLAGS_DATA);
	return 0;
}

static int bm_gpu_write(struct osal_gpu_mem *mem, size_t off, const void *src, size_t n)
{
	if (!b.inited || !osal_gpu_copy_ok(mem, off, src, n)) {
		return -22;
	}
	memcpy((uint8_t *)mem->cpu + off, src, n);
	b.board.cache_clean((uint8_t *)mem->cpu + off, n);
	return 0;
}

static int bm_gpu_read(struct osal_gpu_mem *mem, size_t off, void *dst, size_t n)
{
	if (!b.inited || !osal_gpu_copy_ok(mem, off, dst, n)) {
		return -22;
	}
	b.board.cache_invalidate((uint8_t *)mem->cpu + off, n);
	memcpy(dst, (uint8_t *)mem->cpu + off, n);
	return 0;
}

/* ---- cache maintenance -------------------------------------------- */
/* Before init there is no GPU memory to maintain, and no hook to call. */
static void bm_cache_flush(void *cpu, size_t n)
{
	if (b.inited) {
		b.board.cache_clean(cpu, n);
	}
}
static void bm_cache_invd(void *cpu, size_t n)
{
	if (b.inited) {
		b.board.cache_invalidate(cpu, n);
	}
}

/* ---- submit + completion ------------------------------------------ *
 * The kick descriptor is the driver render service's struct pvr_render_kick
 * (see osal_internal.h). Bare metal has no port of the render service yet
 * (TODO(board): build src/pvr_render.c against the board's hooks; that port
 * adds a KCCB-post hook to struct osal_bm_board), so a submit is refused
 * rather than silently dropped. */
static int64_t bm_submit(const void *kick_desc, size_t kick_desc_len)
{
	(void)kick_desc;
	(void)kick_desc_len;
	if (!b.inited) {
		return -22;
	}
	return -95; /* -ENOTSUP */
}

static int bm_wait(int64_t fence, int64_t timeout_us)
{
	(void)fence;
	(void)timeout_us;
	return -95; /* -ENOTSUP */
}

/* ---- queries -------------------------------------------------------- */
static int bm_query(uint32_t what, void *out, size_t out_sz)
{
	(void)what;
	(void)out;
	(void)out_sz;
	return -95; /* -ENOTSUP: no render service on bare metal yet */
}

/* ---- registers ---------------------------------------------------- */
/* Before init there is no register block: a read gives 0 and a write is
 * dropped, each with a log line, rather than an access at address `off`. */
static uint32_t bm_reg_read(uint32_t off)
{
	if (!b.inited) {
		bm_log("yakogl baremetal: reg_read(0x%x) before init refused\n", (unsigned)off);
		return 0;
	}
	return mmio_r32(b.regs + off);
}
static void bm_reg_write(uint32_t off, uint32_t v)
{
	if (!b.inited) {
		bm_log("yakogl baremetal: reg_write(0x%x) before init refused\n", (unsigned)off);
		return;
	}
	mmio_w32(b.regs + off, v);
}

/* ---- diagnostics -------------------------------------------------- */
static void bm_log(const char *fmt, ...)
{
	/* TODO(board): route to the board UART. Uses printf here so a semihosted
	 * or newlib-nano target logs out of the box. */
	va_list ap;

	va_start(ap, fmt);
	vprintf(fmt, ap);
	va_end(ap);
}

/* ------------------------------------------------------------------ */
const struct osal_ops osal_baremetal_ops = {
	.init        = bm_init,
	.shutdown    = bm_shutdown,
	.gpu_alloc   = bm_gpu_alloc,
	.gpu_free    = bm_gpu_free,
	.mmu_map     = bm_mmu_map,
	.map_phys    = bm_map_phys,
	.gpu_write   = bm_gpu_write,
	.gpu_read    = bm_gpu_read,
	.cache_flush = bm_cache_flush,
	.cache_invd  = bm_cache_invd,
	.submit      = bm_submit,
	.wait        = bm_wait,
	.completed   = NULL,       /* no submits: nothing is ever in flight */
	.reg_read    = bm_reg_read,
	.reg_write   = bm_reg_write,
	.query       = bm_query,
	.log         = bm_log,
	/* no vertical sync here: presentation is immediate */
	.wait_vblank = NULL,
	.present     = NULL,
	/* one flow of control: there is no second thread to tell apart, so
	 * thread_id stays NULL (the same as always 0: nothing is checked) */
	.thread_id   = NULL,
};

void osal_baremetal_register(void)
{
	osal_set_ops(&osal_baremetal_ops);
}
