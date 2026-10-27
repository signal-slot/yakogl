// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the display power domains against the register model.

use super::*;
use crate::model::{Access, Faults, Model, TestEnv, ANATOP, BLK, CCM, GPC};
use std::collections::BTreeMap;

pub(crate) fn bases(m: &Model) -> Bases {
    Bases { blk: m.base(BLK), gpc: m.base(GPC), ccm: m.base(CCM), anatop: m.base(ANATOP) }
}

/// The board from power-on reset as far as these blocks go (RM reset values,
/// system PLLs as the boot firmware leaves them: SYS_PLL1 as the golden
/// dump, SYS_PLL2 at its RM 5.1.8.33 reset dividers, 1000 MHz).
pub(crate) fn cold(m: &Model) {
    m.poke(ANATOP, 0x94, 0x8aaa_aa00);
    m.poke(ANATOP, 0x98, 0x0019_0032);
    m.poke(ANATOP, 0x104, 0x8aaa_aa00);
    m.poke(ANATOP, 0x108, 0x000f_a031);
    m.poke(ANATOP, 0x28, 0x0000_2010); // RM 5.1.8.11 reset
    m.poke(ANATOP, 0x2c, 0x0014_5032); // RM 5.1.8.12 reset
    m.poke(GPC, 0x190, 0x0000_ffff); // RM 5.2.10.57 reset
    for pgc in [0u32, 10] {
        m.poke(GPC, 0xb00 + pgc * 0x40, 0x0604_0202); // GPC_PGC_PUn_CTRL reset
        m.poke(GPC, 0xb0c + pgc * 0x40, 0x0000_1000);
    }
    for g in [14u32, 24, 42, 93] {
        m.poke(CCM, 0x4000 + g * 0x10, 0x2); // CCGR reset (RM 5.1.7 memory map)
    }
}

/// The cold path of the i.MX8MP display facade, as imx8mp_display.c runs it,
/// at the board's Linux clock plan (69.3 MHz: the golden dump's).
pub(crate) fn bring_up(m: &Model, b: &Bases, e: &TestEnv, t: &mut Track, video_pll1_hz: u32) -> Result<u32, i32> {
    bring_up_at(m, b, e, t, 69_300_000, video_pll1_hz)
}

/// [`bring_up`] for a pixel clock of `want`.
pub(crate) fn bring_up_at(
    m: &Model,
    b: &Bases,
    e: &TestEnv,
    t: &mut Track,
    want: u32,
    video_pll1_hz: u32,
) -> Result<u32, i32> {
    early_clocks(m, b.ccm, &[root::I2C2, root::PWM3], &[gate::I2C2, gate::PWM3, gate::GPIO4])?;
    pd_on(m, b, e, t, Pd::Lcdif1)?;
    pd_on(m, b, e, t, Pd::MipiDsi1)?;
    let hz = set_pixel_clock(m, b, e, t, want, video_pll1_hz)?;
    assert_eq!(set_phy_ref_clock(m, b, t, 12_000_000)?, 12_000_000);
    pd_reset(m, b, e, t, Pd::Lcdif1)?;
    pd_reset(m, b, e, t, Pd::MipiDsi1)?;
    Ok(hz)
}

pub(crate) fn golden() -> BTreeMap<u32, u32> {
    const DUMP: &str = include_str!("../../../../../src/display/golden-linux-2026-09-21.txt");
    DUMP.lines()
        .filter_map(|l| {
            let (a, v) = l.split_once(": ")?;
            let a = u32::from_str_radix(a.strip_prefix("0x")?, 16).ok()?;
            let v = u32::from_str_radix(v.trim().strip_prefix("0x")?, 16).ok()?;
            Some((a, v))
        })
        .collect()
}

