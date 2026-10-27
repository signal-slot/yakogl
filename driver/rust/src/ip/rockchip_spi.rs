// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Rockchip SPI controller as a polled host: a short command, then a
//! short receive, under one native chip select, as the earlier stage left
//! the pins and the clocks. The NanoPC-T6 reads its PMIC (the RK806 on
//! SPI2) with it (driver/docs/nanopc-t6-mali.md, "Z1").
//!
//! Measured on the T6 (2026-10-19, t6_gpu at 2b776ed3f): this sequence
//! reads the RK806 registers Linux reads (`POWER_EN0` 0x0f, `BUCK1_ON_VSEL`
//! 0x1c); the same command and one more frame in a single
//! transmit-and-receive transfer read 0x00 for every register. So there is
//! no full-duplex transfer here.
//!
//! Facts from Linux v6.18 drivers/spi/spi-rockchip.c (GPL-2.0: facts and
//! file:line only, nothing transcribed):
//!
//! 1. Registers (:26-46): `CTRLR0` 0x00, `CTRLR1` 0x04 (frames - 1),
//!    `SSIENR` 0x08 (1 enables), `SER` 0x0c (bit n = chip select n),
//!    `BAUDR` 0x10 (an even divider of the controller clock), `TXFLR` 0x1c,
//!    `RXFLR` 0x20 (FIFO levels), `SR` 0x24 (bit 0 busy), `IMR` 0x2c,
//!    `ICR` 0x38, `DMACR` 0x3c, `VERSION` 0x48, `TXDR` 0x400, `RXDR` 0x800.
//! 2. `CTRLR0` for a transfer (`rockchip_spi_config`, :531-615): the frame
//!    format SPI, `BHT` 8 bit (bit 13), `SSD` one cycle (bit 10), `EM` big
//!    endian (bit 11), data frame size 8 bit (`DFS` 1), the SPI mode in
//!    bits 7:6, transmit-and-receive (`XFM` 0 in bits 19:18), host mode;
//!    `CTRLR1` = length - 1; `BAUDR` = 2 · ceil(clock / (2 · speed)).
//!    `CTRLR0` is written only with the controller disabled (:592-596).
//! 3. A transfer (`rockchip_spi_set_cs`, :239-280,
//!    `rockchip_spi_prepare_irq`, :382-405): chip select set in `SER`,
//!    `ICR` written with all ones, the controller enabled, the transmit FIFO
//!    filled; the receive FIFO is read as `RXFLR` says.
//!
//! What this does differently: it polls `RXFLR` instead of taking the
//! interrupt, so it refuses a transfer longer than [`MAX_LEN`] (well inside
//! any FIFO the driver detects, `ROCKCHIP_SPI_VER2_TYPE*` 64), and it masks
//! the controller's interrupts (`IMR` 0).

use crate::mmio::Mmio;

pub const CTRLR0: usize = 0x00;
pub const CTRLR1: usize = 0x04;
pub const SSIENR: usize = 0x08;
pub const SER: usize = 0x0c;
pub const BAUDR: usize = 0x10;
pub const TXFLR: usize = 0x1c;
pub const RXFLR: usize = 0x20;
pub const SR: usize = 0x24;
pub const IMR: usize = 0x2c;
pub const ICR: usize = 0x38;
pub const DMACR: usize = 0x3c;
pub const VERSION: usize = 0x48;
pub const TXDR: usize = 0x400;
pub const RXDR: usize = 0x800;

pub const SR_BUSY: u32 = 1 << 0;

/// `CTRLR0` for 8-bit frames, transmit and receive, mode 0 (fact 2).
pub const CTRLR0_8BIT_TR_MODE0: u32 = (1 << 13) | (1 << 11) | (1 << 10) | 1;

/// The longest transfer this polls through the FIFO in one go.
pub const MAX_LEN: usize = 16;

/// Why [`transfer`] refused. The value crosses to C, one number per cause.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum SpiRefusal {
    /// Empty, longer than [`MAX_LEN`], `tx` and `rx` of different lengths,
    /// a chip select above 1, or an odd or zero divider.
    BadArgument = 1,
    /// The controller read busy before anything was written: someone else
    /// is using it.
    Busy = 2,
    /// The receive FIFO did not fill within the budget.
    RxTimeout = 3,
    /// The controller stayed busy after the last frame.
    IdleTimeout = 4,
}

