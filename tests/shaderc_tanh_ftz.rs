// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GLSL ES 3.00 `tanh` stays inside dEQP's highp bound on a GPU that flushes
//! subnormals to zero: the 8 dEQP-GLES3
//! `shaders.builtin_functions.precision.tanh.highp_*` cases that left it on
//! the AM62P (docs/es3/am62p-gles3-notpass.tsv).
//!
//! The lowering (src/shaderc/online/builtin_call.rs, `lower_es3`) used to
//! compute (e^x - e^-x) * rcp(e^x + e^-x). For |x| > 87.34 the sum passes
//! 2^126, rcp(sum) is subnormal, the PowerVR USC (and the GC7000) flush it
//! to 0, and tanh(88) came out 0. It now keeps the sinh and cosh halves and
//! divides once, sinh * rcp(cosh), as dEQP 93bca01's builtin_precision
//! derives tanh (`sinh(x) / cosh(x)`, each half `(exp(x) -+ exp(-x)) / 2`).
//! cosh stays at or below 2^126 for |x| <= 88.03, so rcp(cosh) is normal
//! wherever dEQP's division bound is finite.
//!
//! The values come from the front end's IR (the IR every back end
//! compiles), run by a small evaluator below that flushes every float
//! operand and result to zero, as those GPUs do. eval.rs's evaluator keeps
//! subnormals, so it does not show the defect. The bound is dEQP's
//! interval for the derived function, rebuilt here in f64: exp highp
//! `3 + 2|x|` ulp, each sum or difference rounded outwards one ulp, each
//! division 2.5 ulp (a denominator inside [2^-126, 2^126]), and a value
//! below 2^-126 allowed to flush (highp's subnormals are "maybe").
//! HOST-ONLY: the GPUs' own arithmetic is for the board.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test shaderc_tanh_ftz
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test shaderc_tanh_ftz
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online --test shaderc_tanh_ftz
//! ```
//! GLSL ES 3.00 is refused by a `report-es2` build (pp.rs out_of_scope), so
//! the file is empty there.
#![cfg(all(
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::ir::{AluOp, IoKind, IoSlot, Module, Node, Op, Range, Src};
use yakogl::shaderc::online::lower;
use yakogl::shaderc::online::sema::Stage;

/// The inputs the item names: small, mid, and the top of the range where
/// dEQP's bound is finite.
const XS: [f64; 7] = [1e-3, 0.5, 9.0, 10.0, 20.0, 44.0, 88.0];

/// `generateFragmentShader` of dEQP's shader executor, integer output.
fn fragment_executor(ty: &str, uty: &str) -> String {
    format!(
        "#version 300 es\nflat in highp {ty} vtx_out_in0;\n\
         layout(location = 0) out highp {uty} o_out0;\n\
         void main (void)\n{{\n\thighp {ty} in0 = vtx_out_in0;\n\thighp {ty} out0;\n\
         \tout0 = tanh(in0);\n\x20   o_out0 = floatBitsToUint(out0);\n}}\n"
    )
}

/// `generateVertexShader` of dEQP's shader executor.
fn vertex_executor(ty: &str) -> String {
    format!(
        "#version 300 es\nin highp vec4 a_position;\nin highp {ty} a_in0;\n\
         flat out highp {ty} vtx_out_out0;\n\
         void main (void)\n{{\n    gl_Position = a_position;\n    gl_PointSize = 1.0;\n\
         \thighp {ty} in0 = a_in0;\n\thighp {ty} out0;\n\tout0 = tanh(in0);\n\
         \tvtx_out_out0 = out0;\n}}\n"
    )
}

/// A float as a flushing GPU holds it: a subnormal is a zero of its sign.
fn ftz(v: f32) -> f32 {
    if v.is_subnormal() {
        0.0f32.copysign(v)
    } else {
        v
    }
}

/// A straight-line evaluator of `m` with every float operand and result
/// flushed. It knows the operations a `tanh` executor lowers to and stops
/// by name at anything else, so a value is never invented.
struct Ftz<'m, 'a> {
    m: &'m Module<'a>,
    values: Vec<[u32; 4]>,
    locals: Vec<[u32; 4]>,
    inputs: Vec<(IoSlot, [u32; 4])>,
    outputs: Vec<(IoSlot, [u32; 4])>,
}

impl<'m, 'a> Ftz<'m, 'a> {
    fn read(&self, s: Src) -> [u32; 4] {
        let v = self.values[s.val.0 as usize];
        core::array::from_fn(|k| v[usize::from(s.swizzle[k])])
    }

