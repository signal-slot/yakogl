// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The G3 rows on the V3D build: `implementation_limits.*` (11) and
//! `state_query.*` (35) of `docs/es3/pi4-gles3-fails.tsv`, as section G3 of
//! `docs/notes/v3d-notpass-rest.md` groups them, pinned from the host on the
//! `gpu-v3d` build.
//!
//! Each dEQP row is a (limit, query form) pair: the `implementation_limits`
//! group queries each limit with `glGetIntegerv` and checks the value against
//! the ES 3.0 minimum (spec tables 6.29-6.33; `tests/es3_caps.rs` holds the
//! spec table itself — this file is the row-by-row map and the query-form
//! half, and does not restate the table); the `state_query.integers` /
//! `.integers64` groups ask the *same* pname through all three integer entry
//! points (`glGetIntegerv`, `glGetFloatv`, `glGetInteger64v`) and expect the
//! same number out of each; the `state_query.indexed` group is the
//! extension-list question (`GL_OES_draw_buffers_indexed` unclaimed) plus the
//! indexed queries it would enable.
//!
//! What this pins on the host today:
//! - the three integer query forms agree for every G3 pname, at no error —
//!   `glGetFloatv` and `glGetInteger64v` fall through to the `glGetIntegerv`
//!   answer and convert it (`src/gl/state.rs` `get_floatv`/`get_integerv`);
//!   the board rows that failed on one form alone (`*_getfloat`,
//!   `*_getinteger64`) were the old non-converting path, and re-run against
//!   main they should pass (the tsv is from the 2026-10-31 build);
//! - the values below the ES 3.0 minimum are pinned as they are, each with
//!   its owner in the comment, and the ES 3.0 result is the `#[ignore]`d
//!   test of it (`cargo test ... -- --ignored` runs the expectations);
//! - `GL_MAX_VARYING_COMPONENTS`-style derived rows pin their derivation
//!   (4 components per varying vector) so raising the root moves them.
//!
//! The 9 `state_query.indexed` rows and the compressed-format list rows are
//! pinned in their own tests; the three remaining G3 state rows
//! (`rbo.renderbuffer_component_size_color`,
//! `shader.program_active_uniform_types`, `shader.program_binary`) are not
//! limits and live with their groups (G7 for `program_binary`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_limits_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::Once;
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-limits-v3d".as_ptr() as *const c_char,
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
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
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

/* ---- queries, each clearing the error first ------------------------------ */

/// Up to `STATE_VALUES`-many values; G3 list pnames write at most
/// `COMPRESSED_FORMATS.len()` + the four-value pnames.
struct Q;

impl Q {
    /// The error of one command, cleared before and read after.
    fn error_clear() {
        // SAFETY: plain GL call.
        unsafe { while glGetError() != GL_NO_ERROR {} }
    }
    fn error() -> GLenum {
        // SAFETY: plain GL call.
        unsafe { glGetError() }
    }
    /// The value(s) glGetIntegerv answers for pname, cleared first.
    fn i(pname: GLenum) -> Vec<i64> {
        Self::error_clear();
        let mut v = [i32::MIN; 16];
        // SAFETY: room for the widest pname (the compressed-format list: eleven).
        unsafe { glGetIntegerv(pname, v.as_mut_ptr()) };
        assert_ne!(v[0], i32::MIN, "glGetIntegerv(0x{pname:x}) wrote nothing");
        v.iter().take_while(|x| **x != i32::MIN).map(|&x| i64::from(x)).collect()
    }
    /// The value(s) glGetFloatv answers; slots the pname does not write
    /// keep the NaN sentinel, so the count is observable.
    fn f(pname: GLenum) -> Vec<f64> {
        Self::error_clear();
        let mut f = [f32::NAN; 16];
        // SAFETY: the widest pname writes eleven values (the format list);
        // room for eight.
        unsafe { glGetFloatv(pname, f.as_mut_ptr()) };
        let out: Vec<f64> = f.iter().map(|&x| f64::from(x)).take_while(|x| !x.is_nan()).collect();
        assert!(!out.is_empty(), "glGetFloatv(0x{pname:x}) wrote nothing");
        out
    }
    /// The value(s) glGetInteger64v answers.
    fn i64(pname: GLenum) -> Vec<i64> {
        Self::error_clear();
        let mut v = [i64::MIN; 16];
        // SAFETY: as for glGetIntegerv.
        unsafe { glGetInteger64v(pname, v.as_mut_ptr()) };
        assert_ne!(v[0], i64::MIN, "glGetInteger64v(0x{pname:x}) wrote nothing");
        v.iter().take_while(|x| **x != i64::MIN).copied().collect()
    }
    /// The single value of a one-value pname, via glGetIntegerv.
    fn one(pname: GLenum) -> i64 {
        let v = Self::i(pname);
        assert_eq!(v.len(), 1, "0x{pname:x}: expected one value");
        v[0]
    }
}

