// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The transmit side of a 16550 UART with 32-bit registers (reg-shift 2, the
//! DesignWare APB UART of the RK3588) that an earlier stage left configured:
//! polled bytes, nothing configured. For an image's first output, before any
//! driver can run and with the MMU possibly off (board::nanopc_t6's early
//! banner).
//!
//! What it touches, and only this:
//!
//! - `LCR` (0x0c) is read once. With `DLAB` (bit 7) set, offset 0 is the
//!   divisor latch, not `THR`, and a byte written there would change the baud
//!   rate: then nothing is written at all ([`Tx::usable`] is false).
//! - `LSR` (0x14) is read before each byte until `THRE` (bit 5) is set, at most
//!   [`POLL_MAX`] times; a read also clears `LSR`'s error bits, which nothing
//!   here needs. After `POLL_MAX` reads the byte is written anyway and counted
//!   in [`Tx::timeouts`]: a UART that never drains must not hold the image.
//! - `THR` (0x00) is written, one byte per 32-bit store.
//!
//! The register offsets are the 16550's (`RBR`/`THR` 0, `LCR` 3, `LSR` 5)
//! times 4: Linux rk3588-base.dtsi gives the UARTs `reg-shift = <2>` and
//! `reg-io-width = <4>` (:2558, cited in nanopc_t6_defconfig); Zephyr's
//! rk3588.dtsi gives uart2 `reg-shift = <2>` (zephyr af17c0c8, :81).
//!
//! What it cannot bound: a read of a block whose clock is off. On the T6 the
//! previous Linux keeps uart2 clocked (it is its console, `console=ttyS2`).

use crate::mmio::Mmio;

/// `RBR` / `THR` (with `LCR.DLAB` clear).
pub const THR: usize = 0x00;
/// `LCR`.
pub const LCR: usize = 0x0c;
/// `LSR`.
pub const LSR: usize = 0x14;
/// `LCR.DLAB`: offsets 0 and 4 are the divisor latch.
pub const LCR_DLAB: u32 = 1 << 7;
/// `LSR.THRE`: the transmit holding register (or FIFO) is empty.
pub const LSR_THRE: u32 = 1 << 5;
/// `LSR` reads per byte at most. A byte at 1.5 Mbaud takes 6.7 us; a device
/// read takes well under 1 us, so this is several hundred byte times.
pub const POLL_MAX: u32 = 100_000;

/// A transmitter on the UART at `base`.
pub struct Tx<'a, M: Mmio> {
    m: &'a M,
    base: usize,
    usable: bool,
    /// Bytes written after [`POLL_MAX`] reads without `THRE`.
    pub timeouts: u32,
}

impl<'a, M: Mmio> Tx<'a, M> {
    /// Reads `LCR` once (see the module).
    pub fn new(m: &'a M, base: usize) -> Self {
        let usable = m.read32(base + LCR) & LCR_DLAB == 0;
        Tx { m, base, usable, timeouts: 0 }
    }

    /// False when `LCR.DLAB` was set: nothing is written.
    pub fn usable(&self) -> bool {
        self.usable
    }

    pub fn byte(&mut self, b: u8) {
        if !self.usable {
            return;
        }
        let mut n = 0;
        while self.m.read32(self.base + LSR) & LSR_THRE == 0 {
            n += 1;
            if n == POLL_MAX {
                self.timeouts += 1;
                break;
            }
        }
        self.m.write32(self.base + THR, u32::from(b));
    }

    pub fn str(&mut self, s: &[u8]) {
        for &b in s {
            self.byte(b);
        }
    }

    /// `0x` and `digits` hex digits of `v` (the low ones), lower case.
    pub fn hex(&mut self, v: u64, digits: u32) {
        self.str(b"0x");
        for i in (0..digits.min(16)).rev() {
            let d = ((v >> (4 * i)) & 0xf) as u8;
            self.byte(if d < 10 { b'0' + d } else { b'a' + d - 10 });
        }
    }

