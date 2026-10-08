// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES2.functional.shaders.invariance.{lowp,mediump,highp}.
//! common_subexpression_{1,2,3}: two vertex shaders that both declare
//! `invariant gl_Position` and compute it with the same expression, one of
//! them also computing an unrelated varying from shared subexpressions. dEQP
//! draws the same triangles with both and fails on any pixel the first draw
//! covered and the second did not (es2fShaderInvarianceTests.cpp:257-366),
//! so the two programs must compute gl_Position bit for bit alike.
//! docs/investigations/es2-invariance.md has the cases, the cause and the
//! proposed fix.
//!
//! What is compared is the computation itself, as each back end hands it to
//! its encoder: gl_Position's four components traced back through the
//! program to the inputs, as a canonical expression (a move is transparent;
//! `a + b`, `a * b` and the factors of `a * b + c` are put in one order,
//! which IEEE 754 makes exact). Two programs with the same expressions
//! compute the same bits on any machine.
//!
//! - PowerVR: the program after allocation, with the epilogue's writes of
//!   output dwords 0..3 (gl_Position) as the roots. The USC interpreter of
//!   tests/usc_exec cannot judge this: it rounds `fmad` as a product and
//!   then a sum (`alu`, "fmad"), so a fused and an unfused form agree there.
//!   [`powervr::fused_model_differs`] evaluates the two trees with `fmad`
//!   rounded once instead (an assumption about the hardware, which the
//!   board's failures support) and counts the inputs whose bits differ.
//! - GC7000: selection's instructions (`select::select`), traced lane by
//!   lane through write masks and swizzles from the position register.
//! - V3D: the prepared scalar program (`v3d::prepare::prepare_vertex`), whose
//!   `position` names gl_Position's four scalars.
//!
//! A case the back end computes differently at the tip is `#[ignore]`d with
//! the cause, so the suite stays green; the fix (plan item es2-invariance)
//! removes the ignore. The shaders are dEQP's (VK-GL-CTS, Apache-2.0),
//! es2fShaderInvarianceTests.cpp:558-660, with each precision's literals
//! (:485-491) substituted as dEQP's formatGLSL does.

#![cfg(feature = "shaderc-online")]

/// One precision's literals, es2fShaderInvarianceTests.cpp:485-491.
struct Precision {
    name: &'static str,
    high: &'static str,
    high_inv: &'static str,
    medium: &'static str,
    low: &'static str,
    low_inv: &'static str,
}

const HIGHP: Precision =
    Precision { name: "highp", high: "1.0e20", high_inv: "1.0e-20", medium: "1.0e14", low: "1.0e9", low_inv: "1.0e-9" };
const MEDIUMP: Precision =
    Precision { name: "mediump", high: "1.0e4", high_inv: "1.0e-4", medium: "1.0e2", low: "1.0e1", low_inv: "1.0e-1" };
const LOWP: Precision =
    Precision { name: "lowp", high: "0.9", high_inv: "1.1", medium: "1.1", low: "1.15", low_inv: "0.87" };

/// common_subexpression_0 (:521-553), which passes on every board: the
/// control, the same shape of test with nothing the fusion decides
/// differently.
const CASE0: [&str; 2] = [
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   v_unrelated = a_input.xzxz + (${HIGH_VALUE}*a_input.x*a_input.xxxx + \
     ${HIGH_VALUE}*a_input.y*a_input.yyyy) * (1.08 * a_input.zyzy * a_input.xzxz) * ${HIGH_VALUE_INV} \
     * (a_input.z * a_input.zzxz - a_input.z * a_input.zzxz) + (${HIGH_VALUE}*a_input.x*a_input.xxxx + \
     ${HIGH_VALUE}*a_input.y*a_input.yyyy) / ${HIGH_VALUE};\n\
     \x20   gl_Position = a_input + (${HIGH_VALUE}*a_input.x*a_input.xxxx + \
     ${HIGH_VALUE}*a_input.y*a_input.yyyy) * ${HIGH_VALUE_INV};\n\
     }\n",
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   v_unrelated = vec4(0.0, 0.0, 0.0, 0.0);\n\
     \x20   gl_Position = a_input + (${HIGH_VALUE}*a_input.x*a_input.xxxx + \
     ${HIGH_VALUE}*a_input.y*a_input.yyyy) * ${HIGH_VALUE_INV};\n\
     }\n",
];

