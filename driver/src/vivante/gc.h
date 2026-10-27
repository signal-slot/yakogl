/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Vivante GC7000 (i.MX8MP) GPU driver for Zephyr: the device interface the
 * later milestones (front end, MMU, render service) and the samples use.
 */
#ifndef GC_H
#define GC_H

#include <stdint.h>
#include <stdbool.h>
#include <zephyr/device.h>

/* What the identity registers report (rnndb state_hi.xml.h CHIP_*), with the
 * part's capabilities and limits from the hardware database. The layout is
 * pinned: driver/drivers/gpu/gpu_vivante_gc_rust.c and the Rust GcIdentity
 * (driver/rust/src/gpu/vivante) assert it. */
struct gc_identity {
	uint32_t model;        /* CHIP_MODEL: 0x7000 on the i.MX8MP */
	uint32_t revision;     /* CHIP_REV:   0x6204 */
	uint32_t product_id;   /* CHIP_PRODUCT_ID */
	uint32_t customer_id;  /* CHIP_CUSTOMER_ID */
	uint32_t eco_id;       /* CHIP_ECO_ID */
	uint32_t date;         /* CHIP_DATE, CHIP_TIME: the RTL build stamp */
	uint32_t time;
	uint32_t features;     /* CHIP_FEATURE */
	uint32_t minor_features[12]; /* CHIP_MINOR_FEATURE_0..11 */
	bool secure_ahb;       /* BIT_SECURITY + SECURITY_AHB: the kernel owns the
				* secure state (reset and FE start go through the
				* MMUv2 secure registers: galcore's "secure in
				* normal" mode) */
	bool from_table;       /* the words above came from the built-in hardware
				* database, not from registers (see
				* driver/rust/src/gpu/vivante/hw.rs) */

	/* Hardware limits. Registers on some parts, the table on this one. */
	uint32_t stream_count;              /* vertex streams */
	uint32_t register_max;              /* temporary registers per shader */
	uint32_t thread_count;
	uint32_t shader_core_count;
	uint32_t vertex_cache_size;
	uint32_t vertex_output_buffer_size;
	uint32_t pixel_pipes;
	uint32_t instruction_count;         /* instruction slots per shader */
	uint32_t num_constants;             /* the constant (uniform) register file */
	uint32_t varyings_count;
};

/* The device's identity, valid once the driver initialised (gc_alive()). */
const struct gc_identity *gc_identity(const struct device *dev);

/* True when the GPU was powered, identified as a GC7000 and reset to idle. */
bool gc_alive(const struct device *dev);

/*
 * Bring the GPU up without a Zephyr device: read its identity and reset it to
 * idle, against a register block that is **already powered and clocked**.
 *
 * gc_init() does the whole i.MX8MP sequence -- GPC power domains, CCM roots and
 * gates, then identity and reset -- because under Zephyr this driver owns the
 * SoC. Under Linux those same GPC and CCM registers belong to the kernel's
 * genpd and clk drivers, and a second writer of them is the kind of bug that
 * shows up as an unrelated peripheral misbehaving. So the Linux module
 * (tools/gclab) powers and clocks the GPU the kernel's way and then calls this,
 * which is the rest of gc_init() and nothing else: the same gc_identify() and
 * gc_reset() the Zephyr path runs.
 *
 * `regs` is the mapped register block. On success the identity is stored and
 * `*id_out`, if given, points at it (owned by the driver, valid until the next
 * call). Returns 0, -ENODEV if the GPU does not answer, or what gc_reset()
 * reported.
 */
int gc_bringup_attach(mm_reg_t regs, const struct gc_identity **id_out);

/* Raw register access for the bring-up samples and the later milestones. */
uint32_t gc_read(const struct device *dev, uint32_t reg);
void gc_write(const struct device *dev, uint32_t reg, uint32_t val);

#endif
