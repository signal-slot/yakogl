// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The STM32 I2C v2 controller (`st,stm32mp25-i2c`) as a polled master, for
//! the ADV7535 on the STM32MP257F-DK (ST's stm32mp257f-dk.dts :328-350).
//! Zephyr has no I2C driver for the MP2's A35, so this is the driver. The bus,
//! its base address, the clock, the reset and the pins are the caller's.
//!
//! Facts from Linux v6.18 drivers/i2c/busses/i2c-stm32f7.c (GPL-2.0: facts
//! and file:line only, nothing transcribed):
//!
//! 1. Registers (:50-51 and the defines above them): `CR1` 0x00, `CR2` 0x04,
//!    `TIMINGR` 0x10, `ISR` 0x18, `ICR` 0x1c, `RXDR` 0x24, `TXDR` 0x28
//!    (accessed as bytes, :793, :804). `CR1`: PE bit 0, DNF bits 11:8, ANFOFF
//!    bit 12. `CR2`: SADD7 bits 7:1, RD_WRN 10, START 13, STOP 14, NBYTES
//!    23:16, RELOAD 24, AUTOEND 25. `ISR`: TXE 0, TXIS 1, RXNE 2, NACKF 4,
//!    STOPF 5, TC 6, BERR 8, ARLO 9, BUSY 15. `TIMINGR`: SCLL 7:0, SCLH
//!    15:8, SDADEL 19:16, SCLDEL 23:20, PRESC 31:28.
//! 2. `stm32f7_i2c_hw_config` (:756-784): TIMINGR first, then `CR1.ANFOFF`
//!    (set when the analog filter is off), then `CR1.DNF` (cleared, then
//!    set), `CR1.PE` last. [`init`] does that, with PE cleared before TIMINGR
//!    (TIMINGR may be written only with PE low, RM0457).
//! 3. `stm32f7_i2c_compute_timing` (:461-650): the spec table
//!    (`stm32f7_get_specs`: 100 kHz, 400 kHz, 1 MHz rows as in [`SPECS`]),
//!    analog filter 50..260 ns (:172-173) when on, DNF = round(DT filter
//!    width / kernel clock period) <= 15 (:496), sdadel/scldel limits
//!    (:517-527), the first (SCLDEL, SDADEL) per prescaler (:534-562), then
//!    SCLL/SCLH inside `clk_min = 1e9/speed` .. `clk_max = 1e9/(0.8 speed)`
//!    (:577-578, `RATE_MIN` :460), the least `|tscl - 1e9/speed|` strictly
//!    below the first candidate bound `i2cbus` winning (:471, :615). Times
//!    are integer ns with the clock period `DIV_ROUND_CLOSEST`ed (:465).
//! 4. `stm32f7_i2c_xfer_msg` (:883): `CR2` gets SADD, RD_WRN, NBYTES (255 at
//!    most per segment, longer ones use RELOAD, not done here), AUTOEND for
//!    the last message, START last. The write-read is two messages: the
//!    write without AUTOEND (ends in TC), then a read with START again (the
//!    restart) and AUTOEND.
//! 5. Errors: NACKF cleared with `ICR.NACKCF` and -ENXIO (:1627-1632), ARLO
//!    -EAGAIN and BERR -EIO (`stm32f7_i2c_handle_isr_errs` :1519-1540); any
//!    error flushes TXDR by writing `ISR.TXE` (:1745-1750). The crate has no
//!    ENXIO: a NACK is [`crate::errno::ENODEV`] here. A busy bus gives
//!    EBUSY, a poll timeout ETIMEDOUT.
//!
//! Kernel clock on the DK: ST's stm32mp251.dtsi gives each I2C node a kernel
//! clock from the RCC flexgen; which rate ST's TF-A/U-Boot leave it at is
//! not known to this item (no ST tree was readable), so the host tests use
//! 64 MHz (HSI) and say so. The caller passes the real rate.

use crate::errno;
use crate::mmio::{field, Mmio};

