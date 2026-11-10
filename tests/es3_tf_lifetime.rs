// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 D.1.2 lifetime of a buffer deleted while an object that is not
//! bound attaches it, through the public GL API on the host
//! (docs/notes/es3-tf-lifetime.md; dEQP-GLES3 lifetime.attach.deleted_output.
//! buffer_transform_feedback and deleted_input.buffer_vertex_array).
//!
//! The dEQP sequence binds the deleted name again without `glGenBuffers`: it
//! is a new object, and the old storage lives on for the transform feedback
//! (or vertex array) object that holds it. The host keeps the capture's bytes
//! in the buffer, so the new buffer's bytes and the old storage are read back.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_tf_lifetime
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vao::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

/// Every storage the stand-in handed out (it never reuses one): a buffer the
/// application can no longer name is still checked through its storage.
static ALLOCS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

/// Some storage of `n` bytes whose first bytes are `head` and whose bytes
/// from `tail_from` on are all `fill`.
fn storage_with(n: usize, head: &[u8], tail_from: usize, fill: u8) -> bool {
    let allocs = ALLOCS.lock().unwrap_or_else(|e| e.into_inner());
    allocs.iter().filter(|&&(_, size)| size == n).any(|&(p, size)| {
        // SAFETY: the stand-in leaks its allocations, so they stay valid.
        let m = unsafe { core::slice::from_raw_parts(p as *const u8, size) };
        m.starts_with(head) && m[tail_from..].iter().all(|&b| b == fill)
    })
}

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    ALLOCS.lock().unwrap_or_else(|e| e.into_inner()).push((p as usize, size));
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
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    reset();
    g
}

/// Back to a known state after the previous test: default objects, no
/// program, the error flag clear.
fn reset() {
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        glBindVertexArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
}

/* ---- helpers ---------------------------------------------------------------- */

const VS: &str = "#version 300 es\nin vec4 a;\nout vec4 v;\nvoid main() { v = a; gl_Position = a; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// A program capturing the varying `v`, current.
fn program() {
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        let v = [c"v".as_ptr()];
        glTransformFeedbackVaryings(p, 1, v.as_ptr(), GL_INTERLEAVED_ATTRIBS);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
    }
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// `glBufferData` of `n` bytes `fill` into `name` bound at GL_ARRAY_BUFFER;
/// no `glGenBuffers` is needed, binding creates the object.
fn specify(name: GLuint, n: usize, fill: u8) {
    let data = vec![fill; n];
    // SAFETY: plain GL calls; `data` outlives the call.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, name);
        glBufferData(GL_ARRAY_BUFFER, n as GLsizeiptr, data.as_ptr().cast(), GL_DYNAMIC_READ);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
    }
}

/// The first `n` bytes of buffer `name`, read through a mapping.
fn bytes(name: GLuint, n: usize) -> Vec<u8> {
    // SAFETY: plain GL calls; the mapping is read inside its lifetime.
    unsafe {
        glBindBuffer(GL_COPY_READ_BUFFER, name);
        let p = glMapBufferRange(GL_COPY_READ_BUFFER, 0, n as GLsizeiptr, GL_MAP_READ_BIT) as *const u8;
        assert!(!p.is_null(), "buffer {name} maps (error {:#x})", err());
        let v = core::slice::from_raw_parts(p, n).to_vec();
        glUnmapBuffer(GL_COPY_READ_BUFFER);
        glBindBuffer(GL_COPY_READ_BUFFER, 0);
        v
    }
}

/// Capture one triangle (three vec4) into the buffer `tf` attaches at index 0.
fn capture_through(tf: GLuint) {
    let input: [f32; 12] = [1.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0, 1.0, 2.0, 3.0, 4.0];
    // SAFETY: plain GL calls; the client array outlives the draw.
    unsafe {
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, input.as_ptr().cast());
        glBeginTransformFeedback(GL_TRIANGLES);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(err(), GL_NO_ERROR);
        glEndTransformFeedback();
        glDisableVertexAttribArray(0);
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
    }
}

