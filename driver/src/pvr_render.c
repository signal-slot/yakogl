/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2023 Imagination Technologies Ltd.
 * Derived from Mesa 26.2.1 src/imagination (common/device_info/bxs-4-64.h,
 * csbgen; MIT). The upstream part stays under its licence
 * (LICENSES/MIT.txt); our changes are Apache-2.0. See NOTICE.
 *
 * pvr_render.c -- the render service for the PowerVR BXS-4-64 (DDK 25.2
 * firmware ABI), the C side. The firmware side -- the render context, the
 * per-size HWRT datasets, the client CCBs, the submit and the fence wait --
 * is Rust (driver/rust/src/gpu/pvr/render.rs; docs/source-form.md step 7,
 * driver/docs/pvr-kernel-rust.md). What stays here: the static PDS/USC areas
 * (they call the command-stream generators of group B), the parameter
 * buffer and freelist sizing, the render pool's API over its free lists
 * (Rust: gpu::pvr::pool, with its host tests), the run-time switches and the
 * API YakoGL's OSAL calls (pvr_render.h).
 */

#include <zephyr/kernel.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <zephyr/cache.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/logging/log.h>
#include <string.h>
#include <errno.h>

#include "pvr_ddk.h"
#include <yakogl_drivers.h>
#include "pvr_internal.h"
#include "pvr_drv.h"
#include "pvr_render.h"
#include "pvr_render_regs.h"
#include "pvr_usc_eot_blob.h"
#include "pvr_usc_vs_passthrough_blob.h"
#include "pvr_pds_render_gen.h"
#include "pvr_pds_pixel_event_gen.h"

/* The DOUTU temps field wraps rather than saturating (see
 * PVR_PDS_DOUTU_MAX_TEMPS), and pvr_pds_pe_build_doutu_src0() cannot refuse,
 * so the end-of-tile blobs' temp counts are checked where they are known: at
 * compile time. Both constants feed that function from this file and from
 * pvr_fw_boot.c, and a constant is checked once wherever it is visible. */
_Static_assert(PVR_USC_EOT_TEMPS <= PVR_PDS_DOUTU_MAX_TEMPS,
	       "EOT shader needs more temporaries than the DOUTU field can hold");
_Static_assert(PVR_USC_EOT_TB_TEMPS <= PVR_PDS_DOUTU_MAX_TEMPS,
	       "EOT tile-buffer shader needs more temporaries than the DOUTU field can hold");

LOG_MODULE_DECLARE(gpu_pvr, CONFIG_YAKOGL_DRV_GPU_PVR_LOG_LEVEL);


/* Parameter-buffer region reserved at GENERAL heap offset 0 (the PM writes
 * tiled geometry at VA 0x8000000000 through the main MMU). */
#define PB_RESERVE_BYTES 0x200000u

/* Static VDM_SYNC USC program (Mesa static_data_areas: pco_usclib
 * vs_nop_common = one `nop.end` instruction group; it is also the trailing
 * group of every pco vertex shader, see pvr_usc_vs_passthrough_blob.h).
 *
 * It MUST be exactly this: an earlier 4-dword form with two leading dwords
 * (0x00000000 0x00000200) had no END in its first group, so the USC ran on
 * from offset 0 through the zero-filled page into whatever was mapped at
 * USC 0x1000. The self-test rig happened to keep its VS there (whose END
 * stopped the runaway); with the EOT program there instead the TA hung
 * before its first PM allocation (HWR dm=2, PM_CAT_* = 0, no MMU fault).
 *
 * The words are pvr_usc_vs_nop_words, which our encoder builds
 * (assemble::build_vs_nop, list line `vdm_sync`); the generator refuses them
 * unless they equal pco's. */

static struct {
	bool inited;
	/* per-heap device-VA bumps (render pool); the general heap's is RR.gen_b */
	uint32_t pds_b, usc_b;
	struct pvr_gpu_bo pbuf, border, usc0, pds0, eot;
	uint64_t eot_doutu;
	struct pvr_gpu_bo eot_n[PVR_RENDER_MAX_RT + 1];
	uint64_t eot_doutu_n[PVR_RENDER_MAX_RT + 1];
	/* the same programs for a render whose last attachment comes from a
	 * tile buffer, and the tile buffers themselves */
	struct pvr_gpu_bo eot_tb_n[PVR_RENDER_MAX_RT + 1];
	uint64_t eot_doutu_tb_n[PVR_RENDER_MAX_RT + 1];
	struct pvr_gpu_bo tile_buf[PVR_RENDER_TILE_BUFFERS];
} R;

/* The firmware side's state (gpu::pvr::render). */
static struct yd_pvr_render RR;

/* ---------------------------------------------------------------------- */

static void flush(const void *p, size_t n)
{
	sys_cache_data_flush_range((void *)p, n);
}

static void flush_bo(const struct pvr_gpu_bo *bo, size_t n)
{
	flush(bo->cpu, n);
}

