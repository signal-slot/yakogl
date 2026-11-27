// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): W10 of docs/notes/gc-tiled-wire.md, a single-level
//! texture sampled straight from its TILED shadow (`gc_tiled_sample`,
//! `gc_debug_set_tiled_sample`, a test knob, default off; it needs
//! `gc_tiled_rt` on), seen through the GL API.
//!
//! The scene: a depth-tested quad into a framebuffer object (a 64x64 RGBA8
//! texture with a DEPTH_COMPONENT16 renderbuffer, the draw W4 gives a
//! shadow), then a quad into the window whose fragment shader samples that
//! texture, then glFinish and glReadPixels of the texture. Both draws are in
//! one stream.
//!
//! - `gc_tiled_rt` on, `gc_tiled_sample` off: the stream of before this item.
//!   The RS copies the shadow back into the storage before the window's draw,
//!   which samples the linear storage.
//! - Both on: the window's draw samples the shadow (TILED, HALIGN SIXTEEN,
//!   level 0 the shadow's address, TX_CTRL 0) and the copy back moves to the
//!   end of the stream: no tile-out copy before the sample.
//! - With the sample knob off the streams are the ones of a run that never
//!   set it, and the sample knob alone (gc_tiled_rt off) changes nothing.
//!
//! How it runs: `gc_tiled_rt.rs`'s rig (the real GL core and Vivante back end
//! over a stand-in OSAL, with a model of the RS and the PE run over each
//! submitted stream), whose model also reads, at each draw, the 64x64 texels
//! sampler slot 0's descriptor names (level 0, in the layout CONFIG0 says).
//! So "the sampled texels match" means: what the sampler would read is what
//! the model's PE painted into the texture, and what glReadPixels returns. It
//! shows the words agree with each other, not what the hardware does (that is
//! an X2 session's).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,test-knobs --test gc_tiled_sample
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
use yakogl::vivante::backend::{gc_debug_set_tiled_rt, gc_debug_set_tiled_sample};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL (gc_tiled_rt.rs's) --------------------------------------------- */

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

/// What sampler slot 0 read at one draw: its descriptor's level 0, CONFIG0,
/// CONFIG1 and TX_CTRL, and the 64x64 texels there.
#[derive(Clone, Debug, PartialEq)]
struct Sampled {
    lod0: u32,
    config0: u32,
    config1: u32,
    tx_ctrl: u32,
    texels: Vec<u32>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
    /// Run the model over each stream; the PE paints a `w` x `h` window.
    paint: Option<(u32, u32)>,
    /// Each draw's slot-0 read, in stream order (`None`: no descriptor yet).
    sampled: Vec<Option<Sampled>>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end, the knobs and
/// `g_backend` are process-global. Holds whether the warm-up ran.
static SERIAL: Mutex<bool> = Mutex::new(false);

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
        if let Some(size) = p.paint {
            run_model(p, &words, size);
        }
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

/* ---- the stream ---------------------------------------------------------------------- */

/// Every LOAD_STATE write of the stream, `(register, value, word index)`, and
/// each DRAW_INSTANCED as `(DRAW, 0, word index)`, in order.
fn events(words: &[u32]) -> Vec<(u32, u32, usize)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v, i));
                }
            }
            i += 1 + count;
            continue;
        }
        if w & 0xf800_0000 == GC_OP_DRAW_INSTANCED {
            out.push((DRAW, 0, i));
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

const DRAW: u32 = u32::MAX;

/// Word index of each DRAW_INSTANCED.
fn draws_at(words: &[u32]) -> Vec<usize> {
    events(words).iter().filter(|(r, _, _)| *r == DRAW).map(|(_, _, i)| *i).collect()
}

/// The value `reg` had at draw `n` of the stream.
fn reg_at_draw_n(words: &[u32], n: usize, reg: u32) -> Option<u32> {
    let ev = events(words);
    let d = ev.iter().enumerate().filter(|(_, (r, _, _))| *r == DRAW).nth(n).map(|(i, _)| i)?;
    ev[..d].iter().rev().find(|(r, _, _)| *r == reg).map(|(_, v, _)| *v)
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct RsJob {
    config: u32,
    src: u32,
    src_stride: u32,
    dst: u32,
    dst_stride: u32,
    window: u32,
    clear_control: u32,
    fill: u32,
    at: usize,
}

impl RsJob {
    fn note(&mut self, reg: u32, v: u32) {
        match reg {
            GC_RS_CONFIG => self.config = v,
            GC_RS_SOURCE_STRIDE => self.src_stride = v,
            GC_RS_DEST_STRIDE => self.dst_stride = v,
            GC_RS_WINDOW_SIZE => self.window = v,
            GC_RS_CLEAR_CONTROL => self.clear_control = v,
            r if r == rs_fill_value(0) => self.fill = v,
            r if r == rs_pipe_source_addr(0) => self.src = v,
            r if r == rs_pipe_dest_addr(0) => self.dst = v,
            _ => {}
        }
    }
}

fn rs_jobs(words: &[u32]) -> Vec<RsJob> {
    let mut cur = RsJob::default();
    let mut out = Vec::new();
    for (reg, v, i) in events(words) {
        cur.note(reg, v);
        if reg == GC_RS_KICKER {
            out.push(RsJob { at: i, ..cur });
        }
    }
    out
}

/// The RS copies of a stream (fills left out).
fn rs_copies(words: &[u32]) -> Vec<RsJob> {
    rs_jobs(words).into_iter().filter(|j| j.clear_control & GC_RS_CLEAR_CONTROL_MODE_ENABLED1 == 0).collect()
}

/* ---- the model of the GPU (gc_tiled_rt.rs's, and the sampler's read) ----------------- */

fn offset(tiled: bool, x: u32, y: u32, stride: u32) -> usize {
    if tiled {
        let tile = (y / 4) * (stride / 16) + x / 4;
        (tile * 64 + ((y % 4) * 4 + x % 4) * 4) as usize
    } else {
        (y * stride + x * 4) as usize
    }
}

fn paint(x: u32, y: u32) -> u32 {
    0xff00_0000 | (y & 0x3ff) << 10 | (x & 0x3ff)
}

fn word_at(p: &mut Platform, gpu: u64, off: usize) -> Option<&mut [u8; 4]> {
    let at = gpu + off as u64;
    let (base, a) = p.allocs.range_mut(..=at).next_back()?;
    let o = (at - base) as usize;
    if o + 4 > a.size {
        return None;
    }
    // SAFETY: inside the allocation (checked just above); any alignment.
    Some(unsafe { &mut *((a.mem.as_mut_ptr() as *mut u8).add(o) as *mut [u8; 4]) })
}

fn read_u32(p: &mut Platform, gpu: u64) -> Option<u32> {
    word_at(p, gpu, 0).map(|w| u32::from_ne_bytes(*w))
}

/// Sampler slot 0 at a draw: the descriptor at NTE_DESCRIPTOR_ADDR(0), and
/// the `T` x `T` texels at its level 0 in the layout CONFIG0's addressing
/// names (TILED: 4x4 tiles, LINEAR_STRIDE a row of the padded width).
fn sample_slot0(p: &mut Platform, regs: &BTreeMap<u32, u32>) -> Option<Sampled> {
    let desc = u64::from(*regs.get(&nte_descriptor_addr(0))?);
    let lod0 = read_u32(p, desc + u64::from(texdesc_lod_addr(0)))?;
    let config0 = read_u32(p, desc + u64::from(GC_TEXDESC_CONFIG0))?;
    let config1 = read_u32(p, desc + u64::from(GC_TEXDESC_CONFIG1))?;
    let stride = read_u32(p, desc + u64::from(GC_TEXDESC_LINEAR_STRIDE))?;
    let tiled = config0 & texdesc_addressing_mode(!0) == texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED);
    let mut texels = Vec::with_capacity((T * T) as usize);
    for y in 0..T {
        for x in 0..T {
            texels.push(read_u32(p, u64::from(lod0) + offset(tiled, x, y, stride) as u64)?);
        }
    }
    let tx_ctrl = regs.get(&nte_descriptor_tx_ctrl(0)).copied().unwrap_or(u32::MAX);
    Some(Sampled { lod0, config0, config1, tx_ctrl, texels })
}

fn run_model(p: &mut Platform, words: &[u32], (w, h): (u32, u32)) {
    let mut regs: BTreeMap<u32, u32> = BTreeMap::new();
    let mut rs = RsJob::default();
    for (reg, v, _) in events(words) {
        if reg == DRAW {
            let s = sample_slot0(p, &regs);
            p.sampled.push(s);
            let reg = |r: u32| regs.get(&r).copied().unwrap_or(0);
            let (addr, stride) = (u64::from(reg(pe_pipe_color_addr(0))), reg(GC_PE_COLOR_STRIDE));
            if reg(GC_PE_COLOR_FORMAT) & (GC_PE_COLOR_FORMAT_SUPER_TILED | GC_PE_COLOR_FORMAT_SUPER_TILED_NEW) != 0 {
                p.faults.push("the model paints no super-tiled colour surface".into());
                continue;
            }
            let tiled = (reg(GC_PE_LOGIC_OP) >> 8) & 3 != 1;
            for y in 0..h {
                for x in 0..w {
                    match word_at(p, addr, offset(tiled, x, y, stride)) {
                        Some(px) => *px = paint(x, y).to_ne_bytes(),
                        None => {
                            p.faults.push(format!("the PE painted ({x},{y}) past its surface at 0x{addr:x}"));
                            return;
                        }
                    }
                }
            }
            continue;
        }
        regs.insert(reg, v);
        rs.note(reg, v);
        if reg == GC_RS_KICKER {
            rs_model(p, &rs);
        }
    }
}

fn rs_model(p: &mut Platform, j: &RsJob) {
    let (w, h) = (j.window & 0xffff, j.window >> 16);
    let (st, dt) = (j.config & GC_RS_SOURCE_TILED != 0, j.config & GC_RS_DEST_TILED != 0);
    let side = |tiled: bool, stride: u32| if tiled { stride >> 2 } else { stride };
    let (ss, ds) = (side(st, j.src_stride), side(dt, j.dst_stride));
    let fill = j.clear_control & GC_RS_CLEAR_CONTROL_MODE_ENABLED1 != 0;
    for y in 0..h {
        for x in 0..w {
            let v = if fill {
                j.fill.to_ne_bytes()
            } else {
                match word_at(p, u64::from(j.src), offset(st, x, y, ss)) {
                    Some(v) => *v,
                    None => {
                        p.faults.push(format!("the RS read ({x},{y}) past its source 0x{:x}", j.src));
                        return;
                    }
                }
            };
            match word_at(p, u64::from(j.dst), offset(dt, x, y, ds)) {
                Some(d) => *d = v,
                None => {
                    p.faults.push(format!("the RS wrote ({x},{y}) past its destination 0x{:x}", j.dst));
                    return;
                }
            }
        }
    }
}

/* ---- the rig ------------------------------------------------------------------------ */

/// The texture's and the window's size.
const T: u32 = 64;

/// The knobs of one run: `None` is never set by the run.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Knobs {
    tiled_rt: Option<bool>,
    tiled_sample: Option<bool>,
}

