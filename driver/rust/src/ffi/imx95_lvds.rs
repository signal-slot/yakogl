// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95's picture on X12 for the Zephyr board layer
//! (driver/zephyr/lib/board/verdin_imx95/verdin_imx95_lvds.c):
//! [`yd_imx95_lvds_up`], [`yd_imx95_lvds_flip`] and [`yd_imx95_lvds_down`]
//! over `board::verdin_imx95_lvds`, the steps of driver/docs/imx95-display.md
//! section 4(b').
//!
//! - up: `display_up` with the buffer the board shows first (XRGB8888,
//!   1280x800 at `fb`, rows `stride` bytes apart), every block where the
//!   image mapped it ([`YdImx95LvdsBases`]), the System Manager over the
//!   board's SCMI channel (the GPU's, `ffi::imx95_display::CScmi`). The env
//!   serves only the waits and the log: the X12 path has no I2C and drives
//!   its one GPIO (the backlight enable) as a register.
//! - flip: `soc::imx95::display::flip`, as the X4 path's.
//! - down: `display_down`: the backlight, the LDB, stream 0 and the power.
//!
//! The state the C side keeps is [`YdImx95Lvds`]; `Stream0` is rebuilt
//! from it for each call (the panel's mode, [`Route::Lvds`] with the pixel
//! link, the LDB PLL).

use super::imx95_display::{CScmi, STATE_DOWN, STATE_FAILED, STATE_NEW, STATE_UP};
use crate::board::verdin_imx95_lvds::{self as lv, display_down, display_up, Config, PANEL};
use crate::env::{Env, YdEnv};
use crate::errno::{EBUSY, EINVAL, EIO, ENODEV, ETIMEDOUT};
use crate::firmware::scmi::Transport;
use crate::mmio::{Hw, Mmio};
use crate::soc::imx95::display::{self, Prepared, Stream0};
use crate::soc::imx95::display_power::PixelSource;
use crate::soc::imx95::dpu::Framebuffer;
use crate::ylog;
use core::mem::{offset_of, size_of};

/// `struct yd_imx95_lvds_bases`: where the image mapped each block (Device
/// memory; `YD_IMX95_*_PHYS`, at least `YD_IMX95_*_SIZE` bytes each).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdImx95LvdsBases {
    /// The DPU, the display CSR and the pixel interleaver.
    pub dpu: usize,
    pub csr: usize,
    pub interleaver: usize,
    /// The LVDS CSR (`soc::imx95::ldb::LDB_BASE`).
    pub ldb: usize,
    /// TPM5, the backlight PWM, and GPIO5, its enable.
    pub tpm: usize,
    pub gpio: usize,
}

/// [`YdImx95Lvds::stop`]: the DPU side stopped (power, the LDB PLL, the
/// routing, FrameGen0's sync, a flip's shadow load, the stop).
pub const STOP_DISPLAY: u32 = 1;
/// The backlight stopped (TPM5's clock over SCMI, the PWM); the picture
/// runs, dark.
pub const STOP_BACKLIGHT: u32 = 2;

/// `struct yd_imx95_lvds`: all zero before the first up; kept by the C side
/// between the calls. A 64-bit value is two words, lo first. The states
/// are `yd_imx95_display`'s (`YD_IMX95_DISPLAY_NEW` .. `_DOWN`).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdImx95Lvds {
    pub fb: u64,
    pub stride: u32,
    pub state: u32,
    pub width: u32,
    pub height: u32,
    /// LDBPLL / 7 as the System Manager made it, and the DISP level.
    pub pix_hz: [u32; 2],
    pub level: u32,
    /// µs FrameGen0 took to sync.
    pub sync_us: u32,
    /// The LVDS CSR's four words read back after the start.
    pub ldb: [u32; 4],
    /// TPM5's clock, Hz.
    pub tpm_hz: u32,
    /// The backlight's level of 255.
    pub backlight: u32,
    pub flips: u32,
    pub flip_us: u32,
    /// 0, [`STOP_DISPLAY`] or [`STOP_BACKLIGHT`].
    pub stop: u32,
    pub reserved: u32,
}

