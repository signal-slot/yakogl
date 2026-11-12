// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! One GPIO bank of the STM32MP25 (RM0457 GPIO chapter): alternate function,
//! output, level and input, plus the per-pin RIF registers (`SECCFGR`,
//! `PRIVCFGR`, `CIDCFGR`, `SEMCR`). The bank's base address is the caller's;
//! [`bank_base`] gives the ones in ST's `stm32mp251.dtsi` (GPIOA..GPIOK
//! 0x44240000 + 0x10000 * n, GPIOZ 0x46200000).
//!
//! Register order follows Linux `drivers/pinctrl/stm32/pinctrl-stm32.c`:
//! `stm32_pmx_set_mode` (the `AFRL`/`AFRH` nibble first, then `MODER`),
//! `stm32_pconf_set_driving` (`OTYPER`), `stm32_pconf_set_speed`
//! (`OSPEEDR`), `stm32_pconf_set_bias` (`PUPDR`), each a read-modify-write of
//! the pin's field; `__stm32_gpio_set` (`BSRR`: bit n sets, bit n+16 clears)
//! and `stm32_gpio_get` (`IDR`). An output's level is written to `BSRR`
//! before its `MODER` field turns to output, as `stm32_gpio_direction_output`
//! does (`__stm32_gpio_set`, then `stm32_pmx_gpio_set_direction`).
//!
//! RIF (RM0457 GPIO chapter, "GPIO resource isolation"): `SECCFGR` bit n set
//! makes pin n secure; `PRIVCFGR` bit n set makes it privileged; `CIDCFGR[n]`
//! (0x50 + 8 n) holds `CFEN` (bit 0), `SEMEN` (bit 1), `SCID` (bits 5:4) and
//! `SEMWL` (bits 23:16); `SEMCR[n]` (0x54 + 8 n) holds `MUTEX` (bit 0) and
//! `SCID` (bits 5:4) of the holder. Linux checks a bank's pins against these
//! before it takes them (the stm32mp25 RIF check in pinctrl-stm32.c); here
//! [`access`] does it for a compartment ID the caller names. The A35 side of
//! ST's images runs as CID 1 ([`CID_A35`]). The code sets none of them: the
//! secure side owns the RIF registers. `PRIVCFGR` is not checked: the A35
//! image runs privileged (EL1).

use crate::mmio::{get, Mmio};

/// Register offsets (RM0457 GPIO register map).
pub mod r {
    pub const MODER: usize = 0x00;
    pub const OTYPER: usize = 0x04;
    pub const OSPEEDR: usize = 0x08;
    pub const PUPDR: usize = 0x0c;
    pub const IDR: usize = 0x10;
    pub const ODR: usize = 0x14;
    pub const BSRR: usize = 0x18;
    pub const AFRL: usize = 0x20;
    pub const AFRH: usize = 0x24;
    pub const SECCFGR: usize = 0x30;
    pub const PRIVCFGR: usize = 0x34;
    /// `CIDCFGR` of pin `n`.
    pub const fn cidcfgr(n: u32) -> usize {
        0x50 + 8 * n as usize
    }
    /// `SEMCR` of pin `n`.
    pub const fn semcr(n: u32) -> usize {
        0x54 + 8 * n as usize
    }
    pub const CID_CFEN: u32 = 1 << 0;
    pub const CID_SEMEN: u32 = 1 << 1;
    pub const CID_SCID_MASK: u32 = 0x3 << 4;
    pub const CID_SEMWL_MASK: u32 = 0xff << 16;
    pub const SEM_MUTEX: u32 = 1 << 0;
    pub const SEM_SCID_MASK: u32 = 0x3 << 4;
}

/// The compartment ID the A35 non-secure world runs as in ST's images.
pub const CID_A35: u32 = 1;

/// Base of a bank by letter ('A'..='K', 'Z'), `stm32mp251.dtsi`.
pub const fn bank_base(letter: char) -> Option<usize> {
    match letter {
        'A'..='K' => Some(0x4424_0000 + 0x1_0000 * (letter as usize - 'A' as usize)),
        'Z' => Some(0x4620_0000),
        _ => None,
    }
}

