// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the GC7000 back end emits computes what the IR says -- checked by
//! running both on the same inputs.
//!
//! Four defects found on 2026-09-23 had the same shape: the program
//! assembled, every structural check passed, and it computed something else
//! (a loop value's register reused, an output written over an input still to
//! be read, both arms of an `if` storing the colour, a negated literal
//! encoded as 5.4e-20). Each was found by reading a listing. This finds that
//! class without reading anything: the IR reference evaluator (`eval.rs`, the
//! front end's) runs the module, the lane-faithful interpreter
//! (`tests/common`) runs the ENCODED program the product's passes and
//! allocator produced, on the same attributes, varyings and uniforms, and
//! every output must agree.
//!
//! Agreement is to a relative 1e-4, not bit for bit: the back end lowers
//! `x / y` as `x * rcp(y)`, `pow` through `exp2`/`log2` and so on, which round
//! differently from the evaluator's direct f32 operations. A lane the
//! evaluator itself makes non-finite, or leaves at `eval::UNSET` (a slot read
//! before any store), is not compared: there is no right answer to match.
//!
//! A program either side cannot run -- a texture, a derivative, an indexed
//! local, an operation the interpreter does not model -- is counted as
//! skipped with its reason, never as a pass.

#![cfg(all(feature = "gpu-vivante", feature = "shaderc-online", not(feature = "no-shader-table")))]

mod common;

use common::{run_mem, Rng, Run};
use yakogl::shaderc::mesa_reference::SHC_SOURCE_TABLE;
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::eval;
use yakogl::shaderc::online::ir::{self, IoSlot};
use yakogl::shaderc::online::vivante::alloc::{self, SelDst, SelInstr, Stage};
use yakogl::shaderc::online::vivante::isa::{Instr, MAX_FIELDS};
use yakogl::shaderc::online::{lower, select, sema};

/// One comparison's outcome.
#[derive(Debug)]
enum Verdict {
    Agree,
    Differ(String),
    Skip(String),
}

/// Where the harness puts a program's uniform block for `load` to read: any
/// address the constant file is filled with works, one not near zero shows
/// a missing base as a load outside memory rather than as block dword 0.
const UBO_BASE: u32 = 0x4000_0000;

fn close(a: u32, b: u32) -> bool {
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    if !x.is_finite() || a == eval::UNSET {
        return true;
    }
    if a == b {
        return true;
    }
    let tol = 1e-4 * x.abs().max(1.0);
    (x - y).abs() <= tol
}

fn compare(name: &str, stage: sema::Stage, text: &str, seed: u64) -> Option<Verdict> {
    let mut mem = vec![0u8; 8 << 20];
    let a = Arena::new(&mut mem);
    let src = a.alloc_str(text).ok()?;
    let l = lower::compile_to_ir(&a, stage, src).ok()?;
    compare_module(name, stage, &l.module, &a, seed)
}

