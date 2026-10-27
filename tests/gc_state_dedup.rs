// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): a draw writes a state group only when its words differ
//! from the ones the open stream holds last for it (src/vivante/dedup.rs,
//! fix F2 of docs/notes/gl-draw-fixed-cost.md).
//!
//! How it runs: the real GL core and the real Vivante back end over a
//! stand-in OSAL that keeps every allocation in host memory and every
//! submitted stream. Each stream is decoded: how often a group's registers
//! are written, and the register file as the stream has loaded it at each
//! DRAW_INSTANCED.
//!
//! * Two identical draws write the state once; the flushes, the stalls and
//!   the prefetches are written at both.
//! * A draw with one changed state writes that group again and no other.
//! * Every path that forgets the groups makes the next draw write all of
//!   them: a new stream, the mipmap chain's kicks, the depth resolve, the
//!   skip switched off and on.
//! * A mixed sequence (clears, three programs, uniforms, blending, viewport
//!   and scissor, client and buffer arrays, indexed draws, a texture with a
//!   GPU-made chain, depth test) leaves every draw with the register file it
//!   has with every group written at every draw: what the GPU renders is
//!   what it rendered before.
//!
//! What it cannot see: the GPU. That a register written once keeps its value
//! across the draws of one stream is the hardware's (and etnaviv's) model.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_state_dedup
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::draw::{glDrawArrays, glDrawElements};
use yakogl::gl::framebuffer::{glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glGetUniformLocation, glLinkProgram,
    glUniform1i, glUniform4f, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{
    glBlendFunc, glClear, glClearColor, glDepthFunc, glDisable, glEnable, glFinish, glFlush, glGetError, glPixelStorei,
    glScissor, glViewport,
};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glDisableVertexAttribArray, glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_depth_resolve, gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::cmd::GcCmd;
use yakogl::vivante::dedup::{self, Group, GROUPS};
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
/// Serialises the tests: the OSAL pointer, the back end, the state cache and
/// `g_backend` are process-global.
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
/// fresh 64x64 context. The stand-in starts empty and hands out the same
/// addresses as at the last start (the memory of earlier cases is leaked,
/// not reused), so two runs of one GL sequence get the same streams.
fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
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

/// A linked program of `vs` and `fs`, attribute `p` at 0 and `q` at 1.
fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glBindAttribLocation(p, 1, c"q".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

const FLAT_VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FLAT_FS: &str = "precision mediump float;\nuniform vec4 c;\nvoid main() { gl_FragColor = c; }\n";
const VARYING_VS: &str = "attribute vec4 p;\nattribute vec4 q;\nvarying vec4 v;\n\
                          void main() { v = q; gl_Position = p; }\n";
const VARYING_FS: &str = "precision mediump float;\nvarying vec4 v;\nvoid main() { gl_FragColor = v; }\n";
const TEX_VS: &str = "attribute vec4 p;\nvarying vec2 t;\n\
                      void main() { t = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const TEX_FS: &str = "precision mediump float;\nuniform sampler2D s;\nvarying vec2 t;\n\
                      void main() { gl_FragColor = texture2D(s, t); }\n";

/// A strip over the target, at z 0.5, then one colour a vertex.
static QUAD: [f32; 32] = [
    -1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0, //
    1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0,
];
static INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

/// A 64x64 RGBA8 texture, its name; `data` its texels or none.
fn texture(w: u32, h: u32, data: Option<&[u8]>) -> GLuint {
    let mut tex: GLuint = 0;
    // SAFETY: plain GL calls; `data` holds w x h texels when given.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            data.map_or(core::ptr::null(), |d| d.as_ptr() as *const c_void),
        );
    }
    no_error("glTexImage2D");
    tex
}

/// A framebuffer with a fresh 64x64 texture as its colour, bound.
fn render_target() -> GLuint {
    let tex = texture(64, 64, None);
    // SAFETY: plain GL calls.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "the texture's framebuffer");
        glViewport(0, 0, 64, 64);
    }
    no_error("the render target");
    tex
}

