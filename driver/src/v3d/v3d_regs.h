/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The registers YakoGL programs on the Raspberry Pi 4: the Broadcom V3D 4.2
 * GPU (its hub and its one core), and the two BCM2711 blocks that put the V3D
 * power domain in and out of reset (the power manager and the V3D's AXI
 * bridge).
 *
 * The offsets and bit positions are hardware facts: what the silicon answers
 * to. Broadcom has published no register document for V3D 4.x. The register
 * names are the ones Mesa's V3D simulator glue uses (Mesa 26.2.1,
 * src/broadcom/simulator/v3dx_simulator.c and v3d_simulator.c, MIT, which
 * take them from the simulator's register header): V3D_HUB_CTL_*, V3D_CTL_0_*,
 * V3D_CLE_0_*, V3D_PTB_0_*, V3D_MMU0_*, V3D_TFU_*, with _SET for a field's
 * mask and _LSB for its lowest bit. A register or field Mesa does not name
 * is named here in the same vocabulary; the BCM2711 blocks have names of our
 * own. The grouping and the comments are ours.
 */
#ifndef V3D_REGS_H
#define V3D_REGS_H

#include <stdint.h>

#define V3D_BIT(n) (1u << (n))

/* ======================================================================== */
/* Where the blocks are (ARM physical addresses on the BCM2711; the device  */
/* tree gives them as bus addresses 0x7exxxxxx, which the ARM sees at        */
/* 0xfexxxxxx).                                                              */
/* ======================================================================== */
#define BCM2711_V3D_HUB_BASE   0xfec00000u   /* 16 KiB */
#define BCM2711_V3D_CORE0_BASE 0xfec04000u   /* 16 KiB */
#define BCM2711_PM_BASE        0xfe100000u   /* the power manager */
#define BCM2711_V3D_ASB_BASE   0xfec11000u   /* the V3D's AXI bridge pair */
#define BCM2711_V3D_GIC_SPI    74            /* one interrupt line, hub and core */

/* ======================================================================== */
/* BCM2711 power manager and the V3D's AXI bridge (names ours)              */
/* ======================================================================== */
/* Every write to a power-manager or bridge register carries this key in its
 * top byte, or the block ignores it. */
#define BCM2711_PM_WRITE_KEY                0x5a000000u
/* The graphics power island's control. */
#define BCM2711_PM_GRAPHICS_CTL             0x10c
/* Set: the V3D is out of reset. */
#define BCM2711_PM_GRAPHICS_V3D_RESET_N_SET V3D_BIT(6)

/* The bridge between the V3D and the AXI bus: a slave port (the CPU's
 * accesses to the V3D) and a master port (the V3D's accesses to memory). A
 * port is stopped by setting its stop request and waiting for the
 * acknowledge to rise, and started by clearing it and waiting for the
 * acknowledge to fall. */
#define BCM2711_ASB_V3D_SLAVE_CTL           0x08
#define BCM2711_ASB_V3D_MASTER_CTL          0x0c
#define BCM2711_ASB_ID                      0x20
#define BCM2711_ASB_CTL_STOP_REQUEST_SET    V3D_BIT(0)
#define BCM2711_ASB_CTL_STOP_ACK_SET        V3D_BIT(1)
/* What BCM2711_ASB_ID reads: "brdg". */
#define BCM2711_ASB_ID_VALUE                0x62726467u

/* ======================================================================== */
/* V3D hub                                                                  */
/* ======================================================================== */
#define V3D_HUB_CTL_UIFCFG     0x00004
#define V3D_HUB_CTL_IDENT0     0x00008
#define V3D_HUB_CTL_IDENT1     0x0000c
#define V3D_HUB_CTL_IDENT2     0x00010
#define V3D_HUB_CTL_IDENT3     0x00014
/* IDENT1: the technology version and revision (4 and 2 on the BCM2711), the
 * number of cores, and whether a TFU is present. */
#define V3D_HUB_CTL_IDENT1_TVER_GET(v)   ((v) & 0xf)
#define V3D_HUB_CTL_IDENT1_REV_GET(v)    (((v) >> 4) & 0xf)
#define V3D_HUB_CTL_IDENT1_NCORES_GET(v) (((v) >> 8) & 0xf)
#define V3D_HUB_CTL_IDENT1_WITH_TFU_SET  V3D_BIT(17)
/* IDENT2: whether the GPU has an MMU. */
#define V3D_HUB_CTL_IDENT2_WITH_MMU_SET  V3D_BIT(8)

/* The hub's interrupts: status, clear (write 1s), mask set and mask clear
 * (a set mask bit keeps the line quiet; the status still reads). */
#define V3D_HUB_CTL_INT_STS     0x00050
#define V3D_HUB_CTL_INT_CLR     0x00058
#define V3D_HUB_CTL_INT_MSK_SET 0x00060
#define V3D_HUB_CTL_INT_MSK_CLR 0x00064
#define V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET V3D_BIT(5) /* a write to a read-only page */
#define V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET V3D_BIT(4) /* an invalid page table entry */
#define V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET V3D_BIT(3) /* an address past the MMU's reach */
#define V3D_HUB_CTL_INT_STS_INT_TFUC_SET    V3D_BIT(1) /* a TFU conversion completed */
#define V3D_HUB_CTL_INT_STS_INT_TFUF_SET    V3D_BIT(0) /* the TFU's input fifo has room */

