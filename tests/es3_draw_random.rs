// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 `draw.random` regression guard on the host (plan #343, item
//! fix-draw-random; docs/notes/es3-draw-random.md): the 14 AM62P
//! dEQP-GLES3.functional.draw.random failures at `ad2b5ed` all came from the
//! GL core's vertex-array handling and were fixed by `6c9606f6`. This file
//! draws one miniature per distinct draw shape the note lists, through the
//! public GL API against a recording stand-in back end, and pins that the
//! draw raises no error and that the rows the back end gets are the
//! converted ones:
//!
//! * case 118 (class C): `glDrawElementsInstanced`, a 2_10_10_10_REV x4 array
//!   in an 8-byte buffer feeding a `vec2`, indices {0, 1} from a buffer, two
//!   instances. It was GL_INVALID_OPERATION ("the array runs past the VBO").
//! * classes A (cases 19, 71, 94, 186): `glVertexAttribPointer` with
//!   GL_INT / GL_UNSIGNED_INT in a buffer is converted to float.
//! * class B (cases 36, 42, 207, 208): an integer array narrower than the
//!   `ivec4` / `uvec4` input fills w with the integer 1.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_draw_random
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
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

/* ---- a context on the host ------------------------------------------------ */

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

/// What the back end was handed for one attribute of a draw.
#[derive(Clone, Debug)]
struct Fetch {
    /// Staged by the GL core: the converted rows (one per vertex, `stride`
    /// apart, as dwords). Empty when the array is fetched in place.
    rows: Vec<u32>,
    /// Fetched in place: the dwords at `buffer_gpu + offset` for the first
    /// vertex (the host back end's GPU address is the CPU address).
    in_place: Vec<u32>,
}

static FETCHES: Mutex<Vec<Vec<Fetch>>> = Mutex::new(Vec::new());
/// (index_count, instance_count) of every draw.
static COUNTS: Mutex<Vec<(u32, u32)>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let mut draw = Vec::new();
    for a in &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())] {
        let mut f = Fetch { rows: Vec::new(), in_place: Vec::new() };
        if a.cpu_bytes != 0 && !a.cpu.is_null() {
            let mut w = vec![0u32; a.cpu_bytes as usize / 4];
            // SAFETY: the contract says `cpu` holds `cpu_bytes` bytes for the
            // duration of the call.
            core::ptr::copy_nonoverlapping(a.cpu as *const u8, w.as_mut_ptr() as *mut u8, a.cpu_bytes as usize);
            f.rows = w;
        } else if a.buffer_gpu != 0 {
            let mut w = vec![0u32; a.components as usize];
            // SAFETY: host test: the GPU address is the buffer's CPU address
            // and the GL core checked offset + one element against its size.
            core::ptr::copy_nonoverlapping(
                (a.buffer_gpu as usize + a.offset as usize) as *const u8,
                w.as_mut_ptr() as *mut u8,
                a.components as usize * 4,
            );
            f.in_place = w;
        }
        draw.push(f);
    }
    FETCHES.lock().unwrap().push(draw);
    COUNTS.lock().unwrap().push((d.index_count, d.instance_count));
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
    name: c"es3-draw-random".as_ptr() as *const c_char,
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
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glUseProgram(0);
        for i in 0..8 {
            glDisableVertexAttribArray(i);
            glVertexAttribDivisor(i, 0);
        }
        while glGetError() != GL_NO_ERROR {}
    }
    FETCHES.lock().unwrap().clear();
    COUNTS.lock().unwrap().clear();
    g
}

const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

