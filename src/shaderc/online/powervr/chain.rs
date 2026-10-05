// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The whole USC path for one shader, in one place: selection, preparation,
//! allocation, the output writes and the bytes.
//!
//! Every caller -- the surveys, the USC-2 diff, and the back end the GL core
//! calls -- goes through this function, so a measurement is always a
//! measurement of the path that ships. Before it existed each test composed
//! the stages itself, and a stage one of them skipped (the output addresses)
//! was a stage the others had no way to notice.

use super::alloc::{allocate, boundary_of, AllocError, Limits};
use super::assemble::{build_shader, vertex_output_writes, BuildError, OutWrite};
use super::prepare::{index_high_shared, prepare_fused, share_common, PrepareError, PrepareStats};
use super::sel::{SelGroup, VReg};
use super::select::{select, SelectError, Selected};
use crate::shaderc::online::arena::Arena;
use crate::shaderc::online::ir;
use crate::shaderc::SHC_STAGE_VERTEX;

/// Where the chain stopped, and why.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ChainError {
    /// `build_vertex` was handed a fragment shader, or `build_fragment` a
    /// vertex shader.
    WrongStage,
    Select(SelectError),
    Prepare(PrepareError),
    Alloc(AllocError),
    Build(BuildError),
    /// A fragment shader that reads a varying: the iterators that evaluate
    /// one are not built.
    FragmentInputsNotBuilt,
    /// A fragment shader writing anything but one `gl_FragColor` vec4: the
    /// only output path built is one RGBA8 colour.
    FragmentOutputsNotBuilt,
    /// The program built but cannot be described to the runtime.
    Record(super::abi::UscAbiError),
    OutOfScratch,
}

/// A built vertex shader, borrowing the scratch arena it was built in.
#[derive(Clone, Copy)]
pub struct Built<'a> {
    /// Bytes of `buf` the shader occupies.
    pub len: usize,
    pub temps: u16,
    pub shared: u16,
    pub vtx_in: u16,
    pub coeff: u16,
    /// The prepared program before allocation, and after.
    pub prepared: &'a [SelGroup],
    pub groups: &'a [SelGroup],
    /// The epilogue's writes, in the order it emits them.
    pub writes: &'a [OutWrite],
    /// What `prepare` rewrote.
    pub prep: PrepareStats,
    /// The sampler bindings the program samples, in descriptor order (the
    /// record lists these samplers; the driver places their descriptors
    /// before the uniform block).
    pub sampled: &'a [u8],
}

/// Build `m` into `buf`.
pub fn build_vertex<'a>(
    m: &'a ir::Module<'a>,
    limits: Limits,
    a: &'a Arena<'a>,
    buf: &mut [u8],
) -> Result<Built<'a>, ChainError> {
    let m = super::abi::with_matrix_varyings_split(m, a).map_err(|_| ChainError::OutOfScratch)?;
    if m.stage != SHC_STAGE_VERTEX {
        return Err(ChainError::WrongStage);
    }
    let sel = select(m, a).map_err(ChainError::Select)?;
    /* Samples, as in a fragment shader: the data and result blocks in fixed
     * temporaries (from 0: a vertex shader pins no iterated input), the
     * uniform block moved past the descriptors. */
    let (sel, sampled) = expand_samples(sel, 0, m.uniforms, a)?;
    let (sel, prep): (Selected<'a>, PrepareStats) = prepare_fused(&sel, a).map_err(ChainError::Prepare)?;
    let sel = share_common(&sel, a).map_err(ChainError::Prepare)?;
    let sel = index_high_shared(&sel, a).map_err(ChainError::Prepare)?;
    /* the default's branch-or-predicate, on the arms' final groups */
    let sel = super::select::choose_if_lowering(sel, a).map_err(ChainError::Select)?;
    let bbuf =
        a.alloc_filled(4 * (sel.inputs.len() + sel.outputs.len()), VReg(0)).map_err(|_| ChainError::OutOfScratch)?;
    let boundary = boundary_of(&sel, bbuf).map_err(ChainError::Alloc)?;
    let al = allocate(sel.groups, limits, [], boundary, a).map_err(ChainError::Alloc)?;

    let n_comp: usize = sel.outputs.iter().map(|(_, c)| c.iter().flatten().count()).sum();
    let wbuf = a
        .alloc_filled(n_comp, OutWrite { bank: super::sel::Bank::Temp, reg: 0, addr: 0 })
        .map_err(|_| ChainError::OutOfScratch)?;
    let outputs = super::abi::written_outputs(m, a).map_err(|_| ChainError::OutOfScratch)?;
    let nw = vertex_output_writes(sel.outputs, al.assignment, outputs, wbuf).map_err(ChainError::Build)?;
    let writes: &'a [OutWrite] = &wbuf[..nw];

    let len = build_shader(al.groups, writes, buf).map_err(ChainError::Build)?;
    Ok(Built {
        len,
        temps: al.temps,
        shared: al.shared,
        vtx_in: al.vtx_in,
        coeff: al.coeff,
        prepared: sel.groups,
        groups: al.groups,
        writes,
        prep,
        sampled,
    })
}

/// The largest shader image the chain builds into, in bytes. It was 16 KiB,
/// four times the vertex corpus peak (574 dwords); pco's own program for
/// sceneeffects' bloom (96bc0c44f5554256.frag) is 30,096 bytes with no loop
/// in it, so the online program for it is at least that long.
pub const MAX_IMAGE_BYTES: usize = 64 << 10;

/// The register-file bounds the product builds against. `temps` is the PDS
/// DOUTU field's measured 252. `shared` is what an operand can address: the
/// widest shared-register index field any source or destination form has is
/// 11 bits (`3b11i`, isa_tables.rs), 2048 registers. It was 1024 from
/// Mesa's rogue_hw_defs.h ROGUE_MAX_PIXEL_SHARED_REGISTERS, which Mesa uses
/// to size the device's coefficient pool, not as a per-program bound:
/// pco's own program for a Qt lit material (0259850a5e89b258.frag, light
/// arrays and matrices in its block) uses 1460 shared registers, and the
/// table path renders such materials on the board (principledmaterial,
/// board-verified). Before that it was a stand-in of 256 above the ES 1.00
/// corpus peak (79). The other two are stand-ins above every corpus peak
/// (vtx_in 21, coeff 0), asserted against those peaks in
/// `tests/usc_limits.rs`.
pub const LIMITS: Limits = Limits { temps: super::abi::USC_MAX_TEMPS, shared: 2048, vtx_in: 64, coeff: 64 };

/// A vertex shader all the way to its record: what the GL core receives.
/// The code is copied into `keep` by the record; everything else lives in
/// `a` and dies with it.
pub fn compile_vertex<'a>(
    m: &'a ir::Module<'a>,
    reflection: &crate::shaderc::online::reflect::Reflection<'a>,
    a: &'a Arena<'a>,
) -> Result<crate::shaderc::ShcShader, ChainError> {
    compile_vertex_with(m, reflection, reflection.attribs, a)
}

