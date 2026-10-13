// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The reflection the GL core reads: uniforms, attributes and varyings, in
//! the offline table's own field shapes.
//!
//! This is the half of the front end that has an exact right answer. The
//! table (`tests/reference/vivante_blobs.rs`, `blobs.rs`) already holds a row per
//! uniform for every shader in the set, and those rows are observable through
//! `glGetActiveUniform`, `glGetActiveAttrib` and `glGetAttribLocation` -- so
//! "equal to the table's row" is the acceptance test, not a nice-to-have.
//!
//! # The rules, and where they come from
//!
//! An ES 1.00 shader has no `layout` anywhere; every number below is assigned
//! by a convention the offline generator established and the runtime now
//! depends on. They are transcribed from `tools/vivante/glsl2vivante.py`
//! (`translate_es100_gl`) and `tools/vivante/gen_shader_table.py`
//! (`block_uniforms`, `block_samplers`) -- the code that built the table this
//! has to agree with:
//!
//! * **Attributes** take locations in declaration order, each advancing the
//!   counter by its locations times its array length; `matN` is N.
//! * **Varyings** take locations the same way, from their own counter.
//! * **Samplers** take bindings in declaration order, one each. They are not
//!   in the uniform block; their row carries [`NO_BLOCK`], and every sampler
//!   row comes *after* every block row -- both generators build the list as
//!   `block + samplers` (`gen_shader_table.py` line 372,
//!   `glsl.rs::reflect_uniforms`), and that concatenation is the
//!   `glGetActiveUniform` order an application enumerates.
//! * **Every other uniform** goes in one std430-packed block, in declaration
//!   order. An array's stride is its size rounded up to its alignment.
//! * **A uniform of a struct type** expands to one row per member, named
//!   `outer.member`, each laid out as if written in its place.
//!
//! Declaration order runs down to the declarator: `uniform float a, b;` is
//! `a` then `b`. That order reaches the application as the
//! `glGetActiveUniform` index, so it is behaviour and not presentation
//! (`tools/yakogl-tools/src/emit.rs` documents the same invariant for the
//! PowerVR table).

use super::arena::{AVec, Arena};
use super::ast::*;
use super::{Error, R};

/// `ShcUniform::dword_off` for a uniform that is not in the block -- a
/// sampler. The same value as `SHC_UNIFORM_NO_BLOCK`.
pub const NO_BLOCK: u16 = 0xffff;

/// One row of the uniform reflection, in `ShcUniform`'s fields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UniformRow<'a> {
    /// The name `glGetUniformLocation` answers for: `u_light.pos` for a
    /// struct member.
    pub name: &'a str,
    /// Dword offset in the std430 block, or [`NO_BLOCK`] for a sampler.
    pub dword_off: u16,
    pub dwords: u16,
    pub gl_type: u16,
    pub array_len: u8,
    /// Samplers: the texture unit. Zero for a block member.
    pub binding: u8,
    /// Elements per outer index of a flattened array-of-structs member.
    /// Always 1 until arrays of structs are supported.
    pub inner: u8,
    /// The shader mentions this name somewhere in an expression.
    ///
    /// Only meaningful for samplers, and deliberately *not* the same question
    /// the table answers. The table drops a sampler the code generator's
    /// dead-code pass removed; a front end can only say whether the name
    /// appears at all. Where the two would differ, reporting both is honest
    /// and letting one pass for the other is not.
    pub statically_used: bool,
    /// The declaration is an array, whatever its length. `float f[1]` is an
    /// array of one element, which glGetActiveUniform names `f[0]` (dEQP's
    /// reference names every array `name[0]`, es2fUniformApiTests.cpp
    /// generateBasicUniforms), and `array_len` cannot tell it from `float f`:
    /// both are 1. Not an ABI field; the online back end keeps the table rows
    /// this marks in its record (`backend::keep_blocks`, `ShcBlocks::arrays`).
    pub is_array: bool,
}

/// One row of the attribute reflection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AttribRow<'a> {
    pub location: u8,
    pub name: &'a str,
    pub gl_type: u16,
    pub components: u8,
}

/// One row of the varying reflection.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VaryingRow<'a> {
    pub location: u8,
    pub name: &'a str,
    pub gl_type: u16,
    pub components: u8,
    pub invariant: bool,
    /// Elements of an array varying (`varying float v[4];`), 1 otherwise.
    /// Each element takes its own location(s) from `location` on.
    pub count: u8,
    /// `ir::INTERP_*`: smooth unless ES 3.00 wrote `flat` or `centroid`.
    pub interp: u8,
}

/// One ES 3.00 fragment output (`layout(location = n) out vec4 c;`), the
/// declaration that replaces `gl_FragColor` / `gl_FragData`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OutputRow<'a> {
    pub location: u8,
    pub name: &'a str,
    pub gl_type: u16,
    pub components: u8,
    /// Elements of an array output; each takes the next location.
    pub count: u8,
}

/// Everything the GL core needs to name a shader's interface.
pub struct Reflection<'a> {
    /// In declaration order. Never sorted: the order is the
    /// `glGetActiveUniform` index.
    pub uniforms: &'a [UniformRow<'a>],
    /// In declaration order, which is also ascending location.
    pub attribs: &'a [AttribRow<'a>],
    pub varyings: &'a [VaryingRow<'a>],
    /// ES 3.00 fragment outputs, in declaration order. Empty for ES 1.00.
    pub outputs: &'a [OutputRow<'a>],
    /// Dwords in the std430 uniform block.
    pub uniform_dwords: u16,
    /// ES 3.00 uniform blocks and their members ([`block_table`]); filled
    /// by `lower::compile_to_ir`, which has the unit. Empty for ES 1.00.
    pub blocks: &'a [BlockDesc<'a>],
    pub block_members: &'a [BlockMember<'a>],
}

/// A type's `(dwords, std430 alignment in dwords, GL type token)`.
///
/// The same table as `es100.py`'s `GLSL_TYPES`, which laid out the blocks
/// that are in the offline table today.
fn type_layout(b: &BaseType<'_>) -> Option<(u16, u16, u16)> {
    Some(match b {
        BaseType::Float => (1, 1, 0x1406),
        BaseType::Vec(2) => (2, 2, 0x8B50),
        BaseType::Vec(3) => (3, 4, 0x8B51),
        BaseType::Vec(4) => (4, 4, 0x8B52),
        BaseType::Int => (1, 1, 0x1404),
        BaseType::IVec(2) => (2, 2, 0x8B53),
        BaseType::IVec(3) => (3, 4, 0x8B54),
        BaseType::IVec(4) => (4, 4, 0x8B55),
        BaseType::Uint => (1, 1, 0x1405),
        BaseType::UVec(2) => (2, 2, 0x8DC6),
        BaseType::UVec(3) => (3, 4, 0x8DC7),
        BaseType::UVec(4) => (4, 4, 0x8DC8),
        BaseType::Bool => (1, 1, 0x8B56),
        BaseType::BVec(2) => (2, 2, 0x8B57),
        BaseType::BVec(3) => (3, 4, 0x8B58),
        BaseType::BVec(4) => (4, 4, 0x8B59),
        /* A column is strided to the alignment of a vector of its rows (a
         * vec2 packs, a vec3 pads to four), and there are C of them. */
        BaseType::Mat(c @ 2..=4, r @ 2..=4) => {
            let stride = if *r == 2 { 2 } else { 4 };
            let token = match (c, r) {
                (2, 2) => 0x8B5A,
                (3, 3) => 0x8B5B,
                (4, 4) => 0x8B5C,
                (2, 3) => 0x8B65,
                (2, 4) => 0x8B66,
                (3, 2) => 0x8B67,
                (3, 4) => 0x8B68,
                (4, 2) => 0x8B69,
                _ => 0x8B6A,
            };
            (u16::from(*c) * stride, stride, token)
        }
        BaseType::Sampler(s) => (1, 1, sampler_gl_type(*s)),
        _ => return None,
    })
}

/* ---- uniform blocks: the std140 table ------------------------------------
 *
 * A block's members are ordinary rows of the default uniform block (the
 * parser desugars the block, see `ast::Block`). What the GL core needs on
 * top of the rows is where each member lives in the BUFFER an application
 * binds: the std140 layout of ES 3.0 section 2.12.6.4, which `shared` and
 * `packed` blocks use too (deliberately; docs/online-shaderc/frontend.md).
 * At draw time the GL core reads each member from the bound range at
 * `offset` (+ k * `array_stride`, + column * `matrix_stride`, transposed
 * when `row_major`) and writes it into `row` as glUniform* would.
 */

/// One active uniform block: its GL name (`Block`, or `Block[k]` for an
/// element of an array of blocks), its std140 size in bytes, and its
/// members, a run of [`BlockTable::members`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockDesc<'a> {
    pub name: &'a str,
    pub data_size: u32,
    pub first_member: u16,
    pub num_members: u16,
}

/// One member of a block as GL names it (`Block.m`, `Block.s.x`,
/// `Block.a[1].x`; just `m` for a block with no instance name; `Block.m` in
/// every element of an array of blocks, never `Block[1].m`), where it
/// lives in the buffer (std140, bytes), and the default-block row it fills.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BlockMember<'a> {
    pub name: &'a str,
    /// Index into [`Reflection::uniforms`], and that row's name.
    pub row: u16,
    pub row_name: &'a str,
    pub offset: u32,
    /// Bytes between array elements; 0 for a member that is not an array.
    pub array_stride: u32,
    /// Bytes between columns (rows when `row_major`); 0 for a non-matrix.
    pub matrix_stride: u32,
    pub row_major: bool,
    pub gl_type: u16,
    /// Elements; 1 for a member that is not an array.
    pub array_size: u16,
}

