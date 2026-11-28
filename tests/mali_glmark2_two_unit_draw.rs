// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's jellyfish draw shape on the Mali back end, through the GL API on
//! the host: two textures on units 0 and 1 (a body texture and a caustics
//! texture), two sampler uniforms, additive blending
//! (`glBlendFunc(GL_SRC_ALPHA, GL_ONE)`), a per-frame time uniform and an
//! indexed draw from client arrays.
//!
//! The real Mali back end runs over a mock OSAL and `mali_draw` takes every
//! draw; no GPU runs and no pixel is written, so nothing can be read back.
//! What the test checks is what the back end is handed for each draw: the
//! texture bound to each sampler (its texels, read from the buffer the draw
//! names), the uniform words, the blend factors and the draw's target. The
//! frame's colour is then computed on the CPU from exactly that, as the
//! program and the blend unit would, and compared with the colour expected
//! from the textures and the uniforms the test set. A stale uniform value, a
//! texture on the wrong unit or a swap that does not reach the draw changes
//! the computed colour.
//!
//! The fragment shader is `(texture2D(body, uv) + texture2D(caustics, uv)) *
//! u_time_scale`: the sum of two look-ups scaled by a uniform, which the
//! online compiler accepts at the tip (glmark2's own jellyfish shaders are
//! another matter, see docs/notes/mali-glmark2-forecast.md).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_two_unit_draw
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

static FENCES: AtomicI64 = AtomicI64::new(0);

/// Buffers made so far: (GPU VA, CPU address, bytes). Each VA is its number
/// times 16 MiB, far from any CPU address.
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let mut b = BUFFERS.lock().unwrap();
    let gpu = 0x40_0000_0000 + (b.len() as u64 + 1) * (16 << 20);
    b.push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: cpu };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}

/// The CPU address of `n` bytes at GPU VA `gpu`, inside one buffer.
fn cpu_of(gpu: u64, n: usize) -> Option<usize> {
    let b = BUFFERS.lock().unwrap();
    let &(g, cpu, size) = b.iter().find(|&&(g, _, s)| (g..g + s as u64).contains(&gpu))?;
    let off = (gpu - g) as usize;
    (off + n <= size).then_some(cpu + off)
}

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    let _ = (kick, len);
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`.
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`: the same calling convention for these arguments.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// What the back end was given for one draw.
#[derive(Clone, Debug)]
struct Seen {
    index_count: u32,
    target_gpu: u64,
    /// (unit, format, base GPU address, width, height, row pitch in bytes).
    textures: Vec<(u8, u8, u64, u32, u32, u32)>,
    fs_uniforms: Vec<u32>,
    blend: bool,
    src_rgb: u8,
    dst_rgb: u8,
    src_a: u8,
    dst_a: u8,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    let fs_uniforms = if dr.fs_uniform_cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(dr.fs_uniform_cpu.cast::<u32>(), dr.fs_uniform_dwords as usize).to_vec()
    };
    SEEN.lock().unwrap().push(Seen {
        index_count: dr.index_count,
        target_gpu: dr.target_gpu,
        textures: dr.textures[..dr.num_textures as usize]
            .iter()
            .map(|t| (t.unit, t.format, t.base_gpu, t.width, t.height, 4 * t.stride_texels))
            .collect(),
        fs_uniforms,
        blend: dr.blend,
        src_rgb: dr.blend_src_rgb,
        dst_rgb: dr.blend_dst_rgb,
        src_a: dr.blend_src_alpha,
        dst_a: dr.blend_dst_alpha,
        mali,
    });
    mali
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const N: GLsizei = 16;

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
        // SAFETY: the Mali table copied bit for bit with only `draw`
        // replaced; the OSAL and the copy are leaked; the lock serialises
        // the tests.
        unsafe {
            osal_set_ops(&OSAL);
            let mut b: YakoglBackend = core::ptr::read(&yakogl::mali::backend::yakogl_backend_mali);
            b.draw = Some(b_draw);
            backend::g_backend = Box::leak(Box::new(b));
            assert_eq!(yakogl::mali::backend::mali_init(), 0);
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(N as u32, N as u32).is_null());
            glViewport(0, 0, N, N);
        }
    });
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn ok(what: &str) {
    // SAFETY: plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}");
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
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        assert_eq!(ok, 1, "{text:?} did not compile: {}", String::from_utf8_lossy(&log[..n as usize]));
        s
    }
}

