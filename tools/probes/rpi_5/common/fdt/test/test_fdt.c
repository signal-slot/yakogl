/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include "fdt_walk.h"

#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int checks, failed;

#define CHECK(c)                                                           \
	do {                                                               \
		checks++;                                                  \
		if (!(c)) {                                                \
			failed++;                                          \
			printf("FAIL %s:%d: %s\n", __FILE__, __LINE__, #c); \
		}                                                          \
	} while (0)

static void put32(uint8_t *p, uint32_t v)
{
	p[0] = v >> 24;
	p[1] = v >> 16;
	p[2] = v >> 8;
	p[3] = v;
}

static uint32_t get32(const uint8_t *p)
{
	return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
	       ((uint32_t)p[2] << 8) | p[3];
}

/* Walk a private, exactly sized copy so ASan sees any read past len. */
static int try_blob(const uint8_t *b, size_t len, struct fdt_trace *t)
{
	uint8_t *c = malloc(len ? len : 1);
	int r;

	memcpy(c, b, len);
	r = fdt_walk(c, len, t);
	free(c);
	return r;
}

/* Copy, set the word at off, walk. */
static int patched(const uint8_t *b, size_t len, size_t off, uint32_t v)
{
	uint8_t *c = malloc(len);
	int r;

	memcpy(c, b, len);
	put32(c + off, v);
	r = try_blob(c, len, NULL);
	free(c);
	return r;
}

/* ---- hand-built blobs: header 40 bytes, struct block at 48 ---- */

#define OS 48

struct bld {
	uint8_t b[4096];
	size_t n;	/* struct block bytes so far */
};

static void tok(struct bld *x, uint32_t v)
{
	put32(x->b + OS + x->n, v);
	x->n += 4;
}

static void begin(struct bld *x, const char *name, size_t nlen)
{
	size_t pad = (nlen + 1 + 3) & ~(size_t)3;

	tok(x, 1);
	memset(x->b + OS + x->n, 0, pad);
	memcpy(x->b + OS + x->n, name, nlen);
	x->n += pad;
}

/* Close the struct block, add the strings block "x", fill the header. */
static size_t finish(struct bld *x)
{
	size_t ostr, total;

	tok(x, 9);
	ostr = OS + x->n;
	x->b[ostr] = 'x';
	x->b[ostr + 1] = 0;
	total = ostr + 2;
	put32(x->b, 0xd00dfeed);
	put32(x->b + 4, total);
	put32(x->b + 8, OS);
	put32(x->b + 12, ostr);
	put32(x->b + 16, 40);
	put32(x->b + 20, 17);
	put32(x->b + 24, 16);
	put32(x->b + 28, 0);
	put32(x->b + 32, 2);
	put32(x->b + 36, x->n);
	return total;
}

/* A chain of `depth` nodes in all, root included, each below it named "a". */
static int chain(unsigned int depth, struct fdt_trace *t)
{
	struct bld *x = calloc(1, sizeof(*x));
	unsigned int i;
	int r;

	begin(x, "", 0);
	for (i = 1; i < depth; i++)
		begin(x, "a", 1);
	for (i = 0; i < depth; i++)
		tok(x, 2);
	r = try_blob(x->b, finish(x), t);
	free(x);
	return r;
}

/* Root plus one child whose name is nlen long: its path is 1 + nlen. */
static int longname(size_t nlen, struct fdt_trace *t)
{
	struct bld *x = calloc(1, sizeof(*x));
	char name[512];
	int r;

	memset(name, 'n', sizeof(name));
	begin(x, "", 0);
	begin(x, name, nlen);
	tok(x, 2);
	tok(x, 2);
	r = try_blob(x->b, finish(x), t);
	free(x);
	return r;
}

static void test_built(void)
{
	struct fdt_trace t;
	struct bld *x = calloc(1, sizeof(*x));

	CHECK(chain(1, &t) == FDT_OK && t.nodes == 1 && t.max_depth == 1);
	CHECK(chain(FDT_MAX_DEPTH, &t) == FDT_OK);
	CHECK(t.nodes == FDT_MAX_DEPTH && t.max_depth == FDT_MAX_DEPTH);
	CHECK(chain(FDT_MAX_DEPTH + 1, &t) == FDT_ERR_DEPTH);

	CHECK(longname(FDT_MAX_PATH - 1, &t) == FDT_OK);
	CHECK(t.max_path == FDT_MAX_PATH);
	CHECK(longname(FDT_MAX_PATH, &t) == FDT_ERR_PATH);

	/* NOPs between tokens are skipped; one 3-byte property is counted. */
	tok(x, 4);
	begin(x, "", 0);
	tok(x, 4);
	tok(x, 3);
	tok(x, 3);
	tok(x, 0);
	tok(x, 0x01020300);
	tok(x, 2);
	tok(x, 4);
	CHECK(try_blob(x->b, finish(x), &t) == FDT_OK);
	CHECK(t.nodes == 1 && t.props == 1);
	free(x);
}

/* ---- the dtc-built tree ---- */

static void test_dtb(const char *path)
{
	FILE *f = fopen(path, "rb");
	uint8_t *b;
	size_t len, os, sz, i;
	struct fdt_trace t;

	if (!f) {
		CHECK(!"open dtb");
		return;
	}
	b = malloc(1 << 16);
	len = fread(b, 1, 1 << 16, f);
	fclose(f);
	CHECK(len > 40 && len < (1 << 16));

	CHECK(try_blob(b, len, &t) == FDT_OK);
	CHECK(t.nodes == 6);
	CHECK(t.props == 11);
	CHECK(t.max_depth == 3);
	CHECK(t.max_path == strlen("/soc/watchdog@7d200000"));

	/* every truncation is refused, and never read past */
	for (i = 0; i < len; i++)
		CHECK(try_blob(b, i, NULL) != FDT_OK);
	/* slack after totalsize is fine */
	b = realloc(b, len + 16);
	memset(b + len, 0, 16);
	CHECK(try_blob(b, len + 16, NULL) == FDT_OK);

	os = get32(b + 8);
	sz = get32(b + 36);
	CHECK(patched(b, len, 0, 0) == FDT_ERR_MAGIC);
	CHECK(patched(b, len, 4, len + 1) == FDT_ERR_TRUNC);
	CHECK(patched(b, len, 20, 15) == FDT_ERR_VERSION);
	CHECK(patched(b, len, 24, 18) == FDT_ERR_VERSION);
	CHECK(patched(b, len, 8, os + 2) == FDT_ERR_LAYOUT);
	CHECK(patched(b, len, 36, sz + 3) == FDT_ERR_LAYOUT);
	CHECK(patched(b, len, 12, 0xfffffffcu) == FDT_ERR_LAYOUT);
	CHECK(patched(b, len, 32, 0xffffffffu) == FDT_ERR_LAYOUT);

	/* The layout the patches below rely on, asserted rather than assumed:
	 * root BEGIN at os, its empty name in one word, then PROP (len, nameoff). */
	CHECK(get32(b + os) == 1);
	CHECK(get32(b + os + 4) == 0);
	CHECK(get32(b + os + 8) == 3);
	CHECK(get32(b + os + sz - 4) == 9);
	CHECK(get32(b + os + sz - 8) == 2);
	CHECK(patched(b, len, os, 7) == FDT_ERR_TOKEN);
	CHECK(patched(b, len, os, 2) == FDT_ERR_TOKEN);
	CHECK(patched(b, len, os + 8, 7) == FDT_ERR_TOKEN);	/* bad token */
	CHECK(patched(b, len, os + 8, 9) == FDT_ERR_TOKEN);	/* END early */
	CHECK(patched(b, len, os + 8, 2) == FDT_ERR_TOKEN);	/* root closed, rest follows */
	CHECK(patched(b, len, os + 12, 0x7fffffff) == FDT_ERR_TRUNC);	/* prop len */
	CHECK(patched(b, len, os + 16, 0x7fffffff) == FDT_ERR_STRING);	/* nameoff */
	CHECK(patched(b, len, os + sz - 4, 7) == FDT_ERR_TOKEN);
	CHECK(patched(b, len, os + sz - 4, 4) == FDT_ERR_TRUNC);	/* END -> NOP */
	CHECK(patched(b, len, os + sz - 8, 4) == FDT_ERR_TOKEN);	/* root unclosed */
	CHECK(patched(b, len, os + sz - 8, 1) != FDT_OK);
	free(b);
}

int main(int argc, char **argv)
{
	test_built();
	if (argc > 1)
		test_dtb(argv[1]);
	CHECK(fdt_walk(NULL, 100, NULL) == FDT_ERR_TRUNC);
	printf("fdt_walk: %d checks, %d failed\n", checks, failed);
	return failed != 0;
}
