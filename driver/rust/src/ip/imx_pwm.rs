// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX "PWM" block (one output, a 16-bit counter, a four-word sample
//! FIFO): the i.MX 8M Plus PWM1-4.
//!
//! Written from the register descriptions in hal_nxp's `PERI_PWM.h`
//! ([`crate::soc::imx8mp::regs::pwm`]); not from any other driver. What those
//! descriptions do not say, and this driver relies on, is listed here with
//! its citation in the i.MX 8M Plus Applications Processor Reference Manual,
//! Rev. 3, 08/2024 (IMX8MPRM Rev. 3; document used, not copied):
//!
//! 1. The output period is `PWMPR + 2` counter clocks, and the counter clock
//!    is the CLKSRC clock divided by `PRESCALER + 1` ("Divide by 1" ..
//!    "Divide by 4096" in the header). IMX8MPRM Rev. 3 §12.2.6.5 "PWM Period
//!    Register (PWMx_PWMPR)" ("PWMO (Hz) = PCLK(Hz) / (period +2)") and
//!    §12.2.6.1 "PWM Control Register (PWMx_PWMCR)", PRESCALER ("the value by
//!    which the clock will be divided before it goes to the counter").
//! 2. `PWMPR` is limited here to at most 0xFFFD, so a period is at most
//!    0xFFFF counter clocks. IMX8MPRM Rev. 3 §12.2.3.1.2 "Rollover and
//!    Compare Event" and §12.2.6.5 give a wider range: `PWMPR` = 0xFFFE
//!    (period 0x10000) is valid and 0xFFFF "will achieve the same result as
//!    writing 0xFFFE" (0xFFFF is not allowed with a non-zero `REPEAT`). The
//!    limit of 0xFFFF counts is therefore this driver's own, one count
//!    below the RM's.
//! 3. With `POUTC` = 0b00 ("set at rollover and cleared at comparison") the
//!    output is high for `PWMSAR` counter clocks of each period. IMX8MPRM
//!    Rev. 3 §12.2.3.1 "Operation" (the output is set at the beginning of a
//!    period and cleared when sample and count match) and §12.2.6.1 PWMCR,
//!    POUTC. A sample above `PWMPR + 1` never compares: §12.2.6.4 "PWM
//!    Sample Register (PWMx_PWMSAR)" says the output "will never be
//!    set/reset depending on POUTC value" and does not name the level; high
//!    for 100 % is the reading of §12.2.3.1 (set at the start of each
//!    period) plus that statement.
//! 4. While `PWMCR.EN` is 0 the output is low: not stated in IMX8MPRM Rev. 3
//!    (§12.2.6.1 PWMCR, EN, says only that the prescaler and counter are
//!    reset and that enabling begins a new period with the output set); this
//!    is the driver's assumption, to be taken from the board. A sample of 0
//!    does not leave a pulse at the rollover: §12.2.6.4 PWMSAR states that
//!    zero "will result in the PWMO output signal always being low/high
//!    (POUTC = 00 it will be low and POUTC = 01 it will be high)" and that
//!    no output waveform is produced. The driver still turns a 0 % request
//!    into "disabled" rather than a sample of 0; both give a constant low
//!    output as far as the RM says.
//! 5. `PWMCR.SWR` = 1 resets the block's registers and clears itself when the
//!    reset is done. IMX8MPRM Rev. 3 §12.2.6.1 PWMCR, SWR ("a self clearing
//!    bit"; "cleared when the reset procedure is over") and §12.2.3.1.
//! 6. A sample written to `PWMSAR` while the PWM runs is used from the next
//!    period on: §12.2.6.4 PWMSAR ("the duty cycle changes after the current
//!    period is over"). The FIFO holds four words (`PWMSR.FIFOAV` = 4,
//!    §12.2.6.2 "PWM Status Register (PWMx_PWMSR)"); a write when it is full
//!    is lost and sets `PWMSR.FWE`: §12.2.3.1.1 "FIFO" ("A write when the
//!    FIFO is full sets FWE (FIFO write error) bit in the status register
//!    and the FIFO contents remain unchanged"). Note that §12.2.4 "Enable
//!    Sequence for the PWM" step 3 says a 4th sample written before enabling
//!    fills the FIFO and triggers FWE, which contradicts the four-word FIFO;
//!    this driver writes one sample after the reset.
//! 7. On the i.MX 8M Plus, `ipg_clk_highfreq` (`CLKSRC` = 0b10) of PWMn is the
//!    PWMn clock root of the CCM (`kCLOCK_RootPwmN` in hal_nxp `fsl_clock.h`).
//!    IMX8MPRM Rev. 3 §5.1.4 "System Clocks", Table 5-2: `pwm3.ipg_clk_highfreq`
//!    is fed by `PWM3_CLK_ROOT`, gated by `ccm_clk_enable_pwm3` (CCGR42); the
//!    same holds for the other PWMs and for `ipg_clk` and `ipg_clk_s`.
//!
//! A write to `PWMPR` while the block runs resets the counter and starts a
//! new period (IMX8MPRM Rev. 3 §12.2.6.5), so the "update" path below is
//! gapless for the sample only; a changed period restarts the cycle.
//!
//! Zephyr's PWM API states a period and a pulse in "cycles" of the rate
//! `get_cycles_per_sec` reports, which here is the clock root's rate. A
//! period longer than the 16-bit counter takes the smallest prescaler that
//! fits; the pulse is scaled with it and rounded to the nearest count.

