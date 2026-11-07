// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! What the test-only QPU interpreter ([`super::sim`]) does differently for
//! ENCODED V3D 7.1 programs (the Raspberry Pi 5): `sim::run_gen(Gen::V71, ..)`
//! (or [`run71`]) runs them through the 4.2 interpreter's own machine, and
//! calls this module where the generations differ. Its state, its TLB, VPM,
//! TMU and SFU models, its flags, timing and undefined-lane rules are
//! [`super::sim`]'s, unchanged; so is its contract: anything it does not
//! model stops the run with an `Err` naming it ("unmodelled").
//!
//! A 7.1 word is decoded by [`super::codec71::decode71`] and lowered
//! ([`lower`]) to the 4.2 model's shape, which the shared loop reads for
//! what both generations have (signals, flags, conditions, write addresses,
//! output packs, branches), with the [`Alu71`] beside it ([`State71::alu`])
//! for what they do not. The four operands are the lowered instruction's
//! muxes R0..R3 (add a, add b, mul a, mul b), which [`operand`] reads from
//! the 7.1 instruction.
//!
//! # What 7.1 changes, and where it comes from
//!
//! From Mesa 26.2.1 (`src/broadcom/qpu` is vendored in
//! `third_party/mesa-26.2.1`; `compiler` is cited as the 4.2 interpreter cites
//! it), never from a GPL source:
//!
//! * **No accumulators.** Every operand is a register-file address or a small
//!   immediate (`qpu_instr.h`, `struct v3d_qpu_input` raddr; the
//!   small-immediate signals a..d, `qpu_pack.c:164-197`). A magic write to
//!   0..5 (the accumulators of 4.x; 5 is QUAD on 7.1) or to an address
//!   `qpu_validate.c:105-115` reserves is unmodelled, as is a signal writing
//!   any magic address but UNIFA.
//! * **rf0 written implicitly.** LDUNIF, LDUNIFA and LDVARY's C coefficient
//!   land in rf0 where 4.2 puts them in r5 (`v3d_qpu_writes_rf0_implicitly`,
//!   `qpu_instr.c:1031-1039`), with 4.2's timing: LDUNIF/LDUNIFA one
//!   instruction later, LDVARY's rf0 one cycle after its own destination
//!   (Mesa's scoreboard, `qpu_schedule.c:1739-1742`, and its "delayed rf0
//!   write", `qpu_schedule.c:715-726`). LDTMU and LDVARY always name their
//!   destination (`v3d71_sig_map`, `qpu_pack.c:164-197`;
//!   `v3d_qpu_sig_writes_address`, `qpu_instr.c:1095-1107`).
//! * **rf0 across a thread switch.** "If the most recent write to rf0 was
//!   from a ldunif, ldunifa, or ldvary instruction in which another signal
//!   also wrote to the register file, and the final instruction of the thread
//!   section contained a signal which wrote to the register file, then the
//!   value of rf0 is undefined at the start of the new section"
//!   (`qpu_schedule.c:1711-1745`, with the cycle of the rf0 write as Mesa's
//!   scoreboard counts it): rf0 then becomes undefined in every lane.
//! * **The fragment payload.** W is in rf3 (rf0 on 4.2), centroid W in rf1, Z
//!   in rf2 (`nir_to_vir.c:4851-4858`).
//! * **New operations.** The add ALU's MOV and FMOV (on 4.2 only the mul
//!   ALU's; `can_do_mul_as_add`, `qpu_schedule.c`), and the input unpacks of
//!   `qpu_instr.h:320-346`: on MOV (either ALU) UL/UH/IL/IH, the low or high
//!   16 bits zero- or sign-extended (`qpu_pack.c:1361-1368`, `1559-1566`); on
//!   the add FMOV SAT, NSAT and MAX0, a clamp of the f32 to [0, 1], [-1, 1]
//!   and [0, +inf] (`qpu_pack.c:1370-1382`; Mesa's `fsat`, `fsat_signed`,
//!   `fclamp_pos`, `nir_to_vir.c:1863-1895`). MOV sets Z as 4.2's MOV does
//!   (with no unpack; with one, no flag is described), FMOV Z and N as 4.2's
//!   FMOV does. ASSUMPTION: the clamps are IEEE min/max on the f32; -0.0
//!   under SAT or MAX0, whose result's sign is not described, is
//!   unmodelled. Every other operation and unpack 7.1 has and 4.2 has not
//!   (the half-float vector family, VPACK/V8PACK/V10PACK/V11FPACK, the
//!   subgroup operations, SETNNMODE, the mul-side normalised conversions,
//!   V8DOT) is unmodelled. The operations both have are computed by
//!   `sim::add_op` / `sim::mul_op`, the SFU ones (RECIP .. SIN, add
//!   operations on 7.1) included: their result goes to the register file and
//!   a read of it in the next instruction stalls (`read_stalls`,
//!   `qpu_schedule.c:779-786`, keyed on the add's waddr for every
//!   `v3d_qpu_instr_is_sfu` op, `qpu_schedule.c:1662-1669`), so it is there
//!   from the next instruction on, as for 4.2's register-file SFU writes.
//! * **Removed.** The accumulators and the magic SFU, VPM and TMUL writes
//!   (`qpu_validate.c:105-115`), LDVPM; the decoder refuses them
//!   ([`super::codec71`]).
//! * **Instruction rules.** One of the restricted peripherals per
//!   instruction, or WRTMUC with a TMU write other than TMUC; one TMU read,
//!   one TLB access (`qpu_compatible_peripheral_access` for 7.x,
//!   `qpu_schedule.c:936-975`, applied between the add half, the mul half and
//!   the signals). The validator's unit count without LDTMU
//!   (`qpu_validate.c:333-344`). No LDVARY in a THRSW's second delay slot
//!   (`qpu_validate.c:298-306`). At the thread end no ALU writes the
//!   register file in the THRSW's own instruction, and nothing writes rf2 or
//!   rf3 in it or its delay slots (`qpu_validate.c:373-421`); the 4.2
//!   GFXH-1625 TMUWT rule is not a 7.1 one.
//!
//! The LDTMU without a thread switch that hangs the Pi 4 ([`super::sim::
//! NO_SWITCH_HANG`]) is refused here too: nobody has measured it on 7.1, and
//! Mesa switches first whenever it is threaded.

