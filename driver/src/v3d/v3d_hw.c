/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Broadcom V3D 4.2 (Raspberry Pi 4) bring-up: power, identity, MMU, caches and
 * the bin/render submit (see v3d.h). The registers are v3d_regs.h. Each
 * sequence says what the hardware needs from it; where Mesa's V3D simulator
 * glue (src/broadcom/simulator/v3dx_simulator.c, MIT) programs the same
 * thing, its function is named, and where the order is ours to choose it
 * says so. tools/v3d/hw_mock_test.c pins every write and its order.
 */
#include <zephyr/kernel.h>
#include <zephyr/sys/sys_io.h>
#include <zephyr/logging/log.h>
#include <errno.h>

#include "v3d.h"
#include "v3d_regs.h"

LOG_MODULE_REGISTER(gpu_v3d, LOG_LEVEL_INF);

#define HUB_RD(r, o)     sys_read32((r)->hub + (o))
#define HUB_WR(r, o, v)  sys_write32((v), (r)->hub + (o))
#define CORE_RD(r, o)    sys_read32((r)->core + (o))
#define CORE_WR(r, o, v) sys_write32((v), (r)->core + (o))

/* Poll until (read & mask) == want, up to timeout_us. */
static int poll32(mm_reg_t addr, uint32_t mask, uint32_t want, uint32_t timeout_us)
{
	for (uint32_t t = 0;; t++) {
		if ((sys_read32(addr) & mask) == want)
			return 0;
		if (t >= timeout_us)
			return -ETIMEDOUT;
		k_busy_wait(1);
	}
}

/* Start one port of the V3D's AXI bridge: its stop request cleared (the
 * write key with the rest of the register kept), then the acknowledge must
 * fall. The bridge answers within a microsecond or not at all. */
static int asb_enable(const struct v3d_regs *r, uint32_t reg)
{
	uint32_t val = sys_read32(r->asb + reg) & ~BCM2711_ASB_CTL_STOP_REQUEST_SET;
	sys_write32(BCM2711_PM_WRITE_KEY | val, r->asb + reg);
	return poll32(r->asb + reg, BCM2711_ASB_CTL_STOP_ACK_SET, 0, 1);
}

/* Stop one port: the stop request set, then the acknowledge must rise, within
 * the same microsecond. */
static int asb_disable(const struct v3d_regs *r, uint32_t reg)
{
	uint32_t val = sys_read32(r->asb + reg) | BCM2711_ASB_CTL_STOP_REQUEST_SET;
	sys_write32(BCM2711_PM_WRITE_KEY | val, r->asb + reg);
	return poll32(r->asb + reg, BCM2711_ASB_CTL_STOP_ACK_SET, BCM2711_ASB_CTL_STOP_ACK_SET, 1);
}

int v3d_power_off(const struct v3d_regs *r, const struct v3d_clock_ops *clk)
{
	/* Nothing may be in flight through the bridge when the V3D loses its
	 * clock: the slave port (the CPU's side) stops first, then the master
	 * (the GPU's own memory traffic); a master that does not stop leaves
	 * the slave started again, so the registers stay reachable. Then the
	 * clock off and the reset asserted. */
	int ret = asb_disable(r, BCM2711_ASB_V3D_SLAVE_CTL);
	if (ret) {
		LOG_ERR("V3D ASB slave did not stop");
		return ret;
	}
	ret = asb_disable(r, BCM2711_ASB_V3D_MASTER_CTL);
	if (ret) {
		LOG_ERR("V3D ASB master did not stop");
		(void)asb_enable(r, BCM2711_ASB_V3D_SLAVE_CTL);
		return ret;
	}
	ret = clk->set(clk->ctx, false);
	sys_write32(BCM2711_PM_WRITE_KEY | (sys_read32(r->pm + BCM2711_PM_GRAPHICS_CTL) & ~BCM2711_PM_GRAPHICS_V3D_RESET_N_SET), r->pm + BCM2711_PM_GRAPHICS_CTL);
	return ret;
}

