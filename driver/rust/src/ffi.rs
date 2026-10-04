// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The C entry points and the structs they share with the shims.
//!
//! Every struct here has a twin in `include/yakogl_drivers.h`; the numbers
//! below are pinned on both sides (`const` asserts here, `_Static_assert`s
//! there), so a field moved on one side stops the build of either. All values
//! crossing are integers or pointers: this code runs where the FP registers
//! may not be touched.
//!
//! Conventions: `cfg` points into the device's `config` (devicetree values,
//! read only), `data` into its `data` (mutable state, one per instance, no
//! statics anywhere), `env` at the shim's [`YdEnv`]. Errors are negative
//! errno. The shim serialises calls on one instance.

#[cfg(any(feature = "am62p", test))]
pub use am62p::*;
pub use board::*;
#[cfg(any(feature = "imx8mp", test))]
pub use hdmi::*;
#[cfg(any(feature = "imx8mp", test))]
pub use imx8mp::*;
#[cfg(any(feature = "imx8mp", test))]
pub use imx8mp_dsi::*;
#[cfg(any(feature = "imx8mp", test))]
pub use imx_wdog::*;
pub use sn65dsi8x::*;
#[cfg(any(feature = "vivante", test))]
pub use vivante::*;

/// The record writer of the board layer (board::record; the shim is
/// driver/zephyr/lib/board/common/record.c): src/ffi/board.rs. Any SoC.
mod board;

/// The AM62P display path (`safeui,am62p-dss`, `safeui,cdns-dsi-j721e`;
/// soc::am62p::dss, ip::cdns_dsi): src/ffi/am62p.rs.
#[cfg(any(feature = "am62p", test))]
mod am62p;

/// The i.MX 8M Plus HDMI display (`yakogl,imx8mp-hdmi`,
/// soc::imx8mp::hdmi_chain): src/ffi/hdmi.rs.
#[cfg(any(feature = "imx8mp", test))]
mod hdmi;

/// The Verdin's LCDIF1 and Samsung DSIM (`yakogl,imx8mp-lcdif`,
/// `yakogl,samsung-dsim`; ip::lcdifv3, ip::samsung_dsim): src/ffi/imx8mp_dsi.rs.
#[cfg(any(feature = "imx8mp", test))]
mod imx8mp_dsi;
/// The i.MX WDOG self-arm (ip::imx_wdog; the shim is
/// driver/zephyr/lib/board/imx8mp/gc_wdog_arm.c): src/ffi/imx_wdog.rs.
#[cfg(any(feature = "imx8mp", test))]
mod imx_wdog;
/// The Vivante GC7000 kernel side (gpu::vivante; the shim is
/// driver/drivers/gpu/gpu_vivante_gc_rust.c): src/ffi/vivante.rs.
#[cfg(any(feature = "vivante", test))]
mod vivante;

/// The SN65DSI8x bridge (`yakogl,sn65dsi8x`, ip::sn65dsi8x): I2C only, on
/// any SoC, so always built.
mod sn65dsi8x {
    use crate::env::YdEnv;
    use crate::ip::sn65dsi8x::{Board, Bridge, Mode};
    use core::mem::{offset_of, size_of};

    pub const FLAG_LVDS_24BPP: u32 = 1 << 0;
    pub const FLAG_FORMAT1: u32 = 1 << 1;
    pub const FLAG_TERM_100_OHM: u32 = 1 << 2;
    pub const FLAG_HS_NEG: u32 = 1 << 3;
    pub const FLAG_VS_NEG: u32 = 1 << 4;
    pub const FLAG_DE_NEG: u32 = 1 << 5;
    pub const FLAG_HAS_EN: u32 = 1 << 6;
    pub const FLAG_HAS_RESET: u32 = 1 << 7;

    /// `struct yd_sn65dsi8x_config`: from the devicetree.
    #[repr(C)]
    pub struct YdSn65dsi8xConfig {
        /// `YD_SN65DSI8X_*` flags.
        pub flags: u32,
        /// DSI data lanes (`data-lanes`).
        pub lanes: u32,
    }

    /// `struct yd_sn65dsi8x_mode`: the video mode, from the devicetree or
    /// from the display controller that calls the bridge.
    #[repr(C)]
    pub struct YdSn65dsi8xMode {
        pub pixel_clock_hz: u32,
        pub hactive: u16,
        pub vactive: u16,
        pub hsync: u16,
        pub vsync: u16,
        pub hbp: u16,
        pub vbp: u16,
        pub hfp: u16,
        pub vfp: u16,
    }

