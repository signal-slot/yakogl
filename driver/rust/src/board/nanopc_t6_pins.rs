// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the NanoPC-T6's start sets up itself so that every route ends in
//! the state Linux leaves (driver/docs/nanopc-t6-display.md, "What the
//! start sets up itself"): the USB Type-A ports' 5 V switch
//! ([`usb_vbus_on`]) and HDMI0's pin functions ([`hdmi0_pins`]). After a
//! reset U-Boot's `go` leaves both as the reset does: the switch's pin an
//! input (the ports unpowered) and the HDMI0 pins plain GPIOs, so the
//! controller sees no hot-plug (issue #11859, the words read on both
//! routes). After Linux (kexec, or the adopt of its picture) they are
//! already set, and the same words written again change nothing.
//!
//! The pins and their settings are Linux v6.18's device trees (facts and
//! file:line only); the register layout is `soc::rk3588_pinctrl`'s.
//!
//! - HDMI0: `pinctrl-0` of `hdmi@fde80000` names the groups
//!   `hdmim0_tx0_cec`, `_hpd`, `_scl`, `_sda` (rk3588-base.dtsi:1624-1625):
//!   GPIO4_C1, GPIO1_A5, GPIO4_B7, GPIO4_C0, each function 5
//!   (rk3588-base-pinctrl.dtsi:564-589). CEC and HPD take `pcfg_pull_none`,
//!   SCL `pcfg_pull_none_drv_level_5_smt`, SDA
//!   `pcfg_pull_none_drv_level_1_smt` (the same lines): no pull, and for SCL
//!   and SDA a drive level of 5 and 1 with schmitt on
//!   (rockchip-pinconf.dtsi:18-20, :336-340, :364-368).
//! - The Type-A ports' 5 V: `vbus5v0_usb`, a fixed regulator on GPIO4_B0,
//!   active high, its pin `usb5v_pwren` a GPIO with no pull
//!   (rk3588-nanopc-t6.dtsi:183-193, :693-695), the supply of `usbdp_phy1`
//!   (:1144-1147). Linux switches it on when that PHY starts, and under
//!   Armbian it is on (the kexec route's GPIO4 words, issue #11859). It is
//!   a board supply, not a part of the display: a sink or a touch panel
//!   powered from those ports needs it whatever the image shows.

use crate::mmio::Mmio;
use crate::soc::rk3588_pinctrl::{self as pc, Field, Pin};

/// A pin's setting as the device tree gives it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PinConf {
    pub pin: Pin,
    pub func: u32,
    pub pull: u32,
    /// `drive-strength`, when the tree names one.
    pub drive: Option<u32>,
    /// `input-schmitt-enable`, when the tree names it.
    pub schmitt: bool,
}

/// `hdmim0_tx0_cec`: GPIO4_C1, function 5, no pull
/// (rk3588-base-pinctrl.dtsi:564-568).
pub const HDMI0_CEC: PinConf =
    PinConf { pin: Pin::new(4, 2, 1), func: 5, pull: pc::PULL_NONE, drive: None, schmitt: false };
/// `hdmim0_tx0_hpd`: GPIO1_A5, function 5, no pull
/// (rk3588-base-pinctrl.dtsi:571-575).
pub const HDMI0_HPD: PinConf =
    PinConf { pin: Pin::new(1, 0, 5), func: 5, pull: pc::PULL_NONE, drive: None, schmitt: false };
/// `hdmim0_tx0_scl`: GPIO4_B7, function 5, no pull, drive level 5, schmitt
/// (rk3588-base-pinctrl.dtsi:578-582; rockchip-pinconf.dtsi:364-368).
pub const HDMI0_SCL: PinConf =
    PinConf { pin: Pin::new(4, 1, 7), func: 5, pull: pc::PULL_NONE, drive: Some(5), schmitt: true };
/// `hdmim0_tx0_sda`: GPIO4_C0, function 5, no pull, drive level 1, schmitt
/// (rk3588-base-pinctrl.dtsi:585-589; rockchip-pinconf.dtsi:336-340).
pub const HDMI0_SDA: PinConf =
    PinConf { pin: Pin::new(4, 2, 0), func: 5, pull: pc::PULL_NONE, drive: Some(1), schmitt: true };

/// HDMI0's pins in the order of its `pinctrl-0` (rk3588-base.dtsi:1624-1625).
pub const HDMI0_PINS: [PinConf; 4] = [HDMI0_CEC, HDMI0_HPD, HDMI0_SCL, HDMI0_SDA];

/// `usb5v_pwren`: GPIO4_B0 as a GPIO, no pull (rk3588-nanopc-t6.dtsi:693-695).
pub const VBUS5V0_USB: PinConf =
    PinConf { pin: Pin::new(4, 1, 0), func: pc::FUNC_GPIO, pull: pc::PULL_NONE, drive: None, schmitt: false };

