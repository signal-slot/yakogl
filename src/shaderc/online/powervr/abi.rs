// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The shader record the runtime receives: what a built USC program has to
//! say about itself before anything can run it.
//!
//! [`super::assemble`] produces the bytes; this says what they are. The two
//! are separate because the bytes are checkable on their own -- they decode
//! back, they tile, they end -- while the record is a claim about the
//! program's interface that only the front end's reflection can supply.
//!
//! # Why the checks are here and not left to the driver
//!
//! Every field below is one the driver or the linker acts on without being
//! able to tell that it is wrong. `temps` becomes a PDS field that WRAPS
//! rather than saturating, so 253 is encoded as 0 and the GPU runs the shader
//! with no temporaries. A missing `gl_Position` links cleanly and draws
//! nothing. A varying without a name is silently skipped by
//! [`crate::shaderc::link::by_name`], so the fragment shader reads whatever
//! dword offset happened to be left in the record. None of those fail loudly
//! downstream, which is exactly why they are refused here.

use super::assemble::{vertex_output_offsets, walk_shader};
use crate::shaderc::online::ir::{IoDecl, IoKind};
use crate::shaderc::online::reflect::AttribRow;
use crate::shaderc::online::store;
use crate::shaderc::{ShcAttrib, ShcShader, ShcUniform, ShcVarying, SHC_STAGE_VERTEX};

/// The most temporaries a USC program may declare.
///
/// Not a USC limit but a PDS one: the DOUTU task-control word carries the
/// count in 6 bits of four-temp blocks, so 63 * 4 is the ceiling, and it
/// wraps rather than saturating. `driver/src/pvr_pds_render_gen.c` refuses
/// above this too; a compiler that never emits it means that refusal is never
/// reached, which is the order the two should fail in.
pub const USC_MAX_TEMPS: u16 = 252;

/// Why a built program cannot be described to the runtime.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UscAbiError {
    /// No code at all.
    CodeEmpty,
    /// The bytes are not a sequence of whole groups, or do not end on one
    /// carrying `end`. Measured by walking them, not by trusting the builder.
    NotWholeGroups,
    /// More temporaries than the PDS DOUTU field can carry. See
    /// [`USC_MAX_TEMPS`].
    TooManyTemps { temps: u16 },
    /// A vertex shader that writes no `gl_Position`. It would link and draw
    /// nothing.
    NoPositionOutput,
    /// More varyings than the record holds.
    TooManyVaryings { got: usize, max: usize },
    /// A varying with no name that follows no other: `link::by_name` places
    /// an unnamed entry after the one before it (a matrix's later columns),
    /// so a first one would be read at an offset nobody set.
    UnnamedVarying { index: usize },
    /// A name too long for the record's fixed field.
    NameTooLong { index: usize, len: usize },
    /// A varying that is not one to four components.
    BadComponents { index: usize, components: u8 },
    /// The keep arena is full.
    OutOfMemory { left: usize },
    /// More uniform rows than the record's count byte holds.
    TooManyUniforms { got: usize },
    /// More attributes than the record holds.
    TooManyAttribs { got: usize, max: usize },
    /// An attribute whose vtxin registers do not fit the record's byte.
    AttribOutOfRange { index: usize },
    /// A declared attribute the link's location map does not name
    /// ([`with_attrib_map`]): the GL core lists every declared attribute,
    /// so this is a map built from another source.
    AttribNotInMap { index: usize },
}

