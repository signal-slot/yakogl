// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The adopt over a register model loaded with the board's dump
//! (driver/rust/tests/t6-display/linux-dump-adopt.txt): the VOP2's video
//! ports, `REG_CFG_DONE`, the port select and the window enables as they
//! take effect at a frame start, the two `vop_mmu` sets, and the read-only
//! blocks (PMU, CRU, GRFs, HDMI controller), which refuse any write.
//! Addresses are restated here from driver/docs/nanopc-t6-display.md and
//! the dump, not taken from the module, so a wrong constant cannot agree
//! with itself.

use super::*;
use std::cell::RefCell;
use std::collections::BTreeMap;

const DUMP: &str = include_str!("../../../tests/t6-display/linux-dump-adopt.txt");

const VOP: usize = 0xfdd9_0000;
const MMU0: usize = 0xfdd9_7e00;
const MMU1: usize = 0xfdd9_7f00;
const PMU: usize = 0xfd8d_8000;
const CRU: usize = 0xfd7c_0000;
const SYS_GRF: usize = 0xfd58_c000;
const PHY_GRF: usize = 0xfd5e_0000;
const HDMI: usize = 0xfde8_0000;
/// The HDMI QP TX0 offsets the board dump read without harm (CMU_STATUS,
/// CMU_VIDQPCLK_FREQ, CMU_LINKQPCLK_FREQ, VIDEO_MONITOR_STATUS0-6). The D0
/// dump measured a bus error on other offsets (0x10, 0x804, 0x80c): the
/// model refuses every read outside this list.
const HDMI_READS: [usize; 10] = [0xb0, 0xb8, 0xbc, 0x884, 0x888, 0x88c, 0x890, 0x894, 0x898, 0x89c];

const B: Bases =
    Bases { vop: VOP, vop_mmu: [MMU0, MMU1], pmu: PMU, cru: CRU, sys_grf: SYS_GRF, phy_grf: PHY_GRF, hdmi: HDMI };

// VOP offsets (driver/docs/nanopc-t6-display.md section 1)
const CFG_DONE: usize = 0x000;
const VP0_INT_CLR: usize = 0x0a4;
const PORT_SEL: usize = 0x608;
const LAYER_SEL: usize = 0x604;
const VP0_DSP_CTRL: usize = 0xc00;
const E0: usize = 0x1800;

/// The frame buffer of the tests: inside the image window, 64-byte aligned.
const FB: u64 = 0x1040_0000;
/// 1344 x 806 at 65 MHz.
const FRAME_US: u64 = 16_666;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Op {
    R(usize, u32),
    W(usize, u32),
    Delay(u32),
}

#[derive(Default, Clone, Copy)]
struct Faults {
    /// The pixel clock stopped: VP0 times no frame.
    no_frames: bool,
    /// A `vop_mmu` set ignores "disable paging".
    paging_stuck: bool,
}

/// The window enable words (Cluster n `WIN_CTRL0`, `CLUSTER_CTRL`; Esmart n
/// `REGION0_CTRL`, `YRGB_MST`), whose values in force change only at a
/// frame start that takes a `REG_CFG_DONE`.
fn window_words() -> Vec<usize> {
    let mut w = Vec::new();
    for n in 0..4 {
        w.extend([0x1000 + 0x200 * n, 0x1100 + 0x200 * n, 0x1810 + 0x200 * n, 0x1814 + 0x200 * n]);
    }
    w
}

#[derive(Default)]
struct State {
    regs: BTreeMap<usize, u32>,
    log: Vec<Op>,
    pending: [bool; 4],
    raw: [u32; 4],
    port_sel_written: Option<u32>,
    next_frame_us: [u64; 4],
    now_us: u64,
    /// The window words in force (offsets from the VOP).
    in_force: BTreeMap<usize, u32>,
    /// What broke a rule of the model (the tests require none).
    violations: Vec<String>,
}

struct Soc {
    s: RefCell<State>,
    f: Faults,
}