/// A linked program whose vertex shader reads one input `a` of GLSL type `ty`.
fn program(ty: &str) -> GLuint {
    let pos = match ty {
        "vec2" => "vec4(a, 0.0, 1.0)",
        "vec3" => "vec4(a, 1.0)",
        _ => "vec4(a)",
    };
    let vs = format!("#version 300 es\nin {ty} a;\nvoid main() {{ gl_Position = {pos}; }}\n");
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
        glAttachShader(p, shader(GL_VERTEX_SHADER, &vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"a".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{ty}: the program did not link");
        p
    }
}

/// How the array is declared.
struct Array {
    size: GLint,
    ty: GLenum,
    normalized: bool,
    /// glVertexAttribIPointer
    integer: bool,
    stride: GLsizei,
    offset: usize,
}

/// The draw entry point of a case.
#[derive(Clone, Copy, Debug)]
enum Call {
    Arrays { mode: GLenum, first: GLint, count: GLsizei },
    ArraysInstanced { mode: GLenum, first: GLint, count: GLsizei, instances: GLsizei },
    Elements { mode: GLenum },
    RangeElements { mode: GLenum, start: GLuint, end: GLuint },
    ElementsInstanced { mode: GLenum, instances: GLsizei },
}

/// One draw: the array, where it lives, and the call.
struct Case<'a> {
    glsl: &'a str,
    arr: Array,
    /// The array's bytes: a buffer object, or a client array when `client`.
    data: Vec<u8>,
    client: bool,
    divisor: GLuint,
    call: Call,
    /// Indices (type, bytes) for the element calls, in a buffer or client.
    indices: Option<(GLenum, Vec<u8>)>,
    index_client: bool,
}

/// What a case did: the first error, and what the back end got.
struct Outcome {
    error: GLenum,
    /// (index_count, instance_count) per draw that reached the back end.
    draws: Vec<(u32, u32)>,
    first: Option<Fetch>,
}

fn run(c: &Case) -> Outcome {
    FETCHES.lock().unwrap().clear();
    COUNTS.lock().unwrap().clear();
    let p = program(c.glsl);
    let count = match (&c.call, &c.indices) {
        (Call::Elements { .. } | Call::RangeElements { .. } | Call::ElementsInstanced { .. }, Some((ty, b))) => {
            b.len()
                / match *ty {
                    GL_UNSIGNED_BYTE => 1,
                    GL_UNSIGNED_SHORT => 2,
                    _ => 4,
                }
        }
        _ => 0,
    } as GLsizei;
    let (mut vbo, mut ibo) = (0, 0);
    // SAFETY: plain GL calls; `c.data` and the indices outlive the draw.
    let error = unsafe {
        let off = if c.client {
            c.data.as_ptr().add(c.arr.offset) as *const c_void
        } else {
            glGenBuffers(1, &mut vbo);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glBufferData(GL_ARRAY_BUFFER, c.data.len() as GLsizeiptr, c.data.as_ptr() as *const c_void, GL_STATIC_DRAW);
            c.arr.offset as *const c_void
        };
        if c.arr.integer {
            glVertexAttribIPointer(0, c.arr.size, c.arr.ty, c.arr.stride, off);
        } else {
            glVertexAttribPointer(0, c.arr.size, c.arr.ty, c.arr.normalized as GLboolean, c.arr.stride, off);
        }
        glEnableVertexAttribArray(0);
        glVertexAttribDivisor(0, c.divisor);
        let idx: *const c_void = match &c.indices {
            Some((_, b)) if c.index_client => b.as_ptr() as *const c_void,
            Some((_, b)) => {
                glGenBuffers(1, &mut ibo);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ibo);
                glBufferData(
                    GL_ELEMENT_ARRAY_BUFFER,
                    b.len() as GLsizeiptr,
                    b.as_ptr() as *const c_void,
                    GL_STATIC_DRAW,
                );
                core::ptr::null()
            }
            None => core::ptr::null(),
        };
        let ity = c.indices.as_ref().map_or(0, |i| i.0);
        glUseProgram(p);
        match c.call {
            Call::Arrays { mode, first, count } => glDrawArrays(mode, first, count),
            Call::ArraysInstanced { mode, first, count, instances } => {
                glDrawArraysInstanced(mode, first, count, instances)
            }
            Call::Elements { mode } => glDrawElements(mode, count, ity, idx),
            Call::RangeElements { mode, start, end } => glDrawRangeElements(mode, start, end, count, ity, idx),
            Call::ElementsInstanced { mode, instances } => glDrawElementsInstanced(mode, count, ity, idx, instances),
        }
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        glFinish();
        glDisableVertexAttribArray(0);
        glVertexAttribDivisor(0, 0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &vbo);
        glDeleteBuffers(1, &ibo);
        glUseProgram(0);
        glDeleteProgram(p);
        e
    };
    let first = FETCHES.lock().unwrap().first().and_then(|d| d.first().cloned());
    Outcome { error, draws: COUNTS.lock().unwrap().clone(), first }
}

