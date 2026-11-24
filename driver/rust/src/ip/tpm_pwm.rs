// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The NXP Timer/PWM Module (TPM) as an edge-aligned PWM on one channel: the
//! i.MX 95's backlight (`PWM_3_DSI`, TPM6 channel 2 on the Verdin iMX95).
//!
//! Written from hal_nxp (BSD-3-Clause, 1e91007b): the register map
//! `mcux-sdk-ng/devices/i.MX/i.MX95/periph/PERI_TPM.h` (VERID 0x0, PARAM
//! 0x4 with the counter WIDTH in 23:16, GLOBAL 0x8 with RST in bit 1, SC
//! 0x10, CNT 0x14, MOD 0x18, CnSC/CnV at 0x20 + 8n, POL 0x70, CONF 0x84),
//! the i.MX 95's TPM features (`MIMX9596_cm7_features.h:866-908`: four
//! channels, a 32-bit counter, GLOBAL and POL present, PWM pause level
//! select), and the driver `drivers/tpm/fsl_tpm.c`, whose order this is:
//!
//! 1. `TPM_Init` (:119-160): GLOBAL.RST set then cleared (`TPM_Reset`,
//!    fsl_tpm.h:464-468), SC = the prescaler, CONF = 0 (the defaults: no
//!    doze, no triggers, debug mode off), POL = 0 (every channel active
//!    high).
//! 2. `TPM_SetupPwm` edge-aligned (:541-640): SC.CPWMS cleared, MOD = clock
//!    / PWM rate - 1, written until it reads back (ERR008068).
//! 3. `TPM_SetupSinglePwmChannel` (:301-520): the channel disabled (its
//!    mode and level bits 0, until they read back), then enabled in
//!    edge-aligned PWM mode (MSB:MSA = 10) until it reads back
//!    (`TPM_EnableChannel`, fsl_tpm.h:614-648), then CnV = the duty count,
//!    written until it reads back (ERR008068).
//! 4. `TPM_StartTimer` (fsl_tpm.h:1000-1011): SC.CMOD = 01, the counter
//!    counts every TPM clock.
//!
//! The channel's level bits: ELSB:ELSA = 10 ([`CNSC_EPWM_HIGH_TRUE`]). On a
//! TPM without pause level select that is "high-true pulses" (hal's
//! `kTPM_HighTrue` = 2 shifted to ELSA, fsl_tpm.h:90-100); on one with it
//! (the i.MX 95's), ELSA 0 is high-true and ELSB 1 sets the output while the
//! counter is paused. Both read the same while the PWM runs; ELSB:ELSA = 00
//! would, on the former, disconnect the pin. The duty is the backlight's
//! brightness level over its maximum, of the period, as Linux's
//! pwm-backlight scales it (a fact of the board's devicetree,
//! `brightness-levels`).
//!
//! Low-true pulses ([`Pwm::low_true`], Linux's `PWM_POLARITY_INVERSED`,
//! which the Verdin iMX95's X12 backlight uses): ELSB:ELSA = 01
//! ([`CNSC_EPWM_LOW_TRUE`]): hal's `kTPM_LowTrue` is 1 shifted to ELSA on
//! both kinds of TPM (fsl_tpm.h:90-100; fsl_tpm.c:320-322 puts the level in
//! ELSA and the pause level in ELSB, here 0: the pin low while paused). The
//! duty count is the same: the output is low for level / max of the period.
//!
//! Every read-back loop is bounded ([`READBACK_TRIES`]); a register that
//! never takes its value is `-ETIMEDOUT`.

use crate::errno::{EINVAL, ETIMEDOUT};
use crate::mmio::Mmio;

