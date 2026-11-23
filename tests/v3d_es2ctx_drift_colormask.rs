// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the V3D colour write mask per render target of
//! 0ebbd507 (`V3dJobState::rt_colormask_own`, `rt_colormask`, v3d_job.c packs
//! one nibble per RT) and 48f6017c (`GlesDraw::per_target_mask`,
//! `target_color_mask`, draw.rs `per_target_masks`, state.rs `job_state`).
//! The Pi 4 whole dEQP-GLES2 tree (46f93767) predates both, and dEQP-GLES2's
//! color_mask and blend cases draw to one target under glColorMask. ES 2.0
//! has one colour target, so nothing may be handed over as per-target masks.
//! Held, through the real GL core (a `report-es2` build) and the real V3D
//! state code, with the COLOR_WRITE_MASKS word built by the real
//! driver/src/v3d/v3d_job.c and decoded with Mesa's v3d_packet.xml:
//!
//! (a) one RGBA8, RGB565, RGBA4, RGB5_A1 target as a renderbuffer, and the
//!     texture formats ES 2.0 can render to (RGBA8, RGB8, RGB565, RGBA4,
//!     RGB5_A1), under every one of the sixteen glColorMask subsets, with
//!     and without GL_BLEND: the draw carries no per-target mask
//!     (`per_target_mask` false, `target_color_mask` all 0), the job state
//!     has `rt_colormask_own` false and `rt_colormask` 0, RT 0's `colormask`
//!     is the application's mask without alpha where the target has no alpha
//!     bits, and the COLOR_WRITE_MASKS word is the one the parent of 0ebbd507
//!     wrote, `off | off << 4 | off << 8 | off << 12` with
//!     `off = ~colormask & 0xf` (written out in `pre_change_word`, not taken
//!     from the new code);
//! (b) the default framebuffer, the same sixteen masks, blend on and off;
//! (c) glColorMask changed between draws of one frame is applied per draw,
//!     in two orders, each draw's word from its own mask;
//! (d) the ES 3.0-only refusal of src/gl/draw.rs `per_target_masks` (uneven
//!     masks under blending with integer and non-integer targets; 48f6017c
//!     put it in place of the older `refuse_uneven_target_masks`) is never
//!     reached from the ES 2.0 entry points: every mask under blending is
//!     taken with one target and no refusal. A second target needs
//!     glDrawBuffers or GL_COLOR_ATTACHMENT1, which are ES 3.0 and which the
//!     `report-es2` build still has; this test does not call them.
//!
//! What it cannot see: the GPU. The back end is a stand-in that records what
//! `state::refusal`, `state::unsupported` and `state::job_state` make of
//! each draw; nothing is rasterized and nothing here ran on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_colormask
//! cargo clippy --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_colormask
//! ```
//!
//! The feature set is `V3D_ES2_CONFIG` of tools/check.sh. The first run
//! compiles driver/src/v3d/v3d_job.c with `cc`; a cold build of the crate
//! takes 30 to 60 minutes.

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", feature = "report-es2"))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::ffi::*;
use yakogl::v3d::state;

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
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

