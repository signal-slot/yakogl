// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (AM62P): GL_TEXTURE_BASE_LEVEL, GL_TEXTURE_MAX_LEVEL,
//! GL_TEXTURE_MIN_LOD and GL_TEXTURE_MAX_LOD of a 2D and a cube texture reach
//! the texture state words a draw binds (dEQP-GLES3
//! `texture.mipmap.{2d,cube}.{base_level,max_level,min_lod,max_lod}.*`).
//!
//! Each case uploads a 16x16 RGBA8 texture (a cube: six faces) with its
//! whole chain, every (face, level) its own colour, sets one parameter
//! through glTexParameter (or a sampler object), draws one quad through a
//! `sampler2D` / `samplerCube` with NEAREST_MIPMAP_NEAREST on a stand-in back
//! end with the PowerVR's flags, takes the texture bind the draw hands the
//! back end and the descriptor the PowerVR back end builds from it
//! (`yakogl::pvr::backend::texture_descriptor`, the draw's own builder
//! input, `pvr::texture::TexLod::of_bind`), and reads the level a look-up
//! at LOD lambda would sample through a model of the TPU that knows only
//! those words (texstate.xml, Mesa 26.2.1):
//!
//! - IMAGE_WORD0: width - 1 at [34:47], height - 1 at [48:61];
//! - the linear 2D image (TEXSTATE_STRIDE_IMAGE_WORD1): texaddr (>> 2) at
//!   [16:53], num_mip_levels at [60:63], the levels following each other
//!   from texaddr (pvr_image.c pvr_image_plane_setup_mip_levels);
//! - the native cube (the twiddled TEXSTATE_IMAGE_WORD1): num_mip_levels at
//!   [0:3], texaddr at [16:53], baselevel at [60:63]; image meta dword 4 the
//!   face stride; each face's levels follow each other from level 0;
//! - SAMPLER_WORD0: minlod at [13:22], maxlod at [23:32] (u4.6, from the
//!   base level).
//!
//! The look-up clamps lambda to [minlod, maxlod] and picks the level as
//! ES 3.0.6 3.8.10.4 does for NEAREST_MIPMAP_NEAREST (the base level for
//! lambda <= 1/2, else base + ceil(lambda + 1/2) - 1, at most the last level
//! the words allow), then reads texel (0, 0) of that level. The host model
//! executes no texel fetch: the TPU's walk is this model's.
//!
//! Before fix-baselod-gl (plan #388) the draw built every descriptor with
//! GL's defaults: base level 1 read level 0, MIN_LOD 1.5 and MAX_LOD 0.5
//! were not applied, which is what the dEQP cases above failed on the AM62P.
//! GL_TEXTURE_MAX_LEVEL already reached the words through `mip_levels`.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_base_lod
//! ```
//!
//! Needs the in-process compiler (the quad's programs are not in the
//! offline table). Compiles to nothing under report-es2: the parameters are
//! ES 3.0's.

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{
    GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXTYPE_2D, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::{glBindSampler, glDeleteSamplers, glGenSamplers, glSamplerParameterf, glSamplerParameteri};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::backend::texture_descriptor;

/* ---- the stand-in back end: it records the texture bind of each draw ---- */

// SAFETY (Send): GlesTextureBind holds a raw `cpu` pointer; the tests read it
// only on the thread that drew, while the context lives.
struct SendBind(GlesTextureBind);
unsafe impl Send for SendBind {}
static BIND_SLOT: Mutex<Option<SendBind>> = Mutex::new(None);

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
    if d.num_textures > 0 {
        *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = Some(SendBind(d.textures[0]));
    }
    0
}

/* The two tables are built from zero (every hook absent, every flag clear)
 * with the fields this file uses set: a field another change adds to
 * YakoglBackend or OsalOps needs no edit here (CLAUDE.md, "Two green pull
 * requests, red main"). Zero is the PowerVR's shape for the flags that
 * matter here: packed mip levels (`mip_level_align` 0), and the TPU walks
 * no layers itself (`native_layered_textures` false). */
static PVR_SHAPE: YakoglBackend = {
    // SAFETY: YakoglBackend is a pointer, Options of fn pointers, bools and
    // integers, for which all-zero is a valid value (null, None, false, 0).
    let mut b: YakoglBackend = unsafe { core::mem::zeroed() };
    b.name = c"pvr base/lod stand-in".as_ptr() as *const c_char;
    b.init = Some(b_init);
    b.draw = Some(b_draw);
    b.flush = Some(b_ok);
    b.finish = Some(b_ok);
    b.heap_base = Some(b_heap_base);
    b.max_instance_divisor = 1;
    b
};

