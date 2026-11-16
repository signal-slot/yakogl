/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hdmi_ctrl_mock: driver/zephyr/lib/board/rpi5_scanout/hdmi_ctrl.h against
 * expected write lists. The expected values below are computed by hand from
 * the CEA timings and the register layouts (not by calling the header's
 * helpers), so a wrong shift or a wrong order in the header fails here.
 *
 * Checked: 640x480@60 (25.175 MHz), 1280x720@60 (74.25 MHz) and
 * 1920x1080@60 (148.5 MHz): the timing registers, the polarity bits, the
 * order of the whole sequence (scheduler, timings, CSC, FIFO, VID_CTL, DVI
 * mode, recenter), the clock refused above 600 MHz (log untouched), the field
 * range refusals, a full log, and the disable sequence.
 *
 * Prints "hdmi_ctrl_mock: N of N checks, 0 failed"; exit 0 when none failed.
 */
#include <stdio.h>
#include <string.h>

#include "../rpi5_scanout/hdmi_ctrl.h"

static unsigned int g_checks, g_failed;

#define CHECK(cond, ...)                                                                           \
	do {                                                                                       \
		g_checks++;                                                                        \
		if (!(cond)) {                                                                     \
			g_failed++;                                                                \
			printf("FAIL %s:%d: ", __FILE__, __LINE__);                                \
			printf(__VA_ARGS__);                                                       \
			printf("\n");                                                              \
		}                                                                                  \
	} while (0)

#define F 0xffffffffu

/* expected whole sequence, by position */
struct exp {
	uint8_t region;
	uint16_t offset;
	uint32_t mask;
};

static const struct exp seq_head[] = {
	{YD5_HDMI_R_HDMI, 0x0e8, 0x8020}, /* SCHEDULER_CONTROL |= MANUAL_FORMAT | IGNORE_VSYNC_PREDICTS */
	{YD5_HDMI_R_HDMI, 0x0ec, F},      /* HORZA */
	{YD5_HDMI_R_HDMI, 0x0f0, F},      /* HORZB */
	{YD5_HDMI_R_HDMI, 0x0f4, F},      /* VERTA0 */
	{YD5_HDMI_R_HDMI, 0x100, F},      /* VERTA1 */
	{YD5_HDMI_R_HDMI, 0x0f8, F},      /* VERTB0 */
	{YD5_HDMI_R_HDMI, 0x104, F},      /* VERTB1 */
	{YD5_HDMI_R_HDMI, 0x18c, 0x70f},  /* DEEP_COLOR_CONFIG_1 */
	{YD5_HDMI_R_HDMI, 0x198, 0xffff}, /* GCP_WORD_1 */
	{YD5_HDMI_R_HDMI, 0x194, 0x80000000u}, /* GCP_CONFIG */
	{YD5_HDMI_R_HDMI, 0x114, 0xf},    /* MISC_CONTROL */
	{YD5_HDMI_R_DVP, 0x0bc, F},       /* CLOCK_STOP */
	{YD5_HDMI_R_CSC, 0x004, F},       /* CSC_12_11 */
	{YD5_HDMI_R_CSC, 0x008, F},       /* CSC_14_13 */
	{YD5_HDMI_R_CSC, 0x00c, F},       /* CSC_22_21 */
	{YD5_HDMI_R_CSC, 0x010, F},       /* CSC_24_23 */
	{YD5_HDMI_R_CSC, 0x014, F},       /* CSC_32_31 */
	{YD5_HDMI_R_CSC, 0x018, F},       /* CSC_34_33 */
	{YD5_HDMI_R_DVP, 0x0f0, F},       /* VEC_INTERFACE_CFG */
	{YD5_HDMI_R_DVP, 0x0f4, F},       /* VEC_INTERFACE_XBAR */
	{YD5_HDMI_R_CSC, 0x02c, F},       /* CSC_CHANNEL_CTL */
	{YD5_HDMI_R_CSC, 0x000, F},       /* CSC_CTL */
	{YD5_HDMI_R_HDMI, 0x07c, F},      /* FIFO_CTL = MASTER_SLAVE_N */
	{YD5_HDMI_R_HD, 0x044, 0xf8810000u}, /* VID_CTL: enable and polarity */
	{YD5_HDMI_R_HD, 0x044, 0x00040000},  /* VID_CTL: BLANKPIX released */
	{YD5_HDMI_R_HDMI, 0x0e8, 0x1},       /* SCHEDULER_CONTROL: DVI mode */
	{YD5_HDMI_R_HDMI, 0x07c, 0x40},      /* FIFO_CTL recenter low */
	{YD5_HDMI_R_HDMI, 0x07c, 0x40},      /* high */
	{YD5_HDMI_R_HDMI, 0x07c, 0x40},      /* low */
	{YD5_HDMI_R_HDMI, 0x07c, 0x40},      /* high */
};
#define SEQ_N (sizeof(seq_head) / sizeof(seq_head[0]))

