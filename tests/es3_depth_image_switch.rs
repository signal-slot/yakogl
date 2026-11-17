// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth plane of a cube or array depth texture when draws switch faces
//! or layers (plan item depth-switch-tests; docs/notes/blit-depth-layers.md,
//! "What this means today" 1 and 2, and design (b)).
//!
//! Today every back end keeps one depth plane per depth texture:
//! `texture::resolve_depth` writes it into face/layer 0 of the level, and
//! `texture::depth_seed` seeds only from face/layer 0. Design (b) records the
//! (level, layer) the plane holds (`depth_image_switch`): a draw into another
//! face first resolves the old face into its own rows, then seeds the new
//! face from its rows.
//!
//! Two kinds of test:
//! - plain tests pin what must not change: 2D depth textures resolve and seed
//!   as before, face/layer 0 of a layered texture resolves into its own rows,
//!   and a whole-depth clear skips the seed;
//! - `#[ignore = "needs gl-ds-layers-core"]` tests hold the design (the item
//!   gl-ds-layers-core removes the ignores): draw face 0 then face 1 (array
//!   layers 0 then 2) and face 0 gets the back end's depth, the new image is
//!   seeded from its own rows, a later read sees each image's own depth, and
//!   a whole-plane clear on the switch draw hands no seed. Run them with
//!   `-- --ignored`; they fail today.
//!
//! The stand-in back end draws nothing. Its `depth_resolve` answers a ramp
//! that depends on the object id and on how many resolves it has answered
//! (so two resolves of one plane never write equal depth), and its draw
//! records `depth_id` and `depth_texels` in order with the resolves. The
//! expected values come from the model below, written apart from the code.
//! Every draw is a colour clear of an FBO that also has a colour
//! renderbuffer (a clear-only kick at glFinish), so no shader is needed.
//! A CPU read of a layered texture is a one-texel glTexSubImage of the last
//! image with the value already there: it settles (resolves) the texture and
//! changes nothing the tests look at.
//!
//! Run: `cargo test --features host-test,gpu-powervr --test es3_depth_image_switch`
//! (a GPU feature is needed to build the crate; add `-- --ignored` for the
//! design tests).
//!
//! Not in a `report-es2` build: cube/array depth and glBlitFramebuffer are
//! ES 3.0 only.

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
use yakogl::gl::state::{glClear, glDisable, glFinish, glGetError, glPixelStorei};
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D, glTexSubImage2D, glTexSubImage3D,
};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host, and a back end that keeps some depth -------- */

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

/// What the stand-in saw, in order.
#[derive(Clone, Debug, PartialEq)]
enum Ev {
    /// A draw: its depth object and the depth texels it carried (`ds_w` x
    /// `ds_h`, row by row), if any.
    Draw(u32, Option<Vec<f32>>),
    /// A `depth_resolve` of the object, the `n`th answered (1-based).
    Resolve(u32, u32),
}

static EVENTS: Mutex<Vec<Ev>> = Mutex::new(Vec::new());
/// The objects the stand-in keeps depth for.
static HELD: Mutex<Vec<u32>> = Mutex::new(Vec::new());
/// The resolves answered so far.
static ANSWERED: Mutex<u32> = Mutex::new(0);

fn events() -> MutexGuard<'static, Vec<Ev>> {
    EVENTS.lock().unwrap_or_else(|e| e.into_inner())
}

/// The model of the depth the stand-in keeps for object `id` at (x, y) when
/// it answers its `n`th resolve: exact in f32 (multiples of 2^-16 below 8).
fn held_depth(id: u32, n: u32, x: u32, y: u32) -> f32 {
    ((id & 0x7) as f32 * 4096.0 + n as f32 * 8192.0 + y as f32 * 64.0 + x as f32) / 65536.0
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core hands over a live draw.
    let d = unsafe { &*d };
    let texels = (!d.depth_texels.is_null()).then(|| {
        assert!(d.depth_texels_pitch >= d.ds_w, "a pitch narrower than the depth width");
        let rows = (0..d.ds_h).flat_map(|y| (0..d.ds_w).map(move |x| (y * d.depth_texels_pitch + x) as usize));
        // SAFETY: the contract: `ds_h` rows of `depth_texels_pitch` floats.
        rows.map(|i| unsafe { d.depth_texels.add(i).read() }).collect()
    });
    *KICKS.lock().unwrap_or_else(|e| e.into_inner()) += 1;
    events().push(Ev::Draw(d.depth_id, texels));
    0
}

