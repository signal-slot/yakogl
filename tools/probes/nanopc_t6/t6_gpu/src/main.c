/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_gpu: the second half of Z1 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md).
 * Before main() the board glue has armed WDT0 (nanopc_t6.c). This prints
 * the build, then yd_t6_gpu_up (board::nanopc_t6_gpu in the drivers crate)
 * reads what the kexec from Linux handed over -- PD_GPU in the PMU, the
 * GPU's clock in the CRU, vdd_gpu_s0 in the RK806 over SPI2, read only --
 * and this prints it before anything is written; then PD_GPU on, the GPU's
 * identity, a soft reset and the L2 on. The last line is PASS or FAIL with
 * the step that stopped; the board then resets back into Armbian.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/kernel/mm.h>

#include <stddef.h>

#include <nanopc_t6.h>
#include <yakogl_drivers.h>

#define CRU_PHYS  0xfd7c0000UL
#define PMU_PHYS  0xfd8d8000UL
#define SPI2_PHYS 0xfeb20000UL
#define QOS_PHYS  0xfdf35000UL
#define GPU_PHYS  0xfb000000UL

/* driver/rust/src/ffi/rk3588.rs pins the same layout. */
BUILD_ASSERT(sizeof(struct yd_t6_gpu_up) == 4 * 108, "struct yd_t6_gpu_up moved");
BUILD_ASSERT(offsetof(struct yd_t6_gpu_up, rk806) == 4 * 17, "struct yd_t6_gpu_up moved");
BUILD_ASSERT(offsetof(struct yd_t6_gpu_up, ident) == 4 * 61, "struct yd_t6_gpu_up moved");
BUILD_ASSERT(offsetof(struct yd_t6_gpu_up, stop) == 4 * 104, "struct yd_t6_gpu_up moved");

static const char *const stop_name[] = {
	[YD_T6_GPU_STOP_NONE] = "none",
	[YD_T6_GPU_STOP_SPI_GATE] = "an SPI2 gate stays closed",
	[YD_T6_GPU_STOP_RK806_READ] = "an RK806 read failed",
	[YD_T6_GPU_STOP_SUPPLY_OFF] = "vdd_gpu_s0 (RK806 DCDC1) is off",
	[YD_T6_GPU_STOP_CLOCK] = "the GPU clock has no rate or the supply is below its OPP",
	[YD_T6_GPU_STOP_PD] = "PD_GPU did not come on",
	[YD_T6_GPU_STOP_NOT_G610] = "GPU_ID is not 0xa8670005",
	[YD_T6_GPU_STOP_RESET] = "the soft reset did not complete",
	[YD_T6_GPU_STOP_L2] = "the L2 did not come up",
};

static void delay_us(uint32_t us)
{
	k_busy_wait(us);
}

static void print_pd(const char *when, const uint32_t r[6])
{
	printk("t6_gpu: PD_GPU %s: PWR_GATE 0x%08x STATUS 0x%08x BUS_IDLE_REQ 0x%08x "
	       "ACK 0x%08x ST 0x%08x REPAIR_STATUS 0x%08x: %s\n",
	       when, r[0], r[1], r[2], r[3], r[4], r[5],
	       (r[5] & 0x2) && !((r[2] | r[3] | r[4]) & 0x1) ? "on" : "off or idle");
}

static void print_ident(const char *when, const uint32_t i[16])
{
	printk("t6_gpu: GPU %s: GPU_ID 0x%08x L2_FEATURES 0x%08x TILER 0x%08x MEM 0x%08x MMU 0x%08x "
	       "AS_PRESENT 0x%08x CSF_ID 0x%08x\n",
	       when, i[0], i[1], i[2], i[3], i[4], i[5], i[6]);
	printk("t6_gpu: GPU %s: SHADER_PRESENT 0x%08x%08x TILER_PRESENT 0x%08x%08x L2_PRESENT "
	       "0x%08x%08x COHERENCY_FEATURES 0x%08x GPU_STATUS 0x%08x MCU_STATUS 0x%08x\n",
	       when, i[8], i[7], i[10], i[9], i[12], i[11], i[13], i[14], i[15]);
}