fn c_string(pname: GLenum) -> String {
    // SAFETY: glGetString answers a static NUL-terminated string for the
    // string pnames; from_ptr reads it while the library is alive.
    unsafe { std::ffi::CStr::from_ptr(glGetString(pname) as *const c_char).to_string_lossy().into_owned() }
}

/* ---- the G3 pnames that meet the ES 3.0 minimum today --------------------- */

/// One G3-or-nearby limit and its ES 3.0 minimum (spec tables 6.29-6.33:
/// the same table `tests/es3_caps.rs` holds against; here these are the
/// rows whose *value* answers ES 3.0 on the V3D build, so their dEQP rows
/// pass on the value alone and pin that they stay so).
struct Spec {
    name: &'static str,
    pname: GLenum,
    min: i64,
}

fn meets_es3_0() -> Vec<Spec> {
    let s = |name, pname, min| Spec { name, pname, min };
    vec![
        // implementation_limits / Table 6.33: the element index an
        // implementation must take; dEQP's max_element_index case.
        s("MAX_ELEMENT_INDEX", GL_MAX_ELEMENT_INDEX, 16_777_215),
        s("MAX_ELEMENTS_VERTICES", GL_MAX_ELEMENTS_VERTICES, 65_536),
        s("MAX_ELEMENTS_INDICES", GL_MAX_ELEMENTS_INDICES, 65_536),
        // the texture-size family, ES 3.0's minimums
        s("MAX_TEXTURE_SIZE", GL_MAX_TEXTURE_SIZE, 2048),
        s("MAX_CUBE_MAP_TEXTURE_SIZE", GL_MAX_CUBE_MAP_TEXTURE_SIZE, 2048),
        s("MAX_RENDERBUFFER_SIZE", GL_MAX_RENDERBUFFER_SIZE, 2048),
        s("MAX_3D_TEXTURE_SIZE", GL_MAX_3D_TEXTURE_SIZE, 256),
        s("MAX_ARRAY_TEXTURE_LAYERS", GL_MAX_ARRAY_TEXTURE_LAYERS, 256),
        // units and vertex attributes
        s("MAX_VERTEX_ATTRIBS", GL_MAX_VERTEX_ATTRIBS, 16),
        s("MAX_TEXTURE_IMAGE_UNITS", GL_MAX_TEXTURE_IMAGE_UNITS, 16),
        s("MAX_VERTEX_TEXTURE_IMAGE_UNITS", GL_MAX_VERTEX_TEXTURE_IMAGE_UNITS, 16),
        s("MAX_COMBINED_TEXTURE_IMAGE_UNITS", GL_MAX_COMBINED_TEXTURE_IMAGE_UNITS, 32),
        // the uniform vectors and block counts (the *components and block
        // size live in below_es3_0(); the vectors and counts reach it)
        s("MAX_VERTEX_UNIFORM_VECTORS", GL_MAX_VERTEX_UNIFORM_VECTORS, 256),
        s("MAX_FRAGMENT_UNIFORM_VECTORS", GL_MAX_FRAGMENT_UNIFORM_VECTORS, 224),
        s("MAX_VERTEX_UNIFORM_BLOCKS", GL_MAX_VERTEX_UNIFORM_BLOCKS, 12),
        s("MAX_FRAGMENT_UNIFORM_BLOCKS", GL_MAX_FRAGMENT_UNIFORM_BLOCKS, 12),
        s("MAX_COMBINED_UNIFORM_BLOCKS", GL_MAX_COMBINED_UNIFORM_BLOCKS, 24),
        s("MAX_UNIFORM_BUFFER_BINDINGS", GL_MAX_UNIFORM_BUFFER_BINDINGS, 24),
        s("MAX_PROGRAM_TEXEL_OFFSET", GL_MAX_PROGRAM_TEXEL_OFFSET, 7),
    ]
}

/* ---- the G3 pnames that are below the ES 3.0 minimum today ---------------- */

/// A limit below its ES 3.0 minimum, pinned as it is, with the dEQP rows it
/// answers and the owner of the fix named. `today` is what this build
/// reports; a row whose value has moved to meet the minimum prints CLOSED
/// and passes (`es3_caps.rs` holds the same rule for its own table).
struct Gap {
    name: &'static str,
    pname: GLenum,
    min: i64,
    today: i64,
    /// The dEQP-GLES3 rows this pins (the G3 group of the note).
    rows: &'static str,
    /// Where the fix lives.
    owner: &'static str,
}

