// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Ported from Mesa 26.2.1 src/broadcom/qpu/qpu_disasm.c (the 7.1 operand
// printing) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The 7.1 disassembler: one decoded 7.1 instruction in the text Mesa's
//! `v3d_qpu_decode` prints for ver 71 (qpu_disasm.c; Mesa 26.2.1, Copyright
//! (c) 2016 Broadcom, MIT licence). The layout is the 4.2 listing's
//! ([`super::listing`]: the mul half at column 30, the signals at column 60);
//! the operands are raddrs, each with its own small-immediate flag, and the
//! operation and unpack names are Mesa's own (`v3d_qpu_add_op_name`,
//! `v3d_qpu_mul_op_name`, `v3d_qpu_unpack_name`) through the FFI.

use super::isa71::*;
use super::listing::{branch_text, cond_name, pack_name, pf_name, uf_name, Col};
use super::qpu_ffi;
use core::fmt::{self, Write};

fn operand<W: Write>(w: &mut Col<W>, o: Operand, small_imm: bool) -> fmt::Result {
    if small_imm {
        return match Alu71::small_imm(o) {
            Some(v) if (-16..=15).contains(&(v as i32)) => write!(w, "{}", v as i32),
            Some(v) => write!(w, "0x{v:08x}"),
            // The decoder refuses this word; a hand-built one prints what it is.
            None => write!(w, "smimm{}", o.raddr),
        };
    }
    write!(w, "rf{}", o.raddr)
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

type Half = (u8, bool, super::isa::OutPack, Operand, bool, Operand, bool);

fn alu_half<W: Write>(w: &mut Col<W>, args: u8, h: Half) -> fmt::Result {
    let (wa, magic, pack, a, a_imm, b, b_imm) = h;
    let has_dst = args & ARG_D != 0;
    let n = super::isa::num_src(args);
    w.write_str(" ")?;
    if has_dst {
        waddr(w, wa, magic)?;
        w.write_str(pack_name(pack))?;
    }
    if n >= 1 {
        if has_dst {
            w.write_str(", ")?;
        }
        operand(w, a, a_imm)?;
        w.write_str(qpu_ffi::unpack_name(a.unpack as u32))?;
    }
    if n >= 2 {
        w.write_str(", ")?;
        operand(w, b, b_imm)?;
        w.write_str(qpu_ffi::unpack_name(b.unpack as u32))?;
    }
    Ok(())
}

fn sig_addr<W: Write>(w: &mut Col<W>, alu: &Alu71) -> fmt::Result {
    if !alu.sig_magic {
        return write!(w, ".rf{}", alu.sig_addr);
    }
    match magic_waddr_name(alu.sig_addr) {
        Some(n) => write!(w, ".{n}"),
        None => write!(w, ".UNKNOWN{}", alu.sig_addr),
    }
}

fn alu_text<W: Write>(w: &mut Col<W>, alu: &Alu71) -> fmt::Result {
    let flags_shown = !alu.sig.writes_address();
    let s = &alu.sig;
    w.write_str(qpu_ffi::add_op_name(alu.add.op as u32))?;
    if flags_shown {
        w.write_str(cond_name(alu.flags.ac))?;
    }
    w.write_str(pf_name(alu.flags.apf))?;
    w.write_str(uf_name(alu.flags.auf))?;
    let a = &alu.add;
    alu_half(w, add_op_args(a.op), (a.waddr, a.magic, a.pack, a.a, s.small_imm_a, a.b, s.small_imm_b))?;

    w.pad_to(30)?;
    w.write_str("; ")?;
    w.write_str(qpu_ffi::mul_op_name(alu.mul.op as u32))?;
    if flags_shown {
        w.write_str(cond_name(alu.flags.mc))?;
    }
    w.write_str(pf_name(alu.flags.mpf))?;
    w.write_str(uf_name(alu.flags.muf))?;
    if alu.mul.op != MulOp71::Nop {
        let m = &alu.mul;
        alu_half(w, mul_op_args(m.op), (m.waddr, m.magic, m.pack, m.a, s.small_imm_c, m.b, s.small_imm_d))?;
    }

    // Mesa prints none of ucb, rotate and the small-immediate flags.
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

/// Write one 7.1 instruction as Mesa's disassembler does.
pub fn write_instr71(w: &mut impl Write, instr: &Instr71) -> fmt::Result {
    let mut c = Col { w, n: 0 };
    match instr {
        Instr71::Alu(alu) => alu_text(&mut c, alu),
        Instr71::Branch(br) => branch_text(&mut c, br),
    }
}
