// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A host interpreter for the subset of the USC our builder emits, shared by
//! the USC integration tests. It executes decoded bytes, so it is the
//! instrument that compares our program with selection's meaning and with
//! the reference compiler's program for the same source.

#![allow(dead_code)]

pub mod ir;

use std::collections::{BTreeMap, BTreeSet};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::isa::ops_for;
use yakogl::shaderc::online::powervr::isa_tables::ENUMS;
use yakogl::shaderc::online::powervr::operands::{dsts, srcs, Operand};
use yakogl::shaderc::online::powervr::prepare::CONST_REGS;
use yakogl::shaderc::online::powervr::verify::skeleton;
use yakogl::shaderc::table::SHC_BLOB_TABLE;

/// The inputs one execution reads: the uniform file and the attributes.
#[derive(Clone)]
pub struct Inputs {
    pub shared: Vec<u32>,
    pub vtxin: Vec<u32>,
    /// A fragment shader's iterated values, by coefficient: component `c` of
    /// a varying starting at coefficient `o` evaluates to `coeff[o + 4c]`.
    pub coeff: Vec<u32>,
    /// What `savmsk.vm` returns (the sample coverage mask) and what the
    /// special register `samp_num` holds: one sample, covered, unless a
    /// caller sets otherwise.
    pub savmsk: u32,
    pub samp_num: u32,
    /// The special registers `gl_FragCoord.xy` is read from: `x_p`, `x_s`,
    /// `y_p`, `y_s` (97, 98, 100, 101), as f32 bits.
    pub frag_xy: [u32; 4],
    /// The special register `face_ccw` (44): nonzero when the primitive is
    /// anticlockwise.
    pub face_ccw: u32,
    /// The draw's samples per pixel, which `pck.cov` of 1.0 covers.
    pub samples: u32,
    /// The colour already in the pixel (RGBA8, R in the low byte), which a
    /// blend variant reads from the pixel output register.
    pub pixout: u32,
    /// An RGBA16F pixel's second dword (B and A), in pixel output 1.
    pub pixout1: u32,
}

impl Inputs {
    /// Deterministic, finite, and varied in sign and magnitude, so that a
    /// sign moved to the wrong factor or a swapped operand shows.
    pub fn new(seed: u32) -> Inputs {
        let mut x = seed.wrapping_mul(2_654_435_761).wrapping_add(1);
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            (((x % 20_001) as f32 - 10_000.0) / 1_250.0).to_bits()
        };
        Inputs {
            shared: (0..512).map(|_| next()).collect(),
            vtxin: (0..128).map(|_| next()).collect(),
            coeff: (0..256).map(|_| next()).collect(),
            savmsk: 1,
            samp_num: 0,
            frag_xy: [0; 4],
            face_ccw: 0,
            samples: 4,
            pixout: 0,
            pixout1: 0,
        }
    }

    /// Values from {0, 1, -1, 2, -0.5}, zero the likeliest: a shader's
    /// conditions test for zero, for sign and for equality with small
    /// constants, which [`Inputs::new`]'s spread almost never meets, so
    /// only these drive both arms of such an `if`.
    pub fn small(seed: u32) -> Inputs {
        Inputs::from_set(seed, &[0.0, 0.0, 1.0, -1.0, 2.0, -0.5])
    }

    /// Positive values around one, for conditions on a ratio or a size
    /// (a particle's age `(now - start) / life` in 0..1, a size against a
    /// minimum of 3) that `small`'s zeros turn into infinities.
    pub fn positive(seed: u32) -> Inputs {
        Inputs::from_set(seed, &[0.25, 0.5, 0.75, 1.0, 1.5, 2.0, 3.0, 4.0])
    }

    /// Log-uniform magnitudes from 0.001 to 1000, either sign, and an exact
    /// zero a quarter of the time (half, on odd seeds): what the search for
    /// an undriven `if` draws from. One non-zero draw in four is instead one
    /// of `literals` exactly: the program's own constants, which is what an
    /// equality condition compares against (`ubuf.entry == 2.0` in Qt's
    /// particle shaders). A log-uniform draw never meets 2.0 exactly, so
    /// without them such an `if` is driven only by the fixed runs' small
    /// sets, and whether it is depends on which registers the program's
    /// inputs happen to land in. An empty list draws as before the literals.
    pub fn mixed_from(seed: u32, literals: &[f32]) -> Inputs {
        let mut x = seed.wrapping_mul(3_266_489_917).wrapping_add(11);
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            let mag = 10f32.powf(((x >> 8) % 6001) as f32 / 1000.0 - 3.0);
            /* An exact zero one time in four, or on odd seeds one time in
             * two: `all(equal(v, vec2(0.0)))` needs two at once, and
             * `length(a * t + b) > 0.0` four. */
            let zero = if seed % 2 == 1 { (x >> 1) & 1 == 0 } else { (x >> 1) & 3 == 0 };
            if zero {
                0
            } else if !literals.is_empty() && (x >> 24) & 3 == 0 {
                literals[((x >> 26) as usize) % literals.len()].to_bits()
            } else {
                (if x & 1 == 0 { mag } else { -mag }).to_bits()
            }
        };
        Inputs {
            shared: (0..512).map(|_| next()).collect(),
            vtxin: (0..128).map(|_| next()).collect(),
            coeff: (0..256).map(|_| next()).collect(),
            savmsk: 1,
            samp_num: 0,
            frag_xy: [0; 4],
            face_ccw: 0,
            samples: 4,
            pixout: 0,
            pixout1: 0,
        }
    }

    fn from_set(seed: u32, set: &[f32]) -> Inputs {
        let mut x = seed.wrapping_mul(2_246_822_519).wrapping_add(7);
        let mut next = || {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            set[(x % set.len() as u32) as usize].to_bits()
        };
        Inputs {
            shared: (0..512).map(|_| next()).collect(),
            vtxin: (0..128).map(|_| next()).collect(),
            coeff: (0..256).map(|_| next()).collect(),
            savmsk: 1,
            samp_num: 0,
            frag_xy: [0; 4],
            face_ccw: 0,
            samples: 4,
            pixout: 0,
            pixout1: 0,
        }
    }
}

/// flr, then abs, then neg: the order a source modifier applies in. The two
/// interpreters below share it, so they agree whatever the hardware's order;
/// it matters only that prepare's moves preserve it, which they do by
/// carrying the same three flags.
pub fn apply_mods(v: u32, neg: bool, abs: bool, flr: bool) -> u32 {
    let mut f = f32::from_bits(v);
    if flr {
        f = f.floor();
    }
    if abs {
        f = f.abs();
    }
    if neg {
        f = -f;
    }
    f.to_bits()
}

pub fn const_value(idx: u32) -> Option<u32> {
    CONST_REGS.iter().find(|(_, i)| *i as u32 == idx).map(|(v, _)| *v)
}

pub fn alu(op: &str, a: &[u32]) -> Option<u32> {
    let f = |i: usize| f32::from_bits(a[i]);
    Some(
        match op {
            "fadd" => f(0) + f(1),
            "fmul" => f(0) * f(1),
            "fmad" => f(0) * f(1) + f(2),
            "frcp" => 1.0 / f(0),
            "frsq" => 1.0 / f(0).sqrt(),
            "fexp" => f(0).exp2(),
            "flog" => f(0).log2(),
            /* A derivative is not a function of one fragment's value; the
             * model gives each a fixed function of its operand, the same on
             * both sides of a comparison, which is all it has to be for the
             * question "do the two programs take the same derivative of the
             * same value". */
            "fdsx" => f(0) * 0.25 + 0.125,
            "fdsy" => f(0) * -0.5 + 0.0625,
            "mbyp" => return Some(a[0]),
            _ => return None,
        }
        .to_bits(),
    )
}

/// The output key [`run_binary`] records the `alphaf` pass feedback under:
/// present when the fragment is kept.
pub const FEEDBACK: u32 = u32::MAX;

/// What the trig model's `fred` part A gives for x: a mark no swap of part
/// B's operands can satisfy (a rotation first, so that it is not its own
/// inverse as a bare XOR would be).
pub fn fred_a_tag(x: u32) -> u32 {
    x.rotate_left(7) ^ 0x5a5a_5a5a
}

/// `samp_num` (Mesa `pco_isa.py`).
pub const SR_SAMP_NUM: u32 = 53;

/// An IEEE half's value (subnormals, infinities and NaN included).
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let e = (h >> 10) & 0x1f;
    let m = f32::from(h & 0x3ff);
    match e {
        0 => sign * m * 2f32.powi(-24),
        31 if m == 0.0 => sign * f32::INFINITY,
        31 => f32::NAN,
        _ => sign * (1.0 + m / 1024.0) * 2f32.powi(i32::from(e) - 15),
    }
}

