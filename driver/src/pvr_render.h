/*
 * SPDX-License-Identifier: Apache-2.0 AND MIT
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 * SPDX-FileCopyrightText: 2023 Imagination Technologies Ltd.
 * Derived from Mesa 26.2.1 src/imagination (common/device_info/bxs-4-64.h,
 * csbgen; MIT) and from the TI/Img Rogue DDK 25.2 kernel source
 * (firmware-interface layouts, services/server/devices/rgxmem.c; Dual
 * MIT/GPLv2, used under MIT).
 * The upstream part stays under its licence (LICENSES/MIT.txt); our
 * changes are Apache-2.0. See NOTICE.
 *
 * pvr_render.h -- the driver's render service: everything a GEOM+FRAG render
 * needs that is NOT per-draw command generation.
 *
 * It owns the firmware-side objects (render context, client CCBs, freelists,
 * MList, HWRT datasets per render-target size, the static VDM_SYNC / EOT
 * programs) and the submit/wait mechanics (CCCB write, KCCB KICK, UFO poll,
 * FWCCB servicing). All of it is the silicon-proven sequence of the self-test
 * render in pvr_fw_boot.c (pvr_render_ta3d), made reusable and per-frame.
 *
 * Who calls it:
 *   - YakoGL's Zephyr OSAL (yakogl/src/osal/zephyr.c) forwards
 *     gpu_alloc / submit / wait / query here.
 *   - The YakoGL PowerVR backend fills `struct pvr_render_kick` (the
 *     per-render register block it generates: VDM stream base, PPP control,
 *     ISP / PBE / background-object registers) but never calls in directly.
 *
 * No FWIF types leak through this header: the register blocks below are
 * plain values that the service packs into the DDK 25.2 command layouts
 * (RGXFWIF_CMDTA = 16 dwords, RGXFWIF_CMD3D = 80 dwords; both captured live
 * from the working DDK triangle test).
 */
#ifndef PVR_RENDER_H
#define PVR_RENDER_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>   /* the extern bool module parameters below */

#include "pvr_types.h"   /* struct pvr_gpu_bo; no OS dependency */

