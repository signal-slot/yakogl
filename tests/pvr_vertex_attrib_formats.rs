// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A sweep of every ES 3.0 vertex attribute type, through the public GL API on
//! the host, with the table of a PowerVR-shaped back end: what the GL core
//! hands the back end for BYTE, UNSIGNED_BYTE, SHORT, UNSIGNED_SHORT, INT,
//! UNSIGNED_INT, FIXED, HALF_FLOAT, FLOAT, INT_2_10_10_10_REV and
//! UNSIGNED_INT_2_10_10_10_REV, normalized and not, sizes 1 to 4 where legal,
//! and glVertexAttribIPointer for the integer types (dEQP-GLES3
//! vertex_arrays.single_attribute on the AM62P; tests/es3_vertex_arrays.rs pins
//! the formats, strides and offsets of the failing cases).
//!
//! The PowerVR vertex fetch reads float32 only (`native_attrib_formats` is 0
//! in `yakogl_backend_pvr`): the GL core converts every other type to float32
//! (or 32-bit integer) rows, which is what the vertex shader sees. The stand-in
//! back end here has the same 0, and the first test asserts the PowerVR
//! table's value, so a change to it (a native attribute path) fails on the
//! host and this sweep must then be re-read.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_vertex_attrib_formats
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

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
    name: c"pvr-vertex-attrib-formats".as_ptr() as *const c_char,
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

/* ---- the PowerVR back end is linked only to read its table ---------------- */

/* Naming yakogl_backend_pvr links src/pvr/backend.rs, whose C command
 * generators (driver/src/pvr) are not part of a host test build. No draw goes
 * through that back end here, so these are never called. */
#[no_mangle]
extern "C" fn pvr_isp_build_scissor_word() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_pds_generate_pixel_event_tq() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_pds_render_coeff_program() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_pds_render_color_upload_program() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_pds_render_dma_program() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_pds_render_kick_usc_program() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_pds_render_vertex_program_ids_base() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_ppp_build_block() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_ppp_build_terminate_block() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_vdm_emit_index_list() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_vdm_emit_index_list_instanced() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_vdm_emit_pds_state() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_vdm_emit_ppp_state() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_vdm_emit_terminate() {
    unreachable!("the PowerVR back end is not driven by this test");
}
#[no_mangle]
extern "C" fn pvr_vdm_emit_vdm_state() {
    unreachable!("the PowerVR back end is not driven by this test");
}

/* ---- the sweep ------------------------------------------------------------ */

/// The PowerVR vertex fetch reads float32 only; every other type is converted
/// by the GL core (src/gl/draw.rs, `native_attrib_formats`). If this fails the
/// back end started fetching a type natively: the sweep below no longer
/// describes the path and the board result must be rechecked.
#[test]
fn powervr_table_has_no_native_attrib_formats() {
    let pvr = yakogl::pvr::backend::yakogl_backend_pvr.native_attrib_formats;
    assert_eq!(pvr, 0, "the PowerVR converts attributes in the GL core");
    assert_eq!(BACKEND.native_attrib_formats, pvr, "the stand-in is PowerVR-shaped");
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|f| f.to_bits()).collect()
}

fn check_floats(got: &[u32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}: row length");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        let g = f32::from_bits(*g);
        assert!((g - w).abs() <= 1e-6 * w.abs().max(1.0), "{what}: element {i}: got {g}, want {w}");
    }
}

const VERTS: usize = 3;

/// Raw component values of a type, cycling; the signed ones include
/// negatives and the extremes.
fn raws(ty: GLenum) -> &'static [i64] {
    match ty {
        GL_BYTE => &[-128, 127, -1, 5, 64, -64, 0, 100],
        GL_UNSIGNED_BYTE => &[0, 255, 1, 128, 77, 200, 3, 64],
        GL_SHORT => &[-32768, 32767, -1, 5, 1000, -1000, 0, 12345],
        GL_UNSIGNED_SHORT => &[0, 65535, 1, 32768, 777, 40000, 3, 9000],
        GL_INT => &[-2147483648, 2147483647, -1, 5, 100000, -100000, 0, 123456789],
        GL_UNSIGNED_INT => &[0, 4294967295, 1, 2147483648, 77777, 3000000000, 3, 65536],
        GL_FIXED => &[-65536, 65536, 32768, -98304, 0, 163840, 1, -1],
        _ => unreachable!(),
    }
}

fn tsize(ty: GLenum) -> usize {
    match ty {
        GL_BYTE | GL_UNSIGNED_BYTE => 1,
        GL_SHORT | GL_UNSIGNED_SHORT => 2,
        _ => 4,
    }
}

fn is_signed(ty: GLenum) -> bool {
    matches!(ty, GL_BYTE | GL_SHORT | GL_INT | GL_FIXED)
}

fn raw(ty: GLenum, i: usize) -> i64 {
    let r = raws(ty);
    r[i % r.len()]
}

fn encode(ty: GLenum, v: i64) -> Vec<u8> {
    (v as u32).to_ne_bytes()[..tsize(ty)].to_vec()
}