/// common_subexpression_1, the two vertex shaders (:558-591).
const CASE1: [&str; 2] = [
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   ${IN_PREC} vec4 a = ${HIGH_VALUE} * a_input.zzxx + a_input.xzxy - ${HIGH_VALUE} * a_input.zzxx;\n\
     \x20   ${IN_PREC} vec4 b = ${HIGH_VALUE} * a_input.zzxx;\n\
     \x20   ${IN_PREC} vec4 c = b - ${HIGH_VALUE} * a_input.zzxx + a_input.xzxy;\n\
     \x20   ${IN_PREC} vec4 d = (${LOW_VALUE} * a_input.yzxx) * (${LOW_VALUE} * a_input.yzzw) * \
     (1.1*${LOW_VALUE_INV} * a_input.yzxx) * (${LOW_VALUE_INV} * a_input.xzzy);\n\
     \x20   ${IN_PREC} vec4 e = ((${LOW_VALUE} * a_input.yzxx) * (1.1*${LOW_VALUE_INV} * a_input.yzxx)) * \
     ((${LOW_VALUE_INV} * a_input.xzzy) * (${LOW_VALUE} * a_input.yzzw));\n\
     \x20   v_unrelated = a + b + c + d + e;\n\
     \x20   gl_Position = a_input + fract(c) + e;\n\
     }\n",
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   ${IN_PREC} vec4 b = ${HIGH_VALUE} * a_input.zzxx;\n\
     \x20   ${IN_PREC} vec4 c = b - ${HIGH_VALUE} * a_input.zzxx + a_input.xzxy;\n\
     \x20   ${IN_PREC} vec4 e = ((${LOW_VALUE} * a_input.yzxx) * (1.1*${LOW_VALUE_INV} * a_input.yzxx)) * \
     ((${LOW_VALUE_INV} * a_input.xzzy) * (${LOW_VALUE} * a_input.yzzw));\n\
     \x20   v_unrelated = vec4(0.0, 0.0, 0.0, 0.0);\n\
     \x20   gl_Position = a_input + fract(c) + e;\n\
     }\n",
];

/// common_subexpression_2 (:593-624).
const CASE2: [&str; 2] = [
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   ${IN_PREC} vec4 a = ${MEDIUM_VALUE} * (a_input.xxxx + a_input.yyyy);\n\
     \x20   ${IN_PREC} vec4 b = (${MEDIUM_VALUE} * (a_input.xxxx + a_input.yyyy)) * \
     (${MEDIUM_VALUE} * (a_input.xxxx + a_input.yyyy)) / ${MEDIUM_VALUE} / ${MEDIUM_VALUE};\n\
     \x20   ${IN_PREC} vec4 c = a * a;\n\
     \x20   ${IN_PREC} vec4 d = c / ${MEDIUM_VALUE} / ${MEDIUM_VALUE};\n\
     \x20   v_unrelated = a + b + c + d;\n\
     \x20   gl_Position = a_input + d;\n\
     }\n",
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   ${IN_PREC} vec4 a = ${MEDIUM_VALUE} * (a_input.xxxx + a_input.yyyy);\n\
     \x20   ${IN_PREC} vec4 c = a * a;\n\
     \x20   ${IN_PREC} vec4 d = c / ${MEDIUM_VALUE} / ${MEDIUM_VALUE};\n\
     \x20   v_unrelated = vec4(0.0, 0.0, 0.0, 0.0);\n\
     \x20   gl_Position = a_input + d;\n\
     }\n",
];

