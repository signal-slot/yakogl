// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's shadow scene on the Mali back end, through the GL API on the
//! host: pass 1 draws into a framebuffer whose only attachment is a depth
//! texture (no colour attachment, `GL_FRAMEBUFFER_COMPLETE` required); pass
//! 2 samples that texture in a second program and draws to the default
//! framebuffer.
//!
//! The real Mali back end runs over a mock OSAL. No GPU runs, so the mock's
//! `submit` plays the part of the depth-only frame's Z/S write-back (the
//! depth in the low 24 bits, rows below h/2 cleared to 1.0, the rest at the
//! quad's depth) and nothing draws pixels: the default framebuffer cannot
//! be read back. Pass 2's expected colours are therefore computed from the
//! depth texels the second draw is handed (the R32F storage
//! `mali_depth_resolve` fills): the test runs the second program's
//! colour ramp on each texel on the CPU and checks that the texels give
//! the colours of the depth the pass drew (0.75 where cleared to 1.0).
//! The second program's shaders are simple ones the online compiler
//! accepts; glmark2's own shadow shaders still refuse (BOOL_AND and
//! CONTROL_FLOW, see docs/notes/mali-glmark2-forecast.md).
//!
//! Which report: the depth texture is the one the report allows. Under the
//! default (ES 3.0) report it is sized (GL_DEPTH_COMPONENT16/24/32F): the
//! unsized GL_DEPTH_COMPONENT is incomplete there (ES 3.0 4.4.4.2,
//! proto-depthtex's finding). Under `report-es2`, glmark2-es2's report (the
//! board library's default), it is GL_OES_depth_texture's unsized
//! GL_DEPTH_COMPONENT, glmark2's own form.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_shadow_pass
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_shadow_pass
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, Ordering};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::cs::Instr;
use yakogl::mali::genxml::v10::{FramebufferParameters, ZsFormat, ZsTarget};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
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

/// The scene the mock GPU renders into the next frame with a Z/S target:
/// rows below `SPLIT` at depth `LOW`, the rest at `HIGH` (unorm24 words).
static ARMED: AtomicBool = AtomicBool::new(false);
static SPLIT: AtomicU32 = AtomicU32::new(0);
static LOW: AtomicU32 = AtomicU32::new(0);
static HIGH: AtomicU32 = AtomicU32::new(0);
/// What the armed frame's ZS target was: (write format, row stride, rows written).
static ZS_SEEN: Mutex<Option<(u32, u32, u32)>> = Mutex::new(None);

/// The GPU's part of one kick: when armed, the fragment job's Z/S
/// write-back of the scene into the frame's ZS target.
unsafe fn play_zs_writeback(k: OsalMaliKick) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    let Some(cpu) = cpu_of(k.stream_gpu, k.stream_bytes as usize) else { return };
    let words = core::slice::from_raw_parts(cpu as *const u64, k.stream_bytes as usize / 8);
    let Some(run) = words.iter().position(|&w| matches!(Instr::decode(w), Ok(Instr::RunFragment(_)))) else {
        return;
    };
    let fbd = words[..run].iter().rev().find_map(|&w| match Instr::decode(w) {
        Ok(Instr::Move48(m)) if m.destination == 40 => Some(m.immediate & !63),
        _ => None,
    });
    let Some(fbd) = fbd else { return };
    let Some(p) = cpu_of(fbd, 192) else { return };
    let params = FramebufferParameters::unpack(&*(p as *const [u8; 64])).unwrap();
    if !params.has_zs_crc_extension {
        return;
    }
    let zs = ZsTarget::unpack(&*((p + 128) as *const [u8; 64])).unwrap();
    /* the unpacked sizes are the frame's (v10.xml's minus-one fields) */
    let rows = params.height as usize;
    let cols = params.width as usize;
    let stride = zs.row_stride as usize;
    let Some(buf) = cpu_of(zs.base, stride * rows) else { return };
    let (split, low, high) =
        (SPLIT.load(Ordering::Relaxed) as usize, LOW.load(Ordering::Relaxed), HIGH.load(Ordering::Relaxed));
    for y in 0..rows {
        let row = core::slice::from_raw_parts_mut((buf + y * stride) as *mut u32, cols);
        for w in row {
            *w = (*w & 0xff00_0000) | if y < split { low } else { high };
        }
    }
    *ZS_SEEN.lock().unwrap() = Some((zs.write_format.0, zs.row_stride, rows as u32));
    ARMED.store(false, Ordering::Relaxed);
}

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    if len == core::mem::size_of::<OsalMaliKick>() {
        play_zs_writeback(*kick.cast::<OsalMaliKick>());
    }
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
#[derive(Clone, Copy, Debug)]
struct Seen {
    index_count: u32,
    target_gpu: u64,
    num_textures: u8,
    tex_format: u8,
    tex_incomplete: u8,
    tex_base_gpu: u64,
    tex_w: u32,
    tex_h: u32,
    tex_pitch: u32,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    let t = &dr.textures[0];
    SEEN.lock().unwrap().push(Seen {
        index_count: dr.index_count,
        target_gpu: dr.target_gpu,
        num_textures: dr.num_textures,
        tex_format: t.format,
        tex_incomplete: t.incomplete,
        tex_base_gpu: t.base_gpu,
        tex_w: t.width,
        tex_h: t.height,
        tex_pitch: 4 * t.stride_texels,
        mali,
    });
    mali
}

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

