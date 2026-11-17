// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A host draw of a 16384-byte uniform block on the PowerVR build: the check
//! for plan #378's acceptance c4 ("a host draw reads a full 16384-byte block
//! at its first and last dword"), drafted before the back end can do it.
//!
//! Design: docs/notes/pvr-ubo-memory-load.md, option A. The compiler reads
//! block members with `ld` from a per-draw copy of the stage's block bytes;
//! the back end writes each block's 64-bit device address into the
//! shared-register pair the compiler names (`ubo_addr_off1`) and no longer
//! stages blocks in the GENERAL arena.
//!
//! What a host-test draw observes today is what the GL core hands the back
//! end: a `draw` hook sees the `GlesDraw` (the stage uniform bytes
//! `*_uniform_cpu`). The back end's own buffers (the per-draw block copy, the
//! address in the shared registers, the GENERAL arena) are in-crate state
//! (src/pvr/backend_tests.rs `Rig`), not visible from an integration test;
//! those cases are ignored and carry a TODO for ubo-backend, which un-ignores
//! them (in-crate, or through a hook it adds).
//!
//! The 16384-byte block is `vec4 a[255]; b[255]; c[255]; d[255]; e[4];` (the
//! u8 array-length limit forbids vec4[1024]). The buffer holds the pattern
//! dword k = 0xA5000000 | k.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_ubo_draw_16k
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_ubo_draw_16k -- --ignored
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard};
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

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
unsafe extern "C" fn b_ok() -> i32 {
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

/// What the draw hook saw of one draw.
#[derive(Clone, Debug, Default)]
struct Rec {
    vs_dwords: Vec<u32>,
}

static DRAWS: Mutex<Vec<Rec>> = Mutex::new(Vec::new());

unsafe fn stage(cpu: *const c_void, dwords: u16) -> Vec<u32> {
    if cpu.is_null() {
        return Vec::new();
    }
    // SAFETY: the draw carries `dwords` dwords at `cpu` for the call.
    core::slice::from_raw_parts(cpu as *const u32, dwords as usize).to_vec()
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    DRAWS.lock().unwrap().push(Rec { vs_dwords: stage(d.vs_uniform_cpu, d.vs_uniform_dwords) });
    0
}

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"pvr-ubo-draw-16k".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_ok),
    shutdown: None,
    draw: Some(b_draw),
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

static LOCK: Mutex<()> = Mutex::new(());

fn context() -> MutexGuard<'static, ()> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        ONCE.call_once(|| assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null()));
    }
    DRAWS.lock().unwrap().clear();
    g
}

/* ---- shaders --------------------------------------------------------------- */

const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

/// The 16384-byte std140 block: 4 * 255 + 4 = 1024 vec4.
const BLOCK_16K: &str =
    "layout(std140) uniform B { vec4 a[255]; vec4 b[255]; vec4 c[255]; vec4 d[255]; vec4 e[4]; } blk;\n";
/// A 256-byte block (16 vec4).
const BLOCK_256: &str = "layout(std140) uniform B { vec4 a[16]; } blk;\n";

/// Reads dword 0 (a[0].x) and the last dword (e[3].w) at constant indices.
fn vs_16k_const() -> String {
    format!("#version 300 es\nin vec4 p;\n{BLOCK_16K}void main() {{ gl_Position = p + blk.a[0] + blk.e[3]; }}\n")
}

/// The same at run-time indices, taken from `p` so they are not constants.
fn vs_16k_runtime() -> String {
    format!(
        "#version 300 es\nin vec4 p;\n{BLOCK_16K}void main() {{ int i = int(p.x); int j = int(p.y);\n\
         gl_Position = p + blk.a[i] + blk.e[j]; }}\n"
    )
}

fn vs_256() -> String {
    format!("#version 300 es\nin vec4 p;\n{BLOCK_256}void main() {{ gl_Position = p + blk.a[0] + blk.a[15]; }}\n")
}

fn cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0u8; 1024];
            glGetShaderInfoLog(s, 1024, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
            return Err(cstr(&log));
        }
        Ok(s)
    }
}

/// Compile, link and use; Err carries the shader or program info log.
fn link(vs: &str) -> Result<GLuint, String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs)?, shader(GL_FRAGMENT_SHADER, FS)?);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut log = [0u8; 1024];
            glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
            return Err(cstr(&log));
        }
        glUseProgram(p);
        Ok(p)
    }
}

/// The pattern bytes: dword k = 0xA5000000 | k.
fn pattern(dwords: usize) -> Vec<u8> {
    (0..dwords as u32).flat_map(|k| (0xA500_0000 | k).to_le_bytes()).collect()
}

fn pat(k: usize) -> u32 {
    0xA500_0000 | k as u32
}