fn below_es3_0() -> Vec<Gap> {
    let g = |name, pname, min, today, rows, owner| Gap { name, pname, min, today, rows, owner };
    vec![
        // Four draw buffers / colour attachments: `src/backend.rs`
        // `DRAW_TARGETS` is 3 for `gpu-v3d` ("the TLB takes targets 0-3 and
        // the compiler writes four, but no board has drawn four");
        // `src/gl/context.rs` `max_draw_buffers`/`max_color_attachments`
        // read it, and every attachment check follows.
        g(
            "MAX_DRAW_BUFFERS",
            GL_MAX_DRAW_BUFFERS,
            4,
            3,
            "implementation_limits.max_draw_buffers + state_query.integers.max_draw_buffers_*",
            "src/backend.rs (DRAW_TARGETS, gpu-v3d 3 -> 4); src/gl/context.rs follows it",
        ),
        g(
            "MAX_COLOR_ATTACHMENTS",
            GL_MAX_COLOR_ATTACHMENTS,
            4,
            3,
            "implementation_limits.max_color_attachments + state_query.integers.max_color_attachments_*",
            "src/backend.rs (DRAW_TARGETS, gpu-v3d 3 -> 4); src/gl/context.rs follows it",
        ),
        // 4x MSAA: `src/gl/state.rs` answers GL_MAX_SAMPLES 1 and
        // `src/gl/framebuffer.rs` refuses samples > 1; the V3D half refuses
        // 4x of several attachments and of an ES 3.0 format
        // (`src/v3d/backend.rs`, design `docs/notes/v3d-msaa-es3.md`).
        // dEQP's 4/8-sample rows are G2; the query-form rows here are G3.
        g(
            "MAX_SAMPLES",
            GL_MAX_SAMPLES,
            4,
            1,
            "state_query.integers.max_samples_getfloat + max_samples_getinteger64 (values; forms below)",
            "src/gl/state.rs + src/gl/framebuffer.rs (GL core caps); 4x design docs/notes/v3d-msaa-es3.md",
        ),
        // The varying family: `src/gl/context.rs` MAX_VARYING_VECTORS is 8
        // on gpu-v3d (the ES 2.0 minimum; Mesa's v3d programs 16 /
        // V3D_MAX_FS_INPUTS 64). The three component rows are 4x the root
        // (`src/gl/state.rs` and `src/gl/context.rs` derive them), so one
        // change moves all four.
        g(
            "MAX_VARYING_VECTORS",
            GL_MAX_VARYING_VECTORS,
            15,
            8,
            "implementation_limits.max_varying_vectors + state_query.integers.max_varying_vectors_*",
            "src/gl/context.rs (MAX_VARYING_VECTORS, gpu-v3d)",
        ),
        g(
            "MAX_VARYING_COMPONENTS",
            GL_MAX_VARYING_COMPONENTS,
            60,
            32,
            "implementation_limits.max_varying_components + state_query.integers.max_varying_components_*",
            "src/gl/context.rs (MAX_VARYING_VECTORS * 4)",
        ),
        g(
            "MAX_VERTEX_OUTPUT_COMPONENTS",
            GL_MAX_VERTEX_OUTPUT_COMPONENTS,
            64,
            32,
            "implementation_limits.max_vertex_output_components + state_query.integers.max_vertex_output_components_*",
            "src/gl/context.rs (MAX_VARYING_VECTORS * 4)",
        ),
        g(
            "MAX_FRAGMENT_INPUT_COMPONENTS",
            GL_MAX_FRAGMENT_INPUT_COMPONENTS,
            60,
            32,
            "implementation_limits.max_fragment_input_components + state_query.integers.max_fragment_input_components_*",
            "src/gl/context.rs (MAX_VARYING_VECTORS * 4)",
        ),
        // The uniform-block family: `src/gl/context.rs` MAX_UNIFORM_DWORDS
        // is 1536 and `src/gl/state.rs` derives the block size
        // (dwords * 4 bytes) and both combined-component limits (the bare
        // dwords — no block share added on top) from it.
        g(
            "MAX_UNIFORM_BLOCK_SIZE",
            GL_MAX_UNIFORM_BLOCK_SIZE,
            16_384,
            6_144,
            "implementation_limits.max_uniform_block_size + state_query.integers64.max_uniform_block_size_*",
            "src/gl/context.rs (MAX_UNIFORM_DWORDS); src/gl/state.rs derives the size",
        ),
        g(
            "MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS",
            GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS,
            50_176, // 12 blocks * 16384 / 4 + 1024 (Table 6.32 note)
            1_536,
            "implementation_limits.max_combined_vertex_uniform_components + state_query.integers64.*_get{float,integer}",
            "src/gl/context.rs (MAX_UNIFORM_DWORDS); src/gl/state.rs: no block share",
        ),
        g(
            "MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS",
            GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
            50_048, // 12 * 16384 / 4 + 896
            1_536,
            "implementation_limits.max_combined_fragment_uniform_components + state_query.integers64.*_get{float,integer}",
            "src/gl/context.rs (MAX_UNIFORM_DWORDS); src/gl/state.rs: no block share",
        ),
    ]
}