pub const CR1: usize = 0x00;
pub const CR2: usize = 0x04;
pub const TIMINGR: usize = 0x10;
pub const ISR: usize = 0x18;
pub const ICR: usize = 0x1c;
pub const RXDR: usize = 0x24;
pub const TXDR: usize = 0x28;

pub const CR1_PE: u32 = 1 << 0;
pub const CR1_DNF: u32 = 0xf << 8;
pub const CR1_ANFOFF: u32 = 1 << 12;

pub const CR2_SADD7: u32 = 0x7f << 1;
pub const CR2_RD_WRN: u32 = 1 << 10;
pub const CR2_START: u32 = 1 << 13;
pub const CR2_STOP: u32 = 1 << 14;
pub const CR2_NBYTES: u32 = 0xff << 16;
pub const CR2_AUTOEND: u32 = 1 << 25;

pub const ISR_TXE: u32 = 1 << 0;
pub const ISR_TXIS: u32 = 1 << 1;
pub const ISR_RXNE: u32 = 1 << 2;
pub const ISR_NACKF: u32 = 1 << 4;
pub const ISR_STOPF: u32 = 1 << 5;
pub const ISR_TC: u32 = 1 << 6;
pub const ISR_BERR: u32 = 1 << 8;
pub const ISR_ARLO: u32 = 1 << 9;
pub const ISR_BUSY: u32 = 1 << 15;

pub const ICR_NACKCF: u32 = 1 << 4;
pub const ICR_STOPCF: u32 = 1 << 5;
pub const ICR_BERRCF: u32 = 1 << 8;
pub const ICR_ARLOCF: u32 = 1 << 9;

pub const TIMINGR_SCLL: u32 = 0xff;
pub const TIMINGR_SCLH: u32 = 0xff << 8;
pub const TIMINGR_SDADEL: u32 = 0xf << 16;
pub const TIMINGR_SCLDEL: u32 = 0xf << 20;
pub const TIMINGR_PRESC: u32 = 0xf << 28;

/// The most bytes one message carries (NBYTES is 8 bits; no RELOAD here).
pub const MAX_LEN: usize = 255;

/// How many times a status bit is polled before a timeout. The caller's
/// transfer stays well inside this at 100 kHz and the 64 MHz or faster
/// register access of the A35.
pub const POLL_LIMIT: u32 = 2_000_000;

/// Bus parameters of the DT node (`clock-frequency`, `i2c-scl-rising-time-ns`,
/// `i2c-scl-falling-time-ns`, `i2c-analog-filter`,
/// `i2c-digital-filter-width-ns`) and the kernel clock.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Config {
    /// The kernel clock rate in Hz (`i2c_clk`).
    pub clock_hz: u32,
    /// 100_000, 400_000 or 1_000_000.
    pub speed_hz: u32,
    pub rise_ns: u32,
    pub fall_ns: u32,
    pub analog_filter: bool,
    /// 0 for no digital filter.
    pub dnf_ns: u32,
}

impl Config {
    /// The Linux defaults for `speed_hz`: rise 25 ns, fall 10 ns, analog
    /// filter on, no digital filter.
    pub const fn new(clock_hz: u32, speed_hz: u32) -> Config {
        Config { clock_hz, speed_hz, rise_ns: 25, fall_ns: 10, analog_filter: true, dnf_ns: 0 }
    }
}

/// The `TIMINGR` fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timing {
    pub presc: u8,
    pub scldel: u8,
    pub sdadel: u8,
    pub sclh: u8,
    pub scll: u8,
    pub dnf: u8,
}

impl Timing {
    pub fn timingr(&self) -> u32 {
        field(TIMINGR_PRESC, u32::from(self.presc))
            | field(TIMINGR_SCLDEL, u32::from(self.scldel))
            | field(TIMINGR_SDADEL, u32::from(self.sdadel))
            | field(TIMINGR_SCLH, u32::from(self.sclh))
            | field(TIMINGR_SCLL, u32::from(self.scll))
    }
}

/// One row of Linux's `i2c_specs`: all times in ns.
struct Spec {
    rate: u32,
    fall_max: i64,
    rise_max: i64,
    hddat_min: i64,
    vddat_max: i64,
    sudat_min: i64,
    l_min: i64,
    h_min: i64,
}