pub mod regs {
    pub const PARAM: u32 = 0x04;
    pub const PARAM_WIDTH_SHIFT: u32 = 16;
    pub const GLOBAL: u32 = 0x08;
    pub const GLOBAL_RST: u32 = 1 << 1;
    pub const SC: u32 = 0x10;
    pub const SC_PS_MASK: u32 = 0x7;
    pub const SC_CMOD_MASK: u32 = 0x3 << 3;
    pub const SC_CMOD_TPM_CLOCK: u32 = 0x1 << 3;
    pub const SC_CPWMS: u32 = 1 << 5;
    pub const MOD: u32 = 0x18;
    pub const fn cnsc(ch: u32) -> u32 {
        0x20 + 8 * ch
    }
    pub const fn cnv(ch: u32) -> u32 {
        0x24 + 8 * ch
    }
    pub const CNSC_ELSA: u32 = 1 << 2;
    pub const CNSC_ELSB: u32 = 1 << 3;
    pub const CNSC_MSA: u32 = 1 << 4;
    pub const CNSC_MSB: u32 = 1 << 5;
    pub const CNSC_MODE_MASK: u32 = CNSC_ELSA | CNSC_ELSB | CNSC_MSA | CNSC_MSB;
    pub const POL: u32 = 0x70;
    pub const CONF: u32 = 0x84;
}

use regs as r;

/// Edge-aligned PWM, high-true (module text).
pub const CNSC_EPWM_HIGH_TRUE: u32 = r::CNSC_MSB | r::CNSC_ELSB;
/// Edge-aligned PWM, low-true (module text).
pub const CNSC_EPWM_LOW_TRUE: u32 = r::CNSC_MSB | r::CNSC_ELSA;
/// The i.MX 95 TPM's channels.
pub const CHANNELS: u32 = 4;
/// Writes of a read-back loop before it gives up.
pub const READBACK_TRIES: u32 = 1000;

/// The PWM asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pwm {
    pub channel: u32,
    /// The period, ns (the devicetree's `pwms` cell).
    pub period_ns: u64,
    /// Duty = level / max of the period.
    pub level: u32,
    pub max: u32,
    /// Low-true pulses (Linux's inversed polarity), else high-true.
    pub low_true: bool,
}

impl Pwm {
    /// The channel's mode and level bits.
    pub const fn cnsc(&self) -> u32 {
        if self.low_true {
            CNSC_EPWM_LOW_TRUE
        } else {
            CNSC_EPWM_HIGH_TRUE
        }
    }
}

/// The register values for a [`Pwm`] at a TPM clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// SC.PS: the clock divided by 2^ps.
    pub ps: u32,
    pub modulo: u32,
    pub cnv: u32,
}

/// The smallest prescaler whose period count fits the counter
/// (`width` bits), MOD = counts - 1, CnV = counts x level / max.
pub fn plan(pwm: &Pwm, clock_hz: u64, width: u32) -> Result<Plan, i32> {
    if pwm.channel >= CHANNELS || pwm.max == 0 || pwm.level > pwm.max || pwm.period_ns == 0 || clock_hz == 0 {
        return Err(-EINVAL);
    }
    let top = if width >= 32 { u64::from(u32::MAX) } else { (1u64 << width.max(1)) - 1 };
    for ps in 0..=7u32 {
        let counts = (clock_hz >> ps) * pwm.period_ns / 1_000_000_000;
        if counts < 2 {
            return Err(-EINVAL);
        }
        if counts - 1 <= top {
            let cnv = counts * u64::from(pwm.level) / u64::from(pwm.max);
            return Ok(Plan { ps, modulo: (counts - 1) as u32, cnv: cnv as u32 });
        }
    }
    Err(-EINVAL)
}

/// The counter width PARAM reports (32 if it reads 0: not a TPM that says).
pub fn width(m: &impl Mmio, base: usize) -> u32 {
    match (m.read32(base + r::PARAM as usize) >> r::PARAM_WIDTH_SHIFT) & 0xff {
        0 => 32,
        w => w,
    }
}

fn write_until(m: &impl Mmio, addr: usize, val: u32, mask: u32) -> Result<(), i32> {
    for _ in 0..READBACK_TRIES {
        let cur = m.read32(addr);
        m.write32(addr, (cur & !mask) | (val & mask));
        if m.read32(addr) & mask == val & mask {
            return Ok(());
        }
    }
    Err(-ETIMEDOUT)
}