const N: GLsizei = 32;

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
    ARMED.store(false, Ordering::Relaxed);
    *ZS_SEEN.lock().unwrap() = None;
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
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        p
    }
}

const VS1: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS1: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
const VS2: &str = "attribute vec4 p;\nvarying vec2 uv;\nvoid main() { gl_Position = p; uv = p.xy * 0.5 + 0.5; }\n";
/// The second program: a depth-dependent colour from the shadow map,
/// `clamp(d * 0.5 + 0.25, 0.25, 1.0)`. The `d < x ? a : b` compare of
/// glmark2's shadow shader and `step` are refused at the tip (BOOL_AND,
/// an integer or boolean value), so the colour is a clamped ramp.
const FS2: &str = "precision mediump float;\nuniform sampler2D shadow;\nvarying vec2 uv;\nvoid main() { float d = texture2D(shadow, uv).r; gl_FragColor = vec4(clamp(d * 0.5 + 0.25, 0.25, 1.0)); }\n";

/// A quad from y `y0` to 1 at constant depth `z`, as two triangles.
fn quad(y0: f32, z: f32, what: &str) -> Seen {
    let p: [f32; 16] = [-1.0, y0, z, 1.0, -1.0, 1.0, z, 1.0, 1.0, y0, z, 1.0, 1.0, 1.0, z, 1.0];
    static IDX: [u16; 6] = [0, 2, 1, 2, 1, 3];
    SEEN.lock().unwrap().clear();
    let before = backend::draws_refused();
    // SAFETY: `p` and `IDX` outlive the draw, which `glFinish` completes.
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, p.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        glFinish();
        glGetError()
    };
    assert_eq!(e, GL_NO_ERROR, "{what}: glGetError 0x{e:x}, draws {:?}", SEEN.lock().unwrap());
    assert_eq!(backend::draws_refused(), before, "{what}: a draw was refused");
    let s = SEEN.lock().unwrap().iter().rev().find(|s| s.index_count == 6).copied();
    let s = s.unwrap_or_else(|| panic!("{what}: the draw did not reach the back end"));
    assert_eq!(s.mali, 0, "{what}: mali_draw refused");
    s
}

fn unorm24(d: f32) -> u32 {
    (d.clamp(0.0, 1.0) as f64 * f64::from(0xff_ffffu32)).round() as u32
}

/// A depth texture: its internal format, pixel type and size.
#[derive(Clone, Copy)]
struct Shape {
    internal: GLenum,
    ty: GLenum,
    w: GLsizei,
    h: GLsizei,
}