/* ---- 1. the limits that meet ES 3.0 --------------------------------------- */

/// dEQP-GLES3 `implementation_limits.max_element_index` and the rest of the
/// G3-adjacent value rows: each answers at least its ES 3.0 minimum.
#[test]
fn limits_that_meet_the_es3_0_minimum_stay_meeting_it() {
    let _lib = context();
    // the version the minimums are claimed for
    assert_eq!((Q::one(GL_MAJOR_VERSION), Q::one(GL_MINOR_VERSION)), (3, 0));
    for s in meets_es3_0() {
        let got = Q::one(s.pname);
        println!("ok      {} = {got} >= {}", s.name, s.min);
        assert!(got >= s.min, "{} = {got}, the ES 3.0 minimum is {}", s.name, s.min);
    }
    // the AtMost ends of the same tables
    assert!(Q::one(GL_UNIFORM_BUFFER_OFFSET_ALIGNMENT) <= 256, "UNIFORM_BUFFER_OFFSET_ALIGNMENT");
    assert!(Q::one(GL_MIN_PROGRAM_TEXEL_OFFSET) <= -8, "MIN_PROGRAM_TEXEL_OFFSET");
    // float-valued: the LOD bias minimum is 2.0, queried as a float
    Q::error_clear();
    assert!(Q::f(GL_MAX_TEXTURE_LOD_BIAS)[0] >= 2.0, "MAX_TEXTURE_LOD_BIAS < 2.0");
    // 64-bit-valued: ES 3.0 asks for at least 0 (a working, if 0-length,
    // wait is legal); the answer must not be negative or an error.
    Q::error_clear();
    assert!(Q::i64(GL_MAX_SERVER_WAIT_TIMEOUT)[0] >= 0, "MAX_SERVER_WAIT_TIMEOUT < 0");
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/* ---- 2. the limits that are below ES 3.0, pinned as they are -------------- */

/// The 10 value gaps G3 names, each pinned to today's answer with its owner;
/// the ES 3.0 expectation is the `#[ignore]`d test at the end of this file
/// (one per family). A pinned value that reaches its minimum prints CLOSED
/// without failing, so the fix does not have to touch this file (the rule
/// `es3_caps.rs` sets).
#[test]
fn g3_limits_below_the_es3_0_minimum_are_pinned_as_they_are() {
    let _lib = context();
    let mut closed = vec![];
    for g in below_es3_0() {
        let got = Q::one(g.pname);
        if got >= g.min {
            closed.push(g.name);
            println!("CLOSED  {} = {got} >= {}: re-pin it in meets_es3_0()", g.name, g.min);
            continue;
        }
        assert_eq!(
            got,
            g.today,
            "{}: pinned {today}, spec needs >= {min}, this build answers {got} — \
             re-pin the table or close the gap ({rows})",
            g.name,
            today = g.today,
            min = g.min,
            rows = g.rows,
        );
        println!("GAP     {} = {got}, ES 3.0 needs >= {} | owner: {} | rows: {}", g.name, g.min, g.owner, g.rows);
    }
    println!("CLOSED families: {closed:?} (re-pin them in meets_es3_0)");
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/// The component rows are exactly four per varying vector, so the family's
/// fix is the one root value (and `tests/es3_caps.rs`'s gl_Max* shader
/// values move with it).
#[test]
fn the_varying_component_rows_are_four_times_their_root() {
    let _lib = context();
    let vectors = Q::one(GL_MAX_VARYING_VECTORS);
    for (name, pname) in [
        ("MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS),
        ("MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS),
        ("MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS),
    ] {
        assert_eq!(Q::one(pname), vectors * 4, "{name} is 4 components per varying vector");
    }
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/* ---- 3. the query-form rows: integers.* and integers64.* ------------------ */

/// The 23 `state_query.integers.*` / `state_query.integers64.*` rows of G3
/// (the 17 + 6 Fail rows, plus the `implementation_limits.max_*` value
/// behind each): dEQP queries one pname through `glGetIntegerv`,
/// `glGetFloatv` and `glGetInteger64v` and expects the same number and no
/// error from all three. This build converts: `glGetFloatv` and
/// `glGetInteger64v` fall through to the `glGetIntegerv` answer
/// (`src/gl/state.rs`), so all 23 are passing tests today. The board rows
/// were the pre-conversion path (the tsv is from 2026-10-31) and are
/// #320's reruns to confirm.
#[test]
fn every_g3_pname_answers_the_same_in_all_three_integer_forms() {
    let _lib = context();
    let pnames: [(&str, GLenum); 15] = [
        ("MAX_COLOR_ATTACHMENTS", GL_MAX_COLOR_ATTACHMENTS),
        ("MAX_DRAW_BUFFERS", GL_MAX_DRAW_BUFFERS),
        ("MAX_FRAGMENT_INPUT_COMPONENTS", GL_MAX_FRAGMENT_INPUT_COMPONENTS),
        ("MAX_SAMPLES", GL_MAX_SAMPLES),
        ("MAX_VARYING_COMPONENTS", GL_MAX_VARYING_COMPONENTS),
        ("MAX_VARYING_VECTORS", GL_MAX_VARYING_VECTORS),
        ("MAX_VERTEX_OUTPUT_COMPONENTS", GL_MAX_VERTEX_OUTPUT_COMPONENTS),
        ("MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE),
        ("MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS),
        ("MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS),
        ("NUM_COMPRESSED_TEXTURE_FORMATS", GL_NUM_COMPRESSED_TEXTURE_FORMATS),
        ("COMPRESSED_TEXTURE_FORMATS", GL_COMPRESSED_TEXTURE_FORMATS),
        ("MAX_ELEMENT_INDEX", GL_MAX_ELEMENT_INDEX),
        // the two ALIASED ranges are float pnames: same rule the other way
        ("ALIASED_LINE_WIDTH_RANGE", GL_ALIASED_LINE_WIDTH_RANGE),
        ("ALIASED_POINT_SIZE_RANGE", GL_ALIASED_POINT_SIZE_RANGE),
    ];
    for (name, pname) in pnames {
        let ints = Q::i(pname);
        let e_int = Q::error();
        let floats = Q::f(pname);
        let e_float = Q::error();
        let wide = Q::i64(pname);
        let e_wide = Q::error();
        assert_eq!(e_int, GL_NO_ERROR, "{name}: glGetIntegerv set an error");
        assert_eq!(e_float, GL_NO_ERROR, "{name}: glGetFloatv set an error");
        assert_eq!(e_wide, GL_NO_ERROR, "{name}: glGetInteger64v set an error");
        assert_eq!(floats.len(), ints.len(), "{name}: glGetFloatv answered a different count");
        assert_eq!(wide.len(), ints.len(), "{name}: glGetInteger64v answered a different count");
        for (i, (f, w)) in floats.iter().zip(wide.iter()).enumerate() {
            let want = ints[i] as f64;
            assert_eq!(f, &want, "{name}[{i}]: glGetFloatv disagrees with glGetIntegerv");
            assert_eq!(*w as f64, want, "{name}[{i}]: glGetInteger64v disagrees with glGetIntegerv");
        }
        println!("ok      {name}: integer/float/64 agree ({ints:?})");
    }
}

/* ---- 4. the ALIASED ranges, as dEQP's floats ------------------------------ */

/// `ALIASED_LINE_WIDTH_RANGE` / `ALIASED_POINT_SIZE_RANGE` are the float
/// rows of the limits table (spec: minimum [1, 1]; Mesa's V3D draws lines to
/// 32 and points to 512, `v3d_limits.h:46-47`, as `es3_caps.rs` cites).
#[test]
fn the_aliased_ranges_are_float_pairs_the_v3d_draws() {
    let _lib = context();
    let line = Q::f(GL_ALIASED_LINE_WIDTH_RANGE);
    assert_eq!(Q::error(), GL_NO_ERROR, "ALIASED_LINE_WIDTH_RANGE queried as float");
    let point = Q::f(GL_ALIASED_POINT_SIZE_RANGE);
    assert_eq!(Q::error(), GL_NO_ERROR, "ALIASED_POINT_SIZE_RANGE queried as float");
    assert_eq!(line.len(), 2);
    assert_eq!(point.len(), 2);
    assert_eq!(line[0], 1.0, "ALIASED_LINE_WIDTH_RANGE[0]");
    assert!(line[1] >= 1.0, "ALIASED_LINE_WIDTH_RANGE[1] = {} < 1", point[1]);
    assert_eq!(point[0], 1.0, "ALIASED_POINT_SIZE_RANGE[0]");
    assert!(point[1] >= 1.0, "ALIASED_POINT_SIZE_RANGE[1] = {} < 1", point[1]);
    assert!(line[1] <= 32.0 && point[1] <= 512.0, "above the V3D hardware (v3d_limits.h:46-47)");
    // the integer form answers the rounded pair (the conversion of test 3)
    assert_eq!(Q::i(GL_ALIASED_LINE_WIDTH_RANGE), vec![1, line[1] as i64]);
    assert_eq!(Q::i(GL_ALIASED_POINT_SIZE_RANGE), vec![1, point[1] as i64]);
}

/* ---- 5. the compressed-format list rows ----------------------------------- */

/// `implementation_limits.compressed_texture_formats` and
/// `num_compressed_texture_formats`, and the three
/// `state_query.integers.compressed_texture_formats_*` value rows: the V3D
/// build stores and reports eleven formats, ETC1 and ES 3.0's ten ETC2/EAC
/// (`src/gl/texture.rs` `stored_compressed_formats`): the four the TMU
/// samples as blocks (`V3D_SAMPLED_TEXFMTS` has the ETC2 classes since plan
/// #354) and the six the GL core decodes at upload (`decoded_format`,
/// docs/notes/etc2-eac.md). The list is pinned exactly, in the order the
/// getter writes it. Storage and listing, not silicon: no board has sampled
/// one (docs/es3-cts-failures.md RC10).
#[test]
fn the_compressed_format_list_is_the_eleven_stored_formats() {
    let _lib = context();
    let n = Q::one(GL_NUM_COMPRESSED_TEXTURE_FORMATS);
    let list = Q::i(GL_COMPRESSED_TEXTURE_FORMATS);
    assert_eq!(n as usize, list.len(), "NUM and the list length disagree");
    assert_eq!(Q::error(), GL_NO_ERROR);
    let mut want = list.clone();
    want.sort_unstable();
    want.dedup();
    assert_eq!(want.len(), list.len(), "a format listed twice");
    assert_eq!(
        list,
        [
            GL_ETC1_RGB8_OES,
            GL_COMPRESSED_RGB8_ETC2,
            GL_COMPRESSED_RGBA8_ETC2_EAC,
            GL_COMPRESSED_SRGB8_ETC2,
            GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
            GL_COMPRESSED_R11_EAC,
            GL_COMPRESSED_SIGNED_R11_EAC,
            GL_COMPRESSED_RG11_EAC,
            GL_COMPRESSED_SIGNED_RG11_EAC,
            GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
            GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        ]
        .map(i64::from)
        .to_vec(),
        "the stored compressed list moved; re-pin (owner: src/gl/texture.rs)"
    );
}

/* ---- 6. the extension list and the 9 indexed rows ------------------------- */

/// The extension list through all three queries (GL_EXTENSIONS,
/// GL_NUM_EXTENSIONS, glGetStringi): one list, consistent, and this build is
/// the V3D's ES 3.0 one. dEQP reads all three before its
/// `state_query.indexed` group.
#[test]
fn the_extension_list_is_one_consistent_list_across_the_three_queries() {
    let _lib = context();
    Q::error_clear();
    let ext = c_string(GL_EXTENSIONS);
    assert_eq!(Q::error(), GL_NO_ERROR, "GL_EXTENSIONS");
    let n = Q::one(GL_NUM_EXTENSIONS);
    assert!(n >= 0);
    let words: Vec<&str> = ext.split(' ').filter(|w| !w.is_empty()).collect();
    assert_eq!(words.len() as i64, n, "GL_EXTENSIONS words vs GL_NUM_EXTENSIONS");
    for (i, w) in words.iter().enumerate() {
        // SAFETY: glGetStringi answers a NUL-terminated static name, or null
        // past the list.
        let one = unsafe {
            assert!(!glGetStringi(GL_EXTENSIONS, i as GLuint).is_null(), "glGetStringi({i}) null");
            std::ffi::CStr::from_ptr(glGetStringi(GL_EXTENSIONS, i as GLuint) as *const c_char)
                .to_string_lossy()
                .into_owned()
        };
        assert_eq!(&one, w, "glGetStringi({i}) vs the GL_EXTENSIONS string");
    }
    assert_eq!(Q::error(), GL_NO_ERROR, "glGetStringi in range set an error");
    // past the end: null and GL_INVALID_VALUE (dEQP probes this)
    Q::error_clear();
    // SAFETY: the answer is read only when non-null, and it must be null.
    unsafe {
        assert!(glGetStringi(GL_EXTENSIONS, n as GLuint).is_null());
    }
    assert_eq!(Q::error(), GL_INVALID_VALUE, "glGetStringi past the list");
    // the identity strings of this build
    assert!(c_string(GL_RENDERER).contains("V3D"), "GL_RENDERER: {}", c_string(GL_RENDERER));
    assert_eq!(c_string(GL_VERSION), "OpenGL ES 3.0 YakoGL");
}

/// The 9 `state_query.indexed.*` rows: dEQP answers NotSupported because
/// `GL_OES_draw_buffers_indexed` is not advertised (the same cause as the
/// `draw_buffers_indexed.*` group, `docs/notes/v3d-notpass-rest.md` G3),
/// and the indexed blend/colour-mask queries the extension would enable are
/// `GL_INVALID_ENUM` here (`indexed_state` in `src/gl/state.rs` knows only
/// the uniform-buffer and transform-feedback points). Pinned as it is; the
/// expectation is `#[ignore]`d below.
#[test]
fn draw_buffers_indexed_is_not_advertised_and_its_indexed_state_is_invalid_enum() {
    let _lib = context();
    Q::error_clear();
    let ext = c_string(GL_EXTENSIONS);
    let mut n = Q::one(GL_NUM_EXTENSIONS);
    let mut advertised = false;
    while n > 0 {
        n -= 1;
        // SAFETY: in-range, NUL-terminated static.
        let name = unsafe {
            std::ffi::CStr::from_ptr(glGetStringi(GL_EXTENSIONS, n as GLuint) as *const c_char)
                .to_string_lossy()
                .into_owned()
        };
        advertised |= name == "GL_OES_draw_buffers_indexed";
    }
    assert!(!ext.contains("draw_buffers_indexed") && !advertised, "the extension appeared: re-pin");

    // The indexed form of the blend/colour-mask state the extension adds
    // (targets of dEQP's blend_func_getintegeri_v, blend_equationi,
    // blend_equation_advanced_getintegeri_v, color_mask_getintegeri_v,
    // blend_isenabledi rows): not indexed state here, so GL_INVALID_ENUM —
    // and nothing written (the buffer keeps its sentinel).
    for (what, target) in [
        ("GL_BLEND_SRC_RGB", GL_BLEND_SRC_RGB),
        ("GL_BLEND_EQUATION_RGB", GL_BLEND_EQUATION_RGB),
        ("GL_COLOR_WRITEMASK", GL_COLOR_WRITEMASK),
    ] {
        Q::error_clear();
        let mut v = i32::MIN;
        // SAFETY: one value of room, or none on the error path.
        unsafe {
            glGetIntegeri_v(target, 0, &mut v);
        }
        assert_eq!(Q::error(), GL_INVALID_ENUM, "glGetIntegeri_v({what}, 0)");
        assert_eq!(v, i32::MIN, "glGetIntegeri_v({what}, 0) wrote despite the error");
        let mut v64 = i64::MIN;
        Q::error_clear();
        // SAFETY: as above.
        unsafe {
            glGetInteger64i_v(target, 0, &mut v64);
        }
        assert_eq!(Q::error(), GL_INVALID_ENUM, "glGetInteger64i_v({what}, 0)");
        assert_eq!(v64, i64::MIN);
    }
}

/* ---- the ES 3.0 expectations, ignored until the fixes land ---------------- */

/* One ignored test per gap family: what dEQP needs, and who owns it. The
 * planner's issue texts (`v3d-es3-docs`) cite these names. Run with
 * `cargo test ... -- --ignored` — they fail today on purpose. */

/// ES 3.0's four draw buffers and colour attachments:
/// implementation_limits.max_draw_buffers, .max_color_attachments and their
/// state_query.integers.* rows.
/// Owner: `src/backend.rs` DRAW_TARGETS (gpu-v3d's "3 ... no board has drawn
/// four" — the V3D TLB takes targets 0-3 and the compiler writes four);
/// `src/gl/context.rs` max_draw_buffers/max_color_attachments follow it, and
/// the fragment_out / draw_buffers_indexed / MRT-clear work crosses the same
/// four (docs/notes/v3d-mrt-clear.md).
#[test]
#[ignore = "src/backend.rs DRAW_TARGETS(gpu-v3d) is 3, ES 3.0 needs 4"]
fn four_draw_buffers_and_colour_attachments() {
    let _lib = context();
    assert!(Q::one(GL_MAX_DRAW_BUFFERS) >= 4);
    assert!(Q::one(GL_MAX_COLOR_ATTACHMENTS) >= 4);
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/// ES 3.0's four samples: implementation_limits and the
/// state_query.integers.max_samples value rows (the 66-sample-coverage rows
/// are G2). Owner: `src/gl/state.rs` answers MAX_SAMPLES 1 and
/// `src/gl/framebuffer.rs` refuses >1; the V3D 4x design is
/// `docs/notes/v3d-msaa-es3.md` (src/v3d refuses 4x of several attachments
/// and of ES 3.0 formats today).
#[test]
#[ignore = "src/gl MAX_SAMPLES is 1, ES 3.0 needs 4; 4x design docs/notes/v3d-msaa-es3.md"]
fn four_samples() {
    let _lib = context();
    assert!(Q::one(GL_MAX_SAMPLES) >= 4);
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/// The varying family at ES 3.0's minimums: implementation_limits.max_varying_vectors,
/// .max_varying_components, .max_vertex_output_components,
/// .max_fragment_input_components and their state_query.integers.* rows.
/// Owner: `src/gl/context.rs` MAX_VARYING_VECTORS, 8 on gpu-v3d ("Mesa's
/// v3d allows 16 (V3D_MAX_FS_INPUTS, 64 components), but nothing here has
/// drawn with more than a few varyings on the board"); the component rows
/// and the shader's gl_Max*Vectors derive from it, and `docs/es3-
/// cts-failures.md`/the board run that measures more varyings carries it.
#[test]
#[ignore = "src/gl/context.rs MAX_VARYING_VECTORS is 8 on gpu-v3d, ES 3.0 needs 15"]
fn the_varying_family_meets_es3_0() {
    let _lib = context();
    assert!(Q::one(GL_MAX_VARYING_VECTORS) >= 15);
    assert!(Q::one(GL_MAX_VARYING_COMPONENTS) >= 60);
    assert!(Q::one(GL_MAX_VERTEX_OUTPUT_COMPONENTS) >= 64);
    assert!(Q::one(GL_MAX_FRAGMENT_INPUT_COMPONENTS) >= 60);
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/// The uniform-block family at ES 3.0's minimums:
/// implementation_limits.max_uniform_block_size,
/// .max_combined_{vertex,fragment}_uniform_components and their
/// state_query.integers64.* rows. Owner: `src/gl/context.rs`
/// MAX_UNIFORM_DWORDS (1536) and the `src/gl/state.rs` arms that derive the
/// block size from it and report the bare dwords for the combined limits
/// (no uniform-block share added on top of the default-block budget).
#[test]
#[ignore = "src/gl MAX_UNIFORM_DWORDS gives 6144/1536, ES 3.0 needs 16384/50176/50048"]
fn the_uniform_block_family_meets_es3_0() {
    let _lib = context();
    assert!(Q::one(GL_MAX_UNIFORM_BLOCK_SIZE) >= 16_384);
    assert!(Q::one(GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS) >= 50_176);
    assert!(Q::one(GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS) >= 50_048);
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/// The compressed-format rows: ES 3.0's ten ETC2/EAC formats sampled and
/// listed (implementation_limits.{compressed,num_compressed}_texture_formats
/// and state_query.integers.compressed_texture_formats_*). Owner:
/// `src/gl/texture.rs` `stored_compressed_format` and `decoded_format`; the
/// V3D TMU decodes the four block formats it is given, the other six are
/// decoded by the GL core, so this is storage and listing, not silicon.
#[test]
fn the_etc2_eac_set_is_listed() {
    let _lib = context();
    let n = Q::one(GL_NUM_COMPRESSED_TEXTURE_FORMATS);
    assert!(n >= 10, "NUM_COMPRESSED_TEXTURE_FORMATS = {n}");
    let list = Q::i(GL_COMPRESSED_TEXTURE_FORMATS);
    for f in [
        GL_COMPRESSED_RGB8_ETC2,
        GL_COMPRESSED_SRGB8_ETC2,
        GL_COMPRESSED_RGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        GL_COMPRESSED_SRGB8_PUNCHTHROUGH_ALPHA1_ETC2,
        GL_COMPRESSED_RGBA8_ETC2_EAC,
        GL_COMPRESSED_SRGB8_ALPHA8_ETC2_EAC,
        GL_COMPRESSED_R11_EAC,
        GL_COMPRESSED_SIGNED_R11_EAC,
        GL_COMPRESSED_RG11_EAC,
        GL_COMPRESSED_SIGNED_RG11_EAC,
    ] {
        assert!(list.contains(&i64::from(f)), "0x{f:x} missing from COMPRESSED_TEXTURE_FORMATS");
    }
    assert_eq!(Q::error(), GL_NO_ERROR);
}

/// The 9 state_query.indexed rows: with the extension advertised, the
/// indexed blend/colour-mask state answers through
/// glGetIntegeri_v/glGetInteger64i_v at every draw-buffer index up to
/// GL_MAX_DRAW_BUFFERS. Owner: `src/gl/state.rs` (the extension table and
/// `indexed_state`); the same owner and dependency as the
/// draw_buffers_indexed.* group.
#[test]
#[ignore = "src/gl advertises no GL_OES_draw_buffers_indexed; the draw_buffers_indexed item"]
fn indexed_blend_state_answers() {
    let _lib = context();
    let ext = c_string(GL_EXTENSIONS);
    assert!(ext.contains("GL_OES_draw_buffers_indexed"));
    for target in [GL_BLEND_SRC_RGB, GL_BLEND_DST_RGB, GL_BLEND_EQUATION_RGB, GL_COLOR_WRITEMASK] {
        for i in 0..4 {
            let mut v = i32::MIN;
            // SAFETY: one value of room.
            unsafe {
                glGetIntegeri_v(target, i, &mut v);
            }
            assert_eq!(Q::error(), GL_NO_ERROR, "glGetIntegeri_v(0x{target:x}, {i})");
            assert_ne!(v, i32::MIN);
        }
    }
}
