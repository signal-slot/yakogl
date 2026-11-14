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
//! section (a) and the contract in [`super::vop71`]'s header say.
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
//! While a 7.1 stage is a stub, a 7.1 compile returns that stage's named
//! refusal, never a panic and never a wrong word.

use super::alloc71::{allocate71, Alloc71Error};
use super::chain::{self, attribute_locations, Prim, Target, V3dError, V3dProgram, VARYING_SLOT_VAR0};
use super::codec71::encode71;
use super::encode::EncodeError;
use super::prepare::{fragment_inputs, prepare_fragment_with, prepare_vertex_with, Prepared, SOp};
use super::qpu_ffi::Gen;
use super::sched71::Sched71Error;
use super::sched71_cf::schedule71;
use super::select::{select_fragment, select_vertex, tmu_input_writes, Pressure, SelectError, Selected, VOp};
use super::select71::{select71, Select71Error};
use super::verify71::{registers_fit_threads71, verify71, VerifyError};
use super::vop71::{Instr71, Uniform};
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
    /// registers at any selection pressure (no spilling yet).
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
pub fn build71<'a>(sel: &Selected<'_>, single_seg: bool, a: &Arena<'a>) -> Result<(Code71<'a>, u32), V3d71Error> {
    let sel71 = select71(sel, a)?;
    let four = tmu_input_writes(sel.ops) <= 16 / 4;
    let (al, threads) = match if four { allocate71(&sel71, 4, a) } else { Err(Alloc71Error::DoesNotFit) } {
        Ok(al) => (al, 4),
        Err(Alloc71Error::DoesNotFit) => (allocate71(&sel71, 2, a)?, 2),
        Err(e) => return Err(e.into()),
    };
    let s = schedule71(&sel71, &al, a)?;
    let words = a.alloc_filled(s.instrs.len(), 0u64)?;
    for (n, (w, i)) in words.iter_mut().zip(s.instrs).enumerate() {
        *w = encode71(i).map_err(|e| V3d71Error::Encode(n, e))?;
    }
    verify71(s.instrs, single_seg).map_err(V3d71Error::Verify)?;
    registers_fit_threads71(s.instrs, threads).map_err(V3d71Error::Verify)?;
    /* chain.rs build ends with verify.rs branches_are_structured; its 7.1
     * form (sched71_cf branches_are_structured71) comes with sched71-cf.
     * Until then a program with a branch is refused, never returned
     * unchecked. */
    if !s.back_edges.is_empty() || !s.fwd_branches.is_empty() {
        return Err(V3d71Error::NotYet("branches_are_structured71 (sched71-cf)"));
    }
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
 * DOES_NOT_FIT. */
fn under_pressure71<'a>(
    stages: &[Pressure],
    select: impl Fn(Pressure) -> Result<Selected<'a>, SelectError>,
    single_seg: bool,
    a: &Arena<'a>,
) -> Result<(Selected<'a>, (Code71<'a>, u32)), V3d71Error> {
    for &pr in stages {
        let sel = select(pr)?;
        match build71(&sel, single_seg && starts_in_last_section(&sel), a) {
            Err(V3d71Error::Alloc(Alloc71Error::DoesNotFit)) => continue,
            r => return Ok((sel, r?)),
        }
    }
    Err(V3d71Error::Alloc(Alloc71Error::DoesNotFit))
}

