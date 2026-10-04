// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of soc::am62p::dss (and ip::ti_dss7 through it) against the
//! AM62P model (crate::model_am62p).

use super::*;
use crate::errno::{EIO, ENODATA};
use crate::mmio::Mmio;
use crate::model::TestEnv;
use crate::model_am62p::{Access, Am62pFaults, Am62pModel, COMMON, OVR, VID, VIDL1, VP};
use crate::soc::am62p::regs::{common as c, ovr as o, vid as v, vp as p};

const SAFETY0: u64 = 0x9000_0000;
const SAFETY1: u64 = 0x9030_0000;

/// The board devicetree's &dss1 (Riverdi RVT70, 1024x600, VP2 -> DSI).
fn board_cfg(irq: bool) -> DssConfig {
    DssConfig {
        pixel_clock_hz: 51_200_000,
        hactive: 1024,
        hfp: 160,
        hsync: 5,
        hbp: 160,
        vactive: 600,
        vfp: 12,
        vsync: 10,
        vbp: 23,
        flags: flags::HSYNC_LOW | flags::VSYNC_LOW | if irq { flags::HAS_IRQ } else { 0 },
        vp_index: 1,
        safety_width: 1024,
        safety_height: 600,
    }
}

fn data(m: &Am62pModel) -> DssData {
    DssData {
        common: m.base(COMMON),
        vid: m.base(VID),
        vidl1: m.base(VIDL1),
        ovr: m.base(OVR),
        vp: m.base(VP),
        safety_buf: [SAFETY0, SAFETY1],
        ..Default::default()
    }
}

fn up(f: Am62pFaults) -> (Am62pModel, TestEnv, DssConfig, DssData) {
    let m = Am62pModel::with_faults(f);
    m.set_port(1);
    let e = TestEnv::default();
    let cfg = board_cfg(true);
    let d = data(&m);
    dss_init(&m, &e, &cfg, &d).expect("init");
    (m, e, cfg, d)
}

fn app(seq: u32, addr: u64) -> AppPlane {
    AppPlane { seq, plane: Plane { addr, width: 1024, height: 600, stride_bytes: 4096, format: vid::FMT_XRGB8888 } }
}

fn index_of(log: &[Access], pred: impl Fn(&Access) -> bool) -> usize {
    log.iter().position(pred).expect("access in the log")
}