/// [`compile_vertex`] with the attribute rows given apart from the rest of
/// the reflection: the link's layout moves them (`abi::with_attrib_map`).
pub fn compile_vertex_with<'a>(
    m: &'a ir::Module<'a>,
    reflection: &crate::shaderc::online::reflect::Reflection<'a>,
    attribs: &'a [crate::shaderc::online::reflect::AttribRow<'a>],
    a: &'a Arena<'a>,
) -> Result<crate::shaderc::ShcShader, ChainError> {
    let m = super::abi::with_matrix_varyings_split(m, a).map_err(|_| ChainError::OutOfScratch)?;
    let image = a.alloc_filled(MAX_IMAGE_BYTES, 0u8).map_err(|_| ChainError::OutOfScratch)?;
    let b = build_vertex(m, LIMITS, a, image)?;
    /* Whole dwords: the shader closes on an 8-byte boundary. */
    let words = a.alloc_filled(b.len / 4, 0u32).map_err(|_| ChainError::OutOfScratch)?;
    for (w, c) in words.iter_mut().zip(image[..b.len].as_chunks::<4>().0) {
        *w = u32::from_le_bytes(*c);
    }
    let uniforms = uniform_rows_sampling(reflection, b.sampled, a)?;
    super::abi::assemble(&super::abi::UscProgram {
        stage: SHC_STAGE_VERTEX,
        code: words,
        temps: b.temps,
        io_out: super::abi::written_outputs(m, a).map_err(|_| ChainError::OutOfScratch)?,
        io_in: &[],
        attribs,
        attribs_read: super::abi::attribs_placed(m).map_err(ChainError::Record)?,
        uniforms,
        uniform_dwords: reflection.uniform_dwords,
        meta_off: None,
    })
    .map_err(ChainError::Record)
    .and_then(|mut sh| {
        record_vs_ids(&mut sh, m)?;
        /* a vertex shader that samples a cube as the TPU's own */
        sh.native_cube =
            u8::from(b.groups.iter().any(|g| matches!(g.op, super::sel::SelOp::Sample { cube: true, .. })));
        Ok(sh)
    })
}

/// A vertex record's `gl_VertexID` / `gl_InstanceID` fields: where the
/// vertex fetch writes them (`abi::vs_id_regs`), plus one; 0 when the shader
/// reads none -- as pco's records carry them.
pub fn record_vs_ids(sh: &mut crate::shaderc::ShcShader, m: &ir::Module<'_>) -> Result<(), ChainError> {
    let (vid, iid) = super::abi::vs_id_regs(m);
    let reg1 = |r: Option<u16>| -> Result<u8, ChainError> {
        r.map_or(Ok(0), |r| {
            u8::try_from(r + 1)
                .map_err(|_| ChainError::Record(super::abi::UscAbiError::AttribOutOfRange { index: r as usize }))
        })
    };
    sh.vs_vertex_id_reg1 = reg1(vid)?;
    sh.vs_instance_id_reg1 = reg1(iid)?;
    Ok(())
}

/// A built fragment shader: its bytes' length, its temporaries, and the
/// varyings it reads in the order their coefficients are laid out -- the
/// order the record must list them in.
pub struct BuiltFragment<'a> {
    pub len: usize,
    pub temps: u16,
    pub varyings: &'a [ir::IoDecl<'a>],
    /// The sampler bindings the program samples, in descriptor-slot order.
    pub sampled: &'a [u8],
    /// A shader that discards: where its epilogue reads the per-draw
    /// control word (absolute), for the record.
    pub meta_off: Option<u16>,
    /// A shader that reads `gl_FragCoord`: where its prologue reads the
    /// driver's y pair (scale, offset), absolute, for the record's
    /// `fragcoord_ytf_off`.
    pub ytf_off: Option<u16>,
    /// The program iterates `gl_FragCoord.z` (the record's `uses_z`) and
    /// `.w` (which needs W's set whatever the varyings: `uses_w`).
    pub frag_z: bool,
    pub frag_w: bool,
    /// The program samples a cube as the TPU's own (`smp` 3d): the record's
    /// `native_cube`.
    pub native_cube: bool,
    /// A shader that reads `gl_FrontFacing`: where its prologue reads the
    /// driver's front-face op, absolute (the record's `fs_front_face_off1`
    /// less one).
    pub ff_off: Option<u16>,
    /// The program iterates `gl_PointCoord`, which the driver must set up
    /// (a point-sprite iterator) for the record's `uses_pntc`.
    pub uses_pntc: bool,
    /// A blend variant: its blend (whether it reads the pixel, for
    /// `uses_fbfetch`, and its target class, for `num_outputs`).
    pub blend: Option<super::blend::Blend>,
    /// Where it reads the blend colour, in dwords from the start of the
    /// uniform block (the record's `blend_consts_off`), when it does.
    pub blend_consts: Option<u16>,
    /// The pixel output registers the colours span (the record's
    /// `num_outputs`), when more than one output is written: the highest
    /// output location plus one. `None` for one output, which the record
    /// counts from its target class.
    pub output_regs: Option<u8>,
}

/// A fragment shader that writes one `gl_FragColor` and reads smooth
/// varyings, built into `buf`.
///
/// Each varying the program reads is evaluated into consecutive temporaries
/// before anything else runs (`fragment::iterate_group`), packed from
/// temporary 0 in declaration order. Selection names those values as
/// virtual registers; here every use becomes the fixed temporary the
/// iterator writes, and those temporaries are pinned so the allocator places
/// nothing else on them.
pub fn build_fragment<'a>(
    m: &'a ir::Module<'a>,
    limits: Limits,
    a: &'a Arena<'a>,
    buf: &mut [u8],
) -> Result<BuiltFragment<'a>, ChainError> {
    build_fragment_for(m, limits, a, buf, 0)
}

