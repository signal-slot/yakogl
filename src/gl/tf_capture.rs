// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! OpenGL ES 3.0 transform feedback capture, the data path
//! (`docs/es3/transform-feedback.md` section 4, item tf-capture): the GL core
//! evaluates the program's vertex shader on the CPU for every vertex the
//! draw captures, assembles the draw's primitives into Begin's primitive
//! type, and writes the captured varyings into the bound buffers. The GPU
//! still draws the draw as it would without transform feedback (or not at
//! all under rasterizer discard): no back end sees the capture, so it is
//! the same on the three GPUs, none of which has a capture path this
//! library can use (the document's section 3).
//!
//! The vertex shader runs as the online compiler's IR on its reference
//! evaluator (`shaderc/online/eval.rs`, `run_in`), lowered again from the
//! source the link kept (`tf.rs`, `TfProg::src`): the IR is not kept past a
//! compile, and the lowering is cheap next to a draw that captures. Its
//! inputs are the draw's: each attribute array element converted as the
//! draw converts it (`draw::attrib_element`), the generic value of a
//! disabled array, `gl_VertexID` and `gl_InstanceID`, and the uniform values
//! of the program's vertex block (uniform blocks and gl_DepthRange filled
//! in first, as a draw fills them), moved by name from the linked stage's
//! layout to the IR's.
//!
//! Primitive assembly is Gallium's decomposition with the last vertex
//! provoking (`draw_decompose_tmp.h`, Mesa 26.2.1, lines 79-160), the order
//! `draw.rs`'s `restart_to_lists` gives as well: a line strip's line k is
//! (k, k+1) and a loop closes with (n-1, 0); a triangle strip's triangle k
//! is (k, k+1, k+2), or (k+1, k, k+2) for odd k; a fan's is (0, k+1, k+2).
//! Each vertex is evaluated once per instance (a strip's are read from a
//! small cache) and its record written once per primitive it is part of.
//!
//! Texture look-ups read the GL texture storage on the CPU ([`GlTexels`],
//! the evaluator's `TexelSource`): the texture bound to the unit the
//! sampler's glUniform1i names, with the sampler object's filters and wraps
//! when one is bound, as the draw binds it (`draw.rs` `build_common`). The
//! filtering is ES 3.0.6 section 3.8.10-3.8.11 on the texels as stored:
//! NEAREST and LINEAR, the four mipmap filters, REPEAT / CLAMP_TO_EDGE /
//! MIRRORED_REPEAT, seamless cube maps (3.8.9.1), GL_TEXTURE_MIN_LOD /
//! MAX_LOD / MAX_LEVEL, GL_TEXTURE_BASE_LEVEL (the chain starts there; the
//! draw does not apply it) and the texture swizzle. A vertex shader's
//! `texture()` reads level of detail 0; `textureGrad` derives it from the
//! gradients (equations 3.17-3.19). A shadow look-up compares the reference
//! with each depth texel the filter reads and filters the results (3.8.15).
//! An incomplete texture reads (0, 0, 0, 1) (3.8.13).
//!
//! What it refuses (GL_INVALID_OPERATION, the reason in the log; nothing is
//! counted and the draw is not drawn): a compressed texture or a storage
//! class it does not decode (named), a sampler whose return type does not
//! match the texture's (an `isampler2D` on an RGBA8 texture: undefined in
//! GL), a shadow sampler on a texture that is not a depth texture or does
//! not compare (undefined in GL), storage with no CPU view, an attribute
//! element outside its buffer, a uniform the evaluated shader reads that the
//! linked stage does not lay out the same way, and anything the evaluator
//! itself stops at. ES 3.0 has no such
//! error; `src/GL_CORE_NOTES.md` lists it.

use super::consts::*;
use super::context::{obj_index, GlContext, GlTexture, CAP_MIP_LEVELS, MAX_VERTEX_ATTRIBS, TEXTURE_SLOTS};
use super::es3_formats::Texel;
use super::tf::TfRow;
use super::types::*;
use crate::backend::{
    self, GLES_TEXFMT_A8, GLES_TEXFMT_L8, GLES_TEXFMT_LA8, GLES_TEXFMT_R11G11B10F, GLES_TEXFMT_R16F, GLES_TEXFMT_R32F,
    GLES_TEXFMT_RGB10A2, GLES_TEXFMT_RGB10A2UI, GLES_TEXFMT_RGB565, GLES_TEXFMT_RGB9E5, GLES_TEXFMT_RGBA16F,
    GLES_TEXFMT_RGBA16I, GLES_TEXFMT_RGBA16UI, GLES_TEXFMT_RGBA32F, GLES_TEXFMT_RGBA32I, GLES_TEXFMT_RGBA32UI,
    GLES_TEXFMT_RGBA8, GLES_TEXFMT_RGBA8I, GLES_TEXFMT_RGBA8UI, GLES_TEXFMT_RGBA8_SNORM, GLES_TEXFMT_SRGB8_A8,
};
use crate::shaderc::online::arena::{AVec, Arena};
use crate::shaderc::online::eval::{self, Fault, Frame, Inputs, Sink, TexLevel, TexLookup, TexelSource};
use crate::shaderc::online::fmath;
use crate::shaderc::online::ir::{IoSlot, Module, Op, ScalarType, TexDim};
use crate::shaderc::online::{lower, sema::Stage, store};
use crate::shaderc::SHC_UNIFORM_NO_BLOCK;
use crate::util;

/// One buffer a captured draw writes: its slot, the byte where this draw's
/// first vertex goes (the binding's offset plus what was written since
/// Begin), and the bytes one vertex takes.
#[derive(Clone, Copy)]
pub(super) struct Target {
    pub slot: GLuint,
    pub at: usize,
    pub stride: u32,
}

impl Target {
    pub const NONE: Target = Target { slot: 0, at: 0, stride: 0 };
}

/// The draw being captured: `instances` instances of vertices `first ..
/// first + count` in `mode`, the first instance being `first_instance`
/// (draw_instanced draws an instanced draw instance by instance when the
/// back end cannot).
pub(super) struct Draw {
    pub program: GLuint,
    pub mode: GLenum,
    pub first: u32,
    pub count: u32,
    pub instances: u32,
    pub first_instance: u32,
}

/// Why a draw could not be captured: the GL error it raises and the reason
/// the log gives.
pub(super) struct Refusal {
    err: GLenum,
    why: Fault,
}

impl Refusal {
    fn new(why: core::fmt::Arguments) -> Refusal {
        Refusal { err: GL_INVALID_OPERATION, why: Fault::new(why) }
    }
    pub fn error(&self) -> GLenum {
        self.err
    }
}

impl core::fmt::Display for Refusal {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(self.why.as_str())
    }
}

macro_rules! refuse {
    ($($t:tt)*) => { Refusal::new(format_args!($($t)*)) };
}

/// The decomposed primitive's vertex `k` of a `count`-vertex draw in `mode`
/// (see the module note), as an index from the draw's first vertex.
pub(super) fn prim_vertex(mode: GLenum, count: u32, k: u32) -> u32 {
    let (p, j) = match mode {
        GL_LINE_STRIP | GL_LINE_LOOP => (k / 2, k % 2),
        GL_TRIANGLE_STRIP | GL_TRIANGLE_FAN => (k / 3, k % 3),
        _ => return k, /* points, lines, triangles: the vertices in order */
    };
    match mode {
        GL_LINE_STRIP => p + j,
        GL_LINE_LOOP if p + 1 == count => {
            if j == 0 {
                count - 1
            } else {
                0
            }
        }
        GL_LINE_LOOP => p + j,
        GL_TRIANGLE_STRIP if p % 2 == 1 && j < 2 => p + 1 - j,
        GL_TRIANGLE_STRIP => p + j,
        _ => {
            if j == 0 {
                0
            } else {
                p + j
            }
        }
    }
}

/// Vertices one primitive of Begin's type has, for a draw `mode`.
fn per_primitive(mode: GLenum) -> u32 {
    match mode {
        GL_POINTS => 1,
        GL_LINES | GL_LINE_STRIP | GL_LINE_LOOP => 2,
        _ => 3,
    }
}

/// One captured run of components: what the shader stores at `slot`, its
/// first `comps` lanes, into binding `binding` at byte `offset` of the
/// vertex's record there.
#[derive(Clone, Copy)]
struct Piece {
    slot: IoSlot,
    comps: u8,
    binding: u8,
    offset: u16,
}

/// Where one shader input's value comes from.
#[derive(Clone, Copy)]
enum Feed {
    /// Generic attribute index (after glBindAttribLocation).
    Attrib(u8),
    VertexId,
    InstanceId,
}

