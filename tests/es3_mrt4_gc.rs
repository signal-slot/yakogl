// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000UL (i.MX8MP): four colour targets in one draw through the host
//! back end (item gc-mrt4-host; docs/es3-cts-failures.md RC14).
//!
//! `backend::DRAW_TARGETS` is 3 on every GPU, so the GL core never hands the
//! Vivante back end a fourth target in a build. The PE has four, and
//! `vivante::backend::mrt_targets` / `hw_targets` take four. This file stands
//! `backend::HOST_DRAW_TARGETS` = 4 in for a back end that draws four, binds
//! four colour attachments through the GL API, and reads the command stream
//! the back end keeps for the kick: RT1..3 must be in the 8-RT bank (address,
//! stride and format, colour mask, blend), PS_CONTROL's RT_COUNT and its
//! saturate bits must count four, PS_OUTPUT_REG must name the fourth output's
//! register, and PS_CONTROL_EXT its output mode.
//!
//! Covered: four RGBA8; the mixed set RGBA8, RGBA16F, RGBA8UI, RGB565; a
//! GL_NONE at draw buffer 2 (the target after it moves down one hardware
//! target and takes its own output); a colour mask; blending, which an
//! integer target never takes; and a clear of the four draw buffers.
//!
//! The GPU draws nothing here: the stand-in OSAL keeps every stream.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_mrt4_gc
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::pe_color_format;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/// The online compiler's store is one static for the process: every test
/// takes this lock.
static STORE: Mutex<()> = Mutex::new(());

/* ---- the stand-in OSAL: host memory, every stream kept -------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
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
        p.kicks.push(words.to_vec());
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

/// Run `case` with the Vivante back end up on the stand-in OSAL, the host
/// claiming four draw targets, and a fresh 64x64 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = STORE.lock().unwrap_or_else(|e| e.into_inner());
    HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
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
    HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    with_platform(|p| {
        /* leaked, not freed, as tests/es3_int_rt_mrt_gc.rs */
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

const SIZE: u32 = 16;

fn renderbuffer(internal: GLenum) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, internal, SIZE as GLsizei, SIZE as GLsizei);
    }
    assert_eq!(err(), GL_NO_ERROR, "glRenderbufferStorage(0x{internal:x})");
    rb
}

/// A framebuffer with a renderbuffer of each of `formats` at colour
/// attachments 0.., and `bufs` as its draw buffers.
fn framebuffer(formats: &[GLenum], bufs: &[GLenum]) {
    let mut fb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &f) in formats.iter().enumerate() {
            let rb = renderbuffer(f);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    assert_eq!(err(), GL_NO_ERROR, "framebuffer");
}

const A: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The register writes of the streams kicked since `mark`.
fn since(mark: usize) -> Vec<(u32, u32)> {
    with_platform(|p| p.kicks[mark..].iter().flat_map(|k| writes(k)).collect())
}

/// Finish, and the number of streams kicked so far.
fn mark() -> usize {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.kicks.len())
}

/// Draw the quad with `fs` over the whole target, finish, and return the GL
/// error and the register writes of the streams kicked.
fn draw(fs: &str, blend: bool) -> (GLenum, Vec<(u32, u32)>) {
    let m = mark();
    // SAFETY: plain GL calls; QUAD is a static.
    let e = unsafe {
        glViewport(0, 0, SIZE as GLsizei, SIZE as GLsizei);
        glUseProgram(program(fs));
        if blend {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        } else {
            glDisable(GL_BLEND);
        }
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glGetError()
    };
    let w = {
        mark();
        since(m)
    };
    (e, w)
}

fn last(w: &[(u32, u32)], reg: u32) -> Option<u32> {
    w.iter().rev().find(|(r, _)| *r == reg).map(|(_, v)| *v)
}

fn count(w: &[(u32, u32)], reg: u32) -> usize {
    w.iter().filter(|(r, _)| *r == reg).count()
}

/// A fragment shader writing one output a target, `types[i]` its GLSL type.
fn fs_outputs(types: &[&str]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, t) in types.iter().enumerate() {
        s += &format!("layout(location = {i}) out {t} o{i};\n");
    }
    s += "void main() {\n";
    for (i, t) in types.iter().enumerate() {
        let v = match *t {
            "uvec4" => format!("uvec4({}u, 2u, 3u, 4u)", i + 1),
            _ => format!("vec4(0.{}, 0.5, 0.25, 0.75)", i + 1),
        };
        s += &format!("  o{i} = {v};\n");
    }
    s + "}\n"
}

/// What the back end programs for a target of surface `format` and `SIZE`
/// pixels wide: the RT_CONFIG of the 8-RT bank.
fn bank_config(format: u8) -> u32 {
    let (fmt, _) = pe_color_format(format);
    let pitch = (SIZE * gles_surf_bytes_per_pixel(format)).next_multiple_of(64);
    rt_config_stride(pitch) | rt_config_format(fmt)
}

/// Byte `k` of PS_OUTPUT_REG: the register hardware target `k` reads.
fn out_reg(w: &[(u32, u32)], k: u32) -> u32 {
    (last(w, GC_PS_OUTPUT_REG).expect("PS_OUTPUT_REG is written") >> (8 * k)) & 0xff
}