    fn alu(&self, op: AluOp, srcs: [Src; 3]) -> [u32; 4] {
        let a = self.read(srcs[0]);
        let b = if op.arity() >= 2 { self.read(srcs[1]) } else { [0; 4] };
        let c = if op.arity() >= 3 { self.read(srcs[2]) } else { [0; 4] };
        let f = |x: u32| ftz(f32::from_bits(x));
        let fl = |g: &dyn Fn(f32, f32, f32) -> f32| -> [u32; 4] {
            core::array::from_fn(|k| ftz(g(f(a[k]), f(b[k]), f(c[k]))).to_bits())
        };
        match op {
            AluOp::Fadd => fl(&|x, y, _| x + y),
            AluOp::Fmul => fl(&|x, y, _| x * y),
            AluOp::Ffma => fl(&|x, y, z| x.mul_add(y, z)),
            AluOp::Fneg => fl(&|x, _, _| -x),
            AluOp::Fabs => fl(&|x, _, _| x.abs()),
            AluOp::Frcp => fl(&|x, _, _| 1.0 / x),
            AluOp::Fexp2 => fl(&|x, _, _| x.exp2()),
            o => panic!("the FTZ evaluator has no {o:?}"),
        }
    }

    fn range(&mut self, r: Range) {
        for n in &self.m.nodes[r.first as usize..(r.first + r.len) as usize] {
            let id = match *n {
                Node::Instr(id) => id,
                /* main's closing return */
                Node::Return => return,
                _ => panic!("not straight-line: {n:?}"),
            };
            let i = id.0 as usize;
            let v = match self.m.instrs[i] {
                Op::ConstF32(l) => l,
                Op::ConstI32(l) => l.map(|x| x as u32),
                Op::Alu(op, srcs) => self.alu(op, srcs),
                Op::LoadInput { slot, .. } => {
                    self.inputs.iter().find(|(s, _)| *s == slot).unwrap_or_else(|| panic!("no input {slot:?}")).1
                }
                Op::StoreOutput { slot, src } => {
                    let v = self.read(src);
                    self.outputs.push((slot, v));
                    v
                }
                Op::LoadLocal { local, index: None, .. } => self.locals[local.0 as usize],
                Op::StoreLocal { local, index: None, src, write_mask } => {
                    let v = self.read(src);
                    let l = &mut self.locals[local.0 as usize];
                    for (k, lane) in l.iter_mut().enumerate() {
                        if write_mask & (1 << k) != 0 {
                            *lane = v[k];
                        }
                    }
                    v
                }
                ref o => panic!("the FTZ evaluator has no {o:?}"),
            };
            self.values[i] = v;
        }
    }
}

fn slot_of(m: &Module<'_>, name: &str, out: bool) -> IoSlot {
    let decls = if out { m.outputs } else { m.inputs };
    let d = decls.iter().find(|d| d.name == name).unwrap_or_else(|| panic!("no {name} in {decls:?}"));
    match d.kind {
        IoKind::Attribute => IoSlot::Attrib(d.location),
        IoKind::Varying => IoSlot::Varying(d.location),
        IoKind::Colour => IoSlot::FragColor(d.location),
        k => panic!("{name} is a {k:?}"),
    }
}

