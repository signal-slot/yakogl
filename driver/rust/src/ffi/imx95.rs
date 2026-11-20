// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Verdin iMX95's Mali bring-up for the Zephyr Mali OSAL
//! (driver/zephyr/lib/board/verdin_imx95/verdin_imx95_mali.c):
//! [`yd_imx95_osal_up`], the i.MX 95's [`crate::board::imx95_gpu::Imx95Power`]
//! over the board's SCMI channel, then the same OSAL bring-up as the
//! NanoPC-T6's [`super::rk3588::yd_t6_osal_up`] ([`super::rk3588::osal_up_on`]).
//! After it the OSAL ops are the `yd_t6_osal_*` entries, which touch the GPU
//! and the OSAL's memory only.

#[cfg(feature = "mali")]
use super::imx95_display::CScmi;
use super::rk3588::{ident_words, lo_hi, YdT6GpuUp};
use crate::board::imx95_gpu::{self, GpuUp};
use core::mem::{offset_of, size_of};

/// `struct yd_imx95_gpu_power`: what the System Manager answered
/// ([`GpuUp::power`], [`GpuUp::power_error`]). A 64-bit value is two
/// words, lo first.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YdImx95GpuPower {
    /// 1 when every SCMI message of the power-on was answered as asked.
    pub on: u32,
    /// The rate asked for, the level set (its index and kHz) and the clock
    /// gate's rate read after the enable.
    pub requested_hz: [u32; 2],
    pub level_index: u32,
    pub level_khz: u32,
    pub rate_hz: [u32; 2],
    /// Where the power-on stopped: the refusal number
    /// ([`imx95_gpu::scmi_error_code`]; 0 when it did not), the message
    /// (protocol << 8 | message id, [`imx95_gpu::step_code`]; 0 when the
    /// stop is not one message's), the SCMI status of a refusal (2; 0
    /// otherwise), and the refusal's detail.
    pub refusal: u32,
    pub message: u32,
    pub status: i32,
    pub detail: [u32; 2],
}

const _: () = assert!(size_of::<YdImx95GpuPower>() == 4 * 12);
const _: () = assert!(offset_of!(YdImx95GpuPower, refusal) == 4 * 7);
const _: () = assert!(offset_of!(YdImx95GpuPower, detail) == 4 * 10);

impl From<&GpuUp> for YdImx95GpuPower {
    fn from(g: &GpuUp) -> YdImx95GpuPower {
        use crate::soc::imx95::gpu_power::Error;
        let mut o = YdImx95GpuPower::default();
        if let Some(up) = g.power {
            o.on = 1;
            o.requested_hz = lo_hi(up.requested_hz);
            o.level_index = up.level.index;
            o.level_khz = up.level.khz;
            o.rate_hz = lo_hi(up.rate_hz);
        }
        if let Some(e) = g.power_error {
            let (refusal, detail) = imx95_gpu::scmi_error_code(&e);
            o.refusal = refusal;
            o.detail = lo_hi(detail);
            o.message = match e {
                Error::Transport { step, .. }
                | Error::Refused { step, .. }
                | Error::Echo { step, .. }
                | Error::Length { step, .. }
                | Error::Version { step, .. }
                | Error::Readback { step, .. } => imx95_gpu::step_code(step),
                Error::Levels { .. } | Error::NoLevel { .. } | Error::Rate { .. } => 0,
            };
            if let Error::Refused { status, .. } = e {
                o.status = status.code();
            }
        }
        o
    }
}

/// The i.MX 95's record in the OSAL's `struct yd_t6_gpu_up`: the GPU's own
/// part (the identity before and after the reset, the reset, the L2) and
/// where the run stopped; the RK3588's words (PMU, CRU, RK806, SPI2, QoS,
/// take-over) stay 0. The SCMI part is [`YdImx95GpuPower`].
impl From<&GpuUp> for YdT6GpuUp {
    fn from(g: &GpuUp) -> YdT6GpuUp {
        YdT6GpuUp {
            ident: ident_words(&g.ident),
            reset: [g.reset.raw_before, g.reset.raw_cleared, g.reset.waited_us],
            ident_after_reset: ident_words(&g.ident_after_reset),
            l2_mask: lo_hi(g.l2.mask),
            l2_ready_before: lo_hi(g.l2.ready_before),
            l2_ready: lo_hi(g.l2.ready),
            l2_waited_us: g.l2.waited_us,
            stop: g.stop as u32,
            refusal: g.refusal,
            detail: lo_hi(g.detail),
            ..YdT6GpuUp::default()
        }
    }
}