/// What comes after the window's sampling draw in the same stream.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Then {
    Nothing,
    /// A draw into the window that samples nothing: it needs the storage
    /// current, so the copy back goes before it.
    PlainWindowDraw,
    /// The texture drawn into again: the shadow is taken back with no tile-in.
    DrawTextureAgain,
}

struct Run {
    kicks: Vec<Vec<u32>>,
    /// Slot 0 at each draw of the run.
    sampled: Vec<Option<Sampled>>,
    /// The texture read back after glFinish.
    pixels: Vec<u8>,
    /// The texture's storage (PE address of the knob-off draw into it).
    storage: Option<u32>,
}

fn run(k: Knobs, then: Then) -> Run {
    with_platform(|p| {
        p.bump.clear();
        p.kicks.clear();
        p.sampled.clear();
        p.paint = Some((T, T));
    });
    // SAFETY: the caller holds SERIAL; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(T, T);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| scene(k, then)));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    gc_debug_set_tiled_rt(0);
    gc_debug_set_tiled_sample(0);
    let (faults, kicks, sampled) = with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        p.paint = None;
        (core::mem::take(&mut p.faults), core::mem::take(&mut p.kicks), core::mem::take(&mut p.sampled))
    });
    let pixels = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    Run { kicks, sampled, pixels, storage: None }
}