/// The stores of one vertex, kept per [`Piece`].
struct Stores<'s> {
    pieces: &'s [Piece],
    values: &'s mut [[u32; 4]],
    /// Lanes defined, cleared per vertex: an output the shader never wrote,
    /// or a lane no instruction defined, is captured as 0 (the value is
    /// undefined; 0 is the deterministic choice, section 5.3 rule 7).
    defined: &'s mut [[bool; 4]],
}

impl Sink for Stores<'_> {
    fn store(&mut self, slot: IoSlot, value: [u32; 4], defined: [bool; 4]) {
        for (k, p) in self.pieces.iter().enumerate() {
            if p.slot == slot {
                self.values[k] = value;
                self.defined[k] = defined;
            }
        }
    }
}

/// Capture `d` into `targets` (one per binding the program writes, in
/// binding order) for the program's `rows`; `separate` is
/// GL_SEPARATE_ATTRIBS. `src` is the vertex shader's text as linked. The
/// caller checked that the capture fits. Returns the primitives written.
pub(super) fn capture(
    c: &mut GlContext,
    rows: &[TfRow],
    separate: bool,
    src: &[u8],
    targets: &[Target],
    d: &Draw,
) -> Result<u32, Refusal> {
    let Ok(text) = core::str::from_utf8(src) else {
        return Err(refuse!("the vertex shader's source is not text"));
    };
    /* the vertex block as a draw would upload it: the bound uniform
     * buffers' members and gl_DepthRange in their rows first */
    super::program::fill_uniform_blocks(c, d.program);
    {
        let range = c.depth_range;
        let p = &mut c.objs().programs[obj_index(d.program)];
        super::draw::fill_depth_range_uniforms(p, range);
    }
    /* a pending kick may still read a buffer: write fresh storage */
    for t in targets {
        if !super::buffer::writable(c, t.slot as usize, "transform feedback capture") {
            return Err(Refusal { err: GL_OUT_OF_MEMORY, why: Fault::new(format_args!("out of memory")) });
        }
    }
    match store::with_scratch(|a| capture_in(c, a, text, rows, separate, targets, d)) {
        Some(r) => r,
        None => Err(refuse!("the shader compiler's scratch is in use")),
    }
}

fn capture_in(
    c: &mut GlContext,
    a: &Arena<'_>,
    text: &str,
    rows: &[TfRow],
    separate: bool,
    targets: &[Target],
    d: &Draw,
) -> Result<u32, Refusal> {
    let oom = |_| refuse!("out of the shader compiler's scratch memory");
    let text = a.alloc_str(text).map_err(oom)?;
    let l = lower::compile_to_ir(a, Stage::Vertex, text)
        .map_err(|e| refuse!("the vertex shader does not lower again (line {}: {})", e.line, e.msg))?;
    let m = &l.module;
    let uniforms = uniform_block(c, a, m, d.program)?;
    let mut texels = GlTexels::new(bind_textures(c, a, m, d.program)?);

    /* the pieces each vertex's records are made of */
    let mut pieces = arena_vec::<Piece>(a, 0)?;
    let mut offsets = [0u16; super::tf::TF_MAX_SEPARATE_ATTRIBS as usize];
    for (ri, r) in rows.iter().enumerate() {
        let binding = if separate { ri } else { 0 };
        let mut push = |slot: IoSlot, comps: u8| -> Result<(), Refusal> {
            let at = &mut offsets[binding];
            pieces.push(a, Piece { slot, comps, binding: binding as u8, offset: *at }).map_err(oom)?;
            *at += u16::from(comps) * 4;
            Ok(())
        };
        match r.builtin() {
            1 => push(IoSlot::Position, 4)?,
            2 => push(IoSlot::PointSize, 1)?,
            _ => {
                let Some(v) = l.reflection.varyings.iter().find(|v| v.name.as_bytes() == r.base_name()) else {
                    return Err(refuse!(
                        "varying {} is not an output of the lowered vertex shader",
                        core::str::from_utf8(r.name()).unwrap_or("?")
                    ));
                };
                let (cols, height) = super::program::gl_type_shape(r.gl_type() as u16);
                for e in r.first_element()..r.first_element() + r.elements() {
                    for j in 0..cols as u32 {
                        let loc = u32::from(v.location) + e * cols as u32 + j;
                        push(IoSlot::Varying(loc as u8), height as u8)?;
                    }
                }
            }
        }
    }
    let pieces = pieces.as_slice();

    /* the shader's inputs, and where each comes from */
    let (mut io, feeds) = inputs(c, a, m, d.program)?;
    let frame_values = a.alloc_filled(m.instrs.len(), [0u32; 4]).map_err(oom)?;
    let frame_def = a.alloc_filled(m.instrs.len(), [false; 4]).map_err(oom)?;
    let frame_locals = a.alloc_filled(m.locals.len(), [0u32; 4]).map_err(oom)?;
    let frame_ldef = a.alloc_filled(m.locals.len(), [false; 4]).map_err(oom)?;
    let values = a.alloc_filled(pieces.len(), [0u32; 4]).map_err(oom)?;
    let defined = a.alloc_filled(pieces.len(), [false; 4]).map_err(oom)?;

    /* a vertex's records in every binding, back to back: `record` bytes */
    let rec_at: [usize; 4] = core::array::from_fn(|b| targets.iter().take(b).map(|t| t.stride as usize).sum::<usize>());
    let record: usize = targets.iter().map(|t| t.stride as usize).sum();
    /* the cache: the draw's vertex 0 (a fan's and a loop's), and the last
     * three evaluated (a strip's) */
    const WAYS: usize = 4;
    let cache = a.alloc_filled(WAYS * record, 0u8).map_err(oom)?;
    let mut next_way = 1usize;

    let per = captured_per_instance(d.mode, d.count);
    let mut out = 0usize; /* vertices written so far, per binding */
    for inst in 0..d.instances {
        let instance = d.first_instance + inst;
        let mut tags = [u32::MAX; WAYS];
        for k in 0..per {
            let vi = prim_vertex(d.mode, d.count, k);
            let way = match tags.iter().position(|&t| t == vi) {
                Some(w) => w,
                None => {
                    let w = if vi == 0 {
                        0
                    } else {
                        let w = next_way;
                        next_way = if next_way + 1 == WAYS { 1 } else { next_way + 1 };
                        w
                    };
                    for (slot, feed) in io.as_mut_slice().iter_mut().zip(feeds.as_slice()) {
                        slot.1 = match *feed {
                            Feed::VertexId => [d.first + vi; 4],
                            Feed::InstanceId => [instance; 4],
                            Feed::Attrib(app) => attrib_value(c, usize::from(app), d.first + vi, instance)?,
                        };
                    }
                    defined.fill([false; 4]);
                    let mut sink = Stores { pieces, values: &mut *values, defined: &mut *defined };
                    let frame = Frame {
                        values: &mut *frame_values,
                        def: &mut *frame_def,
                        locals: &mut *frame_locals,
                        ldef: &mut *frame_ldef,
                    };
                    eval::run_in_with_textures(
                        m,
                        &Inputs { io: io.as_slice(), uniforms },
                        frame,
                        &mut sink,
                        &mut texels,
                    )
                    .map_err(|f| refuse!("vertex {}: {f}", d.first + vi))?;
                    let rec = &mut cache[w * record..(w + 1) * record];
                    rec.fill(0);
                    for (p, (v, def)) in pieces.iter().zip(values.iter().zip(defined.iter())) {
                        let base = rec_at[usize::from(p.binding)] + usize::from(p.offset);
                        for lane in 0..usize::from(p.comps) {
                            let word = if def[lane] { v[lane] } else { 0 };
                            rec[base + 4 * lane..base + 4 * lane + 4].copy_from_slice(&word.to_ne_bytes());
                        }
                    }
                    tags[w] = vi;
                    w
                }
            };
            let rec = &cache[way * record..(way + 1) * record];
            for (b, t) in targets.iter().enumerate() {
                let bytes = &rec[rec_at[b]..rec_at[b] + t.stride as usize];
                if !super::buffer::write_at(c, t.slot as usize, t.at + out * t.stride as usize, bytes) {
                    return Err(Refusal { err: GL_OUT_OF_MEMORY, why: Fault::new(format_args!("the write failed")) });
                }
            }
            out += 1;
        }
    }
    Ok(per / per_primitive(d.mode) * d.instances)
}

/// Vertices one instance captures (`tf::captured_vertices`), as a u32.
fn captured_per_instance(mode: GLenum, count: u32) -> u32 {
    super::tf::captured_vertices(mode, count) as u32
}

/// An empty arena vector (the capture's lists are short).
fn arena_vec<'a, T: Copy>(a: &Arena<'a>, cap: usize) -> Result<AVec<'a, T>, Refusal> {
    AVec::with_capacity(a, cap).map_err(|_| refuse!("out of the shader compiler's scratch memory"))
}

