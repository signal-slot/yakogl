// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2's desktop scene (blur effect) on the Mali back end, through the
//! GL API on the host: a window is rendered into an NPOT texture, blurred in
//! horizontal and vertical passes that alternate between two framebuffers
//! with a colour texture only (200x150 RGBA8, GL_LINEAR, CLAMP_TO_EDGE, no
//! mips), and the result is composited to the default framebuffer.
//!
//! The real Mali back end runs over a mock OSAL; no GPU runs. The mock draw
//! hook plays the GPU's part for each blur pass: it reads the texels of the
//! texture the draw was handed (at its base address and row pitch), runs the
//! pass's three-tap kernel on the CPU, and writes the result at the draw's
//! target address and pitch. So the test checks what the back end controls:
//! that each pass samples the *other* target's storage, draws into its own,
//! that the descriptors (size, pitch, wrap, filter, no mips) are right and
//! that every pass is accepted with GL_NO_ERROR. The shader's arithmetic is
//! not checked by the mock; the read-back after four passes is compared with
//! an independent CPU reference of the same chain.
//!
//! The blur shader is a fixed three-tap one (weights 1/4, 1/2, 1/4, offset
//! one texel along `dir`), without a loop. The kernel offsets are whole
//! texels, so GL_LINEAR reads exact texels and the reference is exact.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_pingpong_blur
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

unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    if len == core::mem::size_of::<OsalMaliKick>() {
        let _ = *kick.cast::<OsalMaliKick>();
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
    target_w: u32,
    target_h: u32,
    target_pitch: u32,
    num_textures: u8,
    tex_incomplete: u8,
    tex_base_gpu: u64,
    tex_w: u32,
    tex_h: u32,
    tex_stride: u32,
    tex_mips: u8,
    tex_linear: u8,
    tex_wrap: (u8, u8),
    tex_bgra: u8,
    mali: i32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The blur direction of the pass the mock GPU plays next: (dx, dy) in
/// texels; (0, 0) plays nothing (the composite draw).
static DIR: Mutex<(i32, i32)> = Mutex::new((0, 0));

const W: usize = 200;
const H: usize = 150;
const KERNEL: [f32; 3] = [0.25, 0.5, 0.25];

/// One blur pass on RGBA8 texels: `dst = sum k[i] * src(x + (i-1)*dx, ...)`,
/// clamped to the edge, rounded to 8 bits.
fn blur_cpu(src: &[[u8; 4]], dx: i32, dy: i32) -> Vec<[u8; 4]> {
    let mut out = vec![[0u8; 4]; W * H];
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let mut acc = [0f32; 4];
            for (i, k) in KERNEL.iter().enumerate() {
                let o = i as i32 - 1;
                let sx = (x + o * dx).clamp(0, W as i32 - 1) as usize;
                let sy = (y + o * dy).clamp(0, H as i32 - 1) as usize;
                for c in 0..4 {
                    acc[c] += k * f32::from(src[sy * W + sx][c]);
                }
            }
            for c in 0..4 {
                out[y as usize * W + x as usize][c] = acc[c].round() as u8;
            }
        }
    }
    out
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let dr = &*d;
    let mali = yakogl::mali::backend::mali_draw(d);
    let t = &dr.textures[0];
    let pitch = if dr.target_pitch != 0 { dr.target_pitch } else { dr.target_w };
    let seen = Seen {
        index_count: dr.index_count,
        target_gpu: dr.target_gpu,
        target_w: dr.target_w,
        target_h: dr.target_h,
        target_pitch: pitch,
        num_textures: dr.num_textures,
        tex_incomplete: t.incomplete,
        tex_base_gpu: t.base_gpu,
        tex_w: t.width,
        tex_h: t.height,
        tex_stride: t.stride_texels,
        tex_mips: t.mip_levels,
        tex_linear: t.linear_filter,
        tex_wrap: (t.wrap_s, t.wrap_t),
        tex_bgra: t.bgra,
        mali,
    };
    SEEN.lock().unwrap().push(seen);
    let (dx, dy) = *DIR.lock().unwrap();
    if mali == 0 && dr.index_count == 6 && (dx, dy) != (0, 0) && dr.target_gpu != 0 {
        /* the GPU's part: the pass's kernel over the sampled texture */
        let sp = cpu_of(t.base_gpu, 4 * t.stride_texels as usize * H).expect("the source texture's buffer");
        let dp = cpu_of(dr.target_gpu, 4 * pitch as usize * H).expect("the target's buffer");
        let mut src = vec![[0u8; 4]; W * H];
        for y in 0..H {
            for x in 0..W {
                src[y * W + x] = *((sp + 4 * (y * t.stride_texels as usize + x)) as *const [u8; 4]);
            }
        }
        let out = blur_cpu(&src, dx, dy);
        for y in 0..H {
            for x in 0..W {
                *((dp + 4 * (y * pitch as usize + x)) as *mut [u8; 4]) = out[y * W + x];
            }
        }
    }
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
    *DIR.lock().unwrap() = (0, 0);
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
/// Three taps along one axis, weights 1/4, 1/2, 1/4, at constant offsets of
/// one texel from the varying (glmark2's blur has the same shape: look-ups
/// at `v_uv + vec2(k * TextureStepX, 0.0)`; the centre tap is the varying
/// itself). A coordinate that is not the varying plus constants, or two
/// varyings, is refused at the tip (TEX_MULTI), so the direction is in the
/// text: one program per axis, and `dir` is `(1, 0)` or `(0, 1)`.
fn fs_blur(dx: i32, dy: i32) -> String {
    let (sx, sy) = (dx as f32 / W as f32, dy as f32 / H as f32);
    format!(
        "precision mediump float;\nuniform sampler2D tex;\nvarying vec2 uv;\nvoid main() {{\n  vec4 c = texture2D(tex, uv + vec2({:.7}, {:.7})) * 0.25 + texture2D(tex, uv) * 0.5 + texture2D(tex, uv + vec2({:.7}, {:.7})) * 0.25;\n  gl_FragColor = vec4(c.xyz, 1.0);\n}}\n",
        -sx, -sy, sx, sy
    )
}
/// The composite: the blurred texture to the default framebuffer.
const FS_COMP: &str = "precision mediump float;\nuniform sampler2D tex;\nvarying vec2 uv;\nvoid main() { gl_FragColor = texture2D(tex, uv); }\n";

