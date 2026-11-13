// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the GC7000 whole-target clears of 11fb5f88
//! (vivante: whole-target depth/stencil clears take the RS fill of rs_clear.rs;
//! emit_pe_state takes DISABLE_ZS and OVERWRITE from pe_zs.rs and
//! pe_overwrite.rs) and de5860b5 (a whole colour-only clear of a 32-bit linear
//! target is the resolve engine's fill). The i.MX8MP whole dEQP-GLES2 tree
//! (0fbe61a3) predates both, and nearly every dEQP-GLES2 case clears; its
//! stage-1 promise is 0 Fail and only the 34 NotSupported of
//! tools/cts/es2-accepted.txt. tests/gc_rs_clear_stream.rs pins the words of the
//! RS fill in a build that has the ES 3.0 surface; this file drives the clears
//! an ES 2.0 context can make (a `report-es2` build: one colour attachment, no
//! GL_DEPTH_STENCIL_ATTACHMENT, the OES renderbuffer names) through the real GL
//! core and the real Vivante back end, and asserts for each: GL_NO_ERROR, no
//! refused draw (`gc_refused_draws()` unchanged), and the path the commits name:
//! the RS fill (an RS_KICKER per plane filled, no DRAW_INSTANCED) for a whole,
//! unmasked clear, the clear quad (one DRAW_INSTANCED, no RS_KICKER) elsewhere.
//!
//! The stand-in OSAL keeps every submitted stream and executes none, so no
//! pixel is read back: the cleared values are checked where they are in the
//! stream, the fill words (the packed A8R8G8B8 colour in the four fill words,
//! `lrint(z * 0xffffff) << 8 | stencil` for D24S8). For the quad path the values
//! are shader constants, so only the path is asserted.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_more_clears
//! cargo clippy --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_more_clears -- -D warnings
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glFramebufferRenderbuffer, glFramebufferTexture2D,
    glGenFramebuffers, glGenRenderbuffers, glRenderbufferStorage,
};
use yakogl::gl::state::{
    glClear, glClearColor, glClearDepthf, glClearStencil, glColorMask, glDepthMask, glDisable, glEnable, glFinish,
    glGetError, glScissor, glStencilMask, glViewport,
};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_debug_rs_clear, gc_init, gc_refused_draws, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are
/// process-global.
static SERIAL: Mutex<()> = Mutex::new(());

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
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not freed: a static of the library that still names the
         * block must find memory (tests/gc_npot_repeat.rs) */
        match p.allocs.remove(&m.gpu) {
            Some(a) => core::mem::forget(a.mem),
            None if !m.impl_.is_null() => {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu))
            }
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The stream is kept and its fence raised; every kick completes at once.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let words = words.to_vec();
        p.kicks.push(words);
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

/* ---- GL plumbing ------------------------------------------------------------------- */

/// Run `case` on the real Vivante back end over the stand-in OSAL and a
/// fresh 64x64 context, the RS clear on (its default).
fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: SERIAL is held; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        gc_debug_rs_clear(true);
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_debug_rs_clear(true);
        gc_shutdown();
    }
    let faults = with_platform(|p| core::mem::take(&mut p.faults));
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

/* ---- reading the stream ------------------------------------------------------------ */

/// Every register a stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    out.push((base + 4 * k as u32, words[i + 1 + k]));
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => i += 4,
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

fn kickers(kicks: &[Vec<u32>]) -> usize {
    kicks.iter().map(|k| writes(k).iter().filter(|(r, _)| *r == GC_RS_KICKER).count()).sum()
}

fn draws(kicks: &[Vec<u32>]) -> usize {
    let mut n = 0;
    for words in kicks {
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => i += 1 + ((w >> 16) & 0x3ff) as usize,
                GC_OP_DRAW_INSTANCED => {
                    n += 1;
                    i += 4;
                }
                GC_OP_STALL => i += 2,
                _ => i += 1,
            }
        }
    }
    n
}

