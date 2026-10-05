// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000 half of the in-process shader compiler: the Vivante instruction
//! set, decoded and re-encoded.
//!
//! The i.MX8MP's GC7000UL is the first target of the online compiler. Its
//! instruction set is far simpler than the PowerVR's: an instruction is a
//! fixed 128 bits, there is no instruction group, no phase model and no
//! padding, and every operation pins its own opcode. So the whole of M1 here
//! is: which operation is this, what are its fields, and does writing them
//! back give the same bits.
//!
//! The gate is the same as the PowerVR one and is in [`roundtrip`]: every
//! shader the driver ships - the GC7000 offline table
//! (`tests/reference/vivante_blobs.rs`) and the backend's two internal clear
//! shaders - decodes into instructions and re-encodes byte for byte.
//!
//! [`build`] is the other direction, the one a code generator needs: an
//! instruction made from an operation name and operands rather than read out
//! of a binary. [`alloc`] is the layer above it: virtual registers in, built
//! instructions and a temporary count out.

pub mod alloc;
pub mod build;
pub mod builtins;
pub mod decode;
pub mod encode;
pub mod isa;
#[rustfmt::skip]
pub mod isa_tables;
pub mod listing;
/* The round trip and the default audit both read the offline table. */
#[cfg(all(test, not(feature = "no-shader-table")))]
mod program_end;
mod roundtrip;
