// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! OpenGL ES 3.0 transform feedback, the GL-layer half (`docs/es3-transform-feedback.md`
//! section 5): the transform feedback objects with their indexed buffer
//! bindings and begin/pause/resume state, the varyings a program is asked to
//! capture and what the link resolves them to, and the draw-time rules.
//!
//! What is NOT here is the capture itself (docs/HANDOFF.md, open work 4,
//! tf-capture): no data is ever
//! written into a bound buffer. A draw that would capture (an object active
//! and not paused, every rule of 2.15 satisfied) is refused with
//! GL_INVALID_OPERATION and a log line naming this gap, so a test fails
//! loudly instead of reading a buffer that nothing filled in
//! (`es3.rs`: "a test that fails loudly is a measurement").
//!
//! Everything in this module is reached only when `crate::REPORT_ES2` is
//! false: an ES 2.0-reporting build keeps its old answers (the callers test
//! it), so the pools and tables here stay zero there.
//!
//! The state lives in Rust-only statics, as the other ES 3.0 pools do
//! (`es3.rs`), so `GlContext`'s C layout does not change. The transform
//! feedback objects are per context (not shared); the generic
//! GL_TRANSFORM_FEEDBACK_BUFFER binding is `GlContext::bound_transform_feedback_buffer`.

use core::ffi::c_char;

use super::consts::*;
use super::context::{obj_index, GlContext, MAX_BUFFERS, MAX_CONTEXTS, MAX_PROGRAMS};
use super::names::{self, Kind};
use super::types::*;
use crate::util::{self, Global};

/// GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS (ES 3.0 minimum).
pub const TF_MAX_INTERLEAVED_COMPONENTS: u32 = 64;
/// GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS (ES 3.0 minimum); also the
/// number of indexed GL_TRANSFORM_FEEDBACK_BUFFER binding points.
pub const TF_MAX_SEPARATE_ATTRIBS: u32 = 4;
/// GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS (ES 3.0 minimum).
pub const TF_MAX_SEPARATE_COMPONENTS: u32 = 4;

/// Transform feedback objects of a context, the default object (name 0)
/// included. dEQP's lifetime group generates several.
pub(super) const MAX_FEEDBACKS: usize = 32;

const BINDINGS: usize = TF_MAX_SEPARATE_ATTRIBS as usize;
/// The longest varying name the program keeps (with its NUL).
pub(super) const NAME_MAX: usize = 64;
/// Rows a program keeps: one per captured varying; every row takes at least
/// one component and the interleaved limit is 64.
#[cfg(feature = "shaderc-online")]
const MAX_ROWS: usize = TF_MAX_INTERLEAVED_COMPONENTS as usize;

/* ------------------------------------------------------------------ objects */

/// One indexed GL_TRANSFORM_FEEDBACK_BUFFER binding point of an object:
/// the buffer's table slot, and the range (`size` 0 is "the whole buffer at
/// draw time", what glBindBufferBase records).
#[derive(Clone, Copy)]
struct TfBinding {
    buffer: GLuint,
    offset: GLintptr,
    size: GLsizeiptr,
}

impl TfBinding {
    const NONE: TfBinding = TfBinding { buffer: 0, offset: 0, size: 0 };
}

/// A transform feedback object. A name glGenTransformFeedbacks handed out
/// is an object (`isTransformFeedback`) only once it has been bound.
#[derive(Clone, Copy)]
struct TfObject {
    used: bool,
    bound: bool,
    active: bool,
    paused: bool,
    /// GL_POINTS / GL_LINES / GL_TRIANGLES given to glBeginTransformFeedback.
    prim_mode: GLenum,
    /// The program that was current at Begin (its handle).
    program: GLuint,
    bind: [TfBinding; BINDINGS],
    /// Bytes written since Begin, per binding (tf-capture's).
    written: [u32; BINDINGS],
}

impl TfObject {
    const FREE: TfObject = TfObject {
        used: false,
        bound: false,
        active: false,
        paused: false,
        prim_mode: 0,
        program: 0,
        bind: [TfBinding::NONE; BINDINGS],
        written: [0; BINDINGS],
    };
    const NEW: TfObject = TfObject { used: true, ..TfObject::FREE };
}

static OBJECTS: Global<[[TfObject; MAX_FEEDBACKS]; MAX_CONTEXTS]> = Global::zeroed();
static BOUND: Global<[GLuint; MAX_CONTEXTS]> = Global::zeroed();
/// Buffer slots whose name was deleted while a transform feedback object
/// that was not current still held them: the storage lives until the last
/// object lets go (per context; the objects are per context).
static ORPHANS: Global<[[bool; MAX_BUFFERS]; MAX_CONTEXTS]> = Global::zeroed();

