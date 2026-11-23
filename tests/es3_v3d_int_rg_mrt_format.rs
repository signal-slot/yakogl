// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! V3D (Raspberry Pi 4): the per-target format, channel mask and bpp class
//! the GL core hands the V3D for an R or RG integer texture drawn as the
//! four-channel surface of its store (`WIDE_INT_RT`, src/gl/framebuffer.rs),
//! at colour attachment 0 and at attachment 1 (gap G2 of
//! docs/notes/v3d-format-matrix.md, plan #383).
//!
//! `es3_formats` keeps each of the twelve formats (R8I .. RG32UI) in the
//! four-channel store of the same channel type and width (R8I in RGBA8I,
//! RG32UI in RGBA32UI). The V3D's render target n takes its output format,
//! tile-buffer bpp class and raster stride from `GlesDraw::targets[n].format`
//! (src/v3d/backend.rs: `state::rt_row(target.rt_format[i])`, stride
//! `target_pitch(rt_w) * bytes`), so each target's format has to be the wide
//! surface of the store, and its write mask the format's own channels.
//!
//! Held, for the twelve formats as a TEXTURE_2D, as layer 1 of a
//! TEXTURE_2D_ARRAY and as level 1 of a mipmapped TEXTURE_2D, each next to
//! an RGBA8 TEXTURE_2D of the same size, every channel on:
//!
//! - the texture at attachment 0, RGBA8 at 1: target 0 is the wide store's
//!   surface (`fb_target_format_n`), writes the format's channels
//!   (`fb_target_channel_mask_n`, through the per-target mask), and its bpp
//!   class and bytes a pixel are the wide surface's; target 1 is RGBA8, 0xf,
//!   32 bpp;
//! - the texture at attachment 1, RGBA8 at 0: the same per target. Here the
//!   format, the bpp class of the 16- and 32-bit formats and the bytes a
//!   pixel are wrong today and the cases are `#[ignore]`d with the cause:
//!   targets past the first come from `fb_target_n` (src/gl/framebuffer.rs
//!   :1398), whose format match names the integer format's own narrow
//!   surface (`renderbuffer_surf_format(f)`, :1439) instead of the store's
//!   wide one, as `fb_target_format_n` (:1083) does for target 0
//!   (review wide-int-store-review). The write mask is right there already
//!   (`per_target_masks` asks `fb_target_channel_mask_n` per attachment).
//!
//! What it cannot see: the GPU. The back end here is a stand-in that records
//! what `GlesDraw` carries and what `state::refusal`, `state::unsupported`
//! and `state::job_state` make of it; the bpp class and bytes a pixel are
//! `state::rt_row` of each target's format, the row the V3D back end itself
//! configures the RT from. Nothing is rasterized and nothing here is
//! verified on hardware.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online --test es3_v3d_int_rg_mrt_format
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
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
    /// `targets[..num_targets]`: memory, size, format.
    targets: Vec<(u64, u32, u32, u8)>,
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
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
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
    name: c"es3-v3d-int-rg-mrt-format".as_ptr() as *const c_char,
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
    /// The four-channel surface of the store `es3_formats` keeps it in.
    wide: u8,
}

const fn row(name: &'static str, ifmt: GLenum, channels: u8, signed: bool, wide: u8) -> Row {
    Row { name, ifmt, channels, signed, wide }
}

const ROWS: [Row; 12] = [
    row("R8I", GL_R8I, 1, true, GLES_SURF_RGBA8I),
    row("R8UI", GL_R8UI, 1, false, GLES_SURF_RGBA8UI),
    row("RG8I", GL_RG8I, 2, true, GLES_SURF_RGBA8I),
    row("RG8UI", GL_RG8UI, 2, false, GLES_SURF_RGBA8UI),
    row("R16I", GL_R16I, 1, true, GLES_SURF_RGBA16I),
    row("R16UI", GL_R16UI, 1, false, GLES_SURF_RGBA16UI),
    row("RG16I", GL_RG16I, 2, true, GLES_SURF_RGBA16I),
    row("RG16UI", GL_RG16UI, 2, false, GLES_SURF_RGBA16UI),
    row("R32I", GL_R32I, 1, true, GLES_SURF_RGBA32I),
    row("R32UI", GL_R32UI, 1, false, GLES_SURF_RGBA32UI),
    row("RG32I", GL_RG32I, 2, true, GLES_SURF_RGBA32I),
    row("RG32UI", GL_RG32UI, 2, false, GLES_SURF_RGBA32UI),
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
    /// 8-bit channels: the narrow surface's tile-buffer bpp class (32) is
    /// the wide one's too, so a wrong format there does not show in the bpp.
    fn eight_bit(&self) -> bool {
        matches!(self.wide, GLES_SURF_RGBA8I | GLES_SURF_RGBA8UI)
    }
}

