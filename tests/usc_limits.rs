// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the USC path actually asks of the machine, measured over every source
//! it accepts — before it is wired in, not after.
//!
//! The GC7000 side learned this the expensive way: a limit nobody had measured
//! (the keep arena) refused 74 sources in a survey, and the refusal was read
//! as a compiler gap until someone measured it. So the numbers this back end
//! depends on get measured while they are still cheap to change.
//!
//! Two different things are called a limit here and they are not the same:
//!
//!   * what the ENCODING can express -- a source slot's reach, the DOUTU
//!     temps field's 252. Measured from the ISA tables in
//!     `roundtrip::the_widest_operand_each_bank_can_reach_in_each_source_slot`
//!     and from the PDS field.
//!   * what our PROGRAMS use -- the peak of each register file over the
//!     corpus. That is this file, and it is what says whether the allocator's
//!     `Limits` stand-ins are safe.
//!
//! A peak is only as good as the population it was taken over, so the
//! population is printed and asserted.

#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr"))]

use yakogl::shaderc::online::ir::{AluOp, IoDecl, IoKind, IoSlot};
use yakogl::shaderc::online::powervr::abi::{assemble, attribs_read, vtxin_reg, UscProgram, USC_MAX_TEMPS};
use yakogl::shaderc::online::powervr::alloc::Limits;
use yakogl::shaderc::online::powervr::builtins::{shape, Shape};
use yakogl::shaderc::online::powervr::chain::{build_vertex, compile_fragment, Built, ChainError};
use yakogl::shaderc::online::powervr::decode::decode_group;
use yakogl::shaderc::online::powervr::sel::{Bank, MachineOp, Mods, SelDst, SelOp, SelSrc, SmpLod, Trig, IDX_SELF};
use yakogl::shaderc::online::powervr::select::{select, Selected};
use yakogl::shaderc::online::powervr::verify::{check_group, check_write, skeleton};
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::sha256::sha256;
use yakogl::shaderc::table::{SHC_BLOB_TABLE, SHC_SOURCE_TABLE};
use yakogl::shaderc::SHC_STAGE_VERTEX;

use std::collections::{BTreeMap, BTreeSet};

mod usc_exec;
use usc_exec::{alu, apply_mods, const_value, corpus_skeletons, named, run_binary, sentinel_inputs, Inputs};
use yakogl::shaderc::online::reflect::NO_BLOCK;

/// The tests of this binary share the compiler's one scratch arena, which
/// `store::with_scratch` hands to one caller at a time and refuses to the
/// rest (`None`): each test holds this for its whole run. Without it the
/// survey and a small test raced, and the loser read `None` as a failure.
///
/// It also lends the arena a region larger than the product's own
/// (`store::SCRATCH_BYTES`), once: the survey checks a compiled program by
/// selecting it again in the same arena, and those checks are the harness's
/// cost, not the compiler's. What the product would refuse is still refused:
/// every compile the survey counts is held to `SCRATCH_BYTES` by
/// [`within_product_scratch`].
fn scratch_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    static LENT: std::sync::Once = std::sync::Once::new();
    let guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    LENT.call_once(|| {
        let region: &'static mut [u8] = Vec::leak(vec![0u8; 4 * store::SCRATCH_BYTES]);
        // SAFETY: a leaked region, never used by anything else, and every
        // compile in this binary runs under `LOCK`.
        assert_eq!(unsafe { store::shc_online_lend_scratch(region.as_mut_ptr(), region.len()) }, 0);
    });
    guard
}

/// The product compiles in `store::SCRATCH_BYTES`; a compile the survey ran
/// in its larger region is refused as the product would refuse it when the
/// front end and the chain together used more.
fn within_product_scratch(a: &yakogl::shaderc::online::arena::Arena<'_>) -> Result<(), String> {
    if a.used() > store::SCRATCH_BYTES {
        return Err(format!("chain: OutOfScratch (used {} of the product's {})", a.used(), store::SCRATCH_BYTES));
    }
    Ok(())
}

const PLACEHOLDER: Limits = Limits { temps: USC_MAX_TEMPS, shared: 256, vtx_in: 64, coeff: 64 };

#[derive(Default)]
struct Peak {
    temps: u16,
    shared: u16,
    vtx_in: u16,
    coeff: u16,
    usc_dw: u32,
    varyings: u8,
    vs_output_size: u8,
    name_len: usize,
    groups: usize,
}

impl Peak {
    fn raise(&mut self, o: &Peak) {
        self.temps = self.temps.max(o.temps);
        self.shared = self.shared.max(o.shared);
        self.vtx_in = self.vtx_in.max(o.vtx_in);
        self.coeff = self.coeff.max(o.coeff);
        self.usc_dw = self.usc_dw.max(o.usc_dw);
        self.varyings = self.varyings.max(o.varyings);
        self.vs_output_size = self.vs_output_size.max(o.vs_output_size);
        self.name_len = self.name_len.max(o.name_len);
        self.groups = self.groups.max(o.groups);
    }
}

/// What the gate found in one program's bytes.
#[derive(Default)]
struct Gate {
    /// Groups the program holds, counted from what the chain returned.
    groups: usize,
    /// Groups read back and checked against their meaning.
    checked: usize,
    mismatches: Vec<String>,
    /// Executions compared, executions the selection interpreter could not
    /// run, and executions that disagreed.
    executed: usize,
    unexecuted: usize,
    exec_failures: Vec<String>,
    prep: yakogl::shaderc::online::powervr::prepare::PrepareStats,
    /// Our bytes against pco's for the same source: `Ok` agreed on every
    /// dword over three inputs; `Err` says why not, or why it could not run.
    pco: Option<Result<(), String>>,
    /// Where our record's interface differs from pco's for the same source.
    record_diffs: Vec<String>,
    /// Skeletons no group of the reference corpus has.
    unattested: Vec<String>,
    /// Our bytes against the front end's IR run with real control flow
    /// (`usc_exec::ir`): runs that agreed, why a run could not be made, and
    /// runs that disagreed.
    ir_agreed: usize,
    ir_unrun: Option<String>,
    ir_failures: Vec<String>,
    /// Runs whose input the IR's operations leave undefined (a NaN).
    ir_undefined: usize,
    /// Runs made against the IR: `IR_RUNS`, plus the search's.
    ir_runs: usize,
    /// The program's `if`s, and how many of them the runs drove both ways.
    ifs: usize,
    ifs_both: usize,
}

/// Fragment programs whose `if`s the survey's inputs cannot all drive both
/// ways, counted apart from the gate that every other program's are. Each
/// with its reason, and each checked to still fall short (the survey fails
/// when one no longer does, so the list cannot go stale).
///
/// - 27371573a9ac1abd.frag: Qt Quick 3D's volume ray march. Its `if`s sit in
///   a loop whose trip count is the ray's length through the volume's box
///   (under the execution mask since 65dc8942), behind an early `return`
///   when the ray misses the box -- which random inputs almost always do, so
///   the loop's `if`s are rarely reached (1 of 8 driven both ways).
///
/// - 96bc0c44f5554256.frag: Qt Quick 3D's extended scene environment
///   post-process (FXAA, dither, lens flare, glow, tonemapping). Its `if`s
///   test integer uniforms for exact values (`tonemapMode == 0..3`) and uint
///   flags, several nested under other flags, so the inputs drive 49 of 59
///   both ways. Surveyed since the compile scratch became 4 MiB (it needed
///   more than 2).
///
/// Per build: both are GLSL ES 3.00, which a report-es2 build refuses (pp.rs
/// out_of_scope), so there they are not surveyed at all.
const IFS_BEYOND_THE_INPUTS: &[&str] =
    if cfg!(feature = "report-es2") { &[] } else { &["27371573a9ac1abd.frag", "96bc0c44f5554256.frag"] };

/// Fragment programs where pco's table program is the odd one out: on an
/// input the survey names, ours and the IR (GLSL's answer, independent of
/// both compilers) give the same pixel and pco's program another. Each is
/// admitted only while that holds exactly, and leaves the list the moment
/// it does not (or the difference goes away).
///
/// - 52660c1a3d65bec6.frag: a Qt Quick 3D lit material (normal map, IBL),
///   on a back face (face_ccw 1, op 0, seed 1): ours and the IR 88eba1f1,
///   pco 88ebacf1 (green 0xa1 vs 0xac). The front faces agree, and the
///   frame's flip alone matches the IR on both faces
///   (`a_tangent_frame_flipped_by_gl_front_facing_matches_the_ir_on_both_faces`).
///   Either pco's program or the model's reading of something only it uses;
///   what decides it is a board capture of a back-facing lit material on the
///   TABLE path against a reference render (am62p-board).
///
/// - e0880dcd5582d4de.frag: Qt Quick 3D's environment-map prefilter (a loop
///   of textureLod on a samplerCube at levels from a uniform array). Since
///   native cube step C pco samples it as the TPU's own cube, with `smp`
///   fields identical to ours (`dmn` 3, level replaced, all sixteen
///   unrolled samples); ours and the IR ffffffff, pco's program ff000000 in
///   the model (NaN in a float variant) at seed 1. Either the model's
///   reading of pco's regenerated program or pco's program itself; what
///   decides it is the board: Quick 3D's IBL on the TABLE path against a
///   reference render (am62p-board, step C acceptance). Its BLEND lines are
///   that pixel blended and carry no IR value.
///
/// Empty under report-es2, where the ES 3.00 Qt sources are not compiled
/// (as IFS_BEYOND_THE_INPUTS).
const PCO_ALONE_DIFFERS: &[&str] =
    if cfg!(feature = "report-es2") { &[] } else { &["52660c1a3d65bec6.frag", "e0880dcd5582d4de.frag"] };

/// Table rows compiled from a glsl2usc helper that has since changed, where
/// ours follows the new helper: admitted while every plain `DIFFERS` line
/// has the IR on our side (at least one), the `BLEND` lines being that same
/// pixel blended (they carry no IR value); off the list the moment the row
/// is regenerated or stops differing.
///
/// Empty since native cube step C regenerated the table's cube rows
/// (cube.frag was on it while its row kept the stacked image's old u clamp).
const TABLE_BEHIND_THE_HELPER: &[&str] = &[];

/// A `DIFFERS` line of the survey where the IR's pixel is ours exactly.
fn ir_sides_with_ours(d: &str) -> bool {
    let (Some(o), Some(i)) = (d.split("ours {0: ").nth(1), d.split(" IR ").nth(1)) else { return false };
    d.starts_with("DIFFERS ")
        && o.split('}').next().is_some_and(|o| i.trim_start_matches('0') == o.trim_start_matches('0'))
}

/// Two output values the same up to operation order: an IR `sqrt` is one
/// rounding and our `frcp(frsq)` two, a dot product's sum is associated as
/// selection expands it. An arm selected wrongly or an operand dropped is
/// not within this.
/// Runs per program against the IR, cycling spread inputs and two sets of
/// small values that drive the conditions (`Inputs::small`,
/// `Inputs::positive`).
const IR_RUNS: u32 = 48;
/// Further runs, of `Inputs::mixed_from` (the program's own literals among
/// them), made only for a program with an `if` the fixed runs never drove
/// both ways.
const IR_SEARCH: u32 = 2000;

/// Every finite literal of a program as a float (an integer one as the
/// float that converts to it), each once, for the search's inputs
/// ([`Inputs::mixed_from`]).
fn float_literals(m: &yakogl::shaderc::online::ir::Module<'_>) -> Vec<f32> {
    let mut out: Vec<f32> = Vec::new();
    for op in m.instrs {
        /* an integer literal too, as the float that converts to it: a
         * condition `uint(int(v.w)) == 4u` compares against 4 */
        let lanes: [f32; 4] = match op {
            yakogl::shaderc::online::ir::Op::ConstF32(v) => v.map(f32::from_bits),
            yakogl::shaderc::online::ir::Op::ConstI32(v) => v.map(|x| x as f32),
            _ => continue,
        };
        for x in lanes {
            if x.is_finite() && !out.iter().any(|y: &f32| y.to_bits() == x.to_bits()) {
                out.push(x);
            }
        }
    }
    out
}

fn close(a: u32, b: u32) -> bool {
    let (x, y) = (f32::from_bits(a), f32::from_bits(b));
    a == b || (x.is_nan() && y.is_nan()) || (x - y).abs() <= 1e-4 * x.abs().max(y.abs()).max(1.0)
}

/// One source all the way through the library's own chain
/// (`powervr::chain::build_vertex`) and the record, then every group read
/// back: against what it was meant to say, and against the corpus.
fn run_one(text: &str, stage: Stage, corpus: &BTreeSet<String>) -> Result<(Peak, Gate), String> {
    let out = store::with_scratch(|a| -> Result<(Peak, Gate), String> {
        /* `assemble` below records into `keep` outside any compile, which
         * would open an arena nobody frees: one per source, kept for the
         * whole test process, until the other tests' compiles find `keep`
         * full. A transaction dropped uncommitted gives it all back. */
        let _keep = store::begin_keep().ok_or("keep: a compile already holds it")?;
        let text = a.alloc_str(text).map_err(|e| format!("arena: {e:?}"))?;
        let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
        let mut image = vec![0u8; 65536];
        let b = match build_vertex(&l.module, PLACEHOLDER, a, &mut image) {
            Ok(b) => b,
            Err(ChainError::Select(e)) => return Err(format!("selection: {e:?}")),
            Err(e) => return Err(format!("chain: {e:?}")),
        };
        within_product_scratch(a)?;
        let len = b.len;
        let words: Vec<u32> =
            (0..len / 4).map(|i| u32::from_le_bytes(image[i * 4..i * 4 + 4].try_into().unwrap())).collect();

        let uniforms = yakogl::shaderc::online::powervr::chain::uniform_rows_sampling(&l.reflection, b.sampled, a)
            .map_err(|e| format!("record: {e:?}"))?;
        let mut rec = assemble(&UscProgram {
            stage: SHC_STAGE_VERTEX,
            code: &words,
            temps: b.temps,
            io_out: yakogl::shaderc::online::powervr::abi::written_outputs(&l.module, a)
                .map_err(|e| format!("record: {e:?}"))?,
            io_in: &[],
            attribs: l.reflection.attribs,
            attribs_read: attribs_read(&l.module).map_err(|e| format!("record: {e:?}"))?,
            uniforms,
            uniform_dwords: l.reflection.uniform_dwords,
            meta_off: None,
        })
        .map_err(|e| format!("record: {e:?}"))?;
        /* the record fields compile_vertex adds after assembling */
        yakogl::shaderc::online::powervr::chain::record_vs_ids(&mut rec, &l.module)
            .map_err(|e| format!("record: {e:?}"))?;
        /* A vertex shader that samples: the record lists its samplers, whose
         * descriptors the driver places before the uniform block. Every
         * input below puts the model's image state there (a sampler's
         * marker), and the block at `base`, as the program reads them;
         * selection's own program reads the block from 0. */
        let samplers: Vec<(u8, u32)> = {
            let rows: &[yakogl::shaderc::ShcUniform] = if rec.num_uniforms == 0 {
                &[]
            } else {
                // SAFETY: the record's rows live in `keep`.
                unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) }
            };
            let mut v: Vec<(u8, u32)> = rows
                .iter()
                .filter(|u| u.dword_off == NO_BLOCK)
                .map(|u| {
                    let n: String = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
                    (u.binding, sampler_marker(&n))
                })
                .collect();
            v.sort_by_key(|(b, _)| *b);
            v
        };
        let base = 20 * samplers.len();
        let markers: BTreeMap<u8, u32> = samplers.iter().copied().collect();
        let with_descriptors = |inp: &mut Inputs| {
            for (i, (_, m)) in samplers.iter().enumerate() {
                inp.shared[20 * i..20 * i + 4].copy_from_slice(&usc_exec::tex_state(*m));
            }
        };
        /* a stacked texture's layer count, as the GL core writes it */
        let layer_dwords: Vec<(u16, u32)> = l
            .module
            .uniforms
            .iter()
            .filter(|u| u.name.starts_with("pvr_layers_") && u.dword_off != NO_BLOCK)
            .map(|u| (u.dword_off, usc_exec::layer_count(0, u.name)))
            .collect();
        /* the same inputs with the block moved to `base`, and the layer
         * counts in it (in both places: selection's program reads the block
         * from 0) */
        let with_layers = |inp: &mut Inputs, at: usize| {
            for (d, v) in &layer_dwords {
                inp.shared[at + *d as usize] = *v;
            }
        };
        let moved = |inp: &Inputs| -> Inputs {
            let mut out = inp.clone();
            if base > 0 {
                for d in (0..inp.shared.len() - base).rev() {
                    out.shared[base + d] = inp.shared[d];
                }
                with_descriptors(&mut out);
            }
            with_layers(&mut out, base);
            out
        };

        let mut gate = read_back(&b, &image[..len], corpus);
        gate.prep = b.prep;

        /* Execution: selection's program and our bytes, on the same inputs,
         * must write the same bits to the same output dwords. This is the
         * check that covers prepare's rewrites, the allocator's placement and
         * the epilogue's addresses at once -- each of which the per-group
         * read-back above takes on trust. */
        let sel = select(&l.module, a).map_err(|e| format!("selection (again): {e:?}"))?;
        for seed in 1..=3u32 {
            let mut inp = Inputs::new(seed);
            with_layers(&mut inp, 0);
            let Some((want, _)) = run_selected_from(&sel, &inp, BTreeMap::new(), &markers) else {
                gate.unexecuted += 1;
                break;
            };
            match run_binary(&image[..len], &moved(&inp), true) {
                Err(e) => {
                    gate.exec_failures.push(e);
                    break;
                }
                Ok(got) => {
                    gate.executed += 1;
                    if got.len() != want.len() {
                        gate.exec_failures.push(format!("{} output dwords written, {} meant", got.len(), want.len()));
                        break;
                    }
                    for ((slot, lane), w) in &want {
                        let Some(d) = expected_dword(*slot, *lane, l.module.outputs, &l.module) else {
                            gate.exec_failures.push(format!("{slot:?}.{lane}: no dword for it"));
                            break;
                        };
                        let g = got.get(&d).copied();
                        let same = g == Some(*w)
                            || g.is_some_and(|g| f32::from_bits(g).is_nan() && f32::from_bits(*w).is_nan());
                        if !same {
                            gate.exec_failures.push(format!(
                                "{slot:?}.{lane} at dword {d}: got {:?} want {}",
                                g.map(f32::from_bits),
                                f32::from_bits(*w)
                            ));
                            break;
                        }
                    }
                }
            }
        }

        /* The IR, with its `if`s taken as branches: the one check here that
         * does not go through selection, so an if-conversion that joins the
         * arms wrongly disagrees with it while agreeing with itself. */
        gate.ifs = usc_exec::ir::if_count(&l.module);
        let ids = yakogl::shaderc::online::powervr::abi::vs_id_regs(&l.module);
        let read = yakogl::shaderc::online::powervr::abi::attribs_placed(&l.module).unwrap_or(0);
        let literals = float_literals(&l.module);
        let mut taken: BTreeMap<u32, u8> = BTreeMap::new();
        let mut run = 0u32;
        loop {
            /* The fixed runs, then -- only while some `if` has not been seen
             * both ways -- inputs of mixed magnitude, up to a bound. */
            let covered = taken.values().filter(|m| **m == 3).count() == gate.ifs;
            if run >= IR_RUNS && (covered || run >= IR_RUNS + IR_SEARCH) {
                break;
            }
            let inp = match run {
                r if r >= IR_RUNS => Inputs::mixed_from(r, &literals),
                r if r % 3 == 0 => Inputs::new(r + 1),
                r if r % 3 == 1 => Inputs::small(r),
                r => Inputs::positive(r),
            };
            run += 1;
            gate.ir_runs += 1;
            let inp = moved(&inp);
            let ir_in = usc_exec::ir::IrInputs {
                input: &|slot, c| match slot {
                    IoSlot::Attrib(loc) => inp.vtxin[vtxin_reg(read, loc, c) as usize],
                    /* where the vertex fetch writes them (abi::vs_id_regs) */
                    IoSlot::VertexId => ids.0.map_or(0, |r| inp.vtxin[r as usize]),
                    IoSlot::InstanceId => ids.1.map_or(0, |r| inp.vtxin[r as usize]),
                    _ => 0,
                },
                uniform: &|d| inp.shared[base + d as usize],
                marker: &|b| markers.get(&b).copied().unwrap_or(0),
            };
            let want = match usc_exec::ir::run_ir(&l.module, &ir_in) {
                Ok(w) if w.undefined => {
                    gate.ir_undefined += 1;
                    continue;
                }
                Ok(w) => {
                    for (k, m) in &w.taken {
                        *taken.entry(*k).or_default() |= m;
                    }
                    w.outputs
                }
                Err(e) => {
                    gate.ir_unrun = Some(e);
                    break;
                }
            };
            /* Our own program not running is a FAILURE, never a refusal
             * that quietly leaves the population (a `?` here did). */
            let got = match run_binary(&image[..len], &inp, true) {
                Ok(g) => g,
                Err(e) => {
                    gate.ir_failures.push(format!("ours unrunnable: {e}"));
                    break;
                }
            };
            let mut bad = None;
            let mut compared = 0usize;
            for (slot, lanes) in &want {
                let comps = match slot {
                    IoSlot::Position => 4,
                    IoSlot::PointSize => 1,
                    IoSlot::Varying(loc) => l
                        .module
                        .outputs
                        .iter()
                        .find(|io| io.kind == IoKind::Varying && io.location == *loc)
                        .map_or(4, |io| io.components),
                    _ => 4,
                };
                for (lane, w) in lanes.iter().enumerate().take(comps as usize) {
                    /* An undefined lane (from a local nothing wrote, or a
                     * NaN an operation decided about) is not compared. */
                    let Some((mut w, false)) = *w else { continue };
                    compared += 1;
                    if *slot == IoSlot::PointSize {
                        w = f32::from_bits(w).clamp(1.0, 511.0).to_bits();
                    }
                    let g = expected_dword(*slot, lane, l.module.outputs, &l.module).and_then(|d| got.get(&d).copied());
                    if !g.is_some_and(|g| close(g, w)) {
                        bad =
                            Some(format!("{slot:?}.{lane}: ours {:?} IR {}", g.map(f32::from_bits), f32::from_bits(w)));
                        break;
                    }
                }
            }
            match bad {
                None if compared == 0 => gate.ir_undefined += 1,
                None => gate.ir_agreed += 1,
                Some(b) => {
                    gate.ir_failures.push(b);
                    break;
                }
            }
        }
        gate.ifs_both = taken.values().filter(|m| **m == 3).count();
        if gate.ir_unrun.is_none() && gate.ir_agreed == 0 && gate.ir_failures.is_empty() {
            gate.ir_failures.push("every input was undefined for the IR: nothing compared".into());
        }

        /* The reference: pco's program for the same text, run on the same
         * inputs. It answers what selection-vs-bytes cannot -- whether our
         * meaning is pco's, including where each uniform and attribute is
         * read from -- for every program whose reference uses only what the
         * interpreter knows. */
        let key = sha256(text.as_bytes());
        match SHC_BLOB_TABLE.iter().find(|r| r.sha256 == key && r.blend_key == 0) {
            None => gate.pco = Some(Err("no reference blob".into())),
            Some(blob) => {
                // SAFETY: `usc` points at a static of the generated table.
                let dw = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
                let pco: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
                /* The record against pco's: the interface the driver acts on.
                 * Attributes by name (width), uniforms by name (block offset
                 * and size), and the block's size. Where each attribute sits in
                 * vtxin is ours to choose and is not compared. */
                let name_of = |n: &[core::ffi::c_char]| -> String {
                    n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
                };
                let sh = &blob.shader;
                let mut rec_diff: Vec<String> = Vec::new();
                for at in &sh.attribs[..sh.num_attribs as usize] {
                    let n = name_of(&at.name);
                    match rec.attribs[..rec.num_attribs as usize].iter().find(|m| name_of(&m.name) == n) {
                        Some(m) if m.components == at.components && m.format == at.format => {}
                        Some(m) => {
                            rec_diff.push(format!("attrib {n}: comps {} vs pco {}", m.components, at.components))
                        }
                        None => rec_diff.push(format!("attrib {n}: missing")),
                    }
                }
                if rec.num_attribs != sh.num_attribs {
                    rec_diff.push(format!("{} attribs vs pco {}", rec.num_attribs, sh.num_attribs));
                }
                let theirs_u: &[yakogl::shaderc::ShcUniform] = if sh.num_uniforms == 0 {
                    &[]
                } else {
                    // SAFETY: `uniforms` points at `num_uniforms` rows of the generated table.
                    unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) }
                };
                for u in theirs_u {
                    let n = name_of(&u.name);
                    match uniforms.iter().find(|m| name_of(&m.name) == n) {
                        Some(m) if m.dword_off == u.dword_off && m.dwords == u.dwords && m.gl_type == u.gl_type => {}
                        Some(m) => rec_diff.push(format!(
                            "uniform {n}: off {} dw {} vs pco {} {}",
                            m.dword_off, m.dwords, u.dword_off, u.dwords
                        )),
                        None => rec_diff.push(format!("uniform {n}: missing")),
                    }
                }
                if rec.uniform_dwords != sh.uniform_dwords {
                    rec_diff.push(format!("uniform_dwords {} vs pco {}", rec.uniform_dwords, sh.uniform_dwords));
                }
                /* Every other scalar the driver or linker reads, against pco's
                 * value for the same source: a field left at the ZERO default
                 * reads as a claim (vs_psize_off 0 says "point size at dword
                 * 0", and the driver turns on the point-sprite vertex format
                 * for it). `temps` is the allocator's and differs by design;
                 * `uniform_sh_dwords` is pco's measured prefix where ours is 0,
                 * "the whole block", which DMAs more and reads the same. */
                let fields: [(&str, u32, u32); 18] = [
                    ("num_varyings", rec.num_varyings as u32, sh.num_varyings as u32),
                    ("uses_w", rec.uses_w as u32, sh.uses_w as u32),
                    ("uses_pntc", rec.uses_pntc as u32, sh.uses_pntc as u32),
                    ("uses_discard", rec.uses_discard as u32, sh.uses_discard as u32),
                    ("reads_front_facing", rec.reads_front_facing as u32, sh.reads_front_facing as u32),
                    ("meta_off", rec.meta_off as u32, sh.meta_off as u32),
                    ("fs_front_face_off1", rec.fs_front_face_off1 as u32, sh.fs_front_face_off1 as u32),
                    ("num_outputs", rec.num_outputs as u32, sh.num_outputs as u32),
                    ("num_tile_buffers", rec.num_tile_buffers as u32, sh.num_tile_buffers as u32),
                    ("tile_buffer_sh", rec.tile_buffer_sh as u32, sh.tile_buffer_sh as u32),
                    ("vs_output_size", rec.vs_output_size as u32, sh.vs_output_size as u32),
                    ("vs_psize_off", rec.vs_psize_off as u32, sh.vs_psize_off as u32),
                    /* Whether the shader reads gl_VertexID / gl_InstanceID;
                     * the register itself follows each compiler's attribute
                     * layout (ours `location * 4`, pco's packed), which is
                     * why `vi_reg` is not compared either. */
                    ("reads gl_VertexID", u32::from(rec.vs_vertex_id_reg1 != 0), u32::from(sh.vs_vertex_id_reg1 != 0)),
                    (
                        "reads gl_InstanceID",
                        u32::from(rec.vs_instance_id_reg1 != 0),
                        u32::from(sh.vs_instance_id_reg1 != 0),
                    ),
                    ("num_uniforms", rec.num_uniforms as u32, sh.num_uniforms as u32),
                    ("blend_key", rec.blend_key, sh.blend_key),
                    ("uses_fbfetch", rec.uses_fbfetch as u32, sh.uses_fbfetch as u32),
                    ("blend_consts_off", rec.blend_consts_off as u32, sh.blend_consts_off as u32),
                ];
                for (n, o, t) in fields {
                    if o != t {
                        rec_diff.push(format!("{n} {o} vs pco {t}"));
                    }
                }
                for tv in &sh.varyings[..sh.num_varyings as usize] {
                    let n = name_of(&tv.name);
                    match rec.varyings[..rec.num_varyings as usize].iter().find(|m| name_of(&m.name) == n) {
                        Some(m)
                            if (m.location, m.components, m.interp, m.vtxout_off)
                                == (tv.location, tv.components, tv.interp, tv.vtxout_off) => {}
                        Some(m) => rec_diff.push(format!(
                            "varying {n}: loc/comps/interp/off {:?} vs pco {:?}",
                            (m.location, m.components, m.interp, m.vtxout_off),
                            (tv.location, tv.components, tv.interp, tv.vtxout_off)
                        )),
                        None => rec_diff.push(format!("varying {n}: missing")),
                    }
                }
                gate.record_diffs = rec_diff;

                let mut verdict = Ok(());
                for seed in 1..=3u32 {
                    /* Inputs placed by NAME through each program's own
                     * interface: ours as selection reads them (an attribute at
                     * its rank among the placed locations, `abi::vtxin_reg`;
                     * one the program never reads has no register and is not
                     * placed, or it would land on another's; a uniform at its
                     * block offset), pco's as its record says (`vi_reg`,
                     * `dword_off`). */
                    let mut mine = sentinel_inputs(1.0e30);
                    let placed = yakogl::shaderc::online::powervr::abi::attribs_placed(&l.module).unwrap_or(0);
                    for at in
                        l.reflection.attribs.iter().filter(|at| at.location < 32 && placed & (1 << at.location) != 0)
                    {
                        for c in 0..at.components as u32 {
                            mine.vtxin[vtxin_reg(placed, at.location, c) as usize] = named(seed, at.name, c);
                        }
                    }
                    /* the shared file by name through each record's rows:
                     * descriptors (a sampler's model image) first, the block
                     * after them (`place_shared`) */
                    {
                        let rows: &[yakogl::shaderc::ShcUniform] = if rec.num_uniforms == 0 {
                            &[]
                        } else {
                            // SAFETY: the record's rows live in `keep`.
                            unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) }
                        };
                        place_shared(&mut mine, rows, &|n, k| named(seed, n, k));
                    }
                    let sh = &blob.shader;
                    let mut theirs_in = sentinel_inputs(-3.0e30);
                    for at in &sh.attribs[..sh.num_attribs as usize] {
                        let n: String = at.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
                        for c in 0..at.components as u32 {
                            theirs_in.vtxin[at.vi_reg as usize + c as usize] = named(seed, &n, c);
                        }
                    }
                    /* gl_VertexID / gl_InstanceID: one small integer each, at
                     * each program's own register (ours `abi::vs_id_regs`,
                     * pco's from its record's `vs_*_id_reg1`). */
                    let ids = yakogl::shaderc::online::powervr::abi::vs_id_regs(&l.module);
                    let read = yakogl::shaderc::online::powervr::abi::attribs_placed(&l.module).unwrap_or(0);
                    for (k, (ours_at, theirs_reg1)) in
                        [(ids.0, sh.vs_vertex_id_reg1), (ids.1, sh.vs_instance_id_reg1)].into_iter().enumerate()
                    {
                        let value = named(seed, "gl_id", k as u32) % 997;
                        if let Some(r) = ours_at {
                            mine.vtxin[r as usize] = value;
                        }
                        if theirs_reg1 != 0 {
                            theirs_in.vtxin[theirs_reg1 as usize - 1] = value;
                        }
                    }
                    // SAFETY: `uniforms` points at `num_uniforms` rows of the generated table.
                    let us: &[yakogl::shaderc::ShcUniform] = if sh.num_uniforms == 0 {
                        &[]
                    } else {
                        unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) }
                    };
                    place_shared(&mut theirs_in, us, &|n, k| named(seed, n, k));
                    let theirs = match run_binary(&pco, &theirs_in, false) {
                        Ok(t) => t,
                        Err(e) => {
                            verdict = Err(format!("unrunnable: {e}"));
                            break;
                        }
                    };
                    let ours = match run_binary(&image[..len], &mine, true) {
                        Ok(o) => o,
                        Err(e) => {
                            gate.exec_failures.push(format!("ours unrunnable against pco's inputs: {e}"));
                            verdict = Err(format!("unrunnable (ours): {e}"));
                            break;
                        }
                    };
                    /* Bit-equal, or within a few units in the last place: pco
                     * is free to order a sum differently (a dot product's
                     * three products, say), and that is rounding, not a
                     * different program. Kept as its own verdict. */
                    let ulps = |a: u32, b: u32| -> Option<u32> {
                        let (x, y) = (f32::from_bits(a), f32::from_bits(b));
                        if a == b || (x.is_nan() && y.is_nan()) {
                            return Some(0);
                        }
                        if x.is_nan() || y.is_nan() || (x < 0.0) != (y < 0.0) {
                            return None;
                        }
                        Some((a & 0x7fff_ffff).abs_diff(b & 0x7fff_ffff))
                    };
                    /* Only the dwords GLSL defines for this input: the IR
                     * run says which outputs were stored on the path taken
                     * and not undefined. An output a path never writes
                     * (imageparticle_colored.vert's fFade outside its inner
                     * `else`) holds whatever each compiler left there. */
                    let defined: BTreeSet<u32> = {
                        let ir_in = usc_exec::ir::IrInputs {
                            input: &|slot, c| match slot {
                                IoSlot::Attrib(loc) => mine.vtxin[vtxin_reg(read, loc, c) as usize],
                                _ => 0,
                            },
                            uniform: &|d| mine.shared[base + d as usize],
                            marker: &|b| markers.get(&b).copied().unwrap_or(0),
                        };
                        match usc_exec::ir::run_ir(&l.module, &ir_in) {
                            Ok(r) if !r.undefined => r
                                .outputs
                                .iter()
                                .flat_map(|(slot, lanes)| {
                                    lanes.iter().enumerate().filter_map(move |(k, x)| match x {
                                        Some((_, false)) => Some((*slot, k)),
                                        _ => None,
                                    })
                                })
                                .filter_map(|(slot, k)| expected_dword(slot, k, l.module.outputs, &l.module))
                                .collect(),
                            _ => BTreeSet::new(),
                        }
                    };
                    let mut worst = 0u32;
                    for (d, v) in theirs.iter().filter(|(d, _)| defined.contains(d)) {
                        match ours.get(d).and_then(|o| ulps(*o, *v)) {
                            Some(u) if u <= 4 => worst = worst.max(u),
                            _ => {
                                verdict = Err(format!(
                                    "DIFFERS at dword {d}: ours {:?} pco {}",
                                    ours.get(d).map(|o| f32::from_bits(*o)),
                                    f32::from_bits(*v)
                                ));
                                break;
                            }
                        }
                    }
                    if verdict.is_err() {
                        break;
                    }
                    if worst > 0 {
                        verdict = Err(format!("WITHIN {worst} ulp of pco"));
                    }
                    if ours.len() != theirs.len() {
                        verdict = Err(format!("DIFFERS: {} dwords written, pco {}", ours.len(), theirs.len()));
                        break;
                    }
                }
                gate.pco = Some(verdict);
            }
        }

        let name_len = rec.varyings[..rec.num_varyings as usize]
            .iter()
            .map(|v| v.name.iter().take_while(|c| **c != 0).count())
            .max()
            .unwrap_or(0);
        Ok((
            Peak {
                temps: b.temps,
                shared: b.shared,
                vtx_in: b.vtx_in,
                coeff: b.coeff,
                usc_dw: rec.usc_dw,
                varyings: rec.num_varyings,
                vs_output_size: rec.vs_output_size,
                name_len,
                groups: b.groups.len(),
            },
            gate,
        ))
    });
    match out {
        None => Err("the scratch arena was already borrowed".into()),
        Some(r) => r,
    }
}

/// Walk the image group by group: the arithmetic in order, then the writes,
/// then the end of task. The walk is driven by the MEANING (the groups and
/// writes the chain says it emitted) and each step decodes the bytes at the
/// current offset, so a group lost, gained or mis-sized breaks the walk.
fn read_back(b: &Built<'_>, image: &[u8], corpus: &BTreeSet<String>) -> Gate {
    let mut g = Gate::default();
    let mut off = 0usize;
    let skel = |bytes: &[u8], what: &str, g: &mut Gate| {
        if let Ok(d) = decode_group(bytes, 0) {
            let mut k = String::new();
            skeleton(&mut k, &d).unwrap();
            if !attested(corpus, &k) {
                g.unattested.push(format!("{what}: {k}"));
            }
        }
    };
    for sg in b.groups {
        /* A loop's top is an address, not a group; its back edge is one
         * `br` (its offset is the byte model's to follow). */
        match sg.op {
            SelOp::LoopBegin => continue,
            SelOp::LoopEnd => {
                g.groups += 1;
                match decode_group(&image[off..], 0) {
                    Ok(d)
                        if d.instrs().any(|(_, ph, x)| {
                            yakogl::shaderc::online::powervr::isa::ops_for(x, ph).any(|o| o.name == "br")
                        }) =>
                    {
                        g.checked += 1;
                        skel(&image[off..], "br", &mut g);
                        off += d.total;
                        continue;
                    }
                    _ => {
                        g.mismatches.push(format!("group {}: the back edge is not a br", g.groups - 1));
                        return g;
                    }
                }
            }
            _ => {}
        }
        g.groups += 1;
        let want = match sg.op {
            SelOp::LoopBegin | SelOp::LoopEnd => unreachable!("placed above"),
            SelOp::Copy => "mbyp",
            SelOp::Sample { .. } => "smp",
            SelOp::WaitData => "wdf",
            SelOp::TexWord { .. } => "<a state word the chain did not resolve>",
            SelOp::SampleZ => "<a cube coordinate the chain did not consume>",
            SelOp::SampleGrad => "<gradients the chain did not consume>",
            SelOp::PlaneCoeff { .. } => "<a plane coefficient the chain did not resolve>",
            SelOp::Machine(_) => "<template>",
            SelOp::Alu(a) => match shape(a) {
                Shape::Group(steps) => steps[0].op,
                _ => "<no single operation>",
            },
        };
        match check_group(sg, want, &image[off..]) {
            Ok(n) => {
                g.checked += 1;
                skel(&image[off..], want, &mut g);
                off += n;
            }
            Err(e) => {
                g.mismatches.push(format!("group {} ({want}): {e:?}", g.groups - 1));
                return g;
            }
        }
    }
    /* The writes, grouped exactly as `build_output_writes` groups them --
     * but the grouping is recomputed here from the list, not read from the
     * builder, and each group is checked for the address and repeat that
     * grouping implies. */
    let w = b.writes;
    let mut i = 0;
    while i < w.len() {
        /* `rpt` holds 1..4: the header field is two bits, and 4 is the
         * longest run the reference writes. */
        let mut run = 1usize;
        while i + run < w.len()
            && run < 4
            && w[i + run].bank == w[i].bank
            && w[i + run].reg as usize == w[i].reg as usize + run
            && w[i + run].addr as usize == w[i].addr as usize + run
        {
            run += 1;
        }
        g.groups += 1;
        match check_write(w[i], run as u32, &image[off..]) {
            Ok(n) => {
                g.checked += 1;
                skel(&image[off..], "uvsw.write", &mut g);
                off += n;
            }
            Err(e) => {
                g.mismatches.push(format!("write at dword {}: {e:?}", w[i].addr));
                return g;
            }
        }
        i += run;
    }
    g.groups += 1;
    /* pco's closing rule: the last group starts and ends on 8 bytes. */
    if !off.is_multiple_of(8) || !image.len().is_multiple_of(8) {
        g.mismatches.push(format!("the last group starts at {off} in a {}-byte image", image.len()));
    }
    match decode_group(image, off) {
        Ok(d) if d.hdr.get("end") == Some(1) && off + d.total == image.len() => {
            g.checked += 1;
            skel(&image[off..], "end", &mut g);
        }
        _ => g.mismatches.push("the last group is not a single end of task closing the image".into()),
    }
    /* Every vertex output dword written at most once, and gl_Position's four
     * always. A varying's dword may be missing: a component no path writes
     * is undefined, and pco writes nothing for it (Qt's particles set
     * `spriteData.x` only), which selection now matches; where each written
     * dword lands is checked by the value comparisons against pco and the
     * IR, which go by the dword each component is meant at. */
    let mut seen = std::collections::BTreeMap::<u8, usize>::new();
    for x in w {
        *seen.entry(x.addr).or_default() += 1;
    }
    if seen.values().any(|n| *n != 1) || (0..4u8).any(|d| !seen.contains_key(&d)) {
        g.mismatches.push(format!("an output dword written twice, or gl_Position not written whole: {seen:?}"));
    }
    g
}

