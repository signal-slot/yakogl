// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin's display chain from cold, as src/display/imx8mp_display.c runs
//! it with the Rust LCDIF and DSIM (driver/docs/imx8mp-display-cold.md):
//! media power and clocks (soc::imx8mp::media), LCDIF1 configured
//! (ip::lcdifv3), the DSIM attached with its data lanes held
//! (ip::samsung_dsim), the SN65DSI84 programmed and soft-reset
//! (ip::sn65dsi8x), LCDIF1 enabled, the data lanes released, the bridge's
//! link checked. Against one register model of every block, so the ordering
//! across blocks is visible, and against the board's Linux register image.

use super::media::tests::{bases, bring_up, bring_up_at, cold, golden};
use super::media::Track;
use crate::env::{Env, Level};
use crate::errno::{EIO, ETIMEDOUT};
use crate::ip::lcdifv3::{self as lcdif, Layer, Output, Panic};
use crate::ip::samsung_dsim::{self as dsim, step, Link};
use crate::ip::sn65dsi8x::{self, Board, Bridge};
use crate::model::{Access, BridgeModel, Faults, Model, TestEnv, BRIDGE_SOFT_RESET, CCM, DSIM, LCDIF1};
use crate::video::VideoMode;

/// The panel's timing at the board's Linux pixel clock (the golden dump's),
/// which is the devicetree's plan too ([`verdin_dt`]).
const RIVERDI: VideoMode = VideoMode {
    pixel_clock_hz: 69_300_000,
    hactive: 1280,
    hfront: 36,
    hsync: 24,
    hback: 64,
    vactive: 800,
    vfront: 8,
    vsync: 8,
    vback: 24,
    hsync_high: true,
    vsync_high: true,
    de_high: true,
};

/// LCDIF1 -> DSIM: HS, VS, DE active high, the DSIM sampling on the rising
/// edge (`INV_PXCK`, the board's Linux: CTRL 0x08).
const OUT: Output = Output { pxck_rising: true, hsync_low: false, vsync_low: false, de_low: false };
const FB: u64 = 0x9000_0000;
const PANIC: Panic = Panic { low: 170, high: 340 };

const BOARD: Board = Board {
    lanes: 4,
    lvds_24bpp: true,
    format1: false,
    term_100_ohm: false,
    hs_neg: false,
    vs_neg: false,
    de_neg: false,
    has_en: true,
    has_reset: false,
};

fn bridge_mode(m: &VideoMode) -> sn65dsi8x::Mode {
    sn65dsi8x::Mode {
        pixel_clock_hz: m.pixel_clock_hz,
        hactive: m.hactive as u16,
        vactive: m.vactive as u16,
        hsync: m.hsync as u16,
        vsync: m.vsync as u16,
        hbp: m.hback as u16,
        vbp: m.vback as u16,
        hfp: m.hfront as u16,
        vfp: m.vfront as u16,
    }
}

/// The bridge on I2C, watched: when its soft reset is written (data sheet
/// Init seq 7, the end of its programming) the DSIM must still hold its data
/// lanes in stop state with the clock lane requested in HS.
struct Watched<'a> {
    bridge: BridgeModel,
    m: &'a Model,
    soft_reset_seen_held: std::cell::Cell<Option<bool>>,
}

impl Env for Watched<'_> {
    fn busy_wait_us(&self, us: u32) {
        self.bridge.busy_wait_us(us)
    }
    fn sleep_ms(&self, ms: u32) {
        self.bridge.sleep_ms(ms)
    }
    fn log_str(&self, level: Level, line: &str) {
        self.bridge.log_str(level, line)
    }
    fn i2c_write(&self, bus: u32, buf: &[u8]) -> Result<(), i32> {
        if buf.first() == Some(&BRIDGE_SOFT_RESET) && buf.get(1).is_some_and(|v| v & 1 != 0) {
            let esc = self.m.peek(DSIM, 0x1c);
            let clk = self.m.peek(DSIM, 0x10);
            self.soft_reset_seen_held.set(Some(esc & (1 << 20) != 0 && clk & (1 << 31) != 0));
        }
        self.bridge.i2c_write(bus, buf)
    }
    fn i2c_write_read(&self, bus: u32, w: &[u8], r: &mut [u8]) -> Result<(), i32> {
        self.bridge.i2c_write_read(bus, w, r)
    }
    fn gpio_set(&self, line: u32, value: bool) -> Result<(), i32> {
        self.bridge.gpio_set(line, value)
    }
}