fn serial() -> std::sync::MutexGuard<'static, bool> {
    let mut g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    if !*g {
        run(Knobs { tiled_rt: None, tiled_sample: None }, Then::Nothing);
        *g = true;
    }
    g
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

const VS: &str = "attribute vec4 p;\nvarying vec2 uv;\nvoid main() { gl_Position = p; uv = p.xy * 0.5 + 0.5; }\n";
const FS_FLAT: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.5, 0.25, 0.125, 1.0); }\n";
const FS_SAMPLE: &str =
    "precision mediump float;\nvarying vec2 uv;\nuniform sampler2D tex;\nvoid main() { gl_FragColor = texture2D(tex, uv); }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The scene; returns the texture read back.
fn scene(k: Knobs, then: Then) -> Vec<u8> {
    if let Some(on) = k.tiled_rt {
        gc_debug_set_tiled_rt(on as i32);
    }
    if let Some(on) = k.tiled_sample {
        gc_debug_set_tiled_sample(on as i32);
    }
    let (mut t, mut rb, mut fb) = (0, 0, 0);
    // SAFETY: plain GL calls; null texel data; QUAD is a static.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        let s = T as GLsizei;
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, s, s, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, s, s);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, s, s);
        let flat = program(VS, FS_FLAT);
        let sample = program(VS, FS_SAMPLE);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        no_error("setup");

        /* into the texture, depth-tested: W4's shadow */
        glUseProgram(flat);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LEQUAL);
        glClear(GL_DEPTH_BUFFER_BIT);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        /* into the window, sampling the texture */
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_DEPTH_TEST);
        glUseProgram(sample);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t);
        glUniform1i(glGetUniformLocation(sample, c"tex".as_ptr()), 0);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        match then {
            Then::Nothing => {}
            Then::PlainWindowDraw => {
                glUseProgram(flat);
                glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            }
            Then::DrawTextureAgain => {
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glEnable(GL_DEPTH_TEST);
                glUseProgram(flat);
                glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
            }
        }
        glFinish();
        no_error("the frame");
        let mut pixels = vec![0u8; (T * T * 4) as usize];
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glPixelStorei(GL_PACK_ALIGNMENT, 1);
        glReadPixels(0, 0, s, s, GL_RGBA, GL_UNSIGNED_BYTE, pixels.as_mut_ptr().cast());
        no_error("glReadPixels");
        pixels
    }
}

