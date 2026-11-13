// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests: the DK's display chain agrees with itself end to end.
//! Plan #353, issues #7 and #1281.
//!
//! For each DK mode (1280x720@60 and 1920x1080@60, four lanes, RGB888) the
//! parts are configured as the DK's image will configure them, and what each
//! one holds is checked against the others:
//!
//! - `ip::stm32_ltdc` programs the timing registers (the MP25's a2 layout);
//! - `ip::dw_mipi_dsi::plan` gives the host's video timing in lane byte
//!   clocks for the rate `ip::stm32_dsi_phy` chooses (`StmDsiPhy::lane_mbps`);
//! - the ADV7535 is started over `ip::stm32_i2c` on `model_mp25_i2c` and its
//!   DSI timing generator is read back.
//!
//! The lane rate follows ST's `dw_mipi_dsi_phy_141_get_lane_mbps` (ST tree
//! `drivers/gpu/drm/stm/dw_mipi_dsi-stm.c:445-481`): `pll_out_khz = clock *
//! bpp / (lanes * 2)`, +20 % only in burst mode (the ADV7535 is not),
//! `lane_mbps = 2 * clkout_khz / 1000`: 445 Mbit/s for 720p60 and 891 Mbit/s
//! for 1080p60.
//!
//! The 720p60 PLL setting is ST's truncated one, pinned below: Linux's
//! `set_rate` (`:693-732`) searches again for `lane_mbps * 1000 / 2` = 222 500
//! kHz (`:695`), the whole Mbit/s of the first search, and lands on `idf 15,
//! ndiv 247` (WRPCR0 0xF5E, 444.6 Mbit/s). The exact setting would be `idf 4,
//! ndiv 66` (WRPCR0 0x403, 445.5 Mbit/s); `ip::stm32_dsi_phy::init` programs
//! that one (see `phy_init_programs_the_exact_720p_setting`, a difference the
//! report names; the IP module is not changed here).

use crate::env::{Env, Level};
use crate::errno;
use crate::ip::adv7535::{self as adv7535, Bridge, Sink};
use crate::ip::dw_mipi_dsi::{self as dsi, flags, Format, Link, Phy};
use crate::ip::stm32_dsi_phy::{self as dphy, regs as pr, StmDsiPhy};
use crate::ip::stm32_i2c::{self as i2c, Config as I2cConfig};
use crate::ip::stm32_ltdc::{self as ltdc, Layer, Ltdc, Output};
use crate::mmio::Mmio;
use crate::model::TestEnv;
use crate::model_mp25_i2c::{edid_block, Adv7535Config, Adv7535Handle, EdidMode, Mp25I2c, BASE as I2C2_BASE};
use crate::soc::stm32mp2::display_regs::{DSI_BASE, LTDC_BASE};
use crate::video::{self, VideoMode};
use std::cell::RefCell;
use std::collections::BTreeMap;

/// `FLEXGEN_28` = 27 MHz, the DSI PHY reference (ip::stm32_dsi_phy's text).
const REF_HZ: u32 = 27_000_000;
const LANES: u32 = 4;
const FB: u64 = 0x9000_0000;

/// A plain register file with the one status bit the PHY wait needs.
#[derive(Default)]
struct Regs {
    regs: RefCell<BTreeMap<usize, u32>>,
}

impl Regs {
    fn with(init: &[(usize, u32)]) -> Regs {
        let r = Regs::default();
        r.regs.borrow_mut().extend(init.iter().copied());
        r
    }
    fn peek(&self, addr: usize) -> u32 {
        self.regs.borrow().get(&addr).copied().unwrap_or(0)
    }
}

impl Mmio for Regs {
    fn read32(&self, addr: usize) -> u32 {
        if addr == DSI_BASE + pr::PSR as usize {
            return pr::PSR_PSSC;
        }
        self.peek(addr)
    }
    fn write32(&self, addr: usize, val: u32) {
        self.regs.borrow_mut().insert(addr, val);
    }
    fn read8(&self, _addr: usize) -> u8 {
        unreachable!("32-bit blocks")
    }
    fn write8(&self, _addr: usize, _val: u8) {
        unreachable!("32-bit blocks")
    }
}

fn ltdc_regs() -> Regs {
    // The MP25's HWVER 0x040101, a 128-bit bus (GC2R.BW = 3), as the ltdc tests.
    Regs::with(&[(LTDC_BASE + ltdc::IDR as usize, ltdc::HWVER_40101), (LTDC_BASE + ltdc::GC2R as usize, 3 << 4)])
}

