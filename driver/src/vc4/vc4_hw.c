/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * VC4 V3D register sequences (vc4.h). Each function names the Linux v6.18
 * function of drivers/gpu/drm/vc4/ it follows; the code is written for this
 * driver, not taken from there.
 */
#include <errno.h>

#include "vc4.h"

static inline uint32_t rd(const struct vc4_io *io, uint32_t off)
{
	return io->read32(io->ctx, off);
}

static inline void wr(const struct vc4_io *io, uint32_t off, uint32_t val)
{
	io->write32(io->ctx, off, val);
}

/* vc4_v3d_bind's identity check (vc4_v3d.c:461-466) and the IDENT1 fields of
 * vc4_v3d_debugfs_ident (:105-116). */
int vc4_identify(const struct vc4_io *v3d, struct vc4_ident *id)
{
	id->ident0 = rd(v3d, V3D_IDENT0);
	id->ident1 = rd(v3d, V3D_IDENT1);
	id->ident2 = rd(v3d, V3D_IDENT2);
	id->rev = V3D_IDENT1_REV(id->ident1);
	id->nslc = V3D_IDENT1_NSLC(id->ident1);
	id->qups = V3D_IDENT1_QUPS(id->ident1);
	id->tups = V3D_IDENT1_TUPS(id->ident1);
	id->nsem = V3D_IDENT1_NSEM(id->ident1);
	id->vpm_kb = V3D_IDENT1_VPM_SIZE(id->ident1);
	return id->ident0 == V3D_EXPECTED_IDENT0 ? 0 : -ENODEV;
}

/* vc4_v3d_init_hw (vc4_v3d.c:162-171) and vc4_irq_enable (vc4_irq.c:261-276):
 * what vc4_v3d_runtime_resume runs after the clock is on. */
static void resume_hw(const struct vc4_io *v3d)
{
	/* all of the VPM to the GPU's own use: no user QPU programs */
	wr(v3d, V3D_VPMBASE, 0);
	/* OUTOMEM waits for bin memory (vc4_bin_mem_enable) */
	wr(v3d, V3D_INTENA, V3D_INT_FLDONE | V3D_INT_FRDONE);
}

int vc4_init(const struct vc4_io *v3d, struct vc4_ident *id)
{
	int ret = vc4_identify(v3d, id);
	if (ret)
		return ret;
	wr(v3d, V3D_VPMBASE, 0);
	/* no overflow memory left over from whoever ran before (vc4_v3d.c:468-472) */
	wr(v3d, V3D_BPOA, 0);
	wr(v3d, V3D_BPOS, 0);
	/* vc4_irq_prepare: clear what is pending (vc4_irq.c:255-258) */
	wr(v3d, V3D_INTCTL, V3D_DRIVER_IRQS);
	/* vc4_irq_enable (:272-275) */
	wr(v3d, V3D_INTENA, V3D_INT_FLDONE | V3D_INT_FRDONE);
	return 0;
}

void vc4_irq_disable(const struct vc4_io *v3d)
{
	wr(v3d, V3D_INTDIS, V3D_DRIVER_IRQS);
	wr(v3d, V3D_INTCTL, V3D_DRIVER_IRQS);
}

/* vc4_flush_caches (vc4_gem.c:435-452). */
void vc4_flush_caches_bin(const struct vc4_io *v3d)
{
	wr(v3d, V3D_L2CACTL, V3D_L2CACTL_L2CCLR);
	wr(v3d, V3D_SLCACTL,
	   V3D_SLCACTL_T1CC(0xf) | V3D_SLCACTL_T0CC(0xf) | V3D_SLCACTL_UCC(0xf) | V3D_SLCACTL_ICC(0xf));
}

/* vc4_flush_texture_caches (vc4_gem.c:454-465). */
void vc4_flush_caches_render(const struct vc4_io *v3d)
{
	wr(v3d, V3D_L2CACTL, V3D_L2CACTL_L2CCLR);
	wr(v3d, V3D_SLCACTL, V3D_SLCACTL_T1CC(0xf) | V3D_SLCACTL_T0CC(0xf));
}

static bool thread_running(const struct vc4_io *v3d, uint32_t thread)
{
	return (rd(v3d, V3D_CTNCS(thread)) & V3D_CTRUN) != 0;
}

/* submit_cl (vc4_gem.c:372-382) after the guide's rule (p. 85-86). */
int vc4_submit_cl(const struct vc4_io *v3d, uint32_t thread, uint32_t start, uint32_t end)
{
	if (thread > VC4_THREAD_RENDER)
		return -EINVAL;
	if (thread_running(v3d, thread))
		return -EBUSY;
	wr(v3d, V3D_CTNCA(thread), start);
	wr(v3d, V3D_CTNEA(thread), end); /* starts the thread */
	return 0;
}

/* ---- bin memory ---- */

