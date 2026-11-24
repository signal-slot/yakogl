// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The counted way back of a direct boot from eMMC boot partition 2 (boot1):
//! the boot count kept in one sector of boot1, and the one EXT_CSD write
//! that points the eMMC's boot operation at boot partition 1 (boot0) again
//! (docs/notes/imx95-direct-boot.md, section 5, "The counted way back").
//!
//! The loader runs [`count_run`] before it starts its payload. In order:
//!
//! 1. The card from reset: CMD0, CMD1 until ready (sector addressed), CMD2,
//!    CMD3 (RCA [`RCA`]), CMD7, all in 1-bit mode at the identification
//!    clock, then the transfer clock (1-bit, legacy timing, at most 20 MHz).
//!    Nothing of the card state the previous stage (U-Boot SPL) left is used:
//!    its bus width, timing and tuning are its own, and CMD0 resets them on
//!    the card (they are volatile fields of the EXT_CSD).
//! 2. EXT_CSD (CMD8): `PARTITION_CONFIG` (byte 179) as found, the boot
//!    partitions' size (`BOOT_SIZE_MULT`, 226), the boot write protection
//!    (`BOOT_WP`, 173), for the record.
//! 3. CMD6 `PARTITION_CONFIG` = found with `PARTITION_ACCESS` = 2 (boot1),
//!    `BOOT_ACK` and `BOOT_PARTITION_ENABLE` as found: the access field is
//!    volatile, the other two are written with the values they hold.
//! 4. The record (CMD17 of the count sector), checked ([`record_parse`]),
//!    and the verdict ([`decide`]): `remaining` > 0 boots the payload with
//!    `remaining - 1` written back (CMD24) and read back (CMD17) first;
//!    `remaining` 0, a record that does not check, or a request on the
//!    console take the way back.
//! 5. CMD6 `PARTITION_ACCESS` = 0 (the user area) again, the rest as found.
//! 6. The way back ([`back_value`]): CMD6 `PARTITION_CONFIG` = `BOOT_ACK` as
//!    found | `BOOT_PARTITION_ENABLE` 1 (boot0) | access 0, then EXT_CSD read
//!    again and the byte compared. The caller then resets the SoC.
//!
//! Every failure after the EXT_CSD read takes the way back (the payload is
//! never started uncounted); a failure of the way back itself, or before the
//! EXT_CSD is known, is [`Outcome::Failed`], and the caller stops (its
//! watchdog then resets the board into the same chain: the stranding case
//! the note's table names). Only `PARTITION_CONFIG` and the count sector are
//! ever written.
//!
//! The commands and fields are JEDEC JESD84-B51's (eMMC 5.1, the board's
//! card): CMD0/1/2/3/7/8/6/13/17/24, the R1 card status bits, EXT_CSD
//! 173/179/226. The same sequence for the AM62P's loader runs through
//! Zephyr's SD stack (src/main.c, `boot0_select`); here it runs over
//! [`Bus`], which is the uSDHC in polled mode on the board (`usdhc.rs`) and
//! a card model in the host tests.

use crate::crc32::crc32;

pub const BLOCK: usize = 512;

/// The record, one 512-byte sector of boot1, little-endian:
///
/// | offset | size | field |
/// |---|---|---|
/// | 0 | 8 | magic `"YKBOOTCT"` |
/// | 8 | 4 | version, 1 |
/// | 12 | 4 | remaining: starts of the payload left |
/// | 16 | 4 | initial: `remaining` as `imx95-mkboot.py count` wrote it |
/// | 20 | 4 | the record's own sector number (a record read from elsewhere does not check) |
/// | 24 | 484 | zero |
/// | 508 | 4 | CRC-32 (zlib's) of bytes 0..508 |
///
/// `tools/imx95-mkboot.py count` writes it.
pub const RECORD_MAGIC: [u8; 8] = *b"YKBOOTCT";
pub const RECORD_VERSION: u32 = 1;

/// The count sector in boot1: 64494, at byte offset 0x1F7DC00. The board's
/// boot partitions are 64512 sectors (31.5 MiB, `BOOT_SIZE_MULT` 0xfc);
/// U-Boot's environment is boot0's last 0x2200 bytes, sectors 64495..64511.
/// 64494 is the highest sector that is outside that range in either
/// partition, so a record written to boot0 by mistake cannot reach U-Boot's
/// environment, and a boot1 image may take everything below it.
pub const COUNT_SECTOR: u32 = 64494;