fn objects(index: u8) -> &'static mut [TfObject; MAX_FEEDBACKS] {
    // SAFETY: single-threaded by contract; callers keep no other reference to the pool live.
    unsafe { &mut OBJECTS.get()[index as usize % MAX_CONTEXTS] }
}

fn bound_name(index: u8) -> &'static mut GLuint {
    // SAFETY: as in `objects`.
    unsafe { &mut BOUND.get()[index as usize % MAX_CONTEXTS] }
}

fn orphans(index: u8) -> &'static mut [bool; MAX_BUFFERS] {
    // SAFETY: as in `objects`.
    unsafe { &mut ORPHANS.get()[index as usize % MAX_CONTEXTS] }
}

/// The bound object of context `c`.
fn current(c: &GlContext) -> &'static mut TfObject {
    let b = *bound_name(c.index) as usize;
    &mut objects(c.index)[b.min(MAX_FEEDBACKS - 1)]
}

/// A new context slot starts with only the default object and nothing bound.
pub(super) fn reset_context(index: u8) {
    *objects(index) = [TfObject::FREE; MAX_FEEDBACKS];
    objects(index)[0] = TfObject::NEW;
    *bound_name(index) = 0;
    *orphans(index) = [false; MAX_BUFFERS];
}

/// Unit tests: empty everything, the default object included.
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised.
    unsafe {
        for i in 0..MAX_CONTEXTS {
            reset_context(i as u8);
        }
        let progs = PROGS.get();
        for g in progs.iter_mut() {
            for p in g.iter_mut() {
                *p = TfProg::NONE;
            }
        }
    }
}

/// GL_TRANSFORM_FEEDBACK_BINDING.
pub(super) fn binding(c: &GlContext) -> GLuint {
    *bound_name(c.index)
}

/// The bound object is active (paused or not).
pub(super) fn is_active(c: &GlContext) -> bool {
    !crate::REPORT_ES2 && current(c).active
}

/// The bound object is paused (GL_TRANSFORM_FEEDBACK_PAUSED).
pub(super) fn is_paused(c: &GlContext) -> bool {
    !crate::REPORT_ES2 && current(c).paused
}

/// Draws capture: the bound object is active and not paused.
pub(super) fn capturing(c: &GlContext) -> bool {
    !crate::REPORT_ES2 && current(c).active && !current(c).paused
}

/// glGenTransformFeedbacks.
///
/// # Safety
/// `ids` must be null or valid for writing `n` GLuints (any alignment).
pub(super) unsafe fn gen(c: &mut GlContext, n: GLsizei, ids: *mut GLuint) {
    if ids.is_null() {
        return;
    }
    if n < 0 {
        c.set_error(GL_INVALID_VALUE, "glGenTransformFeedbacks");
        return;
    }
    let pool = objects(c.index);
    let mut made = 0usize;
    for (h, slot) in pool.iter_mut().enumerate().skip(1) {
        if made >= n as usize {
            break;
        }
        if !slot.used {
            *slot = TfObject::NEW;
            util::app_set(ids, made, h as GLuint);
            made += 1;
        }
    }
    for i in made..n as usize {
        util::app_set(ids, i, 0);
    }
}

/// glIsTransformFeedback.
pub(super) fn is(c: &GlContext, id: GLuint) -> bool {
    let h = id as usize;
    h != 0 && h < MAX_FEEDBACKS && objects(c.index)[h].bound
}

