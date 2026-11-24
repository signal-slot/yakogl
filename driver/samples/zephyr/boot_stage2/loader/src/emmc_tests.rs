// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Host tests of the counted way back (emmc.rs) and the polled uSDHC
//! (usdhc.rs): the record and the verdict, the command sequence against a
//! model of an eMMC 5.1 card (JESD84-B51: the states, the R1 status, EXT_CSD
//! `PARTITION_CONFIG` with its persistent enable and ack fields and its
//! volatile access field, the boot partitions), every fault the sequence
//! answers, and the same sequence through a model of the uSDHC's registers.
//! The card starts as the Verdin iMX95's did on 2026-11-25: `PARTITION_CONFIG`
//! 0x48, `BOOT_SIZE_MULT` 0xfc (64512 sectors per boot partition), boot1 all
//! zero bytes.
use super::emmc::*;
use super::usdhc::{self, bits, reg, Regs};
use std::cell::RefCell;
use std::collections::HashMap;
use std::vec::Vec;

const S: u32 = COUNT_SECTOR;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum St {
    Idle,
    Ready,
    Ident,
    Stby,
    Tran,
}

/// The card. Faults: `ready_after` CMD1s before ready (`u32::MAX`: never);
/// `byte_mode`; `refuse_179`: CMD6 of these values sets SWITCH_ERROR and
/// changes nothing; `wp_boot1`: writes to boot1 fail with WP_VIOLATION;
/// `torn_write`: a write stores other bytes; `busy_forever` after a CMD6;
/// `silent_179`: CMD6 of these values is accepted and not applied.
struct Card {
    st: St,
    rca: u32,
    ext: [u8; BLOCK],
    boot: [HashMap<u32, [u8; BLOCK]>; 2],
    user: HashMap<u32, [u8; BLOCK]>,
    busy: u32,
    pending: u32,
    cmd1s: u32,
    ready_after: u32,
    byte_mode: bool,
    refuse_179: Vec<u8>,
    silent_179: Vec<u8>,
    wp_boot1: bool,
    torn_write: bool,
    busy_forever: bool,
    log: Vec<(u8, u32)>,
    writes_179: Vec<u8>,
    sector_writes: Vec<(u8, u32)>,
}

