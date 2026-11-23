// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The per-instance split of an instanced draw (a back end whose
//! `max_instance_divisor` is smaller than an array's divisor) stages the raw
//! integer dwords of `glVertexAttribIPointer` arrays and restores the typed
//! current vertex value afterwards (`draw_instanced` in `gl/draw.rs`, the
//! `current_attrib_type` of T1).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_instanced_split_typed
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
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
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}

/// The dwords of each constant (zero-stride 16-byte fetch) of a recorded draw.
static DRAWS: Mutex<Vec<Vec<[u32; 4]>>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let consts = d.attribs[..(d.num_attribs as usize).min(d.attribs.len())]
        .iter()
        .filter(|a| a.stride == 0 && a.cpu_bytes == 16 && !a.cpu.is_null())
        .map(|a| {
            let mut v = [0u32; 4];
            // SAFETY: `cpu` holds `cpu_bytes` bytes for the duration of the call.
            core::ptr::copy_nonoverlapping(a.cpu as *const u8, v.as_mut_ptr() as *mut u8, 16);
            v
        })
        .collect();
    DRAWS.lock().unwrap().push(consts);
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

const fn backend(name: &'static core::ffi::CStr, max_instance_divisor: u32) -> YakoglBackend {
    YakoglBackend {
        name: name.as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align: 0,
        compressed_blocks_in_rows: false,
        native_layered_textures: false,
        front_facing_shader_variant: false,
        etc1_decoded_at_upload: false,
        init: Some(b_ok),
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
        max_instance_divisor,
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
    }
}

/// Divisor 1 only: the PowerVR and the GC7000UL.
static DIVISOR_1: YakoglBackend = backend(c"es3-split-typed-div1", 1);

static LOCK: Mutex<()> = Mutex::new(());

/// The one context, with `be` as its back end; the tests take turns.
fn context(be: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = be;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        ONCE.call_once(|| assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null()));
    }
    DRAWS.lock().unwrap().clear();
    g
}

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

fn buffer(target: GLenum, bytes: &[u8]) -> GLuint {
    let mut b = 0;
    // SAFETY: plain GL calls; `bytes` outlives the call.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(target, b);
        glBufferData(target, bytes.len() as isize, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
    }
    b
}

fn bytes_of<T: Copy>(v: &[T]) -> &[u8] {
    // SAFETY: plain-old-data slices of f32 / u16.
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, core::mem::size_of_val(v)) }
}

const TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
/// The first error; the queue is empty again afterwards.
fn error() -> GLenum {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    e
}

const VS: &str = "#version 300 es
layout(location = 0) in vec4 a_pos;
layout(location = 1) in ivec4 a_int;
layout(location = 2) in uvec4 a_uint;
layout(location = 3) in vec4 a_flt;
flat out ivec4 vi;
flat out uvec4 vu;
out vec4 vf;
void main() { gl_Position = a_pos; vi = a_int; vu = a_uint; vf = a_flt; }
";
const FS: &str = "#version 300 es
precision mediump float;
flat in ivec4 vi;
flat in uvec4 vu;
in vec4 vf;
out vec4 o;
void main() { o = vec4(vi) + vec4(vu) + vf; }
";

/// Bit patterns that must survive: negatives, above 2^24, float NaNs.
fn ints(i: usize) -> [i32; 4] {
    [
        [-1, i32::MIN, 0x7fc0_0001][i],
        [0x7fff_ffff, -0x0100_0001, 0xffc0_0000u32 as i32][i],
        [0, 0x7f80_0000, -3][i],
        [i as i32, -7, 0x0200_0003][i],
    ]
}
fn uints(i: usize) -> [u32; 4] {
    [
        [0xffff_ffff, 0x7fc0_0001, 0x8000_0000][i],
        [0xffc0_0001, 0x0100_0003, 0x7f80_0001][i],
        [0x7fc0_0000, 0xdead_beef, 0xffff_fffe][i],
        [i as u32, 0x8000_0001, 0xff80_0000][i],
    ]
}
fn floats(i: usize) -> [f32; 4] {
    [i as f32 + 0.5, -1.25, 1.0e10, 3.0]
}

