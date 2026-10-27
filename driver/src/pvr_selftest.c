/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_selftest.c -- the PowerVR driver's bring-up self-test render: a
 * fence-chained GEOM (TA) + 3D render of a full-screen rectangle into a 64x64
 * target, the on-silicon-verified structure the render service
 * (pvr_render.c) was lifted from, with the bring-up experiments it grew
 * (compile-time switches, register snapshots, the rig A/B of the Linux
 * harness). Run at boot only with CONFIG_YAKOGL_DRV_GPU_PVR_SELFTEST
 * (pvr_fw_boot.c); compiled always, so it keeps building.
 *
 * Ours, lab code: kept in C and not ported to Rust (docs/source-form.md step
 * 7, driver/docs/pvr-kernel-rust.md). It was part of pvr_fw_boot.c until
 * then and reaches the driver only through pvr_drv.h now.
 */

#include <zephyr/kernel.h>
#include <zephyr/kernel/internal/mm.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <zephyr/cache.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/logging/log.h>
#include <stdio.h>
#include <string.h>
#include <errno.h>

#include "pvr_ddk.h"
#include "pvr_internal.h"
#include "pvr_drv.h"
#include "pvr_usc_eot_blob.h"
#include "pvr_usc_vs_passthrough_blob.h"
#include "pvr_usc_clear_fs_blob.h"
#include "pvr_render_regs.h"
#include "pvr_pds_pixel_event_gen.h"
#include "pvr_pds_render_gen.h"
#include "pvr_vdm_ppp_gen.h"
#include "pvr_fw_trace_decode.h"

/* The PDS DOUTU temps field wraps rather than saturating (see
 * PVR_PDS_DOUTU_MAX_TEMPS). Every call this file makes to the PDS program
 * generators passes a compile-time constant from a hand-written blob, so the
 * generators' refusal is unreachable from here -- and these say so to the
 * compiler rather than leaving it as a property a reader has to re-derive by
 * looking up each constant. The one caller that passes a COMPILED shader's
 * temp count is YakoGL's vertex path in src/pvr/backend.rs, which checks the
 * return and fails the draw with -EINVAL. */
_Static_assert(PVR_USC_VS_PASSTHROUGH_TEMPS <= PVR_PDS_DOUTU_MAX_TEMPS,
	       "passthrough VS needs more temporaries than the DOUTU field can hold");
_Static_assert(PVR_USC_CLEAR_FS_TEMPS <= PVR_PDS_DOUTU_MAX_TEMPS,
	       "clear FS needs more temporaries than the DOUTU field can hold");
/* The bring-up paths that replayed data captured from the DDK's closed
 * user-space (ddk_closure.h: PVR_DDK_REPLAY and its PVR_REPLAY_* knobs,
 * PVR_FULL_PANEL, PVR_USE_DDK_VTX, PVR_USE_DDK_PPP, and the DDK vertex and
 * pixel shaders PVR_GRADIENT used without PVR_MULTIVARY) were removed: that
 * data was the output of proprietary software, which no licence lets us
 * redistribute (docs/source-form.md). */
#if defined(PVR_DDK_REPLAY) || defined(PVR_FULL_PANEL) || defined(PVR_USE_DDK_VTX) || \
	defined(PVR_USE_DDK_PPP) || defined(PVR_REPLAY_OUR_VDM)
#error "this DDK-capture bring-up path was removed (docs/source-form.md)"
#endif
#if defined(PVR_GRADIENT) && !defined(PVR_MULTIVARY)
#error "PVR_GRADIENT without PVR_MULTIVARY used the DDK's captured shaders, which were removed"
#endif

/* The TQ_3D fill test (PVR_ISP_CS) was removed with the mainline
 * firmware-interface header: it posted the mainline RGXFWIF_CMDTRANSFER and
 * transfer-context layouts, which the DDK kernel source does not define and
 * which the DDK firmware was never shown to accept. */
#ifdef PVR_ISP_CS
#error "PVR_ISP_CS (the TQ_3D fill test on the mainline transfer layout) was removed"
#endif

LOG_MODULE_DECLARE(gpu_pvr, CONFIG_YAKOGL_DRV_GPU_PVR_LOG_LEVEL);

/* What the rig used from pvr_fw_boot.c, through pvr_drv.h. */
static inline uint8_t *heap_cpu(uint32_t off)    { return pvr_heap_cpu(off); }
static inline uint32_t heap_fwaddr(uint32_t off) { return pvr_heap_fwaddr(off); }
#define g_cpu (pvr_drv_carveout_cpu())
#define gpu_bo_place pvr_gpu_bo_place
#define L0_FLAGS_DATA PVR_L0_FLAGS_DATA
#define L0_FLAGS_PM   PVR_L0_FLAGS_PM
#define L0_FLAGS_PM_CACHED PVR_L0_FLAGS_PM_CACHED
#define KCCB_NUM_CMDS   128u
#define FWCCB_NUM_CMDS  32u

/* PM/clock/power snapshot at a labelled timepoint (single read per reg + timestamp).
 * Used to trace the render/dust power-up handshake across the PM-op kick: boot-done,
 * pre-kick, post-trigger, mid-poll, pre-HWR. Compare CLK_STATUS transition + 0x270
 * vs the DDK working kick. NO polling loop here (1 snapshot/site, side-effect-safe). */
static void pm_snap(mm_reg_t regs, const char *tag)
{
	LOG_INF("PMSNAP[%s] CLK_STATUS(0x8)=0x%llx CLK_CTRL0(0x0)=0x%llx SIDEKICK_IDLE(0x3c8)=0x%x SLC_IDLE(0x3898)=0x%x SPU_ENABLE(0x50)=0x%x SOFT_RESET(0x100)=0x%llx SLC_CTRL_MISC(0x3800)=0x%x",
		tag,
		(unsigned long long)pvr_rd64(regs, 0x0008),
		(unsigned long long)pvr_rd64(regs, 0x0000),
		pvr_rd32(regs, 0x03C8), pvr_rd32(regs, 0x3898),
		pvr_rd32(regs, 0x0050),
		(unsigned long long)pvr_rd64(regs, 0x0100),
		pvr_rd32(regs, 0x3800));
	LOG_INF("PMSNAP[%s]   0x3d0=0x%x 0x1b8=0x%x 0x328=0x%x 0x270=0x%llx PM_CAT_VCE0=0x%llx TE0=0x%llx ALIST0=0x%llx",
		tag, pvr_rd32(regs, 0x03d0), pvr_rd32(regs, 0x01b8),
		pvr_rd32(regs, 0x0328),
		(unsigned long long)pvr_rd64(regs, 0x0270),
		(unsigned long long)pvr_rd64(regs, 0x1248),
		(unsigned long long)pvr_rd64(regs, 0x1250),
		(unsigned long long)pvr_rd64(regs, 0x1260));
}

static void __maybe_unused fwt_emit(void *ctx, const char *line) { (void)ctx; LOG_INF("FWT| %s", line); }

/* Upload a PDS segment into a PDS-heap BO; return its PDS-heap-relative offset. */
static uint32_t pds_put(struct pvr_gpu_bo *bo, uint32_t *bump, const uint32_t *w, uint32_t n)
{
	uint32_t off = ROUND_UP(*bump, 128); /* CODE/DATA addr alignment */

	memcpy(bo->cpu + off, w, n * 4);
	*bump = off + n * 4;
	return bo->heap_off + off;
}

int pvr_render_ta3d(mm_reg_t regs)
{
	const uint32_t W = 64, H = 64;
#ifdef PVR_NO_PB_BASE
	uint32_t gen_b = 0x1000, pds_b = 0x1000, usc_b = 0x1000; /* A/B: leave VA 0x8000000000 unmapped */
#else
	uint32_t gen_b = 0, pds_b = 0x1000, usc_b = 0x1000;
#endif

	/* Parameter-buffer region. The PM (parameter manager) writes tiled geometry
	 * to the GENERAL-heap-BASE virtual address (0x8000000000) through the main
	 * MMU, and the 3D reads it back from there. Reserve it as the FIRST GENERAL
	 * allocation (offset 0 -> VA 0x8000000000) so every other render allocation
	 * sits ABOVE it and can never collide with the PB. 2 MiB covers the global
	 * freelist's 448 pages (1.8 MiB) for a 64x64 render.
	 * (Root cause of the long-standing "TA never finalizes/PM alloc MMU fault":
	 * gen_b used to start at 0x1000, leaving VA 0x8000000000 unmapped, so the
	 * PM's very first parameter-buffer write faulted.) */
#ifdef PVR_SMALL_FENCE
	/* DIAG: only reserve/map the GENERAL-heap static area (FENCE 0..128 + YUV_CSC
	 * 128..1152) at VA 0x8000000000, NOT a 2 MiB PB. If the PM's dynamic PB
	 * allocation was being blocked by the oversized static pbuf, the geom should
	 * now allocate (ALIST/MLIST SP > 0) / complete. */
	struct pvr_gpu_bo pbuf = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 0x2000);

	memset(pbuf.cpu, 0x00, 0x2000);
#else
	struct pvr_gpu_bo pbuf = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 0x200000);

	memset(pbuf.cpu, 0x00, 0x200000);
#endif

#ifdef PVR_MAP_VA0
	/* DIAGNOSTIC: the 3D SPU faults at VA 0 (BIF0 Req Status low-40 = 0). Map a
	 * scratch at absolute VA 0 [0,1MiB) so that access lands; if the 3D advances,
	 * the fault WAS at VA 0 (some 3D unit using a null base). */
	{
		uint32_t tmp = gen_b;
		struct pvr_gpu_bo z = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &tmp, 0x100000);

		memset(z.cpu, 0x00, 0x100000);
		pvr_mmu_map(0x0ULL, pvr_bo_phys(&z), 0x100000, L0_FLAGS_DATA);
		gen_b = tmp;
		LOG_INF("VA0: mapped [0,1MiB) -> phys 0x%llx", (unsigned long long)pvr_bo_phys(&z));
	}
	/* DIAGNOSTIC: also map [0xE01B000000, +1 MiB) — the 3D's next BIF0 fault is
	 * at 0xE01B003000. Map it to scratch to see the following fault. */
	{
		uint32_t tmp = gen_b;
		struct pvr_gpu_bo z2 = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &tmp, 0x100000);

		memset(z2.cpu, 0x00, 0x100000);
		pvr_mmu_map(0xE01B000000ULL, pvr_bo_phys(&z2), 0x100000, L0_FLAGS_DATA);
		gen_b = tmp;
		LOG_INF("MAP2: mapped [0xE01B000000,+1MiB) -> phys 0x%llx",
			(unsigned long long)pvr_bo_phys(&z2));
	}
#endif

	/* ---- data surfaces (GENERAL heap, all ABOVE the PB region) ----
	 * dst BO is oversized (256 KiB) with a 0xA5 guard so we can measure the EXACT
	 * write extent/stride of a WxH render (overflow past W*H*4 shows up as 0xA5
	 * turning into the draw colour). */
	uint32_t dst_bo_sz = 0x40000;
	struct pvr_gpu_bo dst = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, dst_bo_sz);

	memset(dst.cpu, 0xA5, dst_bo_sz);

	uint32_t dst_pitch = W * 4;
#ifdef PVR_RENDER_TO_FB
	/* Render straight into the LVDS scan-out buffer (phys 0xE2900000, 1024x600
	 * XR24=XRGB8888, pitch 4096). CONFIRMED 2026-09-12: this IS the live scan-out
	 * buffer -- the triangle is visible on the physical panel (camera-photo). Note
	 * the panel is XRGB8888 (ARGB), so our AABBGGRR colour constants show with R/B
	 * swapped (bg 0xFFFF0000 -> red, tri 0xFF0000FF -> blue). The g_fbcpu readback
	 * below often reads 0 -- a CPU-cache coherency artifact (the GPU writes SLC-
	 * bypass and the DSS scans it out; the cached CPU view lags), NOT a write
	 * failure; trust the panel. dst.cpu stays the carveout for the default readback. */
	dst_pitch = 4096;
	/* Map the framebuffer at a FRESH high VA (well above all GENERAL-heap
	 * allocations) and point the emit/PBE at it via dst.gpu. Do NOT remap dst's
	 * own VA — a >2MB remap there would clobber the freelist/vtx/surface pages
	 * that sit just above dst and fault the PM (0x280040 / PM_META_RO). */
	dst.gpu = 0x8010000000ULL;
	/* Map the WHOLE 1024x600 framebuffer (not just H rows) so any PBE write
	 * within the panel lands (the render can touch the row just past H). */
	pvr_mmu_map(dst.gpu, 0xE2900000ULL, ROUND_UP(600u * 4096u, 0x1000),
		    L0_FLAGS_DATA);
	/* Also CPU-map the scan-out FB so we can read the PANEL content back (our own
	 * trusted readback stands in for the unavailable camera). Kept separate from
	 * dst.cpu (which stays the carveout) because the FB uses a 1024-px pitch. */
	uint8_t *g_fbcpu = NULL;

	/* Map UNCACHED so the readback sees the GPU's SLC-bypass writes in DRAM directly
	 * (a cached mapping returned stale 0 even for the known background). */
	k_mem_map_phys_bare(&g_fbcpu, 0xE2900000ULL,
			    ROUND_UP(600u * 4096u, 0x1000),
			    K_MEM_CACHE_NONE | K_MEM_PERM_RW);
	LOG_INF("RENDER_TO_FB: dst VA 0x%llx -> fb phys 0xE2900000 pitch %u (fbcpu=%p)",
		(unsigned long long)dst.gpu, dst_pitch, g_fbcpu);
#endif

#ifdef PVR_GEOM_DDK_VA_RANGE
	/* Subtractive/VA test: shift all render geometry into the DDK's VA ranges
	 * (leaving the ~424KB gap after the 2MB PB that the DDK leaves), in case our
	 * geometry packed right after the PB (0x200000) was overwritten by PM PB growth
	 * or hit a reserved range. general@0x26a000, PDS@0x54000, USC@0x92000. */
	gen_b = 0x400000u;   /* VA-range experiment knob (see notes); default off */
#endif
	struct pvr_gpu_bo vtx = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 256);
	/* u16 index buffer for the DDK-proven indexed TRI_LIST draw (sequential
	 * {0,1,2,...}). Non-indexed TRI_STRIP never tiled on this silicon. */
	struct pvr_gpu_bo idx = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 64);
	{
		uint16_t *ix = (uint16_t *)idx.cpu;

		for (uint16_t i = 0; i < 32; i++)
			ix[i] = i;
		sys_cache_data_flush_range(idx.cpu, 64);
	}
	float verts[8][3];

#ifdef PVR_DRAW_TRIANGLE
	/* One triangle (tri-strip of 3 verts = 1 tri) in screen space, filling most
	 * of the WxH render: apex top-centre, base along the bottom. */
	uint32_t index_count = 3;
#if defined(PVR_MULTI_PRIM) || defined(PVR_DEPTH) || defined(PVR_PRIM_LINES)
	/* Multiple primitives via indexed sequential idx {0..5}. PVR_MULTI_PRIM =
	 * full-coverage quad (2 tris); PVR_DEPTH = two overlapping tris at different z;
	 * PVR_PRIM_LINES = 3 lines (LINE_LIST) forming a triangle outline. */
	index_count = 6;
#endif

	/* Visible triangle in screen space (apex top-centre, base near bottom).
	 * z=0.5 (mid [0,1]) so the near/far clip can't reject it (z=0 sits on the
	 * near plane and may be clipped -> whole primitive culled -> TE tiles nothing). */
#if defined(PVR_MULTI_DRAW)
	/* Two separate DRAW CALLS (two INDEX_LIST blocks): draw1 = verts {0,1,2}
	 * (top-left triangle), draw2 = verts {3,4,5} (bottom-right triangle). Non-
	 * overlapping so both draws are individually visible. */
	verts[0][0] = 4.0f;  verts[0][1] = 4.0f;  verts[0][2] = 0.5f;   /* tri A */
	verts[1][0] = 4.0f;  verts[1][1] = 30.0f; verts[1][2] = 0.5f;
	verts[2][0] = 30.0f; verts[2][1] = 4.0f;  verts[2][2] = 0.5f;
	verts[3][0] = 60.0f; verts[3][1] = 60.0f; verts[3][2] = 0.5f;   /* tri B */
	verts[4][0] = 60.0f; verts[4][1] = 34.0f; verts[4][2] = 0.5f;
	verts[5][0] = 34.0f; verts[5][1] = 60.0f; verts[5][2] = 0.5f;
#elif defined(PVR_PRIM_LINES)
	/* Line primitives (LINE_LIST): 3 lines = a triangle outline. Pairs
	 * (v0,v1)(v2,v3)(v4,v5). */
	verts[0][0] = 8.0f;  verts[0][1] = 8.0f;  verts[0][2] = 0.5f;   /* top edge */
	verts[1][0] = 56.0f; verts[1][1] = 8.0f;  verts[1][2] = 0.5f;
	verts[2][0] = 56.0f; verts[2][1] = 8.0f;  verts[2][2] = 0.5f;   /* right edge */
	verts[3][0] = 32.0f; verts[3][1] = 56.0f; verts[3][2] = 0.5f;
	verts[4][0] = 32.0f; verts[4][1] = 56.0f; verts[4][2] = 0.5f;   /* left edge */
	verts[5][0] = 8.0f;  verts[5][1] = 8.0f;  verts[5][2] = 0.5f;
#elif defined(PVR_DEPTH)
	/* Depth-test A/B: two overlapping triangles. tri1 (near, z=0.3) points right
	 * from the left edge; tri2 (far, z=0.7) points left from the right edge; they
	 * overlap in the centre. tri2 (far) is drawn SECOND, so with the depth test the
	 * near (tri1) colour wins the overlap, but with paint-order (depth off) tri2
	 * would win. Colours: tri1 red, tri2 green (set in vcol). */
	verts[0][0] = 2.0f;    verts[0][1] = 2.0f;    verts[0][2] = 0.3f;
	verts[1][0] = 2.0f;    verts[1][1] = 61.0f;   verts[1][2] = 0.3f;
	verts[2][0] = 52.0f;   verts[2][1] = 32.0f;   verts[2][2] = 0.3f;
	verts[3][0] = 61.0f;   verts[3][1] = 2.0f;    verts[3][2] = 0.7f;
	verts[4][0] = 61.0f;   verts[4][1] = 61.0f;   verts[4][2] = 0.7f;
	verts[5][0] = 12.0f;   verts[5][1] = 32.0f;   verts[5][2] = 0.7f;