/// Every block of a stage and every member of each.
#[derive(Clone, Copy, Debug)]
pub struct BlockTable<'a> {
    pub blocks: &'a [BlockDesc<'a>],
    pub members: &'a [BlockMember<'a>],
}

fn round_up(x: u32, to: u32) -> u32 {
    x.div_ceil(to) * to
}

/// std140's base alignment and size, in bytes, of one value of `base`
/// (ES 3.0 section 2.12.6.4, rules 1-9). A struct's alignment and size are
/// rounded up to a vec4's.
fn std140_of(ast: &Ast<'_>, base: &BaseType<'_>, row_major: bool, depth: u32) -> Option<(u32, u32)> {
    let vec = |n: u8| -> (u32, u32) {
        match n {
            1 => (4, 4),
            2 => (8, 8),
            3 => (16, 12),
            _ => (16, 16),
        }
    };
    Some(match *base {
        BaseType::Float | BaseType::Int | BaseType::Uint | BaseType::Bool => vec(1),
        BaseType::Vec(n) | BaseType::IVec(n) | BaseType::UVec(n) | BaseType::BVec(n) => vec(n),
        /* Rules 5 and 7: C columns (R rows when row-major), each a vec4-
         * aligned array element. */
        BaseType::Mat(c, r) => (16, 16 * u32::from(if row_major { r } else { c })),
        BaseType::Struct(s) => {
            if depth > 16 {
                return None;
            }
            let ms = struct_members(ast, s)?;
            let (mut end, mut align) = (0u32, 16u32);
            for m in ast.members[ms.indices()].iter() {
                let rm = m.row_major.unwrap_or(row_major);
                let (a, size) = std140_member(ast, &m.ty, rm, depth + 1)?;
                align = align.max(a);
                end = round_up(end, a) + size;
            }
            (align, round_up(end, align))
        }
        _ => return None,
    })
}

/// A member's alignment and size, arrays included (rule 4: an array's
/// elements are vec4-aligned and strided).
fn std140_member(ast: &Ast<'_>, ty: &TypeSpec<'_>, row_major: bool, depth: u32) -> Option<(u32, u32)> {
    let (a, size) = std140_of(ast, &ty.base, row_major, depth)?;
    match ty.array_len {
        None => Some((a, size)),
        Some(len) => {
            let n = u32::try_from(const_int_expr(ast, len)?).ok()?;
            let a = round_up(a, 16);
            Some((a, round_up(size, a) * n))
        }
    }
}

/// The std140 table of every uniform block in `ast`, against the rows of
/// `reflection` (the desugared members' rows, matched by name).
pub fn block_table<'a>(a: &Arena<'a>, ast: &Ast<'a>, reflection: &Reflection<'a>) -> R<'a, BlockTable<'a>> {
    let mut blocks: AVec<'a, BlockDesc<'a>> = AVec::new();
    let mut members: AVec<'a, BlockMember<'a>> = AVec::new();
    for b in ast.blocks {
        let n = match b.array_len {
            Some(e) => u32::try_from(const_int_expr(ast, e).unwrap_or(0)).unwrap_or(0),
            None => 1,
        };
        for k in 0..n {
            let (gl_block, row_prefix) = match (b.instance, b.array_len.is_some()) {
                (Some(inst), true) => (index_name(a, b.name, k)?, Some(index_name(a, inst, k)?)),
                (Some(inst), false) => (b.name, Some(inst)),
                (None, _) => (b.name, None),
            };
            let first = members.len();
            let mut end = 0u32;
            for m in ast.members[b.members.indices()].iter() {
                let rm = m.row_major.unwrap_or(b.row_major);
                let Some((al, size)) = std140_member(ast, &m.ty, rm, 0) else {
                    return Err(Error::new(m.line, m.name, "this block member has no std140 layout here"));
                };
                let off = round_up(end, al);
                end = off + size;
                /* A member's GL name is the block's name without the
                 * element's index (ES 3.0 2.12.6): every element of an array
                 * of blocks names its members `B.m`, one active uniform the
                 * GL core shares between them; the rows stay the element's. */
                let gl = match row_prefix {
                    Some(_) => join_name(a, b.name, m.name)?,
                    None => m.name,
                };
                let row = match row_prefix {
                    Some(p) => join_name(a, p, m.name)?,
                    None => m.name,
                };
                block_leaves(a, ast, reflection, &mut members, gl, row, &m.ty, rm, off, m.line, 0)?;
            }
            blocks.push(
                a,
                BlockDesc {
                    name: gl_block,
                    data_size: round_up(end, 16),
                    first_member: fit_u16(first as u32, b.line, b.name)?,
                    num_members: fit_u16((members.len() - first) as u32, b.line, b.name)?,
                },
            )?;
        }
    }
    Ok(BlockTable { blocks: blocks.into_slice(), members: members.into_slice() })
}

/// The leaves of one block member at buffer offset `off`: a struct is
/// walked member by member (an array of structs element by element), as
/// the reflection names its rows; anything else is one leaf with its row.
#[allow(clippy::too_many_arguments)]
fn block_leaves<'a>(
    a: &Arena<'a>,
    ast: &Ast<'a>,
    reflection: &Reflection<'a>,
    out: &mut AVec<'a, BlockMember<'a>>,
    gl: &'a str,
    row: &'a str,
    ty: &TypeSpec<'a>,
    row_major: bool,
    off: u32,
    line: u32,
    depth: u32,
) -> R<'a, ()> {
    if depth > 16 {
        return Err(Error::new(line, gl, "a block member nested this deeply is not reflected"));
    }
    let count = match ty.array_len {
        Some(e) => u32::try_from(const_int_expr(ast, e).unwrap_or(0)).unwrap_or(0),
        None => 1,
    };
    let Some((al, size)) = std140_of(ast, &ty.base, row_major, depth) else {
        return Err(Error::new(line, gl, "this block member has no std140 layout here"));
    };
    let stride = if ty.array_len.is_some() { round_up(size, round_up(al, 16)) } else { 0 };
    if let BaseType::Struct(s) = ty.base {
        let Some(ms) = struct_members(ast, s) else {
            return Err(Error::new(line, s, "no such struct"));
        };
        for k in 0..count {
            let (egl, erow, eoff) = if ty.array_len.is_some() {
                (index_name(a, gl, k)?, index_name(a, row, k)?, off + k * stride)
            } else {
                (gl, row, off)
            };
            let mut end = eoff;
            for m in ast.members[ms.indices()].iter() {
                let rm = m.row_major.unwrap_or(row_major);
                let Some((mal, msize)) = std140_member(ast, &m.ty, rm, depth + 1) else {
                    return Err(Error::new(m.line, m.name, "this block member has no std140 layout here"));
                };
                let moff = round_up(end, mal);
                end = moff + msize;
                let mgl = join_name(a, egl, m.name)?;
                let mrow = join_name(a, erow, m.name)?;
                block_leaves(a, ast, reflection, out, mgl, mrow, &m.ty, rm, moff, m.line, depth + 1)?;
            }
        }
        return Ok(());
    }
    let Some(k) = reflection.uniforms.iter().position(|u| u.name == row) else {
        return Err(Error::new(line, row, "a block member with no row"));
    };
    let Some((_, _, gl_type)) = type_layout(&ty.base) else {
        return Err(Error::new(line, gl, "unsupported uniform type"));
    };
    out.push(
        a,
        BlockMember {
            name: gl,
            row: fit_u16(k as u32, line, gl)?,
            row_name: row,
            offset: off,
            array_stride: stride,
            matrix_stride: if matches!(ty.base, BaseType::Mat(..)) { 16 } else { 0 },
            row_major: row_major && matches!(ty.base, BaseType::Mat(..)),
            gl_type,
            array_size: fit_u16(count, line, gl)?,
        },
    )?;
    Ok(())
}

/// A sampler type's GL token (ES 3.0 table 2.10).
fn sampler_gl_type(s: SamplerType) -> u16 {
    use SamplerDim::*;
    use SamplerRet::*;
    match (s.ret, s.dim, s.shadow) {
        (Float, D2, false) => 0x8B5E,
        (Float, D3, _) => 0x8B5F,
        (Float, Cube, false) => 0x8B60,
        (Float, D2, true) => 0x8B62,
        (Float, D2Array, false) => 0x8DC1,
        (Float, D2Array, true) => 0x8DC4,
        (Float, Cube, true) => 0x8DC5,
        (Int, D2, _) => 0x8DCA,
        (Int, D3, _) => 0x8DCB,
        (Int, Cube, _) => 0x8DCC,
        (Int, D2Array, _) => 0x8DCF,
        (Uint, D2, _) => 0x8DD2,
        (Uint, D3, _) => 0x8DD3,
        (Uint, Cube, _) => 0x8DD4,
        (Uint, D2Array, _) => 0x8DD7,
    }
}

/// Locations one element of this type occupies (GL ES 2.0 section 2.10.4:
/// one per matrix column, one for anything else).
pub fn locations(b: &BaseType<'_>) -> u32 {
    match b {
        BaseType::Mat(c, _) => *c as u32,
        _ => 1,
    }
}