/// The relative card address this loader gives the card (CMD3).
pub const RCA: u16 = 2;

/// CMD1's argument: sector addressing, 2.7-3.6 V and 1.70-1.95 V.
pub const OCR_HOST: u32 = 0x40ff_8080;
pub const OCR_BUSY_DONE: u32 = 1 << 31;
pub const OCR_SECTOR_MODE: u32 = 1 << 30;

pub const EXT_CSD_BOOT_WP: usize = 173;
pub const EXT_CSD_PARTITION_CONFIG: usize = 179;
pub const EXT_CSD_BOOT_SIZE_MULT: usize = 226;
pub const PC_BOOT_ACK: u8 = 0x40;
pub const PC_ENABLE_BOOT0: u8 = 1 << 3;
pub const PC_ACCESS_MASK: u8 = 0x07;
pub const PC_ACCESS_BOOT1: u8 = 2;

/// R1 card status: the error bits (31..26, 24..19 and SWITCH_ERROR 7), the
/// state (12..9), READY_FOR_DATA (8).
pub const R1_ERRORS: u32 = 0xfdf8_0080;
pub const R1_SWITCH_ERROR: u32 = 1 << 7;
pub const R1_READY_FOR_DATA: u32 = 1 << 8;
pub const STATE_TRAN: u32 = 4;
pub const fn r1_state(status: u32) -> u32 {
    (status >> 9) & 0xf
}

/// Polls of the card's ready state after CMD1 (1 ms apart): JEDEC gives the
/// card 1 s from the first CMD1.
pub const CMD1_POLLS: u32 = 1000;
/// Milliseconds a CMD6 or a write may keep the card busy: `PARTITION_CONFIG`
/// switches within PARTITION_SWITCH_TIME (EXT_CSD 199, at most 2.55 s), a
/// one-sector write well within this.
pub const BUSY_MS: u32 = 3000;

/// A command's response type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resp {
    None,
    /// 48 bits, CRC and index checked.
    R1,
    /// R1 with busy on DAT0.
    R1b,
    /// 136 bits (CID, CSD).
    R2,
    /// 48 bits, no CRC or index (OCR).
    R3,
}

/// At most one block of data with a command.
pub enum Data<'a> {
    None,
    Read(&'a mut [u8; BLOCK]),
    Write(&'a [u8; BLOCK]),
}

/// What a host controller does for this module.
pub trait Bus {
    /// Send command `index` with `arg`; the response words (R2: the CID or
    /// CSD's bits 127..0 in words 0..3, word 0 the highest; else word 0).
    /// A data block moves with it. `Err` is a code that names the bus
    /// error ([`BusError`]).
    fn command(&mut self, index: u8, arg: u32, resp: Resp, data: Data<'_>) -> Result<[u32; 4], u32>;
    /// The card holds DAT0 low (busy).
    fn busy(&mut self) -> bool;
    fn delay_us(&mut self, us: u32);
    /// The SD clock: at most 400 kHz (`false`, identification) or at most
    /// 20 MHz (`true`, transfer). `Err` as for `command`.
    fn set_clock(&mut self, transfer: bool) -> Result<(), u32>;
}

/// The bus errors a [`Bus`] reports, as codes (they cross to C).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum BusError {
    CmdTimeout = 1,
    CmdCrc = 2,
    CmdEndBit = 3,
    CmdIndex = 4,
    DataTimeout = 5,
    DataCrc = 6,
    DataEndBit = 7,
    /// The controller's command or data inhibit did not clear.
    Inhibit = 8,
    /// The buffer did not become ready, or the transfer did not complete.
    Transfer = 9,
    /// The SD clock did not report stable.
    Clock = 10,
    /// A software reset of the controller did not complete.
    Reset = 11,
}

/// Where the sequence failed: the first failure is reported with its code
/// (a [`BusError`], a card status, a value read).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Stage {
    None = 0,
    Controller = 1,
    Cmd0 = 2,
    /// The card did not leave its busy state within [`CMD1_POLLS`].
    Cmd1 = 3,
    /// The card is byte addressed (code: its OCR).
    Cmd1ByteMode = 4,
    Cmd2 = 5,
    Cmd3 = 6,
    Clock = 7,
    Cmd7 = 8,
    ExtCsd = 9,
    /// The count sector is not inside boot1 (code: the boot partition's sectors).
    BootSize = 10,
    AccessBoot1 = 11,
    ReadRecord = 12,
    WriteRecord = 13,
    /// The record read back differs from the one written.
    VerifyRecord = 14,
    AccessUser = 15,
    Back = 16,
    /// EXT_CSD read after the way back (code: bus error) ...
    BackRead = 17,
    /// ... or its `PARTITION_CONFIG` is not the value written (code: the byte read).
    BackVerify = 18,
}

