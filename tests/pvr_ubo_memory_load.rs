// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The PowerVR online compiler on a 16384-byte uniform block, a draft for
//! plan #378 (acceptance check c2). Block members are to be read by a USC
//! `ld` (global memory load) from the block's address, followed by a `wdf`
//! before the destination is read; default uniforms stay in shared registers
//! (`docs/notes/pvr-ubo-memory-load.md`). HOST-ONLY.
//!
//! The block is `vec4 a[255]; b[255]; c[255]; d[255]; e[4];`: 4*255*16 + 4*16
//! = 16384 std140 bytes. It is not one `vec4 v[1024]` because reflect.rs
//! `fit_u8` caps a uniform array at 255 elements.
//!
//! Four reads, in the vertex and in the fragment stage: the first dword at a
//! constant index (`a[0].x`, byte 0), the last at a constant index
//! (`e[3].w`, byte 16380), and each at a run-time index (`a[i].x`, `e[i].w`,
//! `i` a default uniform int).
//!
//! Live cases pin what holds today and after the raise: a 256-byte block
//! compiles and links, and a shader with no block holds no `ld`.
//!
//! Since `ubo-chain` the online chain reads block members with `ld`, so the
//! 16384-byte block compiles and links: its 4096 dwords stay in memory, and
//! only the default prefix and the address pair are shared registers. The
//! `ld` cases also check the record's `ubo_addr_off1`
//! (`backend::pvr_ubo_addr_off1`, until `ShcShader` carries the field): the
//! `add64_32` reads the pair's low dword there, and at a constant index one
//! `ld` window covers the dword read. Whether the back end writes the pair
//! is `ubo-backend`'s (tests/pvr_ubo_draw_16k.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_ubo_memory_load
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::Once;
use yakogl::backend::{GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::powervr::decode::for_each_group;
use yakogl::shaderc::online::powervr::listing::group_text;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

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
    name: c"pvr-ubo-memory-load".as_ptr() as *const c_char,
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

static COMPILING: Mutex<()> = Mutex::new(());
static CONTEXT: Once = Once::new();

/// The context, made once; the guard keeps the tests of this file off the
/// library (single-threaded by contract) one at a time.
fn context() -> MutexGuard<'static, ()> {
    let guard = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
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

/* ---- shaders, links and listings ------------------------------------------- */

fn cstr(b: &[u8]) -> String {
    let n = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..n]).into_owned()
}

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let c = std::ffi::CString::new(text).unwrap();
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

/// Compile and link; Err carries the shader or program info log.
fn link(vs: &str, fs: &str) -> Result<(), String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs)?, shader(GL_FRAGMENT_SHADER, fs)?);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let r = if ok == 1 {
            Ok(())
        } else {
            let mut log = [0u8; 1024];
            glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
            Err(cstr(&log))
        };
        glDeleteProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        r
    }
}

/// The online compiler's listing of one stage (every group of the USC
/// program, `listing::group_text`), or the compile log.
fn listing(stage: u32, text: &str) -> Result<String, String> {
    compiled(stage, text).map(|(l, _, _)| l)
}

/// [`listing`], the record's `ubo_addr_off1` (`backend::pvr_ubo_addr_off1`:
/// the address pair's register plus one) and its `uniform_sh_dwords`.
fn compiled(stage: u32, text: &str) -> Result<(String, u16, u16), String> {
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let src = format!("{text}\0");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    if unsafe { compile(stage, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) } != 0 {
        // SAFETY: the compiler NUL-terminates the log.
        return Err(unsafe { std::ffi::CStr::from_ptr(log.as_ptr()) }.to_string_lossy().into_owned());
    }
    let bytes: Vec<u8> = out.usc_words().iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut t = String::new();
    for_each_group(&bytes, |off, g| {
        group_text(&mut t, off, g).expect("a String does not fail");
        Ok(())
    })
    .map_err(|e| format!("the compiled program does not decode: {e:?}"))?;
    Ok((t, yakogl::shaderc::online::backend::pvr_ubo_addr_off1(&out), out.uniform_sh_dwords))
}

/// Whether a listing line is an instruction line (`phaseN ld|...`) of `op`.
fn is_op(line: &str, op: &str) -> bool {
    let mut w = line.split_whitespace();
    w.next().is_some_and(|p| p.starts_with("phase")) && w.next().is_some_and(|o| o.split('|').any(|n| n == op))
}

