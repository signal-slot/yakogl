// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): a mip level above 0 of a pitched chain is a complete
//! colour attachment (item gc-r32-rt-survey of plan #335, the "class D"
//! defect of host dEQP fbo.completeness on GPU=vivante).
//!
//! The GC lays every level of a texture's chain out at level 0's row pitch
//! (`YakoglBackend::mip_level_align`, `GlTexture::chain_pitch`), so level 1
//! of a 16x16 RGBA8 chain is 8 texels of 16 a row. `attachment_storage`
//! (src/gl/framebuffer.rs) took a level whose pitch is not its own
//! pair-padded one as no attachment unless the chain was a PowerVR NPOT one
//! or the back end lays linear rows out at its own pitch, so
//! glCheckFramebufferStatus said INCOMPLETE_ATTACHMENT where ES 3.0 4.4.4.2
//! says COMPLETE. What this file pins:
//!
//! - levels 1 and 2 of a pitched 2D RGBA8 chain are GL_FRAMEBUFFER_COMPLETE;
//! - a clear and a draw into level 1 reach the pixel engine at the chain's
//!   pitch (PE_COLOR_STRIDE, the level's offset in PE_PIPE_COLOR_ADDR), and
//!   glReadPixels reads the level back at that pitch, with level 0 untouched;
//! - a level past the chain (glTexStorage2D's last level + 1) is still
//!   GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT.
//!
//! The GPU is modelled as in tests/es3_int32_rt_gc.rs: the stand-in OSAL's
//! submit stores a colour into the colour surface the stream names, linear at
//! the stream's stride.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gl_level_pitch_attach_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/// The online compiler's store is one static for the process: every test
/// takes this one lock (as tests/es3_int32_rt_gc.rs).
static STORE: Mutex<()> = Mutex::new(());

/// Level 0's side, the chain's levels, and its pitch in bytes (32 RGBA8
/// texels, 128 bytes: on `mip_level_align`'s boundary already). Level 1,
/// 16x16, is cleared by the resolve engine (`rs_clear`: a whole target of
/// 16 by 4 pixels); level 2, 8x8, by the clear quad through the PE.
const SIDE: u32 = 32;
const LEVELS: u32 = 3;
const PITCH: u32 = SIDE * 4;

/* ---- the stand-in OSAL: host memory, every stream, a model of the PE ------ */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    /// While set, every kick that names a colour surface gets `(w, h,
    /// pixel)` stored into it, linear at the stream's PE_COLOR_STRIDE; an RS
    /// fill is modelled whenever it is in the stream (its window at
    /// RS_DEST_STRIDE, its fill value).
    model: Option<(u32, u32, [u8; 4])>,
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

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
    w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else { return -22 };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let stream = words.to_vec();
        let ws = writes(&stream);
        let rs_fill = last(&ws, GC_RS_CLEAR_CONTROL).is_some_and(|v| v & GC_RS_CLEAR_CONTROL_MODE_ENABLED1 != 0);
        let target = if rs_fill {
            let size = last(&ws, GC_RS_WINDOW_SIZE).unwrap_or(0);
            let value = last(&ws, rs_fill_value(0)).unwrap_or(0).to_le_bytes();
            last(&ws, rs_pipe_dest_addr(0))
                .zip(last(&ws, GC_RS_DEST_STRIDE))
                .map(|(a, s)| (a, s, size & 0xffff, size >> 16, value))
        } else {
            p.model.and_then(|(w, h, pixel)| {
                last(&ws, pe_pipe_color_addr(0)).zip(last(&ws, GC_PE_COLOR_STRIDE)).map(|(a, s)| (a, s, w, h, pixel))
            })
        };
        if let Some((addr, stride, w, h, pixel)) = target {
            let addr = u64::from(addr);
            if let Some((base, mem)) = p.allocs.range_mut(..=addr).next_back().map(|(g, m)| (*g, m)) {
                let off = (addr - base) as usize;
                // SAFETY: the allocation is `mem.len() * 8` bytes; the writes
                // below are bounds-checked by the slice.
                let bytes = unsafe { core::slice::from_raw_parts_mut(mem.as_mut_ptr() as *mut u8, mem.len() * 8) };
                for y in 0..h {
                    for x in 0..w {
                        let at = off + (y * stride + x * 4) as usize;
                        bytes[at..at + 4].copy_from_slice(&pixel);
                    }
                }
            }
        }
        p.kicks.push(stream);
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
/// 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| p.model = None);
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
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
        /* leaked, not freed, as tests/gc_npot_repeat.rs: an address a static
         * still names must stay memory */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL helpers ------------------------------------------------------------ */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
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

