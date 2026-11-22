// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): an R or RG integer texture rendered into, then
//! sampled by a later draw, keeps its format's view (plan #383, issue #1261;
//! gap G2 of docs/notes/v3d-format-matrix.md).
//!
//! The twelve formats (R8I .. RG32UI) are held in the four-channel store of
//! the same channel type and width and drawn as that wide store
//! (`WIDE_INT_RT`, tests/es3_v3d_fbo_int_rg.rs). This file writes one at
//! attachment 0, at layer 1 of a 2D array and at level 1, then binds it as an
//! `isampler2D` / `usampler2D` / `isampler2DArray` in a draw into another
//! target and checks what the V3D back end derives from that binding
//! (`texture::tmu_format`, `tmu_swizzle`, `layout`, `state`, `image_offset`):
//!
//! - the binding is the wide store's texel format (RGBA8I .. RGBA32UI), the
//!   TMU type and the identity swizzle R G B A of Mesa's rows, the texel
//!   size of the store (4, 8 or 16 bytes), the 32-bit TMU output for the
//!   32-bit classes only, and nothing is refused;
//! - the record's size, depth (layers), levels, base and array stride, and
//!   for level 1 under a mipmap filter the level's offset in the V3D layout;
//! - the render's target is this texture's storage at the attached layer
//!   (one image after layer 0) or level (right after level 0's image): the
//!   image sampled is the image rendered;
//! - the absent channels: the swizzle is the identity, so an `isampler` reads
//!   what the store holds in G, B and A of an R format and B and A of an RG
//!   format. After a clear of 5, 6, 7, 8 the store holds 0, 0, 1 there (the
//!   clear narrowing, tests/es3_v3d_int_rg_clear.rs), which is what the
//!   sampler returns. A draw writes only the format's channels (per-RT
//!   colour mask), so the same holds after a draw.
//!
//! HOST STAND-IN ONLY: the back end here records each draw and stores a
//! clear in the target's memory; no draw is rasterized and no picture is
//! compared. Assertions are on the emitted state and words, built by the
//! real driver/src/v3d/v3d_tex.c (compiled by this test as
//! tests/es3_formats_v3d.rs does). Nothing here is verified on hardware.
//! The render image path (`rt_sample`, which reads the TLB's store without a
//! copy) is not reached; the copy path's record is the one checked.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_render_sample
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::ffi::*;
use yakogl::v3d::state;
use yakogl::v3d::texture;

/* ---- the real layout and record code, built from driver/src/v3d/v3d_tex.c -- */

extern "C" {
    fn dlopen(path: *const c_char, flags: core::ffi::c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own, reached through the handle only (RTLD_LOCAL),
/// as tests/es3_formats_v3d.rs builds it. Only the layout and the record's
/// layer stride and fields are reached from here.
fn lib() -> *mut c_void {
    static L: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *L.get_or_init(|| {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_v3d_int_rg_render_sample");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        let st = std::process::Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the layout code loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_tex.c");
    p
}

/// The library's `extern "C"` texture functions (src/v3d/ffi.rs), each handed
/// to the real C.
#[rustfmt::skip]
mod trampolines {
    use super::*;

    macro_rules! trampoline {
        ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
            #[no_mangle]
            unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
                // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
                let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                    unsafe { std::mem::transmute(sym(stringify!($name))) };
                // SAFETY: the library's call, passed on.
                unsafe { f($($arg),*) }
            }
        };
    }

    trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
    trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
}

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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

/// A texture binding as the draw carried it (its raw pointer field keeps it
/// from `Send`; the lock serialises every use).
#[derive(Clone)]
struct Bind(GlesTextureBind);
// SAFETY: plain data; the pointer field is never dereferenced here.
unsafe impl Send for Bind {}
impl core::fmt::Debug for Bind {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Bind(unit {}, format {})", self.0.unit, self.0.format)
    }
}

/// What the back end was handed by one call, and what the V3D made of it.
#[derive(Clone, Debug)]
#[allow(dead_code)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
    color_mask: u8,
    /// The job state's (colormask, rt_colormask_own, rt_colormask) of a
    /// draw the V3D takes.
    job: Option<(u32, bool, u32)>,
    /// The textures the draw binds (`GlesDraw::textures`).
    textures: Vec<Bind>,
    /// `state::refusal` / `state::unsupported`, or the job builder's reason.
    refused: Option<&'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// Records each draw; a clear of an integer target stores the low bits of