#define BLOCK_256M 0x10000000ull

int vc4_bin_range_pick(uint64_t free_phys, uint64_t free_size, uint32_t size, uint32_t *phys)
{
	if (size == 0 || size % VC4_BIN_SLOT_SIZE || size / VC4_BIN_SLOT_SIZE > VC4_BIN_MAX_SLOTS)
		return -EINVAL;
	uint64_t end = free_phys + free_size;
	if (end < free_phys || end > VC4_DMA_WINDOW)
		end = VC4_DMA_WINDOW; /* the V3D sees nothing above the window */
	uint64_t p = (free_phys + VC4_BIN_ALIGN - 1) & ~(uint64_t)(VC4_BIN_ALIGN - 1);
	/* The bus alias adds 0xC0000000, a multiple of 256 MB, so a 256 MB block
	 * of the bus is one of ARM physical memory: check the physical range. */
	if ((p / BLOCK_256M) != ((p + size - 1) / BLOCK_256M))
		p = (p / BLOCK_256M + 1) * BLOCK_256M;
	if (p + size > end)
		return -ERANGE;
	*phys = (uint32_t)p;
	return 0;
}

int vc4_bin_mem_init(struct vc4_bin_mem *m, uint32_t phys, uint32_t size)
{
	uint32_t got;
	/* the buffer itself must be one vc4_bin_range_pick would choose */
	if (vc4_bin_range_pick(phys, size, size, &got) || got != phys)
		return -EINVAL;
	m->phys = phys;
	m->size = size;
	m->nslots = size / VC4_BIN_SLOT_SIZE;
	m->used = 0;
	m->overflow = 0;
	return 0;
}

int vc4_bin_slot_get(struct vc4_bin_mem *m)
{
	for (uint32_t s = 0; s < m->nslots; s++) {
		if (!(m->used & VC4_BIT(s))) {
			m->used |= VC4_BIT(s);
			return (int)s;
		}
	}
	return -ENOMEM;
}

void vc4_bin_slots_put(struct vc4_bin_mem *m, uint32_t mask)
{
	m->used &= ~mask;
}

void vc4_bin_mem_enable(const struct vc4_io *v3d)
{
	wr(v3d, V3D_INTENA, V3D_INT_OUTOMEM);
}

/* ---- one frame ---- */

/* vc4_submit_next_render_job (vc4_gem.c:517-539). */
static int start_render(const struct vc4_io *v3d, struct vc4_job *job)
{
	if (thread_running(v3d, VC4_THREAD_RENDER))
		return -EBUSY;
	vc4_flush_caches_render(v3d);
	wr(v3d, V3D_CTNCA(VC4_THREAD_RENDER), job->ct1ca);
	wr(v3d, V3D_CTNEA(VC4_THREAD_RENDER), job->ct1ea);
	job->state = VC4_JOB_RENDERING;
	return 0;
}

int vc4_job_submit(const struct vc4_io *v3d, struct vc4_job *job)
{
	if (job->state != VC4_JOB_IDLE)
		return -EINVAL;
	bool bin = job->ct0ca != job->ct0ea;
	/* the thread is checked before the clear, so a refusal writes nothing */
	if (thread_running(v3d, bin ? VC4_THREAD_BIN : VC4_THREAD_RENDER))
		return -EBUSY;
	job->last_ct0ca = 0;
	job->last_ct1ca = 0;
	vc4_flush_caches_bin(v3d);
	if (!bin)
		return start_render(v3d, job);
	wr(v3d, V3D_CTNCA(VC4_THREAD_BIN), job->ct0ca);
	wr(v3d, V3D_CTNEA(VC4_THREAD_BIN), job->ct0ea);
	job->state = VC4_JOB_BINNING;
	return 0;
}

uint32_t vc4_irq(const struct vc4_io *v3d, struct vc4_job *job)
{
	uint32_t ev = 0;
	uint32_t intctl = rd(v3d, V3D_INTCTL);

	/* acknowledge what was read; OUTOMEM stays up until its cause goes */
	wr(v3d, V3D_INTCTL, intctl);

	if (intctl & V3D_INT_OUTOMEM) {
		wr(v3d, V3D_INTDIS, V3D_INT_OUTOMEM);
		ev |= VC4_EV_OOM;
	}
	if (intctl & V3D_INT_FLDONE) {
		if (job && job->state == VC4_JOB_BINNING) {
			if (start_render(v3d, job)) {
				job->state = VC4_JOB_FAILED;
				ev |= VC4_EV_BUSY;
			} else {
				ev |= VC4_EV_RENDER_STARTED;
			}
		} else {
			ev |= VC4_EV_SPURIOUS;
		}
	}
	/* an FRDONE read together with the FLDONE that just started the render
	 * list cannot be that render's: it is reported as spurious */
	if (intctl & V3D_INT_FRDONE) {
		if (job && job->state == VC4_JOB_RENDERING && !(ev & VC4_EV_RENDER_STARTED)) {
			job->state = VC4_JOB_DONE;
			ev |= VC4_EV_FRAME_DONE;
		} else {
			ev |= VC4_EV_SPURIOUS;
		}
	}
	return ev;
}

