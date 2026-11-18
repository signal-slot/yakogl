// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth from a cube face or an array layer of a depth
//! texture, depth into one, and stencil into and out of a DEPTH24_STENCIL8
//! texture, each against a renderbuffer, over the Mali (Valhall G610) back
//! end's table, on the host: what the GL core answers today, pinned, and
//! `#[ignore]`d twins that hold what a working blit leaves. The Mali
//! counterpart of tests/es3_blit_ds_layers.rs (the GL core's half, on a
//! stand-in with all three entries; docs/notes/blit-depth-layers.md).
//!
//! HOST-ONLY: an API verdict of the GL core on the Mali's table; no Mali
//! frame runs and no picture is read (tools/cts/README.md 2b).
//!
//! The Mali's table (`yakogl_backend_mali`, src/mali/backend.rs) has
//! `depth_resolve` but no `ds_write` and no `stencil_resolve` (plan #152).
//! Every shape here is refused today, and which answer it is:
//!   - depth from a cube face or array layer into a renderbuffer, and depth
//!     from a renderbuffer into a face or layer: REFUSED_DEPTH_DST of
//!     src/gl/blit.rs. The destination is a renderbuffer the back end cannot
//!     write (`ds_write` none), or a face/layer, which `depth_texels` does
//!     not take. A face/layer source is REFUSED_DEPTH_SRC besides, but the
//!     blit.rs match names the destination first, so the source alone is not
//!     reachable on this table; the stand-in with `ds_write` of
//!     tests/es3_blit_ds_layers.rs pins it;
//!   - stencil into or out of a D24S8 texture: REFUSED_STENCIL (the copy
//!     needs a renderbuffer on both sides, and this table has neither
//!     `ds_write` nor `stencil_resolve` anyway).
//!
//! A refused call fails whole: exactly one GL_INVALID_OPERATION, nothing
//! copied, the context usable.
//!
//! The rig is tests/es3_blit_ds_window_mali.rs's, copied: the GL core and a
//! stand-in table with the Mali's entries (`depth_resolve` seeded by the
//! test; `ds_write` and `stencil_resolve` none, which a test checks against
//! `yakogl_backend_mali`). The twins run on `BACKEND_LIFTED`: the same table
//! with stand-in `ds_write` and `stencil_resolve` that keep planes in memory.
//! The model (`model_axis`, `model`) is tests/es3_blit_ds_layers.rs's:
//! nearest sample, written apart from the code.
//!
//! The twins need two things before they pass: the GL core's half (plan item
//! gl-ds-layers: depth from/into a face or layer, stencil to/from a texture)
//! and the Mali's entries (plan #152's Mali work). To un-ignore: remove the
//! `#[ignore]`s once both have landed and replace `BACKEND_LIFTED`'s two
//! entries with `yakogl_backend_mali`'s on a Mali OSAL rig that reads the
//! planes back through them; the seeded `depth_resolve` stands until then.
//!
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-mali,shaderc-online,no-shader-table --test
//! es3_blit_ds_layers_mali` (add `-- --ignored` for the twins).

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

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
use yakogl::mali::backend::yakogl_backend_mali;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

const D: u32 = backend::DS_PLANE_DEPTH;
const S: u32 = backend::DS_PLANE_STENCIL;