/// What an unwritten value reads as, per side; finite and different, so a
/// dependence on one cannot agree by accident.
const POISON_SELECTED: u32 = 0x7149_f2ca; /* 1.0e30 */

/// What selection's program computed: each output component, and for a
/// shader that discards whether it killed the fragment.
type SelRun = (Vec<((IoSlot, usize), u32)>, Option<bool>);

/// [`run_selected`] with some values already defined: a fragment shader's
/// varyings, which selection names as the virtual registers of
/// `Selected::inputs`.
fn run_selected_from(
    sel: &Selected<'_>,
    inp: &Inputs,
    mut v: BTreeMap<u32, u32>,
    markers: &BTreeMap<u8, u32>,
) -> Option<SelRun> {
    let idx0 = std::cell::Cell::new(None::<u32>);
    let mut cube_z: Option<u32> = None;
    let read = |s: &SelSrc, v: &BTreeMap<u32, u32>| -> Option<u32> {
        let (raw, m) = match *s {
            /* A read before any write: selection's if-conversion selects
             * every local, including one declared inside the `if`, whose
             * "old" arm has no value. A poison value different from the
             * binary side's: if an output ever depended on it the two would
             * disagree. */
            SelSrc::Virtual { v: r, mods } => (v.get(&r.0).copied().unwrap_or(POISON_SELECTED), mods),
            SelSrc::Fixed { bank, reg, mods } => (
                match bank {
                    Bank::Shared => *inp.shared.get(reg as usize)?,
                    Bank::VtxIn => *inp.vtxin.get(reg as usize)?,
                    Bank::Special if reg as u32 == usc_exec::SR_SAMP_NUM => inp.samp_num,
                    Bank::Special if reg == 44 => inp.face_ccw,
                    Bank::Special if matches!(reg, 97 | 98 | 100 | 101) => {
                        inp.frag_xy[[97u16, 98, 100, 101].iter().position(|r| *r == reg)?]
                    }
                    /* A read through index register 0: the shared file at
                     * idx0 + offset (`sel::idx_shared`). */
                    Bank::Idx0 if reg & 7 == 3 => *inp.shared.get((idx0.get()? + u32::from(reg >> 3)) as usize)?,
                    Bank::Special => const_value(reg as u32)?,
                    _ => return None,
                },
                mods,
            ),
            SelSrc::Imm(x) => (x, Mods::default()),
        };
        Some(apply_mods(raw, m.neg, m.abs, m.flr))
    };
    let mut p0 = false;
    /* A loop kept as a loop: where each open one's body starts, and a step
     * budget so a program that never ends fails rather than hangs. */
    let mut tops: Vec<usize> = Vec::new();
    let mut pc = 0usize;
    let mut steps = 0usize;
    while let Some(g) = sel.groups.get(pc) {
        pc += 1;
        steps += 1;
        if steps > 1 << 22 {
            return None;
        }
        match g.op {
            SelOp::LoopBegin => {
                tops.push(pc);
                continue;
            }
            /* `tstz.p0`: p0 while the loop's test is zero. */
            SelOp::Machine(MachineOp::LoopTest) => {
                p0 = read(g.srcs[0].as_ref()?, &v)? == 0;
                continue;
            }
            SelOp::LoopEnd => {
                let top = *tops.last()?;
                if p0 {
                    pc = top;
                } else {
                    tops.pop();
                }
                continue;
            }
            /* The execution mask, one instance, as the byte model runs it:
             * the emc in srcs[0] (special 0 for the set-up) and dst. No group
             * of selection's sits between an instance leaving and the loop's
             * end, so being inactive never skips one here. */
            SelOp::Machine(
                op @ (MachineOp::CndInit | MachineOp::CndEnter | MachineOp::CndLeave | MachineOp::CndLoopEnd),
            ) => {
                let c = read(g.srcs[0].as_ref()?, &v)?;
                let next = match op {
                    MachineOp::CndInit => 0,
                    MachineOp::CndEnter => {
                        if c != 0 {
                            c + 2
                        } else {
                            0
                        }
                    }
                    MachineOp::CndLeave if c == 0 && !p0 => 2,
                    MachineOp::CndLeave => c,
                    _ if c == 0 => {
                        p0 = true;
                        0
                    }
                    _ => {
                        p0 = false;
                        c.saturating_sub(2)
                    }
                };
                let SelDst::Virtual(d) = g.dst? else { return None };
                v.insert(d.0, next);
                continue;
            }
            _ => {}
        }
        /* Setting index register 0 (`Bank::Idx0`, `IDX_SELF`). */
        if let Some(SelDst::Fixed { bank: Bank::Idx0, reg: IDX_SELF }) = g.dst {
            if g.op != SelOp::Copy {
                return None;
            }
            idx0.set(Some(read(g.srcs[0].as_ref()?, &v)?));
            continue;
        }
        /* A sample: the four results are the model's channels of the
         * texture bound at `binding`, at the coordinate. */
        /* a cube's third coordinate, for the sample that follows */
        if g.op == SelOp::SampleZ {
            cube_z = Some(read(g.srcs[0].as_ref()?, &v)?);
            continue;
        }
        if let SelOp::Sample { binding, proj, lod, fetch, offset, cube } = g.op {
            let (mut u, mut w) = (read(g.srcs[0].as_ref()?, &v)?, read(g.srcs[1].as_ref()?, &v)?);
            /* the TPU's own cube, through the face mapping the IR's cube
             * uses (usc_exec::cube_uv), the explicit level if replaced */
            if cube {
                let z = cube_z.take()?;
                let explicit = if lod == SmpLod::Replace { Some(read(g.srcs[2].as_ref()?, &v)?) } else { None };
                (u, w) = usc_exec::cube_uv(usc_exec::tex_state(*markers.get(&binding)?), [u, w, z], explicit);
            }
            /* texture2DProj: the divide the hardware does, as the byte
             * model does it. */
            if proj {
                let q = f32::from_bits(read(g.srcs[2].as_ref()?, &v)?);
                u = (f32::from_bits(u) / q).to_bits();
                w = (f32::from_bits(w) / q).to_bits();
            }
            let SelDst::Virtual(d) = g.dst? else { return None };
            let marker = *markers.get(&binding)?;
            /* textureLod: the level picks the model's texture; a bias too,
             * on the models' implicit level 0 */
            let marker =
                if lod.in_block() { usc_exec::marker_at_level(marker, read(g.srcs[2].as_ref()?, &v)?) } else { marker };
            /* texelFetch: texels, at a centre (`texel_coord`) */
            if fetch {
                u = usc_exec::texel_coord(f32::from_bits(u)).ok()?;
                w = usc_exec::texel_coord(f32::from_bits(w)).ok()?;
            }
            let offset = offset.map_or([0, 0], |o| usc_exec::lookup_offset(u32::from(o)));
            for k in 0..4 {
                v.insert(d.0 + k, usc_exec::sample_model_at(marker, u, w, offset, k));
            }
            continue;
        }
        /* A word of the sampler's image state: the model's, as `place_shared`
         * puts it in the descriptor. */
        if let SelOp::TexWord { binding, word } = g.op {
            let SelDst::Virtual(d) = g.dst? else { return None };
            v.insert(d.0, usc_exec::tex_state(*markers.get(&binding)?)[usize::from(word)]);
            continue;
        }
        let op = match g.op {
            SelOp::Copy => "mbyp",
            SelOp::Sample { .. }
            | SelOp::TexWord { .. }
            | SelOp::SampleZ
            | SelOp::SampleGrad
            | SelOp::PlaneCoeff { .. }
            | SelOp::WaitData
            | SelOp::LoopBegin
            | SelOp::LoopEnd
            | SelOp::Machine(
                MachineOp::LoopTest
                | MachineOp::CndInit
                | MachineOp::CndEnter
                | MachineOp::CndLeave
                | MachineOp::CndLoopEnd,
            ) => return None,
            SelOp::Machine(MachineOp::Ubfe) => "ubfe",
            SelOp::Machine(MachineOp::PckCov) => "pckcov",
            SelOp::Machine(MachineOp::SavMsk) => "savmsk",
            SelOp::Machine(MachineOp::FredA(_)) => "freda",
            SelOp::Machine(MachineOp::FredB(_)) => "fredb",
            SelOp::Machine(MachineOp::Fsinc(t)) => {
                if t == Trig::Sin {
                    "fsinc.sin"
                } else {
                    "fsinc.cos"
                }
            }
            SelOp::Machine(MachineOp::PselTrig) => "pseltrig",
            SelOp::Machine(MachineOp::Ushr) => "ushr",
            SelOp::Machine(MachineOp::Tstz) => "tstz",
            SelOp::Machine(MachineOp::MovS1) => "mbyp",
            SelOp::Machine(MachineOp::Iabs) => "iabs",
            SelOp::Machine(MachineOp::U2f) => "u2f",
            SelOp::Machine(MachineOp::F2u) => "f2u",
            SelOp::Machine(MachineOp::Uge) => "uge",
            SelOp::Machine(MachineOp::UmulHi) => "umulhi",
            SelOp::Alu(a) => match a {
                AluOp::Fadd => "fadd",
                AluOp::Fmul => "fmul",
                AluOp::Ffma => "fmad",
                AluOp::Frcp => "frcp",
                AluOp::Frsq => "frsq",
                AluOp::Fexp2 => "fexp",
                AluOp::Flog2 => "flog",
                AluOp::Fddx => "fdsx",
                AluOp::Fddy => "fdsy",
                AluOp::Fsqrt => "sqrt",
                AluOp::Fmin => "min",
                AluOp::Fmax => "max",
                AluOp::Bcsel => "bcsel",
                AluOp::Flt => "flt",
                AluOp::Fle => "fle",
                AluOp::Feq => "feq",
                AluOp::Fne => "fne",
                AluOp::Ilt => "ilt",
                AluOp::Ile => "ile",
                AluOp::Ieq => "ieq",
                AluOp::Ine => "ine",
                AluOp::B2f => "b2f",
                AluOp::B2i => "b2i",
                AluOp::F2b => "f2b",
                AluOp::I2b => "i2b",
                AluOp::Iadd => "iadd",
                AluOp::Imul => "imul",
                AluOp::I2f => "i2f",
                AluOp::F2i => "f2i",
                AluOp::Ineg => "ineg",
                AluOp::Bnot => "bnot",
                AluOp::Fsign => "fsign",
                AluOp::Ior => "ior",
                AluOp::Ishl => "ishl",
                AluOp::Fceil => "fceil",
                AluOp::Iand => "iand",
                AluOp::Ixor => "ixor",
                AluOp::Ishr => "ishr",
                AluOp::Ushr => "ushr",
                AluOp::U2f => "u2f",
                AluOp::F2u => "f2u",
                AluOp::Ult => "ult",
                AluOp::Ule => "ule",
                AluOp::Imin => "imin",
                AluOp::Imax => "imax",
                AluOp::Umin => "umin",
                AluOp::Umax => "umax",
                AluOp::Inot => "inot",
                _ => return None,
            },
        };
        let mut a = [0u32; 3];
        for (slot, s) in a.iter_mut().zip(g.srcs.iter()).take(g.op.arity()) {
            *slot = read(s.as_ref()?, &v)?;
        }
        /* The meaning, written from the GLSL ES definitions rather than from
         * the machine: a boolean is all-ones or zero. */
        let f = |i: usize| f32::from_bits(a[i]);
        let b = |x: bool| if x { u32::MAX } else { 0 };
        let r = match op {
            "sqrt" => f(0).sqrt().to_bits(),
            "min" => {
                if f(0) < f(1) {
                    a[0]
                } else {
                    a[1]
                }
            }
            "max" => {
                if f(0) > f(1) {
                    a[0]
                } else {
                    a[1]
                }
            }
            "bcsel" => {
                if a[0] != 0 {
                    a[1]
                } else {
                    a[2]
                }
            }
            "flt" => b(f(0) < f(1)),
            "fle" => b(f(0) <= f(1)),
            "feq" => b(f(0) == f(1)),
            "fne" => b(f(0) != f(1)),
            "ilt" => b((a[0] as i32) < (a[1] as i32)),
            "ile" => b((a[0] as i32) <= (a[1] as i32)),
            "ieq" => b(a[0] == a[1]),
            "ine" => b(a[0] != a[1]),
            "b2f" => {
                if a[0] != 0 {
                    1.0f32.to_bits()
                } else {
                    0
                }
            }
            "b2i" => u32::from(a[0] != 0),
            "f2b" => b(f(0) != 0.0),
            "i2b" => b(a[0] != 0),
            "iadd" => a[0].wrapping_add(a[1]),
            "imul" => a[0].wrapping_mul(a[1]),
            "i2f" => (a[0] as i32 as f32).to_bits(),
            "f2i" => f(0) as i32 as u32,
            "ineg" => a[0].wrapping_neg(),
            "bnot" => b(a[0] == 0),
            "ior" => a[0] | a[1],
            "ishl" => a[0].checked_shl(a[1]).unwrap_or(0),
            "fceil" => f32::from_bits(a[0]).ceil().to_bits(),
            /* ubfe(value, offset, bits) */
            "ubfe" => {
                let mask = if a[2] >= 32 { u32::MAX } else { (1u32 << a[2]) - 1 };
                a[0].checked_shr(a[1]).unwrap_or(0) & mask
            }
            "pckcov" => usc_exec::cov_model_of(a[0], inp.samples),
            "ushr" => a[0].checked_shr(a[1]).unwrap_or(0),
            "ishr" => (a[0] as i32).checked_shr(a[1]).unwrap_or(if (a[0] as i32) < 0 { -1 } else { 0 }) as u32,
            "ult" => b(a[0] < a[1]),
            "ule" => b(a[0] <= a[1]),
            "imin" => (a[0] as i32).min(a[1] as i32) as u32,
            "imax" => (a[0] as i32).max(a[1] as i32) as u32,
            "umin" => a[0].min(a[1]),
            "umax" => a[0].max(a[1]),
            "inot" => !a[0],
            "tstz" => b(a[0] == 0),
            "iabs" => (a[0] as i32).wrapping_abs() as u32,
            "u2f" => (a[0] as f32).to_bits(),
            "f2u" => f(0) as u32,
            "uge" => b(a[0] >= a[1]),
            "umulhi" => ((u64::from(a[0]) * u64::from(a[1])) >> 32) as u32,
            "savmsk" => inp.savmsk,
            /* The same model of the trig chain as `usc_exec::run_binary`. */
            "freda" => usc_exec::fred_a_tag(a[0]),
            "fredb" if a[1] == usc_exec::fred_a_tag(a[0]) => a[0],
            "fredb" => return None,
            "fsinc.sin" => {
                p0 = true;
                let x = f32::from_bits(a[0]);
                (if x == 0.0 { 1.0 } else { x.sin() / x }).to_bits()
            }
            "fsinc.cos" => {
                p0 = false;
                f32::from_bits(a[0]).cos().to_bits()
            }
            "pseltrig" => {
                if p0 {
                    (f32::from_bits(a[0]) * f32::from_bits(a[1])).to_bits()
                } else {
                    a[0]
                }
            }
            "iand" => a[0] & a[1],
            "ixor" => a[0] ^ a[1],
            /* The sign bit decides, as pco\'s does: -0.0 gives -1.0. */
            "fsign" => {
                if (a[0] as i32) < 0 {
                    (-1.0f32).to_bits()
                } else if f(0) > 0.0 {
                    1.0f32.to_bits()
                } else {
                    0
                }
            }
            _ => alu(op, &a)?,
        };
        match g.dst? {
            SelDst::Virtual(d) => {
                v.insert(d.0, r);
            }
            SelDst::Fixed { .. } => return None,
        }
    }
    let mut out = Vec::new();
    for (slot, comps) in sel.outputs {
        for (lane, c) in comps.iter().enumerate() {
            if let Some(c) = c {
                let mut x = read(c, &v)?;
                /* gl_PointSize means the size the rasteriser uses, which GL
                 * clamps to the implementation's range: 1.0..511.0 here. */
                if *slot == IoSlot::PointSize {
                    x = f32::from_bits(x).clamp(1.0, 511.0).to_bits();
                }
                out.push(((*slot, lane), x));
            }
        }
    }
    let kill = match sel.discard {
        None => None,
        Some(k) => Some(read(&k, &v)? != 0),
    };
    Some((out, kill))
}

/// Where each output component lands, from the module's declarations alone:
/// position at 0..3, then the smooth varyings, the flat ones, the
/// noperspective ones, each in declaration order, packed (pco's order).
/// Written here rather than taken from `vertex_output_offsets`, so the
/// builder and this check do not share the arithmetic they are meant to
/// cross-check.
fn expected_dword(
    slot: IoSlot,
    lane: usize,
    io_out: &[IoDecl<'_>],
    m: &yakogl::shaderc::online::ir::Module<'_>,
) -> Option<u32> {
    use yakogl::shaderc::online::ir::{INTERP_FLAT, INTERP_NOPERSPECTIVE};
    match slot {
        IoSlot::Position => Some(lane as u32),
        IoSlot::Varying(loc) => {
            let mut at = 4u32;
            /* only the varyings the program stores (pco's outputs_written) */
            let stored = |loc: u8| {
                m.instrs.iter().any(|op| {
                    matches!(op, yakogl::shaderc::online::ir::Op::StoreOutput { slot: IoSlot::Varying(l), .. } if *l == loc)
                })
            };
            let varyings = || io_out.iter().filter(|io| io.kind == IoKind::Varying && stored(io.location));
            let smooth = varyings().filter(|io| io.interp != INTERP_FLAT && io.interp != INTERP_NOPERSPECTIVE);
            let flat = varyings().filter(|io| io.interp == INTERP_FLAT);
            let npc = varyings().filter(|io| io.interp == INTERP_NOPERSPECTIVE);
            for io in smooth.chain(flat).chain(npc) {
                if io.location == loc {
                    return Some(at + lane as u32);
                }
                at += io.components as u32;
            }
            None
        }
        /* After every varying. */
        IoSlot::PointSize => {
            let stored = |loc: u8| {
                m.instrs.iter().any(|op| {
                    matches!(op, yakogl::shaderc::online::ir::Op::StoreOutput { slot: IoSlot::Varying(l), .. } if *l == loc)
                })
            };
            Some(
                4 + io_out
                    .iter()
                    .filter(|io| io.kind == IoKind::Varying && stored(io.location))
                    .map(|io| io.components as u32)
                    .sum::<u32>(),
            )
        }
        _ => None,
    }
}

/// The per-draw control word and sample coverage a discarding shader is
/// run with, by case: the driver's default word; with alpha to one; with
/// alpha to coverage; and the default word with sample 0 not covered. The
/// first is what the driver writes today; the others check that ours
/// computes pco's answer for every field, not only the default's.
fn meta_case(k: u32) -> (u32, u32) {
    let d = yakogl::shaderc::SHC_FS_META_DEFAULT;
    match k % 4 {
        0 => (d, 1),
        1 => (d | 1, 1),
        2 => (d | 1 << 25, 1),
        _ => (d, 2),
    }
}

/// Put `meta_case(k)` where each side reads it: the record's `meta_off`
/// for a program's bytes, `at` for selection's program.
fn place_meta(inp: &mut Inputs, at: Option<usize>, k: u32) {
    if let Some(at) = at {
        let (w, savmsk) = meta_case(k);
        inp.shared[at] = w;
        inp.savmsk = savmsk;
    }
}

/// A sampler's marker in its descriptor's first word: the model's name for
/// the texture bound there.
fn sampler_marker(name: &str) -> u32 {
    named(7, name, 99)
}

/// Fill the shared file as the driver does from a record's uniform rows:
/// one 20-dword descriptor per sampler row in binding order (its first word
/// the sampler's marker), then the uniform block, each dword by name.
fn place_shared(inp: &mut Inputs, rows: &[yakogl::shaderc::ShcUniform], value: &InputFn<'_>) {
    let name_of =
        |n: &[core::ffi::c_char]| -> String { n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect() };
    let mut samplers: Vec<&yakogl::shaderc::ShcUniform> = rows.iter().filter(|u| u.dword_off == NO_BLOCK).collect();
    samplers.sort_by_key(|u| u.binding);
    for (i, u) in samplers.iter().enumerate() {
        let state = usc_exec::tex_state(sampler_marker(&name_of(&u.name)));
        inp.shared[20 * i..20 * i + 4].copy_from_slice(&state);
    }
    let base = 20 * samplers.len();
    for u in rows.iter().filter(|u| u.dword_off != NO_BLOCK) {
        for k in 0..u.dwords as u32 {
            inp.shared[base + u.dword_off as usize + k as usize] = value(&name_of(&u.name), k);
        }
    }
}

/// One fragment shader through `chain::compile_fragment`, then the same
/// three questions as a vertex shader: every group's skeleton is pco's, the
/// last group is aligned, and the program computes what pco's does -- here
/// the packed colour in the pixel output -- with uniforms placed by name.
/// Returns what differs from pco, or why it stopped.
/// A fragment shader against its IR: runs that agreed, the `if`s and how
/// many were driven both ways, or why the IR model could not run it.
#[derive(Default)]
struct FsIr {
    /// Runs made: `IR_RUNS`, plus the search's.
    runs: usize,
    agreed: usize,
    /// Of the agreed runs, those the IR discarded (and ours did too).
    discarded: usize,
    undefined: usize,
    ifs: usize,
    ifs_both: usize,
    unrun: Option<String>,
    /// The table's blend-variant rows for this source: compared with ours
    /// (`compile_fragment_for`), identical on every input, and those ours
    /// refused or pco's the model could not run, by reason.
    variants: usize,
    variants_same: usize,
    variants_unmeasured: Vec<String>,
}

/// Where a fragment record's inputs sit in the coefficient file, as the
/// driver's coefficient program lays it out and pco's programs read it: Z's
/// four at 0 when the record iterates Z (`gl_FragCoord.z`), W's four next
/// when it iterates W, then `gl_PointCoord`'s eight when it uses the
/// point-sprite iterator, then four per varying component in list order.
/// `(point coordinate, first varying)`.
fn coeff_layout(uses_z: u8, uses_w: u8, uses_pntc: u8) -> (Option<usize>, usize) {
    let zw = if uses_z != 0 { 4 } else { 0 } + if uses_w != 0 { 4 } else { 0 };
    let pntc = (uses_pntc != 0).then_some(zw);
    (pntc, zw + if pntc.is_some() { 8 } else { 0 })
}

/// `gl_FragCoord.z` and `.w` into the Z and W planes' first coefficient,
/// which the record's no-divide iterators read (`value(2)`, `value(3)`),
/// when it iterates them.
fn place_frag_zw(inp: &mut usc_exec::Inputs, r: &yakogl::shaderc::ShcShader, value: &dyn Fn(u32) -> u32) {
    if r.uses_z != 0 {
        inp.coeff[0] = value(2);
    }
    if r.uses_w != 0 {
        inp.coeff[if r.uses_z != 0 { 4 } else { 0 }] = value(3);
    }
}

/// `gl_PointCoord`'s s and t into the coefficient file at `at` (each
/// component's plane, first and constant term, as a varying's), when the
/// record iterates it.
fn place_point_coord(inp: &mut usc_exec::Inputs, at: Option<usize>, value: &dyn Fn(u32) -> u32) {
    if let Some(at) = at {
        for c in 0..2 {
            inp.coeff[at + 4 * c] = value(c as u32);
            inp.coeff[at + 4 * c + 2] = value(c as u32);
        }
    }
}

fn run_fragment(text: &str, corpus: &BTreeSet<String>) -> Result<(Vec<String>, FsIr), String> {
    let out = store::with_scratch(|a| -> Result<(Vec<String>, FsIr), String> {
        let text = a.alloc_str(text).map_err(|e| format!("arena: {e:?}"))?;
        /* `compile_fragment` records into `keep` outside any compile of
         * the backend's, which would open an arena nobody frees: one per
         * source, kept for the whole test process, until the other tests'
         * compiles find `keep` full. Nothing of the record outlives this
         * closure, so a transaction dropped uncommitted gives it back. */
        let _keep = store::begin_keep().ok_or("keep: a compile already holds it")?;
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = match compile_fragment(&l.module, &l.reflection, a) {
            Ok(r) => r,
            Err(ChainError::Select(e)) => return Err(format!("selection: {e:?}")),
            Err(e) => return Err(format!("chain: {e:?}")),
        };
        within_product_scratch(a)?;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut diffs = Vec::new();

        let mut off = 0usize;
        let mut last = 0usize;
        while off < image.len() {
            let g = decode_group(&image, off).map_err(|e| format!("ours does not decode at {off}: {e:?}"))?;
            let mut k = String::new();
            skeleton(&mut k, &g).unwrap();
            if !attested(corpus, &k) {
                diffs.push(format!("UNATTESTED {k}"));
            }
            last = off;
            off += g.total;
        }
        if !last.is_multiple_of(8) || !image.len().is_multiple_of(8) {
            diffs.push(format!("the last group starts at {last} of {}", image.len()));
        }

        let key = sha256(text.as_bytes());
        let blob = SHC_BLOB_TABLE.iter().find(|r| r.sha256 == key && r.blend_key == 0).ok_or("no reference blob")?;
        let sh = &blob.shader;
        /* The control word of a shader that discards: pco's table puts it
         * after the uniform PREFIX its program loads and reports that
         * prefix as `uniform_dwords` (glsl2usc, "full (discard) mode");
         * ours puts it after the whole block and reports the whole block.
         * Both are what the driver needs -- it uploads `uniform_dwords`
         * dwords and writes the word at `meta_off` -- so the two fields are
         * a named note when each record follows its own rule exactly. */
        let desc = |n: u32| n * 20;
        let ours_samplers = (0..rec.num_uniforms as usize)
            // SAFETY: the record's uniform rows live in `keep`.
            .filter(|&i| unsafe { *rec.uniforms.add(i) }.dword_off == NO_BLOCK)
            .count() as u32;
        let theirs_samplers = (0..sh.num_uniforms as usize)
            // SAFETY: `uniforms` points at `num_uniforms` rows of the generated table.
            .filter(|&i| unsafe { *sh.uniforms.add(i) }.dword_off == NO_BLOCK)
            .count() as u32;
        /* ours puts gl_FragCoord's pair between the block and the control
         * word; pco's programs have none */
        let ours_pair = if rec.fragcoord_ytf_off1 != 0 { 2 } else { 0 };
        let own_rule = rec.meta_off != yakogl::shaderc::SHC_NO_META
            && sh.meta_off != yakogl::shaderc::SHC_NO_META
            && rec.meta_off as u32 == desc(ours_samplers) + rec.uniform_dwords as u32 + ours_pair
            && sh.meta_off as u32 == desc(theirs_samplers) + sh.uniform_dwords as u32
            && rec.uniform_dwords >= sh.uniform_dwords;
        if own_rule && rec.meta_off != sh.meta_off {
            diffs.push(format!(
                "NOTE the control word after the whole block (meta_off {}, {} dwords) where pco loads a prefix ({}, {})",
                rec.meta_off, rec.uniform_dwords, sh.meta_off, sh.uniform_dwords
            ));
        }
        /* The front-face op likewise: pco's after its prefix (and its
         * control word), ours after the whole block (and ours). */
        let ff_after = |off1: u16, samplers: u32, dwords: u16, pair: u32, meta: u16| {
            off1 as u32 == desc(samplers) + dwords as u32 + pair + u32::from(meta != yakogl::shaderc::SHC_NO_META) + 1
        };
        let ff_rule = rec.fs_front_face_off1 != 0
            && sh.fs_front_face_off1 != 0
            && ff_after(rec.fs_front_face_off1, ours_samplers, rec.uniform_dwords, ours_pair, rec.meta_off)
            && ff_after(sh.fs_front_face_off1, theirs_samplers, sh.uniform_dwords, 0, sh.meta_off)
            && rec.uniform_dwords >= sh.uniform_dwords;
        if ff_rule && rec.fs_front_face_off1 != sh.fs_front_face_off1 {
            diffs.push(format!(
                "NOTE the front-face op after the whole block (fs_front_face_off1 {}, {} dwords) where pco loads a prefix ({}, {})",
                rec.fs_front_face_off1, rec.uniform_dwords, sh.fs_front_face_off1, sh.uniform_dwords
            ));
        }
        let prefix_rule = own_rule || ff_rule;
        for (n, o, t) in [
            ("num_outputs", rec.num_outputs as u32, sh.num_outputs as u32),
            ("meta_off", if own_rule { 0 } else { rec.meta_off as u32 }, if own_rule { 0 } else { sh.meta_off as u32 }),
            ("blend_consts_off", rec.blend_consts_off as u32, sh.blend_consts_off as u32),
            ("uses_discard", rec.uses_discard as u32, sh.uses_discard as u32),
            (
                "uniform_dwords",
                if prefix_rule { 0 } else { rec.uniform_dwords as u32 },
                if prefix_rule { 0 } else { sh.uniform_dwords as u32 },
            ),
            (
                "fs_front_face_off1",
                if ff_rule { 0 } else { rec.fs_front_face_off1 as u32 },
                if ff_rule { 0 } else { sh.fs_front_face_off1 as u32 },
            ),
            ("num_uniforms", rec.num_uniforms as u32, sh.num_uniforms as u32),
            ("vs_psize_off", rec.vs_psize_off as u32, sh.vs_psize_off as u32),
            ("uses_w", rec.uses_w as u32, sh.uses_w as u32),
            ("uses_pntc", rec.uses_pntc as u32, sh.uses_pntc as u32),
        ] {
            if o != t {
                diffs.push(format!("RECORD {n} {o} vs pco {t}"));
            }
        }

        /* pco drops a varying NIR proves dead; we keep one IEEE arithmetic
         * cannot prove dead (a product with 0.0 is not 0.0 for a NaN). More
         * varyings than pco is its own verdict, named per source, and only
         * when every varying pco reads is ours too and the pixel agrees;
         * fewer is a difference. */
        if rec.num_varyings > sh.num_varyings {
            diffs.push(format!("NOTE iterates {} varyings pco found dead", rec.num_varyings - sh.num_varyings));
        } else if rec.num_varyings < sh.num_varyings {
            diffs.push(format!("RECORD num_varyings {} vs pco {}", rec.num_varyings, sh.num_varyings));
        }
        for tv in &sh.varyings[..sh.num_varyings as usize] {
            let n: String = tv.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
            let m = rec.varyings[..rec.num_varyings as usize]
                .iter()
                .find(|m| m.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect::<String>() == n);
            /* A fragment varying's vtxout_off is overwritten at link with
             * the vertex shader's (src/shaderc/link.rs, `by_name`), so it
             * only mirrors the record's own packing: compared when both
             * records list the same varyings, not when ours also lists ones
             * pco found dead (which shifts the packing). */
            let same_list = rec.num_varyings == sh.num_varyings;
            match m {
                Some(m)
                    if (m.location, m.components, m.interp) == (tv.location, tv.components, tv.interp)
                        && (!same_list || m.vtxout_off == tv.vtxout_off) => {}
                other => diffs.push(format!(
                    "RECORD varying {n}: ours {:?} pco {:?}",
                    other.map(|m| (m.location, m.components, m.interp, m.vtxout_off)),
                    (tv.location, tv.components, tv.interp, tv.vtxout_off)
                )),
            }
        }
        // SAFETY: `usc` points at a static of the generated table.
        let dw = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        let pco: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
        let theirs_u: &[yakogl::shaderc::ShcUniform] = if sh.num_uniforms == 0 {
            &[]
        } else {
            // SAFETY: `uniforms` points at `num_uniforms` rows of the generated table.
            unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) }
        };
        let name_of = |n: &[core::ffi::c_char]| -> String {
            n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        /* Our bytes against selection's own program, whatever pco's does:
         * the fragment counterpart of the vertex check, and the only
         * execution check for a shader whose reference program the model
         * cannot run (pco's predicated groups). */
        let sel = select(&l.module, a).map_err(|e| format!("selection (again): {e:?}"))?;
        let ours_u: &[yakogl::shaderc::ShcUniform] = if rec.num_uniforms == 0 {
            &[]
        } else {
            // SAFETY: the record's uniform rows live in `keep`.
            unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) }
        };
        for seed in 11..=13u32 {
            let unit = |n: &str, k: u32| -> u32 {
                if n.starts_with("pvr_layers_") {
                    return named(seed, n, k);
                }
                (f32::from_bits(named(seed, n, k)).abs() / 8.0).fract().to_bits()
            };
            /* The bytes read the record's layout: descriptors (a marker per
             * sampler, in binding order) then the uniform block. Selection's
             * program reads the block at its own offsets and names samplers
             * by binding. */
            let mut mine = sentinel_inputs(1.0e30);
            place_shared(&mut mine, ours_u, &|n, k| unit(n, k));
            let mut sel_in = sentinel_inputs(1.0e30);
            for u in l.reflection.uniforms.iter().filter(|u| u.dword_off != NO_BLOCK) {
                for k in 0..u.dwords as u32 {
                    sel_in.shared[u.dword_off as usize + k as usize] = unit(u.name, k);
                }
            }
            let markers: BTreeMap<u8, u32> = ours_u
                .iter()
                .filter(|u| u.dword_off == NO_BLOCK)
                .map(|u| (u.binding, sampler_marker(&name_of(&u.name))))
                .collect();
            let mut pre: BTreeMap<u32, u32> = BTreeMap::new();
            let (pntc_at, mut at) = coeff_layout(rec.uses_z, rec.uses_w, rec.uses_pntc);
            let pc = |c: u32| unit("gl_PointCoord", c);
            place_point_coord(&mut mine, pntc_at, &pc);
            place_point_coord(&mut sel_in, pntc_at, &pc);
            let fc = |c: u32| unit("gl_FragCoord", c);
            place_frag_zw(&mut mine, &rec, &fc);
            place_frag_zw(&mut sel_in, &rec, &fc);
            if let Some((_, lanes)) = sel.inputs.iter().find(|(slot, _)| *slot == IoSlot::FragCoord) {
                for (c, lane) in lanes.iter().enumerate() {
                    if let Some(vr) = lane {
                        pre.insert(vr.0, fc(c as u32));
                    }
                }
            }
            if let Some((_, lanes)) = sel.inputs.iter().find(|(slot, _)| *slot == IoSlot::PointCoord) {
                for (c, lane) in lanes.iter().enumerate() {
                    if let Some(vr) = lane {
                        pre.insert(vr.0, pc(c as u32));
                    }
                }
            }
            for rv in &rec.varyings[..rec.num_varyings as usize] {
                let n: String = rv.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
                for c in 0..rv.components as usize {
                    {
                        /* the plane's first coefficient, where the model holds an iterated
                         * value, and its C term, which a flat varying reads */
                        let w = unit(&n, c as u32);
                        mine.coeff[at + 4 * c] = w;
                        mine.coeff[at + 4 * c + 2] = w;
                    }
                    {
                        let w = unit(&n, c as u32);
                        sel_in.coeff[at + 4 * c] = w;
                        sel_in.coeff[at + 4 * c + 2] = w;
                    }
                }
                let io = l.module.inputs.iter().find(|io| io.name == n).ok_or("a record varying the module lacks")?;
                if let Some((_, lanes)) = sel.inputs.iter().find(|(slot, _)| *slot == IoSlot::Varying(io.location)) {
                    for (c, lane) in lanes.iter().enumerate() {
                        if let Some(vr) = lane {
                            pre.insert(vr.0, unit(&n, c as u32));
                        }
                    }
                }
                at += 4 * rv.components as usize;
            }
            let rec_meta = (rec.meta_off != yakogl::shaderc::SHC_NO_META).then_some(rec.meta_off as usize);
            place_meta(&mut mine, rec_meta, seed);
            place_meta(
                &mut sel_in,
                sel.discard.map(|_| yakogl::shaderc::online::powervr::select::meta_dword(&l.module) as usize),
                seed,
            );
            /* The driver's other words, each at the record's register for the
             * bytes and at selection's own for selection: gl_FragCoord's pair
             * as a target that is not flipped, and the face and op cycling. */
            if let Some(y) = yakogl::shaderc::online::powervr::select::ytf_dword(&l.module) {
                /* The record names the pair only when the program still reads
                 * it (chain.rs `ytf_live`), as for the front-face op below. */
                if rec.fragcoord_ytf_off1 != 0 {
                    let at = rec.fragcoord_ytf_off1 as usize - 1;
                    (mine.shared[at], mine.shared[at + 1]) = (1.0f32.to_bits(), 0);
                }
                (sel_in.shared[y as usize], sel_in.shared[y as usize + 1]) = (1.0f32.to_bits(), 0);
            }
            if let Some(f) = yakogl::shaderc::online::powervr::select::ff_dword(&l.module) {
                if rec.fs_front_face_off1 != 0 {
                    mine.shared[rec.fs_front_face_off1 as usize - 1] = (seed / 2) % 3;
                }
                sel_in.shared[f as usize] = (seed / 2) % 3;
                (mine.face_ccw, sel_in.face_ccw) = (seed & 1, seed & 1);
            }
            let Some((want, kill)) = run_selected_from(&sel, &sel_in, pre, &markers) else {
                diffs.push("SELECTION UNRUNNABLE by the model".into());
                break;
            };
            /* The packed colour, or none from a shader that writes none. */
            let mut px = None::<u32>;
            for ((slot, lane), val) in &want {
                if *slot != IoSlot::FragColor(0) {
                    continue;
                }
                let b = (f32::from_bits(*val).clamp(0.0, 1.0) * 255.0).round() as u32;
                px = Some(px.unwrap_or(0) | b << (8 * lane));
            }
            /* The pass feedback: given exactly when selection's program
             * keeps the fragment, and never by a shader that cannot discard. */
            let feedback = kill.map(|k| !k).unwrap_or(false).then_some(&1);
            match run_binary(&image, &mine, true) {
                Ok(o) if o.get(&0) == px.as_ref() && o.get(&usc_exec::FEEDBACK) == feedback => {}
                Ok(o) => {
                    diffs.push(format!("DIFFERS from selection: ours {o:x?} selection {px:x?} kill {kill:?}"));
                    break;
                }
                Err(e) => {
                    diffs.push(format!("OURS UNRUNNABLE: {e}"));
                    break;
                }
            }
        }

        /* The IR with its `if`s taken as branches, against our bytes: the
         * check that does not pass through selection. Varyings by name,
         * the block at the reflection's offsets, samplers by binding. */
        let mut fir = FsIr { ifs: usc_exec::ir::if_count(&l.module), ..FsIr::default() };
        let literals = float_literals(&l.module);
        let mut taken: BTreeMap<u32, u8> = BTreeMap::new();
        let mut run = 0u32;
        loop {
            /* The fixed runs, then -- only while some `if` has not been seen
             * both ways -- mixed magnitudes, up to a bound, as for a vertex
             * shader. */
            let covered = taken.values().filter(|m| **m == 3).count() == fir.ifs;
            if run >= IR_RUNS && (covered || run >= IR_RUNS + IR_SEARCH) {
                break;
            }
            let run = {
                run += 1;
                fir.runs += 1;
                run - 1
            };
            /* GL_INT, GL_INT_VEC2..4, GL_BOOL..GL_BOOL_VEC4: a condition on
             * one compares integers, which no float's bits will meet. */
            let integer = |n: &str| {
                l.reflection.uniforms.iter().find(|u| u.name == n).is_some_and(|u| {
                    /* GL_INT, GL_UNSIGNED_INT, GL_INT_VEC2..GL_BOOL_VEC4,
                     * GL_UNSIGNED_INT_VEC2..4 (ES 3.00: Qt's tonemap_mode
                     * is a uint compared with 1u..4u) */
                    matches!(u.gl_type, 0x1404 | 0x1405)
                        || (0x8b53..=0x8b59).contains(&u.gl_type)
                        || (0x8dc6..=0x8dc8).contains(&u.gl_type)
                })
            };
            let unit = |n: &str, k: u32| -> u32 {
                if n.starts_with("pvr_layers_") {
                    return named(run + 21, n, k);
                }
                let h = named(run + 21, n, k);
                if run >= IR_RUNS {
                    /* Log-uniform magnitude 0.001..1000, either sign, and an
                     * exact zero a quarter of the time, and one non-zero draw
                     * in four one of the program's own literals
                     * (`Inputs::mixed_from`). */
                    if (h >> 1) & 3 == 0 {
                        0
                    } else if !literals.is_empty() && (h >> 24) & 3 == 0 {
                        if integer(n) {
                            literals[((h >> 26) as usize) % literals.len()] as i32 as u32
                        } else {
                            literals[((h >> 26) as usize) % literals.len()].to_bits()
                        }
                    } else {
                        let m = 10f32.powf(((h >> 8) % 6001) as f32 / 1000.0 - 3.0);
                        (if h & 1 == 0 { m } else { -m }).to_bits()
                    }
                } else if run % 3 == 0 {
                    (f32::from_bits(h).abs() / 8.0).fract().to_bits()
                } else if integer(n) {
                    [0u32, 0, 1, 2][(h % 4) as usize]
                } else if run % 3 == 1 {
                    /* Condition-driving values, as `Inputs::small`. */
                    [0.0f32, 0.0, 1.0, 0.5, 0.25, 0.75, 2.0][(h % 7) as usize].to_bits()
                } else {
                    [0.1f32, 0.25, 0.4, 0.5, 0.6, 0.75, 0.9, 1.0][(h % 8) as usize].to_bits()
                }
            };
            let mut mine = sentinel_inputs(1.0e30);
            place_shared(&mut mine, ours_u, &|n, k| unit(n, k));
            let (pntc_at, mut at) = coeff_layout(rec.uses_z, rec.uses_w, rec.uses_pntc);
            place_point_coord(&mut mine, pntc_at, &|c| unit("gl_PointCoord", c));
            place_frag_zw(&mut mine, &rec, &|c| unit("gl_FragCoord", c));
            for rv in &rec.varyings[..rec.num_varyings as usize] {
                let n = name_of(&rv.name);
                for c in 0..rv.components as usize {
                    {
                        let w = unit(&n, c as u32);
                        mine.coeff[at + 4 * c] = w;
                        mine.coeff[at + 4 * c + 2] = w;
                    }
                }
                at += 4 * rv.components as usize;
            }
            let by_binding: BTreeMap<u8, u32> = ours_u
                .iter()
                .filter(|u| u.dword_off == NO_BLOCK)
                .map(|u| (u.binding, sampler_marker(&name_of(&u.name))))
                .collect();
            let uniform_name = |d: u16| -> Option<(&str, u32)> {
                l.reflection
                    .uniforms
                    .iter()
                    .filter(|u| u.dword_off != NO_BLOCK)
                    .find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d))
                    .map(|u| (u.name, (d - u.dword_off) as u32))
            };
            /* gl_FrontFacing: the face and the driver's op cycle with the
             * run, and the IR is told what they mean (Mesa lower_front_face:
             * 0 keeps face_ccw == 0, 1 swaps it, 2 is always front). */
            let (ccw, ff_op) = (run & 1, (run / 2) % 3);
            let front = ff_op == 2 || ((ccw == 0) != (ff_op == 1));
            if rec.fs_front_face_off1 != 0 {
                mine.face_ccw = ccw;
                mine.shared[rec.fs_front_face_off1 as usize - 1] = ff_op;
            }
            if rec.fragcoord_ytf_off1 != 0 {
                let at = rec.fragcoord_ytf_off1 as usize - 1;
                (mine.shared[at], mine.shared[at + 1]) = (1.0f32.to_bits(), 0);
            }
            let ir_in = usc_exec::ir::IrInputs {
                input: &|slot, c| match slot {
                    IoSlot::Varying(loc) => {
                        l.module.inputs.iter().find(|io| io.location == loc).map_or(0, |io| unit(io.name, c))
                    }
                    IoSlot::PointCoord => unit("gl_PointCoord", c),
                    IoSlot::FragCoord if c >= 2 => unit("gl_FragCoord", c),
                    IoSlot::FrontFacing if front => u32::MAX,
                    _ => 0,
                },
                uniform: &|d| uniform_name(d).map_or(0, |(n, k)| unit(n, k)),
                marker: &|b| by_binding.get(&b).copied().unwrap_or(0),
            };
            let want = match usc_exec::ir::run_ir(&l.module, &ir_in) {
                Ok(w) if w.undefined => {
                    fir.undefined += 1;
                    continue;
                }
                Ok(w) => w,
                Err(e) => {
                    fir.unrun = Some(e);
                    break;
                }
            };
            for (k, m) in &want.taken {
                *taken.entry(*k).or_default() |= m;
            }
            /* The driver's control word (the IR is GLSL, which has none of
             * its switches on). */
            let rec_meta = (rec.meta_off != yakogl::shaderc::SHC_NO_META).then_some(rec.meta_off as usize);
            place_meta(&mut mine, rec_meta, 0);
            let (ours, feedback) = match run_binary(&image, &mine, true) {
                Ok(o) => (o.get(&0).copied(), o.contains_key(&usc_exec::FEEDBACK)),
                Err(e) => {
                    diffs.push(format!("OURS UNRUNNABLE: {e}"));
                    break;
                }
            };
            /* A discarding shader gives the pass feedback exactly when the
             * IR kept the fragment; nothing a discarded one wrote counts. */
            if rec_meta.is_some() && feedback == want.discarded {
                diffs.push(format!(
                    "IR DIFFERS (run {run}): the IR {} the fragment, ours {}",
                    if want.discarded { "discards" } else { "keeps" },
                    if feedback { "keeps it" } else { "discards it" }
                ));
                break;
            }
            if want.discarded {
                fir.agreed += 1;
                fir.discarded += 1;
                continue;
            }
            let Some((_, lanes)) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)) else {
                /* A shader that stores no colour: ours must write none. */
                if ours.is_some() {
                    diffs.push(format!("IR DIFFERS (run {run}): the IR stores no colour, ours writes {ours:x?}"));
                    break;
                }
                fir.agreed += 1;
                continue;
            };
            /* One step of the 8-bit packing either way: `close` is about
             * operation order, which can move a value across a rounding
             * boundary. */
            /* Undefined channels (see `IrRun::outputs`) are not compared;
             * a run with none left is undefined as a whole. A channel the IR
             * computes as NaN is undefined too: GL leaves a NaN's conversion
             * to a normalized fixed-point colour undefined, so any byte
             * answers it. Compared, 96bc0c44f5554256.frag (a Qt shader, once
             * refused for scratch at 2 MiB) read "differs" on channels where
             * the IR held NaN, selection's program 0, ours 0x8b and 0xff. */
            let undefined = |l: &Option<(u32, bool)>| l.is_some_and(|(v, t)| t || f32::from_bits(v).is_nan());
            if lanes.iter().all(undefined) {
                fir.undefined += 1;
                continue;
            }
            let agrees = ours.is_some_and(|px| {
                (0..4).all(|c| {
                    if undefined(&lanes[c]) {
                        return true;
                    }
                    let w = lanes[c].map_or(0.0, |(x, _)| f32::from_bits(x));
                    let want = (w.clamp(0.0, 1.0) * 255.0).round() as i32;
                    let got = ((px >> (8 * c)) & 0xff) as i32;
                    (want - got).abs() <= 1
                })
            });
            if !agrees {
                let placed: Vec<(String, Vec<f32>)> = rec.varyings[..rec.num_varyings as usize]
                    .iter()
                    .map(|v| {
                        let n = name_of(&v.name);
                        let vals = (0..v.components as u32).map(|c| f32::from_bits(unit(&n, c))).collect();
                        (n, vals)
                    })
                    .collect();
                /* Which side left the IR: selection's program on the same
                 * input says whether it is selection or what lies below. */
                let mut sel_in = sentinel_inputs(1.0e30);
                for u in l.reflection.uniforms.iter().filter(|u| u.dword_off != NO_BLOCK) {
                    for k in 0..u.dwords as u32 {
                        sel_in.shared[u.dword_off as usize + k as usize] = unit(u.name, k);
                    }
                }
                let mut pre: BTreeMap<u32, u32> = BTreeMap::new();
                for (slot, lanes) in sel.inputs.iter() {
                    let IoSlot::Varying(loc) = slot else { continue };
                    let Some(io) = l.module.inputs.iter().find(|io| io.location == *loc) else { continue };
                    for (c, lane) in lanes.iter().enumerate() {
                        if let Some(vr) = lane {
                            pre.insert(vr.0, unit(io.name, c as u32));
                        }
                    }
                }
                let sel_px = run_selected_from(&sel, &sel_in, pre, &by_binding).map(|w| {
                    w.0.iter()
                        .filter(|((s, _), _)| *s == IoSlot::FragColor(0))
                        .map(|((_, c), v)| ((f32::from_bits(*v).clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c))
                        .fold(0u32, |a, b| a | b)
                });
                diffs.push(format!(
                    "IR DIFFERS (run {run}, varyings {placed:?}): ours {ours:x?} selection {sel_px:x?} IR {:?}",
                    lanes.map(|x| x.map(|(v, t)| (f32::from_bits(v), t)))
                ));
                break;
            }
            fir.agreed += 1;
        }
        fir.ifs_both = taken.values().filter(|m| **m == 3).count();
        if fir.unrun.is_none() && fir.agreed == 0 && !diffs.iter().any(|d| d.starts_with("IR ")) {
            diffs.push("IR every input was undefined: nothing compared".into());
        }

        for seed in 1..=4u32 {
            /* Colour-sized values: the packer clamps, so inputs far outside
             * 0..1 would make every program agree on 0 and 255. */
            let unit = |seed: u32, n: &str, k: u32| -> u32 {
                let x = f32::from_bits(named(seed, n, k)).abs() / 8.0;
                x.fract().to_bits()
            };
            let mut mine = sentinel_inputs(1.0e30);
            place_shared(&mut mine, ours_u, &|n, k| unit(seed, n, k));
            /* Varyings by name through each record's own list, laid out as
             * the coefficient file is (`coeff_layout`): W, the point
             * coordinate, then four per component in list order. */
            let place = |inp: &mut usc_exec::Inputs, r: &yakogl::shaderc::ShcShader| {
                let (pntc_at, mut at) = coeff_layout(r.uses_z, r.uses_w, r.uses_pntc);
                place_point_coord(inp, pntc_at, &|c| unit(seed, "gl_PointCoord", c));
                place_frag_zw(inp, r, &|c| unit(seed, "gl_FragCoord", c));
                /* gl_FragCoord's pair as an unflipped target (pco's programs
                 * have none and read y unchanged), and gl_FrontFacing's face
                 * and op, at the record's own offsets. */
                if r.fragcoord_ytf_off1 != 0 {
                    let at = r.fragcoord_ytf_off1 as usize - 1;
                    (inp.shared[at], inp.shared[at + 1]) = (1.0f32.to_bits(), 0);
                }
                if r.fs_front_face_off1 != 0 {
                    inp.face_ccw = seed & 1;
                    inp.shared[r.fs_front_face_off1 as usize - 1] = (seed / 2) % 3;
                }
                for v in &r.varyings[..r.num_varyings as usize] {
                    let n = name_of(&v.name);
                    for c in 0..v.components as usize {
                        {
                            let w = unit(seed, &n, c as u32);
                            inp.coeff[at + 4 * c] = w;
                            inp.coeff[at + 4 * c + 2] = w;
                        }
                    }
                    at += 4 * v.components as usize;
                }
            };
            place(&mut mine, &rec);
            let mut theirs_in = sentinel_inputs(-3.0e30);
            place(&mut theirs_in, sh);
            place_shared(&mut theirs_in, theirs_u, &|n, k| unit(seed, n, k));
            /* The control word at each record's own offset, cycling through
             * every field (`meta_case`), so a field ours reads differently
             * from pco shows. */
            place_meta(
                &mut mine,
                (rec.meta_off != yakogl::shaderc::SHC_NO_META).then_some(rec.meta_off as usize),
                seed,
            );
            place_meta(
                &mut theirs_in,
                (sh.meta_off != yakogl::shaderc::SHC_NO_META).then_some(sh.meta_off as usize),
                seed,
            );
            /* Our own program not running is a FAILURE of the gate, not a
             * refusal: it was built and would reach a board. */
            let ours = match run_binary(&image, &mine, true) {
                Ok(o) => o,
                Err(e) => {
                    diffs.push(format!("OURS UNRUNNABLE: {e}"));
                    break;
                }
            };
            /* Agreement on an empty output would be two programs writing
             * nothing: require the colours (and, from a shader that
             * discards, the pass feedback or not) -- pixel output n for each
             * location n the program stores (several outputs, each in its
             * own register: clear_mrt_300.frag), pixout0 for a declared
             * colour nothing stores. */
            let colours = ours.keys().copied().filter(|k| *k != usc_exec::FEEDBACK).collect::<Vec<_>>();
            let writes_colour = l.module.outputs.iter().any(|o| o.kind == IoKind::Colour);
            let stored: BTreeSet<u32> = l
                .module
                .instrs
                .iter()
                .filter_map(|op| match op {
                    yakogl::shaderc::online::ir::Op::StoreOutput { slot: IoSlot::FragColor(n), .. } => {
                        Some(u32::from(*n))
                    }
                    _ => None,
                })
                .collect();
            let want: Vec<u32> = match (writes_colour, stored.len()) {
                (false, _) => Vec::new(),
                (true, 0) => vec![0],
                (true, _) => stored.into_iter().collect(),
            };
            if colours != want
                || (rec.meta_off == yakogl::shaderc::SHC_NO_META && ours.contains_key(&usc_exec::FEEDBACK))
            {
                diffs.push(format!(
                    "ours writes pixel outputs {:?}, not exactly {want:?}",
                    ours.keys().collect::<Vec<_>>()
                ));
                break;
            }
            match run_binary(&pco, &theirs_in, false) {
                /* pco's program uses what the model cannot follow
                 * (predicated groups): named, and covered by the check
                 * against selection above. */
                Err(e) => {
                    diffs.push(format!("NOTE pco unrunnable by the model: {e}"));
                    break;
                }
                Ok(t) if t != ours => {
                    /* The IR on the same inputs says which of the two is
                     * GLSL's answer (a pixel, or why it has none). */
                    let front = rec.fs_front_face_off1 == 0 || {
                        let (ccw, op) = (seed & 1, (seed / 2) % 3);
                        op == 2 || ((ccw == 0) != (op == 1))
                    };
                    let by_binding: BTreeMap<u8, u32> = ours_u
                        .iter()
                        .filter(|u| u.dword_off == NO_BLOCK)
                        .map(|u| (u.binding, sampler_marker(&name_of(&u.name))))
                        .collect();
                    let ir_in = usc_exec::ir::IrInputs {
                        input: &|slot, c| match slot {
                            IoSlot::Varying(loc) => l
                                .module
                                .inputs
                                .iter()
                                .find(|io| io.location == loc)
                                .map_or(0, |io| unit(seed, io.name, c)),
                            IoSlot::PointCoord => unit(seed, "gl_PointCoord", c),
                            IoSlot::FragCoord if c >= 2 => unit(seed, "gl_FragCoord", c),
                            IoSlot::FrontFacing if front => u32::MAX,
                            _ => 0,
                        },
                        uniform: &|d| {
                            l.reflection
                                .uniforms
                                .iter()
                                .filter(|u| u.dword_off != NO_BLOCK)
                                .find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d))
                                .map_or(0, |u| unit(seed, u.name, (d - u.dword_off) as u32))
                        },
                        marker: &|b| by_binding.get(&b).copied().unwrap_or(0),
                    };
                    let ir = match usc_exec::ir::run_ir(&l.module, &ir_in) {
                        Ok(w) if w.undefined => "undefined".to_string(),
                        Ok(w) => w.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).map_or(
                            "no colour".into(),
                            |(_, lanes)| {
                                let mut px = 0u32;
                                for (c, lane) in lanes.iter().enumerate() {
                                    let x = lane.map_or(f32::NAN, |v| f32::from_bits(v.0));
                                    px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
                                }
                                format!("{px:08x}")
                            },
                        ),
                        Err(e) => format!("unrunnable: {e}"),
                    };
                    diffs.push(format!(
                        "DIFFERS (seed {seed}, face_ccw {}, op {}): ours {ours:x?} pco {t:x?} IR {ir}",
                        seed & 1,
                        (seed / 2) % 3
                    ));
                    break;
                }
                Ok(_) => {}
            }
        }
        Ok((diffs, fir))
    });
    match out {
        None => Err("the scratch arena was already borrowed".into()),
        Some(Ok((mut diffs, mut fir))) => {
            run_variants(text, &mut diffs, &mut fir);
            Ok((diffs, fir))
        }
        Some(Err(e)) => Err(e),
    }
}

