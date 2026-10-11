// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_OES_texture_npot: the 18 dEQP-GLES2 cases that depend on it, replayed
//! on the host (item es2-npot-cases; the case list and what the CTS source
//! checks are in docs/es2-cts-failures.md, section H.2 and H.3).
//!
//! Every test is named after its case (`texture_completeness_2d_npot_t_repeat`
//! is `dEQP-GLES2.functional.texture.completeness.2d.npot_t_repeat`) and makes
//! the GL calls of `es2fTextureCompletenessTests.cpp` /
//! `es2fNegativeTextureApiTests.cpp`, in the CTS's order and at its sizes:
//!
//! - `Incomplete2DSizeCase::createTexture`, `IncompleteCubeSizeCase::
//!   createTexture`: every `glTexImage2D` of the chain must be GL_NO_ERROR
//!   when GL_OES_texture_npot is listed (the CTS expects GL_INVALID_VALUE
//!   only without it);
//! - `Incomplete2DWrapModeCase`, `IncompleteCubeWrapModeCase`: one level,
//!   NEAREST, the wrap pair under test; the cube case binds its new texture to
//!   GL_TEXTURE_2D and sets parameters and faces on the GL_TEXTURE_CUBE_MAP
//!   binding, i.e. the default cube texture (name 0).
//!
//! Then one quad is drawn, as the CTS renderer does, and the draw the back end
//! receives is read: `GlesTextureBind::incomplete` is what the core decides,
//! and the pixel colour the CTS compares with follows from it (complete: the
//! solid blue texture, `(0,0,255,255)`; incomplete: the black the back end
//! samples instead, `(0,0,0,255)`). The test asserts that outcome as the CTS
//! expects it with the extension listed (doc section 2). The wrap and the
//! chain are asserted on the bind too, so a sampler that read the wrong mode
//! or level count would show.
//!
//! Each case runs on two back end shapes, because the core lays an NPOT chain
//! out differently (`npot_mip_chains`, src/gl/texture.rs): the PowerVR's
//! (level 0 handed to the TPU, 2D chain as its power-of-two extent) and the
//! native-layered one of the GC7000 and the V3D.
//!
//! What this cannot see: pixels. The stand-in back end draws nothing, so a
//! GPU's own NPOT REPEAT or mip addressing (doc section 3.1, "Risk" columns)
//! stays a board check.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_npot
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es2_npot
//! ```
//!
//! Needs the in-process compiler (the quad's two programs are not in the
//! offline table). The extension-listed assertion is made only in a
//! `report-es2` build; an ES 3.0 build has no NPOT rule to test either way.

#![cfg(all(feature = "host-test", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend, GLES_TEXTYPE_CUBE};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the stand-in back ends: they record the texture bind of each draw ------ */

/// The first texture bind of the last draw.
#[derive(Clone, Copy, Default)]
struct Seen {
    draws: u32,
    num_textures: u32,
    cube: bool,
    incomplete: u8,
    mip_filter: u8,
    mip_levels: u8,
    wrap_s: u8,
    wrap_t: u8,
    width: u32,
    height: u32,
}

static SEEN: Mutex<Seen> = Mutex::new(Seen {
    draws: 0,
    num_textures: 0,
    cube: false,
    incomplete: 0,
    mip_filter: 0,
    mip_levels: 0,
    wrap_s: 0,
    wrap_t: 0,
    width: 0,
    height: 0,
});

fn seen() -> Seen {
    *SEEN.lock().unwrap_or_else(|e| e.into_inner())
}

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
    let mut s = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    s.draws += 1;
    s.num_textures = u32::from(d.num_textures);
    if d.num_textures > 0 {
        let t = &d.textures[0];
        s.cube = t.type_ == GLES_TEXTYPE_CUBE || t.layers == 6;
        s.incomplete = t.incomplete;
        s.mip_filter = t.mip_filter;
        s.mip_levels = t.mip_levels;
        s.wrap_s = t.wrap_s;
        s.wrap_t = t.wrap_t;
        s.width = t.width;
        s.height = t.height;
    }
    0
}

const fn backend(name: &'static CStr, native_layered: bool) -> YakoglBackend {
    YakoglBackend {
        name: name.as_ptr() as *const c_char,
        native_attrib_formats: 0,
        rt_rgba8888_is_bgra: false,
        mip_level_align: 0,
        compressed_blocks_in_rows: false,
        native_layered_textures: native_layered,
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
    }
}

