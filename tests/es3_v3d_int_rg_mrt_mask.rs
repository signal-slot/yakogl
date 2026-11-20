// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): a colour write mask per render target, so an R or RG
//! integer texture drawn as its four-channel store (`WIDE_INT_RT`,
//! src/gl/framebuffer.rs) can be drawn beside another colour target whose
//! mask differs (gap G3 of docs/notes/v3d-format-matrix.md, plan #383).
//!
//! The V3D 4.2 COLOR_WRITE_MASKS packet carries four disable bits per RT
//! (Mesa 26.2.1 v3d_packet.xml "Color Write Masks", field "Mask"; RT n at
//! bits 4n..4n+3, as v3dx_emit.c:571-585 packs translate_colormask per RT).
//! The GL core gives a draw whose targets need different masks one per
//! target (draw.rs `per_target_masks`: `GlesDraw::per_target_mask`,
//! `target_color_mask`), each the application's glColorMask limited to its
//! target's channels; `state::job_state` carries RT 1..3's into
//! `V3dJobState::rt_colormask`; driver/src/v3d/v3d_job.c packs them.
//!
//! Held:
//!
//! - each of the twelve R and RG integer formats as a TEXTURE_2D at
//!   attachment 1 beside an RGBA8 renderbuffer at attachment 0, and beside
//!   an RGBA32UI one, every channel on: the draw is taken (no GL error, no
//!   refusal), RT 0 writes all four channels, RT 1 the format's alone;
//! - the same under a glColorMask subset (R and A; G and B): each RT writes
//!   the subset limited to its channels; a subset within the format's
//!   channels is one mask for both, and no per-RT mask is given;
//! - three targets (RGBA8, R8I, RG16UI): each RT's nibble in its place;
//! - the COLOR_WRITE_MASKS word the real job builder (driver/src/v3d/
//!   v3d_job.c, built here as a shared object) writes for the job state of
//!   such a draw, decoded with Mesa's packet XML;
//! - what stays refused by name: glEnable(GL_BLEND) with uneven masks and
//!   both integer and non-integer targets (the V3D has one blend enable for
//!   every RT, v3d_job.c emit_blend): GL_INVALID_OPERATION, nothing handed
//!   to the back end. Beside an RGBA32UI target (both integer) blending is
//!   ignored (ES 3.0 4.1.8) and the draw is taken.
//!
//! What it cannot see: the GPU. The back end here is a stand-in that records
//! what `state::refusal`, `state::unsupported` and `state::job_state` make of
//! each draw; nothing is rasterized and nothing here is verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_int_rg_mrt_mask
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_int, c_void};
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::Ordering;
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
    /// `targets[..num_targets]`: formats.
    formats: Vec<u8>,
    color_mask: u8,
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
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        formats: d.targets[..n].iter().map(|t| t.format).collect(),
        color_mask: d.color_mask,
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
    name: c"es3-v3d-int-rg-mrt-mask".as_ptr() as *const c_char,
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
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
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

/* ---- the twelve formats -------------------------------------------------- */

#[derive(Clone, Copy)]
struct Row {
    name: &'static str,
    ifmt: GLenum,
    channels: u8,
    signed: bool,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool) -> Row {
    Row { name, ifmt, channels, signed }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true),
    row("R8UI", GL_R8UI, 1, false),
    row("RG8I", GL_RG8I, 2, true),
    row("RG8UI", GL_RG8UI, 2, false),
    row("R16I", GL_R16I, 1, true),
    row("R16UI", GL_R16UI, 1, false),
    row("RG16I", GL_RG16I, 2, true),
    row("RG16UI", GL_RG16UI, 2, false),
    row("R32I", GL_R32I, 1, true),
    row("R32UI", GL_R32UI, 1, false),
    row("RG32I", GL_RG32I, 2, true),
    row("RG32UI", GL_RG32UI, 2, false),
];

impl Row {
    fn ty(&self) -> &'static str {
        if self.signed {
            "ivec4"
        } else {
            "uvec4"
        }
    }
    /// The format's channels (bit 0 R, bit 1 G).
    fn own(&self) -> u8 {
        (1u8 << self.channels) - 1
    }
}

/* ---- a framebuffer, a program, a draw ------------------------------------ */

/// A bound framebuffer; dropped, it unbinds and deletes what it made.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.tex.len() as GLsizei, self.tex.as_ptr());
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// What one colour attachment is.
#[derive(Clone, Copy)]
enum Att {
    Renderbuffer(GLenum),
    Texture(GLenum),
}

