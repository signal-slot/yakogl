// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95's picture on X4 for the Zephyr board layer
//! (driver/zephyr/lib/board/verdin_imx95/verdin_imx95_display.c):
//! [`yd_imx95_display_up`], [`yd_imx95_display_flip`] and
//! [`yd_imx95_display_down`] over `board::verdin_imx95_display`, the design
//! note's twelve steps (driver/docs/imx95-display.md section 4(b)).
//!
//! - up: `display_up` with the buffer the board shows first (XRGB8888,
//!   1280x800 at `fb` with rows `stride` bytes apart), every block at the
//!   address the image mapped it ([`YdImx95DisplayBases`]), the System
//!   Manager over the board's SCMI channel (the one the GPU's power-on uses,
//!   [`CScmi`]) and the bridge and its EN line through the board's
//!   `struct yd_env` (I2C bus [`crate::ip::sn65dsi8x::BUS`], GPIO line
//!   [`crate::ip::sn65dsi8x::LINE_EN`]).
//! - flip: `soc::imx95::display::flip`: another buffer of the same stride
//!   at the next frame, waited for (ExtDst0's shadow load, at most three
//!   frames): when it returns the DPU scans the new buffer, so the one shown
//!   before is free.
//! - down: `display_down`, the DSI side then stream 0 and the power.
//!
//! The state the C side keeps is [`YdImx95Display`]; `Stream0` is rebuilt
//! from it for each call (the mode is the panel's, the route DSI). A step
//! that stops is logged through the env with its error (one line), and the
//! record says which side stopped.

use crate::board::verdin_imx95_display::{self as vd, display_down, display_up, Config};
use crate::board::verdin_imx95_dsi::PANEL;
use crate::env::{Env, YdEnv};
use crate::errno::{EBUSY, EINVAL, EIO, ENODEV, ETIMEDOUT};
use crate::firmware::scmi::Transport;
use crate::mmio::{Hw, Mmio};
use crate::soc::imx95::combo_phy;
use crate::soc::imx95::display::{self, Prepared, Stream0};
use crate::soc::imx95::dpu::{Framebuffer, Route};
use crate::ylog;
use core::mem::{offset_of, size_of};

/// The board's SCMI channel to the System Manager: `xfer(tx, tx_len, rx,
/// rx_cap)` sends the command `tx` (header and payload, as built) and
/// writes the whole reply to `rx` (header, status, return values); it
/// returns the reply's length in bytes, or a negative errno. One channel
/// for the GPU's power-on (`ffi::imx95`) and the display's.
pub(super) struct CScmi(pub(super) extern "C" fn(*const u8, usize, *mut u8, usize) -> i32);

impl Transport for CScmi {
    fn transfer(&mut self, tx: &[u8], rx: &mut [u8]) -> Result<usize, i32> {
        let n = (self.0)(tx.as_ptr(), tx.len(), rx.as_mut_ptr(), rx.len());
        if n < 0 {
            Err(n)
        } else {
            Ok(n as usize)
        }
    }
}

/// `struct yd_imx95_display_bases`: where the image mapped each block
/// (Device memory; the physical addresses are the `YD_IMX95_*_PHYS` of
/// the header, at least `YD_IMX95_*_SIZE` bytes each).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdImx95DisplayBases {
    /// The DPU (`soc::imx95::dpu::DPU_BASE`).
    pub dpu: usize,
    /// The display CSR and the pixel interleaver (`soc::imx95::display`).
    pub csr: usize,
    pub interleaver: usize,
    /// The DW DSI host and the camera mix's CSRs (`soc::imx95::combo_phy`).
    pub dsi: usize,
    pub stream: usize,
    pub master: usize,
    pub phy_csr: usize,
    pub csi1: usize,
    /// TPM6, the backlight PWM.
    pub tpm: usize,
}

/// [`YdImx95Display::state`]: no up yet.
pub const STATE_NEW: u32 = 0;
/// The picture is up; flips are taken.
pub const STATE_UP: u32 = 1;
/// The up stopped ([`YdImx95Display::stop`] says where). Final: what was
/// switched on stays on until the board's reset.
pub const STATE_FAILED: u32 = 2;
/// Taken down (or the down stopped half-way).
pub const STATE_DOWN: u32 = 3;

