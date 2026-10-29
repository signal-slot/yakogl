/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The cold-start timeline of route 3, written by both cores.
 *
 * What it is for: the time from power-on to the panel's first complete
 * picture (the A53's frame with the M7's layer composited over it) is the
 * number to drive down, and it is the slowest of three paths running at once
 * -- the A53's own start and first render, the panel's bring-up, the M7's
 * start and first composite. Which one is the critical path is a measurement,
 * not a guess, so each core stamps its stages here.
 *
 * The clock is the SoC's system counter (SYS_CTR), the one counter both cores
 * can read: the A53 as CNTPCT_EL0, the M7 at SYSCNT_RD + CNTCV. SPL starts it,
 * so a stamp is the time since SPL started the counter, not since power-on;
 * the Boot ROM's part is not in it. `cntfrq` is the A53's CNTFRQ_EL0.
 *
 * Where it is: the 4 KiB at 0x8100_7000, the last page of the M7's
 * non-cacheable window (m7_mem.h: below fb_safety, after the A53's fault
 * page). Neither side flushes anything for the other to read it.
 *
 * Who writes what: the A53 clears the page and writes the header and a53[]
 * before it releases the M7; the M7 writes m7[] only. A stamp of 0 is a
 * stage not reached. Each index is written once, at the first time the stage
 * is reached; `magic` is written last by the A53 so a reader of a dump knows
 * the header is whole.
 *
 * The M7 firmware builds from a copy of this file under its own names: an
 * agreement typed twice is not one, so change both in the same step.
 */
#ifndef CLUSTER_BOOT_TIMELINE_H_
#define CLUSTER_BOOT_TIMELINE_H_

#include <stdint.h>

#define CLUSTER_BOOT_TIMELINE_ADDR 0x81007000u
#define CLUSTER_BOOT_TIMELINE_MAGIC 0x314e4c54u /* 'TLN1' */
#define CLUSTER_BOOT_TIMELINE_VERSION 2u
#define CLUSTER_BOOT_TIMELINE_SLOTS 24u

/* The A53's stages. */
enum cluster_tl_a53 {
	CLUSTER_TL_A53_ENTRY = 0,     /* the image's first C code (PRE_KERNEL_1) */
	CLUSTER_TL_A53_PANEL_START,   /* the panel's cold bring-up begins (POST_KERNEL) */
	CLUSTER_TL_A53_PANEL_ON,      /* the panel chain is up and scanning (backlight off) */
	CLUSTER_TL_A53_KERNEL_UP,     /* APPLICATION init: the kernel is up, main() next */
	CLUSTER_TL_A53_WINDOW,        /* first board_window(): the M7 image is loaded */
	CLUSTER_TL_A53_FRAME1_DRAWN,  /* first board_show(): frame 1 is rendered */
	CLUSTER_TL_A53_M7_RELEASED,   /* the M7 is released */
	CLUSTER_TL_A53_M7_UP,         /* the M7's status block was seen */
	CLUSTER_TL_A53_FIRST_SHOWN,   /* the M7 reports a frame of ours shown */
	/* The application's own progress marks (board_progress()), for the
	 * Slint sample: */
	CLUSTER_TL_A53_APP_WINDOW,    /* asks for its windows */
	CLUSTER_TL_A53_APP_EGL,       /* EGL and the GPU come up */
	CLUSTER_TL_A53_APP_FRAME1,    /* Slint builds and renders frame 1 */
	CLUSTER_TL_A53_APP_SWAPPED,   /* frame 1 swapped: the GPU has finished it */
	/* The second-stage loader before this image (its own record, copied
	 * in by the image when the record is there): */
	CLUSTER_TL_A53_LOADER_ENTRY,  /* the loader's first C code */
	CLUSTER_TL_A53_LOADER_MAIN,   /* its main() */
	CLUSTER_TL_A53_LOADER_READ,   /* the payload is in memory (read or linked in) */
	CLUSTER_TL_A53_LOADER_UNPACKED, /* decoded to the load address, both CRCs checked */
	CLUSTER_TL_A53_LOADER_JUMP,   /* jumping to this image */
	CLUSTER_TL_A53_LOADER_STORED_OK, /* the stored payload's CRC checked */
	CLUSTER_TL_A53_LOADER_DECODED, /* decompressed (the image's CRC next) */
	CLUSTER_TL_A53_LOADER_ZEROED, /* this image's .bss zeroed by the loader */
};

/* The M7's stages. */
enum cluster_tl_m7 {
	CLUSTER_TL_M7_MAIN = 0,       /* main() */
	CLUSTER_TL_M7_LAYER_READY,    /* the safety layer is initialised */
	CLUSTER_TL_M7_LOOP,           /* the render loop starts */
	CLUSTER_TL_M7_FIRST_FRAME,    /* the first A53 frame accepted */
	CLUSTER_TL_M7_FIRST_COMPOSE,  /* the first composite is done */
	CLUSTER_TL_M7_FIRST_LATCH,    /* the first flip latched: on the panel */
	CLUSTER_TL_M7_BACKLIGHT,      /* the backlight was turned on */
	CLUSTER_TL_M7_PRERENDERED,    /* the safety layer drawn while waiting for frame 1 */
	CLUSTER_TL_M7_FRAME1_RENDERED, /* the layer rendered for the first composite */
};

struct cluster_boot_timeline {
	uint32_t magic;
	uint32_t version;
	uint32_t cntfrq; /* Hz */
	uint32_t reserved;
	uint64_t a53[CLUSTER_BOOT_TIMELINE_SLOTS];
	uint64_t m7[CLUSTER_BOOT_TIMELINE_SLOTS];
};

#endif /* CLUSTER_BOOT_TIMELINE_H_ */