/// A framebuffer with `atts` at attachments 0.. and every one of them a draw
/// buffer.
fn fbo(atts: &[Att]) -> Fbo {
    let mut f = Fbo { fb: 0, tex: vec![], rbs: vec![] };
    // SAFETY: room for one name each; the attachments named are there.
    unsafe {
        glGenFramebuffers(1, &mut f.fb);
        glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
        let mut bufs = Vec::new();
        for (i, a) in atts.iter().enumerate() {
            let at = GL_COLOR_ATTACHMENT0 + i as GLenum;
            let mut name = 0;
            match *a {
                Att::Renderbuffer(ifmt) => {
                    glGenRenderbuffers(1, &mut name);
                    glBindRenderbuffer(GL_RENDERBUFFER, name);
                    glRenderbufferStorage(GL_RENDERBUFFER, ifmt, 8, 8);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, at, GL_RENDERBUFFER, name);
                    f.rbs.push(name);
                }
                Att::Texture(ifmt) => {
                    glGenTextures(1, &mut name);
                    glBindTexture(GL_TEXTURE_2D, name);
                    glTexStorage2D(GL_TEXTURE_2D, 1, ifmt, 8, 8);
                    glFramebufferTexture2D(GL_FRAMEBUFFER, at, GL_TEXTURE_2D, name, 0);
                    f.tex.push(name);
                }
            }
            bufs.push(at);
        }
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "storage and attach");
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }, GL_FRAMEBUFFER_COMPLETE);
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