/// A built USC program, ready to be described.
pub struct UscProgram<'a> {
    pub stage: u32,
    /// The whole shader as [`super::assemble::build_shader`] emitted it.
    pub code: &'a [u32],
    pub temps: u16,
    /// The stage's declared outputs: a vertex shader's `gl_Position` and
    /// varyings, a fragment shader's colour. The front end's own reflection,
    /// which is what names the varyings -- and the PowerVR linker pairs the
    /// stages BY NAME, so this is load-bearing rather than decoration.
    pub io_out: &'a [IoDecl<'a>],
    /// A fragment shader's declared inputs. A vertex shader's are attributes,
    /// described by `attribs`.
    pub io_in: &'a [IoDecl<'a>],
    /// The vertex attributes, from the front end's reflection. The driver
    /// DMAs attribute `i`'s `components` dwords into vtxin from `vi_reg`, so
    /// `vi_reg` must be where selection reads it: [`vtxin_reg`].
    pub attribs: &'a [AttribRow<'a>],
    /// Bit `n` set when attribute location `n` gets vertex-input registers
    /// ([`attribs_placed`]: every location of an attribute the program reads
    /// any part of). A declared attribute the program never reads is
    /// left out of the record, as pco leaves it out: it gets no stream and
    /// no location an application could bind.
    pub attribs_read: u32,
    /// The uniform rows the GL core resolves names through, and the block
    /// size in dwords. A uniform's dword `k` is shared register
    /// `dword_off + k` (selection reads it there; no sampler base exists
    /// while samplers are refused).
    pub uniforms: &'a [ShcUniform],
    pub uniform_dwords: u16,
    /// A fragment shader that discards: the shared register its epilogue
    /// reads the per-draw control word from (absolute, past the
    /// descriptors and the block, as pco reports it). The driver DMAs
    /// `SHC_FS_META_DEFAULT` there and draws punch-through for
    /// `uses_discard`. `None` for every other shader.
    pub meta_off: Option<u16>,
}

/// Which attribute locations `m` reads, as a mask for
/// [`UscProgram::attribs_read`]. A location of 32 or more sets nothing, and
/// `assemble` then leaves that attribute out, which is a program that reads
/// an attribute the driver never fetches -- so it is refused there instead
/// of silently.
pub fn attribs_read(m: &crate::shaderc::online::ir::Module<'_>) -> Result<u32, UscAbiError> {
    use crate::shaderc::online::ir::{IoSlot, Op};
    let mut mask = 0u32;
    for op in m.instrs {
        if let Op::LoadInput { slot: IoSlot::Attrib(loc), .. } = op {
            if *loc >= 32 {
                return Err(UscAbiError::AttribOutOfRange { index: *loc as usize });
            }
            mask |= 1 << loc;
        }
    }
    Ok(mask)
}

/// The attribute locations that get vertex-input registers: every location
/// of each attribute the program reads any part of ([`attribs_read`]) --
/// all columns of a matrix read in only one, since the record lists and the
/// vertex fetch writes the whole attribute. [`vtxin_reg`]'s ranking is over
/// this set, the same one selection, the record and the id registers use.
pub fn attribs_placed(m: &crate::shaderc::online::ir::Module<'_>) -> Result<u32, UscAbiError> {
    use crate::shaderc::online::ir::IoKind;
    let read = attribs_read(m)?;
    let mut placed = read;
    for io in m.inputs.iter().filter(|io| io.kind == IoKind::Attribute) {
        let cols = u32::from(matrix_columns(io.gl_type).unwrap_or(1));
        let span = (0..cols).map(|c| u32::from(io.location) + c).filter(|l| *l < 32).fold(0u32, |m, l| m | 1 << l);
        if span & read != 0 {
            placed |= span;
        }
    }
    Ok(placed)
}

