// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Selection at V3D 7.1: the 4.2 selection ([`super::select::Selected`]) to
//! 7.1 virtual operations ([`super::vop71::Op71`]), as the contract in
//! [`super::vop71`]'s header says. A stub: it refuses every program by name.

use super::select::Selected;
use super::vop71::{Selected71, NOT_YET_71};
use crate::shaderc::online::arena::{Arena, ArenaError};

/// Why 7.1 selection stopped.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Select71Error {
    /// A 4.2 operation or construct not lowered to 7.1 yet, by name.
    NotYet(&'static str),
    OutOfScratch,
}

impl From<ArenaError> for Select71Error {
    fn from(_: ArenaError) -> Self {
        Select71Error::OutOfScratch
    }
}

/// Lower `sel`'s operations to 7.1 ones.
pub fn select71<'a>(sel: &Selected<'_>, a: &Arena<'a>) -> Result<Selected71<'a>, Select71Error> {
    let _ = (sel, a);
    Err(Select71Error::NotYet(NOT_YET_71))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refuses_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = super::super::vop71::empty_selected(true);
        assert_eq!(select71(&sel, &a).err(), Some(Select71Error::NotYet(NOT_YET_71)));
    }
}
