/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#include "fdt_walk.h"

#include <string.h>

#define MAGIC 0xd00dfeedu
#define TK_BEGIN 1u
#define TK_END_NODE 2u
#define TK_PROP 3u
#define TK_NOP 4u
#define TK_END 9u
#define HDR_SIZE 40u

static uint32_t be32(const uint8_t *p)
{
	return ((uint32_t)p[0] << 24) | ((uint32_t)p[1] << 16) |
	       ((uint32_t)p[2] << 8) | p[3];
}

/* [off, off + n) lies inside size, without overflow. */
static int fits(size_t off, size_t n, size_t size)
{
	return off <= size && n <= size - off;
}

int fdt_walk(const void *blob, size_t len, struct fdt_trace *t)
{
	const uint8_t *b = blob;
	struct fdt_trace scratch;
	size_t total, off_st, off_str, sz_str, sz_st, p, end;
	size_t plen_at[FDT_MAX_DEPTH + 1];	/* path length at each depth */
	unsigned int depth = 0;
	int root_done = 0;

	if (!t)
		t = &scratch;
	memset(t, 0, sizeof(*t));

	if (!b || len < HDR_SIZE)
		return FDT_ERR_TRUNC;
	if (be32(b) != MAGIC)
		return FDT_ERR_MAGIC;
	total = be32(b + 4);
	if (total < HDR_SIZE || total > len)
		return FDT_ERR_TRUNC;
	if (be32(b + 20) < 16 || be32(b + 24) > 17)	/* version, last_comp */
		return FDT_ERR_VERSION;
	off_st = be32(b + 8);
	off_str = be32(b + 12);
	sz_str = be32(b + 32);
	sz_st = be32(b + 36);
	if ((off_st & 3) || (sz_st & 3) || off_st < HDR_SIZE ||
	    off_str < HDR_SIZE || !fits(off_st, sz_st, total) ||
	    !fits(off_str, sz_str, total))
		return FDT_ERR_LAYOUT;

	p = off_st;
	end = off_st + sz_st;
	for (;;) {
		uint32_t tok;

		if (!fits(p, 4, end))
			return FDT_ERR_TRUNC;
		tok = be32(b + p);
		p += 4;
		if (tok == TK_NOP)
			continue;
		if (tok == TK_END)
			return root_done ? FDT_OK : FDT_ERR_TOKEN;
		if (root_done)
			return FDT_ERR_TOKEN;

		if (tok == TK_BEGIN) {
			size_t n = 0, plen;

			while (fits(p + n, 1, end) && b[p + n])
				n++;
			if (!fits(p + n, 1, end))
				return FDT_ERR_STRING;
			if ((depth == 0) != (n == 0))
				return depth ? FDT_ERR_STRING : FDT_ERR_TOKEN;
			if (depth + 1 > FDT_MAX_DEPTH)
				return FDT_ERR_DEPTH;
			plen = depth == 0 ? 1 : plen_at[depth] + (depth > 1) + n;
			if (plen > FDT_MAX_PATH)
				return FDT_ERR_PATH;
			depth++;
			plen_at[depth] = plen;
			t->nodes++;
			if (depth > t->max_depth)
				t->max_depth = depth;
			if (plen > t->max_path)
				t->max_path = plen;
			p += (n + 1 + 3) & ~(size_t)3;
			if (p > end)
				return FDT_ERR_TRUNC;
		} else if (tok == TK_END_NODE) {
			if (depth == 0)
				return FDT_ERR_TOKEN;
			if (--depth == 0)
				root_done = 1;
		} else if (tok == TK_PROP) {
			size_t plen, noff, s;

			if (depth == 0)
				return FDT_ERR_TOKEN;
			if (!fits(p, 8, end))
				return FDT_ERR_TRUNC;
			plen = be32(b + p);
			noff = be32(b + p + 4);
			p += 8;
			if (!fits(p, plen, end))
				return FDT_ERR_TRUNC;
			if (noff >= sz_str)
				return FDT_ERR_STRING;
			for (s = noff; s < sz_str && b[off_str + s]; s++)
				;
			if (s >= sz_str)
				return FDT_ERR_STRING;
			plen = (plen + 3) & ~(size_t)3;
			if (!fits(p, plen, end))
				return FDT_ERR_TRUNC;
			p += plen;
			t->props++;
		} else {
			return FDT_ERR_TOKEN;
		}
	}
}