impl Card {
    fn board() -> Card {
        let mut ext = [0u8; BLOCK];
        ext[EXT_CSD_PARTITION_CONFIG] = 0x48;
        ext[EXT_CSD_BOOT_SIZE_MULT] = 0xfc;
        ext[177] = 0x12; // BOOT_BUS_CONDITIONS
        ext[192] = 8; // EXT_CSD_REV: 5.1
        Card {
            st: St::Idle,
            rca: 0,
            ext,
            boot: [HashMap::new(), HashMap::new()],
            user: HashMap::new(),
            busy: 0,
            pending: 0,
            cmd1s: 0,
            ready_after: 2,
            byte_mode: false,
            refuse_179: Vec::new(),
            silent_179: Vec::new(),
            wp_boot1: false,
            torn_write: false,
            busy_forever: false,
            log: Vec::new(),
            writes_179: Vec::new(),
            sector_writes: Vec::new(),
        }
    }
    fn config(&self) -> u8 {
        self.ext[EXT_CSD_PARTITION_CONFIG]
    }
    /// A power cycle, as the next boot: the card's volatile state is lost
    /// (the access field, the RCA), the persistent fields stay.
    fn power_cycle(&mut self) {
        self.st = St::Idle;
        self.rca = 0;
        self.ext[EXT_CSD_PARTITION_CONFIG] &= !PC_ACCESS_MASK;
        self.cmd1s = 0;
        self.busy = 0;
        self.pending = 0;
        self.log.clear();
        self.writes_179.clear();
        self.sector_writes.clear();
    }
    /// The boot partition the ROM reads next: 1 (boot0), 2 (boot1).
    fn enabled(&self) -> u8 {
        (self.config() >> 3) & 7
    }
    fn record(&self) -> Option<[u8; BLOCK]> {
        self.boot[1].get(&S).copied()
    }
    fn status(&self) -> u32 {
        let state = match self.st {
            St::Idle => 0,
            St::Ready => 1,
            St::Ident => 2,
            St::Stby => 3,
            St::Tran => 4,
        };
        let ready = if self.st == St::Tran && self.busy == 0 { R1_READY_FOR_DATA } else { 0 };
        (state << 9) | ready | self.pending
    }
    fn part(&mut self) -> Option<&mut HashMap<u32, [u8; BLOCK]>> {
        match self.config() & PC_ACCESS_MASK {
            0 => Some(&mut self.user),
            1 => Some(&mut self.boot[0]),
            2 => Some(&mut self.boot[1]),
            _ => None,
        }
    }
    fn dat0_busy(&mut self) -> bool {
        if self.busy_forever && self.busy > 0 {
            return true;
        }
        if self.busy > 0 {
            self.busy -= 1;
            return true;
        }
        false
    }
    fn exec(&mut self, index: u8, arg: u32, data: Data<'_>) -> Result<[u32; 4], u32> {
        self.log.push((index, arg));
        let mine = arg >> 16 == self.rca && self.rca != 0;
        let r1 = |c: &mut Card| {
            let s = c.status();
            c.pending = 0;
            Ok([s, 0, 0, 0])
        };
        match (index, self.st) {
            (0, _) => {
                self.st = St::Idle;
                self.rca = 0;
                self.ext[EXT_CSD_PARTITION_CONFIG] &= !PC_ACCESS_MASK;
                self.cmd1s = 0;
                Ok([0; 4])
            }
            (1, St::Idle) | (1, St::Ready) => {
                self.cmd1s += 1;
                let mut ocr = 0x00ff_8080;
                if !self.byte_mode {
                    ocr |= OCR_SECTOR_MODE;
                }
                if self.cmd1s > self.ready_after {
                    self.st = St::Ready;
                    ocr |= OCR_BUSY_DONE;
                }
                Ok([ocr, 0, 0, 0])
            }
            (2, St::Ready) => {
                self.st = St::Ident;
                Ok([0x1301_0030, 0x494d_3230, 0x4512_3456, 0x7800_0000])
            }
            (3, St::Ident) => {
                let s = self.status();
                self.rca = arg >> 16;
                self.st = St::Stby;
                Ok([s, 0, 0, 0])
            }
            (7, St::Stby) if mine => {
                let s = self.status();
                self.st = St::Tran;
                self.busy = 2;
                Ok([s, 0, 0, 0])
            }
            (13, _) if mine => r1(self),
            (8, St::Tran) => {
                let Data::Read(buf) = data else { panic!("CMD8 without a read buffer") };
                buf.copy_from_slice(&self.ext);
                r1(self)
            }
            (6, St::Tran) => {
                assert_eq!(arg >> 24, 3, "CMD6 not in write-byte mode: {arg:#x}");
                let index = ((arg >> 16) & 0xff) as usize;
                let value = ((arg >> 8) & 0xff) as u8;
                assert_eq!(index, EXT_CSD_PARTITION_CONFIG, "a CMD6 to EXT_CSD[{index}]: only 179 is ever written");
                let s = self.status();
                self.busy = 3;
                if self.refuse_179.contains(&value) {
                    self.pending |= R1_SWITCH_ERROR;
                } else {
                    self.writes_179.push(value);
                    if !self.silent_179.contains(&value) {
                        self.ext[index] = value;
                    }
                }
                Ok([s, 0, 0, 0])
            }
            (17, St::Tran) => {
                let Data::Read(buf) = data else { panic!("CMD17 without a read buffer") };
                let p = self.part().expect("access field");
                if arg >= 64512 {
                    return Ok([self.status() | (1 << 31), 0, 0, 0]);
                }
                *buf = p.get(&arg).copied().unwrap_or([0; BLOCK]);
                r1(self)
            }
            (24, St::Tran) => {
                let Data::Write(buf) = data else { panic!("CMD24 without data") };
                let access = self.config() & PC_ACCESS_MASK;
                if access == 2 && self.wp_boot1 {
                    return Ok([self.status() | (1 << 26), 0, 0, 0]);
                }
                let mut b = *buf;
                if self.torn_write {
                    b[100] ^= 0x5a;
                }
                self.sector_writes.push((access, arg));
                self.part().expect("access field").insert(arg, b);
                self.busy = 4;
                r1(self)
            }
            (idx, st) => panic!("CMD{idx} in state {st:?} (arg {arg:#x})"),
        }
    }
}

/// The card as a [`Bus`]: command level, no controller.
struct ModelBus<'a> {
    card: &'a mut Card,
    clocks: Vec<bool>,
}