/// ES 3.0 2.1.6.1 float conversion of a raw component.
fn to_float(ty: GLenum, v: i64, normalized: bool) -> f32 {
    if ty == GL_FIXED {
        return (v as f64 / 65536.0) as f32;
    }
    if !normalized {
        return v as f32;
    }
    let bits = tsize(ty) as u32 * 8;
    if is_signed(ty) {
        ((v as f64 / ((1i64 << (bits - 1)) - 1) as f64).max(-1.0)) as f32
    } else {
        (v as f64 / ((1u64 << bits) - 1) as f64) as f32
    }
}

/// `size` components of each of VERTS vertices, laid out at `offset` with
/// `stride` (0: tight); returns the buffer and the four values per vertex the
/// shader reads: `conv` of a raw value, 0 for the components the array lacks
/// and `one` for a missing w.
fn build(
    ty: GLenum,
    size: usize,
    stride: usize,
    offset: usize,
    conv: &dyn Fn(i64) -> f64,
    one: f64,
) -> (Vec<u8>, Vec<f64>) {
    let step = if stride == 0 { tsize(ty) * size } else { stride };
    let mut data = vec![0u8; offset + step * VERTS];
    let mut want = Vec::new();
    for v in 0..VERTS {
        for c in 0..4 {
            if c < size {
                let val = raw(ty, v * 4 + c);
                let at = offset + v * step + c * tsize(ty);
                data[at..at + tsize(ty)].copy_from_slice(&encode(ty, val));
                want.push(conv(val));
            } else {
                want.push(if c == 3 { one } else { 0.0 });
            }
        }
    }
    (data, want)
}

/// The layouts swept: tight, and strided with an offset.
fn layouts(ty: GLenum, size: usize) -> [(usize, usize); 2] {
    [(0, 0), (tsize(ty) * size + 4, 2 * tsize(ty))]
}

/// Plain-type arrays: every size 1..4, tight and strided, through
/// glVertexAttribPointer, normalized and not (FIXED has no normalized form).
#[test]
fn float_entry_point_converts_every_plain_type() {
    let _g = context();
    for ty in [GL_BYTE, GL_UNSIGNED_BYTE, GL_SHORT, GL_UNSIGNED_SHORT, GL_INT, GL_UNSIGNED_INT, GL_FIXED] {
        for normalized in [false, true] {
            if ty == GL_FIXED && normalized {
                continue;
            }
            for size in 1..=4usize {
                for (stride, offset) in layouts(ty, size) {
                    let conv = |v: i64| to_float(ty, v, normalized) as f64;
                    let (data, want) = build(ty, size, stride, offset, &conv, 1.0);
                    let want: Vec<f32> = want.iter().map(|v| *v as f32).collect();
                    let arr = Array {
                        size: size as GLint,
                        ty,
                        normalized,
                        integer: false,
                        stride: stride as GLsizei,
                        offset,
                    };
                    let f = draw("vec4", &arr, &data, 0, VERTS as GLsizei);
                    let what = format!("0x{ty:x} normalized {normalized} size {size} stride {stride}");
                    assert!(f.in_place.is_empty(), "{what}: converted, not fetched in place");
                    assert_eq!((f.format, f.components, f.stride), (0, 4, 16), "{what}: float32 rows");
                    check_floats(&f.rows, &want, &what);
                }
            }
        }
    }
}

/// glVertexAttribIPointer: the integer types reach an ivec4/uvec4 input as
/// integers (sign-extended for the signed ones), missing components 0, 0, 0, 1.
#[test]
fn integer_entry_point_keeps_integers() {
    let _g = context();
    for ty in [GL_BYTE, GL_UNSIGNED_BYTE, GL_SHORT, GL_UNSIGNED_SHORT, GL_INT, GL_UNSIGNED_INT] {
        let glsl = if is_signed(ty) { "ivec4" } else { "uvec4" };
        for size in 1..=4usize {
            for (stride, offset) in layouts(ty, size) {
                let (data, want) = build(ty, size, stride, offset, &|v| v as f64, 1.0);
                let want: Vec<u32> = want.iter().map(|v| (*v as i64) as u32).collect();
                let arr = Array {
                    size: size as GLint,
                    ty,
                    normalized: false,
                    integer: true,
                    stride: stride as GLsizei,
                    offset,
                };
                let f = draw(glsl, &arr, &data, 0, VERTS as GLsizei);
                let what = format!("0x{ty:x} size {size} stride {stride}");
                if f.rows.is_empty() {
                    /* a full-width 4-component integer array is fetched as it lies */
                    assert_eq!((tsize(ty), size), (4, 4), "{what}: only a full-width ivec4 is in place");
                    assert_eq!(f.in_place, want[..4], "{what}: in place");
                } else {
                    assert_eq!((f.components, f.stride), (4, 16), "{what}");
                    assert_eq!(f.rows, want, "{what}");
                }
            }
        }
    }
}

