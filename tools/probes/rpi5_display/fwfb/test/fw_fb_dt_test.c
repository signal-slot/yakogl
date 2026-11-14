/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Host test of fw_fb_dt: small DTBs are built in memory by a minimal FDT
 * writer. The addresses are made up for the test.
 */
#include <stdio.h>
#include <string.h>

#include "../fw_fb_dt.h"
#include "../../../rpi_5/common/fdt/fdt_walk.h"

static int checks, failed;

#define CHECK(c)							\
	do {								\
		checks++;						\
		if (!(c)) {						\
			failed++;					\
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__,	\
			       #c);					\
		}							\
	} while (0)

/* ---- minimal FDT writer ---- */

struct w {
	uint8_t st[2048];
	size_t ns;
	char str[512];
	size_t nstr;
	uint8_t blob[4096];
};

static void put32(uint8_t *p, uint32_t v)
{
	p[0] = v >> 24;
	p[1] = v >> 16;
	p[2] = v >> 8;
	p[3] = v;
}

static void tok(struct w *w, uint32_t t)
{
	put32(w->st + w->ns, t);
	w->ns += 4;
}

static void begin(struct w *w, const char *name)
{
	size_t n = strlen(name) + 1;

	tok(w, 1);
	memcpy(w->st + w->ns, name, n);
	w->ns += n;
	while (w->ns & 3)
		w->st[w->ns++] = 0;
}

static void end(struct w *w)
{
	tok(w, 2);
}

static void prop(struct w *w, const char *name, const void *v, size_t len)
{
	size_t no = w->nstr;

	strcpy(w->str + w->nstr, name);
	w->nstr += strlen(name) + 1;
	tok(w, 3);
	tok(w, (uint32_t)len);
	tok(w, (uint32_t)no);
	memcpy(w->st + w->ns, v, len);
	w->ns += len;
	while (w->ns & 3)
		w->st[w->ns++] = 0;
}

static void pstr(struct w *w, const char *name, const char *s)
{
	prop(w, name, s, strlen(s) + 1);
}

static void pu32(struct w *w, const char *name, uint32_t v)
{
	uint8_t b[4];

	put32(b, v);
	prop(w, name, b, 4);
}

/* A reg of 1 or 2 address cells and 1 or 2 size cells. */
static void preg(struct w *w, unsigned int ac, unsigned int sc, uint64_t a,
		 uint64_t s)
{
	uint8_t b[16];
	size_t n = 0;

	if (ac == 2)
		put32(b + n, (uint32_t)(a >> 32)), n += 4;
	put32(b + n, (uint32_t)a), n += 4;
	if (sc == 2)
		put32(b + n, (uint32_t)(s >> 32)), n += 4;
	if (sc)
		put32(b + n, (uint32_t)s), n += 4;
	prop(w, "reg", b, n);
}

static size_t finish(struct w *w)
{
	size_t off_st = 40, off_str, total;

	tok(w, 9);
	off_str = off_st + w->ns;
	total = off_str + w->nstr;
	memset(w->blob, 0, 40);
	put32(w->blob, 0xd00dfeed);
	put32(w->blob + 4, (uint32_t)total);
	put32(w->blob + 8, (uint32_t)off_st);
	put32(w->blob + 12, (uint32_t)off_str);
	put32(w->blob + 16, (uint32_t)off_st);	/* empty reserve map offset */
	put32(w->blob + 20, 17);
	put32(w->blob + 24, 16);
	put32(w->blob + 32, (uint32_t)w->nstr);
	put32(w->blob + 36, (uint32_t)w->ns);
	memcpy(w->blob + off_st, w->st, w->ns);
	memcpy(w->blob + off_str, w->str, w->nstr);
	return total;
}

static void fb_node(struct w *w, const char *name, unsigned int ac,
		    unsigned int sc, uint64_t a, uint64_t s, const char *status,
		    int with_stride)
{
	begin(w, name);
	pstr(w, "compatible", "simple-framebuffer");
	preg(w, ac, sc, a, s);
	pu32(w, "width", 1920);
	pu32(w, "height", 1080);
	if (with_stride)
		pu32(w, "stride", 7680);
	pstr(w, "format", "a8r8g8b8");
	if (status)
		pstr(w, "status", status);
	end(w);
}