/// The `BAUDR` divider for at most `max_hz` from a controller clock of
/// `clk_hz` (fact 2): even, at least 2.
pub const fn baudr_for(clk_hz: u32, max_hz: u32) -> u32 {
    let d = 2 * clk_hz.div_ceil(2 * max_hz);
    if d < 2 {
        2
    } else {
        d
    }
}

/// A controller and the device on one of its chip selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Host {
    /// The controller's mapped base.
    pub base: usize,
    /// The native chip select, 0 or 1.
    pub cs: u32,
    /// `BAUDR` ([`baudr_for`]).
    pub baudr: u32,
    /// The longest each wait polls, in µs.
    pub budget_us: u32,
}

impl Host {
    /// The same device on the controller mapped at `base`.
    pub const fn at(self, base: usize) -> Host {
        Host { base, ..self }
    }
}

/// `CTRLR0.XFM` receive only (bits 19:18 = 2, fact 2).
pub const CTRLR0_XFM_RO: u32 = 2 << 18;

/// Linux's `spi_write_then_read` on this controller (spi.c:4707-4757 and
/// fact 3): one message, the chip select held across two transfers, `tx`
/// sent transmit-and-receive with what comes back dropped, then `rx.len()`
/// frames received in receive-only mode (`XFM` 2; the controller clocks
/// them out on its own, nothing is written to `TXDR`).
pub fn write_then_read(
    m: &impl Mmio,
    h: &Host,
    tx: &[u8],
    rx: &mut [u8],
    delay_us: &mut impl FnMut(u32),
) -> Result<(), (SpiRefusal, u32)> {
    check(m, h, tx.len(), !rx.is_empty() && rx.len() <= MAX_LEN)?;
    m.write32(h.base + SSIENR, 0);
    m.write32(h.base + SER, 1 << h.cs);
    let mut drop = [0u8; MAX_LEN];
    let mut r = phase(m, h, Some(tx), &mut drop[..tx.len()], delay_us);
    if r.is_ok() {
        r = phase(m, h, None, rx, delay_us);
    }
    release(m, h.base);
    r
}

/// The arguments and the controller's state, before anything is written.
fn check(m: &impl Mmio, h: &Host, n: usize, rx_ok: bool) -> Result<(), (SpiRefusal, u32)> {
    if n == 0 || n > MAX_LEN || !rx_ok || h.cs > 1 || h.baudr < 2 || !h.baudr.is_multiple_of(2) {
        return Err((SpiRefusal::BadArgument, 0));
    }
    let sr = m.read32(h.base + SR);
    if m.read32(h.base + SSIENR) & 1 != 0 && sr & SR_BUSY != 0 {
        return Err((SpiRefusal::Busy, sr));
    }
    Ok(())
}

/// One transfer with the chip select already set: transmit-and-receive of
/// `tx`, or, with `tx` `None`, receive-only of `rx.len()` frames. The
/// controller is disabled again at its end.
fn phase(
    m: &impl Mmio,
    h: &Host,
    tx: Option<&[u8]>,
    rx: &mut [u8],
    delay_us: &mut impl FnMut(u32),
) -> Result<(), (SpiRefusal, u32)> {
    let Host { base, baudr, budget_us, .. } = *h;
    let n = rx.len();
    m.write32(base + SSIENR, 0);
    m.write32(base + CTRLR0, CTRLR0_8BIT_TR_MODE0 | if tx.is_none() { CTRLR0_XFM_RO } else { 0 });
    m.write32(base + CTRLR1, (n - 1) as u32);
    m.write32(base + BAUDR, baudr);
    m.write32(base + DMACR, 0);
    m.write32(base + IMR, 0);
    m.write32(base + ICR, 0xffff_ffff);
    m.write32(base + SSIENR, 1);
    for &b in tx.unwrap_or(&[]) {
        m.write32(base + TXDR, u32::from(b));
    }
    let mut got = 0;
    let mut waited = 0;
    while got < n {
        let level = m.read32(base + RXFLR) as usize;
        for _ in 0..level.min(n - got) {
            rx[got] = m.read32(base + RXDR) as u8;
            got += 1;
        }
        if got < n {
            if waited >= budget_us {
                m.write32(base + SSIENR, 0);
                return Err((SpiRefusal::RxTimeout, got as u32));
            }
            delay_us(1);
            waited += 1;
        }
    }
    // the last frame is in; the line goes idle after it
    let mut idle_wait = 0;
    loop {
        let sr = m.read32(base + SR);
        if sr & SR_BUSY == 0 {
            break;
        }
        if idle_wait >= budget_us {
            m.write32(base + SSIENR, 0);
            return Err((SpiRefusal::IdleTimeout, sr));
        }
        delay_us(1);
        idle_wait += 1;
    }
    m.write32(base + SSIENR, 0);
    Ok(())
}