struct Run {
    m: Model,
    pixel_hz: u32,
    plan: Option<dsim::Plan>,
    held_at_bridge_reset: Option<bool>,
}

/// The cold path at the board's Linux plan (69.3 MHz); `video_pll1_hz` 0 is
/// the R4 plan (SYS_PLL2 / 14).
#[allow(clippy::result_large_err)] // a test: the model comes back for inspection
fn run(faults: Faults, video_pll1_hz: u32) -> Result<Run, (Model, dsim::Failed)> {
    run_at(faults, RIVERDI.pixel_clock_hz, video_pll1_hz)
}

/// The cold path for a pixel clock of `want`.
#[allow(clippy::result_large_err)]
fn run_at(faults: Faults, want: u32, video_pll1_hz: u32) -> Result<Run, (Model, dsim::Failed)> {
    let m = Model::with_faults(faults);
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let b = bases(&m);
    let pixel_hz = bring_up_at(&m, &b, &e, &mut t, want, video_pll1_hz).unwrap();
    let mode = VideoMode { pixel_clock_hz: pixel_hz, ..RIVERDI };
    let (lc, ds) = (m.base(LCDIF1), m.base(DSIM));

    lcdif::configure(&m, lc, &mode, &OUT, &Layer { phys: FB, pitch: 0, desc3_extra: 0 }).unwrap();
    let link = Link {
        mode,
        lanes: 4,
        pll_ref_hz: 12_000_000,
        // the devicetree's burst-clock-frequency; the R4 plan cannot use it
        // (426.6 Mbit/s x 4 < 71.43 MHz x 24) and derives its own
        lane_rate_hz: if video_pll1_hz != 0 { 426_600_000 } else { 0 },
        esc_max_hz: 20_000_000,
        burst: true,
    };
    let plan = match dsim::attach(&m, ds, &e, &link) {
        Ok(p) => p,
        Err(f) => return Err((m, f)),
    };
    let w = Watched { bridge: BridgeModel::new(), m: &m, soft_reset_seen_held: Default::default() };
    Bridge::new(&w).start(&BOARD, &bridge_mode(&mode), plan.pms.fout_hz).unwrap();
    lcdif::set_vblank_irq(&m, lc, true);
    lcdif::enable(&m, lc, &e, Some(PANIC)).unwrap();
    dsim::video_start(&m, ds);
    assert_eq!(Bridge::new(&w).check_link(), Ok(0));
    Ok(Run { pixel_hz, plan: Some(plan), held_at_bridge_reset: w.soft_reset_seen_held.get(), m })
}

fn up(video_pll1_hz: u32) -> Run {
    match run(Faults::default(), video_pll1_hz) {
        Ok(r) => r,
        Err((_, f)) => panic!("{f:?}"),
    }
}

fn pos(acc: &[Access], f: impl Fn(&Access) -> bool) -> usize {
    acc.iter().position(f).expect("access")
}

