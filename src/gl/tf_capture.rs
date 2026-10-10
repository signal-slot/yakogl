// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! OpenGL ES 3.0 transform feedback capture, the data path
//! (`docs/es3-transform-feedback.md` section 4, item tf-capture): the GL core
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
//! What it refuses (GL_INVALID_OPERATION, the reason in the log; nothing is
//! counted and the draw is not drawn): a vertex shader that samples a
//! texture (the evaluator reads no texture storage), an attribute element
//! outside its buffer, a uniform the evaluated shader reads that the linked
//! stage does not lay out the same way, and anything the evaluator itself
//! stops at. ES 3.0 has no such error; `src/GL_CORE_NOTES.md` lists it.

use super::consts::*;
use super::context::{obj_index, GlContext, MAX_VERTEX_ATTRIBS};
use super::tf::TfRow;
use super::types::*;
use crate::shaderc::online::arena::{AVec, Arena};
use crate::shaderc::online::eval::{self, Fault, Frame, Inputs, Sink};
use crate::shaderc::online::ir::{IoSlot, Module, Op};
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
    if m.instrs.iter().any(|op| matches!(op, Op::Tex { .. } | Op::TexSize { .. })) {
        return Err(refuse!("the vertex shader samples a texture, which the capture does not evaluate"));
    }
    let uniforms = uniform_block(c, a, m, d.program)?;

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
                    eval::run_in(m, &Inputs { io: io.as_slice(), uniforms }, frame, &mut sink)
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
