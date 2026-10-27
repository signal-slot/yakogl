// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Client-side vertex arrays (glmark2's build:use-vbo=false): `convert_attrib`
//! copies a float array that already is the layout the back end fetches row
//! by row (one copy when tightly packed, one per vertex otherwise) instead of
//! converting each component.
//!
//! The stand-in back end records, at draw time, the bytes of the attribute's
//! arena scratch (`GlesDraw::attribs[0].cpu`). Each case is drawn with the
//! fast path on and off (`CLIENT_COPY_FAST`): the bytes must be equal, and
//! equal to the rows the GL gives. A type that needs converting (normalized
//! ubyte, short) or an array narrower than the input must not take the fast
//! path (`CLIENT_COPY_FAST_ARRAYS` does not move). The timing test prints;
//! it asserts nothing about time.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test gl_client_array_copy -- --nocapture
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
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

/* ---- the model: what the back end got of each draw ------------------------ */

static ARENA: Mutex<Vec<Vec<u8>>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn m_flush(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_invd(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
/// Keep the arena bytes of the draw: the scratch is rewritten by the next.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let a = &d.attribs[0];
    assert!(d.num_attribs == 1 && !a.cpu.is_null() && a.cpu_bytes != 0, "the client array is staged in the arena");
    let bytes = core::slice::from_raw_parts(a.cpu as *const u8, a.cpu_bytes as usize).to_vec();
    ARENA.lock().unwrap_or_else(|e| e.into_inner()).push(bytes);
    0
}
unsafe extern "C" fn b_flush() -> i32 {
    0
}
unsafe extern "C" fn b_finish() -> i32 {
    0
}
unsafe extern "C" fn b_kick_id() -> u32 {
    1
}
unsafe extern "C" fn b_kick_pending(_id: u32) -> bool {
    false
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
    cache_flush: Some(m_flush),
    cache_invd: Some(m_invd),
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
    name: c"gl-client-array-copy".as_ptr() as *const c_char,
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
    flush: Some(b_flush),
    finish: Some(b_finish),
    finish_previous: None,
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static PROGRAM: OnceLock<GLuint> = OnceLock::new();

const VS: &str = "attribute vec3 a;\nvoid main() { gl_Position = vec4(a, 1.0); }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";

fn program() -> GLuint {
    let shader = |kind: GLenum, text: &str| -> GLuint {
        let text = CString::new(text).unwrap();
        // SAFETY: plain GL calls; one NUL-terminated source.
        unsafe {
            let s = glCreateShader(kind);
            let p: [*const c_char; 1] = [text.as_ptr()];
            glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "{text:?} did not compile");
            s
        }
    };
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"a".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program did not link");
        p
    }
}

/// The one context and its program.
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
    let prog = *PROGRAM.get_or_init(program);
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        glUseProgram(prog);
    }
    ARENA.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

/* ---- the cases -------------------------------------------------------------- */

const VERTS: usize = 61;

#[derive(Clone, Copy)]
struct Layout {
    type_: GLenum,
    normalized: bool,
    /// Bytes of one component.
    tsz: usize,
    /// Components of the array.
    size: usize,
    /// Distance between vertices; 0 = tightly packed.
    stride: usize,
    /// Byte offset of the attribute in its vertex (interleaved).
    offset: usize,
}

impl Layout {
    fn elem(&self) -> usize {
        self.size * self.tsz
    }
    fn step(&self) -> usize {
        if self.stride == 0 {
            self.elem()
        } else {
            self.stride
        }
    }
    fn bytes(&self) -> usize {
        self.offset + (VERTS - 1) * self.step() + self.elem()
    }
}

fn pattern(n: usize) -> Vec<u8> {
    (0..n).map(|i| (i.wrapping_mul(37) ^ (i >> 3)) as u8).collect()
}

/// Finite floats, so no bit pattern is a NaN the comparison could trip on.
fn float_pattern(n: usize) -> Vec<u8> {
    (0..n / 4).flat_map(|i| (i as f32 * 0.37 - 5.0).to_ne_bytes()).collect()
}

/// Draw `VERTS` vertices of the client array `l` in `data`; returns the arena
/// bytes and whether the fast path took the array.
fn arena(l: Layout, data: &[u8], fast: bool) -> (Vec<u8>, bool) {
    assert!(data.len() >= l.bytes());
    CLIENT_COPY_FAST.store(fast, Ordering::Relaxed);
    let before = CLIENT_COPY_FAST_ARRAYS.load(Ordering::Relaxed);
    // SAFETY: plain GL calls; `data` outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(
            0,
            l.size as GLint,
            l.type_,
            if l.normalized { GL_TRUE_B } else { GL_FALSE_B },
            l.stride as GLsizei,
            data.as_ptr().add(l.offset) as *const c_void,
        );
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, VERTS as GLsizei);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR);
        glDisableVertexAttribArray(0);
    }
    CLIENT_COPY_FAST.store(true, Ordering::Relaxed);
    let took = CLIENT_COPY_FAST_ARRAYS.load(Ordering::Relaxed) != before;
    (ARENA.lock().unwrap_or_else(|e| e.into_inner()).pop().expect("the draw reached the back end"), took)
}