#elif defined(PVR_MULTI_PRIM)
	/* Full-coverage quad = 2 triangles over the WxH render. Corners
	 * A(top-left) B(top-right) C(bottom-right) D(bottom-left); TRI_LIST draws
	 * tri1={0,1,2}=A,B,C and tri2={3,4,5}=A,C,D. */
	{
		const float x0 = 1.0f, y0 = 1.0f, x1 = (float)W - 1.0f, y1 = (float)H - 1.0f;
		/* A */ verts[0][0] = x0; verts[0][1] = y0; verts[0][2] = 0.5f;
		/* B */ verts[1][0] = x1; verts[1][1] = y0; verts[1][2] = 0.5f;
		/* C */ verts[2][0] = x1; verts[2][1] = y1; verts[2][2] = 0.5f;
		/* A */ verts[3][0] = x0; verts[3][1] = y0; verts[3][2] = 0.5f;
		/* C */ verts[4][0] = x1; verts[4][1] = y1; verts[4][2] = 0.5f;
		/* D */ verts[5][0] = x0; verts[5][1] = y1; verts[5][2] = 0.5f;
	}
#elif defined(PVR_BIG_TRI)
	/* Huge triangle: if vtxin reaches the VS, this covers the whole 64x64 (the
	 * off-screen parts clip); if the VS reads vtxin=0 (data not landing), all
	 * three verts collapse to the origin -> degenerate -> zero coverage. */
	verts[0][0] = 0.0f;    verts[0][1] = 0.0f;    verts[0][2] = 0.5f;
	verts[1][0] = 0.0f;    verts[1][1] = 1000.0f; verts[1][2] = 0.5f;
	verts[2][0] = 1000.0f; verts[2][1] = 0.0f;    verts[2][2] = 0.5f;
#elif defined(PVR_TRI_ALT)
	/* Generality check: a different (small, top-left) triangle. If this renders as
	 * a distinct shape (not the centered one, not a fluke/stale fb), the generator
	 * is coordinate-driven, not hardcoded. */
	verts[0][0] = 10.0f; verts[0][1] = 10.0f; verts[0][2] = 0.5f;
	verts[1][0] = 10.0f; verts[1][1] = 34.0f; verts[1][2] = 0.5f;
	verts[2][0] = 40.0f; verts[2][1] = 10.0f; verts[2][2] = 0.5f;
#else
	verts[0][0] = 32.0f; verts[0][1] = 8.0f;  verts[0][2] = 0.5f;
	verts[1][0] = 8.0f;  verts[1][1] = 56.0f; verts[1][2] = 0.5f;
	verts[2][0] = 56.0f; verts[2][1] = 56.0f; verts[2][2] = 0.5f;
#endif
#else
	uint32_t index_count = 4;

	pvr_clear_vertices_fill(0, 0, W, H, 0.0f, verts);
#endif
#ifdef PVR_VTX_VEC3
	memcpy(vtx.cpu, verts, sizeof(verts));
#else
	/* The passthrough VS (proven identical to the DDK's by bisection) reads the
	 * DDK 8-float/vertex layout: position [x,y,z,w] + varying [r,g,b,a], stride 32,
	 * float screen-space, w=1.0. Position-only (vec3/vec4) delivery never tiled;
	 * the VS/UVSW contract needs the full 8-dword vertex (vs_output_size=32,
	 * vtxsize=8, unified_size=2). Colour = white. */
	{
		float *vc = (float *)vtx.cpu;

#ifdef PVR_GRADIENT
		/* RGB Gouraud gradient: v0=red, v1=green, v2=blue. The interpolating pixel
		 * shader (DDK FS @0x92560) reads the TE-interpolated varying and outputs it. */
#if defined(PVR_MULTI_DRAW)
		/* Two draw calls: draw1 (verts 0-2) red, draw2 (verts 3-5) green. */
		static const float vcol[8][4] = {
			{1.0f, 0.0f, 0.0f, 1.0f}, {1.0f, 0.0f, 0.0f, 1.0f}, {1.0f, 0.0f, 0.0f, 1.0f},
			{0.0f, 1.0f, 0.0f, 1.0f}, {0.0f, 1.0f, 0.0f, 1.0f}, {0.0f, 1.0f, 0.0f, 1.0f},
			{1.0f, 1.0f, 1.0f, 1.0f}, {1.0f, 1.0f, 1.0f, 1.0f},
		};
#elif defined(PVR_PRIM_LINES)
		/* Line colours: 3 bright edges (each line's 2 endpoints same colour). */
		static const float vcol[8][4] = {
			{1.0f, 0.0f, 0.0f, 1.0f}, {1.0f, 0.0f, 0.0f, 1.0f},   /* top: red   */
			{0.0f, 1.0f, 0.0f, 1.0f}, {0.0f, 1.0f, 0.0f, 1.0f},   /* right: green */
			{0.0f, 0.0f, 1.0f, 1.0f}, {0.0f, 0.0f, 1.0f, 1.0f},   /* left: blue  */
			{1.0f, 1.0f, 1.0f, 1.0f}, {1.0f, 1.0f, 1.0f, 1.0f},
		};
#elif defined(PVR_DEPTH)
		/* Depth-test colours: tri1 (near) red, tri2 (far) green. Flat per triangle
		 * (all 3 verts same colour) so the overlap winner is unambiguous. */
		static const float vcol[8][4] = {
			{1.0f, 0.0f, 0.0f, 1.0f},   /* tri1 near, red  */
			{1.0f, 0.0f, 0.0f, 1.0f},
			{1.0f, 0.0f, 0.0f, 1.0f},
			{0.0f, 1.0f, 0.0f, 1.0f},   /* tri2 far, green */
			{0.0f, 1.0f, 0.0f, 1.0f},
			{0.0f, 1.0f, 0.0f, 1.0f},
			{1.0f, 1.0f, 1.0f, 1.0f},
			{1.0f, 1.0f, 1.0f, 1.0f},
		};
#elif defined(PVR_TEXTURE)
		/* UV coordinates as the vec4 varying (.xy = u,v; .zw unused). Quad corners
		 * A(0,0) B(1,0) C(1,1) D(0,1); layout matches the 6-vertex TRI_LIST A,B,C,A,C,D.
		 * The SMP fragment shader samples the texture at the interpolated (u,v).
		 * (Before the SMP FS is wired, the interp FS shows u->R, v->G as a sanity map.) */
		static const float vcol[8][4] = {
			{0.0f, 0.0f, 0.0f, 1.0f},   /* A (0,0) */
			{1.0f, 0.0f, 0.0f, 1.0f},   /* B (1,0) */
			{1.0f, 1.0f, 0.0f, 1.0f},   /* C (1,1) */
			{0.0f, 0.0f, 0.0f, 1.0f},   /* A (0,0) */
			{1.0f, 1.0f, 0.0f, 1.0f},   /* C (1,1) */
			{0.0f, 1.0f, 0.0f, 1.0f},   /* D (0,1) */
			{0.0f, 0.0f, 0.0f, 1.0f},
			{0.0f, 0.0f, 0.0f, 1.0f},
		};
#elif defined(PVR_MULTI_PRIM)
		/* Quad corners A,B,C,D coloured distinctly so each of the two triangles is
		 * visible and interpolation across both is verifiable. Layout matches the
		 * 6-vertex TRI_LIST above: A,B,C,A,C,D. */
#ifdef PVR_GRAD_DIM
		/* Camera-friendly: dim (0.22) + panel R/B pre-swap (panel is XRGB, our R->
		 * panel B). Corners show intuitively: A red / B green / C blue / D white. */
		static const float vcol[8][4] = {
			{0.0f,  0.0f,  0.22f, 1.0f}, /* A -> panel RED    (top-left)     */
			{0.0f,  0.22f, 0.0f,  1.0f}, /* B -> panel GREEN  (top-right)    */
			{0.22f, 0.0f,  0.0f,  1.0f}, /* C -> panel BLUE   (bottom-right) */
			{0.0f,  0.0f,  0.22f, 1.0f}, /* A                                */
			{0.22f, 0.0f,  0.0f,  1.0f}, /* C                                */
			{0.22f, 0.22f, 0.22f, 1.0f}, /* D -> panel WHITE  (bottom-left)  */
			{0.22f, 0.22f, 0.22f, 1.0f},
			{0.22f, 0.22f, 0.22f, 1.0f},
		};
#else
		static const float vcol[8][4] = {
			{1.0f, 0.0f, 0.0f, 1.0f},   /* A red    */
			{0.0f, 1.0f, 0.0f, 1.0f},   /* B green  */
			{0.0f, 0.0f, 1.0f, 1.0f},   /* C blue   */
			{1.0f, 0.0f, 0.0f, 1.0f},   /* A red    */
			{0.0f, 0.0f, 1.0f, 1.0f},   /* C blue   */
			{1.0f, 1.0f, 1.0f, 1.0f},   /* D white  */
			{1.0f, 1.0f, 1.0f, 1.0f},
			{1.0f, 1.0f, 1.0f, 1.0f},
		};
#endif
#elif defined(PVR_GRAD_TEST_ALLRED)
		/* DIAGNOSTIC: all three vertices red. If the whole triangle is uniform red,
		 * the interpolator is fine and only per-vertex varying STORAGE remains. */
		static const float vcol[4][4] = {
			{1.0f, 0.0f, 0.0f, 1.0f},
			{1.0f, 0.0f, 0.0f, 1.0f},
			{1.0f, 0.0f, 0.0f, 1.0f},
			{1.0f, 1.0f, 1.0f, 1.0f},
		};
#elif defined(PVR_GRAD_TEST_V0BLACK)
		/* DIAGNOSTIC: v0 black, v1/v2 coloured. If any colour shows, v1/v2 varyings
		 * DO reach the coefficient engine; if all black, only provoking-vertex v0. */
		static const float vcol[4][4] = {
			{0.0f, 0.0f, 0.0f, 1.0f},   /* v0 black */
			{0.0f, 1.0f, 0.0f, 1.0f},   /* v1 green */
			{0.0f, 0.0f, 1.0f, 1.0f},   /* v2 blue */
			{1.0f, 1.0f, 1.0f, 1.0f},
		};
#else
#ifdef PVR_GRAD_DIM
		/* Half-intensity RGB so the (very bright) panel doesn't blow out the camera —
		 * shows the full red->green->blue gradient in colour rather than white. */
		/* Panel is XRGB (our r->panel B, b->panel R), so swap R/B here to show the
		 * intuitive red-apex / green-left / blue-right on the physical display. */
		static const float vcol[4][4] = {
			{0.0f, 0.0f, 0.22f, 1.0f},   /* v0 -> panel RED apex */
			{0.0f, 0.22f, 0.0f, 1.0f},   /* v1 -> panel GREEN bottom-left */
			{0.22f, 0.0f, 0.0f, 1.0f},   /* v2 -> panel BLUE bottom-right */
			{0.22f, 0.22f, 0.22f, 1.0f},
		};
#else
		static const float vcol[4][4] = {
			{1.0f, 0.0f, 0.0f, 1.0f},   /* v0 red */
			{0.0f, 1.0f, 0.0f, 1.0f},   /* v1 green */
			{0.0f, 0.0f, 1.0f, 1.0f},   /* v2 blue */
			{1.0f, 1.0f, 1.0f, 1.0f},
		};
#endif
#endif
#endif
		uint32_t nverts = 4;
#if defined(PVR_MULTI_PRIM) || defined(PVR_DEPTH) || defined(PVR_PRIM_LINES)
		nverts = index_count;   /* pack all 6 vertices */
#elif defined(PVR_MULTI_DRAW)
		nverts = 6;             /* two 3-vertex draws */
#endif
#if defined(PVR_MULTIVARY)
		/* 10 floats/vertex: pos4 + color4 (VAR0) + uv2 (VAR1). color = RGB gradient,
		 * uv = (1,1) so out = v_color * vec4(uv,1,1) = v_color (the RGB Gouraud). If
		 * VAR1 were mis-delivered (0) the R,G channels would zero out -> blue only,
		 * so a clean RGB gradient proves both varyings are delivered & interpolated. */
		for (uint32_t vi = 0; vi < 3; vi++) {
			vc[vi * 10 + 0] = verts[vi][0];
			vc[vi * 10 + 1] = verts[vi][1];
			vc[vi * 10 + 2] = verts[vi][2];
			vc[vi * 10 + 3] = 1.0f;          /* w */
			vc[vi * 10 + 4] = vcol[vi][0];   /* VAR0.r */
			vc[vi * 10 + 5] = vcol[vi][1];   /* VAR0.g */
			vc[vi * 10 + 6] = vcol[vi][2];   /* VAR0.b */
			vc[vi * 10 + 7] = vcol[vi][3];   /* VAR0.a */
			vc[vi * 10 + 8] = 1.0f;          /* VAR1.u */
			vc[vi * 10 + 9] = 1.0f;          /* VAR1.v */
		}
#else
		for (uint32_t vi = 0; vi < nverts; vi++) {
			vc[vi * 8 + 0] = verts[vi][0];
			vc[vi * 8 + 1] = verts[vi][1];
			vc[vi * 8 + 2] = verts[vi][2];
			vc[vi * 8 + 3] = 1.0f;          /* w */
#ifdef PVR_GRADIENT
			vc[vi * 8 + 4] = vcol[vi][0];   /* r */
			vc[vi * 8 + 5] = vcol[vi][1];   /* g */
			vc[vi * 8 + 6] = vcol[vi][2];   /* b */
			vc[vi * 8 + 7] = vcol[vi][3];   /* a */
#else
			vc[vi * 8 + 4] = 1.0f;          /* r */
			vc[vi * 8 + 5] = 1.0f;          /* g */
			vc[vi * 8 + 6] = 1.0f;          /* b */
			vc[vi * 8 + 7] = 1.0f;          /* a */
#endif
		}
#endif /* PVR_MULTIVARY vertex packing */
	}
#endif
	LOG_INF("VTX pre-render vtx.gpu=0x%llx cpu[0..5]=%08x %08x %08x %08x %08x %08x",
		(unsigned long long)vtx.gpu,
		((uint32_t *)vtx.cpu)[0], ((uint32_t *)vtx.cpu)[1], ((uint32_t *)vtx.cpu)[2],
		((uint32_t *)vtx.cpu)[3], ((uint32_t *)vtx.cpu)[4], ((uint32_t *)vtx.cpu)[5]);

	struct pvr_gpu_bo color = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 6 * 4);

	((uint32_t *)color.cpu)[0] = 0xFF0000FFu; /* triangle: red (AABBGGRR) */

	/* Background object colour (distinct from the triangle) so the drawn triangle
	 * is visible against it. AABBGGRR: 0xFFFF0000 = blue. */
	struct pvr_gpu_bo bgcolor = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 6 * 4);

#ifdef PVR_DARK_BG
	((uint32_t *)bgcolor.cpu)[0] = 0xFF000000u; /* background: black (camera-friendly) */
#else
	((uint32_t *)bgcolor.cpu)[0] = 0xFFFF0000u; /* background: blue */
#endif

	struct pvr_gpu_bo border = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 4096);
	struct pvr_gpu_bo tpc = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 0x4000);
#ifdef PVR_TEXTURE
	/* Texture sampling (toward Slint: present a SW-rendered buffer as a GPU
	 * texture). Layouts from the Mesa PVR ROGUE csbgen (verified). A 64x64 linear
	 * RGBA8888 source (memory order R,G,B,A ascending = LE 0xAABBGGRR), sampled on
	 * the full-coverage quad (build with PVR_MULTI_PRIM; UV = the vec4 varying .xy).
	 * A recognisable 4-quadrant + 8px checker pattern so the sampled output is
	 * unmistakable vs a Gouraud gradient. */
	/* Texture dimensions: default 64x64 (1:1 with the quad, nearest-friendly).
	 * PVR_TEX_SMALL uses a small texture magnified across the quad so LINEAR
	 * filtering (PVR_TEX_LINEAR) is visible (smooth blend) vs NEAREST (hard blocks). */
#ifdef PVR_TEX_SMALL
	const uint32_t TEX_W = 8, TEX_H = 8;
#else
	const uint32_t TEX_W = 64, TEX_H = 64;
#endif
	struct pvr_gpu_bo tex = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, TEX_W * TEX_H * 4);
	{
		uint32_t *tp = (uint32_t *)tex.cpu;
		const uint32_t half_x = TEX_W / 2, half_y = TEX_H / 2;
		const uint32_t cksh = (TEX_W >= 64) ? 3 : 0;   /* 8px checker at 64, 1px at 8 */

		for (uint32_t ty = 0; ty < TEX_H; ty++)
			for (uint32_t tx = 0; tx < TEX_W; tx++) {
				int chk = ((tx >> cksh) ^ (ty >> cksh)) & 1;
				uint32_t c;

				if (tx < half_x && ty < half_y)
					c = chk ? 0xFF0000FFu : 0xFF000060u; /* R quadrant */
				else if (tx >= half_x && ty < half_y)
					c = chk ? 0xFF00FF00u : 0xFF006000u; /* G quadrant */
				else if (tx < half_x && ty >= half_y)
					c = chk ? 0xFFFF0000u : 0xFF600000u; /* B quadrant */
				else
					c = chk ? 0xFFFFFFFFu : 0xFF303030u; /* white/grey */
				tp[ty * TEX_W + tx] = c;
			}
		sys_cache_data_flush_range(tex.cpu, TEX_W * TEX_H * 4);
	}
	/* Image (4 dw) + sampler (4 dw) descriptor, contiguous; DMA'd into the USC
	 * common store by the PDS texture-state program below. */
	struct pvr_gpu_bo texdesc = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 32);
	{
		uint32_t *dd = (uint32_t *)texdesc.cpu;
		uint64_t base = tex.gpu;
		/* IMAGE_WORD0: STRIDE(4) textype + U8U8U8U8(12) fmt + identity swizzle
		 * (fixed = 0x60000A64), width-1 @bit34, height-1 @bit48. */
		{
			uint64_t iw0 = 0x60000A64ULL |
				       ((uint64_t)(TEX_W - 1) << 34) |
				       ((uint64_t)(TEX_H - 1) << 48);
			dd[0] = (uint32_t)(iw0 & 0xFFFFFFFFu);
			dd[1] = (uint32_t)(iw0 >> 32);
		}
		/* STRIDE_IMAGE_WORD1: num_mip=1 (bit60), alpha_msb (bit59),
		 * texaddr[16:53]=base>>2, stride[0:14]=texels-1. */
		{
			uint64_t w1 = 0x1800000000000000ULL | (uint64_t)(TEX_W - 1) |
				      (((base >> 2) & 0x3FFFFFFFFFULL) << 16);
			dd[2] = (uint32_t)(w1 & 0xFFFFFFFFu);
			dd[3] = (uint32_t)(w1 >> 32);
		}
		/* SAMPLER_WORD0 = 0x02002400_00000FFF (nearest, clamp-to-edge, bias 0),
		 * SAMPLER_WORD1 = 0. For linear: OR 0x00000050 into dd[5]. */
		dd[4] = 0x00000FFFu;
#ifdef PVR_TEX_LINEAR
		dd[5] = 0x02002450u;
#else
		dd[5] = 0x02002400u;
#endif
		dd[6] = 0u;
		dd[7] = 0u;
		sys_cache_data_flush_range(texdesc.cpu, 32);
	}
	LOG_INF("TEX: tex.gpu=0x%llx desc.gpu=0x%llx IMG=[%08x %08x %08x %08x] SMP=[%08x %08x]",
		(unsigned long long)tex.gpu, (unsigned long long)texdesc.gpu,
		((uint32_t *)texdesc.cpu)[0], ((uint32_t *)texdesc.cpu)[1],
		((uint32_t *)texdesc.cpu)[2], ((uint32_t *)texdesc.cpu)[3],
		((uint32_t *)texdesc.cpu)[4], ((uint32_t *)texdesc.cpu)[5]);
