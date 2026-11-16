/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * fb_query.h against a model of the firmware's framebuffer tags. The model
 * answers a buffer the way the property interface does: a tag it knows gets
 * bit 31 and its values, a tag it does not know is left as sent. Scenarios:
 * a Pi 4-like grant, a BCM2712 that leaves ALLOCATE unanswered, a refusal
 * with base 0, an existing firmware scanout answering the GET tags, none
 * existing, and truncated or malformed buffers.
 *
 * Build and run by tools/v3d/check71.d/fb_query.sh.
 */
#include <stdio.h>
#include <string.h>

#include "fb_query.h"

static int checks, failed;

#define CHECK(cond, what)                                                        \
	do {                                                                     \
		checks++;                                                        \
		if (!(cond)) {                                                   \
			failed++;                                                \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, what);   \
		}                                                                \
	} while (0)

/* What the model firmware does */
struct fw {
	bool knows_alloc; /* answers FRAMEBUFFER_ALLOCATE at all */
	uint32_t base, size; /* ALLOCATE answer */
	uint32_t w, h, depth, order, pitch; /* the current mode */
	bool honour_set; /* a SET tag changes the mode */
};

static void fw_answer(const struct fw *f, uint32_t *w)
{
	uint32_t total = w[0] / 4;
	for (uint32_t i = 2; i < total && w[i]; i += 3 + w[i + 1] / 4) {
		uint32_t id = w[i], *v = &w[i + 3];
		uint32_t len = w[i + 1];
		switch (id) {
		case FBQ_TAG_SET_PHYS:
		case FBQ_TAG_SET_VIRT:
			if (!f->honour_set)
				break;
			/* the request's size is the answer */
			w[i + 2] = FBQ_TAG_ANSWERED | len;
			break;
		case FBQ_TAG_GET_PHYS:
		case FBQ_TAG_GET_VIRT:
			v[0] = f->w, v[1] = f->h;
			w[i + 2] = FBQ_TAG_ANSWERED | len;
			break;
		case FBQ_TAG_SET_DEPTH:
		case FBQ_TAG_GET_DEPTH:
			if (id == FBQ_TAG_GET_DEPTH || !f->honour_set)
				v[0] = f->depth;
			w[i + 2] = FBQ_TAG_ANSWERED | len;
			break;
		case FBQ_TAG_SET_ORDER:
		case FBQ_TAG_GET_ORDER:
			if (id == FBQ_TAG_GET_ORDER || !f->honour_set)
				v[0] = f->order;
			w[i + 2] = FBQ_TAG_ANSWERED | len;
			break;
		case FBQ_TAG_GET_PITCH:
			v[0] = f->pitch;
			w[i + 2] = FBQ_TAG_ANSWERED | len;
			break;
		case FBQ_TAG_GET_VOFF:
			v[0] = v[1] = 0;
			w[i + 2] = FBQ_TAG_ANSWERED | len;
			break;
		case FBQ_TAG_ALLOCATE:
			if (!f->knows_alloc)
				break;
			v[0] = f->base, v[1] = f->size;
			w[i + 2] = FBQ_TAG_ANSWERED | len;
			break;
		}
	}
	w[1] = FBQ_STATUS_SUCCESS;
}

/* rpi_fb_alloc's request for 1920x1080 as rpi_mbox.c builds it (its layout[]
 * and in[] tables, buf[0] = n * 4, buf[1] = 0, end tag 0), written out */
static const uint32_t rpi_fb_alloc_1080[] = {
	30 * 4, 0,
	0x00048003, 8, 0, 1920, 1080,
	0x00048004, 8, 0, 1920, 1080,
	0x00048005, 4, 0, 32,
	0x00048006, 4, 0, 1,
	0x00040001, 8, 0, 4096, 0,
	0x00040008, 4, 0, 0,
	0,
};