/* ---- a context on the host, and back ends that keep their planes -------- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    // SAFETY: the caller passes a writable descriptor.
    unsafe {
        *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void }
    };
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
/// The ds_write calls it answered with a non-empty rectangle.
static WRITES: Mutex<usize> = Mutex::new(0);

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
    let Some((pw, ph, px)) = planes.get(&(id, D)) else { return 1 };
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
    let Some((pw, ph, px)) = planes.get(&(id, S)) else { return 1 };
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
        *WRITES.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    }
    let mut planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get_mut(&(id, plane)) else { return 1 };
    assert!(x + w <= *pw && y + h <= *ph, "the rectangle is inside the plane");
    for j in 0..h {
        for i in 0..w {
            let at = (j * pitch + i) as usize;
            // SAFETY: the caller's contract: `h` rows of `pitch` elements.
            let v = unsafe {
                if plane == S {
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

type Resolve = Option<unsafe extern "C" fn(u32, *mut f32, u32, u32, u32) -> i32>;
type StencilResolve = Option<unsafe extern "C" fn(u32, *mut u8, u32, u32, u32) -> i32>;
type DsWrite = Option<unsafe extern "C" fn(u32, u32, *const c_void, u32, u32, u32, u32, u32) -> i32>;

/// A stand-in with the given entries.
const fn backend_with(
    name: *const c_char,
    depth_resolve: Resolve,
    ds_write: DsWrite,
    stencil_resolve: StencilResolve,
) -> YakoglBackend {
    YakoglBackend {
        name,
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
        depth_resolve,
        query_begin: None,
        query_end: None,
        query_result: None,
        generate_mipmap: None,
        return_mipmap_chain: None,
        linear_row_align: 0,
        uniforms_host_only: false,
        ds_write,
        stencil_resolve,
        max_samples: 0,
    }
}

/// The Mali's table as the GL core sees it: `depth_resolve` (seeded here),
/// no `ds_write`, no `stencil_resolve` (plan #152).
static BACKEND: YakoglBackend = backend_with(c"layers-mali".as_ptr(), Some(rw_depth_resolve), None, None);
/// The same with the entries #152 adds, as a stand-in keeping planes.
static BACKEND_LIFTED: YakoglBackend =
    backend_with(c"layers-mali-lifted".as_ptr(), Some(rw_depth_resolve), Some(rw_ds_write), Some(rw_stencil_resolve));

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context_on(b: &'static YakoglBackend) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = b;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        backend::g_backend = b;
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glDisable(GL_SCISSOR_TEST);
        while glGetError() != GL_NO_ERROR {}
    }
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    *WRITES.lock().unwrap_or_else(|e| e.into_inner()) = 0;
    g
}

/// The context on the Mali's table (the refusals).
fn context_mali() -> MutexGuard<'static, ()> {
    context_on(&BACKEND)
}

/// The context on the lifted table (the twins).
fn context_lifted() -> MutexGuard<'static, ()> {
    context_on(&BACKEND_LIFTED)
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn blit(s: [i32; 4], d: [i32; 4], mask: GLbitfield) {
    // SAFETY: plain GL call.
    unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, GL_NEAREST) };
}

fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

/// The table is what this file says it is: the Mali's has no `ds_write` and
/// no `stencil_resolve`, and the stand-in agrees; the lifted one has both.
#[test]
fn the_mali_table_has_neither_entry() {
    assert!(yakogl_backend_mali.depth_resolve.is_some());
    assert!(yakogl_backend_mali.ds_write.is_none(), "the Mali's table has a ds_write");
    assert!(yakogl_backend_mali.stencil_resolve.is_none(), "the Mali's table has a stencil_resolve");
    assert_eq!(BACKEND.ds_write.is_some(), yakogl_backend_mali.ds_write.is_some());
    assert_eq!(BACKEND.stencil_resolve.is_some(), yakogl_backend_mali.stencil_resolve.is_some());
    assert!(BACKEND_LIFTED.ds_write.is_some() && BACKEND_LIFTED.stencil_resolve.is_some());
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
const FACE_PX: u32 = 0;
const FACE_NZ: u32 = 5;

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

/// A `side` x `side` cube map (six faces) or 2D array (`n` layers) of depth,
/// depth (x, y) of layer k = `f(k, x, y)`.
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

/// A framebuffer with the 2D texture `tex` at GL_DEPTH_ATTACHMENT (or at
/// GL_DEPTH_STENCIL_ATTACHMENT with `stencil_too`), checked complete.
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

fn face_fbo(tex: GLuint, face: u32) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, FACES[face as usize], tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "a cube face at the depth attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

fn layer_fbo(tex: GLuint, layer: u32) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, tex, 0, layer as i32);
        assert_eq!(err(), GL_NO_ERROR, "an array layer at the depth attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

fn layer_target_fbo(kind: Layered, tex: GLuint, k: u32) -> GLuint {
    match kind {
        Layered::Cube => face_fbo(tex, k),
        Layered::Array => layer_fbo(tex, k),
    }
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

/// The id the back ends keep renderbuffer `rb`'s depth and stencil under.
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

/// The depth plane of the renderbuffer kept under `id`, as floats.
fn plane_depth(id: u32) -> Vec<f32> {
    kept_plane(id, D).into_iter().map(f32::from_bits).collect()
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

/// Depth of layer/face `k` at (x, y): a ramp per layer, distinct everywhere,
/// exact in f32 and in the 24 bits of a D24.
fn layer_depth(k: u32, x: u32, y: u32) -> f32 {
    (k * 256 + y * 16 + x) as f32 / 4096.0
}
fn dst_depth(x: u32, y: u32) -> f32 {
    0.75 + (y * 32 + x) as f32 / 8192.0
}
fn src_depth(x: u32, y: u32) -> f32 {
    (y * 16 + x) as f32 / 1024.0
}

/// Blit once and require today's answer: exactly one GL_INVALID_OPERATION.
fn refused(s: [i32; 4], d: [i32; 4], mask: GLbitfield, what: &str) {
    blit(s, d, mask);
    assert_eq!(err(), GL_INVALID_OPERATION, "{what}");
    assert_eq!(err(), GL_NO_ERROR, "{what}: exactly one error");
}

/// The context still blits after a refusal: a 2D depth texture copies.
fn still_usable() {
    let a = depth_texture(D32F, 4, 4, |x, y| (y * 4 + x) as f32 / 64.0);
    let b = depth_texture(D32F, 4, 4, |_, _| 0.5);
    let (fa, fb) = (texture_fbo(a, false), texture_fbo(b, false));
    bind(fa, fb);
    blit([0, 0, 4, 4], [0, 0, 4, 4], GL_DEPTH_BUFFER_BIT);
    assert_eq!(err(), GL_NO_ERROR, "the context after a refusal");
    assert_eq!(picture(b, 4, 4), picture(a, 4, 4));
    release(&[fa, fb], &[a, b], &[]);
}

const SIDE: u32 = 8;
const WHOLE: [i32; 4] = [0, 0, SIDE as i32, SIDE as i32];

fn layer_count(kind: Layered) -> u32 {
    if matches!(kind, Layered::Cube) {
        6
    } else {
        3
    }
}

/// Every layer of `tex` but `skip` still holds `layer_depth`.
fn assert_layers_untouched(tex: GLuint, kind: Layered, skip: Option<u32>, what: &str) {
    for l in (0..layer_count(kind)).filter(|&l| Some(l) != skip) {
        let want: Vec<f32> = (0..SIDE * SIDE).map(|i| layer_depth(l, i % SIDE, i / SIDE)).collect();
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} must not change");
    }
}

/* ---- (1)/(2) depth from a cube face or an array layer into a renderbuffer */