#[test]
fn cold_chain_order() {
    let r = up(1_039_500_000);
    let acc = r.m.accesses();
    let w = |blk: &'static str, off: u32| move |a: &Access| matches!(*a, Access::W(b, o, _) if b == blk && o == off);
    // the PHY reference root before the DSIM PLL is enabled
    let phy_ref = pos(&acc, w(CCM, 0xbd80));
    let pll_en = pos(&acc, |a| matches!(*a, Access::W(DSIM, 0x94, v) if v & (1 << 23) != 0));
    assert!(phy_ref < pll_en);
    // PLLTMR before PLLCTRL; the PLL polled stable before the software reset
    assert!(pos(&acc, w(DSIM, 0xa0)) < pll_en);
    let stable = pos(&acc, |a| matches!(*a, Access::R(DSIM, 0x04, v) if v & (1 << 31) != 0));
    let swrst = pos(&acc, |a| matches!(*a, Access::W(DSIM, 0x0c, 1)));
    assert!(pll_en < stable && stable < swrst);
    // everything the reset clears is written after it
    for off in [0x18u32, 0x1c, 0x20, 0x24, 0x28, 0x2c, 0xb4, 0xb8, 0xbc] {
        assert!(pos(&acc, w(DSIM, off)) > swrst, "{off:#x} before the software reset");
    }
    // the FIFO pointers initialised once, low then high
    assert_eq!(r.m.dsim_fifo_inits(), 1);
    // LCDIF1 configured before the DSIM is touched, enabled after it is held
    assert!(pos(&acc, w(LCDIF1, 0x200)) < pos(&acc, w(DSIM, 0xa0)));
    let held = pos(&acc, |a| matches!(*a, Access::W(DSIM, 0x1c, v) if v & (1 << 20) != 0));
    let standby = pos(&acc, |a| matches!(*a, Access::W(DSIM, 0x20, v) if v & (1 << 31) != 0));
    let lcdif_on = pos(&acc, |a| matches!(*a, Access::W(LCDIF1, 0x214, v) if v & (1 << 31) != 0));
    let released = pos(&acc, |a| matches!(*a, Access::W(DSIM, 0x1c, v) if v & (1 << 20) == 0));
    assert!(held < standby && standby < lcdif_on && lcdif_on < released);
    // the bridge was reset with the data lanes held and the clock lane in HS
    assert_eq!(r.held_at_bridge_reset, Some(true));
    assert!(dsim::pll_is_stable(&r.m, r.m.base(DSIM)) && dsim::hs_clock_is_ready(&r.m, r.m.base(DSIM)));
    assert!(!dsim::is_held(&r.m, r.m.base(DSIM)));
    assert_eq!(r.pixel_hz, 69_300_000);
}

/// LCDIF1 and DSIM registers this driver owns, with the bits it owns,
/// compared with the board's Linux (golden-linux-2026-09-21.txt).
const OWNED: &[(u32, &str, u32, u32)] = &[
    (0x32e8_0000, LCDIF1, 0x00, 0xffff_ffff), // CTRL
    (0x32e8_0010, LCDIF1, 0x10, 0xffff_ffff), // DISP_PARA
    (0x32e8_0014, LCDIF1, 0x14, 0xffff_ffff), // DISP_SIZE
    (0x32e8_0018, LCDIF1, 0x18, 0xffff_ffff), // HSYN_PARA
    (0x32e8_001c, LCDIF1, 0x1c, 0xffff_ffff), // VSYN_PARA
    (0x32e8_0020, LCDIF1, 0x20, 0xffff_ffff), // VSYN_HSYN_WIDTH
    (0x32e8_0028, LCDIF1, 0x28, 0xffff_ffff), // INT_ENABLE_D0
    (0x32e6_000c, DSIM, 0x0c, 0xffff_ffff),   // SWRST
    (0x32e6_0010, DSIM, 0x10, 0xffff_ffff),   // CLKCTRL
    (0x32e6_0014, DSIM, 0x14, 0xffff_ffff),   // TIMEOUT
    (0x32e6_0018, DSIM, 0x18, 0xffff_ffff),   // CONFIG
    (0x32e6_001c, DSIM, 0x1c, 0xffff_ffff),   // ESCMODE
    (0x32e6_0020, DSIM, 0x20, 0xffff_ffff),   // MDRESOL
    (0x32e6_0024, DSIM, 0x24, 0xffff_ffff),   // MVPORCH
    (0x32e6_0028, DSIM, 0x28, 0xffff_ffff),   // MHPORCH
    (0x32e6_002c, DSIM, 0x2c, 0xffff_ffff),   // MSYNC
    (0x32e6_004c, DSIM, 0x4c, 0x0000_001f),   // FIFOCTRL nInit (the rest is status)
    (0x32e6_0094, DSIM, 0x94, 0xffff_ffff),   // PLLCTRL
    (0x32e6_00a0, DSIM, 0xa0, 0xffff_ffff),   // PLLTMR
    (0x32e6_00a4, DSIM, 0xa4, 0xffff_ffff),   // PHYCTRL_B1
    (0x32e6_00b4, DSIM, 0xb4, 0xffff_ffff),   // PHYTIMING
    (0x32e6_00b8, DSIM, 0xb8, 0xffff_ffff),   // PHYTIMING1
    (0x32e6_00bc, DSIM, 0xbc, 0xffff_ffff),   // PHYTIMING2
];

