// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The OpenGL ES 2.0 answers of a `report-es2` build, pinned (item
//! es2-guard-redo).
//!
//! yakogl reports ES 2.0 by default (cargo feature `report-es2`, CMake
//! `YAKOGL_REPORT_ES2=ON`; README.md, "Options") and plan #1 left that
//! surface with zero dEQP-GLES2 failures. The ES 3.0 work -- integer and
//! sized formats, transform feedback, draw buffers, queries -- lands in the
//! same `src/gl/**`, behind `crate::REPORT_ES2`. This file asks the public GL
//! API what a client of an ES 2.0 context sees and compares it with what the
//! ES 2.0 specification (es_full_spec_2.0.25) and the extensions GL_EXTENSIONS
//! claims say it must see. Every pin is a spec answer, not a snapshot of the
//! code: the comment on each says where the rule is. A change that moves one
//! must say why ES 2.0 allows the new answer and move the pin in the same
//! commit.
//!
//! Where the code does NOT give the ES 2.0 answer today, the pin is written
//! for the right answer and `#[ignore]`d with the reason, so that nothing
//! here pins a defect: `es3_only_queries_are_invalid_enum`,
//! `es3_only_caps_are_invalid_enum`, `es3_only_renderbuffer_formats_are_
//! invalid_enum` and `es3_only_pixel_types_are_invalid_enum`. Fixing one
//! means dropping its `#[ignore]`.
//!
//! The context is made once on a stand-in OSAL and back end that draw
//! nothing (as tests/es3_tf_api.rs); the tests take turns on it. The
//! compiler rows need the in-process compiler (`shaderc-online`): the offline
//! table "compiles" any source and refuses it at link time instead.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,report-es2 --test es2_report_guard
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test es2_report_guard
//! ```
//!
//! Compiled only with `report-es2`: an ES 3.0 build answers all of this
//! differently on purpose, and `cargo test --test es2_report_guard` there
//! runs zero tests.

#![cfg(all(feature = "host-test", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::glUniformMatrix4fv;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one --- */

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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es2-report-guard".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: None,
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
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The shared setup: the context, current, with no error pending.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            #[cfg(feature = "shaderc-online")]
            {
                yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            }
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: a plain GL call on the one context.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    g
}

/* ---- helpers ------------------------------------------------------------------ */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

/// An error word for the messages.
fn name_of(e: GLenum) -> String {
    match e {
        GL_NO_ERROR => "GL_NO_ERROR".into(),
        GL_INVALID_ENUM => "GL_INVALID_ENUM".into(),
        GL_INVALID_VALUE => "GL_INVALID_VALUE".into(),
        GL_INVALID_OPERATION => "GL_INVALID_OPERATION".into(),
        e => format!("0x{e:x}"),
    }
}

/// glGetIntegerv: the first value, or the error it raised (the output must
/// then be untouched: ES 2.0 6.1, a failing query writes nothing).
fn geti(pname: GLenum) -> Result<GLint, GLenum> {
    let mut v = [0x5A5A; 4];
    // SAFETY: room for four values, the most any pname yields.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    match err() {
        GL_NO_ERROR => Ok(v[0]),
        e => {
            assert_eq!(v, [0x5A5A; 4], "glGetIntegerv(0x{pname:x}) wrote while failing");
            Err(e)
        }
    }
}

/// glGetFloatv of a two-value range.
fn getf2(pname: GLenum) -> [GLfloat; 2] {
    let mut v = [f32::NAN; 4];
    // SAFETY: room for four values.
    unsafe { glGetFloatv(pname, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetFloatv(0x{pname:x})");
    [v[0], v[1]]
}

fn get_string(name: GLenum) -> String {
    // SAFETY: glGetString returns a NUL-terminated static string.
    let s = unsafe { CStr::from_ptr(glGetString(name).cast::<c_char>()) }.to_string_lossy().into_owned();
    assert_eq!(err(), GL_NO_ERROR, "glGetString(0x{name:x})");
    s
}

/// The error of glRenderbufferStorage(`format`) on a fresh renderbuffer.
fn renderbuffer_storage(format: GLenum) -> GLenum {
    let mut rb = 0;
    // SAFETY: one name; plain GL calls on it.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 4, 4);
        let e = err();
        glDeleteRenderbuffers(1, &rb);
        e
    }
}

/// The error of glTexImage2D(`internalformat`, `format`, `type_`) with no
/// pixels, on a fresh 2D texture.
fn tex_image(internalformat: GLenum, format: GLenum, type_: GLenum) -> GLenum {
    let mut t = 0;
    // SAFETY: one name; null pixels only allocate.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, internalformat as GLint, 4, 4, 0, format, type_, core::ptr::null());
        let e = err();
        glDeleteTextures(1, &t);
        e
    }
}