/// [`compare`] from the IR on: for a module built by hand, when the
/// operation to check has no front end yet (GLSL ES 3.00's integers).
fn compare_module(name: &str, stage: sema::Stage, m: &ir::Module, a: &Arena, seed: u64) -> Option<Verdict> {
    let sel = select::select(m, a).ok()?;

    /* The product's passes and allocation (codegen.rs). */
    let mut code: Vec<SelInstr> = sel.instrs.to_vec();
    let mut back = sel.back_edges.to_vec();
    if !code.is_empty() {
        let dead = alloc::eliminate_dead(a, &mut code, &mut back).unwrap();
        code.truncate(code.len() - dead);
        alloc::sink_definitions(a, &mut code).unwrap();
        let removed = alloc::coalesce_copies(a, &mut code).unwrap();
        code.truncate(code.len() - removed);
    }
    let outputs: Vec<u8> = sel
        .io_out
        .iter()
        .map(|io| io.reg)
        .chain(sel.vs_pos_out_reg)
        .chain(sel.vs_pointsize_out_reg)
        .chain(sel.ps_color_out_reg)
        .collect();
    let writes = |r: u8| code.iter().any(|s| matches!(s.dst, Some(SelDst::Fixed { reg, .. }) if reg == r));
    let placeable: Vec<u8> = outputs.iter().copied().filter(|r| writes(*r)).collect();
    let fixed: Vec<u8> = outputs.iter().copied().filter(|r| !writes(*r)).collect();
    let st = if stage == sema::Stage::Fragment { Stage::Fragment } else { Stage::Vertex };
    let mut placed = [alloc::NOT_PLACED; alloc::MAX_TEMPS as usize];
    let mut built = vec![Instr { op: 0, vals: [0; MAX_FIELDS], dont_care: 0 }; code.len()];
    let inputs: Vec<u8> = sel.io_in.iter().map(|io| io.reg).collect();
    if !code.is_empty() {
        alloc::allocate_and_build_placed(
            a,
            &code,
            &mut built,
            st,
            fixed,
            inputs.iter().copied(),
            &placeable,
            &mut placed,
        )
        .ok()?;
        let dropped = alloc::drop_identity_movs(a, &mut built, &mut back).unwrap();
        built.truncate(built.len() - dropped);
    }
    let place = |r: u8| placed.get(r as usize).copied().filter(|p| *p != alloc::NOT_PLACED).unwrap_or(r);

    /* Random inputs, the same for both sides. */
    let mut rng = Rng(seed);
    let is_fs = stage == sema::Stage::Fragment;
    let mut io: Vec<(IoSlot, [u32; 4])> = Vec::new();
    let mut regs = vec![[0u32; 4]; 128];
    let base = |n: &str| n.split('[').next().unwrap_or(n).to_string();
    for i in sel.io_in.iter() {
        /* A matrix attribute is ONE row at its first column's register and
         * location; its columns follow in both (GL_FLOAT_MAT2..4). A matrix
         * VARYING arrives already split into one row per column (`m`,
         * `m[1]`, front end 3beded7), each still typed as the matrix: taken
         * as it is, or each column would be fed twice over the next one's
         * register. */
        let split = sel.io_in.iter().filter(|o| base(o.name) == base(i.name)).count() > 1;
        let cols = match i.gl_type {
            _ if split => 1,
            0x8b5a => 2,
            0x8b5b => 3,
            0x8b5c => 4,
            _ => 1,
        };
        for c in 0..cols {
            let v: [u32; 4] = core::array::from_fn(|_| rng.float().to_bits());
            let loc = i.slot + c;
            io.push((if is_fs { IoSlot::Varying(loc) } else { IoSlot::Attrib(loc) }, v));
            regs[(i.reg + c) as usize] = v;
        }
    }
    /* Declared inputs selection dropped as unused still exist for the IR:
     * give the evaluator a value for each (the program never reads them). */
    for d in m.inputs.iter() {
        let slot = match d.kind {
            yakogl::shaderc::online::ir::IoKind::Attribute => IoSlot::Attrib(d.location),
            yakogl::shaderc::online::ir::IoKind::Varying => IoSlot::Varying(d.location),
            _ => continue,
        };
        if !io.iter().any(|(s, _)| *s == slot) {
            io.push((slot, core::array::from_fn(|_| rng.float().to_bits())));
        }
    }
    /* A matrix attribute is one declaration over several locations, one per
     * column; selection lists each column in io_in, the IR reads each by its
     * own location. Any location still missing gets a value too. */
    for k in 0..16u8 {
        let slot = if is_fs { IoSlot::Varying(k) } else { IoSlot::Attrib(k) };
        if !io.iter().any(|(s, _)| *s == slot) {
            io.push((slot, core::array::from_fn(|_| rng.float().to_bits())));
        }
    }
    if is_fs {
        let v: [u32; 4] = core::array::from_fn(|_| rng.float().to_bits());
        io.push((IoSlot::FragCoord, v));
        regs[0] = v;
    }
    let uniform_dwords = (m.uniform_dwords as usize).max(1);
    let mut block: Vec<u32> = (0..uniform_dwords).map(|_| rng.float().to_bits()).collect();
    /* An integer or bool uniform gets a small integer, 0..=3, from the same
     * draw: it may be a loop count or an index (es300/dynidx.frag runs
     * `i < n`), and float bits read as an int are about 10^9 -- a loop that
     * never ends is measured as nothing. Float uniforms are unchanged. */
    for u in m.uniforms.iter().filter(|u| matches!(u.gl_type, 0x1404 | 0x1405 | 0x8b53..=0x8b59 | 0x8dc6..=0x8dc8)) {
        let (a, b) = (u.dword_off as usize, u.dword_off as usize + u.dwords as usize);
        for d in block.iter_mut().take(b).skip(a) {
            *d %= 4;
        }
    }
    /* The constant file as the driver fills it: each dword a uniform-block
     * dword or a compiler literal. */
    let mut cfile = vec![[0u32; 4]; 512];
    for (k, c) in sel.consts.iter().enumerate() {
        /* gl_FragCoord's y transform (select.rs frag_coord) as on a target
         * the backend does not mirror: scale 1, offset 0 -- y' is the IR's y. */
        let v = match c.contents {
            yakogl::shaderc::SHC_VIV_CONST_UNIFORM => block.get(c.data as usize).copied().unwrap_or(0),
            yakogl::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_SCALE => 1.0f32.to_bits(),
            yakogl::shaderc::SHC_VIV_CONST_FRAGCOORD_Y_OFFSET => 0,
            /* gl_PointCoord's t transform as on a mirrored target: identity */
            yakogl::shaderc::SHC_VIV_CONST_PNTC_Y_SCALE => 1.0f32.to_bits(),
            yakogl::shaderc::SHC_VIV_CONST_PNTC_Y_OFFSET => 0,
            /* the stage's uniform block as the driver copies it to memory for
             * a run-time-indexed read (select.rs LoadUniformIdx) */
            yakogl::shaderc::SHC_VIV_CONST_UBO_ADDR => UBO_BASE,
            _ => c.data,
        };
        if let Some(r) = cfile.get_mut(k / 4) {
            r[k % 4] = v;
        }
    }

    /* The reference: the IR evaluator. What it does not implement comes back
     * as an error naming it; that is a skip, not a failure. */
    let inputs_ev = eval::Inputs { io: &io, uniforms: &block };
    let want = match eval::run(m, &inputs_ev) {
        Ok(o) => o,
        Err(why) => return Some(Verdict::Skip(format!("evaluator: {why}"))),
    };

    let mem = [(UBO_BASE, &block[..])];
    let got = if built.is_empty() {
        Run::Done { regs: regs.clone(), killed: false }
    } else {
        run_mem(&built, &regs, &cfile, &mem)
    };
    let (regs_out, killed) = match got {
        Run::Done { regs, killed } => (regs, killed),
        Run::Unsupported(why) => return Some(Verdict::Skip(format!("interpreter: {why}"))),
    };
    if killed != want.discarded {
        return Some(Verdict::Differ(format!("{name}: discard {killed}, the IR says {}", want.discarded)));
    }
    if killed {
        return Some(Verdict::Agree);
    }

    /* Every output the IR stored, against the register it was placed in. */
    let mut rows: Vec<(IoSlot, u8, u8)> = Vec::new();
    for o in sel.io_out.iter() {
        rows.push((IoSlot::Varying(o.slot), place(o.reg), (1u8 << o.components.min(4)) - 1));
    }
    if let Some(r) = sel.vs_pos_out_reg {
        rows.push((IoSlot::Position, place(r), 0xf));
    }
    if let Some(r) = sel.vs_pointsize_out_reg {
        rows.push((IoSlot::PointSize, place(r), 0x1));
    }
    if let Some(r) = sel.ps_color_out_reg {
        rows.push((IoSlot::FragColor(0), place(r), 0xf));
    }
    for (slot, reg, lanes) in rows {
        let Some(mut w) = want.output(slot) else { continue };
        /* The back end clamps gl_PointSize to the reported range, which the
         * IR does not model (select.rs, POINT_SIZE_MIN/MAX). */
        if slot == IoSlot::PointSize {
            w[0] = f32::from_bits(w[0]).clamp(select::POINT_SIZE_MIN, select::POINT_SIZE_MAX).to_bits();
        }
        let g = regs_out[reg as usize];
        for l in 0..4 {
            if lanes & (1 << l) != 0 && !close(w[l], g[l]) {
                return Some(Verdict::Differ(format!(
                    "{name}: {slot:?} lane {l}: the IR says {} ({:#010x}), the program gives {} ({:#010x})",
                    f32::from_bits(w[l]),
                    w[l],
                    f32::from_bits(g[l]),
                    g[l]
                )));
            }
        }
    }
    Some(Verdict::Agree)
}

