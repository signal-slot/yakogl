/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The Verdin iMX95's record sink (CONFIG_YAKOGL_VERDIN_IMX95_RECORD_BBM):
 * the board layer's last record, the four words of the yk-record line, kept
 * in the battery-backed BBNSM GPR4..7, where Linux reads it after the reset
 * (tools/imx95_bbm_record.sh; docs/standalone-boot.md, "The record without a
 * console"). The GPRs are the System Manager's to write: each store is five
 * BBM_GPR_SET messages over the board's SCMI channel, built by the
 * yakogl-drivers crate (yd_imx95_bbm_record_store, soc::imx95::bbm): GPR4
 * cleared, w1..w3, then w0.
 *
 * board_sink_store() runs with interrupts locked, from any context (the
 * fatal handler too), where the channel's lock cannot be waited for. On this
 * one core, with interrupts locked, nobody else can start an exchange: the
 * store goes out at once when nobody is in one (verdin_imx95_scmi_idle),
 * through the channel without its lock. When somebody is (a Zephyr SCMI
 * client or the GPU's or display's sequence was interrupted mid-exchange),
 * or before the channel is up (PRE_KERNEL), the record is kept pending and
 * goes out at the next store, at POST_KERNEL, or at the reset
 * (verdin_imx95_record_flush, from verdin_imx95_reset), which from a thread
 * waits for the lock. A record still pending at the reset is lost to the
 * GPRs (the console line has it).
 */
#include <zephyr/init.h>
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>
#include <errno.h>
#include <stdbool.h>
#include <string.h>

#include <yakogl_drivers.h>
#include "board_internal.h"
#include "verdin_imx95.h"
#include "verdin_imx95_scmi.h"

static uint32_t s_rec[4];
static bool s_pending;
/* the last store's outcome: 0, -EBUSY (deferred), or yd_'s */
static int s_rc = -EBUSY;
static struct yd_imx95_bbm s_why;
static bool s_told; /* the first failure was printed */

/* With interrupts locked: the pending record out now, if nobody is in an
 * exchange. */
static void store_locked(void)
{
	if (!s_pending) {
		return;
	}
	if (!verdin_imx95_scmi_idle()) {
		s_rc = -EBUSY;
		return;
	}
	s_rc = yd_imx95_bbm_record_store(verdin_imx95_scmi_unlocked, s_rec, &s_why);
	/* a refusal is not retried: the SM would refuse it again */
	s_pending = false;
}

void board_sink_store(const uint32_t w[4])
{
	memcpy(s_rec, w, sizeof(s_rec));
	s_pending = true;
	store_locked();
}

static void tell(void)
{
	if (s_rc == 0 || s_rc == -EBUSY || s_told) {
		return;
	}
	s_told = true;
	printk("verdin_imx95: record not stored in the BBM GPRs: rc %d at GPR%u (message %u: "
	       "transport %d, SCMI status %d, malformed %u); the console line has it\n",
	       s_rc, s_why.gpr, s_why.msg, s_why.rc, s_why.status, s_why.malformed);
}

void verdin_imx95_record_flush(void)
{
	unsigned int key = irq_lock();
	uint32_t w[4];
	bool waited = false;

	store_locked();
	if (s_pending && s_rc == -EBUSY && !k_is_in_isr() && arch_irq_unlocked(key)) {
		/* a thread with interrupts on: wait for the channel's lock */
		memcpy(w, s_rec, sizeof(w));
		s_pending = false;
		irq_unlock(key);
		s_rc = yd_imx95_bbm_record_store(verdin_imx95_scmi, w, &s_why);
		waited = true;
	}
	if (!waited) {
		irq_unlock(key);
	}
	if (s_pending) {
		printk("verdin_imx95: the last record is not in the BBM GPRs (the SCMI channel "
		       "was busy); the console line has it\n");
	}
	tell();
}

/* A record stored before the channel was up (the PRE_KERNEL breadcrumbs)
 * goes out once it is (verdin_imx95_scmi.c maps the shared memory at
 * POST_KERNEL 0); the first line says whether the GPRs took it. */
static int record_post_kernel(void)
{
	unsigned int key = irq_lock();

	store_locked();
	irq_unlock(key);
	printk("verdin_imx95: record sink: BBM GPR4..7 over SCMI (%s)\n",
	       s_rc == 0 ? "stored" : s_rc == -EBUSY ? "nothing yet" : "refused");
	tell();
	return 0;
}
SYS_INIT(record_post_kernel, POST_KERNEL, 1);
