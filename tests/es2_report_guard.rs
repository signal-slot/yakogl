// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 answers of a `report-es2` build, pinned.
//!
//! yakogl reports OpenGL ES 2.0 by default (cargo feature `report-es2`, CMake
//! `YAKOGL_REPORT_ES2=ON`; README.md, "Reporting ES 2.0") and plan #1 left
//! zero dEQP-GLES2 failures (docs/es2-cts-failures.md). Plan #3 adds ES 3.0
//! features -- integer render targets, sized formats, transform feedback,
//! four draw buffers, occlusion queries -- in `src/gl/**`, and every one of
//! them has to stay behind `!REPORT_ES2`. This file is the net under that
//! promise: it asks the public GL API what a client of an ES 2.0 context sees
//! and compares it with what the code answered when the file was written. An
//! ES 3.0 item that moves an answer below must justify the move in its commit
//! (what the ES 2.0 specification, or the extension list, says it should be)
//! and change the pin in the same commit; an item that moves one by accident
//! finds out here, on the host, rather than in a dEQP-GLES2 run on a board.
//!
//! The pins are what the code does TODAY, not what ES 2.0 asks for. Where the
//! two differ the line carries a `NOTE (non-conformant)` and the test still
//! pins the answer: this file changes no behaviour. The compiler rows run
//! only with `shaderc-online` (the offline table "compiles" any source and
//! fails at link time). The context is created once, on this thread, because
//! the library refuses a second thread; so there is one `#[test]`.
//!
//! Compiled only with `report-es2`: an ES 3.0 build answers all of this
//! differently on purpose.

#![cfg(feature = "report-es2")]

use core::ffi::{c_char, c_void};
use std::ffi::CStr;
use std::sync::Mutex;

use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::context::gl_context_create;
use yakogl::gl::framebuffer::{glBindRenderbuffer, glGenRenderbuffers, glRenderbufferStorage};
#[cfg(feature = "shaderc-online")]
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderInfoLog, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glGetError, glGetFloatv, glGetIntegerv, glGetString};
use yakogl::gl::texture::{glBindTexture, glGenTextures, glTexImage2D};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a platform that allocates and draws nothing ------------------------------
 *
 * `testutil::TestGl` is `#[cfg(test)]` inside the library and is not visible
 * here, so this file carries the least platform a context needs: heap memory
 * for the default surface and the textures, and a backend table that records
 * nothing (no test below draws). */

const HEAPS: [u64; 3] = [0x80_0000_0000, 0xDA_0000_0000, 0xE0_0000_0000];
static BUMP: Mutex<[u64; 3]> = Mutex::new([0; 3]);

