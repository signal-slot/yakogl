/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * Bounds-checked walk over a flattened device tree blob (the Raspberry Pi 5
 * firmware hands one over in x0). Corrupt input is refused with an FDT_ERR_*
 * code; the walker never reads outside [blob, blob + len).
 */
#ifndef FDT_WALK_H
#define FDT_WALK_H

#include <stddef.h>
#include <stdint.h>

/* Root is depth 1: a tree whose deepest node has depth FDT_MAX_DEPTH is
 * accepted, one level more is FDT_ERR_DEPTH. */
#define FDT_MAX_DEPTH 16
/* Longest accepted node path ("/" is 1, "/a/b" is 4), without the NUL. */
#define FDT_MAX_PATH 255

enum {
	FDT_OK = 0,
	FDT_ERR_TRUNC = -1,	/* header or a block does not fit in len */
	FDT_ERR_MAGIC = -2,
	FDT_ERR_VERSION = -3,
	FDT_ERR_LAYOUT = -4,	/* offsets misaligned or blocks out of range */
	FDT_ERR_TOKEN = -5,	/* unknown or misplaced structure token */
	FDT_ERR_STRING = -6,	/* unterminated name or bad name offset */
	FDT_ERR_DEPTH = -7,
	FDT_ERR_PATH = -8,
};

struct fdt_trace {
	unsigned int nodes;	/* BEGIN_NODE tokens, root included */
	unsigned int props;
	unsigned int max_depth;
	size_t max_path;	/* longest node path seen */
};

/* Returns FDT_OK or an FDT_ERR_*; t may be NULL. */
int fdt_walk(const void *blob, size_t len, struct fdt_trace *t);

#endif