/// common_subexpression_3 (:626-660).
const CASE3: [&str; 2] = [
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   ${IN_PREC} float x = a_input.x * 0.2;\n\
     \x20   ${IN_PREC} vec4 a = a_input.xxyx * 0.7;\n\
     \x20   ${IN_PREC} vec4 b = a_input.yxyz * 0.7;\n\
     \x20   ${IN_PREC} vec4 c = a_input.zxyx * 0.5;\n\
     \x20   ${IN_PREC} vec4 f = x*a + x*b + x*c;\n\
     \x20   v_unrelated = f;\n\
     \x20   ${IN_PREC} vec4 g = x * (a + b + c);\n\
     \x20   gl_Position = a_input + g;\n\
     }\n",
    "${VERSION}${IN} ${IN_PREC} vec4 a_input;\n\
     ${OUT} mediump vec4 v_unrelated;\n\
     invariant gl_Position;\n\
     void main ()\n\
     {\n\
     \x20   ${IN_PREC} float x = a_input.x * 0.2;\n\
     \x20   ${IN_PREC} vec4 a = a_input.xxyx * 0.7;\n\
     \x20   ${IN_PREC} vec4 b = a_input.yxyz * 0.7;\n\
     \x20   ${IN_PREC} vec4 c = a_input.zxyx * 0.5;\n\
     \x20   v_unrelated = vec4(0.0, 0.0, 0.0, 0.0);\n\
     \x20   ${IN_PREC} vec4 g = x * (a + b + c);\n\
     \x20   gl_Position = a_input + g;\n\
     }\n",
];

/// dEQP's formatGLSL for the GLES2 module (:500-517): no version line,
/// `attribute` and `varying`, and the precision's literals.
fn specialise(template: &str, p: &Precision) -> String {
    [
        ("${VERSION}", ""),
        ("${IN}", "attribute"),
        ("${OUT}", "varying"),
        ("${IN_PREC}", p.name),
        ("${HIGH_VALUE_INV}", p.high_inv),
        ("${HIGH_VALUE}", p.high),
        ("${MEDIUM_VALUE}", p.medium),
        ("${LOW_VALUE_INV}", p.low_inv),
        ("${LOW_VALUE}", p.low),
    ]
    .iter()
    .fold(template.to_string(), |s, (k, v)| s.replace(k, v))
}

/// The two programs of a case at a precision.
fn pair(case: &[&str; 2], p: &Precision) -> [String; 2] {
    [specialise(case[0], p), specialise(case[1], p)]
}

/// The compiler's scratch arena goes to one caller at a time
/// (`store::with_scratch` answers `None` to the rest); the tests of this
/// binary take turns.
fn scratch_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// `op(x, y)` with the operands of a commutative operation in one order.
fn commutative(op: &str, mut xs: Vec<String>) -> String {
    xs.sort();
    format!("{op}({})", xs.join(", "))
}

/// `a * b + c` with its two factors in one order.
fn fmad(xs: &[String]) -> String {
    let mut f = [xs[0].clone(), xs[1].clone()];
    f.sort();
    format!("fmad({}, {}, {})", f[0], f[1], xs[2])
}

/// The four components' expressions of both programs, and a report of the
/// components that differ, with both expressions.
/// Both programs' expressions are printed (`--nocapture` shows them).
fn compare(name: &str, trees: [[String; 4]; 2]) -> Result<(), String> {
    let mut bad = String::new();
    for c in 0..4 {
        for (k, t) in trees.iter().enumerate() {
            println!("{name}: program {} gl_Position.{} = {}", k + 1, b"xyzw"[c] as char, t[c]);
        }
        if trees[0][c] != trees[1][c] {
            bad += &format!(
                "\n  gl_Position.{}:\n    program 1: {}\n    program 2: {}",
                b"xyzw"[c] as char, trees[0][c], trees[1][c]
            );
        }
    }
    if bad.is_empty() {
        Ok(())
    } else {
        Err(format!("{name}: the two programs compute gl_Position differently{bad}"))
    }
}

/// The front end: the source to the IR module every back end selects from.
macro_rules! lower {
    ($a:expr, $src:expr) => {{
        let text = $a.alloc_str($src).unwrap();
        yakogl::shaderc::online::lower::compile_to_ir($a, yakogl::shaderc::online::sema::Stage::Vertex, text)
            .map_err(|e| e.msg)
            .expect("dEQP's shader compiles")
    }};
}