/// What the loader does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Outcome {
    /// Start the payload: the decremented record is on the card.
    Boot = 1,
    /// `PARTITION_CONFIG` says boot0 (read back): reset the SoC.
    Back = 2,
    /// Neither: stop (the watchdog resets the board).
    Failed = 3,
}

/// Why the way back was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum Why {
    None = 0,
    /// The record's `remaining` is 0.
    Spent = 1,
    /// The record does not check ([`RecordError`] in the report).
    Invalid = 2,
    /// The word on the console.
    Requested = 3,
    /// A step failed (the report's stage and code).
    Failure = 4,
}

/// Why a record does not check.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum RecordError {
    Magic = 1,
    Version = 2,
    Crc = 3,
    Sector = 4,
    /// A reserved byte is not zero.
    Reserved = 5,
    /// `remaining` is larger than `initial`.
    Count = 6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Record {
    pub remaining: u32,
    pub initial: u32,
}

/// The record's sector bytes.
pub fn record_bytes(r: &Record, sector: u32) -> [u8; BLOCK] {
    let mut s = [0u8; BLOCK];
    s[..8].copy_from_slice(&RECORD_MAGIC);
    s[8..12].copy_from_slice(&RECORD_VERSION.to_le_bytes());
    s[12..16].copy_from_slice(&r.remaining.to_le_bytes());
    s[16..20].copy_from_slice(&r.initial.to_le_bytes());
    s[20..24].copy_from_slice(&sector.to_le_bytes());
    let c = crc32(&s[..508]);
    s[508..].copy_from_slice(&c.to_le_bytes());
    s
}

fn le32(s: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([s[at], s[at + 1], s[at + 2], s[at + 3]])
}

/// The record in `s`, read from `sector`, or why it does not check. The CRC
/// first: a torn or never written sector (boot1 is all zero bytes on the
/// board as delivered) fails there.
pub fn record_parse(s: &[u8; BLOCK], sector: u32) -> Result<Record, RecordError> {
    if s[..8] != RECORD_MAGIC {
        return Err(RecordError::Magic);
    }
    if crc32(&s[..508]) != le32(s, 508) {
        return Err(RecordError::Crc);
    }
    if le32(s, 8) != RECORD_VERSION {
        return Err(RecordError::Version);
    }
    if le32(s, 20) != sector {
        return Err(RecordError::Sector);
    }
    if s[24..508].iter().any(|&b| b != 0) {
        return Err(RecordError::Reserved);
    }
    let r = Record { remaining: le32(s, 12), initial: le32(s, 16) };
    if r.remaining > r.initial {
        return Err(RecordError::Count);
    }
    Ok(r)
}

/// The verdict on a parsed record: `Ok(next)` boots with `next` written
/// first, `Err(why)` takes the way back.
pub fn decide(parsed: Result<Record, RecordError>, requested: bool) -> Result<Record, Why> {
    if requested {
        return Err(Why::Requested);
    }
    match parsed {
        Err(_) => Err(Why::Invalid),
        Ok(r) if r.remaining == 0 => Err(Why::Spent),
        Ok(r) => Ok(Record { remaining: r.remaining - 1, initial: r.initial }),
    }
}

/// `PARTITION_CONFIG` with the access field `part`, the rest as found.
pub const fn access_value(found: u8, part: u8) -> u8 {
    (found & !PC_ACCESS_MASK) | (part & PC_ACCESS_MASK)
}