use super::codec71::decode71;
use super::decode::DecodeError;
use super::isa::{num_src, AddAlu, AddOp, Alu, Input, Instr, MulAlu, MulOp, Mux, Pf, Sig, Uf, Unpack, ARG_D};
use super::isa71::*;
use super::qpu_ffi::Gen;
use super::sim::{
    self, f32_unpack, fl, peripheral_names, unmodelled, Computed, Dest, Env, FlagVals, Inputs, Machine, Outcome, Reg,
    LANES, P_SFU, P_TLB_READ, P_TLB_WRITE, P_TMU_READ, P_TMU_WAIT, P_TMU_WRITE, P_TSY, P_VPM_READ, P_VPM_WAIT,
    P_VPM_WRITE, P_WRTMUC, R,
};
use std::format;

/// The register W is in at a fragment shader's entry (rf0 on 4.2).
pub const PAYLOAD_W: usize = 3;

/// Where LDUNIF, LDUNIFA and LDVARY's C coefficient land (r5 on 4.2).
pub(super) const IMPLICIT: Reg = Reg::Rf(0);

/// Run 7.1 words: `sim::run_gen(Gen::V71, ..)`.
pub fn run71(code: &[u64], uniforms: &[u32], inputs: &Inputs, env: &Env) -> Result<Outcome, String> {
    sim::run_gen(Gen::V71, code, uniforms, inputs, env)
}

/// What a 7.1 run keeps beside the shared machine.
pub(super) struct State71 {
    /// The ALU instruction being run.
    pub(super) alu: Alu71,
    /// The cycle of the most recent implicit rf0 write, as Mesa's scoreboard
    /// counts it (the instruction's own for LDUNIF and LDUNIFA, the next for
    /// LDVARY).
    implicit_rf0_at: Option<usize>,
    /// That write coincided with a signal writing the register file.
    rf0_flops: bool,
    /// The last instruction whose signal wrote the register file.
    sig_rf_at: Option<usize>,
}

impl State71 {
    pub(super) fn new() -> State71 {
        State71 { alu: Alu71::nop(), implicit_rf0_at: None, rf0_flops: false, sig_rf_at: None }
    }
}

/// One 7.1 word as the shared loop runs it: the lowered instruction, and the
/// 7.1 ALU instruction when it is one.
pub(super) fn decode(w: u64) -> Result<(Instr, Option<Alu71>), DecodeError> {
    Ok(match decode71(w)? {
        Instr71::Branch(b) => (Instr::Branch(b), None),
        Instr71::Alu(a) => (Instr::Alu(lower(&a)), Some(a)),
    })
}

/// The 4.2 operation with this 7.1 operation's value, if 4.2 has it (the
/// discriminants are Mesa's enum values, the same for both generations).
fn add_op42(op: AddOp71) -> Option<AddOp> {
    AddOp::ALL.into_iter().find(|o| *o as u32 == op as u32)
}

fn mul_op42(op: MulOp71) -> Option<MulOp> {
    MulOp::ALL.into_iter().find(|o| *o as u32 == op as u32)
}

fn unpack42(u: Unpack71) -> Option<Unpack> {
    Unpack::ALL.into_iter().find(|o| *o as u32 == u as u32)
}

