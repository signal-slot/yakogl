// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 681b728b (gl: GL_RGBA32F in the 128-bit class
//! on the GC7000UL) and 3f11747d (vivante: the TS probe and depth-dump hooks
//! of gl_test cases 246/247). The i.MX8MP whole dEQP-GLES2 tree (0fbe61a3)
//! predates both and its stage-1 promise is 0 Fail. dEQP-GLES2
//! texture.format.* and texture.specification.* upload a float texture only
//! when GL_OES_texture_float is listed.
//!
//! In a `report-es2` context on the real GC7000 back end over a stand-in OSAL:
//!
//! * GL_OES_texture_half_float is listed and GL_OES_texture_float is not
//!   (the list names the precision the GL has: state.rs). So a GL_FLOAT
//!   upload (RGBA, RGB, LUMINANCE, ALPHA; 2D and cube; POT and NPOT)
//!   answers GL_INVALID_ENUM (ES 2.0 2.5: a type that is not allowed), and
//!   so does a sized GL_RGBA32F internal format (no 128-bit class from ES 2.0);
//! * the type the list does give, GL_HALF_FLOAT_OES, is accepted for the same
//!   formats and shapes, converts them as the core does (unorm8, or the 16F
//!   class; RGB gets alpha 1, LUMINANCE r=g=b, ALPHA r=g=b=0) and samples
//!   them in a draw without a refusal;
//! * RGBA/UNSIGNED_BYTE, RGB565, RGBA4, RGB5_A1, LUMINANCE, ALPHA and
//!   LUMINANCE_ALPHA uploads store their usual classes and bytes and sample;
//! * a draw with depth and colour into an FBO, and into the default
//!   framebuffer, writes no tile-status register, makes one kick, and the
//!   probe's and depth dump's hooks (behind `test-knobs`, which this build
//!   does not have) leave no trace in the stream.
//!
//! The gate runs it in its `host-test,gpu-vivante,shaderc-online,report-es2`
//! row (the i.MX8MP dEQP-GLES2 image).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_texstore
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{
    gles_texfmt_bytes, GLES_TEXFMT_A8, GLES_TEXFMT_L8, GLES_TEXFMT_LA8, GLES_TEXFMT_RGBA16F, GLES_TEXFMT_RGBA8,
};
use yakogl::gl::buffer::{glBindBuffer, glBufferData, glGenBuffers};
use yakogl::gl::consts::*;
use yakogl::gl::context::GlTexture;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glClear, glDepthFunc, glEnable, glFinish, glGetError, glGetString, glPixelStorei, glViewport};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_refused_draws, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

struct Alloc {
    mem: Box<[u64]>,
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
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are
/// process-global.
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
        p.allocs.insert(gpu, Alloc { mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    with_platform(|p| {
        /* leaked, not freed: a static of the library that still names the
         * block must find memory (tests/gc_npot_repeat.rs) */
        match p.allocs.remove(&m.gpu) {
            Some(a) => core::mem::forget(a.mem),
            None if !m.impl_.is_null() => {
                p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu))
            }
            None => {}
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The stream is kept and its fence raised; every kick completes at once.
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
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        let words = words.to_vec();
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

/* ---- GL plumbing ------------------------------------------------------------------- */

fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_platform(|p| {
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        *p = Platform::default();
    });
    // SAFETY: SERIAL is held; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    let faults = with_platform(|p| core::mem::take(&mut p.faults));
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn error() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn no_error(what: &str) {
    let e = error();
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

const VS: &str = "attribute vec4 p;\nvarying vec2 t;\nvoid main() { t = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS: &str =
    "precision mediump float;\nvarying vec2 t;\nuniform sampler2D s;\nvoid main() { gl_FragColor = texture2D(s, t); }\n";
const FS_CUBE: &str = "precision mediump float;\nvarying vec2 t;\nuniform samplerCube s;\nvoid main() { gl_FragColor = textureCube(s, vec3(t, 1.0)); }\n";
const FS_FLAT: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0, 0.5, 0.25, 0.5); }\n";

/// One triangle's positions.
static TRI: [f32; 12] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0];

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

/// A program with `fs`, and a buffer of [`TRI`] at attribute 0, bound and enabled.
fn setup(fs: &str) {
    // SAFETY: plain GL calls; TRI is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glViewport(0, 0, 64, 64);
        let mut vbo: GLuint = 0;
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(
            GL_ARRAY_BUFFER,
            core::mem::size_of_val(&TRI) as GLsizeiptr,
            TRI.as_ptr() as *const c_void,
            GL_STATIC_DRAW,
        );
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
    }
    no_error("the setup");
}

/* ---- the extension list ------------------------------------------------------------ */

fn extensions() -> Vec<String> {
    // SAFETY: a plain GL call; the string is a static NUL-terminated one.
    let s = unsafe { core::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) };
    s.to_str().unwrap().split_ascii_whitespace().map(str::to_string).collect()
}