fn idx16(v: &[u16]) -> (GLenum, Vec<u8>) {
    (GL_UNSIGNED_SHORT, v.iter().flat_map(|x| x.to_ne_bytes()).collect())
}
fn idx8(v: &[u8]) -> (GLenum, Vec<u8>) {
    (GL_UNSIGNED_BYTE, v.to_vec())
}
fn idx32(v: &[u32]) -> (GLenum, Vec<u8>) {
    (GL_UNSIGNED_INT, v.iter().flat_map(|x| x.to_ne_bytes()).collect())
}
fn words(v: &[u32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_ne_bytes()).collect()
}
fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|f| f.to_bits()).collect()
}
fn arr(size: GLint, ty: GLenum, integer: bool, stride: GLsizei, offset: usize) -> Array {
    Array { size, ty, normalized: false, integer, stride, offset }
}

/// A buffer-array case with the usual defaults.
fn buf_case<'a>(glsl: &'a str, arr: Array, data: &[u8], call: Call, indices: Option<(GLenum, Vec<u8>)>) -> Case<'a> {
    Case { glsl, arr, data: data.to_vec(), client: false, divisor: 0, call, indices, index_client: false }
}

/// Case 118 (class C): the packed array is 2 elements of 4 bytes = 8 bytes,
/// indices {0, 1} are in range, so the draw is legal. At `ad2b5ed` the range
/// check sized an element as 2 components x 4 bytes and refused it.
#[test]
fn case_118_packed_array_instanced_elements_draw_raises_no_error() {
    let _g = context();
    let w0 = 0x3ff | (0x200 << 10) | (0x100 << 20) | (3 << 30);
    let w1 = 0x001 | (0x3ff << 10) | (0x080 << 20);
    let a = Array { normalized: true, ..arr(4, GL_UNSIGNED_INT_2_10_10_10_REV, false, 0, 0) };
    let o = run(&buf_case(
        "vec2",
        a,
        &words(&[w0, w1]),
        Call::ElementsInstanced { mode: GL_LINE_STRIP, instances: 2 },
        Some(idx16(&[0, 1])),
    ));
    assert_eq!(o.error, GL_NO_ERROR, "glDrawElementsInstanced must not be GL_INVALID_OPERATION");
    assert!(!o.draws.is_empty(), "the draw reached the back end");
    assert_eq!(o.draws.iter().map(|d| d.1).sum::<u32>(), 2, "two instances in all");
    let f = o.first.unwrap();
    assert!(f.in_place.is_empty(), "converted, not fetched in place");
    let n = |v: u32| v as f32 / 1023.0;
    let want = bits(&[n(0x3ff), n(0x200)]);
    assert_eq!(f.rows[..2], want[..], "float row of the decoded word");
}

/// Case 118's shape with a signed packed type and with the array at divisor 1.
#[test]
fn case_118_variants_raise_no_error() {
    let _g = context();
    for (ty, divisor) in [(GL_INT_2_10_10_10_REV, 0), (GL_UNSIGNED_INT_2_10_10_10_REV, 1)] {
        let a = Array { normalized: true, ..arr(4, ty, false, 0, 0) };
        let mut c = buf_case(
            "vec2",
            a,
            &words(&[0x1234_5678, 0x0abc_def0]),
            Call::ElementsInstanced { mode: GL_LINE_STRIP, instances: 2 },
            Some(idx16(&[0, 1])),
        );
        c.divisor = divisor;
        let o = run(&c);
        assert_eq!(o.error, GL_NO_ERROR, "0x{ty:x} divisor {divisor}");
        assert!(!o.draws.is_empty());
    }
}

