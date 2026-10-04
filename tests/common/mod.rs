// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A lane-faithful interpreter of ENCODED GC7000 instructions, shared by the
//! host tests that run the back end's output: `lane_move_equivalence.rs`
//! (two allocations of one program must agree) and `backend_differential.rs`
//! (the program must agree with the IR it was compiled from).
//!
//! Exact about lanes -- swizzles, write masks, scalar splats, dot products,
//! select/cmp, branches -- and only approximate about some operations'
//! arithmetic; a texture fetch is a fixed function of sampler and
//! coordinates. An operation it does not model, relative addressing or an
//! f16 immediate is `Run::Unsupported`, which a caller counts as skipped.

#![allow(dead_code)]

use yakogl::shaderc::online::vivante::isa::Instr;

/// A tiny deterministic generator, so a failure reproduces.
pub struct Rng(pub u64);
impl Rng {
    pub fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 >> 16) as u32
    }
    pub fn float(&mut self) -> f32 {
        (self.next() % 4001) as f32 / 1000.0 - 2.0
    }
}

#[derive(Debug)]
pub enum Run {
    Done { regs: Vec<[u32; 4]>, killed: bool },
    Unsupported(String),
}

fn cond(c: u32, a: f32, b: f32) -> bool {
    match c {
        0 => true,
        1 => a > b,
        2 => a < b,
        3 => a >= b,
        4 => a <= b,
        5 => a == b,
        6 => a != b,
        10 => a == 0.0,
        11 => a != 0.0,
        12 => a >= 0.0,
        13 => a > 0.0,
        14 => a <= 0.0,
        15 => a < 0.0,
        _ => a != 0.0,
    }
}

fn icond<T: PartialOrd + Default + Copy>(c: u32, a: T, b: T) -> bool {
    let z = T::default();
    match c {
        0 => true,
        1 => a > b,
        2 => a < b,
        3 => a >= b,
        4 => a <= b,
        5 => a == b,
        6 => a != b,
        10 => a == z,
        11 => a != z,
        12 => a >= z,
        13 => a > z,
        14 => a <= z,
        15 => a < z,
        _ => a != z,
    }
}

/// Run `code` on `init` (temps) and `uniforms`; outputs are read by the caller.
pub fn run(code: &[Instr], init: &[[u32; 4]], uniforms: &[[u32; 4]]) -> Run {
    run_mem(code, init, uniforms, &[])
}