static uint32_t *heap_bump(enum pvr_render_heap heap)
{
	switch (heap) {
	case PVR_RENDER_HEAP_GENERAL: return &RR.gen_b;
	case PVR_RENDER_HEAP_PDS:     return &R.pds_b;
	case PVR_RENDER_HEAP_USC:     return &R.usc_b;
	default:                      return NULL;
	}
}

static uint64_t heap_base(enum pvr_render_heap heap)
{
	switch (heap) {
	case PVR_RENDER_HEAP_GENERAL: return PVR_GENERAL_HEAP_BASE;
	case PVR_RENDER_HEAP_PDS:     return PVR_PDSCODEDATA_HEAP_BASE;
	case PVR_RENDER_HEAP_USC:     return PVR_USCCODE_HEAP_BASE;
	default:                      return 0;
	}
}

int pvr_render_map_phys(enum pvr_render_heap heap, uint64_t phys, uint32_t size,
			struct pvr_gpu_bo *out)
{
	uint32_t *bump = heap == PVR_RENDER_HEAP_USC ? &R.usc_b :
			 heap == PVR_RENDER_HEAP_PDS ? &R.pds_b : &RR.gen_b;
	uint32_t msz = ROUND_UP(size, 0x1000), hb;

	if (!R.inited || !out || size == 0 || (phys & 0xfffu) != 0)
		return -EINVAL;
	hb = ROUND_UP(*bump, 0x1000);
	*bump = hb + msz;
	pvr_mmu_map(heap_base(heap) + hb, phys, msz, PVR_L0_FLAGS_DATA);
	out->gpu = heap_base(heap) + hb;
	out->cpu = NULL;
	out->heap_off = hb;
	return 0;
}

/* Give a freed block back to the bump when it is the newest allocation
 * (gpu::pvr::pool, through gpu::pvr::mem's bo_rollback). Opt-in until the
 * board has said something about it: #54 is a pool HIGH-WATER problem, and the
 * obvious pass/fail gate for it was void (pooltest's texture workload failed
 * zero times at the board's real budget before any change), so the flag lets
 * one module run both arms.
 */
bool pvr_render_pool_rollback;         /* 0 = off, the default */

/* The render pool's free lists and counters (Rust: gpu::pvr::pool, which has
 * what each one does, and its host tests;
 * docs/notes/pvr-render-pool.md). */
static struct yd_pvr_pool g_pool;

static uint32_t *const *pool_bumps(void)
{
	static uint32_t *const bumps[3] = {
		[PVR_RENDER_HEAP_GENERAL] = &RR.gen_b,
		[PVR_RENDER_HEAP_PDS] = &R.pds_b,
		[PVR_RENDER_HEAP_USC] = &R.usc_b,
	};

	return bumps;
}

static inline struct yd_pvr_bo yd_bo(const struct pvr_gpu_bo *b)
{
	return (struct yd_pvr_bo){ .gpu = b->gpu, .cpu = b->cpu, .heap_off = b->heap_off };
}

/* A free list first, the bump second. SLC-bypassed, like the DDK's
 * GPU_UNCACHED allocations. Measured on 2026-09-15 (gl_test case 39,
 * gl_slint): mapping the GL heaps SLC-cached with a host flush+invalidate
 * before every kick changed no probe and not the UI frame rate, so the
 * simpler policy stays. */
/* The pool's summary at INF (pvr_render.h). */
/* The firmware's HWR counter when the render service started: recoveries
 * are counted from it (pvr_render_hwr_recoveries). */
static uint32_t g_hwr_at_init;

uint32_t pvr_render_hwr_recoveries(void)
{
	return R.inited ? yd_pvr_render_hwr_counter(pvr_km()) - g_hwr_at_init : 0;
}

void pvr_render_pool_report(void)
{
	if (R.inited) {
		yd_pvr_pool_report(&g_pool, pvr_km(), pool_bumps());
		yd_pvr_render_grow_report(pvr_km(), g_hwr_at_init);
	}
}

/* A wedge must reach the GPU_STOP record before the board's watchdog: the
 * stop's time is at most half the "alive" window (gpu::pvr::render
 * STOP_WAIT_MS; fwccb-2's wait timed out in the same second as the RTI). */
#ifdef CONFIG_YAKOGL_BOARD_ALIVE_S
BUILD_ASSERT(CONFIG_YAKOGL_BOARD_ALIVE_S * 1000 >= 2 * YD_PVR_STOP_WAIT_MS,
	     "the render stop's time must be at most half the board's alive window");
#endif

/* The stop (pvr_render.h): set before the first kick, read on the first
 * -ESHUTDOWN. Not reset by pvr_render_init: the handler is the board's.
 * gpu::pvr returns Zephyr's errno numbers (driver/rust/src/lib.rs, errno);
 * the Linux harness's are not these, and there no handler is set. */
#ifdef __ZEPHYR__
BUILD_ASSERT(ESHUTDOWN == 110, "gpu::pvr's errno::ESHUTDOWN is Zephyr's");
#endif
static pvr_render_stop_fn g_stop_fn;
static bool g_stop_handled;

void pvr_render_set_stop_handler(pvr_render_stop_fn fn)
{
	g_stop_fn = fn;
}

