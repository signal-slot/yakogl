// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Every ES 3.0 implementation-dependent limit, queried through the GL API
//! on the host and held against the minimum of the ES 3.0.6 specification
//! (Tables 6.32, 6.33 and 6.34). The numbers are the ones each GPU's build
//! reports (`gpu-powervr`, `gpu-vivante`, `gpu-v3d`: tools/check.sh runs
//! this file in all of them); docs/es3-backend-caps.md compares them with
//! what the hardware can do (Mesa v3d / etnaviv / pvr).
//!
//! A limit below its minimum is not a silent pass: it is listed in
//! [`known_gaps`] by name, with the reason, and the test prints one line per
//! limit. A limit that is NOT listed must meet its minimum. A listed limit
//! that now meets it is reported as CLOSED ("remove it from known_gaps")
//! without failing, so the item that closes a gap does not have to touch
//! this file in the same change.
//!
//! An ES 2.0-reporting build (`report-es2`) answers none of these queries
//! (they are GL_INVALID_ENUM there), so the file is empty in that build.

#![cfg(all(not(feature = "report-es2"), feature = "host-test"))]

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
};

fn context() {
    // SAFETY: both tables are statics, and the one test in this file is the
    // only thread that touches the library.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
    }
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
        lim("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS, 50_048),
        lim("MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, 32),
        lim("MAX_COMBINED_UNIFORM_BLOCKS", GL_MAX_COMBINED_UNIFORM_BLOCKS, 24),
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
        ("MAX_COLOR_ATTACHMENTS", "3, not 4: CAP_COLOR_ATTACHMENTS (gl/context.rs); the mrt4 item"),
        ("MAX_DRAW_BUFFERS", "3, not 4: CAP_COLOR_ATTACHMENTS (gl/context.rs); the mrt4 item"),
        (
            "MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS",
            "1536: no uniform-block share added (gl/state.rs); MAX_UNIFORM_BLOCK_SIZE is the root",
        ),
        ("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", "1536: no uniform-block share added (gl/state.rs)"),
        (
            "MAX_UNIFORM_BLOCK_SIZE",
            "6144, not 16384: a block is bounded by the default-block budget (ubo-uniform item)",
        ),
        ("MAX_SAMPLES", "1: no multisample render targets reported (docs/es3-backend-caps.md, MSAA)"),
        ("MAX_TRANSFORM_FEEDBACK_INTERLEAVED_COMPONENTS", "0: transform feedback is not implemented (tf-* items)"),
        ("MAX_TRANSFORM_FEEDBACK_SEPARATE_ATTRIBS", "0: transform feedback is not implemented (tf-* items)"),
        ("MAX_TRANSFORM_FEEDBACK_SEPARATE_COMPONENTS", "0: transform feedback is not implemented (tf-* items)"),
        ("NUM_COMPRESSED_TEXTURE_FORMATS", "3: ES 3.0 requires the ten ETC2/EAC formats (formats-* items)"),
    ];
    if cfg!(not(feature = "gpu-vivante")) {
        // PowerVR and V3D carry 8 varying vectors (context.rs MAX_VARYING_VECTORS), the ES 2.0 minimum
        v.extend_from_slice(&[
            ("MAX_VARYING_VECTORS", "8, not 15: the ES 2.0 minimum is what this back end programs"),
            ("MAX_VARYING_COMPONENTS", "32 = 8 vectors * 4"),
            ("MAX_FRAGMENT_INPUT_COMPONENTS", "32 = 8 vectors * 4, ES 3.0 needs 60"),
        ]);
    }
    // the GC7000 reports 15 varyings but 60 vertex output components
    v.push((
        "MAX_VERTEX_OUTPUT_COMPONENTS",
        if cfg!(feature = "gpu-vivante") { "60 = 15 * 4, ES 3.0 needs 64" } else { "32 = 8 * 4" },
    ));

    v
}

#[test]
fn every_es3_limit_meets_the_spec_minimum_or_is_a_listed_gap() {
    context();
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
    for l in limits() {
        check(l.name, geti(l.pname) as f64, l.spec as f64, l.rule);
    }
    check("MAX_TEXTURE_LOD_BIAS", getf_n(GL_MAX_TEXTURE_LOD_BIAS)[0] as f64, 2.0, Rule::AtLeast);
    check("MAX_SERVER_WAIT_TIMEOUT", geti64(GL_MAX_SERVER_WAIT_TIMEOUT) as f64, 0.0, Rule::AtLeast);
    /* the 32-bit and 64-bit answers of a limit agree */
    for l in limits() {
        if l.pname != GL_NUM_COMPRESSED_TEXTURE_FORMATS {
            assert_eq!(geti64(l.pname), geti(l.pname) as i64, "{}: glGetInteger64v disagrees", l.name);
        }
    }
    /* ranges */
    let [w0, w1] = getf_n(GL_ALIASED_LINE_WIDTH_RANGE);
    check("ALIASED_LINE_WIDTH_RANGE[0]", w0 as f64, 1.0, Rule::AtMost);
    check("ALIASED_LINE_WIDTH_RANGE[1]", w1 as f64, 1.0, Rule::AtLeast);
    let [p0, p1] = getf_n(GL_ALIASED_POINT_SIZE_RANGE);
    check("ALIASED_POINT_SIZE_RANGE[0]", p0 as f64, 1.0, Rule::AtMost);
    check("ALIASED_POINT_SIZE_RANGE[1]", p1 as f64, 1.0, Rule::AtLeast);
    let vp = geti_n(GL_MAX_VIEWPORT_DIMS);
    check("MAX_VIEWPORT_DIMS[0]", vp[0] as f64, geti(GL_MAX_TEXTURE_SIZE) as f64, Rule::AtLeast);
    check("MAX_VIEWPORT_DIMS[1]", vp[1] as f64, geti(GL_MAX_TEXTURE_SIZE) as f64, Rule::AtLeast);
    /* the derived minima must agree with the block limits they are made of
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
        let name = format!("MAX_COMBINED_{stage}_UNIFORM_COMPONENTS vs blocks");
        let got = geti(combined) as f64;
        // not a spec row of its own: reported, never failed
        report.push(format!("note    {name}: {got} vs {want} from the block limits"));
    }
    for line in &report {
        println!("{line}");
    }
    assert!(
        failures.is_empty(),
        "limits below the ES 3.0 minimum and not in known_gaps():\n  {}",
        failures.join("\n  ")
    );
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
}
