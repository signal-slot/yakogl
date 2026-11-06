/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hw_mock_test: runs driver/src/v3d/v3d_hw.c against a software model of the
 * V3D, PM and ASB registers and checks what it writes, in what order, and how
 * it reports each failure. The model only answers what the sequences
 * wait for (the bridge ACK drops, flushes finish, a job raises its done bit);
 * it proves the driver follows the sequence it claims, not that the silicon
 * behaves as modelled -- only the board can say that.
 *
 *   cc -std=c11 -Wall -Wextra -Itools/v3d/hostshim -Idriver/src/v3d \
 *      tools/v3d/hw_mock_test.c driver/src/v3d/v3d_hw.c -o hw_mock_test
 *
 * Exit status 0 only when every check passed and the number of checks run is
 * the number planned.
 *
 * With --v71 it runs the V3D 7.1 (BCM2712) path instead: the same model
 * answering as a 7.1 with its SMS block and pm's 2712 reset register, and
 * no bridge (tools/v3d/check71.d/hw_mock.sh). Without it the output is the
 * 4.2 run's, unchanged.
 */
#include <errno.h>
#include <stdio.h>
#include <stdint.h>
#include <stdbool.h>
#include <string.h>

#include "v3d.h"
#include "v3d_regs.h"

/* --- the model: five blocks at distinct fake addresses (the SMS is 7.1's) --- */
#define HUB  0x10000000u
#define CORE 0x20000000u
#define PM   0x30000000u
#define ASB  0x40000000u
#define SMS  0x50000000u

static uint32_t hub[0x2000 / 4], core[0x1000 / 4], pm[0x400 / 4], asb[0x40 / 4], sms[0x800 / 4];
/* the SMS's one state machine, which both REE_CS and TEE_CS read in STATE */
static uint32_t sms_state;
static unsigned asb_reads; /* a 7.1 has no bridge: no access at all */

static struct {
	bool asb_ack_sticks;     /* the bridge never acknowledges */
	bool asb_m_never_stops;  /* the bridge master never acknowledges a stop */
	bool bin_raises_oom;     /* starting the binner raises OUTOMEM */
	bool render_never_ends;  /* FRDONE never comes */
	bool bin_never_ends;     /* FLDONE never comes */
	bool mmu_fault_on_bin;   /* starting the binner raises a PTI fault */
	bool tfu_never_ends;     /* a TFU job raises no TFUC */
	bool tfu_faults;         /* a TFU job raises an MMU write violation */
	bool tfu_no_count;       /* a TFU job raises TFUC but CVTCT does not move */
	uint32_t oom_blocks;     /* the binner runs out of memory until given this many
				  * overflow blocks (BPOS writes), then finishes */
	/* 7.1 */
	bool sms_stuck_off;      /* CLEAR_POWER_OFF never reaches IDLE */
	bool sms_never_off;      /* POWER_OFF never reaches the power-off state */
	bool sms_reset_stuck;    /* the reset request stays RESETTING */
	uint32_t sms_reset_reads; /* the reset reads RESETTING this many times, then IDLE */
	bool gmpv_on_bin;        /* starting the binner raises the hub's GMPV */
} model;

/* The write log: which block, offset and value, in order. 'K' is the clock. */
struct wr {
	char blk;
	uint32_t off, val;
};
static struct wr wlog[512];
static unsigned nlog;
static uint32_t now_us;

static void log_write(char blk, uint32_t off, uint32_t val)
{
	if (nlog < sizeof(wlog) / sizeof(wlog[0]))
		wlog[nlog++] = (struct wr){ blk, off, val };
}

static uint32_t *reg(mm_reg_t a, char *blk, uint32_t *off)
{
	if (a >= HUB && a < HUB + sizeof(hub)) {
		*blk = 'H';
		*off = (uint32_t)(a - HUB);
		return &hub[*off / 4];
	}
	if (a >= CORE && a < CORE + sizeof(core)) {
		*blk = 'C';
		*off = (uint32_t)(a - CORE);
		return &core[*off / 4];
	}
	if (a >= PM && a < PM + sizeof(pm)) {
		*blk = 'P';
		*off = (uint32_t)(a - PM);
		return &pm[*off / 4];
	}
	if (a >= ASB && a < ASB + sizeof(asb)) {
		*blk = 'A';
		*off = (uint32_t)(a - ASB);
		return &asb[*off / 4];
	}
	if (a >= SMS && a < SMS + sizeof(sms)) {
		*blk = 'S';
		*off = (uint32_t)(a - SMS);
		return &sms[*off / 4];
	}
	printf("FATAL access outside every block: 0x%lx\n", (unsigned long)a);
	return NULL;
}

uint32_t mock_read32(mm_reg_t a)
{
	char blk;
	uint32_t off;
	uint32_t *p = reg(a, &blk, &off);
	if (p && blk == 'A')
		asb_reads++;
	if (p && blk == 'S' && (off == V3D_SMS_REE_CS || off == V3D_SMS_TEE_CS)) {
		/* a reset in progress ends after its reads, unless it sticks */
		if (sms_state == V3D_SMS_STATE_RESETTING && !model.sms_reset_stuck &&
		    model.sms_reset_reads && --model.sms_reset_reads == 0)
			sms_state = V3D_SMS_STATE_IDLE;
		return (*p & ~V3D_SMS_CS_STATE_MASK) | sms_state;
	}
	return p ? *p : 0xdeadbeefu;
}