/// Compile `src` to IR and hand it to `f`.
fn with_ir<T>(stage: Stage, src: &str, f: impl FnOnce(&Module<'_>) -> T) -> T {
    let mut buf = vec![0u8; 1 << 21];
    let a = Arena::new(&mut buf);
    let text = a.alloc_str(src).unwrap();
    let l = lower::compile_to_ir(&a, stage, text).unwrap_or_else(|e| panic!("front end: {}\n{src}", e.msg));
    f(&l.module)
}

/// tanh of the four lanes of `x` through the executor of `stage`, flushed.
fn tanh_ftz(stage: Stage, x: [f32; 4]) -> [f32; 4] {
    let src = match stage {
        Stage::Vertex => vertex_executor("vec4"),
        _ => fragment_executor("vec4", "uvec4"),
    };
    with_ir(stage, &src, |m| {
        let (inp, outp) = match stage {
            Stage::Vertex => ("a_in0", "vtx_out_out0"),
            _ => ("vtx_out_in0", "o_out0"),
        };
        let mut inputs = vec![(slot_of(m, inp, false), x.map(f32::to_bits))];
        if stage == Stage::Vertex {
            inputs.push((slot_of(m, "a_position", false), [0.0f32, 0.0, 0.0, 1.0].map(f32::to_bits)));
        }
        let mut ev = Ftz {
            m,
            values: vec![[0x7fc0_dead; 4]; m.instrs.len()],
            locals: vec![[0x7fc0_dead; 4]; m.locals.len()],
            inputs,
            outputs: Vec::new(),
        };
        ev.range(m.body);
        let want = slot_of(m, outp, true);
        let (_, v) = *ev.outputs.iter().rev().find(|(s, _)| *s == want).expect("out0 stored");
        v.map(f32::from_bits)
    })
}

/// An f32 ulp at `v` (dEQP FloatFormat::ulp: the exponent clamped at the
/// smallest normal one).
fn ulp(v: f64) -> f64 {
    let e = if v == 0.0 { -126 } else { (v.abs().log2().floor() as i32).max(-126) };
    2f64.powi(e - 23)
}

#[derive(Clone, Copy, Debug)]
struct Iv {
    lo: f64,
    hi: f64,
}

impl Iv {
    /// Outwards by `n` ulp at each end, and down to zero where a value may
    /// flush.
    fn widen(self, n: f64) -> Iv {
        let tiny = 2f64.powi(-126);
        let mut lo = self.lo - n * ulp(self.lo);
        let mut hi = self.hi + n * ulp(self.hi);
        if lo.abs() < tiny || (lo < 0.0 && hi > 0.0) {
            lo = lo.min(0.0);
        }
        if hi.abs() < tiny {
            hi = hi.max(0.0);
        }
        Iv { lo, hi }
    }
    fn exp(x: f64) -> Iv {
        let r = x.exp();
        Iv { lo: r, hi: r }.widen(3.0 + 2.0 * x.abs())
    }
    fn add(self, o: Iv) -> Iv {
        Iv { lo: self.lo + o.lo, hi: self.hi + o.hi }.widen(1.0)
    }
    fn sub(self, o: Iv) -> Iv {
        Iv { lo: self.lo - o.hi, hi: self.hi - o.lo }.widen(1.0)
    }
    /// dEQP's Div: 2.5 ulp while the denominator is in [2^-126, 2^126].
    fn div(self, d: Iv) -> Iv {
        let (lim_lo, lim_hi) = (2f64.powi(-126), 2f64.powi(126));
        assert!(d.lo > 0.0 && d.lo >= lim_lo && d.hi <= lim_hi, "denominator {d:?} outside dEQP's finite bound");
        let q = [self.lo / d.lo, self.lo / d.hi, self.hi / d.lo, self.hi / d.hi];
        let lo = q.iter().copied().fold(f64::INFINITY, f64::min);
        let hi = q.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        Iv { lo, hi }.widen(2.5)
    }
}

/// dEQP's highp interval for tanh(x) = sinh(x) / cosh(x).
fn deqp_tanh(x: f64) -> Iv {
    let two = Iv { lo: 2.0, hi: 2.0 };
    let (ep, en) = (Iv::exp(x), Iv::exp(-x));
    let sinh = ep.sub(en).div(two);
    let cosh = ep.add(en).div(two);
    sinh.div(cosh)
}

fn check(stage: Stage, sign: f64) {
    for chunk in XS.chunks(4) {
        let mut x = [0f32; 4];
        for (k, v) in chunk.iter().enumerate() {
            x[k] = (sign * v) as f32;
        }
        let got = tanh_ftz(stage, x);
        for (k, _) in chunk.iter().enumerate() {
            let (xi, g) = (x[k], got[k]);
            assert!(!g.is_nan(), "{stage:?} tanh({xi}) is NaN");
            assert!(g != 0.0, "{stage:?} tanh({xi}) is 0 under a flush to zero");
            let iv = deqp_tanh(f64::from(xi));
            assert!(
                iv.lo <= f64::from(g) && f64::from(g) <= iv.hi,
                "{stage:?} tanh({xi}) = {g}, outside dEQP's highp [{}, {}]",
                iv.lo,
                iv.hi
            );
        }
    }
}

#[test]
fn fragment_positive_within_deqp_bound() {
    check(Stage::Fragment, 1.0);
}

#[test]
fn fragment_negative_within_deqp_bound() {
    check(Stage::Fragment, -1.0);
}

#[test]
fn vertex_within_deqp_bound() {
    check(Stage::Vertex, 1.0);
    check(Stage::Vertex, -1.0);
}

/// The bound rejects what the old lowering gave: tanh(88) = 0, and the
/// old sum's reciprocal is subnormal there.
#[test]
fn bound_rejects_the_flushed_quotient() {
    let iv = deqp_tanh(88.0);
    assert!(iv.lo > 0.99, "{iv:?}");
    let sum = (88f32 * core::f32::consts::LOG2_E).exp2();
    assert!((1.0 / sum).is_subnormal(), "rcp(e^88) = {}", 1.0 / sum);
}

/// The one reciprocal tanh lowers to never reads a sum directly: its
/// operand is a product (the halved cosh), so no later edit puts the
/// `rcp(e^x + e^-x)` back.
#[test]
fn rcp_reads_the_halved_cosh() {
    with_ir(Stage::Fragment, &fragment_executor("float", "uint"), |m| {
        let rcps: Vec<Src> = m
            .instrs
            .iter()
            .filter_map(|op| match *op {
                Op::Alu(AluOp::Frcp, s) => Some(s[0]),
                _ => None,
            })
            .collect();
        assert_eq!(rcps.len(), 1, "one reciprocal: {:?}", m.instrs);
        let feeder = m.instrs[rcps[0].val.0 as usize];
        assert!(matches!(feeder, Op::Alu(AluOp::Fmul, _)), "rcp reads {feeder:?}");
    });
}
