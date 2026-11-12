// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The host tests' model of the STM32MP25's I2C2 controller with the DK's
//! ADV7535 and its EDID on the bus (tests only), at the real I2C2 base.
//!
//! Like [`crate::model_hdmi`] it records every access in order and answers
//! only what a polled master reads, as far as RM0457's I2C chapter and Linux
//! drivers/i2c/busses/i2c-stm32f7.c say the silicon does. Offsets and bit
//! positions are restated here (not imported from `ip::stm32_i2c`), so a wrong
//! constant in the driver cannot agree with itself. An access the silicon
//! would not take fails the test (a panic naming it):
//! - `TIMINGR` written with `PE` high (RM0457: writable only with PE = 0);
//! - `START` with `PE` low, with a flag (`STOPF`, `NACKF`) still uncleared,
//!   or in the middle of a message (only the idle bus and `TC` accept a
//!   (re)start); `STOP` outside `TC`;
//! - `TXDR` written with no byte due (`TXIS` clear), `RXDR` read with no byte
//!   received (`RXNE` clear), `TXDR` read, `RXDR` written;
//! - byte accesses to anything but `TXDR` / `RXDR`, 32-bit ones to them;
//! - `CR2` with `RELOAD`, `ADD10` or `NBYTES` = 0 (not modelled);
//! - an offset the model does not have, a write to a read-only register.
//!
//! Message-by-message behaviour: `START` latches `SADD`, `RD_WRN`, `NBYTES`
//! and `AUTOEND`. An address no device answers sets `NACKF` and, as the
//! hardware does without `AUTOEND` too, ends the transfer with a STOP
//! (`STOPF` set, `BUSY` clear, `TXE` set). A write message offers `TXIS` per
//! byte; after the last one `AUTOEND` stops (`STOPF`), otherwise `TC` waits
//! for the repeated start. A read message offers `RXNE` per byte, the same
//! end. `PE` cleared returns the state machine and the flags to idle; `TIMINGR`
//! survives it.
//!
//! The devices: [`Adv7535`] answers the main map at 0x3d (fixed, the DK's
//! strap) and the EDID, packet and CEC maps at the addresses its own main
//! registers 0x43, 0x45 and 0xe1 hold (reset values 0x7e, 0x70 and 0x78, so
//! 0x3f, 0x38 and 0x3c: ST's stm32mp257f-dk.dts :428-431, Linux
//! adv7511.h :56-58 and adv7511_drv.c :1282-1299). Every map is a register
//! file with a pointer: a write message's first byte sets it, further bytes
//! store and advance it, a read message returns and advances it. The main map
//! has the chip revision at 0x00 (0x14, which ST's U-Boot expects,
//! board/st/stm32mp2/stm32mp2.c :249-272) and the status at 0x42 (HPD bit 6,
//! monitor sense bit 5, adv7511.h :55, :155), both read-only. The EDID map
//! serves the blob: byte `0xc4 x 256 + pointer` (`EDID_SEGMENT`, the
//! four 64-byte reads of adv7511_drv.c :536-592), zeros past its end.
//!
//! Two things are configurable on [`Adv7535Config`]: an absent address NACKs
//! on its own (nothing answers it), and `in_reset` makes the chip answer
//! nothing until [`Adv7535Handle::set_reset`]`(false)` (pre-study open
//! question 5: PB6 is never driven on a DK by ST's chain). A reset pulse
//! returns the register files to their reset values.
//!
//! Assumed, not read from a datasheet: the ADV7535's `0xf5`/`0xf6` chip ID
//! (0x75 0x35) and `POWER` (0x41) reset value 0x50 (power-down set, as the
//! ADV7511 documents); the driver under test reads neither.
//!
//! Use: `let (bus, adv) = Mp25I2c::dk(Adv7535Config::default());`, pass
//! `&bus` as the [`crate::mmio::Mmio`] with [`BASE`], keep `adv` to change
//! HPD, the EDID or the reset line and to read back what the chip stored. A
//! further device implements [`I2cDevice`] and goes through [`Mp25I2c::attach`].

use crate::mmio::Mmio;
use std::cell::RefCell;
use std::rc::Rc;

/// I2C2 on the MP25 (stm32mp251.dtsi, RM0457 memory map).
pub const BASE: usize = 0x4013_0000;

// Offsets and bits, restated from RM0457.
const CR1: u32 = 0x00;
const CR2: u32 = 0x04;
const TIMINGR: u32 = 0x10;
const ISR: u32 = 0x18;
const ICR: u32 = 0x1c;
const RXDR: u32 = 0x24;
const TXDR: u32 = 0x28;

const CR1_PE: u32 = 1 << 0;
const CR2_RD_WRN: u32 = 1 << 10;
const CR2_ADD10: u32 = 1 << 11;
const CR2_START: u32 = 1 << 13;
const CR2_STOP: u32 = 1 << 14;
const CR2_RELOAD: u32 = 1 << 24;
const CR2_AUTOEND: u32 = 1 << 25;