fn parse_dump(text: &str) -> BTreeMap<usize, u32> {
    let mut regs = BTreeMap::new();
    for l in text.lines().filter(|l| l.starts_with("0x")) {
        let f: Vec<&str> = l.split_whitespace().collect();
        assert_eq!(f.len(), 3, "{l}");
        assert_eq!(f[1], "32", "{l}");
        let a = usize::from_str_radix(&f[0][2..], 16).unwrap();
        let v = u32::from_str_radix(&f[2][2..], 16).unwrap();
        regs.insert(a, v);
    }
    regs
}

impl Soc {
    /// The dump with `edit` applied, `f` injected.
    fn with(f: Faults, edit: &[(usize, u32)]) -> Soc {
        let mut regs = parse_dump(DUMP);
        for (a, v) in edit {
            regs.insert(*a, *v);
        }
        let mut s = State { regs, ..State::default() };
        for w in window_words() {
            s.in_force.insert(w, s.regs.get(&(VOP + w)).copied().unwrap_or(0));
        }
        for p in 0..4 {
            s.next_frame_us[p] = 8000 + 1000 * p as u64;
        }
        Soc { s: RefCell::new(s), f }
    }
    fn dump() -> Soc {
        Soc::with(Faults::default(), &[])
    }
    fn running(s: &State, p: usize) -> bool {
        s.regs.get(&(VOP + 0xc00 + 0x100 * p)).copied().unwrap_or(0) & 1 << 31 == 0
    }
    /// A Linux window (any Cluster, or an Esmart not on our buffer) is
    /// enabled in force on a running port.
    fn linux_fetches(s: &State) -> Option<usize> {
        let sel = s.regs.get(&(VOP + PORT_SEL)).copied().unwrap_or(0);
        for n in 0..4 {
            let port = (sel >> (16 + 2 * n)) as usize & 3;
            let on = s.in_force[&(0x1000 + 0x200 * n)] & 1 != 0 || s.in_force[&(0x1100 + 0x200 * n)] & 1 != 0;
            if on && Self::running(s, port) {
                return Some(0x1000 + 0x200 * n);
            }
            let port = (sel >> (24 + 2 * n)) as usize & 3;
            let on = s.in_force[&(0x1810 + 0x200 * n)] & 1 != 0;
            if on && Self::running(s, port) && u64::from(s.in_force[&(0x1814 + 0x200 * n)]) != FB {
                return Some(0x1800 + 0x200 * n);
            }
        }
        None
    }
    fn advance(&self, us: u32) {
        let mut s = self.s.borrow_mut();
        s.log.push(Op::Delay(us));
        let end = s.now_us + u64::from(us);
        for p in 0..4 {
            while !self.f.no_frames && Self::running(&s, p) && s.next_frame_us[p] <= end {
                s.next_frame_us[p] += FRAME_US;
                s.raw[p] |= 1;
                if s.pending[p] {
                    s.pending[p] = false;
                    if let Some(sel) = s.port_sel_written.take() {
                        s.regs.insert(VOP + PORT_SEL, sel);
                    }
                    for w in window_words() {
                        let v = s.regs.get(&(VOP + w)).copied().unwrap_or(0);
                        s.in_force.insert(w, v);
                    }
                }
                // with paging off, a Linux window fetches its IOVA as a
                // physical address
                let off = [0usize, 1].iter().all(|i| s.regs[&(MMU0 + 0x100 * i + 4)] & 1 == 0);
                if off {
                    if let Some(w) = Self::linux_fetches(&s) {
                        s.violations.push(format!("frame start with paging off while window {w:#x} fetches"));
                    }
                }
            }
        }
        s.now_us = end;
    }
    fn now(&self) -> u64 {
        self.s.borrow().now_us
    }
    fn ops(&self) -> Vec<Op> {
        self.s.borrow().log.clone()
    }
    fn writes(&self) -> Vec<(usize, u32)> {
        self.ops().into_iter().filter_map(|o| if let Op::W(a, v) = o { Some((a, v)) } else { None }).collect()
    }
    fn violations(&self) -> Vec<String> {
        self.s.borrow().violations.clone()
    }
    fn reg(&self, a: usize) -> u32 {
        self.s.borrow().regs.get(&a).copied().unwrap_or(0)
    }
    /// A read or write of PD_VOP's blocks: the VOP or either vop_mmu set.
    fn vop_accessed(&self) -> bool {
        let in_pd_vop =
            |a: &usize| (VOP..VOP + 0x2000).contains(a) || [MMU0, MMU1].iter().any(|b| (*b..b + 0x100).contains(a));
        self.ops().iter().any(|o| matches!(o, Op::R(a, _) | Op::W(a, _) if in_pd_vop(a)))
    }
}