struct mode {
	const char *name;
	struct yd5_hdmi_timing t;
	uint32_t horza, horzb, verta, vertb0, vertb1, vid_ctl;
};

static const struct mode modes[] = {
	{"640x480@60",
	 {25175000u, 640, 16, 96, 48, 480, 10, 2, 33, 0, 0},
	 /* HFP 16<<16 | HAP 640; HBP 48<<16 | HSP 96; VSP 2<<24 | VFP 10<<16 | 480;
	  * VBP 33; (800/2=400)<<16 | 33; both syncs negative: both LOW bits */
	 0x00100280u, 0x00300060u, 0x020a01e0u, 0x00000021u, 0x01900021u, 0xf8810000u},
	{"1280x720@60",
	 {74250000u, 1280, 110, 40, 220, 720, 5, 5, 20, 1, 1},
	 /* VPOS|HPOS (0xc000) | 110<<16 | 1280; 220<<16 | 40; 5<<24 | 5<<16 | 720;
	  * 20; (1650/2=825)<<16 | 20; both positive: no LOW bit */
	 0x006ec500u, 0x00dc0028u, 0x050502d0u, 0x00000014u, 0x03390014u, 0xe0810000u},
	{"1920x1080@60",
	 {148500000u, 1920, 88, 44, 148, 1080, 4, 5, 36, 1, 1},
	 0x0058c780u, 0x0094002cu, 0x05040438u, 0x00000024u, 0x044c0024u, 0xe0810000u},
};

/* a mixed-polarity mode: 720p timings, hsync positive, vsync negative */
static const struct yd5_hdmi_timing mixed = {74250000u, 1280, 110, 40, 220, 720, 5, 5, 20, 1, 0};

static const struct yd5_hdmi_write *find(const struct yd5_hdmi_log *l, struct yd5_hdmi_reg r)
{
	unsigned int i;
	for (i = 0; i < l->n; i++) {
		if (l->w[i].region == r.region && l->w[i].offset == r.offset) {
			return &l->w[i];
		}
	}
	return 0;
}

