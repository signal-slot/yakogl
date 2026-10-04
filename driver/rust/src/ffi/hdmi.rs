// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! C entry points of the i.MX 8M Plus HDMI display (`yakogl,imx8mp-hdmi`;
//! the shim is driver/drivers/display/display_yakogl_imx8mp_hdmi.c). The
//! structs are pinned against include/yakogl_drivers.h.
//!
//! Policy (the shim's `imx8mp_display_*` facade calls these): with
//! [`flags::FORCE_COLD`] the chain is always brought up from cold
//! (soc::imx8mp::hdmi_chain); without it a chain found scanning is adopted
//! the way src/display/imx8mp_lcdif_adopt.c does (re-point the layer, keep
//! everything downstream), and only a dark one is brought up.

use crate::env::{Env, YdEnv};
use crate::errno::{EINVAL, ENODEV};
use crate::ip::dw_hdmi_tx::PhyIf;
use crate::ip::lcdifv3::{self as lcdif, Layer, Panic};
use crate::mmio::{Hw, Mmio};
use crate::soc::imx8mp::hdmi::ClockSel;
use crate::soc::imx8mp::hdmi_chain::{self as chain, Bases, Config, TestPicture};
use crate::video::VideoMode;
use crate::ylog;
use core::mem::{offset_of, size_of};

/// `YD_IMX8MP_HDMI_*` flags of `struct yd_imx8mp_hdmi_config`.
pub mod flags {
    pub const HSYNC_HIGH: u32 = 1 << 0;
    pub const VSYNC_HIGH: u32 = 1 << 1;
    pub const DE_HIGH: u32 = 1 << 2;
    pub const PXCK_RISING: u32 = 1 << 3;
    pub const FORCE_COLD: u32 = 1 << 4;
    pub const POWER_CYCLE: u32 = 1 << 5;
    pub const PIPE_SEL_SET: u32 = 1 << 6;
    pub const PIPE_SEL_ONE: u32 = 1 << 7;
    pub const PHY_SEL_SET: u32 = 1 << 8;
    pub const PHY_SEL_ONE: u32 = 1 << 9;
    pub const CONF0_SET: u32 = 1 << 10;
    pub const PHYRSTZ_SET: u32 = 1 << 11;
    pub const TEST_FORCED: u32 = 1 << 12;
    pub const TEST_BGND: u32 = 1 << 13;
}

/// `struct yd_imx8mp_hdmi_config`: from the devicetree.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct YdImx8mpHdmiConfig {
    pub pixel_clock_hz: u32,
    pub hactive: u32,
    pub hfront: u32,
    pub hsync: u32,
    pub hback: u32,
    pub vactive: u32,
    pub vfront: u32,
    pub vsync: u32,
    pub vback: u32,
    pub flags: u32,
    pub max_ppm: u32,
    pub desc3_extra: u32,
    pub noc_hurry: u32,
    /// Both 0: no FIFO panic thresholds.
    pub panic_low: u32,
    pub panic_high: u32,
    pub stop_after: u32,
    pub test_rgb: u32,
    pub phy_conf0: u8,
    pub phyrstz: u8,
    pub reserved: [u8; 2],
}

/// `struct yd_imx8mp_hdmi_data`: per instance, mutable.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct YdImx8mpHdmiData {
    pub lcdif: usize,
    pub hdmi: usize,
    pub phy: usize,
    pub blk: usize,
    pub pvi: usize,
    pub gpc: usize,
    pub ccm: usize,
    /// The buffer we scan.
    pub fb: u64,
    /// Adopted: what the previous owner scanned, and its pitch / format.
    pub prev_fb: u64,
    pub prev_desc3: u32,
    pub prev_desc5: u32,
    pub width: u32,
    pub height: u32,
    pub pitch: u32,
    pub tmds_hz: u32,
    pub err_ppm: i32,
    /// P | M << 8 | S << 16.
    pub pll_pms: u32,
    pub refresh_mhz: u32,
    /// The stage reached, or the stage that failed.
    pub stage: u32,
    pub error: i32,
    pub on: u8,
    pub adopted: u8,
    pub reserved: [u8; 2],
}