/// The uniform block the IR reads, from the program's vertex block: each
/// IR uniform's dwords from the linked stage's row of the same name (the
/// two layouts are the same compiler's when the stage was compiled online,
/// and may differ when it came from the offline table). A uniform with no
/// such row, or a row of another size, is refused only if the shader reads
/// it.
fn uniform_block<'a>(c: &GlContext, a: &Arena<'a>, m: &Module<'_>, program: GLuint) -> Result<&'a [u32], Refusal> {
    let block = a
        .alloc_filled(usize::from(m.uniform_dwords), 0u32)
        .map_err(|_| refuse!("out of the shader compiler's scratch memory"))?;
    let p = &c.objs_ref().programs[obj_index(program)];
    let have = p.vs_block();
    let table = p.rvs.uniform_table();
    for u in m.uniforms.iter().filter(|u| u.dword_off != SHC_UNIFORM_NO_BLOCK) {
        let (off, n) = (usize::from(u.dword_off), usize::from(u.dwords));
        let row = table.iter().find(|r| util::name_bytes(&r.name) == u.name.as_bytes());
        match row {
            Some(r) if r.dword_off != SHC_UNIFORM_NO_BLOCK && r.dwords == u.dwords => {
                let from = usize::from(r.dword_off);
                for k in 0..n {
                    if let (Some(dst), Some(src)) = (block.get_mut(off + k), have.get(from + k)) {
                        *dst = src.to_bits();
                    }
                }
            }
            _ => {
                let read = m.instrs.iter().any(|op| match *op {
                    Op::LoadUniform { dword_off, comps } => {
                        let at = usize::from(dword_off);
                        at < off + n && at + usize::from(comps) > off
                    }
                    Op::LoadUniformIdx { base_dw, stride_dw, len, comps, .. } => {
                        let at = usize::from(base_dw);
                        let end = at + usize::from(stride_dw) * usize::from(len.max(1) - 1) + usize::from(comps);
                        at < off + n && end > off
                    }
                    _ => false,
                });
                if read {
                    return Err(refuse!("uniform {} is not laid out in the linked vertex stage as it is read", u.name));
                }
            }
        }
    }
    Ok(block)
}

/// The input slots the IR reads (every attribute column, `gl_VertexID` and
/// `gl_InstanceID`) and where each one's value comes from.
#[allow(clippy::type_complexity)]
fn inputs<'a>(
    c: &GlContext,
    a: &Arena<'a>,
    m: &Module<'_>,
    program: GLuint,
) -> Result<(AVec<'a, (IoSlot, [u32; 4])>, AVec<'a, Feed>), Refusal> {
    let oom = |_| refuse!("out of the shader compiler's scratch memory");
    let p = &c.objs_ref().programs[obj_index(program)];
    let linked = &p.rvs.attribs[..(p.rvs.num_attribs as usize).min(p.rvs.attribs.len())];
    let mut io = arena_vec::<(IoSlot, [u32; 4])>(a, m.inputs.len() + 2)?;
    let mut feeds = arena_vec::<Feed>(a, m.inputs.len() + 2)?;
    for decl in m.inputs.iter().filter(|d| d.kind == crate::shaderc::online::ir::IoKind::Attribute) {
        let (cols, _) = super::program::gl_type_shape(decl.gl_type);
        /* the linked stage's location for this name (the app's index table
         * is keyed by it), the IR's own when the name is not there */
        let base = linked
            .iter()
            .find(|r| util::name_bytes(&r.name) == decl.name.as_bytes())
            .map_or(decl.location, |r| r.location);
        for j in 0..cols as u8 {
            let loc = usize::from(base) + usize::from(j);
            let Some(&app) = p.attrib_app_index.get(loc) else {
                return Err(refuse!("attribute {} has location {loc}, past the vertex inputs", decl.name));
            };
            if usize::from(app) >= MAX_VERTEX_ATTRIBS {
                return Err(refuse!("attribute {} reads generic attribute {app}", decl.name));
            }
            io.push(a, (IoSlot::Attrib(decl.location + j), [0; 4])).map_err(oom)?;
            feeds.push(a, Feed::Attrib(app)).map_err(oom)?;
        }
    }
    io.push(a, (IoSlot::VertexId, [0; 4])).map_err(oom)?;
    feeds.push(a, Feed::VertexId).map_err(oom)?;
    io.push(a, (IoSlot::InstanceId, [0; 4])).map_err(oom)?;
    feeds.push(a, Feed::InstanceId).map_err(oom)?;
    Ok((io, feeds))
}

/// Generic attribute `app`'s value for vertex `vertex` of instance
/// `instance`: the array's element (element `instance / divisor` for an
/// array with a divisor), or the glVertexAttrib* value of a disabled array.
fn attrib_value(c: &mut GlContext, app: usize, vertex: u32, instance: u32) -> Result<[u32; 4], Refusal> {
    let ga = c.attribs[app];
    if !ga.enabled {
        return Ok(c.current_attrib[app].map(f32::to_bits));
    }
    let element = u64::from(instance.checked_div(ga.divisor).unwrap_or(vertex));
    super::draw::attrib_element(c, &ga, element)
        .ok_or_else(|| refuse!("attribute {app}: element {element} is outside its array"))
}

/* ---- textures: the vertex shader's look-ups, from the GL texture storage ---- */

/// What a texture's components are, which decides the sampler type that may
/// read it (ES 3.0.6 3.8.14: a float sampler reads a float or normalised
/// texture, an `isampler*` a signed integer one, a `usampler*` an unsigned
/// one; any other pairing is undefined, and refused here).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Comp {
    Float,
    Int,
    Uint,
}

/// One sampler binding the vertex shader reads, resolved before the first
/// vertex: where the texture's texels are (the layout `texture.rs` gives
/// them, read but not changed here) and the sampler state the look-ups
/// apply. `base` is the storage's CPU view: nothing in a capture frees or
/// lays out a texture again, so it stays valid for the capture's length.
#[derive(Clone, Copy)]
pub(super) struct Bound {
    sampler: u8,
    dim: TexDim,
    /// No complete texture: every look-up reads (0, 0, 0, 1) (3.8.13), and
    /// textureSize reads the storage's size (0 without one).
    incomplete: bool,
    base: *const u8,
    bytes: usize,
    /// The base level (GL_TEXTURE_BASE_LEVEL: every level number a look-up
    /// uses counts from it, and the arrays below are shifted by it), one face
    /// or layer or slice.
    width: u32,
    height: u32,
    /// The base level's depth (3D), its layer count (2D array), 6 (cube) or 1.
    layers: u32,
    /// A 3D texture's slice grid (`GlTexture::grid_cols`), 0 when stacked.
    grid_cols: u32,
    level_off: [u32; CAP_MIP_LEVELS],
    /// Each level's row pitch in texels (`texture.rs` `level_row_pitch`).
    pitch: [u32; CAP_MIP_LEVELS],
    /// One bit per level a look-up may read: specified, of the chain's
    /// size and format, held by the storage, at most `q`.
    level_ok: u16,
    /// The chain's last level q (3.8.10.4): min(p, GL_TEXTURE_MAX_LEVEL).
    q: u32,
    bpp: usize,
    texfmt: u8,
    comp: Comp,
    /// The texture's format (GL_RGB, GL_LUMINANCE, GL_DEPTH_COMPONENT, ...).
    format: GLenum,
    /// Stored B,G,R (`bgra` or `pe_order`).
    bgra: bool,
    /// Alpha reads 1 (`draw::texture_has_no_alpha`).
    no_alpha: bool,
    /// A depth texture samples (d, d, d, 1) (`draw::texture_depth_luminance`).
    depth_luminance: bool,
    /// A floating-point depth (GL_DEPTH_COMPONENT32F, GL_DEPTH32F_STENCIL8:
    /// the texture's type is GL_FLOAT or GL_FLOAT_32_UNSIGNED_INT_24_8_REV),
    /// whose compare reference is not clamped (3.8.16).
    float_depth: bool,
    min_filter: GLenum,
    mag_filter: GLenum,
    /// S, T, R.
    wrap: [GLenum; 3],
    min_lod: f32,
    max_lod: f32,
    swizzle: [u16; 4],
    /// GL_TEXTURE_COMPARE_MODE and _FUNC (the sampler object's when one is
    /// bound), read by a shadow look-up.
    compare: (GLenum, GLenum),
}