void mock_write32(uint32_t v, mm_reg_t a)
{
	char blk;
	uint32_t off;
	uint32_t *p = reg(a, &blk, &off);
	if (!p)
		return;
	log_write(blk, off, v);

	switch (blk) {
	case 'A':
		/* clearing REQ_STOP drops ACK unless the model says it sticks;
		 * setting it raises ACK unless the master never stops */
		*p = v & ~BCM2711_PM_WRITE_KEY;
		if (!(v & BCM2711_ASB_CTL_STOP_REQUEST_SET) && !model.asb_ack_sticks)
			*p &= ~BCM2711_ASB_CTL_STOP_ACK_SET;
		if ((v & BCM2711_ASB_CTL_STOP_REQUEST_SET) && !(model.asb_m_never_stops && off == BCM2711_ASB_V3D_MASTER_CTL))
			*p |= BCM2711_ASB_CTL_STOP_ACK_SET;
		return;
	case 'P':
		*p = v & ~BCM2711_PM_WRITE_KEY;
		return;
	case 'S':
		/* TEE_CS: power up (to IDLE) or off (to the power-off state);
		 * REE_CS: the reset request, RESETTING until it ends */
		*p = v & ~V3D_SMS_CS_STATE_MASK;
		if (off == V3D_SMS_TEE_CS && (v & V3D_SMS_CS_CLEAR_POWER_OFF_SET) && !model.sms_stuck_off)
			sms_state = V3D_SMS_STATE_IDLE;
		if (off == V3D_SMS_TEE_CS && (v & V3D_SMS_CS_POWER_OFF_SET) && !model.sms_never_off)
			sms_state = V3D_SMS_STATE_POWER_OFF;
		if (off == V3D_SMS_REE_CS && V3D_SMS_CS_STATE_GET(v) == V3D_SMS_STATE_RESET_REQUEST)
			sms_state = (model.sms_reset_stuck || model.sms_reset_reads) ? V3D_SMS_STATE_RESETTING
										    : V3D_SMS_STATE_IDLE;
		return;
	case 'H':
		if (off == V3D_MMU0_CACHE_CTL) {
			*p = v & ~V3D_MMU0_CACHE_CTL_FLUSHING_SET; /* a flush completes at once */
			return;
		}
		if (off == V3D_MMU0_CTL) {
			*p = v & ~V3D_MMU0_CTL_TLB_CLEARING_SET;
			return;
		}
		if (off == V3D_TFU_ICFG) {
			/* the write that starts a job */
			*p = v;
			if (!(v & V3D_TFU_ICFG_IOC_SET))
				return;
			if (model.tfu_faults) {
				hub[V3D_HUB_CTL_INT_STS / 4] |= V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET;
			} else if (!model.tfu_never_ends) {
				hub[V3D_HUB_CTL_INT_STS / 4] |= V3D_HUB_CTL_INT_STS_INT_TFUC_SET;
				if (!model.tfu_no_count)
					hub[V3D_TFU_CS / 4] += 1u << 16; /* CVTCT */
			}
			return;
		}
		if (off == V3D_HUB_CTL_INT_CLR) {
			hub[V3D_HUB_CTL_INT_STS / 4] &= ~v;
			/* whether the V3D keeps VIO_ID/VIO_ADDR past the clear is
			 * unmeasured: the model clears them, the worst case */
			if (v & V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET) {
				hub[V3D_MMU0_VIO_ID / 4] = 0;
				hub[V3D_MMU0_VIO_ADDR / 4] = 0;
			}
			return;
		}
		*p = v;
		return;
	case 'C':
		if (off == V3D_CTL_0_INT_CLR) {
			core[V3D_CTL_0_INT_STS / 4] &= ~v;
			return;
		}
		if (off == V3D_CTL_0_L2TCACTL) {
			*p = v & ~(V3D_CTL_0_L2TCACTL_TMUWCF_SET | V3D_CTL_0_L2TCACTL_L2TFLS_SET);
			return;
		}
		*p = v;
		if (off == V3D_CLE_0_CT0QEA) {
			if (model.mmu_fault_on_bin) {
				/* the struct.uniform run's fault (00:19): L2T, raw 0x1010000 */
				hub[V3D_HUB_CTL_INT_STS / 4] |= V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET;
				hub[V3D_MMU0_VIO_ID / 4] = 0xd;
				hub[V3D_MMU0_VIO_ADDR / 4] = 0x1010000;
			}
			else if (model.gmpv_on_bin)
				hub[V3D_HUB_CTL_INT_STS / 4] |= V3D71_HUB_CTL_INT_STS_INT_GMPV_SET;
			else if (model.bin_raises_oom || model.oom_blocks)
				core[V3D_CTL_0_INT_STS / 4] |= V3D_CTL_0_INT_STS_INT_OUTOMEM_SET;
			else if (!model.bin_never_ends)
				core[V3D_CTL_0_INT_STS / 4] |= V3D_CTL_0_INT_STS_INT_FLDONE_SET;
		}
		/* a block given: out of memory again, or done */
		if (off == V3D_PTB_0_BPOS && v != 0 && model.oom_blocks) {
			if (--model.oom_blocks)
				core[V3D_CTL_0_INT_STS / 4] |= V3D_CTL_0_INT_STS_INT_OUTOMEM_SET;
			else
				core[V3D_CTL_0_INT_STS / 4] |= V3D_CTL_0_INT_STS_INT_FLDONE_SET;
		}
		if (off == V3D_CLE_0_CT1QEA && !model.render_never_ends)
			core[V3D_CTL_0_INT_STS / 4] |= V3D_CTL_0_INT_STS_INT_FRDONE_SET;
		return;
	}
}

void mock_busy_wait(uint32_t us)
{
	now_us += us;
}

static int clk_calls[8], nclk;
static int clk_set(void *ctx, bool on)
{
	(void)ctx;
	if (nclk < 8)
		clk_calls[nclk] = on;
	nclk++;
	log_write('K', 0, on); /* the clock switch is part of the order */
	return 0;
}

static void reset_model(void)
{
	memset(hub, 0, sizeof(hub));
	memset(core, 0, sizeof(core));
	memset(pm, 0, sizeof(pm));
	memset(asb, 0, sizeof(asb));
	memset(sms, 0, sizeof(sms));
	sms_state = V3D_SMS_STATE_POWER_OFF;
	asb_reads = 0;
	memset(&model, 0, sizeof(model));
	nlog = 0;
	nclk = 0;
	now_us = 0;
	asb[BCM2711_ASB_ID / 4] = BCM2711_ASB_ID_VALUE;
	asb[BCM2711_ASB_V3D_MASTER_CTL / 4] = BCM2711_ASB_CTL_STOP_REQUEST_SET | BCM2711_ASB_CTL_STOP_ACK_SET; /* stopped, as after reset */
	asb[BCM2711_ASB_V3D_SLAVE_CTL / 4] = BCM2711_ASB_CTL_STOP_REQUEST_SET | BCM2711_ASB_CTL_STOP_ACK_SET;
	/* a V3D 4.2: TVER 4, REV 2, one core; an MMU; 2 slices x 4 QPUs;
	 * VPM_SIZE 2 (16 KiB); PA width 40 (field 10), VA width 32 (field 2).
	 * Invented for the model: the board's own values are not known yet. */
	hub[V3D_HUB_CTL_IDENT1 / 4] = (1u << 8) | (2u << 4) | 4u;
	hub[V3D_HUB_CTL_IDENT2 / 4] = V3D_HUB_CTL_IDENT2_WITH_MMU_SET;
	core[V3D_CTL_0_IDENT0 / 4] = 4u << 24;
	core[V3D_CTL_0_IDENT1 / 4] = (2u << 28) | (4u << 8) | (2u << 4) | 2u;
	hub[V3D_MMU0_DEBUG_INFO / 4] = (10u << 8) | (2u << 4);
}

/* --- checks --- */
static int checks, failed;
#define CHECK(cond, ...)                                            \
	do {                                                        \
		checks++;                                           \
		if (!(cond)) {                                      \
			failed++;                                   \
			printf("FAIL %s:%d: ", __FILE__, __LINE__); \
			printf(__VA_ARGS__);                        \
			printf("\n");                               \
		}                                                   \
	} while (0)

/* Index of the first logged write to (blk, off) at or after `from`, or -1. */
static int find(char blk, uint32_t off, int from)
{
	for (unsigned i = (unsigned)(from < 0 ? 0 : from); i < nlog; i++)
		if (wlog[i].blk == blk && wlog[i].off == off)
			return (int)i;
	return -1;
}

static uint32_t val_at(int i)
{
	return i >= 0 ? wlog[i].val : 0;
}

static const struct v3d_regs R = { .hub = HUB, .core = CORE, .pm = PM, .asb = ASB };
static struct v3d_clock_ops CLK = { .set = clk_set };