/// A buffer holding [`QUAD`], bound to GL_ARRAY_BUFFER and fed to attribute
/// 0 (positions) and 1 (colours).
fn quad_buffer() -> GLuint {
    let mut vbo: GLuint = 0;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&QUAD) as GLsizeiptr,
            QUAD.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(1);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, (16 * 4) as *const c_void);
    }
    no_error("the quad buffer");
    vbo
}

fn draw() {
    // SAFETY: a plain GL call on the bound arrays.
    unsafe { glDrawArrays(GL_TRIANGLE_STRIP, 0, 4) };
    no_error("glDrawArrays");
}

fn set_colour(prog: GLuint, rgba: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        let l = glGetUniformLocation(prog, c"c".as_ptr());
        assert!(l >= 0, "uniform c");
        glUniform4f(l, rgba[0], rgba[1], rgba[2], rgba[3]);
    }
}

/// The streams submitted since the last call.
fn take_kicks() -> Vec<Vec<u32>> {
    with_platform(|p| core::mem::take(&mut p.kicks))
}

/// Walk a stream's commands: `f(reg, value)` for every register a
/// LOAD_STATE writes, `draw()` at every DRAW_INSTANCED.
fn walk(words: &[u32], mut f: impl FnMut(u32, u32), mut draw: impl FnMut()) {
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    f(base + 4 * k as u32, words[i + 1 + k]);
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                draw();
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
}

/// How often `reg` is written in `words`.
fn writes(words: &[u32], reg: u32) -> usize {
    let mut n = 0;
    walk(words, |r, _| n += usize::from(r == reg), || {});
    n
}

fn draws(words: &[u32]) -> usize {
    let mut n = 0;
    walk(words, |_, _| {}, || n += 1);
    n
}

/// The register file as the stream has loaded it at each draw.
fn draw_regs(words: &[u32]) -> Vec<BTreeMap<u32, u32>> {
    let regs = core::cell::RefCell::new(BTreeMap::new());
    let mut out = Vec::new();
    walk(
        words,
        |r, v| {
            regs.borrow_mut().insert(r, v);
        },
        || out.push(regs.borrow().clone()),
    );
    out
}

/// One register of each group, and what is written at every draw whatever
/// the groups do.
const VERTEX: u32 = nfe_generic_attrib_config1(0);
const STREAMS: u32 = nfe_vertex_streams_base_addr(0);
const GROUP_REGS: [(Group, u32); 8] = [
    (Group::Vertex, VERTEX),
    (Group::Streams, STREAMS),
    (Group::Raster, GC_PA_CONFIG),
    (Group::Ra, GC_RA_CONTROL),
    (Group::Msaa, GC_GL_MULTI_SAMPLE_CONFIG),
    (Group::Pe, GC_PE_DEPTH_CONFIG),
    (Group::Shader, GC_VS_OUTPUT_COUNT),
    (Group::Program, GC_VS_INST_ADDR),
];
const EVERY_DRAW: [u32; 2] = [GC_TS_FLUSH_CACHE, GC_VS_ICACHE_PREFETCH];

/// How often each group's register is written in `words`.
fn group_writes(words: &[u32]) -> Vec<(Group, usize)> {
    GROUP_REGS.iter().map(|&(g, r)| (g, writes(words, r))).collect()
}

/// What [`group_writes`] reads for every group written `n` times.
fn every_group(n: usize) -> Vec<(Group, usize)> {
    GROUP_REGS.iter().map(|&(g, _)| (g, n)).collect()
}

/// The GPU mipmap chains made since the back end started.
fn mip_chains() -> u32 {
    extern "C" {
        fn gc_backend_mip_chains(made: *mut u32, refused: *mut u32);
    }
    let mut made = 0u32;
    // SAFETY: one valid u32 out-pointer, the other null.
    unsafe { gc_backend_mip_chains(&mut made, core::ptr::null_mut()) };
    made
}

/// A program drawing flat colour into a fresh target from the quad buffer,
/// in use: the setting of the first three tests. One draw is made and
/// finished first: the process's first draw writes
/// GL_VARYING_COMPONENT_USE (whatever an earlier client left there,
/// `shader.rs` `emit_shader_state`) and the next one does not, so their
/// shader groups differ.
fn flat_setup() -> GLuint {
    render_target();
    quad_buffer();
    let p = program(FLAT_VS, FLAT_FS);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glFinish();
    }
    set_colour(p, [0.25, 0.5, 0.75, 1.0]);
    no_error("the flat setup");
    let _ = take_kicks();
    p
}