// ---------------------------------------------------------------- the I2C side

/// The DK's I2C2 (model_mp25_i2c's tests): 100 MHz kernel clock, 400 kHz, SCL
/// rise 108 ns and fall 12 ns.
fn i2c_cfg() -> I2cConfig {
    let mut c = I2cConfig::new(100_000_000, 400_000);
    c.rise_ns = 108;
    c.fall_ns = 12;
    c
}

/// The shim's job: bus ids of ip::adv7535 to the DK's I2C addresses, the
/// transfers through ip::stm32_i2c on the model (copied from
/// model_mp25_i2c.rs's tests, where it is private).
struct DkEnv<'a>(&'a Mp25I2c);

fn addr_of(bus: u32) -> u8 {
    match bus {
        adv7535::BUS_MAIN => 0x3d,
        adv7535::BUS_DSI_CEC => 0x3c,
        adv7535::BUS_EDID => 0x3f,
        adv7535::BUS_PACKET => 0x38,
        _ => panic!("bus {bus}"),
    }
}

impl Env for DkEnv<'_> {
    fn busy_wait_us(&self, _us: u32) {}
    fn sleep_ms(&self, _ms: u32) {}
    fn log_str(&self, _l: Level, _line: &str) {}
    fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32> {
        i2c::write(self.0, I2C2_BASE, addr_of(bus), buf)
    }
    fn i2c_write_read(&self, bus: u32, w: &[u8], r: &mut [u8]) -> Result<(), i32> {
        i2c::write_read(self.0, I2C2_BASE, addr_of(bus), w, r)
    }
    fn gpio_set(&self, _l: u32, _v: bool) -> Result<(), i32> {
        Ok(())
    }
}

// ------------------------------------------------------------------- the chain

/// The ADV7535's host link (adv7533.c:152-153): RGB888, sync pulses, no burst.
fn link(mode: VideoMode) -> Link {
    Link { mode, lanes: LANES, channel: 0, format: Format::Rgb888, flags: flags::ADV7535 }
}

/// Decode the timing generator's 12-bit pairs (`reg` the high byte, `reg + 1`
/// the low nibble in bits 7:4).
fn tg(adv: &Adv7535Handle, reg: u8) -> u32 {
    (u32::from(adv.cec(reg)) << 4) | (u32::from(adv.cec(reg + 1)) >> 4)
}

fn vic_of(m: &VideoMode) -> u8 {
    if m.hactive == 1280 {
        4
    } else {
        16
    }
}

struct Up {
    plan: dsi::Plan,
    pll: dphy::Pll,
}