static void t_power_on(void) /* 8 checks */
{
	reset_model();
	int ret = v3d_power_on(&R, &CLK);
	CHECK(ret == 0, "power_on returned %d", ret);
	/* clock on, off, the power manager's reset released, clock on, the
	 * bridge's master port, then its slave */
	CHECK(nclk == 3 && clk_calls[0] == 1 && clk_calls[1] == 0 && clk_calls[2] == 1,
	      "clock sequence %d calls: %d %d %d", nclk, clk_calls[0], clk_calls[1], clk_calls[2]);
	int k0 = find('K', 0, 0), k1 = find('K', 0, k0 + 1);
	int pmw = find('P', BCM2711_PM_GRAPHICS_CTL, 0);
	int k2 = find('K', 0, k1 + 1);
	int m = find('A', BCM2711_ASB_V3D_MASTER_CTL, 0), s = find('A', BCM2711_ASB_V3D_SLAVE_CTL, 0);
	CHECK(k0 >= 0 && k1 > k0 && pmw > k1 && k2 > pmw && m > k2 && s > m,
	      "order k0 %d k1 %d pm %d k2 %d m %d s %d", k0, k1, pmw, k2, m, s);
	CHECK((val_at(pmw) & 0xff000000u) == BCM2711_PM_WRITE_KEY && (val_at(pmw) & BCM2711_PM_GRAPHICS_V3D_RESET_N_SET),
	      "BCM2711_PM_GRAPHICS_CTL write 0x%08x lacks the password or V3DRSTN", val_at(pmw));
	CHECK(m >= 0 && (val_at(m) & 0xff000000u) == BCM2711_PM_WRITE_KEY && !(val_at(m) & BCM2711_ASB_CTL_STOP_REQUEST_SET),
	      "ASB master write 0x%08x", val_at(m));
	CHECK(s >= 0 && !(val_at(s) & BCM2711_ASB_CTL_STOP_REQUEST_SET), "ASB slave write 0x%08x", val_at(s));

	/* a bridge that never acknowledges is -ETIMEDOUT, and the reset goes back */
	reset_model();
	model.asb_ack_sticks = true;
	ret = v3d_power_on(&R, &CLK);
	CHECK(ret == -ETIMEDOUT && !(pm[BCM2711_PM_GRAPHICS_CTL / 4] & BCM2711_PM_GRAPHICS_V3D_RESET_N_SET),
	      "stuck ACK returned %d, BCM2711_PM_GRAPHICS_CTL 0x%x", ret, pm[BCM2711_PM_GRAPHICS_CTL / 4]);

	/* a wrong bridge ID is -ENODEV before anything is written */
	reset_model();
	asb[BCM2711_ASB_ID / 4] = 0x12345678;
	ret = v3d_power_on(&R, &CLK);
	CHECK(ret == -ENODEV && nlog == 0, "bad bridge ID returned %d after %u writes", ret, nlog);
}

static void t_identify(void) /* 4 checks */
{
	struct v3d_identity id;
	reset_model();
	int ret = v3d_identify(&R, &id);
	CHECK(ret == 0, "identify returned %d", ret);
	CHECK(id.ver == 42 && id.ncores == 1 && id.qpu_count == 8 && id.vpm_size == 16384 &&
		      id.pa_width == 40 && id.va_width == 32 && id.has_mmu,
	      "decoded ver %u cores %u qpus %u vpm %u pa %u va %u mmu %d", id.ver, id.ncores,
	      id.qpu_count, id.vpm_size, id.pa_width, id.va_width, id.has_mmu);
	CHECK(nlog == 0, "identify wrote %u registers", nlog);

	reset_model();
	hub[V3D_HUB_CTL_IDENT1 / 4] = (1u << 8) | (1u << 4) | 7u; /* a 7.1 */
	ret = v3d_identify(&R, &id);
	CHECK(ret == -ENODEV && id.ver == 71, "a 7.1 returned %d (ver %u)", ret, id.ver);
}

static void t_pte(void) /* 10 checks */
{
	static uint32_t pt[1024], before[1024];
	memset(pt, 0, sizeof(pt));
	int ret = v3d_pte_map(pt, 1024, 0x1000, 0x3c000000ull, 0x2800, true);
	CHECK(ret == 0, "map returned %d", ret);
	CHECK(pt[1] == (V3D_MMU_PTE_VALID_SET | V3D_MMU_PTE_WRITABLE_SET | 0x3c000) &&
		      pt[2] == (V3D_MMU_PTE_VALID_SET | V3D_MMU_PTE_WRITABLE_SET | 0x3c001) &&
		      pt[3] == (V3D_MMU_PTE_VALID_SET | V3D_MMU_PTE_WRITABLE_SET | 0x3c002) && pt[4] == 0 &&
		      pt[0] == 0,
	      "entries %08x %08x %08x %08x %08x", pt[0], pt[1], pt[2], pt[3], pt[4]);
	ret = v3d_pte_map(pt, 1024, 0x5000, 0x1000, 0x1000, false);
	CHECK(ret == 0 && pt[5] == (V3D_MMU_PTE_VALID_SET | 1), "read-only entry %08x", pt[5]);

	/* refusals, each writing nothing */
	memcpy(before, pt, sizeof(pt));
	CHECK(v3d_pte_map(pt, 1024, 0, 0x1000, 0x1000, true) == -EINVAL, "VA 0 accepted");
	CHECK(v3d_pte_map(pt, 1024, 0x1800, 0x1000, 0x1000, true) == -EINVAL, "unaligned VA");
	CHECK(v3d_pte_map(pt, 1024, 0x1000, 0x1800, 0x1000, true) == -EINVAL, "unaligned PA");
	CHECK(v3d_pte_map(pt, 1024, 0x3ff000, 0x1000, 0x2000, true) == -EINVAL, "past the table");
	CHECK(v3d_pte_map(pt, 1024, 0x1000, 1ull << 36, 0x1000, true) == -EINVAL,
	      "a 37-bit PA accepted");
	CHECK(v3d_pte_map(pt, 1024, 0x1000, 0x1000, 0, true) == -EINVAL, "zero bytes accepted");
	CHECK(memcmp(before, pt, sizeof(pt)) == 0, "a refused map wrote entries");
}

static void t_mmu(void) /* 6 checks */
{
	reset_model();
	int ret = v3d_mmu_init(&R, 0x3e000000ull, 0x3e400000ull);
	CHECK(ret == 0, "mmu_init returned %d", ret);
	int base = find('H', V3D_MMU0_PT_PA_BASE, 0), ctl = find('H', V3D_MMU0_CTL, 0);
	int ill = find('H', V3D_MMU0_ILLEGAL_ADR, 0), mmuc = find('H', V3D_MMU0_CACHE_CTL, 0);
	int flush = find('H', V3D_MMU0_CACHE_CTL, mmuc + 1), tlb = find('H', V3D_MMU0_CTL, ctl + 1);
	CHECK(base >= 0 && ctl > base && ill > ctl && mmuc > ill && flush > mmuc && tlb > flush,
	      "order base %d ctl %d ill %d mmuc %d flush %d tlb %d", base, ctl, ill, mmuc, flush,
	      tlb);
	CHECK(val_at(base) == 0x3e000, "PT base 0x%x", val_at(base));
	CHECK(val_at(ill) == (0x3e400u | V3D_MMU0_ILLEGAL_ADR_ENABLE_SET), "illegal addr 0x%x",
	      val_at(ill));
	CHECK((val_at(ctl) & V3D_MMU0_CTL_ENABLE_SET) && (val_at(ctl) & V3D_MMU0_CTL_PT_INVALID_ENABLE_SET),
	      "MMU_CTL 0x%x", val_at(ctl));
	CHECK((val_at(tlb) & V3D_MMU0_CTL_TLB_CLEAR_SET) && (val_at(tlb) & V3D_MMU0_CTL_ENABLE_SET),
	      "TLB clear write 0x%x lost the enable", val_at(tlb));
}