#[test]
fn init_resets_then_programs_the_port_and_the_safety_layer() {
    let (m, e, _cfg, _d) = up(Am62pFaults::default());
    let log = m.accesses();
    // The soft reset comes before every other DSS write, and completes.
    let reset = index_of(&log, |a| matches!(*a, Access::W(COMMON, 0x8, val) if val & 2 != 0));
    let first_other = index_of(&log, |a| matches!(*a, Access::W(b, ..) if b != COMMON));
    assert!(reset < first_other);
    assert_eq!(m.peek(COMMON, c::DSS_SYSCONFIG) & c::SYSCONFIG_SOFTRESET, 0);
    // FIFOs per BUFSIZE A00h: high 9FFh, low 9F8h, preload = low, MFLAG 1/4..1/2.
    for b in [VID, VIDL1] {
        assert_eq!(m.peek(b, v::BUF_THRESHOLD), (0x9ff << 16) | 0x9f8);
        assert_eq!(m.peek(b, v::PRELOAD), 0x9f8);
        assert_eq!(m.peek(b, v::MFLAG_THRESHOLD), (0x500 << 16) | 0x280);
        assert_eq!(m.peek(b, v::ATTRIBUTES) & v::ATTR_BUFPRELOAD, 0);
    }
    assert_eq!(m.peek(COMMON, c::DISPC_GLOBAL_MFLAG_ATTRIBUTE), 2);
    // CBA_CFG is the secure host's: never written.
    assert!(!m.writes().iter().any(|w| w.0 == COMMON && w.1 == 0x9c));
    // The port's timings (the display_calc values) and 24 data lines.
    assert_eq!(m.peek(VP, p::TIMING_H), 4 | (159 << 8) | (159 << 20));
    assert_eq!(m.peek(VP, p::TIMING_V), 9 | (12 << 8) | (23 << 20));
    assert_eq!(m.peek(VP, p::SIZE_SCREEN), 1023 | (599 << 16));
    assert_eq!(m.peek(VP, p::POL_FREQ), (1 << 18) | (1 << 17) | (1 << 16) | (1 << 13) | (1 << 12));
    assert_eq!(m.peek(VP, p::CONTROL) & p::CTRL_DATALINES, 3 << 8);
    assert_eq!(m.peek(VP, p::CONTROL) & p::CTRL_ENABLE, 0, "the port stays off");
    // Safety layer: VIDL1, ARGB8888, buffer 0, on layer 1, enabled.
    assert_eq!(m.peek(VIDL1, v::BA_0), SAFETY0 as u32);
    assert_eq!(m.peek(VIDL1, v::BA_1), SAFETY0 as u32);
    assert_eq!(m.peek(VIDL1, v::PICTURE_SIZE), 1023 | (599 << 16));
    assert_eq!(m.peek(VIDL1, v::ROW_INC), 1);
    let a = m.peek(VIDL1, v::ATTRIBUTES);
    assert_eq!((a & v::ATTR_FORMAT) >> 1, vid::FMT_ARGB8888);
    assert_eq!(a & v::ATTR_ENABLE, 1);
    assert_eq!(m.peek(OVR, o::ATTRIBUTES_1), (1 << 1) | 1);
    // VIDL1 has no scaler: VID_SIZE is never written on it.
    assert!(!m.writes().iter().any(|w| w.0 == VIDL1 && w.1 == v::SIZE));
    // Application layer 0 = VID, disabled.
    assert_eq!(m.peek(OVR, o::ATTRIBUTES_0), 0);
    assert_eq!(m.peek(VID, v::ATTRIBUTES) & v::ATTR_ENABLE, 0);
    // Interrupts of VP2 and both pipelines.
    assert_eq!(m.peek(COMMON, c::VP_IRQENABLE_1), c::VP_IRQ_VSYNC | c::VP_IRQ_SYNCLOST);
    assert_eq!(m.peek(COMMON, c::VID_IRQENABLE_0), 1);
    assert_eq!(m.peek(COMMON, c::VID_IRQENABLE_1), 1);
    assert_eq!(m.peek(COMMON, c::DISPC_IRQENABLE_SET), (1 << 1) | (1 << 4) | (1 << 5));
    assert!(e.lines.borrow().iter().any(|l| l.1.contains("DSS rev 0x64040901")));
}

#[test]
fn reset_failures_are_reported() {
    let m = Am62pModel::with_faults(Am62pFaults { dss_por_not_done: true, ..Default::default() });
    let e = TestEnv::default();
    assert_eq!(dss_init(&m, &e, &board_cfg(true), &data(&m)), Err(-EIO));
    assert!(m.writes().is_empty(), "nothing written to a DSS that is not out of reset");
    let m = Am62pModel::with_faults(Am62pFaults { dss_reset_stuck: true, ..Default::default() });
    assert_eq!(dss_init(&m, &e, &board_cfg(true), &data(&m)), Err(-EIO));
    assert_eq!(m.writes().len(), 1, "only the SOFTRESET write");
    assert!(e.lines.borrow().iter().any(|l| l.1.contains("soft reset did not complete")));
}

#[test]
fn bad_configuration_is_refused_before_any_write() {
    let m = Am62pModel::new();
    let e = TestEnv::default();
    let bad = DssConfig { hactive: 1020, ..board_cfg(true) };
    assert_eq!(dss_init(&m, &e, &bad, &data(&m)), Err(-EINVAL));
    let mut d = data(&m);
    d.vp = 0;
    assert_eq!(dss_init(&m, &e, &board_cfg(true), &d), Err(-ENODEV));
    let d = DssData { safety_buf: [SAFETY0, 0], ..data(&m) };
    assert_eq!(dss_init(&m, &e, &board_cfg(true), &d), Err(-EINVAL));
    assert!(m.writes().is_empty());
}