/// The PowerVR's shape: the TPU is handed level 0 and finds the rest itself.
static PVR_SHAPE: YakoglBackend = backend(c"npot stand-in (PowerVR shape)", false);
/// The GC7000's and the V3D's: the back end walks a layered texture itself.
static NATIVE_SHAPE: YakoglBackend = backend(c"npot stand-in (native layered)", true);

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

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    lock: None,
    unlock: None,
    thread_id: None,
};

/* ---- a fresh context per case and back end shape ------------------------------ */

static SERIAL: Mutex<()> = Mutex::new(());

/// Run `case` on a fresh context (so the default texture objects are
/// pristine, as at the start of a CTS case group) on each back end shape.
fn on_each_shape(case: impl Fn(&str, bool)) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    for (name, native, be) in [("powervr", false, &PVR_SHAPE), ("native-layered", true, &NATIVE_SHAPE)] {
        // SAFETY: the guard serialises the tests; the tables are statics.
        let ctx = unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = be;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            let c = yakogl::gl::context::gl_context_create(64, 64);
            assert!(!c.is_null(), "context pool exhausted");
            c
        };
        *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = Seen::default();
        let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| case(name, native)));
        // SAFETY: the context made above.
        unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
        if let Err(e) = r {
            std::panic::resume_unwind(e);
        }
    }
}

/* ---- GL plumbing ------------------------------------------------------------------ */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// Fail naming the call and the back end when the call raised an error.
fn no_error(shape: &str, what: &str) {
    let e = err();
    assert_eq!(e, GL_NO_ERROR, "[{shape}] {what}: 0x{e:x}, the CTS expects GL_NO_ERROR (expectError)");
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

const VS: &str = "attribute vec4 p;\nvarying vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS_2D: &str =
    "precision mediump float;\nuniform sampler2D s;\nvarying vec2 v;\nvoid main() { gl_FragColor = texture2D(s, v); }\n";
const FS_CUBE: &str = "precision mediump float;\nuniform samplerCube s;\nvarying vec2 v;\nvoid main() { gl_FragColor = textureCube(s, vec3(v * 2.0 - 1.0, 1.0)); }\n";

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// The CTS's TextureRenderer::renderQuad: the viewport, the sampling program
/// of the texture type, one quad. Returns what the back end was handed.
fn render_quad(shape: &str, cube: bool) -> Seen {
    let before = seen().draws;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = program(VS, if cube { FS_CUBE } else { FS_2D });
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error(shape, "renderQuad");
    let s = seen();
    assert_eq!(s.draws, before + 1, "[{shape}] the quad was drawn once");
    assert_eq!(s.num_textures, 1, "[{shape}] the program samples one texture");
    assert_eq!(s.cube, cube, "[{shape}] the sampler type that was bound");
    s
}

/// The blue (0,0,255,255) / black (0,0,0,255) the CTS compares with, as the
/// bind's `incomplete` flag.
const BLUE: u8 = 0;
const BLACK: u8 = 1;

fn expect_colour(shape: &str, s: &Seen, colour: u8, what: &str) {
    assert_eq!(
        s.incomplete,
        colour,
        "[{shape}] {what}: incomplete = {}, the CTS expects {} (incomplete is the black texture, complete the blue one)",
        s.incomplete,
        if colour == BLUE { "blue" } else { "black" }
    );
}

fn tex_param(target: GLenum, pname: GLenum, v: GLenum) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(target, pname, v as GLint) };
}

/// The RGBA8 texels the CTS clears its level data to: (0,0,1,1) for `size` x
/// `size`; the largest level is all a call needs.
fn blue(size: usize) -> Vec<u8> {
    [0u8, 0, 255, 255].repeat(size * size)
}

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

fn tex_image(target: GLenum, level: i32, w: i32, h: i32, texels: &[u8]) {
    assert!(texels.len() >= (w * h * 4) as usize);
    // SAFETY: `texels` holds the w x h image.
    unsafe {
        glTexImage2D(target, level, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, texels.as_ptr().cast());
    }
}

fn gen_bind_2d() {
    let mut t: GLuint = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
    }
}