/// Both paths write the same bytes, and they are `expect`'s rows (vec3 of
/// float32 per vertex, given the vertex's source bytes).
fn check(l: Layout, want_fast: bool, expect: impl Fn(&[u8]) -> [f32; 3]) {
    let _g = context();
    let data = if l.type_ == GL_FLOAT { float_pattern(l.bytes() + 16) } else { pattern(l.bytes() + 16) };
    let (a, took_a) = arena(l, &data, true);
    let (b, took_b) = arena(l, &data, false);
    assert!(!took_b, "the switch is off");
    assert_eq!(took_a, want_fast, "fast path taken");
    assert_eq!(a.len(), VERTS * 12);
    assert_eq!(a, b, "the arena bytes of both paths");
    for v in 0..VERTS {
        let want = expect(&data[l.offset + v * l.step()..]);
        let got: Vec<u32> =
            a[v * 12..v * 12 + 12].chunks(4).map(|c| u32::from_ne_bytes(c.try_into().unwrap())).collect();
        assert_eq!(got, want.map(f32::to_bits), "vertex {v}");
    }
}

fn as_floats(b: &[u8]) -> [f32; 3] {
    core::array::from_fn(|i| f32::from_ne_bytes(b[i * 4..i * 4 + 4].try_into().unwrap()))
}

fn float3(size: usize, stride: usize, offset: usize) -> Layout {
    Layout { type_: GL_FLOAT, normalized: false, tsz: 4, size, stride, offset }
}

#[test]
fn float_tightly_packed() {
    check(float3(3, 0, 0), true, as_floats);
    check(float3(3, 12, 0), true, as_floats);
}

#[test]
fn float_strided() {
    check(float3(3, 24, 0), true, as_floats);
    check(float3(3, 13, 0), true, as_floats); /* an unaligned stride */
}

#[test]
fn float_interleaved() {
    check(float3(3, 32, 0), true, as_floats);
    check(float3(3, 32, 12), true, as_floats);
}

#[test]
fn float4_array_into_vec3_input_takes_the_first_three() {
    check(float3(4, 0, 0), true, as_floats);
    check(float3(4, 28, 4), true, as_floats);
}

#[test]
fn narrower_float_array_takes_the_slow_path() {
    check(float3(2, 0, 0), false, |b| {
        let f = as_floats(b);
        [f[0], f[1], 0.0]
    });
}

fn ubyte_n(stride: usize, offset: usize) -> Layout {
    Layout { type_: GL_UNSIGNED_BYTE, normalized: true, tsz: 1, size: 3, stride, offset }
}

fn short(stride: usize, offset: usize) -> Layout {
    Layout { type_: GL_SHORT, normalized: false, tsz: 2, size: 3, stride, offset }
}

#[test]
fn normalized_ubyte_is_converted() {
    let e = |b: &[u8]| core::array::from_fn(|i| b[i] as f32 / 255.0);
    check(ubyte_n(0, 0), false, e);
    check(ubyte_n(4, 0), false, e);
    check(ubyte_n(16, 4), false, e);
}

#[test]
fn short_is_converted() {
    let e = |b: &[u8]| core::array::from_fn(|i| i16::from_ne_bytes([b[i * 2], b[i * 2 + 1]]) as f32);
    check(short(0, 0), false, e);
    check(short(8, 0), false, e);
    check(short(20, 2), false, e);
}

/// Client memory changes after the draw: the arena keeps the old bytes.
#[test]
fn the_arena_keeps_its_copy() {
    let _g = context();
    let l = float3(3, 0, 0);
    let mut data = float_pattern(l.bytes() + 16);
    let (a, took) = arena(l, &data, true);
    assert!(took);
    data.fill(0);
    assert_eq!(a, float_pattern(l.bytes() + 16)[..VERTS * 12]);
}

/// Prints the time of a draw (the copy and the rest of the GL call) with the
/// fast path on and off, strided and tight. Not asserted.
#[test]
fn timing() {
    let _g = context();
    const N: u32 = 3000;
    for l in [float3(3, 0, 0), float3(3, 32, 0)] {
        let data = float_pattern(l.bytes() + 16);
        for (name, fast) in [("fast", true), ("slow", false), ("fast", true), ("slow", false)] {
            let t = std::time::Instant::now();
            for _ in 0..N {
                arena(l, &data, fast);
            }
            println!(
                "client array {} vertices, stride {}: {name} path {:.2} us a draw",
                VERTS,
                l.step(),
                t.elapsed().as_secs_f64() * 1e6 / N as f64
            );
        }
    }
}
