// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer between depth/stencil renderbuffers through the GL API
//! over the GC7000UL (Vivante) host back end: the 16 `fbo.blit.depth_stencil`
//! shapes of dEQP-GLES3 (tests/es3_blit_ds_v3d.rs's shapes), item
//! gc-blit-ds-test.
//!
//! The Vivante table has `depth_resolve` (`gc_depth_resolve`) but
//! `ds_write: None` and `stencil_resolve: None` (src/vivante/backend.rs:4013-4014).
//! So src/gl/blit.rs refuses every blit that writes a depth plane
//! (REFUSED_DEPTH_DST, src/gl/blit.rs:875) or touches stencil (REFUSED_STENCIL,
//! src/gl/blit.rs:874): GL_INVALID_OPERATION, nothing copied, logged once.
//!
//! Per shape, in a module of its own: `<shape>::is_refused_today` pins that
//! answer: exactly one GL_INVALID_OPERATION, the destination's depth plane
//! (read through `depth_resolve`) still its clear, and the context usable
//! (a clear afterwards lands). The stencil plane cannot be read on this back
//! end (no `stencil_resolve`), so it is not checked in the refusal; the
//! stencil half of the expected-result twin reads it through the table entry
//! the lift adds. `<shape>::reads_back_the_source` is `#[ignore]`d: what a
//! working blit leaves (tests/es3_blit_ds.rs's model): no error, each
//! destination pixel takes the nearest source pixel in the planes the mask
//! names, the others keep their clear.
//!
//! The GPU draws nothing here (every kick is kept, none executed), so the
//! sources are clear-only; a lifted blit is checked against the source plane
//! as `depth_resolve` reads it. The GC7000UL accepts all 16 shapes at
//! renderbuffer storage and framebuffer completeness.
//!
//! HOST-ONLY: API verdicts on the host back end, never pictures and not a
//! board result (tools/cts/README.md 2b).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_blit_ds_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
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

/* ---- the shapes -------------------------------------------------------------- */

/// The source and destination renderbuffers' clears: (depth, stencil).
const SRC_CLEAR: (f32, i32) = (0.25, 0x5a);
const DST_CLEAR: (f32, i32) = (0.75, 0x11);

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// One `fbo.blit.depth_stencil.*` case: the renderbuffer format on both
/// sides, where it attaches, the blit mask, and whether the destination is
/// another size than the source (the `_scale` cases).
#[derive(Clone, Copy)]
struct Shape {
    format: GLenum,
    point: GLenum,
    mask: GLbitfield,
    scale: bool,
}

impl Shape {
    fn has_depth(&self) -> bool {
        self.format != GL_STENCIL_INDEX8
    }
    fn has_stencil(&self) -> bool {
        matches!(self.format, GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 | GL_STENCIL_INDEX8)
    }
    /// 32x32 -> 20x40 when scaled (dEQP's non-integer factors).
    fn sizes(&self) -> ((i32, i32), (i32, i32)) {
        ((32, 32), if self.scale { (20, 40) } else { (32, 32) })
    }
    fn clear_bits(&self) -> GLbitfield {
        let d = if self.has_depth() { DEPTH } else { 0 };
        d | if self.has_stencil() { STENCIL } else { 0 }
    }
}

const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;

/// A `w` x `h` renderbuffer of the shape's format at its attachment point of
/// a new framebuffer, checked complete and bound, then cleared to `clear` in
/// the planes it has: (framebuffer, renderbuffer).
fn ds_fbo(s: &Shape, w: i32, h: i32, clear: (f32, i32)) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, s.format, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage 0x{:x}", s.format);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, s.point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "0x{:x}", s.format);
        glClearDepthf(clear.0);
        glClearStencil(clear.1);
        glClear(s.clear_bits());
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "clearing 0x{:x}", s.format);
    (fb, rb)
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

struct Pair {
    src_fb: GLuint,
    src_rb: GLuint,
    dst_fb: GLuint,
    dst_rb: GLuint,
}

