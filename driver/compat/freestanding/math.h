/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* See assert.h in this directory: __pvr_sfixed/__pvr_ufixed name llroundf;
 * the backend packs no fixed-point field, so it is declared, never called. */
#ifndef YAKOGL_PVR_FREESTANDING_MATH_H
#define YAKOGL_PVR_FREESTANDING_MATH_H
long long llroundf(float x);
#endif
