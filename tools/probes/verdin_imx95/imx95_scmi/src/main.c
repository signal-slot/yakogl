/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * imx95_scmi: what does the System Manager tell the A55's non-secure agent,
 * read only (README.md)? yd_imx95_scmi_survey (soc::imx95::survey in the
 * drivers crate) sends only messages that read, over the board's raw SCMI
 * channel, and gives one "key = value" line per fact; this prints each as
 * "imx95_scmi: key = value", then the verdict and the PSCI reset.
 * tools/probes/verdin_imx95/imx95_scmi/check.sh compares the lines with
 * expected.txt (Toradex's public SM configuration).
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/printk.h>

#include <yakogl_drivers.h>

#include "verdin_imx95_scmi.h"
#include "vi95_probe.h"

static void line(const char *s, size_t len)
{
	printk(VI95_PROBE_NAME ": %.*s\n", (int)len, s);
}

int main(void)
{
	uint32_t counts[3] = { 0 };
	int rc;

	vi95_probe_banner();
	rc = yd_imx95_scmi_survey(verdin_imx95_scmi, line, counts);
	if (rc == 0) {
		VI95_P("PASS %u facts, %u answered with a status, %u broken", counts[0], counts[1],
		       counts[2]);
	} else {
		VI95_P("FAIL rc %d: the base protocol did not answer (%u facts)", rc, counts[0]);
	}
	vi95_probe_end();
	return 0;
}