#endif
#ifdef PVR_UNIFORM
	/* Uniform / constant (GL-ES push-constant). The F1 uniform-colour FS reads a
	 * vec4 u_color from shared regs sh0..3; a PDS DOUTD DMAs the 4 float dwords into
	 * the USC common store dwords 0..3. Whole primitive = u_color (flat). */
	struct pvr_gpu_bo ubo = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 16);
	{
		float *u = (float *)ubo.cpu;
		/* magenta (AABBGGRR readback 0xffff00ff): R=1,G=0,B=1,A=1 */
		u[0] = 1.0f; u[1] = 0.0f; u[2] = 1.0f; u[3] = 1.0f;
		sys_cache_data_flush_range(ubo.cpu, 16);
	}
	LOG_INF("UNIFORM: ubo.gpu=0x%llx u_color=[%08x %08x %08x %08x]",
		(unsigned long long)ubo.gpu, ((uint32_t *)ubo.cpu)[0], ((uint32_t *)ubo.cpu)[1],
		((uint32_t *)ubo.cpu)[2], ((uint32_t *)ubo.cpu)[3]);
#endif
#ifdef PVR_MAP_FAULT_VA
	/* DIAGNOSTIC: map a scratch region at the FIXED dm=3 fault VA (0x4c4000000)
	 * so the MCU_PDS_USCA coefficient read hits mapped memory instead of faulting.
	 * This VA is HW-computed (implicit coefficient-store base), not one of our
	 * explicit pointers, and is bit-stable across runs. If the fault CLEARS or
	 * MOVES, it confirms the pixel shader reads its coefficients from this base;
	 * the clear FS ignores the loaded values, so the triangle may rasterize. If a
	 * NEW fault VA appears, it reveals the next stage; if unchanged, the address is
	 * not actually dereferenced there. Map 1 MiB to cover any read span. */
	{
		struct pvr_gpu_bo fva = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 0x100000);
		pvr_mmu_map((uint64_t)(PVR_MAP_FAULT_VA), pvr_bo_phys(&fva), 0x100000,
			    L0_FLAGS_DATA);
		LOG_INF("PVR_MAP_FAULT_VA: mapped scratch 0x%llx..+0x100000 -> phys 0x%llx",
			(unsigned long long)(uint64_t)(PVR_MAP_FAULT_VA),
			(unsigned long long)pvr_bo_phys(&fva));
	}
#endif
#ifdef PVR_VHEAP_SIZE
	struct pvr_gpu_bo vheap = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, PVR_VHEAP_SIZE);
#else
	struct pvr_gpu_bo vheap = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 1536);
#endif
	/* MList is allocated AFTER the freelists (below) so it lands last in the PM heap,
	 * matching the DDK PM-heap order (globals, local, then MList@0xfa..c000). */
	/* allocated below, after the freelists (the BUFADDR line shows 0 for it;
	 * it printed an uninitialised value before step 7) */
	struct pvr_gpu_bo mlist = { 0 };
	struct pvr_gpu_bo rgnhdr = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 0x10000);
	/* Macrotile array: mainline (pvr_hwrt.c) always programs a non-NULL
	 * macrotile_array_dev_addr into the HWRTData; the PM references it during
	 * the first-kick tiling setup, so a NULL here stalls DM2 dispatch. */
	struct pvr_gpu_bo macrotile = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 4096);

	(void)border;
	LOG_INF("BUFADDR dst=0x%llx vtx=0x%llx color=0x%llx bgcolor=0x%llx border=0x%llx",
		(unsigned long long)dst.gpu, (unsigned long long)vtx.gpu,
		(unsigned long long)color.gpu, (unsigned long long)bgcolor.gpu,
		(unsigned long long)border.gpu);
	LOG_INF("BUFADDR tpc=0x%llx vheap=0x%llx mlist=0x%llx rgnhdr=0x%llx macrotile=0x%llx",
		(unsigned long long)tpc.gpu, (unsigned long long)vheap.gpu,
		(unsigned long long)mlist.gpu, (unsigned long long)rgnhdr.gpu,
		(unsigned long long)macrotile.gpu);

	/* ---- parameter-buffer free lists: DISTINCT local + global (see
	 * build_freelist).  Built later (after other GENERAL-heap allocs) below. ---- */

	/* Static VDM_SYNC USC program (mainline static_data_areas): VS_NOP at USC
	 * heap offset 0. The VDM kicks it (via the VDM_SYNC PDS at PDS offset 0) to
	 * synchronise; without it the geom (DM2) faults at USC 0 / locks up. Bytes =
	 * pco_usclib vs_nop_common (temps=0) = one `nop.end` group. usc_b starts at
	 * 0x1000, so this static [0,0x1000) region never overlaps the real USC
	 * shaders. (An earlier 4-dword form with two leading dwords 0x00000000
	 * 0x00000200 had no END in its first group: the USC ran on through the
	 * zero page into the VS at 0x1000, whose END hid the bug. It surfaced when
	 * another program sat at 0x1000; see pvr_usc_vs_nop_words.) */
	uint32_t usc0_b = 0;
	struct pvr_gpu_bo usc0 = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &usc0_b, 4096);

	memcpy(usc0.cpu, pvr_usc_vs_nop_words, sizeof(pvr_usc_vs_nop_words));

	/* ---- USC shaders (USCCODE heap) ---- */
#ifdef PVR_VS_AT_92000
	/* Isolation: place ONLY the VS at USC 0x92000 (the VA where our VS is proven to
	 * tile in the replay), leaving FS/EOT at the default usc_b, to test whether the
	 * VS's USC VA specifically gates native tiling. */
	struct pvr_gpu_bo vs = gpu_bo_place(PVR_USCCODE_HEAP_BASE, 0x92000u,
					PVR_USC_VS_PASSTHROUGH_SIZE_BYTES, L0_FLAGS_DATA);
#else
	struct pvr_gpu_bo vs = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &usc_b, 128);
#endif

#ifdef PVR_GRADIENT
#if defined(PVR_MULTIVARY)
	/* V1 passthrough VS (pco_shc, temps=0): a_pos@vi0..3, a_color@vi4..7,
	 * a_uv@vi8..9 -> emits POS@0, v_color(VAR0)@4, v_uv(VAR1)@8; vs_output_size=10. */
	static const uint32_t V1_passthrough_usc[10] = {
		0x0806a055, 0x00048404, 0xa0553000, 0x88080802, 0x30000004,
		0x0806a056, 0x00048000, 0xfff13000, 0x0580a044, 0xff000000,
	};
	memcpy(vs.cpu, V1_passthrough_usc, sizeof(V1_passthrough_usc));
#endif
#else
	memcpy(vs.cpu, pvr_usc_vs_passthrough, PVR_USC_VS_PASSTHROUGH_SIZE_BYTES);
#endif
#ifdef PVR_VS_USE_NOP
	/* DIAGNOSTIC: replace the passthrough VS with VS_NOP (no UVSW output). If the
	 * geom behaves identically (VCE=0, same DM2), the passthrough's output was
	 * never reaching the PSG anyway; if it differs, the passthrough IS executing
	 * and writing output. */
	memcpy(vs.cpu, pvr_usc_vs_nop_words, sizeof(pvr_usc_vs_nop_words));
#endif

#ifdef PVR_GRADIENT
#ifdef PVR_MULTIVARY
	/* F4 multi-varying FS (pco_shc, temps=10): reads v_color(VAR0 vec4)@cf4..19 and
	 * v_uv(VAR1 vec2)@cf20..27 (w plane@cf0..3), outputs v_color*vec4(v_uv,1,1)->po0. */
	static const uint32_t F4_multivary_usc[52] = {
		0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000,
		0x00080061, 0x80040000, 0x00050478, 0xc0570040, 0xf2b0f300, 0x00408040,
		0xff000000, 0x00608104, 0x00000008, 0x00618104, 0x00000008, 0xff6a8002,
		0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000,
		0x00080061, 0x80040000, 0x00150878, 0xc0570020, 0xf2b0f300, 0x00408040,
		0xff000000, 0x00608104, 0x00000008, 0x00618104, 0x00000008, 0xff6a8002,
		0x40008235, 0x0000a8c4, 0x8235ff40, 0xa9c54000, 0xff410000, 0x87008234,
		0x42000046, 0x87008234, 0x43000047, 0x9c06925a, 0xa0408020, 0x2c001040,
		0xfff3ff44, 0xffffffff, 0x87808a34, 0x20000044,
	};
	struct pvr_gpu_bo fs = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &usc_b, sizeof(F4_multivary_usc));

	memcpy(fs.cpu, F4_multivary_usc, sizeof(F4_multivary_usc));
	uint32_t fs_temps = 10;
#elif defined(PVR_UNIFORM)
	/* F1 uniform-colour FS (pco_shc, lean, temps=5): reads vec4 u_color from
	 * sh0..3 (DOUTD common store) and writes it to po0. No varyings read. */
	static const uint32_t F1_uniform_usc[16] = {
		0x87008235, 0x00000880, 0x82354000, 0x08818700, 0x41000000, 0x87008235,
		0x00000882, 0x82354200, 0x08838700, 0x43000000, 0x9c069258, 0xa0408020,
		0x2c001040, 0xfff1ff44, 0x87808a34, 0x20000044,
	};
	struct pvr_gpu_bo fs = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &usc_b, sizeof(F1_uniform_usc));

	memcpy(fs.cpu, F1_uniform_usc, sizeof(F1_uniform_usc));
	uint32_t fs_temps = 5;
#elif defined(PVR_TEXTURE)
	/* Texture-sample FS (machine-generated by the production Mesa PCO backend for
	 * BVNC 36.53.104.796; smp.2d.fcnorm.replace.pplod). Reads the iterated VAR0.xy
	 * (UV) from cf0..7 via the same DOUTI coeff path, samples tex_state=sh0..3 /
	 * smp_state=sh4..7 (the common-store words our PDS DOUTD loads), writes RGBA ->
	 * po0. temps = 32. */
	static const uint32_t smp_tex_fs[56] = {
		0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000,
		0x00080061, 0x80040000, 0x00001078, 0xc0570020, 0xf2b0f300, 0x00408040,
		0xff000000, 0x00608104, 0x00000008, 0x00618104, 0x00000008, 0xff6a8002,
		0x87008234, 0x52000000, 0x87008234, 0x53000000, 0x87008234, 0x54000000,
		0x87008234, 0x55000000, 0x87008234, 0x56000000, 0x87008234, 0x57000000,
		0x87008234, 0x58000000, 0x87008234, 0x59000000, 0x87008234, 0x5a000000,
		0x87008234, 0x5b000000, 0x87008234, 0x5c000000, 0x87008234, 0x5d000000,
		0x87008234, 0x5e000000, 0x87008234, 0x5f000000, 0xf400a067, 0x7080805e,
		0x80880480, 0x800200a0, 0x9257ff6a, 0x80209c06, 0x1040a040, 0xff442c00,
		0x87808a34, 0x20000044,
	};
	struct pvr_gpu_bo fs = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &usc_b, sizeof(smp_tex_fs));

	memcpy(fs.cpu, smp_tex_fs, sizeof(smp_tex_fs));
	uint32_t fs_temps = 32;
#endif
#else
	struct pvr_gpu_bo fs = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &usc_b, PVR_USC_CLEAR_FS_SIZE_BYTES);

	memcpy(fs.cpu, pvr_usc_clear_fs_bytes, PVR_USC_CLEAR_FS_SIZE_BYTES);
	uint32_t fs_temps = PVR_USC_CLEAR_FS_TEMPS;
#endif

	struct pvr_gpu_bo eot = pvr_gpu_bo_alloc(PVR_USCCODE_HEAP_BASE, &usc_b, PVR_USC_EOT_SIZE_BYTES);

	memcpy(eot.cpu, pvr_usc_eot_bytes, PVR_USC_EOT_SIZE_BYTES);

	LOG_INF("USCOFF: vs=0x%x fs=0x%x eot=0x%x (usc_exec_base=0xE000000000; fault was 0xE01B003000)",
		vs.heap_off, fs.heap_off, eot.heap_off);

	/* Reserve a backed (zero) page at PDS heap offset 0.  The static-clear PPP
	 * uses a "dummy coefficient loading program" with TA_STATE_PDS_VARYINGBASE=0
	 * and pds_varyingsize=0 (matching Mesa pvr_arch_cmd_buffer.c:3665,3685); the
	 * HW nonetheless issues a read at PDS_exec_base+0 (it "always assumes the
	 * data size is 2x128-bit words"), so 0xDA00000000 must be mapped.  pds_b
	 * starts at 0x1000, so this dummy [0,0x1000) never overlaps the real PDS. */
	uint32_t pds0_b = 0;

	struct pvr_gpu_bo pds0 = pvr_gpu_bo_alloc(PVR_PDSCODEDATA_HEAP_BASE, &pds0_b, 4096);

	/* ---- PDS programs (PDSCODEDATA heap) ---- */
	struct pvr_gpu_bo pds = pvr_gpu_bo_alloc(PVR_PDSCODEDATA_HEAP_BASE, &pds_b, 8192);

	memset(pds.cpu, 0, 8192); /* zero the whole PDS BO: unwritten constants must
				   * not be garbage (the EOT DOUTU reads its src0 from
				   * here; garbage bits corrupt the USC exec addr). */
	uint32_t pb = 0;

	/* vertex PDS: reads the 4-vertex clear rect into vtxin reg 0. */
	uint32_t vdata[8], vcode[3];
	struct pvr_pds_render_vertex_info vinfo;

	uint32_t vs_off_dbg = vs.heap_off;
#ifdef PVR_VS_FAULT_TEST
	/* DIAGNOSTIC: point the VS USC exec offset at an unmapped USC addr
	 * (0x8000000 -> 0xE008000000). If the vertex PDS DOUTU launches the VS, the
	 * USC fetch faults there; silent hang = VS never launched. */
	vs_off_dbg = 0x08000000u;
#endif
	uint64_t vtx_addr_dbg = vtx.gpu;
#ifdef PVR_VTX_FAULT_TEST
	/* DIAGNOSTIC: point the vertex DMA source at an unmapped addr. If the PDS
	 * DDMAD runs the DMA, it faults there; silent hang = DMA never runs (PDS
	 * halts before the DMA / DOUTU). */
	vtx_addr_dbg = 0xE800000000ULL;
#endif
#ifdef PVR_VTX_VEC3
	pvr_pds_render_clear_vertex_program(vs_off_dbg, PVR_USC_VS_PASSTHROUGH_TEMPS,
					    vtx_addr_dbg, 12, 12, 0, 0, vdata, vcode, &vinfo);
#elif defined(PVR_MULTIVARY)
	/* V1 VS: 10 floats/vertex (pos4+color4+uv2) = 40-byte contiguous DMA -> vi0..9;
	 * V1 temps=0. */
	pvr_pds_render_clear_vertex_program(vs_off_dbg, 0,
					    vtx_addr_dbg, 40, 40, 0, 0, vdata, vcode, &vinfo);
#else
	/* DDK 8-float vertex (pos vec4 + varying vec4): stride + DMA size = 32 bytes. */
	pvr_pds_render_clear_vertex_program(vs_off_dbg, PVR_USC_VS_PASSTHROUGH_TEMPS,
					    vtx_addr_dbg, 32, 32, 0, 0, vdata, vcode, &vinfo);
#endif
	uint32_t v_pds_data = pds_put(&pds, &pb, vdata, vinfo.data_size_dw);
	uint32_t v_pds_code = pds_put(&pds, &pb, vcode, vinfo.code_size_dw);

	/* fragment kick-USC PDS. */
	uint32_t kdata[8], kcode[1];
	struct pvr_pds_render_kick_usc_info kinfo;

	pvr_pds_render_kick_usc_program(fs.heap_off, fs_temps, 0, kdata, kcode, &kinfo);
	/* Pixel-shader kick program layout (pvr_pds_kick_usc / Mesa): the HW loads a
	 * FIXED-size (8-dword) data segment starting at shader_addr, then executes
	 * the code IMMEDIATELY after it. So DATA and CODE must be CONTIGUOUS and
	 * BGRND0.shader_addr must point at the DATA (block start), not the code.
	 * (pds_put would 128-align the code into a separate slot -> the HW would run
	 * garbage and fault; that was the 0xDF4000020 = PDS+0x54000020 fault.) */
	uint32_t k_pds_data = pds_put(&pds, &pb, kdata, kinfo.data_size_dw);
	uint32_t k_pds_code = pds.heap_off + pb;   /* contiguous, right after data */

	memcpy(pds.cpu + pb, kcode, kinfo.code_size_dw * 4);
	pb += kinfo.code_size_dw * 4;
	(void)k_pds_code;

	/* colour shared-reg upload PDS (DMAs the fill color into sh0). */
	uint32_t cdata[3], ccode[2];
	struct pvr_pds_render_color_upload_info cinfo;

	pvr_pds_render_color_upload_program(color.gpu, 6, 0, 0, cdata, ccode, &cinfo);
	uint32_t c_pds_data = pds_put(&pds, &pb, cdata, cinfo.data_size_dw);
	uint32_t c_pds_code = pds_put(&pds, &pb, ccode, cinfo.code_size_dw);

	/* Background-object colour-upload PDS (separate colour BO) so the background
	 * shades a different colour from the drawn triangle. */
	uint32_t bg_cdata[3], bg_ccode[2];
	struct pvr_pds_render_color_upload_info bg_cinfo;

	pvr_pds_render_color_upload_program(bgcolor.gpu, 6, 0, 0, bg_cdata, bg_ccode,
					    &bg_cinfo);
	uint32_t bg_c_pds_data = pds_put(&pds, &pb, bg_cdata, bg_cinfo.data_size_dw);
	uint32_t bg_c_pds_code = pds_put(&pds, &pb, bg_ccode, bg_cinfo.code_size_dw);

	/* Coefficient/varying-loading PDS program for the drawn primitive's declared
	 * varying (PPP TA_STATE_VARYING0=0x400 => the TE stores + the ISP iterates one
	 * 4-dword varying per covered primitive). Faithful replay of the DDK fragment
	 * closure program @0x540d0 (ddk_fragpds dwords 52..56): DATA=[ctrl,LAST,0,0]
	 * then a DOUTC (DOUT dst=4 = coefficient bank, END) at +0x10. The DOUTC uses
	 * the HW-supplied coefficient-buffer base (the ISP-computed plane equations),
	 * NOT an explicit address, so it cannot fault on a bad pointer. Bound as
	 * TA_STATE_PDS_VARYINGBASE. Previously VARYINGBASE=0 -> the coeff engine ran
	 * the VDM_SYNC program at PDS offset 0 and dereferenced garbage ->
	 * MCU_PDS_USCA fault (dm=3, cat_base=1, VA 0x4c4000000). */
	/* DOUTI (DOUT dst=4) coefficient-iterate program. DATA[0] = iterate control:
	 * size=4D. Shade model: FLAT_VERTEX0 (0x000c0000, whole prim = vertex0 colour)
	 * or GOURAUD+perspective (0x030d0000, smooth interpolation across the 3 verts).
	 * DATA[1]=0x80000000 LAST. CODE = DOUTI (0xf4000004). */
