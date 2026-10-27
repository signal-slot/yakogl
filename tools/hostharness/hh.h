/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * hh.h - the host harness's recording OSAL (rec.c) as seen by the harness
 * programs and the entry-point interposer. See README.md for what is logged.
 */
#ifndef HH_H
#define HH_H

#include <stddef.h>
#include <stdint.h>

/* Install the recording OSAL (osal_set_ops); reads the HH_* environment.
 * Every harness program calls this first. */
void hh_install(void);

/* All harness output goes through here, into stdout (one stream, so the
 * order of OSAL events, library log lines and application results is part
 * of what is compared). */
int hh_printf(const char *fmt, ...) __attribute__((format(printf, 1, 2)));

/* Hash of a byte range; with HH_NHASH=1 the bytes 0x00 and 0xA5 hash alike,
 * so runs with different fills of fresh allocations can be compared. */
uint64_t hh_hash(const void *p, size_t n);

/* Application memory the library may map (window surfaces, native pixmaps):
 * phys values inside such a region are host addresses; the region is hashed
 * with the GPU memory and gpu_read/gpu_write of a mapping of it reach it. */
void hh_region_add(void *p, size_t n, const char *name);
void hh_region_del(void *p);

/* Allocation-failure injection: the n-th gpu_alloc from now fails
 * (1 = the next one); 0 disables. */
void hh_fail_alloc_in(unsigned n);

/* Hash every live allocation and region now (tag names the point). */
void hh_memdump(const char *tag);

/* A named step in a harness program (appears in the log). */
void hh_step(const char *fmt, ...) __attribute__((format(printf, 1, 2)));

/* Non-zero while the application is in an object-storage entry point
 * (wrap_rt.c). */
int hh_storage_call(void);

/* Number of harness-detected errors so far (HH-ERROR lines). */
unsigned hh_errors(void);

#endif
