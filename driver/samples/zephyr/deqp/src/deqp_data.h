/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
#ifndef DEQP_DATA_H
#define DEQP_DATA_H
#include <stddef.h>
#include <stdint.h>
#ifdef __cplusplus
extern "C" {
#endif
/* The test data, embedded at build time (tools/embed_data.py), sorted by name. */
struct deqp_file {
	const char *name;
	const uint8_t *bytes;
	uint32_t size;
};
extern const struct deqp_file deqp_files[];
extern const size_t deqp_num_files;
#ifdef __cplusplus
}
#endif
#endif