/// [`YdImx95Display::stop`] (0: nothing stopped): the DPU side stopped (`soc::imx95::display`: its power and pixel clock,
/// the routing, FrameGen0's sync, a flip's shadow load, the stop).
pub const STOP_DISPLAY: u32 = 1;
/// The DSI side stopped (`board::verdin_imx95_dsi`: CAMERA and the PHY
/// clocks, the host and combo PHY, the bridge, the backlight).
pub const STOP_DSI: u32 = 2;

/// `struct yd_imx95_display`: all zero before the first up; kept by the C
/// side between the calls. A 64-bit value is two words, lo first.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdImx95Display {
    /// The buffer the DPU scans (physical) and the stride of every buffer.
    pub fb: u64,
    pub stride: u32,
    /// [`STATE_NEW`], [`STATE_UP`], [`STATE_FAILED`] or [`STATE_DOWN`].
    pub state: u32,
    /// The mode shown.
    pub width: u32,
    pub height: u32,
    /// DISP1PIX as the System Manager made it, and the DISP level.
    pub pix_hz: [u32; 2],
    pub level: u32,
    /// µs FrameGen0 took to sync.
    pub sync_us: u32,
    /// The DSI lane rate the combo PHY's PLL makes, kbit/s.
    pub lane_kbps: u32,
    /// The bridge's IRQ_STAT after video started (0: a clean link).
    pub link_irq_stat: u32,
    /// The DW host's VERSION.
    pub host_version: u32,
    /// The backlight's level of 255.
    pub backlight: u32,
    /// Flips that changed the address, and the µs the last one waited.
    pub flips: u32,
    pub flip_us: u32,
    /// [`STOP_NONE`], [`STOP_DISPLAY`] or [`STOP_DSI`].
    pub stop: u32,
    pub reserved: u32,
}

const _: () = assert!(size_of::<YdImx95DisplayBases>() == 8 * 9);
const _: () = assert!(offset_of!(YdImx95DisplayBases, tpm) == 64);
const _: () = assert!(size_of::<YdImx95Display>() == 72);
const _: () = assert!(offset_of!(YdImx95Display, stride) == 8);
const _: () = assert!(offset_of!(YdImx95Display, pix_hz) == 24);
const _: () = assert!(offset_of!(YdImx95Display, stop) == 64);

/// Both halves' configuration over the mapped blocks, showing `fb`.
pub fn config(b: &YdImx95DisplayBases, fb: Framebuffer) -> Config {
    let mut c = Config::verdin(fb);
    c.display = display::Bases { dpu: b.dpu, csr: b.csr, interleaver: b.interleaver };
    c.dsi.bases = combo_phy::Bases { dsi: b.dsi, stream: b.stream, master: b.master, phy_csr: b.phy_csr, csi1: b.csi1 };
    c.dsi.tpm = b.tpm;
    c
}

fn complete(b: &YdImx95DisplayBases) -> bool {
    [b.dpu, b.csr, b.interleaver, b.dsi, b.stream, b.master, b.phy_csr, b.csi1, b.tpm].iter().all(|&a| a != 0)
}

/// The running stream as [`display::start`] left it, from the record.
fn stream(d: &YdImx95Display) -> Stream0 {
    let fb = Framebuffer { addr: d.fb, stride: d.stride };
    let pix_hz = u64::from(d.pix_hz[0]) | u64::from(d.pix_hz[1]) << 32;
    Stream0 {
        prepared: Prepared { mode: PANEL, route: Route::Dsi, fb, level: d.level, pix_hz },
        fb,
        sync_us: d.sync_us,
    }
}

/// [`yd_imx95_display_up`] over any transport, register access and env.
pub fn up_on<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    b: &YdImx95DisplayBases,
    fb: u64,
    stride: u32,
    d: &mut YdImx95Display,
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
            *d = YdImx95Display {
                fb,
                stride,
                state: STATE_UP,
                width: p.mode.hactive,
                height: p.mode.vactive,
                pix_hz: [p.pix_hz as u32, (p.pix_hz >> 32) as u32],
                level: p.level,
                sync_us: up.stream.sync_us,
                lane_kbps: (up.dsi.pll.fout_actual_hz * 2 / 1000) as u32,
                link_irq_stat: u32::from(up.dsi.link_irq_stat),
                host_version: up.dsi.host_version,
                backlight: cfg.dsi.backlight.level,
                ..YdImx95Display::default()
            };
            0
        }
        Err(e) => {
            *d = YdImx95Display { fb, stride, state: STATE_FAILED, stop: side(&e), ..YdImx95Display::default() };
            ylog!(env, Err, "verdin-imx95 display: the up stopped: {:?}", e);
            -EIO
        }
    }
}