impl Bus for ModelBus<'_> {
    fn command(&mut self, index: u8, arg: u32, resp: Resp, data: Data<'_>) -> Result<[u32; 4], u32> {
        // A card that is not ready does not answer a command for another state.
        if index == 1 {
            assert_eq!(resp, Resp::R3);
        }
        self.card.exec(index, arg, data)
    }
    fn busy(&mut self) -> bool {
        self.card.dat0_busy()
    }
    fn delay_us(&mut self, _us: u32) {}
    fn set_clock(&mut self, transfer: bool) -> Result<(), u32> {
        self.clocks.push(transfer);
        Ok(())
    }
}

fn install(card: &mut Card, remaining: u32, initial: u32) {
    card.boot[1].insert(S, record_bytes(&Record { remaining, initial }, S));
}

fn boot(card: &mut Card, requested: bool) -> (Outcome, Report) {
    card.power_cycle();
    let mut rep = Report::new();
    let mut bus = ModelBus { card, clocks: Vec::new() };
    let o = count_run(&mut bus, S, requested, &mut rep);
    assert!(
        bus.clocks.len() <= 2 && bus.clocks.iter().enumerate().all(|(i, &t)| t == (i == 1)),
        "identification clock, then the transfer clock: {:?}",
        bus.clocks
    );
    assert_eq!(rep.outcome, o as u32);
    (o, rep)
}

fn remaining(card: &Card) -> u32 {
    record_parse(&card.record().expect("a record"), S).expect("the record checks").remaining
}

#[test]
fn record_layout_and_crc() {
    let s = record_bytes(&Record { remaining: 3, initial: 3 }, S);
    assert_eq!(&s[..8], b"YKBOOTCT");
    assert_eq!(&s[8..24], &[1, 0, 0, 0, 3, 0, 0, 0, 3, 0, 0, 0, 0xee, 0xfb, 0, 0]);
    assert!(s[24..508].iter().all(|&b| b == 0));
    // The same bytes `tools/imx95-mkboot.py count --remaining 3` writes
    // (tools/imx95_mkboot_selftest.py checks this CRC from its side).
    assert_eq!(u32::from_le_bytes([s[508], s[509], s[510], s[511]]), 0xb841_4914);
    assert_eq!(record_parse(&s, S), Ok(Record { remaining: 3, initial: 3 }));
}

#[test]
fn records_that_do_not_check() {
    assert_eq!(record_parse(&[0; BLOCK], S), Err(RecordError::Magic), "boot1 as delivered");
    let good = record_bytes(&Record { remaining: 1, initial: 1 }, S);
    let mut b = good;
    b[12] = 0;
    assert_eq!(record_parse(&b, S), Err(RecordError::Crc), "a changed count without its CRC");
    assert_eq!(record_parse(&good, S + 1), Err(RecordError::Sector), "read from another sector");
    let refix = |mut b: [u8; BLOCK]| {
        let c = crate::crc32::crc32(&b[..508]);
        b[508..].copy_from_slice(&c.to_le_bytes());
        b
    };
    let mut b = good;
    b[8] = 2;
    assert_eq!(record_parse(&refix(b), S), Err(RecordError::Version));
    let mut b = good;
    b[300] = 1;
    assert_eq!(record_parse(&refix(b), S), Err(RecordError::Reserved));
    let b = record_bytes(&Record { remaining: 5, initial: 4 }, S);
    assert_eq!(record_parse(&b, S), Err(RecordError::Count));
}

#[test]
fn the_verdict() {
    let r = |remaining, initial| Ok(Record { remaining, initial });
    assert_eq!(decide(r(2, 3), false), Ok(Record { remaining: 1, initial: 3 }));
    assert_eq!(decide(r(1, 1), false), Ok(Record { remaining: 0, initial: 1 }));
    assert_eq!(decide(r(0, 1), false), Err(Why::Spent));
    assert_eq!(decide(Err(RecordError::Crc), false), Err(Why::Invalid));
    assert_eq!(decide(r(5, 5), true), Err(Why::Requested));
}

