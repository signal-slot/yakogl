// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Moving a value to another lane changes no result -- checked by RUNNING
//! the two allocations of every program and comparing what they output.
//!
//! The allocator may place a single-lane value in any lane of a register
//! (`alloc::allocate_and_build`), and when it does it rewrites the defining
//! instruction's write mask and source swizzle positions and every reader's
//! swizzle. The clobber checker (`alloc_clobber.rs`) proves no live register
//! is overwritten; it cannot see a swizzle that reads the right register in
//! the wrong lane. This can: a small interpreter of the ENCODED instructions,
//! faithful to lanes (swizzles, write masks, scalar splats, dot-product sums,
//! select/cmp, branches), runs each program allocated with and without lane
//! moves on the same random inputs, and every output register must come out
//! bit for bit the same.
//!
//! The interpreter is exact about lanes and only approximate about some
//! operations' arithmetic (both arms go through the same approximation, and
//! the question is whether they agree, not whether either is the hardware).
//! A program with an operation it does not model, relative addressing or a
//! texture fetch is counted as skipped, never as a pass.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::vivante::alloc::{self, SelInstr, Stage};
use yakogl::shaderc::online::vivante::isa::{Instr, MAX_FIELDS};
use yakogl::shaderc::online::{lower, select, sema};

mod common;
use common::{run_mem, Rng, Run};

struct Program {
    code: Vec<SelInstr>,
    stage: Stage,
    reserved: Vec<u8>,
    io_in: Vec<u8>,
    inputs: Vec<u8>,
    outputs: Vec<(u8, u8)>,
    /// Per constant-file dword: it holds an integer or bool uniform.
    int_consts: Vec<bool>,
    /// Per constant-file dword: the uniform block's address, for `load`.
    ubo_consts: Vec<bool>,
}

/// Where the uniform block a `load` reads sits (select.rs LoadUniformIdx),
/// and its size in dwords: random, like every other uniform here, and large
/// enough for any clamped index of any array a program declares.
const UBO_BASE: u32 = 0x4000_0000;
const UBO_DWORDS: usize = 4096;

/// GL's integer and bool uniform types: an int, uint or bool and their
/// vectors.
fn is_int_type(t: u16) -> bool {
    matches!(t, 0x1404 | 0x1405 | 0x8b53..=0x8b59 | 0x8dc6..=0x8dc8)
}

/// Random uniforms, one register per entry. A float uniform gets a float in
/// [-2, 2]; an integer or bool one a small integer, 0..=3, because it may
/// be a loop count or an index (es300/dynidx.frag runs `i < n` over one):
/// float bits read as an int are about 10^9 and the program cannot finish.
/// The same draws either way, so every other program's inputs are
/// unchanged by the distinction.
fn uniforms_for(p: &Program, rng: &mut Rng) -> Vec<[u32; 4]> {
    let mut u: Vec<[u32; 4]> = (0..512).map(|_| core::array::from_fn(|_| rng.float().to_bits())).collect();
    for (k, &int) in p.int_consts.iter().enumerate() {
        if int {
            if let Some(r) = u.get_mut(k / 4) {
                r[k % 4] %= 4;
            }
        }
    }
    for (k, &ubo) in p.ubo_consts.iter().enumerate() {
        if ubo {
            if let Some(r) = u.get_mut(k / 4) {
                r[k % 4] = UBO_BASE;
            }
        }
    }
    u
}

/// The block a `load` reads, random.
fn ubo_block(rng: &mut Rng) -> Vec<u32> {
    (0..UBO_DWORDS).map(|_| rng.float().to_bits()).collect()
}

