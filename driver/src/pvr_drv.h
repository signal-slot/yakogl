/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_drv.h -- driver-internal API exported by the firmware boot
 * (pvr_fw_boot.c) to the other driver translation units: the render service
 * (pvr_render.c) and, through the YakoGL Zephyr OSAL, the GL backend.
 *
 * The memory, MMU, CCB and freelist parts are wrappers over the Rust kernel
 * side (driver/rust/src/gpu/pvr, docs/source-form.md step 7).
 *
 * Memory model recap (pvr_internal.h): one physically contiguous carveout,
 *   [0, 32 MiB)   FW heap   -- FW-addressable objects (fw_addr = 0xC0000000|off)
 *   [32, 34 MiB)  GPU page tables
 *   [34, 48 MiB)  render-object pool -- bump-allocated, mapped into the GPU MMU
 *                 at a caller-chosen device VA inside one of the fixed heaps
 *                 (PVR_GENERAL_HEAP_BASE, PVR_PDSCODEDATA_HEAP_BASE, ...).
 */
#ifndef PVR_DRV_H
#define PVR_DRV_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

#include "pvr_ddk.h"
#include "pvr_internal.h"
#include "pvr_types.h"   /* struct pvr_gpu_bo */

/* GPU MMU leaf (L0) PTE flags used by the render path (pvr_fw_boot.c). */
#define PVR_L0_FLAGS_DATA      RGX_MMUCTRL_PT_DATA_SLC_BYPASS_CTRL_EN
#define PVR_L0_FLAGS_PM        (RGX_MMUCTRL_PT_DATA_PM_META_PROTECT_EN | \
				RGX_MMUCTRL_PT_DATA_SLC_BYPASS_CTRL_EN)
#define PVR_L0_FLAGS_PM_CACHED RGX_MMUCTRL_PT_DATA_PM_META_PROTECT_EN

/* Store one field of a structure the firmware reads from shared memory, at
 * its offset in the DDK layout: a volatile 32- or 64-bit store (the field
 * must be that wide). PVR_FW_WR32_AT stores at a byte offset, for the few
 * places that write a word over a narrower field and its padding. */
#define PVR_FW_FIELD_IS(type, field, n) \
	((void)sizeof(char[sizeof(((type *)0)->field) == (n) ? 1 : -1]))
#define PVR_FW_WR32_AT(base, off, val) \
	(*(volatile uint32_t *)((uint8_t *)(base) + (off)) = (uint32_t)(val))
#define PVR_FW_WR64_AT(base, off, val) \
	(*(volatile uint64_t *)((uint8_t *)(base) + (off)) = (uint64_t)(val))
#define PVR_FW_WR32(base, type, field, val) \
	(PVR_FW_FIELD_IS(type, field, 4), PVR_FW_WR32_AT(base, offsetof(type, field), val))
#define PVR_FW_WR64(base, type, field, val) \
	(PVR_FW_FIELD_IS(type, field, 8), PVR_FW_WR64_AT(base, offsetof(type, field), val))

/* ---- values that are not in the DDK kernel source ----------------------
 * The client commands (RGXFWIF_CMDTA, RGXFWIF_CMD3D) are structures of the
 * DDK's user-mode driver, which the vendored kernel source does not define:
 * past CMDTA3D_SHARED their layout and flags are the captured ones. */
/* RGXFWIF_CMDTA flags: value from DDK capture; not derivable from Mesa:
 * Mesa 26.2.1's only winsys is drm/imagination's, whose geometry/fragment commands are the mainline firmware's layout, not
 * the DDK 25.2 client command, and the DDK's user-mode client headers are
 * not in the vendored kernel source (docs/source-form.md, step 3). */
#define PVR_GEOM_FLAGS_FIRSTKICK     0x00000001u
#define PVR_GEOM_FLAGS_LASTKICK      0x00000002u
/* RGXFWIF_CMD3D flags: value from DDK capture; not derivable from Mesa
 * either (see above). */