/* The first -ESHUTDOWN of a stopped render service: the driver's run-end
 * lines, then the handler. The GL side's lines (the owner report's GL rows,
 * the backend's) are not printed here: this runs inside a GL call. */
static void render_stopped(void)
{
	const struct yd_pvr_km *k = pvr_km();

	if (g_stop_handled) {
		return;
	}
	g_stop_handled = true;
	pvr_render_pool_report();
	if (g_stop_fn) {
		g_stop_fn(k->stop_cause, k->stop_fence, pvr_render_hwr_recoveries(), k->stop_fl,
			  k->stop_kib);
	}
}

/* The render service's owner lines (pvr_render.h): the one-time objects
 * counted here, where they were made (render_init, static_areas_init), the
 * rest by gpu::pvr::render. */
uint64_t pvr_render_pool_owners(void)
{
	uint32_t st[3] = { 0 }, n = 0;
	const struct { const struct pvr_gpu_bo *bo; uint32_t bytes; int heap; } objs[] = {
		{ &R.pbuf, PB_RESERVE_BYTES, 0 },
		{ &R.border, 4096, 0 },
		{ &R.usc0, 4096, 2 },
		{ &R.pds0, 4096, 1 },
	};

	if (!R.inited) {
		return 0;
	}
	for (unsigned i = 0; i < ARRAY_SIZE(objs); i++) {
		if (objs[i].bo->cpu) {
			st[objs[i].heap] += ROUND_UP(objs[i].bytes, 0x1000);
			n++;
		}
	}
	for (unsigned n_rt = 1; n_rt <= PVR_RENDER_MAX_RT; n_rt++) {
		if (R.eot_n[n_rt].cpu) {
			st[2] += ROUND_UP(pvr_usc_eot_n_dwords[n_rt] * 4u, 0x1000);
			n++;
		}
		if (R.eot_tb_n[n_rt].cpu) {
			st[2] += ROUND_UP(pvr_usc_eot_tb_n_dwords[n_rt] * 4u, 0x1000);
			n++;
		}
	}
	for (unsigned i = 0; i < PVR_RENDER_TILE_BUFFERS; i++) {
		if (R.tile_buf[i].cpu) {
			st[0] += ROUND_UP(PVR_TILE_BUFFER_BYTES, 0x1000);
			n++;
		}
	}
	return yd_pvr_render_pool_owners(pvr_km(), &RR, st, n);
}

void pvr_render_pool_usage(uint64_t *handed_out, uint64_t *free_bytes)
{
	uint64_t u[2] = { 0, 0 };

	if (R.inited) {
		yd_pvr_pool_usage(&g_pool, pvr_km(), pool_bumps(), u);
	}
	*handed_out = u[0];
	*free_bytes = u[1];
}

int pvr_render_alloc(enum pvr_render_heap heap, uint32_t size, struct pvr_gpu_bo *out)
{
	struct yd_pvr_bo bo;
	int rc;

	if (!R.inited || !heap_bump(heap) || !out || size == 0) {
		return -EINVAL;
	}
	rc = yd_pvr_pool_alloc(&g_pool, pvr_km(), pool_bumps(), (uint32_t)heap, size, &bo);
	if (rc) {
		return rc;
	}
	*out = (struct pvr_gpu_bo){ .gpu = bo.gpu, .cpu = bo.cpu, .heap_off = bo.heap_off };
	return 0;
}

void pvr_render_free(const struct pvr_gpu_bo *bo, uint32_t size)
{
	struct yd_pvr_bo b;

	if (!R.inited || !bo || size == 0) {
		return;
	}
	b = yd_bo(bo);
	yd_pvr_pool_free(&g_pool, pvr_km(), pool_bumps(), &b, size, pvr_render_pool_rollback);
}

void pvr_render_perf_read(uint32_t out[7])
{
	mm_reg_t regs = pvr_regs();
	static const uint32_t cr[6] = { 0x6020, 0x6028, 0x6038, 0x60B8, 0x60C0, 0x6220 };

	for (int i = 0; i < 6; i++) {
		out[i] = pvr_rd32(regs, cr[i]);
	}
	out[6] = (uint32_t)pvr_rd64(regs, 0x0160);
}

/* One COMBINED_GEOM_FRAG_KICK (the DDK's normal path) or two separate KICKs
 * (its exception path, and what this driver did until 2026-09-22). Default is
 * the DDK's. It is set at compile time by whoever links the driver. */
bool pvr_render_combined_kick = true;

/* Re-ring the KCCB doorbell every 8 ms while waiting for a fence. A probe for
 * the "the last kick is never applied" invariant; see pvr_render_wait(). */
bool pvr_render_wait_rering;

