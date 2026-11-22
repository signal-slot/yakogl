// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC3, dEQP-GLES3.functional.lifetime.attach.deleted_output.
//! buffer_transform_feedback with the name never reused: a buffer attached
//! to an unbound transform feedback object is deleted, and capturing through
//! the object still writes the attached storage (ES 3.0 5.1.2, D.1.2). The
//! values are read back from the stand-in's allocation, not from pixels.
//! tests/es3_tf_lifetime.rs pins the variant that binds the name again; the
//! fix (`tf::on_buffer_delete`, `free_orphan`) is da0eede3, already in the
//! tree: no GL-core change was needed for this item.
//!
//! The stand-in context is the one of tests/es3_tf_lifetime.rs; allocations
//! are never given back, so a freed storage stays readable.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_lifetime_tf
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_lifetime_tf
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]
#![allow(dead_code)]

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
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
    name: c"es3-lifetime-tf".as_ptr() as *const c_char,
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
    max_samples: 0,
};

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
    // SAFETY: plain GL calls on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn storage(slot: GLuint) -> *const u8 {
    let c = yakogl::gl::context::current().expect("a context");
    let b = &c.objs_ref().buffers[slot as usize];
    if b.used {
        b.mem.cpu as *const u8
    } else {
        core::ptr::null()
    }
}

fn bytes_at(p: *const u8, n: usize) -> Vec<u8> {
    assert!(!p.is_null());
    // SAFETY: the OSAL above leaks every allocation of at least `n` bytes.
    unsafe { core::slice::from_raw_parts(p, n) }.to_vec()
}

const VS: &str = "#version 300 es\nin vec4 a;\nvoid main() { gl_Position = a; }\n";
const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

#[test]
fn a_capture_after_the_attached_buffer_is_deleted_lands_in_its_storage() {
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

    let (mut tf, mut x) = (0, 0);
    let init = [0xcdu8; 64];
    // SAFETY: plain GL calls; `init` holds 64 bytes.
    unsafe {
        glGenTransformFeedbacks(1, &mut tf);
        glGenBuffers(1, &mut x);
        glBindBuffer(GL_ARRAY_BUFFER, x);
        glBufferData(GL_ARRAY_BUFFER, 64, init.as_ptr().cast(), GL_STATIC_DRAW);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x);
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
    }
    assert_eq!(err(), GL_NO_ERROR);
    capture(tf, &first);
    let old = storage(x);
    let mut want = as_bytes(&first);
    want.resize(64, 0xcd);
    assert_eq!(bytes_at(old, 64), want, "the first capture is in X");

    // SAFETY: plain GL call.
    unsafe { glDeleteBuffers(1, &x) };
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glIsBuffer(x) }, GL_FALSE_B, "the name is gone");

    capture(tf, &second);
    let mut want = as_bytes(&second);
    want.resize(64, 0xcd);
    assert_eq!(bytes_at(old, 64), want, "the capture still wrote the attached storage");
    // SAFETY: plain GL calls.
    unsafe {
        glDeleteTransformFeedbacks(1, &tf);
        glDeleteProgram(prog);
    }
    assert_eq!(err(), GL_NO_ERROR);
}
