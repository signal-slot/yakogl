// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Scheduling at V3D 7.1, the whole program: split at control flow, each
//! straight-line run packed by [`super::sched71::pack71`], the branches,
//! counted loops and their delay slots placed, and the uniform stream moved
//! with them, as the contract in [`super::vop71`]'s header says. A stub: it
//! refuses every program by name.

use super::alloc71::Allocation71;
use super::sched71::Sched71Error;
use super::vop71::{Instr71, Selected71, Uniform, NOT_YET_71};
use crate::shaderc::online::arena::Arena;

/// The scheduled 7.1 program.
#[derive(Clone, Copy, Debug)]
pub struct Scheduled71<'a> {
    pub instrs: &'a [Instr71],
    /// The uniform stream, in the order the program consumes it.
    pub uniforms: &'a [Uniform],
    /// Every counted loop's back edge, as (branch instruction, target
    /// instruction).
    pub back_edges: &'a [(u32, u32)],
    /// Every forward branch, as (branch instruction, target instruction).
    pub fwd_branches: &'a [(u32, u32)],
}

/// Schedule `sel` with the registers of `al`.
pub fn schedule71<'a>(
    sel: &Selected71<'_>,
    al: &Allocation71<'_>,
    a: &Arena<'a>,
) -> Result<Scheduled71<'a>, Sched71Error> {
    let _ = (sel, al, a);
    Err(Sched71Error::NotYet(NOT_YET_71))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = Selected71 { ops: &[], fragment: true, vregs: 0 };
        let al = Allocation71 { reg: &[], rf_end: 0 };
        assert_eq!(schedule71(&sel, &al, &a).err(), Some(Sched71Error::NotYet(NOT_YET_71)));
    }
}
