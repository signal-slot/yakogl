// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GL core splits an instanced draw only for a divisor above the back
//! end's `max_instance_divisor` (`draw_instanced`, src/gl/draw.rs). A back
//! end that takes 0xffff (the GC7000UL since 6c4759ea, the V3D, and the
//! PowerVR once P3 lands) gets one draw whatever mix of divisors 0..=0xffff
//! the arrays have; a divisor of 0x10000 is above the cap, so the core draws
//! instance by instance, each draw with its `instance_id` and every
//! per-instance array's value for that instance as a constant (the
//! documented fallback).
//!
//! The back end is `yakogl_backend_vivante` with `max_instance_divisor`
//! 0xffff and its `draw` replaced by a recorder (struct update from the
//! public table, so no `YakoglBackend` field is listed here).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_instanced_divisor_cap
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

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
use yakogl::vivante::backend::yakogl_backend_vivante;

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

/// One recorded draw: its instance count and id, the (divisor) of every array
/// fetched per instance, and how many arrays were fed as constants.
#[derive(Debug, Clone)]
struct Rec {
    instance_count: u32,
    instance_id: u32,
    fetched: Vec<u16>,
    constants: usize,
}

static DRAWS: Mutex<Vec<Rec>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let attribs = &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())];
    let mut fetched: Vec<u16> = attribs.iter().filter(|a| a.per_instance != 0).map(|a| a.divisor).collect();
    fetched.sort_unstable();
    DRAWS.lock().unwrap().push(Rec {
        instance_count: d.instance_count,
        instance_id: d.instance_id,
        fetched,
        constants: attribs.iter().filter(|a| a.stride == 0 && a.cpu_bytes == 16).count(),
    });
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

/// A table that takes any divisor up to 0xffff and records its draws.
static CAP_0XFFFF: YakoglBackend = YakoglBackend {
    max_instance_divisor: 0xffff,
    init: Some(b_ok),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    heap_base: Some(b_heap_base),
    ..yakogl_backend_vivante
};

static LOCK: Mutex<()> = Mutex::new(());

fn context() -> MutexGuard<'static, ()> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &CAP_0XFFFF;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        ONCE.call_once(|| assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null()));
    }
    DRAWS.lock().unwrap().clear();
    g
}

const VS: &str = "#version 300 es
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_1;
layout(location = 2) in vec4 a_2;
layout(location = 3) in vec4 a_3;
out vec4 v;
void main() { gl_Position = a_pos; v = a_1 + a_2 + a_3; }
";
const FS: &str = "#version 300 es
precision mediump float;
in vec4 v;
out vec4 o;
void main() { o = v; }
";

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

fn buffer(bytes: &[u8]) -> GLuint {
    let mut b = 0;
    // SAFETY: plain GL calls; `bytes` outlives the call.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, bytes.len() as isize, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
    }
    b
}

fn bytes_of(v: &[f32]) -> &[u8] {
    // SAFETY: a plain-old-data slice of f32.
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, core::mem::size_of_val(v)) }
}

const TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];

/// Draws `instances` instances of a triangle whose arrays 1..=3 have the
/// divisors `divs` (array 0, the position, has divisor 0); returns the
/// recorded draws. Every array holds enough elements for any instance.
fn run(divs: [GLuint; 3], instances: usize) -> Vec<Rec> {
    let _g = context();
    let data = vec![0.5f32; 4 * (instances + 1)];
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
        let mut bufs = vec![buffer(bytes_of(&TRI))];
        for _ in 0..3 {
            bufs.push(buffer(bytes_of(&data)));
        }
        for (loc, b) in bufs.iter().enumerate() {
            glBindBuffer(GL_ARRAY_BUFFER, *b);
            glVertexAttribPointer(loc as GLuint, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(loc as GLuint);
            glVertexAttribDivisor(loc as GLuint, if loc == 0 { 0 } else { divs[loc - 1] });
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, instances as GLsizei);
        assert_eq!(glGetError(), GL_NO_ERROR, "{divs:?} x{instances}");
        for loc in 0..4 {
            glDisableVertexAttribArray(loc);
            glVertexAttribDivisor(loc, 0);
        }
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
    }
    let draws = DRAWS.lock().unwrap().clone();
    draws
}

/// Divisors 0 (position), 1, 2 and 3 on one draw: one draw carrying all the
/// instances, the three arrays fetched with their own divisors.
#[test]
fn mixed_divisors_0_1_2_3_are_one_draw() {
    let draws = run([1, 2, 3], 7);
    assert_eq!(draws.len(), 1, "{draws:?}");
    let d = &draws[0];
    assert_eq!((d.instance_count, d.instance_id), (7, 0));
    assert_eq!(d.fetched, [1, 2, 3]);
    assert_eq!(d.constants, 0);
}

/// 0xffff is the cap itself: still one draw.
#[test]
fn divisor_0xffff_is_one_draw() {
    let draws = run([1, 3, 0xffff], 5);
    assert_eq!(draws.len(), 1, "{draws:?}");
    assert_eq!(draws[0].instance_count, 5);
    assert_eq!(draws[0].fetched, [1, 3, 0xffff]);
    assert_eq!(draws[0].constants, 0);
}

/// 0x10000 is above the cap: the core draws instance by instance, every
/// per-instance array as a constant (divisor 1 and 2 included), the
/// instance's number in `instance_id`.
#[test]
fn divisor_0x10000_splits_into_one_draw_per_instance() {
    let n = 5;
    let draws = run([1, 2, 0x10000], n);
    assert_eq!(draws.len(), n, "{draws:?}");
    for (i, d) in draws.iter().enumerate() {
        assert_eq!((d.instance_count, d.instance_id), (1, i as u32), "instance {i}");
        assert!(d.fetched.is_empty(), "instance {i}: {d:?}");
        assert_eq!(d.constants, 3, "instance {i}");
    }
}

/// A single instance is one draw whatever the divisors.
#[test]
fn one_instance_is_one_draw_even_above_the_cap() {
    let draws = run([1, 2, 0x10000], 1);
    assert_eq!(draws.len(), 1, "{draws:?}");
    assert_eq!(draws[0].instance_count, 1);
}
