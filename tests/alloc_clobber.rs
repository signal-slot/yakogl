// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! No register the GC7000 allocator hands out is overwritten while the value
//! in it is still needed -- checked on the allocated program, against a
//! liveness this file computes for itself.
//!
//! The allocator gives each value one interval and colours the intervals. The
//! intervals were first mention to last, which is a bound only while every
//! branch points forward: on `filter_image.frag` (femtovg's blur) the loop
//! compared its counter against a `sampleCount` whose register the body had
//! already overwritten with `color_sum`, and scaled its taps by a
//! `gaussian_coeff` overwritten by a copy of the counter, from the second trip
//! on. Nothing failed: the program assembled, and it would have drawn.
//!
//! So this does not trust the intervals. It rebuilds the control flow from
//! the branch targets, solves per-LANE liveness to a fixpoint (a lane is the
//! unit the hardware writes), recovers which physical register each value was
//! given from the built instructions, and reports every instruction that
//! writes a physical lane holding some OTHER value that is live after it.
//!
//! Io registers are values too: an input is live from entry to its last
//! read, and an output register must hold what was written to it at the
//! program's end, so both are in the same check.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use std::collections::{BTreeMap, BTreeSet};
use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::vivante::alloc::{self, Cond, SelDst, SelInstr, SelSrc, Stage};
use yakogl::shaderc::online::vivante::isa::{Instr, MAX_FIELDS};
use yakogl::shaderc::online::{lower, select, sema};

/// A value the check follows: a virtual register, an io input register, or a
/// fixed (output) register.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Val {
    V(u32),
    In(u8),
    Out(u8),
}

/// The lanes of a source that instruction `s` consumes, given the lanes `w`
/// its encoded destination writes. Written out here from the machine's rules
/// rather than borrowed from the allocator, so the two can disagree.
fn consumed(s: &SelInstr, swizzle: [u8; 4], w: u8) -> u8 {
    let all = swizzle.iter().fold(0u8, |m, c| m | 1 << (c & 3));
    if s.tex.is_some() || s.target.is_some() || s.dst.is_none() || matches!(s.dst, Some(SelDst::Address { .. })) {
        return all;
    }
    if s.op.reads_one_component() {
        return 1 << (swizzle[0] & 3);
    }
    let lanewise = [
        "mov", "add", "mul", "mad", "select", "cmp", "sign", "floor", "ceil", "frc", "dsx", "dsy", "and", "or", "xor",
        "not", "lshift", "rshift", "f2i", "i2f",
    ]
    .contains(&s.op.op().name);
    if !lanewise {
        return all;
    }
    (0..4).filter(|c| w & (1 << c) != 0).fold(0u8, |m, c| m | 1 << (swizzle[c] & 3))
}

/// Does the write at `s` replace the old contents of its lanes on every run?
fn unguarded(s: &SelInstr) -> bool {
    matches!(s.cond, None | Some(Cond::True)) || matches!(s.op.op().name, "select" | "cmp")
}

