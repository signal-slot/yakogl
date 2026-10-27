// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The CSF command-stream instructions a frame uses: an encoder that writes
//! into the caller's words and a decoder for the same set.
//!
//! A CS instruction is one 64-bit word, opcode in bits 63:56 (`CS Base`,
//! v10.xml:565-568). Every instruction here is one generated struct of
//! [`super::genxml::v10`] (`CsMove48` ... `CsSyncAdd64`, v10.xml:570-843),
//! so a field that does not fit is the packer's refusal, never masked. What
//! the packer cannot see is checked here, after Mesa's builder
//! (src/panfrost/genxml/cs_builder.h, Mesa 26.2.1, MIT; the rules are
//! restated, not transcribed):
//!
//! - a 64-bit operand is an even register pair (`cs_src64` / `cs_dst64`,
//!   cs_builder.h:383-395, 427-438);
//! - a stream stays below the registers the kernel keeps for its own
//!   wrapper, given here as `user_regs` (Mesa 25.0.7 fixes 96 registers
//!   and 4 for the kernel, src/gallium/drivers/panfrost/pan_csf.c:250-252;
//!   26.2.1 takes `cs_reg_count` and keeps `max(unpreserved, 4)` from
//!   panthor's CSIF query, its pan_csf.c:475-476);
//! - the words go into a buffer the caller owns; a full buffer is a
//!   refusal, not a reallocation.
//!
//! Each cause has its own [`CsError`] and its own message.
//!
//! The decoder ([`Instr::decode`]) unpacks a word with the generated
//! unpacker, which refuses a set bit no field owns, and [`Instr::encode`]
//! packs it again: `src/mali/frame_tests.rs` runs every word of two board
//! traces through both and requires the same word back.

use super::genxml::pack::PackError;
use super::genxml::v10::*;
use core::fmt;

/// Why an instruction was not encoded, decoded or appended.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CsError {
    /// A field value the generated packer refused (too wide, not in its
    /// enum, a reserved bit set on decode, ...).
    Pack(PackError),
    /// A 64-bit operand named by an odd register.
    OddPair { instr: &'static str, reg: u32 },
    /// A register at or past the stream's user registers.
    KernelRegister { instr: &'static str, reg: u32, user_regs: u32 },
    /// The caller's word buffer has no room for another instruction.
    StreamFull { capacity: usize },
    /// An opcode this module neither encodes nor decodes.
    NotHandled { opcode: u32 },
}

impl From<PackError> for CsError {
    fn from(e: PackError) -> Self {
        CsError::Pack(e)
    }
}

impl fmt::Display for CsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            CsError::Pack(e) => write!(f, "mali cs: {e}"),
            CsError::OddPair { instr, reg } => {
                write!(f, "mali cs {instr}: a 64-bit operand in the odd register r{reg}")
            }
            CsError::KernelRegister { instr, reg, user_regs } => write!(
                f,
                "mali cs {instr}: r{reg} is not a user register (the stream has r0-r{})",
                user_regs.saturating_sub(1)
            ),
            CsError::StreamFull { capacity } => {
                write!(f, "mali cs: the word buffer is full ({capacity} instructions)")
            }
            CsError::NotHandled { opcode } => {
                write!(f, "mali cs: opcode {opcode:#x} is not one this encoder handles")
            }
        }
    }
}

/// One instruction of the set this module handles, as its generated struct.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Instr {
    Nop(CsNop),
    Move48(CsMove48),
    Move32(CsMove32),
    Wait(CsWait),
    RunIdvs(CsRunIdvs),
    RunFragment(CsRunFragment),
    FinishTiling(CsFinishTiling),
    FinishFragment(CsFinishFragment),
    AddImm32(CsAddImm32),
    LoadMultiple(CsLoadMultiple),
    StoreMultiple(CsStoreMultiple),
    Branch(CsBranch),
    SetSbEntry(CsSetSbEntry),
    SetExceptionHandler(CsSetExceptionHandler),
    ReqResource(CsReqResource),
    FlushCache2(CsFlushCache2),
    StoreState(CsStoreState),
    HeapSet(CsHeapSet),
    HeapOperation(CsHeapOperation),
    SyncAdd64(CsSyncAdd64),
}

/// A register operand: its number and whether it is a 64-bit pair.
#[derive(Clone, Copy)]
struct Operand {
    reg: u32,
    count: u32,
    pair: bool,
}

const fn r32(reg: u32) -> Operand {
    Operand { reg, count: 1, pair: false }
}
const fn r64(reg: u32) -> Operand {
    Operand { reg, count: 2, pair: true }
}
const NONE: Operand = Operand { reg: 0, count: 0, pair: false };