/* The texture formatting unit (TFU): one conversion is its input image
 * address, stride and planes (IIA, IIS, ICA, IUA), its output address and
 * size (IOA, IOS), the colour-space coefficients (COEF0..3), and ICFG, whose
 * write starts it. CS counts the conversions done (CVTCT, 8 bits, wrapping)
 * and says whether it is busy. The layouts of ICFG, IOA and IOS are Mesa's
 * (src/broadcom/common/v3d_tfu.h), kept with the code that packs them. */
#define V3D_TFU_CS             0x00400
#define V3D_TFU_CS_TFURST_SET  V3D_BIT(31)   /* stops the job, empties the fifo */
#define V3D_TFU_CS_CVTCT_GET(v) (((v) >> 16) & 0xff)
#define V3D_TFU_CS_NFREE_GET(v) (((v) >> 8) & 0x3f) /* free input slots */
#define V3D_TFU_CS_BUSY_SET    V3D_BIT(0)
#define V3D_TFU_SU             0x00404
#define V3D_TFU_ICFG           0x00408
#define V3D_TFU_ICFG_IOC_SET   V3D_BIT(0)    /* raise TFUC when done */
#define V3D_TFU_IIA            0x0040c
#define V3D_TFU_ICA            0x00410
#define V3D_TFU_IIS            0x00414
#define V3D_TFU_IUA            0x00418
#define V3D_TFU_IOA            0x0041c
#define V3D_TFU_IOS            0x00420
#define V3D_TFU_COEF0          0x00424
#define V3D_TFU_COEF0_USECOEF_SET V3D_BIT(31) /* COEF1..3 are read */

/* The MMU. It walks a one-level table of 32-bit entries, one per 4 KiB page
 * of GPU virtual address, through a cache of its own (the MMU cache) and a
 * TLB; both are flushed after the table changes. A fault either aborts the
 * access or sends it to a scratch page (the "illegal address"), and raises
 * one of the hub's MMU interrupts; VIO_ADDR and VIO_ID say where and which
 * client. */
#define V3D_MMU0_CACHE_CTL              0x01000
#define V3D_MMU0_CACHE_CTL_FLUSHING_SET V3D_BIT(2)
#define V3D_MMU0_CACHE_CTL_FLUSH_SET    V3D_BIT(1)
#define V3D_MMU0_CACHE_CTL_ENABLE_SET   V3D_BIT(0)

#define V3D_MMU0_CTL                        0x01200
#define V3D_MMU0_CTL_CAP_EXCEEDED_ABORT_SET V3D_BIT(26)
#define V3D_MMU0_CTL_CAP_EXCEEDED_INT_SET   V3D_BIT(25)
#define V3D_MMU0_CTL_PT_INVALID_ABORT_SET   V3D_BIT(19)
#define V3D_MMU0_CTL_PT_INVALID_INT_SET     V3D_BIT(18)
#define V3D_MMU0_CTL_PT_INVALID_ENABLE_SET  V3D_BIT(16)
#define V3D_MMU0_CTL_WRITE_VIOLATION_ABORT_SET V3D_BIT(11)
#define V3D_MMU0_CTL_WRITE_VIOLATION_INT_SET   V3D_BIT(10)
#define V3D_MMU0_CTL_TLB_CLEARING_SET       V3D_BIT(7)
#define V3D_MMU0_CTL_TLB_CLEAR_SET          V3D_BIT(2)
#define V3D_MMU0_CTL_ENABLE_SET             V3D_BIT(0)
#define V3D_MMU0_PT_PA_BASE    0x01204       /* the table's physical address >> 12 */
#define V3D_MMU0_VIO_ID        0x0122c       /* the faulting client's AXI ID */
#define V3D_MMU0_ILLEGAL_ADR   0x01230       /* the scratch page >> 12 */
#define V3D_MMU0_ILLEGAL_ADR_ENABLE_SET V3D_BIT(31)
#define V3D_MMU0_VIO_ADDR      0x01234       /* the fault's address, its top 32 bits */
#define V3D_MMU0_DEBUG_INFO    0x01238
/* The widths of the physical and virtual addresses, each 30 + the field. */
#define V3D_MMU0_DEBUG_INFO_PA_WIDTH_GET(v) (((v) >> 8) & 0xf)
#define V3D_MMU0_DEBUG_INFO_VA_WIDTH_GET(v) (((v) >> 4) & 0xf)

/* A page table entry: the physical page number in the low 24 bits, valid
 * and writable above it. */
#define V3D_MMU_PAGE_SHIFT        12
#define V3D_MMU_PTE_VALID_SET     V3D_BIT(28)
#define V3D_MMU_PTE_WRITABLE_SET  V3D_BIT(29)
#define V3D_MMU_PTE_PFN_MASK      0x00ffffffu