fn side(e: &vd::Error) -> u32 {
    match e {
        vd::Error::Display(_) => STOP_DISPLAY,
        vd::Error::Dsi(_) => STOP_DSI,
    }
}

/// [`yd_imx95_display_flip`] over any register access and env.
pub fn flip_on(m: &impl Mmio, env: &impl Env, b: &YdImx95DisplayBases, fb: u64, d: &mut YdImx95Display) -> i32 {
    if d.state != STATE_UP {
        return -ENODEV;
    }
    if fb == d.fb {
        return 0;
    }
    let bases = display::Bases { dpu: b.dpu, csr: b.csr, interleaver: b.interleaver };
    let mut s = stream(d);
    let mut delay = |us: u32| env.busy_wait_us(us);
    match display::flip(m, &bases, &mut s, fb, &mut delay) {
        Ok(waited) => {
            d.fb = fb;
            d.flips = d.flips.wrapping_add(1);
            d.flip_us = waited;
            0
        }
        Err(display::Error::Refused(_)) => -EINVAL,
        Err(e) => {
            // the address is written: which buffer the DPU scans is not known
            ylog!(env, Err, "verdin-imx95 display: flip to {:#x}: {:?}", fb, e);
            d.fb = fb;
            -ETIMEDOUT
        }
    }
}

/// [`yd_imx95_display_down`] over any transport, register access and env.
pub fn down_on<T: Transport>(
    t: &mut T,
    m: &impl Mmio,
    env: &impl Env,
    b: &YdImx95DisplayBases,
    d: &mut YdImx95Display,
) -> i32 {
    if d.state != STATE_UP {
        return -ENODEV;
    }
    let s = stream(d);
    let cfg = config(b, s.fb);
    d.state = STATE_DOWN;
    match display_down(t, m, env, &cfg, &s) {
        Ok(()) => 0,
        Err(e) => {
            d.stop = side(&e);
            ylog!(env, Err, "verdin-imx95 display: the down stopped: {:?}", e);
            -EIO
        }
    }
}

/// The 10.1" panel on: the design note's twelve steps, showing the
/// XRGB8888 buffer at physical `fb` (rows `stride` bytes apart, at least
/// `YD_IMX95_DISPLAY_WIDTH` x 4). 0 when the picture is up; `-EBUSY`
/// when it already was; `-EINVAL` for a missing block, buffer or callback
/// (nothing sent or written); `-EIO` when a step stopped (`d->stop` says
/// which side; the error is one log line through `env`); `-ENODEV` after
/// an up that stopped or a down.
///
/// # Safety
/// `bases`, `env` and `d` valid; each base the mapping of its block;
/// `scmi` safe to call with valid buffers. One thread only.
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_display_up(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    bases: *const YdImx95DisplayBases,
    env: *const YdEnv,
    fb: u64,
    stride: u32,
    d: *mut YdImx95Display,
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
/// waiting for the DPU to take it (at most three frames). 0 when it did (or
/// `fb` is already shown); `-EINVAL` for a buffer the DPU cannot fetch
/// (nothing written); `-ETIMEDOUT` when the shadow load did not come (the
/// address is written; one log line); `-ENODEV` when the picture is not up.
///
/// # Safety
/// As [`yd_imx95_display_up`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_display_flip(
    bases: *const YdImx95DisplayBases,
    env: *const YdEnv,
    fb: u64,
    d: *mut YdImx95Display,
) -> i32 {
    // SAFETY: the caller's contract.
    let (Some(b), Some(env), Some(d)) = (unsafe { (bases.as_ref(), env.as_ref(), d.as_mut()) }) else {
        return -EINVAL;
    };
    // SAFETY: as above.
    let m = unsafe { Hw::new() };
    flip_on(&m, env, b, fb, d)
}