/// A vertex module and its attribute rows moved to the locations the link
/// decided (`ShcBackend::compile_vs_linked`, SHADERC_NOTES.md "Attribute
/// registers decided at link"): each declared attribute, found in `map` by
/// name, takes the map's location for its first column and the ones after
/// it for the rest, in the module's declarations, in every load and in its
/// reflection row. Two attributes the map binds to one location then load
/// the same slot, and [`vtxin_reg`]'s ranking over the placed locations
/// gives them one location's registers -- which is what lets a program
/// declare more attributes than there are registers, as long as they alias.
/// The declarations are kept in ascending location, as the module promises.
pub fn with_attrib_map<'a>(
    a: &'a crate::shaderc::online::arena::Arena<'a>,
    m: &'a crate::shaderc::online::ir::Module<'a>,
    attribs: &'a [AttribRow<'a>],
    map: &crate::shaderc::ShcAttribMap,
) -> Result<(&'a crate::shaderc::online::ir::Module<'a>, &'a [AttribRow<'a>]), UscAbiError> {
    use crate::shaderc::online::arena::AVec;
    use crate::shaderc::online::ir::{IoKind, IoSlot, Op};
    let oom = |_| UscAbiError::OutOfMemory { left: 0 };
    let entries = &map.b[..(map.count as usize).min(map.b.len())];
    let name_of = |n: &[core::ffi::c_char]| -> usize { n.iter().position(|c| *c == 0).unwrap_or(n.len()) };
    /* old location -> new, for every column of every declared attribute;
     * an attribute the link did not place (inactive: no load reads it) is
     * dropped from the declarations and the rows, and a load of it refused */
    let mut to = [None::<u8>; 256];
    let mut gone = [false; 256];
    let mut rows = AVec::with_capacity(a, attribs.len()).map_err(oom)?;
    for (i, r) in attribs.iter().enumerate() {
        let e = entries
            .iter()
            .find(|e| {
                let n = &e.name[..name_of(&e.name)];
                n.len() == r.name.len() && n.iter().zip(r.name.bytes()).all(|(c, b)| *c as u8 == b)
            })
            .ok_or(UscAbiError::AttribNotInMap { index: i })?;
        if e.location == crate::shaderc::SHC_ATTRIB_NO_LOCATION {
            for c in 0..matrix_columns(r.gl_type).unwrap_or(1) {
                if let Some(g) = gone.get_mut(usize::from(r.location) + usize::from(c)) {
                    *g = true;
                }
            }
            continue;
        }
        for c in 0..matrix_columns(r.gl_type).unwrap_or(1) {
            let old = usize::from(r.location) + usize::from(c);
            let new =
                e.location.checked_add(c).filter(|l| *l < 32).ok_or(UscAbiError::AttribOutOfRange { index: i })?;
            *to.get_mut(old).ok_or(UscAbiError::AttribOutOfRange { index: i })? = Some(new);
        }
        rows.push(a, AttribRow { location: e.location, ..*r }).map_err(oom)?;
    }
    let moved = |l: u8, i: usize| to[usize::from(l)].ok_or(UscAbiError::AttribOutOfRange { index: i });
    let mut instrs = AVec::with_capacity(a, m.instrs.len()).map_err(oom)?;
    for (i, op) in m.instrs.iter().enumerate() {
        instrs
            .push(
                a,
                match *op {
                    Op::LoadInput { slot: IoSlot::Attrib(l), comps } => {
                        Op::LoadInput { slot: IoSlot::Attrib(moved(l, i)?), comps }
                    }
                    other => other,
                },
            )
            .map_err(oom)?;
    }
    let mut inputs = AVec::with_capacity(a, m.inputs.len()).map_err(oom)?;
    for (i, d) in m.inputs.iter().enumerate() {
        if d.kind == IoKind::Attribute && to[usize::from(d.location)].is_none() && gone[usize::from(d.location)] {
            continue;
        }
        let location = if d.kind == IoKind::Attribute { moved(d.location, i)? } else { d.location };
        inputs.push(a, crate::shaderc::online::ir::IoDecl { location, ..*d }).map_err(oom)?;
    }
    inputs.as_mut_slice().sort_unstable_by_key(|d| d.location);
    let module = crate::shaderc::online::ir::Module { instrs: instrs.into_slice(), inputs: inputs.into_slice(), ..*m };
    Ok((&a.alloc_copy(core::slice::from_ref(&module)).map_err(oom)?[0], rows.into_slice()))
}

/// The columns of a matrix attribute type (GL_FLOAT_MAT2..4 and ES 3.00's
/// non-square GL_FLOAT_MATCxR: C columns of R rows), `None` for any other
/// type: each column is one attribute location.
pub const fn matrix_columns(gl_type: u16) -> Option<u8> {
    match gl_type {
        0x8B5A => Some(2),          /* GL_FLOAT_MAT2 */
        0x8B5B => Some(3),          /* GL_FLOAT_MAT3 */
        0x8B5C => Some(4),          /* GL_FLOAT_MAT4 */
        0x8B65 | 0x8B66 => Some(2), /* GL_FLOAT_MAT2x3, GL_FLOAT_MAT2x4 */
        0x8B67 | 0x8B68 => Some(3), /* GL_FLOAT_MAT3x2, GL_FLOAT_MAT3x4 */
        0x8B69 | 0x8B6A => Some(4), /* GL_FLOAT_MAT4x2, GL_FLOAT_MAT4x3 */
        _ => None,
    }
}