/// A full-viewport quad as two triangles.
fn quad(what: &str) -> Seen {
    let p: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
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

/// The input window: a pattern with edges and a gradient, so a blur in
/// either direction and the edge clamp show.
fn pattern() -> Vec<[u8; 4]> {
    (0..W * H)
        .map(|i| {
            let (x, y) = (i % W, i / W);
            let box_ = (60..110).contains(&x) && (40..90).contains(&y);
            [
                if box_ { 255 } else { (x * 255 / (W - 1)) as u8 },
                (y * 255 / (H - 1)) as u8,
                ((x * 7 + y * 3) % 256) as u8,
                255,
            ]
        })
        .collect()
}

/// An NPOT RGBA texture made `data` (or empty) and an FBO with it as the
/// only attachment.
fn target(data: Option<&[[u8; 4]]>) -> (GLuint, GLuint) {
    let (mut tex, mut fbo) = (0, 0);
    // SAFETY: room for one name each; `data` holds W*H texels or is null.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        let px = data.map_or(core::ptr::null(), |d| d.as_ptr().cast::<c_void>());
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, W as GLsizei, H as GLsizei, 0, GL_RGBA, GL_UNSIGNED_BYTE, px);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "colour-only FBO: status 0x{st:x}");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    ok("target");
    (tex, fbo)
}

fn read_back(fbo: GLuint) -> Vec<[u8; 4]> {
    let mut px = vec![[0u8; 4]; W * H];
    // SAFETY: room for W*H RGBA texels.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glReadPixels(0, 0, W as GLsizei, H as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    ok("read back");
    px
}