use crate::env::Env;
use crate::errno::{EBUSY, EINVAL, ETIMEDOUT};
use crate::mmio::Mmio;
use crate::soc::imx8mp::regs::pwm;
use crate::ylog;

/// The longest period in counter clocks (fact 2).
pub const MAX_COUNTS: u32 = 0xffff;
/// The largest prescaler divide (`PRESCALER` = 0xFFF, "Divide by 4096").
pub const MAX_PRESCALE: u32 = (pwm::pwmcr::PRESCALER_MASK >> pwm::pwmcr::PRESCALER_SHIFT) + 1;
/// Zephyr's `PWM_POLARITY_INVERTED` (include/zephyr/dt-bindings/pwm/pwm.h).
pub const POLARITY_INVERTED: u32 = 1 << 0;
/// `CLKSRC` = ipg_clk_highfreq (fact 7).
const CLKSRC_HIGHFREQ: u32 = 0b10;
/// Polls of `PWMCR.SWR`, 1 us apart, before a reset counts as stuck.
pub const SWR_POLLS: u32 = 1000;

/// What to program for one period/pulse request, in counter clocks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Plan {
    /// Clock divide, 1..=4096 (`PRESCALER` + 1).
    pub prescale: u32,
    /// Period in counter clocks, 2..=0xFFFF (`PWMPR` + 2).
    pub counts: u32,
    /// High time in counter clocks, 0..=counts (`PWMSAR`).
    pub high: u32,
}

/// Turn Zephyr's (period, pulse, flags) in input-clock cycles into a plan.
/// An inverted pulse is the complement of a normal one: the output is low for
/// `pulse` and high for the rest, and the block always runs with `POUTC` 0b00.
pub fn plan(period: u32, pulse: u32, flags: u32) -> Result<Plan, i32> {
    if period < 2 || pulse > period || flags & !POLARITY_INVERTED != 0 {
        return Err(-EINVAL);
    }
    let prescale = period.div_ceil(MAX_COUNTS);
    if prescale > MAX_PRESCALE {
        return Err(-EINVAL);
    }
    // round to nearest; period <= prescale * MAX_COUNTS keeps it in range
    let counts = (period + prescale / 2) / prescale;
    let mut high = ((u64::from(pulse) * u64::from(counts) + u64::from(period) / 2) / u64::from(period)) as u32;
    if flags & POLARITY_INVERTED != 0 {
        high = counts - high;
    }
    Ok(Plan { prescale, counts, high })
}

/// What [`Pwm::apply`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    /// The output is (now) off: a 0 % request.
    Off,
    /// The block was reset and started with the new settings.
    Started,
    /// The block was running with the same clocking; only the sample (and
    /// the period, if it changed) were written.
    Updated,
}

/// One PWM block at `base`.
pub struct Pwm<'a, M: Mmio> {
    m: &'a M,
    base: usize,
}