/// A depth texture of `s` alone as GL_DEPTH_ATTACHMENT (glmark2's
/// `DepthRenderTarget::setup`). The ES 2.0 report has no sized depth
/// texture: there it is GL_OES_depth_texture's unsized GL_DEPTH_COMPONENT
/// (GL_UNSIGNED_INT for 24 bits and float, GL_UNSIGNED_SHORT for 16).
fn depth_target(mut s: Shape) -> (GLuint, GLuint) {
    if cfg!(feature = "report-es2") {
        s.ty = if s.ty == GL_UNSIGNED_SHORT { GL_UNSIGNED_SHORT } else { GL_UNSIGNED_INT };
        s.internal = GL_DEPTH_COMPONENT;
    }
    let (mut tex, mut fbo) = (0, 0);
    // SAFETY: room for one name each; null pixels.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, s.internal as GLint, s.w, s.h, 0, GL_DEPTH_COMPONENT, s.ty, core::ptr::null());
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "depth-only FBO: status 0x{st:x}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    ok("depth target");
    (tex, fbo)
}

/// The depth pass (glmark2's `DepthRenderTarget::enable`: the texture
/// attached again, the viewport its size, colour writes off, the depth
/// cleared to 1.0), then a quad over the top half (GL's y from h/2) at NDC
/// `z`, depth test LESS. The mock GPU writes the scene that pass makes:
/// rows below h/2 at 1.0 (cleared), the rest at `z * 0.5 + 0.5`.
fn depth_pass(s: Shape, tex: GLuint, fbo: GLuint, z: f32) {
    let p = program(VS1, FS1);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        glViewport(0, 0, s.w, s.h);
        glColorMask(0, 0, 0, 0);
        glClearDepthf(1.0);
        glClear(GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
    }
    ok("clear");
    SPLIT.store(s.h as u32 / 2, Ordering::Relaxed);
    LOW.store(unorm24(1.0), Ordering::Relaxed);
    HIGH.store(unorm24(z * 0.5 + 0.5), Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    let d = quad(0.0, z, "depth pass");
    assert_eq!(d.target_gpu, 0, "the depth-only pass has no colour surface");
    assert!(!ARMED.load(Ordering::Relaxed), "the depth-only pass submitted no frame with a Z/S target");
    let (fmt, stride, rows) = ZS_SEEN.lock().unwrap().expect("the Z/S target");
    assert_eq!(fmt, ZsFormat::D24S8.0, "the depth-only frame's ZS write format");
    assert_eq!((stride, rows), (4 * s.w as u32, s.h as u32));
    // SAFETY: plain GL calls (glmark2's `DepthRenderTarget::disable`).
    unsafe {
        glDisable(GL_DEPTH_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, N, N);
        glColorMask(1, 1, 1, 1);
    }
    ok("unbind");
}

/// The second pass: `tex` on unit 0, drawn to the default framebuffer.
fn sample_pass(s: Shape, tex: GLuint) -> Seen {
    let p = program(VS2, FS2);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glUniform1i(glGetUniformLocation(p, c"shadow".as_ptr()), 0);
    }
    ok("second pass setup");
    let d = quad(-1.0, 0.0, "sample pass");
    assert_ne!(d.target_gpu, 0, "the second pass draws to the default framebuffer");
    assert_eq!(d.num_textures, 1);
    assert_eq!(d.tex_incomplete, 0, "the depth texture is complete");
    assert_eq!((d.tex_w, d.tex_h), (s.w as u32, s.h as u32));
    assert_eq!(d.tex_format, backend::GLES_TEXFMT_R32F, "the resolved R32_FLOAT texels");
    assert!(d.tex_pitch >= 4 * s.w as u32, "row pitch {} for {} texels", d.tex_pitch, s.w);
    d
}

