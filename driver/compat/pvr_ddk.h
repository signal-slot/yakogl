/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_ddk.h -- the one include of the vendored TI/Img Rogue DDK 25.2 headers
 * (third_party/ti-img-rogue-ddk-25.2, unmodified) for the PowerVR driver.
 *
 * The firmware-interface layouts depend on the build configuration, so it is
 * included first: config/am62p_linux_release/config_kernel.h, the one the
 * AM62P firmware rgx.fw.36.53.104.796 matches (driver/tools/offsetof proves
 * every number the driver uses against it).
 *
 * The DDK knows Linux (kernel and user space) and a few other systems, not
 * Zephyr. TEE_DDK is its own switch for building the kernel-side headers
 * outside a Linux kernel (a trusted execution environment); it selects the
 * standard C headers and the kernel register definitions and changes no
 * firmware-interface layout (driver/tools/offsetof proves the layouts the
 * driver uses, and driver/compat/pvr_ddk_check.c asserts them again in the
 * build that runs on the board). It is set here for every build that is not
 * a Linux kernel module or a Linux host tool.
 *
 * rgx_cr_defs_km.h refuses to be included unless IMG_EXPLICIT_INCLUDE_HWDEFS
 * is defined; the DDK itself only includes it from kernel and TEE builds
 * (rgxdefs_km.h), so it is included explicitly here as well.
 *
 * img_defs.h defines most general-purpose names only when they are not
 * defined yet (MIN, MAX, BIT, ARRAY_SIZE, __packed, ...), so where the system
 * defines them too the system's definitions are the ones in force. Three it
 * defines unconditionally, which Zephyr's toolchain headers define as well
 * (to the same effect): __maybe_unused, likely and unlikely. The system's
 * are set aside around the DDK headers and restored after them.
 */
#ifndef PVR_DDK_H
#define PVR_DDK_H

#if !defined(__linux__) && !defined(TEE_DDK)
#define TEE_DDK
#endif

#pragma push_macro("__maybe_unused")
#pragma push_macro("likely")
#pragma push_macro("unlikely")
#undef __maybe_unused
#undef likely
#undef unlikely

#include "config_kernel.h"
#include "img_types.h"
#include "img_defs.h"
#include "rgx_common.h"
#include "rgx_fwif_km.h"
#include "rgx_fwif_shared.h"
#include "rgx_fwif_alignchecks.h"
#include "rgx_riscv.h"
#include "rgx_fw_info.h"
#include "img_elf.h"
#include "rgxheapconfig.h"
#define IMG_EXPLICIT_INCLUDE_HWDEFS
#include "rgx_cr_defs_km.h"
#undef IMG_EXPLICIT_INCLUDE_HWDEFS
#include "rgxmhdefs_km.h"
#include "rgxmmudefs_km.h"

/* Restore the system's; where it had none, define them as img_defs.h does. */
#pragma pop_macro("__maybe_unused")
#pragma pop_macro("likely")
#pragma pop_macro("unlikely")
#if !defined(__maybe_unused)
#define __maybe_unused __attribute__((unused))
#endif
#if !defined(likely)
#define likely(x) __builtin_expect(!!(x), 1)
#endif
#if !defined(unlikely)
#define unlikely(x) __builtin_expect(!!(x), 0)
#endif

#endif /* PVR_DDK_H */
