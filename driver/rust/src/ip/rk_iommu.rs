// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Rockchip IOMMU in front of a display controller, turned off: paging
//! disabled and checked, so the block behind it fetches physical addresses.
//! Nothing here maps a page. The RK3588's VOP2 has one such IOMMU with two
//! register sets, `vop_mmu` at 0xfdd97e00 and 0xfdd97f00, 0x100 each, in
//! PD_VOP, clocked by ACLK_VOP and HCLK_VOP (Linux v6.18
//! arch/arm64/boot/dts/rockchip/rk3588-base.dtsi:1428-1437). Zephyr does not
//! use it (driver/docs/nanopc-t6-display.md, section 1, "The IOMMU"): a
//! Linux that a crash kexec left behind scans through page tables in its own,
//! now dead, memory, and the cold path must not depend on what the boot
//! loader left.
//!
//! Facts from Linux v6.18 drivers/iommu/rockchip-iommu.c (GPL-2.0: facts and
//! file:line only, nothing transcribed):
//!
//! 1. Registers (:33-42): `DTE_ADDR` 0x00, `STATUS` 0x04, `COMMAND` 0x08,
//!    `INT_MASK` 0x1c. `STATUS` (:51-57): bit 0 paging enabled, bit 2 stall
//!    active. `COMMAND` values (:60-66): 1 disable paging, 2 enable stall,
//!    3 disable stall.
//! 2. Linux's order to turn one IOMMU off (:911-925): stall, disable paging,
//!    `INT_MASK` = 0 and `DTE_ADDR` = 0 on every register set, then stall
//!    off. Each command goes to every register set at once and is followed
//!    by a poll of all of them (:347-353, :411-497).
//! 3. A stall is only requested while paging is enabled (:419-421: it cannot
//!    become active otherwise), and only lifted when it is active (:441-442).
//! 4. Polls: every 100 us, at most 1000 us (:46, :48).
//!
//! What this does differently: Linux treats the register sets as one (paging
//! "enabled" only when every set has it, :388-398), so a set that paged while
//! another did not would be left paging. Here each set is looked at on its
//! own: a set with paging enabled is stalled and switched off, a set without
//! it gets only the two register writes, and the result is checked on every
//! set. [`paging_off`] fails by name when a set keeps paging.

use crate::mmio::Mmio;

/// Register offsets (fact 1).
pub const DTE_ADDR: usize = 0x00;
pub const STATUS: usize = 0x04;
pub const COMMAND: usize = 0x08;
pub const INT_MASK: usize = 0x1c;

/// `STATUS` bits (fact 1).
pub const STATUS_PAGING_ENABLED: u32 = 1 << 0;
pub const STATUS_STALL_ACTIVE: u32 = 1 << 2;

/// `COMMAND` values (fact 1).
pub const CMD_DISABLE_PAGING: u32 = 1;
pub const CMD_ENABLE_STALL: u32 = 2;
pub const CMD_DISABLE_STALL: u32 = 3;

/// Poll step and bound (fact 4).
pub const POLL_STEP_US: u32 = 100;
pub const POLL_BUDGET_US: u32 = 1000;

/// The most register sets one IOMMU has here (the VOP2's has two).
pub const MAX_SETS: usize = 2;

/// Why [`paging_off`] failed. The value crosses to C, one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Refusal {
    /// No register set, or more than [`MAX_SETS`].
    BadArgument = 1,
    /// A set with paging enabled did not report its stall active.
    StallTimeout = 2,
    /// A set still reports paging enabled after the disable command.
    PagingStuck = 3,
    /// A stalled set did not leave the stall.
    UnstallTimeout = 4,
}

/// A failure, with the set it is about and that set's last `STATUS`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    /// Index into the `bases` given.
    pub set: usize,
    pub status: u32,
}

/// What [`paging_off`] found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Off {
    /// `STATUS` of each set before anything was written (0 past the sets given).
    pub status_found: [u32; MAX_SETS],
    /// `STATUS` of each set at the end.
    pub status_now: [u32; MAX_SETS],
}

impl Off {
    /// Some set was paging when [`paging_off`] started.
    pub fn was_paging(&self) -> bool {
        self.status_found.iter().any(|s| s & STATUS_PAGING_ENABLED != 0)
    }
}