impl<'a, M: Mmio> Pwm<'a, M> {
    pub fn new(m: &'a M, base: usize) -> Self {
        Pwm { m, base }
    }

    fn rd(&self, off: u32) -> u32 {
        self.m.read32(self.base + off as usize)
    }

    fn wr(&self, off: u32, v: u32) {
        self.m.write32(self.base + off as usize, v)
    }

    /// The output is running: enabled, with a clock selected.
    pub fn is_enabled(&self) -> bool {
        let cr = self.rd(pwm::PWMCR);
        cr & pwm::pwmcr::EN_MASK != 0 && cr & pwm::pwmcr::CLKSRC_MASK != 0
    }

    /// `PWMCR` for a running block with `prescale`.
    fn control(prescale: u32) -> u32 {
        pwm::pwmcr::EN_MASK
            | (((prescale - 1) << pwm::pwmcr::PRESCALER_SHIFT) & pwm::pwmcr::PRESCALER_MASK)
            | (CLKSRC_HIGHFREQ << pwm::pwmcr::CLKSRC_SHIFT)
            // POUTC 0b00: set at rollover, cleared at comparison (fact 3)
            // Keep running through a debugger halt, WAIT and DOZE: a
            // backlight that goes dark when the CPU idles is a defect.
            | pwm::pwmcr::DBGEN_MASK
            | pwm::pwmcr::WAITEN_MASK
            | pwm::pwmcr::DOZEN_MASK
    }

    /// Program `p` (or switch off for a 0 % plan).
    ///
    /// A running block whose clocking (prescaler, clock source, output mode)
    /// already matches is updated through the FIFO, which changes the duty
    /// cycle from the next period without a gap (a changed `PWMPR` restarts
    /// the period, fact 1 note); with the FIFO full that is
    /// `-EBUSY` and nothing is written. Anything else goes through a software
    /// reset: SWR, wait for it to clear (`-ETIMEDOUT` after [`SWR_POLLS`]
    /// us), sample, period, then `PWMCR` with EN last.
    ///
    /// `restart` forces the reset path even when the block looks right: the
    /// caller changed the input clock under it.
    pub fn apply(&self, env: &impl Env, p: Plan, restart: bool) -> Result<Applied, i32> {
        let cr = self.rd(pwm::PWMCR);
        if p.high == 0 {
            // fact 4: disabled is low
            if cr & pwm::pwmcr::EN_MASK != 0 {
                self.wr(pwm::PWMCR, cr & !pwm::pwmcr::EN_MASK);
            }
            return Ok(Applied::Off);
        }
        if p.prescale == 0 || p.prescale > MAX_PRESCALE || p.counts < 2 || p.counts > MAX_COUNTS || p.high > p.counts {
            return Err(-EINVAL);
        }
        let want = Self::control(p.prescale);
        let period_reg = p.counts - 2; // fact 1
        let fields = pwm::pwmcr::EN_MASK
            | pwm::pwmcr::PRESCALER_MASK
            | pwm::pwmcr::CLKSRC_MASK
            | pwm::pwmcr::POUTC_MASK
            | pwm::pwmcr::REPEAT_MASK;
        if !restart && cr & fields == want & fields {
            let avail = (self.rd(pwm::PWMSR) & pwm::pwmsr::FIFOAV_MASK) >> pwm::pwmsr::FIFOAV_SHIFT;
            if avail >= 4 {
                return Err(-EBUSY); // fact 6: a fifth word would be lost
            }
            self.wr(pwm::PWMSAR, p.high & pwm::pwmsar::SAMPLE_MASK);
            if self.rd(pwm::PWMPR) & pwm::pwmpr::PERIOD_MASK != period_reg {
                self.wr(pwm::PWMPR, period_reg);
            }
            return Ok(Applied::Updated);
        }

        self.wr(pwm::PWMCR, pwm::pwmcr::SWR_MASK); // fact 5
        let mut polls = 0;
        while self.rd(pwm::PWMCR) & pwm::pwmcr::SWR_MASK != 0 {
            polls += 1;
            if polls >= SWR_POLLS {
                ylog!(env, Err, "imx_pwm: software reset did not complete in {} us", SWR_POLLS);
                return Err(-ETIMEDOUT);
            }
            env.busy_wait_us(1);
        }
        self.wr(pwm::PWMSAR, p.high & pwm::pwmsar::SAMPLE_MASK);
        self.wr(pwm::PWMPR, period_reg);
        self.wr(pwm::PWMCR, want);
        Ok(Applied::Started)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Access, Faults, Model, TestEnv, PWM};

