// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth from a cube face or an array layer of a depth
//! texture, depth into one, and stencil to and from a DEPTH24_STENCIL8
//! texture, each against a renderbuffer, through the GL API over the GC7000UL
//! (Vivante) host back end (plan item layers-test-gc; ES 3.0.6 4.3.3). The
//! GL core's half on a stand-in back end is tests/es3_blit_ds_layers.rs and
//! its design docs/notes/blit-depth-layers.md; the rig here is the rig of
//! tests/es3_blit_ds_window_gc.rs and tests/es3_blit_ds_gc.rs.
//!
//! The Vivante table has `depth_resolve` (`gc_depth_resolve`) but
//! `ds_write: None` and `stencil_resolve: None` (src/vivante/backend.rs).
//! Every shape here is exactly one GL_INVALID_OPERATION today; the refusal
//! of src/gl/blit.rs that answers it:
//!
//! - depth from a cube face or an array layer into a renderbuffer: the source
//!   is REFUSED_DEPTH_SRC (`depth_texels` and `texture::resolve_depth` take
//!   GL_TEXTURE_2D only, `GlesDraw::depth_id` names a texture, not a face or
//!   layer), and the destination is REFUSED_DEPTH_DST as well (no `ds_write`
//!   for a renderbuffer). Both causes answer; the call is refused whole.
//! - depth from a renderbuffer into a cube face or an array layer: the
//!   source could be read (`depth_resolve`), the destination is
//!   REFUSED_DEPTH_DST (`depth_texels` takes GL_TEXTURE_2D only). The lift is
//!   the GL core's (gl-ds-layers); no table entry is missing for it.
//! - stencil into a D24S8 texture from a renderbuffer, and out of a D24S8
//!   texture into a renderbuffer: REFUSED_STENCIL (a stencil copy needs
//!   `stencil_resolve` and `ds_write`, which the table lacks, and a
//!   renderbuffer on both sides).
//!
//! The API entries these tests need are not refused before blit.rs: cube and
//! array depth storage, glFramebufferTexture2D with a face target and
//! glFramebufferTextureLayer with a depth attachment give complete
//! framebuffers.
//!
//! Each shape is a pair. `..._is_refused_today` pins today's answer: exactly
//! one GL_INVALID_OPERATION, the destination unchanged (a renderbuffer's
//! depth through `depth_resolve`, a texture's texels as the GL core holds
//! them), every other layer and face unchanged, and the context usable
//! afterwards. The stencil plane cannot be read on this back end, so the
//! stencil refusals do not check it. The `#[ignore]`d twin holds what a
//! working blit leaves, from the nearest-sample model of
//! tests/es3_blit_ds_layers.rs, written apart from the code.
//!
//! What un-ignores the twins: the GL core's gl-ds-layers (the face/layer
//! depth source and destination, and stencil to and from a texture in
//! src/gl/blit.rs) and, for the shapes that read or write a renderbuffer's
//! depth or stencil through the table, the GC7000UL `ds_write` and
//! `stencil_resolve` entries (plans #335/#355). The twin of depth from a
//! renderbuffer into a face or layer needs gl-ds-layers only; the others need
//! both. Removing the `#[ignore]` is all it takes then.
//!
//! HOST-ONLY: API verdicts on the host back end, never pictures and not a
//! board result (tools/cts/README.md 2b).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_blit_ds_layers_gc
//! ```
//! (add `-- --ignored` for the twins, which fail until the lift)

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_depth_resolve, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/// The online compiler's store is one static for the process: every test
/// takes this lock.
static STORE: Mutex<()> = Mutex::new(());

/* ---- the stand-in OSAL: host memory, every kick accepted ------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor; it is cleared (the OSAL contract).
    unsafe {
        let m = *mem;
        with_platform(|p| p.allocs.remove(&m.gpu));
        *mem = OsalGpuMem::ZERO;
    }
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC || !p.allocs.contains_key(&k.stream_gpu) {
            return -22;
        }
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 context; the lock serialises the tests.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    with_platform(|p| {
        /* leaked, not freed, as tests/es3_mrt4_gc.rs */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- the sides ------------------------------------------------------------- */