/// Class A, cases 71 / 94 / 19 / 186 / 93: `glVertexAttribPointer` with
/// GL_INT or GL_UNSIGNED_INT in a buffer is converted to float, whatever the
/// draw call.
#[test]
fn class_a_int_arrays_through_the_float_entry_point_are_converted() {
    let _g = context();
    let ints: [i32; 8] = [-3, 7, 100000, -1, 2_000_000_000, -2_000_000_000, 5, 6];
    let data = words(&ints.map(|v| v as u32));
    let f32s = |v: &[i32]| bits(&v.iter().map(|&x| x as f32).collect::<Vec<_>>());

    /* 71: DrawRangeElements, lines, uint indices in a buffer, INT x1 -> float */
    let o = run(&buf_case(
        "float",
        arr(1, GL_INT, false, 0, 0),
        &data,
        Call::RangeElements { mode: GL_LINES, start: 1, end: 5 },
        Some(idx32(&[1, 5])),
    ));
    assert_eq!(o.error, GL_NO_ERROR, "71");
    let f = o.first.unwrap();
    assert!(f.in_place.is_empty(), "71: converted");
    assert_eq!(f.rows[..2], f32s(&ints[..2])[..], "71");

    /* 94: DrawRangeElements, lines, client ushort indices, INT x4 stride 16 -> vec4 */
    let mut c = buf_case(
        "vec4",
        arr(4, GL_INT, false, 16, 0),
        &data,
        Call::RangeElements { mode: GL_LINES, start: 0, end: 1 },
        Some(idx16(&[0, 1])),
    );
    c.index_client = true;
    let o = run(&c);
    assert_eq!(o.error, GL_NO_ERROR, "94");
    let f = o.first.unwrap();
    assert!(f.in_place.is_empty(), "94: converted");
    assert_eq!(f.rows, f32s(&ints), "94");

    /* 19: DrawArraysInstanced, triangle fan, first 7, INT x2 -> vec2 */
    let big: Vec<i32> = (0..20).map(|i| i * 1000 - 5000).collect();
    let o = run(&buf_case(
        "vec2",
        arr(2, GL_INT, false, 0, 0),
        &words(&big.iter().map(|&v| v as u32).collect::<Vec<_>>()),
        Call::ArraysInstanced { mode: GL_TRIANGLE_FAN, first: 7, count: 3, instances: 1 },
        None,
    ));
    assert_eq!(o.error, GL_NO_ERROR, "19");
    let f = o.first.unwrap();
    assert!(f.in_place.is_empty(), "19: converted");
    assert_eq!(f.rows, f32s(&big[14..20]), "19");

    /* 186: DrawArrays, points, INT x1 -> float, divisor 1 */
    let mut c = buf_case(
        "float",
        arr(1, GL_INT, false, 0, 0),
        &words(&(0..16).map(|v| (v * 3 - 20) as u32).collect::<Vec<_>>()),
        Call::Arrays { mode: GL_POINTS, first: 0, count: 1 },
        None,
    );
    c.divisor = 1;
    let o = run(&c);
    assert_eq!(o.error, GL_NO_ERROR, "186");
    let f = o.first.unwrap();
    assert!(f.in_place.is_empty(), "186: converted");
    assert_eq!(f.rows[0], (-20.0f32).to_bits(), "186");

    /* 93 / 202: UNSIGNED_INT x3 -> vec3, DrawElements, client ubyte indices */
    let mut c = buf_case(
        "vec3",
        arr(3, GL_UNSIGNED_INT, false, 0, 0),
        &words(&[1, 2, 4_000_000_000, 4, 5, 6]),
        Call::Elements { mode: GL_LINES },
        Some(idx8(&[0, 1])),
    );
    c.index_client = true;
    let o = run(&c);
    assert_eq!(o.error, GL_NO_ERROR, "93");
    let f = o.first.unwrap();
    assert!(f.in_place.is_empty(), "93: converted");
    assert_eq!(f.rows[2], 4_000_000_000f32.to_bits(), "93");
}

