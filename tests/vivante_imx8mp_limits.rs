// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! What the i.MX8MP's Vivante build (GC7000UL r6204, `gpu-vivante`) reports
//! today, pinned value by value before the back end's limits and quirk flags
//! move into a per-chip profile. The profile adds a second row (the
//! STM32MP257F's GC 0x8000/0x6205, plan #322, issues #7 and #1281); its
//! acceptance is that the i.MX8MP row changes nothing this file holds.
//!
//! This is a characterization test, not a conformance one: tests/es3_caps.rs
//! holds the same limits against the ES 3.0 minimum and the hardware ceiling;
//! this file holds them to the exact number the build reports now. A value
//! that moves here on purpose is moved in this file in the same change, with
//! its new source.
//!
//! Each value cites the line that decides it. Most of them are GL-core
//! constants chosen per GPU by `cfg(feature = "gpu-vivante")` in src/gl and
//! src/backend.rs, not src/vivante: those are the ones a chip profile would
//! have to take over to move them. The back end's own table
//! (`vivante::backend::yakogl_backend_vivante`) and flags are pinned below
//! as well.
//!
//! Under `report-es2` (the i.MX8MP's dEQP-GLES2 image) the ES 3.0-only
//! queries are GL_INVALID_ENUM (src/gl/state.rs:1047-1095 `es3_only_pname`, applied at
//! :1117, :1242 for the version queries and :1280 for GL_MAX_DRAW_BUFFERS) and the extension list
//! gains its ES 2.0 tail (src/gl/state.rs:186, :213-218); both builds are
//! pinned.

#![cfg(all(feature = "host-test", feature = "gpu-vivante"))]

use core::ffi::{c_char, c_void, CStr};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend as gc;

/* ---- a context on the host (as tests/es3_caps.rs makes one) --------------- */

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

/// The Vivante back end's own table with its entry points taken out: every
/// data field (attribute formats, layout rules, the instance divisor) is the
/// i.MX8MP's, so a query that reads one answers as the board's build does,
/// and no entry point reaches hardware. Only the entry points are named, so
/// a new data field arrives from `yakogl_backend_vivante` by itself; a new
/// entry point does too, and has to be added here as None if a query ever
/// calls it.
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"vivante-imx8mp-limits".as_ptr() as *const c_char,
    init: Some(b_init),
    shutdown: None,
    draw: None,
    flush: None,
    finish: None,
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    wait_kick: None,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    ds_write: None,
    stencil_resolve: None,
    ..gc::yakogl_backend_vivante
};

static LIBRARY: std::sync::Mutex<()> = std::sync::Mutex::new(());
static CONTEXT: std::sync::Once = std::sync::Once::new();

fn context() -> std::sync::MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

/// glGetIntegerv's values, all it wrote (at most 8), and the error it raised.
fn geti(pname: GLenum) -> (Vec<GLint>, GLenum) {
    let mut v = [i32::MIN; 8];
    // SAFETY: room for the eight values the widest pname writes
    // (src/gl/state.rs STATE_VALUES).
    let err = unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetIntegerv(pname, v.as_mut_ptr());
        glGetError()
    };
    (v.into_iter().take_while(|&x| x != i32::MIN).collect(), err)
}

fn getf(pname: GLenum) -> (Vec<GLfloat>, GLenum) {
    let mut v = [f32::NAN; 8];
    // SAFETY: as above.
    let err = unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetFloatv(pname, v.as_mut_ptr());
        glGetError()
    };
    (v.into_iter().take_while(|x| !x.is_nan()).collect(), err)
}

fn gl_string(name: GLenum) -> String {
    // SAFETY: glGetString returns a static NUL-terminated string.
    unsafe { CStr::from_ptr(glGetString(name) as *const c_char) }.to_str().unwrap().to_owned()
}

/* ---- the limits ---------------------------------------------------------- */