static void check_mode(const struct mode *m)
{
	struct yd5_hdmi_log log;
	const struct yd5_hdmi_write *w;
	unsigned int i;
	int rc;

	yd5_hdmi_log_init(&log);
	rc = yd5_hdmi_dvi_enable(&log, &m->t);
	CHECK(rc == YD5_HDMI_OK, "%s: enable returned %d", m->name, rc);
	CHECK(log.n == SEQ_N, "%s: %u writes, expected %u", m->name, log.n, (unsigned)SEQ_N);

	for (i = 0; i < log.n && i < SEQ_N; i++) {
		CHECK(log.w[i].region == seq_head[i].region && log.w[i].offset == seq_head[i].offset &&
			      log.w[i].mask == seq_head[i].mask,
		      "%s: write %u is {%u,0x%03x,mask 0x%x}, expected {%u,0x%03x,mask 0x%x}",
		      m->name, i, log.w[i].region, log.w[i].offset, log.w[i].mask,
		      seq_head[i].region, seq_head[i].offset, seq_head[i].mask);
	}

	w = find(&log, YD5_HDMI_HORZA);
	CHECK(w && w->value == m->horza, "%s: HORZA 0x%08x, expected 0x%08x", m->name,
	      w ? w->value : 0, m->horza);
	w = find(&log, YD5_HDMI_HORZB);
	CHECK(w && w->value == m->horzb, "%s: HORZB 0x%08x, expected 0x%08x", m->name,
	      w ? w->value : 0, m->horzb);
	w = find(&log, YD5_HDMI_VERTA0);
	CHECK(w && w->value == m->verta, "%s: VERTA0 0x%08x, expected 0x%08x", m->name,
	      w ? w->value : 0, m->verta);
	w = find(&log, YD5_HDMI_VERTA1);
	CHECK(w && w->value == m->verta, "%s: VERTA1 0x%08x, expected 0x%08x", m->name,
	      w ? w->value : 0, m->verta);
	w = find(&log, YD5_HDMI_VERTB0);
	CHECK(w && w->value == m->vertb0, "%s: VERTB0 0x%08x, expected 0x%08x", m->name,
	      w ? w->value : 0, m->vertb0);
	w = find(&log, YD5_HDMI_VERTB1);
	CHECK(w && w->value == m->vertb1, "%s: VERTB1 0x%08x, expected 0x%08x", m->name,
	      w ? w->value : 0, m->vertb1);
	w = find(&log, YD5_HDMI_VID_CTL);
	CHECK(w && w->value == m->vid_ctl, "%s: VID_CTL 0x%08x, expected 0x%08x", m->name,
	      w ? w->value : 0, m->vid_ctl);
	/* VID_CTL bit 31 is the enable (rpi:vc4_regs.h:1007) */
	CHECK(w && (w->value & 0x80000000u) && (w->mask & 0x80000000u), "%s: VID_CTL enable bit",
	      m->name);
	/* the enable is the last thing that starts the link: after the timings, the CSC and
	 * the FIFO master write */
	CHECK(log.n > 23 && log.w[23].offset == 0x044 && log.w[23].region == YD5_HDMI_R_HD,
	      "%s: VID_CTL enable is write 23", m->name);
}

