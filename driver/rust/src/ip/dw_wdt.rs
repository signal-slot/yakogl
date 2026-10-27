// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Synopsys DesignWare APB watchdog (DW_apb_wdt; the RK3588's WDT0 at
//! 0xfeaf0000): arm it for one of its fixed periods, whatever state the
//! previous stage left it in, read it back, and restart (feed) it.
//!
//! Why: a Zephyr image that Linux kexecs, or that U-Boot starts, must not
//! depend on an earlier stage having armed the watchdog. Armbian leaves it
//! stopped (measured on the NanoPC-T6, 2026-10-18: `/sys/class/watchdog/
//! watchdog0/state` "inactive", `WDT_CR` 0x00000008). [`arm`] leaves it running
//! with the period asked for, set to reset the system on the first timeout,
//! and reports what it found, so the board says which case it was.
//!
//! Register offsets and fields: Zephyr's `drivers/watchdog/wdt_dw.h`
//! (Apache-2.0, Zephyr af17c0c8). Facts this module relies on:
//!
//! 1. `WDT_COMP_TYPE` (0xfc) reads 0x44570120 ("DW", 0x0120) on every
//!    DW_apb_wdt (`WDT_COMP_TYPE_VALUE`). The T6 reads 0x44570120 and
//!    `WDT_COMP_VERSION` 0x3131302a ("110*", 1.10a) (board, read-only, 2026-10-18).
//! 2. `WDT_COMP_PARAM_1` (0xf4) describes the build: `WDT_CNT_WIDTH` (bits
//!    28:24, plus 16), `WDT_USE_FIX_TOP` (bit 6: the period of `TOP` = i is
//!    2^(16 + i) clock cycles), `WDT_HC_TOP` (bit 5: `TOP` cannot be
//!    written), `WDT_ALWAYS_EN` (bit 0). The T6 reads 0x10000ac0: width 32,
//!    fixed tops, `TOP` writable, not always enabled, no dual top.
//! 3. `WDT_CR` (0x00): `WDT_EN` (bit 0) starts the counter, `RMOD` (bit 1)
//!    0 = "Generate a system reset" on the first timeout, 1 = an interrupt
//!    first and the reset on the second. `RPL` (bits 4:2) is the reset pulse
//!    length; it is written back as found.
//! 4. Once `WDT_EN` is set, software cannot clear it: only a reset of the
//!    block does (Linux v6.18 drivers/watchdog/dw_wdt.c:284-297 stops the
//!    watchdog through the block's reset line and otherwise marks it
//!    "hardware running"; the RK3588 node has no `resets`,
//!    rk3588-base.dtsi:2461-2467; read for the fact only). So an armed
//!    watchdog stays armed, which is the point.
//! 5. `WDT_TORR` (0x04): `TOP` (bits 3:0) and `TOP_INIT` (bits 7:4). Both are
//!    written with the same value, as Linux's dw_wdt_set_timeout does
//!    (dw_wdt.c:225-226).
//! 6. Writing 0x76 to `WDT_CRR` (0x0c) restarts the counter
//!    (`WDT_CRR_RESTART_KEY`); a write of `TOP` takes effect at the next
//!    restart, so [`arm`] restarts after writing it.
//! 7. `WDT_EOI` (0x14) clears the interrupt when READ: nothing here reads it.
//!
//! What the system reset resets on the RK3588 (the CRU's global reset
//! configuration, `GLB_RST_CON` 0xfd7c0c10) is not in any source on this host
//! (the TRM is not); the first board run with a UART measures it
//! (driver/docs/nanopc-t6-mali.md, "Z1").

use crate::mmio::Mmio;

/// Register offsets (`wdt_dw.h`).
pub const CR: usize = 0x00;
pub const TORR: usize = 0x04;
pub const CCVR: usize = 0x08;
pub const CRR: usize = 0x0c;
pub const COMP_PARAM_1: usize = 0xf4;
pub const COMP_VERSION: usize = 0xf8;
pub const COMP_TYPE: usize = 0xfc;

/// `WDT_CR` fields.
pub mod cr {
    pub const WDT_EN: u32 = 1 << 0;
    pub const RMOD: u32 = 1 << 1;
    pub const RPL_MASK: u32 = 0x7 << 2;
}

/// `WDT_COMP_PARAM_1` fields.
pub mod param1 {
    pub const ALWAYS_EN: u32 = 1 << 0;
    pub const HC_TOP: u32 = 1 << 5;
    pub const USE_FIX_TOP: u32 = 1 << 6;
    pub const CNT_WIDTH_SHIFT: u32 = 24;
    pub const CNT_WIDTH_MASK: u32 = 0x1f << CNT_WIDTH_SHIFT;
}