#[test]
#[cfg_attr(
    feature = "gpu-mali",
    ignore = "Mali refuses a fragment shader that mixes highp and mediump floats (not lowered yet)"
)]
fn per_instance_split_stages_raw_integer_dwords_and_restores_typed_current_values() {
    let _g = context(&DIVISOR_1);
    const N: usize = 3;
    let int_data: Vec<i32> = (0..N).flat_map(ints).collect();
    let uint_data: Vec<u32> = (0..N).flat_map(uints).collect();
    let flt_data: Vec<f32> = (0..N.div_ceil(2)).flat_map(floats).collect();
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program did not link");
        glUseProgram(p);
        let bufs = [
            buffer(GL_ARRAY_BUFFER, bytes_of(&TRI)),
            buffer(GL_ARRAY_BUFFER, bytes_of(&int_data)),
            buffer(GL_ARRAY_BUFFER, bytes_of(&uint_data)),
            buffer(GL_ARRAY_BUFFER, bytes_of(&flt_data)),
        ];
        glBindBuffer(GL_ARRAY_BUFFER, bufs[0]);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glBindBuffer(GL_ARRAY_BUFFER, bufs[1]);
        glVertexAttribIPointer(1, 4, GL_INT, 0, core::ptr::null());
        glBindBuffer(GL_ARRAY_BUFFER, bufs[2]);
        glVertexAttribIPointer(2, 4, GL_UNSIGNED_INT, 0, core::ptr::null());
        glBindBuffer(GL_ARRAY_BUFFER, bufs[3]);
        glVertexAttribPointer(3, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        for loc in 0..4 {
            glEnableVertexAttribArray(loc);
            if loc > 0 {
                glVertexAttribDivisor(loc, if loc == 3 { 2 } else { 1 });
            }
        }
        // The typed current values before the draw.
        glVertexAttribI4i(1, -5, 6, i32::MIN, 0x7fc0_0001);
        glVertexAttribI4ui(2, 0xffff_ffff, 0x7fc0_0001, 0x0100_0001, 9);
        glVertexAttrib4f(3, 0.5, -2.0, 1.0e10, 7.0);
        assert_eq!(error(), GL_NO_ERROR);
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, N as GLsizei);
        assert_eq!(error(), GL_NO_ERROR);

        // (a) each per-instance draw staged exactly the array's dwords
        let draws = DRAWS.lock().unwrap().clone();
        assert_eq!(draws.len(), N, "one draw per instance");
        for (i, d) in draws.iter().enumerate() {
            let want = [ints(i).map(|v| v as u32), uints(i), floats(i / 2).map(f32::to_bits)];
            assert_eq!(d, &want, "instance {i}: staged dwords of the int, uint and float arrays");
        }

        // (b) the typed current values and the arrays are back
        let (mut iv, mut uv, mut fv) = ([0i32; 4], [0u32; 4], [0f32; 4]);
        glGetVertexAttribIiv(1, GL_CURRENT_VERTEX_ATTRIB, iv.as_mut_ptr());
        assert_eq!(iv, [-5, 6, i32::MIN, 0x7fc0_0001]);
        glGetVertexAttribIuiv(2, GL_CURRENT_VERTEX_ATTRIB, uv.as_mut_ptr());
        assert_eq!(uv, [0xffff_ffff, 0x7fc0_0001, 0x0100_0001, 9]);
        glGetVertexAttribfv(3, GL_CURRENT_VERTEX_ATTRIB, fv.as_mut_ptr());
        assert_eq!(fv, [0.5, -2.0, 1.0e10, 7.0]);
        for loc in 0..4 {
            let (mut en, mut div) = (0, 0);
            glGetVertexAttribiv(loc, GL_VERTEX_ATTRIB_ARRAY_ENABLED, &mut en);
            glGetVertexAttribiv(loc, GL_VERTEX_ATTRIB_ARRAY_DIVISOR, &mut div);
            assert_eq!((en, div), (1, [0, 1, 1, 2][loc as usize]), "attrib {loc}");
        }
        assert_eq!(error(), GL_NO_ERROR);

        for loc in 0..4 {
            glDisableVertexAttribArray(loc);
            glVertexAttribDivisor(loc, 0);
        }
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteBuffers(4, bufs.as_ptr());
    }
}
