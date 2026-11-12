// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A buffer deleted while an object that is not bound attaches it, and its
//! name bound again without glGenBuffers (docs/notes/es3-tf-lifetime.md):
//! the name is a new buffer, the deleted one's storage stays with the
//! objects that hold it (ES 3.0 2.9, D.1.2). dEQP-GLES3.functional.lifetime.
//! attach.deleted_output.buffer_transform_feedback reuses the name that way
//! (its buffer binder does not generate one), and so does
//! deleted_input.buffer_vertex_array where the test does not generate it.
//!
//! The holders: a transform feedback object (the dEQP sequence, with the
//! capture, and the object bound again before the name is), a vertex array
//! object not bound when the name is bound again, one bound again before
//! the name is (its attachments are then the context's live state), and no
//! free slot for the orphan (the holders lose it). Each case checks the new
//! buffer's bytes, where the holders point, the old storage, and that the
//! old storage is freed once the last holder lets go while the new buffer
//! stays.
//!
//! The context is the smallest OSAL and back end that make one, as in
//! tests/es3_tf_api.rs; the allocations are never given back, so a freed
//! storage stays readable through its old pointer. The vertex array cases
//! run on the ES 2.0 report too (GL_OES_vertex_array_object); the capture
//! needs the in-process compiler (`shaderc-online`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_tf_lifetime
//! ```

#![cfg(feature = "host-test")]
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context::MAX_BUFFERS;
use yakogl::gl::es3::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vao::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-tf-lifetime".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindVertexArrayOES(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLuint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v as GLuint
}

fn attrib_buffer(index: GLuint) -> GLuint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetVertexAttribiv(index, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &mut v) };
    v as GLuint
}

fn gen_buffer() -> GLuint {
    let mut b = 0;
    // SAFETY: one name.
    unsafe { glGenBuffers(1, &mut b) };
    assert_ne!(b, 0);
    b
}

