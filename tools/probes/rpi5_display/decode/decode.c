/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * decode: reads a text dump of Pi 5 scanout register words on stdin and prints
 * the HVS6 channels and display-list planes, the pixel valve and the HDMI0
 * status as readable lines. The decoding is the reviewed header-only code,
 * included unchanged: hvs6_dlist.h and pv_hdmi.h.
 *
 * Accepted line formats (anything else is ignored; the marker is searched for,
 * so a line may carry a prefix such as "rpi5_display: " or a log stamp):
 *   1. linux_ref.sh (tools/probes/rpi5_display/linux_ref.md), at line start:
 *        mem.hvs[0xOFF]: 0xVAL          HVS block, byte offset OFF
 *        mem.pixelvalve0[0xOFF]: 0xVAL  (also pixelvalve1)
 *        mem.hdmi0_hd[0xOFF]: 0xVAL     HDMI0 "hd" block
 *      One set, "linux_ref". The script reads 64 HVS words, 16 pixel valve
 *      words and 16 hd words, so the HVS display list (from 0x4000), the
 *      generation-D channels (from 0x100) and the two VID_CTL words (0x44 and
 *      0x48, both past the 16th word) are normally not in it and are reported
 *      as such. If a dump carries the list anyway (mem.hvs[0x4000+...]) it is
 *      decoded. A dump that names pixelvalve1 (a deeper read than
 *      linux_ref.sh's) is decoded as its own pixel valve.
 *   2. the probe's snapshot lines (rpi5_scanout/snapshot.h), set "u3":
 *        u3 reg <hvs|pv|hdmi0> <name> +0xOFF=0xVAL
 *      One line per table entry: both channel groups are listed, and the
 *      version word decides which one is decoded. "pv" is pixel valve 0. The
 *      hdmi0 lines carry both connectors' status words, hdmi_vid_ctl at 0x044
 *      and hdmi1_vid_ctl at 0x048, at their table offsets (the same table
 *      offset as pv_hdmi.h's HD_VID_CTL).
 *   3.   u3 dlist <ch> [<i>]=0xWORD
 *      A channel's list as snapshot_dlist read it: its index 0 is the start
 *      word the probe was given (the channel's DL.LACT, or LPTRS.HEADE when
 *      LACT is 0), so the list is decoded from index 0. The probe reads a
 *      channel's list twice (Z1, and again after its mailbox allocation),
 *      printing each read with indexes restarting at [0]; the capture resets
 *      at every [0] line, so what is decoded is the last read - the state
 *      after the allocation.
 *
 * A block the probe could not read reads as all-ones words (main.c's rule:
 * unmapped or clock-gated blocks are reported 0xffffffff, with the record's
 * own "not read" lines naming them). Such a block is reported as reading all
 * ones instead of being decoded: a read of ones is not a value. The rule is
 * per block and uniform over it, so one all-ones version word means the whole
 * HVS was not read and no channel is decoded.
 *
 * Words whose fields the reviewed headers do not name (the HVS CONTROL word at
 * 0x20, ctrl1, bgnd, cob, hdmi_dvp_ctl, the frame counters, pv_inten,
 * pv_intstat, pv_stat, pv_hact_act) are counted in the set and not printed:
 * their fields are judged on the board side, by scanout_alive.h and the
 * record's own lines.
 *
 * Output never carries an absolute bus address: register positions are byte
 * offsets from their own block base, display-list positions are word indexes,
 * and a plane's buffer address is printed as an offset from the first plane's
 * address in the same set ("base"). Raw register words are not printed.
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "hvs6_dlist.h"
#include "pv_hdmi.h"

#define UNREAD 0xffffffffu /* what a block the probe could not read returns (main.c) */
#define HD1_VID_CTL 0x0048u /* hdmi1's VID_CTL in the same hd block (snapshot.h's table,
			     * rpi:vc4_hdmi_regs.h:538; pv_hdmi.h names only hdmi0's 0x044) */

enum blk { B_HVS, B_PV0, B_PV1, B_HD, B_N };
#define NW 8192u /* words kept per block: 32 KiB covers registers and the list */
#define NSETS 8
#define ULMAX 64u

struct set {
	char name[40];
	int used;
	uint32_t w[B_N][NW];
	uint8_t has[B_N][NW];
	uint32_t ul[HVS6_CHANNELS][ULMAX];
	unsigned nul[HVS6_CHANNELS];
	unsigned nwords;
};

static struct set sets[NSETS];

static struct set *get_set(const char *name)
{
	int i;

	for (i = 0; i < NSETS; i++)
		if (sets[i].used && strcmp(sets[i].name, name) == 0)
			return &sets[i];
	for (i = 0; i < NSETS; i++)
		if (!sets[i].used) {
			sets[i].used = 1;
			snprintf(sets[i].name, sizeof(sets[i].name), "%s", name);
			return &sets[i];
		}
	return NULL;
}

static void put(struct set *s, enum blk b, unsigned long off, uint32_t v)
{
	if (!s || (off & 3u) || off / 4u >= NW)
		return;
	if (!s->has[b][off / 4u])
		s->nwords++;
	s->w[b][off / 4u] = v;
	s->has[b][off / 4u] = 1;
}

static int has(const struct set *s, enum blk b, uint32_t off)
{
	return off / 4u < NW && s->has[b][off / 4u];
}

static uint32_t at(const struct set *s, enum blk b, uint32_t off)
{
	return s->w[b][off / 4u];
}

static int blk_of(const char *n, enum blk *b)
{
	if (!strcmp(n, "hvs"))
		*b = B_HVS;
	else if (!strcmp(n, "pixelvalve0") || !strcmp(n, "pv"))
		*b = B_PV0;
	else if (!strcmp(n, "pixelvalve1"))
		*b = B_PV1;
	else if (!strcmp(n, "hdmi0_hd") || !strcmp(n, "hdmi0"))
		*b = B_HD;
	else
		return -1;
	return 0;
}

/* "name[0xOFF]" at p: fills name and off, returns the text after ']' or NULL. */
static const char *parse_idx(const char *p, char *name, size_t nl, unsigned long *off)
{
	const char *lb = strchr(p, '[');
	char *end;
	size_t n;

	if (!lb)
		return NULL;
	n = (size_t)(lb - p);
	if (n == 0 || n >= nl)
		return NULL;
	memcpy(name, p, n);
	name[n] = 0;
	*off = strtoul(lb + 1, &end, 0);
	if (*end != ']')
		return NULL;
	return end + 1;
}

static void parse_line(char *line)
{
	char name[48], *p = line, *q, *end;
	char blkname[16], regname[48];
	unsigned long off, v;
	enum blk b;
	struct set *s;
	unsigned ch, i, x;

	while (*p == ' ' || *p == '\t')
		p++;
	if (!strncmp(p, "mem.", 4)) {
		q = (char *)parse_idx(p + 4, name, sizeof(name), &off);
		if (!q || *q != ':' || blk_of(name, &b))
			return;
		v = strtoul(q + 1, &end, 0);
		if (end == q + 1)
			return;
		put(get_set("linux_ref"), b, off, (uint32_t)v);
	} else if ((q = strstr(p, "u3 reg "))) {
		if (sscanf(q + 7, "%15s %47s +%lx=%lx", blkname, regname, &off, &v) != 4 ||
		    blk_of(blkname, &b))
			return;
		put(get_set("u3"), b, off, (uint32_t)v);
	} else if ((q = strstr(p, "u3 dlist "))) {
		if (sscanf(q + 9, "%u [%u]=%x", &ch, &i, &x) != 3 || ch >= HVS6_CHANNELS ||
		    i >= ULMAX)
			return;
		s = get_set("u3");
		if (!s)
			return;
		if (i == 0)
			s->nul[ch] = 0; /* the start of a new read of this channel's list */
		s->ul[ch][i] = x;
		if (i + 1 > s->nul[ch])
			s->nul[ch] = i + 1;
	}
}

/* ---- printing ----------------------------------------------------------- */

static const char *mode_name(uint32_t m)
{
	static const char *const n[4] = { "DISABLED", "INIT", "RUN", "EOF" };

	return n[m & 3u];
}

static const char *order_name(uint32_t o)
{
	static const char *const n[4] = { "RGBA", "BGRA", "ARGB", "ABGR" };

	return n[o & 3u];
}

static const char *yn(int v)
{
	return v ? "yes" : "no";
}

static int have_base;
static uint64_t base_addr;

static void print_addr(uint64_t a)
{
	if (!have_base) {
		have_base = 1;
		base_addr = a;
	}
	if (a >= base_addr)
		printf("base+0x%llx", (unsigned long long)(a - base_addr));
	else
		printf("base-0x%llx", (unsigned long long)(base_addr - a));
}

static void walk(const uint32_t *dl, size_t n, uint32_t start)
{
	unsigned k;
	struct hvs6_plane p;

	for (k = 0; k < 8; k++) {
		int rc = hvs6_dlist_first_plane(dl, n, start, &p);

		if (rc == HVS6_OK) {
			printf("    plane %u (word %u, %u words): %s %s%s %ux%u at %u,%u pitch %u span %llu bytes addr ",
			       k, start, p.nwords, hvs6_format_name(p.format_code), order_name(p.order),
			       p.rgb32 ? " (rgb32)" : "", p.width, p.height, p.x, p.y, p.pitch,
			       (unsigned long long)p.pitch * p.height);
			print_addr(p.addr);
			printf("%s\n", p.unity ? "" : " (scaled)");
			start += p.nwords;
			continue;
		}
		if (rc == HVS6_ERR_END)
			printf("    end of list at word %u\n", start);
		else if (rc == HVS6_ERR_RANGE && k > 0)
			printf("    list continues past the dump at word %u\n", start);
		else
			printf("    word %u: not a plane (error %d)\n", start, rc);
		return;
	}
	printf("    (stopped after 8 planes)\n");
}

static void print_hvs(const struct set *s)
{
	enum hvs6_gen gen;
	static uint32_t dl[NW];
	unsigned ch, n = 0;
	uint32_t ver;

	if (!has(s, B_HVS, HVS6_VERSION)) {
		printf("hvs: version (+0x0000) not in dump; channels not decoded\n");
		return;
	}
	ver = at(s, B_HVS, HVS6_VERSION);
	if (ver == UNREAD) {
		printf("hvs: version reads as all ones (the block was not read); channels not decoded\n");
		return;
	}
	gen = hvs6_gen_from_version(ver);
	printf("hvs: version 0x%02x generation %s", ver & HVS6_VERSION_MASK,
	       gen == HVS6_GEN_C ? "C" : gen == HVS6_GEN_D ? "D" : "unknown");
	if (has(s, B_HVS, HVS6_CXM_SIZE))
		printf(", cxm_size %u words", at(s, B_HVS, HVS6_CXM_SIZE));
	printf("\n");
	if (gen == HVS6_GEN_UNKNOWN)
		return; /* main.c's own rule: every offset would be another generation's register */
	while (n < NW - HVS6_DLIST_OFFSET / 4u && has(s, B_HVS, HVS6_DLIST_OFFSET + 4u * n)) {
		dl[n] = at(s, B_HVS, HVS6_DLIST_OFFSET + 4u * n);
		n++;
	}
	for (ch = 0; ch < HVS6_CHANNELS; ch++) {
		uint32_t o[4];
		struct hvs6_chan c;
		int r;

		for (r = 0; r < 4; r++)
			o[r] = hvs6_chan_reg_offset(gen, ch, (enum hvs6_chan_reg)r);
		if (!(has(s, B_HVS, o[0]) && has(s, B_HVS, o[1]) && has(s, B_HVS, o[2]) &&
		      has(s, B_HVS, o[3]))) {
			printf("hvs ch%u: registers (+0x%04x..) not in dump\n", ch, o[0]);
			continue;
		}
		hvs6_chan_decode(at(s, B_HVS, o[0]), at(s, B_HVS, o[1]), at(s, B_HVS, o[2]),
				 at(s, B_HVS, o[3]), &c);
		printf("hvs ch%u (+0x%04x): enabled=%s running=%s mode=%s empty=%s frcnt=%u yline=%u size=%ux%u head=%u active=%u\n",
		       ch, o[0], yn(c.enabled), yn(c.running), mode_name(c.mode), yn(c.empty),
		       c.frcnt, c.yline, c.fwidth, c.lines, c.head, c.active);
		if (s->nul[ch]) {
			printf("  display list (u3 dlist, %u words, from index 0):\n", s->nul[ch]);
			walk(s->ul[ch], s->nul[ch], 0);
		} else if (n && c.enabled) {
			printf("  display list (memory +0x%04x, %u words, from word %u):\n",
			       HVS6_DLIST_OFFSET, n, c.active);
			walk(dl, n, c.active);
		} else {
			printf("  display list: not in dump\n");
		}
	}
}

static void print_pv(const struct set *s, enum blk b, const char *name)
{
	struct pv_control c;
	struct pv_vcontrol v;
	struct pv_timing t;
	int any = 0, k;

	for (k = 0; k < 7; k++)
		any |= has(s, b, 4u * (unsigned)k);
	if (!any) {
		if (b == B_PV0)
			printf("%s: not in dump\n", name);
		return;
	}
	if (has(s, b, PV_CONTROL) && has(s, b, PV_V_CONTROL) &&
	    at(s, b, PV_CONTROL) == UNREAD && at(s, b, PV_V_CONTROL) == UNREAD) {
		printf("%s: control and v_control read as all ones (the block was not read)\n", name);
		return;
	}
	if (!has(s, b, PV_CONTROL) || !has(s, b, PV_V_CONTROL)) {
		printf("%s: control (+0x%02x) or v_control (+0x%02x) not in dump\n", name, PV_CONTROL,
		       PV_V_CONTROL);
		return;
	}
	c = pv_decode_control(at(s, b, PV_CONTROL));
	v = pv_decode_vcontrol(at(s, b, PV_V_CONTROL));
	printf("%s: running=%s enabled=%s clk_select=%u format=%u fifo_level=%u+%u pixel_rep=%u viden=%s continuous=%s interlace=%s\n",
	       name, yn(pv_running(at(s, b, PV_CONTROL), at(s, b, PV_V_CONTROL))), yn(c.enabled),
	       c.clk_select, c.format, c.fifo_level, c.fifo_level_high, c.pixel_rep, yn(v.viden),
	       yn(v.continuous), yn(v.interlace));
	if (!(has(s, b, PV_HORZA) && has(s, b, PV_HORZB) && has(s, b, PV_VERTA) &&
	      has(s, b, PV_VERTB))) {
		printf("%s timing: not in dump\n", name);
		return;
	}
	t = pv_decode_timing(at(s, b, PV_HORZA), at(s, b, PV_HORZB), at(s, b, PV_VERTA),
			     at(s, b, PV_VERTB));
	if (!pv_timing_valid(&t)) {
		printf("%s timing: not set up (active size zero)\n", name);
		return;
	}
	printf("%s timing: %ux%u active, h total %u (sync %u bp %u fp %u), v total %u (sync %u bp %u fp %u)\n",
	       name, t.hactive, t.vactive, t.htotal, t.hsync, t.hbp, t.hfp, t.vtotal, t.vsync,
	       t.vbp, t.vfp);
}

static void print_hdmi0_hd(const struct set *s, uint32_t off, const char *name)
{
	if (!has(s, B_HD, off))
		return;
	if (at(s, B_HD, off) == UNREAD) {
		printf("hdmi0 hd: %s (+0x%04x) reads as all ones (the block was not read)\n", name,
		       off);
		return;
	}
	printf("hdmi0 hd: %s (+0x%04x) enable=%s\n", name, off,
	       yn(hd_vid_ctl_enabled(at(s, B_HD, off))));
}

static void print_set(const struct set *s)
{
	have_base = 0;
	printf("== %s (%u words) ==\n", s->name, s->nwords);
	print_hvs(s);
	print_pv(s, B_PV0, "pv0");
	print_pv(s, B_PV1, "pv1");
	if (has(s, B_HD, HD_VID_CTL))
		print_hdmi0_hd(s, HD_VID_CTL, "VID_CTL");
	else
		printf("hdmi0 hd: VID_CTL (+0x%04x) not in dump\n", HD_VID_CTL);
	print_hdmi0_hd(s, HD1_VID_CTL, "hdmi1 VID_CTL");
}

int main(void)
{
	char line[1024];
	int i, n = 0;

	while (fgets(line, sizeof(line), stdin))
		parse_line(line);
	for (i = 0; i < NSETS; i++) {
		if (!sets[i].used)
			continue;
		print_set(&sets[i]);
		n++;
	}
	if (!n) {
		fprintf(stderr, "decode: no register words found in the input\n");
		return 1;
	}
	return 0;
}