/// `a` in the 4.2 model's shape (see the module notes): what has no 4.2
/// equivalent (an operation, an unpack) lowers to NOP / no unpack, and is
/// never computed from the lowered form ([`add_op`], [`mul_op`]).
pub(super) fn lower(a: &Alu71) -> Alu {
    let s = a.sig;
    let input = |mux: Mux, o: Operand| Input { mux, unpack: unpack42(o.unpack).unwrap_or_default() };
    Alu {
        sig: Sig {
            thrsw: s.thrsw,
            ldunif: s.ldunif,
            ldunifa: s.ldunifa,
            ldunifrf: s.ldunifrf,
            ldunifarf: s.ldunifarf,
            ldtmu: s.ldtmu,
            ldvary: s.ldvary,
            ldtlb: s.ldtlb,
            ldtlbu: s.ldtlbu,
            ucb: s.ucb,
            rotate: s.rotate,
            wrtmuc: s.wrtmuc,
            small_imm_b: false,
        },
        sig_addr: a.sig_addr,
        sig_magic: a.sig_magic,
        flags: a.flags,
        raddr_a: 0,
        raddr_b: 0,
        add: AddAlu {
            op: add_op42(a.add.op).unwrap_or(AddOp::Nop),
            a: input(Mux::R0, a.add.a),
            b: input(Mux::R1, a.add.b),
            waddr: a.add.waddr,
            magic: a.add.magic,
            pack: a.add.pack,
        },
        mul: MulAlu {
            op: mul_op42(a.mul.op).unwrap_or(MulOp::Nop),
            a: input(Mux::R2, a.mul.a),
            b: input(Mux::R3, a.mul.b),
            waddr: a.mul.waddr,
            magic: a.mul.magic,
            pack: a.mul.pack,
        },
    }
}

/// The operand behind a lowered mux (R0..R3: add a, add b, mul a, mul b),
/// every lane, before its unpack.
pub(super) fn operand(m: &Machine, st: &State71, mux: Mux, what: &str) -> R<[u32; LANES]> {
    let Some(&(op, imm)) = st.alu.operands().get(mux as usize) else {
        return unmodelled(format!("{what}: operand {mux:?} on 7.1"));
    };
    if imm {
        let Some(v) = Alu71::small_imm(op) else {
            return unmodelled(format!("{what}: small immediate {} past the table", op.raddr));
        };
        return Ok([v; LANES]);
    }
    m.read(Reg::Rf(op.raddr), what)
}

/// [`operand`] as it is, undefined lanes included (a MOV straight to the
/// TLB, `sim::TlbWrite`).
pub(super) fn operand_lanes(m: &Machine, st: &State71, mux: Mux, what: &str) -> R<sim::Lanes> {
    match st.alu.operands().get(mux as usize) {
        Some(&(op, false)) => Ok(m.rf[op.raddr as usize]),
        _ => operand(m, st, mux, what).map(|v| v.map(Some)),
    }
}

fn src(m: &Machine, slot: Mux, what: &str) -> R<[u32; LANES]> {
    let Some(st) = &m.v71 else { return unmodelled(format!("{what}: a 7.1 operand outside a 7.1 run")) };
    operand(m, st, slot, what)
}

/// Whether each ALU half has an operation.
pub(super) fn ops_on(a: &Alu71) -> (bool, bool) {
    (a.add.op != AddOp71::Nop, a.mul.op != MulOp71::Nop)
}

/// A MOV's integer unpack (`qpu_instr.h:340-346`).
fn int_unpack(v: u32, u: Unpack71, what: &str) -> R<u32> {
    Ok(match u {
        Unpack71::None => v,
        Unpack71::Ul => v & 0xffff,
        Unpack71::Uh => v >> 16,
        Unpack71::Il => v as u16 as i16 as i32 as u32,
        Unpack71::Ih => (v >> 16) as u16 as i16 as i32 as u32,
        other => return unmodelled(format!("{what}: input unpack {other:?}")),
    })
}

/// An FMOV's float unpack: 4.2's, or one of 7.1's clamps
/// (`qpu_instr.h:320-325`).
fn float_unpack(v: u32, u: Unpack71, what: &str) -> R<u32> {
    let (lo, hi) = match u {
        Unpack71::Sat => (0.0f32, 1.0f32),
        Unpack71::Nsat => (-1.0, 1.0),
        Unpack71::Max0 => (0.0, f32::INFINITY),
        other => {
            let Some(u) = unpack42(other) else { return unmodelled(format!("{what}: input unpack {other:?}")) };
            return f32_unpack(v, u, what);
        }
    };
    let x = f32_unpack(v, Unpack::None, what)?;
    if x == 0x8000_0000 && lo == 0.0 {
        return unmodelled(format!(
            "{what}: -0.0 under the {u:?} unpack (the sign of the clamp's zero is not described)"
        ));
    }
    Ok(fl(x).clamp(lo, hi).to_bits())
}

/// The operands an operation reads have unpacks 4.2 has, so `sim::add_op`
/// / `sim::mul_op` can compute it from the lowered form.
fn unpacks42(n: usize, a: Operand, b: Operand, what: &str) -> R<()> {
    for o in [a, b].iter().take(n) {
        if unpack42(o.unpack).is_none() {
            return unmodelled(format!("{what}: input unpack {:?}", o.unpack));
        }
    }
    Ok(())
}