int v3d_reset(const struct v3d_regs *r, const struct v3d_clock_ops *clk, uint64_t pt_pa, uint64_t scratch_pa)
{
	/* On the BCM2711 the V3D's only reset is the power manager's reset
	 * line, so a reset is a power cycle, and everything the GPU keeps in
	 * registers is lost with it: interrupts masked first (a dying job must
	 * not raise one), off, on, then the cache range and the MMU set again
	 * and whatever the cycle left pending cleared. The page table itself
	 * is in memory and survives, so the GPU sees the same mappings. */
	v3d_irq_quiesce(r);
	int ret = v3d_power_off(r, clk);
	if (ret)
		return ret;
	ret = v3d_power_on(r, clk);
	if (ret)
		return ret;
	v3d_caches_init(r);
	ret = v3d_mmu_init(r, pt_pa, scratch_pa);
	v3d_irq_quiesce(r);
	return ret;
}

int v3d_power_on(const struct v3d_regs *r, const struct v3d_clock_ops *clk)
{
	uint32_t id = sys_read32(r->asb + BCM2711_ASB_ID);
	if (id != BCM2711_ASB_ID_VALUE) {
		LOG_ERR("V3D ASB bridge ID 0x%08x, expected 0x%08x", id, BCM2711_ASB_ID_VALUE);
		return -ENODEV;
	}

	/* The reset must be seen by a clocked block: the clock runs a moment
	 * (a microsecond covers the 32 cycles the reset needs) with the reset
	 * still asserted, stops, the reset is released, the clock runs again,
	 * and only then is the bridge opened, the master port (the GPU's side)
	 * before the slave (the CPU's). */
	int ret = clk->set(clk->ctx, true);
	if (ret)
		return ret;
	k_busy_wait(1);
	ret = clk->set(clk->ctx, false);
	if (ret)
		return ret;

	sys_write32(BCM2711_PM_WRITE_KEY | sys_read32(r->pm + BCM2711_PM_GRAPHICS_CTL) | BCM2711_PM_GRAPHICS_V3D_RESET_N_SET, r->pm + BCM2711_PM_GRAPHICS_CTL);

	ret = clk->set(clk->ctx, true);
	if (ret)
		goto err_reset;
	ret = asb_enable(r, BCM2711_ASB_V3D_MASTER_CTL);
	if (ret) {
		LOG_ERR("V3D ASB master did not acknowledge");
		goto err_clock;
	}
	ret = asb_enable(r, BCM2711_ASB_V3D_SLAVE_CTL);
	if (ret) {
		LOG_ERR("V3D ASB slave did not acknowledge");
		goto err_clock;
	}
	return 0;

err_clock:
	clk->set(clk->ctx, false);
err_reset:
	sys_write32(BCM2711_PM_WRITE_KEY | (sys_read32(r->pm + BCM2711_PM_GRAPHICS_CTL) & ~BCM2711_PM_GRAPHICS_V3D_RESET_N_SET), r->pm + BCM2711_PM_GRAPHICS_CTL);
	return ret;
}

int v3d_identify(const struct v3d_regs *r, struct v3d_identity *id)
{
	for (int i = 0; i < 4; i++)
		id->hub_ident[i] = HUB_RD(r, V3D_HUB_CTL_IDENT0 + 4 * i);
	for (int i = 0; i < 3; i++)
		id->ctl_ident[i] = CORE_RD(r, V3D_CTL_0_IDENT0 + 4 * i);
	id->mmu_debug_info = HUB_RD(r, V3D_MMU0_DEBUG_INFO);

	uint32_t h1 = id->hub_ident[1], c1 = id->ctl_ident[1];
	id->ver = V3D_HUB_CTL_IDENT1_TVER_GET(h1) * 10 + V3D_HUB_CTL_IDENT1_REV_GET(h1);
	id->ncores = V3D_HUB_CTL_IDENT1_NCORES_GET(h1);
	id->qpu_count = V3D_CTL_0_IDENT1_NSLC_GET(c1) * V3D_CTL_0_IDENT1_QUPS_GET(c1);
	id->vpm_size = V3D_CTL_0_IDENT1_VPM_SIZE_GET(c1) * 8192;
	id->pa_width = 30 + V3D_MMU0_DEBUG_INFO_PA_WIDTH_GET(id->mmu_debug_info);
	id->va_width = 30 + V3D_MMU0_DEBUG_INFO_VA_WIDTH_GET(id->mmu_debug_info);
	id->has_mmu = (id->hub_ident[2] & V3D_HUB_CTL_IDENT2_WITH_MMU_SET) != 0;

	LOG_INF("V3D %u.%u: %u core(s), %u QPUs, VPM %u bytes, PA %u / VA %u bits, MMU %s",
		id->ver / 10, id->ver % 10, id->ncores, id->qpu_count, id->vpm_size,
		id->pa_width, id->va_width, id->has_mmu ? "yes" : "no");
	LOG_INF("V3D raw: HUB_IDENT0..3 %08x %08x %08x %08x CTL_IDENT0..2 %08x %08x %08x "
		"MMU_DEBUG_INFO %08x",
		id->hub_ident[0], id->hub_ident[1], id->hub_ident[2], id->hub_ident[3],
		id->ctl_ident[0], id->ctl_ident[1], id->ctl_ident[2], id->mmu_debug_info);

	/* everything in this driver is for a V3D 4.2 with one core and an
	 * MMU, what the BCM2711 has: anything else is refused */
	if (id->ver != 42 || id->ncores != 1 || !id->has_mmu) {
		LOG_ERR("not a single-core V3D 4.2 with an MMU");
		return -ENODEV;
	}
	return 0;
}