/// The GPIO bank of [`VBUS5V0_USB`] (`gpio = <&gpio4 RK_PB0
/// GPIO_ACTIVE_HIGH>`, rk3588-nanopc-t6.dtsi:186).
pub const VBUS5V0_USB_BANK: usize = 4;

/// The block a word is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// The IOC ([`pc::IOC_BASE`]).
    Ioc,
    /// GPIO bank 4.
    Gpio4,
}

impl Block {
    pub const fn phys(self) -> usize {
        match self {
            Block::Ioc => pc::IOC_BASE,
            Block::Gpio4 => pc::GPIO_BASE[VBUS5V0_USB_BANK],
        }
    }
}

/// One write: `field` of the word at `field.off` in `block` set to `value`
/// (with the word's write mask, [`Field::word`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Write {
    pub block: Block,
    pub field: Field,
    pub value: u32,
}

impl Write {
    pub const fn word(self) -> u32 {
        self.field.word(self.value)
    }
}

/// The IOC writes of one pin: function, pull, then drive level and schmitt
/// when the tree names them. The fields are disjoint, so the order changes
/// nothing of the end state.
fn conf_writes(c: &PinConf, out: &mut [Option<Write>; 4]) {
    let ioc = |field: Option<Field>, value| {
        Some(Write { block: Block::Ioc, field: field.expect("the pin is in a bank rk3588_pinctrl covers"), value })
    };
    out[0] = ioc(pc::mux(c.pin), c.func);
    out[1] = ioc(pc::pull(c.pin), c.pull);
    out[2] = c.drive.and_then(|d| ioc(pc::drive(c.pin), d));
    out[3] = if c.schmitt { ioc(pc::schmitt(c.pin), 1) } else { None };
}

/// HDMI0's writes: each pin of [`HDMI0_PINS`] in turn.
pub fn hdmi0_writes() -> [Write; 12] {
    let mut out = [Write { block: Block::Ioc, field: Field { off: 0, shift: 0, width: 1 }, value: 0 }; 12];
    let mut n = 0;
    for c in &HDMI0_PINS {
        let mut w = [None; 4];
        conf_writes(c, &mut w);
        for x in w.into_iter().flatten() {
            out[n] = x;
            n += 1;
        }
    }
    assert_eq!(n, out.len());
    out
}

/// The 5 V switch's writes: the pin a GPIO with no pull, then the output
/// level high and only then the direction output, as Linux's
/// `direction_output` (gpio-rockchip.c:272-278), so the pin never drives
/// low.
pub fn usb_vbus_writes() -> [Write; 4] {
    let mut w = [None; 4];
    conf_writes(&VBUS5V0_USB, &mut w);
    let gpio = |reg_l| Write { block: Block::Gpio4, field: pc::gpio_bit(reg_l, VBUS5V0_USB.pin.pin), value: 1 };
    [w[0].unwrap(), w[1].unwrap(), gpio(pc::GPIO_DR_L), gpio(pc::GPIO_DDR_L)]
}

/// The words a record holds at most.
pub const RECORD_WORDS: usize = 10;

/// What [`apply`] found and left: each word it wrote, once, in the order of
/// its first write.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Record {
    pub count: usize,
    /// The word's physical address.
    pub phys: [u32; RECORD_WORDS],
    /// The word before the first write, and after the last.
    pub found: [u32; RECORD_WORDS],
    pub now: [u32; RECORD_WORDS],
    /// Bit i: a field written to word i reads back other than written.
    pub differ: u32,
}

/// Write `writes` in order (`base` maps a block to its mapping), each word
/// read once before the first write and once after the last.
pub fn apply(m: &impl Mmio, base: impl Fn(Block) -> usize, writes: &[Write]) -> Record {
    let mut r = Record::default();
    let mut words = [(Block::Ioc, 0usize); RECORD_WORDS];
    let index = |words: &[(Block, usize)], w: &Write| words.iter().position(|&k| k == (w.block, w.field.off));
    for w in writes {
        if index(&words[..r.count], w).is_none() {
            assert!(r.count < RECORD_WORDS, "more words than a record holds");
            words[r.count] = (w.block, w.field.off);
            r.phys[r.count] = u32::try_from(w.block.phys() + w.field.off).expect("a 32-bit address");
            r.found[r.count] = m.read32(base(w.block) + w.field.off);
            r.count += 1;
        }
    }
    for w in writes {
        m.write32(base(w.block) + w.field.off, w.word());
    }
    for (i, &(b, off)) in words[..r.count].iter().enumerate() {
        r.now[i] = m.read32(base(b) + off);
    }
    for w in writes {
        let i = index(&words[..r.count], w).unwrap();
        if w.field.get(r.now[i]) != w.value {
            r.differ |= 1 << i;
        }
    }
    r
}