/// LTDC, DSI plan, PHY PLL and ADV7535 for `mode`; asserts they agree.
fn bring_up(mode: VideoMode, edid: Vec<u8>) -> Up {
    // --- LTDC: the timing registers (ltdc.c ltdc_crtc_mode_set_nofb)
    let regs = ltdc_regs();
    let l = Ltdc::probe(&regs, LTDC_BASE).unwrap();
    l.configure(&regs, &mode, &Output::default(), &Layer { phys: FB, pitch: 0, format: ltdc::Format::Xrgb8888 })
        .unwrap();
    let rd = |off: u32| regs.peek(LTDC_BASE + off as usize);
    let (sscr, bpcr, awcr, twcr) = (rd(ltdc::SSCR), rd(ltdc::BPCR), rd(ltdc::AWCR), rd(ltdc::TWCR));
    // each register holds the accumulated count - 1, horizontal in 27:16
    let hs = |r: u32| ((r >> 16) & 0xfff) + 1;
    let vs = |r: u32| (r & 0xfff) + 1;
    let (l_hsync, l_vsync) = (hs(sscr), vs(sscr));
    let (l_hback, l_vback) = (hs(bpcr) - hs(sscr), vs(bpcr) - vs(sscr));
    let (l_hactive, l_vactive) = (hs(awcr) - hs(bpcr), vs(awcr) - vs(bpcr));
    let (l_htotal, l_vtotal) = (hs(twcr), vs(twcr));
    let (l_hfront, l_vfront) = (l_htotal - hs(awcr), l_vtotal - vs(awcr));
    assert_eq!((l_hsync, l_hback, l_hactive, l_hfront), (mode.hsync, mode.hback, mode.hactive, mode.hfront));
    assert_eq!((l_vsync, l_vback, l_vactive, l_vfront), (mode.vsync, mode.vback, mode.vactive, mode.vfront));
    assert_eq!((l_htotal, l_vtotal), (mode.htotal(), mode.vtotal()));
    let gcr = rd(ltdc::GCR);
    assert_eq!(gcr & (ltdc::GCR_HSPOL | ltdc::GCR_VSPOL | ltdc::GCR_DEPOL), ltdc::GCR_HSPOL | ltdc::GCR_VSPOL);

    // --- PHY: ST's get_lane_mbps (dw_mipi_dsi-stm.c:445-481) through the Phy op
    let lk = link(mode);
    let mut phy = StmDsiPhy::new(DSI_BASE, REF_HZ);
    let mbps = phy.lane_mbps(&regs, &lk).unwrap();
    // pll_out_khz = clock * bpp / (lanes * 2), lane_mbps = 2 * clkout / 1000
    let pll_out_khz = mode.pixel_clock_hz / 1000 * 24 / (LANES * 2);
    assert_eq!(mbps, 2 * pll_out_khz / 1000);
    // ST's set_rate (:693-732) searches again for the whole Mbit/s.
    let pll = dphy::search(REF_HZ / 1000, mbps * 1000 / 2).unwrap();
    // The ADV7535's lane limit (adv7533.c) and the PHY's tolerance check
    // (mode_valid, :1177-1186) hold at the rate the PLL gives.
    assert!(pll.lane_kbps() <= adv7535::MAX_LANE_FREQ_KHZ);
    assert!(dphy::pixel_clock_within_tolerance(&pll, LANES, 24, mode.pixel_clock_hz / 1000));
    assert!(adv7535::mode_valid(&mode, LANES as u8));

    // --- DSI host: the video timing in lane byte clocks
    let timing = phy.timing(mbps).unwrap();
    let plan = dsi::plan(&lk, mbps, dsi::ESC_DEFAULT_MHZ, timing).unwrap();
    // Non-burst: N pixels take N * bpp / (lanes * 8) byte clocks, rounded
    // up, and never fewer than the host's minimum (dw-mipi-dsi.c:776-808).
    let lbcc = |px: u32| (px * 24).div_ceil(LANES * 8).max(dsi::MINIMUM_LBCCS[(LANES - 1) as usize]);
    assert_eq!(plan.hline, lbcc(l_htotal));
    assert_eq!(plan.hsa, lbcc(l_hsync));
    assert_eq!(plan.hbp, lbcc(l_hback));
    assert_eq!((plan.vsa, plan.vbp, plan.vfp, plan.vactive), (l_vsync, l_vback, l_vfront, l_vactive));
    // The host's line is the LTDC's line to within a byte clock plus the PLL's
    // 0.5 % tolerance, at the link's real byte clock (the PLL's).
    let line_ns_ltdc = u64::from(l_htotal) * 1_000_000_000 / u64::from(mode.pixel_clock_hz);
    let line_ns_dsi = u64::from(plan.hline) * 1_000_000_000 / u64::from(pll.byte_clock_hz());
    let tol = 1_000_000_000 / u64::from(pll.byte_clock_hz()) + line_ns_ltdc * 5 / 1000;
    assert!(line_ns_ltdc.abs_diff(line_ns_dsi) <= tol, "{line_ns_ltdc} vs {line_ns_dsi} ns");

    // --- ADV7535 over I2C2: the same mode and lanes
    let (bus, adv) = Mp25I2c::dk(Adv7535Config { edid, ..Adv7535Config::default() });
    i2c::init(&bus, I2C2_BASE, &i2c_cfg()).unwrap();
    let denv = DkEnv(&bus);
    let b = Bridge::new(&denv);
    let acfg = adv7535::Config::new(LANES as u8);
    b.init(&acfg).unwrap();
    let sink = Sink { hdmi: true, vic: vic_of(&mode), aspect_16_9: true, refresh_hz: 60 };
    b.start(&acfg, &mode, &sink).unwrap();
    assert_eq!(adv.cec(0x1c), (LANES as u8) << 4, "DSI lanes");
    assert_eq!(
        [tg(&adv, 0x28), tg(&adv, 0x2a), tg(&adv, 0x2c), tg(&adv, 0x2e)],
        [l_htotal, l_hsync, l_hfront, l_hback],
        "ADV7535 horizontal timing = LTDC"
    );
    assert_eq!(
        [tg(&adv, 0x30), tg(&adv, 0x32), tg(&adv, 0x34), tg(&adv, 0x36)],
        [l_vtotal, l_vsync, l_vfront, l_vback],
        "ADV7535 vertical timing = LTDC"
    );
    assert_eq!(adv.reg(0x52 + 3), 0x10, "AVI PB1");
    assert_eq!(adv.reg(0x52 + 6), sink.vic, "AVI PB4 = VIC");
    Up { plan, pll }
}

