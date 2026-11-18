// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Depth blits from a cube face or an array layer of a depth texture, depth
//! blits into one, and stencil blits to and from a texture (plan item
//! gl-ds-layers-tests; ES 3.0.6 4.3.3). The depth ones are
//! GL_INVALID_OPERATION today (src/gl/blit.rs REFUSED_DEPTH_SRC /
//! REFUSED_DEPTH_DST: `depth_texels` and `texture::resolve_depth` take
//! GL_TEXTURE_2D only, and `GlesDraw::depth_id` names a texture, not a face
//! or layer). Stencil goes to and from level 0 of a 2D depth-stencil texture
//! through the plane the back end keeps for it (item gl-ds-layers); a cube
//! face, an array layer or another level stays REFUSED_STENCIL.
//!
//! Each shape is a pair. `..._is_refused_today` pins the answer of today's
//! code: exactly one GL_INVALID_OPERATION, the destination unchanged, the
//! context usable afterwards. The `#[ignore]`d twin holds what a working blit
//! leaves, from the nearest-sample model of es3_blit_ds.rs (written apart
//! from the code); it fails today and is the target of the item that lifts
//! the refusal. Run them with `-- --ignored`.
//!
//! The API entries these tests need are not refused before blit.rs: a
//! GL_DEPTH_COMPONENT24 / 32F cube map (glTexImage2D on the six face
//! targets), a depth GL_TEXTURE_2D_ARRAY (glTexImage3D),
//! glFramebufferTexture2D with a face target and glFramebufferTextureLayer
//! with a depth attachment all succeed and the framebuffer is complete. The
//! blit alone answers GL_INVALID_OPERATION.
//!
//! Layout the helpers rely on (src/gl/context.rs `layers`): a cube map or
//! array keeps its faces/layers stacked vertically in one linear storage,
//! layer k's row y at row k * height + y; faces are +X, -X, +Y, -Y, +Z, -Z.
//! Stencil cannot be read with glReadPixels in ES 3.0, so the stencil checks
//! blit on into a renderbuffer and read the stand-in's `kept_plane`.
//!
//! No GPU runs: the stand-in back end draws nothing.
//!
//! Run: `cargo test --features host-test,gpu-powervr --test es3_blit_ds_layers`
//! (a GPU feature is needed to build the crate; gpu-vivante, gpu-v3d and
//! gpu-mali do as well)
//! (add `-- --ignored` for the expected results, which fail until lifted).
//!
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there (gl/framebuffer.rs `blit_framebuffer_is_es3_only`).

#![cfg(all(feature = "host-test", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glBlitFramebuffer, glCheckFramebufferStatus, glDeleteFramebuffers,
    glDeleteRenderbuffers, glFramebufferRenderbuffer, glFramebufferTexture2D, glFramebufferTextureLayer,
    glGenFramebuffers, glGenRenderbuffers, glRenderbufferStorage,
};
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::{glDisable, glGetError, glPixelStorei};
use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host, and a back end that keeps its planes -------- */

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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
    0
}

/// The planes the stand-in keeps, by (object, plane): width, height and one
/// u32 per pixel (a depth float's bits, or a stencil byte).
type Planes = std::collections::BTreeMap<(u32, u32), (u32, u32, Vec<u32>)>;
static PLANES: Mutex<Planes> = Mutex::new(Planes::new());
/// The ds_write calls it answered: (id, plane, x, y, w, h, pitch).
type Write = (u32, u32, u32, u32, u32, u32, u32);
static WRITES: Mutex<Vec<Write>> = Mutex::new(Vec::new());

/// Make the plane (`id`, `plane`), pixel (x, y) being `f(x, y)`.
fn keep_plane(id: u32, plane: u32, w: u32, h: u32, f: impl Fn(u32, u32) -> u32) {
    let px = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).insert((id, plane), (w, h, px));
}