const ISR_TXE: u32 = 1 << 0;
const ISR_TXIS: u32 = 1 << 1;
const ISR_RXNE: u32 = 1 << 2;
const ISR_NACKF: u32 = 1 << 4;
const ISR_STOPF: u32 = 1 << 5;
const ISR_TC: u32 = 1 << 6;
const ISR_BUSY: u32 = 1 << 15;

const ICR_NACKCF: u32 = 1 << 4;
const ICR_STOPCF: u32 = 1 << 5;
/// The clear bits of the flags the model has (BERR, ARLO never occur).
const ICR_KNOWN: u32 = ICR_NACKCF | ICR_STOPCF | (1 << 8) | (1 << 9);

/// One access to the controller's registers, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    R(u32, u32),
    W(u32, u32),
}

/// One event on the bus wires, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bus {
    /// A (re)start with the address byte acknowledged.
    Start {
        addr: u8,
        read: bool,
        nbytes: u8,
    },
    /// A (re)start nobody acknowledged; the controller stops by itself.
    Nack(u8),
    /// A data byte the target took.
    Write(u8, u8),
    /// A data byte the target sent.
    Read(u8, u8),
    Stop,
}

/// A target on the bus. `addr` is the 7-bit address the message used.
pub trait I2cDevice {
    fn acks(&self, addr: u8) -> bool;
    fn start(&mut self, addr: u8, read: bool);
    /// A byte written to the target; `false` is a NACK of the byte.
    fn write(&mut self, addr: u8, byte: u8) -> bool;
    fn read(&mut self, addr: u8) -> u8;
    fn stop(&mut self);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase {
    Idle,
    Write(u32),
    Read(u32),
    Tc,
}

struct State {
    cr1: u32,
    cr2: u32,
    timingr: u32,
    flags: u32,
    phase: Phase,
    autoend: bool,
    /// The device and address of the message in flight.
    cur: Option<(usize, u8)>,
    devs: Vec<Box<dyn I2cDevice>>,
    log: Vec<Access>,
    bus: Vec<Bus>,
}

/// The I2C2 controller with whatever is attached to its bus.
pub struct Mp25I2c(RefCell<State>);

impl Default for Mp25I2c {
    fn default() -> Self {
        Self::new()
    }
}

impl Mp25I2c {
    /// A bus with nothing on it.
    pub fn new() -> Mp25I2c {
        Mp25I2c(RefCell::new(State {
            cr1: 0,
            cr2: 0,
            timingr: 0,
            flags: 0,
            phase: Phase::Idle,
            autoend: false,
            cur: None,
            devs: vec![],
            log: vec![],
            bus: vec![],
        }))
    }

    /// The DK's bus: an ADV7535 (plus its EDID, packet and CEC maps).
    pub fn dk(cfg: Adv7535Config) -> (Mp25I2c, Adv7535Handle) {
        let bus = Mp25I2c::new();
        let adv = Adv7535Handle::new(cfg);
        bus.attach(Box::new(adv.clone()));
        (bus, adv)
    }

    pub fn attach(&self, dev: Box<dyn I2cDevice>) {
        self.0.borrow_mut().devs.push(dev);
    }

    /// Every register access so far.
    pub fn accesses(&self) -> Vec<Access> {
        self.0.borrow().log.clone()
    }

    /// Every bus event so far.
    pub fn bus(&self) -> Vec<Bus> {
        self.0.borrow().bus.clone()
    }

    pub fn clear_log(&self) {
        let mut s = self.0.borrow_mut();
        s.log.clear();
        s.bus.clear();
    }

    /// `TIMINGR` as last written.
    pub fn timingr(&self) -> u32 {
        self.0.borrow().timingr
    }

    /// `CR1` as last written.
    pub fn cr1(&self) -> u32 {
        self.0.borrow().cr1
    }
}

impl State {
    fn pe(&self) -> bool {
        self.cr1 & CR1_PE != 0
    }

    fn stop(&mut self) {
        if let Some((d, _)) = self.cur.take() {
            self.devs[d].stop();
        }
        self.bus.push(Bus::Stop);
        self.phase = Phase::Idle;
        self.flags |= ISR_STOPF;
    }

    /// The last byte of the message went: STOP with `AUTOEND`, else `TC`.
    fn message_done(&mut self) {
        if self.autoend {
            self.stop();
        } else {
            self.phase = Phase::Tc;
        }
    }

