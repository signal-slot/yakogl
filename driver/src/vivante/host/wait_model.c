/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * wait_model.c: gc_render_wait() (gc_render.c, compiled unchanged against
 * shim/) on a simulated clock, with the interrupt scripted. Each case submits
 * one fence, schedules what the GPU's interrupt does and when, waits, and
 * prints how long the wait took on the model clock, what it returned and the
 * blocks it counted (gc_render_perf wait_sleeps):
 *
 *   before   the fence retired before the wait: no wait at all
 *   woken    the interrupt retires it 300 us in: the wait ends at 300 us,
 *            one block (the 1 ms sleep poll ended it at 1000 us)
 *   stale    the same, with a give left over from an earlier interrupt: one
 *            extra pass, still 300 us
 *   lost     fence_done moves 300 us in with no give reaching this waiter
 *            (another waiter took the give): the 1 ms poll ends it at
 *            1000 us
 *   fault    an AXI or MMU error interrupt 300 us in: -EIO at 300 us
 *   forever  a GC_WAIT_FOREVER wait woken 2300 us in: two polls run out,
 *            the third block is woken
 *   timeout  nothing retires in a 20 ms wait: -ECANCELED after the recovery
 *            at 20 ms, fence_timeouts and resets move by one
 *
 * With --table it prints the rows without the check (run.sh, for an older
 * gc_render.c); otherwise a row off its expected value fails the run.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "gc_internal.h"

/* An older gc_render.c has no wake: the scripted interrupt skips it. */
#pragma weak gc_render_irq_wake

/* --- the shim's clock and log --- */
int64_t model_us;
unsigned int model_log_errors;
bool model_log_quiet = true;

/* One scripted interrupt: at `at_us` the GPU retires through `done` (0: no
 * fence moves), raises `error` (an AXI bus error or MMU exception), and the
 * ISR gives the wake unless `lost`. */
struct model_irq {
	int64_t at_us;
	uint32_t done;
	bool error;
	bool lost;
};
static struct model_irq s_irq[4];
static unsigned int s_nirq, s_next_irq;

int64_t model_advance(int64_t until_us, struct k_sem *sem)
{
	struct gc_dev *g = gc_dev_get();

	while (s_next_irq < s_nirq && s_irq[s_next_irq].at_us <= until_us) {
		const struct model_irq *e = &s_irq[s_next_irq++];

		model_us = e->at_us;
		/* what yd_gc_irq_handle() does, then the shim's give */
		if (e->done != 0 && (int32_t)(e->done - g->fence_done) > 0) {
			g->fence_done = e->done;
		}
		if (e->error) {
			g->irq_errors++;
		}
		if (!e->lost && gc_render_irq_wake != NULL) {
			gc_render_irq_wake(g);
		}
		if (sem != NULL && sem->count > 0u) {
			return model_us;
		}
	}
	return until_us;
}

/* --- what gc_render.c calls in the rest of the driver --- */
static uint8_t *s_carveout;

void k_mem_map_phys_bare(uint8_t **virt, uintptr_t phys, size_t size, uint32_t flags)
{
	ARG_UNUSED(phys);
	ARG_UNUSED(flags);
	if (s_carveout == NULL) {
		s_carveout = calloc(1, size);
	}
	*virt = s_carveout;
}
bool gc_alive(const struct device *dev)
{
	return dev != NULL;
}
int gc_bringup_reset(mm_reg_t regs, const struct gc_identity *idp)
{
	ARG_UNUSED(regs);
	ARG_UNUSED(idp);
	return 0;
}
int gc_hw_init(struct gc_dev *g)
{
	ARG_UNUSED(g);
	return 0;
}
int gc_mmu_init(struct gc_dev *g)
{
	ARG_UNUSED(g);
	return 0;
}
int gc_mmu_map(struct gc_dev *g, uint32_t gpu_va, uint64_t phys, size_t size)
{
	ARG_UNUSED(g);
	ARG_UNUSED(gpu_va);
	ARG_UNUSED(phys);
	ARG_UNUSED(size);
	return 0;
}
int gc_mmu_enable(struct gc_dev *g)
{
	ARG_UNUSED(g);
	return 0;
}
void gc_irq_connect(struct gc_dev *g)
{
	ARG_UNUSED(g);
}
int gc_ring_init(struct gc_dev *g)
{
	ARG_UNUSED(g);
	return 0;
}
int gc_ring_submit(struct gc_dev *g, const struct gc_submit *s, uint32_t event, uint32_t fence)
{
	ARG_UNUSED(g);
	ARG_UNUSED(s);
	ARG_UNUSED(event);
	ARG_UNUSED(fence);
	return 0;
}
void gc_ring_dump(struct gc_dev *g)
{
	ARG_UNUSED(g);
}
void gc_mmu_dump(struct gc_dev *g)
{
	ARG_UNUSED(g);
}
void gc_idle_state_decode(uint32_t idle)
{
	ARG_UNUSED(idle);
}
int gc_event_alloc(struct gc_dev *g, uint32_t fence, uint32_t timeout_ms)
{
	ARG_UNUSED(g);
	ARG_UNUSED(fence);
	ARG_UNUSED(timeout_ms);
	return 0;
}