/// Each RS fill in the streams: its RS_CLEAR_CONTROL bits (without the
/// MODE_ENABLED1 bit) and the four fill words, in order.
fn fills(kicks: &[Vec<u32>]) -> Vec<(u32, [u32; 4])> {
    let mut out = Vec::new();
    for k in kicks {
        let w = writes(k);
        for (i, &(r, v)) in w.iter().enumerate() {
            if r == GC_RS_CLEAR_CONTROL && v != 0 {
                let f: Vec<u32> = w[i + 1..i + 5].iter().map(|&(_, v)| v).collect();
                out.push((v & 0xffff, [f[0], f[1], f[2], f[3]]));
            }
        }
    }
    out
}

/* ---- the ES 2.0 framebuffers ------------------------------------------------------- */

#[derive(Clone, Copy, Debug)]
enum Colour {
    /// The default framebuffer (the context's 64x64 surface).
    Window,
    /// A texture of GL_RGBA / GL_UNSIGNED_BYTE at colour 0.
    TexRgba8,
    /// A GL_RGBA8_OES renderbuffer at colour 0.
    RbRgba8,
    /// A GL_RGB / GL_UNSIGNED_SHORT_5_6_5 texture.
    Tex565,
    /// A GL_RGB565 renderbuffer.
    Rb565,
    /// No colour attachment.
    Nothing,
}

#[derive(Clone, Copy, Debug)]
enum Ds {
    /// Nothing (the default framebuffer keeps its own).
    None,
    Depth16,
    Stencil8,
    /// GL_DEPTH24_STENCIL8_OES attached to DEPTH and to STENCIL (ES 2.0's
    /// packed depth-stencil: OES_packed_depth_stencil has no
    /// GL_DEPTH_STENCIL_ATTACHMENT).
    Packed,
}

fn renderbuffer(format: GLenum, w: i32, h: i32, points: &[GLenum]) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut rb: GLuint = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        for &p in points {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, p, GL_RENDERBUFFER, rb);
        }
    }
}

fn texture(format: GLenum, ty: GLenum, w: i32, h: i32) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut t: GLuint = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, format as GLint, w, h, 0, format, ty, core::ptr::null());
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
    }
}

/// Bind a framebuffer object `w` x `h` of the given attachments (the default
/// framebuffer for `Window` with `Ds::None`), complete, the viewport on it.
fn target(colour: Colour, ds: Ds, w: i32, h: i32) {
    if matches!((colour, ds), (Colour::Window, Ds::None)) {
        return;
    }
    // SAFETY: plain GL calls.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    match colour {
        Colour::Window | Colour::Nothing => {}
        Colour::TexRgba8 => texture(GL_RGBA, GL_UNSIGNED_BYTE, w, h),
        Colour::Tex565 => texture(GL_RGB, GL_UNSIGNED_SHORT_5_6_5, w, h),
        Colour::RbRgba8 => renderbuffer(GL_RGBA8_OES, w, h, &[GL_COLOR_ATTACHMENT0]),
        Colour::Rb565 => renderbuffer(GL_RGB565, w, h, &[GL_COLOR_ATTACHMENT0]),
    }
    match ds {
        Ds::None => {}
        Ds::Depth16 => renderbuffer(GL_DEPTH_COMPONENT16, w, h, &[GL_DEPTH_ATTACHMENT]),
        Ds::Stencil8 => renderbuffer(GL_STENCIL_INDEX8, w, h, &[GL_STENCIL_ATTACHMENT]),
        Ds::Packed => renderbuffer(GL_DEPTH24_STENCIL8_OES, w, h, &[GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT]),
    }
    // SAFETY: plain GL calls.
    unsafe {
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{colour:?} {ds:?}");
        glViewport(0, 0, w, h);
    }
    no_error("the framebuffer object");
}

