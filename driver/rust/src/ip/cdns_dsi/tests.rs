// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of ip::cdns_dsi and ip::cdns_dphy against the AM62P model
//! (crate::model_am62p).

use super::*;
use crate::model::TestEnv;
use crate::model_am62p::{Access, Am62pFaults, Am62pModel, DPHY, DSI, WRAP};
use crate::soc::am62p::regs::{dphy as pr, wiz};

/// MIPI DSI data types (MIPI DSI v1.3 Table 16; Zephyr's mipi_dsi.h
/// MIPI_DSI_PACKED_PIXEL_STREAM_24 / _16).
const PACKED_24: u32 = 0x3e;
const PACKED_16: u32 = 0x0e;

/// The board: RVT70 1024x600 at 51.2 MHz, RGB888 over four lanes, sync
/// events (the devicetree's `burst-mode`), no EoT, 25 MHz reference, 25 MHz
/// PSM clock.
fn rvt70() -> Request {
    Request {
        mode: VideoMode {
            pixel_clock_hz: 51_200_000,
            hactive: 1024,
            hfront: 160,
            hsync: 5,
            hback: 160,
            vactive: 600,
            vfront: 12,
            vsync: 10,
            vback: 23,
            hsync_high: true,
            vsync_high: true,
            de_high: true,
        },
        bpp: 24,
        lanes: 4,
        sync_pulse: false,
        eot: false,
        datatype: PACKED_24,
        pll_ref_hz: 25_000_000,
        psm_hz: 25_000_000,
    }
}

fn bases(m: &Am62pModel) -> Bases {
    Bases { dsi: m.base(DSI), wrap: m.base(WRAP), dphy: m.base(DPHY) }
}

/// Ported from driver/tests/display_calc (test_cdns_dsi_burst_rgb888_4lanes),
/// with the two places where the TRM's rules give other values than
/// cdns_dsi_calc did.
#[test]
fn timings_rvt70_against_display_calc() {
    let t = Timings::from_mode(&rvt70()).unwrap();
    assert_eq!(t.hact, 1024 * 3);
    assert_eq!(t.hfp, 160 * 3 - 6);
    assert_eq!(t.hbp, (160 + 5) * 3 - 16);
    assert_eq!(t.hsa, 0);
    assert_eq!(t.line, t.hact + t.hfp + 6 + t.hbp + 16);
    assert_eq!(t.line, 1349 * 3);
    assert_eq!(t.vact, 600);
    // display_calc: VSA vsync + 1, VBP vbp - 1, VFP vfp (11, 22, 12).
    // 12.9.2.7.9.1: VSA = DPI VSA, VBP = DPI VBP, VFP one less than DPI's.
    assert_eq!((t.vsa, t.vbp, t.vfp), (10, 23, 11));
    assert_eq!(t.rate, 51_200_000 * 24 / 4); // 307.2 Mbit/s per lane
    assert_eq!(t.blkline_event, t.line - 10);
    assert_eq!(t.blkline_pulse, t.line - 20);
    assert_eq!(t.max_line, t.line - 16);
    // display_calc: ceil(line / lanes) = 1012; the TRM's event-mode LP line
    // takes the HSS packet off (1011). Only used in LP blanking modes.
    assert_eq!(t.line_duration, 1011);
    assert_eq!(t.wakeup, 4047 / 4 / 10);
}

