// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 vertex arrays in buffer objects, drawn through the public GL API on
//! the host (item tf-capture; docs/es3-cts-failures.md Appendix A, tasks T2
//! and T3): dEQP-GLES3 vertex_arrays.single_attribute.{first,offset,strides}
//! .int2_10_10_10* and .output_types.*.components{2,3}_{i,u}vec4.
//!
//! The stand-in back end claims no native attribute format besides float32 and
//! records, per attribute of every draw, what the GL core hands it: the
//! format, stride, and either the converted rows (`cpu`, `cpu_bytes`) or the
//! buffer address a float32 array is fetched from in place.
//!
//! * T2: a packed 2_10_10_10 or half-float array in a buffer must be converted
//!   to float32 rows, not fetched in place as float words, and its element
//!   size and stride come from the real type.
//! * T3: an integer array narrower than its ivec4/uvec4 input gets the
//!   integer 1 as w, not the float 1.0.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_vertex_arrays
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
    components: u8,
    /// `GlesAttribFormat`; 0 is GLES_ATTRIBF_FLOAT32.
    format: u8,
    stride: u32,
    /// Staged by the GL core: the converted rows (one per vertex, `stride`
    /// apart, as dwords). Empty when the array is fetched in place.
    rows: Vec<u32>,
    /// Fetched in place: the dwords at `buffer_gpu + offset` for the first
    /// vertex (the host back end's GPU address is the CPU address).
    in_place: Vec<u32>,
}

static FETCHES: Mutex<Vec<Vec<Fetch>>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let mut draw = Vec::new();
    for a in &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())] {
        let mut f = Fetch {
            components: a.components,
            format: a.format,
            stride: a.stride,
            rows: Vec::new(),
            in_place: Vec::new(),
        };
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
    name: c"es3-vertex-arrays".as_ptr() as *const c_char,
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
        glUseProgram(0);
        for i in 0..8 {
            glDisableVertexAttribArray(i);
        }
        while glGetError() != GL_NO_ERROR {}
    }
    FETCHES.lock().unwrap().clear();
    g
}

const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

/// A linked program whose vertex shader reads one input `a` of GLSL type `ty`.
fn program(ty: &str) -> GLuint {
    let pos = if ty == "vec2" { "vec4(a, 0.0, 1.0)" } else { "vec4(a)" };
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

/// Draws `count` vertices from `first` of `data` (a buffer object) with
/// input `a` of GLSL type `glsl`, and returns what the back end got for it.
fn draw(glsl: &str, arr: &Array, data: &[u8], first: GLint, count: GLsizei) -> Fetch {
    let p = program(glsl);
    // SAFETY: plain GL calls; `data` outlives glBufferData.
    unsafe {
        let mut b = 0;
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, data.len() as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        let off = arr.offset as *const c_void;
        if arr.integer {
            glVertexAttribIPointer(0, arr.size, arr.ty, arr.stride, off);
        } else {
            glVertexAttribPointer(0, arr.size, arr.ty, arr.normalized as GLboolean, arr.stride, off);
        }
        glEnableVertexAttribArray(0);
        glUseProgram(p);
        glDrawArrays(GL_TRIANGLES, first, count);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR);
        glDisableVertexAttribArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glDeleteBuffers(1, &b);
        glUseProgram(0);
        glDeleteProgram(p);
    }
    let mut all = FETCHES.lock().unwrap();
    assert_eq!(all.len(), 1, "one draw reached the back end");
    let mut draw = all.pop().unwrap();
    assert_eq!(draw.len(), 1, "one attribute");
    draw.pop().unwrap()
}

/* ---- reference decoders --------------------------------------------------- */

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|f| f.to_bits()).collect()
}

/// ES 3.0 2.1.6.1: a signed `b`-bit field, normalized or as an integer value.
fn signed_field(raw: u32, b: u32, normalized: bool) -> f32 {
    let v = ((raw << (32 - b)) as i32) >> (32 - b);
    if normalized {
        (v as f32 / ((1i32 << (b - 1)) - 1) as f32).max(-1.0)
    } else {
        v as f32
    }
}

fn unsigned_field(raw: u32, b: u32, normalized: bool) -> f32 {
    if normalized {
        raw as f32 / ((1u32 << b) - 1) as f32
    } else {
        raw as f32
    }
}

/// x:10 y:10 z:10 w:2 of a packed word, as the four floats an input reads.
fn unpack(word: u32, signed: bool, normalized: bool) -> [f32; 4] {
    let f = |shift: u32, b: u32| {
        let raw = (word >> shift) & ((1 << b) - 1);
        if signed {
            signed_field(raw, b, normalized)
        } else {
            unsigned_field(raw, b, normalized)
        }
    };
    [f(0, 10), f(10, 10), f(20, 10), f(30, 2)]
}