/* ---- the groups -------------------------------------------------------------------- */

/// The unit itself: equal words are skipped, different words written and
/// kept, and [`dedup::invalidate`] makes the next one written whatever it is.
#[test]
fn equal_words_are_skipped_and_different_ones_written() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let mut buf = vec![0u32; 256];
    let mut w = GcCmd { words: buf.as_mut_ptr(), used: 0, cap: 256, overflow: false };
    dedup::set_enabled(true);
    dedup::reset_counts();
    let pe = |c: &mut GcCmd, v: u32| {
        c.state(GC_PE_DEPTH_CONFIG, v);
        c.state(GC_PE_ALPHA_CONFIG, 7);
    };
    dedup::group(&mut w, Group::Pe, |c| pe(c, 1));
    assert_eq!(w.used, 4, "the first time: written");
    dedup::group(&mut w, Group::Pe, |c| pe(c, 1));
    assert_eq!(w.used, 4, "the same words: skipped");
    dedup::group(&mut w, Group::Raster, |c| c.state(GC_PA_CONFIG, 1));
    assert_eq!(w.used, 6, "another group is its own");
    dedup::group(&mut w, Group::Pe, |c| pe(c, 2));
    assert_eq!(w.used, 10, "one word different: the group again, whole");
    assert_eq!(
        &buf[6..10],
        &[load_state(GC_PE_DEPTH_CONFIG, 1, false), 2, load_state(GC_PE_ALPHA_CONFIG, 1, false), 7]
    );
    dedup::group(&mut w, Group::Pe, |c| pe(c, 2));
    assert_eq!(w.used, 10);
    dedup::invalidate();
    dedup::group(&mut w, Group::Pe, |c| pe(c, 2));
    assert_eq!(w.used, 14, "forgotten: written again");
    /* what a group returns comes back whether it was written or not */
    assert!(dedup::group(&mut w, Group::Pe, |c| {
        pe(c, 2);
        true
    }));
    assert_eq!(dedup::counts(), (4, 3), "four written, three skipped");
    dedup::set_enabled(false);
    dedup::group(&mut w, Group::Pe, |c| pe(c, 2));
    assert_eq!(w.used, 18, "off: written every time");
    dedup::set_enabled(true);
    dedup::group(&mut w, Group::Pe, |c| pe(c, 2));
    assert_eq!(w.used, 22, "on again: nothing known, written");
    /* a stream with no room: nothing kept, so the words are written again
     * where there is room */
    let mut tiny = vec![0u32; 2];
    let mut t = GcCmd { words: tiny.as_mut_ptr(), used: 0, cap: 2, overflow: false };
    dedup::invalidate();
    dedup::group(&mut t, Group::Pe, |c| pe(c, 3));
    assert!(t.overflow && t.used == 0, "did not fit: nothing written");
    dedup::group(&mut w, Group::Pe, |c| pe(c, 3));
    assert_eq!(w.used, 26, "not kept: written");
}

/// Two identical draws in one stream: every group once, the flushes, the
/// stalls and the prefetches twice.
#[test]
fn two_identical_draws_write_the_state_once() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_backend(|| {
        flat_setup();
        draw();
        dedup::reset_counts();
        draw();
        assert_eq!(dedup::counts(), (0, GROUPS as u32), "the second draw: every group equal, none written");
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        let kicks = take_kicks();
        assert_eq!(kicks.len(), 1, "one stream");
        let s = &kicks[0];
        assert_eq!(draws(s), 2);
        assert_eq!(group_writes(s), every_group(1), "every group written once");
        for r in EVERY_DRAW {
            assert_eq!(writes(s, r), 2, "register 0x{r:x} at both draws");
        }
        let regs = draw_regs(s);
        assert_eq!(regs[0], regs[1], "the second draw sees the first one's registers");
    });
}