    /// `struct yd_sn65dsi8x_data`: per instance, what the last start found.
    #[repr(C)]
    pub struct YdSn65dsi8xData {
        /// CSR 0x00..0x08.
        pub id: [u8; 9],
        /// LVDS_CLK_RANGE (0x0A bits 3:1).
        pub lvds_clk_range: u8,
        /// CHA_DSI_CLK_RANGE (0x12).
        pub dsi_clk_range: u8,
        /// DSI_CLK_DIVIDER as written (0x0B bits 7:3): the divide minus one.
        pub dsi_clk_div: u8,
        /// 0x18 as owned by the driver (polarities, 24 bpp, format 1).
        pub lvds_fmt: u8,
        /// IRQ_STAT after the last link check.
        pub irq_stat: u8,
        /// 1 while started (start succeeded, no stop since).
        pub started: u8,
        /// ID reads the last start needed.
        pub id_tries: u8,
        pub dsi_clk_khz: u32,
        pub lvds_clk_khz: u32,
    }

    const _: () = assert!(size_of::<YdSn65dsi8xConfig>() == 8);
    const _: () = assert!(offset_of!(YdSn65dsi8xConfig, flags) == 0);
    const _: () = assert!(offset_of!(YdSn65dsi8xConfig, lanes) == 4);
    const _: () = assert!(size_of::<YdSn65dsi8xMode>() == 20);
    const _: () = assert!(offset_of!(YdSn65dsi8xMode, hactive) == 4);
    const _: () = assert!(offset_of!(YdSn65dsi8xMode, vfp) == 18);
    const _: () = assert!(size_of::<YdSn65dsi8xData>() == 24);
    const _: () = assert!(offset_of!(YdSn65dsi8xData, lvds_clk_range) == 9);
    const _: () = assert!(offset_of!(YdSn65dsi8xData, irq_stat) == 13);
    const _: () = assert!(offset_of!(YdSn65dsi8xData, id_tries) == 15);
    const _: () = assert!(offset_of!(YdSn65dsi8xData, dsi_clk_khz) == 16);
    const _: () = assert!(offset_of!(YdSn65dsi8xData, lvds_clk_khz) == 20);

    fn board(cfg: &YdSn65dsi8xConfig) -> Board {
        let f = |bit| cfg.flags & bit != 0;
        Board {
            lanes: cfg.lanes.min(255) as u8,
            lvds_24bpp: f(FLAG_LVDS_24BPP),
            format1: f(FLAG_FORMAT1),
            term_100_ohm: f(FLAG_TERM_100_OHM),
            hs_neg: f(FLAG_HS_NEG),
            vs_neg: f(FLAG_VS_NEG),
            de_neg: f(FLAG_DE_NEG),
            has_en: f(FLAG_HAS_EN),
            has_reset: f(FLAG_HAS_RESET),
        }
    }

    fn mode(m: &YdSn65dsi8xMode) -> Mode {
        Mode {
            pixel_clock_hz: m.pixel_clock_hz,
            hactive: m.hactive,
            vactive: m.vactive,
            hsync: m.hsync,
            vsync: m.vsync,
            hbp: m.hbp,
            vbp: m.vbp,
            hfp: m.hfp,
            vfp: m.vfp,
        }
    }

    pub(crate) fn start(
        env: &impl crate::env::Env,
        cfg: &YdSn65dsi8xConfig,
        data: &mut YdSn65dsi8xData,
        m: &YdSn65dsi8xMode,
        lane_rate_hz: u32,
    ) -> i32 {
        data.started = 0;
        match Bridge::new(env).start(&board(cfg), &mode(m), lane_rate_hz) {
            Ok(s) => {
                data.id = s.id;
                data.lvds_clk_range = s.plan.lvds_clk_range;
                data.dsi_clk_range = s.plan.dsi_clk_range;
                data.dsi_clk_div = s.plan.divider - 1;
                data.lvds_fmt = s.plan.lvds_fmt;
                data.id_tries = s.id_tries.min(255) as u8;
                data.dsi_clk_khz = s.plan.dsi_clk_hz / 1000;
                data.lvds_clk_khz = s.plan.lvds_clk_hz / 1000;
                data.started = 1;
                0
            }
            Err(e) => e,
        }
    }

