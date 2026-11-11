// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): the command stream a whole-target glClear produces, now
//! that `emit_rs_clear` (src/vivante/backend.rs) takes its words from
//! src/vivante/rs_clear.rs (docs/notes/gc-slow-stream-diff.md, finding 1).
//!
//! * A colour clear, a depth clear, a stencil clear, a depth+stencil clear
//!   and a colour+depth clear of the whole target are each one RS sequence
//!   in Mesa 26.2.1's order (etnaviv_rs.c etna_clear_rs): GL_FLUSH_CACHE
//!   COLOR|DEPTH, a stall RA->PE, the colour fill, GL_FLUSH_CACHE again when
//!   a depth fill follows, the depth/stencil fill, a stall RA->PE. Each fill
//!   is coalesced as etnaviv_emit.h does, with RS_SINGLE_BUFFER around the
//!   kick, and no clear quad (no DRAW_INSTANCED in a clear-only kick).
//! * The depth/stencil fill covers the whole padded D24S8 surface (64x64
//!   here: ds_cache.rs pads to 64), RS_CLEAR_CONTROL 0xeeee for depth,
//!   0x1111 for stencil, the word `lrint(z * 0xffffff) << 8 | stencil` in
//!   fill values 0 and 2. A depth-only framebuffer (shadow's and refract's
//!   depth passes) takes it too.
//! * Refusals keep the quad: a scissored clear, a stencil write mask other
//!   than 0xff, a masked colour channel, a colour target that is not 16x4
//!   aligned (the depth plane of that clear stays the quad with it), a
//!   multisampled draw (with `test-knobs`); and `gc_debug_rs_clear(false)`
//!   gives the quad.
//!
//! The colour-only stream differs from the inline one of de5860b5 in four
//! ways, all of them Mesa's: consecutive registers in one LOAD_STATE,
//! no RS_PIPE_SOURCE_ADDR0 (a fill has no source), RS_SINGLE_BUFFER around
//! RS_KICKER (as `emit_rs`'s copies on this SINGLE_BUFFER core), and one
//! stall RA->PE at the end without a second GL_FLUSH_CACHE.
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every submitted stream (as
//! tests/gc_stream_divisor.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_rs_clear_stream
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_rs_clear_stream
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
#[cfg(feature = "test-knobs")]
use yakogl::backend::{GlesDraw, GlesDrawTarget, GLES_SURF_RGBA8888};
use yakogl::gl::consts::*;
use yakogl::gl::framebuffer::{
    glBindFramebuffer, glBindRenderbuffer, glCheckFramebufferStatus, glFramebufferRenderbuffer, glFramebufferTexture2D,
    glGenFramebuffers, glGenRenderbuffers, glRenderbufferStorage,
};
use yakogl::gl::state::{
    glClear, glClearColor, glClearDepthf, glClearStencil, glColorMask, glEnable, glFinish, glGetError, glScissor,
    glStencilMask, glViewport,
};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_debug_rs_clear, gc_init, gc_shutdown, yakogl_backend_vivante};
#[cfg(feature = "test-knobs")]
use yakogl::vivante::backend::{gc_debug_set_msaa, gc_draw, gc_flush};
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
        #[cfg(feature = "test-knobs")]
        gc_debug_set_msaa(0);
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

/// `glClear(mask)` then glFinish: the streams it submitted, in order.
fn clear(mask: GLbitfield) -> Vec<Vec<u32>> {
    with_platform(|p| p.kicks.clear());
    // SAFETY: plain GL calls.
    unsafe {
        glClear(mask);
        glFinish();
    }
    no_error("the clear");
    with_platform(|p| core::mem::take(&mut p.kicks))
}

/// A complete framebuffer object `w` x `h`: an RGBA8 texture at colour 0
/// when `colour`, a D24S8 renderbuffer at depth and stencil. Bound, with the
/// viewport on it.
fn fbo(w: i32, h: i32, colour: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if colour {
            let mut tex: GLuint = 0;
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        }
        let mut rb: GLuint = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, w, h);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
    }
    no_error("the framebuffer object");
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
                i += 1 + count; /* a pad after it is a NOP, stepped over below */
            }
            GC_OP_DRAW_INSTANCED => i += 4,
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

fn kicks_of(kicks: &[Vec<u32>], reg: u32) -> usize {
    kicks.iter().map(|k| writes(k).iter().filter(|(r, _)| *r == reg).count()).sum()
}

