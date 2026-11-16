/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include "fw_fb_dt.h"

#include <stdio.h>
#include <string.h>

#include "../../rpi_5/common/fdt/fdt_walk.h"

#define TK_BEGIN 1u
#define TK_END_NODE 2u
#define TK_PROP 3u
#define TK_NOP 4u

#define RESV_PREFIX "/reserved-memory/"

static uint32_t be32(const uint8_t *p)
{
	return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
	       ((uint32_t)p[2] << 8) | p[3];
}

/* n big-endian cells (n is 1 or 2) at p. */
static uint64_t cells(const uint8_t *p, unsigned int n)
{
	uint64_t v = be32(p);

	if (n == 2)
		v = (v << 32) | be32(p + 4);
	return v;
}

/* A stringlist property holds the entry s. */
static int has_string(const uint8_t *v, size_t len, const char *s)
{
	size_t i = 0, sl = strlen(s);

	while (i < len) {
		size_t n = strnlen((const char *)v + i, len - i);

		if (n == sl && !memcmp(v + i, s, sl))
			return 1;
		i += n + 1;
	}
	return 0;
}

/* [a, a + n) lies inside [ra, ra + rn), without overflow. */
static int covers(uint64_t ra, uint64_t rn, uint64_t a, uint64_t n)
{
	return a >= ra && a - ra <= rn && n <= rn - (a - ra);
}

/* One open node: what its properties said so far. */
struct open_node {
	struct fw_fb_dt_node n;
	int is_fb;
	unsigned int ac, sc;	/* #address-cells, #size-cells for children */
};

struct resv {
	uint64_t addr, size;
};