fn kept_plane(id: u32, plane: u32) -> Vec<u32> {
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).get(&(id, plane)).expect("a kept plane").2.clone()
}

unsafe extern "C" fn rw_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    let planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get(&(id, backend::DS_PLANE_DEPTH)) else { return 1 };
    assert_eq!((*pw, *ph), (w, h));
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the caller's contract: `h` rows of `pitch` floats.
            unsafe { dst.add((y * pitch + x) as usize).write(f32::from_bits(px[(y * w + x) as usize])) };
        }
    }
    0
}

unsafe extern "C" fn rw_stencil_resolve(id: u32, dst: *mut u8, w: u32, h: u32, pitch: u32) -> i32 {
    let planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get(&(id, backend::DS_PLANE_STENCIL)) else { return 1 };
    assert_eq!((*pw, *ph), (w, h));
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the caller's contract.
            unsafe { dst.add((y * pitch + x) as usize).write(px[(y * w + x) as usize] as u8) };
        }
    }
    0
}

#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn rw_ds_write(
    id: u32,
    plane: u32,
    src: *const c_void,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    pitch: u32,
) -> i32 {
    if w != 0 && h != 0 {
        WRITES.lock().unwrap_or_else(|e| e.into_inner()).push((id, plane, x, y, w, h, pitch));
    }
    let mut planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get_mut(&(id, plane)) else { return 1 };
    assert!(x + w <= *pw && y + h <= *ph, "the rectangle is inside the plane");
    for j in 0..h {
        for i in 0..w {
            let at = (j * pitch + i) as usize;
            // SAFETY: the caller's contract: `h` rows of `pitch` elements.
            let v = unsafe {
                if plane == backend::DS_PLANE_STENCIL {
                    (src as *const u8).add(at).read() as u32
                } else {
                    (src as *const f32).add(at).read().to_bits()
                }
            };
            px[((y + j) * *pw + x + i) as usize] = v;
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

/// The stand-in with a back end that keeps its planes readable and writable
/// (`depth_resolve`, `ds_write`, `stencil_resolve`), so the refusals below
/// are the ones of texture faces, layers and stencil, not of a missing entry.
static BACKEND_RW: YakoglBackend = YakoglBackend {
    name: c"es3-blit-ds-layers".as_ptr() as *const c_char,
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
    depth_resolve: Some(rw_depth_resolve),
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: Some(rw_ds_write),
    stencil_resolve: Some(rw_stencil_resolve),
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND_RW;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glDisable(GL_SCISSOR_TEST);
        while glGetError() != GL_NO_ERROR {}
    }
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    WRITES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn blit(s: [i32; 4], d: [i32; 4], mask: GLbitfield, filter: GLenum) {
    // SAFETY: plain GL call.
    unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, filter) };
}

fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

/* ---- objects ----------------------------------------------------------- */

/// How a depth texture is specified: (internal format, format, type).
type Fmt = (GLenum, GLenum, GLenum);
const D32F: Fmt = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);
const D24: Fmt = (GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT);
const D24S8: Fmt = (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8);

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

/// A `w` x `h` renderbuffer of `format` at `point` of a new framebuffer:
/// (framebuffer, renderbuffer).
fn renderbuffer_fbo(format: GLenum, point: GLenum, w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (fb, rb)
}

/// The id the back ends keep renderbuffer `rb`'s depth under.
fn renderbuffer_ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

fn release(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names these tests made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
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
fn dst_depth(x: u32, y: u32) -> f32 {
    0.75 + (y * 32 + x) as f32 / 8192.0
}

/// Blit once and require today's answer: exactly one GL_INVALID_OPERATION.
fn refused(s: [i32; 4], d: [i32; 4], mask: GLbitfield, what: &str) {
    blit(s, d, mask, GL_NEAREST);
    assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
    assert_eq!(err(), GL_NO_ERROR, "{what}: exactly one error");
}

/// The context still blits after a refusal: a 2D depth texture copies.
fn still_usable() {
    let a = depth_texture(D32F, 4, 4, |x, y| (y * 4 + x) as f32 / 64.0);
    let b = depth_texture(D32F, 4, 4, |_, _| 0.5);
    let (fa, fb) = (texture_fbo(a, false), texture_fbo(b, false));
    bind(fa, fb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], GL_DEPTH_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR, "the context after a refusal");
    assert_eq!(picture(b, 4, 4), picture(a, 4, 4));
    release(&[fa, fb], &[a, b], &[]);
}

