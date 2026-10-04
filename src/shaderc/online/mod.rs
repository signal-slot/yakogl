// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The in-process ("online") shader compiler: a GLSL ES 1.00 front end that
//! runs on the target.
//!
//! Today a shader reaches the GPU one of two ways: [`super::offline`] looks
//! its text up by SHA-256 in a table built on a host, and `super::remote`
//! (feature `shaderc-remote`, a host build) asks a server to run
//! `glslangValidator` and `pco_shc` over TCP. Neither can compile a shader an
//! application writes at run time on a Zephyr or bare-metal image, which is
//! what dEQP-GLES2 does on every case and what any real ES 2.0 application
//! may do. This module is the third way: the compiler itself, in the library.
//!
//! # What lives here
//!
//! * [`arena`] -- the memory model. There is no allocator: the caller lends
//!   the compiler a `&mut [u8]` and every node lives in it.
//! * [`store`] -- the two buffers the back end lends the compiler: the
//!   per-compile scratch, and the one a compiled shader's USC and reflection
//!   live in, which outlives the compile because the `ShcShader` the GL core
//!   keeps points into it, and is freed per shader when the GL core releases
//!   its last copy.
//! * [`lexer`] -- GLSL ES 1.00 tokens.
//! * [`pp`] -- the preprocessor: directives, conditionals, macros.
//! * [`ast`] -- the syntax tree the parser builds.
//! * [`parser`] -- recursive descent to that tree.
//! * [`sema`] -- the ES 1.00 type system over that tree.
//! * [`lower`] -- the tree to [`ir::Module`], and `compile_to_ir`: the whole
//!   front end behind one call.
//! * [`reflect`] -- the uniform, attribute and varying rows the GL core
//!   reads, in the offline table's own field shapes.
//! * [`ir`] -- the contract between the front end and the code generator.
//! * [`select`] -- instruction selection: the IR to instructions in virtual
//!   registers (that GPU only).
//! * [`abi`] -- the `ShcShader` assembly for the GC7000 (that GPU only).
//! * [`codegen`] -- GLSL text to a `ShcShader`, and why a compile failed.
//! * [`backend`] -- `struct shc_backend`: what the GL core calls, the
//!   compiled-shader cache, and the selection that makes this `g_shaderc`.
//!
//! Still to come: the lowering from the syntax tree to the IR the code
//! generator consumes, and the built-in function library it expands.
//! `docs/online-shaderc/frontend.md`, `ir.md`, `design.md` and `backend.md`
//! are the design records.
//!
//! # Diagnostics
//!
//! dEQP checks that an invalid shader *fails to compile*, and reads
//! `glGetShaderInfoLog`. The front end therefore reports every rejection as
//! an [`Error`], which formats as glslang's own shape -- `0:5: 'x' : message`
//! -- so an application that scrapes the log sees what it saw before.
//!
//! # The GPU half
//!
//! The same module owns the instruction set the front end will be compiled
//! down to, because a compiler that cannot write the machine's instructions
//! is not a compiler:
//!
//! * [`vivante`] -- the Vivante GC7000 instruction set, the first target:
//!   a fixed 128-bit instruction, decoded and re-encoded.
//! * [`powervr`] -- the PowerVR Rogue USC instruction set, the reference:
//!   variable-length instruction groups, phases, and the padding rules.
//! * [`v3d`] -- the Broadcom V3D 4.2 QPU instruction set: a 64-bit word, an
//!   add and a mul operation issued together. The V3D's only product path;
//!   its words are packed and unpacked by Mesa's own `qpu_pack.c` (through
//!   FFI, compiled by build.rs), gated by the Mesa v3d corpus and Mesa's
//!   answer on every field combination. Only in a `gpu-v3d` build.
//!
//! Each carries its own generated tables and its own gate, and each gate is a
//! round trip over that GPU's offline blob table: decode every blob into
//! instructions, write them back, and require the bytes to be identical.
//!
//! `docs/online-shaderc/design.md` is the design record for this half and
//! `docs/online-shaderc/isa-coverage.md` what the PowerVR table's shaders
//! actually use.
//!

