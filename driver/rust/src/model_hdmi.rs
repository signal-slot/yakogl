// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' model of the i.MX 8M Plus HDMI path (tests only): CCM,
//! GPC, the HDMI TX block control, the PVI, LCDIF3, the DesignWare HDMI TX
//! controller (8-bit registers) and the HDMI PHY, at fake addresses.
//!
//! Like [`crate::model::Model`] it records every access in order and answers
//! only what the drivers poll, as far as IMX8MPRM Rev. 3 says the hardware
//! does; reset values and offsets are restated here, not shared with the
//! drivers, so a wrong constant cannot agree with itself. On top it checks
//! reachability the way the silicon would fail: a block control access needs
//! HDMIMIX powered, HDMI_APB_CLK_ROOT on and CCGR95 open (Table 5-2: every
//! HDMIMIX APB clock is HDMI_APB_CLK behind `ccm_clk_enable_hdmi`); LCDIF3,
//! the controller, the PHY and the PVI additionally need their block-control
//! clock enable and reset release (13.11.3.1.4-6), the PHY its own domain.
//! An access that breaks this fails the test. The `*_only` constructors skip
//! the checks for the single-block tests.
//!
//! What is modelled beyond plain storage:
//! - CCM and block-control SET/CLR/TOG aliases (5.1.6.1, 13.11.3.1.1), LCDIF
//!   `CTRL` aliases (13.3.3.1.2.1).
//! - GPC PU14/PU15: triggers self-clear, power-up needs the A53 mapping,
//!   power-down needs PCR and sets PSR (write 1 clears); the HDMIMIX ADB400
//!   acknowledge follows its request while HDMIMIX is powered and the HDMI
//!   NoC clock is enabled (the RM states the power-down direction only).
//!   Powering a domain down returns its blocks to their reset values.
//! - PHY: `PHY_REG34` reads locked a few reads after `MODE_SET_DONE` is
//!   released with the PHY out of power-down (`TX_CONTROL0[TX_PHY_PDOWN]`).
//! - Block control `TX_STATUS0` mirrors the PHY lock/ready and HPD
//!   (13.11.3.1.15).
//! - LCDIF `SHADOW_LOAD_EN` clears a read after it is set while the pixel
//!   clock runs (the PHY locked, `LCDIF_PIX_CLK_EN`, `DISP_ON`).
//! - Controller `phy_stat0` lock/HPD/RX_SENSE and the `mc_lockonclock_1`
//!   clock-present flags (write 1 clears; set again while the clocks run).
//! - PVI FIFO underflow on request.

use crate::mmio::Mmio;
use std::cell::RefCell;
use std::collections::BTreeMap;

pub const CCM: &str = "ccm";
pub const GPC: &str = "gpc";
pub const BLK: &str = "hdmi-blk";
pub const PVI: &str = "pvi";
pub const LCDIF: &str = "lcdif3";
pub const HDMI: &str = "hdmi-tx";
pub const PHY: &str = "hdmi-phy";