const _: () = assert!(size_of::<YdImx95LvdsBases>() == 8 * 6);
const _: () = assert!(offset_of!(YdImx95LvdsBases, gpio) == 40);
const _: () = assert!(size_of::<YdImx95Lvds>() == 80);
const _: () = assert!(offset_of!(YdImx95Lvds, ldb) == 40);
const _: () = assert!(offset_of!(YdImx95Lvds, stop) == 72);

/// The X12 configuration over the mapped blocks, showing `fb`.
pub fn config(b: &YdImx95LvdsBases, fb: Framebuffer) -> Config {
    let mut c = Config::verdin(fb);
    c.display = display::Bases { dpu: b.dpu, csr: b.csr, interleaver: b.interleaver };
    c.ldb = b.ldb;
    c.tpm = b.tpm;
    c.gpio = b.gpio;
    c
}

fn complete(b: &YdImx95LvdsBases) -> bool {
    [b.dpu, b.csr, b.interleaver, b.ldb, b.tpm, b.gpio].iter().all(|&a| a != 0)
}

fn stream(d: &YdImx95Lvds, cfg: &Config) -> Stream0 {
    let fb = Framebuffer { addr: d.fb, stride: d.stride };
    let pix_hz = u64::from(d.pix_hz[0]) | u64::from(d.pix_hz[1]) << 32;
    Stream0 {
        prepared: Prepared {
            mode: PANEL,
            route: cfg.stream.route,
            fb,
            source: PixelSource::LdbPll,
            level: d.level,
            pix_hz,
        },
        fb,
        sync_us: d.sync_us,
    }
}

fn side(e: &lv::Error) -> u32 {
    match e {
        lv::Error::Display(_) => STOP_DISPLAY,
        _ => STOP_BACKLIGHT,
    }
}

/// [`yd_imx95_lvds_up`] over any transport, register access and env.
pub fn up_on<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    b: &YdImx95LvdsBases,
    fb: u64,
    stride: u32,
    d: &mut YdImx95Lvds,
) -> i32 {
    if d.state != STATE_NEW {
        return if d.state == STATE_UP { -EBUSY } else { -ENODEV };
    }
    if !complete(b) || fb == 0 || stride == 0 {
        return -EINVAL;
    }
    let cfg = config(b, Framebuffer { addr: fb, stride });
    match display_up(t, m, env, &cfg) {
        Ok(up) => {
            let p = &up.stream.prepared;
            *d = YdImx95Lvds {
                fb,
                stride,
                state: STATE_UP,
                width: p.mode.hactive,
                height: p.mode.vactive,
                pix_hz: [p.pix_hz as u32, (p.pix_hz >> 32) as u32],
                level: p.level,
                sync_us: up.stream.sync_us,
                ldb: up.ldb,
                tpm_hz: up.tpm_hz as u32,
                backlight: cfg.backlight.level,
                ..YdImx95Lvds::default()
            };
            0
        }
        Err(e) => {
            *d = YdImx95Lvds { fb, stride, state: STATE_FAILED, stop: side(&e), ..YdImx95Lvds::default() };
            ylog!(env, Err, "verdin-imx95 X12: the up stopped: {:?}", e);
            -EIO
        }
    }
}

/// [`yd_imx95_lvds_flip`] over any register access and env.
pub fn flip_on(m: &impl Mmio, env: &impl Env, b: &YdImx95LvdsBases, fb: u64, d: &mut YdImx95Lvds) -> i32 {
    if d.state != STATE_UP {
        return -ENODEV;
    }
    if fb == d.fb {
        return 0;
    }
    let cfg = config(b, Framebuffer { addr: d.fb, stride: d.stride });
    let mut s = stream(d, &cfg);
    let mut delay = |us: u32| env.busy_wait_us(us);
    match display::flip(m, &cfg.display, &mut s, fb, &mut delay) {
        Ok(waited) => {
            d.fb = fb;
            d.flips = d.flips.wrapping_add(1);
            d.flip_us = waited;
            0
        }
        Err(display::Error::Refused(_)) => -EINVAL,
        Err(e) => {
            ylog!(env, Err, "verdin-imx95 X12: flip to {:#x}: {:?}", fb, e);
            d.fb = fb;
            -ETIMEDOUT
        }
    }
}