/* chain-hook replaces this: chain.rs:266-276 `vertex`. */
fn vertex71<'a>(
    p: &Prepared<'_>,
    records: &[u8],
    coord: bool,
    ret32: u32,
    a: &Arena<'a>,
) -> Result<(Selected<'a>, (Code71<'a>, u32)), V3d71Error> {
    under_pressure71(&PRESSURES, |pr| select_vertex(p, records, coord, ret32, pr, a), true, a)
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
    /* chain-hook replaces this: chain.rs:406-410, the primitive. */
    match prim {
        Prim::Triangles => {}
        Prim::Lines => return Err(V3dError::NotYet("the lines variant of the fragment shader").into()),
        Prim::Points => return Err(V3dError::NotYet("the points variant of the fragment shader").into()),
    }
    /* chain-hook replaces this: chain.rs:411-418, loops first. */
    let (vs, vs_trips) = unrolled(vs, a)?;
    let (fs, fs_trips) = unrolled(fs, a)?;

    /* chain-hook replaces this: chain.rs:420-449, the fragment shader's
     * inputs and their flat and centroid bits. */
    let fsp = prepare_fragment_with(fs, fs_trips, fs_key.cube_noseam, a).map_err(V3dError::from)?;
    let inputs = fragment_inputs(&fsp, a).map_err(V3dError::from)?;
    let mut flat = 0u64;
    let mut centroid = 0u64;
    if inputs.len() > 64 {
        return Err(V3dError::NotYet("more than 64 fragment shader inputs").into());
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
     * build71. */
    let (fsel, (fs_code, fs_threads)) = under_pressure71(
        &[Pressure::None, Pressure::Remat, Pressure::Serial],
        |pr| select_fragment(&fsp, inputs, flat, centroid, pr, a),
        false,
        a,
    )?;
    let (vsel, vs_code) = vertex71(&vsp, records, false, vs_key.ret32, a)?;
    let (csel, cs_code) = vertex71(&csp, records, true, vs_key.ret32, a)?;

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

    /// A refusal of one of the 7.1 back half's stages by name: what a 7.1
    /// compile returns while select71, alloc71 or sched71 is unfinished.
    fn is_stage_refusal(e: &V3d71Error) -> bool {
        matches!(
            e,
            V3d71Error::NotYet(_)
                | V3d71Error::Select(Select71Error::NotYet(_))
                | V3d71Error::Alloc(Alloc71Error::NotYet(_))
                | V3d71Error::Sched(Sched71Error::NotYet(_) | Sched71Error::ControlFlow(_))
        )
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
    fn v71_compiles_or_refuses_by_stage_name() {
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
                    Err(e) => assert!(is_stage_refusal(&e), "{what}: {e:?} is not a named stage refusal"),
                }
            });
        }
    }

    #[test]
    fn shared_refusals_are_the_4_2_chains_at_both_generations() {
        let fs = |body: &str| std::format!("precision mediump float; varying vec4 v_color; void main() {{ {body} }}");
        /* prepare.rs counted_loop: an output written in a loop that can
         * leave on data (chain.rs what_this_cut_does_not_compile_is_refused_by_name);
         * a varying the vertex shader does not declare; the points and
         * lines variants. */
        let loop_fs = fs("vec4 c = v_color; for (float f = 0.0; f < 2.0; f += 1.0) { if (c.x > 0.5) break; \
                          gl_FragColor = c; c = c * v_color; }");
        let no_varying_vs = "attribute vec4 a_pos; void main() { gl_Position = a_pos; }";
        let cases: [(&str, &str, Prim, V3dError); 4] = [
            (
                PASSTHROUGH_VS,
                &loop_fs,
                Prim::Triangles,
                V3dError::NotYet("an output written inside a loop that can leave on data"),
            ),
            (no_varying_vs, INTERP_FS, Prim::Triangles, V3dError::VaryingWithoutVertexOutput),
            (PASSTHROUGH_VS, INTERP_FS, Prim::Points, V3dError::NotYet("the points variant of the fragment shader")),
            (PASSTHROUGH_VS, INTERP_FS, Prim::Lines, V3dError::NotYet("the lines variant of the fragment shader")),
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

    #[test]
    fn a_stage_is_refused_by_name() {
        let mut buf = std::vec![0u8; 4096];
        let a = Arena::new(&mut buf);
        let sel = super::super::vop71::empty_selected(false);
        match compile_stage71(&sel, &a) {
            Ok(s) => check71("empty", &s.code, s.threads, s.single_seg),
            Err(e) => assert!(is_stage_refusal(&e), "{e:?}"),
        }
    }
}
