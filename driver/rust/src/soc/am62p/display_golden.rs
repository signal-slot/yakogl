// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The golden test of the AM62P display path: the register image the model
//! holds after [`super::dss::dss_init`] + [`super::dss::output_enable`] and
//! [`crate::ip::cdns_dsi::init`] + [`crate::ip::cdns_dsi::attach`] with the
//! Verdin AM62P's configuration (Riverdi RVT70, 1024x600, VP2 -> DSI0 ->
//! SN65DSI83), against the registers of the same blocks read from the board
//! while its Linux drove the panel (driver/docs/am62p-display-dump-linux.txt,
//! taken through /dev/mem; the read list is in driver/docs/am62p-display-cold.md).
//! Bits that may differ, and why, are listed in
//! driver/docs/am62p-display-allow.txt; every other bit of every dumped
//! register of a modelled block must agree.
//!
//! The dump is test data, not a source: a difference outside the allow-list
//! is a question for the TRM and the board, answered either by a change in
//! the driver (with its citation) or by an allow-list line with its reason.
//!
//! VP1 and OVR1 are in the dump (Linux leaves them at reset) but neither
//! driver touches them and the model has one port: they are skipped.
//!
//! File formats (both: `#` starts a comment, blank lines ignored):
//! - dump: `ADDRESS WIDTH VALUE`, hex address, 32, hex value;
//! - allow-list: `ADDRESS MASK REASON...`, the bits of MASK may differ.

use super::dss::{dss_init, flags, output_enable, DssConfig, DssData};
use crate::ip::cdns_dsi::{attach, init, Bases, Request};
use crate::mmio::Mmio;
use crate::model::TestEnv;
use crate::model_am62p::{Am62pModel, COMMON, DPHY, DSI, OVR, VID, VIDL1, VP, WRAP};
use crate::video::VideoMode;

const DUMP: &str = include_str!("../../../../docs/am62p-display-dump-linux.txt");
const ALLOW: &str = include_str!("../../../../docs/am62p-display-allow.txt");

/// The physical bases of the modelled blocks (SPRUJ83D 14.8.9.1, 14.8.9.2,
/// 14.8.6.3; the board uses DSS1's VP2 and OVR2).
const REAL: [(&str, u64, u64); 8] = [
    (COMMON, 0x3022_0000, 0x1000),
    (VIDL1, 0x3022_2000, 0x1000),
    (VID, 0x3022_6000, 0x1000),
    (OVR, 0x3022_8000, 0x1000),
    (VP, 0x3022_b000, 0x1000),
    (WRAP, 0x3027_0000, 0x100),
    (DSI, 0x3050_0000, 0x1000),
    (DPHY, 0x301c_0000, 0x1000),
];

/// Dumped but not modelled: OVR1 and VP1.
const SKIPPED: [(u64, u64); 2] = [(0x3022_7000, 0x1000), (0x3022_a000, 0x1000)];

fn block_of(addr: u64) -> Option<(&'static str, u32)> {
    REAL.iter().find(|(_, b, s)| addr >= *b && addr < b + s).map(|(n, b, _)| (*n, (addr - b) as u32))
}

fn hex(s: &str) -> u64 {
    u64::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or_else(|_| panic!("not hex: {s}"))
}

fn lines(text: &str) -> impl Iterator<Item = Vec<&str>> {
    text.lines()
        .map(|l| l.split('#').next().unwrap_or("").trim())
        .filter(|l| !l.is_empty())
        .map(|l| l.split_whitespace().collect())
}

/// (address, width, value)
fn dump() -> Vec<(u64, u32, u32)> {
    lines(DUMP)
        .map(|f| {
            assert!(f.len() == 3, "dump line {f:?}");
            (hex(f[0]), f[1].parse().expect("width"), hex(f[2]) as u32)
        })
        .collect()
}

/// (address, mask, reason)
fn allow() -> Vec<(u64, u32, String)> {
    lines(ALLOW)
        .map(|f| {
            assert!(f.len() >= 3, "allow-list line {f:?} needs ADDRESS MASK REASON");
            (hex(f[0]), hex(f[1]) as u32, f[2..].join(" "))
        })
        .collect()
}

/// The board devicetree's &dss1 (as soc::am62p::dss's tests).
const DSS: DssConfig = DssConfig {
    pixel_clock_hz: 51_200_000,
    hactive: 1024,
    hfp: 160,
    hsync: 5,
    hbp: 160,
    vactive: 600,
    vfp: 12,
    vsync: 10,
    vbp: 23,
    flags: flags::HSYNC_LOW | flags::VSYNC_LOW | flags::HAS_IRQ,
    vp_index: 1,
    safety_width: 1024,
    safety_height: 600,
};

/// The board's &dsi0 and its SN65DSI83 (as ip::cdns_dsi's tests but the
/// PSM clock).
const DSI_REQ: Request = Request {
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
    datatype: 0x3e,
    pll_ref_hz: 25_000_000,
    // The PSM clock (TISCI 238:16) runs at 16 MHz on the board (Linux's
    // clk_summary in the dump's header); the shim passes what TISCI reports.
    psm_hz: 16_000_000,
};

#[test]
fn the_allow_list_names_dumped_blocks_only() {
    for (a, mask, why) in allow() {
        assert!(block_of(a).is_some(), "allow-list {a:#x}: not in a modelled block");
        assert!(mask != 0 && !why.is_empty(), "allow-list {a:#x}: empty mask or no reason");
    }
}

#[test]
fn cold_image_matches_the_linux_dump_outside_the_allow_list() {
    let d = dump();
    assert!(!d.is_empty(), "the dump is the test's data");
    let m = Am62pModel::new();
    m.set_port(1);
    let e = TestEnv::default();
    let b = Bases { dsi: m.base(DSI), wrap: m.base(WRAP), dphy: m.base(DPHY) };
    let wiz = init(&m, &e, &b).expect("dsi init");
    attach(&m, &e, &b, wiz, &DSI_REQ).expect("attach");
    let dd = DssData {
        common: m.base(COMMON),
        vid: m.base(VID),
        vidl1: m.base(VIDL1),
        ovr: m.base(OVR),
        vp: m.base(VP),
        safety_buf: [0x9000_0000, 0x9030_0000],
        ..Default::default()
    };
    dss_init(&m, &e, &DSS, &dd).expect("dss init");
    output_enable(&m, &dd, true);
    let allow = allow();
    let mut bad = Vec::new();
    for (addr, width, want) in d {
        if SKIPPED.iter().any(|(b, s)| addr >= *b && addr < b + s) {
            continue;
        }
        let Some((blk, off)) = block_of(addr) else {
            panic!("dump {addr:#x}: outside the blocks this test models");
        };
        assert_eq!(width, 32, "dump {addr:#x}: read with the wrong width");
        // Read through the model, so status registers answer as the
        // hardware would after the bring-up (lock, lane ready, enables).
        let got = m.read32(m.base(blk) + off as usize);
        let may = allow.iter().filter(|x| x.0 == addr).fold(0, |a, x| a | x.1);
        if (got ^ want) & !may != 0 {
            bad.push(format!(
                "{addr:#010x} ({blk}+{off:#x}): ours {got:#x}, Linux {want:#x}, differing {:#x}",
                (got ^ want) & !may
            ));
        }
    }
    assert!(bad.is_empty(), "{} register(s) differ outside the allow-list:\n{}", bad.len(), bad.join("\n"));
}
