// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The i.MX 95's uSDHC (USDHC1, the Verdin iMX95's eMMC, at 0x42850000) as an
//! [`emmc::Bus`]: polled, no interrupt, no DMA, one 512-byte block per
//! command through the data port, 1-bit bus.
//!
//! Why not Zephyr's `imx_usdhc` driver: at the workspace pin it builds on
//! the MCUX `fsl_usdhc` driver, which needs `USDHC_Type` and the
//! `FSL_FEATURE_USDHC_*` features; the MIMX9596 A55 device header names the
//! block `WAKEUP_USDHC_Type` and its features file has no USDHC entry
//! (hal_nxp mcux/mcux-sdk/devices/MIMX9596/MIMX9596_ca55{,_features}.h).
//!
//! Register offsets and fields: that device header's `WAKEUP_USDHC_Type`
//! and its `WAKEUP_USDHC_*_MASK` definitions. Two facts it does not state,
//! taken from U-Boot's `fsl_esdhc_imx.c` (the uSDHC path SPL ran on this
//! card): on the uSDHC the transfer-mode bits 6..0 are written to `MIX_CTRL`
//! and only bits 31..16 to `CMD_XFR_TYP`; and `SYS_CTRL.SDCLKFS` is the
//! prescaler / 2 (0: divide by 1, 0x80: by 256), `DVS` the divisor - 1. A
//! wrong reading of the prescaler can only make the clock slower than
//! asked, never faster: see [`divider`].
//!
//! What is relied on from U-Boot SPL (the chain booted from this card):
//! USDHC1's clock root on and its pads muxed to USDHC1 with their supply;
//! the controller's register access granted to the A55 non-secure side.
//! What is redone: the controller's software reset (`RSTA`: bus width,
//! timing, tuning, DLLs, the HS400 mode bits off), the 80 initialisation
//! clocks, the SD clock from the root rate, and the card from CMD0
//! (`emmc::card_up`). `VEND_SPEC.VSELECT` (the I/O voltage select, which SPL
//! left for the card's 1.8 V I/O) is read before the reset and written back
//! after it.

use crate::emmc::{Bus, BusError, Data, Resp, BLOCK};

/// Register offsets (`WAKEUP_USDHC_Type`).
pub mod reg {
    pub const BLK_ATT: usize = 0x04;
    pub const CMD_ARG: usize = 0x08;
    pub const CMD_XFR_TYP: usize = 0x0c;
    pub const CMD_RSP0: usize = 0x10;
    pub const CMD_RSP1: usize = 0x14;
    pub const CMD_RSP2: usize = 0x18;
    pub const CMD_RSP3: usize = 0x1c;
    pub const DATA_BUFF_ACC_PORT: usize = 0x20;
    pub const PRES_STATE: usize = 0x24;
    pub const PROT_CTRL: usize = 0x28;
    pub const SYS_CTRL: usize = 0x2c;
    pub const INT_STATUS: usize = 0x30;
    pub const INT_STATUS_EN: usize = 0x34;
    pub const INT_SIGNAL_EN: usize = 0x38;
    pub const WTMK_LVL: usize = 0x44;
    pub const MIX_CTRL: usize = 0x48;
    pub const DLL_CTRL: usize = 0x60;
    pub const STROBE_DLL_CTRL: usize = 0x70;
    pub const VEND_SPEC: usize = 0xc0;
}