int fw_fb_dt_parse(const void *blob, size_t len, struct fw_fb_dt *out)
{
	const uint8_t *b = blob;
	struct open_node st[FDT_MAX_DEPTH + 2];
	struct resv resv[FW_FB_DT_MAX_RESV];
	unsigned int depth = 0, i, j, nresv = 0;
	size_t p;
	int rc;

	memset(out, 0, sizeof(*out));
	rc = fdt_walk(blob, len, NULL);
	if (rc != FDT_OK) {
		out->err = rc;
		return rc;
	}
	p = be32(b + 8);

	for (;;) {
		uint32_t tok = be32(b + p);

		p += 4;
		if (tok == TK_NOP)
			continue;
		if (tok == TK_BEGIN) {
			const char *name = (const char *)b + p;
			size_t n = strlen(name), pl = 0;
			struct open_node *o = &st[++depth];

			p = (p + n + 1 + 3) & ~(size_t)3;
			memset(o, 0, sizeof(*o));
			o->ac = 2;	/* spec defaults */
			o->sc = 1;
			if (depth == 1) {
				strcpy(o->n.path, "/");
			} else {
				const char *pp = st[depth - 1].n.path;

				pl = strlen(pp);
				memcpy(o->n.path, pp, pl);
				if (pl != 1)
					o->n.path[pl++] = '/';
				memcpy(o->n.path + pl, name, n + 1);
			}
			o->n.status_okay = 1;
		} else if (tok == TK_PROP) {
			uint32_t vl = be32(b + p), no = be32(b + p + 4);
			const uint8_t *v = b + p + 8;
			const char *pn = (const char *)b + be32(b + 12) + no;
			struct open_node *o = &st[depth];

			p = (p + 8 + vl + 3) & ~(size_t)3;
			if (!strcmp(pn, "#address-cells") && vl == 4) {
				o->ac = be32(v);
			} else if (!strcmp(pn, "#size-cells") && vl == 4) {
				o->sc = be32(v);
			} else if (!strcmp(pn, "compatible")) {
				if (has_string(v, vl, "simple-framebuffer"))
					o->is_fb = 1;
			} else if (!strcmp(pn, "reg") && depth > 1) {
				/* Cells of the parent describe this reg. */
				unsigned int a = st[depth - 1].ac;
				unsigned int s = st[depth - 1].sc;

				if ((a == 1 || a == 2) && s <= 2 &&
				    vl >= 4 * (a + s)) {
					o->n.have_reg = 1;
					o->n.addr = cells(v, a);
					o->n.size = s ? cells(v + 4 * a, s) : 0;
				}
			} else if (!strcmp(pn, "width") && vl == 4) {
				o->n.have_width = 1;
				o->n.width = be32(v);
			} else if (!strcmp(pn, "height") && vl == 4) {
				o->n.have_height = 1;
				o->n.height = be32(v);
			} else if (!strcmp(pn, "stride") && vl == 4) {
				o->n.have_stride = 1;
				o->n.stride = be32(v);
			} else if (!strcmp(pn, "format") && vl > 0) {
				size_t n = strnlen((const char *)v, vl);

				if (n >= sizeof(o->n.format))
					n = sizeof(o->n.format) - 1;
				o->n.have_format = 1;
				memcpy(o->n.format, v, n);
				o->n.format[n] = 0;
			} else if (!strcmp(pn, "status")) {
				o->n.status_okay = has_string(v, vl, "okay") ||
						   has_string(v, vl, "ok");
			}
		} else if (tok == TK_END_NODE) {
			struct open_node *o = &st[depth];

			if (o->is_fb) {
				if (out->count < FW_FB_DT_MAX_NODES)
					out->node[out->count] = o->n;
				out->count++;
			} else if (depth == 3 && o->n.have_reg &&
				   !strncmp(o->n.path, RESV_PREFIX,
					    sizeof(RESV_PREFIX) - 1) &&
				   nresv < FW_FB_DT_MAX_RESV) {
				resv[nresv].addr = o->n.addr;
				resv[nresv].size = o->n.size;
				nresv++;
			}
			depth--;
		} else {
			break;	/* FDT_END; fdt_walk checked the rest */
		}
	}

	out->nresv = nresv;
	for (i = 0; i < out->count && i < FW_FB_DT_MAX_NODES; i++) {
		struct fw_fb_dt_node *f = &out->node[i];
		uint64_t ext = f->size ? f->size
				       : (uint64_t)f->stride * f->height;

		if (!f->have_reg || !ext)
			continue;
		for (j = 0; j < nresv; j++) {
			if (covers(resv[j].addr, resv[j].size, f->addr, ext)) {
				f->in_reserved = 1;
				f->resv_addr = resv[j].addr;
				f->resv_size = resv[j].size;
				break;
			}
		}
	}
	return 0;
}

const struct fw_fb_dt_node *fw_fb_dt_pick(const struct fw_fb_dt *r)
{
	unsigned int i, n = r->count < FW_FB_DT_MAX_NODES
				    ? r->count : FW_FB_DT_MAX_NODES;

	for (i = 0; i < n; i++)
		if (r->node[i].status_okay)
			return &r->node[i];
	return n ? &r->node[0] : NULL;
}

const char *fw_fb_dt_addr_token(const struct fw_fb_dt_node *n)
{
	return n && n->have_reg && n->addr ? "addr-set" : "addr-0";
}

void fw_fb_dt_verdict(const struct fw_fb_dt *r, char *buf, size_t n)
{
	const struct fw_fb_dt_node *f = r->err ? NULL : fw_fb_dt_pick(r);
	char stride[16];

	if (!f) {
		snprintf(buf, n, "dt-fb none");
		return;
	}
	if (f->have_stride)
		snprintf(stride, sizeof(stride), "%u", (unsigned)f->stride);
	else
		snprintf(stride, sizeof(stride), "none");
	snprintf(buf, n, "dt-fb %s %ux%u stride %s status %s",
		 f->have_format ? f->format : "none",
		 (unsigned)f->width, (unsigned)f->height, stride,
		 f->status_okay ? "okay" : "disabled");
}