/// Registers this driver owns, with the bits it owns, compared with the
/// golden dump of the board's Linux.
const OWNED: &[(u32, &str, u32, u32)] = &[
    // GPC: triggers, their status, the MEDIAMIX handshake, the A53 mapping
    (0x303a_00d8, GPC, 0xd8, 0x401),
    (0x303a_0124, GPC, 0x124, 0x401),
    (0x303a_0128, GPC, 0x128, 0x401),
    (0x303a_012c, GPC, 0x12c, 0x401),
    (0x303a_0190, GPC, 0x190, 0x4000_4000),
    (0x303a_01cc, GPC, 0x1cc, 0x1004),
    // CCM roots: ENABLE, MUX, PRE_PODF, POST_PODF
    (0x3038_8a00, CCM, 0x8a00, 0x1707_003f),
    (0x3038_8a80, CCM, 0x8a80, 0x1707_003f),
    (0x3038_bd80, CCM, 0xbd80, 0x1707_003f),
    (0x3038_be00, CCM, 0xbe00, 0x1707_003f),
    // early clocks: ENABLE only
    (0x3038_ad80, CCM, 0xad80, 0x1000_0000),
    (0x3038_b480, CCM, 0xb480, 0x1000_0000),
    // gates, domain 0
    (0x3038_45d0, CCM, 0x45d0, 0x3),
    (0x3038_4180, CCM, 0x4180, 0x3),
    (0x3038_42a0, CCM, 0x42a0, 0x3),
    (0x3038_40e0, CCM, 0x40e0, 0x3),
    // VIDEO_PLL1
    (0x3036_0028, ANATOP, 0x28, 0x8001_2213),
    (0x3036_002c, ANATOP, 0x2c, 0x003f_f3f7),
    (0x3036_0030, ANATOP, 0x30, 0xffff),
    // MEDIA_BLK_CTRL
    (0x32ec_0000, BLK, 0x0, 0x0080_0133),
    (0x32ec_0004, BLK, 0x4, 0x0080_0133),
    (0x32ec_0008, BLK, 0x8, 0x0002_0000),
    (0x32ec_004c, BLK, 0x4c, 0x0000_1c00),
];

/// Every expected difference, and why.
const ALLOWED: &[(u32, &str)] = &[
    (
        0x3038_ad80,
        "I2C2_CLK_ROOT: Linux's i2c-imx gates its clock while idle (runtime PM, clk_summary enable=0); \
         the early clocks open it so Zephyr's I2C driver can touch the block (NOTES.md, measured 2026-09-20)",
    ),
    (0x3038_4180, "CCGR24 (I2C2): the same"),
];

/// The R4 clock plan: without VIDEO_PLL1 the pixel clock comes from
/// SYSTEM_PLL2_CLK / 14 = 71.43 MHz; both are valid by the RM's formula
/// (docs/source-form.md risk R4).
const ALLOWED_R4: &[(u32, &str)] = &[
    (0x3038_be00, "MEDIA_DISP1_PIX: SYSTEM_PLL2_CLK (mux 5) / 14 instead of VIDEO_PLL1 / 15"),
    (0x3036_0028, "VIDEO_PLL1 not programmed (left at reset)"),
    (0x3036_002c, "VIDEO_PLL1 not programmed"),
    (0x3036_0030, "VIDEO_PLL1 not programmed"),
];

fn compare(m: &Model, allowed: &[(u32, &str)]) {
    let g = golden();
    let mut diffs = Vec::new();
    for &(abs, blk, off, mask) in OWNED {
        let gv = *g.get(&abs).unwrap_or_else(|| panic!("{abs:#x} not in the golden dump"));
        let mv = m.peek(blk, off);
        if gv & mask != mv & mask {
            diffs.push((abs, gv & mask, mv & mask));
        }
    }
    let unexplained: Vec<_> = diffs.iter().filter(|d| !allowed.iter().any(|a| a.0 == d.0)).collect();
    assert!(unexplained.is_empty(), "differences not on the allow-list: {unexplained:#x?}");
    // and every allow-list entry is a real difference (no stale entries)
    for a in allowed {
        assert!(diffs.iter().any(|d| d.0 == a.0), "allow-list entry {:#x} does not differ: {}", a.0, a.1);
    }
}