/// `WDT_TORR.TOP`.
pub const TORR_TOP_MASK: u32 = 0xf;
pub const TORR_TOP_INIT_SHIFT: u32 = 4;

/// `WDT_COMP_TYPE` of every DW_apb_wdt.
pub const COMP_TYPE_VALUE: u32 = 0x4457_0120;

/// The restart key written to `WDT_CRR`.
pub const RESTART_KEY: u32 = 0x76;

/// The largest `TOP`.
pub const TOP_MAX: u8 = 15;

/// Why [`arm`] refused. The value crosses to C (`YD_DW_WDT_REFUSED_*`), so
/// each cause is one number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Refusal {
    /// `WDT_COMP_TYPE` is not 0x44570120: not a DW_apb_wdt, or a block that
    /// does not answer (its clock gated).
    NotDwWdt = 1,
    /// The build's periods are user-defined (`WDT_USE_FIX_TOP` 0) or fixed
    /// (`WDT_HC_TOP` 1): the period of a `TOP` is not known, or cannot be set.
    TopNotProgrammable = 2,
    /// `TOP` above 15, a period wider than the counter, or a clock of 0 Hz.
    BadArgument = 3,
    /// `WDT_CR.WDT_EN` read back clear after the write.
    NotEnabled = 4,
    /// `WDT_CR.RMOD` read back set: the first timeout would only interrupt.
    RmodReadBack = 5,
    /// `WDT_TORR.TOP` read back other than written.
    TopReadBack = 6,
    /// `WDT_CCVR` above the period right after the restart: the period asked
    /// for is not the one counting.
    CounterReadBack = 7,
}

/// The counter width a `WDT_COMP_PARAM_1` value says.
pub const fn cnt_width(param1: u32) -> u32 {
    ((param1 & param1::CNT_WIDTH_MASK) >> param1::CNT_WIDTH_SHIFT) + 16
}

/// Clock cycles of one period at `TOP` = `top` (fixed tops): 2^(16 + top).
pub const fn period_cycles(top: u8) -> u64 {
    1u64 << (16 + top as u32)
}

/// Milliseconds of one period at `top` for a counter clock of `clk_hz`,
/// rounded down (the reset never comes later than this says). `None` for a
/// `top` above [`TOP_MAX`] or a clock of 0.
pub const fn period_ms(top: u8, clk_hz: u32) -> Option<u32> {
    if top > TOP_MAX || clk_hz == 0 {
        return None;
    }
    Some((period_cycles(top) * 1000 / clk_hz as u64) as u32)
}

/// The `WDT_CR` value to write, given the value found: running, system reset
/// on the first timeout, the reset pulse length as found, every other bit 0.
pub const fn cr_for(found: u32) -> u32 {
    (found & cr::RPL_MASK) | cr::WDT_EN
}

/// What [`arm`] found and left.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Armed {
    pub cr_found: u32,
    pub torr_found: u32,
    pub param1: u32,
    pub version: u32,
    pub cr_now: u32,
    pub torr_now: u32,
    /// `WDT_CCVR` read right after the restart.
    pub ccvr_now: u32,
    /// `WDT_EN` was set before this ran: an earlier stage had started it.
    pub was_running: bool,
    pub period_ms: u32,
}

/// What [`arm`] read before it refused, for the refusal's log line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused {
    pub why: Refusal,
    pub comp_type: u32,
    pub param1: u32,
    pub cr_found: u32,
    pub torr_found: u32,
    /// The last value read from the register the refusal is about (`WDT_CR`,
    /// `WDT_TORR` or `WDT_CCVR`; 0 when nothing was written yet).
    pub read_back: u32,
}

/// Restart (feed) the watchdog at `base`.
pub fn feed(m: &impl Mmio, base: usize) {
    m.write32(base + CRR, RESTART_KEY);
}

