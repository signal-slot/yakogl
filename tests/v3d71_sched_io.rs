// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The V3D 7.1 scheduler's I/O signals (plan #365, item sched71-io): how
//! `sched71::pack71` packs `ldunif`/`ldunifrf`, `ldvary` and its rf0 C term,
//! the TMU sequence and the TLB writes of hand-built [`Op71`] lists under
//! hand-built [`Allocation71`]s (no select71, no alloc71).
//!
//! The rules (sched71.rs's header, Mesa 26.2.1, MIT):
//!
//! * `ldunif` into rf0 is the plain signal; any other destination is
//!   `ldunifrf`, naming it in the condition field, so that word has no
//!   condition and no flag push (vir_to_qpu.c:328-350).
//! * `ldvary`'s C reaches rf0 one instruction later and is read two
//!   instructions later; nothing writes rf0 in the instruction right after an
//!   `ldvary` (qpu_schedule.c:623-635, 713-727, 1739-1742;
//!   qpu_validate.c:220-227).
//! * The TMU: `wrtmuc` before its look-up's register writes, the next
//!   look-up's configuration after the write that retired the last one, an
//!   `ldtmu` after that write (qpu_schedule.c:188-202, 249-250, 511-516); no
//!   new look-up beside an earlier `ldtmu` (docs/notes/v3d71-backend.md (d)).
//! * One TLB access, one TMU read, one TMU register write and one uniform per
//!   word (qpu_schedule.c:937-975).
//!
//! Each program is packed, every word round-trips through codec71
//! (`encode71` then `decode71` gives the instruction back), passes
//! `verify71` and `registers_fit_threads71` at 4 threads, and runs in
//! `sim::run_gen(Gen::V71, ..)` with the packed uniform stream, which it must
//! read to the end, and gives the expected TLB values.
//!
//! Run with `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d71_sched_io`.

use yakogl::shaderc::online::store;
use yakogl::shaderc::online::v3d::alloc71::{Allocation71, Rf};
use yakogl::shaderc::online::v3d::codec71::{decode71, encode71};
use yakogl::shaderc::online::v3d::isa::small_imm_index;
use yakogl::shaderc::online::v3d::isa::{Cond, Pf, Uf};
use yakogl::shaderc::online::v3d::isa71::{add_op_args, mul_op_args, Alu71, Flags, ARG_D};
use yakogl::shaderc::online::v3d::listing71::write_instr71;
use yakogl::shaderc::online::v3d::qpu_ffi::Gen;
use yakogl::shaderc::online::v3d::sched71::pack71;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Outcome, Texture, Varying, LANES};
use yakogl::shaderc::online::v3d::verify71::{registers_fit_threads71, verify71};
use yakogl::shaderc::online::v3d::vop71::{AddOp71, Dst71, Instr71, MulOp71, Op71, Src71, Uniform, V};
use yakogl::shaderc::online::v3d::vop71::{FlagOp71, Sfu71};

/// The fragment payload's W (rf3) and Z (rf2).
const W: f32 = 2.0;
const Z: f32 = 0.5;

/// The magic write addresses used here (`enum v3d_qpu_waddr`).
const TLB: u8 = 7;
const TLBU: u8 = 8;
const TMUS: u8 = 33;
const TMUT: u8 = 34;

/// A TMU register (`v3d_qpu_magic_waddr_is_tmu`, qpu_instr.c:633-648).
fn is_tmu(w: u8) -> bool {
    (11..=13).contains(&w) || (32..=46).contains(&w)
}

fn v(n: u32) -> Src71 {
    Src71::V(V(n))
}
fn f(x: f32) -> Uniform {
    Uniform::constant(x.to_bits())
}
fn imm_f(x: f32) -> Src71 {
    Src71::SmallImm(small_imm_index(x.to_bits()).expect("a small immediate"))
}
fn mov(dst: Dst71, n: u32) -> Op71 {
    Op71::Mul { op: MulOp71::Mov, dst, a: v(n), b: Src71::None }
}
fn add_mov(dst: Dst71, n: u32) -> Op71 {
    Op71::Add { op: AddOp71::Mov, dst, a: v(n), b: Src71::None }
}
fn fadd(d: u32, a: Src71, b: Src71) -> Op71 {
    Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(V(d)), a, b }
}
fn fmul(d: u32, a: Src71, b: Src71) -> Op71 {
    Op71::Mul { op: MulOp71::Fmul, dst: Dst71::V(V(d)), a, b }
}