/// Every blend variant the table holds for `text`, as the product gets it:
/// the base compiled through the online back end, each variant through its
/// `blend_variant` for the table's key, compared with pco's variant row --
/// the colour already in the pixel and the blend colour varied, the packed
/// pixel within one step. Each compiled variant, and the base, is released
/// (`release_shader`), so the keep region is back where it started. A
/// variant ours cannot build, or pco's the model cannot run, is named in
/// `variants_unmeasured`, not counted as agreeing.
fn run_variants(text: &str, diffs: &mut Vec<String>, fir: &mut FsIr) {
    let key = sha256(text.as_bytes());
    let rows: Vec<_> = SHC_BLOB_TABLE.iter().filter(|r| r.sha256 == key && r.blend_key != 0).collect();
    if rows.is_empty() {
        return;
    }
    let backend = &yakogl::shaderc::online::backend::SHC_ONLINE;
    let src = std::ffi::CString::new(text).expect("no NUL in a table source");
    let mut base = yakogl::shaderc::ShcShader::ZERO;
    // SAFETY: a NUL-terminated source, a valid out, no log.
    let rc = unsafe {
        (backend.compile.expect("online compile"))(
            yakogl::shaderc::SHC_STAGE_FRAGMENT,
            src.as_ptr(),
            &mut base,
            core::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        fir.variants += rows.len();
        fir.variants_unmeasured
            .extend(rows.iter().map(|r| format!("{:#x} the base did not compile ({rc})", r.blend_key)));
        return;
    }
    let name_of =
        |n: &[core::ffi::c_char]| -> String { n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect() };
    let uniforms_of = |sh: &yakogl::shaderc::ShcShader| -> Vec<yakogl::shaderc::ShcUniform> {
        if sh.num_uniforms == 0 {
            Vec::new()
        } else {
            // SAFETY: a record's `num_uniforms` rows.
            unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) }.to_vec()
        }
    };
    let image_of = |sh: &yakogl::shaderc::ShcShader| -> Vec<u8> {
        // SAFETY: a record's `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) };
        code.iter().flat_map(|w| w.to_le_bytes()).collect()
    };
    for row in rows {
        let bk = row.blend_key;
        fir.variants += 1;
        let mut v = yakogl::shaderc::ShcShader::ZERO;
        // SAFETY: `base` came from this back end; `v` is a valid out.
        let rc = unsafe { (backend.blend_variant.expect("online blend_variant"))(&base, bk, &mut v) };
        if rc != 0 {
            fir.variants_unmeasured.push(format!("{bk:#x} ours refused ({rc})"));
            continue;
        }
        let (vimage, timage) = (image_of(&v), image_of(&row.shader));
        let (vours_u, vtheirs_u) = (uniforms_of(&v), uniforms_of(&row.shader));
        let tsh = &row.shader;
        let mut same = true;
        for seed in 1..=4u32 {
            let unit = |n: &str, k: u32| -> u32 {
                if n.starts_with("pvr_layers_") {
                    return named(seed, n, k);
                }
                let x = f32::from_bits(named(seed, n, k)).abs() / 8.0;
                x.fract().to_bits()
            };
            /* The pixel in the target's format: bytes for RGBA8, halves of
             * real colour values for the float classes (random bits would be
             * NaN and infinity), a float for R32F. */
            let class = bk >> yakogl::shaderc::SHC_BLEND_KEY_RT_SHIFT;
            let half = |k: u32| u32::from(usc_exec::f32_to_f16(f32::from_bits(unit("__pixel", k)) * 2.0));
            let (pixel, pixel1) = match class {
                yakogl::shaderc::SHC_RT_F16X4 => (half(0) | half(1) << 16, half(2) | half(3) << 16),
                yakogl::shaderc::SHC_RT_F16 => (half(0), 0),
                yakogl::shaderc::SHC_RT_F32 => (unit("__pixel", 0), 0),
                _ => ((0..4).fold(0u32, |p, k| p | ((named(seed, "__pixel", k) >> 3) & 0xff) << (8 * k)), 0),
            };
            let place = |inp: &mut usc_exec::Inputs, r: &yakogl::shaderc::ShcShader| {
                let (pntc_at, mut at) = coeff_layout(r.uses_z, r.uses_w, r.uses_pntc);
                place_point_coord(inp, pntc_at, &|c| unit("gl_PointCoord", c));
                place_frag_zw(inp, r, &|c| unit("gl_FragCoord", c));
                /* gl_FragCoord's pair as an unflipped target (pco's programs
                 * have none and read y unchanged), and gl_FrontFacing's face
                 * and op, at the record's own offsets. */
                if r.fragcoord_ytf_off1 != 0 {
                    let at = r.fragcoord_ytf_off1 as usize - 1;
                    (inp.shared[at], inp.shared[at + 1]) = (1.0f32.to_bits(), 0);
                }
                if r.fs_front_face_off1 != 0 {
                    inp.face_ccw = seed & 1;
                    inp.shared[r.fs_front_face_off1 as usize - 1] = (seed / 2) % 3;
                }
                for vv in &r.varyings[..r.num_varyings as usize] {
                    let n = name_of(&vv.name);
                    for c in 0..vv.components as usize {
                        {
                            let w = unit(&n, c as u32);
                            inp.coeff[at + 4 * c] = w;
                            inp.coeff[at + 4 * c + 2] = w;
                        }
                    }
                    at += 4 * vv.components as usize;
                }
            };
            let consts = |inp: &mut usc_exec::Inputs, rows: &[yakogl::shaderc::ShcUniform], off: u8| {
                if off != yakogl::shaderc::SHC_NO_BLEND_CONSTS {
                    let at = 20 * rows.iter().filter(|u| u.dword_off == NO_BLOCK).count();
                    for k in 0..4 {
                        inp.shared[at + off as usize + k] = unit("__blend", k as u32);
                    }
                }
            };
            let run = |sh: &yakogl::shaderc::ShcShader, rows: &[yakogl::shaderc::ShcUniform], sentinel: f32| {
                let mut inp = sentinel_inputs(sentinel);
                place_shared(&mut inp, rows, &unit);
                place(&mut inp, sh);
                consts(&mut inp, rows, sh.blend_consts_off);
                place_meta(
                    &mut inp,
                    (sh.meta_off != yakogl::shaderc::SHC_NO_META).then_some(sh.meta_off as usize),
                    seed,
                );
                inp.pixout = pixel;
                inp.pixout1 = pixel1;
                inp
            };
            let ours = match run_binary(&vimage, &run(&v, &vours_u, 1.0e30), true) {
                Ok(o) => o,
                Err(e) => {
                    diffs.push(format!("BLEND {bk:#x} OURS UNRUNNABLE: {e}"));
                    same = false;
                    break;
                }
            };
            match run_binary(&timage, &run(tsh, &vtheirs_u, -3.0e30), false) {
                Err(e) => {
                    fir.variants_unmeasured.push(format!("{bk:#x} pco unrunnable by the model: {e}"));
                    same = false;
                    break;
                }
                Ok(t) => {
                    /* Within one step of the target's format: a byte, a half's
                     * bits (pco fuses a multiply-add where ours may not), a
                     * float's relative 1e-6. */
                    let close = |x: u32, y: u32| match class {
                        yakogl::shaderc::SHC_RT_F16X4 | yakogl::shaderc::SHC_RT_F16 => {
                            (0..2).all(|k| ((x >> (16 * k)) & 0xffff).abs_diff((y >> (16 * k)) & 0xffff) <= 1)
                        }
                        yakogl::shaderc::SHC_RT_F32 => {
                            let (a, b) = (f32::from_bits(x), f32::from_bits(y));
                            a == b || (a - b).abs() <= 1e-6 * a.abs().max(b.abs())
                        }
                        _ => (0..4).all(|k| ((x >> (8 * k)) & 0xff).abs_diff((y >> (8 * k)) & 0xff) <= 1),
                    };
                    let agree = t.keys().eq(ours.keys())
                        && t.iter().all(|(k, val)| {
                            ours.get(k).is_some_and(|o| if *k <= 1 { close(*o, *val) } else { o == val })
                        });
                    if !agree {
                        diffs.push(format!("BLEND {bk:#x} DIFFERS (pixel {pixel:#x}): ours {ours:x?} pco {t:x?}"));
                        same = false;
                        break;
                    }
                }
            }
        }
        if same {
            fir.variants_same += 1;
        }
        yakogl::shaderc::release_shader(&v);
    }
    yakogl::shaderc::release_shader(&base);
}

#[test]
fn what_the_usc_path_asks_of_the_machine_over_every_source_it_accepts() {
    let _scratch = scratch_lock();
    /* pco's programs take the TPU's implicit level: compared with them, ours
     * must too (`pvr_proj_lod` 0), and the switch goes back to its default
     * under the same lock, even when the survey panics */
    struct Lod;
    impl Drop for Lod {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_proj_lod(yakogl::shaderc::PVR_PROJ_LOD_DEFAULT);
        }
    }
    let _lod = Lod;
    yakogl::shaderc::pvr_set_proj_lod(0);
    let mut peak = Peak::default();
    let (mut total, mut ok) = (0usize, 0usize);
    let mut refused: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut at_ceiling: Vec<&str> = Vec::new();

    let (corpus, corpus_groups) = corpus_skeletons();
    /* The population, and the instrument's power to say no: a skeleton that
     * differs from a reference group only in its ISS routing must be absent.
     * That is exactly the `uvsw.write` the epilogue used to emit (is4 = ft0
     * where every reference write has fte), so a set that could not tell the
     * two apart would have passed it. */
    assert_eq!(corpus_groups, 651_369, "the reference corpus changed size; re-derive this gate's pins");
    let real = corpus
        .iter()
        .find(|k| k.contains("uvsw.write :uvsw_write_imm") && k.contains("is4=3"))
        .expect("no uvsw.write with is4=fte among the reference skeletons: the probe below has nothing to vary");
    println!("  probe (a reference write): {real}");
    let probe = real.replace("is4=3", "is4=0");
    assert!(!corpus.contains(&probe), "a uvsw.write with is4=ft0 is in the corpus, so this probe proves nothing");

    let (mut gate_groups, mut gate_checked) = (0usize, 0usize);
    let mut mismatches: Vec<String> = Vec::new();
    let mut unattested: BTreeMap<String, usize> = BTreeMap::new();
    let mut through: Vec<&str> = Vec::new();
    let (mut executed, mut unexecuted) = (0usize, 0usize);
    let mut prep: BTreeMap<&str, u32> = BTreeMap::new();
    let mut vs_pco: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let (mut rec_same, mut rec_diffs) = (0usize, Vec::<String>::new());
    let (mut fs_same, mut fs_diffs) = (Vec::<&str>::new(), Vec::<String>::new());
    let mut fs_dead: Vec<String> = Vec::new();
    let mut pco_alone: Vec<String> = Vec::new();
    let mut behind: Vec<String> = Vec::new();
    let mut fs_refused: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut exec_failures: Vec<String> = Vec::new();
    let (mut ir_agreed, mut ir_programs) = (0usize, 0usize);
    let mut ir_unrun: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut ir_failures: Vec<String> = Vec::new();
    let (mut ifs, mut ifs_both) = (0usize, 0usize);
    let (mut fs_ir_agreed, mut fs_ir_programs, mut fs_ir_undefined) = (0usize, 0usize, 0usize);
    let (mut fs_ir_runs, mut fs_ir_discarded) = (0usize, 0usize);
    let (mut ir_undefined, mut ir_runs) = (0usize, 0usize);
    let mut fs_ir_unrun: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut if_programs: Vec<String> = Vec::new();
    let mut beyond: Vec<String> = Vec::new();
    let (mut variants, mut variants_same) = (0usize, 0usize);
    let mut variants_unmeasured: BTreeMap<String, usize> = BTreeMap::new();

    for row in SHC_SOURCE_TABLE.iter() {
        let text = row.text.trim_end_matches('\0');
        let stage = if row.name.ends_with(".frag") { Stage::Fragment } else { Stage::Vertex };
        total += 1;
        if stage == Stage::Fragment {
            match run_fragment(text, &corpus) {
                Ok((d, fir)) => {
                    /* An if the IR model could not run is not half measured
                     * but unmeasured, and is counted with the unrunnable. */
                    let ifs_run = if fir.unrun.is_none() { fir.ifs } else { 0 };
                    if fir.unrun.is_none() {
                        fs_ir_runs += fir.runs;
                    }
                    match fir.unrun {
                        Some(e) => fs_ir_unrun.entry(e).or_default().push(row.name),
                        None => fs_ir_programs += 1,
                    }
                    fs_ir_agreed += fir.agreed;
                    variants += fir.variants;
                    variants_same += fir.variants_same;
                    for u in &fir.variants_unmeasured {
                        /* Keyed on the reason, the blend key dropped. */
                        let why = u.split_once(' ').map_or(u.as_str(), |x| x.1).to_string();
                        *variants_unmeasured.entry(why).or_default() += 1;
                    }
                    fs_ir_discarded += fir.discarded;
                    fs_ir_undefined += fir.undefined;
                    if IFS_BEYOND_THE_INPUTS.contains(&row.name) {
                        /* measured, not fully driven: its own count */
                        beyond.push(format!("{} {}/{}", row.name, fir.ifs_both, ifs_run));
                    } else {
                        ifs += ifs_run;
                        ifs_both += fir.ifs_both;
                    }
                    if ifs_run > 0 {
                        if_programs.push(format!("{} {}/{}", row.name, fir.ifs_both, fir.ifs));
                    }
                    if d.is_empty() {
                        fs_same.push(row.name)
                    } else if d.iter().all(|x| x.starts_with("NOTE ")) {
                        fs_dead.push(format!("{}: {d:?}", row.name))
                    } else if PCO_ALONE_DIFFERS.contains(&row.name)
                        && d.iter().any(|x| ir_sides_with_ours(x))
                        && d.iter().all(|x| x.starts_with("NOTE ") || x.starts_with("BLEND ") || ir_sides_with_ours(x))
                    {
                        pco_alone.push(format!("{}: {d:?}", row.name))
                    } else if TABLE_BEHIND_THE_HELPER.contains(&row.name)
                        && d.iter().any(|x| ir_sides_with_ours(x))
                        && d.iter().all(|x| x.starts_with("NOTE ") || x.starts_with("BLEND ") || ir_sides_with_ours(x))
                    {
                        behind.push(format!("{}: {d:?}", row.name))
                    } else {
                        fs_diffs.push(format!("{}: {d:?}", row.name))
                    }
                }
                Err(e) => fs_refused.entry(e).or_default().push(row.name),
            }
            continue;
        }
        match run_one(text, stage, &corpus) {
            Ok((p, g)) => {
                gate_groups += g.groups;
                gate_checked += g.checked;
                for m in g.mismatches {
                    mismatches.push(format!("{}: {m}", row.name));
                }
                let q = g.prep;
                for (k, v) in [
                    ("imm_const", q.imm_const),
                    ("imm_move", q.imm_move),
                    ("bank_move", q.bank_move),
                    ("sign_moved", q.sign_moved),
                    ("swapped", q.swapped),
                    ("mod_move", q.mod_move),
                    ("output_move", q.output_move),
                    ("plain_first", q.plain_first),
                ] {
                    *prep.entry(k).or_default() += v;
                }
                if g.record_diffs.is_empty() {
                    rec_same += 1;
                } else {
                    rec_diffs.push(format!("{}: {:?}", row.name, g.record_diffs));
                }
                let k = match &g.pco {
                    Some(Ok(())) => "agrees with pco on every output dword".to_string(),
                    Some(Err(e)) if e.starts_with("DIFFERS") => "DIFFERS from pco".to_string(),
                    Some(Err(e)) if e.starts_with("WITHIN") => "within 4 ulp of pco (operation order)".to_string(),
                    Some(Err(e)) => e.clone(),
                    None => "not compared".to_string(),
                };
                if let Some(Err(e)) = &g.pco {
                    if e.starts_with("DIFFERS") || e.starts_with("WITHIN") {
                        println!("    {}: {e}", row.name);
                    }
                }
                vs_pco.entry(k).or_default().push(row.name);
                executed += g.executed;
                unexecuted += g.unexecuted;
                for e in g.exec_failures {
                    exec_failures.push(format!("{}: {e}", row.name));
                }
                let ifs_run = if g.ir_unrun.is_none() { g.ifs } else { 0 };
                if g.ir_unrun.is_none() {
                    ir_runs += g.ir_runs;
                }
                match g.ir_unrun {
                    Some(e) => ir_unrun.entry(e).or_default().push(row.name),
                    None => ir_programs += 1,
                }
                ir_agreed += g.ir_agreed;
                ir_undefined += g.ir_undefined;
                ifs += ifs_run;
                ifs_both += g.ifs_both;
                if ifs_run > 0 {
                    if_programs.push(format!("{} {}/{}", row.name, g.ifs_both, g.ifs));
                }
                for e in g.ir_failures {
                    ir_failures.push(format!("{}: {e}", row.name));
                }
                for u in g.unattested {
                    *unattested.entry(u).or_default() += 1;
                }
                ok += 1;
                through.push(row.name);
                if p.temps > USC_MAX_TEMPS / 2 {
                    at_ceiling.push(row.name);
                }
                peak.raise(&p);
            }
            Err(e) => {
                /* Keyed on the WHOLE message: a histogram keyed on a prefix
                 * hid the reason once already (CLAUDE.md). Selection's
                 * refusals are kept whole too, since they are the next work
                 * list after ours. */
                refused.entry(e).or_default().push(row.name);
            }
        }
    }

    println!("=== what the USC path asks of the machine ===");
    println!("  sources {total}, through the whole chain {ok}");
    for (why, n) in &refused {
        let names = if why.contains("ES 3.00") || why.contains("desktop GLSL") { &n[..0] } else { &n[..] };
        println!("    {:5}  refused at: {why} {names:?}", n.len());
    }
    println!("  peak over the {ok} that got through:");
    println!("    temps          {:4}   (the encoding allows {USC_MAX_TEMPS})", peak.temps);
    println!("    shared         {:4}   (stand-in in Limits: {})", peak.shared, PLACEHOLDER.shared);
    println!("    vtx_in         {:4}   (stand-in: {})", peak.vtx_in, PLACEHOLDER.vtx_in);
    println!("    coeff          {:4}   (stand-in: {})", peak.coeff, PLACEHOLDER.coeff);
    println!("    groups         {:4}", peak.groups);
    println!("    usc_dw         {:4}", peak.usc_dw);
    println!("    varyings       {:4}   (the record holds 16)", peak.varyings);
    println!("    vs_output_size {:4}", peak.vs_output_size);
    println!("    varying name   {:4}   (the record holds 31 + NUL)", peak.name_len);
    println!("  sources above half the temps ceiling: {at_ceiling:?}");
    println!("  through the chain: {through:?}");
    println!(
        "  gate: {gate_checked} of {gate_groups} groups read back to their meaning; \
         {} mismatches; {} skeletons pco never emitted (over {corpus_groups} reference groups)",
        mismatches.len(),
        unattested.len()
    );
    for m in mismatches.iter().take(20) {
        println!("    MISMATCH {m}");
    }
    for (u, n) in &unattested {
        println!("    UNATTESTED x{n}: {u}");
    }
    println!(
        "  execution: {executed} runs of our bytes against selection's program ({} programs x 3 inputs); \
         {unexecuted} programs the interpreter could not run; {} disagreed",
        ok,
        exec_failures.len()
    );
    println!(
        "  against the IR (branches taken, not selected): {ir_agreed} runs agreed over {ir_programs} programs; \
         {} disagreed; {ir_undefined} inputs the IR leaves undefined (a NaN), not compared; not runnable by the IR model:",
        ir_failures.len()
    );
    println!(
        "  ifs: {ifs} in {} programs, {ifs_both} of them driven both ways by the inputs: {if_programs:?}",
        if_programs.len()
    );
    for (k, v) in &ir_unrun {
        println!("    {:4}  {k}: {v:?}", v.len());
    }
    for e in &ir_failures {
        println!("    IR {e}");
    }
    /* A rewrite at zero here is one this corpus never exercised: its only
     * evidence is its unit test in prepare.rs, and that is what the line
     * says rather than letting a green run imply otherwise. */
    println!("  prepare's rewrites over the corpus (0 = exercised only by its unit test): {prep:?}");
    println!("  against the reference (pco's bytes, same source, same inputs):");
    for (k, v) in &vs_pco {
        println!("    {:4}  {k}", v.len());
    }
    println!("  record interface vs pco (attributes, uniforms, block and output size): {rec_same} of {ok} the same");
    for d in rec_diffs.iter().take(20) {
        println!("    RECORD {d}");
    }
    for e in exec_failures.iter().take(20) {
        println!("    EXEC {e}");
    }
    /* Against the reference: every program either agrees, agrees within
     * rounding, or could not be run for a reason printed above -- and none
     * differs. The count that agrees is a floor, like `ok`. */
    assert_eq!(vs_pco.values().map(Vec::len).sum::<usize>(), ok, "a program was not classified against the reference");
    assert!(
        !vs_pco.contains_key("DIFFERS from pco"),
        "our program computes something other than pco's for the same source"
    );
    let agree = vs_pco.get("agrees with pco on every output dword").map_or(0, Vec::len);
    assert!(agree >= 72, "{agree} programs agree with pco bit for bit; it was 72");
    assert_eq!(rec_same, ok, "our record's interface differs from pco's for the same source: {rec_diffs:?}");
    assert_eq!(unexecuted, 0, "a program the execution check could not run is unmeasured, not passed");
    assert_eq!(executed, 3 * ok, "not every program was executed three times");
    assert!(exec_failures.is_empty(), "our bytes compute something other than selection's program");
    assert!(ir_failures.is_empty(), "our bytes compute something other than the IR means");
    assert_eq!(ir_agreed + ir_undefined, ir_runs, "an IR comparison stopped short without a failure");
    assert_eq!(
        ir_programs + ir_unrun.values().map(Vec::len).sum::<usize>(),
        ok,
        "a program was not classified against the IR"
    );
    println!(
        "  fragment shaders: {} through, pco-identical in skeleton, alignment, record and output: {:?}",
        fs_same.len(),
        fs_same
    );
    println!(
        "  fragment shaders identical to pco but for a named note (varyings pco found dead, or pco's program not runnable by the model -- then checked against selection's): {}",
        fs_dead.len()
    );
    for d in &fs_dead {
        println!("    DEAD {d}");
    }
    for d in &fs_diffs {
        println!("    FS {d}");
    }
    for (k, n) in &fs_refused {
        /* The members, not only the count: the next work list is these
         * names, and a bucket's count alone cannot say which shader a fix
         * would carry through. */
        let names = if k.contains("ES 3.00") || k.contains("desktop GLSL") { &n[..0] } else { &n[..] };
        println!("    {:5}  fragment refused at: {k} {names:?}", n.len());
    }
    println!(
        "  blend variants: {variants} rows of the table for sources that go through; {variants_same} blend \
         as pco's variant does on every input; unmeasured: {variants_unmeasured:?}"
    );
    println!(
        "  fragment shaders against the IR: {fs_ir_agreed} runs agreed over {fs_ir_programs} programs \
         ({fs_ir_discarded} of them discarded, and ours withheld the pass feedback); \
         {fs_ir_undefined} inputs the IR leaves undefined (a NaN), not compared; not runnable by the IR model:"
    );
    for (k, v) in &fs_ir_unrun {
        println!("    {:4}  {k}: {v:?}", v.len());
    }
    assert!(fs_diffs.is_empty(), "a fragment shader we build differs from pco's");
    println!("  pco alone differs, ours agreeing with the IR (PCO_ALONE_DIFFERS): {pco_alone:?}");
    for name in PCO_ALONE_DIFFERS {
        assert!(
            pco_alone.iter().any(|d| d.starts_with(&format!("{name}:"))),
            "{name} is in PCO_ALONE_DIFFERS but no longer differs that way: take it off the list"
        );
    }
    println!("  table rows behind the helper, ours agreeing with the IR (TABLE_BEHIND_THE_HELPER): {behind:?}");
    for name in TABLE_BEHIND_THE_HELPER {
        assert!(
            behind.iter().any(|d| d.starts_with(&format!("{name}:"))),
            "{name} is in TABLE_BEHIND_THE_HELPER but no longer differs that way: take it off the list"
        );
    }
    assert_eq!(ifs_both, ifs, "an if the inputs never drove both ways is half measured: {if_programs:?}");
    /* The named exceptions are still what they are: present, and still
     * short -- one that is now fully driven, or gone, leaves the list. */
    println!("  ifs beyond the survey's inputs (IFS_BEYOND_THE_INPUTS): {beyond:?}");
    for name in IFS_BEYOND_THE_INPUTS {
        let row = beyond.iter().find(|b| b.starts_with(&format!("{name} ")));
        let short =
            row.and_then(|r| r.rsplit_once(' ')).and_then(|(_, f)| f.split_once('/')).is_some_and(|(b, n)| b != n);
        assert!(short, "{name} is in IFS_BEYOND_THE_INPUTS but {row:?}: take it off the list");
    }
    assert_eq!(fs_ir_agreed + fs_ir_undefined, fs_ir_runs, "a fragment IR comparison stopped short without a failure");
    assert_eq!(
        fs_ir_programs + fs_ir_unrun.values().map(Vec::len).sum::<usize>(),
        fs_same.len() + fs_dead.len() + pco_alone.len() + behind.len() + fs_diffs.len(),
        "a fragment program was not classified against the IR"
    );
    assert!(fs_same.len() >= 104, "{} fragment shaders went through identical to pco; it was 104", fs_same.len());
    assert!(variants_same >= 468, "{variants_same} blend variants blend as pco's; it was 468 of {variants}");
    let fs_through = fs_same.len() + fs_dead.len() + pco_alone.len() + behind.len();
    assert!(fs_through >= 121, "{fs_through} fragment shaders went through; it was 121");
    /* The gate: population, zero, and nothing skipped, read together. */
    assert!(ok > 0 && gate_groups > 0, "the gate measured nothing");
    assert_eq!(gate_checked, gate_groups, "groups were emitted that the gate did not read back");
    assert!(mismatches.is_empty(), "built groups that do not mean what they were built for");
    assert!(unattested.is_empty(), "built groups whose wiring no reference group shares");

    /* The population, asserted: a peak taken over a set that quietly shrank
     * is a smaller number reported as the same fact. */
    assert!(total >= 300, "the source table shrank to {total}");
    /* A FLOOR, not a pin: this number is meant to rise, and the list of
     * refusals above is the work that raises it. It exists so the number
     * cannot quietly fall -- which is what would happen if a builder change
     * started refusing a shape it used to emit. */
    assert!(ok >= 96, "the USC path carried 96 vertex shaders through and now carries {ok}");
    /* Each peak against what holds it. These are the numbers step (3) will
     * rely on, and a corpus that grows past one of them must say so here
     * rather than in a wrong picture on the board. */
    assert!(peak.temps <= USC_MAX_TEMPS, "a program needs more temporaries than the DOUTU field can carry");
    assert!(peak.varyings <= 16, "a program has more varyings than the record holds");
    assert!(peak.name_len < 32, "a varying name does not fit the record's field");
    assert!(peak.shared <= PLACEHOLDER.shared, "the shared stand-in is below what the corpus uses");
    assert!(peak.vtx_in <= PLACEHOLDER.vtx_in, "the vtx_in stand-in is below what the corpus uses");
    assert!(peak.coeff <= PLACEHOLDER.coeff, "the coeff stand-in is below what the corpus uses");
}

