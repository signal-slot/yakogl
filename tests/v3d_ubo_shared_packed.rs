// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `layout(shared)` and `layout(packed)` uniform blocks at the 16384-byte
//! block bound on the V3D build. The block is
//!
//! ```text
//! layout(<shared|packed>) uniform Big { vec4 m0[128]; ... vec4 m7[128]; [vec4 extra;] };
//! ```
//!
//! whose std140 size is exactly 16384 bytes (1024 vec4), or 16400 bytes with
//! `extra`. Every member a test needs is read by the shader, so a `packed`
//! implementation has no reason to drop it. (The front end lays `shared` and
//! `packed` blocks out by std140 too: `src/shaderc/online/reflect.rs`.)
//!
//! - the link: the 16384-byte block links and answers a
//!   `UNIFORM_BLOCK_DATA_SIZE` of at most 16384 (ES 3.0 lets `packed` and
//!   `shared` be smaller than std140, never the bound larger); the 16400-byte
//!   block is pinned as it links today;
//! - the sim: a fragment shader reads the block's first dword (`m0[0].x`)
//!   and last dword (`m7[127].w`) as `gl_FragDepth`, run by the QPU
//!   interpreter on a uniform stream built from a buffer whose byte offset
//!   `4 k` holds k + 0.5, laid out by the compiler's own block table. The
//!   value read must be the one at the offset `UNIFORM_OFFSET` (plus element
//!   and component) reports for the same program through the GL API.
//!
//! The std140 link-limit cases are `tests/v3d_ubo_std140_edge.rs` and the
//! link-limits item's file.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_shared_packed
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

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
    name: c"v3d-ubo-shared-packed".as_ptr() as *const c_char,
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
    max_samples: 0,
};

/// The compiler keeps its output in one arena that every compile resets, and
/// the GL library is single-threaded by contract: one lock for both halves.
static LIBRARY: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let guard = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    CONTEXT.call_once(|| {
        // SAFETY: both tables are statics, and the guard above is held.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    guard
}

/* ---- the block --------------------------------------------------------- */

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nin highp vec2 a_coord;\nout highp vec2 v_coord;\nvoid main (void)\n{\n    gl_Position = a_position;\n    v_coord = a_coord;\n}\n";

/// Members m0..m7 of `LEN` vec4: 8 * 128 * 16 = 16384 bytes.
const MEMBERS: usize = 8;
const LEN: usize = 128;
const BOUND: GLint = 16384;

/// A fragment shader with the block in `layout` (`extra`: one more vec4 after
/// `m7`), the first and last entries (and `extra`) all read into `o_color`,
/// and `gl_FragDepth = expr`.
fn fs(layout: &str, extra: bool, expr: &str) -> String {
    let members: String = (0..MEMBERS).map(|m| format!("highp vec4 m{m}[{LEN}]; ")).collect();
    let (more, use_more) = if extra { ("highp vec4 extra; ", " + extra.x") } else { ("", "") };
    let last = MEMBERS - 1;
    let end = LEN - 1;
    format!(
        "#version 300 es\nlayout({layout}) uniform Big {{ {members}{more}}};\n\
         in highp vec2 v_coord;\nlayout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(m0[0].x + m{last}[{end}].w{use_more});\n    gl_FragDepth = {expr};\n}}\n"
    )
}

/// The distinct value of buffer dword `k` (byte offset `4 k`).
fn value(k: usize) -> f32 {
    k as f32 + 0.5
}

/// The depth lane 0 writes for `fs`, from a buffer whose dword k is
/// `value(k)`, placed in the default-block rows by the compiler's block table
/// (each member's byte offset and array stride), or why the form is not run.
fn run(fs: &str) -> Result<f32, String> {
    let _g = context();
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower: {}", e.msg))?;
    let target = Target { vpm_size: 65536 };
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, target, &a).map_err(|e| format!("compile: {e:?}"))?;
    let refl = &f.reflection;
    let mut block = vec![0u32; usize::from(refl.uniform_dwords)];
    for m in refl.block_members.iter() {
        if m.matrix_stride != 0 {
            return Err(format!("{}: a matrix member, not modelled by this test", m.name));
        }
        let row = &refl.uniforms[usize::from(m.row)];
        let n = usize::from(m.array_size);
        let per = usize::from(row.dwords) / n;
        for e in 0..n {
            for i in 0..per {
                let src = (m.offset + e as u32 * m.array_stride) as usize / 4 + i;
                block[usize::from(row.dword_off) + e * per + i] = value(src).to_bits();
            }
        }
    }
    let mut stream = Vec::new();
    for u in p.fs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!("stream dword {} past the block", u.data))?,
            c => return Err(format!("uniform contents {c} (UNIFA / UBO address) not modelled by this test")),
        });
    }
    let zero = [0u32; LANES];
    let one = [1.0f32.to_bits(); LANES];
    let varyings = p.fs.input_slots.iter().map(|_| Varying { vary: zero, c: zero }).collect();
    let inputs = FsInputs { w: one, w_centroid: one, z: zero, x: zero, y: zero, back_facing: [false; LANES], varyings };
    let env = Env { threads: p.fs.threads, ..Env::default() };
    let o = sim::run(p.fs.code.words, &stream, &Inputs::Fragment(Box::new(inputs)), &env)
        .map_err(|e| format!("sim: {e}"))?;
    let w = o.tlb.iter().find(|w| w.config == Some(TLB_Z_PER_PIXEL)).ok_or("no depth write")?;
    Ok(f32::from_bits(w.value[0].ok_or("undefined depth")?))
}