/// Face/layer `k` of a layered depth texture blitted `s` -> `d` into a
/// `dz` x `dz` depth renderbuffer seeded with `dst_depth`. Refused today
/// (REFUSED_DEPTH_DST: the Mali cannot write a renderbuffer's depth); the
/// renderbuffer's plane stays. Lifted: it takes the model's picture.
fn from_layer(kind: Layered, fmt: Fmt, k: u32, dz: u32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    let tex = layered_texture(kind, fmt, SIDE, layer_count(kind), layer_depth);
    let sfb = layer_target_fbo(kind, tex, k);
    let (dfb, rb) = renderbuffer_fbo(fmt.0, GL_DEPTH_ATTACHMENT, dz as i32, dz as i32);
    let id = renderbuffer_ds_id(rb);
    keep_plane(id, D, dz, dz, |x, y| dst_depth(x, y).to_bits());
    bind(sfb, dfb);
    let what = format!("{kind:?} {fmt:?} layer {k}: {s:?} -> {d:?}");
    if refused_today {
        refused(s, d, GL_DEPTH_BUFFER_BIT, &what);
        let before: Vec<f32> = (0..dz * dz).map(|i| dst_depth(i % dz, i / dz)).collect();
        assert_eq!(plane_depth(id), before, "{what}: the renderbuffer is unchanged");
        assert_eq!(*WRITES.lock().unwrap(), 0, "{what}: a refused call writes nothing");
    } else {
        blit(s, d, GL_DEPTH_BUFFER_BIT);
        assert_eq!(err(), GL_NO_ERROR, "{what}");
        let src = |x, y| layer_depth(k, x, y);
        let want = model(&src, (SIDE, SIDE), &dst_depth, (dz, dz), s, d);
        assert_eq!(plane_depth(id), want, "{what}");
    }
    assert_layers_untouched(tex, kind, None, &what);
    release(&[sfb, dfb], &[tex], &[rb]);
}

#[test]
fn depth_from_a_cube_face_is_refused_today() {
    let _g = context_mali();
    for fmt in [D24, D32F] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face, 8, WHOLE, WHOLE, true);
            from_layer(Layered::Cube, fmt, face, 16, WHOLE, [0, 0, 16, 16], true);
            from_layer(Layered::Cube, fmt, face, 4, WHOLE, [0, 0, 4, 4], true);
        }
    }
    still_usable();
}