fn draws(kicks: &[Vec<u32>]) -> usize {
    let mut n = 0;
    for words in kicks {
        let mut i = 0usize;
        while i < words.len() {
            let w = words[i];
            match w & 0xf800_0000 {
                GC_OP_LOAD_STATE => {
                    let count = ((w >> 16) & 0x3ff) as usize;
                    i += 1 + count;
                }
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

/* ---- the words the stream must carry ---------------------------------------------- */

const PAD: u32 = GC_OP_NOP;

fn flush() -> Vec<u32> {
    vec![load_state(GC_GL_FLUSH_CACHE, 1, false), GC_FLUSH_COLOR | GC_FLUSH_DEPTH]
}

fn stall_ra_pe() -> Vec<u32> {
    let t = stall_token(GC_SYNC_RA, GC_SYNC_PE);
    vec![load_state(GC_GL_SEMAPHORE_TOKEN, 1, false), t, load_state(GC_GL_STALL_TOKEN, 1, false), t]
}

/// One RS fill, A8R8G8B8, linear, as etna_compile_rs_state +
/// etna_submit_rs_state coalesce it: 32 words.
fn fill(addr: u32, stride: u32, w: u32, h: u32, bits: u32, values: [u32; 4]) -> Vec<u32> {
    let cfg = rs_source_format(GC_RS_FORMAT_A8R8G8B8) | rs_dest_format(GC_RS_FORMAT_A8R8G8B8);
    let mut v = vec![
        load_state(GC_RS_CONFIG, 1, false),
        cfg,
        load_state(GC_RS_SOURCE_STRIDE, 1, false),
        0,
        load_state(GC_RS_DEST_STRIDE, 1, false),
        stride,
        load_state(rs_pipe_dest_addr(0), 1, false),
        addr,
        load_state(rs_pipe_offset(0), 2, false),
        0,
        0,
        PAD,
        load_state(GC_RS_WINDOW_SIZE, 1, false),
        rs_window_width(w) | rs_window_height(h),
        load_state(rs_dither(0), 2, false),
        0xffff_ffff,
        0xffff_ffff,
        PAD,
        load_state(GC_RS_CLEAR_CONTROL, 5, false),
        bits | 0x0001_0000,
    ];
    v.extend_from_slice(&values);
    v.extend_from_slice(&[
        load_state(GC_RS_EXTRA_CONFIG, 1, false),
        0,
        load_state(GC_RS_SINGLE_BUFFER, 1, false),
        GC_RS_SINGLE_BUFFER_ENABLE,
        load_state(GC_RS_KICKER, 1, false),
        GC_RS_KICKER_VALUE,
        load_state(GC_RS_SINGLE_BUFFER, 1, false),
        0,
    ]);
    assert_eq!(v.len(), 32);
    v
}

/// The RS clear in `kicks`: the words from its first GL_FLUSH_CACHE (6 words
/// before the first RS_CONFIG) for `len` words, and the destination of each
/// fill (the word after each RS_PIPE_DEST_ADDR0 header). Exactly one kick
/// holds RS_CONFIG.
fn rs_clear_words(kicks: &[Vec<u32>], len: usize) -> (Vec<u32>, Vec<u32>) {
    let with: Vec<&Vec<u32>> = kicks.iter().filter(|k| k.contains(&load_state(GC_RS_CONFIG, 1, false))).collect();
    assert_eq!(with.len(), 1, "one kick with an RS block");
    let k = with[0];
    let p = k.iter().position(|w| *w == load_state(GC_RS_CONFIG, 1, false)).unwrap();
    assert!(p >= 6, "a flush and a stall before the fill");
    let words = k[p - 6..(p - 6 + len).min(k.len())].to_vec();
    let hdr = load_state(rs_pipe_dest_addr(0), 1, false);
    let dests = k.windows(2).filter(|w| w[0] == hdr).map(|w| w[1]).collect();
    (words, dests)
}

/// The D24S8 surface the 64x64 targets here get: 64 pixels a row (stride),
/// 64 rows (ds_cache.rs pads both to 64).
const DS_STRIDE: u32 = 64 * 4;

/// `lrint(z * 0xffffff) << 8 | stencil` (util_pack_z_stencil).
const fn d24s8(z24: u32, stencil: u32) -> u32 {
    z24 << 8 | stencil
}

/// A whole clear of depth/stencil alone: flush, stall, one fill, stall.
fn ds_only(addr: u32, bits: u32, word: u32) -> Vec<u32> {
    let mut v = flush();
    v.extend(stall_ra_pe());
    v.extend(fill(addr, DS_STRIDE, 64, 64, bits, [word, 0, word, 0]));
    v.extend(stall_ra_pe());
    v
}

/* ---- the cases --------------------------------------------------------------------- */

#[test]
fn a_whole_colour_clear_is_one_fill_in_mesas_order() {
    with_backend(|| {
        // SAFETY: a plain GL call.
        unsafe { glClearColor(1.0, 0.5, 0.0, 1.0) };
        let kicks = clear(GL_COLOR_BUFFER_BIT);
        assert_eq!(draws(&kicks), 0, "no clear quad");
        let (words, dests) = rs_clear_words(&kicks, 6 + 32 + 4);
        assert_eq!(dests.len(), 1);
        let mut want = flush();
        want.extend(stall_ra_pe());
        want.extend(fill(dests[0], 64 * 4, 64, 64, 0xffff, [0xffff_8000; 4]));
        want.extend(stall_ra_pe());
        assert_eq!(words, want);
        /* de5860b5's source address write is gone: a fill has no source */
        assert_eq!(kicks_of(&kicks, rs_pipe_source_addr(0)), 0);
    });
}

#[test]
fn a_whole_depth_clear_is_one_fill_of_the_padded_surface() {
    with_backend(|| {
        // SAFETY: a plain GL call.
        unsafe { glClearDepthf(0.5) };
        let kicks = clear(GL_DEPTH_BUFFER_BIT);
        assert_eq!(draws(&kicks), 0, "no clear quad");
        let (words, dests) = rs_clear_words(&kicks, 6 + 32 + 4);
        /* 0.5 * 0xffffff = 8388607.5, to even: 0x800000 */
        assert_eq!(words, ds_only(dests[0], 0xeeee, d24s8(0x80_0000, 0)));
    });
}

#[test]
fn a_whole_stencil_clear_fills_the_stencil_byte() {
    with_backend(|| {
        // SAFETY: a plain GL call.
        unsafe { glClearStencil(0x5a) };
        let kicks = clear(GL_STENCIL_BUFFER_BIT);
        assert_eq!(draws(&kicks), 0, "no clear quad");
        let (words, dests) = rs_clear_words(&kicks, 6 + 32 + 4);
        /* the depth the GL core sends with a stencil-only clear (1.0) is in
         * the word, as Mesa packs it; bits 0x1111 leave it unwritten */
        assert_eq!(words, ds_only(dests[0], 0x1111, d24s8(0xff_ffff, 0x5a)));
    });
}

#[test]
fn a_whole_depth_and_stencil_clear_is_one_fill_of_every_byte() {
    with_backend(|| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearDepthf(0.0);
            glClearStencil(3);
        }
        let kicks = clear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        assert_eq!(draws(&kicks), 0, "no clear quad");
        let (words, dests) = rs_clear_words(&kicks, 6 + 32 + 4);
        assert_eq!(words, ds_only(dests[0], 0xffff, d24s8(0, 3)));
    });
}

#[test]
fn a_whole_colour_and_depth_clear_is_two_fills_with_a_flush_between() {
    with_backend(|| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.0, 0.0, 1.0, 1.0);
            glClearDepthf(1.0);
        }
        let kicks = clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        assert_eq!(draws(&kicks), 0, "no clear quad");
        let (words, dests) = rs_clear_words(&kicks, 6 + 32 + 2 + 32 + 4);
        assert_eq!(dests.len(), 2);
        assert_ne!(dests[0], dests[1], "the colour target, then the depth surface");
        let mut want = flush();
        want.extend(stall_ra_pe());
        want.extend(fill(dests[0], 64 * 4, 64, 64, 0xffff, [0xff00_00ff; 4]));
        want.extend(flush());
        let z = d24s8(0xff_ffff, 0);
        want.extend(fill(dests[1], DS_STRIDE, 64, 64, 0xeeee, [z, 0, z, 0]));
        want.extend(stall_ra_pe());
        assert_eq!(words, want);
    });
}