const _: () = assert!(size_of::<YdImx8mpHdmiConfig>() == 72);
const _: () = assert!(offset_of!(YdImx8mpHdmiConfig, vback) == 32);
const _: () = assert!(offset_of!(YdImx8mpHdmiConfig, flags) == 36);
const _: () = assert!(offset_of!(YdImx8mpHdmiConfig, stop_after) == 60);
const _: () = assert!(offset_of!(YdImx8mpHdmiConfig, test_rgb) == 64);
const _: () = assert!(offset_of!(YdImx8mpHdmiConfig, phy_conf0) == 68);
const _: () = assert!(offset_of!(YdImx8mpHdmiConfig, phyrstz) == 69);
const _: () = assert!(size_of::<YdImx8mpHdmiData>() == 120);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, ccm) == 48);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, fb) == 56);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, prev_fb) == 64);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, prev_desc3) == 72);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, width) == 80);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, tmds_hz) == 92);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, err_ppm) == 96);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, stage) == 108);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, error) == 112);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, on) == 116);
const _: () = assert!(offset_of!(YdImx8mpHdmiData, adopted) == 117);

// The health bits and stage numbers as include/yakogl_drivers.h states them.
const _: () = {
    use chain::health_bit::*;
    assert!(LCDIF_RUNNING == 1 << 0 && HDMIMIX_UP == 1 << 8 && BLK_ENABLED == 1 << 9);
    assert!(PHY_PLL_LOCK == 1 << 10 && PHY_READY == 1 << 11 && HPD == 1 << 12);
    assert!(TMDS_CLOCK == 1 << 13 && PIXEL_CLOCK == 1 << 14 && PVI_ON == 1 << 15);
    assert!(PVI_FIFO_ERROR == 1 << 16 && RX_SENSE == 1 << 17);
    assert!(chain::stage::START == 1 && chain::stage::PHY == 4 && chain::stage::VIDEO == 8);
};

fn bases(d: &YdImx8mpHdmiData) -> Bases {
    Bases { lcdif: d.lcdif, hdmi: d.hdmi, phy: d.phy, blk: d.blk, pvi: d.pvi, gpc: d.gpc, ccm: d.ccm }
}

/// The chain configuration a devicetree configuration describes.
pub fn config(c: &YdImx8mpHdmiConfig) -> Result<Config, i32> {
    use flags::*;
    let f = c.flags;
    let mode = VideoMode {
        pixel_clock_hz: c.pixel_clock_hz,
        hactive: c.hactive,
        hfront: c.hfront,
        hsync: c.hsync,
        hback: c.hback,
        vactive: c.vactive,
        vfront: c.vfront,
        vsync: c.vsync,
        vback: c.vback,
        hsync_high: f & HSYNC_HIGH != 0,
        vsync_high: f & VSYNC_HIGH != 0,
        de_high: f & DE_HIGH != 0,
    };
    mode.check()?;
    if f & TEST_FORCED != 0 && f & TEST_BGND != 0 {
        return Err(-EINVAL);
    }
    let pick = |set, one| if f & set != 0 { Some(f & one != 0) } else { None };
    Ok(Config {
        mode,
        max_ppm: c.max_ppm,
        clock_sel: ClockSel { htx_pipe: pick(PIPE_SEL_SET, PIPE_SEL_ONE), htxphy: pick(PHY_SEL_SET, PHY_SEL_ONE) },
        phy_if: PhyIf {
            conf0: if f & CONF0_SET != 0 { Some(c.phy_conf0) } else { None },
            phyrstz: if f & PHYRSTZ_SET != 0 { Some(c.phyrstz) } else { None },
        },
        lcdif_pxck_rising: f & PXCK_RISING != 0,
        desc3_extra: c.desc3_extra,
        noc_hurry: c.noc_hurry,
        panic: if c.panic_low == 0 && c.panic_high == 0 {
            None
        } else {
            Some(Panic { low: c.panic_low, high: c.panic_high })
        },
        power_cycle: f & POWER_CYCLE != 0,
        stop_after: c.stop_after,
        test: if f & TEST_FORCED != 0 {
            TestPicture::HdmiForced
        } else if f & TEST_BGND != 0 {
            TestPicture::LcdifBackground(c.test_rgb)
        } else {
            TestPicture::None
        },
    })
}

