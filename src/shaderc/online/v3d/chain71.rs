// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The V3D 7.1 chain: [`super::select71`], [`super::alloc71`] and
//! [`super::sched71_cf`] run on one selected shader at 4 threads, then 2,
//! and the words encoded ([`super::codec71::encode71`]) and verified
//! ([`super::verify71::verify71`]), as `docs/notes/v3d71-backend.md`
//! section (a) and the contract in [`super::vop71`]'s header say. A program
//! with no control-flow [`Op71`] is packed by [`super::sched71::pack71`]
//! directly, which gives the words schedule71 would ([`build71`]); its
//! branches are checked by [`branches_are_structured71`].
//!
//! The front half is chain.rs's and shared: unrolling, prepare and the 4.2
//! selection run unchanged, and their refusals come back as
//! [`V3d71Error::Chain`] with the same [`V3dError`] as at 4.2. Until
//! `chain-hook` gives chain.rs the generation as a parameter,
//! [`compile_program71_keyed`] carries its own copy of
//! `compile_program_keyed`'s orchestration, built from public functions
//! only; each copied block says "chain-hook replaces this" and cites the
//! chain.rs lines it copies (at `6a4def08`).
//!
//! [`compile_program_for`] is the generation as the chain's parameter:
//! [`Gen::V42`] is [`chain::compile_program_keyed`], unchanged.
//!
//! What a 7.1 stage does not compile yet comes back as that stage's named
//! refusal, never a panic and never a wrong word.

use super::alloc71::{allocate71, Alloc71Error};
use super::chain::{self, attribute_locations, Prim, Target, V3dError, V3dProgram, VARYING_SLOT_VAR0};
use super::codec71::encode71;
use super::encode::EncodeError;
use super::prepare::{fragment_inputs, prepare_fragment_with, prepare_vertex_with, Prepared, SOp};
use super::qpu_ffi::Gen;
use super::sched71::{pack71, Sched71Error};
use super::sched71_cf::{branches_are_structured71, schedule71, Scheduled71};
use super::select::{
    select_fragment_keyed, select_vertex_keyed, tmu_input_writes, Pressure, SelectError, Selected, VOp,
};
use super::select71::{interleave71, interleave71_sig, select71, Select71Error};
use super::verify71::{registers_fit_threads71, verify71, VerifyError};
use super::vop71::{AddOp71, Dst71, Instr71, MulOp71, Op71, Selected71, Src71, Uniform};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};
use crate::shaderc::online::ir::{self, IoKind};
use crate::shaderc::online::powervr::unroll::Trip;
use crate::shaderc::LodKey;

/// Why a program was not compiled at 7.1. One variant per stage.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum V3d71Error {
    /// A construct or stage not compiled at 7.1 yet, by name.
    NotYet(&'static str),
    /// The stages shared with 4.2 (unrolling, prepare, 4.2 selection, the
    /// program's own refusals), chain.rs's error as it is.
    Chain(V3dError),
    Select(Select71Error),
    /// [`Alloc71Error::DoesNotFit`] here: the values did not fit 2 threads'
    /// registers at any selection pressure, in select71's order or
    /// [`sunk71`]'s (no spilling yet).
    Alloc(Alloc71Error),
    Sched(Sched71Error),
    /// Instruction `.0` does not encode.
    Encode(usize, EncodeError),
    Verify(VerifyError),
    OutOfScratch,
}

impl From<ArenaError> for V3d71Error {
    fn from(_: ArenaError) -> Self {
        V3d71Error::OutOfScratch
    }
}
impl From<V3dError> for V3d71Error {
    fn from(e: V3dError) -> Self {
        V3d71Error::Chain(e)
    }
}
impl From<SelectError> for V3d71Error {
    /// The 4.2 selection is shared: its refusal is chain.rs's.
    fn from(e: SelectError) -> Self {
        V3d71Error::Chain(e.into())
    }
}
impl From<Select71Error> for V3d71Error {
    fn from(e: Select71Error) -> Self {
        match e {
            Select71Error::OutOfScratch => V3d71Error::OutOfScratch,
            e => V3d71Error::Select(e),
        }
    }
}
impl From<Alloc71Error> for V3d71Error {
    fn from(e: Alloc71Error) -> Self {
        match e {
            Alloc71Error::OutOfScratch => V3d71Error::OutOfScratch,
            e => V3d71Error::Alloc(e),
        }
    }
}
impl From<Sched71Error> for V3d71Error {
    fn from(e: Sched71Error) -> Self {
        match e {
            Sched71Error::OutOfScratch => V3d71Error::OutOfScratch,
            e => V3d71Error::Sched(e),
        }
    }
}

/// One compiled 7.1 QPU program.
#[derive(Clone, Copy, Debug)]
pub struct Code71<'a> {
    pub words: &'a [u64],
    /// The instructions the words encode.
    pub instrs: &'a [Instr71],
    /// The uniform stream, `(contents, data)` with Mesa's quniform_contents.
    pub uniforms: &'a [Uniform],
    /// Every counted loop's back edge (branch, target instruction).
    pub back_edges: &'a [(u32, u32)],
    /// Every forward branch (branch, target instruction).
    pub fwd_branches: &'a [(u32, u32)],
}

/// One compiled shader stage.
#[derive(Clone, Copy, Debug)]
pub struct Stage71<'a> {
    pub code: Code71<'a>,
    pub threads: u32,
    pub single_seg: bool,
}

/// chain.rs [`chain::FsProg`] field for field, `code: Code71`.
#[derive(Clone, Copy, Debug)]
pub struct FsProg71<'a> {
    pub code: Code71<'a>,
    pub threads: u32,
    pub single_seg: bool,
    pub num_inputs: u32,
    pub input_slots: &'a [(u8, u8)],
    pub flat_shade_flags: u32,
    pub noperspective_flags: u32,
    pub centroid_flags: u32,
    pub flat_shade_flags_hi: u32,
    pub noperspective_flags_hi: u32,
    pub centroid_flags_hi: u32,
    pub writes_z: bool,
    pub disable_ez: bool,
    pub uses_center_w: bool,
    pub lock_scoreboard_on_first_thrsw: bool,
    pub uses_implicit_point_line_varyings: bool,
    pub uses_pid: bool,
    pub rt_mask: u8,
    pub force_per_sample_msaa: bool,
    pub tmu_dirty_rcl: bool,
    pub uses_discard: bool,
}

/// chain.rs [`chain::VsProg`] field for field, `code: Code71`.
#[derive(Clone, Copy, Debug)]
pub struct VsProg71<'a> {
    pub code: Code71<'a>,
    pub threads: u32,
    pub single_seg: bool,
    pub vpm_input_size: u32,
    pub vpm_output_size: u32,
    pub separate_segments: bool,
    pub vcm_cache_size: u32,
    pub uses_vid: bool,
    pub uses_iid: bool,
    pub uses_biid: bool,
    pub writes_psiz: bool,
    pub vattr_sizes: [u8; 16],
    pub tmu_dirty_rcl: bool,
}

/// A compiled GL program at 7.1: chain.rs [`V3dProgram`] field for field.
#[derive(Clone, Copy, Debug)]
pub struct V3dProgram71<'a> {
    pub fs: FsProg71<'a>,
    pub vs: VsProg71<'a>,
    pub cs: VsProg71<'a>,
    pub attributes: &'a [(u8, Option<u8>)],
}

/// A compiled GL program of either generation.
#[derive(Clone, Copy, Debug)]
pub enum AnyV3dProgram<'a> {
    V42(V3dProgram<'a>),
    V71(V3dProgram71<'a>),
}

/// The VPM at 7.1: the 64 KiB H2 gave v3d_shc (tools/v3d/README.md, "H2");
/// the Pi 5's own `IDENT1.VPM_SIZE` is unmeasured (pre-study U2/D2).
pub const TARGET71: Target = Target { vpm_size: 65536 };

/// A program all the way to its 7.1 words, and the thread count it runs at:
/// 4 when its values fit 4 threads' registers and no look-up writes more
/// than 16 / 4 TMU inputs, else 2 (chain.rs `build`, chain.rs:277-322).
/// Then encode71 of every word, verify71 and registers_fit_threads71.
///
/// The operations go through two or three times: in select71's order, in
/// [`interleave71`]'s, which sched71 pairs better, and for a fragment shader
/// in [`interleave71_sig`]'s when that is another order. An interleaved
/// build replaces the one kept so far when it is no fewer threads and fewer
/// words, and gives its scratch back otherwise; the in-order one decides
/// every refusal (its error is the stage's), and each interleaved one is
/// tried only when the scratch left holds twice what the first took. (Over
/// es100 and qpu_corpus_71 the signal-aware order never shortened a vertex
/// or coordinate shader, so those are not built a third time.)
pub fn build71<'a>(sel: &Selected<'_>, single_seg: bool, a: &Arena<'a>) -> Result<(Code71<'a>, u32), V3d71Error> {
    build71_order(sel, single_seg, false, true, a)
}