/// Shadow's and refract's depth passes: a framebuffer object with a depth
/// attachment and no colour, its depth cleared every frame.
#[test]
fn a_depth_only_framebuffers_clear_is_a_fill() {
    with_backend(|| {
        fbo(64, 64, false);
        let kicks = clear(GL_DEPTH_BUFFER_BIT);
        assert_eq!(draws(&kicks), 0, "no clear quad");
        let (words, dests) = rs_clear_words(&kicks, 6 + 32 + 4);
        assert_eq!(words, ds_only(dests[0], 0xeeee, d24s8(0xff_ffff, 0)));
    });
}

/// A 60x60 target: the colour plane is not 16x4-aligned, so a colour+depth
/// clear stays the quad, whole. Its depth alone fills the padded 64x64
/// surface, which is aligned.
#[test]
fn an_unaligned_colour_target_keeps_the_quad() {
    with_backend(|| {
        fbo(60, 60, true);
        let kicks = clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        assert_eq!(draws(&kicks), 1, "the clear quad");
        assert_eq!(kicks_of(&kicks, GC_RS_KICKER), 0);

        let kicks = clear(GL_DEPTH_BUFFER_BIT);
        assert_eq!(draws(&kicks), 0, "no clear quad");
        let (words, dests) = rs_clear_words(&kicks, 6 + 32 + 4);
        assert_eq!(words, ds_only(dests[0], 0xeeee, d24s8(0xff_ffff, 0)));
    });
}