fn adopt(m: &impl Mmio, c: &YdImx8mpHdmiConfig, d: &mut YdImx8mpHdmiData, env: &impl Env, fb: u64, pitch: u32) -> i32 {
    let b = bases(d);
    let Some(s) = chain::scanout(m, &b) else {
        return -ENODEV;
    };
    if s.width != c.hactive || s.height != c.vactive {
        ylog!(env, Err, "hdmi: LCDIF3 runs {}x{}, the devicetree says {}x{}", s.width, s.height, c.hactive, c.vactive);
        return -EINVAL;
    }
    d.prev_fb = s.phys;
    d.prev_desc3 = m.read32(d.lcdif + crate::soc::imx8mp::regs::lcdif::CTRLDESCL0_3 as usize);
    d.prev_desc5 = lcdif::layer_control(m, d.lcdif);
    d.adopted = 1;
    d.width = s.width;
    d.height = s.height;
    if fb == 0 {
        // keep what runs (the address and the layout)
        d.fb = s.phys;
        d.pitch = s.pitch;
        d.on = 1;
        return 0;
    }
    let pitch = if pitch == 0 { s.width * 4 } else { pitch };
    if let Err(e) = lcdif::set_layout(m, d.lcdif, pitch, lcdif::BPP_ARGB8888) {
        return e;
    }
    if let Err(e) = lcdif::set_fb(m, d.lcdif, env, fb) {
        return e;
    }
    ylog!(env, Inf, "hdmi: adopted {}x{} at {:#x}, now ours at {:#x} pitch {}", s.width, s.height, s.phys, fb, pitch);
    d.fb = fb;
    d.pitch = pitch;
    d.on = 1;
    0
}

/// Start the display on `fb` (see the module text for the policy).
pub fn on(m: &impl Mmio, c: &YdImx8mpHdmiConfig, d: &mut YdImx8mpHdmiData, env: &impl Env, fb: u64, pitch: u32) -> i32 {
    let cfg = match config(c) {
        Ok(cfg) => cfg,
        Err(e) => {
            ylog!(env, Err, "hdmi: invalid devicetree configuration");
            return e;
        }
    };
    let b = bases(d);
    if c.flags & flags::FORCE_COLD == 0 && chain::is_running(m, &b) {
        return adopt(m, c, d, env, fb, pitch);
    }
    if fb == 0 {
        return -EINVAL;
    }
    d.adopted = 0;
    match chain::bring_up(m, &b, env, &cfg, &Layer { phys: fb, pitch, desc3_extra: cfg.desc3_extra }) {
        Ok(r) => {
            d.fb = fb;
            d.width = cfg.mode.hactive;
            d.height = cfg.mode.vactive;
            d.pitch = r.pitch;
            d.tmds_hz = r.pll.tmds_hz;
            d.err_ppm = r.pll.err_ppm;
            d.pll_pms = r.pll.p | r.pll.m << 8 | r.pll.s << 16;
            d.refresh_mhz = r.refresh_mhz;
            d.stage = chain::stage::VIDEO;
            d.error = 0;
            d.on = 1;
            0
        }
        Err(f) => {
            d.stage = f.stage;
            d.error = f.errno;
            if f.errno == -crate::errno::ECANCELED {
                ylog!(env, Wrn, "hdmi: stopped after stage {} (stop-after)", f.stage);
            } else {
                ylog!(env, Err, "hdmi: failed in stage {}: {}", f.stage, f.errno);
            }
            f.errno
        }
    }
}

pub fn set_fb(m: &impl Mmio, d: &mut YdImx8mpHdmiData, env: &impl Env, fb: u64) -> i32 {
    if d.on == 0 {
        return -ENODEV;
    }
    match lcdif::set_fb(m, d.lcdif, env, fb) {
        Ok(()) => {
            d.fb = fb;
            0
        }
        Err(e) => e,
    }
}

/// An adopted chain gets the previous owner's buffer and layout back; a
/// cold one stops scanning and stays powered.
pub fn off(m: &impl Mmio, d: &mut YdImx8mpHdmiData, env: &impl Env) {
    if d.on == 0 {
        return;
    }
    if d.adopted != 0 {
        use crate::soc::imx8mp::regs::lcdif as r;
        m.write32(d.lcdif + r::CTRLDESCL0_3 as usize, d.prev_desc3);
        m.write32(d.lcdif + r::CTRLDESCL0_5 as usize, d.prev_desc5 & !r::ctrldescl0_5::SHADOW_LOAD_EN_MASK);
        let _ = lcdif::set_fb(m, d.lcdif, env, d.prev_fb);
    } else if chain::is_running(m, &bases(d)) {
        let _ = lcdif::disable(m, d.lcdif, env);
    }
    d.on = 0;
}

