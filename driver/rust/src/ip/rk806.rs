// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The Rockchip RK806 PMIC on SPI, read only: the frame of a register read
//! and the meaning of the registers that say whether a buck converter is on
//! and at what voltage. Nothing here can build a write: the NanoPC-T6's
//! RK806 supplies the whole board, and a wrong write turns it off.
//!
//! Facts from Linux v6.18 (GPL-2.0: facts and file:line only, nothing
//! transcribed):
//!
//! 1. A read is one frame: a command byte, the register address as two
//!    bytes, low byte first (`reg_bits` 16, `REGMAP_ENDIAN_LITTLE`), then
//!    the values (drivers/mfd/rk8xx-spi.c:17-18, 30-35, 66-83). The command
//!    is `RK806_CMD_READ` 0 | `RK806_CMD_CRC_DIS` 0 | (count - 1) in bits
//!    3:0; bit 7 is `RK806_CMD_WRITE` (include/linux/mfd/rk808.h:693-698).
//! 2. `RK806_POWER_EN0` 0x00 holds DCDC1..4: bit n (n = 0..3) reads 1 when
//!    DCDCn+1 is on; bits 7:4 are the write-enable twins
//!    (`ENABLE_MASK`, rk808-regulator.c:73; `rk806_reg` dcdc-reg1,
//!    :1126-1129; rk808.h:435).
//! 3. `RK806_BUCK1_ON_VSEL` 0x1a is DCDC1's voltage: 500 mV + 6.25 mV ·
//!    sel for sel 0..159, 1500 mV + 25 mV · (sel - 160) for 160..235,
//!    3400 mV above (`rk806_buck_voltage_ranges`, rk808-regulator.c:
//!    1114-1118; rk808.h:461). `RK806_BUCK1_CONFIG` 0x10 (rk808.h:451).
//! 4. On the NanoPC-T6, vdd_gpu_s0 is DCDC1 (`dcdc-reg1`, regulator-boot-on,
//!    not always-on; rk3588-nanopc-t6.dtsi:812-821), the GPU's and PD_GPU's
//!    supply (:349-351, 628-630). The PMIC is SPI2 chip select 0 at most
//!    1 MHz, mode 0 (:755-766).
//!
//! Measured on the T6 under Armbian (2026-10-19, debugfs regmap of
//! spi2.0, read only): `POWER_EN0` 0x0f, `POWER_SLP_EN0` 0xe0,
//! `BUCK1_CONFIG` 0xa4, `BUCK1_ON_VSEL` 0x1c (675 mV, what
//! `regulator_summary` shows), with the GPU suspended and active alike.

pub const POWER_EN0: u16 = 0x00;
pub const POWER_SLP_EN0: u16 = 0x06;
pub const BUCK1_CONFIG: u16 = 0x10;
pub const BUCK1_ON_VSEL: u16 = 0x1a;

/// `POWER_EN0` bit of DCDC1 (vdd_gpu_s0 on the T6).
pub const DCDC1_ON: u8 = 1 << 0;

/// The most values one read frame carries (bits 3:0 of the command).
pub const MAX_READ: usize = 16;

/// The bytes of a read of `count` registers from `reg` (fact 1): the command,
/// the address, then `count` zero bytes clocked out while the values come in.
/// `None` for a count of 0 or above [`MAX_READ`], or a frame longer than
/// `out`. The length of the frame otherwise.
pub fn read_frame(reg: u16, count: usize, out: &mut [u8]) -> Option<usize> {
    let len = 3 + count;
    if count == 0 || count > MAX_READ || out.len() < len {
        return None;
    }
    out[0] = (count - 1) as u8;
    out[1] = reg as u8;
    out[2] = (reg >> 8) as u8;
    out[3..len].fill(0);
    Some(len)
}

/// DCDC1..DCDC10's `*_ON_VSEL` value in microvolts (fact 3).
pub const fn buck_uv(sel: u8) -> u32 {
    let s = sel as u32;
    if s <= 159 {
        500_000 + 6_250 * s
    } else if s <= 235 {
        1_500_000 + 25_000 * (s - 160)
    } else {
        3_400_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_frame_is_the_count_the_address_low_first_then_zeros() {
        let mut f = [0xffu8; 8];
        assert_eq!(read_frame(BUCK1_ON_VSEL, 1, &mut f), Some(4));
        assert_eq!(f[..4], [0x00, 0x1a, 0x00, 0x00]);
        assert_eq!(read_frame(0x0123, 3, &mut f), Some(6));
        assert_eq!(f[..6], [0x02, 0x23, 0x01, 0, 0, 0]);
    }

    #[test]
    fn no_frame_this_builds_is_a_write() {
        let mut f = [0u8; 3 + MAX_READ];
        for reg in [0u16, 0x1a, 0xff, 0xffff] {
            for count in 1..=MAX_READ {
                read_frame(reg, count, &mut f).unwrap();
                assert_eq!(f[0] & 0x80, 0, "RK806_CMD_WRITE in {f:02x?}");
                assert_eq!(f[0] & 0x40, 0, "CRC enabled in {f:02x?}");
            }
        }
        assert_eq!(read_frame(0, 0, &mut f), None);
        assert_eq!(read_frame(0, MAX_READ + 1, &mut f), None);
        assert_eq!(read_frame(0, 6, &mut [0u8; 8]), None);
    }

    #[test]
    fn the_t6_reads_675_mv_on_dcdc1() {
        assert_eq!(buck_uv(0x1c), 675_000);
        assert_eq!(buck_uv(0), 500_000);
        assert_eq!(buck_uv(159), 1_493_750);
        assert_eq!(buck_uv(160), 1_500_000);
        assert_eq!(buck_uv(235), 3_375_000);
        assert_eq!(buck_uv(236), 3_400_000);
        assert_eq!(0x0f & DCDC1_ON, DCDC1_ON);
    }
}
