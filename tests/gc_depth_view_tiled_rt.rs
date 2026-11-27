// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): what the depth-sampling experiment's mode 1
//! (`gc_debug_set_depth_tpu(1)`, src/vivante/depth_sample.rs) does when the
//! tiled render targets are on (`gc_debug_set_tiled_rt(1)`, src/vivante/
//! tiled_rt.rs), the open bullet of docs/notes/gc-depth-sample-wire.md,
//! "What this note does not decide": the depth view is LINEAR, and with
//! `gc_tiled_rt` a depth surface can be SUPER_TILED (9cdeff6d, 9f77146d).
//!
//! Today's behaviour, pinned here: a depth surface a draw can sample is never
//! super-tiled. A sampled depth is a depth *texture*, whose `ds_cache` id has
//! bit 30 set (`gl::context::ds_object_id(serial, true)`, the id
//! `gl::texture::depth_for_draw` passes to `samples_rendered_depth`), and
//! only a surface `ds_cache::pe_private` accepts (no texture plane, no window
//! plane) may leave `Linear`: `vivante::backend::tiled_rt_spec` refuses the
//! tiled shadow for any other draw. So with both knobs on, glmark2 shadow's
//! shape (an RGBA8 colour texture and a DEPTH_COMPONENT texture, depth
//! tested, then sampled) draws its depth LINEAR under SINGLE_BUFFER(1), and
//! the sampling draw's descriptor is the LINEAR D24X8 view of exactly that
//! surface: the right addressing ([`a_sampled_depth_texture_stays_linear_
//! and_mode_1_views_it_linear`]). The control
//! ([`a_depth_renderbuffer_goes_super_tiled_in_the_same_rig`]) shows the
//! knob is live in this rig: the same scene with a depth renderbuffer gets
//! a super-tiled depth surface.
//!
//! The guard is direct as well: `ds_cache::depth_view` (and so
//! `gc_depth_sampled` and `depth_sample::binding`) refuses a plane whose
//! surface is not `Linear`, so a live super-tiled plane (a depth
//! renderbuffer's) is never described LINEAR. No GL call reaches that (a
//! renderbuffer cannot be sampled); [`the_depth_view_refuses_a_super_tiled_
//! plane`] pins the refusal.
//!
//! The rig is tests/gc_tiled_rt.rs's stand-in OSAL (host memory, the
//! submitted streams kept), without its model of the GPU: what is read is the
//! PE words of the depth pass and the descriptor the sampling draw points
//! slot 0 at, out of the stand-in's memory. Built without `report-es2` (the
//! library reports ES 3.0, so a DEPTH_COMPONENT24 texture is core; the unsized
//! DEPTH_COMPONENT / UNSIGNED_INT glmark2 asks for under ES 2.0 leaves this
//! ES 3.0 FBO incomplete, which is not what this file is about).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_depth_view_tiled_rt
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "test-knobs"))]

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
use yakogl::vivante::backend::gc_debug_set_tiled_rt;
use yakogl::vivante::depth_sample::{gc_debug_set_depth_tpu, gc_depth_sampled, DEPTH_TPU_D24X8, GC_TEX_FORMAT_D24X8};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL (tests/gc_tiled_rt.rs's, no GPU model) ---------------------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
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
/// Serialises the tests: the OSAL pointer, the back end, both knobs and
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
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

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
        let words: Vec<u32> =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) }.to_vec();
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

/// The 4 bytes at GPU address `gpu` of the stand-in's memory, as a word.
fn read_word(gpu: u64) -> Option<u32> {
    with_platform(|p| {
        let (base, a) = p.allocs.range(..=gpu).next_back()?;
        let o = (gpu - base) as usize;
        if o + 4 > a.size {
            return None;
        }
        // SAFETY: inside the allocation (checked just above); any alignment.
        Some(unsafe { ((a.mem.as_ptr() as *const u8).add(o) as *const u32).read_unaligned() })
    })
}

/* ---- the stream ---------------------------------------------------------------------- */