/* ---- (1)/(2) depth from a cube face or an array layer ------------------- */

const SIDE: u32 = 8;

/// One source (a face or layer of `tex`, by `src_fbo`) blitted `s` -> `d`
/// into a fresh 2D depth texture of `dz` x `dz`. `refused_today` pins the
/// refusal; otherwise the destination is compared with the model.
fn from_layer(kind: Layered, fmt: Fmt, k: u32, dz: u32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    let n = if matches!(kind, Layered::Cube) { 6 } else { 3 };
    let tex = layered_texture(kind, fmt, SIDE, n, layer_depth);
    let sfb = match kind {
        Layered::Cube => face_fbo(tex, k as usize),
        Layered::Array => layer_fbo(tex, k as i32),
    };
    let dst = depth_texture(fmt, dz, dz, dst_depth);
    let dfb = texture_fbo(dst, false);
    bind(sfb, dfb);
    let what = format!("{kind:?} {fmt:?} layer {k}: {s:?} -> {d:?}");
    if refused_today {
        refused(s, d, GL_DEPTH_BUFFER_BIT, &what);
        assert_eq!(picture(dst, dz, dz), (0..dz * dz).map(|i| dst_depth(i % dz, i / dz)).collect::<Vec<_>>());
    } else {
        blit(s, d, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR, "{what}");
        let src = |x, y| layer_depth(k, x, y);
        let want = model(&src, (SIDE, SIDE), &dst_depth, (dz, dz), s, d);
        assert_eq!(picture(dst, dz, dz), want, "{what}");
    }
    for l in 0..n {
        let want: Vec<f32> = (0..SIDE * SIDE).map(|i| layer_depth(l, i % SIDE, i / SIDE)).collect();
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} of the source is untouched");
    }
    release(&[sfb, dfb], &[tex, dst], &[]);
}

const WHOLE: [i32; 4] = [0, 0, SIDE as i32, SIDE as i32];

/// Faces +X and -Z of a D24 and a D32F cube, 1:1 and scaled up and down.
#[test]
fn depth_from_a_cube_face_is_refused_today() {
    let _g = context();
    for fmt in [D24, D32F] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face as u32, 8, WHOLE, WHOLE, true);
            from_layer(Layered::Cube, fmt, face as u32, 16, WHOLE, [0, 0, 16, 16], true);
            from_layer(Layered::Cube, fmt, face as u32, 4, WHOLE, [0, 0, 4, 4], true);
        }
    }
    still_usable();
}

#[test]
#[ignore = "refused today: src/gl/blit.rs REFUSED_DEPTH_SRC (depth from a cube face)"]
fn depth_from_a_cube_face_copies_that_face() {
    let _g = context();
    for fmt in [D24, D32F] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face as u32, 8, WHOLE, WHOLE, false);
            from_layer(Layered::Cube, fmt, face as u32, 16, WHOLE, [0, 0, 16, 16], false);
            from_layer(Layered::Cube, fmt, face as u32, 4, WHOLE, [0, 0, 4, 4], false);
            from_layer(Layered::Cube, fmt, face as u32, 8, [1, 2, 6, 7], [2, 1, 7, 6], false);
        }
    }
}

