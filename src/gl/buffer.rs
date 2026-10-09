// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Vertex/index buffer objects (VBO/IBO).
//!
//! Each GL buffer is backed by one GPU-resident allocation (the OSAL's
//! gpu_alloc). glBufferData (re)sizes it and copies the initial data in;
//! glBufferSubData updates a sub-range. No client-memory shadow: the GPU
//! allocation is the store.
#![allow(non_snake_case)]

use core::ffi::c_void;
use core::fmt::Write as _;

use super::consts::*;
use super::context::{self, gl_trace, heap_data, GlBuffer, GlContext, MAX_BUFFERS};
use super::names::{self, Kind};
use super::types::*;
use crate::backend;
use crate::osal::{self, OsalGpuMem, OSAL_MEM_DATA};
use crate::util::{FmtBuf, Global};

/// One recorded binding event. op: 1 bind elem, 2 delete, 3 bind vao, 4 bind array.
#[derive(Clone, Copy)]
struct BindHistEntry {
    op: u8,
    h: u32,
}

const BIND_HIST_LEN: usize = 48;

/// File-level state: the binding history ring and the "out of names" latch.
struct BufferState {
    hist: [BindHistEntry; BIND_HIST_LEN],
    hist_n: u32,
    /// glGenBuffers has already reported running out of names.
    warned: bool,
}

static STATE: Global<BufferState> = Global::zeroed();

/// A buffer slot as memset(0) leaves it.
const fn zeroed_buffer() -> GlBuffer {
    GlBuffer {
        used: false,
        target: 0,
        usage: 0,
        mem: OsalGpuMem::ZERO,
        size: 0,
        last_use: 0,
        mapped: core::ptr::null_mut(),
        map_off: 0,
        map_len: 0,
        map_access: 0,
    }
}

/// Resolve a buffer *target* enum to the currently bound buffer handle.
fn binding_for_target(c: &mut GlContext, target: GLenum) -> Option<&mut GLuint> {
    match target {
        GL_ARRAY_BUFFER => Some(&mut c.bound_array_buffer),
        GL_ELEMENT_ARRAY_BUFFER => Some(&mut c.bound_element_buffer),
        GL_PIXEL_PACK_BUFFER => Some(&mut c.bound_pixel_pack_buffer),
        GL_PIXEL_UNPACK_BUFFER => Some(&mut c.bound_pixel_unpack_buffer),
        GL_COPY_READ_BUFFER => Some(&mut c.bound_copy_read_buffer),
        GL_COPY_WRITE_BUFFER => Some(&mut c.bound_copy_write_buffer),
        GL_UNIFORM_BUFFER => Some(&mut c.bound_uniform_buffer),
        GL_TRANSFORM_FEEDBACK_BUFFER => Some(&mut c.bound_transform_feedback_buffer),
        _ => None,
    }
}

/// Every buffer binding point of a context, for the unbind glDeleteBuffers
/// owes a deleted name (ES 3.0 2.9).
fn all_bindings(c: &mut GlContext) -> [&mut GLuint; 8] {
    [
        &mut c.bound_array_buffer,
        &mut c.bound_element_buffer,
        &mut c.bound_pixel_pack_buffer,
        &mut c.bound_pixel_unpack_buffer,
        &mut c.bound_copy_read_buffer,
        &mut c.bound_copy_write_buffer,
        &mut c.bound_uniform_buffer,
        &mut c.bound_transform_feedback_buffer,
    ]
}

/// The table index of the buffer bound to `target`: `Err(())` for a target
/// that is not a buffer target, `Ok(None)` when nothing (or a stale name) is
/// bound.
fn bound_buffer_for_target(c: &mut GlContext, target: GLenum) -> Result<Option<usize>, ()> {
    let h = *binding_for_target(c, target).ok_or(())?;
    Ok(c.buffer_ref(h).map(|_| h as usize))
}

/// Copy CPU data into a GPU allocation (CPU-mapped or via the OSAL bulk path),
/// then flush so the GPU sees it.
///
/// # Safety
/// `src` must be valid for `n` bytes and `off + n` must lie inside `mem`.
unsafe fn upload(mem: &mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    if !mem.cpu.is_null() {
        let dst = (mem.cpu as *mut u8).add(off);
        core::ptr::copy_nonoverlapping(src as *const u8, dst, n);
        osal::cache_flush(dst as *mut c_void, n);
        return 0;
    }
    match osal::ops().and_then(|o| o.gpu_write) {
        Some(f) => f(mem, off, src, n),
        None => -1,
    }
}

/// Fill a fresh allocation with zero (CPU-mapped or through the OSAL), so
/// that whatever the pool held before does not reach the application or the
/// GPU. GL leaves the contents of glBufferData(NULL) undefined, but the
/// Zephyr and bare-metal pools are not zeroed and a mapped buffer hands the
/// pages straight to the application. The cost is one memset (or one
/// gpu_write) of the allocation per *new* allocation; storage that is reused
/// keeps its bytes, and data passed to glBufferData overwrites them anyway.
///
/// The whole allocation is cleared, not just the object's size: an allocator
/// that rounds up would otherwise leave the previous user's bytes in the
/// tail (the C clears `out->size` too).
fn clear_storage(mem: &mut OsalGpuMem) -> i32 {
    let n = mem.size;

    if !mem.cpu.is_null() {
        // SAFETY: `n` is the size of the allocation `mem` describes.
        unsafe {
            core::ptr::write_bytes(mem.cpu as *mut u8, 0, n);
            osal::cache_flush(mem.cpu, n);
        }
        return 0;
    }
    let Some(f) = osal::ops().and_then(|o| o.gpu_write) else {
        return -1;
    };
    let zero = [0u8; 256];
    let mut off = 0usize;
    while off < n {
        let k = (n - off).min(zero.len());
        // SAFETY: `mem` is a live allocation of at least `n` bytes.
        if unsafe { f(mem, off, zero.as_ptr() as *const c_void, k) } != 0 {
            return -1;
        }
        off += k;
    }
    0
}

/// As glUnmapBuffer for a buffer whose store is about to be replaced or
/// released (ES 3.0 2.10.2: glBufferData unmaps a mapped buffer).
fn unmap_buffer(b: &mut GlBuffer) {
    if b.mapped.is_null() {
        return;
    }
    if b.map_access & GL_MAP_WRITE_BIT != 0 && b.map_access & GL_MAP_FLUSH_EXPLICIT_BIT == 0 {
        // SAFETY: the mapped range lies inside the buffer's live CPU mapping.
        unsafe { osal::cache_flush(b.mapped, b.map_len as usize) };
    }
    b.mapped = core::ptr::null_mut();
    b.map_off = 0;
    b.map_len = 0;
    b.map_access = 0;
}

/// Application arrays of GLuint carry no alignment guarantee: the names are
/// read and written one by one, unaligned.
///
/// # Safety
/// `p` must be valid for reading `i + 1` GLuints.
unsafe fn name_at(p: *const GLuint, i: usize) -> GLuint {
    p.wrapping_add(i).read_unaligned()
}

/// glBufferData's `usage` hints (ES 2.0's three plus the ES 3.0 six). They
/// are recorded for the query and otherwise ignored: the store is always
/// GPU-resident.
fn is_buffer_usage(usage: GLenum) -> bool {
    matches!(
        usage,
        GL_STREAM_DRAW
            | GL_STATIC_DRAW
            | GL_DYNAMIC_DRAW
            | GL_STREAM_READ
            | GL_STREAM_COPY
            | GL_STATIC_READ
            | GL_STATIC_COPY
            | GL_DYNAMIC_READ
            | GL_DYNAMIC_COPY
    )
}

/* --------------------------------------------------------------------------- */

#[no_mangle]
pub unsafe extern "C" fn glGenBuffers(n: GLsizei, buffers: *mut GLuint) {
    gl_trace!(glGenBuffers);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glGenBuffers");
        return;
    }
    let Some(c) = context::current() else { return };
    if buffers.is_null() {
        return;
    }
    let n = n as usize;
    let mut made = 0usize;
    for h in 1..MAX_BUFFERS {
        if made >= n {
            break;
        }
        if !c.objs().buffers[h].used {
            let b = &mut c.objs().buffers[h];
            *b = zeroed_buffer();
            b.used = true; /* name reserved; storage on first glBufferData */
            // SAFETY: the application passes room for `n` names (n >= 0
            // checked above); written unaligned (see name_at).
            buffers.wrapping_add(made).write_unaligned(h as GLuint);
            made += 1;
        }
    }
    /* Out of names: the rest are 0. A client binding name 0 then draws
     * from client memory it does not have (Qt: indexed draws with buffer
     * offsets), so say so once and report an error. */
    if made < n {
        // SAFETY: single-threaded by contract; the only reference to STATE here.
        let st = STATE.get();
        if !st.warned && osal::ops().is_some_and(|o| o.log.is_some()) {
            st.warned = true;
            glog!("glGenBuffers: out of buffer names (GL_MAX_BUFFERS {})", MAX_BUFFERS);
        }
        c.set_error(GL_OUT_OF_MEMORY, "glGenBuffers");
    }
    for i in made..n {
        // SAFETY: as above.
        buffers.wrapping_add(i).write_unaligned(0);
    }
}

/* Debug: the last binding-related calls (ELEMENT_ARRAY_BUFFER binds, buffer
 * deletes, VAO binds), dumped by glDrawElements when it finds no element
 * buffer bound for what looks like a buffer offset. */

/// Bring-up debug: record a buffer bind/delete event (op 1 = bind element, 2 = delete, 3 = bind VAO, 4 = bind other).
pub fn debug_bind_hist(op: i32, h: u32) {
    // SAFETY: single-threaded by contract; no other reference to STATE is live.
    let st = unsafe { STATE.get() };
    st.hist[st.hist_n as usize % BIND_HIST_LEN] = BindHistEntry { op: op as u8, h };
    st.hist_n = st.hist_n.wrapping_add(1);
}

pub fn debug_dump_bind_hist() {
    const NAMES: [&str; 5] = ["?", "bind-elem", "delete", "bind-vao", "bind-array"];

    if osal::ops().is_none_or(|o| o.log.is_none()) {
        return;
    }
    // SAFETY: single-threaded by contract; no other reference to STATE is live.
    let st = unsafe { STATE.get() };
    let mut line = FmtBuf::<160>::new();
    let start = st.hist_n.saturating_sub(BIND_HIST_LEN as u32);
    for i in start..st.hist_n {
        let e = &st.hist[i as usize % BIND_HIST_LEN];
        let _ = write!(line, " {}:{}", NAMES[if e.op < 5 { e.op as usize } else { 0 }], e.h);
        if line.as_bytes().len() > 120 {
            glog!("  hist:{}", crate::util::Bytes(line.as_bytes()));
            line = FmtBuf::new();
        }
    }
    if !line.as_bytes().is_empty() {
        glog!("  hist:{}", crate::util::Bytes(line.as_bytes()));
    }
}

#[no_mangle]
pub extern "C" fn gl_debug_bind_hist(op: i32, h: u32) {
    debug_bind_hist(op, h);
}

#[no_mangle]
pub extern "C" fn gl_debug_dump_bind_hist() {
    debug_dump_bind_hist();
}

