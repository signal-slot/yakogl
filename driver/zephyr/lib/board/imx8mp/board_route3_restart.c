/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Route 3, the A53 side down and up again (CONFIG_YAKOGL_BOARD_IMX8MP_ROUTE3_A53_RESTART):
 * the protocol is <cluster/a53_power.h>, which cites its sources. This file is
 * the image's half of it.
 *
 * Down. The M7 writes cmd = DOWN and raises MU_1 general-purpose interrupt 1
 * (A side ASR GIP1, GIC SPI 88). mu_isr() clears GIP1, takes a DOWN with a
 * cmd_seq it has not acted on and wakes down_thread(), pinned to CPU 0:
 *   GOING_DOWN, a53_down_seen; no more publishing, no heartbeat
 *   (board_route3_quiesce); the GPU stopped with nothing in flight
 *   (gc_render_quiesce: the submit lock kept, the work waited for, the core
 *   reset, so its front end's WAIT/LINK loop fetches nothing more). The
 *   GPU is the only DMA master this image starts: the LCDIF is the M7's
 *   from the first show on (board_route3.c: no LCDIF write after it) and
 *   is left scanning.
 *   cores 1-3 off: one thread pinned to each (k_thread_cpu_pin), which
 *   masks its own interrupts and calls pm_cpu_off() (PSCI CPU_OFF from its
 *   own core). CPU 0 waits until PSCI AFFINITY_INFO says each is OFF: NXP
 *   TF-A's SYSTEM_SUSPEND is DENIED while another core is not
 *   (lib/psci/psci_main.c psci_system_suspend, psci_common.c
 *   psci_is_last_on_cpu, at lf-6.6.52-2.2.1).
 *   ack_seq, SUSPENDING, a53_suspend; interrupts masked; every SPI but 88
 *   disabled in the GIC and 88 enabled, ICC_PMR_EL1 = 0; SYSTEM_SUSPEND
 *   (0xC400000E, entry the loader at 0x40200000, context 0) as a raw SMC.
 *   If it returns: suspend_rc, SUSPEND_FAIL, and the image goes on (on
 *   CPU 0, without its GPU).
 *
 * What wakes core 0, from TF-A's source (lf-6.6.52-2.2.1):
 *   imx_domain_suspend (plat/imx/imx8m/imx8m_psci_common.c) calls
 *   imx_set_sys_wakeup(core, true) (plat/imx/imx8m/gpc_common.c) for the
 *   system level, on the LPA path too. That writes the calling core's GPC
 *   IMR words to ~GICD_ISENABLER, so every SPI enabled in the GIC is a wake
 *   source, and then, imx_is_m4_enabled() (IOMUXC_GPR22 CPUWAIT clear: the
 *   M7 released), clears IMR word 2 bit 24 = SPI 88, the MU, whatever the
 *   GIC says. The GPC watches the interrupt line, not the GIC ("gic's clock
 *   will be gated in system suspend"). So the wake needs the MU line itself
 *   to rise: ACR GIE1 stays set, and GIP1 must not be cleared by this side
 *   once the DOWN is taken (an UP that comes while going down then wakes the
 *   system the moment it is down). SPI 88 is GIC-enabled at the call all the
 *   same (its IMR bit is then cleared by both routes), every other SPI is
 *   not (the GPIO4 SPI is the M7's touch: enabled, a touch would wake us).
 *   psci_cpu_suspend_start (lib/psci/psci_suspend.c) abandons the suspend
 *   and returns success when ISR_EL1 shows an interrupt pending at the CPU
 *   interface; ICC_PMR_EL1 = 0 signals none (the MU line, the timer PPI and
 *   a late SGI included), so a pending SPI 88 still wakes through the GPC
 *   rather than cancelling the suspend.
 *
 * Up. TF-A resumes core 0 at the non-secure entry point, the loader, which
 * inflates this image again (header: the loader stamps a53_loader_in).
 * board_route3_restart_detect(), from board_route3.c's first hook
 * (tl_entry, PRE_KERNEL_1 0), decides that this boot is a restart: the M7 is
 * released (IOMUXC_GPR22 CPUWAIT clear; a SoC reset sets it again), the
 * power page has the magic and SUSPENDING, and the M7 status block reads
 * 'SAF7' with a heartbeat that moves. Then RUN_RESTART, restarts++,
 * a53_image_in, and ack_seq for the UP. From then on:
 *   the display chain's devices (deferred in a53-restart.overlay) are not
 *   initialised: chain_init() below, which does that on a cold boot, does
 *   nothing; neither do the early clocks (power_domain_yakogl_imx8mp_media.c)
 *   nor the panel console (gc_panel_console.c);
 *   board_route3.c neither loads nor releases the M7 and writes neither the
 *   status, the control nor the timeline page; the ring is taken as it is.
 * The GPU is reset by its device init as on every boot (gc_init: power-up,
 * identify, reset; gc_render_init: registers, MMU, ring), from the reset
 * gc_render_quiesce() left it in. WDOG1 is re-timed and fed by gc_sample.c
 * (PRE_KERNEL_1 0) and gc_wdog_arm.c (1) exactly as on a cold boot.
 *
 * The cold boot with this option: chain_init() initialises the same devices
 * in the device model's order, at the device model's place (the first of them
 * is at POST_KERNEL CONFIG_GPIO_INIT_PRIORITY, and nothing else is between it
 * and the last; the BUILD_ASSERTs below pin the order), and once the route is
 * UP with a frame published the DOWN interrupt is armed.
 */
#include <zephyr/kernel.h>
#include <zephyr/device.h>
#include <zephyr/devicetree.h>
#include <zephyr/init.h>
#include <zephyr/irq.h>
#include <zephyr/kernel/mm.h>
#include <zephyr/sys/barrier.h>
#include <zephyr/sys/device_mmio.h>
#include <zephyr/sys/printk.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/sys/util.h>
#include <zephyr/drivers/pm_cpu_ops.h>
#include <zephyr/drivers/interrupt_controller/gic.h>
#include <errno.h>
#include <stddef.h>

#include <cluster/a53_power.h>
#include <yakogl_drv/imx8mp_media.h>

#include "board_imx8mp_route3.h"
#include "gc_render.h"

/* ---- addresses ------------------------------------------------------------ */

#define M7_STATUS_ADDR   0x81000000u
#define M7_STATUS_MAGIC  0x37464153u /* 'SAF7' (board_route3.c) */
#define ST_MAGIC         0
#define ST_HEARTBEAT     2  /* struct m7_status: +1 every M7 loop (the M7 firmware's main.c) */
#define IOMUXC_GPR_BASE  0x30340000u
#define IOMUXC_GPR22     0x58u
#define GPR22_CM7_CPUWAIT BIT(0) /* TF-A IMX_M4_ENABLED_MASK = GPR_CM7_CPUWAIT */

#define PAGE_BASE        ROUND_DOWN(CLUSTER_A53_POWER_ADDR, CONFIG_MMU_PAGE_SIZE)
BUILD_ASSERT(CLUSTER_A53_POWER_ADDR + sizeof(struct cluster_a53_power) <=
		     PAGE_BASE + CONFIG_MMU_PAGE_SIZE,
	     "the power page sits in one MMU page");
BUILD_ASSERT(offsetof(struct cluster_a53_power, m7_down_req) == 32u &&
		     offsetof(struct cluster_a53_power, a53_first_frame) == 88u &&
		     sizeof(struct cluster_a53_power) == 144u,
	     "<cluster/a53_power.h> layout (both sides build it from the same header)");

/* The MU interrupt as Zephyr numbers it: GIC SPI n is INTID n + 32
 * (scripts/dts/gen_defines.py, map_arm_gic_irq_type). */
#define MU1_IRQ          (CLUSTER_MU1_A53_SPI + 32u)
BUILD_ASSERT(MU1_IRQ < CONFIG_NUM_IRQS);

/* PSCI, SMC64 function IDs (DEN0022). */
#define PSCI_AFFINITY_INFO   0xC4000004u
#define PSCI_SYSTEM_SUSPEND  0xC400000Eu
#define PSCI_AFF_OFF         1

/*
 * How long the M7's heartbeat may take to move. It is bumped once per M7 loop
 * (the M7 firmware's platform/fw_imx8mp/src/main.c,
 * M7_STATUS->heartbeat++), and a loop is at most a full-screen fallback
 * render plus its 1 ms rest
 * (k_sleep at the end of the loop): far less than this. A cold boot never
 * waits for it: the M7 is held (CPUWAIT) after any reset.
 */
#define HEARTBEAT_WAIT_MS    200u
#define GPU_QUIESCE_MS       2000u
#define CORES_OFF_WAIT_MS    500u

/* ---- state ---------------------------------------------------------------- */

static bool s_warm;
static volatile struct cluster_a53_power *s_pw;
static mm_reg_t s_mu;
static atomic_t s_down;         /* a DOWN taken (once per boot) */
static uint64_t s_down_tick;    /* when */
static uint32_t s_down_seq;     /* its cmd_seq */
static K_SEM_DEFINE(s_down_sem, 0, 1);

static K_THREAD_STACK_DEFINE(s_down_stack, 4096);
static struct k_thread s_down_thread;

#define OFF_CPUS (CONFIG_MP_MAX_NUM_CPUS - 1)
static K_THREAD_STACK_ARRAY_DEFINE(s_off_stacks, OFF_CPUS, 1024);
static struct k_thread s_off_threads[OFF_CPUS];
static volatile uint64_t s_off_mpidr[OFF_CPUS]; /* written by each, before its CPU_OFF */
static volatile int s_off_rc[OFF_CPUS];         /* only if CPU_OFF came back */

static inline uint64_t cntpct(void)
{
	uint64_t v;

	__asm__ volatile("isb; mrs %0, cntpct_el0" : "=r"(v) : : "memory");
	return v;
}

static inline uint32_t cntfrq(void)
{
	uint64_t v;

	__asm__ volatile("mrs %0, cntfrq_el0" : "=r"(v));
	return (uint32_t)v;
}

static uint64_t smc3(uint64_t fid, uint64_t a1, uint64_t a2)
{
	register uint64_t x0 __asm__("x0") = fid;
	register uint64_t x1 __asm__("x1") = a1;
	register uint64_t x2 __asm__("x2") = a2;

	__asm__ volatile("smc #0"
			 : "+r"(x0), "+r"(x1), "+r"(x2)
			 :
			 : "x3", "x4", "x5", "x6", "x7", "x8", "x9", "x10", "x11", "x12", "x13",
			   "x14", "x15", "x16", "x17", "memory");
	return x0;
}

static void *map_nc(uintptr_t phys, size_t size)
{
	uint8_t *cpu = NULL;

#ifdef K_MEM_ARM_NORMAL_NC
	k_mem_map_phys_bare(&cpu, phys, size, K_MEM_ARM_NORMAL_NC | K_MEM_PERM_RW);
#else
	k_mem_map_phys_bare(&cpu, phys, size, K_MEM_CACHE_WB | K_MEM_PERM_RW);
#endif
	return cpu;
}

bool board_route3_warm(void)
{
	return s_warm;
}

bool imx8mp_media_early_clocks_skip(void)
{
	return s_warm;
}

volatile struct cluster_a53_power *board_route3_power_page(void)
{
	return s_pw;
}

/* ---- the restart, at the first board hook ----------------------------------- */

void board_route3_restart_detect(uint64_t entry)
{
	mm_reg_t gpr = 0;
	uint8_t *page, *stp;
	const volatile uint32_t *st;
	uint64_t t0, limit;
	uint32_t beat;
	bool moving = false;

	page = map_nc(PAGE_BASE, CONFIG_MMU_PAGE_SIZE);
	if (page == NULL) {
		return; /* no page, no protocol: a cold boot, never armed */
	}
	s_pw = (volatile struct cluster_a53_power *)(page + (CLUSTER_A53_POWER_ADDR - PAGE_BASE));

	/* Any SoC reset holds the M7 again (GPR22 CPUWAIT): a cold boot, at
	 * once, whatever the DDR still says. */
	device_map(&gpr, IOMUXC_GPR_BASE, 0x1000, K_MEM_CACHE_NONE);
	if (gpr == 0 || (sys_read32(gpr + IOMUXC_GPR22) & GPR22_CM7_CPUWAIT) != 0u) {
		return;
	}
	if (s_pw->magic != CLUSTER_A53_POWER_MAGIC || s_pw->a53_state != CLUSTER_A53_ST_SUSPENDING) {
		return;
	}
	stp = map_nc(M7_STATUS_ADDR, CONFIG_MMU_PAGE_SIZE);
	if (stp == NULL) {
		return;
	}
	st = (const volatile uint32_t *)stp;
	if (st[ST_MAGIC] == M7_STATUS_MAGIC) {
		/* the system counter: no kernel timer at PRE_KERNEL_1 */
		beat = st[ST_HEARTBEAT];
		t0 = cntpct();
		limit = (uint64_t)cntfrq() * HEARTBEAT_WAIT_MS / 1000u;
		while (cntpct() - t0 < limit) {
			if (st[ST_HEARTBEAT] != beat) {
				moving = true;
				break;
			}
		}
	}
	k_mem_unmap_phys_bare(stp, CONFIG_MMU_PAGE_SIZE);
	if (!moving) {
		return;
	}
	s_warm = true;
	s_pw->a53_image_in = entry;
	s_pw->restarts++;
	if (s_pw->cmd == CLUSTER_A53_CMD_UP) {
		s_pw->ack_seq = s_pw->cmd_seq; /* the UP is what this boot acts on */
	}
	barrier_dmem_fence_full();
	s_pw->a53_state = CLUSTER_A53_ST_RUN_RESTART;
}

/* ---- the display chain on a cold boot ----------------------------------------- */

#define CHAIN_NODE_GPIO   DT_NODELABEL(gpio4)
#define CHAIN_NODE_PWM    DT_NODELABEL(backlight_pwm)
#define CHAIN_NODE_I2C    DT_NODELABEL(i2c2)
#define CHAIN_NODE_MEDIA  DT_NODELABEL(media_blk_ctrl)
#define CHAIN_NODE_PD0    DT_NODELABEL(pd_lcdif1)
#define CHAIN_NODE_PD1    DT_NODELABEL(pd_mipi_dsi1)
#define CHAIN_NODE_BRIDGE DT_NODELABEL(lvds_bridge)
#define CHAIN_NODE_DISP   DT_NODELABEL(display0)

#define DEFERRED(n) DT_PROP(n, zephyr_deferred_init)
BUILD_ASSERT(DEFERRED(CHAIN_NODE_GPIO) && DEFERRED(CHAIN_NODE_PWM) && DEFERRED(CHAIN_NODE_I2C) &&
		     DEFERRED(CHAIN_NODE_MEDIA) && DEFERRED(CHAIN_NODE_PD0) &&
		     DEFERRED(CHAIN_NODE_PD1) && DEFERRED(CHAIN_NODE_BRIDGE) &&
		     DEFERRED(CHAIN_NODE_DISP),
	     "a53-restart.overlay defers every device of the display chain");

/*
 * The device model's order (kernel/init.c runs the init entries sorted by
 * level, priority, then devicetree dependency ordinal: Z_INIT_ENTRY_SECTION
 * with Z_DEVICE_INIT_SUB_PRIO), each pair pinned here: (priority, ordinal)
 * strictly increasing.
 */
#define BEFORE(p1, n1, p2, n2) ((p1) < (p2) || ((p1) == (p2) && DT_DEP_ORD(n1) < DT_DEP_ORD(n2)))
/* the bridge's own priority: it has no dsi-host (bridge_sn65dsi8x.c, SN_SELF_START) */
BUILD_ASSERT(!DT_NODE_HAS_PROP(CHAIN_NODE_BRIDGE, dsi_host));
BUILD_ASSERT(BEFORE(CONFIG_GPIO_INIT_PRIORITY, CHAIN_NODE_GPIO, CONFIG_PWM_INIT_PRIORITY,
		    CHAIN_NODE_PWM));
BUILD_ASSERT(BEFORE(CONFIG_PWM_INIT_PRIORITY, CHAIN_NODE_PWM, CONFIG_I2C_INIT_PRIORITY,
		    CHAIN_NODE_I2C));
BUILD_ASSERT(BEFORE(CONFIG_I2C_INIT_PRIORITY, CHAIN_NODE_I2C,
		    CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_INIT_PRIORITY, CHAIN_NODE_MEDIA));
BUILD_ASSERT(BEFORE(CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_INIT_PRIORITY, CHAIN_NODE_MEDIA,
		    CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD_INIT_PRIORITY, CHAIN_NODE_PD0));
BUILD_ASSERT(BEFORE(CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD_INIT_PRIORITY, CHAIN_NODE_PD0,
		    CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD_INIT_PRIORITY, CHAIN_NODE_PD1));
BUILD_ASSERT(BEFORE(CONFIG_YAKOGL_DRV_IMX8MP_MEDIA_PD_INIT_PRIORITY, CHAIN_NODE_PD1,
		    CONFIG_YAKOGL_DRV_SN65DSI8X_INIT_PRIORITY, CHAIN_NODE_BRIDGE));
BUILD_ASSERT(BEFORE(CONFIG_YAKOGL_DRV_SN65DSI8X_INIT_PRIORITY, CHAIN_NODE_BRIDGE,
		    CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_INIT_PRIORITY, CHAIN_NODE_DISP));
/* what comes after the chain still does */
#ifdef CONFIG_GC_COMMON_PANEL_CONSOLE_COLD_BRINGUP
BUILD_ASSERT(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_INIT_PRIORITY <
	     CONFIG_GC_COMMON_PANEL_CONSOLE_COLD_PRIORITY);
#endif
BUILD_ASSERT(CONFIG_YAKOGL_DRV_DISPLAY_IMX8MP_INIT_PRIORITY < CONFIG_YAKOGL_DRV_GPU_VIVANTE_INIT_PRIORITY);

static const struct device *const chain[] = {
	DEVICE_DT_GET(CHAIN_NODE_GPIO),  DEVICE_DT_GET(CHAIN_NODE_PWM),
	DEVICE_DT_GET(CHAIN_NODE_I2C),   DEVICE_DT_GET(CHAIN_NODE_MEDIA),
	DEVICE_DT_GET(CHAIN_NODE_PD0),   DEVICE_DT_GET(CHAIN_NODE_PD1),
	DEVICE_DT_GET(CHAIN_NODE_BRIDGE), DEVICE_DT_GET(CHAIN_NODE_DISP),
};

/* At the first chain device's own place. A failure is the device model's:
 * recorded in the device (not ready), and the next one is initialised. */
static int chain_init(void)
{
	if (s_warm) {
		return 0;
	}
	for (size_t i = 0; i < ARRAY_SIZE(chain); i++) {
		int rc = device_init(chain[i]);

		if (rc != 0) {
			printk("route3: %s init: %d\n", chain[i]->name, rc);
		}
	}
	return 0;
}
SYS_INIT(chain_init, POST_KERNEL, CONFIG_GPIO_INIT_PRIORITY);

/* ---- down ----------------------------------------------------------------- */

/* Interrupts masked or in the ISR: a DOWN not yet acted on is taken once. The
 * line is then left disabled in the GIC and GIP1 is no longer cleared, so
 * an UP that follows keeps the MU line up for the wake. */
static void take_down_if_new(void)
{
	uint32_t cmd, seq;

	barrier_dmem_fence_full();
	cmd = s_pw->cmd;
	seq = s_pw->cmd_seq;
	if (cmd != CLUSTER_A53_CMD_DOWN || seq == s_pw->ack_seq || !atomic_cas(&s_down, 0, 1)) {
		return;
	}
	s_down_tick = cntpct();
	s_down_seq = seq;
	irq_disable(MU1_IRQ);
	k_sem_give(&s_down_sem);
}

static void mu_isr(const void *arg)
{
	ARG_UNUSED(arg);
	if (atomic_get(&s_down) != 0) {
		irq_disable(MU1_IRQ);
		return;
	}
	sys_write32(CLUSTER_MU_GI1_PND, s_mu + CLUSTER_MU_SR); /* write 1 to clear */
	take_down_if_new();
}

/* One per core 1..3: that core's interrupts masked (arch_irq_lock: local
 * only; irq_lock() on SMP would also take the global lock and never give it
 * back), then PSCI CPU_OFF from it. Returns only if TF-A refused. */
static void off_entry(void *p1, void *p2, void *p3)
{
	const unsigned int i = (unsigned int)(uintptr_t)p1;

	ARG_UNUSED(p2);
	ARG_UNUSED(p3);
	(void)arch_irq_lock();
	s_off_mpidr[i] = MPIDR_TO_CORE(GET_MPIDR());
	barrier_dmem_fence_full();
	s_off_rc[i] = pm_cpu_off();
	for (;;) {
		__asm__ volatile("wfe");
	}
}

/* Cores 1..3 off; true once PSCI says each is. */
static bool cores_off(void)
{
	const uint64_t limit = (uint64_t)cntfrq() * CORES_OFF_WAIT_MS / 1000u;
	const uint64_t t0 = cntpct();
	bool all;

	for (unsigned int i = 0; i < OFF_CPUS; i++) {
		s_off_mpidr[i] = UINT64_MAX;
		s_off_rc[i] = 0;
		k_thread_create(&s_off_threads[i], s_off_stacks[i],
				K_THREAD_STACK_SIZEOF(s_off_stacks[i]), off_entry,
				(void *)(uintptr_t)i, NULL, NULL, K_HIGHEST_THREAD_PRIO, 0, K_FOREVER);
		(void)k_thread_cpu_pin(&s_off_threads[i], (int)i + 1);
		k_thread_start(&s_off_threads[i]);
	}
	do {
		all = true;
		for (unsigned int i = 0; i < OFF_CPUS && all; i++) {
			all = s_off_mpidr[i] != UINT64_MAX &&
			      smc3(PSCI_AFFINITY_INFO, s_off_mpidr[i], 0) == PSCI_AFF_OFF;
		}
		if (!all) {
			k_busy_wait(100);
		}
	} while (!all && cntpct() - t0 < limit);
	for (unsigned int i = 0; i < OFF_CPUS && !all; i++) {
		printk("route3: core %u (mpidr 0x%llx) not off (CPU_OFF %d)\n", i + 1,
		       (unsigned long long)s_off_mpidr[i], s_off_rc[i]);
	}
	return all;
}

static void down_thread(void *p1, void *p2, void *p3)
{
	unsigned int key;
	int gpu, rc;

	ARG_UNUSED(p1);
	ARG_UNUSED(p2);
	ARG_UNUSED(p3);
	k_sem_take(&s_down_sem, K_FOREVER);
	s_pw->a53_down_seen = s_down_tick;
	s_pw->a53_state = CLUSTER_A53_ST_GOING_DOWN;
	board_route3_quiesce();
	gpu = gc_render_quiesce(GPU_QUIESCE_MS);
	printk("route3: DOWN (cmd_seq %u): GPU stopped (%d); cores 1-%u off next\n", s_down_seq, gpu,
	       OFF_CPUS);
	(void)cores_off();

	/* CPU 0 alone from here */
	s_pw->ack_seq = s_down_seq;
	s_pw->a53_state = CLUSTER_A53_ST_SUSPENDING;
	s_pw->a53_suspend = cntpct();
	barrier_dsync_fence_full();
	key = arch_irq_lock();
	for (unsigned int irq = 32u; irq < CONFIG_NUM_IRQS; irq++) {
		if (irq != MU1_IRQ) {
			irq_disable(irq);
		}
	}
	irq_enable(MU1_IRQ);
	write_sysreg(0, ICC_PMR_EL1);
	barrier_isync_fence_full();
	rc = (int)(int64_t)smc3(PSCI_SYSTEM_SUSPEND, CLUSTER_A53_RESTART_ENTRY, 0);

	/* came back: the suspend did not happen */
	write_sysreg(GIC_IDLE_PRIO, ICC_PMR_EL1);
	barrier_isync_fence_full();
	s_pw->suspend_rc = rc;
	barrier_dmem_fence_full();
	s_pw->a53_state = CLUSTER_A53_ST_SUSPEND_FAIL;
	arch_irq_unlock(key);
	printk("route3: SYSTEM_SUSPEND returned %d: running on, on CPU 0, without the GPU\n", rc);
}

void board_route3_restart_arm(void)
{
	static bool armed;

	if (armed) {
		return;
	}
	armed = true;
	if (s_pw == NULL || s_pw->magic != CLUSTER_A53_POWER_MAGIC) {
		printk("route3: no A53 power page from the M7 (magic 0x%08x): DOWN not armed\n",
		       s_pw != NULL ? s_pw->magic : 0u);
		return;
	}
	if (!s_warm) {
		s_pw->a53_state = CLUSTER_A53_ST_RUN_COLD;
	}
	device_map(&s_mu, CLUSTER_MU1_A_BASE, 0x1000, K_MEM_CACHE_NONE);
	k_thread_create(&s_down_thread, s_down_stack, K_THREAD_STACK_SIZEOF(s_down_stack),
			down_thread, NULL, NULL, NULL, K_PRIO_COOP(1), 0, K_FOREVER);
	(void)k_thread_cpu_pin(&s_down_thread, 0);
	k_thread_name_set(&s_down_thread, "a53_down");
	k_thread_start(&s_down_thread);

	IRQ_CONNECT(MU1_IRQ, IRQ_DEFAULT_PRIORITY, mu_isr, NULL, 0);
	/* a GIP1 left from before (the UP that woke this boot) first, then the
	 * enable; a DOWN written before either is in the page, read below */
	sys_write32(CLUSTER_MU_GI1_PND, s_mu + CLUSTER_MU_SR);
	sys_write32(sys_read32(s_mu + CLUSTER_MU_CR) | CLUSTER_MU_GI1_EN, s_mu + CLUSTER_MU_CR);
	irq_enable(MU1_IRQ);
	{
		unsigned int key = irq_lock();

		take_down_if_new();
		irq_unlock(key);
	}
	printk("route3: DOWN armed (%s boot, cmd %u seq %u ack %u, restarts %u)\n",
	       s_warm ? "restart" : "cold", s_pw->cmd, s_pw->cmd_seq, s_pw->ack_seq,
	       s_pw->restarts);
}