/* Re-POST the last kick (not just the doorbell) if the fence has not moved.
 * The two probes answer different questions: a bare doorbell asks whether the
 * firmware merely missed the notification and already has everything it
 * needs; re-posting asks whether it needs another COMMAND, which would mean
 * the KICK handler consumed the first and deferred its write-offset update to
 * the next command rather than applying it.
 *
 * ON BY DEFAULT, as a MITIGATION (2026-09-24): the firmware sits on a posted
 * kick until another command arrives, and the root cause is not found. On the
 * AM62P, gl_test case 13 alone stalled in about half its runs (two 20 s
 * timeouts, then a read-back of the previous frame); with the re-post, 0 of 60
 * runs stalled and the log showed it firing (27 times). A kick that is only
 * slow (past 20 ms) is re-posted too; that was harmless in those runs.
 * pvr_render_reposts counts them; the log line is rate-limited. */
bool pvr_render_wait_repost = true;
uint32_t pvr_render_reposts;

/* The two halves of what 06bbb930 changed in mmu_cache_sync(), separable
 * because they answer different questions: mmu_ack is whether we wait for the
 * firmware's acknowledgement of the MMU cache invalidate, mmu_gate is whether
 * we send the invalidate at all when no mapping has changed. Both default to
 * the safe answer. Not probes -- these defaults are what the driver should
 * ship -- but parameters, so the four arrangements can be measured from one
 * binary. mmu_ack=1, mmu_gate=0 is exactly the pre-06bbb930 function. */
bool pvr_render_mmu_ack = true;
bool pvr_render_mmu_gate = true;

/* Print-only instrument for "MMUCACHE ack not seen" (off by default; it
 * changes no command and no wait the driver makes). On: every invalidate logs
 * its sequence value, its KCCB slot and how long the acknowledgement took;
 * a miss also logs the KCCB offsets and the slot's return flags, polls the
 * sync word for a further 50 ms (printing it at +10 and +50 ms and when the
 * value first appears), and, if it still has not, reads it once more after
 * the kick's doorbell. The question it answers: is the acknowledgement late
 * (the firmware was busy), never written, or written only once the next
 * command's doorbell rings (a missed notification). */
bool pvr_render_mmu_ack_probe;
/* Re-ring the KCCB doorbell every N ms while the MMUCACHE acknowledgement
 * wait is pending (0: never, the default). A missed acknowledgement on the
 * AM62P is released by the next doorbell and by nothing else
 * (records/am62p/mmuack-instr, pm-latency); this measures whether a bare
 * doorbell releases it. Printed when it does. */
uint32_t pvr_render_mmu_ack_reping;

/* Fail the submit when the render-target pair member this size needs is still
 * in flight, instead of handing it out anyway.
 *
 * Proceeding lets the next geometry phase tile into the region headers and
 * tail pointers the owning fragment phase is still reading. It does not fail
 * where the mistake is made: measured 2026-09-22, the one observed instance
 * was followed by twenty timeouts in the same client session at the wait's own
 * 20 s period, and the session never completed another render -- so proceeding
 * is what turns ONE stalled render into a wedged session. It is an amplifier,
 * not the origin: the wait timed out because a render was already stuck.
 *
 * Off by default because it has never been measured. 182a80f made the same
 * change, was reverted on a 96/100 soak, and that verdict does not stand --
 * its own counters read zero, so the code never ran on that workload.
 * This carries the refusal ALONE; 182a80f also moved
 * the recycle timeout from 2 s to 20 s, and that rider stays out. */
bool pvr_render_hwrt_refuse;

/* Build the global lists with one grow's reserve instead of four, and
 * refill it from the render pool after each served grow
 * (CONFIG_YAKOGL_DRV_GPU_PVR_FL_GROW_ON_DEMAND; gpu::pvr::freelist::serve and
 * docs/notes/pvr-render-pool.md, "Next: refract's 7600 KiB").
 * It frees 2 x 768 pages at the 64 MiB carveout's sizing, and a grow is refused
 * only when the render pool refused the refill and refuses again when the
 * firmware asks. Off until gl_test and glmark2 have run with it. */
#ifndef PVR_FL_GROW_ON_DEMAND_DEFAULT
#define PVR_FL_GROW_ON_DEMAND_DEFAULT 0
#endif
bool pvr_render_fl_grow_on_demand = PVR_FL_GROW_ON_DEMAND_DEFAULT;

/* Keep the firmware's ready pages in the global freelists, as
 * drm/imagination does with Mesa's threshold
 * (CONFIG_YAKOGL_DRV_GPU_PVR_FL_READY_PAGES; gpu::pvr::freelist::ready_pages).
 * Off until gl_test and glmark2 have run with it, knob off and on. */
#ifndef PVR_FL_READY_PAGES_DEFAULT
#define PVR_FL_READY_PAGES_DEFAULT 0
#endif
bool pvr_render_fl_ready_pages = PVR_FL_READY_PAGES_DEFAULT;

