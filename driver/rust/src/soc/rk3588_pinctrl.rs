// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The RK3588's pin controller (the IOC) and its GPIO banks: where a pin's
//! function, pull, drive level and schmitt bit are, and the words that set
//! them. A board's pin table (`board::nanopc_t6_pins`) is built from these.
//!
//! Facts from Linux v6.18 (GPL-2.0: facts and file:line only, nothing
//! transcribed). `pinctrl-rockchip.c` is drivers/pinctrl/pinctrl-rockchip.c,
//! `pinctrl-rockchip.h` its header, `gpio-rockchip.c` drivers/gpio/:
//!
//! - The IOC is one block at 0xfd5f0000, 0x10000 bytes (rk3588-base.dtsi:729-731),
//!   the pin controller's register map (rk3588-base.dtsi:3253-3256). Its
//!   parts: BUS_IOC at +0x8000, VCCIO1_4_IOC +0x9000, VCCIO3_5_IOC +0xa000,
//!   VCCIO2_IOC +0xb000, VCCIO6_IOC +0xc000 (pinctrl-rockchip.c:2462-2469).
//! - A pin's number is bank·32 + pin in the bank (pinctrl-rockchip.h:54,
//!   :161, :169: `RK_GPIO1_A0` 32, `RK_GPIO4_B0` 136, `RK_GPIO4_C0` 144);
//!   A0..A7 are 0..7, B0 8, C0 16, D0 24 (dt-bindings/pinctrl/rockchip.h:12-36).
//! - Function: 4 bits a pin, every bank 4-bit (pinctrl-rockchip.c:4509-4520).
//!   The words follow each other from 0 at 8 bytes a group of eight pins
//!   (pinctrl-rockchip.c:3675, :3698-3707, :3728-3734), so bank b, group g
//!   is at 0x20·b + 8·g; pins 4..7 of a group in the next word, the field at
//!   (pin mod 4)·4 (:1245-1249). Banks 1 to 4 add BUS_IOC (:1300-1301); bank
//!   0 is split between the PMU IOCs and BUS_IOC (:1268-1299) and is not
//!   covered here.
//! - Pull: 2 bits a pin, eight pins a word, the field at (pin mod 8)·2; the
//!   word is the last entry of `rk3588_p_regs` whose first pin is not above
//!   the pin (pinctrl-rockchip.c:2515-2539, :2567-2590). Every bank is
//!   `PULL_TYPE_IO_1V8_ONLY` (:4509-4520; pinctrl-rockchip.h:270-271), whose
//!   list makes "no pull" (bias-disable) the value 0 (:2847-2860, :2965-2973).
//! - Drive: 4 bits a pin, four pins a word, the field at (pin mod 4)·4, the
//!   word from `rk3588_ds_regs` (:2471-2513, :2592-2615); the value is the
//!   device tree's `drive-strength` as it is (:2748-2751, :2836-2840).
//! - Schmitt: 1 bit a pin, eight pins a word, from `rk3588_smt_regs`
//!   (:2541-2565, :2617-2641); 1 is on (:3108-3112).
//! - Every IOC word takes a write mask in bits 31:16: a bit of 15:0 changes
//!   only where its twin is set (:1329-1332, :2836-2842, :2989-2993, :3110).
//! - The GPIO banks: gpio0 0xfd8a0000, gpio1 0xfec20000, gpio2 0xfec30000,
//!   gpio3 0xfec40000, gpio4 0xfec50000 (rk3588-base.dtsi:3260, :3272,
//!   :3284, :3296, :3308). Version 2 registers: `DR_L` 0x00 (output level),
//!   `DDR_L` 0x08 (direction, 1 = output) for pins 0..15, the `_H` twin at
//!   +4 for 16..31, `EXT_PORT` 0x70 (the input levels of all 32 pins)
//!   (gpio-rockchip.c:54-70); a bit is written with its twin in 31:16
//!   (:108-120). An output is set level first, then direction
//!   (gpio-rockchip.c:272-278, :160-175).
//!
//! Only the banks the NanoPC-T6's table uses have their pull, drive and
//! schmitt words here (1 and 4); any other pin is `None`.