static void test_layouts(void)
{
	uint32_t w[64];
	uint32_t n = fbq_build_alloc(w, 1920, 1080);
	CHECK(n == FBQ_ALLOC_WORDS, "alloc list word count");
	CHECK(n == sizeof(rpi_fb_alloc_1080) / 4, "alloc list as long as rpi_fb_alloc's");
	CHECK(memcmp(w, rpi_fb_alloc_1080, sizeof(rpi_fb_alloc_1080)) == 0,
	      "alloc list is rpi_fb_alloc's layout word for word");
	CHECK(w[0] == n * 4 && w[1] == 0 && w[n - 1] == 0, "alloc header and end tag");

	/* other sizes change only the four size words */
	uint32_t m = fbq_build_alloc(w, 640, 480);
	CHECK(m == n && w[5] == 640 && w[6] == 480 && w[10] == 640 && w[11] == 480,
	      "alloc list size words");

	n = fbq_build_query(w);
	CHECK(n == FBQ_QUERY_WORDS && w[0] == n * 4 && w[1] == 0 && w[n - 1] == 0,
	      "query list header and end tag");
	static const uint32_t ids[] = { 0x40003, 0x40004, 0x40005, 0x40006, 0x40008, 0x40009 };
	static const uint32_t nv[] = { 2, 2, 1, 1, 1, 2 };
	uint32_t i = 2;
	bool ok = true;
	for (unsigned k = 0; k < 6; k++) {
		ok = ok && w[i] == ids[k] && w[i + 1] == nv[k] * 4 && w[i + 2] == 0;
		for (uint32_t j = 0; j < nv[k]; j++)
			ok = ok && w[i + 3 + j] == 0;
		i += 3 + nv[k];
	}
	CHECK(ok && i == n - 1, "query list tags, all read-only (0x400xx), values zero");
	bool reads_only = true;
	for (i = 2; i < n - 1; i += 3 + w[i + 1] / 4)
		reads_only = reads_only && !(w[i] & 0x8000);
	CHECK(reads_only, "query list holds no SET tag");
}

static struct fbq_result run(const struct fw *f, bool alloc)
{
	uint32_t w[64];
	uint32_t n = alloc ? fbq_build_alloc(w, 1920, 1080) : fbq_build_query(w);
	fw_answer(f, w);
	return fbq_parse(w, n);
}

static void test_scenarios(void)
{
	/* Pi 4-like: grants a framebuffer */
	struct fw pi4 = { true, 0x3e000000, 1920 * 1080 * 4, 1920, 1080, 32, 1, 7680, true };
	struct fbq_result r = run(&pi4, true);
	CHECK(r.cls == FBQ_ALLOC_GRANTED, "pi4 grant classified granted");
	CHECK(r.base == 0x3e000000 && r.size == 1920 * 1080 * 4, "grant base and size");
	CHECK(r.pitch == 7680 && r.depth == 32 && r.order == 1, "grant pitch, depth, order");
	CHECK(r.phys_w == 1920 && r.phys_h == 1080, "grant granted size");
	CHECK((r.answered & FBQ_A_ALLOC) && (r.answered & FBQ_A_PITCH), "grant answered bits");
	CHECK(strcmp(fbq_class_name(r.cls), "granted") == 0, "granted token");

	/* BCM2712 leaving ALLOCATE unanswered while the SET/GET tags answer */
	struct fw nz = pi4;
	nz.knows_alloc = false;
	r = run(&nz, true);
	CHECK(r.cls == FBQ_TAG_UNKNOWN, "unanswered ALLOCATE is unknown");
	CHECK((r.seen & FBQ_A_ALLOC) && !(r.answered & FBQ_A_ALLOC), "unknown: seen, not answered");
	CHECK(r.base == 0 && r.size == 0, "unknown: no base");
	CHECK(strcmp(fbq_class_name(r.cls), "unknown") == 0, "unknown token");

	/* refusal: answered with base 0 */
	struct fw ref = pi4;
	ref.base = 0, ref.size = 0;
	r = run(&ref, true);
	CHECK(r.cls == FBQ_ALLOC_REFUSED, "base 0 is refused");
	CHECK(r.answered & FBQ_A_ALLOC, "refused: tag answered");
	CHECK(strcmp(fbq_class_name(r.cls), "refused") == 0, "refused token");
	ref.base = 0x3e000000; /* base but no size */
	CHECK(run(&ref, true).cls == FBQ_ALLOC_REFUSED, "size 0 is refused");

	/* an existing firmware scanout answers the GET tags */
	struct fw scan = { false, 0, 0, 1920, 1080, 32, 1, 7680, false };
	r = run(&scan, false);
	CHECK(r.cls == FBQ_PRESENT, "GET answers: present");
	CHECK(r.phys_w == 1920 && r.phys_h == 1080 && r.virt_w == 1920 && r.virt_h == 1080,
	      "present sizes");
	CHECK(r.depth == 32 && r.order == 1 && r.pitch == 7680, "present depth, order, pitch");
	CHECK(r.answered == (FBQ_A_PHYS | FBQ_A_VIRT | FBQ_A_DEPTH | FBQ_A_ORDER | FBQ_A_PITCH |
			     FBQ_A_VOFF),
	      "present answered bits");
	CHECK(strcmp(fbq_class_name(r.cls), "present") == 0, "present token");

	/* sizes zero: tags answered, nothing there */
	struct fw none = { false, 0, 0, 0, 0, 0, 0, 0, false };
	r = run(&none, false);
	CHECK(r.cls == FBQ_ABSENT, "GET answers zero: absent");
	CHECK(strcmp(fbq_class_name(r.cls), "absent") == 0, "absent token");

	/* only the virtual size nonzero still counts as present */
	uint32_t w[64];
	uint32_t n = fbq_build_query(w);
	fw_answer(&none, w);
	w[10] = 800, w[11] = 600; /* the virtual size tag's values */
	CHECK(fbq_parse(w, n).cls == FBQ_PRESENT, "virtual size alone is present");

	/* the whole query unanswered (a firmware without the tags) */
	n = fbq_build_query(w);
	w[1] = FBQ_STATUS_SUCCESS;
	r = fbq_parse(w, n);
	CHECK(r.cls == FBQ_TAG_UNKNOWN, "query without bit 31 is unknown");
	CHECK(r.answered == 0 && r.seen != 0, "query unknown: nothing answered");
	CHECK(strcmp(fbq_class_name(r.cls), "unknown") == 0, "unknown token (query)");
}