const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;
/// Clears: (depth, stencil) of a blit source renderbuffer and of a
/// destination renderbuffer. Constants (exact in 24 bits), so a blit shows
/// only as which pixels took the source's value (`model` below).
const SRC_CLEAR: (f32, i32) = (0.25, 0x5a);
const DST_CLEAR: (f32, i32) = (0.75, 0x11);

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn drain() {
    while err() != GL_NO_ERROR {}
}

/// A `w` x `h` D24S8 renderbuffer at GL_DEPTH_STENCIL_ATTACHMENT of a new
/// framebuffer, cleared to `clear`: (framebuffer, renderbuffer).
fn ds_fbo(w: i32, h: i32, clear: (f32, i32)) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearDepthf(clear.0);
        glClearStencil(clear.1);
        glClear(DEPTH | STENCIL);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing the renderbuffer");
    (fb, rb)
}

/// A `w` x `h` D24S8 *texture* at GL_DEPTH_STENCIL_ATTACHMENT of a new
/// framebuffer, depth cleared to `depth`: (framebuffer, texture).
fn tex_fbo(w: i32, h: i32, depth: f32) -> (GLuint, GLuint) {
    let (mut fb, mut tex) = (0, 0);
    // SAFETY: room for one name each; no pixels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_DEPTH24_STENCIL8 as GLint,
            w,
            h,
            0,
            GL_DEPTH_STENCIL,
            GL_UNSIGNED_INT_24_8,
            core::ptr::null(),
        );
        assert_eq!(glGetError(), GL_NO_ERROR, "texture storage");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearDepthf(depth);
        glClear(DEPTH);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing the texture");
    (fb, tex)
}

/// A `w` x `h` RGBA8 renderbuffer in a new framebuffer, cleared to red.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

fn blit(s: [i32; 4], d: [i32; 4], mask: GLbitfield) {
    drain();
    // SAFETY: a plain GL call.
    unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, GL_NEAREST) };
}

fn release(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/// Depth plane `id` (`w` x `h`, memory order) through the table's
/// `depth_resolve`; None when the back end holds nothing under that id.
fn try_depth(id: u32, w: i32, h: i32) -> Option<Vec<f32>> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { gc_depth_resolve(id, v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    (r == 0).then_some(v)
}

fn read_depth(id: u32, w: i32, h: i32) -> Vec<f32> {
    try_depth(id, w, h).expect("the back end holds the depth plane")
}

/// Stencil plane `id` through the table's `stencil_resolve` (the entry the
/// lift adds; `None` today).
fn read_stencil(id: u32, w: i32, h: i32) -> Vec<u8> {
    let f = yakogl_backend_vivante.stencil_resolve.expect("the GC table has a stencil_resolve entry");
    let mut v = vec![0xffu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { f(id, v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the stencil plane");
    v
}

/* ---- objects ----------------------------------------------------------- */

/// How a depth texture is specified: (internal format, format, type).
type Fmt = (GLenum, GLenum, GLenum);
const D32F: Fmt = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);
const D24: Fmt = (GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT);

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];
const FACE_PX: usize = 0;
const FACE_NZ: usize = 5;

/// What kind of layered depth texture a test makes.
#[derive(Clone, Copy, Debug)]
enum Layered {
    Cube,
    Array,
}

fn texel_ptr(tex: GLuint, layer: u32, x: u32, y: u32) -> *mut f32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert!(x < t.width && y < t.height && layer < t.layers.max(1) as u32);
    let row = layer as usize * t.height as usize + y as usize;
    let off = t.level_off[0] as usize + (row * t.level_pitch_at(0) as usize + x as usize) * 4;
    assert!(off + 4 <= t.mem.size);
    (t.mem.cpu as *mut u8).wrapping_add(off) as *mut f32
}

/// The R32F texel (x, y) of layer/face `layer` of level 0, as stored.
fn texel(tex: GLuint, layer: u32, x: u32, y: u32) -> f32 {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    unsafe { texel_ptr(tex, layer, x, y).read() }
}

fn set_texel(tex: GLuint, layer: u32, x: u32, y: u32, v: f32) {
    // SAFETY: as in `texel`.
    unsafe { texel_ptr(tex, layer, x, y).write(v) }
}

/// A `w` x `h` 2D depth texture, depth (x, y) = `f(x, y)`.
fn depth_texture(fmt: Fmt, w: u32, h: u32, f: impl Fn(u32, u32) -> f32) -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt.0 as GLint, w as i32, h as i32, 0, fmt.1, fmt.2, core::ptr::null());
    }
    assert_eq!(err(), GL_NO_ERROR, "a {:#x} depth texture", fmt.0);
    for y in 0..h {
        for x in 0..w {
            set_texel(tex, 0, x, y, f(x, y));
        }
    }
    tex
}