/// The IOC's physical base and size.
pub const IOC_BASE: usize = 0xfd5f_0000;
pub const IOC_SIZE: usize = 0x1_0000;

/// The IOC's parts, as offsets from [`IOC_BASE`].
pub const BUS_IOC: usize = 0x8000;
pub const VCCIO1_4_IOC: usize = 0x9000;
pub const VCCIO3_5_IOC: usize = 0xa000;
pub const VCCIO2_IOC: usize = 0xb000;
pub const VCCIO6_IOC: usize = 0xc000;

/// The GPIO banks' physical bases, bank 0 first.
pub const GPIO_BASE: [usize; 5] = [0xfd8a_0000, 0xfec2_0000, 0xfec3_0000, 0xfec4_0000, 0xfec5_0000];

/// A GPIO bank's registers (version 2), for pins 0..15.
pub const GPIO_DR_L: usize = 0x00;
pub const GPIO_DDR_L: usize = 0x08;
/// The input levels of the bank's 32 pins.
pub const GPIO_EXT_PORT: usize = 0x70;

/// Pull: none (`bias-disable`).
pub const PULL_NONE: u32 = 0;
/// Function 0 is the pin as a GPIO (`RK_FUNC_GPIO`,
/// dt-bindings/pinctrl/rockchip.h:45).
pub const FUNC_GPIO: u32 = 0;

/// A pin: its bank (0..=4) and its number in the bank (0..=31).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pin {
    pub bank: u8,
    pub pin: u8,
}

impl Pin {
    /// GPIO`bank`_`group``n`: `group` 0..=3 for A..D, `n` 0..=7.
    pub const fn new(bank: u8, group: u8, n: u8) -> Pin {
        assert!(bank <= 4 && group <= 3 && n <= 7);
        Pin { bank, pin: group * 8 + n }
    }
}

/// A field of a word that takes a write mask in its bits 31:16: `off` from
/// the block's base, `shift` and `width` in bits 15:0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub off: usize,
    pub shift: u32,
    pub width: u32,
}

impl Field {
    pub const fn mask(self) -> u32 {
        ((1 << self.width) - 1) << self.shift
    }

    /// The word that sets the field to `v` and leaves the rest of 15:0.
    pub const fn word(self, v: u32) -> u32 {
        assert!(v < 1 << self.width);
        self.mask() << 16 | v << self.shift
    }

    /// The field's value in a word read back.
    pub const fn get(self, word: u32) -> u32 {
        (word & self.mask()) >> self.shift
    }
}

/// The function field (bank 0: `None`).
pub const fn mux(p: Pin) -> Option<Field> {
    if p.bank == 0 || p.bank > 4 || p.pin > 31 {
        return None;
    }
    let group = (p.pin / 8) as usize;
    let high = if p.pin % 8 >= 4 { 4 } else { 0 };
    Some(Field { off: BUS_IOC + 0x20 * p.bank as usize + 8 * group + high, shift: (p.pin as u32 % 4) * 4, width: 4 })
}

/// The last entry of `t` whose first pin (in the bank) is not above `pin`.
const fn lookup(t: &[(u8, usize)], pin: u8) -> Option<usize> {
    let mut i = t.len();
    while i > 0 {
        i -= 1;
        if pin >= t[i].0 {
            return Some(t[i].1);
        }
    }
    None
}

/// `rk3588_p_regs` for banks 1 and 4 (pinctrl-rockchip.c:2521-2524, :2534-2538).
const fn pull_words(bank: u8) -> &'static [(u8, usize)] {
    match bank {
        1 => &[
            (0, VCCIO1_4_IOC + 0x110),
            (8, VCCIO1_4_IOC + 0x114),
            (16, VCCIO1_4_IOC + 0x118),
            (24, VCCIO1_4_IOC + 0x11c),
        ],
        4 => &[
            (0, VCCIO6_IOC + 0x140),
            (8, VCCIO6_IOC + 0x144),
            (16, VCCIO6_IOC + 0x148),
            (18, VCCIO3_5_IOC + 0x148),
            (24, VCCIO2_IOC + 0x14c),
        ],
        _ => &[],
    }
}