fn has_op(listing: &str, op: &str) -> bool {
    listing.lines().any(|l| is_op(l, op))
}

/// The value of `key=` in `text`'s first field of that name.
fn field(text: &str, key: &str) -> Option<String> {
    text.split_whitespace().find_map(|w| w.strip_prefix(key)?.strip_prefix('=').map(str::to_owned))
}

/// The listing's groups, each as its text.
fn groups(listing: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in listing.lines() {
        if line.starts_with('[') || out.is_empty() {
            out.push(String::new());
        }
        let g = out.last_mut().expect("one group");
        g.push_str(line);
        g.push('\n');
    }
    out
}

/// The `add64_32` reads the address pair's low dword from shared register
/// `pair` (an operand below sh256 is named directly), and adds a constant
/// `k` such that one `ld` window `[k, k + 4 * burst)` covers byte `off`:
/// `k` an `imm32` the program loads into the sum's offset operand, or the
/// value of the constant register it names (`prepare::const_reg`).
fn assert_ld_window(l: &str, pair: u16, off: u32, case: &str) {
    use yakogl::shaderc::online::powervr::prepare::const_reg;
    let gs = groups(l);
    let add = gs
        .iter()
        .find(|g| g.lines().any(|x| is_op(x, "add64_32")))
        .unwrap_or_else(|| panic!("{case}: no add64_32:\n{l}"));
    let lo = add.lines().find(|x| x.trim_start().starts_with("lower_src:")).expect("an add64_32 reads sources");
    assert_eq!(
        (field(lo, "sb0").as_deref(), field(lo, "s0")),
        (Some("shared"), Some(pair.to_string())),
        "{case}: the sum's low dword is not sh{pair}:\n{add}"
    );
    let special = (field(lo, "sb2").as_deref() == Some("special")).then(|| field(lo, "s2")).flatten();
    let imms: Vec<u32> = gs.iter().filter_map(|g| field(g, "imm32")?.parse().ok()).collect();
    let burst = gs.iter().filter_map(|g| field(g, "burstlen")?.parse::<u32>().ok()).max().unwrap_or(0);
    let added = |k: u32| match &special {
        Some(r) => const_reg(k).is_some_and(|c| c.to_string() == *r),
        None => imms.contains(&k),
    };
    assert!(
        (off.saturating_sub(4 * burst - 4)..=off).step_by(4).any(added),
        "{case}: no ld window of {burst} dwords covers byte {off}:\n{l}"
    );
}

/* ---- the blocks ------------------------------------------------------------- */

/// 4*255*16 + 4*16 = 16384 std140 bytes.
const BLOCK_16K: &str =
    "layout(std140) uniform B { vec4 a[255]; vec4 b[255]; vec4 c[255]; vec4 d[255]; vec4 e[4]; };\nuniform int i;\n";
/// 16 vec4 = 256 bytes.
const BLOCK_256: &str = "layout(std140) uniform B { vec4 a[16]; };\nuniform int i;\n";

const VS_OK: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS_OK: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

/// A stage that reads `expr`, a float expression over the block's members.
fn vs(block: &str, expr: &str) -> String {
    format!("#version 300 es\nin vec4 p;\n{block}void main() {{ gl_Position = p + vec4({expr}); }}\n")
}
fn fs(block: &str, expr: &str) -> String {
    format!("#version 300 es\nprecision mediump float;\n{block}out vec4 o;\nvoid main() {{ o = vec4({expr}); }}\n")
}

/// The four reads: (name, expression, byte offset at a constant index).
const READS: [(&str, &str, Option<u32>); 4] = [
    ("first_const", "a[0].x", Some(0)),
    ("last_const", "e[3].w", Some(16380)),
    ("first_runtime", "a[i].x", None),
    ("last_runtime", "e[i].w", None),
];

/* ---- live: what is true today and stays true ---------------------------------- */

#[test]
fn small_block_compiles_and_links() {
    let _g = context();
    let expr = "a[3].y + a[i].x";
    link(&vs(BLOCK_256, expr), FS_OK).unwrap_or_else(|e| panic!("vertex: {e}"));
    link(VS_OK, &fs(BLOCK_256, expr)).unwrap_or_else(|e| panic!("fragment: {e}"));
}