/// the four raw values in the target's layout, little endian, rows tightly
/// packed (the stand-in's own layout: the GL core reads back what it was told).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d)).or_else(|| {
        if d.index_count != 0 {
            state::job_state(d).err()
        } else {
            None
        }
    });
    let job = (d.index_count != 0)
        .then(|| state::job_state(d).ok())
        .flatten()
        .map(|st| (st.colormask, st.rt_colormask_own, st.rt_colormask));
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
        color_mask: d.color_mask,
        job,
        textures: d.textures[..d.num_textures as usize].iter().map(|b| Bind(*b)).collect(),
        refused,
    });
    let t = &d.targets[0];
    let Some(l) = backend::gles_surf_int_layout(t.format) else { return 0 };
    if !d.do_clear || t.gpu == 0 {
        return 0;
    }
    let bpp = backend::gles_surf_bytes_per_pixel(t.format) as usize;
    let v = d.clear_rgba.map(f32::to_bits);
    let step = l.bits as usize / 8;
    let mut px = vec![0u8; bpp];
    for ch in 0..l.channels as usize {
        px[ch * step..ch * step + step].copy_from_slice(&v[ch].to_le_bytes()[..step]);
    }
    let base = t.gpu as *mut u8;
    for y in 0..t.h as usize {
        for x in 0..t.w as usize {
            core::ptr::copy_nonoverlapping(px.as_ptr(), base.add((y * t.w as usize + x) * bpp), bpp);
        }
    }
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
    name: c"es3-v3d-int-rg-render-sample".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: true,
    native_layered_textures: true,
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
    samples_texel_copies: true,
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

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
}

/* ---- the twelve formats -------------------------------------------------- */

/// One of the twelve R and RG integer formats.
#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    ifmt: GLenum,
    channels: u8,
    signed: bool,
    /// The format's own surface (the renderbuffer's).
    surf: u8,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool, surf: u8) -> Row {
    Row { name, ifmt, channels, signed, surf }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true, GLES_SURF_R8I),
    row("R8UI", GL_R8UI, 1, false, GLES_SURF_R8UI),
    row("RG8I", GL_RG8I, 2, true, GLES_SURF_RG8I),
    row("RG8UI", GL_RG8UI, 2, false, GLES_SURF_RG8UI),
    row("R16I", GL_R16I, 1, true, GLES_SURF_R16I),
    row("R16UI", GL_R16UI, 1, false, GLES_SURF_R16UI),
    row("RG16I", GL_RG16I, 2, true, GLES_SURF_RG16I),
    row("RG16UI", GL_RG16UI, 2, false, GLES_SURF_RG16UI),
    row("R32I", GL_R32I, 1, true, GLES_SURF_R32I),
    row("R32UI", GL_R32UI, 1, false, GLES_SURF_R32UI),
    row("RG32I", GL_RG32I, 2, true, GLES_SURF_RG32I),
    row("RG32UI", GL_RG32UI, 2, false, GLES_SURF_RG32UI),
];

impl Row {
    fn ty(&self) -> &'static str {
        if self.signed {
            "ivec4"
        } else {
            "uvec4"
        }
    }
    /// The read-back of a clear of 5, 6, 7, 8: the format's channels, then
    /// 0, 0, 1 for the absent ones (ES 3.0 4.3.2).
    fn expected(&self) -> [u32; 4] {
        if self.channels == 1 {
            [5, 0, 0, 1]
        } else {
            [5, 6, 0, 1]
        }
    }
}

/* ---- attachments, a program, a draw, a clear, a read-back ----------------- */

/// What carries the format at attachment 0.
#[derive(Clone, Copy, Debug)]
#[allow(dead_code)]
enum Kind {
    Renderbuffer,
    Texture2d,
    /// Layer 1 of a 3-layer TEXTURE_2D_ARRAY.
    ArrayLayer,
    /// Level 1 of a 3-level, 8x8 TEXTURE_2D.
    Level1,
}

/// A bound framebuffer with the format at attachment 0, and whatever else
/// was attached; dropped, it unbinds and deletes them.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
    /// Width of the attached level (the read-back's size).
    w: GLsizei,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.tex.len() as GLsizei, self.tex.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