/// `rk3588_ds_regs` for banks 1 and 4 (pinctrl-rockchip.c:2480-2487, :2504-2512).
const fn drive_words(bank: u8) -> &'static [(u8, usize)] {
    match bank {
        1 => &[
            (0, VCCIO1_4_IOC + 0x20),
            (4, VCCIO1_4_IOC + 0x24),
            (8, VCCIO1_4_IOC + 0x28),
            (12, VCCIO1_4_IOC + 0x2c),
            (16, VCCIO1_4_IOC + 0x30),
            (20, VCCIO1_4_IOC + 0x34),
            (24, VCCIO1_4_IOC + 0x38),
            (28, VCCIO1_4_IOC + 0x3c),
        ],
        4 => &[
            (0, VCCIO6_IOC + 0x80),
            (4, VCCIO6_IOC + 0x84),
            (8, VCCIO6_IOC + 0x88),
            (12, VCCIO6_IOC + 0x8c),
            (16, VCCIO6_IOC + 0x90),
            (18, VCCIO3_5_IOC + 0x90),
            (20, VCCIO3_5_IOC + 0x94),
            (24, VCCIO2_IOC + 0x98),
            (28, VCCIO2_IOC + 0x9c),
        ],
        _ => &[],
    }
}

/// `rk3588_smt_regs` for banks 1 and 4 (pinctrl-rockchip.c:2547-2550, :2560-2564).
const fn schmitt_words(bank: u8) -> &'static [(u8, usize)] {
    match bank {
        1 => &[
            (0, VCCIO1_4_IOC + 0x210),
            (8, VCCIO1_4_IOC + 0x214),
            (16, VCCIO1_4_IOC + 0x218),
            (24, VCCIO1_4_IOC + 0x21c),
        ],
        4 => &[
            (0, VCCIO6_IOC + 0x240),
            (8, VCCIO6_IOC + 0x244),
            (16, VCCIO6_IOC + 0x248),
            (18, VCCIO3_5_IOC + 0x248),
            (24, VCCIO2_IOC + 0x24c),
        ],
        _ => &[],
    }
}

/// The pull field (banks 1 and 4 only).
pub const fn pull(p: Pin) -> Option<Field> {
    match lookup(pull_words(p.bank), p.pin) {
        Some(off) if p.pin <= 31 => Some(Field { off, shift: (p.pin as u32 % 8) * 2, width: 2 }),
        _ => None,
    }
}

/// The drive level field (banks 1 and 4 only).
pub const fn drive(p: Pin) -> Option<Field> {
    match lookup(drive_words(p.bank), p.pin) {
        Some(off) if p.pin <= 31 => Some(Field { off, shift: (p.pin as u32 % 4) * 4, width: 4 }),
        _ => None,
    }
}

/// The schmitt bit (banks 1 and 4 only).
pub const fn schmitt(p: Pin) -> Option<Field> {
    match lookup(schmitt_words(p.bank), p.pin) {
        Some(off) if p.pin <= 31 => Some(Field { off, shift: p.pin as u32 % 8, width: 1 }),
        _ => None,
    }
}