/// [`run`] with memory for `load` to read: each entry is a byte address and
/// the dwords stored there (a uniform block the driver copied out, whose
/// address the program finds in its constant file).
pub fn run_mem(code: &[Instr], init: &[[u32; 4]], uniforms: &[[u32; 4]], mem: &[(u32, &[u32])]) -> Run {
    let mut t: Vec<[u32; 4]> = init.to_vec();
    /* The address register, a0. `movar`/`movai` write it; a source whose
     * addressing mode is 1..4 adds a0.x..a0.w to its register number. */
    let mut a0 = [0i32; 4];
    let mut pc = 0usize;
    let mut steps = 0usize;
    while pc < code.len() {
        steps += 1;
        if steps > 200_000 {
            return Run::Unsupported("did not finish in 200000 steps".into());
        }
        let ins = &code[pc];
        let name = ins.op().name;
        /* The type is two fields (build.rs `ty`): 0 f32, 1 s32, 6 u32, ...
         * Reading a field called TYPE -- which no operation has -- read every
         * integer operation as a float one: a negative integer compared as
         * a NaN, and integer adds added float bit patterns. */
        let ty = ins.get("TYPE_BIT01").unwrap_or(0) | ins.get("TYPE_BIT2").unwrap_or(0) << 2;
        /* TYPE 6 is U32 (vivante::alloc::OpType): the comparisons, the
         * conversions and `rshift` read their integers unsigned. Every other
         * integer TYPE is read signed, which is all ES 1.00 produces. */
        let unsigned = ty == 6;
        let ic = |c: u32, a: u32, b: u32| if unsigned { icond(c, a, b) } else { icond(c, a as i32, b as i32) };
        let c = ins.get("COND").unwrap_or(0);
        let sat = ins.get("SAT").unwrap_or(0) != 0;
        let mut srcs: Vec<[u32; 4]> = Vec::new();
        for n in 0..3 {
            let Some(s) = ins.src(n) else { continue };
            if s.is_immediate() {
                let p = s.imm_payload_bits().unwrap_or(0);
                let bits = match (s.amode >> 1) & 3 {
                    0 => p << 12,
                    1 => (((p as i32) << 12) >> 12) as u32,
                    2 => p,
                    _ => return Run::Unsupported("an f16 immediate".into()),
                };
                srcs.push([bits; 4]);
                continue;
            }
            let reg = match s.amode {
                0 => s.reg as i64,
                1..=4 => s.reg as i64 + a0[s.amode as usize - 1] as i64,
                m => return Run::Unsupported(format!("addressing mode {m}")),
            };
            /* An index outside the file reads nothing a test can predict:
             * say so rather than invent the zero a missing entry gives. */
            if reg < 0 {
                return Run::Unsupported("a relative read below register 0".into());
            }
            let reg = reg as usize;
            let base = match s.group {
                0 => t.get(reg).copied().unwrap_or([0; 4]),
                /* The internal register gl_FrontFacing is read from
                 * (select.rs front_facing compares it with 0): a front face
                 * here, the same for every run of a comparison. */
                1 if reg == 0 => [1; 4],
                2 => uniforms.get(reg).copied().unwrap_or([0; 4]),
                3 => uniforms.get(reg + 256).copied().unwrap_or([0; 4]),
                g => return Run::Unsupported(format!("register group {g}")),
            };
            let mut v = [0u32; 4];
            for (l, slot) in v.iter_mut().enumerate() {
                let mut x = base[(s.swizzle[l] & 3) as usize];
                if ty == 0 {
                    if s.abs {
                        x &= 0x7fff_ffff;
                    }
                    if s.neg {
                        x ^= 0x8000_0000;
                    }
                } else {
                    if s.abs {
                        x = (x as i32).unsigned_abs();
                    }
                    if s.neg {
                        x = (x as i32).wrapping_neg() as u32;
                    }
                }
                *slot = x;
            }
            srcs.push(v);
        }
        let f = |x: u32| f32::from_bits(x);
        let b = |x: f32| x.to_bits();
        let s0 = srcs.first().copied().unwrap_or([0; 4]);
        let s1 = srcs.get(1).copied().unwrap_or([0; 4]);
        let s2 = srcs.get(2).copied().unwrap_or([0; 4]);
        let target = ins.get("TARGET").unwrap_or(0) as usize;
        let result: Option<[u32; 4]> = match name {
            "nop" => None,
            "texkill" => return Run::Done { regs: t, killed: true },
            "branch" | "branch_unary" | "branch_binary" => {
                let take = if c == 0 {
                    true
                } else if ty == 0 {
                    cond(c, f(s0[0]), f(s1[0]))
                } else {
                    ic(c, s0[0], s1[0])
                };
                if take {
                    pc = target;
                    continue;
                }
                None
            }
            "mov" => Some(s0),
            /* Into the address register, never a temporary: its DST field's
             * register half means nothing. The INTEGER bits of the source,
             * whatever TYPE says -- the reference compiler's 7 of 7 address
             * writes are `movar` at TYPE 0 fed by `f2i` or an integer `add`
             * (shipped programs), and selection emits the same shape. That is
             * a reading of the corpus, not a measured instruction semantic. */
            "movar" | "movai" => {
                let mask = ins.dst().map_or(0x1, |d| d.write_mask);
                for (l, a) in a0.iter_mut().enumerate() {
                    if mask & (1 << l) != 0 {
                        *a = s0[l] as i32;
                    }
                }
                pc += 1;
                continue;
            }
            "add" if ty == 0 => Some(core::array::from_fn(|l| b(f(s0[l]) + f(s1[l])))),
            "add" => Some(core::array::from_fn(|l| s0[l].wrapping_add(s1[l]))),
            "mul" => Some(core::array::from_fn(|l| b(f(s0[l]) * f(s1[l])))),
            "mad" => Some(core::array::from_fn(|l| b(f(s0[l]) * f(s1[l]) + f(s2[l])))),
            "select" if ty == 0 => {
                Some(core::array::from_fn(|l| if cond(c, f(s0[l]), f(s1[l])) { s1[l] } else { s2[l] }))
            }
            "select" => Some(core::array::from_fn(|l| if ic(c, s0[l], s1[l]) { s1[l] } else { s2[l] })),
            "cmp" if ty == 0 => Some(core::array::from_fn(|l| if cond(c, f(s0[l]), f(s1[l])) { s2[l] } else { 0 })),
            "cmp" => Some(core::array::from_fn(|l| if ic(c, s0[l], s1[l]) { s2[l] } else { 0 })),
            "sign" => Some(core::array::from_fn(|l| {
                b(if f(s0[l]) > 0.0 {
                    1.0
                } else if f(s0[l]) < 0.0 {
                    -1.0
                } else {
                    0.0
                })
            })),
            "floor" => Some(core::array::from_fn(|l| b(f(s0[l]).floor()))),
            "ceil" => Some(core::array::from_fn(|l| b(f(s0[l]).ceil()))),
            "frc" => Some(core::array::from_fn(|l| b(f(s0[l]) - f(s0[l]).floor()))),
            "and" => Some(core::array::from_fn(|l| s0[l] & s1[l])),
            "or" => Some(core::array::from_fn(|l| s0[l] | s1[l])),
            "xor" => Some(core::array::from_fn(|l| s0[l] ^ s1[l])),
            "not" => Some(core::array::from_fn(|l| !s0[l])),
            "f2i" if unsigned => Some(core::array::from_fn(|l| f(s0[l]) as u32)),
            "f2i" => Some(core::array::from_fn(|l| f(s0[l]) as i32 as u32)),
            "i2f" if unsigned => Some(core::array::from_fn(|l| b(s0[l] as f32))),
            "i2f" => Some(core::array::from_fn(|l| b(s0[l] as i32 as f32))),
            /* A shift of 32 or more is undefined in GLSL (the IR evaluator
             * stops there as unmeasured), so the count's top bits never
             * decide an answer this is compared on. */
            "lshift" => Some(core::array::from_fn(|l| s0[l].wrapping_shl(s1[l]))),
            "rshift" if unsigned => Some(core::array::from_fn(|l| s0[l].wrapping_shr(s1[l]))),
            "rshift" => Some(core::array::from_fn(|l| (s0[l] as i32).wrapping_shr(s1[l]) as u32)),
            "dp2" | "dp3" | "dp4" => {
                let n = (name.as_bytes()[2] - b'0') as usize;
                let d: f32 = (0..n).map(|l| f(s0[l]) * f(s1[l])).sum();
                Some([b(d); 4])
            }
            /* Scalar: one component, the machine splats the swizzle; the
             * result goes to every written lane. */
            "rcp" => Some([b(1.0 / f(s0[0])); 4]),
            "rsq" => Some([b(1.0 / f(s0[0]).sqrt()); 4]),
            "sqrt" => Some([b(f(s0[0]).sqrt()); 4]),
            "exp" => Some([b(f(s0[0]).exp2()); 4]),
            /* The new transcendental unit of this core (Mesa's etna_lower_alu,
             * the reference's `sin.rtz t.zw` + `mul t.z, t.w`): sin and cos
             * take their argument in units of pi, and sin, cos and log write a
             * PAIR of lanes whose product is the result. How the hardware
             * splits the value across the pair is not known; any split with
             * the right product serves, so the value and 1.0. A program that
             * reads one lane of the pair, or passes radians, gets the wrong
             * number here as it does on silicon. */
            "log" => Some([b(f(s0[0]).log2()), b(1.0), b(f(s0[0]).log2()), b(1.0)]),
            "sin" => {
                let v = (f(s0[0]) * core::f32::consts::PI).sin();
                Some([b(v), b(1.0), b(v), b(1.0)])
            }
            "cos" => {
                let v = (f(s0[0]) * core::f32::consts::PI).cos();
                Some([b(v), b(1.0), b(v), b(1.0)])
            }
            "div" => Some([b(f(s0[0]) / f(s1[0])); 4]),
            "imullo0" => Some([s0[0].wrapping_mul(s1[0]); 4]),
            /* `load dst, offset, base`: consecutive dwords from byte address
             * base.x + offset.x (Mesa's etnaviv lowers a load_ubo to exactly
             * this, `load.u32 t1, t1.xxxx, u0.zxxx`), PACKED into the lanes
             * the destination enables: the first enabled lane takes dword 0,
             * the next dword 1. Measured on the i.MX8MP (gl_test 128 at
             * cc955595): a load written to `.__zw` put fcols[2].xy in z and w.
             * An address no memory entry covers is not a zero the test could
             * predict. */
            "load" => {
                let addr = s1[0].wrapping_add(s0[0]);
                let Some(words) = mem.iter().find_map(|(base, words)| {
                    let rel = addr.checked_sub(*base)?;
                    let i = (rel / 4) as usize;
                    (rel % 4 == 0 && i + 4 <= words.len()).then(|| [words[i], words[i + 1], words[i + 2], words[i + 3]])
                }) else {
                    return Run::Unsupported(format!("a load from 0x{addr:x}, outside every memory entry"));
                };
                let mask = ins.dst().map_or(0xf, |d| d.write_mask);
                let mut packed = [0u32; 4];
                let mut next = 0usize;
                for (l, v) in packed.iter_mut().enumerate() {
                    if mask & (1 << l) != 0 {
                        *v = words[next];
                        next += 1;
                    }
                }
                Some(packed)
            }
            "iabs" => Some(core::array::from_fn(|l| (s0[l] as i32).wrapping_abs() as u32)),
            /* A texture fetch as a fixed function of the sampler and the two
             * coordinates a 2D fetch reads, then TEX_SWIZ: lane-faithful,
             * which is all an equivalence of two allocations needs. */
            /* texldl / texldb: the same function, and the level or bias
             * (SRC1's first lane) too, so a program that moves it differs.
             * Only the lanes every look-up reads are hashed: a 2D
             * coordinate's z and w are whatever the register held. */
            /* txf (texelFetch): the same function of its integer
             * coordinate and its level (SRC1, a float). */
            "texld" | "texldl" | "texldb" | "txf" => {
                let id = ins.get("TEX_ID").unwrap_or(0);
                let sw = ins.get("TEX_SWIZ").unwrap_or(0xe4);
                let mut h = 0xcbf2_9ce4_8422_2325u64 ^ id as u64 ^ ((name == "txf") as u64) << 40;
                let lod = (name != "texld").then_some(s1[0]);
                for x in [s0[0], s0[1]].into_iter().chain(lod) {
                    h = (h ^ x as u64).wrapping_mul(0x100_0000_01b3);
                }
                let texel: [u32; 4] = core::array::from_fn(|l| ((h >> (l * 16)) & 0xffff) as f32 / 65535.0).map(b);
                Some(core::array::from_fn(|l| texel[((sw >> (2 * l)) & 3) as usize]))
            }
            /* A derivative needs the neighbouring pixels, which one
             * invocation does not have: a fixed function of each lane of its
             * source instead, different for dsx and dsy, which is all an
             * equivalence of two allocations needs (the IR evaluator has no
             * derivatives, so backend_differential skips these programs). */
            "dsx" => Some(core::array::from_fn(|l| b(f(s0[l]) * 0.25))),
            "dsy" => Some(core::array::from_fn(|l| b(f(s0[l]) * -0.5))),
            other => return Run::Unsupported(format!("operation `{other}`")),
        };
        if let (Some(r), Some(d)) = (result, ins.dst()) {
            if ins.get("DST_USE") != Some(0) {
                let slot = &mut t[d.reg as usize];
                for l in 0..4 {
                    if d.write_mask & (1 << l) != 0 {
                        let mut x = r[l];
                        if sat && ty == 0 {
                            x = b(f(x).clamp(0.0, 1.0));
                        }
                        slot[l] = x;
                    }
                }
            }
        }
        pc += 1;
    }
    Run::Done { regs: t, killed: false }
}