unsafe extern "C" fn o_init(_: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn o_alloc(heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let Some(h) = HEAPS.iter().position(|&b| b == heap) else { return -22 };
    let mem = Box::leak(vec![0xA5u8; size.max(1)].into_boxed_slice());
    let mut bump = BUMP.lock().unwrap();
    let gpu = heap + bump[h];
    bump[h] += (size as u64).div_ceil(4096) * 4096;
    // SAFETY: `out` is the caller's descriptor.
    unsafe { *out = OsalGpuMem { cpu: mem.as_mut_ptr().cast(), gpu, phys: gpu, size, impl_: mem.as_mut_ptr().cast() } };
    0
}
unsafe extern "C" fn o_free(m: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller's descriptor.
    unsafe { *m = OsalGpuMem::ZERO };
    0
}
unsafe extern "C" fn o_mmu(_: u64, _: u64, _: usize, _: u32) -> i32 {
    0
}
unsafe extern "C" fn o_write(m: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    // SAFETY: the library's range inside the allocation it was handed.
    unsafe { core::ptr::copy_nonoverlapping(src.cast::<u8>(), (*m).cpu.cast::<u8>().add(off), n) };
    0
}
unsafe extern "C" fn o_read(m: *mut OsalGpuMem, off: usize, dst: *mut c_void, n: usize) -> i32 {
    // SAFETY: as `o_write`.
    unsafe { core::ptr::copy_nonoverlapping((*m).cpu.cast::<u8>().add(off), dst.cast::<u8>(), n) };
    0
}
unsafe extern "C" fn o_cache(_: *mut c_void, _: usize) {}

static OSAL: OsalOps = OsalOps {
    init: Some(o_init),
    shutdown: None,
    gpu_alloc: Some(o_alloc),
    gpu_free: Some(o_free),
    mmu_map: Some(o_mmu),
    map_phys: None,
    gpu_write: Some(o_write),
    gpu_read: Some(o_read),
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

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_kick_id() -> u32 {
    1
}
unsafe extern "C" fn b_kick_pending(_: u32) -> bool {
    false
}
unsafe extern "C" fn b_heap_base(h: GlesHeap) -> u64 {
    HEAPS[(h as usize).min(2)]
}

static BACKEND: YakoglBackend = YakoglBackend {
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    name: c"guard".as_ptr(),
    init: Some(b_init),
    shutdown: None,
    draw: None,
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: Some(b_ok),
    kick_id: Some(b_kick_id),
    kick_pending: Some(b_kick_pending),
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
};

/* ---- helpers ------------------------------------------------------------------ */

/// The pins that did not hold, reported together at the end.
struct Pins(Vec<String>);

impl Pins {
    fn eq(&mut self, what: &str, got: impl AsRef<str>, want: impl AsRef<str>) {
        let (got, want) = (got.as_ref(), want.as_ref());
        if got != want {
            self.0.push(format!("{what}: got {got:?}, pinned {want:?}"));
        }
    }
}

/// glGetError as a word; the queue is drained, so a call's error is its own.
fn err() -> String {
    match unsafe { glGetError() } {
        GL_NO_ERROR => "ok".into(),
        GL_INVALID_ENUM => "INVALID_ENUM".into(),
        GL_INVALID_VALUE => "INVALID_VALUE".into(),
        GL_INVALID_OPERATION => "INVALID_OPERATION".into(),
        e => format!("0x{e:x}"),
    }
}

/// glGetIntegerv: the value, or the error word if the query raised one.
fn geti(pname: u32) -> String {
    let mut v = [i32::MIN; 4];
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    match err().as_str() {
        "ok" => v[0].to_string(),
        e => e.to_string(),
    }
}

/// glGetFloatv of a two-value range.
fn getf2(pname: u32) -> String {
    let mut v = [f32::NAN; 4];
    unsafe { glGetFloatv(pname, v.as_mut_ptr()) };
    match err().as_str() {
        "ok" => format!("{} {}", v[0], v[1]),
        e => e.to_string(),
    }
}

fn get_string(name: u32) -> String {
    // SAFETY: glGetString returns a NUL-terminated static string.
    let s = unsafe { CStr::from_ptr(glGetString(name).cast::<c_char>()) }.to_string_lossy().into_owned();
    assert_eq!(err(), "ok", "glGetString({name:#x})");
    s
}

/// Compile `src`: (GL_COMPILE_STATUS, the info log).
#[cfg(feature = "shaderc-online")]
fn compile(kind: u32, src: &str) -> (bool, String) {
    // SAFETY: the sources are live for the calls; the log buffer is as long as it is told.
    unsafe {
        let s = glCreateShader(kind);
        let p = [src.as_ptr().cast::<c_char>()];
        let l = [src.len() as i32];
        glShaderSource(s, 1, p.as_ptr(), l.as_ptr());
        glCompileShader(s);
        let mut ok = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        let mut log = [0u8; 512];
        let mut n = 0;
        glGetShaderInfoLog(s, 512, &mut n, log.as_mut_ptr().cast());
        (ok != 0, String::from_utf8_lossy(&log[..n as usize]).into_owned())
    }
}

/* ---- per-GPU answers ------------------------------------------------------------ */

/// GL_MAX_VARYING_VECTORS (and, times four, GL_MAX_VARYING_COMPONENTS and the
/// vertex-output and fragment-input component limits): the GC7000 has 15, the
/// PowerVR and the V3D 8 (context.rs MAX_VARYING_VECTORS).
const VARYING_VECTORS: u32 = if cfg!(feature = "gpu-vivante") { 15 } else { 8 };

/// GL_ALIASED_POINT_SIZE_RANGE's upper end: the PowerVR 511, the GC7000 64,
/// the V3D 512. The lower end is 1 everywhere.
const POINT_SIZE_MAX: &str = if cfg!(feature = "gpu-powervr") {
    "1 511"
} else if cfg!(feature = "gpu-vivante") {
    "1 64"
} else {
    "1 512"
};

/// The GL_EXTENSIONS string of a report-es2 build: the same on every GPU
/// (state.rs `ALL_EXTENSION_NAMES`), the ES 2.0 tail included. Pinned
/// exactly, in order. Not listed, and so refused below: GL_OES_depth24,
/// GL_OES_texture_float, GL_EXT_sRGB, GL_EXT_draw_buffers (and with it any
/// draw buffer beyond the first).
const EXTENSIONS: &str = "GL_OES_EGL_image GL_OES_standard_derivatives GL_OES_compressed_ETC1_RGB8_texture \
GL_OES_compressed_ETC2_RGB8_texture GL_OES_compressed_ETC2_RGBA8_texture GL_OES_vertex_array_object \
GL_OES_mapbuffer GL_OES_fbo_render_mipmap GL_EXT_texture_format_BGRA8888 GL_OES_rgb8_rgba8 \
GL_OES_required_internalformat GL_EXT_texture_rg GL_OES_texture_half_float GL_EXT_color_buffer_half_float \
GL_NV_fbo_color_attachments GL_EXT_debug_marker GL_OES_texture_npot GL_OES_depth_texture \
GL_OES_packed_depth_stencil";

/* ---- the test --------------------------------------------------------------------- */

#[test]
fn an_es2_report_answers_as_es2() {
    let mut p = Pins(Vec::new());
    // SAFETY: one thread, one context, created here; every pointer passed is live for its call.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        #[cfg(feature = "shaderc-online")]
        {
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        }
        assert!(!gl_context_create(64, 64).is_null(), "no context");

        /* -- identity -- */
        p.eq("GL_VERSION", get_string(GL_VERSION), "OpenGL ES 2.0 YakoGL");
        p.eq("GL_SHADING_LANGUAGE_VERSION", get_string(GL_SHADING_LANGUAGE_VERSION), "OpenGL ES GLSL ES 1.00");
        let ext = get_string(GL_EXTENSIONS);
        p.eq("GL_EXTENSIONS", &ext, EXTENSIONS);
        for (n, q) in [
            ("GL_MAJOR_VERSION", GL_MAJOR_VERSION),
            ("GL_MINOR_VERSION", GL_MINOR_VERSION),
            ("GL_NUM_EXTENSIONS", GL_NUM_EXTENSIONS),
        ] {
            p.eq(n, geti(q), "INVALID_ENUM");
        }

        /* -- draw buffers: ES 2.0 core has neither the query nor the enums -- */
        p.eq("GL_MAX_DRAW_BUFFERS", geti(GL_MAX_DRAW_BUFFERS), "INVALID_ENUM");
        for i in 0..4 {
            p.eq(&format!("GL_DRAW_BUFFER{i}"), geti(GL_DRAW_BUFFER0 + i), "INVALID_ENUM");
        }
        /* GL_NV_fbo_color_attachments is listed: the attachment points exist
         * (three on every GPU, CAP_COLOR_ATTACHMENTS) though nothing draws
         * into more than the first. */
        p.eq("GL_MAX_COLOR_ATTACHMENTS", geti(GL_MAX_COLOR_ATTACHMENTS), "3");

        /* -- transform feedback.
         * NOTE (non-conformant): ES 2.0 has no such enums, so GL_INVALID_ENUM
         * is what an ES 2.0 context owes; the code answers 0 with no error
         * (state.rs get_integerv, "Nothing is ever captured"). Pinned as 0 --
         * the item that implements transform feedback must keep the
         * report-es2 answer 0 (or move it to INVALID_ENUM and say so). -- */
        for (n, q) in [
            ("GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS),
            ("GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS),
            ("GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS),
        ] {
            p.eq(n, geti(q), "0");
        }

        /* -- every MAX_* ES 2.0 defines (2.0 table 6.20 and the shader-limit
         * queries) -- */
        let vv = VARYING_VECTORS;
        for (n, q, want) in [
            ("GL_MAX_VERTEX_ATTRIBS", GL_MAX_VERTEX_ATTRIBS, "16".to_string()),
            ("GL_MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS, "384".into()),
            ("GL_MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS, "384".into()),
            ("GL_MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS, vv.to_string()),
            ("GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, "32".into()),
            ("GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, "16".into()),
            ("GL_MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS, "16".into()),
            ("GL_MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, "4096".into()),
            ("GL_MAX_CUBE_MAP_TEXTURE_SIZE", GL_MAX_CUBE_MAP_TEXTURE_SIZE, "4096".into()),
            ("GL_MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE, "4096".into()),
            ("GL_SUBPIXEL_BITS", GL_SUBPIXEL_BITS, "4".into()),
            ("GL_NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS, "3".into()),
            ("GL_NUM_SHADER_BINARY_FORMATS", GL_NUM_SHADER_BINARY_FORMATS, "0".into()),
            /* NOTE (non-conformant): the ES 3.0 limits below are not ES 2.0
             * enums, yet a report-es2 build answers them instead of raising
             * GL_INVALID_ENUM (only the three version queries and
             * GL_MAX_DRAW_BUFFERS are refused). Pinned as answered. */
            ("GL_MAX_SAMPLES", GL_MAX_SAMPLES, "1".into()),
            ("GL_MAX_3D_TEXTURE_SIZE", GL_MAX_3D_TEXTURE_SIZE, "256".into()),
            ("GL_MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS, "256".into()),
            ("GL_MAX_ELEMENTS_VERTICES", GL_MAX_ELEMENTS_VERTICES, "16777215".into()),
            ("GL_MAX_ELEMENTS_INDICES", GL_MAX_ELEMENTS_INDICES, "16777215".into()),
            ("GL_MAX_ELEMENT_INDEX", GL_MAX_ELEMENT_INDEX, "16777215".into()),
            ("GL_MAX_VERTEX_UNIFORM_COMPONENTS", GL_MAX_VERTEX_UNIFORM_COMPONENTS, "1536".into()),
            ("GL_MAX_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_FRAGMENT_UNIFORM_COMPONENTS, "1536".into()),
            ("GL_MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS, (vv * 4).to_string()),
            ("GL_MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS, (vv * 4).to_string()),
            ("GL_MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS, (vv * 4).to_string()),
            ("GL_MAX_VERTEX_UNIFORM_BLOCKS", GL_MAX_VERTEX_UNIFORM_BLOCKS, "12".into()),
            ("GL_MAX_FRAGMENT_UNIFORM_BLOCKS", GL_MAX_FRAGMENT_UNIFORM_BLOCKS, "12".into()),
            ("GL_MAX_COMBINED_UNIFORM_BLOCKS", GL_MAX_COMBINED_UNIFORM_BLOCKS, "24".into()),
            ("GL_MAX_UNIFORM_BUFFER_BINDINGS", GL_MAX_UNIFORM_BUFFER_BINDINGS, "24".into()),
            ("GL_MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE, "6144".into()),
            ("GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS, "1536".into()),
            ("GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS, "1536".into()),
            ("GL_MAX_PROGRAM_TEXEL_OFFSET", GL_MAX_PROGRAM_TEXEL_OFFSET, "7".into()),
            ("GL_MIN_PROGRAM_TEXEL_OFFSET", GL_MIN_PROGRAM_TEXEL_OFFSET, "-8".into()),
            ("GL_MAX_SERVER_WAIT_TIMEOUT", GL_MAX_SERVER_WAIT_TIMEOUT, "0".into()),
            ("GL_NUM_PROGRAM_BINARY_FORMATS", GL_NUM_PROGRAM_BINARY_FORMATS, "0".into()),
            ("GL_MAX_TEXTURE_LOD_BIAS", GL_MAX_TEXTURE_LOD_BIAS, "2".into()),
        ] {
            p.eq(n, geti(q), want);
        }
        p.eq(
            "GL_MAX_VIEWPORT_DIMS",
            {
                let mut v = [0i32; 2];
                glGetIntegerv(GL_MAX_VIEWPORT_DIMS, v.as_mut_ptr());
                format!("{} {} {}", v[0], v[1], err())
            },
            "4096 4096 ok",
        );
        p.eq("GL_ALIASED_LINE_WIDTH_RANGE", getf2(GL_ALIASED_LINE_WIDTH_RANGE), "1 16");
        p.eq("GL_ALIASED_POINT_SIZE_RANGE", getf2(GL_ALIASED_POINT_SIZE_RANGE), POINT_SIZE_MAX);

        /* -- renderbuffer storage --
         * Extensions that are listed allow their formats: GL_OES_rgb8_rgba8
         * (RGB8, RGBA8), GL_EXT_texture_rg (R8), GL_EXT_color_buffer_half_float
         * (RGBA16F), GL_OES_packed_depth_stencil (DEPTH24_STENCIL8). */
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        let depth24_listed = ext.split(' ').any(|e| e == "GL_OES_depth24");
        for (n, f, want) in [
            ("RGBA4", 0x8056u32, "ok"),
            ("RGB5_A1", 0x8057, "ok"),
            ("RGB565", 0x8D62, "ok"),
            ("DEPTH_COMPONENT16", 0x81A5, "ok"),
            ("STENCIL_INDEX8", 0x8D48, "ok"),
            ("RGBA8", 0x8058, "ok"),
            ("RGB8", 0x8051, "ok"),
            ("R8", 0x8229, "ok"),
            ("RGBA16F", GL_RGBA16F, "ok"),
            ("DEPTH24_STENCIL8", 0x88F0, "ok"),
            /* GL_OES_depth24 is not listed: DEPTH_COMPONENT24 is not an ES 2.0
             * renderbuffer format (4e9397c) */
            ("DEPTH_COMPONENT24", GL_DEPTH_COMPONENT24, if depth24_listed { "ok" } else { "INVALID_ENUM" }),
            /* NOTE (non-conformant): DEPTH_COMPONENT32F is an ES 3.0 format
             * and is accepted with no extension naming it. */
            ("DEPTH_COMPONENT32F", 0x8CAC, "ok"),
            /* the integer, sRGB and packed-float colour formats are refused */
            ("RGBA8UI", GL_RGBA8UI, "INVALID_ENUM"),
            ("R32I", GL_R32I, "INVALID_ENUM"),
            ("SRGB8_ALPHA8", GL_SRGB8_ALPHA8, "INVALID_ENUM"),
            ("R11F_G11F_B10F", GL_R11F_G11F_B10F, "INVALID_ENUM"),
        ] {
            glRenderbufferStorage(GL_RENDERBUFFER, f, 8, 8);
            p.eq(&format!("glRenderbufferStorage({n})"), err(), want);
        }

        /* -- glTexImage2D: (name, internalformat, format, type) -> error.
         * The unsized ES 2.0 formats work; every ES 3.0 sized format that
         * needs an extension this build does not list is refused, and an
         * extension that IS listed (GL_OES_texture_half_float,
         * GL_EXT_texture_rg, GL_OES_required_internalformat,
         * GL_OES_depth_texture, GL_OES_packed_depth_stencil) allows its own.
         * The error words are pinned as the code gives them (INVALID_ENUM for
         * an integer internal format; INVALID_VALUE or INVALID_OPERATION for
         * the others). -- */
        let mut t = 0;
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        const RGBA: u32 = 0x1908;
        const RGB: u32 = 0x1907;
        const UBYTE: u32 = 0x1401;
        for (n, ifmt, fmt, ty, want) in [
            ("RGBA/UNSIGNED_BYTE", RGBA, RGBA, UBYTE, "ok"),
            ("RGBA/UNSIGNED_SHORT_4_4_4_4", RGBA, RGBA, 0x8033, "ok"),
            ("RGBA/HALF_FLOAT_OES", RGBA, RGBA, 0x8D61, "ok"),
            ("RED/UNSIGNED_BYTE", 0x1903, 0x1903, UBYTE, "ok"),
            ("R8", 0x8229, 0x1903, UBYTE, "ok"),
            ("RG8", 0x822B, 0x8227, UBYTE, "ok"),
            ("RGBA8", 0x8058, RGBA, UBYTE, "ok"),
            ("RGBA4", 0x8056, RGBA, 0x8033, "ok"),
            ("DEPTH24_STENCIL8", 0x88F0, 0x84F9, 0x84FA, "ok"),
            /* no GL_OES_texture_float */
            ("RGBA/FLOAT", RGBA, RGBA, 0x1406, "INVALID_OPERATION"),
            /* integer formats */
            ("RGBA8UI", GL_RGBA8UI, 0x8D99, UBYTE, "INVALID_ENUM"),
            ("R32I", GL_R32I, 0x8D94, 0x1404, "INVALID_ENUM"),
            /* sized ES 3.0 formats */
            ("R11F_G11F_B10F/UNSIGNED_INT_10F_11F_11F_REV", GL_R11F_G11F_B10F, RGB, 0x8C3B, "INVALID_OPERATION"),
            ("R11F_G11F_B10F/HALF_FLOAT", GL_R11F_G11F_B10F, RGB, 0x140B, "INVALID_VALUE"),
            ("R11F_G11F_B10F/FLOAT", GL_R11F_G11F_B10F, RGB, 0x1406, "INVALID_VALUE"),
            ("RGBA16F/HALF_FLOAT", GL_RGBA16F, RGBA, 0x140B, "INVALID_VALUE"),
            ("RGBA16F/HALF_FLOAT_OES", GL_RGBA16F, RGBA, 0x8D61, "INVALID_VALUE"),
            ("RGBA16F/FLOAT", GL_RGBA16F, RGBA, 0x1406, "INVALID_VALUE"),
            ("RGBA32F/FLOAT", 0x8814, RGBA, 0x1406, "INVALID_VALUE"),
            ("SRGB8_ALPHA8", GL_SRGB8_ALPHA8, RGBA, UBYTE, "INVALID_VALUE"),
            ("RGB9_E5", 0x8C3D, RGB, 0x8C3E, "INVALID_OPERATION"),
            ("RGBA8_SNORM", 0x8F97, RGBA, UBYTE, "INVALID_VALUE"),
            ("DEPTH_COMPONENT24/UNSIGNED_INT", 0x81A6, 0x1902, 0x1405, "INVALID_VALUE"),
        ] {
            glTexImage2D(GL_TEXTURE_2D, 0, ifmt as i32, 4, 4, 0, fmt, ty, core::ptr::null());
            p.eq(&format!("glTexImage2D({n})"), err(), want);
        }

        /* -- the compiler: GLSL ES 3.00 is out of scope and says so; ES 1.00
         * has one draw buffer -- */
        #[cfg(feature = "shaderc-online")]
        {
            let vs = "#version 300 es\nin vec4 a;\nvoid main() { gl_Position = a; }\n";
            let fs = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";
            for (n, kind, src) in [("vertex", GL_VERTEX_SHADER, vs), ("fragment", GL_FRAGMENT_SHADER, fs)] {
                let (ok, log) = compile(kind, src);
                p.eq(&format!("#version 300 es {n} shader compiles"), ok.to_string(), "false");
                p.eq(&format!("#version 300 es {n} shader has an info log"), (!log.is_empty()).to_string(), "true");
            }
            let es100 = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
            p.eq("ES 1.00 fragment shader compiles", compile(GL_FRAGMENT_SHADER, es100).0.to_string(), "true");
            /* gl_MaxDrawBuffers is 1 (4e9397c): the last element is index 0,
             * the next does not exist */
            let last = "precision mediump float;\nvoid main() { gl_FragData[gl_MaxDrawBuffers - 1] = vec4(1.0); }\n";
            let past = "precision mediump float;\nvoid main() { gl_FragData[gl_MaxDrawBuffers] = vec4(1.0); }\n";
            let one = "precision mediump float;\nvoid main() { gl_FragData[1] = vec4(1.0); }\n";
            p.eq(
                "gl_FragData[gl_MaxDrawBuffers - 1] compiles",
                compile(GL_FRAGMENT_SHADER, last).0.to_string(),
                "true",
            );
            p.eq("gl_FragData[gl_MaxDrawBuffers] compiles", compile(GL_FRAGMENT_SHADER, past).0.to_string(), "false");
            p.eq("gl_FragData[1] compiles", compile(GL_FRAGMENT_SHADER, one).0.to_string(), "false");
        }
        p.eq("no stray error", err(), "ok");
    }
    assert!(p.0.is_empty(), "the ES 2.0 report moved:\n  {}", p.0.join("\n  "));
}