/// GL_FLOAT: a size-4 array is fetched in place; the others are staged
/// with (0, 0, 0, 1) filled in.
#[test]
fn float_arrays_pass_through() {
    let _g = context();
    let vals: Vec<f32> = (0..16).map(|i| i as f32 * 1.5 - 7.0).collect();
    for size in 1..=4usize {
        for (stride, offset) in [(0usize, 0usize), (size * 4 + 8, 4)] {
            let step = if stride == 0 { size * 4 } else { stride };
            let mut data = vec![0u8; offset + step * VERTS];
            let mut want = Vec::new();
            for v in 0..VERTS {
                for c in 0..4 {
                    let x = if c < size {
                        vals[v * 4 + c]
                    } else if c == 3 {
                        1.0
                    } else {
                        0.0
                    };
                    if c < size {
                        let at = offset + v * step + c * 4;
                        data[at..at + 4].copy_from_slice(&x.to_ne_bytes());
                    }
                    want.push(x);
                }
            }
            let arr = Array {
                size: size as GLint,
                ty: GL_FLOAT,
                normalized: false,
                integer: false,
                stride: stride as GLsizei,
                offset,
            };
            let f = draw("vec4", &arr, &data, 0, VERTS as GLsizei);
            let what = format!("float size {size} stride {stride}");
            if f.rows.is_empty() {
                assert_eq!(size, 4, "{what}: only a vec4 is in place");
                assert_eq!(f.in_place, bits(&want[..4]), "{what}: in place");
            } else {
                assert_eq!(f.rows, bits(&want), "{what}");
            }
        }
    }
}

/// GL_HALF_FLOAT, sizes 1..4: converted to float32 (2-byte components).
#[test]
fn half_float_converts_every_size() {
    let _g = context();
    let h: [u16; 12] = [0x3c00, 0xc000, 0x3800, 0x4400, 0x0000, 0x4800, 0xb400, 0x4000, 0x4c00, 0xbe00, 0x3000, 0x4200];
    let f32s: [f32; 12] = [1.0, -2.0, 0.5, 4.0, 0.0, 8.0, -0.25, 2.0, 16.0, -1.5, 0.125, 3.0];
    for size in 1..=4usize {
        let step = size * 2;
        let mut data = vec![0u8; step * VERTS];
        let mut want = Vec::new();
        for v in 0..VERTS {
            for c in 0..4 {
                if c < size {
                    let i = v * 4 + c;
                    data[v * step + c * 2..v * step + c * 2 + 2].copy_from_slice(&h[i].to_ne_bytes());
                    want.push(f32s[i]);
                } else {
                    want.push(if c == 3 { 1.0 } else { 0.0 });
                }
            }
        }
        let arr =
            Array { size: size as GLint, ty: GL_HALF_FLOAT, normalized: false, integer: false, stride: 0, offset: 0 };
        let f = draw("vec4", &arr, &data, 0, VERTS as GLsizei);
        assert!(f.in_place.is_empty(), "half {size}: converted");
        assert_eq!(f.rows, bits(&want), "half size {size}");
    }
}

/// The packed types (size 4 only), normalized and not, tight and strided.
#[test]
fn packed_types_convert_to_float() {
    let _g = context();
    let words: Vec<u32> = (0..12u32)
        .map(|i| {
            let x = (i * 37 + 3) & 0x3ff;
            let y = (i * 91 + 500) & 0x3ff;
            let z = (i * 211 + 511) & 0x3ff;
            x | (y << 10) | (z << 20) | ((i & 3) << 30)
        })
        .collect();
    let field = |w: u32, shift: u32, b: u32, signed: bool, norm: bool| -> f32 {
        let r = (w >> shift) & ((1 << b) - 1);
        if signed {
            let v = ((r << (32 - b)) as i32) >> (32 - b);
            if norm {
                (v as f32 / ((1i32 << (b - 1)) - 1) as f32).max(-1.0)
            } else {
                v as f32
            }
        } else if norm {
            r as f32 / ((1u32 << b) - 1) as f32
        } else {
            r as f32
        }
    };
    let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_ne_bytes()).collect();
    for (ty, signed) in [(GL_INT_2_10_10_10_REV, true), (GL_UNSIGNED_INT_2_10_10_10_REV, false)] {
        for normalized in [false, true] {
            for (stride, offset) in [(0usize, 0usize), (12, 4)] {
                let step = if stride == 0 { 4 } else { stride };
                let want: Vec<f32> = (0..VERTS)
                    .flat_map(|v| {
                        let w = words[(offset + v * step) / 4];
                        [
                            field(w, 0, 10, signed, normalized),
                            field(w, 10, 10, signed, normalized),
                            field(w, 20, 10, signed, normalized),
                            field(w, 30, 2, signed, normalized),
                        ]
                    })
                    .collect();
                let arr = Array { size: 4, ty, normalized, integer: false, stride: stride as GLsizei, offset };
                let f = draw("vec4", &arr, &bytes, 0, VERTS as GLsizei);
                let what = format!("0x{ty:x} normalized {normalized} stride {stride}");
                assert!(f.in_place.is_empty(), "{what}: converted");
                assert_eq!((f.format, f.components, f.stride), (0, 4, 16), "{what}");
                check_floats(&f.rows, &want, &what);
            }
        }
    }
}
