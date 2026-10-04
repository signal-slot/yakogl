/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/*
 * glmark2_platform.h -- what glmark2's patched libmatrix/util.cc
 * (patches/libmatrix-util-zephyr.patch) asks of the image: the data files
 * linked in by CMakeLists.txt (glmark2_assets.S) and the CPU count.
 * Implemented in src/platform.c.
 */
#ifndef GLMARK2_PLATFORM_H
#define GLMARK2_PLATFORM_H

#include <stddef.h>

#ifdef __cplusplus
extern "C" {
#endif

/* The data file at `path` (e.g. "/glmark2/textures/crate-base.png"):
 * 0 and *data, *size; -1 when the image holds no such file. */
int glmark2_asset_find(const char *path, const void **data, size_t *size);

/* The number of linked-in files, and the path of file i (< count). */
size_t glmark2_asset_count(void);
const char *glmark2_asset_name(size_t i);

/* The CPUs Zephyr runs on (arch_num_cpus()). */
unsigned int glmark2_num_cpus(void);

#ifdef __cplusplus
}
#endif

#endif /* GLMARK2_PLATFORM_H */
