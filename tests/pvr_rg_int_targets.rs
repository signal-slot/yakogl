// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: an R or RG integer colour texture (GL_R8I .. GL_RG32UI) rendered
//! into through a framebuffer object and then sampled reads alpha 1, as
//! ES 3.0.6 3.8.15 and table 3.24 say for a format without alpha.
//!
//! The PowerVR samples no packed one- or two-channel integer texel
//! (`PVR_SAMPLED_TEXFMTS`), so the GL core keeps such a texture in the
//! four-channel integer class of the same width and sign (R8UI in RGBA8UI,
//! RG32I in RGBA32I, ...) and draws it as that class's surface. The store's
//! alpha lane is then not 1: glTexImage2D(..., NULL) clears the store to
//! 0, and a draw packs all four lanes from the shader's output. dEQP-GLES3
//! writes 0 into the channels a format lacks (the fbo cases' output scale
//! and bias are `valueMax - valueMin` and `valueMin` of
//! tcu::getTextureFormatInfo, 0 for an absent channel) and samples the
//! texture with an `isampler2D` / `usampler2D` to the window. The PowerVR
//! read (r, 0, 0, 0) and the reference (r, 0, 0, 1): on the AM62P at
//! b25997e4 the 72 R/RG integer lines of fbo.color.tex2d, tex2darray,
//! texcube, invalidate.format, render.resize and render.recreate_color
//! (docs/es3/am62p-gles3-notpass.tsv, docs/es3/fbo-fix-designs.md section 1)
//! and, with the 3D slice cause, the 12 of fbo.color.tex3d.
//!
//! The fix (src/pvr/backend.rs `int_store_alpha`): the texture descriptor
//! of a four-channel integer class whose bind says `no_alpha` (the GL core
//! sets it for GL_RED_INTEGER, GL_RG_INTEGER and GL_RGB_INTEGER) takes
//! SWIZ.ONE for alpha, in every arm (2D, the stacked and native 2D array,
//! the grid of slices and the native 3D, the stacked and native cube).
//!
//! The GL calls go through the GL core to a stand-in back end (the harness
//! of tests/pvr_native3d_sampler_words.rs), which packs each bind it is
//! handed with the PowerVR's own descriptor builder
//! (`pvr::backend::texture_descriptor`, what `pvr_backend_draw` packs). What
//! is pinned:
//!
//! - per R/RG integer format: the texture is complete as a colour
//!   attachment, a draw into it is accepted, and the draw that samples it
//!   binds the four-channel class with `no_alpha` and a descriptor whose
//!   swizzle reads R (and G) from the texel, B from the texel and alpha as
//!   ONE;
//! - the RGBA integer formats as the contrast: alpha from the texel;
//! - the same alpha in the 2D array, 3D and cube arms, with the native
//!   switches on and off;
//! - GL_TEXTURE_SWIZZLE_R = GL_ALPHA on an R8UI texture reads ONE in red.
//!
//! Without the fix the R/RG rows fail: alpha reads SRC_CHAN3.
//!
//! What it cannot see: the GPU. That the TPU's SWIZ.ONE gives an integer 1
//! to an integer sampler is how the L8, R16F and RGB9E5 descriptors already
//! read their alpha, but for an integer class it is the board's to show.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_rg_int_targets
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    self, GlesDraw, GlesHeap, YakoglBackend, GLES_TEXFMT_RGBA16I, GLES_TEXFMT_RGBA16UI, GLES_TEXFMT_RGBA32I,
    GLES_TEXFMT_RGBA32UI, GLES_TEXFMT_RGBA8I, GLES_TEXFMT_RGBA8UI, GLES_TEXTYPE_2D_ARRAY,
    GLES_TEXTYPE_2D_ARRAY_TWIDDLED, GLES_TEXTYPE_3D, GLES_TEXTYPE_3D_TWIDDLED, GLES_TEXTYPE_CUBE,
    GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
};
use yakogl::gl::buffer::*;
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
use yakogl::pvr::backend::texture_descriptor;
use yakogl::shaderc::{
    pvr_set_native_3d, pvr_set_native_array, pvr_set_native_cube, PVR_NATIVE_3D_DEFAULT, PVR_NATIVE_ARRAY_DEFAULT,
    PVR_NATIVE_CUBE_DEFAULT,
};