/// `MODER` values (RM0457).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Input = 0,
    Output = 1,
    Alternate = 2,
    Analog = 3,
}

/// `OTYPER`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OType {
    PushPull = 0,
    OpenDrain = 1,
}

/// `OSPEEDR` (00 low, 01 medium, 10 high, 11 very high).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Speed {
    Low = 0,
    Medium = 1,
    High = 2,
    VeryHigh = 3,
}

/// `PUPDR`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pull {
    None = 0,
    Up = 1,
    Down = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// Pin number above 15, or alternate function above 15.
    Range,
    /// `SECCFGR` makes the pin secure.
    Secure,
    /// `CIDCFGR` gives the pin to another compartment (or the semaphore is
    /// held by one).
    ForeignCid,
}

fn check(pin: u32) -> Result<(), Error> {
    if pin > 15 {
        Err(Error::Range)
    } else {
        Ok(())
    }
}

/// May compartment `cid` (non-secure) use `pin` of the bank at `base`?
/// `SECCFGR` set: [`Error::Secure`]. `CIDCFGR.CFEN` clear: any compartment.
/// `CFEN` set, `SEMEN` clear: only `SCID == cid`. `SEMEN` set: `cid` must be
/// in `SEMWL`, and `SEMCR` must be free or held by `cid`.
pub fn access<M: Mmio>(m: &M, base: usize, pin: u32, cid: u32) -> Result<(), Error> {
    check(pin)?;
    if m.read32(base + r::SECCFGR) >> pin & 1 != 0 {
        return Err(Error::Secure);
    }
    let c = m.read32(base + r::cidcfgr(pin));
    if c & r::CID_CFEN == 0 {
        return Ok(());
    }
    if c & r::CID_SEMEN == 0 {
        return if get(r::CID_SCID_MASK, c) == cid { Ok(()) } else { Err(Error::ForeignCid) };
    }
    if get(r::CID_SEMWL_MASK, c) >> cid & 1 == 0 {
        return Err(Error::ForeignCid);
    }
    let s = m.read32(base + r::semcr(pin));
    if s & r::SEM_MUTEX != 0 && get(r::SEM_SCID_MASK, s) != cid {
        return Err(Error::ForeignCid);
    }
    Ok(())
}

/// `stm32_pmx_set_mode`: the pin's `AFRL`/`AFRH` nibble (always written,
/// 0 for input and output), then its 2-bit `MODER` field.
fn set_mode<M: Mmio>(m: &M, base: usize, pin: u32, mode: Mode, af: u32) {
    let reg = if pin > 7 { r::AFRH } else { r::AFRL };
    m.modify32(base + reg, 0xf << (4 * (pin & 7)), af << (4 * (pin & 7)));
    m.modify32(base + r::MODER, 3 << (2 * pin), (mode as u32) << (2 * pin));
}

/// `stm32_pconf_set_driving`, `_set_speed`, `_set_bias`, in that order.
fn set_pconf<M: Mmio>(m: &M, base: usize, pin: u32, ot: OType, sp: Speed, pull: Pull) {
    m.modify32(base + r::OTYPER, 1 << pin, (ot as u32) << pin);
    m.modify32(base + r::OSPEEDR, 3 << (2 * pin), (sp as u32) << (2 * pin));
    set_bias(m, base, pin, pull);
}

/// `stm32_pconf_set_bias`.
fn set_bias<M: Mmio>(m: &M, base: usize, pin: u32, pull: Pull) {
    m.modify32(base + r::PUPDR, 3 << (2 * pin), (pull as u32) << (2 * pin));
}