/// A draw with one state changed writes that group and no other.
#[test]
fn one_changed_state_writes_only_its_group() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    type Change = fn(GLuint);
    /* each change is plain GL calls on the current context (the unsafe
     * blocks below) */
    let cases: [(&str, Change, Group, u32); 4] = [
        (
            "blending",
            |_| unsafe {
                glEnable(GL_BLEND);
                glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            },
            Group::Pe,
            GC_PE_ALPHA_CONFIG,
        ),
        ("the viewport", |_| unsafe { glViewport(8, 8, 32, 32) }, Group::Raster, GC_PA_VIEWPORT_SCALE_X),
        ("a uniform", |p| set_colour(p, [1.0, 0.0, 0.0, 1.0]), Group::Program, GC_VS_INST_ADDR),
        (
            "the vertex streams",
            |_| unsafe { glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, 16 as *const c_void) },
            Group::Streams,
            STREAMS,
        ),
    ];
    for (what, change, group, reg) in cases {
        with_backend(|| {
            let p = flat_setup();
            draw();
            change(p);
            no_error(what);
            dedup::reset_counts();
            draw();
            assert_eq!(dedup::counts(), (1, GROUPS as u32 - 1), "{what}: one group written, the others skipped");
            // SAFETY: a plain GL call.
            unsafe { glFinish() };
            let kicks = take_kicks();
            assert_eq!(kicks.len(), 1, "{what}: one stream");
            let s = &kicks[0];
            let want: Vec<(Group, usize)> =
                GROUP_REGS.iter().map(|&(g, _)| (g, if g == group { 2 } else { 1 })).collect();
            assert_eq!(group_writes(s), want, "{what}: {group:?} written again, nothing else");
            assert_eq!(writes(s, reg), 2, "{what}: register 0x{reg:x} at both draws");
        });
    }
}

/// Each path that forgets the groups makes the next draw write all of them.
#[test]
fn each_invalidating_path_writes_every_group_again() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    /* a new stream: glFlush between the draws */
    with_backend(|| {
        flat_setup();
        draw();
        // SAFETY: a plain GL call.
        unsafe { glFlush() };
        draw();
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        let kicks = take_kicks();
        assert_eq!(kicks.len(), 2, "two streams");
        for (k, s) in kicks.iter().enumerate() {
            assert_eq!(group_writes(s), every_group(1), "stream {k}: every group");
        }
    });
    /* the depth resolve, inside one stream */
    with_backend(|| {
        flat_setup();
        draw();
        let mut texel = 0.0f32;
        // SAFETY: one float at `texel`; an object with no depth surface.
        let _ = unsafe { gc_depth_resolve(0x4000_1234, &mut texel, 1, 1, 1) };
        dedup::reset_counts();
        draw();
        assert_eq!(dedup::counts(), (GROUPS as u32, 0), "after the depth resolve: every group written");
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        let kicks = take_kicks();
        assert_eq!(kicks.len(), 1, "one stream");
        assert_eq!(group_writes(&kicks[0]), every_group(2), "every group at both draws");
    });
    /* the skip switched off and back on, inside one stream */
    with_backend(|| {
        flat_setup();
        draw();
        dedup::set_enabled(false);
        draw();
        dedup::set_enabled(true);
        dedup::reset_counts();
        draw();
        assert_eq!(dedup::counts(), (GROUPS as u32, 0), "switched back on: every group written");
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        let kicks = take_kicks();
        assert_eq!(group_writes(&kicks[0]), every_group(3), "every group at every draw");
    });
    /* the mipmap chain's kicks: the draw after them writes every group */
    with_backend(|| {
        let sampled = texture(64, 64, Some(&vec![0x5au8; 64 * 64 * 4]));
        let p = flat_setup();
        draw();
        draw();
        let made = mip_chains();
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, sampled);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glGenerateMipmap(GL_TEXTURE_2D);
        }
        no_error("glGenerateMipmap");
        assert_eq!(mip_chains(), made + 1, "the GPU made the chain");
        set_colour(p, [0.25, 0.5, 0.75, 1.0]);
        draw();
        // SAFETY: a plain GL call.
        unsafe { glFinish() };
        let kicks = take_kicks();
        /* the two draws, a stream per level above 0 (64 -> 1: six), the draw */
        assert_eq!(kicks.len(), 1 + 6 + 1, "the draws' streams and the chain's");
        assert_eq!(draws(&kicks[0]), 2);
        for (k, s) in kicks.iter().enumerate() {
            assert_eq!(group_writes(s), every_group(1), "stream {k}: every group, once");
        }
    });
}