fn prepare(stage: sema::Stage, src: &str) -> Option<Program> {
    let mut mem = vec![0u8; 8 << 20];
    let a = Arena::new(&mut mem);
    let src = a.alloc_str(src).ok()?;
    let l = lower::compile_to_ir(&a, stage, src).ok()?;
    let sel = select::select(&l.module, &a).ok()?;
    if sel.instrs.is_empty() {
        return None;
    }
    let mut code: Vec<SelInstr> = vec![sel.instrs[0]; sel.instrs.len() + alloc::MAX_DEFERRED_OUTPUTS];
    /* No defer_io_hazards: codegen places the outputs instead. */
    code[..sel.instrs.len()].copy_from_slice(sel.instrs);
    code.truncate(sel.instrs.len());
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
    /* Each output with the lanes that mean something: a varying's declared
     * width (a vec3's w is never read, and a copy into the output register
     * fills it with whatever the source register held there), position and
     * colour four, point size one. */
    let outputs: Vec<(u8, u8)> = sel
        .io_out
        .iter()
        .map(|io| (io.reg, (1u8 << io.components.min(4)) - 1))
        .chain(sel.vs_pos_out_reg.map(|r| (r, 0xf)))
        .chain(sel.vs_pointsize_out_reg.map(|r| (r, 0x1)))
        .chain(sel.ps_color_out_reg.map(|r| (r, 0xf)))
        .collect();
    let mut inputs: Vec<u8> = sel.io_in.iter().map(|io| io.reg).collect();
    if stage == sema::Stage::Fragment {
        inputs.push(0);
    }
    let reserved: Vec<u8> = outputs.iter().map(|o| o.0).collect();
    let io_in: Vec<u8> = sel.io_in.iter().map(|io| io.reg).collect();
    let st = if stage == sema::Stage::Fragment { Stage::Fragment } else { Stage::Vertex };
    let mut int_dwords = vec![false; l.module.uniform_dwords as usize];
    for u in l.module.uniforms.iter().filter(|u| is_int_type(u.gl_type)) {
        let (a, b) = (u.dword_off as usize, u.dword_off as usize + u.dwords as usize);
        for d in int_dwords.iter_mut().take(b).skip(a) {
            *d = true;
        }
    }
    let int_consts = sel
        .consts
        .iter()
        .map(|c| c.contents == yakogl::shaderc::SHC_VIV_CONST_UNIFORM && int_dwords.get(c.data as usize) == Some(&true))
        .collect();
    let ubo_consts = sel.consts.iter().map(|c| c.contents == yakogl::shaderc::SHC_VIV_CONST_UBO_ADDR).collect();
    Some(Program { code, stage: st, reserved, io_in, inputs, outputs, int_consts, ubo_consts })
}

/// The product's call (codegen.rs); the outputs come back as the registers
/// they were PLACED in, which the two arms may choose differently.
/// An allocation as the interpreter runs it: the code, each output as (the
/// register it was placed in, its meaningful lanes), and the lane moves.
type Allocation = (Vec<Instr>, Vec<(u8, u8)>, usize);