/// Every draw is a kick of its own: the id names the draw made last (1 before
/// any), so a plane resolved for one kick is stale again after the next.
static KICKS: Mutex<u32> = Mutex::new(0);

unsafe extern "C" fn b_kick_id() -> u32 {
    (*KICKS.lock().unwrap_or_else(|e| e.into_inner())).max(1)
}

unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    if !HELD.lock().unwrap_or_else(|e| e.into_inner()).contains(&id) {
        return 1;
    }
    let n = {
        let mut a = ANSWERED.lock().unwrap_or_else(|e| e.into_inner());
        *a += 1;
        *a
    };
    events().push(Ev::Resolve(id, n));
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the caller's contract: `h` rows of `pitch` floats.
            unsafe { dst.add((y * pitch + x) as usize).write(held_depth(id, n, x, y)) };
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
    name: c"es3-depth-image-switch".as_ptr() as *const c_char,
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
    kick_id: Some(b_kick_id),
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
    depth_resolve: Some(b_depth_resolve),
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
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
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glDisable(GL_SCISSOR_TEST);
        while glGetError() != GL_NO_ERROR {}
    }
    HELD.lock().unwrap_or_else(|e| e.into_inner()).clear();
    *ANSWERED.lock().unwrap_or_else(|e| e.into_inner()) = 0;
    *KICKS.lock().unwrap_or_else(|e| e.into_inner()) = 0;
    events().clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- objects ----------------------------------------------------------- */

const SIDE: u32 = 4;
const D32F: (GLenum, GLenum, GLenum) = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

#[derive(Clone, Copy, Debug)]
enum Layered {
    Cube,
    Array,
}

impl Layered {
    fn images(self) -> u32 {
        match self {
            Layered::Cube => 6,
            Layered::Array => 3,
        }
    }
}

/// The depth the test puts into image `k` at (x, y): a ramp per image from
/// 0.5 up, exact in f32. Every `held_depth` is below 0.5 for the ids and
/// resolve counts used here, so no image is mistaken for a resolved one.
fn image_depth(k: u32, x: u32, y: u32) -> f32 {
    (k * 256 + y * 16 + x) as f32 / 65536.0 + 0.5
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

/// The R32F texel (x, y) of image `layer` of level 0, as stored: faces or
/// layers stacked in the level, image k's row y at row k * height + y.
fn texel(tex: GLuint, layer: u32, x: u32, y: u32) -> f32 {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    unsafe { texel_ptr(tex, layer, x, y).read() }
}

fn rows(f: impl Fn(u32, u32) -> f32) -> Vec<f32> {
    (0..SIDE).flat_map(|y| (0..SIDE).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// Image `layer` of `tex` as the CPU holds it.
fn image(tex: GLuint, layer: u32) -> Vec<f32> {
    rows(|x, y| texel(tex, layer, x, y))
}

/// What an image's rows hold when the plane's `n`th resolve is in them.
fn resolved_image(id: u32, n: u32) -> Vec<f32> {
    rows(|x, y| held_depth(id, n, x, y))
}

/// What image `k`'s rows hold as the test made them.
fn seeded_image(k: u32) -> Vec<f32> {
    rows(|x, y| image_depth(k, x, y))
}

/// A cube map or 3-layer array of 4x4 depth, image k holding `image_depth(k)`.
fn layered_texture(kind: Layered) -> GLuint {
    let mut tex = 0;
    let s = SIDE as i32;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                for face in FACES {
                    glTexImage2D(face, 0, D32F.0 as GLint, s, s, 0, D32F.1, D32F.2, core::ptr::null());
                }
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                let n = kind.images() as i32;
                glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, D32F.0 as GLint, s, s, n, 0, D32F.1, D32F.2, core::ptr::null());
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "a {kind:?} depth texture");
    for k in 0..kind.images() {
        for y in 0..SIDE {
            for x in 0..SIDE {
                // SAFETY: as in `texel`.
                unsafe { texel_ptr(tex, k, x, y).write(image_depth(k, x, y)) };
            }
        }
    }
    tex
}

/// A `SIDE` x `SIDE` 2D depth texture holding `image_depth(0)`.
fn depth_texture_2d() -> GLuint {
    let mut tex = 0;
    // SAFETY: room for one name; no pixels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        let s = SIDE as i32;
        glTexImage2D(GL_TEXTURE_2D, 0, D32F.0 as GLint, s, s, 0, D32F.1, D32F.2, core::ptr::null());
    }
    assert_eq!(err(), GL_NO_ERROR);
    for y in 0..SIDE {
        for x in 0..SIDE {
            // SAFETY: as in `texel`.
            unsafe { texel_ptr(tex, 0, x, y).write(image_depth(0, x, y)) };
        }
    }
    tex
}

