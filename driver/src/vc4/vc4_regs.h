/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * VideoCore IV 3D block (the BCM2837 / RP3A0 of the Raspberry Pi Zero 2 W,
 * Linux's `vc4`): the registers the driver core (vc4_hw.c, vc4_power.c) uses.
 * Hardware facts only, in our own words: the offsets and bit positions are
 * the ones Linux v6.18 uses (drivers/gpu/drm/vc4/vc4_regs.h:36-143, the PM
 * and ASB block drivers/pmdomain/bcm/bcm2835-power.c:79-129, the firmware tags
 * include/soc/bcm2835/raspberrypi-firmware.h) and Broadcom's VideoCore IV 3D
 * Architecture Reference Guide gives (register map p. 82-83 Table 48, thread
 * control p. 85-86 Tables 49-52, caches p. 93-94 Tables 71-72, interrupts
 * p. 95 Tables 75-77, identity p. 96 Tables 78-79). Each group names its
 * source line; docs/notes/pre-study-rpi-zero-2w.md sections 1 to 3 explain
 * them.
 */
#ifndef VC4_REGS_H
#define VC4_REGS_H

#include <stdint.h>

#define VC4_BIT(n) (1u << (n))

/* ---- where the blocks are (ARM physical; bus 0x7exxxxxx = ARM 0x3fxxxxxx,
 * linux:arch/arm/boot/dts/broadcom/bcm2837.dtsi:8-10 `ranges`) ---- */
#define VC4_V3D_ARM_BASE   0x3fc00000u /* v3d@7ec00000, size 0x1000 (bcm2835-common.dtsi:138-142) */
#define VC4_V3D_SIZE       0x1000u
#define VC4_PM_ARM_BASE    0x3f100000u /* pm "pm" reg <0x7e100000 0x114> (bcm2835-common.dtsi:63) */
#define VC4_PM_SIZE        0x114u
#define VC4_ASB_ARM_BASE   0x3f00a000u /* pm "asb" reg <0x7e00a000 0x24> (bcm2835-common.dtsi:64) */
#define VC4_ASB_SIZE       0x24u
#define VC4_MBOX_ARM_BASE  0x3f00b880u /* mailbox@7e00b880 (bcm283x.dtsi:100-105) */

/*
 * The V3D sees ARM physical memory through the uncached bus alias: bus =
 * 0xC0000000 + physical, for physical below 0x3f000000 (bcm2837.dtsi:10
 * `dma-ranges = <0xc0000000 0x00000000 0x3f000000>`; BCM2835 ARM Peripherals
 * p. 5-6: the 0xC alias is "direct uncached"). Every address written to a V3D
 * register (CTnCA, CTnEA, BPOA) is a bus address.
 */
#define VC4_BUS_ALIAS      0xc0000000u
#define VC4_DMA_WINDOW     0x3f000000u /* bytes of ARM physical the alias covers */

/* ---- V3D (vc4_regs.h:36-143; guide Table 48) ---- */
#define V3D_IDENT0   0x000
/* technology version 2 in [31:24], "V3D" in [23:0] (vc4_regs.h:37-41; guide
 * Table 79) */
#define V3D_EXPECTED_IDENT0 0x02443356u
#define V3D_IDENT1   0x004
#define V3D_IDENT1_VPM_SIZE(v) (((v) >> 28) & 0xfu) /* in KiB (vc4_regs.h:44-46) */
#define V3D_IDENT1_NSEM(v)     (((v) >> 16) & 0xffu)
#define V3D_IDENT1_TUPS(v)     (((v) >> 12) & 0xfu)
#define V3D_IDENT1_QUPS(v)     (((v) >> 8) & 0xfu)
#define V3D_IDENT1_NSLC(v)     (((v) >> 4) & 0xfu)
#define V3D_IDENT1_REV(v)      ((v) & 0xfu)
#define V3D_IDENT2   0x008
#define V3D_SCRATCH  0x010 /* free read/write (guide Table 78) */

#define V3D_L2CACTL  0x020
#define V3D_L2CACTL_L2CCLR VC4_BIT(2) /* clear the L2 cache (vc4_regs.h:61) */
#define V3D_L2CACTL_L2CDIS VC4_BIT(1)
#define V3D_L2CACTL_L2CENA VC4_BIT(0)

/* Slice cache clears: one bit per slice in each 4-bit field, write-only
 * (vc4_regs.h:65-73; guide Table 72). */
#define V3D_SLCACTL  0x024
#define V3D_SLCACTL_T1CC(m) (((m) & 0xfu) << 24) /* TMU1 cache */
#define V3D_SLCACTL_T0CC(m) (((m) & 0xfu) << 16) /* TMU0 cache */
#define V3D_SLCACTL_UCC(m)  (((m) & 0xfu) << 8)  /* uniforms cache */
#define V3D_SLCACTL_ICC(m)  ((m) & 0xfu)         /* instruction cache */

/* Interrupts: INTCTL is the latched status, write 1 to clear; INTENA and
 * INTDIS set and clear the enables, a 0 written is ignored (vc4_regs.h:75-81,
 * vc4_irq.c:27-32; guide Tables 75-77). OUTOMEM is "essentially level
 * triggered": its status cannot be cleared while the binner still lacks
 * memory. */
#define V3D_INTCTL   0x030
#define V3D_INTENA   0x034
#define V3D_INTDIS   0x038
#define V3D_INT_SPILLUSE VC4_BIT(3)
#define V3D_INT_OUTOMEM  VC4_BIT(2) /* binner out of overflow memory */
#define V3D_INT_FLDONE   VC4_BIT(1) /* binner list flushed: binning done */
#define V3D_INT_FRDONE   VC4_BIT(0) /* render list done: frame done */
/* the three Linux uses (vc4_irq.c:56-58); SPILLUSE is never enabled */
#define V3D_DRIVER_IRQS  (V3D_INT_OUTOMEM | V3D_INT_FLDONE | V3D_INT_FRDONE)

/* Control-list threads: thread 0 is the binner, thread 1 the renderer
 * (vc4_regs.h:83-108; guide Tables 49-52). Writing CTnCA is allowed only
 * while the thread is stopped and sets "stopped at end"; writing CTnEA then
 * starts it. */
#define V3D_CTNCS(n) (0x100u + 4u * (n))
#define V3D_CTNEA(n) (0x108u + 4u * (n))
#define V3D_CTNCA(n) (0x110u + 4u * (n))
#define V3D_CTNRA0(n) (0x118u + 4u * (n))
#define V3D_CTNLC(n) (0x120u + 4u * (n))
#define V3D_CTNPC(n) (0x128u + 4u * (n))
#define V3D_CTRSTA VC4_BIT(15) /* write 1: stop the thread, reset this register */
#define V3D_CTSEMA_MASK (7u << 12)
#define V3D_CTRTSD_MASK (3u << 8)
#define V3D_CTRUN  VC4_BIT(5)  /* the thread is running */
#define V3D_CTSUBS VC4_BIT(4)
#define V3D_CTERR  VC4_BIT(3)
#define V3D_CTMODE VC4_BIT(0)  /* binning thread only */
#define VC4_THREAD_BIN    0u
#define VC4_THREAD_RENDER 1u

#define V3D_PCS      0x130
#define V3D_PCS_BMOOM    VC4_BIT(8) /* binner out of memory (vc4_regs.h:111) */
#define V3D_PCS_RMBUSY   VC4_BIT(3)
#define V3D_PCS_RMACTIVE VC4_BIT(2)
#define V3D_PCS_BMBUSY   VC4_BIT(1)
#define V3D_PCS_BMACTIVE VC4_BIT(0)
#define V3D_BFC      0x134 /* binning flush count */
#define V3D_RFC      0x138 /* rendering frame count */

/* The binner's memory pool: current (BPCA/BPCS) and overflow (BPOA/BPOS)
 * address and size (vc4_regs.h:119-122). */
#define V3D_BPCA     0x300
#define V3D_BPCS     0x304
#define V3D_BPOA     0x308
#define V3D_BPOS     0x30c
#define V3D_BXCF     0x310

#define V3D_VPMBASE  0x504 /* VPM rows reserved for user programs (vc4_v3d.c:166-170) */
#define V3D_ERRSTAT  0xf20

/* ---- the PM block's GRAFX register (bcm2835-power.c:79-100) ---- */
#define PM_GRAFX           0x10c
#define PM_PASSWORD        0x5a000000u /* every PM and ASB write carries it */
#define PM_V3DRSTN         VC4_BIT(6)  /* V3D out of reset */
#define PM_ISFUNC          VC4_BIT(5)  /* functional isolation off */
#define PM_MRDONE          VC4_BIT(4)  /* memory repair done */
#define PM_MEMREP          VC4_BIT(3)  /* run memory repair */
#define PM_ISPOW           VC4_BIT(2)  /* electrical isolation off */
#define PM_POWOK           VC4_BIT(1)  /* power good */
#define PM_POWUP           VC4_BIT(0)  /* power switches closed */
#define PM_INRUSH_SHIFT    13
#define PM_INRUSH_MASK     (3u << PM_INRUSH_SHIFT)
#define PM_INRUSH_3_5_MA   0u
#define PM_INRUSH_20_MA    3u

/* ---- the AXI async bridge in front of the V3D (bcm2835-power.c:112-129) ---- */
#define ASB_V3D_S_CTRL     0x08
#define ASB_V3D_M_CTRL     0x0c
#define ASB_REQ_STOP       VC4_BIT(0)
#define ASB_ACK            VC4_BIT(1)
#define ASB_AXI_BRDG_ID    0x20
#define ASB_BRDG_ID        0x62726467u /* "BRDG" (bcm2835-power.c:129,638-639) */

/* ---- the firmware's property tags and numbers
 * (raspberrypi-firmware.h:54-83,140-144; raspberrypi-power.c:42-45,119-120;
 * dt-bindings/power/raspberrypi-power.h:22; clk-raspberrypi.c:40,184-188) ---- */
#define RPI_TAG_GET_CLOCK_RATE     0x00030002u
#define RPI_TAG_GET_MAX_CLOCK_RATE 0x00030004u
#define RPI_TAG_GET_DOMAIN_STATE   0x00030030u
#define RPI_TAG_SET_CLOCK_STATE    0x00038001u
#define RPI_TAG_SET_CLOCK_RATE     0x00038002u
#define RPI_TAG_SET_DOMAIN_STATE   0x00038030u
#define VC4_FW_CLK_V3D             5u  /* RPI_FIRMWARE_V3D_CLK_ID */
/* the firmware's domain number: the binding index RPI_POWER_DOMAIN_V3D (10)
 * plus one */
#define VC4_FW_DOMAIN_V3D          11u
#define RPI_CLOCK_STATE_ON         VC4_BIT(0)

#endif
