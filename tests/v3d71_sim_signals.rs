// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! sim71's signals at V3D 7.1 (`sim::run_gen(Gen::V71, ..)`): which register
//! each of ldunif, ldunifrf, ldunifa, ldunifarf, ldvary and ldtmu writes, at
//! which instruction, with which value (the uniform stream, the varying
//! inputs, a TMU result), the order the uniform stream is read in, and the two
//! delay slots of thrsw. Hand-written `Instr71` programs, encoded with
//! `encode71`, so the codec is in the loop; the expectations are Mesa 26.2.1's
//! (`src/broadcom`, cited file:line).
//!
//! Features: --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table

#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d"))]

use yakogl::shaderc::online::texref::Texture;
use yakogl::shaderc::online::v3d::codec71::encode71;
use yakogl::shaderc::online::v3d::isa71::*;
use yakogl::shaderc::online::v3d::sim::{Env, FsInputs, Inputs, Outcome, Region, Varying, LANES, NO_SWITCH_HANG};
use yakogl::shaderc::online::v3d::sim71::run71;

const W: f32 = 2.0;
const WC: f32 = 3.0;
const Z: f32 = 0.5;

fn rf(raddr: u8) -> Operand {
    Operand { raddr, unpack: Unpack71::None }
}

fn nop() -> Alu71 {
    Alu71::nop()
}

fn thrsw() -> Alu71 {
    let mut a = nop();
    a.sig.thrsw = true;
    a
}

fn mul(op: MulOp71, waddr: u8, magic: bool, a: Operand, b: Operand) -> Alu71 {
    let mut i = nop();
    i.mul = MulAlu71 { op, a, b, waddr, magic, pack: OutPack::None };
    i
}

/// `mov rfN, rfM` on the mul ALU.
fn mov(dst: u8, src: u8) -> Alu71 {
    mul(MulOp71::Mov, dst, false, rf(src), Operand::default())
}

/// `mov tlb, rfN` on the mul ALU.
fn to_tlb(r: u8) -> Alu71 {
    mul(MulOp71::Mov, 7, true, rf(r), Operand::default())
}

/// `sig` on an otherwise empty instruction.
fn with_sig(f: impl FnOnce(&mut Sig71)) -> Alu71 {
    let mut a = nop();
    f(&mut a.sig);
    a
}

/// ldunif: writes rf0 implicitly (`v3d_qpu_writes_rf0_implicitly`,
/// qpu_instr.c:1031-1040).
fn ldunif() -> Alu71 {
    with_sig(|s| s.ldunif = true)
}

fn ldunifa() -> Alu71 {
    with_sig(|s| s.ldunifa = true)
}

/// ldunifrf rfN: an explicit destination (`v3d_qpu_sig_writes_address`,
/// qpu_instr.c:1095-1107).
fn ldunifrf(addr: u8) -> Alu71 {
    let mut a = with_sig(|s| s.ldunifrf = true);
    a.sig_addr = addr;
    a
}

fn ldunifarf(addr: u8) -> Alu71 {
    let mut a = with_sig(|s| s.ldunifarf = true);
    a.sig_addr = addr;
    a
}

fn ldvary(addr: u8) -> Alu71 {
    let mut a = with_sig(|s| s.ldvary = true);
    a.sig_addr = addr;
    a
}

fn ldtmu(addr: u8) -> Alu71 {
    let mut a = with_sig(|s| s.ldtmu = true);
    a.sig_addr = addr;
    a
}

fn words(code: &[Alu71]) -> Vec<u64> {
    code.iter().map(|a| encode71(&Instr71::Alu(*a)).unwrap_or_else(|e| panic!("{a:?}: {e:?}"))).collect()
}

/// `body` between the last-section THRSW pair and the program end.
fn program(body: &[Alu71]) -> Vec<u64> {
    let mut code = vec![thrsw(), thrsw()];
    code.extend_from_slice(body);
    code.extend([thrsw(), nop(), nop()]);
    words(&code)
}

fn vary(v: f32, c: f32) -> Varying {
    Varying { vary: [v.to_bits(); LANES], c: [c.to_bits(); LANES] }
}