/* ---- a framebuffer, a program, a draw ------------------------------------ */

/// What the R or RG integer texture is.
#[derive(Clone, Copy, Debug)]
enum Kind {
    /// Level 0 of an 8x8 TEXTURE_2D.
    Texture2d,
    /// Layer 1 of a 3-layer 8x8 TEXTURE_2D_ARRAY.
    ArrayLayer,
    /// Level 1 of a 3-level 8x8 TEXTURE_2D (4x4).
    Level1,
}

impl Kind {
    fn w(self) -> u32 {
        if matches!(self, Kind::Level1) {
            4
        } else {
            8
        }
    }
}

/// A bound framebuffer; dropped, it unbinds and deletes what it made.
struct Fbo {
    fb: GLuint,
    tex: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteTextures(self.tex.len() as GLsizei, self.tex.as_ptr());
            glUseProgram(0);
        }
    }
}

/// The R or RG integer texture of `r` as `kind` at attachment `at` (0 or
/// 1), an RGBA8 TEXTURE_2D of the same size at the other; both are draw
/// buffers. Complete.
fn fbo(r: &Row, kind: Kind, at: u32) -> Fbo {
    let mut f = Fbo { fb: 0, tex: vec![] };
    let w = kind.w() as GLsizei;
    // SAFETY: room for one name each; the layer and level named are there.
    unsafe {
        glGenFramebuffers(1, &mut f.fb);
        glBindFramebuffer(GL_FRAMEBUFFER, f.fb);
        let int_at = GL_COLOR_ATTACHMENT0 + at;
        let other_at = GL_COLOR_ATTACHMENT0 + (1 - at);
        let mut name = 0;
        glGenTextures(1, &mut name);
        match kind {
            Kind::Texture2d | Kind::Level1 => {
                let (levels, level) = if matches!(kind, Kind::Level1) { (3, 1) } else { (1, 0) };
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, levels, r.ifmt, 8, 8);
                glFramebufferTexture2D(GL_FRAMEBUFFER, int_at, GL_TEXTURE_2D, name, level);
            }
            Kind::ArrayLayer => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, name);
                glTexStorage3D(GL_TEXTURE_2D_ARRAY, 1, r.ifmt, 8, 8, 3);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, int_at, name, 0, 1);
            }
        }
        f.tex.push(name);
        let mut other = 0;
        glGenTextures(1, &mut other);
        glBindTexture(GL_TEXTURE_2D, other);
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, w, w);
        glFramebufferTexture2D(GL_FRAMEBUFFER, other_at, GL_TEXTURE_2D, other, 0);
        f.tex.push(other);
        let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT0 + 1];
        glDrawBuffers(2, bufs.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR, "{} {kind:?} at {at}: storage and attach", r.name);
    // SAFETY: plain GL call.
    assert_eq!(
        unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) },
        GL_FRAMEBUFFER_COMPLETE,
        "{} {kind:?} at {at}",
        r.name
    );
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

/* ---- what one target is ---------------------------------------------------- */

/// One target of a captured draw, as the V3D configures its RT from it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Rt {
    format: u8,
    /// The channels it writes (bit 0 R .. bit 3 A).
    mask: u8,
    /// `V3D_INTERNAL_BPP_*` of `state::rt_row(format)`.
    bpp: u32,
    /// Bytes a pixel in memory (`state::rt_row(format).bytes`), the unit of
    /// the RT's raster stride.
    bytes: u32,
    w: u32,
}