/// `x` rounded to the nearest IEEE half (ties to even), saturating to
/// infinity; NaN stays NaN.
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    if x.is_nan() {
        return sign | 0x7e00;
    }
    let a = x.abs();
    if a >= 65520.0 {
        return sign | 0x7c00;
    }
    if a < 2f32.powi(-24) * 0.5 {
        return sign;
    }
    /* Scale into the half's integer mantissa at its exponent, round to even. */
    let e = a.log2().floor().max(-14.0) as i32;
    let q = a / 2f32.powi(e - 10);
    let r = q.round_ties_even() as u32;
    let (e, r) = if r >= 2048 { (e + 1, r / 2) } else { (e, r) };
    if e < -14 || r < 1024 {
        return sign | r as u16;
    }
    sign | (((e + 15) as u16) << 10) | (r as u16 & 0x3ff)
}

/// The coverage mask `pck.cov` makes of an alpha: one sample per quarter,
/// from the first. A model, the same on every side of a comparison; 1.0
/// covers every sample and 0.0 none.
pub fn cov_model(x: u32) -> u32 {
    cov_model_of(x, 4)
}

/// [`cov_model`] for a draw of `samples` samples per pixel: 1.0 covers them
/// all, so a single-sampled draw's mask of 1.0 is 1 (pco's
/// `fs_is_single_sampled` shifts it right and tests for zero).
pub fn cov_model_of(x: u32, samples: u32) -> u32 {
    let a = f32::from_bits(x);
    let n = if a.is_nan() { 0 } else { (a.clamp(0.0, 1.0) * samples as f32).round() as u32 };
    (1u32 << n) - 1
}

/// What an unwritten temporary reads as: -3.0e30, finite, and not the
/// selection side's poison.
pub const POISON_BINARY: u32 = 0xf217_7617;

