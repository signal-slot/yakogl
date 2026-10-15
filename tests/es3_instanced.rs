// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 instanced draws on the host (item es3-instanced; the six AM62P
//! dEQP-GLES3 `functional.instanced.{draw_arrays_instanced,
//! draw_elements_instanced}.mixed.{2,4,20}_instances` cases,
//! docs/es3-cts-failures.md Appendix A, docs/handoff/20261014-es3-instanced.md).
//!
//! Those cases draw with one array at divisor 0, one at 1 and one at 2, and
//! the vertex shader reads `gl_InstanceID`. A back end that takes divisor 1
//! only (`max_instance_divisor` 1: the PowerVR and the GC7000UL) makes the GL
//! core draw instance by instance; this file pins what the core hands the
//! back end then (each instance's value of each divisor array as a constant,
//! and `GlesDraw::instance_id`), and what it hands a back end that takes any
//! divisor (one draw, `instance_count` instances, the divisor per array). The
//! back ends' refusal of an instance-by-instance draw that reads
//! `gl_InstanceID` (`pvr/backend.rs`, `vivante/backend.rs`) is not part of
//! the GL core and is not tested here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_instanced
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

/// One attribute of a recorded draw.
#[derive(Debug, Clone, PartialEq)]
struct Fetch {
    per_instance: bool,
    divisor: u16,
    /// The four floats of a constant (zero-stride 16-byte fetch), if it is one.
    constant: Option<[f32; 4]>,
}

#[derive(Debug, Clone)]
struct Rec {
    index_count: u32,
    instance_count: u32,
    instance_id: u32,
    attribs: Vec<Fetch>,
}

static DRAWS: Mutex<Vec<Rec>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let attribs = d.attribs[..(d.num_attribs as usize).min(d.attribs.len())]
        .iter()
        .map(|a| {
            let constant = (a.stride == 0 && a.cpu_bytes == 16 && !a.cpu.is_null()).then(|| {
                let mut v = [0f32; 4];
                // SAFETY: `cpu` holds `cpu_bytes` bytes for the duration of the call.
                core::ptr::copy_nonoverlapping(a.cpu as *const u8, v.as_mut_ptr() as *mut u8, 16);
                v
            });
            Fetch { per_instance: a.per_instance != 0, divisor: a.divisor, constant }
        })
        .collect();
    DRAWS.lock().unwrap().push(Rec {
        index_count: d.index_count,
        instance_count: d.instance_count,
        instance_id: d.instance_id,
        attribs,
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
    lock: None,
    unlock: None,
    thread_id: None,
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
    }
}

/// Divisor 1 only: the PowerVR and the GC7000UL.
static DIVISOR_1: YakoglBackend = backend(c"es3-instanced-div1", 1);
/// Any divisor: the V3D.
static DIVISOR_ANY: YakoglBackend = backend(c"es3-instanced-any", 0xffff);

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

/// The shape of the dEQP `mixed` cases: a per-vertex position, an array at
/// divisor 1, an array at divisor 2, and a shader that reads `gl_InstanceID`.
const VS: &str = "#version 300 es
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_each;
layout(location = 2) in vec4 a_pair;
out vec4 v;
void main() { gl_Position = a_pos; v = a_each + a_pair + vec4(float(gl_InstanceID)); }
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

/// Instance `i`'s value of the divisor-1 array.
fn each(i: usize) -> [f32; 4] {
    [i as f32, 10.0 + i as f32, 0.5, 1.0]
}
/// Value `j` of the divisor-2 array (instances 2j and 2j + 1).
fn pair(j: usize) -> [f32; 4] {
    [100.0 + j as f32, 200.0 + j as f32, 0.25, 1.0]
}