/// glBindTransformFeedback (ES 3.0 2.15.1): the target, then a bound object
/// that is active and not paused, then a name that was never generated.
pub(super) fn bind(c: &mut GlContext, target: GLenum, id: GLuint) {
    const FUNC: &str = "glBindTransformFeedback";
    if target != GL_TRANSFORM_FEEDBACK {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if capturing(c) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let h = id as usize;
    if h >= MAX_FEEDBACKS || (h != 0 && !objects(c.index)[h].used) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    if h != 0 {
        objects(c.index)[h].bound = true;
    }
    *bound_name(c.index) = id;
}

/// glDeleteTransformFeedbacks (ES 3.0 2.15.1): an active object is an error
/// and survives. Mesa walks the names in order and stops at the first active
/// one, so those before it are already deleted (`transformfeedback.c`).
///
/// # Safety
/// `ids` must be null or valid for reading `n` GLuints (any alignment).
pub(super) unsafe fn delete(c: &mut GlContext, n: GLsizei, ids: *const GLuint) {
    const FUNC: &str = "glDeleteTransformFeedbacks";
    if n < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if ids.is_null() {
        return;
    }
    for i in 0..n as usize {
        let h = util::app_get(ids, i) as usize;
        if h == 0 || h >= MAX_FEEDBACKS || !objects(c.index)[h].used {
            continue;
        }
        if objects(c.index)[h].active {
            c.set_error(GL_INVALID_OPERATION, FUNC);
            return;
        }
        let held = objects(c.index)[h].bind;
        objects(c.index)[h] = TfObject::FREE;
        if *bound_name(c.index) as usize == h {
            *bound_name(c.index) = 0;
        }
        for b in held {
            release_slot(c, b.buffer);
        }
    }
}

/* ------------------------------------------------------------------ buffers */

/// glBindBufferBase / glBindBufferRange for GL_TRANSFORM_FEEDBACK_BUFFER
/// (ES 3.0 2.15.2): the indexed point of the bound object, and the generic
/// binding. `size` 0 is Base. The argument errors that need no state were
/// raised by the caller; a bound object that is active (paused or not)
/// comes before the index (Mesa, `transformfeedback.c`).
pub(super) fn bind_buffer(
    c: &mut GlContext,
    index: GLuint,
    buffer: GLuint,
    offset: GLintptr,
    size: GLsizeiptr,
    ranged: bool,
    func: &str,
) {
    if current(c).active {
        c.set_error(GL_INVALID_OPERATION, func);
        return;
    }
    if index as usize >= BINDINGS {
        c.set_error(GL_INVALID_VALUE, func);
        return;
    }
    if ranged && (offset % 4 != 0 || size % 4 != 0) {
        c.set_error(GL_INVALID_VALUE, func);
        return;
    }
    // SAFETY: a plain GL call; it records its own errors.
    unsafe { super::buffer::glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, buffer) };
    /* the binding holds the name's slot (gl/names.rs) */
    let slot = match (buffer, names::slot_of(c, Kind::Buffer, buffer)) {
        (0, _) => 0,
        (_, Some(s)) if c.bound_transform_feedback_buffer == s => s,
        /* glBindBuffer refused the name */
        _ => return,
    };
    let old = current(c).bind[index as usize];
    current(c).bind[index as usize] =
        if slot == 0 { TfBinding::NONE } else { TfBinding { buffer: slot, offset, size } };
    release_slot(c, old.buffer);
}

/// Bytes of the whole buffer slot `slot`, 0 when it is gone.
fn buffer_size(c: &GlContext, slot: GLuint) -> i64 {
    c.objs_ref().buffers.get(slot as usize).filter(|b| b.used).map_or(0, |b| b.size.max(0) as i64)
}

/// glGetIntegeri_v / glGetInteger64i_v of the indexed transform feedback
/// pnames, from the bound object. `Err(GL_INVALID_VALUE)` for an index past
/// the binding points.
pub(super) fn indexed_state(c: &GlContext, target: GLenum, index: GLuint) -> Result<GLint64, GLenum> {
    let Some(b) = current(c).bind.get(index as usize).copied() else {
        return Err(GL_INVALID_VALUE);
    };
    Ok(match target {
        GL_TRANSFORM_FEEDBACK_BUFFER_BINDING => GLint64::from(names::name_of(c, Kind::Buffer, b.buffer)),
        GL_TRANSFORM_FEEDBACK_BUFFER_START => b.offset as GLint64,
        _ => b.size as GLint64,
    })
}

/// A buffer slot is being deleted. The bindings of the bound object (and the
/// generic one, which the caller clears) revert to zero; another object keeps
/// its reference, so the storage must outlive the name (ES 3.0 D.1.2).
/// Returns true when the storage has to be kept (`orphan`): the caller frees
/// it later through [`release_slot`].
pub(super) fn on_buffer_delete(c: &mut GlContext, slot: GLuint) -> bool {
    if crate::REPORT_ES2 {
        return false;
    }
    let cur = *bound_name(c.index) as usize;
    for b in current(c).bind.iter_mut() {
        if b.buffer == slot {
            *b = TfBinding::NONE;
        }
    }
    let held =
        objects(c.index).iter().enumerate().any(|(h, o)| h != cur && o.used && o.bind.iter().any(|b| b.buffer == slot));
    if held {
        orphans(c.index)[slot as usize % MAX_BUFFERS] = true;
    }
    held
}

