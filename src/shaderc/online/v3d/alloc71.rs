// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Register allocation at V3D 7.1: one register-file register per virtual
//! register, no accumulators, rf0 kept for the implicit writes of `ldvary`
//! and `ldunif`, as the contract in [`super::vop71`]'s header says. A stub:
//! it refuses every program by name.

use super::vop71::{Selected71, NOT_YET_71};
use crate::shaderc::online::arena::{Arena, ArenaError};

/// A register-file register, rf0..rf63.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rf(pub u8);

/// Why 7.1 allocation stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Alloc71Error {
    /// Not allocated at 7.1 yet, by name.
    NotYet(&'static str),
    /// More values are live at one point than there are registers at the
    /// thread count asked for (no spilling).
    DoesNotFit,
    /// A thread count the V3D does not run (not 1, 2 or 4).
    Threads,
    OutOfScratch,
}

impl From<ArenaError> for Alloc71Error {
    fn from(_: ArenaError) -> Self {
        Alloc71Error::OutOfScratch
    }
}

/// Where each virtual register lives at 7.1.
#[derive(Clone, Copy, Debug)]
pub struct Allocation71<'a> {
    /// Indexed by virtual register.
    pub reg: &'a [Rf],
    /// One past the highest register used.
    pub rf_end: u8,
}

/// Allocate `sel`'s virtual registers for `threads` threads.
pub fn allocate71<'a>(sel: &Selected71<'_>, threads: u32, a: &Arena<'a>) -> Result<Allocation71<'a>, Alloc71Error> {
    let _ = (sel, threads, a);
    Err(Alloc71Error::NotYet(NOT_YET_71))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = Selected71 { ops: &[], fragment: true, vregs: 0 };
        assert_eq!(allocate71(&sel, 4, &a).err(), Some(Alloc71Error::NotYet(NOT_YET_71)));
    }
}