impl Mmio for Soc {
    fn read32(&self, addr: usize) -> u32 {
        let mut s = self.s.borrow_mut();
        let v = if (VOP..VOP + 0x2000).contains(&addr) {
            let off = addr - VOP;
            let raw = (0..4).find(|p| 0x0ac + 0x10 * p == off);
            if off == CFG_DONE {
                let pend = (0..4).fold(0, |a, p| a | u32::from(s.pending[p]) << p);
                s.regs.get(&addr).copied().unwrap_or(0) | pend
            } else if let Some(p) = raw {
                s.raw[p]
            } else {
                s.regs.get(&addr).copied().unwrap_or(0)
            }
        } else {
            assert!(
                [MMU0, MMU1].iter().any(|b| (*b..b + 0x100).contains(&addr))
                    || (PMU..PMU + 0x1000).contains(&addr)
                    || addr == CRU + 0x8d0
                    || addr == SYS_GRF + 0x384
                    || addr == PHY_GRF + 0x80
                    || HDMI_READS.contains(&addr.wrapping_sub(HDMI)),
                "read of {addr:#x}: not a block of the adopt"
            );
            s.regs.get(&addr).copied().unwrap_or(0)
        };
        s.log.push(Op::R(addr, v));
        v
    }

    fn write32(&self, addr: usize, val: u32) {
        let mut s = self.s.borrow_mut();
        s.log.push(Op::W(addr, val));
        if let Some(i) = [MMU0, MMU1].iter().position(|b| (*b..b + 0x100).contains(&addr)) {
            let st = MMU0 + 0x100 * i + 4;
            let status = s.regs[&st];
            let now = match (addr & 0xff, val) {
                (0x00 | 0x1c, _) => {
                    s.regs.insert(addr, val);
                    return;
                }
                (0x08, 1) if !self.f.paging_stuck => status & !1,
                (0x08, 2) if status & 1 != 0 => status | 4,
                (0x08, 3) => status & !4,
                (0x08, 1 | 2) => status,
                _ => panic!("write {val:#x} to vop_mmu {addr:#x}"),
            };
            if status & 1 != 0 && now & 1 == 0 {
                if let Some(w) = Self::linux_fetches(&s) {
                    s.violations.push(format!("paging off while window {w:#x} fetches"));
                }
            }
            s.regs.insert(st, now);
            return;
        }
        assert!(
            (VOP..VOP + 0x2000).contains(&addr),
            "write {val:#x} to {addr:#x}: the adopt writes the VOP and its IOMMU only"
        );
        let off = addr - VOP;
        assert_ne!(off, 0x004, "VERSION_INFO is read only");
        if off == CFG_DONE {
            s.regs.insert(addr, val & 1 << 15);
            for p in 0..4 {
                if val & 1 << p != 0 && val & 1 << (p + 16) != 0 {
                    s.pending[p] = true;
                }
            }
        } else if let Some(p) = (0..4).find(|p| 0x0a4 + 0x10 * p == off) {
            s.raw[p] &= !(val & (val >> 16) & 0xffff);
        } else if off == PORT_SEL {
            s.port_sel_written = Some(val);
        } else {
            assert!((0..4).all(|p| off != 0xc00 + 0x100 * p), "the adopt never writes DSP_CTRL");
            s.regs.insert(addr, val);
        }
    }

