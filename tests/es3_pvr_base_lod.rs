// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL, GL_TEXTURE_MIN_LOD and
//! GL_TEXTURE_MAX_LOD of a 2D and a cube texture reach the PowerVR's texture
//! state words (ES 3.0.6 3.8.10: lambda is clamped to [min_lod, max_lod];
//! 3.8.13: the levels walked are level_base up to q = min(p, max_level);
//! 3.8.2: a sampler object bound to the unit replaces the texture's LODs).
//!
//! fix-baselod-pvr (253dd427) taught the descriptor builder to encode them
//! (`pvr::texture::TexLod`, tests/pvr_tex_lod_words.rs), and 4c0bcd77 the
//! 2D bind's base level. Here the GL core carries the rest: the bind's
//! `min_lod` / `max_lod` (src/backend.rs, 1/64 levels, the maximum plus one,
//! 0 GL's default) from the texture's or the sampler object's state
//! (`gl::draw::bind_lod_range`), and `pvr::texture::bind_lod` reads them, and
//! the native cube's base level, for the draw's descriptor.
//!
//! Through the GL API on the host, a stand-in back end with the PowerVR's
//! layout fields (all zero, as `pvr::backend`'s table) keeps the bind of each
//! draw; `pvr::backend::texture_descriptor` builds the descriptor the
//! PowerVR draw binds with from it. Checked, from texstate.xml's fields:
//! SAMPLER_WORD0 minlod [13:22] and maxlod [23:32] (u4.6), IMAGE_WORD1
//! baselevel [60:63] of the native cube's twiddled word, and the level
//! count a linear 2D image states (STRIDE_IMAGE_WORD1 num_mip_levels
//! [60:63]). GL's defaults keep the words a zeroed LOD range packs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_base_lod
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{
    GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXTYPE_2D, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::backend::texture_descriptor;

/* ---- the stand-in back end: it keeps the first texture bind of the last draw */

struct Seen(Option<GlesTextureBind>);
// SAFETY: the bind's `cpu` pointer is only carried, never dereferenced
// through SEEN; the tests are serialised by SERIAL.
unsafe impl Send for Seen {}
static SEEN: Mutex<Seen> = Mutex::new(Seen(None));

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    assert!(d.num_textures >= 1, "the program samples one texture");
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).0 = Some(d.textures[0]);
    0
}

/* Every field not named here is zero, as in pvr::backend's own table for
 * the layout fields, so a field another change adds needs no edit here
 * (CLAUDE.md, "Two green pull requests, red main"). */
static BACKEND: YakoglBackend = {
    // SAFETY: YakoglBackend is a pointer, Options of fn pointers, bools and
    // integers, for which all-zero is a valid value.
    let mut b: YakoglBackend = unsafe { core::mem::zeroed() };
    b.name = c"es3-pvr-base-lod".as_ptr() as *const c_char;
    b.init = Some(b_init);
    b.draw = Some(b_draw);
    b.flush = Some(b_ok);
    b.finish = Some(b_ok);
    b.heap_base = Some(b_heap_base);
    b.max_instance_divisor = 1;
    b
};

unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is the core's.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = {
    // SAFETY: OsalOps is Options of fn pointers, for which all-zero is None.
    let mut o: OsalOps = unsafe { core::mem::zeroed() };
    o.gpu_alloc = Some(o_gpu_alloc);
    o.gpu_free = Some(o_gpu_free);
    o.cache_flush = Some(o_cache);
    o.cache_invd = Some(o_cache);
    o
};

/// One case at a time: the GL context, the compiler's arena and its
/// switches are process-wide.
static SERIAL: Mutex<()> = Mutex::new(());

fn on_fresh_context(case: impl Fn()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL plumbing ------------------------------------------------------------ */

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

const VS: &str = "#version 300 es\nin vec4 p;\nout vec3 v;\nvoid main() { v = vec3(p.xy, 1.0); gl_Position = p; }\n";

/// A fragment program sampling `kind` (sampler2D or samplerCube) on unit 0.
fn fs(kind: &str) -> String {
    let coord = if kind == "samplerCube" { "v" } else { "v.xy * 0.5 + 0.5" };
    format!(
        "#version 300 es\nprecision highp float;\nuniform highp {kind} s;\nin vec3 v;\nout vec4 o;\n\
         void main() {{ o = texture(s, {coord}); }}\n"
    )
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad with `kind`'s program over texture unit 0: the bind the draw
/// handed the back end.
fn draw_bind(kind: &str) -> GlesTextureBind {
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).0 = None;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &fs(kind)));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    SEEN.lock().unwrap_or_else(|e| e.into_inner()).0.expect("the quad was drawn")
}