/* Hand the firmware its partial render: the geometry command's PR fence
 * words {timeline, seq - 1}, a second geometry UPDATE to a timeline word,
 * and per kick a FENCE_PR, a 3D_PR (a byte copy of the 3D command, the
 * single-pass shape per Mesa pvr_arch_cmd_buffer.c:1736-1747) and an UPDATE,
 * before today's FENCE + 3D + UPDATE -- the order of Linux drm/imagination's
 * fragment queue (pvr_queue.c:698-737) and the fence arithmetic of
 * Mesa's srvkm (pvr_arch_srv_job_render.c:531-569, :870-902).
 * (CONFIG_YAKOGL_DRV_GPU_PVR_PARTIAL_RENDER; gpu::pvr::render::submit and
 * docs/notes/pvr-render-pool.md, "Design: the firmware's partial render".)
 * It is what the firmware waits for when the geometry phase is out of
 * parameter memory with no 3D render running to free some: today such a
 * kick never completes and ends in the 10 s stop (gl_test case 175,
 * refusehang). Independent of the grow knob. Off until gl_test case 175 and
 * glmark2 have run with it, knob off and on. */
#ifndef PVR_PARTIAL_RENDER_DEFAULT
#define PVR_PARTIAL_RENDER_DEFAULT 0
#endif
bool pvr_render_partial_render = PVR_PARTIAL_RENDER_DEFAULT;

/* Ask the firmware to release the pair's two HWRTDatas before recycling the
 * pair, the DDK's sequence rather than an approximation of it. The freelists
 * are NOT part of it -- see the comment at the call site for why.
 *
 * Off by default: never measured. The case for it is that the DDK's
 * RGXDestroyHWRTDataSet posts CLEANUP/HWRTDATA and waits for the firmware's
 * reply BEFORE the memory is touched, because the firmware owns running state
 * in that struct (eState, the PM's continuation pointers, the freelist HWR
 * snapshot) and answers CLEANUP_BUSY while it still holds it. Our recycle asks
 * nothing and rewrites all of it. cts-runner measured every one of 15 recycle
 * bursts followed 20.02-20.12 s later by exactly one stall, 15 of 15.
 *
 * Prediction, on record before the A/B: at 0 the session reproduces (15 bursts,
 * 15 episodes in 585 s); at 1 the same recycle bursts occur, the firmware's
 * answers are logged at each, and there are no episodes. */
bool pvr_render_hwrt_cleanup;

/* Rebuild an HWRTData from zero, as the DDK's create does, instead of writing
 * over the previous size's struct in place. Off by default, never measured;
 * with hwrt_cleanup it gives the four arms of the recycle experiment. */
bool pvr_render_hwrt_reinit;

/* How often the reuse wait found the pair member still in flight, whether or
 * not it then refused. Counted unconditionally so the A/B can show the change
 * was actually exercised -- the lesson of 182a80f, whose zero counters were
 * the evidence its measurement could not mean what it was read to mean. */
uint32_t pvr_render_debug_hwrt_busy(void)
{
	return RR.hwrt_busy_reuse;
}

void pvr_render_debug_freelist(uint32_t slots_peak[3], uint32_t *slots_cap,
			       uint32_t ovf_peak[3], uint32_t ovf_now[3], uint32_t *bad)
{
	yd_pvr_pool_counters(&g_pool, NULL, slots_peak, slots_cap, ovf_peak, ovf_now, bad);
}

int pvr_render_static_get(struct pvr_render_static *out)
{
	if (!R.inited || !out) {
		return -EINVAL;
	}
	out->eot_doutu_src0 = R.eot_doutu;
	out->event_pixel_pds_info = pvr_cr_event_pixel_pds_info(1);
	for (unsigned n = 1; n <= PVR_RENDER_MAX_RT; n++) {
		out->eot_doutu_src0_n[n] = R.eot_doutu_n[n];
		/* The PDS DATA is always the largest count's (the program at
		 * the fixed offset moves all PVR_RENDER_MAX_RT pairs), and so
		 * are the shared registers it fills. */
		out->event_pixel_pds_info_n[n] = pvr_cr_event_pixel_pds_info(PVR_RENDER_MAX_RT);
		out->eot_doutu_src0_tb_n[n] = R.eot_doutu_tb_n[n];
		out->event_pixel_pds_info_tb_n[n] = out->event_pixel_pds_info_n[n];
	}
	for (unsigned i = 0; i < PVR_RENDER_TILE_BUFFERS; i++)
		out->tile_buffer_gpu[i] = R.tile_buf[i].gpu;
	out->border_gpu = R.border.gpu;
	out->heap_base[PVR_RENDER_HEAP_GENERAL] = PVR_GENERAL_HEAP_BASE;
	out->heap_base[PVR_RENDER_HEAP_PDS] = PVR_PDSCODEDATA_HEAP_BASE;
	out->heap_base[PVR_RENDER_HEAP_USC] = PVR_USCCODE_HEAP_BASE;
	return 0;
}

/* ---------------------------------------------------------------------- *
 * One-time objects
 * ---------------------------------------------------------------------- */

