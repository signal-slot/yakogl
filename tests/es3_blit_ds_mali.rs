// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer between depth/stencil renderbuffers through the GL API
//! over the Mali (Valhall G610) back end's table, on the host: the 16
//! `fbo.blit.depth_stencil` shapes of dEQP-GLES3, each pinned at what the
//! GL core answers today and, `#[ignore]`d, at what a working blit leaves.
//!
//! HOST-ONLY: an API verdict of the GL core on the Mali's table; no Mali
//! frame runs and no picture is read (tools/cts/README.md 2b).
//!
//! The Mali's table (`yakogl_backend_mali`, src/mali/backend.rs) has
//! `depth_resolve` (`mali_depth_resolve`, docs/notes/mali-depth-only.md) but
//! no `ds_write` and no `stencil_resolve` (its `YakoglBackend` literal).
//! src/gl/blit.rs therefore refuses a blit that writes depth
//! (REFUSED_DEPTH_DST) or touches stencil (REFUSED_STENCIL) between
//! renderbuffers: GL_INVALID_OPERATION, said once in the log, nothing
//! copied. tests/es3_blit_ds_v3d.rs has the same 16 shapes over the V3D,
//! whose table has both entries.
//!
//! The rig is tests/es3_msaa_mali.rs's: the GL core and a stand-in back end
//! whose table has the Mali's `ds_write` and `stencil_resolve` (none; a test
//! checks that against `yakogl_backend_mali`) and a `depth_resolve` that
//! answers planes the test seeds (the real `mali_depth_resolve` needs a Mali
//! frame that the host does not run).
//!
//! Per shape, in a module of its own: `is_refused_today` pins exactly one
//! GL_INVALID_OPERATION, the destination depth unchanged (read through
//! `depth_resolve`) and the context usable (a clear and a colour blit
//! afterwards raise no error). `reads_back_the_source` is `#[ignore]`d: it
//! holds what a working blit leaves (no error; each destination pixel takes
//! the nearest source pixel in the planes the mask names, the others keep
//! their clear; the model of tests/es3_blit_ds.rs). When the Mali gets the
//! entries, that test's rig must read the planes through the Mali's own
//! entries (a Mali OSAL rig) in place of the seeded stand-in.
//!
//! The renderbuffer formats and framebuffer completeness are the GL core's:
//! every shape is complete here, so none is pinned as rejected.
//!
//! Not in a `report-es2` build: glBlitFramebuffer is GL_INVALID_OPERATION
//! there.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-mali,shaderc-online,no-shader-table --test es3_blit_ds_mali`.

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::mali::backend::yakogl_backend_mali;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let p = Box::leak(vec![0u8; size].into_boxed_slice()).as_mut_ptr();
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

/// The depth planes the stand-in keeps, by object id: width, height, depth.
type Planes = BTreeMap<u32, (u32, u32, Vec<f32>)>;
static PLANES: Mutex<Planes> = Mutex::new(Planes::new());

unsafe extern "C" fn b_depth_resolve(id: u32, dst: *mut f32, w: u32, h: u32, pitch: u32) -> i32 {
    let planes = PLANES.lock().unwrap_or_else(|e| e.into_inner());
    let Some((pw, ph, px)) = planes.get(&id) else { return 1 };
    assert_eq!((*pw, *ph), (w, h));
    for y in 0..h {
        for x in 0..w {
            // SAFETY: the caller's contract: `h` rows of `pitch` floats.
            unsafe { dst.add((y * pitch + x) as usize).write(px[(y * w + x) as usize]) };
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
    name: c"es3-blit-ds-mali".as_ptr() as *const c_char,
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
    depth_resolve: Some(b_depth_resolve),
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    /* the Mali's: none (src/mali/backend.rs:3731-3732) */
    ds_write: None,
    stencil_resolve: None,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        while glGetError() != GL_NO_ERROR {}
    }
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).clear();
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The destination's depth and stencil clears, and the source's stencil.
const DST_DEPTH: f32 = 0.75;
const DST_STENCIL: u8 = 0x11;
const SRC_STENCIL: u8 = 0x3c;

/// The source depth at (x, y): differs per texel.
fn src_depth(x: u32, y: u32) -> f32 {
    (x * 7 + y * 13 + 5) as f32 / 1024.0
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
    /// 32x32 -> 20x40 when scaled (a non-integer factor in each axis).
    fn sizes(&self) -> ((i32, i32), (i32, i32)) {
        ((32, 32), if self.scale { (20, 40) } else { (32, 32) })
    }
}

const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;

/// A `w` x `h` renderbuffer of `format` at `point` of a new framebuffer,
/// checked complete and bound: (framebuffer, renderbuffer).
fn ds_fbo(format: GLenum, point: GLenum, w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage 0x{format:x}");
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "0x{format:x}");
    }
    (fb, rb)
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