/// The vtxin register attribute location `loc`, component `lane`, is read
/// from, in a program that reads the locations `read` ([`attribs_read`]).
/// One definition, used by selection and by the record, so the DMA and the
/// reads cannot disagree. Four registers per READ location, in location
/// order, whatever the attribute's width, which wastes registers pco would
/// pack (it gave texture.vert's vec2 at location 0 `vi_reg` 4 and the vec4
/// at 1 `vi_reg` 0) and costs nothing the corpus comes near.
///
/// Ranked among the read locations, not placed at `4 * loc`: a declared
/// attribute the program never reads takes no register. dEQP-GLES2
/// attribute_location.bind_aliasing.max_inactive_* declare 32 attributes
/// of which 16 are read, interleaved, so the read ones sat at locations up
/// to 30 and the program was refused (VtxIn needed 65 of 64; AM62P host
/// pre-run). Aliasing that is read (max_cond_*) needs the link-time layout
/// (SHADERC_NOTES, "Attribute registers decided at link").
pub const fn vtxin_reg(read: u32, loc: u8, lane: u32) -> u16 {
    let below = if loc >= 32 { read } else { read & ((1u32 << loc) - 1) };
    below.count_ones() as u16 * 4 + lane as u16
}

/// The vtxin registers a vertex shader reads `gl_VertexID` and
/// `gl_InstanceID` from: after the attribute registers, the vertex ID first
/// and the instance ID next, each only when read -- the order pco's
/// pco_shc.c `alloc_vs_sysvals` gives them (mirroring Mesa's
/// pvr_alloc_vs_sysvals). The vertex fetch (PDS) writes them there from the
/// record's `vs_vertex_id_reg1` / `vs_instance_id_reg1` (backend.rs
/// `pvr_pds_render_vertex_program_ids`). One definition, used by selection
/// and by the record.
pub fn vs_id_regs(m: &crate::shaderc::online::ir::Module<'_>) -> (Option<u16>, Option<u16>) {
    use crate::shaderc::online::ir::{IoSlot, Op};
    let read = attribs_placed(m).unwrap_or(0);
    let mut past = 0u16;
    let (mut vid, mut iid) = (false, false);
    for op in m.instrs {
        if let Op::LoadInput { slot, .. } = op {
            match slot {
                IoSlot::Attrib(loc) => past = past.max(vtxin_reg(read, *loc, 0) + 4),
                IoSlot::VertexId => vid = true,
                IoSlot::InstanceId => iid = true,
                _ => {}
            }
        }
    }
    let v = vid.then_some(past);
    let i = iid.then_some(past + u16::from(vid));
    (v, i)
}

/// `m`'s outputs without the varyings the program never stores: pco lists
/// only what `nir_shader.info.outputs_written` holds (pco_shc.c
/// alloc_vs_varyings), so a declared, unwritten varying gets no vertex
/// output and no record entry -- Qt's particle shaders declare
/// `spriteData` in shaders that never set it. A fragment shader reading
/// such a varying reads an undefined value, as GLSL allows.
pub fn written_outputs<'a>(
    m: &crate::shaderc::online::ir::Module<'a>,
    a: &'a crate::shaderc::online::arena::Arena<'a>,
) -> Result<&'a [IoDecl<'a>], crate::shaderc::online::arena::ArenaError> {
    use crate::shaderc::online::ir::{IoKind, IoSlot, Op};
    let stored =
        |loc: u8| m.instrs.iter().any(|op| matches!(op, Op::StoreOutput { slot: IoSlot::Varying(l), .. } if *l == loc));
    let keep = |io: &&IoDecl<'a>| io.kind != IoKind::Varying || stored(io.location);
    let n = m.outputs.iter().filter(keep).count();
    if n == m.outputs.len() {
        return Ok(m.outputs);
    }
    let out = a.alloc_copy(m.outputs)?;
    let mut k = 0usize;
    for i in 0..out.len() {
        if keep(&&m.outputs[i]) {
            out[k] = m.outputs[i];
            k += 1;
        }
    }
    Ok(&out[..n])
}