#[test]
fn golden_cold_bring_up_matches_linux_except_the_allow_list() {
    let m = Model::new();
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let hz = bring_up(&m, &bases(&m), &e, &mut t, 1_039_500_000).unwrap();
    assert_eq!(hz, 69_300_000);
    compare(&m, ALLOWED);
    assert_eq!(pixel_clock(&m, &bases(&m)), 69_300_000);
}

#[test]
fn golden_r4_plan_without_video_pll1() {
    let m = Model::new();
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let hz = bring_up(&m, &bases(&m), &e, &mut t, 0).unwrap();
    assert_eq!(hz, 71_428_571);
    assert_eq!(m.peek(CCM, 0xbe00), 0x1500_000d);
    let mut all = ALLOWED.to_vec();
    all.extend_from_slice(ALLOWED_R4);
    compare(&m, &all);
}

fn pos(acc: &[Access], f: impl Fn(&Access) -> bool) -> usize {
    acc.iter().position(f).expect("access")
}

#[test]
fn ordering_roots_and_gate_before_blk_power_before_release_gates_before_consumers() {
    let m = Model::new();
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    bring_up(&m, &bases(&m), &e, &mut t, 1_039_500_000).unwrap();
    let a = m.accesses();
    let first_blk = pos(&a, |x| matches!(x, Access::R(BLK, ..) | Access::W(BLK, ..)));
    let apb = pos(&a, |x| matches!(x, Access::W(CCM, 0x8a80, _)));
    let axi = pos(&a, |x| matches!(x, Access::W(CCM, 0x8a00, _)));
    let gate = pos(&a, |x| matches!(x, Access::W(CCM, 0x45d4, _)));
    let pup = pos(&a, |x| matches!(x, Access::W(GPC, 0xd8, v) if v & 0x400 != 0));
    assert!(axi < gate && apb < gate, "roots before their gate");
    assert!(gate < first_blk && pup < first_blk, "CCM and power before any BLK_CTRL access");
    // a block's clock enable before its reset release, for each block
    for bits in [0x100u32, 0x0080_0030, 0x3] {
        let clk = pos(&a, |x| matches!(x, Access::W(BLK, 0x4, v) if v & bits == bits));
        let rst = pos(&a, |x| matches!(x, Access::W(BLK, 0x0, v) if v & bits == bits));
        assert!(clk < rst, "CLK_EN before SFT_RSTN for {bits:#x}");
    }
    // MIPI_PHY1 powered before the DSI or its D-PHY leave reset
    let phy = pos(&a, |x| matches!(x, Access::W(GPC, 0xd8, v) if v & 1 != 0));
    let dsi = pos(&a, |x| matches!(x, Access::W(BLK, 0x0, v) if v & 3 == 3));
    let dphy = pos(&a, |x| matches!(x, Access::W(BLK, 0x8, v) if v & 0x20000 != 0));
    assert!(phy < dsi && phy < dphy);
    // the ADB acknowledge was waited for after the bus clock came on
    let bus = pos(&a, |x| matches!(x, Access::W(BLK, 0x4, v) if v & 0x100 != 0));
    let ack = a.iter().rposition(|x| matches!(x, Access::R(GPC, 0x190, v) if v & (1 << 30) != 0)).unwrap();
    assert!(bus < ack);
    // the status word says it all
    let s = status(&m, &bases(&m), &t);
    let want = st::MEDIAMIX_MAPPED
        | st::MIPI_PHY1_MAPPED
        | st::MEDIAMIX_PUP_CLEAN
        | st::MIPI_PHY1_PUP_CLEAN
        | st::ADB_REQ
        | st::ADB_ACK
        | st::PCR_CLEAR
        | st::MEDIA_AXI_ROOT
        | st::MEDIA_APB_ROOT
        | st::MEDIA_GATE
        | st::DISP1_PIX_ROOT
        | st::PHY1_REF_ROOT
        | st::VIDEO_PLL1_RUNNING
        | st::SYS_PLL1_RUNNING
        | st::SYS_PLL2_RUNNING
        | st::BLK_BUS
        | st::BLK_LCDIF1
        | st::BLK_MIPI_DSI1
        | st::BLK_PHY
        | st::BLK_HURRY
        | st::BLK_SAMPLED
        | step::RESET << st::LAST_SHIFT;
    assert_eq!(s, want, "{s:#010x} vs {want:#010x}");
}