fn alu(i: &Instr71) -> &Alu71 {
    match i {
        Instr71::Alu(a) => a,
        Instr71::Branch(_) => panic!("a branch in straight-line code"),
    }
}

fn listing(instrs: &[Instr71]) -> String {
    let mut s = String::new();
    for (k, i) in instrs.iter().enumerate() {
        s.push_str(&format!("{k:3}: "));
        write_instr71(&mut s, i).unwrap();
        s.push('\n');
    }
    s
}

/// `ops` under registers `regs` (indexed by virtual register), packed: the
/// instructions and the uniform words.
fn pack(ops: &[Op71], regs: &[u8]) -> (Vec<Instr71>, Vec<u32>) {
    /* the scratch region is one per process and the tests run in parallel
     * (store's own lock exists only in the library's unit tests) */
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _serial = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let rfs: Vec<Rf> = regs.iter().map(|&r| Rf(r)).collect();
    store::with_scratch(|a| {
        let al = Allocation71 { reg: &rfs, rf_end: regs.iter().copied().max().unwrap_or(0) + 1 };
        pack71(ops, &al, a).map(|p| (p.instrs.to_vec(), p.uniforms.iter().map(|u| u.data).collect::<Vec<u32>>()))
    })
    .expect("scratch")
    .unwrap_or_else(|e| panic!("pack71: {e:?}"))
}

/// The add unit writes a register-file register.
fn add_writes_rf(a: &Alu71) -> Option<u8> {
    (a.add.op != AddOp71::Nop && add_op_args(a.add.op) & ARG_D != 0 && !a.add.magic).then_some(a.add.waddr)
}
fn mul_writes_rf(a: &Alu71) -> Option<u8> {
    (a.mul.op != MulOp71::Nop && mul_op_args(a.mul.op) & ARG_D != 0 && !a.mul.magic).then_some(a.mul.waddr)
}
/// The magic addresses the word's ALUs write.
fn magic_writes(a: &Alu71) -> Vec<u8> {
    let mut m = Vec::new();
    if a.add.op != AddOp71::Nop && add_op_args(a.add.op) & ARG_D != 0 && a.add.magic {
        m.push(a.add.waddr);
    }
    if a.mul.op != MulOp71::Nop && mul_op_args(a.mul.op) & ARG_D != 0 && a.mul.magic {
        m.push(a.mul.waddr);
    }
    m
}

/// The word writes rf0, by name or implicitly as of this word (a plain
/// `ldunif`; an `ldvary`'s C lands in the next word and is not counted here).
fn writes_rf0(a: &Alu71) -> bool {
    add_writes_rf(a) == Some(0)
        || mul_writes_rf(a) == Some(0)
        || (a.sig.writes_address() && !a.sig_magic && a.sig_addr == 0)
        || a.sig.ldunif
        || a.sig.ldunifa
}

/// The word reads rf0 through a register operand.
fn reads_rf0(a: &Alu71) -> bool {
    a.read_operands().any(|(o, imm)| !imm && o.raddr == 0)
}

/// The uniforms the word consumes (`ldunif`, `ldunifrf`, `wrtmuc`, a TLBU
/// write).
fn uniforms_in(a: &Alu71) -> usize {
    a.sig.ldunif as usize
        + a.sig.ldunifrf as usize
        + a.sig.wrtmuc as usize
        + magic_writes(a).iter().filter(|&&w| w == TLBU).count()
}