#ifdef __cplusplus
extern "C" {
#endif

/* Device-VA heaps a render-pool allocation can be placed in. The exec-base
 * relative offsets the PDS/USC state words carry are relative to these. */
enum pvr_render_heap {
	PVR_RENDER_HEAP_GENERAL = 0, /* data: vertices, indices, textures, PPP, VDM, framebuffers */
	PVR_RENDER_HEAP_PDS,         /* PDS code + data programs */
	PVR_RENDER_HEAP_USC,         /* USC shader code */
};

/* Colour attachments one render may write (the hardware allows eight; the
 * end-of-tile programs captured for this driver cover three, which is what a
 * Quick 3D G-buffer pass asks for). */
#define PVR_RENDER_MAX_RT 3

/* Tile buffers: one per colour attachment that does not fit the pixel's four
 * output registers. Three RGBA16F attachments (a Quick 3D G-buffer) spill one. */
#define PVR_RENDER_TILE_BUFFERS 2
/* One tile's worth of spilled outputs: tile_size_x (16) * tile_size_y (16) *
 * max_partitions (16) * usc_min_output_registers_per_pix (2) dwords, times the
 * one cluster this core has (Mesa pvr_get_tile_buffer_size_per_core). */
#define PVR_TILE_BUFFER_BYTES (8192u * 4u)

/* Static objects the backend references when it builds a render. */
struct pvr_render_static {
	uint64_t eot_doutu_src0;        /* DOUTU task control of the EOT USC program */
	uint32_t event_pixel_pds_info;  /* CR_EVENT_PIXEL_PDS_INFO for that program */
	/* The same two, per colour-attachment count (index 1..PVR_RENDER_MAX_RT):
	 * one end-of-tile program per count, each emitting that many surfaces. */
	uint64_t eot_doutu_src0_n[PVR_RENDER_MAX_RT + 1];
	uint32_t event_pixel_pds_info_n[PVR_RENDER_MAX_RT + 1];
	/* The same, for a render whose LAST attachment comes from a tile buffer:
	 * a pixel has four output registers, so a fragment shader with more
	 * outputs than fit stores the rest to memory and the end-of-tile program
	 * loads each one back before emitting it. */
	uint64_t eot_doutu_src0_tb_n[PVR_RENDER_MAX_RT + 1];
	uint32_t event_pixel_pds_info_tb_n[PVR_RENDER_MAX_RT + 1];
	/* Tile buffers the fragment shader may store outputs into; their
	 * addresses go to the shared registers the shader reads them from. */
	uint64_t tile_buffer_gpu[PVR_RENDER_TILE_BUFFERS];
	uint64_t border_gpu;            /* TPU border colour table (a zeroed page) */
	uint64_t heap_base[3];          /* device-VA base per enum pvr_render_heap */
};

/* Geometry (TA) kick registers. */
struct pvr_render_geom_regs {
	uint64_t vdm_ctrl_stream_base;  /* GPU VA of the VDM control stream */
	uint32_t ppp_ctrl;              /* CR_PPP_CTRL (opengl/wclampen/fixed-point bits) */
	uint32_t te_psg;                /* CR_TE_PSG (completeonterminate, region stride) */
};

/* Fragment (3D) kick registers. */
struct pvr_render_frag_regs {
	uint32_t usc_pixel_output_ctrl;
	uint32_t usc_clear_register[4]; /* ISP per-tile clear colour(s) */
	uint32_t isp_bgobjdepth;        /* background depth (float bits) */
	uint32_t isp_bgobjvals;
	uint32_t isp_aa;
	uint32_t isp_ctl;
	uint32_t event_pixel_pds_info;
	uint32_t event_pixel_pds_data;  /* PDS-heap offset of the EOT data segment */
	/* PBE REG_WORD0/1 per colour attachment (the firmware's 3D registers
	 * hold pbe_word[8][3]; this driver fills the first two words of the
	 * first PVR_RENDER_MAX_RT). num_rt 0 means one, for callers written
	 * before multi-target rendering. */
	uint64_t pbe_word[PVR_RENDER_MAX_RT][2];
	uint32_t num_rt;
	uint64_t pds_bgnd[3];           /* background-object PDS (CR_PDS_BGRND0/1/3) */
	/* ISP Z load/store (depth/stencil to memory, cr.xml ISP_ZLSCTL /
	 * ISP_ZLOAD_BASE / ISP_STENCIL_LOAD_BASE); all zero = on-chip only. */
	uint64_t isp_scissor_base;      /* CR_ISP_SCISSOR_BASE: the scissor table (2 dwords per entry) */
	uint64_t isp_dbias_base;        /* CR_ISP_DBIAS_BASE: the depth-bias table (4-byte aligned) */
	uint64_t isp_zlsctl;
	uint64_t isp_zload_store_base;
	uint64_t isp_stencil_load_store_base;
	/* RGXFWIF_CMD3D.flags (PVR_FRAG_FLAGS_DEPTHBUFFER/STENCILBUFFER when
	 * the ISP loads or stores them; PVR_FRAG_FLAGS_GETVISRESULTS 0x20 when
	 * the render has objects with a visibility test). */
	uint32_t cmd_flags;
	/* CR_ISP_OCLQRY_BASE (cr.xml ISP_OCLQRY_BASE, 16-byte aligned): the
	 * buffer the ISP's visibility test counts into, one dword per
	 * TA_STATE_ISPCTL.visreg; 0 when no object tests. RGXFWIF_CMD3D dwords
	 * 22-23 (driver/docs/gpu-recipes/coeff_base_analysis.md). Appended after
	 * cmd_flags, so no earlier offset moved. */
	uint64_t isp_oclqry_base;
};

/* One render = one GEOM kick + one FRAG kick on the same HWRT dataset. */
struct pvr_render_kick {
	uint32_t width, height;         /* render target size (selects the HWRT dataset) */
	uint32_t samples;               /* 1, 2 or 4 samples per pixel (0 = 1) */
	struct pvr_render_geom_regs geom;
	struct pvr_render_frag_regs frag;
};

/* One-time initialisation after pvr_fw_boot() succeeded. Idempotent. */
int pvr_render_init(void);
int pvr_render_static_get(struct pvr_render_static *out);

/* Render-pool allocation (page rounded, zeroed, mapped, SLC-bypassed data). */
int pvr_render_alloc(enum pvr_render_heap heap, uint32_t size, struct pvr_gpu_bo *out);
/* Return a block from pvr_render_alloc() to its heap's free list (size as
 * requested from pvr_render_alloc). The block keeps its GPU mapping and is
 * handed out again by a later pvr_render_alloc(); the caller must know the GPU
 * is done with it (wait for the fence of the last kick that used it). Blocks
 * from pvr_render_map_phys() are counted as dropped (the pool's counters,
 * pvr_render_pool_report). */
void pvr_render_free(const struct pvr_gpu_bo *bo, uint32_t size);
/* The pool's summary at INF (tools/cts/log/investigations/pvr-render-pool.md):
 * its size, the bump now and its high-water, per heap the free bytes, ranges
 * and largest range and the allocations and frees so far, and the losses.
 * Printed at the end of every run; an exhausted allocation prints it at ERR
 * itself. */
void pvr_render_pool_report(void);
/* CONFIG_YAKOGL_PVR_POOL_OWNERS: the driver's owner lines (render statics,
 * freelists, render context, HWRT sets; "pool owner ..."), returns their
 * KiB; and the pool's bytes handed out by the physical cursor and free in
 * the lists, for the owner report's balance. */
uint64_t pvr_render_pool_owners(void);
void pvr_render_pool_usage(uint64_t *handed_out, uint64_t *free_bytes);
/* Map physical memory the caller owns (a scan-out buffer) at a fresh device
 * VA of the heap: no pool pages are consumed; out->cpu is NULL. */
int pvr_render_map_phys(enum pvr_render_heap heap, uint64_t phys, uint32_t size,
			struct pvr_gpu_bo *out);

/* Submit one render. On success *fence_out is the value the completion UFO
 * reaches when the fragment phase retires. */
int pvr_render_submit(const struct pvr_render_kick *kick, uint32_t *fence_out);
/* Wait for a fence: 0 done, -ETIMEDOUT, -EIO on a firmware HWR. */
int pvr_render_wait(uint32_t fence, uint32_t timeout_ms);
/* The highest fence the GPU has completed (no wait). */
uint32_t pvr_render_completed(void);
/* Instrument: the last completed kick's times in microseconds (struct
 * yd_pvr_kick_us, yakogl_drivers.h; gpu::pvr::kick_times). 0; -ENOTSUP when
 * the driver library was built without pvr-kick-times
 * (CONFIG_SAFEUI_GPU_PVR_KICK_TIMES); -ENODATA before a kick completed. */
struct yd_pvr_kick_us;
int pvr_render_last_kick_us(struct yd_pvr_kick_us *out);
/* Instrument: the application's scene starts now; the grow lines that follow
 * say the kicks and the time since (CONFIG_SAFEUI_GPU_PVR_GROW_SCENE;
 * gpu::pvr::render). Does nothing without it. */
void pvr_render_scene_begin(void);
/* Instrument: the freelist and FWCCB counters (YD_PVR_FL_COUNTERS words:
 * grow requests, served, refused, out-of-memory notices, slots read empty,
 * filled, re-posts, stop cause, partial renders), for the reproducers
 * (gl_test 174, 175). */
void pvr_render_fl_counters(uint32_t out[9]);

/* Bring-up: the free lists' high-water marks, per heap (gen, pds, usc): table
 * slots in use at most (of POOL_FREE_SLOTS, `slots_cap`), and ranges on the
 * overflow chain at most and now (gpu::pvr::pool). `bad` counts
 * chain headers found overwritten while free: a client freed memory the GPU
 * still wrote. It should always be zero. */
void pvr_render_debug_freelist(uint32_t slots_peak[3], uint32_t *slots_cap,
			       uint32_t ovf_peak[3], uint32_t ovf_now[3], uint32_t *bad);

/* Post one COMBINED_GEOM_FRAG_KICK per render (the DDK's normal path, the
 * default) or two separate KICKs (its exception path). Exposed so a harness
 * can compare the two without rebuilding. */
extern bool pvr_render_combined_kick;

/* Re-ring the KCCB doorbell periodically while waiting for a fence, for the
 * stall whose invariant is that the LAST kick's write offset is never
 * applied. Off by default; a probe, not a fix. */
extern bool pvr_render_wait_rering;

/* Re-post the last kick once if the fence has not moved after 20 ms. The
 * other half of the pair: the doorbell asks whether a notification was
 * missed, this asks whether another command is needed. Off by default. */
extern bool pvr_render_wait_repost;
/* Kicks re-posted by the wait_repost mitigation since boot. */
extern uint32_t pvr_render_reposts;

/* Wait for the firmware's acknowledgement of the pre-kick MMU cache
 * invalidate before kicking (default on). Without it the fragment phase can
 * run against a stale TLB and its fence never signals. */
extern bool pvr_render_mmu_ack;

/* Send the pre-kick MMU cache invalidate only when a mapping has changed
 * (default on), as the DDK does. Independent of pvr_render_mmu_ack; see
 * mmu_cache_sync() in pvr_render.c for what each one costs. */
extern bool pvr_render_mmu_gate;

/* Print-only instrument for "MMUCACHE ack not seen" (default off): logs each
 * invalidate's sequence value, KCCB slot and acknowledgement time, and after a
 * miss the KCCB state and the sync word over the next 50 ms and after the
 * kick's doorbell. Changes no command and no wait. See pvr_render.c. */
extern bool pvr_render_mmu_ack_probe;

/* Re-ring the KCCB doorbell every N ms while the MMUCACHE acknowledgement
 * wait is pending (0: never, default). */
extern uint32_t pvr_render_mmu_ack_reping;

/* Fail the submit rather than hand out a render-target pair member whose kick
 * is still in flight. Off by default; never yet measured. See mmu-adjacent
 * notes in pvr_render.c. */
extern bool pvr_render_hwrt_refuse;
/* Give a freed block back to the bump when it is the newest allocation, so the
 * pool's high-water mark falls. Off by default; see pvr_render.c. */
extern bool pvr_render_pool_rollback;
/* CONFIG_SAFEUI_GPU_PVR_FL_GROW_ON_DEMAND: one freelist grow kept per list
 * and refilled from the render pool after each grow (off by default). */
extern bool pvr_render_fl_grow_on_demand;
/* CONFIG_SAFEUI_GPU_PVR_FL_READY_PAGES: the global freelists keep the
 * firmware's ready pages (off by default). */
extern bool pvr_render_fl_ready_pages;
/* GPU recoveries (the firmware's HWR counter) since the render service
 * started: the board layer's recovery count on the PowerVR. */
uint32_t pvr_render_hwr_recoveries(void);

/* The render service stopped taking kicks (with the grow knob only:
 * gpu::pvr::freelist::stop): a kick it cannot complete was found, and every
 * submit and wait returns -ESHUTDOWN from then on. The first such return
 * prints the driver's run-end lines (pvr_render_pool_report) and calls the
 * handler once, from the thread that submitted or waited, inside its GL
 * call: cause (gpu::pvr::freelist STOP_*), the fence a wait found it on (0
 * when a submit did), the GPU recoveries so far, the freelist ID and the
 * KiB the cause names. The board layer's handler ends the run; without one
 * the -ESHUTDOWN is only returned (the Linux harness). */
typedef void (*pvr_render_stop_fn)(uint32_t cause, uint32_t fence, uint32_t recoveries,
				   uint32_t fl_id, uint32_t kib);
void pvr_render_set_stop_handler(pvr_render_stop_fn fn);

/* Post the DDK's CLEANUP commands (HWRTDATA for both members, then the three
 * freelists) and wait for the firmware's answer before recycling a pair; a
 * "busy" answer refuses the recycle. Off by default; never measured. */
extern bool pvr_render_hwrt_cleanup;

/* Rebuild an HWRTData from zero on every build, as the DDK's create does,
 * rather than writing over the previous size's struct. Off by default. */
extern bool pvr_render_hwrt_reinit;

/* How often the reuse wait found the member still in flight (refused or not),
 * so a run can show the path was exercised. */
uint32_t pvr_render_debug_hwrt_busy(void);

/* Free-running GPU performance counters (RGX_CR_PERF_*), for attributing a
 * frame's time: [0] TA busy cycles, [1] 3D busy cycles, [2] TA-or-3D busy
 * cycles, [3] SLC0 read stalls, [4] SLC0 write stalls, [5] 3D spin-up
 * cycles, [6] RGX_CR_TIMER low word (1 tick = 256 core clocks). All wrap at
 * 32 bits; diff two reads. */
void pvr_render_perf_read(uint32_t out[7]);

/* PDS-heap offset of the static EOT (pixel-event) PDS CODE the firmware runs
 * (mainline static_data_areas: PDS heap offset 0x80). The per-render DATA
 * segment is what event_pixel_pds_data points at. */
#define PVR_RENDER_EOT_PDS_CODE_OFF 0x80u

#ifdef __cplusplus
}
#endif

#endif /* PVR_RENDER_H */