int vc4_overflow_give(const struct vc4_io *v3d, struct vc4_bin_mem *m, struct vc4_job *job)
{
	int slot = vc4_bin_slot_get(m);
	if (slot < 0)
		return -ENOMEM;
	/* the slice the binner had is freed with the job that used it, or now
	 * when no job runs (vc4_irq.c:85-103) */
	if (m->overflow) {
		if (job && (job->state == VC4_JOB_BINNING || job->state == VC4_JOB_RENDERING))
			job->bin_slots |= m->overflow;
		else
			vc4_bin_slots_put(m, m->overflow);
	}
	m->overflow = VC4_BIT((uint32_t)slot);
	/* remove the cause first: OUTOMEM is level-like (guide p. 95) */
	wr(v3d, V3D_BPOA, vc4_bin_slot_bus(m, (uint32_t)slot));
	wr(v3d, V3D_BPOS, VC4_BIN_SLOT_SIZE);
	wr(v3d, V3D_INTCTL, V3D_INT_OUTOMEM);
	wr(v3d, V3D_INTENA, V3D_INT_OUTOMEM);
	return 0;
}

void vc4_job_retire(struct vc4_bin_mem *m, struct vc4_job *job)
{
	vc4_bin_slots_put(m, job->bin_slots);
	job->bin_slots = 0;
}

/* ---- hang and reset ---- */

enum vc4_hang vc4_hangcheck(const struct vc4_io *v3d, struct vc4_job *job)
{
	bool binning = job && job->state == VC4_JOB_BINNING;
	bool rendering = job && job->state == VC4_JOB_RENDERING;
	if (!binning && !rendering)
		return VC4_HANG_IDLE;
	uint32_t ct0ca = rd(v3d, V3D_CTNCA(VC4_THREAD_BIN));
	uint32_t ct1ca = rd(v3d, V3D_CTNCA(VC4_THREAD_RENDER));
	if ((binning && ct0ca != job->last_ct0ca) || (rendering && ct1ca != job->last_ct1ca)) {
		job->last_ct0ca = ct0ca;
		job->last_ct1ca = ct1ca;
		return VC4_HANG_PROGRESS;
	}
	return VC4_HANG_HUNG;
}

enum vc4_reset_kind vc4_reset_choose(const struct vc4_power *pw, bool prefer_thread)
{
	if (prefer_thread || !pw || !pw->on || !pw->off)
		return VC4_RESET_THREAD;
	return VC4_RESET_POWER_CYCLE;
}

/* vc4_irq_reset (vc4_irq.c:334-358): stale interrupts cleared, all three on
 * (OUTOMEM is expected first: the binner has no overflow memory now). */
static void irq_reset(const struct vc4_io *v3d)
{
	wr(v3d, V3D_INTCTL, V3D_DRIVER_IRQS);
	wr(v3d, V3D_INTENA, V3D_DRIVER_IRQS);
}

static void cancel(struct vc4_bin_mem *m, struct vc4_job *job)
{
	if (m) {
		if (job)
			vc4_job_retire(m, job);
		vc4_bin_slots_put(m, m->overflow);
		m->overflow = 0;
	}
	if (job)
		job->state = VC4_JOB_FAILED;
}

int vc4_reset(const struct vc4_io *v3d, const struct vc4_power *pw, enum vc4_reset_kind kind,
	      struct vc4_job *job, struct vc4_bin_mem *m)
{
	int ret;

	if (kind == VC4_RESET_THREAD) {
		uint32_t thread;
		if (job && job->state == VC4_JOB_BINNING)
			thread = VC4_THREAD_BIN;
		else if (job && job->state == VC4_JOB_RENDERING)
			thread = VC4_THREAD_RENDER;
		else
			return -EINVAL;
		wr(v3d, V3D_CTNCS(thread), V3D_CTRSTA);
		wr(v3d, V3D_BPOA, 0);
		wr(v3d, V3D_BPOS, 0);
		irq_reset(v3d);
		cancel(m, job);
		return 0;
	}

	if (!pw || !pw->on || !pw->off)
		return -EINVAL;
	/* vc4_v3d_runtime_suspend (vc4_v3d.c:372-382) */
	vc4_irq_disable(v3d);
	ret = pw->off(pw->ctx);
	if (ret)
		return ret;
	/* vc4_v3d_runtime_resume (:384-399) */
	ret = pw->on(pw->ctx);
	if (ret)
		return ret;
	resume_hw(v3d);
	irq_reset(v3d);
	cancel(m, job);
	return 0;
}