/// The CTS keys every one of these cases on the extension string.
fn assert_extension_listed(shape: &str) {
    // SAFETY: glGetString returns a NUL-terminated static string.
    let ext = unsafe { CStr::from_ptr(glGetString(GL_EXTENSIONS).cast::<c_char>()) }.to_string_lossy().into_owned();
    let listed = ext.split(' ').any(|e| e == "GL_OES_texture_npot");
    /* an ES 3.0 build has NPOT without the ES 2.0 extension word */
    if yakogl::REPORT_ES2 {
        assert!(listed, "[{shape}] GL_OES_texture_npot is not in GL_EXTENSIONS: {ext}");
    }
}

/// GL's wrap enum as the back end contract's.
fn wrap_of(w: GLenum) -> u8 {
    match w {
        GL_REPEAT => yakogl::backend::GLES_WRAP_REPEAT,
        GL_MIRRORED_REPEAT => yakogl::backend::GLES_WRAP_MIRRORED_REPEAT,
        _ => yakogl::backend::GLES_WRAP_CLAMP_TO_EDGE,
    }
}

/* ---- Incomplete2DSizeCase::createTexture -------------------------------------------- */

/// `size` base, level `bad_level` of `bad_size` (the others `max(1, size >> l)`):
/// returns the bind of the quad.
fn size_2d(shape: &str, size: i32, bad_size: i32, bad_level: i32) -> Seen {
    assert_extension_listed(shape);
    gen_bind_2d();
    // SAFETY: plain GL call.
    unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    no_error(shape, "set texturing state");
    let texels = blue(size as usize);
    let levels = 1 + (31 - (size as u32).leading_zeros()) as i32;
    for l in 0..levels {
        let w = if l == bad_level { bad_size } else { (size >> l).max(1) };
        tex_image(GL_TEXTURE_2D, l, w, w, &texels);
        no_error(shape, &format!("glTexImage2D(level {l}, {w}x{w})"));
    }
    render_quad(shape, false)
}

/// 255x255, every level the right size: an NPOT chain that must be sampled
/// (blue). The CTS case that needs a complete NPOT mip chain.
#[test]
fn texture_completeness_2d_npot_size() {
    on_each_shape(|shape, native| {
        let s = size_2d(shape, 255, 255, 0);
        expect_colour(shape, &s, BLUE, "255x255 with levels 255..1");
        assert_eq!((s.width, s.height), (255, 255), "[{shape}] the base level that was bound");
        assert_eq!(s.mip_filter, 1, "[{shape}] NEAREST_MIPMAP_NEAREST samples the nearest level");
        assert_eq!((s.wrap_s, s.wrap_t), (wrap_of(GL_REPEAT), wrap_of(GL_REPEAT)), "[{shape}]");
        if native {
            assert_eq!(s.mip_levels, 8, "[{shape}] the whole chain 255 .. 1 is handed over");
        } else {
            assert!(s.mip_levels >= 1, "[{shape}] at least the base level");
        }
    });
}

/// 256x256, level 0 is 255x255 (levels 1.. are 128, 64 ..): level 1 does not
/// fit 255 >> 1 = 127, so the texture is incomplete: black; no GL error.
#[test]
fn texture_completeness_2d_npot_size_level_0() {
    on_each_shape(|shape, _| {
        let s = size_2d(shape, 256, 255, 0);
        expect_colour(shape, &s, BLACK, "level 0 of 255x255 over a 256 chain");
    });
}

/// 256x256, level 1 is 127x127 (the CTS expects no error with the
/// extension): incomplete, black.
#[test]
fn texture_completeness_2d_npot_size_level_1() {
    on_each_shape(|shape, _| {
        let s = size_2d(shape, 256, 127, 1);
        expect_colour(shape, &s, BLACK, "level 1 of 127x127 in a 256 chain");
    });
}

/* ---- Incomplete2DWrapModeCase::createTexture ---------------------------------------- */