/// The add ALU of a 7.1 instruction: its MOV and FMOV here, what 4.2 has in
/// `sim::add_op` on the lowered form, the rest unmodelled.
pub(super) fn add_op(m: &mut Machine, a: &Alu71, lowered: &Alu) -> R<Option<Computed>> {
    let op = a.add.op;
    let what = format!("add {op:?}");
    let u = a.add.a.unpack;
    match op {
        AddOp71::Mov => {
            let x = src(m, Mux::R0, &what)?;
            let mut c = Computed::new(false);
            for l in 0..LANES {
                c.vals[l] = int_unpack(x[l], u, &what)?;
                if u == Unpack71::None {
                    c.flags[l].z = Some(c.vals[l] == 0);
                }
            }
            Ok(Some(c))
        }
        AddOp71::Fmov => {
            let x = src(m, Mux::R0, &what)?;
            let mut c = Computed::new(true);
            for l in 0..LANES {
                let v = float_unpack(x[l], u, &what)?;
                c.vals[l] = v;
                c.flags[l] = FlagVals { z: Some(fl(v) == 0.0), n: Some(fl(v) < 0.0), c: None };
            }
            Ok(Some(c))
        }
        _ => match add_op42(op) {
            Some(op42) => {
                unpacks42(num_src(super::isa::add_op_args(op42)), a.add.a, a.add.b, &what)?;
                sim::add_op(m, lowered)
            }
            None => unmodelled(what),
        },
    }
}

/// The mul ALU of a 7.1 instruction: MOV with an integer unpack here, what
/// 4.2 has in `sim::mul_op`, the rest unmodelled.
pub(super) fn mul_op(m: &mut Machine, a: &Alu71, lowered: &Alu) -> R<Option<Computed>> {
    let op = a.mul.op;
    let what = format!("mul {op:?}");
    let u = a.mul.a.unpack;
    match op {
        MulOp71::Mov if u != Unpack71::None => {
            let x = src(m, Mux::R2, &what)?;
            let mut c = Computed::new(false);
            for l in 0..LANES {
                c.vals[l] = int_unpack(x[l], u, &what)?;
            }
            Ok(Some(c))
        }
        _ => match mul_op42(op) {
            Some(op42) => {
                unpacks42(num_src(super::isa::mul_op_args(op42)), a.mul.a, a.mul.b, &what)?;
                sim::mul_op(m, lowered)
            }
            None => unmodelled(what),
        },
    }
}

/// The mul MOV straight to the TLB that may carry undefined lanes
/// (`sim::TlbWrite`), as on 4.2.
pub(super) fn tlb_mov(a: &Alu71) -> bool {
    a.mul.op == MulOp71::Mov
        && a.mul.magic
        && matches!(a.mul.waddr, 7 | 8)
        && a.mul.a.unpack == Unpack71::None
        && a.flags.mpf == Pf::None
        && a.flags.muf == Uf::None
}

/// Where a 7.1 ALU write goes: `sim::dest` for the magic registers it
/// models (NOP, TLB, TLBU, UNIFA, the TMU's), nothing else.
pub(super) fn dest(waddr: u8, magic: bool, what: &str) -> R<Dest> {
    if magic && !matches!(waddr, 6..=9 | 11..=13 | 32..=46) {
        return unmodelled(format!("{what}: a write to magic register {}", magic_waddr_name(waddr).unwrap_or("?")));
    }
    sim::dest(waddr, magic, what)
}

fn is_tmu(w: u8) -> bool {
    (11..=13).contains(&w) || (32..=46).contains(&w)
}

/// The validator's "Only one of [TMU, SFU, TSY, TLB read, VPM]" count for
/// 7.1: the magic writes of both halves and LDTLB/LDTLBU, not LDTMU
/// (`qpu_validate.c:333-344`). The SFU and VPM magic writes are reserved on
/// 7.1 and do not decode.
pub(super) fn units(a: &Alu71) -> usize {
    let one = |on: bool, magic: bool, w: u8| usize::from(on && magic && (is_tmu(w) || matches!(w, 7 | 8 | 16..=18)));
    let (add_on, mul_on) = ops_on(a);
    one(add_on, a.add.magic, a.add.waddr)
        + one(mul_on, a.mul.magic, a.mul.waddr)
        + usize::from(a.sig.ldtlb)
        + usize::from(a.sig.ldtlbu)
}