/// The state Linux leaves (golden): everything on.
fn linux_left_it_running(m: &Model) {
    cold(m);
    let g = golden();
    for &(abs, blk, off, _) in OWNED {
        if blk == GPC && (0x124..=0x12c).contains(&off) {
            continue;
        }
        m.poke(blk, off, g[&abs]);
    }
    m.set_powered(0x401);
    m.poke(GPC, 0xb00, 0x0604_0202);
    m.poke(GPC, 0xd80, 0x0604_0202);
}

#[test]
fn adopting_a_running_chain_writes_nothing() {
    let m = Model::new();
    linux_left_it_running(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let b = bases(&m);
    pd_on(&m, &b, &e, &mut t, Pd::Lcdif1).unwrap();
    pd_on(&m, &b, &e, &mut t, Pd::MipiDsi1).unwrap();
    assert!(m.writes().is_empty(), "{:#x?}", m.writes());
    assert_eq!(pixel_clock(&m, &b), 69_300_000, "read back through VIDEO_PLL1's registers");
    // the R4 plan on a running VIDEO_PLL1 still finds it
    assert_eq!(set_pixel_clock(&m, &b, &e, &mut t, 69_300_000, 0), Ok(69_300_000));
    assert!(m.writes().is_empty());
}

#[test]
fn power_cycle_reports_powered_from_off() {
    let m = Model::new();
    linux_left_it_running(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let b = bases(&m);
    pd_on(&m, &b, &e, &mut t, Pd::Lcdif1).unwrap();
    pd_on(&m, &b, &e, &mut t, Pd::MipiDsi1).unwrap();
    pd_off(&m, &b, &e, &mut t, Pd::MipiDsi1).unwrap();
    assert_eq!(m.powered(), 0x400, "MIPI_PHY1 down, MEDIAMIX still used by LCDIF1");
    pd_off(&m, &b, &e, &mut t, Pd::Lcdif1).unwrap();
    assert_eq!(m.powered(), 0);
    assert_eq!(status(&m, &b, &t) & (st::ADB_ACK | st::BLK_SAMPLED), 0);
    pd_on(&m, &b, &e, &mut t, Pd::Lcdif1).unwrap();
    pd_on(&m, &b, &e, &mut t, Pd::MipiDsi1).unwrap();
    assert_eq!(m.powered(), 0x401);
    let s = status(&m, &b, &t);
    assert_ne!(s & st::POWERED_FROM_OFF, 0);
    assert_eq!(
        s & (st::BLK_LCDIF1 | st::BLK_MIPI_DSI1 | st::BLK_HURRY),
        st::BLK_LCDIF1 | st::BLK_MIPI_DSI1 | st::BLK_HURRY
    );
}

#[test]
fn a_stuck_power_up_fails_without_touching_blk_ctrl() {
    let m = Model::with_faults(Faults { gpc_pup_stuck: true, ..Default::default() });
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let b = bases(&m);
    assert_eq!(pd_on(&m, &b, &e, &mut t, Pd::Lcdif1), Err(-ETIMEDOUT));
    assert!(!m.accesses().iter().any(|a| matches!(a, Access::R(BLK, ..) | Access::W(BLK, ..))));
    let s = status(&m, &b, &t);
    assert_eq!(s >> st::LAST_SHIFT, step::MEDIAMIX | 1 << 4 | 1 << 5, "{s:#x}");
    assert_eq!(s & st::MEDIAMIX_PUP_CLEAN, 0);
}

#[test]
fn a_refused_power_up_is_an_io_error() {
    let m = Model::with_faults(Faults { gpc_pup_refused: true, ..Default::default() });
    cold(&m);
    let mut t = Track::default();
    assert_eq!(pd_on(&m, &bases(&m), &TestEnv::default(), &mut t, Pd::Lcdif1), Err(-EIO));
    assert_eq!(t.last, step::MEDIAMIX | 1 << 4 | 2 << 5);
}

#[test]
fn no_adb_ack_is_an_error_not_a_hang() {
    let m = Model::with_faults(Faults { adb_no_ack: true, ..Default::default() });
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    assert_eq!(pd_on(&m, &bases(&m), &e, &mut t, Pd::Lcdif1), Err(-ETIMEDOUT));
    assert_eq!(t.last & 0xf, step::ADB_ACK);
    assert!(*e.waited_us.borrow() >= u64::from(gpc::POLL_US));
}

#[test]
fn no_pll_lock_is_an_error_not_a_hang() {
    let m = Model::with_faults(Faults { pll_no_lock: true, ..Default::default() });
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let b = bases(&m);
    pd_on(&m, &b, &e, &mut t, Pd::Lcdif1).unwrap();
    assert_eq!(set_pixel_clock(&m, &b, &e, &mut t, 69_300_000, 1_039_500_000), Err(-ETIMEDOUT));
    assert_eq!(t.last & 0xf, step::VIDEO_PLL1);
    assert_eq!(m.peek(CCM, 0xbe00), 0, "the pixel root not touched");
}

#[test]
fn bus_roots_need_running_system_plls() {
    let m = Model::new();
    cold(&m);
    m.poke(ANATOP, 0x104, 0x0aaa_aa00); // SYS_PLL2 not locked
    let mut t = Track::default();
    assert_eq!(pd_on(&m, &bases(&m), &TestEnv::default(), &mut t, Pd::Lcdif1), Err(-EIO));
    assert!(m.writes().is_empty());
}

#[test]
fn reset_needs_mediamix() {
    let m = Model::new();
    cold(&m);
    let mut t = Track::default();
    assert_eq!(pd_reset(&m, &bases(&m), &TestEnv::default(), &mut t, Pd::Lcdif1), Err(-EIO));
}

#[test]
fn pixel_plan_is_nearest_and_bounded() {
    let parents = [(1, 1_039_500_000), (5, 1_000_000_000), (4, 800_000_000), (0, 24_000_000)];
    assert_eq!(plan_root(69_300_000, &parents, 250_000_000), Some((RootSetting::on(1, 1, 15), 69_300_000)));
    assert_eq!(plan_root(69_300_000, &parents[1..], 250_000_000), Some((RootSetting::on(5, 1, 14), 71_428_571)));
    // never above the root's maximum
    assert_eq!(plan_root(1_000_000_000, &parents[1..2], 250_000_000).unwrap().1, 250_000_000);
    assert_eq!(plan_root(1, &[], 250_000_000), None);
    // the PHY reference: 24 MHz / 2
    let m = Model::new();
    let mut t = Track::default();
    assert_eq!(set_phy_ref_clock(&m, &bases(&m), &mut t, 12_000_000), Ok(12_000_000));
    assert_eq!(m.peek(CCM, 0xbd80), 0x1000_0001);
    assert_eq!(set_phy_ref_clock(&m, &bases(&m), &mut t, 0), Err(-EINVAL));
}

#[test]
fn the_c_header_names_the_same_status_bits() {
    const H: &str = include_str!("../../../../include/yakogl_drivers.h");
    let bit = |name: &str| -> u32 {
        let l = H.lines().find(|l| l.starts_with(&format!("#define YD_MEDIA_ST_{name} "))).expect(name);
        let n: u32 = l
            .split("<<")
            .nth(1)
            .unwrap()
            .trim()
            .chars()
            .take_while(char::is_ascii_digit)
            .collect::<String>()
            .parse()
            .unwrap();
        1 << n
    };
    for (name, v) in [
        ("MEDIAMIX_MAPPED", st::MEDIAMIX_MAPPED),
        ("MIPI_PHY1_MAPPED", st::MIPI_PHY1_MAPPED),
        ("MEDIAMIX_PUP_CLEAN", st::MEDIAMIX_PUP_CLEAN),
        ("MIPI_PHY1_PUP_CLEAN", st::MIPI_PHY1_PUP_CLEAN),
        ("ADB_REQ", st::ADB_REQ),
        ("ADB_ACK", st::ADB_ACK),
        ("PCR_CLEAR", st::PCR_CLEAR),
        ("POWERED_FROM_OFF", st::POWERED_FROM_OFF),
        ("MEDIA_AXI_ROOT", st::MEDIA_AXI_ROOT),
        ("MEDIA_APB_ROOT", st::MEDIA_APB_ROOT),
        ("MEDIA_GATE", st::MEDIA_GATE),
        ("DISP1_PIX_ROOT", st::DISP1_PIX_ROOT),
        ("PHY1_REF_ROOT", st::PHY1_REF_ROOT),
        ("VIDEO_PLL1_RUNNING", st::VIDEO_PLL1_RUNNING),
        ("SYS_PLL1_RUNNING", st::SYS_PLL1_RUNNING),
        ("SYS_PLL2_RUNNING", st::SYS_PLL2_RUNNING),
        ("BLK_BUS", st::BLK_BUS),
        ("BLK_LCDIF1", st::BLK_LCDIF1),
        ("BLK_MIPI_DSI1", st::BLK_MIPI_DSI1),
        ("BLK_PHY", st::BLK_PHY),
        ("BLK_HURRY", st::BLK_HURRY),
        ("BLK_SAMPLED", st::BLK_SAMPLED),
    ] {
        assert_eq!(bit(name), v, "{name}");
    }
}

/// The panel console's guard (gc_panel_console.c, stage 1): on a cold
/// MEDIAMIX it says "unreachable" without touching MEDIA_BLK_CTRL or the
/// LCDIF (the model panics on any such access), and after the bring-up the
/// LCDIF answers.
#[test]
fn lcdif_reachability_reads_only_what_answers() {
    let m = Model::new();
    cold(&m);
    let b = bases(&m);
    let why = lcdif_unreachable(&m, b.gpc, b.ccm, b.blk);
    assert_ne!(why & unreach::MEDIAMIX, 0, "{why:#x}");
    assert!(
        !m.accesses().iter().any(|x| matches!(x, Access::R(BLK, ..) | Access::W(BLK, ..))),
        "the guard read MEDIA_BLK_CTRL with MEDIAMIX off"
    );
    assert!(!m.accesses().iter().any(|x| matches!(x, Access::W(..))), "the guard wrote");

    // MIPI_DSI1 alone: MEDIAMIX and the bus are up, the LCDIF clock is not
    let e = TestEnv::default();
    let mut t = Track::default();
    early_clocks(&m, b.ccm, &[root::I2C2, root::PWM3], &[gate::I2C2, gate::PWM3, gate::GPIO4]).unwrap();
    pd_on(&m, &b, &e, &mut t, Pd::MipiDsi1).unwrap();
    assert_eq!(lcdif_unreachable(&m, b.gpc, b.ccm, b.blk), unreach::LCDIF_APB);

    pd_on(&m, &b, &e, &mut t, Pd::Lcdif1).unwrap();
    assert_eq!(lcdif_unreachable(&m, b.gpc, b.ccm, b.blk), 0);
    // and the model agrees: an LCDIF read is now allowed
    let _ = m.read32(m.base(crate::model::LCDIF1));

    // the MEDIA gate closed again: unreachable, and MEDIA_BLK_CTRL untouched
    m.poke(CCM, 0x4000 + gate::MEDIA * 0x10, 0);
    m.clear_log();
    assert_eq!(lcdif_unreachable(&m, b.gpc, b.ccm, b.blk), unreach::GATE);
    assert!(!m.accesses().iter().any(|x| matches!(x, Access::R(BLK, ..))));
}