/// 127x127, one level, NEAREST, the wrap pair: complete with the extension,
/// blue. Called with (wrap_t, wrap_s) as the CTS constructor takes them.
fn wrap_2d(shape: &str, wrap_t: GLenum, wrap_s: GLenum) {
    assert_extension_listed(shape);
    gen_bind_2d();
    // SAFETY: plain GL call.
    unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, wrap_s);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, wrap_t);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    tex_image(GL_TEXTURE_2D, 0, 127, 127, &blue(127));
    no_error(shape, "set texturing state");
    let s = render_quad(shape, false);
    expect_colour(shape, &s, BLUE, "127x127 with a REPEAT wrap and no mipmap");
    assert_eq!((s.width, s.height), (127, 127), "[{shape}]");
    assert_eq!(s.mip_filter, 0, "[{shape}] NEAREST: no mipmap");
    assert_eq!((s.wrap_s, s.wrap_t), (wrap_of(wrap_s), wrap_of(wrap_t)), "[{shape}] the wrap that was asked for");
}

#[test]
fn texture_completeness_2d_npot_t_repeat() {
    on_each_shape(|shape, _| wrap_2d(shape, GL_REPEAT, GL_CLAMP_TO_EDGE));
}

#[test]
fn texture_completeness_2d_npot_s_repeat() {
    on_each_shape(|shape, _| wrap_2d(shape, GL_CLAMP_TO_EDGE, GL_REPEAT));
}

#[test]
fn texture_completeness_2d_npot_all_repeat() {
    on_each_shape(|shape, _| wrap_2d(shape, GL_REPEAT, GL_REPEAT));
}

#[test]
fn texture_completeness_2d_npot_mirrored_repeat() {
    on_each_shape(|shape, _| wrap_2d(shape, GL_MIRRORED_REPEAT, GL_MIRRORED_REPEAT));
}

/// The CTS builds these as 127x128 and 128x127, REPEAT / REPEAT, NEAREST:
/// one NPOT side is enough to need the extension.
fn repeat_one_side(shape: &str, w: i32, h: i32) {
    assert_extension_listed(shape);
    gen_bind_2d();
    // SAFETY: plain GL call.
    unsafe { glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    tex_param(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    tex_image(GL_TEXTURE_2D, 0, w, h, &blue(128));
    no_error(shape, "set texturing state");
    let s = render_quad(shape, false);
    expect_colour(shape, &s, BLUE, "one NPOT side with REPEAT");
    assert_eq!((s.width, s.height), (w as u32, h as u32), "[{shape}]");
}

#[test]
fn texture_completeness_2d_repeat_width_npot() {
    on_each_shape(|shape, _| repeat_one_side(shape, 127, 128));
}

#[test]
fn texture_completeness_2d_repeat_height_npot() {
    on_each_shape(|shape, _| repeat_one_side(shape, 128, 127));
}

/* ---- IncompleteCubeSizeCase::createTexture ------------------------------------------ */

/// 64x64 faces, a face (`face`, or all six when None) of `bad_size` at
/// `bad_level`. Every glTexImage2D is GL_NO_ERROR with the extension.
fn size_cube(shape: &str, bad_size: i32, bad_level: i32, face: Option<usize>) -> Seen {
    assert_extension_listed(shape);
    let size = 64;
    let mut t: GLuint = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
    }
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, GL_REPEAT);
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, GL_REPEAT);
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST);
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    no_error(shape, "set texturing state");
    let texels = blue(size as usize);
    for l in 0..7 {
        for (f, target) in FACES.iter().enumerate() {
            let mut w = (size >> l).max(1);
            if l == bad_level && face.is_none_or(|b| b == f) {
                w = bad_size;
            }
            tex_image(*target, l, w, w, &texels);
            no_error(shape, &format!("glTexImage2D(face {f}, level {l}, {w}x{w})"));
        }
    }
    render_quad(shape, true)
}

/// Level 0 is 63x63 on all six faces, the rest 32, 16 ..: black.
#[test]
fn texture_completeness_cube_npot_size_level_0() {
    on_each_shape(|shape, _| {
        let s = size_cube(shape, 63, 0, None);
        expect_colour(shape, &s, BLACK, "level 0 of 63x63 over a 64 chain");
    });
}

/// Level 1 is 31x31 on all six faces: black.
#[test]
fn texture_completeness_cube_npot_size_level_1() {
    on_each_shape(|shape, _| {
        let s = size_cube(shape, 31, 1, None);
        expect_colour(shape, &s, BLACK, "level 1 of 31x31 in a 64 chain");
    });
}