/// Switch the USB Type-A ports' 5 V on: [`usb_vbus_writes`] to the IOC
/// mapped at `ioc` and GPIO bank 4 at `gpio4`. Returns the record and the
/// pin's level read back from the bank's `EXT_PORT` after the writes (1: the
/// switch is driven on).
pub fn usb_vbus_on(m: &impl Mmio, ioc: usize, gpio4: usize) -> (Record, u32) {
    let base = |b| match b {
        Block::Ioc => ioc,
        Block::Gpio4 => gpio4,
    };
    let r = apply(m, base, &usb_vbus_writes());
    let level = m.read32(gpio4 + pc::GPIO_EXT_PORT) >> VBUS5V0_USB.pin.pin & 1;
    (r, level)
}

/// Give the HDMI0 pins their functions and settings: [`hdmi0_writes`] to the
/// IOC mapped at `ioc`.
pub fn hdmi0_pins(m: &impl Mmio, ioc: usize) -> Record {
    let base = |b| match b {
        Block::Ioc => ioc,
        Block::Gpio4 => unreachable!("HDMI0's writes are IOC words"),
    };
    apply(m, base, &hdmi0_writes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::BTreeMap;

    /// Registers that take a write mask in bits 31:16 (the IOC's words and
    /// a GPIO bank's `DR`/`DDR`); bits 31:16 read 0. `EXT_PORT` follows
    /// `DR` where `DDR` makes a pin an output, and `inputs` elsewhere.
    #[derive(Default)]
    struct Soc {
        regs: RefCell<BTreeMap<usize, u32>>,
        ops: RefCell<Vec<(char, usize, u32)>>,
        inputs: u32,
        /// Bits that do not take a write (a stuck field, for the refusal).
        stuck: BTreeMap<usize, u32>,
    }

    const IOC: usize = 0x1000_0000;
    const GPIO4: usize = 0x2000_0000;

    impl Mmio for Soc {
        fn read32(&self, addr: usize) -> u32 {
            let regs = self.regs.borrow();
            let v = if addr == GPIO4 + pc::GPIO_EXT_PORT {
                let dr = regs.get(&GPIO4).copied().unwrap_or(0) | regs.get(&(GPIO4 + 4)).copied().unwrap_or(0) << 16;
                let ddr =
                    regs.get(&(GPIO4 + 8)).copied().unwrap_or(0) | regs.get(&(GPIO4 + 0xc)).copied().unwrap_or(0) << 16;
                dr & ddr | self.inputs & !ddr
            } else {
                regs.get(&addr).copied().unwrap_or(0)
            };
            self.ops.borrow_mut().push(('r', addr, v));
            v
        }
        fn write32(&self, addr: usize, val: u32) {
            self.ops.borrow_mut().push(('w', addr, val));
            let mask = (val >> 16) & !self.stuck.get(&addr).copied().unwrap_or(0);
            let mut regs = self.regs.borrow_mut();
            let old = regs.get(&addr).copied().unwrap_or(0);
            regs.insert(addr, old & !mask | val & mask);
        }
        fn read8(&self, _: usize) -> u8 {
            unreachable!("32-bit words only")
        }
        fn write8(&self, _: usize, _: u8) {
            unreachable!("32-bit words only")
        }
    }

    fn writes(s: &Soc) -> Vec<(usize, u32)> {
        s.ops.borrow().iter().filter(|o| o.0 == 'w').map(|o| (o.1, o.2)).collect()
    }

    #[test]
    fn the_vbus_words() {
        // IOC + 0x8088: GPIO4_B0's function (3:0) GPIO; + 0xc144: its pull
        // (1:0) none; then GPIO4 DR_L and DDR_L bit 8, in that order.
        let w: Vec<_> = usb_vbus_writes().iter().map(|w| (w.block.phys() + w.field.off, w.word())).collect();
        assert_eq!(
            w,
            [
                (0xfd5f_8088, 0x000f_0000),
                (0xfd5f_c144, 0x0003_0000),
                (0xfec5_0000, 0x0100_0100),
                (0xfec5_0008, 0x0100_0100)
            ]
        );
    }

    #[test]
    fn the_hdmi0_words() {
        let w: Vec<_> = hdmi0_writes().iter().map(|w| (w.block.phys() + w.field.off, w.word())).collect();
        assert_eq!(
            w,
            [
                // CEC, GPIO4_C1: function 5 at 7:4, no pull at 3:2
                (0xfd5f_8090, 0x00f0_0050),
                (0xfd5f_c148, 0x000c_0000),
                // HPD, GPIO1_A5: function 5 at 7:4, no pull at 11:10
                (0xfd5f_8024, 0x00f0_0050),
                (0xfd5f_9110, 0x0c00_0000),
                // SCL, GPIO4_B7: function 5 at 15:12, no pull at 15:14,
                // drive level 5 at 15:12, schmitt bit 7
                (0xfd5f_808c, 0xf000_5000),
                (0xfd5f_c144, 0xc000_0000),
                (0xfd5f_c08c, 0xf000_5000),
                (0xfd5f_c244, 0x0080_0080),
                // SDA, GPIO4_C0: function 5 at 3:0, no pull at 1:0, drive
                // level 1 at 3:0, schmitt bit 0
                (0xfd5f_8090, 0x000f_0005),
                (0xfd5f_c148, 0x0003_0000),
                (0xfd5f_c090, 0x000f_0001),
                (0xfd5f_c248, 0x0001_0001),
            ]
        );
    }

    /// The `go` route as read on the board (issue #11859): GPIO1_A's high
    /// function word 0, its pull word 0xd555, GPIO4 DR_L 0 and DDR_L 0x20.
    fn after_reset() -> Soc {
        let s = Soc::default();
        let mut r = s.regs.borrow_mut();
        r.insert(IOC + 0x8024, 0);
        r.insert(IOC + 0x9110, 0xd555);
        r.insert(GPIO4, 0);
        r.insert(GPIO4 + 8, 0x20);
        drop(r);
        s
    }

    #[test]
    fn hdmi0_after_a_reset_reads_what_linux_leaves() {
        let s = after_reset();
        let r = hdmi0_pins(&s, IOC);
        assert_eq!(r.differ, 0);
        assert_eq!(r.count, 10);
        assert_eq!(
            r.phys,
            [
                0xfd5f_8090,
                0xfd5f_c148,
                0xfd5f_8024,
                0xfd5f_9110,
                0xfd5f_808c,
                0xfd5f_c144,
                0xfd5f_c08c,
                0xfd5f_c244,
                0xfd5f_c090,
                0xfd5f_c248
            ]
        );
        // GPIO1_A5's fields as the kexec route reads them (issue #11859:
        // 0x550 and 0xc155): function 5 at 7:4, pull 00 at 11:10; the other
        // pins' fields are kept.
        assert_eq!((r.found[2], r.now[2]), (0, 0x050));
        assert_eq!((r.found[3], r.now[3]), (0xd555, 0xd155));
        // every word read once before the writes and once after them
        let ops = s.ops.borrow();
        assert!(ops[..10].iter().all(|o| o.0 == 'r') && ops[10..22].iter().all(|o| o.0 == 'w'));
        assert!(ops[22..].iter().all(|o| o.0 == 'r') && ops.len() == 32);
        drop(ops);
        assert_eq!(writes(&s).len(), 12);
    }

    #[test]
    fn vbus_after_a_reset_drives_the_pin_high() {
        let s = after_reset();
        let (r, level) = usb_vbus_on(&s, IOC, GPIO4);
        assert_eq!((r.count, r.differ, level), (4, 0, 1));
        assert_eq!(&r.phys[..4], [0xfd5f_8088u32, 0xfd5f_c144, 0xfec5_0000, 0xfec5_0008].as_slice());
        // the kexec route's DR_L and DDR_L have bit 8 set too
        assert_eq!((r.found[2], r.now[2]), (0, 0x100));
        assert_eq!((r.found[3], r.now[3]), (0x20, 0x120));
        // the level before the direction: DR_L is written first
        assert_eq!(&writes(&s)[2..], [(GPIO4, 0x0100_0100), (GPIO4 + 8, 0x0100_0100)]);
    }

    #[test]
    fn after_linux_the_words_are_unchanged() {
        // The kexec route's words (issue #11859): writing the table again
        // changes none of them.
        let s = Soc::default();
        let linux = [(IOC + 0x8024, 0x550), (IOC + 0x9110, 0xc155), (GPIO4, 0x4934), (GPIO4 + 8, 0x49f4)];
        s.regs.borrow_mut().extend(linux);
        let (v, level) = usb_vbus_on(&s, IOC, GPIO4);
        let h = hdmi0_pins(&s, IOC);
        assert_eq!((v.differ, h.differ, level), (0, 0, 1));
        for (addr, word) in linux {
            assert_eq!(s.regs.borrow()[&addr], word, "{addr:#x}");
        }
    }

    #[test]
    fn a_field_that_does_not_take_the_write_is_named() {
        let mut s = after_reset();
        // GPIO1_A5's function field stays 0
        s.stuck.insert(IOC + 0x8024, 0x00f0);
        let r = hdmi0_pins(&s, IOC);
        assert_eq!(r.differ, 1 << 2);
        // a pin that reads low although driven: the level says so
        let mut s = after_reset();
        s.stuck.insert(GPIO4 + 8, 0x0100);
        let (r, level) = usb_vbus_on(&s, IOC, GPIO4);
        assert_eq!((r.differ, level), (1 << 3, 0));
    }
}