    /// `v` in decimal, with a leading `-` when negative.
    pub fn dec(&mut self, v: i64) {
        if v < 0 {
            self.byte(b'-');
        }
        let mut u = v.unsigned_abs();
        let mut div = 1u64;
        while u / div >= 10 {
            div *= 10;
        }
        loop {
            self.byte(b'0' + (u / div) as u8);
            u %= div;
            if div == 1 {
                break;
            }
            div /= 10;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    pub const BASE: usize = 0xfeb5_0000;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum Op {
        R(usize, u32),
        W(usize, u32),
    }

    /// A 16550 with 32-bit registers: `LSR.THRE` reads clear for the first
    /// `busy` reads after each byte, then set.
    pub struct Uart {
        pub lcr: u32,
        pub busy: u32,
        left: RefCell<u32>,
        pub ops: RefCell<Vec<Op>>,
    }

    impl Uart {
        pub fn linux_left(busy: u32) -> Self {
            // 8N1, DLAB clear: what Linux's 8250 leaves after set_termios
            Uart { lcr: 0x03, busy, left: RefCell::new(busy), ops: RefCell::new(vec![]) }
        }
        pub fn sent(&self) -> Vec<u8> {
            self.ops
                .borrow()
                .iter()
                .filter_map(|o| match *o {
                    Op::W(a, v) if a == BASE + THR => Some(v as u8),
                    _ => None,
                })
                .collect()
        }
    }

    impl Mmio for Uart {
        fn read32(&self, addr: usize) -> u32 {
            let v = match addr - BASE {
                LCR => self.lcr,
                LSR => {
                    let mut l = self.left.borrow_mut();
                    if *l > 0 {
                        *l -= 1;
                        0x00
                    } else {
                        0x60
                    }
                }
                off => panic!("the transmitter reads only LCR and LSR, not {off:#x}"),
            };
            self.ops.borrow_mut().push(Op::R(addr, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            assert_eq!(addr, BASE + THR, "the transmitter writes only THR");
            assert!(val <= 0xff, "one byte per store");
            *self.left.borrow_mut() = self.busy;
            self.ops.borrow_mut().push(Op::W(addr, val));
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!("32-bit registers")
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!("32-bit registers")
        }
    }

    #[test]
    fn each_byte_waits_for_thre_then_goes_to_thr() {
        let u = Uart::linux_left(2);
        let mut tx = Tx::new(&u, BASE);
        tx.str(b"T6");
        assert_eq!(
            *u.ops.borrow(),
            vec![
                Op::R(BASE + LCR, 0x03),
                Op::R(BASE + LSR, 0x00),
                Op::R(BASE + LSR, 0x00),
                Op::R(BASE + LSR, 0x60),
                Op::W(BASE + THR, b'T' as u32),
                Op::R(BASE + LSR, 0x00),
                Op::R(BASE + LSR, 0x00),
                Op::R(BASE + LSR, 0x60),
                Op::W(BASE + THR, b'6' as u32),
            ]
        );
        assert_eq!(tx.timeouts, 0);
    }

    #[test]
    fn dlab_set_means_nothing_is_written() {
        let mut u = Uart::linux_left(0);
        u.lcr = 0x83;
        let mut tx = Tx::new(&u, BASE);
        tx.str(b"T6:A\r\n");
        assert!(!tx.usable());
        assert_eq!(*u.ops.borrow(), vec![Op::R(BASE + LCR, 0x83)]);
    }

    #[test]
    fn a_uart_that_never_drains_costs_poll_max_reads_per_byte() {
        let u = Uart::linux_left(u32::MAX);
        let mut tx = Tx::new(&u, BASE);
        tx.str(b"AB");
        assert_eq!(tx.timeouts, 2);
        assert_eq!(u.sent(), b"AB");
        let reads = u.ops.borrow().iter().filter(|o| matches!(o, Op::R(a, _) if *a == BASE + LSR)).count();
        assert_eq!(reads, 2 * POLL_MAX as usize);
    }

    #[test]
    fn numbers() {
        let u = Uart::linux_left(0);
        let mut tx = Tx::new(&u, BASE);
        tx.hex(0x8000_0000, 8);
        tx.byte(b' ');
        tx.hex(0x0000_0004_8800_003f, 16);
        tx.byte(b' ');
        for v in [0, 7, 10, -5, 22369, i64::MIN] {
            tx.dec(v);
            tx.byte(b',');
        }
        assert_eq!(
            String::from_utf8(u.sent()).unwrap(),
            "0x80000000 0x000000048800003f 0,7,10,-5,22369,-9223372036854775808,"
        );
    }
}
