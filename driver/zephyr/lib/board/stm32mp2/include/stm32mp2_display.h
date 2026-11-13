/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * stm32mp2_display.h -- the STM32MP257F-DK display path's SoC glue (RIF
 * checks, SCMI clocks and HDMI 5 V, RCC gates and resets, the pixel-clock
 * mux, the ADV7535's and I2C2's pins), from the yakogl-drivers crate
 * (driver/rust/src/soc/stm32mp2/display_ffi.rs over display.rs; feature
 * stm32mp2, no vivante). Plan #353, issues #7 and #1281.
 *
 * Call order: yd_mp2_display_rif_check, yd_mp2_display_power_on (the GPIO
 * bank clocks are among its SCMI clocks), yd_mp2_display_pins, then
 * yd_mp2_display_adv_reset; then the IP drivers (LTDC, DSI, I2C2, ADV7535).
 *
 * Every SMC, OP-TEE message frame and SCMI request and reply is handed to
 * the log callback as one line ("optee smc ...", "optee msg > ...",
 * "scmi > ...", ...), the request before it is sent. The SCMI rules of
 * docs/notes/stm32mp257f-dk-display-bringup.md hold in the crate: no clock
 * is ever disabled, no voltage domain switched off, RATE_SET goes to
 * FLEXGEN_27 only, and every enable is read back (CLOCK_ATTRIBUTES,
 * RATE_GET, VOLTAGE_CONFIG_GET), not taken from the SCMI status. No RIF or
 * RISAF register is written: a secure or foreign resource is refused and
 * reported (err RIF or PIN) with nothing written.
 *
 * Return values: 0, or a negative errno (Zephyr's numbering) with the reason
 * in the result's `err`.
 */
#ifndef YAKOGL_STM32MP2_DISPLAY_H
#define YAKOGL_STM32MP2_DISPLAY_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The memory shared with OP-TEE: three 4 KiB pages, 4 KiB aligned, normal
 * cached memory, physically contiguous (the GPU's may be reused). */
#define YD_MP2_DISPLAY_SHM_SIZE  12288u
#define YD_MP2_DISPLAY_SHM_ALIGN 4096u

/* One log line, `len` bytes, not NUL-terminated. */
typedef void (*yd_mp2_display_log_fn)(void *ctx, const char *line, size_t len);
/* Wait at least `us` microseconds. */
typedef void (*yd_mp2_display_delay_fn)(void *ctx, uint32_t us);

struct yd_mp2_display {
	uintptr_t rcc_base;    /* the RCC, mapped (0x44200000) */
	uintptr_t syscfg_base; /* SYSCFG, mapped (0x44230000; DISPLAYCLKCR at +0x5000) */
	uintptr_t rifsc_base;  /* RIFSC, mapped (0x42080000), read only */
	uintptr_t gpiob_base;  /* GPIOB, mapped (0x44250000): PB4, PB6 */
	uintptr_t gpiof_base;  /* GPIOF, mapped (0x44290000): PF0, PF2 */
	void *shm;             /* YD_MP2_DISPLAY_SHM_SIZE bytes, see above */
	uint64_t shm_pa;       /* its physical address */
	yd_mp2_display_log_fn log; /* may be NULL */
	void *log_ctx;
	yd_mp2_display_delay_fn delay_us; /* required by power_on and adv_reset */
	void *delay_ctx;
	uint64_t ltdc_rate_hz; /* FLEXGEN_27: 74250000 (720p60), 148500000 (1080p60), 0 no RATE_SET */
	uint32_t channel;      /* the SCMI channel: 0, the only one power_on takes */
	uint32_t cid;          /* our compartment; 0: 1 (the A35's) */
};

/* err.kind */
#define YD_MP2_DISPLAY_ERR_NONE      0u
#define YD_MP2_DISPLAY_ERR_OPTEE     1u  /* step 1 open, 2 close; a: the code (b: the channel refused) */
#define YD_MP2_DISPLAY_ERR_TRANSPORT 2u  /* a: the transport's code */
#define YD_MP2_DISPLAY_ERR_REFUSED   3u  /* a: the SCMI status (negative) */
#define YD_MP2_DISPLAY_ERR_ECHO      4u  /* a: header sent, b: received */
#define YD_MP2_DISPLAY_ERR_LENGTH    5u  /* a: the reply's length */
#define YD_MP2_DISPLAY_ERR_READBACK  6u  /* a: wanted, b: read (mode, enable bit, Hz) */
#define YD_MP2_DISPLAY_ERR_FORBIDDEN 7u  /* a request the SCMI rules forbid; never sent */
#define YD_MP2_DISPLAY_ERR_RIF       8u  /* id: the RIFSC entry, a: YD_MP2_DISPLAY_RIF_* */
#define YD_MP2_DISPLAY_ERR_RCC       9u  /* id: the RCC offset, a: the register as read */
#define YD_MP2_DISPLAY_ERR_MUX       10u /* RCC_DSICFGR's muxes changed; a: before, b: after */
#define YD_MP2_DISPLAY_ERR_SYSCFG    11u /* a: DISPLAYCLKCR as read (not 2) */
#define YD_MP2_DISPLAY_ERR_PIN       12u /* id: bank << 8 | pin, a: 1 range, 2 secure, 3 foreign CID */
#define YD_MP2_DISPLAY_ERR_PIN_MODE  13u /* id: the pin, a: its bank's MODER (PB6 not an output) */
#define YD_MP2_DISPLAY_ERR_FB        14u /* a: YD_MP2_DISPLAY_FB_* */

/* err.step for YD_MP2_DISPLAY_ERR_OPTEE */
#define YD_MP2_DISPLAY_OPTEE_OPEN  1u
#define YD_MP2_DISPLAY_OPTEE_CLOSE 2u

/* err.step for the SCMI kinds; err.id the clock or voltage domain */
#define YD_MP2_DISPLAY_STEP_VOLTAGE_VERSION  1u
#define YD_MP2_DISPLAY_STEP_SUPPLY_GET       2u
#define YD_MP2_DISPLAY_STEP_HDMI_GET         3u
#define YD_MP2_DISPLAY_STEP_HDMI_SET         4u
#define YD_MP2_DISPLAY_STEP_HDMI_CHECK       5u
#define YD_MP2_DISPLAY_STEP_CLOCK_VERSION    6u
#define YD_MP2_DISPLAY_STEP_CLOCK_ENABLE     7u
#define YD_MP2_DISPLAY_STEP_CLOCK_RATE_SET   8u
#define YD_MP2_DISPLAY_STEP_CLOCK_ATTRIBUTES 9u
#define YD_MP2_DISPLAY_STEP_CLOCK_RATE_GET   10u

/* A RIF entry's verdict */
#define YD_MP2_DISPLAY_RIF_OK          0u
#define YD_MP2_DISPLAY_RIF_SECURE      1u
#define YD_MP2_DISPLAY_RIF_FOREIGN_CID 2u

/* The framebuffer verdict */
#define YD_MP2_DISPLAY_FB_OK           0u
#define YD_MP2_DISPLAY_FB_EMPTY        1u /* the end overflows */
#define YD_MP2_DISPLAY_FB_UNALIGNED    2u /* base not 4 KiB aligned */
#define YD_MP2_DISPLAY_FB_ABOVE_4G     3u /* LxCFBAR is 32 bits */
#define YD_MP2_DISPLAY_FB_IMAGE        4u /* overlaps the image, 0xa0000000-0xafffffff */
#define YD_MP2_DISPLAY_FB_SECURE_LAYER 5u /* overlaps 0xfe800000-0xffffffff */
#define YD_MP2_DISPLAY_FB_NOT_ALLOWED  6u /* not in RISAF4 regions 9, 10 (0x84000000-0xfe7fffff) */
#define YD_MP2_DISPLAY_FB_SKIPPED      7u /* fb_size 0: not checked */

struct yd_mp2_display_err {
	uint32_t kind;
	uint32_t step;
	uint32_t id;
	uint32_t a;
	uint32_t b;
};

struct yd_mp2_display_power {
	uint32_t voltage_version;
	uint32_t clock_version;
	uint32_t supplies_on;  /* bit 0 vddcore (11), 1 v1v8 (14), 2 v3v3 (16) read on */
	uint32_t hdmi_before;  /* LDO5 (21) CONFIG_GET mode before the set: 0 off, 7 on */
	uint32_t hdmi;         /* and after it */
	uint32_t rate_hz[5];   /* RATE_GET: GPIOB, GPIOF, FLEXGEN_12, FLEXGEN_28, FLEXGEN_27 */
	uint32_t rcc_before[3]; /* RCC_I2C2CFGR, RCC_DSICFGR, RCC_LTDCCFGR as found */
	uint32_t rcc[3];       /* and after the gate and the reset pulse */
	uint32_t displayclkcr; /* SYSCFG DISPLAYCLKCR after the write (bits 1:0 = 2) */
	struct yd_mp2_display_err err;
};

struct yd_mp2_display_pins {
	uint32_t gpiob_moder;
	uint32_t gpiof_moder;
	uint32_t gpiof_afrl;
	uint32_t adv_int;      /* PB4's level (the ADV7535's interrupt, active low) */
	struct yd_mp2_display_err err;
};

struct yd_mp2_display_rif_entry {
	uint32_t id;
	uint32_t secure;
	uint32_t privileged;   /* reported only: the image runs privileged */
	uint32_t cidcfgr;
	uint32_t semcr;        /* read only when CIDCFGR.SEMEN is set */
	uint32_t verdict;      /* YD_MP2_DISPLAY_RIF_* */
};

struct yd_mp2_display_rif {
	/* required: LTDC 80, 119, DSI 81, I2C2 42; reported: LTDC 120, 121, DSI 122, 123 */
	struct yd_mp2_display_rif_entry entries[8];
	uint32_t refused;      /* bit i: entries[i] is not ours */
	uint32_t rimc[3];      /* RIFSC_RIMC_ATTRx of RIMU 11, 12, 13, raw */
	uint32_t fb_region;    /* RISAF4 region of the framebuffer's base (9, 10), 0 if refused */
	uint32_t fb_verdict;   /* YD_MP2_DISPLAY_FB_* */
	struct yd_mp2_display_err err;
};

_Static_assert(sizeof(struct yd_mp2_display) == 104, "struct yd_mp2_display is the crate's YdMp2Display");
_Static_assert(sizeof(struct yd_mp2_display_err) == 20, "struct yd_mp2_display_err is the crate's");
_Static_assert(sizeof(struct yd_mp2_display_power) == 88, "struct yd_mp2_display_power is the crate's");
_Static_assert(sizeof(struct yd_mp2_display_pins) == 36, "struct yd_mp2_display_pins is the crate's");
_Static_assert(sizeof(struct yd_mp2_display_rif_entry) == 24, "struct yd_mp2_display_rif_entry is the crate's");
_Static_assert(sizeof(struct yd_mp2_display_rif) == 236, "struct yd_mp2_display_rif is the crate's");

/* OP-TEE opened (capabilities, the shared page registered, a session to the
 * SCMI pseudo-TA, the channel), then: the RIFSC entries guarding the RCC
 * registers (80, 81, 42) read; VOLTAGE version, CONFIG_GET of 11, 14, 16
 * (recorded, never set), LDO5 (21) CONFIG_GET, CONFIG_SET on, CONFIG_GET
 * back; CLOCK version, then per clock GPIOB 90, GPIOF 94, FLEXGEN_12,
 * FLEXGEN_28, FLEXGEN_27: CONFIG_SET enable, (27 only) RATE_SET
 * ltdc_rate_hz, CLOCK_ATTRIBUTES and RATE_GET back (12 must read 100 MHz,
 * 28 27 MHz, 27 the rate set); RCC I2C2, DSI, LTDC: gate on, reset
 * asserted, held 20 us, released, read back (DSI's muxes kept); SYSCFG
 * DISPLAYCLKCR = 2, read back. The session is closed after, also on a
 * failure. A failed step stops at once; nothing after it is sent or
 * written. -EIO (err says why), -EINVAL (a NULL argument, no delay
 * callback, or a channel other than 0, before any SMC). */
int32_t yd_mp2_display_power_on(const struct yd_mp2_display *d, struct yd_mp2_display_power *out);

/* PF2 (SCL), PF0 (SDA) to AF9, open drain, no bias, low speed; PB4 an input
 * without bias; PB6 an output driven high (ADV7535 out of reset). Every pin's
 * RIF registers are checked first: a secure or foreign pin is -EIO (err
 * PIN) with nothing written. Needs power_on (the bank clocks). */
int32_t yd_mp2_display_pins(const struct yd_mp2_display *d, struct yd_mp2_display_pins *out);

/* The ADV7535's reset pulse: PB6 low 1 ms, high, wait 10 ms (ST U-Boot's
 * EV1 code). -EIO when PB6 is refused or not an output (call pins first),
 * -EINVAL without a delay callback. */
int32_t yd_mp2_display_adv_reset(const struct yd_mp2_display *d, struct yd_mp2_display_pins *out);

/* Read only: the RIFSC entries of the LTDC, DSI and I2C2 and RIMU 11 to 13
 * for d->cid, and, when fb_size is not 0, the framebuffer range
 * [fb_base, fb_base + fb_size) against the DK's RISAF4 table. 0, -EIO (a
 * required entry is not ours; err RIF), -ERANGE (the range is refused;
 * err FB). */
int32_t yd_mp2_display_rif_check(const struct yd_mp2_display *d, uint64_t fb_base, uint64_t fb_size,
				 struct yd_mp2_display_rif *out);

#ifdef __cplusplus
}
#endif

#endif /* YAKOGL_STM32MP2_DISPLAY_H */