/// The TRM's worked example (12.9.2.7.9.2-.3): HSA 12, HBP 12, HACT 1920,
/// HFP 24, 16 bpp, four lanes, 3936 bytes / 984 byte clocks a line.
#[test]
fn timings_match_the_trms_worked_example() {
    let mut req = rvt70();
    req.mode = VideoMode {
        pixel_clock_hz: 148_500_000,
        hactive: 1920,
        hfront: 24,
        hsync: 12,
        hback: 12,
        vactive: 1080,
        vfront: 4,
        vsync: 5,
        vback: 36,
        ..req.mode
    };
    req.bpp = 16;
    req.datatype = PACKED_16;
    req.sync_pulse = true;
    let t = Timings::from_mode(&req).unwrap();
    assert_eq!((t.hsa, t.hbp, t.hact, t.hfp), (10, 12, 3840, 42));
    assert_eq!(t.line, 3936);
    assert_eq!(t.blkline_pulse, 3906);
    assert_eq!(t.line_duration, 978);
    // The VACT line adds up to HLINE x bpp/8 (the TRM's own check).
    assert_eq!(4 + (t.hsa + 6) + 4 + (t.hbp + 6) + (t.hact + 6) + (t.hfp + 6), t.line);

    req.sync_pulse = false;
    req.eot = true;
    let t = Timings::from_mode(&req).unwrap();
    // The TRM's text writes "- 12" and its example 24; its line sum needs 32.
    assert_eq!(t.hbp, 32);
    assert_eq!(4 + (t.hbp + 6) + (t.hact + 6) + (t.hfp + 6), t.line);
    assert_eq!(t.blkline_event, 3926);
    assert_eq!(4 + (4 + t.blkline_event + 2), t.line);
    assert_eq!(t.line_duration, 982);
    req.eot = false;
    assert_eq!(Timings::from_mode(&req).unwrap().line_duration, 983);
}

#[test]
fn timings_refuse_what_the_trm_rules_out() {
    let ok = rvt70();
    let bad = |f: &dyn Fn(&mut Request)| {
        let mut r = ok;
        f(&mut r);
        Timings::from_mode(&r)
    };
    assert_eq!(bad(&|r| r.bpp = 18), Err(-EINVAL));
    assert_eq!(bad(&|r| r.lanes = 0), Err(-EINVAL));
    assert_eq!(bad(&|r| r.lanes = 5), Err(-EINVAL));
    // HACT x bpp / 32 must be an integer.
    assert_eq!(bad(&|r| r.mode.hactive = 1022), Err(-EINVAL));
    // VSA at least 2 lines, the DSI's VFP at least 1.
    assert_eq!(bad(&|r| r.mode.vsync = 1), Err(-EINVAL));
    assert_eq!(bad(&|r| r.mode.vfront = 1), Err(-EINVAL));
    // Porches too short for the packets around them.
    assert_eq!(bad(&|r| r.mode.hfront = 1), Err(-EINVAL));
    // Sync pulses: HSA must hold its 14 bytes of packets.
    assert_eq!(
        bad(&|r| {
            r.sync_pulse = true;
            r.mode.hsync = 4;
        }),
        Err(-EINVAL)
    );
    assert!(bad(&|r| r.sync_pulse = true).is_ok());
}

#[test]
fn wait_burst_time_is_100ns_of_byte_clocks() {
    assert_eq!(wait_burst_time(307_812_500), 4); // 38.5 MHz byte clock
    assert_eq!(wait_burst_time(80_000_000), 1);
    assert_eq!(wait_burst_time(2_500_000_000), 15); // 31.25 -> the field's max
}

fn pos(log: &[Access], pred: impl Fn(&Access) -> bool) -> usize {
    log.iter().position(pred).expect("access in the log")
}

fn last(log: &[Access], pred: impl Fn(&Access) -> bool) -> usize {
    log.iter().rposition(pred).expect("access in the log")
}

fn wr_to(blk: &'static str, off: u32) -> impl Fn(&Access) -> bool {
    move |a| matches!(*a, Access::W(b, o, _) if b == blk && o == off)
}

fn attached(f: Am62pFaults) -> (Am62pModel, TestEnv, Result<Link, i32>) {
    let m = Am62pModel::with_faults(f);
    let e = TestEnv::default();
    let b = bases(&m);
    let wz = init(&m, &e, &b).expect("init");
    let r = attach(&m, &e, &b, wz, &rvt70());
    (m, e, r)
}

