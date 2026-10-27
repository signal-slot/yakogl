/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* The bounded lifetime and the way back to Linux; see gc_sample.h. */

#include <zephyr/kernel.h>
#include <zephyr/arch/cpu.h>
#include <zephyr/init.h>
#include <zephyr/logging/log.h>
#include <zephyr/sys/reboot.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/kernel/mm.h>

#include <yakogl_drivers.h> /* yd_imx8mp_hold_seconds */

#include "gc_sample.h"
#include "gc_panel_console.h"
#include "gc_irq_guard.h"
#include "gc_result.h"
#ifdef CONFIG_GC_BOARD_WDOG_SELF_ARM
#include "gc_wdog_arm.h"
#endif
#include <zephyr/fatal.h>
#include <zephyr/arch/arm64/exception.h>

LOG_MODULE_REGISTER(gc_sample, LOG_LEVEL_INF);

#if CONFIG_GC_SAMPLE_WDOG_BASE != 0
static mm_reg_t s_wdog;

/* Mapped before any driver runs, so that the fatal error handler can feed
 * the watchdog without calling into the memory manager from an exception
 * (a nested fault there is what a first version of the handler did). */
static uint16_t s_wdog_wcr;   /* WCR as found at boot: WT in bits 15:8, WDE bit 2 */

static int wdog_map(void)
{
	device_map(&s_wdog, CONFIG_GC_SAMPLE_WDOG_BASE, 0x1000, K_MEM_CACHE_NONE);
	s_wdog_wcr = sys_read16(s_wdog);
	gc_sample_service_wdog();
	return 0;
}

uint16_t gc_sample_wdog_control(void)
{
	return s_wdog_wcr;
}
SYS_INIT(wdog_map, PRE_KERNEL_1, 0);

#ifdef CONFIG_GC_SAMPLE_WDOG_TIMER_FEED
/*
 * Feed the watchdog from a timer callback rather than from whichever thread
 * happens to be running.
 *
 * Every feed in this file is made by the code doing the work -- the render
 * loop, the supervisor, the hold. That is fine until the work stops, and then
 * the board is reset by the watchdog at a fixed interval with no report: the
 * route-3 image returned exactly 131 s for every configuration because it
 * wedged somewhere and nothing fed from there on (docs/step6-a53-producer.md).
 * A wedge that cannot say anything is the worst case for a board whose only
 * output is how long it takes to come back.
 *
 * So: a timer feeds instead, which keeps the board alive while a thread is
 * stuck, long enough for the supervisor's STUCK code to get out if the
 * supervisor can still run at all.
 *
 * BOUNDED, and that is not negotiable -- feeding a watchdog for ever is how
 * this lab lost a board for a day. After CONFIG_GC_SAMPLE_WDOG_TIMER_CAP_S
 * the timer stops feeding and the watchdog takes the board, so a wedge
 * returns at the cap plus the watchdog rather than never.
 *
 * What it cannot do: a k_timer callback runs from the system timer interrupt,
 * so if the TIMER itself has stopped this never fires and nothing changes.
 * That is a useful thing to learn rather than a limitation to hide -- a run
 * that still returns 131 s with this enabled says the tick is dead, which is
 * a different fault from a blocked thread.
 */
static struct k_timer s_wdog_timer;
static uint32_t s_wdog_feeds;

static void wdog_timer_expiry(struct k_timer *t)
{
	ARG_UNUSED(t);
	/* ISR context: two MMIO writes and a counter, nothing else. */
	if (s_wdog_feeds >= (uint32_t)CONFIG_GC_SAMPLE_WDOG_TIMER_CAP_S) {
		k_timer_stop(&s_wdog_timer);
		return;
	}
	s_wdog_feeds++;
	gc_sample_service_wdog();
}

static int wdog_timer_start(void)
{
	k_timer_init(&s_wdog_timer, wdog_timer_expiry, NULL);
	k_timer_start(&s_wdog_timer, K_SECONDS(1), K_SECONDS(1));
	return 0;
}
SYS_INIT(wdog_timer_start, POST_KERNEL, 0);

uint32_t gc_sample_wdog_feeds(void)
{
	return s_wdog_feeds;
}
#endif /* CONFIG_GC_SAMPLE_WDOG_TIMER_FEED */