/// Pin `pin` to alternate function `af` (0..=15) with the given drive type,
/// speed and bias. Checks [`access`] for `cid` first and writes nothing on
/// error.
#[allow(clippy::too_many_arguments)]
pub fn configure_af<M: Mmio>(
    m: &M,
    base: usize,
    pin: u32,
    af: u32,
    ot: OType,
    sp: Speed,
    pull: Pull,
    cid: u32,
) -> Result<(), Error> {
    if af > 15 {
        return Err(Error::Range);
    }
    access(m, base, pin, cid)?;
    set_mode(m, base, pin, Mode::Alternate, af);
    set_pconf(m, base, pin, ot, sp, pull);
    Ok(())
}

/// Pin `pin` an output at `level`; the level goes to `BSRR` before the
/// direction, as `stm32_gpio_direction_output` does.
#[allow(clippy::too_many_arguments)]
pub fn configure_output<M: Mmio>(
    m: &M,
    base: usize,
    pin: u32,
    level: bool,
    ot: OType,
    sp: Speed,
    pull: Pull,
    cid: u32,
) -> Result<(), Error> {
    access(m, base, pin, cid)?;
    write_level(m, base, pin, level);
    set_mode(m, base, pin, Mode::Output, 0);
    set_pconf(m, base, pin, ot, sp, pull);
    Ok(())
}

/// Pin `pin` an input with the given bias.
pub fn configure_input<M: Mmio>(m: &M, base: usize, pin: u32, pull: Pull, cid: u32) -> Result<(), Error> {
    access(m, base, pin, cid)?;
    set_mode(m, base, pin, Mode::Input, 0);
    set_bias(m, base, pin, pull);
    Ok(())
}

/// `__stm32_gpio_set`: `BSRR` bit n sets, bit n + 16 clears.
fn write_level<M: Mmio>(m: &M, base: usize, pin: u32, level: bool) {
    m.write32(base + r::BSRR, if level { 1 << pin } else { 1 << (pin + 16) });
}

/// Drive the pin high through `BSRR` (no read-modify-write of `ODR`).
pub fn set<M: Mmio>(m: &M, base: usize, pin: u32) {
    write_level(m, base, pin, true);
}

/// Drive the pin low through `BSRR`.
pub fn clear<M: Mmio>(m: &M, base: usize, pin: u32) {
    write_level(m, base, pin, false);
}