/// Run a USC binary -- ours, or the reference's -- decoding each group and
/// executing what it says, reading the register files by the ISA table's
/// bank names. Knows the single-operation groups the builder emits, the
/// immediate move, the vertex output write and the end of task, and refuses
/// anything else by name. Returns the vertex output
/// by dword.
///
/// `whole`: the image must end exactly at the end of task (our own shader);
/// a reference blob may carry further programs after it, so it stops there.
pub fn run_binary(image: &[u8], inp: &Inputs, whole: bool) -> Result<BTreeMap<u32, u32>, String> {
    let mut temp: BTreeMap<u32, u32> = BTreeMap::new();
    let mut out: BTreeMap<u32, u32> = BTreeMap::new();
    /* The attribute file is writable: the reference reuses it as scratch. */
    let mut vtx = inp.vtxin.clone();
    let regbank = ENUMS.iter().find(|e| e.name == "regbank").unwrap();
    /* Index register 0 (`Bank::Idx0`): set by a write of index 5, read
     * through by `3 | offset << 3` (the shared file at idx0 + offset). */
    let idx0 = std::cell::Cell::new(None::<u32>);
    let read = |o: Operand, temp: &BTreeMap<u32, u32>, vtx: &Vec<u32>| -> Result<u32, String> {
        match regbank.name_of(o.bank) {
            Some("idx0") if o.index & 7 == 3 => {
                let base = idx0.get().ok_or("a read through idx0 before anything set it")?;
                let at = base.checked_add(o.index >> 3).ok_or("idx0 overflows")?;
                inp.shared.get(at as usize).copied().ok_or(format!("shared[{at}] through idx0 is out of range"))
            }
            /* An unwritten temporary reads as a poison value (see
             * `POISON_SELECTED` in usc_limits.rs, which differs). */
            Some("temp") => Ok(temp.get(&o.index).copied().unwrap_or(POISON_BINARY)),
            Some("shared") => inp.shared.get(o.index as usize).copied().ok_or("shared out of range".into()),
            /* A read of the coefficient file itself: a flat varying's plane
             * constant (`fragment::flat_group`). */
            Some("coeff") => inp.coeff.get(o.index as usize).copied().ok_or("coeff out of range".into()),
            Some("vtxin") => vtx.get(o.index as usize).copied().ok_or("vtxin out of range".into()),
            Some("special") if o.index == SR_SAMP_NUM => Ok(inp.samp_num),
            /* The pixel output register, read before anything wrote it: the
             * colour already in the pixel (a blend variant). */
            Some("special") if o.index == 32 => Ok(inp.pixout),
            Some("special") if o.index == 33 => Ok(inp.pixout1),
            Some("special") if o.index == 44 => Ok(inp.face_ccw),
            Some("special") if matches!(o.index, 97 | 98 | 100 | 101) => {
                Ok(inp.frag_xy[[97, 98, 100, 101].iter().position(|r| *r == o.index).unwrap_or(0)])
            }
            Some("special") => const_value(o.index).ok_or(format!("special{} holds no known constant", o.index)),
            b => Err(format!("a read of bank {b:?}")),
        }
    };
    let mut p0 = false;
    /* The instance's execution predicate: active until a `cnd*` operation
     * says otherwise (pco's execution-mask construction, `powervr::cf`).
     * One instance is modelled: an instance is active while its execution
     * mask counter (the emc register the `cnd*` groups rewrite) is 0, and a
     * branch taken when NO instance is active (`bpred` allp) is taken when
     * this one is not. The semantics are inferred from how pco_cf.c uses the
     * operations (Mesa gives their encodings, not their behaviour); pco's own
     * programs that use them are run against the IR by the survey, which is
     * the check on the inference. What one instance cannot show -- instances
     * going different ways -- is the board's to test. */
    let mut pe = true;
    /* Which function each `fred` part B result was reduced for, by the
     * temporary it landed in: `fsinc` reads it from there. */
    let mut trig_of: BTreeMap<u32, &str> = BTreeMap::new();
    let mut off = 0usize;
    /* A program that never ends is a failure of its own, not a hung test.
     * The bound follows from the structure: a loop runs at most
     * UNIFORM_LOOP_CAP passes of a body no longer than the program (a group
     * is at least four bytes), and a program with none is far shorter than a
     * million groups. It was a flat million until a divergent loop ran to the
     * cap over a long body (Qt's volume ray march: 4,096 passes of a few
     * hundred groups), which is a long program, not one that never ends.
     * Nested loops run to the cap can exceed even this, and stay unmeasured. */
    let budget = (1usize << 20)
        .max((yakogl::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP as usize + 2) * (image.len() / 4));
    let mut steps = 0usize;
    while off < image.len() {
        steps += 1;
        if steps > budget {
            return Err("the program ran more groups than its loops allow without ending".into());
        }
        let mut jump: Option<i64> = None;
        let g = decode_group(image, off).map_err(|e| format!("decode at {off:#x}: {e:?}"))?;
        let names: Vec<&str> =
            g.instrs().map(|(_, ph, x)| ops_for(x, ph).next().map(|o| o.name).unwrap_or("<unknown>")).collect();
        let (_, phase, r) = g.instrs().next().ok_or("a group with no operation")?;
        /* The execution condition. A group whose condition fails does
         * nothing: its registers, outputs and feedback are left as they
         * were. (A branch's condition only chooses between two ways to the
         * next group here, which the branch arm below checks.) */
        let runs = match fname(&g.hdr, "cc") {
            Some("e1_zx") | None => pe,
            Some("if(p0)") => pe && p0,
            Some("if(!p0)") => pe && !p0,
            /* the `cnd*` groups, which run whatever the predicate */
            Some("(ignorepe)") => true,
            Some(c) => return Err(format!("an execution condition the model does not know: {c}")),
        };
        /* `cnd*`: the execution mask counter c (the emc register, read from
         * s0 and written back) and the predicate, pe = (c == 0).
         *   cndst  adjust: c = s0 != 0 ? s0 + adjust : (pcnd ? 0 : adjust)
         *   cndef  adjust: c == 0 -> adjust, c == adjust -> 0 (the else arm)
         *   cndsm  (s2):   an active instance where pcnd holds takes c = s2
         *   cndlt  adjust: c == 0 -> p0 set (around again); else c -= adjust
         *                  and p0 clear (out of the loop)
         *   cndend adjust: c -= adjust, not below 0 */
        if let [op @ ("cndst" | "cndef" | "cndsm" | "cndlt" | "cndend")] = names.as_slice() {
            let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
            let c = read(lo[0].ok_or("a cnd group with no s0")?, &temp, &vtx)?;
            let adjust = r.get("adjust").unwrap_or(0);
            let holds = match fname(r, "pcnd") {
                Some("always") | None => true,
                Some("p0_true") => p0,
                Some("p0_false") => !p0,
                Some("never") => false,
                Some(x) => return Err(format!("a cnd condition the model does not know: {x}")),
            };
            let next = match *op {
                "cndst" => {
                    if c != 0 {
                        c + adjust
                    } else if holds {
                        0
                    } else {
                        adjust
                    }
                }
                "cndef" if c == 0 => adjust,
                "cndef" if c == adjust => 0,
                "cndef" => c,
                "cndsm" if c == 0 && holds => read(lo[2].ok_or("cndsm with no s2")?, &temp, &vtx)?,
                "cndsm" => c,
                "cndlt" if c == 0 => {
                    p0 = true;
                    0
                }
                "cndlt" => {
                    p0 = false;
                    c.saturating_sub(adjust)
                }
                _ => c.saturating_sub(adjust),
            };
            let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("a cnd group with no destination")?;
            if regbank.name_of(d.bank) != Some("temp") {
                return Err("a cnd group writing other than a temporary".into());
            }
            temp.insert(d.index, next);
            pe = next == 0;
            off += g.total;
            continue;
        }
        if !runs && !matches!(names.as_slice(), ["br" | "br.next"]) {
            off += g.total;
            continue;
        }
        /* `tstz` into p0 with no destination: the discard feedback's test
         * (`shapes::KILL_TEST`). p0 is set when the flag is zero. */
        if names == ["mbyp", "pck", "tst", "movc"] && g.dst.is_none() {
            let t = g.instrs().nth(2).ok_or("no tst")?.2;
            if fname(t, "tst_op") != Some("z") || t.get("pwen") != Some(1) || fname(t, "type") != Some("u32") {
                return Err("a predicate test other than tst z u32 into p0".into());
            }
            let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
            p0 = read(lo[0].ok_or("tstz with no s0")?, &temp, &vtx)? == 0;
            off += g.total;
            continue;
        }
        /* `psel_trig`: the `fsinc` result (s3) when p0 is clear, times part
         * B (s4) when it is set (`shapes::PSEL_TRIG`). */
        if names == ["imadd64", "fmul", "tst", "movc"] {
            let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
            let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
            if fname(g.lower.as_ref().ok_or("no lower")?, "is0") != Some("s3") {
                return Err("a psel_trig whose is0 is not s3".into());
            }
            for s in &lo[..3] {
                if s.is_none_or(|o| regbank.name_of(o.bank) != Some("special") || o.index != 0) {
                    return Err("a psel_trig whose s0..s2 are not zero".into());
                }
            }
            let t = read(up[3].ok_or("psel_trig with no s3")?, &temp, &vtx)?;
            let r = read(up[4].ok_or("psel_trig with no s4")?, &temp, &vtx)?;
            let v = if p0 { (f32::from_bits(t) * f32::from_bits(r)).to_bits() } else { t };
            let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("psel_trig with no destination")?;
            temp.insert(d.index, v);
            off += g.total;
            continue;
        }
        /* One operation per group, except `flush_p0`'s imadd32 + tst and the
         * select/compare shapes, modelled below. */
        if names.len() > 1 && names != ["imadd32", "tst"] {
            let val = run_wired(&g, &names, &|o| read(o, &temp, &vtx))?;
            let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("no destination")?;
            match regbank.name_of(d.bank) {
                Some("temp") => {
                    temp.insert(d.index, val);
                }
                Some("vtxin") => *vtx.get_mut(d.index as usize).ok_or("vtxin write out of range")? = val,
                b => return Err(format!("a wired group writing bank {b:?}")),
            }
            off += g.total;
            continue;
        }
        let op = ops_for(r, phase).next().map(|o| o.name).unwrap_or("<unknown>");
        let s = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
        let m = |f: &str| r.get(f).unwrap_or(0) != 0;
        match op {
            "uvsw.emit.endtask" => {
                off += g.total;
                if whole && off != image.len() {
                    return Err("end of task before the end of the image".into());
                }
                break;
            }
            "uvsw.write" => {
                let base = r.get("imm_addr").ok_or("uvsw.write with no imm_addr")?;
                let rpt = g.hdr.get("rpt").unwrap_or(1);
                let s0 = s[0].ok_or("uvsw.write with no source")?;
                for k in 0..rpt {
                    let val = read(Operand { bank: s0.bank, index: s0.index + k }, &temp, &vtx)?;
                    if out.insert(base + k, val).is_some() {
                        return Err(format!("output dword {} written twice", base + k));
                    }
                }
            }
            /* `pck` u8888 with scale and a repeat of four: four floats from
             * s2.. clamped to 0..1, scaled to 0..255, rounded, byte k from
             * source k. Anything else is refused rather than modelled. The
             * model is shared by both sides of a comparison, so it decides
             * nothing about which side is right -- only whether both mean
             * the same packing. */
            /* `pck.cov`: the coverage mask alpha to coverage derives. */
            "pck" if fname(r, "pck_format") == Some("cov") => {
                if g.hdr.get("rpt").unwrap_or(1) != 1 {
                    return Err("a repeated pck.cov".into());
                }
                let x = read(s[2].ok_or("pck.cov with no s2")?, &temp, &vtx)?;
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("pck.cov with no destination")?;
                temp.insert(d.index, cov_model_of(x, inp.samples));
            }
            /* `fred`: a model of the range reduction, whose arithmetic the
             * ISA tables do not give. Part A gives x tagged (into w1), so
             * that part B's two operands cannot be swapped unseen;
             * part B must be handed x and part A's result, passes x on and
             * notes which function it reduced for; `fsinc` gives sin(r)/r
             * and sets p0, or cos(r) and clears it; `psel_trig` multiplies
             * back by r under p0. The chain yields sin/cos of x only when
             * every link reads what the previous one wrote. */
            "fred" => {
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
                let d = d[0].or(d[1]).ok_or("fred with no destination")?;
                let x = read(s[0].ok_or("fred with no s0")?, &temp, &vtx)?;
                let ty = fname(r, "red_type").ok_or("fred with no type")?;
                match fname(r, "red_part") {
                    Some("a") => {
                        temp.insert(d.index, fred_a_tag(x));
                    }
                    Some("b") => {
                        let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
                        let a = read(up[3].ok_or("fred part B with no s3")?, &temp, &vtx)?;
                        if a != fred_a_tag(x) {
                            return Err("a fred part B whose s3 is not part A of its x".into());
                        }
                        temp.insert(d.index, x);
                        trig_of.insert(d.index, ty);
                    }
                    o => return Err(format!("a fred part {o:?}")),
                }
            }
            "fsinc" => {
                let src = s[0].ok_or("fsinc with no s0")?;
                let ty = *trig_of.get(&src.index).ok_or("an fsinc of something no fred part B wrote")?;
                let x = f32::from_bits(read(src, &temp, &vtx)?);
                let (t, p) =
                    if ty == "sin" { (if x == 0.0 { 1.0 } else { x.sin() / x }, true) } else { (x.cos(), false) };
                p0 = p;
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("fsinc with no destination")?;
                temp.insert(d.index, t.to_bits());
            }
            "savmsk" => {
                if fname(r, "msk_mode") != Some("vm") {
                    return Err("a savmsk other than vm".into());
                }
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("savmsk with no destination")?;
                temp.insert(d.index, inp.savmsk);
            }
            /* The pass feedback: the fragment is kept. Recorded as an
             * output of its own, so a comparison sees whether it was given. */
            "alphaf" => {
                out.insert(FEEDBACK, 1);
            }
            /* `pck` f16f16, repeat 1 or 2: floats to halves, rounding to
             * nearest even, the first into the low half (a float target). */
            "pck" if fname(r, "pck_format") == Some("f16f16") => {
                let rpt = g.hdr.get("rpt").unwrap_or(1);
                if r.get("scale") != Some(0) || r.get("rtz") != Some(0) || !(1..=2).contains(&rpt) {
                    return Err("a pck f16f16 other than unscaled, round to nearest, repeat 1 or 2".into());
                }
                let s2 = s[2].ok_or("pck with no s2")?;
                let mut v = 0u32;
                for k in 0..rpt {
                    let x = read(Operand { bank: s2.bank, index: s2.index + k }, &temp, &vtx)?;
                    v |= u32::from(f32_to_f16(f32::from_bits(x))) << (16 * k);
                }
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("pck with no destination")?;
                temp.insert(d.index, v);
            }
            /* `pck` s32 or u32 with round-to-zero: float to integer,
             * saturating as Rust's casts do. */
            "pck" if matches!(fname(r, "pck_format"), Some("s32" | "u32")) => {
                if r.get("rtz") != Some(1) || r.get("scale") != Some(0) || g.hdr.get("rpt").unwrap_or(1) != 1 {
                    return Err("a pck s32/u32 other than round-to-zero, unscaled, single".into());
                }
                let s2 = s[2].ok_or("pck with no s2")?;
                let x = f32::from_bits(read(s2, &temp, &vtx)?);
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("pck with no destination")?;
                let v = if fname(r, "pck_format") == Some("u32") { x as u32 } else { (x as i32) as u32 };
                temp.insert(d.index, v);
            }
            "pck" => {
                let rpt = g.hdr.get("rpt").unwrap_or(1);
                if r.get("pck_format") != Some(0) || r.get("scale") != Some(1) || rpt != 4 {
                    return Err("a pck other than u8888, scaled, repeat 4".into());
                }
                let s2 = s[2].ok_or("pck with no s2")?;
                let mut v = 0u32;
                for k in 0..4 {
                    let x = f32::from_bits(read(Operand { bank: s2.bank, index: s2.index + k }, &temp, &vtx)?);
                    let b = (x.clamp(0.0, 1.0) * 255.0).round() as u32;
                    v |= b << (8 * k);
                }
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("pck with no destination")?;
                if regbank.name_of(d.bank) != Some("temp") {
                    return Err("a pck to a file other than temp".into());
                }
                temp.insert(d.index, v);
            }
            /* The iterator: `count` components from coefficient `coff`
             * into temporaries from `dest`, perspective-divided (which the
             * model folds into the value it is handed). Only the form pco
             * emits for a smooth varying is modelled. */
            /* `ditr` iterates with no W divide; this model holds the
             * iterated value at the plane's first coefficient either way. */
            /* W's offset is not checked: the model folds the divide into the
             * value it is handed, so where W sits changes nothing it
             * computes. What checks it is the skeleton (woff is one of its
             * fields) against pco's corpus, which holds woff 4 wherever a
             * program reads gl_FragCoord.z beside a smooth varying. */
            "ditrp" | "ditr" => {
                if r.get("drc") != Some(0) || r.get("sat") != Some(0) {
                    return Err("a ditr other than channel 0, no saturate".into());
                }
                let (dest, coff, count) =
                    (r.get("dest").unwrap_or(0), r.get("coff").unwrap_or(0), r.get("count").unwrap_or(0));
                for k in 0..count {
                    let v = *inp.coeff.get((coff + 4 * k) as usize).ok_or("coefficient out of range")?;
                    temp.insert(dest + k, v);
                }
            }
            /* A fence's parts and the data fence change no register this
             * model holds; the branches must go to the next group, or the
             * program is not the straight line this interpreter walks. */
            "wdf" => {}
            /* A `nop` does nothing; the one that ends a program is seen by
             * `ends_program` below. */
            "nop" => {}
            /* `movs1` (`shapes::MOVS1`): s1 through is0 onto `fte`, written
             * by `movwm`'s w0. The wiring is checked, not assumed. */
            "movwm" => {
                let lower = g.lower.as_ref().ok_or("movwm with no lower section")?;
                let iss = g.iss.as_ref().ok_or("movwm with no iss")?;
                if fname(lower, "is0") != Some("s1")
                    || fname(iss, "is4") != Some("fte")
                    || fname(r, "movw0") != Some("fte")
                {
                    return Err("a movwm other than movs1's wiring".into());
                }
                let v = read(s[1].ok_or("movs1 with no s1")?, &temp, &vtx)?;
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("movs1 with no destination")?;
                if regbank.name_of(d.bank) != Some("temp") {
                    return Err("a movs1 writing other than a temporary".into());
                }
                temp.insert(d.index, v);
            }
            /* `br`, `br.next` and `br.skip_next` share one encoding and
             * `ops_for` names `br` first. */
            "br" | "br.next" => {
                if r.get("link") != Some(0) || r.get("abs") != Some(0) {
                    return Err("a branch that links or is absolute".into());
                }
                /* Anywhere else: taken when the header's condition holds
                 * (`bpred` cc), by the offset from this group's start. */
                let o = r.get("offset").ok_or("a branch with no offset")?;
                if o != g.total as u32 {
                    /* `allp`: taken when no instance is active (one
                     * instance: when this one is not); `anyp`: when any is. */
                    let taken = match fname(r, "bpred") {
                        Some("cc") => runs,
                        Some("allp") => !pe,
                        Some("anyp") => pe,
                        _ => return Err("a branch predicated other than by its condition".into()),
                    };
                    if taken {
                        jump = Some(i64::from(o as i32));
                    }
                }
            }
            /* A lone imadd32: `a * b + c` on 32-bit integers (the low half
             * is the same signed or not), no carry-in, no modifiers. */
            "imadd32" if names.len() == 1 => {
                if m("cin") || m("s1neg") || m("s2neg") || m("s1abs") || m("s2abs") || (m("s0abs") && m("s0neg")) {
                    return Err("an imadd32 with a carry or modifiers other than s0neg or s0abs".into());
                }
                let v =
                    |i: usize| -> Result<u32, String> { read(s[i].ok_or(format!("imadd32 slot {i}"))?, &temp, &vtx) };
                /* s0neg: the first operand negated as an integer; s0abs
                 * (`iabs`, signed): its magnitude, INT_MIN staying 2^31. */
                let a0 = if m("s0neg") {
                    v(0)?.wrapping_neg()
                } else if m("s0abs") {
                    if r.get("s") != Some(1) {
                        return Err("an unsigned imadd32 with s0abs".into());
                    }
                    (v(0)? as i32).wrapping_abs() as u32
                } else {
                    v(0)?
                };
                let val = a0.wrapping_mul(v(1)?).wrapping_add(v(2)?);
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("no destination")?;
                /* The bank as named: pco computes an indexed address in
                 * vtxin, which this arm used to write to temp. */
                match regbank.name_of(d.bank) {
                    Some("temp") => {
                        temp.insert(d.index, val);
                    }
                    Some("vtxin") => *vtx.get_mut(d.index as usize).ok_or("vtxin write out of range")? = val,
                    b => return Err(format!("an imadd32 writing bank {b:?}")),
                }
            }
            /* `unpck` u8888 scaled, repeated 4: the RGBA8 dword to four
             * floats in 0..1, R first (a blend variant's pixel). */
            "unpck" if fname(r, "pck_format") == Some("u8888") => {
                if r.get("scale_rtz") != Some(1) || g.hdr.get("rpt") != Some(4) || fname(r, "elem") != Some("e0") {
                    return Err("an unpck u8888 other than scaled, repeat 4, from element 0".into());
                }
                let x = read(s[0].ok_or("unpck with no source")?, &temp, &vtx)?;
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("no destination")?;
                for k in 0..4 {
                    temp.insert(d.index + k, (((x >> (8 * k)) & 0xff) as f32 / 255.0).to_bits());
                }
            }
            /* `unpck` f16f16, repeat 1 or 2: halves to floats, the low
             * half first (a float target's pixel). */
            "unpck" if fname(r, "pck_format") == Some("f16f16") => {
                let rpt = g.hdr.get("rpt").unwrap_or(1);
                if r.get("scale_rtz") != Some(0) || !(1..=2).contains(&rpt) || fname(r, "elem") != Some("e0") {
                    return Err("an unpck f16f16 other than unscaled, repeat 1 or 2, from element 0".into());
                }
                let x = read(s[0].ok_or("unpck with no source")?, &temp, &vtx)?;
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("no destination")?;
                for k in 0..rpt {
                    temp.insert(d.index + k, f16_to_f32((x >> (16 * k)) as u16).to_bits());
                }
            }
            /* `unpck` s32 or u32: an integer to float. */
            "unpck" => {
                let signed = match fname(r, "pck_format") {
                    Some("s32") => true,
                    Some("u32") => false,
                    _ => return Err("an unpck other than s32 or u32".into()),
                };
                if r.get("scale_rtz") != Some(0) {
                    return Err("an unpck with scale_rtz".into());
                }
                let x = read(s[0].ok_or("unpck with no source")?, &temp, &vtx)?;
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("no destination")?;
                temp.insert(d.index, if signed { x as i32 as f32 } else { x as f32 }.to_bits());
            }
            /* A lone `imadd64` (`umul_high`): the high word of the unsigned
             * product, into w1; the addend must be zero. */
            "imadd64" if names.len() == 1 => {
                if r.get("s") != Some(0) || m("cin") || m("s2neg") {
                    return Err("an imadd64 other than umul_high's".into());
                }
                let v = |i: usize| -> Result<u32, String> {
                    let o = if i < 3 { s[i] } else { g.upper.as_ref().map(srcs).unwrap_or([None; 6])[i] };
                    read(o.ok_or(format!("imadd64 slot {i}"))?, &temp, &vtx)
                };
                if v(2)? != 0 || v(3)? != 0 {
                    return Err("an imadd64 with an addend".into());
                }
                let hi = ((u64::from(v(0)?) * u64::from(v(1)?)) >> 32) as u32;
                let ds = g.dst.as_ref().map(dsts).unwrap_or([None; 2]);
                if g.hdr.get("w1p") != Some(1) || g.hdr.get("w0p") != Some(0) {
                    return Err("an imadd64 writing other than w1 alone".into());
                }
                let d = ds.into_iter().flatten().next().ok_or("umul_high with no destination")?;
                temp.insert(d.index, hi);
            }
            "imadd32" => {
                /* Only flush_p0: no destination, a tst after it. */
                if g.dst.is_some() || names != ["imadd32", "tst"] {
                    return Err("an imadd32 other than flush_p0".into());
                }
            }
            /* `smp` 2D, four channels: a deterministic function of the
             * image descriptor's first word (a marker the caller places for
             * each sampler) and the coordinates: the data block's first two
             * words, three for the projective form. Nothing past them is
             * read: pco's programs overlap the rest of the block with live
             * values (sprite.frag's data block starts on a register in use),
             * so the hardware cannot be reading it for these forms. This
             * model used to require it zero, as the first pco program seen
             * made it, and reported eleven of pco's programs unrunnable. */
            "smp" => {
                /* The level: implicit, or a data word after the coordinates
                 * that replaces it (textureLod: `lodm` replace with `pplod`)
                 * or is added to it (a bias: `lodm` bias with `pplod`), in
                 * pco's pco_emit_nir_smp order -- coordinates, divisor,
                 * level. This model's implicit level is 0 (no derivatives),
                 * so a bias b reads level b, as a replaced level b does: the
                 * two differ only in the field, which verify.rs checks. */
                /* Gradients (`lodm` gradients, no `pplod`): this model has
                 * no derivatives, so the level is 0 as for the implicit one;
                 * the gradient words are kept for the caller to check
                 * (`last_gradients`). */
                let gradients = fname(r, "lodm") == Some("gradients") && r.get("pplod").unwrap_or(0) == 0;
                let replace = match (fname(r, "lodm"), r.get("pplod").unwrap_or(0)) {
                    (Some("normal"), 0) => false,
                    (Some("gradients"), 0) => false,
                    (Some("replace" | "bias"), 1) => true,
                    _ => return Err("an smp whose level is neither implicit nor replaced or biased per pixel".into()),
                };
                /* 3d: the TPU's own cube (gl::native_cube), three coordinates;
                 * modelled through the same face mapping the IR's cube uses
                 * (`cube_uv`), so the three sides agree on what a cube reads */
                let cube = fname(r, "dmn") == Some("3d");
                if !(cube || fname(r, "dmn") == Some("2d")) || r.get("chan") != Some(4) {
                    return Err("an smp other than 2d or 3d, four channels".into());
                }
                let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
                let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
                let image = lo[0].ok_or("smp with no image")?;
                let data = lo[1].ok_or("smp with no data")?;
                let dest = up[4].ok_or("smp with no destination")?;
                if regbank.name_of(image.bank) != Some("shared") || regbank.name_of(data.bank) != Some("temp") {
                    return Err("an smp reading its descriptor or data from the wrong file".into());
                }
                /* The descriptor layout every smp of the corpus uses: the
                 * image at a multiple of 20, its sampler state 8 after it,
                 * and special register 0 in s3. */
                let sampler = lo[2].ok_or("smp with no sampler")?;
                let s3 = up[3].ok_or("smp with no s3")?;
                if image.index % 20 != 0
                    || regbank.name_of(sampler.bank) != Some("shared")
                    || sampler.index != image.index + 8
                    || regbank.name_of(s3.bank) != Some("special")
                    || s3.index != 0
                {
                    return Err("an smp whose descriptor operands are not the layout the corpus uses".into());
                }
                /* The extended form's fields: `proj` (texture2DProj: the data
                 * block holds x, y, q and the hardware divides) is modelled;
                 * any other one set is refused rather than ignored. */
                let proj = r.get("proj") == Some(1);
                for f in ["sno", "tao"] {
                    if r.get(f).unwrap_or(0) != 0 {
                        return Err(format!("an smp with {f}, which this model does not apply"));
                    }
                }
                if fname(r, "sbmode").is_some_and(|m| m != "none") {
                    return Err("an smp with a sample-block mode".into());
                }
                let marker = read(image, &temp, &vtx)?;
                let word = |k: u32| read(Operand { bank: data.bank, index: data.index + k }, &temp, &vtx);
                let (mut u, mut v) = (word(0)?, word(1)?);
                /* a cube's level word follows its three coordinates */
                let coords = if cube { 3 } else { 2 };
                if gradients {
                    if proj || r.get("soo") == Some(1) {
                        return Err("gradients with a divisor or an offset, which this model does not place".into());
                    }
                    let g: Vec<u32> = (coords..3 * coords).map(word).collect::<Result<_, _>>()?;
                    LAST_GRADIENTS.with(|c| c.borrow_mut().push(g));
                }
                if cube {
                    if proj {
                        return Err("a projective cube".into());
                    }
                    let explicit = if fname(r, "lodm") == Some("replace") { Some(word(3)?) } else { None };
                    (u, v) = cube_uv(tex_state(marker), [word(0)?, word(1)?, word(2)?], explicit);
                }
                if proj {
                    let q = f32::from_bits(word(2)?);
                    u = (f32::from_bits(u) / q).to_bits();
                    v = (f32::from_bits(v) / q).to_bits();
                }
                /* A replaced level reads another texture of the model per
                 * whole level, so a level dropped, misplaced in the block or
                 * computed wrongly reads other texels. */
                if replace {
                    let level = word(coords + u32::from(proj))?;
                    LAST_LEVELS.with(|c| c.borrow_mut().push(level));
                }
                let marker = if replace { marker_at_level(marker, word(coords + u32::from(proj))?) } else { marker };
                /* texelFetch: the coordinate in texels (`nncoords`). It must
                 * be a texel's centre: at a corner -- what pco passes, the
                 * integer coordinate itself -- a LINEAR sampler blends the
                 * four texels around it (gl_test case 48 read 0.75*80 +
                 * 0.25*40), which this nearest-texel model cannot show, so it
                 * refuses the program instead of reading one of them. */
                if r.get("nncoords") == Some(1) {
                    u = texel_coord(f32::from_bits(u))?;
                    v = texel_coord(f32::from_bits(v))?;
                }
                /* textureOffset: the lookup word after the level, u in bits
                 * 0..5 and v in 6..11, each a signed field (rogue_hw_defs.h) */
                let offset = if r.get("soo") == Some(1) {
                    lookup_offset(word(2 + u32::from(proj) + u32::from(replace))?)
                } else {
                    [0, 0]
                };
                for k in 0..4 {
                    temp.insert(dest.index + k, sample_model_at(marker, u, v, offset, k));
                }
            }
            "bbyp0bm_imm32" => {
                let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("imm move with no destination")?;
                let v = r.get("imm32").ok_or("no imm32")?;
                match regbank.name_of(d.bank) {
                    Some("temp") => {
                        temp.insert(d.index, v);
                    }
                    /* pco computes an indexed address through vtxin. */
                    Some("vtxin") => *vtx.get_mut(d.index as usize).ok_or("vtxin write out of range")? = v,
                    Some("idx0") if d.index == 5 => idx0.set(Some(v)),
                    b => return Err(format!("an immediate move to bank {b:?}")),
                }
            }
            _ => {
                let n = match op {
                    "fadd" | "fmul" => 2,
                    "fmad" => 3,
                    _ => 1,
                };
                /* The header's repeat: the operation `rpt` times, every
                 * register operand one further each time (pco copies a
                 * vector with one `mbyp` of repeat 2 or 4). A model that
                 * ignored it ran pco's programs wrongly. A constant under a
                 * repeat is refused rather than guessed at. */
                let rpt = g.hdr.get("rpt").unwrap_or(1);
                for k in 0..rpt {
                    let step = |o: Operand| -> Result<Operand, String> {
                        if k > 0 && regbank.name_of(o.bank) == Some("special") {
                            return Err(format!("{op} repeated over a special register"));
                        }
                        Ok(Operand { bank: o.bank, index: o.index + k })
                    };
                    let mut a = [0u32; 3];
                    for i in 0..n {
                        let o = step(s[i].ok_or(format!("{op}: no operand in slot {i}"))?)?;
                        let (ng, ab, fl) =
                            [("s0neg", "s0abs", "s0flr"), ("s1neg", "s1abs", "s1flr"), ("s2neg", "s2abs", "s2flr")][i];
                        a[i] = apply_mods(read(o, &temp, &vtx)?, m(ng), m(ab), m(fl));
                    }
                    let mut val = alu(op, &a).ok_or(format!("the interpreter does not know {op}"))?;
                    /* `sat`: the result clamped to 0..1 (pco's clamp(x, 0, 1)). A
                     * model that ignored it computed pco's smoothstep wrongly and
                     * reported a difference that was the interpreter's. */
                    if m("sat") {
                        val = f32::from_bits(val).clamp(0.0, 1.0).to_bits();
                    }
                    /* Any other instruction modifier this model does not apply is
                     * refused rather than ignored. */
                    for f in ["lp", "rtz", "scale"] {
                        if m(f) {
                            return Err(format!("{op} with {f}, which this model does not apply"));
                        }
                    }
                    let d = g.dst.as_ref().map(dsts).unwrap_or([None; 2])[0].ok_or("no destination")?;
                    let d = Operand { bank: d.bank, index: d.index + k };
                    match regbank.name_of(d.bank) {
                        Some("temp") => {
                            temp.insert(d.index, val);
                        }
                        /* The pixel output registers, from PIXOUT0 = special 32:
                         * the fragment shader's result, keyed from 0. */
                        Some("special") if d.index >= 32 && d.index < 36 => {
                            out.insert(d.index - 32, val);
                        }
                        Some("vtxin") => {
                            *vtx.get_mut(d.index as usize).ok_or("vtxin write out of range")? = val;
                        }
                        Some("idx0") if d.index == 5 && k == 0 => idx0.set(Some(val)),
                        b => return Err(format!("{op} writes bank {b:?} index {}", d.index)),
                    }
                }
            }
        }
        off = match jump {
            Some(j) => usize::try_from(off as i64 + j).map_err(|_| "a branch before the program's start")?,
            None => off + g.total,
        };
        /* A fragment shader ends on the group carrying `end`. */
        if yakogl::shaderc::online::powervr::assemble::ends_program(&g) {
            if whole && off != image.len() {
                return Err("end before the end of the image".into());
            }
            break;
        }
    }
    LAST_STEPS.with(|c| c.set(steps));
    Ok(out)
}

