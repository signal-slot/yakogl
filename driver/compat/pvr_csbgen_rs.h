/*
 * SPDX-License-Identifier: Apache-2.0
 * SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
 *
 * pvr_csbgen_rs.h -- what the Rust backend (src/pvr/backend.rs) reaches of
 * Mesa's csbgen packers: the generated headers through driver/compat/pvr_mesa.h,
 * without the PDS generators. `yakogl-tools csbgen-bindings` runs bindgen over
 * this file (--wrap-static-fns): src/pvr/csbgen.rs is the Rust side and
 * driver/src/pvr/generated/csbgen_wrap.c the C wrappers around the static
 * inline _pack() functions, which build.rs compiles. Both are committed, and
 * tools/check.sh regenerates them and compares.
 */
#ifndef PVR_CSBGEN_RS_H
#define PVR_CSBGEN_RS_H
#define PVR_MESA_NO_PDS
#include "pvr_mesa.h"
#endif