/// Pointer-valued device addresses: a GPU address is its CPU pointer.
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

static SERIAL: Mutex<()> = Mutex::new(());

/// A fresh context on the PowerVR shape, for the length of `case`.
fn on_powervr(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &PVR_SHAPE;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL plumbing ---- */

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

const VS: &str = "attribute vec4 p;\nvarying vec3 v;\nvoid main() { v = vec3(p.xy, 1.0); gl_Position = p; }\n";
const FS_2D: &str = "precision mediump float;\nuniform sampler2D s;\nvarying vec3 v;\n\
                     void main() { gl_FragColor = texture2D(s, v.xy); }\n";
const FS_CUBE: &str = "precision mediump float;\nuniform samplerCube s;\nvarying vec3 v;\n\
                       void main() { gl_FragColor = textureCube(s, v); }\n";
static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad sampling unit 0 through `fs`; the bind the back end was handed.
fn draw_quad(fs: &str) -> GlesTextureBind {
    *BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()) = None;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
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
    no_error("the quad");
    BIND_SLOT.lock().unwrap_or_else(|e| e.into_inner()).take().expect("the quad was drawn with a texture").0
}

/// 16x16, levels 16 .. 1.
const SIDE: u32 = 16;
const LEVELS: u32 = 5;

/// The colour of every texel of level `level` of face `face` (0 for 2D).
fn colour(face: u32, level: u32) -> [u8; 4] {
    [0x40 | level as u8, 0x10 * face as u8 + 1, 0x5a, 0xff]
}

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

/// A complete texture of `target` (GL_TEXTURE_2D or GL_TEXTURE_CUBE_MAP),
/// bound to unit 0, NEAREST_MIPMAP_NEAREST / NEAREST.
fn texture(target: GLenum) -> GLuint {
    let faces: &[GLenum] = if target == GL_TEXTURE_2D { &[GL_TEXTURE_2D] } else { &FACES };
    let mut name = 0;
    // SAFETY: plain GL calls; each level's pixels are a live Vec.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for (f, &face) in faces.iter().enumerate() {
            for l in 0..LEVELS {
                let s = SIDE >> l;
                let px: Vec<u8> = (0..s * s).flat_map(|_| colour(f as u32, l)).collect();
                glTexImage2D(
                    face,
                    l as GLint,
                    GL_RGBA as GLint,
                    s as GLsizei,
                    s as GLsizei,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    px.as_ptr() as *const c_void,
                );
            }
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    no_error("the texture");
    name
}

/* ---- the TPU model ---- */

fn word(w: &[u32], i: usize) -> u64 {
    u64::from(w[i]) | (u64::from(w[i + 1]) << 32)
}

fn bits(v: u64, lo: u32, hi: u32) -> u64 {
    (v >> lo) & ((1u64 << (hi - lo + 1)) - 1)
}

/// What the words say about the image and the sampler.
#[derive(Debug)]
struct Image {
    /// The address of the first level the words name.
    addr: u64,
    /// IMAGE_WORD0's extent (a power of two here): the first named level's
    /// for the linear image, level 0's for the cube (its base level is
    /// baselevel's).
    side: u32,
    /// The GL level the first named level is (the cube's baselevel; the
    /// linear image's is where its address points, found by the reader).
    first: u32,
    /// The levels walked from `first`.
    levels: u32,
    /// The cube's face stride (0 for 2D).
    face_bytes: u64,
    minlod: f32,
    maxlod: f32,
}

fn image(t: &GlesTextureBind, w: &[u32]) -> Image {
    let iw0 = word(w, 0);
    let iw1 = word(w, 2);
    let sw0 = word(w, 8);
    let width = bits(iw0, 34, 47) as u32 + 1;
    let height = bits(iw0, 48, 61) as u32 + 1;
    assert_eq!(width, height, "a square image");
    /* texaddr holds 40 bits of the address; a host pointer has more, which
     * the bind's own address supplies (the image is not 1 TiB long) */
    const LOW: u64 = (1 << 40) - 1;
    let addr = (t.base_gpu & !LOW) | (bits(iw1, 16, 53) << 2);
    let minlod = bits(sw0, 13, 22) as f32 / 64.0;
    let maxlod = bits(sw0, 23, 32) as f32 / 64.0;
    if t.type_ == GLES_TEXTYPE_CUBE_TWIDDLED_PROBE {
        let first = bits(iw1, 60, 63) as u32;
        let chain = bits(iw1, 0, 3) as u32;
        assert!(chain > first, "baselevel {first} inside the copy's {chain} levels");
        /* the copy holds level 0 on: the walk starts at level `first` */
        Image { addr, side: width, first, levels: chain - first, face_bytes: u64::from(w[4]), minlod, maxlod }
    } else {
        assert_eq!(t.type_, GLES_TEXTYPE_2D, "a linear 2D image");
        Image { addr, side: width, first: 0, levels: bits(iw1, 60, 63) as u32, face_bytes: 0, minlod, maxlod }
    }
}