/// A 64x64 RGBA8 texture of `target` (GL_TEXTURE_2D or GL_TEXTURE_CUBE_MAP)
/// with all 7 levels (every face), `min_filter` and NEAREST magnification,
/// bound to unit 0.
fn texture(target: GLenum, min_filter: GLenum) {
    let mut name: GLuint = 0;
    let faces: Vec<GLenum> = if target == GL_TEXTURE_CUBE_MAP {
        (0..6).map(|f| GL_TEXTURE_CUBE_MAP_POSITIVE_X + f).collect()
    } else {
        vec![target]
    };
    // SAFETY: one name; each level's pixels cover its extent.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
        glTexStorage2D(target, 7, GL_RGBA8, 64, 64);
        for l in 0..7u32 {
            let n = 64 >> l;
            let px = vec![0x40 | l as u8; (n * n * 4) as usize];
            for &f in &faces {
                glTexSubImage2D(f, l as GLint, 0, 0, n, n, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
            }
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, min_filter as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    no_error("texture");
}

fn param_f(target: GLenum, pname: GLenum, v: f32) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameterf(target, pname, v) };
    no_error("glTexParameterf");
}

fn param_i(target: GLenum, pname: GLenum, v: GLint) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target, pname, v) };
    no_error("glTexParameteri");
}

/* ---- the descriptor's fields (texstate.xml) ---------------------------------- */

fn word(w: &[u32], i: usize) -> u64 {
    u64::from(w[i]) | (u64::from(w[i + 1]) << 32)
}

fn bits(v: u64, lo: u32, hi: u32) -> u64 {
    (v >> lo) & ((1u64 << (hi - lo + 1)) - 1)
}

/// SAMPLER_WORD0 (dwords 8, 9) minlod [13:22] and maxlod [23:32], u4.6.
fn lod_range(desc: &[u32]) -> (u64, u64) {
    let s = word(desc, 8);
    (bits(s, 13, 22), bits(s, 23, 32))
}

/// IMAGE_WORD1 (dwords 2, 3) [60:63]: a twiddled word's baselevel, the
/// STRIDE word's num_mip_levels.
fn word1_top(desc: &[u32]) -> u64 {
    bits(word(desc, 2), 60, 63)
}

/// The descriptor of `t` with GL's default LOD range: what a bind without
/// the fields (a zeroed range) is described as.
fn without_lod(t: &GlesTextureBind) -> [u32; 20] {
    let mut z = *t;
    (z.min_lod, z.max_lod) = (0, 0);
    texture_descriptor(&z)
}

/* ---- the cases -------------------------------------------------------------- */

/// GL's defaults: the bind's range is (0, 0) and the words are the ones a
/// mipmapped 64x64 texture always had (maxlod 6 levels).
#[test]
fn defaults_keep_the_words() {
    on_fresh_context(|| {
        texture(GL_TEXTURE_2D, GL_LINEAR_MIPMAP_LINEAR);
        let t = draw_bind("sampler2D");
        assert_eq!(t.type_, GLES_TEXTYPE_2D);
        assert_eq!((t.min_lod, t.max_lod), (0, 0), "GL's default range is a zeroed one");
        let d = texture_descriptor(&t);
        assert_eq!(d, without_lod(&t));
        assert_eq!(lod_range(&d), (0, 6 * 64), "the chain's six levels past the base");
    });
}

/// The texture's GL_TEXTURE_MIN_LOD and MAX_LOD in SAMPLER_WORD0.
#[test]
fn texture_min_max_lod_2d() {
    on_fresh_context(|| {
        texture(GL_TEXTURE_2D, GL_LINEAR_MIPMAP_LINEAR);
        param_f(GL_TEXTURE_2D, GL_TEXTURE_MIN_LOD, 1.5);
        param_f(GL_TEXTURE_2D, GL_TEXTURE_MAX_LOD, 3.25);
        let t = draw_bind("sampler2D");
        assert_eq!((t.min_lod, t.max_lod), (96, 208 + 1), "1/64 levels, the maximum plus one");
        let d = texture_descriptor(&t);
        assert_eq!(lod_range(&d), (96, 208));
        assert_ne!(d, without_lod(&t), "the range moved the words");
        /* a negative minimum is 0, a maximum past the chain the chain's end */
        param_f(GL_TEXTURE_2D, GL_TEXTURE_MIN_LOD, -3.0);
        param_f(GL_TEXTURE_2D, GL_TEXTURE_MAX_LOD, 12.0);
        let t = draw_bind("sampler2D");
        assert_eq!(lod_range(&texture_descriptor(&t)), (0, 6 * 64));
    });
}

/// MAX_LOD 0 on a one-level-sampled texture whose filters differ: lambda is
/// clamped to 0, so every pixel magnifies (ES 3.0.6 3.8.11 chooses on the
/// clamped lambda): maxlod 0, not the 1.0 the builder gives a min != mag
/// sampler to let it minify.
#[test]
fn max_lod_zero_magnifies() {
    on_fresh_context(|| {
        texture(GL_TEXTURE_2D, GL_LINEAR);
        let t = draw_bind("sampler2D");
        assert_eq!(lod_range(&texture_descriptor(&t)), (0, 64), "min != mag: LOD 1.0 lets it minify");
        param_f(GL_TEXTURE_2D, GL_TEXTURE_MAX_LOD, 0.0);
        let t = draw_bind("sampler2D");
        assert_eq!(t.max_lod, 1, "LOD 0, plus one");
        assert_eq!(lod_range(&texture_descriptor(&t)), (0, 0));
    });
}