fn fbo(r: &Row, kind: Kind) -> Fbo {
    let (mut fb, mut name) = (0, 0);
    let mut f = Fbo { fb: 0, tex: vec![], rbs: vec![], w: 8 };
    // SAFETY: room for one name each; the layer and level named are there.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        f.fb = fb;
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, r.ifmt, 8, 8);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, name);
                f.rbs.push(name);
            }
            Kind::Texture2d | Kind::Level1 => {
                let (levels, level) = if matches!(kind, Kind::Level1) { (3, 1) } else { (1, 0) };
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, levels, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, level);
                f.tex.push(name);
                f.w = 8 >> level;
            }
            Kind::ArrayLayer => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D_ARRAY, name);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, r.ifmt, 8, 8, 3);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, name, 0, 1);
                f.tex.push(name);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: storage and attach", r.name);
    f
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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// Make `fs` (with the trivial vertex shader) current.
fn program(fs: &str) {
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Draw one triangle; what the back end was handed (None: no draw reached it)
/// and the GL error the call left.
fn draw() -> (Option<Seen>, GLenum) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (s.into_iter().rev().find(|s| s.draws), e)
}

/* ---- render into the format, then sample it ------------------------------ */

fn program_one(r: &Row) {
    let val = if r.signed { "ivec4(1, -2, 3, -4)" } else { "uvec4(1u, 2u, 3u, 4u)" };
    program(&format!(
        "#version 300 es\nprecision highp int;\nlayout(location = 0) out highp {} o0;\n\
         void main() {{ o0 = {val}; }}\n",
        r.ty()
    ));
}

/// What the render and the sampling draw handed the back end.
struct Pair {
    /// The render's RT 0 as the back end saw it: (gpu, w, h, format).
    rt: (u64, u32, u32, u8),
    /// The sampling draw.
    sample: Seen,
}

/// How the format was written before it is sampled.
#[derive(Clone, Copy, PartialEq, Debug)]
enum How {
    /// A draw of a known value (1, -2, 3, -4 or 1, 2, 3, 4).
    Draw,
    /// glClearBuffer{i,ui}v with 5, 6, 7, 8.
    Clear,
}

/// Write the format at attachment 0 as `kind` (`how`), then draw into an
/// RGBA32UI renderbuffer sampling the texture with `texelFetch`: level 0, or
/// level 1 under a mipmap filter (`mip`, [`Kind::Level1`] only); layer 1 of an
/// array.
fn render_then_sample(r: &Row, kind: Kind, how: How, mip: bool) -> Pair {
    let f = fbo(r, kind);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{} {kind:?}", r.name);
    let rt = match how {
        How::Draw => {
            program_one(r);
            let (seen, e) = draw();
            assert_eq!(e, GL_NO_ERROR, "{} {kind:?}: the render draw", r.name);
            seen.unwrap_or_else(|| panic!("{} {kind:?}: no render draw", r.name)).targets[0]
        }
        How::Clear => {
            SEEN.lock().unwrap().clear();
            let v: [GLint; 4] = [5, 6, 7, 8];
            // SAFETY: four values.
            unsafe {
                if r.signed {
                    glClearBufferiv(GL_COLOR, 0, v.as_ptr());
                } else {
                    glClearBufferuiv(GL_COLOR, 0, v.as_ptr() as *const GLuint);
                }
                glFinish();
            }
            assert_eq!(err(), GL_NO_ERROR, "{} {kind:?}: the clear", r.name);
            let seen = SEEN.lock().unwrap();
            seen.iter().rev().find(|s| !s.targets.is_empty() && s.targets[0].0 != 0).expect("the clear").targets[0]
        }
    };
    let arr = matches!(kind, Kind::ArrayLayer);
    let tgt = if arr { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_2D };
    let min = if mip { GL_NEAREST_MIPMAP_NEAREST } else { GL_NEAREST };
    let (mut fb2, mut rb) = (0, 0);
    // SAFETY: plain GL calls; room for one name each.
    unsafe {
        glBindTexture(tgt, f.tex[0]);
        glTexParameteri(tgt, GL_TEXTURE_MIN_FILTER, min as GLint);
        glTexParameteri(tgt, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glGenFramebuffers(1, &mut fb2);
        glBindFramebuffer(GL_FRAMEBUFFER, fb2);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA32UI, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
    }
    let p = if r.signed { "i" } else { "u" };
    let sam = if arr { "2DArray" } else { "2D" };
    let coord = if arr { "ivec3(0, 0, 1)" } else { "ivec2(0)" };
    let lod = if mip { 1 } else { 0 };
    let cast = if r.signed { ["uvec4(", ")"] } else { ["", ""] };
    program(&format!(
        "#version 300 es\nprecision highp int;\nprecision highp {p}sampler{sam};\nuniform highp {p}sampler{sam} s;\n\
         layout(location = 0) out highp uvec4 o0;\nvoid main() {{ o0 = {}texelFetch(s, {coord}, {lod}){}; }}\n",
        cast[0], cast[1]
    ));
    let (seen, err) = draw();
    // SAFETY: names made above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb2);
        glDeleteRenderbuffers(1, &rb);
    }
    assert_eq!(err, GL_NO_ERROR, "{} {kind:?} {how:?}: the sampling draw", r.name);
    let sample = seen.unwrap_or_else(|| panic!("{} {kind:?} {how:?}: no sampling draw", r.name));
    assert_eq!(sample.refused, None, "{} {kind:?} {how:?}: the sampling draw is taken", r.name);
    Pair { rt, sample }
}

