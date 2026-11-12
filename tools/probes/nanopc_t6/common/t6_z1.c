/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Z1's record (t6_z1.h): the lines t6_gpu has printed since Z1, with the
 * probe's name as the prefix, for every probe that runs yd_t6_gpu_up's
 * power-on (driver/docs/nanopc-t6-mali.md, "The GPU power-on").
 * driver/tools/kexec-zephyr/t6-verdict.sh judges them; the host test
 * common/test/t6_z1_test.c prints a stop record from this file.
 */
#include "t6_z1.h"

#include <zephyr/sys/printk.h>

#include <stddef.h>

/* driver/rust/src/ffi/rk3588.rs pins the same layout. */
_Static_assert(sizeof(struct yd_t6_gpu_up) == 4 * 125, "struct yd_t6_gpu_up moved");
_Static_assert(offsetof(struct yd_t6_gpu_up, rk806) == 4 * 18, "struct yd_t6_gpu_up moved");
_Static_assert(offsetof(struct yd_t6_gpu_up, ident) == 4 * 65, "struct yd_t6_gpu_up moved");
_Static_assert(offsetof(struct yd_t6_gpu_up, stop) == 4 * 108, "struct yd_t6_gpu_up moved");
_Static_assert(offsetof(struct yd_t6_gpu_up, clock_kept) == 4 * 112, "struct yd_t6_gpu_up moved");
_Static_assert(offsetof(struct yd_t6_gpu_up, supply_waited_us) == 4 * 124,
	       "struct yd_t6_gpu_up moved");

static const char *const stop_name[] = {
	[YD_T6_GPU_STOP_NONE] = "none",
	[YD_T6_GPU_STOP_SPI_GATE] = "an SPI2 gate stays closed",
	[YD_T6_GPU_STOP_RK806_READ] = "an RK806 read failed",
	[YD_T6_GPU_STOP_SUPPLY_OFF] = "vdd_gpu_s0 (RK806 DCDC1) is off while PD_GPU reads powered",
	[YD_T6_GPU_STOP_CLOCK] = "the GPU clock has no rate, the supply is below its OPP or its PLL "
				 "did not lock",
	[YD_T6_GPU_STOP_PD] = "PD_GPU did not come on",
	[YD_T6_GPU_STOP_NOT_G610] = "GPU_ID is not 0xa8670005",
	[YD_T6_GPU_STOP_RESET] = "the soft reset did not complete",
	[YD_T6_GPU_STOP_L2] = "the L2 did not come up",
	[YD_T6_GPU_STOP_CLOCK_SET_REFUSED] = "the GPU clock is on no OPP rate and the CPLL is not "
					     "1500 MHz (nothing written)",
	[YD_T6_GPU_STOP_CLOCK_READ_BACK] = "the clock set did not read back",
	[YD_T6_GPU_STOP_SUPPLY_BELOW_OPP] = "BUCK1_ON_VSEL is below the OPP voltage for the clock",
	[YD_T6_GPU_STOP_SUPPLY_RAMP_TIMEOUT] = "DCDC1 did not read on after the enable frame",
	[YD_T6_GPU_STOP_RK806_WRITE] = "the DCDC1-enable frame could not be sent",
};

const char *t6_z1_stop_name(uint32_t stop)
{
	return stop < sizeof(stop_name) / sizeof(stop_name[0]) && stop_name[stop] != NULL
		       ? stop_name[stop]
		       : "unknown";
}

/* How far the run got when it stopped at `s`, in the order of the steps (the
 * numbers of the causes are in the order they were added, so 9..13 cannot be
 * compared with 5..8): 5 and more means PD_GPU was tried, 6 the GPU_ID was
 * read, 8 the reset ran. Stops 1..4, 9 and 10 (and 11 with refusal 0) come
 * before PD_GPU and before on_handover; the others are after on_handover. */
static unsigned int progress(uint32_t s)
{
	switch (s) {
	case YD_T6_GPU_STOP_PD:
		return 5;
	case YD_T6_GPU_STOP_NOT_G610:
		return 6;
	case YD_T6_GPU_STOP_RESET:
		return 7;
	case YD_T6_GPU_STOP_L2:
		return 8;
	case YD_T6_GPU_STOP_NONE:
		return 100;
	default:
		return 0;
	}
}

static void print_pd(const char *probe, const char *when, const uint32_t r[6])
{
	printk("%s: PD_GPU %s: PWR_GATE 0x%08x STATUS 0x%08x BUS_IDLE_REQ 0x%08x "
	       "ACK 0x%08x ST 0x%08x REPAIR_STATUS 0x%08x: %s\n",
	       probe, when, r[0], r[1], r[2], r[3], r[4], r[5],
	       (r[5] & 0x2) && !((r[2] | r[3] | r[4]) & 0x1) ? "on" : "off or idle");
}

static void print_ident(const char *probe, const char *when, const uint32_t i[16])
{
	printk("%s: GPU %s: GPU_ID 0x%08x L2_FEATURES 0x%08x TILER 0x%08x MEM 0x%08x MMU 0x%08x "
	       "AS_PRESENT 0x%08x CSF_ID 0x%08x\n",
	       probe, when, i[0], i[1], i[2], i[3], i[4], i[5], i[6]);
	printk("%s: GPU %s: SHADER_PRESENT 0x%08x%08x TILER_PRESENT 0x%08x%08x L2_PRESENT "
	       "0x%08x%08x COHERENCY_FEATURES 0x%08x GPU_STATUS 0x%08x MCU_STATUS 0x%08x\n",
	       probe, when, i[8], i[7], i[10], i[9], i[12], i[11], i[13], i[14], i[15]);
}