/// A `side` x `side` cube map (six faces) or 2D array (`n` layers; a cube
/// has six) of depth, depth (x, y) of layer k = `f(k, x, y)`.
fn layered_texture(kind: Layered, fmt: Fmt, side: u32, n: u32, f: impl Fn(u32, u32, u32) -> f32) -> GLuint {
    let mut tex = 0;
    let s = side as i32;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                for face in FACES {
                    glTexImage2D(face, 0, fmt.0 as GLint, s, s, 0, fmt.1, fmt.2, core::ptr::null());
                }
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                glTexImage3D(
                    GL_TEXTURE_2D_ARRAY,
                    0,
                    fmt.0 as GLint,
                    s,
                    s,
                    n as i32,
                    0,
                    fmt.1,
                    fmt.2,
                    core::ptr::null(),
                );
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "a {kind:?} depth texture of {:#x}", fmt.0);
    let layers = if matches!(kind, Layered::Cube) { 6 } else { n };
    for k in 0..layers {
        for y in 0..side {
            for x in 0..side {
                set_texel(tex, k, x, y, f(k, x, y));
            }
        }
    }
    tex
}

fn layer_picture(tex: GLuint, layer: u32, side: u32) -> Vec<f32> {
    (0..side).flat_map(|y| (0..side).map(move |x| (x, y))).map(|(x, y)| texel(tex, layer, x, y)).collect()
}

fn picture(tex: GLuint, w: u32, h: u32) -> Vec<f32> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| texel(tex, 0, x, y)).collect()
}