/// An object let go of `slot`: free an orphaned buffer once nobody holds it.
fn release_slot(c: &mut GlContext, slot: GLuint) {
    if slot == 0 || !orphans(c.index)[slot as usize % MAX_BUFFERS] {
        return;
    }
    if objects(c.index).iter().any(|o| o.used && o.bind.iter().any(|b| b.buffer == slot)) {
        return;
    }
    orphans(c.index)[slot as usize % MAX_BUFFERS] = false;
    super::buffer::free_orphan(c, slot);
}

/* ---------------------------------------------------------------- varyings */

/// What the link made of one requested name.
#[derive(Clone, Copy)]
pub(super) struct TfRow {
    name: [u8; NAME_MAX],
    name_len: u8,
    /// glGetTransformFeedbackVarying's type and size (array elements).
    gl_type: GLenum,
    size: u32,
    /// Scalar components of one element.
    comps: u32,
    /// The first element captured (`v[1]`), 0 for a whole variable.
    first: u32,
    /// 0 a vertex output, 1 gl_Position, 2 gl_PointSize.
    builtin: u8,
}

impl TfRow {
    pub(super) fn name(&self) -> &[u8] {
        &self.name[..self.name_len as usize]
    }
    /// Scalar components the row captures per vertex.
    pub(super) fn components(&self) -> u32 {
        self.comps * self.size
    }
    #[allow(dead_code)] // tf-capture
    pub(super) fn first_element(&self) -> u32 {
        self.first
    }
    #[allow(dead_code)] // tf-capture
    pub(super) fn builtin(&self) -> u8 {
        self.builtin
    }
}

/// A program's transform feedback state. Heap blocks (`util::malloc`) hold
/// the variable-length parts; zero is a valid, empty value.
#[derive(Clone, Copy)]
struct TfProg {
    /// glTransformFeedbackVaryings's names, NUL-separated, applied by the
    /// next link and kept for the links after it.
    pending: *mut u8,
    pending_bytes: u32,
    pending_n: u32,
    pending_mode: GLenum,
    /// The last successful link's rows.
    rows: *mut TfRow,
    nrows: u32,
    mode: GLenum,
}

impl TfProg {
    const NONE: TfProg = TfProg {
        pending: core::ptr::null_mut(),
        pending_bytes: 0,
        pending_n: 0,
        pending_mode: 0,
        rows: core::ptr::null_mut(),
        nrows: 0,
        mode: 0,
    };
}

static PROGS: Global<[[TfProg; MAX_PROGRAMS]; MAX_CONTEXTS]> = Global::zeroed();

fn prog(group: u8, program: GLuint) -> &'static mut TfProg {
    // SAFETY: single-threaded by contract; callers keep no other reference live.
    unsafe { &mut PROGS.get()[group as usize % MAX_CONTEXTS][obj_index(program) % MAX_PROGRAMS] }
}

fn free_block<T>(p: &mut *mut T) {
    if !p.is_null() {
        // SAFETY: a block this module took from util::malloc and has not freed.
        unsafe { util::free(*p as *mut core::ffi::c_void) };
        *p = core::ptr::null_mut();
    }
}

/// glCreateProgram / glDeleteProgram: nothing requested, nothing linked.
pub(super) fn program_reset(group: u8, program: GLuint) {
    let p = prog(group, program);
    free_block(&mut p.pending);
    free_block(&mut p.rows);
    *p = TfProg::NONE;
}

/// glTransformFeedbackVaryings (ES 3.0 2.15.3 / 2.11.8): the names are copied
/// now and applied by the next link. The argument checks are the caller's.
///
/// # Safety
/// `varyings` must hold `count` NUL-terminated strings.
pub(super) unsafe fn set_varyings(
    c: &mut GlContext,
    program: GLuint,
    count: GLsizei,
    varyings: *const *const GLchar,
    mode: GLenum,
) {
    let p = prog(c.group, program);
    free_block(&mut p.pending);
    p.pending_bytes = 0;
    p.pending_n = 0;
    p.pending_mode = mode;
    if count <= 0 {
        return;
    }
    let mut total = 0usize;
    for i in 0..count as usize {
        let s = util::app_get(varyings, i);
        total += if s.is_null() { 1 } else { util::cstr_len(s) + 1 };
    }
    let block = util::malloc(total) as *mut u8;
    if block.is_null() {
        c.set_error(GL_OUT_OF_MEMORY, "glTransformFeedbackVaryings");
        return;
    }
    let mut at = 0usize;
    for i in 0..count as usize {
        let s = util::app_get(varyings, i);
        let n = if s.is_null() { 0 } else { util::cstr_len(s) };
        if n > 0 {
            core::ptr::copy_nonoverlapping(s as *const u8, block.add(at), n);
        }
        *block.add(at + n) = 0;
        at += n + 1;
    }
    p.pending = block;
    p.pending_bytes = total as u32;
    p.pending_n = count as u32;
}

