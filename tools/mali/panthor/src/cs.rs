// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2020, 2022 Collabora Ltd.
// SPDX-FileCopyrightText: 2025, 2026 Arm Ltd.
// The instruction layouts are transcribed from Mesa 26.2.1
// src/panfrost/genxml/v10.xml and the builder rules from
// src/panfrost/genxml/cs_builder.h (MIT). The upstream part stays under its
// licence (LICENSES/MIT.txt); our changes are Apache-2.0.
//! The few CSF command stream instructions the development OSAL emits.
//!
//! A CS instruction is one 64-bit word with the opcode in bits 63:56
//! (`CS Base`, v10.xml:565-568). Each encoder below is one `<struct>` of
//! v10.xml, cited by line, and refuses a value that does not fit its field
//! instead of masking it (CLAUDE.md: a register-field write refuses). The
//! register rules are cs_builder.h's: a 64-bit operand is an even register
//! pair (`cs_reg64`, cs_builder.h:455-459), and a stream built for a queue
//! stays below the registers the kernel keeps for its own wrapper
//! (`cs_reg_tuple`, cs_builder.h:435-446; the kernel's use of them is
//! panthor_sched.c:3013-3017).

use std::fmt;

/// `enum CS Opcode` (v10.xml:518-558): the values this module emits.
pub const OP_NOP: u64 = 0;
pub const OP_MOVE48: u64 = 1;
pub const OP_MOVE32: u64 = 2;
pub const OP_WAIT: u64 = 3;
pub const OP_FLUSH_CACHE2: u64 = 36;
pub const OP_SYNC_ADD64: u64 = 51;

/// `enum CS Flush Mode` (v10.xml:498-502).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FlushMode {
    None = 0,
    Clean = 1,
    CleanInvalidate = 3,
}

/// `enum CS Other Flush Mode` (v10.xml:504-507).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OtherFlushMode {
    None = 0,
    Invalidate = 2,
}

/// `enum CS Sync Scope` (v10.xml:509-512).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SyncScope {
    System = 0,
    Csg = 2,
}

/// When an asynchronous instruction runs and which scoreboard slot it
/// signals (`struct cs_async_op`, `cs_defer`, `cs_now`, cs_builder.h:775-805).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Async {
    /// Synchronous: wait mask 0. Refused for an instruction that is always
    /// asynchronous (cs_builder.h:867-875: "Can't use cs_now() on pure async
    /// instructions").
    Now,
    /// Wait for the slots in `wait`, then run and signal `signal` on
    /// completion. `wait` must not hold `signal` (cs_builder.h:784-796).
    Defer { wait: u16, signal: u8 },
}

