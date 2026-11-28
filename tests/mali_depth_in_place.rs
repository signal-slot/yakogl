// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! G1 of the glmark2 GL audit (scenes 24 shadow and 25 refract): on the
//! Mali, a draw that samples a depth texture a depth-only pass has just
//! rendered reads the attachment pair's D24S8 buffer in place, as Z24X8
//! UNORM, as Mesa panfrost samples a depth resource. Before, the GL core
//! waited for the depth pass in the middle of the frame and
//! `mali_depth_resolve` converted every word of the buffer into the R32F
//! texels on the CPU (`backend::samples_rendered_depth` had no Mali case).
//!
//! The real Mali back end runs over a mock OSAL whose fences complete only
//! when waited for, so every wait the GL core or the back end makes is
//! counted. The mock's `submit` plays the depth pass's Z/S write-back (as
//! tests/mali_shadow_pass.rs does) and records the `ZS Target` base, and it
//! reads every `RUN_IDVS`'s fragment resource table (r4) to find the
//! texture descriptor the sampling draw was built with. Three frames of
//! glmark2 shadow's two passes then check that:
//!
//! - no wait is made between the depth pass's draw and the sampling draw
//!   (the depth pass is submitted, not waited for);
//! - the sampling draw's binding names the depth object
//!   (`GlesTextureBind::depth_id`);
//! - its texture descriptor reads the depth buffer: Z24X8 UNORM, a RAW32
//!   clump, the `ZS Target`'s address and row stride, the texture's size,
//!   one level, and the red channel read as (d, d, d, 1) under the ES 2.0
//!   report (GL_OES_depth_texture) or (d, 0, 0, 1) under ES 3.0.
//!
//! With `mali::backend::DEPTH_IN_PLACE` off the same frames take the old
//! path: one wait before the sampling draw, and the R32F texels.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_depth_in_place
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_depth_in_place
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
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
use yakogl::mali::frame::{swizzle4, SWIZZLE_RRR1};
use yakogl::mali::genxml::v10::{
    ClumpFormat, Format, FramebufferParameters, GenericPlane, Resource, RgbComponentOrder, Texture, ZsTarget,
};
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Fences handed out, and the last one completed: a fence completes only
/// when it is waited for, so a kick is pending until someone waits.
static FENCES: AtomicI64 = AtomicI64::new(0);
static COMPLETED: AtomicI64 = AtomicI64::new(0);
/// Waits made on the OSAL.
static WAITS: AtomicU32 = AtomicU32::new(0);

/// Buffers made so far: (GPU VA, CPU address, bytes).
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

fn bytes_at<const L: usize>(gpu: u64) -> Option<[u8; L]> {
    let cpu = cpu_of(gpu, L)?;
    // SAFETY: `L` bytes inside a leaked buffer (`cpu_of`).
    Some(unsafe { *(cpu as *const [u8; L]) })
}

/// The depth the mock GPU writes into the next frame with a Z/S target
/// (unorm24, every word), and that frame's `ZS Target` (base, row stride).
static ARMED: AtomicBool = AtomicBool::new(false);
static DEPTH: AtomicU32 = AtomicU32::new(0);
static ZS_SEEN: Mutex<Option<(u64, u32)>> = Mutex::new(None);

/// A fragment texture descriptor a `RUN_IDVS` was built with, and its
/// first plane.
#[derive(Clone, Copy, Debug)]
struct TexSeen {
    tex: Texture,
    plane: GenericPlane,
}
static TEX_SEEN: Mutex<Vec<TexSeen>> = Mutex::new(Vec::new());

/// The registers each `RUN_IDVS` of `words` runs with.
fn regs_at_idvs(words: &[u64]) -> Vec<BTreeMap<u32, u32>> {
    let mut r = BTreeMap::new();
    let mut out = Vec::new();
    for &w in words {
        match Instr::decode(w) {
            Ok(Instr::Move32(m)) => {
                r.insert(m.destination, m.immediate);
            }
            Ok(Instr::Move48(m)) => {
                r.insert(m.destination, m.immediate as u32);
                r.insert(m.destination + 1, (m.immediate >> 32) as u32);
            }
            Ok(Instr::RunIdvs(_)) => out.push(r.clone()),
            _ => {}
        }
    }
    out
}

/// The fragment textures of every draw in the kick: the fragment resource
/// table (r4, its low bits the table count), entry 4 (the textures).
fn record_textures(words: &[u64]) {
    for regs in regs_at_idvs(words) {
        let srt = u64::from(regs.get(&4).copied().unwrap_or(0)) | u64::from(regs.get(&5).copied().unwrap_or(0)) << 32;
        let srt = srt & !63;
        if srt == 0 {
            continue;
        }
        let Some(entry) = bytes_at::<16>(srt + 4 * 16) else { continue };
        let Ok(res) = Resource::unpack(&entry) else { continue };
        let n = res.size as usize / Texture::LENGTH;
        for k in 0..n {
            let Some(raw) = bytes_at::<{ Texture::LENGTH }>(res.address + (k * Texture::LENGTH) as u64) else {
                continue;
            };
            let Ok(tex) = Texture::unpack(&raw) else { continue };
            let Some(praw) = bytes_at::<{ GenericPlane::LENGTH }>(tex.surfaces) else { continue };
            let Ok(plane) = GenericPlane::unpack(&praw) else { continue };
            TEX_SEEN.lock().unwrap().push(TexSeen { tex, plane });
        }
    }
}