/// The registers `LOAD_MULTIPLE` / `STORE_MULTIPLE` move: from `base`, one
/// per bit of the 16-bit mask up to its highest set bit (`cs_load_to`,
/// Mesa 26.2.1 cs_builder.h:2099-2102: `count = util_last_bit(mask)`).
fn tuple(base: u32, mask: u32) -> Operand {
    Operand { reg: base, count: 32 - mask.leading_zeros(), pair: false }
}

impl Instr {
    /// The instruction one word holds.
    pub fn decode(word: u64) -> Result<Instr, CsError> {
        let b = word.to_le_bytes();
        let opcode = (word >> 56) as u32;
        Ok(match CsOpcode(opcode) {
            CsOpcode::NOP => Instr::Nop(CsNop::unpack(&b)?),
            CsOpcode::MOVE48 => Instr::Move48(CsMove48::unpack(&b)?),
            CsOpcode::MOVE32 => Instr::Move32(CsMove32::unpack(&b)?),
            CsOpcode::WAIT => Instr::Wait(CsWait::unpack(&b)?),
            CsOpcode::RUN_IDVS => Instr::RunIdvs(CsRunIdvs::unpack(&b)?),
            CsOpcode::RUN_FRAGMENT => Instr::RunFragment(CsRunFragment::unpack(&b)?),
            CsOpcode::FINISH_TILING => Instr::FinishTiling(CsFinishTiling::unpack(&b)?),
            CsOpcode::FINISH_FRAGMENT => Instr::FinishFragment(CsFinishFragment::unpack(&b)?),
            CsOpcode::ADD_IMMEDIATE32 => Instr::AddImm32(CsAddImm32::unpack(&b)?),
            CsOpcode::LOAD_MULTIPLE => Instr::LoadMultiple(CsLoadMultiple::unpack(&b)?),
            CsOpcode::STORE_MULTIPLE => Instr::StoreMultiple(CsStoreMultiple::unpack(&b)?),
            CsOpcode::BRANCH => Instr::Branch(CsBranch::unpack(&b)?),
            CsOpcode::SET_SB_ENTRY => Instr::SetSbEntry(CsSetSbEntry::unpack(&b)?),
            CsOpcode::SET_EXCEPTION_HANDLER => Instr::SetExceptionHandler(CsSetExceptionHandler::unpack(&b)?),
            CsOpcode::REQ_RESOURCE => Instr::ReqResource(CsReqResource::unpack(&b)?),
            CsOpcode::FLUSH_CACHE2 => Instr::FlushCache2(CsFlushCache2::unpack(&b)?),
            CsOpcode::STORE_STATE => Instr::StoreState(CsStoreState::unpack(&b)?),
            CsOpcode::HEAP_SET => Instr::HeapSet(CsHeapSet::unpack(&b)?),
            CsOpcode::HEAP_OPERATION => Instr::HeapOperation(CsHeapOperation::unpack(&b)?),
            CsOpcode::SYNC_ADD64 => Instr::SyncAdd64(CsSyncAdd64::unpack(&b)?),
            _ => return Err(CsError::NotHandled { opcode }),
        })
    }

    /// The word, packed by the instruction's generated packer.
    pub fn encode(&self) -> Result<u64, CsError> {
        let bytes = match self {
            Instr::Nop(i) => i.pack()?,
            Instr::Move48(i) => i.pack()?,
            Instr::Move32(i) => i.pack()?,
            Instr::Wait(i) => i.pack()?,
            Instr::RunIdvs(i) => i.pack()?,
            Instr::RunFragment(i) => i.pack()?,
            Instr::FinishTiling(i) => i.pack()?,
            Instr::FinishFragment(i) => i.pack()?,
            Instr::AddImm32(i) => i.pack()?,
            Instr::LoadMultiple(i) => i.pack()?,
            Instr::StoreMultiple(i) => i.pack()?,
            Instr::Branch(i) => i.pack()?,
            Instr::SetSbEntry(i) => i.pack()?,
            Instr::SetExceptionHandler(i) => i.pack()?,
            Instr::ReqResource(i) => i.pack()?,
            Instr::FlushCache2(i) => i.pack()?,
            Instr::StoreState(i) => i.pack()?,
            Instr::HeapSet(i) => i.pack()?,
            Instr::HeapOperation(i) => i.pack()?,
            Instr::SyncAdd64(i) => i.pack()?,
        };
        Ok(u64::from_le_bytes(bytes))
    }