int v3d_pte_map(uint32_t *pt, uint32_t pt_entries, uint32_t va, uint64_t pa, uint32_t bytes,
		bool writeable)
{
	const uint32_t page_mask = (1u << V3D_MMU_PAGE_SHIFT) - 1;
	uint32_t first = va >> V3D_MMU_PAGE_SHIFT;
	uint32_t pages = (uint32_t)(((uint64_t)bytes + page_mask) >> V3D_MMU_PAGE_SHIFT);
	uint64_t last_pfn = (pa >> V3D_MMU_PAGE_SHIFT) + pages - 1;

	/* VA 0 stays unmapped, so that a null GPU address faults; pages are
	 * whole, and the page number must fit the entry's 24 bits */
	if (va == 0 || (va & page_mask) || (pa & page_mask) || pages == 0 ||
	    (uint64_t)first + pages > pt_entries || last_pfn > V3D_MMU_PTE_PFN_MASK)
		return -EINVAL;

	uint32_t flags = V3D_MMU_PTE_VALID_SET | (writeable ? V3D_MMU_PTE_WRITABLE_SET : 0);
	for (uint32_t i = 0; i < pages; i++)
		pt[first + i] = flags | (uint32_t)((pa >> V3D_MMU_PAGE_SHIFT) + i);
	return 0;
}

int v3d_mmu_flush(const struct v3d_regs *r)
{
	/* The MMU cache holds table entries and the TLB translations made from
	 * them: the cache is flushed first (it stays enabled), then the TLB is
	 * cleared, so that no translation is rebuilt from a stale entry. Each
	 * reports its end in a bit of its own control register. */
	HUB_WR(r, V3D_MMU0_CACHE_CTL, V3D_MMU0_CACHE_CTL_FLUSH_SET | V3D_MMU0_CACHE_CTL_ENABLE_SET);
	if (poll32(r->hub + V3D_MMU0_CACHE_CTL, V3D_MMU0_CACHE_CTL_FLUSHING_SET, 0, 100000)) {
		LOG_ERR("MMU cache flush timed out");
		return -ETIMEDOUT;
	}
	HUB_WR(r, V3D_MMU0_CTL, HUB_RD(r, V3D_MMU0_CTL) | V3D_MMU0_CTL_TLB_CLEAR_SET);
	if (poll32(r->hub + V3D_MMU0_CTL, V3D_MMU0_CTL_TLB_CLEARING_SET, 0, 100000)) {
		LOG_ERR("MMU TLB clear timed out");
		return -ETIMEDOUT;
	}
	return 0;
}