fn keep_depth(rb: GLuint, w: i32, h: i32, f: impl Fn(u32, u32) -> f32) {
    let px = (0..h as u32).flat_map(|y| (0..w as u32).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
    PLANES.lock().unwrap_or_else(|e| e.into_inner()).insert(ds_id(rb), (w as u32, h as u32, px));
}

/// The depth plane of `rb` through the table's `depth_resolve`.
fn read_depth(rb: GLuint, w: i32, h: i32) -> Vec<f32> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { (BACKEND.depth_resolve.unwrap())(ds_id(rb), v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the depth of the renderbuffer");
    v
}

struct Pair {
    src_fb: GLuint,
    src_rb: GLuint,
    dst_fb: GLuint,
    dst_rb: GLuint,
}

/// The source (depth seeded with `src_depth` where the format has depth) and
/// the destination (seeded with `DST_DEPTH`), bound as read and draw
/// framebuffers of the blit.
fn pair(s: &Shape) -> Pair {
    let ((sw, sh), (dw, dh)) = s.sizes();
    let (src_fb, src_rb) = ds_fbo(s.format, s.point, sw, sh);
    let (dst_fb, dst_rb) = ds_fbo(s.format, s.point, dw, dh);
    if s.has_depth() {
        keep_depth(src_rb, sw, sh, src_depth);
        keep_depth(dst_rb, dw, dh, |_, _| DST_DEPTH);
    }
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src_fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst_fb);
    }
    Pair { src_fb, src_rb, dst_fb, dst_rb }
}

fn blit(s: &Shape) -> GLenum {
    let ((sw, sh), (dw, dh)) = s.sizes();
    // SAFETY: plain GL calls.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, sw, sh, 0, 0, dw, dh, s.mask, GL_NEAREST);
    }
    err()
}

fn release(p: &Pair) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(2, [p.src_fb, p.dst_fb].as_ptr());
        glDeleteRenderbuffers(2, [p.src_rb, p.dst_rb].as_ptr());
    }
}

/// The source pixel nearest-filtering gives destination pixel `o` of `d`
/// over `s` (ES 3.0.6 4.3.3, the whole source onto the whole destination).
fn src_of(o: i32, s: i32, d: i32) -> i32 {
    (((o as f64 + 0.5) * s as f64 / d as f64).floor() as i32).min(s - 1)
}

/// Today: exactly one GL_INVALID_OPERATION, the destination depth as it
/// was, the context usable.
fn check_refused(s: Shape) {
    let _g = rig();
    let p = pair(&s);
    let (_, (dw, dh)) = s.sizes();
    let before = s.has_depth().then(|| read_depth(p.dst_rb, dw, dh));
    assert_eq!(blit(&s), GL_INVALID_OPERATION, "the blit is refused");
    assert_eq!(err(), GL_NO_ERROR, "exactly one error");
    if let Some(before) = before {
        assert_eq!(read_depth(p.dst_rb, dw, dh), before, "the destination depth is unchanged");
    }
    /* usable: a clear and a colour-only blit raise nothing */
    // SAFETY: plain GL calls on live names.
    unsafe {
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        assert_eq!(glGetError(), GL_NO_ERROR, "a clear after the refusal");
        glBlitFramebuffer(0, 0, 4, 4, 0, 0, 4, 4, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(glGetError(), GL_NO_ERROR, "a colour blit after the refusal");
    }
    release(&p);
}

/// What a blit that works leaves: no error; every destination pixel takes
/// the nearest source pixel in each plane the mask names; a plane it does not
/// name keeps its clear.
fn check_readback(s: Shape) {
    let _g = rig();
    let p = pair(&s);
    let ((sw, sh), (dw, dh)) = s.sizes();
    assert_eq!(blit(&s), GL_NO_ERROR);
    if s.has_depth() {
        let got = read_depth(p.dst_rb, dw, dh);
        for y in 0..dh {
            for x in 0..dw {
                let want = if s.mask & DEPTH != 0 {
                    src_depth(src_of(x, sw, dw) as u32, src_of(y, sh, dh) as u32)
                } else {
                    DST_DEPTH
                };
                let g = got[(y * dw + x) as usize];
                assert!((g - want).abs() <= 1.0 / 16_777_215.0, "depth ({x},{y}): {g} want {want}");
            }
        }
    }
    if s.has_stencil() {
        let f = yakogl_backend_mali.stencil_resolve.expect("the Mali table has a stencil_resolve entry");
        let mut v = vec![0xffu8; (dw * dh) as usize];
        // SAFETY: `dh` rows of `dw` bytes.
        let r = unsafe { f(ds_id(p.dst_rb), v.as_mut_ptr(), dw as u32, dh as u32, dw as u32) };
        assert_eq!(r, 0, "the back end holds the stencil of the destination");
        let want = if s.mask & STENCIL != 0 { SRC_STENCIL } else { DST_STENCIL };
        assert!(v.iter().all(|&b| b == want), "the stencil plane is 0x{want:x}");
    }
    release(&p);
}

/// The table is what this file says it is: the Mali's has no `ds_write` and
/// no `stencil_resolve`, and the stand-in agrees with it in both.
#[test]
fn the_mali_table_has_neither_entry() {
    assert!(yakogl_backend_mali.depth_resolve.is_some());
    assert!(yakogl_backend_mali.ds_write.is_none(), "the Mali table has a ds_write");
    assert!(yakogl_backend_mali.stencil_resolve.is_none(), "the Mali table has a stencil_resolve");
    assert_eq!(BACKEND.ds_write.is_some(), yakogl_backend_mali.ds_write.is_some());
    assert_eq!(BACKEND.stencil_resolve.is_some(), yakogl_backend_mali.stencil_resolve.is_some());
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
            #[ignore = "needs ds_write/stencil_resolve on the Mali (src/mali/backend.rs:3731)"]
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