void gc_sample_service_wdog(void)
{
	/* i.MX2+ WDOG: 0x5555 then 0xaaaa to WSR (offset 2) services it. */
	if (s_wdog == 0) {
		return;   /* before wdog_map(): nothing to feed with yet */
	}
#ifdef CONFIG_GC_BOARD_WDOG_SELF_ARM
	/* The image's bound (gc_wdog_arm.c): no feeder outlives it. */
	if (!gc_wdog_arm_may_feed()) {
		return;
	}
#endif
	sys_write16(0x5555, s_wdog + 2);
	sys_write16(0xaaaa, s_wdog + 2);
}
#else
void gc_sample_service_wdog(void) {}
uint16_t gc_sample_wdog_control(void) { return 0; }
#endif

/* The last verdict, for a board whose only other output is when it comes
 * back: a failed sample holds for a different, shorter time. */
static bool s_failed;
static int s_hold_code = -1;

/* The Vivante driver's GPU-reset counter. Weak, so a sample that does not link
 * the driver still builds and reads zero; when the driver IS linked its own
 * definition wins and the count arrives without the sample doing anything.
 * Deliberately not a call the sample has to remember to make. */
__weak uint32_t gc_render_resets(void)
{
	return 0;
}

/* Latched at the verdict so the hold code and the printed line agree even if
 * the driver resets again while the hold is running. */
static uint32_t s_gpu_resets;

void gc_sample_hold_code(unsigned int code)
{
	s_hold_code = (int)(code & 0x3fu);
}

bool gc_sample_verdict(const char *name, bool pass, const char *reason)
{
	/* A recovered wedge is not a pass. The driver resets the GPU to get out
	 * of a fence timeout and the frame is lost; on a cluster that is a
	 * safety-relevant event, so it overrides the sample's own checks rather
	 * than being reported beside them. Read here, not left to the sample:
	 * the whole hazard is a recovery going unnoticed. */
	s_gpu_resets = gc_render_resets();
	if (s_gpu_resets != 0u && pass) {
		pass = false;
		reason = "the GPU was reset during the run";
	}

	s_failed = !pass;
	if (pass) {
		LOG_INF("%s: PASS", name);
	} else {
		LOG_ERR("%s: FAIL %s", name, reason ? reason : "(no reason given)");
	}
	if (s_gpu_resets != 0u) {
		LOG_ERR("%s: the GPU was reset %u time(s) -- a recovered hang, not a clean run",
			name, s_gpu_resets);
	}
	return pass;
}

/* The end of every sample: the hold (board::imx8mp::hold_seconds), then the
 * SoC reset. `recorded`: the verdict is already in the result record, so
 * only a code no record carries is held for. */
static void finish(bool recorded)
{
	gc_panel_console_stage(6, 0xff00ff);   /* magenta: the hold began */
#ifdef CONFIG_GC_COMMON_IRQ_GUARD
	gc_irq_guard_report();
#endif

#ifdef CONFIG_GC_BOARD_WDOG_PROOF
	/* The proof image: WDOG1 resets the SoC, not this; and the hold must not
	 * delay the proof record past the 60 s WDOG1 gives. Never returns. */
	gc_wdog_arm_proof_hold();
#endif

#ifdef CONFIG_GC_SAMPLE_HOLD_REPORTS_WDOG
	/* Diagnostic: the watchdog's state at boot instead of the sample's
	 * code. 32 when it was enabled (WDE), plus WT / 8 (timeout in
	 * seconds = (WT + 1) / 2, so each unit is 4 s); 0 when disabled. */
	s_hold_code = (s_wdog_wcr & BIT(2)) ? 32 + ((s_wdog_wcr >> 8) & 0xff) / 8 : 0;
#endif
	/* A code the sample claimed, else the GPU reset count as codes
	 * 56 + (N - 1), N clamped at 8: the one readout no record carries. */
	const struct yd_imx8mp_hold in = {
		.recorded = recorded ? 1u : 0u,
		.failed = s_failed ? 1u : 0u,
		.code = s_hold_code,
		.gpu_resets = s_gpu_resets,
		.pass_s = CONFIG_GC_SAMPLE_HOLD_SECONDS,
		.fail_s = CONFIG_GC_SAMPLE_HOLD_SECONDS_FAIL,
		.step_s = CONFIG_GC_SAMPLE_HOLD_CODE_STEP,
		.reset_base = GC_SAMPLE_HOLD_GPU_RESET_BASE,
		.reset_max = GC_SAMPLE_HOLD_GPU_RESET_MAX,
	};
	int32_t code = -1;
	const int hold = (int)yd_imx8mp_hold_seconds(&in, &code);

	if (code >= 0) {
		if (s_hold_code < 0) {
			LOG_ERR("hold code %d: the GPU was reset %u time(s)", code, s_gpu_resets);
		}
		LOG_INF("hold code %d: %d s", code, hold);
	}
	for (int s = 0; s < hold; s++) {
		gc_sample_service_wdog();
#ifdef CONFIG_GC_SAMPLE_HOLD_BUSY_WAIT
		/* No timer interrupt needed: a diagnostic for a board whose GIC
		 * state after kexec is in doubt. */
		k_busy_wait(1000000);
#else
		k_sleep(K_SECONDS(1));
#endif
	}
	LOG_INF("resetting the SoC");
	gc_panel_console_stage(7, 0x00ffff);   /* cyan: about to reset */
	sys_reboot(SYS_REBOOT_COLD);
}