/* ---- what it saves ------------------------------------------------------------------- */

const MVP_VS: &str = "uniform mat4 m;\nattribute vec4 p;\nvoid main() { gl_Position = m * p; }\n";

/// What a glDrawElements costs on the host, the GL core and this back end
/// together, in the shape of glmark2 `ideas:speed=duration` (scene-ideas
/// `lamp.cc`, `table.cc`): 190 triangle strips a frame out of one element
/// buffer at a new offset each, the uniforms changed once per object (every
/// 19 draws). Printed, not asserted, with the skip off and on, in the same
/// binary; run it with `--release --nocapture`:
///
/// ```text
/// cargo test --release --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_state_dedup time_ -- --nocapture
/// ```
#[test]
fn time_ideas_shaped_draws_with_and_without_the_skip() {
    const FRAMES: usize = 200;
    const DRAWS: usize = 190;
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let indices: Vec<u16> = (0..DRAWS as u16 * 4).map(|i| i % 4).collect();
    let run = |on: bool| -> (f64, usize) {
        with_backend(|| {
            dedup::set_enabled(on);
            render_target();
            quad_buffer();
            let p = program(MVP_VS, FLAT_FS);
            let mut ebo: GLuint = 0;
            let mut best = f64::MAX;
            let mut words = 0;
            // SAFETY: plain GL calls; `indices` outlives them.
            unsafe {
                glDisableVertexAttribArray(1);
                glUseProgram(p);
                let m = glGetUniformLocation(p, c"m".as_ptr());
                glGenBuffers(1, &mut ebo);
                glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ebo);
                glBufferData(
                    GL_ELEMENT_ARRAY_BUFFER,
                    (indices.len() * 2) as GLsizeiptr,
                    indices.as_ptr() as *const c_void,
                    GL_STATIC_DRAW,
                );
                let mut mat = [0.0f32; 16];
                for frame in 0..FRAMES {
                    let t = std::time::Instant::now();
                    for i in 0..DRAWS {
                        if i % 19 == 0 {
                            let s = 0.5 + (i + frame) as f32 * 1e-3;
                            mat[0] = s;
                            mat[5] = s;
                            mat[10] = 1.0;
                            mat[15] = 1.0;
                            yakogl::gl::program::glUniformMatrix4fv(m, 1, 0, mat.as_ptr());
                            set_colour(p, [s, 0.5, 0.25, 1.0]);
                        }
                        glDrawElements(GL_TRIANGLE_STRIP, 4, GL_UNSIGNED_SHORT, (i * 8) as *const c_void);
                    }
                    let us = t.elapsed().as_secs_f64() * 1e6 / DRAWS as f64;
                    glFlush();
                    /* the first frames build the caches either way */
                    if frame >= 10 {
                        best = best.min(us);
                    }
                }
                no_error("the timed draws");
            }
            let kicks = take_kicks();
            if let Some(k) = kicks.last() {
                words = k.len();
            }
            (best, words)
        })
    };
    /* the key hits unchecked, as in a product build */
    dedup::set_verify(false);
    let (mut off, mut on) = ((f64::MAX, 0), (f64::MAX, 0));
    for _ in 0..5 {
        let o = run(false);
        let n = run(true);
        off = (off.0.min(o.0), o.1);
        on = (on.0.min(n.0), n.1);
    }
    dedup::set_verify(true);
    println!(
        "ideas-shaped glDrawElements, best frame of {FRAMES} x {DRAWS}, 5 runs: {:.3} us a draw with every group \
         written ({} stream words a frame), {:.3} us with the skip ({} words a frame)",
        off.0, off.1, on.0, on.1
    );
}

/* ---- the same pictures --------------------------------------------------------------- */

