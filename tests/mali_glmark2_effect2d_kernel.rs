// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 effect2d's convolution pattern on the Mali back end, through the
//! GL API on the host: an NPOT (13x7) RGBA texture with a known pattern,
//! GL_NEAREST and GL_CLAMP_TO_EDGE, is sampled at the centre texel and at the
//! neighbours one texel to each side (offset `1/width` or `1/height` from the
//! varying), the three samples are averaged, and a full-screen quad draws the
//! result into a 13x7 framebuffer texture, so each output pixel is one input
//! texel. Every output texel is checked, the edge texels (which test the
//! clamp) included, and so is the texture descriptor the Mali draw records
//! (size, wrap, filter, one level).
//!
//! The real Mali back end runs over a mock OSAL; no GPU runs. As in
//! `mali_glmark2_pingpong_blur`, the mock draw hook plays the GPU's part: it
//! reads the texels of the texture the draw was handed (at its base address
//! and row pitch), runs the kernel on the CPU and writes the result at the
//! target's address and pitch. The shader's arithmetic is not checked by the
//! mock; the shader must compile and link on the Mali online compiler and
//! the draw must be accepted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_glmark2_effect2d_kernel
//! cargo fmt --check
//! ```
//! (If capi/Cargo.toml defines `vivante-st` twice, delete the duplicate
//! locally to build; do not commit that.)

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
/// texels; (0, 0) plays nothing (no kernel).
static DIR: Mutex<(i32, i32)> = Mutex::new((0, 0));

const W: usize = 13;
const H: usize = 7;