void gc_sample_finish(void)
{
	finish(false);
}

void gc_sample_finish_recorded(void)
{
	finish(true);
}


/* The interrupt this CPU is handling, from the GIC's active bits (the wrapper
 * only ends an interrupt after its handler returns, so a fatal error raised
 * from a handler leaves the line active). i.MX8MP: GICD 0x38800000, GICR
 * 0x38880000 with one 128 KiB frame per CPU, SGI/PPI frame at +0x10000. */
static mm_reg_t s_gicd, s_gicr;

/* Mapped up front: the fatal handler runs in exception context, where
 * asking the memory manager for a mapping faults again. */
static int gic_map(void)
{
	device_map(&s_gicd, 0x38800000, 0x10000, K_MEM_CACHE_NONE);
	device_map(&s_gicr, 0x38880000, 0x80000, K_MEM_CACHE_NONE);
	return 0;
}
SYS_INIT(gic_map, PRE_KERNEL_1, 0);

unsigned int gc_sample_active_intid(void)
{
	mm_reg_t gicd = s_gicd, gicr = s_gicr;
	unsigned int cpu = arch_curr_cpu()->id;
	uint32_t act;

	if (gicd == 0 || gicr == 0) {
		return 0xffffu;
	}
	act = sys_read32(gicr + cpu * 0x20000 + 0x10000 + 0x300);   /* GICR_ISACTIVER0 */
	for (unsigned int b = 0; b < 32; b++) {
		if (act & BIT(b)) {
			return b;
		}
	}
	for (unsigned int r = 1; r < 8; r++) {
		act = sys_read32(gicd + 0x300 + 4u * r);   /* GICD_ISACTIVER<r> */
		for (unsigned int b = 0; b < 32; b++) {
			if (act & BIT(b)) {
				return 32u * r + b;
			}
		}
	}
	return 0xffffu;
}


#ifdef CONFIG_GC_SAMPLE_FAULT_RECORD
/*
 * A black box for a fault that cannot speak.
 *
 * FATAL_HANDLER holds 15 s and resets, so an ordinary fault returns short and
 * variable. What we see on the route-3 image is a flat 131 s, which is the
 * watchdog with nothing feeding it -- the signature of a fault whose handler
 * faults AGAIN. A nested exception leaves the CPU in the handler with
 * interrupts masked for ever: no ISR, no feed, no supervisor, and from
 * outside it is indistinguishable from a system tick that simply stopped.
 *
 * So the syndrome is written to DDR at the very first opportunity, before any
 * printing, unwinding or stack walking -- all of which can fault a second
 * time and are what makes the first fault silent. The area is inside the M7's
 * reserved carveout, which is no-map for Linux and therefore survives the SoC
 * reset; route3_boot.sh dumps it before starting the M7, so the syndrome
 * arrives in ring_at_boot.txt on the next boot.
 *
 * Mapped from SYS_INIT and not at attach, deliberately: a fault BEFORE the
 * ring is attached is exactly the case being chased, and a recorder that
 * needs attach to have happened could not record it.
 *
 * Plain stores to a Normal non-cacheable mapping, no function calls: anything
 * cleverer here is another chance to fault.
 */
