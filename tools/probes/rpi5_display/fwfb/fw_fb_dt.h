/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * The firmware's framebuffer as a kernel= image can learn it from the device
 * tree it is handed: every node whose compatible lists "simple-framebuffer"
 * (status okay or not), with reg, width, height, stride, format and the
 * /reserved-memory region that covers it.
 *
 * The blob is first checked with the bounds-checked walker of
 * tools/probes/rpi_5/common/fdt (used read-only); a refused blob is an
 * error, never a partial answer. Addresses are returned in the struct for
 * the run-time log only: the verdict text names them as "addr-set" or
 * "addr-0" and never prints one.
 */
#ifndef FW_FB_DT_H
#define FW_FB_DT_H

#include <stddef.h>
#include <stdint.h>

#define FW_FB_DT_MAX_NODES 4
#define FW_FB_DT_MAX_RESV 16
#define FW_FB_DT_FORMAT_LEN 16

struct fw_fb_dt_node {
	char path[256];
	int status_okay;	/* no status property, "okay" or "ok" */
	int have_reg;		/* a usable first (address, size) pair */
	uint64_t addr, size;	/* size 0: reg had no size cell */
	int have_width, have_height, have_stride, have_format;
	uint32_t width, height, stride;
	char format[FW_FB_DT_FORMAT_LEN];	/* e.g. "a8r8g8b8" */
	int in_reserved;	/* a /reserved-memory child covers the buffer */
	uint64_t resv_addr, resv_size;
};

struct fw_fb_dt {
	int err;		/* 0, or an FDT_ERR_* from fdt_walk.h */
	unsigned int count;	/* nodes found, may exceed the stored ones */
	struct fw_fb_dt_node node[FW_FB_DT_MAX_NODES];
	unsigned int nresv;	/* /reserved-memory children with a reg */
};

/* Parses blob[0..len). Returns 0, or the FDT_ERR_* (also in out->err). */
int fw_fb_dt_parse(const void *blob, size_t len, struct fw_fb_dt *out);

/* The node the verdict speaks of: the first okay one, else the first;
 * NULL when there is none. */
const struct fw_fb_dt_node *fw_fb_dt_pick(const struct fw_fb_dt *r);

/* "addr-set" (reg present, non-zero address) or "addr-0". */
const char *fw_fb_dt_addr_token(const struct fw_fb_dt_node *n);

/*
 * One token into buf: "dt-fb none" (no node, or a refused blob), else
 * "dt-fb <format> <w>x<h> stride <n> status <okay|disabled>". A missing
 * format or stride reads "none"; a missing width or height reads 0.
 */
void fw_fb_dt_verdict(const struct fw_fb_dt *r, char *buf, size_t n);

#endif
