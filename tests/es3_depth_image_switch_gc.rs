// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth plane of a cube or array depth texture when draws switch faces
//! or layers, over the GC7000UL (Vivante) host back end (plan item
//! switch-test-gc; docs/notes/blit-depth-layers.md, "What this means today"
//! 1 and 2, and design (b)). The scenarios of tests/es3_depth_image_switch.rs
//! (a stand-in back end) over the real GL core and the real Vivante table.
//!
//! The Vivante table has `ds_write: None` and `stencil_resolve: None`, so the
//! plane cannot be written through the table. It is set through GL: a
//! whole-depth `glClear(GL_DEPTH_BUFFER_BIT)` after `glClearDepthf(v)` makes
//! the plane `v` everywhere (a distinct `v` per draw), and a colour-only draw
//! leaves it, or seeds it from the texture's rows when the CPU wrote them
//! (`ds_cache::seed_depth`). The plane is read through the table's
//! `gc_depth_resolve` on the texture's ds id (`ds_cache::resolve_depth`); an
//! image's rows are read from the texture's host memory. A CPU read of a
//! layered texture is a one-texel glTexSubImage of an image other than the
//! one drawn, with the value already there: it settles (resolves) the texture.
//! The GPU draws nothing (every kick is kept, none executed), so every draw
//! is a clear of an FBO with a colour renderbuffer and no shader is needed.
//! Expected values come from the model in this file (`image_depth` and the
//! clear values), not from the code.
//!
//! Two kinds of test:
//! - plain tests pin what must not change: a 2D depth texture resolves into
//!   its rows and seeds the plane from them; image 0 of a layered texture
//!   resolves into its own rows, seeds from them, and leaves the others alone;
//! - `#[ignore = "needs gl-ds-layers-core"]` twins hold design (b): the old
//!   image is resolved into its own rows at the switch, the new image seeds
//!   the plane from its own rows, and a later read sees each image's own
//!   depth. Run them with `-- --ignored`; they fail today.
//!
//! Left out, because this rig cannot tell the cases apart: "a whole-plane
//! clear on the switch draw hands no seed" and "a whole-depth clear skips the
//! seed" of tests/es3_depth_image_switch.rs. A seed lands before the clear in
//! the same draw and the plane after the clear is the clear value either way;
//! the Vivante back end keeps no log of what a draw carried. The stand-in
//! file pins them. The twin here with `clear` covers the other half of that
//! scenario: the old image still resolves into its own rows.
//!
//! HOST-ONLY: API verdicts on the host back end, never pictures and not a
//! board result (tools/cts/README.md 2b).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_depth_image_switch_gc
//! ```
//! (add `-- --ignored` for the design twins).

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
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

/* ---- objects ----------------------------------------------------------- */

const SIDE: u32 = 4;
const D32F: (GLenum, GLenum, GLenum) = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);

/// The depth of the whole-plane clears of the draws, in order; all below 0.5,
/// so no clear is mistaken for an image's seeded rows.
const CLEAR_A: f32 = 0.125;
const CLEAR_B: f32 = 0.25;
const CLEAR_C: f32 = 0.375;

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

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// The depth the test puts into image `k` at (x, y): a ramp per image from
/// 0.5 up, exact in f32.
fn image_depth(k: u32, x: u32, y: u32) -> f32 {
    (k * 256 + y * 16 + x) as f32 / 65536.0 + 0.5
}