impl Bound {
    const NONE: Bound = Bound {
        sampler: 0,
        dim: TexDim::D2,
        incomplete: true,
        base: core::ptr::null(),
        bytes: 0,
        width: 0,
        height: 0,
        layers: 0,
        grid_cols: 0,
        level_off: [0; CAP_MIP_LEVELS],
        pitch: [0; CAP_MIP_LEVELS],
        level_ok: 0,
        q: 0,
        bpp: 0,
        texfmt: 0,
        comp: Comp::Float,
        format: 0,
        bgra: false,
        no_alpha: false,
        depth_luminance: false,
        float_depth: false,
        min_filter: GL_NEAREST,
        mag_filter: GL_NEAREST,
        wrap: [GL_REPEAT; 3],
        min_lod: -1000.0,
        max_lod: 1000.0,
        swizzle: [GL_RED as u16, GL_GREEN as u16, GL_BLUE as u16, GL_ALPHA as u16],
        compare: (GL_NONE, GL_LEQUAL),
    };
}

/// The capture's [`TexelSource`]: the bindings [`bind_textures`] resolved.
pub(super) struct GlTexels<'a> {
    bound: &'a [Bound],
}

impl<'a> GlTexels<'a> {
    /// The source of `bound` (each from [`bound_of`]).
    pub(super) fn new(bound: &'a [Bound]) -> GlTexels<'a> {
        GlTexels { bound }
    }
}

/// The face's (s_c, t_c, |m_a|) of vector `v` (table 3.21): linear in `v`,
/// so a gradient goes through it as the coordinate does.
fn cube_axes(face: u32, v: [f32; 3]) -> (f32, f32, f32) {
    let [x, y, z] = v;
    match face {
        0 => (-z, -y, x),
        1 => (z, -y, -x),
        2 => (x, z, y),
        3 => (x, -z, -y),
        4 => (x, -y, z),
        _ => (-x, -y, -z),
    }
}

/// GL's side of level `l`: max(side >> l, 1).
fn side(s: u32, l: u32) -> u32 {
    super::context::pot_level_side(s, l)
}

fn fault(args: core::fmt::Arguments) -> Fault {
    Fault::new(args)
}

impl GlTexels<'_> {
    fn get(&self, sampler: u8) -> Result<&Bound, Fault> {
        self.bound
            .iter()
            .find(|b| b.sampler == sampler)
            .ok_or_else(|| fault(format_args!("sampler binding {sampler} was not resolved")))
    }
}

impl TexelSource for GlTexels<'_> {
    fn sample(&mut self, q: &TexLookup) -> Result<[u32; 4], Fault> {
        let b = self.get(q.sampler)?;
        if b.dim != q.dim {
            return Err(fault(format_args!("sampler binding {} is read as two texture targets", q.sampler)));
        }
        if q.dref.is_some() {
            return b.shadow(q);
        }
        if b.incomplete {
            return Ok(b.finish(b.opaque_black()));
        }
        let v = match q.level {
            TexLevel::Fetch(l) => b.fetch(q, l)?,
            TexLevel::Lod(lod) => b.filtered(q, lod)?,
            TexLevel::Grad { ddx, ddy } => b.filtered(q, b.grad_lod(q, ddx, ddy))?,
        };
        Ok(b.finish(v))
    }

    fn size(&mut self, sampler: u8, dim: TexDim, lod: i32) -> Result<[u32; 4], Fault> {
        let b = self.get(sampler)?;
        /* undefined past the chain (GLSL ES 3.00 8.9.1): the minified size,
         * and 0 for a texture with no image */
        if b.width == 0 || !(0..32).contains(&lod) {
            return Ok([0; 4]);
        }
        let l = lod as u32;
        let (w, h) = (side(b.width, l), side(b.height, l));
        Ok(match dim {
            TexDim::D3 => [w, h, side(b.layers, l), 0],
            TexDim::D2Array => [w, h, b.layers, 0],
            _ => [w, h, 0, 0],
        })
    }
}

impl Bound {
    /// A shadow look-up (`sampler*Shadow`, ES 3.0.6 3.8.15): the reference
    /// compared with each depth texel the filter reads, the results filtered
    /// as the texels would be; lane 0 is the result. An incomplete texture
    /// reads 0 (the (0, 0, 0, 1) of 3.8.13 as a depth), and the texture
    /// swizzle does not apply to a comparison.
    fn shadow(&self, q: &TexLookup) -> Result<[u32; 4], Fault> {
        if self.incomplete {
            return Ok([0; 4]);
        }
        if !matches!(self.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL) {
            return Err(fault(format_args!("a shadow sampler reads a texture that is not a depth texture")));
        }
        if self.compare.0 != GL_COMPARE_REF_TO_TEXTURE {
            return Err(fault(format_args!(
                "a shadow sampler reads a texture whose GL_TEXTURE_COMPARE_MODE is GL_NONE (undefined in GL)"
            )));
        }
        let v = match q.level {
            TexLevel::Lod(lod) => self.filtered(q, lod)?,
            TexLevel::Grad { ddx, ddy } => self.filtered(q, self.grad_lod(q, ddx, ddy))?,
            TexLevel::Fetch(_) => return Err(fault(format_args!("a texelFetch of a shadow sampler"))),
        };
        let Texel::F(r) = v else { return Err(fault(format_args!("a shadow look-up of an integer texture"))) };
        Ok([r[0].to_bits(), 0, 0, 0])
    }

    /// One texel for look-up `q`: as stored, or for a shadow look-up the
    /// comparison's result (1.0 or 0.0) of the reference with its depth.
    fn texel_for(&self, q: &TexLookup, level: u32, x: u32, y: u32, z: u32) -> Result<Texel, Fault> {
        let t = self.texel(level, x, y, z)?;
        let (Some(r), Texel::F(d)) = (q.dref, t) else { return Ok(t) };
        /* the reference is clamped to [0, 1] for a fixed-point depth only */
        let r = if r.is_nan() {
            0.0
        } else if self.float_depth {
            r
        } else {
            r.clamp(0.0, 1.0)
        };
        let d = d[0];
        let pass = match self.compare.1 {
            GL_LEQUAL => r <= d,
            GL_GEQUAL => r >= d,
            GL_LESS => r < d,
            GL_GREATER => r > d,
            GL_EQUAL => r == d,
            GL_NOTEQUAL => r != d,
            GL_ALWAYS => true,
            _ => false,
        };
        Ok(Texel::F([if pass { 1.0 } else { 0.0 }, 0.0, 0.0, 1.0]))
    }

    /// The level of detail `textureGrad` gives (ES 3.0.6 3.8.10, equations
    /// 3.17-3.19): log2 of the longer of the two gradients, in texels of the
    /// base level. A cube map's are those of the face's (s, t) (table 3.21),
    /// derived through the face's divide by the major axis.
    fn grad_lod(&self, q: &TexLookup, ddx: [f32; 4], ddy: [f32; 4]) -> f32 {
        let (w, h) = (self.width as f32, self.height as f32);
        let len = |g: [f32; 3]| fmath::sqrt(g[0] * g[0] + g[1] * g[1] + g[2] * g[2]);
        let rho = if q.dim == TexDim::Cube {
            let c = q.coord.map(f32::from_bits);
            let (face, ..) = cube_face(c[0], c[1], c[2]);
            let (sc, tc, ma) = cube_axes(face, [c[0], c[1], c[2]]);
            let g = |d: [f32; 4]| {
                let (dsc, dtc, dma) = cube_axes(face, [d[0], d[1], d[2]]);
                /* s = (sc / |ma| + 1) / 2, derived by the quotient rule */
                let ds = 0.5 * (dsc * ma - sc * dma) / (ma * ma);
                let dt = 0.5 * (dtc * ma - tc * dma) / (ma * ma);
                [ds * w, dt * h, 0.0]
            };
            len(g(ddx)).max(len(g(ddy)))
        } else {
            /* lanes past the base width are undefined (an array's layer takes
             * no part) */
            let g = |d: [f32; 4]| {
                let r = if q.dim == TexDim::D3 { d[2] * self.layers as f32 } else { 0.0 };
                [d[0] * w, d[1] * h, r]
            };
            len(g(ddx)).max(len(g(ddy)))
        };
        fmath::log2(rho)
    }

    /// (0, 0, 0, 1) in the texture's component type.
    fn opaque_black(&self) -> Texel {
        match self.comp {
            Comp::Float => Texel::F([0.0, 0.0, 0.0, 1.0]),
            _ => Texel::I([0, 0, 0, 1]),
        }
    }

    /// (0, 0, 0, 0): what an undefined texelFetch (a level or a texel
    /// outside the image, GLSL ES 3.00 8.9.2) reads here.
    fn zero(&self) -> Texel {
        match self.comp {
            Comp::Float => Texel::F([0.0; 4]),
            _ => Texel::I([0; 4]),
        }
    }

    /// The texture swizzle (ES 3.0.6 3.8.14.2), then the lanes' bits.
    fn finish(&self, v: Texel) -> [u32; 4] {
        let (bits, one): ([u32; 4], u32) = match v {
            Texel::F(f) => (f.map(f32::to_bits), 1.0f32.to_bits()),
            Texel::I(i) => (i, 1),
        };
        core::array::from_fn(|k| match u32::from(self.swizzle[k]) {
            GL_RED => bits[0],
            GL_GREEN => bits[1],
            GL_BLUE => bits[2],
            GL_ALPHA => bits[3],
            GL_ZERO => 0,
            GL_ONE => one,
            _ => bits[k],
        })
    }