/// [`yd_imx95_lvds_down`] over any transport, register access and env.
pub fn down_on<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    b: &YdImx95LvdsBases,
    d: &mut YdImx95Lvds,
) -> i32 {
    if d.state != STATE_UP {
        return -ENODEV;
    }
    let cfg = config(b, Framebuffer { addr: d.fb, stride: d.stride });
    let s = stream(d, &cfg);
    d.state = STATE_DOWN;
    match display_down(t, m, env, &cfg, &s) {
        Ok(()) => 0,
        Err(e) => {
            d.stop = side(&e);
            ylog!(env, Err, "verdin-imx95 X12: the down stopped: {:?}", e);
            -EIO
        }
    }
}

/// The 10.1" LVDS panel on (design note section 4(b')), showing the
/// XRGB8888 buffer at physical `fb`. 0, `-EBUSY`, `-EINVAL` (a missing
/// block, buffer or callback: nothing sent or written), `-EIO` (`d->stop`
/// says where; one log line), `-ENODEV` (after a stopped up or a down).
///
/// # Safety
/// `bases`, `env` and `d` valid; each base the mapping of its block;
/// `scmi` safe to call with valid buffers. One thread only.
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_lvds_up(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    bases: *const YdImx95LvdsBases,
    env: *const YdEnv,
    fb: u64,
    stride: u32,
    d: *mut YdImx95Lvds,
) -> i32 {
    let Some(scmi) = scmi else { return -EINVAL };
    // SAFETY: the caller's contract.
    let (Some(b), Some(env), Some(d)) = (unsafe { (bases.as_ref(), env.as_ref(), d.as_mut()) }) else {
        return -EINVAL;
    };
    // SAFETY: the bases are the blocks' mappings (the caller's contract).
    let m = unsafe { Hw::new() };
    up_on(&mut CScmi(scmi), &m, env, b, fb, stride, d)
}

/// Show the buffer at physical `fb` (the up's stride) from the next frame,
/// waited for (at most three frames): as `yd_imx95_display_flip`.
///
/// # Safety
/// As [`yd_imx95_lvds_up`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_lvds_flip(
    bases: *const YdImx95LvdsBases,
    env: *const YdEnv,
    fb: u64,
    d: *mut YdImx95Lvds,
) -> i32 {
    // SAFETY: the caller's contract.
    let (Some(b), Some(env), Some(d)) = (unsafe { (bases.as_ref(), env.as_ref(), d.as_mut()) }) else {
        return -EINVAL;
    };
    // SAFETY: as above.
    let m = unsafe { Hw::new() };
    flip_on(&m, env, b, fb, d)
}