/// gl_test 108 D and 128 J, whose run-time-indexed read is a `load`: J
/// keeps only `.ba` of the element, and a load narrowed to `.__zw` fills
/// z and w with the element's x and y on silicon (the interpreter packs as
/// the hardware does). 128 read (0x33, 0x66) for (0x99, 0xff) at cc955595.
#[test]
fn a_loaded_element_keeps_its_components_whatever_lanes_are_read() {
    for (name, text) in [
        ("uniidx_d.frag", include_str!("../tools/shaders/es100/uniidx_d.frag")),
        ("uniidx_j.frag", include_str!("../tools/shaders/es100/uniidx_j.frag")),
    ] {
        for seed in 0..4u64 {
            match compare(name, sema::Stage::Fragment, text, 0x1234 + seed) {
                Some(Verdict::Agree) => {}
                other => panic!("{name} seed {seed}: {other:?}"),
            }
        }
    }
}

/// `%` over several divisors at once (see the source list below).
const IMOD_FS: &str = "#version 300 es
precision highp float;
precision highp int;
flat in ivec4 a;
layout(location = 0) out vec4 c;
void main() { c = vec4(a % ivec4(3, 4, 7, 1000)) + vec4(a.yzwx % ivec4(-5)); }
";

#[test]
fn the_back_end_computes_what_the_ir_says() {
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
        ("es100/anyall.vert", sema::Stage::Vertex, include_str!("../tools/shaders/es100/anyall.vert")),
        ("es100/mvstruct.vert", sema::Stage::Vertex, include_str!("../tools/shaders/es100/mvstruct.vert")),
        ("es100/branchquad.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/branchquad.frag")),
        ("es100/ifoutput.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/ifoutput.frag")),
        ("es100/ifoutput2.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/ifoutput2.frag")),
        ("es100/immneg.vert", sema::Stage::Vertex, include_str!("../tools/shaders/es100/immneg.vert")),
        ("es100/immneg.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/immneg.frag")),
        ("es100/idiv.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/idiv.frag")),
        ("es100/matvary.vert", sema::Stage::Vertex, include_str!("../tools/shaders/es100/matvary.vert")),
        ("es100/matvary.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/matvary.frag")),
        ("es100/transc.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/transc.frag")),
        ("es100/vtransc.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/vtransc.frag")),
        ("es100/dynvec4.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/dynvec4.frag")),
        ("es100/varyings15.vert", sema::Stage::Vertex, include_str!("../tools/shaders/es100/varyings15.vert")),
        ("es100/varyings15.frag", sema::Stage::Fragment, include_str!("../tools/shaders/es100/varyings15.frag")),
        /* ES 3.00 `%` on signed integers, both signs: builtins::imod (the
         * machine's own `imod` is not used; tests/encoding_vocabulary.rs).
         * gl_test's es300/vid.vert takes gl_VertexID % 4. */
        ("imod.frag", sema::Stage::Fragment, IMOD_FS),
    ] {
        sources.push((n.to_string(), st, t.to_string()));
    }
    /* An output stored more than once, where one store finds its value
     * already in the output's register (the input and the output share it):
     * leaving that store out is sound only for the output's one store
     * (select.rs single_store). The shapes of dEQP-GLES2
     * fragment_ops.interaction.basic_shader.16, 61, 63 and 84. */
    for (n, st, t) in [
        (
            "twostores.vert",
            sema::Stage::Vertex,
            "attribute vec4 p;\nattribute float e;\nattribute float f;\nvarying mediump float d;\nvoid main() { gl_Position = p; d = f; d = e; }\n",
        ),
        (
            "ifstore.vert",
            sema::Stage::Vertex,
            "attribute vec4 p;\nattribute float e;\nattribute float f;\nvarying mediump float d;\nvoid main() { gl_Position = p; if (e > 0.5) d = f; d = e; }\n",
        ),
        (
            "ifcolour.frag",
            sema::Stage::Fragment,
            "precision mediump float;\nvarying mediump vec4 i;\nvarying mediump float t;\nvoid main() { gl_FragColor = i; if (t > 0.5) gl_FragColor = vec4(1.0, 0.0, 0.0, 1.0); }\n",
        ),
    ] {
        sources.push((n.to_string(), st, t.to_string()));
    }

    let (mut agreed, mut differed) = (0usize, 0usize);
    let mut skips: std::collections::BTreeMap<String, usize> = Default::default();
    let mut bad = Vec::new();
    for (name, stage, text) in &sources {
        let mut verdict = None;
        for trial in 0..4u64 {
            match compare(name, *stage, text, 0x5eed_0000 + trial * 7919 + name.len() as u64) {
                None => break,
                Some(Verdict::Agree) => verdict = Some(true),
                Some(Verdict::Differ(why)) => {
                    bad.push(why);
                    verdict = Some(false);
                    break;
                }
                Some(Verdict::Skip(why)) => {
                    *skips.entry(why).or_default() += 1;
                    verdict = None;
                    break;
                }
            }
        }
        match verdict {
            Some(true) => agreed += 1,
            Some(false) => differed += 1,
            None => {}
        }
    }
    println!("{agreed} programs agree with the IR, {differed} differ");
    for (why, n) in &skips {
        println!("  skipped {n}: {why}");
    }
    for b in bad.iter().take(30) {
        println!("  {b}");
    }
    /* The population, pinned: 221 programs compared, 48 skipped because the
     * evaluator has no textures and 2 because it has no derivatives -- 133 /
     * 25 / 0 on main; the GLSL ES 3.00 front end (steps 1 to 2b, texture
     * IR) lets the table's ES 3.00 sources through, dynidx.frag among them
     * now that the interpreter runs the address register and an int uniform
     * is an int. A change that moves either re-derives the
     * pin; widening it silently is how a check stops checking. Re-derived
     * for ES3 step 4: 51 skipped (49 textures), because
     * qt3d/skyboxcube_runtime_tonemapping_300.frag, the one table source with a `switch`, gets through once ES3 step 4 lowers it -- and it samples a cube map, so it is skipped for
     * textures; the compared population does not move. */
    let skipped: usize = skips.values().sum();
    /* Per build: a report-es2 build (the i.MX8MP's dEQP-GLES2 image) refuses
     * GLSL ES 3.00 in the front end (pp.rs out_of_scope), so the population
     * is the one before the ES 3.00 front end; both measured 2026-09-24.
     * Draw buffers (MRT step 1: colour outputs at locations 1..3 select)
     * add 2 compared sources to the ES 3.00 build, clear_mrt_300.frag and
     * 0b4df72d7cd2d1fd.frag, which agree. */
    /* +3 / +4 in both builds (2026-10-02): the six FemtoVG NO_SCISSOR programs
     * (tools/shaders/femtovg/<name>.nodiscard.noscissor.frag); 3 of them are compared and agree. */
    let es2 = cfg!(feature = "report-es2");
    /* +1 in the ES 3.00 build (2026-10-04): imod.frag, `%` through
     * builtins::imod, agrees (a mutation swapping its select arms differs). */
    assert_eq!(
        agreed + differed,
        if es2 { 141 } else { 232 },
        "the compared population moved ({agreed} agree, {differed} differ)"
    );
    /* +18 with the cube level/bias arm (f3eeea0): the Quick 3D sources it lets
     * through all sample textures, which the evaluator does not model. */
    /* 73 = 67 textures + 2 derivatives + 4 that read gl_FrontFacing, selected
     * since 5000a9a: neither this harness's inputs nor the lane interpreter
     * model the input or the internal register yet -- a named gap, to close. */
    /* +17 skipped in the ES 3.00 build with draw buffers: texture samplers,
     * lightmapuvraster_300.frag and 16 qtparticleshaders_*.frag. */
    /* +3 in both builds (2026-10-02): the three FemtoVG NO_SCISSOR programs
     * that sample a texture (fill_image, fill_image_gradient and its conic). */
    /* +11 skipped in the ES 3.00 build (2026-10-04), the table sources the
     * GC7000 now selects (see above): textures (texelFetch, 3D and array
     * look-ups), derivatives, gl_VertexID and gl_InstanceID, none of which
     * the evaluator models or this harness supplies. */
    assert_eq!(skipped, if es2 { 28 } else { 104 }, "the skipped population moved: {skips:?}");
    assert!(bad.is_empty(), "{} programs compute something other than their IR", bad.len());
}