    /// The size of level `l`: width, height, and the depth (3D) or the
    /// layer count (array, cube).
    fn level_size(&self, l: u32) -> (u32, u32, u32) {
        let d = match self.dim {
            TexDim::D3 => side(self.layers, l),
            TexDim::D2 => 1,
            _ => self.layers,
        };
        (side(self.width, l), side(self.height, l), d)
    }

    /// Texel (x, y) of slice, layer or face `z` of level `level`, decoded
    /// as the format samples it (ES 3.0.6 3.8.14.1 and table 3.15: the
    /// components a format lacks read 0, alpha 1). The caller keeps the
    /// coordinates inside the level.
    fn texel(&self, level: u32, x: u32, y: u32, z: u32) -> Result<Texel, Fault> {
        let l = level as usize;
        let (lw, lh) = (side(self.width, level) as usize, side(self.height, level) as usize);
        /* texture.rs `texel_offset_3d` / `slice_origin`: a grid tiles a 3D
         * level's slices in `grid_cols` columns, a stack puts slices,
         * layers and faces one under the other */
        let (x0, y0) = if self.grid_cols != 0 {
            ((z % self.grid_cols) as usize * lw, (z / self.grid_cols) as usize * lh)
        } else {
            (0, z as usize * lh)
        };
        let off =
            self.level_off[l] as usize + ((y0 + y as usize) * self.pitch[l] as usize + x0 + x as usize) * self.bpp;
        if off.checked_add(self.bpp).is_none_or(|end| end > self.bytes) {
            return Err(fault(format_args!(
                "texel ({x}, {y}, {z}) of level {level} lies outside the texture's storage"
            )));
        }
        // SAFETY: `base` is the texture's CPU view of `bytes` bytes (valid
        // for the capture, see `Bound`); `off .. off + bpp` lies inside it.
        let p = unsafe { core::slice::from_raw_parts(self.base.add(off), self.bpp) };
        let Some(v) = decode(p, self.texfmt, self.bgra) else {
            return Err(fault(format_args!("storage class {} is not decoded", self.texfmt)));
        };
        Ok(match v {
            Texel::F(mut f) => {
                if matches!(self.format, GL_DEPTH_COMPONENT | GL_DEPTH_STENCIL) {
                    let d = f[0];
                    f = if self.depth_luminance { [d, d, d, 1.0] } else { [d, 0.0, 0.0, 1.0] };
                } else if matches!(self.texfmt, GLES_TEXFMT_R16F | GLES_TEXFMT_R32F) {
                    /* a one-channel float store of a luminance or alpha image */
                    let r = f[0];
                    match self.format {
                        GL_LUMINANCE => f = [r, r, r, 1.0],
                        GL_ALPHA => f = [0.0, 0.0, 0.0, r],
                        _ => {}
                    }
                }
                if self.no_alpha {
                    f[3] = 1.0;
                }
                Texel::F(f)
            }
            i => i,
        })
    }

    /// `texelFetch(s, P, lod)` (GLSL ES 3.00 8.9.2; ES 3.0.6 3.8.10 "texel
    /// fetches"): the texel at integer coordinates of level `lod`, no
    /// filtering, no wrap. Outside the image or the chain the result is
    /// undefined; it reads (0, 0, 0, 0).
    fn fetch(&self, q: &TexLookup, lod: i32) -> Result<Texel, Fault> {
        if q.dim == TexDim::Cube {
            return Err(fault(format_args!("a texelFetch on a cube map (no such function in GLSL ES 3.00)")));
        }
        if !(0..16).contains(&lod) || self.level_ok & (1 << lod) == 0 {
            return Ok(self.zero());
        }
        let l = lod as u32;
        let (w, h, d) = self.level_size(l);
        let i = |k: usize| q.coord[k] as i32;
        let x = i(0).wrapping_add(q.offset[0]);
        let y = i(1).wrapping_add(q.offset[1]);
        let z = match q.dim {
            TexDim::D3 => i(2).wrapping_add(q.offset[2]),
            TexDim::D2Array => i(2), /* the layer takes no offset */
            _ => 0,
        };
        let inside = |v: i32, n: u32| v >= 0 && (v as u32) < n;
        if !(inside(x, w) && inside(y, h) && inside(z, d)) {
            return Ok(self.zero());
        }
        self.texel(l, x as u32, y as u32, z as u32)
    }

    /// A filtered look-up at level of detail `lod` (ES 3.0.6 3.8.10-3.8.11):
    /// lambda clamped to [MIN_LOD, MAX_LOD], magnification at lambda <= 0
    /// (c = 0 in ES 3.0), the level or the two levels the minification
    /// filter selects, blended by the fraction of lambda.
    fn filtered(&self, q: &TexLookup, lod: f32) -> Result<Texel, Fault> {
        let lam = if lod.is_nan() { 0.0 } else { lod }.max(self.min_lod).min(self.max_lod);
        let qmax = self.q;
        let (filter, d1, d2, frac) = if lam <= 0.0 {
            (self.mag_filter, 0, 0, 0.0)
        } else {
            let lin = matches!(self.min_filter, GL_LINEAR | GL_LINEAR_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_LINEAR);
            let f = if lin { GL_LINEAR } else { GL_NEAREST };
            match self.min_filter {
                GL_NEAREST_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_NEAREST => {
                    /* 3.8.10.4: the nearest level, ceil(lambda + 1/2) - 1 */
                    let d = if lam <= 0.5 { 0 } else { (fmath::ceil(lam + 0.5) - 1.0) as u32 };
                    (f, d.min(qmax), d.min(qmax), 0.0)
                }
                GL_NEAREST_MIPMAP_LINEAR | GL_LINEAR_MIPMAP_LINEAR => {
                    if lam >= qmax as f32 {
                        (f, qmax, qmax, 0.0)
                    } else {
                        let fl = fmath::floor(lam);
                        let d = fl as u32;
                        (f, d, (d + 1).min(qmax), lam - fl)
                    }
                }
                _ => (f, 0, 0, 0.0),
            }
        };
        let t1 = self.at_level(q, filter, d1)?;
        if d2 == d1 || frac == 0.0 {
            return Ok(t1);
        }
        let t2 = self.at_level(q, filter, d2)?;
        match (t1, t2) {
            (Texel::F(a), Texel::F(b)) => Ok(Texel::F(lerp(a, b, frac))),
            _ => Err(fault(format_args!("a mipmap-linear look-up of an integer texture"))),
        }
    }

    /// One level's NEAREST or LINEAR sample (3.8.11).
    fn at_level(&self, q: &TexLookup, filter: GLenum, level: u32) -> Result<Texel, Fault> {
        if self.level_ok & (1 << level) == 0 {
            return Err(fault(format_args!(
                "level {level} of the texture at sampler binding {} is not stored here",
                self.sampler
            )));
        }
        let c = q.coord.map(f32::from_bits);
        if q.dim == TexDim::Cube {
            let (face, s, t) = cube_face(c[0], c[1], c[2]);
            return self.cube_level(q, level, filter, face, s, t);
        }
        let (w, h, d) = self.level_size(level);
        let o = q.offset.map(|v| v as f32);
        let u = c[0] * w as f32 + o[0];
        let v = c[1] * h as f32 + o[1];
        /* the third texel coordinate: a 3D texture's, filtered like the
         * other two; an array's layer, floor(r + 1/2) clamped (3.8.10),
         * never filtered */
        let (wc, layer) = match q.dim {
            TexDim::D3 => (Some(c[2] * d as f32 + o[2]), 0),
            TexDim::D2Array => (None, (fmath::floor(c[2] + 0.5) as i32).clamp(0, d as i32 - 1) as u32),
            _ => (None, 0),
        };
        let [ws, wt, wr] = self.wrap;
        if filter != GL_LINEAR {
            let i = wrap(fmath::floor(u) as i32, w, ws);
            let j = wrap(fmath::floor(v) as i32, h, wt);
            let k = wc.map_or(layer, |wc| wrap(fmath::floor(wc) as i32, d, wr));
            return self.texel_for(q, level, i, j, k);
        }
        /* 3.8.11: the 2x2 (2x2x2) texels around the sample point */
        let (i0, a) = split(u);
        let (j0, b) = split(v);
        let (k0, g) = wc.map_or((0, 0.0), split);
        let mut sum = [0.0f32; 4];
        let ks: &[(i32, f32)] = if wc.is_some() { &[(0, 1.0 - g), (1, g)] } else { &[(0, 1.0)] };
        for &(dk, wk) in ks {
            for (dj, wj) in [(0, 1.0 - b), (1, b)] {
                for (di, wi) in [(0, 1.0 - a), (1, a)] {
                    let i = wrap(i0.wrapping_add(di), w, ws);
                    let j = wrap(j0.wrapping_add(dj), h, wt);
                    let k = if wc.is_some() { wrap(k0.wrapping_add(dk), d, wr) } else { layer };
                    let Texel::F(t) = self.texel_for(q, level, i, j, k)? else {
                        return Err(fault(format_args!("LINEAR filtering of an integer texture")));
                    };
                    let weight = wi * wj * wk;
                    for (s, v) in sum.iter_mut().zip(t) {
                        *s += weight * v;
                    }
                }
            }
        }
        Ok(Texel::F(sum))
    }