static void t_job(void) /* 11 checks */
{
	/* bin then render, each polled to completion */
	reset_model();
	v3d_irq_quiesce(&R);
	v3d_caches_init(&R);
	nlog = 0;
	v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	int bpos = find('C', V3D_PTB_0_BPOS, 0), inv = find('C', V3D_CTL_0_L2TCACTL, 0);
	int qma = find('C', V3D_CLE_0_CT0QMA, 0), qms = find('C', V3D_CLE_0_CT0QMS, 0);
	int qts = find('C', V3D_CLE_0_CT0QTS, 0), qba = find('C', V3D_CLE_0_CT0QBA, 0);
	int qea = find('C', V3D_CLE_0_CT0QEA, 0), clr = find('C', V3D_CTL_0_INT_CLR, 0);
	/* first the stale done bits cleared (t_stale), then the submit order */
	CHECK(clr == 0 && bpos == 1 && inv > bpos && qma > inv && qms > qma && qts > qms && qba > qts &&
		      qea > qba && qea == (int)nlog - 1,
	      "bin order clr %d bpos %d inv %d qma %d qms %d qts %d qba %d qea %d of %u", clr, bpos, inv,
	      qma, qms, qts, qba, qea, nlog);
	CHECK(val_at(qts) == (0x30000u | V3D_CLE_0_CT0QTS_CTQTSEN_SET), "QTS 0x%x", val_at(qts));
	struct v3d_stall st;
	int ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == 0, "bin wait returned %d", ret);
	CHECK(core[V3D_CTL_0_INT_STS / 4] == 0, "FLDONE not cleared");

	nlog = 0;
	v3d_submit_render(&R, 0x11000, 0x1107f);
	qea = find('C', V3D_CLE_0_CT1QEA, 0);
	CHECK(qea == (int)nlog - 1 && find('C', V3D_CLE_0_CT1QBA, 0) < qea &&
		      find('C', V3D_CLE_0_CT1QBA, 0) > find('C', V3D_CTL_0_SLCACTL, 0),
	      "render: invalidate, then start, then end last");
	ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FRDONE_SET, 1000, &st);
	CHECK(ret == 0, "render wait returned %d", ret);

	/* the three failures are three different outcomes */
	reset_model();
	model.bin_raises_oom = true;
	v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == -ENOMEM && (st.core_int_sts & V3D_CTL_0_INT_STS_INT_OUTOMEM_SET), "OOM returned %d sts 0x%x",
	      ret, st.core_int_sts);

	reset_model();
	model.mmu_fault_on_bin = true;
	v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == -EFAULT && (st.hub_int_sts & V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET), "fault returned %d", ret);
	/* the client and the address as the fault left them, not as the clear
	 * that followed left them */
	CHECK(st.mmu_vio_id == 0xd && st.mmu_vio_addr == 0x1010000 && hub[V3D_HUB_CTL_INT_STS / 4] == 0,
	      "fault snapshot vio_id 0x%x vio_addr 0x%x, hub now 0x%x", st.mmu_vio_id, st.mmu_vio_addr,
	      hub[V3D_HUB_CTL_INT_STS / 4]);

	reset_model();
	model.render_never_ends = true;
	v3d_submit_render(&R, 0x11000, 0x1107f);
	core[V3D_CLE_0_CT1CA / 4] = 0x11040; /* where the model says it stopped */
	ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FRDONE_SET, 500, &st);
	CHECK(ret == -ETIMEDOUT && st.ct1ca == 0x11040 && now_us >= 500,
	      "timeout returned %d ct1ca 0x%x after %u us", ret, st.ct1ca, now_us);

	/* a wait for two bits that arrive in two polls still completes */
	reset_model();
	core[V3D_CTL_0_INT_STS / 4] = V3D_CTL_0_INT_STS_INT_FLDONE_SET;
	ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 10, &st); /* consumes FLDONE */
	core[V3D_CTL_0_INT_STS / 4] = V3D_CTL_0_INT_STS_INT_FRDONE_SET;
	int ret2 = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET | V3D_CTL_0_INT_STS_INT_FRDONE_SET, 10, &st);
	CHECK(ret == 0 && ret2 == -ETIMEDOUT,
	      "a consumed bit is not seen again: %d then %d (FLDONE was already cleared)", ret,
	      ret2);
}

/* The binner's overflow pool (v3d_wait_bin): each OUTOMEM gets the next
 * block (BPOA, then BPOS), and a dry pool is -ENOMEM with the count said. */
static void t_overflow(void)
{
	static const uint32_t blocks[2] = { 0x00700000u, 0x00740000u };
	struct v3d_stall st;
	struct v3d_bin_overflow ov;

	for (uint32_t need = 1; need <= 2; need++) {
		reset_model();
		model.oom_blocks = need;
		ov = (struct v3d_bin_overflow){ blocks, 2, 256 * 1024, 0 };
		v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
		int ret = v3d_wait_bin(&R, 1000, &st, &ov);
		CHECK(ret == 0 && ov.used == need, "need %u: returned %d, used %u", need, ret, ov.used);
		int a = find('C', V3D_PTB_0_BPOA, 0), s = find('C', V3D_PTB_0_BPOS, a); /* after the submit's BPOS 0 */
		CHECK(a >= 0 && s > a && val_at(a) == blocks[0] && val_at(s) == 256 * 1024,
		      "need %u: BPOA 0x%x then BPOS %u", need, val_at(a), val_at(s));
	}
	CHECK(val_at(find('C', V3D_PTB_0_BPOA, find('C', V3D_PTB_0_BPOA, 0) + 1)) == blocks[1],
	      "the second OUTOMEM gets the second block");
	/* three needed, two given: dry */
	reset_model();
	model.oom_blocks = 3;
	ov = (struct v3d_bin_overflow){ blocks, 2, 256 * 1024, 0 };
	v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	int ret = v3d_wait_bin(&R, 1000, &st, &ov);
	CHECK(ret == -ENOMEM && ov.used == 2 && (st.core_int_sts & V3D_CTL_0_INT_STS_INT_OUTOMEM_SET),
	      "a dry pool: returned %d, used %u", ret, ov.used);
	/* no pool at all: as before */
	reset_model();
	model.oom_blocks = 1;
	v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	CHECK(v3d_wait_bin(&R, 1000, &st, NULL) == -ENOMEM, "no pool: -ENOMEM");
}

/* A job that timed out can still end late, and its done bit can reach the
 * NEXT job's wait (the E runs on 8c8df5d1: later cases pass after timed-out
 * ones). So a wait clears only the bits it waits for, and each submit clears
 * its own stage's done bit before starting it: a stale bit is never taken as
 * the new job's end. */
static void t_stale(void) /* 5 checks */
{
	struct v3d_stall st;

	/* a wait for FLDONE leaves another stage's FRDONE for that stage */
	reset_model();
	core[V3D_CTL_0_INT_STS / 4] = V3D_CTL_0_INT_STS_INT_FLDONE_SET | V3D_CTL_0_INT_STS_INT_FRDONE_SET;
	CHECK(v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st) == 0 && (core[V3D_CTL_0_INT_STS / 4] & V3D_CTL_0_INT_STS_INT_FRDONE_SET),
	      "the FLDONE wait cleared FRDONE: STS 0x%x", core[V3D_CTL_0_INT_STS / 4]);
	/* a stale FRDONE (the late job's) before a render that never ends */
	reset_model();
	model.render_never_ends = true;
	core[V3D_CTL_0_INT_STS / 4] = V3D_CTL_0_INT_STS_INT_FRDONE_SET;
	v3d_submit_render(&R, 0x40000, 0x40010);
	int ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FRDONE_SET, 1000, &st);
	CHECK(ret == -ETIMEDOUT, "a stale FRDONE was taken as this render's end: returned %d", ret);
	int clr = find('C', V3D_CTL_0_INT_CLR, 0), qea = find('C', V3D_CLE_0_CT1QEA, 0);
	CHECK(clr >= 0 && qea > clr && (val_at(clr) & V3D_CTL_0_INT_STS_INT_FRDONE_SET),
	      "FRDONE is cleared before CT1QEA starts the renderer (clear %d, start %d)", clr, qea);
	/* a stale FLDONE before a bin that never ends */
	reset_model();
	model.bin_never_ends = true;
	core[V3D_CTL_0_INT_STS / 4] = V3D_CTL_0_INT_STS_INT_FLDONE_SET;
	v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == -ETIMEDOUT, "a stale FLDONE was taken as this bin's end: returned %d", ret);
	clr = find('C', V3D_CTL_0_INT_CLR, 0);
	int ea = find('C', V3D_CLE_0_CT0QEA, 0);
	CHECK(clr >= 0 && ea > clr && (val_at(clr) & V3D_CTL_0_INT_STS_INT_FLDONE_SET),
	      "FLDONE is cleared before CT0QEA starts the binner (clear %d, start %d)", clr, ea);
}