#[test]
fn partition_config_values() {
    // The board's 0x48 (boot0, ack) under Toradex's chain; 0x50 (boot1, ack)
    // after `mmc bootpart enable 2 1`; 0x52 with SPL's access to boot1 left.
    assert_eq!(access_value(0x48, PC_ACCESS_BOOT1), 0x4a);
    assert_eq!(access_value(0x4a, 0), 0x48);
    assert_eq!(access_value(0x52, 0), 0x50);
    assert_eq!(back_value(0x48), 0x48);
    assert_eq!(back_value(0x50), 0x48);
    assert_eq!(back_value(0x52), 0x48);
    assert_eq!(back_value(0x10), 0x08, "no ack found: none written");
    assert_eq!(back_value(0x7f) & PC_ACCESS_MASK, 0);
}

#[test]
fn boot1_as_delivered_goes_back_without_writing_a_sector() {
    let mut c = Card::board();
    let (o, rep) = boot(&mut c, false);
    assert_eq!(o, Outcome::Back);
    assert_eq!((rep.why, rep.record_error), (Why::Invalid as u32, RecordError::Magic as u32));
    assert_eq!(c.writes_179, [0x4a, 0x48, 0x48], "access boot1, access user, the way back");
    assert!(c.sector_writes.is_empty());
    assert_eq!((rep.found_config, rep.final_config), (0x48, 0x48));
    assert_eq!((rep.boot_sectors, rep.boot_wp, rep.cid0 >> 24), (64512, 0, 0x13));
    assert_eq!(c.enabled(), 1);
}

#[test]
fn a_count_of_n_boots_n_times_then_boot0() {
    let mut c = Card::board();
    install(&mut c, 3, 3);
    // direct-install: boot1 enabled with ack, as `mmc bootpart enable 2 1`.
    c.ext[EXT_CSD_PARTITION_CONFIG] = 0x50;
    for left in [2, 1, 0] {
        let (o, rep) = boot(&mut c, false);
        assert_eq!(o, Outcome::Boot, "{rep:?}");
        assert_eq!((rep.remaining_before, rep.remaining_after, rep.initial), (left + 1, left, 3));
        assert_eq!(remaining(&c), left, "the decremented record is on the card before the payload starts");
        assert_eq!(c.sector_writes, [(2, S)], "one write, to boot1's count sector");
        assert_eq!(c.writes_179, [0x52, 0x50], "access boot1 and back to the user area, boot1 still enabled");
        assert_eq!((c.config(), c.enabled()), (0x50, 2));
        let cmds: Vec<u8> = c.log.iter().map(|&(i, _)| i).collect();
        let i24 = cmds.iter().position(|&i| i == 24).unwrap();
        assert_eq!(cmds[i24 + 1..].iter().filter(|&&i| i == 17).count(), 1, "the record read back after the write");
    }
    let (o, rep) = boot(&mut c, false);
    assert_eq!((o, rep.why), (Outcome::Back, Why::Spent as u32));
    assert_eq!(c.writes_179, [0x52, 0x50, 0x48]);
    assert_eq!((c.config(), c.enabled(), rep.final_config), (0x48, 1, 0x48), "boot0 with the ack bit as found");
    assert!(c.sector_writes.is_empty());
    assert_eq!(remaining(&c), 0);
}

#[test]
fn spl_left_the_access_on_boot1() {
    let mut c = Card::board();
    install(&mut c, 1, 1);
    c.ext[EXT_CSD_PARTITION_CONFIG] = 0x52;
    c.st = St::Tran; // SPL's card, selected (with an RCA this loader does not know)
    c.rca = 1;
    let mut rep = Report::new();
    let o = count_run(&mut ModelBus { card: &mut c, clocks: Vec::new() }, S, false, &mut rep);
    assert_eq!(o, Outcome::Boot);
    assert_eq!(c.log[0], (0, 0), "CMD0 first: nothing of SPL's card state is used");
    assert_eq!(rep.found_config, 0x50, "CMD0 cleared the volatile access field");
    assert_eq!(c.config(), 0x50);
}

#[test]
fn no_ack_found_none_written() {
    let mut c = Card::board();
    install(&mut c, 0, 2);
    c.ext[EXT_CSD_PARTITION_CONFIG] = 0x10;
    let (o, rep) = boot(&mut c, false);
    assert_eq!((o, rep.why, c.config()), (Outcome::Back, Why::Spent as u32, 0x08));
}

#[test]
fn the_console_request_reads_and_writes_no_sector() {
    let mut c = Card::board();
    install(&mut c, 5, 5);
    c.ext[EXT_CSD_PARTITION_CONFIG] = 0x50;
    let (o, rep) = boot(&mut c, true);
    assert_eq!((o, rep.why), (Outcome::Back, Why::Requested as u32));
    assert!(!c.log.iter().any(|&(i, _)| i == 17 || i == 24));
    assert_eq!(c.writes_179, [0x48]);
    assert_eq!(remaining(&c), 5, "the count is left for the next install");
}