/// Word `i` of a test pattern that exercises every field, both signs.
fn pattern(i: u32) -> u32 {
    let x = (i * 37 + 3) & 0x3ff;
    let y = (i * 91 + 500) & 0x3ff;
    let z = (i * 211 + 511) & 0x3ff;
    let w = i & 3;
    x | (y << 10) | (z << 20) | (w << 30)
}

fn words_to_bytes(w: &[u32]) -> Vec<u8> {
    w.iter().flat_map(|v| v.to_ne_bytes()).collect()
}

/* ---- T2: packed arrays in buffers ----------------------------------------- */

const PACKED: [(GLenum, bool); 2] = [(GL_INT_2_10_10_10_REV, true), (GL_UNSIGNED_INT_2_10_10_10_REV, false)];

/// vertex_arrays.single_attribute.first.int2_10_10_10*: a tightly packed
/// array, drawn from vertex `first`.
#[test]
fn packed_array_in_a_buffer_is_converted_from_first() {
    let _g = context();
    let words: Vec<u32> = (0..16).map(pattern).collect();
    for (ty, signed) in PACKED {
        for normalized in [false, true] {
            for first in [0, 1, 5] {
                let arr = Array { size: 4, ty, normalized, integer: false, stride: 0, offset: 0 };
                let f = draw("vec4", &arr, &words_to_bytes(&words), first, 3);
                assert!(f.in_place.is_empty(), "0x{ty:x}: converted, not fetched in place");
                assert_eq!(f.format, 0, "float32 rows");
                assert_eq!((f.components, f.stride), (4, 16));
                let want: Vec<u32> = (first as usize..first as usize + 3)
                    .flat_map(|i| bits(&unpack(words[i], signed, normalized)))
                    .collect();
                assert_eq!(f.rows, want, "0x{ty:x} normalized {normalized} first {first}");
            }
        }
    }
}

/// .offset.int2_10_10_10*: the array starts `offset` bytes into the buffer.
#[test]
fn packed_array_in_a_buffer_honours_the_offset() {
    let _g = context();
    let words: Vec<u32> = (0..16).map(pattern).collect();
    for (ty, signed) in PACKED {
        for offset in [4usize, 8, 12] {
            let arr = Array { size: 4, ty, normalized: true, integer: false, stride: 0, offset };
            let f = draw("vec4", &arr, &words_to_bytes(&words), 0, 3);
            let want: Vec<u32> = (0..3).flat_map(|i| bits(&unpack(words[offset / 4 + i], signed, true))).collect();
            assert_eq!(f.rows, want, "0x{ty:x} offset {offset}");
        }
    }
}

/// .strides.int2_10_10_10*: an element is one word whatever the stride, and
/// the vertices sit `stride` bytes apart.
#[test]
fn packed_array_in_a_buffer_steps_by_the_stride() {
    let _g = context();
    let words: Vec<u32> = (0..32).map(pattern).collect();
    for (ty, signed) in PACKED {
        for stride in [4usize, 8, 12, 20] {
            let arr = Array { size: 4, ty, normalized: false, integer: false, stride: stride as GLsizei, offset: 4 };
            let f = draw("vec4", &arr, &words_to_bytes(&words), 1, 3);
            let want: Vec<u32> =
                (1..4).flat_map(|v| bits(&unpack(words[(4 + v * stride) / 4], signed, false))).collect();
            assert_eq!(f.rows, want, "0x{ty:x} stride {stride}");
        }
    }
}

/// A tightly packed array that ends exactly at the buffer's end is legal: the
/// element is one word, not four floats.
#[test]
fn packed_array_filling_the_buffer_is_not_refused() {
    let _g = context();
    let words: Vec<u32> = (0..3).map(pattern).collect();
    let arr = Array { size: 4, ty: GL_INT_2_10_10_10_REV, normalized: false, integer: false, stride: 0, offset: 0 };
    let f = draw("vec4", &arr, &words_to_bytes(&words), 0, 3);
    assert_eq!(f.rows.len(), 12);
}

/// GL_HALF_FLOAT in a buffer: converted to float32 rows from the 2-byte
/// elements (component size 2, not 4).
#[test]
fn half_float_array_in_a_buffer_is_converted() {
    let _g = context();
    /* 1.0, -2.0, 0.5, 4.0, 0.0, 8.0, -0.25, 2.0 as binary16 */
    let h: [u16; 8] = [0x3c00, 0xc000, 0x3800, 0x4400, 0x0000, 0x4800, 0xb400, 0x4000];
    let data: Vec<u8> = h.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let arr = Array { size: 2, ty: GL_HALF_FLOAT, normalized: false, integer: false, stride: 0, offset: 0 };
    let f = draw("vec2", &arr, &data, 1, 3);
    assert!(f.in_place.is_empty(), "converted, not fetched in place");
    assert_eq!(f.rows, bits(&[0.5, 4.0, 0.0, 8.0, -0.25, 2.0]));
    /* strided, with an offset */
    let arr = Array { size: 2, ty: GL_HALF_FLOAT, normalized: false, integer: false, stride: 6, offset: 2 };
    let f = draw("vec2", &arr, &data[..], 0, 2);
    assert_eq!(f.rows, bits(&[-2.0, 0.5, 0.0, 8.0]));
}

