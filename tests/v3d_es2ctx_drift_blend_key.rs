// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against the per-target blend key of c2d132ad
//! (`gl::target_keys`: the key list of a context's draw buffers) and cff449d2
//! (`current_blend_key` tries the mixed per-target key first for several
//! targets), on the V3D 4.2 (Raspberry Pi 4). The Pi 4 whole dEQP-GLES2 tree
//! (46f93767) predates both. ES 2.0 has one colour target, so no draw has
//! "several targets" and the mixed key is never taken; and the V3D blends in
//! fixed function (`draw.rs`: `fixed_function_blend`), so its fragment shader
//! is the linked program's own whatever the blend state. This file replays
//! the blend draws of dEQP-GLES2 fragment_ops.blend.* through the real GL
//! core (a `report-es2` build) and asserts, for each draw:
//!
//! - GL_NO_ERROR and no refusal (`state::refusal`, `state::unsupported`,
//!   `state::job_state`);
//! - the draw's fragment shader is the one the program's blend-off draw has
//!   (the single-target variant, key 0), `gl::target_keys::draw_mixed_key` is
//!   None and the key list has at most one target;
//! - the BLEND_ENABLES, BLEND_CFG, BLEND_CONSTANT_COLOR and
//!   COLOR_WRITE_MASKS words are the ones the rule at the parent of cff449d2
//!   gives. The words are packed by the real driver/src/v3d/v3d_job.c (built
//!   here as a shared object) and decoded with Mesa's packet XML; the rule is
//!   written out below from the XML's enums and Mesa's v3dx_emit.c: a GL
//!   factor is its V3D "Blend Factor" (a destination alpha reads 1 on a
//!   target with no alpha, so DST_ALPHA is ONE, ONE_MINUS_DST_ALPHA ZERO and
//!   SRC_ALPHA_SATURATE ZERO), the equations ADD, SUBTRACT, REVERSE_SUBTRACT
//!   are the modes 0, 1, 2, BLEND_ENABLES is 0xf with blending and 0 without
//!   (and then no BLEND_CFG), the constant colour is four halves of the
//!   clamped glBlendColor, and a COLOR_WRITE_MASKS nibble holds the channels
//!   the mask disables, the same for the four RTs (alpha off on a target
//!   with none).
//!
//! Covered: the three ES 2.0 blend equations and separate RGB and alpha
//! equations; every source and destination factor (the full cross product on
//! the default framebuffer, the pairs of fragment_ops.blend.* elsewhere);
//! glBlendColor (in range and clamped); glColorMask subsets; blend on and
//! off; the default framebuffer, an RGBA8 texture, an RGB texture, and RGBA4,
//! RGB5_A1 and RGB565 renderbuffers. The per-RT colour mask of 0ebbd507 and
//! 48f6017c is tests/v3d_es2ctx_drift_colormask.rs's.
//!
//! What it cannot see: the GPU. The back end here is a stand-in that records
//! what the V3D back end's own `state::*` make of each draw.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test v3d_es2ctx_drift_blend_key
//! ```
//!
//! The feature set is `V3D_ES2_CONFIG` of tools/check.sh. The first run
//! compiles driver/src/v3d/v3d_job.c with `cc`; a cold build of the crate
//! takes 30 to 60 minutes.
#![cfg(all(feature = "shaderc-online", feature = "gpu-v3d", feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::target_keys::{draw_mixed_key, draw_target_keys};
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

