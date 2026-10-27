/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Route 3 from the direct image on the Verdin iMX8M Plus
 * (CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3; docs/notes/imx8mp-direct-boot.md,
 * "Route 3"): this image loads the Cortex-M7 firmware it carries, brings the
 * panel up, releases the M7 and from then on hands it finished frames through
 * the frame ring (<cluster/frame_ring.h>) instead of flipping the LCDIF.
 * The M7 blends its safety layer into each frame and flips the panel to it.
 *
 * board_imx8mp.c calls the board_route3_* functions marked "board layer";
 * an application reads the M7's vehicle state and the route's state.
 */
#ifndef BOARD_IMX8MP_ROUTE3_H
#define BOARD_IMX8MP_ROUTE3_H

#include <stdbool.h>
#include <stdint.h>

#include <cluster/frame_ring.h>

#ifdef __cplusplus
extern "C" {
#endif

struct board_window;

enum board_route3_state {
	BOARD_ROUTE3_OFF = 0,   /* not set up (no window asked for yet) */
	BOARD_ROUTE3_LOADED,    /* the M7 image loaded, the ring ready; M7 held */
	BOARD_ROUTE3_STARTING,  /* the M7 released; no 'SAF7' status block yet */
	BOARD_ROUTE3_UP,        /* the M7 runs and its map is the one we built for */
	BOARD_ROUTE3_ABSENT,    /* no M7 in time (or none loaded): this image flips */
	BOARD_ROUTE3_MISMATCH,  /* an M7 runs with another map: it owns the panel,
				 * this image publishes nothing (never two writers) */
};

/* The route's state (enum board_route3_state). */
int board_route3_state(void);

/*
 * The newest whole vehicle state the M7 has published, into `out`. False
 * when there is none yet (or it raced the M7 writing): keep the last one.
 */
bool board_route3_vehicle(struct cluster_vehicle_state *out);

/* ---- board layer ---------------------------------------------------------- */

/*
 * Window `n` (0..CONFIG_YAKOGL_BOARD_IMX8MP_WINDOWS - 1): a ring buffer in the
 * M7's carve-out, `w` x `h` (w = 1280, h = 800 + guard rows at most), its
 * CPU view Normal non-cacheable. The first call loads the M7 image and sets
 * the ring up. -EINVAL for another size, -ENOMEM past the buffers, -EIO when
 * the image does not load.
 */
int board_route3_window(unsigned int n, uint32_t w, uint32_t h, struct board_window *out);

/*
 * Show `win` (one of the windows above): the first call brings the panel up
 * on it, publishes it, starts the heartbeat and releases the M7; later calls
 * publish and wait until the M7 has put the frame on the panel. -EAGAIN when
 * the M7 is ABSENT: the caller then flips its windows itself, the panel being
 * on `win` already for the first show, or on the last window shown.
 */
int board_route3_show(const struct board_window *win);

#ifdef __cplusplus
}
#endif

#endif /* BOARD_IMX8MP_ROUTE3_H */
