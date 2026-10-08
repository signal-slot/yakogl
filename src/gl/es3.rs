// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The OpenGL ES 3.0 entry points this library does not implement elsewhere
//! (`gl_es3.c`).
//!
//! Every function an application can link against has to exist. A conformance
//! run resolves all 246 ES 3.0 entry points up front and calls them without
//! asking whether they are there: dEQP's own state reset between test cases
//! walks queries, transform feedback, samplers and the indexed buffer
//! bindings, so one missing symbol is a jump through a null pointer before the
//! first case has finished, and nothing at all gets measured.
//!
//! What is here, therefore, is the API surface, and it is deliberately honest
//! about the difference between the two kinds of entry point:
//!
//!  - Calls that this library can answer correctly are answered here:
//!    glClearBuffer*, glGetUniformuiv, the program-binary calls (there are no
//!    binary formats, so refusing is the conformant answer), and object-name
//!    management for queries, samplers and transform feedback objects, which
//!    is pure bookkeeping and behaves exactly as the specification describes.
//!    (glCopyBufferSubData and glGetBufferPointerv live in [`super::buffer`],
//!    the 64-bit and indexed state queries in [`super::state`]: they need
//!    those modules' own helpers.)
//!
//!  - Calls that need a feature the backend does not model - occlusion
//!    counting, transform feedback capture, sampler objects overriding a
//!    texture's own parameters at draw time, uniform blocks - set
//!    GL_INVALID_OPERATION rather than quietly succeeding. A test that fails
//!    loudly is a measurement; one that passes because the driver returned a
//!    plausible zero is a lie, and it would hide the feature for as long as
//!    nobody read the source.
//!
//! The object pools live here rather than in `GlContext` because nothing
//! outside this module looks at them yet; they are per-context arrays keyed on
//! `GlContext::index`, the way [`super::state`] keeps its hints.
#![allow(non_snake_case)]

use super::consts::*;
use super::context::{self, gl_trace, GlContext, CAP_COMBINED_TEXTURE_UNITS, MAX_CONTEXTS};
use super::draw;
use super::framebuffer::{self, ColorKind};
use super::types::*;
use crate::backend;
use crate::util::{self, Global};

const MAX_QUERIES: usize = 32;
const MAX_SAMPLERS: usize = 32;

/// A name handed out by glGen* is not an object until its first bind (ES 3.0
/// 6.1.10), which is what glIs* reports; `used` is the name, `bound` the
/// object.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct Es3Name {
    used: bool,
    bound: bool,
}

impl Es3Name {
    const FREE: Es3Name = Es3Name { used: false, bound: false };
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct Es3Sampler {
    n: Es3Name,
    min_filter: GLenum,
    mag_filter: GLenum,
    wrap_s: GLenum,
    wrap_t: GLenum,
    wrap_r: GLenum,
    compare_mode: GLenum,
    compare_func: GLenum,
    min_lod: GLfloat,
    max_lod: GLfloat,
}

impl Es3Sampler {
    /// A slot as memset(0) leaves it (a free one).
    const FREE: Es3Sampler = Es3Sampler {
        n: Es3Name::FREE,
        min_filter: 0,
        mag_filter: 0,
        wrap_s: 0,
        wrap_t: 0,
        wrap_r: 0,
        compare_mode: 0,
        compare_func: 0,
        min_lod: 0.0,
        max_lod: 0.0,
    };

    /// A name glGenSamplers just handed out: the ES 3.0 sampler-object
    /// defaults, not yet bound.
    const NEW: Es3Sampler = Es3Sampler {
        n: Es3Name { used: true, bound: false },
        min_filter: GL_NEAREST_MIPMAP_LINEAR,
        mag_filter: GL_LINEAR,
        wrap_s: GL_REPEAT,
        wrap_t: GL_REPEAT,
        wrap_r: GL_REPEAT,
        compare_mode: GL_NONE,
        compare_func: GL_LEQUAL,
        min_lod: -1000.0,
        max_lod: 1000.0,
    };
}

static QUERIES: Global<[[Es3Name; MAX_QUERIES]; MAX_CONTEXTS]> = Global::zeroed();
static SAMPLERS: Global<[[Es3Sampler; MAX_SAMPLERS]; MAX_CONTEXTS]> = Global::zeroed();
static BOUND_SAMPLER: Global<[[GLuint; CAP_COMBINED_TEXTURE_UNITS]; MAX_CONTEXTS]> = Global::zeroed();

/// The query-name pool of context `index`.
fn queries(index: u8) -> &'static mut [Es3Name; MAX_QUERIES] {
    // SAFETY: single-threaded by contract; no other reference to the pool is live.
    unsafe { &mut QUERIES.get()[index as usize % MAX_CONTEXTS] }
}

/// The sampler-object pool of context `index`.
fn samplers(index: u8) -> &'static mut [Es3Sampler; MAX_SAMPLERS] {
    // SAFETY: single-threaded by contract; no other reference to the pool is live.
    unsafe { &mut SAMPLERS.get()[index as usize % MAX_CONTEXTS] }
}

/// The sampler bound to each texture unit of context `index`.
fn bound_sampler(index: u8) -> &'static mut [GLuint; CAP_COMBINED_TEXTURE_UNITS] {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { &mut BOUND_SAMPLER.get()[index as usize % MAX_CONTEXTS] }
}

/// The filters and wraps (min, mag, wrap S, wrap T) of the sampler object
/// bound to texture unit `unit` of context `index`, which a draw uses in
/// place of the texture's own; None when no sampler is bound.
pub(super) fn sampler_state(index: u8, unit: usize) -> Option<(GLenum, GLenum, GLenum, GLenum)> {
    let h = *bound_sampler(index).get(unit)? as usize;
    let s = samplers(index).get(h).filter(|s| h != 0 && s.n.used)?;
    Some((s.min_filter, s.mag_filter, s.wrap_s, s.wrap_t))
}

/// The compare mode and function (GL_TEXTURE_COMPARE_MODE, _FUNC) of the
/// sampler object bound to texture unit `unit` of context `index`, which a
/// draw uses in place of the texture's own; None when no sampler is bound.
pub(super) fn sampler_compare(index: u8, unit: usize) -> Option<(GLenum, GLenum)> {
    let h = *bound_sampler(index).get(unit)? as usize;
    let s = samplers(index).get(h).filter(|s| h != 0 && s.n.used)?;
    Some((s.compare_mode, s.compare_func))
}

/// GL_SAMPLER_BINDING of the active texture unit, for glGetIntegerv.
pub(super) fn sampler_binding(c: &GlContext) -> GLuint {
    bound_sampler(c.index).get(c.active_texture as usize).copied().unwrap_or(0)
}

/// GL_TRANSFORM_FEEDBACK_BINDING, for glGetIntegerv.
pub(super) fn feedback_binding(c: &GlContext) -> GLuint {
    super::tf::binding(c)
}

/// glGen*: fill `names` from the free slots of a pool, 0 where it is exhausted
/// (0 is never a valid generated name, so a caller that checks sees the
/// failure). Shared by every object kind here because the rule is the same.
///
/// # Safety
/// `names` must be null or valid for writing `n` GLuints (any alignment).
unsafe fn gen_names(c: &mut GlContext, pool: &mut [Es3Name], n: GLsizei, names: *mut GLuint, func: &str) {
    if names.is_null() {
        return;
    }
    if n < 0 {
        c.set_error(GL_INVALID_VALUE, func);
        return;
    }
    let n = n as usize;
    let mut made = 0usize;
    for (h, slot) in pool.iter_mut().enumerate().skip(1) {
        if made >= n {
            break;
        }
        if !slot.used {
            *slot = Es3Name { used: true, bound: false };
            util::app_set(names, made, h as GLuint);
            made += 1;
        }
    }
    for i in made..n {
        util::app_set(names, i, 0);
    }
}

/// glDelete*: free the named slots. 0 and out-of-range names are ignored
/// (deleting a name that is not one is not an error).
///
/// # Safety
/// `names` must be null or valid for reading `n` GLuints (any alignment).
unsafe fn delete_names(c: &mut GlContext, pool: &mut [Es3Name], n: GLsizei, names: *const GLuint, func: &str) {
    if names.is_null() {
        return;
    }
    if n < 0 {
        c.set_error(GL_INVALID_VALUE, func);
        return;
    }
    for i in 0..n as usize {
        let h = util::app_get(names, i) as usize;
        if h != 0 && h < pool.len() {
            pool[h] = Es3Name::FREE;
        }
    }
}

/// glIs*: a name is an object only once it has been bound (ES 3.0 6.1.10).
fn is_object(pool: &[Es3Name], name: GLuint) -> GLboolean {
    let h = name as usize;
    if h != 0 && h < pool.len() && pool[h].bound {
        GL_TRUE_B
    } else {
        GL_FALSE_B
    }
}

/* ---------------------------------------------------------------- queries --
 * Query names are real, and so is one target's counting:
 * GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN is counted on the CPU by the
 * transform feedback capture (`tf.rs`, which adds to the active query through
 * `add_primitives_written`). The BXS backend has no occlusion counter wired
 * up, so glBeginQuery on the ANY_SAMPLES_PASSED targets still refuses rather
 * than letting a caller believe a zero sample count (item occlusion-query
 * fills in that arm).
 */

/// What a query object has been used for (a name is a query object once it
/// was bound; its target is fixed by its first Begin).
#[derive(Clone, Copy)]
struct Es3Query {
    target: GLenum,
    active: bool,
    result: u32,
}

impl Es3Query {
    const NONE: Es3Query = Es3Query { target: 0, active: false, result: 0 };
}

static QSTATE: Global<[[Es3Query; MAX_QUERIES]; MAX_CONTEXTS]> = Global::zeroed();

fn qstate(index: u8) -> &'static mut [Es3Query; MAX_QUERIES] {
    // SAFETY: single-threaded by contract; no other reference to the pool is live.
    unsafe { &mut QSTATE.get()[index as usize % MAX_CONTEXTS] }
}

/// The query active on `target` of context `index`, if any.
fn active_query(index: u8, target: GLenum) -> Option<usize> {
    (1..MAX_QUERIES).find(|&h| qstate(index)[h].active && qstate(index)[h].target == target)
}

/// A captured draw wrote `n` primitives: the active
/// GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN query counts them.
#[allow(dead_code)] // called by the capture (item tf-capture)
pub(super) fn add_primitives_written(c: &GlContext, n: u32) {
    if let Some(h) = active_query(c.index, GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN) {
        let q = &mut qstate(c.index)[h];
        q.result = q.result.saturating_add(n);
    }
}

/// A target glBeginQuery / glEndQuery / glGetQueryiv accepts.
fn query_target(target: GLenum) -> bool {
    matches!(
        target,
        GL_ANY_SAMPLES_PASSED | GL_ANY_SAMPLES_PASSED_CONSERVATIVE | GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN
    )
}

