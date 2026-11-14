// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 chain: [`super::select71`], [`super::alloc71`] and
//! [`super::sched71_cf`] run on one selected shader at 4 threads, then 2,
//! and the words encoded ([`super::codec71::encode71`]) and verified
//! ([`super::verify71::verify71`]), as the contract in [`super::vop71`]'s
//! header says. The GL program ([`compile_program71`]) waits for chain.rs
//! to take the generation as a parameter. A stub: it refuses every program by
//! name.

use super::alloc71::Alloc71Error;
use super::chain::{Prim, Target, V3dError};
use super::encode::EncodeError;
use super::sched71::Sched71Error;
use super::select::Selected;
use super::select71::Select71Error;
use super::verify71::VerifyError;
use super::vop71::{Instr71, Uniform, NOT_YET_71};
use crate::shaderc::online::arena::{Arena, ArenaError};
use crate::shaderc::online::ir;

/// Why a program was not compiled at 7.1. One variant per stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum V3d71Error {
    /// A construct or stage not compiled at 7.1 yet, by name.
    NotYet(&'static str),
    /// The stages shared with 4.2 (unrolling, prepare, 4.2 selection).
    Chain(V3dError),
    Select(Select71Error),
    Alloc(Alloc71Error),
    Sched(Sched71Error),
    /// Instruction `.0` does not encode.
    Encode(usize, EncodeError),
    Verify(VerifyError),
    OutOfScratch,
}

impl From<ArenaError> for V3d71Error {
    fn from(_: ArenaError) -> Self {
        V3d71Error::OutOfScratch
    }
}
impl From<V3dError> for V3d71Error {
    fn from(e: V3dError) -> Self {
        V3d71Error::Chain(e)
    }
}
impl From<Select71Error> for V3d71Error {
    fn from(e: Select71Error) -> Self {
        V3d71Error::Select(e)
    }
}
impl From<Alloc71Error> for V3d71Error {
    fn from(e: Alloc71Error) -> Self {
        V3d71Error::Alloc(e)
    }
}
impl From<Sched71Error> for V3d71Error {
    fn from(e: Sched71Error) -> Self {
        V3d71Error::Sched(e)
    }
}

/// One compiled 7.1 QPU program.
#[derive(Clone, Copy, Debug)]
pub struct Code71<'a> {
    pub words: &'a [u64],
    /// The instructions the words encode.
    pub instrs: &'a [Instr71],
    /// The uniform stream, `(contents, data)` with Mesa's quniform_contents.
    pub uniforms: &'a [Uniform],
    /// Every counted loop's back edge (branch, target instruction).
    pub back_edges: &'a [(u32, u32)],
    /// Every forward branch (branch, target instruction).
    pub fwd_branches: &'a [(u32, u32)],
}

/// One compiled shader stage.
#[derive(Clone, Copy, Debug)]
pub struct Stage71<'a> {
    pub code: Code71<'a>,
    pub threads: u32,
    pub single_seg: bool,
}

/// A compiled GL program at 7.1. The shader-record fields beyond the code
/// (chain.rs `FsProg`, `VsProg`) come with the chain hook.
#[derive(Clone, Copy, Debug)]
pub struct V3dProgram71<'a> {
    pub fs: Stage71<'a>,
    pub vs: Stage71<'a>,
    pub cs: Stage71<'a>,
}

/// Compile one selected shader at 7.1.
pub fn compile_stage71<'a>(sel: &Selected<'_>, a: &Arena<'a>) -> Result<Stage71<'a>, V3d71Error> {
    let _ = (sel, a);
    Err(V3d71Error::NotYet(NOT_YET_71))
}

/// Compile the GL program `vs` + `fs` for `prim` on `target` at 7.1.
pub fn compile_program71<'a>(
    vs: &'a ir::Module<'a>,
    fs: &'a ir::Module<'a>,
    prim: Prim,
    target: Target,
    a: &'a Arena<'a>,
) -> Result<V3dProgram71<'a>, V3d71Error> {
    let _ = (vs, fs, prim, target, a);
    Err(V3d71Error::NotYet(NOT_YET_71))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = super::super::vop71::empty_selected(false);
        assert_eq!(compile_stage71(&sel, &a).err(), Some(V3d71Error::NotYet(NOT_YET_71)));
    }
}