/// The peripherals (`qpu_peripherals`, qpu_schedule.c:855-892) of the add
/// half, the mul half and the signals of a 7.1 instruction.
fn peripherals(a: &Alu71) -> [u32; 3] {
    use AddOp71::*;
    let (add_on, mul_on) = ops_on(a);
    let magic = |on: bool, m: bool, w: u8, f: &dyn Fn(u8) -> bool| on && m && f(w);
    let half = |on: bool, m: bool, w: u8| {
        let mut p = 0;
        if magic(on, m, w, &is_tmu) {
            p |= P_TMU_WRITE;
        }
        if magic(on, m, w, &|w| w == 7 || w == 8) {
            p |= P_TLB_WRITE;
        }
        p
    };
    let mut add = half(add_on, a.add.magic, a.add.waddr);
    if matches!(a.add.op, Vpmsetup | LdvpmvIn | LdvpmvOut | LdvpmdIn | LdvpmdOut | Ldvpmp | LdvpmgIn | LdvpmgOut) {
        add |= P_VPM_READ;
    }
    if matches!(a.add.op, Vpmsetup | Stvpmv | Stvpmd | Stvpmp) {
        add |= P_VPM_WRITE;
    }
    if a.add.op == Vpmwt {
        add |= P_VPM_WAIT;
    }
    // v3d_qpu_instr_is_sfu (qpu_instr.c:788-811).
    if matches!(
        a.add.op,
        Recip | Rsqrt | Exp | Log | Sin | Rsqrt2 | Ballot | Bcastf | Alleq | Allfeq | Rotq | Rot | Shuffle
    ) {
        add |= P_SFU;
    }
    if magic(add_on, a.add.magic, a.add.waddr, &|w| (16..=18).contains(&w)) {
        add |= P_TSY;
    }
    if a.add.op == Tmuwt {
        add |= P_TMU_WAIT;
    }
    let mul = half(mul_on, a.mul.magic, a.mul.waddr);
    let mut sig = 0;
    if a.sig.ldtmu {
        sig |= P_TMU_READ;
    }
    if a.sig.wrtmuc {
        sig |= P_WRTMUC;
    }
    if a.sig.ldtlb || a.sig.ldtlbu {
        sig |= P_TLB_READ;
    }
    [add, mul, sig]
}

/// The per-instruction rules of 7.1 that 4.2's do not cover (the module
/// notes): its peripherals, and no LDVARY in a THRSW's second delay slot
/// (`last_slot`).
pub(super) fn check(a: &Alu71, last_slot: bool, what: &str) -> R<()> {
    if a.sig.ldvary && last_slot {
        return unmodelled(format!("{what}: an LDVARY in a THRSW's second delay slot"));
    }
    const RESTRICTED: u32 = P_TMU_WRITE | P_WRTMUC | P_TSY | P_TLB_READ | P_SFU | P_VPM_READ | P_VPM_WRITE;
    let parts = peripherals(a);
    let restricted: [u32; 3] = parts.map(|p| p & RESTRICTED);
    let with = |bits: u32| parts.iter().filter(|p| *p & bits != 0).count();
    let tmuc = |on: bool, m: bool, w: u8| on && m && w == 32;
    let (add_on, mul_on) = ops_on(a);
    let not_tmuc = !tmuc(add_on, a.add.magic, a.add.waddr) && !tmuc(mul_on, a.mul.magic, a.mul.waddr);
    let busy: std::vec::Vec<u32> = restricted.into_iter().filter(|r| *r != 0).collect();
    let wrtmuc_pair = busy.len() == 2 && busy.contains(&P_WRTMUC) && busy.contains(&P_TMU_WRITE) && not_tmuc;
    if (busy.len() > 1 && !wrtmuc_pair) || with(P_TMU_READ) > 1 || with(P_TLB_READ | P_TLB_WRITE) > 1 {
        return unmodelled(format!(
            "{what}: the peripherals {} in one instruction (V3D 7.1 allows one of TMU write, WRTMUC, TSY, TLB \
             read, SFU, VPM, or WRTMUC with a TMU write other than TMUC)",
            peripheral_names(parts[0] | parts[1] | parts[2])
        ));
    }
    Ok(())
}

/// The ALU halves that write the register file.
fn alu_rf_writes(a: &Alu71) -> [Option<u8>; 2] {
    let (add_on, mul_on) = ops_on(a);
    [(add_on && !a.add.magic).then_some(a.add.waddr), (mul_on && !a.mul.magic).then_some(a.mul.waddr)]
}

fn sig_rf_write(a: &Alu71) -> Option<u8> {
    (a.sig.writes_address() && !a.sig_magic).then_some(a.sig_addr)
}

/// The thread end's own THRSW instruction: no ALU writes the register file,
/// and no signal rf2 or rf3 (qpu_validate.c:373-421).
pub(super) fn at_thrend(a: &Alu71, what: &str) -> R<()> {
    if alu_rf_writes(a).iter().any(Option::is_some) {
        return unmodelled(format!("{what}: an ALU register-file write at the thread end"));
    }
    after_thrend(a, what)
}

/// A delay slot of the thread end: nothing writes rf2 or rf3.
pub(super) fn after_thrend(a: &Alu71, what: &str) -> R<()> {
    let [add, mul] = alu_rf_writes(a);
    if [add, mul, sig_rf_write(a)].iter().any(|w| matches!(w, Some(2 | 3))) {
        return unmodelled(format!("{what}: an rf2 or rf3 write after the thread end"));
    }
    Ok(())
}

/// After an instruction's signals: track the rf0 rule of a thread switch.
pub(super) fn signals_done(m: &mut Machine, tick: usize) {
    let Some(st) = m.v71.as_mut() else { return };
    let a = st.alu;
    let sig_rf = sig_rf_write(&a).is_some();
    // An explicit rf0 write (v3d71_qpu_writes_waddr_explicitly) is the most
    // recent rf0 write now.
    let explicit = (a.add.op != AddOp71::Nop && add_op_args(a.add.op) & ARG_D != 0 && !a.add.magic && a.add.waddr == 0)
        || (a.mul.op != MulOp71::Nop && mul_op_args(a.mul.op) & ARG_D != 0 && !a.mul.magic && a.mul.waddr == 0)
        || sig_rf_write(&a) == Some(0);
    if explicit {
        st.implicit_rf0_at = None;
        st.rf0_flops = false;
    }
    if a.sig.ldunif || a.sig.ldunifa || a.sig.ldvary {
        st.implicit_rf0_at = Some(if a.sig.ldvary { tick + 1 } else { tick });
        st.rf0_flops = false;
    }
    if sig_rf {
        if st.implicit_rf0_at == Some(tick) {
            st.rf0_flops = true;
        }
        st.sig_rf_at = Some(tick);
    }
}