/// A dynamically indexed uniform reads the element the index names, and
/// an index outside the array reads the nearest end (the IR's clamp):
/// never another uniform. The corpus reads its tables only in range, so
/// this is the clamp's one exercise.
#[test]
fn an_indexed_uniform_reads_its_element_and_clamps_outside_the_array() {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a
            .alloc_str(
                "uniform float before; uniform float t[4]; uniform float after; attribute float i; \
                 void main(){ gl_Position = vec4(t[int(i)], before, after, 1.0); }",
            )
            .unwrap();
        let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| e.msg).unwrap();
        let mut image = vec![0u8; 65536];
        let b = build_vertex(&l.module, PLACEHOLDER, a, &mut image).expect("builds");
        let t = l.reflection.uniforms.iter().find(|u| u.name == "t").expect("t is a uniform");
        let loc = l.reflection.attribs.iter().find(|x| x.name == "i").expect("i is an attribute").location;
        let read = yakogl::shaderc::online::powervr::abi::attribs_placed(&l.module).unwrap_or(0);
        for (i, want) in [(-3.0f32, 0usize), (0.0, 0), (2.0, 2), (3.0, 3), (7.0, 3), (100.0, 3)] {
            let mut inp = sentinel_inputs(1.0e30);
            for (k, s) in inp.shared.iter_mut().enumerate() {
                *s = (k as f32 * 10.0 + 1.0).to_bits();
            }
            inp.vtxin[vtxin_reg(read, loc, 0) as usize] = i.to_bits();
            let out = run_binary(&image[..b.len], &inp, true).expect("runs");
            let elem = t.dword_off as usize + want;
            assert_eq!(out.get(&0), Some(&inp.shared[elem]), "t[int({i})] reads element {want}");
        }
    })
    .unwrap();
}

/// `int / int` is exact over 32 bits, truncating toward zero, on every
/// sign: the lowering of Mesa's `nir_lower_idiv` that pco runs. A vertex
/// shader, so each result arrives as full dwords: `q / 65536` and
/// `q - (q / 65536) * 65536`, both exact as floats, rebuild q -- three
/// divides per case. A zero divisor and INT_MIN / -1 are undefined in GLSL
/// and are not judged. Every group's wiring is one pco emitted.
///
/// What this cannot see: the byte model's `frcp` is IEEE-exact, and with an
/// exact reciprocal one refinement always suffices -- dropping the second
/// passes here (checked), dropping both fails. The second is kept for the
/// hardware's approximate `rcp`, as NIR and pco keep it; only silicon
/// exercises it.
#[test]
fn an_integer_divide_is_exact_over_32_bits_on_every_sign() {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a
            .alloc_str(
                "uniform ivec2 u; void main(){ int q = u.x / u.y; int h = q / 65536; \
                 gl_Position = vec4(float(h), float(q - h * 65536), 0.0, 1.0); }",
            )
            .unwrap();
        let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| e.msg).unwrap();
        let mut image = vec![0u8; 65536];
        let b = build_vertex(&l.module, PLACEHOLDER, a, &mut image).expect("builds");
        let (corpus, _) = usc_exec::corpus_skeletons();
        let mut off = 0usize;
        while off < b.len {
            let g = decode_group(&image[..b.len], off).expect("decodes");
            let mut k = String::new();
            skeleton(&mut k, &g).unwrap();
            assert!(corpus.contains(&k), "an unattested group at {off:#x}: {k}");
            off += g.total;
        }
        let u = l.reflection.uniforms.iter().find(|u| u.name == "u").expect("u is a uniform").dword_off as usize;
        const MIN: i32 = i32::MIN;
        const MAX: i32 = i32::MAX;
        let cases = [
            (7, 2),
            (-7, 2),
            (7, -2),
            (-7, -2),
            (0, 5),
            (0, -5),
            (5, 7),
            (-5, 7),
            (6, 3),
            (-6, 3),
            (MAX, 1),
            (MAX, -1),
            (MIN, 1),
            (MIN, 2),
            (MIN, -2),
            (MIN, MAX),
            (MAX, MIN),
            (MAX, MAX),
            (MIN, MIN),
            (123_456_789, 1000),
            (-123_456_789, 1000),
            (1, MAX),
            (MAX, 65536),
            (-1, 1),
            (1_000_000_007, 97),
            (-2_000_000_000, 3),
            (65535, 65536),
            (0x4000_0000, 3),
            (99, -100),
        ];
        let mut judged = 0;
        for (x, y) in cases {
            let mut inp = sentinel_inputs(1.0e30);
            inp.shared[u] = x as u32;
            inp.shared[u + 1] = y as u32;
            let out = run_binary(&image[..b.len], &inp, true).expect("runs");
            let q = x / y;
            let h = q / 65536;
            let want = [(h as f32).to_bits(), ((q - h * 65536) as f32).to_bits()];
            assert_eq!([out[&0], out[&1]], want, "{x} / {y} = {q}");
            judged += 1;
        }
        assert_eq!(judged, cases.len());

        /* The remainder: GLSL ES 1.00 has no `%`, so the same module with
         * the first divide made an `Imod` (the IR's remainder, the
         * dividend's sign). Its result feeds the same two divides by 65536. */
        let mut instrs: Vec<yakogl::shaderc::online::ir::Op> = l.module.instrs.to_vec();
        let first = instrs
            .iter()
            .position(|op| matches!(op, yakogl::shaderc::online::ir::Op::Alu(AluOp::Idiv, _)))
            .expect("a divide");
        if let yakogl::shaderc::online::ir::Op::Alu(o, _) = &mut instrs[first] {
            *o = AluOp::Imod;
        }
        let instrs = a.alloc_copy(&instrs).unwrap();
        let m = yakogl::shaderc::online::ir::Module { instrs, ..l.module };
        let mut image = vec![0u8; 65536];
        let b = build_vertex(&m, PLACEHOLDER, a, &mut image).expect("builds");
        for (x, y) in cases {
            let mut inp = sentinel_inputs(1.0e30);
            inp.shared[u] = x as u32;
            inp.shared[u + 1] = y as u32;
            let out = run_binary(&image[..b.len], &inp, true).expect("runs");
            let r = x % y;
            let h = r / 65536;
            let want = [(h as f32).to_bits(), ((r - h * 65536) as f32).to_bits()];
            assert_eq!([out[&0], out[&1]], want, "{x} % {y} = {r}");
        }
    })
    .unwrap();
}

/// A flat varying is read from its plane's constant, coefficient
/// `start + 4 * c + 2`, one `mbyp` per component after the iterations, as
/// pco reads it -- not iterated. GLSL ES 1.00 has no `flat`, so the module's
/// second varying is marked flat in the IR. The record says so (interp 2),
/// the coefficient the iterator would use holds a decoy, and every group's
/// wiring is one pco emitted.
#[test]
fn a_flat_varying_is_its_planes_constant_term() {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a
            .alloc_str(
                "precision highp float; varying vec2 v_s; varying vec2 v_f; \
                 void main(){ gl_FragColor = vec4(v_s, v_f); }",
            )
            .unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let mut inputs: Vec<IoDecl<'_>> = l.module.inputs.to_vec();
        let f = inputs.iter().position(|io| io.name == "v_f").expect("v_f");
        inputs[f].interp = yakogl::shaderc::online::ir::INTERP_FLAT;
        let inputs = a.alloc_copy(&inputs).unwrap();
        let m = yakogl::shaderc::online::ir::Module { inputs, ..l.module };
        let rec = compile_fragment(&m, &l.reflection, a).expect("compiles");
        let vs = &rec.varyings[..rec.num_varyings as usize];
        let name = |v: &yakogl::shaderc::ShcVarying| -> String {
            v.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let vf = vs.iter().find(|v| name(v) == "v_f").expect("v_f in the record");
        assert_eq!(vf.interp, 2, "the record says flat");
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (corpus, _) = usc_exec::corpus_skeletons();
        let mut off = 0usize;
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            let mut k = String::new();
            skeleton(&mut k, &g).unwrap();
            assert!(corpus.contains(&k), "an unattested group at {off:#x}: {k}");
            off += g.total;
        }
        /* W at 0..3, v_s at 4..11, v_f at 12..19: the smooth value at
         * A (4 + 4c), the flat one at C (12 + 4c + 2), a decoy at its A. */
        let mut inp = sentinel_inputs(1.0e30);
        inp.coeff[4] = 0.25f32.to_bits();
        inp.coeff[8] = 0.5f32.to_bits();
        inp.coeff[12] = 0.0f32.to_bits();
        inp.coeff[16] = 0.0f32.to_bits();
        inp.coeff[14] = 0.75f32.to_bits();
        inp.coeff[18] = 1.0f32.to_bits();
        let out = run_binary(&image, &inp, true).expect("runs");
        let px = *out.get(&0).expect("a colour");
        let want = [0.25f32, 0.5, 0.75, 1.0].map(|v| (v * 255.0).round() as u32);
        for (c, w) in want.iter().enumerate() {
            let got = px >> (8 * c) & 0xff;
            assert!(got.abs_diff(*w) <= 1, "component {c}: {got} for {w} ({px:#x})");
        }
    })
    .unwrap();
}

/// `gl_PointCoord` is iterated as pco iterates it: `ditr` with no divide,
/// two components, in the eight coefficients after W's four (from 0 when
/// nothing iterates W), before every varying, which moves up by eight.
/// The record says so (`uses_pntc`, which the driver's coefficient program
/// reads), and iterates W only beside a varying, as pco's records do
/// (pco_shc build-new, 2026-09-24: `uses: ... pntc=1` with w=0 alone). It
/// was refused by name until the field landed (gl_test case 83 on Zephyr).
#[test]
fn gl_point_coord_is_iterated_where_pco_puts_it() {
    use yakogl::shaderc::online::powervr::chain::{build_fragment, LIMITS};
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        for (src, pntc_at, varying_at) in [
            ("precision highp float; void main(){ gl_FragColor = vec4(gl_PointCoord, 0.0, 1.0); }", 0u32, None),
            (
                "precision highp float; varying vec2 v_c; void main(){ gl_FragColor = vec4(gl_PointCoord, v_c); }",
                4,
                Some(12u32),
            ),
        ] {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            {
                let _keep = store::begin_keep().expect("keep");
                let rec = compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("{src}: {e:?}"));
                assert_eq!(rec.uses_pntc, 1, "{src}: the record carries the point-sprite iterator");
                assert_eq!(rec.uses_w, u8::from(varying_at.is_some()), "{src}: W beside a varying only");
            }
            let mut image = vec![0u8; 16 << 10];
            let b = build_fragment(&l.module, LIMITS, a, &mut image).expect("builds");
            assert!(b.uses_pntc);
            let (corpus, _) = usc_exec::corpus_skeletons();
            let mut iters = Vec::new();
            let mut off = 0usize;
            while off < b.len {
                let g = decode_group(&image[..b.len], off).expect("decodes");
                let mut k = String::new();
                skeleton(&mut k, &g).unwrap();
                assert!(corpus.contains(&k), "an unattested group at {off:#x}: {k}");
                for (_, ph, r) in g.instrs() {
                    let name = yakogl::shaderc::online::powervr::isa::ops_for(r, ph).next().map(|o| o.name);
                    if let Some(n @ ("ditr" | "ditrp")) = name {
                        iters.push((n, r.get("coff").unwrap_or(0), r.get("count").unwrap_or(0)));
                    }
                }
                off += g.total;
            }
            let mut want = vec![("ditr", pntc_at, 2)];
            if let Some(v) = varying_at {
                want.push(("ditrp", v, 2));
            }
            assert_eq!(iters, want, "{src}");
            let mut inp = sentinel_inputs(1.0e30);
            inp.coeff[pntc_at as usize] = 0.25f32.to_bits();
            inp.coeff[pntc_at as usize + 4] = 0.75f32.to_bits();
            if let Some(v) = varying_at {
                inp.coeff[v as usize] = 0.5f32.to_bits();
                inp.coeff[v as usize + 4] = 1.0f32.to_bits();
            }
            let out = run_binary(&image[..b.len], &inp, true).expect("runs");
            let px = *out.get(&0).expect("a colour");
            let want = if varying_at.is_some() { [0.25f32, 0.75, 0.5, 1.0] } else { [0.25, 0.75, 0.0, 1.0] };
            for (c, w) in want.iter().enumerate() {
                let got = px >> (8 * c) & 0xff;
                assert!(got.abs_diff((w * 255.0).round() as u32) <= 1, "{src}: component {c} of {px:#x}");
            }
        }
    })
    .unwrap();
}

/// The masked clear programs (src/pvr/blobs.rs PVR_USC_CLEAR_M*, pco's
/// clear.frag with the colour mask lowered, which the backend draws a clear
/// quad with when glColorMask leaves some channels off): in the byte model,
/// over a pixel already in the output registers, every channel the mask
/// writes takes the clear colour (from the four uniform dwords) and every
/// other keeps the pixel -- for all fourteen partial masks, RGBA8 and
/// RGBA16F, each program looked up through the backend's own table
/// (`clear_masked_program`), so a table entry out of order fails here.
#[test]
fn the_masked_clear_programs_keep_the_channels_the_mask_leaves_off() {
    use yakogl::pvr::backend::clear_masked_program;
    use yakogl::shaderc::{SHC_RT_F16, SHC_RT_F16X4, SHC_RT_F32, SHC_RT_RGBA8};
    for m in [0u8, 0xf] {
        assert!(clear_masked_program(SHC_RT_RGBA8, m).is_none(), "mask {m:#x} is no partial mask");
    }
    for class in [SHC_RT_F16, SHC_RT_F32] {
        assert!(clear_masked_program(class, 0b0101).is_none(), "a one-channel target has no masked form");
    }
    let program = |class: u32, m: u32| clear_masked_program(class, m as u8).expect("a partial mask's program");
    let clear = [0.25f32, 0.5, 0.75, 1.0];
    let old = [0x10u32, 0x20, 0x40, 0x80];
    let old_f = [0.125f32, 1.5, 2.0, 0.375];
    let half = |x: f32| u32::from(usc_exec::f32_to_f16(x));
    let bytes = |w: &[u32]| -> Vec<u8> { w.iter().flat_map(|d| d.to_le_bytes()).collect() };
    for m in 1..=14u32 {
        let mut inp = sentinel_inputs(1.0e30);
        for (k, c) in clear.iter().enumerate() {
            inp.shared[k] = c.to_bits();
        }
        inp.pixout = old.iter().enumerate().fold(0, |p, (k, b)| p | b << (8 * k));
        let out = run_binary(&bytes(program(SHC_RT_RGBA8, m)), &inp, true).expect("runs");
        let got = *out.get(&0).expect("a pixel");
        for k in 0..4 {
            let want = if m >> k & 1 != 0 { (clear[k] * 255.0).round() as u32 } else { old[k] };
            assert_eq!(got >> (8 * k) & 0xff, want, "RGBA8 mask {m:#x}, channel {k}: {got:08x}");
        }
        inp.pixout = half(old_f[0]) | half(old_f[1]) << 16;
        inp.pixout1 = half(old_f[2]) | half(old_f[3]) << 16;
        let out = run_binary(&bytes(program(SHC_RT_F16X4, m)), &inp, true).expect("runs");
        let (w0, w1) = (*out.get(&0).expect("dword 0"), *out.get(&1).expect("dword 1"));
        let got = [w0 & 0xffff, w0 >> 16, w1 & 0xffff, w1 >> 16];
        for k in 0..4 {
            let want = if m >> k & 1 != 0 { clear[k] } else { old_f[k] };
            assert_eq!(usc_exec::f16_to_f32(got[k] as u16), want, "RGBA16F mask {m:#x}, channel {k}");
        }
    }
}

/// gl_test case 8's seven checks, on the host: the case's own fragment
/// shader (the ES 1.00 twin of color_uv.frag) drawing green at alpha 0.5
/// over its opaque red pixel, built as the blend variant each check's GL
/// state asks for, run in the byte model with that pixel underneath. Each
/// must give the word case 8 expects on the board (within the case's own
/// tolerance of 2). Case 8 on the Zephyr gl_test sample is the deciding
/// check; this is the same arithmetic without the silicon.
#[test]
fn gl_test_case_8_blends_on_the_host() {
    use yakogl::shaderc::*;
    let over = shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, 0, 0);
    let cc =
        shc_blend_key(SHC_BF_CONST_COLOR, SHC_BF_INV_CONST_COLOR, SHC_BF_CONST_COLOR, SHC_BF_INV_CONST_COLOR, 0, 0);
    let cases: [(&str, u32, u32); 13] = [
        ("over", over, 0xbf008080),
        ("additive", shc_blend_key(SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, 0, 0), 0xff00ffff),
        (
            "premultiplied",
            shc_blend_key(SHC_BF_ONE, SHC_BF_INV_SRC_ALPHA, SHC_BF_ONE, SHC_BF_INV_SRC_ALPHA, 0, 0),
            0xff00ff80,
        ),
        ("replace", SHC_BLEND_KEY_REPLACE, 0x8000ff00),
        ("constant colour", cc, 0x800080bf),
        ("mask ga", SHC_BLEND_KEY_REPLACE | shc_blend_key_mask_off(0b0101), 0x8000ffff),
        ("over, alpha masked", over | shc_blend_key_mask_off(0b1000), 0xff008080),
        /* GL_FUNC_SUBTRACT / GL_FUNC_REVERSE_SUBTRACT (ES 2.0 4.1.6: s*Fs - d*Fd
         * and d*Fd - s*Fs, clamped for a UNORM target), src (0, 1, 0, 0.5)
         * over the red pixel (1, 0, 0, 1) */
        ("one, one, subtract", shc_blend_key(SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, 1, 1), 0x0000ff00),
        ("one, one, reverse subtract", shc_blend_key(SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, 2, 2), 0x800000ff),
        (
            "src_alpha, inv_src_alpha, subtract",
            shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, 1, 1),
            0x00008000,
        ),
        (
            "src_alpha, inv_src_alpha, reverse subtract",
            shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, 2, 2),
            0x40000080,
        ),
        /* GL_MIN / GL_MAX (ES 3.0): the factors ignored */
        ("min", shc_blend_key(SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, SHC_BF_ONE, 3, 3), 0x80000000),
        ("max", shc_blend_key(SHC_BF_ZERO, SHC_BF_ZERO, SHC_BF_ZERO, SHC_BF_ZERO, 4, 4), 0xff00ffff),
    ];
    let src = include_str!("../tools/shaders/gl_test_es100/color_uv.frag");
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        for (what, key, want) in cases {
            let rec = yakogl::shaderc::online::powervr::chain::compile_fragment_for(&l.module, &l.reflection, a, key)
                .unwrap_or_else(|e| panic!("{what}: {e:?}"));
            /* A plain replace does not read the pixel; everything else does. */
            let reads = u8::from(key != SHC_BLEND_KEY_REPLACE);
            assert_eq!((rec.blend_key, rec.uses_fbfetch), (key, reads), "{what}: the record");
            // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            let mut inp = sentinel_inputs(1.0e30);
            let mut at = 4usize;
            for v in &rec.varyings[..rec.num_varyings as usize] {
                let n: String = v.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
                let vals: &[f32] = if n == "v_color" { &[0.0, 1.0, 0.0, 0.5] } else { &[1.0, 1.0] };
                for (c, x) in vals.iter().enumerate().take(v.components as usize) {
                    {
                        let w = x.to_bits();
                        inp.coeff[at + 4 * c] = w;
                        inp.coeff[at + 4 * c + 2] = w;
                    }
                }
                at += 4 * v.components as usize;
            }
            if rec.blend_consts_off != SHC_NO_BLEND_CONSTS {
                for (k, x) in [0.25f32, 0.5, 0.75, 1.0].iter().enumerate() {
                    inp.shared[rec.blend_consts_off as usize + k] = x.to_bits();
                }
            }
            inp.pixout = 0xff0000ff;
            let out = run_binary(&image, &inp, true).expect("runs");
            let got = *out.get(&0).expect("a colour");
            let close = (0..4).all(|k| ((got >> (8 * k)) & 0xff).abs_diff((want >> (8 * k)) & 0xff) <= 2);
            assert!(close, "{what}: {got:#010x}, case 8 wants {want:#010x}");
        }
    })
    .unwrap();
}

/// gl_test case 91's two blends, on the host: the case's own fragment
/// shader (es100/hdrcolor.frag) with u_color = (0.5, 1.5,
/// 0, 0.5), built as the RGBA16F variant the GL core asks for (the target's
/// class in the key), run in the byte model over the case's clear colour
/// (2.0, 0.5, 0.25, 1.0) as two dwords of halves. The halves it writes, read
/// back as the case does (a quarter of each, into RGBA8), must be the bytes
/// case 91 expects. Values above 1.0 in both the source and the pixel: a
/// clamp anywhere, a UNORM read of the pixel or a lost second dword each
/// changes a channel. Case 91 on the Zephyr gl_test sample is the deciding
/// check; this is the same arithmetic without the silicon.
#[test]
fn gl_test_case_91_blends_into_rgba16f_on_the_host() {
    use yakogl::shaderc::*;
    let rt = shc_blend_key_rt(SHC_RT_F16X4);
    let cases: [(&str, u32, [f32; 4], [u32; 4]); 2] = [
        (
            "premultiplied (one, one_minus_src_alpha)",
            shc_blend_key(SHC_BF_ONE, SHC_BF_INV_SRC_ALPHA, SHC_BF_ONE, SHC_BF_INV_SRC_ALPHA, 0, 0) | rt,
            [1.5, 1.75, 0.125, 1.0],
            [96, 112, 8, 64],
        ),
        (
            "over (src_alpha, one_minus_src_alpha)",
            shc_blend_key(SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, SHC_BF_SRC_ALPHA, SHC_BF_INV_SRC_ALPHA, 0, 0) | rt,
            [1.25, 1.0, 0.125, 0.75],
            [80, 64, 8, 48],
        ),
    ];
    let half = |x: f32| u32::from(usc_exec::f32_to_f16(x));
    let src = include_str!("../tools/shaders/es100/hdrcolor.frag");
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        for (what, key, want, bytes) in cases {
            let rec = yakogl::shaderc::online::powervr::chain::compile_fragment_for(&l.module, &l.reflection, a, key)
                .unwrap_or_else(|e| panic!("{what}: {e:?}"));
            assert_eq!(
                (rec.blend_key, rec.uses_fbfetch, rec.num_outputs),
                (key, 1, 2),
                "{what}: the record (RGBA16F is two output dwords, and a blend reads the pixel)"
            );
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            let mut inp = sentinel_inputs(1.0e30);
            place_shared(&mut inp, rows, &|n: &str, k: u32| {
                assert_eq!(n, "u_color");
                [0.5f32, 1.5, 0.0, 0.5][k as usize].to_bits()
            });
            inp.pixout = half(2.0) | half(0.5) << 16;
            inp.pixout1 = half(0.25) | half(1.0) << 16;
            let out = run_binary(&image, &inp, true).expect("runs");
            let (w0, w1) = (*out.get(&0).expect("dword 0"), *out.get(&1).expect("dword 1"));
            let got: Vec<f32> =
                [w0, w0 >> 16, w1, w1 >> 16].iter().map(|h| usc_exec::f16_to_f32((h & 0xffff) as u16)).collect();
            assert_eq!(got, want, "{what}: the halves written");
            /* case 91's read-back: a quarter of each channel into RGBA8 */
            let read: Vec<u32> = got.iter().map(|x| (x * 0.25 * 255.0).round() as u32).collect();
            assert_eq!(read, bytes, "{what}: the bytes case 91 expects");
        }
    })
    .unwrap();
}

/// The `smp` fields of the first sample in a program's bytes, by name.
#[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
fn smp_fields(bytes: &[u8]) -> Vec<(String, u32)> {
    let mut off = 0usize;
    while off < bytes.len() {
        let g = decode_group(bytes, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let is_smp = yakogl::shaderc::online::powervr::isa::ops_for(r, ph).any(|o| o.name == "smp");
            if is_smp {
                let names = [
                    "fcnorm", "drc", "dmn", "chan", "lodm", "pplod", "proj", "sbmode", "nncoords", "sno", "soo", "tao",
                ];
                return names.iter().map(|n| (n.to_string(), r.get(n).unwrap_or(0))).collect();
            }
        }
        off += g.total;
    }
    panic!("no smp in the program")
}

/// `textureLod` on a 2D sampler: pco's `smp` with the level replacing the
/// implicit one (`lodm` replace, `pplod`), the level the data block's word
/// after the coordinates. Our program runs in the byte model against the IR
/// for three levels and two coordinates, the model's texture being another
/// one per whole level -- so a level dropped, put in the wrong word or
/// ignored reads other texels -- and the three levels must read three
/// different pixels (the instrument can see a level). The `smp` fields are
/// pco's own, from the table's vol.frag (a textureLod through the sampler3D
/// emulation, which samples 2D; skybox_300.frag served until native cube
/// step C made its cube the TPU's own, `dmn` 3).
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_2d_texture_lod_samples_at_the_level_it_names() {
    let src = "#version 300 es
precision highp float;
uniform sampler2D s;
uniform float level;
in vec2 uv;
out vec4 colour;
void main() { colour = textureLod(s, uv, level); }
";
    let _scratch = scratch_lock();
    let name_of =
        |n: &[core::ffi::c_char]| -> String { n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect() };
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec =
            compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("the chain refused it: {e:?}"));
        // SAFETY: the record's code and uniform rows live in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();

        /* the encoding: pco's own smp fields */
        let sky = SHC_SOURCE_TABLE.iter().find(|r| r.name == "vol.frag").expect("vol.frag in the table");
        let key = sha256(sky.text.trim_end_matches('\0').as_bytes());
        let blob = SHC_BLOB_TABLE.iter().find(|r| r.sha256 == key && r.blend_key == 0).expect("its pco program");
        // SAFETY: the table's programs are static.
        let dw = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
        let theirs: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
        assert_eq!(smp_fields(&image), smp_fields(&theirs), "our textureLod smp is not pco's");

        /* the meaning: ours against the IR, at three levels */
        let mut seen = Vec::new();
        for (level, (u, v)) in [(0.0f32, (0.3f32, 0.7f32)), (1.5, (0.3, 0.7)), (3.0, (0.3, 0.7)), (2.0, (0.9, 0.1))] {
            let val = |n: &str, c: u32| -> u32 {
                match n {
                    "level" => level.to_bits(),
                    "uv" => [u, v][c as usize].to_bits(),
                    _ => 0,
                }
            };
            let mut inp = sentinel_inputs(1.0e30);
            place_shared(&mut inp, rows, &val);
            let mut at = 4usize;
            for vy in &rec.varyings[..rec.num_varyings as usize] {
                for c in 0..vy.components as usize {
                    {
                        let w = val(&name_of(&vy.name), c as u32);
                        inp.coeff[at + 4 * c] = w;
                        inp.coeff[at + 4 * c + 2] = w;
                    }
                }
                at += 4 * vy.components as usize;
            }
            let ours = *run_binary(&image, &inp, true).expect("ours runs").get(&0).expect("a pixel");
            let ir_in = usc_exec::ir::IrInputs {
                input: &|slot, c| match slot {
                    IoSlot::Varying(loc) => {
                        l.module.inputs.iter().find(|io| io.location == loc).map_or(0, |io| val(io.name, c))
                    }
                    _ => 0,
                },
                uniform: &|d| {
                    let u = l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                    u.map_or(0, |u| val(u.name, (d - u.dword_off) as u32))
                },
                marker: &|_| sampler_marker("s"),
            };
            let want = usc_exec::ir::run_ir(&l.module, &ir_in).expect("the IR runs");
            let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).expect("a colour");
            let mut px = 0u32;
            for (c, lane) in lanes.iter().enumerate() {
                let x = f32::from_bits(lane.expect("stored").0);
                px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
            }
            assert_eq!(ours, px, "level {level} at ({u}, {v}): ours {ours:08x}, the IR {px:08x}");
            seen.push(ours);
        }
        assert!(
            seen[0] != seen[1] && seen[1] != seen[2] && seen[0] != seen[2],
            "the levels read the same texels: {seen:08x?}"
        );
    })
    .unwrap();
}

/// `textureSize` on a 2D sampler, as pco computes it from the image state
/// (usclib_tex_state_size): our program against the IR at levels 0 to 3 of
/// two samplers, over the model's image state for each (so the textype, the
/// base level and both size fields are each some value), and the sizes read
/// back through `fract(size / 256)`, exact for every size the fields hold.
/// The four answers must not all be the same (the instrument can see a
/// level and a sampler).
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn texture_size_reads_the_image_state_as_pco_does() {
    /* Two samplers whose model state covers both of usclib's paths: one a
     * STRIDE image (the base level ignored), the other not, with a base
     * level that is not 0 (added to `lod`). Chosen by name, and asserted. */
    let state = |n: &str| usc_exec::tex_state(sampler_marker(n));
    let names: Vec<String> = (0..200).map(|i| format!("s{i}")).collect();
    let stride = names.iter().find(|n| state(n)[0] & 7 == 4).expect("a STRIDE name");
    let based = names.iter().find(|n| state(n)[0] & 7 != 4 && state(n)[3] >> 28 != 0).expect("a based name");
    let src = format!(
        "#version 300 es
precision highp float;
precision highp int;
uniform sampler2D {stride};
uniform sampler2D {based};
uniform int level;
out vec4 colour;
void main() {{
    ivec2 s = textureSize({stride}, level);
    ivec2 t = textureSize({based}, level);
    colour = fract(vec4(s, t) / 256.0);
}}
"
    );
    let src = src.as_str();
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec =
            compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("the chain refused it: {e:?}"));
        // SAFETY: the record's code and uniform rows live in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let name_of = |n: &[core::ffi::c_char]| -> String {
            n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let binding_name: Vec<(u8, String)> =
            rows.iter().filter(|u| u.dword_off == NO_BLOCK).map(|u| (u.binding, name_of(&u.name))).collect();
        assert_eq!(binding_name.len(), 2, "both samplers are recorded: {binding_name:?}");
        let mut seen = Vec::new();
        for level in 0..4u32 {
            let val = |n: &str, _c: u32| -> u32 {
                if n == "level" {
                    level
                } else {
                    0
                }
            };
            let mut inp = sentinel_inputs(1.0e30);
            place_shared(&mut inp, rows, &val);
            let ours = *run_binary(&image, &inp, true).expect("ours runs").get(&0).expect("a pixel");
            let ir_in = usc_exec::ir::IrInputs {
                input: &|_, _| 0,
                uniform: &|d| {
                    let u = l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                    u.map_or(0, |u| val(u.name, (d - u.dword_off) as u32))
                },
                marker: &|b| {
                    let n = &binding_name.iter().find(|(x, _)| *x == b).expect("a recorded sampler").1;
                    sampler_marker(n)
                },
            };
            let want = usc_exec::ir::run_ir(&l.module, &ir_in).expect("the IR runs");
            let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).expect("a colour");
            let mut px = 0u32;
            for (c, lane) in lanes.iter().enumerate() {
                let x = f32::from_bits(lane.expect("stored").0);
                px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
            }
            assert_eq!(ours, px, "level {level}: ours {ours:08x}, the IR {px:08x}");
            seen.push(ours);
        }
        assert!(seen.iter().any(|p| *p != seen[0]), "every level read the same sizes: {seen:08x?}");
        let lanes_differ = seen.iter().any(|p| (p & 0xffff) != (p >> 16));
        assert!(lanes_differ, "the two samplers read the same sizes: {seen:08x?}");
    })
    .unwrap();
}

