// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Ported from Mesa 26.2.1 src/broadcom/qpu/qpu_disasm.c and qpu_instr.c (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! A disassembler: one decoded instruction in the text Mesa's `v3d_qpu_decode`
//! prints for ver 42 (qpu_disasm.c, the names in qpu_instr.c; Mesa 26.2.1,
//! Copyright (c) 2016 Broadcom, MIT licence).
//!
//! It exists so that "decode gives the expected fields" can be checked against
//! text someone else wrote: Mesa's golden vectors (`tests/qpu_disasm.c`) are
//! pairs of a word and this text. The spacing is Mesa's (the mul half starts
//! at column 30, the signals at column 60), so the comparison is exact.
//!
//! One deliberate difference: the word Mesa prints as `vdwwt` on 4.2 prints
//! as `iid` here (see [`AddOp`]'s comment).
//!
//! It writes through [`core::fmt::Write`], so it works against a fixed buffer
//! in a `no_std` build as well as a `String` on the host.

use super::isa::*;
use core::fmt::{self, Write};

/// A writer that counts the bytes written, for Mesa's column padding.
pub(super) struct Col<'a, W: Write> {
    pub(super) w: &'a mut W,
    pub(super) n: usize,
}

impl<W: Write> Write for Col<'_, W> {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        self.n += s.len();
        self.w.write_str(s)
    }
}

impl<W: Write> Col<'_, W> {
    pub(super) fn pad_to(&mut self, n: usize) -> fmt::Result {
        while self.n < n {
            self.write_str(" ")?;
        }
        Ok(())
    }
}

pub fn add_op_name(op: AddOp) -> &'static str {
    use AddOp::*;
    match op {
        Fadd => "fadd",
        Faddnf => "faddnf",
        Vfpack => "vfpack",
        Add => "add",
        Sub => "sub",
        Fsub => "fsub",
        Min => "min",
        Max => "max",
        Umin => "umin",
        Umax => "umax",
        Shl => "shl",
        Shr => "shr",
        Asr => "asr",
        Ror => "ror",
        Fmin => "fmin",
        Fmax => "fmax",
        Vfmin => "vfmin",
        And => "and",
        Or => "or",
        Xor => "xor",
        Vadd => "vadd",
        Vsub => "vsub",
        Not => "not",
        Neg => "neg",
        Flapush => "flapush",
        Flbpush => "flbpush",
        Flpop => "flpop",
        Recip => "recip",
        Setmsf => "setmsf",
        Setrevf => "setrevf",
        Nop => "nop",
        Tidx => "tidx",
        Eidx => "eidx",
        Lr => "lr",
        Vfla => "vfla",
        Vflna => "vflna",
        Vflb => "vflb",
        Vflnb => "vflnb",
        Fxcd => "fxcd",
        Xcd => "xcd",
        Fycd => "fycd",
        Ycd => "ycd",
        Msf => "msf",
        Revf => "revf",
        Iid => "iid",
        Sampid => "sampid",
        Barrierid => "barrierid",
        Tmuwt => "tmuwt",
        Vpmsetup => "vpmsetup",
        Vpmwt => "vpmwt",
        Flafirst => "flafirst",
        Flnafirst => "flnafirst",
        LdvpmvIn => "ldvpmv_in",
        LdvpmvOut => "ldvpmv_out",
        LdvpmdIn => "ldvpmd_in",
        LdvpmdOut => "ldvpmd_out",
        Ldvpmp => "ldvpmp",
        Rsqrt => "rsqrt",
        Exp => "exp",
        Log => "log",
        Sin => "sin",
        Rsqrt2 => "rsqrt2",
        LdvpmgIn => "ldvpmg_in",
        LdvpmgOut => "ldvpmg_out",
        Fcmp => "fcmp",
        Vfmax => "vfmax",
        Fround => "fround",
        Ftoin => "ftoin",
        Ftrunc => "ftrunc",
        Ftoiz => "ftoiz",
        Ffloor => "ffloor",
        Ftouz => "ftouz",
        Fceil => "fceil",
        Ftoc => "ftoc",
        Fdx => "fdx",
        Fdy => "fdy",
        Stvpmv => "stvpmv",
        Stvpmd => "stvpmd",
        Stvpmp => "stvpmp",
        Itof => "itof",
        Clz => "clz",
        Utof => "utof",
    }
}