/// Bring the Mali OSAL up on the i.MX 95: the GPU powered over SCMI
/// ([`crate::soc::imx95::gpu_power::power_on`] through `scmi`, at the
/// fastest level at or below `rate_hz`; 0 for the Verdin's 800 MHz), its
/// identity, reset and L2, then the CSF firmware and the group as
/// [`super::rk3588::yd_t6_osal_up`] does, over the GPU mapped at `gpu` for
/// `gpu_len` bytes (at least [`imx95_gpu::GPU_MAP_MIN`]). `blob`,
/// `timer_hz`, `mem`, `clean`, `delay_us`, `out` are `yd_t6_osal_up`'s;
/// `on_handover` sees `z1` once the SM has powered the GPU, before its first
/// register is read. `z1` gets the GPU's part of the record and the stop
/// (`YD_IMX95_GPU_STOP_SCMI` for the power-on, the NanoPC-T6's numbers for
/// the GPU's own steps), `power` what the SM answered. 0 when up (or
/// already up), `-EIO` when a step stopped (`out->step`, as for
/// `yd_t6_osal_up`), `-EINVAL` for a null or misaligned argument, a short
/// `gpu_len`, a firmware that does not parse or does not fit `mem->fw`
/// (nothing sent, nothing touched).
///
/// # Safety
/// As [`super::rk3588::yd_t6_osal_up`]'s; `scmi` is safe to call with
/// valid buffers, `power` valid for a write. One thread only.
#[cfg(feature = "mali")]
#[no_mangle]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn yd_imx95_osal_up(
    scmi: Option<extern "C" fn(*const u8, usize, *mut u8, usize) -> i32>,
    rate_hz: u64,
    gpu: usize,
    gpu_len: usize,
    blob: *const u8,
    blob_len: usize,
    timer_hz: u32,
    mem: *const super::rk3588::YdT6OsalMem,
    clean: Option<extern "C" fn(usize, usize) -> i32>,
    delay_us: Option<extern "C" fn(u32)>,
    on_handover: Option<extern "C" fn(*const YdT6GpuUp)>,
    z1: *mut YdT6GpuUp,
    power: *mut YdImx95GpuPower,
    out: *mut super::rk3588::YdT6SubmitUp,
) -> i32 {
    use crate::board::imx95_gpu::Imx95Power;
    use crate::board::nanopc_t6_submit::SubmitUp;
    use crate::errno::{EINVAL, EIO};
    use crate::soc::imx95::gpu_power::Config;
    let (Some(scmi), Some(clean), Some(delay), Some(on_handover)) = (scmi, clean, delay_us, on_handover) else {
        return -EINVAL;
    };
    if gpu == 0 || gpu_len < imx95_gpu::GPU_MAP_MIN || timer_hz == 0 {
        return -EINVAL;
    }
    if out.is_null() || z1.is_null() || power.is_null() || blob.is_null() || mem.is_null() {
        return -EINVAL;
    }
    // SAFETY: the caller's contract.
    let (z, blob) = unsafe { (*mem, core::slice::from_raw_parts(blob, blob_len)) };
    let cfg = if rate_hz == 0 { Config::VERDIN } else { Config { rate_hz } };
    let mut p = Imx95Power::new(CScmi(scmi), cfg);
    let mut o = SubmitUp::default();
    let mut g = GpuUp::default();
    let mut handover = |h: &GpuUp| {
        let h = YdT6GpuUp::from(h);
        on_handover(&h);
    };
    // SAFETY: the caller's contract.
    let r = unsafe {
        super::rk3588::osal_up_on(&mut p, gpu, blob, timer_hz, z, clean, delay, &mut handover, &mut g, &mut o)
    };
    let Ok(r) = r else { return -EINVAL };
    // SAFETY: the caller's contract.
    unsafe {
        *z1 = YdT6GpuUp::from(&g);
        *power = YdImx95GpuPower::from(&g);
        *out = o;
    }
    if r.is_ok() {
        0
    } else {
        -EIO
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::imx95_gpu::Stop;
    use crate::gpu::mali::{Ident, L2On, Reset};
    use crate::soc::imx95::gpu_power::{Error, Level, Status, Step, Up};

    const H: &str = include_str!("../../include/yakogl_drivers.h");

    fn def(name: &str) -> u64 {
        let l = H.lines().find(|l| l.starts_with(&format!("#define {name} "))).expect(name);
        let v = l.split_whitespace().nth(2).expect(name);
        match v.strip_prefix("0x") {
            Some(h) => u64::from_str_radix(h, 16).unwrap(),
            None => v.parse().unwrap_or_else(|_| panic!("{name}: {v}")),
        }
    }

    /// The header names the stop and the map minimum as the Rust side has
    /// them, and pins the record's layout.
    #[test]
    fn the_header_names_the_imx95_numbers_and_the_record() {
        assert_eq!(def("YD_IMX95_GPU_STOP_SCMI"), Stop::Scmi as u64);
        assert_eq!(def("YD_IMX95_GPU_MAP_MIN"), imx95_gpu::GPU_MAP_MIN as u64);
        assert_eq!(def("YD_IMX95_SCMI_MAX_MSG"), crate::firmware::scmi::MAX_MSG as u64);
        for (what, v) in [
            ("sizeof(struct yd_imx95_gpu_power)", size_of::<YdImx95GpuPower>()),
            ("offsetof(struct yd_imx95_gpu_power, refusal)", offset_of!(YdImx95GpuPower, refusal)),
            ("offsetof(struct yd_imx95_gpu_power, detail)", offset_of!(YdImx95GpuPower, detail)),
        ] {
            let pin = format!("_Static_assert({what} == {v},");
            assert!(H.contains(&pin), "{pin}");
        }
        assert!(H.contains("int yd_imx95_osal_up("));
    }

    /// A power-on that ran: the SM's answers in the record, the GPU's part
    /// in the OSAL's record, the RK3588's words 0.
    #[test]
    fn a_powered_gpu_fills_both_records() {
        let up = Up { requested_hz: 800_000_000, level: Level { index: 2, khz: 800_000 }, rate_hz: 799_999_000 };
        let id = Ident { gpu_id: 0xac74_0001, shader_present: 0x3, ..Ident::default() };
        let g = GpuUp {
            power: Some(up),
            ident: id,
            reset: Reset { raw_before: 0x100, raw_cleared: 0, waited_us: 7 },
            ident_after_reset: id,
            l2: L2On { mask: 1, ready_before: 0, ready: 1, waited_us: [3, 0] },
            ..GpuUp::default()
        };
        let p = YdImx95GpuPower::from(&g);
        assert_eq!((p.on, p.requested_hz, p.level_index, p.level_khz), (1, [800_000_000, 0], 2, 800_000));
        assert_eq!((p.rate_hz, p.refusal, p.message, p.status), ([799_999_000, 0], 0, 0, 0));
        let r = YdT6GpuUp::from(&g);
        assert_eq!((r.ident[0], r.ident_after_reset[7], r.reset, r.l2_ready), (0xac74_0001, 3, [0x100, 0, 7], [1, 0]));
        assert_eq!(r.l2_waited_us, [3, 0]);
        assert_eq!((r.stop, r.refusal, r.detail), (0, 0, [0, 0]));
        assert_eq!(
            YdT6GpuUp {
                ident: [0; 16],
                ident_after_reset: [0; 16],
                reset: [0; 3],
                l2_mask: [0; 2],
                l2_ready: [0; 2],
                l2_waited_us: [0; 2],
                ..r
            },
            YdT6GpuUp::default()
        );
    }

    /// A refusal: the message, the status by its DEN0056 number, the stop.
    #[test]
    fn a_refused_power_on_names_the_message_and_the_status() {
        let e = Error::Refused { step: Step::ClockEnable, status: Status::Denied };
        let (refusal, detail) = imx95_gpu::scmi_error_code(&e);
        let g = GpuUp { power_error: Some(e), stop: Stop::Scmi, refusal, detail, ..GpuUp::default() };
        let p = YdImx95GpuPower::from(&g);
        assert_eq!((p.on, p.refusal, p.message, p.status), (0, 2, 0x1407, -3));
        assert_eq!(p.detail, [(-3i32) as u32, 0x1407]);
        let r = YdT6GpuUp::from(&g);
        assert_eq!((r.stop, r.refusal, r.detail), (15, 2, p.detail));
        // a stop that is not one message's
        let e = Error::NoLevel { rate_hz: 100_000_000 };
        let (refusal, detail) = imx95_gpu::scmi_error_code(&e);
        let p = YdImx95GpuPower::from(&GpuUp { power_error: Some(e), refusal, detail, ..GpuUp::default() });
        assert_eq!((p.refusal, p.message, p.status, p.detail), (8, 0, 0, [100_000_000, 0]));
    }
}