/// Every expected difference, and why (driver/docs/imx8mp-display-cold.md).
const ALLOWED: &[(u32, &str)] = &[
    (0x32e6_0014, "TIMEOUT: the reset value 0x00FFFFFF kept (BTA / LP-read timeouts; no BTA or read is ever started)"),
    (
        0x32e6_0028,
        "MHPORCH HBP 43 = 64 px x 53.325 / 69.3 - 6; Linux's downstream driver writes 42 (no RM basis for either)",
    ),
    (0x32e6_00a0, "PLLTMR 250000 = 3000 PFD periods (Figure 13-69) at the 200 MHz PCLK maximum; Linux writes 0x8000"),
    (0x32e6_00b4, "PHYTIMING: HS_EXIT 6 from 100 ns rounded up; Linux's table has 5 (93.8 ns, under the minimum)"),
    (
        0x32e6_00b8,
        "PHYTIMING1: CLK_ZERO 13 / CLK_POST 10 from the D-PHY minimums; Linux's table 18 / 9 (CLK_POST under)",
    ),
    (0x32e6_00bc, "PHYTIMING2: HS_PREPARE 3, HS_ZERO 6, HS_TRAIL 4 from the minimums (n = 1); Linux's table 4 / 5 / 7"),
];

/// The R4 clock plan (71.43 MHz pixel clock): the lane rate follows the
/// pixel clock.
const ALLOWED_R4: &[(u32, &str)] = &[(
    0x32e6_0094,
    "PLLCTRL p 2 m 293 s 2 = 439.5 MHz: the slowest the PLL makes at or above the 439.29 Mbit/s a 71.43 MHz \
     stream needs over four lanes with LINK_MARGIN_PERMILLE (the devicetree's 426.6 cannot carry it)",
)];

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
    for a in allowed {
        assert!(diffs.iter().any(|d| d.0 == a.0), "allow-list entry {:#x} does not differ: {}", a.0, a.1);
    }
}

#[test]
fn golden_cold_chain_matches_linux_except_the_allow_list() {
    let r = up(1_039_500_000);
    compare(&r.m, ALLOWED);
    let p = r.plan.unwrap();
    assert_eq!((p.pms.p, p.pms.m, p.pms.s), (5, 711, 2));
    // the computed timings, as the design note tabulates them
    assert_eq!(r.m.peek(DSIM, 0xb4), 0x0000_0306);
    assert_eq!(r.m.peek(DSIM, 0xb8), 0x030d_0a04);
    assert_eq!(r.m.peek(DSIM, 0xbc), 0x0003_0604);
}

#[test]
fn golden_r4_plan_without_video_pll1() {
    let r = up(0);
    assert_eq!(r.pixel_hz, 71_428_571);
    // at 439.5 MHz / 71.43 MHz HBP comes out 43, as at 69.3 MHz on 426.6
    let mut all: Vec<_> = ALLOWED.to_vec();
    all.extend_from_slice(ALLOWED_R4);
    compare(&r.m, &all);
}

#[test]
fn no_pll_reference_is_a_timeout_in_the_pll_step() {
    let Err((m, f)) = run(Faults { dsim_pll_no_lock: true, ..Default::default() }, 1_039_500_000) else {
        panic!("attached without a PLL");
    };
    assert_eq!(f, dsim::Failed { step: step::PLL, errno: -ETIMEDOUT });
    // nothing after the PLL was written: no reset, no lanes
    assert!(!m.writes().iter().any(|w| w.0 == DSIM && (w.1 == 0x0c || w.1 == 0x18)));
}

#[test]
fn a_stuck_software_reset_and_stuck_lanes_are_reported_with_their_step() {
    let Err((_, f)) = run(Faults { dsim_swrst_stuck: true, ..Default::default() }, 1_039_500_000) else {
        panic!("attached through a stuck reset");
    };
    assert_eq!(f, dsim::Failed { step: step::RESET, errno: -ETIMEDOUT });
    let Err((m, f)) = run(Faults { dsim_lanes_stuck: true, ..Default::default() }, 1_039_500_000) else {
        panic!("attached with stuck lanes");
    };
    assert_eq!(f, dsim::Failed { step: step::LANES, errno: -EIO });
    assert!(!dsim::is_held(&m, m.base(DSIM)), "lanes never released, never held either");
}