/// The picture down: the backlight (enable low, PWM, TPM5's clock), the LDB,
/// then stream 0, the LDB PLL and DISPLAY. 0, `-EIO` (logged), `-ENODEV`.
///
/// # Safety
/// As [`yd_imx95_lvds_up`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_lvds_down(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    bases: *const YdImx95LvdsBases,
    env: *const YdEnv,
    d: *mut YdImx95Lvds,
) -> i32 {
    let Some(scmi) = scmi else { return -EINVAL };
    // SAFETY: the caller's contract.
    let (Some(b), Some(env), Some(d)) = (unsafe { (bases.as_ref(), env.as_ref(), d.as_mut()) }) else {
        return -EINVAL;
    };
    // SAFETY: as above.
    let m = unsafe { Hw::new() };
    down_on(&mut CScmi(scmi), &m, env, b, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::verdin_imx95_lvds_tests::{rig, FB};
    use crate::soc::imx95::{display as disp, dpu, ldb};

    const H: &str = include_str!("../../include/yakogl_drivers.h");

    fn def(name: &str) -> u64 {
        let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
        let v = l.split_whitespace().nth(2).expect(name);
        match v.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h.trim_end_matches('u'), 16).unwrap(),
            None => v.trim_end_matches('u').parse().unwrap_or_else(|_| panic!("{name}: {v}")),
        }
    }

    const PHYS: YdImx95LvdsBases = YdImx95LvdsBases {
        dpu: dpu::DPU_BASE,
        csr: disp::CSR_BASE,
        interleaver: disp::INTERLEAVER_BASE,
        ldb: ldb::LDB_BASE,
        tpm: lv::TPM5_BASE,
        gpio: lv::GPIO5_BASE,
    };

    #[test]
    fn the_header_names_the_lvds_numbers_and_the_structs() {
        for (name, v) in
            [("YD_IMX95_LDB_PHYS", PHYS.ldb), ("YD_IMX95_TPM5_PHYS", PHYS.tpm), ("YD_IMX95_GPIO5_PHYS", PHYS.gpio)]
        {
            assert_eq!(def(name), v as u64, "{name}");
        }
        assert!(def("YD_IMX95_LDB_SIZE") >= 0x10);
        assert_eq!(def("YD_IMX95_LVDS_STOP_DPU"), u64::from(STOP_DISPLAY));
        assert_eq!(def("YD_IMX95_LVDS_STOP_BACKLIGHT"), u64::from(STOP_BACKLIGHT));
        // the panel is the X4 one's size: the board layer's screen does not
        // depend on the output
        assert_eq!(
            (def("YD_IMX95_DISPLAY_WIDTH"), def("YD_IMX95_DISPLAY_HEIGHT")),
            (u64::from(PANEL.hactive), u64::from(PANEL.vactive))
        );
        for (what, v) in [
            ("sizeof(struct yd_imx95_lvds_bases)", size_of::<YdImx95LvdsBases>()),
            ("offsetof(struct yd_imx95_lvds_bases, gpio)", offset_of!(YdImx95LvdsBases, gpio)),
            ("sizeof(struct yd_imx95_lvds)", size_of::<YdImx95Lvds>()),
            ("offsetof(struct yd_imx95_lvds, ldb)", offset_of!(YdImx95Lvds, ldb)),
            ("offsetof(struct yd_imx95_lvds, stop)", offset_of!(YdImx95Lvds, stop)),
        ] {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
        for f in ["int yd_imx95_lvds_up(", "int yd_imx95_lvds_flip(", "int yd_imx95_lvds_down("] {
            assert!(H.contains(f), "{f}");
        }
    }

    #[test]
    fn up_flip_down_through_the_c_entries_logic() {
        let mut r = rig();
        let mut d = YdImx95Lvds::default();
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), 0);
        assert_eq!((d.state, d.fb, d.stride, d.width, d.height), (STATE_UP, FB.addr, 5120, 1280, 800));
        assert_eq!(
            (d.pix_hz, d.ldb, d.tpm_hz, d.backlight),
            ([71_100_000, 0], [0x14, 0x82221, 0x1075, 0], 24_000_000, 119)
        );
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -EBUSY);

        let second = FB.addr + 0x40_0000;
        assert_eq!(flip_on(&r.m, &r.env, &PHYS, second, &mut d), 0);
        assert_eq!((d.fb, d.flips, r.m.display.scanned()), (second, 1, Some(second)));
        assert_eq!(flip_on(&r.m, &r.env, &PHYS, second + 2, &mut d), -EINVAL);

        assert_eq!(down_on(&mut r.sm, &r.m, &r.env, &PHYS, &mut d), 0);
        assert_eq!(d.state, STATE_DOWN);
        assert!(!r.sm.sm.borrow().on(13) && !r.sm.sm.borrow().enabled(38));
        assert_eq!(flip_on(&r.m, &r.env, &PHYS, FB.addr, &mut d), -ENODEV);
        assert!(r.m.display.refused().is_empty());
    }

    #[test]
    fn a_stopped_up_names_its_side_and_a_missing_block_sends_nothing() {
        let mut r = rig();
        let mut d = YdImx95Lvds::default();
        assert_eq!(
            up_on(&mut r.sm, &r.m, &r.env, &YdImx95LvdsBases { gpio: 0, ..PHYS }, FB.addr, 5120, &mut d),
            -EINVAL
        );
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, 0, &mut d), -EINVAL);
        assert!(r.sm.sm.borrow().log.is_empty());
        assert_eq!(d, YdImx95Lvds::default());

        r.sm.sm.borrow_mut().clk_perm.insert(lv::CLK_TPM5, 0);
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -EIO);
        assert_eq!((d.state, d.stop), (STATE_FAILED, STOP_BACKLIGHT));
        assert!(r.env.lines.borrow().iter().any(|l| l.contains("the up stopped: BacklightClock")));
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -ENODEV);

        let mut r = rig();
        let mut d = YdImx95Lvds::default();
        r.sm.sm.borrow_mut().pd_perm.insert(13, 0);
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -EIO);
        assert_eq!((d.state, d.stop), (STATE_FAILED, STOP_DISPLAY));
    }
}