    /// Init seq 3-7 of the data sheet (see ip::sn65dsi8x): EN cycle, ID,
    /// CSRs, PLL, soft reset. `lane_rate_hz` is the DSI bit rate per lane, 0
    /// to derive it from the pixel clock.
    ///
    /// # Safety
    /// `cfg`, `data`, `env`, `mode` valid for the call; calls on one instance
    /// serialised by the shim.
    #[no_mangle]
    pub unsafe extern "C" fn yd_sn65dsi8x_start(
        cfg: *const YdSn65dsi8xConfig,
        data: *mut YdSn65dsi8xData,
        env: *const YdEnv,
        mode: *const YdSn65dsi8xMode,
        lane_rate_hz: u32,
    ) -> i32 {
        // SAFETY: the caller's contract.
        unsafe { start(&*env, &*cfg, &mut *data, &*mode, lane_rate_hz) }
    }

    /// After the video started: clear IRQ_STAT, read it 1 ms later. Returns
    /// what is still set (0..=255), or a negative errno.
    ///
    /// # Safety
    /// As [`yd_sn65dsi8x_start`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_sn65dsi8x_check_link(data: *mut YdSn65dsi8xData, env: *const YdEnv) -> i32 {
        // SAFETY: the caller's contract.
        let (data, env) = unsafe { (&mut *data, &*env) };
        match Bridge::new(env).check_link() {
            Ok(st) => {
                data.irq_stat = st;
                i32::from(st)
            }
            Err(e) => e,
        }
    }

    /// EN low for the reset time.
    ///
    /// # Safety
    /// As [`yd_sn65dsi8x_start`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_sn65dsi8x_stop(
        cfg: *const YdSn65dsi8xConfig,
        data: *mut YdSn65dsi8xData,
        env: *const YdEnv,
    ) -> i32 {
        // SAFETY: the caller's contract.
        let (cfg, data, env) = unsafe { (&*cfg, &mut *data, &*env) };
        data.started = 0;
        match Bridge::new(env).stop(&board(cfg)) {
            Ok(()) => 0,
            Err(e) => e,
        }
    }

    /// The part's health bits (ip::sn65dsi8x::health), from reads only.
    ///
    /// # Safety
    /// `env` valid for the call.
    #[no_mangle]
    pub unsafe extern "C" fn yd_sn65dsi8x_health(env: *const YdEnv) -> u32 {
        // SAFETY: the caller's contract.
        Bridge::new(unsafe { &*env }).health()
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::ip::sn65dsi8x::csr;
        use crate::model::BridgeModel;

        #[test]
        fn start_through_the_c_structs_fills_data() {
            let m = BridgeModel::new();
            let cfg = YdSn65dsi8xConfig { flags: FLAG_LVDS_24BPP | FLAG_HAS_EN, lanes: 4 };
            let mode = YdSn65dsi8xMode {
                pixel_clock_hz: 69_300_000,
                hactive: 1280,
                vactive: 800,
                hsync: 24,
                vsync: 8,
                hbp: 64,
                vbp: 24,
                hfp: 36,
                vfp: 8,
            };
            let mut d = YdSn65dsi8xData {
                id: [0; 9],
                lvds_clk_range: 0,
                dsi_clk_range: 0,
                dsi_clk_div: 0,
                lvds_fmt: 0,
                irq_stat: 0xff,
                started: 0,
                id_tries: 0,
                dsi_clk_khz: 0,
                lvds_clk_khz: 0,
            };
            assert_eq!(start(&m, &cfg, &mut d, &mode, 426_600_000), 0);
            assert_eq!(d.id, csr::ID_BYTES);
            assert_eq!((d.lvds_clk_range, d.dsi_clk_range, d.dsi_clk_div, d.lvds_fmt), (2, 42, 2, 0x08));
            assert_eq!((d.dsi_clk_khz, d.lvds_clk_khz, d.started, d.id_tries), (213_300, 71_100, 1, 1));
            assert_eq!(m.reg(csr::CLK_DIV), 2 << 3);
            // a bad mode leaves `started` clear
            let bad = YdSn65dsi8xMode { pixel_clock_hz: 0, ..mode };
            assert!(start(&m, &cfg, &mut d, &bad, 0) < 0);
            assert_eq!(d.started, 0);
        }
    }
}

#[cfg(any(feature = "imx8mp", test))]
mod imx8mp {
    use crate::env::YdEnv;
    use crate::errno::EINVAL;
    use crate::ip::imx_pwm::{self, Applied, Pwm};
    use crate::mmio::{Hw, Mmio};
    use crate::soc::imx8mp::{ccm, media, regs};
    use crate::ylog;
    use core::mem::{offset_of, size_of};