/* ---- a GL context on the host: host memory, and a stand-in back end that
 * keeps each draw's texture binds with the descriptor the PowerVR packs for
 * them ---- */

/// What the tests read of a texture bind (the bind itself holds a pointer,
/// so it cannot wait in a static).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bind {
    format: u8,
    type_: u8,
    no_alpha: u8,
}

/// One texture bind of a draw and its descriptor words.
type Bound = (Bind, Vec<u32>);

/// The binds of the last draw the back end received, and how many draws it
/// received.
static LAST: Mutex<(Vec<Bound>, u32)> = Mutex::new((Vec::new(), 0));

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
/// The descriptor is packed here, inside the draw, as the PowerVR packs it
/// (a native volume's reads the view its copy recorded for this draw).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let binds = d.textures[..d.num_textures as usize]
        .iter()
        .map(|t| {
            let b = Bind { format: t.format, type_: t.type_, no_alpha: t.no_alpha };
            (b, texture_descriptor(t).to_vec())
        })
        .collect();
    let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    *last = (binds, last.1 + 1);
    0
}

/* Built from zero with the fields this file uses set, so a field another
 * change adds to YakoglBackend or OsalOps needs no edit here (CLAUDE.md,
 * "Two green pull requests, red main"). */
static OSAL: OsalOps = {
    // SAFETY: OsalOps is Options of fn pointers, for which all-zero is None.
    let mut o: OsalOps = unsafe { core::mem::zeroed() };
    o.gpu_alloc = Some(m_gpu_alloc);
    o.gpu_free = Some(m_gpu_free);
    o.cache_flush = Some(m_cache);
    o.cache_invd = Some(m_cache);
    o
};

static BACKEND: YakoglBackend = {
    // SAFETY: YakoglBackend is a pointer, Options of fn pointers, bools and
    // integers, for which all-zero is a valid value (null, None, false, 0).
    let mut b: YakoglBackend = unsafe { core::mem::zeroed() };
    b.name = c"pvr-rg-int-targets".as_ptr() as *const c_char;
    b.init = Some(b_init);
    b.draw = Some(b_draw);
    b.flush = Some(b_ok);
    b.finish = Some(b_ok);
    b.heap_base = Some(b_heap_base);
    b.max_instance_divisor = 1;
    b
};