/// One blur pass: `src` sampled, `dst_fbo` drawn into.
fn pass(p: GLuint, src: GLuint, dst_fbo: GLuint, dx: i32, dy: i32, what: &str) -> Seen {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, dst_fbo);
        glViewport(0, 0, W as GLsizei, H as GLsizei);
        glUseProgram(p);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, src);
        glUniform1i(glGetUniformLocation(p, c"tex".as_ptr()), 0);
    }
    ok(&format!("{what}: setup"));
    *DIR.lock().unwrap() = (dx, dy);
    let d = quad(what);
    *DIR.lock().unwrap() = (0, 0);
    assert_ne!(d.target_gpu, 0, "{what}: draws to the FBO's texture");
    assert_eq!((d.target_w, d.target_h), (W as u32, H as u32), "{what}: target size");
    assert_eq!(d.num_textures, 1);
    assert_eq!(d.tex_incomplete, 0, "{what}: the sampled texture is complete");
    assert_eq!((d.tex_w, d.tex_h), (W as u32, H as u32), "{what}: texture size");
    assert!(d.tex_stride >= W as u32 && d.target_pitch >= W as u32);
    assert_eq!(d.tex_mips, 1, "{what}: one level, no mips");
    assert_eq!(d.tex_linear, 1, "{what}: GL_LINEAR");
    assert_eq!(d.tex_bgra, 0);
    assert_eq!(d.tex_wrap, (backend::GLES_WRAP_CLAMP_TO_EDGE, backend::GLES_WRAP_CLAMP_TO_EDGE), "{what}: wrap");
    d
}

fn check_equal(got: &[[u8; 4]], want: &[[u8; 4]], what: &str) {
    for i in 0..W * H {
        for c in 0..4 {
            let (g, w) = (i32::from(got[i][c]), i32::from(want[i][c]));
            assert!((g - w).abs() <= 1, "{what}: ({}, {}) channel {c} is {g}, expected {w}", i % W, i / W);
        }
    }
}

/// glmark2 desktop blur: four passes (H, V, H, V) between two NPOT FBO
/// textures, then the composite to the default framebuffer.
#[test]
fn pingpong_blur_four_passes_then_composite() {
    let _g = context();
    let input = pattern();
    let (ta, fa) = target(Some(&input));
    let (tb, fb) = target(None);
    let (bh, bv) = (program(VS, &fs_blur(1, 0)), program(VS, &fs_blur(0, 1)));
    let seq = [(bh, ta, fb, 1, 0), (bv, tb, fa, 0, 1), (bh, ta, fb, 1, 0), (bv, tb, fa, 0, 1)];
    let mut want = input.clone();
    let mut last = None;
    let mut seen = Vec::new();
    for (i, &(blur, src, dst, dx, dy)) in seq.iter().enumerate() {
        let d = pass(blur, src, dst, dx, dy, &format!("pass {i}"));
        want = blur_cpu(&want, dx, dy);
        seen.push(d);
        /* each pass's output is the next pass's input: read it back */
        check_equal(&read_back(dst), &want, &format!("after pass {i}"));
        last = Some(dst);
    }
    /* the two targets alternate: pass k samples what pass k-1 drew into */
    for k in 1..seen.len() {
        assert_eq!(seen[k].tex_base_gpu, seen[k - 1].target_gpu, "pass {k} samples pass {} output", k - 1);
        assert_ne!(seen[k].target_gpu, seen[k - 1].target_gpu, "pass {k} draws into the other target");
    }
    assert_eq!(seen[0].target_gpu, seen[2].target_gpu);
    assert_eq!(seen[1].target_gpu, seen[3].target_gpu);
    assert_eq!(last, Some(fa));

    /* the composite: the last output on the default framebuffer */
    let comp = program(VS, FS_COMP);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, N, N);
        glUseProgram(comp);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, ta);
        glUniform1i(glGetUniformLocation(comp, c"tex".as_ptr()), 0);
    }
    ok("composite setup");
    let d = quad("composite");
    assert_eq!((d.target_w, d.target_h), (N as u32, N as u32));
    assert_eq!(d.tex_base_gpu, seen[3].target_gpu, "the composite samples the last pass's target");
    assert_eq!(d.tex_incomplete, 0);
    assert_eq!(backend::draws_refused(), 0);
}

/// Column 0 clamps to the edge: its left tap is itself.
#[test]
fn pingpong_blur_edges_clamp() {
    let _g = context();
    let input = pattern();
    let (ta, _fa) = target(Some(&input));
    let (_tb, fb) = target(None);
    let blur = program(VS, &fs_blur(1, 0));
    pass(blur, ta, fb, 1, 0, "horizontal");
    let got = read_back(fb);
    let want = blur_cpu(&input, 1, 0);
    check_equal(&got, &want, "horizontal pass");
    /* column 0 clamps: its left tap is itself */
    let e = input[0][0] as f32 * 0.75 + input[1][0] as f32 * 0.25;
    assert!((f32::from(got[0][0]) - e).abs() <= 1.0);
}
