/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * t6_gpu: the second half of Z1 on the NanoPC-T6 (driver/docs/nanopc-t6-mali.md).
 * Before main() the board glue has armed WDT0 (nanopc_t6.c). This prints
 * the build, then yd_t6_gpu_up (board::nanopc_t6_gpu in the drivers crate)
 * reads what the kexec from Linux handed over -- PD_GPU in the PMU, the
 * GPU's clock in the CRU, vdd_gpu_s0 in the RK806 over SPI2, read only --
 * and this prints it before anything is written; then PD_GPU on, the GPU's
 * identity, a soft reset and the L2 on. The lines are Z1's record
 * (../common/t6_z1.c, which the later probes print on a Z1 stop). The last
 * line is PASS or FAIL with the step that stopped; the board then resets back
 * into Armbian.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/kernel/mm.h>

#include <stdbool.h>

#include <nanopc_t6.h>
#include <yakogl_drivers.h>

#include "t6_z1.h"

#define CRU_PHYS  0xfd7c0000UL
#define PMU_PHYS  0xfd8d8000UL
#define SPI2_PHYS 0xfeb20000UL
#define QOS_PHYS  0xfdf35000UL
#define GPU_PHYS  0xfb000000UL

static bool handed_over;

static void delay_us(uint32_t us)
{
	k_busy_wait(us);
}

static void on_handover(const struct yd_t6_gpu_up *h)
{
	t6_z1_print_handover("t6_gpu", h);
	handed_over = true;
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

	/* -DT6_GPU_QOS=reset|linux-zero at build time (CMakeLists.txt) */
	if (yd_t6_gpu_qos_policy(T6_GPU_QOS_POLICY) != 0) {
		printk("t6_gpu: FAIL: QoS policy %d unknown\n", T6_GPU_QOS_POLICY);
		t6_reset_to_linux("t6_gpu done");
	}

	t0 = k_uptime_get();
	rc = yd_t6_gpu_up(cru, pmu, spi2, qos, gpu, delay_us, on_handover, &o);
	t6_watchdog_feed();
	if (rc == -EINVAL) {
		printk("t6_gpu: FAIL: yd_t6_gpu_up refused its arguments\n");
		t6_reset_to_linux("t6_gpu done");
	}
	t6_z1_print_record("t6_gpu", &o, handed_over);
	if (o.stop == YD_T6_GPU_STOP_NONE) {
		printk("t6_gpu: PASS: GPU_ID 0x%08x, L2 ready, %lld ms\n", o.ident[0],
		       (long long)(k_uptime_get() - t0));
	} else {
		printk("t6_gpu: FAIL: stop %u (%s), refusal %u, detail 0x%08x%08x\n", o.stop,
		       t6_z1_stop_name(o.stop), o.refusal, o.detail[1], o.detail[0]);
	}
	t6_reset_to_linux("t6_gpu done");
}