#if defined(PVR_GRADIENT) && !defined(PVR_GRAD_FLAT)
#ifndef PVR_DOUTI_DESC
#define PVR_DOUTI_DESC 0x030c0000u   /* DOUTI GOURAUD 4D non-perspective (npc) */
#endif
#else
#ifndef PVR_DOUTI_DESC
#define PVR_DOUTI_DESC 0x000c0000u   /* DOUTI FLAT_VERTEX0 4D */
#endif
#endif
#if defined(PVR_MULTIVARY)
	/* Two smooth varyings + W plane (V1+F4). 3 DOUTI iterators: W->cf0..3,
	 * VAR0(vec4)->cf4..19, VAR1(vec2)->cf20..27. DATA 8dw (6 + 2 pad) + CODE 3dw. */
	static const uint32_t coeff_prog[11] = {
		0x03000000u, 0x00000000u,   /* iter0 W    -> cf0..3   (GOURUAD 1D, persp=0)      */
		0x030D0202u, 0x00400000u,   /* iter1 VAR0 -> cf4..19  (GOURUAD 4D, persp=1, off=2) */
		0x03050A0Au, 0x81400000u,   /* iter2 VAR1 -> cf20..27 (GOURUAD 2D, persp=1, off=10, LAST) */
		0x00000000u, 0x00000000u,   /* pad to 128-bit */
		0xF0000004u, 0xF0000104u, 0xF4000204u,   /* CODE: DOUTI src0=0/1/2, END on last */
	};
	uint32_t coeff_pds_off = pds_put(&pds, &pb, coeff_prog, 11);
#else
	static const uint32_t coeff_prog[5] = {
		PVR_DOUTI_DESC, 0x80000000u, 0x00000000u, 0x00000000u, /* DATA */
		0xf4000004u,                                           /* CODE: DOUTI END @ +0x10 */
	};
	uint32_t coeff_pds_off = pds_put(&pds, &pb, coeff_prog, 5);
#endif

#ifdef PVR_TEXTURE
	/* PDS texture-state program: one DOUTD DMAs the 8-dword image+sampler
	 * descriptor (texdesc) into the USC common store (dest common[0], size 8 dw,
	 * LAST), then HALT. DATA layout: const[0..1]=64-bit src addr, const[2]=control;
	 * DOUTD src0=addr64>>1=0, src1=control32=2 -> 0xF4020000. */
	uint32_t texpds_code[2] = { 0xF4020000u, 0xD3000000u };
	uint32_t texpds_data[3] = {
		(uint32_t)(texdesc.gpu & 0xFFFFFFFFu),
		(uint32_t)(texdesc.gpu >> 32),
		0x90000008u, /* BSIZE=8 | DEST_COMMON_STORE(bit28) | LAST_EN(bit31), AO=0 */
	};
	uint32_t tex_pds_data = pds_put(&pds, &pb, texpds_data, 3);
	uint32_t tex_pds_code = pds_put(&pds, &pb, texpds_code, 2);

	LOG_INF("TEXPDS: data_off=0x%x code_off=0x%x data=[%08x %08x %08x]",
		tex_pds_data, tex_pds_code, texpds_data[0], texpds_data[1], texpds_data[2]);
#endif

#ifdef PVR_UNIFORM
	/* PDS DOUTD loading the 4-dword uniform into common store dwords 0..3
	 * (size=4, DEST_COMMON_STORE, LAST). Same encoding as the texture DOUTD. */
	uint32_t unipds_code[2] = { 0xF4020000u, 0xD3000000u };
	uint32_t unipds_data[3] = {
		(uint32_t)(ubo.gpu & 0xFFFFFFFFu),
		(uint32_t)(ubo.gpu >> 32),
		0x90000004u, /* BSIZE=4 | DEST_COMMON_STORE | LAST_EN, AO=0 */
	};
	uint32_t uni_pds_data = pds_put(&pds, &pb, unipds_data, 3);
	uint32_t uni_pds_code = pds_put(&pds, &pb, unipds_code, 2);

	LOG_INF("UNIPDS: data_off=0x%x code_off=0x%x data=[%08x %08x %08x]",
		uni_pds_data, uni_pds_code, unipds_data[0], unipds_data[1], unipds_data[2]);
#endif

	/* pixel-event (EOT) PDS. */
	uint32_t emit_words[2];
	uint64_t da = dst.gpu;

	emit_words[0] = (uint32_t)((da >> 2) & 0xFFFFFFFF);
	emit_words[1] = (uint32_t)(((da >> 34) & 0x3F) | 0x82000u);
	uint64_t eot_doutu = pvr_pds_pe_build_doutu_src0(eot.heap_off, PVR_USC_EOT_TEMPS,
							PVR_PDS_PE_SAMPLE_RATE_INSTANCE, 0);
	uint32_t epds_data[8], epds_code[4], epds_dsz, epds_csz;

	memset(epds_data, 0, sizeof(epds_data));
	memset(epds_code, 0, sizeof(epds_code));
	pvr_pds_generate_pixel_event_tq(eot_doutu, emit_words, 1, 0,
					epds_data, epds_code, &epds_dsz, &epds_csz);
	uint32_t e_pds_data = pds_put(&pds, &pb, epds_data, epds_dsz);
	/* The 3D-render pixel-event (EOT) PDS CODE is NOT passed in the frag command
	 * (the 3D registers have event_pixel_pds_data + info but NO _code). The FW
	 * expects it at the fixed STATIC-DATA-AREA_EOT location: PDS_CODE_DATA heap
	 * offset 128 (0x80) (mainline pvr_vm.c static_data_areas). Placing our EOT
	 * code anywhere else -> FW runs the (zero) bytes at 0xDA00000080 as the EOT
	 * program -> the emit state never reaches the eot USC -> it writes the clear
	 * to a garbage base (was the 0x9003000 fault). So put the code at 0x80; the
	 * per-render DATA still comes from event_pixel_pds_data (= e_pds_data). */
#define PVR_EOT_STATIC_PDS_OFF 0x80u
	memcpy(pds0.cpu + PVR_EOT_STATIC_PDS_OFF, epds_code, epds_csz * 4);
	uint32_t e_pds_code = pds0.heap_off + PVR_EOT_STATIC_PDS_OFF;

	/* Static VDM_SYNC PDS at PDS heap offset 0 (mainline static_data_areas): a
	 * kick_usc program that DOUTUs the VS_NOP USC at USC offset 0. [data][code]
	 * contiguous, data first (same layout as the pixel shader). Fits in [0,0x80)
	 * so it never overlaps the EOT code at 0x80. Without it the VDM's sync kick
	 * runs garbage at PDS/USC 0 -> geom fault (0xE000000000) / DM2 lockup. */
	{
		uint32_t vsd[8], vsc[1];
		struct pvr_pds_render_kick_usc_info vsi;

		pvr_pds_render_kick_usc_program(0 /* VS_NOP @ USC off 0 */, 0, 0,
						vsd, vsc, &vsi);
		memcpy(pds0.cpu, vsd, vsi.data_size_dw * 4);
		memcpy(pds0.cpu + vsi.data_size_dw * 4, vsc, vsi.code_size_dw * 4);
	}

	/* ---- PPP state block + VDM control stream (TRANSFER_FRAG not needed) ---- */
	uint32_t ppp_words[64], ppp_count;
	struct pvr_ppp_frag_inputs ppi = {
		/* TA_STATE_PDS_SHADERBASE points at the pixel-shader kick program's DATA
		 * start (the HW reads the fixed 8-dword data segment there, then executes
		 * the code immediately after). Our kick_usc is [data][code] contiguous, so
		 * this is k_pds_data (code auto-follows at +0x20). The working background
		 * object (pds_bgnd[0]=k_pds_data) and the DDK PPP (SHADERBASE=0x540a0=DATA,
		 * code@+0x20) both confirm SHADERBASE=DATA. Passing k_pds_code made the HW
		 * read the code dword as the data segment -> garbage DOUTU/deref. */
#ifdef PVR_SHADERBASE_CODE
		.pixel_program_code_offset = k_pds_code,
#else
		.pixel_program_code_offset = k_pds_data,
#endif
#if defined(PVR_UNIFORM)
		/* Uniform DOUTD program: 4 dwords (u_color) into common store 0..3.
		 * usc_sharedsize = DRU(4,16)=1. The emitted varying stays active (needed for
		 * the TE to tile) but the uniform FS ignores it. */
		.texture_program_code_offset = uni_pds_code,
		.texture_program_data_offset = uni_pds_data,
		.texture_program_data_size_bytes = 3 * 4,   /* 3 DATA dwords -> DRU(3,4) = 1 unit */
		.texture_program_temps = 1,
		.shared_reg_count = 4,
#elif defined(PVR_TEXTURE)
		/* Texture-state PDS program: TEXUNICODEBASE=CODE, TEXTUREDATABASE=DATA.
		 * pds_texturestatesize = DRU(3,4)=1 (3 DATA dwords), pds_tempsize = DRU(1,4)
		 * =1, usc_sharedsize = DRU(8,16)=1 (8 dwords DMA'd into the common store).
		 * The coeff/varying path (usc_varyingsize) is separate and still active for
		 * the UV varying. */
		.texture_program_code_offset = tex_pds_code,
		.texture_program_data_offset = tex_pds_data,
		.texture_program_data_size_bytes = 3 * 4,   /* 3 DATA dwords -> DRU(3,4) = 1 unit */
		.texture_program_temps = 1,
		.shared_reg_count = 8,                   /* image(4)+sampler(4) common dwords */
#elif defined(PVR_GRADIENT)
		.texture_program_code_offset = c_pds_code,
		.texture_program_data_offset = c_pds_data,
		/* The interpolating FS reads coefficients, not shared regs, so it needs NO
		 * texture/uniform (color-upload) program. Zero the texture/shared sizes so
		 * SIZEINFO1 = only the varying bits (0x410) and SIZEINFO2 = 0, matching the
		 * DDK interp path exactly (avoid polluting the coeff layout). */
		.texture_program_data_size_bytes = 0,
		.texture_program_temps = 0,
		.shared_reg_count = 0,
#else
		.texture_program_code_offset = c_pds_code,
		.texture_program_data_offset = c_pds_data,
		.texture_program_data_size_bytes = cinfo.data_size_dw * 4,
		.texture_program_temps = cinfo.temps_used,
		.shared_reg_count = 6,
#endif
		.isp_userpass = 0,
		/* Mesa's clear template sets ISPA passtype = TRANSLUCENT(1)
		 * (pvr_clear.c:116). We had OPAQUE. */
		.passtype = PVR_PPP_PASSTYPE_TRANSLUCENT,
		.varying_program_code_offset = coeff_pds_off,
	};

	{
		struct pvr_vdm_field_refusal why = { 0 };

		if (pvr_ppp_build_clear_block(&ppi, ppp_words, &ppp_count, &why) < 0) {
			if (why.unit_bytes)
				LOG_ERR("rig: PPP block refused: %s = %llu units of %u bytes (max %u)",
					why.field, (unsigned long long)why.value, why.unit_bytes, why.max);
			else /* a plain value: enum, index, tile, low address bits */
				LOG_ERR("rig: PPP block refused: %s = %llu (max %u)", why.field,
					(unsigned long long)why.value, why.max);
			return -EINVAL;
		}
	}
	/* VARYINGBASE (PPP PDS word [4] = ppp_words[8]) must be the coeff program, not 0. */
	LOG_INF("FRAGPDS: k_pds_data=0x%x k_pds_code=0x%x c_pds_data=0x%x c_pds_code=0x%x coeff_prog=0x%x  VARYINGBASE(ppp[8])=0x%08x",
		k_pds_data, k_pds_code, c_pds_data, c_pds_code, coeff_pds_off, ppp_words[8]);
	LOG_INF("PPPWORDS: SHADERBASE[4]=0x%08x TEXUNI[5]=0x%08x SIZEINFO1[6]=0x%08x SIZEINFO2[7]=0x%08x VARYINGBASE[8]=0x%08x TEXDATA[9]=0x%08x OUTSEL[13]=0x%08x VARYING0[14]=0x%08x  coeff_prog[0]=0x%08x fs_temps=%u",
		ppp_words[4], ppp_words[5], ppp_words[6], ppp_words[7], ppp_words[8], ppp_words[9], ppp_words[13], ppp_words[14], coeff_prog[0], fs_temps);
	{
		volatile uint32_t *vb = (volatile uint32_t *)vtx.cpu;

		LOG_INF("VTXBUF: v0.col=%08x %08x %08x %08x | v1.col=%08x %08x %08x %08x | v2.col=%08x %08x %08x %08x  idxcount=%u",
			vb[4], vb[5], vb[6], vb[7], vb[12], vb[13], vb[14], vb[15],
			vb[20], vb[21], vb[22], vb[23], index_count);
	}
	/* The DDK draw needs a 2nd PPP_STATE_UPDATE(3 words) after the index list that
	 * references PPP words 19-21 (the trailing state). Our clear-template PPP emits
	 * only 19 words, so those 3 words were absent -> the 2nd block read garbage ->
	 * TA never tiled. Append the DDK trailing state (DDK PPP words 19..20, +0) at words
	 * 19-21; the main PPP block stays 19 words (vi.ppp_word_count), the 2nd block
	 * (PVR_VDM_2ND_PPP) points at ppp+0x4c=word 19. [DECISIVE fix 2026-09-12.] */
	uint32_t ppp_alloc = ppp_count;
#ifndef PVR_NO_PPP_TAIL
	if (ppp_count == 19) {
		/* The DDK capture's words 0x02000000 0x007c0012 0x00000000 are
		 * the terminate block of the 1024 x 600 target the DDK test
		 * rendered (TERMINATE0 clip_right 31, clip_bottom 18 in 32-px
		 * blocks), which pvr_ppp_build_terminate_block() packs from
		 * those two numbers (csbgen TA_STATE_HEADER/TERMINATE0/1), as
		 * the render service does for its own target. */
		(void)pvr_ppp_build_terminate_block(1024, 600, &ppp_words[19], NULL);
		ppp_alloc = 22;
	}
#endif
	struct pvr_gpu_bo ppp = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, ppp_alloc * 4);

	memcpy(ppp.cpu, ppp_words, ppp_alloc * 4);

	uint32_t vdm_words[32], vdm_count;
	struct pvr_vdm_inputs vi = {
		.ppp_state_dev_addr = ppp.gpu,
		.ppp_word_count = ppp_count,
		.vs_pds_data_offset = v_pds_data,
		.vs_pds_code_offset = v_pds_code,
		.vs_pds_data_size_bytes = vinfo.data_size_dw * 4,
		.vs_pds_temps = vinfo.temps_used,
#if defined(PVR_MULTIVARY)
		.vs_output_size_bytes = 40,   /* V1: 10-dword VS output (pos4+color4+uv2) */
#else
		.vs_output_size_bytes = 32,   /* 8-dword VS output (DDK contract) */
#endif
		.index_count = index_count,
#ifndef PVR_VDM_NONINDEXED
		.index_buf_dev_addr = idx.gpu,   /* DDK-proven indexed TRI_LIST(6)/u16 */
#endif
#ifdef PVR_MULTI_DRAW
		/* Second draw call: verts {3,4,5} via a 2nd INDEX_LIST reading idx[3..5]
		 * (base = idx + 3 u16). draw1 above uses index_count=3 (verts {0,1,2}). */
		.index_count2 = 3,
		.index_buf2_dev_addr = idx.gpu + 3u * 2u,
#endif
	};

	{
		struct pvr_vdm_field_refusal why = { 0 };

		if (!pvr_vdm_build_clear_stream(&vi, vdm_words, &vdm_count, &why)) {
			if (why.unit_bytes)
				LOG_ERR("rig: VDM stream refused: %s = %llu units of %u bytes (max %u)",
					why.field, (unsigned long long)why.value, why.unit_bytes, why.max);
			else /* a plain value: enum, index, tile, low address bits */
				LOG_ERR("rig: VDM stream refused: %s = %llu (max %u)", why.field,
					(unsigned long long)why.value, why.max);
			return -EINVAL;
		}
	}
#ifdef PVR_VDM_TERMINATE_ONLY
	/* DIAGNOSTIC: minimal VDM stream = just STREAM_TERMINATE. If the TA then
	 * finishes instantly (empty render), the VDM processes the stream fine and
	 * the geometry blocks are at fault; if it still hangs, the VDM/context
	 * setup itself is broken. (STREAM_TERMINATE = block_type 6 << 29.) */
	vdm_words[0] = 0xc0000000u;
	vdm_count = 1;
#endif
#ifdef PVR_VDM_NODRAW
	/* DIAGNOSTIC: valid empty render = keep PPP_STATE + VDM_STATE (words 0..6),
	 * drop the INDEX_LIST draw (words 7,8), keep STREAM_TERMINATE. If the TA now
	 * FINISHES, the draw/vertex path is the wall; if it still times out, TA
	 * finalization (TE/PM) is broken independent of any draw. */
	vdm_words[7] = 0xc0000000u;   /* STREAM_TERMINATE right after VDM_STATE5 */
	vdm_count = 8;
#endif
#ifdef PVR_VDM_DRAW_FAULT_TEST
	/* DIAGNOSTIC: point VDM_STATE2 (vs_pds_data base, vdm[3]) at an unmapped
	 * PDS offset (0x8000000 -> 0xDA08000000). If the VDM reaches the INDEX_LIST
	 * draw and fires the vertex PDS, it faults there; silent hang = VDM never
	 * reaches the draw. */
	vdm_words[3] = 0x08000000u;
