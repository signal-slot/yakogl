// SPDX-License-Identifier: Apache-2.0 AND MIT
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2016 Broadcom
// Derived from Mesa 26.2.1 src/broadcom/ (qpu/, compiler/) (MIT).
// The upstream part stays under its licence (LICENSES/MIT.txt); our
// changes are Apache-2.0. See NOTICE.
//! The whole V3D path for one GL program: two IR modules in, three QPU
//! programs out (the fragment shader, the vertex shader and the coordinate
//! shader the binner runs), each with exactly the prog_data the driver's
//! shader record takes (`struct v3d_fs_prog` / `v3d_vs_prog`,
//! `driver/src/v3d/v3d_frame.h`) and its uniform stream.
//!
//! A program, not a stage (`docs/online-shaderc/v3d-backend.md`): the
//! fragment shader is prepared first, its inputs decide which varyings the
//! vertex shader writes and in what order, and the coordinate shader is the
//! vertex shader with none of them.
//!
//! prepare -> select -> allocate -> schedule -> encode -> verify, for each of
//! the three. Every program that comes back has been encoded (the encoder
//! decodes each word again and refuses one that reads back as anything else)
//! and has passed [`super::verify`]. Anything the chain does not build yet is
//! refused by name ([`V3dError::NotYet`]), never emitted wrong.

use super::alloc::{allocate, AllocError};
use super::encode::{encode_program, ProgramEncodeError};
use super::isa::Instr;
use super::prepare::{fragment_inputs, prepare_fragment_with, prepare_vertex_with, PrepError, SOp};
use super::sched::{schedule, SchedError};
use super::select::{select_fragment, select_vertex, Pressure, SelectError, Selected, Uniform, VOp};
use super::verify::{branches_are_structured, registers_fit_threads, verify, VerifyError};
use crate::shaderc::online::arena::{AVec, Arena, ArenaError};
use crate::shaderc::online::ir::{self, IoKind};
use crate::shaderc::online::powervr::unroll::Trip;

/// The primitive the fragment shader is compiled for (Mesa keys the FS by it:
/// points and lines read their coordinate as implicit varyings).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Prim {
    Triangles,
    Lines,
    Points,
}

/// What the program is compiled for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Target {
    /// The VPM in bytes (`IDENT1.VPM_SIZE * 8192`); it sizes
    /// `vcm_cache_size`. The Raspberry Pi 4 reports 65536 (CTL_IDENT1
    /// 0x81001422 under Linux, 2026-09-24); v3d_shc assumes 16384 unless told.
    pub vpm_size: u32,
}

/// Why a program was not compiled. One variant per stage of the chain.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum V3dError {
    /// An IR construct, GL feature or variant this cut does not compile, by
    /// name.
    NotYet(&'static str),
    Prepare(PrepError),
    /// A varying the fragment shader reads that the vertex shader does not
    /// declare. The GL link step reports this first; the chain refuses
    /// rather than inventing a value.
    VaryingWithoutVertexOutput,
    Select(SelectError),
    Alloc(AllocError),
    Sched(SchedError),
    Encode(ProgramEncodeError),
    Verify(VerifyError),
    /// The VPM cannot hold two batches of the vertex outputs
    /// (v3d_vs_set_prog_data asserts `vpm_output_batches >= 2`, vir.c:936).
    VpmTooSmall,
    OutOfScratch,
}

impl From<ArenaError> for V3dError {
    fn from(_: ArenaError) -> Self {
        V3dError::OutOfScratch
    }
}
impl From<PrepError> for V3dError {
    fn from(e: PrepError) -> Self {
        match e {
            PrepError::NotYet(what) => V3dError::NotYet(what),
            e => V3dError::Prepare(e),
        }
    }
}
impl From<SelectError> for V3dError {
    fn from(e: SelectError) -> Self {
        V3dError::Select(e)
    }
}
impl From<AllocError> for V3dError {
    fn from(e: AllocError) -> Self {
        match e {
            /* A named gap, not a defect: [`build`] has already retried at
             * 2 threads, as Mesa does; Mesa's later strategies spill
             * (vir.c strategies[]), which this back end does not yet. */
            AllocError::DoesNotFit => V3dError::NotYet(DOES_NOT_FIT),
            e => V3dError::Alloc(e),
        }
    }
}
impl From<SchedError> for V3dError {
    fn from(e: SchedError) -> Self {
        V3dError::Sched(e)
    }
}

/// One compiled QPU program.
#[derive(Clone, Copy, Debug)]
pub struct Code<'a> {
    pub words: &'a [u64],
    /// The instructions the words encode.
    pub instrs: &'a [Instr],
    /// The uniform stream, `(contents, data)` with Mesa's quniform_contents.
    pub uniforms: &'a [Uniform],
    /// Every counted loop's back edge (branch, target instruction).
    pub back_edges: &'a [(u32, u32)],
    /// Every uniform branch's forward branch (branch, target instruction).
    pub fwd_branches: &'a [(u32, u32)],
    /// Operations sched.rs hoist moved into an earlier word, and those its
    /// 32-word window stopped short of an earlier legal word (Scheduled).
    pub hoists: u32,
    pub window_cuts: u32,
}

/// The fragment shader and the fields of `struct v3d_fs_prog`.
#[derive(Clone, Copy, Debug)]
pub struct FsProg<'a> {
    pub code: Code<'a>,
    pub threads: u32,
    pub single_seg: bool,
    pub num_inputs: u32,
    /// Per input, in `ldvary` order: (VARYING_SLOT_VAR0 + the FS location,
    /// component), as Mesa's `input_slots` prints.
    pub input_slots: &'a [(u8, u8)],
    pub flat_shade_flags: u32,
    pub noperspective_flags: u32,
    pub centroid_flags: u32,
    pub writes_z: bool,
    pub disable_ez: bool,
    pub uses_center_w: bool,
    pub lock_scoreboard_on_first_thrsw: bool,
    pub uses_implicit_point_line_varyings: bool,
    pub uses_pid: bool,
    /// Bit n: the program writes colour to render target n (prepare's
    /// rt_mask; the backend checks it against the draw's attachments).
    pub rt_mask: u8,
    pub force_per_sample_msaa: bool,
    pub tmu_dirty_rcl: bool,
    /// The shader can discard (prog_data `uses_discard`). Such a shader always
    /// runs with late tests and writes the pass-through Z (`writes_z` and
    /// `disable_ez`; select.rs select_fragment): Mesa's output for a draw
    /// that writes depth or stencil (`can_earlyz_with_discard` false).
    pub uses_discard: bool,
}

/// A vertex or coordinate shader and the fields of `struct v3d_vs_prog`.
#[derive(Clone, Copy, Debug)]
pub struct VsProg<'a> {
    pub code: Code<'a>,
    pub threads: u32,
    pub single_seg: bool,
    /// In sectors of 8 rows (vir.c:905-920).
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

/// A compiled GL program.
#[derive(Clone, Copy, Debug)]
pub struct V3dProgram<'a> {
    pub fs: FsProg<'a>,
    pub vs: VsProg<'a>,
    pub cs: VsProg<'a>,
    /// One entry per vertex attribute LOCATION, ascending: (location, the
    /// attribute record it is fetched through), `None` when the vertex
    /// shader does not read it. A `matN` attribute declared at location `L`
    /// takes the N locations `L..L+N`, one column each (ES 2.0 section
    /// 2.10.4), and each column has its own entry and its own record, as
    /// Mesa's `vattr_sizes` has one per column.
    pub attributes: &'a [(u8, Option<u8>)],
}

/// How many attribute locations a declaration of `gl_type` takes: one per
/// column of `GL_FLOAT_MAT2`/`3`/`4` (0x8B5A..0x8B5C), one otherwise.
pub fn attribute_locations(gl_type: u16) -> u8 {
    match gl_type {
        0x8b5a => 2,
        0x8b5b => 3,
        0x8b5c => 4,
        _ => 1,
    }
}

/// `VARYING_SLOT_VAR0` (compiler/shader_enums.h): Mesa's slot number of the
/// first generic varying.
pub const VARYING_SLOT_VAR0: u8 = 32;

/// `m` with its loops expanded where the unroller can (`None` from it: no
/// loop, `m` as it is). Its refusals are named gaps, not verdicts on the
/// shader.
fn unrolled<'a>(m: &'a ir::Module<'a>, a: &'a Arena<'a>) -> Result<(&'a ir::Module<'a>, &'a [Trip]), V3dError> {
    use crate::shaderc::online::powervr::unroll::{unroll, UnrollError};
    match unroll(m, a) {
        Ok(Some(u)) => Ok((u.module, u.trips)),
        Ok(None) => Ok((m, &[])),
        Err(UnrollError::OutOfScratch) => Err(V3dError::OutOfScratch),
        Err(e) => Err(V3dError::NotYet(e.message())),
    }
}

/// The refusal of a program whose values do not fit 2 threads' registers.
const DOES_NOT_FIT: &str = "more live values than 2 threads have registers (no spilling yet)";

/// The selection levels the chain tries, in order, while a program does not
/// fit 2 threads' registers (select.rs [`Pressure`]): Mesa's layout; then
/// each attribute read where it is first needed, which sixteen vec4
/// attributes summed into one need (dEQP attribute_location
/// bind_max_attributes: 64 values live at once otherwise); then every
/// constant loaded again at each read, which a 40-element array of
/// constants read at a run-time index needs (dEQP indexing.tmp_array
/// *_const_write_*: some 50 constants live at once otherwise); then each
/// look-up with its own thread switch, as queued results are live longer
/// (Mesa's "disable TMU pipelining" strategy, vir.c:2197).
const PRESSURES: [Pressure; 4] = [Pressure::None, Pressure::LazyAttributes, Pressure::Remat, Pressure::Serial];

/// A vertex or coordinate shader starts in its last thread section unless
/// it switches threads (Mesa's `c->prog_data->single_seg = !c->last_thrsw`;
/// its non-fragment stages have a THRSW only with a TMU access).
fn starts_in_last_section(sel: &Selected<'_>) -> bool {
    !sel.ops.contains(&VOp::Thrsw)
}

/// Select and build at each of [`PRESSURES`] until the program fits.
fn under_pressure<'a>(
    stages: &[Pressure],
    select: impl Fn(Pressure) -> Result<Selected<'a>, SelectError>,
    single_seg: bool,
    a: &Arena<'a>,
) -> Result<(Selected<'a>, (Code<'a>, u32)), V3dError> {
    for &pr in stages {
        let sel = select(pr)?;
        match build(&sel, single_seg && starts_in_last_section(&sel), a) {
            Err(V3dError::NotYet(w)) if w == DOES_NOT_FIT => continue,
            r => return Ok((sel, r?)),
        }
    }
    Err(V3dError::NotYet(DOES_NOT_FIT))
}

/// A vertex or coordinate shader all the way to its words.
fn vertex<'a>(
    p: &super::prepare::Prepared<'_>,
    records: &[u8],
    coord: bool,
    a: &Arena<'a>,
) -> Result<(Selected<'a>, (Code<'a>, u32)), V3dError> {
    under_pressure(&PRESSURES, |pr| select_vertex(p, records, coord, pr, a), true, a)
}

/// A program all the way to its words, and the thread count it runs at: 4
/// when its values fit 4 threads' registers, else 2, whose threads have
/// twice the register file (v3d_nir_to_vir_finish, nir_to_vir.c:5160-5200:
/// 4, then 2, and never 1 without spilling). Nothing else in the program
/// depends on the count; the shader record's 4-way flag carries it.
fn build<'a>(sel: &Selected<'_>, single_seg: bool, a: &Arena<'a>) -> Result<(Code<'a>, u32), V3dError> {
    let (al, threads) = match allocate(sel, 4, a) {
        Ok(al) => (al, 4),
        Err(AllocError::DoesNotFit) => (allocate(sel, 2, a)?, 2),
        Err(e) => return Err(e.into()),
    };
    let mut s = schedule(sel, &al, a)?;
    if super::order::on() {
        /* the list scheduler's order (order.rs), kept when it packs
         * shorter: its model of a word is not the packer's, and on some
         * programs (femtovg's fill_color: the mul slot given to the longer
         * chain delays the next varying) the selected order packs one to
         * four words shorter */
        let ordered = Selected { ops: super::order::reorder(sel, &al, a)?, ..*sel };
        let t = schedule(&ordered, &al, a)?;
        if t.instrs.len() < s.instrs.len() {
            s = t;
        }
    }
    let words = a.alloc_filled(s.instrs.len(), 0u64)?;
    encode_program(s.instrs, words).map_err(V3dError::Encode)?;
    verify(s.instrs, single_seg).map_err(V3dError::Verify)?;
    registers_fit_threads(s.instrs, threads).map_err(V3dError::Verify)?;
    branches_are_structured(s.instrs, s.back_edges, s.fwd_branches).map_err(V3dError::Verify)?;
    let code = Code {
        words,
        instrs: s.instrs,
        uniforms: s.uniforms,
        back_edges: s.back_edges,
        fwd_branches: s.fwd_branches,
        hoists: s.hoists,
        window_cuts: s.window_cuts,
    };
    Ok((code, threads))
}