static void check_values_fixed(void)
{
	struct yd5_hdmi_log log;
	const struct yd5_hdmi_write *w;

	yd5_hdmi_log_init(&log);
	(void)yd5_hdmi_dvi_enable(&log, &modes[1].t);

	/* the fixed values of DVI RGB 8 bpc full range, by hand from the field layouts */
	w = find(&log, YD5_HDMI_SCHEDULER_CONTROL);
	CHECK(w && w->value == 0x8020u, "scheduler value 0x%x", w ? w->value : 0);
	w = find(&log, YD5_HDMI_DEEP_COLOR_CONFIG_1);
	CHECK(w && w->value == 0x200u && w->mask == 0x70fu, "deep colour 0x%x/0x%x (phase 2, depth 0)",
	      w ? w->value : 0, w ? w->mask : 0);
	w = find(&log, YD5_HDMI_GCP_WORD_1);
	CHECK(w && w->value == 0x10u && w->mask == 0xffffu, "GCP word 0x%x (byte0 = clear AVMUTE)",
	      w ? w->value : 0);
	w = find(&log, YD5_HDMI_GCP_CONFIG);
	CHECK(w && w->value == 0x80000000u, "GCP config enable");
	w = find(&log, YD5_HDMI_MISC_CONTROL);
	CHECK(w && w->value == 0 && w->mask == 0xfu, "pixel repetition 1 -> field 0");
	w = find(&log, YD5_HDMI_CLOCK_STOP);
	CHECK(w && w->value == 0 && w->mask == F, "clock stop 0");
	/* identity CSC: 1.0 is 0x2000 in 2p13; words are (second << 16) | first */
	w = find(&log, YD5_HDMI_CSC_12_11);
	CHECK(w && w->value == 0x00002000u, "CSC_12_11");
	w = find(&log, YD5_HDMI_CSC_14_13);
	CHECK(w && w->value == 0u, "CSC_14_13");
	w = find(&log, YD5_HDMI_CSC_22_21);
	CHECK(w && w->value == 0x20000000u, "CSC_22_21");
	w = find(&log, YD5_HDMI_CSC_24_23);
	CHECK(w && w->value == 0u, "CSC_24_23");
	w = find(&log, YD5_HDMI_CSC_32_31);
	CHECK(w && w->value == 0u, "CSC_32_31");
	w = find(&log, YD5_HDMI_CSC_34_33);
	CHECK(w && w->value == 0x00002000u, "CSC_34_33");
	w = find(&log, YD5_HDMI_VEC_INTERFACE_CFG);
	CHECK(w && w->value == 0u, "VEC_INTERFACE_CFG");
	w = find(&log, YD5_HDMI_VEC_INTERFACE_XBAR);
	CHECK(w && w->value == 0x354021u, "VEC_INTERFACE_XBAR 0x%x", w ? w->value : 0);
	w = find(&log, YD5_HDMI_CSC_CHANNEL_CTL);
	CHECK(w && w->value == 0u, "CSC_CHANNEL_CTL");
	w = find(&log, YD5_HDMI_CSC_CTL);
	CHECK(w && w->value == 0x7u, "CSC_CTL 0x%x (enable | custom)", w ? w->value : 0);
	w = find(&log, YD5_HDMI_FIFO_CTL);
	CHECK(w && w->value == 1u && w->mask == F, "FIFO_CTL master");
	/* the two recenter pairs: low, high, low, high, on the RECENTER bit only */
	CHECK(log.w[26].value == 0 && log.w[27].value == 0x40 && log.w[28].value == 0 &&
		      log.w[29].value == 0x40,
	      "recenter low/high/low/high");
	/* DVI: MODE_HDMI cleared, no packet or infoframe registers touched */
	w = &log.w[25];
	CHECK(w->offset == 0x0e8 && w->value == 0 && w->mask == 1u, "DVI mode: MODE_HDMI cleared");
	CHECK(find(&log, YD5_HDMI_RAM_PACKET_CONFIG) == 0, "no packet RAM write in DVI enable");
}

static void check_polarity(void)
{
	struct yd5_hdmi_log log;
	const struct yd5_hdmi_write *w;
	struct yd5_hdmi_timing t = mixed;

	yd5_hdmi_log_init(&log);
	CHECK(yd5_hdmi_dvi_enable(&log, &t) == YD5_HDMI_OK, "mixed polarity enable");
	w = find(&log, YD5_HDMI_HORZA);
	/* hsync positive, vsync negative: HPOS set, VPOS clear */
	CHECK(w && (w->value & 0x4000u) && !(w->value & 0x8000u), "HORZA: HPOS only (0x%x)",
	      w ? w->value : 0);
	w = find(&log, YD5_HDMI_VID_CTL);
	/* a LOW bit for the negative sync only: vsync */
	CHECK(w && (w->value & YD5_VID_CTL_VSYNC_LOW) && !(w->value & YD5_VID_CTL_HSYNC_LOW),
	      "VID_CTL: VSYNC_LOW only (0x%x)", w ? w->value : 0);
	CHECK(w && (w->mask & YD5_VID_CTL_VSYNC_LOW) && (w->mask & YD5_VID_CTL_HSYNC_LOW),
	      "VID_CTL: both LOW bits are rewritten");

	/* the other way round */
	t.hsync_pos = 0;
	t.vsync_pos = 1;
	yd5_hdmi_log_init(&log);
	CHECK(yd5_hdmi_dvi_enable(&log, &t) == YD5_HDMI_OK, "mixed polarity 2 enable");
	w = find(&log, YD5_HDMI_HORZA);
	CHECK(w && (w->value & 0x8000u) && !(w->value & 0x4000u), "HORZA: VPOS only (0x%x)",
	      w ? w->value : 0);
	w = find(&log, YD5_HDMI_VID_CTL);
	CHECK(w && (w->value & YD5_VID_CTL_HSYNC_LOW) && !(w->value & YD5_VID_CTL_VSYNC_LOW),
	      "VID_CTL: HSYNC_LOW only (0x%x)", w ? w->value : 0);
}