/// Build the reflection of a parsed shader.
pub fn reflect<'a>(a: &Arena<'a>, ast: &Ast<'a>) -> R<'a, Reflection<'a>> {
    let mut uniforms: AVec<UniformRow<'a>> = AVec::new();
    /* Samplers are gathered apart and appended after the block members,
     * because that is the order the table's rows are in and the order is the
     * `glGetActiveUniform` index. */
    let mut samplers: AVec<UniformRow<'a>> = AVec::new();
    let mut attribs: AVec<AttribRow<'a>> = AVec::new();
    let mut varyings: AVec<VaryingRow<'a>> = AVec::new();
    let mut outputs: AVec<OutputRow<'a>> = AVec::new();
    let (mut attrib_loc, mut varying_loc, mut binding, mut block_dw) = (0u32, 0u32, 0u32, 0u32);
    let mut output_loc = 0u32;

    for d in ast.top_level() {
        let Decl::Var { storage, invariant, io, ty, name, line, .. } = d else {
            continue;
        };
        let (storage, invariant, io, ty, name, line) = (*storage, *invariant, *io, *ty, *name, *line);
        let count = array_len(ast, &ty, name, line)?;

        match storage {
            StorageQualifier::Attribute => {
                let Some((_, _, gl_type)) = type_layout(&ty.base) else {
                    return Err(Error::new(line, name, "unsupported attribute type"));
                };
                /* ES 3.00 `layout(location = n) in`: that location, and the
                 * next unannotated attribute counts on from past it. */
                if let Some(l) = io.location {
                    attrib_loc = l;
                }
                attribs.push(
                    a,
                    AttribRow {
                        location: fit_u8(attrib_loc, line, name)?,
                        name,
                        gl_type,
                        components: ty.base.slot_width(),
                    },
                )?;
                attrib_loc += locations(&ty.base) * count;
            }
            StorageQualifier::FragOut => {
                let Some((_, _, gl_type)) = type_layout(&ty.base) else {
                    return Err(Error::new(line, name, "unsupported fragment output type"));
                };
                if let Some(l) = io.location {
                    output_loc = l;
                }
                outputs.push(
                    a,
                    OutputRow {
                        location: fit_u8(output_loc, line, name)?,
                        name,
                        gl_type,
                        components: ty.base.slot_width(),
                        count: fit_u8(count, line, name)?,
                    },
                )?;
                output_loc += count;
            }
            /* An ES 3.00 structure between the stages: one row per member,
             * named `var.m`, at consecutive locations, all with the
             * structure's interpolation. Both stages name them alike, which
             * is what the link pairs. */
            StorageQualifier::Varying if matches!(ty.base, BaseType::Struct(_)) => {
                let BaseType::Struct(sname) = ty.base else { unreachable!("matched above") };
                let Some(ms) = struct_members(ast, sname) else {
                    return Err(Error::new(line, sname, "no such struct"));
                };
                for m in ast.members[ms.indices()].iter() {
                    let Some((_, _, gl_type)) = type_layout(&m.ty.base) else {
                        return Err(Error::new(m.line, m.name, "unsupported varying type"));
                    };
                    varyings.push(
                        a,
                        VaryingRow {
                            location: fit_u8(varying_loc, line, name)?,
                            name: join_name(a, name, m.name)?,
                            gl_type,
                            components: m.ty.base.slot_width(),
                            invariant,
                            count: 1,
                            interp: match (io.interp, io.centroid) {
                                (Interp::Flat, _) => super::ir::INTERP_FLAT,
                                (_, true) => super::ir::INTERP_CENTROID,
                                _ => super::ir::INTERP_SMOOTH,
                            },
                        },
                    )?;
                    varying_loc += locations(&m.ty.base);
                }
            }
            StorageQualifier::Varying => {
                let Some((_, _, gl_type)) = type_layout(&ty.base) else {
                    return Err(Error::new(line, name, "unsupported varying type"));
                };
                varyings.push(
                    a,
                    VaryingRow {
                        location: fit_u8(varying_loc, line, name)?,
                        name,
                        gl_type,
                        components: ty.base.slot_width(),
                        invariant,
                        count: fit_u8(count, line, name)?,
                        interp: match (io.interp, io.centroid) {
                            (Interp::Flat, _) => super::ir::INTERP_FLAT,
                            (_, true) => super::ir::INTERP_CENTROID,
                            _ => super::ir::INTERP_SMOOTH,
                        },
                    },
                )?;
                varying_loc += locations(&ty.base) * count;
            }
            StorageQualifier::Uniform => {
                if ty.base.is_sampler() {
                    let row = sampler_row(name, &ty.base, count, ty.array_len.is_some(), &mut binding, line)?;
                    samplers.push(a, UniformRow { statically_used: mentions(ast, name), ..row })?;
                    /* The PowerVR target has no hardware array or 3D texture
                     * indexing: its selection offsets the image address by the
                     * layer in the SHADER, and needs the image's layer count,
                     * which the GL core writes into a hidden block member,
                     * `float pvr_layers_<name>`, by name. It sits where
                     * glsl2usc puts it (tools/yakogl-tools/src/glsl.rs) -- at
                     * the sampler's declaration point in the block, so later
                     * uniforms move past it -- and only for a sampler the
                     * shader uses. The GC7000 samples arrays natively and has
                     * no such member. */
                    #[cfg(feature = "gpu-powervr")]
                    if let BaseType::Sampler(st) = ty.base {
                        if matches!(st.dim, SamplerDim::D2Array | SamplerDim::D3) && mentions(ast, name) {
                            /* One hidden layer count per IMAGE, and glsl2usc's
                             * layout for an array of them is not known here:
                             * refused by name rather than guessed. */
                            if count > 1 {
                                return Err(Error::new(
                                    line,
                                    name,
                                    "an array of 2D-array or 3D samplers is not reflected on the PowerVR yet",
                                ));
                            }
                            let hidden = prefixed(a, "pvr_layers_", name)?;
                            let fty = TypeSpec {
                                base: BaseType::Float,
                                precision: Precision::High,
                                array_len: None,
                                implicit_size: false,
                            };
                            block_dw = push_block_member(a, &mut uniforms, block_dw, hidden, &fty, 1, line)?;
                        }
                    }
                    continue;
                }
                if let BaseType::Struct(sname) = ty.base {
                    /* An array of structs is its elements, each flattened
                     * like a lone struct under `name[k]` -- the names GL ES
                     * 2.0 section 2.10.4 gives them ("u_lights[3].position")
                     * and glGetUniformLocation answers for. It was refused
                     * as "an array of structs is not reflected" until
                     * 2026-09-23 (257 dEQP cases, most of uniform_api and
                     * light_amount). */
                    if ty.array_len.is_some() {
                        for k in 0..count {
                            let el = index_name(a, name, k)?;
                            block_dw = push_struct_members(
                                a,
                                ast,
                                &mut uniforms,
                                (&mut samplers, &mut binding),
                                block_dw,
                                el,
                                sname,
                                line,
                                0,
                            )?;
                        }
                        continue;
                    }
                    block_dw = push_struct_members(
                        a,
                        ast,
                        &mut uniforms,
                        (&mut samplers, &mut binding),
                        block_dw,
                        name,
                        sname,
                        line,
                        0,
                    )?;
                    continue;
                }
                block_dw = push_block_member(a, &mut uniforms, block_dw, name, &ty, count, line)?;
            }
            /* A `const` is folded and an unqualified global is internal:
             * neither is part of the shader's interface. */
            StorageQualifier::None | StorageQualifier::Const => {}
            /* `resolve_io` has given every ES 3.00 `in` and `out` its
             * meaning before this runs. */
            StorageQualifier::In | StorageQualifier::Out => {
                return Err(Error::new(line, name, "an 'in' or 'out' the front end did not resolve"));
            }
        }
    }

    /* Section 7.5's `gl_DepthRange`, when the shader reads it: three highp
     * floats after the application's uniforms, named as the struct's members
     * so a member access lowers like any uniform struct's. The GL core fills
     * them from glDepthRangef before each draw (`gl::draw`,
     * fill_depth_range_uniforms). */
    if mentions(ast, DEPTH_RANGE) {
        let fty = TypeSpec { base: BaseType::Float, precision: Precision::High, array_len: None, implicit_size: false };
        for m in DEPTH_RANGE_MEMBERS {
            block_dw = push_block_member(a, &mut uniforms, block_dw, m, &fty, 1, 0)?;
        }
    }

    let sampler_rows = samplers.into_slice();
    uniforms.extend_from_slice(a, sampler_rows)?;

    Ok(Reflection {
        uniforms: uniforms.into_slice(),
        attribs: attribs.into_slice(),
        varyings: varyings.into_slice(),
        outputs: outputs.into_slice(),
        uniform_dwords: fit_u16(block_dw, 0, "")?,
        blocks: &[],
        block_members: &[],
    })
}

/// The built-in uniform `gl_DepthRange` and the rows it becomes, in the
/// order section 7.5 declares the members.
pub const DEPTH_RANGE: &str = "gl_DepthRange";
pub const DEPTH_RANGE_MEMBERS: [&str; 3] = crate::shaderc::SHC_DEPTH_RANGE_ROWS;

