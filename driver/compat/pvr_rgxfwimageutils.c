/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_rgxfwimageutils.c -- compiles the TI/Img Rogue DDK 25.2's firmware
 * image loader (third_party/ti-img-rogue-ddk-25.2/services/server/devices/
 * rgxfwimageutils.c, vendored unmodified, Dual MIT/GPLv2, used under MIT:
 * RGXGetFWImageAllocSize, RGXProcessFWImage and the section queries) in the
 * environment of driver/compat/pvr_ddk.h, as pvr_rgxstartstop.c does.
 *
 * Two of its functions leave a parameter unused (FindMMUSegment's hPrivate,
 * RGXFWConfigureSegMMU's psFWRoDataDevVAddrBase); that warning is off for this
 * file only, so a build with -Wextra does not stop on upstream code.
 */
#include "pvr_ddk.h"

#pragma GCC diagnostic push
#pragma GCC diagnostic ignored "-Wunused-parameter"
#include "rgxfwimageutils.c"
#pragma GCC diagnostic pop