#endif
	struct pvr_gpu_bo vdm = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, vdm_count * 4);

	memcpy(vdm.cpu, vdm_words, vdm_count * 4);

	/* ---- freelists (FW heap structs), DDK-faithful ORDER + IDs + sizes ----
	 * DDK ground truth (DDK_LIVE_DUMP / DDK_FL_HWRT_GROUNDTRUTH): the PM-heap layout is
	 * global1(id 0xd)@0xfa00000000, global2(id 0xe)@0xfa00004000, local(id 0xf)@..8000,
	 * MList@..c000 -- i.e. the two GLOBAL freelists are created FIRST (lowest PM-heap
	 * addresses), then local, then MList. Our earlier code created local first, so the
	 * HWRTData LOCAL slot pointed at 0xfa00000000 where the DDK has a GLOBAL -- a
	 * semantic reversal (top PM_CAT=0 suspect). Match the DDK: build global1, global2,
	 * local in that order (globals get the low addresses); MList is allocated AFTER
	 * (below). IDs use the DDK relation global=1/2, local=3. global grow=256/ready=64
	 * (Init=512) per the DDK; local grow=0/ready=0, 26/32. */
	/* The DDK global freelist is ALWAYS max=4096, current=448, grow=256, ready=64 on
	 * this SoC — those are the DDK-faithful values, not an optional tuning, so tie them
	 * to the DDK firmware build (not the legacy PVR_BIG_GLOBAL_FL) so a normal DDK build cannot
	 * silently fall back to max=448/grow=0/ready=0. */
	/* The parameter buffer the TE/VCE fills while tiling. 448 current pages
	 * (1536 backed) is the DDK's own sizing for a small test; a Qt Quick 3D
	 * scene tiles past it and the firmware asks to grow. With a carveout of
	 * 128 MiB or more (the Qt firmware reserves 256) each global list gets
	 * 16 MiB listed plus 8 MiB of reserve for four grows. */
	const int big_pb = (PVR_GPU_MEM_SIZE >= (128UL * 1024 * 1024));
	uint32_t gcur = big_pb ? 4096u : 448u, gmax = big_pb ? 8192u : 4096u;
	const uint32_t ggrow = big_pb ? 512u : 256u, gready = big_pb ? 0u : 64u;

	/* pvr_fl_init_pages overrides the listed pages of each global freelist,
	 * so the "does the grow matter?" arm of the shape-2 experiment can be run
	 * from one binary instead of a -DPVR_BIG_GLOBAL_FL rebuild.
	 *
	 * This is not free: the pages come out of the same render pool the
	 * GENERAL, PDS and USC heaps bump from, so a larger list leaves less for
	 * textures. A run that raises it must record GL_OUT_OF_MEMORY as well as
	 * stalls, or it trades one failure for the other and reports a win. */
	if (pvr_fl_init_pages) {
		const uint32_t cap = (uint32_t)((PVR_GPU_MEM_SIZE / 4096u) / 8u);

		gcur = pvr_fl_init_pages > cap ? cap : pvr_fl_init_pages;
		if (gmax < gcur) {
			gmax = gcur;
		}
		LOG_WRN("freelist sizing: OVERRIDDEN by fl_init_pages=%u -> cur %u (cap %u, one eighth of the carveout)",
			pvr_fl_init_pages, gcur, cap);
	}
	LOG_INF("freelist sizing: carveout %u MiB -> %s (cur %u max %u grow %u ready %u)",
		(unsigned)(PVR_GPU_MEM_SIZE / (1024 * 1024)),
		pvr_fl_init_pages ? "override" : (big_pb ? "big" : "small"),
		gcur, gmax, ggrow, gready);
	uint32_t gfl_off  = pvr_build_freelist(&gen_b, gcur, gmax, ggrow, gready, 1); /* global1 (id 1) */
	uint32_t gfl2_off = pvr_build_freelist(&gen_b, gcur, gmax, ggrow, gready, 2); /* global2 (id 2) */
	uint32_t fl_off = pvr_build_freelist(&gen_b, 26, 32, 0, 0, 3);         /* local (id 3) */
	/* MList allocated LAST in the PM heap (after the freelist PFN arrays), matching the
	 * DDK PM-heap order. PM-managed (records allocated MMU pages; DDK MLIST0 SP 0->9).
	 * L0_FLAGS_PM = bit62 (PM_META_PROTECT) + bit3 (SLC_BYPASS), matching the DDK
	 * GPU_UNCACHED MList/array PMRs. */
	mlist = pvr_gpu_bo_alloc_pm(PVR_PMMETA_PROTECT_HEAP_BASE, pvr_pm_heap_bump(), 4096);

#ifdef PVR_FL_FAULT_TEST
	/* DIAGNOSTIC: corrupt the local freelist current_dev_addr to unmapped. If
	 * the PM faults at 0xE800000000, it IS reading the freelist to allocate
	 * (stall is downstream); silent hang = PM never reads the freelist. */
	((RGXFWIF_FREELIST *)heap_cpu(fl_off))->ui64CurrentDevVAddr = 0xE800000000ULL;
#endif


	/* ---- HWRTData common + one hwrtdata (FW heap) ---- */
	uint32_t hcmn_off = pvr_heap_alloc(sizeof(RGXFWIF_HWRTDATA_COMMON), 8);
	RGXFWIF_HWRTDATA_COMMON *hc = (void *)heap_cpu(hcmn_off);

	hc->ui32ScreenPixelMax = 0x003F003Fu;
	/* Device tile size is 16x16 (bxs-4-64.h: tile_size_x/y=16), so 64x64 = 4x4
	 * tiles. Per mainline pvr_hwrt.c: tile_max = round_up(num_tiles,2)-1 = 3.
	 * te_screen = (xmax<<0)|(ymax<<12) = 3|(3<<12) = 0x3003. (The earlier 0x1001
	 * assumed 32x32 tiles; it is INCONSISTENT with the mtile config below, which
	 * is already computed for 4x4 tiles - mtile_x[0]=mtile_y[0]=2 - and that
	 * mismatch corrupts the TE/PM tiling address -> PM-alloc MMU fault.) */
	hc->ui32TEScreen = 0x00003003u;
	hc->ui32TEMTILE1 = 0x00080000u;
	hc->ui32TEMTILE2 = 0x00080000u;
	hc->ui32ISPMtileSize = 0x00020002u;
	hc->ui32MTileStride = 4;
	hc->ui32TEAA = 0;
	hc->ui64MultiSampleCtl = 0x0000000000000088ull;
	hc->ui64FlippedMultiSampleCtl = 0x0000000000000088ull;
	hc->ui32TPCStride = 0;
	hc->ui32TPCSize = 128;
	hc->uiRgnHeaderSize = 16;
	hc->bTACachesNeedZeroing = false; /* match Linux (pvr_hwrt.c) */

	uint32_t hrt_off = pvr_heap_alloc(sizeof(RGXFWIF_HWRTDATA), 8);
	RGXFWIF_HWRTDATA *hrt = (void *)heap_cpu(hrt_off);

	hrt->sHWRTDataCommonFwAddr.ui32Addr = heap_fwaddr(hcmn_off);
	hrt->eState = RGXFWIF_RTDATA_STATE_NONE;
	/* HWRTData slot <-> freelist mapping, DDK-faithful (AGP config, SUPPORT_AGP on):
	 * LOCAL slot -> local FL (id 3), GLOBAL slot -> global1 (id 1), GLOBAL2 slot ->
	 * global2 (id 2). Earlier the disasm-agent lead (slot2=0) was based on the MAINLINE
	 * HWRTData layout (flags@0x48/freelists@0x50) which does NOT match our DDK-25.2
	 * layout (freelists@176, verified by the FW trace loading our freelist), so that
	 * lead was invalid. Use the full AGP mapping matching the DDK. */
	hrt->apsFreeLists[RGXFW_LOCAL_FREELIST].ui32Addr = heap_fwaddr(fl_off);
	hrt->apsFreeLists[RGXFW_GLOBAL_FREELIST].ui32Addr = heap_fwaddr(gfl_off);
	/* GLOBAL2 slot MUST be 0 for a non-AGP render (FW-disasm b0019688: the freelist-
	 * compare loop computes "global different" = apsFreeLists[2] != FW-loaded-snapshot;
	 * the snapshot is 0 on the first kick, so ANY nonzero slot-2 -- distinct object OR
	 * aliased to GLOBAL -- yields global:1 / CONTEXT_PB_BASE=0x3, binding geom PM to a
	 * 2nd parameter buffer. apsFreeLists[2]==0 makes the loop skip it -> global:0 /
	 * PB_BASE=0x2, matching the working DDK (non-AGP layout, apsFreeLists[2] ZERO). A
	 * nonzero slot 2 must NOT be aliased -- proven on silicon (aliasing kept 0x3). */
	hrt->apsFreeLists[RGXFW_GLOBAL2_FREELIST].ui32Addr = 0;
	(void)gfl2_off;
	hrt->sTailPtrsDevVAddr.uiAddr = tpc.gpu;
	hrt->psVHeapTableDevVAddr.uiAddr = vheap.gpu;
	hrt->psPMMListDevVAddr.uiAddr = mlist.gpu;
	hrt->sRgnHeaderDevVAddr.uiAddr = rgnhdr.gpu;
#ifdef PVR_MACROTILE_NULL
	/* DDK live capture (rgx_triangle_test, working geom): HWRTData
	 * MacrotileArray dev addr = 0x0 (NULL).  Our earlier "NULL stalls DM2" note
	 * is unproven (like the mis-stated PM_INTERACTIVE=OFF).  Match the DDK. */
	hrt->sMacrotileArrayDevVAddr.uiAddr = 0;
	(void)macrotile;
#else
	hrt->sMacrotileArrayDevVAddr.uiAddr = macrotile.gpu;
#endif
	hrt->sRTCDevVAddr.uiAddr = 0; /* RTC only used for Render Target Arrays (layers>1) */
	uint32_t hrt_fwaddr = heap_fwaddr(hrt_off);

	/* ---- FW memory context (points at the page catalogue) ---- */
	uint32_t fmc_off = pvr_heap_alloc(sizeof(RGXFWIF_FWMEMCONTEXT), 8);
	RGXFWIF_FWMEMCONTEXT *fmc = (void *)heap_cpu(fmc_off);

	fmc->sPCDevPAddr.uiAddr = pvr_pc_phys();
	fmc->uiPageCatBaseRegSet = 0xFFFFFFFFu;

	/* ---- render context (geom + frag) + two CCCBs/UFOs (FW heap) ---- */
	uint32_t rc_off = pvr_heap_alloc(sizeof(RGXFWIF_FWRENDERCONTEXT), 8);

	uint32_t gcccb_off = pvr_heap_alloc(32 * 1024, 16);
	uint32_t gcccbctl_off = pvr_heap_alloc(sizeof(RGXFWIF_CCCB_CTL), 8);
	uint32_t gstate_off = pvr_heap_alloc(1024, 8);
	uint32_t gufo_off = pvr_heap_alloc(8, 8);
	uint32_t fcccb_off = pvr_heap_alloc(32 * 1024, 16);
	uint32_t fcccbctl_off = pvr_heap_alloc(sizeof(RGXFWIF_CCCB_CTL), 8);
	uint32_t fstate_off = pvr_heap_alloc(1024, 8);
	uint32_t fufo_off = pvr_heap_alloc(8, 8);
	RGXFWIF_CCCB_CTL *gctl = (void *)heap_cpu(gcccbctl_off);
	RGXFWIF_CCCB_CTL *fctl = (void *)heap_cpu(fcccbctl_off);

	gctl->ui32WrapMask = 32 * 1024 - 1;
	fctl->ui32WrapMask = 32 * 1024 - 1;

	struct pvr_gpu_bo vdm_cs = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 4096);
	/* RGXFWIF_FWRENDERCONTEXT: two RGXFWIF_FWCOMMONCONTEXTs (geom, frag).
	 * Write the fields the FW reads. */
	{
		uint8_t *gc8 = (uint8_t *)heap_cpu(rc_off) + offsetof(RGXFWIF_FWRENDERCONTEXT, sTAContext);
		uint8_t *fc8 = (uint8_t *)heap_cpu(rc_off) + offsetof(RGXFWIF_FWRENDERCONTEXT, s3DContext);

		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, psCCBCtl, heap_fwaddr(gcccbctl_off));
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, psCCB, heap_fwaddr(gcccb_off));
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, psContextState, heap_fwaddr(gstate_off));
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, i32Priority, 0);
		/* FWCommonContextAllocate sets ui32CompatFlags |= HAS_DEFER_COUNT and
		 * ui32MaxDeadlineMS; without these the FW defers the KICK forever
		 * (observed: same KCCB slot 0 re-executed repeatedly). */
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, ui32CompatFlags,
			    RGXFWIF_CONTEXT_COMPAT_FLAGS_HAS_DEFER_COUNT);
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, ui32MaxDeadlineMS, PVR_FW_MAX_DEADLINE_MS);
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, psFWMemContext, heap_fwaddr(fmc_off));
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, ui32ServerCommonContextID, 2);
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, ui32PID, 1);
		PVR_FW_WR32(gc8, RGXFWIF_FWCOMMONCONTEXT, sRangeCheckBeforeUse.eDM, RGXFWIF_DM_GEOM);

		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, psCCBCtl, heap_fwaddr(fcccbctl_off));
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, psCCB, heap_fwaddr(fcccb_off));
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, psContextState, heap_fwaddr(fstate_off));
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, i32Priority, 0);
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, ui32CompatFlags,
			    RGXFWIF_CONTEXT_COMPAT_FLAGS_HAS_DEFER_COUNT);
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, ui32MaxDeadlineMS, PVR_FW_MAX_DEADLINE_MS);
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, psFWMemContext, heap_fwaddr(fmc_off));
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, ui32ServerCommonContextID, 3);
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, ui32PID, 1);
		PVR_FW_WR32(fc8, RGXFWIF_FWCOMMONCONTEXT, sRangeCheckBeforeUse.eDM, RGXFWIF_DM_3D);
		/* geometry context state (RGXFWIF_TACTX_STATE): the VDM call stack. */
		((RGXFWIF_TACTX_STATE *)heap_cpu(gstate_off))
			->asGeomCore[0].uTAReg_VDM_CALL_STACK_POINTER_Init = vdm_cs.gpu;
	}

	/* Static render-context ctx-switch registers: the VDM/TA need a valid
	 * context-state base to run (CR_VDM_CONTEXT_STATE_BASE /
	 * CR_TA_CONTEXT_STATE_BASE = the buffer dev addr, __pvr_address(a,4,4,39) =
	 * the 16-aligned address).  Mesa always sets these (pvr_arch_job_context.c
	 * pvr_render_ctx_ws_static_state_init); leaving them 0 stalls the TA with no
	 * progress.  VOLCANIC_LLS_TA_STATE_BUFFER_SIZE=468; a page each is ample. */
	struct pvr_gpu_bo vdm_ctx_state = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 4096);
	struct pvr_gpu_bo ta_ctx_state = pvr_gpu_bo_alloc(PVR_GENERAL_HEAP_BASE, &gen_b, 4096);
	/* DDK ABI: the ctx-switch base regs are inside the geom common context
	 * (uDMSpecific.sGeom.asCtxSwitch_GeomRegs[core]), NOT a separate static
	 * state. Write VDM/TA CONTEXT_STATE_BASE there for every geom core
	 * (RGX_NUM_GEOM_CORES). Leaving these 0 (the old mainline write went to a
	 * static-state member the DDK FW never reads) made the TA stall with no
	 * context-state buffer -> Alloc=0, PM_CAT_BASE=0, deadline HWR, no fault. */
	{
		uint8_t *gc8cs = (uint8_t *)heap_cpu(rc_off) + offsetof(RGXFWIF_FWRENDERCONTEXT, sTAContext);

		/* FW-disasm (agent a742c6767, b000530e + b001208e): the DDK/FW ZEROES the
		 * asCtxSwitch_GeomRegs base fields at context init and manages them itself; the
		 * host NEVER sets them. On a first kick (resume:0) the working DDK runs with HW
		 * VDM/TA_CONTEXT_STATE_BASE = 0 (they are save/restore targets used only across
		 * preemption). So write 0 to match the DDK exactly and remove these as a variable
		 * (our earlier separate 4KB buffers were a divergence, but provably NOT the
		 * first-kick hang cause). Fault-injection flags still override for diagnosis. */
		uint64_t vdm_base = 0;
		uint64_t vdm_resume = 0;
		uint64_t ta_base = 0;
		(void)vdm_ctx_state; (void)ta_ctx_state;
#ifdef PVR_CS_FAULT_VDMBASE
		vdm_base = 0xE900000000ULL;
#endif
#ifdef PVR_CS_FAULT_VDMRESUME
		vdm_resume = 0xEA00000000ULL;
#endif
#ifdef PVR_CS_FAULT_TABASE
		ta_base = 0xEB00000000ULL;
#endif
		for (uint32_t core = 0; core < RGX_NUM_GEOM_CORES; core++) {
			uint8_t *cs = gc8cs +
				      offsetof(RGXFWIF_FWCOMMONCONTEXT,
					       uDMSpecific.sGeom.asCtxSwitch_GeomRegs) +
				      core * sizeof(RGXFWIF_TAREGISTERS_CSWITCH);

			PVR_FW_WR64(cs, RGXFWIF_TAREGISTERS_CSWITCH,
				    uTAReg_VDM_CONTEXT_STATE_BASE_ADDR, vdm_base);
			PVR_FW_WR64(cs, RGXFWIF_TAREGISTERS_CSWITCH,
				    uTAReg_VDM_CONTEXT_STATE_RESUME_ADDR, vdm_resume);
			PVR_FW_WR64(cs, RGXFWIF_TAREGISTERS_CSWITCH,
				    uTAReg_TA_CONTEXT_STATE_BASE_ADDR, ta_base);
		}
	}

	volatile uint32_t *gufo = (void *)heap_cpu(gufo_off);
	volatile uint32_t *fufo = (void *)heap_cpu(fufo_off);

	*gufo = 0;
	*fufo = 0;

	/* ---- build the geometry command ----
	 * The rig's own scratch for the fields it sets; the command itself is
	 * packed below (RGXFWIF_CMDTA is a user-mode structure the DDK kernel
	 * source does not define: past CMDTA3D_SHARED its dword layout is the
	 * captured one). */
	struct {
		CMDTA3D_SHARED cmd_shared;
		struct {
			uint64_t vdm_ctrl_stream_base;
			uint32_t ppp_ctrl, te_psg;
		} regs;
		uint32_t flags;
	} gcmd;

	memset(&gcmd, 0, sizeof(gcmd));
	gcmd.cmd_shared.sHWRTData.ui32Addr = hrt_fwaddr;
	gcmd.regs.vdm_ctrl_stream_base = vdm.gpu;
#ifdef PVR_VDM_BASE_FAULT_TEST
	/* DIAGNOSTIC: confirmed the VDM faults here -> the FW programs
	 * CR_VDM_CTRL_STREAM_BASE from this field and the VDM DOES fetch the stream. */
	gcmd.regs.vdm_ctrl_stream_base = 0xE800000000ULL;