/// Program and start the PWM (the module documentation's order) at
/// `clock_hz`, the TPM's clock root. Returns the plan written.
pub fn start(m: &impl Mmio, base: usize, pwm: &Pwm, clock_hz: u64) -> Result<Plan, i32> {
    let p = plan(pwm, clock_hz, width(m, base))?;
    let at = |off: u32| base + off as usize;
    // TPM_Init
    m.write32(at(r::GLOBAL), m.read32(at(r::GLOBAL)) | r::GLOBAL_RST);
    m.write32(at(r::GLOBAL), m.read32(at(r::GLOBAL)) & !r::GLOBAL_RST);
    m.write32(at(r::SC), p.ps & r::SC_PS_MASK);
    m.write32(at(r::CONF), 0);
    m.write32(at(r::POL), 0);
    // TPM_SetupPwm
    m.write32(at(r::SC), m.read32(at(r::SC)) & !r::SC_CPWMS);
    write_until(m, at(r::MOD), p.modulo, u32::MAX)?;
    // TPM_SetupSinglePwmChannel
    write_until(m, at(r::cnsc(pwm.channel)), 0, r::CNSC_MODE_MASK)?;
    write_until(m, at(r::cnsc(pwm.channel)), pwm.cnsc(), r::CNSC_MODE_MASK)?;
    write_until(m, at(r::cnv(pwm.channel)), p.cnv, u32::MAX)?;
    // TPM_StartTimer
    let sc = m.read32(at(r::SC));
    m.write32(at(r::SC), (sc & !r::SC_CMOD_MASK) | r::SC_CMOD_TPM_CLOCK);
    Ok(p)
}