/// A thread switch (after `Machine::thread_switch`): rf0 is undefined when
/// its most recent write was a flops conflict and the section's final
/// instruction had a signal writing the register file.
pub(super) fn thread_switch(m: &mut Machine) {
    let last = m.tick.checked_sub(1);
    let Some(st) = m.v71.as_mut() else { return };
    if st.rf0_flops && st.sig_rf_at.is_some() && st.sig_rf_at == last {
        m.rf[0] = [None; LANES];
    }
    st.rf0_flops = false;
    st.implicit_rf0_at = None;
}

#[cfg(test)]
mod tests {
    //! Hand-built 7.1 programs, encoded with `encode71` and run.
    use super::*;
    use crate::shaderc::online::v3d::codec71::encode71;
    use crate::shaderc::online::v3d::isa::small_imm_index;
    use crate::shaderc::online::v3d::sim::{FsInputs, Region, Varying, VsInputs};

    const W: f32 = 2.0;
    const Z: f32 = 0.5;

    fn thrsw(mut a: Alu71) -> Alu71 {
        a.sig.thrsw = true;
        a
    }

    fn rf(raddr: u8) -> Operand {
        Operand { raddr, unpack: Unpack71::None }
    }

    /// The add ALU writing `waddr` (register file unless `magic`).
    fn add(op: AddOp71, waddr: u8, magic: bool, a: Operand, b: Operand) -> Alu71 {
        let mut i = Alu71::nop();
        i.add = AddAlu71 { op, a, b, waddr, magic, pack: OutPack::None };
        i
    }

    fn mul(op: MulOp71, waddr: u8, magic: bool, a: Operand, b: Operand) -> Alu71 {
        let mut i = Alu71::nop();
        i.mul = MulAlu71 { op, a, b, waddr, magic, pack: OutPack::None };
        i
    }

    /// `mov tlb, rfN` on the mul ALU.
    fn to_tlb(r: u8) -> Alu71 {
        mul(MulOp71::Mov, 7, true, rf(r), Operand::default())
    }

    fn words(code: &[Alu71]) -> Vec<u64> {
        code.iter().map(|a| encode71(&Instr71::Alu(*a)).unwrap_or_else(|e| panic!("{a:?}: {e:?}"))).collect()
    }

    /// `body` between the last-section THRSW pair and the program end.
    fn program(body: &[Alu71]) -> Vec<u64> {
        let mut code = vec![thrsw(Alu71::nop()), thrsw(Alu71::nop())];
        code.extend_from_slice(body);
        code.extend([thrsw(Alu71::nop()), Alu71::nop(), Alu71::nop()]);
        words(&code)
    }

    fn fs_inputs(varyings: Vec<Varying>) -> Inputs {
        let f = |x: f32| [x.to_bits(); LANES];
        Inputs::Fragment(Box::new(FsInputs {
            w: f(W),
            w_centroid: f(W),
            z: f(Z),
            x: f(1.0),
            y: f(1.0),
            back_facing: [false; LANES],
            varyings,
        }))
    }

    fn fs(body: &[Alu71], uniforms: &[u32]) -> Result<Outcome, String> {
        run71(&program(body), uniforms, &fs_inputs(Vec::new()), &Env::default())
    }

    /// The one value every lane of TLB write `n` holds.
    fn tlb(o: &Outcome, n: usize) -> u32 {
        let v = o.tlb[n].value;
        assert!(v.iter().all(|x| *x == v[0]), "{v:?}");
        v[0].expect("defined")
    }

    fn tlb_f(o: &Outcome, n: usize) -> f32 {
        f32::from_bits(tlb(o, n))
    }

    #[test]
    fn an_add_reads_the_payload_and_reaches_the_tlb() {
        // W is in rf3 and Z in rf2 on 7.1; an SFU op is an add op whose
        // register-file result is there from the next instruction.
        let o = fs(
            &[
                add(AddOp71::Fadd, 10, false, rf(3), rf(2)),
                add(AddOp71::Recip, 11, false, rf(3), Operand::default()),
                to_tlb(10),
                to_tlb(11),
            ],
            &[],
        )
        .expect("runs");
        assert_eq!(o.tlb.len(), 2);
        assert_eq!(tlb_f(&o, 0), W + Z);
        assert_eq!(tlb_f(&o, 1), 1.0 / W);
        assert_eq!(o.approximate, ["recip"]);
        // rf0 holds nothing at entry on 7.1 (W moved to rf3).
        let o = fs(&[to_tlb(0)], &[]).expect("runs");
        assert!(o.tlb[0].value.iter().all(Option::is_none));
    }