/* ---- GL compile, link and queries ---------------------------------------- */

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        s
    }
}

fn compiled(s: GLuint) -> bool {
    let mut ok: GLint = 0;
    // SAFETY: plain GL call.
    unsafe { glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok) };
    ok == 1
}

/// Compile and link the block in `layout`: the program and its link status,
/// or `None` where a shader did not compile. The caller holds `context()`.
fn link(layout: &str, extra: bool) -> Option<(GLuint, bool)> {
    let v = shader(GL_VERTEX_SHADER, VS);
    let f = shader(GL_FRAGMENT_SHADER, &fs(layout, extra, "m0[0].x"));
    if !compiled(v) || !compiled(f) {
        return None;
    }
    // SAFETY: plain GL calls on the one context.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        Some((p, ok == 1))
    }
}

fn data_size(p: GLuint) -> GLint {
    let c = CString::new("Big").unwrap();
    let mut size: GLint = -1;
    // SAFETY: one NUL-terminated name; one block, one value.
    unsafe {
        let b = glGetUniformBlockIndex(p, c.as_ptr());
        assert_ne!(b, GL_INVALID_INDEX, "Big is an active block");
        glGetActiveUniformBlockiv(p, b, GL_UNIFORM_BLOCK_DATA_SIZE, &mut size);
    }
    size
}

fn member(p: GLuint, name: &str, pname: GLenum) -> GLint {
    let c = CString::new(name).unwrap();
    let names: [*const c_char; 1] = [c.as_ptr()];
    let mut idx: GLuint = GL_INVALID_INDEX;
    let mut v: GLint = -1;
    // SAFETY: one name, one index, one value.
    unsafe {
        glGetUniformIndices(p, 1, names.as_ptr(), &mut idx);
        assert_ne!(idx, GL_INVALID_INDEX, "{name} is an active uniform");
        glGetActiveUniformsiv(p, 1, &idx, pname, &mut v);
    }
    v
}

/// The 16384-byte block links and its data size is at most the bound.
fn links_within_the_bound(layout: &str) {
    let _g = context();
    let (p, ok) = link(layout, false).expect("the shaders of the 16384-byte block compile");
    assert!(ok, "the {layout} block of std140 size 16384 links");
    let size = data_size(p);
    assert!(size > 0 && size <= BOUND, "{layout}: UNIFORM_BLOCK_DATA_SIZE {size}, at most {BOUND}");
    // Pinned: the front end lays the block out by std140, so exactly 16384.
    assert_eq!(size, BOUND, "{layout}: the std140 size today");
}

/// The 16400-byte block: what the link answers today.
fn one_vec4_past_the_bound_today(layout: &str) {
    let _g = context();
    let (p, linked) = link(layout, true).expect("the shaders compile");
    // Pinned: the link does not refuse a block past MAX_UNIFORM_BLOCK_SIZE.
    assert!(linked, "{layout}: the 16400-byte block links today");
    assert_eq!(data_size(p), BOUND + 16, "{layout}: data size of the 16400-byte block today");
    assert_eq!(member(p, "extra", GL_UNIFORM_OFFSET), BOUND, "{layout}: extra at byte 16384");
}

/// The first and last dword: GL_UNIFORM_OFFSET and GL_UNIFORM_ARRAY_STRIDE of
/// `m0` and `m7` say where they are, and the sim reads the value stored there.
fn first_and_last_dword(layout: &str) {
    let (first, last) = {
        let _g = context();
        let (p, ok) = link(layout, false).expect("the shaders compile");
        assert!(ok, "the {layout} block links");
        let last_name = format!("m{}[0]", MEMBERS - 1);
        let stride = member(p, &last_name, GL_UNIFORM_ARRAY_STRIDE);
        assert_eq!(stride, 16, "{layout}: array stride of {last_name}");
        assert_eq!(member(p, &last_name, GL_UNIFORM_SIZE), LEN as GLint, "{layout}: {last_name} size");
        let first = member(p, "m0[0]", GL_UNIFORM_OFFSET);
        let last = member(p, &last_name, GL_UNIFORM_OFFSET) + (LEN as GLint - 1) * stride + 12;
        (first as usize, last as usize)
    };
    assert_eq!(first, 0, "{layout}: m0[0].x at byte 0");
    assert_eq!(last, BOUND as usize - 4, "{layout}: m7[127].w at the last dword of 16384 bytes");
    for (expr, at) in [("m0[0].x".to_string(), first), (format!("m{}[{}].w", MEMBERS - 1, LEN - 1), last)] {
        let src = fs(layout, false, &expr);
        let got = run(&src).unwrap_or_else(|why| panic!("{layout} {expr}: {why}\n{src}"));
        assert_eq!(got, value(at / 4), "{layout} {expr}: the value at byte {at}");
    }
}

#[test]
fn shared_16384_links_within_the_bound() {
    links_within_the_bound("shared");
}

#[test]
fn packed_16384_links_within_the_bound() {
    links_within_the_bound("packed");
}

#[test]
fn shared_16400_links_today() {
    one_vec4_past_the_bound_today("shared");
}

#[test]
fn packed_16400_links_today() {
    one_vec4_past_the_bound_today("packed");
}

#[test]
fn shared_sim_reads_first_and_last_dword() {
    first_and_last_dword("shared");
}

#[test]
fn packed_sim_reads_first_and_last_dword() {
    first_and_last_dword("packed");
}