/// One context for the whole file; the tests take turns on it. The native
/// switches are process-wide, so the lock also serialises their settings.
fn context() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics; the lock serialises the GL users.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// The three native-target switches set for the closure (cube, 2D array,
/// 3D), and back at their defaults after it, panic or not. The caller holds
/// [`context`]'s lock. A switch is read when a shader compiles, so the
/// programs are made inside.
fn with_switches<T>(cube: bool, array: bool, d3: bool, f: impl FnOnce() -> T) -> T {
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            pvr_set_native_cube(PVR_NATIVE_CUBE_DEFAULT);
            pvr_set_native_array(PVR_NATIVE_ARRAY_DEFAULT);
            pvr_set_native_3d(PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _back = Back;
    pvr_set_native_cube(i32::from(cube));
    pvr_set_native_array(i32::from(array));
    pvr_set_native_3d(i32::from(d3));
    f()
}

/* ---- the formats ----------------------------------------------------------- */

/// One sized integer colour format: its GL name, transfer format and type,
/// whether it is signed, and the four-channel class the PowerVR keeps it in.
#[derive(Clone, Copy, Debug)]
struct Fmt {
    name: &'static str,
    internal: GLenum,
    format: GLenum,
    type_: GLenum,
    signed: bool,
    store: u8,
}

const fn f(name: &'static str, internal: GLenum, format: GLenum, type_: GLenum, signed: bool, store: u8) -> Fmt {
    Fmt { name, internal, format, type_, signed, store }
}

/// The twelve R/RG integer formats of the AM62P's fbo fails.
const RG_INT: [Fmt; 12] = [
    f("r8i", GL_R8I, GL_RED_INTEGER, GL_BYTE, true, GLES_TEXFMT_RGBA8I),
    f("r8ui", GL_R8UI, GL_RED_INTEGER, GL_UNSIGNED_BYTE, false, GLES_TEXFMT_RGBA8UI),
    f("r16i", GL_R16I, GL_RED_INTEGER, GL_SHORT, true, GLES_TEXFMT_RGBA16I),
    f("r16ui", GL_R16UI, GL_RED_INTEGER, GL_UNSIGNED_SHORT, false, GLES_TEXFMT_RGBA16UI),
    f("r32i", GL_R32I, GL_RED_INTEGER, GL_INT, true, GLES_TEXFMT_RGBA32I),
    f("r32ui", GL_R32UI, GL_RED_INTEGER, GL_UNSIGNED_INT, false, GLES_TEXFMT_RGBA32UI),
    f("rg8i", GL_RG8I, GL_RG_INTEGER, GL_BYTE, true, GLES_TEXFMT_RGBA8I),
    f("rg8ui", GL_RG8UI, GL_RG_INTEGER, GL_UNSIGNED_BYTE, false, GLES_TEXFMT_RGBA8UI),
    f("rg16i", GL_RG16I, GL_RG_INTEGER, GL_SHORT, true, GLES_TEXFMT_RGBA16I),
    f("rg16ui", GL_RG16UI, GL_RG_INTEGER, GL_UNSIGNED_SHORT, false, GLES_TEXFMT_RGBA16UI),
    f("rg32i", GL_RG32I, GL_RG_INTEGER, GL_INT, true, GLES_TEXFMT_RGBA32I),
    f("rg32ui", GL_RG32UI, GL_RG_INTEGER, GL_UNSIGNED_INT, false, GLES_TEXFMT_RGBA32UI),
];

/// The RGBA integer formats, which pass on the board: the contrast.
const RGBA_INT: [Fmt; 6] = [
    f("rgba8i", GL_RGBA8I, GL_RGBA_INTEGER, GL_BYTE, true, GLES_TEXFMT_RGBA8I),
    f("rgba8ui", GL_RGBA8UI, GL_RGBA_INTEGER, GL_UNSIGNED_BYTE, false, GLES_TEXFMT_RGBA8UI),
    f("rgba16i", GL_RGBA16I, GL_RGBA_INTEGER, GL_SHORT, true, GLES_TEXFMT_RGBA16I),
    f("rgba16ui", GL_RGBA16UI, GL_RGBA_INTEGER, GL_UNSIGNED_SHORT, false, GLES_TEXFMT_RGBA16UI),
    f("rgba32i", GL_RGBA32I, GL_RGBA_INTEGER, GL_INT, true, GLES_TEXFMT_RGBA32I),
    f("rgba32ui", GL_RGBA32UI, GL_RGBA_INTEGER, GL_UNSIGNED_INT, false, GLES_TEXFMT_RGBA32UI),
];

/* ---- the GL side -------------------------------------------------------- */

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut log = [0 as c_char; 1024];
        glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        assert_eq!(ok, 1, "compile: {}\n{text}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        s
    }
}

fn program(fs: &str) -> GLuint {
    const VS: &str = "#version 300 es\nin vec4 a_pos;\nvoid main()\n{\n    gl_Position = a_pos;\n}\n";
    let vs = shader(GL_VERTEX_SHADER, VS);
    let fs = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls on handles this test owns.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glLinkProgram(p);
        glDeleteShader(vs);
        glDeleteShader(fs);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0 as c_char; 2048];
        glGetProgramInfoLog(p, 2048, core::ptr::null_mut(), log.as_mut_ptr());
        assert_eq!(ok, 1, "link: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        p
    }
}

