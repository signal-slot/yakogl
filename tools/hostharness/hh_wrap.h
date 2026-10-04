/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* Runtime of the generated entry-point interposer (wrap_gen.c, gen_wrap.py). */
#ifndef HH_WRAP_H
#define HH_WRAP_H

#include <stddef.h>
#include <stdint.h>
#include <string.h>
#include "hh.h"

extern const unsigned hh_nnames;
extern const char *const hh_names[];
extern uint64_t hh_calls[];

/* the entry point the application is in (-1: none), for hh_storage_call() */
extern int hh_cur;
static inline void hh_call(unsigned idx) { hh_calls[idx]++; }

#define HH_MIN(a, b) ((a) < (b) ? (a) : (b))
/* gen/delete counts: log at most 64 names (a hostile n is not a buffer size) */
#define HH_CAP(n) ((n) < 0 ? 0 : (n) > 64 ? 64 : (size_t)(n))

/* Probe buffer for getters whose output length depends on the arguments. */
/* Bytes sentinel-filled and scanned per probe (the buffer itself is 1 MiB,
 * so a longer write still lands in harness memory and shows as the full
 * extent). */
#define HH_PROBE_BYTES 8192u
extern unsigned char hh_probe_buf[];
extern unsigned char hh_probe_a[];

/* Run CALL (which writes into hh_probe_buf) twice with two different sentinel
 * fills and set N to the number of ESZ-byte elements it wrote. The real call
 * with the application's pointer follows, so the library still sees exactly
 * what the application passed (alignment, size). */
#define HH_PROBE(N, ESZ, CALL) do {                                          \
	memset(hh_probe_buf, 0xA5, HH_PROBE_BYTES);                          \
	CALL;                                                                \
	memcpy(hh_probe_a, hh_probe_buf, HH_PROBE_BYTES);                    \
	memset(hh_probe_buf, 0x5A, HH_PROBE_BYTES);                          \
	CALL;                                                                \
	(N) = hh_probe_extent((ESZ));                                        \
} while (0)

size_t hh_probe_extent(size_t esz);
/* print N values of KIND ("u8", "i32", "x32", "i64", "ptr") read from P
 * (memcpy'd: P may be unaligned) */
void hh_vals(const char *kind, const void *p, size_t n);
/* print a string (at most MAX bytes, stops at NUL; escaped) or "(null)" */
void hh_str(const char *s, size_t max);
/* the same into a rotating static buffer, for use as a %s argument */
const char *hh_cs(const char *s);
/* stable small id for an opaque handle (0 for NULL) */
unsigned hh_hid(void *p);
/* glReadPixels: hash of the bytes the call is specified to write */
void hh_readpixels(int w, int h, unsigned format, unsigned type, void *pixels);
void hh_pixelstore(unsigned pname, int param);
/* eglGetProcAddress: the library's entry point -> its interposer */
void *hh_map_proc(void *p);
const char *hh_proc_name(void *p);

#endif
