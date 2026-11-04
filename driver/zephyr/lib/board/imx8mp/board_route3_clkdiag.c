/* SPDX-License-Identifier: Apache-2.0 */
/*
 * Measurement only (CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_CLOCK_REPORT): what the
 * A53 actually runs at, read back and changed in nothing, for the timeline
 * UART after the cold-start report.
 *
 *   A53CLK cpu_khz <measured> | armpll gen <GEN_CTRL> fdiv0 <FDIV_CTL0>
 *          | a53root <CCM TARGET_ROOT0> coresel <CCM 0x9880>
 *   A53PMIC rc <0|-errno> 10..1f <PCA9450 registers 0x10-0x1f>
 *
 * cpu_khz: the PMU cycle counter against the system counter over 20 ms on
 * this core. The ARM PLL registers are CCM_ANALOG + 0x84 / + 0x88 (IMX8MPRM
 * Rev. 3, CCM_ANALOG "ARM PLL"); TARGET_ROOT0 is ARM_A53_CLK_ROOT. The PMIC
 * is the PCA9450 at 0x25 on I2C1 (Toradex Verdin iMX8M Plus); its BUCK2 is
 * VDD_ARM. I2C1 is driven by hand, polled (IMX8MPRM §17.1.6, 16-bit
 * registers), as the SPL left its clock and pads; nothing in this image uses
 * I2C1.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <zephyr/kernel/mm.h>
#include <errno.h>
#include <stdio.h>
#include "board_imx8mp_route3_clkdiag.h"
#if CONFIG_YAKOGL_BOARD_IMX8MP_A53_MHZ > 0
#include "board_imx8mp_a53_clock.h"
#endif

#define CCM_ANALOG 0x30360000u
#define CCM        0x30380000u
#define I2C1       0x30A20000u
#define PMIC_ADDR  0x25u

static mm_reg_t map(uintptr_t pa, size_t len)
{
	mm_reg_t va;

	device_map(&va, pa, len, K_MEM_CACHE_NONE);
	return va;
}

static uint64_t cntvct(void)
{
	uint64_t v;

	__asm__ volatile("isb; mrs %0, cntvct_el0" : "=r"(v));
	return v;
}

static uint64_t cntfrq(void)
{
	uint64_t v;

	__asm__ volatile("mrs %0, cntfrq_el0" : "=r"(v));
	return v;
}

static uint32_t cpu_khz(void)
{
	uint64_t pmcr, c0, c1, t0, t1;
	unsigned int key = irq_lock();

	__asm__ volatile("mrs %0, pmcr_el0" : "=r"(pmcr));
	__asm__ volatile("msr pmcr_el0, %0" ::"r"(pmcr | 1u)); /* E */
	__asm__ volatile("msr pmcntenset_el0, %0" ::"r"(1ull << 31)); /* PMCCNTR */
	__asm__ volatile("isb; mrs %0, pmccntr_el0" : "=r"(c0));
	t0 = cntvct();
	do {
		t1 = cntvct();
	} while (t1 - t0 < cntfrq() / 50u); /* 20 ms */
	__asm__ volatile("isb; mrs %0, pmccntr_el0" : "=r"(c1));
	irq_unlock(key);
	return (uint32_t)((c1 - c0) * (cntfrq() / 1000u) / (t1 - t0));
}

/* I2C: IFDR 0x04, I2CR 0x08, I2SR 0x0C, I2DR 0x10 */
static int i2_wait(mm_reg_t b, uint16_t mask, uint16_t want)
{
	for (int i = 0; i < 2000; i++) {
		if ((sys_read16(b + 0x0Cu) & mask) == want) {
			return 0;
		}
		k_busy_wait(10);
	}
	return -ETIMEDOUT;
}

static int i2_byte(mm_reg_t b, uint8_t v, bool need_ack)
{
	sys_write16(0u, b + 0x0Cu);
	sys_write16(v, b + 0x10u);
	if (i2_wait(b, 0x02u, 0x02u)) {
		return -ETIMEDOUT;
	}
	if ((sys_read16(b + 0x0Cu) & 0x10u) != 0u) {
		return -EAGAIN; /* arbitration lost */
	}
	if (need_ack && (sys_read16(b + 0x0Cu) & 0x01u) != 0u) {
		return -ENXIO;
	}
	return 0;
}

static int pmic_read(mm_reg_t b, uint8_t reg, uint8_t *val)
{
	int rc;

	sys_write16(0x0Fu, b + 0x04u); /* 24 MHz / 240: 100 kHz */
	sys_write16(0x80u, b + 0x08u); /* IEN */
	k_busy_wait(50);
	sys_write16(0u, b + 0x0Cu);
	if (i2_wait(b, 0x20u, 0x00u)) {
		return -EBUSY;
	}
	sys_write16(0xB0u, b + 0x08u); /* IEN MSTA MTX: start */
	if (i2_wait(b, 0x20u, 0x20u)) {
		rc = -EIO;
		goto stop;
	}
	rc = i2_byte(b, PMIC_ADDR << 1, true);
	if (rc == 0) {
		rc = i2_byte(b, reg, true);
	}
	if (rc == 0) {
		sys_write16(0xB4u, b + 0x08u); /* + RSTA */
		rc = i2_byte(b, (PMIC_ADDR << 1) | 1u, true);
	}
	if (rc == 0) {
		sys_write16(0xA8u, b + 0x08u); /* receive, TXAK: one byte */
		sys_write16(0u, b + 0x0Cu);
		(void)sys_read16(b + 0x10u);   /* dummy read starts it */
		if (i2_wait(b, 0x02u, 0x02u)) {
			rc = -ETIMEDOUT;
		} else {
			sys_write16(0x88u, b + 0x08u); /* MSTA 0: stop */
			*val = (uint8_t)sys_read16(b + 0x10u);
			(void)i2_wait(b, 0x20u, 0x00u);
			return 0;
		}
	}
stop:
	sys_write16(0x80u, b + 0x08u);
	(void)i2_wait(b, 0x20u, 0x00u);
	return rc;
}

void board_route3_clock_report(void (*out)(const char *line))
{
	char buf[200];
	const mm_reg_t an = map(CCM_ANALOG, 0x1000u);
	const mm_reg_t ccm = map(CCM, 0x10000u);
	const mm_reg_t i2 = map(I2C1, 0x1000u);
	uint8_t r[16] = {0};
	int rc = 0;
	int n;

	snprintf(buf, sizeof(buf),
		 "A53CLK cpu_khz %u set %d | armpll gen %08x fdiv0 %08x | a53root %08x coresel %08x\n",
		 (unsigned int)cpu_khz(),
#if CONFIG_YAKOGL_BOARD_IMX8MP_A53_MHZ > 0
		 board_imx8mp_a53_clock_result(),
#else
		 1,
#endif
		 sys_read32(an + 0x84u), sys_read32(an + 0x88u),
		 sys_read32(ccm + 0x8000u), sys_read32(ccm + 0x9880u));
	out(buf);
	for (unsigned int i = 0; i < 16u && rc == 0; i++) {
		rc = pmic_read(i2, (uint8_t)(0x10u + i), &r[i]);
	}
	n = snprintf(buf, sizeof(buf), "A53PMIC rc %d 10..1f", rc);
	for (unsigned int i = 0; i < 16u && n > 0 && n < (int)sizeof(buf) - 4; i++) {
		n += snprintf(buf + n, sizeof(buf) - n, " %02x", r[i]);
	}
	snprintf(buf + n, sizeof(buf) - n, "\n");
	out(buf);
}