fn program(fs: &str) -> GLuint {
    const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// Level `l`'s side.
fn side(l: u32) -> u32 {
    (SIDE >> l).max(1)
}

/// The colour every texel of level `l` is uploaded with.
fn fill(l: u32) -> [u8; 4] {
    [0x10 + l as u8, 0x20 + l as u8, 0x30 + l as u8, 0xff]
}

/// A 16x16 RGBA8 texture with an immutable chain of `LEVELS` levels, each
/// level filled with `fill(l)`, bound to GL_TEXTURE_2D.
fn pitched_chain() -> GLuint {
    let mut tex = 0;
    // SAFETY: plain GL calls; each upload's buffer holds the level.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexStorage2D(GL_TEXTURE_2D, LEVELS as GLsizei, GL_RGBA8, SIDE as GLsizei, SIDE as GLsizei);
        for l in 0..LEVELS {
            let s = side(l);
            let texels: Vec<u8> = (0..s * s).flat_map(|_| fill(l)).collect();
            glTexSubImage2D(
                GL_TEXTURE_2D,
                l as GLint,
                0,
                0,
                s as GLsizei,
                s as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the chain");
    tex
}

/// A framebuffer with level `level` of `tex` at colour attachment 0; the
/// status it reports.
fn attach(tex: GLuint, level: GLint) -> GLenum {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    let status = unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, level);
        glCheckFramebufferStatus(GL_FRAMEBUFFER)
    };
    assert_eq!(err(), GL_NO_ERROR, "attach level {level}");
    status
}

/// The writes of the streams submitted while `f` runs (finished).
fn kicked(f: impl FnOnce()) -> Vec<(u32, u32)> {
    let kicks0 = with_platform(|p| p.kicks.len());
    f();
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.kicks[kicks0..].iter().flat_map(|k| writes(k)).collect())
}

/// The `w` x `h` pixels the bound read framebuffer holds, RGBA8.
fn read(w: u32, h: u32) -> Vec<[u8; 4]> {
    let mut buf = vec![0u8; (w * h * 4) as usize];
    // SAFETY: the buffer holds w * h RGBA8 pixels.
    unsafe { glReadPixels(0, 0, w as GLsizei, h as GLsizei, GL_RGBA, GL_UNSIGNED_BYTE, buf.as_mut_ptr() as *mut _) };
    assert_eq!(err(), GL_NO_ERROR, "glReadPixels");
    buf.chunks(4).map(|p| [p[0], p[1], p[2], p[3]]).collect()
}

/// Level `l` of `tex` read back through a framebuffer of its own, every
/// pixel `want`.
fn assert_level(tex: GLuint, l: u32, want: [u8; 4], what: &str) {
    assert_eq!(attach(tex, l as GLint), GL_FRAMEBUFFER_COMPLETE, "{what}: level {l}");
    for (i, px) in read(side(l), side(l)).iter().enumerate() {
        let (x, y) = (i as u32 % side(l), i as u32 / side(l));
        assert_eq!(*px, want, "{what}: level {l} pixel ({x}, {y})");
    }
}

/// The byte offset of level `l` in the chain: every level `PITCH` bytes a
/// row, each starting on `mip_level_align`'s 64 bytes.
fn level_offset(l: u32) -> u32 {
    (0..l).fold(0, |off, k| (off + PITCH * side(k)).div_ceil(64) * 64)
}

/// `rgba` in the order the PE stores an RGBA8 target (A8R8G8B8: B, G, R, A
/// in memory), which the model writes.
fn pe(rgba: [u8; 4]) -> [u8; 4] {
    [rgba[2], rgba[1], rgba[0], rgba[3]]
}

/// Upload `fill(l)` into level `l` of the bound texture again.
fn refill(l: u32) {
    let s = side(l);
    let texels: Vec<u8> = (0..s * s).flat_map(|_| fill(l)).collect();
    // SAFETY: the buffer holds the level.
    unsafe {
        glTexSubImage2D(
            GL_TEXTURE_2D,
            l as GLint,
            0,
            0,
            s as GLsizei,
            s as GLsizei,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            texels.as_ptr() as *const c_void,
        )
    };
    assert_eq!(err(), GL_NO_ERROR, "refill level {l}");
}

/// glClear of the bound framebuffer, `w` x `h`, with `rgba`; the writes of
/// the streams it submitted. The PE model stores `rgba` where the stream
/// draws the clear quad.
fn clear(w: u32, h: u32, rgba: [u8; 4]) -> Vec<(u32, u32)> {
    let f = |v: u8| f32::from(v) / 255.0;
    with_platform(|p| p.model = Some((w, h, pe(rgba))));
    // SAFETY: plain GL calls.
    let ws = kicked(|| unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glClearColor(f(rgba[0]), f(rgba[1]), f(rgba[2]), f(rgba[3]));
        glClear(GL_COLOR_BUFFER_BIT);
    });
    with_platform(|p| p.model = None);
    assert_eq!(err(), GL_NO_ERROR, "the clear");
    ws
}