fn allocate(p: &Program, moves: bool) -> Option<Allocation> {
    alloc::set_lane_moves_disabled(!moves);
    let mut out = vec![Instr { op: 0, vals: [0; MAX_FIELDS], dont_care: 0 }; p.code.len()];
    let writes = |r: u8| p.code.iter().any(|s| matches!(s.dst, Some(alloc::SelDst::Fixed { reg, .. }) if reg == r));
    let placeable: Vec<u8> = p.reserved.iter().copied().filter(|r| writes(*r)).collect();
    let fixed: Vec<u8> = p.reserved.iter().copied().filter(|r| !writes(*r)).collect();
    let mut placed = [alloc::NOT_PLACED; alloc::MAX_TEMPS as usize];
    let r = alloc::allocate_and_build_placed(
        &yakogl::shaderc::online::arena::Arena::new(&mut vec![0u8; 1 << 20]),
        &p.code,
        &mut out,
        p.stage,
        fixed,
        p.io_in.iter().copied(),
        &placeable,
        &mut placed,
    );
    let n = alloc::last_lane_moves();
    alloc::set_lane_moves_disabled(false);
    let outs = p
        .outputs
        .iter()
        .map(|&(reg, lanes)| {
            (placed.get(reg as usize).copied().filter(|x| *x != alloc::NOT_PLACED).unwrap_or(reg), lanes)
        })
        .collect();
    r.ok().map(|_| (out, outs, n))
}
#[test]
fn moving_a_value_to_another_lane_changes_no_output() {
    let mut sources: Vec<(String, sema::Stage, String)> = SHC_SOURCE_TABLE
        .iter()
        .map(|e| {
            let st = if e.name.contains(".vert") { sema::Stage::Vertex } else { sema::Stage::Fragment };
            (e.name.to_string(), st, e.text.trim_end_matches('\0').to_string())
        })
        .collect();
    for (n, st, t) in [
        ("es100/ioswap.vert", sema::Stage::Vertex, include_str!("../tools/shaders/es100/ioswap.vert")),
        ("es100/loopcarry.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/loopcarry.frag")),
        ("es100/deadloop.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/deadloop.frag")),
        ("es100/branchlane.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/branchlane.frag")),
        ("es100/ifloop.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/ifloop.frag")),
        ("es100/contloops.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/contloops.frag")),
        ("es100/branchquad.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/branchquad.frag")),
    ] {
        sources.push((n.to_string(), st, t.to_string()));
    }

    let (mut compared, mut with_moves, mut skipped) = (0usize, 0usize, 0usize);
    let mut skip_reasons: std::collections::BTreeMap<String, usize> = Default::default();
    let mut bad = Vec::new();
    for (name, stage, text) in &sources {
        let Some(p) = prepare(*stage, text) else { continue };
        let (Some((off, outs_off, _)), Some((on, outs_on, moves))) = (allocate(&p, false), allocate(&p, true)) else {
            continue;
        };
        if moves == 0 {
            continue;
        }
        if std::env::var("EQ_DUMP").is_ok_and(|x| &x == name) {
            println!("{text}");
            for (arm, code) in [("OFF", &off), ("ON ", &on)] {
                for (i, ins) in code.iter().enumerate() {
                    let mut t = String::new();
                    yakogl::shaderc::online::vivante::listing::instr_text(&mut t, i, ins).unwrap();
                    println!("{arm} {}", t.trim_end());
                }
            }
            println!("outputs {:?} inputs {:?}", p.outputs, p.inputs);
        }
        with_moves += 1;
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15 ^ name.len() as u64);
        let mut ok = true;
        let mut ran = false;
        for trial in 0..4 {
            let uniforms = uniforms_for(&p, &mut rng);
            let mut init = vec![[0u32; 4]; 128];
            for &r in &p.inputs {
                init[r as usize] = core::array::from_fn(|_| rng.float().to_bits());
            }
            let block = ubo_block(&mut rng);
            let mem = [(UBO_BASE, &block[..])];
            match (run_mem(&off, &init, &uniforms, &mem), run_mem(&on, &init, &uniforms, &mem)) {
                (Run::Done { regs: a, killed: ka }, Run::Done { regs: b, killed: kb }) => {
                    ran = true;
                    if ka != kb {
                        bad.push(format!("{name} trial {trial}: discard {ka} without moves, {kb} with"));
                        ok = false;
                    } else if !ka {
                        for (&(o, lanes), &(o2, _)) in outs_off.iter().zip(outs_on.iter()) {
                            let differs =
                                (0..4).any(|l| lanes & (1 << l) != 0 && a[o as usize][l] != b[o2 as usize][l]);
                            if differs {
                                bad.push(format!(
                                    "{name} trial {trial}: output t{o}/t{o2} {:08x?} without moves, {:08x?} with",
                                    a[o as usize], b[o2 as usize]
                                ));
                                ok = false;
                            }
                        }
                    }
                }
                (Run::Unsupported(why), _) | (_, Run::Unsupported(why)) => {
                    *skip_reasons.entry(why).or_default() += 1;
                    break;
                }
            }
            if !ok {
                break;
            }
        }
        if ran {
            compared += 1;
        } else {
            skipped += 1;
        }
    }
    println!("{with_moves} programs move a lane; {compared} compared by running, {skipped} skipped");
    for (why, n) in &skip_reasons {
        println!("  skipped {n}: {why}");
    }
    for b in bad.iter().take(20) {
        println!("  {b}");
    }
    /* The population: the programs that move a lane, every one run (a
     * program the interpreter cannot run is its own outcome, not a pass).
     * Per build: a report-es2 build refuses GLSL ES 3.00 in the front end
     * (pp.rs out_of_scope). Measured 2026-10-24; 249 / 165 while the Qt
     * shaders were in the GC7000 table, until #1509. */
    let es2 = cfg!(feature = "report-es2");
    assert_eq!(
        with_moves,
        if es2 { 87 } else { 103 },
        "the population moved: {with_moves} programs move a lane; re-derive the pin"
    );
    assert_eq!(
        skipped, 0,
        "{skipped} programs with lane moves could not be run: model the operation, do not drop them"
    );
    assert!(bad.is_empty(), "{} runs differ with lane moves", bad.len());
}