/*
 * The magic is the M7's, not ours: it reserves the page and the magic word
 * (m7_mem.h, M7_A53_FAULT_MAGIC) and never reads or writes the body. The
 * layout under it is ours to define, which is why only this constant is
 * shared -- a firmware that does not read the record has no business
 * dictating its shape.
 */
#define FAULT_RECORD_MAGIC 0x46333541u   /* 'A53F', M7_A53_FAULT_MAGIC */
/* A second magic, in its own word, so "it faulted" and "it got this far" are
 * independent readings: a hang writes the breadcrumb and never the fault
 * record, and a fault writes both. 'A53P' for progress. */
#define PROGRESS_MAGIC 0x50333541u
/* 'A53M': the page is mapped and writable. Written at init, before anything
 * can have gone wrong, so that an otherwise empty page can be told apart from
 * a page nothing was ever able to write to. */
#define CHANNEL_MAGIC 0x4d333541u

static volatile uint32_t *s_fault_rec;
static uint32_t s_fault_note;

static int fault_record_map(void)
{
	uint8_t *p = NULL;

#ifdef K_MEM_ARM_NORMAL_NC
	k_mem_map_phys_bare(&p, (uintptr_t)CONFIG_GC_SAMPLE_FAULT_RECORD_ADDR, 0x1000,
			    K_MEM_ARM_NORMAL_NC | K_MEM_PERM_RW);
#else
	k_mem_map_phys_bare(&p, (uintptr_t)CONFIG_GC_SAMPLE_FAULT_RECORD_ADDR, 0x1000,
			    K_MEM_CACHE_NONE | K_MEM_PERM_RW);
#endif
	s_fault_rec = (volatile uint32_t *)p;
	if (s_fault_rec == NULL) {
		return 0;   /* nothing more can be done from here; see below */
	}
	/*
	 * Say that the channel exists, as early as anything can be said.
	 *
	 * Every other word here is written only when something happens, so an
	 * all-zero page has two meanings that matter enormously and look
	 * identical: "the run hung without faulting" and "the mapping failed,
	 * so nothing could ever have been written". k_mem_map_phys_bare()
	 * reports failure by leaving the pointer alone, and this used to
	 * discard that and return 0 -- a silent failure in the one instrument
	 * that exists to explain silence.
	 *
	 * This marker makes the page self-validating: if it is there, an
	 * empty fault record is evidence, and if it is not, the reader knows
	 * to distrust the whole page instead of concluding anything from it.
	 */
	s_fault_rec[14] = k_uptime_get_32();
	__asm__ volatile("dmb sy" ::: "memory");
	s_fault_rec[15] = CHANNEL_MAGIC;
	return 0;
}
SYS_INIT(fault_record_map, POST_KERNEL, 0);

/*
 * A breadcrumb, written as the run passes a point rather than when it dies.
 *
 * The fault record answers "where did it fault", and says nothing at all
 * about a run that hangs without faulting -- which on this board looks
 * identical from outside, because either way nothing feeds the watchdog and
 * the board returns at a fixed ~131 s. Bisecting a hang through the return
 * time costs one board run per bit, and the board is shared.
 *
 * This costs two uncached stores per call and leaves the last point reached
 * in DDR that the SoC reset does not clear, so ONE run recovers the whole
 * path. The counter goes up every time, so a loop that spins forever is
 * distinguishable from a stall at a point: the mark stops changing but the
 * count says how many times it got there.
 */
void gc_sample_progress(uint32_t mark)
{
	volatile uint32_t *r = s_fault_rec;

	if (r == NULL) {
		return;
	}
	r[11] = mark;
	r[12] = k_uptime_get_32();
	r[13]++;
	/* Ordered after the body for the same reason the fault record's magic
	 * is written last: a reader must not find the marker word without the
	 * uptime that belongs to it. */
	__asm__ volatile("dmb sy" ::: "memory");
	r[10] = PROGRESS_MAGIC;
}


void gc_sample_fault_note(uint32_t note)
{
	s_fault_note = note;
}