/// A pinned glGetIntegerv answer: name, pname, the values, whether the
/// ES 2.0 report refuses it; the comment above each says where it comes from.
struct Pin {
    name: &'static str,
    pname: GLenum,
    values: &'static [GLint],
    es3_only: bool,
}

const fn pin(name: &'static str, pname: GLenum, values: &'static [GLint]) -> Pin {
    Pin { name, pname, values, es3_only: false }
}

const fn pin3(name: &'static str, pname: GLenum, values: &'static [GLint]) -> Pin {
    Pin { name, pname, values, es3_only: true }
}

/// Every integer limit a chip profile could move, as the i.MX8MP's build
/// answers it today.
const PINS: &[Pin] = &[
    /* ---- ES 2.0 (src/gl/state.rs get_integerv) ---- */
    // state.rs:1225, a literal for every GPU
    pin("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, &[4096]),
    // state.rs:1226
    pin("MAX_CUBE_MAP_TEXTURE_SIZE", GL_MAX_CUBE_MAP_TEXTURE_SIZE, &[4096]),
    // state.rs:1227
    pin("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE, &[4096]),
    // state.rs:1215-1218, MAX_VIEWPORT_DIM at state.rs:292
    pin("MAX_VIEWPORT_DIMS", GL_MAX_VIEWPORT_DIMS, &[4096, 4096]),
    // state.rs:1219, context.rs:78 MAX_VERTEX_ATTRIBS
    pin("MAX_VERTEX_ATTRIBS", GL_MAX_VERTEX_ATTRIBS, &[16]),
    // state.rs:1233, context.rs:89 MAX_UNIFORM_DWORDS / 4 (context.rs:83, 1536)
    pin("MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS, &[384]),
    // state.rs:1234, context.rs:90
    pin("MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS, &[384]),
    // state.rs:1235, context.rs:123-124, the gpu-vivante MAX_VARYING_VECTORS
    pin("MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS, &[15]),
    // state.rs:1220, context.rs:95 CAP_TEXTURE_UNITS
    pin("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS, &[16]),
    // state.rs:1224, backend.rs:688-689, the gpu-vivante GLES_VS_TEX_UNITS
    pin("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, &[16]),
    // state.rs:1221, context.rs:107 CAP_COMBINED_TEXTURE_UNITS
    pin("MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, &[32]),
    // state.rs:1285, context.rs:164 max_color_attachments = draw_targets(),
    // backend.rs:751-754 DRAW_TARGETS for gpu-vivante (3)
    pin("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS, &[3]),
    // state.rs:1211, texture.rs:3388-3423: ETC1, RGB8_ETC2, RGBA8_ETC2_EAC
    // (the sRGB pair needs the back end to sample it, texture.rs:3406)
    pin("NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS, &[3]),
    pin(
        "COMPRESSED_TEXTURE_FORMATS",
        GL_COMPRESSED_TEXTURE_FORMATS,
        &[GL_ETC1_RGB8_OES as GLint, GL_COMPRESSED_RGB8_ETC2 as GLint, GL_COMPRESSED_RGBA8_ETC2_EAC as GLint],
    ),
    /* ---- ES 3.0 (GL_INVALID_ENUM under report-es2) ---- */
    // state.rs:1284, context.rs:154 max_draw_buffers = draw_targets() (3)
    pin3("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS, &[3]),
    // state.rs:1289
    pin3("MAX_3D_TEXTURE_SIZE", GL_MAX_3D_TEXTURE_SIZE, &[256]),
    // state.rs:1288
    pin3("MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS, &[256]),
    // state.rs:1286
    pin3("MAX_SAMPLES", GL_MAX_SAMPLES, &[1]),
    // state.rs:1290
    pin3("MAX_ELEMENT_INDEX", GL_MAX_ELEMENT_INDEX, &[0xFF_FFFF]),
    // state.rs:1291
    pin3("MAX_ELEMENTS_VERTICES", GL_MAX_ELEMENTS_VERTICES, &[0xFF_FFFF]),
    pin3("MAX_ELEMENTS_INDICES", GL_MAX_ELEMENTS_INDICES, &[0xFF_FFFF]),
    // state.rs:1265, MAX_UNIFORM_DWORDS * 4
    pin3("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE, &[6144]),
    // state.rs:1262
    pin3("MAX_VERTEX_UNIFORM_BLOCKS", GL_MAX_VERTEX_UNIFORM_BLOCKS, &[12]),
    pin3("MAX_FRAGMENT_UNIFORM_BLOCKS", GL_MAX_FRAGMENT_UNIFORM_BLOCKS, &[12]),
    // state.rs:1263, program.rs:69 PROG_MAX_BLOCKS
    pin3("MAX_COMBINED_UNIFORM_BLOCKS", GL_MAX_COMBINED_UNIFORM_BLOCKS, &[24]),
    // state.rs:1264, context.rs:182
    pin3("MAX_UNIFORM_BUFFER_BINDINGS", GL_MAX_UNIFORM_BUFFER_BINDINGS, &[24]),
    // state.rs:1266
    pin3("UNIFORM_BUFFER_OFFSET_ALIGNMENT", GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT, &[4]),
    // state.rs:1249-1250, MAX_UNIFORM_DWORDS
    pin3("MAX_VERTEX_UNIFORM_COMPONENTS", GL_MAX_VERTEX_UNIFORM_COMPONENTS, &[1536]),
    pin3("MAX_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_FRAGMENT_UNIFORM_COMPONENTS, &[1536]),
    // state.rs:1267-1269
    pin3("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS, &[1536]),
    pin3("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS, &[1536]),
    // state.rs:1253, MAX_VARYING_VECTORS * 4
    pin3("MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS, &[60]),
    // state.rs:1254-1255, context.rs:172-173
    pin3("MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS, &[60]),
    pin3("MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS, &[60]),
    // state.rs:1308-1309, context.rs:176-177
    pin3("MIN_PROGRAM_TEXEL_OFFSET", GL_MIN_PROGRAM_TEXEL_OFFSET, &[-8]),
    pin3("MAX_PROGRAM_TEXEL_OFFSET", GL_MAX_PROGRAM_TEXEL_OFFSET, &[7]),
    // state.rs:1292
    pin3("MAX_TEXTURE_LOD_BIAS", GL_MAX_TEXTURE_LOD_BIAS, &[2]),
    // state.rs:1294-1305, tf.rs:32, :35, :43
    pin3("MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS, &[64]),
    pin3("MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS, &[4]),
    pin3("MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS, &[64]),
    // state.rs:1310
    pin3("MAX_SERVER_WAIT_TIMEOUT", GL_MAX_SERVER_WAIT_TIMEOUT, &[0]),
    // state.rs:1246-1247 (refused under report-es2 at :1242)
    pin3("MAJOR_VERSION", GL_MAJOR_VERSION, &[3]),
    pin3("MINOR_VERSION", GL_MINOR_VERSION, &[0]),
];

#[test]
fn the_integer_limits_are_what_the_imx8mp_build_reports_today() {
    let _lib = context();
    let mut wrong: Vec<String> = vec![];
    for p in PINS {
        let (got, err) = geti(p.pname);
        if p.es3_only && yakogl::REPORT_ES2 {
            if err != GL_INVALID_ENUM || !got.is_empty() {
                wrong.push(format!("{}: report-es2 wants GL_INVALID_ENUM, got {got:?} error 0x{err:x}", p.name));
            }
        } else if err != GL_NO_ERROR || got != p.values {
            wrong.push(format!("{}: pinned {:?}, got {got:?} error 0x{err:x}", p.name, p.values));
        }
    }
    assert!(wrong.is_empty(), "the i.MX8MP's reported limits moved:\n  {}", wrong.join("\n  "));
}

/// The two float ranges, through glGetFloatv and glGetIntegerv.
#[test]
fn the_line_width_and_point_size_ranges_are_pinned() {
    let _lib = context();
    // state.rs:993-997: 1.0 to 16.0, a literal for every GPU. (The ST row's
    // range is 1.0 to 1.0 by plan #322's Q11 study; that is the profile's
    // change, not this row's.)
    assert_eq!(getf(GL_ALIASED_LINE_WIDTH_RANGE), (vec![1.0, 16.0], GL_NO_ERROR));
    assert_eq!(geti(GL_ALIASED_LINE_WIDTH_RANGE), (vec![1, 16], GL_NO_ERROR));
    // state.rs:998-1002, POINT_SIZE_MAX at state.rs:322-325 (64.0 for
    // gpu-vivante, the online compiler's clamp, shaderc/online/select.rs:431)
    assert_eq!(getf(GL_ALIASED_POINT_SIZE_RANGE), (vec![1.0, 64.0], GL_NO_ERROR));
    assert_eq!(geti(GL_ALIASED_POINT_SIZE_RANGE), (vec![1, 64], GL_NO_ERROR));
}

/* ---- the strings --------------------------------------------------------- */

#[test]
fn the_renderer_names_the_gc7000ul() {
    let _lib = context();
    // state.rs:879-880
    assert_eq!(gl_string(GL_RENDERER), "Vivante GC7000UL (YakoGL)");
    assert_eq!(gl_string(GL_VENDOR), "Signal Slot");
    // state.rs:895-898
    let (version, glsl) = if yakogl::REPORT_ES2 {
        ("OpenGL ES 2.0 YakoGL", "OpenGL ES GLSL ES 1.00")
    } else {
        ("OpenGL ES 3.0 YakoGL", "OpenGL ES GLSL ES 3.00")
    };
    assert_eq!(gl_string(GL_VERSION), version);
    assert_eq!(gl_string(GL_SHADING_LANGUAGE_VERSION), glsl);
}

/// The extension string, sorted: state.rs:41-185 ALL_EXTENSION_NAMES and
/// :213-218 EXTENSION_NAMES and :255-260 EXTENSION_STRING; the last three only in the ES 2.0 report
/// (:186 ES2_ONLY_NAMES), npot there gated on the part's NON_POWER_OF_TWO
/// (:193-194, src/vivante/backend.rs:780).
#[test]
fn the_extension_string_is_pinned() {
    let _lib = context();
    let mut want = vec![
        "GL_EXT_color_buffer_half_float",
        "GL_EXT_debug_marker",
        "GL_EXT_texture_format_BGRA8888",
        "GL_EXT_texture_rg",
        "GL_NV_fbo_color_attachments",
        "GL_OES_EGL_image",
        "GL_OES_compressed_ETC1_RGB8_texture",
        "GL_OES_compressed_ETC2_RGB8_texture",
        "GL_OES_compressed_ETC2_RGBA8_texture",
        "GL_OES_fbo_render_mipmap",
        "GL_OES_mapbuffer",
        "GL_OES_required_internalformat",
        "GL_OES_rgb8_rgba8",
        "GL_OES_standard_derivatives",
        "GL_OES_texture_half_float",
        "GL_OES_vertex_array_object",
    ];
    if yakogl::REPORT_ES2 {
        want.extend(["GL_OES_depth_texture", "GL_OES_packed_depth_stencil", "GL_OES_texture_npot"]);
        want.sort_unstable();
    }
    let all = gl_string(GL_EXTENSIONS);
    let mut got: Vec<&str> = all.split(' ').collect();
    got.sort_unstable();
    assert_eq!(got, want);
    /* the indexed list is the same set (state.rs:910-922 glGetStringi; ES 3.0 only) */
    if !yakogl::REPORT_ES2 {
        assert_eq!(geti(GL_NUM_EXTENSIONS), (vec![want.len() as GLint], GL_NO_ERROR));
        let mut indexed: Vec<String> = (0..want.len() as GLuint)
            // SAFETY: every index is below GL_NUM_EXTENSIONS.
            .map(|i| unsafe { CStr::from_ptr(glGetStringi(GL_EXTENSIONS, i) as *const c_char) })
            .map(|s| s.to_str().unwrap().to_owned())
            .collect();
        indexed.sort_unstable();
        assert_eq!(indexed, want);
    }
}

/* ---- the back end's table and flags (src/vivante/backend.rs) ------------- */

/// The data fields of `yakogl_backend_vivante` (src/vivante/backend.rs
/// :3321-3388) and the flags beside it that a chip profile could take over.
#[test]
fn the_vivante_table_and_flags_are_pinned() {
    let b = &gc::yakogl_backend_vivante;
    // SAFETY: a static C string.
    assert_eq!(unsafe { CStr::from_ptr(b.name) }.to_str().unwrap(), "Vivante GC7000UL (i.MX8MP)"); // :3322
    assert_eq!(
        b.native_attrib_formats, // :3325-3329
        gles_attribf_bit(GLES_ATTRIBF_BYTE)
            | gles_attribf_bit(GLES_ATTRIBF_UBYTE)
            | gles_attribf_bit(GLES_ATTRIBF_SHORT)
            | gles_attribf_bit(GLES_ATTRIBF_USHORT)
            | gles_attribf_bit(GLES_ATTRIBF_FIXED)
    );
    assert!(b.rt_rgba8888_is_bgra); // :3332
    assert_eq!(b.mip_level_align, 64); // :3336
    assert!(b.compressed_blocks_in_rows); // :3341
    assert!(b.native_layered_textures); // :3346
    assert!(b.front_facing_shader_variant); // :3350
    assert!(!b.etc1_decoded_at_upload); // :3351
    assert_eq!(b.linear_row_align, 0); // :3372
    assert!(b.uniforms_host_only); // :3550 (route 3: the host arrays are copied into the arena)
    assert_eq!(b.max_instance_divisor, 0xffff); // :3554 (V1, the stream divisor state)
    assert!(!b.samples_texel_copies); // :3382

    /* which entry points the board's table has (each a behaviour the GL
     * core switches on: the PE mipmap path, the wide-target release) */
    assert!(b.generate_mipmap.is_some() && b.return_mipmap_chain.is_some());
    assert!(b.texture_released.is_some());
    assert!(b.surface_swapped.is_none() && b.texture_uploaded.is_none());
    assert!(b.ds_write.is_none() && b.stencil_resolve.is_none());
    /* the flags beside the table: NON_POWER_OF_TWO and DRAWS_WITHOUT_COLOUR
     * are const-asserted below the test */
    // :340-357: every integer surface but the 128-bit pair is a PE format
    assert_eq!(
        gc::GC_INT_RT_128BPP,
        (1 << (GLES_SURF_RGBA32I - GLES_SURF_INT_FIRST)) | (1 << (GLES_SURF_RGBA32UI - GLES_SURF_INT_FIRST))
    );
    assert_eq!(
        gc::GC_INTEGER_RT_FORMATS,
        ((1u32 << (GLES_SURF_INT_LAST - GLES_SURF_INT_FIRST + 1)) - 1) & !gc::GC_INT_RT_128BPP
    );
    /* the GL-core constants of this build (src/gl/context.rs, src/backend.rs) */
    assert_eq!(DRAW_TARGETS, 3);
    assert_eq!(GLES_VS_TEX_UNITS, 16);
    assert_eq!(yakogl::gl::context::MAX_VARYING_VECTORS, 15);
    assert_eq!(yakogl::gl::context::MAX_UNIFORM_DWORDS, 1536);
}

/* the two boolean flags, checked when the file compiles */
const _: () = assert!(gc::NON_POWER_OF_TWO); // src/vivante/backend.rs:780
const _: () = assert!(gc::DRAWS_WITHOUT_COLOUR); // src/vivante/backend.rs:759