const BLOCKS: [(&str, usize, usize); 7] = [
    (CCM, 0x1000_0000, 0x1_0000),
    (GPC, 0x2000_0000, 0x1_0000),
    (BLK, 0x3000_0000, 0x1_0000),
    (PVI, 0x4000_0000, 0x1000),
    (LCDIF, 0x5000_0000, 0x1_0000),
    (HDMI, 0x6000_0000, 0x8000),
    (PHY, 0x7000_0000, 0x100),
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    R(&'static str, u32, u32),
    W(&'static str, u32, u32),
}

#[derive(Default, Clone, Copy, Debug)]
pub struct HdmiFaults {
    pub gpc_pup_stuck: bool,
    pub adb_no_ack: bool,
    pub phy_no_lock: bool,
    /// The pixel clock never times a frame: shadow loads never complete.
    pub no_vsync: bool,
    pub lcdif_reset_stuck: bool,
    pub pvi_underflow: bool,
    pub no_hpd: bool,
}

// Offsets, restated from IMX8MPRM Rev. 3.
const GPC_PUP: u32 = 0xd8;
const GPC_PDN: u32 = 0xe4;
const GPC_PUP_STATUS0: u32 = 0x124;
const GPC_PWRHSK: u32 = 0x190;
const GPC_MAP: u32 = 0x1cc;
const PU_HDMIMIX: u32 = 1 << 14;
const PU_HDMI_PHY: u32 = 1 << 15;
const ADB_REQ: u32 = 1 << 13;
const ADB_ACK: u32 = 1 << 29;
/// (trigger, A53 mapping bit, PGC offset)
const DOMAINS: [(u32, u32, u32); 2] = [(PU_HDMIMIX, 1 << 16, 0xe80), (PU_HDMI_PHY, 1 << 17, 0xec0)];
const CCM_HDMI_APB_ROOT: u32 = 0x8b00;
const CCM_CCGR95: u32 = 0x45f0;

const B_RESET: u32 = 0x20;
const B_CLK0: u32 = 0x40;
const B_CLK1: u32 = 0x50;
const B_TXCTL0: u32 = 0x200;
const B_TXSTAT0: u32 = 0x230;

const L_CTRL: u32 = 0x0;
const L_DISP_PARA: u32 = 0x10;
const L_INT_STATUS_D0: u32 = 0x24;
const L_DESC5: u32 = 0x214;

const P_CTRL: u32 = 0x0;
const P_STATUS: u32 = 0x8;
const P_CLR: u32 = 0xc;

const H_PHY_STAT0: u32 = 0x3004;
const H_LOCKONCLOCK: u32 = 0x4006;

const PHY_R33: u32 = 0x84;
const PHY_R34: u32 = 0x88;
const PHY_R4: u32 = 0x10;

fn reset_image(block: &'static str) -> Vec<(u32, u32)> {
    match block {
        BLK => vec![(B_TXCTL0, 0x18)],
        PVI => vec![(P_CTRL, 0x0037_7000), (0x4, 0x3f)],
        LCDIF => vec![
            (L_CTRL, 0x8000_0000),
            (0x18, 0x0003_0003),
            (0x1c, 0x0003_0003),
            (0x20, 0x0003_0003),
            (0x21c, 1),
            (0x238, 0x0100_0100),
        ],
        HDMI => vec![
            (0x0, 0x21),
            (0x1ff, 0x03),
            (0x804, 0x24),
            (0x1000, 0x70),
            (0x10e0, 0x10),
            (0x3000, 0x06),
            (0x4002, 0xdf),
        ],
        PHY => vec![
            (0x04, 0xd1),
            (0x08, 0x3e),
            (0x0c, 0x15),
            (0x10, 0xc0),
            (0x14, 0x90),
            (0x18, 0x88),
            (0x1c, 0x42),
            (0x20, 0x47),
            (0x24, 0x30),
            (0x28, 0x33),
            (0x2c, 0x65),
            (0x30, 0x20),
            (0x34, 0xa6),
            (0x38, 0x24),
            (0x3c, 0x80),
            (0x40, 0x6c),
            (0x44, 0xf2),
            (0x48, 0x67),
            (0x50, 0x10),
            (0x5c, 0x10),
            (0x60, 0x60),
            (0x84, 0x80),
            (0xa4, 0xe0),
            (0xa8, 0x83),
            (0xac, 0x0f),
            (0xb0, 0x3e),
            (0xb4, 0xf8),
        ],
        _ => vec![],
    }
}

#[derive(Default)]
struct State {
    regs: BTreeMap<(&'static str, u32), u32>,
    log: Vec<Access>,
    faults: HdmiFaults,
    ungated: bool,
    powered: u32,
    pup_pending: u32,
    pup_reads: u32,
    pdn_pending: u32,
    pdn_reads: u32,
    /// MODE_SET_DONE went 0 -> 1: the PLL is acquiring.
    phy_acquiring: bool,
    phy_lock_reads: u32,
    phy_locked: bool,
    shadow_reads: u32,
}

#[derive(Default)]
pub struct HdmiModel(RefCell<State>);

impl HdmiModel {
    pub fn new() -> HdmiModel {
        HdmiModel::with_faults(HdmiFaults::default())
    }

    pub fn with_faults(f: HdmiFaults) -> HdmiModel {
        let m = HdmiModel::default();
        {
            let mut s = m.0.borrow_mut();
            s.faults = f;
            for b in [BLK, PVI, LCDIF, HDMI, PHY] {
                for (o, v) in reset_image(b) {
                    s.regs.insert((b, o), v);
                }
            }
            // both domains powered down: PSR set
            for (_, _, pgc) in DOMAINS {
                s.regs.insert((GPC, pgc + 0xc), 1);
            }
        }
        m
    }

    fn ungated(f: HdmiFaults) -> HdmiModel {
        let m = HdmiModel::with_faults(f);
        {
            let mut s = m.0.borrow_mut();
            s.ungated = true;
            s.powered = PU_HDMIMIX | PU_HDMI_PHY;
        }
        m
    }

    pub fn lcdif_only() -> HdmiModel {
        HdmiModel::ungated(HdmiFaults::default())
    }
    pub fn lcdif_only_with(f: HdmiFaults) -> HdmiModel {
        HdmiModel::ungated(f)
    }
    pub fn phy_only() -> HdmiModel {
        HdmiModel::ungated(HdmiFaults::default())
    }
    pub fn phy_only_with(f: HdmiFaults) -> HdmiModel {
        HdmiModel::ungated(f)
    }
    pub fn hdmi_only() -> HdmiModel {
        HdmiModel::ungated(HdmiFaults::default())
    }

    pub fn base(&self, block: &str) -> usize {
        BLOCKS.iter().find(|b| b.0 == block).expect("block").1
    }

    pub fn poke(&self, block: &'static str, off: u32, val: u32) {
        self.0.borrow_mut().regs.insert((block, off), val);
    }

    pub fn peek(&self, block: &'static str, off: u32) -> u32 {
        let s = self.0.borrow();
        HdmiModel::value(&s, block, off)
    }

    pub fn powered(&self) -> u32 {
        self.0.borrow().powered
    }

    pub fn accesses(&self) -> Vec<Access> {
        self.0.borrow().log.clone()
    }

    pub fn writes(&self) -> Vec<(&'static str, u32, u32)> {
        self.0
            .borrow()
            .log
            .iter()
            .filter_map(|a| match *a {
                Access::W(b, o, v) => Some((b, o, v)),
                Access::R(..) => None,
            })
            .collect()
    }

    pub fn clear_log(&self) {
        self.0.borrow_mut().log.clear();
    }

    fn raw(s: &State, block: &'static str, off: u32) -> u32 {
        s.regs.get(&(block, off)).copied().unwrap_or(0)
    }

    fn pixel_clock_runs(s: &State) -> bool {
        if s.faults.no_vsync {
            return false;
        }
        s.ungated
            || (s.phy_locked
                && HdmiModel::raw(s, BLK, B_CLK0) & (1 << 19) != 0
                && HdmiModel::raw(s, LCDIF, L_DISP_PARA) & (1 << 31) != 0)
    }

    fn value(s: &State, block: &'static str, off: u32) -> u32 {
        let raw = |o| HdmiModel::raw(s, block, o);
        match (block, off) {
            (CCM, o) if o >= 0x4000 => raw(o & !0xf),
            (BLK, B_TXSTAT0) => {
                let lock = if s.phy_locked { 0xe0 } else { 0 };
                let hpd = if s.faults.no_hpd { 0 } else { 0x100 };
                lock | hpd
            }
            (BLK, o) => raw(o & !0xf),
            (LCDIF, o) if o < 0x10 => raw(L_CTRL),
            (GPC, GPC_PWRHSK) => {
                let noc = s.ungated || HdmiModel::raw(s, BLK, B_CLK0) & (1 << 10) != 0;
                let ack = raw(off) & ADB_REQ != 0 && s.powered & PU_HDMIMIX != 0 && noc && !s.faults.adb_no_ack;
                (raw(off) & !ADB_ACK) | if ack { ADB_ACK } else { 0 }
            }
            (PHY, PHY_R34) => {
                if s.phy_locked {
                    0xf0 | 0x05
                } else {
                    0
                }
            }
            (HDMI, H_PHY_STAT0) => {
                let lock = u32::from(s.phy_locked);
                let hpd = if s.faults.no_hpd { 0 } else { 0xf2 };
                lock | hpd
            }
            (HDMI, H_LOCKONCLOCK) => {
                let run = if s.phy_locked { 0x60 } else { 0 };
                raw(off) | run
            }
            (PVI, P_STATUS) => {
                let on = raw(P_CTRL) & 1 != 0;
                let under = s.faults.pvi_underflow && on && HdmiModel::raw(s, LCDIF, L_DESC5) & (1 << 31) != 0;
                raw(off) | if under { 2 } else { 0 }
            }
            _ => raw(off),
        }
    }

    fn decode(addr: usize, width: u32) -> (&'static str, u32) {
        for (name, base, size) in BLOCKS {
            if addr >= base && addr < base + size {
                if name == HDMI {
                    assert_eq!(width, 8, "the HDMI TX controller has 8-bit registers: {width}-bit access at {addr:#x}");
                } else {
                    assert_eq!(width, 32, "{name} has 32-bit registers: {width}-bit access at {addr:#x}");
                    assert!(addr.is_multiple_of(4), "unaligned access at {addr:#x}");
                }
                return (name, (addr - base) as u32);
            }
        }
        panic!("access outside every modelled block: {addr:#x}");
    }

    fn check(s: &State, block: &'static str, what: &str, off: u32) {
        if s.ungated || block == CCM || block == GPC {
            return;
        }
        let ccm = |o| HdmiModel::value(s, CCM, o);
        assert!(s.powered & PU_HDMIMIX != 0, "{block} {what} {off:#x} with HDMIMIX powered off");
        assert!(ccm(CCM_HDMI_APB_ROOT) & (1 << 28) != 0, "{block} {what} {off:#x} with HDMI_APB_CLK_ROOT off");
        assert!(ccm(CCM_CCGR95) & 3 != 0, "{block} {what} {off:#x} with CCGR95 closed");
        let c0 = HdmiModel::raw(s, BLK, B_CLK0);
        let c1 = HdmiModel::raw(s, BLK, B_CLK1);
        let rst = HdmiModel::raw(s, BLK, B_RESET);
        let need = |cond: bool, name: &str| assert!(cond, "{block} {what} {off:#x} without {name}");
        match block {
            LCDIF => {
                need(c0 & (1 << 16) != 0, "LCDIF_APB_CLK_EN");
                need(rst & (1 << 5) != 0, "LCDIF_APB_RESET_N released");
            }
            HDMI => {
                need(c1 & (1 << 14) != 0, "TX_APB_CLK_EN");
                need(rst & (1 << 11) != 0, "TX_APBRSTZ released");
            }
            PHY => {
                need(s.powered & PU_HDMI_PHY != 0, "the HDMI PHY domain");
                need(c1 & (1 << 22) != 0, "TX_PHY_APB_CLK_EN");
                need(rst & (1 << 12) != 0, "TX_PHY_PRESETN released");
            }
            PVI => {
                need(c0 & 1 != 0, "GLOBAL_APB_CLK_EN");
                need(rst & (1 << 22) != 0, "VID_LINK_SLV_RESETN released");
            }
            _ => {}
        }
    }

    fn domain_reset(s: &mut State, pu: u32) {
        let blocks: &[&'static str] = if pu == PU_HDMIMIX { &[BLK, PVI, LCDIF, HDMI] } else { &[PHY] };
        for b in blocks {
            s.regs.retain(|k, _| k.0 != *b);
            for (o, v) in reset_image(b) {
                s.regs.insert((b, o), v);
            }
        }
        if pu == PU_HDMI_PHY || pu == PU_HDMIMIX {
            s.phy_locked = false;
            s.phy_acquiring = false;
        }
    }

    fn gpc_read(s: &mut State, off: u32) {
        if off == GPC_PUP && s.pup_pending != 0 && !s.faults.gpc_pup_stuck {
            if s.pup_reads > 0 {
                s.pup_reads -= 1;
                return;
            }
            let map = HdmiModel::raw(s, GPC, GPC_MAP);
            for (pu, mbit, _) in DOMAINS {
                if s.pup_pending & pu == 0 {
                    continue;
                }
                if map & mbit == 0 {
                    *s.regs.entry((GPC, GPC_PUP_STATUS0)).or_insert(0) |= pu;
                } else {
                    s.powered |= pu;
                }
            }
            let p = s.pup_pending;
            *s.regs.entry((GPC, GPC_PUP)).or_insert(0) &= !p;
            s.pup_pending = 0;
        }
        if off == GPC_PDN && s.pdn_pending != 0 {
            if s.pdn_reads > 0 {
                s.pdn_reads -= 1;
                return;
            }
            for (pu, _, pgc) in DOMAINS {
                if s.pdn_pending & pu != 0 && HdmiModel::raw(s, GPC, pgc) & 1 != 0 && s.powered & pu != 0 {
                    s.powered &= !pu;
                    *s.regs.entry((GPC, pgc + 0xc)).or_insert(0) |= 1;
                    HdmiModel::domain_reset(s, pu);
                }
            }
            let p = s.pdn_pending;
            *s.regs.entry((GPC, GPC_PDN)).or_insert(0) &= !p;
            s.pdn_pending = 0;
        }
    }

    fn read(&self, addr: usize, width: u32) -> u32 {
        let (blk, off) = HdmiModel::decode(addr, width);
        let mut s = self.0.borrow_mut();
        HdmiModel::check(&s, blk, "read", off);
        if blk == GPC {
            HdmiModel::gpc_read(&mut s, off);
        }
        if blk == PHY && off == PHY_R34 && s.phy_acquiring && !s.phy_locked {
            if s.phy_lock_reads > 0 {
                s.phy_lock_reads -= 1;
            } else {
                s.phy_locked = true;
            }
        }
        if blk == LCDIF
            && off == L_DESC5
            && HdmiModel::raw(&s, LCDIF, L_DESC5) & (1 << 30) != 0
            && HdmiModel::pixel_clock_runs(&s)
        {
            if s.shadow_reads > 0 {
                s.shadow_reads -= 1;
            } else {
                *s.regs.entry((LCDIF, L_DESC5)).or_insert(0) &= !(1 << 30);
            }
        }
        let v = HdmiModel::value(&s, blk, off);
        s.log.push(Access::R(blk, off, v));
        v
    }

    fn write(&self, addr: usize, width: u32, val: u32) {
        let (blk, off) = HdmiModel::decode(addr, width);
        let mut s = self.0.borrow_mut();
        s.log.push(Access::W(blk, off, val));
        HdmiModel::check(&s, blk, "write", off);
        let alias = off & 0xf;
        match blk {
            CCM if off >= 0x4000 && alias != 0 => {
                let r = s.regs.entry((CCM, off & !0xf)).or_insert(0);
                match alias {
                    4 => *r |= val,
                    8 => *r &= !val,
                    0xc => *r ^= val,
                    _ => panic!("CCM alias {off:#x}"),
                }
            }
            BLK if off == B_TXSTAT0 => {}
            BLK if alias != 0 => {
                let r = s.regs.entry((BLK, off & !0xf)).or_insert(0);
                match alias {
                    4 => *r |= val,
                    8 => *r &= !val,
                    0xc => *r ^= val,
                    _ => panic!("block control alias {off:#x}"),
                }
            }
            LCDIF if off < 0x10 => {
                let stuck = s.faults.lcdif_reset_stuck;
                let r = s.regs.entry((LCDIF, L_CTRL)).or_insert(0);
                match off {
                    0 => *r = val,
                    4 => *r |= val,
                    8 => *r &= !(if stuck { val & !(1 << 31) } else { val }),
                    _ => *r ^= val,
                }
            }
            LCDIF if off == L_INT_STATUS_D0 => {
                *s.regs.entry((LCDIF, off)).or_insert(0) &= !val;
            }
            LCDIF if off == L_DESC5 => {
                if val & (1 << 30) != 0 {
                    s.shadow_reads = 1;
                }
                s.regs.insert((LCDIF, off), val);
            }
            GPC if off == GPC_PUP => {
                s.pup_pending |= val & (PU_HDMIMIX | PU_HDMI_PHY);
                s.pup_reads = 1;
                s.regs.insert((GPC, off), val);
            }
            GPC if off == GPC_PDN => {
                s.pdn_pending |= val & (PU_HDMIMIX | PU_HDMI_PHY);
                s.pdn_reads = 1;
                s.regs.insert((GPC, off), val);
            }
            GPC if off == 0xe8c || off == 0xecc => {
                // PSR: write 1 to clear
                *s.regs.entry((GPC, off)).or_insert(0) &= !(val & 1);
            }
            PHY if off == PHY_R34 => {}
            PHY if off == PHY_R33 => {
                let was = HdmiModel::raw(&s, PHY, PHY_R33) & 0x80;
                s.regs.insert((PHY, off), val);
                if val & 0x80 == 0 {
                    s.phy_locked = false;
                    s.phy_acquiring = false;
                } else if was == 0 {
                    let pdown = !s.ungated && HdmiModel::raw(&s, BLK, B_TXCTL0) & 0x8 != 0;
                    let integer = HdmiModel::raw(&s, PHY, PHY_R4) & 0x80 == 0;
                    s.phy_acquiring = !pdown && integer && !s.faults.phy_no_lock;
                    s.phy_lock_reads = 2;
                }
            }
            HDMI if off == H_LOCKONCLOCK => {
                *s.regs.entry((HDMI, off)).or_insert(0) &= !val;
            }
            PVI if off == P_CLR => {
                *s.regs.entry((PVI, P_STATUS)).or_insert(0) &= !val;
            }
            _ => {
                s.regs.insert((blk, off), val);
            }
        }
    }
}

impl Mmio for HdmiModel {
    fn read32(&self, addr: usize) -> u32 {
        self.read(addr, 32)
    }
    fn write32(&self, addr: usize, val: u32) {
        self.write(addr, 32, val)
    }
    fn read8(&self, addr: usize) -> u8 {
        self.read(addr, 8) as u8
    }
    fn write8(&self, addr: usize, val: u8) {
        self.write(addr, 8, u32::from(val))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(expected = "HDMIMIX powered off")]
    fn the_block_control_is_unreachable_while_hdmimix_is_off() {
        let m = HdmiModel::new();
        let _ = m.read32(m.base(BLK) + 0x40);
    }

    #[test]
    #[should_panic(expected = "8-bit registers")]
    fn the_controller_takes_byte_accesses_only() {
        let m = HdmiModel::hdmi_only();
        let _ = m.read32(m.base(HDMI));
    }
}