/// [`build71`], with select71's operations first [`sunk71`] when `sink`,
/// and the in-order build alone unless `orders` (no interleaved one).
fn build71_order<'a>(
    sel: &Selected<'_>,
    single_seg: bool,
    sink: bool,
    orders: bool,
    a: &Arena<'a>,
) -> Result<(Code71<'a>, u32), V3d71Error> {
    let mut sel71 = select71(sel, a)?;
    if sink {
        sel71 = sunk71(&sel71, a)?;
    }
    let four = tmu_input_writes(sel.ops) <= 16 / 4;
    let before = a.used();
    let plain = finish71(&sel71, four, single_seg, a)?;
    let cost = a.used().saturating_sub(before);
    let mut best = plain;
    /* the build of `moved` when it is better than `best`; else its scratch
     * goes back */
    let attempt = |moved: &Selected71<'_>, best: &mut (Code71<'a>, u32)| {
        let mark = a.used();
        match finish71(moved, four, single_seg, a) {
            Ok(alt) if alt.1 >= best.1 && alt.0.words.len() < best.0.words.len() => *best = alt,
            _ => {
                // SAFETY: what finish71 allocated above `mark` is referred
                // to only by its result, dropped here; `best`, `sel71` and
                // the orders were allocated below `mark`. `a` is the
                // compile's bottom-up scratch, which 4.2's build resets the
                // same way (order.rs `reorder`, through chain.rs `build`).
                unsafe { a.reset_to(mark) };
            }
        }
    };
    if !orders || a.left() < 2 * cost {
        return Ok(best);
    }
    let Ok(first) = interleave71(&sel71, a) else { return Ok(best) };
    attempt(&first, &mut best);
    if !sel71.fragment || a.left() < 2 * cost {
        return Ok(best);
    }
    if let Ok(second) = interleave71_sig(&sel71, a) {
        if second.ops != first.ops {
            attempt(&second, &mut best);
        }
    }
    Ok(best)
}

/// [`build71`] of one order of the operations: allocation at 4 threads
/// (when `four`) or 2, packing or scheduling, encoding and the checks.
fn finish71<'a>(
    sel71: &Selected71<'_>,
    four: bool,
    single_seg: bool,
    a: &Arena<'a>,
) -> Result<(Code71<'a>, u32), V3d71Error> {
    let (al, threads) = match if four { allocate71(sel71, 4, a) } else { Err(Alloc71Error::DoesNotFit) } {
        Ok(al) => (al, 4),
        Err(Alloc71Error::DoesNotFit) => (allocate71(sel71, 2, a)?, 2),
        Err(e) => return Err(e.into()),
    };
    /* a program with no control-flow Op71 is one straight-line run, which
     * pack71 packs as schedule71 would (sched71_cf's tests compare the two
     * word for word); anything else goes to schedule71 */
    let s = if sel71.ops.iter().any(Op71::is_control_flow) {
        schedule71(sel71, &al, a)?
    } else {
        let p = pack71(sel71.ops, &al, a)?;
        Scheduled71 { instrs: p.instrs, uniforms: p.uniforms, back_edges: &[], fwd_branches: &[] }
    };
    let words = a.alloc_filled(s.instrs.len(), 0u64)?;
    for (n, (w, i)) in words.iter_mut().zip(s.instrs).enumerate() {
        *w = encode71(i).map_err(|e| V3d71Error::Encode(n, e))?;
    }
    verify71(s.instrs, single_seg).map_err(V3d71Error::Verify)?;
    registers_fit_threads71(s.instrs, threads).map_err(V3d71Error::Verify)?;
    /* chain.rs build ends with verify.rs branches_are_structured; this is
     * its 7.1 form */
    branches_are_structured71(s.instrs, s.back_edges, s.fwd_branches).map_err(V3d71Error::Verify)?;
    let code = Code71 {
        words,
        instrs: s.instrs,
        uniforms: s.uniforms,
        back_edges: s.back_edges,
        fwd_branches: s.fwd_branches,
    };
    Ok((code, threads))
}

/// Compile one selected shader at 7.1 ([`build71`]), with the 4.x rule for
/// the thread section it starts in (chain.rs `vs_prog`).
pub fn compile_stage71<'a>(sel: &Selected<'_>, a: &Arena<'a>) -> Result<Stage71<'a>, V3d71Error> {
    let single_seg = !sel.fragment && starts_in_last_section(sel);
    let (code, threads) = build71(sel, single_seg, a)?;
    Ok(Stage71 { code, threads, single_seg })
}

/* chain-hook replaces this: chain.rs:214-225 `unrolled`. */
fn unrolled<'a>(m: &'a ir::Module<'a>, a: &'a Arena<'a>) -> Result<(&'a ir::Module<'a>, &'a [Trip]), V3dError> {
    use crate::shaderc::online::powervr::unroll::{unroll, UnrollError};
    match unroll(m, a) {
        Ok(Some(u)) => Ok((u.module, u.trips)),
        Ok(None) => Ok((m, &[])),
        Err(UnrollError::OutOfScratch) => Err(V3dError::OutOfScratch),
        Err(e) => Err(V3dError::NotYet(e.message())),
    }
}

/* chain-hook replaces this: chain.rs:240 `PRESSURES`. */
const PRESSURES: [Pressure; 4] = [Pressure::None, Pressure::LazyAttributes, Pressure::Remat, Pressure::Serial];

/* chain-hook replaces this: chain.rs:242-247 `starts_in_last_section`. The
 * rule has no version branch (vir.c:1041 `single_seg = !c->last_thrsw`). */
fn starts_in_last_section(sel: &Selected<'_>) -> bool {
    !sel.ops.contains(&VOp::Thrsw)
}

/* chain-hook replaces this: chain.rs:249-264 `under_pressure`, with
 * build71 for build and alloc71's DoesNotFit (2 threads) for chain.rs's
 * DOES_NOT_FIT. Then, 7.1 only, every pressure again with the operations
 * [`sunk71`]: the last resort for a stage 4.2 fits only through its
 * accumulators (none at 7.1), so a stage that fits today is built as
 * before. */
fn under_pressure71<'a>(
    stages: &[Pressure],
    select: impl Fn(Pressure) -> Result<Selected<'a>, SelectError>,
    single_seg: bool,
    orders: bool,
    a: &Arena<'a>,
) -> Result<(Selected<'a>, (Code71<'a>, u32)), V3d71Error> {
    for sink in [false, true] {
        for &pr in stages {
            let sel = select(pr)?;
            match build71_order(&sel, single_seg && starts_in_last_section(&sel), sink, orders, a) {
                Err(V3d71Error::Alloc(Alloc71Error::DoesNotFit)) => continue,
                r => return Ok((sel, r?)),
            }
        }
    }
    Err(V3d71Error::Alloc(Alloc71Error::DoesNotFit))
}