    /// The opcode's name in v10.xml's `CS Opcode` (v10.xml:518-558).
    pub fn name(&self) -> &'static str {
        match self {
            Instr::Nop(_) => "NOP",
            Instr::Move48(_) => "MOVE48",
            Instr::Move32(_) => "MOVE32",
            Instr::Wait(_) => "WAIT",
            Instr::RunIdvs(_) => "RUN_IDVS",
            Instr::RunFragment(_) => "RUN_FRAGMENT",
            Instr::FinishTiling(_) => "FINISH_TILING",
            Instr::FinishFragment(_) => "FINISH_FRAGMENT",
            Instr::AddImm32(_) => "ADD_IMMEDIATE32",
            Instr::LoadMultiple(_) => "LOAD_MULTIPLE",
            Instr::StoreMultiple(_) => "STORE_MULTIPLE",
            Instr::Branch(_) => "BRANCH",
            Instr::SetSbEntry(_) => "SET_SB_ENTRY",
            Instr::SetExceptionHandler(_) => "SET_EXCEPTION_HANDLER",
            Instr::ReqResource(_) => "REQ_RESOURCE",
            Instr::FlushCache2(_) => "FLUSH_CACHE2",
            Instr::StoreState(_) => "STORE_STATE",
            Instr::HeapSet(_) => "HEAP_SET",
            Instr::HeapOperation(_) => "HEAP_OPERATION",
            Instr::SyncAdd64(_) => "SYNC_ADD64",
        }
    }

    /// The registers the instruction names (at most three).
    fn operands(&self) -> [Operand; 3] {
        match *self {
            Instr::Nop(_)
            | Instr::Wait(_)
            | Instr::RunFragment(_)
            | Instr::FinishTiling(_)
            | Instr::SetSbEntry(_)
            | Instr::ReqResource(_)
            | Instr::HeapOperation(_) => [NONE; 3],
            Instr::Move48(i) => [r64(i.destination), NONE, NONE],
            Instr::Move32(i) => [r32(i.destination), NONE, NONE],
            Instr::RunIdvs(i) => [if i.draw_id_register_enable { r32(i.draw_id) } else { NONE }, NONE, NONE],
            Instr::FinishFragment(i) => [r64(i.first_heap_chunk), r64(i.last_heap_chunk), NONE],
            Instr::AddImm32(i) => [r32(i.destination), r32(i.source), NONE],
            Instr::LoadMultiple(i) => [tuple(i.base_register, i.mask), r64(i.address), NONE],
            Instr::StoreMultiple(i) => [tuple(i.base_register, i.mask), r64(i.address), NONE],
            Instr::Branch(i) => [r32(i.value), NONE, NONE],
            Instr::SetExceptionHandler(i) => [r64(i.address), r32(i.length), NONE],
            Instr::FlushCache2(i) => [r32(i.latest_flush_id), NONE, NONE],
            Instr::StoreState(i) => [r64(i.address), NONE, NONE],
            Instr::HeapSet(i) => [r64(i.address), NONE, NONE],
            Instr::SyncAdd64(i) => [r64(i.data), r64(i.address), NONE],
        }
    }
}

/// A command stream being written into the caller's words.
pub struct Stream<'w> {
    words: &'w mut [u64],
    len: usize,
    user_regs: u32,
}

impl<'w> Stream<'w> {
    /// A stream over `words` that may name r0 .. r(`user_regs` - 1): the
    /// queue's `cs_reg_count` less the kernel's `max(unpreserved, 4)`
    /// (Mesa 26.2.1 pan_csf.c:475-476), which the OSAL reads from panthor's
    /// CSIF query.
    pub fn new(words: &'w mut [u64], user_regs: u32) -> Self {
        Stream { words, len: 0, user_regs }
    }

    /// Append one instruction, refusing an odd 64-bit operand, a kernel
    /// register or a full buffer.
    pub fn push(&mut self, instr: Instr) -> Result<(), CsError> {
        let name = instr.name();
        for op in instr.operands() {
            if op.count == 0 {
                continue;
            }
            if op.pair && !op.reg.is_multiple_of(2) {
                return Err(CsError::OddPair { instr: name, reg: op.reg });
            }
            let last = op.reg + op.count - 1;
            if last >= self.user_regs {
                return Err(CsError::KernelRegister { instr: name, reg: last, user_regs: self.user_regs });
            }
        }
        let word = instr.encode()?;
        let capacity = self.words.len();
        let slot = self.words.get_mut(self.len).ok_or(CsError::StreamFull { capacity })?;
        *slot = word;
        self.len += 1;
        Ok(())
    }

