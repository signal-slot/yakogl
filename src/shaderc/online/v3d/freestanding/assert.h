/* SPDX-License-Identifier: Apache-2.0 */
/* SPDX-FileCopyrightText: 2026 Signal Slot, Inc. */
/* See string.h in this directory. build.rs compiles with NDEBUG on these
 * targets, as Mesa's release builds do; the Rust wrapper checks the packer's
 * preconditions itself (qpu_ffi.rs), so no assert is relied on. */
#ifndef YAKOGL_FREESTANDING_ASSERT_H
#define YAKOGL_FREESTANDING_ASSERT_H
#ifdef NDEBUG
#define assert(e) ((void)0)
#else
#define assert(e) ((e) ? (void)0 : __builtin_trap())
#endif
#ifndef __cplusplus
#define static_assert _Static_assert
#endif
#endif