#[test]
fn a_scissored_clear_keeps_the_quad() {
    with_backend(|| {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(GL_SCISSOR_TEST);
            glScissor(0, 0, 16, 16);
        }
        let kicks = clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        assert_eq!(draws(&kicks), 1, "the clear quad");
        assert_eq!(kicks_of(&kicks, GC_RS_KICKER), 0);
    });
}

#[test]
fn a_partial_stencil_mask_keeps_the_quad() {
    with_backend(|| {
        // SAFETY: a plain GL call.
        unsafe { glStencilMask(0xfe) };
        let kicks = clear(GL_STENCIL_BUFFER_BIT);
        assert_eq!(draws(&kicks), 1, "the clear quad");
        assert_eq!(kicks_of(&kicks, GC_RS_KICKER), 0);
    });
}

#[test]
fn a_masked_colour_channel_keeps_the_quad() {
    with_backend(|| {
        // SAFETY: a plain GL call.
        unsafe { glColorMask(1, 1, 1, 0) };
        let kicks = clear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        assert_eq!(draws(&kicks), 1, "the clear quad");
        assert_eq!(kicks_of(&kicks, GC_RS_KICKER), 0);
    });
}

#[test]
fn the_knob_off_gives_the_quad() {
    with_backend(|| {
        gc_debug_rs_clear(false);
        for mask in [GL_COLOR_BUFFER_BIT, GL_DEPTH_BUFFER_BIT, GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT] {
            let kicks = clear(mask);
            assert_eq!(draws(&kicks), 1, "the clear quad for 0x{mask:x}");
            assert_eq!(kicks_of(&kicks, GC_RS_KICKER), 0);
        }
    });
}

/// A multisampled draw renders through the 4x scratch surface: its clear
/// stays the quad. The GL core makes no 4x framebuffer yet
/// (tests/es3_msaa_gc.rs), so the draw is handed to the back end directly;
/// and 4x is a test knob (gc_debug_msaa), off in the product.
#[cfg(feature = "test-knobs")]
#[test]
fn a_multisampled_clear_keeps_the_quad() {
    with_backend(|| {
        gc_debug_set_msaa(1);
        let mut target = OsalGpuMem::ZERO;
        // SAFETY: a descriptor on this stack frame.
        assert_eq!(unsafe { p_gpu_alloc(0x4000_0000, 64 * 64 * 4, 0, &mut target) }, 0);
        let mut d = GlesDraw::ZERO;
        d.target_gpu = target.gpu;
        (d.target_w, d.target_h) = (64, 64);
        d.target_format = GLES_SURF_RGBA8888;
        d.targets[0] = GlesDrawTarget { gpu: target.gpu, w: 64, h: 64, format: GLES_SURF_RGBA8888, out_shift: 0 };
        d.samples = 4;
        d.depth_id = 0x8000_0000 | (target.gpu >> 12) as u32;
        d.stencil_id = d.depth_id;
        (d.ds_w, d.ds_h) = (64, 64);
        d.viewport = [0, 0, 64, 64];
        d.depth_range = [0.0, 1.0];
        d.color_mask = 0xf;
        d.color_write = true;
        d.do_clear = true;
        d.do_clear_depth = true;
        d.clear_depth = 1.0;
        with_platform(|p| p.kicks.clear());
        // SAFETY: a live draw for the call.
        assert_eq!(unsafe { gc_draw(&d) }, 0);
        assert_eq!(gc_flush(), 0);
        let kicks = with_platform(|p| core::mem::take(&mut p.kicks));
        assert_eq!(draws(&kicks), 1, "the clear quad");
        /* the scratch surface's resolve is an RS copy, not a fill */
        assert!(writes(&kicks.concat()).iter().all(|&(r, v)| r != GC_RS_CLEAR_CONTROL || v == 0));

        /* single-sampled, the same clear is two fills */
        d.samples = 0;
        // SAFETY: as above.
        assert_eq!(unsafe { gc_draw(&d) }, 0);
        assert_eq!(gc_flush(), 0);
        let kicks = with_platform(|p| core::mem::take(&mut p.kicks));
        assert_eq!(draws(&kicks), 0);
        assert_eq!(kicks_of(&kicks, GC_RS_KICKER), 2);
    });
}