std::thread_local! {
    static LAST_STEPS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
    static LAST_GRADIENTS: std::cell::RefCell<Vec<Vec<u32>>> = const { std::cell::RefCell::new(Vec::new()) };
    static LAST_LEVELS: std::cell::RefCell<Vec<u32>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// The level words of every `smp` with a replaced level (`lodm` replace)
/// this thread has run since the last call, in order; taking them clears
/// the list.
#[allow(dead_code)]
pub fn take_levels() -> Vec<u32> {
    LAST_LEVELS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// How many groups the last `run_binary` on this thread executed (a loop's
/// body counted once per iteration): what a test that pins a loop's early
/// exit reads, since the values cannot show it -- the iterations after a
/// break change nothing.
pub fn last_steps() -> usize {
    LAST_STEPS.with(|c| c.get())
}

/// The gradient words (d/dx then d/dy per coordinate) of every `smp` with
/// `lodm` gradients this thread has run since the last call, in order;
/// taking them clears the list.
pub fn take_gradients() -> Vec<Vec<u32>> {
    LAST_GRADIENTS.with(|c| std::mem::take(&mut *c.borrow_mut()))
}

/// A deterministic value for component `k` of the input called `name`:
/// both programs of a comparison get the same number for the same name,
/// whatever register each puts it in.
pub fn named(seed: u32, name: &str, k: u32) -> u32 {
    if name.starts_with("pvr_layers_") {
        return layer_count(seed, name);
    }
    let mut h: u32 = 0x811c_9dc5 ^ seed.wrapping_mul(0x9e37_79b9);
    for b in name.bytes().chain(k.to_le_bytes()) {
        h = (h ^ b as u32).wrapping_mul(0x0100_0193);
    }
    (((h % 20_001) as f32 - 10_000.0) / 1_250.0).to_bits()
}

/// Files filled with a sentinel no named value can equal, different per
/// side, so a program reading a register nothing was placed in disagrees
/// with the other instead of agreeing by accident.
pub fn sentinel_inputs(sentinel: f32) -> Inputs {
    Inputs {
        shared: std::vec![sentinel.to_bits(); 512],
        vtxin: std::vec![sentinel.to_bits(); 128],
        coeff: std::vec![sentinel.to_bits(); 256],
        savmsk: 1,
        samp_num: 0,
        frag_xy: [0; 4],
        face_ccw: 0,
        samples: 4,
        pixout: 0,
        pixout1: 0,
    }
}

/// Every group of the offline table, as a skeleton.
pub fn corpus_skeletons() -> (BTreeSet<String>, usize) {
    let mut set = BTreeSet::new();
    let mut n = 0usize;
    for b in SHC_BLOB_TABLE.iter() {
        // SAFETY: `usc` points at a static of the generated table.
        let dw = unsafe { core::slice::from_raw_parts(b.shader.usc, b.shader.usc_dw as usize) };
        let bytes: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut off = 0;
        while off < bytes.len() {
            let Ok(g) = decode_group(&bytes, off) else { break };
            if g.total == 0 {
                break;
            }
            let mut k = String::new();
            skeleton(&mut k, &g).unwrap();
            set.insert(k);
            n += 1;
            off += g.total;
        }
    }
    (set, n)
}

/// A field's value by the name its enum gives it.
fn fname(r: &yakogl::shaderc::online::powervr::isa::Region, field: &str) -> Option<&'static str> {
    use yakogl::shaderc::online::powervr::isa::bit_set;
    let set = bit_set(r.set);
    let (i, u) = r.variant().uses.iter().enumerate().find(|(_, u)| u.name == field && u.fixed.is_none())?;
    let e = set.fields[u.field].enum_id?;
    ENUMS[e].name_of(set.fields[u.field].xform.decode(r.vals[i]))
}

/// A group wired through the internal registers -- `mbyp` into ft0 (s0) and
/// ft1 (s3), `fte` the lower slot `is0` selects, a `tst` of what `is1` and
/// `is2` select into ftt, an optional `pck` of a constant into ft2, and a
/// `movc` writing `movw0`'s register when ftt holds and `is4`'s when not --
/// modelled from pco's `group_map`s as `shapes.rs` transcribes them. The
/// model is shared by both sides of every comparison, so what it decides is
/// only whether two programs mean the same thing.
fn run_wired(
    g: &yakogl::shaderc::online::powervr::decode::Group,
    names: &[&str],
    read: &dyn Fn(Operand) -> Result<u32, String>,
) -> Result<u32, String> {
    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
    let bw = |i: usize| -> Result<u32, String> {
        read((if i < 3 { lo[i] } else { up[i] }).ok_or(format!("bitwise slot s{i} is empty"))?)
    };
    let nth = |k: usize| g.instrs().nth(k).map(|t| t.2).ok_or("a missing phase");
    /* `ubfe(value = s2, offset = s1, bits = s0)`: mask, OR with zero, shift
     * right by the offset again from s4 (`shapes::UBFE`). */
    if names == ["msk_bbyp0s1", "logical", "shift"] {
        let (l, sh) = (nth(1)?, nth(2)?);
        if fname(l, "logical_op") != Some("or") || l.get("mska") != Some(1) || l.get("mskb") != Some(1) {
            return Err("a masked logical other than ubfe's".into());
        }
        if fname(sh, "shift2_op") != Some("shr") || bw(3)? != 0 || bw(4)? != bw(1)? {
            return Err("a bit-field extract other than ubfe's wiring".into());
        }
        let (bits, offset, value) = (bw(0)?, bw(1)?, bw(2)?);
        let mask = if bits >= 32 { u32::MAX } else { (1u32 << bits) - 1 };
        return Ok(value.checked_shr(offset).unwrap_or(0) & mask);
    }
    /* `tstz` into a register (`shapes::TSTZ`): all ones when s0 is zero,
     * else the packed zero -- `movc` of `fte` (is0 = s1, the all-ones
     * special 143) or `ft2`. Every field of that wiring is checked, not
     * assumed, so another test with these four operations is refused. */
    if names == ["mbyp", "pck", "tst", "movc"] && g.dst.is_some() {
        let (pk, t, mv) = (nth(1)?, nth(2)?, nth(3)?);
        let lower = g.lower.as_ref().ok_or("tstz with no lower section")?;
        let iss = g.iss.as_ref().ok_or("tstz with no iss")?;
        let wired = fname(pk, "pck_format") == Some("zero")
            && fname(t, "tst_op") == Some("z")
            && fname(t, "type") == Some("u32")
            && t.get("pwen") == Some(0)
            && fname(mv, "movw0") == Some("fte")
            && fname(mv, "movw1") == Some("ft0")
            && fname(lower, "is0") == Some("s1")
            && fname(iss, "is4") == Some("ft2")
            && read(lo[1].ok_or("tstz with no s1")?)? == u32::MAX;
        if !wired {
            return Err("a test into a register other than tstz's wiring".into());
        }
        return Ok(if read(lo[0].ok_or("tstz with no s0")?)? == 0 { u32::MAX } else { 0 });
    }
    /* A shift of s1 by s4 (`shapes::ISHL`). */
    if names == ["bbyp0bm", "shift"] {
        let sh = nth(1)?;
        let (v, n) = (bw(1)?, bw(4)?);
        return match fname(sh, "shift2_op") {
            Some("lsl") => Ok(v.checked_shl(n).unwrap_or(0)),
            Some("shr") => Ok(v.checked_shr(n).unwrap_or(0)),
            /* arithmetic: the sign bit fills (NIR ishr); a count of 32 or
             * more is modelled as all sign bits, unmeasured on silicon and
             * undefined in GLSL */
            Some("asr_twb") => Ok((v as i32).checked_shr(n).unwrap_or(if (v as i32) < 0 { -1 } else { 0 }) as u32),
            o => Err(format!("a shift {o:?}")),
        };
    }
    /* The bitwise pipeline's two-operand logical: phase 0 passes `s2` on
     * (`bbyp0bm`/`bbyp0s1`, one encoding), phase 1 combines it with `s3`. */
    if names == ["bbyp0bm", "logical"] {
        if g.hdr.get("rpt").unwrap_or(1) != 1 {
            return Err("a repeated logical".into());
        }
        let mut it = g.instrs();
        let (_, _, p0) = it.next().ok_or("no phase 0")?;
        let (_, _, p1) = it.next().ok_or("no phase 1")?;
        for (f, want) in [("count_src", "s2"), ("count_op", "byp"), ("bitmask_src_op", "byp"), ("shift1_op", "byp")] {
            if fname(p0, f) != Some(want) {
                return Err(format!("a logical whose phase 0 {f} is {:?}", fname(p0, f)));
            }
        }
        if p1.get("mska").unwrap_or(1) != 0 || p1.get("mskb").unwrap_or(1) != 0 {
            return Err("a masked logical".into());
        }
        let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
        let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
        let a = read(lo[2].ok_or("logical with no s2")?)?;
        let b = read(up[3].ok_or("logical with no s3")?)?;
        return Ok(match fname(p1, "logical_op") {
            Some("or") => a | b,
            Some("and") => a & b,
            Some("xor") => a ^ b,
            Some("xnor") => !(a ^ b),
            o => return Err(format!("a logical {o:?}")),
        });
    }
    let known: [&[&str]; 4] = [
        &["mbyp", "mbyp", "tst", "movc"],
        &["mbyp", "mbyp", "pck", "tst", "movc"],
        &["fmul", "mbyp", "tst", "movc"],
        &["fadd", "fadd", "tst", "movc"],
    ];
    if !known.contains(&names) {
        return Err(format!("a wired group this model does not know: {names:?}"));
    }
    let mut slot = [None; 6];
    let lo = g.lower.as_ref().map(srcs).unwrap_or([None; 6]);
    let up = g.upper.as_ref().map(srcs).unwrap_or([None; 6]);
    for i in 0..6 {
        slot[i] = if i < 3 { lo[i] } else { up[i] };
    }
    let val = |i: usize| -> Result<u32, String> { read(slot[i].ok_or(format!("slot s{i} is empty"))?) };
    let instr =
        |n: &str| g.instrs().find(|(_, ph, x)| ops_for(x, *ph).next().is_some_and(|o| o.name == n)).map(|t| t.2);
    /* Phase 0 fills ft0 and phase 1 ft1: a bypass of s0 and s3, or (sign)
     * a saturating multiply of s0 by s1 and a negated bypass of s3. */
    let mods = |r: &yakogl::shaderc::online::powervr::isa::Region, f: &str| r.get(f).unwrap_or(0) != 0;
    let (p0, p1) = {
        let mut it = g.instrs();
        let a = it.next().ok_or("no phase 0")?;
        let b = it.next().ok_or("no phase 1")?;
        (a.2, b.2)
    };
    /* A sum in phase 0 and 1 (`shapes::FCEIL`): each operand's own source
     * modifiers, floor included, then the add. */
    let add = |r: &yakogl::shaderc::online::powervr::isa::Region, a: u32, b: u32| -> u32 {
        let x = apply_mods(a, mods(r, "s0neg"), mods(r, "s0abs"), mods(r, "s0flr"));
        let y = apply_mods(b, false, mods(r, "s1abs"), false);
        (f32::from_bits(x) + f32::from_bits(y)).to_bits()
    };
    let ft0 = if names[0] == "fadd" {
        add(p0, val(0)?, val(1)?)
    } else if names[0] == "fmul" {
        let x = f32::from_bits(val(0)?) * f32::from_bits(val(1)?);
        let x = if mods(p0, "sat") {
            if x.is_nan() {
                0.0
            } else {
                x.clamp(0.0, 1.0)
            }
        } else {
            x
        };
        x.to_bits()
    } else {
        /* A bypass's own source modifiers: pco folds `step(0.0, -x)`'s
         * negate into phase 0's `mbyp` (shapestroke.frag). A model that
         * read only phase 1's ran pco's step backwards. */
        apply_mods(val(0)?, mods(p0, "s0neg"), mods(p0, "s0abs"), false)
    };
    let ft1 = if names[1] == "fadd" {
        add(p1, val(3)?, val(4)?)
    } else {
        apply_mods(val(3)?, mods(p1, "s0neg"), mods(p1, "s0abs"), false)
    };
    let is0 = g.lower.as_ref().and_then(|l| fname(l, "is0")).unwrap_or("s0");
    let fte = val(is0[1..].parse::<usize>().map_err(|_| "is0")?)?;
    let iss = g.iss.as_ref().ok_or("no ISS")?;
    let pick = |n: &str, ft2: u32| -> Result<u32, String> {
        Ok(match n {
            "ft0" => ft0,
            "ft1" => ft1,
            "ft2" => ft2,
            "fte" => fte,
            o => return Err(format!("an internal register the model does not know: {o}")),
        })
    };
    let ft2 = match instr("pck") {
        Some(p) => match fname(p, "pck_format") {
            Some("zero") => 0,
            Some("one") => 1.0f32.to_bits(),
            o => return Err(format!("a pck of {o:?}")),
        },
        None => 0,
    };
    let a = pick(fname(iss, "is1").ok_or("is1")?, ft2)?;
    let b = pick(fname(iss, "is2").ok_or("is2")?, ft2)?;
    let t = instr("tst").ok_or("no tst")?;
    /* The narrow `tst` form has no type field: it encodes the field's zero,
     * which the tst_type enum names f32. */
    let ty = match fname(t, "type") {
        Some(n) => n,
        None if t.variant().uses.iter().all(|u| u.name != "type") => "f32",
        None => return Err("a tst type outside its enum".into()),
    };
    let op = fname(t, "tst_op").ok_or("tst op")?;
    let ord = match ty {
        "f32" => f32::from_bits(a).partial_cmp(&f32::from_bits(b)),
        "u32" => Some(a.cmp(&b)),
        "s32" => Some((a as i32).cmp(&(b as i32))),
        o => return Err(format!("a tst of type {o}")),
    };
    use std::cmp::Ordering::*;
    let ftt = match op {
        "z" => a == 0 && ty != "f32" || ty == "f32" && f32::from_bits(a) == 0.0,
        /* Against zero, by type. */
        "gz" => match ty {
            "f32" => f32::from_bits(a) > 0.0,
            "s32" => (a as i32) > 0,
            _ => a > 0,
        },
        "gez" => match ty {
            "f32" => f32::from_bits(a) >= 0.0,
            "s32" => (a as i32) >= 0,
            _ => true,
        },
        "l" => ord == Some(Less),
        "le" => matches!(ord, Some(Less | Equal)),
        "g" => ord == Some(Greater),
        "ge" => matches!(ord, Some(Greater | Equal)),
        "e" => ord == Some(Equal),
        "ne" => ord != Some(Equal),
        o => return Err(format!("a tst_op the model does not know: {o}")),
    };
    let m = instr("movc").ok_or("no movc")?;
    Ok(if ftt { pick(fname(m, "movw0").ok_or("movw0")?, ft2)? } else { pick(fname(iss, "is4").ok_or("is4")?, ft2)? })
}

/// The four image state words of the model's texture `marker` (the
/// descriptor's words 0..3, TEXSTATE IMAGE_WORD0 and IMAGE_WORD1): word 0 is
/// the marker itself, which is what a sample reads, the others derived from
/// it. Every field a `textureSize` reads -- the textype (word 0 bits 0..2),
/// width and height less one (bits 34..47 and 48..61 of the first pair), the
/// base level (bits 60..63 of the second) -- is thereby some value, the same
/// for pco's program, ours and the IR.
pub fn tex_state(marker: u32) -> [u32; 4] {
    let mix = |k: u32| (marker ^ k.wrapping_mul(0x9e37_79b9)).wrapping_mul(0x85eb_ca6b).rotate_left(11);
    [marker, mix(1), mix(2), mix(3)]
}

/// The size `textureSize` reports for a 2D image of the model at level
/// `lod`: pco's usclib_tex_state_size (usclib/tex.cl, MIT, YakoGL's copy) for
/// two components -- width and height less one from IMAGE_WORD0, the base
/// level from IMAGE_WORD1 added to `lod` unless the image is a STRIDE one
/// (textype 4), each size shifted right by that level and at least 1.
pub fn tex_size_2d(state: [u32; 4], lod: u32) -> [u32; 2] {
    let lod = if state[0] & 7 != 4 { lod.wrapping_add(state[3] >> 28) } else { lod };
    let w = ((state[1] >> 2) & 0x3fff) + 1;
    let h = ((state[1] >> 16) & 0x3fff) + 1;
    [w, h].map(|x| x.checked_shr(lod).unwrap_or(0).max(1))
}

/// Where a cube look-up samples the stacked 2D image YakoGL keeps a cube in:
/// glsl2usc's `pvr_cubeUV` (tools/yakogl-tools/src/glsl.rs EMU_HELPERS),
/// written as the helper is, over the model's image state for the face
/// height. Returns the 2D coordinate's bits.
pub fn cube_uv(state: [u32; 4], d: [u32; 3], lod: Option<u32>) -> (u32, u32) {
    let [x, y, z] = d.map(f32::from_bits);
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, ma, mut u, mut v);
    if ax >= ay && ax >= az {
        ma = ax;
        (face, u, v) = if x > 0.0 { (0.0, -z, -y) } else { (1.0, z, -y) };
    } else if ay >= az {
        ma = ay;
        (face, u, v) = if y > 0.0 { (2.0, x, z) } else { (3.0, x, -z) };
    } else {
        ma = az;
        (face, u, v) = if z > 0.0 { (4.0, x, -y) } else { (5.0, -x, -y) };
    }
    let r = 1.0 / ma;
    u = (u * r) * 0.5 + 0.5;
    v = (v * r) * 0.5 + 0.5;
    let mut h = tex_size_2d(state, 0)[1] as i32 as f32 * (1.0f32 / 6.0);
    /* an explicit level: the face at the coarser level a filter reads
     * (glsl2usc's pvr_cubeUVLod) */
    if let Some(l) = lod {
        h = (h * (-f32::from_bits(l).max(0.0).ceil()).exp2()).max(1.0);
    }
    let lo = 0.5 * (1.0 / h);
    let hi = 1.0 + -lo;
    /* v only: u is the sampler's to wrap inside the face (select.rs cube_uv) */
    v = v.max(lo).min(hi);
    (u.to_bits(), ((v + face) * (1.0f32 / 6.0)).to_bits())
}

