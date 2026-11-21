/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hw_mock_test: the VC4 V3D driver core (driver/src/vc4: vc4_hw.c,
 * vc4_power.c) against a software model of the V3D, PM and ASB registers
 * and of the firmware's property channel, as tools/v3d/hw_mock_test.c is for
 * the Pi 4's V3D. It checks what the core writes, in what order, and how it
 * reports each failure. The model answers only what the sequences wait for
 * and plays the interrupts a frame raises; it proves the driver follows the
 * sequences it claims (Linux v6.18's vc4 driver and the power drivers), not
 * that the silicon behaves as modelled -- only the board can say that.
 *
 * The firmware path goes through driver/zephyr/lib/board/rpi4/rpi_mbox.c,
 * unchanged, built for the BCM2837 (CONFIG_SOC_BCM2837) with the host stubs
 * of driver/zephyr/lib/board/test/stub: the property buffers the firmware
 * model reads are the ones the board would send.
 *
 *   cc -std=gnu11 -Wall -Wextra -Werror -DCONFIG_SOC_BCM2837 \
 *      -Idriver/src/vc4 -Idriver/zephyr/lib/board/test/stub \
 *      -Idriver/zephyr/lib/board/rpi4 tools/vc4/hw_mock_test.c \
 *      driver/src/vc4/vc4_hw.c driver/src/vc4/vc4_power.c \
 *      driver/zephyr/lib/board/rpi4/rpi_mbox.c -o hw_mock_test
 *
 * Cases: a wrong IDENT0; the init writes; the bin memory rule; one frame
 * (bin then render); an out-of-memory during binning (twice) and a dry pool;
 * a hang while rendering (power cycle) and while binning (thread reset); the
 * firmware power path (the property buffers, the rate, the domain probe, an
 * error answer); the PM/ASB power path (island off, island on, wrong bridge,
 * no POWOK, a bridge that never acknowledges, and the power-off).
 *
 * Prints "vc4 hw_mock_test: N checks run (planned N), 0 failed"; exit 0 only
 * then.
 */
#include <errno.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>

#include <zephyr/sys/device_mmio.h>

#include "rpi_mbox.h"
#include "vc4.h"

/* Every CHECK runs once per call site (none sits in a loop): 130 direct,
 * 26 EXPECT, 4 clock_tag_check of 2 each. */
#define PLANNED 164

static int checks, failed;
#define CHECK(cond, ...)                                                                       \
	do {                                                                                   \
		checks++;                                                                      \
		if (!(cond)) {                                                                 \
			failed++;                                                              \
			printf("  FAIL %s:%d: ", __func__, __LINE__);                          \
			printf(__VA_ARGS__);                                                   \
			printf("\n");                                                          \
		}                                                                              \
	} while (0)

/* ---- the log: every write, firmware tag and delay, in order ---- */

struct ev {
	char blk;     /* 'V' V3D, 'P' PM, 'A' ASB write; 'F' firmware tag; 'D' delay */
	uint32_t off; /* register offset, or the tag */
	uint32_t val; /* the value written, or the tag's first two words */
	uint32_t val2;
};
static struct ev elog[4096];
static unsigned nlog;

static void log_ev(char blk, uint32_t off, uint32_t val, uint32_t val2)
{
	if (nlog < sizeof(elog) / sizeof(elog[0]))
		elog[nlog++] = (struct ev){ blk, off, val, val2 };
}

#define W(b, o, v) { b, o, v, 0 }
#define F(t, a, b) { 'F', t, a, b }
#define DLY { 'D', 0, 1, 0 }

static void expect_log(const char *what, unsigned from, const struct ev *want, unsigned n)
{
	bool same = nlog - from == n;
	for (unsigned i = 0; same && i < n; i++) {
		const struct ev *g = &elog[from + i];
		same = g->blk == want[i].blk && g->off == want[i].off && g->val == want[i].val &&
		       (g->blk != 'F' || g->val2 == want[i].val2);
	}
	CHECK(same, "%s: the writes differ", what);
	if (!same) {
		printf("    got %u, want %u:\n", nlog - from, n);
		for (unsigned i = 0; i < nlog - from || i < n; i++) {
			if (i < nlog - from)
				printf("    got  %c 0x%03x 0x%08x 0x%08x", elog[from + i].blk, elog[from + i].off,
				       elog[from + i].val, elog[from + i].val2);
			else
				printf("    got  -");
			if (i < n)
				printf("   want %c 0x%03x 0x%08x 0x%08x\n", want[i].blk, want[i].off, want[i].val,
				       want[i].val2);
			else
				printf("\n");
		}
	}
}
#define EXPECT(what, from, ...)                                                                \
	do {                                                                                   \
		static const struct ev want_[] = { __VA_ARGS__ };                              \
		expect_log(what, from, want_, sizeof(want_) / sizeof(want_[0]));               \
	} while (0)

/* ---- the V3D model ---- */

#define MODEL_IDENT1 0xc0102431u /* VPM 12 KiB, 16 semaphores, 2 TMUs and 4 QPUs
				  * per slice, 3 slices, rev 1: model values */
#define MODEL_IDENT2 0x00000111u

static struct {
	uint32_t regs[0x1000 / 4];
	uint32_t status, enabled;
	bool running[2];
	bool oom_cause;       /* the binner has no overflow memory (level) */
	unsigned oom_rounds;  /* times the binner still runs out before it ends */
	bool bin_hangs, render_hangs;
	bool powered;
	uint32_t ident0;
	unsigned violations;  /* CTnCA/CTnEA written to a running thread */
	unsigned reads_ident;
} hw;

static void hw_power_reset(void)
{
	memset(hw.regs, 0, sizeof(hw.regs));
	hw.status = hw.enabled = 0;
	hw.running[0] = hw.running[1] = false;
	hw.oom_cause = false;
}

static void hw_reset(void)
{
	memset(&hw, 0, sizeof(hw));
	hw.ident0 = V3D_EXPECTED_IDENT0;
	hw.powered = true;
}

static uint32_t v3d_read(void *ctx, uint32_t off)
{
	(void)ctx;
	if (!hw.powered)
		return 0;
	switch (off) {
	case V3D_IDENT0:
		hw.reads_ident++;
		return hw.ident0;
	case V3D_IDENT1:
		hw.reads_ident++;
		return MODEL_IDENT1;
	case V3D_IDENT2:
		hw.reads_ident++;
		return MODEL_IDENT2;
	case V3D_INTCTL:
		return hw.status;
	case V3D_INTENA:
	case V3D_INTDIS:
		return hw.enabled;
	case V3D_CTNCS(0):
	case V3D_CTNCS(1):
		return hw.running[(off - V3D_CTNCS(0)) / 4] ? V3D_CTRUN : 0;
	default:
		return hw.regs[off / 4];
	}
}

static void v3d_write(void *ctx, uint32_t off, uint32_t v)
{
	(void)ctx;
	log_ev('V', off, v, 0);
	if (!hw.powered)
		return;
	switch (off) {
	case V3D_INTCTL:
		hw.status &= ~v;
		if (hw.oom_cause)
			hw.status |= V3D_INT_OUTOMEM; /* level-like: stays while the cause does */
		return;
	case V3D_INTENA:
		hw.enabled |= v & 0xfu;
		return;
	case V3D_INTDIS:
		hw.enabled &= ~v;
		return;
	case V3D_CTNCA(0):
	case V3D_CTNCA(1):
		if (hw.running[(off - V3D_CTNCA(0)) / 4])
			hw.violations++;
		hw.regs[off / 4] = v;
		return;
	case V3D_CTNEA(0):
	case V3D_CTNEA(1): {
		unsigned t = (off - V3D_CTNEA(0)) / 4;
		if (hw.running[t])
			hw.violations++;
		hw.regs[off / 4] = v;
		hw.running[t] = true;
		return;
	}
	case V3D_CTNCS(0):
	case V3D_CTNCS(1):
		if (v & V3D_CTRSTA)
			hw.running[(off - V3D_CTNCS(0)) / 4] = false;
		return;
	case V3D_BPOS:
		hw.regs[off / 4] = v;
		if (v)
			hw.oom_cause = false; /* the binner has memory again */
		return;
	default:
		hw.regs[off / 4] = v;
		return;
	}
}

static const struct vc4_io v3d_io = { v3d_read, v3d_write, NULL, NULL };

/* One step of the hardware: each running thread advances, stalls or ends. */
static void hw_step(void)
{
	if (hw.running[0] && !hw.bin_hangs && !hw.oom_cause) {
		hw.regs[V3D_CTNCA(0) / 4] += 0x10;
		if (hw.oom_rounds) {
			hw.oom_rounds--;
			hw.oom_cause = true;
			hw.status |= V3D_INT_OUTOMEM;
		} else {
			hw.running[0] = false;
			hw.regs[V3D_CTNCA(0) / 4] = hw.regs[V3D_CTNEA(0) / 4];
			hw.status |= V3D_INT_FLDONE;
		}
	}
	if (hw.running[1] && !hw.render_hangs) {
		hw.running[1] = false;
		hw.regs[V3D_CTNCA(1) / 4] = hw.regs[V3D_CTNEA(1) / 4];
		hw.status |= V3D_INT_FRDONE;
	}
}

static bool irq_line(void)
{
	return (hw.status & hw.enabled) != 0;
}

/* ---- the PM and ASB model ---- */

static struct {
	uint32_t pm[0x114 / 4], asb[0x24 / 4];
	bool powok_never, asb_m_ack_sticks;
	unsigned no_password;
} pmm;

static uint32_t pm_read(void *ctx, uint32_t off)
{
	(void)ctx;
	return pmm.pm[off / 4];
}

static void pm_write(void *ctx, uint32_t off, uint32_t v)
{
	(void)ctx;
	log_ev('P', off, v, 0);
	if ((v & 0xff000000u) != PM_PASSWORD)
		pmm.no_password++;
	v &= 0x00ffffffu;
	if (off == PM_GRAFX) {
		v &= ~(PM_POWOK | PM_MRDONE); /* status bits, not written */
		if ((v & PM_POWUP) && !pmm.powok_never)
			v |= PM_POWOK;
		if (v & PM_MEMREP)
			v |= PM_MRDONE;
	}
	pmm.pm[off / 4] = v;
}

static uint32_t asb_read(void *ctx, uint32_t off)
{
	(void)ctx;
	return pmm.asb[off / 4];
}

static void asb_write(void *ctx, uint32_t off, uint32_t v)
{
	(void)ctx;
	log_ev('A', off, v, 0);
	if ((v & 0xff000000u) != PM_PASSWORD)
		pmm.no_password++;
	v &= 0x00ffffffu;
	if (v & ASB_REQ_STOP)
		v |= ASB_ACK;
	else if (!(pmm.asb_m_ack_sticks && off == ASB_V3D_M_CTRL))
		v &= ~ASB_ACK;
	pmm.asb[off / 4] = v;
}

static void pm_delay(void *ctx, uint32_t us)
{
	(void)ctx;
	log_ev('D', 0, us, 0);
}

static const struct vc4_io pm_io = { pm_read, pm_write, pm_delay, NULL };
static const struct vc4_io asb_io = { asb_read, asb_write, pm_delay, NULL };

static void pm_reset(void)
{
	memset(&pmm, 0, sizeof(pmm));
	/* both bridge ports stopped, as after the island was off */
	pmm.asb[ASB_V3D_M_CTRL / 4] = ASB_REQ_STOP | ASB_ACK;
	pmm.asb[ASB_V3D_S_CTRL / 4] = ASB_REQ_STOP | ASB_ACK;
	pmm.asb[ASB_AXI_BRDG_ID / 4] = ASB_BRDG_ID;
}

/* ---- the mailbox and the firmware (what rpi_mbox.c talks to) ---- */

#define MBOX_PHYS_EXPECTED 0x3f00b880ull
#define BUF_PHYS 0x00100000u
#define FW_MAX_HZ 300000000u

static uint32_t mbox_regs[0x40 / 4];
static uint32_t *fw_buf;          /* rpi_mbox.c's message buffer */
static unsigned long long mbox_map_phys;
static uint32_t mail1_word, mail0_pending;
static unsigned fw_layout_bad, fw_clock_third_word;
static uint32_t first_msg[8];
static bool first_msg_taken;
static struct {
	bool error;          /* answer every request with an error code */
	bool old_interface;  /* GET_DOMAIN_STATE unknown: leave it as sent */
	bool domain_on, clock_on;
	uint32_t rate;
} fw;

void device_map(mm_reg_t *virt, unsigned long long phys, unsigned long size, unsigned int flags)
{
	(void)size;
	(void)flags;
	mbox_map_phys = phys;
	*virt = (mm_reg_t)mbox_regs;
}

uintptr_t k_mem_phys_addr(void *virt)
{
	fw_buf = virt;
	return BUF_PHYS;
}

int sys_cache_data_flush_range(void *addr, size_t size)
{
	(void)addr;
	(void)size;
	return 0;
}

int sys_cache_data_invd_range(void *addr, size_t size)
{
	(void)addr;
	(void)size;
	return 0;
}

void k_busy_wait(uint32_t usec)
{
	(void)usec;
}

uint64_t k_cycle_get_64(void)
{
	static uint64_t c;
	return c += 10;
}

int64_t k_uptime_get(void)
{
	return 0;
}

/* The firmware walks the tags (raspberrypi.c:88-118 lays them out: size,
 * request code, then tag, value bytes, request code, values; a 0 end tag)
 * and answers each; the V3D's power follows the domain. */
static void firmware(uint32_t *b)
{
	uint32_t n = 2;
	if (!first_msg_taken) {
		memcpy(first_msg, b, sizeof(first_msg));
		first_msg_taken = true;
	}
	if (b[0] % 4 || b[1] != 0) {
		fw_layout_bad++;
		return;
	}
	while (n < b[0] / 4 && b[n] != 0) {
		uint32_t tag = b[n], bytes = b[n + 1], *v = &b[n + 3];
		if (bytes % 4 || b[n + 2] != 0 || n + 3 + bytes / 4 >= b[0] / 4) {
			fw_layout_bad++;
			return;
		}
		log_ev('F', tag, v[0], bytes >= 8 ? v[1] : 0);
		bool clock_tag = (tag & 0xffff0fffu) == 0x00030001u || (tag & 0xffff0fffu) == 0x00030002u ||
				 (tag & 0xffff0fffu) == 0x00030004u;
		if (clock_tag && (bytes != 12 || v[2] != 0))
			fw_clock_third_word++;
		switch (tag) {
		case RPI_TAG_SET_DOMAIN_STATE:
			if (v[0] == VC4_FW_DOMAIN_V3D) {
				fw.domain_on = v[1] & 1u;
				hw.powered = fw.domain_on;
				if (!hw.powered)
					hw_power_reset();
			}
			break;
		case RPI_TAG_GET_DOMAIN_STATE:
			if (!fw.old_interface)
				v[1] = v[0] == VC4_FW_DOMAIN_V3D ? fw.domain_on : 0;
			break;
		case RPI_TAG_SET_CLOCK_STATE:
			if (v[0] == VC4_FW_CLK_V3D)
				fw.clock_on = v[1] & RPI_CLOCK_STATE_ON;
			break;
		case RPI_TAG_GET_MAX_CLOCK_RATE:
			v[1] = v[0] == VC4_FW_CLK_V3D ? FW_MAX_HZ : 0;
			break;
		case RPI_TAG_SET_CLOCK_RATE:
			if (v[0] == VC4_FW_CLK_V3D)
				fw.rate = v[1];
			break;
		default:
			break;
		}
		b[n + 2] = 0x80000000u | bytes; /* answered */
		n += 3 + bytes / 4;
	}
	if (n >= b[0] / 4 || b[n] != 0 || (n + 1) * 4 != b[0])
		fw_layout_bad++;
	b[1] = fw.error ? 0x80000001u : 0x80000000u;
}

uint32_t sys_read32(mm_reg_t addr)
{
	uint32_t off = (uint32_t)(addr - (mm_reg_t)mbox_regs);
	switch (off) {
	case 0x38: /* MAIL1_STA: never full */
		return 0;
	case 0x18: /* MAIL0_STA: empty unless an answer waits */
		return mail0_pending ? 0 : 0x40000000u;
	case 0x00: { /* MAIL0_RD */
		uint32_t m = mail0_pending;
		mail0_pending = 0;
		return m;
	}
	default:
		return 0;
	}
}

void sys_write32(uint32_t val, mm_reg_t addr)
{
	uint32_t off = (uint32_t)(addr - (mm_reg_t)mbox_regs);
	if (off != 0x20) /* MAIL1_WRT */
		return;
	mail1_word = val;
	if (fw_buf && (val & ~0xfu) == (BUF_PHYS | 0xc0000000u))
		firmware(fw_buf);
	mail0_pending = val; /* the same word back on channel 8 */
}

static void fw_reset(void)
{
	memset(&fw, 0, sizeof(fw));
	fw.domain_on = true;
	fw.clock_on = true;
	fw_layout_bad = fw_clock_third_word = 0;
}

/* ---- helpers ---- */

static void clock_tag_check(const char *what)
{
	CHECK(fw_clock_third_word == 0, "%s: a clock tag without its three words {id, value, 0}", what);
	CHECK(fw_layout_bad == 0, "%s: a property buffer the firmware could not walk", what);
}

/* Play the hardware until the job ends or `steps` run out, taking each
 * interrupt as the board's handler would. Returns the events seen. */
static uint32_t run(struct vc4_job *job, struct vc4_bin_mem *m, unsigned steps, int *give_ret)
{
	uint32_t evs = 0;
	for (unsigned s = 0; s < steps && job->state != VC4_JOB_DONE && job->state != VC4_JOB_FAILED; s++) {
		hw_step();
		for (int guard = 0; irq_line() && guard < 8; guard++) {
			uint32_t ev = vc4_irq(&v3d_io, job);
			evs |= ev;
			if (ev & VC4_EV_OOM) {
				int r = vc4_overflow_give(&v3d_io, m, job);
				if (give_ret)
					*give_ret = r;
			}
		}
	}
	return evs;
}

static struct vc4_power_fw fwp = { rpi_mbox_tag, 0, 0 };
static const struct vc4_power fw_power = { vc4_power_fw_on, vc4_power_fw_off, &fwp };

/* ---- the cases ---- */

static void case_ident_wrong(void)
{
	struct vc4_ident id;
	hw_reset();
	hw.ident0 = 0x02443355u; /* "V3U": not the VC4 V3D */
	unsigned from = nlog;
	CHECK(vc4_init(&v3d_io, &id) == -ENODEV, "a wrong IDENT0 is refused");
	CHECK(nlog == from, "nothing is written after a wrong IDENT0 (%u writes)", nlog - from);
	CHECK(hw.reads_ident == 3, "IDENT0, IDENT1 and IDENT2 are read (%u reads)", hw.reads_ident);
	CHECK(id.ident0 == 0x02443355u && id.ident1 == MODEL_IDENT1 && id.ident2 == MODEL_IDENT2,
	      "the record holds the three words");
	hw.ident0 = 0x00000000u; /* an unpowered block reads 0 */
	CHECK(vc4_identify(&v3d_io, &id) == -ENODEV, "IDENT0 0 is refused");
	hw.ident0 = 0x03443356u; /* "V3D" with another technology version */
	CHECK(vc4_identify(&v3d_io, &id) == -ENODEV, "technology version 3 is refused");
	CHECK(nlog == from, "identification writes nothing");
}

static void case_init(void)
{
	struct vc4_ident id;
	hw_reset();
	unsigned from = nlog;
	CHECK(vc4_init(&v3d_io, &id) == 0, "init with the right identity");
	CHECK(id.ident0 == 0x02443356u, "IDENT0 0x%08x", id.ident0);
	CHECK(id.nslc == 3 && id.qups == 4 && id.tups == 2 && id.nsem == 16 && id.vpm_kb == 12 && id.rev == 1,
	      "IDENT1's fields: slices %u QPUs %u TMUs %u sems %u VPM %u rev %u", id.nslc, id.qups, id.tups,
	      id.nsem, id.vpm_kb, id.rev);
	EXPECT("init", from,
	       W('V', V3D_VPMBASE, 0), W('V', V3D_BPOA, 0), W('V', V3D_BPOS, 0),
	       W('V', V3D_INTCTL, 0x7), W('V', V3D_INTENA, 0x3));
	from = nlog;
	vc4_bin_mem_enable(&v3d_io);
	EXPECT("OUTOMEM enabled once there is bin memory", from, W('V', V3D_INTENA, 0x4));
	CHECK(hw.enabled == 0x7, "FLDONE, FRDONE and OUTOMEM enabled (0x%x)", hw.enabled);
	from = nlog;
	vc4_irq_disable(&v3d_io);
	EXPECT("irq disable", from, W('V', V3D_INTDIS, 0x7), W('V', V3D_INTCTL, 0x7));
	CHECK(hw.enabled == 0, "all masked");
}

static void case_bin_range(void)
{
	uint32_t p = 0;
	const uint32_t MB = 1024u * 1024u;
	CHECK(vc4_bin_range_pick(0x08000000u, 64 * MB, VC4_BIN_SIZE, &p) == 0 && p == 0x08000000u,
	      "a free range inside one 256 MB block: its start (0x%08x)", p);
	CHECK(vc4_bin_range_pick(0x08000123u, 64 * MB, VC4_BIN_SIZE, &p) == 0 && p == 0x08001000u,
	      "an unaligned start rounds up to 4 KiB (0x%08x)", p);
	CHECK(vc4_bin_range_pick(0x0ff00000u, 64 * MB, VC4_BIN_SIZE, &p) == 0 && p == 0x10000000u,
	      "a buffer that would cross 256 MB moves to the next block (0x%08x)", p);
	CHECK(vc4_bin_range_pick(0x0f800000u, 16 * MB, VC4_BIN_SIZE, &p) == -ERANGE,
	      "16 MiB across the boundary with no room after it: refused");
	CHECK(vc4_bin_range_pick(0x0f000000u, 16 * MB, VC4_BIN_SIZE, &p) == 0 && p == 0x0f000000u,
	      "a buffer that ends exactly at 256 MB is inside (0x%08x)", p);
	CHECK(vc4_bin_range_pick(0x1ff00000u, 1 * MB, VC4_BIN_SIZE, &p) == -ERANGE,
	      "a free range smaller than the buffer: refused");
	CHECK(vc4_bin_range_pick(0x3e000000u, 64 * MB, VC4_BIN_SIZE, &p) == 0 && p == 0x3e000000u,
	      "a buffer that ends at the bus window's end (0x3f000000) is inside");
	CHECK(vc4_bin_range_pick(0x3e800000u, 64 * MB, VC4_BIN_SIZE, &p) == -ERANGE,
	      "a buffer past the bus window (0x3f000000) is refused");
	CHECK(vc4_bin_range_pick(0x40000000u, 64 * MB, VC4_BIN_SIZE, &p) == -ERANGE,
	      "memory above the window is refused");
	CHECK(vc4_bin_range_pick(0x08000000u, 64 * MB, 0, &p) == -EINVAL, "size 0: -EINVAL");
	CHECK(vc4_bin_range_pick(0x08000000u, 64 * MB, VC4_BIN_SLOT_SIZE + 4096, &p) == -EINVAL,
	      "not whole slices: -EINVAL");
	CHECK(vc4_bin_range_pick(0x08000000u, 64 * MB, 32 * MB, &p) == -EINVAL, "more than 32 slices: -EINVAL");
	CHECK(vc4_bin_range_pick(0x08000000u, 64 * MB, 2 * VC4_BIN_SLOT_SIZE, &p) == 0 && p == 0x08000000u,
	      "a smaller buffer of whole slices is fine");
	CHECK(vc4_bin_range_pick(0xfffff000ull, 0x2000ull, VC4_BIN_SLOT_SIZE, &p) == -ERANGE,
	      "a range near the top of the address space is refused, not wrapped");

	struct vc4_bin_mem m;
	CHECK(vc4_bin_mem_init(&m, 0x0ff00000u, VC4_BIN_SIZE) == -EINVAL, "init refuses a buffer across 256 MB");
	CHECK(vc4_bin_mem_init(&m, 0x08000800u, VC4_BIN_SIZE) == -EINVAL, "init refuses an unaligned buffer");
	CHECK(vc4_bin_mem_init(&m, 0x08000000u, VC4_BIN_SIZE) == 0 && m.nslots == 32, "init: 32 slices");
	CHECK(vc4_bin_slot_bus(&m, 0) == 0xc8000000u, "slice 0's bus address (0x%08x)", vc4_bin_slot_bus(&m, 0));
	CHECK(vc4_bin_slot_bus(&m, 31) == 0xc8f80000u, "slice 31's bus address (0x%08x)",
	      vc4_bin_slot_bus(&m, 31));
	int s0 = vc4_bin_slot_get(&m), s1 = vc4_bin_slot_get(&m);
	CHECK(s0 == 0 && s1 == 1, "slices are taken lowest first (%d, %d)", s0, s1);
	vc4_bin_slots_put(&m, VC4_BIT(0));
	CHECK(vc4_bin_slot_get(&m) == 0, "a freed slice is taken again");
	m.used = 0xffffffffu;
	CHECK(vc4_bin_slot_get(&m) == -ENOMEM, "a full pool: -ENOMEM");
}

/* The control lists of the frame cases (ARM physical, handed as bus). */
#define BCL_PHYS 0x09000000u
#define RCL_PHYS 0x09100000u

static void frame_setup(struct vc4_bin_mem *m, struct vc4_job *job)
{
	struct vc4_ident id;
	hw_reset();
	vc4_init(&v3d_io, &id);
	vc4_bin_mem_init(m, 0x08000000u, VC4_BIN_SIZE);
	vc4_bin_mem_enable(&v3d_io);
	memset(job, 0, sizeof(*job));
	int slot = vc4_bin_slot_get(m); /* the job's tile state + tile alloc */
	job->bin_slots = VC4_BIT((uint32_t)slot);
	job->ct0ca = vc4_bus(BCL_PHYS);
	job->ct0ea = vc4_bus(BCL_PHYS + 0x200);
	job->ct1ca = vc4_bus(RCL_PHYS);
	job->ct1ea = vc4_bus(RCL_PHYS + 0x400);
}

static void case_frame(void)
{
	struct vc4_bin_mem m;
	struct vc4_job job;
	frame_setup(&m, &job);
	unsigned from = nlog;
	CHECK(vc4_job_submit(&v3d_io, &job) == 0 && job.state == VC4_JOB_BINNING, "the bin list starts");
	EXPECT("bin start", from,
	       W('V', V3D_L2CACTL, 0x4), W('V', V3D_SLCACTL, 0x0f0f0f0f),
	       W('V', V3D_CTNCA(0), 0xc9000000u), W('V', V3D_CTNEA(0), 0xc9000200u));
	CHECK(vc4_job_submit(&v3d_io, &job) == -EINVAL, "a running job is not submitted again");
	CHECK(vc4_hangcheck(&v3d_io, &job) == VC4_HANG_PROGRESS, "the first check after a start is progress");

	from = nlog;
	hw_step(); /* the binner ends: FLDONE */
	CHECK(irq_line(), "FLDONE raises the line");
	uint32_t ev = vc4_irq(&v3d_io, &job);
	CHECK(ev == VC4_EV_RENDER_STARTED && job.state == VC4_JOB_RENDERING, "FLDONE starts the render (ev 0x%x)",
	      ev);
	EXPECT("FLDONE", from,
	       W('V', V3D_INTCTL, 0x2), W('V', V3D_L2CACTL, 0x4), W('V', V3D_SLCACTL, 0x0f0f0000),
	       W('V', V3D_CTNCA(1), 0xc9100000u), W('V', V3D_CTNEA(1), 0xc9100400u));
	CHECK(!irq_line(), "FLDONE cleared");

	from = nlog;
	hw_step(); /* the renderer ends: FRDONE */
	ev = vc4_irq(&v3d_io, &job);
	CHECK(ev == VC4_EV_FRAME_DONE && job.state == VC4_JOB_DONE, "FRDONE completes the frame (ev 0x%x)", ev);
	EXPECT("FRDONE", from, W('V', V3D_INTCTL, 0x1));
	CHECK(!irq_line(), "FRDONE cleared");
	CHECK(hw.violations == 0, "no list address written to a running thread");
	CHECK(vc4_hangcheck(&v3d_io, &job) == VC4_HANG_IDLE, "a done job is not watched");
	vc4_job_retire(&m, &job);
	CHECK(m.used == 0, "the job's slice is free again (0x%x)", m.used);

	/* an FRDONE with no render running is reported, not acted on */
	from = nlog;
	hw.status |= V3D_INT_FRDONE;
	ev = vc4_irq(&v3d_io, &job);
	CHECK(ev == VC4_EV_SPURIOUS && job.state == VC4_JOB_DONE, "a stray FRDONE is spurious");
	EXPECT("stray FRDONE", from, W('V', V3D_INTCTL, 0x1));

	/* a job with an empty bin list goes straight to the render list */
	struct vc4_job r = job;
	r.state = VC4_JOB_IDLE;
	r.ct0ea = r.ct0ca;
	from = nlog;
	CHECK(vc4_job_submit(&v3d_io, &r) == 0 && r.state == VC4_JOB_RENDERING, "no binning: render at once");
	EXPECT("render only", from,
	       W('V', V3D_L2CACTL, 0x4), W('V', V3D_SLCACTL, 0x0f0f0f0f),
	       W('V', V3D_L2CACTL, 0x4), W('V', V3D_SLCACTL, 0x0f0f0000),
	       W('V', V3D_CTNCA(1), 0xc9100000u), W('V', V3D_CTNEA(1), 0xc9100400u));

	/* the guide's rule: a running thread is not restarted, nothing written */
	struct vc4_job busy = job;
	busy.state = VC4_JOB_IDLE;
	hw.running[0] = true;
	from = nlog;
	CHECK(vc4_job_submit(&v3d_io, &busy) == -EBUSY, "the binner runs: -EBUSY");
	CHECK(vc4_submit_cl(&v3d_io, VC4_THREAD_BIN, 1, 2) == -EBUSY, "submit_cl to a running thread: -EBUSY");
	CHECK(vc4_submit_cl(&v3d_io, 2, 1, 2) == -EINVAL, "no thread 2");
	CHECK(nlog == from, "a refused start writes nothing");
	hw.running[0] = false;
	from = nlog;
	CHECK(vc4_submit_cl(&v3d_io, VC4_THREAD_BIN, 0xc9000000u, 0xc9000010u) == 0, "a stopped thread starts");
	EXPECT("submit_cl", from, W('V', V3D_CTNCA(0), 0xc9000000u), W('V', V3D_CTNEA(0), 0xc9000010u));

	/* FLDONE while the renderer still runs: the job fails, nothing started */
	frame_setup(&m, &job);
	vc4_job_submit(&v3d_io, &job);
	hw_step();
	hw.running[1] = true;
	from = nlog;
	ev = vc4_irq(&v3d_io, &job);
	CHECK(ev == VC4_EV_BUSY && job.state == VC4_JOB_FAILED, "render thread busy at FLDONE: FAILED");
	EXPECT("FLDONE, renderer busy", from, W('V', V3D_INTCTL, 0x2));
}

static void case_oom(void)
{
	struct vc4_bin_mem m;
	struct vc4_job job;

	/* The model is level-like: clearing OUTOMEM before its cause is gone
	 * leaves it set, so the order checked below means something. */
	hw_reset();
	hw.oom_cause = true;
	hw.status = V3D_INT_OUTOMEM;
	v3d_write(NULL, V3D_INTCTL, V3D_INT_OUTOMEM);
	CHECK(hw.status & V3D_INT_OUTOMEM, "the model keeps OUTOMEM while the binner lacks memory");

	frame_setup(&m, &job);
	hw.oom_rounds = 2;
	vc4_job_submit(&v3d_io, &job);
	unsigned from = nlog;
	hw_step(); /* the binner runs out */
	CHECK(irq_line() && (hw.status & V3D_INT_OUTOMEM), "OUTOMEM raises the line");
	uint32_t ev = vc4_irq(&v3d_io, &job);
	CHECK(ev == VC4_EV_OOM, "OUTOMEM reported (ev 0x%x)", ev);
	CHECK(hw.status & V3D_INT_OUTOMEM, "the acknowledge does not clear OUTOMEM: its cause is still there");
	CHECK(!irq_line(), "OUTOMEM masked: the line drops");
	EXPECT("OUTOMEM in the handler", from, W('V', V3D_INTCTL, 0x4), W('V', V3D_INTDIS, 0x4));
	from = nlog;
	CHECK(vc4_overflow_give(&v3d_io, &m, &job) == 0, "an overflow slice is given");
	EXPECT("the first overflow slice", from,
	       W('V', V3D_BPOA, 0xc8080000u), W('V', V3D_BPOS, VC4_BIN_SLOT_SIZE),
	       W('V', V3D_INTCTL, 0x4), W('V', V3D_INTENA, 0x4));
	CHECK(!(hw.status & V3D_INT_OUTOMEM), "OUTOMEM cleared once the binner has memory");
	CHECK(m.overflow == VC4_BIT(1) && m.used == 0x3, "slice 1 is the overflow (used 0x%x)", m.used);

	from = nlog;
	hw_step(); /* out again */
	ev = vc4_irq(&v3d_io, &job);
	CHECK(ev == VC4_EV_OOM, "OUTOMEM again");
	CHECK(vc4_overflow_give(&v3d_io, &m, &job) == 0, "a second slice");
	EXPECT("the second overflow slice", from,
	       W('V', V3D_INTCTL, 0x4), W('V', V3D_INTDIS, 0x4),
	       W('V', V3D_BPOA, 0xc8100000u), W('V', V3D_BPOS, VC4_BIN_SLOT_SIZE),
	       W('V', V3D_INTCTL, 0x4), W('V', V3D_INTENA, 0x4));
	CHECK(job.bin_slots == 0x3, "the replaced slice goes with the job (0x%x)", job.bin_slots);

	uint32_t evs = run(&job, &m, 8, NULL);
	CHECK(job.state == VC4_JOB_DONE && (evs & VC4_EV_FRAME_DONE) && !(evs & VC4_EV_OOM),
	      "the frame then ends (state %d, evs 0x%x)", job.state, evs);
	vc4_job_retire(&m, &job);
	CHECK(m.used == VC4_BIT(2) && m.overflow == VC4_BIT(2),
	      "after the job only the binner's current slice is in use (0x%x)", m.used);
	CHECK(hw.violations == 0, "no list address written to a running thread");

	/* a dry pool: -ENOMEM, OUTOMEM stays masked, nothing written */
	frame_setup(&m, &job);
	hw.oom_rounds = 1;
	m.used = 0xffffffffu;
	vc4_job_submit(&v3d_io, &job);
	hw_step();
	vc4_irq(&v3d_io, &job);
	from = nlog;
	CHECK(vc4_overflow_give(&v3d_io, &m, &job) == -ENOMEM, "no free slice: -ENOMEM");
	CHECK(nlog == from, "nothing written without a slice");
	CHECK(!irq_line() && !(hw.enabled & V3D_INT_OUTOMEM), "OUTOMEM stays masked");
	hw_step();
	CHECK(job.state == VC4_JOB_BINNING && hw.running[0], "the binner stalls (the hang check takes over)");

	/* with no job running, the replaced slice is freed at once */
	frame_setup(&m, &job);
	CHECK(vc4_overflow_give(&v3d_io, &m, NULL) == 0 && vc4_overflow_give(&v3d_io, &m, NULL) == 0,
	      "two slices with no job");
	CHECK(m.used == (VC4_BIT(0) | VC4_BIT(2)) && m.overflow == VC4_BIT(2),
	      "the first overflow slice went back to the pool (0x%x)", m.used);
}

static void case_hang(void)
{
	struct vc4_bin_mem m;
	struct vc4_job job;

	/* Hung while rendering: Linux's power cycle through the firmware */
	frame_setup(&m, &job);
	hw.render_hangs = true;
	fw_reset();
	vc4_job_submit(&v3d_io, &job);
	run(&job, &m, 1, NULL);
	CHECK(job.state == VC4_JOB_RENDERING, "the render list started");
	CHECK(vc4_hangcheck(&v3d_io, &job) == VC4_HANG_PROGRESS, "first check: progress (CT1CA moved from 0)");
	run(&job, &m, 3, NULL);
	unsigned from = nlog;
	CHECK(vc4_hangcheck(&v3d_io, &job) == VC4_HANG_HUNG, "second check: neither CTnCA moved: hung");
	CHECK(nlog == from, "the hang check writes nothing");
	CHECK(vc4_reset_choose(&fw_power, false) == VC4_RESET_POWER_CYCLE, "with a power switch: power cycle");
	CHECK(vc4_reset_choose(&fw_power, true) == VC4_RESET_THREAD, "asked for: the thread reset");
	CHECK(vc4_reset_choose(NULL, false) == VC4_RESET_THREAD, "without a power switch: the thread reset");
	fwp.rate_hz = 0;
	from = nlog;
	CHECK(vc4_reset(&v3d_io, &fw_power, VC4_RESET_POWER_CYCLE, &job, &m) == 0, "the power cycle");
	EXPECT("power-cycle reset", from,
	       W('V', V3D_INTDIS, 0x7), W('V', V3D_INTCTL, 0x7),
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0), F(RPI_TAG_SET_DOMAIN_STATE, VC4_FW_DOMAIN_V3D, 0),
	       F(RPI_TAG_SET_DOMAIN_STATE, VC4_FW_DOMAIN_V3D, 1), F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1),
	       W('V', V3D_VPMBASE, 0), W('V', V3D_INTENA, 0x3),
	       W('V', V3D_INTCTL, 0x7), W('V', V3D_INTENA, 0x7));
	clock_tag_check("power cycle");
	CHECK(!hw.running[0] && !hw.running[1], "the power cycle stopped both threads");
	CHECK(hw.enabled == 0x7, "all three interrupts on after the reset (0x%x)", hw.enabled);
	CHECK(job.state == VC4_JOB_FAILED && m.used == 0 && m.overflow == 0, "the job is cancelled, its slices free");

	/* the same hang, the thread reset */
	frame_setup(&m, &job);
	hw.render_hangs = true;
	vc4_job_submit(&v3d_io, &job);
	run(&job, &m, 1, NULL);
	vc4_hangcheck(&v3d_io, &job);
	run(&job, &m, 2, NULL);
	CHECK(vc4_hangcheck(&v3d_io, &job) == VC4_HANG_HUNG, "hung again");
	from = nlog;
	CHECK(vc4_reset(&v3d_io, NULL, VC4_RESET_THREAD, &job, &m) == 0, "the thread reset");
	EXPECT("thread reset, renderer", from,
	       W('V', V3D_CTNCS(1), V3D_CTRSTA), W('V', V3D_BPOA, 0), W('V', V3D_BPOS, 0),
	       W('V', V3D_INTCTL, 0x7), W('V', V3D_INTENA, 0x7));
	CHECK(!hw.running[1] && job.state == VC4_JOB_FAILED, "the renderer stopped, the job failed");

	/* hung while binning: the binner's thread is the one reset */
	frame_setup(&m, &job);
	hw.bin_hangs = true;
	vc4_job_submit(&v3d_io, &job);
	hw.regs[V3D_CTNCA(0) / 4] = 0xc9000040u; /* it went a little way and stopped */
	CHECK(vc4_hangcheck(&v3d_io, &job) == VC4_HANG_PROGRESS, "binner: first check progress");
	run(&job, &m, 2, NULL);
	CHECK(vc4_hangcheck(&v3d_io, &job) == VC4_HANG_HUNG, "binner: hung");
	from = nlog;
	CHECK(vc4_reset(&v3d_io, NULL, VC4_RESET_THREAD, &job, &m) == 0, "the binner's reset");
	EXPECT("thread reset, binner", from,
	       W('V', V3D_CTNCS(0), V3D_CTRSTA), W('V', V3D_BPOA, 0), W('V', V3D_BPOS, 0),
	       W('V', V3D_INTCTL, 0x7), W('V', V3D_INTENA, 0x7));
	CHECK(!hw.running[0], "the binner stopped");
	CHECK(vc4_reset(&v3d_io, NULL, VC4_RESET_THREAD, &job, &m) == -EINVAL, "no running stage: -EINVAL");
	CHECK(vc4_reset(&v3d_io, NULL, VC4_RESET_POWER_CYCLE, &job, &m) == -EINVAL, "no switch: -EINVAL");

	/* a binner that keeps moving is never declared hung */
	frame_setup(&m, &job);
	hw.bin_hangs = true;
	vc4_job_submit(&v3d_io, &job);
	bool hung = false;
	for (int i = 0; i < 5; i++) {
		hw.regs[V3D_CTNCA(0) / 4] += 0x20;
		hung |= vc4_hangcheck(&v3d_io, &job) == VC4_HANG_HUNG;
	}
	CHECK(!hung, "a moving CT0CA is progress");

	/* a power switch that fails stops the reset there */
	frame_setup(&m, &job);
	hw.render_hangs = true;
	vc4_job_submit(&v3d_io, &job);
	run(&job, &m, 1, NULL);
	fw_reset();
	fw.error = true;
	from = nlog;
	CHECK(vc4_reset(&v3d_io, &fw_power, VC4_RESET_POWER_CYCLE, &job, &m) == -EIO, "a firmware error: -EIO");
	EXPECT("failed power cycle", from,
	       W('V', V3D_INTDIS, 0x7), W('V', V3D_INTCTL, 0x7), F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0));
	CHECK(job.state == VC4_JOB_RENDERING, "the job is left for the caller");
	fw.error = false;
}