/// The kernel on RGBA8 texels with GL_NEAREST and CLAMP_TO_EDGE: the mean of
/// the texel and its two neighbours at `(-dx, -dy)` and `(+dx, +dy)`, rounded
/// to 8 bits. An offset past the edge reads the edge texel.
fn kernel_cpu(src: &[[u8; 4]], dx: i32, dy: i32) -> Vec<[u8; 4]> {
    let mut out = vec![[0u8; 4]; W * H];
    for y in 0..H as i32 {
        for x in 0..W as i32 {
            let mut acc = [0f32; 4];
            for o in -1..=1 {
                let sx = (x + o * dx).clamp(0, W as i32 - 1) as usize;
                let sy = (y + o * dy).clamp(0, H as i32 - 1) as usize;
                for c in 0..4 {
                    acc[c] += f32::from(src[sy * W + sx][c]) / 3.0;
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
        /* the GPU's part: the kernel over the sampled texture */
        let sp = cpu_of(t.base_gpu, 4 * t.stride_texels as usize * H).expect("the source texture's buffer");
        let dp = cpu_of(dr.target_gpu, 4 * pitch as usize * H).expect("the target's buffer");
        let mut src = vec![[0u8; 4]; W * H];
        for y in 0..H {
            for x in 0..W {
                src[y * W + x] = *((sp + 4 * (y * t.stride_texels as usize + x)) as *const [u8; 4]);
            }
        }
        let out = kernel_cpu(&src, dx, dy);
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

/// A pattern that differs between every neighbour pair, so a wrong offset or
/// a wrong edge shows in every channel.
fn pattern() -> Vec<[u8; 4]> {
    (0..W * H)
        .map(|i| {
            let (x, y) = (i % W, i / W);
            [(x * 19 + 3) as u8, (y * 37 + 11) as u8, ((x * 7 + y * 53) % 251) as u8, 255]
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
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
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

const VS: &str = "attribute vec4 p;\nvarying vec2 uv;\nvoid main() { gl_Position = p; uv = p.xy * 0.5 + 0.5; }\n";
/// effect2d's shape: look-ups at the varying plus a constant offset of one
/// texel (`(-1/W, 0)`, none, `(+1/W, 0)`), the mean of three. The offset is
/// in the text, so one program per axis (a computed coordinate is TEX_MULTI).
fn fs_kernel(dx: i32, dy: i32) -> String {
    let (sx, sy) = (dx as f32 / W as f32, dy as f32 / H as f32);
    format!(
        "precision mediump float;\nuniform sampler2D tex;\nvarying vec2 uv;\nvoid main() {{\n  vec4 c = (texture2D(tex, uv + vec2({:.7}, {:.7})) + texture2D(tex, uv) + texture2D(tex, uv + vec2({:.7}, {:.7}))) * 0.3333333;\n  gl_FragColor = vec4(c.xyz, 1.0);\n}}\n",
        -sx, -sy, sx, sy
    )
}

/// One kernel pass: `src` sampled, `dst_fbo` drawn into.
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
    assert_eq!(d.tex_linear, 0, "{what}: GL_NEAREST");
    assert_eq!(d.tex_bgra, 0);
    assert_eq!(d.tex_wrap, (backend::GLES_WRAP_CLAMP_TO_EDGE, backend::GLES_WRAP_CLAMP_TO_EDGE), "{what}: wrap");
    assert_ne!(d.tex_base_gpu, d.target_gpu, "{what}: the source and the target are different storage");
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

/// The horizontal kernel over every texel, the left and right columns
/// (clamped taps) included.
#[test]
fn kernel_horizontal_every_texel() {
    let _g = context();
    let input = pattern();
    let (ts, _fs) = target(Some(&input));
    let (_td, fd) = target(None);
    let p = program(VS, &fs_kernel(1, 0));
    pass(p, ts, fd, 1, 0, "horizontal");
    let got = read_back(fd);
    check_equal(&got, &kernel_cpu(&input, 1, 0), "horizontal");
    /* column 0's left tap clamps to itself; the last column's right tap too */
    for y in 0..H {
        for c in 0..3 {
            let l = (2.0 * f32::from(input[y * W][c]) + f32::from(input[y * W + 1][c])) / 3.0;
            assert!((f32::from(got[y * W][c]) - l).abs() <= 1.0, "left edge row {y} channel {c}");
            let r = (2.0 * f32::from(input[y * W + W - 1][c]) + f32::from(input[y * W + W - 2][c])) / 3.0;
            assert!((f32::from(got[y * W + W - 1][c]) - r).abs() <= 1.0, "right edge row {y} channel {c}");
        }
    }
}

/// The vertical kernel: the top and bottom rows clamp.
#[test]
fn kernel_vertical_every_texel() {
    let _g = context();
    let input = pattern();
    let (ts, _fs) = target(Some(&input));
    let (_td, fd) = target(None);
    let p = program(VS, &fs_kernel(0, 1));
    pass(p, ts, fd, 0, 1, "vertical");
    let got = read_back(fd);
    check_equal(&got, &kernel_cpu(&input, 0, 1), "vertical");
    for x in 0..W {
        for c in 0..3 {
            let t = (2.0 * f32::from(input[x][c]) + f32::from(input[W + x][c])) / 3.0;
            assert!((f32::from(got[x][c]) - t).abs() <= 1.0, "top edge column {x} channel {c}");
            let b = (2.0 * f32::from(input[(H - 1) * W + x][c]) + f32::from(input[(H - 2) * W + x][c])) / 3.0;
            assert!((f32::from(got[(H - 1) * W + x][c]) - b).abs() <= 1.0, "bottom edge column {x} channel {c}");
        }
    }
}

/// Two passes in a row (horizontal then vertical, a separable 3x3): the
/// second samples what the first drew, and the source is left unchanged.
#[test]
fn kernel_two_passes_separable() {
    let _g = context();
    let input = pattern();
    let (ta, fa) = target(Some(&input));
    let (tb, fb) = target(None);
    let (ph, pv) = (program(VS, &fs_kernel(1, 0)), program(VS, &fs_kernel(0, 1)));
    let d0 = pass(ph, ta, fb, 1, 0, "pass 0");
    let mid = read_back(fb);
    check_equal(&mid, &kernel_cpu(&input, 1, 0), "after pass 0");
    let d1 = pass(pv, tb, fa, 0, 1, "pass 1");
    assert_eq!(d1.tex_base_gpu, d0.target_gpu, "pass 1 samples pass 0's output");
    let want = kernel_cpu(&mid, 0, 1);
    check_equal(&read_back(fa), &want, "after pass 1");
}