    /// `MOVE48`: a 48-bit immediate into a register pair, zero-extended.
    pub fn move48(&mut self, dst: u32, imm: u64) -> Result<(), CsError> {
        self.push(Instr::Move48(CsMove48 { immediate: imm, destination: dst, ..Default::default() }))
    }

    /// `MOVE32`: a 32-bit immediate into one register.
    pub fn move32(&mut self, dst: u32, imm: u32) -> Result<(), CsError> {
        self.push(Instr::Move32(CsMove32 { immediate: imm, destination: dst, ..Default::default() }))
    }

    /// Mesa's `cs_move64_to` (cs_builder.h:1481-1491): one `MOVE48` when
    /// the value fits 48 bits, else two `MOVE32`s, low word first.
    pub fn move64(&mut self, dst: u32, imm: u64) -> Result<(), CsError> {
        if imm < 1 << 48 {
            return self.move48(dst, imm);
        }
        if !dst.is_multiple_of(2) {
            return Err(CsError::OddPair { instr: "MOVE64", reg: dst });
        }
        self.move32(dst, imm as u32)?;
        self.move32(dst + 1, (imm >> 32) as u32)
    }

    /// `WAIT` on the scoreboard slots in `mask`, no progress increment.
    pub fn wait(&mut self, mask: u32) -> Result<(), CsError> {
        self.push(Instr::Wait(CsWait { wait_mask: mask, ..Default::default() }))
    }

    /// The stream over `words` again, its first `len` words kept: a frame
    /// left open between two calls goes on where it stopped. `None` when
    /// `len` is past the buffer (the caller's defect).
    pub fn resume(words: &'w mut [u64], len: usize, user_regs: u32) -> Option<Self> {
        (len <= words.len()).then_some(Stream { words, len, user_regs })
    }

    /// Drop every word after the first `len`: a draw that did not fit is
    /// undone. A longer `len` is ignored.
    pub fn truncate(&mut self, len: usize) {
        self.len = self.len.min(len);
    }

    /// The words written so far.
    pub fn words(&self) -> &[u64] {
        &self.words[..self.len]
    }

    /// How many words the buffer holds at most.
    pub fn capacity(&self) -> usize {
        self.words.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mali::genxml::pack::Refusal;
    use crate::mali::trace;

    const FRAME: &str = include_str!("../../tools/mali/board-dump/tri-frame-2026-10-18.txt");

    /// pandecode prints MOVE48 as `MOVE`; every other name is v10.xml's.
    fn pandecode_name(i: &Instr) -> &'static str {
        match i {
            Instr::Move48(_) => "MOVE",
            other => other.name(),
        }
    }

    /// Every command-stream word of both board traces (the context
    /// preamble, the tiler OOM handler as pandecode printed it and as raw
    /// memory, the batch as printed and as raw memory), decoded and encoded
    /// again: the same word, and the opcode pandecode named.
    #[test]
    fn every_cs_word_of_the_board_traces_decodes_and_encodes_to_itself() {
        let mut printed = 0usize;
        let mut differ = Vec::new();
        for source in ["linear", "afbc"] {
            for w in trace::cs_words(FRAME, source) {
                printed += 1;
                let i = Instr::decode(w.word).unwrap_or_else(|e| panic!("{source} line {}: {e}", w.line));
                let again = i.encode().unwrap_or_else(|e| panic!("{source} line {}: {e}", w.line));
                if again != w.word {
                    differ.push(format!("{source} line {}: {:#018x} -> {again:#018x}", w.line, w.word));
                }
                let first = w.mnemonic.split(['.', ' ']).next().expect("a mnemonic");
                assert_eq!(pandecode_name(&i), first, "{source} line {}: {}", w.line, w.mnemonic);
            }
        }
        /* the raw buffers: the batch CS (58 words) and the OOM handler
         * (0x140 bytes, SET_EXCEPTION_HANDLER's length, 40 words) */
        let mem = trace::memory_of(FRAME, "linear");
        let mut raw = 0usize;
        for (va, n) in [(0x7fff_fffc_c000u64, 58usize), (0x7fff_fffe_9000, 40)] {
            for k in 0..n {
                let word = u64::from_le_bytes(trace::bytes(&mem, va + 8 * k as u64, 8).try_into().expect("8 bytes"));
                raw += 1;
                let again = Instr::decode(word).and_then(|i| i.encode());
                if again != Ok(word) {
                    differ.push(format!("raw {:#x}: {word:#018x} -> {again:?}", va + 8 * k as u64));
                }
            }
        }
        eprintln!("mali cs round trip: {printed} printed words, {raw} raw words, {} differing", differ.len());
        /* 79 + 12 + 11 printed lines of each trace, 102 a trace */
        assert_eq!(printed, 2 * 102, "the excerpt's printed CS words");
        assert_eq!(raw, 98);
        assert!(differ.is_empty(), "{}", differ.join("\n"));
    }