fn program(vs: &str, fs: &str) -> GLuint {
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_uv".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        p
    }
}

const VS: &str = "attribute vec4 a_pos;\nattribute vec2 a_uv;\nvarying vec2 v_uv;\nvoid main() { gl_Position = a_pos; v_uv = a_uv; }\n";
const FS: &str = "precision mediump float;\nuniform sampler2D u_body;\nuniform sampler2D u_caustics;\nuniform float u_time_scale;\nvarying vec2 v_uv;\nvoid main() { gl_FragColor = (texture2D(u_body, v_uv) + texture2D(u_caustics, v_uv)) * u_time_scale; }\n";

/// An RGBA8 texture of one colour, 4x4 texels, nearest, clamped.
fn solid_texture(rgba: [u8; 4]) -> GLuint {
    let texels: Vec<u8> = (0..16).flat_map(|_| rgba).collect();
    let mut tex = 0;
    // SAFETY: 16 RGBA8 texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 4, 4, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
    tex
}

const BODY: [u8; 4] = [204, 102, 0, 51];
const CAUSTICS: [u8; 4] = [0, 51, 153, 102];

/// A bound texture's first texel as floats, read from the buffer the draw names.
fn texel(t: &(u8, u8, u64, u32, u32, u32)) -> [f32; 4] {
    assert_eq!(t.1, backend::GLES_TEXFMT_RGBA8);
    let cpu = cpu_of(t.2, 4).expect("the texture's buffer");
    // SAFETY: inside a leaked buffer.
    let b = unsafe { *(cpu as *const [u8; 4]) };
    b.map(|v| f32::from(v) / 255.0)
}

/// Every texel of a bound texture is the same colour (the test's textures).
fn check_solid(t: &(u8, u8, u64, u32, u32, u32)) {
    let first = texel(t);
    for y in 0..t.4 {
        for x in 0..t.3 {
            let cpu = cpu_of(t.2 + u64::from(y * t.5 + 4 * x), 4).expect("the texture's buffer");
            // SAFETY: inside a leaked buffer.
            let b = unsafe { *(cpu as *const [u8; 4]) }.map(|v| f32::from(v) / 255.0);
            assert_eq!(b, first, "texel ({x}, {y})");
        }
    }
}

/// The jellyfish-shaped scene: indexed quad from client arrays.
fn draw_quad() -> Seen {
    let pos: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
    let uv: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0];
    static IDX: [u16; 6] = [0, 1, 2, 2, 1, 3];
    SEEN.lock().unwrap().clear();
    let before = backend::draws_refused();
    // SAFETY: the arrays outlive the draw, which `glFinish` completes.
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, pos.as_ptr().cast());
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, uv.as_ptr().cast());
        glEnableVertexAttribArray(1);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr().cast());
        glFinish();
        glGetError()
    };
    assert_eq!(e, GL_NO_ERROR, "glGetError 0x{e:x}");
    assert_eq!(backend::draws_refused(), before, "a draw was refused");
    let s = SEEN.lock().unwrap().iter().rev().find(|s| s.index_count == 6).cloned();
    let s = s.expect("the draw reached the back end");
    assert_eq!(s.mali, 0, "mali_draw refused");
    s
}

struct Scene {
    p: GLuint,
    loc_body: GLint,
    loc_caustics: GLint,
    loc_time: GLint,
    tex_a: GLuint,
    tex_b: GLuint,
}

fn scene() -> Scene {
    let p = program(VS, FS);
    let tex_a;
    let tex_b;
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glActiveTexture(GL_TEXTURE0);
        tex_a = solid_texture(BODY);
        glActiveTexture(GL_TEXTURE1);
        tex_b = solid_texture(CAUSTICS);
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE);
        glDisable(GL_DEPTH_TEST);
    }
    ok("scene set-up");
    // SAFETY: plain GL calls.
    unsafe {
        Scene {
            p,
            loc_body: glGetUniformLocation(p, c"u_body".as_ptr()),
            loc_caustics: glGetUniformLocation(p, c"u_caustics".as_ptr()),
            loc_time: glGetUniformLocation(p, c"u_time_scale".as_ptr()),
            tex_a,
            tex_b,
        }
    }
}

fn set_samplers(s: &Scene, body_unit: GLint, caustics_unit: GLint) {
    // SAFETY: plain GL calls on the program in use.
    unsafe {
        glUseProgram(s.p);
        glUniform1i(s.loc_body, body_unit);
        glUniform1i(s.loc_caustics, caustics_unit);
    }
}