    #[test]
    fn a_mul_with_a_small_immediate() {
        let two = small_imm_index(2.0f32.to_bits()).expect("2.0 is a small immediate");
        let mut fmul = mul(MulOp71::Fmul, 10, false, rf(3), Operand { raddr: two, unpack: Unpack71::None });
        fmul.sig.small_imm_d = true;
        let o = fs(&[fmul, mul(MulOp71::Fmul, 11, false, rf(3), rf(2)), to_tlb(10), to_tlb(11)], &[]).expect("runs");
        assert_eq!(tlb_f(&o, 0), W * 2.0);
        assert_eq!(tlb_f(&o, 1), W * Z);
    }

    #[test]
    fn the_new_unpacks() {
        let mut ld = Alu71::nop();
        ld.sig.ldunif = true; // rf0, one instruction later
        let unpacked = |u: Unpack71| Operand { raddr: 0, unpack: u };
        let o = fs(
            &[
                ld,
                add(AddOp71::Mov, 10, false, unpacked(Unpack71::Ih), Operand::default()),
                ld,
                mul(MulOp71::Mov, 11, false, unpacked(Unpack71::Ul), Operand::default()),
                ld,
                add(AddOp71::Fmov, 12, false, unpacked(Unpack71::Sat), Operand::default()),
                add(AddOp71::Fmov, 13, false, unpacked(Unpack71::Nsat), Operand::default()),
                to_tlb(10),
                to_tlb(11),
                to_tlb(12),
                to_tlb(13),
            ],
            &[0xfffe_0003, 0x8001_0004, 1.5f32.to_bits()],
        )
        .expect("runs");
        assert_eq!(tlb(&o, 0), (-2i32) as u32); // the high half, sign-extended
        assert_eq!(tlb(&o, 1), 4); // the low half, zero-extended
        assert_eq!(tlb_f(&o, 2), 1.0); // 1.5 saturated to [0, 1]
        assert_eq!(tlb_f(&o, 3), 1.0);
        // -0.0 under SAT is not described.
        let e = fs(&[ld, add(AddOp71::Fmov, 12, false, unpacked(Unpack71::Sat), Operand::default())], &[0x8000_0000])
            .unwrap_err();
        assert!(e.contains("-0.0 under the Sat unpack"), "{e}");
    }