/* --- the cases --- */
struct model_case {
	const char *name;
	bool done_before;  /* the fence retired before the wait */
	bool stale_give;   /* a give is pending when the wait starts */
	uint32_t timeout_ms;
	struct model_irq irq; /* at_us 0: no interrupt; done 1 means "the fence" */
	/* expected */
	int rc;
	int64_t us;
	uint32_t sleeps;
	uint32_t waits;
	uint32_t timeouts;
	int64_t wait_us; /* gc_render_perf wait_ns / 1000 */
};

static const struct model_case s_cases[] = {
	{ "before", true, false, 2000, { 0 }, 0, 0, 0, 0, 0, 0 },
	{ "woken", false, false, 2000, { 300, 1, false, false }, 0, 300, 1, 1, 0, 300 },
	{ "stale", false, true, 2000, { 300, 1, false, false }, 0, 300, 2, 1, 0, 300 },
	{ "lost", false, false, 2000, { 300, 1, false, true }, 0, 1000, 1, 1, 0, 1000 },
	/* the -EIO return adds nothing to wait_ns (nor wait_ms), as before */
	{ "fault", false, false, 2000, { 300, 0, true, false }, -EIO, 300, 1, 1, 0, 0 },
	{ "forever", false, false, GC_WAIT_FOREVER, { 2300, 1, false, false }, 0, 2300, 3, 1, 0,
	  2300 },
	{ "timeout", false, false, 20, { 0 }, -ECANCELED, 20000, 20, 1, 1, 20000 },
};

int main(int argc, char **argv)
{
	static const struct device dev = { "gc7000" };
	static const struct gc_identity id = { .model = 0x7000, .revision = 0x6204 };
	struct gc_dev *g = gc_dev_get();
	bool check = !(argc > 1 && strcmp(argv[1], "--table") == 0);
	int fail = 0;

	gc_dev_publish(&dev, 0x38000000u, 0x58000000u, 8u << 20, &id);
	if (gc_render_init_dev(g) != 0) {
		printf("FAIL  gc_render_init_dev\n");
		return 1;
	}
	for (size_t i = 0; i < sizeof(s_cases) / sizeof(s_cases[0]); i++) {
		const struct model_case *c = &s_cases[i];
		struct gc_render_perf p0, p1;
		uint32_t fence = g->fence_done + 1;
		uint32_t to0 = g->fence_timeouts, to1;
		int64_t t0;
		int rc;

		/* one fence submitted and outstanding */
		g->fence_next = fence + 1;
		if (c->done_before) {
			g->fence_done = fence;
		}
		/* the wake as a fresh wait finds it: empty, or one stale give */
		if (gc_render_irq_wake != NULL) {
			while (k_sem_take(&g->fence_wake, K_NO_WAIT) == 0) {
			}
			if (c->stale_give) {
				gc_render_irq_wake(g);
			}
		}
		s_nirq = 0;
		s_next_irq = 0;
		if (c->irq.at_us != 0) {
			s_irq[0] = c->irq;
			s_irq[0].at_us += model_us;
			s_irq[0].done = c->irq.done != 0 ? fence : 0;
			s_nirq = 1;
		}
		gc_render_perf(&p0);
		t0 = model_us;
		rc = gc_render_wait(fence, c->timeout_ms);
		gc_render_perf(&p1);
		to1 = g->fence_timeouts;

		char line[160];
		int64_t us = model_us - t0;
		uint32_t sleeps = p1.wait_sleeps - p0.wait_sleeps;
		uint32_t waits = p1.waits - p0.waits;

		snprintf(line, sizeof(line),
			 "case=%-8s rc=%d us=%lld sleeps=%u waits=%u timeouts=%u wait_ns=%llu",
			 c->name, rc, (long long)us, sleeps, waits, to1 - to0,
			 (unsigned long long)(p1.wait_ns - p0.wait_ns));
		if (!check) {
			printf("      %s\n", line);
		} else if (rc == c->rc && us == c->us && sleeps == c->sleeps && waits == c->waits &&
			   to1 - to0 == c->timeouts &&
			   p1.wait_ns - p0.wait_ns == (uint64_t)c->wait_us * 1000u) {
			printf("ok    %s\n", line);
		} else {
			printf("FAIL  %s (expected rc=%d us=%lld sleeps=%u waits=%u timeouts=%u "
			       "wait_ns=%llu)\n",
			       line, c->rc, (long long)c->us, c->sleeps, c->waits, c->timeouts,
			       (unsigned long long)c->wait_us * 1000u);
			fail = 1;
		}
		/* the next case starts on its own millisecond */
		model_us = (model_us / 1000 + 1) * 1000;
		if (g->fence_done != fence) {
			g->fence_done = fence;
		}
	}
	return fail;
}