/// Where a 2D array look-up samples the stacked 2D image YakoGL keeps the
/// array in: glsl2usc's `pvr_arrUV` (tools/yakogl-tools/src/glsl.rs
/// EMU_HELPERS), written as the helper is, `layers` the float the GL core
/// writes to `pvr_layers_<sampler>`. Returns the 2D coordinate's bits.
pub fn array_uv(state: [u32; 4], layers: u32, c: [u32; 3]) -> (u32, u32) {
    let [x, y, z] = c.map(f32::from_bits);
    let layers = f32::from_bits(layers);
    let l = (z + 0.5).floor().max(0.0).min(layers + -1.0);
    let rl = 1.0 / layers;
    let h = tex_size_2d(state, 0)[1] as i32 as f32 * rl;
    let lo = 0.5 * (1.0 / h);
    let hi = 1.0 + -lo;
    let v = y.max(lo).min(hi);
    (x.to_bits(), ((v + l) * rl).to_bits())
}

/// A `texelFetch` coordinate in texels (an unnormalised sample's), as the
/// normalised one the model samples at: texel `floor(t)` of the model's 64,
/// repeating. Refused unless `t` is a texel's centre -- the only place a
/// LINEAR sampler returns one texel exactly.
///
/// A coordinate of 2^22 texels or more lies outside every image the TPU
/// holds (16384 at most), where GLSL leaves texelFetch undefined and an
/// f32 has no half left to test; it reads one fixed texel of the model
/// (a NaN coordinate's), the same for every program -- random inputs reach
/// it (a vertex shader indexing a texture by an attribute read -63,082,336).
pub fn texel_coord(t: f32) -> Result<u32, String> {
    if t.is_finite() && t.abs() >= 4_194_304.0 {
        return Ok(f32::NAN.to_bits());
    }
    if !(t.is_finite() && (t - t.floor()) == 0.5) {
        return Err(format!(
            "an unnormalised sample at {t}, not a texel centre: a LINEAR sampler blends the texels around it"
        ));
    }
    Ok((t / 64.0).to_bits())
}