/// A framebuffer with the 2D depth texture `tex` at GL_DEPTH_ATTACHMENT (or
/// at GL_DEPTH_STENCIL_ATTACHMENT with `stencil_too`), checked complete.
fn texture_fbo(tex: GLuint, stencil_too: bool) -> GLuint {
    let mut fb = 0;
    let point = if stencil_too { GL_DEPTH_STENCIL_ATTACHMENT } else { GL_DEPTH_ATTACHMENT };
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, point, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

/// A framebuffer with face `face` (index into `FACES`) of cube map `tex` at
/// the depth attachment.
fn face_fbo(tex: GLuint, face: usize) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, FACES[face], tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "a cube face at the depth attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

/// A framebuffer with layer `layer` of array `tex` at the depth attachment.
fn layer_fbo(tex: GLuint, layer: i32) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, tex, 0, layer);
        assert_eq!(err(), GL_NO_ERROR, "an array layer at the depth attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}
/* ---- the model ----------------------------------------------------------- */

/// The source texel destination pixel `o` takes on one axis, or None when
/// its sample point is outside the source cut to `[0, n)`.
fn model_axis(o: i64, s0: i64, s1: i64, d0: i64, d1: i64, n: i64) -> Option<i64> {
    let p = s0 as f64 + (o as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64;
    let (lo, hi) = (s0.min(s1).max(0), s0.max(s1).min(n));
    let t = p.floor() as i64;
    (p >= lo as f64 && p < hi as f64).then_some(t)
}

/// The destination after the blit `s` -> `d` of a `sw` x `sh` source whose
/// depth is `src` into a `dw` x `dh` destination that held `keep`.
fn model(
    src: &dyn Fn(u32, u32) -> f32,
    (sw, sh): (u32, u32),
    keep: &dyn Fn(u32, u32) -> f32,
    (dw, dh): (u32, u32),
    s: [i32; 4],
    d: [i32; 4],
) -> Vec<f32> {
    let mut out = Vec::new();
    for y in 0..dh {
        for x in 0..dw {
            let (xi, yi) = (x as i64, y as i64);
            let in_dst = xi >= d[0].min(d[2]) as i64
                && xi < d[0].max(d[2]) as i64
                && yi >= d[1].min(d[3]) as i64
                && yi < d[1].max(d[3]) as i64;
            let from = in_dst
                .then(|| {
                    let tx = model_axis(xi, s[0] as i64, s[2] as i64, d[0] as i64, d[2] as i64, sw as i64)?;
                    let ty = model_axis(yi, s[1] as i64, s[3] as i64, d[1] as i64, d[3] as i64, sh as i64)?;
                    Some((tx as u32, ty as u32))
                })
                .flatten();
            out.push(match from {
                Some((tx, ty)) => src(tx, ty),
                None => keep(x, y),
            });
        }
    }
    out
}

/// Depth of layer/face `k` at (x, y): a different ramp per layer, distinct
/// everywhere, exact in f32 (and in the 24 bits of a D24).
fn layer_depth(k: u32, x: u32, y: u32) -> f32 {
    (k * 256 + y * 16 + x) as f32 / 4096.0
}

/* ---- helpers of this file --------------------------------------------------- */

fn assert_depth_eq(what: &str, got: &[f32], want: &[f32]) {
    assert_eq!(got.len(), want.len(), "{what}: size");
    for (i, (g, e)) in got.iter().zip(want).enumerate() {
        assert!((g - e).abs() <= 0.5 / 16_777_215.0, "{what}: depth pixel {i}: {g} want {e}");
    }
}

/// A `w` x `h` depth-only renderbuffer matching `fmt` (DEPTH_COMPONENT24 or
/// 32F, so the formats of the two sides agree and the blit is refused only
/// for the causes in the header) at GL_DEPTH_ATTACHMENT of a new
/// framebuffer, depth cleared to `depth`: (framebuffer, renderbuffer).
fn depth_rb(fmt: Fmt, w: i32, h: i32, depth: f32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, fmt.0, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glClearDepthf(depth);
        glClear(DEPTH);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing the renderbuffer");
    (fb, rb)
}

const SIDE: u32 = 8;
const WHOLE: [i32; 4] = [0, 0, SIDE as i32, SIDE as i32];

/// Blit once and require today's answer: exactly one GL_INVALID_OPERATION.
fn refused(s: [i32; 4], d: [i32; 4], mask: GLbitfield, what: &str) {
    blit(s, d, mask);
    assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
    assert_eq!(err(), GL_NO_ERROR, "{what}: exactly one error");
}

/// The context still blits after a refusal: a depth renderbuffer copies
/// into a 2D depth texture (the GL core holds its texels).
fn still_usable() {
    let tex = depth_texture(D24, 4, 4, |_, _| 0.5);
    let tfb = texture_fbo(tex, false);
    let (rfb, rb) = depth_rb(D24, 4, 4, SRC_CLEAR.0);
    bind(rfb, tfb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], DEPTH);
    assert_eq!(err(), GL_NO_ERROR, "the context after a refusal");
    assert_depth_eq("a later blit", &picture(tex, 4, 4), &[SRC_CLEAR.0; 16]);
    release(&[rfb, tfb], &[tex], &[rb]);
}

fn untouched_layers(tex: GLuint, n: u32, except: Option<u32>, what: &str) {
    for l in (0..n).filter(|&l| Some(l) != except) {
        let want: Vec<f32> = (0..SIDE * SIDE).map(|i| layer_depth(l, i % SIDE, i / SIDE)).collect();
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} must not change");
    }
}