/// [`build_fragment`] for the blend variant `blend_key` (`blend.rs`): the
/// blend lowered into the program's tail. 0 is the shader as written.
pub fn build_fragment_for<'a>(
    m: &'a ir::Module<'a>,
    limits: Limits,
    a: &'a Arena<'a>,
    buf: &mut [u8],
    blend_key: u32,
) -> Result<BuiltFragment<'a>, ChainError> {
    let m = super::abi::with_matrix_varyings_split(m, a).map_err(|_| ChainError::OutOfScratch)?;
    use super::fragment::{coefficient_offsets, IterMode, Iterated};
    use super::sel::{Bank, Mods, SelDst, SelSrc};
    use crate::shaderc::online::arena::AVec;
    use crate::shaderc::online::ir::{IoKind, IoSlot};
    let oom = |_| ChainError::OutOfScratch;
    if m.stage != crate::shaderc::SHC_STAGE_FRAGMENT {
        return Err(ChainError::WrongStage);
    }
    let sel = select(m, a).map_err(ChainError::Select)?;

    /* The varyings read, in declaration order, and where each lands. */
    let mut listed: AVec<'a, ir::IoDecl<'a>> = AVec::new();
    for io in m.inputs.iter().filter(|io| io.kind == IoKind::Varying) {
        if sel.inputs.iter().any(|(slot, c)| *slot == IoSlot::Varying(io.location) && c.iter().any(Option::is_some)) {
            /* Smooth, iterated; flat, read from its plane's constant
             * (`fragment::Iterated::flat`). Noperspective is not
             * transcribed. */
            if !matches!(io.interp, ir::INTERP_SMOOTH | ir::INTERP_FLAT) {
                return Err(ChainError::FragmentInputsNotBuilt);
            }
            listed.push(a, *io).map_err(oom)?;
        }
    }
    if sel.inputs.iter().any(|(slot, _)| !matches!(slot, IoSlot::Varying(_) | IoSlot::PointCoord | IoSlot::FragCoord)) {
        return Err(ChainError::FragmentInputsNotBuilt);
    }
    /* `gl_PointCoord`, as pco lays it out: iterated with no divide
     * (`ditr`), in the eight coefficients straight after W's four when the
     * program iterates W, from 0 when it does not, before every varying
     * (pco_shc build-new, 2026-09-23: `ditr coff=0` alone, `coff=4` and a
     * varying at 12 beside one; Mesa vulkan/pco_shc.c allocates it right
     * after Z/W). */
    let pntc = sel.inputs.iter().find(|(slot, _)| *slot == IoSlot::PointCoord).map(|(_, r)| *r);
    let listed: &'a [ir::IoDecl<'a>] = listed.into_slice();
    let mut comps = [0u8; 16];
    let mut coffs = [0u16; 16];
    if listed.len() > comps.len() {
        return Err(ChainError::FragmentInputsNotBuilt);
    }
    for (c, io) in comps.iter_mut().zip(listed) {
        *c = io.components;
    }
    coefficient_offsets(&comps[..listed.len()], &mut coffs).map_err(ChainError::Build)?;
    let mut iterated: AVec<'a, Iterated> = AVec::new();
    let mut dest = 0u16;
    /* `gl_FragCoord.z` / `.w`, as pco lays them out (pco_shc build-new,
     * 2026-09-24: `fs_in POS(z/w)` at cf0, one coefficient set each, Z
     * before W; a varying after both from cf8): Z's set first when .z is
     * read (the record's `uses_z`), then W's -- present when .w is read or
     * any varying is listed (`uses_w`) -- and each plane iterated with no
     * divide, as `gl_PointCoord` is. */
    let zw = sel.inputs.iter().find(|(slot, _)| *slot == IoSlot::FragCoord).map_or([None; 2], |(_, l)| [l[2], l[3]]);
    let z_set = if zw[0].is_some() { 4 } else { 0 };
    let has_w = zw[1].is_some() || !listed.is_empty();
    if zw[0].is_some() {
        iterated.push(a, Iterated { dest, coff: 0, woff: z_set, count: 1, mode: IterMode::Linear }).map_err(oom)?;
        dest += 1;
    }
    if zw[1].is_some() {
        iterated.push(a, Iterated { dest, coff: z_set, woff: z_set, count: 1, mode: IterMode::Linear }).map_err(oom)?;
        dest += 1;
    }
    /* The record iterates W exactly when it lists a varying (`abi.rs`,
     * `uses_w`) or the program reads gl_FragCoord.w. */
    let w = z_set + if has_w { 4 } else { 0 };
    let shift = z_set + if pntc.is_some() { 8 } else { 0 };
    if pntc.is_some() {
        iterated.push(a, Iterated { dest, coff: w, woff: z_set, count: 2, mode: IterMode::Linear }).map_err(oom)?;
        dest += 2;
    }
    for (i, io) in listed.iter().enumerate() {
        let mode = if io.interp == ir::INTERP_FLAT { IterMode::Flat } else { IterMode::Perspective };
        iterated
            .push(a, Iterated { dest, coff: coffs[i] + shift, woff: z_set, count: io.components, mode })
            .map_err(oom)?;
        dest += io.components as u16;
    }
    let iterated: &'a [Iterated] = iterated.into_slice();
    let pinned_n = dest;

    /* Every read of an input value becomes its fixed temporary. */
    let fixed_of = |v: VReg| -> Option<u16> {
        for (slot, lanes) in sel.inputs {
            for (lane, x) in lanes.iter().enumerate() {
                if *x == Some(v) {
                    /* the iterators in order: z, w, the point coordinate,
                     * the varyings */
                    let planes = usize::from(zw[0].is_some()) + usize::from(zw[1].is_some());
                    if *slot == IoSlot::FragCoord {
                        return Some(iterated[usize::from(lane == 3 && zw[0].is_some())].dest);
                    }
                    if *slot == IoSlot::PointCoord {
                        return Some(iterated[planes].dest + lane as u16);
                    }
                    let IoSlot::Varying(loc) = slot else { return None };
                    let i = listed.iter().position(|io| io.location == *loc)?;
                    return Some(iterated[planes + i + usize::from(pntc.is_some())].dest + lane as u16);
                }
            }
        }
        None
    };
    let rewrite = |s: SelSrc| -> SelSrc {
        match s {
            SelSrc::Virtual { v, mods } => match fixed_of(v) {
                Some(r) => SelSrc::Fixed { bank: Bank::Temp, reg: r, mods },
                None => s,
            },
            other => other,
        }
    };
    /* Where a smooth varying component's plane starts in the coefficient
     * file (`SelOp::PlaneCoeff`): its iterator's `coff`, four per
     * component. */
    let plane_of = |v: VReg| -> Option<u16> {
        let planes = usize::from(zw[0].is_some()) + usize::from(zw[1].is_some()) + usize::from(pntc.is_some());
        for (slot, lanes) in sel.inputs {
            let IoSlot::Varying(loc) = slot else { continue };
            let Some(lane) = lanes.iter().position(|x| *x == Some(v)) else { continue };
            let i = listed.iter().position(|io| io.location == *loc)?;
            let it = iterated.get(planes + i)?;
            return (it.mode == IterMode::Perspective).then_some(it.coff + 4 * lane as u16);
        }
        None
    };
    let groups = a.alloc_copy(sel.groups).map_err(oom)?;
    for g in groups.iter_mut() {
        if let Some(SelDst::Virtual(v)) = g.dst {
            if fixed_of(v).is_some() {
                return Err(ChainError::FragmentInputsNotBuilt);
            }
        }
        /* A plane's A or B coefficient: a move from its register -- W's
         * set at `z_set` (W is iterated whenever a varying is), or the
         * varying component's. */
        if let super::sel::SelOp::PlaneCoeff { term } = g.op {
            let base = match g.srcs[0] {
                None if has_w => z_set,
                Some(SelSrc::Virtual { v, .. }) => plane_of(v).ok_or(ChainError::FragmentInputsNotBuilt)?,
                _ => return Err(ChainError::FragmentInputsNotBuilt),
            };
            g.op = super::sel::SelOp::Copy;
            g.ty = super::sel::OpType::F32;
            g.srcs = [
                Some(SelSrc::Fixed { bank: Bank::Coeff, reg: base + u16::from(term), mods: Mods::default() }),
                None,
                None,
            ];
            continue;
        }
        for s in g.srcs.iter_mut().flatten() {
            *s = rewrite(*s);
        }
    }
    let outputs = a.alloc_copy(sel.outputs).map_err(oom)?;
    for (_, c) in outputs.iter_mut() {
        for s in c.iter_mut().flatten() {
            *s = rewrite(*s);
        }
    }
    let discard = sel.discard.map(rewrite);
    let alpha_to_one = sel.alpha_to_one.map(|(x, f)| (rewrite(x), rewrite(f)));
    let sel = Selected { groups, vregs: sel.vregs, outputs, inputs: &[], discard, alpha_to_one };
    let (sel, sampled) = expand_samples(sel, pinned_n, m.uniforms, a)?;
    let native_cube = sel.groups.iter().any(|g| matches!(g.op, super::sel::SelOp::Sample { cube: true, .. }));

    let sel: Selected<'a> = prepare_fused(&sel, a).map_err(ChainError::Prepare)?.0;
    let sel = share_common(&sel, a).map_err(ChainError::Prepare)?;
    /* The front-face op only when the program still reads it: a
     * gl_FrontFacing whose value nothing used is gone, and pco's record then
     * asks for no op either (8ad4bf6d52933374.frag). */
    let read_at = |d: u16| {
        let at = TEX_DESC_DWORDS * sampled.len() as u16 + d;
        sel.groups.iter().any(|g| {
            g.srcs.iter().flatten().any(|s| matches!(s, SelSrc::Fixed { bank: Bank::Shared, reg, .. } if *reg == at))
        })
    };
    let ff_live = super::select::ff_dword(m).filter(|&d| read_at(d));
    /* gl_FragCoord's y pair likewise: a shader reading only .z or .w reads
     * no pair. */
    let ytf_live = super::select::ytf_dword(m).filter(|&d| read_at(d));
    /* After the two scans above: they look for direct reads of those words. */
    let sel = index_high_shared(&sel, a).map_err(ChainError::Prepare)?;
    /* the default's branch-or-predicate, on the arms' final groups */
    let sel = super::select::choose_if_lowering(sel, a).map_err(ChainError::Select)?;
    /* A shader that writes nothing (qtappshaders_0.frag computes locals
     * and stores none): pco's program is one control `nop` that ends it.
     * Everything else it computed is dead. */
    if sel.outputs.is_empty() && sel.discard.is_none() && sel.groups.is_empty() && iterated.is_empty() {
        let len = super::assemble::build_empty_fragment_shader(buf).map_err(ChainError::Build)?;
        return Ok(BuiltFragment {
            len,
            temps: 0,
            varyings: listed,
            sampled,
            meta_off: None,
            ytf_off: None,
            ff_off: None,
            frag_z: false,
            frag_w: false,
            native_cube: false,
            uses_pntc: false,
            /* It writes no colour, so there is nothing to blend. */
            blend: None,
            blend_consts: None,
            output_regs: None,
        });
    }
    /* One gl_FragColor, or several outputs (`layout(location = n) out`,
     * gl_FragData[n]), each at its own location. */
    let single = matches!(sel.outputs, [(ir::IoSlot::FragColor(0), _)]);
    if !sel.outputs.iter().all(|(slot, _)| matches!(slot, ir::IoSlot::FragColor(_))) {
        return Err(ChainError::FragmentOutputsNotBuilt);
    }
    let bbuf = a.alloc_filled(4 * sel.outputs.len() + 3, VReg(0)).map_err(oom)?;
    let boundary = boundary_of(&sel, bbuf).map_err(ChainError::Alloc)?;
    let al =
        allocate(sel.groups, limits, (0..pinned_n).map(|r| (Bank::Temp, r)), boundary, a).map_err(ChainError::Alloc)?;
    let resolve = |comps: &[Option<SelSrc>; 4]| -> Result<[(Bank, u16); 4], ChainError> {
        let mut colour = [(Bank::Temp, 0u16); 4];
        for (c, s) in colour.iter_mut().zip(comps.iter()) {
            *c = match s {
                Some(SelSrc::Virtual { v, mods }) if *mods == Mods::default() => {
                    let r = *al.assignment.get(v.0 as usize).ok_or(ChainError::FragmentOutputsNotBuilt)?;
                    if r == super::alloc::UNASSIGNED {
                        return Err(ChainError::FragmentOutputsNotBuilt);
                    }
                    (Bank::Temp, r)
                }
                Some(SelSrc::Fixed { bank, reg, mods }) if *mods == Mods::default() => (*bank, *reg),
                _ => return Err(ChainError::FragmentOutputsNotBuilt),
            };
        }
        Ok(colour)
    };
    let mut colour = match sel.outputs.first() {
        Some((_, comps)) if single => resolve(comps)?,
        _ => [(Bank::Temp, 0u16); 4],
    };
    /* The discard flag's register, for the feedback test. */
    let discard = match sel.discard {
        None => None,
        Some(SelSrc::Virtual { v, .. }) => {
            let r = *al.assignment.get(v.0 as usize).ok_or(ChainError::FragmentOutputsNotBuilt)?;
            if r == super::alloc::UNASSIGNED {
                return Err(ChainError::FragmentOutputsNotBuilt);
            }
            Some(r)
        }
        Some(_) => return Err(ChainError::FragmentOutputsNotBuilt),
    };
    /* Alpha-to-one's pair, for a blend variant: the alpha before it, and
     * the flag the tail applies after the blend. */
    let reg_of = |s: SelSrc| -> Result<u16, ChainError> {
        let SelSrc::Virtual { v, .. } = s else { return Err(ChainError::FragmentOutputsNotBuilt) };
        let r = *al.assignment.get(v.0 as usize).ok_or(ChainError::FragmentOutputsNotBuilt)?;
        if r == super::alloc::UNASSIGNED {
            return Err(ChainError::FragmentOutputsNotBuilt);
        }
        Ok(r)
    };
    let alpha_to_one = match sel.alpha_to_one {
        Some((x, f)) => Some((reg_of(x)?, reg_of(f)?)),
        None => None,
    };
    /* Selection reads the control word at the end of the block, and
     * `expand_samples` moved the block past the descriptors: the same sum
     * gives the absolute register the driver DMAs to. */
    let base = TEX_DESC_DWORDS * sampled.len() as u16;
    let meta_off = discard.map(|_| base + super::select::meta_dword(m));
    let ytf_off = ytf_live.map(|d| base + d);
    let ff_off = ff_live.map(|d| base + d);
    let free = al.temps.max(pinned_n);
    /* A blend variant: the constants (`glBlendColor`), when a factor reads
     * them, are four dwords the GL core appends to the uniform block, as it
     * does for pco's variants -- where the control word and gl_FragCoord's
     * pair also sit, so those are refused together for now. */
    let blend = super::blend::Blend::from_key(blend_key).map_err(|e| ChainError::Build(BuildError::Unsupported(e)))?;
    let blend_consts = match blend {
        Some(b) if b.reads_constants() => {
            if discard.is_some() || ytf_off.is_some() || ff_off.is_some() {
                return Err(ChainError::Build(BuildError::Unsupported(
                    "a constant-colour blend in a shader that discards or reads gl_FragCoord or gl_FrontFacing",
                )));
            }
            /* The tail reads the four words straight from the shared file,
             * after `index_high_shared` ran: refuse what it cannot name. */
            if u32::from(base) + u32::from(m.uniform_dwords) + 3 >= u32::from(super::prepare::SHARED_DIRECT_LIMIT) {
                return Err(ChainError::Build(BuildError::Unsupported(
                    "blend constants past sh255 (the blend tail has no indexed read built)",
                )));
            }
            Some(m.uniform_dwords)
        }
        _ => None,
    };
    /* A blend variant blends the alpha from before alpha-to-one, and the
     * tail applies the flag to the blended colour (pco's order). */
    let mut a2one_flag = None;
    if let (Some(_), Some((alpha, flag))) = (blend, alpha_to_one) {
        colour[3] = (Bank::Temp, alpha);
        a2one_flag = Some(flag);
    }
    let mut output_regs = None;
    let (len, temps) = if single {
        super::assemble::build_fragment_shader_blended(
            iterated,
            al.groups,
            colour,
            discard,
            free,
            blend.map(|b| (b, blend_consts.map(|c| base + c), a2one_flag)),
            buf,
        )
        .map_err(ChainError::Build)?
    } else {
        /* Several outputs: RGBA8 attachments, one pixel output register
         * each, in location order (src/pvr/backend.rs emits attachment i
         * from register i when every attachment is 8-bit). A blend variant
         * carries the target class and blends one colour, so it is refused
         * here, and so is alpha-to-one, which only a blend variant applies. */
        use super::assemble::MAX_RGBA8_OUTPUTS;
        let unsupported = |why| ChainError::Build(BuildError::Unsupported(why));
        if blend.is_some() {
            return Err(unsupported("a blend variant of a shader with more than one colour output"));
        }
        if alpha_to_one.is_some() {
            return Err(unsupported("alpha-to-one in a shader with more than one colour output"));
        }
        if sel.outputs.len() > MAX_RGBA8_OUTPUTS {
            return Err(unsupported("more colour outputs than the pixel's four output registers"));
        }
        let mut outs = [(0u16, [(Bank::Temp, 0u16); 4]); MAX_RGBA8_OUTPUTS];
        for (o, (slot, comps)) in outs.iter_mut().zip(sel.outputs.iter()) {
            let ir::IoSlot::FragColor(loc) = *slot else { return Err(ChainError::FragmentOutputsNotBuilt) };
            *o = (u16::from(loc), resolve(comps)?);
        }
        let outs = &mut outs[..sel.outputs.len()];
        outs.sort_unstable_by_key(|(loc, _)| *loc);
        output_regs = outs.last().map(|(loc, _)| *loc as u8 + 1);
        super::assemble::build_fragment_shader_outputs(iterated, al.groups, outs, discard, free, buf)
            .map_err(ChainError::Build)?
    };
    if temps > limits.temps {
        return Err(ChainError::Alloc(AllocError::OutOfRegisters {
            file: super::alloc::RegFile::Temp,
            needed: temps as usize,
        }));
    }
    Ok(BuiltFragment {
        len,
        temps,
        varyings: listed,
        sampled,
        meta_off,
        ytf_off,
        ff_off,
        frag_z: zw[0].is_some(),
        frag_w: zw[1].is_some(),
        native_cube,
        uses_pntc: pntc.is_some(),
        blend,
        blend_consts,
        output_regs,
    })
}