/* ======================================================================== */
/* V3D core 0                                                               */
/* ======================================================================== */
#define V3D_CTL_0_IDENT0       0x00000
#define V3D_CTL_0_IDENT1       0x00004
#define V3D_CTL_0_IDENT2       0x00008
#define V3D_CTL_0_IDENT0_VER_GET(v)       (((v) >> 24) & 0xff)
/* IDENT1: the VPM size in 8 KiB units, QPUs per slice, slices, revision. */
#define V3D_CTL_0_IDENT1_VPM_SIZE_GET(v)  (((v) >> 28) & 0xf)
#define V3D_CTL_0_IDENT1_QUPS_GET(v)      (((v) >> 8) & 0xf)
#define V3D_CTL_0_IDENT1_NSLC_GET(v)      (((v) >> 4) & 0xf)
#define V3D_CTL_0_IDENT1_REV_GET(v)       ((v) & 0xf)

/* The caches. SLCACTL invalidates the read-only slice caches (four 4-bit
 * fields: the texture/vertex, texture data, uniform and instruction
 * caches). L2TCACTL runs one operation on the texture L2 over the address
 * range L2TFLSTA..L2TFLEND: L2TFLS starts it (and reads 1 until it is done),
 * the L2TFLM field says which (Mesa's cache flush modes: 0 flush, 1 clear,
 * 2 clean), and TMUWCF flushes the TMU's write combiner into the L2T. */
#define V3D_CTL_0_SLCACTL      0x00024
#define V3D_CTL_0_SLCACTL_ALL  0x0f0f0f0fu
#define V3D_CTL_0_L2TCACTL     0x00030
#define V3D_CTL_0_L2TCACTL_TMUWCF_SET V3D_BIT(8)
#define V3D_CTL_0_L2TCACTL_L2TFLM_LSB 1
#define V3D_CTL_0_L2TCACTL_L2TFLS_SET V3D_BIT(0)
#define V3D_CACHE_FLUSH_MODE_FLUSH 0
#define V3D_CACHE_FLUSH_MODE_CLEAR 1
#define V3D_CACHE_FLUSH_MODE_CLEAN 2
#define V3D_CTL_0_L2TFLSTA     0x00034
#define V3D_CTL_0_L2TFLEND     0x00038

/* The core's interrupts, as the hub's. On V3D 4.x the MMU's own faults are
 * the hub's; GMPV (a memory-protection violation) is the core's. */
#define V3D_CTL_0_INT_STS      0x00050
#define V3D_CTL_0_INT_CLR      0x00058
#define V3D_CTL_0_INT_MSK_SET  0x00060
#define V3D_CTL_0_INT_MSK_CLR  0x00064
#define V3D_CTL_0_INT_STS_INT_GMPV_SET    V3D_BIT(5)
#define V3D_CTL_0_INT_STS_INT_OUTOMEM_SET V3D_BIT(2) /* the binner needs memory */
#define V3D_CTL_0_INT_STS_INT_FLDONE_SET  V3D_BIT(1) /* binning done */
#define V3D_CTL_0_INT_STS_INT_FRDONE_SET  V3D_BIT(0) /* rendering done */

/* The control list executor: thread 0 bins, thread 1 renders. Writing a
 * thread's end address (QEA) starts it at its base (QBA); CA and RA are where
 * it is and where a sub-list returns to. The binner also takes the tile
 * allocation memory (QMA, QMS) and the tile state (QTS, with CTQTSEN). CS
 * and EA, the binning thread's control/status and end address, are read into
 * a stalled job's record only. */
#define V3D_CLE_0_CT0CS        0x00100
#define V3D_CLE_0_CT0EA        0x00108
#define V3D_CLE_0_CT0CA        0x00110
#define V3D_CLE_0_CT1CA        0x00114
#define V3D_CLE_0_CT0RA        0x00118
#define V3D_CLE_0_CT1RA        0x0011c
#define V3D_CLE_0_CT0QTS       0x0015c
#define V3D_CLE_0_CT0QTS_CTQTSEN_SET V3D_BIT(1)
#define V3D_CLE_0_CT0QBA       0x00160
#define V3D_CLE_0_CT1QBA       0x00164
#define V3D_CLE_0_CT0QEA       0x00168
#define V3D_CLE_0_CT1QEA       0x0016c
#define V3D_CLE_0_CT0QMA       0x00170
#define V3D_CLE_0_CT0QMS       0x00174

/* The binner's memory pool: where it is allocating now (BPCA, BPCS) and the
 * next block it takes when it runs out (BPOA, BPOS, address then size). */
#define V3D_PTB_0_BPCA         0x00300
#define V3D_PTB_0_BPCS         0x00304
#define V3D_PTB_0_BPOA         0x00308
#define V3D_PTB_0_BPOS         0x0030c

/* The core's error status, read into a stalled job's record. */
#define V3D_ERR_0_STAT         0x00f20

#endif