/// A GPIO bank's bit of `pin` in the `_L`/`_H` pair at `reg_l`
/// ([`GPIO_DR_L`], [`GPIO_DDR_L`]): the word's offset in the bank and the
/// field.
pub const fn gpio_bit(reg_l: usize, pin: u8) -> Field {
    assert!(pin <= 31);
    Field { off: reg_l + if pin >= 16 { 4 } else { 0 }, shift: pin as u32 % 16, width: 1 }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn gpio(bank: u8, group: u8, n: u8) -> Pin {
        Pin::new(bank, group, n)
    }

    #[test]
    fn mux_words_follow_linuxs_offsets() {
        // GPIO1_A5: bank 1 group A at 0x20, pins 4..7 at +4, field 7:4.
        assert_eq!(mux(gpio(1, 0, 5)), Some(Field { off: 0x8024, shift: 4, width: 4 }));
        // GPIO4_B0 at 0x88, B7 at 0x8c 15:12, C0 at 0x90 3:0, C1 7:4.
        assert_eq!(mux(gpio(4, 1, 0)), Some(Field { off: 0x8088, shift: 0, width: 4 }));
        assert_eq!(mux(gpio(4, 1, 7)), Some(Field { off: 0x808c, shift: 12, width: 4 }));
        assert_eq!(mux(gpio(4, 2, 0)), Some(Field { off: 0x8090, shift: 0, width: 4 }));
        assert_eq!(mux(gpio(4, 2, 1)), Some(Field { off: 0x8090, shift: 4, width: 4 }));
        // the first and the last word of banks 1 to 4
        assert_eq!(mux(gpio(1, 0, 0)).unwrap().off, 0x8020);
        assert_eq!(mux(gpio(4, 3, 7)).unwrap().off, 0x809c);
        assert_eq!(mux(gpio(0, 0, 0)), None);
    }

    #[test]
    fn pull_drive_schmitt_words() {
        let a5 = gpio(1, 0, 5);
        assert_eq!(pull(a5), Some(Field { off: 0x9110, shift: 10, width: 2 }));
        assert_eq!(drive(a5), Some(Field { off: 0x9024, shift: 4, width: 4 }));
        assert_eq!(schmitt(a5), Some(Field { off: 0x9210, shift: 5, width: 1 }));
        let b0 = gpio(4, 1, 0);
        assert_eq!(pull(b0), Some(Field { off: 0xc144, shift: 0, width: 2 }));
        let b7 = gpio(4, 1, 7);
        assert_eq!(pull(b7), Some(Field { off: 0xc144, shift: 14, width: 2 }));
        assert_eq!(drive(b7), Some(Field { off: 0xc08c, shift: 12, width: 4 }));
        assert_eq!(schmitt(b7), Some(Field { off: 0xc244, shift: 7, width: 1 }));
        let c0 = gpio(4, 2, 0);
        assert_eq!(pull(c0), Some(Field { off: 0xc148, shift: 0, width: 2 }));
        assert_eq!(drive(c0), Some(Field { off: 0xc090, shift: 0, width: 4 }));
        assert_eq!(schmitt(c0), Some(Field { off: 0xc248, shift: 0, width: 1 }));
        let c1 = gpio(4, 2, 1);
        assert_eq!(pull(c1), Some(Field { off: 0xc148, shift: 2, width: 2 }));
        assert_eq!(drive(c1), Some(Field { off: 0xc090, shift: 4, width: 4 }));
        // GPIO4_C2 on is in VCCIO3_5, D in VCCIO2
        let c2 = gpio(4, 2, 2);
        assert_eq!(pull(c2).unwrap().off, 0xa148);
        assert_eq!(drive(c2), Some(Field { off: 0xa090, shift: 8, width: 4 }));
        assert_eq!(schmitt(c2).unwrap().off, 0xa248);
        assert_eq!(drive(gpio(4, 2, 4)).unwrap().off, 0xa094);
        assert_eq!(pull(gpio(4, 3, 7)), Some(Field { off: 0xb14c, shift: 14, width: 2 }));
        // banks without a table here
        for b in [0, 2, 3] {
            let p = gpio(b, 0, 0);
            assert_eq!((pull(p), drive(p), schmitt(p)), (None, None, None), "bank {b}");
        }
    }

    #[test]
    fn words_carry_their_write_mask() {
        let f = Field { off: 0, shift: 4, width: 4 };
        assert_eq!(f.word(5), 0x00f0_0050);
        assert_eq!(f.word(0), 0x00f0_0000);
        assert_eq!(f.get(0x0000_0550), 5);
        assert_eq!(Field { off: 0, shift: 10, width: 2 }.word(PULL_NONE), 0x0c00_0000);
        assert_eq!(Field { off: 0, shift: 7, width: 1 }.word(1), 0x0080_0080);
    }

    #[test]
    fn gpio_bits() {
        assert_eq!(gpio_bit(GPIO_DR_L, 8), Field { off: 0x00, shift: 8, width: 1 });
        assert_eq!(gpio_bit(GPIO_DR_L, 8).word(1), 0x0100_0100);
        assert_eq!(gpio_bit(GPIO_DDR_L, 8).word(1), 0x0100_0100);
        assert_eq!(gpio_bit(GPIO_DDR_L, 21), Field { off: 0x0c, shift: 5, width: 1 });
    }
}