#[test]
fn commits_while_the_port_is_off_set_no_go() {
    let (m, _e, _cfg, d) = up(Am62pFaults::default());
    m.clear_log();
    set_app_buffer(&m, &d, &app(1, 0xa000_0000)).unwrap();
    enable_app_plane(&m, &d, true).unwrap();
    assert!(!m.writes().iter().any(|w| w.0 == VP));
    // The port off: what was written is what the hardware uses.
    assert_eq!(m.active(VID, v::BA_0), 0xa000_0000);
    assert_eq!(d.stats.app_frames.load(SeqCst), 1);
}

#[test]
fn go_is_shared_and_never_waited_for() {
    let (m, e, cfg, d) = up(Am62pFaults::default());
    set_app_buffer(&m, &d, &app(1, 0xa000_0000)).unwrap();
    enable_app_plane(&m, &d, true).unwrap();
    output_enable(&m, &d, true);
    m.vsync();
    assert_eq!(dss_isr(&m, &cfg, &d), event::VSYNC);
    // Frame 2: the application commits, then the safety layer flips before
    // the vsync: one GO write, the flip rides along with it.
    m.clear_log();
    let waited = *e.waited_us.borrow();
    set_app_buffer(&m, &d, &app(2, 0xa100_0000)).unwrap();
    assert!(safety_back_free(&d));
    safety_flip(&m, &d);
    let go_writes = m.writes().iter().filter(|w| w.0 == VP && w.1 == p::CONTROL && w.2 & p::CTRL_GOBIT != 0).count();
    assert_eq!(go_writes, 1);
    assert_eq!(d.stats.go_waits.load(SeqCst), 1);
    assert_eq!(*e.waited_us.borrow(), waited, "a commit never waits");
    // Before the latch: the hardware still scans frame 1 and safety buffer 0;
    // the back buffer (0) must not be drawn into.
    assert_eq!(m.active(VID, v::BA_0), 0xa000_0000);
    assert_eq!(m.active(VIDL1, v::BA_0), SAFETY0 as u32);
    assert!(!safety_back_free(&d));
    assert_eq!(d.safety_back(), SAFETY0);
    assert_eq!(d.app_seq_shown.load(SeqCst), 1);
    // The latch.
    m.vsync();
    assert_eq!(dss_isr(&m, &cfg, &d), event::VSYNC);
    assert_eq!(m.active(VID, v::BA_0), 0xa100_0000);
    assert_eq!(m.active(VIDL1, v::BA_0), SAFETY1 as u32);
    assert_eq!(d.app_seq_shown.load(SeqCst), 2);
    assert!(safety_back_free(&d));
    assert_eq!(d.safety_front_addr(), SAFETY1);
    assert_eq!(d.stats.safety_flips.load(SeqCst), 1);
    assert_eq!(d.stats.vsyncs.load(SeqCst), 2);
}

#[test]
fn a_vsync_that_does_not_consume_go_settles_nothing() {
    let (m, _e, cfg, d) = up(Am62pFaults { go_stuck: true, ..Default::default() });
    set_app_buffer(&m, &d, &app(1, 0xa000_0000)).unwrap();
    output_enable(&m, &d, true);
    set_app_buffer(&m, &d, &app(2, 0xa100_0000)).unwrap();
    safety_flip(&m, &d);
    m.vsync();
    assert_eq!(dss_isr(&m, &cfg, &d), event::VSYNC);
    assert_eq!(d.app_seq_shown.load(SeqCst), 0);
    assert!(!safety_back_free(&d));
    assert_eq!(d.app_commit_pending.load(SeqCst), 1);
}

/// An Mmio that runs the vsync interrupt in the middle of the N-th write to
/// the VID pipeline, as the hardware may.
struct VsyncDuringWrite<'a> {
    m: &'a Am62pModel,
    cfg: &'a DssConfig,
    d: &'a DssData,
    at: core::cell::Cell<u32>,
}