#[test]
fn half_float_is_listed_and_single_float_is_not() {
    with_backend(|| {
        let e = extensions();
        assert!(e.iter().any(|x| x == "GL_OES_texture_half_float"), "{e:?}");
        assert!(!e.iter().any(|x| x == "GL_OES_texture_float"), "single precision is not claimed: {e:?}");
        assert!(!e.iter().any(|x| x == "GL_OES_texture_float_linear"), "{e:?}");
        assert!(!e.iter().any(|x| x == "GL_EXT_color_buffer_float"), "{e:?}");
    });
}

/* ---- uploads ----------------------------------------------------------------------- */

/// The shapes: POT and NPOT 2D, POT and NPOT (square) cube faces.
const SHAPES: [(bool, u32, u32); 6] =
    [(false, 4, 4), (false, 8, 2), (false, 5, 3), (false, 7, 7), (true, 4, 4), (true, 5, 5)];

fn target_of(cube: bool) -> GLenum {
    if cube {
        GL_TEXTURE_CUBE_MAP
    } else {
        GL_TEXTURE_2D
    }
}

fn face_target(cube: bool, face: u32) -> GLenum {
    if cube {
        GL_TEXTURE_CUBE_MAP_POSITIVE_X + face
    } else {
        GL_TEXTURE_2D
    }
}

fn texture(target: GLenum) -> &'static GlTexture {
    let c = yakogl::gl::context::current().expect("a current context");
    c.objs_ref().textures.iter().find(|t| t.used && t.target == target).expect("the texture")
}

fn level0(t: &GlTexture, face: u32) -> Vec<u8> {
    let (w, h) = (t.width as usize, t.height as usize);
    let bpp = gles_texfmt_bytes(t.texfmt) as usize;
    let pitch = t.level_pitch_at(0) as usize * bpp;
    // SAFETY: the texture's live storage, mem.size bytes.
    let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
    let base = t.level_off[0] as usize + face as usize * h * pitch;
    (0..h).flat_map(|y| mem[base + y * pitch..base + y * pitch + w * bpp].iter().copied()).collect()
}