    fn start(&mut self, cr2: u32) {
        assert!(self.pe(), "i2c model: START with PE low");
        assert!(
            self.flags & (ISR_STOPF | ISR_NACKF) == 0,
            "i2c model: START with STOPF/NACKF not cleared (ISR flags {:#x})",
            self.flags
        );
        assert!(
            matches!(self.phase, Phase::Idle | Phase::Tc),
            "i2c model: START in the middle of a message ({:?})",
            self.phase
        );
        let addr = ((cr2 >> 1) & 0x7f) as u8;
        let read = cr2 & CR2_RD_WRN != 0;
        let n = (cr2 >> 16) & 0xff;
        assert!(n != 0, "i2c model: NBYTES = 0 is not modelled");
        self.autoend = cr2 & CR2_AUTOEND != 0;
        // A restart ends the previous message on the target.
        if let Some((d, _)) = self.cur.take() {
            self.devs[d].stop();
        }
        match self.devs.iter().position(|d| d.acks(addr)) {
            None => {
                self.bus.push(Bus::Nack(addr));
                self.flags |= ISR_NACKF | ISR_STOPF;
                self.phase = Phase::Idle;
            }
            Some(d) => {
                self.devs[d].start(addr, read);
                self.cur = Some((d, addr));
                self.bus.push(Bus::Start { addr, read, nbytes: n as u8 });
                self.phase = if read { Phase::Read(n) } else { Phase::Write(n) };
            }
        }
    }

    fn isr(&self) -> u32 {
        let mut v = self.flags;
        match self.phase {
            Phase::Idle => v |= ISR_TXE,
            Phase::Write(_) => v |= ISR_TXE | ISR_TXIS | ISR_BUSY,
            Phase::Read(_) => v |= ISR_RXNE | ISR_BUSY,
            Phase::Tc => v |= ISR_TC | ISR_BUSY,
        }
        v
    }
}

impl Mmio for Mp25I2c {
    fn read32(&self, a: usize) -> u32 {
        let mut s = self.0.borrow_mut();
        let off = (a - BASE) as u32;
        let v = match off {
            CR1 => s.cr1,
            CR2 => s.cr2,
            TIMINGR => s.timingr,
            ISR => s.isr(),
            _ => panic!("i2c model: 32-bit read of offset {off:#x} (ICR is write-only; RXDR/TXDR are bytes)"),
        };
        s.log.push(Access::R(off, v));
        v
    }

    fn write32(&self, a: usize, val: u32) {
        let mut s = self.0.borrow_mut();
        let off = (a - BASE) as u32;
        s.log.push(Access::W(off, val));
        match off {
            CR1 => {
                if val & CR1_PE == 0 {
                    // PE low: the state machine and the flags are reset.
                    if let Some((d, _)) = s.cur.take() {
                        s.devs[d].stop();
                    }
                    s.phase = Phase::Idle;
                    s.flags = 0;
                }
                s.cr1 = val;
            }
            CR2 => {
                assert!(val & (CR2_RELOAD | CR2_ADD10) == 0, "i2c model: RELOAD/ADD10 are not modelled ({val:#x})");
                assert!(
                    val & (CR2_START | CR2_STOP) != (CR2_START | CR2_STOP),
                    "i2c model: START and STOP in one CR2 write"
                );
                // START and STOP are request bits that clear themselves.
                s.cr2 = val & !(CR2_START | CR2_STOP);
                if val & CR2_START != 0 {
                    s.start(val);
                } else if val & CR2_STOP != 0 {
                    assert!(s.phase == Phase::Tc, "i2c model: STOP outside TC ({:?})", s.phase);
                    s.stop();
                }
            }
            TIMINGR => {
                assert!(!s.pe(), "i2c model: TIMINGR written with PE high");
                s.timingr = val;
            }
            ISR => assert!(val == ISR_TXE, "i2c model: ISR write {val:#x}; only TXE (flush) is writable"),
            ICR => {
                assert!(val & !ICR_KNOWN == 0, "i2c model: ICR write {val:#x} has bits the model lacks");
                if val & ICR_NACKCF != 0 {
                    s.flags &= !ISR_NACKF;
                }
                if val & ICR_STOPCF != 0 {
                    s.flags &= !ISR_STOPF;
                }
            }
            _ => panic!("i2c model: 32-bit write of offset {off:#x}"),
        }
    }

    fn read8(&self, a: usize) -> u8 {
        let mut s = self.0.borrow_mut();
        let off = (a - BASE) as u32;
        assert!(off == RXDR, "i2c model: byte read of offset {off:#x}; only RXDR is a byte register");
        let Phase::Read(left) = s.phase else {
            panic!("i2c model: RXDR read with no byte received ({:?})", s.phase);
        };
        let (d, addr) = s.cur.expect("read message without a device");
        let b = s.devs[d].read(addr);
        s.log.push(Access::R(RXDR, u32::from(b)));
        s.bus.push(Bus::Read(addr, b));
        if left == 1 {
            s.message_done();
        } else {
            s.phase = Phase::Read(left - 1);
        }
        b
    }

