// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The golden test of the HDMI cold path: the register image the model holds
//! after [`super::hdmi_chain::bring_up`] with the Armadillo-X2's
//! configuration, against the registers of the same blocks read from the
//! board while its Linux showed a picture (driver/docs/x2-hdmi-dump-linux.txt,
//! taken through /dev/mem; the read list and the procedure are in
//! driver/docs/x2-hdmi-cold.md). Bits that may differ, and why, are listed in
//! driver/docs/x2-hdmi-allow.txt; every other bit of every dumped register
//! must agree.
//!
//! The dump is test data, not a source: a difference outside the allow-list
//! is a question for the RM and the board, answered either by a change in
//! the driver (with its citation) or by an allow-list line with its reason.
//!
//! File formats (both: `#` starts a comment, blank lines ignored):
//! - dump: `ADDRESS WIDTH VALUE`, hex address, 8 or 32, hex value;
//! - allow-list: `ADDRESS MASK REASON...`, the bits of MASK may differ.

use super::hdmi::ClockSel;
use super::hdmi_chain::{self, Bases, Config, TestPicture};
use crate::ip::dw_hdmi_tx::PhyIf;
use crate::ip::lcdifv3::{Layer, Panic};
use crate::model::TestEnv;
use crate::model_hdmi::{HdmiModel, BLK, CCM, GPC, HDMI, LCDIF, PHY, PVI};
use crate::video::WUXGA_60_RB;

const DUMP: &str = include_str!("../../../../docs/x2-hdmi-dump-linux.txt");
const ALLOW: &str = include_str!("../../../../docs/x2-hdmi-allow.txt");

/// The physical bases of the modelled blocks (IMX8MPRM Rev. 3 memory map;
/// 13.3.3.1.1, 13.9.3, 13.10.3.1.1, 13.11.3.1.1, 13.13.3.1.1, 5.1, 5.2).
const REAL: [(&str, u64, u64); 7] = [
    (LCDIF, 0x32fc_6000, 0x1000),
    (BLK, 0x32fc_0000, 0x1000),
    (PVI, 0x32fc_4000, 0x800),
    (HDMI, 0x32fd_8000, 0x7f00),
    (PHY, 0x32fd_ff00, 0x100),
    (CCM, 0x3038_0000, 0x1_0000),
    (GPC, 0x303a_0000, 0x1_0000),
];

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

/// address -> (mask, reason)
fn allow() -> Vec<(u64, u32, String)> {
    lines(ALLOW)
        .map(|f| {
            assert!(f.len() >= 3, "allow-list line {f:?} needs ADDRESS MASK REASON");
            (hex(f[0]), hex(f[1]) as u32, f[2..].join(" "))
        })
        .collect()
}

/// The configuration the X2's devicetree gives (boards/atmark/armadillo_x2).
const X2: Config = Config {
    mode: WUXGA_60_RB,
    max_ppm: 5000,
    clock_sel: ClockSel { htx_pipe: None, htxphy: None },
    phy_if: PhyIf { conf0: None, phyrstz: None },
    lcdif_pxck_rising: false,
    desc3_extra: 0,
    noc_hurry: 7,
    panic: Some(Panic { low: 170, high: 340 }),
    power_cycle: false,
    stop_after: 0,
    test: TestPicture::None,
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
    if d.is_empty() {
        // Not taken yet (driver/docs/x2-hdmi-cold.md, board plan (a)).
        return;
    }
    let m = HdmiModel::new();
    let e = TestEnv::default();
    let b = Bases {
        lcdif: m.base(LCDIF),
        hdmi: m.base(HDMI),
        phy: m.base(PHY),
        blk: m.base(BLK),
        pvi: m.base(PVI),
        gpc: m.base(GPC),
        ccm: m.base(CCM),
    };
    hdmi_chain::bring_up(&m, &b, &e, &X2, &Layer { phys: 0x9000_0000, pitch: 0, desc3_extra: 0 }).unwrap();
    let allow = allow();
    let mut bad = Vec::new();
    for (addr, width, want) in d {
        let Some((blk, off)) = block_of(addr) else {
            panic!("dump {addr:#x}: outside the blocks this test models");
        };
        assert_eq!(width, if blk == HDMI { 8 } else { 32 }, "dump {addr:#x}: read with the wrong width");
        let got = m.peek(blk, off);
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