/// The per-word rules every packed program keeps: (b)'s encoding of
/// `ldunifrf`, (c)'s rf0 after an `ldvary`, (d)'s per-word limits, and the
/// uniform count against the stream.
fn check_words(instrs: &[Instr71], unif: &[u32]) {
    let text = listing(instrs);
    let mut uniforms = 0;
    for (k, i) in instrs.iter().enumerate() {
        let a = alu(i);
        let u = uniforms_in(a);
        assert!(u <= 1, "word {k} consumes {u} uniforms\n{text}");
        uniforms += u;
        if a.sig.ldunifrf {
            assert!(!a.sig_magic, "word {k}: ldunifrf to a magic address\n{text}");
            assert_eq!(a.flags, Flags::default(), "word {k}: ldunifrf with a condition or flag push\n{text}");
        }
        let m = magic_writes(a);
        let tlb = m.iter().filter(|&&w| w == TLB || w == TLBU).count();
        assert!(tlb <= 1, "word {k}: {tlb} TLB accesses\n{text}");
        let tmu = m.iter().filter(|&&w| is_tmu(w)).count();
        assert!(tmu <= 1, "word {k}: {tmu} TMU writes\n{text}");
        if a.sig.ldvary {
            assert_ne!(a.sig_addr, 0, "word {k}: ldvary into rf0, where its C lands\n{text}");
            if let Some(n) = instrs.get(k + 1) {
                let n = alu(n);
                assert!(!writes_rf0(n), "word {}: rf0 written right after the ldvary of word {k}\n{text}", k + 1);
                assert!(!reads_rf0(n), "word {}: rf0 read right after the ldvary of word {k}\n{text}", k + 1);
            }
        }
    }
    assert_eq!(uniforms, unif.len(), "the words' uniforms against the stream\n{text}");
}

fn fs_inputs(varyings: Vec<Varying>) -> Inputs {
    let b = |x: f32| [x.to_bits(); LANES];
    Inputs::Fragment(Box::new(FsInputs {
        w: b(W),
        w_centroid: b(W),
        z: b(Z),
        x: b(1.0),
        y: b(1.0),
        back_facing: [false; LANES],
        varyings,
    }))
}

fn vary(x: f32, c: f32) -> Varying {
    Varying { vary: [x.to_bits(); LANES], c: [c.to_bits(); LANES] }
}

/// Pack a fragment shader, round-trip, verify, check the words and run it.
fn run(ops: &[Op71], regs: &[u8], varyings: Vec<Varying>, textures: Vec<Texture>) -> (Vec<Instr71>, Vec<u32>, Outcome) {
    let (instrs, unif) = pack(ops, regs);
    let text = listing(&instrs);
    let words: Vec<u64> =
        instrs.iter().map(|i| encode71(i).unwrap_or_else(|e| panic!("{e:?} encoding {i:?}\n{text}"))).collect();
    for (k, (i, w)) in instrs.iter().zip(&words).enumerate() {
        assert_eq!(decode71(*w).expect("decodes"), *i, "word {k} round trip\n{text}");
    }
    verify71(&instrs, false).unwrap_or_else(|e| panic!("verify71: {e:?}\n{text}"));
    registers_fit_threads71(&instrs, 4).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
    check_words(&instrs, &unif);
    let env = Env { textures, threads: 4, ..Env::default() };
    let o = sim::run_gen(Gen::V71, &words, &unif, &fs_inputs(varyings), &env)
        .unwrap_or_else(|e| panic!("sim71: {e}\n{text}"));
    assert_eq!(o.uniforms_read, unif.len(), "{text}");
    (instrs, unif, o)
}

/// The one value every lane of TLB write `n` holds.
fn tlb_f(o: &Outcome, n: usize) -> f32 {
    let l = o.tlb[n].value;
    assert!(l.iter().all(|x| *x == l[0]), "{l:?}");
    f32::from_bits(l[0].expect("defined"))
}

/// The word of the `n`-th instruction (from 0) for which `p` holds.
fn nth(instrs: &[Instr71], n: usize, p: impl Fn(&Alu71) -> bool) -> usize {
    instrs.iter().enumerate().filter(|(_, i)| p(alu(i))).nth(n).map(|(k, _)| k).expect("found")
}

fn writes_magic(a: &Alu71, w: u8) -> bool {
    magic_writes(a).contains(&w)
}

// ---------------------------------------------------------------------------
// (a) The uniform stream.
// ---------------------------------------------------------------------------