impl Mmio for VsyncDuringWrite<'_> {
    fn read32(&self, addr: usize) -> u32 {
        self.m.read32(addr)
    }
    fn write32(&self, addr: usize, val: u32) {
        self.m.write32(addr, val);
        if addr >= self.m.base(VID) && addr < self.m.base(VID) + 0x1000 {
            let n = self.at.get();
            if n == 1 {
                self.m.vsync();
                dss_isr(self.m, self.cfg, self.d);
            }
            self.at.set(n.saturating_sub(1));
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
fn a_vsync_during_a_commit_writes_it_again() {
    let (m, _e, cfg, d) = up(Am62pFaults::default());
    set_app_buffer(&m, &d, &app(1, 0xa000_0000)).unwrap();
    output_enable(&m, &d, true);
    m.vsync();
    dss_isr(&m, &cfg, &d);
    m.clear_log();
    let hw = VsyncDuringWrite { m: &m, cfg: &cfg, d: &d, at: core::cell::Cell::new(3) };
    set_app_buffer(&hw, &d, &app(2, 0xa100_0000)).unwrap();
    // The BA write happened twice: once torn, once after the vsync.
    let ba = m.writes().iter().filter(|w| w.0 == VID && w.1 == v::BA_0).count();
    assert_eq!(ba, 2);
    // The second pass armed GO again: the next vsync latches the whole frame.
    assert!(m.peek(VP, p::CONTROL) & p::CTRL_GOBIT != 0);
    m.vsync();
    dss_isr(&m, &cfg, &d);
    assert_eq!(m.active(VID, v::BA_0), 0xa100_0000);
    assert_eq!(d.app_seq_shown.load(SeqCst), 2);
}

#[test]
fn the_isr_counts_and_acknowledges_exactly_what_it_read() {
    let (m, _e, cfg, d) = up(Am62pFaults::default());
    output_enable(&m, &d, true);
    m.sync_lost();
    m.underflow(0);
    m.underflow(1);
    m.clear_log();
    assert_eq!(dss_isr(&m, &cfg, &d), event::SYNC_LOST | event::UNDERFLOW);
    assert_eq!(d.stats.sync_lost.load(SeqCst), 1);
    assert_eq!(d.stats.underflows.load(SeqCst), 2);
    assert_eq!(m.peek(COMMON, c::VP_IRQSTATUS_1), 0);
    assert_eq!(m.peek(COMMON, c::VID_IRQSTATUS_0), 0);
    assert_eq!(m.peek(COMMON, c::VID_IRQSTATUS_1), 0);
    let w = m.writes();
    assert!(w.contains(&(COMMON, c::VP_IRQSTATUS_1, c::VP_IRQ_SYNCLOST)));
    assert_eq!(w.last(), Some(&(COMMON, c::DISPC_IRQ_EOI, 1)));
    // Nothing pending: nothing reported, nothing counted.
    assert_eq!(dss_isr(&m, &cfg, &d), 0);
    assert_eq!(d.stats.vsyncs.load(SeqCst), 0);
}

#[test]
fn polling_without_an_interrupt_line() {
    let m = Am62pModel::new();
    m.set_port(1);
    let e = TestEnv::default();
    let cfg = board_cfg(false);
    let d = data(&m);
    dss_init(&m, &e, &cfg, &d).unwrap();
    assert!(!m.writes().iter().any(|w| w.0 == COMMON && w.1 == c::DISPC_IRQENABLE_SET));
    output_enable(&m, &d, true);
    assert!(!poll_vsync(&m, &cfg, &d));
    m.vsync();
    assert!(poll_vsync(&m, &cfg, &d));
    assert!(!poll_vsync(&m, &cfg, &d), "taken");
    assert_eq!(d.stats.vsyncs.load(SeqCst), 1);
}

#[test]
fn application_plane_checks_and_layout() {
    let (m, _e, _cfg, d) = up(Am62pFaults::default());
    assert_eq!(enable_app_plane(&m, &d, true), Err(-ENODATA));
    for bad in [
        AppPlane { plane: Plane { format: 0x16, ..app(1, 0xa000_0000).plane }, ..app(1, 0) },
        app(1, 0),
        AppPlane { plane: Plane { width: 0, ..app(1, 0xa000_0000).plane }, ..app(1, 0) },
        AppPlane { plane: Plane { stride_bytes: 4000, ..app(1, 0xa000_0000).plane }, ..app(1, 0) },
        AppPlane { plane: Plane { addr: 0xa000_0002, ..app(1, 0).plane }, ..app(1, 0) },
    ] {
        assert_eq!(set_app_buffer(&m, &d, &bad), Err(-EINVAL), "{bad:?}");
    }
    assert_eq!(d.stats.app_frames.load(SeqCst), 0);
    assert_eq!(d.app_commit_pending.load(SeqCst), 0, "a refused plane is not pending");
    assert!(!m.writes().iter().any(|w| w.0 == VID && w.1 == v::BA_0), "nor written");
    // A padded RGB565 buffer above 4 GiB: ROW_INC skips the padding, BA_EXT
    // carries bits 47:32.
    let pl = AppPlane {
        seq: 7,
        plane: Plane { addr: 0x8_8000_0000, width: 800, height: 480, stride_bytes: 1664, format: vid::FMT_RGB565 },
    };
    set_app_buffer(&m, &d, &pl).unwrap();
    assert_eq!(m.peek(VID, v::BA_0), 0x8000_0000);
    assert_eq!(m.peek(VID, v::BA_EXT_0), 8);
    assert_eq!(m.peek(VID, v::BA_EXT_1), 8);
    assert_eq!(m.peek(VID, v::ROW_INC), 1 + 1664 - 1600);
    assert_eq!(m.peek(VID, v::PIXEL_INC), 1);
    assert_eq!(m.peek(VID, v::SIZE), 799 | (479 << 16));
    assert_eq!(m.peek(VID, v::PICTURE_SIZE), 799 | (479 << 16));
    assert_eq!((m.peek(VID, v::ATTRIBUTES) & v::ATTR_FORMAT) >> 1, vid::FMT_RGB565);
    assert_eq!(m.peek(VID, v::ATTRIBUTES) & v::ATTR_RESIZEENABLE, 0);
    enable_app_plane(&m, &d, true).unwrap();
    assert_eq!(m.peek(OVR, o::ATTRIBUTES_0), 1, "VID (0) on layer 0, enabled");
    assert_eq!(d.app_enabled.load(SeqCst), 1);
    enable_app_plane(&m, &d, false).unwrap();
    assert_eq!(m.peek(VID, v::ATTRIBUTES) & v::ATTR_ENABLE, 0);
}

#[test]
fn premultiplied_alpha_is_the_safety_layers_only() {
    let (m, _e, _cfg, d) = up(Am62pFaults::default());
    safety_premultiplied(&m, &d, true);
    assert_ne!(m.peek(VIDL1, v::ATTRIBUTES) & v::ATTR_PREMULTIPLYALPHA, 0);
    set_app_buffer(&m, &d, &app(1, 0xa000_0000)).unwrap();
    assert_ne!(m.peek(VIDL1, v::ATTRIBUTES) & v::ATTR_PREMULTIPLYALPHA, 0, "untouched by an app commit");
    assert_eq!(m.peek(VID, v::ATTRIBUTES) & v::ATTR_PREMULTIPLYALPHA, 0);
    safety_premultiplied(&m, &d, false);
    assert_eq!(m.peek(VIDL1, v::ATTRIBUTES) & v::ATTR_PREMULTIPLYALPHA, 0);
}

#[test]
fn safety_flips_alternate_the_two_buffers() {
    let (m, _e, cfg, d) = up(Am62pFaults::default());
    output_enable(&m, &d, true);
    assert_eq!(d.safety_back(), SAFETY1);
    for (i, want) in [SAFETY1, SAFETY0, SAFETY1].into_iter().enumerate() {
        safety_flip(&m, &d);
        m.vsync();
        dss_isr(&m, &cfg, &d);
        assert_eq!(u64::from(m.active(VIDL1, v::BA_0)), want, "flip {i}");
        assert_eq!(d.safety_front_addr(), want);
    }
    output_enable(&m, &d, false);
    assert_eq!(d.output_enabled.load(SeqCst), 0);
    assert_eq!(m.peek(VP, p::CONTROL) & p::CTRL_ENABLE, 0);
}

#[test]
fn the_ffi_entry_points_reach_the_same_code() {
    // Layout is pinned by const asserts in ffi::am62p; here: the flags of
    // the C config map to the mode as the devicetree means them.
    let cfg = DssConfig { flags: flags::DE_LOW | flags::PIXCLK_NEGEDGE, ..board_cfg(true) };
    let mode = cfg.mode();
    assert!(mode.hsync_high && mode.vsync_high && !mode.de_high);
    assert!(cfg.output().data_on_falling_edge);
}
