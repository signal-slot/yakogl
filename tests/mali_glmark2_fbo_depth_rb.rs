// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's render-to-texture scenes (terrain, refract) on the Mali back
//! end, through the GL API on the host: an FBO whose colour attachment is an
//! RGBA texture and whose depth attachment is a GL_DEPTH_COMPONENT16
//! renderbuffer, drawn with the depth test on, then the texture sampled onto
//! a quad of the default framebuffer.
//!
//! The mock OSAL runs no GPU, so nothing draws pixels. The mock back end's
//! `draw` hook therefore plays the rasteriser at the level of the draw the
//! back end is handed (`GlesDraw`): it fills the target at `target_gpu` from
//! the quad's rectangle and depth, using the draw's own `depth_test`,
//! `depth_func`, `depth_write` and `depth_id` (one CPU depth plane per depth
//! object); the sampling draw is checked to read the texels at the FBO's
//! colour surface. What the test pins is the GL core's hand-over
//! (completeness, one depth object shared by both draws, the colour surface
//! being the texture's storage, delete and recreate) with the Mali's
//! `mali_draw` accepting every draw; the pixel values are the emulation's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_fbo_depth_rb
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
use yakogl::gl::framebuffer::*;
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

unsafe extern "C" fn m_submit(_kick: *const c_void, _len: usize) -> i64 {
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

/// What the back end was given for one draw (the fields the test reads).
#[derive(Clone, Copy, Debug)]
struct Seen {
    index_count: u32,
    target_gpu: u64,
    target_w: u32,
    target_h: u32,
    target_pitch: u32,
    depth_test: bool,
    depth_func: u32,
    depth_write: bool,
    depth_id: u32,
    depth_gen: u32,
    num_textures: u8,
    tex_incomplete: u8,
    tex_base_gpu: u64,
    tex_w: u32,
    tex_h: u32,
    tex_pitch: u32,
    mali: i32,
}

/// One quad the next draw rasterises in the emulation: an NDC rectangle at
/// NDC depth `z`, colour `rgba`.
#[derive(Clone, Copy)]
struct Quad {
    rect: [f32; 4],
    z: f32,
    rgba: [u8; 4],
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
static QUAD: Mutex<Option<Quad>> = Mutex::new(None);
/// The CPU depth planes, by `depth_id` (the emulated Z buffer, 1.0 cleared).
static PLANES: Mutex<Vec<(u32, u32, Vec<f32>)>> = Mutex::new(Vec::new());

/// Plays the rasteriser for one draw into the mock buffers: a clear fills the
/// colour surface and the depth plane, a quad is depth-tested per pixel.
unsafe fn rasterise(d: &GlesDraw, q: Option<Quad>) {
    let (w, h) = (d.target_w as usize, d.target_h as usize);
    let pitch = if d.target_pitch != 0 { 4 * d.target_pitch as usize } else { 4 * w };
    let Some(cpu) = cpu_of(d.target_gpu, pitch * h) else { return };
    let px = core::slice::from_raw_parts_mut(cpu as *mut u8, pitch * h);
    let mut planes = PLANES.lock().unwrap();
    let key = (d.depth_id, d.depth_gen);
    if d.depth_id != 0 && !planes.iter().any(|p| (p.0, p.1) == key) {
        planes.retain(|p| p.0 != d.depth_id);
        planes.push((d.depth_id, d.depth_gen, vec![1.0; w * h]));
    }
    if d.do_clear {
        let c = d.clear_rgba.map(|c| (c.clamp(0.0, 1.0) * 255.0).round() as u8);
        for y in 0..h {
            for x in 0..w {
                px[y * pitch + 4 * x..][..4].copy_from_slice(&c);
            }
        }
    }
    if d.do_clear_depth {
        if let Some(p) = planes.iter_mut().find(|p| (p.0, p.1) == key) {
            p.2.fill(d.clear_depth);
        }
    }
    let Some(q) = q else { return };
    let plane = planes.iter_mut().find(|p| (p.0, p.1) == key).map(|p| &mut p.2);
    let z = q.z * 0.5 + 0.5;
    let mut plane = plane;
    for y in 0..h {
        for x in 0..w {
            let (nx, ny) = ((x as f32 + 0.5) / w as f32 * 2.0 - 1.0, (y as f32 + 0.5) / h as f32 * 2.0 - 1.0);
            if nx < q.rect[0] || nx >= q.rect[2] || ny < q.rect[1] || ny >= q.rect[3] {
                continue;
            }
            if d.depth_test {
                let Some(pl) = plane.as_deref_mut() else { continue };
                let old = pl[y * w + x];
                let pass = match d.depth_func {
                    backend::GLES_CMP_LESS => z < old,
                    backend::GLES_CMP_LEQUAL => z <= old,
                    backend::GLES_CMP_ALWAYS => true,
                    f => panic!("depth func {f} not emulated"),
                };
                if !pass {
                    continue;
                }
                if d.depth_write {
                    pl[y * w + x] = z;
                }
            }
            px[y * pitch + 4 * x..][..4].copy_from_slice(&q.rgba);
        }
    }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    let t = &dr.textures[0];
    let quad = if dr.index_count == 6 { QUAD.lock().unwrap().take() } else { None };
    if mali == 0 {
        rasterise(dr, quad);
    }
    SEEN.lock().unwrap().push(Seen {
        index_count: dr.index_count,
        target_gpu: dr.target_gpu,
        target_w: dr.target_w,
        target_h: dr.target_h,
        target_pitch: 4 * if dr.target_pitch != 0 { dr.target_pitch } else { dr.target_w },
        depth_test: dr.depth_test,
        depth_func: dr.depth_func,
        depth_write: dr.depth_write,
        depth_id: dr.depth_id,
        depth_gen: dr.depth_gen,
        num_textures: dr.num_textures,
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
    *QUAD.lock().unwrap() = None;
    PLANES.lock().unwrap().clear();
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

const VS: &str = "attribute vec4 p;\nvarying vec2 uv;\nvoid main() { gl_Position = p; uv = p.xy * 0.5 + 0.5; }\n";
/// The RTT pass: a flat colour from a uniform.
const FS_FLAT: &str =
    "precision mediump float;\nuniform vec4 c;\nvarying vec2 uv;\nvoid main() { gl_FragColor = c; }\n";
/// The second pass: the texture's texel at the quad's coordinate.
const FS_TEX: &str = "precision mediump float;\nuniform sampler2D t;\nvarying vec2 uv;\nvoid main() { gl_FragColor = texture2D(t, uv); }\n";

/// An RTT target as glmark2 makes it: an RGBA texture (linear filtering, no
/// mip chain) and a GL_DEPTH_COMPONENT16 renderbuffer on one FBO.
#[derive(Clone, Copy)]
struct Rtt {
    tex: GLuint,
    rb: GLuint,
    fbo: GLuint,
    w: GLsizei,
    h: GLsizei,
}

fn make_rtt(w: GLsizei, h: GLsizei) -> Rtt {
    let (mut tex, mut rb, mut fbo) = (0, 0, 0);
    // SAFETY: room for one name each; null pixels.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, w, h);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "{w}x{h} RTT FBO: status 0x{st:x}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    ok("make rtt");
    Rtt { tex, rb, fbo, w, h }
}

fn delete_rtt(r: Rtt) {
    // SAFETY: one name each.
    unsafe {
        glDeleteFramebuffers(1, &r.fbo);
        glDeleteRenderbuffers(1, &r.rb);
        glDeleteTextures(1, &r.tex);
    }
    ok("delete rtt");
}

/// One quad of `rect` (NDC x0, y0, x1, y1) at NDC depth `z` and `rgba`, with
/// the current program, as two triangles.
fn quad(p: GLuint, rect: [f32; 4], z: f32, rgba: [u8; 4], what: &str) -> Seen {
    let [x0, y0, x1, y1] = rect;
    let v: [f32; 16] = [x0, y0, z, 1.0, x0, y1, z, 1.0, x1, y0, z, 1.0, x1, y1, z, 1.0];
    static IDX: [u16; 6] = [0, 2, 1, 2, 1, 3];
    SEEN.lock().unwrap().clear();
    *QUAD.lock().unwrap() = Some(Quad { rect, z, rgba });
    let before = backend::draws_refused();
    // SAFETY: `v` and `IDX` outlive the draw, which `glFinish` completes.
    let e = unsafe {
        let c = rgba.map(|b| f32::from(b) / 255.0);
        let loc = glGetUniformLocation(p, c"c".as_ptr());
        if loc >= 0 {
            glUniform4f(loc, c[0], c[1], c[2], c[3]);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, v.as_ptr() as *const c_void);
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

const RED: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 255, 0, 255];
const BLUE: [u8; 4] = [0, 0, 255, 255];
const BLACK: [u8; 4] = [0, 0, 0, 255];

/// The two RTT frames' scene: a far red quad over the left two thirds, a
/// near green quad over the middle third, then a far blue quad over the
/// right two thirds drawn last. Where green is, it must stay green (blue
/// is farther); red and blue meet at the middle, blue over the right.
fn expected(x: usize, w: usize) -> [u8; 4] {
    let nx = (x as f32 + 0.5) / w as f32 * 2.0 - 1.0;
    let (red, green, blue) = (nx < 0.33, (-0.33..0.33).contains(&nx), nx >= -0.33);
    if green {
        GREEN
    } else if blue {
        BLUE
    } else if red {
        RED
    } else {
        BLACK
    }
}

/// The RTT pass: clear, then the three quads with the depth test on. Returns
/// the last RTT draw.
fn rtt_pass(r: Rtt) -> Seen {
    let p = program(VS, FS_FLAT);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glBindFramebuffer(GL_FRAMEBUFFER, r.fbo);
        glViewport(0, 0, r.w, r.h);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
    }
    ok("rtt setup");
    let a = quad(p, [-1.0, -1.0, 0.33, 1.0], 0.5, RED, "far red");
    let b = quad(p, [-0.33, -1.0, 0.33, 1.0], -0.5, GREEN, "near green");
    let c = quad(p, [-0.33, -1.0, 1.0, 1.0], 0.5, BLUE, "far blue");
    for (s, what) in [(a, "far red"), (b, "near green"), (c, "far blue")] {
        assert_eq!((s.target_w, s.target_h), (r.w as u32, r.h as u32), "{what}: target size");
        assert_ne!(s.target_gpu, 0, "{what}: the FBO's colour surface");
        assert!(s.depth_test && s.depth_write, "{what}: depth test and write on");
        assert_eq!(s.depth_func, backend::GLES_CMP_LESS, "{what}: depth func");
        assert_ne!(s.depth_id, 0, "{what}: the renderbuffer is the draw's depth object");
    }
    assert_eq!((a.depth_id, a.depth_gen), (b.depth_id, b.depth_gen), "one depth object for the pass");
    assert_eq!((a.depth_id, a.depth_gen), (c.depth_id, c.depth_gen), "one depth object for the pass");
    assert_eq!(a.target_gpu, c.target_gpu);
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_DEPTH_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, N, N);
    }
    ok("unbind");
    c
}

/// The colour surface of the RTT pass, row `y` pixel `x` (the texture's
/// pitch, which the sampling draw names).
fn texel(gpu: u64, pitch: u32, x: u32, y: u32) -> [u8; 4] {
    let cpu = cpu_of(gpu + u64::from(y * pitch + 4 * x), 4).expect("the texture's buffer");
    // SAFETY: inside a leaked buffer.
    unsafe { *(cpu as *const [u8; 4]) }
}

/// The second pass: the RTT texture on unit 0, a full-screen quad on the
/// default framebuffer; the draw must read the texels the RTT pass wrote.
fn sample_pass(r: Rtt, rtt: Seen, what: &str) {
    let p = program(VS, FS_TEX);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, r.tex);
        glUniform1i(glGetUniformLocation(p, c"t".as_ptr()), 0);
    }
    ok("second pass setup");
    let d = quad(p, [-1.0, -1.0, 1.0, 1.0], 0.0, BLACK, "sample pass");
    assert_ne!(d.target_gpu, 0, "{what}: the default framebuffer is drawn");
    assert_ne!(d.target_gpu, rtt.target_gpu);
    assert_eq!(d.num_textures, 1);
    assert_eq!(d.tex_incomplete, 0, "{what}: the RTT texture is complete");
    assert_eq!((d.tex_w, d.tex_h), (r.w as u32, r.h as u32), "{what}: texture size");
    assert_eq!(d.tex_base_gpu, rtt.target_gpu, "{what}: the texture is the FBO's colour surface");
    assert!(d.tex_pitch >= 4 * r.w as u32, "{what}: row pitch {}", d.tex_pitch);
    assert_eq!(d.tex_pitch, rtt.target_pitch, "{what}: the texture's pitch is the colour surface's");
    for y in 0..r.h as u32 {
        for x in 0..r.w as u32 {
            let got = texel(d.tex_base_gpu, d.tex_pitch, x, y);
            let want = expected(x as usize, r.w as usize);
            assert_eq!(got, want, "{what}: texel ({x}, {y})");
        }
    }
}

fn frame(r: Rtt, what: &str) {
    let rtt = rtt_pass(r);
    sample_pass(r, rtt, what);
}

/// The FBO is complete at a POT size and at NPOT ones (glmark2 terrain's
/// and refract's are the canvas size, rarely a power of two).
#[test]
fn rtt_fbo_complete_pot_and_npot() {
    let _g = context();
    for (w, h) in [(64, 64), (256, 128), (100, 60), (200, 150), (800, 600)] {
        let r = make_rtt(w, h);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, r.fbo);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{w}x{h}");
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
        }
        ok("status");
        delete_rtt(r);
    }
}