#[test]
#[ignore = "needs gl-ds-layers and Mali ds_write/stencil_resolve"]
fn depth_from_a_cube_face_copies_that_face() {
    let _g = context_lifted();
    for fmt in [D24, D32F] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face, 8, WHOLE, WHOLE, false);
            from_layer(Layered::Cube, fmt, face, 16, WHOLE, [0, 0, 16, 16], false);
            from_layer(Layered::Cube, fmt, face, 4, WHOLE, [0, 0, 4, 4], false);
            from_layer(Layered::Cube, fmt, face, 8, [1, 2, 6, 7], [2, 1, 7, 6], false);
        }
    }
}

#[test]
fn depth_from_an_array_layer_is_refused_today() {
    let _g = context_mali();
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
#[ignore = "needs gl-ds-layers and Mali ds_write/stencil_resolve"]
fn depth_from_an_array_layer_copies_that_layer() {
    let _g = context_lifted();
    for fmt in [D24, D32F] {
        for layer in [0, 2] {
            from_layer(Layered::Array, fmt, layer, 8, WHOLE, WHOLE, false);
            from_layer(Layered::Array, fmt, layer, 16, WHOLE, [0, 0, 16, 16], false);
            from_layer(Layered::Array, fmt, layer, 4, WHOLE, [0, 0, 4, 4], false);
            from_layer(Layered::Array, fmt, layer, 8, [1, 2, 6, 7], [2, 1, 7, 6], false);
        }
    }
}

/* ---- (3) depth from a renderbuffer into a cube face or an array layer --- */

/// A `sz` x `sz` depth renderbuffer seeded with `src_depth` blitted `s` -> `d`
/// into face/layer `k`. Refused today (REFUSED_DEPTH_DST: `depth_texels`
/// takes GL_TEXTURE_2D only, and the Mali has no `ds_write`); the layer
/// stays. Lifted: it takes the model's picture, the other layers stay.
fn into_layer(kind: Layered, fmt: Fmt, k: u32, sz: u32, s: [i32; 4], d: [i32; 4], refused_today: bool) {
    let tex = layered_texture(kind, fmt, SIDE, layer_count(kind), layer_depth);
    let (sfb, rb) = renderbuffer_fbo(fmt.0, GL_DEPTH_ATTACHMENT, sz as i32, sz as i32);
    let id = renderbuffer_ds_id(rb);
    keep_plane(id, D, sz, sz, |x, y| src_depth(x, y).to_bits());
    let dfb = layer_target_fbo(kind, tex, k);
    bind(sfb, dfb);
    let what = format!("{kind:?} {fmt:?} into layer {k}: {s:?} -> {d:?}");
    let keep = |x, y| layer_depth(k, x, y);
    let want_k = if refused_today {
        refused(s, d, GL_DEPTH_BUFFER_BIT, &what);
        assert_eq!(*WRITES.lock().unwrap(), 0, "{what}: a refused call writes nothing");
        (0..SIDE * SIDE).map(|i| keep(i % SIDE, i / SIDE)).collect()
    } else {
        blit(s, d, GL_DEPTH_BUFFER_BIT);
        assert_eq!(err(), GL_NO_ERROR, "{what}");
        model(&src_depth, (sz, sz), &keep, (SIDE, SIDE), s, d)
    };
    assert_eq!(layer_picture(tex, k, SIDE), want_k, "{what}: the layer");
    assert_layers_untouched(tex, kind, Some(k), &what);
    assert_eq!(plane_depth(id), (0..sz * sz).map(|i| src_depth(i % sz, i / sz)).collect::<Vec<_>>());
    release(&[sfb, dfb], &[tex], &[rb]);
}

#[test]
fn depth_into_a_cube_face_is_refused_today() {
    let _g = context_mali();
    for fmt in [D24, D32F] {
        into_layer(Layered::Cube, fmt, FACE_PX, 8, WHOLE, WHOLE, true);
        into_layer(Layered::Cube, fmt, FACE_NZ, 4, [0, 0, 4, 4], WHOLE, true);
    }
    still_usable();
}

#[test]
#[ignore = "needs gl-ds-layers and Mali ds_write/stencil_resolve"]
fn depth_into_a_cube_face_changes_only_that_face() {
    let _g = context_lifted();
    for fmt in [D24, D32F] {
        into_layer(Layered::Cube, fmt, FACE_PX, 8, WHOLE, WHOLE, false);
        into_layer(Layered::Cube, fmt, FACE_NZ, 4, [0, 0, 4, 4], WHOLE, false);
        into_layer(Layered::Cube, fmt, 2, 8, [0, 0, 8, 8], [2, 2, 6, 6], false);
    }
}

#[test]
fn depth_into_an_array_layer_is_refused_today() {
    let _g = context_mali();
    for fmt in [D24, D32F] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE, true);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE, true);
    }
    still_usable();
}