/// Is `program` the program of an active transform feedback object (any
/// object, bound or not, paused or not)? glLinkProgram then fails.
pub(super) fn program_in_use(c: &GlContext, program: GLuint) -> bool {
    !crate::REPORT_ES2 && objects(c.index).iter().any(|o| o.used && o.active && o.program == program)
}

/// A link starts: the previous result is gone, whatever happens next.
pub(super) fn link_begin(group: u8, program: GLuint) {
    let p = prog(group, program);
    free_block(&mut p.rows);
    p.nrows = 0;
}

/// Scalar components of a GLSL type given by its GL token.
#[cfg(feature = "shaderc-online")]
fn type_components(t: GLenum) -> u32 {
    match t {
        GL_FLOAT | GL_INT | GL_UNSIGNED_INT => 1,
        GL_FLOAT_VEC2 | GL_INT_VEC2 | GL_UNSIGNED_INT_VEC2 => 2,
        GL_FLOAT_VEC3 | GL_INT_VEC3 | GL_UNSIGNED_INT_VEC3 => 3,
        GL_FLOAT_VEC4 | GL_INT_VEC4 | GL_UNSIGNED_INT_VEC4 | GL_FLOAT_MAT2 => 4,
        GL_FLOAT_MAT2x3 | GL_FLOAT_MAT3x2 => 6,
        GL_FLOAT_MAT2x4 | GL_FLOAT_MAT4x2 => 8,
        GL_FLOAT_MAT3 => 9,
        GL_FLOAT_MAT3x4 | GL_FLOAT_MAT4x3 => 12,
        GL_FLOAT_MAT4 => 16,
        _ => 0,
    }
}

/// `v[3]`: the name before the subscript and the index.
#[cfg(feature = "shaderc-online")]
fn split_element(name: &[u8]) -> Option<(&[u8], u32)> {
    let body = name.strip_suffix(b"]")?;
    let lb = body.iter().rposition(|&b| b == b'[')?;
    let digits = &body[lb + 1..];
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let mut v: u32 = 0;
    for d in digits {
        v = v.saturating_mul(10).saturating_add(u32::from(d - b'0'));
    }
    Some((&body[..lb], v))
}

/// Why a link of requested varyings failed, as the info log says it.
type Say<'a> = &'a mut dyn FnMut(core::fmt::Arguments);

/// Resolve the requested names against the vertex shader's declared outputs
/// (ES 3.0 2.11.8) and keep the rows. `vs_src` is the vertex shader's
/// NUL-terminated source. Err: the log has been written.
pub(super) fn link_resolve(group: u8, program: GLuint, vs_src: *const c_char, say: Say) -> Result<(), ()> {
    let p = prog(group, program);
    p.mode = p.pending_mode;
    if p.pending_n == 0 {
        return Ok(());
    }
    let n = p.pending_n as usize;
    if p.pending.is_null() {
        say(format_args!("link error: out of memory for the transform feedback varyings"));
        return Err(());
    }
    // SAFETY: `pending` is the block `set_varyings` filled with `pending_bytes` bytes.
    let names = unsafe { core::slice::from_raw_parts(p.pending, p.pending_bytes as usize) };
    resolve_rows(p, n, names, vs_src, say)
}

#[cfg(not(feature = "shaderc-online"))]
fn resolve_rows(_p: &mut TfProg, _n: usize, _names: &[u8], _vs: *const c_char, say: Say) -> Result<(), ()> {
    say(format_args!(
        "link error: transform feedback varyings need the in-process shader compiler to find the vertex outputs, which this build does not have"
    ));
    Err(())
}