static void on_handover(const struct yd_t6_gpu_up *h)
{
	print_pd("at hand-over", h->pd_before);
	printk("t6_gpu: CRU at hand-over: CLKSEL_CON(158) 0x%08x CLKSEL_CON(159) 0x%08x "
	       "CLKGATE_CON(66) 0x%08x MODE_CON0 0x%08x PLL_CON 0x%08x 0x%08x 0x%08x: "
	       "clk_gpu_src %u Hz\n",
	       h->clock[0], h->clock[1], h->clock[2], h->clock[3], h->clock[4], h->clock[5],
	       h->clock[6], h->gpu_src_hz[0]);
	printk("t6_gpu: SPI2 gates CLKGATE_CON(14) 0x%08x, opened 0x%x\n", h->spi_gate14,
	       h->spi_opened);
	printk("t6_gpu: SPI2 as found: CTRLR0 0x%08x CTRLR1 0x%08x SSIENR 0x%x SER 0x%x BAUDR 0x%x "
	       "SR 0x%08x RXFLR 0x%x VERSION 0x%08x\n",
	       h->spi_found[0], h->spi_found[1], h->spi_found[2], h->spi_found[3], h->spi_found[4],
	       h->spi_found[5], h->spi_found[6], h->spi_found[7]);
	printk("t6_gpu: RK806 at hand-over: POWER_EN0 0x%02x POWER_SLP_EN0 0x%02x BUCK1_CONFIG "
	       "0x%02x BUCK1_ON_VSEL 0x%02x: vdd_gpu_s0 %s, %u uV\n",
	       h->rk806[0], h->rk806[1], h->rk806[2], h->rk806[3],
	       (h->rk806[0] & 0x1) ? "on" : "off", h->vdd_gpu_uv);
	t6_watchdog_feed();
}

int main(void)
{
	mm_reg_t cru, pmu, spi2, qos, gpu;
	struct yd_t6_gpu_up o;
	int64_t t0;
	int rc;

	printk("t6_gpu: build %s\n", T6_GPU_BUILD);
	printk("t6_gpu: WDT0 period %u ms, fed until %u ms of uptime\n", t6_watchdog_period_ms(),
	       t6_watchdog_deadline_ms());
	device_map(&cru, CRU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&pmu, PMU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&spi2, SPI2_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&qos, QOS_PHYS, 0x1000, K_MEM_CACHE_NONE);
	device_map(&gpu, GPU_PHYS, 0x1000, K_MEM_CACHE_NONE);
	t6_watchdog_feed();

	t0 = k_uptime_get();
	rc = yd_t6_gpu_up(cru, pmu, spi2, qos, gpu, delay_us, on_handover, &o);
	t6_watchdog_feed();
	if (rc == -EINVAL) {
		printk("t6_gpu: FAIL: yd_t6_gpu_up refused its arguments\n");
		t6_reset_to_linux("t6_gpu done");
	}
	if (o.stop <= YD_T6_GPU_STOP_CLOCK && o.stop != YD_T6_GPU_STOP_NONE) {
		/* stopped before the hand-over report: print what was read */
		on_handover(&o);
	}
	if (o.stop == YD_T6_GPU_STOP_NONE || o.stop >= YD_T6_GPU_STOP_PD) {
		printk("t6_gpu: PD_GPU on: gates opened 0x%x, was on %u, waited %u/%u/%u us\n",
		       o.pd_opened, o.pd_was_on, o.pd_waited_us[0], o.pd_waited_us[1],
		       o.pd_waited_us[2]);
		print_pd("after", o.pd_after);
	}
	if (o.stop == YD_T6_GPU_STOP_NONE || o.stop >= YD_T6_GPU_STOP_NOT_G610) {
		for (int b = 0; b < 4; b++) {
			printk("t6_gpu: qos_gpu_m%d: 0x%08x 0x%08x 0x%08x 0x%08x 0x%08x\n", b,
			       o.qos[b][0], o.qos[b][1], o.qos[b][2], o.qos[b][3], o.qos[b][4]);
		}
		print_ident("after power-on", o.ident);
	}
	if (o.stop == YD_T6_GPU_STOP_NONE || o.stop >= YD_T6_GPU_STOP_L2) {
		printk("t6_gpu: soft reset: GPU_INT_RAWSTAT 0x%08x before the clear, 0x%08x after; "
		       "RESET_COMPLETED after %u polls of 1 us\n",
		       o.reset[0], o.reset[1], o.reset[2]);
		print_ident("after reset", o.ident_after_reset);
	}
	if (o.stop == YD_T6_GPU_STOP_NONE) {
		printk("t6_gpu: L2 mask 0x%x: L2_READY 0x%x before L2_PWRON, 0x%x after; waited %u/%u "
		       "polls\n",
		       o.l2_mask[0], o.l2_ready_before[0], o.l2_ready[0], o.l2_waited_us[0],
		       o.l2_waited_us[1]);
		printk("t6_gpu: PASS: GPU_ID 0x%08x, L2 ready, %lld ms\n", o.ident[0],
		       (long long)(k_uptime_get() - t0));
	} else {
		printk("t6_gpu: FAIL: stop %u (%s), refusal %u, detail 0x%08x%08x\n", o.stop,
		       o.stop < ARRAY_SIZE(stop_name) ? stop_name[o.stop] : "unknown", o.refusal,
		       o.detail[1], o.detail[0]);
	}
	t6_reset_to_linux("t6_gpu done");
}