/// Our program against the IR for a fragment shader with one sampler named
/// `sampler`,
/// once per input set (`val`: a uniform or varying name and component to
/// bits); the packed pixels, each asserted equal to the IR's.
#[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
fn stacked_against_ir(sampler: &str, src: &str, inputs: &[&InputFn<'_>]) -> Vec<u32> {
    stacked_against_ir_with(sampler, src, inputs, false, false)
}

/// [`stacked_against_ir`], compiled with the native cube (`pvr_native_cube`)
/// and the in-shader projective divide (`pvr_proj_divide`) on or off. The switch is process-wide: set under the compile lock and
/// cleared by a guard, even when the comparison panics.
#[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
fn stacked_against_ir_with(
    sampler: &str,
    src: &str,
    inputs: &[&InputFn<'_>],
    native: bool,
    proj_divide: bool,
) -> Vec<u32> {
    let _scratch = scratch_lock();
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
            yakogl::shaderc::pvr_set_proj_divide(0);
        }
    }
    let _off = Off;
    yakogl::shaderc::pvr_set_native_cube(i32::from(native));
    yakogl::shaderc::pvr_set_proj_divide(i32::from(proj_divide));
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec =
            compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("the chain refused it: {e:?}"));
        // SAFETY: the record's code and uniform rows live in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let name_of = |n: &[core::ffi::c_char]| -> String {
            n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let mut out = Vec::new();
        for val in inputs {
            let mut inp = sentinel_inputs(1.0e30);
            place_shared(&mut inp, rows, val);
            let (pntc_at, mut at) = coeff_layout(rec.uses_z, rec.uses_w, rec.uses_pntc);
            place_point_coord(&mut inp, pntc_at, &|c| val("gl_PointCoord", c));
            place_frag_zw(&mut inp, &rec, &|c| val("gl_FragCoord", c));
            for vy in &rec.varyings[..rec.num_varyings as usize] {
                for c in 0..vy.components as usize {
                    {
                        let w = val(&name_of(&vy.name), c as u32);
                        inp.coeff[at + 4 * c] = w;
                        inp.coeff[at + 4 * c + 2] = w;
                    }
                }
                at += 4 * vy.components as usize;
            }
            let front = val("gl_FrontFacing", 0) != 0;
            if rec.fs_front_face_off1 != 0 {
                inp.face_ccw = u32::from(!front);
                inp.shared[rec.fs_front_face_off1 as usize - 1] = 0;
            }
            let ours = *run_binary(&image, &inp, true).expect("ours runs").get(&0).expect("a pixel");
            let ir_in = usc_exec::ir::IrInputs {
                input: &|slot, c| match slot {
                    IoSlot::Varying(loc) => {
                        l.module.inputs.iter().find(|io| io.location == loc).map_or(0, |io| val(io.name, c))
                    }
                    IoSlot::PointCoord => val("gl_PointCoord", c),
                    IoSlot::FragCoord if c >= 2 => val("gl_FragCoord", c),
                    IoSlot::FrontFacing if front => u32::MAX,
                    _ => 0,
                },
                uniform: &|d| {
                    let u = l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                    u.map_or(0, |u| val(u.name, (d - u.dword_off) as u32))
                },
                marker: &|_| sampler_marker(sampler),
            };
            let want = usc_exec::ir::run_ir(&l.module, &ir_in).expect("the IR runs");
            let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).expect("a colour");
            let mut px = 0u32;
            for (c, lane) in lanes.iter().enumerate() {
                let x = f32::from_bits(lane.expect("stored").0);
                px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
            }
            assert_eq!(ours, px, "input {}: ours {ours:08x}, the IR {px:08x}", out.len());
            out.push(ours);
        }
        out
    })
    .unwrap()
}

/// A sampler name whose model image (`tex_state`) is at least `rows` texels
/// tall, so a stacked face or layer is tall enough that the half-texel
/// clamp leaves the coordinate free: on a model image a few texels tall the
/// clamp collapses (lo > hi) and every coordinate reads one texel, which
/// hid a wrong face coordinate from the cube test.
#[cfg(not(feature = "report-es2"))] // used only by the tests here that compile GLSL ES 3.00
fn tall_sampler(rows: u32) -> String {
    (0..500)
        .map(|i| format!("s{i}"))
        .find(|n| usc_exec::tex_size_2d(usc_exec::tex_state(sampler_marker(n)), 0)[1] >= rows)
        .expect("a sampler name with a tall enough model image")
}

/// A 2D array look-up through the stacked 2D image (glsl2usc's pvr_arrUV),
/// the layer count read from `pvr_layers_s` -- declared by the test here,
/// standing in for the member the front end is to declare -- with the
/// implicit level and a replacing one, at layers below 0, between layers,
/// in range and past the count, averaged so no channel saturates. Ours
/// equals the IR at each, and the layers read different texels (the
/// instrument can see a layer).
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_2d_array_samples_its_layer_in_the_stacked_image() {
    let name = tall_sampler(4 * 256);
    let src = "#version 300 es
precision highp float;
uniform float pvr_layers_s;
uniform highp sampler2DArray s;
uniform float layer;
uniform float level;
in vec2 uv;
out vec4 colour;
void main() { colour = 0.5 * (texture(s, vec3(uv, layer)) + textureLod(s, vec3(uv.yx, layer), level)); }
"
    .replace("pvr_layers_s", &format!("pvr_layers_{name}"))
    .replace("(s,", &format!("({name},"))
    .replace(" s;", &format!(" {name};"));
    let layers_name = format!("pvr_layers_{name}");
    let cases: Vec<BoxedInput> = [(-1.0f32, 0.0f32), (0.0, 1.0), (0.4, 0.0), (0.6, 2.0), (2.0, 1.0), (5.0, 0.0)]
        .into_iter()
        .map(|(layer, level)| -> BoxedInput {
            let layers_name = layers_name.clone();
            Box::new(move |n: &str, c: u32| match n {
                n if n == layers_name => 4.0f32.to_bits(),
                "layer" => layer.to_bits(),
                "level" => level.to_bits(),
                "uv" => [0.3f32, 0.8][c as usize].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert!(distinct.len() >= 4, "the layers read too few distinct texels: {px:08x?}");
}

/// A cube look-up through the stacked 2D image (glsl2usc's pvr_cubeUV):
/// one direction toward each face, of each sign, and ties between axes
/// (the helper's >= decides them), with the implicit level and a replacing
/// one, averaged so no channel saturates and hides a sample. Ours equals
/// the IR at each, and the six faces read six different pixels.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_cube_samples_the_face_its_direction_names() {
    let name = tall_sampler(6 * 256);
    let src = "#version 300 es
precision highp float;
uniform highp samplerCube s;
uniform vec3 dir;
uniform float level;
out vec4 colour;
void main() { colour = 0.5 * (texture(s, dir) + textureLod(s, dir.zxy, level)); }
"
    .replace("(s,", &format!("({name},"))
    .replace(" s;", &format!(" {name};"));
    let dirs = [
        [1.0f32, 0.2, -0.3],
        [-1.0, 0.3, 0.2],
        [0.1, 0.9, -0.4],
        [0.2, -0.8, 0.3],
        [-0.3, 0.2, 0.7],
        [0.25, -0.1, -0.6],
        [0.5, 0.5, 0.5],
        [-0.5, 0.5, -0.5],
    ];
    let cases: Vec<BoxedInput> = dirs
        .iter()
        .enumerate()
        .map(|(i, d)| -> BoxedInput {
            let d = *d;
            Box::new(move |n: &str, c: u32| match n {
                "dir" => d[c as usize].to_bits(),
                "level" => (i as f32 * 0.7).to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &src, &refs);
    let faces: std::collections::BTreeSet<u32> = px[..6].iter().copied().collect();
    assert_eq!(faces.len(), 6, "the six faces did not read six different pixels: {px:08x?}");
}

/// A cube look-up leaves u to the sampler: a face spans the stacked image's
/// whole width, so the sampler's wrap in u is the face's own, and GLES2 cube
/// maps are not seamless (REPEAT reads the opposite edge of the same face,
/// dEQP's reference). The level-aware inset clamped u too and
/// texture.vertex.cube.*_repeat failed by 18 over 16 on the AM62P
/// (am62p-board, a60104c2). At the level where a face is 2..4 rows the inset
/// is 1/8..1/4, several of the model's 64 texels: a direction on the face's
/// u edge and one at u = inset read the same texel exactly when u is
/// clamped. v keeps its clamp (the stack's next rows are another face's).
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_cube_look_up_leaves_u_to_the_samplers_wrap() {
    let name = tall_sampler(6 * 256);
    let face = usc_exec::tex_size_2d(usc_exec::tex_state(sampler_marker(&name)), 0)[1] as f32 / 6.0;
    let level = face.log2().floor() - 1.0;
    let h = (face * (-level).exp2()).max(1.0);
    let inset = 0.5 / h;
    assert!((1.0 / 64.0..=0.25).contains(&inset), "the inset spans model texels: {inset}");
    let src = "#version 300 es
precision highp float;
uniform highp samplerCube s;
uniform vec3 dir;
uniform float level;
out vec4 colour;
void main() { colour = textureLod(s, dir, level); }
"
    .replace("(s,", &format!("({name},"))
    .replace(" s;", &format!(" {name};"));
    /* +X: u = -z / x * 0.5 + 0.5: the edge (u = 0), then u = inset */
    let dirs = [[1.0f32, 0.0, 1.0], [1.0, 0.0, 1.0 - 2.0 * inset]];
    let cases: Vec<BoxedInput> = dirs
        .iter()
        .map(|d| -> BoxedInput {
            let d = *d;
            Box::new(move |n: &str, c: u32| match n {
                "dir" => d[c as usize].to_bits(),
                "level" => level.to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &src, &refs);
    assert_ne!(px[0], px[1], "u on the face's edge read the inset's texel: u was clamped: {px:08x?}");
}

/// `texture(s, p, bias)`: pco's `smp` with the bias in the data block's
/// word after the coordinates, `lodm` bias and `pplod` (pco_nir_tex.c
/// pco_emit_nir_smp) -- the textureLod form with one field changed. The
/// values cannot tell a bias from a replaced level (the models' implicit
/// level is 0, so a bias b reads level b either way), so the fields are
/// pinned against pco's: pco_shc (build-new) on `o = texture(t, uv, b)`
/// emits the smp_exta of its textureLod with `lodm` bias (2026-09-24), and
/// the test compares with the table's textureLod smp so changed. The values: ours against
/// the IR on a 2D sampler, a cube and a 2D array (the stacked images, where
/// the bias applies to the stacked image's implicit level), at biases that
/// read different levels, which read different pixels.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_bias_is_added_to_the_implicit_level_in_the_data_block() {
    let src2d = "#version 300 es
precision highp float;
uniform highp sampler2D s;
uniform float bias;
in vec2 uv;
out vec4 colour;
void main() { colour = texture(s, uv, bias); }
";
    let biases = [0.0f32, 1.25, 2.0, 3.5];
    let cases: Vec<BoxedInput> = biases
        .into_iter()
        .map(|b| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match n {
                "bias" => b.to_bits(),
                "uv" => [0.3f32, 0.7][c as usize].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir("s", src2d, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 4, "four biases read the same pixel: {px:08x?}");

    /* the encoding: pco's bias fields (the lock scoped: the stacked
     * checks below take it themselves) */
    {
        let _scratch = scratch_lock();
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src2d).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
            // SAFETY: the record's code lives in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let ours: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            /* pco's textureLod smp (the table's vol.frag, 2D) with `lodm`
             * bias -- 1 in isa_tables.rs's lodm enum -- in place of replace:
             * the one field pco_shc's bias program differs in. */
            let sky = SHC_SOURCE_TABLE.iter().find(|r| r.name == "vol.frag").expect("vol.frag in the table");
            let key = sha256(sky.text.trim_end_matches('\0').as_bytes());
            let blob = SHC_BLOB_TABLE.iter().find(|r| r.sha256 == key && r.blend_key == 0).expect("its pco program");
            // SAFETY: the table's programs are static.
            let dw = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
            let theirs: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
            let pco: Vec<(String, u32)> =
                smp_fields(&theirs).into_iter().map(|(n, x)| if n == "lodm" { (n, 1) } else { (n, x) }).collect();
            assert_eq!(smp_fields(&ours), pco, "our bias smp is not pco's");
        })
        .unwrap();
    }

    /* the stacked images: a cube and a 2D array */
    let name = tall_sampler(6 * 256);
    let cube = "#version 300 es
precision highp float;
uniform highp samplerCube s;
uniform vec3 dir;
uniform float bias;
out vec4 colour;
void main() { colour = texture(s, dir, bias); }
"
    .replace("(s,", &format!("({name},"))
    .replace(" s;", &format!(" {name};"));
    let cases: Vec<BoxedInput> = biases
        .into_iter()
        .map(|b| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match n {
                "bias" => b.to_bits(),
                "dir" => [0.2f32, -0.8, 0.3][c as usize].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &cube, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 4, "a cube: four biases read the same pixel: {px:08x?}");

    let array = "#version 300 es
precision highp float;
uniform float pvr_layers_s;
uniform highp sampler2DArray s;
uniform float bias;
in vec2 uv;
out vec4 colour;
void main() { colour = texture(s, vec3(uv, 1.0), bias); }
"
    .replace("pvr_layers_s", &format!("pvr_layers_{name}"))
    .replace("(s,", &format!("({name},"))
    .replace(" s;", &format!(" {name};"));
    let layers_name = format!("pvr_layers_{name}");
    let cases: Vec<BoxedInput> = biases
        .into_iter()
        .map(|b| -> BoxedInput {
            let layers_name = layers_name.clone();
            Box::new(move |n: &str, c: u32| match n {
                n if n == layers_name => 4.0f32.to_bits(),
                "bias" => b.to_bits(),
                "uv" => [0.3f32, 0.8][c as usize].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &array, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 4, "an array: four biases read the same pixel: {px:08x?}");
}

/// The ES 1.00 spellings of a bias, `texture2D(s, uv, b)`,
/// `textureCube(s, d, b)` and `texture2DProj(s, p, b)` (which the front
/// end writes as the divide and then the 2D bias, builtin_call.rs), reach
/// the same selection; a bias the back end has no form for -- ES 3.00's
/// `textureProj(s, p, b)`, whose data block would need a fourth word, or
/// one on a 3D texture -- is refused by name, not sampled at the implicit
/// level.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_bias_in_es100_compiles_and_one_without_a_form_is_refused() {
    let _scratch = scratch_lock();
    for src in [
        "precision highp float; uniform sampler2D s; uniform float b; varying vec2 uv; \
         void main(){ gl_FragColor = texture2D(s, uv, b); }",
        "precision highp float; uniform samplerCube s; uniform float b; varying vec3 d; \
         void main(){ gl_FragColor = textureCube(s, d, b); }",
        "precision highp float; uniform sampler2D s; uniform float b; varying vec3 uv; \
         void main(){ gl_FragColor = texture2DProj(s, uv, b); }",
    ] {
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            compile_fragment(&l.module, &l.reflection, a).unwrap_or_else(|e| panic!("{src}: {e:?}"));
        })
        .unwrap();
    }
    const WHY: &str = "a texture fetch with a projective level or bias, a bias on a 3D texture";
    for src in [
        "#version 300 es\nprecision highp float; uniform highp sampler2D s; uniform float b; in vec3 p; \
         out vec4 o; void main(){ o = textureProj(s, p, b); }",
        "#version 300 es\nprecision highp float; uniform highp sampler3D s; uniform float b; in vec3 p; \
         out vec4 o; void main(){ o = texture(s, p, b); }",
    ] {
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let err = compile_fragment(&l.module, &l.reflection, a).err();
            assert!(format!("{err:?}").contains(WHY), "{src}: {err:?}");
        })
        .unwrap();
    }
}

/// `pvr_proj_lod`: an implicit-level sample of a smooth varying read as it is
/// samples with `lodm` gradients, and its gradient words are the planes'
/// derivatives -- d/dx = (A_P - u A_Q) / Q and d/dy = (B_P - u B_Q) / Q per
/// coordinate, P the component's plane and Q the W plane, A and B each
/// plane's first two coefficients -- in pco's order (d/dx, d/dy per
/// coordinate). Checked word by word on the program's bytes run in the model,
/// with every coefficient distinct, so a slope read from the wrong plane or
/// term, or two words swapped, shows. Mode 0 keeps the implicit level (no
/// gradients); mode 1 does the 2D look-up only; a coordinate computed from
/// the varying, or a flat varying, keeps the implicit level in every mode.
#[test]
fn an_implicit_level_sample_of_a_varying_takes_its_plane_gradients() {
    let es100 = |sampler: &str, ty: &str, call: &str, coord: &str| {
        format!(
            "precision mediump float; uniform {sampler} s; varying {ty} v; \
             void main(){{ gl_FragColor = {call}(s, {coord}) * 0.5 + 0.25; }}"
        )
    };
    #[cfg_attr(feature = "report-es2", allow(unused_mut))]
    let mut shaders: Vec<(String, u32, u32)> = vec![
        /* (source, coordinates, the lowest mode that gives gradients; 3 = none) */
        (es100("sampler2D", "vec2", "texture2D", "v"), 2, 1),
        (es100("sampler2D", "vec2", "texture2D", "v.yx"), 2, 1),
        (es100("samplerCube", "vec3", "textureCube", "v"), 3, 2),
        (es100("sampler2D", "vec2", "texture2D", "v * 0.5"), 2, 3),
    ];
    #[cfg(not(feature = "report-es2"))] // GLSL ES 3.00
    shaders.extend([
        (
            "#version 300 es\nprecision highp float; uniform sampler2D s; in vec2 v; out vec4 o; \
             void main(){ o = texture(s, v); }"
                .to_string(),
            2,
            1,
        ),
        (
            "#version 300 es\nprecision highp float; uniform sampler2D s; flat in vec2 v; out vec4 o; \
             void main(){ o = texture(s, v); }"
                .to_string(),
            2,
            3,
        ),
    ]);
    for (src, n, lowest) in &shaders {
        for mode in 0..=2 {
            let _scratch = scratch_lock();
            yakogl::shaderc::pvr_set_proj_lod(mode);
            let got = store::with_scratch(|a| {
                let _keep = store::begin_keep().expect("keep");
                let text = a.alloc_str(src).unwrap();
                let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
                let rec = compile_fragment(&l.module, &l.reflection, a);
                rec.map(|rec| {
                    // SAFETY: the record's code and uniform rows live in `keep`.
                    let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
                    let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
                    let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
                    let mut inp = sentinel_inputs(1.0e30);
                    place_shared(&mut inp, rows, &|_, _| 0);
                    for (i, c) in inp.coeff.iter_mut().enumerate().take(64) {
                        *c = (0.75 + 0.125 * i as f32 + 0.001 * (i * i) as f32).to_bits();
                    }
                    let (_, at) = coeff_layout(rec.uses_z, rec.uses_w, rec.uses_pntc);
                    let _ = usc_exec::take_gradients();
                    run_binary(&image, &inp, true).expect("runs");
                    (usc_exec::take_gradients(), inp.coeff.clone(), at, rec.uses_z != 0)
                })
            })
            .unwrap();
            yakogl::shaderc::pvr_set_proj_lod(yakogl::shaderc::PVR_PROJ_LOD_DEFAULT);
            let (grads, coeff, at, uses_z) = got.unwrap_or_else(|e| panic!("mode {mode}: refused: {e:?}\n{src}"));
            if (mode as u32) < *lowest {
                assert!(grads.is_empty(), "mode {mode}: gradients where the implicit level was meant\n{src}");
                continue;
            }
            assert_eq!(grads.len(), 1, "mode {mode}: one gradient sample\n{src}");
            let f = |i: usize| f32::from_bits(coeff[i]);
            /* W's plane: straight after Z's when there is one; the model's
             * no-divide iterate of it is its first coefficient */
            let w = if uses_z { 4 } else { 0 };
            let (q, wa, wb) = (f(w), f(w), f(w + 1));
            let rq = 1.0 / q;
            /* the swizzle picks which varying component each coordinate is */
            let comp = |k: usize| if src.contains("v.yx") { 1 - k } else { k };
            let mut want = Vec::new();
            for k in 0..*n as usize {
                let p = at + 4 * comp(k);
                let u = f(p);
                want.push(((f(p) + -(u * wa)) * rq).to_bits());
                want.push(((f(p + 1) + -(u * wb)) * rq).to_bits());
            }
            let close = |x: u32, y: u32| {
                (f32::from_bits(x) - f32::from_bits(y)).abs() <= 1e-5 * f32::from_bits(y).abs().max(1.0)
            };
            assert!(
                grads[0].len() == want.len() && grads[0].iter().zip(&want).all(|(g, w)| close(*g, *w)),
                "mode {mode}: gradients {:?}, want {:?}\n{src}",
                grads[0].iter().map(|x| f32::from_bits(*x)).collect::<Vec<_>>(),
                want.iter().map(|x| f32::from_bits(*x)).collect::<Vec<_>>()
            );
        }
    }
}

/// The c = 0.5 remap of a sampler-state variant (`online::lodkey`) as the
/// PowerVR compiles it, run on the host USC model: the level word each
/// explicit-level `smp` of the vertex shader receives is `lod <= 0.5 ? -1 :
/// lod`, bit for bit, for a 2D and a cube look-up; the program without the
/// remap passes the level through. What it costs, printed (`--nocapture`).
#[test]
fn the_mag_switch_remap_runs_on_the_usc_model() {
    use yakogl::shaderc::online::lodkey::remap_mag_switch;
    let shaders = [
        (
            "2D",
            "attribute vec4 a; uniform highp float u_lod; uniform sampler2D s; varying vec4 v; \
             void main(){ v = texture2DLod(s, a.xy, u_lod); gl_Position = a; }",
        ),
        (
            "cube",
            "attribute vec4 a; uniform highp float u_lod; uniform samplerCube s; varying vec4 v; \
             void main(){ v = textureCubeLod(s, a.xyz, u_lod); gl_Position = a; }",
        ),
    ];
    for (name, src) in shaders {
        let mut cost = [(0usize, 0usize, 0u16); 2];
        for (arm, mask) in [0u32, 1].into_iter().enumerate() {
            let _scratch = scratch_lock();
            store::with_scratch(|a| {
                let text = a.alloc_str(src).unwrap();
                let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| e.msg).unwrap();
                let l = remap_mag_switch(a, l, mask).map_err(|e| e.msg).unwrap();
                let mut image = vec![0u8; 65536];
                let b = build_vertex(&l.module, PLACEHOLDER, a, &mut image).expect("builds");
                let (mut groups, mut off) = (0usize, 0usize);
                while off < b.len {
                    let Ok(g) = decode_group(&image[..b.len], off) else { break };
                    if g.total == 0 {
                        break;
                    }
                    groups += 1;
                    off += g.total;
                }
                cost[arm] = (b.len / 4, groups, b.temps);
                let lod = l.reflection.uniforms.iter().find(|u| u.name == "u_lod").expect("u_lod").dword_off as usize;
                let loc = l.reflection.attribs.iter().find(|x| x.name == "a").expect("a").location;
                let read = yakogl::shaderc::online::powervr::abi::attribs_placed(&l.module).unwrap_or(0);
                for lambda in [0.0f32, 0.25, 0.5, 0.75, 1.0, 1.5, 0.192_645, -1.5] {
                    /* the one sampler's descriptor, then the block (place_shared) */
                    let mut inp = sentinel_inputs(1.0e30);
                    inp.shared[0..4].copy_from_slice(&usc_exec::tex_state(sampler_marker("s")));
                    inp.shared[20 + lod] = lambda.to_bits();
                    for (k, c) in [0.3f32, -0.2, 1.0, 1.0].into_iter().enumerate() {
                        inp.vtxin[vtxin_reg(read, loc, k as u32) as usize] = c.to_bits();
                    }
                    let _ = usc_exec::take_levels();
                    run_binary(&image[..b.len], &inp, true).expect("runs");
                    let levels = usc_exec::take_levels();
                    let want = if mask != 0 && lambda <= 0.5 { -1.0f32 } else { lambda };
                    assert_eq!(levels, [want.to_bits()], "{name}, mask {mask}, lambda {lambda}");
                }
            })
            .unwrap();
        }
        println!(
            "  {name:5} vertex shader: dwords {} -> {}, groups {} -> {}, temps {} -> {}",
            cost[0].0, cost[1].0, cost[0].1, cost[1].1, cost[0].2, cost[1].2
        );
        assert!(cost[1].1 > cost[0].1 && cost[1].1 <= cost[0].1 + 6, "{name}: the remap costs a few groups");
    }
}

/// What `pvr_proj_lod` costs a program: its size, groups and temporaries
/// with the switch off and at the default, for the look-ups it changes -- a
/// textured quad as Slint and gl_test draw it, dEQP's mipmap shader, two
/// look-ups of the same varying, a cube. Printed (`--nocapture`) and pinned
/// by an upper bound equal to the count today, so a change that makes the
/// gradients dearer is seen.
#[test]
fn what_exact_gradients_cost_a_program() {
    let shaders = [
        (
            "textured quad",
            "precision mediump float; uniform sampler2D tex; uniform vec4 color; varying vec2 uv; \
          void main(){ gl_FragColor = texture2D(tex, uv) * color; }",
        ),
        (
            "dEQP mipmap",
            "precision mediump float; uniform lowp sampler2D s; uniform mediump vec4 cs; uniform mediump vec4 cb; \
          varying mediump vec2 v; void main(){ gl_FragColor = vec4(texture2D(s, v)) * cs + cb; }",
        ),
        (
            "two look-ups (2)",
            "precision mediump float; uniform sampler2D a; uniform sampler2D b; varying vec2 uv; \
          void main(){ gl_FragColor = texture2D(a, uv) + texture2D(b, uv); }",
        ),
        (
            "cube",
            "precision mediump float; uniform samplerCube s; varying vec3 d; \
          void main(){ gl_FragColor = textureCube(s, d); }",
        ),
    ];
    for (name, src) in shaders {
        let cost = |mode: i32| {
            let _scratch = scratch_lock();
            yakogl::shaderc::pvr_set_proj_lod(mode);
            let r = store::with_scratch(|a| {
                let _keep = store::begin_keep().expect("keep");
                let text = a.alloc_str(src).unwrap();
                let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
                let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
                // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
                let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
                let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
                let (mut groups, mut off) = (0usize, 0usize);
                while off < image.len() {
                    let Ok(g) = decode_group(&image, off) else { break };
                    if g.total == 0 {
                        break;
                    }
                    groups += 1;
                    off += g.total;
                }
                (rec.usc_dw, groups, rec.temps)
            })
            .unwrap();
            yakogl::shaderc::pvr_set_proj_lod(yakogl::shaderc::PVR_PROJ_LOD_DEFAULT);
            r
        };
        let (off, on) = (cost(0), cost(yakogl::shaderc::PVR_PROJ_LOD_DEFAULT));
        println!("  {name:14} dwords {} -> {}, groups {} -> {}, temps {} -> {}", off.0, on.0, off.1, on.1, off.2, on.2);
        assert!(on.1 > off.1, "{name}: the switch changed nothing");
        /* 23 groups for the first look-up of a 2D varying -- W's no-divide
         * iterate with its fences (8), 1/W, W's two slopes, and per
         * coordinate its two slopes, two fmads and two scalings -- 29 for a
         * cube's three coordinates; a later look-up of the same varying
         * reuses them and adds only its gradient words (4 for 2D). A
         * program whose vertex shader writes a constant W pays nothing
         * (backend.rs a_constant_w_program_samples_as_the_switch_off_does). */
        let lookups = if name.ends_with("(2)") { 2 } else { 1 };
        let coords = if name == "cube" { 3 } else { 2 };
        let bound = 11 + 6 * coords + 2 * coords * (lookups - 1);
        assert!(on.1 <= off.1 + bound, "{name}: the gradients cost {} groups, over {bound}", on.1 - off.1);
    }
}

/// `texture2DProj` with `pvr_proj_divide`: the divide in the shader and the
/// quotient sampled without `smp`'s `proj` (the TPU's level then comes from
/// the quotient's derivatives; mipmap.2d.projected.* on the AM62P drew
/// shifted level boundaries with the hardware's divide, a60104c2). Both
/// forms equal the IR at every input, at divisors that move the sample
/// across texels, and the switch changes the program (three more
/// instructions: a reciprocal and two products).
#[cfg(not(feature = "report-es2"))] // stacked_against_ir_with is compiled only without report-es2
#[test]
fn texture2d_proj_divides_in_the_shader_with_pvr_proj_divide() {
    let name = tall_sampler(64);
    let src = "precision highp float; uniform sampler2D s; varying vec3 p; \
               void main(){ gl_FragColor = texture2DProj(s, p); }"
        .replace("(s,", &format!("({name},"))
        .replace(" s;", &format!(" {name};"));
    let cases: Vec<BoxedInput> = [(0.3f32, 0.7f32, 1.0f32), (0.3, 0.7, 2.0), (0.9, 0.2, 0.5), (0.15, 0.45, 3.0)]
        .into_iter()
        .map(|(x, y, q)| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match (n, c) {
                ("p", 0) => x.to_bits(),
                ("p", 1) => y.to_bits(),
                ("p", 2) => q.to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let hw = stacked_against_ir_with(&name, &src, &refs, false, false);
    let sh = stacked_against_ir_with(&name, &src, &refs, false, true);
    assert_eq!(hw, sh, "the two divides read different texels");
    let distinct: std::collections::BTreeSet<u32> = sh.iter().copied().collect();
    assert!(distinct.len() >= 3, "the divisors did not move the sample: {sh:08x?}");
    let len = |on: bool| {
        let _scratch = scratch_lock();
        yakogl::shaderc::pvr_set_proj_divide(i32::from(on));
        let n = store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(&src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            compile_fragment(&l.module, &l.reflection, a).expect("compiles").usc_dw
        })
        .unwrap();
        yakogl::shaderc::pvr_set_proj_divide(0);
        n
    };
    let (off, on) = (len(false), len(true));
    assert!(on > off, "pvr_proj_divide did not change the program: {off} dwords either way");
}

/// `texelFetch` on a 2D sampler: pco's `smp` form (the coordinate in texels,
/// `nncoords`, and the level replaced -- the fields equal pco's fetch in the
/// table's ssao_300.frag), but at the texel's centre, where every filter
/// returns the texel exactly; pco passes the integer corner, which a LINEAR
/// sampler blends (gl_test case 48). The byte model refuses an unnormalised
/// sample that is not at a centre, so a fetch at the corner fails this test
/// rather than passing on a nearest-texel model. Ours equals the IR at four
/// texels and levels, which read four different pixels.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_texel_fetch_reads_the_texel_at_its_centre() {
    let src = "#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D s;
uniform ivec2 p;
uniform int lvl;
out vec4 colour;
void main() { colour = 0.5 * (texelFetch(s, p, lvl) + texelFetch(s, p + ivec2(1, 0), lvl)); }
";
    let cases: Vec<BoxedInput> = [([0i32, 0i32], 0i32), ([5, 9], 0), ([5, 9], 2), ([63, 1], 1)]
        .into_iter()
        .map(|(pp, l)| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match n {
                "p" => pp[c as usize] as u32,
                "lvl" => l as u32,
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir("s", src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 4, "four texels read the same pixel: {px:08x?}");

    /* the encoding: pco's fetch fields */
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        // SAFETY: the record's code lives in `keep`; the table's is static.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let ours: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let ssao = SHC_SOURCE_TABLE.iter().find(|r| r.name == "ssao_300.frag").expect("ssao_300.frag in the table");
        let key = sha256(ssao.text.trim_end_matches('\0').as_bytes());
        let blob = SHC_BLOB_TABLE.iter().find(|r| r.sha256 == key && r.blend_key == 0).expect("its pco program");
        let dw = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
        let theirs: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
        let fetch_fields = |bytes: &[u8]| -> Vec<(String, u32)> {
            let mut off = 0usize;
            while off < bytes.len() {
                let g = decode_group(bytes, off).expect("decodes");
                for (_, ph, r) in g.instrs() {
                    if yakogl::shaderc::online::powervr::isa::ops_for(r, ph).any(|o| o.name == "smp")
                        && r.get("nncoords") == Some(1)
                    {
                        let names = [
                            "fcnorm", "drc", "dmn", "chan", "lodm", "pplod", "proj", "sbmode", "nncoords", "sno",
                            "soo", "tao",
                        ];
                        return names.iter().map(|n| (n.to_string(), r.get(n).unwrap_or(0))).collect();
                    }
                }
                off += g.total;
            }
            panic!("no fetch smp")
        };
        assert_eq!(fetch_fields(&ours), fetch_fields(&theirs), "our fetch smp is not pco's form");
    })
    .unwrap();
}

/// `textureSize` of a stacked texture, as glsl2usc's pvr_arrSize (w,
/// h / layers, layers) and pvr_cubeSize (w, h / 6) read the stacked image:
/// ours equals the IR at levels 0 and 1 with 3 and 5 layers, read back
/// through `fract(size / 256)`, and the answers differ.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn texture_size_of_a_stacked_texture_divides_the_stacked_height() {
    let name = tall_sampler(6 * 256);
    /* the array, then the cube, each through a sampler whose model image
     * is tall enough that h / layers and h / 6 are not all 0 or 1 */
    let src = format!(
        "#version 300 es
precision highp float;
precision highp int;
uniform float pvr_layers_{name};
uniform highp sampler2DArray {name};
uniform int lvl;
out vec4 colour;
void main() {{
    ivec3 a = textureSize({name}, lvl);
    colour = fract(vec4(a, 0.0) / 256.0);
}}
"
    );
    let layers_name = format!("pvr_layers_{name}");
    let cases: Vec<BoxedInput> = [(0u32, 3.0f32), (1, 3.0), (0, 5.0), (1, 5.0)]
        .into_iter()
        .map(|(l, n)| -> BoxedInput {
            let layers_name = layers_name.clone();
            Box::new(move |k: &str, _c: u32| {
                if k == layers_name {
                    n.to_bits()
                } else if k == "lvl" {
                    l
                } else {
                    0
                }
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 4, "the array sizes did not change with level and count: {px:08x?}");

    let cube = format!(
        "#version 300 es
precision highp float;
precision highp int;
uniform highp samplerCube {name};
uniform int lvl;
out vec4 colour;
void main() {{
    ivec2 c = textureSize({name}, lvl);
    colour = fract(vec4(c, 0.0, 0.0) / 256.0);
}}
"
    );
    let cases: Vec<BoxedInput> = [0u32, 1, 2]
        .into_iter()
        .map(|l| -> BoxedInput { Box::new(move |k: &str, _c: u32| if k == "lvl" { l } else { 0 }) })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &cube, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 3, "the cube sizes did not change with level: {px:08x?}");
}

/// `texelFetch` on a 2D array: glsl2usc's pvr_arrFetch, texel (x, y + z *
/// (h / layers)) of the stacked image, at its centre. Ours equals the IR at
/// texels of three layers and two levels, which read different pixels.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_texel_fetch_from_a_2d_array_reads_its_layers_texel() {
    let name = tall_sampler(4 * 256);
    let src = format!(
        "#version 300 es
precision highp float;
precision highp int;
uniform float pvr_layers_{name};
uniform highp sampler2DArray {name};
uniform ivec3 p;
uniform int lvl;
out vec4 colour;
void main() {{ colour = texelFetch({name}, p, lvl); }}
"
    );
    let layers_name = format!("pvr_layers_{name}");
    let cases: Vec<BoxedInput> =
        [([3i32, 7i32, 0i32], 0i32), ([3, 7, 1], 0), ([3, 7, 3], 0), ([3, 7, 1], 1), ([40, 2, 2], 0)]
            .into_iter()
            .map(|(pp, l)| -> BoxedInput {
                let layers_name = layers_name.clone();
                Box::new(move |k: &str, c: u32| {
                    if k == layers_name {
                        4.0f32.to_bits()
                    } else if k == "p" {
                        pp[c as usize] as u32
                    } else if k == "lvl" {
                        l as u32
                    } else {
                        0
                    }
                })
            })
            .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 5, "five texels read fewer than five pixels: {px:08x?}");
}

/// `textureOffset` and `textureLodOffset` on a 2D sampler: pco's `smp` with
/// the lookup word (`soo`; the fields equal pco's in the table's
/// 96bc0c44f5554256.frag, sceneeffects' bloom), the offset added in texels.
/// Ours equals the IR for the offsets sceneeffects uses and the extremes GLSL
/// allows (7, -8), each reading a different pixel.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_texture_offset_reads_that_many_texels_away() {
    let src = "#version 300 es
precision highp float;
uniform highp sampler2D s;
uniform vec2 uv;
uniform float level;
out vec4 colour;
void main() {
    colour = 0.25 * (textureOffset(s, uv, ivec2(-1, 0)) + textureLodOffset(s, uv, level, ivec2(0, -1))
        + textureLodOffset(s, uv, level, ivec2(1, -1)) + textureLodOffset(s, uv, 0.0, ivec2(7, -8)));
}
";
    let cases: Vec<BoxedInput> = [([0.30f32, 0.60f32], 0.0f32), ([0.30, 0.60], 1.0), ([0.71, 0.05], 0.0)]
        .into_iter()
        .map(|(c, l)| -> BoxedInput {
            Box::new(move |n: &str, k: u32| match n {
                "uv" => c[k as usize].to_bits(),
                "level" => l.to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir("s", src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 3, "the offsets read the same pixels: {px:08x?}");

    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        // SAFETY: the record's code lives in `keep`; the table's is static.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let ours: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let fx =
            SHC_SOURCE_TABLE.iter().find(|r| r.name == "96bc0c44f5554256.frag").expect("sceneeffects in the table");
        let key = sha256(fx.text.trim_end_matches('\0').as_bytes());
        let blob = SHC_BLOB_TABLE.iter().find(|r| r.sha256 == key && r.blend_key == 0).expect("its pco program");
        let dw = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
        let theirs: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
        let soo_fields = |bytes: &[u8]| -> Vec<(String, u32)> {
            let mut off = 0usize;
            while off < bytes.len() {
                let g = decode_group(bytes, off).expect("decodes");
                for (_, ph, r) in g.instrs() {
                    if yakogl::shaderc::online::powervr::isa::ops_for(r, ph).any(|o| o.name == "smp")
                        && r.get("soo") == Some(1)
                        && r.get("pplod") == Some(1)
                    {
                        let names = [
                            "fcnorm", "drc", "dmn", "chan", "lodm", "pplod", "proj", "sbmode", "nncoords", "sno",
                            "soo", "tao",
                        ];
                        return names.iter().map(|n| (n.to_string(), r.get(n).unwrap_or(0))).collect();
                    }
                }
                off += g.total;
            }
            panic!("no offset smp with a level")
        };
        assert_eq!(soo_fields(&ours), soo_fields(&theirs), "our textureLodOffset smp is not pco's form");
    })
    .unwrap();
}

/// `>>` on a signed int is pco's `ishr`: `shift` with `asr_twb`
/// (pco_trans_nir.c trans_shift), the sign bit filling. Ours equals the IR
/// for negative and positive values and shift counts, and every group of
/// our program is one pco's corpus has (sceneeffects' own asr_twb group
/// among them).
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_signed_shift_right_fills_with_the_sign() {
    let src = "#version 300 es
precision highp float;
precision highp int;
uniform ivec2 v;
out vec4 colour;
void main() {
    int s = v.x >> v.y;
    colour = vec4(float(s & 255) / 255.0, float((s >> 8) & 255) / 255.0, s < 0 ? 1.0 : 0.0, 1.0);
}
";
    let cases: Vec<BoxedInput> = [(-1000i32, 3i32), (1000, 3), (-7, 1), (123456, 9), (-123456, 9)]
        .into_iter()
        .map(|(x, n)| -> BoxedInput {
            Box::new(move |k: &str, c: u32| if k == "v" { [x, n][c as usize] as u32 } else { 0 })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(src, &refs);
    assert!(px.iter().any(|p| (p >> 16) & 0xff == 0xff), "no negative result was read: {px:08x?}");
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        // SAFETY: the record's code lives in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (corpus, _) = usc_exec::corpus_skeletons();
        let mut off = 0usize;
        let mut asr = 0;
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            let mut k = String::new();
            skeleton(&mut k, &g).unwrap();
            assert!(corpus.contains(&k), "an unattested group at {off:#x}: {k}");
            asr += g.instrs().filter(|(_, _, r)| r.get("shift2_op") == Some(4)).count();
            off += g.total;
        }
        assert!(asr >= 2, "the program has {asr} asr_twb shifts, two meant");
    })
    .unwrap();
}

/// A 3D texture look-up through the grid of slices (glsl2usc's pvr_vol):
/// 5 slices, so a 3 x 2 grid, sampled between slices, on one, and past both
/// ends, with the implicit level and textureLod (which the helper samples at
/// level 0). Ours equals the IR at each, on a model image wide and tall
/// enough that a grid cell is not collapsed by the half-texel clamp, and the
/// depths read different pixels.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_3d_texture_mixes_the_two_slices_around_its_depth() {
    let name = (0..2000)
        .map(|i| format!("s{i}"))
        .find(|n| {
            let [w, h] = usc_exec::tex_size_2d(usc_exec::tex_state(sampler_marker(n)), 0);
            w >= 3 * 256 && h >= 2 * 256
        })
        .expect("a sampler name with a wide and tall model image");
    let src = format!(
        "#version 300 es
precision highp float;
uniform float pvr_layers_{name};
uniform highp sampler3D {name};
uniform vec3 c;
uniform float level;
out vec4 colour;
void main() {{ colour = 0.5 * (texture({name}, c) + textureLod({name}, c.yxz, level)); }}
"
    );
    let layers_name = format!("pvr_layers_{name}");
    let cases: Vec<BoxedInput> = [0.0f32, 0.15, 0.3, 0.5, 0.77, 1.2, -0.3]
        .into_iter()
        .map(|z| -> BoxedInput {
            let layers_name = layers_name.clone();
            Box::new(move |k: &str, c: u32| {
                if k == layers_name {
                    5.0f32.to_bits()
                } else if k == "c" {
                    [0.3f32, 0.6, z][c as usize].to_bits()
                } else if k == "level" {
                    2.0f32.to_bits()
                } else {
                    0
                }
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert!(distinct.len() >= 5, "the depths read too few distinct pixels: {px:08x?}");
}

/// textureSize and texelFetch of a 3D texture, as glsl2usc's pvr_volSize
/// and pvr_volFetch read the grid of slices: ours equals the IR for 5 and 7
/// slices and texels in several slices, and they read different pixels.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_3d_texture_size_and_fetch_read_the_slice_grid() {
    let name = (0..2000)
        .map(|i| format!("s{i}"))
        .find(|n| {
            let [w, h] = usc_exec::tex_size_2d(usc_exec::tex_state(sampler_marker(n)), 0);
            w >= 3 * 256 && h >= 3 * 256
        })
        .expect("a sampler name with a wide and tall model image");
    let src = format!(
        "#version 300 es
precision highp float;
precision highp int;
uniform float pvr_layers_{name};
uniform highp sampler3D {name};
uniform ivec3 p;
out vec4 colour;
void main() {{
    ivec3 sz = textureSize({name}, 0);
    colour = 0.5 * texelFetch({name}, p, 0) + 0.5 * fract(vec4(sz, 0.0) / 256.0);
}}
"
    );
    let layers_name = format!("pvr_layers_{name}");
    let cases: Vec<BoxedInput> =
        [(5.0f32, [3i32, 4i32, 0i32]), (5.0, [3, 4, 2]), (5.0, [3, 4, 4]), (7.0, [3, 4, 4]), (7.0, [9, 1, 6])]
            .into_iter()
            .map(|(n, pp)| -> BoxedInput {
                let layers_name = layers_name.clone();
                Box::new(move |k: &str, c: u32| {
                    if k == layers_name {
                        n.to_bits()
                    } else if k == "p" {
                        pp[c as usize] as u32
                    } else {
                        0
                    }
                })
            })
            .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir(&name, &src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 5, "the slices read too few distinct pixels: {px:08x?}");
}

/// Groups no harvested shader has, because none uses the operation, each
/// found verbatim in pco's program for `the_unsigned_operations_compute_
/// what_glsl_says`'s own shader (glsl2usc --dump-translation, glslang,
/// pco_shc build-new, 2026-09-24): the integer min/max group with type s32
/// and u32, and the u32 compare. Add a skeleton here only with the same
/// evidence.
const UNSIGNED_OPS_ATTESTED: [&str; 3] = [
    "alu0 hdr oporg=5 olchk=0 w1p=0 w0p=1 cc=0 end=0 atom=0 rpt=1 | p0 mbyp :sngl sngl_op=7 | p1 mbyp :sngl sngl_op=7 | p4 tst :tst_ext tst_op=8 pwen=0 type=5 p2end=1 elem=0 | p5 movc :movc_ext movw1=0 movw0=0 maskw0=15 aw=0 p2end=0 lo up iss is5=0 is4=1 is3=0 is2=0 is1=0 dst=true",
    "alu0 hdr oporg=5 olchk=0 w1p=0 w0p=1 cc=0 end=0 atom=0 rpt=1 | p0 mbyp :sngl sngl_op=7 | p1 mbyp :sngl sngl_op=7 | p4 tst :tst_ext tst_op=8 pwen=0 type=6 p2end=1 elem=0 | p5 movc :movc_ext movw1=0 movw0=0 maskw0=15 aw=0 p2end=0 lo up iss is5=0 is4=1 is3=0 is2=0 is1=0 dst=true",
    "alu0 hdr oporg=5 olchk=0 w1p=0 w0p=1 cc=0 end=0 atom=0 rpt=1 | p0 mbyp :sngl sngl_op=7 | p1 mbyp :sngl sngl_op=7 | p3 pck :pck prog=0 rtz=0 scale=0 pck_format=30 | p4 tst :tst_ext tst_op=8 pwen=0 type=5 p2end=0 elem=0 | p5 movc :movc_ext movw1=0 movw0=3 maskw0=15 aw=0 p2end=0 lo is0=4 up iss is5=0 is4=2 is3=0 is2=0 is1=0 dst=true",
];

/// A group skeleton pco has emitted: one the table's programs have, or one
/// of [`UNSIGNED_OPS_ATTESTED`] (pco's own program for an operation no
/// harvested shader uses). The survey asks this, so a program reaching one
/// of those operations -- Qt's IBL prefilter loops on a uint count, the u32
/// compare -- is held to the same evidence as the operation's own test.
fn attested(corpus: &BTreeSet<String>, k: &str) -> bool {
    corpus.contains(k) || UNSIGNED_OPS_ATTESTED.contains(&k)
}

/// GLSL ES 3.00's unsigned and integer min/max operations, as pco builds
/// them: udiv/umod (nir_lower_idiv's emit_udiv), ushr (`shr`), ult/ule (the
/// compare group with type u32), imin/imax/umin/umax (the min/max group with
/// type s32/u32), inot (`xnor` with zero), uint(float) and float(uint)
/// (pck/unpck u32). Ours equals the IR at inputs that tell signed from
/// unsigned apart (values past 2^31, negative ints, equal operands), and
/// every group of our program is one pco's corpus has.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn the_unsigned_operations_compute_what_glsl_says() {
    let src = "#version 300 es
precision highp float;
precision highp int;
uniform uvec2 u;
uniform ivec2 i;
uniform float f;
out vec4 colour;
void main() {
    uint d = u.x / u.y;
    uint m = u.x % u.y;
    uint sh = u.x >> 3u;
    uint umn = min(u.x, u.y);
    uint umx = max(u.x, u.y);
    int mn = min(i.x, i.y);
    int mx = max(i.x, i.y);
    uint nt = ~u.x;
    uint fu = uint(f);
    uint mix1 = d ^ (m << 8u) ^ (sh << 16u) ^ umn ^ (umx << 5u) ^ nt ^ (fu << 3u) ^ uint(mn) ^ (uint(mx) << 11u);
    colour = vec4(float(mix1 & 255u) / 255.0, float((mix1 >> 8u) & 255u) / 255.0,
                  (u.x < u.y ? 0.25 : 0.0) + (u.x <= u.y ? 0.5 : 0.0), fract(float(u.x) / 65536.0));
}
";
    let cases: Vec<BoxedInput> = [
        ([3_000_000_000u32, 7u32], [-5i32, 3i32], 17.75f32),
        ([7, 3_000_000_000], [3, -5], 2.5),
        ([123_456, 123_456], [-100, -100], 4_000_000_000.0),
        ([0xffff_fff0, 16], [i32::MIN + 1, 1], 0.0),
    ]
    .into_iter()
    .map(|(uu, ii, ff)| -> BoxedInput {
        Box::new(move |n: &str, c: u32| match n {
            "u" => uu[c as usize],
            "i" => ii[c as usize] as u32,
            "f" => ff.to_bits(),
            _ => 0,
        })
    })
    .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    fs_against_ir(src, &refs);
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        // SAFETY: the record's code lives in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (corpus, _) = usc_exec::corpus_skeletons();
        let mut off = 0usize;
        let mut unattested = Vec::new();
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            let mut k = String::new();
            skeleton(&mut k, &g).unwrap();
            if !attested(&corpus, &k) {
                unattested.push(format!("{off:#x}: {k}"));
            }
            off += g.total;
        }
        assert!(unattested.is_empty(), "groups neither pco's corpus nor pco_shc on this shader has: {unattested:#?}");
    })
    .unwrap();
}

/// A shader that samples AND indexes a uniform array dynamically: the
/// descriptors come before the uniform block, so the idx0 address selection
/// computes (the block offset plus the index) must move with the block --
/// `expand_samples` adds the descriptors' size to it. It did not, and such a
/// program read the uniform 20 dwords too low (Qt's morph targets,
/// `qt_morphWeights[i]` beside a texelFetch; on main, any fragment shader with
/// a texture and a uniform-indexed array). Ours equals the IR for each index,
/// and the three indices read three different values.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn an_indexed_uniform_is_read_past_the_descriptors() {
    let src = "#version 300 es
precision highp float;
precision highp int;
uniform highp sampler2D s;
uniform float w[3];
uniform int k;
out vec4 colour;
void main() { colour = vec4(w[k], 0.0, 0.0, 1.0) * 0.5 + 0.5 * texture(s, vec2(0.3, 0.6)); }
";
    let cases: Vec<BoxedInput> = [0i32, 1, 2]
        .into_iter()
        .map(|k| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match n {
                "w" => [0.1f32, 0.5, 0.9][c as usize].to_bits(),
                "k" => k as u32,
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir("s", src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 3, "the three indices read the same value: {px:08x?}");
}

/// The scratch selection spends per `if`: its join copies the local map
/// and the outputs, and gave none of it back, so a shader with many `if`s
/// and a large local set ran the 2 MiB arena out (frontend3's measurement
/// on the random.23 shape: 156 `if`s, 14 KB twice each; likely ssao_300's
/// OutOfScratch too). The copies are now released at the end of each `if`
/// and reused by the next. Measured here, 32 locals: the growth from 80 to
/// 160 `if`s was 1,850 bytes per `if` with the copies kept, 980 with them
/// released (what remains is the groups themselves and the lists that grow
/// with them). Pinned below the kept figure.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn an_if_gives_its_join_copies_back() {
    let used_by = |n: usize| -> usize {
        let mut body = String::new();
        for i in 0..n {
            body.push_str(&format!(
                "    if (u.x > {i}.0) {{ v{} += u.y; }} else {{ v{} -= u.z; }}\n",
                i % 32,
                (i + 7) % 32
            ));
        }
        let decl: String = (0..32).map(|i| format!("    float v{i} = float({i});\n")).collect();
        let src = format!(
            "#version 300 es\nprecision highp float;\nuniform vec4 u;\nout vec4 colour;\nvoid main() {{\n{decl}{body}    colour = vec4(v0 + v5 + v31, 0.0, 0.0, 1.0);\n}}\n"
        );
        let _s = scratch_lock();
        store::with_scratch(|a| {
            let t = a.alloc_str(&src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, t).map_err(|e| e.msg).unwrap();
            let before = a.used();
            yakogl::shaderc::online::powervr::select::select(&l.module, a).expect("selects");
            a.used() - before
        })
        .unwrap()
    };
    let (small, large) = (used_by(80), used_by(160));
    let per_if = large.saturating_sub(small) / 80;
    println!("  selection scratch: 80 ifs {small} bytes, 160 ifs {large} bytes, {per_if} bytes per if");
    assert!(per_if <= 1200, "{per_if} bytes per if: the join's copies are kept again (1,850 when they were)");
}

/// A loop whose trip count is read at run time from a uniform, as Qt's
/// light loops are (ES 3.00: `for (int i = 0; float(i) < count; i++)`): kept
/// as ONE real loop, its back edge taken while the counter is below
/// unroll::UNIFORM_LOOP_CAP and its (uniform) break flag is clear, reading a
/// uniform array by the counter. Ours equals the IR -- which caps its loops
/// the same way -- at counts 0, 1, 3, 7.5, NaN, +inf (stops at the cap, a
/// deliberate deviation), 5000 (past the cap) and -2; the program holds one
/// back edge and no expansion (few groups for any count).
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_loop_bounded_by_a_uniform_runs_as_many_times_as_it_says() {
    let cap = yakogl::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP;
    println!("  a loop counted at run time stops after {cap} iterations (UNIFORM_LOOP_CAP, deliberate)");
    let src = "#version 300 es
precision highp float;
precision highp int;
uniform float count;
uniform float w[8];
out vec4 colour;
void main() {
    float acc = 0.0;
    for (int i = 0; float(i) < count; i++) {
        acc += w[i] * 0.125;
    }
    colour = vec4(fract(acc), fract(acc * 0.0625), 0.0, 1.0);
}
";
    let counts = [0.0f32, 1.0, 3.0, 7.5, f32::NAN, f32::INFINITY, 5000.0, -2.0];
    let cases: Vec<BoxedInput> = counts
        .into_iter()
        .map(|n| -> BoxedInput {
            Box::new(move |name: &str, c: u32| match name {
                "count" => n.to_bits(),
                "w" => [0.3f32, 0.7, 1.1, 0.2, 0.9, 1.3, 0.5, 0.6][c as usize].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px[..4].iter().copied().collect();
    assert_eq!(distinct.len(), 4, "counts 0, 1, 3 and 7.5 read the same pixel: {px:08x?}");
    /* one loop, not an expansion */
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        // SAFETY: the record's code lives in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut off = 0usize;
        let mut branches = 0;
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            branches += g
                .instrs()
                .filter(|(_, ph, r)| yakogl::shaderc::online::powervr::isa::ops_for(r, *ph).any(|o| o.name == "br"))
                .count();
            off += g.total;
        }
        assert_eq!(branches, 1, "one back edge");
        assert!(rec.usc_dw < 400, "{} dwords: the loop was expanded", rec.usc_dw);
        /* It leaves when its bound says, not at the cap: the values cannot
         * show that (the iterations after the break change nothing), the
         * number of groups executed does. */
        let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
        let steps = |n: f32| -> usize {
            let mut inp = sentinel_inputs(1.0e30);
            place_shared(&mut inp, rows, &|name: &str, _c: u32| {
                if name == "count" {
                    n.to_bits()
                } else {
                    0.5f32.to_bits()
                }
            });
            run_binary(&image, &inp, true).expect("runs");
            usc_exec::last_steps()
        };
        let (short, capped) = (steps(3.0), steps(f32::INFINITY));
        println!("  groups executed: count 3 -> {short}, count +inf -> {capped}");
        assert!(short * 100 < capped, "count 3 ran {short} groups, +inf {capped}: the loop does not leave on its flag");
    })
    .unwrap();
}

/// The operations a program's bytes hold, by name, in order.
fn op_names(image: &[u8]) -> Vec<&'static str> {
    let mut out = Vec::new();
    let mut off = 0usize;
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            if let Some(o) = yakogl::shaderc::online::powervr::isa::ops_for(r, ph).next() {
                out.push(o.name);
            }
        }
        off += g.total;
    }
    out
}