/* The GPU reset after a stage that did not end: a power cycle (v3d_power_off
 * then v3d_power_on), the MMU on the same page table. */
static void t_reset(void) /* 9 checks: 3 power-ons, 6 on the reset */
{
	/* power off: slave stop, master stop, clock off, reset asserted */
	reset_model();
	CHECK(v3d_power_on(&R, &CLK) == 0, "power on first");
	nlog = 0;
	nclk = 0;
	int ret = v3d_power_off(&R, &CLK);
	int s = find('A', BCM2711_ASB_V3D_SLAVE_CTL, 0), m = find('A', BCM2711_ASB_V3D_MASTER_CTL, 0), k = find('K', 0, 0);
	int pmw = find('P', BCM2711_PM_GRAPHICS_CTL, 0);
	CHECK(ret == 0 && s >= 0 && m > s && k > m && pmw > k && val_at(k) == 0,
	      "power off returned %d, order slave %d master %d clock %d pm %d", ret, s, m, k, pmw);
	CHECK((val_at(s) & BCM2711_ASB_CTL_STOP_REQUEST_SET) && (val_at(m) & BCM2711_ASB_CTL_STOP_REQUEST_SET) &&
		      (val_at(s) & 0xff000000u) == BCM2711_PM_WRITE_KEY && !(pm[BCM2711_PM_GRAPHICS_CTL / 4] & BCM2711_PM_GRAPHICS_V3D_RESET_N_SET),
	      "power off writes: slave 0x%08x master 0x%08x BCM2711_PM_GRAPHICS_CTL 0x%x", val_at(s), val_at(m),
	      pm[BCM2711_PM_GRAPHICS_CTL / 4]);
	/* a master that never stops: -ETIMEDOUT, the slave enabled again, the
	 * clock left alone */
	reset_model();
	CHECK(v3d_power_on(&R, &CLK) == 0, "power on again");
	model.asb_m_never_stops = true;
	nlog = 0;
	nclk = 0;
	ret = v3d_power_off(&R, &CLK);
	int s2 = find('A', BCM2711_ASB_V3D_SLAVE_CTL, find('A', BCM2711_ASB_V3D_SLAVE_CTL, 0) + 1);
	CHECK(ret == -ETIMEDOUT && s2 >= 0 && !(val_at(s2) & BCM2711_ASB_CTL_STOP_REQUEST_SET) && nclk == 0,
	      "a stuck master: returned %d, slave again %d (0x%08x), %d clock calls", ret, s2, val_at(s2), nclk);
	/* the whole reset: quiesce, power off, power on, the same page table */
	reset_model();
	CHECK(v3d_power_on(&R, &CLK) == 0, "power on for the reset");
	nlog = 0;
	ret = v3d_reset(&R, &CLK, 0x3f000000u, 0x3f080000u);
	int q = find('C', V3D_CTL_0_INT_MSK_SET, 0), off = find('A', BCM2711_ASB_V3D_SLAVE_CTL, 0);
	int on = find('A', BCM2711_ASB_V3D_SLAVE_CTL, off + 1), pt = find('H', V3D_MMU0_PT_PA_BASE, 0);
	CHECK(ret == 0 && q >= 0 && off > q && on > off && pt > on && val_at(pt) == (0x3f000000u >> 12),
	      "reset returned %d, order quiesce %d off %d on %d pt %d (0x%x)", ret, q, off, on, pt, val_at(pt));
	/* a reset whose power off fails stops before the MMU */
	reset_model();
	CHECK(v3d_power_on(&R, &CLK) == 0, "power on for the failing reset");
	model.asb_m_never_stops = true;
	nlog = 0;
	ret = v3d_reset(&R, &CLK, 0x3f000000u, 0x3f080000u);
	CHECK(ret == -ETIMEDOUT && find('H', V3D_MMU0_PT_PA_BASE, 0) < 0,
	      "a failing reset returned %d, MMU written at %d", ret, find('H', V3D_MMU0_PT_PA_BASE, 0));
}

/* The TFU: the registers in v3d_tfu_run's order (Mesa's simulator glue's),
 * ICFG last with IOC, COEF0 without USECOEF; completion by TFUC with CVTCT
 * moved by one; a stale TFUC cleared before the start; the other outcomes by
 * name; TFURST never written. */
static void t_tfu(void)
{
	static const struct v3d_tfu_job j = {
		.icfg = 0x3a00, .iia = 0x200000, .iis = 96, .ica = 0, .iua = 0,
		.ioa = 0x100030, .ios = 0x0025005f, .coef0 = V3D_TFU_COEF0_USECOEF_SET | 5,
	};
	struct v3d_tfu_status st;

	reset_model();
	hub[V3D_TFU_CS / 4] = 7u << 16 | 8u << 8; /* CVTCT 7, 8 free slots */
	int ret = v3d_tfu_run(&R, &j, 1000, &st);
	CHECK(ret == 0 && (st.hub_int_sts & V3D_HUB_CTL_INT_STS_INT_TFUC_SET) && ((st.cs_after >> 16) & 0xff) == 8 &&
		      ((st.cs_before >> 16) & 0xff) == 7,
	      "a job returned %d, int 0x%x, CVTCT %u -> %u", ret, st.hub_int_sts, (st.cs_before >> 16) & 0xff,
	      (st.cs_after >> 16) & 0xff);
	static const struct {
		uint32_t off, val;
	} want[] = {
		{ V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET },
		{ V3D_TFU_IIA, 0x200000 }, { V3D_TFU_IIS, 96 }, { V3D_TFU_ICA, 0 }, { V3D_TFU_IUA, 0 },
		{ V3D_TFU_IOA, 0x100030 }, { V3D_TFU_IOS, 0x0025005f }, { V3D_TFU_COEF0, 5 },
		{ V3D_TFU_ICFG, 0x3a00 | V3D_TFU_ICFG_IOC_SET },
		{ V3D_HUB_CTL_INT_CLR, V3D_HUB_CTL_INT_STS_INT_TFUC_SET | V3D_HUB_CTL_INT_STS_INT_TFUF_SET },
	};
	bool order = nlog == sizeof(want) / sizeof(want[0]);
	for (unsigned i = 0; order && i < nlog; i++)
		order = wlog[i].blk == 'H' && wlog[i].off == want[i].off && wlog[i].val == want[i].val;
	CHECK(order, "the writes: %u logged, not the order of v3d_tfu_job_run", nlog);
	CHECK(hub[V3D_HUB_CTL_INT_STS / 4] == 0, "TFUC left set: 0x%x", hub[V3D_HUB_CTL_INT_STS / 4]);

	/* busy before the start: nothing written */
	reset_model();
	hub[V3D_TFU_CS / 4] = V3D_TFU_CS_BUSY_SET;
	ret = v3d_tfu_run(&R, &j, 1000, &st);
	CHECK(ret == -EBUSY && nlog == 0, "busy: returned %d after %u writes", ret, nlog);

	/* a stale TFUC does not end the wait: the job that never ends times out */
	reset_model();
	hub[V3D_HUB_CTL_INT_STS / 4] = V3D_HUB_CTL_INT_STS_INT_TFUC_SET;
	model.tfu_never_ends = true;
	ret = v3d_tfu_run(&R, &j, 500, &st);
	CHECK(ret == -ETIMEDOUT && now_us >= 500 && find('H', V3D_TFU_CS, 0) < 0,
	      "a job that never ends returned %d after %u us (TFU_CS written at %d)", ret, now_us,
	      find('H', V3D_TFU_CS, 0));

	reset_model();
	model.tfu_faults = true;
	ret = v3d_tfu_run(&R, &j, 500, &st);
	CHECK(ret == -EFAULT && (st.hub_int_sts & V3D_HUB_CTL_INT_STS_INT_MMU_WRV_SET), "a fault returned %d", ret);

	reset_model();
	model.tfu_no_count = true;
	ret = v3d_tfu_run(&R, &j, 500, &st);
	CHECK(ret == -EIO, "TFUC without CVTCT moving returned %d", ret);
}