/// Layers 0 and 2 of a depth array, 1:1 and scaled.
#[test]
fn depth_from_an_array_layer_is_refused_today() {
    let _g = context();
    for fmt in [D24, D32F] {
        for layer in [0, 2] {
            from_layer(Layered::Array, fmt, layer, 8, WHOLE, WHOLE, true);
            from_layer(Layered::Array, fmt, layer, 16, WHOLE, [0, 0, 16, 16], true);
            from_layer(Layered::Array, fmt, layer, 4, WHOLE, [0, 0, 4, 4], true);
        }
    }
    still_usable();
}

#[test]
#[ignore = "refused today: src/gl/blit.rs REFUSED_DEPTH_SRC (depth from an array layer)"]
fn depth_from_an_array_layer_copies_that_layer() {
    let _g = context();
    for fmt in [D24, D32F] {
        for layer in [0, 2] {
            from_layer(Layered::Array, fmt, layer, 8, WHOLE, WHOLE, false);
            from_layer(Layered::Array, fmt, layer, 16, WHOLE, [0, 0, 16, 16], false);
            from_layer(Layered::Array, fmt, layer, 4, WHOLE, [0, 0, 4, 4], false);
            from_layer(Layered::Array, fmt, layer, 8, [1, 2, 6, 7], [2, 1, 7, 6], false);
        }
    }
}

/* ---- (3) depth into a cube face or an array layer ----------------------- */

fn src_depth(x: u32, y: u32) -> f32 {
    (y * 16 + x) as f32 / 1024.0
}

/// A 2D depth texture of `sz` x `sz` blitted `s` -> `d` into layer/face `k`
/// of a layered texture of `fmt`. The layer takes the model's picture when
/// the blit works (`refused_today` false) and stays as it was when it is
/// refused; every other layer stays either way.
fn into_layer(kind: Layered, fmt: Fmt, k: u32, sz: u32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    let n = if matches!(kind, Layered::Cube) { 6 } else { 3 };
    let tex = layered_texture(kind, fmt, SIDE, n, layer_depth);
    let src = depth_texture(fmt, sz, sz, src_depth);
    let sfb = texture_fbo(src, false);
    let dfb = match kind {
        Layered::Cube => face_fbo(tex, k as usize),
        Layered::Array => layer_fbo(tex, k as i32),
    };
    bind(sfb, dfb);
    let what = format!("{kind:?} {fmt:?} into layer {k}: {s:?} -> {d:?}");
    let keep = |x, y| layer_depth(k, x, y);
    if refused_today {
        refused(s, d, GL_DEPTH_BUFFER_BIT, &what);
    } else {
        blit(s, d, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        assert_eq!(err(), GL_NO_ERROR, "{what}");
    }
    let want_k = if refused_today {
        model(&keep, (SIDE, SIDE), &keep, (SIDE, SIDE), s, [0, 0, 0, 0])
    } else {
        model(&src_depth, (sz, sz), &keep, (SIDE, SIDE), s, d)
    };
    assert_eq!(layer_picture(tex, k, SIDE), want_k, "{what}: the layer");
    for l in (0..n).filter(|&l| l != k) {
        let want: Vec<f32> = (0..SIDE * SIDE).map(|i| layer_depth(l, i % SIDE, i / SIDE)).collect();
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} must not change");
    }
    assert_eq!(picture(src, sz, sz), (0..sz * sz).map(|i| src_depth(i % sz, i / sz)).collect::<Vec<_>>());
    release(&[sfb, dfb], &[tex, src], &[]);
}

#[test]
fn depth_into_a_cube_face_is_refused_today() {
    let _g = context();
    for fmt in [D24, D32F] {
        into_layer(Layered::Cube, fmt, FACE_PX as u32, 8, WHOLE, WHOLE, true);
        into_layer(Layered::Cube, fmt, FACE_NZ as u32, 4, [0, 0, 4, 4], WHOLE, true);
    }
    still_usable();
}