/// dEQP's texture-to-FBO shape: an integer output written from a uniform,
/// 0 in the channels the format lacks (as dEQP's output scale makes them).
fn write_program(signed: bool) -> GLuint {
    let t = if signed { "ivec4" } else { "uvec4" };
    program(&format!(
        "#version 300 es\nprecision highp float;\nuniform highp {t} c;\n\
         layout(location=0) out highp {t} o;\nvoid main()\n{{\n    o = c;\n}}\n"
    ))
}

/// The FBO-to-window shape: the integer texture sampled by `sampler`
/// (`usampler2D`, `isampler2DArray`, ...) at coordinate `coord`, converted
/// to float.
fn sample_program(signed: bool, sampler: &str, coord: &str) -> GLuint {
    let s = if signed { "i" } else { "u" };
    program(&format!(
        "#version 300 es\nprecision highp float;\nuniform highp {s}{sampler} s;\n\
         layout(location=0) out vec4 o;\nvoid main()\n{{\n    o = vec4(texture(s, {coord})) / 255.0;\n}}\n"
    ))
}

/// One triangle with `p`: the binds the back end received for it.
fn draw(p: GLuint) -> Vec<Bound> {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    let before = LAST.lock().unwrap_or_else(|e| e.into_inner()).1;
    // SAFETY: plain GL calls on a linked program of this context; every
    // pointer stays alive through the draw.
    unsafe {
        glUseProgram(p);
        let s = glGetUniformLocation(p, c"s".as_ptr());
        if s >= 0 {
            glUniform1i(s, 0);
        }
        let c = glGetUniformLocation(p, c"c".as_ptr());
        if c >= 0 {
            /* the dEQP shape: red set, the rest 0 (ivec4 and uvec4 alike) */
            glUniform4ui(c, 7, 0, 0, 0);
            glUniform4i(c, 7, 0, 0, 0);
            while glGetError() != GL_NO_ERROR {} /* the call of the other type */
        }
        let loc = glGetAttribLocation(p, c"a_pos".as_ptr());
        assert!(loc >= 0, "a_pos is active");
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(loc as u32);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw is accepted");
        glDisableVertexAttribArray(loc as u32);
        glUseProgram(0);
    }
    let last = LAST.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(last.1, before + 1, "the draw reached the back end");
    last.0.clone()
}

/// A `target` texture of format `f`, `w` x `h` (x `d` for an array or a
/// volume, six faces for a cube), level 0 only, its store left as
/// glTexImage*(..., NULL) leaves it, NEAREST, bound to unit 0.
fn texture(target: GLenum, fm: &Fmt, w: i32, h: i32, d: i32) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name; no pixels are read (NULL).
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut name);
        glBindTexture(target, name);
        let (i, fo, ty) = (fm.internal as GLint, fm.format, fm.type_);
        let null = core::ptr::null();
        match target {
            GL_TEXTURE_2D => glTexImage2D(target, 0, i, w, h, 0, fo, ty, null),
            GL_TEXTURE_CUBE_MAP => {
                for face in 0..6 {
                    glTexImage2D(GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, 0, i, w, w, 0, fo, ty, null);
                }
            }
            _ => glTexImage3D(target, 0, i, w, h, d, 0, fo, ty, null),
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR, "{}: the texture is made", fm.name);
    }
    name
}

/// Attach level 0 (layer or face `layer`) of `tex` to a new framebuffer,
/// bound, and check it is complete.
fn framebuffer(target: GLenum, tex: GLuint, layer: i32, fm: &Fmt) -> GLuint {
    let mut fbo = 0;
    // SAFETY: plain GL calls on names this test owns.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        match target {
            GL_TEXTURE_2D => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, target, tex, 0),
            GL_TEXTURE_CUBE_MAP => glFramebufferTexture2D(
                GL_FRAMEBUFFER,
                GL_COLOR_ATTACHMENT0,
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + layer as GLenum,
                tex,
                0,
            ),
            _ => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, tex, 0, layer),
        }
        assert_eq!(
            glCheckFramebufferStatus(GL_FRAMEBUFFER),
            GL_FRAMEBUFFER_COMPLETE,
            "{}: a colour attachment (ES 3.0 table 3.13: colour-renderable)",
            fm.name
        );
    }
    fbo
}

