/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * Minimal polling TISCI client for cores without a secure-proxy RX
 * interrupt (the AM62P MCU R5F: no SEC_PROXY line in its VIM).  Speaks the
 * same messages as Zephyr's interrupt-driven TISCI driver, over the main
 * secure proxy threads assigned to the host in the TI board configuration.
 *
 * The same API is also implemented on top of Zephyr's TISCI driver
 * (lib/tisci_zephyr/tisci_zephyr.c, CONFIG_SAFEUI_TISCI_ZEPHYR) for cores
 * with a secure-proxy interrupt, so drivers using it run on the A53 too.
 */
#ifndef SAFEUI_TISCI_POLL_H_
#define SAFEUI_TISCI_POLL_H_

#include <zephyr/device.h>
#include <stdint.h>
#include <stdbool.h>

int safeui_tisci_version(const struct device *dev, char desc[32], uint16_t *revision,
			 uint8_t *abi_major, uint8_t *abi_minor);
int safeui_tisci_device_on(const struct device *dev, uint32_t dev_id, bool exclusive);
/* power on unless another host already holds it on (no request in that case) */
int safeui_tisci_device_ensure_on(const struct device *dev, uint32_t dev_id);
int safeui_tisci_device_off(const struct device *dev, uint32_t dev_id);
int safeui_tisci_device_is_on(const struct device *dev, uint32_t dev_id, bool *on);
int safeui_tisci_clock_set_state(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				 bool on);
int safeui_tisci_clock_set_parent(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				  uint8_t parent_id);
int safeui_tisci_clock_set_freq(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				uint64_t min_hz, uint64_t target_hz, uint64_t max_hz);
int safeui_tisci_clock_get_freq(const struct device *dev, uint32_t dev_id, uint8_t clk_id,
				uint64_t *hz);

#endif