/// The texel offset a sample's lookup word carries: u in bits 0..5, v in
/// 6..11, each a two's-complement field of six bits.
pub fn lookup_offset(word: u32) -> [i32; 2] {
    let field = |x: u32| (((x & 0x3f) << 26) as i32) >> 26;
    [field(word), field(word >> 6)]
}

/// A 3D texture's sample through the grid of slices YakoGL keeps it in:
/// glsl2usc's `pvr_vol` / `pvr_volUV` (tools/yakogl-tools/src/glsl.rs
/// EMU_HELPERS), written as the helpers are, each operation as the byte
/// model computes the program's (sqrt as frcp(frsq), mod as k - cols *
/// floor(k / cols), mix as t * (b - a) + a). `marker` is the texture the two
/// taps read (the level's). Returns the four channels' bits.
pub fn vol_sample(state: [u32; 4], layers: u32, c: [u32; 3], marker: u32) -> [u32; 4] {
    let [x, y, z] = c.map(f32::from_bits);
    let layers = f32::from_bits(layers);
    let cols = (1.0 / (1.0 / (layers + -0.5).sqrt())).ceil();
    let rcols = 1.0 / cols;
    let rows = (layers * rcols).ceil();
    let rrows = 1.0 / rows;
    let size = tex_size_2d(state, 0);
    let clampto = |t: f32, whole: u32, r: f32| {
        let sz = whole as i32 as f32 * r;
        let lo = 0.5 * (1.0 / sz);
        t.max(lo).min(1.0 + -lo)
    };
    let (su, sv) = (clampto(x, size[0], rcols), clampto(y, size[1], rrows));
    let uv = |k: f32| {
        let row = (k * rcols).floor();
        let col = -cols * row + k;
        (((col + su) * rcols).to_bits(), ((row + sv) * rrows).to_bits())
    };
    let last = layers + -1.0;
    let zz = (z * layers + -0.5).max(0.0).min(last);
    let z0 = zz.floor();
    let z1 = (z0 + 1.0).min(last);
    let t = zz + -z0;
    let (a, b) = (uv(z0), uv(z1));
    [0, 1, 2, 3].map(|k| {
        let pa = f32::from_bits(sample_model(marker, a.0, a.1, k));
        let pb = f32::from_bits(sample_model(marker, b.0, b.1, k));
        (t * (pb + -pa) + pa).to_bits()
    })
}