/* ---- what the tests read ------------------------------------------------------------ */

const ON: Knobs = Knobs { tiled_rt: Some(true), tiled_sample: Some(true) };
const RT_ONLY: Knobs = Knobs { tiled_rt: Some(true), tiled_sample: Some(false) };

impl Run {
    /// The run's one kick: both draws are in one stream.
    fn kick(&self) -> &[u32] {
        assert_eq!(self.kicks.len(), 1, "one kick: {} streams", self.kicks.len());
        &self.kicks[0]
    }

    /// Slot 0 at draw `n` of the run (the run is one kick).
    fn sampled_at(&self, n: usize) -> &Sampled {
        self.sampled[n].as_ref().unwrap_or_else(|| panic!("draw {n} reads no descriptor"))
    }

    /// The texture's storage: where the shadow's copy back writes.
    fn with_storage(mut self) -> Self {
        self.storage = rs_copies(&self.kicks[0]).iter().find(|j| j.config & GC_RS_SOURCE_TILED != 0).map(|j| j.dst);
        self
    }
}

/// The index of the window's sampling draw: the first draw whose slot 0
/// reads 64x64 texels (the clear and the texture's draw bind none).
fn sampling_draw(r: &Run) -> usize {
    r.sampled.iter().position(|s| s.as_ref().is_some_and(|s| s.texels.len() == (T * T) as usize)).expect("a sample")
}

