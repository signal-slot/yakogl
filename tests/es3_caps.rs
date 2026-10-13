// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 3.0 implementation-dependent limits each GPU build reports, queried
//! through the GL API on the host and held (a) against the minimum of the ES
//! 3.0.6 specification (Tables 6.32, 6.33, 6.34) and (b) against what the
//! hardware can do, where Mesa's v3d, etnaviv and pvr sources say so
//! (docs/es3-backend-caps.md gives the file:line of every ceiling). The numbers are the ones each GPU's
//! build reports (`gpu-powervr`, `gpu-vivante`, `gpu-v3d`).
//!
//! A limit below its minimum is not a silent pass: it is listed in
//! [`known_gaps`] by name, with the reason, and the test prints one line per
//! limit. A limit that is NOT listed must meet its minimum. A listed limit
//! that now meets it is reported as CLOSED ("remove it from known_gaps")
//! without failing, so the item that closes a gap does not have to touch
//! this file in the same change.
//!
//! Under `report-es2` the ES 3.0-only queries are GL_INVALID_ENUM
//! (gl/state.rs `es3_only_pname`, plus the version queries and
//! GL_MAX_DRAW_BUFFERS); the ES 2.0 limits and GL_MAX_COLOR_ATTACHMENTS(_NV)
//! keep their answers. The minimum test is therefore not run in that build;
//! `es2_report_answers_are_unchanged` pins both halves instead.

#![cfg(feature = "host-test")]
// each build runs one of the two tests; the helpers of the other are idle
#![cfg_attr(feature = "report-es2", allow(dead_code, unused_imports))]
#![cfg_attr(not(feature = "report-es2"), allow(dead_code))]

use core::ffi::{c_char, c_void};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and backend that make one --- */

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
    lock: None,
    unlock: None,
    thread_id: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-caps".as_ptr() as *const c_char,
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
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: None,
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
};

static LIBRARY: std::sync::Mutex<()> = std::sync::Mutex::new(());
static CONTEXT: std::sync::Once = std::sync::Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
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

fn geti(pname: GLenum) -> GLint {
    let mut v = [i32::MIN; 4];
    // SAFETY: room for the four values the widest pname returns.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    assert_ne!(v[0], i32::MIN, "glGetIntegerv(0x{pname:x}) wrote nothing");
    v[0]
}

fn geti_n(pname: GLenum) -> [GLint; 2] {
    let mut v = [i32::MIN; 4];
    // SAFETY: as above.
    unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
    [v[0], v[1]]
}

fn getf_n(pname: GLenum) -> [GLfloat; 2] {
    let mut v = [f32::NAN; 4];
    // SAFETY: as above.
    unsafe { glGetFloatv(pname, v.as_mut_ptr()) };
    [v[0], v[1]]
}

fn geti64(pname: GLenum) -> i64 {
    let mut v = [i64::MIN; 4];
    // SAFETY: as above.
    unsafe { glGetInteger64v(pname, v.as_mut_ptr()) };
    assert_ne!(v[0], i64::MIN, "glGetInteger64v(0x{pname:x}) wrote nothing");
    v[0]
}

/// The GL error a query raised, cleared.
fn error_of(pname: GLenum) -> GLenum {
    // SAFETY: plain GL calls; the query may write nothing.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        let mut v = [i32::MIN; 4];
        glGetIntegerv(pname, v.as_mut_ptr());
        glGetError()
    }
}

/* ---- the GPU this build is ------------------------------------------------ */

const GPU: &str = if cfg!(feature = "gpu-v3d") {
    "V3D 4.2"
} else if cfg!(feature = "gpu-vivante") {
    "GC7000UL"
} else {
    "BXS-4-64"
};

/* ---- the limits ---------------------------------------------------------- */

/// How a limit is held against its spec value.
#[derive(Clone, Copy)]
enum Rule {
    /// The implementation value must be at least the spec's ("minimum value").
    AtLeast,
    /// ... at most the spec's (the "maximum value" rows: MIN_PROGRAM_TEXEL_OFFSET,
    /// UNIFORM_BUFFER_OFFSET_ALIGNMENT, the lower end of a range).
    AtMost,
}

struct Limit {
    name: &'static str,
    pname: GLenum,
    spec: i64,
    rule: Rule,
}

fn lim(name: &'static str, pname: GLenum, spec: i64) -> Limit {
    Limit { name, pname, spec, rule: Rule::AtLeast }
}

