// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR's texture-state and sampler words for a 3D texture, from the
//! GL API to the descriptor (#1285, docs/notes/pvr-3d-mipmap-hang.md;
//! docs/notes/pvr-3d-native.md): a volume with five levels, GL_TEXTURE_WRAP_R
//! REPEAT and MIRRORED_REPEAT, and BASE_LEVEL, MAX_LEVEL, MIN_LOD and MAX_LOD
//! off their defaults, sampled by one triangle. The program is compiled by
//! the online PowerVR compiler from GLSL, the draw goes through the GL core
//! to a stand-in back end, and the stand-in packs the bind it was given with
//! the PowerVR's own descriptor builder (`pvr::backend::texture_descriptor`,
//! what `pvr_backend_draw` packs for the same bind).
//!
//! - The switch as it boots (`pvr_native_3d`, `PVR_NATIVE_3D_DEFAULT`): OFF.
//!   The draw binds the grid of slices (GLES_TEXTYPE_3D), one level; the
//!   descriptor is not a TEXTYPE_3D image, its third address mode is
//!   CLAMP_TO_EDGE whatever WRAP_R says, and BASE_LEVEL, MAX_LEVEL, MIN_LOD
//!   and MAX_LOD reach no word. This is the path #1285's note describes, and
//!   it is still what a build that does not set the switch runs.
//! - The switch on (`pvr_set_native_3d(1)`, a board's
//!   `CONFIG_DEQP_PVR_NATIVE_3D`, the host's `YAKOGL_PVR_NATIVE_3D=1`): the
//!   draw binds the twiddled copy (GLES_TEXTYPE_3D_TWIDDLED); IMAGE_WORD0's
//!   textype is 3D with the base level's size, IMAGE_WORD1 has its depth and
//!   the levels from BASE_LEVEL capped by MAX_LEVEL, and SAMPLER_WORD0 has
//!   WRAP_R in addrmode_w and MIN_LOD/MAX_LOD in minlod/maxlod (u4.6).
//!
//! Fields are texstate.xml's (Mesa 26.2.1): IMAGE_WORD0 textype [0:2], width
//! [34:47], height [48:61]; twiddled IMAGE_WORD1 num_mip_levels [0:3], depth
//! [4:14], mipmaps_present [15], baselevel [60:63]; SAMPLER_WORD0 minlod
//! [13:22], maxlod [23:32], mipfilter [40], addrmode_u [41:43], addrmode_v
//! [44:46], addrmode_w [56:58]. The host OSAL renders nothing: these are the
//! words, not pictures.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_native3d_sampler_words
//! ```
#![cfg(all(feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{
    self, GlesDraw, GlesHeap, GlesTextureBind, YakoglBackend, GLES_TEXTYPE_3D, GLES_TEXTYPE_3D_TWIDDLED,
    GLES_WRAP_CLAMP_TO_EDGE, GLES_WRAP_MIRRORED_REPEAT, GLES_WRAP_REPEAT,
};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::native_vol::twiddle_extent;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::pvr::backend::texture_descriptor;
use yakogl::shaderc::{pvr_set_native_3d, PVR_NATIVE_3D_DEFAULT};

/* ---- a GL context on the host: host memory, and a stand-in back end that
 * keeps each draw's fragment texture binds with the descriptor the PowerVR
 * packs for them ---- */

/// What the tests read of a fragment texture bind (the bind itself holds a
/// pointer, so it cannot wait in a static).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Bind {
    type_: u8,
    mip_levels: u8,
    mip_filter: u8,
    wrap_r: u8,
}

impl Bind {
    fn of(t: &GlesTextureBind) -> Bind {
        Bind { type_: t.type_, mip_levels: t.mip_levels, mip_filter: t.mip_filter, wrap_r: t.wrap_r }
    }
}

/// One fragment texture bind of a draw and its descriptor words.
type Bound = (Bind, Vec<u32>);

/// The binds of the last draw the back end received.
static LAST: Mutex<Vec<Bound>> = Mutex::new(Vec::new());

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
/// The descriptor is packed here, inside the draw, as the PowerVR packs it:
/// the native volume's descriptor reads the view its copy recorded for this
/// draw (`gl::native_vol::bound_view`).
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    assert!(!d.vs.is_null() && !d.fs.is_null(), "the draw carries both stages' records");
    let binds =
        d.textures[..d.num_textures as usize].iter().map(|t| (Bind::of(t), texture_descriptor(t).to_vec())).collect();
    *LAST.lock().unwrap_or_else(|e| e.into_inner()) = binds;
    0
}

