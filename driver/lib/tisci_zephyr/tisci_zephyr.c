/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * The safeui_tisci_* API (include/safeui/tisci_poll.h) on top of Zephyr's
 * interrupt-driven TISCI driver (ti,k2g-sci). Cores whose secure proxy has an
 * RX interrupt (the A53) use this; the MCU R5F keeps the polling client in
 * drivers/firmware/tisci_poll.c. The display drivers see the same calls.
 */
#include <zephyr/device.h>
#include <zephyr/drivers/firmware/tisci/tisci.h>
#include <safeui/tisci_poll.h>
#include <string.h>

int safeui_tisci_version(const struct device *dev, char desc[32], uint16_t *revision,
			 uint8_t *abi_major, uint8_t *abi_minor)
{
	struct tisci_version_info ver;
	int rc = tisci_cmd_get_revision(dev, &ver);

	if (rc) {
		return rc;
	}
	if (desc) {
		strncpy(desc, ver.firmware_description, 31);
		desc[31] = '\0';
	}
	if (revision) {
		*revision = ver.firmware_revision;
	}
	if (abi_major) {
		*abi_major = ver.abi_major;
	}
	if (abi_minor) {
		*abi_minor = ver.abi_minor;
	}
	return 0;
}

int safeui_tisci_device_on(const struct device *dev, uint32_t dev_id, bool exclusive)
{
	return exclusive ? tisci_cmd_get_device_exclusive(dev, dev_id)
			 : tisci_cmd_get_device(dev, dev_id);
}

int safeui_tisci_device_ensure_on(const struct device *dev, uint32_t dev_id)
{
	bool on = false;

	if (safeui_tisci_device_is_on(dev, dev_id, &on) == 0 && on) {
		return 0;
	}
	return tisci_cmd_get_device(dev, dev_id);
}

int safeui_tisci_device_off(const struct device *dev, uint32_t dev_id)
{
	return tisci_cmd_put_device(dev, dev_id);
}

int safeui_tisci_device_is_on(const struct device *dev, uint32_t dev_id, bool *on)
{
	bool requested = false, current = false;
	int rc = tisci_cmd_dev_is_on(dev, dev_id, &requested, &current);

	if (rc == 0) {
		*on = current;
	}
	return rc;
}

int safeui_tisci_clock_set_state(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				 bool on)
{
	return on ? tisci_cmd_get_clock(dev, dev_id, clk_id, false, false, false)
		  : tisci_cmd_put_clock(dev, dev_id, clk_id);
}

int safeui_tisci_clock_set_parent(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				  uint8_t parent_id)
{
	return tisci_cmd_clk_set_parent(dev, dev_id, clk_id, parent_id);
}

int safeui_tisci_clock_set_freq(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				uint64_t min_hz, uint64_t target_hz, uint64_t max_hz)
{
	return tisci_cmd_clk_set_freq(dev, dev_id, clk_id, min_hz, target_hz, max_hz);
}

int safeui_tisci_clock_get_freq(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				uint64_t *hz)
{
	return tisci_cmd_clk_get_freq(dev, dev_id, clk_id, hz);
}