/// `ldunif`, `ldunifrf`, two `wrtmuc`s and a TLBU write: the packed stream is
/// their uniforms in program order, and each word's uniform is the one the
/// stream names at its place.
#[test]
fn the_uniform_stream_keeps_the_program_order() {
    const P0: u32 = 0x0000_0001; // texture 0, one word returned
    const P1: u32 = 0x0000_0001; // 32-bit results
    const CFG: u32 = 0xffff_ff7a;
    let ops = [
        Op71::Ldunif { dst: V(0), u: f(0.75) },
        Op71::Ldunif { dst: V(1), u: f(0.25) },
        Op71::Wrtmuc { u: Uniform::constant(P0) },
        Op71::Wrtmuc { u: Uniform::constant(P1) },
        mov(Dst71::Magic(TMUT), 1),
        mov(Dst71::Magic(TMUS), 0),
        Op71::Ldunif { dst: V(2), u: f(4.0) },
        Op71::Thrsw,
        Op71::Thrsw,
        Op71::Ldtmu { dst: V(3) },
        fadd(4, v(3), v(2)),
        mov(Dst71::Tlbu(Uniform::constant(CFG)), 4),
    ];
    let texels = vec![[10.0, 0.0, 0.0, 1.0], [20.0, 0.0, 0.0, 1.0], [30.0, 0.0, 0.0, 1.0], [40.0, 0.0, 0.0, 1.0]];
    let (instrs, unif, o) = run(&ops, &[0, 4, 5, 6, 7], Vec::new(), vec![Texture::flat(2, 2, texels)]);
    let want = [0.75f32.to_bits(), 0.25f32.to_bits(), P0, P1, 4.0f32.to_bits(), CFG];
    assert_eq!(unif, want, "{}", listing(&instrs));
    /* the stream, read word by word, gives each consumer its own uniform */
    let mut k = 0;
    let mut kinds = Vec::new();
    for i in &instrs {
        let a = alu(i);
        if a.sig.ldunif || a.sig.ldunifrf {
            kinds.push("ldunif");
        } else if a.sig.wrtmuc {
            kinds.push("wrtmuc");
        } else if writes_magic(a, TLBU) {
            kinds.push("tlbu");
        } else {
            continue;
        }
        k += 1;
    }
    assert_eq!(k, want.len());
    assert_eq!(kinds, ["ldunif", "ldunif", "wrtmuc", "wrtmuc", "ldunif", "tlbu"], "{}", listing(&instrs));
    assert_eq!(o.tmu_lookups, 1);
    assert_eq!(o.tlb.len(), 1);
    assert_eq!(o.tlb[0].config, Some(CFG));
    assert_eq!(tlb_f(&o, 0), 20.0 + 4.0); // s 0.75, t 0.25: texel (1, 0)
}

// ---------------------------------------------------------------------------
// (b) ldunif against ldunifrf.
// ---------------------------------------------------------------------------

/// A uniform given rf0 is the plain `ldunif` (no address named); any other
/// register makes it `ldunifrf` naming that register. The rf0 value
/// survives the `ldunifrf`s after it (which write only their own register,
/// `v3d_qpu_writes_rf0_implicitly`, qpu_instr.c:1030-1040).
#[test]
fn ldunif_into_rf0_is_plain_and_any_other_register_is_ldunifrf() {
    let ops = [
        Op71::Ldunif { dst: V(0), u: f(1.0) },
        Op71::Ldunif { dst: V(1), u: f(2.0) },
        Op71::Ldunif { dst: V(2), u: f(4.0) },
        fadd(3, v(0), v(1)),
        fadd(4, v(3), v(2)),
        Op71::Thrsw,
        Op71::Thrsw,
        mov(Dst71::Tlb, 4),
    ];
    let regs = [0, 7, 31, 5, 6];
    let (instrs, _, o) = run(&ops, &regs, Vec::new(), Vec::new());
    let text = listing(&instrs);
    assert_eq!(tlb_f(&o, 0), 7.0);
    let plain: Vec<usize> = (0..instrs.len()).filter(|&k| alu(&instrs[k]).sig.ldunif).collect();
    let rf: Vec<u8> = instrs.iter().map(alu).filter(|a| a.sig.ldunifrf).map(|a| a.sig_addr).collect();
    assert_eq!(plain.len(), 1, "{text}");
    assert_eq!(rf, [7, 31], "{text}");
    for &k in &plain {
        assert!(!alu(&instrs[k]).sig.ldunifrf, "{text}");
    }
}

