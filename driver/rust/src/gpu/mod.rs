// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GPU kernel-side code.
//!
//! - [`pvr`]: the PowerVR BXS-4-64 (AM62P). The TI/Img Rogue DDK 25.2's
//!   `rgxstartstop.c` and `rgxfwimageutils.c` are compiled unmodified in the
//!   Zephyr build; they call the hardware-abstraction functions of the DDK's
//!   `rgxlayer.h`, which this crate implements (feature `pvr`).
//! - [`mali`]: the Mali-G610 (RK3588): identity, soft reset and the L2,
//!   from panthor under its MIT option (feature `mali`).
//! - [`vivante`]: the Vivante GC7000UL (i.MX 8M Plus): power, identify,
//!   reset, MMU v2, the front-end ring and the interrupt, original code
//!   written from the rnndb headers, NXP's galcore driver (MIT) and the
//!   reference manual (feature `vivante`).

#[cfg(any(feature = "mali", test))]
pub mod mali;

#[cfg(any(feature = "pvr", test))]
pub mod pvr;

#[cfg(any(feature = "vivante", test))]
pub mod vivante;