static void static_areas_init(void)
{
	/* USC heap offset 0: VS_NOP (the VDM_SYNC USC). usc_b then starts at 0x1000
	 * so no real shader can overlap the static region. */
	R.usc0 = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &R.usc_b, 4096);
	memcpy(R.usc0.cpu, pvr_usc_vs_nop_words, sizeof(pvr_usc_vs_nop_words));

	/* PDS heap offset 0: [VDM_SYNC kick program: data(8dw) + code(1dw)]; offset
	 * 0x80: the EOT (pixel-event) CODE the firmware runs at the fixed static
	 * location. The dummy coefficient program the PPP may point at (offset 0)
	 * also lands in this mapped page. */
	R.pds0 = pvr_gpu_bo_alloc(PVR_PDSCODEDATA_HEAP_BASE, &R.pds_b, 4096);
	{
		uint32_t vsd[8], vsc[1];
		struct pvr_pds_render_kick_usc_info vsi;

		pvr_pds_render_kick_usc_program(0 /* VS_NOP @ USC off 0 */, 0, 0, vsd, vsc, &vsi);
		memcpy(R.pds0.cpu, vsd, vsi.data_size_dw * 4);
		memcpy(R.pds0.cpu + vsi.data_size_dw * 4, vsc, vsi.code_size_dw * 4);
	}

	/* EOT USC program + its DOUTU control; the EOT PDS CODE is data-independent
	 * so it is generated once here (with placeholder emit words). */
	/* One end-of-tile program per colour-attachment count: program N emits N
	 * surfaces, reading each one's PBE state from shared registers 2i, 2i+1.
	 * The PDS CODE that fills those registers sits at a fixed heap offset the
	 * firmware runs for every render, so it is generated once for the largest
	 * count -- it then moves all six words, and a program that emits fewer
	 * simply does not read the rest. Which program runs is chosen per render
	 * through the DOUTU in the PDS DATA segment. */
	for (unsigned n = 1; n <= PVR_RENDER_MAX_RT; n++) {
		uint32_t bytes = pvr_usc_eot_n_dwords[n] * 4;

		R.eot_n[n] = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &R.usc_b, bytes);
		memcpy(R.eot_n[n].cpu, pvr_usc_eot_n_words[n], bytes);
		R.eot_doutu_n[n] = pvr_pds_pe_build_doutu_src0(R.eot_n[n].heap_off,
							       PVR_USC_EOT_TEMPS,
							       PVR_PDS_PE_SAMPLE_RATE_INSTANCE, 0);
		flush_bo(&R.eot_n[n], bytes);
	}
	/* Tile buffers and the end-of-tile programs that read them. A tile
	 * buffer holds one tile's worth of the outputs that did not fit the
	 * pixel output registers: tile_size_x * tile_size_y * max_partitions *
	 * usc_min_output_registers_per_pix dwords for this core's one cluster
	 * (Mesa pvr_get_tile_buffer_size_per_core). The program carries the
	 * buffer's address as two immediates, so each one is a patched copy. */
	for (unsigned i = 0; i < PVR_RENDER_TILE_BUFFERS; i++)
		R.tile_buf[i] = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &RR.gen_b,
						 PVR_TILE_BUFFER_BYTES);
	for (unsigned n = 1; n <= PVR_RENDER_MAX_RT; n++) {
		uint32_t bytes = pvr_usc_eot_tb_n_dwords[n] * 4;
		uint32_t *w;

		if (!bytes || !R.tile_buf[0].cpu)
			continue;
		R.eot_tb_n[n] = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &R.usc_b, bytes);
		memcpy(R.eot_tb_n[n].cpu, pvr_usc_eot_tb_n_words[n], bytes);
		w = (uint32_t *)R.eot_tb_n[n].cpu;
		for (uint32_t k = 0; k < bytes / 4; k++) {
			if (w[k] == PVR_USC_EOT_TB_ADDR_LO)
				w[k] = (uint32_t)(R.tile_buf[0].gpu & 0xFFFFFFFFu);
			else if (w[k] == PVR_USC_EOT_TB_ADDR_HI)
				w[k] = (uint32_t)(R.tile_buf[0].gpu >> 32);
		}
		R.eot_doutu_tb_n[n] = pvr_pds_pe_build_doutu_src0(R.eot_tb_n[n].heap_off,
								  PVR_USC_EOT_TB_TEMPS,
								  PVR_PDS_PE_SAMPLE_RATE_INSTANCE, 0);
		flush_bo(&R.eot_tb_n[n], bytes);
	}
	R.eot = R.eot_n[1];
	R.eot_doutu = R.eot_doutu_n[1];
	{
		uint32_t emit[2 * PVR_RENDER_MAX_RT] = { 0 };
		uint32_t d[3 * PVR_RENDER_MAX_RT + 2], c[PVR_RENDER_MAX_RT + 2];
		uint32_t dsz = 0, csz = 0;

		memset(d, 0, sizeof(d));
		memset(c, 0, sizeof(c));
		pvr_pds_generate_pixel_event_tq(R.eot_doutu, emit, PVR_RENDER_MAX_RT, 0,
						d, c, &dsz, &csz);
		memcpy(R.pds0.cpu + PVR_RENDER_EOT_PDS_CODE_OFF, c, csz * 4);
	}
	flush_bo(&R.usc0, 4096);
	flush_bo(&R.pds0, 4096);
	flush_bo(&R.eot, PVR_USC_EOT_SIZE_BYTES);
}