/// The GPU's part of one kick: the fragment textures it reads, and, when
/// armed, the fragment job's Z/S write-back into the frame's ZS target.
unsafe fn play_kick(k: OsalMaliKick) {
    let Some(cpu) = cpu_of(k.stream_gpu, k.stream_bytes as usize) else { return };
    let words = core::slice::from_raw_parts(cpu as *const u64, k.stream_bytes as usize / 8);
    record_textures(words);
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
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
    let (rows, cols, stride) = (params.height as usize, params.width as usize, zs.row_stride as usize);
    let Some(buf) = cpu_of(zs.base, stride * rows) else { return };
    let depth = DEPTH.load(Ordering::Relaxed);
    for y in 0..rows {
        let row = core::slice::from_raw_parts_mut((buf + y * stride) as *mut u32, cols);
        for w in row {
            *w = (*w & 0xff00_0000) | depth;
        }
    }
    *ZS_SEEN.lock().unwrap() = Some((zs.base, zs.row_stride));
    ARMED.store(false, Ordering::Relaxed);
}

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    if len == core::mem::size_of::<OsalMaliKick>() {
        play_kick(*kick.cast::<OsalMaliKick>());
    }
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(fence: i64, _timeout_us: i64) -> i32 {
    WAITS.fetch_add(1, Ordering::Relaxed);
    COMPLETED.fetch_max(fence, Ordering::Relaxed);
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    COMPLETED.load(Ordering::Relaxed)
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

/// What the back end was given for one draw, and the waits made before it.
#[derive(Clone, Copy, Debug)]
struct Seen {
    index_count: u32,
    target_gpu: u64,
    num_textures: u8,
    tex_format: u8,
    depth_id: u32,
    waits: u32,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let waits = WAITS.load(Ordering::Relaxed);
    let mali = yakogl::mali::backend::mali_draw(d);
    let t = &dr.textures[0];
    SEEN.lock().unwrap().push(Seen {
        index_count: dr.index_count,
        target_gpu: dr.target_gpu,
        num_textures: dr.num_textures,
        tex_format: t.format,
        depth_id: if dr.num_textures > 0 { t.depth_id } else { 0 },
        waits,
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
    TEX_SEEN.lock().unwrap().clear();
    ARMED.store(false, Ordering::Relaxed);
    *ZS_SEEN.lock().unwrap() = None;
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
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
/// The second program (tests/mali_shadow_pass.rs's): a colour ramp of the
/// shadow map's depth.
const FS2: &str = "precision mediump float;\nuniform sampler2D shadow;\nvarying vec2 uv;\nvoid main() { float d = texture2D(shadow, uv).r; gl_FragColor = vec4(clamp(d * 0.5 + 0.25, 0.25, 1.0)); }\n";

/// A full-screen quad at NDC depth `z`, two triangles; `finish`: glFinish
/// after it. Returns what the back end was given.
fn quad(z: f32, finish: bool, what: &str) -> Seen {
    let p: [f32; 16] = [-1.0, -1.0, z, 1.0, -1.0, 1.0, z, 1.0, 1.0, -1.0, z, 1.0, 1.0, 1.0, z, 1.0];
    static IDX: [u16; 6] = [0, 2, 1, 2, 1, 3];
    SEEN.lock().unwrap().clear();
    let before = backend::draws_refused();
    // SAFETY: `p` and `IDX` are copied by the GL core into the draw's data
    // (client arrays are staged at the draw).
    let e = unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, p.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        if finish {
            glFinish();
        }
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

/// glmark2's `DepthRenderTarget::setup`: an `N` x `N` depth texture alone
/// as GL_DEPTH_ATTACHMENT; GL_OES_depth_texture's unsized form under the
/// ES 2.0 report, GL_DEPTH_COMPONENT24 under ES 3.0.
fn depth_target() -> (GLuint, GLuint) {
    let internal = if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 };
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
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            internal as GLint,
            N,
            N,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
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

/// One frame of glmark2 shadow: the depth pass into `fbo` at NDC `z` (not
/// waited for, as glmark2 does not), then the ground drawn to the default
/// framebuffer sampling `tex`, then glFinish (the frame's swap). Returns
/// the waits made between the two draws and the sampling draw.
fn frame(tex: GLuint, fbo: GLuint, z: f32, what: &str) -> (u32, Seen) {
    let (p1, p2) = (program(VS1, FS1), program(VS2, FS2));
    // SAFETY: plain GL calls (glmark2's `DepthRenderTarget::enable`).
    unsafe {
        glUseProgram(p1);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glViewport(0, 0, N, N);
        glColorMask(0, 0, 0, 0);
        glClearDepthf(1.0);
        glClear(GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
    }
    ok("depth pass setup");
    DEPTH.store(unorm24(z * 0.5 + 0.5), Ordering::Relaxed);
    ARMED.store(true, Ordering::Relaxed);
    let d = quad(z, false, &format!("{what}: depth pass"));
    assert_eq!(d.target_gpu, 0, "{what}: the depth-only pass has no colour surface");
    let waits_after_depth = WAITS.load(Ordering::Relaxed);
    // SAFETY: plain GL calls (`DepthRenderTarget::disable`, then the
    // ground's program and the shadow map on unit 0).
    unsafe {
        glDisable(GL_DEPTH_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, N, N);
        glColorMask(1, 1, 1, 1);
        glUseProgram(p2);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
        glUniform1i(glGetUniformLocation(p2, c"shadow".as_ptr()), 0);
    }
    ok("sample pass setup");
    TEX_SEEN.lock().unwrap().clear();
    let s = quad(0.0, true, &format!("{what}: sample pass"));
    assert!(!ARMED.load(Ordering::Relaxed), "{what}: the depth pass submitted no frame with a Z/S target");
    assert_ne!(s.target_gpu, 0, "{what}: the sampling draw draws to the default framebuffer");
    assert_eq!(s.num_textures, 1, "{what}");
    (s.waits - waits_after_depth, s)
}

/// The depth texture's descriptor in the sampling kick: the one whose
/// plane reads `base`.
fn depth_descriptor(base: u64, what: &str) -> TexSeen {
    let seen = TEX_SEEN.lock().unwrap();
    *seen.iter().find(|t| t.plane.pointer == base).unwrap_or_else(|| {
        panic!("{what}: no texture of the sampling kick reads the depth buffer 0x{base:x}: {seen:?}")
    })
}

/// Three frames of glmark2 shadow sample the depth pass's buffer in place:
/// no wait between the passes, the binding names the depth object, and the
/// draw's texture descriptor reads the D24S8 buffer as Z24X8 UNORM.
#[test]
fn shadow_frames_sample_the_depth_buffer_in_place() {
    let _g = context();
    assert!(yakogl::mali::backend::DEPTH_IN_PLACE.load(Ordering::Relaxed), "the default is in place");
    let (tex, fbo) = depth_target();
    for (i, z) in [0.0f32, -0.5, 0.75].into_iter().enumerate() {
        let what = format!("frame {i}");
        let (waits, s) = frame(tex, fbo, z, &what);
        assert_eq!(waits, 0, "{what}: the GL core waited for the depth pass before the sampling draw");
        assert_ne!(s.depth_id, 0, "{what}: the binding names no depth object: the texels were resolved");
        assert_eq!(s.tex_format, backend::GLES_TEXFMT_R32F, "{what}: the GL core's binding is the R32F texture");
        let (base, stride) = ZS_SEEN.lock().unwrap().expect("the depth pass's ZS target");
        assert_eq!(stride, 4 * N as u32, "{what}: the D24S8 buffer's row stride");
        let t = depth_descriptor(base, &what);
        assert_eq!(
            t.tex.format,
            Format::Z24X8_UNORM.0 << 12 | RgbComponentOrder::RGBA.0,
            "{what}: the depth buffer read as Z24X8 UNORM"
        );
        assert_eq!(t.plane.clump_format, ClumpFormat::RAW32, "{what}: a RAW32 clump");
        assert_eq!(t.plane.row_stride, stride, "{what}: the plane's row stride is the buffer's");
        assert_eq!((t.tex.width, t.tex.height, t.tex.levels), (N as u32, N as u32, 1), "{what}: size and levels");
        let swizzle = if cfg!(feature = "report-es2") { SWIZZLE_RRR1 } else { swizzle4(0, 4, 4, 5) };
        assert_eq!(t.tex.swizzle, swizzle, "{what}: the depth's red channel read as the report says");
    }
    assert_eq!(backend::draws_refused(), 0);
}

/// With the switch off the same frames resolve the depth on the CPU: the
/// GL core waits for the depth pass, and the draw reads the R32F texels.
#[test]
fn with_depth_in_place_off_the_depth_is_resolved() {
    let _g = context();
    yakogl::mali::backend::DEPTH_IN_PLACE.store(false, Ordering::Relaxed);
    let (tex, fbo) = depth_target();
    let r = std::panic::catch_unwind(|| {
        let (waits, s) = frame(tex, fbo, 0.0, "resolved frame");
        assert!(waits >= 1, "the CPU resolve waits for the depth pass");
        assert_eq!(s.depth_id, 0, "the texels are the texture");
        let (base, _) = ZS_SEEN.lock().unwrap().expect("the depth pass's ZS target");
        assert!(TEX_SEEN.lock().unwrap().iter().all(|t| t.plane.pointer != base), "no texture reads the buffer");
    });
    yakogl::mali::backend::DEPTH_IN_PLACE.store(true, Ordering::Relaxed);
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}