#define PVR_FRAG_FLAGS_DEPTHBUFFER   0x00000080u
#define PVR_FRAG_FLAGS_STENCILBUFFER 0x00000100u
/* RGXFWIF_FWCOMMONCONTEXT.ui32MaxDeadlineMS: the DDK server's workload
 * deadline (MIN(client, ui32MaxWorkloadDeadlineMs = 40000)); value from DDK
 * capture. */
#define PVR_FW_MAX_DEADLINE_MS       40000u

/* ---- state captured at boot ------------------------------------------ */
struct yd_pvr_km;
struct yd_pvr_km *pvr_km(void);   /* the Rust kernel side's context (yakogl_drivers.h) */
mm_reg_t  pvr_regs(void);          /* mapped GPU register base passed to pvr_fw_boot() */
uint8_t  *pvr_drv_carveout_cpu(void);  /* CPU base of the carveout */
uint64_t  pvr_drv_carveout_phys(void); /* physical base of the carveout */
uint64_t  pvr_pc_phys(void);       /* GPU MMU page-catalogue root (physical) */
uint32_t  pvr_kccb_ctl_off(void);  /* FW-heap offset of the KCCB control struct */
uint32_t  pvr_hwrinfobuf_off(void);/* FW-heap offset of the HWR info buffer */
uint32_t  pvr_kccb_rtn_off(void);  /* FW-heap offset of the KCCB return-slot array */
uint32_t  pvr_fwccb_ctl_off(void); /* FW-heap offsets of the FWCCB control and ring */
uint32_t  pvr_fwccb_cmds_off(void);
/* ActivePMLatency written at firmware boot (ms, default PVR_FW_PM_LATENCY_MS = 5; pvr_fw_boot.c). */
extern uint32_t pvr_fw_pm_latency_ms;
/* Print-only: where KCCB slot `slot` and the KCCB write offset live (heap
 * offset, firmware address, physical address, 64-byte line numbers), the
 * read/write offsets, and the slot's command type and MMUCACHE value as the
 * host sees them in memory. `why` names the caller's event. */
void pvr_kccb_probe_slot(uint32_t slot, const char *why);
uint32_t  pvr_tracebuf_off(void);  /* FW-heap offset of the FW trace control struct */
uint32_t  pvr_ddk_trace_buf_off(void); /* FW-heap offset of the DDK thread-0 trace dwords */

/* ---- FW heap (FW-addressable objects) --------------------------------- */
uint32_t  pvr_heap_alloc(uint32_t size, uint32_t align); /* returns heap offset, zeroed */
uint8_t  *pvr_heap_cpu(uint32_t off);
uint32_t  pvr_heap_fwaddr(uint32_t off);
uint64_t  pvr_heap_gpu(uint32_t off);

/* ---- GPU MMU + render-pool buffer objects ------------------------------ */
void pvr_mmu_map(uint64_t va, uint64_t phys, size_t size, uint64_t l0flags);
/* Ring the KCCB doorbell (idempotent; see pvr_fw_boot.c). */
void pvr_kccb_doorbell(mm_reg_t regs);
/* True once if a mapping changed since the last call: what decides whether a
 * kick needs the MMU cache invalidated first. */
bool pvr_mmu_take_dirty(void);
/* Leaf PTE for a GPU VA (0 = unmapped); diagnostics. */
uint64_t pvr_mmu_lookup(uint64_t va);
/* Lab audits (memory-corruption hunts): leaf PTEs outside the carveout and
 * the allowed physical ranges; freelist PFN arrays with entries that are not
 * their own backing pages. Both return the number of bad entries. */
unsigned pvr_mmu_audit(const uint64_t *allow_lo, const uint64_t *allow_hi, unsigned nallow);
unsigned pvr_freelist_audit(void);

/* Bump-allocate `size` bytes (page rounded, zeroed) from the render pool and
 * map them at heap_base + *heap_bump (page rounded); advances *heap_bump. */