/// A sampler object bound to the unit replaces the texture's LOD range
/// (ES 3.0.6 3.8.2), and unbinding it brings the texture's back.
#[test]
fn sampler_object_lod_replaces_the_textures() {
    on_fresh_context(|| {
        texture(GL_TEXTURE_2D, GL_LINEAR_MIPMAP_LINEAR);
        param_f(GL_TEXTURE_2D, GL_TEXTURE_MIN_LOD, 1.0);
        let mut s: GLuint = 0;
        // SAFETY: plain GL calls; one name out.
        unsafe {
            glGenSamplers(1, &mut s);
            glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glSamplerParameterf(s, GL_TEXTURE_MIN_LOD, 2.0);
            glSamplerParameterf(s, GL_TEXTURE_MAX_LOD, 4.5);
            glBindSampler(0, s);
        }
        no_error("sampler");
        let t = draw_bind("sampler2D");
        assert_eq!(lod_range(&texture_descriptor(&t)), (128, 288), "the sampler object's range");
        // SAFETY: a plain GL call.
        unsafe { glBindSampler(0, 0) };
        let t = draw_bind("sampler2D");
        assert_eq!(lod_range(&texture_descriptor(&t)), (64, 6 * 64), "the texture's own range");
    });
}

/// GL_TEXTURE_MAX_LEVEL caps the levels walked (the GL core stops
/// `mip_levels` at it), and with GL_TEXTURE_BASE_LEVEL the linear image
/// counts them from the base: base 1, max level 3 is three levels, LOD 2
/// past the base; MAX_LOD clamps inside that.
#[test]
fn base_and_max_level_2d() {
    on_fresh_context(|| {
        texture(GL_TEXTURE_2D, GL_NEAREST_MIPMAP_NEAREST);
        param_i(GL_TEXTURE_2D, GL_TEXTURE_BASE_LEVEL, 1);
        param_i(GL_TEXTURE_2D, GL_TEXTURE_MAX_LEVEL, 3);
        let t = draw_bind("sampler2D");
        assert_eq!((t.base_level, t.mip_levels), (1, 4));
        let d = texture_descriptor(&t);
        assert_eq!(word1_top(&d), 3, "STRIDE num_mip_levels: levels 1..3");
        assert_eq!(lod_range(&d), (0, 2 * 64));
        param_f(GL_TEXTURE_2D, GL_TEXTURE_MAX_LOD, 1.5);
        let d = texture_descriptor(&draw_bind("sampler2D"));
        assert_eq!(lod_range(&d), (0, 96));
    });
}

/// The native cube: a mipmapped copy holds the whole chain from level 0, so
/// GL_TEXTURE_BASE_LEVEL goes into IMAGE_WORD1.baselevel, and the LOD range
/// (from the base) into SAMPLER_WORD0. Before, the cube was sampled from
/// level 0 with GL's default range whatever the texture said.
#[test]
fn native_cube_base_level_and_lod() {
    on_fresh_context(|| {
        texture(GL_TEXTURE_CUBE_MAP, GL_LINEAR_MIPMAP_LINEAR);
        let t = draw_bind("samplerCube");
        assert_eq!(t.type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, "the online compiler samples the native cube");
        let d0 = texture_descriptor(&t);
        assert_eq!((word1_top(&d0), lod_range(&d0)), (0, (0, 6 * 64)), "GL's defaults");

        param_i(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_BASE_LEVEL, 2);
        param_f(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_LOD, 0.5);
        param_f(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAX_LOD, 2.0);
        let t = draw_bind("samplerCube");
        assert_eq!(t.type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE);
        assert_eq!((t.base_level, t.min_lod, t.max_lod), (2, 32, 129));
        let d = texture_descriptor(&t);
        assert_eq!(word1_top(&d), 2, "IMAGE_WORD1.baselevel");
        assert_eq!(lod_range(&d), (32, 128), "inside levels 2..6, four past the base");
        /* the face stride and level count are the copy's, as before */
        assert_eq!(&d[0..2], &d0[0..2], "IMAGE_WORD0");
        assert_eq!(bits(word(&d, 2), 0, 59), bits(word(&d0, 2), 0, 59), "IMAGE_WORD1 below baselevel");
        assert_eq!(d[4], d0[4], "the face size");
    });
}

/// A cube sampled at one level gets a one-level copy of level 0, which the
/// builder never gives a base level; its LOD range still applies.
#[test]
fn one_level_cube_keeps_base_zero() {
    on_fresh_context(|| {
        texture(GL_TEXTURE_CUBE_MAP, GL_LINEAR);
        param_i(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_BASE_LEVEL, 1);
        param_f(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAX_LOD, 0.0);
        let t = draw_bind("samplerCube");
        assert_eq!(t.type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE);
        let d = texture_descriptor(&t);
        assert_eq!(word1_top(&d), 0, "a one-level copy is level 0 alone");
        assert_eq!(lod_range(&d), (0, 0), "MAX_LOD 0 magnifies");
    });
}
