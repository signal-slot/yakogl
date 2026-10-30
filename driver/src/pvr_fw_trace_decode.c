/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_fw_trace_decode.c - decoder for the PowerVR firmware's trace ring, over
 * the DDK's string-format table (rgx_fwif_sf.h, 944 entries). See the header
 * for the entry format.
 *
 * The format strings are the firmware's own and take up to 20 32-bit
 * arguments; each is formatted with all 20 read from the ring (a format
 * ignores the arguments it does not name).
 */

#include "pvr_fw_trace_decode.h"
#include "rgx_fwif_sf.h"

#include <stdint.h>
#include <stdio.h>
#include <string.h>

/* The DDK's table: format string per ID, in the DDK's own X-macro form. */
static const RGXKM_STID_FMT fwt_formats[] = {
#define X(a, b, c, d, e) { (c), (d) },
	RGXFW_LOG_SFIDLIST
#undef X
};

/* Group ID -> name (RGXFW_SF_GID). */
static const char *const fwt_group_name[] = {
#define X(A, B) #B,
	RGXFW_LOG_SFGROUPLIST
#undef X
};

#define FWT_N(a) (sizeof(a) / sizeof((a)[0]))
#define FWT_MAX_ARGS 20u

struct fwt_walk {
	const uint32_t *ring;
	uint32_t dwords;   /* ring length */
	uint32_t start;    /* the trace pointer at capture time */
	uint32_t idx;      /* dwords walked from start */
};

/* The dword `offset` past the current position, 0 once the walk has gone
 * round the whole ring. */
static uint32_t fwt_read(const struct fwt_walk *w, uint32_t offset)
{
	uint32_t i = w->idx + offset;

	if (i >= w->dwords) {
		return 0;
	}
	return w->ring[(w->start + i) % w->dwords];
}

/* Table index of an ID, or FWT_N(fwt_formats) when it is not in the table. */
static uint32_t fwt_find(uint32_t id)
{
	for (uint32_t i = 0; i < FWT_N(fwt_formats); i++) {
		if (fwt_formats[i].ui32Id == id) {
			return i;
		}
	}
	return FWT_N(fwt_formats);
}

static const char *fwt_group(uint32_t id)
{
	uint32_t g = RGXFW_SF_GID(id);

	return g < FWT_N(fwt_group_name) ? fwt_group_name[g] : "?";
}

static void fwt_emit(pvr_fwt_emit_fn emit, void *ctx, const char *line)
{
	if (emit) {
		emit(ctx, line);
	} else {
		printf("%s\n", line);
	}
}

/* The partial-render lines, named for the PR; NULL for every other ID, which
 * keep the DDK's own string. The IDs and parameter counts are those of
 * rgx_fwif_sf.h: MAIN 47/48 :208-213 (no parameters), SPM 17-20 :1533-1540
 * (two parameters, the 64-bit address as high dword then low dword, the
 * DDK's "0x%08x%08x"). SPM 1/2 (:1499-1502) have no entry here: they keep the
 * DDK strings "Force Z-Load/Z-Store for partial render". */
static const char *fwt_pr_label(uint32_t id)
{
	switch (id) {
	case RGXFW_SF_SPM_ZSTORE_ADDRESS: return "ZStore";   /* :1533 */
	case RGXFW_SF_SPM_SSTORE_ADDRESS: return "SStore";   /* :1535 */
	case RGXFW_SF_SPM_ZLOAD_ADDRESS:  return "ZLoad";    /* :1537 */
	case RGXFW_SF_SPM_SLOAD_ADDRESS:  return "SLoad";    /* :1539 */
	default: return NULL;
	}
}

static void fwt_format(char *out, size_t out_sz, const struct fwt_walk *w, const char *fmt)
{
	uint32_t a[FWT_MAX_ARGS];

	for (uint32_t i = 0; i < FWT_MAX_ARGS; i++) {
		a[i] = fwt_read(w, 3 + i);   /* after the ID and the two timestamp dwords */
	}
#if defined(__GNUC__)
#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wformat-nonliteral"
#pragma GCC diagnostic ignored "-Wformat-security"
#endif
	snprintf(out, out_sz, fmt,
		 a[0], a[1], a[2], a[3], a[4], a[5], a[6], a[7], a[8], a[9],
		 a[10], a[11], a[12], a[13], a[14], a[15], a[16], a[17], a[18], a[19]);
#if defined(__GNUC__)
#pragma GCC diagnostic pop
#endif
}