/// What an RT of surface `surf`, `w` wide, writing `mask`, is.
fn rt_of(surf: u8, mask: u8, w: u32) -> Rt {
    let row = state::rt_row(surf);
    Rt { format: surf, mask, bpp: row.internal_bpp, bytes: row.bytes, w }
}

/// The two targets of a draw into `fbo(r, kind, at)` with every channel on:
/// (the integer texture's, the RGBA8 texture's). The draw must be taken
/// (no GL error, no refusal, a job state).
fn observe(r: &Row, kind: Kind, at: u32) -> (Rt, Rt) {
    let _f = fbo(r, kind, at);
    let mut tys = ["vec4", "vec4"];
    tys[at as usize] = r.ty();
    program(&tys);
    let what = format!("{} {kind:?} at {at}", r.name);
    let (seen, e) = draw();
    assert_eq!(e, GL_NO_ERROR, "{what}: the draw");
    let s = seen.unwrap_or_else(|| panic!("{what}: the back end saw no draw"));
    assert_eq!(s.refused, None, "{what}");
    if let Err(why) = s.job {
        panic!("{what}: job_state refuses: {why}");
    }
    assert_eq!(s.targets.len(), 2, "{what}: two targets");
    let rts: Vec<Rt> = s
        .targets
        .iter()
        .enumerate()
        .map(|(k, t)| {
            assert_ne!(t.0, 0, "{what}: target {k} has a store");
            assert_eq!((t.1, t.2), (kind.w(), kind.w()), "{what}: target {k}'s size");
            let mask = if s.per_target_mask { s.target_color_mask[k] } else { s.color_mask };
            rt_of(t.3, mask, t.1)
        })
        .collect();
    let other = 1 - at as usize;
    (rts[at as usize], rts[other])
}

/// Which property of the integer target a test pins.
#[derive(Clone, Copy, Debug)]
enum Prop {
    /// The surface (and so the bytes a pixel, the stride's unit).
    Format,
    /// The write mask.
    Mask,
    /// The tile-buffer bpp class.
    Bpp,
}

/// Every one of `rows` as `kind` at `at`: the integer target's `prop` is
/// the wide store's (the format's channels for the mask), and the RGBA8
/// target beside it is RGBA8, every channel, 32 bpp.
fn expect(rows: &[Row], kind: Kind, at: u32, prop: Prop) {
    let _g = context();
    let mut bad = Vec::new();
    for r in rows {
        let (int, other) = observe(r, kind, at);
        assert_eq!(other, rt_of(GLES_SURF_RGBA8888, 0xf, kind.w()), "{} {kind:?} at {at}: the RGBA8 target", r.name);
        let want = rt_of(r.wide, r.own(), kind.w());
        let ok = match prop {
            Prop::Format => (int.format, int.bytes) == (want.format, want.bytes),
            Prop::Mask => int.mask == want.mask,
            Prop::Bpp => int.bpp == want.bpp,
        };
        if !ok {
            bad.push(format!(
                "{}: {prop:?} {} ({} B/px, bpp class {}, mask {:#x}), want {} ({} B/px, bpp class {}, mask {:#x})",
                r.name,
                gles_surf_name(int.format),
                int.bytes,
                int.bpp,
                int.mask,
                gles_surf_name(want.format),
                want.bytes,
                want.bpp,
                want.mask
            ));
        }
    }
    assert!(bad.is_empty(), "{kind:?} at attachment {at}:\n{}", bad.join("\n"));
}

fn eight_bit() -> Vec<Row> {
    ROWS.iter().copied().filter(Row::eight_bit).collect()
}

fn sixteen_and_32_bit() -> Vec<Row> {
    ROWS.iter().copied().filter(|r| !r.eight_bit()).collect()
}

/* ---- attachment 0: the texture first, RGBA8 at 1 -------------------------- */