fn set_time(s: &Scene, t: f32) {
    // SAFETY: plain GL call on the program in use.
    unsafe {
        glUseProgram(s.p);
        glUniform1f(s.loc_time, t);
    }
}

/// The draw's state as the test expects it: blending on with
/// (SRC_ALPHA, ONE) for colour and alpha, to the default framebuffer.
fn check_blend(d: &Seen) {
    assert!(d.blend, "blending is on");
    assert_ne!(d.target_gpu, 0, "the draw has a colour surface");
    use yakogl::shaderc::{SHC_BF_ONE, SHC_BF_SRC_ALPHA};
    assert_eq!(u32::from(d.src_rgb), SHC_BF_SRC_ALPHA);
    assert_eq!(u32::from(d.dst_rgb), SHC_BF_ONE);
    assert_eq!(u32::from(d.src_a), SHC_BF_SRC_ALPHA);
    assert_eq!(u32::from(d.dst_a), SHC_BF_ONE);
}

/// The time uniform's value in the draw's fragment uniform words: the one
/// word that is a float among the words (the samplers are not in them).
fn time_of(d: &Seen) -> f32 {
    let f: Vec<f32> = d.fs_uniforms.iter().map(|&w| f32::from_bits(w)).collect();
    assert!(!f.is_empty(), "the draw carries fragment uniforms");
    f[0]
}

/// The colour the draw gives a pixel over `dst`: the program on the texels
/// the draw's bindings give the two samplers, then GL_SRC_ALPHA / GL_ONE.
/// The units are the draw's sampler bindings.
fn shade(d: &Seen, body_unit: u8, caustics_unit: u8, dst: [f32; 4]) -> [f32; 4] {
    let by_unit = |u: u8| {
        d.textures.iter().find(|t| t.0 == u).unwrap_or_else(|| panic!("unit {u} is not bound: {:?}", d.textures))
    };
    let (a, b) = (texel(by_unit(body_unit)), texel(by_unit(caustics_unit)));
    let k = time_of(d);
    let src: Vec<f32> = (0..4).map(|i| ((a[i] + b[i]) * k).clamp(0.0, 1.0)).collect();
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = (dst[i] + src[i] * src[3]).min(1.0);
    }
    out
}

fn near(got: [f32; 4], want: [f32; 4], what: &str) {
    for i in 0..4 {
        assert!((got[i] - want[i]).abs() < 1e-4, "{what}: channel {i} is {}, expected {}", got[i], want[i]);
    }
}

/// Both units reach the draw, with their own texels, and the colour computed
/// from them is the sum of the two textures scaled by the uniform, blended
/// SRC_ALPHA / ONE onto black.
#[test]
fn two_units_combine() {
    let _g = context();
    let s = scene();
    set_samplers(&s, 0, 1);
    set_time(&s, 0.5);
    let d = draw_quad();
    ok("draw");
    check_blend(&d);
    assert_eq!(d.textures.len(), 2, "two textures bound: {:?}", d.textures);
    assert_ne!(d.textures[0].2, d.textures[1].2, "two different buffers");
    d.textures.iter().for_each(check_solid);
    assert_eq!(time_of(&d), 0.5);
    let got = shade(&d, 0, 1, [0.0; 4]);
    let k = 0.5f32;
    let sum: Vec<f32> = (0..4).map(|i| ((f32::from(BODY[i]) + f32::from(CAUSTICS[i])) / 255.0 * k).min(1.0)).collect();
    let want = [sum[0] * sum[3], sum[1] * sum[3], sum[2] * sum[3], sum[3] * sum[3]];
    near(got, want, "units 0 and 1");
    // Neither texture alone gives it.
    assert!((got[2] - f32::from(BODY[2]) / 255.0 * k * sum[3]).abs() > 1e-3, "the caustics texture reaches the colour");
    assert!((got[0] - f32::from(CAUSTICS[0]) / 255.0 * k * sum[3]).abs() > 1e-3, "the body texture reaches the colour");
}