#[test]
#[ignore = "refused today: src/gl/blit.rs REFUSED_DEPTH_DST (depth into a cube face)"]
fn depth_into_a_cube_face_changes_only_that_face() {
    let _g = context();
    for fmt in [D24, D32F] {
        into_layer(Layered::Cube, fmt, FACE_PX as u32, 8, WHOLE, WHOLE, false);
        into_layer(Layered::Cube, fmt, FACE_NZ as u32, 4, [0, 0, 4, 4], WHOLE, false);
        into_layer(Layered::Cube, fmt, 2, 8, [0, 0, 8, 8], [2, 2, 6, 6], false);
    }
}

#[test]
fn depth_into_an_array_layer_is_refused_today() {
    let _g = context();
    for fmt in [D24, D32F] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE, true);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE, true);
    }
    still_usable();
}

#[test]
#[ignore = "refused today: src/gl/blit.rs REFUSED_DEPTH_DST (depth into an array layer)"]
fn depth_into_an_array_layer_changes_only_that_layer() {
    let _g = context();
    for fmt in [D24, D32F] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE, false);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE, false);
        into_layer(Layered::Array, fmt, 1, 8, [0, 0, 8, 8], [2, 2, 6, 6], false);
    }
}

/* ---- (4) stencil to and from a texture ----------------------------------- */

const STENCIL: u32 = backend::DS_PLANE_STENCIL;

fn stencil_of(x: u32, y: u32) -> u32 {
    10 + y * 4 + x
}

/// A D24S8 renderbuffer whose stencil plane the stand-in keeps as
/// `stencil_of`, a D24S8 2D texture whose plane the stand-in keeps (0s),
/// and a second renderbuffer holding 200s.
///
/// The texture's plane stands for the buffer a draw into it leaves on a
/// real back end: the stand-in draws nothing, and a texture whose plane the
/// back end does not hold cannot be written (`ds_write` answers 1, the blit
/// is refused as REFUSED_DS_UNHELD).
struct Stencil {
    src_fb: GLuint,
    src_rb: GLuint,
    tex: GLuint,
    tex_fb: GLuint,
    tex_id: u32,
    out_fb: GLuint,
    out_rb: GLuint,
    out_id: u32,
}

/// The id the back ends keep texture `tex`'s depth/stencil plane under.
fn texture_ds_id(tex: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    context::ds_object_id(c.texture_object(slot).expect("a texture").serial, true)
}

fn stencil_rig() -> Stencil {
    let (src_fb, src_rb) = renderbuffer_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (out_fb, out_rb) = renderbuffer_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (sid, out_id) = (renderbuffer_ds_id(src_rb), renderbuffer_ds_id(out_rb));
    keep_plane(sid, STENCIL, 4, 4, stencil_of);
    keep_plane(out_id, STENCIL, 4, 4, |_, _| 200);
    let tex = depth_texture(D24S8, 4, 4, dst_depth);
    let tex_fb = texture_fbo(tex, true);
    let tex_id = texture_ds_id(tex);
    keep_plane(tex_id, STENCIL, 4, 4, |_, _| 0);
    Stencil { src_fb, src_rb, tex, tex_fb, tex_id, out_fb, out_rb, out_id }
}

impl Stencil {
    fn release(self) {
        release(&[self.src_fb, self.tex_fb, self.out_fb], &[self.tex], &[self.src_rb, self.out_rb]);
    }
}

const R: [i32; 4] = [0, 0, 4, 4];