/// One test per case and precision, calling `$check(name, case, precision)`;
/// `$attr` is the test's attributes (an `#[ignore]` where the tip fails).
macro_rules! cases {
    ($check:ident; $($test:ident: $case:ident, $prec:ident $(, #[$attr:meta])?;)*) => {$(
        #[test]
        $(#[$attr])?
        fn $test() {
            let _s = super::scratch_lock();
            let name = stringify!($test);
            if let Err(e) = $check(name, &super::$case, &super::$prec) {
                panic!("{e}");
            }
        }
    )*};
}

#[cfg(feature = "gpu-powervr")]
mod powervr {
    use super::{commutative, compare, fmad, pair, Precision};
    use yakogl::shaderc::online::ir::AluOp;
    use yakogl::shaderc::online::powervr::abi::USC_MAX_TEMPS;
    use yakogl::shaderc::online::powervr::alloc::Limits;
    use yakogl::shaderc::online::powervr::chain::build_vertex;
    use yakogl::shaderc::online::powervr::prepare::CONST_REGS;
    use yakogl::shaderc::online::powervr::sel::{Bank, Mods, SelDst, SelGroup, SelOp, SelSrc};
    use yakogl::shaderc::online::store;

    const LIMITS: Limits = Limits { temps: USC_MAX_TEMPS, shared: 256, vtx_in: 64, coeff: 64 };

    /// A program after allocation, and the registers the epilogue writes to
    /// gl_Position's dwords (0..3; `vertex_output_writes` puts the position
    /// first).
    struct Prog {
        groups: Vec<SelGroup>,
        position: [(Bank, u16); 4],
    }

    fn build(src: &str) -> Prog {
        store::with_scratch(|a| {
            let l = lower!(a, src);
            let mut image = vec![0u8; 65536];
            let b = build_vertex(&l.module, LIMITS, a, &mut image).expect("builds");
            let mut position = [(Bank::Temp, u16::MAX); 4];
            for w in b.writes {
                if let Some(p) = position.get_mut(usize::from(w.addr)) {
                    *p = (w.bank, w.reg);
                }
            }
            assert!(position.iter().all(|p| p.1 != u16::MAX), "every gl_Position dword is written");
            Prog { groups: b.groups.to_vec(), position }
        })
        .expect("the scratch arena is free")
    }

    fn const_reg(reg: u16) -> Option<u32> {
        CONST_REGS.iter().find(|(_, i)| *i == reg).map(|(v, _)| *v)
    }

    /// The last group before `before` that writes `(bank, reg)`.
    fn def(p: &Prog, before: usize, bank: Bank, reg: u16) -> Option<usize> {
        p.groups[..before].iter().rposition(|g| g.dst == Some(SelDst::Fixed { bank, reg }))
    }

    fn with_mods(x: String, m: Mods) -> String {
        let x = if m.flr { format!("flr({x})") } else { x };
        let x = if m.abs { format!("abs({x})") } else { x };
        if m.neg {
            format!("neg({x})")
        } else {
            x
        }
    }

    fn src_expr(p: &Prog, before: usize, s: SelSrc) -> String {
        match s {
            SelSrc::Imm(v) => format!("{v:#010x}"),
            SelSrc::Fixed { bank, reg, mods } => with_mods(reg_expr(p, before, bank, reg), mods),
            SelSrc::Virtual { v, .. } => format!("virtual{}?", v.0),
        }
    }

    /// What `(bank, reg)` holds just before group `before`: a constant
    /// register as its value, so that it compares equal to an immediate.
    fn reg_expr(p: &Prog, before: usize, bank: Bank, reg: u16) -> String {
        let Some(j) = def(p, before, bank, reg) else {
            return match (bank, const_reg(reg)) {
                (Bank::Special, Some(v)) => format!("{v:#010x}"),
                (Bank::VtxIn, _) => format!("in{reg}"),
                _ => format!("{bank:?}{reg}"),
            };
        };
        let g = p.groups[j];
        let xs: Vec<String> = g.srcs.iter().flatten().map(|s| src_expr(p, j, *s)).collect();
        match g.op {
            SelOp::Copy => xs[0].clone(),
            SelOp::Alu(AluOp::Fadd) => commutative("fadd", xs),
            SelOp::Alu(AluOp::Fmul) => commutative("fmul", xs),
            SelOp::Alu(AluOp::Ffma) => fmad(&xs),
            op => format!("{op:?}({})", xs.join(", ")),
        }
    }

    fn trees(p: &Prog) -> [String; 4] {
        let end = p.groups.len();
        p.position.map(|(bank, reg)| reg_expr(p, end, bank, reg))
    }

    /// The value of `(bank, reg)` before group `before` for attribute
    /// `input`, `fmad` rounded once (`fused`) or as a product then a sum;
    /// `None` for an operation this model does not compute.
    fn eval(p: &Prog, before: usize, bank: Bank, reg: u16, input: &[f32; 4], fused: bool) -> Option<f32> {
        let Some(j) = def(p, before, bank, reg) else {
            return match bank {
                Bank::VtxIn => input.get(usize::from(reg)).copied(),
                Bank::Special => const_reg(reg).map(f32::from_bits),
                _ => None,
            };
        };
        let g = p.groups[j];
        let mut x = [0f32; 3];
        for (k, s) in g.srcs.iter().enumerate() {
            x[k] = match s {
                None => 0.0,
                Some(SelSrc::Imm(v)) => f32::from_bits(*v),
                Some(SelSrc::Fixed { bank, reg, mods }) => {
                    let mut f = eval(p, j, *bank, *reg, input, fused)?;
                    if mods.flr {
                        f = f.floor();
                    }
                    if mods.abs {
                        f = f.abs();
                    }
                    if mods.neg {
                        f = -f;
                    }
                    f
                }
                Some(SelSrc::Virtual { .. }) => return None,
            };
        }
        match g.op {
            SelOp::Copy => Some(x[0]),
            SelOp::Alu(AluOp::Fadd) => Some(x[0] + x[1]),
            SelOp::Alu(AluOp::Fmul) => Some(x[0] * x[1]),
            SelOp::Alu(AluOp::Ffma) if fused => Some(x[0].mul_add(x[1], x[2])),
            SelOp::Alu(AluOp::Ffma) => Some(x[0] * x[1] + x[2]),
            _ => None,
        }
    }

    /// dEQP's vertices (`genRandomVector`, :50-60): x, y, z in [-1, 1], w
    /// in [0.2, 1]. Not dEQP's generator, the same ranges.
    fn inputs(n: usize) -> Vec<[f32; 4]> {
        let mut s = 0x1234_5678u32;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 8) as f32 / (1u32 << 24) as f32
        };
        (0..n).map(|_| [next() * 2.0 - 1.0, next() * 2.0 - 1.0, next() * 2.0 - 1.0, 0.2 + next() * 0.8]).collect()
    }

    /// How many of `n` vertices get a different gl_Position from the two
    /// programs, with `fmad` rounded once and as two operations; `None`
    /// where the model cannot run a program.
    fn differing(progs: &[Prog; 2], n: usize) -> Option<(usize, usize)> {
        let mut out = (0, 0);
        for v in inputs(n) {
            for (fused, count) in [(true, &mut out.0), (false, &mut out.1)] {
                let mut differs = false;
                for c in 0..4 {
                    let at = |p: &Prog| {
                        let (bank, reg) = p.position[c];
                        eval(p, p.groups.len(), bank, reg, &v, fused).map(f32::to_bits)
                    };
                    differs |= at(&progs[0])? != at(&progs[1])?;
                }
                *count += usize::from(differs);
            }
        }
        Some(out)
    }

    fn check(name: &str, case: &[&str; 2], prec: &Precision) -> Result<(), String> {
        let [s0, s1] = pair(case, prec);
        let progs = [build(&s0), build(&s1)];
        compare(name, [trees(&progs[0]), trees(&progs[1])]).map_err(|e| match differing(&progs, 4096) {
            Some((fused, two)) => format!(
                "{e}\n  of 4096 vertices, {fused} get a different gl_Position with fmad rounded once, \
                 {two} with fmad rounded as a product then a sum (the usc_exec model)"
            ),
            None => e,
        })
    }

    cases! { check;
        lowp_common_subexpression_1: CASE1, LOWP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        mediump_common_subexpression_1: CASE1, MEDIUMP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        highp_common_subexpression_1: CASE1, HIGHP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        lowp_common_subexpression_2: CASE2, LOWP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        mediump_common_subexpression_2: CASE2, MEDIUMP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        highp_common_subexpression_2: CASE2, HIGHP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        lowp_common_subexpression_3: CASE3, LOWP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        mediump_common_subexpression_3: CASE3, MEDIUMP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        highp_common_subexpression_3: CASE3, HIGHP,
            #[ignore = "es2-invariance: fusion_pairs fuses a product into an fmad only when one sum reads it"];
        lowp_common_subexpression_0: CASE0, LOWP;
        mediump_common_subexpression_0: CASE0, MEDIUMP;
        highp_common_subexpression_0: CASE0, HIGHP;
    }
}