/// Links `vs`, binds `bytes` bytes of the pattern to block B at binding 0
/// with glBindBufferRange (the whole buffer) and draws one triangle.
/// Returns the GL error and the draws the hook saw.
fn draw_block(vs: &str, bytes: usize) -> Result<(GLenum, Vec<Rec>), String> {
    let p = link(vs)?;
    let data = pattern(bytes / 4);
    let tri: [f32; 12] = [0.0, 3.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    // SAFETY: plain GL calls; every pointer outlives its call.
    let err = unsafe {
        let (mut ub, mut vb) = (0, 0);
        glGenBuffers(1, &mut ub);
        glBindBuffer(GL_UNIFORM_BUFFER, ub);
        glBufferData(GL_UNIFORM_BUFFER, bytes as isize, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glBindBufferRange(GL_UNIFORM_BUFFER, 0, ub, 0, bytes as isize);
        let idx = glGetUniformBlockIndex(p, c"B".as_ptr());
        glUniformBlockBinding(p, idx, 0);
        glGenBuffers(1, &mut vb);
        glBindBuffer(GL_ARRAY_BUFFER, vb);
        glBufferData(GL_ARRAY_BUFFER, 48, tri.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(0);
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        glDisableVertexAttribArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glUseProgram(0);
        glDeleteProgram(p);
        glDeleteBuffers(1, &vb);
        glDeleteBuffers(1, &ub);
        e
    };
    Ok((err, DRAWS.lock().unwrap().clone()))
}

/// Where the pattern's first dword sits in `staged`, if a whole run of `n`
/// pattern dwords is there.
fn find_pattern(staged: &[u32], n: usize) -> Option<usize> {
    (0..staged.len().saturating_sub(n - 1)).find(|&s| (0..n).all(|k| staged[s + k] == pat(k)))
}

/* ---- live: what passes on today's tree ------------------------------------- */

#[test]
fn small_block_draw_runs_and_its_bytes_reach_the_staged_data() {
    let _g = context();
    let (err, draws) = draw_block(&vs_256(), 256).expect("a 256-byte block compiles and links");
    assert_eq!(err, GL_NO_ERROR);
    assert_eq!(draws.len(), 1, "one draw reached the back end");
    let s = &draws[0].vs_dwords;
    assert!(!s.is_empty(), "the vertex stage carries uniform data");
    // Today the GL core stages block bytes with the stage's uniforms; after
    // ubo-backend they travel in the per-draw block copy instead, and this
    // case then checks the default-block part only (or is retired).
    let at = find_pattern(s, 64).unwrap_or_else(|| panic!("pattern not found in {} staged dwords", s.len()));
    assert_eq!(s[at + 63], pat(63));
}

#[test]
fn full_block_compiles_and_links_or_names_the_limit() {
    let _g = context();
    // 6144 bytes today; 16384 after ctx-limits. Either is a clean outcome: a
    // link log naming the uniform limit, or a link.
    if let Err(e) = link(&vs_16k_const()) {
        assert!(e.to_lowercase().contains("uniform"), "a refusal names the uniform limit, got: {e}");
    }
}

/* ---- ignored: need the back end's block address hand-off -------------------- */

#[test]
#[ignore = "until ubo-backend: block address in shared registers"]
fn full_16384_byte_block_draw_runs() {
    let _g = context();
    let (err, draws) = draw_block(&vs_16k_const(), 16384).expect("a 16384-byte block links (needs ctx-limits)");
    assert_eq!(err, GL_NO_ERROR);
    assert_eq!(draws.len(), 1);
}

#[test]
#[ignore = "until ubo-backend: block address in shared registers"]
fn full_block_run_time_index_draw_runs() {
    let _g = context();
    let (err, draws) = draw_block(&vs_16k_runtime(), 16384).expect("a 16384-byte block links (needs ctx-limits)");
    assert_eq!(err, GL_NO_ERROR);
    assert_eq!(draws.len(), 1);
}

#[test]
#[ignore = "until ubo-backend: block address in shared registers"]
fn per_draw_block_copy_holds_dword_0_and_dword_4095() {
    let _g = context();
    let (err, draws) = draw_block(&vs_16k_const(), 16384).expect("a 16384-byte block links");
    assert_eq!(err, GL_NO_ERROR);
    assert_eq!(draws.len(), 1);
    // TODO(ubo-backend): read the back end's per-draw block copy for the
    // vertex stage (a hook, or an in-crate Rig, src/pvr/backend_tests.rs)
    // and assert copy[0] == pat(0), copy[4095] == pat(4095), all 4096 dwords
    // equal to the pattern, the copy 16-byte aligned.
    panic!("TODO(ubo-backend): the per-draw block copy is not observable yet");
}

#[test]
#[ignore = "until ubo-backend: block address in shared registers"]
fn block_address_pair_points_at_the_copy() {
    let _g = context();
    let (err, draws) = draw_block(&vs_16k_const(), 16384).expect("a 16384-byte block links");
    assert_eq!(err, GL_NO_ERROR);
    assert_eq!(draws.len(), 1);
    // TODO(ubo-backend): take `ubo_addr_off1` from the vertex ShcShader (item
    // shc-field), read shared registers off1 (low) and off1 + 1 (high) from
    // what the back end hands the PDS, and assert low | high << 32 equals the
    // device address of the per-draw block copy of the previous case.
    panic!("TODO(ubo-backend): ubo_addr_off1 and the pair's value are not observable yet");
}

#[test]
#[ignore = "until ubo-backend: block address in shared registers"]
fn general_arena_holds_no_block_bytes() {
    let _g = context();
    let (err, draws) = draw_block(&vs_16k_const(), 16384).expect("a 16384-byte block links");
    assert_eq!(err, GL_NO_ERROR);
    assert_eq!(draws.len(), 1);
    // TODO(ubo-backend): scan the GENERAL arena the draw wrote (Rig, pattern
    // 0xA5xxxxxx) and assert no run of pattern dwords is in it; only the
    // default block's dwords are staged there.
    panic!("TODO(ubo-backend): the GENERAL arena is not observable yet");
}