/// The wide texel format, TMU data type and bytes per texel the V3D keeps a
/// format in: the four-channel store of the same channel type and width
/// (`GLES_TEXFMT_RGBA8I` .., Mesa's R8G8B8A8_SINT .. rows).
fn wide(r: &Row) -> (u8, u32, u32) {
    match (r.surf, backend::gles_surf_int_layout(r.surf).unwrap().bits, r.signed) {
        (_, 8, true) => (GLES_TEXFMT_RGBA8I, V3D_TEXTURE_DATA_FORMAT_RGBA8I, 4),
        (_, 8, false) => (GLES_TEXFMT_RGBA8UI, V3D_TEXTURE_DATA_FORMAT_RGBA8UI, 4),
        (_, 16, true) => (GLES_TEXFMT_RGBA16I, V3D_TEXTURE_DATA_FORMAT_RGBA16I, 8),
        (_, 16, false) => (GLES_TEXFMT_RGBA16UI, V3D_TEXTURE_DATA_FORMAT_RGBA16UI, 8),
        (_, _, true) => (GLES_TEXFMT_RGBA32I, V3D_TEXTURE_DATA_FORMAT_RGBA32I, 16),
        (_, _, false) => (GLES_TEXFMT_RGBA32UI, V3D_TEXTURE_DATA_FORMAT_RGBA32UI, 16),
    }
}