    /// One level of a cube map at face `face`, (s, t) in [0, 1]: NEAREST
    /// clamps to the face; LINEAR reads the 2x2 texels around the sample,
    /// those past an edge from the face across it (seamless filtering,
    /// ES 3.0.6 3.8.9.1, always on in ES 3.0).
    fn cube_level(&self, q: &TexLookup, level: u32, filter: GLenum, face: u32, s: f32, t: f32) -> Result<Texel, Fault> {
        let n = side(self.width, level);
        let (u, v) = (s * n as f32, t * n as f32);
        if filter != GL_LINEAR {
            let i = (fmath::floor(u) as i32).clamp(0, n as i32 - 1) as u32;
            let j = (fmath::floor(v) as i32).clamp(0, n as i32 - 1) as u32;
            return self.texel_for(q, level, i, j, face);
        }
        let (i0, a) = split(u);
        let (j0, b) = split(v);
        let mut sum = [0.0f32; 4];
        for (dj, wj) in [(0, 1.0 - b), (1, b)] {
            for (di, wi) in [(0, 1.0 - a), (1, a)] {
                let t = self.cube_texel(q, level, face, i0.wrapping_add(di), j0.wrapping_add(dj), n)?;
                for (s, v) in sum.iter_mut().zip(t) {
                    *s += wi * wj * v;
                }
            }
        }
        Ok(Texel::F(sum))
    }

    /// Texel (i, j) of face `face` of an n x n level, where i or j may be
    /// one texel past the face: then the texel across that edge, found by
    /// projecting the texel's centre onto the cube again; past a corner,
    /// the average of the three texels that meet there (OpenGL 4.6 8.14.1,
    /// the rule ES 3.0's seamless filtering takes from
    /// ARB_seamless_cube_map).
    fn cube_texel(&self, q: &TexLookup, level: u32, face: u32, i: i32, j: i32, n: u32) -> Result<[f32; 4], Fault> {
        let inside = |v: i32| v >= 0 && (v as u32) < n;
        let flt = |t: Texel| match t {
            Texel::F(f) => Ok(f),
            Texel::I(_) => Err(fault(format_args!("LINEAR filtering of an integer texture"))),
        };
        let across = |i: i32, j: i32| -> Result<[f32; 4], Fault> {
            let (f2, i2, j2) = cube_across(face, i, j, n);
            flt(self.texel_for(q, level, i2, j2, f2)?)
        };
        match (inside(i), inside(j)) {
            (true, true) => flt(self.texel_for(q, level, i as u32, j as u32, face)?),
            (false, true) | (true, false) => across(i, j),
            (false, false) => {
                let (ci, cj) = (i.clamp(0, n as i32 - 1), j.clamp(0, n as i32 - 1));
                let a = flt(self.texel_for(q, level, ci as u32, cj as u32, face)?)?;
                let b = across(i, cj)?;
                let c = across(ci, j)?;
                Ok(core::array::from_fn(|k| (a[k] + b[k] + c[k]) / 3.0))
            }
        }
    }
}

/// `(1 - t) a + t b`, lane by lane (3.8.10.4).
fn lerp(a: [f32; 4], b: [f32; 4], t: f32) -> [f32; 4] {
    core::array::from_fn(|k| (1.0 - t) * a[k] + t * b[k])
}

/// A LINEAR coordinate's first texel and the second's weight: floor(u - 1/2)
/// and its fraction (3.8.11).
fn split(u: f32) -> (i32, f32) {
    let x = u - 0.5;
    let f = fmath::floor(x);
    (f as i32, x - f)
}

/// A texel coordinate wrapped into 0 .. n (ES 3.0.6 table 3.19).
pub(super) fn wrap(i: i32, n: u32, mode: GLenum) -> u32 {
    let n = i64::from(n.max(1));
    let i = i64::from(i);
    let r = match mode {
        GL_CLAMP_TO_EDGE => i.clamp(0, n - 1),
        GL_MIRRORED_REPEAT => {
            let m = i.rem_euclid(2 * n) - n;
            (n - 1) - if m >= 0 { m } else { -(1 + m) }
        }
        _ => i.rem_euclid(n),
    };
    r as u32
}

/// The face (0 = +X .. 5 = -Z, the stacked order) and (s, t) a cube map
/// direction selects (ES 3.0.6 table 3.21). A tie goes to X, then Y.
pub(super) fn cube_face(x: f32, y: f32, z: f32) -> (u32, f32, f32) {
    let (ax, ay, az) = (x.abs(), y.abs(), z.abs());
    let (face, sc, tc, ma) = if ax >= ay && ax >= az {
        if x >= 0.0 {
            (0, -z, -y, ax)
        } else {
            (1, z, -y, ax)
        }
    } else if ay >= az {
        if y >= 0.0 {
            (2, x, z, ay)
        } else {
            (3, x, -z, ay)
        }
    } else if z >= 0.0 {
        (4, x, -y, az)
    } else {
        (5, -x, -y, az)
    };
    (face, 0.5 * (sc / ma + 1.0), 0.5 * (tc / ma + 1.0))
}

/// Texel (i, j) of face `face`, one past an edge of the n x n face: the
/// face and texel its centre falls on when projected through the cube
/// (table 3.21 inverted, then applied again).
pub(super) fn cube_across(face: u32, i: i32, j: i32, n: u32) -> (u32, u32, u32) {
    let sc = 2.0 * (i as f32 + 0.5) / n as f32 - 1.0;
    let tc = 2.0 * (j as f32 + 0.5) / n as f32 - 1.0;
    let (x, y, z) = match face {
        0 => (1.0, -tc, -sc),
        1 => (-1.0, -tc, sc),
        2 => (sc, 1.0, tc),
        3 => (sc, -1.0, -tc),
        4 => (sc, -tc, 1.0),
        _ => (-sc, -tc, -1.0),
    };
    let (f, s, t) = cube_face(x, y, z);
    let at = |c: f32| (fmath::floor(c * n as f32) as i32).clamp(0, n as i32 - 1) as u32;
    (f, at(s), at(t))
}