/// Bind `name` at `target` and give it `n` bytes of `byte`.
fn fill(target: GLenum, name: GLuint, n: usize, byte: u8) {
    let data = vec![byte; n];
    // SAFETY: plain GL calls; `data` holds `n` bytes.
    unsafe {
        glBindBuffer(target, name);
        glBufferData(target, n as GLsizeiptr, data.as_ptr().cast(), GL_STATIC_DRAW);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The CPU view of buffer slot `slot`'s storage, null when it has none.
fn storage(slot: GLuint) -> *const u8 {
    let c = yakogl::gl::context::current().expect("a context");
    let b = &c.objs_ref().buffers[slot as usize];
    if b.used {
        b.mem.cpu as *const u8
    } else {
        core::ptr::null()
    }
}

/// `n` bytes at `p`, a storage of this file's OSAL (never given back).
fn bytes_at(p: *const u8, n: usize) -> Vec<u8> {
    assert!(!p.is_null());
    // SAFETY: the OSAL above leaks every allocation, and each one here
    // holds at least `n` bytes.
    unsafe { core::slice::from_raw_parts(p, n) }.to_vec()
}

fn is_slot_used(slot: GLuint) -> bool {
    let c = yakogl::gl::context::current().expect("a context");
    c.objs_ref().buffers[slot as usize].used
}

fn gen_vao() -> GLuint {
    let mut v = 0;
    // SAFETY: one name.
    unsafe { glGenVertexArraysOES(1, &mut v) };
    assert_ne!(v, 0);
    v
}

/// Vertex array object `vao` with buffer `attrib` on attribute 0 and
/// `element` as its element array buffer; vertex array object 0 bound after.
fn attach(vao: GLuint, attrib: GLuint, element: GLuint) {
    // SAFETY: plain GL calls; offset 0 into the bound buffer.
    unsafe {
        glBindVertexArrayOES(vao);
        glBindBuffer(GL_ARRAY_BUFFER, attrib);
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null());
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, element);
        glBindVertexArrayOES(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The bound vertex array object lets go of every buffer: attribute 0 on
/// `other`, no element array buffer.
fn let_go(other: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, other);
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null());
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

fn delete(names: &[GLuint]) {
    // SAFETY: `names` holds the count given.
    unsafe { glDeleteBuffers(names.len() as GLsizei, names.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
}

fn delete_vao(v: GLuint) {
    // SAFETY: one name.
    unsafe { glDeleteVertexArraysOES(1, &v) };
}

/* -------------------------------------------------- vertex array objects -- */

/// The object is not bound when the deleted names are bound again: it keeps
/// the old storages under other slots, the names are new buffers, and the
/// old storages are freed when it lets go.
#[test]
fn an_unbound_vertex_array_object_keeps_the_storage_of_a_name_bound_again() {
    let _g = context();
    let (x, y, other) = (gen_buffer(), gen_buffer(), gen_buffer());
    fill(GL_ARRAY_BUFFER, x, 64, 0xcd);
    fill(GL_ARRAY_BUFFER, y, 32, 0xce);
    fill(GL_ARRAY_BUFFER, other, 16, 0);
    let (old_x, old_y) = (storage(x), storage(y));
    let vao = gen_vao();
    attach(vao, x, y);
    delete(&[x, y]);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glIsBuffer(x) }, GL_FALSE_B);

    fill(GL_ARRAY_BUFFER, x, 64, 0xab);
    fill(GL_ARRAY_BUFFER, y, 32, 0xac);
    // SAFETY: plain GL calls.
    assert_eq!(unsafe { (glIsBuffer(x), glIsBuffer(y)) }, (GL_TRUE_B, GL_TRUE_B));
    assert_eq!(bytes_at(storage(x), 64), vec![0xab; 64], "the new buffer X");
    assert_eq!(bytes_at(storage(y), 32), vec![0xac; 32], "the new buffer Y");
    assert_ne!(storage(x), old_x);
    assert_ne!(storage(y), old_y);

    // SAFETY: plain GL call.
    unsafe { glBindVertexArrayOES(vao) };
    let (ax, ey) = (attrib_buffer(0), geti(GL_ELEMENT_ARRAY_BUFFER_BINDING));
    assert!(ax != 0 && ax != x, "the attribute keeps the old storage, not the name X ({ax})");
    assert!(ey != 0 && ey != y, "the element binding keeps the old storage, not the name Y ({ey})");
    assert_eq!(storage(ax), old_x);
    assert_eq!(storage(ey), old_y);
    assert_eq!(bytes_at(old_x, 64), vec![0xcd; 64], "the old storage of X is untouched");
    assert_eq!(bytes_at(old_y, 32), vec![0xce; 32], "the old storage of Y is untouched");

    let_go(other);
    assert!(!is_slot_used(ax) && !is_slot_used(ey), "the orphans are freed once the object lets go");
    assert_eq!(bytes_at(storage(x), 64), vec![0xab; 64], "the new X lives on");
    assert_eq!(bytes_at(storage(y), 32), vec![0xac; 32], "the new Y lives on");
    // SAFETY: plain GL call.
    unsafe { glBindVertexArrayOES(0) };
    delete_vao(vao);
    delete(&[x, y, other]);
}

/// The object is bound again after the delete and before the names are:
/// its attachments are then the context's live state, and they follow the
/// orphans too. Y is bound again on GL_ARRAY_BUFFER, so the object's
/// element binding stays the old storage.
#[test]
fn a_vertex_array_object_bound_again_before_the_name_keeps_the_old_storage() {
    let _g = context();
    let (x, y, other) = (gen_buffer(), gen_buffer(), gen_buffer());
    fill(GL_ARRAY_BUFFER, x, 64, 0xcd);
    fill(GL_ARRAY_BUFFER, y, 32, 0xce);
    fill(GL_ARRAY_BUFFER, other, 16, 0);
    let (old_x, old_y) = (storage(x), storage(y));
    let vao = gen_vao();
    attach(vao, x, y);
    delete(&[x, y]);
    // SAFETY: plain GL call.
    unsafe { glBindVertexArrayOES(vao) };
    assert_eq!((attrib_buffer(0), geti(GL_ELEMENT_ARRAY_BUFFER_BINDING)), (x, y), "the object still attaches them");

    fill(GL_ARRAY_BUFFER, x, 64, 0xab);
    fill(GL_ARRAY_BUFFER, y, 32, 0xac);
    // SAFETY: plain GL call.
    unsafe { glBindBuffer(GL_ARRAY_BUFFER, 0) };
    let (ax, ey) = (attrib_buffer(0), geti(GL_ELEMENT_ARRAY_BUFFER_BINDING));
    assert!(ax != 0 && ax != x, "the attribute does not name the new X ({ax})");
    assert!(ey != 0 && ey != y, "the element binding does not name the new Y ({ey})");
    assert_eq!(storage(ax), old_x);
    assert_eq!(storage(ey), old_y);
    assert_eq!(bytes_at(storage(x), 64), vec![0xab; 64]);
    assert_eq!(bytes_at(storage(y), 32), vec![0xac; 32]);
    assert_eq!(bytes_at(old_x, 64), vec![0xcd; 64]);
    assert_eq!(bytes_at(old_y, 32), vec![0xce; 32]);

    /* the object lets go, still bound: both orphans are freed */
    let_go(other);
    assert!(!is_slot_used(ax) && !is_slot_used(ey), "the orphans are freed once the object lets go");
    assert!(is_slot_used(x) && is_slot_used(y), "the new buffers live on");
    // SAFETY: plain GL call.
    unsafe { glBindVertexArrayOES(0) };
    delete_vao(vao);
    delete(&[x, y, other]);
}

/// The deleted name bound again on the element array target while the
/// object holding it is bound: the binding is the new buffer, and the old
/// storage, which nothing holds any more, is freed.
#[test]
fn the_name_bound_again_as_the_bound_objects_element_buffer_replaces_the_orphan() {
    let _g = context();
    let y = gen_buffer();
    fill(GL_ARRAY_BUFFER, y, 32, 0xce);
    let old_y = storage(y);
    let vao = gen_vao();
    // SAFETY: plain GL calls.
    unsafe {
        glBindVertexArrayOES(vao);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, y);
        glBindVertexArrayOES(0);
    }
    delete(&[y]);
    // SAFETY: plain GL calls.
    unsafe {
        glBindVertexArrayOES(vao);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, y);
    }
    assert_eq!(geti(GL_ELEMENT_ARRAY_BUFFER_BINDING), y, "the new buffer Y");
    assert!(storage(y).is_null(), "no storage until glBufferData");
    let c = yakogl::gl::context::current().expect("a context");
    let held = c.objs_ref().buffers.iter().any(|b| b.used && b.mem.cpu as *const u8 == old_y);
    assert!(!held, "the old storage is freed: nothing holds it");
    fill(GL_ELEMENT_ARRAY_BUFFER, y, 32, 0xac);
    assert_eq!(bytes_at(old_y, 32), vec![0xce; 32], "the new buffer has its own storage");
    // SAFETY: plain GL calls.
    unsafe {
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glBindVertexArrayOES(0);
    }
    delete_vao(vao);
    delete(&[y]);
}