fn rows(f: impl Fn(u32, u32) -> f32) -> Vec<f32> {
    (0..SIDE).flat_map(|y| (0..SIDE).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// What image `k`'s rows hold as the test made them.
fn seeded_image(k: u32) -> Vec<f32> {
    rows(|x, y| image_depth(k, x, y))
}

/// A plane or image holding `v` everywhere.
fn uniform(v: f32) -> Vec<f32> {
    rows(|_, _| v)
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

/// Image `layer` of `tex` as the CPU holds it.
fn image(tex: GLuint, layer: u32) -> Vec<f32> {
    rows(|x, y| texel(tex, layer, x, y))
}

fn put_image(tex: GLuint, layer: u32, f: impl Fn(u32, u32) -> f32) {
    for y in 0..SIDE {
        for x in 0..SIDE {
            // SAFETY: as in `texel`.
            unsafe { texel_ptr(tex, layer, x, y).write(f(x, y)) };
        }
    }
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
        put_image(tex, k, |x, y| image_depth(k, x, y));
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
    put_image(tex, 0, |x, y| image_depth(0, x, y));
    tex
}

/// The ds id the back ends keep depth texture `tex` under.
fn ds_id(tex: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    context::ds_object_id(c.texture_object(slot).expect("a texture object").serial, true)
}

/// The depth plane the GC back end keeps for `tex`, as `gc_depth_resolve`
/// answers it.
fn plane(tex: GLuint) -> Vec<f32> {
    let mut v = vec![-1.0f32; (SIDE * SIDE) as usize];
    // SAFETY: `SIDE` rows of `SIDE` floats.
    let r = unsafe { gc_depth_resolve(ds_id(tex), v.as_mut_ptr(), SIDE, SIDE, SIDE) };
    assert_eq!(r, 0, "the back end holds a depth plane of the texture");
    v
}

/// Equal to within one 24-bit depth step.
fn assert_same(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= 0.5 / 16_777_215.0, "{what}: texel {i}: {g} want {w}\n got  {got:?}\n want {want:?}");
    }
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

/// A colour clear of the bound framebuffer (`depth`: and a clear of the whole
/// depth plane to that value), kicked and finished.
fn draw(depth: Option<f32>) {
    // SAFETY: plain GL calls.
    unsafe {
        if let Some(d) = depth {
            glClearDepthf(d);
        }
        glClear(GL_COLOR_BUFFER_BIT | if depth.is_some() { GL_DEPTH_BUFFER_BIT } else { 0 });
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// A CPU read of layered texture `tex`: a one-texel glTexSubImage of image
/// `last` with the value it holds, which settles the texture first. `last`
/// must not be the image the plane holds, or the write puts the stale value
/// back over the resolve (texture.rs `settle_texture`).
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

fn release(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names these tests made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/* ---- what must not change: 2D and image 0 ------------------------------- */

/// A 2D depth texture drawn into and then written by the CPU: the rows hold
/// the plane the back end kept (the whole-depth clear), but for the texel
/// written.
#[test]
fn a_2d_depth_texture_resolves_into_its_rows() {
    on_gc7000(|| {
        let tex = depth_texture_2d();
        let (fb, rb) = target(tex, None, 0);
        draw(Some(CLEAR_A));
        assert_same(&plane(tex), &uniform(CLEAR_A), "the plane");
        let one = [0.25f32];
        // SAFETY: one float, a 1x1 sub-image inside the level.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
        }
        assert_eq!(err(), GL_NO_ERROR);
        let mut want = uniform(CLEAR_A);
        want[(SIDE + 1) as usize] = 0.25;
        assert_same(&image(tex, 0), &want, "the texture's rows");
        release(&[fb], &[tex], &[rb]);
    });
}

/// A texel the CPU wrote reaches the plane with the first draw after it, the
/// whole level; later draws do not seed again.
#[test]
fn a_2d_depth_texture_seeds_from_its_rows() {
    on_gc7000(|| {
        let tex = depth_texture_2d();
        let (fb, rb) = target(tex, None, 0);
        draw(Some(CLEAR_A));
        let one = [0.25f32];
        // SAFETY: one float, a 1x1 sub-image inside the level.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
        }
        assert_eq!(err(), GL_NO_ERROR);
        let mut want = uniform(CLEAR_A);
        want[(SIDE + 1) as usize] = 0.25;
        draw(None);
        assert_same(&plane(tex), &want, "the CPU's texel reached the plane");
        draw(Some(CLEAR_B));
        draw(None);
        assert_same(&plane(tex), &uniform(CLEAR_B), "no second seed over the clear");
        release(&[fb], &[tex], &[rb]);
    });
}

/// Image 0 of a cube or array is the one image today's plane lands in: a draw
/// into it, then a CPU read, writes the plane into image 0's rows and leaves
/// every other image's rows alone.
#[test]
fn image_0_of_a_layered_texture_resolves_into_its_own_rows() {
    for kind in [Layered::Cube, Layered::Array] {
        on_gc7000(|| {
            let tex = layered_texture(kind);
            let (fb, rb) = target(tex, Some(kind), 0);
            draw(Some(CLEAR_A));
            assert_same(&plane(tex), &uniform(CLEAR_A), &format!("{kind:?}: the plane"));
            settle_at(tex, kind, kind.images() - 1);
            assert_same(&image(tex, 0), &uniform(CLEAR_A), &format!("{kind:?}: image 0"));
            for k in 1..kind.images() {
                assert_same(&image(tex, k), &seeded_image(k), &format!("{kind:?}: image {k} untouched"));
            }
            release(&[fb], &[tex], &[rb]);
        });
    }
}

/// A texel the CPU wrote into image 0 of a layered texture reaches the plane
/// with the first draw after it, as in 2D.
#[test]
fn image_0_of_a_layered_texture_seeds_from_its_own_rows() {
    for kind in [Layered::Cube, Layered::Array] {
        on_gc7000(|| {
            let tex = layered_texture(kind);
            let (fb, rb) = target(tex, Some(kind), 0);
            draw(Some(CLEAR_A));
            let one = [0.25f32];
            // SAFETY: one float, a 1x1 sub-image of image 0 inside the level.
            unsafe {
                match kind {
                    Layered::Cube => {
                        glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                        glTexSubImage2D(FACES[0], 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
                    }
                    Layered::Array => {
                        glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                        glTexSubImage3D(
                            GL_TEXTURE_2D_ARRAY,
                            0,
                            1,
                            1,
                            0,
                            1,
                            1,
                            1,
                            D32F.1,
                            D32F.2,
                            one.as_ptr() as *const c_void,
                        );
                    }
                }
            }
            assert_eq!(err(), GL_NO_ERROR);
            let mut want = uniform(CLEAR_A);
            want[(SIDE + 1) as usize] = 0.25;
            draw(None);
            assert_same(&plane(tex), &want, &format!("{kind:?}: the plane"));
            release(&[fb], &[tex], &[rb]);
        });
    }
}

/* ---- the design: the plane follows the image drawn ----------------------- */

/// Draw image `a` (a whole clear to `CLEAR_A`), then image `b` (`clear`: the
/// switch draw clears the whole plane to `CLEAR_B`; else it is colour only),
/// then draw `b` again (`CLEAR_C`) and read the texture, and check design (b):
/// - the switch resolves the plane into `a`'s rows first: `a` holds
///   `CLEAR_A` everywhere, `b`'s rows are not touched yet;
/// - the plane after the switch draw is `b`'s own rows (seeded), or the
///   clear's value;
/// - a later read resolves into `b`'s rows and leaves `a`'s as the first
///   resolve wrote them; the other images' rows stay as they were.
fn switch_case(kind: Layered, a: u32, b: u32, clear: bool) {
    on_gc7000(|| {
        let what = format!("{kind:?} {a} -> {b} (clear {clear})");
        let tex = layered_texture(kind);
        let (fa, ra) = target(tex, Some(kind), a);
        draw(Some(CLEAR_A));
        assert_same(&plane(tex), &uniform(CLEAR_A), &format!("{what}: the plane of the first draw"));
        let (fb, rb) = target(tex, Some(kind), b);
        draw(clear.then_some(CLEAR_B));
        assert_same(&image(tex, a), &uniform(CLEAR_A), &format!("{what}: image {a} took the plane"));
        assert_same(&image(tex, b), &seeded_image(b), &format!("{what}: image {b} not yet read"));
        let want = if clear { uniform(CLEAR_B) } else { seeded_image(b) };
        assert_same(&plane(tex), &want, &format!("{what}: the plane after the switch draw"));
        draw(Some(CLEAR_C));
        // Read through an image outside {a, b}: a write into b would put the
        // stale texels back over the plane the read resolved into b.
        let other = (0..kind.images()).rev().find(|k| *k != a && *k != b).unwrap();
        settle_at(tex, kind, other);
        assert_same(&image(tex, a), &uniform(CLEAR_A), &format!("{what}: image {a} keeps its depth"));
        assert_same(&image(tex, b), &uniform(CLEAR_C), &format!("{what}: image {b} took the plane"));
        for k in (0..kind.images()).filter(|k| *k != a && *k != b) {
            assert_same(&image(tex, k), &seeded_image(k), &format!("{what}: image {k} untouched"));
        }
        release(&[fa, fb], &[tex], &[ra, rb]);
    });
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

/// A switch draw that clears the whole plane still resolves the old image
/// first (the seed itself is not observable here, see the header).
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn a_whole_plane_clear_on_the_switch_draw_resolves_the_old_image() {
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
    on_gc7000(|| {
        let kind = Layered::Cube;
        let tex = layered_texture(kind);
        let (f0, r0) = target(tex, Some(kind), 0);
        draw(Some(CLEAR_A));
        let (f1, r1) = target(tex, Some(kind), 1);
        draw(Some(CLEAR_B));
        // SAFETY: plain GL call.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f0) };
        draw(None);
        assert_same(&image(tex, 1), &uniform(CLEAR_B), "face 1 took the plane at the second switch");
        assert_same(&plane(tex), &uniform(CLEAR_A), "face 0 is seeded from the rows the first resolve wrote");
        release(&[f0, f1], &[tex], &[r0, r1]);
    });
}