fn rt_count(w: &[(u32, u32)]) -> u32 {
    (last(w, GC_PS_CONTROL).expect("PS_CONTROL is written") >> 8) & 0x7
}

fn saturated(w: &[(u32, u32)]) -> u32 {
    (last(w, GC_PS_CONTROL).unwrap() >> 1) & 0xf
}

/// The four-target facts every draw here shares: RT1..3 are in the bank with
/// their own address and `formats[k]`'s format, RT_COUNT is 3, and the four
/// outputs sit in four different registers (a fourth that fell back to the
/// first would show as equal bytes).
fn assert_four_targets(w: &[(u32, u32)], formats: &[u8; 4]) {
    for rt in 1..4u32 {
        let addr = last(w, pe_rt_addr_8_pipe(rt, 0)).unwrap_or_else(|| panic!("RT{rt}: no address written"));
        assert_ne!(addr, 0, "RT{rt}: address 0");
        /* one pixel pipe on this part: the bank has pipe 0 only (Mesa writes one per pixel pipe) */
        assert_eq!(last(w, pe_rt_addr_8_pipe(rt, 1)), None, "RT{rt}: one pixel pipe");
        assert_eq!(last(w, pe_rt_config_8(rt)), Some(bank_config(formats[rt as usize])), "RT{rt}: stride and format");
    }
    let addrs: Vec<u32> = (1..4u32).map(|rt| last(w, pe_rt_addr_8_pipe(rt, 0)).unwrap()).collect();
    assert!(addrs[0] != addrs[1] && addrs[1] != addrs[2] && addrs[0] != addrs[2], "three surfaces: {addrs:x?}");
    assert_eq!(rt_count(w), 3, "RT_COUNT(3): four targets");
    let regs: Vec<u32> = (0..4).map(|k| out_reg(w, k)).collect();
    assert!(regs.windows(2).all(|p| p[0] < p[1]), "four outputs, four registers in output order: {regs:?}");
}

/* ---- the tests -------------------------------------------------------------- */

/// Four RGBA8 targets, one draw: RT1..3 in the bank, the draw's mask and
/// blend on each, all four saturated (unorm), all four in the normal output
/// mode. RT0 is the colour surface, not a bank entry.
#[test]
fn four_rgba8_targets_are_all_programmed() {
    on_gc7000(|| {
        framebuffer(&[GL_RGBA8; 4], &A);
        let (e, w) = draw(&fs_outputs(&["vec4"; 4]), true);
        assert_eq!(e, GL_NO_ERROR, "the draw is accepted");
        assert!(count(&w, GC_PE_COLOR_FORMAT) > 0 && last(&w, pe_pipe_color_addr(0)).is_some(), "RT0 programmed");
        assert_four_targets(&w, &[GLES_SURF_RGBA8888; 4]);
        assert_eq!(saturated(&w), 0xf, "all four unorm targets saturated");
        assert_eq!(last(&w, GC_PS_CONTROL_EXT), Some(0), "all four in the normal output mode");
        let alpha = last(&w, GC_PE_ALPHA_CONFIG).expect("RT0 blend");
        for rt in 1..4u32 {
            assert_eq!(last(&w, pe_halti5_rt_alpha_config(rt)), Some(alpha), "RT{rt}: the blend state of RT0");
            assert_eq!(
                last(&w, pe_halti5_rt_colormask(rt)),
                Some(pe_halti5_rt_colormask_components(0xf)),
                "RT{rt}: every channel, blending so no overwrite"
            );
        }
    });
}

/// The same draw unblended and with a colour mask: each bank entry carries
/// the mask and the OVERWRITE bit follows "no blend and the whole mask".
#[test]
fn the_colour_mask_and_blend_reach_all_three_bank_targets() {
    on_gc7000(|| {
        framebuffer(&[GL_RGBA8; 4], &A);
        let fs = fs_outputs(&["vec4"; 4]);
        let (e, w) = draw(&fs, false);
        assert_eq!(e, GL_NO_ERROR);
        for rt in 1..4u32 {
            assert_eq!(
                last(&w, pe_halti5_rt_colormask(rt)),
                Some(pe_halti5_rt_colormask_components(0xf) | GC_PE_HALTI5_RT_COLORMASK_OVERWRITE),
                "RT{rt}: unblended, full mask: overwrite"
            );
            assert_eq!(last(&w, pe_halti5_rt_alpha_config(rt)), Some(0), "RT{rt}: no blend");
        }
        // SAFETY: a plain GL call.
        unsafe { glColorMask(1, 0, 1, 0) };
        let (e, w) = draw(&fs, false);
        assert_eq!(e, GL_NO_ERROR);
        for rt in 1..4u32 {
            assert_eq!(
                last(&w, pe_halti5_rt_colormask(rt)),
                Some(pe_halti5_rt_colormask_components(0b0101)),
                "RT{rt}: R and B only, no overwrite"
            );
        }
    });
}