/// The way back: `BOOT_ACK` as found, boot0 enabled, the user area.
/// 0x48 on the board (found 0x48 under Toradex's chain, 0x50 under ours
/// after `mmc bootpart enable 2 1`).
pub const fn back_value(found: u8) -> u8 {
    (found & PC_BOOT_ACK) | PC_ENABLE_BOOT0
}

/// What [`count_run`] did, for the log (include/boot_stage2.h `struct
/// bs2_count_report`; a field not reached keeps [`NOT_READ`]).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Report {
    pub outcome: u32,
    pub why: u32,
    /// The first failure ([`Stage`], 0: none) and its code.
    pub fail_stage: u32,
    pub fail_code: u32,
    /// [`RecordError`] of the record read, 0 when it checked.
    pub record_error: u32,
    /// EXT_CSD `PARTITION_CONFIG` as found, and as read back after the way back.
    pub found_config: u32,
    pub final_config: u32,
    pub boot_sectors: u32,
    pub boot_wp: u32,
    pub remaining_before: u32,
    pub remaining_after: u32,
    pub initial: u32,
    /// The CID's highest word (manufacturer id in bits 31..24).
    pub cid0: u32,
    /// CMD1 polls until the card was ready.
    pub cmd1_polls: u32,
    /// The controller's root clock (as given, or assumed) and the SD clocks
    /// it gave (the bus fills these; 0 from a bus that does not).
    pub rate_hz: u32,
    pub ident_hz: u32,
    pub transfer_hz: u32,
}

pub const NOT_READ: u32 = u32::MAX;

// include/boot_stage2.h asserts the same size and offset.
const _: () = assert!(core::mem::size_of::<Report>() == 68);
const _: () = assert!(core::mem::offset_of!(Report, cid0) == 48);

impl Report {
    pub const fn new() -> Report {
        Report {
            outcome: Outcome::Failed as u32,
            why: Why::None as u32,
            fail_stage: Stage::None as u32,
            fail_code: 0,
            record_error: NOT_READ,
            found_config: NOT_READ,
            final_config: NOT_READ,
            boot_sectors: NOT_READ,
            boot_wp: NOT_READ,
            remaining_before: NOT_READ,
            remaining_after: NOT_READ,
            initial: NOT_READ,
            cid0: NOT_READ,
            cmd1_polls: 0,
            rate_hz: 0,
            ident_hz: 0,
            transfer_hz: 0,
        }
    }
    pub fn fail(&mut self, stage: Stage, code: u32) {
        if self.fail_stage == Stage::None as u32 {
            self.fail_stage = stage as u32;
            self.fail_code = code;
        }
    }
}

impl Default for Report {
    fn default() -> Report {
        Report::new()
    }
}

type Step<T> = Result<T, (Stage, u32)>;

fn r1(bus: &mut impl Bus, index: u8, arg: u32, stage: Stage) -> Step<u32> {
    let r = bus.command(index, arg, Resp::R1, Data::None).map_err(|e| (stage, e))?;
    if r[0] & R1_ERRORS != 0 {
        return Err((stage, r[0]));
    }
    Ok(r[0])
}

/// CMD13 until the card is in TRAN with READY_FOR_DATA and DAT0 released,
/// for at most `ms`; the status, which must hold no error bit.
fn wait_tran(bus: &mut impl Bus, stage: Stage, ms: u32) -> Step<u32> {
    let mut last = 0;
    for _ in 0..=ms {
        if !bus.busy() {
            let st = bus.command(13, (RCA as u32) << 16, Resp::R1, Data::None).map_err(|e| (stage, e))?;
            last = st[0];
            if last & R1_ERRORS != 0 {
                return Err((stage, last));
            }
            if r1_state(last) == STATE_TRAN && last & R1_READY_FOR_DATA != 0 {
                return Ok(last);
            }
        }
        bus.delay_us(1000);
    }
    Err((stage, last))
}

/// CMD6 in its write-byte mode, then [`wait_tran`]: SWITCH_ERROR fails.
fn switch(bus: &mut impl Bus, index: usize, value: u8, stage: Stage) -> Step<()> {
    let arg = (3u32 << 24) | ((index as u32) << 16) | ((value as u32) << 8);
    let r = bus.command(6, arg, Resp::R1b, Data::None).map_err(|e| (stage, e))?;
    if r[0] & R1_ERRORS != 0 {
        return Err((stage, r[0]));
    }
    wait_tran(bus, stage, BUSY_MS).map(|_| ())
}

