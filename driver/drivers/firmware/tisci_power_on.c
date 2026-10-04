/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Switch a list of TISCI devices on early in the boot, before the drivers
 * that touch their registers initialise. The polling TISCI client has this
 * built in (power-on-devices); with Zephyr's own TISCI driver (the A53) this
 * pseudo device does the same from the device tree:
 *
 *   tisci_power_on { compatible = "safeui,tisci-power-on";
 *                    tisci = <&dmsc>; devices = <103 77 78>; };
 *
 * Needed when the previous OS (Linux, before a kexec) left peripherals such
 * as I2C1 powered down.
 */
#define DT_DRV_COMPAT safeui_tisci_power_on

#include <zephyr/device.h>
#include <zephyr/kernel.h>
#include <zephyr/logging/log.h>
#include <safeui/tisci_poll.h>

LOG_MODULE_REGISTER(tisci_power_on, CONFIG_SAFEUI_TISCI_POWER_ON_LOG_LEVEL);

struct power_on_cfg {
	const struct device *tisci;
	const uint16_t *devices;
	size_t num_devices;
};

static int power_on_init(const struct device *dev)
{
	const struct power_on_cfg *cfg = dev->config;
	int failed = 0;

	if (!device_is_ready(cfg->tisci)) {
		LOG_ERR("TISCI not ready");
		return -ENODEV;
	}
	for (size_t i = 0; i < cfg->num_devices; i++) {
		int rc = safeui_tisci_device_ensure_on(cfg->tisci, cfg->devices[i]);

		if (rc) {
			LOG_ERR("device %u on failed (%d)", cfg->devices[i], rc);
			failed++;
		} else {
			LOG_DBG("device %u on", cfg->devices[i]);
		}
	}
	return failed ? -EIO : 0;
}

#define POWER_ON_DEFINE(inst)                                                                      \
	static const uint16_t power_on_devices_##inst[] = DT_INST_PROP(inst, devices);            \
	static const struct power_on_cfg power_on_cfg_##inst = {                                   \
		.tisci = DEVICE_DT_GET(DT_INST_PHANDLE(inst, tisci)),                              \
		.devices = power_on_devices_##inst,                                                \
		.num_devices = ARRAY_SIZE(power_on_devices_##inst),                                \
	};                                                                                         \
	DEVICE_DT_INST_DEFINE(inst, power_on_init, NULL, NULL, &power_on_cfg_##inst, POST_KERNEL,  \
			      CONFIG_SAFEUI_TISCI_POWER_ON_INIT_PRIORITY, NULL);

DT_INST_FOREACH_STATUS_OKAY(POWER_ON_DEFINE)