fn layer_count(kind: Layered) -> u32 {
    if matches!(kind, Layered::Cube) {
        6
    } else {
        3
    }
}

fn layer_source(kind: Layered, tex: GLuint, k: u32) -> GLuint {
    match kind {
        Layered::Cube => face_fbo(tex, k as usize),
        Layered::Array => layer_fbo(tex, k as i32),
    }
}

/* ---- (1)/(2) depth from a cube face or an array layer into a renderbuffer --- */

/// Face/layer `k` of a layered texture of `fmt` blitted `s` -> `d` into a
/// `SIDE` x `SIDE` depth renderbuffer cleared to `DST_CLEAR`. Refused today
/// (REFUSED_DEPTH_SRC, and REFUSED_DEPTH_DST for the missing `ds_write`):
/// the renderbuffer's depth and every layer stay. Otherwise the
/// renderbuffer holds the model's picture.
fn from_layer(kind: Layered, fmt: Fmt, k: u32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    on_gc7000(|| {
        let n = layer_count(kind);
        let tex = layered_texture(kind, fmt, SIDE, n, layer_depth);
        let sfb = layer_source(kind, tex, k);
        let (dfb, rb) = depth_rb(fmt, SIDE as i32, SIDE as i32, DST_CLEAR.0);
        let id = ds_id(rb);
        let what = format!("{kind:?} {fmt:?} layer {k} into a renderbuffer: {s:?} -> {d:?}");
        let keep = |_, _| DST_CLEAR.0;
        let before = read_depth(id, SIDE as i32, SIDE as i32);
        bind(sfb, dfb);
        let want = if refused_today {
            refused(s, d, DEPTH, &what);
            before.clone()
        } else {
            blit(s, d, DEPTH);
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            let src = |x, y| layer_depth(k, x, y);
            model(&src, (SIDE, SIDE), &keep, (SIDE, SIDE), s, d)
        };
        assert_depth_eq(&what, &read_depth(id, SIDE as i32, SIDE as i32), &want);
        untouched_layers(tex, n, None, &what);
        if refused_today {
            still_usable();
        }
        release(&[sfb, dfb], &[tex], &[rb]);
    });
}

/// Faces +X and -Z of a D24 and a D32F cube, 1:1, scaled, and flipped.
fn each_face(refused_today: bool) {
    for fmt in [D24, D32F] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face as u32, WHOLE, WHOLE, refused_today);
            from_layer(Layered::Cube, fmt, face as u32, WHOLE, [0, 0, 4, 4], refused_today);
            from_layer(Layered::Cube, fmt, face as u32, [1, 2, 6, 7], [7, 1, 2, 6], refused_today);
        }
    }
}

fn each_layer(refused_today: bool) {
    for fmt in [D24, D32F] {
        for layer in [0, 2] {
            from_layer(Layered::Array, fmt, layer, WHOLE, WHOLE, refused_today);
            from_layer(Layered::Array, fmt, layer, WHOLE, [0, 0, 4, 4], refused_today);
            from_layer(Layered::Array, fmt, layer, [1, 2, 6, 7], [7, 1, 2, 6], refused_today);
        }
    }
}

#[test]
fn depth_from_a_cube_face_is_refused_today() {
    each_face(true);
}

#[test]
#[ignore = "needs gl-ds-layers and GC7000UL ds_write/stencil_resolve"]
fn depth_from_a_cube_face_into_a_renderbuffer_copies_that_face() {
    each_face(false);
}