/// Only +X has a 63x63 level 0: the faces disagree, black.
#[test]
fn texture_completeness_cube_npot_size_level_0_pos_x() {
    on_each_shape(|shape, _| {
        let s = size_cube(shape, 63, 0, Some(0));
        expect_colour(shape, &s, BLACK, "level 0 of +X is 63x63, the others 64x64");
    });
}

/// Only -X has a 31x31 level 1: black.
#[test]
fn texture_completeness_cube_npot_size_level_1_neg_x() {
    on_each_shape(|shape, _| {
        let s = size_cube(shape, 31, 1, Some(1));
        expect_colour(shape, &s, BLACK, "level 1 of -X is 31x31, the others 32x32");
    });
}

/* ---- IncompleteCubeWrapModeCase::createTexture -------------------------------------- */

/// The CTS binds its new texture to GL_TEXTURE_2D and then sets the
/// parameters and the six 127x127 faces on the GL_TEXTURE_CUBE_MAP binding:
/// the default cube texture (name 0). It must take them and be sampled, and
/// with the extension listed it is complete (blue).
fn wrap_cube(shape: &str, wrap_t: GLenum, wrap_s: GLenum) {
    assert_extension_listed(shape);
    let mut t: GLuint = 0;
    // SAFETY: one name; plain GL calls.
    unsafe {
        glGenTextures(1, &mut t);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glBindTexture(GL_TEXTURE_2D, t);
    }
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_S, wrap_s);
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_WRAP_T, wrap_t);
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST);
    tex_param(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST);
    let texels = blue(127);
    for target in FACES {
        tex_image(target, 0, 127, 127, &texels);
    }
    no_error(shape, "set texturing state");
    let s = render_quad(shape, true);
    expect_colour(shape, &s, BLUE, "127x127 faces on the default cube texture with a REPEAT wrap");
    assert_eq!(s.mip_filter, 0, "[{shape}] NEAREST: no mipmap");
    assert_eq!((s.wrap_s, s.wrap_t), (wrap_of(wrap_s), wrap_of(wrap_t)), "[{shape}] the wrap that was asked for");
}

#[test]
fn texture_completeness_cube_npot_t_repeat() {
    on_each_shape(|shape, _| wrap_cube(shape, GL_REPEAT, GL_CLAMP_TO_EDGE));
}

#[test]
fn texture_completeness_cube_npot_s_repeat() {
    on_each_shape(|shape, _| wrap_cube(shape, GL_CLAMP_TO_EDGE, GL_REPEAT));
}

#[test]
fn texture_completeness_cube_npot_all_repeat() {
    on_each_shape(|shape, _| wrap_cube(shape, GL_REPEAT, GL_REPEAT));
}

#[test]
fn texture_completeness_cube_npot_mirrored_repeat() {
    on_each_shape(|shape, _| wrap_cube(shape, GL_MIRRORED_REPEAT, GL_MIRRORED_REPEAT));
}

/* ---- negative_api.texture.generatemipmap_npot_wdt_hgt ------------------------------- */

/// With the extension listed the CTS logs and returns before any GL call
/// (es2fNegativeTextureApiTests.cpp:1341-1349); the result is Pass. This
/// asserts what that rests on, the extension, and then replays the body it
/// skips (256x257, 257x256, 257x257 RGB, glGenerateMipmap) for the answer
/// the extension gives: GL_NO_ERROR, where ES 2.0 without it gives
/// GL_INVALID_OPERATION.
#[test]
fn negative_api_texture_generatemipmap_npot_wdt_hgt() {
    on_each_shape(|shape, _| {
        assert_extension_listed(shape);
        // SAFETY: plain GL call.
        unsafe { glActiveTexture(GL_TEXTURE0) };
        gen_bind_2d();
        for (w, h) in [(256, 257), (257, 256), (257, 257)] {
            // SAFETY: null pixels only allocate.
            unsafe {
                glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as GLint, w, h, 0, GL_RGB, GL_UNSIGNED_BYTE, core::ptr::null());
                no_error(shape, &format!("glTexImage2D({w}x{h})"));
                glGenerateMipmap(GL_TEXTURE_2D);
            }
            no_error(shape, &format!("glGenerateMipmap({w}x{h})"));
        }
    });
}