pub mod bits {
    // CMD_XFR_TYP
    pub const RSPTYP_136: u32 = 1 << 16;
    pub const RSPTYP_48: u32 = 2 << 16;
    pub const RSPTYP_48_BUSY: u32 = 3 << 16;
    pub const CCCEN: u32 = 1 << 19;
    pub const CICEN: u32 = 1 << 20;
    pub const DPSEL: u32 = 1 << 21;
    pub const CMDINX_SHIFT: u32 = 24;
    // MIX_CTRL: the transfer-mode bits 6..0; DTDSEL = read
    pub const MIX_MODE_BITS: u32 = 0x7f;
    pub const MIX_DTDSEL: u32 = 1 << 4;
    // PRES_STATE
    pub const CIHB: u32 = 1 << 0;
    pub const CDIHB: u32 = 1 << 1;
    pub const SDSTB: u32 = 1 << 3;
    pub const BWEN: u32 = 1 << 10;
    pub const BREN: u32 = 1 << 11;
    pub const DAT0: u32 = 1 << 24;
    // PROT_CTRL: EMODE 2 = little endian, DTW 0 = 1-bit
    pub const EMODE_LE: u32 = 2 << 4;
    // SYS_CTRL
    pub const DVS_SHIFT: u32 = 4;
    pub const DVS_MASK: u32 = 0xf0;
    pub const SDCLKFS_SHIFT: u32 = 8;
    pub const SDCLKFS_MASK: u32 = 0xff00;
    pub const DTOCV_MASK: u32 = 0xf_0000;
    /// SDCLK x 2^29: "recommended for the speed modes other than HS200,
    /// HS400, SDR104".
    pub const DTOCV_LEGACY: u32 = 0xd << 16;
    pub const RSTA: u32 = 1 << 24;
    pub const RSTC: u32 = 1 << 25;
    pub const RSTD: u32 = 1 << 26;
    pub const INITA: u32 = 1 << 27;
    pub const RESETS: u32 = 0x1f00_0000 | (1 << 22);
    // INT_STATUS
    pub const CC: u32 = 1 << 0;
    pub const TC: u32 = 1 << 1;
    pub const BWR: u32 = 1 << 4;
    pub const BRR: u32 = 1 << 5;
    pub const CTOE: u32 = 1 << 16;
    pub const CCE: u32 = 1 << 17;
    pub const CEBE: u32 = 1 << 18;
    pub const CIE: u32 = 1 << 19;
    pub const DTOE: u32 = 1 << 20;
    pub const DCE: u32 = 1 << 21;
    pub const DEBE: u32 = 1 << 22;
    pub const CMD_ERRORS: u32 = CTOE | CCE | CEBE | CIE;
    pub const DATA_ERRORS: u32 = DTOE | DCE | DEBE;
    pub const USED: u32 = CC | TC | BWR | BRR | CMD_ERRORS | DATA_ERRORS;
    // VEND_SPEC
    pub const VSELECT: u32 = 1 << 1;
}

use bits::*;

/// The buffer watermark, in words: a block moves in 32 bursts of 16.
pub const WML_WORDS: usize = 16;

/// Register access by offset from the block's base.
pub trait Regs {
    fn read32(&self, off: usize) -> u32;
    fn write32(&self, off: usize, val: u32);
}

/// The SD clock's targets.
pub const IDENT_HZ: u32 = 400_000;
pub const TRANSFER_HZ: u32 = 20_000_000;
/// The root rate assumed when the System Manager does not say it (0): high
/// enough that the clocks come out at or below their targets for any root
/// up to it (U-Boot runs USDHC1's root at 400 MHz; 1.6 GHz is four times that).
pub const RATE_ASSUMED: u32 = 1_600_000_000;

/// `(SDCLKFS, DVS, Hz)` for the fastest SD clock at or below `target` from
/// `rate`: prescaler 1, 2, 4 .. 256 times divisor 1 .. 16. At the largest
/// division (4096) the target is not met only for a root above 1.64 GHz.
pub const fn divider(rate: u32, target: u32) -> (u32, u32, u32) {
    let mut best = (0x80u32, 0xfu32, rate / 4096);
    let mut best_div = 4096u32;
    let mut pre = 1u32;
    while pre <= 256 {
        let mut d = 1u32;
        while d <= 16 {
            let div = pre * d;
            // The clock rounded up, so a remainder never pushes it past the target.
            if div < best_div && (rate as u64).div_ceil(div as u64) <= target as u64 {
                best_div = div;
                best = (pre / 2, d - 1, rate / div);
            }
            d += 1;
        }
        pre *= 2;
    }
    best
}

/// Bounds of the polls, in microseconds (each poll waits 1 us after a read).
pub const RESET_US: u32 = 100_000;
pub const CLOCK_US: u32 = 100_000;
pub const INHIBIT_US: u32 = 100_000;
pub const CMD_US: u32 = 1_000_000;
pub const DATA_US: u32 = 1_000_000;

pub struct Usdhc<'a, R: Regs, D: FnMut(u32)> {
    r: &'a R,
    rate: u32,
    delay: D,
    /// The SD clock now, Hz (for the log).
    pub sd_hz: u32,
}

impl<'a, R: Regs, D: FnMut(u32)> Usdhc<'a, R, D> {
    /// A controller whose root runs at `rate` Hz (0: unknown,
    /// [`RATE_ASSUMED`]); `delay` waits the microseconds it is given.
    pub fn new(r: &'a R, rate: u32, delay: D) -> Self {
        Usdhc { r, rate: if rate == 0 { RATE_ASSUMED } else { rate }, delay, sd_hz: 0 }
    }

