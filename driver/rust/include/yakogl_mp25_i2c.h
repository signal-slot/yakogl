/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * yakogl_mp25_i2c.h -- the C view of ffi::mp25_i2c (driver/rust/src/ffi/
 * mp25_i2c.rs): the STM32MP2 I2C controller, a polled master over
 * ip::stm32_i2c, for driver/drivers/i2c/. Compatible yakogl,stm32mp2-i2c.
 *
 * Conventions as yakogl_drivers.h: `cfg` points into dev->config, `data` into
 * dev->data, `env` at the shim's struct yd_env (log only). Errors are negative
 * errno (-ENODEV for a NACK). The shim serialises calls. The clock gate, the
 * reset, the pins and the RIF setting of the controller are the caller's.
 * LP64 only (aarch64).
 */
#ifndef YAKOGL_MP25_I2C_H
#define YAKOGL_MP25_I2C_H

#include "yakogl_drivers.h"

#ifdef __cplusplus
extern "C" {
#endif

#define YD_MP25_I2C_ANALOG_FILTER (1u << 0) /* i2c-analog-filter: CR1.ANFOFF clear */

/* The devicetree node. rise_ns / fall_ns 0 take Linux's 25 / 10 ns. */
struct yd_mp25_i2c_config {
	uintptr_t base;            /* device_map() of the registers */
	uint32_t kernel_clock_hz;  /* kernel-clock-frequency */
	uint32_t speed_hz;         /* clock-frequency: 100000, 400000, 1000000 */
	uint32_t rise_ns;          /* i2c-scl-rising-time-ns */
	uint32_t fall_ns;          /* i2c-scl-falling-time-ns */
	uint32_t dnf_ns;           /* i2c-digital-filter-width-ns, 0 = none */
	uint32_t flags;            /* YD_MP25_I2C_* */
};

/* Left by yd_mp25_i2c_init(). */
struct yd_mp25_i2c_data {
	uint32_t timingr;          /* TIMINGR written */
	uint32_t dnf;              /* CR1.DNF written */
};

/* Zephyr's struct i2c_msg on LP64; flags are Zephyr's I2C_MSG_*. */
#define YD_MP25_I2C_MSG_WRITE   0u
#define YD_MP25_I2C_MSG_READ    1u
#define YD_MP25_I2C_MSG_STOP    2u
#define YD_MP25_I2C_MSG_RESTART 4u

struct yd_mp25_i2c_msg {
	uint8_t *buf;
	uint32_t len;
	uint8_t flags;
};

/* PE low, TIMINGR, filters, PE high. 0, or -EINVAL (no timing fits). */
int yd_mp25_i2c_init(const struct yd_mp25_i2c_config *cfg, struct yd_mp25_i2c_data *data,
		     const struct yd_env *env);

/* START, 1..=255 bytes to the 7-bit addr, STOP. */
int yd_mp25_i2c_write(const struct yd_mp25_i2c_config *cfg, uint16_t addr, const uint8_t *buf,
		      size_t len);

/* wlen bytes, repeated START, rlen bytes, STOP; each 1..=255. */
int yd_mp25_i2c_write_read(const struct yd_mp25_i2c_config *cfg, uint16_t addr,
			   const uint8_t *wbuf, size_t wlen, uint8_t *rbuf, size_t rlen);

/* Zephyr's i2c_transfer list: one write, or a write then a read. Any other
 * list (a read alone, 3+ messages): -ENOTSUP. */
int yd_mp25_i2c_transfer(const struct yd_mp25_i2c_config *cfg, struct yd_mp25_i2c_msg *msgs,
			 uint8_t num_msgs, uint16_t addr);

_Static_assert(sizeof(struct yd_mp25_i2c_config) == 32, "yd_mp25_i2c_config");
_Static_assert(offsetof(struct yd_mp25_i2c_config, base) == 0, "base");
_Static_assert(offsetof(struct yd_mp25_i2c_config, kernel_clock_hz) == 8, "kernel_clock_hz");
_Static_assert(offsetof(struct yd_mp25_i2c_config, speed_hz) == 12, "speed_hz");
_Static_assert(offsetof(struct yd_mp25_i2c_config, rise_ns) == 16, "rise_ns");
_Static_assert(offsetof(struct yd_mp25_i2c_config, fall_ns) == 20, "fall_ns");
_Static_assert(offsetof(struct yd_mp25_i2c_config, dnf_ns) == 24, "dnf_ns");
_Static_assert(offsetof(struct yd_mp25_i2c_config, flags) == 28, "flags");
_Static_assert(sizeof(struct yd_mp25_i2c_data) == 8, "yd_mp25_i2c_data");
_Static_assert(offsetof(struct yd_mp25_i2c_data, timingr) == 0, "timingr");
_Static_assert(offsetof(struct yd_mp25_i2c_data, dnf) == 4, "dnf");
_Static_assert(sizeof(struct yd_mp25_i2c_msg) == 16, "yd_mp25_i2c_msg");
_Static_assert(offsetof(struct yd_mp25_i2c_msg, buf) == 0, "buf");
_Static_assert(offsetof(struct yd_mp25_i2c_msg, len) == 8, "len");
_Static_assert(offsetof(struct yd_mp25_i2c_msg, flags) == 12, "msg flags");

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_MP25_I2C_H */