/* ===================================================================== */
/* V3D 7.1 (BCM2712): --v71                                               */
/* ===================================================================== */

static const struct v3d_regs R71 = { .hub = HUB, .core = CORE, .pm = PM, .asb = ASB, .sms = SMS,
				     .soc = V3D_SOC_BCM2712 };

/* The model as a 7.1: the identity Mesa's drm-shim reports for 7.1
 * (tools/v3d/README.md; docs/notes/pre-study-rpi5.md, section 3.4:
 * HUB_IDENT1 0x00081117, HUB_IDENT3 0x00000700, CORE0_IDENT0 0x07443356,
 * CORE0_IDENT1 0x81001441), the MMU bit, and an MMU_DEBUG_INFO invented for
 * the model (PA 40, VA 32 bits): Mesa's numbers, not a Pi 5's. The SMS
 * starts powered off, the V3D held in reset. */
static void reset_model71(void)
{
	reset_model();
	memset(asb, 0, sizeof(asb)); /* nothing answers there on a BCM2712 */
	hub[V3D_HUB_CTL_IDENT1 / 4] = 0x00081117u;
	hub[V3D_HUB_CTL_IDENT2 / 4] = V3D_HUB_CTL_IDENT2_WITH_MMU_SET;
	hub[V3D_HUB_CTL_IDENT3 / 4] = 0x00000700u;
	core[V3D_CTL_0_IDENT0 / 4] = 0x07443356u;
	core[V3D_CTL_0_IDENT1 / 4] = 0x81001441u;
	hub[V3D_MMU0_DEBUG_INFO / 4] = (10u << 8) | (2u << 4);
}

static bool no_bridge(void)
{
	return find('A', BCM2711_ASB_V3D_MASTER_CTL, 0) < 0 && find('A', BCM2711_ASB_V3D_SLAVE_CTL, 0) < 0 &&
	       asb_reads == 0;
}

/* The register map's 7.1 facts (driver/docs/rpi5-v3d71-facts.md, sections 2
 * and 3; the fork's PM_GRAFX_2712) and the SoC selection. */
static void t71_map(void) /* 4 checks */
{
	CHECK(V3D71_TFU_CS == 0x700 && V3D71_TFU_ICFG == 0x708 && V3D71_TFU_IIA == 0x70c && V3D71_TFU_IOC == 0x71c &&
		      V3D71_TFU_IOA == 0x720 && V3D71_TFU_IOS == 0x724 && V3D71_TFU_COEF0 == 0x728 &&
		      V3D71_TFU_COEF3 == 0x734,
	      "the 7.1 TFU block is not at 0x700");
	CHECK(V3D71_MMU0_CACHE_CTL_CLEAR_SET == (1u << 11) && V3D71_CTL_0_INT_STS_INT_CSDDONE_SET == (1u << 6) &&
		      V3D71_CTL_0_INT_STS_INT_PCTR_SET == (1u << 5) && V3D71_HUB_CTL_INT_STS_INT_GMPV_SET == (1u << 6) &&
		      V3D_MMU0_CACHE_CTL_CLEAR_SET == (1u << 3) && V3D_CTL_0_INT_STS_INT_CSDDONE_SET == (1u << 7),
	      "the 7.1 bits");
	CHECK(V3D_SMS_REE_CS == 0x0 && V3D_SMS_TEE_CS == 0x400 && V3D_SMS_CS_POWER_OFF_SET == (1u << 30) &&
		      V3D_SMS_CS_CLEAR_POWER_OFF_SET == (1u << 29) && V3D_SMS_STATE_IDLE == 0 &&
		      V3D_SMS_STATE_POWER_OFF == 0xd && BCM2712_PM_GRAFX == 0x304 &&
		      BCM2712_PM_GRAFX_V3D_RESET_N_SET == (1u << 6),
	      "the SMS and pm facts");
	/* a caller that names no SoC gets the BCM2711 */
	static const struct v3d_regs none;
	CHECK(none.soc == V3D_SOC_BCM2711 && v3d_soc_ver(&none) == 42 && v3d_soc_ver(&R71) == 71,
	      "the default SoC is the BCM2711: %d, vers %u %u", none.soc, v3d_soc_ver(&none), v3d_soc_ver(&R71));
}

static void t71_power_on(void) /* 9 checks */
{
	reset_model71();
	int ret = v3d_power_on(&R71, &CLK);
	CHECK(ret == 0, "power_on returned %d", ret);
	CHECK(nclk == 3 && clk_calls[0] == 1 && clk_calls[1] == 0 && clk_calls[2] == 1,
	      "clock sequence %d calls: %d %d %d", nclk, clk_calls[0], clk_calls[1], clk_calls[2]);
	/* clock on, off, pm's 2712 reset released, clock on, then the SMS:
	 * power up (TEE_CS), then its reset (REE_CS) */
	int k0 = find('K', 0, 0), k1 = find('K', 0, k0 + 1);
	int pmw = find('P', BCM2712_PM_GRAFX, 0);
	int k2 = find('K', 0, k1 + 1);
	int tee = find('S', V3D_SMS_TEE_CS, 0), ree = find('S', V3D_SMS_REE_CS, 0);
	CHECK(k0 >= 0 && k1 > k0 && pmw > k1 && k2 > pmw && tee > k2 && ree > tee,
	      "order k0 %d k1 %d pm %d k2 %d tee %d ree %d", k0, k1, pmw, k2, tee, ree);
	CHECK((val_at(pmw) & 0xff000000u) == BCM2711_PM_WRITE_KEY && (val_at(pmw) & BCM2712_PM_GRAFX_V3D_RESET_N_SET) &&
		      find('P', BCM2711_PM_GRAPHICS_CTL, 0) < 0,
	      "pm 0x304 write 0x%08x lacks the key or V3DRSTN, or the 2711 register was written", val_at(pmw));
	CHECK(val_at(tee) == V3D_SMS_CS_CLEAR_POWER_OFF_SET && val_at(ree) == V3D_SMS_STATE_RESET_REQUEST &&
		      sms_state == V3D_SMS_STATE_IDLE,
	      "SMS writes TEE_CS 0x%08x REE_CS 0x%08x, state 0x%x", val_at(tee), val_at(ree), sms_state);
	CHECK(no_bridge(), "a 7.1 power-on touched the bridge (%u reads)", asb_reads);

	/* an SMS that does not power up: -ETIMEDOUT after 100 ms, clock off
	 * and the reset asserted again */
	reset_model71();
	model.sms_stuck_off = true;
	ret = v3d_power_on(&R71, &CLK);
	CHECK(ret == -ETIMEDOUT && now_us >= V3D_SMS_TIMEOUT_US && now_us < 2 * V3D_SMS_TIMEOUT_US &&
		      !(pm[BCM2712_PM_GRAFX / 4] & BCM2712_PM_GRAFX_V3D_RESET_N_SET) && nclk == 4 && clk_calls[3] == 0 &&
		      find('S', V3D_SMS_REE_CS, 0) < 0,
	      "SMS stuck off: returned %d after %u us, pm 0x%x, %d clock calls", ret, now_us,
	      pm[BCM2712_PM_GRAFX / 4], nclk);

	/* a reset that never ends: the same */
	reset_model71();
	model.sms_reset_stuck = true;
	ret = v3d_power_on(&R71, &CLK);
	CHECK(ret == -ETIMEDOUT && now_us >= V3D_SMS_TIMEOUT_US && !(pm[BCM2712_PM_GRAFX / 4] & BCM2712_PM_GRAFX_V3D_RESET_N_SET),
	      "SMS reset stuck: returned %d after %u us, pm 0x%x", ret, now_us, pm[BCM2712_PM_GRAFX / 4]);

	/* a reset that takes a while is waited for */
	reset_model71();
	model.sms_reset_reads = 20;
	ret = v3d_power_on(&R71, &CLK);
	CHECK(ret == 0 && sms_state == V3D_SMS_STATE_IDLE && now_us >= 18 && now_us < 100,
	      "a slow SMS reset: returned %d, state 0x%x after %u us", ret, sms_state, now_us);
}