static void check_refusals(void)
{
	struct yd5_hdmi_log log;
	struct yd5_hdmi_timing t = modes[2].t;
	int rc;

	/* exactly 600 MHz is allowed, one hertz more is not */
	t.pixel_clock_hz = 600000000u;
	yd5_hdmi_log_init(&log);
	rc = yd5_hdmi_dvi_enable(&log, &t);
	CHECK(rc == YD5_HDMI_OK && log.n == SEQ_N, "600 MHz accepted (rc %d)", rc);

	t.pixel_clock_hz = 600000001u;
	yd5_hdmi_log_init(&log);
	rc = yd5_hdmi_dvi_enable(&log, &t);
	CHECK(rc == YD5_HDMI_E_CLOCK, "600.000001 MHz refused with E_CLOCK (rc %d)", rc);
	CHECK(log.n == 0, "refused clock leaves the log empty (%u)", log.n);

	/* the refusal leaves earlier entries of the log alone */
	yd5_hdmi_log_init(&log);
	(void)yd5_hdmi_disable(&log);
	t.pixel_clock_hz = 4000000000u;
	rc = yd5_hdmi_dvi_enable(&log, &t);
	CHECK(rc == YD5_HDMI_E_CLOCK && log.n == 4, "4 GHz refused after the disable (n %u)", log.n);

	/* argument and range refusals */
	t = modes[0].t;
	CHECK(yd5_hdmi_dvi_enable(0, &t) == YD5_HDMI_E_ARG, "null log");
	yd5_hdmi_log_init(&log);
	CHECK(yd5_hdmi_dvi_enable(&log, 0) == YD5_HDMI_E_ARG, "null timing");
	t.pixel_clock_hz = 0;
	CHECK(yd5_hdmi_dvi_enable(&log, &t) == YD5_HDMI_E_ARG, "zero clock");
	t = modes[0].t;
	t.v_sync = 32; /* VSP is 5 bits */
	CHECK(yd5_hdmi_dvi_enable(&log, &t) == YD5_HDMI_E_RANGE, "vsync too long for 5 bits");
	t = modes[0].t;
	t.v_fp = 128; /* VFP is 7 bits */
	CHECK(yd5_hdmi_dvi_enable(&log, &t) == YD5_HDMI_E_RANGE, "vfp too long for 7 bits");
	t = modes[0].t;
	t.h_bp = 2048; /* HBP is 11 bits */
	CHECK(yd5_hdmi_dvi_enable(&log, &t) == YD5_HDMI_E_RANGE, "hbp too long for 11 bits");
	CHECK(log.n == 0, "range refusals append nothing (%u)", log.n);

	/* a full log: all or nothing */
	yd5_hdmi_log_init(&log);
	log.n = YD5_HDMI_LOG_MAX - 10;
	CHECK(yd5_hdmi_dvi_enable(&log, &modes[0].t) == YD5_HDMI_E_FULL, "full log refused");
	CHECK(log.n == YD5_HDMI_LOG_MAX - 10, "full log: the log is as it was (%u)", log.n);
}

