/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * gc_identify - the first thing to run on an i.MX8MP: the Vivante driver's
 * milestone 1 (power, clocks, identity, reset), the result on the console,
 * then a bounded wait and a SoC reset back to the boot loader.
 *
 * Boot it from Torizon with tools/kexec-zephyr (the Verdin iMX8MP A53 board
 * in the SafeUI repository puts the image at 0x42000000):
 *   kexec_zephyr zephyr.bin 0x42000000
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/logging/log.h>
#include <gc.h>

#include "gc_sample.h"
#include "gc_panel_console.h"

LOG_MODULE_REGISTER(gc_identify, LOG_LEVEL_INF);

int main(void)
{
	const struct device *gpu = DEVICE_DT_GET_ONE(vivante_gc);

	gc_sample_service_wdog();
	gc_panel_console_stage(4, 0x0000ff);   /* blue: main() runs */

	if (!device_is_ready(gpu)) {
		gc_sample_verdict("gc_identify", false,
				  "the GPU device did not initialise (see the gpu_gc log)");
	} else {
		const struct gc_identity *id = gc_identity(gpu);
		bool expected = (id->model == 0x7000 && id->revision == 0x6204);

		gc_panel_console_stage(5, 0xffffff);   /* white: identity read */

		LOG_INF("GC%x rev 0x%x: %s", id->model, id->revision,
			gc_alive(gpu) ? "alive and idle" : "not alive");
		if (!expected) {
			gc_sample_verdict("gc_identify", false, "unexpected core");
		} else if (!gc_alive(gpu)) {
			gc_sample_verdict("gc_identify", false, "the GPU did not reset to idle");
		} else {
			gc_sample_verdict("gc_identify", true, NULL);
		}
	}

	gc_sample_finish();
	return 0;
}