/// `glClear(mask)`, glFinish: the streams it submitted, after GL_NO_ERROR and
/// with no draw refused.
fn clear(mask: GLbitfield) -> Vec<Vec<u32>> {
    let refused = gc_refused_draws();
    with_platform(|p| p.kicks.clear());
    // SAFETY: plain GL calls.
    unsafe {
        glClear(mask);
        glFinish();
    }
    no_error("the clear");
    assert_eq!(gc_refused_draws(), refused, "a refused draw");
    with_platform(|p| core::mem::take(&mut p.kicks))
}

const C: GLbitfield = GL_COLOR_BUFFER_BIT;
const D: GLbitfield = GL_DEPTH_BUFFER_BIT;
const S: GLbitfield = GL_STENCIL_BUFFER_BIT;

/// `lrint(z * 0xffffff) << 8 | stencil` (util_pack_z_stencil).
const fn d24s8(z24: u32, stencil: u32) -> u32 {
    z24 << 8 | stencil
}

/// The clear colour set for the cases, as A8R8G8B8.
const COLOUR_WORD: u32 = 0xffff_8000;
/// The clear depth set, 0.5: 0.5 * 0xffffff = 8388607.5, to even.
const Z24: u32 = 0x80_0000;
const STENCIL: u32 = 0x5a;

fn set_clear_values() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(1.0, 0.5, 0.0, 1.0);
        glClearDepthf(0.5);
        glClearStencil(STENCIL as GLint);
    }
}

/// The state a case sets before its clear.
#[derive(Clone, Copy, Debug)]
struct State {
    /// A scissor rectangle smaller than the target.
    scissor: bool,
    /// A scissor rectangle that is the whole target (the whole clear).
    scissor_whole: bool,
    /// glColorMask bits R=1 G=2 B=4 A=8.
    colour_mask: u8,
    depth_mask: bool,
    stencil_mask: u32,
}

impl State {
    const OPEN: State =
        State { scissor: false, scissor_whole: false, colour_mask: 0xf, depth_mask: true, stencil_mask: 0xff };

    fn apply(self, w: i32) {
        let m = self.colour_mask;
        // SAFETY: plain GL calls.
        unsafe {
            if self.scissor {
                glEnable(GL_SCISSOR_TEST);
                glScissor(0, 0, w / 4, w / 4);
            }
            if self.scissor_whole {
                glEnable(GL_SCISSOR_TEST);
                glScissor(0, 0, w, w);
            }
            glColorMask(
                (m & 1 != 0) as GLboolean,
                (m & 2 != 0) as GLboolean,
                (m & 4 != 0) as GLboolean,
                (m & 8 != 0) as GLboolean,
            );
            glDepthMask(self.depth_mask as GLboolean);
            glStencilMask(self.stencil_mask);
        }
    }
}

/// Which planes a target has: (colour, depth, stencil). The default
/// framebuffer carries colour, depth and stencil.
fn planes(colour: Colour, ds: Ds) -> (bool, bool, bool) {
    let has_colour = !matches!(colour, Colour::Nothing);
    let (d, s) = match ds {
        Ds::None => (matches!(colour, Colour::Window), matches!(colour, Colour::Window)),
        Ds::Depth16 => (true, false),
        Ds::Stencil8 => (false, true),
        Ds::Packed => (true, true),
    };
    (has_colour, d, s)
}

/// The planes `mask` clears under `st` (a colour mask of 0, a depth mask off, a
/// stencil write mask of 0 and an attachment the target lacks clear nothing).
fn cleared(colour: Colour, ds: Ds, mask: GLbitfield, st: State) -> (bool, bool, bool) {
    let (hc, hd, hs) = planes(colour, ds);
    (
        mask & C != 0 && hc && st.colour_mask != 0,
        mask & D != 0 && hd && st.depth_mask,
        mask & S != 0 && hs && st.stencil_mask & 0xff != 0,
    )
}