/// Each texture descriptor's size in the shared file:
/// `pvr_combined_image_sampler_descriptor`, image 0..7 and sampler 8..19
/// (src/pvr/backend.rs TEX_DESC_DWORDS). Every `smp` in the corpus reads
/// its image at shared 20*i and its sampler at 20*i+8.
pub const TEX_DESC_DWORDS: u16 = 20;

/// Expand selection's sample markers into what the builder emits, as pco
/// emits it for `texture2D` (measured with pco_shc build-new):
///
/// * the coordinate into the first two registers of a 16-register sample
///   data block and zero into the other fourteen (pco's fourteen `mbyp`s),
///   the zeros only where an earlier sample has not already left them
///   (pco writes them before every sample; the block is ours alone, so the
///   words read the same either way);
/// * the `smp`, reading descriptor slot `i` -- the sampler's rank among the
///   sampled bindings, which is the order the driver lays descriptors out in
///   (`build_desc_block` sorts them by binding) -- and writing a 4-register
///   result block;
/// * `wdf`, and a move of each result to the value's own register.
///
/// The data and result blocks are fixed temporaries from `first_free`,
/// which every sample shares: a fixed temporary is reserved for the whole
/// program by the allocator. Uniform reads move past the descriptors, which
/// the driver puts before the block (`sh0.. (sh20.. after a texture)`).
///
/// Returns the program and the sampled bindings in slot order.
fn expand_samples<'a>(
    sel: Selected<'a>,
    first_free: u16,
    decls: &[ir::UniformDecl<'_>],
    a: &'a Arena<'a>,
) -> Result<(Selected<'a>, &'a [u8]), ChainError> {
    use super::sel::{Bank, Mods, OpType, SelDst, SelOp, SelSrc, SmpLod};
    use crate::shaderc::online::arena::AVec;
    let oom = |_| ChainError::OutOfScratch;
    let mut bindings: AVec<'a, u8> = AVec::new();
    let mut samples = 0usize;
    for g in sel.groups {
        let binding = match g.op {
            SelOp::Sample { binding, .. } => {
                samples += 1;
                binding
            }
            /* a sampler only textureSize reads has its descriptor too */
            SelOp::TexWord { binding, .. } => binding,
            _ => continue,
        };
        if !bindings.as_slice().contains(&binding) {
            bindings.push(a, binding).map_err(oom)?;
        }
    }
    let bindings = bindings.into_slice();
    if bindings.is_empty() {
        return Ok((sel, bindings));
    }
    let sorted = a.alloc_copy(bindings).map_err(oom)?;
    sorted.sort_unstable();
    let sorted: &'a [u8] = sorted;
    /* The descriptor slots: every element of every sampler row the record
     * keeps (`uniform_rows_sampling`), which is what the driver lays out
     * (draw.rs gives each element k of a row the binding `binding + k`,
     * build_desc_block sorts them). Ranked among the SAMPLED bindings
     * alone, a program that samples `m3[1]` of `samplerCube m3[3]` read
     * slot 1 where the driver put binding 6 at slot 3 (dEQP
     * uniform_api.*.unused_uniforms.sampler2D_samplerCube_*, host). */
    let slots = descriptor_bindings(decls, sorted, a)?;
    let base = TEX_DESC_DWORDS * slots.len() as u16;
    let data = first_free;
    let dest = first_free + 16;

    let shift = |s: SelSrc| match s {
        SelSrc::Fixed { bank: Bank::Shared, reg, mods } => SelSrc::Fixed { bank: Bank::Shared, reg: reg + base, mods },
        other => other,
    };
    /* At most 23 groups per sample (three coordinate words, fourteen zeros,
     * the sample, the wait, four result moves; gradient words take the
     * place of zeros), sized once. */
    let mut out: AVec<'a, SelGroup> = AVec::with_capacity(a, sel.groups.len() + 22 * samples).map_err(oom)?;
    let copy = |dst: SelDst, src: SelSrc| SelGroup {
        op: SelOp::Copy,
        ty: OpType::F32,
        dst: Some(dst),
        srcs: [Some(src), None, None],
    };
    let temp = |r: u16| SelSrc::Fixed { bank: Bank::Temp, reg: r, mods: Mods::default() };
    /* Which words of the data block hold zero at this point of the program. */
    let mut zeroed = [false; 16];
    let mut vregs = sel.vregs;
    /* a cube sample's third coordinate (SelOp::SampleZ, right before it) */
    let mut cube_z: Option<SelSrc> = None;
    /* a gradient sample's gradients (SelOp::SampleGrad, before it), d/dx
     * and d/dy per coordinate */
    let mut grads: [Option<SelSrc>; 6] = [None; 6];
    let mut n_grads = 0usize;
    for (at, g) in sel.groups.iter().enumerate() {
        let mut g = *g;
        for s in g.srcs.iter_mut().flatten() {
            *s = shift(*s);
        }
        if g.op == SelOp::SampleZ {
            cube_z = g.srcs[0];
            continue;
        }
        if g.op == SelOp::SampleGrad {
            if n_grads + 2 > grads.len() {
                return Err(ChainError::FragmentInputsNotBuilt);
            }
            grads[n_grads] = g.srcs[0];
            grads[n_grads + 1] = g.srcs[1];
            n_grads += 2;
            continue;
        }
        /* An indexed uniform read (`LoadUniformIdx`) addresses the shared
         * file through idx0, whose value selection computed as the block
         * offset plus the index: the block moved past the descriptors, so
         * the address moves with it. Without this, a shader that samples and
         * indexes a uniform array read `base` dwords too low. */
        if g.dst == Some(SelDst::Fixed { bank: Bank::Idx0, reg: super::sel::IDX_SELF }) && base > 0 {
            let at_reg = VReg(vregs);
            vregs += 1;
            out.push(
                a,
                SelGroup {
                    op: SelOp::Alu(crate::shaderc::online::ir::AluOp::Iadd),
                    ty: OpType::S32,
                    dst: Some(SelDst::Virtual(at_reg)),
                    srcs: [g.srcs[0], Some(SelSrc::Imm(u32::from(base))), None],
                },
            )
            .map_err(oom)?;
            g.srcs[0] = Some(SelSrc::Virtual { v: at_reg, mods: Mods::default() });
        }
        /* The top of a loop is reached from before it and from its own end:
         * a word a projective sample in the body writes is not known to be
         * zero there. (The body runs at least once, so what holds at its end
         * holds after the loop.) */
        if g.op == SelOp::LoopBegin {
            let body = sel.groups[at..].iter().take_while(|x| x.op != SelOp::LoopEnd);
            /* words 2 and 3: a divisor or a level, then an offset's lookup */
            for x in body.clone() {
                if let SelOp::Sample { proj, lod, offset, cube, .. } = x.op {
                    let coords = 2 + usize::from(cube);
                    let grad_words = if lod == SmpLod::Gradients { 2 * coords } else { 0 };
                    let data_words =
                        coords + grad_words + usize::from(proj || lod.in_block()) + usize::from(offset.is_some());
                    zeroed[2..data_words.max(2)].fill(false);
                }
            }
        }
        /* A word of the image state: the descriptor's shared register,
         * now that the descriptors are placed. */
        if let SelOp::TexWord { binding, word } = g.op {
            let slot = slots.iter().position(|b| *b == binding).ok_or(ChainError::OutOfScratch)? as u16;
            let from = SelSrc::Fixed {
                bank: Bank::Shared,
                reg: TEX_DESC_DWORDS * slot + u16::from(word),
                mods: Mods::default(),
            };
            let dst = g.dst.ok_or(ChainError::FragmentInputsNotBuilt)?;
            out.push(a, SelGroup { op: SelOp::Copy, ty: OpType::U32, dst: Some(dst), srcs: [Some(from), None, None] })
                .map_err(oom)?;
            continue;
        }
        let SelOp::Sample { binding, proj, lod, fetch, offset, cube } = g.op else {
            out.push(a, g).map_err(oom)?;
            continue;
        };
        let (Some(u), Some(v), Some(SelDst::Virtual(r))) = (g.srcs[0], g.srcs[1], g.dst) else {
            return Err(ChainError::FragmentInputsNotBuilt);
        };
        let slot = slots.iter().position(|b| *b == binding).ok_or(ChainError::OutOfScratch)? as u16;
        out.push(a, copy(SelDst::Fixed { bank: Bank::Temp, reg: data }, u)).map_err(oom)?;
        out.push(a, copy(SelDst::Fixed { bank: Bank::Temp, reg: data + 1 }, v)).map_err(oom)?;
        /* A cube (`smp` 3d): the third coordinate, then the level (pco's
         * program for a samplerCube: x, y, z in the first three words). */
        let coords = if cube {
            let z = cube_z.take().ok_or(ChainError::FragmentInputsNotBuilt)?;
            out.push(a, copy(SelDst::Fixed { bank: Bank::Temp, reg: data + 2 }, z)).map_err(oom)?;
            3
        } else {
            2
        };
        /* The projective form's divisor, or the level word (replacing or
         * biasing the level), follows the coordinate (pco_emit_nir_smp's
         * order). */
        let first_zero = if proj || lod.in_block() {
            let q = g.srcs[2].ok_or(ChainError::FragmentInputsNotBuilt)?;
            out.push(a, copy(SelDst::Fixed { bank: Bank::Temp, reg: data + coords }, q)).map_err(oom)?;
            coords + 1
        } else {
            coords
        };
        /* The gradients (`lodm = gradients`): d/dx then d/dy of each
         * coordinate after the coordinates, pco_emit_nir_smp's order. */
        let first_zero = if lod == SmpLod::Gradients {
            if n_grads != 2 * usize::from(coords) {
                return Err(ChainError::FragmentInputsNotBuilt);
            }
            for (k, gs) in grads[..n_grads].iter().enumerate() {
                let gs = gs.ok_or(ChainError::FragmentInputsNotBuilt)?;
                out.push(a, copy(SelDst::Fixed { bank: Bank::Temp, reg: data + first_zero + k as u16 }, gs))
                    .map_err(oom)?;
            }
            first_zero + n_grads as u16
        } else if n_grads != 0 {
            return Err(ChainError::FragmentInputsNotBuilt);
        } else {
            first_zero
        };
        n_grads = 0;
        /* The offset's lookup word ends the block (pco_emit_nir_smp's order:
         * coordinates, divisor, level, then the lookup). */
        let first_zero = if let Some(word) = offset {
            out.push(a, copy(SelDst::Fixed { bank: Bank::Temp, reg: data + first_zero }, SelSrc::Imm(u32::from(word))))
                .map_err(oom)?;
            first_zero + 1
        } else {
            first_zero
        };
        /* The block is fixed temporaries nothing else writes, and the
         * program is straight-line, so a word zeroed for an earlier sample
         * is still zero: only a word some sample has since written with
         * data (a projective divisor, a level) is zeroed again. */
        for k in first_zero..16 {
            if !zeroed[usize::from(k)] {
                out.push(a, copy(SelDst::Fixed { bank: Bank::Temp, reg: data + k }, SelSrc::Imm(0))).map_err(oom)?;
                zeroed[usize::from(k)] = true;
            }
        }
        zeroed[..usize::from(first_zero)].fill(false);
        let shared = |r: u16| SelSrc::Fixed { bank: Bank::Shared, reg: r, mods: Mods::default() };
        out.push(
            a,
            SelGroup {
                op: SelOp::Sample { binding, proj, lod, fetch, offset, cube },
                ty: OpType::F32,
                dst: Some(SelDst::Fixed { bank: Bank::Temp, reg: dest }),
                srcs: [
                    Some(shared(TEX_DESC_DWORDS * slot)),
                    Some(temp(data)),
                    Some(shared(TEX_DESC_DWORDS * slot + 8)),
                ],
            },
        )
        .map_err(oom)?;
        out.push(a, SelGroup { op: SelOp::WaitData, ty: OpType::F32, dst: None, srcs: [None; 3] }).map_err(oom)?;
        for k in 0..4u16 {
            out.push(a, copy(SelDst::Virtual(VReg(r.0 + k as u32)), temp(dest + k))).map_err(oom)?;
        }
    }
    let outputs = a.alloc_copy(sel.outputs).map_err(oom)?;
    for (_, c) in outputs.iter_mut() {
        for s in c.iter_mut().flatten() {
            *s = shift(*s);
        }
    }
    Ok((
        Selected {
            groups: out.into_slice(),
            vregs,
            outputs,
            inputs: sel.inputs,
            discard: sel.discard,
            alpha_to_one: sel.alpha_to_one.map(|(x, f)| (shift(x), shift(f))),
        },
        sorted,
    ))
}