int v3d_mmu_init(const struct v3d_regs *r, uint64_t pt_pa, uint64_t scratch_pa)
{
	/* The table's page, then the MMU on with every fault both aborting the
	 * access and raising its interrupt, a faulting access sent to the
	 * scratch page, the MMU cache on, and both flushed: the order the
	 * hardware needs is only that the table be named before the MMU is
	 * on and flushed after. */
	HUB_WR(r, V3D_MMU0_PT_PA_BASE, (uint32_t)(pt_pa >> V3D_MMU_PAGE_SHIFT));
	HUB_WR(r, V3D_MMU0_CTL,
	       V3D_MMU0_CTL_ENABLE_SET | V3D_MMU0_CTL_PT_INVALID_ENABLE_SET |
		       V3D_MMU0_CTL_PT_INVALID_ABORT_SET | V3D_MMU0_CTL_PT_INVALID_INT_SET |
		       V3D_MMU0_CTL_WRITE_VIOLATION_ABORT_SET | V3D_MMU0_CTL_WRITE_VIOLATION_INT_SET |
		       V3D_MMU0_CTL_CAP_EXCEEDED_ABORT_SET | V3D_MMU0_CTL_CAP_EXCEEDED_INT_SET);
	HUB_WR(r, V3D_MMU0_ILLEGAL_ADR,
	       (uint32_t)(scratch_pa >> V3D_MMU_PAGE_SHIFT) | V3D_MMU0_ILLEGAL_ADR_ENABLE_SET);
	HUB_WR(r, V3D_MMU0_CACHE_CTL, V3D_MMU0_CACHE_CTL_ENABLE_SET);
	return v3d_mmu_flush(r);
}

void v3d_caches_init(const struct v3d_regs *r)
{
	/* The texture L2's operations work on the range L2TFLSTA..L2TFLEND:
	 * set once to the whole address space, which the registers keep until
	 * the next power cycle (Mesa's simulator glue sets it before each
	 * operation, v3d_invalidate_l2t and v3d_flush_l2t). */
	CORE_WR(r, V3D_CTL_0_L2TFLSTA, 0);
	CORE_WR(r, V3D_CTL_0_L2TFLEND, ~0u);
}

void v3d_caches_invalidate(const struct v3d_regs *r)
{
	/* What the GPU may hold stale from memory the CPU wrote: the texture
	 * L2 flushed (mode 0, flush) and the slice caches invalidated, as
	 * Mesa's v3d_invalidate_caches does for 4.2 (v3d_invalidate_l2t,
	 * v3d_invalidate_slices; the L2C is gone on 4.x). The L2T operation
	 * is not waited for: the GPU's own accesses wait for it. */
	CORE_WR(r, V3D_CTL_0_L2TCACTL, V3D_CTL_0_L2TCACTL_L2TFLS_SET);
	CORE_WR(r, V3D_CTL_0_SLCACTL, V3D_CTL_0_SLCACTL_ALL);
}

int v3d_caches_clean(const struct v3d_regs *r)
{
	/* What the GPU wrote, out to memory for the CPU: the TMU's write
	 * combiner into the L2T, then the L2T cleaned (mode 2), each waited
	 * for; Mesa's v3d_flush_caches (v3d_flush_l1td, v3d_flush_l2t). */
	CORE_WR(r, V3D_CTL_0_L2TCACTL, V3D_CTL_0_L2TCACTL_TMUWCF_SET);
	if (poll32(r->core + V3D_CTL_0_L2TCACTL, V3D_CTL_0_L2TCACTL_TMUWCF_SET, 0, 100000))
		return -ETIMEDOUT;
	CORE_WR(r, V3D_CTL_0_L2TCACTL, V3D_CTL_0_L2TCACTL_L2TFLS_SET | (V3D_CACHE_FLUSH_MODE_CLEAN << V3D_CTL_0_L2TCACTL_L2TFLM_LSB));
	if (poll32(r->core + V3D_CTL_0_L2TCACTL, V3D_CTL_0_L2TCACTL_L2TFLS_SET, 0, 100000))
		return -ETIMEDOUT;
	return 0;
}

void v3d_irq_quiesce(const struct v3d_regs *r)
{
	/* every interrupt of the core and the hub masked, and whatever is
	 * pending cleared; this driver polls the status registers */
	CORE_WR(r, V3D_CTL_0_INT_MSK_SET, ~0u);
	CORE_WR(r, V3D_CTL_0_INT_CLR, ~0u);
	HUB_WR(r, V3D_HUB_CTL_INT_MSK_SET, ~0u);
	HUB_WR(r, V3D_HUB_CTL_INT_CLR, ~0u);
}