/* ---- the cases ------------------------------------------------------------- */

/// Levels 1 and 2 of the pitched chain are complete colour attachments, as
/// ES 3.0 4.4.4.2 says of any level of the texture's chain.
#[test]
fn a_level_above_0_of_the_pitched_chain_is_complete() {
    on_gc7000(|| {
        let tex = pitched_chain();
        for l in 0..LEVELS {
            assert_eq!(attach(tex, l as GLint), GL_FRAMEBUFFER_COMPLETE, "level {l} of the {SIDE}x{SIDE} chain");
        }
    });
}

/// A level past the chain glTexStorage2D made (the first one past it, and
/// one past the levels the storage could hold) is no attachment:
/// incomplete, as before the fix.
#[test]
fn a_level_past_the_chain_stays_incomplete() {
    on_gc7000(|| {
        let tex = pitched_chain();
        for l in [LEVELS, 6] {
            assert_eq!(
                attach(tex, l as GLint),
                GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT,
                "level {l} of a {LEVELS}-level chain"
            );
        }
    });
}

/// A clear of level 1 (an RS fill) and of level 2 (the clear quad) reaches
/// the GPU at the chain's pitch and the level's offset; each reads back as
/// its clear colour while level 0 keeps what was uploaded.
#[test]
fn a_clear_of_a_level_writes_it_at_the_chain_pitch() {
    on_gc7000(|| {
        let tex = pitched_chain();
        /* level 0's own clear (an RS fill too) names the chain's base */
        assert_eq!(attach(tex, 0), GL_FRAMEBUFFER_COMPLETE);
        let ws = clear(SIDE, SIDE, [0, 0, 0, 0xff]);
        let base = last(&ws, rs_pipe_dest_addr(0)).expect("level 0's clear is an RS fill");
        assert_eq!(last(&ws, GC_RS_DEST_STRIDE), Some(PITCH), "level 0");
        // SAFETY: a plain GL call.
        unsafe { glBindTexture(GL_TEXTURE_2D, tex) };
        refill(0);

        let red = [0xff, 0x00, 0x00, 0xff];
        assert_eq!(attach(tex, 1), GL_FRAMEBUFFER_COMPLETE);
        let ws = clear(side(1), side(1), red);
        assert_eq!(last(&ws, GC_RS_DEST_STRIDE), Some(PITCH), "level 1 at the chain's pitch, not its own");
        assert_eq!(last(&ws, rs_pipe_dest_addr(0)), Some(base + level_offset(1)), "level 1's offset");

        let blue = [0x00, 0x00, 0xff, 0xff];
        assert_eq!(attach(tex, 2), GL_FRAMEBUFFER_COMPLETE);
        let ws = clear(side(2), side(2), blue);
        assert_eq!(last(&ws, GC_PE_COLOR_STRIDE), Some(PITCH), "level 2 at the chain's pitch, not its own");
        assert_eq!(last(&ws, pe_pipe_color_addr(0)), Some(base + level_offset(2)), "level 2's offset");

        assert_level(tex, 1, red, "after the clears");
        assert_level(tex, 2, blue, "after the clears");
        assert_level(tex, 0, fill(0), "after the clears");
    });
}

/// A draw into level 1 likewise: the PE at the chain's pitch, the level read
/// back as drawn, levels 0 and 2 as uploaded.
#[test]
fn a_draw_into_level_1_writes_it_at_the_chain_pitch() {
    static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];
    on_gc7000(|| {
        let tex = pitched_chain();
        let green = [0x00, 0xff, 0x00, 0xff];
        let prog = program(
            "#version 300 es\nlayout(location = 0) out mediump vec4 o;\nvoid main() { o = vec4(0.0, 1.0, 0.0, 1.0); }\n",
        );
        assert_eq!(attach(tex, 1), GL_FRAMEBUFFER_COMPLETE);
        with_platform(|p| p.model = Some((side(1), side(1), pe(green))));
        // SAFETY: plain GL calls; QUAD is a static.
        let ws = kicked(|| unsafe {
            glViewport(0, 0, side(1) as GLsizei, side(1) as GLsizei);
            glUseProgram(prog);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        });
        with_platform(|p| p.model = None);
        assert_eq!(err(), GL_NO_ERROR, "the draw");
        assert_eq!(last(&ws, GC_PE_COLOR_STRIDE), Some(PITCH), "level 1 at the chain's pitch, not its own");
        assert_level(tex, 1, green, "after the draw");
        assert_level(tex, 0, fill(0), "after the draw");
        assert_level(tex, 2, fill(2), "after the draw");
    });
}