/// An `ldunifrf` names its register in the condition field: a flag push
/// and a conditional move that could have gone into its word go elsewhere.
#[test]
fn ldunifrf_shares_no_word_with_a_condition_or_a_flag_push() {
    let ops = [
        Op71::Ldunif { dst: V(0), u: f(0.25) },
        Op71::Flags { unit: FlagOp71::Add(AddOp71::Fcmp), a: Src71::Payload(2), b: v(0), pf: Pf::PushN, uf: Uf::None },
        Op71::Ldunif { dst: V(1), u: f(1.0) },
        Op71::Ldunif { dst: V(2), u: f(2.0) },
        Op71::CondMov { dst: V(1), src: v(2), cond: Cond::IfA },
        Op71::Ldunif { dst: V(3), u: f(8.0) },
        fadd(4, v(1), v(3)),
        Op71::Thrsw,
        Op71::Thrsw,
        mov(Dst71::Tlb, 4),
    ];
    let (instrs, _, o) = run(&ops, &[4, 5, 6, 7, 8], Vec::new(), Vec::new());
    /* Z 0.5 is not below 0.25: every lane keeps 1.0 */
    assert_eq!(tlb_f(&o, 0), 9.0);
    let text = listing(&instrs);
    assert_eq!(instrs.iter().filter(|i| alu(i).sig.ldunifrf).count(), 4, "{text}");
    assert!(instrs.iter().any(|i| alu(i).flags.apf != Pf::None), "{text}");
}

// ---------------------------------------------------------------------------
// (c) ldvary's C term in rf0.
// ---------------------------------------------------------------------------

/// Two smooth varyings: each read of C lands two words after its `ldvary`;
/// a plain `ldunif` and an ALU result given rf0, both right after an
/// `ldvary` in program order, land at least two words after it.
#[test]
fn ldvary_c_is_read_two_words_later_and_rf0_is_left_alone_after_it() {
    let ops = [
        Op71::Ldvary { dst: V(0) },
        fmul(1, v(0), Src71::Payload(3)),
        fadd(2, v(1), Src71::LdvaryC),
        Op71::Ldunif { dst: V(3), u: f(0.5) }, // rf0, plain
        fadd(4, v(2), v(3)),
        Op71::Ldvary { dst: V(5) },
        fadd(6, v(4), imm_f(1.0)), // rf0, by name
        fmul(7, v(5), Src71::Payload(3)),
        fadd(8, v(7), v(6)),
        Op71::Thrsw,
        Op71::Thrsw,
        mov(Dst71::Tlb, 4),
        mov(Dst71::Tlb, 8),
    ];
    let regs = [4, 5, 6, 0, 7, 8, 0, 9, 10];
    let (instrs, _, o) = run(&ops, &regs, vec![vary(3.0, 0.25), vary(-1.5, 4.0)], Vec::new());
    let text = listing(&instrs);
    assert_eq!(tlb_f(&o, 0), 3.0 * W + 0.25 + 0.5);
    assert_eq!(tlb_f(&o, 1), -1.5 * W + (3.0 * W + 0.25 + 0.5 + 1.0));
    let ld0 = nth(&instrs, 0, |a| a.sig.ldvary);
    let ld1 = nth(&instrs, 1, |a| a.sig.ldvary);
    /* the C read: the FADD reading rf0 */
    let c = nth(&instrs, 0, |a| a.add.op == AddOp71::Fadd && reads_rf0(a));
    assert!(c >= ld0 + 2, "C read at {c}, ldvary at {ld0}\n{text}");
    /* the plain ldunif may share the C read's word: the read takes rf0
     * before the word's own write (sim71 gives C there) */
    let plain = nth(&instrs, 0, |a| a.sig.ldunif);
    assert!(plain >= ld0 + 2 && plain >= c, "{text}");
    let by_name = nth(&instrs, 0, |a| add_writes_rf(a) == Some(0) || mul_writes_rf(a) == Some(0));
    assert!(by_name >= ld1 + 2, "rf0 written at {by_name}, ldvary at {ld1}\n{text}");
}