static void test_malformed(void)
{
	uint32_t w[64], c[64];
	struct fw pi4 = { true, 0x3e000000, 1920 * 1080 * 4, 1920, 1080, 32, 1, 7680, true };
	uint32_t n = fbq_build_alloc(w, 1920, 1080);
	fw_answer(&pi4, w);
	CHECK(fbq_parse(w, n).cls == FBQ_ALLOC_GRANTED, "intact buffer parses");

	/* truncated: the declared size runs past the words given */
	CHECK(fbq_parse(w, n - 1).cls == FBQ_MALFORMED, "truncated by a word");
	CHECK(fbq_parse(w, 2).cls == FBQ_MALFORMED, "two words");
	CHECK(fbq_parse(w, 0).cls == FBQ_MALFORMED, "no words");
	CHECK(fbq_parse(NULL, 8).cls == FBQ_MALFORMED, "null buffer");

	/* size field cut and the end tag lost */
	memcpy(c, w, sizeof(w));
	c[0] = (n - 1) * 4;
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "end tag missing");

	/* a tag whose value size overruns the buffer */
	memcpy(c, w, sizeof(w));
	c[3] = 4000;
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "value size overruns");

	/* value size not a word multiple */
	memcpy(c, w, sizeof(w));
	c[3] = 6;
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "value size odd");

	/* a tag too short for its fields */
	memcpy(c, w, sizeof(w));
	c[3] = 4; /* SET_PHYS with one value word; the next tag shifts, still bad */
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "tag too short for its fields");

	/* firmware error status */
	memcpy(c, w, sizeof(w));
	c[1] = 0x80000001;
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "error status");
	c[1] = FBQ_STATUS_REQUEST;
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "still a request");

	/* size field not a word multiple, or below a header */
	memcpy(c, w, sizeof(w));
	c[0] = n * 4 - 2;
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "size not a word multiple");
	c[0] = 8;
	CHECK(fbq_parse(c, n).cls == FBQ_MALFORMED, "size below a header");

	/* only an end tag, or only tags this parser does not know */
	uint32_t e[3] = { 12, FBQ_STATUS_SUCCESS, 0 };
	CHECK(fbq_parse(e, 3).cls == FBQ_MALFORMED, "empty list");
	uint32_t u[8] = { 28, FBQ_STATUS_SUCCESS, 0x00010001, 4, FBQ_TAG_ANSWERED | 4, 7, 0, 0 };
	CHECK(fbq_parse(u, 7).cls == FBQ_MALFORMED, "no framebuffer tag at all");

	CHECK(strcmp(fbq_class_name(FBQ_MALFORMED), "malformed") == 0, "malformed token");
	CHECK(strcmp(fbq_class_name((enum fbq_class)99), "malformed") == 0, "out of range token");
}

int main(void)
{
	test_layouts();
	test_scenarios();
	test_malformed();
	printf("fb_query_mock: %d of %d checks, %d failed\n", checks - failed, checks, failed);
	return failed ? 1 : 0;
}