/// The model's picture in its own coordinates: what its PE paints at each
/// (x, y), so what a sampler reading the texture's texels must find.
fn painted() -> Vec<u32> {
    (0..T).flat_map(|y| (0..T).map(move |x| paint(x, y))).collect()
}

/// The texture read back by a run with neither knob set: the picture of
/// before W4 (glReadPixels' row order and byte order, not the model's).
fn baseline() -> Vec<u8> {
    run(Knobs { tiled_rt: None, tiled_sample: None }, Then::Nothing).pixels
}

/* ---- the cases ---------------------------------------------------------------------- */

/// (1) The knob's default is off, and with it off the stream is today's:
/// `gc_tiled_rt` on with `gc_tiled_sample` off records the words of a run
/// that never set the sample knob, and the sample knob alone (gc_tiled_rt
/// off) records the words of a run that set neither.
#[test]
fn knob_off_records_todays_stream() {
    let _g = serial();
    // SAFETY: a plain read of an int; SERIAL is held.
    assert_eq!(unsafe { core::ptr::addr_of!(yakogl::vivante::backend::gc_debug_tiled_sample).read() }, 0);
    let rt_never = run(Knobs { tiled_rt: Some(true), tiled_sample: None }, Then::Nothing);
    let _ = run(ON, Then::Nothing);
    let rt_off = run(RT_ONLY, Then::Nothing);
    assert_eq!(rt_off.kick(), rt_never.kick(), "gc_tiled_sample off changed the knob-on stream");
    assert_eq!(rt_off.pixels, rt_never.pixels);

    let never = run(Knobs { tiled_rt: None, tiled_sample: None }, Then::Nothing);
    let alone = run(Knobs { tiled_rt: None, tiled_sample: Some(true) }, Then::Nothing);
    assert_eq!(alone.kick(), never.kick(), "gc_tiled_sample without gc_tiled_rt changed the stream");
    assert_eq!(alone.pixels, never.pixels);
}

/// (2) Sample knob off, gc_tiled_rt on: the copy back of the shadow comes
/// before the window's draw, which samples the linear storage.
#[test]
fn knob_off_copies_the_shadow_back_before_the_sample() {
    let _g = serial();
    let r = run(RT_ONLY, Then::Nothing).with_storage();
    let k = r.kick();
    let storage = r.storage.expect("the copy back names the storage");
    let n = sampling_draw(&r);
    let at = draws_at(k)[n];
    let back: Vec<_> = rs_copies(k).into_iter().filter(|j| j.config & GC_RS_SOURCE_TILED != 0).collect();
    assert_eq!(back.len(), 1, "one copy back: {back:x?}");
    assert!(back[0].at < at, "the copy back before the sample");
    let s = r.sampled_at(n);
    assert_eq!(s.lod0, storage, "the linear storage is sampled");
    assert_eq!(s.config0 & texdesc_addressing_mode(!0), texdesc_addressing_mode(GC_TEX_ADDRESSING_LINEAR));
    assert!(s.texels == painted(), "the sampled texels are the texture's");
    assert!(r.pixels == baseline(), "glReadPixels of the texture");
}