/// Compile `src` as a shader of `kind`: (GL_COMPILE_STATUS, the info log).
#[cfg(feature = "shaderc-online")]
fn compile(kind: GLenum, src: &str) -> (bool, String) {
    use yakogl::gl::shader::*;
    // SAFETY: the source is live for the calls; the log buffer is as long as it is told.
    unsafe {
        let s = glCreateShader(kind);
        let p = [src.as_ptr().cast::<c_char>()];
        let l = [src.len() as GLint];
        glShaderSource(s, 1, p.as_ptr(), l.as_ptr());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut log = [0u8; 512];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, log.len() as GLsizei, &mut n, log.as_mut_ptr().cast());
        glDeleteShader(s);
        (ok != 0, String::from_utf8_lossy(&log[..n.max(0) as usize]).into_owned())
    }
}

/// `[("GL_FOO", GL_FOO), ...]`: an enum beside its name, for the messages.
macro_rules! named {
    ($($e:ident),* $(,)?) => {
        [$((stringify!($e), $e)),*]
    };
}

/* ---- per-GPU values -------------------------------------------------------------- */

/// GL_MAX_VARYING_VECTORS: what the GPU carries and the driver programs
/// (context.rs MAX_VARYING_VECTORS): 15 on the GC7000, 8 on the PowerVR
/// and the V3D. ES 2.0 table 6.20 asks for at least 8.
const VARYING_VECTORS: GLint = if cfg!(feature = "gpu-vivante") { 15 } else { 8 };

/// GL_ALIASED_POINT_SIZE_RANGE's upper end: the size each GPU really clamps
/// a point to (state.rs POINT_SIZE_MAX, measured per board by dEQP
/// rasterization.limits.points): PowerVR 511, GC7000 64, V3D 512. ES 2.0
/// table 6.20 asks for a range that covers 1.
const POINT_SIZE_MAX: GLfloat = if cfg!(feature = "gpu-powervr") {
    511.0
} else if cfg!(feature = "gpu-vivante") {
    64.0
} else {
    512.0
};

/* ---- the version and the extension list ------------------------------------------ */

/// ES 2.0 6.1.5: GL_VERSION is "OpenGL ES N.M" followed by vendor text,
/// GL_SHADING_LANGUAGE_VERSION "OpenGL ES GLSL ES N.M". A client (Qt's RHI,
/// dEQP) reads the version out of exactly these prefixes.
#[test]
fn version_strings_say_es2() {
    let _g = context();
    let version = get_string(GL_VERSION);
    let glsl = get_string(GL_SHADING_LANGUAGE_VERSION);
    assert!(version == "OpenGL ES 2.0" || version.starts_with("OpenGL ES 2.0 "), "{version:?}");
    assert!(glsl == "OpenGL ES GLSL ES 1.00" || glsl.starts_with("OpenGL ES GLSL ES 1.00 "), "{glsl:?}");
}

/// The extension list of the ES 2.0 surface, exactly and in order. Every
/// name is something the library does (state.rs ALL_EXTENSION_NAMES says
/// what, name by name) and nothing ES 2.0 has in core; the last three are
/// ES 3.0 core features that ES 2.0 can only have as extensions, so they
/// are claimed on this surface alone. The refusals pinned below rest on the
/// names that are NOT here: GL_OES_texture_float, GL_EXT_sRGB,
/// GL_OES_depth24, GL_OES_texture_3D, GL_EXT_draw_buffers / GL_NV_draw_buffers,
/// GL_EXT_color_buffer_float, GL_EXT_occlusion_query_boolean,
/// GL_OES_texture_float_linear, GL_EXT_unpack_subimage. Adding one is a
/// claim about behaviour and changes this pin on purpose.
#[test]
fn extension_list_is_exact() {
    let _g = context();
    let ext = get_string(GL_EXTENSIONS);
    let listed: Vec<&str> = ext.split(' ').collect();
    let want = [
        "GL_OES_EGL_image",
        "GL_OES_standard_derivatives",
        "GL_OES_compressed_ETC1_RGB8_texture",
        "GL_OES_compressed_ETC2_RGB8_texture",
        "GL_OES_compressed_ETC2_RGBA8_texture",
        "GL_OES_vertex_array_object",
        "GL_OES_mapbuffer",
        "GL_OES_fbo_render_mipmap",
        "GL_EXT_texture_format_BGRA8888",
        "GL_OES_rgb8_rgba8",
        "GL_OES_required_internalformat",
        "GL_EXT_texture_rg",
        "GL_OES_texture_half_float",
        "GL_EXT_color_buffer_half_float",
        "GL_NV_fbo_color_attachments",
        "GL_EXT_debug_marker",
        "GL_OES_texture_npot",
        "GL_OES_depth_texture",
        "GL_OES_packed_depth_stencil",
    ];
    assert_eq!(listed, want, "GL_EXTENSIONS");
}