struct pvr_gpu_bo pvr_gpu_bo_alloc_f(uint64_t heap_base, uint32_t *heap_bump,
				     uint32_t size, uint64_t l0flags);
/* Hand a freed block back to the bump when it is the newest one; see the
 * definition in pvr_fw_boot.c for why both cursors have to agree. */
bool pvr_gpu_bo_bump_rollback(uint64_t heap_base, uint32_t *heap_bump, uint32_t heap_off,
			      const uint8_t *cpu, uint32_t size);
struct pvr_gpu_bo pvr_gpu_bo_alloc(uint64_t heap_base, uint32_t *heap_bump, uint32_t size);
/* Map a block at an exact heap offset (gpu::pvr::mem bo_place; the self-test rig). */
struct pvr_gpu_bo pvr_gpu_bo_place(uint64_t heap_base, uint32_t start_off, uint32_t size,
				   uint64_t l0flags);
/* PM-protected allocation (freelist PFN arrays, MList): 16 KiB aligned. */
struct pvr_gpu_bo pvr_gpu_bo_alloc_pm(uint64_t heap_base, uint32_t *heap_bump, uint32_t size);
uint64_t pvr_bo_phys(const struct pvr_gpu_bo *bo);
/* The shared device-VA bump of the PM-META protect heap (0xFA...). */
uint32_t *pvr_pm_heap_bump(void);

/* Build a PM freelist (backing pages + PFN array + FW struct); returns the FW
 * heap offset of the RGXFWIF_FREELIST. Registers it for FWCCB grow /
 * reconstruction servicing. */
/* Shared pool the firmware's freelist grows are served from (pvr_fwccb_service),
 * and the reserve multiplier a list keeps of its own when there is no pool. */
void     pvr_fl_pool_init(uint32_t *gen_b, uint32_t pages);
uint32_t pvr_fl_pool_peak_pages(void);
uint32_t pvr_fl_refusals(void);
void     pvr_fl_set_reserve_mult(uint32_t mult);
uint32_t pvr_build_freelist(uint32_t *gen_b, uint32_t npages, uint32_t maxpages,
			    uint32_t grow, uint32_t ready, uint32_t fl_id);

/* ---- host <-> firmware command path ------------------------------------ */
/* Post a prebuilt KCCB command and ring the MTS doorbell. */
/* Post one command and return the KCCB slot it was written into. The firmware
 * answers a command in its slot of the return array (pvr_kccb_rtn_off()), which
 * is how a CLEANUP learns whether the resource was busy; callers that do not
 * need an answer ignore the value. */
uint32_t pvr_kccb_post(mm_reg_t regs, const RGXFWIF_KCCB_CMD *cmd);
/* Service FW -> host requests (freelist grow / reconstruction, context reset
 * notifications). Returns the number of commands serviced. */
int  pvr_fwccb_service(mm_reg_t regs);
/* FWCCB freelist-grow requests seen, and grows completed, since insmod. Live
 * counters: they answer "did the firmware ask, and did we answer?" without
 * depending on the dmesg ring still holding the log lines. */
/* Listed pages for each global freelist at boot; 0 = the default derived from
 * the carveout size. Capped at an eighth of the carveout. The pages come from
 * the render pool the GENERAL/PDS/USC heaps share, so raising this lowers the
 * texture ceiling -- a run that raises it records GL_OUT_OF_MEMORY too. */
extern uint32_t pvr_fl_init_pages;

uint32_t pvr_fl_grow_reqs(void);
uint32_t pvr_fl_grows_done(void);

/* The bring-up self-test render (pvr_selftest.c): 0 when the GPU wrote the
 * expected pixel. Run at boot with CONFIG_YAKOGL_DRV_GPU_PVR_SELFTEST. */
int pvr_render_ta3d(mm_reg_t regs);

#endif /* PVR_DRV_H */