/// A struct uniform's members as rows named by their GL names (`s.a`,
/// `s.b.c`), in declaration order, recursing into a member that is itself a
/// struct -- which was refused as "a nested struct is not reflected" until
/// 2026-09-23 (dEQP linkage.uniform_struct_partial_vec4_struct). The rows
/// are the leaves; an intermediate struct has no row of its own, as GL has
/// no location for one. Returns the new block size.
#[allow(clippy::too_many_arguments)]
fn push_struct_members<'a>(
    a: &Arena<'a>,
    ast: &Ast<'a>,
    out: &mut AVec<'a, UniformRow<'a>>,
    samplers: (&mut AVec<'a, UniformRow<'a>>, &mut u32),
    mut block_dw: u32,
    prefix: &'a str,
    sname: &'a str,
    line: u32,
    depth: u32,
) -> R<'a, u32> {
    /* A struct cannot contain itself (a member's type must already be
     * declared), so the depth is bounded by the unit; this bounds the stack
     * against a checker gap rather than the language. */
    if depth > 16 {
        return Err(Error::new(line, sname, "a struct nested this deeply is not reflected"));
    }
    let Some(members) = struct_members(ast, sname) else {
        return Err(Error::new(line, sname, "no such struct"));
    };
    for m in ast.members[members.indices()].iter() {
        let full = join_name(a, prefix, m.name)?;
        if let BaseType::Struct(inner) = m.ty.base {
            if m.ty.array_len.is_some() {
                /* A member that is an array of structs: the same, one
                 * level down (`s.arr[1].x`). */
                let mcount = array_len(ast, &m.ty, m.name, m.line)?;
                for k in 0..mcount {
                    let el = index_name(a, full, k)?;
                    block_dw = push_struct_members(
                        a,
                        ast,
                        out,
                        (&mut *samplers.0, &mut *samplers.1),
                        block_dw,
                        el,
                        inner,
                        m.line,
                        depth + 1,
                    )?;
                }
                continue;
            }
            block_dw = push_struct_members(
                a,
                ast,
                out,
                (&mut *samplers.0, &mut *samplers.1),
                block_dw,
                full,
                inner,
                m.line,
                depth + 1,
            )?;
            continue;
        }
        /* A sampler member is a SAMPLER ROW -- a binding, not block dwords --
         * named by its path (`s.c`), as GL makes every struct member its own
         * active uniform. It was placed in the block until 2026-09-23 and
         * the look-up that named it refused ("reading this uniform type"). */
        if m.ty.base.is_sampler() {
            let mcount = array_len(ast, &m.ty, m.name, m.line)?;
            let row = sampler_row(full, &m.ty.base, mcount, m.ty.array_len.is_some(), samplers.1, m.line)?;
            samplers.0.push(a, row)?;
            continue;
        }
        let mcount = array_len(ast, &m.ty, m.name, m.line)?;
        block_dw = push_block_member(a, out, block_dw, full, &m.ty, mcount, m.line)?;
    }
    Ok(block_dw)
}

/// Place one non-sampler uniform in the std430 block; returns the new block
/// size.
fn push_block_member<'a>(
    a: &Arena<'a>,
    out: &mut AVec<'a, UniformRow<'a>>,
    block_dw: u32,
    name: &'a str,
    ty: &TypeSpec<'a>,
    count: u32,
    line: u32,
) -> R<'a, u32> {
    let Some((size, align, gl_type)) = type_layout(&ty.base) else {
        return Err(Error::new(line, name, "unsupported uniform type"));
    };
    let (size, align) = (size as u32, align as u32);
    /* An array's elements are strided to their alignment; a lone value is
     * packed at its own size. */
    let stride = if ty.array_len.is_some() { size.div_ceil(align) * align } else { size };
    let off = block_dw.div_ceil(align) * align;
    out.push(
        a,
        UniformRow {
            name,
            dword_off: fit_u16(off, line, name)?,
            dwords: fit_u16(stride * count, line, name)?,
            gl_type,
            array_len: fit_u8(count, line, name)?,
            binding: 0,
            inner: 1,
            statically_used: true,
            is_array: ty.array_len.is_some(),
        },
    )?;
    Ok(off + stride * count)
}

/// One uniform as DECLARED, for the rule that joins the two stages.
///
/// The rows above are what the GL core can see, and they say too little for
/// the link: a struct member the stage never reads is still a row, but a
/// member one stage's struct HAS and the other's lacks, or one written
/// `highp` here and `mediump` there, leaves no trace in them. dEQP's
/// `linkage.uniform_struct_type_conflict_1` and
/// `uniform_struct_precision_conflict_1..4` linked for exactly that reason.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UniformDecl<'a> {
    pub name: &'a str,
    /// The whole type, spelled with every precision resolved against the
    /// defaults in force where it was declared, and a struct spelled with
    /// its name and every member: `struct S{highp float a;lowp vec3 b[2];}`.
    /// Two stages agree on a uniform exactly when these are equal -- except
    /// for the precision of a plain uniform, which is [`Self::precision`].
    pub ty: &'a str,
    /// A plain (non-struct) uniform's resolved precision; `Default` for a
    /// type that has none (`bool`), and for a struct, whose members carry
    /// theirs inside [`Self::ty`].
    pub precision: Precision,
    /// The shader mentions the name in an expression.
    pub used: bool,
}

/// The precision a declaration of `base` gets in `stage` when it writes
/// none, under the defaults `float` and `int` (and the samplers', which
/// nothing here lets a shader change) currently have.
fn resolved(p: Precision, base: &BaseType<'_>, float: Precision, int: Precision) -> Precision {
    if p != Precision::Default {
        return p;
    }
    match base {
        BaseType::Float | BaseType::Vec(_) | BaseType::Mat(..) => float,
        /* ES 3.00 section 4.5.4: `uint` shares `int`'s default. */
        BaseType::Int | BaseType::IVec(_) | BaseType::Uint | BaseType::UVec(_) => int,
        /* Section 4.5.3 (ES 3.00 4.5.4): both languages default
         * `sampler2D` and `samplerCube` to lowp, and no other sampler. */
        &BaseType::SAMPLER_2D | &BaseType::SAMPLER_CUBE => Precision::Low,
        _ => Precision::Default,
    }
}

fn precision_word(p: Precision) -> &'static str {
    match p {
        Precision::Default => "",
        Precision::Low => "lowp ",
        Precision::Medium => "mediump ",
        Precision::High => "highp ",
    }
}

fn base_word(b: &BaseType<'_>) -> &'static str {
    match b {
        BaseType::Void => "void",
        BaseType::Float => "float",
        BaseType::Int => "int",
        BaseType::Bool => "bool",
        BaseType::Vec(2) => "vec2",
        BaseType::Vec(3) => "vec3",
        BaseType::Vec(_) => "vec4",
        BaseType::IVec(2) => "ivec2",
        BaseType::IVec(3) => "ivec3",
        BaseType::IVec(_) => "ivec4",
        BaseType::Uint => "uint",
        BaseType::UVec(2) => "uvec2",
        BaseType::UVec(3) => "uvec3",
        BaseType::UVec(_) => "uvec4",
        BaseType::BVec(2) => "bvec2",
        BaseType::BVec(3) => "bvec3",
        BaseType::BVec(_) => "bvec4",
        BaseType::Mat(2, 2) => "mat2",
        BaseType::Mat(3, 3) => "mat3",
        BaseType::Mat(4, 4) => "mat4",
        BaseType::Mat(2, 3) => "mat2x3",
        BaseType::Mat(2, 4) => "mat2x4",
        BaseType::Mat(3, 2) => "mat3x2",
        BaseType::Mat(3, 4) => "mat3x4",
        BaseType::Mat(4, 2) => "mat4x2",
        BaseType::Mat(..) => "mat4x3",
        BaseType::Sampler(s) => s.name(),
        BaseType::Struct(_) => "struct",
    }
}

/// Append `s` to a spelling under construction.
fn put<'a>(a: &Arena<'a>, out: &mut AVec<'a, u8>, s: &str) -> R<'a, ()> {
    Ok(out.extend_from_slice(a, s.as_bytes())?)
}

/// `[n]` for an array, nothing otherwise.
fn put_array<'a>(a: &Arena<'a>, ast: &Ast<'a>, out: &mut AVec<'a, u8>, ty: &TypeSpec<'a>) -> R<'a, ()> {
    if let Some(e) = ty.array_len {
        /* A size the checker could not fold never reaches here: sema
         * refused the shader first. */
        let mut n = const_int_expr(ast, e).map_or(0, |n| n.max(0) as u64);
        let mut digits = [0u8; 20];
        let mut d = digits.len();
        loop {
            d -= 1;
            digits[d] = b'0' + (n % 10) as u8;
            n /= 10;
            if n == 0 {
                break;
            }
        }
        put(a, out, "[")?;
        out.extend_from_slice(a, &digits[d..])?;
        put(a, out, "]")?;
    }
    Ok(())
}

/// Every uniform of the unit as declared: see [`UniformDecl`].
///
/// The precision defaults are those of section 4.5.3 for `stage` -- the
/// vertex language `highp` for float and int, the fragment language
/// `mediump` for int and none for float (the type checker has already
/// refused a float with neither) -- updated by each top-level `precision`
/// statement in source order, so a struct's members resolve against the
/// defaults in force where the struct was declared.
pub fn uniform_decls<'a>(a: &Arena<'a>, ast: &Ast<'a>, stage: super::sema::Stage) -> R<'a, &'a [UniformDecl<'a>]> {
    let vertex = stage == super::sema::Stage::Vertex;
    let mut float = if vertex { Precision::High } else { Precision::Default };
    let mut int = if vertex { Precision::High } else { Precision::Medium };
    /* Each struct's spelling, made where it was declared. */
    let mut structs: AVec<(&'a str, &'a str)> = AVec::new();
    let mut out: AVec<UniformDecl<'a>> = AVec::new();

    for d in ast.top_level() {
        match *d {
            Decl::Precision { precision, base, .. } => match base {
                BaseType::Float => float = precision,
                BaseType::Int => int = precision,
                _ => {}
            },
            Decl::Struct { name, members, .. } => {
                let mut s: AVec<u8> = AVec::new();
                put(a, &mut s, "struct ")?;
                put(a, &mut s, name)?;
                put(a, &mut s, "{")?;
                for m in &ast.members[members.indices()] {
                    if let BaseType::Struct(inner) = m.ty.base {
                        let spelled = structs.as_slice().iter().rev().find(|(n, _)| *n == inner).map_or(inner, |s| s.1);
                        put(a, &mut s, spelled)?;
                    } else {
                        put(a, &mut s, precision_word(resolved(m.ty.precision, &m.ty.base, float, int)))?;
                        put(a, &mut s, base_word(&m.ty.base))?;
                    }
                    put(a, &mut s, " ")?;
                    put(a, &mut s, m.name)?;
                    put_array(a, ast, &mut s, &m.ty)?;
                    put(a, &mut s, ";")?;
                }
                put(a, &mut s, "}")?;
                // SAFETY: built from `&str` pieces only, so it is UTF-8.
                let spelled = unsafe { core::str::from_utf8_unchecked(s.into_slice()) };
                structs.push(a, (name, spelled))?;
            }
            Decl::Var { storage: StorageQualifier::Uniform, ty, name, .. } => {
                let mut s: AVec<u8> = AVec::new();
                let precision = if let BaseType::Struct(sname) = ty.base {
                    let spelled = structs.as_slice().iter().rev().find(|(n, _)| *n == sname).map_or(sname, |s| s.1);
                    put(a, &mut s, spelled)?;
                    Precision::Default
                } else {
                    put(a, &mut s, base_word(&ty.base))?;
                    resolved(ty.precision, &ty.base, float, int)
                };
                put_array(a, ast, &mut s, &ty)?;
                // SAFETY: as above.
                let spelled = unsafe { core::str::from_utf8_unchecked(s.into_slice()) };
                out.push(a, UniformDecl { name, ty: spelled, precision, used: mentions(ast, name) })?;
            }
            _ => {}
        }
    }
    Ok(out.into_slice())
}