/// Why an instruction was not encoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CsError {
    /// A value wider than its field.
    TooWide { instr: &'static str, field: &'static str, value: u64, bits: u32 },
    /// A 64-bit operand in an odd register.
    OddPair { instr: &'static str, reg: u8 },
    /// A register the kernel keeps for its wrapper, or past the file.
    KernelRegister { instr: &'static str, reg: u8, first_kernel_reg: u8 },
    /// A scoreboard slot this queue does not have.
    NoSuchSlot { instr: &'static str, mask: u16, slots: u8 },
    /// `Async::Defer` whose wait mask holds its own signal slot.
    WaitsOnItsOwnSignal { instr: &'static str, signal: u8 },
    /// `Async::Now` on an instruction that is always asynchronous.
    NowOnAsyncOnly { instr: &'static str },
    /// An opcode this module does not emit, so it has no operand rules.
    NotEmitted { opcode: u64 },
    /// A queue limit from the CSIF query that this builder cannot represent.
    OutOfRange { what: &'static str, value: u64, max: u64 },
}

impl fmt::Display for CsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CsError::TooWide { instr, field, value, bits } => {
                write!(f, "{instr}: {field} {value:#x} does not fit {bits} bits")
            }
            CsError::OddPair { instr, reg } => write!(f, "{instr}: 64-bit operand in odd register r{reg}"),
            CsError::KernelRegister { instr, reg, first_kernel_reg } => {
                write!(f, "{instr}: r{reg} is not a user register (the kernel's start at r{first_kernel_reg})")
            }
            CsError::NoSuchSlot { instr, mask, slots } => {
                write!(f, "{instr}: scoreboard mask {mask:#x} names a slot past the queue's {slots}")
            }
            CsError::WaitsOnItsOwnSignal { instr, signal } => {
                write!(f, "{instr}: the wait mask holds its own signal slot {signal}")
            }
            CsError::NowOnAsyncOnly { instr } => write!(f, "{instr}: always asynchronous, needs a signal slot"),
            CsError::NotEmitted { opcode } => write!(f, "opcode {opcode:#x} is not one this module emits"),
            CsError::OutOfRange { what, value, max } => write!(f, "{what} {value} is above the supported {max}"),
        }
    }
}

impl std::error::Error for CsError {}

fn field(instr: &'static str, name: &'static str, value: u64, bits: u32, shift: u32) -> Result<u64, CsError> {
    if bits < 64 && value >> bits != 0 {
        return Err(CsError::TooWide { instr, field: name, value, bits });
    }
    Ok(value << shift)
}

fn pair(instr: &'static str, reg: u8) -> Result<u64, CsError> {
    if !reg.is_multiple_of(2) {
        return Err(CsError::OddPair { instr, reg });
    }
    Ok(reg as u64)
}

/// The wait mask and signal slot fields of an instruction that may run
/// asynchronously (wait mask bits 31:16, signal slot bits 51:48 in every such
/// struct of v10.xml). `always_async`: the instruction never runs
/// synchronously (`cs_instr_is_asynchronous`, cs_builder.h:818-864); on v10 a
/// sometimes-asynchronous instruction with wait mask 0 carries signal slot 0
/// (`cs_apply_async`, cs_builder.h:867-875).
fn async_fields(instr: &'static str, a: Async, always_async: bool) -> Result<u64, CsError> {
    match a {
        Async::Now if always_async => Err(CsError::NowOnAsyncOnly { instr }),
        Async::Now => Ok(0),
        Async::Defer { wait, signal } => {
            if signal < 16 && wait & (1 << signal) != 0 {
                return Err(CsError::WaitsOnItsOwnSignal { instr, signal });
            }
            let signal = if always_async || wait != 0 { signal as u64 } else { 0 };
            Ok(field(instr, "wait mask", wait as u64, 16, 16)? | field(instr, "signal slot", signal, 4, 48)?)
        }
    }
}

/// `CS NOP` (v10.xml:570-573).
pub fn nop() -> u64 {
    OP_NOP << 56
}

/// `CS MOVE48` (v10.xml:575-579): a 48-bit immediate, zero-extended into the
/// register pair `dst` (`cs_move48_to`, cs_builder.h:902-909).
pub fn move48(dst: u8, imm: u64) -> Result<u64, CsError> {
    const I: &str = "MOVE48";
    Ok(OP_MOVE48 << 56 | field(I, "destination", pair(I, dst)?, 8, 48)? | field(I, "immediate", imm, 48, 0)?)
}

/// `CS MOVE32` (v10.xml:581-585) (`cs_move32_to`, cs_builder.h:893-900).
pub fn move32(dst: u8, imm: u32) -> Result<u64, CsError> {
    const I: &str = "MOVE32";
    Ok(OP_MOVE32 << 56 | field(I, "destination", dst as u64, 8, 48)? | imm as u64)
}

/// `CS WAIT` (v10.xml:587-591): wait for the scoreboard slots in `mask`
/// (`cs_wait_slots`, cs_builder.h:934-951). Progress increment: no.
pub fn wait(mask: u16) -> u64 {
    OP_WAIT << 56 | (mask as u64) << 16
}

/// `CS FLUSH_CACHE2` (v10.xml:739-747), always asynchronous
/// (`cs_flush_caches`, cs_builder.h:2259-2272). `flush_id` is the register
/// that holds the latest flush ID (0: flush unconditionally).
pub fn flush_cache2(
    l2: FlushMode,
    lsc: FlushMode,
    other: OtherFlushMode,
    flush_id: u8,
    a: Async,
) -> Result<u64, CsError> {
    const I: &str = "FLUSH_CACHE2";
    Ok(OP_FLUSH_CACHE2 << 56
        | l2 as u64
        | (lsc as u64) << 4
        | (other as u64) << 8
        | async_fields(I, a, true)?
        | field(I, "latest flush id", flush_id as u64, 8, 40)?)
}

/// `CS SYNC_ADD64` (v10.xml:835-843): add the 64-bit register pair `data` to
/// the 64-bit sync object at the address in the register pair `addr`
/// (`cs_sync64_add`, cs_builder.h:2274-2318).
pub fn sync_add64(propagate_error: bool, scope: SyncScope, data: u8, addr: u8, a: Async) -> Result<u64, CsError> {
    const I: &str = "SYNC_ADD64";
    Ok(OP_SYNC_ADD64 << 56
        | propagate_error as u64
        | (scope as u64) << 1
        | async_fields(I, a, false)?
        | field(I, "data", pair(I, data)?, 8, 32)?
        | field(I, "address", pair(I, addr)?, 8, 40)?)
}

/// The register pairs and slots an instruction names, for the builder's
/// queue checks. `None` for an opcode this module does not emit.
fn operands(word: u64) -> Option<(Vec<(u8, u8)>, u16)> {
    let r = |shift: u32| ((word >> shift) & 0xff) as u8;
    let wait_mask = ((word >> 16) & 0xffff) as u16;
    let signal = 1u16 << ((word >> 48) & 0xf);
    Some(match word >> 56 {
        OP_NOP => (vec![], 0),
        OP_MOVE48 => (vec![(r(48), 2)], 0),
        OP_MOVE32 => (vec![(r(48), 1)], 0),
        OP_WAIT => (vec![], wait_mask),
        OP_FLUSH_CACHE2 => (vec![(r(40), 1)], wait_mask | signal),
        OP_SYNC_ADD64 => (vec![(r(32), 2), (r(40), 2)], wait_mask | if wait_mask != 0 { signal } else { 0 }),
        _ => return None,
    })
}

/// A command stream for one queue: the instructions and the queue's limits.
pub struct Stream {
    words: Vec<u64>,
    first_kernel_reg: u8,
    slots: u8,
    /// The most words this stream held (printed on drop).
    high_water: usize,
    label: &'static str,
}

impl Stream {
    /// `cs_reg_count` and `unpreserved_cs_reg_count` from the CSIF query; the
    /// kernel keeps at least 4 (Mesa's `nr_kernel_registers`,
    /// pan_csf.c:1909-1914). `slots`: `scoreboard_slot_count`.
    ///
    /// Refuses a register count above 255 or more than 16 slots (the mask
    /// of `WAIT` is 16 bits) instead of clamping it.
    pub fn new(label: &'static str, cs_reg_count: u32, unpreserved: u32, slots: u32) -> Result<Self, CsError> {
        let user = u8::try_from(cs_reg_count.saturating_sub(unpreserved.max(4))).map_err(|_| CsError::OutOfRange {
            what: "user registers",
            value: cs_reg_count as u64,
            max: 255,
        })?;
        let slots = u8::try_from(slots).ok().filter(|&n| n <= 16).ok_or(CsError::OutOfRange {
            what: "scoreboard slots",
            value: slots as u64,
            max: 16,
        })?;
        Ok(Stream { words: Vec::new(), first_kernel_reg: user, slots, high_water: 0, label })
    }

    /// Append one encoded instruction, refusing a register the kernel keeps
    /// or a slot the queue does not have.
    pub fn push(&mut self, word: Result<u64, CsError>) -> Result<(), CsError> {
        let word = word?;
        let instr = "stream";
        let (regs, slots) = operands(word).ok_or(CsError::NotEmitted { opcode: word >> 56 })?;
        for (reg, n) in regs {
            if reg as u32 + n as u32 > self.first_kernel_reg as u32 {
                return Err(CsError::KernelRegister { instr, reg, first_kernel_reg: self.first_kernel_reg });
            }
        }
        if self.slots < 16 && slots >> self.slots != 0 {
            return Err(CsError::NoSuchSlot { instr, mask: slots, slots: self.slots });
        }
        self.words.push(word);
        self.high_water = self.high_water.max(self.words.len());
        Ok(())
    }

    pub fn words(&self) -> &[u64] {
        &self.words
    }

    /// The bytes as the GPU reads them (little-endian words).
    pub fn bytes(&self) -> Vec<u8> {
        self.words.iter().flat_map(|w| w.to_le_bytes()).collect()
    }

    /// Mesa's batch end, `csf_emit_batch_end` (pan_csf.c:556-605) without
    /// the PAN_DBG_SYNC state store and the trace points: wait on every
    /// slot, `MOVE32 r74, #0`, `FLUSH_CACHE2` clean L2, clean LSC, invalidate
    /// the others with flush ID r74 signalling slot 0 (`cs_defer(0, 0)`), and
    /// wait on slot 0 (`PANFROST_SB_LS`, pan_csf.h:61).
    pub fn push_mesa_batch_end(&mut self) -> Result<(), CsError> {
        let all = if self.slots >= 16 { u16::MAX } else { (1u16 << self.slots) - 1 };
        self.push(Ok(wait(all)))?;
        self.push(move32(74, 0))?;
        self.push(flush_cache2(
            FlushMode::Clean,
            FlushMode::Clean,
            OtherFlushMode::Invalidate,
            74,
            Async::Defer { wait: 0, signal: 0 },
        ))?;
        self.push(Ok(wait(1)))
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        println!(
            "arena cs '{}': high-water {} instructions ({} bytes)",
            self.label,
            self.high_water,
            self.high_water * 8
        );
    }
}

/// A one-line reading of an instruction this module emits (for the log;
/// pandecode's spelling where it has one).
pub fn describe(word: u64) -> String {
    let r = |shift: u32| (word >> shift) & 0xff;
    match word >> 56 {
        OP_NOP => "NOP".into(),
        OP_MOVE48 => format!("MOVE48 d{}, #{:#x}", r(48), word & 0xffff_ffff_ffff),
        OP_MOVE32 => format!("MOVE32 r{}, #{:#x}", r(48), word & 0xffff_ffff),
        OP_WAIT => format!("WAIT #{:#x}", (word >> 16) & 0xffff),
        OP_FLUSH_CACHE2 => format!(
            "FLUSH_CACHE2 l2={} lsc={} other={} r{} wait={:#x} signal={}",
            word & 0xf,
            (word >> 4) & 0xf,
            (word >> 8) & 0xf,
            r(40),
            (word >> 16) & 0xffff,
            (word >> 48) & 0xf
        ),
        OP_SYNC_ADD64 => format!(
            "SYNC_ADD64 propagate={} scope={} [d{}] += d{} wait={:#x} signal={}",
            word & 1,
            (word >> 1) & 3,
            r(40),
            r(32),
            (word >> 16) & 0xffff,
            (word >> 48) & 0xf
        ),
        op => format!("opcode {op} (not emitted here)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A word as pandecode prints it: eight bytes, least significant first.
    fn le(bytes: [u8; 8]) -> u64 {
        u64::from_le_bytes(bytes)
    }

    // Words the GPU executed: Mesa 25.0.7 drawing one triangle on the
    // NanoPC-T6 (PAN_MESA_DEBUG=trace,sync; the trace of
    // driver/docs/nanopc-t6-mali.md), its batch end as pandecode printed it:
    //   00 00 ff 00 00 00 00 03    WAIT #ff
    //   c0 32 fd ff ff 7f 5a 01    MOVE d90, #0x7FFFFFFD32C0
    //   00 00 00 00 00 00 4a 02    MOVE32 r74, #0x0
    //   11 02 00 00 00 4a 00 24    FLUSH_CACHE2.clean_l2.clean_lsc.invalidate_other r74, #0, #0
    //   00 00 01 00 00 00 00 03    WAIT #1
    // and from the context's init stream:
    //   40 01 00 00 00 00 58 02    MOVE32 r88, #0x140
    #[test]
    fn encoders_give_the_words_the_board_ran() {
        assert_eq!(wait(0xff), le([0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0x03]));
        assert_eq!(move48(90, 0x7fff_fffd_32c0), Ok(le([0xc0, 0x32, 0xfd, 0xff, 0xff, 0x7f, 0x5a, 0x01])));
        assert_eq!(move32(74, 0), Ok(le([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4a, 0x02])));
        assert_eq!(
            flush_cache2(
                FlushMode::Clean,
                FlushMode::Clean,
                OtherFlushMode::Invalidate,
                74,
                Async::Defer { wait: 0, signal: 0 }
            ),
            Ok(le([0x11, 0x02, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x24]))
        );
        assert_eq!(wait(1), le([0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x03]));
        assert_eq!(move32(88, 0x140), Ok(le([0x40, 0x01, 0x00, 0x00, 0x00, 0x00, 0x58, 0x02])));
        assert_eq!(nop(), 0);
    }

    /// The kernel's own SYNC_ADD64 and FLUSH_CACHE2 around every job, as
    /// panthor_sched.c:3046-3048 and 3088-3090 (Linux v6.18) spell them with
    /// shifts: an independent spelling of the same fields.
    #[test]
    fn encoders_agree_with_the_kernels_literal_words() {
        let (addr_reg, val_reg) = (92u64, 94u64);
        let kernel_sync = (51u64 << 56) | (addr_reg << 40) | (val_reg << 32) | 1;
        assert_eq!(sync_add64(true, SyncScope::System, 94, 92, Async::Now), Ok(kernel_sync));
        let kernel_flush = (36u64 << 56) | (val_reg << 40) | 0x233;
        assert_eq!(
            flush_cache2(
                FlushMode::CleanInvalidate,
                FlushMode::CleanInvalidate,
                OtherFlushMode::Invalidate,
                94,
                Async::Defer { wait: 0, signal: 0 }
            ),
            Ok(kernel_flush)
        );
        let kernel_move48 = (1u64 << 56) | (val_reg << 48) | 1;
        assert_eq!(move48(94, 1), Ok(kernel_move48));
        let kernel_wait_all = (3u64 << 56) | (0xff << 16);
        assert_eq!(wait(0xff), kernel_wait_all);
    }

    #[test]
    fn encoders_refuse_what_does_not_fit() {
        assert!(matches!(move48(1, 0), Err(CsError::OddPair { reg: 1, .. })));
        assert!(matches!(move48(0, 1 << 48), Err(CsError::TooWide { field: "immediate", .. })));
        assert!(matches!(sync_add64(true, SyncScope::System, 3, 0, Async::Now), Err(CsError::OddPair { reg: 3, .. })));
        assert!(matches!(
            sync_add64(true, SyncScope::System, 2, 0, Async::Defer { wait: 1, signal: 0 }),
            Err(CsError::WaitsOnItsOwnSignal { signal: 0, .. })
        ));
        assert!(matches!(
            sync_add64(true, SyncScope::System, 2, 0, Async::Defer { wait: 2, signal: 16 }),
            Err(CsError::TooWide { field: "signal slot", .. })
        ));
        assert!(matches!(
            flush_cache2(FlushMode::Clean, FlushMode::Clean, OtherFlushMode::None, 0, Async::Now),
            Err(CsError::NowOnAsyncOnly { .. })
        ));
    }

    #[test]
    fn synchronous_sync_add_carries_signal_slot_zero() {
        // cs_apply_async on v10: wait mask 0 makes SYNC_ADD64 synchronous and
        // the signal slot field 0, whatever slot was asked for.
        let w = sync_add64(false, SyncScope::Csg, 2, 0, Async::Defer { wait: 0, signal: 5 }).unwrap();
        assert_eq!((w >> 48) & 0xf, 0);
        assert_eq!((w >> 1) & 3, 2);
    }

    #[test]
    fn stream_refuses_kernel_registers_and_missing_slots() {
        // 96 registers, 4 unpreserved, 8 slots: the values the board's CSIF
        // query is expected to give are not assumed here; these are inputs.
        let mut s = Stream::new("test", 96, 4, 8).unwrap();
        assert_eq!(s.push(move48(90, 0)), Ok(()));
        assert!(matches!(s.push(move48(92, 0)), Err(CsError::KernelRegister { reg: 92, first_kernel_reg: 92, .. })));
        assert!(matches!(s.push(move32(92, 0)), Err(CsError::KernelRegister { .. })));
        assert!(matches!(s.push(Ok(wait(0x100))), Err(CsError::NoSuchSlot { .. })));
        assert_eq!(s.push(Ok(wait(0x80))), Ok(()));
        assert_eq!(s.words().len(), 2);
        // Mesa's own kernel-register floor: unpreserved 2 still keeps 4.
        let mut t = Stream::new("test", 96, 2, 8).unwrap();
        assert!(matches!(t.push(move32(92, 0)), Err(CsError::KernelRegister { .. })));
    }

    #[test]
    fn mesa_batch_end_is_the_traced_sequence() {
        let mut s = Stream::new("test", 96, 4, 8).unwrap();
        s.push_mesa_batch_end().unwrap();
        let traced = [
            le([0x00, 0x00, 0xff, 0x00, 0x00, 0x00, 0x00, 0x03]),
            le([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x4a, 0x02]),
            le([0x11, 0x02, 0x00, 0x00, 0x00, 0x4a, 0x00, 0x24]),
            le([0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x03]),
        ];
        assert_eq!(s.words(), &traced, "the trace's batch end without its PAN_DBG_SYNC MOVE d90 / STORE_STATE pair");
        assert_eq!(s.bytes().len(), 32);
    }

    #[test]
    fn an_unknown_opcode_is_its_own_refusal() {
        let mut s = Stream::new("test", 96, 4, 8).unwrap();
        // Opcode 0x7f is not one this module emits.
        assert_eq!(s.push(Ok(0x7f << 56)), Err(CsError::NotEmitted { opcode: 0x7f }));
        assert!(s.words().is_empty());
    }

    #[test]
    fn stream_limits_are_refused_not_clamped() {
        assert!(matches!(Stream::new("t", 400, 4, 8), Err(CsError::OutOfRange { what: "user registers", .. })));
        assert!(matches!(
            Stream::new("t", 96, 4, 17),
            Err(CsError::OutOfRange { what: "scoreboard slots", value: 17, .. })
        ));
        assert!(Stream::new("t", 96, 4, 16).is_ok());
    }
}