/// (3) Both knobs on: the window's draw samples the shadow, with
/// tiled_rt::sampler_words_of's words (TILED, HALIGN SIXTEEN, a 64-texel
/// row of 4 bytes, level 0 the shadow the texture's draw rendered into, TX_CTRL
/// 0); no tile-out copy comes before it, the copy back ends the stream. What
/// the sampler reads there is what the PE painted, and glReadPixels returns
/// the knob-off picture.
///
/// Deliberate break: drop the `Phase::Sampled` arm of `tiled_rt_enter` and
/// the copy back comes before the sample again.
#[test]
fn knob_on_samples_the_shadow_with_no_copy_back_before_it() {
    let _g = serial();
    let off = run(RT_ONLY, Then::Nothing).with_storage();
    let on = run(ON, Then::Nothing).with_storage();
    let k = on.kick();
    let n = sampling_draw(&on);
    assert_eq!(n, sampling_draw(&off), "the same draws");
    let at = draws_at(k)[n];
    let shadow = reg_at_draw_n(k, n - 1, pe_pipe_color_addr(0)).expect("the texture's draw");
    assert_eq!(on.storage, off.storage, "the same storage");
    let storage = on.storage.expect("the copy back names the storage");
    assert_ne!(shadow, storage, "the texture's draw renders into the shadow");

    let copies = rs_copies(k);
    let back: Vec<_> = copies.iter().filter(|j| j.config & GC_RS_SOURCE_TILED != 0).collect();
    assert_eq!(back.len(), 1, "one copy back: {copies:x?}");
    assert!(back[0].at > at, "no tile-out copy before the sample: {copies:x?}");
    assert_eq!((back[0].src, back[0].dst), (shadow, storage));
    assert!(!copies.iter().any(|j| j.at > draws_at(k)[n - 1] && j.at < at), "no RS copy between render and sample");
    assert_eq!(copies.len(), rs_copies(off.kick()).len(), "the same copies, the copy back moved");

    let s = on.sampled_at(n);
    assert_eq!(s.lod0, shadow, "level 0 is the shadow");
    assert_eq!(s.config0 & texdesc_addressing_mode(!0), texdesc_addressing_mode(GC_TEX_ADDRESSING_TILED));
    assert_eq!(s.config1 & texdesc_halign(!0), texdesc_halign(GC_TEXTURE_HALIGN_SIXTEEN));
    assert_eq!(s.tx_ctrl, 0, "no tile status");
    let o = off.sampled_at(n);
    assert_eq!(s.config1 & !texdesc_halign(!0), o.config1 & !texdesc_halign(!0), "the format words but HALIGN");
    assert!(s.texels == painted(), "the shadow's texels sampled tiled are the texture's");
    assert!(s.texels == o.texels, "the knob-off sample");
    assert!(on.pixels == off.pixels, "glReadPixels of the texture");
    assert!(on.pixels == baseline(), "glReadPixels of the texture, neither knob set");
}

/// (4) Both knobs on, and a draw into the window that samples nothing after
/// the sampling one: the copy back the sample left owed goes before it, once.
#[test]
fn a_draw_that_samples_nothing_of_the_shadow_finds_the_storage_current() {
    let _g = serial();
    let r = run(ON, Then::PlainWindowDraw).with_storage();
    let k = r.kick();
    let n = sampling_draw(&r);
    let draws = draws_at(k);
    let back: Vec<_> = rs_copies(k).into_iter().filter(|j| j.config & GC_RS_SOURCE_TILED != 0).collect();
    assert_eq!(back.len(), 1, "one copy back: {back:x?}");
    assert!(back[0].at > draws[n] && back[0].at < draws[n + 1], "between the sample and the plain draw");
    assert!(r.sampled_at(n).texels == painted());
    assert!(r.pixels == baseline());
}

/// (5) Both knobs on, and the texture drawn into again after the sample: the
/// shadow, which holds the newest pixels, is taken back with no tile-in and
/// no copy back in between; one copy back ends the stream.
#[test]
fn the_texture_drawn_again_after_the_sample_takes_the_shadow_back_with_no_tile_in() {
    let _g = serial();
    let off = run(RT_ONLY, Then::DrawTextureAgain).with_storage();
    let on = run(ON, Then::DrawTextureAgain).with_storage();
    let k = on.kick();
    let n = sampling_draw(&on);
    let draws = draws_at(k);
    let copies = rs_copies(k);
    let shadow = reg_at_draw_n(k, n - 1, pe_pipe_color_addr(0)).unwrap();
    assert_eq!(reg_at_draw_n(k, n + 1, pe_pipe_color_addr(0)), Some(shadow), "the same shadow again");
    assert!(
        !copies.iter().any(|j| j.at > draws[n - 1] && j.at < draws[n + 1]),
        "no RS copy around the sample: {copies:x?}"
    );
    let back: Vec<_> = copies.iter().filter(|j| j.config & GC_RS_SOURCE_TILED != 0).collect();
    assert_eq!(back.len(), 1, "one copy back: {copies:x?}");
    assert!(back[0].at > draws[n + 1]);
    assert!(rs_copies(off.kick()).len() > copies.len(), "knob off copies back and tiles in again around the sample");
    assert!(on.pixels == off.pixels);
    assert!(on.pixels == baseline());
}