/// Arm the DW_apb_wdt at `base` for the period of `top` on a counter clock
/// of `clk_hz`, from whatever state it is in, restart it and read it back.
/// On success it runs with exactly that period, counted from the restart
/// this made, and resets the system on the first timeout. A refusal leaves
/// the block as the failed step left it; the caller resets the SoC.
///
/// Nothing is written before the identity and the build parameters are
/// checked, so a block that is not a DW_apb_wdt is never touched.
pub fn arm(m: &impl Mmio, base: usize, top: u8, clk_hz: u32) -> Result<Armed, Refused> {
    let comp_type = m.read32(base + COMP_TYPE);
    let param1 = m.read32(base + COMP_PARAM_1);
    let cr_found = m.read32(base + CR);
    let torr_found = m.read32(base + TORR);
    let refuse = |why, read_back| Refused { why, comp_type, param1, cr_found, torr_found, read_back };

    if comp_type != COMP_TYPE_VALUE {
        return Err(refuse(Refusal::NotDwWdt, 0));
    }
    if param1 & param1::USE_FIX_TOP == 0 || param1 & param1::HC_TOP != 0 {
        return Err(refuse(Refusal::TopNotProgrammable, 0));
    }
    let Some(period_ms) = period_ms(top, clk_hz) else {
        return Err(refuse(Refusal::BadArgument, 0));
    };
    if 16 + top as u32 > cnt_width(param1) {
        return Err(refuse(Refusal::BadArgument, 0));
    }
    let version = m.read32(base + COMP_VERSION);

    let t = top as u32;
    m.write32(base + TORR, t | t << TORR_TOP_INIT_SHIFT);
    m.write32(base + CR, cr_for(cr_found));
    feed(m, base);

    let cr_now = m.read32(base + CR);
    if cr_now & cr::WDT_EN == 0 {
        return Err(refuse(Refusal::NotEnabled, cr_now));
    }
    if cr_now & cr::RMOD != 0 {
        return Err(refuse(Refusal::RmodReadBack, cr_now));
    }
    let torr_now = m.read32(base + TORR);
    if torr_now & TORR_TOP_MASK != t {
        return Err(refuse(Refusal::TopReadBack, torr_now));
    }
    let ccvr_now = m.read32(base + CCVR);
    if u64::from(ccvr_now) > period_cycles(top) {
        return Err(refuse(Refusal::CounterReadBack, ccvr_now));
    }
    Ok(Armed {
        cr_found,
        torr_found,
        param1,
        version,
        cr_now,
        torr_now,
        ccvr_now,
        was_running: cr_found & cr::WDT_EN != 0,
        period_ms,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    pub const BASE: usize = 0xfeaf_0000;

    /// The values the T6's WDT0 read under Armbian, 2026-10-18 (read-only).
    pub const T6_CR: u32 = 0x0000_0008;
    pub const T6_TORR: u32 = 0x0000_000e;
    pub const T6_PARAM1: u32 = 0x1000_0ac0;
    pub const T6_VERSION: u32 = 0x3131_302a;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Op {
        R(usize, u32),
        W(usize, u32),
    }

    /// A DW_apb_wdt as `wdt_dw.h` describes it: `WDT_EN` cannot be cleared
    /// once set (fact 4), a restart loads the counter with the period of
    /// `TOP` minus one, plus the faults the refusals are for.
    pub struct Wdt {
        pub regs: RefCell<[u32; 64]>,
        pub ops: RefCell<Vec<Op>>,
        pub restarts: RefCell<u32>,
        /// Writes to `WDT_CR` are ignored.
        pub cr_dead: bool,
        /// `RMOD` reads 1 whatever is written.
        pub rmod_stuck: bool,
        /// Writes to `WDT_TORR` are ignored.
        pub torr_dead: bool,
        /// A restart does not reload the counter.
        pub no_reload: bool,
    }

    impl Wdt {
        pub fn t6() -> Wdt {
            let w = Wdt {
                regs: RefCell::new([0; 64]),
                ops: RefCell::new(Vec::new()),
                restarts: RefCell::new(0),
                cr_dead: false,
                rmod_stuck: false,
                torr_dead: false,
                no_reload: false,
            };
            {
                let mut r = w.regs.borrow_mut();
                r[CR / 4] = T6_CR;
                r[TORR / 4] = T6_TORR;
                r[CCVR / 4] = 0xffff;
                r[COMP_PARAM_1 / 4] = T6_PARAM1;
                r[COMP_VERSION / 4] = T6_VERSION;
                r[COMP_TYPE / 4] = COMP_TYPE_VALUE;
            }
            w
        }
        pub fn ops(&self) -> Vec<Op> {
            self.ops.borrow().clone()
        }
        pub fn writes(&self) -> Vec<(usize, u32)> {
            self.ops()
                .into_iter()
                .filter_map(|o| if let Op::W(a, v) = o { Some((a - BASE, v)) } else { None })
                .collect()
        }
    }

    impl Mmio for Wdt {
        fn read32(&self, addr: usize) -> u32 {
            let off = addr - BASE;
            assert_ne!(off, 0x14, "WDT_EOI must not be read: a read clears the interrupt");
            let mut v = self.regs.borrow()[off / 4];
            if off == CR && self.rmod_stuck {
                v |= cr::RMOD;
            }
            self.ops.borrow_mut().push(Op::R(addr, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            self.ops.borrow_mut().push(Op::W(addr, val));
            let off = addr - BASE;
            let mut r = self.regs.borrow_mut();
            match off {
                CR if !self.cr_dead => r[CR / 4] = val | (r[CR / 4] & cr::WDT_EN),
                TORR if !self.torr_dead => r[TORR / 4] = val & 0xff,
                CRR if val == RESTART_KEY => {
                    *self.restarts.borrow_mut() += 1;
                    if !self.no_reload && r[CR / 4] & cr::WDT_EN != 0 {
                        let top = (r[TORR / 4] & TORR_TOP_MASK) as u8;
                        r[CCVR / 4] = (period_cycles(top) - 1) as u32;
                    }
                }
                CR | TORR | CRR => {}
                _ => panic!("write to read-only register 0x{off:02x}"),
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!("the DW_apb_wdt is accessed 32 bits wide")
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!("the DW_apb_wdt is accessed 32 bits wide")
        }
    }

    #[test]
    fn the_periods_at_24_mhz() {
        // tclk_wdt0 is xin24m (clk-rk3588.c:1052; board clk_summary 24000000)
        let ms: Vec<u32> = (0..=TOP_MAX).map(|t| period_ms(t, 24_000_000).unwrap()).collect();
        assert_eq!(ms[0], 2); // 65536 / 24 MHz = 2.73 ms
        assert_eq!(ms[11], 5592);
        assert_eq!(ms[12], 11184);
        assert_eq!(ms[13], 22369);
        assert_eq!(ms[14], 44739); // Linux's 44 s (board: timeout=44, TORR 0x0e)
        assert_eq!(ms[15], 89478);
        assert_eq!(period_ms(16, 24_000_000), None);
        assert_eq!(period_ms(13, 0), None);
    }

    #[test]
    fn the_t6_as_armbian_left_it_is_armed_with_one_torr_write_then_the_cr_then_a_restart() {
        let w = Wdt::t6();
        let a = arm(&w, BASE, 13, 24_000_000).unwrap();
        assert_eq!(w.writes(), vec![(TORR, 0xdd), (CR, 0x09), (CRR, RESTART_KEY)]);
        assert_eq!(
            a,
            Armed {
                cr_found: T6_CR,
                torr_found: T6_TORR,
                param1: T6_PARAM1,
                version: T6_VERSION,
                cr_now: 0x09,
                torr_now: 0xdd,
                ccvr_now: (1 << 29) - 1,
                was_running: false,
                period_ms: 22369,
            }
        );
        // the reset pulse length is kept, RMOD is 0 (system reset at once)
        assert_eq!(a.cr_now & cr::RPL_MASK, T6_CR & cr::RPL_MASK);
        assert_eq!(a.cr_now & cr::RMOD, 0);
    }

    #[test]
    fn a_running_watchdog_in_interrupt_mode_is_moved_to_reset_mode_and_restarted() {
        let w = Wdt::t6();
        w.regs.borrow_mut()[CR / 4] = cr::WDT_EN | cr::RMOD | (0x5 << 2);
        let a = arm(&w, BASE, 14, 24_000_000).unwrap();
        assert!(a.was_running);
        assert_eq!(a.cr_now, cr::WDT_EN | (0x5 << 2));
        assert_eq!(*w.restarts.borrow(), 1);
    }

    /// What Linux's dw_wdt leaves when `/dev/watchdog0` is open at the kexec
    /// (`nanopc-t6.sh --go --linux-wdt`): dw_wdt_start wrote TORR for its
    /// 44 s (TOP 14, 0xee), restarted, and set `WDT_EN` with `RMOD` 0
    /// (linux v6.18 dw_wdt.c:198-245, 260-283); the counter is part way down.
    /// [`arm`] only changes the period and restarts: every `WDT_CR` write
    /// keeps `WDT_EN`, so the watchdog never stops between Linux and us.
    #[test]
    fn a_watchdog_linux_left_running_is_re_topped_and_restarted_never_stopped() {
        let w = Wdt::t6();
        {
            let mut r = w.regs.borrow_mut();
            r[CR / 4] = T6_CR | cr::WDT_EN;
            r[TORR / 4] = 0xee;
            r[CCVR / 4] = 0x1234_5678;
        }
        let a = arm(&w, BASE, 13, 24_000_000).unwrap();
        assert_eq!(w.writes(), vec![(TORR, 0xdd), (CR, T6_CR | cr::WDT_EN), (CRR, RESTART_KEY)]);
        for (off, v) in w.writes() {
            if off == CR {
                assert_ne!(v & cr::WDT_EN, 0, "a WDT_CR write without WDT_EN: {v:#x}");
            }
        }
        assert!(a.was_running);
        assert_eq!((a.cr_found, a.torr_found), (T6_CR | cr::WDT_EN, 0xee));
        assert_eq!((a.cr_now, a.torr_now, a.period_ms), (T6_CR | cr::WDT_EN, 0xdd, 22369));
        // the restart reloaded the counter with the new period, not Linux's
        assert_eq!(a.ccvr_now, (1 << 29) - 1);
        assert_eq!(*w.restarts.borrow(), 1);
        // the TOP 15 retry after a refusal takes the same path
        let a = arm(&w, BASE, 15, 24_000_000).unwrap();
        assert!(a.was_running);
        assert_eq!((a.torr_now, a.period_ms), (0xff, 89478));
    }

    #[test]
    fn nothing_is_written_to_a_block_that_is_not_a_dw_wdt() {
        for (off, v, why) in [
            (COMP_TYPE, 0, Refusal::NotDwWdt),
            (COMP_TYPE, 0xffff_ffff, Refusal::NotDwWdt),
            (COMP_PARAM_1, T6_PARAM1 & !param1::USE_FIX_TOP, Refusal::TopNotProgrammable),
            (COMP_PARAM_1, T6_PARAM1 | param1::HC_TOP, Refusal::TopNotProgrammable),
        ] {
            let w = Wdt::t6();
            w.regs.borrow_mut()[off / 4] = v;
            assert_eq!(arm(&w, BASE, 13, 24_000_000).unwrap_err().why, why, "{off:#x} = {v:#x}");
            assert!(w.writes().is_empty(), "{off:#x} = {v:#x}: {:?}", w.writes());
        }
    }

    #[test]
    fn bad_arguments_are_refused_before_any_write() {
        for (top, hz) in [(16u8, 24_000_000u32), (13, 0)] {
            let w = Wdt::t6();
            assert_eq!(arm(&w, BASE, top, hz).unwrap_err().why, Refusal::BadArgument);
            assert!(w.writes().is_empty());
        }
        // a 24-bit counter cannot count the period of TOP 9 (2^25 cycles)
        let w = Wdt::t6();
        w.regs.borrow_mut()[COMP_PARAM_1 / 4] = (T6_PARAM1 & !param1::CNT_WIDTH_MASK) | (8 << 24);
        assert_eq!(arm(&w, BASE, 9, 24_000_000).unwrap_err().why, Refusal::BadArgument);
        assert!(arm(&w, BASE, 8, 24_000_000).is_ok());
    }

    #[test]
    fn each_read_back_fault_has_its_refusal() {
        let w = Wdt { cr_dead: true, ..Wdt::t6() };
        let r = arm(&w, BASE, 13, 24_000_000).unwrap_err();
        assert_eq!((r.why, r.read_back), (Refusal::NotEnabled, T6_CR));

        let w = Wdt { rmod_stuck: true, ..Wdt::t6() };
        assert_eq!(arm(&w, BASE, 13, 24_000_000).unwrap_err().why, Refusal::RmodReadBack);

        let w = Wdt { torr_dead: true, ..Wdt::t6() };
        let r = arm(&w, BASE, 13, 24_000_000).unwrap_err();
        assert_eq!((r.why, r.read_back), (Refusal::TopReadBack, T6_TORR));

        // Armbian's counter value 0xffff is below every period: a block that
        // does not reload is caught only when it was left above the period
        let w = Wdt { no_reload: true, ..Wdt::t6() };
        w.regs.borrow_mut()[CCVR / 4] = 0xffff_ffff;
        let r = arm(&w, BASE, 13, 24_000_000).unwrap_err();
        assert_eq!((r.why, r.read_back), (Refusal::CounterReadBack, 0xffff_ffff));
    }

    #[test]
    fn feed_writes_the_key_and_nothing_else() {
        let w = Wdt::t6();
        feed(&w, BASE);
        assert_eq!(w.writes(), vec![(CRR, RESTART_KEY)]);
    }
}
