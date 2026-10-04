/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_rgxlayer_check.c -- the DDK side of the Rust rgxlayer
 * (driver/rust/src/gpu/pvr/rgxlayer.rs), asserted by the compiler that builds
 * the board's image (and by the host check, tools/pvr/rgxlayer_check.c). No
 * code: a number that moved stops the build.
 *
 * The Rust side mirrors these DDK types and values in its exported
 * signatures; the layout of struct yd_pvr_layer itself is pinned in
 * yakogl_drivers.h (both sides).
 */
#include <stdbool.h>
#include <stddef.h>

#include "pvr_ddk.h"
#include "rgxlayer.h"
#include "pvr_rgx_core.h"
#include <yakogl_drivers.h>

/* The types in the signatures (rgxlayer.rs: ImgBool = bool, PvrsrvError =
 * u32, ImgDevPhyAddr / ImgDevVirtAddr = one u64). */
_Static_assert(sizeof(IMG_BOOL) == 1 && sizeof(bool) == 1, "IMG_BOOL is C bool");
_Static_assert(sizeof(PVRSRV_ERROR) == 4, "PVRSRV_ERROR");
_Static_assert(sizeof(IMG_DEV_PHYADDR) == 8 && offsetof(IMG_DEV_PHYADDR, uiAddr) == 0, "IMG_DEV_PHYADDR");
_Static_assert(sizeof(IMG_DEV_VIRTADDR) == 8 && offsetof(IMG_DEV_VIRTADDR, uiAddr) == 0, "IMG_DEV_VIRTADDR");
_Static_assert(sizeof(IMG_UINT16) == 2 && sizeof(IMG_UINT32) == 4 && sizeof(IMG_UINT64) == 8, "IMG_UINTn");
_Static_assert(sizeof(IMG_INT32) == 4 && sizeof(IMG_UINT8) == 1, "IMG_INT32, IMG_UINT8");

/* The PVRSRV_ERROR values the Rust side returns (rgxlayer.rs PVRSRV_*). */
_Static_assert(PVRSRV_OK == 0, "PVRSRV_OK");
_Static_assert(PVRSRV_ERROR_INVALID_PARAMS == 3, "PVRSRV_ERROR_INVALID_PARAMS");
_Static_assert(PVRSRV_ERROR_TIMEOUT == 9, "PVRSRV_ERROR_TIMEOUT");
_Static_assert(PVRSRV_ERROR_NOT_SUPPORTED == 20, "PVRSRV_ERROR_NOT_SUPPORTED");

/* The core's words (yakogl_drivers.h, = rgxlayer.rs FEATURES / ERNS_BRNS)
 * against what the configuration headers say, feature by feature. */
_Static_assert(YD_PVR_RGX_FEATURES == PVR_RGX_FEATURES_FROM_HEADERS,
	       "YD_PVR_RGX_FEATURES != the features of rgxconfig_km_36.V.104.796.h");
_Static_assert(YD_PVR_RGX_ERNS_BRNS == PVR_RGX_ERNS_BRNS_FROM_HEADERS,
	       "YD_PVR_RGX_ERNS_BRNS != the errata of rgxcore_km_36.53.104.796.h");

/* rgxlayer.rs answers every feature query from array index 0 and keeps
 * RGX_FEATURE_WITH_VALUE_INDEX's 30 values in that order. */
#define PIN_INDEX0(name, mask, present) \
	_Static_assert(RGX_FEATURE_##name##_ARRAY_INDEX == 0, "RGX_FEATURE_" #name " not in word 0");
PVR_RGX_FEATURE_BITS(PIN_INDEX0)
#undef PIN_INDEX0
_Static_assert(RGX_FEATURE_WITH_VALUES_MAX_IDX == 30, "RGX_FEATURE_WITH_VALUE_INDEX");

/* The configuration rgxstartstop.c is compiled in: the META and MIPS entry
 * points exist (rgxlayer.rs answers them "not supported"); the paths that
 * would need more of this layer (RGXInitMMURangeRegisters uses
 * RGX_LAYER_PARAMS) are not compiled. */
#if !defined(RGX_FEATURE_MIPS_BIT_MASK) || !defined(RGX_FEATURE_META_MAX_VALUE_IDX)
#error "rgxlayer.rs exports the MIPS and META entry points this configuration declares"
#endif
#if defined(RGX_FEATURE_MMU_VERSION_MAX_VALUE_IDX) || defined(RGX_FEATURE_HOST_SECURITY_VERSION_MAX_VALUE_IDX) || \
	defined(PDUMP) || defined(SUPPORT_HW_BRN_76176) || defined(SUPPORT_TRUSTED_DEVICE)
#error "rgxstartstop.c would need rgxlayer functions rgxlayer.rs does not provide"
#endif
_Static_assert(PVR_RGX_HAS_RISCV_FW_PROCESSOR && !PVR_RGX_HAS_MIPS && PVR_RGX_VALUE_META < 0,
	       "rgxlayer.rs is written for the RISC-V firmware processor");