// ---- the C entry points ------------------------------------------------------

/// # Safety
/// `cfg`, `data`, `env` valid for the call; `data`'s bases mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hdmi_on(
    cfg: *const YdImx8mpHdmiConfig,
    data: *mut YdImx8mpHdmiData,
    env: *const YdEnv,
    fb_phys: u64,
    pitch: u32,
) -> i32 {
    // SAFETY: the caller's contract; the bases are the shim's mappings.
    unsafe { on(&Hw::new(), &*cfg, &mut *data, &*env, fb_phys, pitch) }
}

/// # Safety
/// As [`yd_imx8mp_hdmi_on`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hdmi_set_fb(data: *mut YdImx8mpHdmiData, env: *const YdEnv, fb_phys: u64) -> i32 {
    // SAFETY: as above.
    unsafe { set_fb(&Hw::new(), &mut *data, &*env, fb_phys) }
}

/// # Safety
/// As [`yd_imx8mp_hdmi_on`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hdmi_off(data: *mut YdImx8mpHdmiData, env: *const YdEnv) {
    // SAFETY: as above.
    unsafe { off(&Hw::new(), &mut *data, &*env) }
}

/// soc::imx8mp::hdmi_chain::health_bit; reads only blocks that are reachable.
///
/// # Safety
/// `data` valid for the call, its bases mapped.
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hdmi_health(data: *const YdImx8mpHdmiData) -> u32 {
    // SAFETY: as above.
    unsafe { chain::health(&Hw::new(), &bases(&*data)) }
}

/// The address LCDIF3 scans; 0 when it does not, or cannot be read.
///
/// # Safety
/// As [`yd_imx8mp_hdmi_health`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hdmi_scanout(data: *const YdImx8mpHdmiData) -> u64 {
    // SAFETY: as above.
    unsafe { chain::scanout(&Hw::new(), &bases(&*data)).map_or(0, |s| s.phys) }
}

/// # Safety
/// As [`yd_imx8mp_hdmi_health`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hdmi_is_running(data: *const YdImx8mpHdmiData) -> bool {
    // SAFETY: as above.
    unsafe { chain::is_running(&Hw::new(), &bases(&*data)) }
}