/* The two tables are built from zero (every hook absent, every flag clear)
 * with the fields this file uses set, rather than written out in full: a
 * field another change adds to YakoglBackend or OsalOps then needs no edit
 * here (CLAUDE.md, "Two green pull requests, red main"). */
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
    b.name = c"pvr-native3d-sampler-words".as_ptr() as *const c_char;
    b.init = Some(b_init);
    b.draw = Some(b_draw);
    b.flush = Some(b_ok);
    b.finish = Some(b_ok);
    b.heap_base = Some(b_heap_base);
    b.max_instance_divisor = 1;
    b
};

/// One context for the whole file; the tests take turns on it. The switch
/// is process-wide, so the lock also serialises its settings.
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
        glUseProgram(0);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// The switch set for the closure and back at its default after it, panic
/// or not. The caller holds [`context`]'s lock.
fn with_native_3d<T>(on: bool, f: impl FnOnce() -> T) -> T {
    struct Back;
    impl Drop for Back {
        fn drop(&mut self) {
            pvr_set_native_3d(PVR_NATIVE_3D_DEFAULT);
        }
    }
    let _back = Back;
    pvr_set_native_3d(i32::from(on));
    f()
}

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
        assert_eq!(ok, 1, "compile: {}", CStr::from_ptr(log.as_ptr()).to_string_lossy());
        s
    }
}

/// A program that samples `sampler3D s` (unit 0) at a uniform coordinate in
/// its fragment stage, compiled with the switch as it is now: the switch is
/// read when a shader compiles.
fn volume_program() -> GLuint {
    const VS: &str = "#version 300 es\nin vec4 a_pos;\nvoid main()\n{\n    gl_Position = a_pos;\n}\n";
    const FS: &str = "#version 300 es\nprecision highp float;\nprecision highp sampler3D;\nuniform sampler3D s;\n\
                      uniform vec3 c;\nlayout(location=0) out vec4 o;\nvoid main()\n{\n    o = texture(s, c);\n}\n";
    let vs = shader(GL_VERTEX_SHADER, VS);
    let fs = shader(GL_FRAGMENT_SHADER, FS);
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

/// The volume of this file: 16x8x4 RGBA8, its whole chain (five levels,
/// 16x8x4 .. 1x1x1) stored and uploaded, LINEAR_MIPMAP_LINEAR, bound to
/// GL_TEXTURE_3D of unit 0. Every other parameter is GL's default.
const W: u32 = 16;
const H: u32 = 8;
const D: u32 = 4;
const CHAIN: u32 = 5;

fn volume() -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name; each level's pixels hold its texels.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_3D, name);
        glTexStorage3D(GL_TEXTURE_3D, CHAIN as GLsizei, GL_RGBA8, W as GLsizei, H as GLsizei, D as GLsizei);
        for l in 0..CHAIN {
            let (lw, lh, ld) = twiddle_extent(W, H, D, l);
            let px = vec![0x40u8 | l as u8; (lw * lh * ld * 4) as usize];
            glTexSubImage3D(
                GL_TEXTURE_3D,
                l as GLint,
                0,
                0,
                0,
                lw as GLsizei,
                lh as GLsizei,
                ld as GLsizei,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                px.as_ptr() as *const c_void,
            );
        }
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    name
}

fn parami(pname: GLenum, v: GLenum) {
    // SAFETY: plain GL call on the bound volume.
    unsafe { glTexParameteri(GL_TEXTURE_3D, pname, v as GLint) };
}

fn paramf(pname: GLenum, v: GLfloat) {
    // SAFETY: plain GL call on the bound volume.
    unsafe { glTexParameterf(GL_TEXTURE_3D, pname, v) };
}