#[no_mangle]
pub unsafe extern "C" fn glBindBuffer(target: GLenum, buffer: GLuint) {
    gl_trace!(glBindBuffer);
    let Some(c) = context::current() else { return };
    if binding_for_target(c, target).is_none() {
        c.set_error(GL_INVALID_ENUM, "glBindBuffer");
        return;
    }
    debug_bind_hist(if target == GL_ELEMENT_ARRAY_BUFFER { 1 } else { 4 }, buffer);

    /* the slot the name lives in; a name past the pool borrows one */
    let Some(buffer) = names::bind_slot(c, Kind::Buffer, buffer, |s| !c.objs_ref().buffers[s].used) else {
        c.set_error(GL_OUT_OF_MEMORY, "glBindBuffer");
        return;
    };
    if buffer != 0 {
        let b = &mut c.objs().buffers[buffer as usize];
        if !b.used {
            /* first bind creates the name */
            *b = zeroed_buffer();
            b.used = true;
        }
        b.target = target;
    }
    if let Some(bind) = binding_for_target(c, target) {
        *bind = buffer;
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBufferData(target: GLenum, size: GLsizeiptr, data: *const c_void, usage: GLenum) {
    gl_trace!(glBufferData);
    let Some(c) = context::current() else { return };
    let Ok(bound) = bound_buffer_for_target(c, target) else {
        c.set_error(GL_INVALID_ENUM, "glBufferData");
        return;
    };
    if !is_buffer_usage(usage) {
        c.set_error(GL_INVALID_ENUM, "glBufferData");
        return;
    }
    if size < 0 {
        c.set_error(GL_INVALID_VALUE, "glBufferData");
        return;
    }

    let Some(i) = bound else {
        c.set_error(GL_INVALID_OPERATION, "glBufferData"); /* no buffer bound */
        return;
    };
    let b = &mut c.objs().buffers[i];
    b.usage = usage; /* the usage is a hint; kept for the query */
    unmap_buffer(b); /* ES 3.0 2.10.2: the store goes, so does the mapping */

    /* (Re)allocate GPU storage if the existing allocation can't hold `size`,
     * or if a kick that has not completed may still read it (the frame being
     * recorded, or one in flight): the old storage is released once those
     * kicks are done (deferred free), the new data goes into fresh pages. */
    let mut fresh = false;
    if b.mem.size < size as usize || b.mem.impl_.is_null() || backend::kick_pending(b.last_use) {
        if !b.mem.impl_.is_null() {
            osal::gpu_free(&mut b.mem);
        }
        b.mem = OsalGpuMem::ZERO;
        if size > 0 && osal::gpu_alloc(heap_data(), size as usize, OSAL_MEM_DATA, &mut b.mem) != 0 {
            b.size = 0;
            c.set_error(GL_OUT_OF_MEMORY, "glBufferData");
            return;
        }
        fresh = !b.mem.impl_.is_null();
    }
    b.size = size;

    let rc = if !data.is_null() && size > 0 && !b.mem.impl_.is_null() {
        // SAFETY: the application passes `size` bytes at `data`; the allocation holds at least `size`.
        unsafe { upload(&mut b.mem, 0, data, size as usize) }
    } else if fresh {
        /* no data: the fresh pages must not show what the pool held */
        clear_storage(&mut b.mem)
    } else {
        0
    };
    if rc != 0 {
        /* the store could not be filled: leave no buffer rather than one
         * holding stale bytes the application never wrote */
        osal::gpu_free(&mut b.mem);
        b.mem = OsalGpuMem::ZERO;
        b.size = 0;
        c.set_error(GL_OUT_OF_MEMORY, "glBufferData");
    }
}

#[no_mangle]
pub unsafe extern "C" fn glBufferSubData(target: GLenum, offset: GLintptr, size: GLsizeiptr, data: *const c_void) {
    gl_trace!(glBufferSubData);
    let Some(c) = context::current() else { return };
    let Ok(bound) = bound_buffer_for_target(c, target) else {
        c.set_error(GL_INVALID_ENUM, "glBufferSubData");
        return;
    };
    if offset < 0 || size < 0 {
        c.set_error(GL_INVALID_VALUE, "glBufferSubData");
        return;
    }

    /* ES 3.0 2.10.2: not on a mapped buffer */
    let Some(i) = bound.filter(|&i| !c.objs().buffers[i].mem.impl_.is_null() && c.objs().buffers[i].mapped.is_null())
    else {
        c.set_error(GL_INVALID_OPERATION, "glBufferSubData");
        return;
    };
    /* range check without the sum (offset + size can wrap) */
    if offset > c.objs().buffers[i].size || size > c.objs().buffers[i].size - offset {
        c.set_error(GL_INVALID_VALUE, "glBufferSubData");
        return;
    }
    if data.is_null() || size == 0 {
        return;
    }
    if backend::kick_pending(c.objs().buffers[i].last_use) {
        /* A pending kick still reads this storage: copy on write. */
        let mut fresh = OsalGpuMem::ZERO;
        let bytes = c.objs().buffers[i].size as usize;

        if osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, &mut fresh) != 0 {
            c.set_error(GL_OUT_OF_MEMORY, "glBufferSubData");
            return;
        }
        let b = &mut c.objs().buffers[i];
        let bounce = osal::ops().and_then(|o| o.gpu_read.zip(o.gpu_write));
        let mut rc = 0;
        if !b.mem.cpu.is_null() && !fresh.cpu.is_null() {
            // SAFETY: both allocations are live, distinct and hold at least `bytes`.
            osal::cache_invd(b.mem.cpu, bytes);
            core::ptr::copy_nonoverlapping(b.mem.cpu as *const u8, fresh.cpu as *mut u8, bytes);
            /* the carried bytes reach the GPU only with a flush: the update
             * below publishes its own range, this one the rest */
            osal::cache_flush(fresh.cpu, bytes);
        } else if let Some((gpu_read, gpu_write)) = bounce {
            /* platforms without a CPU view: bounce through the stack in chunks */
            let mut tmp = [0u8; 512];
            let mut o = 0usize;

            while o < bytes {
                let n = (bytes - o).min(tmp.len());

                // SAFETY: `tmp` holds `n` bytes; `o + n` lies inside both allocations.
                rc |= gpu_read(&mut b.mem, o, tmp.as_mut_ptr() as *mut c_void, n);
                rc |= gpu_write(&mut fresh, o, tmp.as_ptr() as *const c_void, n);
                o += n;
            }
        } else {
            rc = -1; /* no way to carry the contents over */
        }
        if rc != 0 {
            /* the copy failed: keep the old storage (the kick in flight
             * still reads it) and report, rather than moving the buffer to
             * pages holding garbage */
            osal::gpu_free(&mut fresh);
            c.set_error(GL_OUT_OF_MEMORY, "glBufferSubData");
            return;
        }
        osal::gpu_free(&mut b.mem); /* deferred until the kicks are done */
        b.mem = fresh;
        b.last_use = 0;
    }
    // SAFETY: the application passes `size` bytes at `data`; offset + size <= the buffer's size (checked above).
    if upload(&mut c.objs().buffers[i].mem, offset as usize, data, size as usize) != 0 {
        c.set_error(GL_OUT_OF_MEMORY, "glBufferSubData");
    }
}

#[no_mangle]
pub unsafe extern "C" fn glDeleteBuffers(n: GLsizei, buffers: *const GLuint) {
    gl_trace!(glDeleteBuffers);
    if n < 0 {
        context::set_error_no_ctx(GL_INVALID_VALUE, "glDeleteBuffers");
        return;
    }
    let Some(c) = context::current() else { return };
    if buffers.is_null() {
        return;
    }
    for i in 0..n as usize {
        // SAFETY: the application passes `n` names (n >= 0 checked above).
        let Some(h) = names::slot_of(c, Kind::Buffer, name_at(buffers, i)) else {
            continue;
        };
        let b = &mut c.objs().buffers[h as usize];
        if !b.used {
            continue;
        }
        /* Unbind from every binding point (ES 3.0 2.9: deleting a buffer
         * reverts each binding it holds to zero). Missing the ES 3.0 points
         * left a deleted name bound to PIXEL_UNPACK_BUFFER, and the next
         * pixel transfer resolved its offset against a buffer that no
         * longer existed. */
        for bind in all_bindings(c) {
            if *bind == h {
                *bind = 0;
            }
        }
        /* ES 2.0.25 2.9: "If a buffer object is deleted while it is bound,
         * all bindings to that object in the current context ... are reset
         * to zero", and a vertex array's buffer binding
         * (VERTEX_ATTRIB_ARRAY_BUFFER_BINDING) is one of them. `c.attribs`
         * is the bound vertex array object's live state (vao.rs), so this
         * covers the default VAO and a bound one; ES 3.0 D.1.2 leaves the
         * attachments of an unbound VAO alone. Before, the attribute kept
         * the deleted slot: the next draw failed, and after glGenBuffers
         * reused the slot it read an unrelated buffer. */
        for a in c.attribs.iter_mut() {
            if a.buffer == h {
                a.buffer = 0;
            }
        }
        /* The indexed points too (a uniform block would otherwise read a
         * buffer that no longer exists). */
        for u in super::context::uniform_buffers(c).iter_mut() {
            if u.buffer == h {
                *u = super::context::UboBinding::default();
            }
        }
        debug_bind_hist(2, h);
        names::forget(c, Kind::Buffer, h);
        /* A transform feedback object that is not current keeps its
         * reference (ES 3.0 D.1.2): the name is gone, the storage lives on
         * until the object lets go (`tf::release_slot` -> `free_orphan`). */
        if super::tf::on_buffer_delete(c, h) {
            c.objs().buffers[h as usize].target = 0; /* glIsBuffer: false */
            continue;
        }
        free_buffer_storage(c, h);
    }
}

/// Give a deleted buffer's storage back and free its slot.
fn free_buffer_storage(c: &mut GlContext, h: GLuint) {
    let b = &mut c.objs().buffers[h as usize];
    if !b.mem.impl_.is_null() {
        osal::gpu_free(&mut b.mem);
    }
    c.objs().buffers[h as usize] = zeroed_buffer();
}

/// The last transform feedback object holding a deleted buffer let go of it.
pub(super) fn free_orphan(c: &mut GlContext, h: GLuint) {
    if c.objs().buffers.get(h as usize).is_some_and(|b| b.used) {
        free_buffer_storage(c, h);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glIsBuffer(buffer: GLuint) -> GLboolean {
    gl_trace!(glIsBuffer);
    /* ES 3.0 6.1.4: a name glGenBuffers returned is a buffer object only
     * once it has been bound; `target` is 0 until then. */
    match context::current() {
        Some(c)
            if names::slot_of(c, Kind::Buffer, buffer).and_then(|s| c.buffer_ref(s)).is_some_and(|b| b.target != 0) =>
        {
            GL_TRUE_B
        }
        _ => GL_FALSE_B,
    }
}

#[no_mangle]
pub unsafe extern "C" fn glGetBufferParameteriv(target: GLenum, pname: GLenum, params: *mut GLint) {
    gl_trace!(glGetBufferParameteriv);
    let Some(c) = context::current() else { return };
    if params.is_null() {
        return;
    }
    let Ok(bound) = bound_buffer_for_target(c, target) else {
        c.set_error(GL_INVALID_ENUM, "glGetBufferParameteriv");
        return;
    };
    let Some(i) = bound else {
        c.set_error(GL_INVALID_OPERATION, "glGetBufferParameteriv");
        return;
    };
    let b = &c.objs_ref().buffers[i];
    let v = match pname {
        GL_BUFFER_SIZE => b.size.clamp(0, GLint::MAX as GLsizeiptr) as GLint,
        GL_BUFFER_USAGE => (if b.usage != 0 { b.usage } else { GL_STATIC_DRAW }) as GLint,
        /* ES 3.0 6.1.9: the mapping state */
        GL_BUFFER_MAPPED => !b.mapped.is_null() as GLint,
        GL_BUFFER_ACCESS_FLAGS => b.map_access as GLint,
        GL_BUFFER_MAP_OFFSET => b.map_off.clamp(0, GLint::MAX as GLintptr) as GLint,
        GL_BUFFER_MAP_LENGTH => b.map_len.clamp(0, GLint::MAX as GLsizeiptr) as GLint,
        /* GL_OES_mapbuffer: its one access value, the initial state too */
        GL_BUFFER_ACCESS_OES => GL_WRITE_ONLY_OES as GLint,
        _ => return c.set_error(GL_INVALID_ENUM, "glGetBufferParameteriv"),
    };
    // SAFETY: non-null (checked above), room for one value; GLint* carries
    // no alignment guarantee.
    params.write_unaligned(v);
}

/// `gl_pixel_buffer_resolve()`: a pixel transfer with PIXEL_UNPACK_BUFFER
/// (or PIXEL_PACK_BUFFER, for a read) bound takes its pointer argument as a
/// byte offset into that buffer instead of a client address (ES 3.0 3.7.1,
/// 4.3.1). Resolving it once, here, lets every transfer path go on working
/// with a plain pointer; the paths used to read the offset as a pointer and
/// dereference it.
///
/// Returns the pointer to use, or `None` when the binding cannot be honoured
/// (the error is set). `ptr` comes back unchanged when no buffer is bound.
/// `bytes` is what the transfer will touch, so that an offset past the end of
/// the buffer is caught rather than walked off.
pub fn pixel_buffer_resolve(
    c: &mut GlContext,
    pack: bool,
    ptr: *const c_void,
    bytes: usize,
    whom: &str,
) -> Option<*const c_void> {
    let name = if pack { c.bound_pixel_pack_buffer } else { c.bound_pixel_unpack_buffer };

    if name == 0 {
        return Some(ptr);
    }
    let bad = |c: &mut GlContext| {
        c.set_error(GL_INVALID_OPERATION, whom);
        None
    };
    let Some(b) = c.buffer_ref(name) else { return bad(c) };
    /* A mapped buffer may not be the source or the target of a transfer. */
    if b.mem.cpu.is_null() || !b.mapped.is_null() {
        return bad(c);
    }
    let size = b.size.max(0) as usize;
    let off = ptr as usize;

    if off > size || bytes > size - off {
        return bad(c);
    }
    let cpu = b.mem.cpu;
    // SAFETY: `off` lies inside the allocation (checked just above).
    Some(unsafe { (cpu as *const u8).add(off) } as *const c_void)
}

/// OpenGL ES 3.0: the pointer a mapped buffer handed out.
#[no_mangle]
pub unsafe extern "C" fn glGetBufferPointerv(target: GLenum, pname: GLenum, params: *mut *mut c_void) {
    gl_trace!(glGetBufferPointerv);
    let Some(c) = context::current() else { return };
    buffer_pointer(c, target, pname, params, "glGetBufferPointerv");
}

/// glGetBufferPointerv's body, for it and glGetBufferPointervOES
/// (GL_BUFFER_MAP_POINTER_OES is GL_BUFFER_MAP_POINTER, 0x88BD).
fn buffer_pointer(c: &mut GlContext, target: GLenum, pname: GLenum, params: *mut *mut c_void, func: &'static str) {
    if params.is_null() {
        return;
    }
    let Ok(bound) = bound_buffer_for_target(c, target) else {
        c.set_error(GL_INVALID_ENUM, func);
        return;
    };
    if pname != GL_BUFFER_MAP_POINTER {
        c.set_error(GL_INVALID_ENUM, func);
        return;
    }
    let Some(i) = bound else {
        c.set_error(GL_INVALID_OPERATION, func);
        return;
    };
    let mapped = c.objs().buffers[i].mapped;
    // SAFETY: non-null (checked above), room for one pointer, any alignment.
    unsafe { params.write_unaligned(mapped) };
}

/// OpenGL ES 3.0: a copy between two buffers that never goes through the
/// application.
#[no_mangle]
pub unsafe extern "C" fn glCopyBufferSubData(
    readTarget: GLenum,
    writeTarget: GLenum,
    readOffset: GLintptr,
    writeOffset: GLintptr,
    size: GLsizeiptr,
) {
    gl_trace!(glCopyBufferSubData);
    let Some(c) = context::current() else { return };
    let (Ok(r), Ok(w)) = (bound_buffer_for_target(c, readTarget), bound_buffer_for_target(c, writeTarget)) else {
        c.set_error(GL_INVALID_ENUM, "glCopyBufferSubData");
        return;
    };
    let (Some(ri), Some(wi)) = (r, w) else {
        c.set_error(GL_INVALID_OPERATION, "glCopyBufferSubData");
        return;
    };
    if !c.objs().buffers[ri].mapped.is_null() || !c.objs().buffers[wi].mapped.is_null() {
        c.set_error(GL_INVALID_OPERATION, "glCopyBufferSubData");
        return;
    }
    if readOffset < 0
        || writeOffset < 0
        || size < 0
        || readOffset.saturating_add(size) > c.objs().buffers[ri].size
        || writeOffset.saturating_add(size) > c.objs().buffers[wi].size
    {
        c.set_error(GL_INVALID_VALUE, "glCopyBufferSubData");
        return;
    }
    /* The same buffer may be both source and destination as long as the
     * ranges do not overlap (ES 3.0 2.9.5). */
    if ri == wi && readOffset < writeOffset + size && writeOffset < readOffset + size {
        c.set_error(GL_INVALID_VALUE, "glCopyBufferSubData");
        return;
    }
    let (src, dst) = (c.objs().buffers[ri].mem.cpu, c.objs().buffers[wi].mem.cpu);
    if size == 0 || src.is_null() || dst.is_null() {
        return;
    }
    // SAFETY: both ranges lie inside their allocation (checked above); they
    // are in different buffers unless they do not overlap.
    unsafe {
        let dst = (dst as *mut u8).offset(writeOffset);
        core::ptr::copy((src as *const u8).offset(readOffset), dst, size as usize);
        osal::cache_flush(dst as *mut c_void, size as usize);
    }
}

/* --------------------------------------------------------------------------- */
/* OpenGL ES 3.0 buffer mapping. The GPU allocation is the store, so a map
 * hands out its CPU view. A write into storage a pending kick still reads
 * gets fresh pages first (copy on write, as glBufferSubData), unless the
 * caller invalidates the whole buffer. A read needs no wait: the GPU never
 * writes a buffer, only the cache lines may be stale. */
#[no_mangle]
pub unsafe extern "C" fn glMapBufferRange(
    target: GLenum,
    offset: GLintptr,
    length: GLsizeiptr,
    access: GLbitfield,
) -> *mut c_void {
    gl_trace!(glMapBufferRange);
    let Some(c) = context::current() else { return core::ptr::null_mut() };
    map_range(c, target, offset, length, access, "glMapBufferRange")
}

/// glMapBufferRange's body, for it and glMapBufferOES: `func` names the entry
/// point in the error it sets.
unsafe fn map_range(
    c: &mut GlContext,
    target: GLenum,
    offset: GLintptr,
    length: GLsizeiptr,
    access: GLbitfield,
    func: &'static str,
) -> *mut c_void {
    let Ok(bound) = bound_buffer_for_target(c, target) else {
        c.set_error(GL_INVALID_ENUM, func);
        return core::ptr::null_mut();
    };
    let Some(i) = bound.filter(|&i| !c.objs().buffers[i].mem.impl_.is_null()) else {
        c.set_error(GL_INVALID_OPERATION, func);
        return core::ptr::null_mut();
    };
    /* ES 3.0 2.10.3. GL_INVALID_VALUE: a negative offset or length, a range
     * that leaves the buffer (checked without the sum, which can wrap),
     * access bits that are not defined. */
    if offset < 0 || length < 0 || offset > c.objs().buffers[i].size || length > c.objs().buffers[i].size - offset {
        c.set_error(GL_INVALID_VALUE, func);
        return core::ptr::null_mut();
    }
    const MAP_ACCESS_BITS: GLbitfield = GL_MAP_READ_BIT
        | GL_MAP_WRITE_BIT
        | GL_MAP_INVALIDATE_RANGE_BIT
        | GL_MAP_INVALIDATE_BUFFER_BIT
        | GL_MAP_FLUSH_EXPLICIT_BIT
        | GL_MAP_UNSYNCHRONIZED_BIT;
    if access & !MAP_ACCESS_BITS != 0 {
        c.set_error(GL_INVALID_VALUE, func);
        return core::ptr::null_mut();
    }
    /* GL_INVALID_OPERATION: zero length, an already mapped buffer, neither
     * READ nor WRITE, a read mapping with a write-only flag, FLUSH_EXPLICIT
     * without WRITE. */
    let bad_access = access & (GL_MAP_READ_BIT | GL_MAP_WRITE_BIT) == 0
        || (access & GL_MAP_READ_BIT != 0
            && access & (GL_MAP_INVALIDATE_RANGE_BIT | GL_MAP_INVALIDATE_BUFFER_BIT | GL_MAP_UNSYNCHRONIZED_BIT) != 0)
        || (access & GL_MAP_FLUSH_EXPLICIT_BIT != 0 && access & GL_MAP_WRITE_BIT == 0);
    if length == 0 || !c.objs().buffers[i].mapped.is_null() || bad_access {
        c.set_error(GL_INVALID_OPERATION, func);
        return core::ptr::null_mut();
    }
    if c.objs().buffers[i].mem.cpu.is_null() {
        /* no CPU view of GPU memory on this platform: nothing to hand out */
        c.set_error(GL_OUT_OF_MEMORY, func);
        return core::ptr::null_mut();
    }
    if access & GL_MAP_WRITE_BIT != 0 && backend::kick_pending(c.objs().buffers[i].last_use) {
        let mut fresh = OsalGpuMem::ZERO;
        let bytes = c.objs().buffers[i].size as usize;

        if osal::gpu_alloc(heap_data(), bytes, OSAL_MEM_DATA, &mut fresh) != 0 || fresh.cpu.is_null() {
            /* no CPU view of the fresh pages: they are of no use here */
            osal::gpu_free(&mut fresh);
            c.set_error(GL_OUT_OF_MEMORY, func);
            return core::ptr::null_mut();
        }
        let b = &mut c.objs().buffers[i];
        if access & GL_MAP_INVALIDATE_BUFFER_BIT == 0 {
            // SAFETY: both allocations are live, distinct, CPU mapped and hold at least `bytes`.
            osal::cache_invd(b.mem.cpu, bytes);
            core::ptr::copy_nonoverlapping(b.mem.cpu as *const u8, fresh.cpu as *mut u8, bytes);
            osal::cache_flush(fresh.cpu, bytes);
        } else {
            /* the contents are undefined after INVALIDATE_BUFFER, but they
             * must not be what the pool held before (see clear_storage) */
            let _ = clear_storage(&mut fresh);
        }
        osal::gpu_free(&mut b.mem); /* deferred until the kicks are done */
        b.mem = fresh;
        b.last_use = 0;
    }
    let b = &mut c.objs().buffers[i];
    // SAFETY: offset + length <= the buffer's size (checked above), inside the CPU mapping.
    let mapped = (b.mem.cpu as *mut u8).add(offset as usize) as *mut c_void;
    if access & GL_MAP_READ_BIT != 0 {
        osal::cache_invd(mapped, length as usize);
    }
    b.mapped = mapped;
    b.map_off = offset;
    b.map_len = length;
    b.map_access = access;
    b.mapped
}

/* ---- GL_OES_mapbuffer ------------------------------------------------------
 * The whole data store mapped write-only. The extension (OES_mapbuffer.txt)
 * refers to OpenGL 2.0 for how the calls work, with one departure: the access
 * must be GL_WRITE_ONLY_OES; and "any portion of a mapped buffer that is not
 * written will retain its original contents". ES 3.0's mapping above is that
 * behaviour with offset 0, the store's size and GL_MAP_WRITE_BIT alone (no
 * invalidation, so unwritten bytes keep their contents, copy on write
 * included): one mapping state, so glBufferData, glBufferSubData, the copies
 * and glUnmapBuffer treat an OES mapping as any other. The targets are the
 * ones the extension was written against (ES 1.1 / 2.0 vertex buffer objects:
 * GL_ARRAY_BUFFER and GL_ELEMENT_ARRAY_BUFFER); any other is GL_INVALID_ENUM.
 * A zero-sized store maps as ES 3.0's zero-length range does:
 * GL_INVALID_OPERATION and NULL (OpenGL 2.0 states no rule for it). */

fn oes_target(target: GLenum) -> bool {
    matches!(target, GL_ARRAY_BUFFER | GL_ELEMENT_ARRAY_BUFFER)
}

#[no_mangle]
pub unsafe extern "C" fn glMapBufferOES(target: GLenum, access: GLenum) -> *mut c_void {
    gl_trace!(glMapBufferOES);
    const FUNC: &str = "glMapBufferOES";
    let Some(c) = context::current() else { return core::ptr::null_mut() };
    if !oes_target(target) || access != GL_WRITE_ONLY_OES {
        c.set_error(GL_INVALID_ENUM, FUNC);
        return core::ptr::null_mut();
    }
    let size = match bound_buffer_for_target(c, target) {
        Ok(Some(i)) => c.objs().buffers[i].size,
        _ => 0, /* nothing bound: map_range sets GL_INVALID_OPERATION */
    };
    map_range(c, target, 0, size, GL_MAP_WRITE_BIT, FUNC)
}

#[no_mangle]
pub unsafe extern "C" fn glUnmapBufferOES(target: GLenum) -> GLboolean {
    gl_trace!(glUnmapBufferOES);
    let Some(c) = context::current() else { return GL_FALSE_B };
    if !oes_target(target) {
        c.set_error(GL_INVALID_ENUM, "glUnmapBufferOES");
        return GL_FALSE_B;
    }
    unmap_target(c, target, "glUnmapBufferOES")
}

#[no_mangle]
pub unsafe extern "C" fn glGetBufferPointervOES(target: GLenum, pname: GLenum, params: *mut *mut c_void) {
    gl_trace!(glGetBufferPointervOES);
    let Some(c) = context::current() else { return };
    if !oes_target(target) {
        c.set_error(GL_INVALID_ENUM, "glGetBufferPointervOES");
        return;
    }
    buffer_pointer(c, target, pname, params, "glGetBufferPointervOES");
}

#[no_mangle]
pub unsafe extern "C" fn glFlushMappedBufferRange(target: GLenum, offset: GLintptr, length: GLsizeiptr) {
    gl_trace!(glFlushMappedBufferRange);
    let Some(c) = context::current() else { return };
    let Ok(bound) = bound_buffer_for_target(c, target) else {
        c.set_error(GL_INVALID_ENUM, "glFlushMappedBufferRange");
        return;
    };
    let Some(i) = bound.filter(|&i| {
        !c.objs().buffers[i].mapped.is_null() && c.objs().buffers[i].map_access & GL_MAP_FLUSH_EXPLICIT_BIT != 0
    }) else {
        c.set_error(GL_INVALID_OPERATION, "glFlushMappedBufferRange");
        return;
    };
    if offset < 0 || length < 0 || offset > c.objs().buffers[i].map_len || length > c.objs().buffers[i].map_len - offset
    {
        c.set_error(GL_INVALID_VALUE, "glFlushMappedBufferRange");
        return;
    }
    if length != 0 {
        // SAFETY: offset + length <= the mapped length (checked above), inside the CPU mapping.
        osal::cache_flush((c.objs().buffers[i].mapped as *mut u8).add(offset as usize) as *mut c_void, length as usize);
    }
}

#[no_mangle]
pub unsafe extern "C" fn glUnmapBuffer(target: GLenum) -> GLboolean {
    gl_trace!(glUnmapBuffer);
    let Some(c) = context::current() else { return GL_FALSE_B };
    unmap_target(c, target, "glUnmapBuffer")
}

/// glUnmapBuffer's body, for it and glUnmapBufferOES.
fn unmap_target(c: &mut GlContext, target: GLenum, func: &'static str) -> GLboolean {
    let Ok(bound) = bound_buffer_for_target(c, target) else {
        c.set_error(GL_INVALID_ENUM, func);
        return GL_FALSE_B;
    };
    let Some(i) = bound.filter(|&i| !c.objs().buffers[i].mapped.is_null()) else {
        c.set_error(GL_INVALID_OPERATION, func);
        return GL_FALSE_B;
    };
    unmap_buffer(&mut c.objs().buffers[i]);
    GL_TRUE_B
}

/// Unit tests: forget the binding history and the out-of-names latch
/// (process-global; testutil::TestGl calls this when it is created and dropped).
#[cfg(test)]
pub(crate) fn reset_for_tests() {
    // SAFETY: tests holding a TestGl are serialised; every field is valid when all-zero.
    unsafe { core::ptr::write_bytes(STATE.as_ptr(), 0, 1) };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{CacheOp, MemEvent, TestGl, HEAP_DATA};
    use std::string::String;
    use std::vec::Vec;

    fn gen1() -> GLuint {
        let mut h = 0;
        // SAFETY: room for one name.
        unsafe { glGenBuffers(1, &mut h) };
        assert_ne!(h, 0);
        h
    }

    /// Generate a buffer, bind it to `target` and give it `data`.
    fn buffer_with(target: GLenum, data: &[u8]) -> GLuint {
        let h = gen1();
        // SAFETY: `data` is valid for its length.
        unsafe {
            glBindBuffer(target, h);
            glBufferData(target, data.len() as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        }
        h
    }

    fn sub_data(target: GLenum, offset: GLintptr, data: &[u8]) {
        // SAFETY: `data` is valid for its length.
        unsafe { glBufferSubData(target, offset, data.len() as GLsizeiptr, data.as_ptr() as *const c_void) };
    }

    fn param(target: GLenum, pname: GLenum) -> GLint {
        let mut v = -1;
        // SAFETY: room for one value.
        unsafe { glGetBufferParameteriv(target, pname, &mut v) };
        v
    }

    /// The buffer's storage as the "GPU" sees it.
    fn contents(gl: &TestGl, h: GLuint) -> Vec<u8> {
        let b = &gl.ctx().objs_ref().buffers[h as usize];
        gl.gpu_bytes(b.mem.gpu, b.size as usize)
    }

    /// Mark the buffer as read by kick `id`, still running.
    fn make_busy(gl: &TestGl, h: GLuint, id: u32) {
        gl.ctx().objs().buffers[h as usize].last_use = id;
        gl.set_kick_pending(id, true);
    }

    /* ---- helpers ------------------------------------------------------------- */

    /* ---- the ES 3.0 binding points, copies and pixel buffers ----------------- */

    #[test]
    fn every_es3_buffer_target_has_a_binding_point() {
        /* ES 3.0 has eight. Binding zero to GL_PIXEL_PACK_BUFFER, which a
         * conformance run does between cases, used to be an error. */
        let gl = TestGl::new(8, 8);
        let targets = [
            GL_ARRAY_BUFFER,
            GL_ELEMENT_ARRAY_BUFFER,
            GL_PIXEL_PACK_BUFFER,
            GL_PIXEL_UNPACK_BUFFER,
            GL_COPY_READ_BUFFER,
            GL_COPY_WRITE_BUFFER,
            GL_UNIFORM_BUFFER,
            GL_TRANSFORM_FEEDBACK_BUFFER,
        ];
        for t in targets {
            let h = buffer_with(t, &[0; 16]);
            assert_eq!(gl.error(), GL_NO_ERROR, "0x{t:x}");
            assert_eq!(binding_for_target(gl.ctx(), t).copied(), Some(h), "0x{t:x}");
            // SAFETY: plain GL call.
            unsafe { glBindBuffer(t, 0) };
            assert_eq!(gl.error(), GL_NO_ERROR, "unbinding 0x{t:x}");
        }
    }

    #[test]
    fn deleting_a_buffer_unbinds_it_from_every_binding_point() {
        /* A deleted name left bound to PIXEL_UNPACK_BUFFER made the next
         * pixel transfer resolve its offset against a buffer that was gone. */
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            for t in [GL_PIXEL_PACK_BUFFER, GL_PIXEL_UNPACK_BUFFER, GL_COPY_READ_BUFFER, GL_UNIFORM_BUFFER] {
                glBindBuffer(t, h);
            }
            glDeleteBuffers(1, &h);
        }
        let c = gl.ctx();
        for b in all_bindings(c) {
            assert_eq!(*b, 0);
        }
    }

    #[test]
    fn copy_buffer_sub_data_copies_between_two_buffers() {
        let gl = TestGl::new(8, 8);
        let src = buffer_with(GL_COPY_READ_BUFFER, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let dst = buffer_with(GL_COPY_WRITE_BUFFER, &[0; 8]);
        // SAFETY: plain GL call.
        unsafe { glCopyBufferSubData(GL_COPY_READ_BUFFER, GL_COPY_WRITE_BUFFER, 2, 4, 3) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(contents(&gl, dst), [0, 0, 0, 0, 3, 4, 5, 0]);
        assert_eq!(contents(&gl, src), [1, 2, 3, 4, 5, 6, 7, 8], "the source is untouched");
    }

    #[test]
    fn copy_buffer_sub_data_refuses_what_the_spec_refuses() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_COPY_READ_BUFFER, &[1, 2, 3, 4, 5, 6, 7, 8]);
        buffer_with(GL_COPY_WRITE_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls.
        unsafe {
            glCopyBufferSubData(GL_TEXTURE_2D, GL_COPY_WRITE_BUFFER, 0, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a buffer target");
            glCopyBufferSubData(GL_COPY_READ_BUFFER, GL_COPY_WRITE_BUFFER, -1, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "a negative offset");
            glCopyBufferSubData(GL_COPY_READ_BUFFER, GL_COPY_WRITE_BUFFER, 6, 0, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "past the end of the source");
            /* the same buffer, ranges that overlap (ES 3.0 2.9.5) */
            glCopyBufferSubData(GL_COPY_READ_BUFFER, GL_COPY_READ_BUFFER, 0, 2, 4);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glCopyBufferSubData(GL_COPY_READ_BUFFER, GL_COPY_READ_BUFFER, 0, 4, 4);
            assert_eq!(gl.error(), GL_NO_ERROR, "the same buffer, ranges that do not");
        }
    }

    #[test]
    fn get_buffer_pointerv_answers_the_map_pointer() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        let mut sentinel = 0u8;
        let mut p: *mut c_void = &mut sentinel as *mut u8 as *mut c_void;
        // SAFETY: room for one pointer.
        unsafe {
            glGetBufferPointerv(GL_ARRAY_BUFFER, GL_BUFFER_MAP_POINTER, &mut p);
            assert!(p.is_null(), "not mapped");
            let m = glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT);
            assert!(!m.is_null());
            glGetBufferPointerv(GL_ARRAY_BUFFER, GL_BUFFER_MAP_POINTER, &mut p);
            assert_eq!(p, m);
            glGetBufferPointerv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut p);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a pointer query");
            glUnmapBuffer(GL_ARRAY_BUFFER);
        }
        assert_eq!(gl.ctx().objs().buffers[h as usize].mapped, core::ptr::null_mut());
    }

    #[test]
    fn a_pixel_transfer_takes_its_pointer_as_an_offset_into_a_bound_buffer() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_PIXEL_UNPACK_BUFFER, &[0; 64]);
        let base = gl.ctx().objs().buffers[h as usize].mem.cpu;
        let c = gl.ctx();
        assert_eq!(
            pixel_buffer_resolve(c, false, 16 as *const c_void, 8, "test"),
            // SAFETY: inside the 64-byte allocation.
            Some(unsafe { (base as *const u8).add(16) } as *const c_void)
        );
        assert_eq!(gl.error(), GL_NO_ERROR);
        /* an offset past the end is caught here rather than walked off */
        assert_eq!(pixel_buffer_resolve(gl.ctx(), false, 60 as *const c_void, 8, "test"), None);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(pixel_buffer_resolve(gl.ctx(), false, 65 as *const c_void, 0, "test"), None);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        /* the pack binding is a different one, and it is unbound */
        let p = 123 as *const c_void;
        assert_eq!(pixel_buffer_resolve(gl.ctx(), true, p, 8, "test"), Some(p), "no buffer: a client address");
    }

    #[test]
    fn a_mapped_buffer_is_not_a_pixel_transfer_source() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_PIXEL_UNPACK_BUFFER, &[0; 64]);
        // SAFETY: plain GL call.
        assert!(!unsafe { glMapBufferRange(GL_PIXEL_UNPACK_BUFFER, 0, 8, GL_MAP_WRITE_BIT) }.is_null());
        assert_eq!(pixel_buffer_resolve(gl.ctx(), false, core::ptr::null(), 0, "test"), None);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        // SAFETY: plain GL call.
        unsafe { glUnmapBuffer(GL_PIXEL_UNPACK_BUFFER) };
    }

    #[test]
    fn binding_for_target_maps_the_two_buffer_targets() {
        let gl = TestGl::new(8, 8);
        let c = gl.ctx();
        c.bound_array_buffer = 11;
        c.bound_element_buffer = 22;
        assert_eq!(binding_for_target(c, GL_ARRAY_BUFFER).copied(), Some(11));
        assert_eq!(binding_for_target(c, GL_ELEMENT_ARRAY_BUFFER).copied(), Some(22));
        assert!(binding_for_target(c, GL_TEXTURE_2D).is_none());
        assert!(binding_for_target(c, 0).is_none());
        c.bound_array_buffer = 0;
        c.bound_element_buffer = 0;
    }

    #[test]
    fn bound_buffer_for_target_ignores_stale_names() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        let c = gl.ctx();
        assert_eq!(bound_buffer_for_target(c, GL_ARRAY_BUFFER), Ok(None), "nothing bound");
        c.bound_array_buffer = h;
        assert_eq!(bound_buffer_for_target(c, GL_ARRAY_BUFFER), Ok(Some(h as usize)));
        c.bound_array_buffer = h + 1; /* a name that is not a buffer */
        assert_eq!(bound_buffer_for_target(c, GL_ARRAY_BUFFER), Ok(None));
        assert_eq!(bound_buffer_for_target(c, GL_FRAMEBUFFER), Err(()));
        c.bound_array_buffer = 0;
    }

    /* ---- gen / bind / is / delete -------------------------------------------- */

    #[test]
    fn gen_returns_distinct_nonzero_names() {
        let gl = TestGl::new(8, 8);
        let mut names = [0; 5];
        // SAFETY: room for five names.
        unsafe { glGenBuffers(5, names.as_mut_ptr()) };
        for (i, a) in names.iter().enumerate() {
            assert_ne!(*a, 0);
            assert!(!names[..i].contains(a), "{names:?}");
        }
        let more = gen1();
        assert!(!names.contains(&more), "an unbound generated name stays reserved");
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.mem_events().is_empty(), "no storage before glBufferData");
    }

    #[test]
    fn gen_negative_count_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let mut name = 77;
        // SAFETY: nothing is written for n < 0.
        unsafe { glGenBuffers(-1, &mut name) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(name, 77);
    }

    #[test]
    fn gen_zero_names_or_null_pointer_is_a_no_op() {
        let gl = TestGl::new(8, 8);
        let mut name = 77;
        // SAFETY: nothing is written for n = 0; null is tolerated.
        unsafe {
            glGenBuffers(0, &mut name);
            glGenBuffers(3, core::ptr::null_mut());
        }
        assert_eq!(name, 77);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().objs().buffers.iter().all(|b| !b.used));
    }

    #[test]
    fn gen_runs_out_of_names_with_out_of_memory() {
        let gl = TestGl::new(8, 8);
        let mut names = std::vec![0 as GLuint; MAX_BUFFERS + 1];
        // SAFETY: room for every name asked for.
        unsafe { glGenBuffers(names.len() as GLsizei, names.as_mut_ptr()) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        /* name 0 is never handed out, so the table yields MAX_BUFFERS - 1 names */
        let (made, rest) = names.split_at(MAX_BUFFERS - 1);
        assert!(made.iter().all(|&h| h != 0 && (h as usize) < MAX_BUFFERS));
        assert_eq!(rest, [0, 0], "names that could not be made are 0");

        /* deleting one makes exactly one name available again */
        let victim = made[100];
        let mut again = [0; 2];
        // SAFETY: valid name arrays.
        unsafe {
            glDeleteBuffers(1, &victim);
            glGenBuffers(1, again.as_mut_ptr());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(again[0], victim);
        // SAFETY: room for one name.
        unsafe { glGenBuffers(1, &mut again[1]) };
        assert_eq!(again[1], 0);
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
    }

    #[test]
    fn is_buffer_is_false_until_the_name_is_first_bound() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsBuffer(h), GL_FALSE_B);
            glBindBuffer(GL_ARRAY_BUFFER, h);
            assert_eq!(glIsBuffer(h), GL_TRUE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn is_buffer_for_bound_deleted_zero_and_out_of_range_names() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            assert_eq!(glIsBuffer(h), GL_TRUE_B);
            assert_eq!(glIsBuffer(0), GL_FALSE_B);
            assert_eq!(glIsBuffer(h + 1), GL_FALSE_B);
            assert_eq!(glIsBuffer(MAX_BUFFERS as GLuint), GL_FALSE_B);
            assert_eq!(glIsBuffer(GLuint::MAX), GL_FALSE_B);
            glDeleteBuffers(1, &h);
            assert_eq!(glIsBuffer(h), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_records_the_binding_per_target() {
        let gl = TestGl::new(8, 8);
        let (a, e) = (gen1(), gen1());
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, a);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, e);
        }
        let c = gl.ctx();
        assert_eq!((c.bound_array_buffer, c.bound_element_buffer), (a, e));
        assert_eq!(
            (c.objs().buffers[a as usize].target, c.objs().buffers[e as usize].target),
            (GL_ARRAY_BUFFER, GL_ELEMENT_ARRAY_BUFFER)
        );
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_ARRAY_BUFFER, 0) };
        let c = gl.ctx();
        assert_eq!((c.bound_array_buffer, c.bound_element_buffer), (0, e));
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn bind_creates_a_name_that_was_not_generated() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glIsBuffer(40), GL_FALSE_B);
            glBindBuffer(GL_ARRAY_BUFFER, 40);
            assert_eq!(glIsBuffer(40), GL_TRUE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.ctx().bound_array_buffer, 40);
        assert_ne!(gen1(), 40, "glGenBuffers skips a name in use");
    }

    #[test]
    fn bind_invalid_target_is_invalid_enum() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_TEXTURE_2D, h) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        let c = gl.ctx();
        assert_eq!((c.bound_array_buffer, c.bound_element_buffer), (0, 0));
    }

    #[test]
    fn bind_name_beyond_the_table_is_a_far_name_in_its_own_slot() {
        /* ES 2.0 binds names nobody generated (gl/names.rs): the first name
         * past the pool borrows the top free slot, not the generated one */
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBindBuffer(GL_ARRAY_BUFFER, MAX_BUFFERS as GLuint);
            assert_eq!(gl.error(), GL_NO_ERROR);
            assert_eq!(glIsBuffer(MAX_BUFFERS as GLuint), GL_TRUE_B);
        }
        assert_eq!(gl.ctx().bound_array_buffer, (MAX_BUFFERS - 1) as GLuint);
        assert!(gl.ctx().objs().buffers[h as usize].used, "the generated buffer is untouched");
    }

    #[test]
    fn delete_frees_storage_unbinds_and_releases_the_name() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1, 2, 3, 4]);
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, h) };
        let gpu = gl.ctx().objs().buffers[h as usize].mem.gpu;
        let live = gl.live_allocs();
        gl.mem_events();
        // SAFETY: one valid name.
        unsafe { glDeleteBuffers(1, &h) };
        assert_eq!(gl.mem_events(), [MemEvent::Free { gpu }]);
        assert_eq!(gl.live_allocs(), live - 1);
        let c = gl.ctx();
        assert_eq!((c.bound_array_buffer, c.bound_element_buffer), (0, 0), "bindings revert to 0");
        assert!(!c.objs().buffers[h as usize].used);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gen1(), h, "the name is free again");
    }

    #[test]
    fn delete_leaves_other_bindings_alone() {
        let gl = TestGl::new(8, 8);
        let a = buffer_with(GL_ARRAY_BUFFER, &[1]);
        let e = buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[2]);
        // SAFETY: one valid name.
        unsafe { glDeleteBuffers(1, &a) };
        let c = gl.ctx();
        assert_eq!((c.bound_array_buffer, c.bound_element_buffer), (0, e));
        assert_eq!(contents(&gl, e), [2]);
    }

    #[test]
    fn delete_ignores_zero_unknown_and_repeated_names() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[9; 16]);
        let names = [0, h, h, 300, MAX_BUFFERS as GLuint, GLuint::MAX];
        gl.mem_events();
        // SAFETY: `names` holds the six names passed.
        unsafe { glDeleteBuffers(names.len() as GLsizei, names.as_ptr()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(gl.mem_events().len(), 1, "freed exactly once");
    }

    #[test]
    fn delete_negative_count_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: nothing is read for n < 0; null is tolerated.
        unsafe {
            glDeleteBuffers(-1, &h);
            assert_eq!(gl.error(), GL_INVALID_VALUE);
            glDeleteBuffers(1, core::ptr::null());
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().objs().buffers[h as usize].used);
    }

    #[test]
    fn delete_of_a_buffer_without_storage_frees_nothing() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: one valid name.
        unsafe { glDeleteBuffers(1, &h) };
        assert!(gl.mem_events().is_empty());
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- glBufferData -------------------------------------------------------- */

    #[test]
    fn buffer_data_allocates_in_the_data_heap_and_uploads() {
        let gl = TestGl::new(8, 8);
        let data: Vec<u8> = (0..23).collect();
        gl.mem_events();
        let h = buffer_with(GL_ARRAY_BUFFER, &data);
        let gpu = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_eq!(gl.mem_events(), [MemEvent::Alloc { heap: HEAP_DATA, size: 23, gpu }]);
        assert_eq!(gl.gpu_bytes(gpu, 23), data);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 23);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn buffer_data_through_the_element_target() {
        let gl = TestGl::new(8, 8);
        let idx: [u16; 3] = [0, 1, 0x0102];
        let bytes: Vec<u8> = idx.iter().flat_map(|i| i.to_ne_bytes()).collect();
        let h = buffer_with(GL_ELEMENT_ARRAY_BUFFER, &bytes);
        assert_eq!(contents(&gl, h), bytes);
        assert_eq!(gl.ctx().bound_array_buffer, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn buffer_data_null_allocates_without_uploading() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: null data is allowed.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBufferData(GL_ARRAY_BUFFER, 64, core::ptr::null(), GL_DYNAMIC_DRAW);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let b = &gl.ctx().objs_ref().buffers[h as usize];
        assert_eq!((b.size, b.mem.size), (64, 64));
        assert_ne!(b.mem.gpu, 0);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 64);
    }

    #[test]
    fn buffer_data_null_keeps_the_old_bytes_when_the_storage_is_reused() {
        /* orphaning is only done when a kick still reads the storage */
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[5; 8]);
        gl.mem_events();
        // SAFETY: null data is allowed.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 8, core::ptr::null(), GL_STATIC_DRAW) };
        assert!(gl.mem_events().is_empty());
        assert_eq!(contents(&gl, h), [5; 8]);
    }

    #[test]
    fn buffer_data_size_zero_has_no_storage() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        gl.mem_events();
        // SAFETY: nothing is read for size 0.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBufferData(GL_ARRAY_BUFFER, 0, [1u8].as_ptr() as *const c_void, GL_STATIC_DRAW);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.mem_events().is_empty());
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 0);
        assert!(gl.ctx().objs().buffers[h as usize].mem.impl_.is_null());
    }

    #[test]
    fn buffer_data_usage_is_recorded_and_queried() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_ARRAY_BUFFER, h) };
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_USAGE), GL_STATIC_DRAW as GLint, "the initial usage");
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 0);
        for u in [GL_STREAM_DRAW, GL_DYNAMIC_DRAW, GL_STATIC_DRAW] {
            // SAFETY: null data is allowed.
            unsafe { glBufferData(GL_ARRAY_BUFFER, 4, core::ptr::null(), u) };
            assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_USAGE), u as GLint);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn buffer_data_invalid_usage_is_invalid_enum_and_keeps_the_store() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1, 2, 3]);
        // SAFETY: valid data pointer.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 1, [9u8].as_ptr() as *const c_void, GL_ARRAY_BUFFER) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(contents(&gl, h), [1, 2, 3]);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_USAGE), GL_STATIC_DRAW as GLint);
    }

    #[test]
    fn buffer_data_accepts_the_es3_usages() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_ARRAY_BUFFER, h) };
        for u in [GL_STREAM_READ, GL_STREAM_COPY, GL_STATIC_READ, GL_STATIC_COPY, GL_DYNAMIC_READ, GL_DYNAMIC_COPY] {
            // SAFETY: null data is allowed.
            unsafe { glBufferData(GL_ARRAY_BUFFER, 4, core::ptr::null(), u) };
            assert_eq!(gl.error(), GL_NO_ERROR, "usage 0x{u:x}");
            assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_USAGE), u as GLint);
        }
    }

    #[test]
    fn buffer_data_argument_errors() {
        let gl = TestGl::new(8, 8);
        // SAFETY: null data is allowed.
        unsafe {
            glBufferData(GL_ARRAY_BUFFER, 4, core::ptr::null(), GL_STATIC_DRAW);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "no buffer bound");
            glBufferData(GL_TEXTURE_2D, 4, core::ptr::null(), GL_STATIC_DRAW);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "not a buffer target");
        }
        let h = buffer_with(GL_ARRAY_BUFFER, &[7; 4]);
        // SAFETY: null data is allowed.
        unsafe { glBufferData(GL_ARRAY_BUFFER, -1, core::ptr::null(), GL_STATIC_DRAW) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
        assert_eq!(contents(&gl, h), [7; 4]);
    }

    #[test]
    fn buffer_data_smaller_reuses_the_allocation() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 32]);
        let gpu = gl.ctx().objs().buffers[h as usize].mem.gpu;
        gl.mem_events();
        // SAFETY: valid data pointer.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 4, [2u8; 4].as_ptr() as *const c_void, GL_STATIC_DRAW) };
        assert!(gl.mem_events().is_empty());
        assert_eq!(gl.ctx().objs().buffers[h as usize].mem.gpu, gpu);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 4, "the GL size shrinks all the same");
        assert_eq!(contents(&gl, h), [2; 4]);
        /* and the GL size, not the allocation, bounds glBufferSubData */
        sub_data(GL_ARRAY_BUFFER, 4, &[3]);
        assert_eq!(gl.error(), GL_INVALID_VALUE);
    }

    #[test]
    fn buffer_data_larger_frees_and_reallocates() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 8]);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        gl.mem_events();
        let data: Vec<u8> = (100..150).collect();
        // SAFETY: `data` holds the 50 bytes named.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 50, data.as_ptr() as *const c_void, GL_STATIC_DRAW) };
        let new = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_eq!(
            gl.mem_events(),
            [MemEvent::Free { gpu: old }, MemEvent::Alloc { heap: HEAP_DATA, size: 50, gpu: new }]
        );
        assert_eq!(contents(&gl, h), data);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn buffer_data_gets_fresh_pages_while_a_kick_still_reads_the_old_ones() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 8]);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        make_busy(&gl, h, 7);
        gl.mem_events();
        // SAFETY: valid data pointer.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 8, [2u8; 8].as_ptr() as *const c_void, GL_STATIC_DRAW) };
        let new = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_ne!(new, old, "same size, but the in-flight frame keeps its bytes");
        assert_eq!(
            gl.mem_events(),
            [MemEvent::Free { gpu: old }, MemEvent::Alloc { heap: HEAP_DATA, size: 8, gpu: new }]
        );
        assert_eq!(contents(&gl, h), [2; 8]);
    }

    #[test]
    fn buffer_data_reuses_storage_once_the_kick_is_done() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 8]);
        make_busy(&gl, h, 7);
        gl.set_kick_pending(7, false);
        gl.mem_events();
        // SAFETY: valid data pointer.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 8, [2u8; 8].as_ptr() as *const c_void, GL_STATIC_DRAW) };
        assert!(gl.mem_events().is_empty());
        assert_eq!(contents(&gl, h), [2; 8]);
    }

    #[test]
    fn buffer_data_allocation_failure_is_out_of_memory() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        gl.fail_alloc_in(1);
        // SAFETY: valid data pointer.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBufferData(GL_ARRAY_BUFFER, 8, [1u8; 8].as_ptr() as *const c_void, GL_STATIC_DRAW);
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_SIZE), 0);
        assert!(gl.ctx().objs().buffers[h as usize].mem.impl_.is_null());
        /* the buffer is still usable afterwards */
        // SAFETY: valid data pointer.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 2, [3u8; 2].as_ptr() as *const c_void, GL_STATIC_DRAW) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(contents(&gl, h), [3, 3]);
    }

    #[test]
    fn a_failed_regrow_leaves_an_empty_buffer_not_a_dangling_one() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 8]);
        let live = gl.live_allocs();
        gl.fail_alloc_in(1);
        // SAFETY: null data is allowed.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 4096, core::ptr::null(), GL_STATIC_DRAW) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), live - 1);
        let b = &gl.ctx().objs_ref().buffers[h as usize];
        assert_eq!((b.size, b.mem.gpu, b.mem.size), (0, 0, 0));
        sub_data(GL_ARRAY_BUFFER, 0, &[1]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    /* ---- glBufferSubData ----------------------------------------------------- */

    #[test]
    fn sub_data_updates_only_its_range() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0, 1, 2, 3, 4, 5, 6, 7]);
        gl.mem_events();
        sub_data(GL_ARRAY_BUFFER, 3, &[0xA0, 0xA1]);
        assert_eq!(contents(&gl, h), [0, 1, 2, 0xA0, 0xA1, 5, 6, 7]);
        sub_data(GL_ARRAY_BUFFER, 0, &[0xB0]);
        sub_data(GL_ARRAY_BUFFER, 7, &[0xB7]);
        assert_eq!(contents(&gl, h), [0xB0, 1, 2, 0xA0, 0xA1, 5, 6, 0xB7]);
        assert!(gl.mem_events().is_empty(), "updated in place");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn sub_data_range_errors_are_invalid_value() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[7; 8]);
        let one = [1u8; 16];
        let p = one.as_ptr() as *const c_void;
        for (off, size) in [(-1, 1), (0, -1), (8, 1), (0, 9), (5, 4), (9, 0)] {
            // SAFETY: `one` holds every size passed.
            unsafe { glBufferSubData(GL_ARRAY_BUFFER, off, size, p) };
            assert_eq!(gl.error(), GL_INVALID_VALUE, "offset {off} size {size}");
        }
        assert_eq!(contents(&gl, h), [7; 8]);
    }

    #[test]
    /* a BUG test until upstream fixed it (7e76640) */
    fn sub_data_offset_plus_size_overflow_is_invalid_value() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[7; 8]);
        /* null data: the call must fail on the range alone, and cannot write anywhere if it does not */
        // SAFETY: null data is never dereferenced.
        unsafe { glBufferSubData(GL_ARRAY_BUFFER, GLintptr::MAX, 1, core::ptr::null()) };
        assert_eq!(gl.error(), GL_INVALID_VALUE);
    }

    #[test]
    fn sub_data_of_the_whole_buffer_and_of_nothing() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[7; 4]);
        sub_data(GL_ARRAY_BUFFER, 0, &[1, 2, 3, 4]);
        sub_data(GL_ARRAY_BUFFER, 4, &[]); /* zero bytes at the end are in range */
        // SAFETY: null data is tolerated.
        unsafe { glBufferSubData(GL_ARRAY_BUFFER, 0, 4, core::ptr::null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(contents(&gl, h), [1, 2, 3, 4]);
    }

    #[test]
    fn sub_data_target_and_binding_errors() {
        let gl = TestGl::new(8, 8);
        sub_data(GL_TEXTURE_2D, 0, &[1]);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        sub_data(GL_ARRAY_BUFFER, 0, &[1]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "nothing bound");
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_ARRAY_BUFFER, h) };
        sub_data(GL_ARRAY_BUFFER, 0, &[1]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "no data store yet");
    }

    #[test]
    fn sub_data_copies_on_write_while_a_kick_reads_the_buffer() {
        let gl = TestGl::new(8, 8);
        let data: Vec<u8> = (0..600).map(|i| (i % 251) as u8).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &data);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        make_busy(&gl, h, 3);
        gl.mem_events();

        sub_data(GL_ARRAY_BUFFER, 10, &[0xEE; 5]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        let new = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_ne!(new, old);
        assert_eq!(
            gl.mem_events(),
            [MemEvent::Alloc { heap: HEAP_DATA, size: 600, gpu: new }, MemEvent::Free { gpu: old }]
        );
        let mut want = data.clone();
        want[10..15].fill(0xEE);
        /* the CPU view: the carried bytes reach the GPU only with a flush,
         * see sub_data_copy_on_write_flushes_the_carried_bytes */
        assert_eq!(gl.cpu_view(new, 600), want, "the untouched bytes moved with the buffer");
        assert_eq!(gl.gpu_bytes(new + 10, 5), [0xEE; 5]);
        assert_eq!(gl.ctx().objs().buffers[h as usize].last_use, 0, "the fresh pages are not in use");

        /* the next update goes in place */
        sub_data(GL_ARRAY_BUFFER, 0, &[1]);
        assert!(gl.mem_events().is_empty());
    }

    #[test]
    fn sub_data_copy_on_write_flushes_the_carried_bytes() {
        let gl = TestGl::new(8, 8);
        let data: Vec<u8> = (0..600).map(|i| (i % 251) as u8).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &data);
        make_busy(&gl, h, 3);
        sub_data(GL_ARRAY_BUFFER, 10, &[0xEE; 5]);
        let mut want = data.clone();
        want[10..15].fill(0xEE);
        assert_eq!(contents(&gl, h), want, "what the GPU reads");
        let new = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_eq!(gl.unflushed(new, 600), None);
    }

    #[test]
    fn sub_data_copy_on_write_allocation_failure_is_out_of_memory() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[4; 8]);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        make_busy(&gl, h, 3);
        gl.fail_alloc_in(1);
        sub_data(GL_ARRAY_BUFFER, 0, &[9; 8]);
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.ctx().objs().buffers[h as usize].mem.gpu, old);
        assert_eq!(contents(&gl, h), [4; 8], "the frame in flight still sees its data");
    }

    #[test]
    fn a_draw_stamps_the_vertex_buffer_with_the_current_kick() {
        let gl = TestGl::new(16, 16);
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        let verts: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let bytes: Vec<u8> = verts.iter().flat_map(|f| f.to_ne_bytes()).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &bytes);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        // SAFETY: plain GL calls; the attribute reads the bound buffer.
        unsafe {
            use crate::gl::{draw::*, program::*, state::*, vertex::*};
            glFlush(); /* the mock's kick ids start at 0 = "never used" */
            glUseProgram(prog);
            for i in 0..2 {
                /* both VS inputs read the VBO, so nothing is staged in draw.rs' scratch */
                glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, core::ptr::null());
                glEnableVertexAttribArray(i);
            }
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        let draws = gl.draws();
        assert_eq!(draws.len(), 1);
        assert_eq!(draws[0].draw.attribs[0].buffer_gpu, old, "fetched in place");
        assert_eq!(gl.ctx().objs().buffers[h as usize].last_use, 1);

        /* while that kick runs, an update must not touch the bytes it reads */
        gl.set_kick_pending(1, true);
        sub_data(GL_ARRAY_BUFFER, 0, &[0; 4]);
        assert_ne!(gl.ctx().objs().buffers[h as usize].mem.gpu, old);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- glGetBufferParameteriv ---------------------------------------------- */

    #[test]
    fn get_buffer_parameter_errors() {
        let gl = TestGl::new(8, 8);
        assert_eq!(param(GL_TEXTURE_2D, GL_BUFFER_SIZE), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert_eq!(param(GL_ELEMENT_ARRAY_BUFFER, GL_BUFFER_SIZE), -1);
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "nothing bound");
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[1, 2]);
        assert_eq!(param(GL_ELEMENT_ARRAY_BUFFER, GL_BUFFER_SIZE), 2);
        assert_eq!(param(GL_ELEMENT_ARRAY_BUFFER, GL_VIEWPORT), -1);
        assert_eq!(gl.error(), GL_INVALID_ENUM, "unknown pname");
        // SAFETY: null is tolerated.
        unsafe { glGetBufferParameteriv(GL_ELEMENT_ARRAY_BUFFER, GL_BUFFER_SIZE, core::ptr::null_mut()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn get_buffer_parameter_reports_the_mapping() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_MAPPED), 0);
        // SAFETY: plain GL calls.
        unsafe { glMapBufferRange(GL_ARRAY_BUFFER, 4, 8, GL_MAP_WRITE_BIT) };
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_MAPPED), 1);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_ACCESS_FLAGS), GL_MAP_WRITE_BIT as GLint);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_MAP_OFFSET), 4);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_MAP_LENGTH), 8);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- mapping ------------------------------------------------------------- */

    #[test]
    fn map_write_unmap_writes_through_to_the_store() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0, 1, 2, 3, 4, 5, 6, 7]);
        // SAFETY: the mapping is valid for the 4 bytes asked for until glUnmapBuffer.
        unsafe {
            let p = glMapBufferRange(GL_ARRAY_BUFFER, 2, 4, GL_MAP_WRITE_BIT) as *mut u8;
            assert!(!p.is_null());
            core::slice::from_raw_parts_mut(p, 4).copy_from_slice(&[0xC2, 0xC3, 0xC4, 0xC5]);
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
        assert_eq!(contents(&gl, h), [0, 1, 0xC2, 0xC3, 0xC4, 0xC5, 6, 7]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn map_read_sees_the_buffer_contents_at_the_offset() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[10, 11, 12, 13, 14]);
        // SAFETY: the mapping is valid for the 3 bytes asked for until glUnmapBuffer.
        unsafe {
            let p = glMapBufferRange(GL_ELEMENT_ARRAY_BUFFER, 1, 3, GL_MAP_READ_BIT) as *const u8;
            assert!(!p.is_null());
            assert_eq!(core::slice::from_raw_parts(p, 3), [11, 12, 13]);
            assert_eq!(glUnmapBuffer(GL_ELEMENT_ARRAY_BUFFER), GL_TRUE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn map_records_and_unmap_clears_the_mapping_state() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        let access = GL_MAP_WRITE_BIT | GL_MAP_INVALIDATE_RANGE_BIT;
        // SAFETY: plain GL calls.
        let p = unsafe { glMapBufferRange(GL_ARRAY_BUFFER, 4, 12, access) };
        let b = &gl.ctx().objs_ref().buffers[h as usize];
        assert_eq!(b.mapped, p);
        assert_eq!(p as usize, b.mem.cpu as usize + 4);
        assert_eq!((b.map_off, b.map_len, b.map_access), (4, 12, access));
        // SAFETY: plain GL calls.
        unsafe { glUnmapBuffer(GL_ARRAY_BUFFER) };
        let b = &gl.ctx().objs_ref().buffers[h as usize];
        assert!(b.mapped.is_null());
        assert_eq!((b.map_off, b.map_len, b.map_access), (0, 0, 0));
    }

    #[test]
    fn map_range_errors_are_invalid_value() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        for (off, len) in [(-1, 4), (0, -1), (0, 9), (8, 1), (5, 4)] {
            // SAFETY: plain GL calls.
            let p = unsafe { glMapBufferRange(GL_ARRAY_BUFFER, off, len, GL_MAP_READ_BIT) };
            assert!(p.is_null(), "offset {off} length {len}");
            assert_eq!(gl.error(), GL_INVALID_VALUE, "offset {off} length {len}");
        }
        assert!(gl.ctx().objs().buffers[h as usize].mapped.is_null());
    }

    #[test]
    fn map_of_zero_length_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls.
        unsafe { assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 0, GL_MAP_READ_BIT).is_null()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn map_access_flag_combinations_are_validated() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        for access in [
            0,
            GL_MAP_INVALIDATE_RANGE_BIT,
            GL_MAP_READ_BIT | GL_MAP_INVALIDATE_RANGE_BIT,
            GL_MAP_READ_BIT | GL_MAP_INVALIDATE_BUFFER_BIT,
            GL_MAP_READ_BIT | GL_MAP_UNSYNCHRONIZED_BIT,
            GL_MAP_READ_BIT | GL_MAP_FLUSH_EXPLICIT_BIT,
        ] {
            // SAFETY: plain GL calls.
            unsafe {
                let p = glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, access);
                assert_eq!(gl.error(), GL_INVALID_OPERATION, "access 0x{access:x}");
                assert!(p.is_null(), "access 0x{access:x}");
            }
        }
        // SAFETY: plain GL calls.
        unsafe { assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT | 0x8000).is_null()) };
        assert_eq!(gl.error(), GL_INVALID_VALUE, "unknown access bits");
    }

    #[test]
    fn map_without_read_or_write_fails() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls.
        unsafe { assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_UNSYNCHRONIZED_BIT).is_null()) };
        assert_ne!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().objs().buffers[h as usize].mapped.is_null());
    }

    #[test]
    fn map_target_binding_and_state_errors() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            assert!(glMapBufferRange(GL_TEXTURE_2D, 0, 1, GL_MAP_READ_BIT).is_null());
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 1, GL_MAP_READ_BIT).is_null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "nothing bound");
        }
        buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls.
        unsafe {
            assert!(!glMapBufferRange(GL_ARRAY_BUFFER, 0, 4, GL_MAP_READ_BIT).is_null());
            assert!(glMapBufferRange(GL_ARRAY_BUFFER, 4, 4, GL_MAP_READ_BIT).is_null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "already mapped");
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_TRUE_B);
            assert!(!glMapBufferRange(GL_ARRAY_BUFFER, 4, 4, GL_MAP_READ_BIT).is_null(), "mappable again");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn map_for_write_copies_on_write_while_a_kick_reads_the_buffer() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        make_busy(&gl, h, 5);
        gl.mem_events();
        // SAFETY: the mapping is valid for the 2 bytes asked for until glUnmapBuffer.
        unsafe {
            let p = glMapBufferRange(GL_ARRAY_BUFFER, 6, 2, GL_MAP_WRITE_BIT) as *mut u8;
            assert!(!p.is_null());
            *p = 0x77;
            glUnmapBuffer(GL_ARRAY_BUFFER);
        }
        let new = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_ne!(new, old);
        assert_eq!(
            gl.mem_events(),
            [MemEvent::Alloc { heap: HEAP_DATA, size: 8, gpu: new }, MemEvent::Free { gpu: old }]
        );
        assert_eq!(contents(&gl, h), [1, 2, 3, 4, 5, 6, 0x77, 8]);
        assert_eq!(gl.ctx().objs().buffers[h as usize].last_use, 0);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn map_with_invalidate_buffer_skips_the_copy() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[9; 8]);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        make_busy(&gl, h, 5);
        // SAFETY: plain GL calls.
        unsafe {
            assert!(!glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT | GL_MAP_INVALIDATE_BUFFER_BIT).is_null());
            glUnmapBuffer(GL_ARRAY_BUFFER);
        }
        assert_ne!(gl.ctx().objs().buffers[h as usize].mem.gpu, old);
        /* the contents are undefined: only that nothing was copied is checked */
        assert!(!gl.cache_ops().iter().any(|o| matches!(*o, CacheOp::Invd { gpu, .. } if gpu == old)));
        assert_ne!(contents(&gl, h), [9; 8], "old bytes not carried over");
    }

    #[test]
    fn map_for_read_never_moves_a_busy_buffer() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[9; 8]);
        make_busy(&gl, h, 5);
        gl.mem_events();
        // SAFETY: plain GL calls.
        unsafe {
            assert!(!glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_READ_BIT).is_null());
            glUnmapBuffer(GL_ARRAY_BUFFER);
        }
        assert!(gl.mem_events().is_empty());
        assert_eq!(gl.ctx().objs().buffers[h as usize].last_use, 5);
    }

    #[test]
    fn map_copy_on_write_allocation_failure_is_out_of_memory() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[9; 8]);
        make_busy(&gl, h, 5);
        gl.fail_alloc_in(1);
        // SAFETY: plain GL calls.
        unsafe { assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT).is_null()) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert!(gl.ctx().objs().buffers[h as usize].mapped.is_null());
        assert_eq!(contents(&gl, h), [9; 8]);
    }

    #[test]
    fn flush_mapped_range_needs_an_explicit_flush_mapping() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        // SAFETY: plain GL calls.
        unsafe {
            glFlushMappedBufferRange(GL_TEXTURE_2D, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            glFlushMappedBufferRange(GL_ARRAY_BUFFER, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not mapped");
            glMapBufferRange(GL_ARRAY_BUFFER, 0, 16, GL_MAP_WRITE_BIT);
            glFlushMappedBufferRange(GL_ARRAY_BUFFER, 0, 1);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "mapped without FLUSH_EXPLICIT");
        }
    }

    #[test]
    fn flush_mapped_range_is_relative_to_the_mapping() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        // SAFETY: plain GL calls.
        unsafe {
            glMapBufferRange(GL_ARRAY_BUFFER, 8, 8, GL_MAP_WRITE_BIT | GL_MAP_FLUSH_EXPLICIT_BIT);
            glFlushMappedBufferRange(GL_ARRAY_BUFFER, 0, 8);
            glFlushMappedBufferRange(GL_ARRAY_BUFFER, 7, 1);
            glFlushMappedBufferRange(GL_ARRAY_BUFFER, 8, 0);
            assert_eq!(gl.error(), GL_NO_ERROR);
            for (off, len) in [(-1, 1), (0, -1), (0, 9), (8, 1), (4, 5)] {
                glFlushMappedBufferRange(GL_ARRAY_BUFFER, off, len);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "offset {off} length {len}");
            }
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
    }

    #[test]
    fn unmap_errors() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glUnmapBuffer(GL_TEXTURE_2D), GL_FALSE_B);
            assert_eq!(gl.error(), GL_INVALID_ENUM);
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_FALSE_B);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "nothing bound");
        }
        buffer_with(GL_ARRAY_BUFFER, &[0; 4]);
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_FALSE_B);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not mapped");
            glMapBufferRange(GL_ARRAY_BUFFER, 0, 4, GL_MAP_READ_BIT);
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_TRUE_B);
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_FALSE_B);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "unmapped twice");
        }
    }

    #[test]
    fn buffer_data_unmaps_a_mapped_buffer() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls; the stale mapping is never dereferenced.
        unsafe {
            glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT);
            glBufferData(GL_ARRAY_BUFFER, 4096, core::ptr::null(), GL_STATIC_DRAW);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert!(gl.ctx().objs().buffers[h as usize].mapped.is_null());
        // SAFETY: plain GL calls.
        unsafe { assert!(!glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT).is_null()) };
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn sub_data_on_a_mapped_buffer_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls.
        unsafe { glMapBufferRange(GL_ARRAY_BUFFER, 0, 4, GL_MAP_READ_BIT) };
        sub_data(GL_ARRAY_BUFFER, 4, &[1; 4]);
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
        assert_eq!(contents(&gl, h), [0; 8]);
    }

    #[test]
    fn delete_of_a_mapped_buffer_releases_it() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        let live = gl.live_allocs();
        // SAFETY: plain GL calls; the mapping is not used after the delete.
        unsafe {
            glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT);
            glDeleteBuffers(1, &h);
        }
        assert_eq!(gl.live_allocs(), live - 1);
        assert!(gl.ctx().objs().buffers[h as usize].mapped.is_null(), "the slot is reset for the next owner");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    /* ---- hostile offsets, unaligned pointers (fix round 2) ------------------- */

    #[test]
    fn sub_data_map_and_flush_offsets_that_would_wrap_are_refused() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 32]);
        let huge = [GLintptr::MAX, GLintptr::MAX - 8, -1, GLintptr::MIN];
        for off in huge {
            sub_data(GL_ARRAY_BUFFER, off, &[9; 8]);
            assert_eq!(gl.error(), GL_INVALID_VALUE, "sub_data offset {off}");
            // SAFETY: plain GL calls; nothing is read for a refused range.
            unsafe {
                glBufferSubData(GL_ARRAY_BUFFER, 8, off, [0u8; 8].as_ptr() as *const c_void);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "sub_data size {off}");
                assert!(glMapBufferRange(GL_ARRAY_BUFFER, off, 8, GL_MAP_READ_BIT).is_null());
                assert_eq!(gl.error(), GL_INVALID_VALUE, "map offset {off}");
                assert!(glMapBufferRange(GL_ARRAY_BUFFER, 8, off, GL_MAP_READ_BIT).is_null());
                assert_ne!(gl.error(), GL_NO_ERROR, "map length {off}");
            }
        }
        // SAFETY: plain GL calls.
        unsafe { glMapBufferRange(GL_ARRAY_BUFFER, 0, 32, GL_MAP_WRITE_BIT | GL_MAP_FLUSH_EXPLICIT_BIT) };
        for off in huge {
            // SAFETY: plain GL calls.
            unsafe {
                glFlushMappedBufferRange(GL_ARRAY_BUFFER, off, 8);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "flush offset {off}");
                glFlushMappedBufferRange(GL_ARRAY_BUFFER, 8, off);
                assert_eq!(gl.error(), GL_INVALID_VALUE, "flush length {off}");
            }
        }
        // SAFETY: plain GL call.
        unsafe { glUnmapBuffer(GL_ARRAY_BUFFER) };
        assert_eq!(contents(&gl, h), [1; 32], "nothing was written");
        assert!(gl.take_faults().is_empty());
    }

    #[test]
    fn names_and_parameters_take_misaligned_pointers() {
        let gl = TestGl::new(8, 8);
        let mut buf = [0u8; 1 + 2 * 4];
        let p = buf[1..].as_mut_ptr() as *mut GLuint;
        // SAFETY: `p` has room for two names (unaligned).
        unsafe { glGenBuffers(2, p) };
        let a = u32::from_ne_bytes(buf[1..5].try_into().unwrap());
        let b = u32::from_ne_bytes(buf[5..9].try_into().unwrap());
        assert!(a != 0 && b != 0 && a != b);
        // SAFETY: as above.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, a);
            glBufferData(GL_ARRAY_BUFFER, 8, core::ptr::null(), GL_STATIC_DRAW);
            glGetBufferParameteriv(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, buf[1..].as_mut_ptr() as *mut GLint);
        }
        assert_eq!(i32::from_ne_bytes(buf[1..5].try_into().unwrap()), 8);
        // SAFETY: `p` holds the two names again.
        unsafe {
            buf[1..5].copy_from_slice(&a.to_ne_bytes());
            buf[5..9].copy_from_slice(&b.to_ne_bytes());
            glDeleteBuffers(2, p as *const GLuint);
            assert_eq!(glIsBuffer(a), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn buffer_data_reports_a_failing_clear_of_fresh_storage() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let h = gen1();
        gl.fail_gpu_write_in(1);
        // SAFETY: null data is allowed.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBufferData(GL_ARRAY_BUFFER, 16, core::ptr::null(), GL_STATIC_DRAW);
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        let b = &gl.ctx().objs_ref().buffers[h as usize];
        assert!(b.mem.impl_.is_null() && b.size == 0, "no store rather than one full of pool leftovers");
    }

    #[test]
    fn a_map_with_invalidate_buffer_hands_out_cleared_pages() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[9; 8]);
        make_busy(&gl, h, 5);
        // SAFETY: the mapping is valid for the 8 bytes asked for.
        let m = unsafe {
            let p = glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT | GL_MAP_INVALIDATE_BUFFER_BIT);
            assert!(!p.is_null());
            core::slice::from_raw_parts(p as *const u8, 8)
        };
        assert_eq!(m, [0; 8], "no pool leftovers");
        // SAFETY: plain GL call.
        unsafe { glUnmapBuffer(GL_ARRAY_BUFFER) };
    }

    /* ---- the binding history (bring-up debug) -------------------------------- */

    fn dumped_history(gl: &TestGl) -> String {
        gl.logs();
        gl_debug_dump_bind_hist();
        let lines = gl.logs();
        assert!(lines.iter().all(|l| l.starts_with("  hist:")), "{lines:?}");
        lines.iter().map(|l| l.trim_start_matches("  hist:")).collect()
    }

    #[test]
    fn bind_history_records_binds_and_deletes_in_order() {
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: plain GL calls.
        unsafe {
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, h);
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glDeleteBuffers(1, &h);
        }
        gl_debug_bind_hist(3, 2);
        gl_debug_bind_hist(9, 1); /* an unknown op prints as "?" */
        let hist = dumped_history(&gl);
        let want = std::format!(" bind-elem:{h} bind-array:{h} delete:{h} bind-vao:2 ?:1");
        assert!(hist.ends_with(&want), "{hist}");
    }

    #[test]
    fn bind_history_keeps_the_last_48_events() {
        let gl = TestGl::new(8, 8);
        for i in 0..60 {
            gl_debug_bind_hist(4, 1000 + i);
        }
        let hist = dumped_history(&gl);
        let entries: Vec<&str> = hist.split_whitespace().collect();
        assert_eq!(entries.len(), BIND_HIST_LEN);
        assert_eq!(entries[0], "bind-array:1012", "the oldest 12 were overwritten");
        assert_eq!(entries[BIND_HIST_LEN - 1], "bind-array:1059");
    }

    #[test]
    fn an_invalid_bind_target_is_not_recorded() {
        let gl = TestGl::new(8, 8);
        gl_debug_bind_hist(4, 4242);
        // SAFETY: plain GL calls.
        unsafe { glBindBuffer(GL_TEXTURE_2D, 17) };
        assert_eq!(gl.error(), GL_INVALID_ENUM);
        assert!(dumped_history(&gl).ends_with(" bind-array:4242"));
    }

    /* ---- cache maintenance: what the GPU reads is flushed, what the CPU reads is invalidated ---- */

    fn gpu_of(gl: &TestGl, h: GLuint) -> u64 {
        gl.ctx().objs().buffers[h as usize].mem.gpu
    }

    /// Map `[offset, offset + length)` of the buffer bound to GL_ARRAY_BUFFER.
    fn map(offset: GLintptr, length: GLsizeiptr, access: GLbitfield) -> &'static mut [u8] {
        // SAFETY: plain GL call; a successful map hands out `length` bytes.
        unsafe {
            let p = glMapBufferRange(GL_ARRAY_BUFFER, offset, length, access);
            assert!(!p.is_null(), "map {offset} {length} 0x{access:x}");
            core::slice::from_raw_parts_mut(p as *mut u8, length as usize)
        }
    }

    fn unmap() {
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glUnmapBuffer(GL_ARRAY_BUFFER) }, GL_TRUE_B);
    }

    #[test]
    fn buffer_data_flushes_exactly_the_uploaded_bytes() {
        let gl = TestGl::new(8, 8);
        let data: Vec<u8> = (1..=23).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &data);
        let gpu = gpu_of(&gl, h);
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: 23 }]);
        assert_eq!(gl.unflushed(gpu, 23), None);
        /* reusing the storage: the new bytes are flushed too */
        // SAFETY: valid data pointer.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 4, [7u8; 4].as_ptr() as *const c_void, GL_STATIC_DRAW) };
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: 4 }]);
        assert_eq!(contents(&gl, h), [7; 4]);
    }

    #[test]
    fn buffer_data_without_data_clears_fresh_storage_and_publishes_it() {
        /* GL leaves the contents undefined, but they must not be what the
         * pool held before (the Zephyr / bare-metal pools are not zeroed) */
        let gl = TestGl::new(8, 8);
        let h = gen1();
        // SAFETY: null data is allowed.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBufferData(GL_ARRAY_BUFFER, 16, core::ptr::null(), GL_STATIC_DRAW);
        }
        let gpu = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 0, len: 16 }]);
        assert_eq!(contents(&gl, h), [0; 16], "no pool leftovers");
        assert_eq!(gl.unflushed(gpu, 16), None);
        /* storage that is reused keeps the buffer's own bytes (no memset) */
        sub_data(GL_ARRAY_BUFFER, 0, &[7; 16]);
        gl.cache_ops();
        // SAFETY: null data is allowed.
        unsafe { glBufferData(GL_ARRAY_BUFFER, 8, core::ptr::null(), GL_STATIC_DRAW) };
        assert!(gl.cache_ops().is_empty());
        assert_eq!(contents(&gl, h), [7; 8]);
    }

    #[test]
    fn buffer_data_without_a_cpu_view_clears_fresh_storage_too() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let h = gen1();
        // SAFETY: null data is allowed.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBufferData(GL_ARRAY_BUFFER, 700, core::ptr::null(), GL_STATIC_DRAW);
            /* three chunks */
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_eq!(contents(&gl, h), std::vec![0u8; 700]);
    }

    #[test]
    fn sub_data_flushes_exactly_the_updated_range() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        let gpu = gpu_of(&gl, h);
        gl.cache_ops();
        sub_data(GL_ARRAY_BUFFER, 5, &[1, 2, 3]);
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 5, len: 3 }]);
        assert_eq!(contents(&gl, h)[4..9], [0, 1, 2, 3, 0], "the GPU sees the update");
    }

    #[test]
    fn map_for_read_invalidates_the_mapped_range_first() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let gpu = gpu_of(&gl, h);
        gl.cache_ops();
        assert_eq!(map(2, 4, GL_MAP_READ_BIT), [3, 4, 5, 6]);
        assert_eq!(gl.cache_ops(), [CacheOp::Invd { gpu, off: 2, len: 4 }]);
        unmap();
        assert!(gl.cache_ops().is_empty(), "a read mapping publishes nothing");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn map_for_write_leaves_the_cache_alone_until_the_unmap_flushes_the_range() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        let gpu = gpu_of(&gl, h);
        gl.cache_ops();
        map(2, 4, GL_MAP_WRITE_BIT).copy_from_slice(&[9, 8, 7, 6]);
        assert!(gl.cache_ops().is_empty(), "a write-only mapping is not invalidated");
        assert_eq!(gl.unflushed(gpu, 8), Some(2), "written, not yet published");
        unmap();
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 2, len: 4 }]);
        assert_eq!(contents(&gl, h), [0, 0, 9, 8, 7, 6, 0, 0]);
    }

    #[test]
    fn map_for_read_and_write_invalidates_then_flushes() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[5; 8]);
        let gpu = gpu_of(&gl, h);
        gl.cache_ops();
        let m = map(0, 8, GL_MAP_READ_BIT | GL_MAP_WRITE_BIT);
        m[7] = m[0] + 1;
        unmap();
        assert_eq!(gl.cache_ops(), [CacheOp::Invd { gpu, off: 0, len: 8 }, CacheOp::Flush { gpu, off: 0, len: 8 }]);
        assert_eq!(contents(&gl, h), [5, 5, 5, 5, 5, 5, 5, 6]);
    }

    #[test]
    fn explicit_flushes_publish_only_the_flushed_ranges() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 16]);
        let gpu = gpu_of(&gl, h);
        gl.cache_ops();
        map(4, 8, GL_MAP_WRITE_BIT | GL_MAP_FLUSH_EXPLICIT_BIT).fill(0xCC);
        // SAFETY: plain GL calls.
        unsafe {
            glFlushMappedBufferRange(GL_ARRAY_BUFFER, 1, 2); /* relative to the mapping */
            glFlushMappedBufferRange(GL_ARRAY_BUFFER, 0, 0); /* nothing */
        }
        assert_eq!(gl.cache_ops(), [CacheOp::Flush { gpu, off: 5, len: 2 }]);
        unmap();
        assert!(gl.cache_ops().is_empty(), "the unmap of an explicit-flush mapping publishes nothing more");
        let want: Vec<u8> = (0..16).map(|i| if (5..7).contains(&i) { 0xCC } else { 0 }).collect();
        assert_eq!(contents(&gl, h), want, "only the flushed bytes reach the GPU");
        assert_eq!(gl.unflushed(gpu, 16), Some(4), "the rest stays unpublished (undefined in GL)");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn map_copy_on_write_invalidates_the_old_pages_and_flushes_the_fresh_ones() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let old = gpu_of(&gl, h);
        make_busy(&gl, h, 5);
        gl.cache_ops();
        let m = map(0, 8, GL_MAP_WRITE_BIT);
        let new = gpu_of(&gl, h);
        assert_ne!(new, old);
        assert_eq!(
            gl.cache_ops(),
            [CacheOp::Invd { gpu: old, off: 0, len: 8 }, CacheOp::Flush { gpu: new, off: 0, len: 8 }]
        );
        assert_eq!(contents(&gl, h), [1, 2, 3, 4, 5, 6, 7, 8], "the carried bytes are published");
        m[0] = 0x10;
        unmap();
        assert_eq!(contents(&gl, h)[0], 0x10);
        assert_eq!(gl.gpu_bytes(old, 8), [1, 2, 3, 4, 5, 6, 7, 8], "the kick in flight keeps its bytes");
    }

    #[test]
    fn a_draw_between_updates_sees_each_update_published() {
        let gl = TestGl::new(16, 16);
        let prog = gl.program("passthrough.vert", "uniform_color.frag");
        let verts: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
        let bytes: Vec<u8> = verts.iter().flat_map(|f| f.to_ne_bytes()).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &bytes);
        // SAFETY: plain GL calls; the attributes read the bound buffer.
        unsafe {
            use crate::gl::{draw::*, program::*, state::*, vertex::*};
            glUseProgram(prog);
            for i in 0..2 {
                glVertexAttribPointer(i, 4, GL_FLOAT, 0, 0, core::ptr::null());
                glEnableVertexAttribArray(i);
            }
            glDrawArrays(GL_TRIANGLES, 0, 3);
            /* the frame being recorded reads the buffer: the update moves it
             * (the whole buffer, see sub_data_copy_on_write_flushes_the_carried_bytes) */
            let mut next = bytes.clone();
            next[..4].copy_from_slice(&2.0f32.to_ne_bytes());
            sub_data(GL_ARRAY_BUFFER, 0, &next);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        /* the mock checked both draws' vertices for unflushed bytes and for changes before the kick ran */
        let draws = gl.draws();
        assert_eq!(draws.len(), 2);
        assert_ne!(draws[0].draw.attribs[0].buffer_gpu, draws[1].draw.attribs[0].buffer_gpu);
        assert_eq!(gl.gpu_bytes(draws[1].draw.attribs[0].buffer_gpu, 4), 2.0f32.to_ne_bytes());
        assert_eq!(gl.gpu_bytes(draws[0].draw.attribs[0].buffer_gpu, 4), (-1.0f32).to_ne_bytes());
        assert_eq!(gl.ctx().objs().buffers[h as usize].last_use, 1, "the second draw stamped the fresh pages");
    }

    /* ---- platforms without a CPU view of GPU memory ---- */

    #[test]
    fn buffer_data_without_a_cpu_view_uploads_through_gpu_write() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let data: Vec<u8> = (10..40).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &data);
        assert!(gl.ctx().objs().buffers[h as usize].mem.cpu.is_null());
        assert_eq!(contents(&gl, h), data);
        sub_data(GL_ARRAY_BUFFER, 28, &[1, 2]);
        assert_eq!(contents(&gl, h)[26..], [36, 37, 1, 2]);
        assert!(gl.cache_ops().is_empty(), "no mapping, nothing to flush");
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn sub_data_copy_on_write_without_a_cpu_view_bounces_every_chunk() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        /* three chunks of the 512-byte bounce buffer, the last one partial */
        let data: Vec<u8> = (0..1300).map(|i| (i % 253) as u8).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &data);
        let old = gpu_of(&gl, h);
        make_busy(&gl, h, 3);
        sub_data(GL_ARRAY_BUFFER, 1290, &[0xEE; 5]);
        assert_eq!(gl.error(), GL_NO_ERROR);
        assert_ne!(gpu_of(&gl, h), old);
        let mut want = data.clone();
        want[1290..1295].fill(0xEE);
        assert_eq!(contents(&gl, h), want);
        assert_eq!(gl.gpu_bytes(old, 1300), data, "the kick in flight keeps its bytes");
    }

    #[test]
    fn sub_data_copy_on_write_into_unmapped_fresh_pages_bounces_too() {
        let gl = TestGl::new(8, 8);
        let data: Vec<u8> = (0..600).map(|i| (i % 7) as u8).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &data); /* mapped */
        make_busy(&gl, h, 3);
        gl.set_cpu_mapping(false); /* the fresh pages are not */
        sub_data(GL_ARRAY_BUFFER, 0, &[0xEE]);
        assert!(gl.ctx().objs().buffers[h as usize].mem.cpu.is_null());
        let mut want = data.clone();
        want[0] = 0xEE;
        assert_eq!(contents(&gl, h), want);
    }

    #[test]
    fn map_without_a_cpu_view_is_out_of_memory() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 8]);
        // SAFETY: plain GL calls.
        unsafe {
            assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_READ_BIT).is_null());
            assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
            assert!(gl.ctx().objs().buffers[h as usize].mapped.is_null());
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_FALSE_B);
        }
        assert_eq!(gl.error(), GL_INVALID_OPERATION);
    }

    #[test]
    fn map_copy_on_write_needs_mapped_fresh_pages() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1; 8]);
        let old = gpu_of(&gl, h);
        make_busy(&gl, h, 3);
        gl.set_cpu_mapping(false);
        // SAFETY: plain GL call.
        unsafe { assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT).is_null()) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gpu_of(&gl, h), old, "the buffer keeps its storage");
    }

    #[test]
    fn a_refused_map_copy_on_write_releases_the_fresh_pages() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[1; 8]);
        make_busy(&gl, 1, 3);
        gl.set_cpu_mapping(false);
        let live = gl.live_allocs();
        // SAFETY: plain GL call.
        unsafe { assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 8, GL_MAP_WRITE_BIT).is_null()) };
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gl.live_allocs(), live);
    }

    #[test]
    fn a_failing_upload_is_reported() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let h = gen1();
        gl.fail_gpu_write_in(1);
        // SAFETY: valid data pointer.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, h);
            glBufferData(GL_ARRAY_BUFFER, 4, [1u8; 4].as_ptr() as *const c_void, GL_STATIC_DRAW);
        }
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY, "glBufferData");
        buffer_with(GL_ARRAY_BUFFER, &[1; 4]);
        gl.fail_gpu_write_in(1);
        sub_data(GL_ARRAY_BUFFER, 0, &[2; 4]);
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY, "glBufferSubData");
    }

    #[test]
    fn a_failing_copy_on_write_bounce_is_reported_and_keeps_the_old_storage() {
        let gl = TestGl::new(8, 8);
        gl.set_cpu_mapping(false);
        let data: Vec<u8> = (0..1300).map(|i| i as u8).collect();
        let h = buffer_with(GL_ARRAY_BUFFER, &data);
        let old = gpu_of(&gl, h);
        make_busy(&gl, h, 3);
        gl.fail_gpu_read_in(2); /* the second chunk */
        sub_data(GL_ARRAY_BUFFER, 0, &[0xEE]);
        assert_eq!(gl.error(), GL_OUT_OF_MEMORY);
        assert_eq!(gpu_of(&gl, h), old);
        assert_eq!(contents(&gl, h), data);
    }

    /* ---- GL_OES_mapbuffer -------------------------------------------------- */

    fn oes_pointer(target: GLenum) -> *mut c_void {
        let mut p = core::ptr::dangling_mut::<c_void>(); /* not null, so a call that writes nothing shows */
        // SAFETY: room for one pointer.
        unsafe { glGetBufferPointervOES(target, GL_BUFFER_MAP_POINTER_OES, &mut p) };
        p
    }

    #[test]
    fn oes_map_hands_out_the_whole_store_and_unwritten_bytes_keep_their_contents() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[0, 1, 2, 3, 4, 5, 6, 7]);
        // SAFETY: the mapping covers the whole 8-byte store until glUnmapBufferOES.
        unsafe {
            let p = glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES) as *mut u8;
            assert!(!p.is_null());
            assert_eq!(p as usize, gl.ctx().objs_ref().buffers[h as usize].mem.cpu as usize, "offset 0");
            assert_eq!(gl.ctx().objs_ref().buffers[h as usize].map_len, 8, "the whole store");
            *p.add(1) = 0xA1;
            *p.add(6) = 0xA6;
            assert_eq!(glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
        /* OES_mapbuffer: "any portion of a mapped buffer that is not written
         * will retain its original contents" */
        assert_eq!(contents(&gl, h), [0, 0xA1, 2, 3, 4, 5, 0xA6, 7]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn oes_map_works_on_the_element_array_buffer_too() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ELEMENT_ARRAY_BUFFER, &[9, 9, 9, 9]);
        // SAFETY: the mapping covers the 4-byte store until glUnmapBufferOES.
        unsafe {
            let p = glMapBufferOES(GL_ELEMENT_ARRAY_BUFFER, GL_WRITE_ONLY_OES) as *mut u8;
            assert!(!p.is_null());
            *p = 1;
            assert_eq!(glUnmapBufferOES(GL_ELEMENT_ARRAY_BUFFER), GL_TRUE_B);
        }
        assert_eq!(contents(&gl, h), [1, 9, 9, 9]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn oes_map_errors() {
        let gl = TestGl::new(8, 8);
        // SAFETY: plain GL calls; no mapping is dereferenced.
        unsafe {
            assert!(glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES).is_null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "nothing bound");
        }
        let h = buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls; no mapping is dereferenced.
        unsafe {
            /* the one access value; GL_MAP_WRITE_BIT and GL 2.0's READ_ONLY /
             * READ_WRITE (0x88B8, 0x88BA) are other values */
            for access in [GL_MAP_WRITE_BIT, 0x88b8, 0x88ba, 0] {
                assert!(glMapBufferOES(GL_ARRAY_BUFFER, access).is_null(), "access {access:#x}");
                assert_eq!(gl.error(), GL_INVALID_ENUM, "access {access:#x}");
            }
            /* the ES 2.0 targets only: an ES 3.0 target and a non-target */
            for target in [GL_PIXEL_UNPACK_BUFFER, GL_TEXTURE_2D] {
                assert!(glMapBufferOES(target, GL_WRITE_ONLY_OES).is_null(), "target {target:#x}");
                assert_eq!(gl.error(), GL_INVALID_ENUM, "target {target:#x}");
            }
            assert!(gl.ctx().objs_ref().buffers[h as usize].mapped.is_null(), "no error maps");
            assert!(!glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES).is_null());
            assert!(glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES).is_null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "mapped twice");
            assert_eq!(glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE_B);
        }
    }

    #[test]
    fn oes_map_of_a_zero_sized_store_is_invalid_operation() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[]);
        // SAFETY: plain GL calls.
        unsafe { assert!(glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES).is_null()) };
        assert_eq!(gl.error(), GL_INVALID_OPERATION, "as ES 3.0's zero-length range");
    }

    #[test]
    fn oes_state_queries_follow_the_mapping() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        /* the extension's initial state: WRITE_ONLY_OES, FALSE, NULL */
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_ACCESS_OES), GL_WRITE_ONLY_OES as GLint);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_MAPPED_OES), 0);
        assert!(oes_pointer(GL_ARRAY_BUFFER).is_null());
        // SAFETY: plain GL calls.
        let p = unsafe { glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES) };
        assert!(!p.is_null());
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_ACCESS_OES), GL_WRITE_ONLY_OES as GLint);
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_MAPPED_OES), 1);
        assert_eq!(oes_pointer(GL_ARRAY_BUFFER), p);
        // SAFETY: plain GL calls.
        unsafe { assert_eq!(glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE_B) };
        assert_eq!(param(GL_ARRAY_BUFFER, GL_BUFFER_MAPPED_OES), 0);
        assert!(oes_pointer(GL_ARRAY_BUFFER).is_null());
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn oes_pointer_and_unmap_errors() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 4]);
        let mut p = core::ptr::dangling_mut::<c_void>();
        // SAFETY: plain GL calls; room for one pointer.
        unsafe {
            glGetBufferPointervOES(GL_ARRAY_BUFFER, GL_BUFFER_SIZE, &mut p);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "pname");
            glGetBufferPointervOES(GL_PIXEL_UNPACK_BUFFER, GL_BUFFER_MAP_POINTER_OES, &mut p);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "an ES 3.0 target");
            assert_eq!(p, core::ptr::dangling_mut::<c_void>(), "nothing written on an error");
            assert_eq!(glUnmapBufferOES(GL_PIXEL_UNPACK_BUFFER), GL_FALSE_B);
            assert_eq!(gl.error(), GL_INVALID_ENUM, "an ES 3.0 target");
            assert_eq!(glUnmapBufferOES(GL_ARRAY_BUFFER), GL_FALSE_B);
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "not mapped");
        }
    }

    #[test]
    fn oes_and_core_calls_share_one_mapping_state() {
        let gl = TestGl::new(8, 8);
        buffer_with(GL_ARRAY_BUFFER, &[0; 8]);
        // SAFETY: plain GL calls; no mapping is dereferenced.
        unsafe {
            assert!(!glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES).is_null());
            assert!(glMapBufferRange(GL_ARRAY_BUFFER, 0, 4, GL_MAP_WRITE_BIT).is_null());
            assert_eq!(gl.error(), GL_INVALID_OPERATION, "an OES mapping is a mapping");
            assert_eq!(glUnmapBuffer(GL_ARRAY_BUFFER), GL_TRUE_B, "the core call unmaps it");
            assert!(!glMapBufferRange(GL_ARRAY_BUFFER, 0, 4, GL_MAP_WRITE_BIT).is_null());
            assert_eq!(glUnmapBufferOES(GL_ARRAY_BUFFER), GL_TRUE_B, "and the OES call a range mapping");
        }
        assert_eq!(gl.error(), GL_NO_ERROR);
    }

    #[test]
    fn oes_map_copies_on_write_while_a_kick_reads_the_buffer() {
        let gl = TestGl::new(8, 8);
        let h = buffer_with(GL_ARRAY_BUFFER, &[1, 2, 3, 4, 5, 6, 7, 8]);
        let old = gl.ctx().objs().buffers[h as usize].mem.gpu;
        make_busy(&gl, h, 5);
        gl.mem_events();
        // SAFETY: the mapping covers the 8-byte store until glUnmapBufferOES.
        unsafe {
            let p = glMapBufferOES(GL_ARRAY_BUFFER, GL_WRITE_ONLY_OES) as *mut u8;
            assert!(!p.is_null());
            *p.add(2) = 0x33;
            glUnmapBufferOES(GL_ARRAY_BUFFER);
        }
        let new = gl.ctx().objs().buffers[h as usize].mem.gpu;
        assert_ne!(new, old, "the kick still reads the old pages");
        assert_eq!(
            gl.mem_events(),
            [MemEvent::Alloc { heap: HEAP_DATA, size: 8, gpu: new }, MemEvent::Free { gpu: old }]
        );
        /* the fresh pages start as a copy: unwritten bytes keep their contents */
        assert_eq!(contents(&gl, h), [1, 2, 0x33, 4, 5, 6, 7, 8]);
        assert_eq!(gl.error(), GL_NO_ERROR);
    }
}