#[cfg(feature = "gpu-vivante")]
mod gc7000 {
    use super::{commutative, compare, fmad, pair, Precision};
    use yakogl::shaderc::online::select::{select, Selected};
    use yakogl::shaderc::online::store;
    use yakogl::shaderc::online::vivante::alloc::{SelDst, SelInstr, SelSrc};
    use yakogl::shaderc::online::vivante::build::{RGROUP_IMMED, RGROUP_UNIFORM_0, RGROUP_UNIFORM_1};
    use yakogl::shaderc::{SHC_VIV_CONST_LITERAL, SHC_VIV_CONST_UNIFORM};

    /// The operations that read one lane of each source (the x of its
    /// swizzle) and write it to every lane they write.
    const SCALAR: &[&str] = &["rcp", "rsq", "log", "exp", "sqrt", "sin", "cos"];

    /// Where a value lives before allocation.
    #[derive(Clone, Copy, PartialEq)]
    enum Reg {
        Virtual(u32),
        Fixed(u8),
    }

    fn writes(i: &SelInstr, r: Reg, lane: usize) -> bool {
        match (i.dst, r) {
            (Some(SelDst::Virtual { v, write_mask }), Reg::Virtual(x)) => v.0 == x && write_mask & (1 << lane) != 0,
            (Some(SelDst::Fixed { reg, write_mask }), Reg::Fixed(x)) => reg == x && write_mask & (1 << lane) != 0,
            _ => false,
        }
    }