/// ES 3.0.6 Tables 6.32, 6.33, 6.34 (the integer-valued ones).
fn limits() -> Vec<Limit> {
    vec![
        lim("MAX_3D_TEXTURE_SIZE", GL_MAX_3D_TEXTURE_SIZE, 256),
        lim("MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS, 256),
        lim("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS, 4),
        // 12 blocks * 16384 / 4 + 896 (Table 6.32 note: blocks * block size / 4 + default-block components)
        lim("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS, 50_048),
        lim("MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, 32),
        lim("MAX_COMBINED_UNIFORM_BLOCKS", GL_MAX_COMBINED_UNIFORM_BLOCKS, 24),
        // 12 * 16384 / 4 + 1024
        lim("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS, 50_176),
        lim("MAX_CUBE_MAP_TEXTURE_SIZE", GL_MAX_CUBE_MAP_TEXTURE_SIZE, 2048),
        lim("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS, 4),
        lim("MAX_ELEMENT_INDEX", GL_MAX_ELEMENT_INDEX, 16_777_215),
        lim("MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS, 60),
        lim("MAX_FRAGMENT_UNIFORM_BLOCKS", GL_MAX_FRAGMENT_UNIFORM_BLOCKS, 12),
        lim("MAX_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_FRAGMENT_UNIFORM_COMPONENTS, 896),
        lim("MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS, 224),
        lim("MAX_PROGRAM_TEXEL_OFFSET", GL_MAX_PROGRAM_TEXEL_OFFSET, 7),
        Limit { name: "MIN_PROGRAM_TEXEL_OFFSET", pname: GL_MIN_PROGRAM_TEXEL_OFFSET, spec: -8, rule: Rule::AtMost },
        lim("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE, 2048),
        lim("MAX_SAMPLES", GL_MAX_SAMPLES, 4),
        lim("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS, 16),
        lim("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, 2048),
        lim("MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS, 64),
        lim("MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS, 4),
        lim("MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS, 4),
        lim("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE, 16_384),
        lim("MAX_UNIFORM_BUFFER_BINDINGS", GL_MAX_UNIFORM_BUFFER_BINDINGS, 24),
        lim("MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS, 60),
        lim("MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS, 15),
        lim("MAX_VERTEX_ATTRIBS", GL_MAX_VERTEX_ATTRIBS, 16),
        lim("MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS, 64),
        lim("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, 16),
        lim("MAX_VERTEX_UNIFORM_BLOCKS", GL_MAX_VERTEX_UNIFORM_BLOCKS, 12),
        lim("MAX_VERTEX_UNIFORM_COMPONENTS", GL_MAX_VERTEX_UNIFORM_COMPONENTS, 1024),
        lim("MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS, 256),
        Limit {
            name: "UNIFORM_BUFFER_OFFSET_ALIGNMENT",
            pname: GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT,
            spec: 256,
            rule: Rule::AtMost,
        },
        lim("NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS, 10),
    ]
}

/// The limits that are below their minimum today, with the reason. Keep this
/// list true: it is the limit half of the gap list in docs/es3-backend-caps.md.
/// An entry is by name and holds for every GPU build unless it says otherwise.
fn known_gaps() -> Vec<(&'static str, &'static str)> {
    let mut v: Vec<(&'static str, &'static str)> = vec![
        ("MAX_COLOR_ATTACHMENTS", "3, not 4: gl/context.rs CAP_COLOR_ATTACHMENTS; the mrt4 item"),
        ("MAX_DRAW_BUFFERS", "3, not 4: gl/context.rs MAX_DRAW_BUFFERS = CAP_COLOR_ATTACHMENTS; the mrt4 item"),
        (
            "MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS",
            "1536: MAX_UNIFORM_DWORDS, no uniform-block share added (gl/state.rs); MAX_UNIFORM_BLOCK_SIZE is the root",
        ),
        (
            "MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS",
            "1536: MAX_UNIFORM_DWORDS, no uniform-block share added (gl/state.rs)",
        ),
        (
            "MAX_UNIFORM_BLOCK_SIZE",
            "6144, not 16384: a block gets the default-block budget, MAX_UNIFORM_DWORDS * 4 (ubo-uniform)",
        ),
        ("MAX_SAMPLES", "1: no multisample render targets reported (gl/state.rs)"),
        (
            "NUM_COMPRESSED_TEXTURE_FORMATS",
            "3: ETC1, RGB8_ETC2, RGBA8_ETC2_EAC (gl/state.rs); ES 3.0 needs the ten ETC2/EAC formats (formats-*)",
        ),
    ];
    if cfg!(not(feature = "gpu-vivante")) {
        // PowerVR and V3D carry 8 varying vectors (gl/context.rs MAX_VARYING_VECTORS), the ES 2.0 minimum
        v.extend_from_slice(&[
            ("MAX_VARYING_VECTORS", "8, not 15: the ES 2.0 minimum is what this back end programs (gl/context.rs)"),
            ("MAX_VARYING_COMPONENTS", "32 = 8 vectors * 4"),
            ("MAX_FRAGMENT_INPUT_COMPONENTS", "32 = 8 vectors * 4, ES 3.0 needs 60 (gl/context.rs)"),
        ]);
    }
    // the GC7000 reports 15 varyings but 60 vertex output components
    v.push((
        "MAX_VERTEX_OUTPUT_COMPONENTS",
        if cfg!(feature = "gpu-vivante") { "60 = 15 * 4, ES 3.0 needs 64" } else { "32 = 8 * 4" },
    ));
    v
}

#[cfg(not(feature = "report-es2"))]
#[test]
fn every_es3_limit_meets_the_spec_minimum_or_is_a_listed_gap() {
    let _lib = context();
    let gaps = known_gaps();
    let mut report: Vec<String> = vec![];
    let mut failures: Vec<String> = vec![];
    let mut check = |name: &str, got: f64, spec: f64, rule: Rule| {
        let ok = match rule {
            Rule::AtLeast => got >= spec,
            Rule::AtMost => got <= spec,
        };
        let listed = gaps.iter().find(|(n, _)| *n == name);
        let sign = if matches!(rule, Rule::AtLeast) { ">=" } else { "<=" };
        match (ok, listed) {
            (true, None) => report.push(format!("ok      {name} = {got} ({sign} {spec})")),
            (true, Some(_)) => {
                report.push(format!("CLOSED  {name} = {got} ({sign} {spec}): remove it from known_gaps()"))
            }
            (false, Some((_, why))) => report.push(format!("GAP     {name} = {got}, spec {sign} {spec}: {why}")),
            (false, None) => failures.push(format!("{name} = {got}, the spec needs {sign} {spec}")),
        }
    };
    /* the version the limits are claimed for */
    assert_eq!(geti(GL_MAJOR_VERSION), 3);
    assert_eq!(geti(GL_MINOR_VERSION), 0);
    for l in limits() {
        check(l.name, geti(l.pname) as f64, l.spec as f64, l.rule);
    }
    check("MAX_TEXTURE_LOD_BIAS", getf_n(GL_MAX_TEXTURE_LOD_BIAS)[0] as f64, 2.0, Rule::AtLeast);
    check("MAX_SERVER_WAIT_TIMEOUT", geti64(GL_MAX_SERVER_WAIT_TIMEOUT) as f64, 0.0, Rule::AtLeast);
    /* the 32-bit and 64-bit answers of a limit agree */
    for l in limits() {
        assert_eq!(geti64(l.pname), geti(l.pname) as i64, "{}: glGetInteger64v disagrees", l.name);
    }
    /* ranges (Table 6.32: line width and point size ranges, both ends 1 at least) */
    let [w0, w1] = getf_n(GL_ALIASED_LINE_WIDTH_RANGE);
    check("ALIASED_LINE_WIDTH_RANGE[0]", w0 as f64, 1.0, Rule::AtMost);
    check("ALIASED_LINE_WIDTH_RANGE[1]", w1 as f64, 1.0, Rule::AtLeast);
    let [p0, p1] = getf_n(GL_ALIASED_POINT_SIZE_RANGE);
    check("ALIASED_POINT_SIZE_RANGE[0]", p0 as f64, 1.0, Rule::AtMost);
    check("ALIASED_POINT_SIZE_RANGE[1]", p1 as f64, 1.0, Rule::AtLeast);
    /* MAX_VIEWPORT_DIMS has no minimum in the table; the library's own rule
     * is that a framebuffer of the largest texture can be drawn whole. */
    let vp = geti_n(GL_MAX_VIEWPORT_DIMS);
    report.push(format!("note    MAX_VIEWPORT_DIMS = {}x{} (no spec minimum)", vp[0], vp[1]));
    /* the derived minima against the block limits they are made of
     * (Table 6.32: blocks * block size / 4 + default-block components) */
    for (stage, combined, blocks, comps) in [
        (
            "FRAGMENT",
            GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
            GL_MAX_FRAGMENT_UNIFORM_BLOCKS,
            GL_MAX_FRAGMENT_UNIFORM_COMPONENTS,
        ),
        (
            "VERTEX",
            GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS,
            GL_MAX_VERTEX_UNIFORM_BLOCKS,
            GL_MAX_VERTEX_UNIFORM_COMPONENTS,
        ),
    ] {
        let want = geti(blocks) as i64 * geti(GL_MAX_UNIFORM_BLOCK_SIZE) as i64 / 4 + geti(comps) as i64;
        report.push(format!(
            "note    MAX_COMBINED_{stage}_UNIFORM_COMPONENTS: {} vs {want} from the block limits",
            geti(combined)
        ));
    }
    for line in &report {
        println!("{line}");
    }
    /* SEPARATE_COMPONENTS is the interleaved limit, not ES 3.0's 4: the
     * capture runs on the CPU (gl/tf.rs TF_MAX_SEPARATE_COMPONENTS) */
    assert_eq!(
        geti(GL_MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS),
        geti(GL_MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS)
    );
    assert!(
        failures.is_empty(),
        "limits below the ES 3.0 minimum and not in known_gaps():\n  {}",
        failures.join("\n  ")
    );
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
}

/* ---- the hardware ceilings ------------------------------------------------ */

/// What a build reports must not exceed what the hardware does. Each ceiling
/// is a statement checked in docs/es3-backend-caps.md section 1 at the
/// file:line given here, all in Mesa 26.2.1 (da14d65e): v3d_limits.h, the
/// Imagination device-info / csbgen files, and the etnaviv ones (E/ =
/// src/gallium/drivers/etnaviv/, H/ = the i.MX8MP hwdb row).
#[cfg(not(feature = "report-es2"))]
fn ceilings() -> Vec<(&'static str, GLenum, i64, &'static str)> {
    if cfg!(feature = "gpu-v3d") {
        vec![
            // v3d_limits.h:44, V3D_MAX_RENDER_TARGETS(ver < 71): 4 on V3D 4.2
            ("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS, 4, "v3d_limits.h:44"),
            ("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS, 4, "v3d_limits.h:44"),
            // v3d_limits.h:41
            ("MAX_SAMPLES", GL_MAX_SAMPLES, 4, "v3d_limits.h:41"),
            // v3d_limits.h:62 image size; :63 framebuffer size, ver < 71
            ("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, 4096, "v3d_limits.h:62"),
            ("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE, 4096, "v3d_limits.h:63"),
            // v3d_limits.h:77
            ("MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS, 2048, "v3d_limits.h:77"),
            // v3d_limits.h:32, V3D_MAX_FS_INPUTS 64: a vertex output is a
            // fragment input, so the FS input limit bounds both
            ("MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS, 64, "v3d_limits.h:32"),
            ("MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS, 64, "v3d_limits.h:32"),
            ("MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS, 64, "v3d_limits.h:32"),
            // v3d_limits.h:39, 24 samplers a stage
            ("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS, 24, "v3d_limits.h:39"),
            ("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, 24, "v3d_limits.h:39"),
        ]
    } else if cfg!(feature = "gpu-powervr") {
        vec![
            // bxs-4-64.h:92 max_multisample
            ("MAX_SAMPLES", GL_MAX_SAMPLES, 4, "bxs-4-64.h:92"),
            // pvr_limits.h:35 + rogue_hw_defs.h:154: eight PBE emit registers
            ("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS, 8, "pvr_limits.h:35, rogue_hw_defs.h:154"),
            // texstate.xml:276: IMAGE_WORD0 width MAX_SIZE 16383 (14 bits)
            ("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, 16384, "texstate.xml:276"),
            // texstate.xml:310: IMAGE_WORD1 depth MAX_SIZE 2047 (11 bits)
            ("MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS, 2048, "texstate.xml:310"),
            ("MAX_3D_TEXTURE_SIZE", GL_MAX_3D_TEXTURE_SIZE, 2048, "texstate.xml:310"),
        ]
    } else {
        vec![
            // E/etnaviv_screen.c:299-304: num_rts / 2 with HALTI2; num_rts 8 for halti >= 2 (:774-782)
            ("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS, 4, "E/etnaviv_screen.c:299-304"),
            ("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS, 4, "E/etnaviv_screen.c:299-304"),
            // E/etnaviv_screen.c:451-468 gpu_supports_msaa (MSAA H:18958, SmallMSAA
            // H:19129 refuses 2x) with E/etnaviv_translate.h:501-517 (1, 2, 4 only)
            ("MAX_SAMPLES", GL_MAX_SAMPLES, 4, "E/etnaviv_screen.c:451-468"),
            // E/etnaviv_screen.c:970-973 with Texture8K / Render8K set (H:18986,18992)
            ("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, 8192, "E/etnaviv_screen.c:970-971"),
            ("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE, 8192, "E/etnaviv_screen.c:972-973"),
            // E/etnaviv_screen.c:965-968 MIN3(16, hwdb VaryingCount 16 (H:18876), vs outputs - 1)
            ("MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS, 16, "E/etnaviv_screen.c:965-968"),
            // E/etnaviv_screen.c:824-827, halti >= 1
            ("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS, 16, "E/etnaviv_screen.c:826"),
            ("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, 16, "E/etnaviv_screen.c:827"),
        ]
    }
}

#[cfg(not(feature = "report-es2"))]
#[test]
fn reported_limits_do_not_exceed_the_hardware_ceilings() {
    let _lib = context();
    let mut over = vec![];
    for (name, pname, ceiling, cite) in ceilings() {
        let got = geti(pname) as i64;
        println!("ceiling {GPU}: {name} = {got} <= {ceiling} ({cite})");
        if got > ceiling {
            over.push(format!("{name} = {got} exceeds the hardware's {ceiling} ({cite})"));
        }
    }
    /* the point-size and line-width ranges have a hardware ceiling too */
    let [_, line_max] = getf_n(GL_ALIASED_LINE_WIDTH_RANGE);
    let [_, point_max] = getf_n(GL_ALIASED_POINT_SIZE_RANGE);
    if cfg!(feature = "gpu-v3d") {
        // v3d_limits.h:46-47
        if point_max > 512.0 || line_max > 32.0 {
            over.push(format!("V3D point {point_max} / line {line_max} exceed 512 / 32 (v3d_limits.h:46-47)"));
        }
    } else if cfg!(feature = "gpu-powervr") {
        // pvr_limits.h:44,48 (Mesa's Vulkan figures for the same silicon)
        if point_max > 511.0 || line_max > 16.0 {
            over.push(format!("PowerVR point {point_max} / line {line_max} exceed 511 / 16 (pvr_limits.h:44,48)"));
        }
    }
    assert!(over.is_empty(), "limits above the hardware:\n  {}", over.join("\n  "));
}

/* ---- ES 2.0 build: the answers that must not move -------------------------- */

#[cfg(feature = "report-es2")]
#[test]
fn es2_report_answers_are_unchanged() {
    let _lib = context();
    // SAFETY: plain GL calls.
    unsafe { while glGetError() != GL_NO_ERROR {} }
    /* the queries that give ES 3 away are refused (gl/state.rs get_integerv) */
    for (name, p) in [
        ("MAJOR_VERSION", GL_MAJOR_VERSION),
        ("MINOR_VERSION", GL_MINOR_VERSION),
        ("NUM_EXTENSIONS", GL_NUM_EXTENSIONS),
        ("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS),
    ] {
        assert_eq!(error_of(p), GL_INVALID_ENUM, "{name} must be GL_INVALID_ENUM in an ES 2.0 report");
    }
    /* ES 2.0's own limits (Table 6.18 of ES 2.0.25), per GPU where they differ */
    let varyings = if cfg!(feature = "gpu-vivante") { 15 } else { 8 }; // gl/context.rs MAX_VARYING_VECTORS
    let point_max = if cfg!(feature = "gpu-v3d") {
        512.0
    } else if cfg!(feature = "gpu-vivante") {
        64.0
    } else {
        511.0
    }; // gl/state.rs POINT_SIZE_MAX
    for (name, p, want) in [
        ("MAX_VERTEX_ATTRIBS", GL_MAX_VERTEX_ATTRIBS, 16),
        ("MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS, 384),
        ("MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS, 384),
        ("MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS, varyings),
        ("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS, 16),
        ("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, 16),
        ("MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, 32),
        ("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, 4096),
        ("MAX_CUBE_MAP_TEXTURE_SIZE", GL_MAX_CUBE_MAP_TEXTURE_SIZE, 4096),
        ("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE, 4096),
        ("NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS, 3),
    ] {
        assert_eq!(geti(p), want, "{name} (ES 2.0 report, {GPU})");
    }
    /* every other limit of the ES 3.0 table is ES 3.0-only and refused
     * (gl/state.rs es3_only_pname), except MAX_COLOR_ATTACHMENTS: its token is
     * GL_MAX_COLOR_ATTACHMENTS_NV of the claimed GL_NV_fbo_color_attachments */
    let es2_pnames = [
        GL_MAX_VERTEX_ATTRIBS,
        GL_MAX_VERTEX_UNIFORM_VECTORS,
        GL_MAX_FRAGMENT_UNIFORM_VECTORS,
        GL_MAX_VARYING_VECTORS,
        GL_MAX_TEXTURE_IMAGE_UNITS,
        GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS,
        GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS,
        GL_MAX_TEXTURE_SIZE,
        GL_MAX_CUBE_MAP_TEXTURE_SIZE,
        GL_MAX_RENDERBUFFER_SIZE,
        GL_NUM_COMPRESSED_TEXTURE_FORMATS,
        GL_MAX_COLOR_ATTACHMENTS,
    ];
    for l in limits() {
        if !es2_pnames.contains(&l.pname) {
            let name = l.name;
            assert_eq!(error_of(l.pname), GL_INVALID_ENUM, "{name} must be GL_INVALID_ENUM in an ES 2.0 report");
        }
    }
    for (name, p) in
        [("MAX_TEXTURE_LOD_BIAS", GL_MAX_TEXTURE_LOD_BIAS), ("MAX_SERVER_WAIT_TIMEOUT", GL_MAX_SERVER_WAIT_TIMEOUT)]
    {
        assert_eq!(error_of(p), GL_INVALID_ENUM, "{name} must be GL_INVALID_ENUM in an ES 2.0 report");
    }
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 3, "MAX_COLOR_ATTACHMENTS(_NV)");
    assert_eq!(geti_n(GL_MAX_VIEWPORT_DIMS), [4096, 4096], "MAX_VIEWPORT_DIMS");
    assert_eq!(getf_n(GL_ALIASED_LINE_WIDTH_RANGE), [1.0, 16.0], "ALIASED_LINE_WIDTH_RANGE");
    assert_eq!(getf_n(GL_ALIASED_POINT_SIZE_RANGE), [1.0, point_max], "ALIASED_POINT_SIZE_RANGE");
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
}

/* ---- the formats: what this build says today -------------------------------- */

/// The sampled texel classes and integer render-target bits the build
/// reports (informational: the class tables are the formats-* and int-rt-*
/// items' to grow, and docs/es3-backend-caps.md section 4 holds the
/// hardware side). Run with --nocapture.
#[cfg(not(feature = "report-es2"))]
#[test]
fn print_the_format_support_this_build_reports() {
    use yakogl::backend::*;
    let sampled = sampled_texfmts();
    let names = [
        "RGBA8",
        "ETC2_RGB",
        "ETC2_RGBA",
        "ETC2_RGB_PT",
        "RGB565",
        "RGBA16F",
        "R16F",
        "R32F",
        "L8",
        "A8",
        "LA8",
        "R11G11B10F",
        "RGB9E5",
        "RGB10A2",
        "RGB10A2UI",
        "RGBA8_SNORM",
        "RGBA8UI",
        "RGBA8I",
        "RGBA16UI",
        "RGBA16I",
        "RGBA32UI",
        "RGBA32I",
        "RGBA32F",
        "SRGB8_A8",
    ];
    assert_eq!(names.len() as u8, GLES_TEXFMT_SRGB8_A8 + 1);
    for (i, n) in names.iter().enumerate() {
        println!("{GPU}: texel class {n:<12} sampled: {}", (sampled >> i) & 1 != 0);
    }
    // the classes every GPU samples today: RGBA8 is the base of everything
    assert_ne!(sampled & gles_texfmt_bit(GLES_TEXFMT_RGBA8), 0);
    for f in GLES_SURF_INT_FIRST..=GLES_SURF_INT_LAST {
        println!("{GPU}: integer render target {:<10} drawable: {}", gles_surf_name(f), rt_format_drawable(f));
    }
    // the mask only names surfaces that exist
    assert_eq!(INTEGER_RT_FORMATS >> (GLES_SURF_INT_LAST - GLES_SURF_INT_FIRST + 1), 0);
}