void t6_z1_print_handover(const char *probe, const struct yd_t6_gpu_up *h)
{
	print_pd(probe, "at hand-over", h->pd_before);
	printk("%s: CRU at hand-over: CLKSEL_CON(158) 0x%08x CLKSEL_CON(159) 0x%08x "
	       "CLKGATE_CON(66) 0x%08x MODE_CON0 0x%08x PLL_CON 0x%08x 0x%08x 0x%08x: "
	       "clk_gpu_src %u Hz\n",
	       probe, h->clock[0], h->clock[1], h->clock[2], h->clock[3], h->clock[4],
	       h->clock[5], h->clock[6], h->gpu_src_hz[0]);
	printk("%s: SPI2 gates CLKGATE_CON(14) 0x%08x, opened 0x%x\n", probe, h->spi_gate14,
	       h->spi_opened);
	printk("%s: SPI2 as found: CTRLR0 0x%08x CTRLR1 0x%08x SSIENR 0x%x SER 0x%x BAUDR 0x%x "
	       "SR 0x%08x RXFLR 0x%x VERSION 0x%08x\n",
	       probe, h->spi_found[0], h->spi_found[1], h->spi_found[2], h->spi_found[3],
	       h->spi_found[4], h->spi_found[5], h->spi_found[6], h->spi_found[7]);
	printk("%s: RK806 at hand-over: POWER_EN0 0x%02x POWER_SLP_EN0 0x%02x BUCK1_CONFIG "
	       "0x%02x BUCK1_ON_VSEL 0x%02x: vdd_gpu_s0 %s, %u uV\n",
	       probe, h->rk806[0], h->rk806[1], h->rk806[2], h->rk806[3],
	       (h->rk806[0] & 0x1) ? "on" : "off", h->vdd_gpu_uv);
}

void t6_z1_print_record(const char *probe, const struct yd_t6_gpu_up *o, bool handover_printed)
{
	/* Stops 1..4 and the causes added after them (9, 10, and 11 with refusal 0) come
	 * before the hand-over report, so the test is whether on_handover printed, not a
	 * comparison of stop numbers. A stop before the clock set has the clock as found
	 * and the supply as read so far. */
	if (!handover_printed) {
		t6_z1_print_handover(probe, o);
	}
	printk("%s: clock set: kept %u, CPLL %u Hz, wrote CLKSEL_CON(158) 0x%08x (159) 0x%08x, "
	       "now 0x%08x 0x%08x: clk_gpu_src %u Hz\n",
	       probe, o->clock_kept, o->clock_cpll_hz[0], o->clock_wrote[0], o->clock_wrote[1],
	       o->clock_sel[0], o->clock_sel[1], o->clock_set_hz[0]);
	printk("%s: vdd_gpu_s0 enable: %s; POWER_EN0 0x%02x BUCK1_ON_VSEL 0x%02x after, waited "
	       "%u us\n",
	       probe, o->supply_enabled ? "frame sent" : "not sent", o->supply_after[0],
	       o->supply_after[1], o->supply_waited_us);
	/* The PLL's power-on ran on every stop from 5 on; a PLL that did not lock is a
	 * stop of its own (its PLL_CON6 after the wait is the detail, pll_on stays 0). */
	if (progress(o->stop) >= 5 ||
	    (o->stop == YD_T6_GPU_STOP_CLOCK && o->refusal == YD_T6_GPU_CLOCK_PLL_NO_LOCK)) {
		printk("%s: GPU PLL on: PLL_CON6 0x%08x at hand-over, was on %u, waited %u us, "
		       "PLL_CON6 0x%08x\n",
		       probe, o->clock[7], o->pll_on[0], o->pll_on[1], o->pll_on[2]);
	}
	if (progress(o->stop) >= 5) {
		printk("%s: PD_GPU on: gates opened 0x%x, was on %u, waited %u/%u/%u us\n", probe,
		       o->pd_opened, o->pd_was_on, o->pd_waited_us[0], o->pd_waited_us[1],
		       o->pd_waited_us[2]);
		print_pd(probe, "after", o->pd_after);
	}
	if (progress(o->stop) >= 6) {
		for (int b = 0; b < 4; b++) {
			printk("%s: qos_gpu_m%d: 0x%08x 0x%08x 0x%08x 0x%08x 0x%08x\n", probe, b,
			       o->qos[b][0], o->qos[b][1], o->qos[b][2], o->qos[b][3],
			       o->qos[b][4]);
		}
		print_ident(probe, "after power-on", o->ident);
	}
	if (progress(o->stop) >= 8) {
		printk("%s: soft reset: GPU_INT_RAWSTAT 0x%08x before the clear, 0x%08x after; "
		       "RESET_COMPLETED after %u polls of 1 us\n",
		       probe, o->reset[0], o->reset[1], o->reset[2]);
		print_ident(probe, "after reset", o->ident_after_reset);
	}
	if (o->stop == YD_T6_GPU_STOP_NONE) {
		printk("%s: L2 mask 0x%x: L2_READY 0x%x before L2_PWRON, 0x%x after; waited %u/%u "
		       "polls\n",
		       probe, o->l2_mask[0], o->l2_ready_before[0], o->l2_ready[0],
		       o->l2_waited_us[0], o->l2_waited_us[1]);
	}
}

void t6_z1_print_stop(const char *probe, const struct yd_t6_gpu_up *o)
{
	t6_z1_print_record(probe, o, false);
	printk("%s: Z1 stopped: stop %u (%s), refusal %u, detail 0x%08x%08x\n", probe, o->stop,
	       t6_z1_stop_name(o->stop), o->refusal, o->detail[1], o->detail[0]);
}