/// `m` with each matrix varying declared as its columns: one declaration
/// per location, each the column's vector type. Since 3beded7 the front end
/// already declares a matrix varying once per location (`m`, `m[1]`), each
/// still typed as the matrix, and the link pairs the stages by those names:
/// such a declaration only takes its column's type here. A matrix declared
/// ONCE (location 0, stored to `Varying(0)` and `Varying(1)`) is expanded,
/// the later columns unnamed as pco's record lists them (measured with
/// pco_shc on dEQP's scalar_to_matrix.float_to_mat2). Split is recognised
/// by another varying of the same base name, the rule select.rs
/// `varying_columns` uses (7902405). Idempotent: a column is not a matrix.
pub fn with_matrix_varyings_split<'a>(
    m: &'a crate::shaderc::online::ir::Module<'a>,
    a: &'a crate::shaderc::online::arena::Arena<'a>,
) -> Result<&'a crate::shaderc::online::ir::Module<'a>, crate::shaderc::online::arena::ArenaError> {
    use crate::shaderc::online::arena::AVec;
    /* GL_FLOAT_MAT2..4 and the column's GL_FLOAT_VEC2..4. */
    let cols = |t: u16| match t {
        0x8B5A => Some((2u8, 0x8B50u16)),
        0x8B5B => Some((3, 0x8B51)),
        0x8B5C => Some((4, 0x8B52)),
        _ => None,
    };
    let is_matrix = |io: &IoDecl<'_>| io.kind == IoKind::Varying && cols(io.gl_type).is_some();
    if !m.outputs.iter().chain(m.inputs.iter()).any(is_matrix) {
        return Ok(m);
    }
    let base = |n: &'a str| n.split('[').next().unwrap_or(n);
    let split = |list: &'a [IoDecl<'a>]| -> Result<&'a [IoDecl<'a>], crate::shaderc::online::arena::ArenaError> {
        let mut out: AVec<'a, IoDecl<'a>> = AVec::new();
        for io in list {
            let already =
                list.iter().filter(|o| o.kind == IoKind::Varying && base(o.name) == base(io.name)).count() > 1;
            match cols(io.gl_type).filter(|_| io.kind == IoKind::Varying) {
                Some((_, column)) if already => out.push(a, IoDecl { gl_type: column, ..*io })?,
                Some((n, column)) => {
                    for c in 0..n {
                        out.push(
                            a,
                            IoDecl {
                                name: if c == 0 { io.name } else { "" },
                                location: io.location + c,
                                gl_type: column,
                                ..*io
                            },
                        )?;
                    }
                }
                None => out.push(a, *io)?,
            }
        }
        Ok(out.into_slice())
    };
    let module = crate::shaderc::online::ir::Module { outputs: split(m.outputs)?, inputs: split(m.inputs)?, ..*m };
    Ok(&a.alloc_copy(core::slice::from_ref(&module))?[0])
}