/// Source and destination cleared (`SRC_CLEAR`, `DST_CLEAR`), bound as the
/// blit's read and draw framebuffers.
fn pair(s: &Shape) -> Pair {
    let ((sw, sh), (dw, dh)) = s.sizes();
    let (src_fb, src_rb) = ds_fbo(s, sw, sh, SRC_CLEAR);
    let (dst_fb, dst_rb) = ds_fbo(s, dw, dh, DST_CLEAR);
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src_fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst_fb);
    }
    Pair { src_fb, src_rb, dst_fb, dst_rb }
}

fn blit(s: &Shape) {
    let ((sw, sh), (dw, dh)) = s.sizes();
    // SAFETY: plain GL calls.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, sw, sh, 0, 0, dw, dh, s.mask, GL_NEAREST);
    }
}

fn release(p: &Pair) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(2, [p.src_fb, p.dst_fb].as_ptr());
        glDeleteRenderbuffers(2, [p.src_rb, p.dst_rb].as_ptr());
    }
}

/// The depth plane of renderbuffer `rb` as the GC back end resolves it.
fn read_depth(rb: GLuint, w: i32, h: i32) -> Vec<f32> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { gc_depth_resolve(ds_id(rb), v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the depth of the renderbuffer");
    v
}

/// The stencil plane through the table's `stencil_resolve` (the entry the
/// lift adds; `None` today).
fn read_stencil(rb: GLuint, w: i32, h: i32) -> Vec<u8> {
    let f = yakogl_backend_vivante.stencil_resolve.expect("the GC table has a stencil_resolve entry");
    let mut v = vec![0xffu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { f(ds_id(rb), v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the stencil of the renderbuffer");
    v
}

/// The source pixel nearest-filtering gives destination pixel `o` of `d`
/// over `s` (ES 3.0.6 4.3.3, the whole source onto the whole destination).
fn src_of(o: i32, s: i32, d: i32) -> i32 {
    (((o as f64 + 0.5) * s as f64 / d as f64).floor() as i32).min(s - 1)
}

fn scaled<T: Copy>(src: &[T], s: &Shape) -> Vec<T> {
    let ((sw, sh), (dw, dh)) = s.sizes();
    (0..dh)
        .flat_map(|y| (0..dw).map(move |x| (x, y)))
        .map(|(x, y)| src[(src_of(y, sh, dh) * sw + src_of(x, sw, dw)) as usize])
        .collect()
}

/// Today: the blit is refused whole, with one GL_INVALID_OPERATION.
fn check_refused(s: Shape) {
    on_gc7000(|| {
        let p = pair(&s);
        let (dw, dh) = s.sizes().1;
        blit(&s);
        assert_eq!(err(), GL_INVALID_OPERATION, "the blit is refused");
        assert_eq!(err(), GL_NO_ERROR, "exactly one error");
        if s.has_depth() {
            let got = read_depth(p.dst_rb, dw, dh);
            assert!(got.iter().all(|&d| d == DST_CLEAR.0), "the destination depth is untouched");
        }
        /* usable afterwards: a clear of the destination lands */
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(0.5);
            glClear(s.clear_bits());
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR, "the context is usable after the refusal");
        if s.has_depth() {
            assert!(read_depth(p.dst_rb, dw, dh).iter().all(|&d| d == 0.5), "a later clear lands");
        }
        release(&p);
    });
}

/// What a blit that works leaves: no error, each destination pixel takes the
/// nearest source pixel in the planes the mask names, the others keep their
/// clear.
fn check_readback(s: Shape) {
    on_gc7000(|| {
        let p = pair(&s);
        let ((sw, sh), (dw, dh)) = s.sizes();
        let src_depth = s.has_depth().then(|| read_depth(p.src_rb, sw, sh));
        let src_stencil = s.has_stencil().then(|| read_stencil(p.src_rb, sw, sh));
        blit(&s);
        assert_eq!(err(), GL_NO_ERROR, "the blit works");
        if s.has_depth() {
            let want = if s.mask & DEPTH != 0 {
                scaled(src_depth.as_ref().unwrap(), &s)
            } else {
                vec![DST_CLEAR.0; (dw * dh) as usize]
            };
            for (i, (g, w)) in read_depth(p.dst_rb, dw, dh).iter().zip(&want).enumerate() {
                assert!((g - w).abs() <= 0.5 / 16_777_215.0, "depth pixel {i}: {g} want {w}");
            }
        }
        if s.has_stencil() {
            let want = if s.mask & STENCIL != 0 {
                scaled(src_stencil.as_ref().unwrap(), &s)
            } else {
                vec![DST_CLEAR.1 as u8; (dw * dh) as usize]
            };
            assert_eq!(read_stencil(p.dst_rb, dw, dh), want, "the stencil plane");
        }
        release(&p);
    });
}

macro_rules! shapes {
    ($($name:ident: $format:expr, $point:expr, $mask:expr, $scale:expr;)*) => {$(
        mod $name {
            use super::*;
            const S: Shape = Shape { format: $format, point: $point, mask: $mask, scale: $scale };
            #[test]
            fn is_refused_today() {
                check_refused(S);
            }
            #[test]
            #[ignore = "needs ds_write/stencil_resolve on the GC7000UL (src/vivante/backend.rs:4013)"]
            fn reads_back_the_source() {
                check_readback(S);
            }
        }
    )*};
}

const D: GLenum = GL_DEPTH_ATTACHMENT;
const DS: GLenum = GL_DEPTH_STENCIL_ATTACHMENT;
const DS_MASK: GLbitfield = DEPTH | STENCIL;

shapes! {
    depth_component16_basic: GL_DEPTH_COMPONENT16, D, DEPTH, false;
    depth_component16_scale: GL_DEPTH_COMPONENT16, D, DEPTH, true;
    depth_component24_basic: GL_DEPTH_COMPONENT24, D, DEPTH, false;
    depth_component24_scale: GL_DEPTH_COMPONENT24, D, DEPTH, true;
    depth_component32f_basic: GL_DEPTH_COMPONENT32F, D, DEPTH, false;
    depth_component32f_scale: GL_DEPTH_COMPONENT32F, D, DEPTH, true;
    depth24_stencil8_basic: GL_DEPTH24_STENCIL8, DS, DS_MASK, false;
    depth24_stencil8_scale: GL_DEPTH24_STENCIL8, DS, DS_MASK, true;
    depth24_stencil8_depth_only: GL_DEPTH24_STENCIL8, DS, DEPTH, false;
    depth24_stencil8_stencil_only: GL_DEPTH24_STENCIL8, DS, STENCIL, false;
    depth32f_stencil8_basic: GL_DEPTH32F_STENCIL8, DS, DS_MASK, false;
    depth32f_stencil8_scale: GL_DEPTH32F_STENCIL8, DS, DS_MASK, true;
    depth32f_stencil8_depth_only: GL_DEPTH32F_STENCIL8, DS, DEPTH, false;
    depth32f_stencil8_stencil_only: GL_DEPTH32F_STENCIL8, DS, STENCIL, false;
    stencil_index8_basic: GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, STENCIL, false;
    stencil_index8_scale: GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, STENCIL, true;
}

/// The two table entries the refusals name are absent today; this is the
/// line the lift changes.
#[test]
fn the_table_lacks_ds_write_and_stencil_resolve_today() {
    assert!(yakogl_backend_vivante.ds_write.is_none(), "ds_write (src/vivante/backend.rs:4013)");
    assert!(yakogl_backend_vivante.stencil_resolve.is_none(), "stencil_resolve (src/vivante/backend.rs:4014)");
    assert!(yakogl_backend_vivante.depth_resolve.is_some(), "depth_resolve stays");
}
