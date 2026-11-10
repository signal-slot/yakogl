// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A buffer deleted while a transform feedback object or a vertex array object
//! that is not bound attaches it lives on for that object (ES 3.0 D.1.2), and
//! its name is free at once: binding the name again (no glGenBuffers, as dEQP's
//! lifetime.attach.deleted_{output.buffer_transform_feedback,input.buffer_vertex_array}
//! do) makes a new buffer, which the old holder never touches
//! (docs/notes/es3-tf-lifetime.md).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_tf_lifetime
//! ```

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

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

/// Every storage the stand-in allocator handed out (address, size); none is freed.
static ALLOCS: Mutex<Vec<(usize, usize)>> = Mutex::new(Vec::new());

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
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        glBindVertexArray(0);
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A new buffer of 64 bytes of `fill` under `name`; with a name glGenBuffers
/// did not hand out this is the "bind creates the object" case.
fn specify(name: GLuint, fill: u8) {
    let data = [fill; 64];
    // SAFETY: plain GL calls; the client array outlives the call.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, name);
        glBufferData(GL_ARRAY_BUFFER, 64, data.as_ptr().cast(), GL_DYNAMIC_READ);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
    }
}

fn gen_buffer(fill: u8) -> GLuint {
    let mut b = 0;
    // SAFETY: one name.
    unsafe { glGenBuffers(1, &mut b) };
    specify(b, fill);
    b
}

fn contents(name: GLuint) -> Vec<u8> {
    // SAFETY: plain GL calls; 64 bytes were specified.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, name);
        let p = glMapBufferRange(GL_ARRAY_BUFFER, 0, 64, GL_MAP_READ_BIT) as *const u8;
        assert!(!p.is_null(), "map {name}");
        let v = core::slice::from_raw_parts(p, 64).to_vec();
        glUnmapBuffer(GL_ARRAY_BUFFER);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        v
    }
}

#[test]
fn vertex_array_keeps_a_deleted_buffer_when_the_name_is_bound_again() {
    let _g = context();
    let x = gen_buffer(0xcd);
    let (mut va, mut other) = (0, 0);
    // SAFETY: plain GL calls.
    unsafe {
        glGenVertexArrays(1, &mut va);
        glGenVertexArrays(1, &mut other);
        glBindVertexArray(va);
        glBindBuffer(GL_ARRAY_BUFFER, x);
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindVertexArray(other);
        glDeleteBuffers(1, &x);
        assert_eq!(glIsBuffer(x), GL_FALSE_B, "the name is gone");
    }
    specify(x, 0xab); // glBindBuffer without glGenBuffers
    let mut held: GLint = -1;
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glIsBuffer(x), GL_TRUE_B, "a new buffer");
        glBindVertexArray(va);
        glGetVertexAttribiv(0, GL_VERTEX_ATTRIB_ARRAY_BUFFER_BINDING, &mut held);
    }
    assert!(held != 0 && held as GLuint != x, "the vertex array object keeps the old buffer, not {x}: {held}");
    assert_eq!(contents(x), vec![0xab; 64]);
    // the last holder lets go: the live buffer stays
    // SAFETY: plain GL calls.
    unsafe {
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, core::ptr::null());
        glBindVertexArray(0);
        glDeleteVertexArrays(1, &va);
        glDeleteVertexArrays(1, &other);
        assert_eq!(glIsBuffer(x), GL_TRUE_B);
    }
    assert_eq!(contents(x), vec![0xab; 64]);
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: one name.
    unsafe { glDeleteBuffers(1, &x) };
}

#[cfg(feature = "shaderc-online")]
mod capture {
    use super::*;

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

    fn program() {
        // SAFETY: plain GL calls; the names outlive the link.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
            glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
            let name = c"v";
            let ptrs: [*const c_char; 1] = [name.as_ptr()];
            glTransformFeedbackVaryings(p, 1, ptrs.as_ptr(), GL_INTERLEAVED_ATTRIBS);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1);
            glUseProgram(p);
        }
    }

    /// A capture of the three vertices `input` through `tf`.
    fn capture(tf: GLuint, input: &[f32; 12]) {
        // SAFETY: plain GL calls; the client array outlives the draw.
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, input.as_ptr().cast());
            glBeginTransformFeedback(GL_TRIANGLES);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glEndTransformFeedback();
            glDisableVertexAttribArray(0);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            assert_eq!(err(), GL_NO_ERROR);
        }
    }

    /// The 64-byte buffer storages that hold `want` over their first 48 bytes
    /// and the old buffer's 0xcd after them (the client array's staging copy
    /// is not one).
    fn storages_with(want: &[f32; 12]) -> usize {
        let allocs = ALLOCS.lock().unwrap_or_else(|e| e.into_inner()).clone();
        allocs
            .iter()
            .filter(|&&(a, n)| {
                // SAFETY: the address is a live leaked allocation of `n` >= 64 bytes.
                n >= 64
                    && unsafe { core::slice::from_raw_parts(a as *const f32, 12) } == want
                    && unsafe { core::slice::from_raw_parts((a + 48) as *const u8, 16) } == [0xcd; 16]
            })
            .count()
    }

    #[test]
    fn capture_through_a_transform_feedback_object_leaves_the_new_buffer_alone() {
        let _g = context();
        program();
        let input: [f32; 12] = [1.5, 2.5, 3.5, 4.5, -1.5, -2.5, -3.5, -4.5, 0.75, 0.5, 0.25, 0.125];
        let x = gen_buffer(0xcd);
        let mut tf = 0;
        // SAFETY: plain GL calls.
        unsafe {
            glGenTransformFeedbacks(1, &mut tf);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, x);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            glDeleteBuffers(1, &x);
        }
        assert_eq!(storages_with(&input), 0);
        specify(x, 0xab); // the name again, no glGenBuffers
        capture(tf, &input);
        assert_eq!(contents(x), vec![0xab; 64], "the new buffer is untouched");
        assert_eq!(storages_with(&input), 1, "the capture went to the old storage");
        let mut held: GLint = -1;
        // SAFETY: plain GL calls.
        unsafe {
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, tf);
            glGetIntegeri_v(GL_TRANSFORM_FEEDBACK_BUFFER_BINDING, 0, &mut held);
            assert!(held != 0 && held as GLuint != x, "tf keeps the old buffer, not {x}: {held}");
            // the last holder lets go: the live buffer stays
            glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, 0);
            glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
            glDeleteTransformFeedbacks(1, &tf);
            assert_eq!(glIsBuffer(x), GL_TRUE_B);
        }
        assert_eq!(contents(x), vec![0xab; 64]);
        assert_eq!(err(), GL_NO_ERROR);
    }
}