#[test]
fn the_request_before_the_jump() {
    let mut c = Card::board();
    install(&mut c, 2, 2);
    c.ext[EXT_CSD_PARTITION_CONFIG] = 0x50;
    assert_eq!(boot(&mut c, false).0, Outcome::Boot);
    let mut rep = Report::new();
    let o = back_run(&mut ModelBus { card: &mut c, clocks: Vec::new() }, &mut rep);
    assert_eq!((o, rep.why, rep.found_config, rep.final_config), (Outcome::Back, Why::Requested as u32, 0x50, 0x48));
    assert_eq!(remaining(&c), 1);
}

/// Every failure after the EXT_CSD read takes the way back, with the first
/// failure in the report; the payload is never started.
fn failure_goes_back(mut c: Card, stage: Stage) -> Card {
    let (o, rep) = boot(&mut c, false);
    assert_eq!((o, rep.why, rep.fail_stage), (Outcome::Back, Why::Failure as u32, stage as u32), "{rep:?}");
    assert_eq!((c.config(), c.enabled()), (0x48, 1));
    c
}

fn installed() -> Card {
    let mut c = Card::board();
    install(&mut c, 2, 2);
    c.ext[EXT_CSD_PARTITION_CONFIG] = 0x50;
    c
}

#[test]
fn failures_after_ext_csd_go_back() {
    let mut c = installed();
    c.refuse_179 = vec![0x52];
    failure_goes_back(c, Stage::AccessBoot1);

    let mut c = installed();
    c.wp_boot1 = true;
    let c = failure_goes_back(c, Stage::WriteRecord);
    assert_eq!(remaining(&c), 2);

    let mut c = installed();
    c.torn_write = true;
    failure_goes_back(c, Stage::VerifyRecord);

    let mut c = installed();
    c.refuse_179 = vec![0x50];
    let c = failure_goes_back(c, Stage::AccessUser);
    assert_eq!(remaining(&c), 1, "the count was taken before the failure");

    let mut c = installed();
    c.ext[EXT_CSD_BOOT_SIZE_MULT] = 0x10;
    let c = failure_goes_back(c, Stage::BootSize);
    assert!(!c.log.iter().any(|&(i, _)| i == 17));
}

#[test]
fn a_card_that_stays_busy_is_a_failure_not_a_hang() {
    let mut c = installed();
    c.busy_forever = true;
    let (o, rep) = boot(&mut c, false);
    // CMD7's busy already: before the EXT_CSD is read nothing is written.
    assert_eq!((o, rep.fail_stage), (Outcome::Failed, Stage::Cmd7 as u32));
    assert!(c.writes_179.is_empty() && c.sector_writes.is_empty());
}

#[test]
fn failures_before_ext_csd_write_nothing() {
    for (setup, stage) in [
        (Box::new(|c: &mut Card| c.ready_after = u32::MAX) as Box<dyn Fn(&mut Card)>, Stage::Cmd1),
        (Box::new(|c: &mut Card| c.byte_mode = true), Stage::Cmd1ByteMode),
    ] {
        let mut c = installed();
        setup(&mut c);
        let (o, rep) = boot(&mut c, false);
        assert_eq!((o, rep.fail_stage), (Outcome::Failed, stage as u32));
        assert!(c.writes_179.is_empty() && c.sector_writes.is_empty());
        assert_eq!(c.config(), 0x50, "boot1 stays enabled: the board is stranded (the note's table)");
    }
}

#[test]
fn a_way_back_that_does_not_take_is_failed() {
    let mut c = installed();
    install(&mut c, 0, 2);
    c.silent_179 = vec![0x48];
    let (o, rep) = boot(&mut c, false);
    assert_eq!(
        (o, rep.why, rep.fail_stage, rep.fail_code),
        (Outcome::Failed, Why::Spent as u32, Stage::BackVerify as u32, 0x50)
    );
    let mut c = installed();
    install(&mut c, 0, 2);
    c.refuse_179 = vec![0x48];
    let (o, rep) = boot(&mut c, false);
    assert_eq!((o, rep.fail_stage), (Outcome::Failed, Stage::Back as u32));
}