    fn write8(&self, a: usize, val: u8) {
        let mut s = self.0.borrow_mut();
        let off = (a - BASE) as u32;
        assert!(off == TXDR, "i2c model: byte write of offset {off:#x}; only TXDR is a byte register");
        s.log.push(Access::W(TXDR, u32::from(val)));
        let Phase::Write(left) = s.phase else {
            panic!("i2c model: TXDR written with no byte due (TXIS clear, {:?})", s.phase);
        };
        let (d, addr) = s.cur.expect("write message without a device");
        if !s.devs[d].write(addr, val) {
            s.flags |= ISR_NACKF;
            s.stop();
            return;
        }
        s.bus.push(Bus::Write(addr, val));
        if left == 1 {
            s.message_done();
        } else {
            s.phase = Phase::Write(left - 1);
        }
    }
}

// ---------------------------------------------------------------- ADV7535

/// Main map registers (adv7511.h, restated).
pub const REG_CHIP_REVISION: u8 = 0x00;
pub const REG_POWER: u8 = 0x41;
pub const REG_STATUS: u8 = 0x42;
pub const REG_EDID_I2C_ADDR: u8 = 0x43;
pub const REG_PACKET_I2C_ADDR: u8 = 0x45;
pub const REG_EDID_SEGMENT: u8 = 0xc4;
pub const REG_CEC_I2C_ADDR: u8 = 0xe1;
pub const STATUS_HPD: u8 = 1 << 6;
pub const STATUS_MONITOR_SENSE: u8 = 1 << 5;
/// The main map's address on the DK.
pub const ADDR_MAIN: u8 = 0x3d;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Map {
    Main,
    Edid,
    Packet,
    Cec,
}

#[derive(Clone, Debug)]
pub struct Adv7535Config {
    /// Held in reset at the start: answers nothing until `set_reset(false)`.
    pub in_reset: bool,
    /// A sink is plugged in (`STATUS` HPD and monitor sense).
    pub hpd: bool,
    /// Served by the EDID map.
    pub edid: Vec<u8>,
}

impl Default for Adv7535Config {
    fn default() -> Self {
        Adv7535Config { in_reset: false, hpd: true, edid: edid_block(EdidMode::P720) }
    }
}

pub struct Adv7535 {
    in_reset: bool,
    hpd: bool,
    edid: Vec<u8>,
    main: [u8; 256],
    packet: [u8; 256],
    cec: [u8; 256],
    ptr: [u8; 4],
    expect_ptr: bool,
    cur: Map,
    /// Every main-map store, (register, value), in order.
    main_writes: Vec<(u8, u8)>,
    edid_reads: usize,
}

fn reset_main() -> [u8; 256] {
    let mut r = [0u8; 256];
    r[REG_CHIP_REVISION as usize] = 0x14;
    r[REG_POWER as usize] = 0x50;
    r[REG_EDID_I2C_ADDR as usize] = 0x7e;
    r[REG_PACKET_I2C_ADDR as usize] = 0x70;
    r[REG_CEC_I2C_ADDR as usize] = 0x78;
    r[0xf5] = 0x75;
    r[0xf6] = 0x35;
    r
}

impl Adv7535 {
    fn new(cfg: Adv7535Config) -> Adv7535 {
        Adv7535 {
            in_reset: cfg.in_reset,
            hpd: cfg.hpd,
            edid: cfg.edid,
            main: reset_main(),
            packet: [0; 256],
            cec: [0; 256],
            ptr: [0; 4],
            expect_ptr: false,
            cur: Map::Main,
            main_writes: vec![],
            edid_reads: 0,
        }
    }

    fn map_of(&self, addr: u8) -> Option<Map> {
        if self.in_reset {
            return None;
        }
        if addr == ADDR_MAIN {
            Some(Map::Main)
        } else if addr == self.main[REG_EDID_I2C_ADDR as usize] >> 1 {
            Some(Map::Edid)
        } else if addr == self.main[REG_PACKET_I2C_ADDR as usize] >> 1 {
            Some(Map::Packet)
        } else if addr == self.main[REG_CEC_I2C_ADDR as usize] >> 1 {
            Some(Map::Cec)
        } else {
            None
        }
    }

    fn load(&mut self, map: Map, p: u8) -> u8 {
        match map {
            Map::Main if p == REG_STATUS => {
                if self.hpd {
                    STATUS_HPD | STATUS_MONITOR_SENSE
                } else {
                    0
                }
            }
            Map::Main => self.main[p as usize],
            Map::Edid => {
                self.edid_reads += 1;
                let i = usize::from(self.main[REG_EDID_SEGMENT as usize]) * 256 + usize::from(p);
                self.edid.get(i).copied().unwrap_or(0)
            }
            Map::Packet => self.packet[p as usize],
            Map::Cec => self.cec[p as usize],
        }
    }

    fn store(&mut self, map: Map, p: u8, b: u8) {
        match map {
            Map::Main => {
                self.main_writes.push((p, b));
                // The chip revision and the status are read-only.
                if p != REG_CHIP_REVISION && p != REG_STATUS {
                    self.main[p as usize] = b;
                }
            }
            Map::Edid => {}
            Map::Packet => self.packet[p as usize] = b,
            Map::Cec => self.cec[p as usize] = b,
        }
    }
}

impl I2cDevice for Adv7535 {
    fn acks(&self, addr: u8) -> bool {
        self.map_of(addr).is_some()
    }