/// The level (counted from the image's first named level) a
/// NEAREST_MIPMAP_NEAREST look-up at `lambda` samples (ES 3.0.6 3.8.10.4,
/// the LOD clamped to the sampler's range first, 3.8.10).
fn level_at(img: &Image, lambda: f32) -> u32 {
    let l = lambda.clamp(img.minlod, img.maxlod);
    let d = if l <= 0.5 { 0 } else { ((l + 0.5).ceil() as u32).saturating_sub(1) };
    d.min(img.levels.saturating_sub(1))
}

/// The bytes of texel (0, 0) of level `k` (from the first named level) of
/// face `face`: the levels follow each other, each `side >> k` square at
/// 4 bytes a texel; the cube's chain starts at level 0 of the face.
fn texel(img: &Image, face: u32, k: u32) -> [u8; 4] {
    let at = |side: u32, n: u32| -> u64 { (0..n).map(|j| u64::from((side >> j).max(1)).pow(2) * 4).sum() };
    let off = if img.face_bytes != 0 {
        u64::from(face) * img.face_bytes + at(img.side, img.first + k)
    } else {
        at(img.side, k)
    };
    // SAFETY: the address is a live host allocation of the GL storage or
    // the cube's copy (pointer-valued device addresses), and the offset is
    // inside the chain the words describe.
    unsafe { *((img.addr + off) as *const [u8; 4]) }
}

/// The GL level a look-up at `lambda` reads from `face`, found by its colour.
fn sampled(t: &GlesTextureBind, face: u32, lambda: f32) -> u32 {
    let img = image(t, &texture_descriptor(t));
    let k = level_at(&img, lambda);
    let got = texel(&img, face, k);
    (0..LEVELS)
        .find(|&l| colour(face, l) == got)
        .unwrap_or_else(|| panic!("lambda {lambda}: {got:x?} is no level's colour ({img:?})"))
}

/// The GL level read at LOD 0, 1, 2, 3 and 4.
fn walk(t: &GlesTextureBind, face: u32) -> [u32; 5] {
    core::array::from_fn(|i| sampled(t, face, i as f32))
}

fn param(target: GLenum, pname: GLenum, v: f32) {
    // SAFETY: plain GL call on the bound texture.
    unsafe {
        if matches!(pname, GL_TEXTURE_MIN_LOD | GL_TEXTURE_MAX_LOD) {
            glTexParameterf(target, pname, v);
        } else {
            glTexParameteri(target, pname, v as GLint);
        }
    }
    no_error("the parameter");
}

