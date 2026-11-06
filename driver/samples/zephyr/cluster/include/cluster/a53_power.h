/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The A53 down / up protocol between the M7 and the A53 image (route 3,
 * owner's request 2026-11-03: while the screen is touched the A53 side is
 * down; when the touch is released it boots again).
 *
 * Mechanism, the only one NXP documents and its code exercises on the i.MX 8M
 * Plus with the M7 running from DDR (2026-11-03 study, sources there):
 *   - The A53 takes itself down: cores 1-3 PSCI CPU_OFF, then core 0 PSCI
 *     SYSTEM_SUSPEND (0xC400000E) with entry point 0x4020_0000 (the loader,
 *     BL33). NXP TF-A lf-6.6.52-2.2.1 powers the cluster down through the GPC
 *     (imx_domain_suspend, imx8m_psci_common.c:104-136) and, because SRC_GPR10
 *     (LPA_STATUS, platform_def.h:189) holds 0x5555 (M4_LPA_ACTIVE,
 *     gpc_common.c:35, 57-67), keeps the DDR out of retention. The M7 writes
 *     SRC_GPR10 = 0x5555 when it starts. NXP's 8MP sai_low_power_audio demo
 *     uses the same word for the same reason.
 *   - The M7 wakes it with an MU general-purpose interrupt: with the M7 enabled
 *     TF-A unmasks GPC IMR word 2 bit 24 for core 0 (gpc_common.c:254-256),
 *     SPI 88, "MU1 interrupt to A53" (IMX8MPRM Rev. 3 Table 7-1). TF-A then
 *     enters the non-secure entry point with the MMU off: the loader starts
 *     from its first instruction and the A53 image boots again.
 *   - The M7 sees the cluster go off in GPC_PGC_A53SCU_SR (0x303A_090C) bit 0
 *     PSR, write-1-to-clear (IMX8MPRM §5.2.11.4). The same register holds
 *     L2RSTDIS_DEASSERT_CNT (17:8, reset 0x10, never 0) and PUP_CLK_DIV_SEL:
 *     clear PSR by read, OR, write, never by writing 1 alone.
 *
 * MU channel: MU_1, general-purpose interrupt 1 (IMX8MPRM §4.3.6.1.10-11;
 * n = 0 is the high-order bit). The M7 is processor B (MU_1_B, 0x30AB_0000)
 * and sets BCR GIR1 (bit 18); the A53 is processor A (MU_1_A, 0x30AA_0000),
 * enables ACR GIE1 (bit 30) and clears ASR GIP1 (bit 30) by writing 1. The
 * same interrupt is the "go down" request while the A53 runs and the wake-up
 * while it is off; `cmd` says which.
 *
 * The page: the last 512 bytes of the boot timeline page (boot_timeline.h,
 * 0x8100_7000; struct cluster_boot_timeline is about 400 bytes), inside the
 * M7's non-cacheable window, so neither side flushes. Who writes what:
 *   the M7: magic (at its start), cmd, cmd_seq, m7_* stamps;
 *   the A53 (the image and, on a restart, the loader): a53_state, ack_seq,
 *     restarts, suspend_rc, a53_* stamps.
 * Stamps are system-counter ticks (8 MHz, the boot timeline's clock).
 *
 * The M7 firmware builds from a copy of this file under its own names: keep
 * the two layouts identical and change both in the same step.
 */
#ifndef CLUSTER_A53_POWER_H_
#define CLUSTER_A53_POWER_H_
#include <stdint.h>

#define CLUSTER_A53_POWER_ADDR  0x81007E00u
#define CLUSTER_A53_POWER_MAGIC 0x31575041u /* 'APW1' */

/* cmd, written by the M7 before it raises the MU interrupt */
#define CLUSTER_A53_CMD_NONE 0u
#define CLUSTER_A53_CMD_DOWN 1u /* take the A53 side down now */
#define CLUSTER_A53_CMD_UP   2u /* the wake-up: boot again */

/* a53_state, written by the A53 side */
#define CLUSTER_A53_ST_UNKNOWN      0u
#define CLUSTER_A53_ST_RUN_COLD     1u /* booted from power-on */
#define CLUSTER_A53_ST_RUN_RESTART  2u /* booted again after a SYSTEM_SUSPEND */
#define CLUSTER_A53_ST_GOING_DOWN   3u /* DOWN seen, quiescing */
#define CLUSTER_A53_ST_SUSPENDING   4u /* cores 1-3 off, SYSTEM_SUSPEND called */
#define CLUSTER_A53_ST_SUSPEND_FAIL 5u /* SYSTEM_SUSPEND returned; suspend_rc */

/* MU_1 registers and bits */
#define CLUSTER_MU1_A_BASE 0x30AA0000u /* the A53's side */
#define CLUSTER_MU1_B_BASE 0x30AB0000u /* the M7's side */
#define CLUSTER_MU_SR      0x20u
#define CLUSTER_MU_CR      0x24u
#define CLUSTER_MU_GI1_EN  (1u << 30) /* xCR GIE1 */
#define CLUSTER_MU_GI1_REQ (1u << 18) /* xCR GIR1 */
#define CLUSTER_MU_GI1_PND (1u << 30) /* xSR GIP1 */
#define CLUSTER_MU1_A53_SPI 88u       /* GIC SPI of "MU1 interrupt to A53" */

/* SRC_GPR10, the LPA word TF-A reads; GPC_PGC_A53SCU_SR */
#define CLUSTER_SRC_GPR10        0x30390098u
#define CLUSTER_LPA_M_CORE_ACTIVE 0x5555u
#define CLUSTER_GPC_A53SCU_SR    0x303A090Cu
#define CLUSTER_GPC_PSR          (1u << 0)

/* The non-secure entry point the A53 suspends to: the loader (BL33). */
#define CLUSTER_A53_RESTART_ENTRY 0x40200000u

struct cluster_a53_power {
	uint32_t magic;       /* M7: CLUSTER_A53_POWER_MAGIC once it has started */
	uint32_t cmd;         /* M7: CLUSTER_A53_CMD_* */
	uint32_t cmd_seq;     /* M7: +1 with every cmd */
	uint32_t a53_state;   /* A53: CLUSTER_A53_ST_* */
	uint32_t ack_seq;     /* A53: the cmd_seq it acted on */
	uint32_t restarts;    /* A53: boots after a SYSTEM_SUSPEND, so far */
	int32_t suspend_rc;   /* A53: what SYSTEM_SUSPEND returned, if it did */
	uint32_t reserved0;
	uint64_t m7_down_req;    /* M7: DOWN written */
	uint64_t m7_scu_off;     /* M7: GPC A53SCU PSR seen set */
	uint64_t m7_up_req;      /* M7: UP written, MU raised */
	uint64_t a53_down_seen;  /* A53: the DOWN interrupt taken */
	uint64_t a53_suspend;    /* A53: SYSTEM_SUSPEND called */
	uint64_t a53_loader_in;  /* loader: entered again after a suspend */
	uint64_t a53_image_in;   /* A53 image: entry after a suspend */
	uint64_t a53_first_frame;/* A53 image: first frame published after it */
	uint64_t reserved1[6];
};

#endif /* CLUSTER_A53_POWER_H_ */