    /// The raw batch words after the RUN_IDVS hold one word the printed
    /// trace does not: the MOVE d40 a taken BRANCH skipped (the
    /// incremental-rendering framebuffer, Mesa 25.0.7 pan_csf.c:823-827). Every other
    /// raw word is a printed one, in order: the two views agree.
    #[test]
    fn the_raw_batch_words_are_the_printed_ones_and_the_skipped_move() {
        let mem = trace::memory_of(FRAME, "linear");
        let raw: Vec<u64> = (0..58u64)
            .map(|k| u64::from_le_bytes(trace::bytes(&mem, 0x7fff_fffc_c000 + 8 * k, 8).try_into().expect("8 bytes")))
            .collect();
        let printed: Vec<u64> =
            trace::cs_words(FRAME, "linear").into_iter().filter(|w| w.line >= 46).map(|w| w.word).collect();
        assert_eq!(printed.len(), 57);
        let skipped = 45; /* raw 0x168 */
        assert_eq!(Instr::decode(raw[skipped]).map(|i| i.name()), Ok("MOVE48"));
        let mut without = raw.clone();
        without.remove(skipped);
        assert_eq!(without, printed);
    }

    #[test]
    fn each_refusal_has_its_own_message() {
        let mut words = [0u64; 2];
        let mut s = Stream::new(&mut words, 92);
        let odd = s.move48(1, 0).unwrap_err();
        assert_eq!(odd, CsError::OddPair { instr: "MOVE48", reg: 1 });
        let kernel = s.move32(92, 0).unwrap_err();
        assert_eq!(kernel, CsError::KernelRegister { instr: "MOVE32", reg: 92, user_regs: 92 });
        /* d90 is r90:r91, inside; d92 is not */
        assert!(s.move48(90, 0).is_ok());
        assert!(matches!(s.move48(92, 0), Err(CsError::KernelRegister { reg: 93, .. })));
        let wide = s.move48(0, 1 << 48).unwrap_err();
        assert!(
            matches!(wide, CsError::Pack(PackError { refusal: Refusal::TooWide { bits: 48, .. }, .. })),
            "{wide:?}"
        );
        assert!(s.wait(0).is_ok());
        let full = s.wait(0).unwrap_err();
        assert_eq!(full, CsError::StreamFull { capacity: 2 });
        let unknown = Instr::decode(0x7f << 56).unwrap_err();
        assert_eq!(unknown, CsError::NotHandled { opcode: 0x7f });
        /* a MOVE32 with bits 32..47 set: bits no field owns */
        let reserved = Instr::decode((2 << 56) | (1 << 40)).unwrap_err();
        assert!(
            matches!(reserved, CsError::Pack(PackError { refusal: Refusal::ReservedBits { .. }, .. })),
            "{reserved:?}"
        );
        let msgs: Vec<String> = [odd, kernel, wide, full, unknown, reserved].iter().map(|e| e.to_string()).collect();
        for (i, a) in msgs.iter().enumerate() {
            for b in &msgs[i + 1..] {
                assert_ne!(a, b);
            }
        }
        assert!(msgs[1].contains("not a user register") && msgs[3].contains("full"));
    }

    /// `move64` is Mesa's `cs_move64_to`: below 2^48 one MOVE48, above two
    /// MOVE32s; the trace's FAU pointer (r8, r9) is the second case.
    #[test]
    fn move64_splits_where_mesa_does() {
        let mut words = [0u64; 3];
        let mut s = Stream::new(&mut words, 92);
        s.move64(8, 0x0300_7fff_fffd_4200).expect("fits");
        s.move64(60, 0x3f80_0000).expect("fits");
        assert_eq!(
            s.words(),
            [
                u64::from_le_bytes([0x00, 0x42, 0xfd, 0xff, 0x00, 0x00, 0x08, 0x02]),
                u64::from_le_bytes([0xff, 0x7f, 0x00, 0x03, 0x00, 0x00, 0x09, 0x02]),
                u64::from_le_bytes([0x00, 0x00, 0x80, 0x3f, 0x00, 0x00, 0x3c, 0x01]),
            ]
        );
        assert!(matches!(s.move64(9, 1 << 50), Err(CsError::OddPair { instr: "MOVE64", reg: 9 })));
    }
}