fn le16(v: &[u16]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// `n` finite halves, each its own, in [0.125, 1.0).
fn halves(n: usize, seed: u16) -> Vec<u16> {
    (0..n).map(|i| 0x3000 + (seed.wrapping_mul(7).wrapping_add(i as u16 * 3) % 0x0c00)).collect()
}

/// A normal half as an f32.
fn half_to_f32(v: u16) -> f32 {
    let (e, m) = ((v >> 10) & 0x1f, (v & 0x3ff) as f32);
    (1.0 + m / 1024.0) * 2f32.powi(e as i32 - 15)
}

/// A texture of the shape with `data` for every face, NEAREST and CLAMP
/// (NPOT-legal in ES 2.0); returns the first error of the uploads.
fn upload(cube: bool, w: u32, h: u32, format: GLenum, ty: GLenum, internal: GLint, data: &[u8]) -> GLenum {
    let target = target_of(cube);
    let mut tex: GLuint = 0;
    let mut err = GL_NO_ERROR;
    // SAFETY: plain GL calls; `data` holds a face's w x h texels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(target, tex);
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(target, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        for face in 0..if cube { 6 } else { 1 } {
            glTexImage2D(
                face_target(cube, face),
                0,
                internal,
                w as GLsizei,
                h as GLsizei,
                0,
                format,
                ty,
                data.as_ptr() as *const c_void,
            );
            let e = glGetError();
            if err == GL_NO_ERROR {
                err = e;
            }
        }
    }
    err
}

/// Draw with the bound texture; the back end must not refuse it.
fn sampled_draw(cube: bool, what: &str) {
    setup(if cube { FS_CUBE } else { FS });
    let (kicks0, refused0) = (with_platform(|p| p.kicks.len()), gc_refused_draws());
    // SAFETY: plain GL calls; setup() bound the program and the array.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    no_error(&format!("{what}: the draw"));
    assert_eq!(gc_refused_draws(), refused0, "{what}: the draw was refused");
    assert!(with_platform(|p| p.kicks.len()) > kicks0, "{what}: the draw made no kick");
}

const FORMATS: [(GLenum, &str, usize); 4] =
    [(GL_RGBA, "RGBA", 4), (GL_RGB, "RGB", 3), (GL_LUMINANCE, "LUMINANCE", 1), (GL_ALPHA, "ALPHA", 1)];

#[test]
fn gl_float_uploads_are_invalid_enum() {
    for (format, name, comps) in FORMATS {
        for (cube, w, h) in SHAPES {
            let what = format!("{name}/FLOAT {}{w}x{h}", if cube { "cube " } else { "" });
            with_backend(|| {
                let data = vec![0u8; (w * h) as usize * comps * 4];
                let e = upload(cube, w, h, format, GL_FLOAT, format as GLint, &data);
                assert_eq!(e, GL_INVALID_ENUM, "{what}");
                /* nothing was defined: sampling the incomplete texture is no fault */
                setup(if cube { FS_CUBE } else { FS });
                // SAFETY: plain GL calls.
                unsafe {
                    glDrawArrays(GL_TRIANGLES, 0, 3);
                    glFinish();
                }
                no_error(&format!("{what}: a draw with the undefined texture"));
            });
        }
    }
}

#[test]
fn sized_rgba32f_is_refused() {
    /* ES 2.0 3.7.1: internalformat is one of the unsized formats and equals
     * `format`; GL_RGBA32F is ES 3.0's, and the 128-bit class of 681b728b is
     * not reachable from an ES 2.0 context. */
    for ty in [GL_FLOAT, GL_UNSIGNED_BYTE] {
        with_backend(|| {
            let data = vec![0u8; 4 * 4 * 16];
            let e = upload(false, 4, 4, GL_RGBA, ty, GL_RGBA32F as GLint, &data);
            assert!(
                e == GL_INVALID_ENUM || e == GL_INVALID_VALUE || e == GL_INVALID_OPERATION,
                "type 0x{ty:x}: 0x{e:x}"
            );
        });
    }
}

#[test]
fn half_float_uploads_are_accepted_converted_and_sampled() {
    /* The GL core keeps an ES 2.0 half-float texture in its unorm8 store
     * (src/gl/texture.rs `half_float_oes_is_unpacked_like_half_float`): a
     * texel is the half clamped to [0, 1] and rounded to a byte. The halves
     * here are all in [0.125, 1), so the rounding is the whole story. */
    for (format, name, comps) in FORMATS {
        for (cube, w, h) in SHAPES {
            let what = format!("{name}/HALF_FLOAT_OES {}{w}x{h}", if cube { "cube " } else { "" });
            with_backend(|| {
                let src = halves((w * h) as usize * comps, (w * 31 + h) as u16);
                let e = upload(cube, w, h, format, GL_HALF_FLOAT_OES, format as GLint, &le16(&src));
                assert_eq!(e, GL_NO_ERROR, "{what}: the upload");
                let t = texture(target_of(cube));
                assert!(
                    matches!(t.texfmt, GLES_TEXFMT_RGBA8 | GLES_TEXFMT_RGBA16F | GLES_TEXFMT_L8 | GLES_TEXFMT_A8),
                    "{what}: class {}",
                    t.texfmt
                );
                let q = |v: u16| (half_to_f32(v).clamp(0.0, 1.0) * 255.0).round() as i32;
                let mut want = Vec::new();
                for i in 0..(w * h) as usize {
                    let s = &src[i * comps..(i + 1) * comps];
                    /* the narrow classes keep the one channel they have */
                    match (format, t.texfmt) {
                        (GL_LUMINANCE, GLES_TEXFMT_L8) | (GL_ALPHA, GLES_TEXFMT_A8) => want.push(q(s[0])),
                        (GL_RGBA, _) => want.extend([q(s[0]), q(s[1]), q(s[2]), q(s[3])]),
                        (GL_RGB, _) => want.extend([q(s[0]), q(s[1]), q(s[2]), 255]),
                        (GL_LUMINANCE, _) => want.extend([q(s[0]), q(s[0]), q(s[0]), 255]),
                        _ => want.extend([0, 0, 0, q(s[0])]),
                    }
                }
                for face in 0..if cube { 6 } else { 1 } {
                    let got = level0(t, face);
                    if t.texfmt != GLES_TEXFMT_RGBA16F {
                        assert_eq!(got.len(), want.len(), "{what}: size");
                        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                            assert!((*g as i32 - w).abs() <= 1, "{what}: face {face} byte {i}: {g} vs {w}");
                        }
                    } else {
                        let g16: Vec<i32> = got.chunks(2).map(|c| q(u16::from_le_bytes([c[0], c[1]]))).collect();
                        assert_eq!(g16, want, "{what}: face {face} halves");
                    }
                }
                sampled_draw(cube, &what);
            });
        }
    }
}