/// The GLSL ES 3.00 integer operations (IR v1) against the evaluator, from
/// hand-built fragment modules: the ES 3.00 front end is not on main yet, so
/// no source reaches them. The inputs are floats in [-2, 2] read as bits, so
/// half of them are >= 2^31 unsigned and negative signed: a signed/unsigned
/// mix-up in selection's TYPE, or in the interpreter's reading of it, cannot
/// agree by accident. A comparison's mask becomes 1.0/0.0 (B2f) and an
/// integer becomes a float (U2f/I2f) before it is stored, because the
/// comparison skips non-finite lanes and a mask read as a float is a NaN.
#[test]
fn the_es3_integer_operations_compute_what_the_ir_says() {
    use ir::{AluOp::*, Op, Src, ValueId};
    let v = |i: u32| Src::id(ValueId(i));
    /* (name, the op under test, how its result becomes a float) */
    let binary = [
        ("ult", Ult, B2f),
        ("ule", Ule, B2f),
        ("imin", Imin, I2f),
        ("imax", Imax, I2f),
        ("umin", Umin, U2f),
        ("umax", Umax, U2f),
    ];
    let mut cases: Vec<(&str, Vec<Op>)> = Vec::new();
    for (name, op, conv) in binary {
        cases.push((name, vec![Op::Alu(op, [v(0), v(1), v(0)]), Op::Alu(conv, [v(2); 3])]));
    }
    cases.push(("inot", vec![Op::Alu(Inot, [v(0); 3]), Op::Alu(U2f, [v(2); 3])]));
    /* ushr by (b & 31): a count of 32 or more is undefined, and the
     * evaluator would stop there instead of answering. */
    cases.push((
        "ushr",
        vec![
            Op::ConstI32([31; 4]),
            Op::Alu(Iand, [v(1), v(2), v(0)]),
            Op::Alu(Ushr, [v(0), v(3), v(0)]),
            Op::Alu(U2f, [v(4); 3]),
        ],
    ));
    /* f2u of |a| * 1.5e9, up to 3e9: above 2^31, where a signed f2i
     * saturates or wraps, and below 2^32. */
    cases.push((
        "f2u",
        vec![
            Op::ConstF32([1.5e9f32.to_bits(); 4]),
            Op::Alu(Fabs, [v(0); 3]),
            Op::Alu(Fmul, [v(3), v(2), v(0)]),
            Op::Alu(F2u, [v(4); 3]),
            Op::Alu(U2f, [v(5); 3]),
        ],
    ));
    /* The unsigned divide: a / b and a % b with b = (v1 >> (v2 & 31)) | 1,
     * so divisors range from ~2^30 down to 1 and are never 0 (undefined), and
     * a is v0's bits, half of them >= 2^31. */
    for (name, op) in [("udiv", Udiv), ("umod", Umod)] {
        cases.push((
            name,
            vec![
                Op::LoadInput { slot: IoSlot::Varying(2), comps: 4 },
                Op::ConstI32([31; 4]),
                Op::Alu(Iand, [v(2), v(3), v(0)]),
                Op::Alu(Ushr, [v(1), v(4), v(0)]),
                Op::ConstI32([1; 4]),
                Op::Alu(Ior, [v(5), v(6), v(0)]),
                Op::Alu(op, [v(0), v(7), v(0)]),
                Op::Alu(U2f, [v(8); 3]),
            ],
        ));
    }
    let (mut agreed, mut bad) = (0usize, Vec::new());
    for (name, body) in &cases {
        let mut instrs = vec![
            Op::LoadInput { slot: IoSlot::Varying(0), comps: 4 },
            Op::LoadInput { slot: IoSlot::Varying(1), comps: 4 },
        ];
        instrs.extend(body.iter().copied());
        let last = instrs.len() as u32 - 1;
        instrs.push(Op::StoreOutput { slot: IoSlot::FragColor(0), src: v(last) });
        let nodes: Vec<ir::Node> =
            (0..instrs.len() as u32).map(|i| ir::Node::Instr(ir::InstrId(i))).chain([ir::Node::Return]).collect();
        let decl = |name: &'static str, kind, location| ir::IoDecl {
            name,
            kind,
            location,
            components: 4,
            interp: ir::INTERP_FLAT,
            gl_type: 0,
        };
        let inputs =
            [decl("a", ir::IoKind::Varying, 0), decl("b", ir::IoKind::Varying, 1), decl("c", ir::IoKind::Varying, 2)];
        let outputs = [decl("fragColor", ir::IoKind::Colour, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: nodes.len() as u32 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(yakogl::shaderc::SHC_STAGE_FRAGMENT)
        };
        for trial in 0..8u64 {
            let mut mem = vec![0u8; 1 << 20];
            let a = Arena::new(&mut mem);
            match compare_module(name, sema::Stage::Fragment, &m, &a, 0xe53_0000 + trial * 7919) {
                Some(Verdict::Agree) => agreed += 1,
                Some(Verdict::Differ(why)) => bad.push(why),
                Some(Verdict::Skip(why)) => bad.push(format!("{name}: skipped, which proves nothing: {why}")),
                None => bad.push(format!("{name}: did not select or allocate")),
            }
        }
    }
    assert!(bad.is_empty(), "{} of {} disagree:\n{}", bad.len(), cases.len() * 8, bad.join("\n"));
    assert_eq!(agreed, cases.len() * 8, "every case and trial was compared");
}