/// What the back end was handed by one draw, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    num_targets: u8,
    per_target_mask: bool,
    target_color_mask: [u8; GLES_MAX_TARGETS],
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`: the job's state, or its refusal.
    job: Result<V3dJobState, &'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.index_count == 0 {
        return 0;
    }
    SEEN.lock().unwrap().push(Seen {
        num_targets: d.num_targets,
        per_target_mask: d.per_target_mask,
        target_color_mask: d.target_color_mask,
        refused: state::refusal(d).or_else(|| state::unsupported(d)),
        job: state::job_state(d),
    });
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
    name: c"v3d-es2ctx-drift-colormask".as_ptr() as *const c_char,
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
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- a framebuffer, a program, a draw ------------------------------------ */

/// A target an ES 2.0 framebuffer can have: its internal format (a
/// renderbuffer) or format and type (a texture), and whether it has alpha.
#[derive(Clone, Copy)]
struct Target {
    name: &'static str,
    texture: bool,
    /// Renderbuffer internal format, or texture format.
    ifmt: GLenum,
    /// Texture type (unused for a renderbuffer).
    ty: GLenum,
    alpha: bool,
}

const TARGETS: [Target; 9] = [
    Target { name: "RGBA8 renderbuffer", texture: false, ifmt: GL_RGBA8, ty: 0, alpha: true },
    Target { name: "RGB565 renderbuffer", texture: false, ifmt: GL_RGB565, ty: 0, alpha: false },
    Target { name: "RGBA4 renderbuffer", texture: false, ifmt: GL_RGBA4, ty: 0, alpha: true },
    Target { name: "RGB5_A1 renderbuffer", texture: false, ifmt: GL_RGB5_A1, ty: 0, alpha: true },
    Target { name: "RGBA8 texture", texture: true, ifmt: GL_RGBA, ty: GL_UNSIGNED_BYTE, alpha: true },
    Target { name: "RGB8 texture", texture: true, ifmt: GL_RGB, ty: GL_UNSIGNED_BYTE, alpha: false },
    Target { name: "RGB565 texture", texture: true, ifmt: GL_RGB, ty: GL_UNSIGNED_SHORT_5_6_5, alpha: false },
    Target { name: "RGBA4 texture", texture: true, ifmt: GL_RGBA, ty: GL_UNSIGNED_SHORT_4_4_4_4, alpha: true },
    Target { name: "RGB5_A1 texture", texture: true, ifmt: GL_RGBA, ty: GL_UNSIGNED_SHORT_5_5_5_1, alpha: true },
];

/// A bound framebuffer; dropped, it unbinds and deletes what it made.
struct Fbo {
    fb: GLuint,
    tex: GLuint,
    rb: GLuint,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(1, &self.tex);
            glDeleteRenderbuffers(1, &self.rb);
            glUseProgram(0);
        }
    }
}

fn fbo(t: Target) -> Fbo {
    let mut f = Fbo { fb: 0, tex: 0, rb: 0 };
    // SAFETY: room for one name each; the attachment named is there.
    unsafe {
        glGenFramebuffers(1, &mut f.fb);
        glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
        if t.texture {
            glGenTextures(1, &mut f.tex);
            glBindTexture(GL_TEXTURE_2D, f.tex);
            glTexImage2D(GL_TEXTURE_2D, 0, t.ifmt as GLint, 8, 8, 0, t.ifmt, t.ty, core::ptr::null());
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, f.tex, 0);
        } else {
            glGenRenderbuffers(1, &mut f.rb);
            glBindRenderbuffer(GL_RENDERBUFFER, f.rb);
            glRenderbufferStorage(GL_RENDERBUFFER, t.ifmt, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, f.rb);
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{}: storage and attach", t.name);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE, "{}", t.name);
    f
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

/// A GLSL ES 1.00 program writing gl_FragColor, current.
fn program() {
    let v = shader(GL_VERTEX_SHADER, "attribute vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0); }\n",
    );
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program did not link");
        glUseProgram(p);
    }
}

/// Draw one triangle; what the back end was handed (None: no draw reached it)
/// and the GL error the call left.
fn draw() -> (Option<Seen>, GLenum) {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let e = err();
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    (s.into_iter().last(), e)
}

fn color_mask(m: u8) {
    // SAFETY: plain GL call.
    unsafe { glColorMask(m & 1, (m >> 1) & 1, (m >> 2) & 1, (m >> 3) & 1) };
}

fn blend(on: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(GL_BLEND);
            glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        } else {
            glDisable(GL_BLEND);
        }
    }
}

/// RT 0's mask before 0ebbd507: the application's glColorMask, without alpha
/// where the target has no alpha bits (state.rs `colormask &= 0x7`).
fn pre_change_colormask(app: u8, alpha: bool) -> u32 {
    u32::from(app & if alpha { 0xf } else { 0x7 })
}

/// The COLOR_WRITE_MASKS "Mask" the parent of 0ebbd507 wrote (v3d_job.c
/// emit_state): `off | off << 4 | off << 8 | off << 12`, `off` the disable
/// bits of RT 0's mask, every RT taking it.
fn pre_change_word(colormask: u32) -> u32 {
    let off = !colormask & 0xf;
    off | off << 4 | off << 8 | off << 12
}

/// One draw under glColorMask `app` on a target with or without alpha: taken,
/// no per-target mask anywhere, RT 0's mask and the word as before.
fn expect_unchanged(what: &str, app: u8, alpha: bool) {
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{what}");
    let s = seen.unwrap_or_else(|| panic!("{what}: the back end saw no draw"));
    assert_eq!(s.refused, None, "{what}");
    assert!(s.num_targets <= 1, "{what}: one colour target, not {}", s.num_targets);
    assert!(!s.per_target_mask, "{what}: no per-target mask");
    assert_eq!(s.target_color_mask, [0; GLES_MAX_TARGETS], "{what}: no per-target masks");
    let st = s.job.unwrap_or_else(|why| panic!("{what}: job_state refuses: {why}"));
    let want = pre_change_colormask(app, alpha);
    assert_eq!(st.colormask, want, "{what}: RT 0's mask");
    assert!(!st.rt_colormask_own, "{what}: no own RT masks");
    assert_eq!(st.rt_colormask, 0, "{what}: RT 1..3 masks");
    assert_eq!(color_write_masks(&st), pre_change_word(want), "{what}: COLOR_WRITE_MASKS");
}

/* ---- the cases ------------------------------------------------------------ */

/// (a) every ES 2.0 target format, as a renderbuffer or a texture, under the
/// sixteen masks, with and without blending.
#[test]
fn one_target_of_every_format_takes_one_mask_as_before() {
    let _g = context();
    for t in TARGETS {
        let _f = fbo(t);
        program();
        for b in [false, true] {
            blend(b);
            for m in 0..16u8 {
                color_mask(m);
                expect_unchanged(&format!("{}, mask {m:#x}, blend {b}", t.name), m, t.alpha);
            }
            color_mask(0xf);
        }
        blend(false);
    }
}

/// (b) the default framebuffer.
#[test]
fn the_default_framebuffer_takes_one_mask_as_before() {
    let _g = context();
    program();
    let mut alpha_bits: GLint = -1;
    // SAFETY: plain GL call; one integer.
    unsafe { glGetIntegerv(GL_ALPHA_BITS, &mut alpha_bits) };
    assert!(alpha_bits >= 0, "GL_ALPHA_BITS answers");
    for b in [false, true] {
        blend(b);
        for m in 0..16u8 {
            color_mask(m);
            expect_unchanged(&format!("default framebuffer, mask {m:#x}, blend {b}"), m, alpha_bits > 0);
        }
    }
    blend(false);
    // SAFETY: plain GL call.
    unsafe { glUseProgram(0) };
}

/// (c) glColorMask changed between draws of one frame, with no clear or bind
/// between: each draw has its own mask and word, whichever order.
#[test]
fn a_mask_changed_between_draws_is_applied_per_draw() {
    let _g = context();
    for t in [TARGETS[0], TARGETS[1], TARGETS[4]] {
        let _f = fbo(t);
        program();
        for seq in [[0xfu8, 0x1, 0xe, 0x0, 0xf], [0x0, 0xf, 0x8, 0x7, 0x5]] {
            for b in [false, true] {
                blend(b);
                for (i, &m) in seq.iter().enumerate() {
                    color_mask(m);
                    expect_unchanged(&format!("{}, draw {i} of {seq:x?}, blend {b}", t.name), m, t.alpha);
                }
                color_mask(0xf);
            }
        }
        blend(false);
    }
}

/// (d) no ES 3.0-only refusal is reachable from the ES 2.0 entry points. A
/// second colour target needs glDrawBuffers and GL_COLOR_ATTACHMENT1, which
/// are ES 3.0 (the `report-es2` build only changes what the context reports,
/// not which entry points the library has, so this test does not call
/// them). With ES 2.0's one attachment, every mask of a draw under blending
/// is taken, with one target, no per-target mask and no refusal: the uneven
/// masks `per_target_masks` refuses under blending need a second target.
#[test]
fn the_es3_only_refusals_are_not_reachable() {
    let _g = context();
    for t in [TARGETS[0], TARGETS[1], TARGETS[4]] {
        let _f = fbo(t);
        program();
        blend(true);
        for m in 0..16u8 {
            color_mask(m);
            let (seen, e) = draw();
            assert_eq!(e, GL_NO_ERROR, "{}, blend, mask {m:#x}", t.name);
            let s = seen.expect("the draw reaches the back end");
            assert_eq!(s.refused, None, "{}, blend, mask {m:#x}", t.name);
            assert!(s.num_targets <= 1, "{}, blend, mask {m:#x}: one target", t.name);
            assert!(!s.per_target_mask, "{}, blend, mask {m:#x}", t.name);
            assert!(s.job.is_ok(), "{}, blend, mask {m:#x}", t.name);
        }
        blend(false);
    }
}

/* ---- the COLOR_WRITE_MASKS word of the real job builder ------------------ */

const XML: &str = "third_party/mesa-26.2.1/src/broadcom/cle/v3d_packet.xml";

/// A 4.2 packet of Mesa's XML: name, opcode, size with the opcode, fields
/// (name, start bit after the opcode, size).
struct Pkt {
    code: u8,
    name: String,
    sub_id: Option<u8>,
    fields: Vec<(String, u32, u32)>,
    size: usize,
}

fn attr(line: &str, name: &str) -> Option<String> {
    let key = format!(" {name}=\"");
    let at = line.find(&key)? + key.len();
    let end = line[at..].find('"')?;
    Some(line[at..at + end].to_string())
}

/// The V3D 4.2 packets: those with no `min_ver` above 42.
fn packets() -> &'static [Pkt] {
    static P: OnceLock<Vec<Pkt>> = OnceLock::new();
    P.get_or_init(|| {
        let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(XML))
            .expect("Mesa's v3d_packet.xml");
        let (mut out, mut cur): (Vec<Pkt>, Option<(Pkt, bool)>) = (Vec::new(), None);
        for line in text.lines() {
            let l = line.trim();
            if l.starts_with("<packet ") {
                let v71 = attr(l, "min_ver").is_some_and(|v| v.parse::<u32>().unwrap() > 42);
                let p = Pkt {
                    code: attr(l, "code").map(|c| c.parse().unwrap()).unwrap_or(0),
                    name: attr(l, "name").unwrap(),
                    sub_id: None,
                    fields: Vec::new(),
                    size: 1,
                };
                if l.ends_with("/>") {
                    if !v71 {
                        out.push(p);
                    }
                } else {
                    cur = Some((p, v71));
                }
            } else if l.starts_with("<field ") {
                if let Some((p, _)) = cur.as_mut() {
                    let name = attr(l, "name").unwrap();
                    let size: u32 = attr(l, "size").unwrap().parse().unwrap();
                    let start: u32 = attr(l, "start").unwrap().parse().unwrap();
                    if name == "sub-id" {
                        p.sub_id = attr(l, "default").map(|d| d.parse().unwrap());
                    }
                    p.size = p.size.max(1 + (start + size).div_ceil(8) as usize);
                    p.fields.push((name, start, size));
                }
            } else if l.starts_with("</packet>") {
                if let Some((p, v71)) = cur.take() {
                    if !v71 {
                        out.push(p);
                    }
                }
            }
        }
        out
    })
}

/// Field `field` of every `name` packet in a control list, walked with the
/// XML's packet sizes.
fn fields_of(bytes: &[u8], name: &str, field: &str) -> Vec<u64> {
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let code = bytes[at];
        let cands: Vec<&Pkt> = packets().iter().filter(|p| p.code == code).collect();
        assert!(!cands.is_empty(), "opcode {code} at byte {at}: not a 4.2 packet in {XML}");
        let p = if cands.len() == 1 {
            cands[0]
        } else {
            let sub = bytes[at + 1] & 0xf;
            cands.iter().find(|c| c.sub_id == Some(sub)).unwrap_or_else(|| panic!("opcode {code} sub-id {sub}"))
        };
        assert!(at + p.size <= bytes.len(), "packet \"{}\" runs past the list", p.name);
        if p.name == name {
            let &(_, start, size) = p.fields.iter().find(|f| f.0 == field).expect("the field");
            let mut v: u128 = 0;
            for (i, b) in bytes[at + 1..at + p.size].iter().enumerate().take(16) {
                v |= (*b as u128) << (8 * i);
            }
            out.push(((v >> start) & ((1u128 << size) - 1)) as u64);
        }
        at += p.size;
    }
    out
}

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_job.c` built as a shared object of this test's own and
/// reached through the handle only (RTLD_LOCAL), as tests/v3d_frame_rt4.rs.
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_es2ctx_drift_colormask");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_job-{}.so", std::process::id()));
        let st = Command::new("cc")
            .current_dir(&root)
            .env("TMPDIR", &dir)
            .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
            .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
            .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
            .arg("-Ithird_party/mesa-26.2.1/include")
            .args([
                "driver/src/v3d/v3d_job.c",
                "driver/src/v3d/v3d_frame.c",
                "driver/src/v3d/v3d_tex.c",
                "driver/src/v3d/v3d_tiling_mesa.c",
                "-lm",
                "-o",
            ])
            .arg(&so)
            .status()
            .expect("cc runs");
        assert!(st.success(), "driver/src/v3d/v3d_job.c builds");
        let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
        // SAFETY: a path of a library this test just built; RTLD_NOW.
        let h = unsafe { dlopen(path.as_ptr(), 2) };
        assert!(!h.is_null(), "the job builder loads");
        let _ = std::fs::remove_file(&so);
        h as usize
    }) as *mut c_void
}