#[test]
fn att0_texture_2d_format() {
    expect(&ROWS, Kind::Texture2d, 0, Prop::Format);
}
#[test]
fn att0_texture_2d_mask() {
    expect(&ROWS, Kind::Texture2d, 0, Prop::Mask);
}
#[test]
fn att0_texture_2d_bpp() {
    expect(&ROWS, Kind::Texture2d, 0, Prop::Bpp);
}
#[test]
fn att0_array_layer_format() {
    expect(&ROWS, Kind::ArrayLayer, 0, Prop::Format);
}
#[test]
fn att0_array_layer_mask() {
    expect(&ROWS, Kind::ArrayLayer, 0, Prop::Mask);
}
#[test]
fn att0_array_layer_bpp() {
    expect(&ROWS, Kind::ArrayLayer, 0, Prop::Bpp);
}
#[test]
fn att0_level_1_format() {
    expect(&ROWS, Kind::Level1, 0, Prop::Format);
}
#[test]
fn att0_level_1_mask() {
    expect(&ROWS, Kind::Level1, 0, Prop::Mask);
}
#[test]
fn att0_level_1_bpp() {
    expect(&ROWS, Kind::Level1, 0, Prop::Bpp);
}

/* ---- attachment 1: RGBA8 first, the texture at 1 -------------------------- */

#[test]
#[ignore = "fb_target_n names the narrow surface of the format, not the wide store's, for a target past 0 (src/gl/framebuffer.rs:1439, into targets[n].format at src/gl/draw.rs:2830)"]
fn att1_texture_2d_format() {
    expect(&ROWS, Kind::Texture2d, 1, Prop::Format);
}
#[test]
fn att1_texture_2d_mask() {
    expect(&ROWS, Kind::Texture2d, 1, Prop::Mask);
}
#[test]
fn att1_texture_2d_bpp_8_bit() {
    expect(&eight_bit(), Kind::Texture2d, 1, Prop::Bpp);
}
#[test]
#[ignore = "bpp class of the narrow surface fb_target_n names for a target past 0, not the wide store's (src/gl/framebuffer.rs:1439; the tile size at src/v3d/backend.rs:828)"]
fn att1_texture_2d_bpp_16_and_32_bit() {
    expect(&sixteen_and_32_bit(), Kind::Texture2d, 1, Prop::Bpp);
}
#[test]
#[ignore = "fb_target_n names the narrow surface of the format, not the wide store's, for a target past 0 (src/gl/framebuffer.rs:1439, into targets[n].format at src/gl/draw.rs:2830)"]
fn att1_array_layer_format() {
    expect(&ROWS, Kind::ArrayLayer, 1, Prop::Format);
}
#[test]
fn att1_array_layer_mask() {
    expect(&ROWS, Kind::ArrayLayer, 1, Prop::Mask);
}
#[test]
fn att1_array_layer_bpp_8_bit() {
    expect(&eight_bit(), Kind::ArrayLayer, 1, Prop::Bpp);
}
#[test]
#[ignore = "bpp class of the narrow surface fb_target_n names for a target past 0, not the wide store's (src/gl/framebuffer.rs:1439; the tile size at src/v3d/backend.rs:828)"]
fn att1_array_layer_bpp_16_and_32_bit() {
    expect(&sixteen_and_32_bit(), Kind::ArrayLayer, 1, Prop::Bpp);
}
#[test]
#[ignore = "fb_target_n names the narrow surface of the format, not the wide store's, for a target past 0 (src/gl/framebuffer.rs:1439, into targets[n].format at src/gl/draw.rs:2830)"]
fn att1_level_1_format() {
    expect(&ROWS, Kind::Level1, 1, Prop::Format);
}
#[test]
fn att1_level_1_mask() {
    expect(&ROWS, Kind::Level1, 1, Prop::Mask);
}
#[test]
fn att1_level_1_bpp_8_bit() {
    expect(&eight_bit(), Kind::Level1, 1, Prop::Bpp);
}
#[test]
#[ignore = "bpp class of the narrow surface fb_target_n names for a target past 0, not the wide store's (src/gl/framebuffer.rs:1439; the tile size at src/v3d/backend.rs:828)"]
fn att1_level_1_bpp_16_and_32_bit() {
    expect(&sixteen_and_32_bit(), Kind::Level1, 1, Prop::Bpp);
}