// ---- the uSDHC: the divider, then the same sequence through its registers ----

#[test]
fn the_sd_clock_divider() {
    // U-Boot's USDHC1 root: 400 MHz.
    assert_eq!(usdhc::divider(400_000_000, 400_000), (0x20, 15, 390_625));
    assert_eq!(usdhc::divider(400_000_000, 20_000_000), (1, 9, 20_000_000));
    // Unknown root: the assumed 1.6 GHz, at or below the targets.
    assert_eq!(usdhc::divider(1_600_000_000, 400_000).2, 390_625);
    assert_eq!(usdhc::divider(1_600_000_000, 20_000_000).2, 20_000_000);
    for rate in [24_000_000u32, 100_000_000, 200_000_000, 333_333_333, 400_000_000, 800_000_000, 1_600_000_000] {
        for target in [usdhc::IDENT_HZ, usdhc::TRANSFER_HZ] {
            let (fs, dvs, hz) = usdhc::divider(rate, target);
            let pre = if fs == 0 { 1 } else { fs * 2 };
            assert!(pre.is_power_of_two() && pre <= 256 && dvs <= 15);
            let div = pre as u64 * (dvs as u64 + 1);
            assert!((rate as u64).div_ceil(div) <= target as u64, "{rate} / {div} > {target}");
            assert_eq!(hz as u64, rate as u64 / div);
            // Nothing faster at or below the target.
            for p in [1u64, 2, 4, 8, 16, 32, 64, 128, 256] {
                for d in 1..=16u64 {
                    if (rate as u64).div_ceil(p * d) <= target as u64 {
                        assert!(p * d >= div, "{rate}->{target}: {p}x{d} beats {div}");
                    }
                }
            }
        }
    }
}

/// The uSDHC's registers as this module uses them, in front of a [`Card`].
/// The controller asserts what the module must do: the mode bits in
/// MIX_CTRL and not in CMD_XFR_TYP, one 512-byte block, little-endian 1-bit
/// mode, no interrupt signalled, the buffer used only after BRR / BWR.
struct UsdhcModel {
    r: RefCell<HashMap<usize, u32>>,
    card: RefCell<Card>,
    rx: RefCell<Vec<u32>>,
    tx: RefCell<Option<(u32, Vec<u32>)>>,
    data_left: RefCell<usize>,
    burst: RefCell<usize>,
    resets: RefCell<u32>,
}