/// [`draws`]' state at one DRAW_INSTANCED: every register's latest value in
/// the stream up to it.
type Regs = BTreeMap<u32, u32>;

/// Each DRAW_INSTANCED of `words` with the register values in force at it
/// (tests/gc_tiled_rt.rs's `events`, folded).
fn draws(words: &[u32]) -> Vec<Regs> {
    let mut regs = Regs::new();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    regs.insert(base + 4 * k as u32, *v);
                }
            }
            i += 1 + count;
            continue;
        }
        if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
            out.push(regs.clone());
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1, /* NOP and the pad words */
        };
    }
    out
}

fn reg(r: &Regs, at: u32) -> u32 {
    *r.get(&at).unwrap_or_else(|| panic!("register 0x{at:x} was not written before the draw"))
}

/* ---- the rig ------------------------------------------------------------------------ */

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// `w` x `h` RGBA8888 context; both knobs are put back to 0 at the end.
fn on_gc7000<R>(w: u32, h: u32, case: impl FnOnce() -> R) -> R {
    with_platform(|p| {
        p.bump.clear();
        p.kicks.clear();
    });
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(w, h);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    gc_debug_set_tiled_rt(0);
    gc_debug_set_depth_tpu(0);
    let faults = with_platform(|p| {
        /* leaked, not freed, as in gc_tiled_rt.rs */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SERIAL.lock().unwrap_or_else(|e| e.into_inner())
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

fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

const VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.5, 0.25, 0.125, 1.0); }\n";
/// The sampling pass: glmark2 shadow's read of the depth map, `.x` of a
/// depth texture on unit 0.
const VS_SAMPLE: &str = "attribute vec4 p;\nvarying vec2 uv;\n\
                         void main() { uv = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS_SAMPLE: &str = "precision mediump float;\nuniform sampler2D u_depth;\nvarying vec2 uv;\n\
                         void main() { gl_FragColor = vec4(texture2D(u_depth, uv).x); }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The scene's size: one 64x64 super tile, the size of the depth-view golden.
const W: u32 = 64;
const H: u32 = 64;

/// What the depth pass's attachment is.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Depth {
    /// A DEPTH_COMPONENT24 / UNSIGNED_INT texture, glmark2 shadow's depth map.
    Texture,
    /// A DEPTH_COMPONENT16 renderbuffer: tests/gc_tiled_rt.rs's, the draw W4
    /// takes into a tiled shadow.
    Renderbuffer,
}

/// What a scene left.
struct Run {
    /// The draws of every kick of the depth pass (its depth clear and quad).
    depth_pass: Vec<Regs>,
    /// The sampling pass's draws (empty for [`Depth::Renderbuffer`]).
    sample_pass: Vec<Regs>,
    /// The descriptor slot 0 of the last sampling draw points at, read out
    /// of the stand-in's memory right after its glFinish: `(CONFIG0,
    /// CONFIG1, LINEAR_STRIDE, LOD 0 address)`.
    desc: Option<(u32, u32, u32, u32)>,
    /// `gc_depth_sampled` for every renderbuffer-class id up to
    /// [`ID_SCAN`] that answers yes after the depth pass, mode 1 on.
    rb_ids_answered: Vec<u32>,
}

/// How far [`Run::rb_ids_answered`] looks: object stamps are one counter for
/// the process, and a test binary makes a few hundred objects.
const ID_SCAN: u32 = 1 << 16;