/// Links the mixed program and sets the three arrays up for `instances`
/// instances. Returns the program and the buffers to delete.
fn mixed_setup(instances: usize) -> (GLuint, Vec<GLuint>) {
    let each_data: Vec<f32> = (0..instances).flat_map(each).collect();
    let pair_data: Vec<f32> = (0..instances.div_ceil(2)).flat_map(pair).collect();
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
        let bufs = vec![
            buffer(GL_ARRAY_BUFFER, bytes_of(&TRI)),
            buffer(GL_ARRAY_BUFFER, bytes_of(&each_data)),
            buffer(GL_ARRAY_BUFFER, bytes_of(&pair_data)),
        ];
        for (loc, (b, div)) in [(bufs[0], 0), (bufs[1], 1), (bufs[2], 2)].into_iter().enumerate() {
            glBindBuffer(GL_ARRAY_BUFFER, b);
            glVertexAttribPointer(loc as GLuint, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(loc as GLuint);
            glVertexAttribDivisor(loc as GLuint, div);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        (p, bufs)
    }
}

fn teardown(p: GLuint, bufs: &[GLuint]) {
    // SAFETY: plain GL calls.
    unsafe {
        for loc in 0..3 {
            glDisableVertexAttribArray(loc);
            glVertexAttribDivisor(loc, 0);
        }
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
    }
}

/// The first error; the queue is empty again afterwards.
fn error() -> GLenum {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    e
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Arrays,
    Elements,
}

fn draw(kind: Kind, instances: usize) {
    // SAFETY: plain GL calls; the index buffer is bound for the call.
    unsafe {
        match kind {
            Kind::Arrays => glDrawArraysInstanced(GL_TRIANGLES, 0, 3, instances as GLsizei),
            Kind::Elements => {
                let idx: [u16; 3] = [0, 1, 2];
                let b = buffer(GL_ELEMENT_ARRAY_BUFFER, bytes_of(&idx));
                glDrawElementsInstanced(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, core::ptr::null(), instances as GLsizei);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
                glDeleteBuffers(1, &b);
            }
        }
    }
}

/// A back end that takes divisor 1 only: one draw per instance. Instance `i`
/// reads `gl_InstanceID` = i (`instance_id`), the divisor-1 array's element
/// i and the divisor-2 array's element i / 2, both as constants; the position
/// array stays an array. The 2, 4 and 20 instance cases of dEQP, drawn with
/// both entry points.
#[test]
fn divisor_1_backend_draws_each_instance_with_its_id_and_values() {
    for kind in [Kind::Arrays, Kind::Elements] {
        for n in [2usize, 4, 20] {
            let _g = context(&DIVISOR_1);
            let (p, bufs) = mixed_setup(n);
            draw(kind, n);
            assert_eq!(error(), GL_NO_ERROR, "{kind:?} x{n}");
            let draws = DRAWS.lock().unwrap().clone();
            assert_eq!(draws.len(), n, "{kind:?} x{n}: one draw per instance");
            for (i, d) in draws.iter().enumerate() {
                let what = format!("{kind:?} x{n} instance {i}");
                assert_eq!(d.index_count, 3, "{what}");
                assert_eq!(d.instance_count, 1, "{what}");
                assert_eq!(d.instance_id, i as u32, "{what}: gl_InstanceID");
                let consts: Vec<[f32; 4]> = d.attribs.iter().filter_map(|a| a.constant).collect();
                assert_eq!(consts, [each(i), pair(i / 2)], "{what}: divisor 1 then divisor 2 value");
                assert!(d.attribs.iter().all(|a| !a.per_instance), "{what}: nothing fetched per instance");
            }
            teardown(p, &bufs);
        }
    }
}

/// A back end that takes any divisor: one draw carrying the instance count,
/// the two instanced arrays fetched with their own divisors, gl_InstanceID
/// left to the hardware (`instance_id` 0).
#[test]
fn any_divisor_backend_gets_one_draw_with_the_divisors() {
    for kind in [Kind::Arrays, Kind::Elements] {
        for n in [2usize, 4, 20] {
            let _g = context(&DIVISOR_ANY);
            let (p, bufs) = mixed_setup(n);
            draw(kind, n);
            assert_eq!(error(), GL_NO_ERROR, "{kind:?} x{n}");
            let draws = DRAWS.lock().unwrap().clone();
            assert_eq!(draws.len(), 1, "{kind:?} x{n}");
            let d = &draws[0];
            assert_eq!((d.index_count, d.instance_count, d.instance_id), (3, n as u32, 0), "{kind:?} x{n}");
            let mut divisors: Vec<u16> = d.attribs.iter().filter(|a| a.per_instance).map(|a| a.divisor).collect();
            divisors.sort_unstable();
            assert_eq!(divisors, [1, 2], "{kind:?} x{n}");
            assert!(d.attribs.iter().all(|a| a.constant.is_none()), "{kind:?} x{n}: no constants");
            teardown(p, &bufs);
        }
    }
}

/// Instancing leaves no state behind: the arrays stay enabled with their
/// divisors, and a plain draw afterwards is not an instance.
#[test]
fn instanced_draw_leaves_the_arrays_and_the_next_draw_alone() {
    let _g = context(&DIVISOR_1);
    let (p, bufs) = mixed_setup(4);
    draw(Kind::Arrays, 4);
    // SAFETY: plain GL calls.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let (mut en, mut div) = (0, 0);
        glGetVertexAttribiv(2, GL_VERTEX_ATTRIB_ARRAY_ENABLED, &mut en);
        glGetVertexAttribiv(2, GL_VERTEX_ATTRIB_ARRAY_DIVISOR, &mut div);
        assert_eq!((en, div), (1, 2));
    }
    let ids: Vec<u32> = DRAWS.lock().unwrap().iter().map(|d| d.instance_id).collect();
    assert_eq!(ids, [0, 1, 2, 3, 0]);
    teardown(p, &bufs);
}
