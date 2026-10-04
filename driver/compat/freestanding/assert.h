/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* The C library headers Mesa's csbgen helpers include
 * (csbgen/pvr_packet_helpers.h, include/pvr_types.h), for a target with no
 * operating system: build.rs compiles driver/src/pvr/generated/csbgen_wrap.c
 * against this directory there (target_os = "none"). Declarations only; no
 * packer the backend calls uses any of them at run time. build.rs compiles
 * with NDEBUG on every target, as Mesa's release builds do. */
#ifndef YAKOGL_PVR_FREESTANDING_ASSERT_H
#define YAKOGL_PVR_FREESTANDING_ASSERT_H
#ifdef NDEBUG
#define assert(e) ((void)0)
#else
#define assert(e) ((e) ? (void)0 : __builtin_trap())
#endif
#endif