    #[test]
    fn a_varying_load_puts_c_in_rf0_two_instructions_later() {
        let mut ldvary = Alu71::nop();
        ldvary.sig.ldvary = true;
        ldvary.sig_addr = 10;
        let vary = Varying { vary: [3.0f32.to_bits(); LANES], c: [0.25f32.to_bits(); LANES] };
        // (After the THRSW pair's second delay slot, where no LDVARY may be.)
        let body = [
            Alu71::nop(),
            ldvary,
            Alu71::nop(),
            mul(MulOp71::Fmul, 11, false, rf(10), rf(3)),
            add(AddOp71::Fadd, 12, false, rf(11), rf(0)),
            to_tlb(12),
        ];
        let o = run71(&program(&body), &[], &fs_inputs(vec![vary.clone()]), &Env::default()).expect("runs");
        assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25);
        // The next instruction does not see C yet: rf0 is still undefined.
        let o = run71(&program(&[Alu71::nop(), ldvary, to_tlb(0)]), &[], &fs_inputs(vec![vary]), &Env::default())
            .expect("runs");
        assert!(o.tlb[0].value.iter().all(Option::is_none));
    }

    #[test]
    fn uniform_loads_to_rf0_to_an_address_and_by_unifa() {
        let mut ldunif = Alu71::nop();
        ldunif.sig.ldunif = true;
        let ldunifrf = |addr: u8, magic: bool| {
            let mut a = Alu71::nop();
            a.sig.ldunifrf = true;
            a.sig_addr = addr;
            a.sig_magic = magic;
            a
        };
        let mut ldunifa = Alu71::nop();
        ldunifa.sig.ldunifa = true;
        let body = [
            ldunif,
            ldunifrf(5, false),
            add(AddOp71::Add, 10, false, rf(0), rf(5)),
            ldunifrf(9, true), // straight into UNIFA
            Alu71::nop(),
            Alu71::nop(),
            Alu71::nop(),
            ldunifa, // rf0, the word at the UNIFA address
            to_tlb(10),
            to_tlb(0),
        ];
        let env = Env { memory: vec![Region { base: 0x100, words: vec![Some(42)] }], ..Env::default() };
        let o = run71(&program(&body), &[7, 9, 0x100], &fs_inputs(Vec::new()), &env).expect("runs");
        assert_eq!(tlb(&o, 0), 16);
        assert_eq!(tlb(&o, 1), 42);
        assert_eq!(o.uniforms_read, 3);
    }

    #[test]
    fn tlb_writes_take_their_configuration_from_the_stream() {
        // An add MOV (7.1 has one) to TLBU, which reads its configuration;
        // a mul MOV of an undefined register (Mesa's "Z from FEP" write)
        // reaches the TLB as undefined.
        let o = fs(
            &[
                add(AddOp71::Mov, 8, true, rf(3), Operand::default()),
                mul(MulOp71::Mov, 8, true, rf(50), Operand::default()),
                to_tlb(2),
            ],
            &[0xffff_ff33, 0xffff_ff44],
        )
        .expect("runs");
        assert_eq!(o.tlb.len(), 3);
        assert_eq!(o.tlb[0].config, Some(0xffff_ff33));
        assert_eq!(tlb_f(&o, 0), W);
        assert_eq!(o.tlb[1].config, Some(0xffff_ff44));
        assert!(o.tlb[1].value.iter().all(Option::is_none));
        assert_eq!(o.tlb[2].config, None);
        assert_eq!(tlb_f(&o, 2), Z);
    }

    #[test]
    fn a_vertex_shader_reads_and_writes_the_vpm() {
        let zero = small_imm_index(0).expect("0 is a small immediate");
        let mut ld = add(AddOp71::LdvpmvIn, 10, false, Operand { raddr: zero, unpack: Unpack71::None }, rf(0));
        ld.sig.small_imm_a = true;
        let mut st = add(AddOp71::Stvpmv, 0, false, Operand { raddr: zero, unpack: Unpack71::None }, rf(11));
        st.sig.small_imm_a = true;
        let body = [ld, add(AddOp71::Fadd, 11, false, rf(10), rf(10)), st];
        let inputs = Inputs::Vertex(VsInputs { vpm: vec![[1.5f32.to_bits(); LANES]] });
        let o = run71(&program(&body), &[], &inputs, &Env::default()).expect("runs");
        assert_eq!(o.vpm_out, vec![Some([3.0f32.to_bits(); LANES])]);
    }

    #[test]
    fn what_7_1_does_not_model() {
        // A 7.1-only operation.
        let e = fs(&[add(AddOp71::Vpack, 10, false, rf(3), rf(2))], &[]).unwrap_err();
        assert!(e.contains("add Vpack"), "{e}");
        let e = fs(&[mul(MulOp71::Ftounorm16, 10, false, rf(3), Operand::default())], &[]).unwrap_err();
        assert!(e.contains("mul Ftounorm16"), "{e}");
        // QUAD, R5's place on 4.x.
        let e = fs(&[add(AddOp71::Fadd, WADDR_QUAD, true, rf(3), rf(2))], &[]).unwrap_err();
        assert!(e.contains("a write to magic register quad"), "{e}");
        // An LDVARY in the second delay slot of a THRSW.
        let mut ldvary = Alu71::nop();
        ldvary.sig.ldvary = true;
        ldvary.sig_addr = 10;
        let vary = Varying { vary: [0; LANES], c: [0; LANES] };
        let code = words(&[thrsw(Alu71::nop()), thrsw(Alu71::nop()), thrsw(Alu71::nop()), Alu71::nop(), ldvary]);
        let e = run71(&code, &[], &fs_inputs(vec![vary]), &Env::default()).unwrap_err();
        assert!(e.contains("an LDVARY in a THRSW's second delay slot"), "{e}");
        // An ALU register-file write in the thread end's THRSW.
        let code = words(&[
            thrsw(Alu71::nop()),
            thrsw(Alu71::nop()),
            thrsw(add(AddOp71::Fadd, 10, false, rf(3), rf(2))),
            Alu71::nop(),
            Alu71::nop(),
        ]);
        let e = run71(&code, &[], &fs_inputs(Vec::new()), &Env::default()).unwrap_err();
        assert!(e.contains("an ALU register-file write at the thread end"), "{e}");
    }

    /// rf0 after a thread switch: undefined when its last write (LDVARY's C)
    /// coincided with a signal writing the register file and the section's
    /// final instruction has one too; else what LDVARY left.
    #[test]
    fn rf0_across_a_thread_switch() {
        let mut ldvary = Alu71::nop();
        ldvary.sig.ldvary = true;
        ldvary.sig_addr = 10;
        let ldunifrf = |addr: u8| {
            let mut a = Alu71::nop();
            a.sig.ldunifrf = true;
            a.sig_addr = addr;
            a
        };
        let run = |conflict: bool, last: bool| {
            let code = words(&[
                ldvary,
                if conflict { ldunifrf(11) } else { Alu71::nop() },
                thrsw(Alu71::nop()),                            // the switch, after two delay slots
                thrsw(Alu71::nop()),                            // marks the last section
                if last { ldunifrf(12) } else { Alu71::nop() }, // the section's final instruction
                to_tlb(0),
                thrsw(Alu71::nop()),
                Alu71::nop(),
                Alu71::nop(),
            ]);
            let vary = Varying { vary: [0; LANES], c: [0.25f32.to_bits(); LANES] };
            run71(&code, &[1, 2], &fs_inputs(vec![vary]), &Env::default()).expect("runs")
        };
        assert!(run(true, true).tlb[0].value.iter().all(Option::is_none));
        assert_eq!(tlb_f(&run(true, false), 0), 0.25);
        assert_eq!(tlb_f(&run(false, true), 0), 0.25);
    }
}