#[cfg(feature = "shaderc-online")]
fn resolve_rows(p: &mut TfProg, n: usize, names: &[u8], vs_src: *const c_char, say: Say) -> Result<(), ()> {
    use crate::shaderc::online::{sema::Stage, store};

    if vs_src.is_null() {
        say(format_args!("link error: the vertex shader's source is gone"));
        return Err(());
    }
    // SAFETY: the shader object's NUL-terminated copy of its source.
    let src = unsafe { core::slice::from_raw_parts(vs_src as *const u8, util::cstr_len(vs_src)) };
    let Ok(src) = core::str::from_utf8(src) else {
        say(format_args!("link error: the vertex shader's source is not text"));
        return Err(());
    };
    let mut rows =
        [TfRow { name: [0; NAME_MAX], name_len: 0, gl_type: 0, size: 0, comps: 0, first: 0, builtin: 0 }; MAX_ROWS];
    let mut count = 0usize;
    let mut failure: Option<()> = None;
    let ran = store::with_scratch(|a| {
        let Ok(text) = a.alloc_str(src) else {
            say(format_args!("link error: out of scratch memory for the transform feedback varyings"));
            failure = Some(());
            return;
        };
        let Ok(iface) = crate::shaderc::online::reflect::interface(a, Stage::Vertex, text) else {
            say(format_args!(
                "link error: the vertex shader does not compile again for its transform feedback varyings"
            ));
            failure = Some(());
            return;
        };
        let outs = iface.reflection.varyings;
        let mut rest = names;
        for _ in 0..n {
            let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            let name = &rest[..end];
            rest = rest.get(end + 1..).unwrap_or(&[]);
            let shown = core::str::from_utf8(name).unwrap_or("?");
            if name.len() >= NAME_MAX {
                say(format_args!("link error: Transform feedback varying {shown} undeclared."));
                failure = Some(());
                return;
            }
            let mut row = TfRow {
                name: [0; NAME_MAX],
                name_len: name.len() as u8,
                gl_type: 0,
                size: 1,
                comps: 0,
                first: 0,
                builtin: 0,
            };
            row.name[..name.len()].copy_from_slice(name);
            match name {
                b"gl_Position" => (row.gl_type, row.comps, row.builtin) = (GL_FLOAT_VEC4, 4, 1),
                b"gl_PointSize" => (row.gl_type, row.comps, row.builtin) = (GL_FLOAT, 1, 2),
                _ => {
                    let whole = outs.iter().find(|v| v.name.as_bytes() == name);
                    let elem = split_element(name)
                        .and_then(|(base, k)| outs.iter().find(|v| v.name.as_bytes() == base).map(|v| (v, k)));
                    match (whole, elem) {
                        (Some(v), _) => {
                            row.gl_type = GLenum::from(v.gl_type);
                            row.comps = type_components(row.gl_type);
                            row.size = u32::from(v.count).max(1);
                        }
                        (None, Some((v, k))) => {
                            if k >= u32::from(v.count).max(1) {
                                say(format_args!(
                                    "link error: Transform feedback varying {shown}: element {k} is outside the array of {}.",
                                    v.count
                                ));
                                failure = Some(());
                                return;
                            }
                            row.gl_type = GLenum::from(v.gl_type);
                            row.comps = type_components(row.gl_type);
                            row.first = k;
                        }
                        (None, None) => {
                            say(format_args!("link error: Transform feedback varying {shown} undeclared."));
                            failure = Some(());
                            return;
                        }
                    }
                    if row.comps == 0 {
                        say(format_args!(
                            "link error: Transform feedback varying {shown} has a type that cannot be captured."
                        ));
                        failure = Some(());
                        return;
                    }
                }
            }
            /* the same variable and the same element twice (Mesa reads the
             * rule so that the elements of one array can be captured apart) */
            let key = |r: &TfRow| -> (u8, [u8; NAME_MAX], u32, bool) {
                let base =
                    split_element(&r.name[..r.name_len as usize]).map_or(&r.name[..r.name_len as usize], |(b, _)| b);
                let mut k = [0u8; NAME_MAX];
                k[..base.len()].copy_from_slice(base);
                (r.builtin, k, r.first, r.name[..r.name_len as usize].ends_with(b"]"))
            };
            if rows[..count].iter().any(|r| key(r) == key(&row)) {
                say(format_args!("link error: Transform feedback varying {shown} specified more than once."));
                failure = Some(());
                return;
            }
            rows[count] = row;
            count += 1;
        }
    });
    if ran.is_none() {
        say(format_args!("link error: the shader compiler is busy; cannot resolve the transform feedback varyings"));
        return Err(());
    }
    if failure.is_some() {
        return Err(());
    }
    /* the limits (ES 3.0 2.11.8) */
    if p.mode == GL_SEPARATE_ATTRIBS {
        if let Some(r) = rows[..count].iter().find(|r| r.components() > TF_MAX_SEPARATE_COMPONENTS) {
            say(format_args!(
                "link error: Transform feedback varying {} exceeds MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS.",
                core::str::from_utf8(r.name()).unwrap_or("?")
            ));
            return Err(());
        }
    } else if rows[..count].iter().map(TfRow::components).sum::<u32>() > TF_MAX_INTERLEAVED_COMPONENTS {
        say(format_args!("link error: The MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS limit has been exceeded."));
        return Err(());
    }
    // SAFETY: a plain heap block, freed by `link_begin` / `program_reset`.
    let block = unsafe { util::malloc(count * core::mem::size_of::<TfRow>()) } as *mut TfRow;
    if block.is_null() {
        say(format_args!("link error: out of memory for the transform feedback varyings"));
        return Err(());
    }
    // SAFETY: `block` holds `count` rows; `rows` has at least that many initialised.
    unsafe { core::ptr::copy_nonoverlapping(rows.as_ptr(), block, count) };
    p.rows = block;
    p.nrows = count as u32;
    Ok(())
}

