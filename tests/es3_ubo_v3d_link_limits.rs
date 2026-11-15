// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Link-time limits of uniform blocks on the V3D, at and past what it reports
//! (`cb52fae8`): GL_MAX_UNIFORM_BLOCK_SIZE = 16384 bytes and
//! GL_MAX_{VERTEX,FRAGMENT}_UNIFORM_BLOCKS = 12 (ES 3.0 section 2.11.7: a
//! link error when a block's data size exceeds the first, when a stage uses
//! more active blocks than the second, or when a program uses more than
//! GL_MAX_COMBINED_UNIFORM_BLOCKS).
//!
//! Every member of every block is read by the shader, so each stays active.
//! Only what the driver does today is asserted: the refusals ES 3.0 2.11.7
//! asks for (a block over 16384 bytes, 13 blocks in a stage) are not made by
//! `build_blocks`, and 12 + 12 full blocks overrun the shaderc scratch arena;
//! none of these is an ignored test, the gate allows none.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_v3d_link_limits
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::fmt::Write as _;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
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
unsafe extern "C" fn b_draw(_d: *const GlesDraw) -> i32 {
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
    name: c"es3-ubo-v3d-link-limits".as_ptr() as *const c_char,
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
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
};

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            yakogl::backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(64, 64).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/* ---- the shaders ---------------------------------------------------------- */

/// The vec4 elements of one array member, and the members of a full block:
/// 16 x vec4[64] x 16 bytes = 16384 bytes.
const ELEMS: usize = 64;
const FULL_MEMBERS: usize = 16;

/// (blocks, array members `vec4 m[64]` per block, plain `vec4` members per
/// block) of one stage; 0 blocks declares none.
type Spec = (usize, usize, usize);

/// `n` std140 blocks `{prefix}0..` of `members` array members and `extra`
/// plain vec4 members each, and the statements reading every one of them
/// (the first and last element of an array).
fn blocks(prefix: &str, (n, members, extra): Spec) -> (String, String) {
    let (mut decl, mut read) = (String::new(), String::new());
    for b in 0..n {
        let _ = writeln!(decl, "layout(std140) uniform {prefix}{b} {{");
        for m in 0..members {
            let _ = writeln!(decl, "    highp vec4 m{m}[{ELEMS}];");
            let _ = writeln!(read, "    r += {prefix}i{b}.m{m}[0] + {prefix}i{b}.m{m}[{}];", ELEMS - 1);
        }
        for x in 0..extra {
            let _ = writeln!(decl, "    highp vec4 x{x};");
            let _ = writeln!(read, "    r += {prefix}i{b}.x{x};");
        }
        let _ = writeln!(decl, "}} {prefix}i{b};");
    }
    (decl, read)
}

fn vertex(sp: Spec) -> String {
    let (decl, read) = blocks("V", sp);
    format!("#version 300 es\n{decl}in vec4 a;\nvoid main() {{\n    vec4 r = a;\n{read}    gl_Position = r;\n}}\n")
}

fn fragment(sp: Spec) -> String {
    let (decl, read) = blocks("F", sp);
    format!(
        "#version 300 es\nprecision highp float;\n{decl}out vec4 c;\nvoid main() {{\n    vec4 r = vec4(0.0);\n{read}    c = r;\n}}\n"
    )
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0 as c_char; 1024];
            glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr());
            panic!("compile failed: {}", core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy());
        }
        s
    }
}

/// Compile the two stages and link them: Ok, or the info log of a failed
/// link (asserted non-empty).
fn link(vs: Spec, fs: Spec) -> Result<(), String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, &vertex(vs)), shader(GL_FRAGMENT_SHADER, &fragment(fs)));
    // SAFETY: plain GL calls on handles this function owns.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        let log = core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned();
        glDeleteProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        if ok == 1 {
            Ok(())
        } else {
            assert!(!log.is_empty(), "a failed link has an info log");
            Err(log)
        }
    }
}

/* ---- the cases ------------------------------------------------------------ */

/// The limits the V3D reports, the base of every case below.
#[test]
fn the_v3d_reports_16384_bytes_and_12_blocks() {
    let _g = context();
    let geti = |pname| {
        let mut x: GLint = -1;
        // SAFETY: plain GL call; one value.
        unsafe { glGetIntegerv(pname, &mut x) };
        x
    };
    assert_eq!(geti(GL_MAX_UNIFORM_BLOCK_SIZE), 16384);
    assert_eq!(geti(GL_MAX_VERTEX_UNIFORM_BLOCKS), 12);
    assert_eq!(geti(GL_MAX_FRAGMENT_UNIFORM_BLOCKS), 12);
}

#[test]
fn a_block_of_16384_bytes_links_in_the_vertex_shader() {
    let _g = context();
    link((1, FULL_MEMBERS, 0), (0, 0, 0)).expect("16384 bytes is the limit");
}

#[test]
fn a_block_of_16384_bytes_links_in_the_fragment_shader() {
    let _g = context();
    link((0, 0, 0), (1, FULL_MEMBERS, 0)).expect("16384 bytes is the limit");
}

#[test]
fn twelve_blocks_of_16384_bytes_link_in_one_stage() {
    let _g = context();
    link((12, FULL_MEMBERS, 0), (0, 0, 0)).expect("12 blocks in the vertex shader");
    link((0, 0, 0), (12, FULL_MEMBERS, 0)).expect("12 blocks in the fragment shader");
}
