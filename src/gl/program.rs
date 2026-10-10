// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Program objects: attach/link shaders, uniform storage.
//!
//! glLinkProgram snapshots the attached shaders' compiled `ShcShader` into
//! the program (rvs/rfs) and asks the back end to validate/resolve the VS<->FS
//! wiring (`shaderc::backend().link`). Uniforms are kept as one packed dword
//! block per stage in the program (`GlProgram::vs_uniforms` / `fs_uniforms`);
//! `draw` uploads them and points the VS/FS push-const at them.
//!
//! Uniform names come from the shader reflection the compiler table carries
//! ([`ShcUniform`]: block member offsets in the stage's push-constant block,
//! or a sampler's binding). glLinkProgram builds one location table for the
//! program: a name present in both stages gets one location that writes both
//! blocks. Samplers are not in a block; glUniform1i on one records the texture
//! unit the sampler reads (`draw` wires the bound texture of that unit).
#![allow(non_snake_case)]

use core::ffi::{c_char, c_ulong};
use core::fmt::Write as _;

use super::consts::*;
use super::context::{
    self, gl_trace, obj_index, obj_name, GlContext, GlProgram, CAP_COMBINED_TEXTURE_UNITS, CAP_SAMPLERS,
    CAP_TEXTURE_UNITS, MAX_CONTEXTS, MAX_PROGRAMS, MAX_UNIFORM_DWORDS, MAX_VERTEX_ATTRIBS, OBJ_PROGRAM,
};
use super::draw;
use super::shader::{self, copy_out_text, log_text, INFO_LOG_SIZE};
use super::types::*;
use crate::shaderc::{
    self, packing, ShcAttrib, ShcAttribBinding, ShcAttribMap, ShcShader, ShcUniform, SHC_ATTRIB_NO_LOCATION,
    SHC_MAX_DECLARED_ATTRIBS, SHC_UNIFORM_NAME_MAX, SHC_UNIFORM_NO_BLOCK,
};
use crate::util::{self, Bytes, CStrWriter, Global};

/// The location index byte; Qt Quick 3D lit materials link 100+ after
/// flattening their light arrays.
const PROG_MAX_UNIFORMS: usize = 255;

/// One uniform location: where the value lives in each stage's block.
#[derive(Clone, Copy)]
struct ProgLocation {
    /// The reflection entry of the uniform: `uniforms[ref_index]` of the
    /// program's VS copy (`ref_stage` 0, when the VS declares it) or FS copy (1).
    ref_stage: u8,
    ref_index: u8,
    /// Dword offsets, -1 = not in that stage.
    vs_off: i32,
    fs_off: i32,
    /// A fragment-shader sampler: value = texture unit.
    fs_sampler: bool,
    /// A vertex-stage sampler: its binding PLUS ONE (0: none). The vertex
    /// stage has descriptors of its own (Qt Quick 3D reads bone matrices and
    /// morph targets from a texture in the vertex shader).
    vs_binding1: u8,
    /// A member of an array of structs, which the offline table flattens into
    /// one array per member ("A.B[0].C", `inner` elements per struct): the
    /// array a location of it addresses is the member's own inner array.
    flattened: bool,
}

struct ProgLocTable {
    u: [ProgLocation; PROG_MAX_UNIFORMS],
    count: u8,
}

/// Uniform blocks a program may have, both stages together: ES 3.0's
/// MAX_COMBINED_UNIFORM_BLOCKS minimum.
pub(super) const PROG_MAX_BLOCKS: usize = 24;

/// One active uniform block of a linked program: which block of each stage
/// it is (index + 1; 0 when the stage does not declare it) and the binding
/// point glUniformBlockBinding gave it (initially 0).
#[derive(Clone, Copy)]
pub(super) struct ProgBlock {
    pub vs: u8,
    pub fs: u8,
    pub binding: u8,
}

pub(super) struct ProgBlocks {
    pub b: [ProgBlock; PROG_MAX_BLOCKS],
    pub count: u8,
}

/// File-static state of this module, per context and program.
struct ProgState {
    loc: [[ProgLocTable; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// The active uniform blocks of each program (glLinkProgram).
    blocks: [[ProgBlocks; MAX_PROGRAMS]; MAX_CONTEXTS],
    log: [[[u8; INFO_LOG_SIZE]; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// glDeleteProgram of the current program: flagged (DELETE_STATUS true,
    /// still a program) until it stops being current (GL ES 3.0 2.12.3).
    delete_pending: [[bool; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// What the last glValidateProgram decided (GL_VALIDATE_STATUS). GL says
    /// the initial value is GL_FALSE and that a link resets it: it is the
    /// answer of a command that was run, not a property of the program, so it
    /// cannot be derived from the link status.
    validated: [[bool; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// How many varyings glTransformFeedbackVaryings asked the next link
    /// to capture (interleaved). This GPU path captures none, so a link
    /// with any fails (`glLinkProgram`).
    tf_varyings: [[u32; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// The last link compiled the vertex shader again with the attribute
    /// layout (`attrib_layout`): its record has one row per active location,
    /// so an aliased name may have no row, and glGetAttribLocation answers
    /// a bound name from the bindings.
    relinked: [[bool; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// With `relinked`: the program's own copy of the ACTIVE entries of the
    /// map the link decided (declared order, each with its location and
    /// declared type), from which GL_ACTIVE_ATTRIBUTES, glGetActiveAttrib,
    /// glGetAttribLocation and GL_ACTIVE_ATTRIBUTE_MAX_LENGTH answer: the
    /// record's rows are one per active location, so two names on one
    /// location have one row between them. A heap block (`util::malloc`),
    /// null when not relinked or when the compile did not describe its
    /// declared attributes (`gl_type` 0), in which case the rows answer.
    active_attribs: [[*mut ShcAttribMap; MAX_PROGRAMS]; MAX_CONTEXTS],
    /// Bring-up: names an application asked for that the program does not
    /// have, still to be logged (see [`glGetUniformLocation`]); `!started`
    /// means the budget has not been handed out yet.
    miss_log_left: u32,
    miss_log_started: bool,
}

static STATE: Global<ProgState> = Global::zeroed();

/// The error for a name that is not a program object (GL ES 3.0 2.5):
/// GL_INVALID_OPERATION when it names a shader, else GL_INVALID_VALUE. The
/// two object kinds are numbered separately here, so a name can be both a
/// shader and nothing else; that case takes the shader rule.
pub(super) fn not_a_program(c: &GlContext, name: GLuint) -> GLenum {
    if c.shader_ref(name).is_some() {
        GL_INVALID_OPERATION
    } else {
        GL_INVALID_VALUE
    }
}

/// The error for a name that is not a shader object (mirror of
/// [`not_a_program`]).
pub(super) fn not_a_shader(c: &GlContext, name: GLuint) -> GLenum {
    if c.program_ref(name).is_some() {
        GL_INVALID_OPERATION
    } else {
        GL_INVALID_VALUE
    }
}

/// Whether any program of the context has `shader` attached.
pub(super) fn shader_is_attached(c: &GlContext, shader: GLuint) -> bool {
    shader != 0 && c.objs_ref().programs.iter().any(|p| p.used && (p.vs == shader || p.fs == shader))
}

/// The reflection entry a location refers to. A location table is built from
/// the very tables it indexes (glLinkProgram), so the entry is there; an
/// index that is not reads as an empty uniform rather than panicking.
/// The widest name `active_uniform_name` can build: a reflection name plus the
/// "[0]" an array's GL name carries. The index substitution cannot grow it --
/// "[0]" is as short as a bracketed index gets.
const ACTIVE_NAME_MAX: usize = SHC_UNIFORM_NAME_MAX + 4;

/// How many active uniforms one location-table entry stands for.
///
/// GL ES 2.0 2.10.4 flattens a struct: an array of structs is not itself a
/// uniform, and neither is one of its elements -- only the leaves are, one per
/// outer index. The offline table keeps ONE row per member (`A.B[0].C`,
/// `inner` leaves per index), so a row of a struct array stands for
/// `array_len / inner` active uniforms and everything else stands for one.
fn entry_active_count(p: &GlProgram, l: &ProgLocation) -> u32 {
    if !l.flattened {
        return 1;
    }
    let u = location_ref(p, l);
    let inner = (u.inner as u32).max(1);
    ((u.array_len as u32).max(1) / inner).max(1)
}

/// `(entry, outer index)` of the `index`-th active uniform, walking the table
/// in order and counting what each entry stands for.
fn active_uniform_nth<'a>(p: &GlProgram, tab: &'a ProgLocTable, index: u32) -> Option<(&'a ProgLocation, u32)> {
    let mut seen = 0u32;
    for l in &tab.u[..tab.count as usize] {
        let n = entry_active_count(p, l);
        if index < seen + n {
            return Some((l, index - seen));
        }
        seen += n;
    }
    None
}

/// Every active uniform of the program outside its uniform blocks, as GL
/// counts them.
fn active_uniform_total(p: &GlProgram, tab: &ProgLocTable) -> u32 {
    tab.u[..tab.count as usize].iter().map(|l| entry_active_count(p, l)).sum()
}

/* ---- uniform blocks (ES 3.0 2.12.6) ----------------------------------------
 *
 * A linked program's blocks are its stages' (`shaderc::uniform_blocks`),
 * matched by name. Their members are active uniforms too: numbered after
 * the default-block ones, block by block, named as GL names them
 * (`Block.m`), with no location. */

/// The block table of the stage block `b` belongs to, and the block.
fn prog_block_desc(p: &GlProgram, b: &ProgBlock) -> Option<(shaderc::ShcBlocks, shaderc::ShcBlock)> {
    let (sh, k) = if b.vs != 0 { (&p.rvs, b.vs) } else { (&p.rfs, b.fs) };
    let t = shaderc::uniform_blocks(sh);
    t.blocks.get(usize::from(k).checked_sub(1)?).map(|d| (t, *d))
}

/// A block's name without an array element's index: `B[3]` is `B`.
fn block_base_name(n: &str) -> &str {
    match n.strip_suffix(']').and_then(|s| s.rfind('[').map(|i| &n[..i])) {
        Some(b) => b,
        None => n,
    }
}

/// The first element of the array of blocks block `k` belongs to; `k`
/// itself when it is no array's, or the first. The elements of an array of
/// blocks share their members' active uniforms (ES 3.0 2.12.6: each is
/// `B.m`, not `B[k].m`), which belong to the first element's block.
fn first_instance(p: &GlProgram, bl: &ProgBlocks, k: usize) -> usize {
    let Some((_, d)) = bl.b.get(k).and_then(|b| prog_block_desc(p, b)) else { return k };
    if !d.name.ends_with(']') {
        return k;
    }
    let base = block_base_name(d.name);
    bl.b[..k]
        .iter()
        .position(|b| {
            prog_block_desc(p, b).is_some_and(|(_, o)| o.name.ends_with(']') && block_base_name(o.name) == base)
        })
        .unwrap_or(k)
}

/// Every uniform-block member of the program, in active-uniform order: the
/// members of each block that is not a later element of an array of blocks.
fn block_members<'a>(
    p: &'a GlProgram,
    bl: &'a ProgBlocks,
) -> impl Iterator<Item = (usize, shaderc::ShcBlockMember)> + 'a {
    bl.b[..usize::from(bl.count)].iter().enumerate().flat_map(move |(i, b)| {
        let ms: &'static [shaderc::ShcBlockMember] = match prog_block_desc(p, b) {
            Some((t, d)) if first_instance(p, bl, i) == i => t.members_of(&d),
            _ => &[],
        };
        ms.iter().map(move |m| (i, *m))
    })
}

/// A block member's GL name as glGetActiveUniform hands it out: an array's
/// ends in "[0]", an array of one element's too (`float f[1]` is `B.f[0]`,
/// size 1; 30 cases of dEQP-GLES3 ubo.random). An array is the member with an
/// array stride, which std140 never makes 0.
fn block_member_name<'a>(m: &shaderc::ShcBlockMember, out: &'a mut [u8; ACTIVE_NAME_MAX + 64]) -> &'a [u8] {
    let n = m.name.as_bytes();
    let len = n.len().min(out.len() - 3);
    out[..len].copy_from_slice(&n[..len]);
    if m.array_size > 1 || m.array_stride != 0 {
        out[len..len + 3].copy_from_slice(b"[0]");
        return &out[..len + 3];
    }
    &out[..len]
}

/// Match the two stages' blocks by name: a block both declare must have the
/// same layout in both (ES 3.0 2.12.6.2, "... must match"), and a program
/// has at most [`PROG_MAX_BLOCKS`].
fn build_blocks(p: &GlProgram, out: &mut ProgBlocks, log: &mut [u8; INFO_LOG_SIZE]) -> i32 {
    out.count = 0;
    let (vs, fs) = (shaderc::uniform_blocks(&p.rvs), shaderc::uniform_blocks(&p.rfs));
    for (st, t) in [(0u8, vs), (1u8, fs)] {
        for (i, d) in t.blocks.iter().enumerate() {
            let found = out.b[..usize::from(out.count)]
                .iter()
                .position(|b| prog_block_desc(p, b).is_some_and(|(_, o)| o.name == d.name));
            let k = match found {
                Some(k) => {
                    let (ot, od) = prog_block_desc(p, &out.b[k]).expect("found by it");
                    let same = od.data_size == d.data_size
                        && ot.members_of(&od).len() == t.members_of(d).len()
                        && ot.members_of(&od).iter().zip(t.members_of(d)).all(|(a, b)| {
                            a.name == b.name
                                && a.offset == b.offset
                                && a.gl_type == b.gl_type
                                && a.array_size == b.array_size
                                && a.row_major == b.row_major
                        });
                    if !same {
                        write_log(
                            log,
                            format_args!(
                                "link error: uniform block '{}' is declared differently in the vertex and fragment shaders",
                                d.name
                            ),
                        );
                        return -1;
                    }
                    k
                }
                None => {
                    if usize::from(out.count) >= PROG_MAX_BLOCKS {
                        write_log(log, format_args!("link error: more than {PROG_MAX_BLOCKS} uniform blocks"));
                        return -1;
                    }
                    out.count += 1;
                    let k = usize::from(out.count) - 1;
                    out.b[k] = ProgBlock { vs: 0, fs: 0, binding: 0 };
                    k
                }
            };
            if st == 0 {
                out.b[k].vs = (i + 1) as u8;
            } else {
                out.b[k].fs = (i + 1) as u8;
            }
        }
    }
    0
}

/// The program's active uniform `index`: a default-block one (its table
/// entry and element) or a block member (its block and the member).
enum Active<'a> {
    Default(&'a ProgLocation, u32),
    Member(usize, shaderc::ShcBlockMember),
}

fn active_nth<'a>(p: &'a GlProgram, tab: &'a ProgLocTable, bl: &'a ProgBlocks, index: u32) -> Option<Active<'a>> {
    if let Some((l, k)) = active_uniform_nth(p, tab, index) {
        return Some(Active::Default(l, k));
    }
    let j = index.checked_sub(active_uniform_total(p, tab))?;
    block_members(p, bl).nth(j as usize).map(|(b, m)| Active::Member(b, m))
}

fn active_total(p: &GlProgram, tab: &ProgLocTable, bl: &ProgBlocks) -> u32 {
    active_uniform_total(p, tab) + block_members(p, bl).count() as u32
}

/// The GL name, size and type of an active uniform.
fn active_describe<'a>(
    p: &GlProgram,
    a: &Active<'_>,
    out: &'a mut [u8; ACTIVE_NAME_MAX + 64],
) -> (&'a [u8], GLint, GLenum) {
    match *a {
        Active::Default(l, k) => {
            let mut buf = [0u8; ACTIVE_NAME_MAX];
            let (n, size) = active_uniform_name(p, l, k, &mut buf);
            let len = n.len();
            out[..len].copy_from_slice(n);
            (&out[..len], size, location_ref(p, l).gl_type as GLenum)
        }
        Active::Member(_, m) => {
            let size = GLint::from(m.array_size.max(1));
            (block_member_name(&m, out), size, GLenum::from(m.gl_type))
        }
    }
}

/// The program's blocks and the location and block tables of `program` in
/// the current context, for the entry points below.
fn prog_tables(c: &GlContext, program: GLuint) -> (&'static ProgLocTable, &'static ProgBlocks) {
    // SAFETY: single-threaded by contract; read-only views of this module's
    // state, taken once per entry point.
    let st = unsafe { STATE.get() };
    (&st.loc[c.group as usize][obj_index(program)], &st.blocks[c.group as usize][obj_index(program)])
}

/// glTransformFeedbackVaryings: remember how many varyings the next link of
/// `program` is asked to capture (0 clears the request).
pub(super) fn set_transform_feedback_varyings(group: u8, program: GLuint, count: u32) {
    // SAFETY: as in `prog_tables`; the only writer of this field.
    let st = unsafe { STATE.get() };
    st.tf_varyings[usize::from(group)][obj_index(program)] = count;
}

/// The blocks of `program` and their binding points, for the draw-time fill.
pub(super) fn program_blocks(group: u8, program: GLuint) -> &'static ProgBlocks {
    // SAFETY: as in `prog_tables`.
    let st = unsafe { STATE.get() };
    &st.blocks[usize::from(group)][obj_index(program)]
}

#[no_mangle]
pub unsafe extern "C" fn glGetUniformBlockIndex(program: GLuint, uniformBlockName: *const GLchar) -> GLuint {
    gl_trace!(glGetUniformBlockIndex);
    const FUNC: &str = "glGetUniformBlockIndex";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return GL_INVALID_INDEX;
    };
    if program_or_error(c, program, FUNC).is_none() || uniformBlockName.is_null() {
        return GL_INVALID_INDEX;
    }
    let (_, bl) = prog_tables(c, program);
    let p = &c.objs_ref().programs[obj_index(program)];
    // SAFETY: a NUL-terminated string (GL contract).
    let name = util::cstr_bytes(uniformBlockName);
    let named = |want: &[u8]| {
        bl.b[..usize::from(bl.count)]
            .iter()
            .position(|b| prog_block_desc(p, b).is_some_and(|(_, d)| d.name.as_bytes() == want))
    };
    /* an element of an array of blocks may be asked for without its "[0]" */
    let mut with0 = [0u8; ACTIVE_NAME_MAX + 64];
    let n = name.len().min(with0.len() - 3);
    with0[..n].copy_from_slice(&name[..n]);
    with0[n..n + 3].copy_from_slice(b"[0]");
    named(name).or_else(|| named(&with0[..n + 3])).map_or(GL_INVALID_INDEX, |k| k as GLuint)
}

#[no_mangle]
pub unsafe extern "C" fn glGetUniformIndices(
    program: GLuint,
    uniformCount: GLsizei,
    uniformNames: *const *const GLchar,
    uniformIndices: *mut GLuint,
) {
    gl_trace!(glGetUniformIndices);
    const FUNC: &str = "glGetUniformIndices";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    if uniformCount < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if uniformIndices.is_null() {
        return;
    }
    let (tab, bl) = prog_tables(c, program);
    let p = &c.objs_ref().programs[obj_index(program)];
    let total = active_total(p, tab, bl);
    for i in 0..uniformCount as usize {
        // SAFETY: the application passes `uniformCount` names (GL contract).
        let np = if uniformNames.is_null() { core::ptr::null() } else { util::app_get(uniformNames, i) };
        let found = if np.is_null() {
            None
        } else {
            // SAFETY: a NUL-terminated string (GL contract).
            let want = util::cstr_bytes(np);
            (0..total).find(|&k| {
                let mut buf = [0u8; ACTIVE_NAME_MAX + 64];
                active_nth(p, tab, bl, k).is_some_and(|a| {
                    let (n, _, _) = active_describe(p, &a, &mut buf);
                    /* "a[0]" is also "a" (2.12.6) */
                    n == want || n.strip_suffix(b"[0]") == Some(want)
                })
            })
        };
        // SAFETY: the application passes room for `uniformCount` indices.
        util::app_set(uniformIndices, i, found.unwrap_or(GL_INVALID_INDEX));
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetActiveUniformsiv(
    program: GLuint,
    uniformCount: GLsizei,
    uniformIndices: *const GLuint,
    pname: GLenum,
    params: *mut GLint,
) {
    gl_trace!(glGetActiveUniformsiv);
    const FUNC: &str = "glGetActiveUniformsiv";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    if uniformCount < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let (tab, bl) = prog_tables(c, program);
    let p = &c.objs_ref().programs[obj_index(program)];
    let total = active_total(p, tab, bl);
    /* Every index is checked before anything is written (2.12.6). */
    for i in 0..uniformCount as usize {
        // SAFETY: the application passes `uniformCount` indices.
        let k = if uniformIndices.is_null() { u32::MAX } else { util::app_get(uniformIndices, i) };
        if k >= total {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return;
        }
    }
    let known = matches!(
        pname,
        GL_UNIFORM_TYPE
            | GL_UNIFORM_SIZE
            | GL_UNIFORM_NAME_LENGTH
            | GL_UNIFORM_BLOCK_INDEX
            | GL_UNIFORM_OFFSET
            | GL_UNIFORM_ARRAY_STRIDE
            | GL_UNIFORM_MATRIX_STRIDE
            | GL_UNIFORM_IS_ROW_MAJOR
    );
    if !known {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if params.is_null() {
        return;
    }
    for i in 0..uniformCount as usize {
        // SAFETY: as above.
        let k = util::app_get(uniformIndices, i);
        let Some(a) = active_nth(p, tab, bl, k) else { continue };
        let mut buf = [0u8; ACTIVE_NAME_MAX + 64];
        let (name, size, ty) = active_describe(p, &a, &mut buf);
        let v: GLint = match (pname, &a) {
            (GL_UNIFORM_TYPE, _) => ty as GLint,
            (GL_UNIFORM_SIZE, _) => size,
            (GL_UNIFORM_NAME_LENGTH, _) => name.len() as GLint + 1,
            (GL_UNIFORM_BLOCK_INDEX, Active::Member(b, _)) => *b as GLint,
            (GL_UNIFORM_OFFSET, Active::Member(_, m)) => m.offset as GLint,
            (GL_UNIFORM_ARRAY_STRIDE, Active::Member(_, m)) => m.array_stride as GLint,
            (GL_UNIFORM_MATRIX_STRIDE, Active::Member(_, m)) => m.matrix_stride as GLint,
            (GL_UNIFORM_IS_ROW_MAJOR, Active::Member(_, m)) => GLint::from(m.row_major),
            /* a default-block uniform: no block, no offset or strides */
            (GL_UNIFORM_IS_ROW_MAJOR, Active::Default(..)) => 0,
            _ => -1,
        };
        // SAFETY: room for `uniformCount` values (GL contract).
        util::app_set(params, i, v);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetActiveUniformBlockiv(
    program: GLuint,
    uniformBlockIndex: GLuint,
    pname: GLenum,
    params: *mut GLint,
) {
    gl_trace!(glGetActiveUniformBlockiv);
    const FUNC: &str = "glGetActiveUniformBlockiv";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    let (tab, bl) = prog_tables(c, program);
    let p = &c.objs_ref().programs[obj_index(program)];
    let k = uniformBlockIndex as usize;
    let Some((t, d)) = bl.b[..usize::from(bl.count)].get(k).and_then(|b| prog_block_desc(p, b)) else {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    };
    let b = bl.b[k];
    /* an element of an array of blocks lists the first element's members */
    let k0 = first_instance(p, bl, k);
    let first = active_uniform_total(p, tab)
        + (0..k0)
            .filter(|&i| first_instance(p, bl, i) == i)
            .filter_map(|i| prog_block_desc(p, &bl.b[i]))
            .map(|(t, d)| t.members_of(&d).len() as u32)
            .sum::<u32>();
    let n = t.members_of(&d).len();
    let v: GLint = match pname {
        GL_UNIFORM_BLOCK_BINDING => GLint::from(b.binding),
        GL_UNIFORM_BLOCK_DATA_SIZE => d.data_size as GLint,
        GL_UNIFORM_BLOCK_NAME_LENGTH => d.name.len() as GLint + 1,
        GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS => n as GLint,
        GL_UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER => GLint::from(b.vs != 0),
        GL_UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER => GLint::from(b.fs != 0),
        GL_UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES => {
            if !params.is_null() {
                for j in 0..n {
                    // SAFETY: room for GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS values.
                    util::app_set(params, j, (first + j as u32) as GLint);
                }
            }
            return;
        }
        _ => {
            c.set_error(GL_INVALID_ENUM, FUNC);
            return;
        }
    };
    // SAFETY: params is null or points at one GLint (GL contract).
    util::app_put(params, v);
}

#[no_mangle]
pub unsafe extern "C" fn glGetActiveUniformBlockName(
    program: GLuint,
    uniformBlockIndex: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    uniformBlockName: *mut GLchar,
) {
    gl_trace!(glGetActiveUniformBlockName);
    const FUNC: &str = "glGetActiveUniformBlockName";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    if bufSize < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let (_, bl) = prog_tables(c, program);
    let p = &c.objs_ref().programs[obj_index(program)];
    let Some((_, d)) =
        bl.b[..usize::from(bl.count)].get(uniformBlockIndex as usize).and_then(|b| prog_block_desc(p, b))
    else {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    };
    // SAFETY: name holds bufSize bytes, length one GLsizei, or null.
    util::copy_out_str(d.name.as_bytes(), bufSize, length, uniformBlockName);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformBlockBinding(
    program: GLuint,
    uniformBlockIndex: GLuint,
    uniformBlockBinding: GLuint,
) {
    gl_trace!(glUniformBlockBinding);
    const FUNC: &str = "glUniformBlockBinding";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let st = STATE.get();
    let bl = &mut st.blocks[c.group as usize][obj_index(program)];
    if uniformBlockIndex >= GLuint::from(bl.count)
        || uniformBlockBinding as usize >= context::MAX_UNIFORM_BUFFER_BINDINGS
    {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    bl.b[uniformBlockIndex as usize].binding = uniformBlockBinding as u8;
}

/// Before a draw: each active uniform block of `program`, read from the
/// range bound to its binding point, into the block's rows in each stage's
/// default block -- element by element, column by column, a row-major
/// matrix transposed, as glUniform* would have written them. A block whose
/// binding point holds no buffer (or one the CPU cannot read) leaves its
/// rows as they are, and so does a member past the bound range.
pub(super) fn fill_uniform_blocks(c: &mut GlContext, program: GLuint) {
    let bl = program_blocks(c.group, program);
    if bl.count == 0 {
        return;
    }
    let ubo = *context::uniform_buffers(c);
    let o = context::objects(c.group);
    let p = &o.programs[obj_index(program)];
    let (vs_ptr, vs_cap) = (p.vs_uniforms, p.vs_uniform_cap as usize);
    let (fs_ptr, fs_cap) = (p.fs_uniforms, p.fs_uniform_cap as usize);
    for b in &bl.b[..usize::from(bl.count)] {
        let bind = ubo[usize::from(b.binding)];
        let Some(buf) = o.buffers.get(bind.buffer as usize).filter(|x| x.used && bind.buffer != 0) else { continue };
        if buf.mem.cpu.is_null() {
            continue;
        }
        let whole = buf.size.max(0) as usize;
        let start = (bind.offset.max(0) as usize).min(whole);
        let len = if bind.size > 0 { (bind.size as usize).min(whole - start) } else { whole - start };
        // SAFETY: the buffer's storage is `size` bytes the CPU can read.
        let data = unsafe { core::slice::from_raw_parts((buf.mem.cpu as *const u8).add(start), len) };
        for (sh, k, ptr, cap) in [(&p.rvs, b.vs, vs_ptr, vs_cap), (&p.rfs, b.fs, fs_ptr, fs_cap)] {
            if k == 0 || ptr.is_null() {
                continue;
            }
            let t = shaderc::uniform_blocks(sh);
            let Some(d) = t.blocks.get(usize::from(k) - 1) else { continue };
            // SAFETY: the stage's block is `cap` dwords the program owns
            // (glLinkProgram), distinct from the other stage's.
            let block = unsafe { core::slice::from_raw_parts_mut(ptr as *mut u32, cap) };
            for m in t.members_of(d) {
                if let Some(row) = sh.uniform_table().get(usize::from(m.row)) {
                    copy_block_member(data, m, row, block);
                }
            }
        }
    }
}

/// One block member from its std140 place in `data` into its default-block
/// row: `(columns, rows)` of the type, each element at the row's own stride
/// (a column padded to its vector's alignment, as `reflect` lays rows out).
fn copy_block_member(data: &[u8], m: &shaderc::ShcBlockMember, row: &ShcUniform, block: &mut [u32]) {
    let (cols, rows) = gl_type_shape(m.gl_type);
    let n = usize::from(m.array_size.max(1));
    let el_dw = usize::from(row.dwords) / n.max(1);
    /* a column of R rows is strided to 2 dwords for R = 2, else 4 */
    let col_dw = if cols > 1 {
        if rows == 2 {
            2
        } else {
            4
        }
    } else {
        rows
    };
    let read =
        |at: usize| -> Option<u32> { data.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]])) };
    /* a bool is any non-zero word in the buffer, and 0 / 1 in the block, as
     * glUniform*i writes it */
    let boolean = matches!(u32::from(m.gl_type), GL_BOOL | GL_BOOL_VEC2 | GL_BOOL_VEC3 | GL_BOOL_VEC4);
    for e in 0..n {
        let base = m.offset as usize + e * m.array_stride as usize;
        for c in 0..cols {
            for r in 0..rows {
                let at = if cols > 1 {
                    if m.row_major {
                        base + r * m.matrix_stride as usize + c * 4
                    } else {
                        base + c * m.matrix_stride as usize + r * 4
                    }
                } else {
                    base + r * 4
                };
                let Some(v) = read(at) else { continue };
                let v = if boolean { u32::from(v != 0) } else { v };
                let dst = usize::from(row.dword_off) + e * el_dw + c * col_dw + r;
                if let Some(slot) = block.get_mut(dst) {
                    *slot = v;
                }
            }
        }
    }
}

/// `(columns, rows)` of a GL uniform type: a scalar is (1, 1), a vecN
/// (1, N), a matCxR (C, R).
fn gl_type_shape(t: u16) -> (usize, usize) {
    match u32::from(t) {
        GL_FLOAT_VEC2 | GL_INT_VEC2 | GL_UNSIGNED_INT_VEC2 | GL_BOOL_VEC2 => (1, 2),
        GL_FLOAT_VEC3 | GL_INT_VEC3 | GL_UNSIGNED_INT_VEC3 | GL_BOOL_VEC3 => (1, 3),
        GL_FLOAT_VEC4 | GL_INT_VEC4 | GL_UNSIGNED_INT_VEC4 | GL_BOOL_VEC4 => (1, 4),
        GL_FLOAT_MAT2 => (2, 2),
        GL_FLOAT_MAT3 => (3, 3),
        GL_FLOAT_MAT4 => (4, 4),
        GL_FLOAT_MAT2x3 => (2, 3),
        GL_FLOAT_MAT2x4 => (2, 4),
        GL_FLOAT_MAT3x2 => (3, 2),
        GL_FLOAT_MAT3x4 => (3, 4),
        GL_FLOAT_MAT4x2 => (4, 2),
        GL_FLOAT_MAT4x3 => (4, 3),
        _ => (1, 1),
    }
}

/// The GL name and array size of one active uniform, written into `out`.
///
/// Two rules, both from 2.10.4 and both about the same "[0]":
///
///   - an array's GL name ends in "[0]" -- `u_v` declared `[4]` is reported as
///     `u_v[0]` with size 4, and dEQP builds exactly that name to look it up;
///   - a struct array's rows are named for element zero (`u_lights[0].atten`),
///     so element k is that name with the first bracketed index rewritten, and
///     its size is the member's OWN array (`inner`), not the flattened total.
fn active_uniform_name<'a>(
    p: &GlProgram,
    l: &ProgLocation,
    k: u32,
    out: &'a mut [u8; ACTIVE_NAME_MAX],
) -> (&'a [u8], GLint) {
    let u = location_ref(p, l);
    let name = util::name_bytes(&u.name);
    let mut n = 0usize;
    let push = |out: &mut [u8; ACTIVE_NAME_MAX], n: &mut usize, b: u8| {
        if *n < ACTIVE_NAME_MAX {
            out[*n] = b;
            *n += 1;
        }
    };

    let size = if l.flattened {
        /* "A.B[0].C" -> "A.B[k].C": the first bracketed index is the outer one */
        let open = name.iter().position(|&b| b == b'[');
        let close = name.iter().position(|&b| b == b']');
        match (open, close) {
            (Some(o), Some(c)) if c > o => {
                for &b in &name[..=o] {
                    push(out, &mut n, b);
                }
                let mut digits = [0u8; 10];
                let mut d = 0usize;
                let mut v = k;
                loop {
                    digits[d] = b'0' + (v % 10) as u8;
                    d += 1;
                    v /= 10;
                    if v == 0 {
                        break;
                    }
                }
                while d > 0 {
                    d -= 1;
                    push(out, &mut n, digits[d]);
                }
                for &b in &name[c..] {
                    push(out, &mut n, b);
                }
            }
            /* no bracket to rewrite: the row is not shaped as advertised, so
             * report it as it stands rather than inventing a name */
            _ => {
                for &b in name {
                    push(out, &mut n, b);
                }
            }
        }
        (u.inner as GLint).max(1)
    } else {
        for &b in name {
            push(out, &mut n, b);
        }
        (u.array_len as GLint).max(1)
    };

    /* An array's name ends in "[0]" whatever its length: `float f[1]` is
     * `f[0]` of size 1. A flattened member's array is its `inner` one. */
    let array = if l.flattened { size > 1 } else { declared_array(p, l) };
    if array {
        for &b in b"[0]" {
            push(out, &mut n, b);
        }
    }
    (&out[..n], size)
}

fn location_ref<'a>(p: &'a GlProgram, l: &ProgLocation) -> &'a ShcUniform {
    static NO_UNIFORM: ShcUniform = ShcUniform::ZERO;
    let stage = if l.ref_stage == 0 { &p.rvs } else { &p.rfs };
    stage.uniform_table().get(l.ref_index as usize).unwrap_or(&NO_UNIFORM)
}

/// Is the (not flattened) row a location refers to declared as an array?
/// `array_len > 1` says so for every array but one of ONE element, which
/// `array_len` (1) cannot tell from a lone value; the online compiler's record
/// knows (`ShcBlocks::arrays`). A table shader's rows fall back to
/// `array_len > 1`. dEQP-GLES2 uniform_api.random 2 8 12 13 18 31 38 43 46
/// 47 90 99 failed on the Pi 4 and the i.MX8MP because `float f[1]` was
/// reported as `f`: dEQP names every array `f[0]`.
fn declared_array(p: &GlProgram, l: &ProgLocation) -> bool {
    let stage = if l.ref_stage == 0 { &p.rvs } else { &p.rfs };
    location_ref(p, l).array_len > 1 || shaderc::uniform_blocks(stage).is_array_row(l.ref_index as usize)
}

/// `snprintf` into an info log.
fn write_log(log: &mut [u8; INFO_LOG_SIZE], args: core::fmt::Arguments) {
    // SAFETY: the writer stays inside the INFO_LOG_SIZE bytes of `log`.
    let mut w = unsafe { CStrWriter::new(log.as_mut_ptr() as *mut c_char, INFO_LOG_SIZE) };
    let _ = w.write_fmt(args);
}

/// The program object `program`, or `None` after recording the GL error for
/// a name that is not one.
fn program_or_error<'a>(c: &'a mut GlContext, program: GLuint, func: &str) -> Option<&'a mut GlProgram> {
    if c.program_ref(program).is_none() {
        let e = not_a_program(c, program);
        c.set_error(e, func);
        return None;
    }
    c.program(program)
}
#[no_mangle]
pub unsafe extern "C" fn glCreateProgram() -> GLuint {
    gl_trace!(glCreateProgram);
    let Some(c) = context::current() else {
        return 0;
    };
    for h in 1..MAX_PROGRAMS {
        if !c.objs().programs[h].used {
            let p = &mut c.objs().programs[h];
            // SAFETY: every field of GlProgram is valid when all-zero.
            core::ptr::write_bytes(p as *mut GlProgram, 0, 1);
            p.used = true;
            // SAFETY: single-threaded by contract; taken once in this entry point.
            let st = STATE.get();
            st.loc[c.group as usize][h].count = 0;
            st.blocks[c.group as usize][h].count = 0;
            st.log[c.group as usize][h][0] = 0;
            st.delete_pending[c.group as usize][h] = false;
            st.validated[c.group as usize][h] = false;
            st.tf_varyings[c.group as usize][h] = 0;
            super::tf::program_reset(c.group, obj_name(h, OBJ_PROGRAM));
            return obj_name(h, OBJ_PROGRAM);
        }
    }
    c.set_error(GL_OUT_OF_MEMORY, "glCreateProgram");
    0
}

#[no_mangle]
pub unsafe extern "C" fn glAttachShader(program: GLuint, shader: GLuint) {
    gl_trace!(glAttachShader);
    const FUNC: &str = "glAttachShader";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let Some(type_) = c.shader_ref(shader).map(|s| s.type_) else {
        let e = not_a_shader(c, shader);
        c.set_error(e, FUNC);
        return;
    };
    let Some(p) = program_or_error(c, program, FUNC) else {
        return;
    };

    let slot = match type_ {
        GL_VERTEX_SHADER => &mut p.vs,
        GL_FRAGMENT_SHADER => &mut p.fs,
        _ => {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
    };
    if *slot != 0 {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    *slot = shader;
}

#[no_mangle]
pub unsafe extern "C" fn glDetachShader(program: GLuint, shader: GLuint) {
    gl_trace!(glDetachShader);
    const FUNC: &str = "glDetachShader";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    /* 0 is never a shader: without this check it matched an empty slot */
    if shader == 0 || c.shader_ref(shader).is_none() {
        let e = not_a_shader(c, shader);
        c.set_error(e, FUNC);
        return;
    }
    let p = &mut c.objs().programs[obj_index(program)];
    if p.vs == shader {
        p.vs = 0;
    } else if p.fs == shader {
        p.fs = 0;
    } else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    /* a shader glDeleteShader flagged goes with its last attachment */
    shader::release_if_orphaned(c, shader);
}

/* the draw wires a descriptor for every fragment sampler the link allows */
const _: () =
    assert!(CAP_TEXTURE_UNITS <= crate::backend::GLES_MAX_TEX_BINDS, "the draw wires every unit the link allows");

/// The link's sampler-count rule (OpenGL ES 2.0.25, section 2.10.4
/// "Samplers", and section 2.10.5 "Texture Access"):
///
/// ```text
/// The LinkProgram command will attempt to determine if the active samplers
/// in the shader(s) contained in the program object exceed the maximum
/// allowable limits. If it determines that the count of active samplers
/// exceeds the allowable limits, then the link fails (these limits can be
/// different for different types of shaders). Each active sampler variable
/// counts against the limit, even if multiple samplers refer to the same
/// texture image unit.
///
/// The maximum number of texture image units available to a vertex shader is
/// MAX_VERTEX_TEXTURE_IMAGE_UNITS; [...] The maximum number of texture image
/// units available to the fragment stage of the GL is MAX_TEXTURE_IMAGE_UNITS.
/// Both the vertex shader and fragment processing combined cannot use more
/// than MAX_COMBINED_TEXTURE_IMAGE_UNITS texture image units. If both the
/// vertex shader and the fragment processing stage access the same texture
/// image unit, then that counts as using two texture image units against the
/// MAX_COMBINED_TEXTURE_IMAGE_UNITS limit.
/// ```
///
/// A stage's active samplers are its [`ShcShader::sampler_bindings`], one per
/// sampler and per element of a sampler array. The combined count is the sum
/// of the two stages' counts, so a sampler both stages use counts twice.
/// Without this check a vertex stage with more samplers than
/// GLES_VS_TEX_UNITS reached the draw, which wires only that many
/// descriptors: on the AM62P the unwired ones hung the GPU.
///
/// With today's limits the combined rule never refuses a program the two
/// stage rules accepted: the vertex limit + CAP_TEXTURE_UNITS is 32 on the
/// V3D, the PowerVR and the GC7000, and CAP_COMBINED_TEXTURE_UNITS is 32. It
/// stays because the spec states it; the unit tests drive it with a smaller
/// combined limit ([`check_sampler_counts_against`]).
fn check_sampler_counts(vs: &ShcShader, fs: &ShcShader, log: &mut [u8; INFO_LOG_SIZE]) -> Result<(), ()> {
    check_sampler_counts_against(vs, fs, &SAMPLER_LIMITS, log)
}

/// The three limits the link's sampler-count rule applies.
struct SamplerLimits {
    /// GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS
    vertex: usize,
    /// GL_MAX_TEXTURE_IMAGE_UNITS
    fragment: usize,
    /// GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS
    combined: usize,
}

/// The limits the queries report: one constant per limit.
const SAMPLER_LIMITS: SamplerLimits = SamplerLimits {
    vertex: crate::backend::GLES_VS_TEX_UNITS,
    fragment: CAP_TEXTURE_UNITS,
    combined: CAP_COMBINED_TEXTURE_UNITS,
};

/// [`check_sampler_counts`] against the given limits.
fn check_sampler_counts_against(
    vs: &ShcShader,
    fs: &ShcShader,
    limits: &SamplerLimits,
    log: &mut [u8; INFO_LOG_SIZE],
) -> Result<(), ()> {
    let v = vs.sampler_bindings().count();
    let f = fs.sampler_bindings().count();
    let checks = [
        ("the vertex shader uses", v, limits.vertex, "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS"),
        ("the fragment shader uses", f, limits.fragment, "GL_MAX_TEXTURE_IMAGE_UNITS"),
        ("the vertex and fragment shaders use", v + f, limits.combined, "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS"),
    ];
    for (who, used, max, cap) in checks {
        if used > max {
            write_log(
                log,
                format_args!("link error: {who} {used} active samplers; this implementation exposes {max} ({cap})"),
            );
            glog!("yakogl: {}", Bytes(log_text(log)));
            return Err(());
        }
    }
    Ok(())
}

/// Whether uniform-table row `name` of `stage` is a flattened member of an
/// array of structs: the offline tables keep ONE row per member,
/// named "A.B[0].C", whose `array_len` counts that member in every element
/// and `inner` in one. The online compiler's rows can carry the same "[0]."
/// (`s[0].b[0].b`, a member's own array) and are never flattened. Read as
/// flattened, such a row's `inner` of 1 made it a scalar, and
/// glUniform2fv(s[0].b[0].b, 2, ..) was GL_INVALID_OPERATION until
/// 2026-09-24 (dEQP-GLES2 struct.uniform.nested_struct_array*, AM62P Zephyr).
fn row_is_flattened(stage: &ShcShader, name: &[u8]) -> bool {
    !shaderc::is_online(stage) && name.windows(4).any(|w| w == b"[0].")
}

/// The variables a stage's uniforms are for the Appendix A packing: one per
/// default-block row that has storage (a sampler has none: it is counted in
/// texture units, [`check_sampler_counts`]), with its GL type's kind and its
/// own array length. A flattened row ([`row_is_flattened`]) is that member
/// once per element of the outer array, each `inner` long -- the variables
/// glGetActiveUniform lists for it ([`entry_active_count`]).
///
/// A GL type the packing table does not know takes a full row per element,
/// the most one element can occupy, rather than nothing.
fn uniform_vars(stage: &ShcShader) -> impl Iterator<Item = packing::Var> + Clone + '_ {
    let blocks = shaderc::uniform_blocks(stage);
    stage.uniform_table().iter().enumerate().filter_map(move |(i, u)| {
        /* a uniform block's member is ES 3.00 buffer storage, not a
         * default-block uniform (as in build_locations) */
        if u.dword_off == SHC_UNIFORM_NO_BLOCK || blocks.members.iter().any(|m| usize::from(m.row) == i) {
            return None;
        }
        let kind = packing::Kind::of_gl_type(u32::from(u.gl_type)).unwrap_or(packing::Kind::Vec4);
        let all = u32::from(u.array_len).max(1);
        Some(if row_is_flattened(stage, util::name_bytes(&u.name)) {
            let inner = u32::from(u.inner).max(1);
            packing::Var { kind, len: inner, count: (all / inner).max(1) }
        } else {
            packing::Var::new(kind, all)
        })
    })
}

/// The link's uniform budget, per stage: the GLSL ES 1.00 Appendix A
/// packing ([`packing`], which quotes it) of the stage's uniforms on a grid
/// of GL_MAX_VERTEX_UNIFORM_VECTORS / GL_MAX_FRAGMENT_UNIFORM_VECTORS rows,
/// the values glGetIntegerv answers with. OpenGL ES 2.0.25 sections 2.10.4
/// and 3.8.1: "A link error will be generated if an attempt is made to
/// utilize more than the space available".
///
/// The rows counted are those the back end's reflection lists, which is the
/// storage it gives them: the offline tables list what pco keeps,
/// the online compiler every declared uniform. Literal constants are not
/// counted, although the appendix counts them: its algorithm is the minimum
/// an implementation must support ("The implementation is allowed to support
/// more than the minimum"), so counting fewer only accepts more. Whether a
/// back end has room for what that accepts is its own bound, which this
/// check does not see: the GC7000 keeps literals in the same constant file
/// as the uniforms (`ShcViv::const_count`).
fn check_uniform_packing(vs: &ShcShader, fs: &ShcShader, log: &mut [u8; INFO_LOG_SIZE]) -> Result<(), ()> {
    for (stage, budget) in [(vs, packing::Budget::VertexUniforms), (fs, packing::Budget::FragmentUniforms)] {
        if let Err(unfit) = packing::check(budget, uniform_vars(stage)) {
            write_log(log, format_args!("link error: {unfit}"));
            glog!("yakogl: {}", Bytes(log_text(log)));
            return Err(());
        }
    }
    Ok(())
}

/// Merge the two stages' reflection into the program's location table. A
/// uniform declared in both stages must agree in type and size (GL link rule).
fn build_locations(p: &mut GlProgram, tab: &mut ProgLocTable, log: &mut [u8; INFO_LOG_SIZE]) -> i32 {
    tab.count = 0;
    for st in 0..2u8 {
        let stage = if st == 0 { &p.rvs } else { &p.rfs };
        let blocks = shaderc::uniform_blocks(stage);
        for (i, u) in stage.uniform_table().iter().enumerate() {
            /* A uniform block's member is no default-block uniform: it has
             * no location, and is filled from the bound buffer at draw time
             * (`fill_uniform_blocks`). */
            if blocks.members.iter().any(|m| usize::from(m.row) == i) {
                continue;
            }
            let off = if u.dword_off == SHC_UNIFORM_NO_BLOCK { -1 } else { u.dword_off as i32 };
            let name = util::name_bytes(&u.name);

            let found = tab.u[..tab.count as usize]
                .iter()
                .position(|l| uniform_name_eq(util::name_bytes(&location_ref(p, l).name), name));
            let k = match found {
                Some(k) => {
                    let r = location_ref(p, &tab.u[k]);
                    if r.gl_type != u.gl_type || r.dwords != u.dwords {
                        write_log(
                            log,
                            format_args!(
                                "link error: uniform '{}' is declared with different types in the vertex and fragment shaders",
                                Bytes(name)
                            ),
                        );
                        return -1;
                    }
                    k
                }
                None => {
                    if tab.count as usize >= PROG_MAX_UNIFORMS {
                        write_log(log, format_args!("link error: more than {} uniforms", PROG_MAX_UNIFORMS));
                        return -1;
                    }
                    let k = tab.count as usize;
                    tab.count += 1;
                    let l = &mut tab.u[k];
                    /* the slot may be left over from an earlier program with
                     * the same id: a stale fs_sampler turned a plain int/float
                     * uniform into a texture unit (glUniform1f(1.0) -> 0x501) */
                    l.fs_sampler = false;
                    l.vs_binding1 = 0;
                    l.flattened = row_is_flattened(stage, name);
                    l.ref_stage = st;
                    l.ref_index = i as u8;
                    l.vs_off = -1;
                    l.fs_off = -1;
                    k
                }
            };
            let l = &mut tab.u[k];
            if st == 0 {
                l.vs_off = off;
            } else {
                l.fs_off = off;
            }
            if st == 0 && u.dword_off == SHC_UNIFORM_NO_BLOCK {
                /* a sampler the VERTEX shader reads (Qt Quick 3D's bone and
                 * morph-target textures): its unit goes to the vertex
                 * stage's descriptors, not the fragment's */
                if u.binding as usize >= CAP_SAMPLERS {
                    write_log(
                        log,
                        format_args!(
                            "link error: sampler '{}' binding {} is beyond {}",
                            Bytes(name),
                            u.binding,
                            CAP_SAMPLERS
                        ),
                    );
                    return -1;
                }
                l.vs_binding1 = u.binding + 1;
            }
            if st == 1 && u.dword_off == SHC_UNIFORM_NO_BLOCK {
                if u.binding as usize >= CAP_SAMPLERS {
                    write_log(
                        log,
                        format_args!(
                            "link error: sampler '{}' binding {} is beyond {}",
                            Bytes(name),
                            u.binding,
                            CAP_SAMPLERS
                        ),
                    );
                    return -1;
                }
                l.fs_sampler = true;
                /* the FS entry carries the binding */
                l.ref_stage = st;
                l.ref_index = i as u8;
            }
        }
    }
    p.fs_sampler_unit = [0; CAP_SAMPLERS];
    p.vs_sampler_unit = [0; CAP_SAMPLERS];
    0
}

/// The name after the struct instance a SPIR-V translation invents. Qt Quick
/// 3D's shaders declare their uniform block as a struct instance whose name
/// the translation numbers per shader ("_69.qt_modelViewProjection" in the
/// vertex shader, "_661.qt_modelViewProjection" in the fragment shader of the
/// same program), and the application asks for whichever of the two its own
/// reflection carries. Only a leading `_<digits>.` counts as such an
/// instance, so "light.color" and "fog.color" stay different uniforms.
fn uniform_after_instance(n: &[u8]) -> &[u8] {
    let Some(rest) = n.strip_prefix(b"_") else {
        return n;
    };
    let digits = rest.iter().take_while(|c| c.is_ascii_digit()).count();

    if digits != 0 && rest.get(digits) == Some(&b'.') {
        &rest[digits + 1..]
    } else {
        n
    }
}

/// A GLSL matrix type, which is the only reason two VS inputs share a name.
fn is_matrix_type(gl_type: u16) -> bool {
    matches!(
        gl_type as GLenum,
        GL_FLOAT_MAT2
            | GL_FLOAT_MAT3
            | GL_FLOAT_MAT4
            | GL_FLOAT_MAT2x3
            | GL_FLOAT_MAT2x4
            | GL_FLOAT_MAT3x2
            | GL_FLOAT_MAT3x4
            | GL_FLOAT_MAT4x2
            | GL_FLOAT_MAT4x3
    )
}

/// Two reflection rows that are columns of one matrix attribute. The matrix
/// TYPE is part of the test, not the name alone: `shc_attrib.name` is 32 bytes,
/// so two longer names truncate to the same text, and those are two attributes
/// rather than one attribute's columns.
fn same_matrix_attrib(a: &ShcAttrib, b: &ShcAttrib) -> bool {
    is_matrix_type(a.gl_type) && a.gl_type == b.gl_type && util::name_bytes(&a.name) == util::name_bytes(&b.name)
}

/// The lowest location among the reflection rows that are columns of `a`.
///
/// A matrix vertex attribute is **one** attribute occupying N consecutive
/// locations, one per column (GL ES 2.0 section 2.10.4). The shader front end
/// splits it into N inputs so the compiler never sees a matrix input, and each
/// of them reports the matrix's own name and type
/// (`tools/yakogl-tools/src/glsl.rs`, `MAT_ATTR_PREFIX`). So the reflection has
/// N rows where GL has one attribute: the one at the lowest location, and the
/// rest are its further columns. Every attribute of every other type is a
/// single row, for which this returns that row's own location.
fn attrib_base_location(p: &GlProgram, a: &ShcAttrib) -> u8 {
    let n = (p.rvs.num_attribs as usize).min(p.rvs.attribs.len());
    p.rvs.attribs[..n].iter().filter(|o| same_matrix_attrib(a, o)).map(|o| o.location).min().unwrap_or(a.location)
}

/// Which column of its attribute the reflection row `i` is: 0 for an ordinary
/// attribute and for a matrix's first column, 1.. for the rest.
fn attrib_column(p: &GlProgram, i: usize) -> u8 {
    let a = &p.rvs.attribs[i];
    a.location.saturating_sub(attrib_base_location(p, a))
}

/// Whether reflection row `i` is the attribute GL enumerates, rather than a
/// further column of one.
fn attrib_is_whole(p: &GlProgram, i: usize) -> bool {
    attrib_column(p, i) == 0
}

/// Two reflection names that are the same uniform.
fn uniform_name_eq(a: &[u8], b: &[u8]) -> bool {
    a == b || uniform_after_instance(a) == uniform_after_instance(b)
}

/// Log the attribute budget's refusal, worded as the packing's are
/// (`shaderc::packing::Unfit`). OpenGL ES 2.0.25 section 2.10.4 counts
/// attributes in generic attribute locations, not by the GLSL ES 1.00
/// Appendix A packing:
///
/// ```text
/// LinkProgram will fail if the assigned binding of an active attribute
/// variable would cause the GL to reference a non-existant generic attribute
/// (one greater than or equal to MAX_VERTEX_ATTRIBS). LinkProgram will fail
/// if the attribute bindings assigned by BindAttribLocation do not leave
/// enough space to assign a location for an active matrix attribute, which
/// requires multiple contiguous generic attributes.
/// ```
///
/// The same section's "A program object will fail to link if the number of
/// active vertex attributes exceeds MAX_VERTEX_ATTRIBS" is read as locations
/// too: aliasing puts two active attributes on one location, and dEQP's
/// `attribute_location.bind_aliasing.max_cond_*` binds 2 x MAX_VERTEX_ATTRIBS
/// of them and expects the program to link.
fn attribs_unfit(log: &mut [u8; INFO_LOG_SIZE], detail: core::fmt::Arguments<'_>) {
    write_log(
        log,
        format_args!(
            "link error: the vertex shader's attributes do not fit MAX_VERTEX_ATTRIBS = {} (OpenGL ES 2.0.25 section \
             2.10.4): {detail}",
            context::MAX_VERTEX_ATTRIBS
        ),
    );
    glog!("yakogl: {}", Bytes(log_text(log)));
}

/// Every attribute the vertex shader `vs` declares, with the location the link
/// gives it (`ShcAttribMap`), and whether two of them alias. A bound attribute
/// takes glBindAttribLocation's index (a matrix its columns from there); an
/// unbound one the lowest run of free locations, in declaration order. Err
/// (with a log line) when an unbound attribute finds no room.
fn attrib_layout(p: &GlProgram, vs: &ShcShader, log: &mut [u8; INFO_LOG_SIZE]) -> Result<(ShcAttribMap, bool), ()> {
    let mut map = ShcAttribMap::ZERO;
    /* the declared attributes: the compile's own list when it left the
     * layout to the link, else its rows (a matrix is one row per column,
     * all under its name) */
    let n = if vs.layout_at_link != 0 && !vs.declared.is_null() {
        // SAFETY: the compile's list of num_declared entries, kept with the shader.
        let declared = unsafe { core::slice::from_raw_parts(vs.declared, vs.num_declared as usize) };
        for (d, s) in map.b.iter_mut().zip(declared) {
            *d = ShcAttribBinding { location: SHC_ATTRIB_NO_LOCATION, ..*s };
        }
        declared.len().min(SHC_MAX_DECLARED_ATTRIBS)
    } else {
        let rows = &vs.attribs[..(vs.num_attribs as usize).min(vs.attribs.len())];
        let mut n = 0;
        for a in rows {
            let name = util::name_bytes(&a.name);
            if map.b[..n].iter().any(|d| util::name_bytes(&d.name) == name) {
                continue;
            }
            let columns = rows.iter().filter(|o| util::name_bytes(&o.name) == name).count();
            /* a row is an attribute the shader reads */
            map.b[n] = ShcAttribBinding {
                name: a.name,
                location: SHC_ATTRIB_NO_LOCATION,
                columns: columns as u8,
                gl_type: a.gl_type,
                active: 1,
            };
            n += 1;
        }
        n
    };
    map.count = n as u8;
    const LOCS: usize = context::MAX_VERTEX_ATTRIBS;
    let mut taken = [false; LOCS];
    let mut aliases = false;
    /* an attribute the shader never reads is not active (ES 2.0 2.10.4):
     * it takes no location and aliases nothing, bound or not */
    for d in map.b[..n].iter_mut().filter(|d| attrib_is_active(d)) {
        let bound = p.attrib_binds[..p.num_attrib_binds as usize]
            .iter()
            .rfind(|b| util::name_bytes(&b.name) == util::name_bytes(&d.name));
        if let Some(b) = bound {
            d.location = b.index;
            let end = (b.index as usize + d.columns.max(1) as usize).min(LOCS);
            for t in taken.iter_mut().take(end).skip(b.index as usize) {
                aliases |= *t;
                *t = true;
            }
        }
    }
    for d in map.b[..n].iter_mut().filter(|d| d.location == SHC_ATTRIB_NO_LOCATION && attrib_is_active(d)) {
        let cols = d.columns.max(1) as usize;
        let Some(l) = (0..=LOCS.saturating_sub(cols)).find(|&l| taken[l..l + cols].iter().all(|t| !t)) else {
            attribs_unfit(
                log,
                format_args!("no {cols} free location(s) left for '{}'", Bytes(util::name_bytes(&d.name))),
            );
            return Err(());
        };
        d.location = l as u8;
        taken[l..l + cols].iter_mut().for_each(|t| *t = true);
    }
    Ok((map, aliases))
}

/// A declared attribute the shader reads. One whose compile did not describe
/// it (`gl_type` 0, a back end from before the field) counts as read.
fn attrib_is_active(d: &ShcAttribBinding) -> bool {
    d.gl_type == 0 || d.active != 0
}

/// The link-time attribute layout (SHADERC_NOTES.md): when the vertex
/// shader's compile left its inputs to the link (`layout_at_link`) or the
/// bindings alias, compile it again with the layout and make that the
/// program's vertex shader. Ok(Some(the map it used)) when it did; Err after writing the link
/// error (the compiler's own log included).
fn link_attrib_layout(
    p: &mut GlProgram,
    vs_src: *const c_char,
    log: &mut [u8; INFO_LOG_SIZE],
) -> Result<Option<ShcAttribMap>, ()> {
    let deferred = p.rvs.layout_at_link != 0;
    let (map, aliases) = attrib_layout(p, &p.rvs, log)?;
    if !deferred && !aliases {
        return Ok(None);
    }
    let Some(relink) = shaderc::backend().and_then(|b| b.compile_vs_linked) else {
        if deferred {
            write_log(
                log,
                format_args!(
                    "link error: the vertex shader's attributes need their layout at link, which this shader compiler does not do"
                ),
            );
            return Err(());
        }
        return Ok(None); /* aliasing handled as before */
    };
    if vs_src.is_null() {
        write_log(log, format_args!("link error: the vertex shader's source is gone"));
        return Err(());
    }
    let mut out = ShcShader::ZERO;
    let mut clog = [0u8; INFO_LOG_SIZE];
    // SAFETY: the source is the shader object's NUL-terminated copy; `map`
    // and `out` live across the call; the log buffer is INFO_LOG_SIZE bytes.
    let rc = unsafe { relink(vs_src, &map, &mut out, clog.as_mut_ptr() as *mut c_char, INFO_LOG_SIZE) };
    if rc != 0 {
        write_log(
            log,
            format_args!(
                "link error: the vertex shader with the program's attribute locations: {}",
                Bytes(&clog[..clog.iter().position(|&b| b == 0).unwrap_or(clog.len())])
            ),
        );
        return Err(());
    }
    /* ES 2.0 2.10.4: more active locations than GL_MAX_VERTEX_ATTRIBS is a
     * link error; the record has one row per active location */
    let rows = (out.num_attribs as usize).min(out.attribs.len());
    let mut active = [false; 256];
    for a in &out.attribs[..rows] {
        active[a.location as usize] = true;
    }
    let count = active.iter().filter(|a| **a).count();
    if count > context::MAX_VERTEX_ATTRIBS {
        attribs_unfit(log, format_args!("{count} active locations"));
        shaderc::release_shader(&out);
        return Err(());
    }
    shaderc::release_shader(&p.rvs);
    p.rvs = out;
    Ok(Some(map))
}

/// The active entries of a link's attribute map, in declaration order, or
/// None when the compile did not describe its declared attributes (an entry
/// with no `gl_type`): then the record's rows answer, as before the fields.
fn active_entries(map: &ShcAttribMap) -> Option<ShcAttribMap> {
    let all = &map.b[..(map.count as usize).min(map.b.len())];
    if all.iter().any(|d| d.gl_type == 0) {
        return None;
    }
    let mut out = ShcAttribMap::ZERO;
    for d in all.iter().filter(|d| d.active != 0) {
        out.b[out.count as usize] = *d;
        out.count += 1;
    }
    Some(out)
}

/// Let go of a program's copy of its active attributes (`active_attribs`).
fn drop_active_attribs(slot: &mut *mut ShcAttribMap) {
    let m = core::mem::replace(slot, core::ptr::null_mut());
    if !m.is_null() {
        // SAFETY: from util::malloc in glLinkProgram, owned by this slot only.
        unsafe { util::free(m as *mut core::ffi::c_void) };
    }
}

/// The active attributes of a relinked program (`ProgState::active_attribs`).
fn relinked_attribs(group: usize, program: GLuint) -> Option<&'static ShcAttribMap> {
    // SAFETY: single-threaded by contract; a plain read of the slot.
    let m = unsafe { STATE.get() }.active_attribs[group][obj_index(program)];
    // SAFETY: a live block owned by the slot until the next link or delete.
    (!m.is_null()).then(|| unsafe { &*m })
}

/// Generic attribute index per VS input: the GLSL location, unless the app
/// bound the name to another index before the link (glBindAttribLocation).
/// A VS input location beyond the 16-entry table fails the link: the C read
/// `attrib_app_index[location]` past the array for it.
fn build_attrib_indices(p: &mut GlProgram, log: &mut [u8; INFO_LOG_SIZE]) -> i32 {
    for (i, a) in p.attrib_app_index.iter_mut().enumerate() {
        *a = i as u8;
    }
    let num_attribs = (p.rvs.num_attribs as usize).min(p.rvs.attribs.len());
    for i in 0..num_attribs {
        let a = p.rvs.attribs[i];
        if a.location as usize >= p.attrib_app_index.len() {
            write_log(
                log,
                format_args!(
                    "link error: attribute '{}' has location {}, beyond the {} vertex inputs",
                    Bytes(util::name_bytes(&a.name)),
                    a.location,
                    p.attrib_app_index.len()
                ),
            );
            return -1;
        }
        let mut app = a.location;

        /* glBindAttribLocation names an attribute, and a matrix attribute is
         * N of these rows: binding it to index k puts its columns at
         * k..k+N-1, so each row takes the bound index plus its own column.
         * Giving every column the same index would make them collide with
         * each other and fail the link below. */
        let column = attrib_column(p, i);
        for b in &p.attrib_binds[..p.num_attrib_binds as usize] {
            if util::name_bytes(&b.name) == util::name_bytes(&a.name) {
                app = b.index.saturating_add(column);
            }
        }
        if app as usize >= p.attrib_app_index.len() {
            write_log(
                log,
                format_args!(
                    "link error: attribute '{}' is bound to index {}, beyond the {} vertex inputs",
                    Bytes(util::name_bytes(&a.name)),
                    app,
                    p.attrib_app_index.len()
                ),
            );
            return -1;
        }
        p.attrib_app_index[a.location as usize] = app;
    }
    /* Two attributes on one index is ALIASING, and the GL allows it
     * (ES 2.0 2.10.4): it works when only one of them is active in the
     * executable, or when no path through the shader consumes more than one
     * of a set aliased together. A link error is permitted only when every
     * path consumes several, and "implementations are not required to
     * generate an error in this case" -- so refusing every alias, which this
     * used to do, refuses programs the GL says must link.
     *
     * dEQP holds us to it: `attribute_location.bind_aliasing.cond_*` and
     * `.max_cond_*` bind two names to one index behind a conditional that
     * reads one of them, and expect a linked program that draws.
     *
     * Nothing below needs protecting from it. `draw.rs` gives each input its
     * own staging from the one array the shared index names
     * (`two_inputs_bound_to_one_attribute_index_get_their_own_staging`), so
     * two inputs reading index 5 both read index 5, which is what aliasing
     * means. Detecting the every-path case would need the shader's control
     * flow, which the reflection does not carry, and the GL does not ask for
     * it. */
    0
}

/// Dwords of a stage's uniform block: the table's `uniform_dwords` (what the
/// shader reads and `draw` uploads), extended to the end of every member the
/// reflection places behind it. The reflection can lay a member out past
/// `uniform_dwords` (FemtoVG's discard fragment shaders: `frag[14]` spans
/// dwords 0..56 of a 52-dword block, `viewSize` sits at 56); a block sized to
/// `uniform_dwords` alone refused every write of those uniforms
/// (upstream bcef68b), so glUniform4fv(frag, 14, ..) never reached the GPU.
/// The upload size stays `uniform_dwords`: the dwords behind it are storage
/// the shader does not read.
fn block_dwords(sh: &shaderc::ShcShader) -> usize {
    let layout_end = sh
        .uniform_table()
        .iter()
        .filter(|u| u.dword_off != SHC_UNIFORM_NO_BLOCK)
        .map(|u| u.dword_off as usize + u.dwords as usize)
        .max()
        .unwrap_or(0);
    (sh.uniform_dwords as usize).max(layout_end).min(MAX_UNIFORM_DWORDS)
}

#[no_mangle]
pub unsafe extern "C" fn glLinkProgram(program: GLuint) {
    gl_trace!(glLinkProgram);
    const FUNC: &str = "glLinkProgram";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let ctx = c.group as usize;
    let group = c.group;
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    /* ES 3.0 2.11.7: a program an active transform feedback object uses
     * cannot be linked */
    if super::tf::program_in_use(c, program) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let Some(p) = program_or_error(c, program, FUNC) else {
        return;
    };
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let st = STATE.get();
    st.validated[ctx][obj_index(program)] = false; /* GL: a link resets it */
    if !crate::REPORT_ES2 {
        super::tf::link_begin(group, program);
    }
    let tab = &mut st.loc[ctx][obj_index(program)];
    let log = &mut st.log[ctx][obj_index(program)];
    let blocks = &mut st.blocks[ctx][obj_index(program)];

    p.linked = false;
    log[0] = 0;
    tab.count = 0;
    blocks.count = 0;
    p.free_uniforms();
    let (vs, fs) = (p.vs, p.fs);
    draw::invalidate_program(c, program);

    /* A stage the offline table lacks reports COMPILE_STATUS true so the
     * application compiles the other stage too (its text is logged once),
     * but no program can be made of it (upstream 18692b4). */
    let missing = |h: GLuint| c.shader_ref(h).is_some_and(|s| s.missing);
    if c.shader_ref(vs).is_some() && c.shader_ref(fs).is_some() && (missing(vs) || missing(fs)) {
        let stage = if missing(vs) { "vertex" } else { "fragment" };
        write_log(log, format_args!("link error: the {stage} shader is not in the offline table"));
        return;
    }
    let compiled = |h: GLuint| c.shader_ref(h).filter(|s| s.compiled).map(|s| s.compiled_shader);
    let (Some(rvs), Some(rfs)) = (compiled(vs), compiled(fs)) else {
        write_log(log, format_args!("link error: vertex and fragment shaders must be attached and compiled"));
        return;
    };
    /* ES 2.0 report: a link fails when the varyings to capture exceed
     * MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS, which is 0 there --
     * transform feedback is not implemented. */
    let tf = st.tf_varyings[ctx][obj_index(program)];
    if !crate::REPORT_ES2 {
        /* ES 3.0 2.11.8: the requested varyings are resolved against the
         * vertex shader's outputs; the link errors are the spec's (tf.rs) */
        let vs_src = c.shader_ref(vs).map_or(core::ptr::null(), |s| s.source);
        if super::tf::link_resolve(c.group, program, vs_src, &mut |a| write_log(log, a)).is_err() {
            return;
        }
    } else if tf > 0 {
        write_log(
            log,
            format_args!(
                "link error: {tf} transform feedback varying(s) requested; MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS is 0 (transform feedback is not implemented)"
            ),
        );
        return;
    }

    if check_sampler_counts(&rvs, &rfs, log).is_err() {
        return;
    }
    if check_uniform_packing(&rvs, &rfs, log).is_err() {
        return;
    }

    let p = &mut c.objs().programs[obj_index(program)];
    /* The program's own copies, held until it is linked again or deleted:
     * the shader objects may be recompiled or deleted meanwhile. Taken
     * before the old ones are let go, so a relink of the same shaders never
     * frees what it is about to keep. */
    shaderc::retain_shader(&rvs);
    shaderc::retain_shader(&rfs);
    shaderc::release_shader(&p.rvs);
    shaderc::release_shader(&p.rfs);
    p.rvs = rvs;
    p.rfs = rfs;
    st.relinked[ctx][obj_index(program)] = false;
    drop_active_attribs(&mut st.active_attribs[ctx][obj_index(program)]);
    /* The attribute layout, decided here from the bindings (SHADERC_NOTES.md,
     * "Attribute registers decided at link"): a vertex shader whose compile
     * left its inputs to the link, or whose bindings alias, is compiled
     * again with it. */
    let vs_src = c.shader_ref(vs).map_or(core::ptr::null(), |s| s.source);
    let p = &mut c.objs().programs[obj_index(program)];
    match link_attrib_layout(p, vs_src, log) {
        Err(()) => return,
        Ok(None) => {}
        Ok(Some(map)) => {
            st.relinked[ctx][obj_index(program)] = true;
            if let Some(active) = active_entries(&map) {
                // SAFETY: a plain heap block for one map, freed by drop_active_attribs.
                let m = unsafe { util::malloc(core::mem::size_of::<ShcAttribMap>()) } as *mut ShcAttribMap;
                if !m.is_null() {
                    // SAFETY: m is a fresh block of the map's size and alignment (malloc's).
                    unsafe { m.write(active) };
                    st.active_attribs[ctx][obj_index(program)] = m;
                }
            }
        }
    }
    /* the two blocks, as large as the stages' layouts */
    let vdw = block_dwords(&p.rvs);
    let fdw = block_dwords(&p.rfs);
    // SAFETY: plain C heap allocations (at least one dword each, like the C);
    // the program owns them until free_uniforms. Zeroed f32s are 0.0.
    unsafe {
        p.vs_uniforms = util::calloc(vdw.max(1), 4) as *mut f32;
        /* + BLEND_CONSTS_DWORDS: a blend variant's block is the shader's
         * uniforms followed by glBlendColor, and the backend snapshots the
         * whole variant block out of this array (see GlProgram). */
        p.fs_uniforms = util::calloc(fdw + context::BLEND_CONSTS_DWORDS, 4) as *mut f32;
    }
    if p.vs_uniforms.is_null() || p.fs_uniforms.is_null() {
        p.free_uniforms();
        write_log(log, format_args!("link error: out of memory for the uniform blocks"));
        c.set_error(GL_OUT_OF_MEMORY, FUNC);
        return;
    }
    p.vs_uniform_cap = vdw as u16;
    p.fs_uniform_cap = fdw as u16;
    p.fs_uniform_alloc = (fdw + context::BLEND_CONSTS_DWORDS) as u16;

    if let Some(link) = shaderc::backend().and_then(|b| b.link) {
        // SAFETY: both stages are valid ShcShader copies; the log buffer is
        // INFO_LOG_SIZE bytes.
        let rc = link(&p.rvs, &mut p.rfs, log.as_mut_ptr() as *mut c_char, INFO_LOG_SIZE);
        if rc != 0 {
            glog!("yakogl: {}", Bytes(log_text(log)));
            return;
        }
    }
    if build_locations(p, tab, log) != 0 {
        return;
    }
    if build_blocks(p, blocks, log) != 0 {
        return;
    }
    if build_attrib_indices(p, log) != 0 {
        return;
    }
    p.linked = true;
}

/// Destroy a program object: its GPU state, location table and blocks.
unsafe fn destroy_program(c: &mut GlContext, program: GLuint) {
    draw::release_program(c, program);
    // SAFETY: single-threaded by contract; the callers hold no other reference to STATE.
    let st = STATE.get();
    st.loc[c.group as usize][obj_index(program)].count = 0;
    st.blocks[c.group as usize][obj_index(program)].count = 0;
    st.delete_pending[c.group as usize][obj_index(program)] = false;
    st.validated[c.group as usize][obj_index(program)] = false;
    st.relinked[c.group as usize][obj_index(program)] = false;
    drop_active_attribs(&mut st.active_attribs[c.group as usize][obj_index(program)]);
    super::tf::program_reset(c.group, program);
    let (vs, fs) = (c.objs().programs[obj_index(program)].vs, c.objs().programs[obj_index(program)].fs);
    c.objs().programs[obj_index(program)].free_uniforms();
    shaderc::release_shader(&c.objs().programs[obj_index(program)].rvs);
    shaderc::release_shader(&c.objs().programs[obj_index(program)].rfs);
    // SAFETY: every field of GlProgram is valid when all-zero.
    core::ptr::write_bytes(&mut c.objs().programs[obj_index(program)] as *mut GlProgram, 0, 1);
    /* destroying a program detaches its shaders: one glDeleteShader flagged may go now */
    for s in [vs, fs] {
        if s != 0 {
            shader::release_if_orphaned(c, s);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn glUseProgram(program: GLuint) {
    gl_trace!(glUseProgram);
    const FUNC: &str = "glUseProgram";
    let Some(c) = context::current() else {
        return;
    };
    if program != 0 {
        let Some(linked) = c.program_ref(program).map(|p| p.linked) else {
            let e = not_a_program(c, program);
            c.set_error(e, FUNC);
            return;
        };
        if !linked {
            /* GL ES 3.0 2.12.3: no executable to install */
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
    }
    /* ES 3.0 2.11.7: no program change while transform feedback is active
     * and not paused */
    if super::tf::capturing(c) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let prev = c.cur_program;
    c.cur_program = program;
    // SAFETY: single-threaded by contract; taken once in this entry point.
    if prev != program && prev != 0 && STATE.get().delete_pending[c.group as usize][obj_index(prev)] {
        /* glDeleteProgram flagged it while it was current */
        destroy_program(c, prev);
    }
}

/// `strtoul(s, NULL, 10)` on a string that starts with a digit: saturates at
/// the C `unsigned long` of the target.
fn parse_ulong(s: &[u8]) -> c_ulong {
    let mut v: c_ulong = 0;
    for &b in s.iter().take_while(|b| b.is_ascii_digit()) {
        v = v.saturating_mul(10).saturating_add((b - b'0') as c_ulong);
    }
    v
}

/// A trailing `[k]` array subscript: the name before it and k.
fn split_subscript(name: &[u8]) -> Option<(&[u8], c_ulong)> {
    let body = name.strip_suffix(b"]")?;
    let lb = body.iter().rposition(|&b| b == b'[')?;
    let digits = &body[lb + 1..];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    Some((&body[..lb], parse_ulong(digits)))
}

/// Look `name` up in the location table: the location index and the array
/// element it addresses (GL ES 3.0 2.12.6: every active array element has a
/// location). `None`: not an active uniform.
fn find_location(p: &GlProgram, locs: &[ProgLocation], name: &[u8]) -> Option<(usize, u32)> {
    /* the same uniform under either stage's struct instance name counts as a
     * match (see `uniform_name_eq`) */
    let exact = |n: &[u8]| locs.iter().position(|l| uniform_name_eq(util::name_bytes(&location_ref(p, l).name), n));
    if let Some(i) = exact(name) {
        return Some((i, 0));
    }
    /* a trailing "[k]": element k of an array (of a plain array, or of the
     * inner array of a flattened struct member) */
    let (base, sub) = match split_subscript(name) {
        Some((b, k)) if !b.is_empty() => (b, Some(k)),
        _ => (name, None),
    };
    let (i, mut elem) = match sub.and_then(|_| exact(base)) {
        Some(i) => (i, 0 as c_ulong),
        None => find_struct_member(p, locs, base)?,
    };
    let u = location_ref(p, &locs[i]);
    let array_len = (u.array_len as c_ulong).max(1);
    if let Some(k) = sub {
        /* "[0]" names the array itself (and, as before, any uniform) */
        let limit = if locs[i].flattened { (u.inner as c_ulong).max(1) } else { array_len };
        if k != 0 && k >= limit {
            return None;
        }
        elem += k;
    }
    if elem >= array_len {
        return None;
    }
    Some((i, elem as u32))
}

/// "A.B[k].C": element k of an array-of-structs member the offline table
/// flattened per member (its name is "A.B[0].C", `inner` elements per k).
fn find_struct_member(p: &GlProgram, locs: &[ProgLocation], name: &[u8]) -> Option<(usize, c_ulong)> {
    let head = name.iter().position(|&b| b == b'[')?;
    let after = &name[head + 1..];
    if !after.first().is_some_and(|b| b.is_ascii_digit()) {
        return None;
    }
    let k = parse_ulong(after);
    let rb = after.iter().position(|&b| b == b']')?;
    if after.get(rb + 1) != Some(&b'.') {
        return None;
    }
    let tail = &after[rb + 1..];
    let mut key = [0u8; SHC_UNIFORM_NAME_MAX + 8];
    if head + 4 + tail.len() >= key.len() {
        return None;
    }
    key[..head].copy_from_slice(&name[..head]);
    key[head..head + 3].copy_from_slice(b"[0]");
    key[head + 3..head + 3 + tail.len()].copy_from_slice(tail);
    let key = &key[..head + 3 + tail.len()];

    let i = locs.iter().position(|l| util::name_bytes(&location_ref(p, l).name) == key)?;
    let u = location_ref(p, &locs[i]);
    /* checked: a saturated k must not wrap to a valid element */
    let elem = k.checked_mul((u.inner as c_ulong).max(1))?;
    Some((i, elem))
}

#[no_mangle]
pub unsafe extern "C" fn glGetUniformLocation(program: GLuint, name: *const GLchar) -> GLint {
    gl_trace!(glGetUniformLocation);
    const FUNC: &str = "glGetUniformLocation";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return -1;
    };
    let Some(p) = program_or_error(c, program, FUNC) else {
        return -1;
    };
    if !p.linked {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return -1;
    }
    if name.is_null() {
        return -1;
    }

    let p = &c.objs_ref().programs[obj_index(program)];
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let tab = &STATE.get().loc[c.group as usize][obj_index(program)];
    // SAFETY: name is a NUL-terminated string (GL contract).
    let name = util::cstr_bytes(name);
    /* "If name starts with the reserved prefix "gl_", a value of -1 is
     * returned" (ES 3.0 section 2.12.6): the online compiler's
     * gl_DepthRange rows are the GL core's to fill, not the application's. */
    if name.starts_with(b"gl_") {
        return -1;
    }
    match find_location(p, &tab.u[..tab.count as usize], name) {
        /* the element rides in the bits above the index byte */
        Some((i, elem)) => i as GLint | (elem << 8) as GLint,
        None => {
            log_missing_uniform(p, &tab.u[..tab.count as usize], program, name);
            -1 /* not an active uniform: no error (GL) */
        }
    }
}

/// Bring-up: a name the program does not have. An application that asks for
/// a uniform under a name the offline table does not carry leaves that
/// uniform at zero, which is invisible until the shader misbehaves. An array
/// element past what the shader declares (Qt asks for every light of its
/// fixed-size array) is ordinary, so those are skipped.
fn log_missing_uniform(p: &GlProgram, locs: &[ProgLocation], program: GLuint, name: &[u8]) {
    // SAFETY: single-threaded by contract; taken once here.
    let st = unsafe { STATE.get() };

    if !st.miss_log_started {
        st.miss_log_started = true;
        st.miss_log_left = MISS_LOG_BUDGET;
    }
    if st.miss_log_left == 0 {
        return;
    }
    let idx = (b'1'..=b'7').any(|d| name.windows(3).any(|w| w == [b'[', d, b']']));

    if idx || locs.iter().any(|l| uniform_name_eq(util::name_bytes(&location_ref(p, l).name), name)) {
        return;
    }
    st.miss_log_left -= 1;
    glog!("yakogl: program {} has no uniform '{}'", program, Bytes(name));
}

/// Lines `log_missing_uniform` prints per run.
const MISS_LOG_BUDGET: u32 = 200;

/* ---- uniform types and setters --------------------------------------------- */

/// The scalar kind of a uniform type.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Base {
    Float,
    Int,
    Uint,
    Bool,
    Sampler,
}

/// What one element of a uniform holds: `comps` values of `base`, laid out
/// as a `mat` x `mat` matrix when `mat` is not 0.
#[derive(Clone, Copy, PartialEq, Debug)]
struct Class {
    base: Base,
    comps: u8,
    mat: u8,
}

/// The class of a GL uniform type; `None` for a type no setter can load
/// (the reflection only carries the ES types below).
fn uniform_class(t: u16) -> Option<Class> {
    let k = |base, comps, mat| Some(Class { base, comps, mat });
    match t as u32 {
        GL_FLOAT => k(Base::Float, 1, 0),
        GL_FLOAT_VEC2 => k(Base::Float, 2, 0),
        GL_FLOAT_VEC3 => k(Base::Float, 3, 0),
        GL_FLOAT_VEC4 => k(Base::Float, 4, 0),
        GL_FLOAT_MAT2 => k(Base::Float, 4, 2),
        GL_FLOAT_MAT3 => k(Base::Float, 9, 3),
        GL_FLOAT_MAT4 => k(Base::Float, 16, 4),
        /* ES 3.0's non-square matrices: matCxR is C columns of R rows, and
         * `comps` is their product (the row count is comps / mat) */
        GL_FLOAT_MAT2x3 => k(Base::Float, 6, 2),
        GL_FLOAT_MAT3x2 => k(Base::Float, 6, 3),
        GL_FLOAT_MAT2x4 => k(Base::Float, 8, 2),
        GL_FLOAT_MAT4x2 => k(Base::Float, 8, 4),
        GL_FLOAT_MAT3x4 => k(Base::Float, 12, 3),
        GL_FLOAT_MAT4x3 => k(Base::Float, 12, 4),
        GL_INT => k(Base::Int, 1, 0),
        GL_INT_VEC2 => k(Base::Int, 2, 0),
        GL_INT_VEC3 => k(Base::Int, 3, 0),
        GL_INT_VEC4 => k(Base::Int, 4, 0),
        GL_UNSIGNED_INT => k(Base::Uint, 1, 0),
        GL_UNSIGNED_INT_VEC2 => k(Base::Uint, 2, 0),
        GL_UNSIGNED_INT_VEC3 => k(Base::Uint, 3, 0),
        GL_UNSIGNED_INT_VEC4 => k(Base::Uint, 4, 0),
        GL_BOOL => k(Base::Bool, 1, 0),
        GL_BOOL_VEC2 => k(Base::Bool, 2, 0),
        GL_BOOL_VEC3 => k(Base::Bool, 3, 0),
        GL_BOOL_VEC4 => k(Base::Bool, 4, 0),
        /* every ES 3.0 sampler is set with glUniform1i */
        GL_SAMPLER_2D
        | GL_SAMPLER_CUBE
        | GL_SAMPLER_3D
        | GL_SAMPLER_2D_SHADOW
        | GL_SAMPLER_2D_ARRAY
        | GL_SAMPLER_2D_ARRAY_SHADOW
        | GL_SAMPLER_CUBE_SHADOW => k(Base::Sampler, 1, 0),
        _ => None,
    }
}

/// The command family of a glUniform* call.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Setter {
    /// glUniform{n}f[v]
    Float(u8),
    /// glUniform{n}i[v]
    Int(u8),
    /// glUniform{n}ui[v]
    Uint(u8),
    /// glUniformMatrix{cols}fv, or ES 3.0's glUniformMatrix{cols}x{rows}fv.
    Mat { cols: u8, rows: u8 },
}

/// Whether a setter may load a uniform of class `k` (GL ES 3.0 2.12.6: the
/// size must match; bools take any scalar kind; samplers only glUniform1i[v]).
///
/// Known, deliberate deviation, one way round: the UNSIGNED setter loads an
/// int uniform. Qt's RHI chooses the setter from the SPIR-V type (uint) even
/// when the GLSL ES 1.00 text declares the uniform int (Qt Quick 3D's light
/// counts); the bit patterns are the same. The signed setter on a uint
/// uniform is what 2.12.6 says, GL_INVALID_OPERATION (dEQP-GLES3
/// negative_api.shader.uniformi[v]_incompatible_type); it was accepted too
/// until 2026-09-24.
fn setter_accepts(s: Setter, k: Class) -> bool {
    match s {
        Setter::Float(n) => k.mat == 0 && k.comps == n && matches!(k.base, Base::Float | Base::Bool),
        Setter::Int(n) => {
            (k.comps == n && matches!(k.base, Base::Int | Base::Bool)) || (n == 1 && k.base == Base::Sampler)
        }
        Setter::Uint(n) => k.comps == n && matches!(k.base, Base::Int | Base::Uint | Base::Bool),
        /* matCxR is C columns of R rows; `comps` is their product */
        Setter::Mat { cols, rows } => k.base == Base::Float && k.mat == cols && k.comps == cols * rows,
    }
}

/// A resolved uniform location: the location's offsets (moved to the
/// addressed array element) plus what the setters and getters need of the
/// reflection entry.
struct UniformEntry {
    class: Class,
    binding: u8,
    /// The array element the location addresses.
    elem: u32,
    /// Whether the uniform is an array (count > 1 is allowed).
    is_array: bool,
    /// Elements from `elem` to the end of the array (at least 1).
    elems: u32,
    /// Dwords per element in the block.
    stride: u32,
    vs_off: i32,
    fs_off: i32,
    fs_sampler: bool,
    /// A vertex-stage sampler: its binding plus one (0: none).
    vs_binding1: u8,
}

/// Resolve `location` on the linked program `program`. Returns the entry, or
/// `None` with GL_INVALID_OPERATION recorded (a location that is not one of
/// the program's, including -1: the setters skip -1 before calling this).
fn uniform_entry(c: &mut GlContext, st: &ProgState, program: GLuint, location: GLint) -> Option<UniformEntry> {
    const FUNC: &str = "uniform_entry";
    if !c.program_ref(program).is_some_and(|p| p.linked) || location < 0 {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return None;
    }

    let tab = &st.loc[c.group as usize][obj_index(program)];
    let index = (location & 0xff) as usize;
    let elem = location as u32 >> 8; /* the array element (glGetUniformLocation) */

    if index >= tab.count as usize {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return None;
    }
    let l = &tab.u[index];
    let prog = &c.objs_ref().programs[obj_index(program)];
    let r = location_ref(prog, l);
    let declared = declared_array(prog, l);
    let array_len = (r.array_len as u32).max(1);
    let Some(class) = uniform_class(r.gl_type) else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return None;
    };
    if elem >= array_len {
        c.set_error(GL_INVALID_OPERATION, FUNC); /* a location past the array */
        return None;
    }
    let (is_array, elems) = if l.flattened {
        /* the array a flattened member's location addresses is the member's
         * own (inner) array, not the struct array around it */
        let inner = (r.inner as u32).max(1);
        (inner > 1, (inner - elem % inner).min(array_len - elem))
    } else {
        /* an array of one element is still an array: `is_array` is what
         * lets a count past one through, the elements beyond the array
         * ignored (the count check where the entry is used) */
        (declared, array_len - elem)
    };
    let stride = r.dwords as u32 / array_len;
    /* offsets are u16 and elem < 256: no overflow */
    let base = (elem * stride) as i32;
    Some(UniformEntry {
        class,
        binding: r.binding,
        elem,
        is_array,
        elems,
        stride,
        vs_off: element_off(l.vs_off, base),
        fs_off: element_off(l.fs_off, base),
        fs_sampler: l.fs_sampler,
        vs_binding1: l.vs_binding1,
    })
}

/// Copy `src` bit for bit into a stage's block at dword `off`, unless the
/// uniform is not in that stage (`off` < 0) or `bound` dwords from `off`
/// would not fit the block.
fn store_dwords(block: &mut [f32], off: i32, bound: u32, src: &[u32]) {
    if off >= 0 && (off as u64) + (bound as u64) <= block.len() as u64 && src.len() <= bound as usize {
        for (d, s) in block[off as usize..][..src.len()].iter_mut().zip(src) {
            *d = f32::from_bits(*s);
        }
    }
}

/// Offset of an array element `base` dwords into the uniform; a stage the
/// uniform is not in keeps its negative offset.
fn element_off(off: i32, base: i32) -> i32 {
    if off >= 0 {
        off.saturating_add(base)
    } else {
        off
    }
}

/// The application's values of a glUniform*v call (any alignment).
#[derive(Clone, Copy)]
enum Values {
    Floats(*const GLfloat),
    Ints(*const GLint),
}

impl Values {
    fn is_null(self) -> bool {
        match self {
            Values::Floats(p) => p.is_null(),
            Values::Ints(p) => p.is_null(),
        }
    }

    /// Value `i` bit for bit.
    ///
    /// # Safety
    /// The pointer must hold more than `i` values.
    unsafe fn bits(self, i: usize) -> u32 {
        match self {
            Values::Floats(p) => util::app_get(p, i).to_bits(),
            Values::Ints(p) => util::app_get(p, i) as u32,
        }
    }

    /// Value `i` as a GLSL bool (GL: 0 / 0.0 is false, anything else true).
    ///
    /// # Safety
    /// As [`Values::bits`].
    unsafe fn truth(self, i: usize) -> bool {
        match self {
            Values::Floats(p) => util::app_get(p, i) != 0.0,
            Values::Ints(p) => util::app_get(p, i) != 0,
        }
    }
}

/// The glUniform* body: `count` elements of `values` into the uniform at
/// `location` of the current program, with GL ES 3.0 2.12.6's checks.
///
/// # Safety
/// `values` must be null or hold `count` elements of the setter's size.
unsafe fn set_uniform(location: GLint, count: GLsizei, setter: Setter, values: Values, transpose: bool) {
    const FUNC: &str = "store_uniform";
    let Some(c) = context::current() else {
        return;
    };
    if count < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* ES 2.0 has no transposing upload at all: glUniformMatrix*fv takes
     * GL_FALSE and nothing else (2.10). ES 3.0 added it, so this follows the
     * version reported -- and it is an argument error, so it comes before
     * the program and location rules. */
    if crate::REPORT_ES2 && transpose {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* GL ES 2.0 2.10: with no current linked program the call is
     * GL_INVALID_OPERATION, and that outranks location -1 -- which is only
     * silently ignored *while a linked program is in use*. */
    if !c.program_ref(c.cur_program).is_some_and(|p| p.linked) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    if location == -1 {
        return; /* GL: -1 is silently ignored */
    }
    // SAFETY: single-threaded by contract; taken once per entry point.
    let st = STATE.get();
    let cur = c.cur_program;
    let Some(e) = uniform_entry(c, st, cur, location) else {
        return;
    };
    if !setter_accepts(setter, e.class) {
        c.set_error(GL_INVALID_OPERATION, FUNC); /* type or size mismatch */
        return;
    }
    if count > 1 && !e.is_array {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    /* elements beyond the array are ignored (GL) */
    let n = (count as u32).min(e.elems) as usize;
    if n == 0 || values.is_null() {
        return; /* a null array has no defined meaning: nothing is read */
    }
    let p = &mut c.objs().programs[obj_index(cur)];
    if e.fs_sampler || e.vs_binding1 != 0 {
        /* a sampler holds the texture unit it reads, not a block value; the
         * whole command fails if one unit is out of range */
        for i in 0..n {
            let unit = values.bits(i) as GLint;
            if unit < 0 || unit >= CAP_COMBINED_TEXTURE_UNITS as GLint {
                c.set_error(GL_INVALID_VALUE, FUNC);
                return;
            }
        }
        for i in 0..n {
            if e.fs_sampler {
                let b = e.binding as usize + e.elem as usize + i;
                if b < CAP_SAMPLERS {
                    p.fs_sampler_unit[b] = values.bits(i) as u8;
                }
            }
            if e.vs_binding1 != 0 {
                let b = e.vs_binding1 as usize - 1 + e.elem as usize + i;
                if b < CAP_SAMPLERS {
                    p.vs_sampler_unit[b] = values.bits(i) as u8;
                }
            }
        }
        return;
    }

    let k = e.class;
    let per = k.comps as usize;
    for i in 0..n {
        let mut tmp = [0u32; 16];
        let first = i * per;
        let len = if k.mat != 0 {
            /* column-major in the block (std430), columns strided (a mat3
             * column takes a vec4's 4 dwords); padding is written as 0.
             * matCxR is `mat` columns of `comps / mat` rows, so the square
             * forms are the case where the two are equal. */
            let cols = k.mat as usize;
            let rows = per / cols;
            let col_stride = e.stride as usize / cols;
            if col_stride < rows || cols * col_stride > tmp.len() {
                return; /* a layout this cannot be: store nothing */
            }
            for col in 0..cols {
                for r in 0..rows {
                    let src = if transpose { first + r * cols + col } else { first + col * rows + r };
                    tmp[col * col_stride + r] = values.bits(src);
                }
            }
            cols * col_stride
        } else {
            for (j, d) in tmp[..per].iter_mut().enumerate() {
                /* a bool holds 0 or 1 whatever setter loaded it */
                *d = if k.base == Base::Bool { values.truth(first + j) as u32 } else { values.bits(first + j) };
            }
            per
        };
        let base = (i as u32 * e.stride) as i32;
        let (vb, fb) = p.blocks_mut();
        store_dwords(vb, element_off(e.vs_off, base), len as u32, &tmp[..len]);
        store_dwords(fb, element_off(e.fs_off, base), len as u32, &tmp[..len]);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glUniform1f(l: GLint, v0: GLfloat) {
    let v = [v0];
    set_uniform(l, 1, Setter::Float(1), Values::Floats(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform2f(l: GLint, v0: GLfloat, v1: GLfloat) {
    let v = [v0, v1];
    set_uniform(l, 1, Setter::Float(2), Values::Floats(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform3f(l: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat) {
    let v = [v0, v1, v2];
    set_uniform(l, 1, Setter::Float(3), Values::Floats(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform4f(l: GLint, v0: GLfloat, v1: GLfloat, v2: GLfloat, v3: GLfloat) {
    let v = [v0, v1, v2, v3];
    set_uniform(l, 1, Setter::Float(4), Values::Floats(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform1fv(l: GLint, count: GLsizei, value: *const GLfloat) {
    set_uniform(l, count, Setter::Float(1), Values::Floats(value), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform2fv(l: GLint, count: GLsizei, value: *const GLfloat) {
    set_uniform(l, count, Setter::Float(2), Values::Floats(value), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform3fv(l: GLint, count: GLsizei, value: *const GLfloat) {
    set_uniform(l, count, Setter::Float(3), Values::Floats(value), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform4fv(l: GLint, count: GLsizei, value: *const GLfloat) {
    set_uniform(l, count, Setter::Float(4), Values::Floats(value), false);
}

/* Integer uniforms (ints/bools/samplers) are stored bit-for-bit. For a sampler
 * the value is the texture unit the app binds its texture to. */
#[no_mangle]
pub unsafe extern "C" fn glUniform1i(location: GLint, v0: GLint) {
    let v = [v0];
    set_uniform(location, 1, Setter::Int(1), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform2i(l: GLint, v0: GLint, v1: GLint) {
    let v = [v0, v1];
    set_uniform(l, 1, Setter::Int(2), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform3i(l: GLint, v0: GLint, v1: GLint, v2: GLint) {
    let v = [v0, v1, v2];
    set_uniform(l, 1, Setter::Int(3), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform4i(l: GLint, v0: GLint, v1: GLint, v2: GLint, v3: GLint) {
    let v = [v0, v1, v2, v3];
    set_uniform(l, 1, Setter::Int(4), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform1iv(location: GLint, count: GLsizei, value: *const GLint) {
    set_uniform(location, count, Setter::Int(1), Values::Ints(value), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform2iv(l: GLint, count: GLsizei, value: *const GLint) {
    set_uniform(l, count, Setter::Int(2), Values::Ints(value), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform3iv(l: GLint, count: GLsizei, value: *const GLint) {
    set_uniform(l, count, Setter::Int(3), Values::Ints(value), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform4iv(l: GLint, count: GLsizei, value: *const GLint) {
    set_uniform(l, count, Setter::Int(4), Values::Ints(value), false);
}

/* Matrices are column-major in the block (std430), i.e. GL's natural order. */
#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix2fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 2, rows: 2 }, Values::Floats(value), transpose != 0);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix3fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 3, rows: 3 }, Values::Floats(value), transpose != 0);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix4fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 4, rows: 4 }, Values::Floats(value), transpose != 0);
}

/* OpenGL ES 3.0 matCxR: C columns of R rows. The column stride is the column
 * vector's own std430 alignment -- 2 dwords for a vec2 column, 4 for vec3 and
 * vec4 -- which is the rule the square forms above follow too, and which the
 * reflection's `stride` already carries. */
#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix2x3fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 2, rows: 3 }, Values::Floats(value), transpose != 0);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix3x2fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 3, rows: 2 }, Values::Floats(value), transpose != 0);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix2x4fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 2, rows: 4 }, Values::Floats(value), transpose != 0);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix4x2fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 4, rows: 2 }, Values::Floats(value), transpose != 0);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix3x4fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 3, rows: 4 }, Values::Floats(value), transpose != 0);
}

#[no_mangle]
pub unsafe extern "C" fn glUniformMatrix4x3fv(l: GLint, count: GLsizei, transpose: GLboolean, value: *const GLfloat) {
    set_uniform(l, count, Setter::Mat { cols: 4, rows: 3 }, Values::Floats(value), transpose != 0);
}

/* OpenGL ES 3.0's unsigned setters (see setter_accepts for the int/uint
 * interchangeability). */
#[no_mangle]
pub unsafe extern "C" fn glUniform1ui(location: GLint, v0: GLuint) {
    let v = [v0 as GLint];
    set_uniform(location, 1, Setter::Uint(1), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform2ui(location: GLint, v0: GLuint, v1: GLuint) {
    let v = [v0 as GLint, v1 as GLint];
    set_uniform(location, 1, Setter::Uint(2), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform3ui(location: GLint, v0: GLuint, v1: GLuint, v2: GLuint) {
    let v = [v0 as GLint, v1 as GLint, v2 as GLint];
    set_uniform(location, 1, Setter::Uint(3), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform4ui(location: GLint, v0: GLuint, v1: GLuint, v2: GLuint, v3: GLuint) {
    let v = [v0 as GLint, v1 as GLint, v2 as GLint, v3 as GLint];
    set_uniform(location, 1, Setter::Uint(4), Values::Ints(v.as_ptr()), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform1uiv(location: GLint, count: GLsizei, value: *const GLuint) {
    set_uniform(location, count, Setter::Uint(1), Values::Ints(value as *const GLint), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform2uiv(location: GLint, count: GLsizei, value: *const GLuint) {
    set_uniform(location, count, Setter::Uint(2), Values::Ints(value as *const GLint), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform3uiv(location: GLint, count: GLsizei, value: *const GLuint) {
    set_uniform(location, count, Setter::Uint(3), Values::Ints(value as *const GLint), false);
}

#[no_mangle]
pub unsafe extern "C" fn glUniform4uiv(location: GLint, count: GLsizei, value: *const GLuint) {
    set_uniform(location, count, Setter::Uint(4), Values::Ints(value as *const GLint), false);
}

#[no_mangle]
pub unsafe extern "C" fn glGetActiveUniform(
    program: GLuint,
    index: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    size: *mut GLint,
    type_: *mut GLenum,
    name: *mut GLchar,
) {
    gl_trace!(glGetActiveUniform);
    const FUNC: &str = "glGetActiveUniform";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    if bufSize < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }

    let (tab, bl) = prog_tables(c, program);
    let p = &c.objs_ref().programs[obj_index(program)];
    let Some(a) = active_nth(p, tab, bl, index) else {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    };
    let mut buf = [0u8; ACTIVE_NAME_MAX + 64];
    let (gl_name, gl_size, gl_type) = active_describe(p, &a, &mut buf);

    // SAFETY: size/type point at one value each, name at bufSize bytes, length
    // at one GLsizei, or they are null (GL contract); any alignment.
    util::app_put(size, gl_size);
    util::app_put(type_, gl_type);
    util::copy_out_str(gl_name, bufSize, length, name);
}

/// The application's output array of a glGetUniform*v call (any alignment).
#[derive(Clone, Copy)]
enum Out {
    Floats(*mut GLfloat),
    Ints(*mut GLint),
}

/// A uniform value of `base` (bit pattern `bits`) as the getter's type, with
/// GL ES 3.0 6.1.2's conversions: a float read as int is rounded to the
/// nearest integer, an unsigned value beyond INT_MAX clamps, a bool is 0 / 1.
///
/// # Safety
/// `out` must hold more than `i` values.
unsafe fn put_value(out: Out, i: usize, base: Base, bits: u32) {
    match out {
        Out::Floats(p) => {
            let f = match base {
                Base::Float => f32::from_bits(bits),
                Base::Int | Base::Sampler => bits as i32 as f32,
                Base::Uint => bits as f32,
                Base::Bool => (bits != 0) as u32 as f32,
            };
            util::app_set(p, i, f);
        }
        Out::Ints(p) => {
            let v = match base {
                /* rounded half away from zero, as the C did (no libm in
                 * no_std); `as` saturates and maps NaN to 0 */
                Base::Float => {
                    let f = f32::from_bits(bits);
                    (if f >= 0.0 { f + 0.5 } else { f - 0.5 }) as i32
                }
                Base::Int | Base::Sampler => bits as i32,
                Base::Uint => bits.min(i32::MAX as u32) as i32,
                Base::Bool => (bits != 0) as i32,
            };
            util::app_set(p, i, v);
        }
    }
}

/// glGetUniform*v: the value of the uniform element `location` addresses
/// (one element: `comps` values, a matrix column-major without the block's
/// column padding), converted to the getter's type.
unsafe fn get_uniform(program: GLuint, location: GLint, out: Out) {
    const FUNC: &str = "uniform_read_ptr";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let Some(p) = program_or_error(c, program, FUNC) else {
        return;
    };
    if !p.linked {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    // SAFETY: single-threaded by contract; taken once per entry point.
    let Some(e) = uniform_entry(c, STATE.get(), program, location) else {
        return;
    };
    let null = match out {
        Out::Floats(p) => p.is_null(),
        Out::Ints(p) => p.is_null(),
    };
    let p = &c.objs_ref().programs[obj_index(program)];
    let (block, off) = if e.fs_sampler || e.vs_binding1 != 0 {
        if !null {
            /* samplers read back their texture unit */
            let (units, b) = if e.fs_sampler {
                (&p.fs_sampler_unit, e.binding as usize + e.elem as usize)
            } else {
                (&p.vs_sampler_unit, e.vs_binding1 as usize - 1 + e.elem as usize)
            };
            let unit = units.get(b).copied().unwrap_or(0);
            put_value(out, 0, Base::Sampler, unit as u32);
        }
        return;
    } else if e.vs_off >= 0 {
        (p.vs_block(), e.vs_off as usize)
    } else if e.fs_off >= 0 {
        (p.fs_block(), e.fs_off as usize)
    } else {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if null {
        return;
    }
    let k = e.class;
    /* matCxR is `mat` columns of `comps / mat` rows, each column strided as
     * `set_uniform` writes it (a column of 3 rows takes 4 dwords): a
     * non-square matrix's rows are not its columns (dEQP-GLES3
     * uniform_api.value.*.get_uniform.basic.mat2x3, mat2x4, mat3x4, mat4x3
     * read the wrong dwords until 2026-10-09) */
    let (rows, col_stride) = if k.mat != 0 {
        let rows = k.comps as usize / k.mat as usize;
        (rows, (e.stride as usize / k.mat as usize).max(rows))
    } else {
        (1, 1)
    };
    for i in 0..k.comps as usize {
        /* value i of a matrix is column i / rows, row i % rows */
        let dw = if k.mat != 0 { (i / rows) * col_stride + i % rows } else { i };
        /* the block covers the layout; a dword outside it reads as 0 */
        let bits = block.get(off + dw).map_or(0, |f| f.to_bits());
        put_value(out, i, k.base, bits);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetUniformfv(program: GLuint, location: GLint, params: *mut GLfloat) {
    gl_trace!(glGetUniformfv);
    get_uniform(program, location, Out::Floats(params));
}

#[no_mangle]
pub unsafe extern "C" fn glGetUniformiv(program: GLuint, location: GLint, params: *mut GLint) {
    gl_trace!(glGetUniformiv);
    get_uniform(program, location, Out::Ints(params));
}

#[no_mangle]
pub unsafe extern "C" fn glGetProgramiv(program: GLuint, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetProgramiv);
    const FUNC: &str = "glGetProgramiv";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let (ctx, h) = (c.group as usize, obj_index(program));
    let Some(p) = program_or_error(c, program, FUNC) else {
        return;
    };
    let p = &*p;
    /* a relinked program's own active attributes, read before STATE is taken */
    let rel = relinked_attribs(ctx, program);
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let st = STATE.get();

    let v = match pname {
        GL_LINK_STATUS => (if p.linked { GL_TRUE } else { GL_FALSE }) as GLint,
        GL_DELETE_STATUS => st.delete_pending[ctx][h] as GLint,
        GL_VALIDATE_STATUS => (if st.validated[ctx][h] { GL_TRUE } else { GL_FALSE }) as GLint,
        /* no program binary format exists (GL_NUM_PROGRAM_BINARY_FORMATS 0),
         * so a binary has no length; dEQP's program_binary TCU_CHECKs the
         * query answered */
        GL_PROGRAM_BINARY_LENGTH => 0,
        GL_PROGRAM_BINARY_RETRIEVABLE_HINT => 0,
        GL_ATTACHED_SHADERS => (p.vs != 0) as GLint + (p.fs != 0) as GLint,
        GL_ACTIVE_UNIFORMS => active_total(p, &st.loc[ctx][h], &st.blocks[ctx][h]) as GLint,
        GL_ACTIVE_UNIFORM_BLOCKS => GLint::from(st.blocks[ctx][h].count),
        GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH => {
            let bl = &st.blocks[ctx][h];
            let longest = bl.b[..usize::from(bl.count)]
                .iter()
                .filter_map(|b| prog_block_desc(p, b))
                .map(|(_, d)| d.name.len())
                .max();
            longest.map_or(0, |n| n as GLint + 1)
        }
        GL_ACTIVE_ATTRIBUTES => {
            /* The columns of a matrix attribute are one attribute. */
            if let (true, Some(m)) = (p.linked, rel) {
                m.count as GLint
            } else if p.linked {
                (0..(p.rvs.num_attribs as usize).min(p.rvs.attribs.len())).filter(|&i| attrib_is_whole(p, i)).count()
                    as GLint
            } else {
                0
            }
        }
        GL_INFO_LOG_LENGTH => {
            let n = log_text(&st.log[ctx][h]).len();
            if n != 0 {
                (n + 1) as GLint
            } else {
                0
            }
        }
        /* GL: the length of the LONGEST active name, including its NUL -- not
         * the size of the buffer this implementation keeps names in, which is
         * what both of these used to answer. A program with no active
         * uniforms or attributes (one that has not linked) reports 0. The
         * names are taken through the same path glGetActiveUniform and
         * glGetActiveAttrib hand out, so the answer cannot drift from them. */
        GL_ACTIVE_UNIFORM_MAX_LENGTH => {
            /* the names glGetActiveUniform HANDS OUT, which are not the
             * reflection's: an array's carries "[0]" and a struct array's
             * carries its element index, and both are longer */
            let (tab, bl) = (&st.loc[ctx][h], &st.blocks[ctx][h]);
            let mut buf = [0u8; ACTIVE_NAME_MAX + 64];
            let longest = (0..active_total(p, tab, bl))
                .filter_map(|i| active_nth(p, tab, bl, i))
                .map(|a| active_describe(p, &a, &mut buf).0.len())
                .max();
            longest.map_or(0, |n| n as GLint + 1)
        }
        GL_ACTIVE_ATTRIBUTE_MAX_LENGTH if p.linked && rel.is_some() => {
            let m = rel.unwrap_or(&ShcAttribMap::ZERO);
            let longest = m.b[..m.count as usize].iter().map(|d| util::name_bytes(&d.name).len()).max();
            longest.map_or(0, |n| n as GLint + 1)
        }
        GL_ACTIVE_ATTRIBUTE_MAX_LENGTH => {
            let n = if p.linked { (p.rvs.num_attribs as usize).min(p.rvs.attribs.len()) } else { 0 };
            let longest = (0..n).map(|i| util::name_bytes(&p.rvs.attribs[i].name).len()).max();
            longest.map_or(0, |n| n as GLint + 1)
        }
        GL_TRANSFORM_FEEDBACK_BUFFER_MODE
        | GL_TRANSFORM_FEEDBACK_VARYINGS
        | GL_TRANSFORM_FEEDBACK_VARYING_MAX_LENGTH
            if !crate::REPORT_ES2 =>
        {
            super::tf::program_query(c.group, program, pname).unwrap_or(0)
        }
        _ => {
            c.set_error(GL_INVALID_ENUM, FUNC);
            return;
        }
    };
    // SAFETY: params is null or points at one GLint (GL contract), any alignment.
    util::app_put(params, v);
}

#[no_mangle]
pub unsafe extern "C" fn glGetProgramInfoLog(
    program: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    infoLog: *mut GLchar,
) {
    gl_trace!(glGetProgramInfoLog);
    const FUNC: &str = "glGetProgramInfoLog";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    if bufSize < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    // SAFETY: single-threaded by contract; taken once in this entry point.
    let log = &STATE.get().log[c.group as usize][obj_index(program)];
    // SAFETY: infoLog holds bufSize bytes, length one GLsizei, or they are null.
    copy_out_text(log_text(log), bufSize, length, infoLog);
}

/// `strncmp(field, s, sizeof(field)) == 0` for a fixed-size name field.
fn field_matches(field: &[c_char], s: &[u8]) -> bool {
    let name = util::name_bytes(field);
    if name.len() == field.len() {
        /* no NUL in the field: the comparison stops at its end */
        s.len() >= name.len() && &s[..name.len()] == name
    } else {
        s == name
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetAttribLocation(program: GLuint, name: *const GLchar) -> GLint {
    gl_trace!(glGetAttribLocation);
    const FUNC: &str = "glGetAttribLocation";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return -1;
    };
    let Some(p) = program_or_error(c, program, FUNC) else {
        return -1;
    };
    if !p.linked {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return -1;
    }
    if name.is_null() {
        return -1;
    }
    let p = &c.objs_ref().programs[obj_index(program)];
    // SAFETY: name is a NUL-terminated string (GL contract).
    let name = util::cstr_bytes(name);
    /* a relinked program: its active names at the locations the link
     * decided, whether or not the record kept a row for them */
    if let Some(m) = relinked_attribs(c.group as usize, program) {
        return m.b[..m.count as usize]
            .iter()
            .find(|d| field_matches(&d.name, name))
            .map_or(-1, |d| d.location as GLint);
    }
    for a in &p.rvs.attribs[..(p.rvs.num_attribs as usize).min(p.rvs.attribs.len())] {
        if field_matches(&a.name, name) {
            /* A matrix attribute is several rows; GL says its location is the
             * first of them, and the application binds each column itself. */
            let base = attrib_base_location(p, a) as usize;
            if base < p.attrib_app_index.len() {
                return p.attrib_app_index[base] as GLint;
            }
        }
    }
    /* a relinked record has one row per active location: an aliased name
     * that shares its location's row is where it was bound */
    // SAFETY: single-threaded by contract; a plain read of the flag.
    if unsafe { STATE.get() }.relinked[c.group as usize][obj_index(program)] {
        if let Some(b) = p.attrib_binds[..p.num_attrib_binds as usize].iter().rfind(|b| field_matches(&b.name, name)) {
            return b.index as GLint;
        }
    }
    -1
}

/// Takes effect at the next glLinkProgram (GL). The compiled location cannot
/// move, so the binding is a remap: `draw` sources the VS input from the
/// app's index.
#[no_mangle]
pub unsafe extern "C" fn glBindAttribLocation(program: GLuint, index: GLuint, name: *const GLchar) {
    gl_trace!(glBindAttribLocation);
    const FUNC: &str = "glBindAttribLocation";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    if index >= MAX_VERTEX_ATTRIBS as GLuint || name.is_null() {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    // SAFETY: name is a NUL-terminated string (GL contract).
    let name = util::cstr_bytes(name);
    if name.starts_with(b"gl_") {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }

    let p = &mut c.objs().programs[obj_index(program)];
    let binds = p.num_attrib_binds as usize;
    if let Some(b) = p.attrib_binds[..binds].iter_mut().find(|b| field_matches(&b.name, name)) {
        b.index = index as u8;
        return;
    }
    /* a name beyond what can be kept: GL has no error for it, and none
     * can name an attribute a shader declares (SHC_MAX_DECLARED_ATTRIBS) */
    if binds >= context::MAX_ATTRIB_BINDS {
        return;
    }
    util::set_name(&mut p.attrib_binds[binds].name, name);
    p.attrib_binds[binds].index = index as u8;
    p.num_attrib_binds += 1;
}

#[no_mangle]
pub unsafe extern "C" fn glGetActiveAttrib(
    program: GLuint,
    index: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    size: *mut GLint,
    type_: *mut GLenum,
    name: *mut GLchar,
) {
    gl_trace!(glGetActiveAttrib);
    const FUNC: &str = "glGetActiveAttrib";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let group = c.group as usize;
    let Some(p) = program_or_error(c, program, FUNC) else {
        return;
    };
    let num_attribs = (p.rvs.num_attribs as usize).min(p.rvs.attribs.len());
    if bufSize < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if let (true, Some(m)) = (p.linked, relinked_attribs(group, program)) {
        let Some(d) = m.b[..m.count as usize].get(index as usize) else {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return;
        };
        // SAFETY: as below.
        util::app_put(size, 1);
        util::app_put(type_, d.gl_type as GLenum);
        util::copy_out_str(util::name_bytes(&d.name), bufSize, length, name);
        return;
    }
    /* `index` counts attributes, and a matrix attribute is one of them
     * however many reflection rows its columns take. */
    let Some(row) = (0..num_attribs).filter(|&i| attrib_is_whole(p, i)).nth(index as usize) else {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    };

    let a = &c.objs_ref().programs[obj_index(program)].rvs.attribs[row];

    // SAFETY: size/type point at one value each, name at bufSize bytes, length
    // at one GLsizei, or they are null (GL contract); any alignment.
    util::app_put(size, 1);
    util::app_put(type_, a.gl_type as GLenum);
    util::copy_out_str(util::name_bytes(&a.name), bufSize, length, name);
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteProgram(program: GLuint) {
    gl_trace!(glDeleteProgram);
    const FUNC: &str = "glDeleteProgram";
    if program == 0 {
        return;
    }
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    };
    if program_or_error(c, program, FUNC).is_none() {
        return;
    }
    if c.cur_program == program {
        /* GL ES 3.0 2.12.3: flagged; it goes when it stops being current */
        // SAFETY: single-threaded by contract; taken once in this entry point.
        STATE.get().delete_pending[c.group as usize][obj_index(program)] = true;
        return;
    }
    destroy_program(c, program);
}

#[no_mangle]
pub unsafe extern "C" fn glIsProgram(program: GLuint) -> GLboolean {
    gl_trace!(glIsProgram);
    match context::current().and_then(|c| c.program(program)) {
        Some(_) => GL_TRUE_B,
        None => GL_FALSE_B,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glValidateProgram(program: GLuint) {
    gl_trace!(glValidateProgram);
    const FUNC: &str = "glValidateProgram";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    };
    /* GL_VALIDATE_STATUS is the answer of the command that was run, not a
     * property of the program: GL says it starts GL_FALSE and that a link
     * resets it, so it cannot be read off the link status. What this library
     * checks is the whole of what it can: a program that linked can execute,
     * because every draw state it needs was resolved at link time. */
    let Some(p) = program_or_error(c, program, FUNC) else {
        return;
    };
    let linked = p.linked;
    // SAFETY: single-threaded by contract; taken once in this entry point.
    STATE.get().validated[c.group as usize][obj_index(program)] = linked;
}

#[no_mangle]
pub unsafe extern "C" fn glGetAttachedShaders(
    program: GLuint,
    maxCount: GLsizei,
    count: *mut GLsizei,
    shaders: *mut GLuint,
) {
    gl_trace!(glGetAttachedShaders);
    const FUNC: &str = "glGetAttachedShaders";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    };
    let Some(p) = program_or_error(c, program, FUNC) else {
        return;
    };
    let attached = [p.vs, p.fs];
    if maxCount < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let mut n: GLsizei = 0;
    for h in attached {
        if h != 0 && n < maxCount && !shaders.is_null() {
            // SAFETY: shaders holds maxCount handles (GL contract), n < maxCount; any alignment.
            util::app_set(shaders, n as usize, h);
            n += 1;
        } else if h != 0 && shaders.is_null() {
            n += 1;
        }
    }
    // SAFETY: count is null or points at one GLsizei (GL contract), any alignment.
    util::app_put(count, if !shaders.is_null() { n } else { n.min(maxCount) });
}

/// Unit tests: clear the per-program location tables and info logs
/// (process-global; testutil::TestGl calls this when it is created and dropped).
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised; every field is valid when all-zero.
    unsafe { core::ptr::write_bytes(STATE.as_ptr(), 0, 1) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::shader::{glCompileShader, glCreateShader, glShaderSource};
    use crate::testutil::{MemEvent, TestGl};
    use core::ffi::c_void;
    use std::boxed::Box;
    use std::ffi::CString;
    use std::string::String;
    use std::vec::Vec;

    /// ES 3.0 uniform blocks end to end on the in-process compiler: the
    /// program's blocks and their members as the queries report them, the
    /// members hidden from glGetUniformLocation, and a draw filling each
    /// stage's block from the range bound to the block's binding point -- a
    /// row-major matrix transposed, an array element at its std140 stride.
    ///
    /// Revert check: drop the `fill_uniform_blocks` call in `draw` and the
    /// recorded blocks hold zeros.
    #[cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn uniform_blocks_are_queried_bound_and_filled_at_draw() {
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = TestGl::new(16, 16);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let vs = CString::new(
            "#version 300 es\nlayout(std140) uniform Xf { mat4 mvp; } xf;\nin vec4 a_pos;\n\
             void main() { gl_Position = xf.mvp * a_pos; }\n",
        )
        .unwrap();
        let fs = CString::new(
            "#version 300 es\nprecision highp float;\n\
             layout(std140) uniform Colours { vec4 tint; layout(row_major) mat2 rm; float f[2]; bool on; } col;\n\
             layout(location = 0) out vec4 o;\n\
             void main() { o = col.on ? col.tint + vec4(col.rm[0], col.f[1], 0.0) : vec4(0.0); }\n",
        )
        .unwrap();
        let shader = |ty: GLenum, text: &CString| {
            // SAFETY: plain GL calls on a handle this test owns.
            unsafe {
                let s = glCreateShader(ty);
                let p: [*const core::ffi::c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                crate::gl::shader::glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "{:?} did not compile", text);
                s
            }
        };
        let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs));
        let iv = |p: GLuint, pname: GLenum| {
            let mut x: GLint = -9;
            // SAFETY: one GLint.
            unsafe { glGetProgramiv(p, pname, &mut x) };
            x
        };
        // SAFETY: plain GL calls on handles this test owns; every out pointer
        // has room for what is asked.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            assert_eq!(iv(p, GL_LINK_STATUS), 1);
            assert_eq!(iv(p, GL_ACTIVE_UNIFORM_BLOCKS), 2);
            assert_eq!(iv(p, GL_ACTIVE_UNIFORM_BLOCK_MAX_NAME_LENGTH), 8);
            assert_eq!(glGetUniformBlockIndex(p, c"Xf".as_ptr()), 0);
            assert_eq!(glGetUniformBlockIndex(p, c"Colours".as_ptr()), 1);
            assert_eq!(glGetUniformBlockIndex(p, c"col".as_ptr()), GL_INVALID_INDEX, "the instance names no block");
            let biv = |k: GLuint, pname: GLenum| {
                let mut x: GLint = -9;
                glGetActiveUniformBlockiv(p, k, pname, &mut x);
                x
            };
            assert_eq!(biv(0, GL_UNIFORM_BLOCK_DATA_SIZE), 64);
            assert_eq!(biv(1, GL_UNIFORM_BLOCK_DATA_SIZE), 96);
            assert_eq!(
                (
                    biv(0, GL_UNIFORM_BLOCK_REFERENCED_BY_VERTEX_SHADER),
                    biv(0, GL_UNIFORM_BLOCK_REFERENCED_BY_FRAGMENT_SHADER)
                ),
                (1, 0)
            );
            assert_eq!(biv(1, GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS), 4);
            let mut name = [0 as core::ffi::c_char; 16];
            let mut len: GLsizei = 0;
            glGetActiveUniformBlockName(p, 1, 16, &mut len, name.as_mut_ptr());
            assert_eq!(len, 7);
            assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 5);
            let names = [c"Colours.f".as_ptr(), c"Xf.mvp".as_ptr(), c"Colours.f[0]".as_ptr(), c"col.tint".as_ptr()];
            let mut idx = [0u32; 4];
            glGetUniformIndices(p, 4, names.as_ptr(), idx.as_mut_ptr());
            assert_eq!(idx, [3, 0, 3, GL_INVALID_INDEX]);
            let members = [1u32, 2, 3, 4];
            let uiv = |pname: GLenum| {
                let mut out = [-9 as GLint; 4];
                glGetActiveUniformsiv(p, 4, members.as_ptr(), pname, out.as_mut_ptr());
                out
            };
            assert_eq!(uiv(GL_UNIFORM_OFFSET), [0, 16, 48, 80]);
            assert_eq!(uiv(GL_UNIFORM_ARRAY_STRIDE), [0, 0, 16, 0]);
            assert_eq!(uiv(GL_UNIFORM_MATRIX_STRIDE), [0, 16, 0, 0]);
            assert_eq!(uiv(GL_UNIFORM_IS_ROW_MAJOR), [0, 1, 0, 0]);
            assert_eq!(uiv(GL_UNIFORM_BLOCK_INDEX), [1; 4]);
            assert_eq!(uiv(GL_UNIFORM_SIZE), [1, 1, 2, 1]);
            assert_eq!(glGetUniformLocation(p, c"col.tint".as_ptr()), -1, "a block member has no location");
            assert_eq!(glGetUniformLocation(p, c"Colours.tint".as_ptr()), -1);
            assert_eq!(gl.error(), GL_NO_ERROR);

            /* Colours at binding 3 through a range; Xf at the default 0. */
            glUniformBlockBinding(p, 1, 3);
            assert_eq!(biv(1, GL_UNIFORM_BLOCK_BINDING), 3);
            let mut colours = [0.0f32; 24 + 4];
            colours[4..8].copy_from_slice(&[1.0, 2.0, 3.0, 4.0]); /* tint at byte 16 of the range */
            colours[8..10].copy_from_slice(&[5.0, 6.0]); /* rm row 0 */
            colours[12..14].copy_from_slice(&[7.0, 8.0]); /* rm row 1 */
            colours[16] = 9.0; /* f[0] */
            colours[20] = 10.0; /* f[1] */
            colours[24] = f32::from_bits(7); /* on: any non-zero word */
            let mut bufs = [0u32; 2];
            crate::gl::buffer::glGenBuffers(2, bufs.as_mut_ptr());
            crate::gl::buffer::glBindBuffer(GL_UNIFORM_BUFFER, bufs[0]);
            crate::gl::buffer::glBufferData(GL_UNIFORM_BUFFER, 112, colours.as_ptr() as *const c_void, GL_STATIC_DRAW);
            crate::gl::es3::glBindBufferRange(GL_UNIFORM_BUFFER, 3, bufs[0], 16, 96);
            let mvp: [f32; 16] = core::array::from_fn(|k| k as f32 + 100.0);
            crate::gl::buffer::glBindBuffer(GL_UNIFORM_BUFFER, bufs[1]);
            crate::gl::buffer::glBufferData(GL_UNIFORM_BUFFER, 64, mvp.as_ptr() as *const c_void, GL_STATIC_DRAW);
            crate::gl::es3::glBindBufferBase(GL_UNIFORM_BUFFER, 0, bufs[1]);
            let mut bound: GLint = 0;
            crate::gl::state::glGetIntegeri_v(GL_UNIFORM_BUFFER_BINDING, 3, &mut bound);
            assert_eq!(bound as u32, bufs[0]);
            let mut start: GLint64 = 0;
            crate::gl::state::glGetInteger64i_v(GL_UNIFORM_BUFFER_START, 3, &mut start);
            assert_eq!(start, 16);
            assert_eq!(gl.error(), GL_NO_ERROR);

            glUseProgram(p);
            static POS: [f32; 12] = [0.0; 12];
            crate::gl::vertex::glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            crate::gl::vertex::glEnableVertexAttribArray(0);
            crate::gl::draw::glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let d = gl.draws().pop().expect("a draw");
            let prog = &gl.ctx().objs_ref().programs[obj_index(p)];
            let row = |sh: &crate::shaderc::ShcShader, n: &str| {
                let u = sh.uniform_table().iter().find(|u| util::name_bytes(&u.name) == n.as_bytes()).expect("row");
                usize::from(u.dword_off)
            };
            let fsb = |at: usize, n: usize| {
                d.fs_uniforms[at..at + n].iter().map(|b| f32::from_bits(*b)).collect::<std::vec::Vec<_>>()
            };
            assert_eq!(fsb(row(&prog.rfs, "col.tint"), 4), [1.0, 2.0, 3.0, 4.0]);
            /* row-major rows (5, 6), (7, 8): columns (5, 7) and (6, 8) */
            assert_eq!(fsb(row(&prog.rfs, "col.rm"), 4), [5.0, 7.0, 6.0, 8.0]);
            assert_eq!(fsb(row(&prog.rfs, "col.f"), 2), [9.0, 10.0]);
            assert_eq!(d.fs_uniforms[row(&prog.rfs, "col.on")], 1, "a bool is 0 / 1 in the block");
            let at = row(&prog.rvs, "xf.mvp");
            let vsb: std::vec::Vec<f32> = d.vs_uniforms[at..at + 16].iter().map(|b| f32::from_bits(*b)).collect();
            assert_eq!(vsb, mvp);
        }
    }

    /// An array of one element is still an array: glGetActiveUniform names
    /// `float f[1]` `f[0]` (size 1), as it names `f[4]` `f[0]` (size 4), and
    /// a struct member's `m[1]` is `s.m[0]`; a lone value keeps its name.
    /// `array_len` is 1 for both `float f[1]` and `float g`, so the online
    /// record carries which rows are declared arrays (`ShcBlocks::arrays`).
    /// dEQP-GLES2 uniform_api.random 2 8 12 13 18 31 38 43 46 47 90 99 failed
    /// "invalid non-built-in uniform name" on the Pi 4 and the i.MX8MP
    /// (host: dEQP's own generator and getActiveUniforms check over all 100
    /// seeds, 27 seeds carrying the shape before, 0 after).
    ///
    /// Revert check: append "[0]" by `size > 1` alone in
    /// `active_uniform_name` and the names below lose it.
    #[cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
    #[test]
    fn an_array_of_one_element_is_named_as_an_array() {
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = TestGl::new(16, 16);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let vs = CString::new("attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n").unwrap();
        let fs = CString::new(
            "precision mediump float;\n\
             struct S { vec2 m[1]; float k; };\n\
             uniform float f[1];\n\
             uniform float g;\n\
             uniform vec4 h[3];\n\
             uniform S s;\n\
             uniform sampler2D t[1];\n\
             void main() { gl_FragColor = vec4(f[0] + g + h[2].x + s.m[0].y + s.k) + texture2D(t[0], vec2(0.5)); }\n",
        )
        .unwrap();
        let shader = |ty: GLenum, text: &CString| {
            // SAFETY: plain GL calls on a handle this test owns.
            unsafe {
                let s = glCreateShader(ty);
                let p: [*const core::ffi::c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                crate::gl::shader::glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "{:?} did not compile", text);
                s
            }
        };
        let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs));
        // SAFETY: plain GL calls on handles this test owns; every out pointer
        // has room for what is asked.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut n: GLint = -9;
            glGetProgramiv(p, GL_LINK_STATUS, &mut n);
            assert_eq!(n, 1);
            glGetProgramiv(p, GL_ACTIVE_UNIFORMS, &mut n);
            let mut got: Vec<(String, GLint)> = Vec::new();
            for i in 0..n as u32 {
                let mut buf = [0 as core::ffi::c_char; 32];
                let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
                glGetActiveUniform(p, i, 32, &mut len, &mut size, &mut ty, buf.as_mut_ptr());
                let name = core::ffi::CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned();
                assert_eq!(name.len() as GLsizei, len, "{name}");
                got.push((name, size));
            }
            got.sort();
            let want = [("f[0]", 1), ("g", 1), ("h[0]", 3), ("s.k", 1), ("s.m[0]", 1), ("t[0]", 1)];
            let want: Vec<(String, GLint)> = want.iter().map(|&(n, s)| (String::from(n), s)).collect();
            assert_eq!(got, want);
            /* a count past the end of an array of one: the element is set
             * and the rest ignored, not an error (is_array) */
            let loc = glGetUniformLocation(p, c"f".as_ptr());
            assert!(loc >= 0);
            glUseProgram(p);
            let two = [1.5f32, 2.5];
            glUniform1fv(loc, 2, two.as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            let mut back = 0.0f32;
            glGetUniformfv(p, loc, &mut back);
            assert_eq!(back, 1.5);
            /* and a lone value still refuses one */
            glUniform1fv(glGetUniformLocation(p, c"g".as_ptr()), 2, two.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    /// An array of blocks (ES 3.0 2.12.6): one block per element, `B[k]`,
    /// whose members are the same active uniforms, `B.m`, in the first
    /// element's block; every element lists them; each element is still
    /// filled from its own binding point. dEQP-GLES3 ubo's reference layout
    /// is exactly this, and 717 of its cases failed "Invalid layout" while
    /// the members were `B[k].m` (host run, 2026-09-24).
    ///
    /// Revert check: name the members from `gl_block` again in
    /// `reflect::block_table`, or drop the `first_instance` filter in
    /// `block_members`, and the counts and names below fail.
    #[cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn an_array_of_blocks_shares_its_members() {
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = TestGl::new(16, 16);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let vs = CString::new("#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n").unwrap();
        let fs = CString::new(
            "#version 300 es\nprecision highp float;\n\
             uniform Solo { vec4 s; };\n\
             layout(std140) uniform Pal { vec4 c; float w; float one[1]; } pal[3];\n\
             layout(location = 0) out vec4 o;\n\
             void main() { o = s + pal[0].c + pal[1].c * pal[2].w; }\n",
        )
        .unwrap();
        let shader = |ty: GLenum, text: &CString| {
            // SAFETY: plain GL calls on a handle this test owns.
            unsafe {
                let s = glCreateShader(ty);
                let p: [*const core::ffi::c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                crate::gl::shader::glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "{:?} did not compile", text);
                s
            }
        };
        let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs));
        // SAFETY: plain GL calls on handles this test owns; every out pointer
        // has room for what is asked.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let iv = |pname: GLenum| {
                let mut x: GLint = -9;
                glGetProgramiv(p, pname, &mut x);
                x
            };
            assert_eq!(iv(GL_LINK_STATUS), 1);
            assert_eq!(iv(GL_ACTIVE_UNIFORM_BLOCKS), 4, "Solo, Pal[0], Pal[1], Pal[2]");
            assert_eq!(iv(GL_ACTIVE_UNIFORMS), 4, "s, Pal.c, Pal.w, Pal.one[0]: once, not once per element");
            assert_eq!(glGetUniformBlockIndex(p, c"Pal[1]".as_ptr()), 2);
            assert_eq!(glGetUniformBlockIndex(p, c"Pal".as_ptr()), 1, "the name alone is element 0");
            let mut idx = [0u32; 3];
            let names = [c"Pal.c".as_ptr(), c"Pal.w".as_ptr(), c"Pal[1].c".as_ptr()];
            glGetUniformIndices(p, 3, names.as_ptr(), idx.as_mut_ptr());
            assert_eq!(idx, [1, 2, GL_INVALID_INDEX]);
            let mut buf = [0 as core::ffi::c_char; 16];
            let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
            glGetActiveUniform(p, 2, 16, &mut len, &mut size, &mut ty, buf.as_mut_ptr());
            assert_eq!(core::ffi::CStr::from_ptr(buf.as_ptr()).to_bytes(), b"Pal.w");
            /* An array of one element is still an array: `Pal.one[0]`, size
             * 1 (dEQP-GLES3 ubo.random). Revert check: name it by
             * `array_size > 1` alone in `block_member_name`. */
            glGetActiveUniform(p, 3, 16, &mut len, &mut size, &mut ty, buf.as_mut_ptr());
            assert_eq!((core::ffi::CStr::from_ptr(buf.as_ptr()).to_bytes(), size), (&b"Pal.one[0]"[..], 1));
            let members = [1u32, 2];
            let mut blk = [-9 as GLint; 2];
            glGetActiveUniformsiv(p, 2, members.as_ptr(), GL_UNIFORM_BLOCK_INDEX, blk.as_mut_ptr());
            assert_eq!(blk, [1, 1], "the first element's block");
            let mut off = [-9 as GLint; 2];
            glGetActiveUniformsiv(p, 2, members.as_ptr(), GL_UNIFORM_OFFSET, off.as_mut_ptr());
            assert_eq!(off, [0, 16]);
            for k in 1..4u32 {
                let mut n: GLint = -9;
                glGetActiveUniformBlockiv(p, k, GL_UNIFORM_BLOCK_ACTIVE_UNIFORMS, &mut n);
                assert_eq!(n, 3);
                let mut ix = [-9 as GLint; 3];
                glGetActiveUniformBlockiv(p, k, GL_UNIFORM_BLOCK_ACTIVE_UNIFORM_INDICES, ix.as_mut_ptr());
                assert_eq!(ix, [1, 2, 3], "Pal[{}] lists the shared members", k - 1);
            }
            assert_eq!(gl.error(), GL_NO_ERROR);

            /* each element from its own binding point */
            let mut bufs = [0u32; 3];
            crate::gl::buffer::glGenBuffers(3, bufs.as_mut_ptr());
            for k in 0..3u32 {
                let data: [f32; 8] = core::array::from_fn(|i| (10 * k + i as u32) as f32);
                crate::gl::buffer::glBindBuffer(GL_UNIFORM_BUFFER, bufs[k as usize]);
                crate::gl::buffer::glBufferData(GL_UNIFORM_BUFFER, 32, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
                crate::gl::es3::glBindBufferBase(GL_UNIFORM_BUFFER, 5 + k, bufs[k as usize]);
                glUniformBlockBinding(p, 1 + k, 5 + k);
            }
            glUseProgram(p);
            static POS: [f32; 12] = [0.0; 12];
            crate::gl::vertex::glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            crate::gl::vertex::glEnableVertexAttribArray(0);
            crate::gl::draw::glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let d = gl.draws().pop().expect("a draw");
            let prog = &gl.ctx().objs_ref().programs[obj_index(p)];
            let row = |n: &str| {
                let u = prog.rfs.uniform_table().iter().find(|u| util::name_bytes(&u.name) == n.as_bytes());
                usize::from(u.expect("row").dword_off)
            };
            let fsb =
                |at: usize, n: usize| d.fs_uniforms[at..at + n].iter().map(|b| f32::from_bits(*b)).collect::<Vec<_>>();
            assert_eq!(fsb(row("pal[1].c"), 4), [10.0, 11.0, 12.0, 13.0]);
            assert_eq!(fsb(row("pal[2].w"), 1), [24.0]);
        }
    }

    /// dEQP-GLES2 shaders.struct.uniform.nested_struct_array (and the loop
    /// variant) through the GL core, as dEQP drives it: every leaf set by
    /// glGetUniformLocation + glUniform*, `s[k].b[j].b` as ONE location with
    /// count 2, then a draw. The recorded fragment block is fed to the front
    /// end's own IR (the evaluator), whose colour must be the case's. Failed
    /// on the AM62P Zephyr run (2026-09-24); this pins everything above the
    /// PowerVR back end. HOST-ONLY.
    #[cfg(feature = "shaderc-online")]
    #[test]
    fn nested_struct_array_uniforms_reach_the_shader_as_deqp_sets_them() {
        use crate::shaderc::online::{arena::Arena, eval, lower};
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = TestGl::new(16, 16);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let decl = "precision mediump float;\nuniform int ui_zero; uniform int ui_one; uniform int ui_two;\n\
                    struct T { mediump float a; mediump vec2 b[2]; };\n\
                    struct S { mediump float a; T b[3]; int c; };\nuniform S s[2];\nuniform mediump float uf_sixth;\n";
        let bodies = [
            /* nested_struct_array */
            "mediump float r = (s[0].b[1].b[0].x + s[1].b[2].b[1].y) * s[0].b[0].a;\n\
             mediump float g = s[1].b[0].b[0].y * s[0].b[2].a * s[1].b[2].a;\n\
             mediump float b = (s[0].b[2].b[1].y + s[0].b[1].b[0].y + s[1].a) * s[0].b[1].a;\n\
             mediump float a = float(s[0].c) + s[1].b[2].a - s[1].b[1].a;\n\
             gl_FragColor = vec4(r, g, b, a);",
            /* nested_struct_array_dynamic_index */
            "mediump float r = (s[0].b[ui_one].b[ui_one-1].x + s[ui_one].b[ui_two].b[ui_zero+1].y) * s[0].b[0].a;\n\
             mediump float g = s[ui_two-1].b[ui_two-2].b[ui_zero].y * s[0].b[ui_two].a * s[ui_one].b[2].a;\n\
             mediump float b = (s[0].b[ui_zero+2].b[1].y + s[0].b[ui_one].b[ui_zero].y + s[ui_one].a) * s[0].b[ui_two-ui_one].a;\n\
             mediump float a = float(s[0].c) + s[ui_one].b[2].a - s[ui_one].b[1].a;\n\
             gl_FragColor = vec4(r, g, b, a);",
            /* loop_nested_struct_array */
            "mediump float r = 0.0; mediump float g = 0.0; mediump float b = 0.0; mediump float a = 1.0;\n\
             for (int i = 0; i < 2; i++) { for (int j = 0; j < 3; j++) {\n\
             r += s[0].b[j].b[i].y; g += s[i].b[j].b[0].x; b += s[i].b[j].b[1].x; a *= s[i].b[j].a; } }\n\
             gl_FragColor = vec4(r*uf_sixth, g*uf_sixth, b*uf_sixth, a);",
        ];
        let (x, y, z, w) = (0.1f32, 0.3f32, 0.6f32, 0.9f32);
        /* the values, as dEQP's set-up body gives them: s[k].b[j].b as pairs */
        let pairs: [[[f32; 4]; 3]; 2] =
            [[[x, y, z, w], [z, w, x, y], [x, z, y, w]], [[x, x, y, y], [z, z, w, w], [y, x, w, z]]];
        let ta = [[0.5f32, 1.0 / 3.0, 0.25], [2.0, 3.0, 4.0]];
        let expect = [
            [(z + z) * 0.5, x * 0.25 * 4.0, (w + w + w) * (1.0 / 3.0), 0.0 + 4.0 - 3.0],
            [(z + z) * 0.5, x * 0.25 * 4.0, (w + w + w) * (1.0 / 3.0), 0.0 + 4.0 - 3.0],
            {
                let (mut r, mut g, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 1.0f32);
                for i in 0..2 {
                    for j in 0..3 {
                        r += pairs[0][j][i * 2 + 1];
                        g += pairs[i][j][0];
                        b += pairs[i][j][2];
                        a *= ta[i][j];
                    }
                }
                [r / 6.0, g / 6.0, b / 6.0, a]
            },
        ];
        let vs = CString::new("attribute vec4 p;\nvoid main() { gl_Position = p; }\n").unwrap();
        for (body, want) in bodies.iter().zip(expect) {
            let fs_src = std::format!("{decl}void main (void) {{\n{body}\n}}\n");
            let fs = CString::new(fs_src.clone()).unwrap();
            // SAFETY: plain GL calls on handles this test owns; every pointer
            // is valid for what is read or written.
            unsafe {
                let sh = |ty: GLenum, t: &CString| {
                    let s = glCreateShader(ty);
                    let p: [*const core::ffi::c_char; 1] = [t.as_ptr()];
                    glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                    glCompileShader(s);
                    s
                };
                let p = glCreateProgram();
                glAttachShader(p, sh(GL_VERTEX_SHADER, &vs));
                glAttachShader(p, sh(GL_FRAGMENT_SHADER, &fs));
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                assert_eq!(ok, 1, "{fs_src}");
                glUseProgram(p);
                /* a name the program does not use is location -1, which
                 * glUniform* ignores, as dEQP's set-up relies on */
                let loc = |n: &str| {
                    let c = CString::new(n).unwrap();
                    glGetUniformLocation(p, c.as_ptr())
                };
                let check = |what: &str| assert_eq!(gl.error(), GL_NO_ERROR, "{what}\n{fs_src}");
                glUniform1i(loc("ui_zero"), 0);
                glUniform1i(loc("ui_one"), 1);
                glUniform1i(loc("ui_two"), 2);
                glUniform1f(loc("uf_sixth"), 1.0 / 6.0);
                check("the scalars");
                for k in 0..2 {
                    glUniform1f(loc(&std::format!("s[{k}].a")), if k == 0 { x } else { w });
                    check(&std::format!("s[{k}].a"));
                    glUniform1i(loc(&std::format!("s[{k}].c")), k as GLint);
                    check(&std::format!("s[{k}].c"));
                    for j in 0..3 {
                        glUniform1f(loc(&std::format!("s[{k}].b[{j}].a")), ta[k][j]);
                        check(&std::format!("s[{k}].b[{j}].a"));
                        glUniform2fv(loc(&std::format!("s[{k}].b[{j}].b")), 2, pairs[k][j].as_ptr());
                        check(&std::format!("s[{k}].b[{j}].b, count 2"));
                    }
                }
                static POS: [f32; 12] = [0.0; 12];
                crate::gl::vertex::glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
                crate::gl::vertex::glEnableVertexAttribArray(0);
                crate::gl::draw::glDrawArrays(GL_TRIANGLES, 0, 3);
                assert_eq!(gl.error(), GL_NO_ERROR);
                let d = gl.draws().pop().expect("a draw");
                let mut buf = std::vec![0u8; 4 << 20];
                let a = Arena::new(&mut buf);
                let l = lower::compile_to_ir(&a, crate::shaderc::online::sema::Stage::Fragment, &fs_src)
                    .unwrap_or_else(|e| panic!("refused: {}", e.msg));
                let out = eval::run(&l.module, &eval::Inputs { io: &[], uniforms: &d.fs_uniforms }).expect("runs");
                let got = out.output_f32(crate::shaderc::online::ir::IoSlot::FragColor(0)).expect("colour");
                for c in 0..4 {
                    assert!((got[c] - want[c]).abs() < 1e-5, "{body}\n got {got:?}\nwant {want:?}");
                }
                glDeleteProgram(p);
            }
        }
    }

    /// dEQP-GLES2 uniform_api.value.assigned.by_pointer.render.
    /// {struct_in_array,array_in_struct}.sampler2D_samplerCube_both through
    /// the GL core, as dEQP drives it: every sampler leaf of a struct array
    /// (and of arrays inside a struct) set by glGetUniformLocation +
    /// glUniform1iv to its own unit, then a draw. Both failed on the i.MX8MP
    /// Zephyr run (2026-09-24). Here: every leaf has a location, the setter
    /// takes it, and the draw binds one texture per leaf on the unit it was
    /// given -- a 2D texture for a sampler2D, a cube map for a samplerCube.
    /// HOST-ONLY.
    #[cfg(feature = "shaderc-online")]
    #[test]
    fn sampler_leaves_of_struct_arrays_reach_the_draw_on_their_units() {
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = TestGl::new(16, 16);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let vs = CString::new("attribute vec4 p;\nvoid main() { gl_Position = p; }\n").unwrap();
        /* (declaration, the leaves: name, is a cube map) */
        let cases: [(&str, std::vec::Vec<(std::string::String, bool)>); 2] = [
            (
                "struct S { mediump sampler2D m0; mediump samplerCube m1; };\nuniform S u_var[3];\n",
                (0..3).flat_map(|k| [(std::format!("u_var[{k}].m0"), false), (std::format!("u_var[{k}].m1"), true)]).collect(),
            ),
            (
                "struct S { mediump sampler2D m0; mediump samplerCube m1; mediump sampler2D m2[3]; mediump samplerCube m3[3]; };\n\
                 uniform S u_var;\n",
                [("u_var.m0".to_string(), false), ("u_var.m1".to_string(), true)]
                    .into_iter()
                    .chain((0..3).map(|j| (std::format!("u_var.m2[{j}]"), false)))
                    .chain((0..3).map(|j| (std::format!("u_var.m3[{j}]"), true)))
                    .collect(),
            ),
        ];
        for (decl, leaves) in &cases {
            let mut body = std::string::String::from("    mediump vec4 c = vec4(0.0);\n");
            for (name, cube) in leaves {
                if *cube {
                    body += &std::format!("    c += textureCube({name}, vec3(0.5, 0.5, 0.5));\n");
                } else {
                    body += &std::format!("    c += texture2D({name}, vec2(0.5, 0.5));\n");
                }
            }
            let fs_src =
                std::format!("precision mediump float;\n{decl}void main (void) {{\n{body}    gl_FragColor = c;\n}}\n");
            let fs = CString::new(fs_src.clone()).unwrap();
            let mut names: std::vec::Vec<GLuint> = std::vec::Vec::new();
            // SAFETY: plain GL calls on handles this test owns; every pointer
            // is valid for what is read or written.
            unsafe {
                let sh = |ty: GLenum, t: &CString| {
                    let s = glCreateShader(ty);
                    let p: [*const core::ffi::c_char; 1] = [t.as_ptr()];
                    glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                    glCompileShader(s);
                    s
                };
                let p = glCreateProgram();
                glAttachShader(p, sh(GL_VERTEX_SHADER, &vs));
                glAttachShader(p, sh(GL_FRAGMENT_SHADER, &fs));
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                assert_eq!(ok, 1, "{fs_src}");
                glUseProgram(p);
                /* leaf i: unit 2 + i, with a texture of its kind bound there */
                for (i, (name, cube)) in leaves.iter().enumerate() {
                    let unit = 2 + i as GLint;
                    let c = CString::new(name.as_str()).unwrap();
                    let loc = glGetUniformLocation(p, c.as_ptr());
                    assert!(loc >= 0, "{name} has no location\n{fs_src}");
                    glUniform1iv(loc, 1, &unit);
                    assert_eq!(gl.error(), GL_NO_ERROR, "{name}");
                    crate::gl::texture::glActiveTexture(GL_TEXTURE0 + unit as GLenum);
                    let mut t: GLuint = 0;
                    crate::gl::texture::glGenTextures(1, &mut t);
                    let texels = [0u8; 4 * 4 * 4];
                    if *cube {
                        crate::gl::texture::glBindTexture(GL_TEXTURE_CUBE_MAP, t);
                        for f in 0..6 {
                            crate::gl::texture::glTexImage2D(
                                GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                                0,
                                GL_RGBA as GLint,
                                4,
                                4,
                                0,
                                GL_RGBA,
                                GL_UNSIGNED_BYTE,
                                texels.as_ptr() as *const c_void,
                            );
                        }
                    } else {
                        crate::gl::texture::glBindTexture(GL_TEXTURE_2D, t);
                        crate::gl::texture::glTexImage2D(
                            GL_TEXTURE_2D,
                            0,
                            GL_RGBA as GLint,
                            4,
                            4,
                            0,
                            GL_RGBA,
                            GL_UNSIGNED_BYTE,
                            texels.as_ptr() as *const c_void,
                        );
                    }
                    names.push(t);
                }
                assert_eq!(gl.error(), GL_NO_ERROR);
                static POS: [f32; 12] = [0.0; 12];
                crate::gl::vertex::glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
                crate::gl::vertex::glEnableVertexAttribArray(0);
                crate::gl::draw::glDrawArrays(GL_TRIANGLES, 0, 3);
                assert_eq!(gl.error(), GL_NO_ERROR);
                let d = gl.draws().pop().expect("a draw").draw;
                assert_eq!(usize::from(d.num_textures), leaves.len(), "one binding per leaf\n{fs_src}");
                /* `GlesTextureBind::unit` is the shader's sampler BINDING (the
                 * descriptor the code reads), not the GL unit: leaf `name` is
                 * the row of that name, or element j of the row `name[j]`
                 * belongs to, at the row's binding + j. */
                let pr = &gl.ctx().objs().programs[obj_index(p)];
                let rows: std::vec::Vec<(std::string::String, u8, u8)> = pr
                    .rfs
                    .uniform_table()
                    .iter()
                    .filter(|u| u.dword_off == SHC_UNIFORM_NO_BLOCK)
                    .map(|u| {
                        (core::str::from_utf8(util::name_bytes(&u.name)).unwrap().to_string(), u.binding, u.array_len)
                    })
                    .collect();
                let mut copies: std::vec::Vec<u64> = std::vec::Vec::new();
                for (i, (name, cube)) in leaves.iter().enumerate() {
                    let binding = rows.iter().find(|r| r.0 == *name).map(|r| r.1).or_else(|| {
                        let (base, j) = name.rsplit_once('[')?;
                        let j: u8 = j.trim_end_matches(']').parse().ok()?;
                        rows.iter().find(|r| r.0 == base && j < r.2).map(|r| r.1 + j)
                    });
                    let binding = binding.unwrap_or_else(|| panic!("{name}: no sampler row among {rows:?}"));
                    let want = gl.ctx().objs().textures[names[i] as usize].mem.gpu;
                    let bound: std::vec::Vec<_> =
                        d.textures[..usize::from(d.num_textures)].iter().filter(|t| t.unit == binding).collect();
                    assert_eq!(bound.len(), 1, "{name}: bound once on binding {binding}");
                    /* a cube leaf of a native-cube program (the PowerVR's
                     * default since step C) is bound as the TPU's twiddled
                     * copy of that texture (gl::native_cube), whose address
                     * is the copy's; the copies of different leaves differ */
                    if *cube && bound[0].type_ == crate::backend::GLES_TEXTYPE_CUBE_TWIDDLED_PROBE {
                        copies.push(bound[0].base_gpu);
                    } else {
                        assert_eq!(bound[0].base_gpu, want, "{name}: the texture on binding {binding}");
                    }
                    assert_eq!(bound[0].layers, if *cube { 6 } else { 1 }, "{name}: its kind");
                }
                let distinct: std::collections::BTreeSet<u64> = copies.iter().copied().collect();
                assert_eq!(distinct.len(), copies.len(), "one copy per cube leaf: {copies:x?}");
                glDeleteProgram(p);
                crate::gl::texture::glDeleteTextures(names.len() as GLsizei, names.as_ptr());
            }
        }
    }

    /// The signed integer setter does not load a uint uniform (ES 3.0
    /// 2.12.6; dEQP-GLES3 negative_api.shader.uniformi[v]_incompatible_type),
    /// while the unsigned one still loads an int uniform: the deliberate
    /// deviation Qt's RHI relies on (`setter_accepts`).
    #[cfg(feature = "shaderc-online")]
    #[cfg(not(feature = "report-es2"))] // compiles GLSL ES 3.00, which a report-es2 build refuses (pp.rs out_of_scope)
    #[test]
    fn an_int_setter_does_not_load_a_uint_uniform() {
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = TestGl::new(16, 16);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let vs = CString::new("#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n").unwrap();
        let fs = CString::new(
            "#version 300 es\nprecision highp float;\nuniform uvec4 u;\nuniform ivec4 i;\n\
             layout(location = 0) out vec4 o;\nvoid main() { o = vec4(u) + vec4(i); }\n",
        )
        .unwrap();
        // SAFETY: plain GL calls on handles this test owns.
        unsafe {
            let sh = |t: GLenum, s: &CString| {
                let h = glCreateShader(t);
                let ptrs: [*const core::ffi::c_char; 1] = [s.as_ptr()];
                glShaderSource(h, 1, ptrs.as_ptr(), core::ptr::null());
                glCompileShader(h);
                h
            };
            let p = glCreateProgram();
            glAttachShader(p, sh(GL_VERTEX_SHADER, &vs));
            glAttachShader(p, sh(GL_FRAGMENT_SHADER, &fs));
            glLinkProgram(p);
            glUseProgram(p);
            let (lu, li) = (glGetUniformLocation(p, c"u".as_ptr()), glGetUniformLocation(p, c"i".as_ptr()));
            assert!(lu >= 0 && li >= 0);
            let v = [1i32; 4];
            glUniform4iv(lu, 1, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "int setter on a uint uniform");
            glUniform4uiv(lu, 1, v.as_ptr() as *const GLuint);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glUniform4uiv(li, 1, v.as_ptr() as *const GLuint);
            assert_eq!(gl.error(), GL_NO_ERROR, "uint setter on an int uniform: Qt's deviation, kept");
            glUniform4iv(li, 1, v.as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    /// dEQP-GLES2 shaders.indexing.uniform_array, every type and read access,
    /// through the GL core as dEQP drives it: `u_arr` set by ONE
    /// glUniform*fv of 4 elements at the location of "u_arr", the ints by
    /// glUniform1i, a draw, and the front end's IR evaluated on the recorded
    /// block. The i.MX8MP Zephyr run (99fc5c9d) fails the dynamic, dynamic
    /// loop and static loop reads (24) and passes the static one; this pins
    /// that everything above the back end is right for all four, so those
    /// failures are below it. HOST-ONLY.
    #[cfg(feature = "shaderc-online")]
    #[test]
    fn uniform_array_reads_reach_the_shader_as_deqp_sets_them() {
        use crate::shaderc::online::{arena::Arena, eval, lower};
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }
        let gl = TestGl::new(16, 16);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };
        let reads = [
            ("static", "res += u_arr[0]; res += u_arr[1]; res += u_arr[2]; res += u_arr[3];"),
            ("dynamic", "res += u_arr[ui_zero]; res += u_arr[ui_one]; res += u_arr[ui_two]; res += u_arr[ui_three];"),
            ("static_loop", "for (int i = 0; i < 4; i++) res += u_arr[i];"),
            ("dynamic_loop", "for (int i = 0; i < ui_four; i++) res += u_arr[i];"),
        ];
        let vs = CString::new("attribute vec4 p;\nvoid main() { gl_Position = p; }\n").unwrap();
        let base = [0.25f32, 0.5, 0.75, 1.0];
        for (n, ty) in [(1usize, "float"), (2, "vec2"), (3, "vec3"), (4, "vec4")] {
            let pad = ["", ", 0.0, 0.0, 1.0", ", 0.0, 1.0", ", 1.0", ""][n];
            for (what, read) in reads {
                let fs_src = std::format!(
                    "precision mediump float;\nuniform mediump int ui_zero, ui_one, ui_two, ui_three, ui_four;\n\
                     uniform mediump {ty} u_arr[4];\n\
                     void main() {{ mediump {ty} res = {ty}(0.0); {read} gl_FragColor = vec4(res{pad}); }}\n"
                );
                let fs = CString::new(fs_src.clone()).unwrap();
                /* element k is base[..n] * 2^-k, as dEQP's halving */
                let mut vals = std::vec![0f32; 4 * n];
                for k in 0..4 {
                    for c in 0..n {
                        vals[k * n + c] = base[c] / (1 << k) as f32;
                    }
                }
                let mut want = [0f32, 0.0, 0.0, 1.0];
                for c in 0..n {
                    want[c] = (0..4).map(|k| vals[k * n + c]).sum();
                }
                // SAFETY: plain GL calls on handles this test owns; every
                // pointer is valid for what is read.
                unsafe {
                    let sh = |t: GLenum, s: &CString| {
                        let h = glCreateShader(t);
                        let p: [*const core::ffi::c_char; 1] = [s.as_ptr()];
                        glShaderSource(h, 1, p.as_ptr(), core::ptr::null());
                        glCompileShader(h);
                        h
                    };
                    let p = glCreateProgram();
                    glAttachShader(p, sh(GL_VERTEX_SHADER, &vs));
                    glAttachShader(p, sh(GL_FRAGMENT_SHADER, &fs));
                    glLinkProgram(p);
                    let mut ok: GLint = 0;
                    glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                    assert_eq!(ok, 1, "{fs_src}");
                    glUseProgram(p);
                    let loc = |s: &str| {
                        let c = CString::new(s).unwrap();
                        glGetUniformLocation(p, c.as_ptr())
                    };
                    for (i, u) in ["ui_zero", "ui_one", "ui_two", "ui_three", "ui_four"].iter().enumerate() {
                        glUniform1i(loc(u), if i == 4 { 4 } else { i as GLint });
                    }
                    let l = loc("u_arr");
                    assert!(l >= 0, "{ty} {what}: u_arr has no location");
                    match n {
                        1 => glUniform1fv(l, 4, vals.as_ptr()),
                        2 => glUniform2fv(l, 4, vals.as_ptr()),
                        3 => glUniform3fv(l, 4, vals.as_ptr()),
                        _ => glUniform4fv(l, 4, vals.as_ptr()),
                    }
                    assert_eq!(gl.error(), GL_NO_ERROR, "{ty} {what}");
                    static POS: [f32; 12] = [0.0; 12];
                    crate::gl::vertex::glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
                    crate::gl::vertex::glEnableVertexAttribArray(0);
                    crate::gl::draw::glDrawArrays(GL_TRIANGLES, 0, 3);
                    assert_eq!(gl.error(), GL_NO_ERROR);
                    let d = gl.draws().pop().expect("a draw");
                    let mut buf = std::vec![0u8; 4 << 20];
                    let a = Arena::new(&mut buf);
                    let lw = lower::compile_to_ir(&a, crate::shaderc::online::sema::Stage::Fragment, &fs_src)
                        .unwrap_or_else(|e| panic!("refused: {}", e.msg));
                    let out = eval::run(&lw.module, &eval::Inputs { io: &[], uniforms: &d.fs_uniforms }).expect("runs");
                    let got = out.output_f32(crate::shaderc::online::ir::IoSlot::FragColor(0)).expect("colour");
                    for c in 0..4 {
                        assert!((got[c] - want[c]).abs() < 1e-5, "{ty} {what}: got {got:?} want {want:?}");
                    }
                    glDeleteProgram(p);
                }
            }
        }
    }

    /// Deleting programs and shaders gives the compiler's keep region back.
    ///
    /// The in-process compiler keeps every compiled shader in a bounded
    /// region (`store::KEEP_BYTES`, 512 KiB of `.bss`) because a `ShcShader`
    /// the GL core holds points straight into it. Until 2026-09-23 nothing
    /// ever gave it back: an application that creates, links and deletes
    /// programs -- a UI as pages come and go, or dEQP, one program per case --
    /// walked up to the ceiling in about 192 cycles and then could not
    /// compile at all (`Abi(OutOfMemory { left: 0 })`).
    ///
    /// Now the GL core holds a reference for each copy it keeps (the shader
    /// object's, the program's snapshot at link) and the last release frees
    /// the shader. Pinned here, through the GL entry points an application
    /// calls:
    ///
    /// * 1,000 create / compile / link / delete cycles of distinct programs,
    ///   five times the old ceiling, each returning the region to exactly
    ///   where it was;
    /// * a program outlives the shader objects it was linked from (deleted
    ///   right after the link, as Qt does) and frees them when it goes;
    /// * a relink of the same shaders, and a recompile of the same text, cost
    ///   nothing.
    ///
    /// Checked: removing the `release_shader` calls from `destroy_program`
    /// fails the first cycle's equality, removing the one in `destroy_shader`
    /// does too, and removing the one in `glCompileShader` fails the first
    /// recompile.
    /* The region belongs to the in-process compiler, so this says nothing in
     * a build without one; and the PowerVR code generator does not build a
     * fragment shader that reads nothing yet everywhere, so the GC7000 is the
     * build where a whole program compiles. */
    #[cfg(all(feature = "shaderc-online", feature = "gpu-vivante"))]
    #[test]
    fn deleting_programs_and_shaders_gives_the_compilers_region_back() {
        use crate::shaderc::online::store::keep_in_use;

        /* The host build's back end is the offline table; an application on a
         * no-table image gets the in-process compiler, and that is the one
         * with a region. Restored on the way out INCLUDING through a panic --
         * a test that left the global pointing at another back end would make
         * the next test fail for a reason nothing in it explains. */
        struct BackEnd(*const crate::shaderc::ShcBackend);
        impl Drop for BackEnd {
            fn drop(&mut self) {
                // SAFETY: restoring what was saved, under the suite's guard.
                unsafe { crate::shaderc::g_shaderc = self.0 };
            }
        }

        let gl = TestGl::new(64, 64);
        // SAFETY: the guard serialises the suite; the restorer puts it back.
        let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
        // SAFETY: as above; a static table pointer.
        unsafe { crate::shaderc::g_shaderc = &crate::shaderc::online::SHC_ONLINE };

        let vs = CString::new("#version 100\nattribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n").unwrap();
        let fs_text = |i: u32| {
            CString::new(std::format!(
                "#version 100\nprecision mediump float;\nvoid main() {{ gl_FragColor = vec4({i}.0/1000.0, 0.0, 0.0, 1.0); }}\n"
            ))
            .unwrap()
        };
        /* One shader object, compiled; its handle. */
        let shader = |ty: GLenum, text: &CString| {
            // SAFETY: plain GL calls on a handle this test owns.
            unsafe {
                let s = glCreateShader(ty);
                let p: [*const core::ffi::c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                let mut ok: GLint = 0;
                crate::gl::shader::glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
                assert_eq!(ok, 1, "{:?} did not compile", text);
                s
            }
        };
        let program = |v: GLuint, f: GLuint| {
            // SAFETY: plain GL calls on handles this test owns.
            unsafe {
                let p = glCreateProgram();
                glAttachShader(p, v);
                glAttachShader(p, f);
                glLinkProgram(p);
                let mut lk: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut lk);
                assert_eq!(lk, 1, "program did not link");
                p
            }
        };

        let before = keep_in_use();
        for i in 0..1_000u32 {
            let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs_text(i)));
            let p = program(v, f);
            assert!(keep_in_use() > before, "cycle {i}: the shaders are held");
            // SAFETY: plain GL calls on handles this test owns.
            unsafe {
                glDeleteProgram(p);
                crate::gl::shader::glDeleteShader(v);
                crate::gl::shader::glDeleteShader(f);
            }
            assert_eq!(keep_in_use(), before, "cycle {i}: the region is back where it started");
        }

        /* The program outlives the shader objects it was linked from. */
        let (v, f) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs_text(5000)));
        let p = program(v, f);
        let held = keep_in_use();
        // SAFETY: plain GL calls on handles this test owns.
        unsafe {
            crate::gl::shader::glDeleteShader(v);
            crate::gl::shader::glDeleteShader(f);
        }
        /* Deleting an attached shader only flags it (ES 2.0 2.10.3). Detach
         * them, so the shader objects really go and only the program's
         * snapshot is left. */
        // SAFETY: as above.
        unsafe {
            glDetachShader(p, v);
            glDetachShader(p, f);
        }
        assert_eq!(keep_in_use(), held, "the program still holds both shaders");
        let rfs = gl.ctx().objs_ref().programs[obj_index(p)].rfs;
        assert_eq!(crate::shaderc::online::backend::holders(&rfs), Some(1), "and is their only holder");

        /* A relink of the same shaders and a recompile of the same text cost
         * nothing: the second is the cache's answer, not a second copy. */
        let (v2, f2) = (shader(GL_VERTEX_SHADER, &vs), shader(GL_FRAGMENT_SHADER, &fs_text(5000)));
        assert_eq!(keep_in_use(), held, "a recompile of text the program holds is a cache hit");
        // SAFETY: as above.
        unsafe {
            glAttachShader(p, v2);
            glAttachShader(p, f2);
            for _ in 0..8 {
                glLinkProgram(p);
            }
        }
        assert_eq!(keep_in_use(), held, "relinks cost nothing");
        assert_eq!(crate::shaderc::online::backend::holders(&rfs), Some(2), "the object and the program");
        // SAFETY: as above.
        unsafe {
            glDeleteProgram(p);
            crate::gl::shader::glDeleteShader(v2);
            crate::gl::shader::glDeleteShader(f2);
        }
        assert_eq!(keep_in_use(), before, "and it all goes when the last holder does");

        /* A shader object compiled again lets go of what it compiled before:
         * after each recompile the region holds exactly the new shader.
         * (Its own size, not the first one's: two texts of one length need
         * not compile to one size -- after the merge with 63b50b3
         * `6001.0/1000.0` took a granule more than `6000.0/1000.0`.) */
        let s = shader(GL_FRAGMENT_SHADER, &fs_text(6000));
        let held = |gl: &TestGl| gl.ctx().objs_ref().shaders[obj_index(s)].compiled_shader;
        for i in 6001..6100 {
            let t = fs_text(i);
            // SAFETY: as above.
            unsafe {
                let tp: [*const core::ffi::c_char; 1] = [t.as_ptr()];
                glShaderSource(s, 1, tp.as_ptr(), core::ptr::null());
                glCompileShader(s);
            }
            let now = held(&gl);
            assert_eq!(crate::shaderc::online::backend::holders(&now), Some(1), "recompile {i}: the object holds it");
            let own = crate::shaderc::online::backend::owned_bytes(&now).expect("a live record");
            assert_eq!(keep_in_use(), before + own, "recompile {i} holds one shader, not two");
        }
        // SAFETY: as above.
        unsafe { crate::gl::shader::glDeleteShader(s) };
        assert_eq!(keep_in_use(), before);
    }

    /* ---- helpers ---- */

    fn is_float_type(t: u16) -> bool {
        uniform_class(t).is_some_and(|k| k.base == Base::Float)
    }

    fn is_int_type(t: u16) -> bool {
        uniform_class(t).is_some_and(|k| k.base != Base::Float)
    }

    /// Value the uniform blocks are pre-filled with to see what a setter leaves alone (a normal float, so the
    /// bit pattern survives being moved around as `f32`).
    const SENTINEL: u32 = 0x1234_5678;

    fn fresh() -> TestGl {
        TestGl::new(16, 16)
    }

    /* ---- the link-time attribute layout (SHADERC_NOTES.md) -------------------------------------
     *
     * A fake back end stands in for the compilers' halves: its vertex
     * compile leaves the layout to the link when the source says so
     * ("LAYOUT_AT_LINK", with the declared list dEQP's max_* shapes have),
     * and its link-time compile records the map it is handed and answers
     * one row per active location. */

    /// dEQP bind_aliasing.max_cond_float: 17 floats a_0..a_16, a_0 and a_16
    /// bound to one location, the rest one each.
    fn declared_17() -> &'static [ShcAttribBinding] {
        static D: std::sync::OnceLock<Vec<ShcAttribBinding>> = std::sync::OnceLock::new();
        D.get_or_init(|| (0..17).map(|i| decl(&std::format!("a_{i}"), 1, GL_FLOAT, 1)).collect())
    }

    /// declared_17 from a compile that does not describe its attributes
    /// (no gl_type, no active): the record's rows answer the queries.
    fn declared_17_bare() -> &'static [ShcAttribBinding] {
        static D: std::sync::OnceLock<Vec<ShcAttribBinding>> = std::sync::OnceLock::new();
        D.get_or_init(|| (0..17).map(|i| decl(&std::format!("a_{i}"), 1, 0, 0)).collect())
    }

    /// dEQP max_inactive's shape: 17 declared, a_5 never read.
    fn declared_17_inactive() -> &'static [ShcAttribBinding] {
        static D: std::sync::OnceLock<Vec<ShcAttribBinding>> = std::sync::OnceLock::new();
        D.get_or_init(|| (0..17).map(|i| decl(&std::format!("a_{i}"), 1, GL_FLOAT, (i != 5) as u8)).collect())
    }

    /// Aliased names of different types: v (vec4) and m (mat2) bound to one
    /// location, u (vec3) declared but not read.
    fn declared_typed() -> &'static [ShcAttribBinding] {
        static D: std::sync::OnceLock<Vec<ShcAttribBinding>> = std::sync::OnceLock::new();
        D.get_or_init(|| {
            std::vec![decl("v", 1, GL_FLOAT_VEC4, 1), decl("u", 1, GL_FLOAT_VEC3, 0), decl("m", 2, GL_FLOAT_MAT2, 1)]
        })
    }

    fn decl(name: &str, columns: u8, gl_type: GLenum, active: u8) -> ShcAttribBinding {
        ShcAttribBinding {
            name: util::cname::<32>(name),
            location: SHC_ATTRIB_NO_LOCATION,
            columns,
            gl_type: gl_type as u16,
            active,
        }
    }

    /// The map the last link-time compile was handed, and how many ran.
    static LAST_MAP: std::sync::Mutex<Option<(ShcAttribMap, u32)>> = std::sync::Mutex::new(None);

    unsafe extern "C" fn fake_compile(
        stage: u32,
        glsl: *const c_char,
        out: *mut ShcShader,
        _log: *mut c_char,
        _log_sz: usize,
    ) -> i32 {
        // SAFETY: the caller passes a NUL-terminated source and a writable out.
        let src = unsafe { util::cstr_bytes(glsl) };
        let mut sh = ShcShader::ZERO;
        if stage == shaderc::SHC_STAGE_VERTEX && src.windows(14).any(|w| w == b"LAYOUT_AT_LINK") {
            let has = |k: &[u8]| src.windows(k.len()).any(|w| w == k);
            let d = if has(b"BARE") {
                declared_17_bare()
            } else if has(b"INACTIVE") {
                declared_17_inactive()
            } else if has(b"TYPED") {
                declared_typed()
            } else {
                declared_17()
            };
            sh.layout_at_link = 1;
            sh.declared = d.as_ptr();
            sh.num_declared = d.len() as u8;
        } else if stage == shaderc::SHC_STAGE_VERTEX {
            /* two ordinary attributes */
            for (i, n) in ["p", "q"].iter().enumerate() {
                sh.attribs[i] = ShcAttrib {
                    location: i as u8,
                    components: 4,
                    name: util::cname::<32>(n),
                    gl_type: GL_FLOAT_VEC4 as u16,
                    ..ShcAttrib::ZERO
                };
            }
            sh.num_attribs = 2;
        }
        // SAFETY: as above.
        unsafe { *out = sh };
        0
    }

    unsafe extern "C" fn fake_relink(
        glsl: *const c_char,
        map: *const ShcAttribMap,
        out: *mut ShcShader,
        log: *mut c_char,
        log_sz: usize,
    ) -> i32 {
        // SAFETY: the caller passes a valid map, source, out and log.
        let (map, src) = unsafe { (*map, util::cstr_bytes(glsl)) };
        let mut last = LAST_MAP.lock().unwrap();
        let runs = last.map_or(0, |l| l.1) + 1;
        *last = Some((map, runs));
        if src.windows(13).any(|w| w == b"FAIL_AT_LINK_") {
            use core::fmt::Write as _;
            // SAFETY: as above.
            let mut w = unsafe { util::CStrWriter::new(log, log_sz) };
            let _ = write!(w, "VtxIn needed 65");
            return -util::EINVAL;
        }
        /* one row per active location, the first name there */
        let mut sh = ShcShader::ZERO;
        let mut n = 0;
        for d in map.b[..map.count as usize].iter().filter(|d| d.location != SHC_ATTRIB_NO_LOCATION) {
            if !sh.attribs[..n].iter().any(|a| a.location == d.location) && n < 16 {
                sh.attribs[n] = ShcAttrib { location: d.location, components: 1, name: d.name, ..ShcAttrib::ZERO };
                n += 1;
            }
        }
        sh.num_attribs = n as u8;
        // SAFETY: as above.
        unsafe { *out = sh };
        0
    }

    static FAKE_LAYOUT: shaderc::ShcBackend = shaderc::ShcBackend {
        compile: Some(fake_compile),
        link: None,
        blend_variant: None,
        compile_vs_linked: Some(fake_relink),
        lod_sensitivity: None,
        sampler_variant: None,
    };

    /// Compile and link `vs` with the fake back end after `binds`; the
    /// program, its VS object, and the map the link-time compile got.
    fn link_with_layout(vs: &str, binds: &[(&str, GLuint)]) -> (GLuint, GLuint, Option<(ShcAttribMap, u32)>) {
        *LAST_MAP.lock().unwrap() = None;
        // SAFETY: TestGl serialises the tests; the back end is restored right after.
        unsafe {
            let saved = shaderc::g_shaderc;
            shaderc::g_shaderc = &FAKE_LAYOUT;
            let sh = |ty: GLenum, t: &str| {
                let s = glCreateShader(ty);
                let c = CString::new(t).unwrap();
                let p: [*const c_char; 1] = [c.as_ptr()];
                glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
                glCompileShader(s);
                s
            };
            let p = glCreateProgram();
            let v = sh(GL_VERTEX_SHADER, vs);
            glAttachShader(p, v);
            glAttachShader(p, sh(GL_FRAGMENT_SHADER, "void main() {}"));
            for (n, i) in binds {
                let c = CString::new(*n).unwrap();
                glBindAttribLocation(p, *i, c.as_ptr());
            }
            glLinkProgram(p);
            shaderc::g_shaderc = saved;
            (p, v, *LAST_MAP.lock().unwrap())
        }
    }

    /// max_cond_float: 17 declared floats, a_0 and a_16 on location 0, the
    /// rest bound one each. The compile reports COMPILE_STATUS true (dEQP
    /// reads vertexCompileOk: today's failure), the link hands the back end
    /// the map with the aliased pair on one location, and the program links
    /// with 16 active locations; the aliased name the record gave no row
    /// still answers glGetAttribLocation.
    #[test]
    fn a_vertex_shader_with_more_attributes_than_locations_links_with_the_bindings() {
        let gl = fresh();
        let mut binds: Vec<(String, GLuint)> = (0..16).map(|i| (std::format!("a_{i}"), i)).collect();
        binds.push(("a_16".into(), 0));
        let b: Vec<(&str, GLuint)> = binds.iter().map(|(n, i)| (n.as_str(), *i)).collect();
        let (p, v, map) = link_with_layout("LAYOUT_AT_LINK", &b);
        assert_eq!(shader_iv(v, GL_COMPILE_STATUS), 1, "compiled for its reflection, laid out at link");
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        let (map, runs) = map.expect("the link-time compile ran");
        assert_eq!((map.count, runs), (17, 1));
        let loc_of = |n: &str| map.b[..17].iter().find(|d| util::name_bytes(&d.name) == n.as_bytes()).unwrap().location;
        assert_eq!((loc_of("a_0"), loc_of("a_16"), loc_of("a_15")), (0, 0, 15), "the aliased pair on one location");
        let name = CString::new("a_16").unwrap();
        // SAFETY: a NUL-terminated name.
        assert_eq!(unsafe { glGetAttribLocation(p, name.as_ptr()) }, 0);
        assert_eq!(gl.ctx().objs_ref().programs[obj_index(p)].rvs.num_attribs, 16, "one row per active location");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// Unbound attributes take the lowest free locations around the bound
    /// ones; a failing link-time compile is a LINK error carrying the
    /// compiler's own log; a program with no alias and nothing deferred
    /// never reaches the link-time compile.
    #[test]
    fn the_link_time_layout_packs_unbound_attributes_and_reports_a_failed_recompile() {
        let _gl = fresh();
        /* a_3 and a_16 share 0, a_9 has 1: the other 14 fill 2..15 in order */
        let (_, _, map) = link_with_layout("LAYOUT_AT_LINK", &[("a_3", 0), ("a_16", 0), ("a_9", 1)]);
        let (map, _) = map.unwrap();
        let loc_of = |n: &str| map.b[..17].iter().find(|d| util::name_bytes(&d.name) == n.as_bytes()).unwrap().location;
        assert_eq!((loc_of("a_3"), loc_of("a_9"), loc_of("a_0"), loc_of("a_1")), (0, 1, 2, 3));
        /* 17 unaliased attributes cannot all have a location */
        let (p, _, _) = link_with_layout("LAYOUT_AT_LINK", &[]);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 0);
        assert_eq!(
            info_log(p),
            "link error: the vertex shader's attributes do not fit MAX_VERTEX_ATTRIBS = 16 \
             (OpenGL ES 2.0.25 section 2.10.4): no 1 free location(s) left for 'a_16'"
        );
        let mut b: Vec<(String, GLuint)> = (0..16).map(|i| (std::format!("a_{i}"), i)).collect();
        b.push(("a_16".into(), 3));
        let b: Vec<(&str, GLuint)> = b.iter().map(|(n, i)| (n.as_str(), *i)).collect();
        let (p, v, _) = link_with_layout("LAYOUT_AT_LINK FAIL_AT_LINK_", &b);
        assert_eq!(shader_iv(v, GL_COMPILE_STATUS), 1);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 0);
        assert!(info_log(p).contains("VtxIn needed 65"), "the compiler's log: {}", info_log(p));
        let (p, _, map) = link_with_layout("plain", &[]);
        assert_eq!((get_iv(p, GL_LINK_STATUS), map.is_none()), (1, true), "no alias, nothing deferred");
        let (p, _, map) = link_with_layout("plain", &[("p", 2), ("q", 2)]);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        assert_eq!(map.map(|m| m.1), Some(1), "two bound to one location: the layout is compiled in");
    }

    fn active_attrib(p: GLuint, i: GLuint) -> (String, GLenum, GLint, GLenum) {
        let (mut len, mut size, mut ty) = (0, 0, 0);
        let mut buf = [0 as c_char; 64];
        // SAFETY: out-params sized as glGetActiveAttrib writes them.
        unsafe { glGetActiveAttrib(p, i, buf.len() as GLsizei, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
        let name = String::from_utf8_lossy(util::name_bytes(&buf)).into_owned();
        (name, ty, size, unsafe { crate::gl::state::glGetError() })
    }

    /// max_cond_float: 17 active names over 16 locations. Every one is an
    /// active attribute (dEQP: "Active attribute a_16 wasn't returned by
    /// glGetActiveAttrib()"), at the location the link decided, though the
    /// record has one row per location.
    #[test]
    fn a_relinked_program_lists_every_active_name_though_two_share_a_location() {
        let gl = fresh();
        let mut binds: Vec<(String, GLuint)> = (0..16).map(|i| (std::format!("a_{i}"), i)).collect();
        binds.push(("a_16".into(), 0));
        let b: Vec<(&str, GLuint)> = binds.iter().map(|(n, i)| (n.as_str(), *i)).collect();
        let (p, _, _) = link_with_layout("LAYOUT_AT_LINK", &b);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!(gl.ctx().objs_ref().programs[obj_index(p)].rvs.num_attribs, 16, "one row per active location");
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 17);
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTE_MAX_LENGTH), 5, "\"a_16\" and its NUL");
        let names: Vec<String> = (0..17).map(|i| active_attrib(p, i).0).collect();
        for i in 0..17 {
            assert!(names.contains(&std::format!("a_{i}")), "a_{i} is listed: {names:?}");
        }
        assert_eq!(active_attrib(p, 16), ("a_16".into(), GL_FLOAT, 1, GL_NO_ERROR));
        assert_eq!(active_attrib(p, 17).3, GL_INVALID_VALUE);
        assert_eq!((attrib_loc(p, "a_16"), attrib_loc(p, "a_0"), attrib_loc(p, "a_15")), (0, 0, 15));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// A declared attribute the shader never reads is not active: not
    /// listed, no location, and it takes no location from the others.
    #[test]
    fn a_relinked_program_leaves_a_declared_but_unread_attribute_out() {
        let _gl = fresh();
        let mut binds: Vec<(String, GLuint)> = (0..16).map(|i| (std::format!("a_{i}"), i)).collect();
        binds.push(("a_16".into(), 5));
        let b: Vec<(&str, GLuint)> = binds.iter().map(|(n, i)| (n.as_str(), *i)).collect();
        let (p, _, _) = link_with_layout("LAYOUT_AT_LINK INACTIVE", &b);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 16);
        assert!((0..16).all(|i| active_attrib(p, i).0 != "a_5"));
        assert_eq!((attrib_loc(p, "a_5"), attrib_loc(p, "a_16")), (-1, 5));
        /* unbound: the 16 read ones fit the 16 locations (max_inactive_*) */
        let (p, _, _) = link_with_layout("LAYOUT_AT_LINK INACTIVE", &[]);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!((attrib_loc(p, "a_4"), attrib_loc(p, "a_6"), attrib_loc(p, "a_16")), (4, 5, 15));
    }

    /// Two names of different types on one location: each is reported with
    /// its own declared type, the matrix as one attribute of size 1.
    #[test]
    fn aliased_names_keep_their_declared_types() {
        let _gl = fresh();
        let (p, _, _) = link_with_layout("LAYOUT_AT_LINK TYPED", &[("v", 3), ("m", 3)]);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 2);
        assert_eq!(active_attrib(p, 0), ("v".into(), GL_FLOAT_VEC4, 1, GL_NO_ERROR));
        assert_eq!(active_attrib(p, 1), ("m".into(), GL_FLOAT_MAT2, 1, GL_NO_ERROR));
        assert_eq!((attrib_loc(p, "v"), attrib_loc(p, "m"), attrib_loc(p, "u")), (3, 3, -1));
    }

    /// A compile that does not describe its declared attributes (a back end
    /// from before gl_type/active): the queries answer from the rows, as
    /// before, and a relink without them does not lose an attribute.
    #[test]
    fn an_undescribed_declared_list_answers_from_the_rows() {
        let _gl = fresh();
        let mut binds: Vec<(String, GLuint)> = (0..16).map(|i| (std::format!("a_{i}"), i)).collect();
        binds.push(("a_16".into(), 0));
        let b: Vec<(&str, GLuint)> = binds.iter().map(|(n, i)| (n.as_str(), *i)).collect();
        let (p, _, _) = link_with_layout("LAYOUT_AT_LINK BARE", &b);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 16, "the rows");
        assert_eq!(attrib_loc(p, "a_16"), 0, "the bindings, as before");
    }

    /// A relink with plain rows (bindings that alias, nothing deferred):
    /// the map built from the rows says what they said.
    #[test]
    fn a_map_built_from_rows_carries_their_types() {
        let _gl = fresh();
        let (p, _, map) = link_with_layout("plain", &[("p", 2), ("q", 2)]);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        let (map, _) = map.unwrap();
        assert!(map.b[..2].iter().all(|d| d.active == 1 && d.gl_type == GL_FLOAT_VEC4 as u16));
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 2);
        assert_eq!((attrib_loc(p, "p"), attrib_loc(p, "q")), (2, 2));
    }

    fn uni(name: &str, dword_off: u16, dwords: u16, gl_type: u32, array_len: u8) -> ShcUniform {
        let mut u = ShcUniform { dword_off, dwords, gl_type: gl_type as u16, array_len, inner: 1, ..ShcUniform::ZERO };
        util::set_name(&mut u.name, name.as_bytes());
        u
    }

    fn sampler(name: &str, binding: u8) -> ShcUniform {
        let mut u = uni(name, SHC_UNIFORM_NO_BLOCK, 1, GL_SAMPLER_2D, 1);
        u.binding = binding;
        u
    }

    /// Point a stage's reflection at a hand-built (leaked) table.
    fn set_table(stage: &mut crate::shaderc::ShcShader, table: Vec<ShcUniform>) {
        let table: &'static [ShcUniform] = Box::leak(table.into_boxed_slice());
        stage.uniforms = table.as_ptr();
        stage.num_uniforms = table.len() as u8;
    }

    /// A linked, current program whose uniform reflection is replaced by hand-built tables: the offline table has
    /// no ivec/mat2/bool uniforms and few arrays, and the setters only look at the reflection.
    fn fake_program(gl: &TestGl, vs: Vec<ShcUniform>, fs: Vec<ShcUniform>) -> GLuint {
        fake_program_of(gl, vs, fs, false)
    }

    fn fake_program_of(gl: &TestGl, vs: Vec<ShcUniform>, fs: Vec<ShcUniform>, offline: bool) -> GLuint {
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        let c = gl.ctx();
        let p = &mut context::objects(c.group).programs[obj_index(prog)];
        if offline {
            crate::testutil::mark_stage_offline(&mut p.rvs);
            crate::testutil::mark_stage_offline(&mut p.rfs);
        }
        set_table(&mut p.rvs, vs);
        set_table(&mut p.rfs, fs);
        // SAFETY: TestGl serialises the tests; no other reference to STATE is live.
        let st = unsafe { STATE.get() };
        let mut log = [0u8; INFO_LOG_SIZE];
        assert_eq!(build_locations(p, &mut st.loc[c.group as usize][obj_index(prog)], &mut log), 0);
        /* the hand-built layouts reach further than the linked shaders' blocks */
        p.free_uniforms();
        // SAFETY: C heap blocks the program owns from here (freed with it).
        unsafe {
            p.vs_uniforms = util::calloc(MAX_UNIFORM_DWORDS, 4) as *mut f32;
            p.fs_uniforms = util::calloc(MAX_UNIFORM_DWORDS, 4) as *mut f32;
        }
        assert!(!p.vs_uniforms.is_null() && !p.fs_uniforms.is_null());
        (p.vs_uniform_cap, p.fs_uniform_cap) = (MAX_UNIFORM_DWORDS as u16, MAX_UNIFORM_DWORDS as u16);
        let (vb, fb) = p.blocks_mut();
        vb.fill(f32::from_bits(SENTINEL));
        fb.fill(f32::from_bits(SENTINEL));
        // SAFETY: plain GL call.
        unsafe { glUseProgram(prog) };
        prog
    }

    /// [`fake_program`] with both stages read as the offline table's: the
    /// reflection shape of `light_array_reflection` (flattened struct-array
    /// rows) is only read as that for an offline stage, which a
    /// `no-shader-table` build has none of until the stages are marked.
    fn fake_offline_program(gl: &TestGl, vs: Vec<ShcUniform>, fs: Vec<ShcUniform>) -> GLuint {
        fake_program_of(gl, vs, fs, true)
    }

    #[test]
    fn every_es2_uniform_type_is_enumerated_and_reported_as_itself() {
        /* `dEQP-GLES2.functional.state_query.shader.program_active_uniform_types`,
         * the one survivor of the 8b5428a sweep ("got invalid value"). It
         * builds a program carrying one uniform of each ES 2.0 type and
         * compares what glGetActiveUniform reports, type for type. The two
         * opaque types are the suspects -- a sampler has no block offset, so
         * it reaches the enumeration by a different route than a float does --
         * which is what this pins. */
        let gl = fresh();
        let types: [(&str, u32, u16); 17] = [
            ("u_float", GL_FLOAT, 1),
            ("u_vec2", GL_FLOAT_VEC2, 2),
            ("u_vec3", GL_FLOAT_VEC3, 3),
            ("u_vec4", GL_FLOAT_VEC4, 4),
            ("u_int", GL_INT, 1),
            ("u_ivec2", GL_INT_VEC2, 2),
            ("u_ivec3", GL_INT_VEC3, 3),
            ("u_ivec4", GL_INT_VEC4, 4),
            ("u_bool", GL_BOOL, 1),
            ("u_bvec2", GL_BOOL_VEC2, 2),
            ("u_bvec3", GL_BOOL_VEC3, 3),
            ("u_bvec4", GL_BOOL_VEC4, 4),
            ("u_mat2", GL_FLOAT_MAT2, 4),
            ("u_mat3", GL_FLOAT_MAT3, 12),
            ("u_mat4", GL_FLOAT_MAT4, 16),
            ("u_sampler2D", GL_SAMPLER_2D, 1),
            ("u_samplerCube", GL_SAMPLER_CUBE, 1),
        ];
        let mut vs = std::vec::Vec::new();
        let mut off = 0u16;
        for (name, t, dwords) in types {
            if t == GL_SAMPLER_2D || t == GL_SAMPLER_CUBE {
                let mut u = uni(name, SHC_UNIFORM_NO_BLOCK, 1, t, 1);
                u.binding = if t == GL_SAMPLER_2D { 0 } else { 1 };
                vs.push(u);
            } else {
                vs.push(uni(name, off, dwords, t, 1));
                off += dwords.max(4); /* vec4-strided, so nothing overlaps */
            }
        }
        let prog = fake_program(&gl, vs, std::vec![]);

        let mut count = 0 as GLint;
        // SAFETY: plain GL calls; one GLint out.
        unsafe { glGetProgramiv(prog, GL_ACTIVE_UNIFORMS, &mut count) };
        assert_eq!(count, types.len() as GLint, "every type is one active uniform");

        /* Collect what is reported, then compare as a set: the enumeration
         * order is ours to choose, the names and types are not. */
        let mut seen = std::vec::Vec::new();
        for i in 0..count {
            let (mut size, mut kind) = (0 as GLint, 0 as GLenum);
            let mut name = [0 as c_char; 64];
            // SAFETY: one GLint, one GLenum and 64 bytes of name.
            unsafe {
                glGetActiveUniform(
                    prog,
                    i as GLuint,
                    name.len() as GLsizei,
                    core::ptr::null_mut(),
                    &mut size,
                    &mut kind,
                    name.as_mut_ptr(),
                )
            };
            assert_eq!(gl.error(), GL_NO_ERROR, "index {i} of {count}");
            assert_eq!(size, 1, "none of these is an array");
            // SAFETY: the writer NUL-terminated it.
            let n = unsafe { util::cstr_bytes(name.as_ptr()) };
            seen.push((String::from_utf8(n.to_vec()).unwrap(), kind));
        }
        for (name, t, _) in types {
            let got = seen.iter().find(|(n, _)| n == name);
            let Some((_, kind)) = got else {
                panic!("{name} is missing from the enumeration: {seen:?}");
            };
            assert_eq!(*kind, t, "{name} is reported as 0x{kind:x}, not 0x{t:x}");
        }
    }

    /// The layout the setter tests run on. Offsets follow std430 as the compiler packs the block: vec3 arrays and
    /// mat3 columns are vec4-strided, everything else is tight.
    fn setter_program(gl: &TestGl) -> GLuint {
        let vs = std::vec![
            uni("f1", 0, 1, GL_FLOAT, 1),
            uni("v2", 2, 2, GL_FLOAT_VEC2, 1),
            uni("v3", 4, 3, GL_FLOAT_VEC3, 1),
            uni("v4", 8, 4, GL_FLOAT_VEC4, 1),
            uni("fa", 12, 4, GL_FLOAT, 4),
            uni("v2a", 16, 6, GL_FLOAT_VEC2, 3),
            uni("v3a", 24, 12, GL_FLOAT_VEC3, 3),
            uni("v4a", 36, 8, GL_FLOAT_VEC4, 2),
            uni("m2", 44, 4, GL_FLOAT_MAT2, 1),
            uni("m3", 48, 12, GL_FLOAT_MAT3, 1),
            uni("m4", 60, 16, GL_FLOAT_MAT4, 1),
            uni("m3a", 76, 24, GL_FLOAT_MAT3, 2),
            uni("m4a", 100, 32, GL_FLOAT_MAT4, 2),
            uni("i1", 132, 1, GL_INT, 1),
            uni("i2", 134, 2, GL_INT_VEC2, 1),
            uni("i3", 136, 3, GL_INT_VEC3, 1),
            uni("i4", 140, 4, GL_INT_VEC4, 1),
            uni("ia", 144, 3, GL_INT, 3),
            uni("i4a", 148, 8, GL_INT_VEC4, 2),
            uni("b1", 156, 1, GL_BOOL, 1),
        ];
        let fs = std::vec![
            uni("fonly", 0, 1, GL_FLOAT, 1),
            uni("v4", 20, 4, GL_FLOAT_VEC4, 1),
            sampler("tex", 3),
            sampler("tex7", 7)
        ];
        fake_program(gl, vs, fs)
    }

    fn loc(prog: GLuint, name: &str) -> GLint {
        let name = CString::new(name).unwrap();
        // SAFETY: a valid NUL-terminated name.
        unsafe { glGetUniformLocation(prog, name.as_ptr()) }
    }

    fn vs_bits(gl: &TestGl, prog: GLuint, off: usize, n: usize) -> Vec<u32> {
        gl.ctx().objs().programs[obj_index(prog)].vs_block()[off..off + n].iter().map(|f| f.to_bits()).collect()
    }

    fn fs_bits(gl: &TestGl, prog: GLuint, off: usize, n: usize) -> Vec<u32> {
        gl.ctx().objs().programs[obj_index(prog)].fs_block()[off..off + n].iter().map(|f| f.to_bits()).collect()
    }

    /// Bit patterns of floats, `None` standing for "left as it was".
    fn bits(v: &[Option<f32>]) -> Vec<u32> {
        v.iter().map(|f| f.map_or(SENTINEL, f32::to_bits)).collect()
    }

    fn fbits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|f| f.to_bits()).collect()
    }

    fn get_iv(prog: GLuint, pname: GLenum) -> GLint {
        let mut v: GLint = -12345;
        // SAFETY: v is one GLint.
        unsafe { glGetProgramiv(prog, pname, &mut v) };
        v
    }

    /// glGetShaderiv of one pname (the shader mirror of [`get_iv`]).
    fn shader_iv(sh: GLuint, pname: GLenum) -> GLint {
        let mut v: GLint = -12345;
        // SAFETY: v is one GLint.
        unsafe { shader::glGetShaderiv(sh, pname, &mut v) };
        v
    }

    fn info_log(prog: GLuint) -> String {
        let mut buf = [0x55u8; INFO_LOG_SIZE + 8];
        let mut len: GLsizei = -1;
        // SAFETY: buf holds the announced size.
        unsafe { glGetProgramInfoLog(prog, buf.len() as GLsizei, &mut len, buf.as_mut_ptr() as *mut GLchar) };
        assert_eq!(buf[len as usize], 0, "NUL right after the reported length");
        String::from_utf8(buf[..len as usize].to_vec()).unwrap()
    }

    /// Create a program with the two table shaders attached, not linked.
    fn attached(gl: &TestGl, vs: &str, fs: &str) -> (GLuint, GLuint, GLuint) {
        let v = gl.shader(GL_VERTEX_SHADER, vs);
        let f = gl.shader(GL_FRAGMENT_SHADER, fs);
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            (p, v, f)
        }
    }

    fn attrib_loc(prog: GLuint, name: &str) -> GLint {
        let name = CString::new(name).unwrap();
        // SAFETY: a valid NUL-terminated name.
        unsafe { glGetAttribLocation(prog, name.as_ptr()) }
    }

    fn bind_attrib(prog: GLuint, index: GLuint, name: &str) {
        let name = CString::new(name).unwrap();
        // SAFETY: a valid NUL-terminated name.
        unsafe { glBindAttribLocation(prog, index, name.as_ptr()) }
    }

    /* ---- pure helpers ---- */

    #[test]
    fn parse_ulong_reads_leading_digits() {
        assert_eq!(parse_ulong(b"0"), 0);
        assert_eq!(parse_ulong(b"12].x"), 12);
        assert_eq!(parse_ulong(b"007"), 7);
        assert_eq!(parse_ulong(b""), 0);
        assert_eq!(parse_ulong(b"x1"), 0);
    }

    #[test]
    fn parse_ulong_saturates_like_strtoul() {
        assert_eq!(parse_ulong(b"99999999999999999999999999999999]"), c_ulong::MAX);
    }

    #[test]
    fn float_and_int_type_classes() {
        for t in [GL_FLOAT, GL_FLOAT_VEC2, GL_FLOAT_VEC3, GL_FLOAT_VEC4, GL_FLOAT_MAT2, GL_FLOAT_MAT3, GL_FLOAT_MAT4] {
            assert!(is_float_type(t as u16) && !is_int_type(t as u16), "{t:#x}");
        }
        for t in [
            GL_INT,
            GL_INT_VEC2,
            GL_INT_VEC3,
            GL_INT_VEC4,
            GL_BOOL,
            GL_BOOL_VEC2,
            GL_BOOL_VEC3,
            GL_BOOL_VEC4,
            GL_SAMPLER_2D,
            GL_SAMPLER_CUBE,
        ] {
            assert!(is_int_type(t as u16) && !is_float_type(t as u16), "{t:#x}");
        }
        for t in [0, GL_UNSIGNED_BYTE, GL_TEXTURE_2D] {
            assert!(!is_int_type(t as u16) && !is_float_type(t as u16), "{t:#x}");
        }
    }

    #[test]
    fn store_dwords_copies_bit_patterns() {
        let mut block = [0.0f32; MAX_UNIFORM_DWORDS];
        /* a NaN payload and an integer pattern must survive */
        store_dwords(&mut block, 3, 2, &[0xffff_ffff, 0x8000_0001]);
        assert_eq!(block[3].to_bits(), 0xffff_ffff);
        assert_eq!(block[4].to_bits(), 0x8000_0001);
        assert_eq!((block[2], block[5]), (0.0, 0.0));
    }

    #[test]
    fn store_dwords_skips_a_stage_without_the_uniform() {
        let mut block = [0.0f32; MAX_UNIFORM_DWORDS];
        store_dwords(&mut block, -1, 1, &[1]);
        assert!(block.iter().all(|f| f.to_bits() == 0));
    }

    #[test]
    fn store_dwords_refuses_to_leave_the_block() {
        let mut block = [0.0f32; MAX_UNIFORM_DWORDS];
        let last = MAX_UNIFORM_DWORDS as i32 - 2;
        store_dwords(&mut block, last, 2, &[1, 2]);
        assert_eq!(block[MAX_UNIFORM_DWORDS - 1].to_bits(), 2, "ending exactly at the block's end fits");
        store_dwords(&mut block, last + 1, 2, &[7, 7]);
        assert_eq!(block[MAX_UNIFORM_DWORDS - 1].to_bits(), 2, "nothing is written, not even the part that fits");
        store_dwords(&mut block, i32::MAX, 2, &[7, 7]);
    }

    #[test]
    fn element_off_keeps_absent_stages_absent() {
        assert_eq!(element_off(10, 8), 18);
        assert_eq!(element_off(0, 0), 0);
        assert_eq!(element_off(-1, 8), -1);
    }

    #[test]
    fn field_matches_is_strncmp_over_the_field() {
        let mut field = [0 as c_char; 8];
        util::set_name(&mut field, b"a_pos");
        assert!(field_matches(&field, b"a_pos"));
        assert!(!field_matches(&field, b"a_po"));
        assert!(!field_matches(&field, b"a_pos2"));
        assert!(!field_matches(&field, b""));
        /* a field filled to the brim has no NUL: the comparison stops at its end */
        let full = [b'x' as c_char; 8];
        assert!(field_matches(&full, b"xxxxxxxx"));
        assert!(field_matches(&full, b"xxxxxxxxyz"));
        assert!(!field_matches(&full, b"xxxxxxx"));
    }

    #[test]
    fn write_log_truncates_with_a_nul() {
        let mut log = [0x55u8; INFO_LOG_SIZE];
        write_log(&mut log, format_args!("{}", "y".repeat(400)));
        assert_eq!(log_text(&log).len(), INFO_LOG_SIZE - 1);
        write_log(&mut log, format_args!("short {}", 5));
        assert_eq!(log_text(&log), b"short 5");
    }

    /// A zeroed program/table pair for the link helpers.
    fn blank() -> (Box<GlProgram>, Box<ProgLocTable>, [u8; INFO_LOG_SIZE]) {
        // SAFETY: every field of both types is valid when all-zero (the library zero-fills them the same way).
        unsafe { (Box::new(core::mem::zeroed()), Box::new(core::mem::zeroed()), [0u8; INFO_LOG_SIZE]) }
    }

    #[test]
    fn build_locations_numbers_vs_uniforms_first_and_merges_shared_names() {
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rvs, std::vec![uni("a", 0, 4, GL_FLOAT_VEC4, 1), uni("shared", 4, 2, GL_FLOAT_VEC2, 1)]);
        set_table(
            &mut p.rfs,
            std::vec![uni("b", 0, 1, GL_FLOAT, 1), uni("shared", 9, 2, GL_FLOAT_VEC2, 1), sampler("s", 2)],
        );
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        assert_eq!(tab.count, 4);
        let got: Vec<_> = tab.u[..4]
            .iter()
            .map(|l| (util::name_bytes(&location_ref(&p, l).name).to_vec(), l.vs_off, l.fs_off, l.fs_sampler))
            .collect();
        assert_eq!(
            got,
            [
                (b"a".to_vec(), 0, -1, false),
                (b"shared".to_vec(), 4, 9, false),
                (b"b".to_vec(), -1, 0, false),
                (b"s".to_vec(), -1, -1, true),
            ]
        );
        assert_eq!(location_ref(&p, &tab.u[3]).binding, 2);
    }

    #[test]
    fn build_locations_rejects_a_uniform_with_two_types() {
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rvs, std::vec![uni("u_x", 0, 4, GL_FLOAT_VEC4, 1)]);
        set_table(&mut p.rfs, std::vec![uni("u_x", 0, 4, GL_INT_VEC4, 1)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), -1);
        let text = String::from_utf8(log_text(&log).to_vec()).unwrap();
        assert!(text.contains("'u_x'") && text.contains("different types"), "{text}");
    }

    #[test]
    fn build_locations_rejects_a_uniform_with_two_sizes() {
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rvs, std::vec![uni("arr", 0, 4, GL_FLOAT, 4)]);
        set_table(&mut p.rfs, std::vec![uni("arr", 0, 8, GL_FLOAT, 8)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), -1);
        assert!(!log_text(&log).is_empty());
    }

    #[test]
    fn build_locations_limits_the_table_to_255_names() {
        let (mut p, mut tab, mut log) = blank();
        let names = |prefix: &str, n: usize| -> Vec<ShcUniform> {
            (0..n).map(|i| uni(&std::format!("{prefix}{i}"), i as u16, 1, GL_FLOAT, 1)).collect()
        };
        set_table(&mut p.rvs, names("v", 200));
        set_table(&mut p.rfs, names("f", 55));
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        assert_eq!(tab.count, 255);

        set_table(&mut p.rfs, names("f", 56));
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), -1);
        let text = String::from_utf8(log_text(&log).to_vec()).unwrap();
        assert!(text.contains("more than 255 uniforms"), "{text}");
    }

    #[test]
    fn build_locations_rejects_a_sampler_binding_beyond_the_sampler_count() {
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rfs, std::vec![sampler("ok", CAP_SAMPLERS as u8 - 1)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        set_table(&mut p.rfs, std::vec![sampler("far", CAP_SAMPLERS as u8)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), -1);
        let text = String::from_utf8(log_text(&log).to_vec()).unwrap();
        assert!(text.contains("'far'") && text.contains(&std::format!("binding {CAP_SAMPLERS}")), "{text}");
    }

    #[test]
    fn build_locations_resets_the_sampler_units() {
        let (mut p, mut tab, mut log) = blank();
        p.fs_sampler_unit = [5; CAP_SAMPLERS];
        set_table(&mut p.rfs, std::vec![sampler("s", 0)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        assert_eq!(p.fs_sampler_unit, [0; CAP_SAMPLERS]);
    }

    /* ---- the uniform budget (GLSL ES 1.00 Appendix A 7; check_uniform_packing) ---- */

    /// Run the uniform budget on two hand-built stages; the log text on a
    /// refusal.
    fn packing_check(vs: Vec<ShcUniform>, fs: Vec<ShcUniform>) -> Result<(), String> {
        let (mut p, _, mut log) = blank();
        set_table(&mut p.rvs, vs);
        set_table(&mut p.rfs, fs);
        check_uniform_packing(&p.rvs, &p.rfs, &mut log)
            .map_err(|()| String::from_utf8(log_text(&log).to_vec()).unwrap())
    }

    /// `rows` vec4s in rows of at most 255 elements (`array_len` is a byte).
    fn vec4_rows(rows: usize) -> Vec<ShcUniform> {
        (0..rows.div_ceil(255))
            .map(|k| {
                let n = (rows - k * 255).min(255);
                uni(&std::format!("v{k}"), 0, (4 * n) as u16, GL_FLOAT_VEC4, n as u8)
            })
            .collect()
    }

    const VS_ROWS: usize = context::MAX_VERTEX_UNIFORM_VECTORS;
    const FS_ROWS: usize = context::MAX_FRAGMENT_UNIFORM_VECTORS;

    fn over(stage: &str, name: &str, rows: usize) -> String {
        std::format!("link error: the {stage} shader's uniforms do not fit {name} = {rows} (Appendix A packing)")
    }

    #[test]
    fn uniforms_that_fill_each_stage_exactly_link() {
        assert_eq!(packing_check(vec4_rows(VS_ROWS), vec4_rows(FS_ROWS)), Ok(()));
    }

    #[test]
    fn one_uniform_over_the_vertex_limit_fails_the_link() {
        let mut vs = vec4_rows(VS_ROWS);
        vs.push(uni("one_more", 0, 1, GL_FLOAT, 1));
        assert_eq!(packing_check(vs, std::vec![]), Err(over("vertex", "MAX_VERTEX_UNIFORM_VECTORS", VS_ROWS)));
    }

    #[test]
    fn one_uniform_over_the_fragment_limit_fails_the_link() {
        let mut fs = vec4_rows(FS_ROWS);
        fs.push(uni("one_more", 0, 2, GL_BOOL_VEC2, 1));
        assert_eq!(packing_check(std::vec![], fs), Err(over("fragment", "MAX_FRAGMENT_UNIFORM_VECTORS", FS_ROWS)));
    }

    /// Appendix A's mat3 rule: three columns of three rows each, the fourth
    /// column left to the floats. mat3[128] takes all 384 rows' x..z and two
    /// float arrays fill w; one float more has no cell.
    #[test]
    fn a_mat3_array_leaves_the_w_column_to_float_arrays() {
        assert_eq!(VS_ROWS, 384, "the vectors below are written for 384 rows");
        let mut vs = std::vec![
            uni("m", 0, 128 * 12, GL_FLOAT_MAT3, 128),
            uni("f", 0, 255, GL_FLOAT, 255),
            uni("g", 0, 129, GL_FLOAT, 129),
        ];
        assert_eq!(packing_check(vs.clone(), std::vec![]), Ok(()));
        vs.push(uni("h", 0, 1, GL_FLOAT, 1));
        assert_eq!(packing_check(vs, std::vec![]), Err(over("vertex", "MAX_VERTEX_UNIFORM_VECTORS", VS_ROWS)));
    }

    /// Appendix A: "Elements of an array must be in different rows", so the
    /// last row of a grid takes four floats but not a float[4]; a table's
    /// flattened struct-array member (`s[0].f`, `inner` 1 of 4) is four
    /// variables of one element, not one array of four.
    #[test]
    fn an_array_takes_a_row_per_element_and_a_flattened_member_does_not() {
        let mut vs = vec4_rows(VS_ROWS - 1);
        vs.push(uni("f", 0, 4, GL_FLOAT, 4));
        assert_eq!(packing_check(vs.clone(), std::vec![]), Err(over("vertex", "MAX_VERTEX_UNIFORM_VECTORS", VS_ROWS)));
        vs.pop();
        let mut member = uni("s[0].f", 0, 4, GL_FLOAT, 4);
        member.inner = 1;
        vs.push(member);
        assert_eq!(packing_check(vs, std::vec![]), Ok(()));
    }

    /// A sampler takes a texture unit, not uniform storage (counted by
    /// check_sampler_counts).
    #[test]
    fn samplers_take_no_uniform_rows() {
        let mut fs = vec4_rows(FS_ROWS);
        fs.push(samplers("s", 0, 4));
        assert_eq!(packing_check(std::vec![], fs), Ok(()));
    }

    /* ---- the sampler counts (ES 2.0.25 2.10.4, 2.10.5; check_sampler_counts) ---- */

    /// An array of `n` 2D samplers from `binding` on: `n` active samplers.
    fn samplers(name: &str, binding: u8, n: u8) -> ShcUniform {
        let mut u = uni(name, SHC_UNIFORM_NO_BLOCK, n as u16, GL_SAMPLER_2D, n);
        u.binding = binding;
        u
    }

    fn sampler_count_message(who: &str, used: usize, max: usize, cap: &str) -> String {
        std::format!("link error: {who} {used} active samplers; this implementation exposes {max} ({cap})")
    }

    /// Run the check on two hand-built stages; the log text on a refusal.
    fn sampler_check(vs: Vec<ShcUniform>, fs: Vec<ShcUniform>) -> Result<(), String> {
        sampler_check_against(&SAMPLER_LIMITS, vs, fs)
    }

    /// The same, against other limits.
    fn sampler_check_against(limits: &SamplerLimits, vs: Vec<ShcUniform>, fs: Vec<ShcUniform>) -> Result<(), String> {
        let (mut p, _, mut log) = blank();
        set_table(&mut p.rvs, vs);
        set_table(&mut p.rfs, fs);
        check_sampler_counts_against(&p.rvs, &p.rfs, limits, &mut log)
            .map_err(|()| String::from_utf8(log_text(&log).to_vec()).unwrap())
    }

    /// The limits with the combined one at 16, its value before it became
    /// 32: the only way to reach the combined rule (see
    /// [`check_sampler_counts`]).
    const COMBINED_16: SamplerLimits = SamplerLimits { combined: 16, ..SAMPLER_LIMITS };

    #[test]
    fn sampler_counts_accept_each_stage_at_its_limit() {
        use crate::backend::GLES_VS_TEX_UNITS as VS;
        assert_eq!(sampler_check(std::vec![samplers("v", 0, VS as u8)], std::vec![]), Ok(()));
        assert_eq!(sampler_check(std::vec![], std::vec![samplers("f", 0, CAP_TEXTURE_UNITS as u8)]), Ok(()));
        /* one sampler per row counts as one, like an array of one */
        let singles: Vec<ShcUniform> = (0..VS as u8).map(|i| sampler(&std::format!("v{i}"), i)).collect();
        assert_eq!(sampler_check(singles, std::vec![]), Ok(()));
    }

    /// Both stages at their own limits at once: 32 units on the V3D and the
    /// PowerVR and the GC7000, all within the combined limit (it was 16,
    /// and dEQP-GLES2 uniform_api's "18/20/28 combined units" cases were
    /// refused).
    #[test]
    fn sampler_counts_accept_both_stages_at_their_limits() {
        use crate::backend::GLES_VS_TEX_UNITS as VS;
        const { assert!(VS + CAP_TEXTURE_UNITS <= CAP_COMBINED_TEXTURE_UNITS) };
        assert_eq!(
            sampler_check(std::vec![samplers("v", 0, VS as u8)], std::vec![samplers("f", 0, CAP_TEXTURE_UNITS as u8)]),
            Ok(())
        );
    }

    #[test]
    fn sampler_counts_refuse_a_vertex_shader_over_max_vertex_texture_image_units() {
        use crate::backend::GLES_VS_TEX_UNITS as VS;
        assert_eq!(
            sampler_check(std::vec![samplers("v", 0, VS as u8 + 1)], std::vec![]),
            Err(sampler_count_message("the vertex shader uses", VS + 1, VS, "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS"))
        );
    }

    /// The GC7000 with sixteen vertex units: the link takes 9..16 vertex
    /// samplers, alone and beside sixteen fragment ones, and refuses the
    /// 17th with the limit it reports.
    #[test]
    #[cfg(feature = "gpu-vivante")]
    fn sampler_counts_take_sixteen_gc7000_vertex_samplers() {
        const V: &str = "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS";
        let results: Vec<_> = (9..=17u8).map(|n| sampler_check(std::vec![samplers("v", 0, n)], std::vec![])).collect();
        let both = sampler_check(std::vec![samplers("v", 0, 16)], std::vec![samplers("f", 0, CAP_TEXTURE_UNITS as u8)]);
        for (n, r) in (9..=16).zip(&results) {
            assert_eq!(*r, Ok(()), "{n} vertex samplers");
        }
        assert_eq!(results[8], Err(sampler_count_message("the vertex shader uses", 17, 16, V)));
        assert_eq!(both, Ok(()), "16 + 16 is within the combined 32");
    }

    #[test]
    fn sampler_counts_refuse_a_fragment_shader_over_max_texture_image_units() {
        let n = CAP_TEXTURE_UNITS + 1;
        /* two rows: the online compiler refuses more than 16 bindings in one stage, a table need not */
        let fs = std::vec![samplers("a", 0, 8), samplers("b", 8, (n - 8) as u8)];
        assert_eq!(
            sampler_check(std::vec![], fs),
            Err(sampler_count_message("the fragment shader uses", n, CAP_TEXTURE_UNITS, "GL_MAX_TEXTURE_IMAGE_UNITS"))
        );
    }

    #[test]
    fn sampler_counts_refuse_two_stages_over_max_combined_texture_image_units() {
        use crate::backend::GLES_VS_TEX_UNITS as VS;
        const C: usize = COMBINED_16.combined;
        /* each stage within its own limit, the sum one over the combined one */
        let v = VS.min(C);
        let f = C + 1 - v;
        assert!(f <= CAP_TEXTURE_UNITS);
        /* at the limit; a row of zero elements would still count one (V3D: v = 16, f - 1 = 0) */
        let at_limit = if f > 1 { std::vec![samplers("f", 0, (f - 1) as u8)] } else { std::vec![] };
        assert_eq!(sampler_check_against(&COMBINED_16, std::vec![samplers("v", 0, v as u8)], at_limit), Ok(()));
        assert_eq!(
            sampler_check_against(
                &COMBINED_16,
                std::vec![samplers("v", 0, v as u8)],
                std::vec![samplers("f", 0, f as u8)]
            ),
            Err(sampler_count_message(
                "the vertex and fragment shaders use",
                C + 1,
                C,
                "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS"
            ))
        );
    }

    /// ES 2.0.25 2.10.5: a unit both stages access counts as two against the
    /// combined limit, so a sampler both stages declare and use counts once
    /// per stage.
    #[test]
    fn sampler_counts_count_a_sampler_both_stages_use_once_per_stage() {
        use crate::backend::GLES_VS_TEX_UNITS as VS;
        const C: usize = COMBINED_16.combined;
        let a = VS.min(C);
        let b = (C + 1).saturating_sub(2 * a);
        let mut fs = std::vec![samplers("s", 0, a as u8)];
        if b > 0 {
            fs.push(samplers("t", a as u8, b as u8));
        }
        assert!(a + b <= C, "counted once, the program would fit: {a} + {b}");
        assert_eq!(
            sampler_check_against(&COMBINED_16, std::vec![samplers("s", 0, a as u8)], fs),
            Err(sampler_count_message(
                "the vertex and fragment shaders use",
                2 * a + b,
                C,
                "GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS"
            ))
        );
    }

    /// Through glLinkProgram: the status stays false, the message is the
    /// program's info log, and no GL error is recorded (a failed link is no
    /// GL error).
    /// End to end through glLinkProgram; the shaders come from the offline
    /// table, which a `no-shader-table` build leaves out (the unit tests
    /// above cover that build through the fake reflection tables).
    #[cfg(not(feature = "no-shader-table"))]
    #[test]
    fn link_refuses_a_vertex_shader_over_max_vertex_texture_image_units() {
        use crate::backend::GLES_VS_TEX_UNITS as VS;
        let gl = fresh();
        let (p, v, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        let c = gl.ctx();
        set_table(&mut c.shader(v).unwrap().compiled_shader, std::vec![samplers("v", 0, VS as u8 + 1)]);
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 0);
        assert_eq!(
            info_log(p),
            sampler_count_message("the vertex shader uses", VS + 1, VS, "GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS")
        );
        assert_eq!(gl.error(), GL_NO_ERROR);

        set_table(&mut c.shader(v).unwrap().compiled_shader, std::vec![samplers("v", 0, VS as u8)]);
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
    }

    /// A mat<N> attribute as the shader front end reflects it: N rows, one
    /// per column, every one carrying the matrix's own name and gl_type.
    fn matrix_attrib(p: &mut GlProgram, first: u8, columns: u8, name: &[u8], gl_type: u16) {
        for c in 0..columns {
            let i = (p.rvs.num_attribs + c) as usize;
            p.rvs.attribs[i].location = first + c;
            p.rvs.attribs[i].components = 4;
            p.rvs.attribs[i].gl_type = gl_type;
            util::set_name(&mut p.rvs.attribs[i].name, name);
        }
        p.rvs.num_attribs += columns;
    }

    #[test]
    fn a_matrix_attributes_columns_are_one_attribute() {
        /* GL ES 2.0 2.10.4: a mat4 attribute is ONE active attribute at the
         * first of the four locations it occupies. The reflection has four
         * rows because the front end splits the matrix so pco_shc never sees
         * a matrix input. */
        let (mut p, _, mut log) = blank();
        p.rvs.attribs[0].location = 0;
        util::set_name(&mut p.rvs.attribs[0].name, b"pos");
        p.rvs.num_attribs = 1;
        matrix_attrib(&mut p, 1, 4, b"xf", GL_FLOAT_MAT4 as u16);
        assert_eq!(build_attrib_indices(&mut p, &mut log), 0);
        let whole: std::vec::Vec<usize> = (0..p.rvs.num_attribs as usize).filter(|&i| attrib_is_whole(&p, i)).collect();
        assert_eq!(whole, std::vec![0, 1], "pos and the matrix's first column, not its other three");
        assert_eq!(attrib_base_location(&p, &p.rvs.attribs[1]), 1);
        assert_eq!((0..4).map(|c| attrib_column(&p, 1 + c)).collect::<std::vec::Vec<u8>>(), std::vec![0, 1, 2, 3]);
    }

    #[test]
    fn binding_a_matrix_attribute_moves_all_of_its_columns() {
        /* glBindAttribLocation("xf", 5) puts the mat4's columns at 5..8. The
         * columns share a name, so giving each of them the bound index would
         * make them collide with one another and fail the link. */
        let (mut p, _, mut log) = blank();
        p.rvs.attribs[0].location = 0;
        util::set_name(&mut p.rvs.attribs[0].name, b"pos");
        p.rvs.num_attribs = 1;
        matrix_attrib(&mut p, 1, 4, b"xf", GL_FLOAT_MAT4 as u16);
        util::set_name(&mut p.attrib_binds[0].name, b"xf");
        p.attrib_binds[0].index = 5;
        p.num_attrib_binds = 1;
        assert_eq!(build_attrib_indices(&mut p, &mut log), 0, "{:?}", log_text(&log));
        assert_eq!(&p.attrib_app_index[1..5], &[5, 6, 7, 8]);
        assert_eq!(p.attrib_app_index[0], 0, "the other attribute is where it was");
    }

    #[test]
    fn binding_a_matrix_attribute_past_the_inputs_fails_the_link() {
        /* A mat4 bound to index 14 would need 14..17, and there are 16. */
        let (mut p, _, mut log) = blank();
        matrix_attrib(&mut p, 0, 4, b"xf", GL_FLOAT_MAT4 as u16);
        util::set_name(&mut p.attrib_binds[0].name, b"xf");
        p.attrib_binds[0].index = MAX_VERTEX_ATTRIBS as u8 - 2;
        p.num_attrib_binds = 1;
        assert_eq!(build_attrib_indices(&mut p, &mut log), -1);
        let text = String::from_utf8(log_text(&log).to_vec()).unwrap();
        assert!(text.contains("'xf'") && text.contains("beyond"), "{text}");
    }

    #[test]
    fn two_long_names_that_truncate_alike_are_still_two_attributes() {
        /* shc_attrib.name is 32 bytes, so these two differ only past its end
         * and read back as the same name. Grouping the columns of a matrix by
         * name alone would make the second a column of the first: one active
         * attribute instead of two, and a binding of that name would move
         * both. Only a matrix has columns, so the gl_type settles it. */
        let (mut p, _, mut log) = blank();
        let name = b"an_attribute_name_of_full_length";
        assert_eq!(name.len(), 32, "the name has to fill the field for this to bite");
        p.rvs.num_attribs = 2;
        for (i, a) in p.rvs.attribs.iter_mut().enumerate().take(2) {
            a.location = i as u8;
            a.components = 4;
            a.gl_type = GL_FLOAT_VEC4 as u16;
            util::set_name(&mut a.name, name);
        }
        assert_eq!(build_attrib_indices(&mut p, &mut log), 0);
        assert_eq!(&p.attrib_app_index[..2], &[0, 1]);
        assert!(attrib_is_whole(&p, 0) && attrib_is_whole(&p, 1), "neither is a column of the other");
    }

    #[test]
    fn build_attrib_indices_defaults_to_the_glsl_location() {
        let (mut p, _, mut log) = blank();
        p.rvs.num_attribs = 2;
        p.rvs.attribs[0].location = 0;
        p.rvs.attribs[1].location = 3;
        util::set_name(&mut p.rvs.attribs[0].name, b"a");
        util::set_name(&mut p.rvs.attribs[1].name, b"b");
        assert_eq!(build_attrib_indices(&mut p, &mut log), 0);
        let identity: [u8; 16] = core::array::from_fn(|i| i as u8);
        assert_eq!(p.attrib_app_index, identity);
    }

    #[test]
    fn build_attrib_indices_applies_binds_and_allows_aliasing() {
        let (mut p, _, mut log) = blank();
        p.rvs.num_attribs = 2;
        p.rvs.attribs[0].location = 0;
        p.rvs.attribs[1].location = 1;
        util::set_name(&mut p.rvs.attribs[0].name, b"a");
        util::set_name(&mut p.rvs.attribs[1].name, b"b");
        util::set_name(&mut p.attrib_binds[0].name, b"b");
        p.attrib_binds[0].index = 7;
        util::set_name(&mut p.attrib_binds[1].name, b"unused");
        p.attrib_binds[1].index = 0;
        p.num_attrib_binds = 2;
        assert_eq!(build_attrib_indices(&mut p, &mut log), 0);
        assert_eq!(&p.attrib_app_index[..2], &[0, 7]);

        /* `b` onto 0, where `a` already is: aliasing, which the GL allows
         * (ES 2.0 2.10.4) and which this used to refuse. Both inputs then
         * read vertex attribute 0, and the draw path stages each separately.
         * dEQP's `bind_aliasing.cond_*` is this case. */
        p.attrib_binds[0].index = 0;
        assert_eq!(build_attrib_indices(&mut p, &mut log), 0, "aliasing links");
        assert_eq!(&p.attrib_app_index[..2], &[0, 0], "both inputs read attribute 0");
        assert_eq!(log_text(&log), b"", "and a link that succeeds writes no error");
    }

    #[test]
    fn build_attrib_indices_accepts_location_15() {
        let (mut p, _, mut log) = blank();
        p.rvs.num_attribs = 2;
        p.rvs.attribs[0].location = 15;
        p.rvs.attribs[1].location = 0;
        util::set_name(&mut p.rvs.attribs[0].name, b"a");
        util::set_name(&mut p.rvs.attribs[1].name, b"b");
        util::set_name(&mut p.attrib_binds[0].name, b"a");
        p.attrib_binds[0].index = 3;
        p.num_attrib_binds = 1;
        assert_eq!(build_attrib_indices(&mut p, &mut log), 0);
        assert_eq!((p.attrib_app_index[15], p.attrib_app_index[0]), (3, 0));
    }

    #[test]
    fn build_attrib_indices_rejects_locations_beyond_the_attribute_table() {
        let (mut p, _, mut log) = blank();
        p.rvs.num_attribs = 2;
        p.rvs.attribs[0].location = 16;
        p.rvs.attribs[1].location = 3;
        util::set_name(&mut p.rvs.attribs[0].name, b"a");
        util::set_name(&mut p.rvs.attribs[1].name, b"b");
        util::set_name(&mut p.attrib_binds[0].name, b"\x03"); /* what the emulated read finds for location 16 */
        p.num_attrib_binds = 1;
        assert_eq!(build_attrib_indices(&mut p, &mut log), -1, "location 16 has no attribute index");
        let text = String::from_utf8(log_text(&log).to_vec()).unwrap();
        assert!(text.contains("location") && !text.contains("same index"), "{text}");
    }

    /* ---- task #46: struct and array-of-struct uniforms, from the spec side ----
     *
     * These say what GL answers for
     *
     *     struct Light { highp vec3 color; highp float atten[3]; };
     *     uniform Light u_lights[3];
     *
     * They are written from dEQP's expectations (es2fUniformApiTests.cpp,
     * `UniformCase::generateBasicUniforms`), not from the reflection the ES
     * 1.00 front end happens to produce, so that the reflection shape has to
     * meet them rather than the other way round. If that shape changes, the
     * ONE helper below changes and none of the assertions do.
     */

    /// The reflection task #46's contract describes: one flattened array per
    /// struct member, named for element zero, `array_len` counting every leaf
    /// and `inner` the stride within one outer index.
    fn light_array_reflection() -> Vec<ShcUniform> {
        /* dwords is the member's TOTAL, as the lit material's vec3[16] at 64
         * dwords shows: a vec3 element is padded to four, a float packed to
         * one. So color is 3 x 4 = 12 dwords at 0 and atten 9 x 1 = 9 at 12. */
        let mut color = uni("u_lights[0].color", 0, 12, GL_FLOAT_VEC3, 3);
        color.inner = 1;
        let mut atten = uni("u_lights[0].atten", 12, 9, GL_FLOAT, 9);
        atten.inner = 3;
        std::vec![color, atten]
    }

    /// `(name, size, type)` of every active uniform, in no particular order:
    /// GL does not fix the order, so a test that depended on it would be
    /// testing this implementation rather than the specification.
    fn active_uniforms(prog: GLuint) -> Vec<(String, GLint, GLenum)> {
        let mut out = Vec::new();
        for i in 0..get_iv(prog, GL_ACTIVE_UNIFORMS) as GLuint {
            let mut name = [0u8; 64];
            let (mut len, mut size, mut ty): (GLsizei, GLint, GLenum) = (0, 0, 0);
            // SAFETY: name holds the announced bytes.
            unsafe {
                glGetActiveUniform(prog, i, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr() as *mut GLchar);
            }
            out.push((String::from_utf8_lossy(&name[..len as usize]).into_owned(), size, ty));
        }
        out.sort();
        out
    }

    /* The three tests on `light_array_reflection` hand the GL core the
     * OFFLINE table's reflection shape (one flattened row per struct member)
     * on a program whose stages the build's back end compiled. build_locations
     * flattens only rows of an offline-compiled stage (`!shaderc::is_online`),
     * so they run on `fake_offline_program`, which marks the stages offline
     * (`testutil::mark_stage_offline`): a `no-shader-table` build has no
     * offline stage of its own. The online shape of the same dEQP contract
     * is pinned by
     * `nested_struct_array_uniforms_reach_the_shader_as_deqp_sets_them`. */
    #[test]
    fn get_active_uniform_enumerates_one_entry_per_struct_array_element() {
        /* dEQP recurses per outer index for an array of structs and collapses
         * to a single "name[0]" entry ONLY where the element type is basic.
         * So `u_lights[3]` of a struct is six active uniforms, not two, and
         * `atten` reports the size of the INNER array (3), not of the nine
         * leaves the reflection flattens it into. */
        let gl = fresh();
        let p = fake_offline_program(&gl, light_array_reflection(), std::vec![]);
        let want: Vec<(String, GLint, GLenum)> = std::vec![
            ("u_lights[0].atten[0]".into(), 3, GL_FLOAT),
            ("u_lights[0].color".into(), 1, GL_FLOAT_VEC3),
            ("u_lights[1].atten[0]".into(), 3, GL_FLOAT),
            ("u_lights[1].color".into(), 1, GL_FLOAT_VEC3),
            ("u_lights[2].atten[0]".into(), 3, GL_FLOAT),
            ("u_lights[2].color".into(), 1, GL_FLOAT_VEC3),
        ];
        assert_eq!(get_iv(p, GL_ACTIVE_UNIFORMS), 6);
        assert_eq!(active_uniforms(p), want);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn active_uniform_max_length_spans_the_indexed_struct_names() {
        /* the longest is "u_lights[0].atten[0]", 20 characters, and GL counts
         * the NUL */
        let gl = fresh();
        let p = fake_program(&gl, light_array_reflection(), std::vec![]);
        let longest = active_uniforms(p).iter().map(|(n, _, _)| n.len()).max().unwrap();
        assert_eq!(longest, 20);
        assert_eq!(get_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 21);
    }

    /// ES 3.0 section 2.12.6: a name with the reserved prefix `gl_` has no
    /// location, even when the program carries a row of that name (the
    /// online compiler's gl_DepthRange rows, which the GL core fills).
    ///
    /// Revert check: drop the prefix test in glGetUniformLocation and the
    /// row answers a location.
    #[test]
    fn a_gl_prefixed_name_has_no_location() {
        let gl = fresh();
        let p = fake_program(
            &gl,
            std::vec![],
            std::vec![uni("k", 0, 1, GL_FLOAT, 1), uni("gl_DepthRange.near", 1, 1, GL_FLOAT, 1)],
        );
        assert!(loc(p, "k") >= 0);
        assert_eq!(loc(p, "gl_DepthRange.near"), -1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn struct_array_uniform_locations_follow_the_gl_naming_rules() {
        let gl = fresh();
        let p = fake_offline_program(&gl, light_array_reflection(), std::vec![]);

        /* every element of the struct array has its own location */
        let l0 = loc(p, "u_lights[0].color");
        let l1 = loc(p, "u_lights[1].color");
        let l2 = loc(p, "u_lights[2].color");
        assert!(l0 >= 0 && l1 >= 0 && l2 >= 0, "{l0} {l1} {l2}");
        assert!(l0 != l1 && l1 != l2 && l0 != l2, "each element is its own uniform");

        /* ... and so does every element of a member's own array */
        assert!(loc(p, "u_lights[2].atten[1]") >= 0);
        assert_ne!(loc(p, "u_lights[2].atten[1]"), loc(p, "u_lights[2].atten[0]"));

        /* "[0]" of an array names the array itself (GL ES 2.0 2.10.4) */
        assert_eq!(loc(p, "u_lights[0].atten[0]"), loc(p, "u_lights[0].atten"));

        /* the struct array itself is not an active uniform, with or without
         * an index: only its leaves are */
        assert_eq!(loc(p, "u_lights"), -1, "the array of structs is not a uniform");
        assert_eq!(loc(p, "u_lights[0]"), -1, "nor is one of its structs");
        assert_eq!(loc(p, "u_lights[3].color"), -1, "past the end");
        assert_eq!(loc(p, "u_lights[0].atten[3]"), -1, "past the inner array");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_write_to_a_struct_members_array_stops_at_that_array() {
        /* GL ES 2.0 2.10.4: a count that runs past the end of the array is
         * clamped to the elements that remain -- and the array that bounds it
         * is `atten`'s own three, not the nine leaves the flattening makes of
         * it, so light 0's write must not spill into light 1's. */
        let gl = fresh();
        let p = fake_offline_program(&gl, light_array_reflection(), std::vec![]);
        let v = [7.0f32; 8];
        // SAFETY: v holds the eight elements announced.
        unsafe { glUniform1fv(loc(p, "u_lights[0].atten[1]"), 8, v.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);

        let block = vs_bits(&gl, p, 12, 9);
        assert_eq!(&block[1..3], &[7.0f32.to_bits(); 2], "atten[1] and atten[2] of light 0");
        assert_eq!(block[0], SENTINEL, "atten[0] was not in the range");
        assert_eq!(&block[3..9], &[SENTINEL; 6], "lights 1 and 2 are untouched");
    }

    /* ---- program objects: create / delete / is ---- */

    #[test]
    fn create_program_returns_distinct_nonzero_handles() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let (a, b) = (glCreateProgram(), glCreateProgram());
            assert!(a != 0 && b != 0 && a != b);
            assert_eq!(glIsProgram(a), GL_TRUE_B);
            assert_eq!(glIsProgram(0), GL_FALSE_B);
            assert_eq!(glIsProgram(b + 1), GL_FALSE_B);
            assert_eq!(glIsProgram(MAX_PROGRAMS as GLuint), GL_FALSE_B);
            assert_eq!(glIsProgram(GLuint::MAX), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn new_program_state() {
        let gl = fresh();
        // SAFETY: plain GL call.
        let p = unsafe { glCreateProgram() };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 0);
        assert_eq!(get_iv(p, GL_VALIDATE_STATUS), 0);
        assert_eq!(get_iv(p, GL_DELETE_STATUS), 0);
        assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 0);
        assert_eq!(get_iv(p, GL_ACTIVE_UNIFORMS), 0);
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 0);
        assert_eq!(get_iv(p, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(info_log(p), "");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn create_program_out_of_handles_is_out_of_memory() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let handles: Vec<GLuint> = (1..MAX_PROGRAMS).map(|_| glCreateProgram()).collect();
            assert!(handles.iter().all(|h| *h != 0));
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glCreateProgram(), 0);
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            glDeleteProgram(handles[5]);
            assert_eq!(glCreateProgram(), handles[5]);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn create_program_without_a_current_context_returns_zero() {
        let gl = fresh();
        // SAFETY: the context is made current again before any other GL call.
        unsafe {
            context::gl_context_set_current(core::ptr::null_mut());
            let p = glCreateProgram();
            context::gl_context_set_current(gl.ctx());
            assert_eq!(p, 0);
        }
    }

    #[test]
    fn delete_program_zero_is_ignored_and_unknown_is_invalid_value() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteProgram(0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteProgram(3);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDeleteProgram(GLuint::MAX);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            let p = glCreateProgram();
            glDeleteProgram(p);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glIsProgram(p), GL_FALSE_B);
            glDeleteProgram(p);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn deleted_program_handle_is_reused_with_clean_state() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "texture.frag");
        bind_attrib(p, 4, "a_color");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p); /* fails: v_uv is not written */
            assert!(get_iv(p, GL_INFO_LOG_LENGTH) > 0);
            glDeleteProgram(p);
            assert_eq!(glCreateProgram(), p);
        }
        assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 0);
        assert_eq!(get_iv(p, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(get_iv(p, GL_LINK_STATUS), 0);
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].num_attrib_binds, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn delete_of_the_current_program_leaves_no_dangling_binding() {
        let gl = fresh();
        let p = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glDeleteProgram(p);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let c = gl.ctx();
            let cur = c.cur_program;
            assert!(cur == 0 || c.objs().programs[obj_index(cur)].used);
            glUseProgram(0);
            assert_eq!(glIsProgram(p), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn delete_of_the_current_program_is_deferred() {
        let gl = fresh();
        let p = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glDeleteProgram(p);
            assert_eq!(glIsProgram(p), GL_TRUE_B);
            assert_eq!(get_iv(p, GL_DELETE_STATUS), 1);
            glUniform4f(loc(p, "u_color"), 1.0, 2.0, 3.0, 4.0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glUseProgram(0);
            assert_eq!(glIsProgram(p), GL_FALSE_B);
        }
    }

    #[test]
    fn delete_program_releases_its_gpu_allocations() {
        let gl = fresh();
        let p = gl.program("passthrough.vert", "uniform_color.frag");
        let verts: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        // SAFETY: plain GL calls; the client vertex array outlives the draw.
        unsafe {
            use crate::gl::{draw::glDrawArrays, vertex::*};
            glUseProgram(p);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let program_heap: Vec<u64> = gl
            .mem_events()
            .iter()
            .filter_map(|e| match e {
                MemEvent::Alloc { heap, gpu, .. } if *heap != crate::testutil::HEAP_DATA => Some(*gpu),
                _ => None,
            })
            .collect();
        assert!(!program_heap.is_empty(), "the draw uploaded the program's USC");
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteProgram(p);
            glUseProgram(0); /* the current program goes when it stops being current */
        }
        let freed: Vec<u64> = gl
            .mem_events()
            .iter()
            .filter_map(|e| match e {
                MemEvent::Free { gpu } => Some(*gpu),
                _ => None,
            })
            .collect();
        for gpu in program_heap {
            assert!(freed.contains(&gpu), "USC allocation {gpu:#x} is freed with the program");
        }
    }

    /* ---- attach / detach ---- */

    #[test]
    fn attach_and_detach_track_the_attached_shaders() {
        let gl = fresh();
        let (p, v, f) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 2);
        // SAFETY: plain GL calls.
        unsafe {
            glDetachShader(p, v);
            assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 1);
            glDetachShader(p, f);
            assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 0);
            glAttachShader(p, f);
            assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 1);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attach_needs_no_compiled_shader() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            let s = glCreateShader(GL_VERTEX_SHADER);
            glAttachShader(p, s);
            assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 1);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attach_twice_or_a_second_shader_of_a_stage_is_invalid_operation() {
        let gl = fresh();
        let (p, v, f) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        let v2 = gl.shader(GL_VERTEX_SHADER, "uv.vert");
        // SAFETY: plain GL calls.
        unsafe {
            glAttachShader(p, v);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "already attached");
            glAttachShader(p, v2);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "GL ES: one shader per stage");
            glAttachShader(p, f);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        let c = gl.ctx();
        assert_eq!((c.objs().programs[obj_index(p)].vs, c.objs().programs[obj_index(p)].fs), (v, f));
    }

    #[test]
    fn a_shader_can_be_attached_to_several_programs() {
        let gl = fresh();
        let (p, v, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            let q = glCreateProgram();
            glAttachShader(q, v);
            assert_eq!(get_iv(q, GL_ATTACHED_SHADERS), 1);
        }
        assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attach_and_detach_reject_unknown_names() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            let s = glCreateShader(GL_VERTEX_SHADER);
            glAttachShader(p, 99);
            assert_ne!(gl.error(), GL_NO_ERROR);
            glAttachShader(40, s);
            assert_ne!(gl.error(), GL_NO_ERROR);
            glDetachShader(40, s);
            assert_ne!(gl.error(), GL_NO_ERROR);
            assert_eq!(get_iv(p, GL_ATTACHED_SHADERS), 0);
        }
    }

    #[test]
    fn detach_of_a_shader_that_is_not_attached_is_invalid_operation() {
        let gl = fresh();
        let (p, v, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        let other = gl.shader(GL_VERTEX_SHADER, "uv.vert");
        // SAFETY: plain GL calls.
        unsafe {
            glDetachShader(p, other);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glDetachShader(p, v);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDetachShader(p, v);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "already detached");
        }
    }

    #[test]
    fn detach_of_shader_zero_is_an_error() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glDetachShader(p, 0);
        }
        assert_ne!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn detach_keeps_a_linked_program_linked() {
        let gl = fresh();
        let (p, v, f) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p);
            glDetachShader(p, v);
            glDetachShader(p, f);
        }
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "the executable is independent of the attachments");
        assert_eq!(loc(p, "u_color"), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_attached_shaders_lists_vs_then_fs() {
        let gl = fresh();
        let (p, v, f) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: the output arrays hold maxCount handles.
        unsafe {
            let mut out = [0xffu32; 4];
            let mut n: GLsizei = -1;
            glGetAttachedShaders(p, 4, &mut n, out.as_mut_ptr());
            assert_eq!((n, out), (2, [v, f, 0xff, 0xff]));

            let mut out = [0xffu32; 4];
            glGetAttachedShaders(p, 1, &mut n, out.as_mut_ptr());
            assert_eq!((n, out), (1, [v, 0xff, 0xff, 0xff]), "maxCount limits what is written");

            glGetAttachedShaders(p, 0, &mut n, out.as_mut_ptr());
            assert_eq!(n, 0);

            glDetachShader(p, v);
            let mut out = [0xffu32; 4];
            glGetAttachedShaders(p, 4, &mut n, out.as_mut_ptr());
            assert_eq!((n, out), (1, [f, 0xff, 0xff, 0xff]));

            /* a null count is fine */
            glGetAttachedShaders(p, 4, core::ptr::null_mut(), out.as_mut_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn get_attached_shaders_errors() {
        let gl = fresh();
        // SAFETY: nothing may be written on error.
        unsafe {
            let p = glCreateProgram();
            let mut out = [0xffu32; 2];
            let mut n: GLsizei = -1;
            glGetAttachedShaders(p, -1, &mut n, out.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetAttachedShaders(p + 1, 2, &mut n, out.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert_eq!((n, out), (-1, [0xff; 2]));
        }
    }

    /* ---- link ---- */

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn link_success_sets_status_and_an_empty_log() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "main.vert", "fill_color.frag");
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        /* GL: the link does not validate. GL_VALIDATE_STATUS is what the last
         * glValidateProgram decided, and there has not been one. */
        assert_eq!(get_iv(p, GL_VALIDATE_STATUS), 0);
        assert_eq!(get_iv(p, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(get_iv(p, GL_ACTIVE_UNIFORMS), 2);
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn link_without_both_compiled_stages_fails_with_a_log() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glLinkProgram(p);
            assert_eq!(get_iv(p, GL_LINK_STATUS), 0, "nothing attached");
            let log = info_log(p);
            assert!(log.contains("vertex and fragment shaders must be attached and compiled"), "{log}");
            assert_eq!(get_iv(p, GL_INFO_LOG_LENGTH), log.len() as GLint + 1);

            glAttachShader(p, gl.shader(GL_VERTEX_SHADER, "passthrough.vert"));
            glLinkProgram(p);
            assert_eq!(get_iv(p, GL_LINK_STATUS), 0, "no fragment shader");

            let f = glCreateShader(GL_FRAGMENT_SHADER);
            /* not GLSL: the offline table lacks it and the online compiler
             * refuses it, so no back end gives a linkable fragment stage */
            let strings = [c"void main() { not glsl }".as_ptr()];
            glShaderSource(f, 1, strings.as_ptr(), core::ptr::null());
            glCompileShader(f);
            glAttachShader(p, f);
            glLinkProgram(p);
            assert_eq!(get_iv(p, GL_LINK_STATUS), 0, "fragment shader failed to compile");
            assert!(get_iv(p, GL_INFO_LOG_LENGTH) > 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "a link failure is not a GL error");
    }

    #[test]
    fn link_fails_when_the_vs_does_not_write_an_fs_varying() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "texture.frag");
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 0);
        /* the vertex shader does not declare v_uv at all; each back end says
         * so in its own words (the online one: src/shaderc/online/v3d_abi.rs,
         * backend.rs `a_varying_the_vertex_shader_never_declares_is_not_declared`) */
        let want = if cfg!(feature = "no-shader-table") {
            "fragment varying 'v_uv' is not declared in the vertex shader"
        } else {
            "'v_uv' is not written by the vertex shader"
        };
        let log = info_log(p);
        assert!(log.contains(want), "{log}");
        assert_eq!(get_iv(p, GL_ACTIVE_UNIFORMS), 0);
        assert_eq!(get_iv(p, GL_ACTIVE_ATTRIBUTES), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn link_patches_the_fs_inputs_to_the_vs_outputs_by_name() {
        let gl = fresh();
        let p = gl.program("sprite.vert", "sprite.frag");
        let c = gl.ctx();
        let pr = &c.objs_ref().programs[obj_index(p)];
        for fin in &pr.rfs.varyings[..pr.rfs.num_varyings as usize] {
            let out = pr.rvs.varyings[..pr.rvs.num_varyings as usize]
                .iter()
                .find(|o| util::name_bytes(&o.name) == util::name_bytes(&fin.name))
                .unwrap();
            assert_eq!(fin.vtxout_off, out.vtxout_off);
        }
    }

    #[test]
    fn link_snapshots_the_shaders_so_later_source_changes_do_not_matter() {
        let gl = fresh();
        let (p, _, f) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p);
            let strings = [c"garbage".as_ptr()];
            glShaderSource(f, 1, strings.as_ptr(), core::ptr::null());
            glCompileShader(f);
        }
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        assert_eq!(loc(p, "u_color"), 0);
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 0, "the relink sees the broken shader");
    }

    #[test]
    fn link_of_an_unknown_program_is_an_error() {
        let gl = fresh();
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(7) };
        assert_ne!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn relink_resets_uniform_values_and_sampler_units() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "uv.vert", "texture2.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p);
            glUseProgram(p);
            glUniform1i(loc(p, "u_tex1"), 5);
            assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_sampler_unit[1], 5);
            glLinkProgram(p);
        }
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_sampler_unit, [0; CAP_SAMPLERS]);

        let (q, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(q);
            glUseProgram(q);
            glUniform4f(loc(q, "u_color"), 1.0, 2.0, 3.0, 4.0);
            assert_eq!(fs_bits(&gl, q, 0, 4), fbits(&[1.0, 2.0, 3.0, 4.0]));
            glLinkProgram(q);
        }
        assert_eq!(fs_bits(&gl, q, 0, 4), [0; 4], "GL: a successful link initialises every uniform to 0");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn failed_relink_invalidates_the_locations() {
        let gl = fresh();
        let (p, _, f) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p);
            glUseProgram(p);
            let l = loc(p, "u_color");
            assert_eq!(l, 0);
            glDetachShader(p, f);
            glLinkProgram(p);
            assert_eq!(get_iv(p, GL_LINK_STATUS), 0);
            assert_eq!(get_iv(p, GL_ACTIVE_UNIFORMS), 0);
            assert_eq!(gl.error(), GL_NO_ERROR);

            assert_eq!(loc(p, "u_color"), -1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "glGetUniformLocation on an unlinked program");
            glUniform4f(l, 1.0, 1.0, 1.0, 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the old location is dead");
            let mut out = [0f32; 4];
            glGetUniformfv(p, l, out.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn relink_with_other_shaders_renumbers_the_locations() {
        let gl = fresh();
        let (p, v, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p);
            assert_eq!(loc(p, "u_color"), 0);
            glDetachShader(p, v);
            glAttachShader(p, gl.shader(GL_VERTEX_SHADER, "mvp.vert"));
            glLinkProgram(p);
        }
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        assert_eq!(loc(p, "u_mvp"), 0, "vertex-stage uniforms come first");
        assert_eq!(loc(p, "u_color"), 1);
        assert_eq!(get_iv(p, GL_ACTIVE_UNIFORMS), 2);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn uniform_after_handle_reuse_is_not_mistaken_for_a_sampler() {
        let gl = fresh();
        let textured = gl.program("uv.vert", "texture.frag"); /* u_tex: location 0, a sampler */
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteProgram(textured);
            let colored = gl.program("passthrough.vert", "uniform_color.frag");
            assert_eq!(colored, textured, "the handle is reused");
            glUseProgram(colored);
            glUniform4f(loc(colored, "u_color"), 0.25, 0.5, 0.75, 1.0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(fs_bits(&gl, colored, 0, 4), fbits(&[0.25, 0.5, 0.75, 1.0]));
        }
    }

    #[test]
    fn build_locations_clears_the_sampler_flag_of_a_reused_slot() {
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rfs, std::vec![sampler("s", 0)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        assert!(tab.u[0].fs_sampler);
        set_table(&mut p.rfs, std::vec![uni("f", 0, 1, GL_FLOAT, 1)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        assert!(!tab.u[0].fs_sampler);
    }

    /* ---- glUseProgram ---- */

    #[test]
    fn use_program_selects_the_target_of_the_uniform_setters() {
        let gl = fresh();
        let a = gl.program("passthrough.vert", "uniform_color.frag");
        let b = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(a);
            assert_eq!(gl.ctx().cur_program, a);
            glUniform4f(0, 1.0, 2.0, 3.0, 4.0);
            glUseProgram(b);
            glUniform4f(0, 5.0, 6.0, 7.0, 8.0);
            glUseProgram(0);
            assert_eq!(gl.ctx().cur_program, 0);
        }
        assert_eq!(fs_bits(&gl, a, 0, 4), fbits(&[1.0, 2.0, 3.0, 4.0]));
        assert_eq!(fs_bits(&gl, b, 0, 4), fbits(&[5.0, 6.0, 7.0, 8.0]));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn use_program_of_an_unknown_name_is_an_error_and_keeps_the_binding() {
        let gl = fresh();
        let a = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(a);
            glUseProgram(a + 1);
            assert_ne!(gl.error(), GL_NO_ERROR);
            glUseProgram(GLuint::MAX);
            assert_ne!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(gl.ctx().cur_program, a);
    }

    #[test]
    fn use_program_rejects_an_unlinked_program() {
        let gl = fresh();
        let a = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(a);
            let unlinked = glCreateProgram();
            glUseProgram(unlinked);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert_eq!(gl.ctx().cur_program, a);
    }

    #[test]
    fn uniform_setter_on_an_unlinked_current_program_is_invalid_operation() {
        /* whether or not glUseProgram accepts it, nothing can be set on a program without an executable */
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let unlinked = glCreateProgram();
            glUseProgram(unlinked);
            let _ = gl.error();
            glUniform1f(0, 1.0);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn program_entry_points_raise_invalid_value_for_unknown_names() {
        let gl = fresh();
        let mut iv: GLint = 0;
        let mut buf = [0u8; 8];
        // SAFETY: plain GL calls; nothing is written for an unknown program.
        unsafe {
            let s = glCreateShader(GL_VERTEX_SHADER);
            glUseProgram(9);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glLinkProgram(9);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glAttachShader(9, s);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetProgramiv(9, GL_LINK_STATUS, &mut iv);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetProgramInfoLog(9, 8, core::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetUniformLocation(9, c"x".as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetAttribLocation(9, c"x".as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glBindAttribLocation(9, 0, c"x".as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /* ---- uniform locations ---- */

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn locations_number_vs_uniforms_first_then_fs_only_names() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        /* main.vert: viewSize; fill_color.frag: frag[14], viewSize */
        assert_eq!(loc(p, "viewSize"), 0);
        assert_eq!(loc(p, "frag"), 1);
        assert_eq!(get_iv(p, GL_ACTIVE_UNIFORMS), 2, "a name in both stages is one uniform");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn location_of_array_element_zero_is_the_arrays_location() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        assert_eq!(loc(p, "frag[0]"), loc(p, "frag"));
        /* a non-array may not be addressed as [0]... but the offline reflection cannot tell; only pin arrays */
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn location_of_an_inactive_or_malformed_name_is_minus_one_without_error() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        for name in ["", "nope", "fra", "frag2", "frag[", "frag[x]", "frag[0", "[0]", "[0].x", "frag[14]", "frag[-1]"] {
            assert_eq!(loc(p, name), -1, "{name:?}");
        }
        // SAFETY: a null name is tolerated.
        assert_eq!(unsafe { glGetUniformLocation(p, core::ptr::null()) }, -1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn location_lookup_survives_an_over_long_name() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let long = std::format!("{}[3].{}", "a".repeat(40), "b".repeat(200));
        assert_eq!(loc(p, &long), -1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn location_on_an_unlinked_program_is_invalid_operation() {
        let gl = fresh();
        // SAFETY: plain GL call.
        let p = unsafe { glCreateProgram() };
        assert_eq!(loc(p, "u_color"), -1);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn location_of_a_plain_array_element() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let l = loc(p, "frag[3]");
        assert_ne!(l, -1);
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glUniform4f(l, 1.0, 2.0, 3.0, 4.0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(fs_bits(&gl, p, 12, 4), fbits(&[1.0, 2.0, 3.0, 4.0]));
    }

    /* ---- array-of-structs members (flattened per member by the offline table) ---- */

    /// 83a830e25f56fd85.vert + 0119aeb276db508c.frag (Qt Quick 3D lit material). FS reflection rows used below
    /// (blobs.rs): ubLights.lights[0].position vec3[16] @620 (64 dwords), .coneAngle float[16] @684,
    /// .pcfSamples int[16] @956, ubDirectionalLights.directionalLights[0].matrices mat4[16] @180 with inner 4
    /// (4 lights x 4 cascades), .direction vec3[4] @120, .shadowBias float[4] @136.
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* only the PowerVR table has these shaders */
    fn lit_program(gl: &TestGl) -> GLuint {
        let p = gl.program("83a830e25f56fd85.vert", "0119aeb276db508c.frag");
        let c = gl.ctx();
        c.objs().programs[obj_index(p)].blocks_mut().1.fill(f32::from_bits(SENTINEL));
        // SAFETY: plain GL call.
        unsafe { glUseProgram(p) };
        p
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn struct_array_member_location_carries_the_element_in_the_upper_bits() {
        let gl = fresh();
        let p = lit_program(&gl);
        let base = loc(p, "ubLights.lights[0].position");
        assert!((0..256).contains(&base));
        assert_eq!(loc(p, "ubLights.lights[3].position"), base | (3 << 8));
        assert_eq!(loc(p, "ubLights.lights[15].position"), base | (15 << 8));
        assert_eq!(loc(p, "ubLights.lights[16].position"), -1, "beyond the array");
        assert_eq!(loc(p, "ubLights.lights[3].nothing"), -1);
        assert_eq!(loc(p, "ubLights.nope[3].position"), -1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn struct_array_member_with_an_inner_array_scales_the_element() {
        let gl = fresh();
        let p = lit_program(&gl);
        let base = loc(p, "ubDirectionalLights.directionalLights[0].matrices");
        assert!(base >= 0);
        /* each light holds 4 matrices: light 1 starts at element 4 */
        assert_eq!(loc(p, "ubDirectionalLights.directionalLights[1].matrices"), base | (4 << 8));
        assert_eq!(loc(p, "ubDirectionalLights.directionalLights[3].matrices"), base | (12 << 8));
        assert_eq!(loc(p, "ubDirectionalLights.directionalLights[4].matrices"), -1);
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn set_vec3_member_of_struct_array_element() {
        let gl = fresh();
        let p = lit_program(&gl);
        // SAFETY: plain GL calls.
        unsafe { glUniform3f(loc(p, "ubLights.lights[3].position"), 1.0, 2.0, 3.0) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* vec3 elements are vec4-strided: element 3 starts at 620 + 3 * 4 */
        assert_eq!(
            fs_bits(&gl, p, 631, 5),
            bits(&[None, Some(1.0), Some(2.0), Some(3.0), None]),
            "only the three components of element 3"
        );
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn set_scalar_members_of_struct_array_elements() {
        let gl = fresh();
        let p = lit_program(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform1f(loc(p, "ubLights.lights[15].coneAngle"), 0.5);
            glUniform1i(loc(p, "ubLights.lights[2].pcfSamples"), -2);
            glUniform1f(loc(p, "ubDirectionalLights.directionalLights[3].shadowBias"), 8.0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(fs_bits(&gl, p, 684 + 14, 2), bits(&[None, Some(0.5)]));
        assert_eq!(fs_bits(&gl, p, 956, 4), [SENTINEL, SENTINEL, (-2i32) as u32, SENTINEL]);
        assert_eq!(fs_bits(&gl, p, 136, 4), bits(&[None, None, None, Some(8.0)]));
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn set_matrices_of_one_struct_array_element() {
        let gl = fresh();
        let p = lit_program(&gl);
        let m: Vec<f32> = (0..64).map(|i| i as f32 + 0.5).collect();
        let l = loc(p, "ubDirectionalLights.directionalLights[1].matrices");
        // SAFETY: m holds the four matrices announced.
        unsafe { glUniformMatrix4fv(l, 4, GL_FALSE_B, m.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(fs_bits(&gl, p, 180 + 64, 64), fbits(&m));
        assert_eq!(fs_bits(&gl, p, 180 + 63, 1), [SENTINEL]);
        assert_eq!(fs_bits(&gl, p, 180 + 128, 1), [SENTINEL]);
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn a_struct_member_that_is_not_an_array_takes_count_one_only() {
        /* This test used to expect the write to run on into the following structs' members. GL ES 3.0 2.12.6:
         * "ubLights.lights[14].position" is a vec3, not an array, so count > 1 is GL_INVALID_OPERATION; the
         * struct array around it is not the uniform's array. */
        let gl = fresh();
        let p = lit_program(&gl);
        let v: Vec<f32> = (0..15).map(|i| i as f32 + 1.0).collect();
        let ones = [1.0f32; 8];
        // SAFETY: the arrays hold the values announced.
        unsafe {
            glUniform3fv(loc(p, "ubLights.lights[14].position"), 5, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1fv(loc(p, "ubDirectionalLights.directionalLights[2].shadowBias"), 8, ones.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform3fv(loc(p, "ubLights.lights[14].position"), 1, v.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(
            fs_bits(&gl, p, 620 + 56, 8),
            bits(&[Some(1.0), Some(2.0), Some(3.0), None, None, None, None, None])
        );
        assert_eq!(fs_bits(&gl, p, 136, 4), [SENTINEL; 4]);
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn an_inner_array_write_is_clamped_to_the_inner_array() {
        /* matrices is mat4[4] per light: the location of light k's matrices addresses a 4-element array */
        let gl = fresh();
        let p = lit_program(&gl);
        let m = [2.0f32; 16 * 8];
        // SAFETY: m holds the eight matrices announced.
        unsafe {
            glUniformMatrix4fv(loc(p, "ubDirectionalLights.directionalLights[3].matrices"), 8, TRANSPOSE_B, m.as_ptr());
            glUniformMatrix4fv(loc(p, "ubDirectionalLights.directionalLights[1].matrices"), 8, TRANSPOSE_B, m.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(fs_bits(&gl, p, 180 + 192, 64), fbits(&[2.0; 64]));
        assert_eq!(fs_bits(&gl, p, 436, 2), [SENTINEL; 2], "dimensionsInverted, behind the matrices");
        assert_eq!(fs_bits(&gl, p, 180 + 64, 64), fbits(&[2.0; 64]));
        assert_eq!(fs_bits(&gl, p, 180 + 128, 64), [SENTINEL; 64], "light 2's matrices are not light 1's array");
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn an_inner_array_element_has_its_own_location() {
        let gl = fresh();
        let p = lit_program(&gl);
        let base = loc(p, "ubDirectionalLights.directionalLights[0].matrices");
        assert_eq!(loc(p, "ubDirectionalLights.directionalLights[1].matrices[2]"), base | (6 << 8));
        assert_eq!(loc(p, "ubDirectionalLights.directionalLights[0].matrices[3]"), base | (3 << 8));
        assert_eq!(loc(p, "ubDirectionalLights.directionalLights[0].matrices[4]"), -1, "past the inner array");
        assert_eq!(loc(p, "ubDirectionalLights.directionalLights[3].matrices[3]"), base | (15 << 8));
        assert_eq!(loc(p, "ubLights.lights[2].position[1]"), -1, "position is not an array");
        assert_eq!(loc(p, "ubLights.lights[2].position[0]"), loc(p, "ubLights.lights[2].position"));
        let big = std::format!("ubDirectionalLights.directionalLights[{}].matrices", u64::MAX / 2);
        assert_eq!(loc(p, &big), -1, "a huge index does not wrap to a valid element");
        let m: Vec<f32> = (0..32).map(|i| i as f32).collect();
        // SAFETY: m holds the two matrices announced.
        unsafe { glUniformMatrix4fv(loc(p, "ubDirectionalLights.directionalLights[1].matrices[2]"), 2, 0, m.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(fs_bits(&gl, p, 180 + 6 * 16, 32), fbits(&m));
        assert_eq!(fs_bits(&gl, p, 180 + 8 * 16, 1), [SENTINEL]);
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn element_bits_beyond_the_array_write_nothing() {
        let gl = fresh();
        let p = lit_program(&gl);
        let l = loc(p, "ubDirectionalLights.directionalLights[0].shadowBias") | (4 << 8);
        // SAFETY: plain GL call.
        unsafe { glUniform1f(l, 9.0) };
        assert_eq!(fs_bits(&gl, p, 136, 8), [SENTINEL; 8]);
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn get_uniform_reads_the_element_the_location_addresses() {
        let gl = fresh();
        let p = lit_program(&gl);
        let l = loc(p, "ubDirectionalLights.directionalLights[2].shadowBias");
        let mut out = [0f32; 8];
        // SAFETY: plain GL calls; out is larger than the four floats of the member.
        unsafe {
            glUniform1f(loc(p, "ubDirectionalLights.directionalLights[0].shadowBias"), 3.0);
            glUniform1f(l, 7.0);
            glGetUniformfv(p, l, out.as_mut_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(out, [7.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0], "one float, element 2");
    }

    /* ---- float setters ---- */

    #[test]
    fn uniform_1f_to_4f_store_their_components() {
        let gl = fresh();
        let p = setter_program(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform1f(loc(p, "f1"), 1.5);
            glUniform2f(loc(p, "v2"), 2.5, -0.0);
            glUniform3f(loc(p, "v3"), 3.5, 4.5, 5.5);
            glUniform4f(loc(p, "v4"), 6.5, 7.5, 8.5, f32::INFINITY);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(
            vs_bits(&gl, p, 0, 12),
            bits(&[
                Some(1.5),
                None,
                Some(2.5),
                Some(-0.0),
                Some(3.5),
                Some(4.5),
                Some(5.5),
                None,
                Some(6.5),
                Some(7.5),
                Some(8.5),
                Some(f32::INFINITY),
            ])
        );
    }

    #[test]
    fn uniform_in_both_stages_is_written_to_both_blocks() {
        let gl = fresh();
        let p = setter_program(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform4f(loc(p, "v4"), 1.0, 2.0, 3.0, 4.0);
            glUniform1f(loc(p, "fonly"), 9.0);
            glUniform1f(loc(p, "f1"), 8.0);
        }
        assert_eq!(vs_bits(&gl, p, 8, 4), fbits(&[1.0, 2.0, 3.0, 4.0]));
        assert_eq!(fs_bits(&gl, p, 20, 4), fbits(&[1.0, 2.0, 3.0, 4.0]));
        /* single-stage uniforms stay out of the other block: fonly is FS dword 0, f1 VS dword 0 */
        assert_eq!(fs_bits(&gl, p, 0, 1), fbits(&[9.0]));
        assert_eq!(vs_bits(&gl, p, 0, 1), fbits(&[8.0]));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn real_shared_uniform_lands_at_each_stages_own_offset() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glUniform2f(loc(p, "viewSize"), 640.0, 480.0);
        }
        /* blobs.rs: viewSize is VS dword 0 and FS dword 56 (behind frag[14]); the table reports a 52-dword FS
         * block, the block sized at link time reaches the end of the layout (58) */
        assert_eq!(vs_bits(&gl, p, 0, 2), fbits(&[640.0, 480.0]));
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_block().len(), 58);
        assert_eq!(fs_bits(&gl, p, 56, 2), fbits(&[640.0, 480.0]));
    }

    /* ---- uniform blocks sized at link time (upstream bcef68b) --------------------------------------- */

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn link_sizes_each_block_to_its_stage() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let pr = &gl.ctx().objs_ref().programs[obj_index(p)];
        assert_eq!((pr.rvs.uniform_dwords, pr.rfs.uniform_dwords), (2, 52));
        /* the FS layout ends at 58 (viewSize behind frag[14]), past the table's 52 */
        assert_eq!((pr.vs_uniform_cap, pr.fs_uniform_cap), (2, 58));
        assert_eq!((pr.vs_block().len(), pr.fs_block().len()), (2, 58));
        assert!(pr.vs_block().iter().chain(pr.fs_block()).all(|f| f.to_bits() == 0), "zeroed");
    }

    #[test]
    fn a_stage_without_uniforms_gets_an_empty_block() {
        let gl = fresh();
        let p = gl.program("passthrough.vert", "interp_color.frag");
        let pr = &gl.ctx().objs_ref().programs[obj_index(p)];
        assert_eq!((pr.vs_uniform_cap, pr.fs_uniform_cap), (0, 0));
        /* calloc(1): a live allocation even for no dwords, as in the C */
        assert!(!pr.vs_uniforms.is_null() && !pr.fs_uniforms.is_null());
        assert!(pr.vs_block().is_empty() && pr.fs_block().is_empty());
    }

    #[test]
    fn a_failed_link_leaves_no_blocks() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        assert!(!gl.ctx().objs().programs[obj_index(p)].fs_uniforms.is_null());
        let fs = gl.ctx().objs().programs[obj_index(p)].fs;
        // SAFETY: plain GL calls.
        unsafe {
            glDetachShader(p, fs);
            glLinkProgram(p);
        }
        let pr = &gl.ctx().objs_ref().programs[obj_index(p)];
        assert!(!pr.linked);
        assert!(pr.vs_uniforms.is_null() && pr.fs_uniforms.is_null());
        assert_eq!((pr.vs_uniform_cap, pr.fs_uniform_cap), (0, 0));
    }

    #[test]
    fn relink_and_delete_release_the_blocks() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glUniform2f(loc(p, "viewSize"), 640.0, 480.0);
            glLinkProgram(p);
        }
        let pr = &gl.ctx().objs_ref().programs[obj_index(p)];
        assert!(pr.linked && pr.fs_block().len() == 58);
        assert_eq!(vs_bits(&gl, p, 0, 2), [0, 0], "a fresh block after the relink");
        // SAFETY: plain GL calls.
        unsafe {
            glDeleteProgram(p);
            glUseProgram(0); /* the current program goes when it stops being current */
        }
        let pr = &gl.ctx().objs_ref().programs[obj_index(p)];
        assert!(pr.vs_uniforms.is_null() && pr.fs_uniforms.is_null() && !pr.used);
    }

    #[test]
    fn the_program_pool_has_512_slots() {
        let gl = fresh();
        assert_eq!(MAX_PROGRAMS, 512);
        // SAFETY: plain GL calls.
        unsafe {
            let handles: Vec<GLuint> = (1..MAX_PROGRAMS).map(|_| glCreateProgram()).collect();
            /* names carry the object kind in the low bit, so program slot i is
             * name 2i: 0 and 1 are never handed out (one name space, 2.10) */
            assert_eq!(handles, (1..MAX_PROGRAMS).map(|i| obj_name(i, OBJ_PROGRAM)).collect::<Vec<_>>());
            assert_eq!(glCreateProgram(), 0, "slot 0 is never handed out");
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
    }

    #[test]
    fn uniforms_the_reflection_places_past_uniform_dwords_are_stored() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let frag = [0.5f32; 56];
        // SAFETY: plain GL calls; frag holds 14 vec4.
        unsafe {
            glUseProgram(p);
            glUniform4fv(loc(p, "frag"), 14, frag.as_ptr());
            glUniform2f(loc(p, "viewSize"), 640.0, 480.0);
        }
        assert_eq!(fs_bits(&gl, p, 0, 1), fbits(&[0.5]));
        assert_eq!(fs_bits(&gl, p, 56, 2), fbits(&[640.0, 480.0]));
    }

    #[test]
    fn block_dwords_covers_the_layout_and_the_table_size() {
        let (mut p, _, _) = blank();
        p.rvs.uniform_dwords = 8;
        set_table(&mut p.rvs, std::vec![uni("a", 0, 4, GL_FLOAT_VEC4, 1), sampler("s", 0)]);
        assert_eq!(block_dwords(&p.rvs), 8, "a table size beyond the layout is kept");
        set_table(&mut p.rvs, std::vec![uni("a", 4, 16, GL_FLOAT_VEC4, 4)]);
        assert_eq!(block_dwords(&p.rvs), 20, "a member past uniform_dwords extends the block");
        p.rvs.uniform_dwords = 0;
        set_table(&mut p.rvs, std::vec![sampler("s", 0)]);
        assert_eq!(block_dwords(&p.rvs), 0, "samplers take no block storage");
        set_table(&mut p.rvs, std::vec![uni("far", 0xfff0, 8, GL_FLOAT_VEC4, 2)]);
        assert_eq!(block_dwords(&p.rvs), MAX_UNIFORM_DWORDS, "capped");
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn femtovg_frag_array_reaches_the_draw_and_the_upload_keeps_the_table_size() {
        /* the Slint UI on the device: FemtoVG's fill shaders set frag[14] with one glUniform4fv; the upload is the
         * 52 dwords the shader reads, now holding the application's values */
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let frag: Vec<f32> = (1..=56).map(|i| i as f32).collect();
        let verts: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        // SAFETY: plain GL calls; the client arrays outlive the draw.
        unsafe {
            use crate::gl::{draw::glDrawArrays, vertex::*};
            glUseProgram(p);
            glUniform4fv(loc(p, "frag"), 14, frag.as_ptr());
            glUniform2f(loc(p, "viewSize"), 640.0, 480.0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].draw.fs_uniform_dwords, 52);
        assert_eq!(draws[0].fs_uniforms, fbits(&frag[..52]));
        assert_eq!(draws[0].vs_uniforms, fbits(&[640.0, 480.0]));
    }

    #[test]
    fn uniform_fv_with_count_one_matches_the_scalar_forms() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v = [1.0f32, 2.0, 3.0, 4.0];
        // SAFETY: v holds four floats.
        unsafe {
            glUniform1fv(loc(p, "f1"), 1, v.as_ptr());
            glUniform2fv(loc(p, "v2"), 1, v.as_ptr());
            glUniform3fv(loc(p, "v3"), 1, v.as_ptr());
            glUniform4fv(loc(p, "v4"), 1, v.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(vs_bits(&gl, p, 0, 2), bits(&[Some(1.0), None]));
        assert_eq!(vs_bits(&gl, p, 2, 2), fbits(&[1.0, 2.0]));
        assert_eq!(vs_bits(&gl, p, 4, 4), bits(&[Some(1.0), Some(2.0), Some(3.0), None]));
        assert_eq!(vs_bits(&gl, p, 8, 4), fbits(&v));
    }

    #[test]
    fn uniform_1fv_array_is_tight_and_count_is_clamped() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v: Vec<f32> = (1..=9).map(|i| i as f32).collect();
        // SAFETY: v holds the floats announced.
        unsafe {
            glUniform1fv(loc(p, "fa"), 2, v.as_ptr());
            assert_eq!(vs_bits(&gl, p, 12, 4), bits(&[Some(1.0), Some(2.0), None, None]));
            glUniform1fv(loc(p, "fa"), 9, v.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "GL: elements beyond the array are ignored");
        assert_eq!(vs_bits(&gl, p, 12, 5), bits(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0), None]));
    }

    #[test]
    fn uniform_2fv_and_4fv_arrays_are_tight_and_clamped() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v: Vec<f32> = (1..=16).map(|i| i as f32).collect();
        // SAFETY: v holds the floats announced.
        unsafe {
            glUniform2fv(loc(p, "v2a[0]"), 8, v.as_ptr());
            glUniform4fv(loc(p, "v4a"), 4, v.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(
            vs_bits(&gl, p, 16, 8),
            bits(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0), Some(5.0), Some(6.0), None, None])
        );
        assert_eq!(vs_bits(&gl, p, 36, 8), fbits(&v[..8]));
        assert_eq!(vs_bits(&gl, p, 44, 1), [SENTINEL], "m2 behind v4a is untouched");
    }

    #[test]
    fn uniform_3fv_array_uses_vec4_strides() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v: Vec<f32> = (1..=15).map(|i| i as f32).collect();
        // SAFETY: v holds the floats announced.
        unsafe {
            glUniform3fv(loc(p, "v3a"), 2, v.as_ptr());
            assert_eq!(
                vs_bits(&gl, p, 24, 12),
                bits(&[
                    Some(1.0),
                    Some(2.0),
                    Some(3.0),
                    None,
                    Some(4.0),
                    Some(5.0),
                    Some(6.0),
                    None,
                    None,
                    None,
                    None,
                    None
                ])
            );
            glUniform3fv(loc(p, "v3a"), 5, v.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(
            vs_bits(&gl, p, 32, 5),
            bits(&[Some(7.0), Some(8.0), Some(9.0), None, None]),
            "clamped to 3 elements"
        );
    }

    #[test]
    fn uniform_fv_ignores_zero_count_and_null_values() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v = [1.0f32; 16];
        // SAFETY: nothing is read for count 0 or a null pointer.
        unsafe {
            glUniform1fv(loc(p, "fa"), 0, v.as_ptr());
            glUniform3fv(loc(p, "v3a"), 0, v.as_ptr());
            glUniform4fv(loc(p, "v4"), 1, core::ptr::null());
            glUniform1iv(loc(p, "ia"), 0, v.as_ptr() as *const GLint);
            glUniform1iv(loc(p, "ia"), 1, core::ptr::null());
            glUniformMatrix4fv(loc(p, "m4"), 0, GL_FALSE_B, v.as_ptr());
            glUniformMatrix4fv(loc(p, "m4"), 1, GL_FALSE_B, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(vs_bits(&gl, p, 0, 160).iter().all(|b| *b == SENTINEL));
    }

    #[test]
    fn uniform_negative_count_is_invalid_value() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v = [1.0f32; 16];
        // SAFETY: nothing is read for a negative count.
        unsafe {
            glUniform1fv(loc(p, "fa"), -1, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glUniform1iv(loc(p, "ia"), -1, v.as_ptr() as *const GLint);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glUniformMatrix4fv(loc(p, "m4"), -1, GL_FALSE_B, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /* ---- matrices ---- */

    #[test]
    fn matrix4_is_stored_column_major_as_given() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: Vec<f32> = (0..16).map(|i| i as f32).collect();
        // SAFETY: m holds one matrix.
        unsafe { glUniformMatrix4fv(loc(p, "m4"), 1, GL_FALSE_B, m.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(vs_bits(&gl, p, 60, 16), fbits(&m));
        assert_eq!(vs_bits(&gl, p, 59, 1), [SENTINEL]);
        assert_eq!(vs_bits(&gl, p, 76, 1), [SENTINEL]);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* the transposing upload is ES 3.0's */
    fn matrix4_transpose_flag_swaps_rows_and_columns() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: Vec<f32> = (0..16).map(|i| i as f32).collect();
        // SAFETY: m holds one matrix.
        unsafe { glUniformMatrix4fv(loc(p, "m4"), 1, GL_TRUE_B, m.as_ptr()) };
        /* row-major input: block[col * 4 + row] = m[row * 4 + col] */
        let want: Vec<f32> = (0..16).map(|i| ((i % 4) * 4 + i / 4) as f32).collect();
        assert_eq!(vs_bits(&gl, p, 60, 16), fbits(&want));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn matrix3_columns_are_vec4_strided_in_the_block() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: [f32; 9] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        // SAFETY: m holds one matrix.
        unsafe { glUniformMatrix3fv(loc(p, "m3"), 1, GL_FALSE_B, m.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let got = vs_bits(&gl, p, 48, 12);
        for col in 0..3 {
            assert_eq!(&got[col * 4..col * 4 + 3], &fbits(&m[col * 3..col * 3 + 3])[..], "column {col}");
        }
        assert_eq!(vs_bits(&gl, p, 47, 1), [SENTINEL]);
        assert_eq!(vs_bits(&gl, p, 60, 1), [SENTINEL], "m4 behind it is untouched");
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* the transposing upload is ES 3.0's */
    fn matrix3_transposed() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: [f32; 9] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        // SAFETY: m holds one matrix.
        unsafe { glUniformMatrix3fv(loc(p, "m3"), 1, GL_TRUE_B, m.as_ptr()) };
        let got = vs_bits(&gl, p, 48, 12);
        assert_eq!(&got[0..3], &fbits(&[1.0, 4.0, 7.0])[..]);
        assert_eq!(&got[4..7], &fbits(&[2.0, 5.0, 8.0])[..]);
        assert_eq!(&got[8..11], &fbits(&[3.0, 6.0, 9.0])[..]);
    }

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn real_normal_matrix_is_strided_in_the_vs_block() {
        let gl = fresh();
        let p = gl.program("fd0c8a22ac0eef86.vert", "0119aeb276db508c.frag");
        let m: [f32; 9] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        // SAFETY: m holds one matrix.
        unsafe {
            glUseProgram(p);
            glUniformMatrix3fv(loc(p, "_50.qt_normalMatrix"), 1, GL_FALSE_B, m.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* blobs.rs: _50.qt_normalMatrix mat3 @64, 12 dwords */
        let got = vs_bits(&gl, p, 64, 12);
        assert_eq!(&got[0..3], &fbits(&[1.0, 2.0, 3.0])[..]);
        assert_eq!(&got[4..7], &fbits(&[4.0, 5.0, 6.0])[..]);
        assert_eq!(&got[8..11], &fbits(&[7.0, 8.0, 9.0])[..]);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* the transposing upload is ES 3.0's */
    fn matrix2_plain_and_transposed() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: [f32; 4] = [1.0, 2.0, 3.0, 4.0];
        // SAFETY: m holds one matrix.
        unsafe {
            glUniformMatrix2fv(loc(p, "m2"), 1, GL_FALSE_B, m.as_ptr());
            assert_eq!(vs_bits(&gl, p, 44, 4), fbits(&[1.0, 2.0, 3.0, 4.0]));
            glUniformMatrix2fv(loc(p, "m2"), 1, GL_TRUE_B, m.as_ptr());
            assert_eq!(vs_bits(&gl, p, 44, 4), fbits(&[1.0, 3.0, 2.0, 4.0]));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(not(feature = "report-es2"))] /* the transposing upload is ES 3.0's */
    fn matrix_arrays_are_clamped_to_the_array_length() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: Vec<f32> = (0..48).map(|i| i as f32).collect();
        // SAFETY: m holds the three matrices announced.
        unsafe { glUniformMatrix4fv(loc(p, "m4a"), 3, GL_FALSE_B, m.as_ptr()) };
        assert_eq!(vs_bits(&gl, p, 100, 32), fbits(&m[..32]));
        assert_eq!(vs_bits(&gl, p, 132, 1), [SENTINEL], "i1 behind the array is untouched");

        // SAFETY: as above.
        unsafe { glUniformMatrix4fv(loc(p, "m4a"), 3, GL_TRUE_B, m.as_ptr()) };
        let got = vs_bits(&gl, p, 100, 32);
        assert_eq!(got[1], 4.0f32.to_bits(), "matrix 0: row 1, column 0 of the row-major input");
        assert_eq!(got[16 + 1], 20.0f32.to_bits(), "matrix 1");
        assert_eq!(vs_bits(&gl, p, 132, 1), [SENTINEL]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn matrix3_array_elements_are_twelve_dwords_apart() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: Vec<f32> = (1..=27).map(|i| i as f32).collect();
        // SAFETY: m holds the three matrices announced (one more than the array).
        unsafe { glUniformMatrix3fv(loc(p, "m3a"), 3, GL_FALSE_B, m.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        let got = vs_bits(&gl, p, 76, 24);
        assert_eq!(&got[0..3], &fbits(&[1.0, 2.0, 3.0])[..]);
        assert_eq!(&got[8..11], &fbits(&[7.0, 8.0, 9.0])[..]);
        assert_eq!(&got[12..15], &fbits(&[10.0, 11.0, 12.0])[..]);
        assert_eq!(&got[20..23], &fbits(&[16.0, 17.0, 18.0])[..]);
        assert_eq!(vs_bits(&gl, p, 100, 1), [SENTINEL], "m4a behind the array is untouched");
    }

    #[test]
    fn matrix_setter_on_an_int_uniform_is_invalid_operation() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m = [1.0f32; 16];
        // SAFETY: m holds one matrix.
        unsafe {
            glUniformMatrix4fv(loc(p, "i4"), 1, GL_FALSE_B, m.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniformMatrix4fv(loc(p, "i4"), 1, TRANSPOSE_B, m.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniformMatrix3fv(loc(p, "i4"), 1, GL_FALSE_B, m.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert_eq!(vs_bits(&gl, p, 140, 4), [SENTINEL; 4]);
    }

    /// The transpose flag the matrix setters take in this build. ES 3.0
    /// accepts GL_TRUE; ES 2.0 has no transposing upload at all and answers
    /// GL_INVALID_VALUE, so a test whose subject is some *other* error asks
    /// for the flag this build takes rather than pinning GL_TRUE.
    const TRANSPOSE_B: GLboolean = if crate::REPORT_ES2 { GL_FALSE_B } else { GL_TRUE_B };

    /* ---- int / uint setters ---- */

    #[test]
    fn uniform_1i_to_4i_store_raw_bit_patterns() {
        let gl = fresh();
        let p = setter_program(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform1i(loc(p, "i1"), -1);
            glUniform2i(loc(p, "i2"), 1, GLint::MIN);
            glUniform3i(loc(p, "i3"), 2, 3, GLint::MAX);
            glUniform4i(loc(p, "i4"), 4, 5, 6, -7);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(
            vs_bits(&gl, p, 132, 12),
            [0xffff_ffff, SENTINEL, 1, 0x8000_0000, 2, 3, 0x7fff_ffff, SENTINEL, 4, 5, 6, 0xffff_fff9]
        );
    }

    #[test]
    fn uniform_iv_forms_store_arrays_and_clamp_the_count() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v: [GLint; 12] = [10, -11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21];
        // SAFETY: v holds the ints announced.
        unsafe {
            glUniform1iv(loc(p, "ia"), 5, v.as_ptr());
            glUniform4iv(loc(p, "i4a"), 3, v.as_ptr());
            glUniform2iv(loc(p, "i2"), 1, v.as_ptr());
            glUniform3iv(loc(p, "i3"), 1, v.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(vs_bits(&gl, p, 144, 4), [10, (-11i32) as u32, 12, SENTINEL]);
        assert_eq!(vs_bits(&gl, p, 148, 9), [10, (-11i32) as u32, 12, 13, 14, 15, 16, 17, SENTINEL]);
        assert_eq!(vs_bits(&gl, p, 134, 2), [10, (-11i32) as u32]);
        assert_eq!(vs_bits(&gl, p, 136, 4), [10, (-11i32) as u32, 12, SENTINEL]);
    }

    #[test]
    fn unsigned_setters_store_the_same_bits_through_the_int_path() {
        /* Known, commented deviation: the ES 3.0 unsigned setters are accepted on int uniforms (Qt's RHI picks
         * them from the SPIR-V type); the bit patterns are identical. */
        let gl = fresh();
        let p = setter_program(&gl);
        let v: [GLuint; 8] = [0xffff_ffff, 0x8000_0000, 3, 4, 5, 6, 7, 8];
        // SAFETY: v holds the values announced.
        unsafe {
            glUniform1ui(loc(p, "i1"), 0xdead_beef);
            glUniform2ui(loc(p, "i2"), 0xffff_fffe, 2);
            glUniform3ui(loc(p, "i3"), 1, 0x8000_0001, 3);
            glUniform4ui(loc(p, "i4"), 1, 2, 3, 0xf000_0000);
            assert_eq!(
                vs_bits(&gl, p, 132, 12),
                [0xdead_beef, SENTINEL, 0xffff_fffe, 2, 1, 0x8000_0001, 3, SENTINEL, 1, 2, 3, 0xf000_0000]
            );
            glUniform1uiv(loc(p, "ia"), 3, v.as_ptr());
            glUniform4uiv(loc(p, "i4a"), 2, v.as_ptr());
            glUniform2uiv(loc(p, "i2"), 1, v.as_ptr());
            glUniform3uiv(loc(p, "i3"), 1, v.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(vs_bits(&gl, p, 144, 3), [0xffff_ffff, 0x8000_0000, 3]);
        assert_eq!(vs_bits(&gl, p, 148, 8), v);
        assert_eq!(vs_bits(&gl, p, 134, 2), [0xffff_ffff, 0x8000_0000]);
        assert_eq!(vs_bits(&gl, p, 136, 3), [0xffff_ffff, 0x8000_0000, 3]);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn real_int_uniform_in_both_stages() {
        let gl = fresh();
        let p = gl.program("visualization.vert", "visualization.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glUniform1i(loc(p, "ubuf.projection"), -3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* blobs.rs: ubuf.projection int @37 in both stages */
        assert_eq!(vs_bits(&gl, p, 36, 2), [0, (-3i32) as u32]);
        assert_eq!(fs_bits(&gl, p, 36, 2), [0, (-3i32) as u32]);
    }

    #[test]
    fn bool_uniform_takes_the_int_setter() {
        let gl = fresh();
        let p = setter_program(&gl);
        // SAFETY: plain GL call.
        unsafe { glUniform1i(loc(p, "b1"), 1) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(vs_bits(&gl, p, 156, 1), [1]);
    }

    #[test]
    fn bool_uniform_takes_the_float_setter() {
        let gl = fresh();
        let p = setter_program(&gl);
        // SAFETY: plain GL call.
        unsafe { glUniform1f(loc(p, "b1"), 1.0) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_ne!(vs_bits(&gl, p, 156, 1), [0]);
        assert_ne!(vs_bits(&gl, p, 156, 1), [SENTINEL]);
    }

    /* ---- setter errors ---- */

    #[test]
    fn float_setter_on_int_and_int_setter_on_float_are_invalid_operation() {
        let gl = fresh();
        let p = setter_program(&gl);
        let fv = [1.0f32; 4];
        let iv = [1 as GLint; 4];
        // SAFETY: the arrays hold the values announced.
        unsafe {
            glUniform1f(loc(p, "i1"), 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform4fv(loc(p, "i4"), 1, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1i(loc(p, "f1"), 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform4iv(loc(p, "v4"), 1, iv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1ui(loc(p, "f1"), 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1f(loc(p, "tex"), 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "samplers are set with glUniform1i only");
        }
        assert!(vs_bits(&gl, p, 0, 160).iter().all(|b| *b == SENTINEL), "nothing was stored");
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_sampler_unit, [0; CAP_SAMPLERS]);
    }

    #[test]
    fn location_minus_one_is_silently_ignored_by_every_setter() {
        let gl = fresh();
        let p = setter_program(&gl);
        let fv = [1.0f32; 16];
        let iv = [1 as GLint; 4];
        // SAFETY: the arrays hold the values announced.
        unsafe {
            glUniform1f(-1, 1.0);
            glUniform2f(-1, 1.0, 1.0);
            glUniform3f(-1, 1.0, 1.0, 1.0);
            glUniform4f(-1, 1.0, 1.0, 1.0, 1.0);
            glUniform1fv(-1, 1, fv.as_ptr());
            glUniform3fv(-1, 2, fv.as_ptr());
            glUniform1i(-1, 1);
            glUniform4i(-1, 1, 1, 1, 1);
            glUniform4iv(-1, 1, iv.as_ptr());
            glUniform1ui(-1, 1);
            glUniformMatrix4fv(-1, 1, GL_FALSE_B, fv.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(vs_bits(&gl, p, 0, 160).iter().all(|b| *b == SENTINEL));

        /* Without a current program the call is GL_INVALID_OPERATION, and
         * that outranks location -1: -1 is ignored only while a linked
         * program is in use (dEQP negative_api.shader.uniformf_invalid_program). */
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(0);
            glUniform1f(-1, 1.0);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    /// The same, entry point by entry point: dEQP asks it of the four scalar
    /// setters, the four vector ones and glUniformMatrix*fv.
    #[test]
    fn a_uniform_setter_with_no_current_program_is_invalid_operation() {
        let gl = fresh();
        setter_program(&gl);
        let fv = [1.0f32; 16];
        let iv = [1 as GLint; 4];
        // SAFETY: the arrays hold the values announced.
        unsafe { glUseProgram(0) };
        let calls: [(&str, &dyn Fn()); 10] = [
            ("glUniform1f", &|| unsafe { glUniform1f(-1, 0.0) }),
            ("glUniform2f", &|| unsafe { glUniform2f(-1, 0.0, 0.0) }),
            ("glUniform3f", &|| unsafe { glUniform3f(-1, 0.0, 0.0, 0.0) }),
            ("glUniform4f", &|| unsafe { glUniform4f(-1, 0.0, 0.0, 0.0, 0.0) }),
            ("glUniform1fv", &|| unsafe { glUniform1fv(-1, 1, fv.as_ptr()) }),
            ("glUniform4fv", &|| unsafe { glUniform4fv(-1, 1, fv.as_ptr()) }),
            ("glUniform1i", &|| unsafe { glUniform1i(-1, 0) }),
            ("glUniform4i", &|| unsafe { glUniform4i(-1, 0, 0, 0, 0) }),
            ("glUniform1iv", &|| unsafe { glUniform1iv(-1, 1, iv.as_ptr()) }),
            ("glUniformMatrix4fv", &|| unsafe { glUniformMatrix4fv(-1, 1, GL_FALSE_B, fv.as_ptr()) }),
        ];
        for (name, call) in calls {
            call();
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "{name}");
        }
    }

    #[test]
    fn invalid_location_is_invalid_operation() {
        let gl = fresh();
        let p = setter_program(&gl);
        let count = get_iv(p, GL_ACTIVE_UNIFORMS);
        let fv = [1.0f32; 16];
        // SAFETY: fv holds the values announced.
        unsafe {
            glUniform1f(count, 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1i(200, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform3fv(count, 2, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniformMatrix4fv(count, 1, TRANSPOSE_B, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1f(-2, 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "only -1 is exempt");
        }
    }

    #[test]
    fn setters_without_a_current_program_are_invalid_operation() {
        let gl = fresh();
        let fv = [1.0f32; 16];
        // SAFETY: fv holds the values announced.
        unsafe {
            glUniform1f(0, 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1i(0, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform3fv(0, 2, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniformMatrix4fv(0, 1, TRANSPOSE_B, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn setter_size_mismatch_is_invalid_operation() {
        let gl = fresh();
        let p = setter_program(&gl);
        let fv = [1.0f32; 16];
        // SAFETY: fv holds the values announced.
        unsafe {
            glUniform1f(loc(p, "v4"), 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform4f(loc(p, "f1"), 1.0, 1.0, 1.0, 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniformMatrix2fv(loc(p, "v4"), 1, GL_FALSE_B, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform2f(loc(p, "m4"), 1.0, 1.0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform2i(loc(p, "i4"), 1, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform2i(loc(p, "tex"), 1, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert!(vs_bits(&gl, p, 0, 160).iter().all(|b| *b == SENTINEL), "nothing was stored");
    }

    #[test]
    fn count_above_one_on_a_non_array_is_invalid_operation() {
        let gl = fresh();
        let p = setter_program(&gl);
        let fv = [1.0f32; 32];
        // SAFETY: fv holds the values announced.
        unsafe {
            glUniform4fv(loc(p, "v4"), 2, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniformMatrix4fv(loc(p, "m4"), 2, GL_FALSE_B, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn uniform_3fv_array_path_checks_the_type() {
        let gl = fresh();
        let p = setter_program(&gl);
        let fv = [1.0f32; 6];
        // SAFETY: fv holds the two vec3 announced.
        unsafe {
            glUniform3fv(loc(p, "i4a"), 2, fv.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert_eq!(vs_bits(&gl, p, 148, 8), [SENTINEL; 8]);
    }

    /* ---- samplers ---- */

    #[test]
    fn sampler_uniform_records_the_texture_unit_per_binding() {
        let gl = fresh();
        let p = setter_program(&gl);
        let seven: [GLint; 1] = [7];
        // SAFETY: plain GL calls.
        unsafe {
            glUniform1i(loc(p, "tex"), 5);
            glUniform1iv(loc(p, "tex7"), 1, seven.as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* tex has binding 3, tex7 binding 7 */
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_sampler_unit[..8], [0, 0, 0, 5, 0, 0, 0, 7]);
        assert!(fs_bits(&gl, p, 0, 64).iter().all(|b| *b == SENTINEL), "a sampler has no block storage");
    }

    #[test]
    fn real_sampler_uniforms_map_to_their_bindings() {
        let gl = fresh();
        let p = gl.program("uv.vert", "texture2.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glUseProgram(p);
            glUniform1i(loc(p, "u_tex0"), 2);
            glUniform1i(loc(p, "u_tex1"), 6);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(&gl.ctx().objs_ref().programs[obj_index(p)].fs_sampler_unit[..2], &[2, 6]);
    }

    #[test]
    fn sampler_unit_out_of_range_is_invalid_value() {
        let gl = fresh();
        let p = setter_program(&gl);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform1i(loc(p, "tex"), 4);
            glUniform1i(loc(p, "tex"), CAP_COMBINED_TEXTURE_UNITS as GLint);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glUniform1i(loc(p, "tex"), -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glUniform1i(loc(p, "tex"), CAP_COMBINED_TEXTURE_UNITS as GLint - 1);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_sampler_unit[3], CAP_COMBINED_TEXTURE_UNITS as u8 - 1);
    }

    /* ---- glGetUniform ---- */

    #[test]
    fn get_uniformfv_reads_back_what_was_set() {
        let gl = fresh();
        let p = setter_program(&gl);
        let mut out = [-1.0f32; 6];
        // SAFETY: out is larger than the vec4 read.
        unsafe {
            glUniform4f(loc(p, "v4"), 0.25, 0.5, 0.75, 1.0);
            glUseProgram(0); /* the query names its program */
            glGetUniformfv(p, loc(p, "v4"), out.as_mut_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(out, [0.25, 0.5, 0.75, 1.0, -1.0, -1.0]);
    }

    #[test]
    fn get_uniformfv_reads_an_fs_only_uniform_and_a_matrix() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: Vec<f32> = (0..16).map(|i| i as f32).collect();
        let mut one = [-1.0f32; 2];
        let mut mat = [-1.0f32; 17];
        // SAFETY: the outputs are larger than the values read.
        unsafe {
            glUniform1f(loc(p, "fonly"), 42.0);
            glUniformMatrix4fv(loc(p, "m4"), 1, GL_FALSE_B, m.as_ptr());
            glGetUniformfv(p, loc(p, "fonly"), one.as_mut_ptr());
            glGetUniformfv(p, loc(p, "m4"), mat.as_mut_ptr());
        }
        assert_eq!(one, [42.0, -1.0]);
        assert_eq!(&mat[..16], &m[..]);
        assert_eq!(mat[16], -1.0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_uniformiv_reads_ints_and_sampler_units() {
        let gl = fresh();
        let p = setter_program(&gl);
        let mut iv = [-99 as GLint; 5];
        let mut unit = [-99 as GLint; 2];
        // SAFETY: the outputs are larger than the values read.
        unsafe {
            glUniform4i(loc(p, "i4"), -1, 2, -3, 4);
            glUniform1i(loc(p, "tex7"), 6);
            glGetUniformiv(p, loc(p, "i4"), iv.as_mut_ptr());
            glGetUniformiv(p, loc(p, "tex7"), unit.as_mut_ptr());
        }
        assert_eq!(iv, [-1, 2, -3, 4, -99]);
        assert_eq!(unit, [6, -99]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_uniform_errors() {
        let gl = fresh();
        let p = setter_program(&gl);
        let mut out = [-1.0f32; 4];
        // SAFETY: nothing is written on error; a null output is tolerated.
        unsafe {
            glGetUniformfv(p, get_iv(p, GL_ACTIVE_UNIFORMS), out.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "location beyond the table");
            let unlinked = glCreateProgram();
            glGetUniformfv(unlinked, 0, out.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not linked");
            glGetUniformiv(60, 0, out.as_mut_ptr() as *mut GLint);
            assert_ne!(gl.error(), GL_NO_ERROR, "no such program");
            glGetUniformfv(p, loc(p, "v4"), core::ptr::null_mut());
            glGetUniformiv(p, loc(p, "tex"), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(out, [-1.0; 4]);
    }

    #[test]
    fn get_uniformfv_on_an_array_returns_one_element() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let v: Vec<f32> = (1..=56).map(|i| i as f32).collect();
        let mut out = [-1.0f32; 64];
        // SAFETY: out is large enough even for the whole array.
        unsafe {
            glUseProgram(p);
            glUniform4fv(loc(p, "frag"), 14, v.as_ptr());
            glGetUniformfv(p, loc(p, "frag"), out.as_mut_ptr());
        }
        assert_eq!(&out[..4], &[1.0, 2.0, 3.0, 4.0]);
        assert!(out[4..].iter().all(|f| *f == -1.0), "nothing behind the first vec4 is written");
    }

    #[test]
    fn get_uniformfv_on_a_mat3_returns_nine_floats() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m: [f32; 9] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
        let mut out = [-1.0f32; 12];
        // SAFETY: out is large enough even for the strided copy.
        unsafe {
            glUniformMatrix3fv(loc(p, "m3"), 1, GL_FALSE_B, m.as_ptr());
            glGetUniformfv(p, loc(p, "m3"), out.as_mut_ptr());
        }
        assert_eq!(&out[..9], &m[..]);
        assert_eq!(&out[9..], &[-1.0; 3]);
    }

    #[test]
    fn get_uniform_converts_between_float_and_int() {
        let gl = fresh();
        let p = setter_program(&gl);
        let mut i: GLint = 0;
        let mut f: GLfloat = 0.0;
        // SAFETY: one value each.
        unsafe {
            glUniform1f(loc(p, "f1"), 3.0);
            glUniform1i(loc(p, "i1"), 5);
            glGetUniformiv(p, loc(p, "f1"), &mut i);
            glGetUniformfv(p, loc(p, "i1"), &mut f);
        }
        assert_eq!((i, f), (3, 5.0));
    }

    /* ---- application pointers of any alignment ---- */

    /// A copy of `v` at an odd address, as the setters receive it from an application that cast a byte buffer.
    fn misaligned<T: Copy>(buf: &mut [u8], v: &[T]) -> *const T {
        let n = core::mem::size_of_val(v);
        assert!(buf.len() > n);
        // SAFETY: buf holds n bytes from offset 1.
        unsafe { core::ptr::copy_nonoverlapping(v.as_ptr() as *const u8, buf.as_mut_ptr().add(1), n) };
        buf[1..].as_ptr() as *const T
    }

    #[test]
    fn setters_and_getters_accept_misaligned_application_arrays() {
        let gl = fresh();
        let p = setter_program(&gl);
        let mut inbuf = [0u8; 80];
        let mut outbuf = [0u8; 80];
        let m: Vec<f32> = (0..16).map(|i| i as f32 + 1.0).collect();
        // SAFETY: the misaligned copies hold the values announced; the output holds 16 floats from offset 1.
        unsafe {
            glUniform4fv(loc(p, "v4"), 1, misaligned(&mut inbuf, &[1.5f32, 2.5, 3.5, 4.5]));
            assert_eq!(vs_bits(&gl, p, 8, 4), fbits(&[1.5, 2.5, 3.5, 4.5]));
            glUniform4iv(loc(p, "i4"), 1, misaligned(&mut inbuf, &[-1i32, 2, -3, 4]));
            assert_eq!(vs_bits(&gl, p, 140, 4), [0xffff_ffff, 2, 0xffff_fffd, 4]);
            glUniformMatrix4fv(loc(p, "m4"), 1, GL_FALSE_B, misaligned(&mut inbuf, &m));
            assert_eq!(vs_bits(&gl, p, 60, 16), fbits(&m));

            let out = outbuf.as_mut_ptr().add(1) as *mut GLfloat;
            glGetUniformfv(p, loc(p, "m4"), out);
            for (i, want) in m.iter().enumerate() {
                assert_eq!(f32::from_ne_bytes(outbuf[1 + i * 4..5 + i * 4].try_into().unwrap()), *want);
            }
            let iv = outbuf.as_mut_ptr().add(1) as *mut GLint;
            glGetUniformiv(p, loc(p, "i4"), iv);
            assert_eq!(GLint::from_ne_bytes(outbuf[1..5].try_into().unwrap()), -1);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn program_queries_write_through_misaligned_pointers() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let mut buf = [0u8; 24];
        // SAFETY: the outputs are inside buf, at an odd offset.
        unsafe {
            glGetProgramiv(p, GL_ACTIVE_UNIFORMS, buf.as_mut_ptr().add(1) as *mut GLint);
            assert_eq!(GLint::from_ne_bytes(buf[1..5].try_into().unwrap()), 2);
            let mut name = [0u8; 16];
            glGetActiveUniform(
                p,
                0,
                16,
                buf.as_mut_ptr().add(5) as *mut GLsizei,
                buf.as_mut_ptr().add(9) as *mut GLint,
                buf.as_mut_ptr().add(13) as *mut GLenum,
                name.as_mut_ptr() as *mut GLchar,
            );
            assert_eq!(GLsizei::from_ne_bytes(buf[5..9].try_into().unwrap()), 8, "length of \"viewSize\"");
            assert_eq!(GLint::from_ne_bytes(buf[9..13].try_into().unwrap()), 1, "size");
            assert_eq!(GLenum::from_ne_bytes(buf[13..17].try_into().unwrap()), GL_FLOAT_VEC2, "type");
            let v = gl.ctx().objs().programs[obj_index(p)].vs;
            glGetAttachedShaders(
                p,
                2,
                buf.as_mut_ptr().add(17) as *mut GLsizei,
                buf.as_mut_ptr().add(1) as *mut GLuint,
            );
            assert_eq!(GLsizei::from_ne_bytes(buf[17..21].try_into().unwrap()), 2);
            assert_eq!(GLuint::from_ne_bytes(buf[1..5].try_into().unwrap()), v);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- glGetUniform conversions and single elements ---- */

    #[test]
    fn get_uniformfv_on_an_array_element_reads_that_element() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let v: Vec<f32> = (1..=56).map(|i| i as f32).collect();
        let mut out = [-1.0f32; 8];
        // SAFETY: out is larger than the vec4 read.
        unsafe {
            glUseProgram(p);
            glUniform4fv(loc(p, "frag"), 14, v.as_ptr());
            glGetUniformfv(p, loc(p, "frag[3]"), out.as_mut_ptr());
        }
        assert_eq!(out, [13.0, 14.0, 15.0, 16.0, -1.0, -1.0, -1.0, -1.0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_uniform_of_bools_and_unsigned_values() {
        let gl = fresh();
        let p = setter_program(&gl);
        let mut i: GLint = -7;
        let mut f: GLfloat = -7.0;
        // SAFETY: one value each.
        unsafe {
            glUniform1f(loc(p, "b1"), -0.5); /* any non-zero float is true */
            glGetUniformiv(p, loc(p, "b1"), &mut i);
            assert_eq!(i, 1);
            glGetUniformfv(p, loc(p, "b1"), &mut f);
            assert_eq!(f, 1.0);
            glUniform1i(loc(p, "b1"), 0);
            glGetUniformiv(p, loc(p, "b1"), &mut i);
            assert_eq!((i, vs_bits(&gl, p, 156, 1)), (0, std::vec![0]), "a bool holds 0 or 1");
            glUniform1f(loc(p, "f1"), -2.5);
            glGetUniformiv(p, loc(p, "f1"), &mut i);
            assert_eq!(i, -3, "rounded to the nearest integer (ES 3.0 6.1.2)");
            glUniform1f(loc(p, "f1"), f32::NAN);
            glGetUniformiv(p, loc(p, "f1"), &mut i);
            assert_eq!(i, 0, "NaN has no integer value");
            glUniform1ui(loc(p, "i1"), 0xffff_ffff);
            glGetUniformiv(p, loc(p, "i1"), &mut i);
            assert_eq!(i, -1, "an int uniform keeps its bits");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_uniform_of_a_location_that_is_not_one_is_invalid_operation() {
        let gl = fresh();
        let p = setter_program(&gl);
        let mut out = [-1.0f32; 4];
        // SAFETY: nothing is written on error.
        unsafe {
            glGetUniformfv(p, -1, out.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "-1 is not a location of the program");
            glGetUniformfv(p, -2, out.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
        assert_eq!(out, [-1.0; 4]);
    }

    /* ---- setter checks ---- */

    #[test]
    fn a_sampler_takes_glUniform1i_only() {
        let gl = fresh();
        let p = setter_program(&gl);
        let one: [GLuint; 1] = [1];
        // SAFETY: the array holds the value announced.
        unsafe {
            glUniform1ui(loc(p, "tex"), 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "ES 3.0: samplers are set with the signed setter");
            glUniform1uiv(loc(p, "tex"), 1, one.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1fv(loc(p, "tex"), 1, [1.0f32].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glUniform1i(loc(p, "tex"), 1);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_sampler_unit[3], 1);
    }

    #[test]
    fn a_sampler_unit_out_of_range_changes_nothing() {
        let gl = fresh();
        let p = setter_program(&gl);
        let units: [GLint; 2] = [1, CAP_COMBINED_TEXTURE_UNITS as GLint];
        // SAFETY: the array holds the two values announced.
        unsafe {
            glUniform1i(loc(p, "tex"), 2);
            glUniform1iv(loc(p, "tex"), 1, units.as_ptr().add(1));
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].fs_sampler_unit[3], 2, "a failing command has no effect");
    }

    #[test]
    fn count_zero_after_a_failed_check_still_reports_the_error() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v = [1.0f32; 4];
        // SAFETY: v holds the values announced.
        unsafe {
            glUniform1fv(loc(p, "i1"), 0, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the type is wrong whatever the count");
            glUniform4fv(loc(p, "v4"), -3, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "a negative count is checked first");
        }
    }

    #[test]
    fn array_setters_take_a_count_beyond_the_array_but_not_a_non_array() {
        let gl = fresh();
        let p = setter_program(&gl);
        let v: Vec<f32> = (1..=16).map(|i| i as f32).collect();
        // SAFETY: v holds the values announced.
        unsafe {
            glUniform1fv(loc(p, "fa"), 6, v.as_ptr()); /* 4 elements: the rest is ignored */
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(vs_bits(&gl, p, 12, 5), bits(&[Some(1.0), Some(2.0), Some(3.0), Some(4.0), None]));
            glUniform1fv(loc(p, "fa[2]"), 6, v.as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(vs_bits(&gl, p, 12, 5), bits(&[Some(1.0), Some(2.0), Some(1.0), Some(2.0), None]));
            glUniform1fv(loc(p, "f1"), 2, v.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "f1 is not an array");
        }
    }

    #[test]
    fn plain_array_elements_have_their_own_locations() {
        let gl = fresh();
        let p = setter_program(&gl);
        let l = loc(p, "v3a[2]");
        assert_eq!(l, loc(p, "v3a") | (2 << 8));
        assert_eq!(loc(p, "v3a[3]"), -1, "past the array");
        assert_eq!(loc(p, "v2a[1]"), loc(p, "v2a") | (1 << 8));
        assert_eq!(loc(p, "m3a[1]"), loc(p, "m3a") | (1 << 8));
        // SAFETY: plain GL calls.
        unsafe {
            glUniform3f(l, 7.0, 8.0, 9.0);
            /* v3a: 3 vec3 at dword 24, vec4-strided */
            assert_eq!(vs_bits(&gl, p, 32, 4), bits(&[Some(7.0), Some(8.0), Some(9.0), None]));
            glUniformMatrix3fv(loc(p, "m3a[1]"), 1, GL_FALSE_B, [1.0f32; 9].as_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(vs_bits(&gl, p, 88, 3), fbits(&[1.0, 1.0, 1.0]), "m3a element 1 is 12 dwords in");
        assert_eq!(vs_bits(&gl, p, 76, 1), [SENTINEL], "element 0 untouched");
    }

    /* ---- glGetActiveUniform / glGetActiveAttrib ---- */

    #[test]
    fn get_active_uniform_reports_name_type_and_array_size() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let mut name = [0x55u8; 32];
        let (mut len, mut size, mut type_): (GLsizei, GLint, GLenum) = (-1, -1, 0);
        // SAFETY: name holds the announced 32 bytes.
        unsafe {
            glGetActiveUniform(p, 0, 32, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!((&name[..9], len, size, type_), (&b"viewSize\0"[..], 8, 1, GL_FLOAT_VEC2));
            /* `frag` is declared vec4[14], and GL reports an ARRAY by the name
             * of its element zero. This used to answer "frag", which dEQP
             * fails on an exact string compare against the `name + "[0]"` it
             * builds (es2fUniformApiTests.cpp, "invalid non-built-in uniform
             * name reported") -- and every array uniform was reported that
             * way, not only the struct-array ones task #46 is about. */
            glGetActiveUniform(p, 1, 32, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!((len, size, type_), (7, 14, GL_FLOAT_VEC4));
            assert_eq!(&name[..8], b"frag[0]\0");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_active_uniform_reports_samplers() {
        let gl = fresh();
        let p = gl.program("uv.vert", "texture.frag");
        let mut name = [0x55u8; 32];
        let (mut size, mut type_): (GLint, GLenum) = (-1, 0);
        // SAFETY: name holds the announced 32 bytes; a null length is allowed.
        unsafe {
            glGetActiveUniform(p, 0, 32, core::ptr::null_mut(), &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar)
        };
        assert_eq!((&name[..6], size, type_), (&b"u_tex\0"[..], 1, GL_SAMPLER_2D));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_active_uniform_truncates_the_name() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let mut name = [0x55u8; 8];
        let mut len: GLsizei = -1;
        // SAFETY: only the announced bytes may be written; null size/type are allowed.
        unsafe {
            glGetActiveUniform(
                p,
                0,
                5,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr() as *mut GLchar,
            );
            assert_eq!((len, &name[..6]), (4, &b"view\0\x55"[..]));
            let mut name = [0x55u8; 8];
            glGetActiveUniform(
                p,
                0,
                0,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr() as *mut GLchar,
            );
            assert_eq!((len, name), (0, [0x55; 8]));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own values */
    fn get_active_uniform_errors() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let mut name = [0x55u8; 8];
        let (mut len, mut size, mut type_): (GLsizei, GLint, GLenum) = (-1, -1, 0x55);
        // SAFETY: nothing is written on error.
        unsafe {
            glGetActiveUniform(p, 2, 8, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "index == ACTIVE_UNIFORMS");
            glGetActiveUniform(p, 0, -1, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "negative bufSize");
            glGetActiveUniform(p + 1, 0, 8, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_ne!(gl.error(), GL_NO_ERROR, "no such program");
            let unlinked = glCreateProgram();
            glGetActiveUniform(unlinked, 0, 8, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "an unlinked program has no active uniforms");
        }
        assert_eq!((len, size, type_, name), (-1, -1, 0x55, [0x55; 8]));
    }

    #[test]
    fn get_active_attrib_reports_name_type_and_size_one() {
        let gl = fresh();
        let p = gl.program("uv.vert", "texture.frag");
        let mut name = [0x55u8; 32];
        let (mut len, mut size, mut type_): (GLsizei, GLint, GLenum) = (-1, -1, 0);
        // SAFETY: name holds the announced bytes.
        unsafe {
            glGetActiveAttrib(p, 0, 32, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!((&name[..6], len, size, type_), (&b"a_pos\0"[..], 5, 1, GL_FLOAT_VEC4));
            glGetActiveAttrib(p, 1, 32, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!((&name[..5], len, size, type_), (&b"a_uv\0"[..], 4, 1, GL_FLOAT_VEC2));
            /* truncation */
            let mut name = [0x55u8; 8];
            glGetActiveAttrib(
                p,
                0,
                3,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr() as *mut GLchar,
            );
            assert_eq!((len, &name[..4]), (2, &b"a_\0\x55"[..]));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_active_attrib_errors() {
        let gl = fresh();
        let p = gl.program("uv.vert", "texture.frag");
        let mut name = [0x55u8; 8];
        let (mut len, mut size, mut type_): (GLsizei, GLint, GLenum) = (-1, -1, 0x55);
        // SAFETY: nothing is written on error.
        unsafe {
            glGetActiveAttrib(p, 2, 8, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetActiveAttrib(p, 0, -1, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetActiveAttrib(p + 1, 0, 8, &mut len, &mut size, &mut type_, name.as_mut_ptr() as *mut GLchar);
            assert_ne!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!((len, size, type_, name), (-1, -1, 0x55, [0x55; 8]));
    }

    /* ---- attribute locations ---- */

    #[test]
    fn attrib_location_defaults_to_the_compiled_location() {
        let gl = fresh();
        let p = gl.program("color_uv.vert", "color_uv.frag");
        assert_eq!(attrib_loc(p, "a_pos"), 0);
        assert_eq!(attrib_loc(p, "a_color"), 1);
        assert_eq!(attrib_loc(p, "a_uv"), 2);
        assert_eq!(attrib_loc(p, "a_u"), -1);
        assert_eq!(attrib_loc(p, "a_uvw"), -1);
        assert_eq!(attrib_loc(p, ""), -1);
        // SAFETY: a null name is tolerated.
        assert_eq!(unsafe { glGetAttribLocation(p, core::ptr::null()) }, -1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn attrib_location_on_an_unlinked_program_is_invalid_operation() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        assert_eq!(attrib_loc(p, "a_pos"), -1);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn bind_attrib_location_takes_effect_at_the_next_link() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p);
            bind_attrib(p, 5, "a_color");
            assert_eq!(attrib_loc(p, "a_color"), 1, "not before the relink");
            glLinkProgram(p);
        }
        assert_eq!(attrib_loc(p, "a_color"), 5);
        assert_eq!(attrib_loc(p, "a_pos"), 0);
        assert_eq!(
            gl.ctx().objs().programs[obj_index(p)].attrib_app_index[1],
            5,
            "VS input 1 is fed from generic attribute 5"
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_attrib_location_can_swap_two_attributes() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        bind_attrib(p, 1, "a_pos");
        bind_attrib(p, 0, "a_color");
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        assert_eq!((attrib_loc(p, "a_pos"), attrib_loc(p, "a_color")), (1, 0));
    }

    #[test]
    fn rebinding_a_name_replaces_the_earlier_binding() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        bind_attrib(p, 5, "a_color");
        bind_attrib(p, 9, "a_color");
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].num_attrib_binds, 1);
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(attrib_loc(p, "a_color"), 9);
    }

    #[test]
    fn binding_a_name_the_shader_does_not_have_is_harmless() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        bind_attrib(p, 0, "not_in_the_shader");
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        assert_eq!((attrib_loc(p, "a_pos"), attrib_loc(p, "a_color")), (0, 1));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn two_attributes_bound_to_one_index_alias_and_link() {
        /* ES 2.0 2.10.4 allows aliasing, and a link error is permitted only
         * when EVERY path through the shader consumes several of a set bound
         * together -- which an implementation "is not required" to detect,
         * and which this one cannot: the reflection carries no control flow.
         * This used to fail the link, which refused programs the GL says must
         * link; `attribute_location.bind_aliasing.cond_*` is dEQP holding us
         * to it.
         *
         * Both names then report the index they were bound to, and both
         * inputs read that one vertex attribute. */
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        bind_attrib(p, 3, "a_pos");
        bind_attrib(p, 3, "a_color");
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!(info_log(p), "", "a link that succeeds writes no error");
        assert_eq!((attrib_loc(p, "a_pos"), attrib_loc(p, "a_color")), (3, 3));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn rebinding_and_relinking_moves_the_attributes_and_leaves_a_hole() {
        /* `dEQP-GLES2.functional.attribute_location.bind_relink_hole.*` from
         * the 8b5428a sweep. A binding takes effect at the NEXT link, and the
         * one before it must leave nothing behind: the case binds, links,
         * draws, then rebinds so a different index goes unused ("the hole"),
         * relinks and draws again. A location cached from the first link is
         * what it is built to catch. */
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        bind_attrib(p, 0, "a_pos");
        bind_attrib(p, 2, "a_color"); /* 1 is the hole */
        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!((attrib_loc(p, "a_pos"), attrib_loc(p, "a_color")), (0, 2));

        /* the binding changes; the linked program does NOT, until it relinks */
        bind_attrib(p, 3, "a_pos");
        bind_attrib(p, 1, "a_color"); /* 0 and 2 are the holes now */
        assert_eq!(
            (attrib_loc(p, "a_pos"), attrib_loc(p, "a_color")),
            (0, 2),
            "glBindAttribLocation takes effect at the next link, not at the call"
        );

        // SAFETY: plain GL call.
        unsafe { glLinkProgram(p) };
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1, "{}", info_log(p));
        assert_eq!(
            (attrib_loc(p, "a_pos"), attrib_loc(p, "a_color")),
            (3, 1),
            "the relink moves both, and nothing is left at the old indices"
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_attrib_location_errors() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls; a null name is rejected.
        unsafe {
            bind_attrib(p, MAX_VERTEX_ATTRIBS as GLuint, "a_pos");
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            bind_attrib(p, 0, "gl_Vertex");
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "the gl_ prefix is reserved");
            glBindAttribLocation(p, 0, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            bind_attrib(p + 1, 0, "a_pos");
            assert_ne!(gl.error(), GL_NO_ERROR);
        }
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].num_attrib_binds, 0);
        bind_attrib(p, MAX_VERTEX_ATTRIBS as GLuint - 1, "a_pos");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn one_binding_per_vertex_attribute_is_accepted() {
        let gl = fresh();
        // SAFETY: plain GL call.
        let p = unsafe { glCreateProgram() };
        for i in 0..MAX_VERTEX_ATTRIBS {
            bind_attrib(p, i as GLuint, &std::format!("attr{i}"));
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().objs().programs[obj_index(p)].num_attrib_binds as usize, MAX_VERTEX_ATTRIBS);
    }

    /* ---- glGetProgramiv / glGetProgramInfoLog / glValidateProgram ---- */

    #[test]
    #[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))] /* the PowerVR table's own values */
    fn get_programiv_max_lengths_cover_the_longest_names() {
        let gl = fresh();
        let p = gl.program("83a830e25f56fd85.vert", "0119aeb276db508c.frag");
        let longest_uniform = "ubDirectionalLights.directionalLights[0].dimensionsInverted".len() as GLint;
        assert!(get_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH) > longest_uniform, "includes the NUL");
        assert!(get_iv(p, GL_ACTIVE_ATTRIBUTE_MAX_LENGTH) > "attr_norm".len() as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_programiv_unknown_pname_is_invalid_enum() {
        let gl = fresh();
        // SAFETY: plain GL calls; a null params is tolerated.
        unsafe {
            let p = glCreateProgram();
            let mut v: GLint = -7;
            glGetProgramiv(p, GL_COMPILE_STATUS, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert_eq!(v, -7);
            glGetProgramiv(p, GL_LINK_STATUS, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glGetProgramiv(p + 1, GL_LINK_STATUS, &mut v);
            assert_ne!(gl.error(), GL_NO_ERROR);
            assert_eq!(v, -7);
        }
    }

    #[test]
    fn program_info_log_buffer_semantics() {
        let gl = fresh();
        // SAFETY: buf holds the announced sizes.
        unsafe {
            let p = glCreateProgram();
            glLinkProgram(p);
            let full = info_log(p);
            assert!(full.starts_with("link error: "), "{full}");

            let mut buf = [0x55u8; 16];
            let mut len: GLsizei = -1;
            glGetProgramInfoLog(p, 5, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!((len, &buf[..6]), (4, &b"link\0\x55"[..]));

            let mut buf = [0x55u8; 4];
            glGetProgramInfoLog(p, 0, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!((len, buf), (0, [0x55; 4]));

            glGetProgramInfoLog(p, 4, core::ptr::null_mut(), buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(&buf, b"lin\0");
            assert_eq!(gl.error(), GL_NO_ERROR);

            len = -1;
            glGetProgramInfoLog(p, -1, &mut len, buf.as_mut_ptr() as *mut GLchar);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            assert_eq!(len, -1);
        }
    }

    #[test]
    fn successful_relink_clears_the_info_log() {
        let gl = fresh();
        let (p, _, f) = attached(&gl, "passthrough.vert", "texture.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glLinkProgram(p);
            assert!(get_iv(p, GL_INFO_LOG_LENGTH) > 0);
            glDetachShader(p, f);
            glAttachShader(p, gl.shader(GL_FRAGMENT_SHADER, "interp_color.frag"));
            glLinkProgram(p);
        }
        assert_eq!(get_iv(p, GL_LINK_STATUS), 1);
        assert_eq!(get_iv(p, GL_INFO_LOG_LENGTH), 0);
        assert_eq!(info_log(p), "");
    }

    #[test]
    fn validate_status_follows_the_link_status() {
        let gl = fresh();
        let (p, _, _) = attached(&gl, "passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls.
        unsafe {
            glValidateProgram(p);
            assert_eq!(get_iv(p, GL_VALIDATE_STATUS), 0);
            glLinkProgram(p);
            glValidateProgram(p);
            assert_eq!(get_iv(p, GL_VALIDATE_STATUS), 1);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glValidateProgram(p + 100);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glValidateProgram(p + 1); /* a shader's name */
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    /* ---- the draw sees the blocks ---- */

    #[test]
    fn draw_receives_both_stage_blocks() {
        let gl = fresh();
        let p = gl.program("mvp.vert", "uniform_color.frag");
        let verts: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let m: Vec<f32> = (0..16).map(|i| i as f32 * 0.5).collect();
        // SAFETY: plain GL calls; the client vertex array outlives the draw.
        unsafe {
            use crate::gl::{draw::glDrawArrays, vertex::*};
            glUseProgram(p);
            glUniformMatrix4fv(loc(p, "u_mvp"), 1, GL_FALSE_B, m.as_ptr());
            glUniform4f(loc(p, "u_color"), 0.25, 0.5, 0.75, 1.0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].vs_uniforms, fbits(&m));
        assert_eq!(draws[0].fs_uniforms, fbits(&[0.25, 0.5, 0.75, 1.0]));
    }
    /* ---- upstream 7e76640 -------------------------------------------------------------------------- */

    #[test]
    fn every_es3_sampler_type_and_the_unsigned_types_take_integers() {
        for t in [
            GL_SAMPLER_3D,
            GL_SAMPLER_2D_SHADOW,
            GL_SAMPLER_2D_ARRAY,
            GL_SAMPLER_2D_ARRAY_SHADOW,
            GL_SAMPLER_CUBE_SHADOW,
            GL_UNSIGNED_INT,
            GL_UNSIGNED_INT_VEC2,
            GL_UNSIGNED_INT_VEC3,
            GL_UNSIGNED_INT_VEC4,
        ] {
            assert!(is_int_type(t as u16), "0x{t:x}");
        }
        assert!(!is_int_type(GL_FLOAT as u16));
    }

    /* ---- the sync to upstream e6c449c ------------------------------------ */

    #[test]
    fn a_uniform_under_either_stages_struct_instance_name_is_one_location() {
        /* upstream e0e804c: Qt Quick 3D's shaders declare their uniform block
         * as a struct instance the SPIR-V translation numbers per shader. */
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rvs, std::vec![uni("_69.qt_mvp", 0, 16, GL_FLOAT_MAT4, 1)]);
        set_table(&mut p.rfs, std::vec![uni("_661.qt_mvp", 0, 16, GL_FLOAT_MAT4, 1)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        assert_eq!(tab.count, 1, "one uniform, not two");
        assert_eq!((tab.u[0].vs_off, tab.u[0].fs_off), (0, 0));
        /* either name finds it, and so does the bare one */
        let locs = &tab.u[..tab.count as usize];
        for n in [&b"_69.qt_mvp"[..], b"_661.qt_mvp", b"qt_mvp"] {
            assert_eq!(find_location(&p, locs, n).map(|(i, _)| i), Some(0), "{}", Bytes(n));
        }
    }

    #[test]
    fn only_a_leading_underscore_digits_dot_counts_as_a_struct_instance() {
        assert_eq!(uniform_after_instance(b"_69.color"), b"color");
        assert_eq!(uniform_after_instance(b"light.color"), b"light.color");
        assert_eq!(uniform_after_instance(b"_x.color"), b"_x.color");
        assert_eq!(uniform_after_instance(b"_69color"), b"_69color");
        assert_eq!(uniform_after_instance(b"color"), b"color");
        assert!(!uniform_name_eq(b"light.color", b"fog.color"));
        assert!(uniform_name_eq(b"_1.a", b"_22.a"));
    }

    #[test]
    fn a_vertex_stage_sampler_keeps_its_own_unit() {
        /* upstream 0a57cc6: the unit a glUniform1i names for a vertex-only
         * sampler used to be dropped at link time. */
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rvs, std::vec![sampler("boneTex", 2)]);
        set_table(&mut p.rfs, std::vec![sampler("baseColor", 0)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), 0);
        let vs_loc = tab.u[..tab.count as usize]
            .iter()
            .position(|l| util::name_bytes(&location_ref(&p, l).name) == b"boneTex")
            .expect("the vertex sampler has a location");
        assert_eq!(tab.u[vs_loc].vs_binding1, 3, "binding 2 plus one");
        assert!(!tab.u[vs_loc].fs_sampler);
    }

    #[test]
    fn a_vertex_sampler_binding_beyond_the_sampler_count_fails_the_link() {
        let (mut p, mut tab, mut log) = blank();
        set_table(&mut p.rvs, std::vec![sampler("far", CAP_SAMPLERS as u8)]);
        assert_eq!(build_locations(&mut p, &mut tab, &mut log), -1);
        let text = String::from_utf8(log_text(&log).to_vec()).unwrap();
        assert!(text.contains("'far'"), "{text}");
    }

    /* ---- one name space for shader and program objects (GL ES 2.0 2.10),
     * as dEQP's negative_api.shader.* and negative_api.state.* ask for it. -- */

    /// A name nothing created is GL_INVALID_VALUE; a name of the other kind is
    /// GL_INVALID_OPERATION. Both errors have to exist, so the two kinds
    /// cannot be numbered from 1 each.
    #[test]
    fn a_shader_name_and_a_program_name_are_never_the_same_number() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        let (sh, pr) = unsafe { (glCreateShader(GL_VERTEX_SHADER), glCreateProgram()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_ne!(sh, pr);
        assert!(sh > 1 && pr > 1, "0 and 1 are never handed out: {sh}, {pr}");
        let c = gl.ctx();
        assert!(c.shader_ref(sh).is_some() && c.program_ref(sh).is_none());
        assert!(c.program_ref(pr).is_some() && c.shader_ref(pr).is_none());
    }

    /// The program entry points: a shader name is GL_INVALID_OPERATION, a name
    /// nothing created GL_INVALID_VALUE.
    #[test]
    fn the_program_entry_points_tell_a_shader_name_from_an_unknown_one() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        let sh = unsafe { glCreateShader(GL_VERTEX_SHADER) };
        let unknown = obj_name(MAX_PROGRAMS - 1, OBJ_PROGRAM); /* in range, never created */
        let calls: [(&str, &dyn Fn(GLuint)); 7] = [
            ("glLinkProgram", &|n| unsafe { glLinkProgram(n) }),
            ("glUseProgram", &|n| unsafe { glUseProgram(n) }),
            ("glValidateProgram", &|n| unsafe { glValidateProgram(n) }),
            ("glGetProgramiv", &|n| unsafe {
                let mut iv: GLint = 0;
                glGetProgramiv(n, GL_LINK_STATUS, &mut iv)
            }),
            ("glGetProgramInfoLog", &|n| unsafe {
                let (mut len, mut buf) = (0 as GLsizei, [0 as GLchar; 32]);
                glGetProgramInfoLog(n, 32, &mut len, buf.as_mut_ptr())
            }),
            ("glAttachShader", &|n| unsafe { glAttachShader(n, sh) }),
            ("glBindAttribLocation", &|n| unsafe { glBindAttribLocation(n, 0, c"a".as_ptr()) }),
        ];
        for (name, call) in calls {
            call(sh);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "{name} of a shader name");
            call(unknown);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "{name} of a name nothing created");
        }
    }

    /// The mirror: the shader entry points on a program name.
    #[test]
    fn the_shader_entry_points_tell_a_program_name_from_an_unknown_one() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        let pr = unsafe { glCreateProgram() };
        let unknown = obj_name(super::super::context::MAX_SHADERS - 1, super::super::context::OBJ_SHADER);
        let src = c"void main(){}".as_ptr();
        let calls: [(&str, &dyn Fn(GLuint)); 5] = [
            ("glShaderSource", &|n| unsafe { shader::glShaderSource(n, 1, &src, core::ptr::null()) }),
            ("glCompileShader", &|n| unsafe { shader::glCompileShader(n) }),
            ("glGetShaderiv", &|n| unsafe {
                let mut iv: GLint = 0;
                shader::glGetShaderiv(n, GL_COMPILE_STATUS, &mut iv)
            }),
            ("glGetShaderInfoLog", &|n| unsafe {
                let (mut len, mut buf) = (0 as GLsizei, [0 as GLchar; 32]);
                shader::glGetShaderInfoLog(n, 32, &mut len, buf.as_mut_ptr())
            }),
            ("glGetShaderSource", &|n| unsafe {
                let (mut len, mut buf) = (0 as GLsizei, [0 as GLchar; 32]);
                shader::glGetShaderSource(n, 32, &mut len, buf.as_mut_ptr())
            }),
        ];
        for (name, call) in calls {
            call(pr);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "{name} of a program name");
            call(unknown);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "{name} of a name nothing created");
        }
    }

    /// `uniform_matrixfv_invalid_transpose`: ES 2.0 has no transposing upload
    /// at all, and the argument error comes before the program and location
    /// rules. ES 3.0 added it, so this is what the build reports.
    #[test]
    fn the_matrix_setters_take_transpose_only_where_the_version_has_it() {
        let gl = fresh();
        let p = setter_program(&gl);
        let m = [1.0f32; 16];
        // SAFETY: m holds one matrix.
        unsafe { glUniformMatrix4fv(loc(p, "m4"), 1, GL_TRUE_B, m.as_ptr()) };
        let want = if crate::REPORT_ES2 { GL_INVALID_VALUE } else { GL_NO_ERROR };
        assert_eq!(gl.error(), want);

        /* and it outranks the program: with none current, ES 2.0 still says
         * GL_INVALID_VALUE for the flag it cannot take */
        // SAFETY: plain GL calls; m holds one matrix.
        unsafe {
            glUseProgram(0);
            glUniformMatrix4fv(0, 1, GL_TRUE_B, m.as_ptr());
        }
        let want = if crate::REPORT_ES2 { GL_INVALID_VALUE } else { GL_INVALID_OPERATION };
        assert_eq!(gl.error(), want);
    }

    /* ---- dEQP-GLES2 state_query.shader.*: the object status queries ------- */

    /// `delete_status`: glDeleteShader / glDeleteProgram of an object that is
    /// still in use only FLAGS it (ES 3.0 2.12.1, 2.12.3). The object stays
    /// queryable, and GL_DELETE_STATUS reads GL_TRUE.
    #[test]
    fn a_deleted_but_still_used_object_stays_queryable_and_says_so() {
        let gl = fresh();
        let vs = gl.shader(GL_VERTEX_SHADER, "passthrough.vert");
        let fs = gl.shader(GL_FRAGMENT_SHADER, "uniform_color.frag");
        // SAFETY: plain GL calls on live handles.
        let prog = unsafe {
            let p = glCreateProgram();
            glAttachShader(p, vs);
            glAttachShader(p, fs);
            glLinkProgram(p);
            p
        };
        assert_eq!(get_iv(prog, GL_LINK_STATUS), GL_TRUE as GLint);
        assert_eq!(shader_iv(vs, GL_DELETE_STATUS), GL_FALSE as GLint);
        assert_eq!(shader_iv(fs, GL_DELETE_STATUS), GL_FALSE as GLint);
        assert_eq!(get_iv(prog, GL_DELETE_STATUS), GL_FALSE as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);

        // SAFETY: plain GL calls on live handles.
        unsafe {
            glUseProgram(prog);
            shader::glDeleteShader(vs);
            shader::glDeleteShader(fs);
            glDeleteProgram(prog);
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "deleting an object in use is not an error");
        assert_eq!(shader_iv(vs, GL_DELETE_STATUS), GL_TRUE as GLint, "the vertex shader is still attached");
        assert_eq!(shader_iv(fs, GL_DELETE_STATUS), GL_TRUE as GLint, "the fragment shader is still attached");
        assert_eq!(get_iv(prog, GL_DELETE_STATUS), GL_TRUE as GLint, "the program is still current");
        assert_eq!(gl.error(), GL_NO_ERROR, "all three are still queryable");
    }

    /// `program_validate_status`: GL_VALIDATE_STATUS is what the LAST
    /// glValidateProgram decided, and GL_FALSE until one is called at all
    /// (ES 2.0 2.10 / table 6.20). It is not the link status.
    #[test]
    fn validate_status_is_what_gl_validate_program_decided() {
        let gl = fresh();
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        assert_eq!(get_iv(prog, GL_LINK_STATUS), GL_TRUE as GLint);
        assert_eq!(get_iv(prog, GL_VALIDATE_STATUS), GL_FALSE as GLint, "nothing has validated it yet");

        // SAFETY: plain GL call on a live program.
        unsafe { glValidateProgram(prog) };
        assert_eq!(get_iv(prog, GL_VALIDATE_STATUS), GL_TRUE as GLint, "a linked program validates");
        assert_eq!(gl.error(), GL_NO_ERROR);

        /* a program that never linked cannot validate */
        // SAFETY: plain GL calls.
        let empty = unsafe {
            let p = glCreateProgram();
            glValidateProgram(p);
            p
        };
        assert_eq!(get_iv(empty, GL_VALIDATE_STATUS), GL_FALSE as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);

        /* relinking clears it again: the new program was never validated */
        // SAFETY: plain GL call on a live program.
        unsafe { glLinkProgram(prog) };
        assert_eq!(get_iv(prog, GL_VALIDATE_STATUS), GL_FALSE as GLint, "a relink undoes the validation");
    }

    /// `active_attributes` / the uniform half of the same group:
    /// GL_ACTIVE_ATTRIBUTE_MAX_LENGTH and GL_ACTIVE_UNIFORM_MAX_LENGTH are the
    /// length of the longest name the program actually has, INCLUDING the NUL
    /// -- not the size of the buffer this implementation keeps names in. dEQP
    /// compares them against the names it declared, exactly.
    #[test]
    #[cfg(feature = "gpu-powervr")] /* the PowerVR table's own shaders */
    fn the_max_name_lengths_are_the_longest_name_not_the_buffer() {
        let gl = fresh();
        let p = gl.program("main.vert", "fill_color.frag");
        let mut buf = [0 as GLchar; 128];
        let (mut len, mut size, mut ty) = (0 as GLsizei, 0 as GLint, 0 as GLenum);

        let n_attr = get_iv(p, GL_ACTIVE_ATTRIBUTES);
        let n_unif = get_iv(p, GL_ACTIVE_UNIFORMS);
        assert!(n_attr > 0 && n_unif > 0, "the fixture program has both");

        let mut longest_attr = 0usize;
        for i in 0..n_attr as GLuint {
            // SAFETY: buf holds 128 chars; the out pointers are one value each.
            unsafe { glGetActiveAttrib(p, i, 128, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
            assert_eq!(gl.error(), GL_NO_ERROR);
            longest_attr = longest_attr.max(len as usize);
        }
        let mut longest_unif = 0usize;
        for i in 0..n_unif as GLuint {
            // SAFETY: as above.
            unsafe { glGetActiveUniform(p, i, 128, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
            assert_eq!(gl.error(), GL_NO_ERROR);
            longest_unif = longest_unif.max(len as usize);
        }
        assert!(longest_attr > 0 && longest_unif > 0, "the names are not empty");

        assert_eq!(
            get_iv(p, GL_ACTIVE_ATTRIBUTE_MAX_LENGTH),
            longest_attr as GLint + 1,
            "the longest attribute name plus its NUL"
        );
        assert_eq!(
            get_iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH),
            longest_unif as GLint + 1,
            "the longest uniform name plus its NUL"
        );

        /* a program that has not linked has no active anything, so both are 0 */
        // SAFETY: plain GL call.
        let empty = unsafe { glCreateProgram() };
        assert_eq!(get_iv(empty, GL_ACTIVE_ATTRIBUTE_MAX_LENGTH), 0);
        assert_eq!(get_iv(empty, GL_ACTIVE_UNIFORM_MAX_LENGTH), 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// dEQP `negative_api.shader.get_uniform_location`, the check the board
    /// still fails: a program name that was never generated is
    /// GL_INVALID_VALUE, not GL_INVALID_OPERATION. dEQP passes -2, which
    /// arrives as 0xFFFFFFFE.
    #[test]
    fn a_program_name_never_generated_is_invalid_value_everywhere() {
        let gl = fresh();
        /* -2 as dEQP passes it, and -1, which is the other kind's low bit */
        for name in [0xFFFF_FFFEu32, 0xFFFF_FFFFu32, 12345, 4242] {
            // SAFETY: plain GL calls; the out pointers hold one value each.
            unsafe { glGetUniformLocation(name, c"test".as_ptr()) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glGetUniformLocation({name:#x})");

            // SAFETY: as above.
            unsafe { glGetAttribLocation(name, c"test".as_ptr()) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glGetAttribLocation({name:#x})");

            let mut v: GLfloat = 0.0;
            // SAFETY: `v` holds one value.
            unsafe { glGetUniformfv(name, 0, &mut v) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glGetUniformfv({name:#x})");
        }

        /* and the contrast: a name that IS a live object of the other kind is
         * GL_INVALID_OPERATION, which is the distinction dEQP is checking */
        // SAFETY: plain GL calls.
        let sh = unsafe { crate::gl::shader::glCreateShader(GL_VERTEX_SHADER) };
        // SAFETY: as above.
        unsafe { glGetUniformLocation(sh, c"test".as_ptr()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "a shader name is the other kind");

        /* an existing program that has not linked is GL_INVALID_OPERATION too */
        // SAFETY: plain GL calls.
        let empty = unsafe { glCreateProgram() };
        // SAFETY: as above.
        unsafe { glGetUniformLocation(empty, c"test".as_ptr()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "a program that has not linked");
    }

    /// The whole object-name rule, over every entry point that takes one.
    ///
    /// GL ES 2.0 2.10 splits two errors that are easy to collapse into each
    /// other: a name that was NEVER GENERATED is GL_INVALID_VALUE, while a
    /// generated name of the WRONG KIND is GL_INVALID_OPERATION. dEQP checks
    /// the difference on nearly every one of these, so this is the net.
    #[test]
    fn every_object_name_entry_point_splits_invalid_value_from_invalid_operation() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        let (sh, pr) = unsafe { (shader::glCreateShader(GL_VERTEX_SHADER), glCreateProgram()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* in range but never generated, of each kind, plus dEQP's own -2/-1 */
        let unknown_prog = obj_name(MAX_PROGRAMS - 1, OBJ_PROGRAM);
        let unknown_shad = obj_name(super::super::context::MAX_SHADERS - 1, super::super::context::OBJ_SHADER);
        let src = c"void main(){}".as_ptr();

        /* takes a PROGRAM: a shader name is the wrong kind, an ungenerated
         * name of either kind was never an object at all */
        let prog_calls: [(&str, &dyn Fn(GLuint)); 12] = [
            ("glLinkProgram", &|n| unsafe { glLinkProgram(n) }),
            ("glUseProgram", &|n| unsafe { glUseProgram(n) }),
            ("glValidateProgram", &|n| unsafe { glValidateProgram(n) }),
            ("glDeleteProgram", &|n| unsafe { glDeleteProgram(n) }),
            ("glGetProgramiv", &|n| unsafe {
                let mut v: GLint = 0;
                glGetProgramiv(n, GL_LINK_STATUS, &mut v)
            }),
            ("glGetProgramInfoLog", &|n| unsafe {
                let (mut l, mut b) = (0 as GLsizei, [0 as GLchar; 32]);
                glGetProgramInfoLog(n, 32, &mut l, b.as_mut_ptr())
            }),
            ("glBindAttribLocation", &|n| unsafe { glBindAttribLocation(n, 0, c"a".as_ptr()) }),
            ("glGetAttribLocation", &|n| unsafe {
                glGetAttribLocation(n, c"a".as_ptr());
            }),
            ("glGetUniformLocation", &|n| unsafe {
                glGetUniformLocation(n, c"u".as_ptr());
            }),
            ("glGetActiveAttrib", &|n| unsafe {
                let (mut l, mut sz, mut ty, mut b) = (0 as GLsizei, 0 as GLint, 0 as GLenum, [0 as GLchar; 32]);
                glGetActiveAttrib(n, 0, 32, &mut l, &mut sz, &mut ty, b.as_mut_ptr())
            }),
            ("glGetAttachedShaders", &|n| unsafe {
                let (mut c_, mut out) = (0 as GLsizei, [0 as GLuint; 4]);
                glGetAttachedShaders(n, 4, &mut c_, out.as_mut_ptr())
            }),
            ("glGetUniformfv", &|n| unsafe {
                let mut v: GLfloat = 0.0;
                glGetUniformfv(n, 0, &mut v)
            }),
        ];
        for (name, call) in prog_calls {
            call(sh);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "{name}: a live shader name is the wrong kind");
            for bad in [unknown_prog, unknown_shad, 0xFFFF_FFFE, 0xFFFF_FFFF, 4242] {
                call(bad);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "{name}({bad:#x}): never generated");
            }
        }

        /* takes a SHADER: the mirror */
        let shad_calls: [(&str, &dyn Fn(GLuint)); 6] = [
            ("glShaderSource", &|n| unsafe { shader::glShaderSource(n, 1, &src, core::ptr::null()) }),
            ("glCompileShader", &|n| unsafe { shader::glCompileShader(n) }),
            ("glDeleteShader", &|n| unsafe { shader::glDeleteShader(n) }),
            ("glGetShaderiv", &|n| unsafe {
                let mut v: GLint = 0;
                shader::glGetShaderiv(n, GL_COMPILE_STATUS, &mut v)
            }),
            ("glGetShaderInfoLog", &|n| unsafe {
                let (mut l, mut b) = (0 as GLsizei, [0 as GLchar; 32]);
                shader::glGetShaderInfoLog(n, 32, &mut l, b.as_mut_ptr())
            }),
            ("glGetShaderSource", &|n| unsafe {
                let (mut l, mut b) = (0 as GLsizei, [0 as GLchar; 32]);
                shader::glGetShaderSource(n, 32, &mut l, b.as_mut_ptr())
            }),
        ];
        for (name, call) in shad_calls {
            call(pr);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "{name}: a live program name is the wrong kind");
            for bad in [unknown_prog, unknown_shad, 0xFFFF_FFFE, 0xFFFF_FFFF, 4242] {
                call(bad);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "{name}({bad:#x}): never generated");
            }
        }

        /* the two that take BOTH, one argument at a time */
        // SAFETY: plain GL calls.
        unsafe {
            glAttachShader(unknown_prog, sh);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glAttachShader: the program was never generated");
            glAttachShader(pr, unknown_shad);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glAttachShader: the shader was never generated");
            glAttachShader(sh, sh);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "glAttachShader: a shader name where a program goes");
            glDetachShader(unknown_prog, sh);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glDetachShader: the program was never generated");
            glDetachShader(pr, unknown_shad);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "glDetachShader: the shader was never generated");
        }

        /* glIs* never raise: they answer false for anything that is not one */
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsProgram(unknown_prog), GL_FALSE_B);
            assert_eq!(glIsProgram(sh), GL_FALSE_B, "a shader is not a program");
            assert_eq!(shader::glIsShader(unknown_shad), GL_FALSE_B);
            assert_eq!(shader::glIsShader(pr), GL_FALSE_B, "a program is not a shader");
        }
        assert_eq!(gl.error(), GL_NO_ERROR, "glIs* raise nothing");
    }
}