#endif
#ifdef PVR_VDM_PPP_FAULT_TEST
	/* DIAGNOSTIC: the first VDM block is PPP_STATE_UPDATE referencing ppp.gpu.
	 * Overwrite the PPP addr in the already-built stream with an UNMAPPED addr.
	 * If the VDM faults at 0xE800000000, it PROCESSES blocks (not just fetches)
	 * -> the stall is downstream (draw/PDS/USC/TE finalize). If no fault, the
	 * VDM fetches but never advances past the first block. */
	{
		uint64_t bad = 0xE800000000ULL;
		uint32_t *vw = (uint32_t *)vdm.cpu;
		/* Rebuild VDMCTRL_PPP_STATE0/1 with the bad address. */
		(void)pvr_vdm_emit_ppp_state(vw, bad, ppp_count, NULL);
		sys_cache_data_flush_range(vdm.cpu, 8);
	}
#endif
	/* (tpu_border_colour_table: the packed command sends 0, as the DDK does.) */
	/* CR_PPP_CTRL: MUST match the vertex-shader coordinate output format.  We use
	 * Mesa's pco vs_passthrough, so we MUST use Mesa's CR_PPP_CTRL, not the closed
	 * DDK's (the DDK ships its own compiler's VS with a different coord format).
	 * Mesa (pvr_arch_job_render.c pvr_geom_state_stream_init) sets:
	 *   wclampen (bit1) = 1, fixed_point_format (bit9) = 1  -> 0x202.
	 * The earlier 0x3 (opengl bit0 + wclampen, fixed_point_format=0) was copied
	 * from the DDK dump and is INCOMPATIBLE with Mesa's VS: the PSG then reads the
	 * VS position output in the wrong fixed-point format -> degenerate primitive
	 * -> zero tiled coverage (TE emits nothing) while the ISP bg-clear still runs.
	 */
	/* The DDK dump shows ppp_ctrl=0x3, but that is for the DDK's OWN VS coord
	 * format. Our native path uploads Mesa's pco vs_passthrough (fixed-point
	 * position), which REQUIRES fixed_point_format(bit9)=1 => 0x202. Copying the
	 * DDK's 0x3 made the PSG mis-read the VS position -> degenerate/clipped
	 * primitive -> VHeap empty, TE tiles nothing (root cause, confirmed 2026-09-12
	 * by A/B replay: replay overrides the VDM base to the DDK VS, so 0x3 is right
	 * there; native = Mesa VS needs 0x202). Under replay, keep the DDK's 0x3. */
	/* CR_PPP_CTRL=0x3 (opengl bit0 + wclampen bit1, fixed_point_format=0). The DDK
	 * decode proved its verts are FLOAT screen-space (w=1.0) driven with 0x3; our
	 * passthrough VS emits vec4(in.xyz,1.0) float too, so 0x3 is correct (0x202's
	 * fixed_point_format=1 would misread our float coords as 16.16 -> off-screen). */
	gcmd.regs.ppp_ctrl = 0x3u;
#ifdef PVR_PPP_CTRL_OVERRIDE
	gcmd.regs.ppp_ctrl = PVR_PPP_CTRL_OVERRIDE;   /* diagnostic sweep */
#endif
	/* CR_TE_PSG: completeonterminate (bit17), region_stride (bits0:10)=0.
	 * (Mesa: completeonterminate = geometry_terminate; region_stride from the
	 * rgn-header stride, 0 for this single small RT -- matches the DDK dump.) */
	gcmd.regs.te_psg = 0x00020000u;
	gcmd.flags = PVR_GEOM_FLAGS_FIRSTKICK | PVR_GEOM_FLAGS_LASTKICK;

	/* ---- build the fragment command ----
	 * Scratch as for the geometry command; RGXFWIF_CMD3D is a user-mode
	 * structure, its dword layout past CMDTA3D_SHARED is the captured one. */
	struct {
		CMDTA3D_SHARED cmd_shared;
		struct {
			uint32_t usc_pixel_output_ctrl;
			uint32_t usc_clear_register[4];
			uint32_t isp_bgobjvals, isp_aa, isp_ctl;
			uint32_t event_pixel_pds_info, event_pixel_pds_data;
			uint64_t pbe_word[1][2];
			uint64_t pds_bgnd[3];
		} regs;
	} fcmd;

	memset(&fcmd, 0, sizeof(fcmd));
	fcmd.cmd_shared.sHWRTData.ui32Addr = hrt_fwaddr;
	/* CR_USC_PIXEL_OUTPUT_CTRL as Mesa's pvr_arch_setup_tiles_in_flight()
	 * makes it for a pixel of four output registers on this core: width
	 * 4REGISTERS, the partition mask of its 16 partitions
	 * (usc_pixel_partition_mask) -- the DDK capture's 0x0007fff9. */
	fcmd.regs.usc_pixel_output_ctrl = pvr_cr_usc_pixel_output_ctrl(4, 16);
	/* ISP tile-clear (background) colour = BLUE, so a RED drawn triangle on top is
	 * distinguishable. (This usc_clear_register is the ISP per-tile clear — the
	 * base fill — independent of the drawn geometry, verified: empty VDM + no bg
	 * object still fills the whole tile with this colour.) */
#ifdef PVR_DARK_BG
	fcmd.regs.usc_clear_register[0] = 0xFF000000u; /* black tile-clear (camera-friendly) */
#else
	fcmd.regs.usc_clear_register[0] = 0xFFFF0000u; /* blue (AABBGGRR) */
#endif
	/* CR_ISP_BGOBJVALS: the DDK real-draw capture (ddk_3dcmd dw10) uses 0x00000100
	 * (enablebgtag in the reduced/Volcanic layout = bit8). Our earlier 0x300 was
	 * tuned against the WRONG (mis-delivered) offset; now the frag cmd is hand-packed
	 * to the DDK layout, so match the oracle. */
#ifdef PVR_BGOBJVALS
	fcmd.regs.isp_bgobjvals = PVR_BGOBJVALS;
#else
	fcmd.regs.isp_bgobjvals = pvr_cr_isp_bgobjvals(false, true, 0);
#endif
	fcmd.regs.isp_aa = pvr_cr_isp_aa(1);
	/* SKIP_INIT_HDRS | PROCESS_EMPTY | SAMPLE_POS | PIPE(5): the pipe count is
	 * Mesa's pvr_arch_setup_tiles_in_flight() for this core (six ISP tiles
	 * in flight); skip_init_hdrs and sample_pos as pvr_arch_job_render.c
	 * sets them for a gpu_multicore_support core (the BRN72168/72463
	 * exceptions for targets at most two tiles wide do not apply to the
	 * 64-px rig target). */
	fcmd.regs.isp_ctl = pvr_cr_isp_ctl(true, true, true, 5);
	fcmd.regs.event_pixel_pds_info = pvr_cr_event_pixel_pds_info(1);
	fcmd.regs.event_pixel_pds_data = e_pds_data;
	/* PBE REG_WORD0: tilerelative(bit57) | linestride[49:34] | swizzle 0x68800000
	 * (RGBA identity, linear). linestride is in DWORD units (mainline: (stride-1)
	 * /2 where stride is the row length in DWORDS), NOT bytes: for a 64px row
	 * (256 B) linestride=(64-1)/2=31; using bytes gave 127 -> a 1024 B row -> the
	 * PBE over-wrote 64 KiB instead of 16 KiB (verified with a 256 KiB guard dst). */
	uint32_t pitch_dwords = dst_pitch / 4;

	pvr_pbe_reg_words_linear_rgba(W, H, pitch_dwords, fcmd.regs.pbe_word[0]);
	/* ISP background-object PDS program (CR_PDS_BGRND0/1/3). This is what shades
	 * the background object that fills every (empty) tile for a clear: the pixel
	 * PDS (k_pds -> clear FS) + the texture/uniform PDS (c_pds -> colour upload
	 * into shared regs). WITHOUT this the ISP runs a NULL background PDS and the
	 * 3D faults (VA0 / garbage) and never writes the clear colour. Field layout
	 * from csbgen rogue/cr.xml PDS_BGRND{0,1,3}: offsets are PDS-heap-relative.
	 *   BGRND0: shader_addr[31:4] | texunicode_addr[63:36]  (both <<4)
	 *   BGRND1: texturedata_addr[63:36] (<<4)
	 *   BGRND3: usc_sharedsize[63:55]/16 | pds_texturestatesize[22:16]/4 |
	 *           pds_tempsize[3:0]/4  */
	/* shader_addr = the kick program's DATA start; usc_sharedsize the six
	 * colour words in units of 16, pds_texturestatesize the rig's
	 * bring-up rule (the data size in dwords), pds_tempsize in units of 4. */
	pvr_cr_pds_bgnd(k_pds_data, bg_c_pds_code, bg_c_pds_data, (6u + 15u) / 16u,
			(bg_cinfo.data_size_dw * 4u + 3u) / 4u, (bg_cinfo.temps_used + 3u) / 4u,
			fcmd.regs.pds_bgnd);
	/* (The partial-render background PDS, pds_pr_bgnd, is not in the packed
	 * command: the captured layout has no dwords for it.) */
#ifdef PVR_NO_BGND
	/* DIAGNOSTIC: disable the background object + its PDS entirely. If the
	 * 0xDF4000020 (PDS-heap+0x54000020) fault disappears, the background PDS
	 * path IS the culprit; if it persists, the fault is elsewhere. */
	fcmd.regs.isp_bgobjvals = 0;
	fcmd.regs.pds_bgnd[0] = fcmd.regs.pds_bgnd[1] = fcmd.regs.pds_bgnd[2] = 0;
	LOG_INF("PVR_NO_BGND: background object + PDS disabled");
#endif
	/* (The mainline command also pointed the TPU border table and every ISP
	 * base register at the border page; the packed command below carries
	 * none of them, so they were never sent to the DDK firmware.) */
	/* NOTE: event_pixel_pds_code goes via a separate CR at kick; the frag regs
	 * carry event_pixel_pds_data + info; code offset is e_pds_code. */
	(void)e_pds_code;
	(void)k_pds_data;

	/* ---- write the two CCCBs ---- */
	uint8_t *gring = heap_cpu(gcccb_off);
	uint32_t gwo = 0;
	RGXFWIF_CCB_CMD_HEADER *gh = (void *)(gring + gwo);

	memset(gh, 0, sizeof(*gh));
	gh->eCmdType = RGXFWIF_CCB_CMD_TYPE_GEOM;
	/* DDK RGXFWIF_CMDTA body (value from DDK capture; not derivable from Mesa,
	 * see pvr_drv.h) — layout confirmed by a LIVE capture of the working
	 * DDK triangle (YAKOGL_DRV_DDK GEOMCMD dump, rgx_triangle_test):
	 *   TACmdSize = 64 bytes (16 dwords) total.
	 *   CMDTA3D_SHARED @0 (16B): frame_num, sHWRTData, asPRBuffer[2]
	 *   sGeomRegs @16 (32B, COMPACT — NOT the mainline 64B geom_regs; no vdm_draw_indirect,
	 *     no pds_ctrl/pds_coeff_free_prog):
	 *       vdm_ctrl_stream_base u64 @16  (DDK dw4-5)
	 *       tpu_border_colour_table u64 @24  (DDK sets 0; dw6-7)
	 *       ppp_ctrl @32 (dw8; DDK=0x3, we keep the Mesa value for our stream)
	 *       te_psg  @36 (dw9; =0x20000)
	 *       @40 (dw10)=0, @44 (dw11)=0
	 *   flags @48 (dw12) = 0x3 (FIRSTKICK|LASTKICK)
	 *   partial-render fence UFO {addr @52 (dw13), value @56 (dw14)}, padding @60 (dw15)
	 * A mainline-64B geom_regs here put ppp_ctrl/te_psg at the wrong offsets, leaving a
	 * PM-TE1 tiling base = 0 -> BIF0 page fault reading VA 0x0 (subagent MMU decode). */
	{
		uint32_t dk[16];

		memset(dk, 0, sizeof(dk));
		dk[offsetof(CMDTA3D_SHARED, sCmn.ui32FrameNum) / 4] = gcmd.cmd_shared.sCmn.ui32FrameNum;
		dk[offsetof(CMDTA3D_SHARED, sHWRTData) / 4] = gcmd.cmd_shared.sHWRTData.ui32Addr;
		dk[offsetof(CMDTA3D_SHARED, asPRBuffer[0]) / 4] = gcmd.cmd_shared.asPRBuffer[0].ui32Addr;
		dk[offsetof(CMDTA3D_SHARED, asPRBuffer[1]) / 4] = gcmd.cmd_shared.asPRBuffer[1].ui32Addr;
		dk[4] = (uint32_t)(gcmd.regs.vdm_ctrl_stream_base & 0xFFFFFFFFu);
		dk[5] = (uint32_t)(gcmd.regs.vdm_ctrl_stream_base >> 32);
		dk[6] = 0;   /* tpu_border_colour_table lo (DDK=0) */
		dk[7] = 0;   /* tpu_border_colour_table hi */
		dk[8] = gcmd.regs.ppp_ctrl;
		dk[9] = gcmd.regs.te_psg;
		dk[10] = 0;
		dk[11] = 0;
		dk[12] = gcmd.flags;              /* 0x3 = FIRSTKICK|LASTKICK */
		dk[13] = heap_fwaddr(gufo_off);   /* partial-render fence addr */
		dk[14] = 0;
		dk[15] = 0;
		gh->ui32CmdSize = 64;
		gwo += sizeof(*gh);
		memcpy(gring + gwo, dk, sizeof(dk));
		gwo += 64;
	}
	/* UPDATE geom ufo = 1 */
	RGXFWIF_CCB_CMD_HEADER *gh2 = (void *)(gring + gwo);

	memset(gh2, 0, sizeof(*gh2));
	gh2->eCmdType = RGXFWIF_CCB_CMD_TYPE_UPDATE;
	gh2->ui32CmdSize = ROUND_UP(sizeof(RGXFWIF_UFO), 8);
	gwo += sizeof(*gh2);
	RGXFWIF_UFO *gu = (void *)(gring + gwo);

	gu->puiAddrUFO.ui32Addr = heap_fwaddr(gufo_off);
	gu->ui32Value = 1;
	gwo += ROUND_UP(sizeof(*gu), 8);

	uint8_t *fring = heap_cpu(fcccb_off);
	uint32_t fwo = 0;
#ifdef PVR_COMBINED_KICK
	/* FENCE_PR: the frag waits until the geom signals its done-fence (gufo>=1),
	 * so the combined kick runs geom->frag in order (mainline pvr_queue.c). */
	{
		RGXFWIF_CCB_CMD_HEADER *fpr = (void *)(fring + fwo);

		memset(fpr, 0, sizeof(*fpr));
		fpr->eCmdType = RGXFWIF_CCB_CMD_TYPE_FENCE_PR;
		fpr->ui32CmdSize = ROUND_UP(sizeof(RGXFWIF_UFO), 8);
		fwo += sizeof(*fpr);
		RGXFWIF_UFO *pru = (void *)(fring + fwo);

		pru->puiAddrUFO.ui32Addr = heap_fwaddr(gufo_off);
		pru->ui32Value = 1;
		fwo += ROUND_UP(sizeof(*pru), 8);
	}
#endif
	/* FRAG cmd (ordering enforced by CPU-polling gufo before the frag kick). */
	RGXFWIF_CCB_CMD_HEADER *fh1 = (void *)(fring + fwo);

	memset(fh1, 0, sizeof(*fh1));
	fh1->eCmdType = RGXFWIF_CCB_CMD_TYPE_3D;
