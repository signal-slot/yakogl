// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Scheduling at V3D 7.1, straight-line code: one run of [`Op71`]s with no
//! control flow packed into [`Instr71`] words, thread switches and their
//! delay slots included, as the contract in [`super::vop71`]'s header says.
//! Branches are [`super::sched71_cf`]'s. A stub: it refuses every run by
//! name.

use super::alloc71::Allocation71;
use super::vop71::{Instr71, Op71, Uniform, NOT_YET_71};
use crate::shaderc::online::arena::{Arena, ArenaError};

/// Why 7.1 scheduling stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Sched71Error {
    /// Not scheduled at 7.1 yet, by name.
    NotYet(&'static str),
    /// Operation `.0` is control flow, which [`pack71`] does not take.
    ControlFlow(usize),
    OutOfScratch,
}

impl From<ArenaError> for Sched71Error {
    fn from(_: ArenaError) -> Self {
        Sched71Error::OutOfScratch
    }
}

/// One packed straight-line run.
#[derive(Clone, Copy, Debug)]
pub struct Packed71<'a> {
    pub instrs: &'a [Instr71],
    /// The uniforms the run consumes, in order.
    pub uniforms: &'a [Uniform],
}

/// Pack `ops`, one straight-line run, into words.
pub fn pack71<'a>(ops: &[Op71], al: &Allocation71<'_>, a: &Arena<'a>) -> Result<Packed71<'a>, Sched71Error> {
    let _ = (al, a);
    if let Some(i) = ops.iter().position(Op71::is_control_flow) {
        return Err(Sched71Error::ControlFlow(i));
    }
    Err(Sched71Error::NotYet(NOT_YET_71))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::v3d::vop71::Label;

    #[test]
    fn refuses_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let al = Allocation71 { reg: &[], rf_end: 0 };
        assert_eq!(pack71(&[Op71::Thrsw], &al, &a).err(), Some(Sched71Error::NotYet(NOT_YET_71)));
        assert_eq!(pack71(&[Op71::Label(Label(0))], &al, &a).err(), Some(Sched71Error::ControlFlow(0)));
    }
}