/// v3d_vs_set_prog_data's VPM arithmetic (vir.c:896-937): the sizes in
/// sectors with shared input/output segments, and the VCM cache size.
fn vpm_fields(sel: &Selected<'_>, target: Target) -> Result<(u32, u32, u32), V3dError> {
    /* the attribute rows and one per builtin read (vir.c:893-898), and the
     * row the driver loads for attribute 0 when the stage reads an ID and
     * none of attribute 0 (GFXH-1602 in a coordinate shader, or the
     * GFXH-930 dummy record when there is no attribute at all; v3d_job.c).
     * Mesa leaves that row out of the count; counting it can only make the
     * shared segment larger, never smaller than the rows the fetch writes. */
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

fn vs_prog<'a>(
    sel: &Selected<'_>,
    (code, threads): (Code<'a>, u32),
    target: Target,
    writes_psiz: bool,
) -> Result<VsProg<'a>, V3dError> {
    let (vpm_input_size, vpm_output_size, vcm_cache_size) = vpm_fields(sel, target)?;
    Ok(VsProg {
        code,
        threads,
        /* 4.x spawns the non-fragment stages in the last thread section
         * (vir_emit_last_thrsw, nir_to_vir.c:5010-5017; vir.c:1041), one
         * with no thread switch of its own. */
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

/// Compile the GL program `vs` + `fs` for `prim` on `target`, with the
/// fragment stage's sampler-state key the compile in progress carries
/// ([`crate::shaderc::compile_lod_key`]) and no vertex key.
pub fn compile_program<'a>(
    vs: &'a ir::Module<'a>,
    fs: &'a ir::Module<'a>,
    prim: Prim,
    target: Target,
    a: &'a Arena<'a>,
) -> Result<V3dProgram<'a>, V3dError> {
    compile_program_keyed(vs, fs, prim, target, crate::shaderc::LodKey::NONE, crate::shaderc::compile_lod_key(), a)
}

/// [`compile_program`] with each stage's [`crate::shaderc::LodKey`]: what a
/// draw's bound samplers ask of the stage's code. The V3D's keys are
/// `cube_noseam` (both stages: the cube look-ups of those bindings are
/// emulated per face, docs/v3d-noseamless-cube.md) and, of the fragment
/// key, `ret32` (read by select.rs from the compile's key). The `mag_switch`
/// remap is IR to IR and applied to the vertex module by the caller
/// (`codegen::link_sources`).
pub fn compile_program_keyed<'a>(
    vs: &'a ir::Module<'a>,
    fs: &'a ir::Module<'a>,
    prim: Prim,
    target: Target,
    vs_key: crate::shaderc::LodKey,
    fs_key: crate::shaderc::LodKey,
    a: &'a Arena<'a>,
) -> Result<V3dProgram<'a>, V3dError> {
    match prim {
        Prim::Triangles => {}
        Prim::Lines => return Err(V3dError::NotYet("the lines variant of the fragment shader")),
        Prim::Points => return Err(V3dError::NotYet("the points variant of the fragment shader")),
    }
    /* Loops first, IR to IR (powervr/unroll.rs, used as it is; its doc
     * says what it does): every loop that leaves only by its GLSL ES 1.00
     * Appendix A test is expanded into its iterations. A loop it keeps (one
     * that can leave on data) stays a Node::Loop, which prepare refuses by
     * name until the counted loop exists (docs/online-shaderc/v3d-backend.md,
     * "Control flow, step 2"). */
    let (vs, vs_trips) = unrolled(vs, a)?;
    let (fs, fs_trips) = unrolled(fs, a)?;

    /* The fragment shader, and the order of its inputs. */
    let fsp = prepare_fragment_with(fs, fs_trips, fs_key.cube_noseam, a)?;
    let inputs = fragment_inputs(&fsp, a)?;
    /* Bit i: input i is flat (Mesa emit_fragment_varying: BITSET_SET of
     * flat_shade_flags at the input's index). The job builder reads word 0
     * only and refuses more than 24 inputs. */
    let mut flat = 0u32;
    for (i, &(loc, _)) in inputs.iter().enumerate() {
        let d = fs
            .inputs
            .iter()
            .find(|d| d.kind == IoKind::Varying && d.location == loc)
            .ok_or(V3dError::VaryingWithoutVertexOutput)?;
        if d.interp == crate::shaderc::online::ir::INTERP_FLAT {
            if i >= 32 {
                return Err(V3dError::NotYet("a flat varying past input 31"));
            }
            flat |= 1 << i;
        }
    }

    /* Each input to the vertex shader's varying of the same name. */
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

    let vsp = prepare_vertex_with(vs, wanted, vs_trips, vs_key.cube_noseam, a)?;
    let csp = prepare_vertex_with(vs, &[], vs_trips, vs_key.cube_noseam, a)?;

    /* The attribute records: every attribute the vertex shader reads, in
     * ascending location (the coordinate shader reads a subset). */
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
        return Err(V3dError::NotYet("more than 16 vertex attributes"));
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
    /* Every location read has an entry: a record with no location would be
     * fetched from a vertex array the driver cannot name. */
    if records.iter().any(|l| !attrs.as_slice().iter().any(|e| e.0 == *l)) {
        return Err(V3dError::NotYet("an attribute location with no declaration"));
    }

    /* LazyAttributes changes nothing in a fragment shader. */
    let (fsel, (fs_code, fs_threads)) = under_pressure(
        &[Pressure::None, Pressure::Remat, Pressure::Serial],
        |pr| select_fragment(&fsp, inputs, flat, pr, a),
        false,
        a,
    )?;
    let (vsel, vs_code) = vertex(&vsp, records, false, a)?;
    let (csel, cs_code) = vertex(&csp, records, true, a)?;

    let slots = a.alloc_filled(inputs.len(), (0u8, 0u8))?;
    for (s, &(loc, comp)) in slots.iter_mut().zip(inputs) {
        *s = (VARYING_SLOT_VAR0 + loc, comp);
    }
    Ok(V3dProgram {
        fs: FsProg {
            code: fs_code,
            threads: fs_threads,
            /* The fragment shader always has its last-thread-switch pair. */
            single_seg: false,
            num_inputs: inputs.len() as u32,
            input_slots: slots,
            flat_shade_flags: flat,
            noperspective_flags: 0,
            centroid_flags: 0,
            /* select.rs select_fragment: a discard or a read of Z */
            writes_z: fsel.writes_z,
            disable_ez: fsel.disable_ez,
            uses_center_w: fsel.uses_center_w,
            lock_scoreboard_on_first_thrsw: false,
            /* select.rs select_fragment: the shader reads gl_PointCoord */
            uses_implicit_point_line_varyings: fsel.implicit_point_varyings,
            uses_pid: false,
            rt_mask: fsp.rt_mask,
            /* Mesa flags a shader with no colour write as discarding
             * (nir_to_vir.c:2124-2126): its Z write is its TLB write. */
            uses_discard: fsp.discard.is_some() || fsp.rt_mask == 0,
            force_per_sample_msaa: false,
            tmu_dirty_rcl: false,
        },
        vs: vs_prog(&vsel, vs_code, target, vsp.psiz.is_some())?,
        cs: vs_prog(&csel, cs_code, target, csp.psiz.is_some())?,
        attributes: attrs.into_slice(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::sema::Stage;
    use crate::shaderc::online::v3d::decode::{decode, same_instr};
    use crate::shaderc::online::v3d::isa::{AddOp, Alu, BranchCond, MulOp, Mux};
    use crate::shaderc::online::v3d::listing::write_instr;
    use crate::shaderc::online::v3d::select::{QUNIFORM_FB_WPOS_Y_OFFSET, QUNIFORM_FB_WPOS_Y_SCALE, TLB_Z_FROM_FEP};
    use crate::shaderc::online::{lower, store};
    use std::string::String;
    use std::vec::Vec;

    static PASSTHROUGH_VS: &str = include_str!("../../../../tools/shaders/gl_test_es100/passthrough.vert");
    static INTERP_FS: &str = include_str!("../../../../tools/shaders/gl_test_es100/interp_color.frag");
    static COLOR_UV_VS: &str = include_str!("../../../../tools/shaders/gl_test_es100/color_uv.vert");
    static COLOR_UV_FS: &str = include_str!("../../../../tools/shaders/gl_test_es100/color_uv.frag");
    /// A constant colour: `vec4(1.0, 1.0, 0.66, 1.0)`.
    static HELLO_FS: &str = include_str!("../../../../tools/shaders/es100/hello.frag");

    const TARGET: Target = Target { vpm_size: 65536 };

    /* Mesa's code for the same three programs, for reading side by side
     * (never compared word for word: the two compilers differ). Made with
     * tools/v3d/v3d_shc (Mesa 26.2.1's v3d compiler) from the SPIR-V
     * tools/v3d/glsl2v3d.py makes of these ES 1.00 sources:
     *   v3d_shc vs.spv fs.spv --vpm-size 65536
     * (the same words as with the default 16384 for all nine). */
    const MESA_INTERP_FS: &[u64] = &[
        0x3d103186bb800000,
        0x5510b046bbc00000,
        0x551120c305ca9000,
        0x5530600405d2b000,
        0x5420208505c68000,
        0x3c0021860582a000,
        0x3c2031873583e0c4,
        0x3c0031873583e146,
        0x3c003186bb800000,
    ];
    const MESA_INTERP_VS: &[u64] = &[
        0x3c403186bb800000,
        0x3d903186bb800000,
        0x3d906183bc800000,
        0x559082c4bcb810c0,
        0x3d90e185bc802000,
        0x3d912186bc803000,
        0x3d902187bc804000,
        0x3d906188bc800000,
        0x3d90a189bc801000,
        0x3d90e18abc802000,
        0x3c40218cba82e180,
        0x54002100f87bc107,
        0x540030c6bbf802cc,
        0x54002100f8f3020c,
        0x3c003180f6803000,
        0x54402040f8bb9149,
        0x3c003183f6804000,
        0x54003004f5c78300,
        0x3d90a180f8832280,
        0x3c003181f583b000,
        0x3c40318305828000,
        0x3c002180f8822000,
        0x3c402180f880d000,
        0x3c402180f881d000,
        0x3c002180f8835300,
        0x3c003186bb816000,
        0x3c203186bb800000,
        0x3c003186bb800000,
        0x3c003186bb800000,
    ];
    const MESA_INTERP_CS: &[u64] = &[
        0x3c403186bb800000,
        0x3d903186bb800000,
        0x3d906183bc800000,
        0x559081c4bcb810c0,
        0x3d90e185bc802000,
        0x3d912186bc803000,
        0x3c402188ba82e180,
        0x54002000f89f00c4,
        0x54003106bbf801c8,
        0x54002000f8e31108,
        0x3c003181f6804000,
        0x3c002180f8832140,
        0x3c003184f5839000,
        0x3c002180f8833180,
        0x3c003182f6800000,
        0x3c402180f8825000,
        0x3c003180f583a000,
        0x3c002180f8805000,
        0x3c003186bb816000,
        0x3c203186bb800000,
        0x3c003186bb800000,
        0x3c003186bb800000,
    ];
    const MESA_COLOR_UV_FS: &[u64] = &[
        0x3d103186bb800000,
        0x5510f046bbc00000,
        0x5510710205ce9000,
        0x551130c005c6c000,
        0x5510e04305d2b000,
        0x5510e10405ce9000,
        0x5420310105cec000,
        0x54201143052ac000,
        0x54001186bb600000,
        0x3c2031873583e146,
        0x3c0031873583e0c4,
        0x3c003186bb800000,
    ];
    const MESA_COLOR_UV_VS: &[u64] = &[
        0x3c403186bb800000,
        0x3c402183bc805000,
        0x3c402184bc805000,
        0x3c402185bc805000,
        0x3c402186bc805000,
        0x3d902187bc805000,
        0x3d906188bc800000,
        0x3d90a189bc801000,
        0x3d90e18abc802000,
        0x3d91218bbc803000,
        0x3c00218cbc804000,
        0x3c402180f88351c0,
        0x3c00218dba82e180,
        0x54402000f8bb80c8,
        0x54002040f8e3124d,
        0x54402000f8bba10a,
        0x54002080f8e332cd,
        0x544030c1f6b81140,
        0x3d912180f8834300,
        0x3c003180f5839000,
        0x54003081f6cc2340,
        0x3c002180f8804000,
        0x3c4031800582a000,
        0x3c003183f5839000,
        0x3c402180f881d000,
        0x3c402180f8805000,
        0x3c002180f8835340,
        0x3c003186bb816000,
        0x3c203186bb800000,
        0x3c003186bb800000,
        0x3c003186bb800000,
    ];
    const MESA_COLOR_UV_CS: &[u64] = &[
        0x3c403186bb800000,
        0x3d903186bb800000,
        0x3d906183bc800000,
        0x559081c4bcb810c0,
        0x3d90e185bc802000,
        0x3d912186bc803000,
        0x3c402188ba82e180,
        0x54002000f89f00c4,
        0x54003106bbf801c8,
        0x54002000f8e31108,
        0x3c003181f6804000,
        0x3c002180f8832140,
        0x3c003184f5839000,
        0x3c002180f8833180,
        0x3c003182f6800000,
        0x3c402180f8825000,
        0x3c003180f583a000,
        0x3c002180f8805000,
        0x3c003186bb816000,
        0x3c203186bb800000,
        0x3c003186bb800000,
        0x3c003186bb800000,
    ];
    const MESA_HELLO_FS: &[u64] = &[
        0x3c203186bb800000,
        0x3c203186bb800000,
        0x3c003186bb800000,
        0x3d903186bb800000,
        0x3db071c6bbe00000,
        0x3c0031c6bbe40000,
        0x3c003186bb800000,
    ];
    const MESA_HELLO_VS: &[u64] = &[
        0x3c403186bb800000,
        0x3d903186bb800000,
        0x3d906183bc800000,
        0x558160c4bcb810c0,
        0x3d90a187ba82e100,
        0x3c402186bc806140,
        0x55906108bc585180,
        0x54003006bbcc01c0,
        0x54003083f6d001c0,
        0x54003101f53bb200,
        0x5590b0c0f6d021c0,
        0x3d903184f5838000,
        0x3c002180f8808000,
        0x3c0031810581a000,
        0x3c002180f8826140,
        0x3c402180f880d000,
        0x3c002180f88351c0,
        0x3c003186bb816000,
        0x3c203186bb800000,
        0x3c003186bb800000,
        0x3c003186bb800000,
    ];
    const MESA_HELLO_CS: &[u64] = &[
        0x3c403186bb800000,
        0x3d903186bb800000,
        0x3d906183bc800000,
        0x559081c4bcb810c0,
        0x3d90e185bc802000,
        0x3d912186bc803000,
        0x3c402188ba82e180,
        0x54002000f89f00c4,
        0x54003106bbf801c8,
        0x54002000f8e31108,
        0x3c003181f6804000,
        0x3c002180f8832140,
        0x3c003184f5839000,
        0x3c002180f8833180,
        0x3c003182f6800000,
        0x3c402180f8825000,
        0x3c003180f583a000,
        0x3c002180f8805000,
        0x3c003186bb816000,
        0x3c203186bb800000,
        0x3c003186bb800000,
        0x3c003186bb800000,
    ];

    /// Compile `vs` + `fs` and hand the program to `f`.
    fn with_program(vs: &str, fs: &str, prim: Prim, f: impl FnOnce(Result<&V3dProgram<'_>, V3dError>)) {
        store::with_scratch(|a| {
            let vs = a.alloc_str(vs).unwrap();
            let fs = a.alloc_str(fs).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vs).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fs).map_err(|e| e.msg).unwrap();
            match compile_program(&v.module, &fr.module, prim, TARGET, a) {
                Ok(p) => f(Ok(&p)),
                Err(e) => f(Err(e)),
            }
        })
        .expect("scratch");
    }

    fn text(i: &Instr) -> String {
        let mut s = String::new();
        write_instr(&mut s, i).unwrap();
        s
    }

    /// The words decode, and to the instructions the chain meant.
    fn decodes_back(what: &str, c: &Code<'_>) {
        assert_eq!(c.words.len(), c.instrs.len(), "{what}");
        for (n, (w, i)) in c.words.iter().zip(c.instrs).enumerate() {
            let back = decode(*w).unwrap_or_else(|e| panic!("{what} word {n} 0x{w:016x} does not decode: {e:?}"));
            assert!(same_instr(i, &back), "{what} word {n} reads back as something else: {back:?}, meant {i:?}");
        }
    }

    fn side_by_side(what: &str, ours: &Code<'_>, mesa: &[u64]) {
        let theirs: Vec<Instr> = mesa.iter().map(|w| decode(*w).expect("Mesa's word decodes")).collect();
        std::println!("--- {what}: ours {} instructions, Mesa {}", ours.instrs.len(), theirs.len());
        for n in 0..ours.instrs.len().max(theirs.len()) {
            let o = ours.instrs.get(n).map(text).unwrap_or_default();
            let m = theirs.get(n).map(text).unwrap_or_default();
            std::println!("{n:3} {o:<74}| {m}");
        }
        std::print!("    our uniforms:");
        for u in ours.uniforms {
            std::print!(" ({}, 0x{:08x})", u.contents, u.data);
        }
        std::println!();
    }

    /// What every program that comes back must satisfy, whatever it is.
    /// The first TMU access that is not a 2D look-up of a bound texture, as
    /// text; `None` when every one is. A look-up writes only TMUS, TMUT and
    /// TMUB (its coordinates and bias) and a `wrtmuc` per P0/P1 uniform
    /// (contents 40/41, which the driver resolves to that texture's state), so
    /// it reads nothing but the texture. Every other TMU register (TMUD, TMUA,
    /// TMUAU, TMUC, TMUR, TMUI, TMUDREF, TMUOFF, TMUSCM, TMUSF, TMUSLOD, TMUHS*)
    /// is a general memory access or a form this back end does not select, and
    /// is reported. `uniforms` are the stream's contents codes.
    fn first_tmu_memory_access(instrs: &[Instr], uniforms: &[(u32, u32)]) -> Option<String> {
        let tmu = |magic: bool, w: u8| magic && ((11..=13).contains(&w) || (32..=46).contains(&w));
        /* a look-up's writes: TMUS, TMUSLOD (a 2D explicit level, every
         * vertex-stage 2D look-up) or TMUSCM (a cube) retires it; TMUT, TMUR
         * (a cube's r), TMUI (an array look-up's layer) and TMUB (bias or
         * level) */
        let lookup = |w: u8| matches!(w, 33 | 34 | 35 | 36 | 37 | 40 | 42);
        for (n, i) in instrs.iter().enumerate() {
            if let Instr::Alu(a) = i {
                for (magic, w) in [(a.add.magic, a.add.waddr), (a.mul.magic, a.mul.waddr)] {
                    if tmu(magic, w) && !lookup(w) {
                        return Some(std::format!("word {n}: {}", text(i)));
                    }
                }
            }
        }
        let wrtmuc = instrs.iter().filter(|i| matches!(i, Instr::Alu(a) if a.sig.wrtmuc)).count();
        /* P0, P1 (their own contents) and a cube's P2 at a level (a constant
         * uniform, select.rs TMU_P2_CUBE_LOD) */
        let p2 = super::super::select::TMU_P2_CUBE_LOD;
        let configs = uniforms.iter().filter(|&&(c, d)| c == 40 || c == 41 || (c == 0 && d == p2)).count();
        (wrtmuc != configs).then(|| std::format!("{wrtmuc} wrtmuc words but {configs} P0/P1 uniforms"))
    }

    /// Per word of `c` that consumes a uniform, in order, the word and its
    /// uniform: ldunif, ldunifrf, wrtmuc, a TLBU write on either unit, and a
    /// branch that moves the stream (`ub`). Asserts that the stream holds
    /// exactly one entry per such word.
    fn uniform_consumers<'c>(what: &str, c: &'c Code<'_>) -> Vec<(&'c Instr, Uniform)> {
        let mut out = Vec::new();
        for i in c.instrs {
            let takes = match i {
                Instr::Alu(a) => {
                    a.sig.ldunif
                        || a.sig.ldunifrf
                        || a.sig.wrtmuc
                        || (a.add.magic && a.add.waddr == 8)
                        || (a.mul.magic && a.mul.waddr == 8)
                }
                Instr::Branch(b) => b.ub,
            };
            if takes {
                let u = *c.uniforms.get(out.len()).unwrap_or_else(|| panic!("{what}: a word with no uniform left"));
                out.push((i, u));
            }
        }
        assert_eq!(out.len(), c.uniforms.len(), "{what}: uniforms nothing consumes");
        out
    }

    /// The TLBU writes of `c` whose specifier is the pass-through Z.
    fn z_from_fep_writes(what: &str, c: &Code<'_>) -> usize {
        uniform_consumers(what, c)
            .iter()
            .filter(|(i, u)| {
                let tlbu = matches!(i, Instr::Alu(a) if (a.add.magic && a.add.waddr == 8) || (a.mul.magic && a.mul.waddr == 8));
                tlbu && *u == Uniform::constant(TLB_Z_FROM_FEP)
            })
            .count()
    }

    fn check_all(p: &V3dProgram<'_>) {
        for (what, c, seg) in [
            ("fs", &p.fs.code, p.fs.single_seg),
            ("vs", &p.vs.code, p.vs.single_seg),
            ("cs", &p.cs.code, p.cs.single_seg),
        ] {
            decodes_back(what, c);
            assert_eq!(verify(c.instrs, seg), Ok(()), "{what}");
            assert_eq!(branches_are_structured(c.instrs, c.back_edges, c.fwd_branches), Ok(()), "{what}");
            let words: Vec<(u32, u32)> = c.uniforms.iter().map(|u| (u.contents, u.data)).collect();
            assert_eq!(first_tmu_memory_access(c.instrs, &words), None, "{what} reaches memory through the TMU");
        }
        /* 4 threads unless the registers do not fit them, then 2: the
         * fallback is taken exactly when it is needed. */
        for (what, c, threads) in
            [("fs", &p.fs.code, p.fs.threads), ("vs", &p.vs.code, p.vs.threads), ("cs", &p.cs.code, p.cs.threads)]
        {
            assert!(threads == 4 || threads == 2, "{what}: {threads} threads");
            assert_eq!(registers_fit_threads(c.instrs, threads), Ok(()), "{what}");
            assert_eq!(threads == 2, registers_fit_threads(c.instrs, 4).is_err(), "{what}: {threads} threads");
        }
        /* The fragment shader always has its last-thread-switch pair; a
         * vertex or coordinate shader starts in its last section exactly when
         * its code has no thread switch (a texture look-up gives it one). */
        assert!(!p.fs.single_seg);
        for (what, c, seg) in [("vs", &p.vs.code, p.vs.single_seg), ("cs", &p.cs.code, p.cs.single_seg)] {
            /* one THRSW signal is the program end (THREND) */
            let thrsw = c.instrs.iter().filter(|i| matches!(i, Instr::Alu(a) if a.sig.thrsw)).count();
            assert_eq!(seg, thrsw == 1, "{what}: single_seg and the code's {thrsw} THRSW signals disagree");
        }
        /* A discarding shader runs with late tests and writes the pass-
         * through Z, once; one that reads Z writes it too (select.rs
         * select_fragment); no vertex or coordinate shader writes Z. */
        assert_eq!(p.fs.disable_ez, p.fs.uses_discard, "disable_ez is the discard");
        assert!(p.fs.writes_z || !p.fs.uses_discard, "a discard writes Z");
        assert_eq!(z_from_fep_writes("fs", &p.fs.code), usize::from(p.fs.writes_z), "fs Z writes");
        assert_eq!(z_from_fep_writes("vs", &p.vs.code), 0, "vs Z writes");
        assert_eq!(z_from_fep_writes("cs", &p.cs.code), 0, "cs Z writes");
    }

    #[test]
    fn passthrough_and_interp_color() {
        with_program(PASSTHROUGH_VS, INTERP_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            /* The fields Mesa reports for the same pair (v3d_shc). */
            assert_eq!(p.fs.num_inputs, 4);
            assert_eq!(p.fs.input_slots, [(32, 0), (32, 1), (32, 2), (32, 3)]);
            assert!(p.fs.uses_center_w);
            assert_eq!(p.fs.code.uniforms, []);
            assert_eq!(p.vs.vattr_sizes[..3], [4, 4, 0]);
            assert_eq!(p.cs.vattr_sizes[..3], [4, 0, 0]);
            assert_eq!((p.vs.vpm_input_size, p.vs.vpm_output_size, p.vs.vcm_cache_size), (0, 1, 4));
            assert_eq!((p.cs.vpm_input_size, p.cs.vpm_output_size, p.cs.vcm_cache_size), (0, 1, 4));
            assert_eq!(p.attributes, [(0, Some(0)), (1, Some(1))]);
            side_by_side("interp_color FS", &p.fs.code, MESA_INTERP_FS);
            side_by_side("passthrough VS", &p.vs.code, MESA_INTERP_VS);
            side_by_side("passthrough CS", &p.cs.code, MESA_INTERP_CS);
        });
    }

    #[test]
    fn color_uv() {
        with_program(COLOR_UV_VS, COLOR_UV_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert_eq!(p.fs.num_inputs, 6);
            assert_eq!(p.fs.input_slots, [(32, 0), (32, 1), (32, 2), (32, 3), (33, 0), (33, 1)]);
            assert!(p.fs.uses_center_w);
            assert_eq!(p.vs.vattr_sizes[..4], [4, 4, 2, 0]);
            assert_eq!(p.cs.vattr_sizes[..4], [4, 0, 0, 0]);
            assert_eq!((p.vs.vpm_output_size, p.vs.vcm_cache_size), (2, 4));
            assert_eq!((p.cs.vpm_output_size, p.cs.vcm_cache_size), (1, 4));
            assert_eq!(p.attributes, [(0, Some(0)), (1, Some(1)), (2, Some(2))]);
            side_by_side("color_uv FS", &p.fs.code, MESA_COLOR_UV_FS);
            side_by_side("color_uv VS", &p.vs.code, MESA_COLOR_UV_VS);
            side_by_side("color_uv CS", &p.cs.code, MESA_COLOR_UV_CS);
        });
    }

    #[test]
    fn constant_colour() {
        with_program(PASSTHROUGH_VS, HELLO_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert_eq!(p.fs.num_inputs, 0);
            assert!(!p.fs.uses_center_w);
            /* a_color feeds only v_color, which this FS does not read. */
            assert_eq!(p.vs.vattr_sizes[..2], [4, 0]);
            assert_eq!(p.attributes, [(0, Some(0)), (1, None)]);
            /* 1.0 is a small immediate; 0.66 is not, and is the one uniform. */
            assert_eq!(p.fs.code.uniforms, [Uniform::constant(0.66f32.to_bits())]);
            side_by_side("hello FS", &p.fs.code, MESA_HELLO_FS);
            side_by_side("passthrough VS (for hello)", &p.vs.code, MESA_HELLO_VS);
            side_by_side("passthrough CS (for hello)", &p.cs.code, MESA_HELLO_CS);
        });
    }

    #[test]
    fn the_uniform_stream_is_the_order_the_words_consume_it() {
        /* One entry per ldunifrf or tlbu, word by word: the driver writes
         * exactly this list, so a word that consumes a uniform without an
         * entry (or an entry nothing consumes) shifts every later one. */
        with_program(COLOR_UV_VS, COLOR_UV_FS, Prim::Triangles, |p| {
            let p = p.unwrap();
            for (what, c) in [("fs", &p.fs.code), ("vs", &p.vs.code), ("cs", &p.cs.code)] {
                let consumers = c
                    .instrs
                    .iter()
                    .filter(|i| match i {
                        Instr::Alu(a) => {
                            a.sig.ldunifrf
                                || a.sig.ldunif
                                || (a.add.magic && a.add.waddr == 8)
                                || (a.mul.magic && a.mul.waddr == 8)
                        }
                        _ => false,
                    })
                    .count();
                assert_eq!(consumers, c.uniforms.len(), "{what}");
                assert!(consumers > 0 || what == "fs", "{what}: a vertex program always reads uniforms");
            }
        });
    }

    #[test]
    fn mesa_programs_pass_the_verifier() {
        /* The verifier's positive control: nine programs of the reference
         * compiler, including our own accumulator rule. */
        for (what, words, seg) in [
            ("interp FS", MESA_INTERP_FS, false),
            ("interp VS", MESA_INTERP_VS, true),
            ("interp CS", MESA_INTERP_CS, true),
            ("color_uv FS", MESA_COLOR_UV_FS, false),
            ("color_uv VS", MESA_COLOR_UV_VS, true),
            ("color_uv CS", MESA_COLOR_UV_CS, true),
            ("hello FS", MESA_HELLO_FS, false),
            ("hello VS", MESA_HELLO_VS, true),
            ("hello CS", MESA_HELLO_CS, true),
        ] {
            let prog: Vec<Instr> = words.iter().map(|w| decode(*w).unwrap()).collect();
            assert_eq!(verify(&prog, seg), Ok(()), "{what}");
        }
    }

    fn alu_mut(p: &mut [Instr], n: usize) -> &mut Alu {
        match &mut p[n] {
            Instr::Alu(a) => a,
            _ => panic!("not an ALU word"),
        }
    }

    #[test]
    fn the_verifier_refuses_each_broken_rule() {
        /* Negative controls: one rule broken per copy of a program that
         * passes, and the rule named must be the one broken. */
        with_program(PASSTHROUGH_VS, INTERP_FS, Prim::Triangles, |p| {
            let p = p.unwrap();
            let good: Vec<Instr> = p.fs.code.instrs.to_vec();
            assert_eq!(verify(&good, false), Ok(()));
            let first_thrsw = good.iter().position(|i| matches!(i, Instr::Alu(a) if a.sig.thrsw)).unwrap();
            let end = good.len() - 3;
            let broke = |f: &dyn Fn(&mut Vec<Instr>)| {
                let mut b = good.clone();
                f(&mut b);
                verify(&b, false).unwrap_err().rule
            };
            assert_eq!(
                broke(&|b| {
                    for i in b.iter_mut() {
                        if let Instr::Alu(a) = i {
                            if a.sig.thrsw && !matches!(a.add.op, AddOp::Nop) {
                                a.sig.thrsw = false;
                            }
                        }
                    }
                    let last = b.len() - 3;
                    alu_mut(b, last).sig.thrsw = false;
                }),
                "No program-end THRSW found"
            );
            assert_eq!(broke(&|b| alu_mut(b, first_thrsw + 2).sig.ldvary = true), "LDVARY during THRSW delay slots");
            assert_eq!(
                broke(&|b| {
                    let a = alu_mut(b, end + 2);
                    a.mul =
                        crate::shaderc::online::v3d::isa::MulAlu { op: MulOp::Fmul, magic: false, waddr: 9, ..a.mul };
                }),
                "RF write after THREND"
            );
            assert_eq!(
                broke(&|b| {
                    let a = alu_mut(b, end);
                    a.mul = crate::shaderc::online::v3d::isa::MulAlu { op: MulOp::Mov, magic: true, waddr: 7, ..a.mul };
                }),
                "Only one of [TMU, SFU, TSY, TLB read, VPM] allowed"
            );
            assert_eq!(
                broke(&|b| {
                    /* The first colour write after the switch reads r0. */
                    let n = b.iter().position(|i| matches!(i, Instr::Alu(a) if a.add.op == AddOp::Vfpack)).unwrap();
                    alu_mut(b, n).add.a.mux = Mux::R0;
                }),
                "accumulator read with no value written in this thread section (ours)"
            );
            assert_eq!(
                broke(&|b| {
                    /* The first word whose mul writes a register gets an
                     * FADD of r0 and rf0 writing that same register: the add
                     * and the mul then both write it in one instruction. */
                    let n = b
                        .iter()
                        .position(|i| matches!(i, Instr::Alu(a) if a.mul.op == MulOp::Fmul && a.add.op == AddOp::Nop))
                        .unwrap();
                    let w = alu_mut(b, n);
                    w.add = crate::shaderc::online::v3d::isa::AddAlu {
                        op: AddOp::Fadd,
                        a: w.mul.a,
                        b: w.mul.b,
                        waddr: w.mul.waddr,
                        magic: w.mul.magic,
                        ..w.add
                    };
                }),
                "two writes to one register in one instruction (ours)"
            );
            assert_eq!(
                broke(&|b| {
                    let t = first_thrsw;
                    alu_mut(b, t + 1).sig.thrsw = false;
                    alu_mut(b, t + 2).sig.thrsw = true;
                }),
                "THRSW too close to another THRSW."
            );
        });
    }

    #[test]
    fn what_this_cut_does_not_compile_is_refused_by_name() {
        let refused = |vs: &str, fs: &str, prim: Prim| -> V3dError {
            let mut out = None;
            with_program(vs, fs, prim, |p| out = Some(p.map(|_| ()).unwrap_err()));
            out.unwrap()
        };
        let fs = |body: &str| std::format!("precision mediump float; varying vec4 v_color; void main() {{ {body} }}");
        /* A counted loop's body may not write an output yet (prepare.rs
         * counted_loop), named. */
        assert_eq!(
            refused(
                PASSTHROUGH_VS,
                &fs("vec4 c = v_color; for (float f = 0.0; f < 2.0; f += 1.0) { if (c.x > 0.5) break; \
                     gl_FragColor = c; c = c * v_color; }"),
                Prim::Triangles
            ),
            V3dError::NotYet("an output written inside a loop that can leave on data")
        );
        /* A loop that can leave on data is a counted loop: one back edge. */
        with_program(
            PASSTHROUGH_VS,
            &fs("vec4 c = v_color; for (float f = 0.0; f < 2.0; f += 1.0) { if (c.x > 0.5) break; c = c * v_color; } \
                 gl_FragColor = c;"),
            Prim::Triangles,
            |p| {
                let p = p.expect("a counted loop");
                check_all(p);
                assert_eq!(p.fs.code.back_edges.len(), 1);
            },
        );
        /* A loop that leaves only by its Appendix A test is expanded
         * (powervr/unroll.rs) and compiles. */
        with_program(
            PASSTHROUGH_VS,
            &fs("vec4 c = v_color; for (int i = 0; i < 3; i++) { c = c * v_color; } gl_FragColor = c;"),
            Prim::Triangles,
            |p| check_all(p.expect("a constant loop is expanded")),
        );
        with_program(
            PASSTHROUGH_VS,
            "precision mediump float; uniform vec4 u; void main() { gl_FragColor = u; }",
            Prim::Triangles,
            |p| {
                let p = p.expect("a plain uniform compiles");
                check_all(p);
                /* QUNIFORM_UNIFORM, data = the dword in the stage's block, in
                 * the order the words consume them. */
                let u: Vec<(u32, u32)> = p.fs.code.uniforms.iter().map(|u| (u.contents, u.data)).collect();
                assert_eq!(u, [(1, 0), (1, 1), (1, 2), (1, 3)]);
            },
        );
        assert_eq!(
            refused(PASSTHROUGH_VS, INTERP_FS, Prim::Points),
            V3dError::NotYet("the points variant of the fragment shader")
        );
        assert_eq!(
            refused(PASSTHROUGH_VS, INTERP_FS, Prim::Lines),
            V3dError::NotYet("the lines variant of the fragment shader")
        );
    }

    /// `p` in v3d_shc's JSON schema, key for key and in its layout (one
    /// line per array: tools/v3d/frame_check.c reads it with a small reader
    /// that expects that), plus `gpu.compiler`, so the driver's tools
    /// (tools/v3d/v3d_prog_header.py, frame_check --triangle) take our code as
    /// they take Mesa's.
    /// `fs_ir` gives the fragment half of `uniform_names`; the vertex half
    /// comes from `vs` with the attributes.
    fn v3d_shc_json(p: &V3dProgram<'_>, vs: &ir::Module<'_>, fs_ir: &ir::Module<'_>) -> String {
        use std::fmt::Write;
        let b = |v: bool| if v { "true" } else { "false" };
        let mut o = String::new();
        let common = |o: &mut String, threads: u32, single_seg: bool, tmu_dirty: bool| {
            write!(
                o,
                "\"threads\": {threads}, \"single_seg\": {}, \"spill_size\": 0, \"tmu_dirty_rcl\": {}, ",
                b(single_seg),
                b(tmu_dirty)
            )
            .unwrap();
        };
        let tail = |o: &mut String, c: &Code<'_>| {
            o.push_str("\n    \"uniforms\": [");
            for (i, u) in c.uniforms.iter().enumerate() {
                write!(
                    o,
                    "{}{{ \"contents\": {}, \"data\": \"0x{:08x}\" }}",
                    if i > 0 { ", " } else { "" },
                    u.contents,
                    u.data
                )
                .unwrap();
            }
            o.push_str("],\n    \"code\": [");
            for (i, w) in c.words.iter().enumerate() {
                write!(o, "{}\"0x{w:016x}\"", if i > 0 { ", " } else { "" }).unwrap();
            }
            o.push_str("] }");
        };
        write!(
            o,
            "{{\n  \"gpu\": {{ \"ver\": 42, \"vpm_size\": {}, \"vpm_size_source\": \"given\", \"prim\": \"triangles\", \
             \"per_vertex_point_size\": false, \"compiler\": \"yakogl-online\" }},\n",
            TARGET.vpm_size
        )
        .unwrap();
        /* v3d_shc prints Mesa's vec4 slot ("slot", "vec4s"); ours is the
         * word our QUNIFORM_UNIFORM names (select.rs), the IR's dword_off,
         * so the record carries "word" and "words" instead, and a reader
         * cannot mistake one for the other. Block members only: a sampler
         * has no word, and this back end selects no texture yet. */
        o.push_str("  \"uniform_names\": [");
        let mut n = 0;
        for (stage, m) in [("vert", vs), ("frag", fs_ir)] {
            for u in m.uniforms {
                assert!(u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK, "{stage} {}: a sampler", u.name);
                write!(
                    o,
                    "{}{{ \"stage\": \"{stage}\", \"name\": \"{}\", \"word\": {}, \"words\": {} }}",
                    if n > 0 { ", " } else { "" },
                    u.name,
                    u.dword_off,
                    u.dwords
                )
                .unwrap();
                n += 1;
            }
        }
        o.push_str("],\n  \"attributes\": [");
        /* v3d_shc: one entry per declaration, one record per column. */
        for (i, d) in vs.inputs.iter().filter(|d| d.kind == IoKind::Attribute).enumerate() {
            let recs: Vec<String> = (0..attribute_locations(d.gl_type))
                .map(|k| {
                    let r = p.attributes.iter().find(|e| e.0 == d.location + k).and_then(|e| e.1);
                    r.map_or(String::from("null"), |r| std::format!("{r}"))
                })
                .collect();
            let sep = if i > 0 { ", " } else { "" };
            write!(
                o,
                "{sep}{{ \"name\": \"{}\", \"location\": {}, \"records\": [{}] }}",
                d.name,
                d.location,
                recs.join(", ")
            )
            .unwrap();
        }
        o.push_str("],\n  \"fs\": { ");
        let f = &p.fs;
        common(&mut o, f.threads, f.single_seg, f.tmu_dirty_rcl);
        write!(o, "\"num_inputs\": {}, \"input_slots\": [", f.num_inputs).unwrap();
        for (i, (slot, comp)) in f.input_slots.iter().enumerate() {
            write!(o, "{}{{ \"slot\": {slot}, \"component\": {comp} }}", if i > 0 { ", " } else { "" }).unwrap();
        }
        write!(
            o,
            "], \"flat_shade_flags\": \"0x{:08x}\", \"noperspective_flags\": \"0x{:08x}\", \"centroid_flags\": \"0x{:08x}\", \
             \"writes_z\": {}, \"disable_ez\": {}, \"uses_center_w\": {}, \"lock_scoreboard_on_first_thrsw\": {}, \
             \"uses_discard\": {}, \"uses_implicit_point_line_varyings\": {}, \"uses_pid\": {}, \"force_per_sample_msaa\": {},",
            f.flat_shade_flags,
            f.noperspective_flags,
            f.centroid_flags,
            b(f.writes_z),
            b(f.disable_ez),
            b(f.uses_center_w),
            b(f.lock_scoreboard_on_first_thrsw),
            b(f.uses_discard),
            b(f.uses_implicit_point_line_varyings),
            b(f.uses_pid),
            b(f.force_per_sample_msaa)
        )
        .unwrap();
        tail(&mut o, &f.code);
        o.push_str(",\n");
        for (name, v, last) in [("vs", &p.vs, false), ("cs", &p.cs, true)] {
            write!(o, "  \"{name}\": {{ ").unwrap();
            common(&mut o, v.threads, v.single_seg, v.tmu_dirty_rcl);
            write!(
                o,
                "\"vpm_input_size\": {}, \"vpm_output_size\": {}, \"separate_segments\": {}, \"vcm_cache_size\": {}, \
                 \"uses_vid\": {}, \"uses_iid\": {}, \"uses_biid\": {}, \"writes_psiz\": {}, \"vattr_sizes\": [",
                v.vpm_input_size,
                v.vpm_output_size,
                b(v.separate_segments),
                v.vcm_cache_size,
                b(v.uses_vid),
                b(v.uses_iid),
                b(v.uses_biid),
                b(v.writes_psiz)
            )
            .unwrap();
            for (i, s) in v.vattr_sizes.iter().enumerate() {
                write!(o, "{}{s}", if i > 0 { ", " } else { "" }).unwrap();
            }
            o.push_str("],");
            tail(&mut o, &v.code);
            o.push_str(if last { "\n" } else { ",\n" });
        }
        o.push_str("}\n");
        o
    }

    /// Where the discard variant of the sample reads our program from.
    const OURS_TRI_DISCARD_JSON: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/driver/samples/zephyr/v3d_triangle/shaders/ours_tri_discard.json");
    static TRI_DISCARD_VS: &str = include_str!("../../../../tools/shaders/es100/tri_discard.vert");
    static TRI_DISCARD_FS: &str = include_str!("../../../../tools/shaders/es100/tri_discard.frag");

    /// Where the gl_FragCoord variant of the sample reads our program from.
    const OURS_TRI_FRAGCOORD_JSON: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/driver/samples/zephyr/v3d_triangle/shaders/ours_tri_fragcoord.json");
    static TRI_FRAGCOORD_VS: &str = include_str!("../../../../tools/shaders/es100/tri_fragcoord.vert");
    static TRI_FRAGCOORD_FS: &str = include_str!("../../../../tools/shaders/es100/tri_fragcoord.frag");

    /// Where the int variant of the sample reads our program from.
    const OURS_TRI_INT_JSON: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/driver/samples/zephyr/v3d_triangle/shaders/ours_tri_int.json");
    static TRI_INT_VS: &str = include_str!("../../../../tools/shaders/es100/tri_int.vert");
    static TRI_INT_FS: &str = include_str!("../../../../tools/shaders/es100/tri_int.frag");

    /// Where the if variant of the sample reads our program from.
    const OURS_TRI_IF_JSON: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/driver/samples/zephyr/v3d_triangle/shaders/ours_tri_if.json");
    static TRI_IF_VS: &str = include_str!("../../../../tools/shaders/es100/tri_if.vert");
    static TRI_IF_FS: &str = include_str!("../../../../tools/shaders/es100/tri_if.frag");

    /// Where the uniform variant of the sample reads our program from.
    const OURS_UTRI_JSON: &str =
        concat!(env!("CARGO_MANIFEST_DIR"), "/driver/samples/zephyr/v3d_triangle/shaders/ours_utri.json");

    /// Where the triangle sample reads our program from.
    const OURS_JSON: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/driver/samples/zephyr/v3d_triangle/shaders/ours_passthrough_interp_color.json"
    );

    /// `path` holds what this chain compiles `vs` + `fs` to now; with
    /// V3D_WRITE_OURS_JSON set it is rewritten first.
    fn sample_json_is_current(vs: &str, fs: &str, path: &str) {
        let mut json = None;
        store::with_scratch(|a| {
            let vs = a.alloc_str(vs).unwrap();
            let fs = a.alloc_str(fs).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vs).map_err(|e| e.msg).unwrap();
            let f = lower::compile_to_ir(a, Stage::Fragment, fs).map_err(|e| e.msg).unwrap();
            let p = compile_program(&v.module, &f.module, Prim::Triangles, TARGET, a).unwrap();
            check_all(&p);
            json = Some(v3d_shc_json(&p, &v.module, &f.module));
        })
        .unwrap();
        let json = json.unwrap();
        if std::env::var_os("V3D_WRITE_OURS_JSON").is_some() {
            std::fs::write(path, &json).unwrap();
            std::println!("wrote {path} ({} bytes)", json.len());
        }
        let on_disk = std::fs::read_to_string(path).unwrap_or_default();
        assert!(on_disk == json, "{path} is not this compiler's output; regenerate it (see this test)");
    }

    #[test]
    fn the_triangle_samples_json_is_this_compilers_output() {
        /* The JSON driver/samples/zephyr/v3d_triangle embeds with
         * -DTRI_JSON=<this file>: passthrough + interp_color (the ES 1.00
         * twins in tools/shaders/gl_test_es100, which Mesa's JSON was made
         * from as 450 sources) compiled by this chain for the board's VPM.
         * It must be what the chain produces now, or a board run would test
         * a compiler that no longer exists. Regenerate with
         *   V3D_WRITE_OURS_JSON=1 cargo test --no-default-features \
         *     --features host-test,gpu-powervr,shaderc-online --lib samples_json */
        sample_json_is_current(PASSTHROUGH_VS, INTERP_FS, OURS_JSON);
    }

    #[test]
    fn the_if_samples_json_is_this_compilers_output() {
        /* v3d_triangle's if variant (-DTRI_IF=ON): es100/tri_if, an if on
         * v_color.r whose arms each write a lane the other does not. The same
         * rule and regeneration as the triangle's. The FS must hold the
         * converted if: one flag push and a conditional move per arm's lane. */
        sample_json_is_current(TRI_IF_VS, TRI_IF_FS, OURS_TRI_IF_JSON);
        with_program(TRI_IF_VS, TRI_IF_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
            assert!(n("fcmp") >= 1 && n("mov.if") >= 2, "{ops:#?}");
        });
    }

    #[test]
    fn the_discard_samples_json_is_this_compilers_output() {
        /* v3d_triangle's discard variant (-DTRI_DISCARD=ON): es100/tri_discard.
         * The same rule and regeneration as the triangle's; the FS must hold
         * one conditional setmsf and report uses_discard. */
        sample_json_is_current(TRI_DISCARD_VS, TRI_DISCARD_FS, OURS_TRI_DISCARD_JSON);
        with_program(TRI_DISCARD_VS, TRI_DISCARD_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            assert_eq!(ops.iter().filter(|o| o.contains("setmsf.if")).count(), 1, "{ops:#?}");
            assert!(p.fs.uses_discard);
        });
    }

    #[test]
    fn the_fragcoord_samples_json_is_this_compilers_output() {
        /* v3d_triangle's gl_FragCoord variant (-DTRI_FRAGCOORD=ON):
         * es100/tri_fragcoord, the window transform visible in the picture.
         * The same rule and regeneration as the triangle's. The FS must read
         * FXCD and FYCD once each and ask for the two transform words. */
        sample_json_is_current(TRI_FRAGCOORD_VS, TRI_FRAGCOORD_FS, OURS_TRI_FRAGCOORD_JSON);
        with_program(TRI_FRAGCOORD_VS, TRI_FRAGCOORD_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
            assert_eq!((n("fxcd"), n("fycd")), (1, 1), "{ops:#?}");
            let codes: Vec<u32> = p.fs.code.uniforms.iter().map(|u| u.contents).collect();
            assert!(
                codes.contains(&QUNIFORM_FB_WPOS_Y_SCALE) && codes.contains(&QUNIFORM_FB_WPOS_Y_OFFSET),
                "{codes:?}"
            );
        });
    }

    #[test]
    fn the_int_samples_json_is_this_compilers_output() {
        /* v3d_triangle's int variant (-DTRI_INT=ON): es100/tri_int, the int
         * and bool operations in a picture. The same rule and regeneration as
         * the triangle's. The FS must hold each operation it is there for. */
        sample_json_is_current(TRI_INT_VS, TRI_INT_FS, OURS_TRI_INT_JSON);
        with_program(TRI_INT_VS, TRI_INT_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let n = |s: &str| ops.iter().map(|o| o.matches(s).count()).sum::<usize>();
            for want in ["ftoiz", "multop", "umul24", "neg", "itof", "xor.pushz", "fmov.pushz", "fmov.pushn", "mov.if"]
            {
                assert!(n(want) >= 1, "{want} is missing from the int variant's FS: {ops:#?}");
            }
        });
    }

    #[test]
    fn the_uniform_samples_json_is_this_compilers_output() {
        /* v3d_triangle's uniform variant (-DTRI_UNIFORMS=ON): es100/utri,
         * a uniform in the VS (and so the CS) and two in the FS. The same
         * rule and regeneration as the triangle's. */
        sample_json_is_current(UTRI_VS, UTRI_FS, OURS_UTRI_JSON);
        let json = std::fs::read_to_string(OURS_UTRI_JSON).unwrap();
        /* the words main.c fills, by name: pinned so a layout change is seen */
        for rec in [
            r#"{ "stage": "vert", "name": "u_offset", "word": 0, "words": 4 }"#,
            r#"{ "stage": "frag", "name": "u_scale", "word": 0, "words": 4 }"#,
            r#"{ "stage": "frag", "name": "u_bias", "word": 4, "words": 4 }"#,
        ] {
            assert!(json.contains(rec), "{rec} not in {json}");
        }
    }

    static MATATTR2_VS: &str = include_str!("../../../../tools/shaders/es100/matattr2.vert");

    #[test]
    fn a_matrix_attribute_has_one_entry_and_one_record_per_column() {
        /* v3d-isa's three-way check found `attributes` one entry short for
         * a mat2 at locations 1 and 2 (the driver needs a record for each
         * location). a_pos reads .xy, each a_xf column two lanes. */
        with_program(MATATTR2_VS, INTERP_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert_eq!(p.attributes, [(0, Some(0)), (1, Some(1)), (2, Some(2))]);
            assert_eq!(p.vs.vattr_sizes[..4], [2, 2, 2, 0]);
        });
        store::with_scratch(|a| {
            let vs = a.alloc_str(MATATTR2_VS).unwrap();
            let fs = a.alloc_str(INTERP_FS).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vs).map_err(|e| e.msg).unwrap();
            let f = lower::compile_to_ir(a, Stage::Fragment, fs).map_err(|e| e.msg).unwrap();
            let p = compile_program(&v.module, &f.module, Prim::Triangles, TARGET, a).unwrap();
            let json = v3d_shc_json(&p, &v.module, &f.module);
            assert!(json.contains(r#"{ "name": "a_xf", "location": 1, "records": [1, 2] }"#), "{json}");
        })
        .unwrap();
    }

    #[test]
    fn a_local_lane_no_store_reached_is_refused_only_when_it_is_used() {
        let fs = |body: &str| std::format!("precision mediump float; varying vec4 v_color; void main() {{ {body} }}");
        /* c.w is never written and never used: nothing to refuse. */
        with_program(
            PASSTHROUGH_VS,
            &fs("vec4 c; c.xyz = v_color.xyz; gl_FragColor = vec4(c.xyz, 1.0);"),
            Prim::Triangles,
            |p| {
                check_all(p.expect("a dead undefined lane compiles"));
            },
        );
        /* c.w reaches gl_FragColor as it is: an output lane the shader never
         * gave a value, which GLSL leaves undefined and which is written as
         * 0 (prepare.rs, the output stores; Mesa stores 0 to a varying
         * nothing wrote). The same shape is how the front end stores a
         * partial output write (`gl_FragColor.xy = ...`, `tt.x = t`). */
        with_program(PASSTHROUGH_VS, &fs("vec4 c; c.xyz = v_color.xyz; gl_FragColor = c;"), Prim::Triangles, |p| {
            check_all(p.expect("an output lane never given a value compiles"));
        });
        /* c.w used in arithmetic: the undefined value feeds a computation,
         * and it is refused by name rather than given a value (a front-end
         * defect that made such a value would otherwise draw silently). */
        with_program(
            PASSTHROUGH_VS,
            &fs("vec4 c; c.xyz = v_color.xyz; gl_FragColor = c * 2.0;"),
            Prim::Triangles,
            |p| {
                assert_eq!(p.map(|_| ()).unwrap_err(), V3dError::NotYet(super::super::prepare::UNDEFINED_READ));
            },
        );
    }

    /// The corpus this chain is measured on: every `real` program pair of
    /// tests/fixtures/v3d_mesa_programs.txt (tools/v3d/sim_fixture.py), the
    /// pairs Mesa's v3d_shc was run on. Paths are under tools/shaders.
    static MESA_PAIRS: &str = include_str!("../../../../tests/fixtures/v3d_mesa_programs.txt");

    /// Pinned: the pairs the chain compiles. Raise it when a step compiles
    /// more; a drop is a regression. A build that reports ES 2.0 refuses the
    /// corpus's GLSL ES 3.00 sources in the front end (pp.rs out_of_scope), so
    /// it compiles fewer pairs, by design; each build has its own pin.
    /* 172 -> 176 with gl_VertexID (2026-09-26): es300/vid.vert with vcolor,
     * dynidx, facing and uniloop. Only vcolor reads the varying the ID
     * feeds, so only that pair's vertex shader reads the ID; in the other
     * three it is dead. Four more that read it stop at gl_InstanceID (5 -> 9).
     * 176 -> 185 with gl_InstanceID (2026-09-26): those nine pairs.
     * 185 -> 194 with flat varyings (2026-09-26): es300/flatlast and eight
     * Qt Quick 3D particle pairs; eight more flat ones stop at
     * gl_FragData[n > 0].
     * 159 / 194 -> 163 / 198 with derivatives (2026-09-26): the four Qt
     * wireframe pairs (ES 1.00, so both builds); grid_300 stops at
     * gl_FragDepth.
     * 198 -> 216 with multiple render targets (2026-09-26): the 18 pairs
     * that write gl_FragData[n > 0] / an output at location n > 0 (ES 3.00
     * only, so report-es2 keeps 163).
     * 216 -> 217 with textureSize of a 2D sampler (2026-09-26):
     * qt3d/lightmapdilate_300 (ES 3.00 only). */
    const COMPILED: usize = if cfg!(feature = "report-es2") { 163 } else { 217 };

    #[test]
    fn corpus_coverage_is_pinned() {
        use std::collections::BTreeMap;
        let pairs: Vec<(&str, &str)> = MESA_PAIRS
            .lines()
            .filter_map(|l| l.strip_prefix("real "))
            .map(|l| {
                let mut it = l.split_whitespace();
                (it.next().unwrap(), it.next().unwrap())
            })
            .collect();
        let (mut compiled, mut front_end, mut other) = (0usize, 0usize, Vec::new());
        let mut not_yet: BTreeMap<&str, usize> = BTreeMap::new();
        /* The stages that fall back to 2 threads, by pair and stage. */
        let mut two_threads: Vec<String> = Vec::new();
        /* Fragment shaders with late tests and the pass-through Z write (a
         * discard), and those that write it because they read Z. */
        let (mut late_z, mut reads_z_only, mut no_colour) = (0usize, 0usize, 0usize);
        for &(vp, fp) in &pairs {
            let read =
                |p: &str| std::fs::read_to_string(std::format!("{}/tools/shaders/{p}", env!("CARGO_MANIFEST_DIR")));
            let (Ok(vt), Ok(ft)) = (read(vp), read(fp)) else {
                other.push(std::format!("{vp} + {fp}: source missing"));
                continue;
            };
            store::with_scratch(|a| {
                let vs = a.alloc_str(&vt).unwrap();
                let fs = a.alloc_str(&ft).unwrap();
                let (Ok(v), Ok(f)) =
                    (lower::compile_to_ir(a, Stage::Vertex, vs), lower::compile_to_ir(a, Stage::Fragment, fs))
                else {
                    front_end += 1;
                    return;
                };
                match compile_program(&v.module, &f.module, Prim::Triangles, TARGET, a) {
                    Ok(p) => {
                        check_all(&p);
                        compiled += 1;
                        /* a colour write is a VFPACK to the TLB */
                        let colour = p.fs.code.instrs.iter().any(|i| text(i).contains("vfpack tlb"));
                        no_colour += usize::from(!colour);
                        late_z += usize::from(colour && p.fs.uses_discard && p.fs.writes_z && p.fs.disable_ez);
                        reads_z_only += usize::from(!p.fs.uses_discard && p.fs.writes_z);
                        for (stage, t) in [("fs", p.fs.threads), ("vs", p.vs.threads), ("cs", p.cs.threads)] {
                            if t == 2 {
                                two_threads.push(std::format!("{vp} + {fp} {stage}"));
                            }
                        }
                    }
                    Err(V3dError::NotYet(what)) => *not_yet.entry(what).or_default() += 1,
                    Err(e) => other.push(std::format!("{vp} + {fp}: {e:?}")),
                }
            })
            .unwrap();
        }
        let refused: usize = not_yet.values().sum();
        std::println!(
            "v3d chain on {} corpus pairs: {compiled} compiled, {front_end} refused by the front end, {refused} NotYet, {} other",
            pairs.len(),
            other.len()
        );
        for (what, n) in &not_yet {
            std::println!("  NotYet {n:3} x {what}");
        }
        std::println!("  {} stage(s) at 2 threads: {two_threads:#?}", two_threads.len());
        std::println!(
            "  {late_z} discarding FS with late tests and the Z write, {reads_z_only} writing Z because they read it, \
             {no_colour} writing no colour (the Z write alone)"
        );
        /* The population, the zero and the pin, together. */
        assert_eq!(pairs.len(), 287, "the fixture's pair count changed: re-derive the pin");
        assert_eq!(compiled + front_end + refused + other.len(), pairs.len());
        assert!(other.is_empty(), "refusals that are not NotYet: {other:#?}");
        assert_eq!(compiled, COMPILED, "compiled pairs changed: raise the pin if this is progress");
        /* Measured 2026-09-24, both builds: every discarding FS of the corpus
         * (femtovg's 14 non-`nodiscard` shaders among them) has late tests
         * and the Z write; none reads Z (the known positive for that count
         * is a_shader_that_reads_z_writes_it); Qt Quick 3D's depth pre-pass
         * shaders write no colour (qtappshaders_0 and its two twins, the ES
         * 3.00 one in the ES 3.00 build only). */
        assert_eq!(
            (late_z, reads_z_only, no_colour),
            /* 16 read Z since multiple render targets compile: Qt Quick 3D's
             * weighted-blended OIT particle passes (qtparticleshaders_3, 7,
             * 27, 29, 34, 35, 36, 45, 53, 56, 60, 61, 64, 70, 74, 79: `float
             * z = abs(gl_FragCoord.z)`, two outputs), GLSL ES 3.00 only */
            (19, if cfg!(feature = "report-es2") { 0 } else { 16 }, if cfg!(feature = "report-es2") { 2 } else { 3 }),
            "the fragment shaders with the Z write changed"
        );
        /* Measured 2026-09-24: 10 stages. Qt's imageparticle_colored (vertex
         * and coordinate shaders) and imageparticle_sprite and
         * imageparticle_tabled (coordinate shaders), plain and batchable, and
         * es100/varyings15 (its vertex AND fragment shader: 60 varying
         * components). 15 before each uniform and constant came to be loaded
         * at its first read (select.rs Sel::val); tabled's two since a lane
         * stored as never written stays unwritten (prepare.rs, the varying
         * store). check_all has asserted each one needs it. 9 since float
         * operations on constants fold (prepare.rs fold_float): varyings15's
         * vertex shader fits 4 threads now; its fragment shader does not.
         * 19 since gl_InstanceID compiles (2026-09-26): the nine earlier ones
         * unchanged, and ten stages of the Qt Quick 3D particle shaders that
         * compile only now (qtparticleshaders_10, _26, _51, _86 vertex and
         * coordinate shaders, _12 and _82 vertex shaders), GLSL ES 3.00, so
         * a report-es2 build keeps the nine. 29 with flat varyings: ten
         * stages of the flat particle shaders (_21 _38 _73 _78 vertex and
         * coordinate, _43 and _90 vertex). 49 with multiple render
         * targets: twenty stages of the 18 MRT pairs (the OIT particle
         * passes, clear_mrt_300, lightmapuvraster_300), counted by name.
         * 50 with common-subexpression reuse (longer live ranges): Qt's
         * imageparticle_sprite (both) and imageparticle_tabled vertex shaders
         * go to 2 threads, imageparticle_colored's two coordinate shaders
         * come back to 4 (GLSL ES 1.00, so 9 -> 10 in report-es2 too); no
         * femtovg stage moves. 49 (report-es2 9) when abs() folds into
         * its readers also from a select's result (select.rs fold_unpacks):
         * imageparticle_tabled's vertex shader, with fewer live values,
         * comes back to 4 threads. */
        assert_eq!(
            two_threads.len(),
            if cfg!(feature = "report-es2") { 9 } else { 49 },
            "the stages at 2 threads changed: {two_threads:#?}"
        );
    }

    /// The words of every corpus program this compiler compiles, pinned as
    /// one FNV-1a digest over (pair, stage, word count, words) in fixture
    /// order. Pinned on 2026-10-01 with the hand-transcribed QPU encoder, the
    /// last commit before the encoder became Mesa's `v3d_qpu_instr_pack`
    /// through FFI (docs/source-form.md, F1): the digest staying the same is
    /// what shows the switch changed no word the compiler emits. A change of
    /// the compiler that changes code re-derives it, deliberately.
    #[test]
    fn compiled_corpus_words_are_pinned() {
        let pairs: Vec<(&str, &str)> = MESA_PAIRS
            .lines()
            .filter_map(|l| l.strip_prefix("real "))
            .map(|l| {
                let mut it = l.split_whitespace();
                (it.next().unwrap(), it.next().unwrap())
            })
            .collect();
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        let mut eat = |bytes: &[u8]| {
            for &b in bytes {
                h ^= b as u64;
                h = h.wrapping_mul(0x0000_0100_0000_01b3);
            }
        };
        let (mut programs, mut words) = (0usize, 0usize);
        for &(vp, fp) in &pairs {
            let read =
                |p: &str| std::fs::read_to_string(std::format!("{}/tools/shaders/{p}", env!("CARGO_MANIFEST_DIR")));
            let (Ok(vt), Ok(ft)) = (read(vp), read(fp)) else { continue };
            store::with_scratch(|a| {
                let vs = a.alloc_str(&vt).unwrap();
                let fs = a.alloc_str(&ft).unwrap();
                let (Ok(v), Ok(f)) =
                    (lower::compile_to_ir(a, Stage::Vertex, vs), lower::compile_to_ir(a, Stage::Fragment, fs))
                else {
                    return;
                };
                let Ok(p) = compile_program(&v.module, &f.module, Prim::Triangles, TARGET, a) else { return };
                programs += 1;
                eat(vp.as_bytes());
                eat(fp.as_bytes());
                for (stage, c) in [("fs", &p.fs.code), ("vs", &p.vs.code), ("cs", &p.cs.code)] {
                    eat(stage.as_bytes());
                    eat(&(c.words.len() as u64).to_le_bytes());
                    for w in c.words {
                        eat(&w.to_le_bytes());
                    }
                    words += c.words.len();
                }
            })
            .unwrap();
        }
        std::println!("v3d compiled corpus: {programs} programs, {words} words, digest {h:#018x}");
        assert_eq!(programs, COMPILED);
        let want: (usize, u64) = if cfg!(feature = "report-es2") {
            (27_881, 0x4b54_b240_51b1_3721)
        } else {
            (63_930, 0x3c0a_e950_d0e7_225b)
        };
        assert_eq!((words, h), want, "the words the compiler emits changed");
    }

    #[test]
    fn fneg_fabs_fmin_fmax() {
        /* Each of the four once, and abs() of a constant (a small immediate
         * cannot take the ABS unpack, so it is moved to a register first). */
        let fs = "precision mediump float; varying vec4 v_color; void main() { \
                  gl_FragColor = vec4(-v_color.x, abs(v_color.y), min(v_color.z, 0.5), max(v_color.w, abs(-0.25))); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let has = |s: &str| ops.iter().any(|o| o.contains(s));
            assert!(has("xor") && has("fmin") && has("fmax") && has(".abs"), "{ops:#?}");
            /* fneg's sign bit is a CONSTANT uniform */
            assert!(p.fs.code.uniforms.contains(&Uniform::constant(0x8000_0000)));
            for o in &ops {
                std::println!("{o}");
            }
        });
    }

    #[test]
    fn comparisons_and_bcsel() {
        let fs = "precision mediump float; varying vec4 v_color; void main() { vec4 v = v_color; \
                  gl_FragColor = vec4(v.x < v.y ? v.z : v.w, v.x <= 0.5 ? 1.0 : 0.25, \
                  v.y == v.z ? 0.5 : 0.75, v.z != 0.0 ? v.x : v.y); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
            assert_eq!(n("fcmp"), 4, "one flag push per comparison");
            assert!(n(".pushn") + n(".pushc") + n(".pushz") >= 4);
            assert_eq!(n("mov.ifa") + n("mov.ifna"), 4, "one conditional write per select");
        });
    }

    static UTRI_VS: &str = include_str!("../../../../tools/shaders/es100/utri.vert");
    static UTRI_FS: &str = include_str!("../../../../tools/shaders/es100/utri.frag");

    #[test]
    fn the_uniform_sample_reads_its_storage_by_word() {
        with_program(UTRI_VS, UTRI_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            for (name, c) in [("vs", &p.vs.code), ("cs", &p.cs.code), ("fs", &p.fs.code)] {
                std::println!("{name} uniforms {:?}", c.uniforms);
                for o in c.instrs.iter().map(text) {
                    std::println!("  {o}");
                }
            }
        });
    }

    /// QUNIFORM_UNIFORM's `data` is the dword index into the stage's uniform
    /// block: `LoadUniform::dword_off` plus the lane, below `uniform_dwords`.
    /// The driver's write_uniforms (storage word `data`) and the GL core's
    /// storage layout (reflect.rs) both depend on it; a change of meaning
    /// must fail here.
    #[test]
    fn quniform_uniform_data_is_the_dword_index_into_the_block() {
        /* u_a takes dwords 0..3, u_b 4..7 (std430 vec4s, reflect.rs); the
         * shader reads u_b.z and u_b.x, in that order. */
        let fs = "precision mediump float; uniform vec4 u_a; uniform vec4 u_b; \
                  void main() { gl_FragColor = vec4(u_b.z, u_b.x, 0.0, 1.0); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let reads: Vec<u32> =
                p.fs.code
                    .uniforms
                    .iter()
                    .filter(|u| u.contents == super::super::select::QUNIFORM_UNIFORM)
                    .map(|u| u.data)
                    .collect();
            /* the words, not their order (which is the loads' order) */
            let mut reads = reads;
            reads.sort_unstable();
            assert_eq!(reads, [4, 6], "u_b.x is dword 4 + 0, u_b.z dword 4 + 2");
        });
        /* The same load with the block declared one vec4 long: u_b is past
         * it, and the compile is refused, never emitted. */
        store::with_scratch(|a| {
            let vs = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(PASSTHROUGH_VS).unwrap()).unwrap();
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(fs).unwrap()).map_err(|e| e.msg).unwrap();
            assert_eq!(f.module.uniform_dwords, 8, "the block as reflect.rs lays it out");
            let short = ir::Module { uniform_dwords: 4, ..f.module };
            let r = compile_program(&vs.module, &short, Prim::Triangles, TARGET, a).map(|_| ());
            assert_eq!(r, Err(V3dError::Prepare(PrepError::UniformOutOfBlock)));
            /* The bound is on the whole load, used lanes or not: u_b is one
             * vec4 load of dwords 4..7, so a block of 7 refuses it and the
             * real 8 (dword 7 the last, the bound < and not <=) takes it. */
            let edge = ir::Module { uniform_dwords: 7, ..f.module };
            let r = compile_program(&vs.module, &edge, Prim::Triangles, TARGET, a).map(|_| ());
            assert_eq!(r, Err(V3dError::Prepare(PrepError::UniformOutOfBlock)));
            assert!(compile_program(&vs.module, &f.module, Prim::Triangles, TARGET, a).is_ok());
        })
        .unwrap();
    }

    /// The zero-varying list of glLinkProgram (a varying the FS reads that
    /// the VS declares and never writes) goes through
    /// `lower::zero_varyings` on the FS before `compile_program`, as for the
    /// other GPUs (codegen.rs): the input is gone and every read is 0.
    #[test]
    fn a_declared_unwritten_varying_is_zeroed_before_the_chain() {
        let vs = "attribute vec4 a_pos; varying vec4 v_color; varying vec4 v_unused; \
                  void main() { gl_Position = a_pos; v_color = vec4(1.0); }";
        let fs = "precision mediump float; varying vec4 v_color; varying vec4 v_unused; \
                  void main() { gl_FragColor = v_color + v_unused; }";
        store::with_scratch(|a| {
            let v = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(vs).unwrap()).map_err(|e| e.msg).unwrap();
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(fs).unwrap()).map_err(|e| e.msg).unwrap();
            let before = f.module.inputs.len();
            let z = lower::zero_varyings(a, f, &["v_unused"]).map_err(|e| e.msg).unwrap();
            assert_eq!(z.module.inputs.len(), before - 1);
            let p = compile_program(&v.module, &z.module, Prim::Triangles, TARGET, a).expect("compiles");
            check_all(&p);
            assert_eq!(p.fs.num_inputs, 4, "v_color only");
        })
        .unwrap();
        /* A varying the VS does not declare at all is the link error, and
         * the chain refuses it by name rather than inventing a value. */
        let vs2 =
            "attribute vec4 a_pos; varying vec4 v_color; void main() { gl_Position = a_pos; v_color = vec4(1.0); }";
        with_program(vs2, fs, Prim::Triangles, |p| {
            assert_eq!(p.map(|_| ()), Err(V3dError::VaryingWithoutVertexOutput));
        });
    }

    /// Front-end report (the width table, ir.rs `Module::comps`): a
    /// built-in emitted through builtin_call.rs (`abs`, `max` of a vec2)
    /// leaves its width UNRECORDED (0), so `comps_of` answers the documented
    /// over-estimate 4, while `u + u` records 2. Why it cannot draw wrong on
    /// the V3D: every store bounds its lanes by the destination (a varying
    /// by its declaration's `components`, a local by `write_mask`), and a
    /// lane whose source lanes do not exist is left uncomputed (prepare.rs),
    /// so the two extra lanes are never read. It costs work, not pixels.
    fn builtin_widths(src: &str) -> Vec<(ir::AluOp, Option<u8>, u8)> {
        let mut out = Vec::new();
        store::with_scratch(|a| {
            let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(src).unwrap()).map_err(|e| e.msg).unwrap();
            let m = &f.module;
            for (i, op) in m.instrs.iter().enumerate() {
                if let ir::Op::Alu(k, _) = op {
                    out.push((*k, m.comps.get(i).copied(), m.comps_of(ir::ValueId(i as u32))));
                }
            }
        })
        .unwrap();
        out
    }

    #[test]
    #[ignore = "front-end gap: built-in widths unrecorded (comps 0 -> fallback 4); costs lanes, not pixels; see v3d-backend.md"]
    fn a_builtin_on_a_vec2_records_two_lanes() {
        let w = builtin_widths(
            "precision mediump float; uniform vec2 u; \
             void main() { vec2 s = abs(u); vec2 t = max(u, vec2(0.5)); vec2 r = u + u; \
             gl_FragColor = vec4(s + t + r, 0.0, 1.0); }",
        );
        std::println!("{w:?}");
        for (k, _, c) in &w {
            if matches!(k, ir::AluOp::Fabs | ir::AluOp::Fmax) {
                assert_eq!(
                    *c, 2,
                    "front-end gap: {k:?} of a vec2 leaves its width unrecorded (comps 0 -> fallback 4); \
                     costs lanes, not pixels; see v3d-backend.md, Open reports to the front end: {w:?}"
                );
            }
        }
    }

    #[test]
    fn ffma_dots_and_the_rounding_family() {
        /* dot2/3/4, fract, floor, ceil, and a*b+c where the front end fuses
         * it (mix() lowers through Ffma, builtin_call.rs). */
        let fs = "precision mediump float; varying vec4 v_color; void main() { vec4 v = v_color; \
                  gl_FragColor = vec4(dot(v.xy, v.zw) + dot(v.xyz, v.yzw), dot(v, v.wzyx), \
                  fract(v.x) + floor(v.y) + ceil(v.z), mix(v.x, v.y, v.w)); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            let n = |s: &str| ops.iter().map(|o| o.matches(s).count()).sum::<usize>();
            /* 2 + 3 + 4 products, and at least mix's multiply */
            assert!(n("fmul") > 2 + 3 + 4, "{ops:#?}");
            assert_eq!(n("ffloor"), 2, "fract's and floor's: {ops:#?}");
            assert_eq!(n("fceil"), 1, "{ops:#?}");
            assert!(n("fsub") >= 1, "fract is x - floor(x): {ops:#?}");
        });
    }

    #[test]
    fn a_select_result_read_by_the_next_add_waits_for_the_conditional_move() {
        /* The select's conditional move is on the mul unit; the fadd reading
         * its result is on the add unit, so the scheduler could put both in
         * one word, where the fadd reads the value from before the move
         * (sched.rs: a CondMov's destination is ready one word later). */
        let fs = "precision mediump float; varying vec4 v_color; void main() { vec4 v = v_color; \
                  float f = v.x < 0.5 ? v.y : v.z; gl_FragColor = vec4(f + v.w, f + 0.25, v.x, 1.0); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            /* No word holds both a conditional move and an add reading the
             * register it writes. */
            for i in p.fs.code.instrs.iter() {
                if let Instr::Alu(a) = i {
                    if a.flags.mc != crate::shaderc::online::v3d::isa::Cond::None && a.add.op != AddOp::Nop {
                        let m = crate::shaderc::online::v3d::isa::Mux::from_bits(a.mul.waddr as u32);
                        assert!(
                            !(a.mul.magic && a.mul.waddr < 6 && (a.add.a.mux == m || a.add.b.mux == m)),
                            "an add reads the conditional move's register in its own word: {ops:#?}"
                        );
                    }
                }
            }
        });
    }

    #[test]
    fn sign_ints_and_bool_conversions() {
        /* sign; int constants, add, negate, multiply, the float/int
         * conversions; the int equalities; bool to float/int. */
        let fs = "precision mediump float; varying vec4 v_color; void main() { vec4 v = v_color; \
                  int i = int(v.x * 8.0); int j = -i * 3 + 2; bool e = (j == i); bool n = (j != 5); \
                  gl_FragColor = vec4(sign(v.y - 0.5), float(j), float(e), float(n) + float(int(n))); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            let n = |s: &str| ops.iter().map(|o| o.matches(s).count()).sum::<usize>();
            assert!(n("multop") >= 1 && n("umul24") >= 1, "imul is the MULTOP/UMUL24 pair: {ops:#?}");
            assert!(n("ftoiz") >= 1 && n("itof") >= 1 && n("neg") >= 1, "{ops:#?}");
            assert!(n("fmov.pushz") >= 1 && n("fmov.pushn") >= 1, "sign's two pushes: {ops:#?}");
            assert!(n("xor.pushz") >= 2, "ieq and ine on XOR: {ops:#?}");
        });
    }

    /// No program this back end emits reaches memory through the TMU: its
    /// only TMU accesses are 2D look-ups of bound textures (see
    /// first_tmu_memory_access, run by check_all on every test and every
    /// compiled corpus pair). An indirectly indexed local reaches the back
    /// end as a select chain (the front end's) or is refused ("local array",
    /// "indexed local"), and nothing spills (`spill_size` 0; the allocator
    /// refuses instead), so an out-of-range index (undefined in GLSL ES 1.00
    /// sections 4.1.9 and 5.5) can give a wrong value but never an access
    /// outside the program. es100/matdyn indexes `arr[i]` and `p[i]` with a
    /// uniform, out of range for every u_i but 1.
    #[test]
    fn no_emitted_program_touches_memory_through_the_tmu() {
        static MATDYN_FS: &str = include_str!("../../../../tools/shaders/es100/matdyn.frag");
        with_program(PASSTHROUGH_VS, MATDYN_FS, Prim::Triangles, |p| {
            let p = p.expect("matdyn compiles (its indexed arrays are select chains)");
            check_all(p);
        });
        /* The instrument, both ways, on Mesa's code from the fixture: its
         * es100/textured FS (a texture2D) is a look-up and passes; its
         * es100/matdyn FS spills its dynamically indexed arrays to scratch
         * through the TMU's memory registers and must be found. */
        let mesa_fs = |key: &str| -> (Vec<Instr>, Vec<(u32, u32)>) {
            let block = MESA_PAIRS.split(&std::format!("\npair {key}\n")).nth(1).expect("the pair is in the fixture");
            let code = block.lines().find_map(|l| l.strip_prefix("fs code ")).expect("fs code");
            let unis = block.lines().find_map(|l| l.strip_prefix("fs uniforms ")).unwrap_or("");
            let instrs =
                code.split_whitespace().map(|h| decode(u64::from_str_radix(&h[2..], 16).unwrap()).unwrap()).collect();
            let contents = unis
                .split_whitespace()
                .map(|u| {
                    let mut it = u.split(':');
                    let c = it.next().unwrap().parse().unwrap();
                    let d = it.next().map_or(0, |d| u32::from_str_radix(d.trim_start_matches("0x"), 16).unwrap_or(0));
                    (c, d)
                })
                .collect();
            (instrs, contents)
        };
        let (i, u) = mesa_fs("es100/textured.vert es100/textured.frag");
        assert_eq!(first_tmu_memory_access(&i, &u), None, "a 2D look-up must pass");
        let (i, u) = mesa_fs("gl_test_es100/passthrough.vert es100/matdyn.frag");
        let found = first_tmu_memory_access(&i, &u);
        std::println!("known positive (Mesa's matdyn): {found:?}");
        assert!(found.is_some(), "the check misses a TMU memory access");
    }

    /// Slint's GL renderer is femtovg: its shaders (tools/shaders/femtovg,
    /// every fragment variant linked with main.vert) are the target list of
    /// this back end. Each one's outcome is pinned by name, so a step that
    /// unblocks one shows here and a regression cannot hide in a count.
    #[test]
    fn femtovg_outcomes_are_pinned() {
        use std::collections::BTreeMap;
        let dir = std::format!("{}/tools/shaders/femtovg", env!("CARGO_MANIFEST_DIR"));
        let vs_text = std::fs::read_to_string(std::format!("{dir}/main.vert")).unwrap();
        let mut frags: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .filter(|n| n.ends_with(".frag"))
            .collect();
        frags.sort();
        let mut got: BTreeMap<String, String> = BTreeMap::new();
        for f in &frags {
            let fs_text = std::fs::read_to_string(std::format!("{dir}/{f}")).unwrap();
            store::with_scratch(|a| {
                let vs = a.alloc_str(&vs_text).unwrap();
                let fs = a.alloc_str(&fs_text).unwrap();
                let (v, fr) =
                    match (lower::compile_to_ir(a, Stage::Vertex, vs), lower::compile_to_ir(a, Stage::Fragment, fs)) {
                        (Ok(v), Ok(fr)) => (v, fr),
                        (Err(e), _) | (_, Err(e)) => {
                            got.insert(f.clone(), std::format!("front end: {}", e.msg));
                            return;
                        }
                    };
                let outcome = match compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a) {
                    Ok(p) => {
                        check_all(&p);
                        "compiles".to_string()
                    }
                    Err(V3dError::NotYet(w)) => std::format!("NotYet {w}"),
                    Err(e) => std::format!("{e:?}"),
                };
                got.insert(f.clone(), outcome);
            })
            .unwrap();
        }
        for (f, o) in &got {
            std::println!("  {f:45} {o}");
        }
        assert_eq!(frags.len(), 35, "the femtovg fragment set changed: re-derive the pins");
        /* The first refusal per shader, measured 2026-09-24 (discard and 2D look-ups selected); a
         * discard variant's later needs show in its .nodiscard twin. When a
         * step changes a row, the row is re-derived by running. */
        let want: &[(&str, &str)] = &[
            ("fill_color.frag", "compiles"),
            ("fill_color.glyph.frag", "compiles"),
            ("fill_color.glyph.nodiscard.frag", "compiles"),
            ("fill_color.nodiscard.frag", "compiles"),
            ("fill_color.nodiscard.noscissor.frag", "compiles"),
            ("fill_gradient.frag", "compiles"),
            ("fill_gradient.glyph.frag", "compiles"),
            ("fill_gradient.glyph.nodiscard.frag", "compiles"),
            ("fill_gradient.nodiscard.frag", "compiles"),
            ("fill_gradient.nodiscard.noscissor.frag", "compiles"),
            ("fill_gradient_conic.frag", "compiles"),
            ("fill_gradient_conic.glyph.frag", "compiles"),
            ("fill_gradient_conic.glyph.nodiscard.frag", "compiles"),
            ("fill_gradient_conic.nodiscard.frag", "compiles"),
            ("fill_gradient_conic.nodiscard.noscissor.frag", "compiles"),
            ("fill_image.frag", "compiles"),
            ("fill_image.glyph.frag", "compiles"),
            ("fill_image.glyph.nodiscard.frag", "compiles"),
            ("fill_image.nodiscard.frag", "compiles"),
            ("fill_image.nodiscard.noscissor.frag", "compiles"),
            ("fill_image_gradient.frag", "compiles"),
            ("fill_image_gradient.glyph.frag", "compiles"),
            ("fill_image_gradient.glyph.nodiscard.frag", "compiles"),
            ("fill_image_gradient.nodiscard.frag", "compiles"),
            ("fill_image_gradient.nodiscard.noscissor.frag", "compiles"),
            ("fill_image_gradient_conic.frag", "compiles"),
            ("fill_image_gradient_conic.glyph.frag", "compiles"),
            ("fill_image_gradient_conic.glyph.nodiscard.frag", "compiles"),
            ("fill_image_gradient_conic.nodiscard.frag", "compiles"),
            ("fill_image_gradient_conic.nodiscard.noscissor.frag", "compiles"),
            ("filter_image.frag", "compiles"),
            ("filter_image.nodiscard.frag", "compiles"),
            ("stencil.frag", "compiles"),
            ("stencil.nodiscard.frag", "compiles"),
            ("texture_copy_unclipped.frag", "compiles"),
        ];
        let got: Vec<(&str, &str)> = got.iter().map(|(f, o)| (f.as_str(), o.as_str())).collect();
        assert_eq!(got, want, "a femtovg shader's outcome moved: re-derive its row by running");
    }

    #[test]
    fn a_discard_clears_the_multisample_flags_under_its_condition() {
        /* Mesa's form (nir_to_vir.c:3771-3792): the comparison's flag push,
         * then `setmsf.ifa -, 0`; and late tests with the pass-through Z
         * write (nir_to_vir.c:2071-2129, can_earlyz_with_discard false):
         * uses_discard, writes_z and disable_ez all true. */
        let fs = "precision mediump float; varying vec4 v_color; \
                  void main() { if (v_color.r > 0.5) discard; gl_FragColor = v_color; }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            let setmsf: Vec<&String> = ops.iter().filter(|o| o.contains("setmsf")).collect();
            assert_eq!(setmsf.len(), 1, "{ops:#?}");
            assert!(setmsf[0].contains("setmsf.if"), "conditional: {ops:#?}");
            assert!(ops.iter().any(|o| o.contains("fcmp.push")), "the comparison's own flags: {ops:#?}");
            assert!((p.fs.uses_discard, p.fs.writes_z, p.fs.disable_ez) == (true, true, true));
            /* one Z write, after the last thread-switch pair and before the
             * first colour write */
            assert_eq!(z_from_fep_writes("fs", &p.fs.code), 1);
            let z = ops.iter().position(|o| o.contains("tlbu") && o.contains("mov")).expect("the Z write");
            let colour = ops.iter().position(|o| o.contains("vfpack")).expect("a colour write");
            assert!(z < colour, "{ops:#?}");
        });
        /* An unconditional discard: every lane, no condition. */
        let fs = "precision mediump float; varying vec4 v_color; void main() { gl_FragColor = v_color; discard; }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            assert!(ops.iter().any(|o| o.starts_with("setmsf -, ")), "{ops:#?}");
        });
        /* No discard: no setmsf, and uses_discard false; early tests, no Z
         * write. */
        with_program(PASSTHROUGH_VS, INTERP_FS, Prim::Triangles, |p| {
            let p = p.unwrap();
            check_all(p);
            assert!(!p.fs.uses_discard);
            assert!(!p.fs.writes_z && !p.fs.disable_ez);
            assert!(!p.fs.code.instrs.iter().map(text).any(|o| o.contains("setmsf")));
        });
    }

    #[test]
    fn a_shader_that_writes_no_colour_writes_z_alone() {
        /* Legal GLSL (a depth pre-pass): Mesa writes the pass-through Z as
         * the program's one TLB write and flags the shader as discarding, so
         * it runs with late tests (nir_to_vir.c:2067-2078, 2113-2129). */
        let fs = "precision mediump float; varying vec4 v_color; void main() { float x = v_color.x; }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!((p.fs.uses_discard, p.fs.writes_z, p.fs.disable_ez) == (true, true, true));
            assert_eq!(z_from_fep_writes("fs", &p.fs.code), 1);
            assert!(!p.fs.code.instrs.iter().any(|i| text(i).contains("vfpack")), "no colour write");
        });
        /* A colour with some lanes never written: those lanes are 0, and the
         * colour is written (early tests, no Z write). */
        let fs = "precision mediump float; varying vec4 v_color; void main() { gl_FragColor.xy = v_color.xy; }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!((p.fs.uses_discard, p.fs.writes_z, p.fs.disable_ez) == (false, false, false));
            assert_eq!(p.fs.code.instrs.iter().filter(|i| text(i).contains("vfpack tlb")).count(), 2);
        });
    }

    #[test]
    fn gl_point_coord_reads_the_implicit_point_varyings_first() {
        /* Mesa nir_to_vir.c:4869-4873 before ntq_setup_fs_inputs: two ldvary
         * for the implicit point x and y before the declared varyings, y
         * through QUNIFORM_FB_PNTC_Y_SCALE/_OFFSET, and the program flagged
         * (the job builder refuses a non-points draw with it). */
        let fs = "precision mediump float; varying vec4 v_color; \
                  void main() { gl_FragColor = vec4(gl_PointCoord, v_color.x, 1.0); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!(p.fs.uses_implicit_point_line_varyings);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            /* the declared inputs (v_color's x) plus the two implicit ones */
            assert_eq!(ops.iter().filter(|o| o.contains("ldvary")).count(), p.fs.num_inputs as usize + 2, "{ops:#?}");
            use super::super::select::{QUNIFORM_FB_PNTC_Y_OFFSET, QUNIFORM_FB_PNTC_Y_SCALE};
            for c in [QUNIFORM_FB_PNTC_Y_SCALE, QUNIFORM_FB_PNTC_Y_OFFSET] {
                assert!(p.fs.code.uniforms.contains(&Uniform { contents: c, data: 0 }), "{c:#x}");
            }
        });
        /* no gl_PointCoord: no implicit varyings */
        with_program(PASSTHROUGH_VS, INTERP_FS, Prim::Triangles, |p| {
            let p = p.unwrap();
            assert!(!p.fs.uses_implicit_point_line_varyings);
            let ldvary = p.fs.code.instrs.iter().filter(|i| text(i).contains("ldvary")).count();
            assert_eq!(ldvary, p.fs.num_inputs as usize);
        });
    }

    #[test]
    fn a_shader_that_reads_z_writes_it() {
        /* The 4.2 spec, as nir_to_vir.c:2089-2095 quotes it: a shader that
         * reads Z (gl_FragCoord.z, the payload rf2) must have "Fragment
         * shader does Z writes" set, so it writes the pass-through Z; with
         * no discard its tests stay early (disable_ez false). */
        let fs = "precision mediump float; void main() { gl_FragColor = vec4(gl_FragCoord.z); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!((p.fs.uses_discard, p.fs.writes_z, p.fs.disable_ez) == (false, true, false));
            assert_eq!(z_from_fep_writes("fs", &p.fs.code), 1);
        });
        /* x and y only: no Z read, no Z write. */
        let fs = "precision mediump float; void main() { gl_FragColor = vec4(gl_FragCoord.xy, 0.0, 1.0); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!(!p.fs.writes_z && !p.fs.disable_ez);
        });
    }

    #[test]
    fn nested_counted_loops_pass_and_each_broken_loop_rule_is_refused() {
        /* Two loops with uniform bounds, one inside the other: two back
         * edges, the inner one's range inside the outer one's. */
        let fs = "precision mediump float; varying vec4 v_color; uniform int ui_two, ui_three; \
                  void main() { vec4 r = v_color; \
                  for (int i = 0; i < ui_two; i++) { for (int j = 0; j < ui_three; j++) r = r.yzwx; } \
                  gl_FragColor = r; }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            assert_eq!(c.back_edges.len(), 2, "two loops");
            let (inner, outer) = (c.back_edges[0], c.back_edges[1]);
            assert!(outer.1 < inner.1 && inner.0 < outer.0, "nested: {:?}", c.back_edges);
            /* the outer loop's uniform bound is a break branch to its exit
             * (prepare.rs SOp::LoopBreak); the inner loop's, inside another
             * counted loop, stays its flag */
            assert_eq!(c.fwd_branches.len(), 1, "the outer loop's break: {:?}", c.fwd_branches);
            assert_eq!(c.fwd_branches[0].1, outer.0 + 4, "to the outer loop's exit");
            assert!(c.fwd_branches[0].0 < inner.1, "before the inner loop");
            let rule = |prog: &[Instr], edges: &[(u32, u32)]| {
                branches_are_structured(prog, edges, c.fwd_branches).map_err(|e| e.rule)
            };
            let retarget = |prog: &mut [Instr], b: u32, head: u32| {
                let Instr::Branch(br) = &mut prog[b as usize] else { panic!("a back edge is a branch") };
                br.offset = ((head as i64 - (b as i64 + 4)) * 8) as u32;
            };
            assert_eq!(rule(c.instrs, c.back_edges), Ok(()));
            /* each rule, broken one way */
            assert_eq!(
                rule(c.instrs, &[outer]),
                Err("a branch that is neither a loop's back edge nor a forward branch")
            );
            let mut prog = c.instrs.to_vec();
            let Instr::Branch(b) = &mut prog[inner.0 as usize] else { panic!() };
            b.cond = BranchCond::Always;
            assert_eq!(rule(&prog, c.back_edges), Err("an unconditional back edge"));
            let mut prog = c.instrs.to_vec();
            let Instr::Branch(b) = &mut prog[inner.0 as usize] else { panic!() };
            b.ub = false;
            assert_eq!(rule(&prog, c.back_edges), Err("a back edge that is not REL with the uniform stream moved"));
            assert_eq!(
                rule(c.instrs, &[(inner.0, inner.1 + 1), outer]),
                Err("a back edge whose target is not its loop's head")
            );
            assert_eq!(
                rule(c.instrs, &[inner, outer, (inner.1, inner.1)]),
                Err("a recorded back edge that is not a branch")
            );
            /* Crossing: the outer branch retargeted to a head inside the
             * inner loop, so [inner head, inner edge] and [that head, outer
             * edge] overlap without nesting; every other rule still holds. */
            let mid = inner.1 + 1;
            assert!(mid < inner.0);
            let mut prog = c.instrs.to_vec();
            retarget(&mut prog, outer.0, mid);
            assert_eq!(rule(&prog, &[inner, (outer.0, mid)]), Err("two loops that cross"));
            /* and its control: the same retarget to a head before the inner
             * loop's still nests */
            let mut prog = c.instrs.to_vec();
            retarget(&mut prog, outer.0, inner.1 - 1);
            assert_eq!(rule(&prog, &[inner, (outer.0, inner.1 - 1)]), Ok(()));
        });
    }

    #[test]
    fn a_cube_look_up_is_mesas_sequence() {
        /* v3d_tex.c v3d_vir_emit_tex for a cube (:111-128, 137-151,
         * 371-378): P0, P1, the T and R writes (the direction's y and z),
         * then S (x) on TMUSCM, which retires it; no P2 without a level. */
        let fs = "precision mediump float; varying vec4 v_color; uniform samplerCube s; \
                  void main() { gl_FragColor = textureCube(s, v_color.xyz); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let at = |needle: &str| {
                ops.iter().position(|o| o.contains(needle)).unwrap_or_else(|| panic!("{needle}: {ops:#?}"))
            };
            let (t, r, s) = (at("tmut"), at("tmur"), at("tmuscm"));
            assert!(t < s && r < s, "T and R before the retiring S: {ops:#?}");
            assert!(!ops.iter().any(|o| o.contains("tmus,") || o.contains("tmuslod")), "{ops:#?}");
            let wrtmuc = ops.iter().filter(|o| o.contains("wrtmuc")).count();
            assert_eq!(wrtmuc, 2, "P0 and P1 only: {ops:#?}");
            assert!(!p.fs.code.uniforms.contains(&Uniform::constant(super::super::select::TMU_P2_CUBE_LOD)));
        });
        /* In the vertex shader the look-up is at level 0 (Lod(0.0)): the
         * level on TMUB, P2 with disable_autolod after P1, and still TMUSCM. */
        let vs = "attribute vec4 a_pos; attribute vec4 a_col; uniform samplerCube s; varying vec4 v_color; \
                  void main() { gl_Position = a_pos; v_color = textureCube(s, a_col.xyz); }";
        with_program(vs, INTERP_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.vs.code.instrs.iter().map(text).collect();
            assert!(ops.iter().any(|o| o.contains("tmub")), "{ops:#?}");
            assert!(ops.iter().any(|o| o.contains("tmuscm")), "{ops:#?}");
            assert!(!ops.iter().any(|o| o.contains("tmuslod")), "{ops:#?}");
            assert_eq!(ops.iter().filter(|o| o.contains("wrtmuc")).count(), 3, "P0, P1, P2: {ops:#?}");
            assert!(p.vs.code.uniforms.contains(&Uniform::constant(super::super::select::TMU_P2_CUBE_LOD)));
        });
    }

    /// Where each THRSW signal stands, and the first `ldtmu`.
    fn thrsw_shape(c: &Code<'_>) -> (Vec<usize>, Option<usize>) {
        let sig = |i: &Instr| match i {
            Instr::Alu(a) => Some(a.sig),
            Instr::Branch(_) => None,
        };
        let thrsw =
            c.instrs.iter().enumerate().filter(|(_, i)| sig(i).is_some_and(|s| s.thrsw)).map(|(n, _)| n).collect();
        let ldtmu = c.instrs.iter().position(|i| sig(i).is_some_and(|s| s.ldtmu));
        (thrsw, ldtmu)
    }

    /// The first `ip` in `from..to - 1` where `prog[ip]` and `prog[ip + 1]`
    /// are both ALU words: a place a last-thread-switch pair could stand.
    fn pair_site(prog: &[Instr], from: usize, to: usize) -> Option<usize> {
        (from..to.saturating_sub(1)).find(|&i| matches!((&prog[i], &prog[i + 1]), (Instr::Alu(_), Instr::Alu(_))))
    }

    /// `prog` with every THRSW signal cleared and a pair set at `at` and
    /// `at + 1`: the last-thread-switch pair moved there (a structural
    /// mutant for verify.rs branches_are_structured, never run).
    fn pair_moved(prog: &[Instr], at: usize) -> Vec<Instr> {
        let mut m = prog.to_vec();
        for (ip, i) in m.iter_mut().enumerate() {
            if let Instr::Alu(a) = i {
                a.sig.thrsw = ip == at || ip == at + 1;
            }
        }
        m
    }

    /// A vertex shader sampling twelve textures, past the eight every GPU
    /// used to report (GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, backend.rs
    /// GLES_VS_TEX_UNITS: 16 on the V3D): it compiles, passes check_all,
    /// and its P0 words name the twelve bindings 0..11 (the unit in bits
    /// 24..31, as the fragment stage's do). dEQP-GLES2 uniform_api needs up
    /// to 14 (records/pi4/record_SW28_U1_2c87c2e4.txt).
    #[test]
    fn a_vertex_shader_samples_twelve_textures() {
        let names: Vec<String> = (0..12).map(|k| std::format!("u_t{k}")).collect();
        let mut vs = std::string::String::from("attribute vec2 a_uv; varying vec4 v_color; ");
        for n in &names {
            vs += &std::format!("uniform sampler2D {n}; ");
        }
        vs += "void main() { vec4 c = vec4(0.0); ";
        for n in &names {
            vs += &std::format!("c += texture2DLod({n}, a_uv, 0.0); ");
        }
        vs += "v_color = c; gl_Position = vec4(a_uv, 0.0, 1.0); }";
        with_program(&vs, INTERP_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let mut units: Vec<u32> =
                p.vs.code.uniforms.iter().filter(|u| u.contents == 40).map(|u| u.data >> 24).collect();
            units.sort();
            units.dedup();
            assert_eq!(units, (0..12).collect::<Vec<u32>>(), "one P0 per binding");
        });
        /* the chain's tests also run in the PowerVR and GC7000 online builds,
         * whose GLES_VS_TEX_UNITS is 8 */
        #[cfg(feature = "gpu-v3d")]
        assert_eq!(crate::backend::GLES_VS_TEX_UNITS, 16, "the V3D's vertex texture units");
    }

    #[test]
    fn a_vertex_look_up_switches_threads_once() {
        /* Mesa's vertex shader switches threads once: the look-up's own
         * THRSW becomes the last-thread-switch pair (nir_to_vir.c:5203-5207,
         * vir_restore_last_thrsw), so the code has the pair, then the
         * results, then only the program end. gl_test 126's VS, whose
         * earlier form (a switch for the look-up and a second pair before
         * the VPM writes) hung the render stage on the Pi 4. */
        let vs = "attribute vec2 a_uv; uniform sampler2D u_vtex; uniform float u_lod; varying vec4 v_color; \
                  void main() { v_color = texture2DLod(u_vtex, a_uv, u_lod); \
                  gl_Position = vec4(a_uv * 2.0 - 1.0 + vec2(1.0 / 64.0), 0.0, 1.0); gl_PointSize = 1.0; }";
        with_program(vs, INTERP_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let (thrsw, ldtmu) = thrsw_shape(&p.vs.code);
            let n = p.vs.code.instrs.len();
            assert_eq!(thrsw.len(), 3, "the pair and the end: {thrsw:?}");
            assert_eq!(thrsw[1], thrsw[0] + 1, "the pair is two in a row: {thrsw:?}");
            /* the switch takes effect after the first THRSW's two delay slots */
            assert!(ldtmu.is_some_and(|l| l > thrsw[0] + 2), "the results after the switch: {thrsw:?} {ldtmu:?}");
            assert_eq!(thrsw[2], n - 3, "then only the program end: {thrsw:?} of {n}");
            assert!(!p.vs.single_seg);
        });
        /* A look-up inside a counted loop switches on every pass, so its
         * switch cannot be the last one (last_thrsw_at_top_level,
         * nir_to_vir.c:5007): a new pair follows the loop. */
        let vs = "attribute vec2 a_uv; uniform sampler2D u_vtex; uniform int u_n; varying vec4 v_color; \
                  void main() { vec4 c = vec4(0.0); \
                  for (int i = 0; i < 64; i++) { if (i >= u_n) break; c += texture2DLod(u_vtex, a_uv + vec2(float(i)), 0.0); } \
                  v_color = c; gl_Position = vec4(a_uv, 0.0, 1.0); }";
        with_program(vs, INTERP_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!(!p.vs.code.back_edges.is_empty(), "the loop is kept as a loop");
            let (thrsw, _) = thrsw_shape(&p.vs.code);
            let (b, _) = p.vs.code.back_edges[p.vs.code.back_edges.len() - 1];
            let pair: Vec<usize> = thrsw.windows(2).filter(|w| w[1] == w[0] + 1).map(|w| w[0]).collect();
            assert_eq!(pair.len(), 1, "one last-thread-switch pair: {thrsw:?}");
            assert!(pair[0] > b as usize, "the pair after the loop's back edge {b}: {thrsw:?}");
        });
    }

    /// V3D 4.2 makes one peripheral access per instruction, bar two pairs
    /// (qpu_compatible_peripheral_access, qpu_schedule.c:895-930). The first
    /// `wrtmuc` of a look-up used to merge into whatever the word held: an
    /// `rsqrt` in dEQP random all_features.fragment.1's shader, an
    /// `ldvpmv_in` in gl_test 126's and struct.uniform
    /// sampler_in_array_function_arg's vertex shaders. Each of them faulted
    /// on the Pi 4 (TMU fetch from a wild address, MMU fault L2T), while
    /// every look-up whose `wrtmuc` shared a word only with ALU work passed
    /// (es100/textured, random.texture.vertex.5, gl_test 131's literal and
    /// centre programs). `check_all` runs `verify`, which now refuses the
    /// pairs; this pins the three programs by name.
    ///
    /// Revert check: drop the `peripherals_compatible` refusal at the end of
    /// sched.rs `merge` and `check_all` fails on all three.
    #[test]
    fn a_look_up_config_word_shares_no_word_with_another_peripheral() {
        static RSG_F1_VS: &str = "attribute vec4 dEQP_Position;\nvoid main()\n{\n\tgl_Position = dEQP_Position;\n}\n";
        static RSG_F1_FS: &str = "precision mediump float;\n\
            const int a = (ivec2(true, 4).t - 0 * 1) * (ivec4(0.75, -4.5, 1, true).z * int(1));\n\
            int b = ((ivec4(-1.25, true, true, true))).sp.s;\n\
            const int c = 1;\nconst int d = 0;\n\
            int e = int(-26.5 + float(7.5) - log2(16.0) - float(float(d) - 4.0));\n\
            uniform mediump int f;\n\
            int g = (10) - (int(log2(float(32))));\n\
            const float h = ((vec4(-3.5, false, 6, -10.0)) + vec4(bool(false), int(-1), int(true), int(4.25))).g;\n\
            const int i = (int((vec2(false, 7)).g));\n\
            int j = int(h - float(6.0)) * (ivec4(true, a, c, d)).b - (ivec3(-4, h, true) * ivec3(true, c, true)).r;\n\
            const bool k = atan(float(int(-1.0))) < vec2(int(i), int(-4)).t * h;\n\
            uniform sampler2D l;\n\
            int m = ((ivec2(c, i))).y - (-3 + 6 * c);\n\
            void main()\n{\n\
            \tgl_FragColor = (texture2D(l, inversesqrt((vec2(k, m)))));\n\
            \tif (k)\n\t\t(vec3(j, bool(int(i) + int(h) - g), (float(f)) * float(float(1.0)) - float(e - -9)));\n\
            \telse \n\t\td * int(int(0 + c)) < b * (-3 + a);\n}\n";
        static VSTEXLOD_VS: &str =
            "attribute vec2 a_uv; uniform sampler2D u_vtex; uniform float u_lod; varying vec4 v_color; \
            void main() { v_color = texture2DLod(u_vtex, a_uv, u_lod); \
            gl_Position = vec4(a_uv * 2.0 - 1.0 + vec2(1.0 / 64.0), 0.0, 1.0); gl_PointSize = 1.0; }";
        static SARR_VS: &str =
            "attribute highp vec4 a_position;\nattribute highp vec4 a_coords;\nvarying mediump vec4 v_color;\n\
            struct S { sampler2D source; };\n\
            mediump vec4 fun(S s[2]) { return texture2D(s[0].source, vec2(0.5)); }\n\
            uniform S s[2];\nvoid main (void) { v_color = fun(s); gl_Position = a_position; }\n";
        static COLOR_FS: &str = "varying mediump vec4 v_color;\nvoid main (void) { gl_FragColor = v_color; }\n";
        for (what, vs, fs, stage) in [
            ("fragment.1", RSG_F1_VS, RSG_F1_FS, 'f'),
            ("126", VSTEXLOD_VS, COLOR_FS, 'v'),
            ("sarr", SARR_VS, COLOR_FS, 'v'),
        ] {
            with_program(vs, fs, Prim::Triangles, |p| {
                let p = p.unwrap_or_else(|e| panic!("{what}: {e:?}"));
                check_all(p);
                let code = if stage == 'f' { &p.fs.code } else { &p.vs.code };
                let ops: Vec<String> = code.instrs.iter().map(text).collect();
                let wrtmuc: Vec<&String> = ops.iter().filter(|o| o.contains("wrtmuc")).collect();
                assert_eq!(wrtmuc.len(), 2, "{what}: P0 and P1: {ops:#?}");
                for o in wrtmuc {
                    for other in ["rsqrt", "recip", "exp ", "log ", "sin ", "ldvpm", "stvpm", "vpmwt", "ldtmu", "tlb"] {
                        assert!(!o.contains(other), "{what}: wrtmuc with {other}: {o}");
                    }
                }
            });
        }
    }

    /// The rule itself, instruction by instruction: what 4.2 allows
    /// together and what it does not.
    #[test]
    fn the_peripheral_pairs_4_2_allows() {
        use super::super::verify::peripherals_compatible;
        let with = |f: &dyn Fn(&mut Alu)| {
            let mut a = Alu::nop();
            f(&mut a);
            peripherals_compatible(&a)
        };
        assert!(with(&|a| a.sig.wrtmuc = true), "a lone wrtmuc");
        assert!(!with(&|a| {
            a.sig.wrtmuc = true;
            a.add.op = AddOp::Rsqrt;
        }));
        assert!(!with(&|a| {
            a.sig.wrtmuc = true;
            a.add.op = AddOp::LdvpmvIn;
        }));
        assert!(
            with(&|a| {
                a.sig.wrtmuc = true;
                a.mul.op = MulOp::Mov;
                a.mul.magic = true;
                a.mul.waddr = 34; /* tmut */
            }),
            "wrtmuc with a TMU write"
        );
        assert!(
            !with(&|a| {
                a.sig.wrtmuc = true;
                a.mul.op = MulOp::Mov;
                a.mul.magic = true;
                a.mul.waddr = 32; /* tmuc */
            }),
            "not with a TMUC write"
        );
        assert!(
            with(&|a| {
                a.sig.ldtmu = true;
                a.add.op = AddOp::LdvpmvIn;
            }),
            "a TMU read with a VPM read"
        );
        assert!(
            with(&|a| {
                a.sig.ldtmu = true;
                a.add.op = AddOp::Stvpmv;
            }),
            "a TMU read with a VPM write"
        );
        assert!(
            !with(&|a| {
                a.sig.ldtmu = true;
                a.add.op = AddOp::Rsqrt;
            }),
            "a TMU read with an SFU op"
        );
        assert!(
            with(&|a| {
                a.sig.wrtmuc = true;
                a.add.op = AddOp::Fadd;
            }),
            "wrtmuc with ALU work"
        );
    }

    /// Float operations on constants fold when compiling, exactly, as Mesa
    /// folds them in NIR and as a GLSL `const` initializer is evaluated.
    /// dEQP random all_features.fragment.53 holds these two constants and
    /// the comparison between them: on the QPU's SFU, sqrt(12.25) was
    /// rsqrt then recip = 3.4999998, `n` truncated to 1 instead of 2, the
    /// branch dEQP's reference takes (2 >= 2) was skipped, and the Pi 4
    /// drew a constant colour (host: our code 0.4724121 = exp(-0.75)
    /// against the IR's 1). Folded, the choice is made when compiling: no
    /// SFU operation is left and the colour is a constant.
    ///
    /// Revert check: drop the `fold_float` call in prepare.rs `fold` and
    /// the rsqrt, recip and log come back.
    #[test]
    fn float_operations_on_constants_fold() {
        let fs = "precision mediump float;\n\
            const float o = (exp2(float(6.5) + -7.5)) * vec2(-9 - 5, (float(-1))).y;\n\
            const int n = ivec2(exp(o), sqrt(float(12.25)) + (-1.5)).g;\n\
            const float l = log2((float(20.5) * 1.0 - (float(4.5))));\n\
            void main() { gl_FragColor = n * 1 >= (int(float(l)) + -2) ? vec4(0.0, 1.0, 0.0, 1.0) : vec4(1.0, 0.0, 0.0, 1.0); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for sfu in ["rsqrt", "recip", "log", "exp", "sin"] {
                assert!(!ops.iter().any(|o| o.contains(sfu)), "{sfu} left: {ops:#?}");
            }
            /* the comparison is made when compiling: no flag is pushed */
            assert!(!ops.iter().any(|o| o.contains("push")), "a run-time comparison: {ops:#?}");
            /* the colour is a constant; which one (green, n = 2 and l = 4)
             * is the three-way's to check against the IR */
            let vals: Vec<u32> = p.fs.code.uniforms.iter().map(|u| u.data).collect();
            assert!(vals.iter().all(|&v| v == 0 || v == 1.0f32.to_bits()), "{vals:x?} {ops:#?}");
        });
    }

    /// A cube direction reaches the TMU divided by its major axis, as
    /// Mesa's nir_normalize_cubemap_coords makes it for every cube look-up
    /// (gallium v3d_program.c:374): two fmax of the absolute values, one
    /// recip, and T, R and SCM written from the three products. The host's
    /// cube model divides by the major axis itself, so only the sequence is
    /// pinned here; the board is the measurement (dEQP random cube shaders).
    ///
    /// Revert check: drop the normalisation in prepare.rs's Tex arm and
    /// the recip and the fmax pair disappear.
    #[test]
    fn a_cube_direction_is_divided_by_its_major_axis() {
        let fs = "precision mediump float; varying vec4 v_color; uniform samplerCube s; \
                  void main() { gl_FragColor = textureCube(s, v_color.xyz * 7.0); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let at = |needle: &str| ops.iter().position(|o| o.contains(needle));
            let n = |needle: &str| ops.iter().filter(|o| o.contains(needle)).count();
            assert_eq!(n("fmax"), 2, "max(max(|x|, |y|), |z|): {ops:#?}");
            assert!(ops.iter().any(|o| o.contains("fmax") && o.contains(".abs")), "on absolute values: {ops:#?}");
            let recip = at("recip").unwrap_or_else(|| panic!("no recip: {ops:#?}"));
            for w in ["tmut", "tmur", "tmuscm"] {
                let k = at(w).unwrap_or_else(|| panic!("no {w}: {ops:#?}"));
                assert!(k > recip, "{w} after the recip: {ops:#?}");
            }
        });
    }

    /// Two attributes the link binds to one location (glBindAttribLocation
    /// aliasing, `with_attrib_map`) are fetched through that location's one
    /// record and read its one VPM row: run in the interpreter, the vertex
    /// shader's `a` and `b` both come back as the row's value. The same
    /// shader bound apart (a at 0, b at 1) reads two rows, each name its
    /// own: the control that shows the run can tell the two layouts apart.
    #[test]
    fn aliased_attributes_read_one_vpm_row() {
        use crate::shaderc::online::powervr::abi::with_attrib_map;
        use crate::shaderc::online::v3d::sim::{self, Env, Inputs, VsInputs, LANES};
        use crate::shaderc::{ShcAttribBinding, ShcAttribMap};
        let vs = "attribute float a; attribute float b; uniform float u; varying vec4 v; \
                  void main() { v = vec4(a, b, u, 0.0); gl_Position = vec4(0.0, 0.0, 0.0, 1.0); }";
        let fs = "precision mediump float; varying vec4 v; void main() { gl_FragColor = v; }";
        /* per lane: row 0 holds 10 + lane, row 1 (bound apart) 100 + lane */
        let row = |base: f32| -> [u32; LANES] { core::array::from_fn(|l| (base + l as f32).to_bits()) };
        let run = |loc_b: u8| -> Vec<[u32; LANES]> {
            let mut outs = Vec::new();
            store::with_scratch(|a| {
                let vsrc = a.alloc_str(vs).unwrap();
                let fsrc = a.alloc_str(fs).unwrap();
                let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
                let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
                let mut map = ShcAttribMap::ZERO;
                for (i, (name, loc)) in [("a", 0u8), ("b", loc_b)].into_iter().enumerate() {
                    let mut b = ShcAttribBinding {
                        location: loc,
                        columns: 1,
                        gl_type: 0x1406,
                        active: 1,
                        ..ShcAttribBinding::ZERO
                    };
                    b.name[0] = name.as_bytes()[0] as core::ffi::c_char;
                    map.b[i] = b;
                }
                map.count = 2;
                let module = a.alloc_copy(core::slice::from_ref(&v.module)).unwrap();
                let (m, _) = with_attrib_map(a, &module[0], v.reflection.attribs, &map).expect("mapped");
                let p = compile_program(m, &fr.module, Prim::Triangles, TARGET, a).expect("compiles");
                let records = p.attributes.iter().filter(|e| e.1.is_some()).count();
                assert_eq!(records, if loc_b == 0 { 1 } else { 2 }, "records: {:?}", p.attributes);
                /* the uniform stream, resolved: constants as they are, `u`
                 * (dword 0 of the block) 7.0, the viewport words 1.0 */
                let uniforms: Vec<u32> =
                    p.vs.code
                        .uniforms
                        .iter()
                        .map(|un| match un.contents {
                            0 => un.data,
                            1 => {
                                assert_eq!(un.data, 0, "u is the block's first dword");
                                7.0f32.to_bits()
                            }
                            2..=5 => 1.0f32.to_bits(),
                            c => panic!("uniform contents {c} not modelled here"),
                        })
                        .collect();
                let rows = [row(10.0), row(100.0)][..records].to_vec();
                let env = Env { threads: p.vs.threads, ..Env::default() };
                let got = sim::run(p.vs.code.words, &uniforms, &Inputs::Vertex(VsInputs { vpm: rows }), &env)
                    .expect("the vertex shader runs");
                outs = got.vpm_out.iter().map(|r| r.expect("every output row written")).collect();
            })
            .expect("scratch");
            outs
        };
        /* v's rows are the last four the vertex shader writes */
        let v_of = |out: &[[u32; LANES]]| -> Vec<[u32; LANES]> { out[out.len() - 4..].to_vec() };
        let aliased = v_of(&run(0));
        assert_eq!(aliased[2], [7.0f32.to_bits(); LANES], "u, so these rows are v's");
        assert_eq!(aliased[0], row(10.0), "a reads row 0");
        assert_eq!(aliased[1], row(10.0), "b, bound to a's location, reads the same row");
        let apart = v_of(&run(1));
        assert_eq!(apart[0], row(10.0), "control: a reads row 0");
        assert_eq!(apart[1], row(100.0), "control: b bound apart reads its own row");
    }

    /// A loop the compiler cannot count stops at UNIFORM_LOOP_CAP (4096), the
    /// structural termination guarantee (CLAUDE.md): the V3D keeps it as a
    /// counted loop with the shared unroller's count, and select.rs loads that
    /// count before the head and takes the back edge only while a lane has
    /// neither broken nor counted down to 0.
    ///
    /// The cap is the SHARED unroller's (powervr/unroll.rs:93, applied at
    /// :522-523 to every data-dependent exit, uniform or divergent), which the
    /// V3D reaches through chain.rs `unrolled`. If the unroller is ever split
    /// per GPU, the V3D's copy must keep the cap: this test fails if it does
    /// not.
    /// verify::branches_are_counted_loops checks the structure; this pins the
    /// VALUE, run in the interpreter:
    /// - gl_test 104's divloop (tools/shaders/es300/divloop.frag): band b = 0..6
    ///   loops b times, and band 7's bound of 1e9 stops at 4096 (blue 240/255,
    ///   red and green 0), as gl_test.c case_divloop expects;
    /// - its divnest: both loops are counted 4096 and it ends;
    /// - `while (true)` with a data break past the cap: every lane ends at 4096;
    /// - a CONSTANT bound above the cap, `for (i = 0; i < 5000; i++)`, runs
    ///   exactly 5000 passes: past its 256-pass simulation the shared unroller
    ///   reads a constant `for` loop's count from its index
    ///   (online/loop_count.rs, exact up to EXACT_PASSES_MAX = 65,536 passes) and
    ///   keeps it as a loop with that exact count. The GC7000 applies the same
    ///   recognizer (since 9d413725), so the three GPUs agree. Before (C')
    ///   (imx8mp, 2026-09-25) such a loop was cut at 4096 on the PowerVR and
    ///   the V3D. A constant bound of 1000 is the control, exact at 1000.
    ///
    /// The GC7000 reset on 104 because its path had no cap (imx8mp,
    /// 2026-09-25); this makes the V3D's cap a test, not only a comment. The
    /// first three are GLSL ES 3.00 (gl_test 104's own sources) and run where
    /// ES 3.00 compiles; the constant-bound pair is GLSL ES 1.00 (legal under
    /// Appendix A), so it runs in every build, report-es2 (the dEQP-GLES2
    /// images) included.
    #[test]
    fn a_loop_the_compiler_cannot_count_stops_at_the_cap() {
        use crate::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP;
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
        assert_eq!(UNIFORM_LOOP_CAP, 4096, "gl_test 104 and the colours below assume 4096");
        const WHILE_TRUE: &str = "#version 300 es\nprecision highp float;\nin float v_s;\n\
            layout(location = 0) out vec4 o;\nvoid main() { float n = 0.0; \
            while (true) { n += 1.0; if (n > v_s * 1.0e9) break; } \
            o = vec4(vec3(mod(n, 16.0), mod(floor(n / 16.0), 16.0), min(floor(n / 256.0), 15.0)) * (16.0 / 255.0), 1.0); }";
        const BANDS_VS: &str = include_str!("../../../../tools/shaders/es300/bands.vert");
        const ES100_VS: &str = "attribute vec4 p;\nvarying float v_s;\nvoid main() { v_s = p.x; gl_Position = p; }";
        /* per lane: v_s at the centre of band (lane % 8), so all eight bands run */
        let v_s = |l: usize| ((l % 8) as f32 + 0.5) / 8.0;
        /* the f16 colour gl_test.c divloop_colour draws for n passes: (r|g<<16, b|a<<16);
         * f16 of a normal f32 or 0, rounded to nearest even as the TLB's f16
         * write does */
        let f16 = |x: f32| -> u32 {
            let b = x.to_bits();
            if b == 0 {
                return 0;
            }
            let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
            assert!((1..31).contains(&exp), "{x} is not a normal f16");
            let (mant, rem) = ((b >> 13) & 0x3ff, b & 0x1fff);
            let h = ((exp as u32) << 10) | mant;
            if rem > 0x1000 || (rem == 0x1000 && h & 1 == 1) {
                h + 1
            } else {
                h
            }
        };
        let colour = |n: u32| -> (u32, u32) {
            let (r, g, b) = (n % 16, (n / 16) % 16, (n / 256).min(15));
            let h = |k: u32| f16(k as f32 * 16.0 / 255.0);
            (h(r) | h(g) << 16, h(b) | 0x3c00 << 16)
        };
        let run = |vs: &str, fs: &str| -> (Vec<u32>, Vec<(u32, u32)>, usize) {
            let mut out = (Vec::new(), Vec::new(), 0);
            store::with_scratch(|a| {
                let vsrc = a.alloc_str(vs).unwrap();
                let fsrc = a.alloc_str(fs).unwrap();
                let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
                let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
                let (_, trips) = unrolled(&fr.module, a).unwrap();
                let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a).expect("compiles");
                check_all(&p);
                let uniforms: Vec<u32> =
                    p.fs.code
                        .uniforms
                        .iter()
                        .map(|u| match u.contents {
                            0 => u.data,
                            c => panic!("uniform contents {c} not modelled here"),
                        })
                        .collect();
                let vary = Varying { vary: core::array::from_fn(|l| v_s(l).to_bits()), c: [0; LANES] };
                let inputs = FsInputs {
                    w: [1.0f32.to_bits(); LANES],
                    w_centroid: [1.0f32.to_bits(); LANES],
                    z: [0; LANES],
                    x: [0; LANES],
                    y: [0; LANES],
                    back_facing: [false; LANES],
                    varyings: std::vec![vary; p.fs.num_inputs as usize],
                };
                let env = Env { threads: p.fs.threads, ..Env::default() };
                let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
                    .expect("the program ends (a loop that does not end is a GPU reset on the board)");
                assert_eq!(o.tlb.len(), 2, "one f16 colour, two TLB words");
                let px: Vec<(u32, u32)> =
                    (0..LANES).map(|l| (o.tlb[0].value[l].unwrap(), o.tlb[1].value[l].unwrap())).collect();
                out = (trips.iter().map(|t| t.count).collect(), px, p.fs.code.back_edges.len());
            })
            .expect("scratch");
            out
        };
        /* GLSL ES 3.00: a report-es2 build refuses it in the front end */
        if !cfg!(feature = "report-es2") {
            let (trips, px, edges) = run(BANDS_VS, include_str!("../../../../tools/shaders/es300/divloop.frag"));
            assert_eq!((trips, edges), (std::vec![UNIFORM_LOOP_CAP], 1), "divloop: one loop, counted to the cap (anything else, and band 7's bound of 1e9 is an unbounded loop: a GPU reset on the board, as the GC7000 had on gl_test 104)");
            for (l, &p) in px.iter().enumerate() {
                let b = (l % 8) as u32;
                let n = if b == 7 { UNIFORM_LOOP_CAP } else { b };
                assert_eq!(
                    p,
                    colour(n),
                    "divloop lane {l} (band {b}): {n} passes (band 7: a different count means the cap moved, and gl_test 104 fails on the board)"
                );
            }
            let (trips, _, edges) = run(BANDS_VS, include_str!("../../../../tools/shaders/es300/divnest.frag"));
            assert_eq!(
                (trips, edges),
                (std::vec![UNIFORM_LOOP_CAP; 2], 2),
                "divnest: two loops, each counted to the cap (an uncapped loop is a GPU reset on the board)"
            );
            let (trips, px, _) = run(BANDS_VS, WHILE_TRUE);
            assert_eq!(
                trips,
                std::vec![UNIFORM_LOOP_CAP],
                "while (true): counted to the cap (uncapped, it never ends: a GPU reset on the board)"
            );
            assert!(
                px.iter().all(|&p| p == colour(UNIFORM_LOOP_CAP)),
                "while (true): every lane ends at the cap: {px:x?}"
            );
        }
        let constant = |bound: u32| {
            std::format!(
                "precision highp float;\nvarying float v_s;\n\
                 void main() {{ float n = 0.0; for (int i = 0; i < {bound}; i++) n += 1.0; \
                 gl_FragColor = vec4(vec3(mod(n, 16.0), mod(floor(n / 16.0), 16.0), min(floor(n / 256.0), 15.0)) * (16.0 / 255.0), 1.0); }}"
            )
        };
        let (_, px, _) = run(ES100_VS, &constant(5000));
        assert!(
            px.iter().all(|&p| p == colour(5000)),
            "a constant bound of 5000 runs exactly 5000 passes (online/loop_count.rs counts a constant `for` \
             loop up to 65,536 entries on all three GPUs); 4096 here means the unroller cuts constant loops at \
             the cap again: {px:x?}"
        );
        let (_, px, _) = run(ES100_VS, &constant(1000));
        assert!(
            px.iter().all(|&p| p == colour(1000)),
            "control: a constant bound of 1000 runs exactly 1000 passes: {px:x?}"
        );
    }

    /* GLSL ES 3.00: a report-es2 build refuses it in the front end */
    #[cfg(not(feature = "report-es2"))]
    /// Run the vertex (`coord` false) or coordinate shader of `vs` + `fs` in
    /// the interpreter on the VPM input `rows` (one value per lane each), the
    /// viewport words 1.0; returns the program facts and the output rows.
    fn run_vertex_stage(
        vs: &str,
        fs: &str,
        coord: bool,
        rows: &[[u32; 16]],
    ) -> ((bool, bool), [u8; 16], Vec<[u32; 16]>) {
        use crate::shaderc::online::v3d::sim::{self, Env, Inputs, VsInputs};
        let mut out = ((false, false), [0u8; 16], Vec::new());
        store::with_scratch(|a| {
            let vsrc = a.alloc_str(vs).unwrap();
            let fsrc = a.alloc_str(fs).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
            let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a).expect("compiles");
            check_all(&p);
            let st = if coord { &p.cs } else { &p.vs };
            let uniforms: Vec<u32> = st
                .code
                .uniforms
                .iter()
                .map(|u| match u.contents {
                    0 => u.data,
                    2..=5 => 1.0f32.to_bits(),
                    c => panic!("uniform contents {c} not modelled here"),
                })
                .collect();
            let env = Env { threads: st.threads, ..Env::default() };
            let got = sim::run(st.code.words, &uniforms, &Inputs::Vertex(VsInputs { vpm: rows.to_vec() }), &env)
                .expect("the stage runs");
            out = (
                (st.uses_vid, st.uses_iid),
                st.vattr_sizes,
                got.vpm_out.iter().map(|r| r.expect("every output row written")).collect(),
            );
        })
        .expect("scratch");
        out
    }

    /* GLSL ES 3.00: a report-es2 build refuses it in the front end */
    #[cfg(not(feature = "report-es2"))]
    /// gl_VertexID is VPM input row 0, ahead of the attributes (Mesa
    /// ntq_setup_vs_inputs, nir_to_vir.c:2402-2421; the attribute index skips
    /// the builtin rows, :2890-2922), and the stage says it reads it
    /// (uses_vid: the shader record's "reads vertex ID" bit, v3d_job.c). Run in
    /// the interpreter with IDs 0..15 in row 0: the colour gl_test 98's
    /// es300/vid.vert makes of `gl_VertexID % 4` comes back per lane, and an
    /// attribute read in the same shader comes back from ITS rows, so the
    /// attribute index is offset by the ID row. On the board the ID was refused
    /// before this (gl_test 98: NotYet("gl_VertexID"), records CPB/CPF).
    #[test]
    fn gl_vertex_id_is_vpm_row_0_ahead_of_the_attributes() {
        const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 pos;\nlayout(location = 1) in vec4 extra;\n\
            out vec4 v_col;\nout vec4 v_extra;\nvoid main() {\n    int i = gl_VertexID % 4;\n    \
            v_col = vec4(i == 0 || i == 3 ? 1.0 : 0.0, i == 1 || i == 3 ? 1.0 : 0.0, i == 2 || i == 3 ? 1.0 : 0.0, 1.0);\n    \
            v_extra = extra;\n    gl_Position = pos;\n}\n";
        const FS: &str = "#version 300 es\nprecision highp float;\nin vec4 v_col;\nin vec4 v_extra;\n\
            layout(location = 0) out vec4 o;\nvoid main() { o = v_col + v_extra; }\n";
        let f = |x: f32| -> u32 { x.to_bits() };
        let lanes = |g: &dyn Fn(usize) -> u32| -> [u32; 16] { core::array::from_fn(g) };
        /* row 0 the ID (an int), 1..4 pos, 5..8 extra */
        let mut rows = std::vec![lanes(&|l| l as u32)];
        for c in 0..4 {
            rows.push(lanes(&|_| f(if c == 3 { 1.0 } else { 0.0 })));
        }
        for c in 0..4 {
            rows.push(lanes(&|l| f(100.0 + 16.0 * c as f32 + l as f32)));
        }
        let ((uses_vid, _), sizes, out) = run_vertex_stage(VS, FS, false, &rows);
        assert!(uses_vid, "the vertex shader reads the ID: the record's bit must be set");
        assert_eq!(sizes[..2], [4, 4], "pos and extra: 4 rows each, after the ID row");
        let colour = |c: usize| -> [u32; 16] {
            lanes(&|l| {
                let i = l % 4;
                f(match c {
                    0 => (i == 0 || i == 3) as u32 as f32,
                    1 => (i == 1 || i == 3) as u32 as f32,
                    2 => (i == 2 || i == 3) as u32 as f32,
                    _ => 1.0,
                })
            })
        };
        let find = |want: &[[u32; 16]]| out.windows(want.len()).any(|w| w == want);
        let col: Vec<[u32; 16]> = (0..4).map(colour).collect();
        assert!(find(&col), "v_col = the colour of gl_VertexID % 4 per lane: {out:x?}");
        assert!(find(&rows[5..9]), "v_extra = extra's rows 5..8: the attribute index skips the ID row: {out:x?}");
    }

    /* GLSL ES 3.00: a report-es2 build refuses it in the front end */
    #[cfg(not(feature = "report-es2"))]
    /// GFXH-1602 (Mesa nir_to_vir.c:2907-2915): a coordinate shader that reads
    /// a builtin but none of attribute 0 still has attribute 0 active, one
    /// row the driver loads (v3d_job.c, the attribute records, after
    /// v3dx_draw.c:832-843), so its later attributes start one row further.
    /// Here the coordinate shader reads the ID and attribute 1 only; row 1 is
    /// the dummy, filled with a poison value the result must not show.
    #[test]
    fn a_coordinate_shader_reading_the_id_skips_the_gfxh_1602_row() {
        const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 col;\nlayout(location = 1) in vec4 off;\n\
            out vec4 v;\nvoid main() { v = col; gl_Position = vec4(float(gl_VertexID), 0.0, 0.0, 1.0) + off; }\n";
        const FS: &str = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
            layout(location = 0) out vec4 o;\nvoid main() { o = v; }\n";
        let f = |x: f32| -> u32 { x.to_bits() };
        let lanes = |g: &dyn Fn(usize) -> u32| -> [u32; 16] { core::array::from_fn(g) };
        let poison = f(-7777.0);
        /* the coordinate shader: row 0 the ID, row 1 the GFXH-1602 dummy, 2..5 off */
        let mut rows = std::vec![lanes(&|l| l as u32), lanes(&|_| poison)];
        for c in 0..4 {
            rows.push(lanes(&|l| f(0.5 * c as f32 + 1000.0 * (l as f32 + 1.0))));
        }
        let ((uses_vid, _), sizes, out) = run_vertex_stage(VS, FS, true, &rows);
        assert!(uses_vid, "the coordinate shader reads the ID (its position does)");
        assert_eq!(sizes[..2], [0, 4], "the coordinate shader reads none of attribute 0 (col) and all of off");
        /* clip x y z w at rows 0..3 (select_vertex, the coordinate shader) */
        let want: Vec<[u32; 16]> = (0..4)
            .map(|c| {
                lanes(&|l| {
                    let off = 0.5 * c as f32 + 1000.0 * (l as f32 + 1.0);
                    f(match c {
                        0 => l as f32 + off,
                        3 => 1.0 + off,
                        _ => off,
                    })
                })
            })
            .collect();
        assert_eq!(out[..4], want[..], "clip position = (ID, 0, 0, 1) + off, off read from rows 2..5");
        assert!(!out.iter().flatten().any(|&x| x == poison), "the GFXH-1602 row is never read");
    }

    /// gl_InstanceID is VPM input row 0 and pushes gl_VertexID to row 1
    /// (Mesa ntq_setup_vs_inputs, nir_to_vir.c:2415-2421: instance ID, base
    /// instance, vertex ID), and the attributes start after both
    /// (:2890-2905). On the board the driver draws one instance, so the
    /// row holds 0; here every row holds a distinct value per lane, so a
    /// read of the wrong row shows. The pairs that read gl_InstanceID were
    /// refused before this (NotYet("gl_InstanceID"), tests/v3d_sim_mesa.rs).
    /* GLSL ES 3.00: a report-es2 build refuses it in the front end */
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn gl_instance_id_is_vpm_row_0_ahead_of_the_vertex_id() {
        const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 pos;\nlayout(location = 1) in vec4 extra;\n\
            out vec4 v;\nvoid main() {\n    v = vec4(float(gl_InstanceID), float(gl_VertexID), extra.x, extra.w);\n    \
            gl_Position = pos;\n}\n";
        const FS: &str = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
            layout(location = 0) out vec4 o;\nvoid main() { o = v; }\n";
        let f = |x: f32| -> u32 { x.to_bits() };
        let lanes = |g: &dyn Fn(usize) -> u32| -> [u32; 16] { core::array::from_fn(g) };
        /* row 0 the instance ID, 1 the vertex ID (ints), 2..5 pos, 6..9 extra */
        let mut rows = std::vec![lanes(&|l| 1000 + l as u32), lanes(&|l| l as u32)];
        for c in 0..4 {
            rows.push(lanes(&|_| f(if c == 3 { 1.0 } else { 0.0 })));
        }
        for c in 0..4 {
            rows.push(lanes(&|l| f(100.0 + 16.0 * c as f32 + l as f32)));
        }
        let (facts, sizes, out) = run_vertex_stage(VS, FS, false, &rows);
        assert_eq!(facts, (true, true), "the vertex shader reads both IDs: (uses_vid, uses_iid)");
        assert_eq!(sizes[..2], [4, 4], "pos: 4 rows; extra: x and w, so 4 rows (w is component 3)");
        let want = [lanes(&|l| f(1000.0 + l as f32)), lanes(&|l| f(l as f32)), rows[6], rows[9]];
        assert!(
            out.windows(4).any(|w| w == want),
            "v = (instance ID, vertex ID, extra.x, extra.w) from rows 0, 1, 6, 9: {out:x?}"
        );
    }

    /// GFXH-1602 with both IDs: the coordinate shader reads the instance ID
    /// (row 0), the vertex ID (row 1) and none of attribute 0, so the
    /// driver's attribute 0 row is row 2 and attribute 1 starts at row 3.
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn a_coordinate_shader_reading_both_ids_skips_the_gfxh_1602_row() {
        const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 col;\nlayout(location = 1) in vec4 off;\n\
            out vec4 v;\nvoid main() { v = col; \
            gl_Position = vec4(float(gl_InstanceID), float(gl_VertexID), 0.0, 1.0) + off; }\n";
        const FS: &str = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
            layout(location = 0) out vec4 o;\nvoid main() { o = v; }\n";
        let f = |x: f32| -> u32 { x.to_bits() };
        let lanes = |g: &dyn Fn(usize) -> u32| -> [u32; 16] { core::array::from_fn(g) };
        let poison = f(-7777.0);
        let mut rows = std::vec![lanes(&|l| 500 + l as u32), lanes(&|l| l as u32), lanes(&|_| poison)];
        for c in 0..4 {
            rows.push(lanes(&|l| f(0.5 * c as f32 + 1000.0 * (l as f32 + 1.0))));
        }
        let (facts, sizes, out) = run_vertex_stage(VS, FS, true, &rows);
        assert_eq!(facts, (true, true), "the coordinate shader reads both IDs (its position does)");
        assert_eq!(sizes[..2], [0, 4], "none of attribute 0 (col), all of off");
        let want: Vec<[u32; 16]> = (0..4)
            .map(|c| {
                lanes(&|l| {
                    let off = 0.5 * c as f32 + 1000.0 * (l as f32 + 1.0);
                    f(match c {
                        0 => (500 + l) as f32 + off,
                        1 => l as f32 + off,
                        3 => 1.0 + off,
                        _ => off,
                    })
                })
            })
            .collect();
        assert_eq!(out[..4], want[..], "clip position = (iid, vid, 0, 1) + off, off from rows 3..6");
        assert!(!out.iter().flatten().any(|&x| x == poison), "the GFXH-1602 row is never read");
    }

    /// A flat varying is the C coefficient alone (Mesa emit_flat_varying:
    /// MOV of r5, the ldvary result unread), and its inputs' bits are set in
    /// flat_shade_flags, which makes the hardware put the provoking vertex's
    /// value in C (the job builder's FLAT_SHADE_FLAGS). Here a smooth vec2
    /// (inputs 0, 1) and a flat vec2 (inputs 2, 3): the flags are 0b1100, and
    /// in the interpreter the flat inputs carry noise in the ldvary result
    /// and their value in C, the smooth ones the reverse, so a flat read of
    /// the ldvary result, or a smooth read of C alone, shows in the colour.
    /// Before this, prepare refused "flat varying" (17 corpus pairs, gl_test
    /// 89 flatlast among them).
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn a_flat_varying_is_the_c_coefficient_and_sets_its_flag() {
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
        const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 p;\nout vec2 s;\nflat out vec2 fl;\n\
            void main() { s = p.xy; fl = p.zw; gl_Position = p; }\n";
        const FS: &str = "#version 300 es\nprecision highp float;\nin vec2 s;\nflat in vec2 fl;\n\
            layout(location = 0) out vec4 o;\nvoid main() { o = vec4(s, fl); }\n";
        /* per lane and input: a value exact in f16 */
        let value = |l: usize, i: usize| (l as f32 + 16.0 * i as f32) / 64.0;
        let noise = 1000.0f32.to_bits();
        store::with_scratch(|a| {
            let vsrc = a.alloc_str(VS).unwrap();
            let fsrc = a.alloc_str(FS).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
            let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a).expect("compiles");
            check_all(&p);
            assert_eq!(p.fs.num_inputs, 4);
            assert_eq!(p.fs.flat_shade_flags, 0b1100, "fl.x and fl.y are inputs 2 and 3");
            let uniforms: Vec<u32> =
                p.fs.code
                    .uniforms
                    .iter()
                    .map(|u| match u.contents {
                        0 => u.data,
                        c => panic!("uniform contents {c} not modelled here"),
                    })
                    .collect();
            let varyings = (0..4)
                .map(|i| {
                    let v: [u32; LANES] = core::array::from_fn(|l| value(l, i).to_bits());
                    if p.fs.flat_shade_flags & (1 << i) != 0 {
                        Varying { vary: [noise; LANES], c: v }
                    } else {
                        /* W is 1: vary * W + C with C 0 */
                        Varying { vary: v, c: [0; LANES] }
                    }
                })
                .collect();
            let inputs = FsInputs {
                w: [1.0f32.to_bits(); LANES],
                w_centroid: [1.0f32.to_bits(); LANES],
                z: [0; LANES],
                x: [0; LANES],
                y: [0; LANES],
                back_facing: [false; LANES],
                varyings,
            };
            let env = Env { threads: p.fs.threads, ..Env::default() };
            let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
                .expect("runs");
            assert_eq!(o.tlb.len(), 2, "one f16 colour, two TLB words");
            let h = |x: f32| u32::from(sim::f32_to_half(x));
            for l in 0..LANES {
                let want = [h(value(l, 0)) | h(value(l, 1)) << 16, h(value(l, 2)) | h(value(l, 3)) << 16];
                let got = [o.tlb[0].value[l].unwrap(), o.tlb[1].value[l].unwrap()];
                assert_eq!(got, want, "lane {l}");
            }
        })
        .expect("scratch");
    }

    /// Multiple render targets, the compiler's half (docs, "Multiple render
    /// targets", step 1): one f16 vec4 write per RT the shader writes, in RT
    /// order, the first word of RT n > 0 through TLBU with Mesa's config
    /// (vir_emit_tlb_color_write, nir_to_vir.c:1953-1984: RT number reversed,
    /// "7 = RT 0"). A shader writing locations 0 and 2 (RT 1 skipped) runs in
    /// the interpreter: four TLB words, RT 0's with no config (the default,
    /// as today), RT 2's first with 0xffffffef, and the program reports
    /// rt_mask 0b101, which the backend checks against the draw's
    /// attachments.
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn each_render_target_is_one_tlb_write_with_its_config() {
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
        assert_eq!(crate::shaderc::online::v3d::select::tlb_config_f16(0, 4), !0, "RT 0: the TLB's default, no TLBU");
        assert_eq!(crate::shaderc::online::v3d::select::tlb_config_f16(1, 4), 0xffff_fff7, "RT 1: (7 - 1) << 3");
        assert_eq!(crate::shaderc::online::v3d::select::tlb_config_f16(2, 4), 0xffff_ffef, "RT 2: (7 - 2) << 3");
        const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 p;\nout vec4 v;\n\
            void main() { v = p; gl_Position = p; }\n";
        const FS: &str = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
            layout(location = 0) out vec4 o0;\nlayout(location = 2) out vec4 o2;\n\
            void main() { o0 = v; o2 = v.wzyx; }\n";
        let value = |l: usize, c: usize| (l as f32 + 16.0 * c as f32) / 64.0;
        store::with_scratch(|a| {
            let vsrc = a.alloc_str(VS).unwrap();
            let fsrc = a.alloc_str(FS).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
            let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a).expect("compiles");
            assert_eq!(p.fs.rt_mask, 0b101, "RT 0 and RT 2: the backend checks it against the attachments");
            check_all(&p);
            let uniforms: Vec<u32> =
                p.fs.code
                    .uniforms
                    .iter()
                    .map(|u| match u.contents {
                        0 => u.data,
                        c => panic!("uniform contents {c} not modelled here"),
                    })
                    .collect();
            let varyings = (0..4)
                .map(|c| Varying { vary: core::array::from_fn(|l| value(l, c).to_bits()), c: [0; LANES] })
                .collect();
            let inputs = FsInputs {
                w: [1.0f32.to_bits(); LANES],
                w_centroid: [1.0f32.to_bits(); LANES],
                z: [0; LANES],
                x: [0; LANES],
                y: [0; LANES],
                back_facing: [false; LANES],
                varyings,
            };
            let env = Env { threads: p.fs.threads, ..Env::default() };
            let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
                .expect("runs");
            let configs: Vec<Option<u32>> = o.tlb.iter().map(|t| t.config).collect();
            assert_eq!(configs, [None, None, Some(0xffff_ffef), None], "RT 0 as today, then RT 2 with its config");
            let h = |x: f32| u32::from(sim::f32_to_half(x));
            for l in 0..LANES {
                let got: Vec<u32> = o.tlb.iter().map(|t| t.value[l].unwrap()).collect();
                let rt0 = [h(value(l, 0)) | h(value(l, 1)) << 16, h(value(l, 2)) | h(value(l, 3)) << 16];
                let rt2 = [h(value(l, 3)) | h(value(l, 2)) << 16, h(value(l, 1)) | h(value(l, 0)) << 16];
                assert_eq!(got, [rt0[0], rt0[1], rt2[0], rt2[1]], "lane {l}");
            }
        })
        .expect("scratch");
    }

    /// dFdx and dFdy are Mesa's (nir_to_vir.c:4052-4065): FDX of the value,
    /// and FDY of the value times the window transform's y scale, the word
    /// QUNIFORM_FB_WPOS_Y_SCALE that gl_FragCoord.y reads (st/mesa's
    /// nir_lower_wpos_ytransform lower_ddy: ddy(p * transform.x), -1 for a
    /// window, +1 for an FBO). The interpreter does not model a quad's lanes,
    /// so the values are the board's to measure; this pins the sequence.
    /// Before this, prepare refused "Fddx" (5 corpus pairs).
    #[test]
    fn derivatives_are_fdx_and_fdy_of_the_y_scaled_value() {
        /* gl_test's own case shaders (gl_test.c "deriv") */
        const VS: &str = include_str!("../../../../tools/shaders/es100/deriv.vert");
        const FS: &str = include_str!("../../../../tools/shaders/es100/deriv.frag");
        store::with_scratch(|a| {
            let vsrc = a.alloc_str(VS).unwrap();
            let fsrc = a.alloc_str(FS).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
            let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a).expect("compiles");
            check_all(&p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let has = |needle: &str| ops.iter().filter(|o| o.contains(needle)).count();
            assert_eq!((has("fdx"), has("fdy")), (2, 2), "two of each: {ops:#?}");
            let codes: Vec<u32> = p.fs.code.uniforms.iter().map(|u| u.contents).collect();
            assert!(codes.contains(&QUNIFORM_FB_WPOS_Y_SCALE), "dFdy reads the y scale: {codes:x?}");
            /* The data path, in select's output (a word in the stream is
             * not enough: an unread uniform is loaded all the same): FDY's
             * operand is an FMUL's result, and one of that FMUL's operands
             * is the value loaded from the y-scale word. */
            use crate::shaderc::online::v3d::isa::{AddOp, MulOp};
            use crate::shaderc::online::v3d::select::{select_fragment, Dst, Operand, Pressure, VOp};
            let fsp = prepare_fragment_with(&fr.module, &[], 0, a).expect("prepares");
            let inputs = fragment_inputs(&fsp, a).expect("inputs");
            let sel = select_fragment(&fsp, inputs, 0, Pressure::None, a).expect("selects");
            let def = |v: Operand| {
                sel.ops.iter().find(|o| match (o, v) {
                    (VOp::Add { dst: Dst::V(d), .. } | VOp::Mul { dst: Dst::V(d), .. }, Operand::V(x)) => *d == x,
                    (VOp::Ldunif { dst, .. }, Operand::V(x)) => *dst == x,
                    _ => false,
                })
            };
            let is_scale =
                |v: Operand| matches!(def(v), Some(VOp::Ldunif { u, .. }) if u.contents == QUNIFORM_FB_WPOS_Y_SCALE);
            let mut fdys = 0;
            for o in sel.ops.iter() {
                let VOp::Add { op: AddOp::Fdy, a: x, .. } = o else { continue };
                fdys += 1;
                let Some(VOp::Mul { op: MulOp::Fmul, a: m0, b: m1, .. }) = def(*x) else {
                    panic!("FDY's operand is not an FMUL's result: {:#?}", sel.ops)
                };
                assert!(is_scale(*m0) || is_scale(*m1), "the FMUL feeding FDY takes the y scale: {:#?}", sel.ops);
            }
            assert_eq!(fdys, 2, "dFdy(v.y) and dFdy(gl_FragCoord.y)");
        })
        .expect("scratch");
    }

    /// Merging a loop's two exits into one branch (select.rs merge_exits)
    /// leaves the loop's bound where it was: the counted back edge (ES 1.00
    /// Appendix A; verify.rs branches_are_counted_loops in check_all). With
    /// a constant bound it counts the bound; with a bound read at run time it
    /// counts to UNIFORM_LOOP_CAP (powervr/unroll.rs, the deliberate cap), so
    /// the merged exit is never the only way out. A `while (true)` with two
    /// breaks on data (no bound at all in GLSL) is no exception: it is
    /// counted to the cap too, its two breaks merged into one exit, so an
    /// unbounded loop still cannot run on (the assembler's guarantee, and
    /// what a merge must not weaken). Revert check: with merge_exits off the
    /// exit counts are 2.
    #[test]
    fn a_merged_exit_leaves_the_loop_bound_to_the_back_edge() {
        use crate::shaderc::online::powervr::unroll::UNIFORM_LOOP_CAP;
        use crate::shaderc::online::v3d::select::Uniform;
        const VS: &str = "attribute vec4 p; void main() { gl_Position = p; }";
        let head = "precision mediump float; uniform float u_n; uniform float u_k;\nvoid main() { float acc = 0.0; float i = 0.0;\n";
        let tail = " gl_FragColor = vec4(acc, 0.0, 0.0, 1.0); }";
        let for_loop = |bound: &str| {
            std::format!(
                "{head}for (float j = 0.0; j < {bound}; j += 1.0) {{ if (j >= u_k) break; acc += 0.25; }}{tail}"
            )
        };
        let run = |src: &str| -> Result<(usize, usize, bool), std::string::String> {
            store::with_scratch(|a| {
                let vs = a.alloc_str(VS).unwrap();
                let fsrc = a.alloc_str(src).unwrap();
                let v = lower::compile_to_ir(a, Stage::Vertex, vs).map_err(|e| std::format!("{:?}", e.msg))?;
                let f = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| std::format!("{:?}", e.msg))?;
                let p = compile_program(&v.module, &f.module, Prim::Triangles, TARGET, a)
                    .map_err(|e| std::format!("{e:?}"))?;
                check_all(&p);
                let c = &p.fs.code;
                let (edge, head) = c.back_edges[0];
                let exits = c.fwd_branches.iter().filter(|&&(b, _)| b >= head && b < edge).count();
                let cap = c.uniforms.contains(&Uniform::constant(UNIFORM_LOOP_CAP));
                Ok((c.back_edges.len(), exits, cap))
            })
            .unwrap()
        };
        assert_eq!(run(&for_loop("8.0")), Ok((1, 1, false)), "constant bound: counted by it, one merged exit");
        assert_eq!(run(&for_loop("u_n")), Ok((1, 1, true)), "bound read at run time: counted to the cap");
        let w =
            run(&std::format!("{head}while (true) {{ if (i >= u_n) break; if (i >= u_k) break; i += 1.0; }}{tail}"));
        assert_eq!(w, Ok((1, 1, true)), "while (true), two data breaks: one exit, counted to the cap");
    }

    /// A uniform defined before a counted loop and read inside it is loaded
    /// before the loop's head (select.rs SOp::LoopHead, Mesa's nir_opt_move
    /// stays in the block), not on every pass: femtovg filter_image's
    /// direction (dir.x, dir.y) was loaded twice a pass. Revert check:
    /// without the loads at the head, both are between LoopHead and
    /// BackEdge and this fails.
    #[test]
    fn a_uniform_read_in_a_loop_is_loaded_before_its_head() {
        use crate::shaderc::online::v3d::select::{select_fragment, Pressure, VOp, QUNIFORM_UNIFORM};
        const VS: &str = include_str!("../../../../tools/shaders/femtovg/main.vert");
        const FS: &str = include_str!("../../../../tools/shaders/femtovg/filter_image.nodiscard.frag");
        store::with_scratch(|a| {
            let fsrc = a.alloc_str(FS).unwrap();
            let vsrc = a.alloc_str(VS).unwrap();
            let _ = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
            let (fm, trips) = unrolled(&fr.module, a).expect("the loop's trip count");
            let fsp = prepare_fragment_with(fm, trips, 0, a).expect("prepares");
            let inputs = fragment_inputs(&fsp, a).expect("inputs");
            let sel = select_fragment(&fsp, inputs, 0, Pressure::None, a).expect("selects");
            let head = sel.ops.iter().position(|o| *o == VOp::LoopHead).expect("a loop");
            let edge = sel.ops.iter().position(|o| matches!(o, VOp::BackEdge { .. })).expect("a back edge");
            let user = |o: &VOp| matches!(o, VOp::Ldunif { u, .. } if u.contents == QUNIFORM_UNIFORM);
            let inside = sel.ops[head..edge].iter().filter(|o| user(o)).count();
            let before = sel.ops[..head].iter().filter(|o| user(o)).count();
            assert_eq!(inside, 0, "no user uniform loaded in the body: {:#?}", &sel.ops[head..edge]);
            assert!(before >= 2, "dir.x and dir.y before the head: {before}");
        })
        .expect("scratch");
    }

    /// One flag push serves every conditional move that follows it while
    /// it holds (select.rs Sel::push): a vec4 select on one condition is ONE
    /// push and four conditional moves. A texture look-up in between (its
    /// thread switch loses the flags) makes the same condition push again.
    /// And no plain move copies a register onto itself (sched.rs drops the
    /// select's else move that coalescing made `mov rX, rX`). Revert checks:
    /// without the reuse the first program has 4 pushes; reusing across the
    /// thread switch leaves the second with 1; without the drop the third
    /// has 6 `mov rX, rX`.
    #[test]
    fn a_flag_push_is_reused_until_something_changes_it() {
        const VS: &str = "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";
        const FS1: &str = "precision mediump float;\nvarying vec4 v;\nuniform vec4 u;\n\
            void main() { gl_FragColor = u.x > 0.5 ? v : v.wzyx; }\n";
        const FS2: &str = "precision mediump float;\nvarying vec4 v;\nuniform vec4 u;\nuniform sampler2D s;\n\
            void main() { float a = u.x > 0.5 ? v.x : v.y; vec4 t = texture2D(s, v.xy); \
            float b = u.x > 0.5 ? t.x : v.z; gl_FragColor = vec4(a, b, 0.0, 1.0); }\n";
        let pushes = |fs: &str| -> (usize, usize) {
            let mut out = (0, 0);
            with_program(VS, fs, Prim::Triangles, |p| {
                let p = p.expect("compiles");
                check_all(p);
                let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
                let push = ops.iter().filter(|o| o.contains(".push")).count();
                let self_mov = ops
                    .iter()
                    .filter(|o| {
                        o.split(';').any(|part| {
                            let w: Vec<&str> = part.split([' ', ',']).filter(|x| !x.is_empty()).collect();
                            w.len() == 3 && w[0] == "mov" && w[1] == w[2]
                        })
                    })
                    .count();
                out = (push, self_mov);
            });
            out
        };
        assert_eq!(pushes(FS1), (1, 0), "one push for the four lanes, and no mov rX, rX");
        assert_eq!(pushes(FS2).0, 2, "the same test pushed again after the look-up's thread switch");
        /* FemtoVG's texType shape: the texel's own value is the else of two
         * selects and dead after, so coalescing gives the else move one
         * register for source and destination (6 of them without the drop) */
        const FS3: &str = "precision mediump float;\nvarying vec4 v;\nuniform vec4 u;\nuniform sampler2D s;\n\
            void main() { vec4 c = texture2D(s, v.xy); int k = int(u.x); \
            if (k == 1) c = vec4(c.xyz * c.w, c.w); if (k == 2) c = vec4(c.x); gl_FragColor = c * u.y; }\n";
        assert_eq!(pushes(FS3), (2, 0), "one push per test, and no mov rX, rX");
    }

    /// `LodKey::ret32` (a depth texture's R32F storage, Mesa's return size
    /// 32): a keyed binding's look-up sets TMU Config Parameter 1's "Output
    /// Type 32-bit" (bit 0, v3d_packet.xml: P1 0x5 instead of 0x4), asks one
    /// word per channel in P0 (0xf for a vec4 instead of 0x3), reads them with
    /// one ldtmu each (4 instead of 2), and a channel is its word: the TLB
    /// write packs `r0, r1`, not the f16 halves `r0.l, r0.h`. A key bit at a
    /// binding the shader does not read changes nothing, and no key compiles
    /// the program as before. Revert checks: without the P1 bit or with the
    /// f16 word mask the first assertion fails; reading lane l / 2 fails the
    /// pack.
    #[test]
    fn a_ret32_binding_reads_one_32_bit_word_per_channel() {
        const VS: &str = "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";
        const FS: &str = "precision mediump float;\nvarying vec4 v;\nuniform sampler2D s;\n\
            void main() { gl_FragColor = texture2D(s, v.xy); }\n";
        let compile = |ret32: u32| -> (Vec<(u32, u32)>, Vec<String>) {
            let _serial = store::test_lock();
            let key = crate::shaderc::LodKey { ret32, ..crate::shaderc::LodKey::NONE };
            let old = crate::shaderc::set_compile_lod_key(key);
            let mut out = (Vec::new(), Vec::new());
            with_program(VS, FS, Prim::Triangles, |p| {
                let p = p.expect("compiles");
                check_all(p);
                out.0 =
                    p.fs.code
                        .uniforms
                        .iter()
                        .filter(|u| u.contents == 40 || u.contents == 41)
                        .map(|u| (u.contents, u.data))
                        .collect();
                out.1 = p.fs.code.instrs.iter().map(text).collect();
            });
            crate::shaderc::set_compile_lod_key(old);
            out
        };
        let count = |ops: &[String], w: &str| ops.iter().filter(|o| o.contains(w)).count();
        let (plain_cfg, plain) = compile(0);
        let (keyed_cfg, keyed) = compile(1);
        let (other_cfg, other) = compile(2);
        assert_eq!(plain_cfg, [(40, 0x3), (41, 0x4)], "f16: two words, 16-bit output");
        assert_eq!(keyed_cfg, [(40, 0xf), (41, 0x5)], "32-bit: four words, Output Type 32-bit");
        assert_eq!((count(&plain, "ldtmu"), count(&keyed, "ldtmu")), (2, 4));
        assert!(count(&plain, "r0.l, r0.h") == 1 && count(&keyed, ".l") == 0 && count(&keyed, ".h") == 0, "{keyed:?}");
        assert_eq!(count(&keyed, "vfpack tlb, r0, r1"), 1, "{keyed:?}");
        assert_eq!((other_cfg, other), (plain_cfg, plain), "a key bit at a binding not read changes nothing");
    }

    /// abs() is an input unpack on the operation that reads it (select.rs
    /// fold_unpacks, Mesa's copy propagation of the unpacking move): FSUB, FMAX
    /// and FMUL read `.abs` straight from the varying's register, and only
    /// FFLOOR, which has no ABS code, keeps the `fmov .abs`. The program
    /// runs in the interpreter on inputs of both signs, so a read that lost
    /// its `.abs` (or took it where the hardware has none) shows in the
    /// colour. Revert check: without the fold there are 4 `fmov .abs`.
    /// A second program is `max(a, a)`: FMAX of two identical inputs has no
    /// encoding (encode.rs: the order of the inputs is the opcode), and it
    /// was refused at scheduling (NotEncodable, ReadsBackAs) until prepare
    /// made it `a` (nir_opt_algebraic's fmax(a, a) rule).
    #[test]
    fn abs_is_the_readers_input_unpack() {
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
        const VS: &str = "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";
        const FS: &str = "precision highp float;\nvarying vec4 v;\n\
            void main() { gl_FragColor = vec4(1.0 - abs(v.x), max(abs(v.y), 0.25), abs(v.z) * v.y, \
            floor(abs(v.w))); }\n";
        const FS2: &str = "precision highp float;\nvarying vec4 v;\n\
            void main() { float a = abs(v.x); gl_FragColor = vec4(max(a, a), v.y, v.z, v.w); }\n";
        /* per lane and component: both signs, multiples of 1/8 */
        let value = |l: usize, i: usize| (l as f32 - 8.0 + 0.25 * i as f32) / 2.0;
        store::with_scratch(|a| {
            let vsrc = a.alloc_str(VS).unwrap();
            let fsrc = a.alloc_str(FS).unwrap();
            let v = lower::compile_to_ir(a, Stage::Vertex, vsrc).map_err(|e| e.msg).unwrap();
            let fr = lower::compile_to_ir(a, Stage::Fragment, fsrc).map_err(|e| e.msg).unwrap();
            let p = compile_program(&v.module, &fr.module, Prim::Triangles, TARGET, a).expect("compiles");
            check_all(&p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let fmov_abs =
                ops.iter().filter(|o| o.split(';').any(|x| x.contains("fmov") && x.contains(".abs"))).count();
            assert_eq!(fmov_abs, 1, "only FFLOOR's operand is moved: {ops:#?}");
            let reads: usize = ops.iter().map(|o| o.matches(".abs").count()).sum();
            assert_eq!(reads, 4, "FSUB, FMAX, FMUL and the move read .abs: {ops:#?}");
            let uniforms: Vec<u32> =
                p.fs.code
                    .uniforms
                    .iter()
                    .map(|u| match u.contents {
                        0 => u.data,
                        c => panic!("uniform contents {c} not modelled here"),
                    })
                    .collect();
            let varyings = (0..4)
                .map(|i| Varying { vary: core::array::from_fn(|l| value(l, i).to_bits()), c: [0; LANES] })
                .collect();
            let inputs = FsInputs {
                w: [1.0f32.to_bits(); LANES],
                w_centroid: [1.0f32.to_bits(); LANES],
                z: [0; LANES],
                x: [0; LANES],
                y: [0; LANES],
                back_facing: [false; LANES],
                varyings,
            };
            let env = Env { threads: p.fs.threads, ..Env::default() };
            let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
                .expect("runs");
            let h = |x: f32| u32::from(sim::f32_to_half(x));
            for l in 0..LANES {
                let [x, y, z, w] = [0, 1, 2, 3].map(|i| value(l, i));
                let c = [1.0 - x.abs(), y.abs().max(0.25), z.abs() * y, w.abs().floor()];
                let want = [h(c[0]) | h(c[1]) << 16, h(c[2]) | h(c[3]) << 16];
                let got = [o.tlb[0].value[l].unwrap(), o.tlb[1].value[l].unwrap()];
                assert_eq!(got, want, "lane {l}: inputs {x} {y} {z} {w}");
            }
            assert!((0..LANES).any(|l| value(l, 0) < 0.0) && (0..LANES).any(|l| value(l, 0) > 0.0), "both signs");
        })
        .expect("scratch");
        with_program(VS, FS2, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            assert!(!ops.iter().any(|o| o.contains("fmax")), "max(a, a) is a: {ops:#?}");
        });
    }

    /// Runs the ES 3.00 fragment shader `fs` (reading `in vec4 v`) in the
    /// interpreter on 16 lanes, varying component c of lane l being
    /// `input(l)[c]`, and checks each lane's colour against `want` of those
    /// inputs (the RT is f16, so every wanted value is exact in f16).
    #[cfg(not(feature = "report-es2"))]
    fn run_es3_fs(fs: &str, input: impl Fn(usize) -> [f32; 4], want: impl Fn([f32; 4]) -> [f32; 4]) {
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
        const VS: &str = "#version 300 es\nin vec4 p;\nout vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";
        with_program(VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let uniforms: Vec<u32> =
                p.fs.code
                    .uniforms
                    .iter()
                    .map(|u| match u.contents {
                        0 => u.data,
                        c => panic!("uniform contents {c} not modelled here"),
                    })
                    .collect();
            let varyings =
                p.fs.input_slots
                    .iter()
                    .map(|&(_, comp)| Varying {
                        vary: core::array::from_fn(|l| input(l)[comp as usize].to_bits()),
                        c: [0; LANES],
                    })
                    .collect();
            let inputs = FsInputs {
                w: [1.0f32.to_bits(); LANES],
                w_centroid: [1.0f32.to_bits(); LANES],
                z: [0; LANES],
                x: [0; LANES],
                y: [0; LANES],
                back_facing: [false; LANES],
                varyings,
            };
            let env = Env { threads: p.fs.threads, ..Env::default() };
            let o = sim::run(p.fs.code.words, &uniforms, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
                .expect("runs");
            let h = |x: f32| u32::from(sim::f32_to_half(x));
            for l in 0..LANES {
                let c = want(input(l));
                let wanted = [h(c[0]) | h(c[1]) << 16, h(c[2]) | h(c[3]) << 16];
                let got = [o.tlb[0].value[l].unwrap(), o.tlb[1].value[l].unwrap()];
                assert_eq!(got, wanted, "lane {l}: inputs {:?}, want {c:?}: {fs}", input(l));
            }
        });
    }

    /// GLSL ES 3.00's `trunc`, integer shifts, integer and unsigned
    /// min/max, `~` and the unsigned orderings, each Mesa's one operation
    /// (nir_to_vir.c: FTRUNC, SHL, ASR, SHR, MIN, MAX, UMIN, UMAX, NOT;
    /// ult32 / uge32 as SUB.pushc), run in the interpreter on both signs
    /// (the shift right of a negative int is arithmetic, of a uint
    /// logical). Revert check: with these out of prepare.rs SELECTED the
    /// programs are refused (`NotYet("Ftrunc")`, `NotYet("Ishl")`, the
    /// dEQP corpus's 8 indexing and 2 constant_expressions refusals); with
    /// Ule's operands in Ult's order the third program fails at lane 0.
    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn trunc_shifts_and_integer_min_max_are_one_operation_each() {
        let head = "#version 300 es\nprecision highp float;\nprecision highp int;\nin vec4 v;\n\
                    layout(location = 0) out vec4 o;\nvoid main() { int i = int(v.y); uint u = uint(v.z); ";
        /* x: non-integers of both signs; y: -8..7; z: 0..15; w: 3 */
        let input = |l: usize| [(l as f32 - 8.0) * 0.75 + 0.125, l as f32 - 8.0, l as f32, 3.0];
        let trunc_shift = std::format!(
            "{head} o = vec4(trunc(v.x), float(i << 2), float(i >> 1), float(u >> 1u) + float(i << int(v.w))); }}"
        );
        run_es3_fs(&trunc_shift, input, |[x, y, z, w]| {
            let (i, u) = (y as i32, z as u32);
            [x.trunc(), (i << 2) as f32, (i >> 1) as f32, (u >> 1) as f32 + (i << w as i32) as f32]
        });
        let min_max = std::format!(
            "{head} o = vec4(float(min(i, -2)), float(max(i, 1)), float(min(u, 5u)), float(max(u, 9u))); }}"
        );
        run_es3_fs(&min_max, input, |[_, y, z, _]| {
            let (i, u) = (y as i32, z as u32);
            [i.min(-2) as f32, i.max(1) as f32, u.min(5) as f32, u.max(9) as f32]
        });
        let not_cmp =
            std::format!("{head} o = vec4(float(~i), u < 5u ? 1.0 : 0.0, u <= 5u ? 1.0 : 0.0, float(~u & 7u)); }}");
        run_es3_fs(&not_cmp, input, |[_, y, z, _]| {
            let (i, u) = (y as i32, z as u32);
            [(!i) as f32, f32::from(u8::from(u < 5)), f32::from(u8::from(u <= 5)), (!u & 7) as f32]
        });
        assert!(input(0)[0] < 0.0 && input(15)[0] > 0.0 && input(0)[1] < 0.0, "both signs");
        /* gl_test 196's fragment shader, from the file it compiles, on the
         * case's v = (2.75, -6, 200, 0): (0.5, 0.25, 45/64, 50/64) */
        run_es3_fs(
            include_str!("../../../../tools/shaders/gltest/es3trunc_fs.frag"),
            |_| [2.75, -6.0, 200.0, 0.0],
            |_| [0.5, 0.25, 45.0 / 64.0, 50.0 / 64.0],
        );
    }

    /// A look-up inside a uniform branch's arm (step (b)): the `if` is a
    /// branch, as Mesa never flattens an `if` with a look-up in an arm
    /// (nir_opt_peephole_select), and the look-up's switch, which the arm
    /// may skip, is not the last one (Mesa's last_thrsw_at_top_level is
    /// false in control flow): a new pair follows the `if`, after every
    /// forward branch's target. Then gl_test 148's own shader
    /// (es100/texbranch.frag) at 4 threads, its loop kept, in the
    /// interpreter with the case's three draws: each gives the case's
    /// colour, and a look-up in an untaken arm or a pass past the break is
    /// not requested (8, 1 and 50 look-ups). Revert checks: with the in-arm
    /// switch taken as the last one, verify refuses the program ("the
    /// last-thread-switch pair where a forward branch skips it"); with
    /// look-ups kept out of branch arms, no branch. verify.rs's two new
    /// refusals are also checked directly, on the compiled programs with
    /// the pair moved into the arm and into the loop body.
    #[test]
    fn a_look_up_in_a_uniform_branch_switches_again_after_it() {
        let vs = "attribute vec2 a_uv; varying vec2 v_uv; \
                  void main() { v_uv = a_uv; gl_Position = vec4(a_uv, 0.0, 1.0); }";
        let fs = "precision mediump float; uniform float u; uniform sampler2D t; varying vec2 v_uv; \
                  void main() { vec4 c = vec4(0.25); if (u > 0.5) { c = texture2D(t, v_uv); } gl_FragColor = c; }";
        with_program(vs, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            assert!(!c.fwd_branches.is_empty(), "the `if` is a branch");
            let (thrsw, ldtmu) = thrsw_shape(c);
            let end = c.fwd_branches.iter().map(|&(_, t)| t as usize).max().unwrap();
            let pair: Vec<usize> = thrsw.windows(2).filter(|w| w[1] == w[0] + 1).map(|w| w[0]).collect();
            assert_eq!(pair.len(), 1, "one last-thread-switch pair: {thrsw:?}");
            assert!(pair[0] >= end, "the pair after the arm (targets end at {end}): {thrsw:?}");
            assert!(ldtmu.is_some_and(|l| l < end), "the look-up inside the arm: {ldtmu:?}, {end}");
            assert!(!p.fs.single_seg);
            /* verify.rs's rule, directly: the same program with its pair moved
             * between the branch's delay slots and its target is refused */
            let rule = |prog: &[Instr], edges: &[(u32, u32)], fwd: &[(u32, u32)]| {
                branches_are_structured(prog, edges, fwd).map_err(|e| e.rule)
            };
            assert_eq!(rule(c.instrs, c.back_edges, c.fwd_branches), Ok(()));
            let (b, t) = c.fwd_branches[0];
            let at = pair_site(c.instrs, b as usize + 4, t as usize).expect("two ALU words in the arm");
            assert_eq!(
                rule(&pair_moved(c.instrs, at), c.back_edges, c.fwd_branches),
                Err("the last-thread-switch pair where a forward branch skips it")
            );
        });
        /* gl_test 148 texbranch: its shader in the interpreter, the case's
         * three draws, with its two one-texel textures (ta binding 0, tb 1) */
        use crate::shaderc::online::texref::{Level, MinFilter};
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Texture, LANES};
        static TEXBRANCH_VS: &str = include_str!("../../../../tools/shaders/es100/deriv.vert");
        static TEXBRANCH_FS: &str = include_str!("../../../../tools/shaders/es100/texbranch.frag");
        const TA: [f32; 4] = [0.8, 0.6, 0.0, 1.0];
        const TB: [f32; 4] = [0.2, 0.0, 1.0, 1.0];
        let one_texel = |c: [f32; 4]| Texture {
            cube: false,
            min: MinFilter::Nearest,
            levels: vec![Level { width: 1, height: 1, faces: vec![vec![c]] }],
        };
        with_program(TEXBRANCH_VS, TEXBRANCH_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            assert_eq!(p.fs.threads, 4, "4 threads, as Mesa's filter_image");
            assert!(!c.back_edges.is_empty(), "the 24-pass loop is kept as a loop");
            /* verify.rs's rule: the pair moved into the loop body is refused */
            let (b, h) = c.back_edges[0];
            let at = pair_site(c.instrs, h as usize, b as usize).expect("two ALU words in the loop body");
            assert_eq!(
                branches_are_structured(&pair_moved(c.instrs, at), c.back_edges, c.fwd_branches).map_err(|e| e.rule),
                Err("the last-thread-switch pair in a loop body")
            );
            assert!(c.fwd_branches.len() >= 4, "the two ifs and the loop's break guard branch: {:?}", c.fwd_branches);
            /* (u, the colour the case reads back) */
            let draws: [([f32; 4], [f32; 3]); 3] = [
                ([1.0, 1.0, 4.0, 0.0], [TA[0], TA[1], 6.0 / 48.0]),
                ([0.0, 0.0, 1.0, 0.0], [TB[0], 0.0, 0.0]),
                ([0.0, 1.0, 25.0, 0.0], [TB[0], TA[1], 1.0]),
            ];
            let mut executed = Vec::new();
            for (u, want) in draws {
                let stream: Vec<u32> = c
                    .uniforms
                    .iter()
                    .map(|w| match w.contents {
                        0 | 40 | 41 => w.data,
                        1 => u[w.data as usize].to_bits(),
                        k => panic!("uniform contents {k} not modelled here"),
                    })
                    .collect();
                let inputs = FsInputs {
                    w: [1.0f32.to_bits(); LANES],
                    w_centroid: [1.0f32.to_bits(); LANES],
                    z: [0; LANES],
                    x: [0; LANES],
                    y: [0; LANES],
                    back_facing: [false; LANES],
                    varyings: vec![],
                };
                let env = Env { threads: p.fs.threads, textures: vec![one_texel(TA), one_texel(TB)], ..Env::default() };
                let o =
                    sim::run(c.words, &stream, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env).expect("runs");
                let h = |x: f32| u32::from(sim::f32_to_half(x));
                let rg = h(want[0]) | h(want[1]) << 16;
                let ba = h(want[2]) | h(1.0) << 16;
                let got: Vec<_> = o.tlb.iter().map(|t| t.value).collect();
                assert_eq!(got, vec![[Some(rg); LANES], [Some(ba); LANES]], "gl_test 148, u = {u:?}");
                executed.push((o.instructions, o.tmu_lookups));
            }
            /* the draw that takes neither `if` nor a pass requests one look-up
             * (the else arm's); the full one 1 + 1 + 48 */
            assert_eq!(
                executed.iter().map(|e| e.1).collect::<Vec<_>>(),
                vec![1 + 1 + 6, 1, 1 + 1 + 48],
                "{executed:?}"
            );
        });
    }

    /// A THRSW goes onto one of the last three words already placed, which
    /// become its delay slots (sched.rs merge_back, Mesa's emit_thrsw), and
    /// a fragment shader's last look-up switch is its last-thread-switch
    /// pair (select.rs Sel::last_thrsw, Mesa's vir_restore_last_thrsw). For
    /// es100/textured that is Mesa's own shape (femtovg fill_image, Mesa
    /// words 15-18): the pair on the second `wrtmuc` and the T write, the S
    /// write in the last slot, the first `ldtmu` right after; three THRSW
    /// signals in all (the pair and the thrend). A fragment shader with no
    /// look-up gets a pair of its own, on the last words of its body.
    /// Revert checks: with every THRSW in a word of its own the pair is not
    /// on the `wrtmuc`; with a new pair at the end the count is not 3.
    #[test]
    fn a_thread_switch_takes_the_words_before_it_as_delay_slots() {
        static TEXTURED_VS: &str = include_str!("../../../../tools/shaders/es100/textured.vert");
        static TEXTURED_FS: &str = include_str!("../../../../tools/shaders/es100/textured.frag");
        let thrsw_at = |ops: &[String]| -> Vec<usize> {
            ops.iter().enumerate().filter(|(_, o)| o.contains("thrsw")).map(|(i, _)| i).collect()
        };
        /* Mesa's shape on the selected order (order.rs off); the list
         * order (on, what ships) moves both wrtmuc signals up to the
         * varyings, and the pair lands on the words before T, with the same
         * slots */
        for off in [true, false] {
            super::super::order::ORDER_OFF.with(|h| h.set(off));
            with_program(TEXTURED_VS, TEXTURED_FS, Prim::Triangles, |p| {
                let p = p.expect("compiles");
                check_all(p);
                let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
                let at = |s: &str| ops.iter().position(|o| o.contains(s)).unwrap();
                let sw = thrsw_at(&ops);
                assert_eq!(sw.len(), 3, "the look-up's pair and the thrend: {ops:#?}");
                assert_eq!(sw[1], sw[0] + 1, "a pair: {ops:#?}");
                if off {
                    assert!(ops[sw[0]].contains("wrtmuc"), "on the second wrtmuc: {ops:#?}");
                }
                assert_eq!(at("mov tmut"), sw[0] + 1, "T in slot 1: {ops:#?}");
                assert_eq!(at("mov tmus"), sw[0] + 2, "S in slot 2: {ops:#?}");
                assert_eq!(at("ldtmu"), sw[0] + 3, "the result read right after the slots: {ops:#?}");
            });
            super::super::order::ORDER_OFF.with(|h| h.set(false));
        }
        let fs = "precision mediump float;\nvarying vec4 v_color;\n\
            void main() { gl_FragColor = v_color * v_color.w + v_color.yzwx; }\n";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let sw = thrsw_at(&ops);
            assert_eq!(sw.len(), 3, "a pair of its own and the thrend: {ops:#?}");
            assert_eq!(sw[1], sw[0] + 1, "a pair: {ops:#?}");
            assert!(
                ops[sw[0]].split(';').take(2).any(|x| x.trim() != "nop"),
                "the pair is on a word of the body, not a word of its own: {ops:#?}"
            );
        });
    }

    /// A large `if` on a uniform condition is a branch round each arm
    /// (prepare.rs Walk::if_convert, select.rs SOp::IfBranch, sched.rs
    /// VOp::FwdBranch; Mesa's ntq_emit_uniform_if): `bu.anynap` to the else
    /// arm, `bu` over it at the end of the then arm, and one register per
    /// value the arms leave different. The program runs in the interpreter
    /// with the condition true and false; the inputs are multiples of 1/8,
    /// so every product is exact and the host computes the same colour. Each
    /// run executes fewer instructions than the program holds (an arm is
    /// skipped). Hand mutants that must be caught: the first branch's
    /// condition inverted (each run takes the other arm), and each branch
    /// with its uniform stream not moved (`ub` off), on the run where it is
    /// taken: the code after it reads the uniform of the arm it skipped (on
    /// the other run the mutant is equivalent). Controls: the
    /// same `if` with small arms, and with a varying condition, stays a
    /// select (no branch); and a large one whose values nothing reads leaves
    /// no branch at all (prepare.rs compact, femtovg's stencil: without it
    /// the empty skeleton cost 6 to 10 instructions). A third program reads gl_FragCoord.x in the
    /// then arm and again after the `if`: the second read must not reuse the
    /// first (prepare.rs: FXCD is computed where it stands, so it is reused
    /// from an open region only), or the else path reads a register nothing
    /// wrote. Revert check: with FragCoord deduplicated from anywhere, as
    /// before, the else run of that program fails. And verify.rs's rules for
    /// forward branches, each broken one way on the first program. Last,
    /// gl_test 142's own shader (es100/unibranch.frag) is a branch, and each
    /// arm gives the colour the case expects.
    #[test]
    fn a_large_uniform_if_is_a_branch_round_each_arm() {
        use crate::shaderc::online::v3d::isa::{Branch, BranchDest};
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
        const VS: &str = "attribute vec4 p;\nvarying vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";
        let fs = |cond: &str, then_: &str, else_: &str| {
            std::format!(
                "precision highp float;\nvarying vec4 v;\nuniform vec4 u;\nvoid main() {{\n vec4 c;\n \
                 if ({cond}) {{ {then_} }} else {{ {else_} }}\n gl_FragColor = c * u.y;\n}}\n"
            )
        };
        /* each arm reads a uniform of its own (u.w, u.z): skipping either
         * one moves the uniform stream */
        const THEN: &str = "c = v * v.yzwx + v.zwxy * v.wxyz - v * u.w; c = c * c.yzwx + v;";
        const ELSE: &str = "c = v.wzyx * 2.0 - v * v.xxyy + v.zzww * u.z; c = c * c + v.yzwx;";
        const U_Z: f32 = 0.5;
        const U_W: f32 = 0.25;
        let value = |l: usize, i: usize| ((l as f32) - 7.0 + 2.0 * i as f32) / 8.0;
        let host = |l: usize, then_: bool| -> [f32; 4] {
            let v: [f32; 4] = core::array::from_fn(|i| value(l, i));
            let sw = |k: [usize; 4]| k.map(|i| v[i]);
            let (yzwx, zwxy, wxyz) = (sw([1, 2, 3, 0]), sw([2, 3, 0, 1]), sw([3, 0, 1, 2]));
            let (wzyx, xxyy, zzww) = (sw([3, 2, 1, 0]), sw([0, 0, 1, 1]), sw([2, 2, 3, 3]));
            if then_ {
                let c: [f32; 4] = core::array::from_fn(|i| v[i] * yzwx[i] + zwxy[i] * wxyz[i] - v[i] * U_W);
                core::array::from_fn(|i| c[i] * c[(i + 1) % 4] + v[i])
            } else {
                let c: [f32; 4] = core::array::from_fn(|i| wzyx[i] * 2.0 - v[i] * xxyy[i] + zzww[i] * U_Z);
                core::array::from_fn(|i| c[i] * c[i] + yzwx[i])
            }
        };
        /* the colour words of one run, or why it stopped */
        let fx = |l: usize| l as f32 + 0.5;
        let run = |words: &[u64], uniforms: &[Uniform], threads: u32, u: [f32; 4]| {
            let stream: Vec<u32> = uniforms
                .iter()
                .map(|w| match w.contents {
                    0 => w.data,
                    1 => u[w.data as usize].to_bits(),
                    c => panic!("uniform contents {c} not modelled here"),
                })
                .collect();
            let inputs = FsInputs {
                w: [1.0f32.to_bits(); LANES],
                w_centroid: [1.0f32.to_bits(); LANES],
                z: [0; LANES],
                x: core::array::from_fn(|l| fx(l).to_bits()),
                y: [0; LANES],
                back_facing: [false; LANES],
                varyings: (0..4)
                    .map(|i| Varying { vary: core::array::from_fn(|l| value(l, i).to_bits()), c: [0; LANES] })
                    .collect(),
            };
            let env = Env { threads, ..Env::default() };
            sim::run(words, &stream, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env)
                .map(|o| (o.tlb.iter().map(|t| t.value).collect::<Vec<_>>(), o.instructions))
        };
        let h = |x: f32| Some(u32::from(sim::f32_to_half(x)));
        let want_with = |c: Vec<[f32; 4]>| -> Vec<[Option<u32>; LANES]> {
            [0, 2]
                .iter()
                .map(|&k| core::array::from_fn(|l| Some(h(c[l][k]).unwrap() | h(c[l][k + 1]).unwrap() << 16)))
                .collect()
        };
        let want = |then_: bool| want_with((0..LANES).map(|l| host(l, then_)).collect());
        with_program(VS, &fs("u.x > 0.5", THEN, ELSE), Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            let ops: Vec<String> = c.instrs.iter().map(text).collect();
            assert_eq!(c.fwd_branches.len(), 2, "to the else arm, and over it: {ops:#?}");
            assert!(ops.iter().any(|o| o.starts_with("bu.anynap")), "{ops:#?}");
            assert!(ops.iter().any(|o| o.starts_with("bu ")), "{ops:#?}");
            for then_ in [true, false] {
                let u = [if then_ { 1.0 } else { 0.0 }, 1.0, U_Z, U_W];
                let (got, executed) = run(c.words, c.uniforms, p.fs.threads, u).expect("runs");
                assert_eq!(got, want(then_), "then arm taken: {then_}");
                assert!(executed < c.instrs.len(), "{executed} of {} executed: an arm is skipped", c.instrs.len());
            }
            /* the mutants */
            let mutate = |b: u32, f: &dyn Fn(&mut Branch)| {
                let mut instrs = c.instrs.to_vec();
                let Instr::Branch(br) = &mut instrs[b as usize] else { panic!("a branch") };
                f(br);
                let mut words = std::vec![0u64; instrs.len()];
                encode_program(&instrs, &mut words).expect("encodes");
                words
            };
            let (to_else, over_else) = (c.fwd_branches[0].0, c.fwd_branches[1].0);
            let flipped = mutate(to_else, &|br| {
                br.cond = if br.cond == BranchCond::AnyNA { BranchCond::AnyA } else { BranchCond::AnyNA }
            });
            let unmoved = |b: u32| {
                mutate(b, &|br| {
                    br.ub = false;
                    br.bdu = BranchDest::Abs;
                })
            };
            for then_ in [true, false] {
                let u = [if then_ { 1.0 } else { 0.0 }, 1.0, U_Z, U_W];
                /* caught: the run stops, or its colour is not the right one */
                let caught = |words: &[u64]| run(words, c.uniforms, p.fs.threads, u).map(|r| r.0) != Ok(want(then_));
                assert!(caught(&flipped), "the inverted branch, then {then_}");
                let taken = if then_ { over_else } else { to_else };
                assert!(caught(&unmoved(taken)), "the stream not moved, then {then_}");
            }
            /* the verifier's rules */
            let rule = |prog: &[Instr], edges: &[(u32, u32)], fwd: &[(u32, u32)]| {
                branches_are_structured(prog, edges, fwd).map_err(|e| e.rule)
            };
            let (b, t) = c.fwd_branches[0];
            assert_eq!(rule(c.instrs, &[], c.fwd_branches), Ok(()));
            assert_eq!(
                rule(c.instrs, &[], &[]),
                Err("a branch that is neither a loop's back edge nor a forward branch")
            );
            assert_eq!(
                rule(c.instrs, &[], &[(b, t + 1), c.fwd_branches[1]]),
                Err("a forward branch whose target is not its label")
            );
            let mut prog = c.instrs.to_vec();
            let Instr::Branch(br) = &mut prog[b as usize] else { panic!("a branch") };
            br.offset = (-8i32) as u32;
            assert_eq!(
                rule(&prog, &[], &[(b, b + 3), c.fwd_branches[1]]),
                Err("a forward branch that is not forward inside the program")
            );
            /* a loop body (as recorded) holding the target and not the branch */
            assert_eq!(rule(c.instrs, &[(t, t)], c.fwd_branches), Err("a forward branch into a loop body"));
        });
        /* gl_FragCoord.x in the then arm and after the `if` */
        let fs_xy = std::format!(
            "precision highp float;\nvarying vec4 v;\nuniform vec4 u;\nvoid main() {{\n vec4 c;\n \
             if (u.x > 0.5) {{ {THEN} c.x = c.x + gl_FragCoord.x; }} else {{ {ELSE} }}\n \
             gl_FragColor = c * u.y + vec4(gl_FragCoord.x);\n}}\n"
        );
        with_program(VS, &fs_xy, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            assert_eq!(c.fwd_branches.len(), 2);
            for then_ in [true, false] {
                let u = [if then_ { 1.0 } else { 0.0 }, 1.0, U_Z, U_W];
                let got = run(c.words, c.uniforms, p.fs.threads, u).map(|r| r.0);
                let colour = (0..LANES)
                    .map(|l| {
                        let mut k = host(l, then_);
                        if then_ {
                            k[0] += fx(l);
                        }
                        k.map(|x| x + fx(l))
                    })
                    .collect();
                assert_eq!(got, Ok(want_with(colour)), "gl_FragCoord after the if, then arm taken: {then_}");
            }
        });
        /* gl_test 142 unibranch: its shader, both arms */
        static UNIBRANCH_VS: &str = include_str!("../../../../tools/shaders/es100/deriv.vert");
        static UNIBRANCH_FS: &str = include_str!("../../../../tools/shaders/es100/unibranch.frag");
        let unibranch = |then_: bool, y: f32, z: f32, w: f32| -> [f32; 4] {
            let clamp = |x: f32| x.clamp(0.0, 1.0);
            if then_ {
                let a = [y, z, w, 0.5];
                let b: [f32; 4] = core::array::from_fn(|i| a[i] * a[(i + 1) % 4] + a[(i + 2) % 4] * a[(i + 3) % 4]);
                let c: [f32; 4] = core::array::from_fn(|i| b[i] * 0.5 - a[i] * 0.25 + a[3 - i] * 0.125);
                core::array::from_fn(|i| clamp(c[i] * c[(i + 1) % 4] + a[i] * 0.5))
            } else {
                let a = [w, z, y, 0.25];
                let b: [f32; 4] = core::array::from_fn(|i| a[i] * a[i] + a[(i + 1) % 4] * 0.5);
                let c: [f32; 4] =
                    core::array::from_fn(|i| b[i] * a[(i + 2) % 4] - a[i] * 0.125 + a[(i + 3) % 4] * 0.25);
                core::array::from_fn(|i| clamp(c[i] + c[(i + 1) % 4] * 0.25))
            }
        };
        with_program(UNIBRANCH_VS, UNIBRANCH_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            assert_eq!(c.fwd_branches.len(), 2, "gl_test 142's shader is a branch");
            for then_ in [true, false] {
                let (got, _) =
                    run(c.words, c.uniforms, p.fs.threads, [f32::from(u8::from(then_)), 0.9, 0.3, 0.6]).expect("runs");
                let k = unibranch(then_, 0.9, 0.3, 0.6);
                let colour = (0..LANES).map(|_| [k[0], k[1], k[2], 1.0]).collect();
                assert_eq!(got, want_with(colour), "gl_test 142, then arm taken: {then_}");
            }
        });
        /* controls: no branch */
        with_program(VS, &fs("u.x > 0.5", "c = v * 2.0;", "c = v * 0.5;"), Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!(p.fs.code.fwd_branches.is_empty(), "small arms are selects");
        });
        with_program(VS, &fs("v.x > 0.5", THEN, ELSE), Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!(p.fs.code.fwd_branches.is_empty(), "a varying condition is a select");
        });
        let dead = "precision highp float;\nvarying vec4 v;\nuniform vec4 u;\nvoid main() {\n vec4 c;\n \
            if (u.x > 0.5) { c = v * v.yzwx + v.zwxy * v.wxyz; c = c * c.yzwx + v; } \
            else { c = v.wzyx * 2.0 - v * v.xxyy + v.zzww; c = c * c + v.yzwx; }\n \
            gl_FragColor = v;\n}\n";
        with_program(VS, dead, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert!(p.fs.code.fwd_branches.is_empty(), "a branch whose values are all dead goes whole");
        });
    }

    /// Varyings take one word each, as in Mesa's listings: each `ldvary`
    /// moves into the word of the varying before it's FMUL (sched.rs
    /// ldvary_back, Mesa's fixup_pipelined_ldvary), which works because an
    /// ldvary's register is taken round-robin (alloc.rs). es100/textured's
    /// six inputs are read in words 0..=5, one per word, and its FADDs
    /// follow two words after their ldvary. Revert check: without the move
    /// every other word has one (0, 2, 4, ...).
    #[test]
    fn varyings_take_one_word_each() {
        static TEXTURED_VS: &str = include_str!("../../../../tools/shaders/es100/textured.vert");
        static TEXTURED_FS: &str = include_str!("../../../../tools/shaders/es100/textured.frag");
        with_program(TEXTURED_VS, TEXTURED_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            assert_eq!(p.fs.num_inputs, 6);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            let at: Vec<usize> = ops.iter().enumerate().filter(|(_, o)| o.contains("ldvary")).map(|(i, _)| i).collect();
            assert_eq!(at, [0, 1, 2, 3, 4, 5], "one ldvary per word: {ops:#?}");
            for w in 2..=7 {
                assert!(ops[w].starts_with("fadd ") && ops[w].contains(", r5"), "word {w} is a FADD of r5: {ops:#?}");
            }
        });
    }

    /// textureSize of a 2D sampler (prepare.rs, Mesa's ntq_emit_txs): each
    /// lane is the base level's size, a uniform word the driver fills
    /// (QUNIFORM_TEXTURE_WIDTH / _HEIGHT, data = the sampler's binding),
    /// minified as Mesa's ntq_minify does, max(size >> lod, 1). Run in the
    /// interpreter for a 64x16 texture at levels 0, 1 and 5; level 5 is past
    /// the height's chain (16 >> 5 = 0), so the clamp to 1 shows.
    #[cfg(not(feature = "report-es2"))]
    #[test]
    fn texture_size_is_the_base_size_minified() {
        use crate::shaderc::online::v3d::select::{QUNIFORM_TEXTURE_HEIGHT, QUNIFORM_TEXTURE_WIDTH};
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, LANES};
        const VS: &str = "#version 300 es\nlayout(location = 0) in vec4 p;\nvoid main() { gl_Position = p; }\n";
        const FS: &str = "#version 300 es\nprecision highp float;\nuniform highp sampler2D t;\nuniform int lod;\n\
            layout(location = 0) out vec4 o;\n\
            void main() { ivec2 s = textureSize(t, lod); o = vec4(vec2(s) / 64.0, 0.0, 1.0); }\n";
        with_program(VS, FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            let words: Vec<(u32, u32)> = c.uniforms.iter().map(|u| (u.contents, u.data)).collect();
            assert!(
                words.contains(&(QUNIFORM_TEXTURE_WIDTH, 0)) && words.contains(&(QUNIFORM_TEXTURE_HEIGHT, 0)),
                "{words:?}"
            );
            let h = |x: f32| u32::from(sim::f32_to_half(x));
            for (lod, w, ht) in [(0i32, 64.0f32, 16.0f32), (1, 32.0, 8.0), (5, 2.0, 1.0)] {
                let stream: Vec<u32> = c
                    .uniforms
                    .iter()
                    .map(|u| match u.contents {
                        0 => u.data,
                        1 => lod as u32,
                        QUNIFORM_TEXTURE_WIDTH => 64,
                        QUNIFORM_TEXTURE_HEIGHT => 16,
                        k => panic!("uniform contents {k} not modelled here"),
                    })
                    .collect();
                let inputs = FsInputs {
                    w: [1.0f32.to_bits(); LANES],
                    w_centroid: [1.0f32.to_bits(); LANES],
                    z: [0; LANES],
                    x: [0; LANES],
                    y: [0; LANES],
                    back_facing: [false; LANES],
                    varyings: std::vec::Vec::new(),
                };
                let env = Env { threads: p.fs.threads, ..Env::default() };
                let o =
                    sim::run(c.words, &stream, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env).expect("runs");
                let want = [h(w / 64.0) | h(ht / 64.0) << 16, h(0.0) | h(1.0) << 16];
                for l in 0..LANES {
                    assert_eq!([o.tlb[0].value[l], o.tlb[1].value[l]], want.map(Some), "lod {lod}, lane {l}");
                }
            }
        });
    }

    /// dEQP-GLES2 uniform_api's nested_structs_arrays mat4/mat2 "both"
    /// program (es2fUniformApiTests.cpp's generator: struct arrays of mat4
    /// and mat2, every leaf compared in both stages), compiled with the list
    /// scheduler's order (order.rs) and without it: its scratch high-water
    /// with the order stays under twice the one without, and both compile.
    /// 26eca726 took 4.4x (2,995,032 against 680,440 bytes) and larger
    /// programs of the group ran the 4 MiB scratch out on the Pi 4 (UC6A:
    /// "out of compiler memory on this fragment shader"). Revert check: an
    /// edge for every earlier operation in order.rs runs the 4 MiB scratch
    /// out on this program ("compiles: OutOfScratch").
    #[test]
    fn deqps_nested_struct_arrays_program_keeps_the_order_passes_scratch_small() {
        const DECL: &str = "struct subSubStructType { mediump mat4 mss0; mediump mat2 mss1; };\n\
            struct subStructType { mediump mat2 ms0; mediump mat4 ms1[2]; subSubStructType ms2[2]; };\n\
            struct structType { mediump mat4 m0; subStructType m1; mediump mat2 m2; };\n\
            uniform structType u_var;\n\
            mediump float compare_float(mediump float a, mediump float b) { return abs(a - b) < 0.05 ? 1.0 : 0.0; }\n\
            mediump float compare_vec2(mediump vec2 a, mediump vec2 b) { return compare_float(a.x, b.x)*compare_float(a.y, b.y); }\n\
            mediump float compare_vec4(mediump vec4 a, mediump vec4 b) { return compare_float(a.x, b.x)*compare_float(a.y, b.y)*compare_float(a.z, b.z)*compare_float(a.w, b.w); }\n\
            mediump float compare_mat2(mediump mat2 a, mediump mat2 b) { return compare_vec2(a[0], b[0])*compare_vec2(a[1], b[1]); }\n\
            mediump float compare_mat4(mediump mat4 a, mediump mat4 b) { return compare_vec4(a[0], b[0])*compare_vec4(a[1], b[1])*compare_vec4(a[2], b[2])*compare_vec4(a[3], b[3]); }\n";
        let leaves: [(&str, u32); 9] = [
            ("u_var.m0", 4),
            ("u_var.m1.ms0", 2),
            ("u_var.m1.ms1[0]", 4),
            ("u_var.m1.ms1[1]", 4),
            ("u_var.m1.ms2[0].mss0", 4),
            ("u_var.m1.ms2[0].mss1", 2),
            ("u_var.m1.ms2[1].mss0", 4),
            ("u_var.m1.ms2[1].mss1", 2),
            ("u_var.m2", 2),
        ];
        let compares = |var: &str| -> String {
            let mut s = String::new();
            for (k, (name, n)) in leaves.iter().enumerate() {
                let vals: Vec<String> =
                    (0..n * n).map(|i| std::format!("{:.2}", ((k as u32 * 7 + i) % 19) as f32 / 10.0 - 0.9)).collect();
                s += &std::format!("\t{var} *= compare_mat{n}({name}, mat{n}({}));\n", vals.join(", "));
            }
            s
        };
        let vs = std::format!(
            "attribute highp vec4 a_position;\nvarying mediump float v_vtxOut;\n{DECL}\nvoid main (void)\n{{\n    gl_Position = a_position;\n    v_vtxOut = 1.0;\n{}}}\n",
            compares("v_vtxOut")
        );
        let fs = std::format!(
            "varying mediump float v_vtxOut;\n{DECL}\nvoid main (void)\n{{\n    mediump float result = v_vtxOut;\n{}    gl_FragColor = vec4(result, result, result, 1.0);\n}}\n",
            compares("result")
        );
        let peak = |off: bool| {
            super::super::order::ORDER_OFF.with(|h| h.set(off));
            let p = store::with_scratch(|a| {
                let v = lower::compile_to_ir(a, Stage::Vertex, a.alloc_str(&vs).unwrap()).map_err(|e| e.msg).unwrap();
                let f = lower::compile_to_ir(a, Stage::Fragment, a.alloc_str(&fs).unwrap()).map_err(|e| e.msg).unwrap();
                compile_program(&v.module, &f.module, Prim::Triangles, TARGET, a).expect("compiles");
                a.peak()
            })
            .expect("scratch");
            super::super::order::ORDER_OFF.with(|h| h.set(false));
            p
        };
        let (off, on) = (peak(true), peak(false));
        assert!(on < 2 * off, "scratch high-water with the order {on} bytes, without {off}");
    }

    /// sched.rs hoist, an A/B in one process (its switch is per thread):
    /// femtovg's fill_color.nodiscard is shorter with operations moved into
    /// earlier free units than without, and both pass check_all. The
    /// values are v3d_sim_mesa's to vouch for (every pair agrees with the IR
    /// with the hoist on).
    #[test]
    fn hoisting_packs_femtovg_fill_color_tighter() {
        static VS: &str = include_str!("../../../../tools/shaders/femtovg/main.vert");
        static FS: &str = include_str!("../../../../tools/shaders/femtovg/fill_color.nodiscard.frag");
        let len = |off: bool| {
            super::super::sched::HOIST_OFF.with(|h| h.set(off));
            let mut n = 0;
            with_program(VS, FS, Prim::Triangles, |p| {
                let p = p.expect("compiles");
                check_all(p);
                n = p.fs.code.instrs.len();
            });
            super::super::sched::HOIST_OFF.with(|h| h.set(false));
            n
        };
        let (off, on) = (len(true), len(false));
        assert!(on < off, "with the hoist {on} words, without {off}");
    }

    /// order.rs, an A/B in one process (its switch is per thread):
    /// femtovg's fill_image_gradient.nodiscard (an image read beside a
    /// gradient's arithmetic) packs shorter with the list scheduler's order
    /// tried than with the selected order alone, and both pass check_all.
    /// The values are v3d_sim_mesa's to vouch for.
    #[test]
    fn the_list_order_packs_femtovg_fill_image_gradient_tighter() {
        static VS: &str = include_str!("../../../../tools/shaders/femtovg/main.vert");
        static FS: &str = include_str!("../../../../tools/shaders/femtovg/fill_image_gradient.nodiscard.frag");
        let len = |off: bool| {
            super::super::order::ORDER_OFF.with(|h| h.set(off));
            let mut n = 0;
            with_program(VS, FS, Prim::Triangles, |p| {
                let p = p.expect("compiles");
                check_all(p);
                n = p.fs.code.instrs.len();
            });
            super::super::order::ORDER_OFF.with(|h| h.set(false));
            n
        };
        let (off, on) = (len(true), len(false));
        assert!(on < off, "with the list order {on} words, without {off}");
    }

    #[test]
    fn a_2d_look_up_is_mesas_sequence() {
        /* es100/textured's FS, as Mesa's v3d_vir_emit_tex writes it
         * (v3d_tex.c:227-386) and as its output in the fixture has it: P0
         * (40, binding << 24 | the two result words), P1 (41, binding << 24 |
         * per-pixel mask enable 4), the T write, then S (which retires the
         * look-up), then two ldtmu read through the .l / .h f16 unpacks. */
        static TEXTURED_VS: &str = include_str!("../../../../tools/shaders/es100/textured.vert");
        static TEXTURED_FS: &str = include_str!("../../../../tools/shaders/es100/textured.frag");
        with_program(TEXTURED_VS, TEXTURED_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            let tmu: Vec<Uniform> =
                p.fs.code.uniforms.iter().copied().filter(|u| u.contents == 40 || u.contents == 41).collect();
            assert_eq!(tmu, [Uniform { contents: 40, data: 0x3 }, Uniform { contents: 41, data: 0x4 }]);
            let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
            assert_eq!((n("wrtmuc"), n("mov tmut"), n("mov tmus"), n("ldtmu")), (2, 1, 1, 2), "{ops:#?}");
            let at = |s: &str| ops.iter().position(|o| o.contains(s)).unwrap();
            assert!(
                at("mov tmut") < at("mov tmus") && at("mov tmus") < at("ldtmu"),
                "T, then S, then the result: {ops:#?}"
            );
            assert!(n(".l") >= 2 && n(".h") >= 2, "four channels from two words: {ops:#?}");
        });
        /* A projective look-up divides s and t by w first (a reciprocal and
         * two multiplies), and a bias goes to TMUB. */
        let fs = |body: &str| {
            std::format!("precision mediump float; uniform sampler2D s; varying vec4 v_color; void main() {{ {body} }}")
        };
        with_program(PASSTHROUGH_VS, &fs("gl_FragColor = texture2DProj(s, v_color);"), Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            assert!(ops.iter().any(|o| o.starts_with("recip ")), "{ops:#?}");
        });
        with_program(
            PASSTHROUGH_VS,
            &fs("gl_FragColor = texture2D(s, v_color.xy, v_color.z);"),
            Prim::Triangles,
            |p| {
                let p = p.expect("compiles");
                check_all(p);
                let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
                assert!(ops.iter().any(|o| o.contains("mov tmub")), "{ops:#?}");
            },
        );
    }

    #[test]
    fn gl_fragcoord_is_mesas_sequence_then_the_window_transform() {
        /* emit_fragcoord_input (nir_to_vir.c:1017-1024): x = FXCD, y = FYCD,
         * z = the payload's Z (rf2), w = RECIP(the payload's W, rf0); then
         * y * scale + offset, the words QUNIFORM_FB_WPOS_Y_SCALE then
         * QUNIFORM_FB_WPOS_Y_OFFSET, data 0 in both (select.rs). */
        let fs = |body: &str| std::format!("precision mediump float; void main() {{ {body} }}");
        with_program(PASSTHROUGH_VS, &fs("gl_FragColor = gl_FragCoord;"), Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            /* each word once; their order is the words' (the scheduler may
             * load either first), and the driver fills each by its contents
             * (v3d_wpos_uniform) */
            let mut wpos: Vec<Uniform> =
                p.fs.code.uniforms.iter().copied().filter(|u| u.contents >= QUNIFORM_FB_WPOS_Y_SCALE).collect();
            wpos.sort_by_key(|u| u.contents);
            assert_eq!(
                wpos,
                [
                    Uniform { contents: QUNIFORM_FB_WPOS_Y_SCALE, data: 0 },
                    Uniform { contents: QUNIFORM_FB_WPOS_Y_OFFSET, data: 0 }
                ]
            );
            let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
            assert_eq!((n("fxcd"), n("fycd"), n("recip")), (1, 1, 1), "{ops:#?}");
            let recip = ops.iter().find(|o| o.contains("recip")).unwrap();
            assert!(recip.contains("rf0"), "w is 1 / the payload's W: {recip}");
            assert!(ops.iter().any(|o| o.contains("rf2")), "z is the payload's Z: {ops:#?}");
            /* No varyings, and still rf0 is read: the flag follows the read,
             * as Mesa's vir_check_payload_w decides it. */
            assert!(p.fs.uses_center_w);
        });
        /* x and y only: no reciprocal, no rf0 read, so no centre W. Two
         * reads of gl_FragCoord.y are one FYCD. */
        with_program(
            PASSTHROUGH_VS,
            &fs("gl_FragColor = vec4(gl_FragCoord.xy, gl_FragCoord.y * 2.0, 1.0);"),
            Prim::Triangles,
            |p| {
                let p = p.expect("compiles");
                check_all(p);
                let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
                let n = |s: &str| ops.iter().filter(|o| o.contains(s)).count();
                assert_eq!((n("fxcd"), n("fycd"), n("recip")), (1, 1, 0), "{ops:#?}");
                assert!(!p.fs.uses_center_w);
            },
        );
    }

    #[test]
    fn transcendentals() {
        /* inversesqrt, sqrt, exp2, log2, 1/x, sin and cos, one each. */
        let fs = "precision mediump float; varying vec4 v_color; void main() { vec4 v = v_color; \
                  gl_FragColor = vec4(inversesqrt(v.x) + sqrt(v.y), exp2(v.z) + log2(v.w), \
                  1.0 / v.x, sin(v.y) + cos(v.z)); }";
        with_program(PASSTHROUGH_VS, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let ops: Vec<String> = p.fs.code.instrs.iter().map(text).collect();
            for o in &ops {
                std::println!("{o}");
            }
            /* the add op opens the line */
            let n = |s: &str| ops.iter().filter(|o| o.starts_with(&std::format!("{s} "))).count();
            /* rsqrt: inversesqrt + sqrt's; recip: sqrt's, 1/x, and none else
             * (the FS has no 1/Wc) */
            assert_eq!(n("rsqrt"), 2, "{ops:#?}");
            assert_eq!(n("recip"), 2, "{ops:#?}");
            assert_eq!(n("exp"), 1, "{ops:#?}");
            assert_eq!(n("log"), 1, "{ops:#?}");
            assert_eq!(n("sin"), 2, "one SIN each for sin and cos: {ops:#?}");
            assert_eq!(n("fround"), 2, "{ops:#?}");
            assert_eq!(n("ftoin"), 2, "{ops:#?}");
        });
    }
    /// femtovg's filter_image (the cluster's blur: a 24-pass loop, two
    /// look-ups a pass, `if (i >= sampleCount) break;` on a uniform) in the
    /// interpreter, at four sigmas: 0, 2, 23 and 24 passes (the last leaves
    /// by the loop's own test, `i <= 24`, the others by the break). Both are
    /// uniform breaks, so each is a branch to the loop's exit (prepare.rs
    /// SOp::LoopBreak): two forward branches out of the loop, no guard round
    /// the rest of a pass, and a back edge on the counter alone. Each run's
    /// colour is the host's blur of the same texels (a box of 2n + 1 taps,
    /// read back per lane), its look-ups 1 + 2n, and its executed
    /// instructions are pinned: 49 + 36 per pass (the 24th pass leaves by
    /// the loop's test, 5 fewer). Before step (c) (origin/main 6ced5426, the
    /// same test with the branch asserts dropped: the same colours and
    /// look-ups) it was 99 + 71 per pass: 99 / 241 / 1732 / 1797; step (c)
    /// made it 63 + 56 (63 / 175 / 1351 / 1402); step (d) issues a pass's
    /// two look-ups under one thread switch (one THRSW in the body): 63 +
    /// 54; step (f) reads the texels' f16 halves through the FMULs' own
    /// input unpack, with no unpacking FMOV (8 a pass): 61 + 45; step (g)
    /// counts the pass with one `sub c, c, 1` (the flag push reads the
    /// counter after its Copy, select.rs SOp::LoopEnd), the push shares a
    /// word (sched.rs hoist's flag rule) and the back edge takes the two
    /// FMULs after it as delay slots (sched.rs branch_back): 61 + 41, as
    /// Mesa's loop (its fixture program: head at 21, `bu` at 58, slots
    /// 59-61); step (l) (alloc.rs, the hoist-aware register choice) moves
    /// two words of the prologue into earlier free slots: 59 + 41, and the
    /// counter lands in another register (read here from the `sub`); step
    /// (m): the loop-carried sums are written where they are kept (alloc.rs,
    /// a MOV into the register file puts its source there: five prologue
    /// MOVs gone), the direction is loaded before the loop (select.rs
    /// SOp::LoopHead: one signal-only word a pass gone): 53 + 40; step (p):
    /// the loop's test and the break take one branch (select.rs
    /// merge_exits: the break's FCMP folds into flag A, `fcmp.andnc`), 4
    /// words a pass and on the path that leaves at once: 49 + 36, against
    /// Mesa's 57 + 41. Revert checks: with uniform_break returning None, no forward
    /// branch leaves the loop and this fails; with select.rs issue_order
    /// returning p's own order, the body has two switches; with the push on
    /// t (the value before the Copy), the counter has a move and a `sub r0`;
    /// with branch_back off (sched.rs BRANCH_BACK_OFF), the slots are NOPs.
    #[test]
    fn a_uniform_break_leaves_filter_images_loop_by_a_branch() {
        use crate::shaderc::online::texref::{Level, MinFilter};
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Texture, Varying, LANES};
        static FV_VS: &str = include_str!("../../../../tools/shaders/femtovg/main.vert");
        static FV_FS: &str = include_str!("../../../../tools/shaders/femtovg/filter_image.nodiscard.frag");
        const W: usize = 64;
        /* texel x: red (x * x mod 64) / 64, exact in f16, so the mean of a
         * window says how wide it was */
        let red = |x: usize| ((x * x) % 64) as f32 / 64.0;
        let texture = Texture {
            cube: false,
            min: MinFilter::Nearest,
            levels: vec![Level {
                width: W as u32,
                height: 1,
                faces: vec![(0..W).map(|x| [red(x), 0.25, 0.5, 1.0]).collect()],
            }],
        };
        /* lane l at texel 24 + l's centre; the blur runs along x */
        let fpos_x = |l: usize| 24.5 + l as f32;
        with_program(FV_VS, FV_FS, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            assert_eq!(c.back_edges.len(), 1, "the loop is kept");
            let (edge, head) = c.back_edges[0];
            let exit = edge + 4;
            let breaks: Vec<_> = c.fwd_branches.iter().filter(|&&(b, _)| b >= head && b < edge).collect();
            /* the loop's test and its break, one branch (select.rs
             * merge_exits): the break's compare folds into flag A */
            assert_eq!(breaks.len(), 1, "the loop's test and its break in one branch: {:?}", c.fwd_branches);
            let mut before = std::string::String::new();
            crate::shaderc::online::v3d::listing::write_instr(&mut before, &c.instrs[breaks[0].0 as usize - 1])
                .unwrap();
            assert!(before.contains(".and") || before.contains(".nor"), "the second compare updates A: {before}");
            assert!(breaks.iter().all(|&&(_, t)| t == exit), "both to the exit: {:?}", c.fwd_branches);
            let (thrsw, _) = thrsw_shape(c);
            let in_body: Vec<_> = thrsw.iter().filter(|&&t| t > head as usize && t < edge as usize).collect();
            assert_eq!(in_body.len(), 1, "a pass's two look-ups under one switch: {thrsw:?}, body {head}..{edge}");
            assert_eq!(p.fs.input_slots.len(), 2, "fpos.xy only: {:?}", p.fs.input_slots);
            /* The counter: one `sub c, c, 1` a pass and no move of it; the
             * back edge's flag push shares a word, and the two coefficient
             * FMULs after it run in the back edge's delay slots. */
            let text: Vec<std::string::String> = c.instrs[head as usize..edge as usize + 4]
                .iter()
                .map(|i| {
                    let mut s = std::string::String::new();
                    crate::shaderc::online::v3d::listing::write_instr(&mut s, i).unwrap();
                    s
                })
                .collect();
            let sub: Vec<_> = text.iter().filter(|t| t.starts_with("sub ")).collect();
            assert_eq!(sub.len(), 1, "one decrement: {text:#?}");
            /* the counter's register, whichever the allocator gave it */
            let ctr = sub[0].trim_start_matches("sub ").split(',').next().unwrap_or("");
            assert!(sub[0].starts_with(&std::format!("sub {ctr}, {ctr}, 1")), "the decrement in place: {text:#?}");
            assert!(!text.iter().any(|t| t.contains(&std::format!("mov {ctr},"))), "no move of the counter: {text:#?}");
            let push = &text[text.len() - 5];
            assert!(
                push.contains(&std::format!("mov.pushz -, {ctr}")) && !push.starts_with("nop "),
                "a shared push word: {text:#?}"
            );
            let slots = &text[text.len() - 3..];
            assert!(slots[..2].iter().all(|t| t.contains("; fmul ")), "work in the first two delay slots: {text:#?}");
            let mut runs = Vec::new();
            for (sigma, n) in [(0.0f32, 0usize), (1.0, 2), (8.0, 23), (9.0, 24)] {
                /* frag[14]: extent (9.xy), texType 0 (10.z), direction
                 * (11.yz), sigma (11.w), the coefficients (12.xyz: 1, 1, 1,
                 * every tap weighs 1), scissorRadius 0 (12.w) */
                let mut frag = [0f32; 56];
                (frag[36], frag[37]) = (W as f32, 1.0);
                (frag[45], frag[46], frag[47]) = (1.0, 0.0, sigma);
                (frag[48], frag[49], frag[50]) = (1.0, 1.0, 1.0);
                let stream: Vec<u32> = c
                    .uniforms
                    .iter()
                    .map(|w| match w.contents {
                        0 | 40 | 41 => w.data,
                        1 => frag[w.data as usize].to_bits(),
                        k => panic!("uniform contents {k} not modelled here"),
                    })
                    .collect();
                let varyings = [0, 1]
                    .iter()
                    .map(|&k| Varying {
                        vary: core::array::from_fn(|l| if k == 0 { fpos_x(l) } else { 0.5f32 }.to_bits()),
                        c: [0; LANES],
                    })
                    .collect();
                let inputs = FsInputs {
                    w: [1.0f32.to_bits(); LANES],
                    w_centroid: [1.0f32.to_bits(); LANES],
                    z: [0; LANES],
                    x: [0; LANES],
                    y: [0; LANES],
                    back_facing: [false; LANES],
                    varyings,
                };
                let env = Env { threads: p.fs.threads, textures: vec![texture.clone()], ..Env::default() };
                let o =
                    sim::run(c.words, &stream, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env).expect("runs");
                for (l, got) in o.tlb[0].value[..LANES].iter().enumerate() {
                    let centre = 24 + l;
                    let want = (centre - n..=centre + n).map(|x| f64::from(red(x))).sum::<f64>() / (2 * n + 1) as f64;
                    let got = got.expect("red and green written");
                    let got = f64::from(sim::half_to_f32((got & 0xffff) as u16));
                    assert!(
                        (got - want).abs() <= 1.0 / 1024.0,
                        "sigma {sigma}, lane {l}: red {got}, the host's {want}"
                    );
                }
                runs.push((o.instructions, o.tmu_lookups));
            }
            assert_eq!(runs.iter().map(|r| r.1).collect::<Vec<_>>(), vec![1, 5, 47, 49], "look-ups: {runs:?}");
            let executed: Vec<usize> = runs.iter().map(|r| r.0).collect();
            assert_eq!(executed, vec![49, 121, 877, 913], "executed per sigma: {runs:?}");
        });
    }

    /// Runs `fs` (reading `v_uv.x` and a 64 x 1 texture, nearest) in the
    /// interpreter on 16 lanes, lane l at texel 8 + l, and checks each
    /// lane's colour against `want(l)`; the texels are [`texel`]'s, exact in
    /// f16, so a result read into another look-up's register shows. Returns
    /// the fragment program's THRSW signal count and its `ldtmu`s after the
    /// first THRSW.
    fn run_lookups(fs: &str, want: impl Fn(usize) -> [f32; 4]) -> (usize, usize) {
        use crate::shaderc::online::texref::{Level, MinFilter};
        use crate::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Texture, Varying, LANES};
        let vs = "attribute vec2 a_uv; varying vec2 v_uv; \
                  void main() { v_uv = a_uv; gl_Position = vec4(a_uv, 0.0, 1.0); }";
        const W: usize = 64;
        let texture = Texture {
            cube: false,
            min: MinFilter::Nearest,
            levels: vec![Level { width: W as u32, height: 1, faces: vec![(0..W).map(texel).collect()] }],
        };
        let mut shape = (0, 0);
        with_program(vs, fs, Prim::Triangles, |p| {
            let p = p.expect("compiles");
            check_all(p);
            let c = &p.fs.code;
            let stream: Vec<u32> = c
                .uniforms
                .iter()
                .map(|w| match w.contents {
                    0 | 40 | 41 => w.data,
                    k => panic!("uniform contents {k} not modelled here"),
                })
                .collect();
            let varyings =
                p.fs.input_slots
                    .iter()
                    .map(|&(_, comp)| Varying {
                        vary: core::array::from_fn(|l| {
                            if comp == 0 { (8.5 + l as f32) / W as f32 } else { 0.5f32 }.to_bits()
                        }),
                        c: [0; LANES],
                    })
                    .collect();
            let inputs = FsInputs {
                w: [1.0f32.to_bits(); LANES],
                w_centroid: [1.0f32.to_bits(); LANES],
                z: [0; LANES],
                x: [0; LANES],
                y: [0; LANES],
                back_facing: [false; LANES],
                varyings,
            };
            let env = Env { threads: p.fs.threads, textures: vec![texture], ..Env::default() };
            let o = sim::run(c.words, &stream, &Inputs::Fragment(std::boxed::Box::new(inputs)), &env).expect("runs");
            let h = |x: f32| u32::from(sim::f32_to_half(x));
            for l in 0..LANES {
                let [r, g, b, a] = want(l);
                assert_eq!(
                    (o.tlb[0].value[l], o.tlb[1].value[l]),
                    (Some(h(r) | h(g) << 16), Some(h(b) | h(a) << 16)),
                    "lane {l}: {fs}"
                );
            }
            let (thrsw, _) = thrsw_shape(c);
            let ldtmu = |i: &Instr| matches!(i, Instr::Alu(a) if a.sig.ldtmu);
            shape = (thrsw.len(), c.instrs.iter().skip(thrsw[0] + 1).filter(|i| ldtmu(i)).count());
        });
        shape
    }

    /// [`run_lookups`]'s texel x: (x / 64, 0.5, (63 - x) / 64, 1).
    fn texel(x: usize) -> [f32; 4] {
        [x as f32 / 64.0, 0.5, (63 - x) as f32 / 64.0, 1.0]
    }

    /// Step (d): independent look-ups of one stretch share one thread
    /// switch, their `ldtmu`s after it in the order the look-ups were
    /// requested (the TMU returns words in that order: Mesa's ntq_flush_tmu,
    /// nir_to_vir.c:262-293). Two look-ups of two words each and three of
    /// one word each fill at most the output FIFO's 4 words (16 / 4 threads,
    /// ntq_tmu_fifo_overflow), so each group has one switch: 3 THRSW signals
    /// in all (that switch made the last-thread-switch pair, and the
    /// thrend), every `ldtmu` after the first. Three look-ups of two words
    /// each are 6: two groups, 4 signals. Every lane's colour is its
    /// texels' own, so an `ldtmu` read into another look-up's register
    /// fails the colour. Revert checks: with issue_order returning p's own
    /// order, or with pipelining off (Pressure::Serial), the 2- and
    /// 3-look-up programs have more switches; with Sel::flush_tmu reading
    /// the queue newest first, the colours swap.
    #[test]
    fn independent_look_ups_share_one_switch_in_request_order() {
        let two = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                   vec4 a = texture2D(t, vec2(v_uv.x, 0.5)); \
                   vec4 b = texture2D(t, vec2(v_uv.x + 0.125, 0.5)); \
                   gl_FragColor = vec4(a.r, b.r, a.b, b.b); }";
        let want = |l: usize| [texel(8 + l)[0], texel(16 + l)[0], texel(8 + l)[2], texel(16 + l)[2]];
        assert_eq!(run_lookups(two, want), (3, 4), "two look-ups, 2 + 2 words: one switch, 4 ldtmu after it");
        let three = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                     float a = texture2D(t, vec2(v_uv.x, 0.5)).r; \
                     float b = texture2D(t, vec2(v_uv.x + 0.125, 0.5)).r; \
                     float c = texture2D(t, vec2(v_uv.x + 0.25, 0.5)).r; \
                     gl_FragColor = vec4(a, b, c, 1.0); }";
        let want = |l: usize| [texel(8 + l)[0], texel(16 + l)[0], texel(24 + l)[0], 1.0];
        assert_eq!(run_lookups(three, want), (3, 3), "three look-ups, 1 word each: one switch, 3 ldtmu after it");
        let six = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                   vec4 a = texture2D(t, vec2(v_uv.x, 0.5)); \
                   vec4 b = texture2D(t, vec2(v_uv.x + 0.125, 0.5)); \
                   vec4 c = texture2D(t, vec2(v_uv.x + 0.25, 0.5)); \
                   gl_FragColor = vec4(a.r, b.r, c.r, a.b * b.b * c.b); }";
        let want = |l: usize| {
            [texel(8 + l)[0], texel(16 + l)[0], texel(24 + l)[0], texel(8 + l)[2] * texel(16 + l)[2] * texel(24 + l)[2]]
        };
        assert_eq!(run_lookups(six, want), (4, 6), "three look-ups, 2 words each: past the FIFO, two switches");
    }

    /// Step (d): a look-up whose coordinates read an earlier look-up's
    /// result is not queued with it (select.rs issue_order: a dependent
    /// read), so it has a switch of its own: 4 THRSW signals (the first
    /// look-up's, the second's made the last pair, the thrend), and the
    /// colour is the texel the first result points at (texel 16 + l for
    /// lane l). Revert check: with issue_order's dependent test off, the
    /// first look-up's lane moves above the look-up and selection fails
    /// (Select(NotSelected)).
    #[test]
    fn a_dependent_look_up_has_its_own_switch() {
        let fs = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                  float a = texture2D(t, vec2(v_uv.x, 0.5)).r; \
                  gl_FragColor = texture2D(t, vec2(a + 0.1328125, 0.5)); }";
        assert_eq!(run_lookups(fs, |l| texel(16 + l)), (4, 3), "two switches: 1 + 2 ldtmu after the first");
    }

    /// The fragment program of `fs` (with [`run_lookups`]'s vertex shader):
    /// its FMOVs with an L or H input unpack that write a register (the
    /// unpacking moves), and its other operand reads through an L or H
    /// unpack (a flag-pushing FMOV, which writes no register, is a reader).
    fn half_unpacks(fs: &str) -> (usize, usize) {
        use crate::shaderc::online::v3d::isa::Unpack;
        let vs = "attribute vec2 a_uv; varying vec2 v_uv; \
                  void main() { v_uv = a_uv; gl_Position = vec4(a_uv, 0.0, 1.0); }";
        let half = |u: Unpack| usize::from(matches!(u, Unpack::L | Unpack::H));
        let mut counts = (0, 0);
        with_program(vs, fs, Prim::Triangles, |p| {
            for i in p.expect("compiles").fs.code.instrs {
                let Instr::Alu(al) = i else { continue };
                let flag_push = al.mul.magic && al.mul.waddr == crate::shaderc::online::v3d::isa::WADDR_NOP;
                if al.mul.op == MulOp::Fmov && !flag_push {
                    counts.0 += half(al.mul.a.unpack);
                } else {
                    counts.1 += half(al.mul.a.unpack) + half(al.mul.b.unpack);
                }
                counts.1 += half(al.add.a.unpack) + half(al.add.b.unpack);
            }
        });
        counts
    }

    /// Steps (f) and (h): a look-up lane is read as `word.l` / `word.h`
    /// through the reader's own input unpack wherever the selected
    /// operation has that field (select.rs fold_unpacks; Mesa folds the
    /// FMOV the same way: vir_opt_copy_propagate.c:167-216), and its FMOV
    /// goes when no reader is left. Here FMUL, FADD and FMIN (step (f)),
    /// then the colour written through VFPACK and a comparison's FCMP
    /// (step (h)). Colours per lane in the interpreter against the texels.
    /// Revert checks: with fold_unpacks's `takes_unpack` false for L and H,
    /// every program here keeps its FMOVs; with VFPACK left out of
    /// `takes_unpack`, the colour program has one FMOV.
    #[test]
    fn a_lane_read_only_through_an_unpack_has_no_fmov() {
        let head = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                    vec4 a = texture2D(t, vec2(v_uv.x, 0.5)); ";
        /* a.r and a.b: every reader is an FMUL, an FADD or an FMIN */
        let folded = std::format!("{head} gl_FragColor = vec4(a.r * 2.0, a.b + 0.25, min(a.r, a.b), 1.0); }}");
        let want = |l: usize| {
            let [r, _, b, _] = texel(8 + l);
            [r * 2.0, b + 0.25, r.min(b), 1.0]
        };
        run_lookups(&folded, want);
        assert_eq!(half_unpacks(&folded), (0, 4), "no FMOV; a.r and a.b each read twice through L/H");
        /* a.r is read by an FMUL and written as the colour: VFPACK reads a.l */
        let colour = std::format!("{head} gl_FragColor = vec4(a.r * 2.0, a.r, 0.5, 1.0); }}");
        run_lookups(&colour, |l| [texel(8 + l)[0] * 2.0, texel(8 + l)[0], 0.5, 1.0]);
        assert_eq!(half_unpacks(&colour), (0, 2), "no FMOV; the FMUL and the VFPACK read a.l");
        /* a.r is read by an FMUL and a comparison: FCMP reads a.l */
        let compare = std::format!("{head} gl_FragColor = vec4(a.r * 2.0, a.r < 0.25 ? 1.0 : 0.0, 0.5, 1.0); }}");
        let want = |l: usize| {
            let r = texel(8 + l)[0];
            [r * 2.0, if r < 0.25 { 1.0 } else { 0.0 }, 0.5, 1.0]
        };
        run_lookups(&compare, want);
        assert_eq!(half_unpacks(&compare), (0, 2), "no FMOV; the FMUL and the FCMP read a.l");
    }

    /// Step (h): FFLOOR and FCEIL take the L/H unpack on their one input,
    /// and FMOV on its one (sign() pushes flags from `FMOV a.l`), so those
    /// lanes have no FMOV either. Colours per lane in the interpreter.
    /// Revert check: with FFLOOR/FCEIL left out of `takes_unpack`, the
    /// first program has two FMOVs; with FMOV left out, the second has one.
    #[test]
    fn a_lane_read_by_ffloor_fceil_or_a_flag_fmov_has_no_fmov() {
        let head = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                    vec4 a = texture2D(t, vec2(v_uv.x, 0.5)); ";
        /* fract(a.r) is a.r - floor(a.r): FFLOOR and FSUB read a.l */
        let round = std::format!("{head} gl_FragColor = vec4(fract(a.r), ceil(a.b), floor(a.b), 1.0); }}");
        let want = |l: usize| {
            let [r, _, b, _] = texel(8 + l);
            [r - r.floor(), b.ceil(), b.floor(), 1.0]
        };
        run_lookups(&round, want);
        assert_eq!(half_unpacks(&round), (0, 4), "no FMOV; FFLOOR, FSUB, FCEIL and FFLOOR read a.l / a.b's L");
        /* sign(a.r - 0.25) would read an FSUB's result; sign(a.r) reads the
         * lane: FMOV.pushz and FMOV.pushn of a.l */
        let sign = std::format!("{head} gl_FragColor = vec4(sign(a.r), 0.5, 0.5, 1.0); }}");
        run_lookups(&sign, |l| [texel(8 + l)[0].signum(), 0.5, 0.5, 1.0]);
        assert_eq!(half_unpacks(&sign), (0, 2), "no FMOV; the two flag-pushing FMOVs read a.l");
    }

    /// Step (h): a lane with one reader that has no f32 unpack field keeps
    /// its FMOV, and every reader reads the FMOV's register: a select's
    /// value (MOV / CondMov), FTOIZ (our encoder refuses L/H there as
    /// unmeasured on silicon), and fabs (the fabs FMOV has its own ABS
    /// unpack: no composing, Mesa vir_opt_copy_propagate.c:192-194). Mesa
    /// folds into the other readers too; here that measured 1-4 words
    /// longer (select.rs fold_unpacks). Colours per lane in the interpreter.
    /// Revert check: with the every-reader rule off, the first two
    /// programs read a lane through L/H beside the FMOV.
    #[test]
    fn a_lane_another_reader_cannot_unpack_keeps_its_fmov() {
        let head = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                    vec4 a = texture2D(t, vec2(v_uv.x, 0.5)); ";
        /* a.b is a select's value: MOV/CondMov take no unpack */
        let select = std::format!("{head} gl_FragColor = vec4(a.r < 0.25 ? a.b : 0.5, a.b * 2.0, 0.5, 1.0); }}");
        let want = |l: usize| {
            let [r, _, b, _] = texel(8 + l);
            [if r < 0.25 { b } else { 0.5 }, b * 2.0, 0.5, 1.0]
        };
        run_lookups(&select, want);
        assert_eq!(
            half_unpacks(&select),
            (1, 1),
            "a.b's FMOV stays for the select, and the FMUL reads it; the FCMP reads a.r's L"
        );
        /* FTOIZ keeps a.r's FMOV; the FMUL reads its register too */
        let ftoi = std::format!("{head} gl_FragColor = vec4(float(int(a.r)), a.r * 2.0, 0.5, 1.0); }}");
        run_lookups(&ftoi, |l| [0.0, texel(8 + l)[0] * 2.0, 0.5, 1.0]);
        assert_eq!(half_unpacks(&ftoi), (1, 0), "a.r's FMOV stays for FTOIZ and the FMUL");
        /* 1.0 - abs(a.r): the ABS folds into the FSUB, on the lane's FMOV */
        let abs = std::format!("{head} gl_FragColor = vec4(1.0 - abs(a.r), 0.5, 0.5, 1.0); }}");
        run_lookups(&abs, |l| [1.0 - texel(8 + l)[0].abs(), 0.5, 0.5, 1.0]);
        assert_eq!(half_unpacks(&abs), (1, 0), "the lane's FMOV stays: the FSUB reads it as rf.abs");
    }

    /// abs(abs(x)): fabs's move read through ABS by a second fabs move,
    /// whose ABS folds into the FSUB. The first move is still read, through
    /// an unpack, so it stays. Before step (h) fold_unpacks (then fold_abs)
    /// counted only plain reads, dropped it, and the program was refused
    /// (Alloc(UseBeforeDef), measured on 8472118d). Colours per lane in the
    /// interpreter.
    #[test]
    fn abs_of_abs_keeps_the_inner_move() {
        let fs = "precision mediump float; uniform sampler2D t; varying vec2 v_uv; void main() { \
                  vec4 a = texture2D(t, vec2(v_uv.x, 0.5)); \
                  gl_FragColor = vec4(1.0 - abs(abs(a.r - 0.25)), 0.5, 0.5, 1.0); }";
        run_lookups(fs, |l| [1.0 - (texel(8 + l)[0] - 0.25).abs(), 0.5, 0.5, 1.0]);
    }
}