static void root_cells(struct w *w, unsigned int ac, unsigned int sc)
{
	pu32(w, "#address-cells", ac);
	pu32(w, "#size-cells", sc);
}

/* ---- cases ---- */

static void verdict_is(const struct fw_fb_dt *r, const char *want)
{
	char buf[128];

	fw_fb_dt_verdict(r, buf, sizeof(buf));
	if (strcmp(buf, want))
		printf("verdict '%s' want '%s'\n", buf, want);
	CHECK(!strcmp(buf, want));
}

static void test_none(void)
{
	struct w w = {0};
	struct fw_fb_dt r;

	begin(&w, "");
	root_cells(&w, 1, 1);
	begin(&w, "chosen");
	pstr(&w, "bootargs", "x");
	end(&w);
	begin(&w, "soc");
	pstr(&w, "compatible", "simple-bus");
	end(&w);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.count == 0);
	CHECK(fw_fb_dt_pick(&r) == NULL);
	CHECK(!strcmp(fw_fb_dt_addr_token(NULL), "addr-0"));
	verdict_is(&r, "dt-fb none");
}

static void test_okay_chosen(void)
{
	struct w w = {0};
	struct fw_fb_dt r;
	const struct fw_fb_dt_node *n;

	begin(&w, "");
	root_cells(&w, 1, 1);
	begin(&w, "chosen");
	root_cells(&w, 1, 1);
	fb_node(&w, "framebuffer@a00000", 1, 1, 0xa00000, 0x7e9000, "okay", 1);
	end(&w);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.count == 1);
	n = fw_fb_dt_pick(&r);
	CHECK(n != NULL);
	if (!n)
		return;
	CHECK(!strcmp(n->path, "/chosen/framebuffer@a00000"));
	CHECK(n->status_okay && n->have_reg && n->addr == 0xa00000);
	CHECK(n->size == 0x7e9000);
	CHECK(n->width == 1920 && n->height == 1080 && n->stride == 7680);
	CHECK(!strcmp(n->format, "a8r8g8b8"));
	CHECK(!n->in_reserved);
	CHECK(!strcmp(fw_fb_dt_addr_token(n), "addr-set"));
	verdict_is(&r, "dt-fb a8r8g8b8 1920x1080 stride 7680 status okay");
}

static void test_no_status_is_okay(void)
{
	struct w w = {0};
	struct fw_fb_dt r;

	begin(&w, "");
	root_cells(&w, 1, 1);
	fb_node(&w, "framebuffer", 1, 1, 0x1000, 0x2000, NULL, 1);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.count == 1 && r.node[0].status_okay);
	CHECK(!strcmp(r.node[0].path, "/framebuffer"));
}

static void test_disabled(void)
{
	struct w w = {0};
	struct fw_fb_dt r;

	begin(&w, "");
	root_cells(&w, 1, 1);
	begin(&w, "chosen");
	fb_node(&w, "fb0", 1, 1, 0x1000, 0x2000, "disabled", 1);
	end(&w);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.count == 1 && !r.node[0].status_okay);
	verdict_is(&r, "dt-fb a8r8g8b8 1920x1080 stride 7680 status disabled");

	/* An okay node later in the tree wins over a disabled first one. */
	memset(&w, 0, sizeof(w));
	begin(&w, "");
	root_cells(&w, 1, 1);
	fb_node(&w, "fb0", 1, 1, 0x1000, 0x2000, "disabled", 1);
	fb_node(&w, "fb1", 1, 1, 0x3000, 0x2000, "okay", 1);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.count == 2);
	CHECK(fw_fb_dt_pick(&r) == &r.node[1]);
}