fn sym(name: &str) -> *mut c_void {
    let c = CString::new(name).unwrap();
    // SAFETY: a handle `lib` made and a NUL-terminated name.
    let p = unsafe { dlsym(lib(), c.as_ptr()) };
    assert!(!p.is_null(), "{name} in v3d_job.c");
    p
}

type BeginFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobTarget, *mut V3dCl, *mut V3dCl, *mut V3dCl) -> bool;
type DrawFn = unsafe extern "C" fn(*mut V3dJob, *const V3dJobDraw, *mut c_void) -> i32;

/// The COLOR_WRITE_MASKS "Mask" the job builder writes for one triangle
/// drawn with `st` in a four-RT RGBA8 job (the packet's word does not depend
/// on the RTs' formats).
fn color_write_masks(st: &V3dJobState) -> u32 {
    const LIST_BYTES: usize = 16 * 1024;
    let mut bufs = [vec![0u8; LIST_BYTES], vec![0u8; LIST_BYTES], vec![0u8; LIST_BYTES]];
    let [b, r, i] = &mut bufs;
    let cl = |buf: &mut Vec<u8>, gpu: u32| V3dCl {
        buf: buf.as_mut_ptr(),
        gpu,
        size: LIST_BYTES as u32,
        used: 0,
        overflow: false,
        bad_field: false,
    };
    let (mut bcl, mut rcl, mut ind) = (cl(b, 0x1000_0000), cl(r, 0x1100_0000), cl(i, 0x1200_0000));
    let fs = V3dFsProg {
        code: 0x0020_0000,
        threads: 4,
        single_seg: false,
        num_inputs: 0,
        flat_shade_flags: 0,
        noperspective_flags: 0,
        centroid_flags: 0,
        flat_shade_flags_hi: 0,
        noperspective_flags_hi: 0,
        centroid_flags_hi: 0,
        writes_z: false,
        disable_ez: false,
        uses_center_w: false,
        lock_scoreboard_on_first_thrsw: false,
        uses_implicit_point_line_varyings: false,
        uses_pid: false,
        uniforms: core::ptr::null(),
        num_uniforms: 0,
    };
    let vs_prog = |code: u32| V3dVsProg {
        code,
        threads: 4,
        single_seg: true,
        vpm_input_size: 0,
        vpm_output_size: 2,
        separate_segments: false,
        vcm_cache_size: 4,
        uses_vid: false,
        uses_iid: false,
        uses_biid: false,
        vattr_sizes: [0; 16],
        uniforms: core::ptr::null(),
        num_uniforms: 0,
    };
    let (vs, cs) = (vs_prog(0x0020_1000), vs_prog(0x0020_2000));
    let d = V3dJobDraw {
        st,
        fs: &fs,
        vs: &vs,
        cs: &cs,
        vs_storage: V3dUniformStorage::NONE,
        fs_storage: V3dUniformStorage::NONE,
        attrs: core::ptr::null(),
        num_attrs: 0,
        prim: V3D_PRIM_TRIANGLES,
        count: 3,
        first: 0,
        indexed: false,
        index_type: 0,
        index_buffer: 0,
        index_buffer_bytes: 0,
        index_offset: 0,
        tmu: core::ptr::null(),
        num_tmu: 0,
        vs_tmu: core::ptr::null(),
        num_vs_tmu: 0,
        vs_writes_psiz: false,
        instances: 0,
    };
    let mut t = V3dJobTarget {
        width: 64,
        height: 64,
        address: 0x2000_0000,
        stride: 256,
        tile_alloc: 0x3000_0000,
        rts: 4,
        typed: true,
        rt_format: [V3D_OUTPUT_IMAGE_FORMAT_RGBA8; 4],
        ..Default::default()
    };
    for n in 0..3 {
        t.rt[n] = V3dJobRt { address: 0x2100_0000 + 0x10_0000 * n as u32, stride: 256, ..Default::default() };
    }
    let mut job = V3dJob::ZERO;
    // SAFETY: the C signatures of v3d_job.h; the lists, the programs and the
    // state are live for the calls.
    let res = unsafe {
        let begin: BeginFn = std::mem::transmute(sym("v3d_job_begin"));
        let draw: DrawFn = std::mem::transmute(sym("v3d_job_draw"));
        assert!(begin(&mut job, &t, &mut bcl, &mut rcl, &mut ind), "a four-RT job begins");
        draw(&mut job, &d, core::ptr::null_mut())
    };
    assert_eq!(res, V3D_JOB_OK, "the draw goes in");
    let masks = fields_of(&bufs[0][..bcl.used as usize], "Color Write Masks", "Mask");
    assert_eq!(masks.len(), 1, "one COLOR_WRITE_MASKS in the draw ({XML}:804)");
    masks[0] as u32
}
