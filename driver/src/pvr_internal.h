/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Internal glue between the PowerVR bring-up (M1/M2, pvr_bringup.c) and the
 * firmware boot (M3/M4, pvr_fw_boot.c).
 */
#ifndef PVR_INTERNAL_H
#define PVR_INTERNAL_H

#include <zephyr/kernel.h>
#include <zephyr/sys/sys_io.h>
#include "pvr_ddk.h"

/* GPU-visible DDR carveout (see docs/gpu-port-plan.md).  Physically contiguous,
 * mapped CPU-non-cached so it is coherent with the non-coherent GPU.
 * Its physical base is chosen at runtime (pvr_carveout_phys): the device
 * tree memory-region on Zephyr (src/pvr_bringup.c). Bare metal builds place it at
 * a fixed address clear of everything else in DDR. */
#if defined(__ZEPHYR__)
extern unsigned long long pvr_carveout_phys;
extern size_t pvr_carveout_size;
#define PVR_GPU_MEM_PHYS   pvr_carveout_phys
#else
#define PVR_GPU_MEM_PHYS   0xC0000000UL
#endif
/* Carveout layout: the physically backed part of the FW heap (4 MiB main
 * region + 192 KiB config region), 2 MiB of GPU page tables, and the
 * render-object pool with everything else (the parameter buffer, the
 * freelists, the backend's frame arenas, every GL object, the per-target
 * depth/stencil buffers: 4x MSAA at 1024x600 alone needs 9.5 MiB of those). */
#define PVR_GPU_MEM_MIN_SIZE (64UL * 1024 * 1024)
/* The carveout's size is the memory-region's (Zephyr), at least
 * PVR_GPU_MEM_MIN_SIZE: a Qt Quick 3D
 * firmware reserves 256 MiB (image based lighting cube maps and their mip
 * chains did not fit into 64). */
#if defined(__ZEPHYR__)
#define PVR_GPU_MEM_SIZE   pvr_carveout_size
#else
#define PVR_GPU_MEM_SIZE   PVR_GPU_MEM_MIN_SIZE
#endif

/* FW heap: a 32 MiB GPU-VA window at 0xE1C0000000 (the size the RISC-V
 * remap and the DDK firmware assume; rgx_heap_firmware.h
 * RGX_FIRMWARE_RAW_HEAP_BASE / _SIZE with RGX_FW_HEAP_SHIFT 25) whose last
 * 192 KiB are the config region (RGX_FIRMWARE_CONFIG_HEAP_SIZE: connection
 * control, OS init, sys init at fixed offsets the firmware is linked against). The host
 * fills well under 1 MiB of the main region, so only its first
 * PVR_FW_HEAP_MAIN_PHYS bytes and the config region get physical pages:
 * main region -> carveout [0, PVR_FW_HEAP_MAIN_PHYS), config region ->
 * carveout [PVR_FW_HEAP_MAIN_PHYS, PVR_FW_HEAP_PHYS_SIZE). Before 2026-09-15
 * the whole window was backed and the pool was left with 30 MiB; a pipelined
 * UI (two sets of frame arenas plus GL objects in flight) ran it dry. */
#define PVR_FW_HEAP_GPU_BASE      RGX_FIRMWARE_RAW_HEAP_BASE
#define PVR_FW_HEAP_SIZE          ((unsigned long)RGX_FIRMWARE_RAW_HEAP_SIZE)
#define PVR_FW_HEAP_CONFIG_OFFSET \
	(PVR_FW_HEAP_SIZE - (unsigned long)RGX_FIRMWARE_CONFIG_HEAP_SIZE)   /* 0x1FD0000 */
#define PVR_FW_HEAP_MAIN_PHYS     (4UL * 1024 * 1024)
#define PVR_FW_HEAP_PHYS_SIZE     (PVR_FW_HEAP_MAIN_PHYS + (PVR_FW_HEAP_SIZE - PVR_FW_HEAP_CONFIG_OFFSET))

/* Page tables: 2 MiB after the backed FW heap. */
#define PVR_PT_AREA_OFFSET    PVR_FW_HEAP_PHYS_SIZE
#define PVR_PT_AREA_END       (PVR_PT_AREA_OFFSET + 2UL * 1024 * 1024)
/* Render-object backing pool: carveout [PVR_PT_AREA_END, PVR_GPU_MEM_SIZE). */
#define PVR_RENDER_POOL_OFFSET PVR_PT_AREA_END

/* Additional GPU-MMU heap bases (rgxheapconfig.h) used by render. */
#define PVR_GENERAL_HEAP_BASE       RGX_GENERAL_HEAP_BASE
/* PM-META protect heap (RGX_PMMETA_PROTECT_HEAP_BASE). The DDK
 * places the freelist PFN arrays and the PM MList here (on-board capture:
 * freelists 0xfa00000000.., MList 0xfa0000c000, PMMETA_PROTECT=1), NOT in the
 * general heap. The PM builds its catalog from allocations in this heap. */
#define PVR_PMMETA_PROTECT_HEAP_BASE RGX_PMMETA_PROTECT_HEAP_BASE
#define PVR_PDSCODEDATA_HEAP_BASE   RGX_PDSCODEDATA_HEAP_BASE
#define PVR_USCCODE_HEAP_BASE       RGX_USCCODE_HEAP_BASE
/* the mainline "transfer frag" heap: the DDK's TQ3D parameters heap */
#define PVR_TRANSFER_FRAG_HEAP_BASE RGX_TQ3DPARAMETERS_HEAP_BASE

static inline uint32_t pvr_rd32(mm_reg_t base, uint32_t off)
{
	return sys_read32(base + off);
}
static inline void pvr_wr32(mm_reg_t base, uint32_t off, uint32_t v)
{
	sys_write32(v, base + off);
}
static inline uint64_t pvr_rd64(mm_reg_t base, uint32_t off)
{
	uint32_t lo = sys_read32(base + off);
	uint32_t hi = sys_read32(base + off + 4);

	return ((uint64_t)hi << 32) | lo;
}
static inline void pvr_wr64(mm_reg_t base, uint32_t off, uint64_t v)
{
	sys_write32((uint32_t)v, base + off);
	sys_write32((uint32_t)(v >> 32), base + off + 4);
}

/* M3+M4: build the GPU MMU, load the RISC-V firmware, create the FWIF control
 * structures and boot the FW processor.  Returns 0 if firmware_started flips.
 * @regs: mapped GPU register base.  The GPU must already be powered (M1) and
 * out of internal soft-reset staging is done here (the DDK's RGXStart). */
int pvr_fw_boot(mm_reg_t regs);

/* Stop the firmware processor (the DDK's RGXStop) after the firmware has been
 * asked to power off. 0, -ETIMEDOUT if the GPU did not idle, -EIO. */
int pvr_fw_stop(void);

#endif /* PVR_INTERNAL_H */