impl UsdhcModel {
    fn new(card: Card) -> UsdhcModel {
        let m = UsdhcModel {
            r: RefCell::new(HashMap::new()),
            card: RefCell::new(card),
            rx: RefCell::new(Vec::new()),
            tx: RefCell::new(None),
            data_left: RefCell::new(0),
            burst: RefCell::new(0),
            resets: RefCell::new(0),
        };
        // SPL's state: VSELECT (1.8 V), the 8-bit bus, HS400 bits, the DLLs on.
        m.set(reg::VEND_SPEC, 0x2000_7809 | bits::VSELECT);
        m.set(reg::PROT_CTRL, 0x0000_0024);
        m.set(reg::MIX_CTRL, 0x0c00_0008);
        m.set(reg::DLL_CTRL, 0x7800_0001);
        m.set(reg::SYS_CTRL, 0x008e_0108);
        m
    }
    fn get(&self, off: usize) -> u32 {
        *self.r.borrow().get(&off).unwrap_or(&0)
    }
    fn set(&self, off: usize, v: u32) {
        self.r.borrow_mut().insert(off, v);
    }
    fn int(&self, bits: u32) {
        let v = self.get(reg::INT_STATUS) | bits;
        self.set(reg::INT_STATUS, v);
    }
    fn xfer(&self, xfr: u32) {
        assert_eq!(xfr & 0xffff, 0, "transfer-mode bits in CMD_XFR_TYP (the uSDHC takes them in MIX_CTRL)");
        assert_eq!(self.get(reg::PROT_CTRL), bits::EMODE_LE, "little-endian, 1-bit");
        assert_eq!(self.get(reg::INT_SIGNAL_EN), 0, "polled: no interrupt to the GIC");
        let index = ((xfr >> 24) & 0x3f) as u8;
        let arg = self.get(reg::CMD_ARG);
        let dpsel = xfr & bits::DPSEL != 0;
        let read = self.get(reg::MIX_CTRL) & bits::MIX_DTDSEL != 0;
        let rsptyp = (xfr >> 16) & 3;
        if dpsel {
            assert_eq!(self.get(reg::BLK_ATT), 512 | (1 << 16), "one 512-byte block");
        }
        let mut card = self.card.borrow_mut();
        let mut buf = [0u8; BLOCK];
        let r = if !dpsel {
            card.exec(index, arg, Data::None)
        } else if read {
            card.exec(index, arg, Data::Read(&mut buf))
        } else {
            // the data comes after the response: CMD24 is executed when the block is in
            *self.tx.borrow_mut() = Some((arg, Vec::new()));
            *self.burst.borrow_mut() = 0;
            drop(card);
            self.set(reg::CMD_RSP0, self.card.borrow().status());
            self.int(bits::CC | bits::BWR);
            return;
        };
        match r {
            Err(_) => self.int(bits::CTOE),
            Ok(rsp) => {
                if rsptyp == 1 {
                    // bits 127..8 shifted down by 8, as the uSDHC keeps them
                    let w = [rsp[0], rsp[1], rsp[2], rsp[3]];
                    self.set(reg::CMD_RSP3, w[0] >> 8);
                    self.set(reg::CMD_RSP2, (w[0] << 24) | (w[1] >> 8));
                    self.set(reg::CMD_RSP1, (w[1] << 24) | (w[2] >> 8));
                    self.set(reg::CMD_RSP0, (w[2] << 24) | (w[3] >> 8));
                } else {
                    self.set(reg::CMD_RSP0, rsp[0]);
                }
                let ok = index == 0 || rsptyp != 0;
                assert!(ok, "CMD{index} sent without a response type");
                self.int(bits::CC);
                if dpsel {
                    *self.rx.borrow_mut() =
                        buf.chunks(4).map(|w| u32::from_le_bytes([w[0], w[1], w[2], w[3]])).collect();
                    *self.data_left.borrow_mut() = 128;
                    *self.burst.borrow_mut() = 0;
                    self.int(bits::BRR);
                }
            }
        }
    }
}