    pub fn rate(&self) -> u32 {
        self.rate
    }

    fn poll(&mut self, off: usize, mask: u32, want: u32, us: u32) -> Result<u32, u32> {
        let mut v = 0;
        for _ in 0..=us {
            v = self.r.read32(off);
            if v & mask == want {
                return Ok(v);
            }
            (self.delay)(1);
        }
        Err(v)
    }

    /// The software reset and the set-up for this module's transfers; the
    /// clock is the identification clock after it.
    pub fn init(&mut self) -> Result<(), u32> {
        let vselect = self.r.read32(reg::VEND_SPEC) & VSELECT;
        let sys = self.r.read32(reg::SYS_CTRL);
        self.r.write32(reg::SYS_CTRL, sys | RSTA);
        self.poll(reg::SYS_CTRL, RSTA, 0, RESET_US).map_err(|_| BusError::Reset as u32)?;
        let vs = self.r.read32(reg::VEND_SPEC);
        self.r.write32(reg::VEND_SPEC, (vs & !VSELECT) | vselect);
        self.r.write32(reg::MIX_CTRL, 0);
        self.r.write32(reg::DLL_CTRL, 0);
        self.r.write32(reg::STROBE_DLL_CTRL, 0);
        self.r.write32(reg::PROT_CTRL, EMODE_LE);
        self.r.write32(reg::WTMK_LVL, (WML_WORDS as u32) | ((WML_WORDS as u32) << 16));
        self.r.write32(reg::INT_SIGNAL_EN, 0);
        self.r.write32(reg::INT_STATUS_EN, USED);
        self.r.write32(reg::INT_STATUS, u32::MAX);
        self.set_clock(false)?;
        let sys = self.r.read32(reg::SYS_CTRL) & !RESETS;
        self.r.write32(reg::SYS_CTRL, sys | INITA);
        self.poll(reg::SYS_CTRL, INITA, 0, RESET_US).map_err(|_| BusError::Reset as u32)?;
        Ok(())
    }

    /// Clear the CMD (and DAT) line state after an error.
    fn reset_lines(&mut self, data: bool) {
        let bits = RSTC | if data { RSTD } else { 0 };
        let sys = self.r.read32(reg::SYS_CTRL) & !RESETS;
        self.r.write32(reg::SYS_CTRL, sys | bits);
        let _ = self.poll(reg::SYS_CTRL, bits, 0, RESET_US);
        self.r.write32(reg::INT_STATUS, u32::MAX);
    }

    fn cmd_error(st: u32) -> u32 {
        (if st & CTOE != 0 {
            BusError::CmdTimeout
        } else if st & CCE != 0 {
            BusError::CmdCrc
        } else if st & CEBE != 0 {
            BusError::CmdEndBit
        } else {
            BusError::CmdIndex
        }) as u32
    }

    fn data_error(st: u32) -> u32 {
        (if st & DTOE != 0 {
            BusError::DataTimeout
        } else if st & DCE != 0 {
            BusError::DataCrc
        } else {
            BusError::DataEndBit
        }) as u32
    }

    /// INT_STATUS once any bit of `mask` is set, or its last value after `us`.
    fn poll_any(&mut self, mask: u32, us: u32) -> u32 {
        let mut v = 0;
        for _ in 0..=us {
            v = self.r.read32(reg::INT_STATUS);
            if v & mask != 0 {
                break;
            }
            (self.delay)(1);
        }
        v
    }

    /// Wait for `flag` in INT_STATUS, failing on the data errors.
    fn wait_data(&mut self, flag: u32) -> Result<(), u32> {
        let v = self.poll_any(flag | DATA_ERRORS, DATA_US);
        if v & DATA_ERRORS != 0 {
            Err(Self::data_error(v))
        } else if v & flag == 0 {
            Err(BusError::Transfer as u32)
        } else {
            Ok(())
        }
    }
}