/// Deleting a copy of a register onto itself changes no output -- and the
/// branches that jumped over or onto it still land where they should. Every
/// program in which `drop_identity_movs` deletes something is run before and
/// after, on the same inputs.
#[test]
fn dropping_identity_copies_changes_no_output() {
    let (mut with_drops, mut compared, mut skipped) = (0usize, 0usize, 0usize);
    let mut bad = Vec::new();
    for e in SHC_SOURCE_TABLE.iter() {
        let stage = if e.name.contains(".vert") { sema::Stage::Vertex } else { sema::Stage::Fragment };
        let Some(p) = prepare(stage, e.text.trim_end_matches('\0')) else { continue };
        let Some((code, outs, _)) = allocate(&p, true) else { continue };
        let mut dropped = code.clone();
        let mut back = vec![0usize; 0];
        let n = alloc::drop_identity_movs(
            &yakogl::shaderc::online::arena::Arena::new(&mut vec![0u8; 1 << 20]),
            &mut dropped,
            &mut back,
        )
        .unwrap();
        if n == 0 {
            continue;
        }
        dropped.truncate(dropped.len() - n);
        with_drops += 1;
        let mut rng = Rng(0x2545_f491_4f6c_dd1d ^ e.name.len() as u64);
        let mut ran = false;
        for trial in 0..4 {
            let uniforms = uniforms_for(&p, &mut rng);
            let mut init = vec![[0u32; 4]; 128];
            for &r in &p.inputs {
                init[r as usize] = core::array::from_fn(|_| rng.float().to_bits());
            }
            let block = ubo_block(&mut rng);
            let mem = [(UBO_BASE, &block[..])];
            match (run_mem(&code, &init, &uniforms, &mem), run_mem(&dropped, &init, &uniforms, &mem)) {
                (Run::Done { regs: a, killed: ka }, Run::Done { regs: b, killed: kb }) => {
                    ran = true;
                    let same = ka == kb
                        && (ka
                            || outs.iter().all(|&(o, lanes)| {
                                (0..4).all(|l| lanes & (1 << l) == 0 || a[o as usize][l] == b[o as usize][l])
                            }));
                    if !same {
                        bad.push(format!("{} trial {trial}", e.name));
                        break;
                    }
                }
                _ => {
                    skipped += 1;
                    break;
                }
            }
        }
        compared += ran as usize;
    }
    println!("{with_drops} programs drop an identity copy; {compared} compared by running, {skipped} skipped");
    for b in &bad {
        println!("  differs: {b}");
    }
    assert!(with_drops >= 60, "only {with_drops} programs drop an identity copy; the check has lost its population");
    assert_eq!(skipped, 0, "{skipped} programs could not be run");
    assert!(bad.is_empty(), "{} programs differ after dropping identity copies", bad.len());
}