/// An operation [`sunk71`] may move later: a plain ALU operation or a
/// uniform load into a virtual register, reading only virtual registers
/// and small immediates. Left in place: SFU operations (their latency),
/// the VPM, TMU, TLB and flag operations, `multop`/`umul24` (a pair
/// through rtop), lane-crossing and per-lane-state operations, and every
/// reader of the payload or of ldvary's C term.
fn sinkable71(o: &Op71) -> bool {
    use AddOp71 as A;
    use MulOp71 as M;
    let plain = |s: &Src71| matches!(s, Src71::V(_) | Src71::Unpacked(..) | Src71::SmallImm(_) | Src71::None);
    match *o {
        Op71::Ldunif { .. } => true,
        Op71::Add { op, dst: Dst71::V(_), a, b } => {
            plain(&a)
                && plain(&b)
                && matches!(
                    op,
                    A::Fadd
                        | A::Faddnf
                        | A::Vfpack
                        | A::Add
                        | A::Sub
                        | A::Fsub
                        | A::Min
                        | A::Max
                        | A::Umin
                        | A::Umax
                        | A::Shl
                        | A::Shr
                        | A::Asr
                        | A::Ror
                        | A::Fmin
                        | A::Fmax
                        | A::Vfmin
                        | A::Vfmax
                        | A::Vfadd
                        | A::Vfsub
                        | A::Vfmov
                        | A::Vfabs
                        | A::Vfneg
                        | A::Vfnab
                        | A::And
                        | A::Or
                        | A::Xor
                        | A::Vadd
                        | A::Vsub
                        | A::Not
                        | A::Neg
                        | A::Fround
                        | A::Ftoin
                        | A::Ftrunc
                        | A::Ftoiz
                        | A::Ffloor
                        | A::Ftouz
                        | A::Fceil
                        | A::Ftoc
                        | A::Itof
                        | A::Clz
                        | A::Utof
                        | A::Fmov
                        | A::Mov
                        | A::Vpack
                        | A::V8pack
                        | A::V10pack
                        | A::V11fpack
                )
        }
        Op71::Mul { op, dst: Dst71::V(_), a, b } => {
            plain(&a)
                && plain(&b)
                && matches!(
                    op,
                    M::Add
                        | M::Sub
                        | M::Vfmul
                        | M::Smul24
                        | M::Fmov
                        | M::Mov
                        | M::Fmul
                        | M::Ftounorm16
                        | M::Ftosnorm16
                        | M::Vftounorm8
                        | M::Vftosnorm8
                        | M::Funpackunormlo
                        | M::Funpackunormhi
                        | M::Funpacksnormlo
                        | M::Funpacksnormhi
                        | M::Vftounorm10lo
                        | M::Vftounorm10hi
                        | M::V8dot
                )
        }
        _ => false,
    }
}

/// `sel` with each [`sinkable71`] operation moved down to just before the
/// first operation that reads its result, the last first, so that a chain
/// of them follows its consumer down: a vertex shader that computes every
/// varying before its first `stvpmv` then computes each one next to its
/// store, and holds a few values at a time where it held them all. (Mesa
/// emits each output's VPM write where NIR stores it, `store_output` at
/// nir_to_vir.c:3734-3736 through `emit_store_output_vs`, :3016-3034, so its
/// register pressure is that of this order; for the GLES2 varyings limit
/// shapes of tests/shaderc71_limits.rs its 7.1 vertex shaders and ours are
/// the same length at the same 4 threads.) An operation stays above every [`Op71::CondMov`] or
/// [`Op71::Copy`] that rewrites one of its operands (the value it reads
/// would change), and a uniform load above every `ldvary` and read of its
/// C term (its implicit rf0 write would land on C). A program with control
/// flow, and one where nothing moves, is returned as it is.
fn sunk71<'a>(sel: &Selected71<'_>, a: &Arena<'a>) -> Result<Selected71<'a>, ArenaError> {
    let ops = a.alloc_filled(sel.ops.len(), Op71::Thrsw)?;
    ops.copy_from_slice(sel.ops);
    if !sel.ops.iter().any(Op71::is_control_flow) {
        for i in (0..ops.len()).rev() {
            let o = ops[i];
            let Some(v) = o.def() else { continue };
            if !sinkable71(&o) {
                continue;
            }
            let reads = o.uses();
            let stop = ops[i + 1..].iter().position(|n| {
                n.uses().iter().any(|u| u.vreg() == Some(v))
                    || matches!(*n, Op71::CondMov { dst, .. } | Op71::Copy { dst, .. }
                        if reads.iter().any(|r| r.vreg() == Some(dst)))
                    || (matches!(o, Op71::Ldunif { .. })
                        && (matches!(n, Op71::Ldvary { .. }) || n.uses().contains(&Src71::LdvaryC)))
            });
            /* a result nothing reads stays where it is */
            let Some(k) = stop else { continue };
            ops[i..=i + k].rotate_left(1);
        }
    }
    Ok(Selected71 { ops, fragment: sel.fragment, vregs: sel.vregs })
}

/* chain-hook replaces this: chain.rs:266-276 `vertex`. */
fn vertex71<'a>(
    p: &Prepared<'_>,
    records: &[u8],
    coord: bool,
    key: crate::shaderc::LodKey,
    a: &Arena<'a>,
) -> Result<(Selected<'a>, (Code71<'a>, u32)), V3d71Error> {
    under_pressure71(&PRESSURES, |pr| select_vertex_keyed(p, records, coord, key, pr, a), true, true, a)
}

/* chain-hook replaces this: chain.rs:324-347 `vpm_fields`
 * (v3d_vs_set_prog_data, vir.c:896-937, which has no version branch). */
fn vpm_fields(sel: &Selected<'_>, target: Target) -> Result<(u32, u32, u32), V3dError> {
    let driver_row = (sel.uses_vid || sel.uses_iid) && sel.vattr_sizes[0] == 0;
    let input_rows: u32 = sel.vattr_sizes.iter().map(|&x| x as u32).sum::<u32>()
        + u32::from(sel.uses_iid)
        + u32::from(sel.uses_vid)
        + u32::from(driver_row);
    let input = input_rows.div_ceil(8);
    let output = sel.vpm_output_rows.max(1).div_ceil(8).max(input);
    /* 16 channels of 32 bits, 8 rows to a sector */
    let sectors = target.vpm_size / (16 * 4 * 8);
    let batches = (sectors / 2) / output;
    if batches < 2 {
        return Err(V3dError::VpmTooSmall);
    }
    Ok((0, output, (batches - 1).clamp(2, 4)))
}

/* chain-hook replaces this: chain.rs:349-374 `vs_prog`. */
fn vs_prog71<'a>(
    sel: &Selected<'_>,
    (code, threads): (Code71<'a>, u32),
    target: Target,
    writes_psiz: bool,
) -> Result<VsProg71<'a>, V3d71Error> {
    let (vpm_input_size, vpm_output_size, vcm_cache_size) = vpm_fields(sel, target)?;
    Ok(VsProg71 {
        code,
        threads,
        single_seg: starts_in_last_section(sel),
        vpm_input_size,
        vpm_output_size,
        separate_segments: false,
        vcm_cache_size,
        uses_vid: sel.uses_vid,
        uses_iid: sel.uses_iid,
        uses_biid: false,
        writes_psiz,
        vattr_sizes: sel.vattr_sizes,
        tmu_dirty_rcl: false,
    })
}

/// The lines variant's fragment operations: each `gl_PointCoord` lane a
/// constant 0.0, so that select_fragment takes no implicit point varying
/// (Mesa nir_to_vir.c:4869-4878 takes none for a lines FS without line
/// smoothing). The same operations otherwise, at the same indices.
fn point_coord_as_zero<'a>(ops: &'a [SOp], a: &'a Arena<'a>) -> Result<&'a [SOp], ArenaError> {
    if !ops.iter().any(|o| matches!(o, SOp::PointCoord { .. })) {
        return Ok(ops);
    }
    let out = a.alloc_filled(ops.len(), SOp::Const(0))?;
    for (o, &op) in out.iter_mut().zip(ops) {
        *o = match op {
            SOp::PointCoord { .. } => SOp::Const(0.0f32.to_bits()),
            op => op,
        };
    }
    Ok(out)
}

/// Compile the GL program `vs` + `fs` for `prim` on `target` at 7.1, with
/// the keys chain.rs `compile_program` uses.
pub fn compile_program71<'a>(
    vs: &'a ir::Module<'a>,
    fs: &'a ir::Module<'a>,
    prim: Prim,
    target: Target,
    a: &'a Arena<'a>,
) -> Result<V3dProgram71<'a>, V3d71Error> {
    compile_program71_keyed(vs, fs, prim, target, LodKey::NONE, crate::shaderc::compile_lod_key(), a)
}