    fn lane_of(op: &str, swizzle: [u8; 4], lane: usize) -> usize {
        usize::from(if SCALAR.contains(&op) { swizzle[0] } else { swizzle[lane] })
    }

    fn mods(x: String, neg: bool, abs: bool) -> String {
        let x = if abs { format!("abs({x})") } else { x };
        if neg {
            format!("neg({x})")
        } else {
            x
        }
    }

    fn src_expr(s: &Selected<'_>, before: usize, op: &str, src: SelSrc, lane: usize) -> String {
        match src {
            SelSrc::Virtual { v, swizzle, neg, abs } => {
                mods(reg_expr(s, before, Reg::Virtual(v.0), lane_of(op, swizzle, lane)), neg, abs)
            }
            SelSrc::Input { reg, swizzle, neg, abs } => {
                mods(format!("in{reg}.{}", lane_of(op, swizzle, lane)), neg, abs)
            }
            SelSrc::Physical(p) if p.group == RGROUP_UNIFORM_0 || p.group == RGROUP_UNIFORM_1 => {
                let dword = usize::from(p.reg) * 4 + lane_of(op, p.swizzle, lane);
                let c = s.consts[dword];
                let x = match c.contents {
                    SHC_VIV_CONST_LITERAL => format!("{:#010x}", c.data),
                    SHC_VIV_CONST_UNIFORM => format!("u{}", c.data),
                    _ => format!("{c:?}"),
                };
                mods(x, p.neg, p.abs)
            }
            SelSrc::Physical(p) if p.group == RGROUP_IMMED => format!("imm{:?}/{}", p.imm_payload_bits(), p.amode),
            SelSrc::Physical(p) => format!("{p:?}"),
        }
    }

    /// Lane `lane` of `r` just before instruction `before`.
    fn reg_expr(s: &Selected<'_>, before: usize, r: Reg, lane: usize) -> String {
        let Some(j) = s.instrs[..before].iter().rposition(|i| writes(i, r, lane)) else {
            return "undefined".to_string();
        };
        let i = s.instrs[j];
        let op = i.op.op().name;
        let xs: Vec<String> = i.srcs.iter().flatten().map(|x| src_expr(s, j, op, *x, lane)).collect();
        let x = match op {
            "mov" => xs[0].clone(),
            "add" | "mul" => commutative(op, xs),
            "mad" => fmad(&xs),
            _ => format!("{op}({})", xs.join(", ")),
        };
        if i.saturate {
            format!("sat({x})")
        } else {
            x
        }
    }

    fn trees(src: &str) -> [String; 4] {
        store::with_scratch(|a| {
            let l = lower!(a, src);
            let s = select(&l.module, a).expect("selects");
            let pos = s.vs_pos_out_reg.expect("a vertex shader writes the position");
            let end = s.instrs.len();
            [0, 1, 2, 3].map(|c| reg_expr(&s, end, Reg::Fixed(pos), c))
        })
        .expect("the scratch arena is free")
    }