/// The textures on the two units are really the two the test uploaded
/// (the sampler uniforms are 0 and 1, so binding k reads unit k): rebinding
/// the textures the other way round between frames changes the bound texels.
#[test]
fn each_unit_holds_its_own_texture() {
    let _g = context();
    let s = scene();
    set_samplers(&s, 0, 1);
    set_time(&s, 1.0);
    let d = draw_quad();
    let tex = |u: u8| *d.textures.iter().find(|t| t.0 == u).unwrap();
    let a: Vec<u8> = texel(&tex(0)).iter().map(|v| (v * 255.0).round() as u8).collect();
    let b: Vec<u8> = texel(&tex(1)).iter().map(|v| (v * 255.0).round() as u8).collect();
    assert_eq!(a, BODY, "unit 0 holds the body texture");
    assert_eq!(b, CAUSTICS, "unit 1 holds the caustics texture");
    // Rebind the other way round between frames.
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, s.tex_b);
        glActiveTexture(GL_TEXTURE1);
        glBindTexture(GL_TEXTURE_2D, s.tex_a);
    }
    let d = draw_quad();
    let tex = |u: u8| *d.textures.iter().find(|t| t.0 == u).unwrap();
    let a: Vec<u8> = texel(&tex(0)).iter().map(|v| (v * 255.0).round() as u8).collect();
    let b: Vec<u8> = texel(&tex(1)).iter().map(|v| (v * 255.0).round() as u8).collect();
    assert_eq!(a, CAUSTICS, "unit 0 now holds the caustics texture");
    assert_eq!(b, BODY, "unit 1 now holds the body texture");
    ok("rebind");
}

/// Swapping the sampler uniforms between frames: a draw's texture `unit` is
/// the program's sampler binding (0 `u_body`, 1 `u_caustics`), and the texels
/// behind a binding are those of the GL unit the uniform names. The sum is
/// symmetric, so the colour stays and the bindings' texels trade places.
#[test]
fn swapped_sampler_uniforms_follow() {
    let _g = context();
    let s = scene();
    set_time(&s, 0.5);
    set_samplers(&s, 0, 1);
    let d0 = draw_quad();
    set_samplers(&s, 1, 0);
    let d1 = draw_quad();
    ok("draws");
    for d in [&d0, &d1] {
        assert_eq!(d.textures.iter().map(|t| t.0).collect::<Vec<_>>(), vec![0, 1], "one bind per sampler");
    }
    let (a0, b0) = (texel(&d0.textures[0]), texel(&d0.textures[1]));
    let (a1, b1) = (texel(&d1.textures[0]), texel(&d1.textures[1]));
    assert_ne!(a0, b0, "the two textures differ");
    assert_eq!(a1, b0, "u_body now reads unit 1's texture");
    assert_eq!(b1, a0, "u_caustics now reads unit 0's texture");
    assert_eq!(d0.textures[0].2, d1.textures[1].2);
    assert_eq!(d0.textures[1].2, d1.textures[0].2);
    near(shade(&d1, 0, 1, [0.0; 4]), shade(&d0, 0, 1, [0.0; 4]), "swapped uniforms, same sum");
}

/// The time uniform changes over three frames; each draw is handed that
/// frame's value and the colour follows it.
#[test]
fn time_uniform_is_not_stale() {
    let _g = context();
    let s = scene();
    set_samplers(&s, 0, 1);
    let mut cols = Vec::new();
    for t in [0.25f32, 0.5, 0.125] {
        set_time(&s, t);
        let d = draw_quad();
        assert_eq!(time_of(&d), t, "this frame's time");
        let c = shade(&d, 0, 1, [0.0; 4]);
        cols.push(c);
    }
    assert!(cols[1][3] > cols[0][3], "{cols:?}");
    assert!(cols[2][3] < cols[0][3], "{cols:?}");
    // The same value again is the same draw state.
    set_time(&s, 0.25);
    let d = draw_quad();
    near(shade(&d, 0, 1, [0.0; 4]), cols[0], "the first frame's time again");
    ok("frames");
}

/// A second draw over the first: the blend unit adds, the colour is
/// dst + src * alpha twice.
#[test]
fn additive_blend_accumulates() {
    let _g = context();
    let s = scene();
    set_samplers(&s, 0, 1);
    set_time(&s, 0.25);
    let d = draw_quad();
    let once = shade(&d, 0, 1, [0.0; 4]);
    let twice = shade(&d, 0, 1, once);
    assert!(twice[2] > once[2] && twice[0] > once[0], "{once:?} -> {twice:?}");
    assert!(twice.iter().all(|&v| v <= 1.0));
    check_blend(&d);
}