#[test]
fn chain_720p60_four_lanes() {
    let up = bring_up(video::HD720_60, edid_block(EdidMode::P720));
    assert_eq!(up.plan.lane_mbps, 445);
    // ST's truncated setting: idf 15, ndiv 247, odf 2, 444.6 Mbit/s.
    // dw_mipi_dsi-stm.c:693-695 searches for lane_mbps * 1000 / 2 = 222500
    // kHz, not the 222750 kHz of the mode (:1163). The exact setting is
    // idf 4, ndiv 66 (WRPCR0 0x403); it is not what ST's driver programs.
    assert_eq!((up.pll.idf, up.pll.ndiv, up.pll.odf), (15, 247, 2));
    let [wpcr1, wrpcr0, wrpcr1] = up.pll.regs().unwrap();
    assert_eq!(wrpcr0, 0x0000_0f5e);
    assert_eq!(wpcr1, 0x0000_1628);
    assert_eq!(wrpcr1, 0x1910_004b);
    assert_eq!(up.pll.lane_kbps(), 444_600);
}

#[test]
fn chain_1080p60_four_lanes() {
    let up = bring_up(video::FHD1080_60, edid_block(EdidMode::P1080));
    assert_eq!(up.plan.lane_mbps, 891);
    // 891 Mbit/s is exact either way: idf 4, ndiv 66, odf 1, WRPCR0 0x403.
    assert_eq!((up.pll.idf, up.pll.ndiv, up.pll.odf), (4, 66, 1));
    assert_eq!(up.pll.regs().unwrap()[1], 0x0000_0403);
    assert_eq!(up.pll.lane_kbps(), 891_000);
}

/// The EDID the sink presents picks the mode (`video::edid_preferred`), and
/// the chain built for that mode is consistent. A bad block falls back to
/// HD720_60.
#[test]
fn chain_follows_the_edid_preferred_mode_with_720p_fallback() {
    for (m, want) in [(EdidMode::P720, video::HD720_60), (EdidMode::P1080, video::FHD1080_60)] {
        let edid = edid_block(m);
        let (bus, _adv) = Mp25I2c::dk(Adv7535Config { edid: edid.clone(), ..Adv7535Config::default() });
        i2c::init(&bus, I2C2_BASE, &i2c_cfg()).unwrap();
        let denv = DkEnv(&bus);
        let b = Bridge::new(&denv);
        b.init(&adv7535::Config::new(LANES as u8)).unwrap();
        let mut blk = [0u8; 128];
        b.read_edid_block(0, &mut blk).unwrap();
        let mode = video::edid_preferred(&blk).unwrap_or(video::HD720_60);
        assert_eq!(mode, want);
        bring_up(mode, edid);
    }
    // A block with a bad checksum: Err(EINVAL), the fallback is 720p60.
    let mut bad = edid_block(EdidMode::P1080);
    bad[127] = bad[127].wrapping_add(1);
    assert_eq!(video::edid_preferred(&bad), Err(-errno::EINVAL));
    let mode = video::edid_preferred(&bad).unwrap_or(video::HD720_60);
    assert_eq!(mode, video::HD720_60);
    bring_up(mode, edid_block(EdidMode::P720));
}

/// The difference to ST's driver: `ip::stm32_dsi_phy::init` programs the PLL
/// for the rate it is given in kbit/s, so for 720p60 it writes the exact
/// idf 4, ndiv 66 (WRPCR0 0x403), where ST's `set_rate` writes 0xF5E.
#[test]
fn phy_init_programs_the_exact_720p_setting() {
    let regs = Regs::default();
    let env = TestEnv::default();
    let lane = dphy::lane_kbps_for(74_250, 24, 4, false);
    assert_eq!(lane, 445_500);
    let pll = dphy::init(&regs, &env, DSI_BASE, REF_HZ, lane, dphy::ColorCoding::Rgb888).unwrap();
    assert_eq!((pll.idf, pll.ndiv), (4, 66));
    assert_eq!(regs.peek(DSI_BASE + pr::WRPCR0 as usize), 0x403);
    // ST's: the same reference through the truncated rate.
    let st = dphy::search(27_000, 445 * 1000 / 2).unwrap();
    assert_ne!((st.idf, st.ndiv), (pll.idf, pll.ndiv));
}