/// No free slot for the orphan: it is freed and the object loses the
/// attachment rather than reading the new buffer.
#[test]
fn without_a_free_slot_the_holders_lose_the_orphan() {
    let _g = context();
    let x = gen_buffer();
    fill(GL_ARRAY_BUFFER, x, 64, 0xcd);
    let vao = gen_vao();
    attach(vao, x, 0);
    delete(&[x]);
    /* every other slot taken */
    let mut rest = vec![0; MAX_BUFFERS];
    // SAFETY: room for MAX_BUFFERS names.
    unsafe { glGenBuffers(MAX_BUFFERS as GLsizei, rest.as_mut_ptr()) };
    while err() != GL_NO_ERROR {}
    rest.retain(|&n| n != 0);
    assert!((1..MAX_BUFFERS as GLuint).all(is_slot_used), "the table is full");

    fill(GL_ARRAY_BUFFER, x, 64, 0xab);
    // SAFETY: plain GL calls.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindVertexArrayOES(vao);
    }
    assert_eq!(attrib_buffer(0), 0, "the object lost the buffer");
    assert_eq!(bytes_at(storage(x), 64), vec![0xab; 64], "the new buffer X");
    // SAFETY: plain GL call.
    unsafe { glBindVertexArrayOES(0) };
    delete_vao(vao);
    delete(&rest);
    delete(&[x]);
}

