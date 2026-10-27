// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Mali-G610's descriptors and command-stream instructions, packed.
//!
//! | module | what it is |
//! | --- | --- |
//! | [`v10`] | GENERATED from Mesa's v10.xml by tools/mali/gen_pack_rs.py |
//! | [`pack`] | the bit packer the generated code calls, and its refusals |
//!
//! The tests compare every struct with Mesa's own packers on the same values
//! (`mesa_tests`, over the generated `v10_cases`), pin each refusal and its
//! message (`tests`), and pack descriptors decoded from a board trace back
//! to the trace's bytes (`board_tests`).

pub mod pack;
pub mod v10;

#[cfg(test)]
mod board_tests;
#[cfg(test)]
mod mesa_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod v10_cases;