/// A fragment shader all the way to its record.
pub fn compile_fragment<'a>(
    m: &'a ir::Module<'a>,
    reflection: &crate::shaderc::online::reflect::Reflection<'a>,
    a: &'a Arena<'a>,
) -> Result<crate::shaderc::ShcShader, ChainError> {
    compile_fragment_for(m, reflection, a, 0)
}

/// [`compile_fragment`] for the blend variant `blend_key`: the record says
/// the program reads the pixel (`uses_fbfetch`), carries the key, and when
/// a factor reads the blend colour, where (`blend_consts_off`) -- the four
/// dwords counted in `uniform_dwords`, as pco's variants count them.
pub fn compile_fragment_for<'a>(
    m: &'a ir::Module<'a>,
    reflection: &crate::shaderc::online::reflect::Reflection<'a>,
    a: &'a Arena<'a>,
    blend_key: u32,
) -> Result<crate::shaderc::ShcShader, ChainError> {
    let m = super::abi::with_matrix_varyings_split(m, a).map_err(|_| ChainError::OutOfScratch)?;
    let image = a.alloc_filled(MAX_IMAGE_BYTES, 0u8).map_err(|_| ChainError::OutOfScratch)?;
    let BuiltFragment {
        len,
        temps,
        varyings,
        sampled,
        meta_off,
        ytf_off,
        ff_off,
        frag_z,
        frag_w,
        native_cube,
        uses_pntc,
        blend,
        blend_consts,
        output_regs,
    } = build_fragment_for(m, LIMITS, a, image, blend_key)?;
    let words = a.alloc_filled(len / 4, 0u32).map_err(|_| ChainError::OutOfScratch)?;
    for (w, c) in words.iter_mut().zip(image[..len].as_chunks::<4>().0) {
        *w = u32::from_le_bytes(*c);
    }
    let uniforms = uniform_rows_sampling(reflection, sampled, a)?;
    super::abi::assemble(&super::abi::UscProgram {
        stage: crate::shaderc::SHC_STAGE_FRAGMENT,
        code: words,
        temps,
        io_out: m.outputs,
        /* The varyings the program READS, in coefficient order. A declared
         * varying nobody reads would make the driver iterate it; pco lists
         * none for fvgtest_inner.frag, which declares two and reads
         * neither. */
        io_in: varyings,
        attribs: &[],
        attribs_read: 0,
        uniforms,
        uniform_dwords: reflection.uniform_dwords,
        meta_off,
    })
    .map_err(ChainError::Record)
    .map(|mut sh| {
        /* The point coordinate's iterator: the driver's PDS coefficient
         * program iterates it when the record says so (ShcShader.uses_pntc,
         * passed to pvr_pds_render_coeff_program since e00ada6c). */
        sh.uses_pntc = u8::from(uses_pntc);
        /* gl_FragCoord.z's set first, and W's whenever .w is read (the
         * driver's coefficient program, pvr_pds_render_coeff_program) */
        sh.uses_z = u8::from(frag_z);
        /* the draw binds the twiddled cube copy for this program */
        sh.native_cube = u8::from(native_cube);
        if frag_w {
            sh.uses_w = 1;
        }
        /* The front-face op: the driver DMAs it to the register before this
         * one-based offset, per draw (src/pvr/backend.rs). */
        if let Some(f) = ff_off {
            sh.fs_front_face_off1 = f + 1;
        }
        /* gl_FragCoord.y's (scale, offset) pair: the driver writes it per
         * draw from the target's flip_y (src/pvr/backend.rs). */
        if let Some(y) = ytf_off {
            sh.fragcoord_ytf_off1 = y + 1;
        }
        if let Some(b) = blend {
            sh.blend_key = blend_key;
            sh.uses_fbfetch = u8::from(b.reads_pixel());
            if b.class == crate::shaderc::SHC_RT_F16X4 {
                /* Two pixel output dwords, as pco's f16x4 records say. */
                sh.num_outputs = 2;
            }
        }
        if let Some(r) = output_regs {
            sh.num_outputs = r;
        }
        if let Some(c) = blend_consts {
            sh.blend_consts_off = c as u8;
            sh.uniform_dwords += 4;
        }
        sh
    })
}