/// chain.rs `compile_program_keyed` at 7.1: the same front half, the same
/// order, [`build71`] for each of the fragment, vertex and coordinate
/// shaders.
pub fn compile_program71_keyed<'a>(
    vs: &'a ir::Module<'a>,
    fs: &'a ir::Module<'a>,
    prim: Prim,
    target: Target,
    vs_key: LodKey,
    fs_key: LodKey,
    a: &'a Arena<'a>,
) -> Result<V3dProgram71<'a>, V3d71Error> {
    /* The primitive (chain.rs:437-441 refuses Points and Lines at 4.2; 7.1
     * compiles both). Mesa keys the fragment shader by it (v3d_program.c:
     * 604-606, is_points and is_lines) and reads it in one place,
     * nir_to_vir.c:4869-4878: a points FS that reads gl_PointCoord takes
     * the two implicit point varyings first; a lines FS takes the implicit
     * line varying only for SYSTEM_VALUE_LINE_COORD, which only the line
     * smoothing lowering reads (vir.c:1282-1283, key line_smoothing,
     * v3d_program.c:607-608), and ES has no line smoothing. The vertex and
     * coordinate shaders are the same for every primitive here: Mesa's
     * per_vertex_point_size (v3d_program.c:783-785, v3d_nir_lower_io.c:427)
     * only leaves gl_PointSize out of a non-points VPM row, and our records
     * carry the row whenever the shader writes it (writes_psiz).
     *
     * Triangles keeps the link-time program 4.2 builds (select.rs
     * select_fragment: the implicit point varyings whenever the shader
     * reads gl_PointCoord; the GL table draws such a program as points
     * only, src/v3d/backend.rs), so Points is that program word for word.
     * Lines is Mesa's is_lines program: no implicit varying, and
     * gl_PointCoord, undefined off a point (GLSL ES 1.00 section 7.2),
     * reads 0.0 ([`point_coord_as_zero`]). */
    let lines = match prim {
        Prim::Triangles | Prim::Points => false,
        Prim::Lines => true,
    };
    /* chain-hook replaces this: chain.rs:411-418, loops first. */
    let (vs, vs_trips) = unrolled(vs, a)?;
    let (fs, fs_trips) = unrolled(fs, a)?;

    /* chain-hook replaces this: chain.rs:420-449, the fragment shader's
     * inputs and their flat and centroid bits. */
    let mut fsp = prepare_fragment_with(fs, fs_trips, fs_key.cube_noseam, a).map_err(V3dError::from)?;
    if lines {
        fsp.ops = point_coord_as_zero(fsp.ops, a)?;
    }
    let inputs = fragment_inputs(&fsp, a).map_err(V3dError::from)?;
    let mut flat = 0u64;
    let mut centroid = 0u64;
    if inputs.len() > chain::MAX_FS_INPUTS {
        return Err(V3dError::TooManyFragmentInputs.into());
    }
    for (i, &(loc, _)) in inputs.iter().enumerate() {
        let d = fs
            .inputs
            .iter()
            .find(|d| d.kind == IoKind::Varying && d.location == loc)
            .ok_or(V3dError::VaryingWithoutVertexOutput)?;
        if d.interp == ir::INTERP_FLAT {
            flat |= 1 << i;
        }
        if d.interp == ir::INTERP_CENTROID {
            centroid |= 1 << i;
        }
    }

    /* chain-hook replaces this: chain.rs:451-468, each input to the vertex
     * shader's varying of the same name. */
    let wanted = a.alloc_filled(inputs.len(), (0u8, 0u8))?;
    for (w, &(loc, comp)) in wanted.iter_mut().zip(inputs) {
        let name = fs
            .inputs
            .iter()
            .find(|d| d.kind == IoKind::Varying && d.location == loc)
            .map(|d| d.name)
            .ok_or(V3dError::VaryingWithoutVertexOutput)?;
        let vs_loc = vs
            .outputs
            .iter()
            .find(|d| d.kind == IoKind::Varying && d.name == name)
            .map(|d| d.location)
            .ok_or(V3dError::VaryingWithoutVertexOutput)?;
        *w = (vs_loc, comp);
    }

    let vsp = prepare_vertex_with(vs, wanted, vs_trips, vs_key.cube_noseam, a).map_err(V3dError::from)?;
    let csp = prepare_vertex_with(vs, &[], vs_trips, vs_key.cube_noseam, a).map_err(V3dError::from)?;

    /* chain-hook replaces this: chain.rs:473-502, the attribute records. */
    let mut records: AVec<'a, u8> = AVec::new();
    for op in vsp.ops {
        if let SOp::Attr { loc, .. } = *op {
            if !records.as_slice().contains(&loc) {
                records.push(a, loc)?;
            }
        }
    }
    records.as_mut_slice().sort_unstable();
    let records = records.into_slice();
    if records.len() > 16 {
        return Err(V3dError::NotYet("more than 16 vertex attributes").into());
    }
    let mut attrs: AVec<'a, (u8, Option<u8>)> = AVec::new();
    for d in vs.inputs.iter().filter(|d| d.kind == IoKind::Attribute) {
        for k in 0..attribute_locations(d.gl_type) {
            let loc = d.location + k;
            if attrs.as_slice().iter().any(|e| e.0 == loc) {
                continue;
            }
            let r = records.iter().position(|&l| l == loc).map(|r| r as u8);
            attrs.push(a, (loc, r))?;
        }
    }
    attrs.as_mut_slice().sort_unstable_by_key(|e| e.0);
    if records.iter().any(|l| !attrs.as_slice().iter().any(|e| e.0 == *l)) {
        return Err(V3dError::NotYet("an attribute location with no declaration").into());
    }

    /* chain-hook replaces this: chain.rs:504-513, the three stages, with
     * build71. The fragment shader's interleaved builds keep the scratch of
     * the one that wins (build71_order): in a program whose front end
     * already holds much of the scratch, that can leave too little for the
     * vertex or coordinate shader (dEQP-GLES2 uniform_api.random.79: 1.7 of
     * the 4 MiB after the front end, 1.2 MiB more kept by the fragment
     * shader's orders, OutOfScratch in the vertex shader). Such a program is
     * built again from the same mark with the fragment shader in select71's
     * order alone, so a program that fits today keeps every word. */
    let stages = |fs_orders: bool| -> Result<_, V3d71Error> {
        let fs = under_pressure71(
            &[Pressure::None, Pressure::Remat, Pressure::Serial],
            |pr| select_fragment_keyed(&fsp, inputs, flat, centroid, fs_key, pr, a),
            false,
            fs_orders,
            a,
        )?;
        let vs = vertex71(&vsp, records, false, vs_key, a)?;
        let cs = vertex71(&csp, records, true, vs_key, a)?;
        Ok((fs, vs, cs))
    };
    let mark = a.used();
    let built = match stages(true) {
        Err(V3d71Error::OutOfScratch) => {
            // SAFETY: the failed build's results were dropped with its
            // error; everything it allocated is above `mark`, and nothing
            // kept (the prepared stages, `inputs`, `records`, `attrs`) is.
            unsafe { a.reset_to(mark) };
            stages(false)
        }
        r => r,
    };
    let ((fsel, (fs_code, fs_threads)), (vsel, vs_code), (csel, cs_code)) = built?;

    /* chain-hook replaces this: chain.rs:515-553, the prog_data. */
    let slots = a.alloc_filled(inputs.len(), (0u8, 0u8))?;
    for (s, &(loc, comp)) in slots.iter_mut().zip(inputs) {
        *s = (VARYING_SLOT_VAR0 + loc, comp);
    }
    Ok(V3dProgram71 {
        fs: FsProg71 {
            code: fs_code,
            threads: fs_threads,
            single_seg: false,
            num_inputs: inputs.len() as u32,
            input_slots: slots,
            flat_shade_flags: flat as u32,
            noperspective_flags: 0,
            centroid_flags: centroid as u32,
            flat_shade_flags_hi: (flat >> 32) as u32,
            noperspective_flags_hi: 0,
            centroid_flags_hi: (centroid >> 32) as u32,
            writes_z: fsel.writes_z,
            disable_ez: fsel.disable_ez,
            uses_center_w: fsel.uses_center_w,
            lock_scoreboard_on_first_thrsw: false,
            uses_implicit_point_line_varyings: fsel.implicit_point_varyings,
            uses_pid: false,
            rt_mask: fsp.rt_mask,
            uses_discard: fsp.discard.is_some() || fsp.rt_mask == 0,
            force_per_sample_msaa: false,
            tmu_dirty_rcl: false,
        },
        vs: vs_prog71(&vsel, vs_code, target, vsp.psiz.is_some())?,
        cs: vs_prog71(&csel, cs_code, target, csp.psiz.is_some())?,
        attributes: attrs.into_slice(),
    })
}