#[test]
fn depth_from_an_array_layer_is_refused_today() {
    each_layer(true);
}

#[test]
#[ignore = "needs gl-ds-layers and GC7000UL ds_write/stencil_resolve"]
fn depth_from_an_array_layer_into_a_renderbuffer_copies_that_layer() {
    each_layer(false);
}

/* ---- (3) depth from a renderbuffer into a cube face or an array layer ------- */

/// A `sz` x `sz` depth renderbuffer cleared to `SRC_CLEAR` blitted `s` -> `d`
/// into face/layer `k` of a layered texture of `fmt`. Refused today
/// (REFUSED_DEPTH_DST: the source is readable by `depth_resolve`): every
/// layer stays. Otherwise only layer `k` takes the model's picture.
fn into_layer(kind: Layered, fmt: Fmt, k: u32, sz: i32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    on_gc7000(|| {
        let n = layer_count(kind);
        let tex = layered_texture(kind, fmt, SIDE, n, layer_depth);
        let (sfb, rb) = depth_rb(fmt, sz, sz, SRC_CLEAR.0);
        let id = ds_id(rb);
        let dfb = layer_source(kind, tex, k);
        bind(sfb, dfb);
        let what = format!("{kind:?} {fmt:?} renderbuffer into layer {k}: {s:?} -> {d:?}");
        let keep = |x, y| layer_depth(k, x, y);
        let want = if refused_today {
            refused(s, d, DEPTH, &what);
            (0..SIDE * SIDE).map(|i| keep(i % SIDE, i / SIDE)).collect()
        } else {
            blit(s, d, DEPTH);
            assert_eq!(err(), GL_NO_ERROR, "{what}");
            let src = |_, _| SRC_CLEAR.0;
            model(&src, (sz as u32, sz as u32), &keep, (SIDE, SIDE), s, d)
        };
        assert_depth_eq(&what, &layer_picture(tex, k, SIDE), &want);
        untouched_layers(tex, n, Some(k), &what);
        assert_depth_eq(&what, &read_depth(id, sz, sz), &vec![SRC_CLEAR.0; (sz * sz) as usize]);
        if refused_today {
            still_usable();
        }
        release(&[sfb, dfb], &[tex], &[rb]);
    });
}

fn into_faces(refused_today: bool) {
    for fmt in [D24, D32F] {
        into_layer(Layered::Cube, fmt, FACE_PX as u32, 8, WHOLE, WHOLE, refused_today);
        into_layer(Layered::Cube, fmt, FACE_NZ as u32, 4, [0, 0, 4, 4], WHOLE, refused_today);
        into_layer(Layered::Cube, fmt, 2, 8, WHOLE, [2, 2, 6, 6], refused_today);
    }
}

fn into_layers(refused_today: bool) {
    for fmt in [D24, D32F] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE, refused_today);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE, refused_today);
        into_layer(Layered::Array, fmt, 1, 8, WHOLE, [2, 2, 6, 6], refused_today);
    }
}

#[test]
fn depth_into_a_cube_face_is_refused_today() {
    into_faces(true);
}

#[test]
#[ignore = "needs gl-ds-layers and GC7000UL ds_write/stencil_resolve"]
fn depth_from_a_renderbuffer_into_a_cube_face_changes_only_that_face() {
    into_faces(false);
}

#[test]
fn depth_into_an_array_layer_is_refused_today() {
    into_layers(true);
}

#[test]
#[ignore = "needs gl-ds-layers and GC7000UL ds_write/stencil_resolve"]
fn depth_from_a_renderbuffer_into_an_array_layer_changes_only_that_layer() {
    into_layers(false);
}

/* ---- (4) stencil to and from a D24S8 texture -------------------------------- */

const TEX_DEPTH: f32 = 0.5;
const R: [i32; 4] = [0, 0, 4, 4];