/// A framebuffer with an image of D24S8 texture `tex` at
/// GL_DEPTH_STENCIL_ATTACHMENT, checked complete: level `level` of face
/// target `target` (glFramebufferTexture2D), or with `layer` that layer of
/// level `level` (glFramebufferTextureLayer).
fn ds_image_fbo(target: GLenum, tex: GLuint, level: i32, layer: Option<i32>) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match layer {
            Some(l) => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, tex, level, l),
            None => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, target, tex, level),
        }
        assert_eq!(err(), GL_NO_ERROR, "a depth-stencil image at the attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

/// Stencil to and from what the plane the back end keeps for a texture
/// does not name stays refused: a face of a D24S8 cube, a layer of a D24S8
/// array, level 1 of a D24S8 2D texture (each with a plane the stand-in
/// keeps, so the refusal is not REFUSED_DS_UNHELD).
#[test]
fn stencil_to_and_from_a_face_a_layer_or_a_level_is_refused() {
    let _g = context();
    let r = stencil_rig();
    let cube = layered_texture(Layered::Cube, D24S8, 4, 6, layer_depth);
    let array = layered_texture(Layered::Array, D24S8, 4, 3, layer_depth);
    let mut mip = 0;
    // SAFETY: room for one name; no pixels.
    unsafe {
        glGenTextures(1, &mut mip);
        glBindTexture(GL_TEXTURE_2D, mip);
        for (level, side) in [(0, 4), (1, 2), (2, 1)] {
            glTexImage2D(GL_TEXTURE_2D, level, D24S8.0 as GLint, side, side, 0, D24S8.1, D24S8.2, core::ptr::null());
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "a D24S8 texture of three levels");
    let shapes = [
        (cube, ds_image_fbo(FACES[FACE_NZ], cube, 0, None), "a cube face"),
        (array, ds_image_fbo(GL_NONE, array, 0, Some(2)), "an array layer"),
        (mip, ds_image_fbo(GL_TEXTURE_2D, mip, 1, None), "level 1"),
    ];
    for &(tex, fb, what) in &shapes {
        keep_plane(texture_ds_id(tex), STENCIL, 4, 4, |_, _| 7);
        bind(r.src_fb, fb);
        refused(R, R, GL_STENCIL_BUFFER_BIT, &format!("stencil into {what}"));
        bind(fb, r.out_fb);
        refused(R, R, GL_STENCIL_BUFFER_BIT, &format!("stencil from {what}"));
        assert_eq!(kept_plane(texture_ds_id(tex), STENCIL), vec![7; 16], "{what}: its plane is unchanged");
    }
    assert_eq!(kept_plane(r.out_id, STENCIL), vec![200; 16], "the destination is unchanged");
    assert!(WRITES.lock().unwrap().is_empty(), "a refused call writes nothing");
    let fbs: Vec<GLuint> = shapes.iter().map(|s| s.1).collect();
    release(&fbs, &[cube, array, mip], &[]);
    r.release();
    still_usable();
}

/// Renderbuffer -> texture -> second renderbuffer, flipped in x on the
/// second hop: the second renderbuffer's stencil plane, the only place
/// stencil can be seen, holds the source's stencil mirrored, and the
/// texture's depth is not what the stencil blit wrote.
#[test]
fn stencil_goes_through_a_texture_and_back() {
    let _g = context();
    let r = stencil_rig();
    bind(r.src_fb, r.tex_fb);
    blit(R, R, GL_STENCIL_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR, "stencil into a texture");
    assert_eq!(picture(r.tex, 4, 4), (0..16).map(|i| dst_depth(i % 4, i / 4)).collect::<Vec<_>>(), "depth untouched");
    assert_eq!(kept_plane(r.tex_id, STENCIL), (0..16).map(|i| stencil_of(i % 4, i / 4)).collect::<Vec<_>>());
    assert!(WRITES.lock().unwrap().iter().all(|w| w.0 == r.tex_id && w.1 == STENCIL), "the texture's stencil");
    bind(r.tex_fb, r.out_fb);
    blit(R, [4, 0, 0, 4], GL_STENCIL_BUFFER_BIT, GL_NEAREST);
    assert_eq!(err(), GL_NO_ERROR, "stencil from a texture");
    let got = kept_plane(r.out_id, STENCIL);
    for y in 0..4u32 {
        for x in 0..4u32 {
            assert_eq!(got[(y * 4 + x) as usize], stencil_of(3 - x, y), "stencil ({x}, {y})");
        }
    }
    r.release();
}