/// A stage's interface as the link needs it: its reflection, its uniforms
/// as declared, and the checked tree the static-use questions are asked of.
pub struct Interface<'a> {
    pub reflection: Reflection<'a>,
    pub uniforms: &'a [UniformDecl<'a>],
    ast: Ast<'a>,
}

impl Interface<'_> {
    /// Whether the shader mentions `name` in an expression after
    /// preprocessing: a static use (GLSL ES 1.00 Appendix A 7 counts a
    /// varying only if "statically used in both shaders", and a fragment
    /// built-in "If referenced in the fragment shader (after
    /// preprocessing)").
    pub fn uses(&self, name: &str) -> bool {
        mentions(&self.ast, name)
    }

    /// Parsed as GLSL ES 3.00.
    pub fn es3(&self) -> bool {
        self.ast.es3
    }
}

/// The variables GLSL ES 1.00 Appendix A 7 counts against the varying
/// budget of the pair `vs` + `fs`:
///
/// ```text
/// If referenced in the fragment shader (after preprocessing), the built-in
/// special variables (gl_FragCoord, gl_FrontFacing and gl_PointCoord) are
/// included when calculating the storage requirements of varyings.
/// Only varyings statically used in both shaders are counted.
/// ```
///
/// A varying is taken with the fragment shader's type (the link refuses a
/// pair whose declarations differ); gl_FrontFacing is a bool, one component.
pub fn packed_varyings<'i>(
    vs: &'i Interface<'_>,
    fs: &'i Interface<'_>,
) -> impl Iterator<Item = crate::shaderc::packing::Var> + Clone + 'i {
    use crate::shaderc::packing::{Kind, Var};
    let shared =
        fs.reflection.varyings.iter().filter(move |f| {
            fs.uses(f.name) && vs.reflection.varyings.iter().any(|v| v.name == f.name) && vs.uses(f.name)
        });
    let builtins = [("gl_FragCoord", Kind::Vec4), ("gl_FrontFacing", Kind::Float), ("gl_PointCoord", Kind::Vec2)];
    shared
        .map(|f| Var::new(Kind::of_gl_type(u32::from(f.gl_type)).unwrap_or(Kind::Vec4), u32::from(f.count).max(1)))
        .chain(builtins.into_iter().filter(move |(name, _)| fs.uses(name)).map(|(_, kind)| Var::new(kind, 1)))
}

/// The varying budget of a GLSL ES 1.00 pair: [`packed_varyings`] packed by
/// Appendix A 7 ([`crate::shaderc::packing`]) on `rows` rows, the
/// MAX_VARYING_VECTORS of the GPU the program is for. The one rule both the
/// in-process link and the host front end (`yakogl-tools` es100.rs) apply.
///
/// An ES 3.00 pair passes: GLSL ES 3.00 section 11 counts the outputs and
/// the inputs apart and keeps flat and smooth values in different rows,
/// which is not this algorithm, and ES 3.0 work is on hold.
pub fn varying_budget(
    vs: &Interface<'_>,
    fs: &Interface<'_>,
    rows: usize,
) -> Result<(), crate::shaderc::packing::Unfit> {
    use crate::shaderc::packing::{check_rows, Budget};
    if vs.es3() || fs.es3() {
        return Ok(());
    }
    check_rows(Budget::Varyings, packed_varyings(vs, fs), rows)
}

/// Run the front half over `src` -- preprocess, parse, check, reflect -- for
/// the link, which has the two sources and needs what the compiled shaders do
/// not carry: every varying the vertex shader DECLARES (written or not), and
/// every uniform's declared type.
pub fn interface<'a>(a: &Arena<'a>, stage: super::sema::Stage, src: &'a str) -> R<'a, Interface<'a>> {
    let pre = super::pp::preprocess(a, src)?;
    let ast = super::parser::resolve_io(a, super::parser::parse(a, pre.tokens)?, stage)?;
    super::sema::check(a, &ast, stage)?;
    let reflection = reflect(a, &ast)?;
    let uniforms = uniform_decls(a, &ast, stage)?;
    Ok(Interface { reflection, uniforms, ast })
}

/// Texture units a stage's samplers can take: bindings 0..15, the size of
/// the GL core's per-stage table (`context::CAP_SAMPLERS`).
pub const MAX_SAMPLER_BINDINGS: u32 = 16;

/// The row of a sampler, or of an array of `count` samplers, taking the
/// next bindings from `*next`.
///
/// An array is ONE row, `array_len` elements, whose element k reads binding
/// `binding + k` -- the shape the GL core's uniform table already handles
/// (`glUniform1iv` on `s[k]` sets unit `binding + k`) and the one
/// `glGetActiveUniform` reports as `s[0]` of size N. It was refused as "an
/// array of samplers is not reflected" until 2026-09-23 (79 dEQP
/// `uniform_api` cases). An array that would run past the last binding is
/// refused here, by name, rather than wrap onto another sampler's unit.
fn sampler_row<'a>(
    name: &'a str,
    base: &BaseType<'a>,
    count: u32,
    is_array: bool,
    next: &mut u32,
    line: u32,
) -> R<'a, UniformRow<'a>> {
    let Some((_, _, gl_type)) = type_layout(base) else {
        return Err(Error::new(line, name, "unsupported sampler type"));
    };
    if *next + count > MAX_SAMPLER_BINDINGS {
        return Err(Error::new(line, name, "the samplers need more than the 16 texture bindings a stage has"));
    }
    let row = UniformRow {
        name,
        dword_off: NO_BLOCK,
        dwords: fit_u16(count, line, name)?,
        gl_type,
        array_len: fit_u8(count, line, name)?,
        binding: fit_u8(*next, line, name)?,
        inner: 1,
        statically_used: true,
        is_array,
    };
    *next += count;
    Ok(row)
}

/// The members of the struct named `name`, if the unit declares one.
fn struct_members(ast: &Ast<'_>, name: &str) -> Option<Range> {
    ast.decls.iter().find_map(|d| match d {
        Decl::Struct { name: n, members, .. } if *n == name => Some(*members),
        _ => None,
    })
}

/// The front end's reflection as the `ShcUniform` rows the GL core reads.
///
/// **The set is filtered here, the numbering is not.** The front end reports
/// every declared sampler with `statically_used`, because it can only answer
/// "statically referenced"; a sampler the program does not use gets no
/// descriptor, so it must not appear. Block members are copied unfiltered:
/// nothing prunes them, and their order is load-bearing because
/// `glGetActiveUniform` indices follow it.
pub fn uniform_rows<'a>(
    r: &Reflection<'_>,
    scratch: &crate::shaderc::online::arena::Arena<'a>,
) -> Result<&'a mut [crate::shaderc::ShcUniform], crate::shaderc::online::arena::ArenaError> {
    let keep_it = |u: &UniformRow<'_>| u.dword_off != crate::shaderc::SHC_UNIFORM_NO_BLOCK || u.statically_used;
    let n = r.uniforms.iter().filter(|u| keep_it(u)).count();
    let out = scratch.alloc_filled(n, crate::shaderc::ShcUniform::ZERO)?;
    for (dst, u) in out.iter_mut().zip(r.uniforms.iter().filter(|u| keep_it(u))) {
        crate::util::set_name(&mut dst.name, u.name.as_bytes());
        dst.dword_off = u.dword_off;
        dst.dwords = u.dwords;
        dst.gl_type = u.gl_type;
        dst.array_len = u.array_len;
        dst.binding = u.binding;
        dst.inner = u.inner;
    }
    Ok(out)
}