/// The C term read by an SFU and by a mul operand, and an `ldvary` back to
/// back with the next: each C read still lands two words after its own
/// `ldvary`, never on the word after the next one's C.
#[test]
fn ldvary_c_reads_by_every_unit() {
    let ops = [
        Op71::Ldvary { dst: V(0) },
        Op71::Sfu { op: Sfu71::Recip, dst: V(1), a: Src71::LdvaryC },
        Op71::Ldvary { dst: V(2) },
        fmul(3, v(2), Src71::LdvaryC),
        fadd(4, v(1), v(3)),
        Op71::Thrsw,
        Op71::Thrsw,
        mov(Dst71::Tlb, 4),
        mov(Dst71::Tlb, 0),
    ];
    let (instrs, _, o) = run(&ops, &[4, 5, 6, 7, 8], vec![vary(1.0, 0.25), vary(3.0, 2.0)], Vec::new());
    let text = listing(&instrs);
    assert_eq!(tlb_f(&o, 0), 1.0 / 0.25 + 3.0 * 2.0);
    assert_eq!(tlb_f(&o, 1), 1.0);
    let ld0 = nth(&instrs, 0, |a| a.sig.ldvary);
    let ld1 = nth(&instrs, 1, |a| a.sig.ldvary);
    let r = nth(&instrs, 0, |a| a.add.op == AddOp71::Recip);
    let m = nth(&instrs, 0, |a| a.mul.op == MulOp71::Fmul);
    assert!(r >= ld0 + 2 && r < ld1 + 1, "recip at {r}, ldvarys at {ld0}, {ld1}\n{text}");
    assert!(m >= ld1 + 2, "fmul at {m}, ldvary at {ld1}\n{text}");
}

// ---------------------------------------------------------------------------
// (d) The TMU sequence and the TLB writes.
// ---------------------------------------------------------------------------

/// Two look-ups with a thread switch between: each `wrtmuc` before its
/// look-up's T and S writes, the second look-up's configuration after the S
/// that retired the first and after the first's `ldtmu`, each `ldtmu` after
/// its S.
#[test]
fn the_tmu_sequence_keeps_its_order() {
    const P1: u32 = 0x0000_0001;
    let ops = [
        Op71::Ldunif { dst: V(0), u: f(0.75) },
        Op71::Ldunif { dst: V(1), u: f(0.25) },
        Op71::Wrtmuc { u: Uniform::constant(0x0000_0001) }, // texture 0
        Op71::Wrtmuc { u: Uniform::constant(P1) },
        mov(Dst71::Magic(TMUT), 1),
        mov(Dst71::Magic(TMUS), 0),
        Op71::Thrsw,
        Op71::Ldtmu { dst: V(2) },
        Op71::Wrtmuc { u: Uniform::constant(0x0100_0001) }, // texture 1
        Op71::Wrtmuc { u: Uniform::constant(P1) },
        mov(Dst71::Magic(TMUT), 0),
        mov(Dst71::Magic(TMUS), 1),
        Op71::Thrsw,
        Op71::Thrsw,
        Op71::Ldtmu { dst: V(3) },
        mov(Dst71::Tlb, 2),
        mov(Dst71::Tlb, 3),
    ];
    let t0 = vec![[10.0, 0.0, 0.0, 1.0], [20.0, 0.0, 0.0, 1.0], [30.0, 0.0, 0.0, 1.0], [40.0, 0.0, 0.0, 1.0]];
    let t1 = vec![[1.0, 0.0, 0.0, 1.0], [2.0, 0.0, 0.0, 1.0], [3.0, 0.0, 0.0, 1.0], [4.0, 0.0, 0.0, 1.0]];
    let tex = vec![Texture::flat(2, 2, t0), Texture::flat(2, 2, t1)];
    let (instrs, _, o) = run(&ops, &[4, 5, 6, 7], Vec::new(), tex);
    let text = listing(&instrs);
    assert_eq!(o.tmu_lookups, 2);
    assert_eq!(o.ldtmus_without_switch, 0, "{text}");
    assert_eq!(tlb_f(&o, 0), 20.0); // s 0.75, t 0.25: texel (1, 0)
    assert_eq!(tlb_f(&o, 1), 3.0); // s 0.25, t 0.75: texel (0, 1)
    let wrtmuc = |n| nth(&instrs, n, |a| a.sig.wrtmuc);
    let t = |n| nth(&instrs, n, |a| writes_magic(a, TMUT));
    let s = |n| nth(&instrs, n, |a| writes_magic(a, TMUS));
    let ldtmu = |n| nth(&instrs, n, |a| a.sig.ldtmu);
    for n in 0..2 {
        assert!(wrtmuc(2 * n) < wrtmuc(2 * n + 1), "{text}");
        assert!(wrtmuc(2 * n + 1) <= t(n), "look-up {n}: wrtmuc after its T\n{text}");
        assert!(t(n) < s(n), "{text}");
        assert!(ldtmu(n) > s(n), "look-up {n}: ldtmu at {} not after its S at {}\n{text}", ldtmu(n), s(n));
    }
    assert!(wrtmuc(2) > s(0), "the second configuration before the first S\n{text}");
    assert!(wrtmuc(2) > ldtmu(0), "the second look-up beside the first ldtmu\n{text}");
    /* one TMU read per word */
    for i in &instrs {
        let a = alu(i);
        assert!(!(a.sig.ldtmu && a.sig.ldunif), "{text}");
    }
}