#[test]
#[ignore = "needs gl-ds-layers and Mali ds_write/stencil_resolve"]
fn depth_into_an_array_layer_changes_only_that_layer() {
    let _g = context_lifted();
    for fmt in [D24, D32F] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE, false);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE, false);
        into_layer(Layered::Array, fmt, 1, 8, [0, 0, 8, 8], [2, 2, 6, 6], false);
    }
}

/* ---- (4) stencil to and from a texture ----------------------------------- */

fn stencil_of(x: u32, y: u32) -> u32 {
    10 + y * 4 + x
}

/// A D24S8 renderbuffer whose stencil plane the stand-in keeps as
/// `stencil_of`, a D24S8 2D texture, and a second renderbuffer holding 200s.
struct Stencil {
    src_fb: GLuint,
    src_rb: GLuint,
    tex: GLuint,
    tex_fb: GLuint,
    out_fb: GLuint,
    out_rb: GLuint,
    out_id: u32,
}

fn stencil_rig() -> Stencil {
    let (src_fb, src_rb) = renderbuffer_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (out_fb, out_rb) = renderbuffer_fbo(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT, 4, 4);
    let (sid, out_id) = (renderbuffer_ds_id(src_rb), renderbuffer_ds_id(out_rb));
    keep_plane(sid, S, 4, 4, stencil_of);
    keep_plane(out_id, S, 4, 4, |_, _| 200);
    let tex = depth_texture(D24S8, 4, 4, dst_depth);
    let tex_fb = texture_fbo(tex, true);
    Stencil { src_fb, src_rb, tex, tex_fb, out_fb, out_rb, out_id }
}

impl Stencil {
    fn release(self) {
        release(&[self.src_fb, self.tex_fb, self.out_fb], &[self.tex], &[self.src_rb, self.out_rb]);
    }
}

const R: [i32; 4] = [0, 0, 4, 4];

/// REFUSED_STENCIL: stencil from a renderbuffer into a D24S8 texture, and
/// from a D24S8 texture into a renderbuffer (with or without depth).
#[test]
fn stencil_to_and_from_a_texture_is_refused_today() {
    let _g = context_mali();
    let r = stencil_rig();
    bind(r.src_fb, r.tex_fb);
    refused(R, R, GL_STENCIL_BUFFER_BIT, "stencil into a texture");
    bind(r.tex_fb, r.out_fb);
    refused(R, R, GL_STENCIL_BUFFER_BIT, "stencil from a texture");
    refused(R, R, GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT, "depth and stencil from a texture");
    assert_eq!(kept_plane(r.out_id, S), vec![200; 16], "the destination is unchanged");
    assert_eq!(*WRITES.lock().unwrap(), 0, "a refused call writes nothing");
    assert_eq!(picture(r.tex, 4, 4), (0..16).map(|i| dst_depth(i % 4, i / 4)).collect::<Vec<_>>());
    r.release();
    still_usable();
}

/// Renderbuffer -> texture -> second renderbuffer, flipped in x on the
/// second hop: the second renderbuffer's stencil plane, the only place
/// stencil can be seen, holds the source's stencil mirrored, and the
/// texture's depth is not what the stencil blit wrote.
#[test]
#[ignore = "needs gl-ds-layers and Mali ds_write/stencil_resolve"]
fn stencil_goes_through_a_texture_and_back() {
    let _g = context_lifted();
    let r = stencil_rig();
    bind(r.src_fb, r.tex_fb);
    blit(R, R, GL_STENCIL_BUFFER_BIT);
    assert_eq!(err(), GL_NO_ERROR, "stencil into a texture");
    assert_eq!(picture(r.tex, 4, 4), (0..16).map(|i| dst_depth(i % 4, i / 4)).collect::<Vec<_>>(), "depth untouched");
    bind(r.tex_fb, r.out_fb);
    blit(R, [4, 0, 0, 4], GL_STENCIL_BUFFER_BIT);
    assert_eq!(err(), GL_NO_ERROR, "stencil from a texture");
    let got = kept_plane(r.out_id, S);
    for y in 0..4u32 {
        for x in 0..4u32 {
            assert_eq!(got[(y * 4 + x) as usize], stencil_of(3 - x, y), "stencil ({x}, {y})");
        }
    }
    r.release();
}