fn fs_inputs(varyings: Vec<Varying>) -> Inputs {
    let f = |x: f32| [x.to_bits(); LANES];
    Inputs::Fragment(Box::new(FsInputs {
        w: f(W),
        w_centroid: f(WC),
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

/// The one value every lane of TLB write `n` holds (`None`: undefined).
fn tlb(o: &Outcome, n: usize) -> Option<u32> {
    let v = o.tlb[n].value;
    assert!(v.iter().all(|x| *x == v[0]), "{v:?}");
    v[0]
}

fn tlb_f(o: &Outcome, n: usize) -> f32 {
    f32::from_bits(tlb(o, n).expect("defined"))
}

// ---------------------------------------------------------------------------
// The payload.
// ---------------------------------------------------------------------------

/// The fragment payload on 7.1: W in rf3 (rf0 on 4.2), centroid W in rf1, Z in
/// rf2 (nir_to_vir.c:4851-4858); rf0 holds nothing at entry.
#[test]
fn the_payload_is_w_in_rf3_wc_in_rf1_z_in_rf2() {
    let o = fs(&[to_tlb(3), to_tlb(1), to_tlb(2), to_tlb(0)], &[]).expect("runs");
    assert_eq!(tlb_f(&o, 0), W);
    assert_eq!(tlb_f(&o, 1), WC);
    assert_eq!(tlb_f(&o, 2), Z);
    assert_eq!(tlb(&o, 3), None, "rf0 is undefined at entry on 7.1");
}

// ---------------------------------------------------------------------------
// ldunif / ldunifrf: the uniform stream.
// ---------------------------------------------------------------------------

/// ldunif writes rf0 implicitly (qpu_instr.c:1031-1040), one instruction
/// later, as r5 does on 4.2: an ALU in the ldunif's own instruction reads
/// rf0's old value, the next instruction the uniform (qpu_schedule.c:1462-1470:
/// "ldunif and ldvary both write the same register ... but ldunif does so a
/// tick sooner").
#[test]
fn ldunif_lands_in_rf0_one_instruction_later() {
    let mut same = ldunif();
    same.mul = mov(10, 0).mul;
    let o = fs(&[ldunif(), same, to_tlb(10), to_tlb(0)], &[0x1111, 0x2222]).expect("runs");
    // `same` read rf0 = the first word; its own ldunif then replaced rf0.
    assert_eq!(tlb(&o, 0), Some(0x1111));
    assert_eq!(tlb(&o, 1), Some(0x2222));
    assert_eq!(o.uniforms_read, 2);
    // In the ldunif's own instruction rf0 is still undefined.
    let mut first = ldunif();
    first.mul = mov(10, 0).mul;
    let o = fs(&[first, to_tlb(10), to_tlb(0)], &[0x1111]).expect("runs");
    assert_eq!(tlb(&o, 0), None);
    assert_eq!(tlb(&o, 1), Some(0x1111));
}

/// ldunifrf names its register (v3d71_sig_map, qpu_pack.c:164-197), reaching it
/// one instruction later; rf0 is not touched.
#[test]
fn ldunifrf_writes_its_own_register_one_instruction_later() {
    let mut same = ldunifrf(5);
    same.mul = mov(11, 5).mul;
    let o = fs(&[ldunifrf(5), same, to_tlb(11), to_tlb(5), to_tlb(0)], &[0xaaaa, 0xbbbb]).expect("runs");
    assert_eq!(tlb(&o, 0), Some(0xaaaa), "read before the second ldunifrf lands");
    assert_eq!(tlb(&o, 1), Some(0xbbbb));
    assert_eq!(tlb(&o, 2), None, "rf0 untouched by ldunifrf");
    // The top of the register file is addressable too.
    let o = fs(&[ldunifrf(63), to_tlb(63)], &[0x3f3f]).expect("runs");
    assert_eq!(tlb(&o, 0), Some(0x3f3f));
}

/// ldunif and ldunifrf draw from one stream, in program order, and the stream
/// ends where the shader says (`uniforms_read`).
#[test]
fn ldunif_and_ldunifrf_read_the_stream_in_program_order() {
    let o = fs(
        &[
            ldunif(),    // u0 -> rf0
            ldunifrf(5), // u1 -> rf5
            mov(10, 0),  // rf0 = u0
            ldunif(),    // u2 -> rf0
            ldunifrf(6), // u3 -> rf6
            mov(11, 0),  // rf0 = u2
            ldunifrf(7), // u4 -> rf7
            nop(),
            to_tlb(10),
            to_tlb(5),
            to_tlb(11),
            to_tlb(6),
            to_tlb(7),
        ],
        &[10, 11, 12, 13, 14],
    )
    .expect("runs");
    let got: Vec<_> = (0..5).map(|n| tlb(&o, n)).collect();
    assert_eq!(got, [Some(10), Some(11), Some(12), Some(13), Some(14)]);
    assert_eq!(o.uniforms_read, 5);
}

/// A load past the stream's end is refused, not read as zero.
#[test]
fn a_load_past_the_uniform_stream_is_refused() {
    let e = fs(&[ldunif(), ldunifrf(5)], &[1]).unwrap_err();
    assert!(e.to_lowercase().contains("uniform"), "{e}");
}

/// ldunifrf to the magic UNIFA (waddr 9) sets the address `ldunifa` reads
/// (nir_to_vir.c:3340-3362); ldunifa writes rf0, ldunifarf an explicit
/// register, the address advancing by one word each. Neither reads the stream.
#[test]
fn ldunifa_and_ldunifarf_read_memory_at_the_unifa_address() {
    let mut to_unifa = ldunifrf(9);
    to_unifa.sig_magic = true;
    // (Clear of the THRSW pair's switch: no UNIFA write within 3 of one.)
    let body = [
        nop(),
        nop(),
        nop(),
        nop(),
        to_unifa,
        nop(),
        nop(),
        nop(),
        ldunifa(),
        ldunifarf(6),
        nop(),
        mov(10, 0),
        to_tlb(10),
        to_tlb(6),
    ];
    let env = Env { memory: vec![Region { base: 0x100, words: vec![Some(42), Some(43)] }], ..Env::default() };
    let o = run71(&program(&body), &[0x100], &fs_inputs(Vec::new()), &env).expect("runs");
    assert_eq!(o.uniforms_read, 1, "ldunifa/ldunifarf read memory, not the stream");
    assert_eq!(tlb(&o, 0), Some(42));
    assert_eq!(tlb(&o, 1), Some(43));
}

// ---------------------------------------------------------------------------
// ldvary.
// ---------------------------------------------------------------------------

/// ldvary writes its explicit destination one instruction later and the C
/// coefficient to rf0 one cycle after that ("ldvary delayed write of C
/// coefficient to rf0", qpu_schedule.c:626-629 and 715-726).
#[test]
fn ldvary_writes_p_to_its_register_and_c_to_rf0_a_cycle_later() {
    // (After the THRSW pair's second delay slot, where no LDVARY may be.)
    let mut at_1 = mov(11, 0); // rf0 read at +1
    at_1.add =
        AddAlu71 { op: AddOp71::Mov, a: rf(10), b: Operand::default(), waddr: 13, magic: false, pack: OutPack::None };
    let body = [nop(), ldvary(10), at_1, mov(12, 0), to_tlb(10), to_tlb(13), to_tlb(11), to_tlb(12)];
    let o = run71(&program(&body), &[], &fs_inputs(vec![vary(3.0, 0.25)]), &Env::default()).expect("runs");
    assert_eq!(tlb_f(&o, 0), 3.0, "P is in its register at +1 ...");
    assert_eq!(tlb_f(&o, 1), 3.0, "... and an add reads it there");
    assert_eq!(tlb(&o, 2), None, "C is not yet in rf0 at +1");
    assert_eq!(tlb_f(&o, 3), 0.25, "C is in rf0 at +2");
}

/// Two ldvary take successive varyings; the later one's C replaces the
/// earlier's in rf0 (qpu_schedule.c:715-726 lets a second ldvary follow the
/// first).
#[test]
fn successive_ldvarys_take_successive_varyings() {
    let body = [nop(), ldvary(10), ldvary(11), nop(), to_tlb(10), to_tlb(11), to_tlb(0)];
    let inputs = fs_inputs(vec![vary(1.0, 0.5), vary(2.0, 0.75)]);
    let o = run71(&program(&body), &[], &inputs, &Env::default()).expect("runs");
    assert_eq!(tlb_f(&o, 0), 1.0);
    assert_eq!(tlb_f(&o, 1), 2.0);
    assert_eq!(tlb_f(&o, 2), 0.75);
    // Running out of varyings is refused by name.
    let e = run71(&program(&body), &[], &fs_inputs(vec![vary(1.0, 0.5)]), &Env::default()).unwrap_err();
    assert!(e.contains("LDVARY number 1 has no varying"), "{e}");
}

/// P (the ldvary's register), C (rf0) and W (rf3) combine as Mesa's
/// interpolation does: P * W + C.
#[test]
fn ldvary_with_w_from_rf3() {
    let mut fadd = nop();
    fadd.add = AddAlu71 { op: AddOp71::Fadd, a: rf(11), b: rf(0), waddr: 12, magic: false, pack: OutPack::None };
    let body = [nop(), ldvary(10), nop(), mul(MulOp71::Fmul, 11, false, rf(10), rf(3)), fadd, to_tlb(12)];
    let o = run71(&program(&body), &[], &fs_inputs(vec![vary(3.0, 0.25)]), &Env::default()).expect("runs");
    assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25);
}

/// An ldvary in the second delay slot of a THRSW is refused (qpu_validate.c:298-306);
/// one in the first, or after both, is not.
#[test]
fn ldvary_and_the_two_delay_slots_of_thrsw() {
    let run = |slots: [Alu71; 3]| {
        let mut code = vec![thrsw(), thrsw(), thrsw()];
        code.extend(slots);
        run71(&words(&code), &[], &fs_inputs(vec![vary(1.0, 1.0)]), &Env::default())
    };
    run([ldvary(10), nop(), nop()]).expect("slot 1");
    let e = run([nop(), ldvary(10), nop()]).unwrap_err();
    assert!(e.contains("an LDVARY in a THRSW's second delay slot"), "{e}");
    run([nop(), nop(), ldvary(10)]).expect("after both slots");
}

// ---------------------------------------------------------------------------
// ldtmu and thrsw.
// ---------------------------------------------------------------------------

/// A TMU lookup of the 1x1 texture [0.5, 0.25, 0.125, 1], R only (P0 = 1,
/// P1 = 1: one 32-bit word): the coordinate by ldunifrf, then P0 (wrtmuc), P1 with the T write
/// (wrtmuc + TMUT), the S write that issues it (magic 33, TMUS). Its uniforms:
/// the coordinate, P0, P1.
fn lookup() -> Vec<Alu71> {
    let mut c0 = nop();
    c0.sig.wrtmuc = true;
    let mut c1 = mul(MulOp71::Mov, 34, true, rf(5), Operand::default());
    c1.sig.wrtmuc = true;
    vec![ldunifrf(5), nop(), c0, c1, mul(MulOp71::Mov, 33, true, rf(5), Operand::default())]
}

const LOOKUP_UNIFORMS: [u32; 3] = [0x3e99_999a, 0x1, 0x1];

/// `body` after one thread switch (a THRSW and two delay slots), then the last
/// section's THRSW pair and the end, with one texture and `threads` threads.
fn tex(threads: u32, uniforms: &[u32], body: &[Alu71]) -> Result<Outcome, String> {
    let mut code = vec![thrsw(), nop(), nop()];
    code.extend_from_slice(body);
    code.extend([thrsw(), thrsw(), thrsw(), nop(), nop()]);
    let env = Env { textures: vec![Texture::flat(1, 1, vec![[0.5, 0.25, 0.125, 1.0]])], threads, ..Env::default() };
    run71(&words(&code), uniforms, &fs_inputs(Vec::new()), &env)
}

/// ldtmu pops the lookup's result into its explicit register, one instruction
/// later (`v3d71_sig_map`, qpu_pack.c:164-197; `v3d_qpu_sig_writes_address`,
/// qpu_instr.c:1095-1107); a read in its own instruction sees the old value.
/// The switch before it is Mesa's (`ntq_flush_tmu`).
#[test]
fn ldtmu_writes_its_register_one_instruction_later() {
    let mut body = lookup();
    body.extend([thrsw(), nop(), nop(), ldunifrf(20), nop()]);
    let mut ld = ldtmu(20);
    ld.mul = mov(21, 20).mul;
    body.extend([ld, to_tlb(21), to_tlb(20), to_tlb(0)]);
    let uniforms = [LOOKUP_UNIFORMS[0], LOOKUP_UNIFORMS[1], LOOKUP_UNIFORMS[2], 0xdead];
    let o = tex(4, &uniforms, &body).expect("runs");
    assert_eq!(tlb(&o, 0), Some(0xdead), "the old rf20, read in ldtmu's own instruction");
    assert_eq!(tlb_f(&o, 1), 0.5, "the texel's R");
    assert_eq!(tlb(&o, 2), None, "rf0 is not an ldtmu destination");
    assert_eq!((o.tmu_lookups, o.ldtmus, o.ldtmus_without_switch), (1, 1, 0));
}

/// thrsw takes effect after two delay slots: an ldtmu in either of them still
/// has no switch since its request (refused in a threaded program,
/// `NO_SWITCH_HANG`), one in the third instruction has (qpu_schedule.c:731-741
/// counts `last_thrsw_tick + 3`).
#[test]
fn thrsw_has_two_delay_slots_before_its_switch() {
    let after = |slots_before_ld: usize| {
        let mut body = lookup();
        body.push(thrsw());
        body.extend(std::iter::repeat_n(nop(), slots_before_ld));
        body.extend([ldtmu(20), nop(), to_tlb(20)]);
        tex(4, &LOOKUP_UNIFORMS, &body)
    };
    for n in [0, 1] {
        let e = after(n).unwrap_err();
        assert!(e.contains(NO_SWITCH_HANG), "{n} slots: {e}");
    }
    let o = after(2).expect("the third instruction is in the new section");
    assert_eq!((o.ldtmus, o.ldtmus_without_switch), (1, 0));
    assert_eq!(tlb_f(&o, 0), 0.5);
}

/// An ldtmu with no lookup waiting is refused by name.
#[test]
fn ldtmu_with_nothing_requested_is_refused() {
    let e = tex(4, &[], &[ldtmu(20)]).unwrap_err();
    assert!(e.contains("LDTMU with no TMU result waiting"), "{e}");
}