pub mod powervr;
/// The Broadcom V3D 4.2 compiler: the QPU instruction set (through Mesa's
/// `qpu_pack.c`) and the back end. Only in a `gpu-v3d` build: the C it links
/// stays out of the PowerVR and GC7000 images.
#[cfg(feature = "gpu-v3d")]
pub mod v3d;
pub mod vivante;

/// The `ShcShader` assembly for the GC7000 (`docs/online-shaderc/gc7000-abi.md`).
#[cfg(feature = "gpu-vivante")]
pub mod abi;
pub mod arena;
pub mod ast;
/// `struct shc_backend` for this compiler: the GL core's three entry points,
/// the compiled-shader cache, and the selection that makes it `g_shaderc`.
pub mod backend;
/// GLSL built-in functions to IR opcodes -- the language half. The machine
/// half, opcodes to GC7000 instructions, is `vivante::builtins`.
pub mod builtin_call;
/// The pipeline from GLSL text to a `ShcShader`, and why a compile failed.
pub mod codegen;
/// The reference evaluator: the lowering's tests, `yakogl-tools online-eval`,
/// and the host tests that compare what the back end emits against what the
/// IR means (`tests/backend_differential.rs`). `host-test` only -- never in a
/// product.
#[cfg(any(test, feature = "host-test"))]
pub mod eval;
/// Float operations on constants, folded exactly (the V3D's `prepare` and the
/// USC's `select`).
pub(crate) mod fold;
/// The contract between the front end and the code generator.
pub mod ir;
pub mod lexer;
/// Sampler-state variants in the IR: the look-ups whose code depends on the
/// bound sampler's filters, and the c = 0.5 remap of an explicit level.
pub mod lodkey;
pub mod loop_count;
pub mod lower;
pub mod parser;
pub mod pp;
pub mod reflect;
/// The two buffers the back end lends the compiler.
/// Instruction selection: the IR to GC7000 instructions in virtual registers
/// (the code generator's front half).
#[cfg(feature = "gpu-vivante")]
pub mod select;
pub mod sema;
pub mod store;
/// The texture model `eval` and the V3D interpreter share (dEQP's rules,
/// from v3d-tex's C ports in tools/v3d).
#[cfg(any(test, feature = "host-test"))]
pub mod texref;
/// The `ShcShader` assembly for the V3D: a stage's reflection, and the
/// linked program's three QPU programs (`docs/online-shaderc/v3d-backend.md`).
#[cfg(feature = "gpu-v3d")]
pub mod v3d_abi;

pub use backend::{select_once, SHC_ONLINE};

/// The one triangle sample's shader texts through this compiler (the gate
/// that they are GLSL ES 1.00 it builds).
#[cfg(test)]
mod triangle_sample_tests;

/// One rejection: where, what token, and why.
///
/// Heap-free by construction: `msg` is a `&'static str` chosen from a fixed
/// set and `tok` borrows the source, so building a diagnostic allocates
/// nothing and cannot itself fail.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Error<'a> {
    /// 1-based line in the text the application passed to `glShaderSource`.
    pub line: u32,
    /// The offending token's spelling, or "" when the error is not at one
    /// (end of file, a whole directive).
    pub tok: &'a str,
    /// What is wrong, in the info log's words.
    pub msg: &'static str,
}

impl<'a> Error<'a> {
    pub fn new(line: u32, tok: &'a str, msg: &'static str) -> Error<'a> {
        Error { line, tok, msg }
    }
}

/// The front end's result type.
pub type R<'a, T> = Result<T, Error<'a>>;

impl From<arena::ArenaError> for Error<'_> {
    fn from(_: arena::ArenaError) -> Self {
        /* The scratch buffer is the caller's to size, so this is a message
         * about the compile as a whole and belongs at line 0 with no token.
         * It is a distinct message from any language error: "your shader is
         * wrong" and "my buffer is too small" must never read the same. */
        Error { line: 0, tok: "", msg: "compiler scratch buffer exhausted" }
    }
}
