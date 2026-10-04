/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* See assert.h in this directory: pvr_types.h includes it for PRIx64 in a
 * format macro the backend never expands. */
#ifndef YAKOGL_PVR_FREESTANDING_INTTYPES_H
#define YAKOGL_PVR_FREESTANDING_INTTYPES_H
#include <stdint.h>
#define PRIx64 "llx"
#endif