/// A loop whose bound may differ between instances -- a varying, a texture,
/// a derivative in a fragment shader; an attribute or gl_VertexID in a
/// vertex shader -- runs under pco's execution mask (`powervr::cf`) instead
/// of being refused (dEQP-GLES2 loops.*_dynamic_iterations, 170 cases on the
/// host run): an instance leaves at the end of the pass its flag was set in,
/// and the back edge is taken while it is in, up to UNIFORM_LOOP_CAP. Ours
/// equals the IR at bounds 0, 1, 3, 7.5, -2, NaN and past the cap; the
/// program holds the mask's cndsm and cndlt; and it leaves when its bound
/// says (count 3 runs far fewer groups than +inf -- the values alone cannot
/// show that). The vertex shaders go through the survey's whole per-source
/// check. (gl_FragCoord.x as a bound is still refused, for the flip pair's
/// record field.) What one modelled instance cannot show -- instances
/// leaving at different passes -- is gl_test 104's.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_loop_bounded_per_instance_runs_under_the_execution_mask() {
    let src = "#version 300 es
precision highp float;
in float n;
out vec4 colour;
void main() {
    float acc = 0.0;
    for (int i = 0; float(i) < n; i++) acc += 0.125;
    colour = vec4(fract(acc), fract(acc * 0.0625), 0.0, 1.0);
}
";
    let counts = [0.0f32, 1.0, 3.0, 7.5, -2.0, f32::NAN, 5000.0];
    let cases: Vec<BoxedInput> = counts
        .into_iter()
        .map(|x| -> BoxedInput { Box::new(move |name: &str, _c: u32| if name == "n" { x.to_bits() } else { 0 }) })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(src, &refs);
    let distinct: std::collections::BTreeSet<u32> = px[..4].iter().copied().collect();
    assert_eq!(distinct.len(), 4, "bounds 0, 1, 3 and 7.5 read the same pixel: {px:08x?}");
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles under the mask");
        // SAFETY: the record's code lives in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let names = op_names(&image);
        for want in ["cndst", "cndsm", "cndlt"] {
            assert!(names.contains(&want), "no {want} in {names:?}");
        }
        let steps = |x: f32| -> usize {
            let mut inp = sentinel_inputs(1.0e30);
            inp.coeff[4] = x.to_bits();
            inp.coeff[6] = x.to_bits();
            run_binary(&image, &inp, true).expect("runs");
            usc_exec::last_steps()
        };
        let (short, long) = (steps(3.0), steps(f32::INFINITY));
        println!("  groups executed under the mask: bound 3 -> {short}, +inf -> {long}");
        assert!(short * 100 < long, "bound 3 ran {short} groups, +inf {long}: the instance does not leave");
    })
    .unwrap();
    drop(_scratch);

    /* a texture and a derivative as the bound */
    let name = tall_sampler(64);
    let tex = "#version 300 es
precision highp float;
uniform highp sampler2D t;
in vec2 uv;
out vec4 colour;
void main() {
    float acc = 0.0;
    for (int i = 0; float(i) < texture(t, uv).x * 8.0; i++) acc += 0.0625;
    colour = vec4(acc, 0.0, 0.0, 1.0);
}
"
    .replace("(t,", &format!("({name},"))
    .replace(" t;", &format!(" {name};"));
    let cases: Vec<BoxedInput> = [(0.1f32, 0.2f32), (0.6, 0.3), (0.9, 0.8)]
        .into_iter()
        .map(|(u, v)| -> BoxedInput {
            Box::new(move |n: &str, c: u32| if n == "uv" { [u, v][c as usize].to_bits() } else { 0 })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    stacked_against_ir(&name, &tex, &refs);
    let deriv = "#version 300 es\nprecision highp float;\nuniform float u;\nout vec4 colour;\nvoid main() {\n    float acc = 0.0;\n    for (int i = 0; float(i) < dFdx(u) + u; i++) acc += 0.25;\n    colour = vec4(acc);\n}\n";
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(deriv).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        compile_fragment(&l.module, &l.reflection, a).expect("a derivative bound compiles under the mask");
    })
    .unwrap();

    /* the vertex stage: the survey's whole check -- a uniform bound too, the
     * counted-at-run-time loop with no mask, which no vertex test had kept
     * before (its p0 test read back as a destination mismatch) */
    let (corpus, _) = corpus_skeletons();
    for (what, decl, bound) in [
        ("a uniform", "uniform float u;\n", "u"),
        ("an attribute", "in float n;\n", "n"),
        ("gl_VertexID", "", "float(gl_VertexID)"),
    ] {
        let src = format!(
            "#version 300 es\n{decl}void main() {{\n    float acc = 0.0;\n    for (int i = 0; float(i) < {bound}; i++) acc += 0.25;\n    gl_Position = vec4(acc);\n}}\n"
        );
        let (_, g) = run_one(&src, Stage::Vertex, &corpus).unwrap_or_else(|e| panic!("{what}: {e}"));
        assert!(g.checked == g.groups && g.mismatches.is_empty(), "{what}: {:?}", g.mismatches);
        assert!(g.unattested.is_empty(), "{what}: {:?}", g.unattested);
        assert!(g.executed > 0 && g.exec_failures.is_empty(), "{what}: {:?}", g.exec_failures);
        assert!(g.ir_unrun.is_none() && g.ir_agreed > 0 && g.ir_failures.is_empty(), "{what}: {:?}", g.ir_failures);
    }
}

/// One input to a small shader: a uniform or varying name and component
/// to bits.
type InputFn<'a> = dyn Fn(&str, u32) -> u32 + 'a;
type BoxedInput = Box<InputFn<'static>>;

/// Run a small fragment shader through the whole chain and compare its
/// packed pixel with the IR's for each input `val` gives (uniform or
/// varying name, component -> bits), within one step of the 8-bit packing.
/// Returns the IR's pixels, so a caller can assert its inputs reached what
/// they were meant to.
fn fs_against_ir(src: &str, inputs: &[&InputFn<'_>]) -> Vec<u32> {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = match compile_fragment(&l.module, &l.reflection, a) {
            Ok(r) => r,
            Err(e) => panic!("the chain refused it: {e:?}"),
        };
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let rows: &[yakogl::shaderc::ShcUniform] = if rec.num_uniforms == 0 {
            &[]
        } else {
            // SAFETY: the record's uniform rows live in `keep`.
            unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) }
        };
        let name_of = |n: &[core::ffi::c_char]| -> String {
            n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let mut pixels = Vec::new();
        for val in inputs {
            let mut mine = sentinel_inputs(1.0e30);
            place_shared(&mut mine, rows, val);
            let (pntc_at, mut at) = coeff_layout(rec.uses_z, rec.uses_w, rec.uses_pntc);
            place_point_coord(&mut mine, pntc_at, &|c| val("gl_PointCoord", c));
            place_frag_zw(&mut mine, &rec, &|c| val("gl_FragCoord", c));
            for v in &rec.varyings[..rec.num_varyings as usize] {
                let n = name_of(&v.name);
                for c in 0..v.components as usize {
                    {
                        let w = val(&n, c as u32);
                        mine.coeff[at + 4 * c] = w;
                        mine.coeff[at + 4 * c + 2] = w;
                    }
                }
                at += 4 * v.components as usize;
            }
            /* gl_FrontFacing from the input's "gl_FrontFacing" (nonzero:
             * front), the driver's op 0 keeping the ISP's face. */
            let front = val("gl_FrontFacing", 0) != 0;
            if rec.fs_front_face_off1 != 0 {
                mine.face_ccw = u32::from(!front);
                mine.shared[rec.fs_front_face_off1 as usize - 1] = 0;
            }
            let ours = run_binary(&image, &mine, true).expect("ours runs");
            let ir_in = usc_exec::ir::IrInputs {
                input: &|slot, c| match slot {
                    IoSlot::Varying(loc) => {
                        l.module.inputs.iter().find(|io| io.location == loc).map_or(0, |io| val(io.name, c))
                    }
                    IoSlot::PointCoord => val("gl_PointCoord", c),
                    IoSlot::FragCoord if c >= 2 => val("gl_FragCoord", c),
                    IoSlot::FrontFacing if front => u32::MAX,
                    _ => 0,
                },
                uniform: &|d| {
                    let u = l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                    u.map_or(0, |u| val(u.name, (d - u.dword_off) as u32))
                },
                marker: &|_| 0,
            };
            let want = usc_exec::ir::run_ir(&l.module, &ir_in).expect("the IR runs");
            let (_, lanes) = want.outputs.iter().find(|(s, _)| *s == IoSlot::FragColor(0)).expect("a colour");
            let mut px = 0u32;
            for (c, lane) in lanes.iter().enumerate() {
                let x = f32::from_bits(lane.expect("stored").0);
                px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
            }
            let got = *ours.get(&0).expect("a pixel");
            /* An undefined channel (a local no path wrote) is not compared. */
            let near = (0..4).all(|c| {
                lanes[c].is_some_and(|(_, t)| t) || ((got >> (8 * c)) & 0xff).abs_diff((px >> (8 * c)) & 0xff) <= 1
            });
            assert!(near, "ours {got:08x}, the IR {px:08x}");
            pixels.push(px);
        }
        pixels
    })
    .unwrap()
}

/// `fract(x)` is `x - floor(x)` (`lower_ffract`) and `ceil` pco's one-group
/// `group_map(O_FCEIL)`, negative and whole `x` included; the table's one
/// `ceil` (filter_image.frag) does not fit the image yet, so this is its
/// exercise.
#[test]
fn fract_is_x_minus_its_floor_and_ceil_rounds_up() {
    let cases: Vec<BoxedInput> = [-2.75f32, -1.0, -0.3, 0.0, 0.4, 1.5, 3.99]
        .into_iter()
        .map(|x| Box::new(move |_: &str, _: u32| x.to_bits()) as BoxedInput)
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(
        "precision highp float; varying vec2 v; void main(){ gl_FragColor = vec4(fract(v.x), fract(-v.y), \
         ceil(v.x) * 0.2 + 0.6, ceil(-v.y) * 0.2 + 0.6); }",
        &refs,
    );
    /* -0.3 gives 0.7 and 0.3: the floor, not a truncation. */
    assert_eq!(px[2] & 0xff, (0.7f32 * 255.0).round() as u32);
}

/// A loop that leaves on data -- a `break` and a `continue` whose
/// conditions are uniforms -- computes what the loop does. It is kept as a
/// loop (`unroll.rs`): its body once under flags, and ONE backward branch on
/// its counter, which the byte model follows; the IR interpreter runs the
/// ORIGINAL loop. Every break point and every skipped iteration is reached.
#[test]
fn a_loop_that_leaves_on_data_computes_what_the_loop_does() {
    let cases: Vec<BoxedInput> = (0..=9u32)
        .flat_map(|brk| (0..=9u32).map(move |skip| (brk, skip)))
        .map(|(brk, skip)| {
            Box::new(move |n: &str, k: u32| -> u32 {
                match (n, k) {
                    ("u", 0) => (brk as f32).to_bits(),
                    ("u", 1) => (skip as f32).to_bits(),
                    ("u", _) => 0,
                    ("v", 0) => 0.37f32.to_bits(),
                    _ => 0.5f32.to_bits(),
                }
            }) as BoxedInput
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(
        "precision highp float; uniform vec4 u; varying vec2 v; \
         void main(){ float s = 0.0; float c = 0.0; \
         for (float i = 1.0; i <= 8.0; i += 1.0) { \
             if (i >= u.x) { break; } \
             if (i == u.y) { continue; } \
             s += v.x * i; c += 1.0; } \
         gl_FragColor = vec4(s * 0.05, c * 0.125, v.y, 1.0); }",
        &refs,
    );
    /* The iteration count reaches every value it can: 0 (break at the
     * first) through 8 (neither), so every exit point was taken. */
    let counts: BTreeSet<u32> = px.iter().map(|p| p >> 8 & 0xff).collect();
    assert!(counts.len() >= 9, "the inputs reached {} distinct iteration counts", counts.len());
    let src = "precision highp float; uniform vec4 u; varying vec2 v; \
         void main(){ float s = 0.0; float c = 0.0; \
         for (float i = 1.0; i <= 8.0; i += 1.0) { \
             if (i >= u.x) { break; } \
             if (i == u.y) { continue; } \
             s += v.x * i; c += 1.0; } \
         gl_FragColor = vec4(s * 0.05, c * 0.125, v.y, 1.0); }";
    assert_eq!(backward_branches(src), 1, "the loop is one body and one back edge");
}

/// gl_test case 87's shader, on the host: the byte model runs our program
/// (one body, one back edge) at the centre of each of the eight bands in both
/// halves, the IR runs the loop, and both give the count the board case
/// expects -- b + 1 in band b, one fewer in the top half from band 2 up.
#[test]
fn gl_test_case_87_counts_what_the_board_case_expects() {
    let src = include_str!("../tools/shaders/es100/databreak.frag");
    let mut cases: Vec<BoxedInput> = Vec::new();
    let mut want = Vec::new();
    for top in [false, true] {
        for b in 0..8u32 {
            /* A band's centre, as case 87 reads it on a 64-wide target. */
            let s = (b as f32 + 0.5) / 8.0;
            let t = if top { 0.75f32 } else { 0.25 };
            cases.push(Box::new(move |n: &str, k: u32| -> u32 {
                match (n, k) {
                    ("v_color", 0) => (1.0 - t).to_bits(),
                    ("v_color", 1) => t.to_bits(),
                    ("v_color", 2) => (1.0 - s).to_bits(),
                    ("v_color", 3) => s.to_bits(),
                    _ => 0,
                }
            }) as BoxedInput);
            want.push((b + 1 - u32::from(top && b >= 2), top));
        }
    }
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(src, &refs);
    for (p, (count, top)) in px.iter().zip(&want) {
        let r = p & 0xff;
        let expect = (count * 255 + 4) / 8;
        assert!(r.abs_diff(expect) <= 1, "count {count}: red {r}, want {expect}");
        assert_eq!(p >> 16 & 0xff, if *top { 0xff } else { 0 }, "the half");
    }
    assert_eq!(backward_branches(src), 1, "case 87's loop is kept as a loop");
}

/// gl_test case 98's shader, on the host: the byte model and the IR give the
/// colours the board case expects for counts 0, 1 and 7, and for 4097 the
/// cap's 4096 (UNIFORM_LOOP_CAP, the deliberate deviation: GL would count
/// 4097, red 16 instead of 0). The counter agrees with the count every time
/// (never white), and the loop is kept as one.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn gl_test_case_98_counts_what_the_board_case_expects() {
    let src = include_str!("../tools/shaders/es300/uniloop.frag");
    let cap = yakogl::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP;
    assert_eq!(cap, 4096, "case 98 expects the cap at 4096");
    let counts = [0.0f32, 1.0, 7.0, 4097.0];
    let cases: Vec<BoxedInput> = counts
        .into_iter()
        .map(|n| -> BoxedInput { Box::new(move |name: &str, _c: u32| if name == "count" { n.to_bits() } else { 0 }) })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(src, &refs);
    /* What gl_test.c's case_uniloop expects, AABBGGRR. */
    let want = [0xff00_0000u32, 0xff00_0010, 0xff00_0070, 0xfff0_0000];
    for ((n, p), w) in counts.iter().zip(&px).zip(want) {
        for sh in [0, 8, 16, 24] {
            let (g, e) = (p >> sh & 0xff, w >> sh & 0xff);
            assert!(g.abs_diff(e) <= 1, "count {n}: {p:08x}, the board case expects {w:08x}");
        }
    }
    assert_eq!(backward_branches(src), 1, "case 98's loop is kept as a loop");
}

/// The execution-mask groups (`powervr::cf`) are pco's to the byte: each
/// one built with the registers pco chose for femtovg's filter_image.frag (a
/// loop that leaves on data; the table's program, blend key 0) equals pco's
/// group at the offset it sits at there -- the emc set up and a loop level
/// entered, the skip when no instance enters, a `break` (cndsm on p0 false,
/// leaving by 2), the level left and re-entered at the body's end, and
/// cndlt setting p0 for the back edge.
#[test]
fn the_execution_mask_groups_are_pcos_to_the_byte() {
    use yakogl::shaderc::online::powervr::cf::{branch_none_active, cnd, Cnd};
    use yakogl::shaderc::online::powervr::operands::Operand;
    let row = SHC_SOURCE_TABLE.iter().find(|r| r.name == "filter_image.frag").expect("filter_image.frag in the table");
    let key = sha256(row.text.trim_end_matches('\0').as_bytes());
    let blob = SHC_BLOB_TABLE.iter().find(|r| r.sha256 == key && r.blend_key == 0).expect("its pco program");
    // SAFETY: the table's programs are static.
    let dw = unsafe { core::slice::from_raw_parts(blob.shader.usc, blob.shader.usc_dw as usize) };
    let theirs: Vec<u8> = dw.iter().flat_map(|w| w.to_le_bytes()).collect();
    const SPECIAL: u32 = 0;
    const TEMP: u32 = 1;
    let emc = Operand { bank: TEMP, index: 29 };
    let special = |i: u32| Operand { bank: SPECIAL, index: i };
    let cases: [(usize, yakogl::shaderc::online::powervr::decode::Group); 7] = [
        (0x284, cnd(Cnd::St, 1, "always", &[special(0)], emc).expect("cndst 1")),
        (0x28c, cnd(Cnd::St, 2, "always", &[emc], emc).expect("cndst 2")),
        (0x294, branch_none_active(648).expect("br allp")),
        (0x2f8, cnd(Cnd::Sm, 0, "p0_false", &[emc, special(0), special(2)], emc).expect("cndsm")),
        (0x50c, cnd(Cnd::End, 1, "always", &[emc], emc).expect("cndend 1")),
        (0x514, cnd(Cnd::St, 1, "always", &[emc], emc).expect("cndst 1 again")),
        (0x51c, cnd(Cnd::Lt, 2, "always", &[emc], emc).expect("cndlt 2")),
    ];
    for (off, g) in cases {
        let mut buf = [0u8; 64];
        let n = yakogl::shaderc::online::powervr::encode::encode_group(&g, &mut buf, 0).expect("encodes");
        assert_eq!(&buf[..n], &theirs[off..off + n], "the group at {off:#x}");
        let back = decode_group(&theirs, off).expect("pco's decodes");
        assert_eq!(back.total, g.total, "the group at {off:#x}: pco's is {} bytes", back.total);
    }
}

/// The byte model runs pco's execution-mask construction (`cnd*`, the
/// `allp` branch, `(ignorepe)`) with semantics inferred from how pco_cf.c
/// uses it, one instance: the check on that inference is pco's own
/// programs. femtovg's filter_image.frag -- a loop that leaves on data, built
/// by pco with an emc -- and its nodiscard twin run to their end in the
/// model and agree with ours on every input the survey gives them (no
/// "pco unrunnable", no difference). Mutation-checked: a cndsm that does
/// nothing (a `break` that never leaves) runs pco's loop forever.
#[test]
fn pcos_execution_mask_loops_run_in_the_model_and_agree_with_ours() {
    let (corpus, _) = corpus_skeletons();
    let _scratch = scratch_lock();
    for name in ["filter_image.frag", "filter_image.nodiscard.frag"] {
        let row = SHC_SOURCE_TABLE.iter().find(|r| r.name == name).expect("in the table");
        let (diffs, _) =
            run_fragment(row.text.trim_end_matches('\0'), &corpus).unwrap_or_else(|e| panic!("{name}: {e}"));
        let bad: Vec<&String> =
            diffs.iter().filter(|d| !d.starts_with("NOTE ") || d.contains("pco unrunnable")).collect();
        assert!(bad.is_empty(), "{name}: {bad:?}");
    }
}

/// A matrix vertex attribute: one record row per column, each with the
/// matrix's name and type at its column's location and one column's width,
/// the program reading column c from location + c (GL ES 2.0 2.10.4; the GL
/// core reads the rows back as one attribute, gl/program.rs). It was refused
/// as Record(MatrixAttrib) -- 417 dEQP-GLES2 cases on the host run
/// (shaders.matrix.*, conversions.matrix_*, attribute_location.*mat*).
/// gl_test case 63's three shaders go through the survey's whole per-source
/// check (read back, attested, ours against selection and the IR, with the
/// vertex inputs placed from the record's rows); a mat3 read only in its
/// last column still lists all three.
#[test]
fn a_matrix_attribute_is_one_record_row_per_column() {
    let (corpus, _) = corpus_skeletons();
    let _scratch = scratch_lock();
    for (src, cols) in [
        (include_str!("../tools/shaders/es100/matattr2.vert"), 2u8),
        (include_str!("../tools/shaders/es100/matattr3.vert"), 3),
        (include_str!("../tools/shaders/es100/matattr4.vert"), 4),
    ] {
        let (_, g) = run_one(src, Stage::Vertex, &corpus).unwrap_or_else(|e| panic!("mat{cols}: {e}"));
        assert!(g.checked == g.groups && g.mismatches.is_empty(), "mat{cols}: {:?}", g.mismatches);
        assert!(g.unattested.is_empty(), "mat{cols}: {:?}", g.unattested);
        assert!(g.executed > 0 && g.exec_failures.is_empty(), "mat{cols}: {:?}", g.exec_failures);
        assert!(g.ir_unrun.is_none() && g.ir_agreed > 0 && g.ir_failures.is_empty(), "mat{cols}: {:?}", g.ir_failures);
    }
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let src = "attribute vec4 a_pos; attribute mat3 a_m; varying vec3 v; \
                   void main(){ v = a_m[2]; gl_Position = a_pos; }";
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| e.msg).unwrap();
        let rec = yakogl::shaderc::online::powervr::chain::compile_vertex(&l.module, &l.reflection, a)
            .unwrap_or_else(|e| panic!("{e:?}"));
        let name_of = |n: &[core::ffi::c_char]| -> String {
            n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
        };
        let rows: Vec<(String, u8, u8, u8, u16)> = rec.attribs[..rec.num_attribs as usize]
            .iter()
            .map(|r| (name_of(&r.name), r.location, r.components, r.vi_reg, r.gl_type))
            .collect();
        let m = |loc: u8| ("a_m".to_string(), loc, 3u8, loc * 4, 0x8B5Bu16);
        assert_eq!(rows, [("a_pos".to_string(), 0, 4, 0, 0x8B52), m(1), m(2), m(3)], "one row per column");
    })
    .unwrap();
}