/// glVertexAttribPointer with a 32-bit integer type converts to float
/// (ES 3.0 2.9); in a buffer it used to be fetched as raw integer words.
#[test]
fn int_array_through_the_float_entry_point_is_converted() {
    let _g = context();
    let ints: [i32; 4] = [-3, 7, 100000, -1];
    let data: Vec<u8> = ints.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let arr = Array { size: 2, ty: GL_INT, normalized: false, integer: false, stride: 0, offset: 0 };
    let f = draw("vec2", &arr, &data, 0, 2);
    assert_eq!(f.rows, bits(&[-3.0, 7.0, 100000.0, -1.0]));
}

/// A float array and an integer array of a plain 32-bit type stay fetched in
/// place (nothing to convert).
#[test]
fn float_and_integer_arrays_stay_in_place() {
    let _g = context();
    let fl: Vec<u8> = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0]
        .iter()
        .flat_map(|v| v.to_ne_bytes())
        .collect();
    let arr = Array { size: 4, ty: GL_FLOAT, normalized: false, integer: false, stride: 0, offset: 0 };
    let f = draw("vec4", &arr, &fl, 0, 3);
    assert!(f.rows.is_empty());
    assert_eq!(f.in_place, bits(&[1.0, 2.0, 3.0, 4.0]));
    let arr = Array { size: 4, ty: GL_INT, normalized: false, integer: true, stride: 0, offset: 0 };
    let f = draw("ivec4", &arr, &fl, 0, 3);
    assert!(f.rows.is_empty(), "a full-width integer array is fetched as it lies");
}

/* ---- T3: the w of a narrow integer array ---------------------------------- */

/// output_types.*.components{2,3}_{i,u}vec4: the missing components read
/// (0, 0, 0, 1) in the input's type.
#[test]
fn narrow_integer_arrays_fill_w_with_integer_one() {
    let _g = context();
    let cases: [(GLenum, &str, usize); 6] = [
        (GL_BYTE, "ivec4", 1),
        (GL_SHORT, "ivec4", 2),
        (GL_INT, "ivec4", 4),
        (GL_UNSIGNED_BYTE, "uvec4", 1),
        (GL_UNSIGNED_SHORT, "uvec4", 2),
        (GL_UNSIGNED_INT, "uvec4", 4),
    ];
    for (ty, glsl, tsz) in cases {
        for size in [2usize, 3] {
            /* distinct small values per component, three vertices */
            let mut data = Vec::new();
            for v in 0..3 {
                for c in 0..size {
                    let val = (v * 10 + c + 1) as u32;
                    data.extend_from_slice(&val.to_ne_bytes()[..tsz]);
                }
            }
            let arr = Array { size: size as GLint, ty, normalized: false, integer: true, stride: 0, offset: 0 };
            let f = draw(glsl, &arr, &data, 0, 3);
            assert_eq!(f.components, 4);
            let want: Vec<u32> = (0..3)
                .flat_map(|v| {
                    (0..4).map(move |c| match c {
                        c if c < size => (v * 10 + c + 1) as u32,
                        3 => 1,
                        _ => 0,
                    })
                })
                .collect();
            assert_eq!(f.rows, want, "0x{ty:x} {glsl} components{size}");
        }
    }
}

/// A negative signed value is sign-extended (the dEQP data is signed).
#[test]
fn narrow_signed_integer_array_sign_extends() {
    let _g = context();
    let data: Vec<u8> = [-5i16, 6, -7, 8].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let arr = Array { size: 2, ty: GL_SHORT, normalized: false, integer: true, stride: 0, offset: 0 };
    let f = draw("ivec4", &arr, &data, 0, 2);
    assert_eq!(f.rows, vec![(-5i32) as u32, 6, 0, 1, (-7i32) as u32, 8, 0, 1]);
}

/// A narrow float array still fills the float 1.0 (ES 2.0 behaviour).
#[test]
fn narrow_float_array_still_fills_float_one() {
    let _g = context();
    let data: Vec<u8> = [1.5f32, 2.5, 3.5, 4.5].iter().flat_map(|v| v.to_ne_bytes()).collect();
    let arr = Array { size: 2, ty: GL_FLOAT, normalized: false, integer: false, stride: 0, offset: 0 };
    let f = draw("vec4", &arr, &data, 0, 2);
    assert_eq!(f.rows, bits(&[1.5, 2.5, 0.0, 1.0, 3.5, 4.5, 0.0, 1.0]));
}