    /// `struct yd_imx_pwm_config`: from the devicetree.
    #[repr(C)]
    pub struct YdImxPwmConfig {
        /// CCM `ROOT[]` index of the PWM's clock root (`ccm-root`).
        pub ccm_root: u32,
        /// CCM `CCGR[]` index of the PWM's clock gate (`ccm-gate`).
        pub ccm_gate: u32,
    }

    /// `struct yd_imx_pwm_data`: per instance, mutable.
    #[repr(C)]
    pub struct YdImxPwmData {
        /// The PWM block, as the shim's `device_map()` returned it.
        pub pwm_base: usize,
        /// The CCM, likewise.
        pub ccm_base: usize,
        /// What `get_cycles_per_sec` reports: the clock root's rate once the
        /// driver owns it.
        pub rate_hz: u32,
        /// The last request (for a debugger; nothing reads them back).
        pub period: u32,
        pub pulse: u32,
        pub flags: u32,
    }

    const _: () = assert!(size_of::<YdImxPwmConfig>() == 8);
    const _: () = assert!(offset_of!(YdImxPwmConfig, ccm_root) == 0);
    const _: () = assert!(offset_of!(YdImxPwmConfig, ccm_gate) == 4);
    const _: () = assert!(size_of::<YdImxPwmData>() == 32);
    const _: () = assert!(offset_of!(YdImxPwmData, pwm_base) == 0);
    const _: () = assert!(offset_of!(YdImxPwmData, ccm_base) == 8);
    const _: () = assert!(offset_of!(YdImxPwmData, rate_hz) == 16);
    const _: () = assert!(offset_of!(YdImxPwmData, period) == 20);
    const _: () = assert!(offset_of!(YdImxPwmData, pulse) == 24);
    const _: () = assert!(offset_of!(YdImxPwmData, flags) == 28);

    fn check(cfg: &YdImxPwmConfig) -> Result<(), i32> {
        if cfg.ccm_root >= regs::ccm::ROOT_COUNT || cfg.ccm_gate >= regs::ccm::CCGR_COUNT {
            return Err(-EINVAL);
        }
        Ok(())
    }

    /// Device init: validate the configuration and report what the previous
    /// owner left. Writes nothing: a backlight Linux left on stays on until
    /// someone asks for something else.
    pub(crate) fn init(m: &impl Mmio, cfg: &YdImxPwmConfig, data: &mut YdImxPwmData, env: &YdEnv) -> i32 {
        if let Err(e) = check(cfg) {
            ylog!(env, Err, "imx_pwm: ccm-root {} / ccm-gate {} out of range", cfg.ccm_root, cfg.ccm_gate);
            return e;
        }
        data.rate_hz = ccm::OSC_24M_HZ;
        let pwm = Pwm::new(m, data.pwm_base);
        let root = m.read32(data.ccm_base + regs::ccm::target_root(cfg.ccm_root) as usize);
        ylog!(
            env,
            Inf,
            "imx_pwm: found {} (PWMCR {:#010x}), clock root {} {:#010x}; reports {} Hz",
            if pwm.is_enabled() { "running" } else { "stopped" },
            m.read32(data.pwm_base + regs::pwm::PWMCR as usize),
            cfg.ccm_root,
            root,
            data.rate_hz
        );
        0
    }

    #[allow(clippy::too_many_arguments)] // the PWM API's arguments, as they come
    pub(crate) fn set_cycles(
        m: &impl Mmio,
        cfg: &YdImxPwmConfig,
        data: &mut YdImxPwmData,
        env: &YdEnv,
        channel: u32,
        period: u32,
        pulse: u32,
        flags: u32,
    ) -> i32 {
        if channel != 0 {
            return -EINVAL;
        }
        if let Err(e) = check(cfg) {
            return e;
        }
        let plan = match imx_pwm::plan(period, pulse, flags) {
            Ok(p) => p,
            Err(e) => {
                ylog!(env, Err, "imx_pwm: cannot do period {} pulse {} flags {:#x}", period, pulse, flags);
                return e;
            }
        };
        let mut restart = false;
        if plan.high != 0 {
            match ccm::root_on_osc24m(m, data.ccm_base, cfg.ccm_root, cfg.ccm_gate) {
                Ok((hz, ch)) => {
                    data.rate_hz = hz;
                    if ch.rewrote_root || ch.opened_gate {
                        ylog!(
                            env,
                            Inf,
                            "imx_pwm: clock root {} was {:#010x}; now 24 MHz, gate open",
                            cfg.ccm_root,
                            ch.was
                        );
                    }
                    restart = ch.rewrote_root;
                }
                Err(e) => return e,
            }
        }
        match Pwm::new(m, data.pwm_base).apply(env, plan, restart) {
            Ok(a) => {
                data.period = period;
                data.pulse = pulse;
                data.flags = flags;
                if a == Applied::Started {
                    ylog!(
                        env,
                        Inf,
                        "imx_pwm: started, {} cycles / {} = {} counts, high {}",
                        period,
                        plan.prescale,
                        plan.counts,
                        plan.high
                    );
                }
                0
            }
            Err(e) => e,
        }
    }