/// `prefix` followed by `name`, in the arena.
#[cfg(feature = "gpu-powervr")]
fn prefixed<'a>(a: &Arena<'a>, prefix: &str, name: &str) -> R<'a, &'a str> {
    let buf = a.alloc_filled::<u8>(prefix.len() + name.len(), 0)?;
    buf[..prefix.len()].copy_from_slice(prefix.as_bytes());
    buf[prefix.len()..].copy_from_slice(name.as_bytes());
    core::str::from_utf8(buf).map_err(|_| Error::new(0, "", "a uniform name is not valid UTF-8"))
}

/// `outer.member`, in the arena.
pub fn join_name<'a>(a: &Arena<'a>, outer: &str, member: &str) -> R<'a, &'a str> {
    let n = outer.len() + 1 + member.len();
    let buf = a.alloc_filled::<u8>(n, 0)?;
    buf[..outer.len()].copy_from_slice(outer.as_bytes());
    buf[outer.len()] = b'.';
    buf[outer.len() + 1..].copy_from_slice(member.as_bytes());
    /* Both halves are GLSL identifiers, so the result is ASCII. */
    core::str::from_utf8(buf).map_err(|_| Error::new(0, "", "a uniform name is not valid UTF-8"))
}

/// `name[k]`, in the arena: the GL name of an element of an array.
pub fn index_name<'a>(a: &Arena<'a>, name: &str, k: u32) -> R<'a, &'a str> {
    let mut digits = [0u8; 10];
    let (mut n, mut d) = (k, 0usize);
    loop {
        digits[d] = b'0' + (n % 10) as u8;
        d += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    let len = name.len() + 2 + d;
    let buf = a.alloc_filled::<u8>(len, 0)?;
    buf[..name.len()].copy_from_slice(name.as_bytes());
    buf[name.len()] = b'[';
    for j in 0..d {
        buf[name.len() + 1 + j] = digits[d - 1 - j];
    }
    buf[len - 1] = b']';
    core::str::from_utf8(buf).map_err(|_| Error::new(0, "", "a uniform name is not valid UTF-8"))
}

/// A declaration's element count: its array length, or 1.
fn array_len<'a>(ast: &Ast<'a>, ty: &TypeSpec<'a>, name: &'a str, line: u32) -> R<'a, u32> {
    let Some(e) = ty.array_len else {
        return Ok(1);
    };
    match const_int_expr(ast, e) {
        Some(n) if n > 0 => Ok(n as u32),
        Some(_) => Err(Error::new(line, name, "an array size must be greater than zero")),
        None => Err(Error::new(line, name, "an array size must be a constant expression")),
    }
}

/// A folded constant: GLSL ES 1.00's constant expressions are integer or
/// floating point, and which one it is decides whether it may size an array.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Const {
    Int(i64),
    Float(f64),
}

impl Const {
    fn as_f64(self) -> f64 {
        match self {
            Const::Int(v) => v as f64,
            Const::Float(v) => v,
        }
    }
}

/// Fold a constant integer expression, or `None` if it is not one. The entry
/// point `sema.rs` shares, so that an array size means the same thing to the
/// type checker and to the reflection.
///
/// Only an *integer* constant may size an array (section 4.1.9), so a folded
/// float is rejected here even though it folded: `float a[2.0]` is an error,
/// and `float a[int(max(-1.0, 2.0))]` is not.
pub(super) fn const_int_expr(ast: &Ast<'_>, e: ExprId) -> Option<i64> {
    match const_eval(ast, e, 0)? {
        Const::Int(v) => Some(v),
        Const::Float(_) => None,
    }
}

/// The value of a section 7.4 built-in constant, if `name` is one.
///
/// "Implementation dependent constants": each is the number this build's GL
/// core reports for the matching query, read from the same Rust constant so
/// the two cannot drift -- dEQP's shaders.builtin_variable.max_* compare the
/// shader's value with `glGetIntegerv`.
///
/// The two languages list different constants (`es3`):
/// - ES 1.00 has `gl_MaxVaryingVectors`. Its `gl_MaxDrawBuffers` is 1 when
///   the GL reports ES 2.0 (`report-es2`), and `GL_MAX_DRAW_BUFFERS`
///   when it reports ES 3.0: dEQP-GLES2 builtin_variable.max_draw_buffers
///   expects the query's value on an ES3-compatible context and 1 only
///   otherwise (es2fShaderBuiltinVarTests.cpp). It was 1 always until
///   2026-09-24 and failed both cases on the PowerVR (3). `gl_FragData`
///   stays one element in ES 1.00: the ES 1.00 lowering writes only
///   `gl_FragData[0]`, and an index past it is refused rather than dropped.
/// - ES 3.00 section 7.3 drops `gl_MaxVaryingVectors` and adds
///   `gl_MaxVertexOutputVectors`, `gl_MaxFragmentInputVectors` and the texel
///   offset range; its `gl_MaxDrawBuffers` is `GL_MAX_DRAW_BUFFERS`, the
///   back end's `max_draw_buffers()` (`backend::draw_targets`: 3 on every
///   GPU today, below ES 3.0's minimum of 4; see RESULTS.md's gap list).
pub fn builtin_constant(name: &str, es3: bool) -> Option<i64> {
    use crate::gl::context::{
        CAP_COMBINED_TEXTURE_UNITS, CAP_TEXTURE_UNITS, MAX_FRAGMENT_UNIFORM_VECTORS, MAX_VARYING_VECTORS,
        MAX_VERTEX_ATTRIBS, MAX_VERTEX_UNIFORM_VECTORS,
    };
    Some(match name {
        "gl_MaxVertexAttribs" => MAX_VERTEX_ATTRIBS as i64,
        "gl_MaxVertexUniformVectors" => MAX_VERTEX_UNIFORM_VECTORS as i64,
        "gl_MaxFragmentUniformVectors" => MAX_FRAGMENT_UNIFORM_VECTORS as i64,
        "gl_MaxVertexTextureImageUnits" => crate::backend::GLES_VS_TEX_UNITS as i64,
        "gl_MaxCombinedTextureImageUnits" => CAP_COMBINED_TEXTURE_UNITS as i64,
        "gl_MaxTextureImageUnits" => CAP_TEXTURE_UNITS as i64,
        "gl_MaxVaryingVectors" if !es3 => MAX_VARYING_VECTORS as i64,
        "gl_MaxDrawBuffers" if !es3 && crate::REPORT_ES2 => 1,
        /* Section 7.3 defines them from the component limits. */
        "gl_MaxVertexOutputVectors" if es3 => (crate::gl::context::MAX_VERTEX_OUTPUT_COMPONENTS / 4) as i64,
        "gl_MaxFragmentInputVectors" if es3 => (crate::gl::context::MAX_FRAGMENT_INPUT_COMPONENTS / 4) as i64,
        "gl_MaxDrawBuffers" => crate::gl::context::max_draw_buffers() as i64,
        "gl_MinProgramTexelOffset" if es3 => crate::gl::context::MIN_PROGRAM_TEXEL_OFFSET as i64,
        "gl_MaxProgramTexelOffset" if es3 => crate::gl::context::MAX_PROGRAM_TEXEL_OFFSET as i64,
        _ => return None,
    })
}

/// The section 7.4 (ES 1.00) / 7.3 (ES 3.00) constant names of one
/// language, for the checker to declare.
pub const BUILTIN_CONSTANTS_ES100: &[&str] = &[
    "gl_MaxVertexAttribs",
    "gl_MaxVertexUniformVectors",
    "gl_MaxVaryingVectors",
    "gl_MaxVertexTextureImageUnits",
    "gl_MaxCombinedTextureImageUnits",
    "gl_MaxTextureImageUnits",
    "gl_MaxFragmentUniformVectors",
    "gl_MaxDrawBuffers",
];
pub const BUILTIN_CONSTANTS_ES300: &[&str] = &[
    "gl_MaxVertexAttribs",
    "gl_MaxVertexUniformVectors",
    "gl_MaxVertexOutputVectors",
    "gl_MaxFragmentInputVectors",
    "gl_MaxVertexTextureImageUnits",
    "gl_MaxCombinedTextureImageUnits",
    "gl_MaxTextureImageUnits",
    "gl_MaxFragmentUniformVectors",
    "gl_MaxDrawBuffers",
    "gl_MinProgramTexelOffset",
    "gl_MaxProgramTexelOffset",
];