#[test]
fn without_the_phy_reference_root_the_pll_never_settles() {
    // the chain with MEDIA_MIPI_PHY1_REF gated after the clock plan: the
    // model's PLL needs its reference, the driver reports the PLL step
    let m = Model::new();
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    let b = bases(&m);
    let hz = bring_up(&m, &b, &e, &mut t, 1_039_500_000).unwrap();
    let root = m.peek(CCM, 0xbd80);
    m.poke(CCM, 0xbd80, root & !(1 << 28));
    let link = Link {
        mode: VideoMode { pixel_clock_hz: hz, ..RIVERDI },
        lanes: 4,
        pll_ref_hz: 12_000_000,
        lane_rate_hz: 426_600_000,
        esc_max_hz: 0,
        burst: true,
    };
    assert_eq!(
        dsim::attach(&m, m.base(DSIM), &e, &link).map(|_| ()),
        Err(dsim::Failed { step: step::PLL, errno: -ETIMEDOUT })
    );
    assert!(*e.waited_us.borrow() >= u64::from(dsim::PLL_WAIT_US));
}

#[test]
fn a_bad_link_writes_nothing() {
    let m = Model::new();
    cold(&m);
    let e = TestEnv::default();
    let mut t = Track::default();
    bring_up(&m, &bases(&m), &e, &mut t, 1_039_500_000).unwrap();
    m.clear_log();
    let link =
        Link { mode: RIVERDI, lanes: 1, pll_ref_hz: 12_000_000, lane_rate_hz: 426_600_000, esc_max_hz: 0, burst: true };
    assert_eq!(dsim::attach(&m, m.base(DSIM), &e, &link).map(|_| ()).unwrap_err().step, step::PLAN);
    assert!(m.writes().is_empty());
}

#[test]
fn disable_holds_the_lanes_and_stops_the_pll() {
    let r = up(1_039_500_000);
    let ds = r.m.base(DSIM);
    dsim::disable(&r.m, ds);
    assert!(dsim::is_held(&r.m, ds));
    assert!(!dsim::pll_is_stable(&r.m, ds));
    assert_eq!(r.m.peek(DSIM, 0x20) & (1 << 31), 0, "no standby");
    assert_eq!(r.m.peek(DSIM, 0x10) & 0x91f8_0000, 0, "clocks and HS request off");
    // and the panel side: LCDIF1 stops at a frame boundary
    let e = TestEnv::default();
    lcdif::disable(&r.m, r.m.base(LCDIF1), &e).unwrap();
    assert!(!lcdif::is_running(&r.m, r.m.base(LCDIF1)));
}

/// The devicetree's clock plan for the Verdin
/// (samples/zephyr/common/verdin_imx8mp_display.dtsi): the board's Linux
/// plan, 69.3 MHz on a 426.6 Mbit/s link, 58.76 Hz. Read from the file, so a
/// change there is checked here against the four limits it must satisfy.
mod verdin_dt {
    use super::*;
    use crate::errno::EINVAL;
    use crate::model::ANATOP;
    use crate::soc::imx8mp::anatop;
    use crate::soc::imx8mp::ccm::{mux, root};
    use crate::soc::imx8mp::media::plan_root;

    const DTSI: &str = include_str!("../../../../samples/zephyr/common/verdin_imx8mp_display.dtsi");

    /// The panel's clock range: its `panel-timing` in the board's Linux
    /// devicetree (/proc/device-tree/panel-rvt101hvdwc00b, read 2026-10-02:
    /// clock-frequency 66300000 71100000 78900000).
    const PANEL_MIN_HZ: u32 = 66_300_000;
    const PANEL_MAX_HZ: u32 = 78_900_000;