/// The unsigned divide at the values where an estimate-and-refine sequence
/// goes wrong if it is going to: the largest dividend, a divisor of 1 (the
/// reciprocal whose scaled estimate must not overflow), the largest
/// divisor, powers of two either side, a dividend just below a multiple.
/// Chosen, not drawn, because a random draw almost never lands on them. The
/// result is split into its two 16-bit halves (each exact as a float) so
/// the comparison is bit for bit; the evaluator's u32 `/` and `%` are the
/// reference.
#[test]
fn the_unsigned_divide_is_exact_at_the_edges() {
    use ir::{AluOp::*, Op, Src, ValueId};
    let v = |i: u32| Src::id(ValueId(i));
    let pairs: [(u32, u32); 16] = [
        (u32::MAX, 1),
        (u32::MAX, 2),
        (u32::MAX, 3),
        (u32::MAX, u32::MAX),
        (u32::MAX - 1, u32::MAX),
        (0x8000_0000, 0x8000_0001),
        (0x8000_0000, 3),
        (0xffff_fffe, 0x7fff_ffff),
        (1_000_000_007, 65_537),
        (0xdead_beef, 0x0001_0000),
        (0xdead_beef, 0xffff),
        (4_294_967_291, 4_294_967_279),
        (12_345, 12_346),
        (0, 7),
        (0x7fff_ffff, 0x0000_b505),
        (0xc000_0000, 0x4000_0000),
    ];
    let mut bad = Vec::new();
    let mut compared = 0usize;
    for chunk in pairs.chunks(4) {
        let a: [i32; 4] = core::array::from_fn(|l| chunk[l].0 as i32);
        let b: [i32; 4] = core::array::from_fn(|l| chunk[l].1 as i32);
        for (name, op) in [("udiv", Udiv), ("umod", Umod)] {
            /* hi16 and lo16 of the result, each as a float, into two outputs */
            let instrs = vec![
                Op::ConstI32(a),
                Op::ConstI32(b),
                Op::Alu(op, [v(0), v(1), v(0)]),
                Op::ConstI32([16; 4]),
                Op::Alu(Ushr, [v(2), v(3), v(0)]),
                Op::Alu(U2f, [v(4); 3]),
                Op::ConstI32([0xffff; 4]),
                Op::Alu(Iand, [v(2), v(6), v(0)]),
                Op::Alu(U2f, [v(7); 3]),
                Op::StoreOutput { slot: IoSlot::Varying(0), src: v(5) },
                Op::StoreOutput { slot: IoSlot::Varying(1), src: v(8) },
                Op::LoadInput { slot: IoSlot::Attrib(0), comps: 4 },
                Op::StoreOutput { slot: IoSlot::Position, src: v(11) },
            ];
            let nodes: Vec<ir::Node> =
                (0..instrs.len() as u32).map(|i| ir::Node::Instr(ir::InstrId(i))).chain([ir::Node::Return]).collect();
            let decl = |name: &'static str, kind, location| ir::IoDecl {
                name,
                kind,
                location,
                components: 4,
                interp: ir::INTERP_SMOOTH,
                gl_type: 0,
            };
            let inputs = [decl("p", ir::IoKind::Attribute, 0)];
            let outputs = [
                decl("hi", ir::IoKind::Varying, 0),
                decl("lo", ir::IoKind::Varying, 1),
                decl("gl_Position", ir::IoKind::Position, 0),
            ];
            let m = ir::Module {
                instrs: &instrs,
                nodes: &nodes,
                body: ir::Range { first: 0, len: nodes.len() as u32 },
                inputs: &inputs,
                outputs: &outputs,
                ..ir::Module::empty(yakogl::shaderc::SHC_STAGE_VERTEX)
            };
            /* The evaluator is the reference; check it against Rust's own
             * u32 arithmetic first, so the comparison below means what it
             * says. */
            let mut mem = vec![0u8; 1 << 20];
            let arena = Arena::new(&mut mem);
            let want = eval::run(&m, &eval::Inputs { io: &[(IoSlot::Attrib(0), [0; 4])], uniforms: &[] })
                .expect("the evaluator runs it");
            for l in 0..4 {
                let (x, y) = (a[l] as u32, b[l] as u32);
                let r = if op == Udiv { x / y } else { x % y };
                let got = |slot| f32::from_bits(want.output(slot).expect("stored")[l]) as u32;
                assert_eq!((got(IoSlot::Varying(0)) << 16) | got(IoSlot::Varying(1)), r, "the evaluator's {name}");
            }
            match compare_module(name, sema::Stage::Vertex, &m, &arena, 1) {
                Some(Verdict::Agree) => compared += 1,
                Some(Verdict::Differ(why)) => bad.push(format!("{chunk:x?}: {why}")),
                Some(Verdict::Skip(why)) => bad.push(format!("{name} {chunk:x?}: skipped: {why}")),
                None => bad.push(format!("{name} {chunk:x?}: did not select or allocate")),
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert_eq!(compared, 8, "four chunks, two operations, all compared");
}

/// The allocator restructure's target through the whole back end: a
/// straight-line vertex program of `n` values, selected, passed, allocated,
/// encoded and interpreted, against the evaluator. The old 512-value bound
/// refused anything this long (random.23, 5,785 values, tools/cts/RESULTS.md).
///
/// The chain is x_k = x_{k-1} - x_{k-2}: period six, so it neither grows
/// nor contracts and a single wrong read anywhere reaches the output. (A
/// first version averaged, x_k = (x_{k-1} + x_{k-2}) / 2, which damps an
/// error away: an allocator mutation that clobbered live values passed it.)
#[test]
fn a_long_straight_line_program_computes_what_the_ir_says() {
    use ir::{AluOp::*, Op, Src, ValueId};
    let v = |i: u32| Src::id(ValueId(i));
    for n in [6_000u32, 20_000] {
        let mut instrs = vec![
            Op::LoadInput { slot: IoSlot::Attrib(0), comps: 4 },
            Op::ConstF32([0.75f32.to_bits(); 4]),
            Op::Alu(Fmul, [v(0), v(1), v(0)]),
        ];
        let (mut a, mut b) = (0u32, 2u32);
        while (instrs.len() as u32) < n {
            let t = instrs.len() as u32;
            instrs.push(Op::Alu(Fneg, [v(a); 3]));
            instrs.push(Op::Alu(Fadd, [v(b), v(t), v(0)]));
            (a, b) = (b, t + 1);
        }
        let last = instrs.len() as u32 - 1;
        instrs.push(Op::StoreOutput { slot: IoSlot::Varying(0), src: v(last) });
        instrs.push(Op::StoreOutput { slot: IoSlot::Position, src: v(0) });
        let nodes: Vec<ir::Node> =
            (0..instrs.len() as u32).map(|i| ir::Node::Instr(ir::InstrId(i))).chain([ir::Node::Return]).collect();
        let decl = |name: &'static str, kind, location| ir::IoDecl {
            name,
            kind,
            location,
            components: 4,
            interp: ir::INTERP_SMOOTH,
            gl_type: 0,
        };
        let inputs = [decl("p", ir::IoKind::Attribute, 0)];
        let outputs = [decl("x", ir::IoKind::Varying, 0), decl("gl_Position", ir::IoKind::Position, 0)];
        let m = ir::Module {
            instrs: &instrs,
            nodes: &nodes,
            body: ir::Range { first: 0, len: nodes.len() as u32 },
            inputs: &inputs,
            outputs: &outputs,
            ..ir::Module::empty(yakogl::shaderc::SHC_STAGE_VERTEX)
        };
        let mut mem = vec![0u8; 64 << 20];
        let arena = Arena::new(&mut mem);
        let t0 = std::time::Instant::now();
        let got = compare_module("chain", sema::Stage::Vertex, &m, &arena, 3);
        println!("{n} values: {:?} in {:?}", got.as_ref().map(|g| matches!(g, Verdict::Agree)), t0.elapsed());
        match got {
            Some(Verdict::Agree) => {}
            Some(Verdict::Differ(why)) => panic!("{n} values: {why}"),
            Some(Verdict::Skip(why)) => panic!("{n} values: skipped, which proves nothing: {why}"),
            None => panic!("{n} values: did not select or allocate"),
        }
    }
}

/// `gl_FragCoord` through the y transform (select.rs `frag_coord`): with the
/// pair filled as on an unmirrored target (1, 0), every lane of the program's
/// result is the IR's. es100/fragcoord.frag (gl_test 84) must be compared, not
/// skipped, so a transform that broke x, z, w or y shows here.
#[test]
fn gl_fragcoord_through_its_y_transform_computes_what_the_ir_says() {
    let src = include_str!("../tools/shaders/es100/fragcoord.frag");
    for seed in [1u64, 2, 3] {
        match compare("es100/fragcoord.frag", sema::Stage::Fragment, src, seed) {
            Some(Verdict::Agree) => {}
            Some(Verdict::Differ(d)) => panic!("{d}"),
            Some(Verdict::Skip(w)) => panic!("skipped, which proves nothing: {w}"),
            None => panic!("es100/fragcoord.frag did not select"),
        }
    }
}