/// A 3D texture's slice grid in integers, as glsl2usc's pvr_volSize and
/// pvr_volFetch compute it: (cols, one slice's width and height,
/// int(layers)), the square root as the byte model computes the program's.
pub fn vol_grid(state: [u32; 4], layers: u32) -> (i32, [i32; 2], i32) {
    let layers = f32::from_bits(layers);
    let cols = (1.0 / (1.0 / (layers + -0.5).sqrt())).ceil() as i32;
    let n = layers as i32;
    let div = |x: i32, y: i32| x.checked_div(y).unwrap_or(0);
    let rows = div(n + cols - 1, cols);
    let [w, h] = tex_size_2d(state, 0);
    (cols, [div(w as i32, cols), div(h as i32, rows)], n)
}

/// The value of a stacked texture's `pvr_layers_<sampler>` in the model:
/// a whole number of layers from 2 to 6, as the GL core writes it (never
/// the random bits another uniform gets, which would leave every array and
/// 3D look-up undefined and the comparison blind).
pub fn layer_count(seed: u32, name: &str) -> u32 {
    let h = name.bytes().fold(seed.wrapping_mul(0x9e37_79b9), |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193));
    ((h % 5 + 2) as f32).to_bits()
}

/// The texture of the model a sample at level `lod` (a float's bits) reads:
/// `marker` itself at level 0 and another one per whole level above it,
/// shared by the byte model and the IR interpreter so the two agree.
pub fn marker_at_level(marker: u32, lod: u32) -> u32 {
    let lod = f32::from_bits(lod);
    let level = if lod.is_finite() { lod.max(0.0).floor() as u32 } else { u32::MAX };
    if level == 0 {
        marker
    } else {
        marker ^ level.wrapping_mul(0x85eb_ca6b)
    }
}