static void t71_power_off_reset(void) /* 5 checks */
{
	/* power off: SMS POWER_OFF, clock off, the reset asserted */
	reset_model71();
	CHECK(v3d_power_on(&R71, &CLK) == 0, "power on first");
	nlog = 0;
	nclk = 0;
	int ret = v3d_power_off(&R71, &CLK);
	int tee = find('S', V3D_SMS_TEE_CS, 0), k = find('K', 0, 0), pmw = find('P', BCM2712_PM_GRAFX, 0);
	CHECK(ret == 0 && tee >= 0 && k > tee && pmw > k && val_at(tee) == V3D_SMS_CS_POWER_OFF_SET && val_at(k) == 0 &&
		      (val_at(pmw) & 0xff000000u) == BCM2711_PM_WRITE_KEY &&
		      !(pm[BCM2712_PM_GRAFX / 4] & BCM2712_PM_GRAFX_V3D_RESET_N_SET) &&
		      sms_state == V3D_SMS_STATE_POWER_OFF && no_bridge(),
	      "power off returned %d, order tee %d (0x%08x) clock %d pm %d (0x%08x)", ret, tee, val_at(tee), k, pmw,
	      val_at(pmw));

	/* an SMS that never powers off: -ETIMEDOUT after 100 ms, clock and
	 * reset left alone */
	reset_model71();
	CHECK(v3d_power_on(&R71, &CLK) == 0, "power on again");
	model.sms_never_off = true;
	nlog = 0;
	nclk = 0;
	now_us = 0;
	ret = v3d_power_off(&R71, &CLK);
	CHECK(ret == -ETIMEDOUT && now_us >= V3D_SMS_TIMEOUT_US && nclk == 0 && find('P', BCM2712_PM_GRAFX, 0) < 0,
	      "SMS never off: returned %d after %u us, %d clock calls, pm written at %d", ret, now_us, nclk,
	      find('P', BCM2712_PM_GRAFX, 0));

	/* the whole reset: quiesce, SMS off, reset asserted, released, SMS up,
	 * the same page table */
	reset_model71();
	v3d_power_on(&R71, &CLK);
	nlog = 0;
	ret = v3d_reset(&R71, &CLK, 0x3f000000u, 0x3f080000u);
	int q = find('C', V3D_CTL_0_INT_MSK_SET, 0), off = find('S', V3D_SMS_TEE_CS, 0);
	int pa = find('P', BCM2712_PM_GRAFX, 0), pr = find('P', BCM2712_PM_GRAFX, pa + 1);
	int up = find('S', V3D_SMS_TEE_CS, off + 1), pt = find('H', V3D_MMU0_PT_PA_BASE, 0);
	CHECK(ret == 0 && q >= 0 && off > q && pa > off && pr > pa && up > pr && pt > up &&
		      val_at(off) == V3D_SMS_CS_POWER_OFF_SET && val_at(up) == V3D_SMS_CS_CLEAR_POWER_OFF_SET &&
		      val_at(pt) == (0x3f000000u >> 12) && no_bridge(),
	      "reset returned %d, order quiesce %d sms off %d pm %d %d sms up %d pt %d", ret, q, off, pa, pr, up, pt);
}

static void t71_identify(void) /* 5 checks */
{
	struct v3d_identity id;
	reset_model71();
	int ret = v3d_identify(&R71, &id);
	CHECK(ret == 0, "identify returned %d", ret);
	CHECK(id.ver == 71 && id.ncores == 1 && id.qpu_count == 16 && id.vpm_size == 65536 && id.pa_width == 40 &&
		      id.va_width == 32 && id.has_mmu && id.hub_ident[3] == 0x700 && id.ctl_ident[0] == 0x07443356u,
	      "decoded ver %u cores %u qpus %u vpm %u pa %u va %u mmu %d", id.ver, id.ncores, id.qpu_count,
	      id.vpm_size, id.pa_width, id.va_width, id.has_mmu);
	CHECK(nlog == 0, "identify wrote %u registers", nlog);

	/* a 4.2 where a 7.1 was selected is refused, and the other way round
	 * (t_identify) */
	reset_model71();
	hub[V3D_HUB_CTL_IDENT1 / 4] = (1u << 8) | (2u << 4) | 4u;
	ret = v3d_identify(&R71, &id);
	CHECK(ret == -ENODEV && id.ver == 42, "a 4.2 on the BCM2712 returned %d (ver %u)", ret, id.ver);

	/* the hub says 7.1 but the core's IDENT0 does not */
	reset_model71();
	core[V3D_CTL_0_IDENT0 / 4] = 0x04443356u;
	ret = v3d_identify(&R71, &id);
	CHECK(ret == -ENODEV && id.ver == 71, "a core IDENT0 of 4 returned %d", ret);
}

static void t71_mmu(void) /* 3 checks */
{
	reset_model71();
	int ret = v3d_mmu_init(&R71, 0x3e000000ull, 0x3e400000ull);
	CHECK(ret == 0, "mmu_init returned %d", ret);
	int base = find('H', V3D_MMU0_PT_PA_BASE, 0), ctl = find('H', V3D_MMU0_CTL, 0);
	int ill = find('H', V3D_MMU0_ILLEGAL_ADR, 0), mmuc = find('H', V3D_MMU0_CACHE_CTL, 0);
	int flush = find('H', V3D_MMU0_CACHE_CTL, mmuc + 1), tlb = find('H', V3D_MMU0_CTL, ctl + 1);
	CHECK(base >= 0 && ctl > base && ill > ctl && mmuc > ill && flush > mmuc && tlb > flush &&
		      val_at(base) == 0x3e000 && val_at(ill) == (0x3e400u | V3D_MMU0_ILLEGAL_ADR_ENABLE_SET) &&
		      (val_at(ctl) & V3D_MMU0_CTL_ENABLE_SET) && (val_at(tlb) & V3D_MMU0_CTL_TLB_CLEAR_SET),
	      "MMU order base %d ctl %d ill %d mmuc %d flush %d tlb %d", base, ctl, ill, mmuc, flush, tlb);
	/* the MMU cache is enabled and flushed, never cleared (bit 3 is not
	 * 7.1's clear, bit 11 is, and neither is written) */
	bool clean = true;
	for (unsigned i = 0; i < nlog; i++)
		if (wlog[i].blk == 'H' && wlog[i].off == V3D_MMU0_CACHE_CTL &&
		    (wlog[i].val & (V3D_MMU0_CACHE_CTL_CLEAR_SET | V3D71_MMU0_CACHE_CTL_CLEAR_SET)))
			clean = false;
	CHECK(clean && val_at(mmuc) == V3D_MMU0_CACHE_CTL_ENABLE_SET &&
		      val_at(flush) == (V3D_MMU0_CACHE_CTL_FLUSH_SET | V3D_MMU0_CACHE_CTL_ENABLE_SET),
	      "MMUC writes 0x%x then 0x%x", val_at(mmuc), val_at(flush));
}

