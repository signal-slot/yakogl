// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_MAX_UNIFORM_BLOCK_SIZE 16384 and the combined uniform components on
//! the V3D: ES 3.0's minimums, carried by the GL core, the compiler and the
//! back end's uniform streams (gl/context.rs MAX_UNIFORM_BLOCK_SIZE says
//! why this GPU can and the other two cannot yet).
//!
//! The limits are asked in all three integer forms (dEQP
//! implementation_limits.max_uniform_block_size and
//! max_combined_{vertex,fragment}_uniform_components, and the
//! state_query.integers64 `*_getfloat` / `*_getinteger` rows of them), the
//! combined ones against ES 3.0's formula (section 6.2: the stage's blocks
//! times the block size in components, plus its default-block components).
//! Then a program with one uniform block of exactly 16384 bytes
//! (4096 dwords: sixteen `vec4[64]` members) links, a buffer of that size
//! is bound to it, and the draw hands the back end the whole block: the
//! dwords of the application's buffer, in the stage's storage the compiled
//! shader's `QUNIFORM_UNIFORM` words index (the job builder writes each
//! word the shader reads into the uniform stream, v3d_job.c).
//!
//! HOST-ONLY: the values the draw hands the back end are checked, no pixel.
//! Under `report-es2` there are no blocks (tests/es2_report_guard.rs); this
//! file is not built there.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_v3d_ubo_size
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/// ES 3.0 (table 6.31): the block size in bytes and the combined components.
const BLOCK_BYTES: i64 = 16384;
const COMBINED_VERTEX: i64 = 50176;
const COMBINED_FRAGMENT: i64 = 50048;

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

/// What the draw handed the back end: the fragment stage's block.
static DRAWN: Mutex<Option<Vec<u32>>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    if d.fs_uniform_cpu.is_null() {
        return 0;
    }
    let words = core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, d.fs_uniform_dwords as usize);
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(words.to_vec());
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
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-v3d-ubo-size".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
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
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    /* the V3D back end reads the host arrays and stages nothing (v3d/backend.rs) */
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
};

static LIBRARY: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LIBRARY.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    g
}

fn geti(p: GLenum) -> i64 {
    let mut v: GLint = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { glGetIntegerv(p, &mut v) };
    i64::from(v)
}

fn getf(p: GLenum) -> f64 {
    let mut v: GLfloat = -1.0;
    // SAFETY: plain GL call with a real output.
    unsafe { glGetFloatv(p, &mut v) };
    f64::from(v)
}

fn geti64(p: GLenum) -> i64 {
    let mut v: GLint64 = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { glGetInteger64v(p, &mut v) };
    v
}

/// dEQP implementation_limits.max_uniform_block_size, max_combined_*_uniform_components
/// and the state_query.integers64 forms of them.
#[test]
fn the_block_size_and_the_combined_components_meet_es3_in_every_query_form() {
    let _g = context();
    let (size, vblocks, fblocks) =
        (geti(GL_MAX_UNIFORM_BLOCK_SIZE), geti(GL_MAX_VERTEX_UNIFORM_BLOCKS), geti(GL_MAX_FRAGMENT_UNIFORM_BLOCKS));
    let (vcomp, fcomp) = (geti(GL_MAX_VERTEX_UNIFORM_COMPONENTS), geti(GL_MAX_FRAGMENT_UNIFORM_COMPONENTS));
    assert_eq!(size, BLOCK_BYTES, "GL_MAX_UNIFORM_BLOCK_SIZE");
    /* ES 3.0 section 6.2: blocks * size / 4 + the default block's components */
    let (vwant, fwant) = (vblocks * size / 4 + vcomp, fblocks * size / 4 + fcomp);
    assert_eq!(geti(GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS), vwant, "the formula, vertex");
    assert_eq!(geti(GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS), fwant, "the formula, fragment");
    assert!(vwant >= COMBINED_VERTEX && fwant >= COMBINED_FRAGMENT, "the ES 3.0 minimums: {vwant} {fwant}");
    for (name, pname) in [
        ("GL_MAX_UNIFORM_BLOCK_SIZE", GL_MAX_UNIFORM_BLOCK_SIZE),
        ("GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS", GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS),
        ("GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS", GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS),
    ] {
        let i = geti(pname);
        assert_eq!(geti64(pname), i, "{name} (glGetInteger64v)");
        assert_eq!(getf(pname), i as f64, "{name} (glGetFloatv)");
    }
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glGetError() }, GL_NO_ERROR);
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
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("did not compile: {}\n{text:?}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        s
    }
}