/// Fold a constant expression (section 5.10): literals, `const` variables,
/// the unary and binary arithmetic operators, the scalar constructors, and a
/// built-in call whose arguments are all constant expressions.
///
/// That last clause is the specification's, and it is not a nicety: dEQP's
/// `constant_expressions.array` sizes an array with
/// `int(max(-1.0, 2.0))` and expects the shader to build.
fn const_eval(ast: &Ast<'_>, e: ExprId, depth: u32) -> Option<Const> {
    if depth > 32 {
        return None;
    }
    match ast.expr(e) {
        Expr::IntLit(v) => Some(Const::Int(*v as i64)),
        /* A uint folds to its value: it may size an array (ES 3.00 section
         * 4.1.9, "an integral constant expression"). */
        Expr::UintLit(v) => Some(Const::Int(i64::from(*v))),
        Expr::FloatLit(v) => Some(Const::Float(*v as f64)),
        Expr::BoolLit(b) => Some(Const::Int(i64::from(*b))),
        Expr::Ident(name) if builtin_constant(name, ast.es3).is_some() => {
            builtin_constant(name, ast.es3).map(Const::Int)
        }
        Expr::Ident(name) => {
            /* A `const` initialised by another constant expression. ES 1.00
             * requires a name to be declared before use and the parser
             * accepted the shader, so searching every declaration cannot find
             * a later one that shadows this. */
            let init = ast.decls.iter().find_map(|d| match d {
                Decl::Var { storage: StorageQualifier::Const, name: n, init: Some(i), .. } if n == name => Some(*i),
                _ => None,
            })?;
            const_eval(ast, init, depth + 1)
        }
        Expr::Unary(op, x) => {
            let v = const_eval(ast, *x, depth + 1)?;
            match (op, v) {
                (UnOp::Plus, v) => Some(v),
                (UnOp::Minus, Const::Int(v)) => Some(Const::Int(-v)),
                (UnOp::Minus, Const::Float(v)) => Some(Const::Float(-v)),
                (UnOp::Not, Const::Int(v)) => Some(Const::Int(i64::from(v == 0))),
                _ => None,
            }
        }
        Expr::Binary(op, l, r) => {
            let (l, r) = (const_eval(ast, *l, depth + 1)?, const_eval(ast, *r, depth + 1)?);
            match (l, r) {
                (Const::Int(a), Const::Int(b)) => Some(Const::Int(match op {
                    BinOp::Add => a.wrapping_add(b),
                    BinOp::Sub => a.wrapping_sub(b),
                    BinOp::Mul => a.wrapping_mul(b),
                    BinOp::Div if b != 0 => a.wrapping_div(b),
                    _ => return None,
                })),
                _ => {
                    let (a, b) = (l.as_f64(), r.as_f64());
                    Some(Const::Float(match op {
                        BinOp::Add => a + b,
                        BinOp::Sub => a - b,
                        BinOp::Mul => a * b,
                        BinOp::Div if b != 0.0 => a / b,
                        _ => return None,
                    }))
                }
            }
        }
        Expr::Call { name, args } => const_call(ast, name, *args, depth),
        _ => None,
    }
}

/// A constructor or built-in call in a constant expression.
fn const_call(ast: &Ast<'_>, name: &str, args: Range, depth: u32) -> Option<Const> {
    let mut vals = [Const::Int(0); 3];
    if args.len as usize > vals.len() {
        return None;
    }
    for (slot, i) in vals.iter_mut().zip(args.indices()) {
        *slot = const_eval(ast, ast.expr_ids[i], depth + 1)?;
    }
    let n = args.len as usize;
    match (name, n) {
        /* The scalar constructors. `int(x)` truncates towards zero
         * (section 5.4.1). */
        ("int", 1) => Some(Const::Int(match vals[0] {
            Const::Int(v) => v,
            Const::Float(v) => v as i64,
        })),
        /* Only an ES 3.00 source has a `uint` constructor, and a negative
         * operand's value is not folded: its bit pattern is what `uint(-1)`
         * means, and nothing here needs one. */
        ("uint", 1) if ast.es3 => match vals[0] {
            Const::Int(v) if v >= 0 => Some(Const::Int(v)),
            Const::Float(v) if v >= 0.0 => Some(Const::Int(v as i64)),
            _ => None,
        },
        ("float", 1) => Some(Const::Float(vals[0].as_f64())),
        ("bool", 1) => Some(Const::Int(i64::from(vals[0].as_f64() != 0.0))),
        /* The scalar built-ins that fold. A texture lookup never does
         * (section 5.10), and the transcendentals are left out because
         * nothing needs them and a constant that differs from the hardware's
         * answer would be worse than not folding. */
        ("abs", 1) | ("sign", 1) | ("floor", 1) | ("ceil", 1) => {
            let v = vals[0];
            Some(match (name, v) {
                ("abs", Const::Int(x)) => Const::Int(x.abs()),
                ("abs", Const::Float(x)) => Const::Float(x.abs()),
                ("sign", Const::Int(x)) => Const::Int(x.signum()),
                ("sign", Const::Float(x)) => Const::Float(if x > 0.0 {
                    1.0
                } else if x < 0.0 {
                    -1.0
                } else {
                    0.0
                }),
                ("floor", v) => Const::Float(floor_f64(v.as_f64())),
                ("ceil", v) => Const::Float(-floor_f64(-v.as_f64())),
                _ => return None,
            })
        }
        ("min", 2) | ("max", 2) => {
            let want_max = name == "max";
            Some(match (vals[0], vals[1]) {
                (Const::Int(a), Const::Int(b)) => Const::Int(if (a > b) == want_max { a } else { b }),
                (a, b) => {
                    let (a, b) = (a.as_f64(), b.as_f64());
                    Const::Float(if (a > b) == want_max { a } else { b })
                }
            })
        }
        ("clamp", 3) => {
            let (x, lo, hi) = (vals[0], vals[1], vals[2]);
            Some(match (x, lo, hi) {
                (Const::Int(x), Const::Int(lo), Const::Int(hi)) => Const::Int(x.max(lo).min(hi)),
                _ => Const::Float(x.as_f64().max(lo.as_f64()).min(hi.as_f64())),
            })
        }
        _ => None,
    }
}

/// `floor` without a libm: `core` has none for `f64` on a bare-metal target.
fn floor_f64(v: f64) -> f64 {
    let t = v as i64 as f64;
    if v < 0.0 && t != v {
        t - 1.0
    } else {
        t
    }
}

/// Does any expression in the unit name `name`?
///
/// A deliberate over-approximation of "the shader uses this": it counts a
/// mention anywhere, dead code included. It answers for samplers, where the
/// table records the code generator's post-dead-code answer and a front end
/// cannot have one.
fn mentions(ast: &Ast<'_>, name: &str) -> bool {
    ast.exprs.iter().any(|e| matches!(e, Expr::Ident(n) if *n == name))
}

fn fit_u8<'a>(v: u32, line: u32, name: &'a str) -> R<'a, u8> {
    u8::try_from(v).map_err(|_| Error::new(line, name, "too many of this kind of declaration"))
}