    /// The text of the node whose header line contains `label`.
    fn node(label: &str) -> &'static str {
        let start = DTSI.find(label).unwrap_or_else(|| panic!("{label} not in the dtsi"));
        let rest = &DTSI[start..];
        &rest[..rest.find("\n\t\t};").expect("node end")]
    }

    /// `name = <N>;` in a node.
    fn prop(node: &str, name: &str) -> u32 {
        let pre = format!("{name} = <");
        let line = node
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(&pre))
            .unwrap_or_else(|| panic!("{name} not in the node"));
        line[pre.len()..line.find('>').unwrap()].parse().unwrap()
    }

    struct Dt {
        pixel_hz: u32,
        video_pll1_hz: u32,
        lane_hz: u32,
        lanes: u32,
        mode: VideoMode,
    }

    fn dt() -> Dt {
        let d = node("display0: display@");
        let dsi = node("mipi_dsi: dsi@");
        let pixel_hz = prop(d, "clock-frequency");
        assert_eq!(prop(dsi, "pixel-clock-frequency"), pixel_hz, "the DSIM node repeats display0's clock");
        assert_eq!(prop(dsi, "burst-clock-frequency"), prop(d, "burst-clock-frequency"));
        assert_eq!(prop(dsi, "data-lanes"), prop(d, "dsi-lanes"));
        assert_eq!(prop(d, "pll-ref-frequency"), 12_000_000);
        let mode = VideoMode {
            pixel_clock_hz: pixel_hz,
            hactive: prop(d, "hactive"),
            hfront: prop(d, "hfront-porch"),
            hsync: prop(d, "hsync-len"),
            hback: prop(d, "hback-porch"),
            vactive: prop(d, "vactive"),
            vfront: prop(d, "vfront-porch"),
            vsync: prop(d, "vsync-len"),
            vback: prop(d, "vback-porch"),
            ..RIVERDI
        };
        assert_eq!(mode, VideoMode { pixel_clock_hz: pixel_hz, ..RIVERDI }, "the panel timing itself is unchanged");
        Dt {
            pixel_hz,
            video_pll1_hz: prop(d, "video-pll1-frequency"),
            lane_hz: prop(d, "burst-clock-frequency"),
            lanes: prop(d, "dsi-lanes"),
            mode,
        }
    }

    fn link(d: &Dt, pixel_clock_hz: u32) -> Link {
        Link {
            mode: VideoMode { pixel_clock_hz, ..d.mode },
            lanes: d.lanes,
            pll_ref_hz: 12_000_000,
            lane_rate_hz: d.lane_hz,
            esc_max_hz: 20_000_000,
            burst: true,
        }
    }

    #[test]
    fn the_plan_is_linux_s_69_3_mhz_and_58_76_hz() {
        let d = dt();
        assert_eq!((d.pixel_hz, d.video_pll1_hz, d.lane_hz), (69_300_000, 1_039_500_000, 426_600_000));
        let total = (d.mode.hactive + d.mode.hfront + d.mode.hsync + d.mode.hback)
            * (d.mode.vactive + d.mode.vfront + d.mode.vsync + d.mode.vback);
        assert_eq!(total, 1404 * 840);
        assert_eq!(u64::from(d.pixel_hz) * 1000 / u64::from(total), 58_760); // 58.7607 Hz
    }

    #[test]
    fn limit_1_the_panel() {
        assert!((PANEL_MIN_HZ..=PANEL_MAX_HZ).contains(&dt().pixel_hz));
    }

    #[test]
    fn limit_2_the_dsi_link_carries_it() {
        let d = dt();
        let p = dsim::plan(&link(&d, d.pixel_hz)).unwrap();
        assert_eq!((p.pms.p, p.pms.m, p.pms.s, p.pms.fout_hz), (5, 711, 2, 426_600_000));
        assert_eq!((p.byte_hz, p.esc_div, p.esc_hz), (53_325_000, 3, 17_775_000));
        // HFP and HSA are Linux's (MHPORCH 0x0015002a, HSA 12); HBP is 43
        // at 69.3 MHz, Linux's 42 being a 71.1 MHz figure (ran 2026-09-24)
        assert_eq!((p.hfp, p.hbp, p.hsa), (21, 43, 12));
    }

    /// The fourth limit: the lane rate carries the pixel stream AND every
    /// line's packet overhead (ip::samsung_dsim::LINK_MARGIN_PERMILLE, the
    /// board's measured bound). A devicetree with no margin -- 71.1 MHz on
    /// 426.6 Mbit/s, the 2026-10-06 picture with 1.5 copies a line -- fails
    /// here by name.
    #[test]
    fn limit_4_the_dsi_link_has_the_packet_overhead_margin() {
        let d = dt();
        let need = dsim::min_lane_rate(d.pixel_hz, d.lanes);
        assert!(
            u64::from(d.lane_hz) >= need,
            "the DSI link has no margin for packet overhead: {} Hz x 24 / {} lanes needs a lane rate of at least \
             {need} Hz with LINK_MARGIN_PERMILLE = {} (ip::samsung_dsim), burst-clock-frequency is {} Hz \
             (71.1 MHz on 426.6 Mbit/s broke the picture on 2026-10-06, 71.43 on 428.6 on 2026-09-23)",
            d.pixel_hz,
            d.lanes,
            dsim::LINK_MARGIN_PERMILLE,
            d.lane_hz,
        );
        assert!(dsim::plan(&link(&d, d.pixel_hz)).is_ok());
        // the headroom the board was seen right at: 426.6 / 415.8 = 25.97 per mille
        let headroom = (u64::from(d.lane_hz) * 100_000 / (u64::from(d.pixel_hz) * 24 / u64::from(d.lanes))) - 100_000;
        assert_eq!(headroom, 2597);
        // and the two plans that broke it are refused on this link
        assert_eq!(dsim::plan(&link(&d, 71_100_000)), Err(-EINVAL), "71.1 MHz: no margin");
        assert_eq!(dsim::plan(&link(&d, 71_428_571)), Err(-EINVAL), "71.43 MHz: over the link");
    }

    #[test]
    fn limit_2_the_bridge_range_follows() {
        let d = dt();
        let p = sn65dsi8x::plan(&BOARD, &bridge_mode(&d.mode), d.lane_hz).unwrap();
        assert_eq!(p.dsi_clk_hz, 213_300_000);
        assert_eq!(p.dsi_clk_range, 42); // 210 <= f < 215 MHz
        assert_eq!(p.divider, 3);
        // the LVDS clock is the DSI clock / 3 = 71.1 MHz, not the pixel
        // clock: Linux's bridge has the same (0x0a = 0x85, 0x0b = 0x10)
        assert_eq!(p.lvds_clk_hz, 71_100_000);
        assert_eq!(p.lvds_clk_range, 2);
    }

    #[test]
    fn limit_3_video_pll1_makes_it_exactly() {
        let d = dt();
        let divs = anatop::video_pll1_divs(d.video_pll1_hz).unwrap();
        // m + k / 65536 = 1039.5 MHz x 2 x 2 / 24 MHz = 173.25: Linux's
        // FDIV_CTL0 0x000ad021, FDIV_CTL1 0x4000 (golden dump)
        assert_eq!((divs.m, divs.p, divs.s, divs.k), (173, 2, 1, 16384));
        assert_eq!(divs.rate_hz(), u64::from(d.video_pll1_hz));
        assert_eq!(divs.rate_hz() << divs.s, 2_079_000_000);
        // the pixel root takes VIDEO_PLL1 / 15 over every other running parent
        let parents = [
            (mux::DISP1_PIX_VIDEO_PLL1, d.video_pll1_hz),
            (mux::DISP1_PIX_SYS_PLL2, 1_000_000_000),
            (mux::DISP1_PIX_SYS_PLL1, 800_000_000),
            (mux::OSC_24M, 24_000_000),
        ];
        let (setting, hz) = plan_root(d.pixel_hz, &parents, root::MEDIA_DISP1_PIX_MAX_HZ).unwrap();
        assert_eq!((setting.mux, setting.pre, setting.post, hz), (mux::DISP1_PIX_VIDEO_PLL1, 1, 15, 69_300_000));
    }

    #[test]
    fn the_cold_chain_at_the_plan_matches_linux_except_the_allow_list() {
        let d = dt();
        let r = match run_at(Faults::default(), d.pixel_hz, d.video_pll1_hz) {
            Ok(r) => r,
            Err((_, f)) => panic!("{f:?}"),
        };
        assert_eq!(r.pixel_hz, 69_300_000);
        // VIDEO_PLL1 as Linux has it: FDIV_CTL0 0x000ad021, DSM 0x4000
        assert_eq!(r.m.peek(ANATOP, 0x2c), 0x000a_d021);
        assert_eq!(r.m.peek(ANATOP, 0x30) & 0xffff, 0x4000);
        compare(&r.m, ALLOWED);
        assert_eq!(r.held_at_bridge_reset, Some(true));
    }
}