    // ---- the C entry points --------------------------------------------------

    /// # Safety
    /// `cfg`, `data`, `env` valid for the call; `data`'s bases mapped.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx_pwm_init(
        cfg: *const YdImxPwmConfig,
        data: *mut YdImxPwmData,
        env: *const YdEnv,
    ) -> i32 {
        // SAFETY: the caller's contract; the bases are the shim's mappings.
        unsafe { init(&Hw::new(), &*cfg, &mut *data, &*env) }
    }

    /// Zephyr `pwm_driver_api.set_cycles`.
    ///
    /// # Safety
    /// As [`yd_imx_pwm_init`]; calls on one instance are serialised.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx_pwm_set_cycles(
        cfg: *const YdImxPwmConfig,
        data: *mut YdImxPwmData,
        env: *const YdEnv,
        channel: u32,
        period: u32,
        pulse: u32,
        flags: u32,
    ) -> i32 {
        // SAFETY: as above.
        unsafe { set_cycles(&Hw::new(), &*cfg, &mut *data, &*env, channel, period, pulse, flags) }
    }

    /// Zephyr `pwm_driver_api.get_cycles_per_sec`.
    ///
    /// # Safety
    /// `data` and `cycles` valid for the call.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx_pwm_get_cycles_per_sec(
        _cfg: *const YdImxPwmConfig,
        data: *const YdImxPwmData,
        channel: u32,
        cycles: *mut u64,
    ) -> i32 {
        if channel != 0 {
            return -EINVAL;
        }
        // SAFETY: the caller's contract.
        unsafe { *cycles = u64::from((*data).rate_hz) };
        0
    }

    /// The output is running (enabled with a clock): read from the hardware,
    /// for health reports.
    ///
    /// # Safety
    /// `data` valid for the call, its PWM base mapped.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx_pwm_is_enabled(data: *const YdImxPwmData) -> bool {
        // SAFETY: the caller's contract.
        unsafe { Pwm::new(&Hw::new(), (*data).pwm_base).is_enabled() }
    }

    // ---- the display power domains (soc::imx8mp::media) ------------------------

    /// `struct yd_imx8mp_media_data`: one per `yakogl,imx8mp-media-blk-ctrl`
    /// instance; the sub-domain devices use their parent's.
    #[repr(C)]
    pub struct YdImx8mpMediaData {
        /// MEDIA_BLK_CTRL, GPC, CCM, CCM_ANALOG, as `device_map()` returned them.
        pub blk_base: usize,
        pub gpc_base: usize,
        pub ccm_base: usize,
        pub anatop_base: usize,
        /// [`media::Track`]: sub-domains on, the last BLK_CTRL sample, the last
        /// operation.
        pub users: u32,
        pub sampled: u32,
        pub last: u32,
        pub reserved: u32,
    }

    const _: () = assert!(size_of::<YdImx8mpMediaData>() == 48);
    const _: () = assert!(offset_of!(YdImx8mpMediaData, anatop_base) == 24);
    const _: () = assert!(offset_of!(YdImx8mpMediaData, users) == 32);
    const _: () = assert!(offset_of!(YdImx8mpMediaData, last) == 40);

    fn split(d: &YdImx8mpMediaData) -> (media::Bases, media::Track) {
        (
            media::Bases { blk: d.blk_base, gpc: d.gpc_base, ccm: d.ccm_base, anatop: d.anatop_base },
            media::Track { users: d.users, sampled: d.sampled, last: d.last },
        )
    }

    fn keep(d: &mut YdImx8mpMediaData, t: media::Track) {
        d.users = t.users;
        d.sampled = t.sampled;
        d.last = t.last;
    }

    fn rc(r: Result<(), i32>) -> i32 {
        r.err().unwrap_or(0)
    }

    pub(crate) fn pd_op(m: &impl Mmio, d: &mut YdImx8mpMediaData, env: &YdEnv, pd: u32, op: u32) -> i32 {
        let Some(pd) = media::Pd::from_u32(pd) else {
            return -EINVAL;
        };
        let (b, mut t) = split(d);
        let r = match op {
            0 => media::pd_on(m, &b, env, &mut t, pd),
            1 => media::pd_off(m, &b, env, &mut t, pd),
            _ => media::pd_reset(m, &b, env, &mut t, pd),
        };
        keep(d, t);
        rc(r)
    }

    /// Turn sub-domain `pd` (0 LCDIF1, 1 MIPI_DSI1) on; MEDIAMIX first.
    ///
    /// # Safety
    /// `data` and `env` valid for the call, the bases mapped; calls on one
    /// instance serialised.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_media_pd_on(data: *mut YdImx8mpMediaData, env: *const YdEnv, pd: u32) -> i32 {
        // SAFETY: the caller's contract.
        unsafe { pd_op(&Hw::new(), &mut *data, &*env, pd, 0) }
    }

    /// Turn sub-domain `pd` off; the last one off powers MEDIAMIX down.
    ///
    /// # Safety
    /// As [`yd_imx8mp_media_pd_on`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_media_pd_off(data: *mut YdImx8mpMediaData, env: *const YdEnv, pd: u32) -> i32 {
        // SAFETY: the caller's contract.
        unsafe { pd_op(&Hw::new(), &mut *data, &*env, pd, 1) }
    }

    /// Pulse sub-domain `pd`'s resets with its clocks on.
    ///
    /// # Safety
    /// As [`yd_imx8mp_media_pd_on`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_media_pd_reset(data: *mut YdImx8mpMediaData, env: *const YdEnv, pd: u32) -> i32 {
        // SAFETY: the caller's contract.
        unsafe { pd_op(&Hw::new(), &mut *data, &*env, pd, 2) }
    }

    pub(crate) fn set_pixel(
        m: &impl Mmio,
        d: &mut YdImx8mpMediaData,
        env: &YdEnv,
        want_hz: u32,
        video_pll1_hz: u32,
        out_hz: &mut u32,
    ) -> i32 {
        let (b, mut t) = split(d);
        let r = media::set_pixel_clock(m, &b, env, &mut t, want_hz, video_pll1_hz);
        keep(d, t);
        match r {
            Ok(hz) => {
                *out_hz = hz;
                0
            }
            Err(e) => e,
        }
    }

    /// Program VIDEO_PLL1 to `video_pll1_hz` (0: leave it) and the pixel
    /// clock root to the rate nearest `want_hz`, written to `*out_hz`.
    ///
    /// # Safety
    /// As [`yd_imx8mp_media_pd_on`]; `out_hz` valid for the call.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_media_set_pixel_clock(
        data: *mut YdImx8mpMediaData,
        env: *const YdEnv,
        want_hz: u32,
        video_pll1_hz: u32,
        out_hz: *mut u32,
    ) -> i32 {
        // SAFETY: the caller's contract.
        unsafe { set_pixel(&Hw::new(), &mut *data, &*env, want_hz, video_pll1_hz, &mut *out_hz) }
    }

    /// What the pixel clock root produces now; 0 when unknown.
    ///
    /// # Safety
    /// `data` valid, its CCM and ANATOP bases mapped.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_media_pixel_clock(data: *const YdImx8mpMediaData) -> u32 {
        // SAFETY: the caller's contract.
        unsafe { media::pixel_clock(&Hw::new(), &split(&*data).0) }
    }

    /// MEDIA_MIPI_PHY1_REF from the 24 MHz oscillator, at most `hz`.
    /// Returns the rate, 0 on error.
    ///
    /// # Safety
    /// As [`yd_imx8mp_media_pd_on`].
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_media_set_phy_ref_clock(data: *mut YdImx8mpMediaData, hz: u32) -> u32 {
        // SAFETY: the caller's contract.
        let d = unsafe { &mut *data };
        let (b, mut t) = split(d);
        // SAFETY: as above.
        let r = media::set_phy_ref_clock(unsafe { &Hw::new() }, &b, &mut t, hz);
        keep(d, t);
        r.unwrap_or(0)
    }

    /// The status word (`YD_MEDIA_ST_*`).
    ///
    /// # Safety
    /// `data` valid, its GPC, CCM and ANATOP bases mapped.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_media_status(data: *const YdImx8mpMediaData) -> u32 {
        // SAFETY: the caller's contract.
        unsafe {
            let (b, t) = split(&*data);
            media::status(&Hw::new(), &b, &t)
        }
    }

    // The YD_LCDIF_UNREACH_* values of yakogl_drivers.h.
    const _: () = assert!(
        media::unreach::MEDIAMIX == 1
            && media::unreach::APB_ROOT == 2
            && media::unreach::GATE == 4
            && media::unreach::LCDIF_APB == 8
    );

    /// Whether an LCDIF1 register access would be answered now: 0, or the
    /// `media::unreach` bits saying why not (`YD_LCDIF_UNREACH_*`). Reads the
    /// GPC and the CCM, and MEDIA_BLK_CTRL only when those say it answers;
    /// writes nothing. Callable at PRE_KERNEL_1 on the caller's own mappings.
    ///
    /// # Safety
    /// `gpc_base`, `ccm_base` and `blk_base` are mapped GPC, CCM and
    /// MEDIA_BLK_CTRL blocks.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_lcdif_unreachable(gpc_base: usize, ccm_base: usize, blk_base: usize) -> u32 {
        // SAFETY: the caller's contract.
        media::lcdif_unreachable(unsafe { &Hw::new() }, gpc_base, ccm_base, blk_base)
    }

    /// Enable `n_roots` clock roots (ENABLE only) and open `n_gates` gates for
    /// domain 0, before any driver runs. Returns the registers written or a
    /// negative errno.
    ///
    /// # Safety
    /// `ccm_base` mapped; `roots` / `gates` valid for their counts.
    #[no_mangle]
    pub unsafe extern "C" fn yd_imx8mp_ccm_early_clocks(
        ccm_base: usize,
        roots: *const u32,
        n_roots: usize,
        gates: *const u32,
        n_gates: usize,
    ) -> i32 {
        // SAFETY: the caller's contract; a zero count may come with NULL.
        let (r, g) = unsafe {
            (
                if n_roots == 0 { &[][..] } else { core::slice::from_raw_parts(roots, n_roots) },
                if n_gates == 0 { &[][..] } else { core::slice::from_raw_parts(gates, n_gates) },
            )
        };
        // SAFETY: as above.
        match media::early_clocks(unsafe { &Hw::new() }, ccm_base, r, g) {
            Ok(n) => n as i32,
            Err(e) => e,
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::model::{Model, CCM, PWM};
        use core::ffi::c_void;

        unsafe extern "C" fn wait(_: *mut c_void, _: u32) {}
        unsafe extern "C" fn log(ctx: *mut c_void, _: u32, b: *const u8, n: usize) {
            // SAFETY: ctx is the test's Vec<String>
            let v = unsafe { &mut *(ctx as *mut Vec<String>) };
            v.push(String::from_utf8_lossy(unsafe { core::slice::from_raw_parts(b, n) }).into_owned());
        }
        unsafe extern "C" fn i2c_w(_: *mut c_void, _: u32, _: *const u8, _: usize) -> i32 {
            -crate::errno::ENOTSUP
        }
        unsafe extern "C" fn i2c_wr(_: *mut c_void, _: u32, _: *const u8, _: usize, _: *mut u8, _: usize) -> i32 {
            -crate::errno::ENOTSUP
        }
        unsafe extern "C" fn gpio(_: *mut c_void, _: u32, _: u32) -> i32 {
            -crate::errno::ENOTSUP
        }

        fn env(lines: &mut Vec<String>) -> YdEnv {
            YdEnv {
                ctx: lines as *mut Vec<String> as *mut c_void,
                busy_wait_us: wait,
                sleep_ms: wait,
                log,
                i2c_write: i2c_w,
                i2c_write_read: i2c_wr,
                gpio_set: gpio,
            }
        }

        fn data(m: &Model) -> YdImxPwmData {
            YdImxPwmData { pwm_base: m.base(PWM), ccm_base: m.base(CCM), rate_hz: 0, period: 0, pulse: 0, flags: 0 }
        }

        const CFG: YdImxPwmConfig = YdImxPwmConfig { ccm_root: 105, ccm_gate: 42 };

        #[test]
        fn media_through_the_c_struct_keeps_its_state_in_data() {
            use crate::model::{ANATOP, BLK, GPC};
            let m = Model::new();
            m.poke(ANATOP, 0x94, 0x8aaa_aa00);
            m.poke(ANATOP, 0x98, 0x0019_0032);
            m.poke(ANATOP, 0x104, 0x8aaa_aa00);
            m.poke(ANATOP, 0x108, 0x000f_a031);
            m.poke(GPC, 0x190, 0xffff);
            let mut lines = Vec::new();
            let e = env(&mut lines);
            let mut d = YdImx8mpMediaData {
                blk_base: m.base(BLK),
                gpc_base: m.base(GPC),
                ccm_base: m.base(CCM),
                anatop_base: m.base(ANATOP),
                users: 0,
                sampled: 0,
                last: 0,
                reserved: 0,
            };
            assert_eq!(pd_op(&m, &mut d, &e, 0, 0), 0);
            assert_eq!(d.users, 1);
            assert_eq!(d.last, media::step::ON);
            assert_eq!(pd_op(&m, &mut d, &e, 2, 0), -EINVAL);
            let mut hz = 0;
            assert_eq!(set_pixel(&m, &mut d, &e, 69_300_000, 1_039_500_000, &mut hz), 0);
            assert_eq!(hz, 69_300_000);
            assert_eq!(pd_op(&m, &mut d, &e, 0, 1), 0);
            assert_eq!((d.users, m.powered()), (0, 0));
        }

        #[test]
        fn init_writes_nothing_and_reports_24_mhz() {
            let m = Model::new();
            let mut lines = Vec::new();
            let e = env(&mut lines);
            let mut d = data(&m);
            assert_eq!(init(&m, &CFG, &mut d, &e), 0);
            assert!(m.writes().is_empty());
            assert_eq!(d.rate_hz, 24_000_000);
            assert!(lines[0].contains("stopped"), "{lines:?}");
        }

        #[test]
        fn init_refuses_indices_outside_the_ccm() {
            let m = Model::new();
            let mut lines = Vec::new();
            let e = env(&mut lines);
            let mut d = data(&m);
            let bad = YdImxPwmConfig { ccm_root: 142, ccm_gate: 42 };
            assert_eq!(init(&m, &bad, &mut d, &e), -EINVAL);
            assert!(m.accesses().is_empty());
        }

        #[test]
        fn the_backlight_from_cold_clock_first_then_the_pwm() {
            let m = Model::new();
            let mut lines = Vec::new();
            let e = env(&mut lines);
            let mut d = data(&m);
            assert_eq!(set_cycles(&m, &CFG, &mut d, &e, 0, 160_012, 160_012, 0), 0);
            let w = m.writes();
            assert_eq!(&w[..2], &[(CCM, 0xb480, 0x1000_0000), (CCM, 0x42a4, 0x3)]);
            assert_eq!(&w[2..], &[(PWM, 0, 8), (PWM, 0xc, 53_337), (PWM, 0x10, 53_335), (PWM, 0, 0x01c2_0021)]);
            assert_eq!((d.period, d.pulse), (160_012, 160_012));
        }

        #[test]
        fn a_root_moved_under_a_running_pwm_forces_a_restart() {
            let m = Model::new();
            let mut lines = Vec::new();
            let e = env(&mut lines);
            let mut d = data(&m);
            m.poke(CCM, 0xb480, 0x1200_0000); // SYS_PLL1/5
            m.poke(CCM, 0x42a0, 0x3);
            m.poke(PWM, 0, 0x01c2_0021); // looks like ours
            assert_eq!(set_cycles(&m, &CFG, &mut d, &e, 0, 160_012, 160_012, 0), 0);
            assert!(m.writes().contains(&(PWM, 0, 8)));
        }

        #[test]
        fn zero_percent_touches_no_clock() {
            let m = Model::new();
            let mut lines = Vec::new();
            let e = env(&mut lines);
            let mut d = data(&m);
            m.poke(PWM, 0, 0x01c2_0021);
            assert_eq!(set_cycles(&m, &CFG, &mut d, &e, 0, 160_012, 0, 0), 0);
            assert_eq!(m.writes(), vec![(PWM, 0, 0x01c2_0020)]);
        }

        #[test]
        fn channel_one_and_bad_requests_are_refused_without_access() {
            let m = Model::new();
            let mut lines = Vec::new();
            let e = env(&mut lines);
            let mut d = data(&m);
            assert_eq!(set_cycles(&m, &CFG, &mut d, &e, 1, 1000, 500, 0), -EINVAL);
            assert_eq!(set_cycles(&m, &CFG, &mut d, &e, 0, 1000, 1001, 0), -EINVAL);
            assert!(m.accesses().is_empty());
            let mut hz = 0u64;
            d.rate_hz = 24_000_000;
            // SAFETY: valid pointers
            assert_eq!(unsafe { yd_imx_pwm_get_cycles_per_sec(&CFG, &d, 0, &mut hz) }, 0);
            assert_eq!(hz, 24_000_000);
            assert_eq!(unsafe { yd_imx_pwm_get_cycles_per_sec(&CFG, &d, 1, &mut hz) }, -EINVAL);
        }
    }
}