/// The rows a program's last link resolved and the buffer mode (zero rows
/// before any link or after a failed one).
pub(super) fn rows(group: u8, program: GLuint) -> (&'static [TfRow], GLenum) {
    let p = prog(group, program);
    let mode = if p.mode == GL_SEPARATE_ATTRIBS { GL_SEPARATE_ATTRIBS } else { GL_INTERLEAVED_ATTRIBS };
    if p.rows.is_null() || p.nrows == 0 {
        return (&[], mode);
    }
    // SAFETY: `rows` is a live block of `nrows` rows written by `resolve_rows`.
    (unsafe { core::slice::from_raw_parts(p.rows, p.nrows as usize) }, mode)
}

/// glGetProgramiv's transform feedback pnames. None for any other pname.
pub(super) fn program_query(group: u8, program: GLuint, pname: GLenum) -> Option<GLint> {
    let (rows, mode) = rows(group, program);
    match pname {
        GL_TRANSFORM_FEEDBACK_BUFFER_MODE => Some(mode as GLint),
        GL_TRANSFORM_FEEDBACK_VARYINGS => Some(rows.len() as GLint),
        GL_TRANSFORM_FEEDBACK_VARYING_MAX_LENGTH => {
            Some(rows.iter().map(|r| r.name().len() + 1).max().unwrap_or(0) as GLint)
        }
        _ => None,
    }
}

/// glGetTransformFeedbackVarying.
///
/// # Safety
/// The out pointers are null or valid for their element counts (any alignment).
#[allow(clippy::too_many_arguments)]
pub(super) unsafe fn get_varying(
    c: &mut GlContext,
    program: GLuint,
    index: GLuint,
    buf_size: GLsizei,
    length: *mut GLsizei,
    size: *mut GLsizei,
    type_: *mut GLenum,
    name: *mut GLchar,
) {
    const FUNC: &str = "glGetTransformFeedbackVarying";
    if c.program_ref(program).is_none() {
        let e = super::program::not_a_program(c, program);
        c.set_error(e, FUNC);
        return;
    }
    if buf_size < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let (rows, _) = rows(c.group, program);
    let Some(r) = rows.get(index as usize) else {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    };
    util::app_put(size, r.size as GLsizei);
    util::app_put(type_, r.gl_type);
    let n = if buf_size == 0 { 0 } else { r.name().len().min(buf_size as usize - 1) };
    util::app_put(length, n as GLsizei);
    if !name.is_null() && buf_size != 0 {
        for (i, b) in r.name()[..n].iter().enumerate() {
            util::app_set(name, i, *b as GLchar);
        }
        util::app_set(name, n, 0);
    }
}

/* ------------------------------------------------------------ begin and end */

/// Bytes one vertex takes in binding `i` for the program's rows.
fn stride(rows: &[TfRow], mode: GLenum, i: usize) -> u32 {
    if mode == GL_SEPARATE_ATTRIBS {
        rows.get(i).map_or(0, |r| r.components() * 4)
    } else if i == 0 {
        rows.iter().map(|r| r.components() * 4).sum()
    } else {
        0
    }
}

