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

#[cfg(test)]
mod tests {
    //! [`write_instr71`], against the text Mesa's `v3d_qpu_decode` prints for
    //! ver 71 (qpu_disasm.c): the layout (mul half at column 30, signals at
    //! column 60), the operand forms, and the branch text. The function
    //! prints one instruction; it has no line numbers or labels of its own.
    //!
    //! ```text
    //! cargo test --lib --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table listing71
    //! ```

    use super::*;

    fn text(i: &Instr71) -> String {
        let mut s = String::new();
        write_instr71(&mut s, i).unwrap();
        s
    }

    fn alu_text_of(f: impl FnOnce(&mut Alu71)) -> String {
        let mut a = Alu71::nop();
        f(&mut a);
        text(&Instr71::Alu(a))
    }

    fn branch(f: impl FnOnce(&mut Branch)) -> String {
        let mut b = Branch {
            cond: BranchCond::Always,
            msfign: Msfign::None,
            bdi: BranchDest::Rel,
            ub: false,
            bdu: BranchDest::Abs,
            raddr_a: 0,
            offset: 0,
        };
        f(&mut b);
        text(&Instr71::Branch(b))
    }

    #[test]
    fn nop_is_both_halves_with_the_mul_at_column_30() {
        let s = text(&Instr71::Alu(Alu71::nop()));
        assert_eq!(s, "nop                           ; nop");
        assert_eq!(s.find("; ").unwrap(), 30);
        assert!(!s.contains('\n'));
    }

    #[test]
    fn add_with_operands_and_a_mul_nop() {
        let s = alu_text_of(|a| {
            a.add.op = AddOp71::Add;
            a.add.waddr = 3;
            a.add.magic = false;
            a.add.a = Operand { raddr: 1, unpack: Unpack71::None };
            a.add.b = Operand { raddr: 2, unpack: Unpack71::None };
        });
        assert!(s.starts_with("add rf3, rf1, rf2"), "{s}");
        assert_eq!(s.find("; nop"), Some(30), "{s}");
        assert!(!s[30..].contains("thrsw"));
    }

    #[test]
    fn mul_half_prints_its_operands_after_the_semicolon() {
        let s = alu_text_of(|a| {
            a.mul.op = MulOp71::Fmul;
            a.mul.waddr = 7;
            a.mul.magic = false;
            a.mul.a = Operand { raddr: 4, unpack: Unpack71::None };
            a.mul.b = Operand { raddr: 5, unpack: Unpack71::None };
        });
        assert!(s.contains("; fmul rf7, rf4, rf5"), "{s}");
        assert_eq!(s.find("; "), Some(30));
    }

    #[test]
    fn an_add_wider_than_column_30_pushes_the_mul_out() {
        // pad_to never truncates: the mul half follows the add text directly.
        let s = alu_text_of(|a| {
            a.add.op = AddOp71::Add;
            a.add.waddr = 100;
            a.add.magic = false;
            a.add.a = Operand { raddr: 100, unpack: Unpack71::Swap16 };
            a.add.b = Operand { raddr: 100, unpack: Unpack71::Swap16 };
        });
        let at = s.find("; nop").unwrap();
        assert!(at > 30, "{s}");
    }

    #[test]
    fn magic_destination_prints_its_name_and_an_unnamed_one_says_so() {
        let named = (0..=u8::MAX).find(|w| magic_waddr_name(*w).is_some()).unwrap();
        let s = alu_text_of(|a| {
            a.add.op = AddOp71::Add;
            a.add.waddr = named;
            a.add.magic = true;
        });
        assert!(s.contains(&format!(" {},", magic_waddr_name(named).unwrap())), "{s}");
        let s = alu_text_of(|a| {
            a.add.op = AddOp71::Add;
            a.add.waddr = 0;
            a.add.magic = true;
        });
        assert!(s.contains("waddr UNKNOWN 0"), "{s}");
    }

    #[test]
    fn small_immediates_print_as_ints_floats_and_unknown_indices() {
        let imm = |raddr: u8| {
            alu_text_of(|a| {
                a.add.op = AddOp71::Add;
                a.add.waddr = 1;
                a.add.magic = false;
                a.add.a = Operand { raddr, unpack: Unpack71::None };
                a.sig.small_imm_a = true;
            })
        };
        assert!(imm(3).contains("rf1, 3,"), "{}", imm(3));
        assert!(imm(15).contains("rf1, 15,"));
        // 16..31 are -16..-1.
        assert!(imm(16).contains("rf1, -16,"));
        assert!(imm(31).contains("rf1, -1,"));
        // 32 is 2^-8 as an f32: outside -16..15 it prints as hex.
        assert!(imm(32).contains("rf1, 0x3b800000,"), "{}", imm(32));
        // Past the 48-entry table the word is undecodable and prints the index.
        assert!(imm(63).contains("smimm63"), "{}", imm(63));
    }