void v3d_submit_bin(const struct v3d_regs *r, uint32_t bcl_start, uint32_t bcl_end,
		    uint32_t tile_alloc, uint32_t tile_alloc_size, uint32_t tile_state)
{
	/* a done or out-of-memory bit left by a job that ended late is not
	 * this job's: cleared before the binner starts */
	CORE_WR(r, V3D_CTL_0_INT_CLR, V3D_CTL_0_INT_STS_INT_FLDONE_SET | V3D_CTL_0_INT_STS_INT_OUTOMEM_SET);
	/* no overflow block yet (one is given when the binner asks, see
	 * wait_bits), the caches invalidated, then Mesa's
	 * v3dX(simulator_submit_cl_ioctl) order: the tile allocation memory,
	 * the tile state with CTQTSEN, the list's base and its end, whose
	 * write starts the binner */
	CORE_WR(r, V3D_PTB_0_BPOS, 0);
	v3d_caches_invalidate(r);
	CORE_WR(r, V3D_CLE_0_CT0QMA, tile_alloc);
	CORE_WR(r, V3D_CLE_0_CT0QMS, tile_alloc_size);
	CORE_WR(r, V3D_CLE_0_CT0QTS, V3D_CLE_0_CT0QTS_CTQTSEN_SET | tile_state);
	CORE_WR(r, V3D_CLE_0_CT0QBA, bcl_start);
	CORE_WR(r, V3D_CLE_0_CT0QEA, bcl_end); /* starts the binner */
}

void v3d_submit_render(const struct v3d_regs *r, uint32_t rcl_start, uint32_t rcl_end)
{
	/* a done bit left by a job that ended late is not this job's */
	CORE_WR(r, V3D_CTL_0_INT_CLR, V3D_CTL_0_INT_STS_INT_FRDONE_SET);
	/* the caches invalidated again (the binner's output is in memory),
	 * then the list's base and end, as v3dX(simulator_submit_cl_ioctl) */
	v3d_caches_invalidate(r);
	CORE_WR(r, V3D_CLE_0_CT1QBA, rcl_start);
	CORE_WR(r, V3D_CLE_0_CT1QEA, rcl_end); /* starts the renderer */
}

static void snapshot(const struct v3d_regs *r, uint32_t core_sts, uint32_t hub_sts,
		     struct v3d_stall *s)
{
	if (!s)
		return;
	s->core_int_sts = core_sts;
	s->hub_int_sts = hub_sts;
	s->ct0ca = CORE_RD(r, V3D_CLE_0_CT0CA);
	s->ct0ra = CORE_RD(r, V3D_CLE_0_CT0RA);
	s->ct1ca = CORE_RD(r, V3D_CLE_0_CT1CA);
	s->ct1ra = CORE_RD(r, V3D_CLE_0_CT1RA);
	s->err_stat = CORE_RD(r, V3D_ERR_0_STAT);
	s->mmu_ctl = HUB_RD(r, V3D_MMU0_CTL);
	s->mmu_vio_addr = HUB_RD(r, V3D_MMU0_VIO_ADDR);
	s->mmu_vio_id = HUB_RD(r, V3D_MMU0_VIO_ID);
}

static int wait_bits(const struct v3d_regs *r, uint32_t bits, uint32_t timeout_us, struct v3d_stall *stall,
		     struct v3d_bin_overflow *ov)
{
	uint32_t seen = 0;
	const uint32_t mmu_faults = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET | V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET;

	/* only the bits this wait is for (and the binner's out-of-memory, which
	 * only binning raises): another stage's done bit stays for that stage,
	 * and a stale one is cleared by its own submit before it starts
	 * (v3d_submit_bin / v3d_submit_render). Clearing every bit seen let a
	 * late job's done bit reach the next job's wait. */
	const uint32_t own = bits | V3D_CTL_0_INT_STS_INT_OUTOMEM_SET;

	for (uint32_t t = 0;; t++) {
		uint32_t core = CORE_RD(r, V3D_CTL_0_INT_STS) & own;
		uint32_t hub = HUB_RD(r, V3D_HUB_CTL_INT_STS);
		if (core)
			CORE_WR(r, V3D_CTL_0_INT_CLR, core);
		seen |= core;
		if (hub & mmu_faults) {
			snapshot(r, seen, hub, stall);
			HUB_WR(r, V3D_HUB_CTL_INT_CLR, hub);
			LOG_ERR("V3D MMU fault: hub INT_STS 0x%08x", hub);
			return -EFAULT;
		}
		if (core & V3D_CTL_0_INT_STS_INT_OUTOMEM_SET) {
			/* the next overflow block, address then size, as Mesa's
			 * v3d_isr_core gives the binner more memory; the
			 * interrupt was cleared above and is edge-signalled */
			if (ov && ov->used < ov->nblocks) {
				CORE_WR(r, V3D_PTB_0_BPOA, ov->blocks[ov->used]);
				CORE_WR(r, V3D_PTB_0_BPOS, ov->block_size);
				ov->used++;
				seen &= ~V3D_CTL_0_INT_STS_INT_OUTOMEM_SET;
			} else {
				snapshot(r, seen, hub, stall);
				LOG_ERR("V3D binner out of memory (%u overflow blocks given)",
					ov ? ov->used : 0u);
				return -ENOMEM;
			}
		}
		if ((seen & bits) == bits)
			return 0;
		if (t >= timeout_us) {
			snapshot(r, seen, hub, stall);
			LOG_ERR("V3D wait for 0x%x timed out after %u us, seen 0x%x", bits,
				timeout_us, seen);
			return -ETIMEDOUT;
		}
		k_busy_wait(1);
	}
}

