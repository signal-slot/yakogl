// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The Broadcom V3D 4.2 (Raspberry Pi 4) half of the in-process shader
//! compiler: the QPU instruction set, decoded and encoded. This is the V3D's
//! only product path (there is no offline table for it); Mesa's v3d
//! compiler, through tools/v3d/v3d_shc, is only the reference it is compared
//! against. Instruction selection is not written yet; it builds on
//! [`isa`]'s model and [`encode::encode_program`].
//!
//! A QPU instruction is one 64-bit word: an add ALU operation and a mul ALU
//! operation issued together with one signal, or a branch. The word is
//! Mesa's: [`encode`] and [`decode`] are `v3d_qpu_instr_pack` and
//! `v3d_qpu_instr_unpack` from Mesa 26.2.1's `qpu_pack.c`, compiled
//! unmodified by build.rs (`third_party/mesa-26.2.1`) and called through
//! [`qpu_ffi`], a pinned mirror of `struct v3d_qpu_instr`. What stays Rust is
//! the model ([`isa`]), which the back end builds, and the rules by which
//! this compiler is stricter than Mesa (listed in [`decode`] and
//! [`encode`]).
//!
//! The contract is "correct or refused": the encoder decodes every word it
//! writes and refuses an instruction that would read back as anything else,
//! and the decoder accepts a word only if Mesa's pack of Mesa's unpack gives
//! it back. The gates:
//!
//! * `qpu_ffi`: the mirror's layout and enum values against the header and
//!   Mesa's name tables.
//! * `roundtrip`: every distinct word of the tools/shaders corpus, decoded
//!   and written back bit for bit, with pinned counts; the strict rules, fed
//!   known-bad words and instructions.
//! * `golden`: Mesa's own ver 42 test vectors, as text and as words.
//! * `mesa_ref`: ~590k enumerated words; which are refused and the listing
//!   of the rest, against Mesa's (tools/v3d/qpu_ref.c), pinned as a digest.
//!
//! Compiled only in a `gpu-v3d` build (and its C with it); the tests only in
//! test builds.

pub mod decode;
pub mod encode;
pub mod isa;
pub mod listing;
pub mod qpu_ffi;
/// A test-only interpreter of encoded QPU programs, validated on Mesa's code
/// (`tests/v3d_sim_mesa.rs`). Built like `eval.rs`: tests and `host-test`.
#[cfg(any(test, feature = "host-test"))]
pub mod sim;

// The back end, first cut (`docs/online-shaderc/v3d-backend.md`, order step
// 2): IR to scalar operations (`prepare`), selection (`select`), allocation
// (`alloc`), scheduling (`sched`), the structural verifier (`verify`), and the
// chain that runs them for a GL program (`chain`).
pub mod alloc;
pub mod chain;
pub mod order;
pub mod prepare;
pub mod sched;
pub mod select;
pub mod verify;

#[cfg(test)]
pub(crate) mod fields;
#[cfg(test)]
mod golden;
#[cfg(test)]
mod mesa_ref;
#[cfg(test)]
mod roundtrip;