    fn start(&mut self, addr: u8, read: bool) {
        self.cur = self.map_of(addr).expect("start on an address the chip does not answer");
        if !read {
            self.expect_ptr = true;
        }
    }

    fn write(&mut self, _addr: u8, byte: u8) -> bool {
        let i = self.cur as usize;
        if self.expect_ptr {
            self.ptr[i] = byte;
            self.expect_ptr = false;
        } else {
            self.store(self.cur, self.ptr[i], byte);
            self.ptr[i] = self.ptr[i].wrapping_add(1);
        }
        true
    }

    fn read(&mut self, _addr: u8) -> u8 {
        let i = self.cur as usize;
        let v = self.load(self.cur, self.ptr[i]);
        self.ptr[i] = self.ptr[i].wrapping_add(1);
        v
    }

    fn stop(&mut self) {
        self.expect_ptr = false;
    }
}

/// A shared handle on the ADV7535: the bus holds one, the test another.
#[derive(Clone)]
pub struct Adv7535Handle(Rc<RefCell<Adv7535>>);

impl Adv7535Handle {
    pub fn new(cfg: Adv7535Config) -> Adv7535Handle {
        Adv7535Handle(Rc::new(RefCell::new(Adv7535::new(cfg))))
    }

    /// The reset line (PB6, active low, so `true` is the pin low): asserting
    /// returns every register to its reset value and the chip answers
    /// nothing; releasing makes it answer again.
    pub fn set_reset(&self, asserted: bool) {
        let mut a = self.0.borrow_mut();
        if asserted {
            a.main = reset_main();
            a.packet = [0; 256];
            a.cec = [0; 256];
            a.ptr = [0; 4];
            a.expect_ptr = false;
        }
        a.in_reset = asserted;
    }

    pub fn in_reset(&self) -> bool {
        self.0.borrow().in_reset
    }

    pub fn set_hpd(&self, plugged: bool) {
        self.0.borrow_mut().hpd = plugged;
    }

    pub fn set_edid(&self, edid: Vec<u8>) {
        self.0.borrow_mut().edid = edid;
    }

    /// A main-map register as stored (not the computed `STATUS`).
    pub fn reg(&self, r: u8) -> u8 {
        self.0.borrow().main[r as usize]
    }

    pub fn packet(&self, r: u8) -> u8 {
        self.0.borrow().packet[r as usize]
    }

    pub fn cec(&self, r: u8) -> u8 {
        self.0.borrow().cec[r as usize]
    }

    /// Every main-map store, (register, value), in order: for pinning a
    /// register sequence against Linux's.
    pub fn main_writes(&self) -> Vec<(u8, u8)> {
        self.0.borrow().main_writes.clone()
    }