#[test]
fn shader_without_a_block_holds_no_ld() {
    let _g = context();
    let src_v = "#version 300 es\nin vec4 p;\nuniform vec4 u[8];\nuniform int i;\nvoid main() { gl_Position = p + u[0] + u[i]; }\n";
    let src_f = "#version 300 es\nprecision mediump float;\nuniform vec4 u[8];\nuniform int i;\nout vec4 o;\nvoid main() { o = u[7] + u[i]; }\n";
    for (stage, src) in [(SHC_STAGE_VERTEX, src_v), (SHC_STAGE_FRAGMENT, src_f)] {
        let l = listing(stage, src).unwrap_or_else(|e| panic!("stage {stage}: {e}"));
        assert!(has_op(&l, "mul") || has_op(&l, "fadd") || !l.is_empty(), "stage {stage}: empty listing");
        assert!(!has_op(&l, "ld"), "stage {stage} holds an ld:\n{l}");
    }
}

/* ---- ignored: the 16384-byte block, read by ld -------------------------------- */

#[test]
fn block_16k_vertex_links() {
    let _g = context();
    for (name, expr, _) in READS {
        link(&vs(BLOCK_16K, expr), FS_OK).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

#[test]
fn block_16k_fragment_links() {
    let _g = context();
    for (name, expr, _) in READS {
        link(VS_OK, &fs(BLOCK_16K, expr)).unwrap_or_else(|e| panic!("{name}: {e}"));
    }
}

/// The stage's listing has an `ld` followed by a `wdf` (the wait on the
/// load's result, before the destination is read).
fn ld_then_wdf(stage: u32, src: &str, case: &str, offset: Option<u32>) {
    let _g = context();
    let (l, off1, prefix) = compiled(stage, src).unwrap_or_else(|e| panic!("{case}: {e}"));
    let at = |op| l.lines().position(|x| is_op(x, op));
    let ld = at("ld").unwrap_or_else(|| panic!("{case}: no ld:\n{l}"));
    let wdf = at("wdf").unwrap_or_else(|| panic!("{case}: no wdf:\n{l}"));
    assert!(ld < wdf, "{case}: wdf before the ld:\n{l}");
    assert_ne!(off1, 0, "{case}: the record names no address pair");
    // The block's 4096 dwords stay in memory: the shared registers hold the
    // default prefix (`uniform_sh_dwords`) and the pair right after it, no
    // sampler descriptor before them. `i` is declared after the block, so its
    // row is past the block's: the prefix is empty and `i` is read with ld
    // too.
    assert_eq!((prefix, off1 - 1), (0, 0), "{case}: the shared prefix and the pair");
    if let Some(off) = offset {
        // The ld's address is the pair at ubo_addr_off1 - 1 plus a constant
        // byte offset; a vec4 member is one 4-dword burst from its start, so
        // the window covers `off` (0 for the first dword, 16380 for the last).
        assert_ld_window(&l, off1 - 1, off, case);
    }
}

macro_rules! read_case {
    ($name:ident, $stage:expr, $build:ident, $idx:expr) => {
        #[test]
        fn $name() {
            let (case, expr, off) = READS[$idx];
            ld_then_wdf($stage, &$build(BLOCK_16K, expr), case, off);
        }
    };
}
read_case!(ld_then_wdf_vertex_first_const, SHC_STAGE_VERTEX, vs, 0);
read_case!(ld_then_wdf_vertex_last_const, SHC_STAGE_VERTEX, vs, 1);
read_case!(ld_then_wdf_vertex_first_runtime, SHC_STAGE_VERTEX, vs, 2);
read_case!(ld_then_wdf_vertex_last_runtime, SHC_STAGE_VERTEX, vs, 3);
read_case!(ld_then_wdf_fragment_first_const, SHC_STAGE_FRAGMENT, fs, 0);
read_case!(ld_then_wdf_fragment_last_const, SHC_STAGE_FRAGMENT, fs, 1);
read_case!(ld_then_wdf_fragment_first_runtime, SHC_STAGE_FRAGMENT, fs, 2);
read_case!(ld_then_wdf_fragment_last_runtime, SHC_STAGE_FRAGMENT, fs, 3);