/// Every clobber in one allocated program, as readable lines.
fn clobbers(sel: &[SelInstr], out: &[Instr]) -> Vec<String> {
    let n = sel.len();
    // The physical register of every value, from the built instructions.
    let mut phys: BTreeMap<Val, u16> = BTreeMap::new();
    let mut note = |v: Val, r: u16, problems: &mut Vec<String>| {
        if let Some(old) = phys.insert(v, r) {
            if old != r {
                problems.push(format!("{v:?} is in t{old} and t{r}"));
            }
        }
    };
    let mut problems = Vec::new();
    // What each instruction writes: (value, register, lanes).
    let mut writes: Vec<Option<(Val, u16, u8)>> = vec![None; n];
    let mut uses: Vec<Vec<(Val, u8)>> = vec![Vec::new(); n];
    for i in 0..n {
        let s = &sel[i];
        let d = out[i].dst();
        let w = d.map_or(0, |d| d.write_mask);
        match s.dst {
            Some(SelDst::Virtual { v, .. }) => {
                let d = d.expect("a virtual destination was built");
                note(Val::V(v.0), d.reg as u16, &mut problems);
                writes[i] = Some((Val::V(v.0), d.reg as u16, w));
            }
            Some(SelDst::Fixed { reg, .. }) => {
                /* The ENCODED register: the allocator may place an output
                 * somewhere other than where selection wrote it. */
                let d = d.expect("an output destination was built");
                note(Val::Out(reg), d.reg as u16, &mut problems);
                writes[i] = Some((Val::Out(reg), d.reg as u16, w));
            }
            _ => {}
        }
        for (k, src) in s.srcs.iter().enumerate() {
            let (val, swizzle) = match *src {
                Some(SelSrc::Virtual { v, swizzle, .. }) => (Val::V(v.0), swizzle),
                Some(SelSrc::Input { reg, swizzle, .. }) => (Val::In(reg), swizzle),
                _ => continue,
            };
            let built = out[i].src(k).expect("a register source was built");
            note(val, built.reg, &mut problems);
            /* The ENCODED swizzle, not selection's: the allocator may move a
             * single-lane value to another lane and rewrite every reader, and
             * the hardware reads what was encoded. */
            let _ = swizzle;
            uses[i].push((val, consumed(s, built.swizzle, w)));
        }
    }
    let exit_live: BTreeSet<(Val, u8)> = writes
        .iter()
        .flatten()
        .filter(|(v, _, _)| matches!(v, Val::Out(_)))
        .flat_map(|(v, _, w)| (0..4u8).filter(move |c| w & (1 << c) != 0).map(move |c| (*v, c)))
        .collect();
    let succ = |i: usize| -> Vec<usize> {
        let s = &sel[i];
        let mut v = Vec::new();
        if s.target.is_none() || !matches!(s.cond, None | Some(Cond::True)) {
            v.push(i + 1);
        }
        if let Some(t) = s.target {
            v.push(t as usize);
        }
        v
    };
    let mut live_in: Vec<BTreeSet<(Val, u8)>> = vec![BTreeSet::new(); n];
    let mut live_out: Vec<BTreeSet<(Val, u8)>> = vec![BTreeSet::new(); n];
    loop {
        let mut changed = false;
        for i in (0..n).rev() {
            let mut o = BTreeSet::new();
            for j in succ(i) {
                if j >= n {
                    o.extend(exit_live.iter().copied());
                } else {
                    o.extend(live_in[j].iter().copied());
                }
            }
            let mut inn = o.clone();
            if let Some((v, _, w)) = writes[i] {
                if unguarded(&sel[i]) {
                    for c in 0..4u8 {
                        if w & (1 << c) != 0 {
                            inn.remove(&(v, c));
                        }
                    }
                }
            }
            for &(v, lanes) in &uses[i] {
                for c in 0..4u8 {
                    if lanes & (1 << c) != 0 {
                        inn.insert((v, c));
                    }
                }
            }
            if o != live_out[i] {
                live_out[i] = o;
                changed = true;
            }
            if inn != live_in[i] {
                live_in[i] = inn;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    /* A lane no write can reach holds nothing a program could have meant:
     * selection reads a scalar through `.xyzw`, and the if-conversion saves a
     * slot before any store to it (e439b73), reading a register nobody wrote.
     * Losing such a lane changes no result, so a live lane is followed only
     * where some write may have reached it -- forward, to a fixpoint, over the
     * same edges. Inputs are written by the hardware before the program runs. */
    let mut defined: Vec<BTreeSet<(Val, u8)>> = vec![BTreeSet::new(); n];
    loop {
        let mut changed = false;
        for i in 0..n {
            let mut d = defined[i].clone();
            if let Some((v, _, w)) = writes[i] {
                d.extend((0..4u8).filter(|c| w & (1 << c) != 0).map(|c| (v, c)));
            }
            for j in succ(i) {
                if j < n && !d.is_subset(&defined[j]) {
                    defined[j].extend(d.iter().copied());
                    changed = true;
                }
            }
            if d != defined[i] && writes[i].is_none() {
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    for i in 0..n {
        let Some((dv, reg, w)) = writes[i] else { continue };
        for &(v, c) in &live_out[i] {
            /* Defined on entry to i, or by i itself -- and i writes dv, not v. */
            let meaningful = matches!(v, Val::In(_)) || defined[i].contains(&(v, c));
            if meaningful && v != dv && w & (1 << c) != 0 && phys.get(&v) == Some(&reg) {
                problems.push(format!("instruction {i} writes t{reg} lane {c} ({dv:?}) while {v:?} is live in it"));
            }
        }
    }
    problems
}

/// The product's passes and allocation, then the check. `defer` and `place`
/// (outputs placed by the allocator) are off only for the control below.
fn check(name: &str, stage: sema::Stage, src: &str, defer: bool, place: bool) -> Option<(Vec<String>, bool)> {
    let mut mem = vec![0u8; 8 << 20];
    let a = Arena::new(&mut mem);
    let src = a.alloc_str(src).ok()?;
    let l = lower::compile_to_ir(&a, stage, src).ok()?;
    let sel = select::select(&l.module, &a).ok()?;
    if sel.instrs.is_empty() {
        /* Nothing to allocate: every output already sits in its input's
         * register. Counted by the caller, not dropped. */
        return Some((Vec::new(), false));
    }
    let mut code: Vec<SelInstr> = vec![sel.instrs[0]; sel.instrs.len() + alloc::MAX_DEFERRED_OUTPUTS];
    let len = if defer {
        alloc::defer_io_hazards(sel.instrs, &mut code).ok()?
    } else {
        code[..sel.instrs.len()].copy_from_slice(sel.instrs);
        sel.instrs.len()
    };
    code.truncate(len);
    let mut back_edges = sel.back_edges.to_vec();
    let dead = alloc::eliminate_dead(
        &yakogl::shaderc::online::arena::Arena::new(&mut vec![0u8; 1 << 20]),
        &mut code,
        &mut back_edges,
    )
    .unwrap();
    code.truncate(code.len() - dead);
    alloc::sink_definitions(&yakogl::shaderc::online::arena::Arena::new(&mut vec![0u8; 1 << 20]), &mut code).unwrap();
    let removed =
        alloc::coalesce_copies(&yakogl::shaderc::online::arena::Arena::new(&mut vec![0u8; 1 << 20]), &mut code)
            .unwrap();
    code.truncate(code.len() - removed);
    /* The product's call (codegen.rs): inputs as values that die at their
     * last read, every output an instruction writes placed like a value, the
     * rest reserved. */
    let outputs: Vec<u8> = sel
        .io_out
        .iter()
        .map(|io| io.reg)
        .chain(sel.vs_pos_out_reg)
        .chain(sel.vs_pointsize_out_reg)
        .chain(sel.ps_color_out_reg)
        .collect();
    let inputs: Vec<u8> = sel.io_in.iter().map(|io| io.reg).collect();
    let st = if stage == sema::Stage::Fragment { Stage::Fragment } else { Stage::Vertex };
    let mut out = vec![Instr { op: 0, vals: [0; MAX_FIELDS], dont_care: 0 }; code.len()];
    let writes = |r: u8| place && code.iter().any(|s| matches!(s.dst, Some(SelDst::Fixed { reg, .. }) if reg == r));
    let placeable: Vec<u8> = outputs.iter().copied().filter(|r| writes(*r)).collect();
    let fixed: Vec<u8> = outputs.iter().copied().filter(|r| !writes(*r)).collect();
    let mut placed = [alloc::NOT_PLACED; alloc::MAX_TEMPS as usize];
    alloc::allocate_and_build_placed(
        &yakogl::shaderc::online::arena::Arena::new(&mut vec![0u8; 1 << 20]),
        &code,
        &mut out,
        st,
        fixed,
        inputs,
        &placeable,
        &mut placed,
    )
    .ok()?;
    if std::env::var("CLOB_DUMP").is_ok_and(|x| x == name) {
        for (i, s) in code.iter().enumerate() {
            let mut t = String::new();
            yakogl::shaderc::online::vivante::listing::instr_text(&mut t, i, &out[i]).unwrap();
            let d = match s.dst {
                Some(SelDst::Virtual { v, write_mask }) => format!("v{}.{:x}", v.0, write_mask),
                o => format!("{o:?}"),
            };
            let ss: Vec<String> = s
                .srcs
                .iter()
                .flatten()
                .map(|x| match x {
                    SelSrc::Virtual { v, swizzle, .. } => format!("v{}{:?}", v.0, swizzle),
                    o => format!("{o:?}").chars().take(30).collect(),
                })
                .collect();
            println!("{:<90} ;; {d} <- {}", t.trim_end(), ss.join(", "));
        }
    }
    let looped = code.iter().enumerate().any(|(i, s)| s.target.is_some_and(|t| t as usize <= i));
    Some((clobbers(&code, &out), looped))
}

/// A loop whose body reads a value made before it, then makes a value of its
/// own: the shape that lost the first value's register on main.
const LOOP_READS_OUTER_VALUE: &str = "precision mediump float;
uniform vec4 u;
void main() {
    vec4 k = u * 2.0;
    float s = 0.0;
    for (float i = 0.0; i < 4.0; i += 1.0) { s += k.x; float t = s * 3.0; s = t + 1.0; }
    gl_FragColor = vec4(s);
}
";

/// A loop bound computed before the loop and only compared at its top, the
/// `sampleCount` of `filter_image.frag` reduced to itself.
const LOOP_BOUND_BEFORE_LOOP: &str = "precision mediump float;
uniform vec4 u;
void main() {
    float n = ceil(u.x * 3.0);
    vec4 acc = vec4(0.0);
    for (float i = 0.0; i < 24.0; i += 1.0) {
        if (i >= n) { break; }
        acc += u * i;
        acc = acc * 0.5;
    }
    gl_FragColor = acc;
}
";

#[test]
fn no_allocated_register_is_overwritten_while_its_value_is_live() {
    let mut checked = 0usize;
    let mut looped = 0usize;
    let mut bad: Vec<(String, Vec<String>)> = Vec::new();

    for (name, stage, text, loops) in [
        ("LOOP_READS_OUTER_VALUE", sema::Stage::Fragment, LOOP_READS_OUTER_VALUE, true),
        ("LOOP_BOUND_BEFORE_LOOP", sema::Stage::Fragment, LOOP_BOUND_BEFORE_LOOP, true),
        ("es100/loopcarry.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/loopcarry.frag"), true),
        ("es100/ioswap.vert", sema::Stage::Vertex, include_str!("../tools/shaders/es100/ioswap.vert"), false),
        ("es100/deadloop.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/deadloop.frag"), true),
        /* four loops, each with a `continue`: a back edge per continue */
        ("es100/contloops.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/contloops.frag"), true),
    ] {
        let (p, l) = check(name, stage, text, false, true)
            .unwrap_or_else(|| panic!("{name} must compile: it is the shape under test"));
        assert_eq!(l, loops, "{name}: a back edge after selection is what this shape is for, or is not");
        checked += 1;
        looped += l as usize;
        if !p.is_empty() {
            bad.push((name.to_string(), p));
        }
    }
    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { sema::Stage::Vertex } else { sema::Stage::Fragment };
        let Some((p, l)) = check(e.name, stage, e.text.trim_end_matches('\0'), false, true) else { continue };
        checked += 1;
        looped += l as usize;
        if !p.is_empty() {
            bad.push((e.name.to_string(), p));
        }
    }

    println!("checked {checked} programs, {looped} with a back edge, {} with a clobber", bad.len());
    for (n, p) in &bad {
        println!("  {n}: {} clobbers, first: {}", p.len(), p[0]);
    }
    /* The population, so that a harness that stopped compiling anything cannot
     * pass: 139 corpus sources got through selection (7 of them with no
     * instruction at all), two of them loop, plus the five shapes above (four
     * loop) -- 145 and 7. When coverage grows these move: re-derive the pin, do
     * not widen it. Re-derived 2026-09-23 when the front end began to accept
     * `#version 300 es` (ES3 step 1): 46 GLSL ES 3.00 sources joined, one of
     * them with a loop, and none of them clobbers. Re-derived again the same
     * day for ES3 step 2a (`uint`, `uvec` and the integer operators): 8 more
     * ES 3.00 sources get through, none of them loops and none clobbers.
     * Re-derived for ES3 step 3a (the ES 3.00 samplers, look-ups and matrix
     * functions): 44 more ES 3.00 sources, still 8 loops, none clobbers.
     * Re-derived on merging main 2843adc (gc-backend's allocator
     * restructure and ES 3.00 integer selection): 15 more ES 3.00 sources
     * reach the check, one of them loops, none clobbers. Re-derived for ES3
     * step 4 (`switch`, lowered as a loop that runs once):
     * qt3d/skyboxcube_runtime_tonemapping_300.frag, the one table source
     * with a switch, now reaches the check with that loop, and does not
     * clobber. Re-derived for the cube level/bias arm (f3eeea0): 18 more
     * Quick 3D sources sample a cube with textureLod and now select, two of
     * them with a loop; none clobbers. */
    /* Per build: a report-es2 build (the i.MX8MP's dEQP-GLES2 image) refuses
     * GLSL ES 3.00 in the front end (pp.rs out_of_scope), so the population
     * is the one before the ES 3.00 front end; both measured 2026-09-24.
     * Draw buffers (MRT step 1: colour outputs at locations 1..3 select)
     * add 19 sources to the ES 3.00 build: clear_mrt_300.frag,
     * lightmapuvraster_300.frag, 0b4df72d7cd2d1fd.frag and 16
     * qtparticleshaders_*.frag; none loops and none clobbers. */
    let es2 = cfg!(feature = "report-es2");
    /* +6 in both builds (2026-10-02): the six FemtoVG NO_SCISSOR programs
     * (tools/shaders/femtovg/<name>.nodiscard.noscissor.frag, appended to the
     * GC7000 table); none loops, none clobbers. */
    /* +10 in the ES 3.00 build (2026-10-04): the GC7000 now selects
     * gl_VertexID, texelFetch / textureSize / 3D and array look-ups, dsx and
     * dsy (SRC2) and `%` (builtins::imod), so more table sources get
     * through; one more loops, none clobbers. */
    assert_eq!(checked, if es2 { 151 } else { 316 }, "the population moved: {checked} programs reached the check");
    assert_eq!(looped, if es2 { 7 } else { 13 }, "the loop population moved: {looped} programs with a back edge");
    assert!(bad.is_empty(), "{} programs overwrite a live register", bad.len());
}

/// dEQP invariance.*.loop_2's first program, reduced: gl_Position written in
/// ONE arm of a loop and nowhere else. The output's interval began at that
/// write, so the loop's top -- which the next trip runs again after the write
/// -- took its register for a copy of `value`, and the program ended with
/// that copy as the position (full width in this program, `.xyz` in the
/// other one of the pair: the two positions differed in w). The output is
/// live from its write to the end on every path, round the back edge too.
#[test]
fn an_output_written_only_in_a_loop_arm_keeps_its_register_to_the_end() {
    let src = "attribute highp vec4 a_input;
varying mediump vec4 v_unrelated;
invariant gl_Position;
void main ()
{
    highp vec4 value = a_input;
    v_unrelated = vec4(0.0, 0.0, -1.0, 1.0);
    for (mediump int i = 0; i < 14; ++i)
    {
        value *= 1.9;
        if (i == 11)
            gl_Position = a_input + 0.05 * vec4(fract(value.xyz / 1.0e2), 1.0);
        else
            v_unrelated = value + a_input;
    }
}
";
    let (p, looped) = check("loop_2", sema::Stage::Vertex, src, false, true).expect("loop_2 compiles");
    assert!(looped, "the shape under test is a loop");
    assert!(p.is_empty(), "a live output register is overwritten: {p:?}");
}

/// The control: the check can see a clobber at all. On the fixed io layout
/// (outputs where selection wrote them) and without `defer_io_hazards`,
/// `es100/ioswap.vert` writes its varying over the position attribute, and a
/// check that reported nothing here would be reporting nothing everywhere.
#[test]
fn the_check_sees_the_io_clobber_the_pass_prevents() {
    let src = include_str!("../tools/shaders/es100/ioswap.vert");
    let (p, _) = check("ioswap", sema::Stage::Vertex, src, false, false).expect("ioswap.vert compiles");
    assert!(
        p.iter().any(|x| x.contains("Out(1)") && x.contains("In(1)")),
        "without the pass the varying write to t1 should destroy the position in t1; the check saw: {p:?}"
    );
}