/* The run-time switches, for the Rust side. */
static void rr_sync_cfg(void)
{
	RR.cfg.combined_kick = pvr_render_combined_kick;
	RR.cfg.wait_rering = pvr_render_wait_rering;
	RR.cfg.wait_repost = pvr_render_wait_repost;
	RR.cfg.mmu_ack = pvr_render_mmu_ack;
	RR.cfg.mmu_gate = pvr_render_mmu_gate;
	RR.cfg.mmu_ack_probe = pvr_render_mmu_ack_probe;
	RR.cfg.mmu_ack_reping = pvr_render_mmu_ack_reping;
	RR.cfg.hwrt_refuse = pvr_render_hwrt_refuse;
	RR.cfg.hwrt_cleanup = pvr_render_hwrt_cleanup;
	RR.cfg.hwrt_reinit = pvr_render_hwrt_reinit;
	RR.cfg.fl_grow_on_demand = pvr_render_fl_grow_on_demand;
	RR.cfg.partial_render = pvr_render_partial_render;
#ifdef PVR_RENDER_VERBOSE
	RR.cfg.verbose = 1;
#else
	RR.cfg.verbose = 0;
#endif
}

static void rr_free_bo(void *ctx, const struct yd_pvr_bo *bo, uint32_t size)
{
	const struct pvr_gpu_bo b = { .gpu = bo->gpu, .cpu = bo->cpu, .heap_off = bo->heap_off };

	ARG_UNUSED(ctx);
	pvr_render_free(&b, size);
}

static const struct yd_pvr_render_ops rr_ops = {
	.ctx = NULL,
	.free_bo = rr_free_bo,
};

int pvr_render_init(void)
{
	if (R.inited) {
		return 0;
	}
	if (!pvr_drv_carveout_cpu()) {
		LOG_ERR("render: firmware not booted (no carveout)");
		return -ENODEV;
	}
	memset(&R, 0, sizeof(R));
	memset(&RR, 0, sizeof(RR));
	RR.ops = &rr_ops;
	RR.last_hs = -1;
	rr_sync_cfg();

	/* GENERAL heap: the parameter buffer MUST be the first object (VA
	 * 0x8000000000), then everything else sits above it. */
	R.pbuf = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &RR.gen_b, PB_RESERVE_BYTES);
	R.border = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &RR.gen_b, 4096);
	static_areas_init();

	/* Freelists in the DDK-faithful order (globals first, then local, MList
	 * last in the PM heap); IDs global=1/2, local=3. */
	/* The parameter buffer the TE/VCE fills while tiling. 448 current pages
	 * (1536 backed) is the DDK's sizing for a small test; a Qt Quick 3D scene
	 * tiles past it, the firmware asks to grow, and a render that cannot grow
	 * never completes (fence timeout, wedged GPU). Each global list starts
	 * small and grows out of one shared pool sized from the carveout, so a
	 * scene that fills a single list (the only one the firmware has ever asked
	 * to grow here) can spend the whole budget on it. */
	{
		const int big_pb = (PVR_GPU_MEM_SIZE >= (128UL * 1024 * 1024));
		const uint32_t pool = big_pb
			? (uint32_t)(((PVR_GPU_MEM_SIZE / 4) / 4096) & ~1023u)
			: 0u;
		const uint32_t cur = big_pb ? 2048u : 448u;
		const uint32_t grow = big_pb ? 1024u : 256u, ready = big_pb ? 0u : 64u;
#if defined(PVR_FL_PROBE_MAX_PAGES) && PVR_FL_PROBE_MAX_PAGES > 0
		/* A probe (CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_MAX_PAGES): the global
		 * lists cannot grow past this many pages, so a scene that needs a
		 * grow gets an unserved one, on purpose. */
		const uint32_t max = MAX((uint32_t)PVR_FL_PROBE_MAX_PAGES, cur + ready);

		LOG_WRN("render: PROBE: the global freelists are capped at %u pages: a grow past them is unserved on purpose",
			max);
#else
		const uint32_t max = big_pb ? cur + pool : 4096u;
#endif

		if (pool) {
			pvr_fl_set_reserve_mult(0);   /* all growth comes from the pool */
			pvr_fl_pool_init(&RR.gen_b, pool);
		} else if (pvr_render_fl_grow_on_demand) {
			/* one grow's pages kept per list, refilled from the render
			 * pool after each served grow (gpu::pvr::freelist::serve) */
			pvr_fl_set_reserve_mult(1);
			LOG_INF("render: freelist grows: one grow kept per list, refilled from the render pool after each grow");
		}
#if defined(PVR_FL_PROBE_NO_RESERVE)
		/* A probe (CONFIG_YAKOGL_DRV_GPU_PVR_FL_PROBE_NO_RESERVE): no
		 * reserve and no on-demand grow, so every grow is answered
		 * with zero pages, on purpose. */
		if (!pool && !pvr_render_fl_grow_on_demand) {
			pvr_fl_set_reserve_mult(0);
			LOG_WRN("render: PROBE: the global freelists have no grow reserve: every grow is answered with zero pages, on purpose");
		}
#endif
		LOG_INF("render: parameter buffer: carveout %u MiB -> %u pages listed, %u max, grow %u, pool %u pages",
			(unsigned)(PVR_GPU_MEM_SIZE / (1024 * 1024)), cur, max, grow, pool);
		yd_pvr_fl_set_ready_pages(pvr_km(), pvr_render_fl_ready_pages);
		if (pvr_render_fl_ready_pages)
			LOG_INF("render: freelist ready pages: the global lists keep the firmware's ready pages (gpu::pvr::freelist::ready_pages)");
		RR.gfl_off = pvr_build_freelist(&RR.gen_b, cur, max, grow, ready, 1);
		RR.gfl2_off = pvr_build_freelist(&RR.gen_b, cur, max, grow, ready, 2);
	}
	RR.fl_off = pvr_build_freelist(&RR.gen_b, 26, 32, 0, 0, 3);

	/* The render context, its client CCBs, and the HWR counter's start
	 * (gpu::pvr::render). */
	yd_pvr_render_ctx_init(pvr_km(), &RR);
	g_hwr_at_init = yd_pvr_render_hwr_counter(pvr_km());

	R.inited = true;
	RR.inited = 1;
	{
		const uint32_t *p0 = (const uint32_t *)R.pds0.cpu, *u0 = (const uint32_t *)R.usc0.cpu;

		LOG_INF("render: pds0: %08x %08x %08x %08x %08x %08x %08x %08x %08x | @0x80: %08x %08x  usc0: %08x %08x %08x %08x",
			p0[0], p0[1], p0[2], p0[3], p0[4], p0[5], p0[6], p0[7], p0[8], p0[32], p0[33],
			u0[0], u0[1], u0[2], u0[3]);
	}
	LOG_INF("render: init ok (PB@0x%llx border@0x%llx eot_usc_off=0x%x gen_b=0x%x pds_b=0x%x usc_b=0x%x)",
		(unsigned long long)R.pbuf.gpu, (unsigned long long)R.border.gpu,
		R.eot.heap_off, RR.gen_b, R.pds_b, R.usc_b);
	return 0;
}