#ifdef PVR_FRAG_STRUCT_COPY
#error "PVR_FRAG_STRUCT_COPY (the mainline 3D command struct) was removed with the mainline FWIF header"
#endif
	/* DDK RGXFWIF_CMD3D body (value from DDK capture; not derivable from Mesa,
	 * see pvr_drv.h) — layout confirmed by a LIVE capture of the working
	 * DDK triangle (YAKOGL_DRV_DDK FRAGKICK: 3DCmdSize=320 bytes = 80 dwords). The
	 * mainline-derived 3D command struct this driver once used was 448 bytes with the ROGUE
	 * ([8] usc_clear_register + isp_xtp/tpu/pixel_phantom/brn feature fields) layout;
	 * this FW (BVNC 36.53.104.796) uses the reduced 320-byte layout, so a raw struct
	 * memcpy delivered isp_ctl / isp_bgobjvals / pbe / pds_bgnd to the WRONG FW
	 * registers (isp_ctl read as 0 etc.). Hand-pack to the captured offsets, exactly
	 * as the geom cmd (dk[16]) is packed. Offsets (dword) from ddk_3dcmd[80]:
	 *   0-3   CMDTA3D_SHARED (frame_num, hwrt, prbuf[2])
	 *   4     usc_pixel_output_ctrl        9  isp_bgobjdepth(=1.0f) 10 isp_bgobjvals
	 *   5-8   usc_clear_register[4]        11 isp_aa               12 isp_ctl
	 *   13    event_pixel_pds_info         15 event_pixel_pds_data
	 *   30-31 pbe_word[0][0]               32-33 pbe_word[0][1] (RT dims)
	 *   70-75 pds_bgnd[3]                  79 (trailing const =1) */
	{
		uint32_t dk[80];

		memset(dk, 0, sizeof(dk));
		dk[offsetof(CMDTA3D_SHARED, sCmn.ui32FrameNum) / 4] = fcmd.cmd_shared.sCmn.ui32FrameNum;
		dk[offsetof(CMDTA3D_SHARED, sHWRTData) / 4] = fcmd.cmd_shared.sHWRTData.ui32Addr;
		dk[offsetof(CMDTA3D_SHARED, asPRBuffer[0]) / 4] = fcmd.cmd_shared.asPRBuffer[0].ui32Addr;
		dk[offsetof(CMDTA3D_SHARED, asPRBuffer[1]) / 4] = fcmd.cmd_shared.asPRBuffer[1].ui32Addr;
		dk[4]  = fcmd.regs.usc_pixel_output_ctrl;
		dk[5]  = fcmd.regs.usc_clear_register[0];
		dk[6]  = fcmd.regs.usc_clear_register[1];
		dk[7]  = fcmd.regs.usc_clear_register[2];
		dk[8]  = fcmd.regs.usc_clear_register[3];
		dk[9]  = 0x3f800000u;                 /* isp_bgobjdepth = 1.0f (DDK dw9) */
		dk[10] = fcmd.regs.isp_bgobjvals;
		dk[11] = fcmd.regs.isp_aa;
		dk[12] = fcmd.regs.isp_ctl;
		dk[13] = fcmd.regs.event_pixel_pds_info;
		dk[15] = fcmd.regs.event_pixel_pds_data;
		dk[30] = (uint32_t)(fcmd.regs.pbe_word[0][0] & 0xFFFFFFFFu);
		dk[31] = (uint32_t)(fcmd.regs.pbe_word[0][0] >> 32);
		dk[32] = (uint32_t)(fcmd.regs.pbe_word[0][1] & 0xFFFFFFFFu);
		dk[33] = (uint32_t)(fcmd.regs.pbe_word[0][1] >> 32);
		dk[70] = (uint32_t)(fcmd.regs.pds_bgnd[0] & 0xFFFFFFFFu);
		dk[71] = (uint32_t)(fcmd.regs.pds_bgnd[0] >> 32);
		dk[72] = (uint32_t)(fcmd.regs.pds_bgnd[1] & 0xFFFFFFFFu);
		dk[73] = (uint32_t)(fcmd.regs.pds_bgnd[1] >> 32);
		dk[74] = (uint32_t)(fcmd.regs.pds_bgnd[2] & 0xFFFFFFFFu);
		dk[75] = (uint32_t)(fcmd.regs.pds_bgnd[2] >> 32);
		dk[79] = 0x00000001u;                 /* DDK trailing constant (dw79) */
		fh1->ui32CmdSize = 320;
		fwo += sizeof(*fh1);
		memcpy(fring + fwo, dk, sizeof(dk));
		fwo += 320;
		LOG_INF("FRAG3D handpack: hwrt=0x%x isp_ctl=0x%x bgobjvals=0x%x epds_data=0x%x pbe0=0x%llx pds_bgnd0=0x%llx (320B)",
			dk[1], dk[12], dk[10], dk[15],
			(unsigned long long)fcmd.regs.pbe_word[0][0],
			(unsigned long long)fcmd.regs.pds_bgnd[0]);
	}
	/* UPDATE frag ufo = 1 */
	RGXFWIF_CCB_CMD_HEADER *fh2 = (void *)(fring + fwo);

	memset(fh2, 0, sizeof(*fh2));
	fh2->eCmdType = RGXFWIF_CCB_CMD_TYPE_UPDATE;
	fh2->ui32CmdSize = ROUND_UP(sizeof(RGXFWIF_UFO), 8);
	fwo += sizeof(*fh2);
	RGXFWIF_UFO *fu = (void *)(fring + fwo);

	fu->puiAddrUFO.ui32Addr = heap_fwaddr(fufo_off);
	fu->ui32Value = 1;
	fwo += ROUND_UP(sizeof(*fu), 8);

	/* publish everything to DDR */
	sys_cache_data_flush_range(g_cpu, PVR_GPU_MEM_SIZE);
	barrier_dmem_fence_full();
	/* DDK FW: the CLIENT must NOT pre-write the CCCB ctl WriteOffset.  The FW's
	 * kick handler compares the ctl's CURRENT WriteOffset against the KICK's
	 * ui32CWoffUpdate to detect NEW work; if we set the ctl to gwo up front, the
	 * kick's CWoff==ctl.WriteOffset -> "no new work" -> DM never scheduled
	 * ("Bg Task complete DM Bitfield: 0").  Leave the ctl WriteOffset at 0 and let
	 * the KICK (client_woff_update=gwo/fwo) advance it. */
	sys_cache_data_flush_range(gctl, sizeof(*gctl));
	sys_cache_data_flush_range(fctl, sizeof(*fctl));
	barrier_dmem_fence_full();

	/* ---- two separate KICKs: GEOM then FRAG (frag ordered by its FENCE_PR) ---- */
	uint32_t geom_ctx_fa = heap_fwaddr(rc_off) + offsetof(RGXFWIF_FWRENDERCONTEXT, sTAContext);
	uint32_t frag_ctx_fa = heap_fwaddr(rc_off) + offsetof(RGXFWIF_FWRENDERCONTEXT, s3DContext);
	RGXFWIF_KCCB_CMD gkick, fkick;

	memset(&gkick, 0, sizeof(gkick));
	gkick.eCmdType = RGXFWIF_KCCB_CMD_KICK;
	gkick.uCmdData.sCmdKickData.psContext.ui32Addr = geom_ctx_fa;
	gkick.uCmdData.sCmdKickData.ui32CWoffUpdate = gwo;
	gkick.uCmdData.sCmdKickData.ui32CWrapMaskUpdate = gctl->ui32WrapMask;
	/* Attach the HWRTData cleanup_ctl so the FW can track this render's
	 * submitted/executed command counts and close out the geom DM cleanly.
	 * Mainline (pvr_cccb.c fill_cmd_kick_data) always passes this; without it
	 * the geom DM never retires -> DM2 signature-check watchdog HWR, and the
	 * RTData state is left UNKNOWN. */
	gkick.uCmdData.sCmdKickData.ui32NumCleanupCtl = 1;
	gkick.uCmdData.sCmdKickData.apsCleanupCtl[0].ui32Addr =
		hrt_fwaddr + offsetof(RGXFWIF_HWRTDATA, sCleanupState);

	memset(&fkick, 0, sizeof(fkick));
	fkick.eCmdType = RGXFWIF_KCCB_CMD_KICK;
	fkick.uCmdData.sCmdKickData.psContext.ui32Addr = frag_ctx_fa;
	fkick.uCmdData.sCmdKickData.ui32CWoffUpdate = fwo;
	fkick.uCmdData.sCmdKickData.ui32CWrapMaskUpdate = fctl->ui32WrapMask;
	fkick.uCmdData.sCmdKickData.ui32NumCleanupCtl = 1;
	fkick.uCmdData.sCmdKickData.apsCleanupCtl[0].ui32Addr =
		hrt_fwaddr + offsetof(RGXFWIF_HWRTDATA, sCleanupState);

	LOG_INF("render TA+3D: KICK geomctx=0x%x fragctx=0x%x hwrt=0x%x vdm=0x%llx dst=0x%llx",
		geom_ctx_fa, frag_ctx_fa, hrt_fwaddr,
		(unsigned long long)vdm.gpu, (unsigned long long)dst.gpu);
	LOG_INF("render TA+3D: pds.gpu=0x%llx v_pds_data=0x%x v_pds_code=0x%x vtx=0x%llx vs.heap_off=0x%x",
		(unsigned long long)pds.gpu, v_pds_data, v_pds_code,
		(unsigned long long)vtx.gpu, vs.heap_off);
	LOG_INF("render TA+3D: ppp.gpu=0x%llx ppp_count=%u vdm_count=%u", (unsigned long long)ppp.gpu,
		ppp_count, vdm_count);
	LOG_INF("render TA+3D: k_pds d/c=0x%x/0x%x c_pds d/c=0x%x/0x%x e_pds d/c=0x%x/0x%x",
		k_pds_data, k_pds_code, c_pds_data, c_pds_code, e_pds_data, e_pds_code);
	LOG_INF("render TA+3D: BGND0=0x%llx BGND1=0x%llx BGND3=0x%llx",
		(unsigned long long)fcmd.regs.pds_bgnd[0],
		(unsigned long long)fcmd.regs.pds_bgnd[1],
		(unsigned long long)fcmd.regs.pds_bgnd[2]);
	LOG_INF("render TA+3D: kinfo d/c=%u/%u cinfo d/c/temp=%u/%u/%u",
		kinfo.data_size_dw, kinfo.code_size_dw,
		cinfo.data_size_dw, cinfo.code_size_dw, cinfo.temps_used);
	LOG_INF("render TA+3D: cdata=[0x%08x 0x%08x 0x%08x] ccode=[0x%08x 0x%08x] color.gpu=0x%llx",
		cdata[0], cdata[1], cdata[2], ccode[0], ccode[1],
		(unsigned long long)color.gpu);
	LOG_INF("render TA+3D: fs.gpu=0x%llx fs.heap_off=0x%x eot.gpu=0x%llx eot.heap_off=0x%x",
		(unsigned long long)fs.gpu, fs.heap_off,
		(unsigned long long)eot.gpu, eot.heap_off);
	LOG_INF("render TA+3D: kdata=[0x%08x 0x%08x 0x%08x 0x%08x] kcode=0x%08x k_pds_data=0x%x k_pds_code=0x%x",
		kdata[0], kdata[1], kdata[2], kdata[3], kcode[0], k_pds_data, k_pds_code);
	for (uint32_t i = 0; i < ppp_count; i++) {
		LOG_INF("  ppp[%u]=0x%08x", i, ppp_words[i]);
	}
	for (uint32_t i = 0; i < vdm_count; i++) {
		LOG_INF("  vdm[%u]=0x%08x", i, vdm_words[i]);
	}
	{
		uint32_t nd = vinfo.data_size_dw, nc = vinfo.code_size_dw;

		for (uint32_t i = 0; i < nd && i < 8; i++) {
			LOG_INF("  vpds_data[%u]=0x%08x", i, vdata[i]);
		}
		for (uint32_t i = 0; i < nc && i < 3; i++) {
			LOG_INF("  vpds_code[%u]=0x%08x", i, vcode[i]);
		}
	}
	/* Pre-kick MMU cache invalidate (DDK RGXPreKickCacheCommand -> RGXFWIF_KCCB_CMD_MMUCACHE,
	 * issued inside the power lock right before the first KICK). It flushes the GPU MMU/SLC
	 * caches for the render-context PC / freelist page-list / PM page tables the KM just
	 * mapped; the FW programs BIF_PM_CAT_BASE from the render-context PM MMU context during
	 * the kick, so a stale/empty cached PM catalog walk yields PM_CAT_BASE=0. We omit this
	 * (boot-then-KICK), so issue a full-flag invalidate (PC|PD|PT|TLB) once here and let the
	 * FW ack via a sync UFO. (Agent afb0ad5ec: the single FW-directed action our minimal boot
	 * skips between boot and first render.) */
	{
		uint32_t mc_sync_off = pvr_heap_alloc(sizeof(uint32_t), 8);
		volatile uint32_t *mc_sync = (void *)heap_cpu(mc_sync_off);
		RGXFWIF_KCCB_CMD mc;

		*mc_sync = 0;
		sys_cache_data_flush_range((void *)mc_sync, 4);
		memset(&mc, 0, sizeof(mc));
		mc.eCmdType = RGXFWIF_KCCB_CMD_MMUCACHE;
		mc.uCmdData.sMMUCacheData.ui32CacheFlags =
			RGXFWIF_MMUCACHEDATA_FLAGS_PT |
			RGXFWIF_MMUCACHEDATA_FLAGS_PD |
			RGXFWIF_MMUCACHEDATA_FLAGS_PC |
			RGXFWIF_MMUCACHEDATA_FLAGS_TLB;
		mc.uCmdData.sMMUCacheData.sMMUCacheSync.ui32Addr = heap_fwaddr(mc_sync_off);
		mc.uCmdData.sMMUCacheData.ui32MMUCacheSyncUpdateValue = 1;
		pvr_kccb_post(regs, &mc);
		for (int i = 0; i < 2000; i++) {
			sys_cache_data_invd_range((void *)mc_sync, 4);
			if (*mc_sync == 1)
				break;
			k_busy_wait(50);
		}
		LOG_INF("render TA+3D: pre-kick MMUCACHE ack sync=%u", *mc_sync);
	}
	/* Queue BOTH the GEOM (DM2) and FRAG (DM3) kicks up front, before polling —
	 * this mirrors the DDK, which registers both DMs before the render runs.
	 * EMPIRICALLY REQUIRED on this silicon: with only the geom kick queued, the
	 * geom TA never finalizes and idles to the 93.75M-tick deadline -> HWR; with
	 * both queued the render actually executes (the 3D's PR-fence makes the FW
	 * order them correctly). See gpu-powervr-port.md 2026-09-07. */
	pm_snap(regs, "T2-prekick");
#ifdef PVR_COMBINED_KICK
	/* Mainline (pvr_cccb_send_kccb_combined_kick) submits geom+frag as ONE
	 * COMBINED_GEOM_FRAG_KICK so the FW runs them as a properly-fenced pair and
	 * closes out the geom DM cleanly (vs two separate kicks + CPU polling). */
	{
		RGXFWIF_KCCB_CMD ck;

		memset(&ck, 0, sizeof(ck));
		ck.eCmdType = RGXFWIF_KCCB_CMD_COMBINED_TA_3D_KICK;
		ck.uCmdData.sCombinedTA3DCmdKickData.sTACmdKickData =
			gkick.uCmdData.sCmdKickData;
		ck.uCmdData.sCombinedTA3DCmdKickData.s3DCmdKickData =
			fkick.uCmdData.sCmdKickData;
		pvr_kccb_post(regs, &ck);
	}
#else
	pvr_kccb_post(regs, &gkick);
#ifndef PVR_GEOM_ONLY
	pvr_kccb_post(regs, &fkick);
#else
	LOG_INF("PVR_GEOM_ONLY: frag kick skipped (isolate geom completion)");
	(void)fkick;
#endif
#endif
	/* T3: immediately after the geom KICK doorbell (the FW is about to process it;
	 * 0x3d0 trigger may not be written yet). */
	pm_snap(regs, "T3-postkick");
	/* DDK early raw-trace: dump the trace buffer a few ms after the kick, BEFORE
	 * the HWR/retry loop wraps it 1000+ times, so the actual geom/frag execution
	 * (RTData load, VDM, VCE Alloc, lockup) is visible.  Decode offline (sfdecode.py). */
	for (int d = 0; d < 8; d++)   /* ~8 ms: geom has run/stalled, ring not yet wrapped */
		k_busy_wait(1000);
	{
		uint32_t *tb = (uint32_t *)heap_cpu(pvr_ddk_trace_buf_off());
		uint32_t tptr = *(volatile uint32_t *)(heap_cpu(pvr_tracebuf_off()) + 8);
		uint32_t wrapc = *(volatile uint32_t *)(heap_cpu(pvr_tracebuf_off()) + 12);
		uint32_t nn = tptr < 660 ? tptr : 660;

		sys_cache_data_invd_range(heap_cpu(pvr_tracebuf_off()), 16);
		sys_cache_data_invd_range(tb, 660 * 4);
		LOG_INF("EARLY DDK trace: TracePointer=%u WrapCount=%u", tptr, wrapc);
		for (uint32_t i = 0; i < nn; i += 6)
			LOG_INF(" e[%u]: %08x %08x %08x %08x %08x %08x", i,
				tb[i], tb[i+1], tb[i+2], tb[i+3], tb[i+4], tb[i+5]);
	}
	/* While the geom is still executing (GPU powered), read the PM catalog bases +
	 * geom signature regs. Non-zero PM_CAT_VCE0/TE0/ALIST0 => the PM built page
	 * tables (tiling happening); TE_SIG/VCE non-zero => VS output reached the VCE.
	 * All zero => geometry produced no primitives (A) or PM never set up (B). */
	LOG_INF("GEOMSIG@t+8ms: PM_CAT_VCE0=0x%llx TE0=0x%llx ALIST0=0x%llx  TE_SIG=0x%x VCE=0x%x PPP_SIG=0x%x USC_UVS0=0x%x  EVENT=0x%x",
		(unsigned long long)pvr_rd64(regs, 0x1248),
		(unsigned long long)pvr_rd64(regs, 0x1250),  /* TE0 (was 0x1258, wrong) */
		(unsigned long long)pvr_rd64(regs, 0x1260),  /* ALIST0 (was 0x1268, wrong) */
		pvr_rd32(regs, 0x5028), pvr_rd32(regs, 0x5030),
		pvr_rd32(regs, 0x5020), pvr_rd32(regs, 0x5000),
		pvr_rd32(regs, 0x0130));
	/* DID THE TA/GEOMETRY HW EXECUTE AT ALL? PERF_TA_CYCLE(0x6020)/PHASE(0x6008) are
	 * the geom perf counters; nonzero => the VDM/TA ran cycles (then stalled), zero =>
	 * the TA never started a single cycle (kick didn't reach the HW / DM not powered).
	 * VDM_CTX_STORE(0x430) bit0=COMPLETE (0x1=idle). CLK_STATUS2(0xD208) shows geom-
	 * side clocks. SAFETY_EVENT_STATUS(0xF398) at this early point: if already 0x80
	 * (GPU_LOCKUP) the lockup is near-instant; if 0 the watchdog hasn't tripped yet. */
	pm_snap(regs, "T4-midpoll");
	LOG_INF("GEOMSIG@t+8ms: PERF_TA_CYCLE=0x%x PERF_TA_PHASE=0x%x VDM_CTX_STORE=0x%x CLK_STATUS=0x%llx CLK_STATUS2=0x%llx SAFETY_EVT=0x%x",
		pvr_rd32(regs, 0x6020), pvr_rd32(regs, 0x6008),
		pvr_rd32(regs, 0x0430),
		(unsigned long long)pvr_rd64(regs, 0x0008),
		(unsigned long long)pvr_rd64(regs, 0xD208),
		pvr_rd32(regs, 0xF398));
#ifdef PVR_EARLY_TRACE
	/* Capture the FW trace ~40 ms after the kick: the geom has run (RTData load,
	 * VDM, VS) and stalled, but the ~125 ms watchdog HWR (which spams the ring and
	 * overwrites the early geom lines) has NOT fired yet.  This is the only way to
	 * see the geom's LAST action before it idles (does the VDM reach TERMINATE? a
	 * context store? any TA event?). */
	for (int d = 0; d < 40; d++)
		k_busy_wait(1000);
	sys_cache_data_invd_range(g_cpu, PVR_GPU_MEM_SIZE);
	LOG_INF("EARLY-TRACE (t+40ms, pre-HWR) ----");
	{
		uint32_t *ring = (uint32_t *)heap_cpu(pvr_ddk_trace_buf_off());

		pvr_fw_trace_dump((const RGXFWIF_TRACEBUF *)heap_cpu(pvr_tracebuf_off()),
				  &ring, 1, fwt_emit, NULL);
	}
	LOG_INF("EARLY-TRACE end ----");