/// A program with one output per type of `tys`, at locations 0.., current.
fn program(tys: &[&str]) {
    let mut fs = String::from("#version 300 es\nprecision highp int;\nprecision mediump float;\n");
    let mut body = String::new();
    for (i, ty) in tys.iter().enumerate() {
        fs += &format!("layout(location = {i}) out highp {ty} o{i};\n");
        body += &match *ty {
            "vec4" => format!("o{i} = vec4(0.25, 0.5, 0.75, 1.0); "),
            "ivec4" => format!("o{i} = ivec4(1, -2, 3, -4); "),
            _ => format!("o{i} = uvec4(1u, 2u, 3u, 4u); "),
        };
    }
    fs += &format!("void main() {{ {body}}}\n");
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{fs:?} did not link");
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

/// A draw taken with RT k writing `want[k]`: the GL core's per-target masks
/// (given exactly where they differ), the job state's, and the
/// COLOR_WRITE_MASKS word the job builder packs from it.
fn expect_taken(what: &str, want: &[u8]) -> V3dJobState {
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{what}");
    let s = seen.unwrap_or_else(|| panic!("{what}: the back end saw no draw"));
    assert_eq!(s.refused, None, "{what}");
    assert_eq!(s.formats.len(), want.len(), "{what}: the targets");
    assert_eq!(s.color_mask, want[0], "{what}: RT 0's mask");
    let st = s.job.unwrap_or_else(|why| panic!("{what}: job_state refuses: {why}"));
    assert_eq!(st.colormask, u32::from(want[0]), "{what}: the job's RT 0 mask");
    let uneven = want.iter().any(|&m| m != want[0]);
    assert_eq!(s.per_target_mask, uneven, "{what}: a mask per target exactly where they differ");
    assert_eq!(st.rt_colormask_own, uneven, "{what}: the job's own RT masks");
    if uneven {
        for (k, &m) in want.iter().enumerate() {
            assert_eq!(s.target_color_mask[k], m, "{what}: target {k}'s mask");
        }
        let packed = want[1..].iter().enumerate().map(|(k, &m)| u32::from(m) << (4 * k)).fold(0, |a, m| a | m);
        assert_eq!(st.rt_colormask, packed, "{what}: RT 1..3 in the job state");
    } else {
        assert_eq!(st.rt_colormask, 0, "{what}");
    }
    assert_eq!(color_write_masks(&st), expected_wmask(want), "{what}: COLOR_WRITE_MASKS");
    st
}

/// The COLOR_WRITE_MASKS "Mask" for targets writing `want` (a set bit
/// disables the channel, RT n at bits 4n..4n+3): one mask for every RT
/// where they agree, as before; else each target's own, and an RT that is no
/// target of the draw (not in the job) none.
fn expected_wmask(want: &[u8]) -> u32 {
    let even = want.iter().all(|&m| m == want[0]);
    (0..4)
        .map(|n| {
            let m = if even { want[0] } else { want.get(n).copied().unwrap_or(0) };
            (!u32::from(m) & 0xf) << (4 * n)
        })
        .fold(0, |a, m| a | m)
}

/* ---- the cases ------------------------------------------------------------ */

/// An R or RG integer texture at attachment 1 beside an RGBA8 renderbuffer
/// at attachment 0 (vec4 output) and beside an RGBA32UI one (uvec4), every
/// channel on: RT 0 all four, RT 1 the format's channels.
#[test]
fn an_r_or_rg_integer_texture_at_attachment_1_takes_its_own_mask() {
    let _g = context();
    for r in ROWS {
        for (first, ty0) in [(GL_RGBA8, "vec4"), (GL_RGBA32UI, "uvec4")] {
            let _f = fbo(&[Att::Renderbuffer(first), Att::Texture(r.ifmt)]);
            program(&[ty0, r.ty()]);
            let what = format!("{} at 1 beside {first:#x} at 0, every channel on", r.name);
            expect_taken(&what, &[0xf, r.own()]);
        }
    }
}

/// glColorMask subsets. R and A: RT 0 writes R and A, RT 1 R alone (the
/// texture has no A). G and B: RT 1 writes G for an RG format, nothing for
/// an R one. R alone (within every format's channels): one mask for both,
/// no per-RT mask given.
#[test]
fn a_colour_mask_subset_is_limited_per_target() {
    let _g = context();
    for r in ROWS {
        for (first, ty0) in [(GL_RGBA8, "vec4"), (GL_RGBA32UI, "uvec4")] {
            let _f = fbo(&[Att::Renderbuffer(first), Att::Texture(r.ifmt)]);
            program(&[ty0, r.ty()]);
            for app in [0b1001u8, 0b0110, 0b0001, 0b0011] {
                color_mask(app);
                let what = format!("{} at 1 beside {first:#x}, glColorMask {app:#06b}", r.name);
                expect_taken(&what, &[app, app & r.own()]);
            }
            color_mask(0xf);
        }
    }
}

/// Three targets: RGBA8, an R8I texture, an RG16UI texture. Each RT's nibble
/// in its place.
#[test]
fn three_targets_each_take_their_own_nibble() {
    let _g = context();
    let _f = fbo(&[Att::Renderbuffer(GL_RGBA8), Att::Texture(GL_R8I), Att::Texture(GL_RG16UI)]);
    program(&["vec4", "ivec4", "uvec4"]);
    expect_taken("RGBA8, R8I, RG16UI", &[0xf, 0x1, 0x3]);
    /* spelled out once: RT 0 none off, RT 1 G, B, A off, RT 2 B, A off, RT 3
     * (no target) all off */
    assert_eq!(expected_wmask(&[0xf, 0x1, 0x3]), 0xfce0);
}

/// The R or RG integer texture at attachment 0 beside an RGBA8 renderbuffer
/// at attachment 1: RT 0 the format's channels, RT 1 all four.
#[test]
fn the_texture_at_attachment_0_beside_rgba8_takes_its_own_mask() {
    let _g = context();
    for r in ROWS {
        let _f = fbo(&[Att::Texture(r.ifmt), Att::Renderbuffer(GL_RGBA8)]);
        program(&[r.ty(), "vec4"]);
        let what = format!("{} at 0 beside RGBA8 at 1", r.name);
        expect_taken(&what, &[r.own(), 0xf]);
    }
}

/// glEnable(GL_BLEND) with uneven masks: beside RGBA8 the V3D's one blend
/// enable would blend the integer store or not blend the RGBA8 target, so
/// the draw is refused by name (GL_INVALID_OPERATION, nothing handed to the
/// back end). Beside RGBA32UI every target is integer, blending is ignored
/// (ES 3.0 4.1.8), and the draw is taken with a mask per target.
#[test]
fn blending_beside_a_non_integer_target_stays_refused_by_name() {
    let _g = context();
    for r in ROWS {
        {
            let _f = fbo(&[Att::Renderbuffer(GL_RGBA8), Att::Texture(r.ifmt)]);
            program(&["vec4", r.ty()]);
            // SAFETY: plain GL call.
            unsafe { glEnable(GL_BLEND) };
            let (seen, e) = draw();
            assert_eq!(e, GL_INVALID_OPERATION, "{} beside RGBA8, blending: refused", r.name);
            assert!(seen.is_none(), "{} beside RGBA8, blending: the back end saw a draw", r.name);
            /* within the format's channels the mask is one for both: taken
             * as before (one mask, one blend state for every RT) */
            color_mask(0b0001);
            let (seen, e) = draw();
            assert_eq!(e, GL_NO_ERROR, "{} beside RGBA8, blending, R alone", r.name);
            assert!(seen.is_some_and(|s| !s.per_target_mask), "{} beside RGBA8, blending, R alone", r.name);
            color_mask(0xf);
            // SAFETY: plain GL call.
            unsafe { glDisable(GL_BLEND) };
        }
        let _f = fbo(&[Att::Renderbuffer(GL_RGBA32UI), Att::Texture(r.ifmt)]);
        program(&["uvec4", r.ty()]);
        // SAFETY: plain GL call.
        unsafe { glEnable(GL_BLEND) };
        let st = expect_taken(&format!("{} beside RGBA32UI, blending", r.name), &[0xf, r.own()]);
        assert!(!st.blend_enabled, "{} beside RGBA32UI: no blending of integer targets", r.name);
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_BLEND) };
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_v3d_int_rg_mrt_mask");
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