static void case_power_fw(void)
{
	hw_reset();
	fw_reset();
	first_msg_taken = false;
	fw.domain_on = false;
	fw.clock_on = false;
	hw.powered = false;
	fwp.rate_hz = VC4_FW_RATE_MAX;
	unsigned from = nlog;
	CHECK(vc4_power_fw_on(&fwp) == 0, "power on through the firmware");
	EXPECT("firmware power on, maximum rate", from,
	       F(RPI_TAG_SET_DOMAIN_STATE, VC4_FW_DOMAIN_V3D, 1), F(RPI_TAG_GET_MAX_CLOCK_RATE, VC4_FW_CLK_V3D, 0),
	       F(RPI_TAG_SET_CLOCK_RATE, VC4_FW_CLK_V3D, FW_MAX_HZ), F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1));
	clock_tag_check("power on");
	CHECK(fw.domain_on && fw.clock_on && fw.rate == FW_MAX_HZ && fwp.set_hz == FW_MAX_HZ,
	      "domain on, clock on at the maximum");
	CHECK(mail1_word == 0xc0100008u, "MAIL1 gets the buffer's bus address with channel 8 (0x%08x)", mail1_word);
	/* the first request as the firmware reads it: size, request, tag,
	 * value bytes, tag request, {11, 1}, end */
	const uint32_t want_msg[8] = { 32, 0, RPI_TAG_SET_DOMAIN_STATE, 8, 0, 11, 1, 0 };
	CHECK(memcmp(first_msg, want_msg, sizeof(want_msg)) == 0,
	      "the SET_DOMAIN_STATE buffer: %u %u 0x%x %u %u %u %u %u", first_msg[0], first_msg[1], first_msg[2],
	      first_msg[3], first_msg[4], first_msg[5], first_msg[6], first_msg[7]);

	bool on = false;
	CHECK(vc4_power_fw_state(&fwp, &on) == 0 && on, "GET_DOMAIN_STATE reads on");
	from = nlog;
	CHECK(vc4_power_fw_off(&fwp) == 0, "power off");
	EXPECT("firmware power off", from,
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0), F(RPI_TAG_SET_DOMAIN_STATE, VC4_FW_DOMAIN_V3D, 0));
	CHECK(!fw.domain_on && !fw.clock_on, "domain and clock off");
	CHECK(vc4_power_fw_state(&fwp, &on) == 0 && !on, "GET_DOMAIN_STATE reads off");

	fwp.rate_hz = 250000000u;
	from = nlog;
	CHECK(vc4_power_fw_on(&fwp) == 0 && fw.rate == 250000000u, "a fixed rate");
	EXPECT("firmware power on, fixed rate", from,
	       F(RPI_TAG_SET_DOMAIN_STATE, VC4_FW_DOMAIN_V3D, 1), F(RPI_TAG_SET_CLOCK_RATE, VC4_FW_CLK_V3D, 250000000u),
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1));
	fwp.rate_hz = 0;
	vc4_power_fw_off(&fwp);
	from = nlog;
	CHECK(vc4_power_fw_on(&fwp) == 0 && fwp.set_hz == 0, "the firmware's rate left alone");
	EXPECT("firmware power on, rate left", from,
	       F(RPI_TAG_SET_DOMAIN_STATE, VC4_FW_DOMAIN_V3D, 1), F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1));

	/* with the domain on, the identity reads: the day-one order */
	struct vc4_ident id;
	CHECK(vc4_identify(&v3d_io, &id) == 0, "the block answers once powered");

	fw.old_interface = true;
	CHECK(vc4_power_fw_state(&fwp, &on) == -ENOTSUP, "a firmware without GET_DOMAIN_STATE: -ENOTSUP");
	fw.old_interface = false;

	fw.error = true;
	from = nlog;
	CHECK(vc4_power_fw_on(&fwp) == -EIO, "an error answer: -EIO");
	EXPECT("firmware error", from, F(RPI_TAG_SET_DOMAIN_STATE, VC4_FW_DOMAIN_V3D, 1));
	fw.error = false;
	clock_tag_check("all firmware cases");
}