    /// Bytes the EDID map has served.
    pub fn edid_bytes_read(&self) -> usize {
        self.0.borrow().edid_reads
    }
}

impl I2cDevice for Adv7535Handle {
    fn acks(&self, addr: u8) -> bool {
        self.0.borrow().acks(addr)
    }
    fn start(&mut self, addr: u8, read: bool) {
        self.0.borrow_mut().start(addr, read)
    }
    fn write(&mut self, addr: u8, byte: u8) -> bool {
        self.0.borrow_mut().write(addr, byte)
    }
    fn read(&mut self, addr: u8) -> u8 {
        self.0.borrow_mut().read(addr)
    }
    fn stop(&mut self) {
        self.0.borrow_mut().stop()
    }
}

// ------------------------------------------------------------------- EDID

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EdidMode {
    /// 1280x720 at 60 Hz, 74.25 MHz (CEA-861 VIC 4).
    P720,
    /// 1920x1080 at 60 Hz, 148.5 MHz (CEA-861 VIC 16).
    P1080,
}

/// A valid 128-byte EDID 1.4 base block, the mode as the preferred detailed
/// timing, a monitor name descriptor, no extension, checksum correct.
pub fn edid_block(mode: EdidMode) -> Vec<u8> {
    // (pixel clock in 10 kHz, hactive, hblank, hfp, hsync, vactive, vblank,
    // vfp, vsync, width mm, height mm)
    let (clk, ha, hb, hfp, hs, va, vb, vfp, vs, wmm, hmm): (u16, u16, u16, u16, u16, u16, u16, u16, u16, u16, u16) =
        match mode {
            EdidMode::P720 => (7425, 1280, 370, 110, 40, 720, 30, 5, 5, 521, 293),
            EdidMode::P1080 => (14850, 1920, 280, 88, 44, 1080, 45, 4, 5, 521, 293),
        };
    let mut e = vec![0u8; 128];
    e[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
    // Manufacturer "YKG" (5-bit letters, big endian), product 1, week 1, 2026.
    e[8] = 0x65;
    e[9] = 0x67;
    e[10] = 1;
    e[16] = 1;
    e[17] = 36; // 2026 - 1990
    e[18] = 1;
    e[19] = 4;
    e[20] = 0x80; // digital
    e[21] = (wmm / 10) as u8;
    e[22] = (hmm / 10) as u8;
    e[23] = 0x78;
    e[24] = 0x0a; // RGB 4:4:4, preferred timing is the first descriptor
    for i in 0..8 {
        e[38 + 2 * i] = 0x01;
        e[39 + 2 * i] = 0x01;
    }
    let d = &mut e[54..72];
    d[0..2].copy_from_slice(&clk.to_le_bytes());
    d[2] = ha as u8;
    d[3] = hb as u8;
    d[4] = (((ha >> 8) << 4) | (hb >> 8)) as u8;
    d[5] = va as u8;
    d[6] = vb as u8;
    d[7] = (((va >> 8) << 4) | (vb >> 8)) as u8;
    d[8] = hfp as u8;
    d[9] = hs as u8;
    d[10] = (((vfp & 0xf) << 4) | (vs & 0xf)) as u8;
    d[11] = (((hfp >> 8) << 6) | ((hs >> 8) << 4) | ((vfp >> 4) << 2) | (vs >> 4)) as u8;
    d[12] = wmm as u8;
    d[13] = hmm as u8;
    d[14] = (((wmm >> 8) << 4) | (hmm >> 8)) as u8;
    d[17] = 0x1e; // digital separate sync, both positive
    e[72..77].copy_from_slice(&[0, 0, 0, 0xfc, 0]);
    e[77..90].copy_from_slice(b"YAKOGL\n      ");
    e[90..95].copy_from_slice(&[0, 0, 0, 0x10, 0]);
    e[127] = 0u8.wrapping_sub(e[..127].iter().fold(0u8, |a, &b| a.wrapping_add(b)));
    e
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::errno;
    use crate::ip::stm32_i2c::{self as drv, Config};

    /// The DK's I2C2 kernel clock per the bring-up note (FLEXGEN_12, 100 MHz),
    /// 400 kHz, SCL rise 108 ns and fall 12 ns (ST's stm32mp257f-dk.dts).
    fn cfg() -> Config {
        let mut c = Config::new(100_000_000, 400_000);
        c.rise_ns = 108;
        c.fall_ns = 12;
        c
    }

    fn up() -> (Mp25I2c, Adv7535Handle) {
        let (bus, adv) = Mp25I2c::dk(Adv7535Config::default());
        drv::init(&bus, BASE, &cfg()).unwrap();
        bus.clear_log();
        (bus, adv)
    }

    fn sum(b: &[u8]) -> u8 {
        b.iter().fold(0u8, |a, &x| a.wrapping_add(x))
    }

    #[test]
    fn edid_blocks_are_valid() {
        for m in [EdidMode::P720, EdidMode::P1080] {
            let e = edid_block(m);
            assert_eq!(e.len(), 128);
            assert_eq!(sum(&e), 0, "checksum");
            assert_eq!(&e[..8], &[0, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0]);
            let d = &e[54..72];
            let clk = u16::from_le_bytes([d[0], d[1]]);
            let ha = u16::from(d[2]) | (u16::from(d[4] >> 4) << 8);
            let va = u16::from(d[5]) | (u16::from(d[7] >> 4) << 8);
            match m {
                EdidMode::P720 => assert_eq!((clk, ha, va), (7425, 1280, 720)),
                EdidMode::P1080 => assert_eq!((clk, ha, va), (14850, 1920, 1080)),
            }
            // htotal x vtotal x 60 Hz is the pixel clock.
            let ht = u32::from(ha) + (u32::from(d[3]) | (u32::from(d[4] & 0xf) << 8));
            let vt = u32::from(va) + (u32::from(d[6]) | (u32::from(d[7] & 0xf) << 8));
            assert_eq!(ht * vt * 60, u32::from(clk) * 10_000);
        }
    }

    #[test]
    fn timing_register_as_programmed() {
        let (bus, _adv) = Mp25I2c::dk(Adv7535Config::default());
        let t = drv::init(&bus, BASE, &cfg()).unwrap();
        assert_eq!(bus.timingr(), t.timingr());
        assert_eq!(bus.timingr(), drv::compute_timing(&cfg()).unwrap().timingr());
        assert_ne!(bus.timingr(), 0);
        // PE is the last CR1 write and the only bit left set.
        assert_eq!(bus.cr1(), CR1_PE);
        let acc = bus.accesses();
        let w: Vec<_> = acc.iter().filter(|a| matches!(a, Access::W(..))).collect();
        assert_eq!(w[1], &Access::W(TIMINGR, t.timingr()));
        assert_eq!(*w.last().unwrap(), &Access::W(CR1, CR1_PE));
    }

    #[test]
    fn main_map_register_write_and_read() {
        let (bus, adv) = up();
        let mut rd = [0u8; 1];
        drv::write_read(&bus, BASE, 0x3d, &[0x00], &mut rd).unwrap();
        assert_eq!(rd[0], 0x14, "chip revision");
        // 0x98 is one of adv7533.c's fixed-patch registers: plain storage.
        drv::write(&bus, BASE, 0x3d, &[0x98, 0x03]).unwrap();
        drv::write_read(&bus, BASE, 0x3d, &[0x98], &mut rd).unwrap();
        assert_eq!(rd[0], 0x03);
        assert_eq!(adv.reg(0x98), 0x03);
        assert_eq!(adv.main_writes(), vec![(0x98, 0x03)]);
        // Two data bytes after the pointer go to consecutive registers.
        drv::write(&bus, BASE, 0x3d, &[0x10, 0xaa, 0xbb]).unwrap();
        assert_eq!((adv.reg(0x10), adv.reg(0x11)), (0xaa, 0xbb));
        // The chip revision is read-only.
        drv::write(&bus, BASE, 0x3d, &[0x00, 0xff]).unwrap();
        drv::write_read(&bus, BASE, 0x3d, &[0x00], &mut rd).unwrap();
        assert_eq!(rd[0], 0x14);
    }

    #[test]
    fn bus_events_of_a_write_read() {
        let (bus, _adv) = up();
        let mut rd = [0u8; 2];
        drv::write_read(&bus, BASE, 0x3d, &[0x00], &mut rd).unwrap();
        assert_eq!(rd, [0x14, 0x00]);
        assert_eq!(
            bus.bus(),
            vec![
                Bus::Start { addr: 0x3d, read: false, nbytes: 1 },
                Bus::Write(0x3d, 0x00),
                Bus::Start { addr: 0x3d, read: true, nbytes: 2 },
                Bus::Read(0x3d, 0x14),
                Bus::Read(0x3d, 0x00),
                Bus::Stop,
            ]
        );
    }

    #[test]
    fn hpd_status_is_readable_and_follows_the_sink() {
        let (bus, adv) = up();
        let mut rd = [0u8; 1];
        drv::write_read(&bus, BASE, 0x3d, &[REG_STATUS], &mut rd).unwrap();
        assert_eq!(rd[0] & STATUS_HPD, STATUS_HPD);
        adv.set_hpd(false);
        drv::write_read(&bus, BASE, 0x3d, &[REG_STATUS], &mut rd).unwrap();
        assert_eq!(rd[0] & (STATUS_HPD | STATUS_MONITOR_SENSE), 0);
    }

    #[test]
    fn edid_block_in_linux_chunks_of_64() {
        let (bus, adv) = up();
        // Linux: segment 0, then 64-byte reads, each with its offset byte.
        drv::write(&bus, BASE, 0x3d, &[REG_EDID_SEGMENT, 0]).unwrap();
        let mut edid = vec![];
        for off in [0u8, 64] {
            let mut c = [0u8; 64];
            drv::write_read(&bus, BASE, 0x3f, &[off], &mut c).unwrap();
            edid.extend_from_slice(&c);
        }
        assert_eq!(edid, edid_block(EdidMode::P720));
        assert_eq!(sum(&edid), 0);
        assert_eq!(adv.edid_bytes_read(), 128);
    }

    #[test]
    fn edid_block_in_one_read_and_the_pointer_runs_on() {
        let (bus, _adv) = up();
        let mut e = [0u8; 128];
        drv::write_read(&bus, BASE, 0x3f, &[0], &mut e).unwrap();
        assert_eq!(&e[..], &edid_block(EdidMode::P720)[..]);
        // A 255-byte message (the driver's most) runs past the 128 bytes: zeros.
        let mut big = [0xffu8; 255];
        drv::write_read(&bus, BASE, 0x3f, &[0], &mut big).unwrap();
        assert_eq!(&big[..128], &e[..]);
        assert!(big[128..].iter().all(|&b| b == 0));
        let mut a = [0u8; 4];
        drv::write_read(&bus, BASE, 0x3f, &[0x36], &mut a).unwrap();
        assert_eq!(a, e[0x36..0x3a]);
    }

    #[test]
    fn edid_blob_and_second_segment_are_configurable() {
        let mut blob = edid_block(EdidMode::P1080);
        blob.resize(256, 0);
        blob.extend(std::iter::repeat_n(0x5a, 256));
        let (bus, adv) = Mp25I2c::dk(Adv7535Config { edid: blob, ..Adv7535Config::default() });
        drv::init(&bus, BASE, &cfg()).unwrap();
        let mut b = [0u8; 4];
        drv::write_read(&bus, BASE, 0x3f, &[0x36], &mut b).unwrap();
        assert_eq!(u16::from_le_bytes([b[0], b[1]]), 14850);
        // Segment 1 starts at byte 256 of the blob.
        drv::write(&bus, BASE, 0x3d, &[REG_EDID_SEGMENT, 1]).unwrap();
        drv::write_read(&bus, BASE, 0x3f, &[0], &mut b).unwrap();
        assert_eq!(b, [0x5a; 4]);
        adv.set_edid(vec![]);
        drv::write_read(&bus, BASE, 0x3f, &[0], &mut b).unwrap();
        assert_eq!(b, [0; 4]);
    }

    #[test]
    fn absent_address_nacks_and_the_bus_recovers() {
        let (bus, _adv) = up();
        assert_eq!(drv::write(&bus, BASE, 0x50, &[1, 2]), Err(-errno::ENODEV));
        let mut rd = [0u8; 1];
        assert_eq!(drv::write_read(&bus, BASE, 0x1a, &[0], &mut rd), Err(-errno::ENODEV));
        assert_eq!(bus.bus(), vec![Bus::Nack(0x50), Bus::Nack(0x1a)]);
        drv::write_read(&bus, BASE, 0x3d, &[0], &mut rd).unwrap();
        assert_eq!(rd[0], 0x14);
    }

    #[test]
    fn four_addresses_answer_and_the_ancillary_maps_move() {
        let (bus, adv) = up();
        let mut rd = [0u8; 1];
        for a in [0x3d, 0x3f, 0x3c, 0x38] {
            drv::write_read(&bus, BASE, a, &[0], &mut rd).unwrap();
        }
        // CEC and packet maps are plain register files.
        drv::write(&bus, BASE, 0x3c, &[0x05, 0x77]).unwrap();
        drv::write(&bus, BASE, 0x38, &[0x06, 0x66]).unwrap();
        assert_eq!((adv.cec(5), adv.packet(6)), (0x77, 0x66));
        // The driver writes the ancillary addresses into the main map
        // (adv7511_drv.c :1282-1299); the EDID map follows at once.
        drv::write(&bus, BASE, 0x3d, &[REG_EDID_I2C_ADDR, 0x80]).unwrap();
        assert_eq!(drv::write_read(&bus, BASE, 0x3f, &[0], &mut rd), Err(-errno::ENODEV));
        drv::write_read(&bus, BASE, 0x40, &[0], &mut rd).unwrap();
        assert_eq!(rd[0], 0x00);
    }

    #[test]
    fn silent_until_the_reset_is_released() {
        let (bus, adv) = Mp25I2c::dk(Adv7535Config { in_reset: true, ..Adv7535Config::default() });
        drv::init(&bus, BASE, &cfg()).unwrap();
        let mut rd = [0u8; 1];
        for a in [0x3d, 0x3f, 0x3c, 0x38] {
            assert_eq!(drv::write_read(&bus, BASE, a, &[0], &mut rd), Err(-errno::ENODEV), "{a:#x}");
        }
        assert!(adv.in_reset());
        adv.set_reset(false);
        drv::write_read(&bus, BASE, 0x3d, &[0], &mut rd).unwrap();
        assert_eq!(rd[0], 0x14);
        // A pulse returns the registers to reset values.
        drv::write(&bus, BASE, 0x3d, &[0x41, 0x00]).unwrap();
        assert_eq!(adv.reg(REG_POWER), 0x00);
        adv.set_reset(true);
        assert_eq!(drv::write(&bus, BASE, 0x3d, &[0x41, 0x00]), Err(-errno::ENODEV));
        adv.set_reset(false);
        assert_eq!(adv.reg(REG_POWER), 0x50);
    }

    #[test]
    #[should_panic(expected = "TIMINGR written with PE high")]
    fn timingr_with_pe_high_fails() {
        let (bus, _adv) = up();
        bus.write32(BASE + TIMINGR as usize, 1);
    }

    #[test]
    #[should_panic(expected = "START with PE low")]
    fn start_with_pe_low_fails() {
        let bus = Mp25I2c::new();
        bus.write32(BASE + CR2 as usize, (0x3d << 1) | (1 << 16) | CR2_START);
    }

    #[test]
    #[should_panic(expected = "RXDR read with no byte received")]
    fn rxdr_read_with_nothing_received_fails() {
        let (bus, _adv) = up();
        let _ = bus.read8(BASE + RXDR as usize);
    }

    #[test]
    #[should_panic(expected = "TXDR written with no byte due")]
    fn txdr_write_outside_a_message_fails() {
        let (bus, _adv) = up();
        bus.write8(BASE + TXDR as usize, 0);
    }

    #[test]
    #[should_panic(expected = "START in the middle of a message")]
    fn start_mid_message_fails() {
        let (bus, _adv) = up();
        let cr2 = (0x3d << 1) | (2 << 16) | CR2_START;
        bus.write32(BASE + CR2 as usize, cr2);
        bus.write32(BASE + CR2 as usize, cr2);
    }
}