/// One stored texel of storage class `texfmt` (`backend::GLES_TEXFMT_*`),
/// as the texture unit reads it: normalised and float classes as floats,
/// integer classes as their bits (signed ones sign-extended). `bgra`: the
/// RGBA8 bytes are B,G,R,A. None for a class not decoded here (the ETC
/// block formats).
pub(super) fn decode(p: &[u8], texfmt: u8, bgra: bool) -> Option<Texel> {
    use super::es3_formats::{half_to_f32, unpack_r11g11b10f, unpack_rgb9e5};
    let u16_at = |o: usize| u16::from_ne_bytes([p[o], p[o + 1]]);
    let u32_at = |o: usize| u32::from_ne_bytes([p[o], p[o + 1], p[o + 2], p[o + 3]]);
    let h = |o: usize| half_to_f32(u16_at(o));
    let f = |o: usize| f32::from_bits(u32_at(o));
    let n8 = |b: u8| f32::from(b) / 255.0;
    let swap = |v: [f32; 4]| if bgra { [v[2], v[1], v[0], v[3]] } else { v };
    Some(match texfmt {
        GLES_TEXFMT_RGBA8 => Texel::F(swap([n8(p[0]), n8(p[1]), n8(p[2]), n8(p[3])])),
        GLES_TEXFMT_SRGB8_A8 => {
            let d = super::srgb::decode;
            Texel::F(swap([d(p[0]), d(p[1]), d(p[2]), n8(p[3])]))
        }
        GLES_TEXFMT_L8 => Texel::F([n8(p[0]), n8(p[0]), n8(p[0]), 1.0]),
        GLES_TEXFMT_A8 => Texel::F([0.0, 0.0, 0.0, n8(p[0])]),
        GLES_TEXFMT_LA8 => Texel::F([n8(p[0]), n8(p[0]), n8(p[0]), n8(p[1])]),
        GLES_TEXFMT_RGB565 => {
            let v = u32::from(u16_at(0));
            Texel::F([(v >> 11) as f32 / 31.0, ((v >> 5) & 0x3f) as f32 / 63.0, (v & 0x1f) as f32 / 31.0, 1.0])
        }
        GLES_TEXFMT_RGBA16F => Texel::F([h(0), h(2), h(4), h(6)]),
        GLES_TEXFMT_R16F => Texel::F([h(0), 0.0, 0.0, 1.0]),
        GLES_TEXFMT_R32F => Texel::F([f(0), 0.0, 0.0, 1.0]),
        GLES_TEXFMT_RGBA32F => Texel::F([f(0), f(4), f(8), f(12)]),
        GLES_TEXFMT_R11G11B10F => {
            let [r, g, b] = unpack_r11g11b10f(u32_at(0));
            Texel::F([r, g, b, 1.0])
        }
        GLES_TEXFMT_RGB9E5 => {
            let [r, g, b] = unpack_rgb9e5(u32_at(0));
            Texel::F([r, g, b, 1.0])
        }
        GLES_TEXFMT_RGB10A2 => {
            let v = u32_at(0);
            let c = |s: u32, m: u32| ((v >> s) & m) as f32 / m as f32;
            Texel::F([c(0, 0x3ff), c(10, 0x3ff), c(20, 0x3ff), c(30, 3)])
        }
        /* ES 3.0.6 equation 2.2: max(c / 127, -1) */
        GLES_TEXFMT_RGBA8_SNORM => Texel::F(core::array::from_fn(|k| (p[k] as i8 as f32 / 127.0).max(-1.0))),
        GLES_TEXFMT_RGB10A2UI => {
            let v = u32_at(0);
            Texel::I([v & 0x3ff, (v >> 10) & 0x3ff, (v >> 20) & 0x3ff, v >> 30])
        }
        GLES_TEXFMT_RGBA8UI => Texel::I(core::array::from_fn(|k| u32::from(p[k]))),
        GLES_TEXFMT_RGBA8I => Texel::I(core::array::from_fn(|k| p[k] as i8 as i32 as u32)),
        GLES_TEXFMT_RGBA16UI => Texel::I(core::array::from_fn(|k| u32::from(u16_at(2 * k)))),
        GLES_TEXFMT_RGBA16I => Texel::I(core::array::from_fn(|k| u16_at(2 * k) as i16 as i32 as u32)),
        GLES_TEXFMT_RGBA32UI | GLES_TEXFMT_RGBA32I => Texel::I(core::array::from_fn(|k| u32_at(4 * k))),
        _ => return None,
    })
}

/// The name a refusal gives a texture's format: its ES 3.0 sized format,
/// or the format it was specified with.
fn format_name(t: &GlTexture) -> &'static str {
    if let Some(e) = super::es3_formats::by_index(t.es3_fmt) {
        return e.name;
    }
    match t.format {
        GL_RGBA => "GL_RGBA",
        GL_RGB => "GL_RGB",
        GL_RG => "GL_RG",
        GL_RED => "GL_RED",
        GL_LUMINANCE => "GL_LUMINANCE",
        GL_ALPHA => "GL_ALPHA",
        GL_LUMINANCE_ALPHA => "GL_LUMINANCE_ALPHA",
        GL_DEPTH_COMPONENT => "GL_DEPTH_COMPONENT",
        GL_DEPTH_STENCIL => "GL_DEPTH_STENCIL",
        GL_ETC1_RGB8_OES => "GL_ETC1_RGB8_OES",
        GL_COMPRESSED_RGB8_ETC2 => "GL_COMPRESSED_RGB8_ETC2",
        GL_COMPRESSED_RGBA8_ETC2_EAC => "GL_COMPRESSED_RGBA8_ETC2_EAC",
        GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2 => "GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2",
        _ => "a format without a name here",
    }
}

/// Does minification filter `f` read mipmaps?
fn mipmapped(f: GLenum) -> bool {
    matches!(
        f,
        GL_NEAREST_MIPMAP_NEAREST | GL_LINEAR_MIPMAP_NEAREST | GL_NEAREST_MIPMAP_LINEAR | GL_LINEAR_MIPMAP_LINEAR
    )
}

fn comp_name(k: Comp) -> &'static str {
    match k {
        Comp::Float => "float",
        Comp::Int => "signed integer",
        Comp::Uint => "unsigned integer",
    }
}

/// Resolve every sampler binding the module reads, as the draw binds it
/// (`draw.rs` `build_common`): the binding's sampler named in the linked
/// vertex stage, the unit its glUniform1i chose, the texture bound there at
/// the sampler's target (0: the target's default texture), the sampler
/// object's filters and wraps when one is bound. What the capture cannot
/// read is refused here, before any vertex runs.
fn bind_textures<'a>(
    c: &mut GlContext,
    a: &Arena<'a>,
    m: &Module<'_>,
    program: GLuint,
) -> Result<&'a [Bound], Refusal> {
    let oom = |_| refuse!("out of the shader compiler's scratch memory");
    let mut list = arena_vec::<Bound>(a, 0)?;
    for op in m.instrs.iter() {
        let (sampler, dim, ret) = match *op {
            Op::Tex { sampler, dim, ret, .. } => (sampler, dim, Some(ret)),
            Op::TexSize { sampler, dim, .. } => (sampler, dim, None),
            _ => continue,
        };
        let at = match list.as_slice().iter().position(|b| b.sampler == sampler) {
            Some(k) => k,
            None => {
                let b = resolve_sampler(c, m, program, sampler, dim)?;
                list.push(a, b).map_err(oom)?;
                list.len() - 1
            }
        };
        let b = list[at];
        if b.dim != dim {
            return Err(refuse!("sampler binding {sampler} is read as two texture targets"));
        }
        let want = match ret {
            Some(ScalarType::I32) => Comp::Int,
            Some(ScalarType::U32) => Comp::Uint,
            Some(_) => Comp::Float,
            None => continue, /* textureSize reads any texture */
        };
        if !b.incomplete && b.comp != want {
            return Err(refuse!(
                "sampler binding {sampler}: a {} sampler reads a {} texture (undefined in GL)",
                comp_name(want),
                comp_name(b.comp)
            ));
        }
    }
    Ok(list.into_slice())
}

/// One sampler binding of the module, resolved ([`bind_textures`]).
fn resolve_sampler(
    c: &mut GlContext,
    m: &Module<'_>,
    program: GLuint,
    sampler: u8,
    dim: TexDim,
) -> Result<Bound, Refusal> {
    /* the IR's sampler (element k of an array), then the linked stage's
     * binding of the same name: the two layouts are one compiler's when the
     * stage was compiled online, and may differ when it came from the table */
    let Some(decl) = m.uniforms.iter().find(|u| {
        u.dword_off == SHC_UNIFORM_NO_BLOCK
            && sampler >= u.binding
            && u32::from(sampler) < u32::from(u.binding) + u32::from(u.array_len.max(1))
    }) else {
        return Err(refuse!("sampler binding {sampler} is no sampler of the lowered vertex shader"));
    };
    let k = usize::from(sampler - decl.binding);
    let unit = {
        let p = &c.objs_ref().programs[obj_index(program)];
        let row = p
            .rvs
            .uniform_table()
            .iter()
            .find(|r| r.dword_off == SHC_UNIFORM_NO_BLOCK && util::name_bytes(&r.name) == decl.name.as_bytes());
        let Some(row) = row else {
            return Err(refuse!("sampler {} is not a sampler of the linked vertex stage", decl.name));
        };
        match p.vs_sampler_unit.get(usize::from(row.binding) + k) {
            Some(&u) => usize::from(u),
            None => return Err(refuse!("sampler {} has binding {}, past the samplers", decl.name, row.binding)),
        }
    };
    let (bound, target) = match dim {
        TexDim::Cube => (c.bound_texture_cube.get(unit), GL_TEXTURE_CUBE_MAP),
        TexDim::D2Array => (c.bound_texture_2d_array.get(unit), GL_TEXTURE_2D_ARRAY),
        TexDim::D3 => (c.bound_texture_3d.get(unit), GL_TEXTURE_3D),
        TexDim::D2 => (c.bound_texture_2d.get(unit), GL_TEXTURE_2D),
    };
    let Some(&bound) = bound else {
        return Err(refuse!("sampler {} reads texture unit {unit}, past the units", decl.name));
    };
    let slot = if bound == 0 { super::texture::default_slot(target) } else { Some(bound as usize) };
    let none = Bound { sampler, dim, ..Bound::NONE };
    let Some(ti) = slot.filter(|&s| s < TEXTURE_SLOTS && c.objs_ref().textures[s].used) else {
        return Ok(none); /* no texture object: (0, 0, 0, 1), as the draw's black 1x1 */
    };
    if !c.objs_ref().textures[ti].has_image() {
        return Ok(none); /* no storage: no image was ever specified */
    }
    settle_for_read(c, ti)?;
    let sampler_state = super::es3::sampler_state(c.index, unit);
    let t = &c.objs_ref().textures[ti];
    let mut b = bound_of(t, sampler, dim, sampler_state).map_err(|why| refuse!("sampler {}: {why}", decl.name))?;
    if let Some(compare) = super::es3::sampler_compare(c.index, unit) {
        b.compare = compare;
    }
    // SAFETY: the texture's whole CPU mapping; the kick that rendered into
    // it completed (settle_for_read).
    unsafe { crate::osal::cache_invd(t.mem.cpu, t.mem.size) };
    Ok(b)
}

