/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * verdin_imx95_scmi.h -- the Verdin iMX95's one SCMI channel to the System
 * Manager as raw messages (verdin_imx95_scmi.c), for the Rust side's
 * requests: the GPU's power-on (yd_imx95_osal_up, verdin_imx95_mali.c) and
 * the display's (yd_imx95_display_*, verdin_imx95_display.c).
 */
#ifndef YAKOGL_VERDIN_IMX95_SCMI_H
#define YAKOGL_VERDIN_IMX95_SCMI_H

#include <stddef.h>
#include <stdint.h>

/* One command `tx` (header and payload words as built, tx_len bytes) out,
 * the whole reply (header, status, return values) into `rx`: its length in
 * bytes, or a negative errno (-ENODEV: the channel is not up; -ETIMEDOUT:
 * the System Manager did not free it; -EMSGSIZE: a reply longer than
 * rx_cap or the shared memory). Under the channel's lock, polled; a thread,
 * not an ISR. The Rust side's `scmi` callback as it is. */
int verdin_imx95_scmi(const uint8_t *tx, size_t tx_len, uint8_t *rx, size_t rx_cap);

#endif /* YAKOGL_VERDIN_IMX95_SCMI_H */