/// `CTRLDESCL0_5` of LCDIF3; 0 when it cannot be read.
///
/// # Safety
/// As [`yd_imx8mp_hdmi_health`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx8mp_hdmi_layer_control(data: *const YdImx8mpHdmiData) -> u32 {
    // SAFETY: as above.
    unsafe {
        let hw = Hw::new();
        let b = bases(&*data);
        if chain::scanout(&hw, &b).is_some() || chain::is_running(&hw, &b) {
            lcdif::layer_control(&hw, b.lcdif)
        } else {
            0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errno::ECANCELED;
    use crate::model::TestEnv;
    use crate::model_hdmi::{HdmiModel, BLK, CCM, GPC, HDMI, LCDIF, PHY, PVI};

    /// What the Armadillo-X2 devicetree node gives (armadillo_x2_mimx8ml8_a53.dts).
    const X2: YdImx8mpHdmiConfig = YdImx8mpHdmiConfig {
        pixel_clock_hz: 154_000_000,
        hactive: 1920,
        hfront: 48,
        hsync: 32,
        hback: 80,
        vactive: 1200,
        vfront: 3,
        vsync: 6,
        vback: 26,
        flags: flags::HSYNC_HIGH | flags::DE_HIGH | flags::FORCE_COLD,
        max_ppm: 5000,
        desc3_extra: 0,
        noc_hurry: 7,
        panic_low: 170,
        panic_high: 340,
        stop_after: 0,
        test_rgb: 0,
        phy_conf0: 0,
        phyrstz: 0,
        reserved: [0; 2],
    };

    fn data(m: &HdmiModel) -> YdImx8mpHdmiData {
        YdImx8mpHdmiData {
            lcdif: m.base(LCDIF),
            hdmi: m.base(HDMI),
            phy: m.base(PHY),
            blk: m.base(BLK),
            pvi: m.base(PVI),
            gpc: m.base(GPC),
            ccm: m.base(CCM),
            ..Default::default()
        }
    }

    #[test]
    fn config_maps_the_flags() {
        let c = config(&YdImx8mpHdmiConfig {
            flags: X2.flags | flags::PIPE_SEL_SET | flags::PHY_SEL_SET | flags::PHY_SEL_ONE | flags::CONF0_SET,
            phy_conf0: 0x0e,
            ..X2
        })
        .unwrap();
        assert!(c.mode.hsync_high && !c.mode.vsync_high && c.mode.de_high);
        assert_eq!(c.clock_sel, ClockSel { htx_pipe: Some(false), htxphy: Some(true) });
        assert_eq!(c.phy_if, PhyIf { conf0: Some(0x0e), phyrstz: None });
        assert_eq!(c.panic, Some(Panic { low: 170, high: 340 }));
        assert!(config(&YdImx8mpHdmiConfig { flags: flags::TEST_FORCED | flags::TEST_BGND, ..X2 }).is_err());
        assert!(config(&YdImx8mpHdmiConfig { hsync: 0, ..X2 }).is_err());
    }

    #[test]
    fn cold_on_flip_off_and_health() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let mut d = data(&m);
        assert_eq!(on(&m, &X2, &mut d, &e, 0, 0), -EINVAL, "no buffer to scan");
        assert_eq!(on(&m, &X2, &mut d, &e, 0x9000_0000, 0), 0);
        assert_eq!((d.on, d.adopted, d.pitch, d.stage), (1, 0, 7680, chain::stage::VIDEO));
        assert_eq!(d.pll_pms, 7 | 225 << 8 | 1 << 16);
        assert_eq!(set_fb(&m, &mut d, &e, 0x9100_0000), 0);
        assert_eq!(chain::scanout(&m, &bases(&d)).unwrap().phys, 0x9100_0000);
        assert_ne!(chain::health(&m, &bases(&d)) & chain::health_bit::LCDIF_RUNNING, 0);
        off(&m, &mut d, &e);
        assert!(!chain::is_running(&m, &bases(&d)));
        assert_eq!(set_fb(&m, &mut d, &e, 0x9100_0000), -ENODEV);
    }

    #[test]
    fn stop_after_is_reported_with_its_stage() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let mut d = data(&m);
        assert_eq!(on(&m, &YdImx8mpHdmiConfig { stop_after: 4, ..X2 }, &mut d, &e, 0x9000_0000, 0), -ECANCELED);
        assert_eq!((d.stage, d.error, d.on), (4, -ECANCELED, 0));
    }

    #[test]
    fn without_force_cold_a_running_chain_is_adopted_and_given_back() {
        let m = HdmiModel::new();
        let e = TestEnv::default();
        let mut d = data(&m);
        // "Linux": the chain up, scanning 0x8000_0000 at RGB565
        assert_eq!(on(&m, &X2, &mut d, &e, 0x8000_0000, 0), 0);
        lcdif::set_layout(&m, d.lcdif, 3840, lcdif::BPP_RGB565).unwrap();
        lcdif::set_fb(&m, d.lcdif, &e, 0x8000_0000).unwrap();
        let mut d = data(&m);
        let adopt_cfg = YdImx8mpHdmiConfig { flags: X2.flags & !flags::FORCE_COLD, ..X2 };
        m.clear_log();
        assert_eq!(on(&m, &adopt_cfg, &mut d, &e, 0x9000_0000, 0), 0);
        assert_eq!((d.adopted, d.prev_fb, d.pitch), (1, 0x8000_0000, 7680));
        assert!(!m.writes().iter().any(|w| w.0 == GPC || w.0 == PHY || w.0 == HDMI), "adopt touches LCDIF3 only");
        let s = chain::scanout(&m, &bases(&d)).unwrap();
        assert_eq!((s.phys, s.bytes_pp), (0x9000_0000, 4));
        off(&m, &mut d, &e);
        let s = chain::scanout(&m, &bases(&d)).unwrap();
        assert_eq!((s.phys, s.bytes_pp, s.pitch), (0x8000_0000, 2, 3840));
        // and a mode that does not match the devicetree is refused
        let mut d = data(&m);
        assert_eq!(on(&m, &YdImx8mpHdmiConfig { hactive: 1280, ..adopt_cfg }, &mut d, &e, 0x9000_0000, 0), -EINVAL);
    }
}