/// The [`Bound`] of texture `t` read at sampler binding `sampler` as a
/// `dim` sampler, with `sampler_state` (a sampler object's min, mag, wrap S,
/// wrap T and wrap R) in place of the texture's own when one is bound; or
/// why the capture cannot read it. The caller settled the texels
/// ([`settle_for_read`]) and keeps `t` alive and its storage in place while
/// the bound is used.
pub(super) fn bound_of(
    t: &GlTexture,
    sampler: u8,
    dim: TexDim,
    sampler_state: Option<(GLenum, GLenum, GLenum, GLenum, GLenum)>,
) -> Result<Bound, Fault> {
    let name = format_name(t);
    if t.block_bytes != 0 {
        return Err(fault(format_args!("a compressed texture ({name}), which the capture does not decode")));
    }
    if decode(&[0u8; 16], t.texfmt, false).is_none() {
        return Err(fault(format_args!("texture format {name} (storage class {}) is not decoded", t.texfmt)));
    }
    if t.mem.cpu.is_null() {
        return Err(fault(format_args!("the texture's storage has no CPU view")));
    }
    /* GL_TEXTURE_BASE_LEVEL (3.8.13): the chain starts at that level; a base
     * past the storage's levels is an incomplete texture */
    let base_out = t.base_level < 0 || t.base_level as usize >= CAP_MIP_LEVELS;
    let base = if base_out { 0 } else { t.base_level as u32 };
    let (min_filter, mag_filter, wrap_s, wrap_t, wrap_r) =
        sampler_state.unwrap_or((t.min_filter, t.mag_filter, t.wrap_s, t.wrap_t, t.wrap_r));
    let comp = match super::es3_formats::by_index(t.es3_fmt).map(|e| e.class) {
        Some(super::es3_formats::Class::Int) => Comp::Int,
        Some(super::es3_formats::Class::Uint) => Comp::Uint,
        _ => Comp::Float,
    };
    let layers0 = if dim == TexDim::D2 { 1 } else { u32::from(t.layers).max(1) };
    /* the base level's sizes; an array's layers and a cube's faces are the
     * same at every level */
    let (width, height) = (side(t.width, base), side(t.height, base));
    let layers = if dim == TexDim::D3 { side(layers0, base) } else { layers0 };
    /* the chain's levels from the base, q + 1 (draw.rs `mip_levels_needed`,
     * with a 3D texture's depth in p as ES 3.0.6 3.8.14 has it) */
    let deep = if dim == TexDim::D3 { layers } else { 1 };
    let p = u32::from(super::context::gl_mip_levels(width.max(deep), height)).min(CAP_MIP_LEVELS as u32 - base);
    let last = u32::try_from(t.max_level_set).unwrap_or(0).saturating_add(1).saturating_sub(base);
    let mut needed = p.min(last);
    if t.immutable && t.immutable_levels > 0 {
        needed = needed.min(u32::from(t.immutable_levels).saturating_sub(base));
    }
    let needed = needed.max(1);
    let mut level_off = [0; CAP_MIP_LEVELS];
    for (l, off) in level_off.iter_mut().enumerate() {
        *off = t.level_off.get(l + base as usize).copied().unwrap_or(0);
    }
    let mut b = Bound {
        sampler,
        dim,
        incomplete: base_out || incomplete(t, base, min_filter, mag_filter, needed),
        base: t.mem.cpu as *const u8,
        bytes: t.mem.size,
        width,
        height,
        layers,
        grid_cols: u32::from(t.grid_cols),
        level_off,
        pitch: [0; CAP_MIP_LEVELS],
        level_ok: 0,
        q: needed - 1,
        bpp: usize::from(t.bpp.max(1)),
        texfmt: t.texfmt,
        comp,
        format: t.format,
        bgra: t.bgra || t.pe_order,
        no_alpha: comp == Comp::Float && super::draw::texture_has_no_alpha(t),
        depth_luminance: super::draw::texture_depth_luminance(t),
        float_depth: matches!(t.type_, GL_FLOAT | GL_FLOAT_32_UNSIGNED_INT_24_8_REV),
        min_filter,
        mag_filter,
        wrap: [wrap_s, wrap_t, wrap_r],
        min_lod: t.min_lod,
        max_lod: t.max_lod,
        swizzle: t.swizzle,
        compare: (t.compare_mode, t.compare_func),
    };
    for l in 0..needed {
        let abs = l + base;
        let lw = side(t.width, abs);
        /* texture.rs `level_row_pitch` of the level's rows: a grid's rows
         * hold `grid_cols` slices side by side */
        let rows = if t.grid_cols != 0 { lw * u32::from(t.grid_cols) } else { lw };
        b.pitch[l as usize] = if t.pot_chain { t.level_pitch_at(abs) } else { t.level_pitch(rows) };
        let bit = 1u16 << l;
        let abit = 1u16 << abs;
        let held =
            if t.grid_cols != 0 { abs == 0 || t.level_off[abs as usize] != 0 } else { abs < u32::from(t.chain_levels) };
        let faces = dim != TexDim::Cube || abs == 0 || t.level_faces[abs as usize] == 0x3f;
        if t.level_defined & abit != 0 && t.level_bad & abit == 0 && held && faces {
            b.level_ok |= bit;
        }
    }
    Ok(b)
}

/// Is the texture incomplete under these filters (ES 3.0.6 3.8.13)? The
/// rules of `draw.rs` `incomplete_reason`, which a draw applies to the same
/// texture, with `needed` levels for a mipmap filter.
fn incomplete(t: &GlTexture, base: u32, min_filter: GLenum, mag_filter: GLenum, needed: u32) -> bool {
    let cube = t.target == GL_TEXTURE_CUBE_MAP;
    if cube && t.faces_set != 0x3f {
        return true;
    }
    /* the base level's faces, when it is not level 0 */
    if cube && base > 0 && t.level_faces[base as usize] != 0x3f {
        return true;
    }
    if (t.level_defined >> base) & 1 == 0 || (t.level_bad >> base) & 1 != 0 || t.width == 0 || t.height == 0 {
        return true;
    }
    if super::es3_formats::by_index(t.es3_fmt).is_some_and(|e| e.incomplete_with(min_filter, mag_filter)) {
        return true;
    }
    if mipmapped(min_filter) {
        let chain = ((1u32 << needed) - 1) as u16;
        if (t.level_defined >> base) & chain != chain || (t.level_bad >> base) & chain != 0 {
            return true;
        }
        let first = base as usize + 1;
        let end = (base as usize + needed as usize).min(t.level_faces.len());
        if cube && t.level_faces[first.min(end)..end].iter().any(|&f| f != 0x3f) {
            return true;
        }
    }
    false
}

/// Before the CPU reads texture `ti`'s texels: the glClear waiting to be
/// merged into the next draw goes out when the texture is its target, the
/// depth a kick rendered into it is resolved into its texels, and the kick
/// that last rendered into it is waited for (`texture.rs` `settle_texture`,
/// the read half: a kick that only samples it changes nothing).
fn settle_for_read(c: &mut GlContext, ti: usize) -> Result<(), Refusal> {
    let is_target = c.bound_framebuffer != 0
        && c.framebuffer_ref(c.bound_framebuffer).is_some_and(|fb| {
            core::iter::once(&fb.color)
                .chain(fb.color_n.iter())
                .chain(core::iter::once(&fb.depth))
                .any(|a| a.type_ == GL_TEXTURE && a.name as usize == ti)
        });
    if is_target {
        super::draw::flush_pending_clear(c);
    }
    super::texture::resolve_depth(c, ti);
    /* a deferred depth store is allocated by its first reader */
    if !super::texture::ensure_storage_at(c, ti) {
        return Err(refuse!("no memory for the storage of a texture the vertex shader samples"));
    }
    let kick = super::framebuffer::texture_target_use(c, ti);
    if backend::kick_pending(kick) && (backend::flush() != 0 || backend::finish() != 0) {
        return Err(refuse!("the kick rendering into a texture the vertex shader samples did not complete"));
    }
    /* level 0 a backend's jobs stored in its image alone */
    super::texture::storage_back(c, ti);
    Ok(())
}