/// The bind's texture state as the back end builds it (`tex_state_for`:
/// `texture::layout`, `texture::state`), checked field by field.
fn check_state(r: &Row, kind: Kind, mip: bool, p: &Pair) {
    let what = format!("{} {kind:?}{}", r.name, if mip { " (mip)" } else { "" });
    let (fmt, type_, texel) = wide(r);
    assert_eq!(p.sample.textures.len(), 1, "{what}: one bound texture");
    let b = &p.sample.textures[0].0;
    assert_eq!((b.unit, b.incomplete), (0, 0), "{what}: complete, unit 0");
    assert_eq!(b.format, fmt, "{what}: the bind is the wide store's format");
    assert_eq!(b.swizzle, [0, 0, 0, 0], "{what}: the GL texture swizzle is the identity");
    assert_eq!((b.width, b.height), (8, 8), "{what}: one layer's size");
    let arr = matches!(kind, Kind::ArrayLayer);
    assert_eq!(texture::layers(b), if arr { 3 } else { 1 }, "{what}: layers");

    /* the format the TMU reads: the store's type, the identity swizzle R G B A:
     * the absent channels are what the store holds there (0, 0, 1, see
     * `the_absent_channels_of_the_store_hold_0_0_1`), not a swizzle constant */
    let xyzw = [V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA];
    assert_eq!(texture::tmu_format(b), (type_, xyzw), "{what}");
    assert_eq!(texture::tmu_swizzle(b), xyzw, "{what}");
    assert_eq!(texture::texel_bytes(b), texel, "{what}: texel bytes of the wide store");
    let ret32 = texel == 16;
    assert_eq!(texture::return_32(b), ret32, "{what}: TMU output width");
    assert_eq!(texture::refusal_for(b, ret32), None, "{what}: sampled at its own output width");
    assert!(texture::refusal_for(b, !ret32).is_some(), "{what}: the other output width is refused");

    /* layout and record */
    /* the levels the application wrote (draw.rs sampling_fields: level 0 and the
     * rendered level 1 of a 3-level storage), as far as a mipmap filter reads */
    let levels = if mip { 2 } else { 1 };
    assert_eq!(texture::levels_sampled(b), levels, "{what}: levels sampled");
    let l = texture::layout(b).unwrap_or_else(|| panic!("{what}: no layout"));
    assert_eq!((l.cpp, l.levels, l.width, l.height), (texel, levels, 8, 8), "{what}: layout");
    let gpu = 0x0010_0000u32;
    let st = texture::state(b, &l, gpu);
    assert_eq!((st.type_, st.swizzle), (type_, xyzw), "{what}: record type and swizzle");
    assert_eq!((st.width, st.height), (8, 8), "{what}: record size");
    assert_eq!(st.depth, if arr { 3 } else { 1 }, "{what}: record depth");
    assert_eq!((st.base_level, st.max_level), (0, levels - 1), "{what}: record levels");
    assert_eq!(st.base, gpu + l.slices[0].offset, "{what}: record base");
    assert!(!st.srgb, "{what}");
    let l0 = texture::image_offset(b, &l, 0, 0).unwrap();
    assert_eq!(l0, l.slices[0].offset, "{what}");
    if arr {
        let l1 = texture::image_offset(b, &l, 0, 1).unwrap();
        assert!(l1 > l0, "{what}: layer 1 follows layer 0");
        assert_eq!(st.array_stride_64 * 64, l1 - l0, "{what}: record's array stride is the layer stride");
        assert_eq!(texture::image_offset(b, &l, 0, 3), None, "{what}: three layers");
    }
    if mip {
        let m1 = texture::image_offset(b, &l, 1, 0).unwrap();
        assert_eq!(m1, l.slices[1].offset, "{what}: level 1 offset");
        assert_ne!(m1, l0, "{what}: level 1 is another image");
        assert_eq!(l.slices[1].offset > 0, true, "{what}");
    }

    /* the right image: the render's target is the bound storage, at the
     * attached layer or level (GL's storage: layers one image apart, level 1
     * right after level 0's image) */
    let img = u64::from(texel) * 64;
    let want = match kind {
        Kind::ArrayLayer => img,
        Kind::Level1 => img,
        _ => 0,
    };
    assert_eq!(p.rt.0 - b.base_gpu, want, "{what}: the render's target is this texture's storage at the attachment");
    assert_eq!((p.rt.1, p.rt.2), if matches!(kind, Kind::Level1) { (4, 4) } else { (8, 8) }, "{what}: target size");
    assert_eq!(b.stride_texels, 8, "{what}");
}

#[test]
fn a_rendered_r_rg_integer_texture_is_sampled_as_its_wide_store() {
    let _g = context();
    for r in ROWS {
        for kind in [Kind::Texture2d, Kind::ArrayLayer, Kind::Level1] {
            check_state(&r, kind, false, &render_then_sample(&r, kind, How::Draw, false));
        }
    }
}

/// Level 1 sampled under a mipmap filter: the record covers the whole chain.
#[test]
fn a_rendered_level_1_is_sampled_through_the_mip_chain() {
    let _g = context();
    for r in ROWS {
        check_state(&r, Kind::Level1, true, &render_then_sample(&r, Kind::Level1, How::Draw, true));
    }
}

/// The texel of the store the sampler reads where the render wrote: after a
/// clear of 5, 6, 7, 8 the absent channels hold 0, 0, 1 (the swizzle is the
/// identity, so these words are what an `isampler` / `usampler` returns).
#[test]
fn the_absent_channels_of_the_store_hold_0_0_1() {
    let _g = context();
    for r in ROWS {
        for kind in [Kind::Texture2d, Kind::ArrayLayer, Kind::Level1] {
            let p = render_then_sample(&r, kind, How::Clear, false);
            let l = backend::gles_surf_int_layout(p.rt.3).unwrap();
            assert_eq!(l.channels, 4, "{} {kind:?}: wide store", r.name);
            let step = l.bits as usize / 8;
            let mut got = [0u32; 4];
            for (ch, g) in got.iter_mut().enumerate() {
                let mut w = [0u8; 4];
                // SAFETY: the stand-in's clear wrote 4 channels at the target.
                unsafe {
                    core::ptr::copy_nonoverlapping((p.rt.0 as *const u8).add(ch * step), w.as_mut_ptr(), step);
                }
                *g = u32::from_le_bytes(w);
            }
            assert_eq!(got, r.expected(), "{} {kind:?}: the store's texel", r.name);
        }
    }
}
