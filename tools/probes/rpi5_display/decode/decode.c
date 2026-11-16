/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * decode: reads a text dump of Pi 5 scanout register words on stdin and prints
 * the HVS6 channels and display-list planes, the pixel valve and the HDMI0
 * status as readable lines. The decoding is the reviewed header-only code,
 * included unchanged: hvs6_dlist.h and pv_hdmi.h.
 *
 * Accepted line formats (anything else is ignored; formats 2 to 4 may carry a
 * prefix such as a timestamp, the marker is searched for):
 *   1. linux_ref.sh (tools/probes/rpi5_display/linux_ref.md), at line start:
 *        mem.hvs[0xOFF]: 0xVAL          HVS block, byte offset OFF
 *        mem.pixelvalve0[0xOFF]: 0xVAL  (also pixelvalve1)
 *        mem.hdmi0_hd[0xOFF]: 0xVAL     HDMI0 "hd" block
 *      One set, "linux_ref". The script reads 64 HVS words, 16 pixel valve
 *      words and 16 hd words, so the HVS display list (from 0x4000), the
 *      generation-D channels (0x100 and up) and HD_VID_CTL (0x44) are normally
 *      not in it and are reported as such. If a dump carries them anyway
 *      (mem.hvs[0x4000+...]) they are decoded.
 *   2. the probe console log, raw words:
 *        rpi5_display: D2 <when> <block>[0xOFF]: w0 w1 w2 w3
 *      <block> is hvs, pixelvalve0 or pixelvalve1, w* hex words (no 0x),
 *      consecutive from OFF. One set per <when> ("D2 before", "D2 after").
 *   3. the probe's snapshot lines (rpi5_scanout/snapshot.h), set "u3":
 *        u3 reg <hvs|pv|hdmi0> <name> +0xOFF=0xVAL
 *      "pv" is pixel valve 0. The hdmi_vid_ctl line is shown as a named line:
 *      its table offset (0x0038) differs from pv_hdmi.h HD_VID_CTL (0x44).
 *   4.   u3 dlist <ch> [<i>]=0xWORD
 *      decoded from its index 0 (the start word the probe was given).
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
	int vid_named;
	uint32_t vid_off, vid_val;
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
	char name[48], when[16], *p = line, *q, *end;
	char blkname[16], regname[48], lab[40];
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
	} else if ((q = strstr(p, "rpi5_display: D2 "))) {
		if (sscanf(q + 17, "%15s", when) != 1)
			return;
		q = (char *)parse_idx(q + 17 + strlen(when) + 1, name, sizeof(name), &off);
		if (!q || *q != ':' || blk_of(name, &b))
			return;
		snprintf(lab, sizeof(lab), "D2 %s", when);
		s = get_set(lab);
		q++;
		for (i = 0; i < 4; i++) {
			v = strtoul(q, &end, 16);
			if (end == q)
				break;
			put(s, b, off + 4ul * i, (uint32_t)v);
			q = end;
		}
	} else if ((q = strstr(p, "u3 reg "))) {
		if (sscanf(q + 7, "%15s %47s +%lx=%lx", blkname, regname, &off, &v) != 4 ||
		    blk_of(blkname, &b))
			return;
		s = get_set("u3");
		if (b == B_HD) {
			if (!strcmp(regname, "hdmi_vid_ctl") && s) {
				s->vid_named = 1;
				s->vid_off = (uint32_t)off;
				s->vid_val = (uint32_t)v;
			}
			return;
		}
		put(s, b, off, (uint32_t)v);
	} else if ((q = strstr(p, "u3 dlist "))) {
		if (sscanf(q + 9, "%u [%u]=%x", &ch, &i, &x) != 3 || ch >= HVS6_CHANNELS ||
		    i >= ULMAX)
			return;
		s = get_set("u3");
		if (!s)
			return;
		s->ul[ch][i] = x;
		if (i + 1 > s->nul[ch])
			s->nul[ch] = i + 1;
	}
}

/* ---- printing ----------------------------------------------------------- */

static const char *mode_name(uint32_t m)
{
	static const char *const n[4] = {"DISABLED", "INIT", "RUN", "EOF"};

	return n[m & 3u];
}

static const char *order_name(uint32_t o)
{
	static const char *const n[4] = {"RGBA", "BGRA", "ARGB", "ABGR"};

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

	if (!has(s, B_HVS, HVS6_VERSION)) {
		printf("hvs: version (+0x0000) not in dump; channels not decoded\n");
		return;
	}
	gen = hvs6_gen_from_version(at(s, B_HVS, HVS6_VERSION));
	printf("hvs: version 0x%02x generation %s", at(s, B_HVS, HVS6_VERSION) & HVS6_VERSION_MASK,
	       gen == HVS6_GEN_C ? "C" : gen == HVS6_GEN_D ? "D" : "unknown");
	if (has(s, B_HVS, HVS6_CXM_SIZE))
		printf(", cxm_size %u words", at(s, B_HVS, HVS6_CXM_SIZE));
	printf("\n");
	if (gen == HVS6_GEN_UNKNOWN)
		return;
	while (n < NW - HVS6_DLIST_OFFSET / 4u && has(s, B_HVS, HVS6_DLIST_OFFSET + 4u * n)) {
		dl[n] = at(s, B_HVS, HVS6_DLIST_OFFSET + 4u * n);
		n++;
	}
	for (ch = 0; ch < HVS6_CHANNELS; ch++) {
		uint32_t o[4];
		struct hvs6_chan c;
		int r, all = 1;

		for (r = 0; r < 4; r++) {
			o[r] = hvs6_chan_reg_offset(gen, ch, (enum hvs6_chan_reg)r);
			all &= has(s, B_HVS, o[r]);
		}
		if (!all) {
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

static void print_set(const struct set *s)
{
	have_base = 0;
	printf("== %s (%u words) ==\n", s->name, s->nwords);
	print_hvs(s);
	print_pv(s, B_PV0, "pv0");
	print_pv(s, B_PV1, "pv1");
	if (has(s, B_HD, HD_VID_CTL))
		printf("hdmi0: HD_VID_CTL (+0x%04x) enable=%s\n", HD_VID_CTL,
		       yn(hd_vid_ctl_enabled(at(s, B_HD, HD_VID_CTL))));
	else
		printf("hdmi0: HD_VID_CTL (+0x%04x) not in dump\n", HD_VID_CTL);
	if (s->vid_named)
		printf("hdmi0: hdmi_vid_ctl named line (+0x%04x) enable=%s (offset differs from HD_VID_CTL)\n",
		       s->vid_off, yn(hd_vid_ctl_enabled(s->vid_val)));
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