static void test_two_cell_reg(void)
{
	struct w w = {0};
	struct fw_fb_dt r;

	begin(&w, "");
	root_cells(&w, 2, 2);
	begin(&w, "chosen");
	root_cells(&w, 2, 2);
	fb_node(&w, "fb", 2, 2, 0x100000000ull + 0x4000, 0x300000, "okay", 1);
	end(&w);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.count == 1 && r.node[0].have_reg);
	CHECK(r.node[0].addr == 0x100000000ull + 0x4000);
	CHECK(r.node[0].size == 0x300000);
	CHECK(!strcmp(fw_fb_dt_addr_token(&r.node[0]), "addr-set"));

	/* Address cells 2, size cells 1 (a mixed parent). */
	memset(&w, 0, sizeof(w));
	begin(&w, "");
	root_cells(&w, 2, 1);
	fb_node(&w, "fb", 2, 1, 0x200000000ull, 0x10000, "okay", 1);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.node[0].addr == 0x200000000ull && r.node[0].size == 0x10000);

	/* A zero address reads addr-0. */
	memset(&w, 0, sizeof(w));
	begin(&w, "");
	root_cells(&w, 1, 1);
	fb_node(&w, "fb", 1, 1, 0, 0x10000, "okay", 1);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(!strcmp(fw_fb_dt_addr_token(&r.node[0]), "addr-0"));
}

static void test_missing_stride(void)
{
	struct w w = {0};
	struct fw_fb_dt r;

	begin(&w, "");
	root_cells(&w, 1, 1);
	fb_node(&w, "fb", 1, 1, 0x1000, 0x2000, "okay", 0);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.count == 1 && !r.node[0].have_stride);
	verdict_is(&r, "dt-fb a8r8g8b8 1920x1080 stride none status okay");
}

static void test_reserved(void)
{
	struct w w = {0};
	struct fw_fb_dt r;

	begin(&w, "");
	root_cells(&w, 1, 1);
	/* The framebuffer comes before /reserved-memory: order must not
	 * matter. */
	fb_node(&w, "fb", 1, 1, 0x5000, 0x1000, "okay", 1);
	begin(&w, "reserved-memory");
	root_cells(&w, 1, 1);
	pu32(&w, "ranges", 0);
	begin(&w, "other@100");
	preg(&w, 1, 1, 0x100, 0x100);
	end(&w);
	begin(&w, "fb@5000");
	preg(&w, 1, 1, 0x4000, 0x4000);
	pstr(&w, "no-map", "");
	end(&w);
	end(&w);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.nresv == 2);
	CHECK(r.node[0].in_reserved);
	CHECK(r.node[0].resv_addr == 0x4000 && r.node[0].resv_size == 0x4000);

	/* A region that ends before the buffer does is not a cover. */
	memset(&w, 0, sizeof(w));
	begin(&w, "");
	root_cells(&w, 1, 1);
	fb_node(&w, "fb", 1, 1, 0x5000, 0x4000, "okay", 1);
	begin(&w, "reserved-memory");
	root_cells(&w, 1, 1);
	begin(&w, "r");
	preg(&w, 1, 1, 0x4000, 0x2000);
	end(&w);
	end(&w);
	end(&w);
	CHECK(fw_fb_dt_parse(w.blob, finish(&w), &r) == 0);
	CHECK(r.nresv == 1 && !r.node[0].in_reserved);
}

static void test_corrupt(void)
{
	struct w w = {0};
	struct fw_fb_dt r;
	size_t n;

	begin(&w, "");
	root_cells(&w, 1, 1);
	fb_node(&w, "fb", 1, 1, 0x1000, 0x2000, "okay", 1);
	end(&w);
	n = finish(&w);
	CHECK(fw_fb_dt_parse(w.blob, n - 1, &r) == FDT_ERR_TRUNC);
	CHECK(r.err == FDT_ERR_TRUNC && r.count == 0);
	verdict_is(&r, "dt-fb none");
	w.blob[0] ^= 1;
	CHECK(fw_fb_dt_parse(w.blob, n, &r) == FDT_ERR_MAGIC);
	CHECK(fw_fb_dt_parse(NULL, 0, &r) == FDT_ERR_TRUNC);
}

int main(void)
{
	test_none();
	test_okay_chosen();
	test_no_status_is_okay();
	test_disabled();
	test_two_cell_reg();
	test_missing_stride();
	test_reserved();
	test_corrupt();
	printf("fw_fb_dt_test: %d of %d checks, %d failed\n", checks - failed,
	       checks, failed);
	return failed ? 1 : 0;
}