    const CR_RUN_P3: u32 = 0x01c2_0021; // EN, PRESCALER 2, CLKSRC 0b10, DBGEN|WAITEN|DOZEN

    #[test]
    fn plan_for_the_verdin_backlight() {
        // 6667179 ns at 24 MHz = 160012 cycles (pwm_set_dt's rounding down),
        // 100 % as the board's devicetree asks
        let p = plan(160_012, 160_012, 0).unwrap();
        assert_eq!(p, Plan { prescale: 3, counts: 53_337, high: 53_337 });
        let half = plan(160_012, 80_006, 0).unwrap();
        assert_eq!(half.high, 26_669); // 26668.5 rounds up
    }

    #[test]
    fn plan_edges() {
        assert_eq!(plan(2, 1, 0), Ok(Plan { prescale: 1, counts: 2, high: 1 }));
        assert_eq!(plan(0xffff, 0, 0), Ok(Plan { prescale: 1, counts: 0xffff, high: 0 }));
        assert_eq!(plan(0x1_0000, 0x1_0000, 0).unwrap(), Plan { prescale: 2, counts: 0x8000, high: 0x8000 });
        let longest = MAX_PRESCALE * MAX_COUNTS;
        assert_eq!(plan(longest, 1, 0).unwrap().counts, MAX_COUNTS);
        assert_eq!(plan(longest + 1, 1, 0), Err(-EINVAL));
        assert_eq!(plan(1, 0, 0), Err(-EINVAL));
        assert_eq!(plan(100, 101, 0), Err(-EINVAL));
        assert_eq!(plan(100, 10, 1 << 1), Err(-EINVAL));
    }

    #[test]
    fn inverted_is_the_complement() {
        assert_eq!(plan(1000, 0, POLARITY_INVERTED).unwrap().high, 1000);
        assert_eq!(plan(1000, 1000, POLARITY_INVERTED).unwrap().high, 0);
        assert_eq!(plan(1000, 250, POLARITY_INVERTED).unwrap().high, 750);
    }

    #[test]
    fn a_stopped_block_is_reset_then_sample_period_control_in_that_order() {
        let m = Model::new();
        let e = TestEnv::default();
        let pwm = Pwm::new(&m, m.base(PWM));
        assert!(!pwm.is_enabled());
        let p = plan(160_012, 160_012, 0).unwrap();
        assert_eq!(pwm.apply(&e, p, false), Ok(Applied::Started));
        assert_eq!(m.writes(), vec![(PWM, 0x0, 0x8), (PWM, 0xc, 53_337), (PWM, 0x10, 53_335), (PWM, 0x0, CR_RUN_P3)]);
        // SWR was polled until it cleared, and nothing was written before that
        let a = m.accesses();
        let reset = a.iter().position(|x| *x == Access::W(PWM, 0, 0x8)).unwrap();
        assert_eq!(a[reset + 1], Access::R(PWM, 0, 0x8)); // still in reset
        assert_eq!(a[reset + 2], Access::R(PWM, 0, 0)); // cleared
        assert_eq!(a[reset + 3], Access::W(PWM, 0xc, 53_337));
        assert_eq!(*e.waited_us.borrow(), 1);
        assert!(pwm.is_enabled());
    }

    #[test]
    fn a_running_block_with_the_same_clocking_is_updated_through_the_fifo() {
        let m = Model::new();
        let e = TestEnv::default();
        m.poke(PWM, 0x0, CR_RUN_P3);
        m.poke(PWM, 0x10, 53_335);
        m.poke(PWM, 0x4, 1);
        let pwm = Pwm::new(&m, m.base(PWM));
        assert_eq!(pwm.apply(&e, plan(160_012, 80_006, 0).unwrap(), false), Ok(Applied::Updated));
        assert_eq!(m.writes(), vec![(PWM, 0xc, 26_669)]);
        // a new period on the same prescaler is written after the sample
        m.clear_log();
        assert_eq!(pwm.apply(&e, plan(150_000, 75_000, 0).unwrap(), false), Ok(Applied::Updated));
        assert_eq!(m.writes(), vec![(PWM, 0xc, 25_000), (PWM, 0x10, 49_998)]);
    }