static void t71_job(void) /* 10 checks */
{
	struct v3d_stall st;

	reset_model71();
	v3d_irq_quiesce(&R71);
	v3d_caches_init(&R71);
	nlog = 0;
	v3d_submit_bin(&R71, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	int clr = find('C', V3D_CTL_0_INT_CLR, 0), bpos = find('C', V3D_PTB_0_BPOS, 0);
	int inv = find('C', V3D_CTL_0_L2TCACTL, 0), qma = find('C', V3D_CLE_0_CT0QMA, 0);
	int qms = find('C', V3D_CLE_0_CT0QMS, 0), qts = find('C', V3D_CLE_0_CT0QTS, 0);
	int qba = find('C', V3D_CLE_0_CT0QBA, 0), qea = find('C', V3D_CLE_0_CT0QEA, 0);
	CHECK(clr == 0 && bpos == 1 && inv > bpos && qma > inv && qms > qma && qts > qms && qba > qts && qea > qba &&
		      qea == (int)nlog - 1 && val_at(qts) == (0x30000u | V3D_CLE_0_CT0QTS_CTQTSEN_SET) &&
		      val_at(qea) == 0x10011,
	      "bin order clr %d bpos %d inv %d qma %d qms %d qts %d qba %d qea %d of %u", clr, bpos, inv, qma, qms,
	      qts, qba, qea, nlog);
	int ret = v3d_wait(&R71, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == 0 && core[V3D_CTL_0_INT_STS / 4] == 0, "bin wait returned %d", ret);

	nlog = 0;
	v3d_submit_render(&R71, 0x11000, 0x1107f);
	qea = find('C', V3D_CLE_0_CT1QEA, 0);
	CHECK(qea == (int)nlog - 1 && find('C', V3D_CLE_0_CT1QBA, 0) < qea &&
		      find('C', V3D_CLE_0_CT1QBA, 0) > find('C', V3D_CTL_0_SLCACTL, 0) &&
		      find('C', V3D_CTL_0_INT_CLR, 0) == 0,
	      "render: clear, invalidate, then start, then end last");
	ret = v3d_wait(&R71, V3D_CTL_0_INT_STS_INT_FRDONE_SET, 1000, &st);
	CHECK(ret == 0, "render wait returned %d", ret);

	/* the four outcomes of a wait other than done: out of memory, an MMU
	 * fault, a timeout, and 7.1's hub GMPV */
	reset_model71();
	model.bin_raises_oom = true;
	v3d_submit_bin(&R71, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait(&R71, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == -ENOMEM && (st.core_int_sts & V3D_CTL_0_INT_STS_INT_OUTOMEM_SET), "OOM returned %d", ret);

	reset_model71();
	model.mmu_fault_on_bin = true;
	v3d_submit_bin(&R71, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait(&R71, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == -EFAULT && (st.hub_int_sts & V3D_HUB_CTL_INT_STS_INT_MMU_PTI_SET) && st.mmu_vio_id == 0xd &&
		      st.mmu_vio_addr == 0x1010000 && hub[V3D_HUB_CTL_INT_STS / 4] == 0,
	      "fault returned %d, vio_id 0x%x vio_addr 0x%x", ret, st.mmu_vio_id, st.mmu_vio_addr);

	reset_model71();
	model.render_never_ends = true;
	v3d_submit_render(&R71, 0x11000, 0x1107f);
	core[V3D_CLE_0_CT1CA / 4] = 0x11040;
	ret = v3d_wait(&R71, V3D_CTL_0_INT_STS_INT_FRDONE_SET, 500, &st);
	CHECK(ret == -ETIMEDOUT && st.ct1ca == 0x11040 && now_us >= 500, "timeout returned %d ct1ca 0x%x", ret,
	      st.ct1ca);

	reset_model71();
	model.gmpv_on_bin = true;
	v3d_submit_bin(&R71, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait(&R71, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 1000, &st);
	CHECK(ret == -EFAULT && (st.hub_int_sts & V3D71_HUB_CTL_INT_STS_INT_GMPV_SET) && hub[V3D_HUB_CTL_INT_STS / 4] == 0,
	      "hub GMPV returned %d, hub 0x%x", ret, st.hub_int_sts);
	/* the same bit is no fault of the 4.2's: its wait goes on to the end */
	reset_model();
	model.gmpv_on_bin = true;
	v3d_submit_bin(&R, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait(&R, V3D_CTL_0_INT_STS_INT_FLDONE_SET, 100, &st);
	CHECK(ret == -ETIMEDOUT, "hub bit 6 on the 4.2 path returned %d", ret);

	/* the binner's overflow pool, as on 4.2 */
	static const uint32_t blocks[1] = { 0x00700000u };
	struct v3d_bin_overflow ov = { blocks, 1, 256 * 1024, 0 };
	reset_model71();
	model.oom_blocks = 1;
	v3d_submit_bin(&R71, 0x10000, 0x10011, 0x20000, 0x4000, 0x30000);
	ret = v3d_wait_bin(&R71, 1000, &st, &ov);
	int a = find('C', V3D_PTB_0_BPOA, 0);
	CHECK(ret == 0 && ov.used == 1 && val_at(a) == blocks[0] &&
		      val_at(find('C', V3D_PTB_0_BPOS, a)) == 256 * 1024,
	      "overflow: returned %d, used %u", ret, ov.used);
}

static void t71_tfu(void) /* 1 check */
{
	static const struct v3d_tfu_job j = { .icfg = 0x3a00, .iia = 0x200000, .ioa = 0x100030 };
	struct v3d_tfu_status st;
	reset_model71();
	hub[V3D_TFU_CS / 4] = 0x12345678u; /* what a 4.2 TFU_CS read would see, never read */
	int ret = v3d_tfu_run(&R71, &j, 1000, &st);
	CHECK(ret == -ENOTSUP && nlog == 0 && st.cs_before == 0, "the 4.2 TFU job on a 7.1 returned %d after %u writes",
	      ret, nlog);
}

static int main71(void)
{
	const int planned = 4 + 9 + 5 + 5 + 3 + 10 + 1;
	t71_map();
	t71_power_on();
	t71_power_off_reset();
	t71_identify();
	t71_mmu();
	t71_job();
	t71_tfu();
	printf("hw_mock_test 7.1: %d checks run (planned %d), %d failed\n", checks, planned, failed);
	return (failed == 0 && checks == planned) ? 0 : 1;
}

int main(int argc, char **argv)
{
	if (argc > 1 && strcmp(argv[1], "--v71") == 0)
		return main71();
	if (argc > 1) {
		printf("usage: hw_mock_test [--v71]\n");
		return 2;
	}
	const int planned = 8 + 4 + 10 + 6 + 11 + 7 + 5 + 9 + 7;
	t_power_on();
	t_identify();
	t_pte();
	t_mmu();
	t_job();
	t_overflow();
	t_stale();
	t_reset();
	t_tfu();
	printf("hw_mock_test: %d checks run (planned %d), %d failed\n", checks, planned, failed);
	return (failed == 0 && checks == planned) ? 0 : 1;
}