/// Compile the GL program `vs` + `fs` for `gen`. [`Gen::V42`] is
/// [`chain::compile_program_keyed`] unchanged, its error in
/// [`V3d71Error::Chain`]; [`Gen::V71`] is [`compile_program71_keyed`].
#[allow(clippy::too_many_arguments)]
pub fn compile_program_for<'a>(
    gen: Gen,
    vs: &'a ir::Module<'a>,
    fs: &'a ir::Module<'a>,
    prim: Prim,
    target: Target,
    vs_key: LodKey,
    fs_key: LodKey,
    a: &'a Arena<'a>,
) -> Result<AnyV3dProgram<'a>, V3d71Error> {
    match gen {
        Gen::V42 => chain::compile_program_keyed(vs, fs, prim, target, vs_key, fs_key, a)
            .map(AnyV3dProgram::V42)
            .map_err(V3d71Error::Chain),
        Gen::V71 => compile_program71_keyed(vs, fs, prim, target, vs_key, fs_key, a).map(AnyV3dProgram::V71),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::v3d::codec71::decode71;
    use crate::shaderc::online::{lower, store};

    static PASSTHROUGH_VS: &str = include_str!("../../../../tools/shaders/gl_test_es100/passthrough.vert");
    static INTERP_FS: &str = include_str!("../../../../tools/shaders/gl_test_es100/interp_color.frag");
    static UTRI_VS: &str = include_str!("../../../../tools/shaders/es100/utri.vert");
    static UTRI_FS: &str = include_str!("../../../../tools/shaders/es100/utri.frag");

    /// The pairs of tools/v3d/tri-shaders' passthrough_interp_color and
    /// utri.
    const PAIRS: [(&str, &str, &str); 2] =
        [("passthrough+interp_color", PASSTHROUGH_VS, INTERP_FS), ("utri", UTRI_VS, UTRI_FS)];

    /// Compile `vs` + `fs` to IR and hand both modules and the arena to `f`.
    fn with_modules(vs: &str, fs: &str, f: impl for<'a> FnOnce(&'a ir::Module<'a>, &'a ir::Module<'a>, &'a Arena<'a>)) {
        store::with_scratch(|a| {
            let v = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(vs).unwrap()).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(fs).unwrap()).map_err(|e| e.msg).unwrap();
            let v = a.alloc_filled(1, v.module).unwrap();
            let fr = a.alloc_filled(1, fr.module).unwrap();
            f(&v[0], &fr[0], a)
        })
        .expect("scratch");
    }

    /// A 7.1 stage that compiled: its words decode to its instructions and
    /// pass verify71 at its thread count.
    fn check71(what: &str, c: &Code71<'_>, threads: u32, single_seg: bool) {
        assert_eq!(c.words.len(), c.instrs.len(), "{what}");
        for (n, (w, i)) in c.words.iter().zip(c.instrs).enumerate() {
            let back = decode71(*w).unwrap_or_else(|e| panic!("{what} word {n} 0x{w:016x}: {e:?}"));
            assert!(super::super::codec71::same_instr71(i, &back), "{what} word {n}");
        }
        assert_eq!(verify71(c.instrs, single_seg), Ok(()), "{what}");
        assert_eq!(registers_fit_threads71(c.instrs, threads), Ok(()), "{what}");
    }

    #[test]
    fn v42_is_the_4_2_chain_word_for_word() {
        for (what, vs, fs) in PAIRS {
            with_modules(vs, fs, |v, f, a| {
                let ours =
                    compile_program_for(Gen::V42, v, f, Prim::Triangles, TARGET71, LodKey::NONE, LodKey::NONE, a);
                let theirs =
                    chain::compile_program_keyed(v, f, Prim::Triangles, TARGET71, LodKey::NONE, LodKey::NONE, a)
                        .expect(what);
                let Ok(AnyV3dProgram::V42(ours)) = ours else { panic!("{what}: {ours:?}") };
                for (s, o, t) in [
                    ("fs", &ours.fs.code, &theirs.fs.code),
                    ("vs", &ours.vs.code, &theirs.vs.code),
                    ("cs", &ours.cs.code, &theirs.cs.code),
                ] {
                    assert!(!o.words.is_empty(), "{what} {s}");
                    assert_eq!(o.words, t.words, "{what} {s}");
                    assert_eq!(o.uniforms, t.uniforms, "{what} {s}");
                }
                assert_eq!(
                    (ours.fs.threads, ours.vs.threads, ours.cs.threads),
                    (theirs.fs.threads, theirs.vs.threads, theirs.cs.threads)
                );
                assert_eq!(ours.attributes, theirs.attributes, "{what}");
            });
        }
    }

    #[test]
    fn v71_compiles_straight_line_pairs() {
        for (what, vs, fs) in PAIRS {
            with_modules(vs, fs, |v, f, a| {
                match compile_program_for(Gen::V71, v, f, Prim::Triangles, TARGET71, LodKey::NONE, LodKey::NONE, a) {
                    Ok(AnyV3dProgram::V71(p)) => {
                        check71("fs", &p.fs.code, p.fs.threads, p.fs.single_seg);
                        check71("vs", &p.vs.code, p.vs.threads, p.vs.single_seg);
                        check71("cs", &p.cs.code, p.cs.threads, p.cs.single_seg);
                        assert!(!p.fs.single_seg, "{what}");
                        /* the shared front half's facts are the 4.2 chain's */
                        let q = chain::compile_program_keyed(
                            v,
                            f,
                            Prim::Triangles,
                            TARGET71,
                            LodKey::NONE,
                            LodKey::NONE,
                            a,
                        )
                        .expect(what);
                        assert_eq!(p.fs.input_slots, q.fs.input_slots, "{what}");
                        assert_eq!(p.attributes, q.attributes, "{what}");
                        assert_eq!(p.vs.vattr_sizes, q.vs.vattr_sizes, "{what}");
                    }
                    Ok(AnyV3dProgram::V42(_)) => panic!("{what}: a 4.2 program for Gen::V71"),
                    /* both pairs are straight-line after select71: pack71
                     * compiles them */
                    Err(e) => panic!("{what}: {e:?}"),
                }
            });
        }
    }

    /// es100/databreak.frag's loop, which leaves on data, is a back edge at
    /// 7.1: schedule71 places it and build71 returns it only after
    /// branches_are_structured71 (run here again on the returned code).
    #[test]
    fn v71_compiles_a_program_with_a_branch() {
        let fs = include_str!("../../../../tools/shaders/es100/databreak.frag");
        with_modules(PASSTHROUGH_VS, fs, |v, f, a| {
            match compile_program_for(Gen::V71, v, f, Prim::Triangles, TARGET71, LodKey::NONE, LodKey::NONE, a) {
                Ok(AnyV3dProgram::V71(p)) => {
                    check71("fs", &p.fs.code, p.fs.threads, p.fs.single_seg);
                    let c = &p.fs.code;
                    assert!(!c.back_edges.is_empty(), "the loop is not a back edge: {:?}", c.fwd_branches);
                    assert_eq!(branches_are_structured71(c.instrs, c.back_edges, c.fwd_branches), Ok(()));
                }
                r => panic!("{r:?}"),
            }
        });
    }

    #[test]
    fn shared_refusals_are_the_4_2_chains_at_both_generations() {
        let fs = |body: &str| std::format!("precision mediump float; varying vec4 v_color; void main() {{ {body} }}");
        /* prepare.rs counted_loop: an output written in a loop that can
         * leave on data (chain.rs what_this_cut_does_not_compile_is_refused_by_name);
         * a varying the vertex shader does not declare. (The points and
         * lines variants compile at 7.1: points_and_lines_compile_at_7_1.) */
        let loop_fs = fs("vec4 c = v_color; for (float f = 0.0; f < 2.0; f += 1.0) { if (c.x > 0.5) break; \
                          gl_FragColor = c; c = c * v_color; }");
        let no_varying_vs = "attribute vec4 a_pos; void main() { gl_Position = a_pos; }";
        let cases: [(&str, &str, Prim, V3dError); 2] = [
            (
                PASSTHROUGH_VS,
                &loop_fs,
                Prim::Triangles,
                V3dError::NotYet("an output written inside a loop that can leave on data"),
            ),
            (no_varying_vs, INTERP_FS, Prim::Triangles, V3dError::VaryingWithoutVertexOutput),
        ];
        for (vs, fs, prim, want) in cases {
            with_modules(vs, fs, |v, f, a| {
                let r42 = chain::compile_program_keyed(v, f, prim, TARGET71, LodKey::NONE, LodKey::NONE, a).map(|_| ());
                assert_eq!(r42, Err(want));
                for gen in Gen::ALL {
                    let r = compile_program_for(gen, v, f, prim, TARGET71, LodKey::NONE, LodKey::NONE, a).map(|_| ());
                    assert_eq!(r.err(), Some(V3d71Error::Chain(want)), "{gen:?}");
                }
            });
        }
    }

    /// es100/pointcoord.frag's shape with a declared varying beside it:
    /// gl_PointCoord in red and green, the varying in blue.
    const POINT_FS: &str = "precision mediump float; varying vec4 v_color; \
                            void main() { gl_FragColor = vec4(gl_PointCoord, v_color.x, 1.0); }";

    /// `vs` + `fs` at 7.1 for `prim`, every stage through [`check71`].
    fn compile71<'a>(v: &'a ir::Module<'a>, f: &'a ir::Module<'a>, prim: Prim, a: &'a Arena<'a>) -> V3dProgram71<'a> {
        let p = compile_program71_keyed(v, f, prim, TARGET71, LodKey::NONE, LodKey::NONE, a)
            .unwrap_or_else(|e| panic!("{prim:?}: {e:?}"));
        check71("fs", &p.fs.code, p.fs.threads, p.fs.single_seg);
        check71("vs", &p.vs.code, p.vs.threads, p.vs.single_seg);
        check71("cs", &p.cs.code, p.cs.threads, p.cs.single_seg);
        p
    }

    fn ldvarys(c: &Code71<'_>) -> usize {
        c.instrs.iter().filter(|i| matches!(i, Instr71::Alu(x) if x.sig.ldvary)).count()
    }

    /// Both variants compile at 7.1 (the 4.2 chain still refuses them), every
    /// word decoding to its instruction and verify71 passing. The vertex and
    /// coordinate shaders are the Triangles program's for every primitive;
    /// so is the fragment shader of a program that does not read
    /// gl_PointCoord, and of every points variant.
    #[test]
    fn points_and_lines_compile_at_7_1() {
        for (what, vs, fs) in [PAIRS[0], PAIRS[1], ("passthrough+point", PASSTHROUGH_VS, POINT_FS)] {
            with_modules(vs, fs, |v, f, a| {
                let tri = compile71(v, f, Prim::Triangles, a);
                let reads_pntc = tri.fs.uses_implicit_point_line_varyings;
                assert_eq!(reads_pntc, fs == POINT_FS, "{what}");
                for prim in [Prim::Points, Prim::Lines] {
                    let p = compile71(v, f, prim, a);
                    for (s, o, t) in [("vs", &p.vs, &tri.vs), ("cs", &p.cs, &tri.cs)] {
                        assert_eq!(o.code.words, t.code.words, "{what} {prim:?} {s}");
                        assert_eq!(o.code.uniforms, t.code.uniforms, "{what} {prim:?} {s}");
                    }
                    assert_eq!(p.fs.input_slots, tri.fs.input_slots, "{what} {prim:?}");
                    assert_eq!(p.attributes, tri.attributes, "{what} {prim:?}");
                    let same_fs = prim == Prim::Points || !reads_pntc;
                    assert_eq!(p.fs.code.words == tri.fs.code.words, same_fs, "{what} {prim:?}");
                    assert_eq!(p.fs.uses_implicit_point_line_varyings, prim == Prim::Points && reads_pntc, "{what}");
                    /* 4.2 is unchanged: it refuses both variants by name */
                    assert!(
                        matches!(
                            chain::compile_program_keyed(v, f, prim, TARGET71, LodKey::NONE, LodKey::NONE, a),
                            Err(V3dError::NotYet(_))
                        ),
                        "{what} {prim:?} at 4.2"
                    );
                }
            });
        }
    }

    /// The points variant of a program reading gl_PointCoord: two LDVARY
    /// before the declared ones (Mesa nir_to_vir.c:4869-4872), y through
    /// the PNTC transform; the lines variant: none, and gl_PointCoord reads
    /// 0.0. Run in sim71 with the implicit varyings (x, y) = (0.25, 0.125),
    /// v_color.x = 0.5 and the window transform (scale -1.0, offset 1.0).
    #[test]
    fn the_point_coord_of_each_variant_in_sim71() {
        use super::super::select::{
            QUNIFORM_CONSTANT, QUNIFORM_FB_PNTC_Y_OFFSET, QUNIFORM_FB_PNTC_Y_SCALE, QUNIFORM_UNIFORM,
        };
        use super::super::sim::{half_to_f32, Env, FsInputs, Inputs, Varying, LANES};
        use super::super::sim71::run71;
        let stream = |u: &[Uniform]| -> std::vec::Vec<u32> {
            u.iter()
                .map(|u| match u.contents {
                    QUNIFORM_CONSTANT => u.data,
                    QUNIFORM_FB_PNTC_Y_SCALE => (-1.0f32).to_bits(),
                    QUNIFORM_FB_PNTC_Y_OFFSET => 1.0f32.to_bits(),
                    QUNIFORM_UNIFORM => panic!("no user uniform here"),
                    c => panic!("uniform contents {c:#x} not modelled here"),
                })
                .collect()
        };
        /* smooth: vary * W + C, with W = 1.0 and vary = 0 */
        let vary = |x: f32| Varying { vary: [0; LANES], c: [x.to_bits(); LANES] };
        let run = |c: &Code71<'_>, threads: u32, varyings: std::vec::Vec<Varying>| {
            let one = [1.0f32.to_bits(); LANES];
            let inputs = Inputs::Fragment(std::boxed::Box::new(FsInputs {
                w: one,
                w_centroid: one,
                z: one,
                x: one,
                y: one,
                back_facing: [false; LANES],
                varyings,
            }));
            let o = run71(c.words, &stream(c.uniforms), &inputs, &Env { threads, ..Env::default() })
                .unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(o.tlb.len(), 2, "two colour TLB writes");
            let h = |w: Option<u32>, hi: bool| {
                let w = w.expect("a defined colour");
                half_to_f32(if hi { (w >> 16) as u16 } else { w as u16 })
            };
            let (p, q) = (o.tlb[0].value[0], o.tlb[1].value[0]);
            for l in 0..LANES {
                assert_eq!((o.tlb[0].value[l], o.tlb[1].value[l]), (p, q), "lane {l}");
            }
            [h(p, false), h(p, true), h(q, false), h(q, true)]
        };
        with_modules(PASSTHROUGH_VS, POINT_FS, |v, f, a| {
            let p = compile71(v, f, Prim::Points, a);
            assert!(p.fs.uses_implicit_point_line_varyings);
            assert_eq!(ldvarys(&p.fs.code), p.fs.num_inputs as usize + 2);
            for c in [QUNIFORM_FB_PNTC_Y_SCALE, QUNIFORM_FB_PNTC_Y_OFFSET] {
                assert!(p.fs.code.uniforms.contains(&Uniform { contents: c, data: 0 }), "{c:#x}");
            }
            let got = run(&p.fs.code, p.fs.threads, std::vec![vary(0.25), vary(0.125), vary(0.5)]);
            assert_eq!(got, [0.25, 1.0 - 0.125, 0.5, 1.0]);

            let l = compile71(v, f, Prim::Lines, a);
            assert!(!l.fs.uses_implicit_point_line_varyings);
            assert_eq!(ldvarys(&l.fs.code), l.fs.num_inputs as usize);
            assert!(!l.fs.code.uniforms.iter().any(|u| u.contents == QUNIFORM_FB_PNTC_Y_SCALE));
            let got = run(&l.fs.code, l.fs.threads, std::vec![vary(0.5)]);
            assert_eq!(got, [0.0, 0.0, 0.5, 1.0]);
        });
    }

    #[test]
    fn a_stage_is_refused_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        /* A fragment shader: an empty vertex shader has no viewport x/y
         * pair, which select71 refuses as a defect (ViewportXyMissing),
         * not as an unfinished stage. An empty fragment selection has no
         * THRSW of its own (select.rs `last_thrsw` gives every selected
         * fragment shader the last-thread-switch pair), so pack71's end
         * THRSW is taken as the last thread switch, and verify71 refuses
         * the program by its rule (qpu_validate.c:478-484), never
         * returning it. */
        let sel = super::super::vop71::empty_selected(true);
        assert_eq!(
            compile_stage71(&sel, &a).err(),
            Some(V3d71Error::Verify(VerifyError { ip: 2, rule: "No program-end THRSW found" }))
        );
    }

    /* sunk71's operations: vregs as numbers */
    fn sv(n: u32) -> Src71 {
        Src71::V(super::super::vop71::V(n))
    }
    fn un(d: u32, data: u32) -> Op71 {
        Op71::Ldunif { dst: super::super::vop71::V(d), u: Uniform { contents: 0, data } }
    }
    fn fadd(d: u32, x: u32, y: u32) -> Op71 {
        Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(super::super::vop71::V(d)), a: sv(x), b: sv(y) }
    }
    fn st(addr: u32, x: u32) -> Op71 {
        Op71::Add { op: AddOp71::Stvpmv, dst: Dst71::None, a: sv(addr), b: sv(x) }
    }
    fn sunk(ops: &[Op71], fragment: bool) -> std::vec::Vec<Op71> {
        let mut buf = std::vec![0u8; 1 << 16];
        let a = Arena::new(&mut buf);
        let sel = Selected71 { ops, fragment, vregs: 64 };
        sunk71(&sel, &a).unwrap().ops.to_vec()
    }

    #[test]
    fn sunk71_computes_each_output_next_to_its_store() {
        /* the shape of the GLES2 varyings limit: every output computed,
         * then every output stored */
        let ops = [un(0, 0), fadd(1, 0, 0), un(2, 1), fadd(3, 2, 2), un(4, 9), st(4, 1), un(5, 9), st(5, 3)];
        let want = [un(4, 9), un(0, 0), fadd(1, 0, 0), st(4, 1), un(5, 9), un(2, 1), fadd(3, 2, 2), st(5, 3)];
        assert_eq!(sunk(&ops, false), want);
        /* each value already next to its reader: nothing moves */
        let tight = [un(0, 0), fadd(1, 0, 0), st(0, 1)];
        assert_eq!(sunk(&tight, false), tight);
        /* a result nothing reads stays where it is */
        let dead = [un(0, 0), fadd(1, 0, 0), un(2, 1), st(2, 0)];
        assert_eq!(sunk(&dead, false), dead);
    }

    #[test]
    fn sunk71_keeps_side_effects_rewrites_and_ldvary_c_in_order() {
        /* a read of an operand stays above a CondMov that rewrites it */
        let cm = Op71::CondMov { dst: super::super::vop71::V(0), src: sv(9), cond: super::super::isa::Cond::IfA };
        let ops = [un(0, 0), fadd(1, 0, 0), cm, un(2, 1), st(2, 1)];
        assert_eq!(sunk(&ops, false), [un(0, 0), fadd(1, 0, 0), cm, un(2, 1), st(2, 1)]);
        /* a uniform load stays above an ldvary and a read of its C */
        let lv = Op71::Ldvary { dst: super::super::vop71::V(3) };
        let c = Op71::Add { op: AddOp71::Fadd, dst: Dst71::V(super::super::vop71::V(4)), a: sv(3), b: Src71::LdvaryC };
        let ops = [un(0, 0), lv, c, fadd(5, 0, 4)];
        let got = sunk(&ops, true);
        assert_eq!(got[0], un(0, 0), "{got:?}");
        /* an SFU, a VPM read and a stvpmv never move */
        let rcp = Op71::Sfu { op: super::super::vop71::Sfu71::Recip, dst: super::super::vop71::V(1), a: sv(0) };
        let ld =
            Op71::Add { op: AddOp71::LdvpmvIn, dst: Dst71::V(super::super::vop71::V(2)), a: sv(0), b: Src71::None };
        let ops = [un(0, 0), rcp, ld, un(3, 1), st(3, 3), st(3, 1), st(3, 2)];
        let got = sunk(&ops, false);
        assert_eq!(&got[1..3], &[rcp, ld]);
        assert_eq!(&got[4..], &[st(3, 3), st(3, 1), st(3, 2)]);
    }

    #[test]
    fn sunk71_leaves_a_program_with_control_flow_as_it_is() {
        let ops = [un(0, 0), fadd(1, 0, 0), Op71::LoopHead, un(2, 1), st(2, 1)];
        assert_eq!(sunk(&ops, false), ops);
    }

    /// A vertex shader with `n` vec4 attributes, all read.
    fn attrs_vs(n: usize) -> std::string::String {
        let mut s = std::string::String::new();
        for i in 0..n {
            s += &std::format!("attribute vec4 a{i}; ");
        }
        s += "varying vec4 v_color; void main() { vec4 s = vec4(0.0); ";
        for i in 0..n {
            s += &std::format!("s += a{i}; ");
        }
        s += "v_color = s; gl_Position = s; }";
        s
    }

    /// A fragment shader reading `n` vec4 varyings, each combined with a
    /// uniform twice, all the results summed last: a great many values live
    /// at once.
    fn wide_fs(n: usize) -> std::string::String {
        let mut s = std::string::String::from("precision mediump float; uniform vec4 u; ");
        for i in 0..n {
            s += &std::format!("varying vec4 v{i}; ");
        }
        s += "void main() { ";
        for i in 0..n {
            s += &std::format!("vec4 p{i} = v{i} * u; vec4 q{i} = v{i} + u; ");
        }
        s += "vec4 s = vec4(0.0); ";
        for i in 0..n {
            s += &std::format!("s += p{i} * q{i}; ");
        }
        s += "gl_FragColor = s; }";
        s
    }

    fn wide_vs(n: usize) -> std::string::String {
        let mut s = std::string::String::from("attribute vec4 a_pos; ");
        for i in 0..n {
            s += &std::format!("varying vec4 v{i}; ");
        }
        s += "void main() { gl_Position = a_pos; ";
        for i in 0..n {
            s += &std::format!("v{i} = a_pos; ");
        }
        s += "}";
        s
    }

    const OPTS: (Prim, LodKey, LodKey) = (Prim::Triangles, LodKey::NONE, LodKey::NONE);

    fn tri_result(vs: &str, fs: &str) -> Result<(), V3d71Error> {
        let mut r = Ok(());
        with_modules(vs, fs, |v, f, a| {
            r = compile_program71_keyed(v, f, OPTS.0, TARGET71, OPTS.1, OPTS.2, a).map(|_| ());
        });
        r
    }

    /// `records.len() > 16` (the check after the attribute records are
    /// collected): 16 attribute locations compile, 17 are refused by name,
    /// as the 4.2 chain does.
    #[test]
    fn more_than_16_attribute_records_is_refused() {
        assert_eq!(tri_result(&attrs_vs(16), INTERP_FS), Ok(()));
        let want = V3dError::NotYet("more than 16 vertex attributes");
        assert_eq!(tri_result(&attrs_vs(17), INTERP_FS), Err(V3d71Error::Chain(want)));
        with_modules(&attrs_vs(17), INTERP_FS, |v, f, a| {
            let r = chain::compile_program_keyed(v, f, OPTS.0, TARGET71, OPTS.1, OPTS.2, a).map(|_| ());
            assert_eq!(r, Err(want));
        });
    }

    /// A fragment shader reading 65 scalar varying inputs is past
    /// MAX_FS_INPUTS (64): refused before selection, in the chain's error.
    #[test]
    fn more_than_64_fragment_inputs_is_refused() {
        let mut fs = std::string::String::from("precision mediump float; varying float extra; ");
        let mut vs = wide_vs(16);
        vs = vs.replace("void main() {", "varying float extra; void main() { extra = 1.0;");
        for i in 0..16 {
            fs += &std::format!("varying vec4 v{i}; ");
        }
        fs += "void main() { gl_FragColor = extra";
        for i in 0..16 {
            fs += &std::format!(" + v{i}");
        }
        fs += "; }";
        assert_eq!(tri_result(&vs, &fs), Err(V3d71Error::Chain(V3dError::TooManyFragmentInputs)));
    }

    /// A vertex shader the program's fragment shader reads no varying of is
    /// still a program; but a varying the vertex shader lacks is
    /// VaryingWithoutVertexOutput, in the chain's own error.
    #[test]
    fn a_varying_without_a_vertex_output_is_the_chains_error() {
        let vs = "attribute vec4 a_pos; void main() { gl_Position = a_pos; }";
        assert_eq!(tri_result(vs, INTERP_FS), Err(V3d71Error::Chain(V3dError::VaryingWithoutVertexOutput)));
        let fs = "precision mediump float; void main() { gl_FragColor = vec4(1.0); }";
        assert_eq!(tri_result(vs, fs), Ok(()));
    }

    /// A vertex stage with no viewport x/y pair is select71's defect
    /// refusal, [`V3d71Error::Select`], not an unfinished stage.
    #[test]
    fn a_select71_refusal_is_the_select_variant() {
        let sel = super::super::vop71::empty_selected(false);
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        assert_eq!(compile_stage71(&sel, &a).err(), Some(V3d71Error::Select(Select71Error::ViewportXyMissing)));
        assert_eq!(build71(&sel, true, &a).err(), Some(V3d71Error::Select(Select71Error::ViewportXyMissing)));
    }

    /// Every stage error converts as the `From` impls say: the arena's and
    /// the stages' own out-of-scratch to [`V3d71Error::OutOfScratch`], the
    /// rest to the stage's variant.
    #[test]
    fn stage_errors_convert_to_their_variant() {
        assert_eq!(V3d71Error::from(ArenaError::OutOfScratch), V3d71Error::OutOfScratch);
        assert_eq!(V3d71Error::from(Select71Error::OutOfScratch), V3d71Error::OutOfScratch);
        assert_eq!(V3d71Error::from(Alloc71Error::OutOfScratch), V3d71Error::OutOfScratch);
        assert_eq!(V3d71Error::from(Sched71Error::OutOfScratch), V3d71Error::OutOfScratch);
        assert_eq!(V3d71Error::from(Select71Error::NotOn71("x")), V3d71Error::Select(Select71Error::NotOn71("x")));
        assert_eq!(V3d71Error::from(Alloc71Error::Threads), V3d71Error::Alloc(Alloc71Error::Threads));
        assert_eq!(V3d71Error::from(Sched71Error::ControlFlow(3)), V3d71Error::Sched(Sched71Error::ControlFlow(3)));
        assert_eq!(V3d71Error::from(V3dError::VpmTooSmall), V3d71Error::Chain(V3dError::VpmTooSmall));
        assert_eq!(
            V3d71Error::from(SelectError::NotSelected),
            V3d71Error::Chain(V3dError::from(SelectError::NotSelected))
        );
    }

    /// A scratch arena with nothing left: every compile that reaches
    /// allocation ends in an out-of-scratch error (the stage's own, or the
    /// chain's), never a panic, and never a program.
    #[test]
    fn an_arena_with_no_room_left_is_out_of_scratch() {
        for left in [0usize, 64, 1024] {
            let mut r = Ok(());
            with_modules(PASSTHROUGH_VS, INTERP_FS, |v, f, a| {
                let keep = a.left().saturating_sub(left);
                let _ = a.alloc_filled(keep, 0u8);
                r = compile_program71_keyed(v, f, OPTS.0, TARGET71, OPTS.1, OPTS.2, a).map(|_| ());
            });
            let oom = r.err().is_some_and(|e| std::format!("{e:?}").contains("OutOfScratch"));
            assert!(oom, "{left} bytes left: {r:?}");
        }
    }

    /// Too many values live at once at any pressure and either order:
    /// DoesNotFit at 2 threads, the allocator's own refusal (no spilling).
    #[test]
    fn values_that_fit_no_register_file_are_refused_as_alloc_does_not_fit() {
        assert_eq!(tri_result(&wide_vs(16), &wide_fs(16)), Err(V3d71Error::Alloc(Alloc71Error::DoesNotFit)));
        /* fifteen of them fit, at some pressure or order */
        assert_eq!(tri_result(&wide_vs(15), &wide_fs(15)), Ok(()));
    }

    /// The fragment shader's [`Selected`] at pressure `pr`, as
    /// compile_program71_keyed selects it (no flat or centroid inputs).
    fn fs_selected<'a>(f: &'a ir::Module<'a>, pr: Pressure, a: &'a Arena<'a>) -> Result<Selected<'a>, SelectError> {
        let fsp = prepare_fragment_with(f, &[], 0, a).unwrap();
        let inputs = fragment_inputs(&fsp, a).unwrap();
        select_fragment_keyed(&fsp, inputs, 0, 0, LodKey::NONE, pr, a)
    }

    /// under_pressure71 asks for each pressure in the order given, with
    /// select71's order of operations; when none fits it asks for them all
    /// again with the operations sunk, then gives DoesNotFit. A stage that
    /// fits stops the walk at once.
    #[test]
    fn under_pressure71_walks_the_pressures_twice_then_gives_up() {
        use std::cell::RefCell;
        let stages = [Pressure::None, Pressure::Remat, Pressure::Serial];
        with_modules(&wide_vs(16), &wide_fs(16), |_, f, a| {
            let asked = RefCell::new(std::vec::Vec::new());
            let r = under_pressure71(
                &stages,
                |pr| {
                    asked.borrow_mut().push(pr);
                    fs_selected(f, pr, a)
                },
                false,
                false,
                a,
            );
            assert_eq!(r.err(), Some(V3d71Error::Alloc(Alloc71Error::DoesNotFit)));
            let want: std::vec::Vec<Pressure> = stages.iter().chain(stages.iter()).copied().collect();
            assert_eq!(*asked.borrow(), want);
        });
        with_modules(PASSTHROUGH_VS, INTERP_FS, |_, f, a| {
            let asked = RefCell::new(0);
            let (sel, (code, threads)) = under_pressure71(
                &stages,
                |pr| {
                    *asked.borrow_mut() += 1;
                    fs_selected(f, pr, a)
                },
                false,
                false,
                a,
            )
            .unwrap();
            assert_eq!(*asked.borrow(), 1, "the first pressure fits");
            assert!(sel.fragment);
            check71("fs", &code, threads, false);
        });
    }

    /// A selection error is the walk's end: its own variant, no further
    /// pressure asked; an empty list of pressures is DoesNotFit.
    #[test]
    fn under_pressure71_stops_at_a_selection_error() {
        with_modules(PASSTHROUGH_VS, INTERP_FS, |_, f, a| {
            let n = std::cell::Cell::new(0);
            let r = under_pressure71(
                &[Pressure::None, Pressure::Remat],
                |_| {
                    n.set(n.get() + 1);
                    Err(SelectError::NotSelected)
                },
                false,
                false,
                a,
            );
            let _ = f;
            assert_eq!(r.err(), Some(V3d71Error::Chain(V3dError::from(SelectError::NotSelected))));
            assert_eq!(n.get(), 1);
            let r = under_pressure71(&[], |_| Err(SelectError::NotSelected), false, false, a);
            assert_eq!(r.err(), Some(V3d71Error::Alloc(Alloc71Error::DoesNotFit)));
        });
    }

    /// A fragment shader reading nothing but gl_PointCoord: the points
    /// variant takes the two implicit varyings (and no declared one), the
    /// lines variant none, and the triangles build is the points one.
    #[test]
    fn a_point_coord_only_shader_in_each_variant() {
        let fs = "precision mediump float; void main() { gl_FragColor = vec4(gl_PointCoord, 0.0, 1.0); }";
        with_modules(PASSTHROUGH_VS, fs, |v, f, a| {
            let tri = compile71(v, f, Prim::Triangles, a);
            let pts = compile71(v, f, Prim::Points, a);
            let lines = compile71(v, f, Prim::Lines, a);
            assert_eq!(pts.fs.code.words, tri.fs.code.words);
            assert!(pts.fs.uses_implicit_point_line_varyings && tri.fs.uses_implicit_point_line_varyings);
            assert_eq!(pts.fs.num_inputs, 0);
            assert_eq!(ldvarys(&pts.fs.code), 2);
            assert!(!lines.fs.uses_implicit_point_line_varyings);
            assert_eq!(lines.fs.num_inputs, 0);
            assert_eq!(ldvarys(&lines.fs.code), 0);
            assert_ne!(lines.fs.code.words, pts.fs.code.words);
        });
    }

    /// Every word of a wide set of programs decodes with codec71 to its
    /// instruction and passes verify71 at the stage's thread count, for each
    /// primitive: 16 attribute records, 15 live varyings (built sunk or at a
    /// higher pressure), a loop with a branch.
    #[test]
    fn every_word_of_other_programs_decodes_and_verifies() {
        let looped = include_str!("../../../../tools/shaders/es100/databreak.frag");
        let progs: [(&str, std::string::String, std::string::String); 3] = [
            ("attrs16", attrs_vs(16), INTERP_FS.into()),
            ("wide15", wide_vs(15), wide_fs(15)),
            ("loop", PASSTHROUGH_VS.into(), looped.into()),
        ];
        for (what, vs, fs) in &progs {
            with_modules(vs, fs, |v, f, a| {
                for prim in [Prim::Triangles, Prim::Points, Prim::Lines] {
                    let p = compile71(v, f, prim, a);
                    for c in [&p.fs.code, &p.vs.code, &p.cs.code] {
                        assert!(!c.words.is_empty(), "{what} {prim:?}");
                    }
                }
            });
        }
    }
}