fn ext_csd(bus: &mut impl Bus, buf: &mut [u8; BLOCK], stage: Stage) -> Step<()> {
    let r = bus.command(8, 0, Resp::R1, Data::Read(buf)).map_err(|e| (stage, e))?;
    if r[0] & R1_ERRORS != 0 {
        return Err((stage, r[0]));
    }
    Ok(())
}

/// Step 1: the card from CMD0 to TRAN at the transfer clock.
pub fn card_up(bus: &mut impl Bus, rep: &mut Report) -> Step<()> {
    bus.set_clock(false).map_err(|e| (Stage::Clock, e))?;
    bus.command(0, 0, Resp::None, Data::None).map_err(|e| (Stage::Cmd0, e))?;
    bus.delay_us(1000);
    let mut ocr = 0;
    let mut ready = false;
    for n in 0..CMD1_POLLS {
        ocr = bus.command(1, OCR_HOST, Resp::R3, Data::None).map_err(|e| (Stage::Cmd1, e))?[0];
        if ocr & OCR_BUSY_DONE != 0 {
            rep.cmd1_polls = n + 1;
            ready = true;
            break;
        }
        bus.delay_us(1000);
    }
    if !ready {
        return Err((Stage::Cmd1, ocr));
    }
    if ocr & OCR_SECTOR_MODE == 0 {
        return Err((Stage::Cmd1ByteMode, ocr));
    }
    let cid = bus.command(2, 0, Resp::R2, Data::None).map_err(|e| (Stage::Cmd2, e))?;
    rep.cid0 = cid[0];
    r1(bus, 3, (RCA as u32) << 16, Stage::Cmd3)?;
    bus.set_clock(true).map_err(|e| (Stage::Clock, e))?;
    let r = bus.command(7, (RCA as u32) << 16, Resp::R1b, Data::None).map_err(|e| (Stage::Cmd7, e))?;
    if r[0] & R1_ERRORS != 0 {
        return Err((Stage::Cmd7, r[0]));
    }
    wait_tran(bus, Stage::Cmd7, BUSY_MS).map(|_| ())
}

/// Step 6, on a card in TRAN: `PARTITION_CONFIG` = [`back_value`] of
/// `found`, read back. `Ok` when the card holds it.
pub fn way_back(bus: &mut impl Bus, found: u8, rep: &mut Report) -> Step<()> {
    let want = back_value(found);
    switch(bus, EXT_CSD_PARTITION_CONFIG, want, Stage::Back)?;
    let mut e = [0u8; BLOCK];
    ext_csd(bus, &mut e, Stage::BackRead)?;
    rep.final_config = e[EXT_CSD_PARTITION_CONFIG] as u32;
    if e[EXT_CSD_PARTITION_CONFIG] != want {
        return Err((Stage::BackVerify, e[EXT_CSD_PARTITION_CONFIG] as u32));
    }
    Ok(())
}