/// Describe a built program to the runtime.
pub fn assemble(p: &UscProgram<'_>) -> Result<ShcShader, UscAbiError> {
    if p.code.is_empty() {
        return Err(UscAbiError::CodeEmpty);
    }
    /* The structural check, done by walking rather than trusting: the groups
     * must tile the code exactly and the last must carry `end`. A program
     * that does not end leaves the GPU running, and on this family a program
     * that never ends is not a wrong picture but a stuck pipeline. */
    let bytes: &[u8] = unsafe { core::slice::from_raw_parts(p.code.as_ptr().cast::<u8>(), p.code.len() * 4) };
    match walk_shader(bytes) {
        Some((_, consumed, ended)) if consumed == bytes.len() && ended => {}
        _ => return Err(UscAbiError::NotWholeGroups),
    }
    if p.temps > USC_MAX_TEMPS {
        return Err(UscAbiError::TooManyTemps { temps: p.temps });
    }

    let mut sh = ShcShader::ZERO;
    sh.temps = p.temps;

    if p.stage == SHC_STAGE_VERTEX {
        /* No gl_PointSize unless the program writes one: the ZERO default
         * says "point size at dword 0", and the driver turns on the
         * point-sprite size for it (`psprite_size_pres` in
         * src/pvr/backend.rs). pco's records carry the sentinel when the
         * shader writes none; `tests/usc_limits.rs` compares this field and
         * every other against pco's for the same source. */
        sh.vs_psize_off = crate::shaderc::SHC_NO_PSIZE;
        let writes_psize = p.io_out.iter().any(|io| io.kind == IoKind::PointSize);
        if !p.io_out.iter().any(|io| io.kind == IoKind::Position) {
            return Err(UscAbiError::NoPositionOutput);
        }
        /* Position is not a varying: it has its own four dwords at the front
         * of the vertex output and no name to pair on. */
        let mut widths = [0u8; 16];
        let mut interps = [0u8; 16];
        let mut n = 0usize;
        for io in p.io_out.iter().filter(|io| io.kind == IoKind::Varying) {
            if n >= widths.len() {
                return Err(UscAbiError::TooManyVaryings { got: n + 1, max: widths.len() });
            }
            widths[n] = io.components;
            interps[n] = io.interp;
            n += 1;
        }
        let mut offs = [0u8; 16];
        let size = vertex_output_offsets(&widths[..n], &interps[..n], &mut offs)
            .map_err(|_| UscAbiError::BadComponents { index: 0, components: 0 })?;
        fill(&mut sh, p.io_out.iter().filter(|io| io.kind == IoKind::Varying))?;
        for (v, off) in sh.varyings.iter_mut().zip(offs.iter()) {
            v.vtxout_off = *off;
        }
        /* gl_PointSize takes the dword after the varyings
         * (`assemble::vertex_output_writes` writes it there). */
        if writes_psize {
            sh.vs_psize_off = size;
            sh.vs_output_size = size.checked_add(1).ok_or(UscAbiError::BadComponents { index: 0, components: 0 })?;
        } else {
            sh.vs_output_size = size;
        }
    } else {
        fill(&mut sh, p.io_in.iter().filter(|io| io.kind == IoKind::Varying))?;
        /* The varyings' vertex-output offsets, packed after gl_Position as
         * the vertex record packs them (pco's fragment records carry the
         * same numbers), and W: every iterator divides by it. */
        let n = sh.num_varyings as usize;
        let mut widths = [0u8; 16];
        let mut interps = [0u8; 16];
        for ((w, i), v) in widths.iter_mut().zip(interps.iter_mut()).zip(sh.varyings[..n].iter()) {
            *w = v.components;
            *i = v.interp;
        }
        let mut offs = [0u8; 16];
        vertex_output_offsets(&widths[..n], &interps[..n], &mut offs)
            .map_err(|_| UscAbiError::BadComponents { index: 0, components: 0 })?;
        for (v, off) in sh.varyings[..n].iter_mut().zip(offs.iter()) {
            v.vtxout_off = *off;
        }
        sh.uses_w = u8::from(n > 0);
        /* One colour output, and none of the per-draw blocks a fragment
         * shader can ask the driver for: no meta dword (the driver DMAs one
         * into sh[meta_off] when this is not the sentinel) and no blend
         * constants. pco's record for a constant-colour shader carries
         * exactly these three; left at ZERO, meta_off 0 would have the driver
         * overwrite shared register 0. */
        sh.num_outputs = 1;
        sh.meta_off = crate::shaderc::SHC_NO_META;
        if let Some(off) = p.meta_off {
            sh.meta_off = off;
            sh.uses_discard = 1;
        }
        sh.blend_consts_off = crate::shaderc::SHC_NO_BLEND_CONSTS;
    }

    if p.stage == SHC_STAGE_VERTEX {
        /* One row per location the vertex fetch fills: an attribute is one,
         * a matrix one per column (GL ES 2.0 2.10.4: a matN attribute is N
         * consecutive locations), each row carrying the matrix's own name and
         * type at its column's location -- the convention the GL core reads
         * back as one attribute (gl/program.rs attrib_base_location) and the
         * table's records already follow (glsl2usc splits the matrix
         * into columns for pco, which cannot take a matrix input). The front
         * end reads column c from location + c. A matrix is listed whole when
         * any column is read, so its first location is always there for
         * glGetAttribLocation. */
        let rows = |a: &AttribRow<'_>| -> u8 { matrix_columns(a.gl_type).unwrap_or(1) };
        let read = |a: &&AttribRow<'_>| {
            (0..rows(a)).any(|c| {
                let l = a.location as u32 + c as u32;
                l < 32 && p.attribs_read & (1 << l) != 0
            })
        };
        /* One row per LOCATION: attributes the link bound to one location
         * (aliases, [`with_attrib_map`]) share its registers and its fetch,
         * and the row is as wide as the widest of them. A compile-time
         * layout gives every attribute its own locations, so there each
         * attribute column is its own row, as before. */
        let mut seen = [false; 256];
        let mut n = 0usize;
        for a in p.attribs.iter().filter(read) {
            for c in 0..rows(a) {
                let l = a.location as usize + c as usize;
                if l < seen.len() && !seen[l] {
                    seen[l] = true;
                    n += 1;
                }
            }
        }
        if n > sh.attribs.len() {
            return Err(UscAbiError::TooManyAttribs { got: n, max: sh.attribs.len() });
        }
        let mut i = 0usize;
        for a in p.attribs.iter().filter(read) {
            for c in 0..rows(a) {
                let location = a.location.checked_add(c).ok_or(UscAbiError::AttribOutOfRange { index: i })?;
                if let Some(d) = sh.attribs[..i].iter_mut().find(|d| d.location == location) {
                    d.components = d.components.max(a.components);
                    continue;
                }
                let vi = vtxin_reg(p.attribs_read, location, 0);
                let vi = u8::try_from(vi).map_err(|_| UscAbiError::AttribOutOfRange { index: i })?;
                let d: &mut ShcAttrib = &mut sh.attribs[i];
                if a.name.len() >= d.name.len() {
                    return Err(UscAbiError::NameTooLong { index: i, len: a.name.len() });
                }
                for (ch, b) in d.name.iter_mut().zip(a.name.as_bytes()) {
                    *ch = *b as _;
                }
                d.location = location;
                /* a matrix row is one column: the attribute's slot width */
                d.components = a.components;
                d.vi_reg = vi;
                /* float32, the only format the vertex fetch builds today. */
                d.format = 0;
                d.gl_type = a.gl_type;
                i += 1;
            }
        }
        sh.num_attribs = i as u8;
    }

    let keep = store::keep();
    if !p.uniforms.is_empty() {
        let u: &'static [ShcUniform] =
            keep.alloc_copy(p.uniforms).map_err(|_| UscAbiError::OutOfMemory { left: store::keep().left() })?;
        sh.uniforms = u.as_ptr();
        sh.num_uniforms = u8::try_from(u.len()).map_err(|_| UscAbiError::TooManyUniforms { got: u.len() })?;
    }
    sh.uniform_dwords = p.uniform_dwords;
    /* 0: the whole block reaches the shared registers. */
    sh.uniform_sh_dwords = 0;

    let code: &'static [u32] =
        keep.alloc_copy(p.code).map_err(|_| UscAbiError::OutOfMemory { left: store::keep().left() })?;
    sh.usc = code.as_ptr();
    sh.usc_dw = code.len() as u32;
    Ok(sh)
}

/// Copy the varyings into the record, refusing what the linker cannot use.
fn fill<'a>(sh: &mut ShcShader, ios: impl Iterator<Item = &'a IoDecl<'a>> + Clone) -> Result<(), UscAbiError> {
    let count = ios.clone().count();
    if count > sh.varyings.len() {
        return Err(UscAbiError::TooManyVaryings { got: count, max: sh.varyings.len() });
    }
    for (i, io) in ios.enumerate() {
        if io.name.is_empty() && i == 0 {
            return Err(UscAbiError::UnnamedVarying { index: i });
        }
        if io.components == 0 || io.components > 4 {
            return Err(UscAbiError::BadComponents { index: i, components: io.components });
        }
        let dst: &mut ShcVarying = &mut sh.varyings[i];
        if io.name.len() >= dst.name.len() {
            return Err(UscAbiError::NameTooLong { index: i, len: io.name.len() });
        }
        for (d, b) in dst.name.iter_mut().zip(io.name.as_bytes()) {
            *d = *b as _;
        }
        dst.location = io.location;
        dst.components = io.components;
        dst.interp = io.interp;
    }
    sh.num_varyings = count as u8;
    Ok(())
}