/// The controller disabled, no chip select: how Linux leaves it between
/// transfers.
fn release(m: &impl Mmio, base: usize) {
    m.write32(base + SSIENR, 0);
    m.write32(base + SER, 0);
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    pub const BASE: usize = 0xfeb2_0000;

    /// A controller with a device on CS0 that answers each frame with what
    /// `answer` gives for the frames so far; the frames arrive in the
    /// receive FIFO `lag` polls after they are written.
    pub type Answer = Box<dyn Fn(&[u8]) -> u8>;

    pub struct Spi {
        pub regs: RefCell<std::collections::HashMap<usize, u32>>,
        pub ops: RefCell<Vec<(char, usize, u32)>>,
        pub sent: RefCell<Vec<u8>>,
        pub rxq: RefCell<VecDeque<u8>>,
        pub answer: Answer,
        /// Frames reach the receive FIFO only when this is false.
        pub deaf: bool,
    }

    impl Spi {
        pub fn new(answer: Answer) -> Spi {
            Spi {
                regs: RefCell::new([(SSIENR, 0), (SR, 0x0c), (SER, 0)].into_iter().collect()),
                ops: RefCell::new(vec![]),
                sent: RefCell::new(vec![]),
                rxq: RefCell::new(VecDeque::new()),
                answer,
                deaf: false,
            }
        }
    }

    impl Mmio for Spi {
        fn read32(&self, addr: usize) -> u32 {
            let off = addr - BASE;
            let v = match off {
                RXFLR => self.rxq.borrow().len() as u32,
                RXDR => u32::from(self.rxq.borrow_mut().pop_front().expect("read of an empty RX FIFO")),
                _ => *self.regs.borrow().get(&off).unwrap_or(&0),
            };
            self.ops.borrow_mut().push(('r', off, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            let off = addr - BASE;
            self.ops.borrow_mut().push(('w', off, val));
            if off == TXDR {
                assert_eq!(self.regs.borrow()[&SSIENR], 1, "TXDR written with the controller disabled");
                assert_eq!(self.regs.borrow()[&SER], 1, "TXDR written with no chip select");
                self.sent.borrow_mut().push(val as u8);
                let a = (self.answer)(&self.sent.borrow());
                if !self.deaf {
                    self.rxq.borrow_mut().push_back(a);
                }
                return;
            }
            if off == CTRLR0 || off == CTRLR1 || off == BAUDR {
                assert_eq!(self.regs.borrow()[&SSIENR], 0, "{off:#x} written with the controller enabled");
            }
            self.regs.borrow_mut().insert(off, val);
            // receive only: enabling the controller clocks CTRLR1 + 1 frames
            // in, MOSI idle (0 here)
            if off == SSIENR && val == 1 && self.regs.borrow()[&CTRLR0] & CTRLR0_XFM_RO != 0 {
                assert_eq!(self.regs.borrow()[&SER], 1, "receive-only enabled with no chip select");
                for _ in 0..=self.regs.borrow()[&CTRLR1] {
                    self.sent.borrow_mut().push(0);
                    let a = (self.answer)(&self.sent.borrow());
                    if !self.deaf {
                        self.rxq.borrow_mut().push_back(a);
                    }
                }
            }
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!()
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!()
        }
    }

    #[test]
    fn baudr_is_even_and_never_faster_than_asked() {
        // the T6's clk_spi2 198 MHz, the RK806's spi-max-frequency 1 MHz
        assert_eq!(baudr_for(198_000_000, 1_000_000), 198);
        assert_eq!(baudr_for(200_000_000, 1_000_000), 200);
        assert_eq!(baudr_for(150_000_000, 1_000_000), 150);
        assert_eq!(baudr_for(24_000_000, 1_000_000), 24);
        assert_eq!(baudr_for(1_000_000, 1_000_000), 2);
        assert_eq!(baudr_for(201_000_000, 1_000_000), 202);
    }

    #[test]
    fn write_then_read_holds_the_chip_select_across_a_transmit_and_a_receive_only_phase() {
        // a device that answers 0x1c to the frame after a 3-byte command
        let s = Spi::new(Box::new(|sent| if sent.len() == 4 { 0x1c } else { 0xee }));
        let mut rx = [0u8; 1];
        let h = Host { base: BASE, cs: 0, baudr: 198, budget_us: 100 };
        write_then_read(&s, &h, &[0x00, 0x1a, 0x00], &mut rx, &mut |_| {}).unwrap();
        assert_eq!(rx, [0x1c]);
        let ctl: Vec<_> = s
            .ops
            .borrow()
            .iter()
            .filter(|o| o.0 == 'w' && matches!(o.1, SER | CTRLR0 | CTRLR1 | SSIENR))
            .map(|o| (o.1, o.2))
            .collect();
        assert_eq!(
            ctl,
            vec![
                (SSIENR, 0),
                (SER, 1),
                (SSIENR, 0),
                (CTRLR0, 0x2c01),
                (CTRLR1, 2),
                (SSIENR, 1),
                (SSIENR, 0),
                (SSIENR, 0),
                (CTRLR0, 0x2c01 | CTRLR0_XFM_RO),
                (CTRLR1, 0),
                (SSIENR, 1),
                (SSIENR, 0),
                (SSIENR, 0),
                (SER, 0),
            ]
        );
        // nothing written to TXDR in the receive phase
        assert_eq!(s.ops.borrow().iter().filter(|o| o.0 == 'w' && o.1 == TXDR).count(), 3);
    }

    #[test]
    fn a_device_that_never_answers_times_out_and_releases_the_bus() {
        let mut s = Spi::new(Box::new(|_| 0));
        s.deaf = true;
        let mut rx = [0u8; 2];
        let mut slept = 0;
        assert_eq!(
            write_then_read(&s, &Host { base: BASE, cs: 0, baudr: 200, budget_us: 50 }, &[1, 2], &mut rx, &mut |us| {
                slept += us
            }),
            Err((SpiRefusal::RxTimeout, 0))
        );
        assert_eq!(slept, 50);
        assert_eq!(s.regs.borrow()[&SSIENR], 0);
        assert_eq!(s.regs.borrow()[&SER], 0);
    }

    #[test]
    fn a_busy_controller_and_bad_arguments_are_refused_before_any_write() {
        let s = Spi::new(Box::new(|_| 0));
        s.regs.borrow_mut().insert(SSIENR, 1);
        s.regs.borrow_mut().insert(SR, 0x1);
        let mut rx = [0u8; 1];
        assert_eq!(
            write_then_read(&s, &Host { base: BASE, cs: 0, baudr: 200, budget_us: 10 }, &[1], &mut rx, &mut |_| {}),
            Err((SpiRefusal::Busy, 1))
        );
        let s = Spi::new(Box::new(|_| 0));
        let mut big = [0u8; MAX_LEN + 1];
        assert_eq!(
            write_then_read(
                &s,
                &Host { base: BASE, cs: 0, baudr: 200, budget_us: 10 },
                &[0; MAX_LEN + 1],
                &mut big,
                &mut |_| {}
            )
            .unwrap_err()
            .0,
            SpiRefusal::BadArgument
        );
        assert_eq!(
            write_then_read(&s, &Host { base: BASE, cs: 2, baudr: 200, budget_us: 10 }, &[1], &mut rx, &mut |_| {})
                .unwrap_err()
                .0,
            SpiRefusal::BadArgument
        );
        assert_eq!(
            write_then_read(&s, &Host { base: BASE, cs: 0, baudr: 199, budget_us: 10 }, &[1], &mut rx, &mut |_| {})
                .unwrap_err()
                .0,
            SpiRefusal::BadArgument
        );
        assert_eq!(
            write_then_read(&s, &Host { base: BASE, cs: 0, baudr: 200, budget_us: 10 }, &[1, 2], &mut [], &mut |_| {})
                .unwrap_err()
                .0,
            SpiRefusal::BadArgument
        );
        assert!(s.ops.borrow().iter().all(|o| o.0 == 'r'));
    }
}
