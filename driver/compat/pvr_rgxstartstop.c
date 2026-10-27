/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_rgxstartstop.c -- compiles the TI/Img Rogue DDK 25.2's RGXStart and
 * RGXStop (third_party/ti-img-rogue-ddk-25.2/services/server/devices/rogue/
 * rgxstartstop.c, vendored unmodified, Dual MIT/GPLv2, used under MIT) in the
 * environment of driver/compat/pvr_ddk.h: the AM62P build configuration
 * (config_kernel.h) and TEE_DDK, the DDK's own switch for building its
 * kernel-side code outside a Linux kernel. Nothing here but that environment;
 * the rgxlayer.h functions it calls are driver/rust/src/gpu/pvr/rgxlayer.rs
 * (and driver/compat/pvr_rgxlayer_log.c for the two variadic ones).
 */
#include "pvr_ddk.h"

#include "rogue/rgxstartstop.c"