impl Regs for UsdhcModel {
    fn read32(&self, off: usize) -> u32 {
        match off {
            reg::PRES_STATE => {
                let mut v = bits::SDSTB;
                if !self.card.borrow_mut().dat0_busy() {
                    v |= bits::DAT0;
                }
                if !self.rx.borrow().is_empty() {
                    v |= bits::BREN;
                }
                if self.tx.borrow().is_some() {
                    v |= bits::BWEN;
                }
                v
            }
            reg::DATA_BUFF_ACC_PORT => {
                assert_eq!(*self.burst.borrow(), 0, "data port read without BRR cleared first");
                let w = self.rx.borrow_mut().remove(0);
                let mut left = self.data_left.borrow_mut();
                *left -= 1;
                if (*left).is_multiple_of(usdhc::WML_WORDS) {
                    *self.burst.borrow_mut() = 1;
                    if *left == 0 {
                        self.int(bits::TC);
                    } else {
                        self.int(bits::BRR);
                    }
                }
                w
            }
            _ => self.get(off),
        }
    }
    fn write32(&self, off: usize, val: u32) {
        match off {
            reg::INT_STATUS => {
                let v = self.get(off) & !val;
                self.set(off, v);
                if val & (bits::BRR | bits::BWR) != 0 {
                    *self.burst.borrow_mut() = 0;
                }
            }
            reg::SYS_CTRL => {
                if val & bits::RSTA != 0 {
                    *self.resets.borrow_mut() += 1;
                    self.r.borrow_mut().clear();
                    self.set(reg::VEND_SPEC, 0x2000_7809);
                    self.set(reg::SYS_CTRL, 0x0080_800f);
                    return;
                }
                assert_ne!(val & (1 << 23), 0, "IPP_RST_N kept (the card's RST_n not driven)");
                self.set(off, val & !(bits::INITA | bits::RSTC | bits::RSTD));
            }
            reg::CMD_XFR_TYP => self.xfer(val),
            reg::DATA_BUFF_ACC_PORT => {
                assert_eq!(*self.burst.borrow(), 0, "data port write without BWR cleared first");
                let mut tx = self.tx.borrow_mut();
                let (arg, words) = tx.as_mut().expect("a data port write without a write command");
                words.push(val);
                if words.len().is_multiple_of(usdhc::WML_WORDS) {
                    *self.burst.borrow_mut() = 1;
                    if words.len() == 128 {
                        let mut b = [0u8; BLOCK];
                        for (i, w) in words.iter().enumerate() {
                            b[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
                        }
                        let arg = *arg;
                        *tx = None;
                        drop(tx);
                        let r = self.card.borrow_mut().exec(24, arg, Data::Write(&b));
                        let st = r.map(|r| r[0]).unwrap_or(0);
                        self.set(reg::CMD_RSP0, st);
                        if st & (1 << 26) != 0 {
                            self.int(bits::DCE);
                        } else {
                            self.int(bits::TC);
                        }
                    } else {
                        self.int(bits::BWR);
                    }
                }
            }
            _ => self.set(off, val),
        }
    }
}

fn boot_regs(card: Card, requested: bool) -> (Outcome, Report, UsdhcModel) {
    let m = UsdhcModel::new(card);
    m.card.borrow_mut().power_cycle();
    let mut rep = Report::new();
    let o = crate::count_on(&m, 400_000_000, S, requested, &mut rep, |_| {});
    (o, rep, m)
}

#[test]
fn through_the_registers_a_count_boots_then_goes_back() {
    let mut c = Card::board();
    install(&mut c, 1, 1);
    c.ext[EXT_CSD_PARTITION_CONFIG] = 0x50;
    let (o, rep, m) = boot_regs(c, false);
    assert_eq!(o, Outcome::Boot, "{rep:?}");
    assert_eq!((rep.rate_hz, rep.ident_hz, rep.transfer_hz), (400_000_000, 390_625, 20_000_000));
    assert_eq!(rep.cid0 >> 24, 0x13, "the CID's manufacturer id through the R2 layout");
    assert_eq!(*m.resets.borrow(), 1);
    assert_eq!(m.get(reg::VEND_SPEC) & bits::VSELECT, bits::VSELECT, "VSELECT as SPL left it");
    assert_eq!((m.get(reg::MIX_CTRL) & !bits::MIX_MODE_BITS, m.get(reg::DLL_CTRL)), (0, 0), "HS400 and the DLL off");
    let c = m.card.into_inner();
    assert_eq!(remaining(&c), 0);
    assert_eq!(c.config(), 0x50);

    let (o, rep, m) = boot_regs(c, false);
    assert_eq!((o, rep.why, rep.final_config), (Outcome::Back, Why::Spent as u32, 0x48));
    assert_eq!(m.card.borrow().enabled(), 1);
}

#[test]
fn through_the_registers_boot1_as_delivered_and_faults() {
    let (o, rep, m) = boot_regs(Card::board(), false);
    assert_eq!((o, rep.why), (Outcome::Back, Why::Invalid as u32));
    assert!(m.card.borrow().sector_writes.is_empty());

    let mut c = installed();
    c.wp_boot1 = true;
    let (o, rep, m) = boot_regs(c, false);
    assert_eq!(
        (o, rep.fail_stage, rep.fail_code),
        (Outcome::Back, Stage::WriteRecord as u32, crate::emmc::BusError::DataCrc as u32)
    );
    assert_eq!(m.card.borrow().config(), 0x48);

    let mut c = installed();
    c.ready_after = u32::MAX;
    let (o, rep, _) = boot_regs(c, false);
    assert_eq!((o, rep.fail_stage), (Outcome::Failed, Stage::Cmd1 as u32));

    // The request on a card left in TRAN by a count that booted.
    let (o, _, m) = boot_regs(installed(), false);
    assert_eq!(o, Outcome::Boot);
    let mut rep = Report::new();
    let o = crate::back_on(&m, 400_000_000, &mut rep, |_| {});
    assert_eq!((o, rep.final_config), (Outcome::Back, 0x48));
}

#[test]
fn a_controller_that_does_not_reset_is_failed() {
    struct Stuck;
    impl Regs for Stuck {
        fn read32(&self, off: usize) -> u32 {
            if off == reg::SYS_CTRL {
                bits::RSTA
            } else {
                0
            }
        }
        fn write32(&self, _off: usize, _val: u32) {}
    }
    let mut rep = Report::new();
    let o = crate::count_on(&Stuck, 0, S, false, &mut rep, |_| {});
    assert_eq!((o, rep.fail_stage, rep.rate_hz), (Outcome::Failed, Stage::Controller as u32, usdhc::RATE_ASSUMED));
}