/// glBeginTransformFeedback (ES 3.0 2.15.2). The primitive mode first (as
/// this library always did), then the object, program and bindings.
pub(super) fn begin(c: &mut GlContext, mode: GLenum) {
    const FUNC: &str = "glBeginTransformFeedback";
    if !matches!(mode, GL_POINTS | GL_LINES | GL_TRIANGLES) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if current(c).active {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let program = c.cur_program;
    if program == 0 || !c.program_ref(program).is_some_and(|p| p.linked) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let (rows, buffer_mode) = rows(c.group, program);
    if rows.is_empty() {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let needed = if buffer_mode == GL_SEPARATE_ATTRIBS { rows.len() } else { 1 };
    let o = current(c);
    if o.bind[..needed].iter().any(|b| b.buffer == 0) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    o.active = true;
    o.paused = false;
    o.prim_mode = mode;
    o.program = program;
    o.written = [0; BINDINGS];
}

/// glEndTransformFeedback.
pub(super) fn end(c: &mut GlContext) {
    let o = current(c);
    if !o.active {
        c.set_error(GL_INVALID_OPERATION, "glEndTransformFeedback");
        return;
    }
    o.active = false;
    o.paused = false;
}

/// glPauseTransformFeedback.
pub(super) fn pause(c: &mut GlContext) {
    let o = current(c);
    if !o.active || o.paused {
        c.set_error(GL_INVALID_OPERATION, "glPauseTransformFeedback");
        return;
    }
    o.paused = true;
}

/// glResumeTransformFeedback: the program must still be the one Begin saw.
pub(super) fn resume(c: &mut GlContext) {
    const FUNC: &str = "glResumeTransformFeedback";
    let o = current(c);
    if !o.active || !o.paused || c.cur_program != o.program {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    o.paused = false;
}

/* -------------------------------------------------------------------- draws */

/// Whether a draw mode feeds the primitive type Begin named (ES 3.0 2.15.2).
fn mode_matches(prim_mode: GLenum, draw: GLenum) -> bool {
    match prim_mode {
        GL_POINTS => draw == GL_POINTS,
        GL_LINES => matches!(draw, GL_LINES | GL_LINE_LOOP | GL_LINE_STRIP),
        _ => matches!(draw, GL_TRIANGLES | GL_TRIANGLE_STRIP | GL_TRIANGLE_FAN),
    }
}

/// Vertices a draw of `count` vertices in `mode` captures, per instance:
/// the primitives it assembles times the vertices of Begin's primitive type.
pub(super) fn captured_vertices(mode: GLenum, count: u32) -> u64 {
    let (prims, per) = match mode {
        GL_POINTS => (count, 1),
        GL_LINES => (count / 2, 2),
        GL_LINE_STRIP => (count.saturating_sub(1), 2),
        GL_LINE_LOOP => (if count >= 2 { count } else { 0 }, 2),
        GL_TRIANGLES => (count / 3, 3),
        _ => (count.saturating_sub(2), 3),
    };
    u64::from(prims) * per
}

/// The rules every draw meets while capture is on (ES 3.0 2.15.2, 2.8.3):
/// an indexed draw, a mode that does not match Begin's primitive, and a
/// capture that does not fit the buffers are GL_INVALID_OPERATION. Returns
/// whether the draw may go on. Capture itself is not built (tf-capture): a
/// draw that gets through is refused, loudly, with GL_INVALID_OPERATION.
pub(super) fn pre_draw(
    c: &mut GlContext,
    mode: GLenum,
    indexed: bool,
    count: GLsizei,
    instances: u32,
    func: &str,
) -> bool {
    if !capturing(c) {
        return true;
    }
    let o = *current(c);
    if indexed {
        c.set_error(GL_INVALID_OPERATION, func);
        return false;
    }
    if !mode_matches(o.prim_mode, mode) {
        c.set_error(GL_INVALID_OPERATION, func);
        return false;
    }
    let (rows, buffer_mode) = rows(c.group, o.program);
    let vertices = captured_vertices(mode, count.max(0) as u32) * u64::from(instances.max(1));
    let used = if buffer_mode == GL_SEPARATE_ATTRIBS { rows.len() } else { 1 };
    for i in 0..used {
        let b = o.bind[i];
        let avail = if b.size > 0 { b.size as i64 } else { buffer_size(c, b.buffer) - b.offset as i64 };
        let need = vertices as i64 * i64::from(stride(rows, buffer_mode, i)) + i64::from(o.written[i]);
        if need > avail {
            glog!(
                "{func}: transform feedback buffer {i} is too small for the draw ({need} bytes needed, {avail} bound)"
            );
            c.set_error(GL_INVALID_OPERATION, func);
            return false;
        }
    }
    glog!("{func}: transform feedback capture is not implemented yet (tf-capture); the draw is refused");
    c.set_error(GL_INVALID_OPERATION, func);
    false
}