    fn read8(&self, _: usize) -> u8 {
        unreachable!("32-bit blocks only")
    }
    fn write8(&self, _: usize, _: u8) {
        unreachable!("32-bit blocks only")
    }
}

fn run(soc: &Soc, fb: u64) -> Adopt {
    adopt(soc, &B, fb, &mut |us| soc.advance(us))
}

/// What the adopt must write from the dump's state, in order (section 2
/// route (a), steps 2-5, and the module text). A read-modify-write that
/// leaves the value unchanged is still a write.
fn expected_writes() -> Vec<(usize, u32)> {
    let v = |off: usize, val: u32| (VOP + off, val);
    let mut w = vec![
        // take_over: every Cluster off (WIN_CTRL0 enable and YUV clip, then
        // CLUSTER_CTRL enable), whatever its port
        v(0x1000, 0),
        v(0x1100, 0),
        v(0x1200, 0),
        v(0x1300, 0),
        v(0x1400, 0),
        v(0x1500, 0x2000),
        v(0x1600, 0),
        v(0x1700, 0x2000),
        // Esmart1-3 off; Esmart0 is the one kept
        v(0x1a10, 0),
        v(0x1c10, 0),
        v(0x1e10, 0),
        // REG_CFG_DONE for VP0, the only running port; frame start cleared
        v(CFG_DONE, 0x0001_8001),
        v(VP0_INT_CLR, 0x0001_0001),
        // paging_off: stall both sets, disable paging, INT_MASK and DTE 0,
        // stall off
        (MMU0 + 8, 2),
        (MMU1 + 8, 2),
        (MMU0 + 8, 1),
        (MMU1 + 8, 1),
        (MMU0 + 0x1c, 0),
        (MMU0, 0),
        (MMU1 + 0x1c, 0),
        (MMU1, 0),
        (MMU0 + 8, 3),
        (MMU1 + 8, 3),
        // overlay_single: OVL_CTRL, the port select (Esmart0 on VP0 as
        // found; PORT0_MUX 0, PORT1/2 none, PORT3 7) and its CFG_DONE, the
        // layer select (Esmart0, id 2, on layer 0, Cluster0 to layer 2), the
        // mixer and the delays
        v(0x600, 0),
        v(PORT_SEL, 0xa054_7880),
        v(CFG_DONE, 0x0001_8001),
        v(LAYER_SEL, 0x7651_3042),
        v(0x6c0, 0),
        v(0x6f0, 0),
        v(0x6f8, 23),
        // window_show: AXI bus 0, read ids 0x0a/0x0b, stride 1024 words,
        // our address, unscaled, no colour key, 1024x768 at (0, 0), no CSC,
        // ARGB8888 enabled last, then CFG_DONE for VP0
        v(E0 + 0x08, 0),
        v(E0 + 0x04, 0xb0a0),
        v(E0 + 0x1c, 1024),
        v(E0 + 0x14, FB as u32),
        v(E0 + 0x34, 0),
        v(E0 + 0x30, 0x44),
        v(E0 + 0xd0, 0),
        v(E0 + 0x20, 0x02ff_03ff),
        v(E0 + 0x24, 0x02ff_03ff),
        v(E0 + 0x28, 0),
        v(E0, 0),
        v(E0 + 0x10, 1),
        v(CFG_DONE, 0x0001_8001),
        // the frame start that takes it
        v(VP0_INT_CLR, 0x0001_0001),
    ];
    w.shrink_to_fit();
    w
}

