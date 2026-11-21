/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * verdin_imx95_scmi.h -- the Verdin iMX95's one SCMI channel to the System
 * Manager as raw messages (verdin_imx95_scmi.c), for the Rust side's
 * requests: the GPU's power-on (yd_imx95_osal_up, verdin_imx95_mali.c) and
 * the display's (yd_imx95_display_*, verdin_imx95_display.c), and the
 * record's BBM GPR writes (yd_imx95_bbm_record_*, verdin_imx95_record.c).
 */
#ifndef YAKOGL_VERDIN_IMX95_SCMI_H
#define YAKOGL_VERDIN_IMX95_SCMI_H

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/* One command `tx` (header and payload words as built, tx_len bytes) out,
 * the whole reply (header, status, return values) into `rx`: its length in
 * bytes, or a negative errno (-ENODEV: the channel is not up; -ETIMEDOUT:
 * the System Manager did not free it; -EMSGSIZE: a reply longer than
 * rx_cap or the shared memory). Under the channel's lock, polled; a thread,
 * not an ISR. The Rust side's `scmi` callback as it is. */
int verdin_imx95_scmi(const uint8_t *tx, size_t tx_len, uint8_t *rx, size_t rx_cap);

/* True when the channel is up, nobody holds its lock and no message is in
 * flight. Read with interrupts locked (one core), it stays true until they
 * are unlocked: verdin_imx95_scmi_unlocked may then send. */
bool verdin_imx95_scmi_idle(void);

/* verdin_imx95_scmi without the channel's lock, polled: only with
 * interrupts locked and verdin_imx95_scmi_idle() true (the record sink,
 * verdin_imx95_record.c). Any context. */
int verdin_imx95_scmi_unlocked(const uint8_t *tx, size_t tx_len, uint8_t *rx, size_t rx_cap);

#endif /* YAKOGL_VERDIN_IMX95_SCMI_H */