/// A byte or packed upload, the class it is stored in and its first texel.
struct Narrow {
    name: &'static str,
    format: GLenum,
    ty: GLenum,
    data: &'static [u8],
    stored: u8,
    /// The texel's (r, g, b, a) in an RGBA8-class store; unused for the narrow ones.
    rgba: [u8; 4],
}

const NARROW: [Narrow; 8] = [
    Narrow {
        name: "RGBA/UNSIGNED_BYTE",
        format: GL_RGBA,
        ty: GL_UNSIGNED_BYTE,
        data: &[10, 20, 30, 40],
        stored: GLES_TEXFMT_RGBA8,
        rgba: [10, 20, 30, 40],
    },
    Narrow {
        name: "RGB/UNSIGNED_BYTE",
        format: GL_RGB,
        ty: GL_UNSIGNED_BYTE,
        data: &[10, 20, 30],
        stored: GLES_TEXFMT_RGBA8,
        rgba: [10, 20, 30, 255],
    },
    /* 0xF800: pure red */
    Narrow {
        name: "RGB/UNSIGNED_SHORT_5_6_5",
        format: GL_RGB,
        ty: GL_UNSIGNED_SHORT_5_6_5,
        data: &[0x00, 0xF8],
        stored: GLES_TEXFMT_RGBA8,
        rgba: [255, 0, 0, 255],
    },
    /* 0x1234: r=1 g=2 b=3 a=4 of 15 */
    Narrow {
        name: "RGBA/UNSIGNED_SHORT_4_4_4_4",
        format: GL_RGBA,
        ty: GL_UNSIGNED_SHORT_4_4_4_4,
        data: &[0x34, 0x12],
        stored: GLES_TEXFMT_RGBA8,
        rgba: [17, 34, 51, 68],
    },
    /* 0xF801: r=31 g=0 b=0 a=1 */
    Narrow {
        name: "RGBA/UNSIGNED_SHORT_5_5_5_1",
        format: GL_RGBA,
        ty: GL_UNSIGNED_SHORT_5_5_5_1,
        data: &[0x01, 0xF8],
        stored: GLES_TEXFMT_RGBA8,
        rgba: [255, 0, 0, 255],
    },
    Narrow {
        name: "LUMINANCE/UNSIGNED_BYTE",
        format: GL_LUMINANCE,
        ty: GL_UNSIGNED_BYTE,
        data: &[77],
        stored: GLES_TEXFMT_L8,
        rgba: [0; 4],
    },
    Narrow {
        name: "LUMINANCE_ALPHA/UNSIGNED_BYTE",
        format: GL_LUMINANCE_ALPHA,
        ty: GL_UNSIGNED_BYTE,
        data: &[77, 99],
        stored: GLES_TEXFMT_LA8,
        rgba: [0; 4],
    },
    Narrow {
        name: "ALPHA/UNSIGNED_BYTE",
        format: GL_ALPHA,
        ty: GL_UNSIGNED_BYTE,
        data: &[99],
        stored: GLES_TEXFMT_A8,
        rgba: [0; 4],
    },
];

#[test]
fn byte_and_packed_uploads_are_unchanged() {
    for n in &NARROW {
        for (cube, w, h) in [(false, 1, 1), (false, 5, 3), (true, 1, 1), (true, 5, 5)] {
            let what = format!("{} {}{w}x{h}", n.name, if cube { "cube " } else { "" });
            with_backend(|| {
                let bpt = n.data.len();
                let data: Vec<u8> = n.data.iter().copied().cycle().take((w * h) as usize * bpt).collect();
                let e = upload(cube, w, h, n.format, n.ty, n.format as GLint, &data);
                assert_eq!(e, GL_NO_ERROR, "{what}: the upload");
                let t = texture(target_of(cube));
                assert_eq!(t.texfmt, n.stored, "{what}: the stored class");
                let bytes = level0(t, 0);
                let bpp = gles_texfmt_bytes(t.texfmt) as usize;
                for texel in bytes.chunks(bpp) {
                    if n.stored == GLES_TEXFMT_RGBA8 {
                        let mut got = [texel[0], texel[1], texel[2], texel[3]];
                        if t.bgra {
                            got.swap(0, 2);
                        }
                        assert_eq!(got, n.rgba, "{what}: texel");
                    } else {
                        assert_eq!(texel, n.data, "{what}: texel");
                    }
                }
                sampled_draw(cube, &what);
            });
        }
    }
}