#[test]
fn the_dump_adopts_with_exactly_the_expected_writes_in_order() {
    let soc = Soc::dump();
    let a = run(&soc, FB);
    assert_eq!(a.stop, Stop::None as u32, "{a:?}");
    assert_eq!(soc.writes(), expected_writes());
    assert_eq!(soc.violations(), Vec::<String>::new());
    // what was found is the dump's
    assert_eq!(a.version, 0x4017_6786);
    assert_eq!(a.dsp_ctrl, [0xf, 0x8000_000f, 0x8000_000f, 0x8000_000f]);
    assert_eq!(a.timing, [0x0540_0088, 0x0128_0528, 0x0326_0006, 0x0023_0323]);
    assert_eq!((a.dsp_if_en, a.pd_ctrl), (0x0004_0028, 0x70));
    assert_eq!((a.layer_sel, a.port_sel), (0x7651_3240, 0xa054_7873));
    assert_eq!((a.cluster0, a.esmart0_ctrl), ([1, 1], 0));
    assert_eq!((a.mmu_status, a.mmu_dte), ([0x19, 0x19], [0x0ad1_7100, 0x0ad1_7100]));
    assert_eq!(a.cru_gate52, 0x400);
    // what it did
    // VP0 running; Cluster0-3 (bits 0-3) and Esmart1-3 (bits 5-7) off
    assert_eq!((a.running, a.off), (0b0001, 0xef));
    assert_eq!((a.mmu_status_now, a.mmu_timeout), ([0x18, 0x18], 0));
    assert_eq!((a.ovl_layer_sel, a.ovl_port_sel), (0x7651_3042, 0xa054_7880));
    assert_eq!((a.fb, a.stride), (FB as u32, 4096));
    // the model takes VP0's bit at the frame start (U8 is the board's answer)
    assert_eq!((a.cfg_done_written & 1, a.cfg_done_after & 1), (1, 0));
    assert_eq!(a.shown, [FB as u32, 1, 0xa054_7880, 0x7651_3042]);
    // the registers are in that state
    assert_eq!(soc.reg(MMU0 + 4) & 1, 0);
    assert_eq!(soc.reg(MMU1 + 4) & 1, 0);
    assert_eq!(soc.reg(VOP + VP0_DSP_CTRL), 0xf);
}