/* Called first thing in the fatal handler. Nothing before it. */
static inline void fault_record_store(unsigned int reason, uint64_t esr, uint64_t far,
				      uint64_t elr)
{
	volatile uint32_t *r = s_fault_rec;

	if (r == NULL) {
		return;
	}
	r[1] = reason;
	r[2] = (uint32_t)esr;
	r[3] = (uint32_t)(esr >> 32);
	r[4] = (uint32_t)far;
	r[5] = (uint32_t)(far >> 32);
	r[6] = (uint32_t)elr;
	r[7] = (uint32_t)(elr >> 32);
	r[8] = s_fault_note;
	r[9] = k_uptime_get_32();
	/*
	 * Body first, barrier, magic last -- the same rule the frame ring
	 * uses for `produced`, and for the same reason: a reader that sees
	 * the magic must be able to trust everything under it. Without the
	 * barrier the stores may reach DDR out of order and a reader can
	 * find the magic over a half-written record. This hook may not get
	 * to finish, so "half-written" is the expected case and not an
	 * unlucky one.
	 */
	/* dmb sy, not __DMB(): this is aarch64, and __DMB is a CMSIS macro
	 * for Cortex-M. Inline asm rather than a Zephyr barrier helper so
	 * the fatal path pulls in no header and calls nothing. */
	__asm__ volatile("dmb sy" ::: "memory");
	r[0] = FAULT_RECORD_MAGIC;
}
#else
static inline void fault_record_store(unsigned int reason, uint64_t esr, uint64_t far,
				      uint64_t elr)
{
	ARG_UNUSED(reason); ARG_UNUSED(esr); ARG_UNUSED(far); ARG_UNUSED(elr);
}
void gc_sample_fault_note(uint32_t note) { ARG_UNUSED(note); }
void gc_sample_progress(uint32_t mark) { ARG_UNUSED(mark); }
#endif /* CONFIG_GC_SAMPLE_FAULT_RECORD */

/* Declared in the GPU driver's private header, repeated rather than included:
 * gc_board builds for images that do not link the Vivante driver at all,
 * and overriding a weak symbol needs only its name and type. */
void gc_render_mark(unsigned int step);
/*
 * The driver's weak hook (src/vivante/gc_render.h), pointed at both
 * breadcrumbs: the DDR fault record when it is built in, and the SNVS record,
 * which is the one that survives the reset. gc_render_init() is the one
 * stretch of a sample with no console, no verdict and no way to report -- and
 * the route-3 wedge sits inside it.
 */
void gc_render_mark(unsigned int step)
{
	gc_sample_progress(step);
	gc_result_progress(GC_RESULT_WHERE_GPU_STEP | (step & 0xffu));
}

/* The GPU device init's weak hook (driver/drivers/gpu/gpu_vivante_gc_rust.c),
 * declared there and repeated here for the same reason as gc_render_mark:
 * kind STAGE breadcrumbs 0x30 + step (gc_result.h). */
void gc_init_mark(unsigned int step, int value);
void gc_init_mark(unsigned int step, int value)
{
	gc_result_stage((uint8_t)(GC_RESULT_STAGE_GPU_INIT + (step & 0xfu)), (uint32_t)value);
}

#ifdef CONFIG_GC_SAMPLE_FATAL_HANDLER
/*
 * A fatal error must not turn into two minutes of silence. Zephyr has
 * printed the exception (reason, ESR, ELR, FAR, the registers) through
 * printk, which on this board is the panel: keep the picture up for
 * CONFIG_GC_SAMPLE_HOLD_SECONDS_FATAL while servicing the watchdog, paint
 * the last stage bar red, then reset the SoC straight through SMC (PSCI
 * SYSTEM_RESET, SMCCC 0x84000009), since no driver can be trusted here.
 * The time until Linux is back then says "fatal" as clearly as PASS/FAIL.
 */