/// Steps 3 to 5 on a card whose EXT_CSD is read: `Ok(())` to boot, `Err`
/// with the reason for the way back.
fn count(bus: &mut impl Bus, found: u8, sector: u32, requested: bool, rep: &mut Report) -> Result<(), Why> {
    if requested {
        return Err(Why::Requested);
    }
    let failed = |rep: &mut Report, (stage, code): (Stage, u32)| {
        rep.fail(stage, code);
        Why::Failure
    };
    switch(bus, EXT_CSD_PARTITION_CONFIG, access_value(found, PC_ACCESS_BOOT1), Stage::AccessBoot1)
        .map_err(|f| failed(rep, f))?;
    let mut s = [0u8; BLOCK];
    let r = bus.command(17, sector, Resp::R1, Data::Read(&mut s)).map_err(|e| (Stage::ReadRecord, e)).and_then(|r| {
        if r[0] & R1_ERRORS != 0 {
            Err((Stage::ReadRecord, r[0]))
        } else {
            Ok(())
        }
    });
    let verdict = match r {
        Err(f) => Err(failed(rep, f)),
        Ok(()) => {
            let parsed = record_parse(&s, sector);
            match parsed {
                Ok(p) => {
                    rep.record_error = 0;
                    rep.remaining_before = p.remaining;
                    rep.initial = p.initial;
                }
                Err(e) => rep.record_error = e as u32,
            }
            decide(parsed, false)
        }
    };
    let verdict = verdict.and_then(|next| {
        let w = record_bytes(&next, sector);
        let r = bus.command(24, sector, Resp::R1, Data::Write(&w)).map_err(|e| (Stage::WriteRecord, e));
        let r = r.and_then(|r| if r[0] & R1_ERRORS != 0 { Err((Stage::WriteRecord, r[0])) } else { Ok(()) });
        r.and_then(|()| wait_tran(bus, Stage::WriteRecord, BUSY_MS)).map_err(|f| failed(rep, f))?;
        let mut back = [0u8; BLOCK];
        let r = bus.command(17, sector, Resp::R1, Data::Read(&mut back)).map_err(|e| (Stage::VerifyRecord, e));
        let r = r.and_then(|r| if r[0] & R1_ERRORS != 0 { Err((Stage::VerifyRecord, r[0])) } else { Ok(()) });
        r.map_err(|f| failed(rep, f))?;
        if back != w {
            return Err(failed(rep, (Stage::VerifyRecord, 0)));
        }
        rep.remaining_after = next.remaining;
        Ok(())
    });
    // Step 5 also on the way back: the way back's CMD6 then starts from the
    // user area, as on every other path.
    switch(bus, EXT_CSD_PARTITION_CONFIG, access_value(found, 0), Stage::AccessUser).map_err(|f| failed(rep, f))?;
    verdict
}

/// The whole sequence, steps 1 to 6 (module doc). `sector` is the count
/// sector in boot1, `requested` the console's request.
pub fn count_run(bus: &mut impl Bus, sector: u32, requested: bool, rep: &mut Report) -> Outcome {
    let outcome = run(bus, sector, requested, rep);
    rep.outcome = outcome as u32;
    outcome
}

fn run(bus: &mut impl Bus, sector: u32, requested: bool, rep: &mut Report) -> Outcome {
    if let Err((stage, code)) = card_up(bus, rep) {
        rep.fail(stage, code);
        return Outcome::Failed;
    }
    let mut e = [0u8; BLOCK];
    if let Err((stage, code)) = ext_csd(bus, &mut e, Stage::ExtCsd) {
        rep.fail(stage, code);
        return Outcome::Failed;
    }
    let found = e[EXT_CSD_PARTITION_CONFIG];
    rep.found_config = found as u32;
    rep.boot_wp = e[EXT_CSD_BOOT_WP] as u32;
    let boot_sectors = e[EXT_CSD_BOOT_SIZE_MULT] as u32 * 256;
    rep.boot_sectors = boot_sectors;
    let why = if sector >= boot_sectors {
        rep.fail(Stage::BootSize, boot_sectors);
        Why::Failure
    } else {
        match count(bus, found, sector, requested, rep) {
            Ok(()) => return Outcome::Boot,
            Err(why) => why,
        }
    };
    rep.why = why as u32;
    back_from(bus, found, rep)
}

fn back_from(bus: &mut impl Bus, found: u8, rep: &mut Report) -> Outcome {
    match way_back(bus, found, rep) {
        Ok(()) => Outcome::Back,
        Err((stage, code)) => {
            rep.fail(stage, code);
            Outcome::Failed
        }
    }
}

/// The way back on a card [`count_run`] left in TRAN (it returned
/// [`Outcome::Boot`]): the console's request before the jump. EXT_CSD is
/// read again for `found`.
pub fn back_run(bus: &mut impl Bus, rep: &mut Report) -> Outcome {
    let mut e = [0u8; BLOCK];
    rep.why = Why::Requested as u32;
    let outcome = match ext_csd(bus, &mut e, Stage::ExtCsd) {
        Err((stage, code)) => {
            rep.fail(stage, code);
            Outcome::Failed
        }
        Ok(()) => {
            rep.found_config = e[EXT_CSD_PARTITION_CONFIG] as u32;
            back_from(bus, e[EXT_CSD_PARTITION_CONFIG], rep)
        }
    };
    rep.outcome = outcome as u32;
    outcome
}