pub fn mul_op_name(op: MulOp) -> &'static str {
    match op {
        MulOp::Add => "add",
        MulOp::Sub => "sub",
        MulOp::Umul24 => "umul24",
        MulOp::Vfmul => "vfmul",
        MulOp::Smul24 => "smul24",
        MulOp::Multop => "multop",
        MulOp::Fmov => "fmov",
        MulOp::Mov => "mov",
        MulOp::Nop => "nop",
        MulOp::Fmul => "fmul",
    }
}

pub(super) fn cond_name(c: Cond) -> &'static str {
    match c {
        Cond::None => "",
        Cond::IfA => ".ifa",
        Cond::IfB => ".ifb",
        Cond::IfNA => ".ifna",
        Cond::IfNB => ".ifnb",
    }
}

pub(super) fn pf_name(p: Pf) -> &'static str {
    match p {
        Pf::None => "",
        Pf::PushZ => ".pushz",
        Pf::PushN => ".pushn",
        Pf::PushC => ".pushc",
    }
}

pub(super) fn uf_name(u: Uf) -> &'static str {
    match u {
        Uf::None => "",
        Uf::AndZ => ".andz",
        Uf::AndNZ => ".andnz",
        Uf::NorZ => ".norz",
        Uf::NorNZ => ".nornz",
        Uf::AndN => ".andn",
        Uf::AndNN => ".andnn",
        Uf::NorN => ".norn",
        Uf::NorNN => ".nornn",
        Uf::AndC => ".andc",
        Uf::AndNC => ".andnc",
        Uf::NorC => ".norc",
        Uf::NorNC => ".nornc",
    }
}

pub(super) fn pack_name(p: OutPack) -> &'static str {
    match p {
        OutPack::None => "",
        OutPack::L => ".l",
        OutPack::H => ".h",
    }
}

fn unpack_name(u: Unpack) -> &'static str {
    match u {
        Unpack::None => "",
        Unpack::L => ".l",
        Unpack::H => ".h",
        Unpack::Abs => ".abs",
        Unpack::ReplicateF32To16 => ".ff",
        Unpack::ReplicateL16 => ".ll",
        Unpack::ReplicateH16 => ".hh",
        Unpack::Swap16 => ".swp",
    }
}

fn raddr<W: Write>(w: &mut Col<W>, alu: &Alu, mux: Mux) -> fmt::Result {
    match mux {
        Mux::A => write!(w, "rf{}", alu.raddr_a),
        Mux::B if alu.sig.small_imm_b => match alu.small_imm() {
            Some(v) if (-16..=15).contains(&(v as i32)) => write!(w, "{}", v as i32),
            Some(v) => write!(w, "0x{v:08x}"),
            // The decoder refuses this word; a hand-built one prints what it is.
            None => write!(w, "smimm{}", alu.raddr_b),
        },
        Mux::B => write!(w, "rf{}", alu.raddr_b),
        m => write!(w, "r{}", m as u8),
    }
}

fn waddr<W: Write>(w: &mut Col<W>, waddr: u8, magic: bool) -> fmt::Result {
    if !magic {
        return write!(w, "rf{waddr}");
    }
    match magic_waddr_name(waddr) {
        Some(n) => w.write_str(n),
        None => write!(w, "waddr UNKNOWN {waddr}"),
    }
}

/// The destination of one ALU half: waddr, magic, output pack.
type Dst = (u8, bool, OutPack);

/// The operands of one ALU half, as Mesa's `v3d_qpu_disasm_add`/`_mul`
/// print them after the name.
fn alu_half<W: Write>(w: &mut Col<W>, alu: &Alu, args: u8, dst: Dst, a: Input, b: Input) -> fmt::Result {
    let has_dst = args & ARG_D != 0;
    let n = num_src(args);
    w.write_str(" ")?;
    if has_dst {
        waddr(w, dst.0, dst.1)?;
        w.write_str(pack_name(dst.2))?;
    }
    if n >= 1 {
        if has_dst {
            w.write_str(", ")?;
        }
        raddr(w, alu, a.mux)?;
        w.write_str(unpack_name(a.unpack))?;
    }
    if n >= 2 {
        w.write_str(", ")?;
        raddr(w, alu, b.mux)?;
        w.write_str(unpack_name(b.unpack))?;
    }
    Ok(())
}

fn sig_addr<W: Write>(w: &mut Col<W>, alu: &Alu) -> fmt::Result {
    if !alu.sig_magic {
        return write!(w, ".rf{}", alu.sig_addr);
    }
    match magic_waddr_name(alu.sig_addr) {
        Some(n) => write!(w, ".{n}"),
        None => write!(w, ".UNKNOWN{}", alu.sig_addr),
    }
}