/// The R32F texel (x, y) the sampling draw was handed, at the row pitch
/// its texture descriptor names.
fn texel_at(d: &Seen, x: u32, y: u32) -> f32 {
    let cpu = cpu_of(d.tex_base_gpu + u64::from(y * d.tex_pitch + 4 * x), 4).expect("the texture's buffer");
    // SAFETY: inside a leaked buffer, 4-byte aligned (R32F texels).
    unsafe { *(cpu as *const f32) }
}

/// Every texel against the scene: rows below h/2 at 1.0, the rest at
/// `top`, within one unorm16 step (the coarsest format's).
fn check_texels(s: Shape, d: &Seen, top: f32, what: &str) {
    let tol = 1.5 / f32::from(0xffffu16);
    for y in 0..s.h as u32 {
        for x in 0..s.w as u32 {
            let want = if y < s.h as u32 / 2 { 1.0 } else { top };
            let t = texel_at(d, x, y);
            assert!((t - want).abs() <= tol, "{what}: texel ({x}, {y}) is {t}, the depth pass wrote {want}");
        }
    }
}

/// The colour pass 2's colour ramp gives a texel, on the CPU.
fn shade(d: f32) -> f32 {
    (d * 0.5 + 0.25).clamp(0.25, 1.0)
}

/// Pass 2's expected colour at every texel (from what the depth pass wrote:
/// 1.0 below h/2, `top` above) against the colour of the texels the draw was
/// handed.
fn check_colours(s: Shape, d: &Seen, top: f32, what: &str) {
    for y in 0..s.h as u32 {
        for x in 0..s.w as u32 {
            let want = shade(if y < s.h as u32 / 2 { 1.0 } else { top });
            let got = shade(texel_at(d, x, y));
            assert!((got - want).abs() < 1e-3, "{what}: pixel ({x}, {y}) is {got}, expected {want}");
        }
    }
}

/// Frames of glmark2 shadow's two passes; the FBO is complete (checked in
/// `depth_target`), pass 2 draws the depth-dependent colours.
fn frames(s: Shape, zs: &[f32]) {
    let (tex, fbo) = depth_target(s);
    for (i, &z) in zs.iter().enumerate() {
        depth_pass(s, tex, fbo, z);
        let d = sample_pass(s, tex);
        let top = z * 0.5 + 0.5;
        let what = format!("format 0x{:x} {}x{} frame {i}", s.internal, s.w, s.h);
        check_texels(s, &d, top, &what);
        check_colours(s, &d, top, &what);
    }
    assert_eq!(backend::draws_refused(), 0);
}

/// Depth 0.5 (NDC 0.0): the top half draws colour 0.5, the rest 0.75.
#[test]
fn shadow_pass_24_bit_top_half_nearer() {
    let _g = context();
    frames(Shape { internal: GL_DEPTH_COMPONENT24, ty: GL_UNSIGNED_INT, w: N, h: N }, &[0.0]);
}

/// Depth 0.875 (NDC 0.75): the top half draws colour 0.6875.
#[test]
fn shadow_pass_16_bit_far_quad() {
    let _g = context();
    frames(Shape { internal: GL_DEPTH_COMPONENT16, ty: GL_UNSIGNED_SHORT, w: N, h: N }, &[0.75]);
}

#[test]
fn shadow_pass_32f_top_half_nearer() {
    let _g = context();
    frames(Shape { internal: GL_DEPTH_COMPONENT32F, ty: GL_FLOAT, w: N, h: N }, &[-0.5]);
}

/// The depth pass runs again every frame: the next pass 2 shows the new depth.
#[test]
fn shadow_pass_follows_each_frames_depth() {
    let _g = context();
    frames(Shape { internal: GL_DEPTH_COMPONENT24, ty: GL_UNSIGNED_INT, w: N, h: N }, &[0.0, 0.75, -0.5]);
}

/// glmark2 shadow's own texture size: 1600x1200 (the canvas twice over).
#[test]
fn shadow_pass_glmark2_texture_size() {
    let _g = context();
    frames(Shape { internal: GL_DEPTH_COMPONENT24, ty: GL_UNSIGNED_INT, w: 1600, h: 1200 }, &[0.0, 0.75]);
}