/// Class B, cases 207 / 42 / 208 / 36: an integer array narrower than the
/// `ivec4` / `uvec4` input fills w with the integer 1, in a buffer or client.
#[test]
fn class_b_narrow_integer_arrays_fill_w_with_integer_one() {
    let _g = context();

    /* 207: DrawElements, lines, uint indices in a buffer, INT x3 (IPointer) -> ivec4 */
    let ints = words(&[(-1i32) as u32, 2, 3, 4, 5, (-6i32) as u32]);
    let o = run(&buf_case(
        "ivec4",
        arr(3, GL_INT, true, 0, 0),
        &ints,
        Call::Elements { mode: GL_LINES },
        Some(idx32(&[0, 1])),
    ));
    assert_eq!(o.error, GL_NO_ERROR, "207");
    let f = o.first.unwrap();
    assert_eq!(f.rows[..8], [(-1i32) as u32, 2, 3, 1, 4, 5, (-6i32) as u32, 1], "207");

    /* 42: DrawArraysInstanced, triangles, first 7, UNSIGNED_INT x2, client, stride 16 -> uvec4 */
    let data: Vec<u32> = (0..40).collect();
    let mut c = buf_case(
        "uvec4",
        arr(2, GL_UNSIGNED_INT, true, 16, 0),
        &words(&data),
        Call::ArraysInstanced { mode: GL_TRIANGLES, first: 7, count: 3, instances: 1 },
        None,
    );
    c.client = true;
    let o = run(&c);
    assert_eq!(o.error, GL_NO_ERROR, "42");
    let f = o.first.unwrap();
    assert_eq!(f.rows[..4], [28, 29, 0, 1], "42: first = 7, stride 16, w is the integer 1");

    /* 208: DrawArrays, points, SHORT x3, client, offset 1, stride 7 -> ivec4 */
    let mut bytes = vec![0u8; 1];
    for v in 0..5i16 {
        for k in 0..3i16 {
            bytes.extend_from_slice(&(-(v * 10 + k + 1)).to_ne_bytes());
        }
        bytes.push(0);
    }
    let mut c = buf_case(
        "ivec4",
        arr(3, GL_SHORT, true, 7, 1),
        &bytes,
        Call::Arrays { mode: GL_POINTS, first: 0, count: 5 },
        None,
    );
    c.client = true;
    let o = run(&c);
    assert_eq!(o.error, GL_NO_ERROR, "208");
    let f = o.first.unwrap();
    assert_eq!(f.rows[..4], [(-1i32) as u32, (-2i32) as u32, (-3i32) as u32, 1], "208");
    assert_eq!(f.rows[4..8], [(-11i32) as u32, (-12i32) as u32, (-13i32) as u32, 1], "208 vertex 1");

    /* 36 / 151: USHORT x3 -> uvec4, DrawRangeElements, line strip, client ubyte indices */
    let us: Vec<u8> = [7u16, 8, 9, 10, 11, 12].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut c = buf_case(
        "uvec4",
        arr(3, GL_UNSIGNED_SHORT, true, 0, 0),
        &us,
        Call::RangeElements { mode: GL_LINE_STRIP, start: 0, end: 1 },
        Some(idx8(&[0, 1])),
    );
    c.index_client = true;
    let o = run(&c);
    assert_eq!(o.error, GL_NO_ERROR, "36");
    let f = o.first.unwrap();
    assert_eq!(f.rows[..8], [7, 8, 9, 1, 10, 11, 12, 1], "36");
}