/// dEQP-GLES2.functional.light_amount.14omni, vertex shader: fourteen
/// unrolled calls that each copy a light struct. dEQP (VK-GL-CTS) is
/// Apache-2.0; the text is what the host run's .qpa records.
const DEQP_LIGHT_AMOUNT_14OMNI_VS: &str = r#"struct Light
{
    vec3    position;
    vec3    diffuse;
    vec3    specular;
    vec3    attenuation;
};
uniform Light u_lights[0 + 14];
uniform mat4 u_modelviewMatrix;
uniform mat4 u_modelviewProjectionMatrix;
uniform mat3 u_normalMatrix;

attribute vec4 a_position;
attribute vec3 a_normal;

varying vec3 v_color;

float computeAttenuation(vec3 dirToLight, vec3 attenuation)
{
    float dist = length(dirToLight);
    return 1.0 / (attenuation.x + attenuation.y*dist + attenuation.z*dist*dist);
}

vec3 computeDirLight(int ndx, vec3 position, vec3 normal)
{
    Light light = u_lights[ndx];
    float cosAngle = dot(light.position, normal);
    return cosAngle * light.diffuse;
}

vec3 computeOmniLight(int ndx, vec3 position, vec3 normal)
{
    Light light = u_lights[ndx];
    vec3 dirToLight = light.position - position;
    float cosAngle = dot(normalize(dirToLight), normal);
    float atten = computeAttenuation(dirToLight, light.attenuation);
    return atten * cosAngle * light.diffuse;
}

void main()
{
    vec3 lightSpacePos = vec3(u_modelviewMatrix * a_position);
    vec3 lightNormal = normalize(u_normalMatrix * a_normal);
    vec3 color = vec3(0.0);
    for (int i = 0; i < 0; i++)
        color += computeDirLight(i, lightSpacePos, lightNormal);
    for (int i = 0; i < 14; i++)
        color += computeOmniLight(0+i, lightSpacePos, lightNormal);
    v_color = color;
    gl_Position = u_modelviewProjectionMatrix * a_position;
}
"#;

/// dEQP-GLES2.functional.shaders.indexing.tmp_array.vec3_const_write_static_
/// read_fragment, fragment shader: a 40-element local array written at an
/// index held in a local, which the front end lowers as a select per
/// element (about 6,000 instructions). Same source and licence.
const DEQP_TMP_ARRAY_VEC3_STATIC_FS: &str = r#"
void main()
{
    mediump vec3 arr[40];
    arr[0] = vec3(0.125, 0.25, 0.5);
    arr[1] = vec3(0.125, 0.25, 0.5) * 0.5;
    arr[2] = vec3(0.125, 0.25, 0.5) * 0.25;
    arr[3] = vec3(0.125, 0.25, 0.5) * 0.125;
    int i = 4;
    arr[i++] = vec3(4.0);
    arr[i++] = vec3(5.0);
    arr[i++] = vec3(6.0);
    arr[i++] = vec3(7.0);
    arr[i++] = vec3(8.0);
    arr[i++] = vec3(9.0);
    arr[i++] = vec3(10.0);
    arr[i++] = vec3(11.0);
    arr[i++] = vec3(12.0);
    arr[i++] = vec3(13.0);
    arr[i++] = vec3(14.0);
    arr[i++] = vec3(15.0);
    arr[i++] = vec3(16.0);
    arr[i++] = vec3(17.0);
    arr[i++] = vec3(18.0);
    arr[i++] = vec3(19.0);
    arr[i++] = vec3(20.0);
    arr[i++] = vec3(21.0);
    arr[i++] = vec3(22.0);
    arr[i++] = vec3(23.0);
    arr[i++] = vec3(24.0);
    arr[i++] = vec3(25.0);
    arr[i++] = vec3(26.0);
    arr[i++] = vec3(27.0);
    arr[i++] = vec3(28.0);
    arr[i++] = vec3(29.0);
    arr[i++] = vec3(30.0);
    arr[i++] = vec3(31.0);
    arr[i++] = vec3(32.0);
    arr[i++] = vec3(33.0);
    arr[i++] = vec3(34.0);
    arr[i++] = vec3(35.0);
    arr[i++] = vec3(36.0);
    arr[i++] = vec3(37.0);
    arr[i++] = vec3(38.0);
    arr[i++] = vec3(39.0);
    mediump vec3 res = vec3(0.0);
    res += arr[0];
    res += arr[1];
    res += arr[2];
    res += arr[3];
    gl_FragColor = vec4(res, 1.0);
}
"#;

/// Two dEQP shapes that ran the 2 MiB scratch arena out on the PowerVR
/// online path (Prepare(OutOfScratch), host run 9ca356db) now compile
/// through the product's steps -- the front end, its garbage given back
/// (`lower::relocate`), the whole chain -- and the arena's peak stays under
/// a bound set from what the chain holds now (1.88 MB and 1.29 MB, measured
/// 2026-09-24): selection kept its tables and copies before. Given back
/// now: merge_nested_selects' tables and the list it replaces,
/// zero_unwritten_locals' search lists (now flags) and its copy, unroll's
/// three lists doubling in turn (reserved), and the environment copies of an
/// `if` unroll cannot decide.
///
/// 14omni's bound was re-derived on 2026-09-29 (2.0 MB -> 2.12 MB): its
/// light arrays reach past sh255, and `prepare::index_high_shared` now reads
/// those through idx0 as pco does (a sh256+k read returned sh<k> on the
/// AM62P). The pass copies the group list once, with one idx0 write per such
/// read: 1,882,660 -> 2,001,456 bytes (b7c5cedf -> am62p/sh-index).
#[test]
fn deqp_shapes_that_ran_the_scratch_arena_out_now_fit_it() {
    for (name, stage, src, bound) in [
        ("light_amount.14omni", Stage::Vertex, DEQP_LIGHT_AMOUNT_14OMNI_VS, 2_120_000usize),
        ("tmp_array vec3 static", Stage::Fragment, DEQP_TMP_ARRAY_VEC3_STATIC_FS, 1_450_000),
    ] {
        let _scratch = scratch_lock();
        let peak = store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let mark = a.used();
            let l = lower::compile_to_ir(a, stage, text)
                .and_then(|l| lower::relocate(a, l, text, mark))
                .map_err(|e| e.msg)
                .unwrap();
            let after_front = a.used();
            let r = if stage == Stage::Vertex {
                yakogl::shaderc::online::powervr::chain::compile_vertex(&l.module, &l.reflection, a).map(|_| ())
            } else {
                compile_fragment(&l.module, &l.reflection, a).map(|_| ())
            };
            assert!(r.is_ok(), "{name}: {r:?}");
            (a.used() - after_front, a.peak())
        })
        .unwrap();
        let (chain, peak) = peak;
        println!("  {name}: the chain holds {chain} bytes after the front end; arena peak {peak}");
        assert!(chain < bound, "{name}: the chain holds {chain} bytes, past {bound}");
        /* The test arena is larger than the product's: "it compiled" here is
         * not "it fits"; the peak against the product's size is. */
        assert!(peak <= store::SCRATCH_BYTES, "{name}: a peak of {peak} bytes does not fit the product's arena");
    }
}

/// gl_test case 100's shaders, on the host: each stage has a texture and a
/// dynamically indexed uniform array, so the indexed read must be addressed
/// past the sampler descriptors (18ae1599). The vertex shader goes through
/// the survey's whole per-source check (every group read back and attested,
/// ours against selection and against the IR over their runs); the fragment
/// shader's four indices read four different pixels, ours equal to the IR.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn gl_test_case_100_reads_past_the_descriptors_in_both_stages() {
    let vs = include_str!("../tools/shaders/es300/idxtex.vert");
    let fs = include_str!("../tools/shaders/es300/idxtex.frag");
    let (corpus, _) = corpus_skeletons();
    let _scratch = scratch_lock();
    let (_, g) = run_one(vs, Stage::Vertex, &corpus).unwrap_or_else(|e| panic!("case 100's vertex shader: {e}"));
    assert!(g.checked == g.groups && g.mismatches.is_empty(), "read back: {:?}", g.mismatches);
    assert!(g.unattested.is_empty(), "unattested: {:?}", g.unattested);
    assert!(g.executed > 0 && g.exec_failures.is_empty(), "ours against selection: {:?}", g.exec_failures);
    assert!(g.ir_unrun.is_none(), "the IR could not run it: {:?}", g.ir_unrun);
    assert!(g.ir_agreed > 0 && g.ir_failures.is_empty(), "ours against the IR: {:?}", g.ir_failures);
    drop(_scratch);
    let cases: Vec<BoxedInput> = (0..4i32)
        .map(|k| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match n {
                "fidx" => k as u32,
                "fcols" => (0.05f32 * (c as f32 + 1.0)).to_bits(),
                "v_col" => 0.5f32.to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = stacked_against_ir("ftex", fs, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert_eq!(distinct.len(), 4, "the four indices read the same value: {px:08x?}");
}

/// dEQP-GLES2.functional.shaders.loops.do_while_uniform_iterations.nested_
/// fragment (dEQP, VK-GL-CTS, Apache-2.0; the text the host run's .qpa
/// records): a loop over a uniform count inside another.
const DEQP_LOOPS_NESTED_FS: &str = r#"varying mediump vec4 v_coords;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    mediump vec4 coords = v_coords;
    mediump vec4 res = coords;
    mediump int i = 0;
    do
    {
        mediump int j = 0;
        do
            res = res.yzwx;
        while (++j < ui_three);
    } while (++i < ui_two);
    gl_FragColor = vec4(res.rgb, 1.0);
}
"#;

/// dEQP-GLES2.functional.shaders.loops.while_dynamic_iterations.nested_
/// sequence_vertex: two loops bounded by an attribute inside a third.
const DEQP_LOOPS_NESTED_SEQUENCE_VS: &str = r#"attribute highp vec4 a_position;
attribute highp vec4 a_coords;
attribute mediump float a_one;
varying mediump vec3 v_color;
uniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four, ui_five, ui_six;

void main()
{
    gl_Position = a_position;
    mediump int one = int(a_one + 0.5);
    mediump vec4 coords = a_coords;
    mediump vec4 res = coords;
    mediump int i = 0;
    while (i++ < one*ui_three)
    {
        mediump int j = 0;
        while (j++ < one*ui_one)
            res = res.yzwx;
        while (j++ < one*ui_three)
            res = res.yzwx;
    }
    v_color = res.rgb;
}
"#;

/// A loop kept as a loop inside another (step 4 of the loops design): each
/// has its own counter, flags and -- under the execution mask -- its own
/// level of the one emc. dEQP's nested loop shapes (48 of the 625
/// shaders.loops cases still refused after 38ebdc15, "a loop too long to
/// unroll") compile: the fragment one equal to the IR over uniform counts
/// that vary both loops' trips, the vertex one through the survey's whole
/// per-source check, and the program holds two back edges.
#[test]
fn a_loop_kept_as_a_loop_inside_another() {
    let cases: Vec<BoxedInput> = [(2i32, 3i32), (0, 3), (3, 0), (1, 1), (4, 5)]
        .into_iter()
        .map(|(two, three)| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match n {
                "ui_two" => two as u32,
                "ui_three" => three as u32,
                "ui_one" => 1,
                "v_coords" => [0.1f32, 0.3, 0.6, 0.9][c as usize].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(DEQP_LOOPS_NESTED_FS, &refs);
    let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
    assert!(distinct.len() >= 3, "the trip counts read too few distinct pixels: {px:08x?}");
    assert_eq!(backward_branches(DEQP_LOOPS_NESTED_FS), 2, "two loops kept as loops, one inside the other");
    let (corpus, _) = corpus_skeletons();
    let _scratch = scratch_lock();
    let (_, g) = run_one(DEQP_LOOPS_NESTED_SEQUENCE_VS, Stage::Vertex, &corpus).unwrap_or_else(|e| panic!("{e}"));
    assert!(g.checked == g.groups && g.mismatches.is_empty(), "{:?}", g.mismatches);
    assert!(g.unattested.is_empty(), "{:?}", g.unattested);
    assert!(g.executed > 0 && g.exec_failures.is_empty(), "{:?}", g.exec_failures);
    /* The survey's inputs are random, and an attribute-derived bound can be
     * huge: both loops then run to the cap, 4096 x 4096 passes, past the IR
     * evaluator's million steps. The runs before that agreed; that one
     * reason for stopping is accepted, and nothing may disagree. */
    let stopped_by_budget = g.ir_unrun.as_deref().is_none_or(|u| u == "more than a million steps");
    assert!(
        stopped_by_budget && g.ir_agreed > 0 && g.ir_failures.is_empty(),
        "unrun {:?}, agreed {}, failures {:?}",
        g.ir_unrun,
        g.ir_agreed,
        g.ir_failures
    );
}

/// gl_test case 104's shader, on the host: at the centre of each of the
/// eight bands, our program (under the execution mask) and the IR give the
/// colour the board case expects -- band b loops b times, band 0 never
/// enters, band 7 stops at the cap's 4096 -- and the program holds the
/// mask. What the host cannot show, bands leaving at different passes in
/// one task, is what the board case is for.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn gl_test_case_104_counts_what_the_board_case_expects() {
    let src = include_str!("../tools/shaders/es300/divloop.frag");
    let cases: Vec<BoxedInput> = (0..8u32)
        .map(|b| -> BoxedInput {
            let s = (b as f32 + 0.5) / 8.0;
            Box::new(move |name: &str, _c: u32| if name == "v_s" { s.to_bits() } else { 0 })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(src, &refs);
    /* What gl_test.c's case_divloop expects, AABBGGRR. */
    let want = |b: u32| -> u32 {
        let n = if b == 7 { 4096 } else { b };
        let (r, g, bl) = ((n % 16) * 16, ((n / 16) % 16) * 16, (n / 256).min(15) * 16);
        0xff00_0000 | bl << 16 | g << 8 | r
    };
    for (b, p) in px.iter().enumerate() {
        let w = want(b as u32);
        for sh in [0, 8, 16, 24] {
            assert!(
                (p >> sh & 0xff).abs_diff(w >> sh & 0xff) <= 1,
                "band {b}: {p:08x}, the board case expects {w:08x}"
            );
        }
    }
    /* The nested half: a loop inside a loop, both per pixel, a continue and
     * a break at each level. The counts gl_test.c recomputes in C. */
    let nested = include_str!("../tools/shaders/es300/divnest.frag");
    let px = fs_against_ir(nested, &refs);
    let counts = [0u32, 0, 0, 2, 3, 6, 6, 8];
    let outer = [0u32, 1, 2, 3, 4, 4, 2, 2];
    for (b, p) in px.iter().enumerate() {
        let n = counts[b];
        let w = ((outer[b] * 32) << 24) | ((((n / 16) % 16) * 16) << 8) | ((n % 16) * 16);
        for sh in [0, 8, 16, 24] {
            assert!(
                (p >> sh & 0xff).abs_diff(w >> sh & 0xff) <= 1,
                "nested, band {b}: {p:08x}, the board case expects {w:08x}"
            );
        }
    }
    assert_eq!(backward_branches(nested), 2, "the nested half keeps both loops");
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles under the mask");
        // SAFETY: the record's code lives in `keep`.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let names = op_names(&image);
        for want in ["cndst", "cndsm", "cndlt"] {
            assert!(names.contains(&want), "case 104's program has no {want}");
        }
    })
    .unwrap();
}

/// `gl_FragCoord.xy` is the pixel-centre special register (`x_p`, `y_p`),
/// or the sample one (`x_s`, `y_s`) when the draw is multisampled, as pco
/// reads it; y then goes through the driver's pair at the record's
/// `fragcoord_ytf_off1` (one-based): unchanged under (1, 0), mirrored under
/// (-1, height) -- gl_test case 82 measured the +0.5 already in the
/// registers and only y mirrored on the window.
#[test]
fn gl_frag_coord_is_the_centre_register_with_y_through_the_drivers_pair() {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a
            .alloc_str(
                "precision highp float; void main(){ \
                 gl_FragColor = vec4(gl_FragCoord.x / 64.0, gl_FragCoord.y / 64.0, 0.0, 1.0); }",
            )
            .unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        assert_eq!(rec.fragcoord_ytf_off1, 1, "the pair straight after the (empty) uniform block");
        let pair = rec.fragcoord_ytf_off1 as usize - 1;
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (corpus, _) = usc_exec::corpus_skeletons();
        let mut off = 0usize;
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            let mut k = String::new();
            skeleton(&mut k, &g).unwrap();
            assert!(corpus.contains(&k), "an unattested group at {off:#x}: {k}");
            off += g.total;
        }
        let (x_p, x_s, y_p, y_s) = (20.5f32, 20.25f32, 10.5f32, 10.75f32);
        for (samples, flip) in [(1u32, false), (1, true), (4, true)] {
            let mut inp = Inputs::new(1);
            inp.frag_xy = [x_p.to_bits(), x_s.to_bits(), y_p.to_bits(), y_s.to_bits()];
            inp.samples = samples;
            let (scale, offset) = if flip { (-1.0f32, 64.0f32) } else { (1.0, 0.0) };
            inp.shared[pair] = scale.to_bits();
            inp.shared[pair + 1] = offset.to_bits();
            let out = run_binary(&image, &inp, true).expect("runs");
            let px = *out.get(&0).expect("a colour");
            let (x, y) = if samples == 1 { (x_p, y_p) } else { (x_s, y_s) };
            let y = y * scale + offset;
            let want = |v: f32| (v / 64.0 * 255.0).round() as u32;
            let (r, g) = (px & 0xff, px >> 8 & 0xff);
            assert!(r.abs_diff(want(x)) <= 1 && g.abs_diff(want(y)) <= 1, "samples {samples} flip {flip}: {px:#x}");
        }
    })
    .unwrap();
}

/// Every driver word at once, in the order select.rs places them after a
/// uniform vec4: gl_FragCoord's pair at 4..5, the control word at 6, the
/// front-face op at 7 -- each named by its record field, the pair and the
/// op one-based -- and a run with all three written draws what they say.
#[test]
fn the_driver_words_sit_after_the_uniforms_in_one_order() {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a
            .alloc_str(
                "precision highp float; uniform vec4 u; void main(){ if (u.x > 1000.0) discard; \
                 gl_FragColor = vec4(gl_FragCoord.y / 64.0, gl_FrontFacing ? 1.0 : 0.0, u.y, 1.0); }",
            )
            .unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        assert_eq!(
            (rec.uniform_dwords, rec.fragcoord_ytf_off1, rec.meta_off, rec.fs_front_face_off1),
            (4, 5, 6, 8),
            "uniforms, pair (one-based), control word, op (one-based)"
        );
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut inp = Inputs::new(5);
        inp.shared[0] = 0.5f32.to_bits();
        inp.shared[1] = 0.25f32.to_bits();
        inp.shared[4] = (-1.0f32).to_bits();
        inp.shared[5] = 64.0f32.to_bits();
        inp.shared[6] = yakogl::shaderc::SHC_FS_META_DEFAULT;
        inp.shared[7] = 1;
        inp.frag_xy = [0, 0, 16.5f32.to_bits(), 0];
        inp.samples = 1;
        let px = *run_binary(&image, &inp, true).expect("runs").get(&0).expect("a colour");
        /* y' = 64 - 16.5 = 47.5 -> 189; op 1 on face_ccw 0: back -> 0;
         * u.y = 0.25 -> 64 */
        assert_eq!(px, 0xff40_00bd, "{px:#x}");
    })
    .unwrap();
}

/// `gl_FrontFacing` is the ISP's `face_ccw` (special 44) being zero, then
/// the driver's op chooses: 0 keeps it, 1 swaps it, 2 is always front
/// (Mesa lower_front_face; src/pvr/backend.rs writes the op per draw). The
/// op sits after the uniforms and, in a shader that discards, after the
/// control word too; the record's `fs_front_face_off1` names it, one-based.
/// Every group is one pco emitted, and all six (op, face) pairs draw what
/// the table says, with the other shared registers holding noise so a read
/// from the wrong one shows.
#[test]
fn gl_front_facing_is_face_ccw_through_the_drivers_op() {
    let green = 0xff00_ff00u32;
    let red = 0xff00_00ffu32;
    let cases = [
        (
            "precision mediump float; void main(){ \
          gl_FragColor = gl_FrontFacing ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0); }",
            None,
            0u16,
        ),
        (
            "precision mediump float; uniform vec4 u; void main(){ if (u.x > 1000.0) discard; \
          gl_FragColor = gl_FrontFacing ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0); }",
            Some(4u16),
            5,
        ),
    ];
    let (corpus, _) = usc_exec::corpus_skeletons();
    let _scratch = scratch_lock();
    for (src, meta, ff) in cases {
        store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
            assert_eq!(rec.fs_front_face_off1, ff + 1, "{src}");
            assert_eq!(rec.meta_off, meta.unwrap_or(yakogl::shaderc::SHC_NO_META), "{src}");
            // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            let mut off = 0usize;
            while off < image.len() {
                let g = decode_group(&image, off).expect("decodes");
                let mut k = String::new();
                skeleton(&mut k, &g).unwrap();
                assert!(corpus.contains(&k), "an unattested group at {off:#x}: {k}");
                off += g.total;
            }
            for (op, ccw, front) in
                [(0u32, 0u32, true), (0, 1, false), (1, 0, false), (1, 7, true), (2, 0, true), (2, 1, true)]
            {
                let mut inp = Inputs::new(3);
                inp.face_ccw = ccw;
                inp.shared[ff as usize] = op;
                if let Some(m) = meta {
                    inp.shared[m as usize] = yakogl::shaderc::SHC_FS_META_DEFAULT;
                    inp.shared[0] = 0.5f32.to_bits();
                }
                let out = run_binary(&image, &inp, true).expect("runs");
                let px = *out.get(&0).expect("a colour");
                assert_eq!(px, if front { green } else { red }, "{src}: op {op} face_ccw {ccw}: {px:#x}");
            }
        })
        .unwrap();
    }
}

/// The Qt lit materials' tangent frame (52660c1a3d65bec6.frag and its
/// kin): built from derivatives, then every vector of it multiplied by
/// `gl_FrontFacing ? 1.0 : -1.0`. On a front face the multiply is by one and
/// hides whatever it does; each output here reads one vector of the frame,
/// on both faces, against the IR.
#[test]
fn a_tangent_frame_flipped_by_gl_front_facing_matches_the_ir_on_both_faces() {
    let src = "precision highp float; varying vec3 n; varying vec2 uv; varying vec3 p; \
               void main(){ vec3 N = normalize(n); vec2 dx = dFdx(uv); vec2 dy = dFdy(uv); \
               vec3 T = ((dFdx(p) * dy.y) - (dFdy(p) * dx.y)) / vec3((dx.x * dy.y) - (dx.y * dy.x)); \
               T -= N * dot(N, T); T = normalize(T); vec3 B = cross(N, T); \
               float f = gl_FrontFacing ? 1.0 : (-1.0); N *= f; T *= f; B *= f; \
               gl_FragColor = vec4(N.x, T.y, B.z, N.z + T.x) * 0.25 + 0.5; }";
    let cases: Vec<BoxedInput> = [(1u32, 0.3f32), (0, 0.3), (1, 0.7), (0, 0.7)]
        .into_iter()
        .map(|(front, s)| -> BoxedInput {
            Box::new(move |n: &str, c: u32| match n {
                "gl_FrontFacing" => front,
                "n" => [s, 0.5, -0.2][c as usize % 3].to_bits(),
                "uv" => [0.2 + s, 0.9][c as usize % 2].to_bits(),
                "p" => [0.4, -s, 0.6][c as usize % 3].to_bits(),
                _ => 0,
            })
        })
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let px = fs_against_ir(src, &refs);
    assert_ne!(px[0], px[1], "the two faces drew the same: {px:08x?}");
}

/// How many backward branches `src`'s fragment program holds.
fn backward_branches(src: &str) -> usize {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (mut off, mut back) = (0usize, 0usize);
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            for (_, ph, r) in g.instrs() {
                let br = yakogl::shaderc::online::powervr::isa::ops_for(r, ph).any(|o| o.name == "br");
                if br && (r.get("offset").unwrap_or(0) as i32) < 0 {
                    back += 1;
                }
            }
            off += g.total;
        }
        back
    })
    .unwrap()
}

/// Source modifiers compose in the hardware's one order -- floor, abs,
/// negate -- so of the nestings GLSL can write, these fold exactly and are
/// checked against the IR on both signs: |-x|, -|x|, |floor(x)|, -floor(x).
/// `floor(-x)` and `floor(abs(x))` cannot be one modifier set and are
/// refused by name; they used to fold to -floor(x) and |floor(x)|.
#[test]
fn source_modifiers_compose_in_the_hardware_order_or_refuse() {
    let cases: Vec<BoxedInput> = [(-2.5f32, 0.75f32), (2.5, -0.75), (-0.25, 1.5), (0.0, -3.0)]
        .into_iter()
        .map(|(x, y)| Box::new(move |_: &str, k: u32| if k == 0 { x.to_bits() } else { y.to_bits() }) as BoxedInput)
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    fs_against_ir(
        "precision highp float; varying vec2 v; void main(){ gl_FragColor = vec4(abs(-v.x) * 0.25, \
         -abs(v.y) + 1.0, abs(floor(v.x)) * 0.25, -floor(v.y) * 0.25); }",
        &refs,
    );
    let _scratch = scratch_lock();
    for (src, why) in
        [("floor(-v.x)", "a floor of a negated value"), ("floor(abs(v.x))", "a floor of an absolute value")]
    {
        let text = format!("precision highp float; varying vec2 v; void main(){{ gl_FragColor = vec4({src}); }}");
        let got = store::with_scratch(|a| {
            let text = a.alloc_str(&text).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            format!("{:?}", compile_fragment(&l.module, &l.reflection, a).err())
        })
        .unwrap();
        assert!(got.contains(why), "{src}: {got}");
    }
}

/// A local's value is its value AT the load: a later store to the slot
/// must not change what an earlier load reads, in an expression, in an
/// output, or in an `if` condition read at the join. Each of these read the
/// slot's register at the USE and computed the stored-after value (`x =
/// i++` got the incremented i; `gl_FragColor = c; c = ...;` wrote the new c;
/// the inlined early return in hueToIntensity joined on its `returned` flag
/// after the arm had set it). Each against the IR through the whole chain.
#[test]
fn a_load_reads_the_value_the_slot_held_when_it_ran() {
    let cases: Vec<BoxedInput> = [(0.1f32, 0.3f32), (0.6, 0.2), (0.9, 0.55), (0.3, 0.7), (0.05, 0.62)]
        .into_iter()
        .map(|(x, y)| Box::new(move |_: &str, k: u32| if k == 0 { x.to_bits() } else { y.to_bits() }) as BoxedInput)
        .collect();
    let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
    let p = "precision highp float; varying vec2 v;";
    for body in [
        "void main(){ float i = v.x; float x = i++; gl_FragColor = vec4(x, i * 0.5, 0.0, 1.0); }",
        "void main(){ float i = v.x; gl_FragColor = vec4(i, (i = 0.9), i, 1.0); }",
        "void main(){ vec4 c = vec4(v.x, v.y, 0.5, 1.0); gl_FragColor = c; c = vec4(0.0); }",
        "float f(float a) { if (a < 0.16) { return 0.1; } else { if (a < 0.5) { return 0.8; } } return 0.2; } \
         void main(){ float r = f(v.y); gl_FragColor = vec4(r, 0.0, 0.0, 1.0); }",
    ] {
        fs_against_ir(&format!("{p} {body}"), &refs);
    }
}

/// dEQP's ES 1.00 shader tests, vertex variants, through the USC chain and
/// against the IR -- an instrument for back-end defects beyond the table's
/// shapes, run when `DEQP_PROGRAMS` names a JSON-lines file of
/// `{name, vert, frag}` (built from VK-GL-CTS `data/gles2/shaders` with
/// tools/es100_oracle.py's own `programs_of`). The `_vertex` variants write
/// what they computed to varyings, so every output dword is compared.
/// Absent the variable it measures nothing and says so.
#[test]
fn deqp_vertex_shaders_compute_what_the_ir_means() {
    let Ok(path) = std::env::var("DEQP_PROGRAMS") else {
        println!("DEQP_PROGRAMS not set: nothing measured");
        return;
    };
    let _scratch = scratch_lock();
    let text = std::fs::read_to_string(&path).expect("the programs file");
    let (mut total, mut through, mut agreed) = (0usize, 0usize, 0usize);
    let mut refused: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut differ: Vec<String> = Vec::new();
    for line in text.lines() {
        let name = line.split("\"name\": \"").nth(1).and_then(|r| r.split('"').next()).unwrap_or("?").to_string();
        if !name.ends_with("_vertex") {
            continue;
        }
        let vert = {
            let at = line.find("\"vert\": ").expect("vert") + 8;
            let rest = &line[at..];
            let mut out = String::new();
            let mut chars = rest[1..].chars();
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => match chars.next() {
                        Some('n') => out.push('\n'),
                        Some('t') => out.push('\t'),
                        Some(o) => out.push(o),
                        None => break,
                    },
                    c => out.push(c),
                }
            }
            out
        };
        total += 1;
        let r = store::with_scratch(|a| -> Result<Option<String>, String> {
            let t = a.alloc_str(&vert).map_err(|e| format!("{e:?}"))?;
            let l = lower::compile_to_ir(a, Stage::Vertex, t).map_err(|e| format!("front end: {}", e.msg))?;
            /* Matrix varyings as their columns, as the chain and the record
             * see them (`abi::with_matrix_varyings_split`). */
            let outs = yakogl::shaderc::online::powervr::abi::with_matrix_varyings_split(&l.module, a)
                .map_err(|e| format!("{e:?}"))?
                .outputs;
            let mut image = vec![0u8; 65536];
            let b = build_vertex(&l.module, PLACEHOLDER, a, &mut image).map_err(|e| format!("{e:?}"))?;
            let ids = yakogl::shaderc::online::powervr::abi::vs_id_regs(&l.module);
            let read = yakogl::shaderc::online::powervr::abi::attribs_placed(&l.module).unwrap_or(0);
            for run in 0..IR_RUNS {
                let inp = match run % 3 {
                    0 => Inputs::new(run + 1),
                    1 => Inputs::small(run),
                    _ => Inputs::positive(run),
                };
                let ir_in = usc_exec::ir::IrInputs {
                    input: &|slot, c| match slot {
                        IoSlot::Attrib(loc) => inp.vtxin[vtxin_reg(read, loc, c) as usize],
                        IoSlot::VertexId => ids.0.map_or(0, |r| inp.vtxin[r as usize]),
                        IoSlot::InstanceId => ids.1.map_or(0, |r| inp.vtxin[r as usize]),
                        _ => 0,
                    },
                    uniform: &|d| inp.shared[d as usize],
                    marker: &|_| 0,
                };
                let want = match usc_exec::ir::run_ir(&l.module, &ir_in) {
                    Ok(w) if w.undefined => continue,
                    Ok(w) => w.outputs,
                    Err(e) => return Err(format!("the IR model: {e}")),
                };
                let got = run_binary(&image[..b.len], &inp, true).map_err(|e| format!("OURS UNRUNNABLE: {e}"))?;
                for (slot, lanes) in &want {
                    let comps = match slot {
                        IoSlot::Position => 4,
                        IoSlot::PointSize => 1,
                        IoSlot::Varying(loc) => outs
                            .iter()
                            .find(|io| io.kind == IoKind::Varying && io.location == *loc)
                            .map_or(4, |io| io.components),
                        _ => 4,
                    };
                    for (lane, w) in lanes.iter().enumerate().take(comps as usize) {
                        let Some((mut w, false)) = *w else { continue };
                        if *slot == IoSlot::PointSize {
                            w = f32::from_bits(w).clamp(1.0, 511.0).to_bits();
                        }
                        let g = expected_dword(*slot, lane, outs, &l.module).and_then(|d| got.get(&d).copied());
                        if !g.is_some_and(|g| close(g, w)) {
                            return Ok(Some(format!(
                                "run {run} {slot:?}.{lane}: ours {:?} IR {}",
                                g.map(f32::from_bits),
                                f32::from_bits(w)
                            )));
                        }
                    }
                }
            }
            Ok(None)
        })
        .expect("the scratch arena");
        match r {
            Ok(None) => {
                through += 1;
                agreed += 1;
            }
            Ok(Some(d)) => {
                through += 1;
                differ.push(format!("{name}: {d}"));
            }
            Err(e) => refused.entry(e).or_default().push(name),
        }
    }
    println!("=== dEQP vertex variants through the USC chain ===");
    println!("  {total} programs, {through} through, {agreed} agree with the IR on every output");
    for (why, names) in &refused {
        println!("    {:4}  refused: {why} {:?}", names.len(), &names[..names.len().min(4)]);
    }
    for d in &differ {
        println!("    DIFFERS {d}");
    }
    assert!(total > 0, "the programs file held no vertex variant");
    assert!(differ.is_empty(), "our bytes compute something other than the IR means");
}

/// A matrix varying is recorded column by column, laid out as pco lays it
/// out: for dEQP's scalar_to_matrix.float_to_mat2 pco_shc (build-new)
/// reports `out0` (2 components, location 0, dword 4) and a second column
/// (2, location 1, dword 6), and a vertex output of 8 dwords. pco leaves
/// the second column unnamed; ours carries the front end's `out0[1]`
/// (3beded7), the name the fragment shader's declaration has too, which
/// is what the link pairs by.
#[test]
fn a_matrix_varying_is_recorded_as_its_columns_as_pco_records_it() {
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let text = a
            .alloc_str(
                "precision mediump float; attribute highp vec4 dEQP_Position; attribute float in0; \
                 varying mat2 out0; void main(){ out0 = mat2(in0); gl_Position = dEQP_Position; }",
            )
            .unwrap();
        let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| e.msg).unwrap();
        let rec =
            yakogl::shaderc::online::powervr::chain::compile_vertex(&l.module, &l.reflection, a).expect("compiles");
        let got: Vec<(String, u8, u8, u8)> = rec.varyings[..rec.num_varyings as usize]
            .iter()
            .map(|v| {
                let n: String = v.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect();
                (n, v.components, v.location, v.vtxout_off)
            })
            .collect();
        assert_eq!(got, [("out0".to_string(), 2, 0, 4), ("out0[1]".to_string(), 2, 1, 6)]);
        assert_eq!(rec.vs_output_size, 8);
    })
    .unwrap();
}

/// dEQP constant_expressions.faceforward_vec2..4 (fragment):
/// `const float cval = faceforward(vecN(1.0), vecN(1.0), vecN(1.0)).y;`
/// is -1.0 -- dot(Nref, I) = N > 0, so the result is -N.
#[test]
fn faceforward_of_a_vector_constant_is_minus_n() {
    for (t, lane) in [("vec2", ".y"), ("vec3", ".z"), ("vec4", ".w")] {
        let src = format!(
            "precision highp float; void main(){{ const float cval = faceforward({t}(1.0), {t}(1.0), {t}(1.0)){lane}; \
             gl_FragColor = vec4(cval * 0.25 + 0.5, 0.0, 0.0, 1.0); }}"
        );
        let cases: Vec<BoxedInput> = vec![Box::new(|_: &str, _: u32| 0)];
        let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
        let px = fs_against_ir(&src, &refs);
        assert_eq!(px[0] & 0xff, 64, "{t}: the IR's red is {px:08x?}, want 0.25 -> 64");
    }
}

/// `gl_FragCoord.z` and `.w`: iterated with no divide from the Z and W
/// coefficient sets, laid out as pco lays them out (pco_shc build-new,
/// 2026-09-24: Z's set at cf0 when .z is read, W's next, a varying after
/// both from cf8) and named in the record (`uses_z`, `uses_w`). Each shape
/// against the IR, with values that tell the two planes and the varying
/// apart; dEQP builtin_variable.fragcoord_xyz and fragcoord_w.
#[test]
fn gl_frag_coord_z_and_w_are_the_z_and_w_planes() {
    let cases = [
        ("vec4(gl_FragCoord.z, 0.0, 0.0, 1.0)", (1u8, 0u8)),
        ("vec4(gl_FragCoord.w, 0.0, 0.0, 1.0)", (0, 1)),
        ("vec4(gl_FragCoord.z, gl_FragCoord.w, v.x, 1.0)", (1, 1)),
        ("vec4(v.x, gl_FragCoord.w, gl_FragCoord.x * 0.0, 1.0)", (0, 1)),
    ];
    for (e, (z, w)) in cases {
        let src = format!("precision highp float; varying vec4 v; void main(){{ gl_FragColor = {e}; }}");
        {
            let _scratch = scratch_lock();
            store::with_scratch(|a| {
                let text = a.alloc_str(&src).unwrap();
                let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
                let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
                assert_eq!((rec.uses_z, rec.uses_w), (z, w), "{e}");
            })
            .unwrap();
        }
        let cases: Vec<BoxedInput> = [(0.25f32, 0.75f32, 0.5f32), (0.625, 0.125, 0.875)]
            .into_iter()
            .map(|(fz, fw, vx)| -> BoxedInput {
                Box::new(move |n: &str, c: u32| match (n, c) {
                    ("gl_FragCoord", 2) => fz.to_bits(),
                    ("gl_FragCoord", 3) => fw.to_bits(),
                    ("v", _) => vx.to_bits(),
                    _ => 0,
                })
            })
            .collect();
        let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
        let px = fs_against_ir(&src, &refs);
        assert_ne!(px[0], px[1], "{e}: the inputs reached nothing: {px:08x?}");
    }
}