/// Does sampler row (binding, array_len) cover a sampled binding? Element
/// k of an array of samplers is binding `binding + k`.
fn row_sampled(binding: u8, array_len: u8, sampled: &[u8]) -> bool {
    let end = u16::from(binding) + u16::from(array_len.max(1));
    sampled.iter().any(|&b| (u16::from(binding)..end).contains(&u16::from(b)))
}

/// The descriptor slots of a program that samples `sampled`, in order: every
/// element binding of each sampler declaration with an element sampled, and
/// any sampled binding no declaration covers. The driver's layout for the
/// rows [`uniform_rows_sampling`] keeps.
fn descriptor_bindings<'a>(
    decls: &[ir::UniformDecl<'_>],
    sampled: &[u8],
    a: &'a Arena<'a>,
) -> Result<&'a [u8], ChainError> {
    let mut all = [false; 256];
    for &b in sampled {
        all[usize::from(b)] = true;
    }
    for d in decls.iter().filter(|d| d.dword_off == crate::shaderc::SHC_UNIFORM_NO_BLOCK) {
        if row_sampled(d.binding, d.array_len, sampled) {
            for k in 0..u16::from(d.array_len.max(1)) {
                if let Some(e) = all.get_mut(usize::from(d.binding) + usize::from(k)) {
                    *e = true;
                }
            }
        }
    }
    let n = all.iter().filter(|x| **x).count();
    let out = a.alloc_filled(n, 0u8).map_err(|_| ChainError::OutOfScratch)?;
    for (o, b) in out.iter_mut().zip((0..=255u8).filter(|b| all[usize::from(*b)])) {
        *o = b;
    }
    Ok(out)
}