/// RGBA8, RGBA16F, RGBA8UI, RGB565: each bank entry has its own format and
/// stride; only the unorm ones (RT0, RT3) are saturated; the integer target
/// is never blended while the float and unorm ones beside it are.
#[test]
fn a_mixed_set_programs_each_target_by_its_own_format() {
    on_gc7000(|| {
        framebuffer(&[GL_RGBA8, GL_RGBA16F, GL_RGBA8UI, GL_RGB565], &A);
        let (e, w) = draw(&fs_outputs(&["vec4", "vec4", "uvec4", "vec4"]), true);
        assert_eq!(e, GL_NO_ERROR, "the draw is accepted");
        /* a GL_RGB565 renderbuffer is stored as RGBA8888 (gl/framebuffer.rs, the
         * format table of glRenderbufferStorage); a true R5G6B5 surface only
         * comes from a window or an external image */
        let formats = [GLES_SURF_RGBA8888, GLES_SURF_RGBA16F, GLES_SURF_RGBA8UI, GLES_SURF_RGBA8888];
        assert_four_targets(&w, &formats);
        assert_eq!(saturated(&w), 0b1001, "RT0 (RGBA8) and RT3 (RGB565) saturated; the float and integer ones not");
        let alpha = last(&w, GC_PE_ALPHA_CONFIG).expect("RT0 blend");
        assert_eq!(last(&w, pe_halti5_rt_alpha_config(1)), Some(alpha), "RT1 (RGBA16F) blends");
        assert_eq!(last(&w, pe_halti5_rt_alpha_config(2)), Some(0), "RT2 (RGBA8UI) is never blended");
        assert_eq!(last(&w, pe_halti5_rt_alpha_config(3)), Some(alpha), "RT3 (RGB565) blends");
        assert_eq!(
            last(&w, GC_PS_CONTROL_EXT),
            Some(ps_output_mode(2, GC_PS_OUTPUT_MODE_U8)),
            "only RT2 (RGBA8UI) leaves the normal output mode, for U8"
        );
    });
}

/// glDrawBuffers(ATT0, ATT1, NONE, ATT3): three hardware targets. The fourth
/// attachment moves down to RT2 and takes its own shader output (register of
/// output 3), RT_COUNT(2), nothing at all for a third bank entry, and no
/// target at address 0.
#[test]
fn a_gap_in_the_draw_buffers_compacts_the_targets() {
    on_gc7000(|| {
        framebuffer(&[GL_RGBA8; 4], &A);
        let fs = fs_outputs(&["vec4"; 4]);
        let (e, full) = draw(&fs, true);
        assert_eq!(e, GL_NO_ERROR);
        let bufs = [A[0], A[1], GL_NONE, A[3]];
        // SAFETY: four readable enums.
        unsafe { glDrawBuffers(4, bufs.as_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
        let (e, w) = draw(&fs, true);
        assert_eq!(e, GL_NO_ERROR, "the gap is a draw buffer GL_NONE, not an error");
        assert_eq!(rt_count(&w), 2, "RT_COUNT(2): three targets");
        assert_eq!(saturated(&w), 0b0111, "three saturated");
        assert_eq!(last(&w, pe_rt_addr_8_pipe(2, 0)), last(&full, pe_rt_addr_8_pipe(3, 0)), "RT2 is attachment 3");
        assert_eq!(last(&w, pe_rt_addr_8_pipe(1, 0)), last(&full, pe_rt_addr_8_pipe(1, 0)), "RT1 is attachment 1");
        assert_eq!(out_reg(&w, 2), out_reg(&full, 3), "RT2 takes output 3's register, not output 2's");
        assert_ne!(out_reg(&w, 2), out_reg(&full, 2));
        /* the third bank entry is not written by this draw: it is only what
         * the previous four-target draw left in the (deduplicated) stream,
         * so the streams of this draw alone have no write for RT3 */
        assert_eq!(count(&w, pe_rt_addr_8_pipe(3, 0)), 0, "RT3 is not programmed with three targets");
        assert!(
            (0..2).all(|p| (1..8u32).all(|rt| last(&w, pe_rt_addr_8_pipe(rt, p)) != Some(0))),
            "no target at address 0"
        );
    });
}

/// glClear of four draw buffers: every target is named by the clear's
/// stream, each at its own address (not RT0's four times).
#[test]
fn a_clear_of_four_draw_buffers_reaches_all_four() {
    on_gc7000(|| {
        framebuffer(&[GL_RGBA8; 4], &A);
        let (e, drawn) = draw(&fs_outputs(&["vec4"; 4]), false);
        assert_eq!(e, GL_NO_ERROR);
        let surfaces: Vec<u32> = std::iter::once(last(&drawn, pe_pipe_color_addr(0)).unwrap())
            .chain((1..4u32).map(|rt| last(&drawn, pe_rt_addr_8_pipe(rt, 0)).unwrap()))
            .collect();
        let m = mark();
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
        let _ = mark();
        let w = since(m);
        assert_eq!(err(), GL_NO_ERROR);
        let named: Vec<u32> = surfaces.iter().copied().filter(|s| w.iter().any(|&(_, v)| v == *s)).collect();
        assert_eq!(named, surfaces, "every one of the four surfaces is in the clear's stream");
    });
}