const SPECS: [Spec; 3] = [
    Spec {
        rate: 100_000,
        fall_max: 300,
        rise_max: 1000,
        hddat_min: 0,
        vddat_max: 3450,
        sudat_min: 250,
        l_min: 4700,
        h_min: 4000,
    },
    Spec {
        rate: 400_000,
        fall_max: 300,
        rise_max: 300,
        hddat_min: 0,
        vddat_max: 900,
        sudat_min: 100,
        l_min: 1300,
        h_min: 600,
    },
    Spec {
        rate: 1_000_000,
        fall_max: 100,
        rise_max: 120,
        hddat_min: 0,
        vddat_max: 450,
        sudat_min: 50,
        l_min: 500,
        h_min: 260,
    },
];

const AF_MIN_NS: i64 = 50;
const AF_MAX_NS: i64 = 260;
const DNF_MAX: i64 = 15;
const NS: i64 = 1_000_000_000;

fn div_closest(n: i64, d: i64) -> i64 {
    (n + d / 2) / d
}

/// `stm32f7_i2c_compute_timing`: the `TIMINGR` fields for `cfg`, or
/// `-EINVAL` when the speed or the rise and fall times are outside the spec
/// or no setting fits.
pub fn compute_timing(cfg: &Config) -> Result<Timing, i32> {
    if cfg.clock_hz == 0 {
        return Err(-errno::EINVAL);
    }
    let spec = SPECS.iter().find(|s| s.rate >= cfg.speed_hz).ok_or(-errno::EINVAL)?;
    if cfg.speed_hz == 0 || i64::from(cfg.rise_ns) > spec.rise_max || i64::from(cfg.fall_ns) > spec.fall_max {
        return Err(-errno::EINVAL);
    }
    let (rise, fall) = (i64::from(cfg.rise_ns), i64::from(cfg.fall_ns));
    let i2cclk = div_closest(NS, i64::from(cfg.clock_hz));
    let i2cbus = div_closest(NS, i64::from(cfg.speed_hz));
    let dnf = div_closest(i64::from(cfg.dnf_ns), i2cclk);
    if dnf > DNF_MAX {
        return Err(-errno::EINVAL);
    }
    let dnf_delay = dnf * i2cclk;
    let (af_min, af_max) = if cfg.analog_filter { (AF_MIN_NS, AF_MAX_NS) } else { (0, 0) };

    let sdadel_min = (spec.hddat_min + fall - af_min - (dnf + 3) * i2cclk).max(0);
    let sdadel_max = (spec.vddat_max - rise - af_max - (dnf + 4) * i2cclk).max(0);
    let scldel_min = rise + spec.sudat_min;

    // One (presc, scldel, sdadel) per prescaler: the first that fits.
    let mut cands: [Option<(u8, u8, u8)>; 16] = [None; 16];
    for p in 0..16i64 {
        'p: for l in 0..16i64 {
            if (l + 1) * (p + 1) * i2cclk < scldel_min {
                continue;
            }
            for a in 0..16i64 {
                let sdadel = (a * (p + 1) + 1) * i2cclk;
                if sdadel >= sdadel_min && sdadel <= sdadel_max {
                    cands[p as usize] = Some((p as u8, l as u8, a as u8));
                    break 'p;
                }
            }
        }
    }

    let tsync = af_min + dnf_delay + 2 * i2cclk;
    let clk_min = NS / i64::from(cfg.speed_hz);
    let clk_max = NS / (i64::from(cfg.speed_hz) * 8 / 10);
    let mut best: Option<Timing> = None;
    let mut best_err = i2cbus;
    for (p, l, a) in cands.iter().flatten().copied() {
        let prescaler = (i64::from(p) + 1) * i2cclk;
        for scll in 0..256i64 {
            let tscl_l = (scll + 1) * prescaler + tsync;
            if tscl_l < spec.l_min || i2cclk >= (tscl_l - af_min - dnf_delay) / 4 {
                continue;
            }
            for sclh in 0..256i64 {
                let tscl_h = (sclh + 1) * prescaler + tsync;
                if tscl_h < spec.h_min || i2cclk >= tscl_h {
                    continue;
                }
                let tscl = tscl_l + tscl_h + rise + fall;
                if tscl < clk_min || tscl > clk_max {
                    continue;
                }
                let err = (tscl - i2cbus).abs();
                if err < best_err {
                    best_err = err;
                    best = Some(Timing {
                        presc: p,
                        scldel: l,
                        sdadel: a,
                        sclh: sclh as u8,
                        scll: scll as u8,
                        dnf: dnf as u8,
                    });
                }
            }
        }
    }
    best.ok_or(-errno::EINVAL)
}