/// The four gl_FragCoord.z / .w shapes iterate what pco's programs
/// iterate: every `ditr`/`ditrp`, in order, with the same coefficient, W
/// offset, count, mode and sample-rate fields -- pco_shc build-new on the
/// same expression (`#version 310 es`, `in vec4 v` at location 0,
/// 2026-09-24), its `dwords:` line pasted here. Where the programs differ
/// is register choice (pco iterates into temp 4, ours into temp 0) and the
/// colour tail, which other tests hold. No table program checks these: the
/// table's 32 readers of gl_FragCoord.z are Qt particle shaders with two
/// colour outputs, which the online path does not build yet.
#[test]
fn gl_frag_coord_z_and_w_iterate_what_pcos_programs_iterate() {
    let cases: &[(&str, &[u32])] = &[
        (
            "vec4(gl_FragCoord.z, 0.0, 0.0, 1.0)",
            &[
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00000478, 0xc0570010, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0x87008234, 0x45000000, 0x87008234, 0x46000000, 0x87008235, 0x00000180,
                0x92574700, 0x80209c06, 0x1044a040, 0xff402c00, 0x87808a34, 0x20000040,
            ],
        ),
        (
            "vec4(gl_FragCoord.w, 0.0, 0.0, 1.0)",
            &[
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00000478, 0xc0570010, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0x87008234, 0x45000000, 0x87008234, 0x46000000, 0x87008235, 0x00000180,
                0x92574700, 0x80209c06, 0x1044a040, 0xff402c00, 0x87808a34, 0x20000040,
            ],
        ),
        (
            "vec4(gl_FragCoord.z, gl_FragCoord.w, 0.0, 1.0)",
            &[
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00000478, 0xc0570010, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000,
                0x00080061, 0x80040000, 0x00040578, 0xc0570010, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104,
                0x00000008, 0x00618104, 0x00000008, 0xff6a8002, 0x87008234, 0x46000000, 0x87008235, 0x00000180,
                0x92574700, 0x80209c06, 0x1044a040, 0xff402c00, 0x87808a34, 0x20000040,
            ],
        ),
        (
            "vec4(gl_FragCoord.z, gl_FragCoord.w, 1.0, 1.0) * v",
            &[
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00000078, 0xc0570010, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000,
                0x00080061, 0x80040000, 0x00040478, 0xc0570010, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104,
                0x00000008, 0x00618104, 0x00000008, 0xff6a8002, 0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00,
                0x00080060, 0x81040000, 0x00080061, 0x80040000, 0x04090578, 0xc0570040, 0xf2b0f300, 0x00408040,
                0xff000000, 0x00608104, 0x00000008, 0x00618104, 0x00000008, 0xff6a8002, 0x40008235, 0x0000a5c0,
                0x8235ff40, 0xa6c44000, 0xff410000, 0x87008234, 0x42000047, 0x87008234, 0x43000048, 0x9c06925a,
                0xa0408020, 0x2c001040, 0xfff3ff44, 0xffffffff, 0x87808a34, 0x20000044,
            ],
        ),
    ];
    let _scratch = scratch_lock();
    for (e, pco) in cases {
        let src = format!("precision highp float; varying vec4 v; void main(){{ gl_FragColor = {e}; }}");
        store::with_scratch(|a| {
            let text = a.alloc_str(&src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
            // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
            let ours = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let iterates = |words: &[u32]| -> Vec<String> {
                let image: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                let (mut off, mut out) = (0usize, Vec::new());
                while off < image.len() {
                    let g = decode_group(&image, off).expect("decodes");
                    for (_, ph, r) in g.instrs() {
                        for o in yakogl::shaderc::online::powervr::isa::ops_for(r, ph) {
                            if o.name == "ditr" || o.name == "ditrp" {
                                let f = |n: &str| r.get(n).map_or(-1, |v| v as i64);
                                out.push(format!(
                                    "{} coff {} woff {} count {} mode {} drc {} sat {}",
                                    o.name,
                                    f("coff"),
                                    f("woff"),
                                    f("count"),
                                    f("mode"),
                                    f("drc"),
                                    f("sat")
                                ));
                            }
                        }
                    }
                    off += g.total;
                }
                out
            };
            let (o, t) = (iterates(ours), iterates(pco));
            assert!(!t.is_empty(), "{e}: pco's program iterates nothing?");
            assert_eq!(o, t, "{e}");
        })
        .unwrap();
    }
}

/// A cube sample whose result nothing reads is deleted with its third
/// coordinate. With the native cube, selection carries z in a group of its
/// own before the sample (`SelOp::SampleZ`: a group has three sources), and
/// dead-code removal kept that group as an effect after deleting the
/// sample; the chain found no sample to fold it into and the assembler
/// refused the program with "a cube coordinate the chain did not consume".
/// That was the AM62P's shaders.random regression of step C (2026-09-25):
/// eleven dEQP sources, each with a dead `textureCube`, in
/// tests/fixtures/deqp_random_dead_cube (README.md there).
///
/// The minimal pair pins the rule in both directions: the dead look-up
/// compiles to a program with no cube sample (the record says
/// `native_cube` 0), the same look-up made live keeps its native sample.
/// The eleven sources then compile, with the fixture's population pinned.
#[test]
fn a_dead_cube_sample_takes_its_z_with_it() {
    /* (stage, source) -> the record's native_cube, or the refusal */
    let compile = |stage: Stage, src: &str| -> Result<u8, String> {
        let _scratch = scratch_lock();
        yakogl::shaderc::pvr_set_native_cube(1);
        let got = store::with_scratch(|a| {
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("front end: {}", e.msg))?;
            let rec = if stage == Stage::Fragment {
                compile_fragment(&l.module, &l.reflection, a)
            } else {
                yakogl::shaderc::online::powervr::chain::compile_vertex(&l.module, &l.reflection, a)
            };
            rec.map(|r| r.native_cube).map_err(|e| format!("{e:?}"))
        })
        .unwrap();
        yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
        got
    };
    let pair = |body: &str| {
        format!(
            "precision mediump float;\nuniform samplerCube s;\nvarying vec3 d;\n\
             void main() {{ {body} }}\n"
        )
    };
    assert_eq!(
        compile(Stage::Fragment, &pair("vec4 t = textureCube(s, d * 2.0); gl_FragColor = vec4(d, 1.0);")),
        Ok(0),
        "a dead cube look-up: compiled, and no cube sample is left"
    );
    assert_eq!(
        compile(Stage::Fragment, &pair("gl_FragColor = textureCube(s, d * 2.0);")),
        Ok(1),
        "the same look-up, live: the native cube sample stays"
    );

    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/deqp_random_dead_cube");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| matches!(p.extension().and_then(|x| x.to_str()), Some("frag" | "vert")))
        .collect();
    files.sort();
    assert_eq!(files.len(), 11, "the fixture population changed: re-derive this pin from README.md");
    let mut compiled = 0;
    let mut refused = Vec::new();
    for p in &files {
        let src = std::fs::read_to_string(p).unwrap();
        assert!(src.contains("textureCube"), "{}: every fixture has its cube look-up", p.display());
        let stage = if p.extension().unwrap() == "frag" { Stage::Fragment } else { Stage::Vertex };
        match compile(stage, &src) {
            Ok(_) => compiled += 1,
            Err(e) => refused.push(format!("{}: {e}", p.file_name().unwrap().to_string_lossy())),
        }
    }
    assert_eq!((compiled, refused.len()), (files.len(), 0), "refused: {refused:#?}");
}

/// The native cube (switch `pvr_native_cube` at compile time): a samplerCube
/// sampled as the TPU's own cube, `smp` 3d with the direction's three
/// coordinates and then the level, as pco's program for the same
/// `#version 310 es` samplerCube does (pco_shc build-new, 2026-09-24, its
/// `dwords:` lines pasted here): every `smp`'s fields equal pco's, the
/// record says `native_cube`, and the program agrees with the IR. Off, the
/// same source keeps the stacked cube and says nothing.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_native_cube_samples_as_pcos_programs_do() {
    let cases: &[(&str, &[u32])] = &[
        (
            "texture(s, d)",
            &[
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00051078, 0xc0570030, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0x87008234, 0x53000000, 0x87008234, 0x54000000, 0x87008234, 0x55000000,
                0x87008234, 0x56000000, 0x87008234, 0x57000000, 0x87008234, 0x58000000, 0x87008234, 0x59000000,
                0x87008234, 0x5a000000, 0x87008234, 0x5b000000, 0x87008234, 0x5c000000, 0x87008234, 0x5d000000,
                0x87008234, 0x5e000000, 0x87008234, 0x5f000000, 0xf400a057, 0x8070806c, 0xa0808808, 0x8002ff00,
                0x9257ff6a, 0x80209c06, 0x1040a040, 0xff442c00, 0x87808a34, 0x20000044,
            ],
        ),
        (
            "textureLod(s, d, l)",
            &[
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00051078, 0xc0570030, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0x87008235, 0x000004d2, 0x82345300, 0x00008700, 0x82345400, 0x00008700,
                0x82345500, 0x00008700, 0x82345600, 0x00008700, 0x82345700, 0x00008700, 0x82345800, 0x00008700,
                0x82345900, 0x00008700, 0x82345a00, 0x00008700, 0x82345b00, 0x00008700, 0x82345c00, 0x00008700,
                0x82345d00, 0x00008700, 0x82345e00, 0x00008700, 0xa0675f00, 0x807ef400, 0x08807080, 0x00a08088,
                0xff6a8002, 0x9c06925a, 0xa0408020, 0x2c001040, 0xfff3ff44, 0xffffffff, 0x87808a34, 0x20000044,
            ],
        ),
        (
            "texture(s, d, l)",
            &[
                0xf300c057, 0x8040f2b0, 0x00000040, 0x8104ff00, 0x00080060, 0x81040000, 0x00080061, 0x80040000,
                0x00051078, 0xc0570030, 0xf2b0f300, 0x00408040, 0xff000000, 0x00608104, 0x00000008, 0x00618104,
                0x00000008, 0xff6a8002, 0x87008235, 0x000004d2, 0x82345300, 0x00008700, 0x82345400, 0x00008700,
                0x82345500, 0x00008700, 0x82345600, 0x00008700, 0x82345700, 0x00008700, 0x82345800, 0x00008700,
                0x82345900, 0x00008700, 0x82345a00, 0x00008700, 0x82345b00, 0x00008700, 0x82345c00, 0x00008700,
                0x82345d00, 0x00008700, 0x82345e00, 0x00008700, 0xa0675f00, 0x807df400, 0x08807080, 0x00a08088,
                0xff6a8002, 0x9c06925a, 0xa0408020, 0x2c001040, 0xfff3ff44, 0xffffffff, 0x87808a34, 0x20000044,
            ],
        ),
    ];
    let smps = |words: &[u32]| -> Vec<String> {
        let image: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let (mut off, mut out) = (0usize, Vec::new());
        while off < image.len() {
            let g = decode_group(&image, off).expect("decodes");
            for (_, ph, r) in g.instrs() {
                for o in yakogl::shaderc::online::powervr::isa::ops_for(r, ph) {
                    if o.name == "smp" {
                        let f = |n: &str| r.get(n).map_or(-1, |v| v as i64);
                        out.push(format!(
                            "fcnorm {} drc {} dmn {} chan {} lodm {} pplod {} proj {} nncoords {} soo {}",
                            f("fcnorm"),
                            f("drc"),
                            f("dmn"),
                            f("chan"),
                            f("lodm"),
                            f("pplod"),
                            f("proj"),
                            f("nncoords"),
                            f("soo")
                        ));
                    }
                }
            }
            off += g.total;
        }
        out
    };
    for (e, pco) in cases {
        let src = format!(
            "#version 300 es\nprecision highp float;\nuniform highp samplerCube s;\nin vec3 d;\nflat in float l;\n\
             out vec4 o;\nvoid main() {{ o = {e}; }}\n"
        );
        for on in [true, false] {
            /* the switch is process-wide: set and cleared only under the
             * compile lock, so no other test's compile sees it */
            let _scratch = scratch_lock();
            yakogl::shaderc::pvr_set_native_cube(i32::from(on));
            /* pco's cube programs take the TPU's implicit level */
            yakogl::shaderc::pvr_set_proj_lod(0);
            let got = store::with_scratch(|a| {
                let text = a.alloc_str(&src).unwrap();
                let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
                let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
                // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
                let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
                (rec.native_cube, smps(code))
            })
            .unwrap();
            yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
            yakogl::shaderc::pvr_set_proj_lod(yakogl::shaderc::PVR_PROJ_LOD_DEFAULT);
            if on {
                assert_eq!(got.0, 1, "{e}: the record says native");
                assert_eq!(got.1, smps(pco), "{e}");
            } else {
                assert_eq!(got.0, 0, "{e}: off, the stacked cube");
                assert_ne!(got.1, smps(pco), "{e}: off, a 2D sample of the stacked image");
            }
        }
    }
}

/// A native cube program (smp 3d) computes what the IR means: the three
/// forms, each on directions toward several faces, against the IR's cube.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn a_native_cube_program_agrees_with_the_ir() {
    let s = tall_sampler(48);
    for e in ["texture(SAMP, d)", "textureLod(SAMP, d, l)", "texture(SAMP, d, l)"] {
        let src = format!(
            "#version 300 es\nprecision highp float;\nuniform highp samplerCube {s};\nin vec3 d;\nflat in float l;\n\
             out vec4 o;\nvoid main() {{ o = {}; }}\n",
            e.replace("SAMP", &s)
        );
        let dirs = [[0.9f32, 0.2, -0.3], [-0.4, 0.95, 0.1], [0.3, -0.2, -0.8], [-0.7, 0.1, 0.2]];
        let cases: Vec<BoxedInput> = dirs
            .into_iter()
            .map(|dir| -> BoxedInput {
                Box::new(move |n: &str, c: u32| match n {
                    "d" => dir[c as usize % 3].to_bits(),
                    "l" => 1.0f32.to_bits(),
                    _ => 0,
                })
            })
            .collect();
        let refs: Vec<&InputFn<'_>> = cases.iter().map(|b| b.as_ref()).collect();
        let px = stacked_against_ir_with(&s, &src, &refs, true, false);
        let distinct: std::collections::BTreeSet<u32> = px.iter().copied().collect();
        assert!(distinct.len() > 1, "{e}: every direction read the same: {px:08x?}");
    }
}

/// dEQP-GLES2 light_amount.16omni's vertex shader.
const PRE_RUN_16OMNI_VS: &str = r#"struct Light
{
    vec3    position;
    vec3    diffuse;
    vec3    specular;
    vec3    attenuation;
};
uniform Light u_lights[0 + 16];
uniform mat4 u_modelviewMatrix;
uniform mat4 u_modelviewProjectionMatrix;
uniform mat3 u_normalMatrix;

attribute vec4 a_position;
attribute vec3 a_normal;

varying vec3 v_color;

float computeAttenuation(vec3 dirToLight, vec3 attenuation)
{
    float dist = length(dirToLight);
    return 1.0 / (attenuation.x + attenuation.y*dist + attenuation.z*dist*dist);
}

vec3 computeDirLight(int ndx, vec3 position, vec3 normal)
{
    Light light = u_lights[ndx];
    float cosAngle = dot(light.position, normal);
    return cosAngle * light.diffuse;
}

vec3 computeOmniLight(int ndx, vec3 position, vec3 normal)
{
    Light light = u_lights[ndx];
    vec3 dirToLight = light.position - position;
    float cosAngle = dot(normalize(dirToLight), normal);
    float atten = computeAttenuation(dirToLight, light.attenuation);
    return atten * cosAngle * light.diffuse;
}

void main()
{
    vec3 lightSpacePos = vec3(u_modelviewMatrix * a_position);
    vec3 lightNormal = normalize(u_normalMatrix * a_normal);
    vec3 color = vec3(0.0);
    for (int i = 0; i < 0; i++)
        color += computeDirLight(i, lightSpacePos, lightNormal);
    for (int i = 0; i < 16; i++)
        color += computeOmniLight(0+i, lightSpacePos, lightNormal);
    v_color = color;
    gl_Position = u_modelviewProjectionMatrix * a_position;
}
"#;

/// dEQP-GLES2 uniform_api.random.23's fragment shader.
const PRE_RUN_RANDOM23_FS: &str = r#"varying mediump float v_vtxOut;

struct structType2
{
	mediump mat4 m0;
	mediump vec3 m1;
	mediump samplerCube m2;
	mediump mat4 m3;
};
struct structType1
{
	mediump mat2 m0[3];
	mediump sampler2D m1;
	bvec4 m2[4];
	structType2 m3;
};
struct structType4
{
	mediump vec3 m0[1];
	mediump samplerCube m1;
};
struct structType5
{
	bool m0;
	mediump mat4 m1;
	mediump ivec4 m2;
};
struct structType3
{
	structType4 m0[3];
	structType5 m1[4];
};
struct structType6
{
	bool m0[5];
	mediump sampler2D m1;
};
struct structType0
{
	structType1 m0;
	mediump mat4 m1[4];
	mediump vec4 m2;
	structType3 m3[4];
	structType6 m4;
};
uniform structType0 u_var0;
uniform mediump float u_var1[5];
uniform bvec2 u_var2[2];

mediump float compare_float    (mediump float a, mediump float b)  { return abs(a - b) < 0.05 ? 1.0 : 0.0; }
mediump float compare_vec2     (mediump vec2 a, mediump vec2 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }
mediump float compare_vec3     (mediump vec3 a, mediump vec3 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z); }
mediump float compare_vec4     (mediump vec4 a, mediump vec4 b)    { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }
mediump float compare_mat2     (mediump mat2 a, mediump mat2 b)    { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }
mediump float compare_mat4     (mediump mat4 a, mediump mat4 b)    { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }
mediump float compare_ivec4    (mediump ivec4 a, mediump ivec4 b)  { return a == b ? 1.0 : 0.0; }
mediump float compare_bool     (bool a, bool b)                    { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec2    (bvec2 a, bvec2 b)                  { return a == b ? 1.0 : 0.0; }
mediump float compare_bvec4    (bvec4 a, bvec4 b)                  { return a == b ? 1.0 : 0.0; }

void main (void)
{
    mediump float result = v_vtxOut;
	result *= compare_mat2(u_var0.m0.m0[0], mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_mat2(u_var0.m0.m0[1], mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_mat2(u_var0.m0.m0[2], mat2(0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(texture2D(u_var0.m0.m1, vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bvec4(u_var0.m0.m2[0], bvec4(false, false, false, false));
	result *= compare_bvec4(u_var0.m0.m2[1], bvec4(false, false, false, false));
	result *= compare_bvec4(u_var0.m0.m2[2], bvec4(false, false, false, false));
	result *= compare_bvec4(u_var0.m0.m2[3], bvec4(false, false, false, false));
	result *= compare_mat4(u_var0.m0.m3.m0, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec3(u_var0.m0.m3.m1, vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m0.m3.m2, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_mat4(u_var0.m0.m3.m3, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[0], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[1], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[2], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_mat4(u_var0.m1[3], mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_vec4(u_var0.m2, vec4(0.00, 0.00, 0.00, 0.00));
	result *= compare_vec3(u_var0.m3[0].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[0].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[0].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[0].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[0].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[0].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[0].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[0].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[0].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[0].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[0].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[0].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec3(u_var0.m3[1].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[1].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[1].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[1].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[1].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[1].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[1].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[1].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[1].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[1].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[1].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[1].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec3(u_var0.m3[2].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[2].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[2].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[2].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[2].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[2].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[2].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[2].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[2].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[2].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[2].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[2].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_vec3(u_var0.m3[3].m0[0].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[3].m0[0].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[3].m0[1].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[3].m0[1].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_vec3(u_var0.m3[3].m0[2].m0[0], vec3(0.00, 0.00, 0.00));
	result *= compare_vec4(textureCube(u_var0.m3[3].m0[2].m1, vec3(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_bool(u_var0.m3[3].m1[0].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[0].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[0].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[3].m1[1].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[1].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[1].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[3].m1[2].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[2].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[2].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m3[3].m1[3].m0, false);
	result *= compare_mat4(u_var0.m3[3].m1[3].m1, mat4(0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00, 0.00));
	result *= compare_ivec4(u_var0.m3[3].m1[3].m2, ivec4(0, 0, 0, 0));
	result *= compare_bool(u_var0.m4.m0[0], false);
	result *= compare_bool(u_var0.m4.m0[1], false);
	result *= compare_bool(u_var0.m4.m0[2], false);
	result *= compare_bool(u_var0.m4.m0[3], false);
	result *= compare_bool(u_var0.m4.m0[4], false);
	result *= compare_vec4(texture2D(u_var0.m4.m1, vec2(0.0)), vec4(0.00, 0.12, 0.24, 0.36));
	result *= compare_float(u_var1[0], 0.00);
	result *= compare_float(u_var1[1], 0.00);
	result *= compare_float(u_var1[2], 0.00);
	result *= compare_float(u_var1[3], 0.00);
	result *= compare_float(u_var1[4], 0.00);
	result *= compare_bvec2(u_var2[0], bvec2(false, false));
	result *= compare_bvec2(u_var2[1], bvec2(false, false));
    gl_FragColor = vec4(result, result, result, 1.0);
}
"#;

/// The two shaders the AM62P's host pre-run found refused by the 2 MiB
/// compile scratch (light_amount.16omni's vertex shader needed 3 MiB,
/// uniform_api.random.23's fragment shader 4 MiB) compile inside the
/// default region, `store::SCRATCH_BYTES`, on the PowerVR online back end:
/// the product path (`store::with_scratch`), judged by its high-water mark
/// (2,529,024 and 3,455,740 bytes when this was written).
#[test]
fn the_default_scratch_compiles_the_pre_runs_two_largest_shaders() {
    let _scratch = scratch_lock();
    for (stage, src) in [(Stage::Vertex, PRE_RUN_16OMNI_VS), (Stage::Fragment, PRE_RUN_RANDOM23_FS)] {
        let r = store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, stage, text).map_err(|e| format!("{:?}", e.msg))?;
            let rec = if stage == Stage::Vertex {
                yakogl::shaderc::online::powervr::chain::compile_vertex(&l.module, &l.reflection, a)
            } else {
                compile_fragment(&l.module, &l.reflection, a)
            };
            rec.map(|_| ()).map_err(|e| format!("{e:?}"))
        })
        .expect("the scratch region");
        assert_eq!(r, Ok(()), "{stage:?}: refused");
        /* this binary lends a region four times the product's
         * (`scratch_lock`), so the product's limit is the peak's */
        let peak = store::last_scratch_peak();
        assert!(peak <= store::SCRATCH_BYTES, "{stage:?}: needs {peak} bytes, the default is {}", store::SCRATCH_BYTES);
    }
}

/// dEQP uniform_api.*.unused_uniforms.sampler2D_samplerCube_* (host
/// pre-run): a struct with `samplerCube m3[3]`, of which only `m3[1]` is
/// sampled. The record kept a sampler row only when its FIRST binding was
/// sampled, so `u_var.m3` vanished and glGetActiveUniform never reported
/// it; a whole array row is kept when any element is.
#[test]
fn a_sampler_array_row_is_kept_when_any_element_is_sampled() {
    let _scratch = scratch_lock();
    let src = "varying mediump float v; struct S { mediump sampler2D m0; mediump samplerCube m1; \
               mediump sampler2D m2[3]; mediump samplerCube m3[3]; }; uniform S u_var; \
               void main (void) { gl_FragColor = vec4(v) * texture2D(u_var.m0, vec2(0.0)) \
               * textureCube(u_var.m3[1], vec3(0.0)); }";
    let names = store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
        let rec = compile_fragment(&l.module, &l.reflection, a).expect("compiles");
        // SAFETY: the record's uniform rows live in `keep`.
        let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
        rows.iter()
            .map(|r| r.name.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect::<String>())
            .collect::<Vec<_>>()
    })
    .unwrap();
    assert_eq!(names, ["u_var.m0", "u_var.m3"], "the sampled rows, the array whole");
}

/// dEQP-GLES2 attribute_location.bind_aliasing.max_inactive_*'s shape: 32
/// attributes declared interleaved (a_0, a_16, a_1, a_17, ...), of which
/// only a_0..a_15 are read. A vertex-input register belongs to a READ
/// location (`abi::vtxin_reg` ranks among them), so the sixteen read
/// attributes fit the 64 registers; placed at 4 * location, the read ones
/// sat at locations up to 30 and the shader was refused (VtxIn needed 65,
/// AM62P host pre-run).
#[test]
fn unread_attributes_take_no_vertex_input_registers() {
    let _scratch = scratch_lock();
    let mut src = String::new();
    for i in 0..16 {
        src += &format!("attribute float a_{i};\nattribute float a_{};\n", i + 16);
    }
    src += "void main() { float s = 0.0;";
    for i in 0..16 {
        src += &format!(" s += a_{i};");
    }
    src += " gl_Position = vec4(s); }";
    let regs = store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(&src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| format!("{:?}", e.msg))?;
        let rec = yakogl::shaderc::online::powervr::chain::compile_vertex(&l.module, &l.reflection, a)
            .map_err(|e| format!("{e:?}"))?;
        Ok::<_, String>(rec.attribs[..rec.num_attribs as usize].iter().map(|x| x.vi_reg).collect::<Vec<u8>>())
    })
    .unwrap()
    .expect("sixteen read attributes compile");
    assert_eq!(regs.len(), 16, "the read attributes, and only those: {regs:?}");
    let distinct: BTreeSet<u8> = regs.iter().copied().collect();
    assert_eq!(distinct.len(), 16, "one register group each: {regs:?}");
    assert!(regs.iter().all(|r| *r < 64 && r % 4 == 0), "inside the 64, four apiece: {regs:?}");
}

/// The link-time attribute layout (`abi::with_attrib_map`) on the byte
/// model: dEQP bind_aliasing.max_cond_float's shape -- 32 float attributes,
/// `a_k` and `a_(k+16)` read under opposite conditions of `u_A` -- with the
/// map binding both to location k. Built at the product's limits (it does
/// not fit one location per attribute: 65 vertex-input registers), and run
/// against the IR of the ORIGINAL module with every attribute read from the
/// registers of the location the map gave it, so each alias pair reads one
/// value. Both conditions are driven. A map not applied to the loads, or two
/// locations sharing registers, reads another value and differs.
#[test]
fn attributes_the_link_binds_to_one_location_read_one_value() {
    use yakogl::shaderc::online::powervr::abi::with_attrib_map;
    use yakogl::shaderc::online::powervr::chain::LIMITS;
    use yakogl::shaderc::{ShcAttribBinding, ShcAttribMap};
    let mut src = String::from("varying mediump vec4 v_color;\n");
    for k in 0..16 {
        src += &format!("attribute mediump float a_{k};\nattribute mediump float a_{};\n", k + 16);
    }
    src += "uniform mediump float u_A;\nvoid main (void)\n{\n\tmediump vec4 color = vec4(0.0);\n";
    for k in 0..16 {
        src += &format!(
            "\tif (u_A != 0.0) color += vec4(a_{k}) * {}.0;\n\tif (u_A == 0.0) color += vec4(a_{});\n",
            k + 1,
            k + 16
        );
    }
    src += "\tv_color = color;\n\tgl_Position = color;\n}\n";
    let _scratch = scratch_lock();
    store::with_scratch(|a| {
        let t = a.alloc_str(&src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Vertex, t).map_err(|e| e.msg).unwrap();
        let mut image = vec![0u8; 65536];
        assert!(
            matches!(
                build_vertex(&l.module, LIMITS, a, &mut image),
                Err(ChainError::Alloc(yakogl::shaderc::online::powervr::alloc::AllocError::OutOfRegisters { .. }))
            ),
            "one location per attribute must not fit, or this measures nothing"
        );
        let mut map = ShcAttribMap::ZERO;
        map.count = l.reflection.attribs.len() as u8;
        for (b, r) in map.b.iter_mut().zip(l.reflection.attribs) {
            let k: u8 = r.name[2..].parse().unwrap();
            let mut name = [0 as core::ffi::c_char; 32];
            for (c, ch) in name.iter_mut().zip(r.name.bytes()) {
                *c = ch as _;
            }
            *b = ShcAttribBinding { name, location: k % 16, columns: 1, gl_type: r.gl_type, active: 1 };
        }
        let (m, rows) = with_attrib_map(a, &l.module, l.reflection.attribs, &map).expect("mapped");
        let b = build_vertex(m, LIMITS, a, &mut image).unwrap_or_else(|e| panic!("sixteen locations fit: {e:?}"));
        let read = yakogl::shaderc::online::powervr::abi::attribs_placed(m).unwrap();
        assert_eq!(read, 0xffff, "sixteen locations placed");
        /* each original location's new one */
        let moved = |old: u8| {
            let i = l.reflection.attribs.iter().position(|r| r.location == old).expect("a declared location");
            rows[i].location
        };
        let outs = yakogl::shaderc::online::powervr::abi::with_matrix_varyings_split(m, a).unwrap().outputs;
        let mut compared = 0;
        for run in 0..8u32 {
            let mut inp = Inputs::positive(run);
            inp.shared[0] = if run % 2 == 0 { 0.0f32 } else { 1.0f32 }.to_bits();
            let ir_in = usc_exec::ir::IrInputs {
                input: &|slot, c| match slot {
                    IoSlot::Attrib(loc) => inp.vtxin[vtxin_reg(read, moved(loc), c) as usize],
                    _ => 0,
                },
                uniform: &|d| inp.shared[d as usize],
                marker: &|_| 0,
            };
            let want = usc_exec::ir::run_ir(&l.module, &ir_in).expect("the IR runs");
            let got = run_binary(&image[..b.len], &inp, true).expect("ours runs");
            for (slot, lanes) in &want.outputs {
                for (lane, w) in lanes.iter().enumerate() {
                    let Some((w, false)) = *w else { continue };
                    let g = expected_dword(*slot, lane, outs, m).and_then(|d| got.get(&d).copied());
                    assert!(
                        g.is_some_and(|g| close(g, w)),
                        "run {run} {slot:?}.{lane}: ours {:?}, the IR {}",
                        g.map(f32::from_bits),
                        f32::from_bits(w)
                    );
                    compared += 1;
                }
            }
        }
        assert!(compared >= 8 * 8, "position and colour, every run: {compared}");
    })
    .unwrap();
}

/// Several colour outputs (`layout(location = n) out`): each is packed to
/// RGBA8 and written to pixel output register n, which is where the
/// end-of-tile program emits attachment n from when every attachment is
/// 8-bit (src/pvr/backend.rs, one register per 8-bit attachment). Declared
/// out of location order, with a location left unwritten, and with a
/// varying, so a write to the declaration's index instead of its location,
/// a pack of the wrong output or a lost prologue each shows. Ours against
/// the IR on every written location; nothing is written anywhere else; the
/// record spans the highest location plus one.
#[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
#[test]
fn several_colour_outputs_are_packed_to_their_own_pixel_output_registers() {
    let sources: [(&str, &[u16]); 2] = [
        (
            "#version 300 es
precision highp float;
uniform vec4 a;
uniform vec4 b;
uniform vec4 c;
in vec4 v;
layout(location = 2) out vec4 o2;
layout(location = 0) out vec4 o0;
layout(location = 1) out vec4 o1;
void main() { o2 = a * v; o0 = b; o1 = c.wzyx * 0.5 + v * 0.25; }
",
            &[0, 1, 2],
        ),
        (
            "#version 300 es
precision highp float;
uniform vec4 a;
in vec4 v;
layout(location = 3) out vec4 o3;
layout(location = 1) out vec4 o1;
void main() { o3 = v.yxwz; o1 = a + v * 0.125; }
",
            &[1, 3],
        ),
    ];
    let _scratch = scratch_lock();
    let name_of =
        |n: &[core::ffi::c_char]| -> String { n.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect() };
    for (src, locs) in sources {
        store::with_scratch(|a| {
            let _keep = store::begin_keep().expect("keep");
            let text = a.alloc_str(src).unwrap();
            let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| e.msg).unwrap();
            let rec = compile_fragment(&l.module, &l.reflection, a)
                .unwrap_or_else(|e| panic!("the chain refused it: {e:?}\n{src}"));
            assert_eq!(rec.num_outputs as u16, locs.last().unwrap() + 1, "the registers the outputs span");
            // SAFETY: the record's code and uniform rows live in `keep`.
            let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
            let rows = unsafe { core::slice::from_raw_parts(rec.uniforms, rec.num_uniforms as usize) };
            let image: Vec<u8> = code.iter().flat_map(|w| w.to_le_bytes()).collect();
            for seed in 0..8u32 {
                /* values in 0..1, distinct per name and component */
                let val = |n: &str, k: u32| -> u32 {
                    let h = n.bytes().fold(seed.wrapping_mul(977) ^ k.wrapping_mul(131), |h, b| {
                        h.wrapping_mul(31).wrapping_add(u32::from(b))
                    });
                    ((h % 997) as f32 / 997.0).to_bits()
                };
                let mut inp = sentinel_inputs(1.0e30);
                place_shared(&mut inp, rows, &val);
                let mut at = 4usize;
                for vy in &rec.varyings[..rec.num_varyings as usize] {
                    for c in 0..vy.components as usize {
                        let w = val(&name_of(&vy.name), c as u32);
                        inp.coeff[at + 4 * c] = w;
                        inp.coeff[at + 4 * c + 2] = w;
                    }
                    at += 4 * vy.components as usize;
                }
                let ours = run_binary(&image, &inp, true).expect("ours runs");
                let ir_in = usc_exec::ir::IrInputs {
                    input: &|slot, c| match slot {
                        IoSlot::Varying(loc) => {
                            l.module.inputs.iter().find(|io| io.location == loc).map_or(0, |io| val(io.name, c))
                        }
                        _ => 0,
                    },
                    uniform: &|d| {
                        let u =
                            l.reflection.uniforms.iter().find(|u| (u.dword_off..u.dword_off + u.dwords).contains(&d));
                        u.map_or(0, |u| val(u.name, (d - u.dword_off) as u32))
                    },
                    marker: &|_| 0,
                };
                let want = usc_exec::ir::run_ir(&l.module, &ir_in).expect("the IR runs");
                let written: Vec<u32> = ours.keys().copied().collect();
                let expect: Vec<u32> = locs.iter().map(|&x| u32::from(x)).collect();
                assert_eq!(written, expect, "the pixel output registers written\n{src}");
                for &loc in locs {
                    let (_, lanes) = want
                        .outputs
                        .iter()
                        .find(|(s, _)| *s == IoSlot::FragColor(loc as u8))
                        .unwrap_or_else(|| panic!("the IR stores location {loc}"));
                    let mut px = 0u32;
                    for (c, lane) in lanes.iter().enumerate() {
                        let x = f32::from_bits(lane.expect("stored").0);
                        px |= ((x.clamp(0.0, 1.0) * 255.0).round() as u32) << (8 * c);
                    }
                    let got = ours[&u32::from(loc)];
                    let close =
                        (0..4).all(|c| (((got >> (8 * c)) & 0xff) as i32 - ((px >> (8 * c)) & 0xff) as i32).abs() <= 1);
                    assert!(close, "seed {seed} location {loc}: ours {got:08x}, the IR {px:08x}\n{src}");
                }
            }
        })
        .unwrap();
    }
}

/// The background object's clear programs for one to three output dwords
/// (src/pvr/blobs.rs; three is built by our encoder, not captured)
/// on the byte model: pixel output k holds shared register k for every k
/// below the count, nothing else is written, and the program ends at the
/// end of its image. The shared file holds distinct values, so a move from
/// the wrong register or to the wrong output shows.
#[test]
fn the_clear_programs_move_shared_k_to_pixel_output_k() {
    use yakogl::pvr::blobs::*;
    let programs: [&[u32]; 3] = [&PVR_USC_CLEAR_FS_WORDS, &PVR_USC_CLEAR_FS2_WORDS, &PVR_USC_CLEAR_FS3_WORDS];
    for (i, words) in programs.iter().enumerate() {
        let n = i as u32 + 1;
        let image: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        let mut inp = Inputs::new(7);
        for k in 0..8 {
            inp.shared[k] = 0x1100_0000 + k as u32;
        }
        let out = run_binary(&image, &inp, true).unwrap_or_else(|e| panic!("{n} dword(s): {e}"));
        let want: BTreeMap<u32, u32> = (0..n).map(|k| (k, 0x1100_0000 + k)).collect();
        assert_eq!(out, want, "{n} dword(s): the pixel outputs written");
    }
}

/// dEQP-GLES2 shaders.random.all_features.fragment.53, failing on the AM62P
/// since at least 2026-09-25: `const int n = ivec2(exp(o), sqrt(float(12.25))
/// + (-1.5)).g` is 2 in dEQP's reference, and `n * m.s >= int(float(l)) - 2`
/// (2 >= 2) chooses `a`, whose `exp(a)` is the picture. On the USC `sqrt` is
/// `frcp(frsq(x))`: 1 / (1 / 3.5) is 3.4999998 even correctly rounded, `n`
/// truncated to 1 and the whole quad drew `a`'s initial value (the host
/// model: ours 789b2378 everywhere, the IR's exp(a) varying). Folded when
/// compiling (`select.rs` `fold_constants`), `n` is 2 as the IR says.
///
/// Revert check: drop the `fold_constants` call in `select` and this reads
/// red (ours ff0000ff, the IR ff00ff00).
#[test]
fn a_constant_square_root_truncates_as_the_exact_one_does() {
    let px = fs_against_ir(
        "precision mediump float;\n\
         const int n = ivec2(0, sqrt(float(12.25)) + (-1.5)).g;\n\
         void main(){ gl_FragColor = n == 2 ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0); }",
        &[&|_: &str, _: u32| 0u32],
    );
    assert_eq!(px[0], 0xff00ff00);
}

/// dEQP-GLES2 shaders.random.all_features.vertex.39 (AM62P): three constants
/// its picture depends on exactly, each computed by an approximate unit when
/// it ran on the USC. Replacing each by its neighbouring float in the source
/// changes the IR's picture (host, 2026-09-30): `sin(float(false))`
/// compared with 0 in `const bool y` (all of `v_color.a`), `int(sqrt(9.0))`
/// in the global `f` compared with `!=`, and `int(inversesqrt(exp(0.0)))`
/// in `u` and `w`. Each is folded when compiling now: no `fred`/`fsinc`,
/// `frsq`/`frcp` or `fexp` is left, and the values are the IR's.
///
/// Revert check: drop the `fold_constants` call in `select` and the four
/// units come back.
#[test]
fn constants_the_approximate_units_would_round_fold_exactly() {
    let consts = "const bool y = vec2(0.0, -1.0) == vec2(sin(float(false)), -1);\n\
                  ivec4 f = ivec4(1, sqrt(vec2(-9, -18) * vec2(-1.0, false)).s, 0, 0);\n\
                  const float t = inversesqrt(exp(vec4(1, 1, 0, 1)).zy).s;\n";
    let colour = "vec4(float(f.y) * 0.25, float(int(t)) * 0.5, float(y), 1.0)";
    let vs = format!(
        "attribute vec4 p;\nvarying mediump vec4 v_color;\n{consts}\
         void main() {{ gl_Position = p; v_color = {colour}; }}"
    );
    {
        let _scratch = scratch_lock();
        store::with_scratch(|a| {
            let text = a.alloc_str(&vs).unwrap();
            let l = lower::compile_to_ir(a, Stage::Vertex, text).map_err(|e| e.msg).unwrap();
            let mut image = vec![0u8; 65536];
            let b = build_vertex(&l.module, PLACEHOLDER, a, &mut image).expect("builds");
            let ops = op_names(&image[..b.len]);
            for unit in ["fred", "fsinc", "frsq", "frcp", "fexp", "flog"] {
                assert!(!ops.contains(&unit), "{unit} left: {ops:?}");
            }
        })
        .unwrap();
    }
    /* The values, through the fragment path against the IR: 3, 1 and true. */
    let px = fs_against_ir(
        &format!("precision mediump float;\n{consts}void main() {{ gl_FragColor = {colour}; }}"),
        &[&|_: &str, _: u32| 0u32],
    );
    assert_eq!(px[0], 0xffff_80bf, "f.y = 3, int(t) = 1, y");
}