void k_sys_fatal_error_handler(unsigned int reason, const struct arch_esf *esf)
{
	int hold = CONFIG_GC_SAMPLE_HOLD_SECONDS_FATAL;
	uint64_t far = read_sysreg(FAR_EL1);
	uint64_t esr = read_sysreg(ESR_EL1);

	/* FIRST, before printk or anything that can fault again. */
	fault_record_store(reason, esr, far, esf != NULL ? esf->elr : 0);
	gc_result_write(GC_RESULT_FAULT, (uint32_t)esr,
			(uint32_t)(esf != NULL ? esf->elr : 0), (uint32_t)far);

	ARG_UNUSED(esf);
	printk("gc_common: fatal error %u, ESR 0x%llx FAR 0x%llx\n", reason,
	       (unsigned long long)esr, (unsigned long long)far);
#ifdef CONFIG_GC_SAMPLE_FATAL_ENCODE_ELR
	/* The faulting PC, 4 KiB windows from the image base, 70 s + 3 s per
	 * window (the return time jitters by a second or two; three seconds
	 * per step keeps the reading in the right window): look the window
	 * up in the ELF's symbol table. The watchdog is fed throughout, so a
	 * hold beyond its 128 s is fine. A fatal with no frame (z_irq_spurious
	 * passes none) encodes the interrupt instead: 70 + 3 * (INTID / 4),
	 * the INTID being whichever line is active in the GIC for this CPU;
	 * 250 when nothing is active. */
	if (esf != NULL) {
		hold = 70 + 3 * (int)(((esf->elr - CONFIG_SRAM_BASE_ADDRESS) >> 12) & 0x3f);
	} else {
		unsigned int intid = gc_sample_active_intid();

		hold = intid != 0xffffu ? 70 + 3 * (int)(intid / 4u) : 250;
	}
#endif
#ifdef CONFIG_GC_SAMPLE_FATAL_ENCODE_ELR_FINE
	/* Bits 11:6 of the faulting PC (64-byte steps inside the 4 KiB window
	 * that ENCODE_ELR found): 70 s + 3 s per step. */
	hold = esf != NULL ? 70 + 3 * (int)((esf->elr >> 6) & 0x3f) : 250 + (int)reason;
#endif
#ifdef CONFIG_GC_SAMPLE_FATAL_ENCODE_EC
	/* The exception class: 70 s + 3 s * ESR_EL1.EC, so SVC (k_panic,
	 * k_oops) 0x15 -> 133, instruction abort 0x21 -> 169, data abort
	 * 0x25 -> 181, SError 0x2f -> 211; 250 + reason without a frame. */
	hold = esf != NULL ? 70 + 3 * (int)((esr >> 26) & 0x3f) : 250 + (int)reason;
#endif
#ifdef CONFIG_GC_SAMPLE_FATAL_ENCODE_FAR
	/* No console: say where the fault was through the return time.
	 * 70 s + 3 s * bits 23:18 of the faulting address, so that a
	 * peripheral base is readable: I2C2 0x30a3.... -> 70 + 3 * 40,
	 * GPIO4 0x3023.... -> 70 + 3 * 8, IOMUXC 0x3033.... -> 70 + 3 * 12,
	 * PWM3 0x3068.... -> 70 + 3 * 26, DSIM/LCDIF/BLK 0x32e6..0x32ec....
	 * -> 70 + 3 * 57..59, GPC/CCM 0x303a/0x3038 -> 70 + 3 * 14, a RAM
	 * address -> whatever its bits 23:18 are. */
	hold = 70 + 3 * (int)((far >> 18) & 0x3f);
#endif
#ifdef CONFIG_GC_SAMPLE_FATAL_ENCODE_DFSC
	/* The fault status code of a data or instruction abort: 70 s + 3 s *
	 * ESR_EL1.ISS[5:0] (a base above the 60 s PASS hold). Translation
	 * fault level 0..3 -> 82..91, access flag 97..106, permission
	 * 109..118, synchronous external abort 118, alignment 133; the reason when there is no frame: 250 + reason. */
	hold = esf != NULL ? 70 + 3 * (int)(esr & 0x3f) : 250 + (int)reason;
#endif
	printk("gc_common: holding %d s then resetting\n", hold);
	gc_panel_console_stage(7, 0xff0000);
	for (int s = 0; s < hold; s++) {
		gc_sample_service_wdog();
		k_busy_wait(1000000);
	}
	register unsigned long x0 __asm__("x0") = 0x84000009UL;

	__asm__ volatile("smc #0" : "+r"(x0) : : "x1", "x2", "x3", "memory");
	for (;;) {
	}
}
#endif /* CONFIG_GC_SAMPLE_FATAL_HANDLER */