/// glmark2's kinds of state changes in one sequence, with the skip `on` or
/// off: the streams submitted.
fn mixed_sequence(on: bool) -> Vec<Vec<u32>> {
    with_backend(|| {
        dedup::set_enabled(on);
        render_target();
        let vbo = quad_buffer();
        let flat = program(FLAT_VS, FLAT_FS);
        let varying = program(VARYING_VS, VARYING_FS);
        let tex = program(TEX_VS, TEX_FS);
        let texels: Vec<u8> = (0..32 * 32 * 4).map(|i| (i * 7 % 251) as u8).collect();
        let sampled = texture(32, 32, Some(&texels));
        let mut ebo: GLuint = 0;
        // SAFETY: plain GL calls; the arrays are statics or live locals.
        unsafe {
            glGenBuffers(1, &mut ebo);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ebo);
            glBufferData(
                GL_ELEMENT_ARRAY_BUFFER,
                core::mem::size_of_val(&INDICES) as GLsizeiptr,
                INDICES.as_ptr() as *const c_void,
                GL_STATIC_DRAW,
            );

            glClearColor(0.1, 0.2, 0.3, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
            glUseProgram(flat);
            set_colour(flat, [1.0, 0.0, 0.0, 1.0]);
            draw();
            draw();
            set_colour(flat, [0.0, 1.0, 0.0, 0.5]);
            draw();
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
            draw();
            glViewport(8, 8, 32, 32);
            draw();
            glUseProgram(varying);
            draw();
            draw();
            glEnable(GL_SCISSOR_TEST);
            glScissor(4, 4, 40, 40);
            draw();
            glDisable(GL_BLEND);
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, core::ptr::null());
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, core::ptr::null());
            glDrawElements(GL_TRIANGLES, 3, GL_UNSIGNED_SHORT, 6 as *const c_void);
            /* client arrays and client indices: copies in the frame arena */
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, QUAD[16..].as_ptr() as *const c_void);
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, INDICES.as_ptr() as *const c_void);
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, INDICES.as_ptr() as *const c_void);
            glDisable(GL_SCISSOR_TEST);
            glViewport(0, 0, 64, 64);
            glBindBuffer(GL_ARRAY_BUFFER, vbo);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, (16 * 4) as *const c_void);
            /* a texture with a GPU-made chain, then sampled */
            glBindTexture(GL_TEXTURE_2D, sampled);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glGenerateMipmap(GL_TEXTURE_2D);
            glUseProgram(tex);
            glUniform1i(glGetUniformLocation(tex, c"s".as_ptr()), 0);
            glDisableVertexAttribArray(1);
            draw();
            draw();
            glFlush();
            glEnableVertexAttribArray(1);
            glUseProgram(flat);
            draw();
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            draw();
            glClear(GL_COLOR_BUFFER_BIT);
            draw();
            glDisable(GL_DEPTH_TEST);
            glUseProgram(varying);
            draw();
            glFinish();
        }
        no_error("the mixed sequence");
        take_kicks()
    })
}

/// Every draw of the mixed sequence sees the registers it sees with every
/// group written at every draw, in the same streams; the streams are shorter.
#[test]
fn every_draw_sees_the_registers_it_did_with_every_group_written() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    /* once to settle what the library keeps across runs */
    let _ = mixed_sequence(true);
    let all = mixed_sequence(false);
    let skipped = mixed_sequence(true);
    assert_eq!(all.len(), skipped.len(), "the same streams");
    let (mut n, mut words_all, mut words_skipped) = (0, 0, 0);
    for (k, (a, s)) in all.iter().zip(&skipped).enumerate() {
        let (ra, rs) = (draw_regs(a), draw_regs(s));
        assert_eq!(ra.len(), rs.len(), "stream {k}: the same draws");
        for (i, (x, y)) in ra.iter().zip(&rs).enumerate() {
            let differ: Vec<_> = x
                .keys()
                .chain(y.keys())
                .filter(|r| x.get(r) != y.get(r))
                .map(|r| (*r, x.get(r).copied(), y.get(r).copied()))
                .collect();
            assert!(differ.is_empty(), "stream {k} draw {i}: (register, every group, skipped) {differ:x?}");
        }
        n += ra.len();
        words_all += a.len();
        words_skipped += s.len();
    }
    assert!(n >= 20, "the sequence draws ({n})");
    assert!(words_skipped < words_all, "the skip shortens the streams: {words_skipped} words against {words_all}");
    println!("mixed sequence: {n} draws, {words_skipped} stream words with the skip, {words_all} without");
}