impl<R: Regs, D: FnMut(u32)> Bus for Usdhc<'_, R, D> {
    fn command(&mut self, index: u8, arg: u32, resp: Resp, mut data: Data<'_>) -> Result<[u32; 4], u32> {
        let with_data = !matches!(data, Data::None);
        let inhibit = CIHB | if with_data || resp == Resp::R1b { CDIHB } else { 0 };
        self.poll(reg::PRES_STATE, inhibit, 0, INHIBIT_US).map_err(|_| BusError::Inhibit as u32)?;
        self.r.write32(reg::INT_STATUS, u32::MAX);
        let mut xfr = (index as u32 & 0x3f) << CMDINX_SHIFT;
        xfr |= match resp {
            Resp::None => 0,
            Resp::R1 => RSPTYP_48 | CCCEN | CICEN,
            Resp::R1b => RSPTYP_48_BUSY | CCCEN | CICEN,
            Resp::R2 => RSPTYP_136 | CCCEN,
            Resp::R3 => RSPTYP_48,
        };
        let mut mode = 0;
        if with_data {
            xfr |= DPSEL;
            self.r.write32(reg::BLK_ATT, BLOCK as u32 | (1 << 16));
            if matches!(data, Data::Read(_)) {
                mode = MIX_DTDSEL;
            }
        }
        self.r.write32(reg::CMD_ARG, arg);
        let mix = self.r.read32(reg::MIX_CTRL);
        self.r.write32(reg::MIX_CTRL, (mix & !MIX_MODE_BITS) | mode);
        self.r.write32(reg::CMD_XFR_TYP, xfr);

        let st = self.poll_any(CC | CMD_ERRORS, CMD_US);
        if st & CMD_ERRORS != 0 || st & CC == 0 {
            self.reset_lines(with_data);
            return Err(if st & CMD_ERRORS != 0 { Self::cmd_error(st) } else { BusError::CmdTimeout as u32 });
        }
        self.r.write32(reg::INT_STATUS, CC);
        let rsp = [
            self.r.read32(reg::CMD_RSP0),
            self.r.read32(reg::CMD_RSP1),
            self.r.read32(reg::CMD_RSP2),
            self.r.read32(reg::CMD_RSP3),
        ];
        let out = if resp == Resp::R2 {
            // The controller keeps bits 127..8 (no CRC) in RSP3..RSP0 shifted
            // down by 8 (U-Boot fsl_esdhc_imx.c, esdhc_send_cmd_common).
            [
                (rsp[3] << 8) | (rsp[2] >> 24),
                (rsp[2] << 8) | (rsp[1] >> 24),
                (rsp[1] << 8) | (rsp[0] >> 24),
                rsp[0] << 8,
            ]
        } else {
            [rsp[0], 0, 0, 0]
        };

        let moved = match &mut data {
            Data::None => Ok(()),
            Data::Read(buf) => {
                let mut r = Ok(());
                for chunk in buf.as_chunks_mut::<{ WML_WORDS * 4 }>().0 {
                    r = self.wait_data(BRR);
                    if r.is_err() {
                        break;
                    }
                    self.r.write32(reg::INT_STATUS, BRR);
                    for w in chunk.as_chunks_mut::<4>().0 {
                        *w = self.r.read32(reg::DATA_BUFF_ACC_PORT).to_le_bytes();
                    }
                }
                r
            }
            Data::Write(buf) => {
                let mut r = Ok(());
                for chunk in buf.as_chunks::<{ WML_WORDS * 4 }>().0 {
                    r = self.wait_data(BWR);
                    if r.is_err() {
                        break;
                    }
                    self.r.write32(reg::INT_STATUS, BWR);
                    for w in chunk.as_chunks::<4>().0 {
                        self.r.write32(reg::DATA_BUFF_ACC_PORT, u32::from_le_bytes(*w));
                    }
                }
                r
            }
        };
        let moved = moved.and_then(|()| if with_data { self.wait_data(TC) } else { Ok(()) });
        if let Err(e) = moved {
            self.reset_lines(true);
            return Err(e);
        }
        self.r.write32(reg::INT_STATUS, u32::MAX);
        Ok(out)
    }

    fn busy(&mut self) -> bool {
        self.r.read32(reg::PRES_STATE) & DAT0 == 0
    }

    fn delay_us(&mut self, us: u32) {
        (self.delay)(us);
    }

    fn set_clock(&mut self, transfer: bool) -> Result<(), u32> {
        let (fs, dvs, hz) = divider(self.rate, if transfer { TRANSFER_HZ } else { IDENT_HZ });
        let sys = self.r.read32(reg::SYS_CTRL) & !(RESETS | DVS_MASK | SDCLKFS_MASK | DTOCV_MASK);
        self.r.write32(reg::SYS_CTRL, sys | (fs << SDCLKFS_SHIFT) | (dvs << DVS_SHIFT) | DTOCV_LEGACY);
        self.poll(reg::PRES_STATE, SDSTB, SDSTB, CLOCK_US).map_err(|_| BusError::Clock as u32)?;
        self.sd_hz = hz;
        Ok(())
    }
}