/// Two `wrtmuc`s never share a word (each consumes a uniform), and a
/// `wrtmuc` shares its word with a TMU write when they are free to.
#[test]
fn a_wrtmuc_may_share_its_word_with_a_tmu_write() {
    let ops = [
        Op71::Ldunif { dst: V(0), u: f(0.25) },
        Op71::Ldunif { dst: V(1), u: f(0.25) },
        Op71::Wrtmuc { u: Uniform::constant(0x0000_0001) },
        mov(Dst71::Magic(TMUT), 1),
        mov(Dst71::Magic(TMUS), 0),
        Op71::Thrsw,
        Op71::Thrsw,
        Op71::Ldtmu { dst: V(2) },
        mov(Dst71::Tlb, 2),
    ];
    let t0 = vec![[10.0, 0.0, 0.0, 1.0], [20.0, 0.0, 0.0, 1.0], [30.0, 0.0, 0.0, 1.0], [40.0, 0.0, 0.0, 1.0]];
    let (instrs, _, o) = run(&ops, &[4, 5, 6], Vec::new(), vec![Texture::flat(2, 2, t0)]);
    let text = listing(&instrs);
    /* P0 alone: 16-bit results, one word, the red/green pair */
    let l = o.tlb[0].value[0].expect("defined");
    assert_eq!(sim::half_to_f32(l as u16), 10.0, "{text}");
    let w = nth(&instrs, 0, |a| a.sig.wrtmuc);
    assert!(w <= nth(&instrs, 0, |a| writes_magic(a, TMUT)), "{text}");
}

/// Four TLB writes, alternately on the add and the mul unit (which could
/// otherwise share words), the first a TLBU with its configuration: one TLB
/// access per word, in program order, none in the program end's last slot.
#[test]
fn tlb_writes_one_per_word_in_order() {
    const CFG: u32 = 0xffff_ff7a;
    let ops = [
        Op71::Ldunif { dst: V(0), u: f(1.0) },
        Op71::Ldunif { dst: V(1), u: f(2.0) },
        Op71::Ldunif { dst: V(2), u: f(3.0) },
        Op71::Ldunif { dst: V(3), u: f(4.0) },
        Op71::Thrsw,
        Op71::Thrsw,
        mov(Dst71::Tlbu(Uniform::constant(CFG)), 0),
        add_mov(Dst71::Tlb, 1),
        mov(Dst71::Tlb, 2),
        add_mov(Dst71::Tlb, 3),
    ];
    let (instrs, unif, o) = run(&ops, &[4, 5, 6, 7], Vec::new(), Vec::new());
    let text = listing(&instrs);
    assert_eq!(unif, [1.0f32.to_bits(), 2.0f32.to_bits(), 3.0f32.to_bits(), 4.0f32.to_bits(), CFG]);
    assert_eq!(o.tlb.len(), 4, "{text}");
    assert_eq!(o.tlb[0].config, Some(CFG));
    for (n, want) in [1.0, 2.0, 3.0, 4.0].into_iter().enumerate() {
        assert_eq!(tlb_f(&o, n), want, "{text}");
    }
    let last = instrs.len() - 1;
    assert!(!magic_writes(alu(&instrs[last])).iter().any(|&w| w == TLB || w == TLBU), "{text}");
    /* the last thread switch comes before every TLB write */
    let thrsw = (0..instrs.len()).filter(|&k| alu(&instrs[k]).sig.thrsw).collect::<Vec<_>>();
    let first_tlb = nth(&instrs, 0, |a| magic_writes(a).iter().any(|&w| w == TLB || w == TLBU));
    assert!(thrsw[1] < first_tlb, "{text}");
}
