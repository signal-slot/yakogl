/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_ddk_check.c -- the firmware-interface numbers the driver depends on
 * (driver/tools/offsetof/ddk_abi_pins.h), asserted in the build that runs on
 * the board: the same vendored DDK headers and configuration as the host check
 * (driver/tools/offsetof/ddk_abi_check.c, a tools/check.sh row), compiled by
 * the target compiler in the TEE_DDK mode driver/compat/pvr_ddk.h selects.
 * No code: a layout that moved stops the build.
 */
#include "pvr_ddk.h"

#define PIN(value, ddk) _Static_assert((ddk) == (value), "DDK: " #ddk " != " #value);
#define MISMATCH(ddk_value, ddk, fork_value, fork, why) PIN(ddk_value, ddk)
#include "../tools/offsetof/ddk_abi_pins.h"
#undef PIN
#undef MISMATCH