#[no_mangle]
pub unsafe extern "C" fn glGenQueries(n: GLsizei, ids: *mut GLuint) {
    gl_trace!(glGenQueries);
    let Some(c) = context::current() else { return };
    let pool = queries(c.index);
    gen_names(c, pool, n, ids, "glGenQueries");
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteQueries(n: GLsizei, ids: *const GLuint) {
    gl_trace!(glDeleteQueries);
    let Some(c) = context::current() else { return };
    if !ids.is_null() && n > 0 {
        for i in 0..n as usize {
            // SAFETY: the application passes `n` names (n > 0 checked above).
            let h = util::app_get(ids, i) as usize;
            if h != 0 && h < MAX_QUERIES {
                qstate(c.index)[h] = Es3Query::NONE; /* an active query ends with its name */
            }
        }
    }
    let pool = queries(c.index);
    delete_names(c, pool, n, ids, "glDeleteQueries");
}

#[no_mangle]
pub unsafe extern "C" fn glIsQuery(id: GLuint) -> GLboolean {
    gl_trace!(glIsQuery);
    let Some(c) = context::current() else { return GL_FALSE_B };
    is_object(queries(c.index), id)
}

#[no_mangle]
pub unsafe extern "C" fn glBeginQuery(target: GLenum, id: GLuint) {
    gl_trace!(glBeginQuery);
    const FUNC: &str = "glBeginQuery";
    let Some(c) = context::current() else { return };
    if !query_target(target) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    let h = id as usize;
    if h == 0 || h >= MAX_QUERIES || !queries(c.index)[h].used {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    queries(c.index)[h].bound = true;
    if target != GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN || crate::REPORT_ES2 {
        c.set_error(GL_INVALID_OPERATION, FUNC); /* no counter behind it (see the section comment) */
        return;
    }
    /* ES 3.0 2.16: one active query per target; a query keeps the target of
     * its first Begin; one that is active cannot begin again */
    let q = qstate(c.index)[h];
    if active_query(c.index, target).is_some() || q.active || (q.target != 0 && q.target != target) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    qstate(c.index)[h] = Es3Query { target, active: true, result: 0 };
}

#[no_mangle]
pub unsafe extern "C" fn glEndQuery(target: GLenum) {
    gl_trace!(glEndQuery);
    const FUNC: &str = "glEndQuery";
    if !query_target(target) {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    let Some(c) = context::current() else { return };
    match active_query(c.index, target) {
        /* the capture is synchronous: the result is complete at once */
        Some(h) => qstate(c.index)[h].active = false,
        None => c.set_error(GL_INVALID_OPERATION, FUNC), /* no query is active on the target */
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetQueryiv(target: GLenum, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetQueryiv);
    const FUNC: &str = "glGetQueryiv";
    if params.is_null() {
        return;
    }
    if !query_target(target) {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    if pname != GL_CURRENT_QUERY {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    let id = context::current().and_then(|c| active_query(c.index, target)).unwrap_or(0);
    // SAFETY: params is non-null and valid for one GLint (GL contract), not
    // necessarily aligned.
    util::app_set(params, 0, id as GLint);
}

#[no_mangle]
pub unsafe extern "C" fn glGetQueryObjectuiv(id: GLuint, pname: GLenum, params: *mut GLuint) {
    gl_trace!(glGetQueryObjectuiv);
    const FUNC: &str = "glGetQueryObjectuiv";
    if pname != GL_QUERY_RESULT && pname != GL_QUERY_RESULT_AVAILABLE {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    let Some(c) = context::current() else { return };
    let h = id as usize;
    /* a query object that was counted, and is not active now */
    if h == 0 || h >= MAX_QUERIES || !queries(c.index)[h].bound || qstate(c.index)[h].active {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    let q = qstate(c.index)[h];
    if q.target == 0 {
        /* bound but never counted (a refused Begin): nothing to report */
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    // SAFETY: params is null or valid for one GLuint (GL contract), not necessarily aligned.
    util::app_put(params, if pname == GL_QUERY_RESULT { q.result } else { GL_TRUE });
}

/* ------------------------------------------------------ transform feedback --
 * The objects, varyings, binding points and begin/end rules are `tf.rs`; these
 * are its entry points. Capture itself is not built (item tf-capture): a draw
 * while a capture is active is refused loudly (GL_INVALID_OPERATION).
 */

#[no_mangle]
pub unsafe extern "C" fn glGenTransformFeedbacks(n: GLsizei, ids: *mut GLuint) {
    gl_trace!(glGenTransformFeedbacks);
    let Some(c) = context::current() else { return };
    super::tf::gen(c, n, ids);
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteTransformFeedbacks(n: GLsizei, ids: *const GLuint) {
    gl_trace!(glDeleteTransformFeedbacks);
    let Some(c) = context::current() else { return };
    super::tf::delete(c, n, ids);
}

#[no_mangle]
pub unsafe extern "C" fn glIsTransformFeedback(id: GLuint) -> GLboolean {
    gl_trace!(glIsTransformFeedback);
    let Some(c) = context::current() else { return GL_FALSE_B };
    if super::tf::is(c, id) {
        GL_TRUE_B
    } else {
        GL_FALSE_B
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBindTransformFeedback(target: GLenum, id: GLuint) {
    gl_trace!(glBindTransformFeedback);
    let Some(c) = context::current() else { return };
    super::tf::bind(c, target, id);
}

#[no_mangle]
pub unsafe extern "C" fn glBeginTransformFeedback(primitiveMode: GLenum) {
    gl_trace!(glBeginTransformFeedback);
    const FUNC: &str = "glBeginTransformFeedback";
    if crate::REPORT_ES2 {
        if !matches!(primitiveMode, GL_POINTS | GL_LINES | GL_TRIANGLES) {
            context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
            return;
        }
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC); /* nothing captures the vertex stage */
        return;
    }
    let Some(c) = context::current() else { return };
    super::tf::begin(c, primitiveMode);
}

#[no_mangle]
pub unsafe extern "C" fn glEndTransformFeedback() {
    gl_trace!(glEndTransformFeedback);
    match context::current() {
        Some(c) if !crate::REPORT_ES2 => super::tf::end(c),
        _ => context::set_error_no_ctx(GL_INVALID_OPERATION, "glEndTransformFeedback"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glPauseTransformFeedback() {
    gl_trace!(glPauseTransformFeedback);
    match context::current() {
        Some(c) if !crate::REPORT_ES2 => super::tf::pause(c),
        _ => context::set_error_no_ctx(GL_INVALID_OPERATION, "glPauseTransformFeedback"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glResumeTransformFeedback() {
    gl_trace!(glResumeTransformFeedback);
    match context::current() {
        Some(c) if !crate::REPORT_ES2 => super::tf::resume(c),
        _ => context::set_error_no_ctx(GL_INVALID_OPERATION, "glResumeTransformFeedback"),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glTransformFeedbackVaryings(
    program: GLuint,
    count: GLsizei,
    varyings: *const *const GLchar,
    bufferMode: GLenum,
) {
    gl_trace!(glTransformFeedbackVaryings);
    const FUNC: &str = "glTransformFeedbackVaryings";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    /* ES 3.0 2.11.8's errors, and no others (this call raised
     * GL_INVALID_OPERATION for any count > 0 until 2026-09-24, which no rule
     * allows; dEQP-GLES3 takes an error here as fatal and the lifetime
     * group's init killed its process). Under report-es2 every
     * MAX_TRANSFORM_FEEDBACK_* stays 0: SEPARATE_ATTRIBS with a varying is
     * GL_INVALID_VALUE and an INTERLEAVED request fails the next link
     * (`program::glLinkProgram`). */
    if c.program_ref(program).is_none() {
        let e = super::program::not_a_program(c, program);
        c.set_error(e, FUNC);
        return;
    }
    if count < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if bufferMode != GL_INTERLEAVED_ATTRIBS && bufferMode != GL_SEPARATE_ATTRIBS {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if crate::REPORT_ES2 {
        if bufferMode == GL_SEPARATE_ATTRIBS && count > 0 {
            c.set_error(GL_INVALID_VALUE, FUNC);
            return;
        }
        super::program::set_transform_feedback_varyings(c.group, program, count as u32);
        return;
    }
    if bufferMode == GL_SEPARATE_ATTRIBS && count > super::tf::TF_MAX_SEPARATE_ATTRIBS as GLsizei {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* the repo's NULL convention (as glDrawBuffers): names owed, none given */
    if count > 0 && varyings.is_null() {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if super::tf::is_active(c) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    super::tf::set_varyings(c, program, count, varyings, bufferMode);
}

#[no_mangle]
pub unsafe extern "C" fn glGetTransformFeedbackVarying(
    program: GLuint,
    index: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    size: *mut GLsizei,
    type_: *mut GLenum,
    name: *mut GLchar,
) {
    gl_trace!(glGetTransformFeedbackVarying);
    const FUNC: &str = "glGetTransformFeedbackVarying";
    if crate::REPORT_ES2 {
        /* no program has any recorded varyings */
        if bufSize < 0 {
            context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
            return;
        }
        // SAFETY: length is null or valid for one GLsizei, name null or valid for
        // bufSize GLchars (GL contract), neither necessarily aligned.
        util::app_put(length, 0);
        if !name.is_null() && bufSize != 0 {
            util::app_set(name, 0, 0);
        }
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    }
    let Some(c) = context::current() else { return };
    super::tf::get_varying(c, program, index, bufSize, length, size, type_, name);
}

/* --------------------------------------------------------------- samplers --
 * A sampler object's parameters are stored and read back exactly, and a draw
 * takes the bound sampler's filters and wraps in place of the texture's own
 * (`sampler_state`, draw.rs). Its compare mode and LOD range are stored
 * only. Until 2026-09-24 the draw ignored samplers and glBindSampler refused
 * a non-zero one with GL_INVALID_OPERATION, after recording it.
 */

#[no_mangle]
pub unsafe extern "C" fn glGenSamplers(count: GLsizei, samplers_out: *mut GLuint) {
    gl_trace!(glGenSamplers);
    const FUNC: &str = "glGenSamplers";
    let Some(c) = context::current() else { return };
    if samplers_out.is_null() {
        return;
    }
    if count < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    /* gen_names() cannot walk this pool: the slots are not bare names */
    let pool = samplers(c.index);
    let count = count as usize;
    let mut made = 0usize;
    for (h, slot) in pool.iter_mut().enumerate().skip(1) {
        if made >= count {
            break;
        }
        if slot.n.used {
            continue;
        }
        *slot = Es3Sampler::NEW;
        util::app_set(samplers_out, made, h as GLuint);
        made += 1;
    }
    for i in made..count {
        util::app_set(samplers_out, i, 0);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteSamplers(count: GLsizei, samplers_in: *const GLuint) {
    gl_trace!(glDeleteSamplers);
    const FUNC: &str = "glDeleteSamplers";
    let Some(c) = context::current() else { return };
    /* a negative count is the error whatever the pointer (dEQP passes null) */
    if count < 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    if samplers_in.is_null() {
        return;
    }
    let index = c.index;
    for i in 0..count as usize {
        // SAFETY: the application passes `count` names (count >= 0 above).
        let h = util::app_get(samplers_in, i);
        if h == 0 || h as usize >= MAX_SAMPLERS {
            continue;
        }
        for u in bound_sampler(index).iter_mut() {
            if *u == h {
                *u = 0;
            }
        }
        samplers(index)[h as usize] = Es3Sampler::FREE;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glIsSampler(sampler: GLuint) -> GLboolean {
    gl_trace!(glIsSampler);
    let Some(c) = context::current() else { return GL_FALSE_B };
    /* ES 3.0 3.8.2: glGenSamplers CREATES the objects, so a name it handed
     * out is a sampler before any bind (dEQP negative_api.state.is_sampler);
     * `bound` said no until 2026-09-24. */
    let h = sampler as usize;
    if h != 0 && h < MAX_SAMPLERS && samplers(c.index)[h].n.used {
        GL_TRUE_B
    } else {
        GL_FALSE_B
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBindSampler(unit: GLuint, sampler: GLuint) {
    gl_trace!(glBindSampler);
    const FUNC: &str = "glBindSampler";
    let Some(c) = context::current() else { return };
    if unit as usize >= CAP_COMBINED_TEXTURE_UNITS {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let h = sampler as usize;
    if h >= MAX_SAMPLERS || (h != 0 && !samplers(c.index)[h].n.used) {
        c.set_error(GL_INVALID_OPERATION, FUNC);
        return;
    }
    if h != 0 {
        samplers(c.index)[h].n.bound = true;
    }
    bound_sampler(c.index)[unit as usize] = sampler;
}

/// The pool slot `sampler` names, or `None` with GL_INVALID_OPERATION recorded
/// (a name glGenSamplers never handed out).
fn sampler_slot(c: &mut GlContext, sampler: GLuint, func: &str) -> Option<usize> {
    let h = sampler as usize;
    if h == 0 || h >= MAX_SAMPLERS || !samplers(c.index)[h].n.used {
        c.set_error(GL_INVALID_OPERATION, func);
        return None;
    }
    Some(h)
}

/// Store one sampler parameter; false for a pname a sampler object has none of.
fn sampler_set(s: &mut Es3Sampler, pname: GLenum, iv: GLint, fv: GLfloat, is_float: bool) -> bool {
    match pname {
        GL_TEXTURE_MIN_FILTER => s.min_filter = iv as GLenum,
        GL_TEXTURE_MAG_FILTER => s.mag_filter = iv as GLenum,
        GL_TEXTURE_WRAP_S => s.wrap_s = iv as GLenum,
        GL_TEXTURE_WRAP_T => s.wrap_t = iv as GLenum,
        GL_TEXTURE_WRAP_R => s.wrap_r = iv as GLenum,
        GL_TEXTURE_COMPARE_MODE => s.compare_mode = iv as GLenum,
        GL_TEXTURE_COMPARE_FUNC => s.compare_func = iv as GLenum,
        GL_TEXTURE_MIN_LOD => s.min_lod = if is_float { fv } else { iv as GLfloat },
        GL_TEXTURE_MAX_LOD => s.max_lod = if is_float { fv } else { iv as GLfloat },
        _ => return false,
    }
    true
}

/// Read one sampler parameter as a float; `None` for a pname a sampler object
/// has none of.
fn sampler_get(s: &Es3Sampler, pname: GLenum) -> Option<GLfloat> {
    Some(match pname {
        GL_TEXTURE_MIN_FILTER => s.min_filter as GLfloat,
        GL_TEXTURE_MAG_FILTER => s.mag_filter as GLfloat,
        GL_TEXTURE_WRAP_S => s.wrap_s as GLfloat,
        GL_TEXTURE_WRAP_T => s.wrap_t as GLfloat,
        GL_TEXTURE_WRAP_R => s.wrap_r as GLfloat,
        GL_TEXTURE_COMPARE_MODE => s.compare_mode as GLfloat,
        GL_TEXTURE_COMPARE_FUNC => s.compare_func as GLfloat,
        GL_TEXTURE_MIN_LOD => s.min_lod,
        GL_TEXTURE_MAX_LOD => s.max_lod,
        _ => return None,
    })
}

#[no_mangle]
pub unsafe extern "C" fn glSamplerParameteri(sampler: GLuint, pname: GLenum, param: GLint) {
    gl_trace!(glSamplerParameteri);
    const FUNC: &str = "glSamplerParameteri";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let Some(h) = sampler_slot(c, sampler, FUNC) else { return };
    /* a value the parameter does not take is GL_INVALID_ENUM, as for a
     * texture's (dEQP negative_api.shader.sampler_parameteri: WRAP_S of -1) */
    if !super::texture::valid_tex_param(pname, param) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if !sampler_set(&mut samplers(c.index)[h], pname, param, 0.0, false) {
        c.set_error(GL_INVALID_ENUM, FUNC);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glSamplerParameterf(sampler: GLuint, pname: GLenum, param: GLfloat) {
    gl_trace!(glSamplerParameterf);
    const FUNC: &str = "glSamplerParameterf";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let Some(h) = sampler_slot(c, sampler, FUNC) else { return };
    if !super::texture::valid_tex_param(pname, param as GLint) {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if !sampler_set(&mut samplers(c.index)[h], pname, param as GLint, param, true) {
        c.set_error(GL_INVALID_ENUM, FUNC);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glSamplerParameteriv(sampler: GLuint, pname: GLenum, param: *const GLint) {
    gl_trace!(glSamplerParameteriv);
    if !param.is_null() {
        // SAFETY: param points at one GLint (GL contract), not necessarily aligned.
        glSamplerParameteri(sampler, pname, util::app_get(param, 0));
    }
}

#[no_mangle]
pub unsafe extern "C" fn glSamplerParameterfv(sampler: GLuint, pname: GLenum, param: *const GLfloat) {
    gl_trace!(glSamplerParameterfv);
    if !param.is_null() {
        // SAFETY: param points at one GLfloat (GL contract), not necessarily aligned.
        glSamplerParameterf(sampler, pname, util::app_get(param, 0));
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetSamplerParameteriv(sampler: GLuint, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetSamplerParameteriv);
    const FUNC: &str = "glGetSamplerParameteriv";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let Some(h) = sampler_slot(c, sampler, FUNC) else { return };
    if params.is_null() {
        return;
    }
    match sampler_get(&samplers(c.index)[h], pname) {
        /* a float read as an integer rounds to the nearest (ES 3.0 6.1.2):
         * the LODs; truncated until 2026-09-25 (-74.5 read -74) */
        // SAFETY: params is non-null and valid for one GLint (GL contract).
        Some(v) => util::app_set(params, 0, (if v >= 0.0 { v + 0.5 } else { v - 0.5 }) as GLint),
        None => c.set_error(GL_INVALID_ENUM, FUNC),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetSamplerParameterfv(sampler: GLuint, pname: GLenum, params: *mut GLfloat) {
    gl_trace!(glGetSamplerParameterfv);
    const FUNC: &str = "glGetSamplerParameterfv";
    let Some(c) = context::current() else {
        context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
        return;
    };
    let Some(h) = sampler_slot(c, sampler, FUNC) else { return };
    if params.is_null() {
        return;
    }
    match sampler_get(&samplers(c.index)[h], pname) {
        // SAFETY: params is non-null and valid for one GLfloat (GL contract).
        Some(v) => util::app_set(params, 0, v),
        None => c.set_error(GL_INVALID_ENUM, FUNC),
    }
}

/* ----------------------------------------------------------- sync objects --
 * The same mechanism EGL's fence syncs use: a sync records the kick the
 * commands recorded so far go into, and is signalled once the backend reports
 * that kick complete.
 */

const MAX_SYNCS: usize = 8;

#[derive(Clone, Copy)]
struct Es3Sync {
    used: bool,
    kick: u32,
}

static SYNCS: Global<[Es3Sync; MAX_SYNCS]> = Global::zeroed();

fn syncs() -> &'static mut [Es3Sync; MAX_SYNCS] {
    // SAFETY: single-threaded by contract; no other reference is live.
    unsafe { SYNCS.get() }
}

/// The GLsync an application holds for slot `i`: the address of the slot, as
/// in the C (and as `egl`'s fences do it).
fn sync_handle(i: usize) -> GLsync {
    (&raw mut syncs()[i]) as GLsync
}

/// The slot a GLsync names, or `None` for 0, a stale handle, or a pointer this
/// library never handed out.
fn lookup_sync(sync: GLsync) -> Option<usize> {
    (0..MAX_SYNCS).find(|&i| syncs()[i].used && sync_handle(i) == sync)
}

/// gl_flush_pending_clear(): emit the clear-only kick of a glClear no draw
/// followed.
fn flush_pending_clear() {
    if let Some(c) = context::current() {
        draw::flush_pending_clear(c);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glFenceSync(condition: GLenum, flags: GLbitfield) -> GLsync {
    gl_trace!(glFenceSync);
    const FUNC: &str = "glFenceSync";
    if condition != GL_SYNC_GPU_COMMANDS_COMPLETE {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return core::ptr::null_mut();
    }
    if flags != 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return core::ptr::null_mut();
    }
    let Some(i) = (0..MAX_SYNCS).find(|&i| !syncs()[i].used) else {
        context::set_error_no_ctx(GL_OUT_OF_MEMORY, FUNC);
        return core::ptr::null_mut();
    };
    /* Everything recorded so far goes into the kick being built, so submit it
     * and wait on the last kick actually submitted; a flush with nothing
     * recorded submits none, and the kick "being recorded" never completes. */
    flush_pending_clear();
    if backend::flush() != 0 {
        context::set_error_no_ctx(GL_OUT_OF_MEMORY, FUNC);
        return core::ptr::null_mut();
    }
    syncs()[i].kick = backend::kick_id().wrapping_sub(1);
    syncs()[i].used = true;
    sync_handle(i)
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteSync(sync: GLsync) {
    gl_trace!(glDeleteSync);
    let slot = lookup_sync(sync);
    if sync.is_null() {
        return; /* ES 3.0: deleting 0 is silently ignored */
    }
    let Some(i) = slot else {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glDeleteSync");
        return;
    };
    syncs()[i].used = false;
}

#[no_mangle]
pub unsafe extern "C" fn glIsSync(sync: GLsync) -> GLboolean {
    gl_trace!(glIsSync);
    if lookup_sync(sync).is_some() {
        GL_TRUE_B
    } else {
        GL_FALSE_B
    }
}

#[no_mangle]
pub unsafe extern "C" fn glClientWaitSync(sync: GLsync, flags: GLbitfield, timeout: GLuint64) -> GLenum {
    gl_trace!(glClientWaitSync);
    const FUNC: &str = "glClientWaitSync";
    let Some(i) = lookup_sync(sync) else {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return GL_WAIT_FAILED;
    };
    if flags & !GL_SYNC_FLUSH_COMMANDS_BIT != 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return GL_WAIT_FAILED;
    }
    if !backend::kick_pending(syncs()[i].kick) {
        return GL_ALREADY_SIGNALED;
    }
    if timeout == 0 {
        return GL_TIMEOUT_EXPIRED;
    }
    /* ES 3.0 5.2.1: the wait flushes only with SYNC_FLUSH_COMMANDS_BIT, and
     * the fence was flushed at glFenceSync. So wait for this sync's own
     * kick and submit nothing recorded since; a backend without that wait
     * finishes (a superset: correct, never early, but it submits the open
     * frame too). The timeout is honoured only at its two ends. */
    let rc = backend::wait_kick(syncs()[i].kick).unwrap_or_else(backend::finish);
    if rc != 0 {
        return GL_WAIT_FAILED;
    }
    GL_CONDITION_SATISFIED
}

#[no_mangle]
pub unsafe extern "C" fn glWaitSync(sync: GLsync, flags: GLbitfield, timeout: GLuint64) {
    gl_trace!(glWaitSync);
    const FUNC: &str = "glWaitSync";
    if lookup_sync(sync).is_none() {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    }
    if flags != 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    }
    if timeout != GL_TIMEOUT_IGNORED {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
    }
    /* Otherwise a server-side wait, and there is one command queue: the
     * ordering it asks for already holds. */
}

#[no_mangle]
pub unsafe extern "C" fn glGetSynciv(
    sync: GLsync,
    pname: GLenum,
    count: GLsizei,
    length: *mut GLsizei,
    values: *mut GLint,
) {
    gl_trace!(glGetSynciv);
    const FUNC: &str = "glGetSynciv";
    let Some(i) = lookup_sync(sync) else {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    };
    if count < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    }
    let v: GLint = match pname {
        GL_OBJECT_TYPE => GL_SYNC_FENCE as GLint,
        GL_SYNC_STATUS => {
            if backend::kick_pending(syncs()[i].kick) {
                GL_UNSIGNALED as GLint
            } else {
                GL_SIGNALED as GLint
            }
        }
        GL_SYNC_CONDITION => GL_SYNC_GPU_COMMANDS_COMPLETE as GLint,
        GL_SYNC_FLAGS => 0,
        _ => {
            context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
            return;
        }
    };
    // SAFETY: values is null or valid for `count` GLints, length null or valid
    // for one GLsizei (GL contract); neither is necessarily aligned.
    if count > 0 && !values.is_null() {
        util::app_set(values, 0, v);
        util::app_put(length, 1);
    } else {
        util::app_put(length, 0);
    }
}

/* --------------------------------------------------------- uniform blocks --
 * The block queries and glUniformBlockBinding are in `program`, next to
 * the location tables they extend: a program's blocks are its stages'
 * (`shaderc::uniform_blocks`), matched at link time.
 */

/// gl_lookup_program(): true when `program` names a program object of the
/// current context. Records GL_INVALID_OPERATION and answers false otherwise,
/// which is the first thing the program queries below do.
fn program_exists(program: GLuint, func: &str) -> bool {
    let found = context::current().is_some_and(|c| c.program_ref(program).is_some());
    if !found {
        context::set_error_no_ctx(GL_INVALID_OPERATION, func);
    }
    found
}

/* ------------------------------------------------------- buffers, indexed --
 * The uniform-buffer points are recorded in the context (`uniform_buffers`)
 * and read at draw time; transform feedback is not modelled, so a real
 * buffer is refused there. glCopyBufferSubData is an ordinary copy and is
 * done in `buffer`.
 */

fn indexed_buffer_target(target: GLenum) -> bool {
    target == GL_UNIFORM_BUFFER || target == GL_TRANSFORM_FEEDBACK_BUFFER
}

#[no_mangle]
pub unsafe extern "C" fn glBindBufferBase(target: GLenum, index: GLuint, buffer: GLuint) {
    gl_trace!(glBindBufferBase);
    const FUNC: &str = "glBindBufferBase";
    if !indexed_buffer_target(target) {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    if target == GL_UNIFORM_BUFFER {
        bind_uniform_buffer(index, buffer, 0, 0, FUNC);
        return;
    }
    bind_transform_feedback_buffer(index, buffer, 0, 0, false, FUNC);
}

/// glBindBufferBase / glBindBufferRange for GL_TRANSFORM_FEEDBACK_BUFFER:
/// the bound transform feedback object's indexed point (`tf.rs`). Under
/// report-es2 binding zero is an unbind that succeeds and a real buffer
/// cannot be honoured.
fn bind_transform_feedback_buffer(index: GLuint, buffer: GLuint, offset: GLintptr, size: GLsizeiptr, ranged: bool, func: &str) {
    if crate::REPORT_ES2 {
        if buffer != 0 {
            context::set_error_no_ctx(GL_INVALID_OPERATION, func);
        }
        return;
    }
    let Some(c) = context::current() else { return };
    super::tf::bind_buffer(c, index, buffer, offset, size, ranged, func);
}

/// glBindBufferBase / glBindBufferRange for GL_UNIFORM_BUFFER (ES 3.0
/// 2.10.1.1): the indexed point records the buffer and range, and the
/// generic GL_UNIFORM_BUFFER binding takes the buffer too (through
/// glBindBuffer, which creates a generated name and refuses an unknown one).
/// `size` 0 is the whole buffer.
fn bind_uniform_buffer(index: GLuint, buffer: GLuint, offset: GLintptr, size: GLsizeiptr, func: &str) {
    if index as usize >= context::MAX_UNIFORM_BUFFER_BINDINGS {
        context::set_error_no_ctx(GL_INVALID_VALUE, func);
        return;
    }
    // SAFETY: a plain GL call; it records its own errors.
    unsafe { super::buffer::glBindBuffer(GL_UNIFORM_BUFFER, buffer) };
    let Some(c) = context::current() else { return };
    /* the binding holds the name's slot (gl/names.rs) */
    let buffer = match (buffer, super::names::slot_of(c, super::names::Kind::Buffer, buffer)) {
        (0, _) => 0,
        (_, Some(s)) if c.bound_uniform_buffer == s => s,
        /* glBindBuffer refused the name */
        _ => return,
    };
    context::uniform_buffers(c)[index as usize] = context::UboBinding { buffer, offset, size };
}

#[no_mangle]
pub unsafe extern "C" fn glBindBufferRange(
    target: GLenum,
    index: GLuint,
    buffer: GLuint,
    offset: GLintptr,
    size: GLsizeiptr,
) {
    gl_trace!(glBindBufferRange);
    const FUNC: &str = "glBindBufferRange";
    if !indexed_buffer_target(target) {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    if offset < 0 || size <= 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    }
    if target == GL_UNIFORM_BUFFER {
        /* GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT is 4 (state.rs) */
        if offset % 4 != 0 {
            context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
            return;
        }
        bind_uniform_buffer(index, buffer, offset, size, FUNC);
        return;
    }
    bind_transform_feedback_buffer(index, buffer, offset, size, true, FUNC);
}

#[no_mangle]
pub unsafe extern "C" fn glGetBufferParameteri64v(target: GLenum, pname: GLenum, params: *mut GLint64) {
    gl_trace!(glGetBufferParameteri64v);
    let mut v: GLint = 0;
    // SAFETY: &mut v is one valid GLint; params is null or valid for one
    // GLint64 (GL contract), not necessarily aligned.
    super::buffer::glGetBufferParameteriv(target, pname, &mut v);
    util::app_put(params, v as GLint64);
}

/* ---------------------------------------------------- glClearBuffer* (4.2.3)
 * The same pending-clear the ES 2.0 path records, addressed per buffer. The
 * clear values are context state, so they are put back afterwards: unlike
 * glClear, these calls do not change them.
 */

/// glClearBuffer{f,i,ui}v's argument rules (ES 3.0 4.2.3), in the order
/// dEQP checks them: a buffer the command does not take is GL_INVALID_ENUM
/// before anything is said about the draw buffer; then GL_COLOR takes a
/// draw buffer below GL_MAX_DRAW_BUFFERS and the others take 0 only
/// (GL_INVALID_VALUE). The draw-buffer test came first until 2026-09-24, so
/// glClearBufferfv(GL_STENCIL, 1, ..) was GL_INVALID_VALUE.
fn clear_buffer_args_ok(c: &mut GlContext, buffer: GLenum, drawbuffer: GLint, takes: &[GLenum], func: &str) -> bool {
    if !takes.contains(&buffer) {
        c.set_error(GL_INVALID_ENUM, func);
        return false;
    }
    let ok = if buffer == GL_COLOR {
        (0..context::MAX_DRAW_BUFFERS as GLint).contains(&drawbuffer)
    } else {
        drawbuffer == 0
    };
    if !ok {
        c.set_error(GL_INVALID_VALUE, func);
        return false;
    }
    true
}

/// The kind of colour image draw buffer `drawbuffer` of the bound framebuffer
/// writes; `None` when it writes none (GL_NONE, no attachment, no storage),
/// where a clear is no error and clears nothing.
fn clear_image_kind(c: &GlContext, drawbuffer: GLint) -> Option<ColorKind> {
    framebuffer::draw_buffer_image_format(c, drawbuffer as usize).map(framebuffer::color_kind)
}

/// glClearBufferiv / glClearBufferuiv on GL_COLOR (ES 3.0 4.2.3): the draw
/// buffer must hold integers of that signedness, else GL_INVALID_OPERATION.
/// The four values travel as raw bits in the pending clear's colour
/// (`GlesDraw::clear_rgba` of an integer target), and the clear colour of the
/// context is put back.
fn clear_color_integer(c: &mut GlContext, drawbuffer: GLint, want: ColorKind, bits: [u32; 4], func: &str) {
    match clear_image_kind(c, drawbuffer) {
        Some(k) if k == want => {}
        Some(_) => {
            c.set_error(GL_INVALID_OPERATION, func);
            return;
        }
        None => return,
    }
    let save = c.clear_color;
    c.clear_color = bits.map(f32::from_bits);
    // SAFETY: a plain GL call, on the current context.
    super::draw::with_clear_target(drawbuffer as u32, || unsafe { super::state::glClear(GL_COLOR_BUFFER_BIT) });
    if let Some(c) = context::current() {
        c.clear_color = save;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glClearBufferfv(buffer: GLenum, drawbuffer: GLint, value: *const GLfloat) {
    gl_trace!(glClearBufferfv);
    const FUNC: &str = "glClearBufferfv";
    let Some(c) = context::current() else { return };
    if value.is_null() {
        return;
    }
    if !clear_buffer_args_ok(c, buffer, drawbuffer, &[GL_COLOR, GL_DEPTH], FUNC) {
        return;
    }
    /* GL_COLOR clears draw buffer `drawbuffer` alone: the draw record says
     * which (`GlesDraw::clear_target_mask`). Until 2026-09-24 a draw buffer
     * above 0 was dropped here and drawbuffer 0 cleared every target. */
    /* The state to put back is read out of the context before the nested entry
     * points take the context themselves. */
    match buffer {
        GL_COLOR => {
            /* ES 3.0 4.2.3: the float form clears a fixed or float buffer */
            if clear_image_kind(c, drawbuffer).is_some_and(|k| k != ColorKind::Float) {
                c.set_error(GL_INVALID_OPERATION, FUNC);
                return;
            }
            let save = c.clear_color;
            // SAFETY: value points at four GLfloats (GL contract), unaligned.
            let v =
                [util::app_get(value, 0), util::app_get(value, 1), util::app_get(value, 2), util::app_get(value, 3)];
            super::state::glClearColor(v[0], v[1], v[2], v[3]);
            super::draw::with_clear_target(drawbuffer as u32, || super::state::glClear(GL_COLOR_BUFFER_BIT));
            super::state::glClearColor(save[0], save[1], save[2], save[3]);
        }
        GL_DEPTH => {
            let save = c.clear_depth;
            // SAFETY: value points at one GLfloat (GL contract), unaligned.
            super::state::glClearDepthf(util::app_get(value, 0));
            super::state::glClear(GL_DEPTH_BUFFER_BIT);
            super::state::glClearDepthf(save);
        }
        _ => c.set_error(GL_INVALID_ENUM, FUNC),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glClearBufferiv(buffer: GLenum, drawbuffer: GLint, value: *const GLint) {
    gl_trace!(glClearBufferiv);
    const FUNC: &str = "glClearBufferiv";
    let Some(c) = context::current() else { return };
    if value.is_null() {
        return;
    }
    if !clear_buffer_args_ok(c, buffer, drawbuffer, &[GL_COLOR, GL_STENCIL], FUNC) {
        return;
    }
    match buffer {
        GL_STENCIL => {
            let save = c.clear_stencil;
            // SAFETY: value points at one GLint (GL contract), unaligned.
            super::state::glClearStencil(util::app_get(value, 0));
            super::state::glClear(GL_STENCIL_BUFFER_BIT);
            super::state::glClearStencil(save);
        }
        GL_COLOR => {
            // SAFETY: value points at four GLints (GL contract), unaligned.
            let v = [0, 1, 2, 3].map(|i| util::app_get(value, i) as u32);
            clear_color_integer(c, drawbuffer, ColorKind::Int, v, FUNC);
        }
        _ => c.set_error(GL_INVALID_ENUM, FUNC),
    }
}

#[no_mangle]
pub unsafe extern "C" fn glClearBufferuiv(buffer: GLenum, drawbuffer: GLint, value: *const GLuint) {
    gl_trace!(glClearBufferuiv);
    let Some(c) = context::current() else { return };
    if value.is_null() {
        return;
    }
    if !clear_buffer_args_ok(c, buffer, drawbuffer, &[GL_COLOR], "glClearBufferuiv") {
        return;
    }
    // SAFETY: value points at four GLuints (GL contract), unaligned.
    let v = [0, 1, 2, 3].map(|i| util::app_get(value, i));
    clear_color_integer(c, drawbuffer, ColorKind::Uint, v, "glClearBufferuiv");
}

#[no_mangle]
pub unsafe extern "C" fn glClearBufferfi(buffer: GLenum, drawbuffer: GLint, depth: GLfloat, stencil: GLint) {
    gl_trace!(glClearBufferfi);
    const FUNC: &str = "glClearBufferfi";
    let Some(c) = context::current() else { return };
    if buffer != GL_DEPTH_STENCIL {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return;
    }
    if drawbuffer != 0 {
        c.set_error(GL_INVALID_VALUE, FUNC);
        return;
    }
    let (sd, ss) = (c.clear_depth, c.clear_stencil);
    super::state::glClearDepthf(depth);
    super::state::glClearStencil(stencil);
    super::state::glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
    super::state::glClearDepthf(sd);
    super::state::glClearStencil(ss);
}

/* The 64-bit and indexed state queries live in `state`: they need its
 * per-pname value count, and writing one GLint64 for a pname that yields four
 * smashed the caller's stack (GL_COLOR_WRITEMASK). */

/* ------------------------------------------------------------ programs ---- */

#[no_mangle]
pub unsafe extern "C" fn glGetUniformuiv(program: GLuint, location: GLint, params: *mut GLuint) {
    gl_trace!(glGetUniformuiv);
    // SAFETY: params is the application's array, forwarded unchanged; the
    // signed and unsigned forms have the same storage.
    super::program::glGetUniformiv(program, location, params as *mut GLint);
}

#[no_mangle]
pub unsafe extern "C" fn glGetFragDataLocation(program: GLuint, _name: *const GLchar) -> GLint {
    gl_trace!(glGetFragDataLocation);
    if !program_exists(program, "glGetFragDataLocation") {
        return -1;
    }
    /* ES 3.0 2.12.5: only a linked program has locations to ask about */
    let linked = context::current().is_some_and(|c| c.program_ref(program).is_some_and(|p| p.linked));
    if !linked {
        context::set_error_no_ctx(GL_INVALID_OPERATION, "glGetFragDataLocation");
        return -1;
    }
    /* One colour output, always at location 0 in the shaders this table holds;
     * anything else is not a fragment output of this program. */
    0
}

#[no_mangle]
pub unsafe extern "C" fn glProgramParameteri(program: GLuint, pname: GLenum, value: GLint) {
    gl_trace!(glProgramParameteri);
    const FUNC: &str = "glProgramParameteri";
    if !program_exists(program, FUNC) {
        return;
    }
    if pname != GL_PROGRAM_BINARY_RETRIEVABLE_HINT {
        context::set_error_no_ctx(GL_INVALID_ENUM, FUNC);
        return;
    }
    if value != GL_TRUE as GLint && value != GL_FALSE as GLint {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
    }
    /* Otherwise accepted and ignored: there are no binary formats to retrieve into. */
}

#[no_mangle]
pub unsafe extern "C" fn glGetProgramBinary(
    program: GLuint,
    bufSize: GLsizei,
    length: *mut GLsizei,
    _binaryFormat: *mut GLenum,
    _binary: *mut GLvoid,
) {
    gl_trace!(glGetProgramBinary);
    const FUNC: &str = "glGetProgramBinary";
    if !program_exists(program, FUNC) {
        return;
    }
    /* GL_NUM_PROGRAM_BINARY_FORMATS is zero: there is no binary to hand out
     * whatever the buffer, so this is the answer for every bufSize (dEQP
     * negative_api.shader.get_program_binary asks with 0, length - 1 and
     * on an unlinked program, and expects it each time). */
    let _ = bufSize;
    // SAFETY: length is null or valid for one GLsizei (GL contract).
    util::app_put(length, 0);
    context::set_error_no_ctx(GL_INVALID_OPERATION, FUNC);
}

#[no_mangle]
pub unsafe extern "C" fn glProgramBinary(
    program: GLuint,
    _binaryFormat: GLenum,
    _binary: *const GLvoid,
    length: GLsizei,
) {
    gl_trace!(glProgramBinary);
    const FUNC: &str = "glProgramBinary";
    if !program_exists(program, FUNC) {
        return;
    }
    if length < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, FUNC);
        return;
    }
    context::set_error_no_ctx(GL_INVALID_ENUM, FUNC); /* no format is accepted */
}

/// Unit tests: empty the object pools and the sync table (process-global, like
/// the C's statics). `testutil::TestGl` does not know about this module, so the
/// tests below call this themselves.
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised.
    unsafe {
        *QUERIES.get() = [[Es3Name::FREE; MAX_QUERIES]; MAX_CONTEXTS];
        *QSTATE.get() = [[Es3Query::NONE; MAX_QUERIES]; MAX_CONTEXTS];
        *SAMPLERS.get() = [[Es3Sampler::FREE; MAX_SAMPLERS]; MAX_CONTEXTS];
        *BOUND_SAMPLER.get() = [[0; CAP_COMBINED_TEXTURE_UNITS]; MAX_CONTEXTS];
        super::tf::reset_for_tests();
        *SYNCS.get() = [Es3Sync { used: false, kick: 0 }; MAX_SYNCS];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gl::program::glCreateProgram;
    use crate::gl::program::{
        glGetActiveUniformBlockName, glGetActiveUniformBlockiv, glGetActiveUniformsiv, glGetUniformBlockIndex,
        glGetUniformIndices, glUniformBlockBinding,
    };
    use crate::gl::state::{glClearColor, glClearDepthf, glClearStencil, glGetError};
    use crate::testutil::TestGl;

    /// A fresh context with this module's pools empty.
    fn fresh() -> TestGl {
        let gl = TestGl::new(8, 8);
        reset_for_tests();
        gl
    }

    fn gen1(f: unsafe extern "C" fn(GLsizei, *mut GLuint)) -> GLuint {
        let mut h = 0;
        // SAFETY: room for one name.
        unsafe { f(1, &mut h) };
        assert_ne!(h, 0);
        h
    }

    fn gen_query() -> GLuint {
        gen1(glGenQueries)
    }

    fn gen_feedback() -> GLuint {
        gen1(glGenTransformFeedbacks)
    }

    fn gen_sampler() -> GLuint {
        gen1(glGenSamplers)
    }

    /* ---- queries ---- */

    #[test]
    fn query_names_are_distinct_and_run_out_as_zeroes() {
        let gl = fresh();
        let mut names = [7u32; MAX_QUERIES + 1];
        // SAFETY: room for every name asked for.
        unsafe { glGenQueries(names.len() as GLsizei, names.as_mut_ptr()) };
        /* 0 is the reserved name: MAX_QUERIES - 1 names exist */
        assert!(names[..MAX_QUERIES - 1].iter().all(|&n| n != 0));
        for (i, n) in names[..MAX_QUERIES - 1].iter().enumerate() {
            assert!(!names[..i].contains(n));
        }
        assert_eq!(names[MAX_QUERIES - 1..], [0, 0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn query_gen_and_delete_reject_a_negative_count_and_tolerate_null() {
        let gl = fresh();
        let mut n = 55;
        // SAFETY: nothing is written for n < 0; null is tolerated.
        unsafe {
            glGenQueries(-1, &mut n);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDeleteQueries(-1, &n);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGenQueries(2, core::ptr::null_mut());
            glDeleteQueries(2, core::ptr::null());
        }
        assert_eq!(n, 55);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn a_query_name_is_an_object_only_after_begin_query() {
        let gl = fresh();
        let q = gen_query();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsQuery(q), GL_FALSE_B, "a generated name is not yet an object");
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q);
            /* the name became an object, and the counting is refused */
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert_eq!(glIsQuery(q), GL_TRUE_B);
            glDeleteQueries(1, &q);
            assert_eq!(glIsQuery(q), GL_FALSE_B);
            assert_eq!(glIsQuery(0), GL_FALSE_B);
            assert_eq!(glIsQuery(GLuint::MAX), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn begin_query_checks_the_target_before_the_name() {
        let gl = fresh();
        let q = gen_query();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginQuery(GL_TRIANGLES, q);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert_eq!(glIsQuery(q), GL_FALSE_B, "a rejected target does not create the object");
            glBeginQuery(GL_ANY_SAMPLES_PASSED, q + 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not a generated name");
            glBeginQuery(GL_ANY_SAMPLES_PASSED_CONSERVATIVE, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBeginQuery(GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN, MAX_QUERIES as GLuint);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn end_query_and_the_object_query_refuse() {
        let gl = fresh();
        // SAFETY: plain GL calls; the output pointer is null.
        unsafe {
            glEndQuery(GL_POINTS);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glEndQuery(GL_ANY_SAMPLES_PASSED);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no query was ever active");
            glGetQueryObjectuiv(1, GL_CURRENT_QUERY, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetQueryObjectuiv(1, GL_QUERY_RESULT, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glGetQueryObjectuiv(1, GL_QUERY_RESULT_AVAILABLE, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn get_queryiv_answers_that_no_query_is_active() {
        let gl = fresh();
        let mut v = 9;
        // SAFETY: room for one value; null is tolerated.
        unsafe {
            glGetQueryiv(GL_ANY_SAMPLES_PASSED, GL_CURRENT_QUERY, &mut v);
            assert_eq!(v, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glGetQueryiv(GL_ANY_SAMPLES_PASSED, GL_CURRENT_QUERY, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
            v = 9;
            glGetQueryiv(GL_TRIANGLES, GL_CURRENT_QUERY, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetQueryiv(GL_ANY_SAMPLES_PASSED, GL_QUERY_RESULT, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert_eq!(v, 9, "a rejected query writes nothing");
        }
    }

    /* ---- transform feedback ---- */

    #[test]
    fn a_transform_feedback_name_is_an_object_only_after_bind() {
        let gl = fresh();
        let f = gen_feedback();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsTransformFeedback(f), GL_FALSE_B);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, f);
            assert_eq!(gl.error(), GL_NO_ERROR, "the binding itself is bookkeeping and succeeds");
            assert_eq!(glIsTransformFeedback(f), GL_TRUE_B);
            assert_eq!(glIsTransformFeedback(0), GL_FALSE_B);
        }
        assert_eq!(feedback_binding(gl.ctx()), f);
    }

    #[test]
    fn bind_transform_feedback_checks_the_target_and_the_name() {
        let gl = fresh();
        let f = gen_feedback();
        // SAFETY: plain GL calls.
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK_BUFFER, f);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, super::super::tf::MAX_FEEDBACKS as GLuint);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, f + 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not a generated name");
            assert_eq!(feedback_binding(gl.ctx()), 0, "a rejected bind changes nothing");
            /* 0 is always bindable: it is the default object */
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn deleting_the_bound_transform_feedback_reverts_the_binding() {
        let gl = fresh();
        let (keep, victim) = (gen_feedback(), gen_feedback());
        // SAFETY: plain GL calls.
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, keep);
            glDeleteTransformFeedbacks(1, &victim);
            assert_eq!(feedback_binding(gl.ctx()), keep, "an unbound name does not touch the binding");
            glDeleteTransformFeedbacks(1, &keep);
            assert_eq!(feedback_binding(gl.ctx()), 0);
            assert_eq!(glIsTransformFeedback(keep), GL_FALSE_B);
            glDeleteTransformFeedbacks(-1, &keep);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn transform_feedback_capture_is_refused() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            glBeginTransformFeedback(GL_TRIANGLE_FAN);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            for mode in [GL_POINTS, GL_LINES, GL_TRIANGLES] {
                glBeginTransformFeedback(mode);
                assert_eq!(gl.error(), GL_INVALID_OPERATION);
            }
            glEndTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glPauseTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glResumeTransformFeedback();
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    /// A sampler exists from glGenSamplers on (ES 3.0 3.8.2; dEQP-GLES3
    /// negative_api.state.is_sampler), a value its parameter does not take
    /// is GL_INVALID_ENUM (negative_api.shader.sampler_parameter*), and a
    /// negative delete count is GL_INVALID_VALUE before the pointer is
    /// looked at (negative_api.shader.delete_samplers). Each was otherwise
    /// until 2026-09-24.
    #[test]
    fn samplers_exist_from_gen_and_refuse_bad_values() {
        let gl = fresh();
        let mut s: GLuint = 0;
        // SAFETY: plain GL calls; `s` is one name.
        unsafe {
            glGenSamplers(1, &mut s);
            assert_eq!(glIsSampler(s), GL_TRUE_B, "a sampler before any bind");
            glSamplerParameteri(s, GL_TEXTURE_WRAP_S, -1);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glSamplerParameterf(s, GL_TEXTURE_WRAP_S, -1.0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glSamplerParameteri(s, GL_TEXTURE_COMPARE_MODE, 7);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glSamplerParameteri(s, GL_TEXTURE_WRAP_R, GL_MIRRORED_REPEAT as GLint);
            glSamplerParameterf(s, GL_TEXTURE_MIN_LOD, -3.5);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteSamplers(-1, core::ptr::null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDeleteSamplers(1, &s);
            assert_eq!(glIsSampler(s), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /// glClearBuffer{f,i,ui}v: a buffer the command does not take is
    /// GL_INVALID_ENUM whatever the draw buffer, then the draw buffer's
    /// range (ES 3.0 4.2.3; dEQP-GLES3 negative_api.buffer.clear_buffer*).
    /// The draw buffer was tested first until 2026-09-24.
    #[test]
    fn clear_buffer_checks_the_buffer_before_the_draw_buffer() {
        let gl = fresh();
        let (f, i, u) = ([0.0f32; 4], [0i32; 4], [0u32; 4]);
        let max = context::MAX_DRAW_BUFFERS as GLint;
        // SAFETY: plain GL calls; each value array has four entries.
        unsafe {
            glClearBufferfv(GL_STENCIL, 1, f.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glClearBufferfv(GL_DEPTH_STENCIL, 1, f.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glClearBufferfv(GL_DEPTH, 1, f.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glClearBufferfv(GL_COLOR, max, f.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glClearBufferiv(GL_DEPTH, 1, i.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glClearBufferiv(GL_STENCIL, 1, i.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glClearBufferuiv(GL_DEPTH, 1, u.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glClearBufferuiv(GL_COLOR, max, u.as_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /// No program binary format exists: GL_PROGRAM_BINARY_LENGTH answers 0
    /// (dEQP's program_binary TCU_CHECKs it), glGetProgramBinary is
    /// GL_INVALID_OPERATION for any buffer, and glGetFragDataLocation needs
    /// a linked program (ES 3.0 2.12.5; dEQP negative_api.state.
    /// get_frag_data_location).
    #[test]
    fn program_binary_length_is_zero_and_frag_data_needs_a_link() {
        let gl = fresh();
        let p = gl.program("passthrough.vert", "uniform_color.frag");
        let (mut len, mut fmt, mut n): (GLsizei, GLenum, GLint) = (7, 7, 7);
        let mut buf = [0u8; 16];
        // SAFETY: plain GL calls; every out pointer has room.
        unsafe {
            crate::gl::program::glGetProgramiv(p, GL_PROGRAM_BINARY_LENGTH, &mut n);
            assert_eq!((n, gl.error()), (0, GL_NO_ERROR));
            glGetProgramBinary(p, 0, &mut len, &mut fmt, buf.as_mut_ptr() as *mut GLvoid);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glGetProgramBinary(p, 16, &mut len, &mut fmt, buf.as_mut_ptr() as *mut GLvoid);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert_eq!(glGetFragDataLocation(p, c"gl_FragColor".as_ptr()), 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let q = glCreateProgram();
            assert_eq!(glGetFragDataLocation(q, c"gl_FragColor".as_ptr()), -1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not linked");
        }
    }

    /// ES 3.0 2.11.8 with every MAX_TRANSFORM_FEEDBACK_* at 0: an
    /// interleaved request is accepted and fails the next link; a separate
    /// one past MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS is GL_INVALID_VALUE;
    /// a name that is no program is an error. This test pinned
    /// GL_INVALID_OPERATION for any count > 0 until 2026-09-24, which no
    /// rule of the spec allows: dEQP-GLES3 treats an error from this call
    /// as fatal, and the lifetime group's init (an interleaved
    /// `gl_Position`) killed the process.
    ///
    /// Revert check: raise GL_INVALID_OPERATION for count > 0 again and the
    /// interleaved call below errs; drop the link check and the program
    /// links.
    #[cfg(feature = "report-es2")] // the ES 3.0 rules are tests/es3_tf_api.rs and gl/tf_tests.rs
    #[test]
    fn transform_feedback_varyings_fail_the_link_not_the_call() {
        let gl = fresh();
        let p = gl.program("passthrough.vert", "uniform_color.frag");
        let names = [c"gl_Position".as_ptr()];
        let link_status = |p: GLuint| {
            let mut ok: GLint = -1;
            // SAFETY: one GLint.
            unsafe { crate::gl::program::glGetProgramiv(p, GL_LINK_STATUS, &mut ok) };
            ok
        };
        // SAFETY: `names` holds one NUL-terminated name; plain GL calls.
        unsafe {
            glTransformFeedbackVaryings(p, 0, core::ptr::null(), GL_SEPARATE_ATTRIBS);
            assert_eq!(gl.error(), GL_NO_ERROR, "asking for nothing");
            glTransformFeedbackVaryings(p, 1, names.as_ptr(), GL_SEPARATE_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "one past MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS (0)");
            glTransformFeedbackVaryings(p, -1, names.as_ptr(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glTransformFeedbackVaryings(p, 0, core::ptr::null(), GL_POINTS);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glTransformFeedbackVaryings(p + 100, 0, core::ptr::null(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "no such program");

            glTransformFeedbackVaryings(p, 1, names.as_ptr(), GL_INTERLEAVED_ATTRIBS);
            assert_eq!(gl.error(), GL_NO_ERROR, "accepted: the link is where it fails");
            crate::gl::program::glLinkProgram(p);
            assert_eq!(link_status(p), 0, "MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS is 0");
            glTransformFeedbackVaryings(p, 0, core::ptr::null(), GL_INTERLEAVED_ATTRIBS);
            crate::gl::program::glLinkProgram(p);
            assert_eq!(link_status(p), 1, "the request cleared, the program links again");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[cfg(feature = "report-es2")] // the ES 3.0 rules are tests/es3_tf_api.rs and gl/tf_tests.rs
    #[test]
    fn get_transform_feedback_varying_empties_its_outputs_and_refuses() {
        let gl = fresh();
        let mut len: GLsizei = 7;
        let mut name = [b'x' as GLchar; 4];
        // SAFETY: len is one GLsizei, name four GLchars; the rest is null.
        unsafe {
            glGetTransformFeedbackVarying(
                1,
                0,
                name.len() as GLsizei,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr(),
            );
            assert_eq!(gl.error(), GL_INVALID_VALUE, "no program has any recorded varyings");
            assert_eq!(len, 0);
            assert_eq!(name[0], 0);
            glGetTransformFeedbackVarying(
                1,
                0,
                -1,
                &mut len,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                name.as_mut_ptr(),
            );
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /* ---- samplers ---- */

    #[test]
    fn a_new_sampler_holds_the_specification_defaults() {
        let gl = fresh();
        let s = gen_sampler();
        let geti = |pname| {
            let mut v: GLint = -1;
            // SAFETY: room for one value.
            unsafe { glGetSamplerParameteriv(s, pname, &mut v) };
            v
        };
        let getf = |pname| {
            let mut v: GLfloat = -1.0;
            // SAFETY: room for one value.
            unsafe { glGetSamplerParameterfv(s, pname, &mut v) };
            v
        };
        assert_eq!(geti(GL_TEXTURE_MIN_FILTER), GL_NEAREST_MIPMAP_LINEAR as GLint);
        assert_eq!(geti(GL_TEXTURE_MAG_FILTER), GL_LINEAR as GLint);
        assert_eq!(geti(GL_TEXTURE_WRAP_S), GL_REPEAT as GLint);
        assert_eq!(geti(GL_TEXTURE_WRAP_T), GL_REPEAT as GLint);
        assert_eq!(geti(GL_TEXTURE_WRAP_R), GL_REPEAT as GLint);
        assert_eq!(geti(GL_TEXTURE_COMPARE_MODE), GL_NONE as GLint);
        assert_eq!(geti(GL_TEXTURE_COMPARE_FUNC), GL_LEQUAL as GLint);
        assert_eq!(getf(GL_TEXTURE_MIN_LOD), -1000.0);
        assert_eq!(getf(GL_TEXTURE_MAX_LOD), 1000.0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn sampler_parameters_are_stored_and_read_back_exactly() {
        let gl = fresh();
        let s = gen_sampler();
        let mut i: GLint = 0;
        let mut f: GLfloat = 0.0;
        // SAFETY: each pointer has room for one value.
        unsafe {
            glSamplerParameteri(s, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glSamplerParameterf(s, GL_TEXTURE_MIN_LOD, -2.5);
            glSamplerParameteriv(s, GL_TEXTURE_MAG_FILTER, &(GL_NEAREST as GLint));
            glSamplerParameterfv(s, GL_TEXTURE_MAX_LOD, &3.5);
            assert_eq!(gl.error(), GL_NO_ERROR);

            glGetSamplerParameteriv(s, GL_TEXTURE_WRAP_S, &mut i);
            assert_eq!(i, GL_CLAMP_TO_EDGE as GLint);
            glGetSamplerParameteriv(s, GL_TEXTURE_MAG_FILTER, &mut i);
            assert_eq!(i, GL_NEAREST as GLint);
            glGetSamplerParameterfv(s, GL_TEXTURE_MIN_LOD, &mut f);
            assert_eq!(f, -2.5);
            glGetSamplerParameterfv(s, GL_TEXTURE_MAX_LOD, &mut f);
            assert_eq!(f, 3.5);
            /* the integer form of a float parameter rounds to the nearest
             * (ES 3.0 6.1.2; it truncated until 2026-09-25), halves away
             * from zero */
            glGetSamplerParameteriv(s, GL_TEXTURE_MIN_LOD, &mut i);
            assert_eq!(i, -3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn sampler_parameter_calls_check_the_name_and_the_pname() {
        let gl = fresh();
        let s = gen_sampler();
        let mut i: GLint = 0;
        // SAFETY: room for one value; the null forms write nothing.
        unsafe {
            glSamplerParameteri(s + 1, GL_TEXTURE_WRAP_S, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not a generated name");
            glSamplerParameteri(0, GL_TEXTURE_WRAP_S, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glSamplerParameteri(MAX_SAMPLERS as GLuint, GL_TEXTURE_WRAP_S, 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glSamplerParameteri(s, GL_TEXTURE_BASE_LEVEL, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a sampler-object parameter");
            glSamplerParameterf(s, GL_TEXTURE_BASE_LEVEL, 0.0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetSamplerParameteriv(s, GL_TEXTURE_BASE_LEVEL, &mut i);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glSamplerParameteriv(s, GL_TEXTURE_WRAP_S, core::ptr::null());
            glSamplerParameterfv(s, GL_TEXTURE_MIN_LOD, core::ptr::null());
            glGetSamplerParameteriv(s, GL_TEXTURE_WRAP_S, core::ptr::null_mut());
            glGetSamplerParameterfv(s, GL_TEXTURE_MIN_LOD, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR);
        }
    }

    #[test]
    fn binding_a_sampler_keeps_the_binding() {
        let gl = fresh();
        let s = gen_sampler();
        let index = gl.ctx().index;
        // SAFETY: plain GL calls.
        unsafe {
            /* ES 3.0 3.8.2: glGenSamplers creates the object (this pinned
             * GL_FALSE until 2026-09-24; dEQP negative_api.state.is_sampler) */
            assert_eq!(glIsSampler(s), GL_TRUE_B, "a generated name is a sampler");
            glBindSampler(2, s);
            /* the draw applies it (draw.rs); GL_INVALID_OPERATION until 2026-09-24 */
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glIsSampler(s), GL_TRUE_B);
            assert_eq!(bound_sampler(index)[2], s, "the binding is kept so the state query is right");

            glBindSampler(2, 0);
            assert_eq!(gl.error(), GL_NO_ERROR, "unbinding needs no feature");
            assert_eq!(bound_sampler(index)[2], 0);

            glBindSampler(CAP_COMBINED_TEXTURE_UNITS as GLuint, s);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glBindSampler(0, s + 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glBindSampler(0, MAX_SAMPLERS as GLuint);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert_eq!(bound_sampler(index)[0], 0);
        }
    }

    #[test]
    fn deleting_a_sampler_frees_its_slot_and_unbinds_it() {
        let gl = fresh();
        let s = gen_sampler();
        let index = gl.ctx().index;
        // SAFETY: plain GL calls; null and the reserved names are tolerated.
        unsafe {
            glBindSampler(1, s);
            glBindSampler(5, s);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteSamplers(1, &s);
            assert_eq!((bound_sampler(index)[1], bound_sampler(index)[5]), (0, 0));
            assert_eq!(glIsSampler(s), GL_FALSE_B);
            assert_eq!(gen_sampler(), s, "a deleted name is available again");

            let ignored = [0, MAX_SAMPLERS as GLuint, GLuint::MAX];
            glDeleteSamplers(ignored.len() as GLsizei, ignored.as_ptr());
            glDeleteSamplers(1, core::ptr::null());
            assert_eq!(gl.error(), GL_NO_ERROR);
            glDeleteSamplers(-1, &s);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGenSamplers(-1, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR, "a null array is checked before the count");
        }
    }

    #[test]
    fn a_reused_sampler_name_starts_from_the_defaults_again() {
        let gl = fresh();
        let s = gen_sampler();
        let mut v: GLint = 0;
        // SAFETY: room for one value.
        unsafe {
            glSamplerParameteri(s, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glDeleteSamplers(1, &s);
            assert_eq!(gen_sampler(), s);
            glGetSamplerParameteriv(s, GL_TEXTURE_WRAP_S, &mut v);
        }
        assert_eq!(v, GL_REPEAT as GLint);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn sampler_names_run_out_as_zeroes() {
        let gl = fresh();
        let mut names = [7u32; MAX_SAMPLERS + 1];
        // SAFETY: room for every name asked for.
        unsafe { glGenSamplers(names.len() as GLsizei, names.as_mut_ptr()) };
        assert!(names[..MAX_SAMPLERS - 1].iter().all(|&n| n != 0));
        assert_eq!(names[MAX_SAMPLERS - 1..], [0, 0]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- sync objects ---- */

    #[test]
    fn a_fence_flushes_and_names_the_kick_just_submitted() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        let s = unsafe { glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0) };
        assert!(!s.is_null());
        // SAFETY: `s` is a live sync.
        unsafe {
            assert_eq!(glIsSync(s), GL_TRUE_B);
            assert_eq!(glIsSync(core::ptr::null_mut()), GL_FALSE_B);
        }
        assert_eq!(syncs()[0].kick, backend::kick_id().wrapping_sub(1));
        assert_eq!(gl.flushes(), 1);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn fence_sync_checks_its_arguments_and_the_pool() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            assert!(glFenceSync(GL_SYNC_FENCE, 0).is_null());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert!(glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 1).is_null());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            for _ in 0..MAX_SYNCS {
                assert!(!glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0).is_null());
            }
            assert!(glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0).is_null());
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        }
    }

    #[test]
    fn deleting_a_sync_frees_its_slot_and_zero_is_ignored() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let s = glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0);
            glDeleteSync(core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR, "deleting 0 is silently ignored");
            glDeleteSync(s);
            assert_eq!(glIsSync(s), GL_FALSE_B);
            glDeleteSync(s);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "a stale handle is not a sync");
        }
    }

    #[test]
    fn client_wait_sync_reports_the_kick_and_waits_only_when_asked() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let s = glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0);
            let kick = backend::kick_id().wrapping_sub(1);
            gl.set_kick_pending(kick, false);
            assert_eq!(glClientWaitSync(s, 0, 0), GL_ALREADY_SIGNALED);
            assert_eq!(gl.finishes(), 0);

            gl.set_kick_pending(kick, true);
            assert_eq!(glClientWaitSync(s, 0, 0), GL_TIMEOUT_EXPIRED);
            assert_eq!(gl.finishes(), 0);
            assert_eq!(glClientWaitSync(s, GL_SYNC_FLUSH_COMMANDS_BIT, 1000), GL_CONDITION_SATISFIED);
            assert_eq!(gl.finishes(), 1);

            assert_eq!(glClientWaitSync(s, 2, 0), GL_WAIT_FAILED);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "no other flag bit is defined");
            assert_eq!(glClientWaitSync(core::ptr::null_mut(), 0, 0), GL_WAIT_FAILED);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /// A backend that waits for one kick: glClientWaitSync waits for the
    /// fence's kick alone and submits nothing (ES 3.0 5.2.1 flushes only with
    /// SYNC_FLUSH_COMMANDS_BIT; glFenceSync flushed). Revert check: the
    /// wait back on finish() counts a finish.
    #[test]
    fn client_wait_sync_waits_for_its_kick_alone_and_submits_nothing() {
        let gl = fresh();
        gl.backend_waits_for_kicks();
        // SAFETY: plain GL calls.
        unsafe {
            let s = glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0);
            let kick = backend::kick_id().wrapping_sub(1);
            gl.set_kick_pending(kick, true);
            let flushes = gl.flushes();
            assert_eq!(glClientWaitSync(s, GL_SYNC_FLUSH_COMMANDS_BIT, 1000), GL_CONDITION_SATISFIED);
            assert_eq!(gl.waited_kicks(), [kick], "the fence's own kick");
            assert_eq!((gl.flushes(), gl.finishes()), (flushes, 0), "nothing submitted, no finish");
            assert_eq!(glClientWaitSync(s, 0, 0), GL_ALREADY_SIGNALED);
        }
    }

    #[test]
    fn wait_sync_is_a_no_op_with_the_arguments_the_specification_pins() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let s = glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0);
            glWaitSync(s, 0, GL_TIMEOUT_IGNORED);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(gl.finishes(), 0, "one command queue: the ordering already holds");
            glWaitSync(s, 1, GL_TIMEOUT_IGNORED);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glWaitSync(s, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glWaitSync(core::ptr::null_mut(), 0, GL_TIMEOUT_IGNORED);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn get_synciv_reports_the_fence_and_its_status() {
        let gl = fresh();
        let mut len: GLsizei = 7;
        let mut v: GLint = -1;
        // SAFETY: len is one GLsizei, v one GLint.
        unsafe {
            let s = glFenceSync(GL_SYNC_GPU_COMMANDS_COMPLETE, 0);
            let kick = backend::kick_id().wrapping_sub(1);

            glGetSynciv(s, GL_OBJECT_TYPE, 1, &mut len, &mut v);
            assert_eq!((len, v), (1, GL_SYNC_FENCE as GLint));
            glGetSynciv(s, GL_SYNC_CONDITION, 1, &mut len, &mut v);
            assert_eq!(v, GL_SYNC_GPU_COMMANDS_COMPLETE as GLint);
            glGetSynciv(s, GL_SYNC_FLAGS, 1, &mut len, &mut v);
            assert_eq!(v, 0);

            gl.set_kick_pending(kick, true);
            glGetSynciv(s, GL_SYNC_STATUS, 1, &mut len, &mut v);
            assert_eq!(v, GL_UNSIGNALED as GLint);
            gl.set_kick_pending(kick, false);
            glGetSynciv(s, GL_SYNC_STATUS, 1, &mut len, &mut v);
            assert_eq!(v, GL_SIGNALED as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);

            /* no room asked for: nothing is written, the length says so */
            len = 7;
            glGetSynciv(s, GL_OBJECT_TYPE, 0, &mut len, &mut v);
            assert_eq!((len, v), (0, GL_SIGNALED as GLint));

            glGetSynciv(s, GL_SYNC_STATUS, -1, &mut len, &mut v);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetSynciv(s, GL_TEXTURE_WRAP_S, 1, &mut len, &mut v);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetSynciv(core::ptr::null_mut(), GL_OBJECT_TYPE, 1, &mut len, &mut v);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /* ---- uniform blocks ---- */

    #[test]
    fn a_program_has_no_uniform_blocks_and_says_so() {
        let gl = fresh();
        // SAFETY: plain GL call.
        let p = unsafe { glCreateProgram() };
        let mut idx = [1u32, 2];
        let mut len: GLsizei = 7;
        let mut name = [b'x' as GLchar; 4];
        // SAFETY: each pointer has room for the values asked for.
        unsafe {
            assert_eq!(glGetUniformBlockIndex(p, core::ptr::null()), GL_INVALID_INDEX);
            assert_eq!(gl.error(), GL_NO_ERROR);

            glGetUniformIndices(p, 2, core::ptr::null(), idx.as_mut_ptr());
            assert_eq!(idx, [GL_INVALID_INDEX; 2]);
            assert_eq!(gl.error(), GL_NO_ERROR);

            glGetActiveUniformsiv(p, 0, core::ptr::null(), GL_UNIFORM_TYPE, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_NO_ERROR, "asking about no uniform is not an error");
            glGetActiveUniformsiv(p, 1, core::ptr::null(), GL_UNIFORM_TYPE, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_VALUE, "every index is out of range");

            glGetActiveUniformBlockiv(p, 0, GL_UNIFORM_BLOCK_BINDING, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_VALUE);

            glGetActiveUniformBlockName(p, 0, name.len() as GLsizei, &mut len, name.as_mut_ptr());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            /* A command that errs has no other effect (ES 3.0 2.5); the stub
             * this replaced wrote an empty name before refusing. */
            assert_eq!((len, name[0]), (7, b'x' as GLchar));

            glUniformBlockBinding(p, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /// ES 3.0 2.5: a name that is no object at all is GL_INVALID_VALUE (dEQP
    /// negative_api.shader.uniform_block_binding asks exactly that of
    /// glUniformBlockBinding(-1, ..)); a shader's name is
    /// GL_INVALID_OPERATION. The stubs these calls replaced answered
    /// GL_INVALID_OPERATION to both.
    #[test]
    fn the_uniform_block_calls_reject_a_name_that_is_not_a_program() {
        let gl = fresh();
        // SAFETY: plain GL call.
        let shader = unsafe { crate::gl::shader::glCreateShader(GL_VERTEX_SHADER) };
        /* no object of either kind: shaders and programs are numbered apart,
         * so "the next program" may well be a shader */
        let bad = 1000;
        let mut len: GLsizei = 7;
        for (name, want) in [(bad, GL_INVALID_VALUE), (shader, GL_INVALID_OPERATION)] {
            // SAFETY: len is one GLsizei; the rest is null.
            unsafe {
                assert_eq!(glGetUniformBlockIndex(name, core::ptr::null()), GL_INVALID_INDEX);
                assert_eq!(gl.error(), want);
                glGetUniformIndices(name, 1, core::ptr::null(), core::ptr::null_mut());
                assert_eq!(gl.error(), want);
                glGetActiveUniformsiv(name, 1, core::ptr::null(), GL_UNIFORM_TYPE, core::ptr::null_mut());
                assert_eq!(gl.error(), want);
                glGetActiveUniformBlockiv(name, 0, GL_UNIFORM_BLOCK_BINDING, core::ptr::null_mut());
                assert_eq!(gl.error(), want);
                glGetActiveUniformBlockName(name, 0, 0, &mut len, core::ptr::null_mut());
                assert_eq!(gl.error(), want);
                glUniformBlockBinding(name, 0, 0);
                assert_eq!(gl.error(), want);
            }
        }
        assert_eq!(len, 7, "a rejected query writes nothing");
    }

    #[test]
    fn get_uniform_indices_and_block_name_reject_a_negative_count() {
        let gl = fresh();
        // SAFETY: plain GL call.
        let p = unsafe { glCreateProgram() };
        // SAFETY: the output pointers are null.
        unsafe {
            glGetUniformIndices(p, -1, core::ptr::null(), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetActiveUniformsiv(p, -1, core::ptr::null(), GL_UNIFORM_TYPE, core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glGetActiveUniformBlockName(p, 0, -1, core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    /* ---- indexed buffer bindings ---- */

    /// Transform feedback is not modelled: its points take an unbind and
    /// refuse a buffer.
    #[test]
    fn an_indexed_binding_point_takes_an_unbind_and_refuses_a_buffer() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            {
                let target = GL_TRANSFORM_FEEDBACK_BUFFER;
                glBindBufferBase(target, 0, 0);
                assert_eq!(gl.error(), GL_NO_ERROR, "there is nothing bound to undo");
                glBindBufferRange(target, 0, 0, 0, 16);
                assert_eq!(gl.error(), GL_NO_ERROR);
                /* a real buffer: refused only where nothing captures (the
                 * ES 3.0 binding is gl/tf_tests.rs) */
                if crate::REPORT_ES2 {
                    glBindBufferBase(target, 0, 1);
                    assert_eq!(gl.error(), GL_INVALID_OPERATION);
                    glBindBufferRange(target, 0, 1, 0, 16);
                    assert_eq!(gl.error(), GL_INVALID_OPERATION);
                }
            }
            glBindBufferBase(GL_ARRAY_BUFFER, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glBindBufferRange(GL_ARRAY_BUFFER, 0, 0, 0, 16);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glBindBufferRange(GL_UNIFORM_BUFFER, 0, 0, -1, 16);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glBindBufferRange(GL_UNIFORM_BUFFER, 0, 0, 0, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "an empty range is not a range");
        }
    }

    /// The uniform-buffer points record what is bound to them (ES 3.0
    /// 2.10.1.1), for the draw to read: the buffer, the range, and the
    /// generic binding too; an index past the points, an offset that is not
    /// a multiple of GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT, and an empty range
    /// are refused; deleting the buffer unbinds it.
    ///
    /// Revert check: make `bind_uniform_buffer` record nothing and the
    /// queried binding stays 0.
    #[test]
    fn a_uniform_buffer_point_records_its_buffer_and_range() {
        use crate::gl::context::MAX_UNIFORM_BUFFER_BINDINGS;
        let gl = fresh();
        let q = |pname: GLenum, i: GLuint| {
            let mut v: GLint64 = -1;
            // SAFETY: one GLint64.
            unsafe { crate::gl::state::glGetInteger64i_v(pname, i, &mut v) };
            v
        };
        let mut b = [0u32; 1];
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            crate::gl::buffer::glGenBuffers(1, b.as_mut_ptr());
            crate::gl::buffer::glBindBuffer(GL_UNIFORM_BUFFER, b[0]);
            crate::gl::buffer::glBufferData(GL_UNIFORM_BUFFER, 256, core::ptr::null(), GL_STATIC_DRAW);
            crate::gl::buffer::glBindBuffer(GL_UNIFORM_BUFFER, 0);
            glBindBufferRange(GL_UNIFORM_BUFFER, 5, b[0], 64, 32);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(gl.ctx().bound_uniform_buffer, b[0], "the generic point too");
            glBindBufferBase(GL_UNIFORM_BUFFER, 2, b[0]);
        }
        assert_eq!(
            (q(GL_UNIFORM_BUFFER_BINDING, 5), q(GL_UNIFORM_BUFFER_START, 5), q(GL_UNIFORM_BUFFER_SIZE, 5)),
            (i64::from(b[0]), 64, 32)
        );
        assert_eq!(
            (q(GL_UNIFORM_BUFFER_BINDING, 2), q(GL_UNIFORM_BUFFER_START, 2), q(GL_UNIFORM_BUFFER_SIZE, 2)),
            (i64::from(b[0]), 0, 0)
        );
        assert_eq!(q(GL_UNIFORM_BUFFER_BINDING, 0), 0);
        // SAFETY: plain GL calls.
        unsafe {
            glBindBufferBase(GL_UNIFORM_BUFFER, MAX_UNIFORM_BUFFER_BINDINGS as GLuint, b[0]);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glBindBufferRange(GL_UNIFORM_BUFFER, 1, b[0], 2, 16);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "an offset not a multiple of the alignment");
            glBindBufferRange(GL_UNIFORM_BUFFER, 1, b[0], 0, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
        let mut v: GLint = 0;
        // SAFETY: one GLint.
        unsafe {
            crate::gl::state::glGetIntegeri_v(GL_UNIFORM_BUFFER_BINDING, MAX_UNIFORM_BUFFER_BINDINGS as GLuint, &mut v)
        };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        // SAFETY: room for one name.
        unsafe { crate::gl::buffer::glDeleteBuffers(1, b.as_ptr()) };
        assert_eq!((q(GL_UNIFORM_BUFFER_BINDING, 5), q(GL_UNIFORM_BUFFER_BINDING, 2)), (0, 0));
    }

    #[test]
    fn the_64_bit_buffer_query_answers_what_the_32_bit_one_does() {
        let gl = fresh();
        let mut h = 0;
        let mut v64: GLint64 = -1;
        let mut v32: GLint = -1;
        // SAFETY: each pointer has room for one value.
        unsafe {
            crate::gl::buffer::glGenBuffers(1, &mut h);
            crate::gl::buffer::glBindBuffer(GL_ARRAY_BUFFER, h);
            crate::gl::buffer::glBufferData(GL_ARRAY_BUFFER, 64, core::ptr::null(), GL_STATIC_DRAW);
            crate::gl::buffer::glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut v32);
            glGetBufferParameteri64v(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut v64);
            assert_eq!((v32, v64), (64, 64));
            /* the error path of the 32-bit form reaches the caller unchanged */
            glGetBufferParameteri64v(GL_ARRAY_BUFFER, GL_TEXTURE_WRAP_S, &mut v64);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glGetBufferParameteri64v(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, core::ptr::null_mut());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- glClearBuffer* ---- */

    #[test]
    fn clear_bufferfv_clears_and_puts_the_clear_values_back() {
        let gl = fresh();
        // SAFETY: each value array has the four / one elements the call reads.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClearDepthf(0.5);
            glClearBufferfv(GL_COLOR, 0, [1.0f32, 0.0, 0.0, 0.5].as_ptr());
        }
        let p = draw::pending_clear();
        assert!(p.color && !p.depth);
        assert_eq!(p.rgba, [1.0, 0.0, 0.0, 0.5]);
        assert_eq!(gl.ctx().clear_color, [0.25, 0.5, 0.75, 1.0], "the clear colour is context state");

        // SAFETY: one GLfloat.
        unsafe { glClearBufferfv(GL_DEPTH, 0, [0.125f32].as_ptr()) };
        let p = draw::pending_clear();
        assert!(p.depth);
        assert_eq!(p.depth_value, 0.125);
        assert_eq!(gl.ctx().clear_depth, 0.5);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_bufferiv_clears_the_stencil_and_the_colour() {
        let gl = fresh();
        // SAFETY: each value array has the four / one elements the call reads.
        unsafe {
            glClearStencil(3);
            glClearBufferiv(GL_STENCIL, 0, [9i32].as_ptr());
            let p = draw::pending_clear();
            assert!(p.stencil);
            assert_eq!(p.stencil_value, 9);
            assert_eq!(gl.ctx().clear_stencil, 3, "the clear stencil is context state");

            /* the default framebuffer is fixed point: the integer forms do not
             * apply to it (ES 3.0 4.2.3), and a refused clear records nothing */
            glClearBufferiv(GL_COLOR, 0, [1i32, 0, 0, 1].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert!(!draw::pending_clear().color);
        }
    }

    /// A framebuffer with one integer renderbuffer of `format`, bound.
    #[cfg(not(feature = "report-es2"))]
    fn integer_fbo(format: GLenum) {
        use crate::gl::framebuffer::*;
        let (rb, fb) = (gen1(glGenRenderbuffers), gen1(glGenFramebuffers));
        // SAFETY: plain GL calls on names just generated.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, format, 4, 4);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        }
    }

    #[test]
    #[cfg(not(feature = "report-es2"))]
    fn clear_bufferiv_and_uiv_record_the_raw_integers_for_an_integer_buffer() {
        let gl = fresh();
        integer_fbo(GL_RGBA16I);
        // SAFETY: four values each; plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClearBufferiv(GL_COLOR, 0, [-1i32, 7, i32::MIN, 3].as_ptr());
            assert_eq!(gl.error(), GL_NO_ERROR);
            let bits = draw::pending_clear().rgba.map(f32::to_bits);
            assert_eq!(bits, [-1i32 as u32, 7, i32::MIN as u32, 3], "the integers, bit for bit");
            assert_eq!(gl.ctx().clear_color, [0.25, 0.5, 0.75, 1.0], "the clear colour is context state");
            glClearBufferuiv(GL_COLOR, 0, [1u32, 2, 3, 4].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "unsigned form on a signed buffer");
        }
    }

    #[test]
    fn clear_bufferuiv_clears_the_colour_only() {
        let gl = fresh();
        // SAFETY: four GLuints, or null.
        unsafe {
            /* the default framebuffer is fixed point (ES 3.0 4.2.3) */
            glClearBufferuiv(GL_COLOR, 0, [0u32, 1, 2, 3].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert!(!draw::pending_clear().color);
            glClearBufferuiv(GL_DEPTH, 0, [0u32; 4].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            /* draw buffer 1: past GL_MAX_DRAW_BUFFERS on the GC7000 (1), in
             * range and a no-op on the PowerVR (3); this pinned
             * GL_INVALID_VALUE for both until 2026-09-24 */
            glClearBufferuiv(GL_COLOR, 1, [0u32; 4].as_ptr());
            let want = if context::MAX_DRAW_BUFFERS == 1 { GL_INVALID_VALUE } else { GL_NO_ERROR };
            assert_eq!(gl.error(), want, "draw buffer 1 against MAX_DRAW_BUFFERS");
            /* draw buffer 0 of the default framebuffer is fixed point, so a null value is the only no-op */
            glClearBufferuiv(GL_COLOR, 0, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn clear_bufferfi_clears_depth_and_stencil_together() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(1.0);
            glClearStencil(0);
            glClearBufferfi(GL_DEPTH_STENCIL, 0, 0.25, 7);
            let p = draw::pending_clear();
            assert!(p.depth && p.stencil);
            assert_eq!((p.depth_value, p.stencil_value), (0.25, 7));
            assert_eq!((gl.ctx().clear_depth, gl.ctx().clear_stencil), (1.0, 0));
            assert_eq!(gl.error(), GL_NO_ERROR);

            glClearBufferfi(GL_DEPTH, 0, 0.0, 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glClearBufferfi(GL_DEPTH_STENCIL, 1, 0.0, 0);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
        }
    }

    #[test]
    fn clear_buffer_calls_reject_a_second_draw_buffer_and_an_unknown_one() {
        let gl = fresh();
        // SAFETY: each value array has four elements, or is null.
        unsafe {
            /* as in clear_bufferuiv_clears_the_colour_only: the range is
             * GL_MAX_DRAW_BUFFERS' (this pinned one draw buffer everywhere) */
            let want = if context::MAX_DRAW_BUFFERS == 1 { GL_INVALID_VALUE } else { GL_NO_ERROR };
            glClearBufferfv(GL_COLOR, 1, [0.0f32; 4].as_ptr());
            assert_eq!(gl.error(), want);
            glClearBufferiv(GL_COLOR, 1, [0i32; 4].as_ptr());
            assert_eq!(gl.error(), want);
            glClearBufferfv(GL_STENCIL, 0, [0.0f32; 4].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "the stencil buffer has no float form");
            glClearBufferiv(GL_DEPTH, 0, [0i32; 4].as_ptr());
            assert_eq!(gl.error(), GL_INVALID_ENUM, "the depth buffer has no integer form");
            glClearBufferfv(GL_COLOR, 0, core::ptr::null());
            glClearBufferiv(GL_COLOR, 0, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- programs ---- */

    #[test]
    fn the_program_binary_calls_refuse_because_there_are_no_formats() {
        let gl = fresh();
        // SAFETY: plain GL call.
        let p = unsafe { glCreateProgram() };
        let mut len: GLsizei = 7;
        // SAFETY: len is one GLsizei; the rest is null.
        unsafe {
            glGetProgramBinary(p, 0, &mut len, core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert_eq!(len, 0);
            /* the refusal is the answer for every bufSize, a negative one
             * too (dEQP get_program_binary expects it on an unlinked program
             * with bufSize -1; this pinned GL_INVALID_VALUE until 2026-09-24) */
            glGetProgramBinary(p, -1, &mut len, core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            glGetProgramBinary(p + 1, 0, &mut len, core::ptr::null_mut(), core::ptr::null_mut());
            assert_eq!(gl.error(), GL_INVALID_OPERATION);

            glProgramBinary(p, 0, core::ptr::null(), 0);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "no format is accepted");
            glProgramBinary(p, 0, core::ptr::null(), -1);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glProgramBinary(p + 1, 0, core::ptr::null(), 0);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn the_program_binary_hint_is_accepted_and_ignored() {
        let gl = fresh();
        // SAFETY: plain GL calls.
        unsafe {
            let p = glCreateProgram();
            glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_FALSE as GLint);
            assert_eq!(gl.error(), GL_NO_ERROR);
            glProgramParameteri(p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, 2);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glProgramParameteri(p, GL_TEXTURE_WRAP_S, GL_TRUE as GLint);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glProgramParameteri(p + 1, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    /// A linked program's one colour output is at location 0; an unlinked
    /// one has no locations to ask about (ES 3.0 2.12.5, GL_INVALID_OPERATION;
    /// dEQP negative_api.state.get_frag_data_location). This pinned 0 for
    /// an unlinked program until 2026-09-24.
    #[test]
    fn the_fragment_output_of_a_program_is_at_location_zero() {
        let gl = fresh();
        let linked = gl.program("passthrough.vert", "uniform_color.frag");
        // SAFETY: plain GL calls; the name is never read.
        unsafe {
            assert_eq!(glGetFragDataLocation(linked, core::ptr::null()), 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let p = glCreateProgram();
            assert_eq!(glGetFragDataLocation(p, core::ptr::null()), -1, "not linked");
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
            assert_eq!(glGetFragDataLocation(p + 1, core::ptr::null()), -1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION);
        }
    }

    #[test]
    fn get_uniformuiv_is_the_signed_query_with_the_same_storage() {
        let gl = fresh();
        let mut u: GLuint = 5;
        let mut i: GLint = 5;
        // SAFETY: each pointer has room for one value.
        unsafe {
            let p = glCreateProgram();
            glGetUniformuiv(p, 0, &mut u);
            let e_u = glGetError();
            crate::gl::program::glGetUniformiv(p, 0, &mut i);
            let e_i = glGetError();
            assert_eq!((e_u, u), (e_i, i as GLuint), "the two forms answer alike");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- application pointers ---- */

    #[test]
    fn the_name_arrays_are_read_and_written_unaligned() {
        let gl = fresh();
        /* names at an odd byte offset: a GLuint* the ABI does not align */
        let mut buf = [0u8; 1 + 2 * 4];
        let p = buf[1..].as_mut_ptr() as *mut GLuint;
        // SAFETY: `p` has room for two names (unaligned).
        unsafe {
            glGenSamplers(2, p);
            let a = u32::from_ne_bytes(buf[1..5].try_into().unwrap());
            let b = u32::from_ne_bytes(buf[5..9].try_into().unwrap());
            assert!(a != 0 && b != 0 && a != b);
            glBindSampler(0, a);
            assert_eq!(gl.error(), GL_NO_ERROR);
            let p = buf[1..].as_ptr() as *const GLuint;
            glDeleteSamplers(2, p);
            assert_eq!(glIsSampler(a), GL_FALSE_B);
            assert_eq!(bound_sampler(gl.ctx().index)[0], 0);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- per context ---- */

    #[test]
    fn the_pools_are_per_context() {
        let gl = fresh();
        let q = gen_query();
        let s = gen_sampler();
        let other: u8 = if gl.ctx().index == 0 { 1 } else { 0 };
        assert!(!queries(other)[q as usize].used, "another context's pool is untouched");
        assert_eq!(samplers(other)[s as usize], Es3Sampler::FREE);
        assert!(queries(gl.ctx().index)[q as usize].used);
    }
}