#[test]
fn init_identifies_quiesces_and_finds_the_wiz() {
    let m = Am62pModel::new();
    let e = TestEnv::default();
    assert_eq!(init(&m, &e, &bases(&m)), Ok(0xf00));
    for off in [r::MCTL_MAIN_DATA_CTL, r::MCTL_MAIN_EN, r::MCTL_MAIN_PHY_CTL, r::VID_MODE_STS_CTL] {
        assert!(m.writes().contains(&(DSI, off, 0)));
    }
    // The TRM's place for the WIZ, when that is where it is.
    let m = Am62pModel::with_faults(Am62pFaults { wiz_at_zero: true, ..Default::default() });
    assert_eq!(init(&m, &e, &bases(&m)), Ok(0));
    let m = Am62pModel::with_faults(Am62pFaults { no_wiz: true, ..Default::default() });
    assert_eq!(init(&m, &e, &bases(&m)), Err(-ENODEV));
    let m = Am62pModel::with_faults(Am62pFaults { foreign_dsi: true, ..Default::default() });
    assert_eq!(init(&m, &e, &bases(&m)), Err(-ENODEV));
    assert!(m.writes().is_empty(), "nothing written to something that is not the DSI host");
}

#[test]
fn attach_follows_the_trm_order() {
    let (m, _e, r) = attached(Am62pFaults::default());
    let l = r.expect("link up");
    assert_eq!((l.pll.ipdiv, l.pll.opdiv, l.pll.fbdiv, l.pll.band), (2, 8, 394, 5));
    let log = m.accesses();
    let link_en = pos(&log, |a| matches!(*a, Access::W(DSI, 0x4, v) if v & r::LINK_EN != 0));
    // MCTL_DPHY_* and the physical line before LINK_EN (12.9.2.7.1.3).
    for off in [r::MCTL_DPHY_TIMEOUT1, r::MCTL_DPHY_TIMEOUT2, r::MCTL_ULPOUT_TIME, r::MCTL_MAIN_PHY_CTL] {
        assert!(pos(&log, wr_to(DSI, off)) < link_en, "{off:#x}");
    }
    // PLL start with the link (Figure 12-551), then the D-PHY programmed
    // between the first two MCTL_DPHY_CFG0 values.
    let pll_start = pos(&log, |a| matches!(*a, Access::W(DSI, 0xc, v) if v == r::PLL_START));
    let cfg0: Vec<usize> =
        log.iter().enumerate().filter(|(_, a)| wr_to(DSI, r::MCTL_DPHY_CFG0)(a)).map(|(i, _)| i).collect();
    assert_eq!(cfg0.len(), 3);
    assert!(link_en < pll_start && pll_start < cfg0[0]);
    let first_dphy = pos(&log, |a| matches!(*a, Access::W(DPHY, ..)));
    let last_dphy = last(&log, |a| matches!(*a, Access::W(DPHY, ..)));
    assert!(cfg0[0] < first_dphy && last_dphy < cfg0[1]);
    // D-PHY: PSM, PWM and PLL before the common reset release, then band,
    // then the state machines, then lock and ready polled.
    let wz = 0xf00;
    let rst = pos(&log, wr_to(DPHY, wz + wiz::RST_CTRL));
    for off in [wz + wiz::PSM_FREQ, pr::CMN_DIG_TBIT10, wz + wiz::PLL_CTRL] {
        assert!(pos(&log, wr_to(DPHY, off)) < rst, "{off:#x}");
    }
    let band = pos(&log, wr_to(DPHY, pr::PCS_TX_DIG_TBIT0));
    let ssm = pos(&log, wr_to(DPHY, pr::CMN_DIG_TBIT2));
    assert!(rst < band && band < ssm && ssm < cfg0[1]);
    // PLL_LCK waited for between CFG0[1] and CFG0[2]; lanes after.
    let sts_clr = pos(&log, wr_to(DSI, r::MCTL_MAIN_STS_CLR));
    assert!(last_dphy < sts_clr && sts_clr < cfg0[1]);
    let lock_read = pos(&log, |a| matches!(*a, Access::R(DSI, 0x24, v) if v & 1 != 0));
    assert!(cfg0[1] < lock_read && lock_read < cfg0[2]);
    let lanes_en = pos(&log, |a| matches!(*a, Access::W(DSI, 0xc, v) if v & r::CKLANE_EN != 0));
    assert!(cfg0[2] < lanes_en);
    // Every VID_* before VID_MAIN_CTL, and the interface last.
    let main_ctl = pos(&log, wr_to(DSI, r::VID_MAIN_CTL));
    for off in [
        r::VID_HSIZE1,
        r::VID_HSIZE2,
        r::VID_VSIZE1,
        r::VID_VSIZE2,
        r::VID_BLKSIZE1,
        r::VID_BLKSIZE2,
        r::VID_VCA_SETTING2,
        r::VID_DPHY_TIME,
    ] {
        let i = pos(&log, wr_to(DSI, off));
        assert!(lanes_en < i && i < main_ctl, "{off:#x}");
    }
    let dpi = pos(&log, wr_to(WRAP, w::DPI_CONTROL));
    let if2 = last(&log, |a| matches!(*a, Access::W(DSI, 0xc, v) if v & r::IF2_EN != 0));
    assert!(main_ctl < dpi && dpi < if2);
    assert_eq!(if2, last(&log, |a| matches!(*a, Access::W(..))), "IF2_EN is the last write");
}