/// `stm32_gpio_get`: the pin's `IDR` bit.
pub fn read<M: Mmio>(m: &M, base: usize, pin: u32) -> bool {
    m.read32(base + r::IDR) >> pin & 1 != 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    const B: usize = 0x4425_0000;

    #[derive(Default)]
    struct Bank {
        regs: RefCell<HashMap<usize, u32>>,
        writes: RefCell<Vec<(usize, u32)>>,
    }
    impl Bank {
        fn set(&self, off: usize, v: u32) {
            self.regs.borrow_mut().insert(B + off, v);
        }
        fn get(&self, off: usize) -> u32 {
            *self.regs.borrow().get(&(B + off)).unwrap_or(&0)
        }
        fn log(&self) -> Vec<(usize, u32)> {
            self.writes.borrow().iter().map(|&(a, v)| (a - B, v)).collect()
        }
    }
    impl Mmio for Bank {
        fn read32(&self, a: usize) -> u32 {
            *self.regs.borrow().get(&a).unwrap_or(&0)
        }
        fn write32(&self, a: usize, v: u32) {
            self.writes.borrow_mut().push((a, v));
            self.regs.borrow_mut().insert(a, v);
        }
        fn read8(&self, _: usize) -> u8 {
            0
        }
        fn write8(&self, _: usize, _: u8) {}
    }

    #[test]
    fn layout_and_bases() {
        assert_eq!((r::cidcfgr(0), r::cidcfgr(15), r::semcr(1)), (0x50, 0xc8, 0x5c));
        assert_eq!(bank_base('A'), Some(0x4424_0000));
        assert_eq!(bank_base('K'), Some(0x442e_0000));
        assert_eq!(bank_base('Z'), Some(0x4620_0000));
        assert_eq!(bank_base('L'), None);
    }

    // Citations: OP-TEE 4.0.0-stm32mp-r3.1 (0063536697f1) and ST Linux
    // v6.6-stm32mp (548f960c), paths relative to each tree.

    #[test]
    fn register_offsets_match_optee_stm32_gpio_c() {
        // core/drivers/stm32_gpio.c:41-49
        assert_eq!(
            [r::MODER, r::OTYPER, r::OSPEEDR, r::PUPDR, r::IDR, r::ODR, r::BSRR, r::AFRL, r::AFRH],
            [0x00, 0x04, 0x08, 0x0c, 0x10, 0x14, 0x18, 0x20, 0x24]
        );
        // :50 GPIO_SECR_OFFSET (SECCFGR), :51 GPIO_PRIVCFGR_OFFSET
        assert_eq!((r::SECCFGR, r::PRIVCFGR), (0x30, 0x34));
        // :53 GPIO_CIDCFGR(x) = 0x50 + 8 x, :54 GPIO_SEMCR(x) = 0x54 + 8 x
        assert_eq!((r::cidcfgr(3), r::semcr(3)), (0x50 + 24, 0x54 + 24));
    }

    #[test]
    fn rif_fields_match_optee() {
        // core/include/drivers/stm32_rif.h:17-22 CFEN bit 0, SCID shift 4,
        // SEMEN bit 1, SEMWL shift 16; :27-29 SEMCR MUTEX bit 0; stm32_gpio.c:
        // 61 SEMWL 23:16. The SCID width is 2 bits for GPIO's 3 CIDs
        // (core/drivers/firewall/stm32_rif.c:13-32 get_scid_mask(3) = 5:4);
        // OP-TEE's stm32_gpio.c:62,85 and stm32_rif.h:18,29 say 6:4, Linux
        // drivers/pinctrl/stm32/pinctrl-stm32.c:69,73 says GENMASK(5, 4).
        // Corrected 6:4 -> 5:4.
        assert_eq!((r::CID_CFEN, r::CID_SEMEN), (1, 2));
        assert_eq!((r::CID_SCID_MASK, r::CID_SEMWL_MASK), (0x30, 0x00ff_0000));
        assert_eq!((r::SEM_MUTEX, r::SEM_SCID_MASK), (1, 0x30));
        // core/include/dt-bindings/soc/stm32mp25-rif.h:11 RIF_CID1 = 1
        assert_eq!(CID_A35, 1);
    }

    #[test]
    fn access_follows_optee_pin_is_accessible() {
        // core/drivers/stm32_gpio.c:462-475 with core/include/drivers/
        // stm32_rif.h:73-97: CFEN clear -> ok; SCID == CID1 with SEMEN clear
        // -> ok; SEMEN set and CID1 in SEMWL -> take the semaphore, which
        // core/drivers/firewall/stm32_rif.c:51-54 grants when MUTEX is clear
        // or its SCID is CID1. SECCFGR is read separately (stm32_gpio.c:502).
        let m = Bank::default();
        m.set(r::cidcfgr(0), r::CID_CFEN | r::CID_SEMEN | (1 << 4) | (4 << 16));
        // SEMEN set: static SCID does not grant, CID1 is not in SEMWL.
        assert_eq!(access(&m, B, 0, CID_A35), Err(Error::ForeignCid));
        m.set(r::cidcfgr(1), r::CID_CFEN | r::CID_SEMEN | (2 << 16));
        m.set(r::semcr(1), r::SEM_MUTEX | (1 << 4));
        assert_eq!(access(&m, B, 1, CID_A35), Ok(()));
        m.set(r::cidcfgr(2), r::CID_CFEN | (1 << 4));
        assert_eq!(access(&m, B, 2, CID_A35), Ok(()));
    }

    #[test]
    fn offsets_and_rif_match_linux_pinctrl_stm32_c() {
        // drivers/pinctrl/stm32/pinctrl-stm32.c:41-56 MODER 0x00, TYPER 0x04,
        // SPEEDR 0x08, PUPDR 0x0c, IDR 0x10, ODR 0x14, BSRR 0x18, AFRL 0x20,
        // AFRH 0x24, SECCFGR 0x30, CIDCFGR(x) 0x50 + 8x, SEMCR(x) 0x54 + 8x;
        // :67-73 CFEN bit 0, SEMEN bit 1, SCID GENMASK(5, 4), SEMWL of CID1
        // BIT(16 + 1), MUTEX bit 0, SEMCID GENMASK(5, 4); :39 CID1 = 1.
        assert_eq!(
            [r::MODER, r::OTYPER, r::OSPEEDR, r::PUPDR, r::IDR, r::ODR, r::BSRR, r::AFRL, r::AFRH, r::SECCFGR],
            [0x00, 0x04, 0x08, 0x0c, 0x10, 0x14, 0x18, 0x20, 0x24, 0x30]
        );
        assert_eq!((r::cidcfgr(15), r::semcr(15)), (0x50 + 8 * 15, 0x54 + 8 * 15));
        assert_eq!((r::CID_SCID_MASK, r::SEM_SCID_MASK), (0x30, 0x30));
        // :263-283 stm32_gpio_rif_valid / :285-322 acquire_semaphore: a held
        // semaphore of another SCID refuses, a free one is taken (the module
        // does not take it: the secure side owns RIF).
        let m = Bank::default();
        m.set(r::cidcfgr(0), r::CID_CFEN | r::CID_SEMEN | (2 << 16));
        m.set(r::semcr(0), r::SEM_MUTEX | (2 << 4));
        assert_eq!(access(&m, B, 0, CID_A35), Err(Error::ForeignCid));
        m.set(r::semcr(0), r::SEM_MUTEX | (1 << 4));
        assert_eq!(access(&m, B, 0, CID_A35), Ok(()));
    }

    #[test]
    fn set_mode_writes_afr_for_every_mode_like_stm32_pmx_set_mode() {
        // pinctrl-stm32.c:1033-1070 stm32_pmx_set_mode writes the AFR nibble
        // (alt 0 for gpio_set_direction, :1134) before MODER, whatever the
        // mode. Corrected: the module wrote AFR only for alternate.
        let m = Bank::default();
        m.set(r::AFRL, 0x0000_0900);
        configure_input(&m, B, 2, Pull::None, CID_A35).unwrap();
        assert_eq!(m.log(), vec![(r::AFRL, 0), (r::MODER, 0), (r::PUPDR, 0)]);
    }

    #[test]
    fn bank_bases_match_linux_dtsi() {
        // arch/arm64/boot/dts/st/stm32mp251.dtsi:2759 gpioa@44240000 ... :2879
        // gpiok@442e0000, :2960 gpioz@46200000 (pinctrl_z at :2943).
        assert_eq!(bank_base('A'), Some(0x4424_0000));
        assert_eq!(bank_base('K'), Some(0x442e_0000));
        assert_eq!(bank_base('Z'), Some(0x4620_0000));
    }

    #[test]
    fn bank_bases_match_optee_stm32mp251_dtsi() {
        // core/arch/arm/dts/stm32mp251.dtsi:1081 gpioa 0x44240000, :1093 gpiob
        // 0x44250000, :1105 gpioc, :1117 gpiod, :1129 gpioe, :1141 gpiof
        // 0x44290000, :1153 gpiog, :1165 gpioh, :1177 gpioi, :1189 gpioj,
        // :1201 gpiok 0x442e0000, :1265 gpioz 0x46200000.
        let want = [
            ('A', 0x4424_0000),
            ('B', 0x4425_0000),
            ('C', 0x4426_0000),
            ('D', 0x4427_0000),
            ('E', 0x4428_0000),
            ('F', 0x4429_0000),
            ('G', 0x442a_0000),
            ('H', 0x442b_0000),
            ('I', 0x442c_0000),
            ('J', 0x442d_0000),
            ('K', 0x442e_0000),
            ('Z', 0x4620_0000),
        ];
        for (l, a) in want {
            assert_eq!(bank_base(l), Some(a), "GPIO{l}");
        }
    }

    #[test]
    fn i2c_open_drain_af_pin_order() {
        // pin 5, AF9, open drain, pull-up; the neighbours' bits survive.
        let m = Bank::default();
        m.set(r::AFRL, 0x1111_1111);
        m.set(r::MODER, 0xffff_ffff);
        m.set(r::OSPEEDR, 0xffff_ffff);
        configure_af(&m, B, 5, 9, OType::OpenDrain, Speed::Low, Pull::Up, CID_A35).unwrap();
        // access() only reads; then AFR, MODER, OTYPER, OSPEEDR, PUPDR.
        assert_eq!(
            m.log(),
            vec![
                (r::AFRL, 0x1191_1111),
                (r::MODER, (!(3u32 << 10)) | (2 << 10)),
                (r::OTYPER, 1 << 5),
                (r::OSPEEDR, !(3u32 << 10)),
                (r::PUPDR, 1 << 10),
            ]
        );
    }

    #[test]
    fn af_on_a_high_pin_uses_afrh() {
        let m = Bank::default();
        configure_af(&m, B, 10, 0xa, OType::PushPull, Speed::VeryHigh, Pull::None, CID_A35).unwrap();
        assert_eq!(m.get(r::AFRH), 0xa << 8);
        assert_eq!(m.get(r::AFRL), 0);
        assert_eq!(m.get(r::OSPEEDR), 3 << 20);
        assert_eq!(configure_af(&m, B, 3, 16, OType::PushPull, Speed::Low, Pull::None, CID_A35), Err(Error::Range));
        assert_eq!(configure_af(&m, B, 16, 1, OType::PushPull, Speed::Low, Pull::None, CID_A35), Err(Error::Range));
    }

    #[test]
    fn output_pin_level_before_direction() {
        let m = Bank::default();
        configure_output(&m, B, 3, true, OType::PushPull, Speed::Low, Pull::None, CID_A35).unwrap();
        assert_eq!(
            m.log(),
            vec![(r::BSRR, 1 << 3), (r::AFRL, 0), (r::MODER, 1 << 6), (r::OTYPER, 0), (r::OSPEEDR, 0), (r::PUPDR, 0)]
        );
        clear(&m, B, 3);
        set(&m, B, 4);
        assert_eq!(&m.log()[6..], &[(r::BSRR, 1 << 19), (r::BSRR, 1 << 4)]);
        m.set(r::IDR, 1 << 7);
        assert!(read(&m, B, 7) && !read(&m, B, 6));
    }

    #[test]
    fn secure_pin_is_refused_and_nothing_written() {
        let m = Bank::default();
        m.set(r::SECCFGR, 1 << 5);
        let e = configure_af(&m, B, 5, 9, OType::OpenDrain, Speed::Low, Pull::Up, CID_A35);
        assert_eq!(e, Err(Error::Secure));
        assert!(m.log().is_empty());
        assert_eq!(access(&m, B, 6, CID_A35), Ok(()));
    }

    #[test]
    fn foreign_cid_is_refused() {
        let m = Bank::default();
        // CFEN, no semaphore, SCID 2 (the M33's)
        m.set(r::cidcfgr(4), r::CID_CFEN | (2 << 4));
        assert_eq!(access(&m, B, 4, CID_A35), Err(Error::ForeignCid));
        assert_eq!(access(&m, B, 4, 2), Ok(()));
        // CFEN clear: anyone
        assert_eq!(access(&m, B, 5, CID_A35), Ok(()));
        // CFEN | SEMEN, whitelist {1}: ok while free, refused when held by 2
        m.set(r::cidcfgr(6), r::CID_CFEN | r::CID_SEMEN | (2 << 16));
        assert_eq!(access(&m, B, 6, CID_A35), Ok(()));
        m.set(r::semcr(6), r::SEM_MUTEX | (2 << 4));
        assert_eq!(access(&m, B, 6, CID_A35), Err(Error::ForeignCid));
        // whitelist without bit 1
        m.set(r::cidcfgr(7), r::CID_CFEN | r::CID_SEMEN | (4 << 16));
        assert_eq!(access(&m, B, 7, CID_A35), Err(Error::ForeignCid));
    }
}