/// Configure the controller (`stm32f7_i2c_hw_config`): PE low, `TIMINGR`,
/// the analog filter, the digital filter, PE high. Returns the timing used.
pub fn init<M: Mmio>(m: &M, base: usize, cfg: &Config) -> Result<Timing, i32> {
    let t = compute_timing(cfg)?;
    m.modify32(base + CR1, CR1_PE, 0);
    m.write32(base + TIMINGR, t.timingr());
    m.modify32(base + CR1, CR1_ANFOFF, if cfg.analog_filter { 0 } else { CR1_ANFOFF });
    m.modify32(base + CR1, CR1_DNF, 0);
    m.modify32(base + CR1, CR1_DNF, field(CR1_DNF, u32::from(t.dnf)));
    m.modify32(base + CR1, CR1_PE, CR1_PE);
    Ok(t)
}

/// Poll `ISR` until `want` is set. A NACK, an arbitration loss or a bus
/// error ends the wait with the errno; the flags are cleared.
fn wait<M: Mmio>(m: &M, base: usize, want: u32) -> Result<(), i32> {
    for _ in 0..POLL_LIMIT {
        let isr = m.read32(base + ISR);
        if isr & ISR_NACKF != 0 {
            m.write32(base + ICR, ICR_NACKCF);
            flush_txdr(m, base);
            // The controller sends the STOP itself after a NACK.
            for _ in 0..POLL_LIMIT {
                if m.read32(base + ISR) & ISR_STOPF != 0 {
                    break;
                }
            }
            m.write32(base + ICR, ICR_STOPCF);
            return Err(-errno::ENODEV);
        }
        if isr & ISR_ARLO != 0 {
            m.write32(base + ICR, ICR_ARLOCF);
            flush_txdr(m, base);
            return Err(-errno::EAGAIN);
        }
        if isr & ISR_BERR != 0 {
            m.write32(base + ICR, ICR_BERRCF);
            flush_txdr(m, base);
            return Err(-errno::EIO);
        }
        if isr & want != 0 {
            return Ok(());
        }
    }
    Err(-errno::ETIMEDOUT)
}

/// Any error flushes TXDR so old data is not sent by the next transfer
/// (i2c-stm32f7.c :1745-1750, writing `ISR.TXE`).
fn flush_txdr<M: Mmio>(m: &M, base: usize) {
    m.write32(base + ISR, ISR_TXE);
}

fn wait_idle<M: Mmio>(m: &M, base: usize) -> Result<(), i32> {
    for _ in 0..POLL_LIMIT {
        if m.read32(base + ISR) & ISR_BUSY == 0 {
            return Ok(());
        }
    }
    Err(-errno::EBUSY)
}

fn start<M: Mmio>(m: &M, base: usize, addr: u8, read: bool, len: usize, autoend: bool) {
    // SADD7 is bits 7:1: `field` places the 7-bit address there.
    let mut cr2 = field(CR2_SADD7, u32::from(addr)) | field(CR2_NBYTES, len as u32) | CR2_START;
    if read {
        cr2 |= CR2_RD_WRN;
    }
    if autoend {
        cr2 |= CR2_AUTOEND;
    }
    m.write32(base + CR2, cr2);
}

fn finish<M: Mmio>(m: &M, base: usize) -> Result<(), i32> {
    wait(m, base, ISR_STOPF)?;
    m.write32(base + ICR, ICR_STOPCF);
    Ok(())
}