#[test]
fn rtt_nearer_quad_wins_pot() {
    let _g = context();
    let r = make_rtt(64, 64);
    frame(r, "64x64");
}

#[test]
fn rtt_nearer_quad_wins_npot() {
    let _g = context();
    let r = make_rtt(100, 60);
    frame(r, "100x60");
}

/// Delete the renderbuffer, the FBO and the texture, make them again at
/// another size and draw the same frame: no GL error, and the new depth
/// storage starts from the clear (a stale plane would hide the quads).
#[test]
fn rtt_recreate_renderbuffer_and_fbo() {
    let _g = context();
    let r = make_rtt(64, 64);
    frame(r, "first");
    delete_rtt(r);
    for (i, (w, h)) in [(64, 64), (100, 60), (64, 64)].into_iter().enumerate() {
        let r = make_rtt(w, h);
        frame(r, &format!("recreated {i} {w}x{h}"));
        delete_rtt(r);
    }
    assert_eq!(backend::draws_refused(), 0);
}

/// glmark2 resizes the target by deleting and respecifying only the
/// renderbuffer's storage and the texture; the FBO stays.
#[test]
fn rtt_respecify_renderbuffer_storage() {
    let _g = context();
    let r = make_rtt(64, 64);
    frame(r, "before");
    let r2 = Rtt { w: 100, h: 60, ..r };
    // SAFETY: plain GL calls.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, r.rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, r2.w, r2.h);
        glBindRenderbuffer(GL_RENDERBUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, r.tex);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, r2.w, r2.h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glBindTexture(GL_TEXTURE_2D, 0);
        glBindFramebuffer(GL_FRAMEBUFFER, r.fbo);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    ok("respecify");
    frame(r2, "after");
    delete_rtt(r2);
}