/// The ds id the back ends keep depth texture `tex` under.
fn ds_id(tex: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    context::ds_object_id(c.texture_object(slot).expect("a texture object").serial, true)
}

/// Make the stand-in keep depth for `tex`; its ds id.
fn hold(tex: GLuint) -> u32 {
    let id = ds_id(tex);
    HELD.lock().unwrap_or_else(|e| e.into_inner()).push(id);
    id
}

/// A framebuffer with image `k` of `tex` at the depth attachment (the 2D
/// texture itself for `None`) and a 4x4 colour renderbuffer; left bound.
/// (framebuffer, renderbuffer).
fn target(tex: GLuint, kind: Option<Layered>, k: u32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            None => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0),
            Some(Layered::Cube) => {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, FACES[k as usize], tex, 0)
            }
            Some(Layered::Array) => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, tex, 0, k as GLint),
        }
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 4, 4);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    (fb, rb)
}

/// A colour clear (`depth_too`: and a clear of the whole depth plane) of the
/// bound framebuffer, sent to the back end.
fn draw(depth_too: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        glClear(GL_COLOR_BUFFER_BIT | if depth_too { GL_DEPTH_BUFFER_BIT } else { 0 });
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// A CPU read of layered texture `tex`: a one-texel glTexSubImage of image
/// `last` with the value it holds, which settles the texture first. The
/// settle resolves the plane into the recorded image and the write then puts
/// back the value read BEFORE the resolve, so `last` must not be the image
/// the plane holds (the last one drawn): it would overwrite the resolve with
/// the stale stand-in value (texture.rs `settle_texture`: `resolve_depth`,
/// then the sub-image write over the rows).
fn settle_at(tex: GLuint, kind: Layered, last: u32) {
    let v = [texel(tex, last, 0, 0)];
    // SAFETY: one float of pixels, a 1x1 sub-image inside the level.
    unsafe {
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                glTexSubImage2D(FACES[last as usize], 0, 0, 0, 1, 1, D32F.1, D32F.2, v.as_ptr() as *const c_void);
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                let z = last as GLint;
                glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, z, 1, 1, 1, D32F.1, D32F.2, v.as_ptr() as *const c_void);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// `settle_at` the last image (the tests that draw only into image 0).
fn settle(tex: GLuint, kind: Layered) {
    settle_at(tex, kind, kind.images() - 1);
}

fn take() -> Vec<Ev> {
    std::mem::take(&mut *events())
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

/* ---- what must not change: 2D, image 0, whole-depth clears --------------- */

/// A 2D depth texture drawn into and read by a depth blit: the rows hold
/// what the back end resolved, all of them.
#[test]
fn a_2d_depth_texture_resolves_into_its_rows() {
    let _g = context();
    let tex = depth_texture_2d();
    let id = hold(tex);
    let (fb, rb) = target(tex, None, 0);
    draw(false);
    assert_eq!(take(), [Ev::Draw(id, None)]);
    // The read: a depth blit from the texture into a second 2D texture.
    let copy = depth_texture_2d();
    let (cfb, crb) = target(copy, None, 0);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, cfb);
        let s = SIDE as i32;
        glBlitFramebuffer(0, 0, s, s, 0, 0, s, s, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(take(), [Ev::Resolve(id, 1)]);
    assert_eq!(image(tex, 0), resolved_image(id, 1), "the texture's rows");
    assert_eq!(image(copy, 0), resolved_image(id, 1), "the blit read the resolved rows");
    release(&[fb, cfb], &[tex, copy], &[rb, crb]);
}

/// Texels the CPU wrote reach the back end with the first draw after them,
/// the whole level; the draw after that carries none.
#[test]
fn a_2d_depth_texture_seeds_from_its_rows() {
    let _g = context();
    let tex = depth_texture_2d();
    let id = hold(tex);
    let (fb, rb) = target(tex, None, 0);
    draw(false);
    let one = [0.25f32];
    // SAFETY: one float, a 1x1 sub-image inside the level.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    // The upload settled the texture first: the plane came into the rows.
    let mut want = resolved_image(id, 1);
    want[(SIDE + 1) as usize] = 0.25;
    assert_eq!(image(tex, 0), want);
    take();
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    draw(false);
    draw(false);
    assert_eq!(take(), [Ev::Draw(id, Some(want)), Ev::Draw(id, None)]);
    release(&[fb], &[tex], &[rb]);
}

/// A whole-depth clear replaces what the rows held: the clear draw carries
/// no seed, and the draw after it none either.
#[test]
fn a_whole_depth_clear_skips_the_seed() {
    let _g = context();
    let tex = depth_texture_2d();
    let id = hold(tex);
    let (fb, rb) = target(tex, None, 0);
    draw(false);
    // The CPU wrote the rows (a sub-image), so a draw would seed them.
    let one = [0.25f32];
    // SAFETY: one float, a 1x1 sub-image inside the level.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    take();
    draw(true);
    draw(false);
    assert_eq!(take(), [Ev::Draw(id, None), Ev::Draw(id, None)]);
    release(&[fb], &[tex], &[rb]);
}

/// Image 0 of a cube or array is the one image today's plane lands in: a draw
/// into it, then a CPU read, writes the plane into image 0's rows and leaves
/// every other image's rows alone.
#[test]
fn image_0_of_a_layered_texture_resolves_into_its_own_rows() {
    for kind in [Layered::Cube, Layered::Array] {
        let _g = context();
        let tex = layered_texture(kind);
        let id = hold(tex);
        let (fb, rb) = target(tex, Some(kind), 0);
        draw(false);
        assert_eq!(take(), [Ev::Draw(id, None)], "{kind:?}");
        settle(tex, kind);
        assert_eq!(take(), [Ev::Resolve(id, 1)], "{kind:?}");
        assert_eq!(image(tex, 0), resolved_image(id, 1), "{kind:?}: image 0");
        for k in 1..kind.images() {
            assert_eq!(image(tex, k), seeded_image(k), "{kind:?}: image {k} untouched");
        }
        release(&[fb], &[tex], &[rb]);
    }
}

/// A whole-depth clear of image 0 of a layered texture hands no seed, as in 2D.
#[test]
fn a_whole_depth_clear_of_image_0_skips_the_seed() {
    for kind in [Layered::Cube, Layered::Array] {
        let _g = context();
        let tex = layered_texture(kind);
        let id = hold(tex);
        let (fb, rb) = target(tex, Some(kind), 0);
        draw(true);
        assert_eq!(take(), [Ev::Draw(id, None)], "{kind:?}");
        release(&[fb], &[tex], &[rb]);
    }
}

/* ---- the design: the plane follows the image drawn ----------------------- */

/// Draw image `a`, then image `b` (`clear` makes the switch draw clear the
/// whole depth plane), then read the texture, and check design (b):
/// - the draw into `a` carries no seed (nothing written yet);
/// - the switch to `b` resolves the plane into `a`'s rows BEFORE the draw
///   into `b` reaches the back end (the old frame is waited for, then the
///   new image's texels go over), and the draw is seeded from `b`'s rows
///   (nothing, if it clears the whole plane);
/// - a later read resolves into `b`'s rows and leaves `a`'s as the first
///   resolve wrote them, and the other images' rows stay as they were.
fn switch_case(kind: Layered, a: u32, b: u32, clear: bool) {
    let _g = context();
    let tex = layered_texture(kind);
    let id = hold(tex);
    let (fa, ra) = target(tex, Some(kind), a);
    draw(false);
    let (fb, rb) = target(tex, Some(kind), b);
    draw(clear);
    let seed = (!clear).then(|| seeded_image(b));
    assert_eq!(
        take(),
        [Ev::Draw(id, None), Ev::Resolve(id, 1), Ev::Draw(id, seed)],
        "{kind:?} {a} -> {b} (clear {clear}): the old image resolves before the new draw"
    );
    assert_eq!(image(tex, a), resolved_image(id, 1), "{kind:?}: image {a} took the plane");
    assert_eq!(image(tex, b), seeded_image(b), "{kind:?}: image {b} not yet read");
    // Read through an image outside {a, b}: a write into b would put the
    // stale seed back over the plane the read resolved into b.
    let other = (0..kind.images()).rev().find(|k| *k != a && *k != b).unwrap();
    settle_at(tex, kind, other);
    assert_eq!(take(), [Ev::Resolve(id, 2)], "{kind:?}: the read");
    assert_eq!(image(tex, a), resolved_image(id, 1), "{kind:?}: image {a} keeps its depth");
    assert_eq!(image(tex, b), resolved_image(id, 2), "{kind:?}: image {b} took the plane");
    for k in (0..kind.images()).filter(|k| *k != a && *k != b) {
        assert_eq!(image(tex, k), seeded_image(k), "{kind:?}: image {k} untouched");
    }
    release(&[fa, fb], &[tex], &[ra, rb]);
}

/// Cube face +X drawn, then -X: +X's rows get the back end's depth, -X's
/// seed comes from -X's rows, and later each face reads its own depth.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn drawing_face_1_after_face_0_resolves_face_0_and_seeds_face_1() {
    switch_case(Layered::Cube, 0, 1, false);
}

/// The same for array layers 0 and 2 (layer 1 between them is not touched).
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn drawing_layer_2_after_layer_0_resolves_layer_0_and_seeds_layer_2() {
    switch_case(Layered::Array, 0, 2, false);
}

/// A switch draw that clears the whole plane resolves the old image but
/// hands no seed (the new image's rows are overwritten anyway).
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn a_whole_plane_clear_on_the_switch_draw_hands_no_seed() {
    switch_case(Layered::Cube, 0, 1, true);
    switch_case(Layered::Array, 0, 2, true);
}

/// Switching between faces other than 0 (+Y, then -Z): the plane goes to the
/// face drawn, never to face 0.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn a_switch_between_faces_other_than_0_leaves_face_0_alone() {
    switch_case(Layered::Cube, 2, 5, false);
}

/// Switching back: face 0, face 1, face 0 again. The second switch resolves
/// the plane into face 1's rows and seeds face 0 from face 0's rows, which
/// hold the first resolve.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn switching_back_seeds_from_the_rows_the_first_resolve_wrote() {
    let kind = Layered::Cube;
    let _g = context();
    let tex = layered_texture(kind);
    let id = hold(tex);
    let (f0, r0) = target(tex, Some(kind), 0);
    draw(false);
    let (f1, r1) = target(tex, Some(kind), 1);
    draw(false);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f0) };
    draw(false);
    assert_eq!(
        take(),
        [
            Ev::Draw(id, None),
            Ev::Resolve(id, 1),
            Ev::Draw(id, Some(seeded_image(1))),
            Ev::Resolve(id, 2),
            Ev::Draw(id, Some(resolved_image(id, 1))),
        ]
    );
    assert_eq!(image(tex, 1), resolved_image(id, 2));
    release(&[f0, f1], &[tex], &[r0, r1]);
}