/// The picture down: backlight off, the bridge's EN low, the DSI host and
/// PHY down, CAMERA off, then stream 0 stopped and DISPLAY off. 0, `-EIO`
/// when a step stopped (one log line), `-ENODEV` when the picture is not up.
///
/// # Safety
/// As [`yd_imx95_display_up`].
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_display_down(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    bases: *const YdImx95DisplayBases,
    env: *const YdEnv,
    d: *mut YdImx95Display,
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

/// The probe imx95_scmi's survey ([`crate::soc::imx95::survey::survey`]):
/// every fact to `line` as `key = value` (UTF-8, no newline), the counts
/// (facts, refused, broken) to `counts`. 0, `-EIO` when the base version
/// was not answered (the channel is dead), `-EINVAL` for a null argument.
/// Sends only messages that read.
///
/// # Safety
/// `counts` is valid for writes of three words.
#[no_mangle]
pub unsafe extern "C" fn yd_imx95_scmi_survey(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    line: Option<extern "C" fn(*const u8, usize)>,
    counts: *mut u32,
) -> i32 {
    use crate::soc::imx95::survey::survey;
    let (Some(scmi), Some(line)) = (scmi, line) else { return -EINVAL };
    if counts.is_null() {
        return -EINVAL;
    }
    let r = survey(&mut CScmi(scmi), |l| line(l.as_ptr(), l.len()));
    let (c, rc) = match r {
        Ok(c) => (c, 0),
        Err((_, c)) => (c, -EIO),
    };
    // SAFETY: the caller's contract.
    unsafe { core::ptr::copy_nonoverlapping([c.facts, c.refused, c.broken].as_ptr(), counts, 3) };
    rc
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::verdin_imx95_display_tests::{rig, FB};
    use crate::ip::sn65dsi8x;
    use crate::soc::imx95::{display as disp, dpu};

    const H: &str = include_str!("../../include/yakogl_drivers.h");

    fn def(name: &str) -> u64 {
        let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
        let v = l.split_whitespace().nth(2).expect(name);
        match v.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h.trim_end_matches('u'), 16).unwrap(),
            None => v.trim_end_matches('u').parse().unwrap_or_else(|_| panic!("{name}: {v}")),
        }
    }

    const PHYS: YdImx95DisplayBases = YdImx95DisplayBases {
        dpu: dpu::DPU_BASE,
        csr: disp::CSR_BASE,
        interleaver: disp::INTERLEAVER_BASE,
        dsi: combo_phy::DSI_BASE,
        stream: combo_phy::STREAM_CSR_BASE,
        master: combo_phy::MASTER_CSR_BASE,
        phy_csr: combo_phy::PHY_CSR_BASE,
        csi1: combo_phy::CSI1_BASE,
        tpm: crate::board::verdin_imx95_dsi::TPM6_BASE,
    };

    /// The header's addresses, panel, env indices and states are the Rust
    /// side's, and its asserts pin both structs.
    #[test]
    fn the_header_names_the_display_numbers_and_the_structs() {
        for (name, v) in [
            ("YD_IMX95_DPU_PHYS", PHYS.dpu),
            ("YD_IMX95_DISPLAY_CSR_PHYS", PHYS.csr),
            ("YD_IMX95_INTERLEAVER_PHYS", PHYS.interleaver),
            ("YD_IMX95_DSI_PHYS", PHYS.dsi),
            ("YD_IMX95_DSI_STREAM_CSR_PHYS", PHYS.stream),
            ("YD_IMX95_DSI_MASTER_CSR_PHYS", PHYS.master),
            ("YD_IMX95_COMBO_PHY_CSR_PHYS", PHYS.phy_csr),
            ("YD_IMX95_CSI1_PHYS", PHYS.csi1),
            ("YD_IMX95_TPM6_PHYS", PHYS.tpm),
        ] {
            assert_eq!(def(name), v as u64, "{name}");
        }
        // the sizes cover what the models answer for
        assert!(def("YD_IMX95_DPU_SIZE") >= 0x40_0000);
        assert!(def("YD_IMX95_INTERLEAVER_SIZE") >= 0x2_0000);
        assert_eq!(
            (def("YD_IMX95_DISPLAY_WIDTH"), def("YD_IMX95_DISPLAY_HEIGHT")),
            (u64::from(PANEL.hactive), u64::from(PANEL.vactive))
        );
        assert_eq!(def("YD_IMX95_DISPLAY_BRIDGE_BUS"), u64::from(sn65dsi8x::BUS));
        assert_eq!(def("YD_IMX95_DISPLAY_LINE_EN"), u64::from(sn65dsi8x::LINE_EN));
        for (name, v) in [
            ("YD_IMX95_DISPLAY_NEW", STATE_NEW),
            ("YD_IMX95_DISPLAY_UP", STATE_UP),
            ("YD_IMX95_DISPLAY_FAILED", STATE_FAILED),
            ("YD_IMX95_DISPLAY_DOWN", STATE_DOWN),
            ("YD_IMX95_DISPLAY_STOP_DPU", STOP_DISPLAY),
            ("YD_IMX95_DISPLAY_STOP_DSI", STOP_DSI),
        ] {
            assert_eq!(def(name), u64::from(v), "{name}");
        }
        for (what, v) in [
            ("sizeof(struct yd_imx95_display_bases)", size_of::<YdImx95DisplayBases>()),
            ("offsetof(struct yd_imx95_display_bases, tpm)", offset_of!(YdImx95DisplayBases, tpm)),
            ("sizeof(struct yd_imx95_display)", size_of::<YdImx95Display>()),
            ("offsetof(struct yd_imx95_display, stride)", offset_of!(YdImx95Display, stride)),
            ("offsetof(struct yd_imx95_display, pix_hz)", offset_of!(YdImx95Display, pix_hz)),
            ("offsetof(struct yd_imx95_display, stop)", offset_of!(YdImx95Display, stop)),
        ] {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
        for f in ["int yd_imx95_display_up(", "int yd_imx95_display_flip(", "int yd_imx95_display_down("] {
            assert!(H.contains(f), "{f}");
        }
    }

    /// Up, a flip to a second buffer, down: the record follows, the model
    /// scans what was shown, and a second up is refused.
    #[test]
    fn up_flip_down_through_the_c_entries_logic() {
        let mut r = rig();
        let mut d = YdImx95Display::default();
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), 0);
        assert_eq!((d.state, d.fb, d.stride, d.width, d.height), (STATE_UP, FB.addr, 5120, 1280, 800));
        assert_eq!((d.pix_hz, d.lane_kbps, d.link_irq_stat, d.backlight), ([70_000_000, 0], 420_000, 0, 119));
        assert_eq!(r.m.display.scanned(), Some(FB.addr));
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -EBUSY);

        let second = FB.addr + 0x40_0000;
        assert_eq!(flip_on(&r.m, &r.env, &PHYS, second, &mut d), 0);
        assert_eq!((d.fb, d.flips), (second, 1));
        assert_eq!(r.m.display.scanned(), Some(second));
        // the same address again is no flip
        assert_eq!(flip_on(&r.m, &r.env, &PHYS, second, &mut d), 0);
        assert_eq!(d.flips, 1);
        // an address the DPU cannot fetch writes nothing
        assert_eq!(flip_on(&r.m, &r.env, &PHYS, second + 2, &mut d), -EINVAL);
        assert_eq!(r.m.display.scanned(), Some(second));

        assert_eq!(down_on(&mut r.sm, &r.m, &r.env, &PHYS, &mut d), 0);
        assert_eq!(d.state, STATE_DOWN);
        assert!(!r.sm.sm.borrow().on(3) && !r.sm.sm.borrow().on(13));
        assert_eq!(flip_on(&r.m, &r.env, &PHYS, FB.addr, &mut d), -ENODEV);
        assert_eq!(down_on(&mut r.sm, &r.m, &r.env, &PHYS, &mut d), -ENODEV);
        assert!(r.m.display.refused().is_empty());
    }

    /// An up the SM refuses names the side, logs the error, and is final;
    /// missing blocks or buffers are refused before anything is sent.
    #[test]
    fn a_stopped_up_names_its_side_and_a_missing_block_sends_nothing() {
        let mut r = rig();
        let mut d = YdImx95Display::default();
        assert_eq!(
            up_on(&mut r.sm, &r.m, &r.env, &YdImx95DisplayBases { tpm: 0, ..PHYS }, FB.addr, 5120, &mut d),
            -EINVAL
        );
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, 0, 5120, &mut d), -EINVAL);
        assert!(r.sm.sm.borrow().log.is_empty());
        assert_eq!(d, YdImx95Display::default());

        r.sm.sm.borrow_mut().pd_perm.insert(3, 0);
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -EIO);
        assert_eq!((d.state, d.stop), (STATE_FAILED, STOP_DSI));
        assert!(
            r.env.bridge.lines().iter().any(|l| l.contains("the up stopped: Dsi(Power(")),
            "{:?}",
            r.env.bridge.lines()
        );
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -ENODEV);
        assert_eq!(down_on(&mut r.sm, &r.m, &r.env, &PHYS, &mut d), -ENODEV);

        // the DPU side's refusal: DISPLAY denied
        let mut r = rig();
        let mut d = YdImx95Display::default();
        r.sm.sm.borrow_mut().pd_perm.insert(13, 0);
        assert_eq!(up_on(&mut r.sm, &r.m, &r.env, &PHYS, FB.addr, FB.stride, &mut d), -EIO);
        assert_eq!((d.state, d.stop), (STATE_FAILED, STOP_DISPLAY));
    }
}