/// A fragment shader with one block `B` of `n` vec4s (sixteen arrays of 64
/// would be 4096 dwords for n = 1024), reading the first and last element of
/// every array, the first array also by a dynamic index.
fn block_fs(arrays: usize) -> String {
    let mut decl = String::new();
    let mut body = String::from("vec4 s = vec4(0.0);\n");
    for j in 0..arrays {
        decl += &format!("vec4 a{j}[64];\n");
        body += &if j == 0 { format!("s += a{j}[i] + a{j}[63];\n") } else { format!("s += a{j}[0] + a{j}[63];\n") };
    }
    format!(
        "#version 300 es\nprecision highp float;\nuniform B {{ {decl} }};\nuniform int i;\n\
         layout(location = 0) out highp vec4 o;\nvoid main() {{ {body} o = s; }}\n"
    )
}

/// A block of `arrays` * 64 vec4: 16 arrays are exactly 16384 bytes.
fn run(arrays: usize) -> (usize, Option<Vec<u32>>, Vec<u32>) {
    let bytes = arrays * 64 * 16;
    let data: Vec<u32> = (0..bytes / 4).map(|j| 0x4000_0000 + j as u32).collect();
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        let vs = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n");
        let fs = shader(GL_FRAGMENT_SHADER, &block_fs(arrays));
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("{bytes} bytes did not link: {}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        glUseProgram(p);
        let idx = glGetUniformBlockIndex(p, c"B".as_ptr());
        assert_eq!(idx, 0, "block B");
        let mut size: GLint = 0;
        glGetActiveUniformBlockiv(p, idx, GL_UNIFORM_BLOCK_DATA_SIZE, &mut size);
        assert_eq!(size as usize, bytes, "std140 size of the block");
        glUniformBlockBinding(p, idx, 0);
        let mut buf = 0;
        glGenBuffers(1, &mut buf);
        glBindBuffer(GL_UNIFORM_BUFFER, buf);
        glBufferData(GL_UNIFORM_BUFFER, bytes as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glBindBufferBase(GL_UNIFORM_BUFFER, 0, buf);
        static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        let refused = backend::draws_refused();
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
        assert_eq!(backend::draws_refused(), refused, "the draw was not refused");
        glDeleteBuffers(1, &buf);
        glUseProgram(0);
        glDeleteProgram(p);
    }
    let drawn = DRAWN.lock().unwrap_or_else(|e| e.into_inner()).take();
    (bytes, drawn, data)
}

/// The block's words, wherever the compiled shader placed it in the stage's
/// storage: the application's data as a run of the storage.
fn contains_run(storage: &[u32], data: &[u32]) -> bool {
    storage.windows(data.len()).any(|w| w == data)
}

#[test]
fn a_block_of_16384_bytes_links_and_the_draw_hands_over_all_of_it() {
    let _g = context();
    let (bytes, drawn, data) = run(16);
    assert_eq!(bytes as i64, BLOCK_BYTES);
    let storage = drawn.expect("the draw reached the back end with a uniform block");
    assert!(storage.len() >= data.len(), "{} dwords handed over for a block of {}", storage.len(), data.len());
    assert!(contains_run(&storage, &data), "the 4096 dwords of the buffer are in the stage's storage");
}

#[test]
fn a_block_of_six_thousand_bytes_still_draws_as_before() {
    let _g = context();
    /* 6 arrays of 64 vec4: 6144 bytes, the old limit */
    let (_, drawn, data) = run(6);
    assert!(contains_run(&drawn.expect("handed over"), &data));
}