/// The counter stopped (`TPM_Deinit`'s first step, fsl_tpm.c:174-185).
pub fn stop(m: &impl Mmio, base: usize) {
    let sc = base + r::SC as usize;
    m.write32(sc, m.read32(sc) & !r::SC_CMOD_MASK);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::vec::Vec;

    const BASE: usize = 0x4251_0000;

    /// A register file that logs writes; PARAM reads a 32-bit counter, and
    /// `stuck` keeps one register at 0.
    #[derive(Default)]
    struct Tpm {
        regs: RefCell<HashMap<u32, u32>>,
        writes: RefCell<Vec<(u32, u32)>>,
        width: u32,
        stuck: Option<u32>,
    }

    impl Mmio for Tpm {
        fn read32(&self, a: usize) -> u32 {
            let off = (a - BASE) as u32;
            if off == r::PARAM {
                return self.width << 16 | 4;
            }
            *self.regs.borrow().get(&off).unwrap_or(&0)
        }
        fn write32(&self, a: usize, v: u32) {
            let off = (a - BASE) as u32;
            self.writes.borrow_mut().push((off, v));
            if self.stuck != Some(off) {
                self.regs.borrow_mut().insert(off, v);
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    /// The Verdin iMX95 DSI backlight: `pwms = <&tpm6 2 6666667 0>`,
    /// default level 4 of <0 45 63 88 119 158 203 255>: 119 / 255.
    const BACKLIGHT: Pwm = Pwm { channel: 2, period_ns: 6_666_667, level: 119, max: 255, low_true: false };

    #[test]
    fn the_backlight_at_24_mhz_is_150_hz_and_119_of_255() {
        let t = Tpm { width: 32, ..Default::default() };
        let p = start(&t, BASE, &BACKLIGHT, 24_000_000).unwrap();
        // 24 MHz x 6.666667 ms = 160000 counts; 160000 x 119 / 255 = 74666
        assert_eq!(p, Plan { ps: 0, modulo: 159_999, cnv: 74_666 });
        assert_eq!(
            *t.writes.borrow(),
            [
                (r::GLOBAL, 2),
                (r::GLOBAL, 0),
                (r::SC, 0),
                (r::CONF, 0),
                (r::POL, 0),
                (r::SC, 0),
                (r::MOD, 159_999),
                (r::cnsc(2), 0),
                (r::cnsc(2), 0x28),
                (r::cnv(2), 74_666),
                (r::SC, 0x08),
            ]
        );
        assert_eq!((r::cnsc(2), r::cnv(2)), (0x30, 0x34));
    }

    #[test]
    fn the_x12_backlight_is_tpm5_channel_0_low_true_at_119_of_255() {
        // Linux on the board (2026-11-25): platform/42500000.pwm channel 0,
        // period 6,666,667 ns, duty 3,111,111 ns, polarity inverse: 119 /
        // 255 of the period (3,111,111 / 6,666,667 = 0.4667), low-true
        let pwm = Pwm { channel: 0, low_true: true, ..BACKLIGHT };
        let t = Tpm { width: 32, ..Default::default() };
        let p = start(&t, BASE, &pwm, 24_000_000).unwrap();
        assert_eq!(p, Plan { ps: 0, modulo: 159_999, cnv: 74_666 });
        // MSB | ELSA: edge-aligned, low-true pulses (kTPM_LowTrue = 1 << ELSA)
        assert_eq!(pwm.cnsc(), 0x24);
        assert!(t.writes.borrow().contains(&(r::cnsc(0), 0x24)));
        assert!(t.writes.borrow().contains(&(r::cnv(0), 74_666)));
        // 74,666 / 24 MHz = 3,111.08 µs: Linux's duty to the count
        assert_eq!(74_666u64 * 1_000_000_000 / 24_000_000, 3_111_083);
    }

    #[test]
    fn a_16_bit_counter_takes_the_smallest_prescaler_that_fits() {
        // 160000 counts do not fit 16 bits; /4 gives 40000
        assert_eq!(plan(&BACKLIGHT, 24_000_000, 16).unwrap(), Plan { ps: 2, modulo: 39_999, cnv: 18_666 });
        let t = Tpm { width: 16, ..Default::default() };
        assert_eq!(start(&t, BASE, &BACKLIGHT, 24_000_000).unwrap().ps, 2);
    }

    #[test]
    fn full_and_zero_brightness_and_bad_requests() {
        assert_eq!(plan(&Pwm { level: 255, ..BACKLIGHT }, 24_000_000, 32).unwrap().cnv, 160_000);
        assert_eq!(plan(&Pwm { level: 0, ..BACKLIGHT }, 24_000_000, 32).unwrap().cnv, 0);
        assert_eq!(plan(&Pwm { channel: 4, ..BACKLIGHT }, 24_000_000, 32), Err(-EINVAL));
        assert_eq!(plan(&Pwm { level: 256, ..BACKLIGHT }, 24_000_000, 32), Err(-EINVAL));
        assert_eq!(plan(&BACKLIGHT, 0, 32), Err(-EINVAL));
        assert_eq!(plan(&Pwm { period_ns: 50, ..BACKLIGHT }, 24_000_000, 32), Err(-EINVAL));
    }

    #[test]
    fn a_modulo_that_never_reads_back_times_out_before_the_counter_starts() {
        let t = Tpm { width: 32, stuck: Some(r::MOD), ..Default::default() };
        assert_eq!(start(&t, BASE, &BACKLIGHT, 24_000_000), Err(-ETIMEDOUT));
        let mods = t.writes.borrow().iter().filter(|w| w.0 == r::MOD).count();
        assert_eq!(mods, READBACK_TRIES as usize);
        assert!(t.writes.borrow().iter().all(|w| w.0 != r::cnsc(2)));
        assert_eq!(t.read32(BASE + r::SC as usize) & r::SC_CMOD_MASK, 0);
    }

    #[test]
    fn stop_clears_the_clock_mode_only() {
        let t = Tpm { width: 32, ..Default::default() };
        start(&t, BASE, &BACKLIGHT, 24_000_000).unwrap();
        stop(&t, BASE);
        assert_eq!(t.read32(BASE + r::SC as usize), 0);
    }
}