/// With `tiled_rt` and dtpu mode 1 set: an FBO of an RGBA8 colour texture and
/// `depth`, a depth clear and a depth-tested quad, glFinish; then (for a
/// depth texture) a quad into the window that samples it, glFinish.
fn run(depth: Depth, tiled_rt: bool) -> Run {
    on_gc7000(W, H, || {
        gc_debug_set_tiled_rt(tiled_rt as i32);
        gc_debug_set_depth_tpu(DEPTH_TPU_D24X8);
        let mut dt = 0;
        // SAFETY: plain GL calls; null texel data; QUAD is a static.
        unsafe {
            let (mut ct, mut fb) = (0, 0);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenTextures(1, &mut ct);
            glBindTexture(GL_TEXTURE_2D, ct);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            let (fw, fh) = (W as GLsizei, H as GLsizei);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, fw, fh, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, ct, 0);
            match depth {
                Depth::Texture => {
                    glGenTextures(1, &mut dt);
                    glBindTexture(GL_TEXTURE_2D, dt);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        GL_DEPTH_COMPONENT24 as GLint,
                        fw,
                        fh,
                        0,
                        GL_DEPTH_COMPONENT,
                        GL_UNSIGNED_INT,
                        core::ptr::null(),
                    );
                    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, dt, 0);
                }
                Depth::Renderbuffer => {
                    let mut rb = 0;
                    glGenRenderbuffers(1, &mut rb);
                    glBindRenderbuffer(GL_RENDERBUFFER, rb);
                    glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, fw, fh);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
                }
            }
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            glViewport(0, 0, fw, fh);
            glUseProgram(program(VS, FS));
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LEQUAL);
            glClear(GL_DEPTH_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            glFinish();
        }
        no_error("the depth pass");
        let k0 = with_platform(|p| p.kicks.len());
        let depth_pass: Vec<Regs> = with_platform(|p| p.kicks.iter().flat_map(|k| draws(k)).collect());
        /* the plane's own answer, before anything else touches the cache */
        let rb_ids_answered: Vec<u32> = (1..ID_SCAN).filter(|&id| gc_depth_sampled(id, W, H)).collect();
        let (mut sample_pass, mut desc) = (Vec::new(), None);
        if depth == Depth::Texture {
            // SAFETY: plain GL calls.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glDisable(GL_DEPTH_TEST);
                let p = program(VS_SAMPLE, FS_SAMPLE);
                glUseProgram(p);
                glUniform1i(glGetUniformLocation(p, c"u_depth".as_ptr()), 0);
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(GL_TEXTURE_2D, dt);
                glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
                glFinish();
            }
            no_error("the sampling pass");
            sample_pass = with_platform(|p| p.kicks[k0..].iter().flat_map(|k| draws(k)).collect());
            desc = sample_pass.last().and_then(|r| r.get(&nte_descriptor_addr(0)).copied()).map(|d| {
                let at = |off: u32| read_word(u64::from(d) + u64::from(off)).expect("the descriptor is in memory");
                (at(GC_TEXDESC_CONFIG0), at(GC_TEXDESC_CONFIG1), at(GC_TEXDESC_LINEAR_STRIDE), at(texdesc_lod_addr(0)))
            });
        }
        Run { depth_pass, sample_pass, desc, rb_ids_answered }
    })
}

fn super_tiled(r: &Regs) -> bool {
    reg(r, GC_PE_DEPTH_CONFIG) & GC_PE_DEPTH_SUPER_TILED != 0
}

fn single_buffer(r: &Regs) -> u32 {
    (reg(r, GC_PE_LOGIC_OP) >> 8) & 3
}

/* ---- the cases ---------------------------------------------------------------------- */