static void check_disable(void)
{
	struct yd5_hdmi_log log;

	yd5_hdmi_log_init(&log);
	CHECK(yd5_hdmi_disable(&log) == YD5_HDMI_OK, "disable");
	CHECK(log.n == 4, "disable: %u writes, expected 4", log.n);
	/* RAM_PACKET_CONFIG = RAM_PACKET_ENABLE, whole register */
	CHECK(log.w[0].region == YD5_HDMI_R_HDMI && log.w[0].offset == 0x0c4 &&
		      log.w[0].value == 0x10000u && log.w[0].mask == F,
	      "disable 0: RAM_PACKET_CONFIG");
	/* VID_CTL |= CLRRGB, then |= BLANKPIX, then BLANKPIX again (powerdown) */
	CHECK(log.w[1].region == YD5_HDMI_R_HD && log.w[1].offset == 0x044 &&
		      log.w[1].value == 0x00800000u && log.w[1].mask == 0x00800000u,
	      "disable 1: VID_CTL CLRRGB");
	CHECK(log.w[2].region == YD5_HDMI_R_HD && log.w[2].offset == 0x044 &&
		      log.w[2].value == 0x00040000u && log.w[2].mask == 0x00040000u,
	      "disable 2: VID_CTL BLANKPIX");
	CHECK(log.w[3].region == YD5_HDMI_R_HD && log.w[3].offset == 0x044 &&
		      log.w[3].value == 0x00040000u && log.w[3].mask == 0x00040000u,
	      "disable 3: VID_CTL BLANKPIX (powerdown)");
	/* the enable bit is never cleared on the BCM2712 (Linux: it locks the system up) */
	{
		unsigned int i, clears = 0;
		for (i = 0; i < log.n; i++) {
			if (log.w[i].region == YD5_HDMI_R_HD && log.w[i].offset == 0x044 &&
			    (log.w[i].mask & 0x80000000u)) {
				clears++;
			}
		}
		CHECK(clears == 0, "disable does not touch VID_CTL enable (%u)", clears);
	}
	/* a full log refuses without a partial write beyond the room: it stops at the limit */
	log.n = YD5_HDMI_LOG_MAX - 1;
	CHECK(yd5_hdmi_disable(&log) == YD5_HDMI_E_FULL, "disable on a nearly full log: E_FULL");
	CHECK(yd5_hdmi_disable(0) == YD5_HDMI_E_ARG, "disable(null)");
}

static void check_phases(void)
{
	/* the three phases appended one after the other equal the one-call enable, so the
	 * pixel valve can start between phases 4 and 6 */
	struct yd5_hdmi_log a, b;
	const struct yd5_hdmi_timing *t = &modes[1].t;

	yd5_hdmi_log_init(&a);
	yd5_hdmi_log_init(&b);
	CHECK(yd5_hdmi_dvi_enable(&a, t) == YD5_HDMI_OK, "one-call enable");
	CHECK(yd5_hdmi_dvi_configure(&b, t) == YD5_HDMI_OK, "phase 3");
	CHECK(b.n == 12, "phase 3 writes %u, expected 12", b.n);
	CHECK(yd5_hdmi_dvi_pre_enable(&b) == YD5_HDMI_OK, "phase 4");
	CHECK(b.n == 23, "phases 3+4 write %u, expected 23", b.n);
	CHECK(yd5_hdmi_dvi_post_enable(&b, t) == YD5_HDMI_OK, "phase 6");
	{
		unsigned int i, same = (a.n == b.n);
		for (i = 0; same && i < a.n; i++) {
			same = a.w[i].region == b.w[i].region && a.w[i].offset == b.w[i].offset &&
			       a.w[i].value == b.w[i].value && a.w[i].mask == b.w[i].mask;
		}
		CHECK(same, "phases equal enable");
	}
}

int main(void)
{
	unsigned int i;

	for (i = 0; i < sizeof(modes) / sizeof(modes[0]); i++) {
		check_mode(&modes[i]);
	}
	check_values_fixed();
	check_polarity();
	check_refusals();
	check_disable();
	check_phases();

	printf("hdmi_ctrl_mock: %u of %u checks, %u failed\n", g_checks - g_failed, g_checks,
	       g_failed);
	return g_failed ? 1 : 0;
}