/// The uniform rows a fragment record carries: the block members and the
/// samplers the program samples (a whole array row when any element is). The driver gives each listed sampler a
/// descriptor, in binding order, before the block -- the layout
/// `expand_samples` reads.
pub fn uniform_rows_sampling<'a>(
    reflection: &crate::shaderc::online::reflect::Reflection<'a>,
    sampled: &[u8],
    a: &'a Arena<'a>,
) -> Result<&'a [crate::shaderc::ShcUniform], ChainError> {
    let all = crate::shaderc::online::reflect::uniform_rows(reflection, a).map_err(|_| ChainError::OutOfScratch)?;
    let keep = |u: &crate::shaderc::ShcUniform| {
        u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK || row_sampled(u.binding, u.array_len, sampled)
    };
    let n = all.iter().filter(|u| keep(u)).count();
    let out = a.alloc_filled(n, crate::shaderc::ShcUniform::ZERO).map_err(|_| ChainError::OutOfScratch)?;
    for (d, u) in out.iter_mut().zip(all.iter().filter(|u| keep(u))) {
        *d = *u;
    }
    Ok(out)
}

/// The uniform rows a USC record carries: the block members, and no
/// sampler.
///
/// No USC program samples (selection refuses `smp`), so a sampler row would
/// describe a descriptor nothing reads -- and a listed sampler is not
/// harmless: the driver places descriptors at the front of the shared file
/// and the uniform block after them (abi.md), while selection reads uniform
/// dword `k` at shared register `k`. pco lists no sampler for
/// fill_color.nodiscard.frag, whose front end reports `tex` statically used
/// in a branch the program folds away; ours did, and the survey's record
/// comparison caught it. When the texture path arrives, the rows it keeps
/// and the base selection reads from change together.
pub fn block_uniform_rows<'a>(
    reflection: &crate::shaderc::online::reflect::Reflection<'a>,
    a: &'a Arena<'a>,
) -> Result<&'a [crate::shaderc::ShcUniform], ChainError> {
    let all = crate::shaderc::online::reflect::uniform_rows(reflection, a).map_err(|_| ChainError::OutOfScratch)?;
    let n = all.iter().filter(|u| u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK).count();
    let out = a.alloc_filled(n, crate::shaderc::ShcUniform::ZERO).map_err(|_| ChainError::OutOfScratch)?;
    for (d, u) in out.iter_mut().zip(all.iter().filter(|u| u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK)) {
        *d = *u;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::powervr::sel::{Mods, OpType, SelDst, SelOp, SelSrc, SmpLod};

    /// An array of samplers: the descriptor slots are every element of each
    /// sampled row (what the driver lays out, draw.rs + build_desc_block),
    /// and a sample of `m3[1]` (binding 6 of `samplerCube m3[3]` at 5) reads
    /// slot 2 of {0, 5, 6, 7} -- not slot 1 of the sampled {0, 6}.
    #[test]
    fn an_array_element_sample_reads_its_slot_among_every_element() {
        let decl = |name, binding, array_len| ir::UniformDecl {
            name,
            dword_off: crate::shaderc::SHC_UNIFORM_NO_BLOCK,
            dwords: u16::from(array_len),
            gl_type: 0x8B60,
            array_len,
            binding,
            inner: 1,
            statically_used: true,
        };
        let decls = [decl("u_var.m0", 0, 1), decl("u_var.m1", 1, 1), decl("u_var.m3", 5, 3)];
        let v = |r: u32| Some(SelSrc::Virtual { v: VReg(r), mods: Mods::default() });
        let sample = |binding: u8, dst: u32| SelGroup {
            op: SelOp::Sample { binding, proj: false, lod: SmpLod::Implicit, fetch: false, offset: None, cube: false },
            ty: OpType::F32,
            dst: Some(SelDst::Virtual(VReg(dst))),
            srcs: [v(0), v(1), None],
        };
        let groups = [sample(0, 10), sample(6, 20)];
        let sel = Selected { groups: &groups, vregs: 40, outputs: &[], inputs: &[], discard: None, alpha_to_one: None };
        let mut buf = std::vec![0u8; 64 << 10];
        let a = Arena::new(&mut buf);
        assert_eq!(descriptor_bindings(&decls, &[0, 6], &a).unwrap(), &[0, 5, 6, 7], "m1 is not sampled");
        let (out, sampled) = expand_samples(sel, 8, &decls, &a).expect("expands");
        assert_eq!(sampled, &[0, 6]);
        let desc_regs: std::vec::Vec<u16> = out
            .groups
            .iter()
            .filter(|g| matches!(g.op, SelOp::Sample { binding: 6, .. }))
            .flat_map(|g| g.srcs.iter().flatten().copied())
            .filter_map(|s| match s {
                SelSrc::Fixed { bank: super::super::sel::Bank::Shared, reg, .. } => Some(reg),
                _ => None,
            })
            .collect();
        assert_eq!(desc_regs.first(), Some(&(TEX_DESC_DWORDS * 2)), "m3[1] reads slot 2: {desc_regs:?}");
        assert!(row_sampled(5, 3, &[6]) && !row_sampled(5, 3, &[8]) && !row_sampled(1, 1, &[0, 6]));
    }

    /// The sample data block's spare words are zeroed where nothing has
    /// zeroed them yet: all fourteen before the first sample, none before a
    /// projective one that follows (its divisor is word 2), and only word 2
    /// again before the plain sample after that.
    #[test]
    fn the_sample_block_is_zeroed_once_and_again_only_where_data_was_written() {
        let v = |r: u32| Some(SelSrc::Virtual { v: VReg(r), mods: Mods::default() });
        let sample = |proj: bool, dst: u32| SelGroup {
            op: SelOp::Sample { binding: 0, proj, lod: SmpLod::Implicit, fetch: false, offset: None, cube: false },
            ty: OpType::F32,
            dst: Some(SelDst::Virtual(VReg(dst))),
            srcs: [v(0), v(1), if proj { v(2) } else { None }],
        };
        let groups = [sample(false, 10), sample(true, 20), sample(false, 30)];
        let sel = Selected { groups: &groups, vregs: 40, outputs: &[], inputs: &[], discard: None, alpha_to_one: None };
        let mut buf = std::vec![0u8; 64 << 10];
        let a = Arena::new(&mut buf);
        let (out, _) = expand_samples(sel, 8, &[], &a).expect("expands");
        let zeros_before: std::vec::Vec<usize> = out
            .groups
            .split(|g| matches!(g.op, SelOp::Sample { .. }))
            .map(|run| run.iter().filter(|g| g.op == SelOp::Copy && g.srcs[0] == Some(SelSrc::Imm(0))).count())
            .collect();
        assert_eq!(zeros_before[..3], [14, 0, 1]);
    }
}