/// Both knobs on, glmark2 shadow's shape: the depth texture is drawn LINEAR
/// (no shadow: `pe_private` refuses a texture plane), and mode 1 samples it
/// through a LINEAR D24X8 view of that very surface, its row stride the PE's.
/// Deliberate break: drop the texture-bit test from `ds_cache::pe_private`
/// and the depth pass goes super-tiled, which the first assertion names.
#[test]
fn a_sampled_depth_texture_stays_linear_and_mode_1_views_it_linear() {
    let _g = serial();
    let r = run(Depth::Texture, true);
    assert!(!r.depth_pass.is_empty(), "the depth pass drew");
    for (i, d) in r.depth_pass.iter().enumerate() {
        assert!(!super_tiled(d), "depth pass draw {i}: a sampled depth surface is never super-tiled");
        assert_eq!(single_buffer(d), 1, "depth pass draw {i}: no tiled shadow for a depth texture's draw");
    }
    let quad = r.depth_pass.last().unwrap();
    let (depth_addr, depth_stride) = (reg(quad, pe_pipe_depth_addr(0)), reg(quad, GC_PE_DEPTH_STRIDE));
    assert!(r.rb_ids_answered.is_empty(), "no renderbuffer-class id is a live depth plane: {:?}", r.rb_ids_answered);
    assert!(!r.sample_pass.is_empty(), "the sampling pass drew");
    let (c0, c1, stride, lod0) = r.desc.expect("slot 0 has a descriptor at the sampling draw");
    assert_eq!((c0 >> 13) & 0x1f, GC_TEX_FORMAT_D24X8, "mode 1 took the view: FORMAT D24X8, not the R32F texels");
    assert_eq!((c0 >> 20) & 3, GC_TEX_ADDRESSING_LINEAR, "the view is LINEAR, and so is the surface");
    assert_eq!((c1 >> 26) & 7, 0, "HALIGN 0: no super-tiled level");
    assert_eq!(lod0, depth_addr, "LOD 0 is the depth pass's PE_PIPE_DEPTH_ADDR");
    assert_eq!(stride, depth_stride, "LINEAR_STRIDE is the PE's depth row stride");
}

/// The rig's control: with the tiled targets off the depth texture's pass is
/// the same LINEAR draw and the same view, so the knob changes nothing for a
/// sampled depth.
#[test]
fn the_tiled_knob_does_not_move_a_sampled_depth() {
    let _g = serial();
    let off = run(Depth::Texture, false);
    let on = run(Depth::Texture, true);
    let words =
        |r: &Run| r.depth_pass.iter().map(|d| (reg(d, GC_PE_DEPTH_CONFIG), single_buffer(d))).collect::<Vec<_>>();
    assert_eq!(words(&on), words(&off), "PE_DEPTH_CONFIG and SINGLE_BUFFER of every depth-pass draw");
    let view = |r: &Run| r.desc.map(|(c0, c1, s, _)| (c0, c1, s));
    assert_eq!(view(&on), view(&off), "the view's CONFIG0, CONFIG1 and LINEAR_STRIDE");
    assert!(on.desc.is_some());
}

/// The control that the knob is live in this rig: a depth renderbuffer in
/// the same scene is drawn into a super-tiled depth surface under a tiled
/// shadow (tests/gc_tiled_rt.rs (2)).
#[test]
fn a_depth_renderbuffer_goes_super_tiled_in_the_same_rig() {
    let _g = serial();
    let r = run(Depth::Renderbuffer, true);
    let quad = r.depth_pass.last().expect("the depth pass drew");
    assert!(super_tiled(quad), "a depth renderbuffer's surface is super-tiled with gc_tiled_rt");
    assert_eq!(single_buffer(quad), 2, "under the tiled shadow");
}

/// The rule gc-depth-sample-wire.md asks for ("What this note does not
/// decide", last bullet): the depth view reports the surface's layout, or
/// refuses one it does not describe. `gc_depth_sampled` (through
/// `ds_cache::depth_view`) looks at no layout, so it answers yes for the
/// live super-tiled plane of the renderbuffer above, and `binding` would
/// describe that surface LINEAR. Unreachable from GL today (only a depth
/// texture's id is asked, and a texture plane is never super-tiled), so no
/// wrong texel is drawn; this pins the latent defect for the fix.
#[test]
fn the_depth_view_refuses_a_super_tiled_plane() {
    let _g = serial();
    let r = run(Depth::Renderbuffer, true);
    assert!(super_tiled(r.depth_pass.last().unwrap()), "the plane is super-tiled");
    assert!(
        r.rb_ids_answered.is_empty(),
        "dtpu mode 1 answers yes for the super-tiled plane(s) {:?}: a LINEAR view of a SUPER_TILED surface",
        r.rb_ids_answered
    );
}