uint32_t pvr_fw_trace_dump_one(const RGXFWIF_TRACEBUF_SPACE *space,
			       const uint32_t *ring_cpu, uint32_t ring_dwords,
			       uint32_t thread, pvr_fwt_emit_fn emit, void *ctx)
{
	char line[2 * RGXFW_TRACE_BUFFER_ASSERT_SIZE + IMG_SF_STRING_MAX_SIZE];
	char msg[IMG_SF_STRING_MAX_SIZE + 64];
	struct fwt_walk w;
	uint32_t count = 0;

	if (!space || !ring_cpu || ring_dwords == 0) {
		return 0;
	}
	w.ring = ring_cpu;
	w.dwords = ring_dwords;
	w.start = space->ui32TracePointer % ring_dwords;
	w.idx = 0;

	/* Skip to the first whole entry. */
	while (w.idx < w.dwords && !RGXFW_LOG_VALIDID(fwt_read(&w, 0))) {
		w.idx++;
	}

	while (w.idx < w.dwords) {
		uint32_t id = fwt_read(&w, 0);
		uint64_t ts;
		uint32_t sf;

		if (!RGXFW_LOG_VALIDID(id)) {
			w.idx++;
			continue;
		}
		ts = ((uint64_t)fwt_read(&w, 1) << 32) | fwt_read(&w, 2);
		ts = (ts & ~RGXFWT_TIMESTAMP_TIME_CLRMSK) >> RGXFWT_TIMESTAMP_TIME_SHIFT;

		if (id == RGXFW_SF_MAIN_ASSERT_FAILED) {
			snprintf(line, sizeof(line), "T%u-%s> [%llu] ASSERTION %.*s failed at %.*s:%u",
				 thread, fwt_group(id), (unsigned long long)ts,
				 (int)RGXFW_TRACE_BUFFER_ASSERT_SIZE, space->sAssertBuf.szInfo,
				 (int)RGXFW_TRACE_BUFFER_ASSERT_SIZE, space->sAssertBuf.szPath,
				 space->sAssertBuf.ui32LineNum);
			fwt_emit(emit, ctx, line);
			count++;
			break;
		}

		if (fwt_pr_label(id) && RGXFW_SF_PARAMNUM(id) == 2) {
			snprintf(line, sizeof(line), "T%u-%s> [%llu] PR %s address 0x%08x%08x",
				 thread, fwt_group(id), (unsigned long long)ts,
				 fwt_pr_label(id), fwt_read(&w, 3), fwt_read(&w, 4));
			fwt_emit(emit, ctx, line);
			count++;
			w.idx += 3 + 2;
			continue;
		}
		if (id == RGXFW_SF_MAIN_PARTIALRENDER_WITHOUT_ZSBUFFER_STORE ||
		    id == RGXFW_SF_MAIN_PARTIALRENDER_WITHOUT_ZSBUFFER_LOAD) {
			/* rgx_fwif_sf.h:208-213 */
			snprintf(line, sizeof(line), "T%u-%s> [%llu] PR: no ZS buffer (%s)",
				 thread, fwt_group(id), (unsigned long long)ts,
				 id == RGXFW_SF_MAIN_PARTIALRENDER_WITHOUT_ZSBUFFER_STORE ?
				 "store" : "load");
			fwt_emit(emit, ctx, line);
			count++;
			w.idx += 3 + RGXFW_SF_PARAMNUM(id);
			continue;
		}

		sf = fwt_find(id);
		if (sf == FWT_N(fwt_formats)) {
			uint32_t np = RGXFW_SF_PARAMNUM(id);
			int n = snprintf(line, sizeof(line), "T%u-%s> [%llu] SFID=0x%08x (%u args):",
					 thread, fwt_group(id), (unsigned long long)ts, id, np);

			for (uint32_t p = 0; p < np && n > 0 && (size_t)n < sizeof(line); p++) {
				n += snprintf(line + n, sizeof(line) - (size_t)n, " 0x%08x",
					      fwt_read(&w, 3 + p));
			}
		} else {
			fwt_format(msg, sizeof(msg), &w, fwt_formats[sf].psName);
			snprintf(line, sizeof(line), "T%u-%s> [%llu] %s",
				 thread, fwt_group(id), (unsigned long long)ts, msg);
		}
		fwt_emit(emit, ctx, line);
		count++;
		w.idx += 3 + RGXFW_SF_PARAMNUM(id);
	}
	return count;
}

void pvr_fw_trace_dump(const RGXFWIF_TRACEBUF *tb, uint32_t *const *ring_cpu,
		       uint32_t num_threads, pvr_fwt_emit_fn emit, void *ctx)
{
	char hdr[64];

	if (!tb || !ring_cpu) {
		return;
	}
	if (num_threads > RGXFW_THREAD_NUM) {
		num_threads = RGXFW_THREAD_NUM;
	}
	snprintf(hdr, sizeof(hdr), "=== FW trace: log_type=0x%08x size=%u dw ===",
		 tb->ui32LogType, tb->ui32TraceBufSizeInDWords);
	fwt_emit(emit, ctx, hdr);

	for (uint32_t t = 0; t < num_threads; t++) {
		if (pvr_fw_trace_dump_one(&tb->sTraceBuf[t], ring_cpu[t],
					  tb->ui32TraceBufSizeInDWords, t, emit, ctx) == 0) {
			char e[48];

			snprintf(e, sizeof(e), "T%u> (no valid trace entries)", t);
			fwt_emit(emit, ctx, e);
		}
	}
}