static int clk_count_on, clk_count_off;
static int fw_clock_logged(void *ctx, bool on)
{
	on ? clk_count_on++ : clk_count_off++;
	return vc4_fw_clock(ctx, on);
}

static void case_power_pm(void)
{
	struct vc4_power_pm p = { &pm_io, &asb_io, fw_clock_logged, &fwp };
	const uint32_t K = PM_PASSWORD;
	const uint32_t I = PM_INRUSH_SHIFT;

	/* island off: the full power-on */
	pm_reset();
	fw_reset();
	unsigned from = nlog;
	CHECK(vc4_power_pm_on(&p) == 0, "the PM/ASB power-on");
	EXPECT("PM/ASB power on, island off", from,
	       W('P', PM_GRAFX, K | (0u << I) | PM_POWUP),
	       W('P', PM_GRAFX, K | PM_POWOK | (1u << I) | PM_POWUP),
	       W('P', PM_GRAFX, K | PM_POWOK | (2u << I) | PM_POWUP),
	       W('P', PM_GRAFX, K | PM_POWOK | (3u << I) | PM_POWUP),
	       W('P', PM_GRAFX, K | (3u << I) | PM_ISPOW | PM_POWOK | PM_POWUP),
	       W('P', PM_GRAFX, K | (3u << I) | PM_MEMREP | PM_ISPOW | PM_POWOK | PM_POWUP),
	       W('P', PM_GRAFX, K | (3u << I) | PM_ISFUNC | PM_MRDONE | PM_MEMREP | PM_ISPOW | PM_POWOK | PM_POWUP),
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1), DLY, F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0),
	       W('P', PM_GRAFX,
		 K | (3u << I) | PM_V3DRSTN | PM_ISFUNC | PM_MRDONE | PM_MEMREP | PM_ISPOW | PM_POWOK | PM_POWUP),
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1),
	       W('A', ASB_V3D_M_CTRL, K | ASB_ACK), W('A', ASB_V3D_S_CTRL, K | ASB_ACK));
	CHECK(pmm.no_password == 0, "every PM and ASB write carries the password");
	CHECK(!(pmm.asb[ASB_V3D_M_CTRL / 4] & ASB_ACK) && !(pmm.asb[ASB_V3D_S_CTRL / 4] & ASB_ACK),
	      "both bridge ports run");
	CHECK(fw.clock_on, "the clock is on");
	clock_tag_check("PM power on");

	from = nlog;
	CHECK(vc4_power_pm_off(&p) == 0, "the PM/ASB power-off");
	EXPECT("PM/ASB power off", from,
	       W('A', ASB_V3D_S_CTRL, K | ASB_REQ_STOP), W('A', ASB_V3D_M_CTRL, K | ASB_REQ_STOP),
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0),
	       W('P', PM_GRAFX, K | (3u << I) | PM_ISFUNC | PM_MRDONE | PM_MEMREP | PM_ISPOW | PM_POWOK | PM_POWUP),
	       W('P', PM_GRAFX, K | (3u << I) | PM_MRDONE | PM_MEMREP | PM_ISPOW | PM_POWOK | PM_POWUP),
	       W('P', PM_GRAFX, K | (3u << I) | PM_MRDONE | PM_MEMREP | PM_POWOK | PM_POWUP),
	       W('P', PM_GRAFX, K | (3u << I) | PM_MRDONE | PM_MEMREP | PM_POWOK));
	CHECK(!(pmm.pm[PM_GRAFX / 4] & (PM_POWUP | PM_V3DRSTN)), "island off, V3D in reset");

	/* island already on (the firmware's): only the V3D's own steps */
	pm_reset();
	pmm.pm[PM_GRAFX / 4] = PM_POWUP | PM_POWOK | PM_ISPOW | PM_ISFUNC;
	from = nlog;
	CHECK(vc4_power_pm_on(&p) == 0, "power-on with the island on");
	EXPECT("PM/ASB power on, island on", from,
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1), DLY, F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0),
	       W('P', PM_GRAFX, K | PM_V3DRSTN | PM_ISFUNC | PM_ISPOW | PM_POWOK | PM_POWUP),
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1),
	       W('A', ASB_V3D_M_CTRL, K | ASB_ACK), W('A', ASB_V3D_S_CTRL, K | ASB_ACK));

	/* a wrong bridge: nothing done */
	pm_reset();
	pmm.asb[ASB_AXI_BRDG_ID / 4] = 0;
	from = nlog;
	CHECK(vc4_power_pm_on(&p) == -ENODEV, "a wrong bridge ID: -ENODEV");
	CHECK(nlog == from, "nothing written without the bridge");

	/* no POWOK at any inrush step: the switches opened again */
	pm_reset();
	pmm.powok_never = true;
	from = nlog;
	CHECK(vc4_power_pm_on(&p) == -ETIMEDOUT, "no POWOK: -ETIMEDOUT");
	CHECK(nlog - from == 4 + 4 * 3 + 1, "four steps, each waited out, then the undo (%u)", nlog - from);
	CHECK(elog[nlog - 1].blk == 'P' && elog[nlog - 1].val == K, "the last write opens the switches (0x%08x)",
	      elog[nlog - 1].val);

	/* the bridge master never acknowledges: Linux's unwinding */
	pm_reset();
	pmm.pm[PM_GRAFX / 4] = PM_POWUP | PM_POWOK | PM_ISPOW | PM_ISFUNC;
	pmm.asb_m_ack_sticks = true;
	from = nlog;
	CHECK(vc4_power_pm_on(&p) == -ETIMEDOUT, "a bridge that never acknowledges: -ETIMEDOUT");
	EXPECT("PM/ASB bridge timeout", from,
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1), DLY, F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0),
	       W('P', PM_GRAFX, K | PM_V3DRSTN | PM_ISFUNC | PM_ISPOW | PM_POWOK | PM_POWUP),
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 1),
	       W('A', ASB_V3D_M_CTRL, K | ASB_ACK), DLY,
	       F(RPI_TAG_SET_CLOCK_STATE, VC4_FW_CLK_V3D, 0),
	       W('P', PM_GRAFX, K | PM_ISFUNC | PM_ISPOW | PM_POWOK | PM_POWUP));
	CHECK(!fw.clock_on && !(pmm.pm[PM_GRAFX / 4] & PM_V3DRSTN), "clock off and the V3D back in reset");

	/* the PM path as the reset's power switch */
	struct vc4_bin_mem m;
	struct vc4_job job;
	frame_setup(&m, &job);
	pm_reset();
	pmm.pm[PM_GRAFX / 4] = PM_POWUP | PM_POWOK | PM_ISPOW | PM_ISFUNC | PM_V3DRSTN;
	pmm.asb[ASB_V3D_M_CTRL / 4] = 0;
	pmm.asb[ASB_V3D_S_CTRL / 4] = 0;
	hw.render_hangs = true;
	vc4_job_submit(&v3d_io, &job);
	run(&job, &m, 1, NULL);
	const struct vc4_power pm_power = { vc4_power_pm_on, vc4_power_pm_off, &p };
	from = nlog;
	CHECK(vc4_reset(&v3d_io, &pm_power, vc4_reset_choose(&pm_power, false), &job, &m) == 0,
	      "the PM path as the reset's switch");
	CHECK(elog[from].blk == 'V' && elog[from].off == V3D_INTDIS && elog[nlog - 1].blk == 'V' &&
		      elog[nlog - 1].off == V3D_INTENA && elog[nlog - 1].val == 0x7,
	      "the V3D writes frame the power steps");
	CHECK(pmm.pm[PM_GRAFX / 4] & PM_V3DRSTN, "the V3D is out of reset again");
	CHECK(pmm.no_password == 0, "every PM and ASB write carries the password");
	CHECK(clk_count_on > 0 && clk_count_off > 0, "the clock went through the firmware");
}

int main(void)
{
	/* rpi_mbox.c as the board's glue would start it */
	CHECK(rpi_mbox_init() == 0, "the mailbox maps");
	CHECK(mbox_map_phys == MBOX_PHYS_EXPECTED, "the BCM2837's mailbox, ARM 0x3f00b880 (got 0x%llx)",
	      mbox_map_phys);
	fw_reset();
	case_ident_wrong();
	case_init();
	case_bin_range();
	case_frame();
	case_oom();
	case_hang();
	case_power_fw();
	case_power_pm();
	printf("vc4 hw_mock_test: %d checks run (planned %d), %d failed\n", checks, PLANNED, failed);
	return failed == 0 && checks == PLANNED ? 0 : 1;
}