/// Paging enabled in the register set at `base`.
pub fn paging_enabled(m: &impl Mmio, base: usize) -> bool {
    m.read32(base + STATUS) & STATUS_PAGING_ENABLED != 0
}

/// Poll every set in `sel` until `done(STATUS)` holds for all of them, at
/// most [`POLL_BUDGET_US`]. Returns the first set that did not get there.
fn poll_all(
    m: &impl Mmio,
    bases: &[usize],
    sel: &[bool; MAX_SETS],
    done: impl Fn(u32) -> bool,
    delay_us: &mut impl FnMut(u32),
) -> Result<(), (usize, u32)> {
    let mut waited = 0;
    loop {
        let late = bases.iter().enumerate().filter(|(i, _)| sel[*i]).find_map(|(i, b)| {
            let s = m.read32(b + STATUS);
            (!done(s)).then_some((i, s))
        });
        match late {
            None => return Ok(()),
            Some(l) if waited >= POLL_BUDGET_US => return Err(l),
            Some(_) => {
                delay_us(POLL_STEP_US);
                waited += POLL_STEP_US;
            }
        }
    }
}

/// Turn paging off in every register set of one IOMMU (`bases`, the mapped
/// sets) in Linux's order (fact 2), per set as the module text says, and
/// check that no set pages afterwards. A set that was not paging gets only
/// `INT_MASK` = 0 and `DTE_ADDR` = 0. The block behind the IOMMU must be
/// idle or able to stall: while a set is stalled its master waits.
///
/// Every step runs even when an earlier poll timed out, as Linux's does
/// (:915: errors are ignored while disabling). The result: `PagingStuck`
/// when a set still pages at the end; otherwise the first stall or unstall
/// time-out (paging is then off on every set, checked); otherwise `Ok`.
pub fn paging_off(m: &impl Mmio, bases: &[usize], delay_us: &mut impl FnMut(u32)) -> Result<Off, Refused> {
    if bases.is_empty() || bases.len() > MAX_SETS {
        return Err(Refused { why: Refusal::BadArgument, set: 0, status: 0 });
    }
    let mut status_found = [0; MAX_SETS];
    let mut paging = [false; MAX_SETS];
    for (i, b) in bases.iter().enumerate() {
        status_found[i] = m.read32(b + STATUS);
        paging[i] = status_found[i] & STATUS_PAGING_ENABLED != 0;
    }
    // Like Linux (:915), a failed step does not stop the next one: the
    // first failure is kept and reported once everything was done.
    let mut first: Option<Refused> = None;
    let mut keep = |why, r: Result<(), (usize, u32)>| {
        if let (Err((set, status)), None) = (r, first) {
            first = Some(Refused { why, set, status });
        }
    };

    // Stall each paging set that is not stalled already (fact 3).
    let mut stall = [false; MAX_SETS];
    for (i, b) in bases.iter().enumerate() {
        stall[i] = paging[i] && status_found[i] & STATUS_STALL_ACTIVE == 0;
        if stall[i] {
            m.write32(b + COMMAND, CMD_ENABLE_STALL);
        }
    }
    keep(Refusal::StallTimeout, poll_all(m, bases, &stall, |s| s & STATUS_STALL_ACTIVE != 0, delay_us));

    for (i, b) in bases.iter().enumerate() {
        if paging[i] {
            m.write32(b + COMMAND, CMD_DISABLE_PAGING);
        }
    }
    // A set that keeps paging is reported by the check at the end.
    let _ = poll_all(m, bases, &paging, |s| s & STATUS_PAGING_ENABLED == 0, delay_us);

    for b in bases {
        m.write32(b + INT_MASK, 0);
        m.write32(b + DTE_ADDR, 0);
    }

    // Lift every stall that is active now, ours or one found (fact 3).
    let mut stalled = [false; MAX_SETS];
    for (i, b) in bases.iter().enumerate() {
        stalled[i] = m.read32(b + STATUS) & STATUS_STALL_ACTIVE != 0;
        if stalled[i] {
            m.write32(b + COMMAND, CMD_DISABLE_STALL);
        }
    }
    keep(Refusal::UnstallTimeout, poll_all(m, bases, &stalled, |s| s & STATUS_STALL_ACTIVE == 0, delay_us));

    // The check: no set pages, whatever the commands did.
    let mut status_now = [0; MAX_SETS];
    for (i, b) in bases.iter().enumerate() {
        status_now[i] = m.read32(b + STATUS);
        if status_now[i] & STATUS_PAGING_ENABLED != 0 {
            return Err(Refused { why: Refusal::PagingStuck, set: i, status: status_now[i] });
        }
    }
    match first {
        Some(r) => Err(r),
        None => Ok(Off { status_found, status_now }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    /// The VOP2's two register sets (rk3588-base.dtsi:1430).
    const SETS: [usize; 2] = [0xfdd9_7e00, 0xfdd9_7f00];

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Op {
        W(usize, u32),
        Delay(u32),
    }

    #[derive(Default, Clone, Copy)]
    struct Set {
        status: u32,
        dte: u32,
        int_mask: u32,
    }

    /// Two register sets as rockchip-iommu.c describes them: a stall becomes
    /// active only while paging (fact 3), the commands act at once. Offsets
    /// are restated here, not taken from the module.
    #[derive(Default)]
    struct Mmu {
        sets: RefCell<[Set; 2]>,
        log: RefCell<Vec<Op>>,
        /// Disable paging is ignored.
        paging_stuck: bool,
        /// Enable stall is ignored.
        stall_never: bool,
        /// Disable stall is ignored.
        stall_stuck: bool,
    }

    impl Mmu {
        /// Both sets paging through Linux's tables, as a crash kexec leaves them.
        fn crash_kexec() -> Mmu {
            let m = Mmu::default();
            for s in m.sets.borrow_mut().iter_mut() {
                *s = Set { status: 1, dte: 0x0123_4000, int_mask: 3 };
            }
            m
        }
        fn idx(addr: usize) -> (usize, usize) {
            let i = SETS.iter().position(|b| addr & !0xff == *b).expect("outside the vop_mmu sets");
            (i, addr & 0xff)
        }
        fn writes(&self) -> Vec<(usize, usize, u32)> {
            self.log
                .borrow()
                .iter()
                .filter_map(|o| match o {
                    Op::W(a, v) => Some((Self::idx(*a).0, Self::idx(*a).1, *v)),
                    Op::Delay(_) => None,
                })
                .collect()
        }
        fn delay(&self, us: u32) {
            self.log.borrow_mut().push(Op::Delay(us));
        }
    }

    impl Mmio for Mmu {
        fn read32(&self, addr: usize) -> u32 {
            let (i, off) = Self::idx(addr);
            let s = self.sets.borrow()[i];
            match off {
                0x00 => s.dte,
                0x04 => s.status,
                0x1c => s.int_mask,
                _ => panic!("read of vop_mmu offset {off:#x}"),
            }
        }
        fn write32(&self, addr: usize, val: u32) {
            self.log.borrow_mut().push(Op::W(addr, val));
            let (i, off) = Self::idx(addr);
            let mut sets = self.sets.borrow_mut();
            let s = &mut sets[i];
            match (off, val) {
                (0x00, v) => s.dte = v,
                (0x1c, v) => s.int_mask = v,
                (0x08, 1) if !self.paging_stuck => s.status &= !1,
                (0x08, 2) if !self.stall_never && s.status & 1 != 0 => s.status |= 4,
                (0x08, 3) if !self.stall_stuck => s.status &= !4,
                (0x08, 1..=3) => {}
                _ => panic!("write {val:#x} to vop_mmu offset {off:#x}"),
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[test]
    fn the_layout_is_rockchip_iommu_c() {
        assert_eq!((DTE_ADDR, STATUS, COMMAND, INT_MASK), (0x00, 0x04, 0x08, 0x1c));
        assert_eq!((STATUS_PAGING_ENABLED, STATUS_STALL_ACTIVE), (1, 4));
        assert_eq!((CMD_DISABLE_PAGING, CMD_ENABLE_STALL, CMD_DISABLE_STALL), (1, 2, 3));
        assert_eq!((POLL_STEP_US, POLL_BUDGET_US), (100, 1000));
    }

    #[test]
    fn paging_left_on_by_linux_is_stalled_switched_off_cleared_and_unstalled_in_linuxs_order() {
        let m = Mmu::crash_kexec();
        let off = paging_off(&m, &SETS, &mut |us| m.delay(us)).unwrap();
        assert_eq!(
            m.writes(),
            vec![
                (0, 0x08, 2),
                (1, 0x08, 2),
                (0, 0x08, 1),
                (1, 0x08, 1),
                (0, 0x1c, 0),
                (0, 0x00, 0),
                (1, 0x1c, 0),
                (1, 0x00, 0),
                (0, 0x08, 3),
                (1, 0x08, 3),
            ]
        );
        assert!(off.was_paging());
        assert_eq!(off.status_now, [0, 0]);
        for s in m.sets.borrow().iter() {
            assert_eq!((s.status, s.dte, s.int_mask), (0, 0, 0));
        }
        assert!(!paging_enabled(&m, SETS[0]) && !paging_enabled(&m, SETS[1]));
    }

    #[test]
    fn an_iommu_already_off_gets_only_the_two_register_writes() {
        let m = Mmu::default();
        let off = paging_off(&m, &SETS, &mut |us| m.delay(us)).unwrap();
        assert_eq!(m.writes(), vec![(0, 0x1c, 0), (0, 0x00, 0), (1, 0x1c, 0), (1, 0x00, 0)]);
        assert!(!off.was_paging());
    }

    /// One set paging, the other not: each set on its own (the module text's
    /// difference from Linux), so the paging one is still switched off.
    #[test]
    fn one_set_paging_is_switched_off_on_its_own() {
        let m = Mmu::default();
        m.sets.borrow_mut()[1].status = 1;
        paging_off(&m, &SETS, &mut |us| m.delay(us)).unwrap();
        let cmds: Vec<_> = m.writes().into_iter().filter(|w| w.1 == 0x08).collect();
        assert_eq!(cmds, vec![(1, 0x08, 2), (1, 0x08, 1), (1, 0x08, 3)]);
        assert_eq!(m.sets.borrow()[1].status, 0);
    }

    #[test]
    fn a_stuck_paging_bit_is_a_named_error_after_the_bounded_poll() {
        let m = Mmu { paging_stuck: true, ..Mmu::crash_kexec() };
        let r = paging_off(&m, &SETS, &mut |us| m.delay(us)).unwrap_err();
        assert_eq!(r, Refused { why: Refusal::PagingStuck, set: 0, status: 0x1 });
        let waited: u32 =
            m.log.borrow().iter().filter_map(|o| if let Op::Delay(d) = o { Some(*d) } else { None }).sum();
        assert_eq!(waited, POLL_BUDGET_US);
    }

    #[test]
    fn a_stall_that_never_comes_and_one_that_never_goes_are_named() {
        let m = Mmu { stall_never: true, ..Mmu::crash_kexec() };
        let r = paging_off(&m, &SETS, &mut |us| m.delay(us)).unwrap_err();
        assert_eq!((r.why, r.set, r.status), (Refusal::StallTimeout, 0, 1));
        // every step was still taken: paging is off on both sets
        assert!(m.sets.borrow().iter().all(|s| s.status & STATUS_PAGING_ENABLED == 0));

        let m = Mmu { stall_stuck: true, ..Mmu::crash_kexec() };
        let r = paging_off(&m, &SETS, &mut |us| m.delay(us)).unwrap_err();
        assert_eq!((r.why, r.set, r.status), (Refusal::UnstallTimeout, 0, 4));
    }

    #[test]
    fn no_set_or_too_many_is_refused_before_any_access() {
        let m = Mmu::default();
        assert_eq!(paging_off(&m, &[], &mut |_| {}).unwrap_err().why, Refusal::BadArgument);
        assert_eq!(paging_off(&m, &[SETS[0]; 3], &mut |_| {}).unwrap_err().why, Refusal::BadArgument);
        assert!(m.log.borrow().is_empty());
    }

    /// Review S2: Linux goes on with the next steps after a stall time-out
    /// (rockchip-iommu.c:915-923).
    #[test]
    fn a_stall_time_out_does_not_stop_the_paging_off() {
        let m = Mmu { stall_never: true, ..Mmu::crash_kexec() };
        let _ = paging_off(&m, &SETS, &mut |us| m.delay(us));
        for s in m.sets.borrow().iter() {
            assert_eq!((s.status & STATUS_PAGING_ENABLED, s.dte, s.int_mask), (0, 0, 0));
        }
    }
}