/// The level walks of `target` for each parameter, with `fs` drawing it;
/// the cube's are read on face `face`.
fn check(target: GLenum, fs: &str, ty: u8, face: u32) {
    let name = texture(target);
    let t = draw_quad(fs);
    assert_eq!(t.type_, ty);
    assert_eq!((t.base_level, t.lod_range()), (0, (-127.0, 127.0)), "GL's defaults, saturated");
    assert_eq!(walk(&t, face), [0, 1, 2, 3, 4], "the defaults: level 0 and the whole chain");

    param(target, GL_TEXTURE_BASE_LEVEL, 1.0);
    let t = draw_quad(fs);
    assert_eq!(t.base_level, 1);
    assert_eq!(walk(&t, face), [1, 2, 3, 4, 4], "base level 1: LOD 0 reads level 1");
    param(target, GL_TEXTURE_BASE_LEVEL, 0.0);

    param(target, GL_TEXTURE_MAX_LEVEL, 1.0);
    assert_eq!(walk(&draw_quad(fs), face), [0, 1, 1, 1, 1], "max level 1: never past level 1");
    param(target, GL_TEXTURE_MAX_LEVEL, 1000.0);

    param(target, GL_TEXTURE_MIN_LOD, 1.5);
    let t = draw_quad(fs);
    assert_eq!(t.lod_range(), (1.5, 127.0));
    assert_eq!(walk(&t, face), [1, 1, 2, 3, 4], "min LOD 1.5: LOD 0 reads level 1");
    param(target, GL_TEXTURE_MIN_LOD, -1000.0);

    param(target, GL_TEXTURE_MAX_LOD, 0.5);
    let t = draw_quad(fs);
    assert_eq!(t.lod_range(), (-127.0, 0.5));
    assert_eq!(walk(&t, face), [0, 0, 0, 0, 0], "max LOD 0.5: level 0 alone");
    param(target, GL_TEXTURE_MAX_LOD, 1000.0);

    /* the LOD range is counted from the base level (ES 3.0.6 3.8.10.4:
     * level_base + lambda) */
    param(target, GL_TEXTURE_BASE_LEVEL, 1.0);
    param(target, GL_TEXTURE_MIN_LOD, 1.5);
    assert_eq!(walk(&draw_quad(fs), face), [2, 2, 3, 4, 4], "base 1, min LOD 1.5: LOD 0 reads level 2");
    param(target, GL_TEXTURE_MIN_LOD, -1000.0);
    param(target, GL_TEXTURE_MAX_LOD, 0.5);
    assert_eq!(walk(&draw_quad(fs), face), [1, 1, 1, 1, 1], "base 1, max LOD 0.5: level 1 alone");
    param(target, GL_TEXTURE_MAX_LOD, 1000.0);
    param(target, GL_TEXTURE_BASE_LEVEL, 0.0);

    /* back to the defaults: the words of before */
    assert_eq!(walk(&draw_quad(fs), face), [0, 1, 2, 3, 4], "the defaults again");
    // SAFETY: the texture made above.
    unsafe { glDeleteTextures(1, &name) };
}

#[test]
fn a_2d_texture_samples_the_level_its_parameters_name() {
    on_powervr(|| check(GL_TEXTURE_2D, FS_2D, GLES_TEXTYPE_2D, 0));
}

#[test]
fn a_cube_texture_samples_the_level_its_parameters_name() {
    on_powervr(|| {
        for face in [0, 5] {
            check(GL_TEXTURE_CUBE_MAP, FS_CUBE, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, face);
        }
    });
}

/// A sampler object bound to the unit replaces the texture's MIN_LOD and
/// MAX_LOD (ES 3.0.6 3.8.2); unbinding it puts the texture's back.
#[test]
fn a_sampler_objects_lod_range_replaces_the_textures() {
    on_powervr(|| {
        let name = texture(GL_TEXTURE_2D);
        param(GL_TEXTURE_2D, GL_TEXTURE_MAX_LOD, 0.5);
        let mut s = 0;
        // SAFETY: plain GL calls; room for one name.
        unsafe {
            glGenSamplers(1, &mut s);
            glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
            glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glSamplerParameterf(s, GL_TEXTURE_MIN_LOD, 2.0);
            glBindSampler(0, s);
        }
        no_error("the sampler");
        let t = draw_quad(FS_2D);
        assert_eq!(t.lod_range(), (2.0, 127.0), "the sampler's, not the texture's MAX_LOD");
        assert_eq!(walk(&t, 0), [2, 2, 2, 3, 4]);
        // SAFETY: plain GL calls on the sampler made above.
        unsafe {
            glBindSampler(0, 0);
            glDeleteSamplers(1, &s);
        }
        let t = draw_quad(FS_2D);
        assert_eq!(t.lod_range(), (-127.0, 0.5), "the texture's again");
        assert_eq!(walk(&t, 0), [0, 0, 0, 0, 0]);
        // SAFETY: the texture made above.
        unsafe { glDeleteTextures(1, &name) };
    });
}

/// The bind's LOD fields: a zeroed bind is GL's defaults, and a value is
/// kept in 1/256 of a level, rounded down, saturated.
#[test]
fn the_binds_lod_range_round_trips() {
    let mut t = GlesTextureBind::ZERO;
    assert_eq!(t.lod_range(), (-1000.0, 1000.0));
    t.set_lod_range(1.5, 0.5);
    assert_eq!(t.lod_range(), (1.5, 0.5));
    t.set_lod_range(-1000.0, 1000.0);
    assert_eq!(t.lod_range(), (-127.0, 127.0));
    t.set_lod_range(0.3, f32::NAN);
    assert_eq!(t.lod_range(), (76.0 / 256.0, 0.0));
}