    #[test]
    fn a_register_operand_with_the_small_imm_flag_off_stays_rf() {
        let s = alu_text_of(|a| {
            a.add.op = AddOp71::Add;
            a.add.a = Operand { raddr: 3, unpack: Unpack71::None };
        });
        assert!(s.contains("rf3"), "{s}");
    }

    #[test]
    fn condition_push_and_update_suffixes_follow_the_op_name() {
        let s = alu_text_of(|a| {
            a.add.op = AddOp71::Add;
            a.flags.ac = Cond::IfA;
            a.flags.apf = Pf::PushZ;
            a.mul.op = MulOp71::Mov;
            a.flags.mc = Cond::IfNB;
            a.flags.muf = Uf::AndZ;
        });
        assert!(s.starts_with("add.ifa.pushz "), "{s}");
        assert!(s.contains("; mov.ifnb.andz"), "{s}");
    }

    #[test]
    fn a_signal_that_writes_an_address_prints_no_condition() {
        let mut a = Alu71::nop();
        a.sig.ldunifrf = true;
        a.sig_addr = 9;
        a.sig_magic = false;
        a.flags.ac = Cond::IfA;
        a.flags.mc = Cond::IfB;
        let s = text(&Instr71::Alu(a));
        assert!(!s.contains(".ifa") && !s.contains(".ifb"), "{s}");
        assert!(s.ends_with("; ldunifrf.rf9"), "{s}");
    }

    #[test]
    fn signals_start_at_column_60_in_the_order_the_code_prints_them() {
        let mut a = Alu71::nop();
        a.sig.thrsw = true;
        a.sig.ldunif = true;
        let s = text(&Instr71::Alu(a));
        assert_eq!(&s[60..], "; thrsw; ldunif", "{s}");
        let mut a = Alu71::nop();
        a.sig.ldtmu = true;
        a.sig_magic = false;
        a.sig_addr = 2;
        let s = text(&Instr71::Alu(a));
        assert!(s.ends_with("; ldtmu.rf2"), "{s}");
        assert_eq!(s.find("; ldtmu"), Some(60));
        let mut a = Alu71::nop();
        a.sig.wrtmuc = true;
        assert!(text(&Instr71::Alu(a)).ends_with("; wrtmuc"));
    }

    #[test]
    fn signals_mesa_does_not_print_leave_the_line_unpadded() {
        let mut a = Alu71::nop();
        a.sig.ucb = true;
        a.sig.rotate = true;
        let s = text(&Instr71::Alu(a));
        assert_eq!(s, text(&Instr71::Alu(Alu71::nop())));
        assert!(s.len() < 60);
    }

    #[test]
    fn branches_print_their_condition_and_destination() {
        assert_eq!(branch(|_| {}), "b  0");
        assert_eq!(branch(|b| b.offset = (-16i32) as u32), "b  -16");
        assert_eq!(branch(|b| b.cond = BranchCond::A0), "b.a0  0");
        assert_eq!(branch(|b| b.cond = BranchCond::AllNA), "b.allna  0");
        assert_eq!(branch(|b| b.msfign = Msfign::Q), "bq  0");
        assert_eq!(
            branch(|b| {
                b.bdi = BranchDest::Abs;
                b.offset = 0x40;
            }),
            "b  zero_addr+0x00000040"
        );
        assert_eq!(branch(|b| b.bdi = BranchDest::LinkReg), "b  lri");
        assert_eq!(
            branch(|b| {
                b.bdi = BranchDest::Regfile;
                b.raddr_a = 12;
            }),
            "b  rf12"
        );
    }

    #[test]
    fn a_uniform_branch_adds_its_second_destination() {
        assert_eq!(
            branch(|b| {
                b.ub = true;
                b.bdu = BranchDest::Rel;
            }),
            "bu  0, r:unif"
        );
        assert!(branch(|b| {
            b.ub = true;
            b.bdu = BranchDest::Abs;
        })
        .ends_with(", a:unif"));
        assert!(branch(|b| {
            b.ub = true;
            b.bdu = BranchDest::LinkReg;
        })
        .ends_with(", lri"));
        assert!(branch(|b| {
            b.ub = true;
            b.bdu = BranchDest::Regfile;
            b.raddr_a = 5;
        })
        .ends_with(", rf5"));
    }

    #[test]
    fn output_is_appended_to_the_writer() {
        let mut s = String::from("0: ");
        write_instr71(&mut s, &Instr71::Alu(Alu71::nop())).unwrap();
        assert!(s.starts_with("0: nop"));
        // Column counting starts at the instruction, not at the writer's start.
        assert_eq!(s.find("; nop"), Some(3 + 30));
    }
}