/// What the back end was handed by one draw, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// The draw's fragment shader (the program's own, or a variant of it).
    fs: usize,
    num_targets: u8,
    alpha_bits: u8,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`: the job's state, or its refusal.
    job: Result<V3dJobState, &'static str>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core passes a live draw.
    let d = unsafe { &*d };
    if d.index_count == 0 {
        return 0;
    }
    SEEN.lock().unwrap().push(Seen {
        fs: d.fs as usize,
        num_targets: d.num_targets,
        alpha_bits: d.target_alpha_bits,
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
    name: c"es2ctx-drift-blend-key".as_ptr() as *const c_char,
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

fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        glBlendEquation(GL_FUNC_ADD);
        glBlendFunc(GL_ONE, GL_ZERO);
        glBlendColor(0.0, 0.0, 0.0, 0.0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- a surface, a program, a draw ---------------------------------------- */

/// A colour target of ES 2.0, with whether it has alpha.
#[derive(Clone, Copy, Debug)]
enum Surface {
    Default,
    Texture(GLenum, bool),
    Renderbuffer(GLenum, bool),
}

const SURFACES: [Surface; 6] = [
    Surface::Default,
    Surface::Texture(GL_RGBA, true),
    Surface::Texture(GL_RGB, false),
    Surface::Renderbuffer(GL_RGBA4, true),
    Surface::Renderbuffer(GL_RGB5_A1, true),
    Surface::Renderbuffer(GL_RGB565, false),
];

impl Surface {
    fn alpha(self) -> bool {
        match self {
            Surface::Default => true,
            Surface::Texture(_, a) | Surface::Renderbuffer(_, a) => a,
        }
    }
}

/// Bind `s` (a framebuffer object for all but the default) for `f`; an
/// incomplete one fails the test: every one of them is complete in ES 2.0.
fn on_surface(s: Surface, f: impl FnOnce()) {
    let mut fb: GLuint = 0;
    let (mut tex, mut rb): (GLuint, GLuint) = (0, 0);
    // SAFETY: plain GL calls; room for one name each.
    unsafe {
        match s {
            Surface::Default => {}
            Surface::Texture(fmt, _) => {
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexImage2D(GL_TEXTURE_2D, 0, fmt as GLint, 8, 8, 0, fmt, GL_UNSIGNED_BYTE, core::ptr::null());
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            }
            Surface::Renderbuffer(fmt, _) => {
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, fmt, 8, 8);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
            }
        }
        if fb != 0 {
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{s:?}");
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{s:?}: the target");
    f();
    // SAFETY: names this function made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteTextures(1, &tex);
        glDeleteRenderbuffers(1, &rb);
    }
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

/// A GLSL ES 1.00 program that writes a colour, current.
fn program() {
    let v = shader(GL_VERTEX_SHADER, "attribute vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(
        GL_FRAGMENT_SHADER,
        "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 0.5); }\n",
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

/* ---- the rule at the parent of cff449d2 ---------------------------------- */

/// A GL blend factor as the V3D's "Blend Factor" (v3d_packet.xml enum, Mesa's
/// v3d_factor) on a target with (`alpha`) or without destination alpha.
fn v3d_factor(gl: GLenum, alpha: bool) -> u64 {
    match gl {
        GL_ZERO => 0,
        GL_ONE => 1,
        GL_SRC_COLOR => 2,
        GL_ONE_MINUS_SRC_COLOR => 3,
        GL_DST_COLOR => 4,
        GL_ONE_MINUS_DST_COLOR => 5,
        GL_SRC_ALPHA => 6,
        GL_ONE_MINUS_SRC_ALPHA => 7,
        GL_DST_ALPHA if alpha => 8,
        GL_DST_ALPHA => 1,
        GL_ONE_MINUS_DST_ALPHA if alpha => 9,
        GL_ONE_MINUS_DST_ALPHA => 0,
        GL_CONSTANT_COLOR => 10,
        GL_ONE_MINUS_CONSTANT_COLOR => 11,
        GL_CONSTANT_ALPHA => 12,
        GL_ONE_MINUS_CONSTANT_ALPHA => 13,
        GL_SRC_ALPHA_SATURATE if alpha => 14,
        GL_SRC_ALPHA_SATURATE => 0,
        _ => panic!("factor {gl:#x}"),
    }
}

/// An ES 2.0 blend equation as the V3D's "Blend Mode".
fn v3d_mode(gl: GLenum) -> u64 {
    match gl {
        GL_FUNC_ADD => 0,
        GL_FUNC_SUBTRACT => 1,
        GL_FUNC_REVERSE_SUBTRACT => 2,
        _ => panic!("equation {gl:#x}"),
    }
}

/// A float in [0, 1] as an IEEE half, for the values the file uses (exact).
fn half(v: f32) -> u64 {
    match v {
        0.0 => 0x0000,
        0.125 => 0x3000,
        0.25 => 0x3400,
        0.5 => 0x3800,
        0.75 => 0x3a00,
        1.0 => 0x3c00,
        _ => panic!("half({v})"),
    }
}

/// The state of one draw: what the application set.
#[derive(Clone, Copy, Debug)]
struct Gl {
    blend: bool,
    eq: (GLenum, GLenum),
    /// src RGB, dst RGB, src alpha, dst alpha
    func: (GLenum, GLenum, GLenum, GLenum),
    /// The blend colour as set, and as the rule clamps it.
    color: [f32; 4],
    clamped: [f32; 4],
    mask: u8,
}

impl Gl {
    const fn new() -> Gl {
        Gl {
            blend: false,
            eq: (GL_FUNC_ADD, GL_FUNC_ADD),
            func: (GL_ONE, GL_ZERO, GL_ONE, GL_ZERO),
            color: [0.0; 4],
            clamped: [0.0; 4],
            mask: 0xf,
        }
    }
    fn blend(mut self, on: bool) -> Gl {
        self.blend = on;
        self
    }
    fn func(mut self, s: GLenum, d: GLenum) -> Gl {
        self.func = (s, d, s, d);
        self
    }
    fn func_sep(mut self, sr: GLenum, dr: GLenum, sa: GLenum, da: GLenum) -> Gl {
        self.func = (sr, dr, sa, da);
        self
    }
    fn eq(mut self, e: GLenum) -> Gl {
        self.eq = (e, e);
        self
    }
    fn eq_sep(mut self, r: GLenum, a: GLenum) -> Gl {
        self.eq = (r, a);
        self
    }
    fn color(mut self, c: [f32; 4], clamped: [f32; 4]) -> Gl {
        self.color = c;
        self.clamped = clamped;
        self
    }
    fn mask(mut self, m: u8) -> Gl {
        self.mask = m;
        self
    }
    fn apply(&self) {
        // SAFETY: plain GL calls.
        unsafe {
            glBlendEquationSeparate(self.eq.0, self.eq.1);
            glBlendFuncSeparate(self.func.0, self.func.1, self.func.2, self.func.3);
            glBlendColor(self.color[0], self.color[1], self.color[2], self.color[3]);
            if self.blend {
                glEnable(GL_BLEND);
            } else {
                glDisable(GL_BLEND);
            }
        }
        color_mask(self.mask);
    }
}

/// The words a draw of state `g` on a target with (`alpha`) or without alpha
/// must give, by the rule at cff449d2's parent.
fn expected(g: &Gl, alpha: bool) -> Words {
    let mask = if alpha { g.mask } else { g.mask & 0x7 };
    let off = u64::from(!mask & 0xf);
    Words {
        enables: if g.blend { 0xf } else { 0 },
        cfg: g.blend.then(|| {
            [
                0xf,
                v3d_factor(g.func.1, alpha),
                v3d_factor(g.func.0, alpha),
                v3d_mode(g.eq.0),
                v3d_factor(g.func.3, alpha),
                v3d_factor(g.func.2, alpha),
                v3d_mode(g.eq.1),
            ]
        }),
        /* the constant colour is the same whether or not it blends */
        color: [half(g.clamped[0]), half(g.clamped[1]), half(g.clamped[2]), half(g.clamped[3])],
        wmask: off | off << 4 | off << 8 | off << 12,
    }
}

/// One draw: taken with GL_NO_ERROR, no refusal, the single-target fragment
/// shader `fs` (None: the first draw names it), a key list of one target at
/// most and no mixed key; the packed words are `g`'s.
fn check(what: &str, s: Surface, g: &Gl, fs: &mut Option<usize>) {
    g.apply();
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{what}");
    let seen = seen.unwrap_or_else(|| panic!("{what}: the back end saw no draw"));
    assert_eq!(seen.refused, None, "{what}: refused");
    assert!(seen.num_targets <= 1, "{what}: {} targets", seen.num_targets);
    assert_eq!(seen.alpha_bits != 0, s.alpha(), "{what}: the target's alpha bits");
    let st = seen.job.unwrap_or_else(|why| panic!("{what}: job_state refuses: {why}"));
    /* the single-target variant: the program's own fragment shader, however
     * it blends (the V3D blends in fixed function; key 0) */
    assert_eq!(*fs.get_or_insert(seen.fs), seen.fs, "{what}: a different fragment shader");
    let c = context::current().expect("a current context");
    assert_eq!(draw_mixed_key(c), None, "{what}: a mixed key for one target");
    if let Some((_, n)) = draw_target_keys(c) {
        assert_eq!(n, 1, "{what}: the key list");
    }
    assert_eq!(st.blend_enabled, g.blend, "{what}: the job state's blend");
    assert_eq!(words(&st), expected(g, s.alpha()), "{what}: the packed words");
}

/// BLEND_ENABLES "Mask", BLEND_CFG (render target mask, colour dst, colour
/// src, colour mode, alpha dst, alpha src, alpha mode; None when the draw has
/// none), BLEND_CONSTANT_COLOR (red, green, blue, alpha halves) and
/// COLOR_WRITE_MASKS "Mask", as the job builder wrote them.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Words {
    enables: u64,
    cfg: Option<[u64; 7]>,
    color: [u64; 4],
    wmask: u64,
}

/* ---- Mesa's packet XML and the real job builder -------------------------- */

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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_es2ctx_drift_blend_key");
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

/// The words the job builder writes for one triangle drawn with `st` in a
/// four-RT RGBA8 job (the packets' words do not depend on the RTs' formats).
fn words(st: &V3dJobState) -> Words {
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
    let list = &bufs[0][..bcl.used as usize];
    let one = |name: &str, field: &str| -> Vec<u64> { fields_of(list, name, field) };
    let enables = one("Blend Enables", "Mask");
    assert_eq!(enables.len(), 1, "one BLEND_ENABLES in the draw");
    let cfg = |f: &str| one("Blend Cfg", f);
    let n = cfg("Render Target Mask").len();
    assert!(n <= 1, "at most one BLEND_CFG in the draw");
    let cfg = (n == 1).then(|| {
        [
            cfg("Render Target Mask")[0],
            cfg("Color blend dst factor")[0],
            cfg("Color blend src factor")[0],
            cfg("Color blend mode")[0],
            cfg("Alpha blend dst factor")[0],
            cfg("Alpha blend src factor")[0],
            cfg("Alpha blend mode")[0],
        ]
    });
    let col = |f: &str| {
        let v = one("Blend Constant Color", f);
        assert_eq!(v.len(), 1, "one BLEND_CONSTANT_COLOR in the draw");
        v[0]
    };
    let wmask = one("Color Write Masks", "Mask");
    assert_eq!(wmask.len(), 1, "one COLOR_WRITE_MASKS in the draw ({XML}:804)");
    Words {
        enables: enables[0],
        cfg,
        color: [col("Red (F16)"), col("Green (F16)"), col("Blue (F16)"), col("Alpha (F16)")],
        wmask: wmask[0],
    }
}

/* ---- the cases ------------------------------------------------------------ */

const SRC: [GLenum; 15] = [
    GL_ZERO,
    GL_ONE,
    GL_SRC_COLOR,
    GL_ONE_MINUS_SRC_COLOR,
    GL_DST_COLOR,
    GL_ONE_MINUS_DST_COLOR,
    GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA,
    GL_DST_ALPHA,
    GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR,
    GL_ONE_MINUS_CONSTANT_COLOR,
    GL_CONSTANT_ALPHA,
    GL_ONE_MINUS_CONSTANT_ALPHA,
    GL_SRC_ALPHA_SATURATE,
];

/// The destination factors: the source's, but SRC_ALPHA_SATURATE (ES 2.0 2.2).
const DST: [GLenum; 14] = [
    GL_ZERO,
    GL_ONE,
    GL_SRC_COLOR,
    GL_ONE_MINUS_SRC_COLOR,
    GL_DST_COLOR,
    GL_ONE_MINUS_DST_COLOR,
    GL_SRC_ALPHA,
    GL_ONE_MINUS_SRC_ALPHA,
    GL_DST_ALPHA,
    GL_ONE_MINUS_DST_ALPHA,
    GL_CONSTANT_COLOR,
    GL_ONE_MINUS_CONSTANT_COLOR,
    GL_CONSTANT_ALPHA,
    GL_ONE_MINUS_CONSTANT_ALPHA,
];

const EQS: [GLenum; 3] = [GL_FUNC_ADD, GL_FUNC_SUBTRACT, GL_FUNC_REVERSE_SUBTRACT];

/// The func pairs of dEQP-GLES2 fragment_ops.blend.* (default_framebuffer and
/// fbo_rgba*; funcs.cpp's equation and function lists).
const PAIRS: [(GLenum, GLenum); 14] = [
    (GL_ZERO, GL_ONE),
    (GL_ONE, GL_ONE),
    (GL_SRC_COLOR, GL_ONE_MINUS_SRC_COLOR),
    (GL_DST_COLOR, GL_ONE_MINUS_DST_COLOR),
    (GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA),
    (GL_DST_ALPHA, GL_ONE_MINUS_DST_ALPHA),
    (GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_COLOR),
    (GL_CONSTANT_ALPHA, GL_ONE_MINUS_CONSTANT_ALPHA),
    (GL_ONE_MINUS_CONSTANT_COLOR, GL_CONSTANT_COLOR),
    (GL_ONE_MINUS_CONSTANT_ALPHA, GL_CONSTANT_ALPHA),
    (GL_SRC_ALPHA_SATURATE, GL_ONE),
    (GL_SRC_ALPHA_SATURATE, GL_SRC_COLOR),
    (GL_ONE, GL_ONE_MINUS_SRC_ALPHA),
    (GL_ONE_MINUS_DST_ALPHA, GL_DST_ALPHA),
];

const COLOR: [f32; 4] = [0.25, 0.5, 0.75, 0.125];

/// Sanity: this is an ES 2.0 context (GL_VERSION says ES 2.0, the Pi 4 tree's
/// build, `report-es2`).
#[test]
fn the_context_is_es2() {
    let _g = rig();
    let mut major = 0;
    // SAFETY: plain GL call.
    unsafe { glGetIntegerv(GL_MAJOR_VERSION, &mut major) };
    assert_eq!(err(), GL_INVALID_ENUM, "GL_MAJOR_VERSION is an ES 3.0 query");
    assert_eq!(major, 0);
}

/// Every source and destination factor, on the default framebuffer, blend on
/// (each one's blend-off draw first, with the same words but for the blend's).
#[test]
fn every_factor_pair_on_the_default_framebuffer() {
    let _g = rig();
    program();
    let mut fs = None;
    check("default, blend off", Surface::Default, &Gl::new().color(COLOR, COLOR), &mut fs);
    for s in SRC {
        for d in DST {
            let g = Gl::new().blend(true).func(s, d).color(COLOR, COLOR);
            check(&format!("default, blend {s:#x} {d:#x}"), Surface::Default, &g, &mut fs);
        }
    }
}

/// The three equations, RGB and alpha separately, with separate RGB and alpha
/// functions, on every surface: blend on and off.
#[test]
fn equations_and_separate_funcs_on_every_surface() {
    let _g = rig();
    program();
    let mut fs = None;
    for s in SURFACES {
        on_surface(s, || {
            for on in [false, true] {
                for rgb in EQS {
                    for alpha in EQS {
                        let g = Gl::new()
                            .blend(on)
                            .eq_sep(rgb, alpha)
                            .func_sep(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE_MINUS_SRC_ALPHA)
                            .color(COLOR, COLOR);
                        check(&format!("{s:?}, blend {on}, eq {rgb:#x} {alpha:#x}"), s, &g, &mut fs);
                    }
                }
                /* separate RGB and alpha functions, with every equation */
                for (a, b) in [(0, 5), (3, 9), (8, 12), (10, 2), (14, 13)] {
                    let g = Gl::new()
                        .blend(on)
                        .eq(GL_FUNC_SUBTRACT)
                        .func_sep(SRC[a], DST[b % 14], SRC[b], DST[a % 14])
                        .color(COLOR, COLOR);
                    check(&format!("{s:?}, blend {on}, separate funcs {a} {b}"), s, &g, &mut fs);
                }
            }
        });
    }
}

/// The func pairs of fragment_ops.blend.*, with each equation, on every
/// surface (the destination-alpha factors on the targets with no alpha read
/// one).
#[test]
fn the_deqp_pairs_on_every_surface() {
    let _g = rig();
    program();
    let mut fs = None;
    for s in SURFACES {
        on_surface(s, || {
            for (src, dst) in PAIRS {
                for eq in EQS {
                    let g = Gl::new().blend(true).eq(eq).func(src, dst).color(COLOR, COLOR);
                    check(&format!("{s:?}, {src:#x} {dst:#x}, eq {eq:#x}"), s, &g, &mut fs);
                }
            }
        });
    }
}

/// glBlendColor: values in range, and clamped to [0, 1] (ES 2.0 4.1.3); the
/// words carry it with and without blending.
#[test]
fn the_blend_colour_is_four_halves() {
    let _g = rig();
    program();
    let mut fs = None;
    for s in SURFACES {
        on_surface(s, || {
            for on in [false, true] {
                for (set, clamped) in [
                    ([0.0; 4], [0.0; 4]),
                    ([1.0; 4], [1.0; 4]),
                    (COLOR, COLOR),
                    ([0.75, 0.5, 0.25, 1.0], [0.75, 0.5, 0.25, 1.0]),
                    ([2.0, -1.0, 0.5, 0.25], [1.0, 0.0, 0.5, 0.25]),
                    ([-0.5, 7.0, -3.0, 1.5], [0.0, 1.0, 0.0, 1.0]),
                ] {
                    let g =
                        Gl::new().blend(on).func(GL_CONSTANT_COLOR, GL_ONE_MINUS_CONSTANT_ALPHA).color(set, clamped);
                    check(&format!("{s:?}, blend {on}, colour {set:?}"), s, &g, &mut fs);
                }
            }
        });
    }
}

/// glColorMask: every one of the sixteen subsets, blend on and off, on every
/// surface; a target with no alpha never writes it.
#[test]
fn every_colour_mask_subset_on_every_surface() {
    let _g = rig();
    program();
    let mut fs = None;
    for s in SURFACES {
        on_surface(s, || {
            for on in [false, true] {
                for m in 0..16u8 {
                    let g = Gl::new().blend(on).func(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA).color(COLOR, COLOR).mask(m);
                    check(&format!("{s:?}, blend {on}, mask {m:#06b}"), s, &g, &mut fs);
                }
            }
        });
    }
}