fn fit_u16<'a>(v: u32, line: u32, name: &'a str) -> R<'a, u16> {
    u16::try_from(v).map_err(|_| Error::new(line, name, "this declaration is too large"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shaderc::online::{parser, pp};

    fn reflect_src<'a>(buf: &'a mut [u8], src: &'a str) -> R<'a, Reflection<'a>> {
        let a = Arena::new(buf);
        let pre = pp::preprocess(&a, src)?;
        let ast = parser::parse(&a, pre.tokens)?;
        reflect(&a, &ast)
    }

    /// gl_MaxVertexTextureImageUnits is the query's answer on the GC7000,
    /// 16 (dEQP builtin_variable.max_vertex_texture_image_units compares the
    /// two), in either language version.
    #[cfg(feature = "gpu-vivante")]
    #[test]
    fn the_vertex_unit_constant_is_the_gc7000s_sixteen() {
        let both = [false, true].map(|es3| builtin_constant("gl_MaxVertexTextureImageUnits", es3));
        assert_eq!(both, [Some(16), Some(16)]);
    }

    fn ok<'a>(buf: &'a mut [u8], src: &'a str) -> Reflection<'a> {
        match reflect_src(buf, src) {
            Ok(r) => r,
            Err(e) => panic!("expected success, got line {} at {:?}: {}", e.line, e.tok, e.msg),
        }
    }

    type Row = (std::string::String, u16, u16, u16, u8, u8);

    fn rows(r: &Reflection<'_>) -> std::vec::Vec<Row> {
        r.uniforms
            .iter()
            .map(|u| (u.name.to_string(), u.dword_off, u.dwords, u.gl_type, u.array_len, u.binding))
            .collect()
    }

    fn row(name: &str, off: u16, dwords: u16, gl_type: u16, len: u8, binding: u8) -> Row {
        (name.to_string(), off, dwords, gl_type, len, binding)
    }

    #[test]
    fn a_block_is_packed_std430_in_declaration_order() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "uniform float a;\nuniform vec3 b;\nuniform vec2 c;\nuniform mat4 m;\nvoid main() {}");
        assert_eq!(
            rows(&r),
            [
                row("a", 0, 1, 0x1406, 1, 0),
                /* vec3 aligns to 4, so it starts at 4 and occupies 3. */
                row("b", 4, 3, 0x8B51, 1, 0),
                /* vec2 aligns to 2: 7 rounds up to 8. */
                row("c", 8, 2, 0x8B50, 1, 0),
                row("m", 12, 16, 0x8B5C, 1, 0),
            ]
        );
        assert_eq!(r.uniform_dwords, 28);
    }

    /// An array's stride is its size rounded up to its alignment, so a
    /// `vec3[2]` takes eight dwords and not six.
    #[test]
    fn an_array_strides_to_its_alignment() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "uniform vec3 v[2];\nuniform float f[3];\nvoid main() {}");
        assert_eq!(rows(&r), [row("v", 0, 8, 0x8B51, 2, 0), row("f", 8, 3, 0x1406, 3, 0)]);
    }

    /// The FemtoVG family sizes its array with a `#define`, which the
    /// preprocessor has already turned into a literal; `const int` is the
    /// other spelling and needs folding.
    #[test]
    fn an_array_size_may_come_from_a_define_or_a_const() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "#define N 14\nuniform vec4 frag[N];\nvoid main() {}");
        assert_eq!(rows(&r), [row("frag", 0, 56, 0x8B52, 14, 0)]);

        let mut buf2 = [0u8; 1 << 18];
        let r = ok(&mut buf2, "const int N = 2;\nuniform vec4 v[N * 2];\nvoid main() {}");
        assert_eq!(rows(&r), [row("v", 0, 16, 0x8B52, 4, 0)]);
    }

    /// Section 5.10: a constant expression may contain a built-in call whose
    /// arguments are all constant expressions. dEQP's
    /// `constant_expressions.array` sizes an array with
    /// `int(max(-1.0, 2.0))` and expects the shader to BUILD, so a folder
    /// that stops at literals and operators refuses a valid shader.
    #[test]
    fn a_constant_expression_may_contain_a_builtin_call() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "uniform vec4 v[int(max(-1.0, 2.0))];\nvoid main() {}");
        assert_eq!(rows(&r), [row("v", 0, 8, 0x8B52, 2, 0)]);

        let mut buf2 = [0u8; 1 << 18];
        let r = ok(&mut buf2, "uniform float f[int(clamp(9.0, 1.0, 3.0))];\nvoid main() {}");
        assert_eq!(rows(&r), [row("f", 0, 3, 0x1406, 3, 0)]);

        /* A float constant expression is still not an array size: only an
         * integer one is (section 4.1.9). */
        let mut buf3 = [0u8; 1 << 18];
        let Err(e) = reflect_src(&mut buf3, "uniform float f[2.0];\nvoid main() {}") else {
            panic!("a float is not an array size");
        };
        assert_eq!(e.msg, "an array size must be a constant expression");
    }

    #[test]
    fn a_size_that_is_not_constant_is_refused() {
        let mut buf = [0u8; 1 << 18];
        let Err(e) = reflect_src(&mut buf, "uniform int n;\nuniform vec4 v[n];\nvoid main() {}") else {
            panic!("a uniform is not a constant expression");
        };
        assert_eq!(e.msg, "an array size must be a constant expression");
    }

    #[test]
    fn samplers_take_bindings_in_declaration_order_and_are_not_in_the_block() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(
            &mut buf,
            "uniform sampler2D s0;\nuniform vec4 c;\nuniform samplerCube s1;\n\
             void main() { vec4 x = texture2D(s0, vec2(0.0)); }",
        );
        assert_eq!(
            rows(&r),
            [
                /* Block members first, in declaration order, then the
                 * samplers: the order both offline generators build
                 * (`block + samplers`), and the order an application
                 * enumerates with glGetActiveUniform. */
                row("c", 0, 4, 0x8B52, 1, 0),
                row("s0", NO_BLOCK, 1, 0x8B5E, 1, 0),
                row("s1", NO_BLOCK, 1, 0x8B60, 1, 1),
            ],
            "bindings count samplers only, and the block offset ignores them"
        );
        assert_eq!(r.uniform_dwords, 4);
        let used: std::vec::Vec<bool> = r.uniforms.iter().map(|u| u.statically_used).collect();
        assert_eq!(used, [true, true, false], "s1 is declared and never mentioned");
    }

    #[test]
    fn a_struct_uniform_expands_to_one_row_per_member() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "struct Light { vec3 pos; float k; mat4 m; };\nuniform Light u_light;\nvoid main() {}");
        assert_eq!(
            rows(&r),
            [
                row("u_light.pos", 0, 3, 0x8B51, 1, 0),
                row("u_light.k", 3, 1, 0x1406, 1, 0),
                row("u_light.m", 4, 16, 0x8B5C, 1, 0),
            ]
        );
        assert_eq!(r.uniform_dwords, 20);
    }

    /// `matN` occupies N locations, so the attribute after one starts N
    /// higher. Getting this wrong puts every later attribute at the wrong
    /// vertex-input register.
    #[test]
    fn locations_advance_by_the_types_location_count() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(
            &mut buf,
            "attribute vec4 a_pos;\nattribute mat3 a_m;\nattribute vec2 a_uv;\n\
             varying vec2 v_uv;\nvarying vec4 v_c;\nvoid main() {}",
        );
        let a: std::vec::Vec<(u8, &str)> = r.attribs.iter().map(|x| (x.location, x.name)).collect();
        assert_eq!(a, [(0, "a_pos"), (1, "a_m"), (4, "a_uv")], "mat3 takes locations 1, 2 and 3");
        let v: std::vec::Vec<(u8, &str)> = r.varyings.iter().map(|x| (x.location, x.name)).collect();
        assert_eq!(v, [(0, "v_uv"), (1, "v_c")], "varyings have their own counter");
    }

    /// Declaration order runs down to the declarator, because that order is
    /// the `glGetActiveUniform` index the application sees.
    #[test]
    fn one_declaration_list_keeps_its_order() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "uniform float a, b, c;\nvoid main() {}");
        let names: std::vec::Vec<&str> = r.uniforms.iter().map(|u| u.name).collect();
        assert_eq!(names, ["a", "b", "c"]);
    }

    #[test]
    fn locals_and_consts_are_not_part_of_the_interface() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "const float K = 2.0;\nfloat g;\nuniform vec4 c;\nvoid main() { float l; }");
        let names: std::vec::Vec<&str> = r.uniforms.iter().map(|u| u.name).collect();
        assert_eq!(names, ["c"]);
    }

    /// An array of samplers is one row whose element k reads binding
    /// `binding + k`, and the samplers after it start past its last
    /// element -- at top level and as a struct member alike.
    #[test]
    fn an_array_of_samplers_takes_one_binding_per_element() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(
            &mut buf,
            "precision mediump float;\nuniform sampler2D a;\nuniform sampler2D s[3];\n\
             struct T { samplerCube c[2]; sampler2D d; };\nuniform T t;\nvoid main() {}",
        );
        let rows: std::vec::Vec<(&str, u8, u8, u16)> =
            r.uniforms.iter().map(|u| (u.name, u.binding, u.array_len, u.dword_off)).collect();
        assert_eq!(
            rows,
            [("a", 0, 1, NO_BLOCK), ("s", 1, 3, NO_BLOCK), ("t.c", 4, 2, NO_BLOCK), ("t.d", 6, 1, NO_BLOCK)]
        );
    }

    /// Past the 16th binding is refused by name, not wrapped onto another
    /// sampler's unit.
    #[test]
    fn samplers_past_the_last_binding_are_refused() {
        let mut buf = [0u8; 1 << 18];
        assert!(ok(&mut buf, "uniform sampler2D s[16];\nvoid main() {}").uniforms[0].array_len == 16);
        let mut buf = [0u8; 1 << 18];
        let Err(e) = reflect_src(&mut buf, "uniform sampler2D a;\nuniform sampler2D s[16];\nvoid main() {}") else {
            panic!("17 bindings reflected");
        };
        assert!(e.msg.contains("16 texture bindings"), "{}", e.msg);
    }

    /// A sampler inside a struct is a sampler ROW named by its path, with a
    /// binding of its own, after the block members -- not block dwords
    /// (dEQP shaders.struct.uniform.sampler_*, uniform_api *sampler*).
    #[test]
    fn a_sampler_member_is_a_sampler_row() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(
            &mut buf,
            "uniform sampler2D t;\nstruct S { float a; sampler2D c; vec3 b; };\nuniform S s;\nvoid main() {}",
        );
        let rows: std::vec::Vec<(&str, bool, u8)> =
            r.uniforms.iter().map(|u| (u.name, u.dword_off == NO_BLOCK, u.binding)).collect();
        assert_eq!(rows, [("s.a", false, 0), ("s.b", false, 0), ("t", true, 0), ("s.c", true, 1)]);
    }

    /// An array of structs is its elements, each flattened under `name[k]`
    /// -- the GL names (`s[1].a`) -- whether it is the uniform itself or a
    /// member of one. It was refused as "an array of structs is not
    /// reflected" until 2026-09-23.
    #[test]
    fn an_array_of_structs_is_flattened_per_element() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(&mut buf, "struct S { float a; vec2 b; };\nuniform S s[2];\nvoid main() {}");
        let names: std::vec::Vec<&str> = r.uniforms.iter().map(|u| u.name).collect();
        assert_eq!(names, ["s[0].a", "s[0].b", "s[1].a", "s[1].b"]);
        let mut buf2 = [0u8; 1 << 18];
        let r2 = ok(&mut buf2, "struct I { float a; };\nstruct O { I i[2]; float c; };\nuniform O o;\nvoid main() {}");
        let names2: std::vec::Vec<&str> = r2.uniforms.iter().map(|u| u.name).collect();
        assert_eq!(names2, ["o.i[0].a", "o.i[1].a", "o.c"]);
    }

    /// A struct member that is a struct is flattened to its leaves, named by
    /// their GL names (`o.i.c`), in declaration order: the rows dEQP sets by
    /// name (linkage.uniform_struct_partial_vec4_struct). It was refused as
    /// "a nested struct is not reflected" until 2026-09-23.
    #[test]
    fn a_nested_struct_is_flattened_to_its_leaves() {
        let mut buf = [0u8; 1 << 18];
        let r = ok(
            &mut buf,
            "struct I { vec4 c; float d; };\nstruct O { vec4 a; I i; float e; };\nuniform O o;\nvoid main() {}",
        );
        let rows: std::vec::Vec<(&str, u16)> = r.uniforms.iter().map(|u| (u.name, u.dword_off)).collect();
        assert_eq!(rows, [("o.a", 0), ("o.i.c", 4), ("o.i.d", 8), ("o.e", 9)]);
    }
}