fn alu_text<W: Write>(w: &mut Col<W>, alu: &Alu) -> fmt::Result {
    let flags_shown = !alu.sig.writes_address();
    w.write_str(add_op_name(alu.add.op))?;
    if flags_shown {
        w.write_str(cond_name(alu.flags.ac))?;
    }
    w.write_str(pf_name(alu.flags.apf))?;
    w.write_str(uf_name(alu.flags.auf))?;
    alu_half(w, alu, add_op_args(alu.add.op), (alu.add.waddr, alu.add.magic, alu.add.pack), alu.add.a, alu.add.b)?;

    w.pad_to(30)?;
    w.write_str("; ")?;
    w.write_str(mul_op_name(alu.mul.op))?;
    if flags_shown {
        w.write_str(cond_name(alu.flags.mc))?;
    }
    w.write_str(pf_name(alu.flags.mpf))?;
    w.write_str(uf_name(alu.flags.muf))?;
    if alu.mul.op != MulOp::Nop {
        alu_half(w, alu, mul_op_args(alu.mul.op), (alu.mul.waddr, alu.mul.magic, alu.mul.pack), alu.mul.a, alu.mul.b)?;
    }

    // Mesa prints none of ucb, rotate and small_imm_b.
    let s = &alu.sig;
    if !(s.thrsw
        || s.ldvary
        || s.ldtmu
        || s.ldtlb
        || s.ldtlbu
        || s.ldunif
        || s.ldunifrf
        || s.ldunifa
        || s.ldunifarf
        || s.wrtmuc)
    {
        return Ok(());
    }
    w.pad_to(60)?;
    if s.thrsw {
        w.write_str("; thrsw")?;
    }
    if s.ldvary {
        w.write_str("; ldvary")?;
        sig_addr(w, alu)?;
    }
    if s.ldtmu {
        w.write_str("; ldtmu")?;
        sig_addr(w, alu)?;
    }
    if s.ldtlb {
        w.write_str("; ldtlb")?;
        sig_addr(w, alu)?;
    }
    if s.ldtlbu {
        w.write_str("; ldtlbu")?;
        sig_addr(w, alu)?;
    }
    if s.ldunif {
        w.write_str("; ldunif")?;
    }
    if s.ldunifrf {
        w.write_str("; ldunifrf")?;
        sig_addr(w, alu)?;
    }
    if s.ldunifa {
        w.write_str("; ldunifa")?;
    }
    if s.ldunifarf {
        w.write_str("; ldunifarf")?;
        sig_addr(w, alu)?;
    }
    if s.wrtmuc {
        w.write_str("; wrtmuc")?;
    }
    Ok(())
}

pub(super) fn branch_text<W: Write>(w: &mut Col<W>, br: &Branch) -> fmt::Result {
    w.write_str("b")?;
    if br.ub {
        w.write_str("u")?;
    }
    w.write_str(match br.cond {
        BranchCond::Always => "",
        BranchCond::A0 => ".a0",
        BranchCond::NA0 => ".na0",
        BranchCond::AllA => ".alla",
        BranchCond::AnyNA => ".anyna",
        BranchCond::AnyA => ".anya",
        BranchCond::AllNA => ".allna",
    })?;
    w.write_str(match br.msfign {
        Msfign::None => "",
        Msfign::P => "p",
        Msfign::Q => "q",
    })?;
    match br.bdi {
        BranchDest::Abs => write!(w, "  zero_addr+0x{:08x}", br.offset)?,
        BranchDest::Rel => write!(w, "  {}", br.offset as i32)?,
        BranchDest::LinkReg => w.write_str("  lri")?,
        BranchDest::Regfile => write!(w, "  rf{}", br.raddr_a)?,
    }
    if br.ub {
        match br.bdu {
            BranchDest::Abs => w.write_str(", a:unif")?,
            BranchDest::Rel => w.write_str(", r:unif")?,
            BranchDest::LinkReg => w.write_str(", lri")?,
            BranchDest::Regfile => write!(w, ", rf{}", br.raddr_a)?,
        }
    }
    Ok(())
}

/// Write one instruction as Mesa's disassembler does.
pub fn write_instr(w: &mut impl Write, instr: &Instr) -> fmt::Result {
    let mut c = Col { w, n: 0 };
    match instr {
        Instr::Alu(alu) => alu_text(&mut c, alu),
        Instr::Branch(br) => branch_text(&mut c, br),
    }
}