    fn check(name: &str, case: &[&str; 2], prec: &Precision) -> Result<(), String> {
        let [s0, s1] = pair(case, prec);
        compare(name, [trees(&s0), trees(&s1)])
    }

    cases! { check;
        lowp_common_subexpression_0: CASE0, LOWP;
        mediump_common_subexpression_0: CASE0, MEDIUMP;
        highp_common_subexpression_0: CASE0, HIGHP;
        lowp_common_subexpression_1: CASE1, LOWP;
        mediump_common_subexpression_1: CASE1, MEDIUMP;
        highp_common_subexpression_1: CASE1, HIGHP;
        lowp_common_subexpression_2: CASE2, LOWP;
        mediump_common_subexpression_2: CASE2, MEDIUMP;
        highp_common_subexpression_2: CASE2, HIGHP;
        lowp_common_subexpression_3: CASE3, LOWP;
        mediump_common_subexpression_3: CASE3, MEDIUMP;
        highp_common_subexpression_3: CASE3, HIGHP;
    }
}

#[cfg(feature = "gpu-v3d")]
mod v3d {
    use super::{commutative, compare, fmad, pair, Precision};
    use yakogl::shaderc::online::ir::{AluOp, IoKind};
    use yakogl::shaderc::online::store;
    use yakogl::shaderc::online::v3d::prepare::{prepare_vertex, Prepared, SOp, S};

    fn expr(p: &Prepared<'_>, s: S) -> String {
        let e = |x: &S| expr(p, *x);
        match p.ops[s.0 as usize] {
            SOp::Const(v) => format!("{v:#010x}"),
            SOp::Attr { loc, comp } => format!("in{loc}.{comp}"),
            SOp::Uniform(d) => format!("u{d}"),
            SOp::Alu(AluOp::Fadd, x) => commutative("fadd", x[..2].iter().map(e).collect()),
            SOp::Alu(AluOp::Fmul, x) => commutative("fmul", x[..2].iter().map(e).collect()),
            SOp::Alu(AluOp::Ffma, x) => fmad(&x.iter().map(e).collect::<Vec<_>>()),
            SOp::Alu(op, x) => format!("{op:?}({})", x[..op.arity()].iter().map(e).collect::<Vec<_>>().join(", ")),
            SOp::Fsub(x) => format!("fsub({}, {})", e(&x[0]), e(&x[1])),
            other => format!("{other:?}"),
        }
    }

    /// gl_Position as the render pass's vertex shader computes it, with
    /// every component of every varying asked for, as the link asks for
    /// the ones the fragment shader reads (chain.rs, `wanted`).
    fn trees(src: &str) -> [String; 4] {
        store::with_scratch(|a| {
            let l = lower!(a, src);
            let wanted: Vec<(u8, u8)> = l
                .module
                .outputs
                .iter()
                .filter(|d| d.kind == IoKind::Varying)
                .flat_map(|d| (0..d.components).map(move |c| (d.location, c)))
                .collect();
            let p = prepare_vertex(&l.module, &wanted, a).expect("prepares");
            p.position.map(|s| expr(&p, s))
        })
        .expect("the scratch arena is free")
    }

    fn check(name: &str, case: &[&str; 2], prec: &Precision) -> Result<(), String> {
        let [s0, s1] = pair(case, prec);
        compare(name, [trees(&s0), trees(&s1)])
    }

    cases! { check;
        lowp_common_subexpression_0: CASE0, LOWP;
        mediump_common_subexpression_0: CASE0, MEDIUMP;
        highp_common_subexpression_0: CASE0, HIGHP;
        lowp_common_subexpression_1: CASE1, LOWP;
        mediump_common_subexpression_1: CASE1, MEDIUMP;
        highp_common_subexpression_1: CASE1, HIGHP;
        lowp_common_subexpression_2: CASE2, LOWP;
        mediump_common_subexpression_2: CASE2, MEDIUMP;
        highp_common_subexpression_2: CASE2, HIGHP;
        lowp_common_subexpression_3: CASE3, LOWP;
        mediump_common_subexpression_3: CASE3, MEDIUMP;
        highp_common_subexpression_3: CASE3, HIGHP;
    }
}