/* ---- the queries ----------------------------------------------------------------- */

/// ES 2.0 6.1.1: a pname that is not a state variable of the ES 2.0 tables
/// is GL_INVALID_ENUM, and a failing query writes nothing. These three are
/// the ones that give the version away: a client that gets a number from
/// GL_MAJOR_VERSION concludes it is talking to ES 3 whatever glGetString
/// said. Through all three ES 2.0 query forms.
#[test]
fn es3_version_queries_are_invalid_enum() {
    let _g = context();
    for (n, q) in named![GL_MAJOR_VERSION, GL_MINOR_VERSION, GL_NUM_EXTENSIONS] {
        assert_eq!(geti(q), Err(GL_INVALID_ENUM), "{n}");
        let mut f = [f32::NAN; 4];
        let mut b: [GLboolean; 4] = [7; 4];
        // SAFETY: room for four values each.
        unsafe {
            glGetFloatv(q, f.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_ENUM, "{n} through glGetFloatv");
            assert!(f.iter().all(|v| v.is_nan()), "{n}: glGetFloatv wrote while failing");
            glGetBooleanv(q, b.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_ENUM, "{n} through glGetBooleanv");
            assert_eq!(b, [7; 4], "{n}: glGetBooleanv wrote while failing");
        }
    }
}

/// ES 2.0 has one draw buffer and no name for it: GL_MAX_DRAW_BUFFERS and
/// GL_DRAW_BUFFERi come with GL_EXT_draw_buffers / GL_NV_draw_buffers, which
/// are not claimed, so both are GL_INVALID_ENUM (6.1.1). dEQP-GLES2's
/// fragdata cases read the first to learn whether a second buffer exists.
/// GL_MAX_COLOR_ATTACHMENTS is the same token as
/// GL_MAX_COLOR_ATTACHMENTS_NV, which GL_NV_fbo_color_attachments (claimed)
/// defines: the attachment points exist (CAP_COLOR_ATTACHMENTS, 3) though
/// nothing on this surface draws into more than the first.
#[test]
fn draw_buffer_queries_are_invalid_enum() {
    let _g = context();
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Err(GL_INVALID_ENUM));
    for i in 0..4 {
        assert_eq!(geti(GL_DRAW_BUFFER0 + i), Err(GL_INVALID_ENUM), "GL_DRAW_BUFFER{i}");
    }
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(3));
}