/* ---- the cases ---------------------------------------------------------------- */

/// The dEQP sequence of `OutputAttachmentTest` for a transform feedback
/// object: the new buffer under the deleted name is written by nobody, the
/// old storage is the object's.
#[test]
fn a_deleted_output_buffer_rebound_by_name_is_a_new_object() {
    let _g = context();
    program();
    let (mut tf, mut x) = (0, 0);
    // SAFETY: plain GL calls.
    unsafe {
        glGenTransformFeedbacks(1, &mut tf);
        glGenBuffers(1, &mut x);
    }
    specify(x, 64, 0xcd);
    // SAFETY: plain GL calls.
    unsafe {
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x);
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, 0);
        glDeleteBuffers(1, &x);
        assert_eq!(glIsBuffer(x), GL_FALSE_B);
    }
    specify(x, 64, 0xab);
    // SAFETY: plain GL call.
    unsafe { assert_eq!(glIsBuffer(x), GL_TRUE_B, "binding the free name created a buffer") };
    capture_through(tf);
    assert_eq!(bytes(x, 64), [0xab; 64], "the new buffer is not written through the object");

    // SAFETY: plain GL calls.
    unsafe {
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
        let mut held: GLint = -1;
        glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0, &mut held);
        let old = held as GLuint;
        assert_ne!(old, 0, "the object still attaches the old storage");
        assert_ne!(old, x, "and it is not the new buffer");
        let want: Vec<u8> = [1.0f32, 2.0, 3.0, 4.0].iter().flat_map(|f| f.to_ne_bytes()).collect();
        /* the old storage is not nameable (its name is free): find it by its bytes */
        assert!(storage_with(64, &want, 48, 0xcd), "the old storage took the capture, and only it");

        /* the last holder lets go: the old storage goes, the live buffer stays */
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        assert_eq!(glIsBuffer(x), GL_TRUE_B);
        let patch = [0x11u8; 4];
        glBindBuffer(GL_ARRAY_BUFFER, x);
        glBufferSubData(GL_ARRAY_BUFFER, 0, 4, patch.as_ptr().cast());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        assert_eq!(err(), GL_NO_ERROR, "the live buffer was not freed with the orphan");
        assert_eq!(&bytes(x, 8), &[0x11, 0x11, 0x11, 0x11, 0xab, 0xab, 0xab, 0xab]);
        glDeleteTransformFeedbacks(1, &tf);
        glDeleteBuffers(1, &x);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// The same rule for a vertex array object that attaches the deleted buffer
/// (deleted_input.buffer_vertex_array without a generated name).
#[test]
fn a_deleted_input_buffer_rebound_by_name_is_a_new_object() {
    let _g = context();
    let (mut vao, mut x) = (0, 0);
    // SAFETY: plain GL calls.
    unsafe {
        glGenVertexArrays(1, &mut vao);
        glGenBuffers(1, &mut x);
    }
    specify(x, 64, 0xc2);
    // SAFETY: plain GL calls.
    unsafe {
        glBindVertexArray(vao);
        glBindBuffer(GL_ARRAY_BUFFER, x);
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindVertexArray(0);
        glDeleteBuffers(1, &x);
    }
    specify(x, 64, 0xab);
    assert_eq!(bytes(x, 64), [0xab; 64], "the new buffer holds its own bytes");
    // SAFETY: plain GL calls.
    unsafe {
        glBindVertexArray(vao);
        let mut held: GLint = -1;
        glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &mut held);
        assert_ne!(held, 0);
        assert_ne!(held as GLuint, x, "the object attaches the old storage, not the new buffer");
        assert!(storage_with(64, &[0xc2; 64], 0, 0xc2), "the old storage is intact");

        /* the last holder lets go: the live buffer stays */
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null());
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &vao);
        assert_eq!(glIsBuffer(x), GL_TRUE_B);
        assert_eq!(bytes(x, 64), [0xab; 64]);
        glDeleteBuffers(1, &x);
    }
    assert_eq!(err(), GL_NO_ERROR);
}