/// The value a sample returns in this model: channel `k` of texture
/// `marker` at `(u, v)`, a float in 0..1.
///
/// A 64x64 texture sampled at the nearest texel, repeating: the value
/// depends on which texel the coordinate falls in, not on its bits. It used
/// to hash the coordinate's bits, which made two programs that compute the
/// same coordinate one rounding apart (`sqrt` as `frcp(frsq)`, an `atan`
/// polynomial associated differently) read unrelated values -- a difference
/// in the instrument, not in either program. A non-finite coordinate reads
/// a texel of its own.
pub fn sample_model(marker: u32, u: u32, v: u32, k: u32) -> u32 {
    sample_model_at(marker, u, v, [0, 0], k)
}

/// [`sample_model`] `offset` texels away (`textureOffset`): the offset is
/// added to the texel the coordinate falls in, exactly, as the hardware's
/// integer lookup offset is.
pub fn sample_model_at(marker: u32, u: u32, v: u32, offset: [i32; 2], k: u32) -> u32 {
    const SIZE: f32 = 64.0;
    let texel = |x: u32, d: i32| -> u32 {
        let x = f32::from_bits(x);
        if x.is_finite() {
            ((x * SIZE).floor() as i64 + i64::from(d)).rem_euclid(SIZE as i64) as u32
        } else {
            u32::MAX
        }
    };
    let mut h = marker ^ 0x9e37_79b9;
    for x in [texel(u, offset[0]), texel(v, offset[1]), k] {
        h = (h ^ x).wrapping_mul(0x0100_0193).rotate_left(13);
    }
    ((h % 1000) as f32 / 1000.0).to_bits()
}