/// Each read-only refusal: its stop, the value it is about, and not one
/// write. A domain or clock refusal also reads nothing of the VOP.
#[test]
fn each_state_it_does_not_know_is_refused_by_name_before_any_write() {
    let pmu_repair = PMU + 0x290;
    let gate52 = CRU + 0x8d0;
    // name, the dump word changed, the stop, its index and detail
    type Case = (&'static str, (usize, u32), Stop, u32, u32);
    let cases: [Case; 12] = [
        ("PD_VOP off", (pmu_repair, 0xfad4_0001), Stop::VopDomain, 0, 0xfad4_0001),
        ("PD_VOP idle requested", (PMU + 0x10c, 0x0000_a3cf), Stop::VopDomain, 0, 0xfad5_0001),
        ("ACLK_VOP gated", (gate52, 0x0600), Stop::VopClock, 9, 0x0600),
        ("HCLK_VOP gated", (gate52, 0x0500), Stop::VopClock, 8, 0x0500),
        ("not the RK3588's VOP", (VOP + 0x004, 0x4015_8f45), Stop::Version, 0, 0x4015_8f45),
        ("VP0 in standby", (VOP + VP0_DSP_CTRL, 0x8000_000f), Stop::Standby, 0, 0x8000_000f),
        ("VP0 in P888", (VOP + VP0_DSP_CTRL, 0x0000_0000), Stop::VpState, 0, 0),
        ("1920x1080's htotal", (VOP + 0xc48, 0x0898_002c), Stop::Mode, 0, 0x0898_002c),
        ("another vertical start", (VOP + 0xc54, 0x0024_0324), Stop::Mode, 3, 0x0024_0324),
        ("HDMI0 on VP1", (VOP + 0x028, 0x0005_0028), Stop::Route, 0, 0x0005_0028),
        ("HDMI0 disabled", (VOP + 0x028, 0x0004_0020), Stop::Route, 0, 0x0004_0020),
        ("Esmart power off", (VOP + 0x034, 0xf0), Stop::EsmartPower, 0, 0xf0),
    ];
    for (name, edit, stop, index, detail) in cases {
        let soc = Soc::with(Faults::default(), &[edit]);
        let a = run(&soc, FB);
        assert_eq!((a.stop, a.index, a.detail), (stop as u32, index, detail), "{name}: {a:?}");
        assert_eq!(soc.writes(), vec![], "{name}");
        if matches!(stop, Stop::VopDomain | Stop::VopClock) {
            assert!(!soc.vop_accessed(), "{name}: the VOP was read with its domain or clock off");
        }
    }
}

/// The IOMMU that does not stop is a named failure: nothing after it is
/// written (no overlay, no window), and the record has the set and its
/// STATUS.
#[test]
fn an_iommu_that_keeps_paging_stops_the_adopt_by_name() {
    let soc = Soc::with(Faults { paging_stuck: true, ..Faults::default() }, &[]);
    let a = run(&soc, FB);
    assert_eq!(a.stop, Stop::Iommu as u32);
    assert_eq!((a.refusal, a.index, a.detail), (rk_iommu::Refusal::PagingStuck as u32, 0, 0x19));
    assert_eq!(a.mmu_status_now, [0x19, 0x19]);
    let w = soc.writes();
    assert!(
        !w.iter().any(|(addr, _)| *addr == VOP + PORT_SEL || (VOP + E0..VOP + E0 + 0x100).contains(addr)),
        "{w:x?}"
    );
    // Linux's windows were off before paging was asked to stop
    assert_eq!(soc.violations(), Vec::<String>::new());
}

/// No pixel clock: the take-over's frame wait times out, and the IOMMU is
/// not touched (Linux's Cluster0 may still be in force).
#[test]
fn a_port_that_times_no_frame_stops_before_the_iommu() {
    let soc = Soc::with(Faults { no_frames: true, ..Faults::default() }, &[]);
    let a = run(&soc, FB);
    assert_eq!((a.stop, a.refusal), (Stop::TakeOver as u32, vop2::Refusal::FrameTimeout as u32));
    assert!(!soc.writes().iter().any(|(addr, _)| (MMU0..MMU1 + 0x100).contains(addr)));
}

/// Esmart0 enabled by Linux: its address is Linux's; nothing is written.
#[test]
fn an_esmart0_already_enabled_is_refused_before_any_write() {
    let soc = Soc::with(Faults::default(), &[(VOP + E0 + 0x10, 1)]);
    let a = run(&soc, FB);
    assert_eq!((a.stop, a.refusal, a.detail), (Stop::TakeOver as u32, vop2::Refusal::KeepEnabled as u32, 1));
    assert_eq!(soc.writes(), vec![]);
}

/// A buffer at or above 4 GiB cannot be shown with the IOMMU off.
#[test]
fn a_buffer_above_4_gib_is_refused() {
    let soc = Soc::dump();
    let a = run(&soc, 0x1_0000_0000);
    assert_eq!((a.stop, a.refusal), (Stop::Window as u32, vop2::Refusal::WindowAbove4G as u32));
    assert_eq!(soc.reg(VOP + E0 + 0x10) & 1, 0, "Esmart0 stays off");
}

/// Section 5's read-backs after the adopt: frames run, no underflow or bus
/// error, the frame interval of 65 MHz, our address, and the GRF and HDMI
/// controller words the dump has.
#[test]
fn the_status_after_the_adopt_reads_section_5() {
    let soc = Soc::dump();
    assert_eq!(run(&soc, FB).stop, 0);
    let s = status(&soc, &B, 100_000, 60, &mut || soc.now(), &mut |us| soc.advance(us));
    assert_eq!((s.refusal, s.frame_start, s.underflow, s.bus_error), (0, 1, 0, [0, 0]));
    assert_eq!(s.frames, 60);
    // the polls are 100 us apart: the interval is right to one step
    assert!(s.frames_us.abs_diff(60 * FRAME_US as u32) <= 100, "{}", s.frames_us);
    assert_eq!(s.vop[0], 0x4017_6786);
    assert_eq!(s.vop[10], FB as u32);
    assert_eq!(s.vop[9] & 1, 1);
    assert_eq!((s.soc_status1 >> 16 & 1, s.phy_status & 0xe), (1, 0xe));
    assert_eq!(s.vo1_usable, 1);
    assert_eq!(s.cmu, [0x255, 0x3f7a, 0x3f79]);
    assert_eq!(s.monitor[2], 0x0540_0400);
    assert_eq!(soc.violations(), Vec::<String>::new());
    // only the VP0 event clears are written
    let after: Vec<_> = soc.writes().split_off(expected_writes().len());
    assert!(
        after.iter().all(|w| *w == (VOP + VP0_INT_CLR, 0x0001_0001) || *w == (VOP + VP0_INT_CLR, 0x0011_0011)),
        "{after:x?}"
    );
}

/// PD_VO1 not usable: the HDMI controller is not read.
#[test]
fn the_hdmi_controller_is_not_read_with_pd_vo1_off() {
    // REPAIR bit 18 (VO1) clear, bit 16 (VOP) kept
    let soc = Soc::with(Faults::default(), &[(PMU + 0x290, 0xfad1_0001)]);
    assert_eq!(run(&soc, FB).stop, 0);
    let s = status(&soc, &B, 100_000, 1, &mut || soc.now(), &mut |us| soc.advance(us));
    assert_eq!((s.vo1_usable, s.cmu, s.monitor), (0, [0; 3], [0; 7]));
    assert!(!soc.ops().iter().any(|o| matches!(o, Op::R(a, _) if (HDMI..HDMI + 0x1000).contains(a))));
}

#[test]
fn the_mode_is_the_dumps() {
    let t = vop2::vp_timing(&MODE, Vp::Vp0).unwrap();
    let d = parse_dump(DUMP);
    assert_eq!(
        [t.htotal_hs_end, t.hact_st_end, t.vtotal_vs_end, t.vact_st_end, t.dsp_ctrl],
        [d[&(VOP + 0xc48)], d[&(VOP + 0xc4c)], d[&(VOP + 0xc50)], d[&(VOP + 0xc54)], d[&(VOP + 0xc00)]]
    );
    assert_eq!((MODE.htotal(), MODE.vtotal(), MODE.refresh_mhz(MODE.pixel_clock_hz)), (1344, 806, 60_003));
    assert_eq!(FB_BYTES, 1024 * 768 * 4);
    assert_eq!(FB_STRIDE % 64, 0);
}

#[test]
fn the_pattern_has_four_quadrants_and_a_white_border() {
    let (w, h, stride) = (1024, 768, 1024);
    let mut fb = vec![0u32; stride * h];
    assert!(draw_pattern(&mut fb, w, h, stride));
    let at = |x: usize, y: usize| fb[y * stride + x];
    for (x, y) in [(0, 0), (1023, 0), (0, 767), (1023, 767), (512, 0), (0, 384), (1023, 400), (300, 767)] {
        assert_eq!(at(x, y), WHITE, "({x}, {y})");
    }
    assert_eq!([at(1, 1), at(511, 383), at(512, 1), at(1022, 383)], [RED, RED, GREEN, GREEN]);
    assert_eq!([at(1, 384), at(511, 766), at(512, 384), at(1022, 766)], [BLUE, BLUE, YELLOW, YELLOW]);
    // XRGB8888 in memory: B, G, R, X
    assert_eq!(RED.to_le_bytes(), [0, 0, 0xff, 0]);
    // a stride wider than the line: the pixels past it are not touched
    let mut fb = vec![7u32; 8 * 4];
    assert!(draw_pattern(&mut fb, 6, 4, 8));
    assert_eq!((fb[6], fb[7], fb[5]), (7, 7, WHITE));
    // too short, or a line wider than the stride: nothing written
    let mut fb = vec![7u32; 10];
    assert!(!draw_pattern(&mut fb, 4, 4, 4));
    assert!(!draw_pattern(&mut fb, 5, 1, 4));
    assert!(fb.iter().all(|p| *p == 7));
}