void v3d_stall_snapshot(const struct v3d_regs *r, struct v3d_stall *stall)
{
	snapshot(r, CORE_RD(r, V3D_CTL_0_INT_STS), HUB_RD(r, V3D_HUB_CTL_INT_STS), stall);
}

int v3d_wait(const struct v3d_regs *r, uint32_t bits, uint32_t timeout_us, struct v3d_stall *stall)
{
	return wait_bits(r, bits, timeout_us, stall, NULL);
}

int v3d_wait_bin(const struct v3d_regs *r, uint32_t timeout_us, struct v3d_stall *stall,
		 struct v3d_bin_overflow *ov)
{
	return wait_bits(r, V3D_CTL_0_INT_STS_INT_FLDONE_SET, timeout_us, stall, ov);
}

int v3d_tfu_run(const struct v3d_regs *r, const struct v3d_tfu_job *j, uint32_t timeout_us,
		struct v3d_tfu_status *st)
{
	struct v3d_tfu_status none;
	const uint32_t mmu_bits = V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET | V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET | V3D_HUB_CTL_INT_STS_INT_MMU_CAP_SET;

	if (!st)
		st = &none;
	*st = (struct v3d_tfu_status){ 0 };
	st->cs_before = HUB_RD(r, V3D_TFU_CS);
	if (st->cs_before & V3D_TFU_CS_BUSY_SET)
		return -EBUSY;
	/* a stale completion must not end this wait */
	HUB_WR(r, V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET);

	/* The job's registers in the order of Mesa's
	 * v3dX(simulator_submit_tfu_ioctl) for 4.2 (no IOC register; the
	 * interrupt on completion is ICFG's IOC bit), ICFG last since its
	 * write starts the job. COEF1..3 are not written: USECOEF is kept
	 * clear, so the TFU does not read them. */
	HUB_WR(r, V3D_TFU_IIA, j->iia);
	HUB_WR(r, V3D_TFU_IIS, j->iis);
	HUB_WR(r, V3D_TFU_ICA, j->ica);
	HUB_WR(r, V3D_TFU_IUA, j->iua);
	HUB_WR(r, V3D_TFU_IOA, j->ioa);
	HUB_WR(r, V3D_TFU_IOS, j->ios);
	HUB_WR(r, V3D_TFU_COEF0, j->coef0 & ~V3D_TFU_COEF0_USECOEF_SET);
	HUB_WR(r, V3D_TFU_ICFG, j->icfg | V3D_TFU_ICFG_IOC_SET);

	int rc = -ETIMEDOUT;
	for (uint32_t t = 0;; t++) {
		uint32_t sts = HUB_RD(r, V3D_HUB_CTL_INT_STS);

		st->hub_int_sts = sts;
		st->us = t;
		if (sts & mmu_bits) {
			rc = -EFAULT;
			break;
		}
		if (sts & V3D_HUB_CTL_INT_STS_INT_TFUC_SET) {
			rc = 0;
			break;
		}
		if (t >= timeout_us)
			break;
		k_busy_wait(1);
	}
	st->cs_after = HUB_RD(r, V3D_TFU_CS);
	HUB_WR(r, V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET);
	if (rc != 0)
		return rc; /* TFURST is not used: the caller resets the whole GPU */
	if (rc == 0 &&
	    ((V3D_TFU_CS_CVTCT_GET(st->cs_after) - V3D_TFU_CS_CVTCT_GET(st->cs_before)) & 0xff) != 1)
		return -EIO;
	return rc;
}