/* ---------------------------------------------------------------------- *
 * Submit / wait: gpu::pvr::render (render.rs has the sequences and why).
 * ---------------------------------------------------------------------- */
BUILD_ASSERT(sizeof(struct pvr_render_kick) == 216, "gpu::pvr::render::PvrRenderKick");
BUILD_ASSERT(offsetof(struct pvr_render_kick, frag) == 32, "PvrRenderKick.frag");
BUILD_ASSERT(offsetof(struct pvr_render_kick, frag.pbe_word) == 80, "PvrRenderKick.frag.pbe_word");
BUILD_ASSERT(offsetof(struct pvr_render_kick, frag.num_rt) == 128, "PvrRenderKick.frag.num_rt");
BUILD_ASSERT(offsetof(struct pvr_render_kick, frag.pds_bgnd) == 136, "PvrRenderKick.frag.pds_bgnd");
BUILD_ASSERT(offsetof(struct pvr_render_kick, frag.isp_scissor_base) == 160,
	     "PvrRenderKick.frag.isp_scissor_base");
BUILD_ASSERT(offsetof(struct pvr_render_kick, frag.cmd_flags) == 200, "PvrRenderKick.frag.cmd_flags");
BUILD_ASSERT(offsetof(struct pvr_render_kick, frag.isp_oclqry_base) == 208, "PvrRenderKick.frag.isp_oclqry_base");

int pvr_render_submit(const struct pvr_render_kick *k, uint32_t *fence_out)
{
	int rc;

	if (!R.inited || !k || !fence_out) {
		return -EINVAL;
	}
	rr_sync_cfg();
	rc = yd_pvr_render_submit(pvr_km(), &RR, k, fence_out);
	pvr_render_reposts = RR.reposts;
	if (rc == -ESHUTDOWN) {
		render_stopped();
	}
	return rc;
}

uint32_t pvr_render_completed(void)
{
	return yd_pvr_render_completed(pvr_km(), &RR);
}

int pvr_render_last_kick_us(struct yd_pvr_kick_us *out)
{
	return yd_pvr_render_last_kick_us(&RR, out);
}

void pvr_render_scene_begin(void)
{
	yd_pvr_render_scene_begin(&RR);
}

void pvr_render_fl_counters(uint32_t out[YD_PVR_FL_COUNTERS])
{
	yd_pvr_render_fl_counters(pvr_km(), &RR, out);
}

/* The application waiting for its own frame: the OSAL's wait, and the
 * chardev's WAIT ioctl and close-time drain. The submit path waits inside
 * gpu::pvr::render with its own labels. */
int pvr_render_wait(uint32_t fence, uint32_t timeout_ms)
{
	int rc;

	rr_sync_cfg();
	rc = yd_pvr_render_wait(pvr_km(), &RR, fence, timeout_ms, "frame fence");
	pvr_render_reposts = RR.reposts;
	if (rc == -ESHUTDOWN) {
		render_stopped();
	}
	return rc;
}