fn check(addr: u8, lens: &[usize]) -> Result<(), i32> {
    if addr > 0x7f || lens.iter().any(|&l| l == 0 || l > MAX_LEN) {
        return Err(-errno::EINVAL);
    }
    Ok(())
}

fn send<M: Mmio>(m: &M, base: usize, data: &[u8]) -> Result<(), i32> {
    for &b in data {
        wait(m, base, ISR_TXIS)?;
        m.write8(base + TXDR, b);
    }
    Ok(())
}

/// Write `data` (1..=255 bytes) to the 7-bit address `addr`, START .. STOP.
pub fn write<M: Mmio>(m: &M, base: usize, addr: u8, data: &[u8]) -> Result<(), i32> {
    check(addr, &[data.len()])?;
    wait_idle(m, base)?;
    start(m, base, addr, false, data.len(), true);
    send(m, base, data)?;
    finish(m, base)
}

/// Write `wr` then, after a repeated START, read `rd.len()` bytes; one STOP
/// at the end. Each part is 1..=255 bytes.
pub fn write_read<M: Mmio>(m: &M, base: usize, addr: u8, wr: &[u8], rd: &mut [u8]) -> Result<(), i32> {
    check(addr, &[wr.len(), rd.len()])?;
    wait_idle(m, base)?;
    start(m, base, addr, false, wr.len(), false);
    send(m, base, wr)?;
    wait(m, base, ISR_TC)?;
    start(m, base, addr, true, rd.len(), true);
    for b in rd.iter_mut() {
        wait(m, base, ISR_RXNE)?;
        *b = m.read8(base + RXDR);
    }
    finish(m, base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mmio::get;
    use std::cell::RefCell;
    use std::collections::{HashMap, VecDeque};

    const BASE: usize = 0x4018_0000;
    /// The kernel clock the tests assume (HSI); see the module doc.
    const CLK: u32 = 64_000_000;

    /// A controller with one device at `addr` (or none): ISR follows the
    /// last CR2 START as the reference manual describes.
    struct I2c {
        addr: u8,
        regs: RefCell<HashMap<usize, u32>>,
        /// Every write, (offset, value), in order.
        writes: RefCell<Vec<(usize, u32)>>,
        sent: RefCell<Vec<u8>>,
        rx: RefCell<VecDeque<u8>>,
        left: RefCell<u32>,
        read: RefCell<bool>,
        autoend: RefCell<bool>,
        stop: RefCell<bool>,
        nack: RefCell<bool>,
        active: RefCell<bool>,
    }

    impl I2c {
        fn new(addr: u8, rx: &[u8]) -> I2c {
            I2c {
                addr,
                regs: RefCell::new(HashMap::new()),
                writes: RefCell::new(vec![]),
                sent: RefCell::new(vec![]),
                rx: RefCell::new(rx.iter().copied().collect()),
                left: RefCell::new(0),
                read: RefCell::new(false),
                autoend: RefCell::new(false),
                stop: RefCell::new(false),
                nack: RefCell::new(false),
                active: RefCell::new(false),
            }
        }
    }

    impl Mmio for I2c {
        fn read32(&self, a: usize) -> u32 {
            match a - BASE {
                ISR => {
                    let mut v = 0;
                    if *self.nack.borrow() {
                        v |= ISR_NACKF | ISR_TXE;
                    }
                    if *self.stop.borrow() {
                        v |= ISR_STOPF;
                    }
                    if v != 0 {
                        return v;
                    }
                    if !*self.active.borrow() {
                        return 0;
                    }
                    let left = *self.left.borrow();
                    match (*self.read.borrow(), left) {
                        (false, l) if l > 0 => v |= ISR_TXIS,
                        (true, l) if l > 0 => v |= ISR_RXNE,
                        _ if *self.autoend.borrow() => v |= ISR_STOPF,
                        _ => v |= ISR_TC,
                    }
                    v | ISR_BUSY
                }
                o => *self.regs.borrow().get(&o).unwrap_or(&0),
            }
        }
        fn write32(&self, a: usize, val: u32) {
            let off = a - BASE;
            self.writes.borrow_mut().push((off, val));
            match off {
                CR2 if val & CR2_START != 0 => {
                    assert_eq!(
                        self.regs.borrow().get(&CR1).copied().unwrap_or(0) & CR1_PE,
                        CR1_PE,
                        "START with PE low"
                    );
                    *self.read.borrow_mut() = val & CR2_RD_WRN != 0;
                    *self.autoend.borrow_mut() = val & CR2_AUTOEND != 0;
                    *self.left.borrow_mut() = get(CR2_NBYTES, val);
                    *self.active.borrow_mut() = true;
                    if get(CR2_SADD7, val) as u8 != self.addr {
                        *self.nack.borrow_mut() = true;
                        *self.stop.borrow_mut() = true;
                    }
                }
                ICR => {
                    if val & ICR_NACKCF != 0 {
                        *self.nack.borrow_mut() = false;
                    }
                    if val & ICR_STOPCF != 0 {
                        *self.stop.borrow_mut() = false;
                        *self.active.borrow_mut() = false;
                    }
                }
                _ => {
                    self.regs.borrow_mut().insert(off, val);
                }
            }
        }
        fn read8(&self, a: usize) -> u8 {
            assert_eq!(a - BASE, RXDR);
            *self.left.borrow_mut() -= 1;
            self.rx.borrow_mut().pop_front().expect("RXDR read with nothing queued")
        }
        fn write8(&self, a: usize, val: u8) {
            assert_eq!(a - BASE, TXDR);
            self.writes.borrow_mut().push((TXDR, u32::from(val)));
            assert!(*self.left.borrow() > 0, "TXDR written with no byte due");
            *self.left.borrow_mut() -= 1;
            self.sent.borrow_mut().push(val);
        }
    }

    fn cr2(addr: u8, rd: bool, n: u32, autoend: bool) -> u32 {
        (u32::from(addr) << 1)
            | if rd { CR2_RD_WRN } else { 0 }
            | (n << 16)
            | if autoend { CR2_AUTOEND } else { 0 }
            | CR2_START
    }

    #[test]
    fn timing_400k_64mhz_meets_the_spec() {
        let t = compute_timing(&Config::new(CLK, 400_000)).unwrap();
        // 64 MHz, 400 kHz, analog filter on, rise 25 / fall 10, no DNF.
        // The tuple is what compute_timing gives; the checks below are the
        // spec limits it must satisfy, computed independently.
        assert_eq!(t.dnf, 0);
        let clk = 16.0; // DIV_ROUND_CLOSEST(1e9, 64e6), integer ns as the kernel
        let pre = f64::from(t.presc + 1) * clk;
        let tsync = 50.0 + 2.0 * clk;
        let low = (f64::from(t.scll) + 1.0) * pre + tsync;
        let high = (f64::from(t.sclh) + 1.0) * pre + tsync;
        assert!(low >= 1300.0 && high >= 600.0);
        let period = low + high + 25.0 + 10.0;
        assert!((2500.0..=3125.0).contains(&period), "{period}");
        assert!((f64::from(t.scldel) + 1.0) * pre >= 25.0 + 100.0);
        let sda = (f64::from(t.sdadel) * pre) + clk;
        assert!((0.0..=900.0 - 25.0 - 260.0 - 4.0 * clk).contains(&sda));
        assert_eq!(t.timingr() >> 28, u32::from(t.presc));
        // Pinned: what the algorithm gives for 64 MHz / 400 kHz.
        assert_eq!(t.timingr(), 0x0070424c);
    }

    #[test]
    fn timing_rejects_out_of_spec() {
        let mut c = Config::new(CLK, 400_000);
        c.rise_ns = 400;
        assert_eq!(compute_timing(&c), Err(-errno::EINVAL));
        assert_eq!(compute_timing(&Config::new(CLK, 3_000_000)), Err(-errno::EINVAL));
        let mut c = Config::new(CLK, 100_000);
        c.dnf_ns = 1000;
        assert_eq!(compute_timing(&c), Err(-errno::EINVAL));
    }

    #[test]
    fn timing_100k_fits() {
        let t = compute_timing(&Config::new(CLK, 100_000)).unwrap();
        let clk = 16.0; // DIV_ROUND_CLOSEST(1e9, 64e6), integer ns as the kernel
        let pre = f64::from(t.presc + 1) * clk;
        let tsync = 50.0 + 2.0 * clk;
        let period = (f64::from(t.scll) + 1.0) * pre + (f64::from(t.sclh) + 1.0) * pre + 2.0 * tsync + 35.0;
        assert!((10_000.0..=12_500.0).contains(&period), "{period}");
    }

    #[test]
    fn init_400k_register_sequence() {
        let m = I2c::new(0x3d, &[]);
        let t = init(&m, BASE, &Config::new(CLK, 400_000)).unwrap();
        // PE low (the modify reads 0 and writes 0), TIMINGR, ANFOFF clear
        // (analog filter on), DNF cleared then set to 0, PE high last.
        assert_eq!(
            *m.writes.borrow(),
            vec![(CR1, 0), (TIMINGR, t.timingr()), (CR1, 0), (CR1, 0), (CR1, 0), (CR1, CR1_PE),]
        );
    }

    #[test]
    fn init_analog_filter_off_sets_anfoff() {
        let m = I2c::new(0x3d, &[]);
        let mut c = Config::new(CLK, 400_000);
        c.analog_filter = false;
        init(&m, BASE, &c).unwrap();
        assert_eq!(m.writes.borrow()[2], (CR1, CR1_ANFOFF));
        assert_eq!(m.writes.borrow().last(), Some(&(CR1, CR1_ANFOFF | CR1_PE)));
    }

    fn up() -> I2c {
        let m = I2c::new(0x3d, &[0xaa, 0xbb, 0xcc]);
        init(&m, BASE, &Config::new(CLK, 400_000)).unwrap();
        m.writes.borrow_mut().clear();
        m
    }

    #[test]
    fn write_two_bytes() {
        let m = up();
        write(&m, BASE, 0x3d, &[0x41, 0x10]).unwrap();
        assert_eq!(
            *m.writes.borrow(),
            vec![(CR2, cr2(0x3d, false, 2, true)), (TXDR, 0x41), (TXDR, 0x10), (ICR, ICR_STOPCF)]
        );
        assert_eq!(*m.sent.borrow(), vec![0x41, 0x10]);
    }

    #[test]
    fn write_then_read_restarts() {
        let m = up();
        let mut rd = [0u8; 3];
        write_read(&m, BASE, 0x3d, &[0x94], &mut rd).unwrap();
        assert_eq!(rd, [0xaa, 0xbb, 0xcc]);
        assert_eq!(
            *m.writes.borrow(),
            vec![(CR2, cr2(0x3d, false, 1, false)), (TXDR, 0x94), (CR2, cr2(0x3d, true, 3, true)), (ICR, ICR_STOPCF),]
        );
    }

    #[test]
    fn nack_is_an_errno_and_clears() {
        let m = up();
        assert_eq!(write(&m, BASE, 0x50, &[1, 2]), Err(-errno::ENODEV));
        assert_eq!(
            *m.writes.borrow(),
            vec![(CR2, cr2(0x50, false, 2, true)), (ICR, ICR_NACKCF), (ISR, ISR_TXE), (ICR, ICR_STOPCF)]
        );
        // The bus is usable again.
        write(&m, BASE, 0x3d, &[1]).unwrap();
    }

    #[test]
    fn bad_lengths_and_addresses() {
        let m = up();
        assert_eq!(write(&m, BASE, 0x3d, &[]), Err(-errno::EINVAL));
        assert_eq!(write(&m, BASE, 0x3d, &[0; 256]), Err(-errno::EINVAL));
        assert_eq!(write(&m, BASE, 0x80, &[0]), Err(-errno::EINVAL));
        assert!(m.writes.borrow().is_empty());
        write(&m, BASE, 0x3d, &[0; 255]).unwrap();
    }
}