/// What the back end does: Some(n) is the RS fill path with n fills (0: no plane
/// is left, nothing is drawn), None the clear quad.
fn expected(colour: Colour, ds: Ds, aligned: bool, mask: GLbitfield, st: State) -> Option<usize> {
    let (c, d, s) = cleared(colour, ds, mask, st);
    if !(c || d || s) {
        return Some(0);
    }
    let refused = st.scissor || (c && (st.colour_mask != 0xf || !aligned)) || (s && st.stencil_mask & 0xff != 0xff);
    if refused {
        return None;
    }
    Some(c as usize + (d || s) as usize)
}

fn check(colour: Colour, ds: Ds, w: i32, st: State, mask: GLbitfield) {
    let what = format!("{colour:?} {ds:?} {w}x{w} {st:?} mask 0x{mask:x}");
    with_backend(|| {
        target(colour, ds, w, w);
        set_clear_values();
        st.apply(w);
        let k = clear(mask);
        let Some(n) = expected(colour, ds, w % 16 == 0, mask, st) else {
            assert_eq!(draws(&k), 1, "{what}: the clear quad");
            assert_eq!(kickers(&k), 0, "{what}: no RS fill");
            return;
        };
        assert_eq!(draws(&k), 0, "{what}: no clear quad");
        assert_eq!(kickers(&k), n, "{what}: RS fills");
        let f = fills(&k);
        assert_eq!(f.len(), n, "{what}: fills {f:x?}");
        /* the cleared values, where they are: the stream's fill words */
        let (c, d, s) = cleared(colour, ds, mask, st);
        if c {
            let (bits, v) = f[0];
            assert_eq!(bits, 0xffff, "{what}: colour fill bits");
            assert!(v.iter().all(|&x| x == v[0]), "{what}: one colour in all four words");
            match colour {
                Colour::Tex565 | Colour::Rb565 => assert_eq!(v[0] >> 24, 0xff, "{what}: a 565 target has no alpha"),
                _ => assert_eq!(v[0], COLOUR_WORD, "{what}: colour"),
            }
        }
        if d || s {
            let (bits, word) = match (d, s) {
                (true, false) => (0xeeee, d24s8(Z24, 0)),
                /* the depth the GL core sends with a stencil-only clear is 1.0 */
                (false, true) => (0x1111, d24s8(0xff_ffff, STENCIL)),
                _ => (0xffff, d24s8(Z24, STENCIL)),
            };
            assert_eq!(f[c as usize], (bits, [word, 0, word, 0]), "{what}: depth/stencil fill");
        }
    });
}

const TARGETS: [(Colour, Ds); 14] = [
    (Colour::Window, Ds::None),
    (Colour::TexRgba8, Ds::None),
    (Colour::RbRgba8, Ds::None),
    (Colour::Tex565, Ds::None),
    (Colour::Rb565, Ds::None),
    (Colour::TexRgba8, Ds::Packed),
    (Colour::RbRgba8, Ds::Packed),
    (Colour::Rb565, Ds::Packed),
    (Colour::TexRgba8, Ds::Depth16),
    (Colour::Rb565, Ds::Depth16),
    (Colour::TexRgba8, Ds::Stencil8),
    (Colour::Nothing, Ds::Packed),
    (Colour::Nothing, Ds::Depth16),
    (Colour::Nothing, Ds::Stencil8),
];

const MASKS: [GLbitfield; 7] = [C, D, S, C | D, D | S, C | S, C | D | S];

fn all_targets(w: i32, st: State) {
    for (colour, ds) in TARGETS {
        for mask in MASKS {
            check(colour, ds, w, st, mask);
        }
    }
}

/// Whole, unmasked: the RS fill on the default framebuffer and every ES 2.0
/// framebuffer object, with the values in the fill words.
#[test]
fn whole_unmasked_clears_are_the_rs_fill() {
    all_targets(64, State::OPEN);
}

/// A scissor rectangle that is the whole target is the whole clear.
#[test]
fn a_scissor_of_the_whole_target_is_the_rs_fill() {
    all_targets(64, State { scissor_whole: true, ..State::OPEN });
}