/// Stencil from a renderbuffer into a D24S8 texture, and from a D24S8
/// texture into a renderbuffer (REFUSED_STENCIL), each alone and with depth.
#[test]
fn stencil_to_and_from_a_texture_is_refused_today() {
    on_gc7000(|| {
        let (src_fb, src_rb) = ds_fbo(4, 4, SRC_CLEAR);
        let (out_fb, out_rb) = ds_fbo(4, 4, DST_CLEAR);
        let (tex_fb, tex) = tex_fbo(4, 4, TEX_DEPTH);
        let tex_before = picture(tex, 4, 4);
        let (src_id, out_id) = (ds_id(src_rb), ds_id(out_rb));
        let (src_before, out_before) = (read_depth(src_id, 4, 4), read_depth(out_id, 4, 4));
        bind(src_fb, tex_fb);
        refused(R, R, STENCIL, "stencil into a texture");
        refused(R, R, DEPTH | STENCIL, "depth and stencil into a texture");
        bind(tex_fb, out_fb);
        refused(R, R, STENCIL, "stencil from a texture");
        refused(R, R, DEPTH | STENCIL, "depth and stencil from a texture");
        assert_depth_eq("source", &read_depth(src_id, 4, 4), &src_before);
        assert_depth_eq("destination", &read_depth(out_id, 4, 4), &out_before);
        assert_depth_eq("the texture", &picture(tex, 4, 4), &tex_before);
        still_usable();
        release(&[src_fb, out_fb, tex_fb], &[tex], &[src_rb, out_rb]);
    });
}

/// Renderbuffer -> texture -> second renderbuffer, the second hop into the
/// middle of the destination: the second renderbuffer's stencil plane, the
/// only place stencil can be seen, holds the source's stencil in the
/// destination rectangle and its own clear elsewhere; the texture's depth is
/// not what the stencil blits wrote.
#[test]
#[ignore = "needs gl-ds-layers and GC7000UL ds_write/stencil_resolve"]
fn stencil_goes_through_a_texture_and_back() {
    on_gc7000(|| {
        let (src_fb, src_rb) = ds_fbo(4, 4, SRC_CLEAR);
        let (out_fb, out_rb) = ds_fbo(8, 8, DST_CLEAR);
        let (tex_fb, tex) = tex_fbo(4, 4, TEX_DEPTH);
        let tex_before = picture(tex, 4, 4);
        let out_id = ds_id(out_rb);
        bind(src_fb, tex_fb);
        blit(R, R, STENCIL);
        assert_eq!(err(), GL_NO_ERROR, "stencil into a texture");
        assert_depth_eq("depth untouched", &picture(tex, 4, 4), &tex_before);
        bind(tex_fb, out_fb);
        let d = [2, 2, 6, 6];
        blit(R, d, STENCIL);
        assert_eq!(err(), GL_NO_ERROR, "stencil from a texture");
        let want: Vec<u8> = model(&|_, _| SRC_CLEAR.1 as f32, (4, 4), &|_, _| DST_CLEAR.1 as f32, (8, 8), R, d)
            .into_iter()
            .map(|v| v as u8)
            .collect();
        assert_eq!(read_stencil(out_id, 8, 8), want, "the stencil of the second renderbuffer");
        release(&[src_fb, out_fb, tex_fb], &[tex], &[src_rb, out_rb]);
    });
}

/// The table entries the refusals rest on; the line the lift changes.
#[test]
fn the_table_lacks_ds_write_and_stencil_resolve_today() {
    assert!(yakogl_backend_vivante.ds_write.is_none(), "ds_write (src/vivante/backend.rs)");
    assert!(yakogl_backend_vivante.stencil_resolve.is_none(), "stencil_resolve (src/vivante/backend.rs)");
    assert!(yakogl_backend_vivante.depth_resolve.is_some(), "depth_resolve stays");
}