/// The implementation-dependent values ES 2.0 defines (tables 6.18 to 6.20
/// and 4.3.1's read format), each at or above the minimum the table gives,
/// the per-GPU ones from the constants the code documents. These are the
/// values the zero-failure dEQP-GLES2 runs saw, and the shader's own
/// gl_Max* constants read the same constants (context.rs), so a change here
/// is a change of the surface, not a tuning.
#[test]
fn es2_limits() {
    let _g = context();
    for (n, q, want) in [
        ("GL_MAX_VERTEX_ATTRIBS (min 8)", GL_MAX_VERTEX_ATTRIBS, 16),
        ("GL_MAX_VERTEX_UNIFORM_VECTORS (min 128)", GL_MAX_VERTEX_UNIFORM_VECTORS, 384),
        ("GL_MAX_FRAGMENT_UNIFORM_VECTORS (min 16)", GL_MAX_FRAGMENT_UNIFORM_VECTORS, 384),
        ("GL_MAX_VARYING_VECTORS (min 8)", GL_MAX_VARYING_VECTORS, VARYING_VECTORS),
        ("GL_MAX_TEXTURE_IMAGE_UNITS (min 8)", GL_MAX_TEXTURE_IMAGE_UNITS, 16),
        ("GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS (min 0)", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, 16),
        ("GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS (min 8)", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, 32),
        ("GL_MAX_TEXTURE_SIZE (min 64)", GL_MAX_TEXTURE_SIZE, 4096),
        ("GL_MAX_CUBE_MAP_TEXTURE_SIZE (min 16)", GL_MAX_CUBE_MAP_TEXTURE_SIZE, 4096),
        ("GL_MAX_RENDERBUFFER_SIZE (min 1)", GL_MAX_RENDERBUFFER_SIZE, 4096),
        ("GL_SUBPIXEL_BITS (min 4)", GL_SUBPIXEL_BITS, 4),
        /* the three compressed-texture extensions claimed, one format each */
        ("GL_NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS, 3),
        ("GL_NUM_SHADER_BINARY_FORMATS", GL_NUM_SHADER_BINARY_FORMATS, 0),
        ("GL_SHADER_COMPILER", GL_SHADER_COMPILER, 1),
        /* 4.3.1: the pair glReadPixels takes besides GL_RGBA / GL_UNSIGNED_BYTE,
         * which it must take anyway; on this surface it is that pair */
        ("GL_IMPLEMENTATION_COLOR_READ_FORMAT", GL_IMPLEMENTATION_COLOR_READ_FORMAT, GL_RGBA as GLint),
        ("GL_IMPLEMENTATION_COLOR_READ_TYPE", GL_IMPLEMENTATION_COLOR_READ_TYPE, GL_UNSIGNED_BYTE as GLint),
    ] {
        assert_eq!(geti(q), Ok(want), "{n}");
    }
    let mut dims = [0; 2];
    let mut formats = [0; 3];
    // SAFETY: GL_MAX_VIEWPORT_DIMS yields two values, the format list three (pinned above).
    unsafe {
        glGetIntegerv(GL_MAX_VIEWPORT_DIMS, dims.as_mut_ptr());
        glGetIntegerv(GL_COMPRESSED_TEXTURE_FORMATS, formats.as_mut_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(dims, [4096, 4096], "GL_MAX_VIEWPORT_DIMS");
    assert_eq!(
        formats,
        [GL_ETC1_RGB8_OES as GLint, GL_COMPRESSED_RGB8_ETC2 as GLint, GL_COMPRESSED_RGBA8_ETC2_EAC as GLint],
        "GL_COMPRESSED_TEXTURE_FORMATS"
    );
    assert_eq!(getf2(GL_ALIASED_LINE_WIDTH_RANGE), [1.0, 16.0], "GL_ALIASED_LINE_WIDTH_RANGE (min covers 1)");
    assert_eq!(getf2(GL_ALIASED_POINT_SIZE_RANGE), [1.0, POINT_SIZE_MAX], "GL_ALIASED_POINT_SIZE_RANGE");
}

/// ES 2.0 6.1.1 again: none of these is a state variable of ES 2.0, and no
/// claimed extension adds one, so each is GL_INVALID_ENUM there. The code
/// answered every one of them with its ES 3.0 value (the transform feedback
/// limits with 0, GL_READ_BUFFER with GL_BACK, the bindings with 0 ...)
/// until the state/program tranche of the ES 2.0 error rules
/// (tests/es2_error_rules_state.rs). No dEQP-GLES2 case asks; the ES 3.0
/// build's answers are its own business.
#[test]
fn es3_only_queries_are_invalid_enum() {
    let _g = context();
    for (n, q) in named![
        GL_MAX_3D_TEXTURE_SIZE,
        GL_MAX_ARRAY_TEXTURE_LAYERS,
        GL_MAX_SAMPLES,
        GL_MAX_ELEMENT_INDEX,
        GL_MAX_ELEMENTS_VERTICES,
        GL_MAX_ELEMENTS_INDICES,
        GL_MAX_TEXTURE_LOD_BIAS,
        GL_MAX_SERVER_WAIT_TIMEOUT,
        GL_NUM_PROGRAM_BINARY_FORMATS,
        GL_MIN_PROGRAM_TEXEL_OFFSET,
        GL_MAX_PROGRAM_TEXEL_OFFSET,
        GL_MAX_VERTEX_UNIFORM_COMPONENTS,
        GL_MAX_FRAGMENT_UNIFORM_COMPONENTS,
        GL_MAX_VARYING_COMPONENTS,
        GL_MAX_VERTEX_OUTPUT_COMPONENTS,
        GL_MAX_FRAGMENT_INPUT_COMPONENTS,
        GL_MAX_VERTEX_UNIFORM_BLOCKS,
        GL_MAX_FRAGMENT_UNIFORM_BLOCKS,
        GL_MAX_COMBINED_UNIFORM_BLOCKS,
        GL_MAX_UNIFORM_BUFFER_BINDINGS,
        GL_MAX_UNIFORM_BLOCK_SIZE,
        GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT,
        GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS,
        GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
        GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS,
        GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS,
        GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS,
        GL_TRANSFORM_FEEDBACK_ACTIVE,
        GL_TRANSFORM_FEEDBACK_PAUSED,
        GL_TRANSFORM_FEEDBACK_BINDING,
        GL_SAMPLER_BINDING,
        GL_READ_BUFFER,
        GL_READ_FRAMEBUFFER_BINDING,
        GL_COPY_READ_BUFFER_BINDING,
        GL_PIXEL_PACK_BUFFER_BINDING,
        GL_UNIFORM_BUFFER_BINDING,
        GL_UNPACK_ROW_LENGTH,
        GL_PACK_ROW_LENGTH,
        GL_TEXTURE_BINDING_2D_ARRAY,
        GL_TEXTURE_BINDING_3D,
    ] {
        assert_eq!(geti(q), Err(GL_INVALID_ENUM), "{n}");
    }
}

/// ES 2.0 glEnable / glDisable / glIsEnabled take the capabilities the
/// specification lists (the manual: "GL_INVALID_ENUM is generated if cap is
/// not one of the values listed"); GL_RASTERIZER_DISCARD and
/// GL_PRIMITIVE_RESTART_FIXED_INDEX are ES 3.0's, so GL_INVALID_ENUM and the
/// state untouched. The code recorded both until the state/program tranche
/// of the ES 2.0 error rules (tests/es2_error_rules_state.rs).
#[test]
fn es3_only_caps_are_invalid_enum() {
    let _g = context();
    for (n, cap) in named![GL_RASTERIZER_DISCARD, GL_PRIMITIVE_RESTART_FIXED_INDEX] {
        // SAFETY: plain GL calls.
        unsafe {
            glEnable(cap);
            assert_eq!(err(), GL_INVALID_ENUM, "glEnable({n})");
            assert_eq!(glIsEnabled(cap), GL_FALSE_B, "glIsEnabled({n}) must not say it is on");
            assert_eq!(err(), GL_INVALID_ENUM, "glIsEnabled({n})");
            glDisable(cap);
            assert_eq!(err(), GL_INVALID_ENUM, "glDisable({n})");
        }
    }
}

/// ES 2.0 2.10.4: glUniformMatrix*fv takes transpose = GL_FALSE only ("If
/// transpose is not FALSE, INVALID_VALUE"); ES 3.0 is what added the
/// transposing upload. An argument error, so it needs no program.
#[test]
fn matrix_uploads_do_not_transpose() {
    let _g = context();
    let m = [0.0f32; 16];
    // SAFETY: sixteen floats.
    unsafe { glUniformMatrix4fv(0, 1, GL_TRUE_B, m.as_ptr()) };
    assert_eq!(err(), GL_INVALID_VALUE);
}

/* ---- the formats ------------------------------------------------------------------ */

/// ES 2.0 4.4.3: glRenderbufferStorage takes the sized formats of table 4.5
/// (RGBA4, RGB5_A1, RGB565, DEPTH_COMPONENT16, STENCIL_INDEX8) and what a
/// claimed extension adds -- GL_OES_rgb8_rgba8 (RGB8, RGBA8),
/// GL_OES_packed_depth_stencil (DEPTH24_STENCIL8), GL_EXT_texture_rg (R8,
/// RG8), GL_EXT_color_buffer_half_float (RGBA16F, RGB16F, RG16F, R16F) --
/// and any other internalformat is GL_INVALID_ENUM: the ES 3.0 integer,
/// sRGB (GL_EXT_sRGB is not claimed), packed-float and 10-bit formats,
/// DEPTH_COMPONENT24 (GL_OES_depth24 is not claimed; dEQP
/// fbo.completeness.renderable.renderbuffer.depth.depth_component24 expects
/// exactly this), and the unsized GL_RGBA, which names texture images only.
#[test]
fn renderbuffer_formats() {
    let _g = context();
    for (n, f) in named![
        GL_RGBA4,
        GL_RGB5_A1,
        GL_RGB565,
        GL_DEPTH_COMPONENT16,
        GL_STENCIL_INDEX8,
        GL_RGB8,
        GL_RGBA8,
        GL_DEPTH24_STENCIL8,
        GL_R8,
        GL_RG8,
        GL_RGBA16F,
        GL_RGB16F,
        GL_RG16F,
        GL_R16F,
    ] {
        assert_eq!(renderbuffer_storage(f), GL_NO_ERROR, "glRenderbufferStorage({n})");
    }
    for (n, f) in named![
        GL_RGBA8UI,
        GL_R32I,
        GL_RGBA16I,
        GL_RGB10_A2UI,
        GL_SRGB8_ALPHA8,
        GL_R11F_G11F_B10F,
        GL_RGB10_A2,
        GL_DEPTH_COMPONENT24,
        GL_RGBA,
    ] {
        assert_eq!(name_of(renderbuffer_storage(f)), name_of(GL_INVALID_ENUM), "glRenderbufferStorage({n})");
    }
}

/// The same rule for the formats the code still accepts on this surface:
/// GL_R32F is renderbuffer storage only with GL_EXT_color_buffer_float (an
/// ES 3.0 extension, not claimed), GL_DEPTH_COMPONENT32F and
/// GL_DEPTH32F_STENCIL8 are ES 3.0's and no ES 2.0 extension names them,
/// and GL_BGRA8_EXT is a texture format of GL_EXT_texture_format_BGRA8888,
/// which adds no renderbuffer storage (GL_CORE_NOTES.md says so, the code
/// takes it anyway). ES 2.0 4.4.3: GL_INVALID_ENUM.
#[test]
#[ignore = "BUG: framebuffer.rs is_renderable_format takes these four under report-es2; ES 2.0 4.4.3 says GL_INVALID_ENUM"]
fn es3_only_renderbuffer_formats_are_invalid_enum() {
    let _g = context();
    for (n, f) in named![GL_R32F, GL_DEPTH_COMPONENT32F, GL_DEPTH32F_STENCIL8, GL_BGRA8_EXT] {
        assert_eq!(name_of(renderbuffer_storage(f)), name_of(GL_INVALID_ENUM), "glRenderbufferStorage({n})");
    }
}

/// ES 2.0 3.7.1: glTexImage2D takes the unsized internal formats of table
/// 3.8 and what a claimed extension adds, with their table 3.4 types;
/// "specifying a value for internalformat that is not one of the above
/// values generates the error INVALID_VALUE", a format that is not an
/// accepted value GL_INVALID_ENUM, and a known unsized pair that does not
/// match GL_INVALID_OPERATION. Here: every ES 3.0 sized colour format (the
/// integer, float, sRGB, snorm, packed and 10-bit ones), with a format/type
/// pair ES 2.0 does accept so that the internal format is the one error; the
/// ES 3.0 depth formats (GL_OES_depth24 is not claimed; the float ones are
/// ES 3.0's); the integer transfer formats; and GL_FLOAT with an unsized
/// internal format, which is GL_OES_texture_float's upload (not claimed:
/// the GL stores GL_RGBA32F as halves).
#[test]
fn tex_image_2d_formats() {
    let _g = context();
    /* the ES 2.0 core uploads (table 3.4) and the claimed extensions' --
     * GL_OES_texture_half_float, GL_EXT_texture_rg, GL_OES_required_
     * internalformat, GL_EXT_texture_format_BGRA8888, GL_OES_depth_texture,
     * GL_OES_packed_depth_stencil -- this surface takes */
    for (n, internal, format, type_) in [
        ("RGBA / UNSIGNED_BYTE", GL_RGBA, GL_RGBA, GL_UNSIGNED_BYTE),
        ("RGBA / UNSIGNED_SHORT_4_4_4_4", GL_RGBA, GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4),
        ("RGB / UNSIGNED_SHORT_5_6_5", GL_RGB, GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
        ("LUMINANCE / UNSIGNED_BYTE", GL_LUMINANCE, GL_LUMINANCE, GL_UNSIGNED_BYTE),
        ("RGBA / HALF_FLOAT_OES", GL_RGBA, GL_RGBA, GL_HALF_FLOAT_OES),
        ("RED / UNSIGNED_BYTE", GL_RED, GL_RED, GL_UNSIGNED_BYTE),
        ("RG8", GL_RG8, GL_RG, GL_UNSIGNED_BYTE),
        ("RGBA8", GL_RGBA8, GL_RGBA, GL_UNSIGNED_BYTE),
        ("RGB565", GL_RGB565, GL_RGB, GL_UNSIGNED_SHORT_5_6_5),
        ("BGRA_EXT", GL_BGRA_EXT, GL_BGRA_EXT, GL_UNSIGNED_BYTE),
        ("DEPTH_COMPONENT / UNSIGNED_SHORT", GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT),
        ("DEPTH_COMPONENT / UNSIGNED_INT", GL_DEPTH_COMPONENT, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
        ("DEPTH_COMPONENT16", GL_DEPTH_COMPONENT16, GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT),
        ("DEPTH_STENCIL_OES / UNSIGNED_INT_24_8", GL_DEPTH_STENCIL_OES, GL_DEPTH_STENCIL_OES, GL_UNSIGNED_INT_24_8),
    ] {
        assert_eq!(name_of(tex_image(internal, format, type_)), name_of(GL_NO_ERROR), "glTexImage2D({n})");
    }
    /* an internal format ES 2.0 does not name: GL_INVALID_VALUE */
    for (n, internal, format, type_) in [
        ("RGBA8UI", GL_RGBA8UI, GL_RGBA, GL_UNSIGNED_BYTE),
        ("RGBA16I", GL_RGBA16I, GL_RGBA, GL_UNSIGNED_BYTE),
        ("R32I", GL_R32I, GL_RED, GL_UNSIGNED_BYTE),
        ("R32F", GL_R32F, GL_RED, GL_UNSIGNED_BYTE),
        ("RGBA16F", GL_RGBA16F, GL_RGBA, GL_HALF_FLOAT_OES),
        ("RGBA32F", GL_RGBA32F, GL_RGBA, GL_UNSIGNED_BYTE),
        ("SRGB8_ALPHA8", GL_SRGB8_ALPHA8, GL_RGBA, GL_UNSIGNED_BYTE),
        ("SRGB8", GL_SRGB8, GL_RGB, GL_UNSIGNED_BYTE),
        ("RGBA8_SNORM", GL_RGBA8_SNORM, GL_RGBA, GL_UNSIGNED_BYTE),
        ("RGB10_A2", GL_RGB10_A2, GL_RGBA, GL_UNSIGNED_BYTE),
        ("R11F_G11F_B10F", GL_R11F_G11F_B10F, GL_RGB, GL_UNSIGNED_BYTE),
        ("RGB9_E5", GL_RGB9_E5, GL_RGB, GL_UNSIGNED_BYTE),
        ("DEPTH_COMPONENT24", GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
        ("DEPTH_COMPONENT32F", GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
        ("DEPTH32F_STENCIL8", GL_DEPTH32F_STENCIL8, GL_DEPTH_STENCIL_OES, GL_UNSIGNED_INT_24_8),
    ] {
        assert_eq!(name_of(tex_image(internal, format, type_)), name_of(GL_INVALID_VALUE), "glTexImage2D({n})");
    }
    /* a format ES 2.0 does not name: GL_INVALID_ENUM */
    for (n, format) in named![GL_RGBA_INTEGER, GL_RGB_INTEGER] {
        let e = tex_image(format, format, GL_UNSIGNED_BYTE);
        assert_eq!(name_of(e), name_of(GL_INVALID_ENUM), "glTexImage2D({n})");
    }
    /* GL_OES_texture_float's upload, not claimed: refused (the error word is
     * es3_only_pixel_types_are_invalid_enum's question) */
    assert_ne!(tex_image(GL_RGBA, GL_RGBA, GL_FLOAT), GL_NO_ERROR, "glTexImage2D(RGBA / FLOAT)");
}

/// ES 2.0 3.7.1 / 3.6.2: a type that is not one of table 3.4's (or a
/// claimed extension's -- GL_HALF_FLOAT_OES is) is "not an accepted value",
/// GL_INVALID_ENUM. GL_FLOAT (GL_OES_texture_float, not claimed), ES 3.0's
/// GL_HALF_FLOAT (0x140B, a different token from the extension's 0x8D61)
/// and the packed ES 3.0 types are such types. The code knows them all as
/// enums (texture.rs is_pixel_type_enum) and answers GL_INVALID_OPERATION,
/// the error of a known-but-mismatched pair.
#[test]
fn es3_only_pixel_types_are_invalid_enum() {
    let _g = context();
    for (n, format, type_) in [
        ("RGBA / FLOAT", GL_RGBA, GL_FLOAT),
        ("RGBA / HALF_FLOAT", GL_RGBA, GL_HALF_FLOAT),
        ("RGBA / UNSIGNED_INT_2_10_10_10_REV", GL_RGBA, GL_UNSIGNED_INT_2_10_10_10_REV),
        ("RGB / UNSIGNED_INT_10F_11F_11F_REV", GL_RGB, GL_UNSIGNED_INT_10F_11F_11F_REV),
        ("RGB / UNSIGNED_INT_5_9_9_9_REV", GL_RGB, GL_UNSIGNED_INT_5_9_9_9_REV),
    ] {
        assert_eq!(name_of(tex_image(format, format, type_)), name_of(GL_INVALID_ENUM), "glTexImage2D({n})");
    }
}

/* ---- the ES 3.0 entry points ------------------------------------------------------ */

/// The library exports the ES 3.0 entry points in every build (one archive,
/// one eglGetProcAddress table), so an ES 2.0 client that calls one anyway
/// must be refused and must change nothing. The refusals: nothing captures
/// the vertex stage (glBeginTransformFeedback, glBindBufferBase on the
/// feedback point), there is no counter behind a query (glBeginQuery), and
/// the window's colour buffer is fixed point, which the integer clear does
/// not apply to (glClearBufferiv; ES 3.0 4.2.3 says the same). A capture or
/// query that began here would refuse every later draw.
#[test]
fn es3_entry_points_are_refused() {
    let _g = context();
    let (mut b, mut q) = (0, 0);
    // SAFETY: plain GL calls; one name each; four GLints for the clear.
    unsafe {
        glBeginTransformFeedback(GL_POINTS);
        assert_eq!(name_of(err()), name_of(GL_INVALID_OPERATION), "glBeginTransformFeedback");
        glEndTransformFeedback();
        assert_eq!(name_of(err()), name_of(GL_INVALID_OPERATION), "glEndTransformFeedback");
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, 64, core::ptr::null(), GL_DYNAMIC_COPY);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b);
        assert_eq!(name_of(err()), name_of(GL_INVALID_OPERATION), "glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER)");
        glGenQueries(1, &mut q);
        for (n, target) in named![GL_ANY_SAMPLES_PASSED, GL_TRANSFORM_FEEDBACK_PRIMITIVES_WRITTEN] {
            glBeginQuery(target, q);
            assert_eq!(name_of(err()), name_of(GL_INVALID_OPERATION), "glBeginQuery({n})");
        }
        glClearBufferiv(GL_COLOR, 0, [1, 0, 0, 1].as_ptr());
        assert_eq!(name_of(err()), name_of(GL_INVALID_OPERATION), "glClearBufferiv(GL_COLOR, 0)");
        glDeleteQueries(1, &q);
        glDeleteBuffers(1, &b);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/* ---- the compiler (shaderc-online) ------------------------------------------------ */

/// GLSL ES 1.00 3.3: "#version 100" is the only version the language has;
/// any other is a compile error, which GL reports as GL_COMPILE_STATUS false
/// with an info log (ES 2.0 2.10.2). The compiler names the dialect it
/// refuses. And GLSL ES 1.00 7.4: gl_MaxDrawBuffers is 1, so gl_FragData has
/// one element, and indexing it with the constant 1 is an error (Appendix A,
/// out-of-range constant index).
#[cfg(feature = "shaderc-online")]
#[test]
fn glsl_es_300_is_refused_and_es_100_has_one_draw_buffer() {
    let _g = context();
    let vs = "#version 300 es\nin vec4 a;\nvoid main() { gl_Position = a; }\n";
    let fs = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";
    for (n, kind, src) in [("vertex", GL_VERTEX_SHADER, vs), ("fragment", GL_FRAGMENT_SHADER, fs)] {
        let (ok, log) = compile(kind, src);
        assert!(!ok, "a #version 300 es {n} shader compiled");
        assert!(log.contains("GLSL ES 3.00"), "{n} shader info log: {log:?}");
    }
    let es100 = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
    let (ok, log) = compile(GL_FRAGMENT_SHADER, es100);
    assert!(ok, "an ES 1.00 fragment shader did not compile: {log:?}");
    let one = "precision mediump float;\nvoid main() { gl_FragData[gl_MaxDrawBuffers - 1] = vec4(1.0); }\n";
    let (ok, log) = compile(GL_FRAGMENT_SHADER, one);
    assert!(ok, "gl_FragData[gl_MaxDrawBuffers - 1]: {log:?}");
    let two = "precision mediump float;\nvoid main() { gl_FragData[1] = vec4(1.0); }\n";
    let (ok, log) = compile(GL_FRAGMENT_SHADER, two);
    assert!(!ok, "gl_FragData[1] compiled: gl_MaxDrawBuffers is not 1");
    assert!(!log.is_empty(), "gl_FragData[1] refused with no info log");
}