/* ---- the probe and the dump leave the stream alone --------------------------------- */

/// Every `(register byte address, value)` a stream's LOAD_STATEs write.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for k in 0..count {
                    out.push((base + 4 * k as u32, words[i + 1 + k]));
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => i += 4,
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    out
}

fn ts_registers() -> Vec<(String, u32)> {
    let mut v = vec![
        ("TS_COLOR_STATUS_BASE".to_string(), GC_TS_COLOR_STATUS_BASE),
        ("TS_COLOR_SURFACE_BASE".to_string(), GC_TS_COLOR_SURFACE_BASE),
        ("TS_COLOR_CLEAR_VALUE".to_string(), GC_TS_COLOR_CLEAR_VALUE),
        ("TS_COLOR_CLEAR_VALUE_EXT".to_string(), GC_TS_COLOR_CLEAR_VALUE_EXT),
        ("TS_COLOR_AUTO_DISABLE_COUNT".to_string(), GC_TS_COLOR_AUTO_DISABLE_COUNT),
        ("TS_DEPTH_STATUS_BASE".to_string(), GC_TS_DEPTH_STATUS_BASE),
        ("TS_DEPTH_SURFACE_BASE".to_string(), GC_TS_DEPTH_SURFACE_BASE),
        ("TS_DEPTH_CLEAR_VALUE".to_string(), GC_TS_DEPTH_CLEAR_VALUE),
        ("TS_DEPTH_AUTO_DISABLE_COUNT".to_string(), GC_TS_DEPTH_AUTO_DISABLE_COUNT),
    ];
    for i in 0..16 {
        v.push((format!("TS_SAMPLER_CONFIG({i})"), ts_sampler_config(i)));
        v.push((format!("TS_SAMPLER_STATUS_BASE({i})"), ts_sampler_status_base(i)));
    }
    v
}

/// Clear colour and depth, then draw with the depth test on; into an FBO (a
/// colour texture and a 16-bit depth renderbuffer) or the default
/// framebuffer. The kicks of the whole frame.
fn frame(fbo: bool) -> Vec<Vec<u32>> {
    with_backend(|| {
        setup(FS_FLAT);
        if fbo {
            // SAFETY: plain GL calls.
            unsafe {
                let (mut tex, mut rb, mut fb): (GLuint, GLuint, GLuint) = (0, 0, 0);
                glGenTextures(1, &mut tex);
                glBindTexture(GL_TEXTURE_2D, tex);
                glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                glTexImage2D(
                    GL_TEXTURE_2D,
                    0,
                    GL_RGBA as GLint,
                    64,
                    64,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    core::ptr::null(),
                );
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, 64, 64);
                glGenFramebuffers(1, &mut fb);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
                assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
            }
        }
        no_error("the target");
        with_platform(|p| p.kicks.clear());
        // SAFETY: plain GL calls; setup() bound the program and the array.
        unsafe {
            glEnable(GL_DEPTH_TEST);
            glDepthFunc(GL_LESS);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        no_error("the frame");
        with_platform(|p| core::mem::take(&mut p.kicks))
    })
}

#[test]
fn depth_and_colour_frames_carry_no_probe_or_dump() {
    let regs = ts_registers();
    for fbo in [false, true] {
        let what = if fbo { "FBO" } else { "default framebuffer" };
        let kicks = frame(fbo);
        assert_eq!(kicks.len(), 1, "{what}: one kick for a clear and a draw, no probe or dump submit");
        for (reg, val) in writes(&kicks[0]) {
            if let Some((n, _)) = regs.iter().find(|(_, r)| *r == reg) {
                panic!("{what}: {n} (0x{reg:x}) written with 0x{val:x}");
            }
            if reg == GC_PE_DEPTH_CONFIG {
                assert_eq!(val & GC_PE_DEPTH_EARLY_Z, 0, "{what}: PE early depth on");
            }
            if reg == GC_RA_EARLY_DEPTH {
                assert_eq!(val & GC_RA_EARLY_DEPTH_TEST_ENABLE, 0, "{what}: RA early depth on");
            }
        }
    }
}