    #[test]
    fn restart_resets_even_a_matching_block() {
        let m = Model::new();
        let e = TestEnv::default();
        m.poke(PWM, 0x0, CR_RUN_P3);
        let pwm = Pwm::new(&m, m.base(PWM));
        assert_eq!(pwm.apply(&e, plan(160_012, 160_012, 0).unwrap(), true), Ok(Applied::Started));
        assert_eq!(m.writes()[0], (PWM, 0x0, 0x8));
    }

    #[test]
    fn a_full_fifo_is_busy_and_nothing_is_written() {
        let m = Model::new();
        let e = TestEnv::default();
        m.poke(PWM, 0x0, CR_RUN_P3);
        m.poke(PWM, 0x4, 4);
        let pwm = Pwm::new(&m, m.base(PWM));
        assert_eq!(pwm.apply(&e, plan(160_012, 1000, 0).unwrap(), false), Err(-EBUSY));
        assert!(m.writes().is_empty());
    }

    #[test]
    fn other_clocking_restarts_through_a_reset() {
        let m = Model::new();
        let e = TestEnv::default();
        // running from ipg_clk (CLKSRC 0b01), as some other owner left it
        m.poke(PWM, 0x0, 0x0001_0001);
        let pwm = Pwm::new(&m, m.base(PWM));
        assert_eq!(pwm.apply(&e, plan(1000, 500, 0).unwrap(), false), Ok(Applied::Started));
        assert_eq!(m.writes()[0], (PWM, 0x0, 0x8));
        assert_eq!(*m.writes().last().unwrap(), (PWM, 0x0, 0x01c2_0001));
    }

    #[test]
    fn a_stuck_reset_times_out_and_never_enables() {
        let m = Model::with_faults(Faults { pwm_swr_stuck: true, ..Default::default() });
        let e = TestEnv::default();
        let pwm = Pwm::new(&m, m.base(PWM));
        assert_eq!(pwm.apply(&e, plan(1000, 500, 0).unwrap(), false), Err(-ETIMEDOUT));
        assert_eq!(m.writes(), vec![(PWM, 0x0, 0x8)]);
        assert_eq!(*e.waited_us.borrow(), u64::from(SWR_POLLS - 1));
        assert!(e.lines.borrow()[0].1.contains("software reset"));
    }

    #[test]
    fn zero_percent_disables_and_a_disabled_block_is_not_written() {
        let m = Model::new();
        let e = TestEnv::default();
        m.poke(PWM, 0x0, CR_RUN_P3);
        let pwm = Pwm::new(&m, m.base(PWM));
        assert_eq!(pwm.apply(&e, plan(1000, 0, 0).unwrap(), false), Ok(Applied::Off));
        assert_eq!(m.writes(), vec![(PWM, 0x0, CR_RUN_P3 & !1)]);
        assert!(!pwm.is_enabled());
        m.clear_log();
        assert_eq!(pwm.apply(&e, plan(1000, 0, 0).unwrap(), false), Ok(Applied::Off));
        assert!(m.writes().is_empty());
    }

    #[test]
    fn enabled_without_a_clock_is_not_enabled() {
        let m = Model::new();
        m.poke(PWM, 0x0, 0x1); // EN with CLKSRC "clock is off"
        assert!(!Pwm::new(&m, m.base(PWM)).is_enabled());
    }

    #[test]
    fn a_plan_out_of_range_is_refused_without_access() {
        let m = Model::new();
        let e = TestEnv::default();
        let pwm = Pwm::new(&m, m.base(PWM));
        let bad = Plan { prescale: 1, counts: 0x1_0000, high: 1 };
        assert_eq!(pwm.apply(&e, bad, false), Err(-EINVAL));
        assert!(m.writes().is_empty());
    }
}