/// A scissor rectangle smaller than the target: the clear quad for every plane.
#[test]
fn a_scissored_clear_is_the_quad() {
    all_targets(64, State { scissor: true, ..State::OPEN });
}

/// A colour mask other than all four channels: the colour plane is the quad
/// (the whole clear with it), a depth/stencil-only clear is still the fill; all
/// four channels off clears no colour at all.
#[test]
fn a_colour_mask_is_the_quad_for_the_colour_plane() {
    for colour_mask in [0x7, 0xe, 0x1, 0x0] {
        all_targets(64, State { colour_mask, ..State::OPEN });
    }
}

/// The depth mask and the stencil write mask: a depth mask off clears no depth
/// (glClear obeys glDepthMask); a stencil write mask of 0xff is the fill, 0xfe
/// the quad (11fb5f88), 0 clears no stencil.
#[test]
fn the_depth_and_stencil_masks() {
    for st in [
        State { depth_mask: false, ..State::OPEN },
        State { stencil_mask: 0xfe, ..State::OPEN },
        State { stencil_mask: 0x0f, ..State::OPEN },
        State { stencil_mask: 0, ..State::OPEN },
        State { depth_mask: false, stencil_mask: 0, ..State::OPEN },
    ] {
        all_targets(64, st);
    }
}

/// A colour target that is not 16 pixels a side keeps the colour plane a quad
/// (de5860b5: 16x4 aligned); its depth/stencil plane, the padded surface, stays
/// the fill.
#[test]
fn an_unaligned_target_keeps_its_colour_plane_a_quad() {
    for (colour, ds) in [
        (Colour::TexRgba8, Ds::None),
        (Colour::RbRgba8, Ds::Packed),
        (Colour::Rb565, Ds::Depth16),
        (Colour::Nothing, Ds::Packed),
    ] {
        for mask in MASKS {
            check(colour, ds, 60, State::OPEN, mask);
        }
    }
}

/// The path follows the state of each clear: no state is left behind by a
/// scissored or masked one.
#[test]
fn a_whole_clear_after_a_scissored_or_masked_one_is_the_fill_again() {
    with_backend(|| {
        target(Colour::TexRgba8, Ds::Packed, 64, 64);
        set_clear_values();
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(8, 8, 16, 16);
        }
        let k = clear(C | D | S);
        assert_eq!((draws(&k), kickers(&k)), (1, 0), "scissored: the quad");
        // SAFETY: a plain GL call.
        unsafe { glDisable(GL_SCISSOR_TEST) };
        let k = clear(C | D | S);
        assert_eq!((draws(&k), kickers(&k)), (0, 2), "whole: colour fill, then depth/stencil fill");
        // SAFETY: a plain GL call.
        unsafe { glColorMask(1, 0, 1, 1) };
        let k = clear(C | D | S);
        assert_eq!((draws(&k), kickers(&k)), (1, 0), "masked: the quad");
        // SAFETY: a plain GL call.
        unsafe { glColorMask(1, 1, 1, 1) };
        let k = clear(C | D | S);
        assert_eq!((draws(&k), kickers(&k)), (0, 2), "unmasked: the fills");
    });
}

/// gc_debug_rs_clear(false) is the quad for every whole clear, in an ES 2.0
/// context as in any other.
#[test]
fn the_knob_off_is_the_quad() {
    for (colour, ds) in [(Colour::Window, Ds::None), (Colour::TexRgba8, Ds::Packed), (Colour::Rb565, Ds::Depth16)] {
        with_backend(|| {
            gc_debug_rs_clear(false);
            target(colour, ds, 64, 64);
            for mask in [C, D, S, C | D | S] {
                let k = clear(mask);
                let drawn = expected(colour, ds, true, mask, State::OPEN) != Some(0);
                assert_eq!((draws(&k), kickers(&k)), (drawn as usize, 0), "{colour:?} {ds:?} 0x{mask:x}");
            }
        });
    }
}