/* ------------------------------------------- transform feedback objects -- */

#[cfg(not(feature = "report-es2"))]
mod es3 {
    use super::*;

    fn gen_tf() -> GLuint {
        let mut t = 0;
        // SAFETY: one name.
        unsafe { glGenTransformFeedbacks(1, &mut t) };
        t
    }

    fn tf_binding() -> GLuint {
        let mut v = i32::MIN;
        // SAFETY: room for one value.
        unsafe { glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0, &mut v) };
        v as GLuint
    }

    /// Transform feedback object `tf` attaches `name` at index 0, then the
    /// default object is bound again (dEQP's BufferTfAttacher).
    fn attach_tf(tf: GLuint, name: GLuint) {
        // SAFETY: plain GL calls.
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, name);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        }
        assert_eq!(err(), GL_NO_ERROR);
    }

    /// The object is bound again after the delete and before the name is:
    /// its binding follows the orphan, and dropping it frees the orphan.
    #[test]
    fn a_transform_feedback_object_bound_again_before_the_name_keeps_the_old_storage() {
        let _g = context();
        let x = gen_buffer();
        fill(GL_ARRAY_BUFFER, x, 64, 0xcd);
        let old_x = storage(x);
        let tf = gen_tf();
        attach_tf(tf, x);
        delete(&[x]);
        // SAFETY: plain GL call.
        unsafe { glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf) };
        assert_eq!(tf_binding(), x, "the object still attaches it");
        fill(GL_ARRAY_BUFFER, x, 64, 0xab);
        let held = tf_binding();
        assert!(held != 0 && held != x, "the binding does not name the new X ({held})");
        assert_eq!(storage(held), old_x);
        assert_eq!(bytes_at(old_x, 64), vec![0xcd; 64]);
        // SAFETY: plain GL calls.
        unsafe {
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            glDeleteTransformFeedbacks(1, &tf);
        }
        assert!(!is_slot_used(held), "the orphan is freed once the object lets go");
        assert_eq!(bytes_at(storage(x), 64), vec![0xab; 64], "the new X lives on");
        delete(&[x]);
    }

    /// dEQP lifetime.attach.deleted_output.buffer_transform_feedback, byte
    /// by byte: capture into X through an unbound object, delete X, make a
    /// new buffer under the name X without glGenBuffers, capture again
    /// through the object. The new X keeps its bytes; the capture lands in
    /// the old storage; dropping the binding frees it and not the new X.
    #[cfg(feature = "shaderc-online")]
    #[test]
    fn a_capture_through_an_unbound_object_never_writes_the_new_buffer_under_the_name() {
        use std::ffi::CString;
        use yakogl::gl::draw::*;
        use yakogl::gl::program::*;
        use yakogl::gl::shader::*;

        const VS: &str = "#version 300 es\nin vec4 a;\nvoid main() { gl_Position = a; }\n";
        const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";
        let _g = context();
        // SAFETY: plain GL calls; every pointer outlives its call.
        let prog = unsafe {
            let p = glCreateProgram();
            for (ty, text) in [(GL_VERTEX_SHADER, VS), (GL_FRAGMENT_SHADER, FS)] {
                let text = CString::new(text).unwrap();
                let s = glCreateShader(ty);
                let src: [*const c_char; 1] = [text.as_ptr()];
                glShaderSource(s, 1, src.as_ptr(), core::ptr::null());
                glCompileShader(s);
                glAttachShader(p, s);
            }
            let name = CString::new("gl_Position").unwrap();
            let names: [*const c_char; 1] = [name.as_ptr()];
            glTransformFeedbackVaryings(p, 1, names.as_ptr(), GL_INTERLEAVED_ATTRIBS);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link");
            p
        };
        /* three points through `tf`: 48 bytes of gl_Position */
        let capture = |tf: GLuint, input: &[f32; 12]| {
            // SAFETY: plain GL calls; the client array outlives the draw.
            unsafe {
                glUseProgram(prog);
                glBindBuffer(GL_ARRAY_BUFFER, 0);
                glEnableVertexAttribArray(0);
                glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, input.as_ptr().cast());
                glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
                glBeginTransformFeedback(GL_POINTS);
                glDrawArrays(GL_POINTS, 0, 3);
                glEndTransformFeedback();
                glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
                glDisableVertexAttribArray(0);
                glUseProgram(0);
            }
            assert_eq!(err(), GL_NO_ERROR);
        };
        let as_bytes = |v: &[f32]| v.iter().flat_map(|f| f.to_ne_bytes()).collect::<Vec<u8>>();
        let first: [f32; 12] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
        let second: [f32; 12] = [-1.0, -2.0, -3.0, -4.0, -5.0, -6.0, -7.0, -8.0, -9.0, -10.0, -11.0, -12.0];

        /* steps 1-4: X attached to the unbound object, captured into */
        let tf = gen_tf();
        let x = gen_buffer();
        fill(GL_ARRAY_BUFFER, x, 64, 0xcd);
        attach_tf(tf, x);
        capture(tf, &first);
        let old_x = storage(x);
        let mut want = as_bytes(&first);
        want.resize(64, 0xcd);
        assert_eq!(bytes_at(old_x, 64), want, "the first capture is in X");

        /* steps 5-6: X deleted, the name made a new buffer without glGenBuffers */
        delete(&[x]);
        // SAFETY: plain GL call.
        assert_eq!(unsafe { glIsBuffer(x) }, GL_FALSE_B);
        fill(GL_ARRAY_BUFFER, x, 64, 0xab);
        // SAFETY: plain GL calls.
        assert_eq!(unsafe { glIsBuffer(x) }, GL_TRUE_B);
        unsafe { glBindBuffer(GL_ARRAY_BUFFER, 0) };
        assert_ne!(storage(x), old_x, "the new X has its own storage");

        /* step 8: capture through the object again */
        capture(tf, &second);
        assert_eq!(bytes_at(storage(x), 64), vec![0xab; 64], "the capture did not write the new X");
        let mut want = as_bytes(&second);
        want.resize(64, 0xcd);
        assert_eq!(bytes_at(old_x, 64), want, "the capture wrote the old storage");
        // SAFETY: plain GL call.
        unsafe { glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf) };
        let held = tf_binding();
        assert!(held != 0 && held != x, "the object's binding is not the new X ({held})");
        assert_eq!(storage(held), old_x);

        /* step 9: the object drops the binding: the orphan goes, X stays */
        // SAFETY: plain GL calls.
        unsafe {
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        }
        assert!(!is_slot_used(held), "the orphan is freed once the object lets go");
        let patch = [0x11u8; 8];
        // SAFETY: plain GL calls; `patch` holds 8 bytes.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, x);
            glBufferSubData(GL_ARRAY_BUFFER, 0, 8, patch.as_ptr().cast());
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glDeleteTransformFeedbacks(1, &tf);
            glDeleteProgram(prog);
        }
        assert_eq!(err(), GL_NO_ERROR, "the new X is still a buffer");
        let mut want = vec![0xab; 64];
        want[..8].copy_from_slice(&patch);
        assert_eq!(bytes_at(storage(x), 64), want);
        delete(&[x]);
    }
}