fn done(target: GLenum, tex: GLuint, fbo: GLuint, progs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        if fbo != 0 {
            glDeleteFramebuffers(1, &fbo);
        }
        glBindTexture(target, 0);
        glDeleteTextures(1, &tex);
        for &p in progs {
            glDeleteProgram(p);
        }
        while glGetError() != GL_NO_ERROR {}
    }
}

/* ---- the words ----------------------------------------------------------- */

/// texstate.xml's SWIZ values: SRC_CHAN0..3 are 0..3, SRC_ONE 4, SRC_ZERO 5.
const CHAN0: u64 = 0;
const CHAN1: u64 = 1;
const CHAN2: u64 = 2;
const CHAN3: u64 = 3;
const ONE: u64 = 4;

/// TEXSTATE_IMAGE_WORD0's swiz0..3 (bits 14..16, 11..13, 8..10, 5..7): what
/// R, G, B and A of the look-up read.
fn swizzle(w: &[u32]) -> [u64; 4] {
    let w0 = w[0] as u64 | (w[1] as u64) << 32;
    [(w0 >> 14) & 7, (w0 >> 11) & 7, (w0 >> 8) & 7, (w0 >> 5) & 7]
}

/// The one texture of a draw's binds.
fn the_one(binds: &[Bound], fm: &Fmt) -> Bound {
    assert_eq!(binds.len(), 1, "{}: the sampling draw binds one texture", fm.name);
    binds[0].clone()
}

/* ---- the tests ----------------------------------------------------------- */

/// The dEQP fbo.color.tex2d / render.resize / invalidate.format shape: an
/// R/RG integer texture made with NULL, drawn into through a framebuffer,
/// then sampled to the window. The sampling bind is the four-channel class
/// with `no_alpha`, and its descriptor reads alpha as ONE, red (and green)
/// from the texel.
#[test]
fn an_r_or_rg_integer_texture_drawn_into_samples_alpha_one() {
    let _g = context();
    for fm in &RG_INT {
        let tex = texture(GL_TEXTURE_2D, fm, 33, 17, 1);
        let fbo = framebuffer(GL_TEXTURE_2D, tex, 0, fm);
        let wp = write_program(fm.signed);
        let into = draw(wp);
        assert!(into.is_empty(), "{}: the draw into the texture samples nothing", fm.name);

        // SAFETY: plain GL calls.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        let sp = sample_program(fm.signed, "sampler2D", "vec2(0.5)");
        let (b, w) = the_one(&draw(sp), fm);
        assert_eq!(
            b,
            Bind { format: fm.store, type_: backend::GLES_TEXTYPE_2D, no_alpha: 1 },
            "{}: kept in its four-channel class, without alpha",
            fm.name
        );
        assert_eq!(swizzle(&w), [CHAN0, CHAN1, CHAN2, ONE], "{}: (r, g, b) from the texel, alpha 1", fm.name);
        done(GL_TEXTURE_2D, tex, fbo, &[wp, sp]);
    }
}

/// The RGBA integer formats, which pass on the board, keep alpha from the
/// texel: the change is the formats without alpha only.
#[test]
fn an_rgba_integer_texture_samples_its_own_alpha() {
    let _g = context();
    for fm in &RGBA_INT {
        let tex = texture(GL_TEXTURE_2D, fm, 33, 17, 1);
        let fbo = framebuffer(GL_TEXTURE_2D, tex, 0, fm);
        let wp = write_program(fm.signed);
        draw(wp);
        // SAFETY: plain GL calls.
        unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
        let sp = sample_program(fm.signed, "sampler2D", "vec2(0.5)");
        let (b, w) = the_one(&draw(sp), fm);
        assert_eq!((b.format, b.no_alpha), (fm.store, 0), "{}", fm.name);
        assert_eq!(swizzle(&w), [CHAN0, CHAN1, CHAN2, CHAN3], "{}: the identity", fm.name);
        done(GL_TEXTURE_2D, tex, fbo, &[wp, sp]);
    }
}

