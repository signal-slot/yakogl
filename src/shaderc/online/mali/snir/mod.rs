// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//
//! The shader NIR (docs/notes/mali-shader-nir.md): Mesa's NIR for the Mali
//! back end's front half. Compiled only for tests and `host-test` until a
//! stage class moves to it (the note's section 7.4).

pub mod intrinsics;
pub mod opcodes;
#[cfg(test)]
mod tables_tests;