#endif
	for (int i = 0; i < 4000; i++) {
		sys_cache_data_invd_range((void *)gufo, 8);
		if (*gufo >= 1) {
			break;
		}
		/* Service FW->host requests (freelist grow / reconstruction). Without
		 * this the FW stalls its DMs after an HWR waiting for the host, so a
		 * geom that HWRs can never recover/retry. */
		pvr_fwccb_service(regs);
		if (i == 20) {
		}
		k_busy_wait(1000);
	}
	sys_cache_data_invd_range((void *)gufo, 8);
	LOG_INF("render TA+3D: after geom poll, geom_ufo=%u (frag already queued)", *gufo);


	/* ---- wait for the FRAG UFO, then read back ---- */
	for (int i = 0; i < 3000; i++) {
		sys_cache_data_invd_range((void *)fufo, 8);
		if (*fufo >= 1) {
			break;
		}
		pvr_fwccb_service(regs);
		k_busy_wait(1000);
	}
	/* Keep servicing FW->host requests (freelist reconstruction posted at HWR)
	 * so the FW can unblock its DMs; log whether anything was serviced. */
	{
		int svc = 0;

		for (int i = 0; i < 2000; i++) {
			svc += pvr_fwccb_service(regs);
			k_busy_wait(1000);
		}
		LOG_INF("render TA+3D: post-render FWCCB serviced=%d", svc);
	}
	sys_cache_data_invd_range((void *)gufo, 8);
	sys_cache_data_invd_range((void *)fufo, 8);
	LOG_INF("render TA+3D: geom_ufo=%u frag_ufo=%u", *gufo, *fufo);
	sys_cache_data_invd_range(gctl, sizeof(*gctl));
	sys_cache_data_invd_range(fctl, sizeof(*fctl));
	LOG_INF("render TA+3D: geom cccb r/w=%u/%u  frag cccb r/w=%u/%u",
		gctl->ui32ReadOffset, gwo, fctl->ui32ReadOffset, fwo);
	/* Verify what the FW peeks: geom CCCB[0] command header + the geom
	 * FWCommonContext first fields (psCCBCtl@0, psCCB@4, psContextState@8,
	 * flags@12, prio@16, compat@28, deadline@64, memctx@96, ctxid@100, rangeDM@204). */
	{
		uint32_t *gr = (uint32_t *)heap_cpu(gcccb_off);
		uint8_t *gc8 = (uint8_t *)heap_cpu(rc_off) + offsetof(RGXFWIF_FWRENDERCONTEXT, sTAContext);
		uint32_t *g = (uint32_t *)gc8;

		sys_cache_data_invd_range(gr, 128);
		sys_cache_data_invd_range(gc8, 256);
		LOG_INF("render TA+3D: geom CCCB[0] hdr: type=%08x size=%08x ext=%08x int=%08x  body: %08x %08x %08x %08x",
			gr[0], gr[1], gr[2], gr[3], gr[4], gr[5], gr[6], gr[7]);
		/* Full geom body dump (header@0..15, CMDTA3D_SHARED@16, sGeomRegs@32..95) to
		 * compare byte-for-byte against the DDK sGeomRegs layout / a live-DDK capture. */
		for (uint32_t i = 4; i < 28; i += 6)
			LOG_INF("   gbody[%u]: %08x %08x %08x %08x %08x %08x", i,
				gr[i], gr[i+1], gr[i+2], gr[i+3], gr[i+4], gr[i+5]);
		LOG_INF("render TA+3D: geom ctx: ccbctl=%08x ccb=%08x ctxstate=%08x flags=%08x prio=%08x",
			g[0], g[1], g[2], g[3], g[4]);
		LOG_INF("render TA+3D: geom ctx: compat@28=%08x deadline@64=%08x memctx@96=%08x ctxid@100=%08x rangeDM@204=%08x",
			g[7], g[16], g[24], g[25], g[51]);
		LOG_INF("render TA+3D: geom_ctx_fa=%08x geomCCCBctl_fa=%08x (expect ccbctl match)",
			heap_fwaddr(rc_off) + (uint32_t)offsetof(RGXFWIF_FWRENDERCONTEXT, sTAContext),
			heap_fwaddr(gcccbctl_off));
	}
	LOG_INF("render TA+3D: EVENT_STATUS=0x%08x BIF_MMU=0x%08x BIF_REQ=0x%08x%08x "
		"MMU_FAULT=0x%08x%08x",
		pvr_rd32(regs, 0x0130), pvr_rd32(regs, 0x12B0),
		pvr_rd32(regs, 0x12B8 + 4), pvr_rd32(regs, 0x12B8),
		pvr_rd32(regs, 0xE150 + 4), pvr_rd32(regs, 0xE150));
	{
		RGXFWIF_CCB_CTL *kc = (void *)heap_cpu(pvr_kccb_ctl_off());
		RGXFWIF_CCB_CTL *wc = (void *)heap_cpu(pvr_fwccb_ctl_off());

		sys_cache_data_invd_range(kc, sizeof(*kc));
		sys_cache_data_invd_range(wc, sizeof(*wc));
		LOG_INF("render TA+3D: KCCB r/w=%u/%u  FWCCB r/w=%u/%u (FW posts here on error)",
			kc->ui32ReadOffset, kc->ui32WriteOffset, wc->ui32ReadOffset, wc->ui32WriteOffset);
		/* Decode what the FW posted to the FWCCB. cmd_type low byte:
		 * 101=ZSBACK 103=FLGROW 104=FLRECON 105=CTX_RESET 106=DBG_DUMP
		 * 107=STATS 108=CLKRATE 109=GPU_RESTART 110=FW_PAGEFAULT. */
		RGXFWIF_FWCCB_CMD *wcmds = (void *)heap_cpu(pvr_fwccb_cmds_off());

		sys_cache_data_invd_range(wcmds, FWCCB_NUM_CMDS * sizeof(*wcmds));
		for (uint32_t i = 0; i < wc->ui32WriteOffset && i < 4; i++) {
			uint32_t *d = (uint32_t *)&wcmds[i];

			LOG_INF("render TA+3D:   FWCCB[%u] type=0x%08x (low=%u) data=%08x %08x %08x",
				i, wcmds[i].eCmdType, wcmds[i].eCmdType & 0xFF,
				d[2], d[3], d[4]);
		}
	}
	/* DIAGNOSTIC: did the TA actually tile the drawn primitive? Dump the TA
	 * output buffers. If region headers / VHeap / macrotile stay all-zero, the
	 * TA generated no per-tile primitive list -> the 3D rasterises nothing from
	 * geometry (only the background object shades). */
	{
		struct { const char *n; void *p; uint32_t sz; } outs[] = {
			{ "dst", dst.cpu, W * H * 4 },
			{ "vtx", vtx.cpu, 64 },
			{ "color", color.cpu, 24 },
			{ "bgcolor", bgcolor.cpu, 24 },
			{ "border", border.cpu, 4096 },
			{ "tpc", tpc.cpu, 128 },
			{ "vheap", vheap.cpu, 1536 },
			{ "mlist", mlist.cpu, 4096 },
			{ "rgnhdr", rgnhdr.cpu, 256 },
			{ "macrotile", macrotile.cpu, 4096 },
			{ "pbuf", pbuf.cpu, 0x4000 },
		};
		sys_cache_data_invd_range(vtx.cpu, 64);
		sys_cache_data_invd_range(vheap.cpu, 1536);
		uint32_t *vt = (uint32_t *)vtx.cpu, *vh = (uint32_t *)vheap.cpu;

		LOG_INF("VTXIN  %08x %08x %08x | %08x %08x %08x | %08x %08x %08x",
			vt[0], vt[1], vt[2], vt[3], vt[4], vt[5], vt[6], vt[7], vt[8]);
		LOG_INF("VHEAP  %08x %08x %08x %08x %08x %08x %08x %08x",
			vh[0], vh[1], vh[2], vh[3], vh[4], vh[5], vh[6], vh[7]);
		LOG_INF("VHEAP+ %08x %08x %08x %08x %08x %08x %08x %08x",
			vh[8], vh[9], vh[10], vh[11], vh[12], vh[13], vh[14], vh[15]);
		for (uint32_t k = 0; k < sizeof(outs) / sizeof(outs[0]); k++) {
			sys_cache_data_invd_range(outs[k].p, outs[k].sz);
			uint32_t *w = (uint32_t *)outs[k].p;
			uint32_t nz = 0, first = 0xffffffff;

			for (uint32_t i = 0; i < outs[k].sz / 4; i++) {
				if (w[i]) {
					nz++;
					if (first == 0xffffffff)
						first = i;
				}
			}
			LOG_INF("TA-out %-9s nz=%u/%u first@%d [0]=%08x [1]=%08x [2]=%08x [3]=%08x",
				outs[k].n, nz, outs[k].sz / 4,
				(first == 0xffffffff) ? -1 : (int)first,
				w[0], w[1], w[2], w[3]);
		}
	}

	/* Dump the FWCCB (FW->host requests, e.g. FREELIST_GROW).  If the FW posted a
	 * request and the host never answered, the PM/geom stalls -> DM2 lockup. */
	{
		RGXFWIF_CCB_CTL *fc = (void *)heap_cpu(pvr_fwccb_ctl_off());
		RGXFWIF_FWCCB_CMD *fcmds = (void *)heap_cpu(pvr_fwccb_cmds_off());

		sys_cache_data_invd_range(fc, sizeof(*fc));
		sys_cache_data_invd_range(fcmds, FWCCB_NUM_CMDS * sizeof(*fcmds));
		LOG_INF("render TA+3D: FWCCB w=%u r=%u (pending FW->host cmds if w!=r)",
			fc->ui32WriteOffset, fc->ui32ReadOffset);
		for (uint32_t i = 0; i < FWCCB_NUM_CMDS && i < 6; i++) {
			uint32_t *cw = (uint32_t *)&fcmds[i];

			if (cw[0] || cw[1])
				LOG_INF("  fwccb[%u] type=0x%08x data=0x%08x 0x%08x 0x%08x",
					i, cw[0], cw[1], cw[2], cw[3]);
		}
	}

	/* Dump the FW trace (shows the TA/3D kicks + any HWR/assert/lockup). */
	sys_cache_data_invd_range(g_cpu, PVR_GPU_MEM_SIZE);
	LOG_INF("render TA+3D: ---- FW trace ----");
	/* Raw dwords (pvr_fw_trace_dump decodes the same ring, PVR_EARLY_TRACE). */
	{
		uint32_t *tb = (uint32_t *)heap_cpu(pvr_ddk_trace_buf_off());
		uint32_t tptr = *(volatile uint32_t *)(heap_cpu(pvr_tracebuf_off()) +
						       offsetof(RGXFWIF_TRACEBUF, sTraceBuf[0].ui32TracePointer));
		uint32_t wrapc = *(volatile uint32_t *)(heap_cpu(pvr_tracebuf_off()) +
							offsetof(RGXFWIF_TRACEBUF, sTraceBuf[0].ui32WrapCount));
		uint32_t szdw = *(volatile uint32_t *)(heap_cpu(pvr_tracebuf_off()) +
						       offsetof(RGXFWIF_TRACEBUF, ui32TraceBufSizeInDWords));
		uint32_t nn = tptr < 300 ? tptr : 300;

		LOG_INF("DDK FW trace: TracePointer=%u WrapCount=%u SizeInDwords=%u", tptr, wrapc, szdw);
		for (uint32_t i = 0; i < nn; i += 6)
			LOG_INF(" t[%u]: %08x %08x %08x %08x %08x %08x", i,
				tb[i], tb[i+1], tb[i+2], tb[i+3], tb[i+4], tb[i+5]);
		/* KCCB return slots: FW writes a value per slot after it processes that
		 * command; non-zero here proves the FW consumed & completed the slot. */
		{
			uint32_t *rtn = (uint32_t *)heap_cpu(pvr_kccb_rtn_off());
			sys_cache_data_invd_range(rtn, 8 * 4);
			LOG_INF("DDK KCCB rtn[0..7]=%08x %08x %08x %08x %08x %08x %08x %08x",
				rtn[0], rtn[1], rtn[2], rtn[3], rtn[4], rtn[5], rtn[6], rtn[7]);
		}
	}
	LOG_INF("render TA+3D: ---- end FW trace ----");

	/* Dump the HWR info buffer: the FW records the BIF/MMU fault address here
	 * for each HWR event, which survives the HWR that clears the GPU regs. */
	{
		RGXFWIF_HWRINFOBUF *hib = (RGXFWIF_HWRINFOBUF *)heap_cpu(pvr_hwrinfobuf_off());
		LOG_INF("render TA+3D: HWR wr_idx=%u counter=%u dd_req=%u",
			hib->ui32WriteIndex, hib->ui32HwrCounter, hib->ui32DDReqCount);
		for (uint32_t i = 0; i < RGXFWIF_HWINFO_MAX; i++) {
			RGX_HWRINFO *hi = &hib->sHWRInfo[i];

			if (hi->ui32EventStatus == 0 && hi->eHWRType == 0 &&
			    hi->eDM == 0 && hi->uHWRData.sBIFInfo.ui64BIFMMUStatus == 0) {
				continue;
			}
			LOG_INF("  HWR[%u] dm=%u type=%u evt=0x%08x rflags=0x%08x hwrno=%u",
				i, hi->eDM, hi->eHWRType, hi->ui32EventStatus,
				hi->ui32HWRRecoveryFlags, hi->ui32HWRNumber);
			LOG_INF("       BIF req_status=0x%016llx mmu_status=0x%016llx pc=0x%016llx",
				(unsigned long long)hi->uHWRData.sBIFInfo.ui64BIFReqStatus,
				(unsigned long long)hi->uHWRData.sBIFInfo.ui64BIFMMUStatus,
				(unsigned long long)hi->uHWRData.sBIFInfo.ui64PCAddress);
		}
	}

	sys_cache_data_invd_range(dst.cpu, W * H * 4);
	uint32_t px0 = ((volatile uint32_t *)dst.cpu)[0];
	uint32_t pxN = ((volatile uint32_t *)dst.cpu)[W * H - 1];

	LOG_INF("render TA+3D: dst[0]=0x%08x dst[last]=0x%08x (want 0xff0000ff)", px0, pxN);
	/* Precise write-extent map of the oversized dst: one char per 1 KiB block of
	 * the 256 KiB BO. R=block contains draw colour, B=blue, .=all guard(0xA5),
	 * ?=other. dst is WxH=64x64 (16 KiB) so only the first 16 blocks should be
	 * touched; anything past that is overflow. */
	sys_cache_data_invd_range(dst.cpu, 0x40000);
	for (uint32_t blk = 0; blk < 256; blk += 64) {
		char row[72];
		uint32_t n = 0;

		for (uint32_t b = blk; b < blk + 64; b++) {
			volatile uint32_t *w = (volatile uint32_t *)dst.cpu + b * 256;
			uint32_t red = 0, blue = 0, guard = 0, other = 0;

			for (uint32_t i = 0; i < 256; i++) {
				uint32_t v = w[i];

				if (v == 0xFF0000FFu) red++;
				else if (v == 0xFFFF0000u) blue++;
				else if (v == 0xA5A5A5A5u) guard++;
				else other++;
			}
			row[n++] = red ? 'R' : blue ? 'B' : other ? '?' : '.';
		}
		row[n] = 0;
		LOG_INF("dstblk[%3u..] %s", blk, row);
	}
#ifdef PVR_GRADIENT
	/* Gradient verification: print the actual pixel value near each of the 3
	 * vertices + the centroid, so we can confirm interpolation (distinct colours)
	 * and the exact channel order. Verts (64x64): v0=(32,8) v1=(8,56) v2=(56,56). */
	{
		volatile uint32_t *d = (volatile uint32_t *)dst.cpu;

		LOG_INF("GRAD px: v0(31,12)=0x%08x v1(12,52)=0x%08x v2(52,52)=0x%08x centroid(32,40)=0x%08x",
			d[12 * W + 31], d[52 * W + 12], d[52 * W + 52], d[40 * W + 32]);
	}
#endif
	/* Per-pixel shape map. Default samples every 2px (-> 32 wide). Thin primitives
	 * (lines/points) need full 1px resolution or the map misses them. */
#if defined(PVR_PRIM_LINES) || defined(PVR_PRIM_POINTS) || defined(PVR_FINE_MAP)
	const uint32_t map_step = 1;
#else
	const uint32_t map_step = 2;
#endif
	for (uint32_t y = 0; y < H; y += map_step) {
		char row[72];
		uint32_t n = 0;

		for (uint32_t x = 0; x < W && n < 70; x += map_step) {
			uint32_t v = ((volatile uint32_t *)dst.cpu)[y * W + x];
#ifdef PVR_GRADIENT
			/* Dominant-channel map (AABBGGRR): R/G/B by strongest colour
			 * channel, ' ' for black bg, to reveal the Gouraud gradient. */
			uint32_t rr = v & 0xff, gg = (v >> 8) & 0xff, bb = (v >> 16) & 0xff;

			if (rr < 8 && gg < 8 && bb < 8)
				row[n++] = ' ';
			else if (rr >= gg && rr >= bb)
				row[n++] = 'R';
			else if (gg >= bb)
				row[n++] = 'G';
			else
				row[n++] = 'B';
#else
			row[n++] = (v == 0xFF0000FFu) ? 'R' : (v == 0xFFFF0000u) ? 'B' :
				   (v == 0xA5A5A5A5u) ? '.' : (v == 0xFF000000u) ? ' ' : '?';
#endif
		}
		row[n] = 0;
		LOG_INF("px %s", row);
	}
#ifdef PVR_RENDER_TO_FB
	/* Read the PANEL scan-out buffer back (pitch 1024 px) to confirm the triangle
	 * actually landed in the display framebuffer, not just the carveout. */
	if (g_fbcpu) {
		sys_cache_data_invd_range(g_fbcpu, 600 * 4096);
		uint32_t f0 = ((volatile uint32_t *)g_fbcpu)[(H / 2) * 1024 + (W / 2 < 1024 ? W / 2 : 512)];

		LOG_INF("RENDER_TO_FB readback: fb(center)=0x%08x fb(2,2)=0x%08x (bg=0xffff0000 tri=0xff0000ff)",
			f0, ((volatile uint32_t *)g_fbcpu)[2 * 1024 + 2]);
#ifdef PVR_GRADIENT
		/* Full-panel gradient verts: v0=(512,40) v1=(60,560) v2=(964,560). */
		LOG_INF("GRAD fb: v0(512,60)=0x%08x v1(90,540)=0x%08x v2(930,540)=0x%08x mid(512,300)=0x%08x",
			((volatile uint32_t *)g_fbcpu)[60 * 1024 + 512],
			((volatile uint32_t *)g_fbcpu)[540 * 1024 + 90],
			((volatile uint32_t *)g_fbcpu)[540 * 1024 + 930],
			((volatile uint32_t *)g_fbcpu)[300 * 1024 + 512]);
#endif
		uint32_t sy = (H > 64) ? (H / 32) : 2;
		uint32_t sx = (W > 64) ? (W / 32) : 2;

		for (uint32_t y = 0; y < H; y += sy) {
			char row[40];
			uint32_t n = 0;

			for (uint32_t x = 0; x < W && n < 38; x += sx) {
				uint32_t v = ((volatile uint32_t *)g_fbcpu)[y * 1024 + x];

				row[n++] = (v == 0xFF0000FFu) ? 'R' : (v == 0xFFFF0000u) ? 'B' : '.';
			}
			row[n] = 0;
			LOG_INF("fbpx %s", row);
		}
		if (f0 == 0xFF0000FFu) {
			LOG_INF("GPU bring-up milestone: triangle in PANEL scan-out buffer (verified)");
			return 0;
		}
	}
#endif
	if (px0 == 0xFF0000FFu) {
		LOG_INF("GPU bring-up milestone: TA+3D FILL rendered by GPU (verified)");
		return 0;
	}
	return -EIO;
}

