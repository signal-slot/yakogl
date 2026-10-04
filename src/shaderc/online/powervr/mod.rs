// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2024 Imagination Technologies Ltd.
// Derived from Mesa 26.2.1 src/imagination/pco/ (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The PowerVR Rogue USC half of the in-process shader compiler.
//!
//! The instruction set as data ([`isa_tables`], generated from Mesa's pco
//! declarations by `tools/pco-isa/`), a [`decode`]r that reads a USC binary
//! into instruction groups, an [`encode`]r that writes them back, and a
//! [`listing`] disassembler. The gate is in [`roundtrip`]: every one of the
//! 2,042 blobs of the PowerVR offline table decodes into instructions and
//! re-encodes byte for byte.
//!
//! The GPU the compiler targets first is the GC7000 ([`super::vivante`]);
//! this half stays as the reference and its round trip stays in the gate.

pub mod abi;
pub mod alloc;
pub mod assemble;
pub mod blend;
pub mod builtins;
pub mod cf;
pub mod chain;
pub mod decode;
pub mod encode;
pub mod fragment;
pub mod instr;
pub mod isa;
#[rustfmt::skip]
pub mod isa_tables;
pub mod legalise;
pub mod listing;
pub mod operands;
pub mod pack;
pub mod prepare;
pub mod sel;
pub mod select;
pub mod shapes;
pub mod unroll;
pub mod verify;
/* The round trip and the default audit both read the offline table. */
#[cfg(all(test, not(feature = "no-shader-table")))]
mod roundtrip;