/// One triangle with `p`, the bound volume on unit 0: the bind the back end
/// received for it and its descriptor.
fn draw(p: GLuint) -> Bound {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
    LAST.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // SAFETY: plain GL calls on a linked program of this context; every
    // pointer stays alive through the draw.
    unsafe {
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glUniform3f(glGetUniformLocation(p, c"c".as_ptr()), 0.25, 0.5, 1.75);
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
    let mut binds = LAST.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(binds.len(), 1, "the draw reached the back end with its one texture");
    binds.remove(0)
}

fn done(p: GLuint, tex: GLuint) {
    // SAFETY: names this test made.
    unsafe {
        glBindTexture(GL_TEXTURE_3D, 0);
        glDeleteTextures(1, &tex);
        glDeleteProgram(p);
        while glGetError() != GL_NO_ERROR {}
    }
}

/* ---- the words ----------------------------------------------------------- */

/// texstate.xml's TEXTYPE and ADDRMODE values.
const TEXTYPE_3D: u64 = 2;
const ADDRMODE_REPEAT: u64 = 0;
const ADDRMODE_FLIP: u64 = 1;
const ADDRMODE_CLAMP_TO_EDGE: u64 = 2;

fn word(w: &[u32], at: usize) -> u64 {
    w[at] as u64 | (w[at + 1] as u64) << 32
}

/// IMAGE_WORD0.textype.
fn textype(w: &[u32]) -> u64 {
    word(w, 0) & 7
}

/// A twiddled image's (width, height, depth), each stored minus one.
fn size(w: &[u32]) -> (u64, u64, u64) {
    let (iw0, iw1) = (word(w, 0), word(w, 2));
    (((iw0 >> 34) & 0x3fff) + 1, ((iw0 >> 48) & 0x3fff) + 1, ((iw1 >> 4) & 0x7ff) + 1)
}

/// A twiddled image's (num_mip_levels, mipmaps_present, baselevel).
fn levels(w: &[u32]) -> (u64, u64, u64) {
    let iw1 = word(w, 2);
    (iw1 & 0xf, (iw1 >> 15) & 1, iw1 >> 60)
}

/// SAMPLER_WORD0's (minlod, maxlod) in u4.6 and mipfilter.
fn lod(w: &[u32]) -> (u64, u64, u64) {
    let s = word(w, 8);
    ((s >> 13) & 0x3ff, (s >> 23) & 0x3ff, (s >> 40) & 1)
}

/// SAMPLER_WORD0's (addrmode_u, addrmode_v, addrmode_w).
fn addrmodes(w: &[u32]) -> (u64, u64, u64) {
    let s = word(w, 8);
    ((s >> 41) & 7, (s >> 44) & 7, (s >> 56) & 7)
}

/// The switch boots OFF, and a program compiled so samples the volume as
/// the grid of slices: one level of a non-3D image, CLAMP_TO_EDGE on the
/// third axis for every WRAP_R, and the same words whatever BASE_LEVEL,
/// MAX_LEVEL, MIN_LOD and MAX_LOD say. Native 3D is NOT the default at this
/// commit (#1285: a board image samples natively only when it sets the
/// switch, `CONFIG_DEQP_PVR_NATIVE_3D`). When the default is turned on this
/// test is the one to change.
#[test]
fn the_default_is_the_grid_of_slices_without_wrap_r_or_a_level_range() {
    let _g = context();
    assert_eq!(PVR_NATIVE_3D_DEFAULT, 0, "pvr_native_3d boots off");
    pvr_set_native_3d(PVR_NATIVE_3D_DEFAULT);
    let p = volume_program();
    let tex = volume();
    parami(GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
    parami(GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT);
    parami(GL_TEXTURE_WRAP_R, GL_REPEAT);
    let (b, plain) = draw(p);
    assert_eq!(b.type_, GLES_TEXTYPE_3D, "the grid of slices");
    assert_eq!((b.mip_levels, b.mip_filter), (1, 0), "level 0 alone, under LINEAR_MIPMAP_LINEAR");
    assert_eq!(b.wrap_r, GLES_WRAP_REPEAT, "the GL core hands WRAP_R over");
    assert_ne!(textype(&plain), TEXTYPE_3D, "not the TPU's 3D texture");
    assert_eq!(
        addrmodes(&plain),
        (ADDRMODE_CLAMP_TO_EDGE, ADDRMODE_FLIP, ADDRMODE_CLAMP_TO_EDGE),
        "S and T as set; REPEAT on R is not in the sampler"
    );
    assert_eq!(lod(&plain), (0, 0, 0), "one level: no LOD range, no mip filter");

    parami(GL_TEXTURE_WRAP_R, GL_MIRRORED_REPEAT);
    let (b, w) = draw(p);
    assert_eq!((b.type_, b.wrap_r), (GLES_TEXTYPE_3D, GLES_WRAP_MIRRORED_REPEAT));
    assert_eq!(w, plain, "MIRRORED_REPEAT on R: the same words");

    parami(GL_TEXTURE_BASE_LEVEL, 1);
    parami(GL_TEXTURE_MAX_LEVEL, 3);
    paramf(GL_TEXTURE_MIN_LOD, 0.5);
    paramf(GL_TEXTURE_MAX_LOD, 1.75);
    let (b, w) = draw(p);
    assert_eq!((b.type_, b.mip_levels, b.mip_filter), (GLES_TEXTYPE_3D, 1, 0), "still the grid, one level");
    assert_ne!(textype(&w), TEXTYPE_3D);
    assert_eq!(addrmodes(&w).2, ADDRMODE_CLAMP_TO_EDGE);
    assert_eq!(lod(&w), (0, 0, 0), "MIN_LOD and MAX_LOD reach no word");
    done(p, tex);
}

/// The switch on: the volume is the TPU's own 3D texture, and GL_TEXTURE_WRAP_R
/// is the sampler's third address mode -- REPEAT, MIRRORED_REPEAT (FLIP) and
/// CLAMP_TO_EDGE, apart from S and T.
#[test]
fn native_3d_puts_wrap_r_in_the_samplers_third_address_mode() {
    let _g = context();
    with_native_3d(true, || {
        let p = volume_program();
        let tex = volume();
        parami(GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
        parami(GL_TEXTURE_WRAP_T, GL_MIRRORED_REPEAT);
        for (wrap, bind, mode) in [
            (GL_REPEAT, GLES_WRAP_REPEAT, ADDRMODE_REPEAT),
            (GL_MIRRORED_REPEAT, GLES_WRAP_MIRRORED_REPEAT, ADDRMODE_FLIP),
            (GL_CLAMP_TO_EDGE, GLES_WRAP_CLAMP_TO_EDGE, ADDRMODE_CLAMP_TO_EDGE),
        ] {
            parami(GL_TEXTURE_WRAP_R, wrap);
            let (b, w) = draw(p);
            assert_eq!(b.type_, GLES_TEXTYPE_3D_TWIDDLED, "wrap {wrap:#x}: the twiddled copy, not the grid");
            assert_eq!(b.wrap_r, bind, "wrap {wrap:#x}: the bind's wrap_r");
            assert_eq!(textype(&w), TEXTYPE_3D, "wrap {wrap:#x}: IMAGE_WORD0.textype 3D");
            assert_eq!(
                addrmodes(&w),
                (ADDRMODE_CLAMP_TO_EDGE, ADDRMODE_FLIP, mode),
                "wrap {wrap:#x}: addrmode_u, addrmode_v, addrmode_w"
            );
        }
        /* the defaults: the whole volume and its whole chain */
        let (b, w) = draw(p);
        assert_eq!((b.mip_levels, b.mip_filter), (CHAIN as u8, 2), "five levels, LINEAR between them");
        assert_eq!(size(&w), (16, 8, 4), "a volume: width, height and depth, not a slice grid's");
        assert_eq!(levels(&w), (5, 1, 0));
        assert_eq!(lod(&w), (0, 4 * 64, 1), "LOD 0 to the last level");
        done(p, tex);
    });
}

/// The switch on: BASE_LEVEL, MAX_LEVEL, MIN_LOD and MAX_LOD off their
/// defaults. The image is the base level's size with the levels from it up
/// to MAX_LEVEL (baselevel 0: the copy starts at the base level), and the
/// sampler clamps the LOD to MIN_LOD and MAX_LOD inside those levels' range.
#[test]
fn native_3d_carries_the_level_range_and_the_lod_clamp() {
    let _g = context();
    with_native_3d(true, || {
        let p = volume_program();
        let tex = volume();
        parami(GL_TEXTURE_WRAP_R, GL_MIRRORED_REPEAT);
        parami(GL_TEXTURE_BASE_LEVEL, 1);
        parami(GL_TEXTURE_MAX_LEVEL, 3);
        paramf(GL_TEXTURE_MIN_LOD, 0.5);
        paramf(GL_TEXTURE_MAX_LOD, 1.75);
        let (b, w) = draw(p);
        assert_eq!(b.type_, GLES_TEXTYPE_3D_TWIDDLED);
        assert_eq!(textype(&w), TEXTYPE_3D);
        assert_eq!(size(&w), (8, 4, 2), "level 1's size");
        assert_eq!(levels(&w), (3, 1, 0), "levels 1 to 3, counted from the base level");
        assert_eq!(lod(&w), (32, 112, 1), "LOD 0.5 to 1.75 in u4.6");
        assert_eq!(addrmodes(&w).2, ADDRMODE_FLIP, "WRAP_R with the level range set");

        /* MAX_LOD past the levels MAX_LEVEL leaves: the levels' range */
        paramf(GL_TEXTURE_MAX_LOD, 1000.0);
        let (_, w) = draw(p);
        assert_eq!(lod(&w), (32, 2 * 64, 1), "LOD 0.5 to level 3, two past the base");

        /* MAX_LEVEL at the base level: one level, nothing to mix */
        parami(GL_TEXTURE_BASE_LEVEL, 2);
        parami(GL_TEXTURE_MAX_LEVEL, 2);
        paramf(GL_TEXTURE_MIN_LOD, -1000.0);
        let (_, w) = draw(p);
        assert_eq!(size(&w), (4, 2, 1), "level 2's size");
        assert_eq!(levels(&w), (1, 0, 0), "level 2 alone");
        assert_eq!(lod(&w), (0, 0, 0), "no LOD range over one level");
        assert_eq!(addrmodes(&w).2, ADDRMODE_FLIP);
        done(p, tex);
    });
}