/// The register image after attach for the board's mode, against what the
/// replaced C driver (dsi_cdns_j721e.c + display_calc.c) wrote. Every
/// difference is one of the TRM rules in the module texts.
#[test]
fn register_image_against_the_replaced_driver() {
    let (m, _e, r) = attached(Am62pFaults::default());
    r.unwrap();
    let wz = 0xf00;
    // (register, ours, the C driver's)
    let same: [(&'static str, u32, u32); 13] = [
        (DSI, r::VID_HSIZE1, 479 << 16),
        (DSI, r::VID_HSIZE2, (474 << 16) | 3072),
        (DSI, r::VID_VSIZE2, 600),
        (DSI, r::VID_BLKSIZE1, 4037),
        (DSI, r::VID_BLKSIZE2, 4027),
        (DSI, r::VID_VCA_SETTING2, 4031 << 16),
        // pixel mode 24 bit, data type 3Eh, blanking packets, ignore missing vsync
        (DSI, r::VID_MAIN_CTL, (3 << 14) | (0x3e << 8) | (1 << 21) | (1 << 23) | (1 << 31)),
        (DSI, r::MCTL_DPHY_CFG0, 0x1003f1 | (0xf << 16)),
        (WRAP, w::DPI_CONTROL, 1),
        (DPHY, wz + wiz::PLL_CTRL, 2 | (8 << 8) | (394 << 16)),
        (DPHY, wz + wiz::PSM_FREQ, 25),
        (DPHY, pr::CMN_DIG_TBIT10, 6 | (0x101 << 10) | (8 << 20)),
        (DPHY, pr::PCS_TX_DIG_TBIT0, 5 | (5 << 5)),
    ];
    for (b, off, want) in same {
        assert_eq!(m.peek(b, off), want, "{b} {off:#x}");
    }
    assert_eq!(m.peek(DSI, r::MCTL_MAIN_DATA_CTL), r::LINK_EN | r::SDI_IF_VID_MODE | (1 << 2) | r::VID_EN);
    assert_eq!(m.peek(DSI, r::MCTL_MAIN_EN), r::PLL_START | r::CKLANE_EN | (0xf << 4) | r::IF2_EN);
    assert_eq!(m.peek(DPHY, pr::CMN_DIG_TBIT2), 0x28 | 1 | (1 << 9));
    assert_eq!(m.peek(DPHY, wz + wiz::RST_CTRL), 1 << 31);
    // The differences, each from a TRM rule:
    // VID_VSIZE1: C (22 << 6) | (12 << 12) | 11; TRM 12.9.2.7.9.1.
    assert_eq!(m.peek(DSI, r::VID_VSIZE1), (23 << 6) | (11 << 12) | 10);
    // VID_DPHY_TIME: C (101 << 17) | 1012; TRM 12.9.2.7.9.3 takes the HSS off.
    assert_eq!(m.peek(DSI, r::VID_DPHY_TIME), (101 << 17) | 1011);
    // MCTL_MAIN_PHY_CTL: C 0x17 with WAIT_BURST_TIME 0, which 14.8.9.2.2.24
    // forbids; 4 byte clocks = 104 ns at 38.5 MHz.
    assert_eq!(m.peek(DSI, r::MCTL_MAIN_PHY_CTL), 0x17 | (4 << 10));
    // Time-outs and ULP exit: units not in the TRM / ULP never used: maxima.
    assert_eq!(m.peek(DSI, r::MCTL_DPHY_TIMEOUT1), 0x3f_ffff);
    assert_eq!(m.peek(DSI, r::MCTL_DPHY_TIMEOUT2), 0x3_ffff);
    assert_eq!(m.peek(DSI, r::MCTL_ULPOUT_TIME), 0x3_ffff);
}

#[test]
fn failures_on_the_way_up() {
    let (_m, e, r) = attached(Am62pFaults { dphy_no_lock: true, ..Default::default() });
    assert_eq!(r, Err(-ETIMEDOUT));
    assert!(e.lines.borrow().iter().any(|l| l.1.contains("D-PHY PLL did not lock")));
    let (_m, e, r) = attached(Am62pFaults { dphy_cmn_not_ready: true, ..Default::default() });
    assert_eq!(r, Err(-ETIMEDOUT));
    assert!(e.lines.borrow().iter().any(|l| l.1.contains("O_CMN_READY")));
    let (m, e, r) = attached(Am62pFaults { lanes_stuck: true, ..Default::default() });
    assert_eq!(r, Err(-ETIMEDOUT));
    assert!(e.lines.borrow().iter().any(|l| l.1.contains("lanes not ready")));
    assert!(!m.writes().iter().any(|w| w.0 == DSI && w.1 == r::VID_MAIN_CTL), "no video on a dead link");
    // The waits are bounded.
    assert!(*e.waited_us.borrow() <= u64::from(LANES_WAIT_US) + 2 * u64::from(PLL_WAIT_US) + 100);
    // A mode the PLL cannot make: refused before any write.
    let m = Am62pModel::new();
    let e = TestEnv::default();
    let b = bases(&m);
    let wz = init(&m, &e, &b).unwrap();
    m.clear_log();
    let mut req = rvt70();
    req.mode.pixel_clock_hz = 52_500_000; // 315 Mbit/s: between OPDIV ranges
    assert_eq!(attach(&m, &e, &b, wz, &req), Err(-EINVAL));
    assert!(m.writes().is_empty());
    req.lanes = 4;
    req.mode.pixel_clock_hz = 51_200_000;
    req.pll_ref_hz = 200_000_000;
    assert_eq!(attach(&m, &e, &b, wz, &req), Err(-EINVAL));
}

#[test]
fn the_wiz_at_the_trms_offset_works_the_same() {
    let (m, _e, r) = attached(Am62pFaults { wiz_at_zero: true, ..Default::default() });
    r.unwrap();
    assert_eq!(m.peek(DPHY, wiz::PLL_CTRL), 2 | (8 << 8) | (394 << 16));
    assert!(!m.writes().iter().any(|w| w.0 == DPHY && w.1 >= 0xf00));
}

#[test]
fn detach_stops_video_and_link() {
    let (m, _e, r) = attached(Am62pFaults::default());
    r.unwrap();
    detach(&m, &bases(&m));
    assert_eq!(m.peek(DSI, r::MCTL_MAIN_DATA_CTL), 0);
    assert_eq!(m.peek(DSI, r::MCTL_MAIN_EN), 0);
    assert_eq!(m.peek(WRAP, w::DPI_CONTROL), 0);
}

#[test]
fn direct_commands() {
    let m = Am62pModel::new();
    let e = TestEnv::default();
    let b = bases(&m);
    // DCS short write with one parameter, LP, channel 0.
    assert_eq!(transfer(&m, &e, &b, 0, 0x15, true, &[0x36, 0x08]), Ok(2));
    let s = m.peek(DSI, r::DIRECT_CMD_MAIN_SETTINGS);
    assert_eq!(s, (0x15 << 8) | (2 << 16) | r::CMD_LP_EN);
    assert!(m.writes().contains(&(DSI, r::DIRECT_CMD_WRDAT, 0x0836)));
    let log = m.accesses();
    let clr = pos(&log, wr_to(DSI, r::DIRECT_CMD_STS_CLR));
    let send = pos(&log, wr_to(DSI, r::DIRECT_CMD_SEND));
    assert!(
        clr < pos(&log, wr_to(DSI, r::DIRECT_CMD_MAIN_SETTINGS)) && pos(&log, wr_to(DSI, r::DIRECT_CMD_WRDAT)) < send
    );
    // Generic long write, 6 bytes, channel 2, HS.
    m.clear_log();
    assert_eq!(transfer(&m, &e, &b, 2, 0x29, false, &[1, 2, 3, 4, 5, 6]), Ok(6));
    assert_eq!(m.peek(DSI, r::DIRECT_CMD_MAIN_SETTINGS), (0x29 << 8) | (2 << 14) | (6 << 16) | r::CMD_LONGNOTSHORT);
    let words: Vec<u32> = m.writes().iter().filter(|w| w.1 == r::DIRECT_CMD_WRDAT).map(|w| w.2).collect();
    assert_eq!(words, [0x0403_0201, 0x0605]);
    // Refusals: a read type, a short write with 3 bytes, past the FIFO (64
    // bytes from IP_CONF), a bad channel.
    assert_eq!(transfer(&m, &e, &b, 0, 0x06, true, &[0x0a]), Err(-ENOTSUP));
    assert_eq!(transfer(&m, &e, &b, 0, 0x15, true, &[1, 2, 3]), Err(-EMSGSIZE));
    assert_eq!(transfer(&m, &e, &b, 0, 0x39, true, &[0; 65]), Err(-EMSGSIZE));
    assert_eq!(transfer(&m, &e, &b, 4, 0x05, true, &[0x29]), Err(-EMSGSIZE));
    let m = Am62pModel::with_faults(Am62pFaults { cmd_stuck: true, ..Default::default() });
    assert_eq!(transfer(&m, &e, &bases(&m), 0, 0x05, true, &[0x29]), Err(-ETIMEDOUT));
}

#[test]
fn ffi_attach_fills_data_and_refuses_a_second() {
    use crate::ffi::{dsi_flags, YdCdnsDsiData, YdCdnsDsiRequest};
    let m = Am62pModel::new();
    let e = TestEnv::default();
    let mut d = YdCdnsDsiData { dsi: m.base(DSI), wrap: m.base(WRAP), dphy: m.base(DPHY), ..Default::default() };
    d.wiz = init(&m, &e, &bases(&m)).unwrap();
    let req = YdCdnsDsiRequest {
        pixel_clock_hz: 51_200_000,
        hactive: 1024,
        hfp: 160,
        hsync: 5,
        hbp: 160,
        vactive: 600,
        vfp: 12,
        vsync: 10,
        vbp: 23,
        bpp: 24,
        lanes: 4,
        flags: 0,
        datatype: PACKED_24,
        pll_ref_hz: 25_000_000,
        psm_hz: 25_000_000,
    };
    assert_eq!(crate::ffi::request(&req), rvt70());
    assert_eq!(crate::ffi::attach(&m, &e, &mut d, &req), 0);
    assert_eq!((d.link_up, d.rate, d.fbdiv, d.band, d.line_bytes), (1, 307_812_500, 394, 5, 4047));
    assert_eq!(crate::ffi::attach(&m, &e, &mut d, &req), -crate::errno::EBUSY);
    let pulses = YdCdnsDsiRequest { flags: dsi_flags::SYNC_PULSE | dsi_flags::EOT, ..req };
    let r = crate::ffi::request(&pulses);
    assert!(r.sync_pulse && r.eot);
}