/// fbo.color.tex2darray, tex3d and texcube draw into one layer, slice or
/// face and sample the whole texture: alpha reads ONE in the 2D array, 3D
/// and cube arms of the descriptor too, with the native switches on (the
/// TPU's own array, volume and cube) and off (stacked images, the grid of
/// slices).
#[test]
fn the_array_volume_and_cube_arms_read_alpha_one_too() {
    let _g = context();
    let kinds: [(GLenum, &str, &str); 3] = [
        (GL_TEXTURE_2D_ARRAY, "sampler2DArray", "vec3(0.5, 0.5, 1.0)"),
        (GL_TEXTURE_3D, "sampler3D", "vec3(0.5, 0.5, 0.375)"),
        (GL_TEXTURE_CUBE_MAP, "samplerCube", "vec3(1.0, 0.25, 0.5)"),
    ];
    for native in [false, true] {
        with_switches(native, native, native, || {
            for (k, (target, sampler, coord)) in kinds.into_iter().enumerate() {
                for fm in [&RG_INT[1], &RG_INT[8], &RG_INT[5], &RG_INT[11], &RGBA_INT[1]] {
                    let tex = texture(target, fm, 16, 16, 4);
                    let fbo = framebuffer(target, tex, 1, fm);
                    let wp = write_program(fm.signed);
                    draw(wp);
                    // SAFETY: plain GL calls.
                    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
                    let sp = sample_program(fm.signed, sampler, coord);
                    let (b, w) = the_one(&draw(sp), fm);
                    let alpha = if b.no_alpha != 0 { ONE } else { CHAN3 };
                    assert_eq!(b.format, fm.store, "{} {sampler} native {native}", fm.name);
                    /* the arm the descriptor took */
                    let arm = if native {
                        [GLES_TEXTYPE_2D_ARRAY_TWIDDLED, GLES_TEXTYPE_3D_TWIDDLED, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE]
                    } else {
                        [GLES_TEXTYPE_2D_ARRAY, GLES_TEXTYPE_3D, GLES_TEXTYPE_CUBE]
                    };
                    assert_eq!(b.type_, arm[k], "{} {sampler} native {native}: the arm", fm.name);
                    assert_eq!(swizzle(&w), [CHAN0, CHAN1, CHAN2, alpha], "{} {sampler} native {native}", fm.name);
                    if fm.format != GL_RGBA_INTEGER {
                        assert_eq!(alpha, ONE, "{} {sampler}: the GL core says it has no alpha", fm.name);
                    }
                    done(target, tex, fbo, &[wp, sp]);
                }
            }
        });
    }
}

/// GL_TEXTURE_SWIZZLE_* composes with the format's alpha: an R8UI texture
/// with SWIZZLE_R = GL_ALPHA reads ONE in red (ES 3.0.6 3.8.14.2: the
/// swizzle picks the base format's components, and R8UI's alpha is 1), and
/// SWIZZLE_A = GL_RED the texel's red in alpha.
#[test]
fn the_texture_swizzle_reads_the_format_alpha_of_one() {
    let _g = context();
    let fm = &RG_INT[1];
    let tex = texture(GL_TEXTURE_2D, fm, 8, 8, 1);
    // SAFETY: plain GL calls on the bound texture.
    unsafe {
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_R, GL_ALPHA as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_SWIZZLE_A, GL_RED as GLint);
    }
    let sp = sample_program(false, "sampler2D", "vec2(0.5)");
    let (_, w) = the_one(&draw(sp), fm);
    assert_eq!(swizzle(&w), [ONE, CHAN1, CHAN2, CHAN0], "red reads alpha (1), alpha reads red");
    done(GL_TEXTURE_2D, tex, 0, &[sp]);
}
