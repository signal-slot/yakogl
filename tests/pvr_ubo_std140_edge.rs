// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The std140 layout rewrites of uniform-block members on the PowerVR build,
//! the PowerVR twin of tests/v3d_ubo_std140_edge.rs and
//! tests/v3d_ubo_matrix_layouts.rs, for plan #378.
//!
//! Today the GL core copies a block's bytes into the stage's flattened uniform
//! array while it fills the draw (src/gl/program.rs `fill_uniform_blocks` /
//! `copy_block_member`): a row_major matrix is transposed, a mat3 / mat2x3
//! column is strided to 4 dwords (a mat2 column to 2), an array element sits
//! at the row's own stride, a bool is normalised to 0 / 1, ints and uints are
//! copied. After the raise the online compiler reads block members with a USC
//! `ld` from the per-draw copy of that array
//! (docs/notes/pvr-ubo-memory-load.md, option A), so the rewrites must
//! survive: the `ld` reads what the core wrote.
//!
//! Live cases (small blocks, under 6144 bytes): each shape compiles and links
//! on the PowerVR build, and the `draw` hook pins the flattened dwords the
//! stage array holds for it. The buffer holds dword k = 0xA5000000 | k, with
//! a `uint mk` marker first (dword 0); bool cases hold other values so the
//! normalisation shows.
//!
//! `ld_*` cases: the member is read by an `ld` followed by a `wdf` in the
//! vertex listing (since `ubo-chain`), from the address pair the record
//! names (`ubo_addr_off1`, read with `backend::pvr_ubo_addr_off1` until
//! `ShcShader` carries it) plus a constant whose `ld` window covers the
//! element's place in the stage array: the `ld` reads the core's per-draw
//! copy of that array, not the buffer, so the element is where the draw
//! stages its value (`Case::ld_offset` is its std140 offset; the core
//! packs arrays and transposes a row_major matrix). `ubo-backend` retires
//! or moves in-crate the staged-dword cases only if the per-draw copy stops
//! being the stage array the hook sees.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_ubo_std140_edge
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
use yakogl::shaderc::online::powervr::decode::for_each_group;
use yakogl::shaderc::online::powervr::listing::group_text;
use yakogl::shaderc::{ShcShader, SHC_STAGE_VERTEX};

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
    name: c"pvr-ubo-std140-edge".as_ptr() as *const c_char,
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
    max_samples: 0,
};

static COMPILING: Mutex<()> = Mutex::new(());

fn context() -> MutexGuard<'static, ()> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
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

/* ---- shaders, links, draws and listings ------------------------------------ */

const FS: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

/// A vertex shader over block B with `members`, adding the float `expr`.
fn vs(members: &str, expr: &str) -> String {
    format!(
        "#version 300 es\nin vec4 p;\nstruct S {{ vec3 a; float b; ivec2 c; }};\n\
         layout(std140) uniform B {{ {members} }};\nvoid main() {{ gl_Position = p + vec4({expr}); }}\n"
    )
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

fn pat(k: usize) -> u32 {
    0xA500_0000 | k as u32
}

/// Binds `data` to block B at binding 0 and draws one triangle; returns the
/// vertex stage's flattened uniform dwords the hook saw.
fn draw_block(vs: &str, data: &[u32]) -> Result<Vec<u32>, String> {
    DRAWS.lock().unwrap().clear();
    let p = link(vs)?;
    let bytes: Vec<u8> = data.iter().flat_map(|w| w.to_le_bytes()).collect();
    let tri: [f32; 12] = [0.0, 3.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
    // SAFETY: plain GL calls; every pointer outlives its call.
    let err = unsafe {
        let (mut ub, mut vb) = (0, 0);
        glGenBuffers(1, &mut ub);
        glBindBuffer(GL_UNIFORM_BUFFER, ub);
        glBufferData(GL_UNIFORM_BUFFER, bytes.len() as isize, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glBindBufferRange(GL_UNIFORM_BUFFER, 0, ub, 0, bytes.len() as isize);
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
    assert_eq!(err, GL_NO_ERROR);
    let draws = DRAWS.lock().unwrap();
    assert_eq!(draws.len(), 1, "one draw reached the back end");
    Ok(draws[0].vs_dwords.clone())
}

/// The online compiler's listing of the vertex stage (every group of the USC
/// program, `listing::group_text`) and the record's `ubo_addr_off1`
/// (`backend::pvr_ubo_addr_off1`: the address pair's register plus one), or
/// the compile log.
fn listing(text: &str) -> Result<(String, u16), String> {
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    yakogl::shaderc::online::backend::forget_compiled();
    let mut out = ShcShader::ZERO;
    let mut log = [0 as c_char; 512];
    let src = format!("{text}\0");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    if unsafe { compile(SHC_STAGE_VERTEX, src.as_ptr() as *const c_char, &mut out, log.as_mut_ptr(), log.len()) } != 0 {
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
    Ok((t, yakogl::shaderc::online::backend::pvr_ubo_addr_off1(&out)))
}

/// Whether a listing line is an instruction line (`phaseN ld|...`) of `op`.
fn is_op(line: &str, op: &str) -> bool {
    let mut w = line.split_whitespace();
    w.next().is_some_and(|p| p.starts_with("phase")) && w.next().is_some_and(|o| o.split('|').any(|n| n == op))
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

/* ---- the cases -------------------------------------------------------------- */

/// One block shape. `members` follow `uint mk` (dword 0) in block B.
struct Case {
    name: &'static str,
    members: &'static str,
    /// Reads every member (the live cases).
    expr: &'static str,
    /// Reads one element (the ld cases), at std140 byte offset `ld_offset`.
    ld_expr: &'static str,
    ld_offset: u32,
    /// Buffer length in dwords; dword k holds `pat(k)` unless overridden.
    dwords: usize,
    data: Vec<(usize, u32)>,
    /// Contiguous runs of staged dwords (None = padding, not checked), each
    /// found after the previous one, the first after the marker.
    runs: Vec<Vec<Option<u32>>>,
}

fn pv(k: usize) -> Option<u32> {
    Some(pat(k))
}

/// A `cols` x `rows` matrix staged column by column at `col_dw` dwords per
/// column; `at(c, r)` is the std140 dword of element [c][r].
fn matrix(cols: usize, rows: usize, col_dw: usize, at: impl Fn(usize, usize) -> usize) -> Vec<Option<u32>> {
    let mut v = vec![None; cols * col_dw];
    for c in 0..cols {
        for r in 0..rows {
            v[c * col_dw + r] = pv(at(c, r));
        }
    }
    v
}

#[allow(clippy::too_many_arguments)]
fn case(
    name: &'static str,
    members: &'static str,
    expr: &'static str,
    ld_expr: &'static str,
    ld_offset: u32,
    dwords: usize,
    data: Vec<(usize, u32)>,
    runs: Vec<Vec<Option<u32>>>,
) -> Case {
    Case { name, members, expr, ld_expr, ld_offset, dwords, data, runs }
}

fn cases() -> Vec<Case> {
    vec![
        /* row_major: staged transposed; std140 [c][r] is at row * 16 + column * 4 */
        case(
            "row_major_mat4",
            "uint mk; layout(row_major) mat4 m;",
            "float(mk) + m[0][0] + m[3][3]",
            "m[1][2]",
            (4 + 2 * 4 + 1) * 4,
            20,
            vec![],
            vec![matrix(4, 4, 4, |c, r| 4 + r * 4 + c)],
        ),
        case(
            "row_major_mat3",
            "uint mk; layout(row_major) mat3 m;",
            "float(mk) + m[0][0] + m[2][2]",
            "m[2][1]",
            (4 + 4 + 2) * 4,
            16,
            vec![],
            vec![matrix(3, 3, 4, |c, r| 4 + r * 4 + c)],
        ),
        case(
            "row_major_mat2x3",
            "uint mk; layout(row_major) mat2x3 m;",
            "float(mk) + m[0][0] + m[1][2]",
            "m[1][2]",
            (4 + 2 * 4 + 1) * 4,
            16,
            vec![],
            vec![matrix(2, 3, 4, |c, r| 4 + r * 4 + c)],
        ),
        case(
            "row_major_mat2",
            "uint mk; layout(row_major) mat2 m;",
            "float(mk) + m[0][0] + m[1][1]",
            "m[1][0]",
            (4 + 1) * 4,
            12,
            vec![],
            vec![matrix(2, 2, 2, |c, r| 4 + r * 4 + c)],
        ),
        /* column_major mat3: a column strided to 4 dwords, as std140 has it */
        case(
            "column_major_mat3",
            "uint mk; layout(column_major) mat3 m;",
            "float(mk) + m[0][0] + m[2][2]",
            "m[2][1]",
            (4 + 2 * 4 + 1) * 4,
            16,
            vec![],
            vec![matrix(3, 3, 4, |c, r| 4 + c * 4 + r)],
        ),
        /* arrays: std140 stride 16 bytes per element */
        case(
            "float_array_3",
            "uint mk; float f[3];",
            "float(mk) + f[0] + f[2]",
            "f[2]",
            (4 + 2 * 4) * 4,
            16,
            vec![],
            vec![vec![pv(4), pv(8), pv(12)]],
        ),
        case(
            "vec2_array_3",
            "uint mk; vec2 v[3];",
            "float(mk) + v[0].x + v[2].y",
            "v[2].y",
            (4 + 2 * 4 + 1) * 4,
            16,
            vec![],
            vec![vec![pv(4), pv(5), pv(8), pv(9), pv(12), pv(13)]],
        ),
        /* bool is 0 / 1 in the stage array; ints and uints are copied */
        case(
            "bool_bvec3",
            "uint mk; bool b; bvec3 bv;",
            "float(mk) + float(b) + float(bv.z)",
            "float(bv.z)",
            (4 + 2) * 4,
            8,
            vec![(1, 7), (4, 0), (5, 9), (6, 3)],
            vec![vec![Some(1)], vec![Some(0), Some(1), Some(1)]],
        ),
        case(
            "bvec4",
            "uint mk; bvec4 bv;",
            "float(mk) + float(bv.x) + float(bv.w)",
            "float(bv.w)",
            (4 + 3) * 4,
            8,
            vec![(4, 7), (5, 0), (6, 9), (7, 3)],
            vec![vec![Some(1), Some(0), Some(1), Some(1)]],
        ),
        case(
            "int_ivec3_uint",
            "uint mk; int i; ivec3 iv; uint u;",
            "float(mk) + float(i) + float(iv.y) + float(u)",
            "float(iv.y)",
            (4 + 1) * 4,
            8,
            vec![],
            vec![vec![pv(1)], vec![pv(4), pv(5), pv(6)], vec![pv(7)]],
        ),
        /* a struct member: std140 offsets a 16, b 28, c 32; size rounded to 16 */
        case(
            "struct_member",
            "uint mk; S s;",
            "float(mk) + s.a.x + s.b + float(s.c.y)",
            "s.b",
            (4 + 3) * 4,
            12,
            vec![],
            vec![vec![pv(4), pv(5), pv(6)], vec![pv(7)], vec![pv(8), pv(9)]],
        ),
    ]
}

fn data_of(c: &Case) -> Vec<u32> {
    let mut d: Vec<u32> = (0..c.dwords).map(pat).collect();
    for &(k, v) in &c.data {
        d[k] = v;
    }
    d
}

/// The position of `run` in `staged` at or after `from`, if all of it is there.
fn find_run(staged: &[u32], from: usize, run: &[Option<u32>]) -> Option<usize> {
    (from..staged.len().saturating_sub(run.len() - 1))
        .find(|&s| run.iter().enumerate().all(|(k, v)| v.is_none_or(|v| staged[s + k] == v)))
}

/* ---- live: what passes on today's tree -------------------------------------- */

#[test]
fn every_shape_compiles_and_links() {
    let _g = context();
    for c in cases() {
        link(&vs(c.members, c.expr)).unwrap_or_else(|e| panic!("{}: {e}", c.name));
    }
}

#[test]
fn the_stage_array_holds_the_rewritten_dwords() {
    let _g = context();
    for c in cases() {
        let staged = draw_block(&vs(c.members, c.expr), &data_of(&c)).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        // The marker is a uint: copied as it is.
        let mut at =
            staged.iter().position(|&w| w == pat(0)).unwrap_or_else(|| panic!("{}: no marker in {staged:x?}", c.name));
        at += 1;
        for (n, run) in c.runs.iter().enumerate() {
            let s = find_run(&staged, at, run)
                .unwrap_or_else(|| panic!("{}: run {n} {run:x?} not found after dword {at} in {staged:x?}", c.name));
            at = s + run.len();
        }
    }
}

/* ---- the member is read by ld ------------------------------------------------- */

/// The vertex listing has an `ld` followed by a `wdf` (the wait on the load's
/// result, before the destination is read). The `ld`'s address is the pair
/// at the record's `ubo_addr_off1 - 1` plus a constant byte offset whose
/// window covers the element's place in the stage array the `ld` reads:
/// where the draw stages the element's pattern value (std140 dword
/// `ld_offset / 4`; the core packs arrays and transposes a row_major
/// matrix). A bool case's values are normalised, so it takes `ld_offset`,
/// which its staged rows keep.
fn ld_then_wdf(c: &Case) {
    let _g = context();
    let text = vs(c.members, c.ld_expr);
    let off = if c.data.is_empty() {
        let staged = draw_block(&text, &data_of(c)).unwrap_or_else(|e| panic!("{}: {e}", c.name));
        let k = staged
            .iter()
            .position(|&w| w == pat(c.ld_offset as usize / 4))
            .unwrap_or_else(|| panic!("{}: std140 dword {} not staged in {staged:x?}", c.name, c.ld_offset / 4));
        4 * k as u32
    } else {
        c.ld_offset
    };
    let (l, off1) = listing(&text).unwrap_or_else(|e| panic!("{}: {e}", c.name));
    let at = |op| l.lines().position(|x| is_op(x, op));
    let ld = at("ld").unwrap_or_else(|| panic!("{}: no ld:\n{l}", c.name));
    let wdf = at("wdf").unwrap_or_else(|| panic!("{}: no wdf:\n{l}", c.name));
    assert!(ld < wdf, "{}: wdf before the ld:\n{l}", c.name);
    assert_ne!(off1, 0, "{}: the record names no address pair", c.name);
    assert_ld_window(&l, off1 - 1, off, c.name);
}

macro_rules! ld_case {
    ($name:ident, $case:expr) => {
        #[test]
        fn $name() {
            let c = cases().into_iter().find(|c| c.name == $case).expect("a known case");
            ld_then_wdf(&c);
        }
    };
}
ld_case!(ld_row_major_mat4, "row_major_mat4");
ld_case!(ld_row_major_mat3, "row_major_mat3");
ld_case!(ld_row_major_mat2x3, "row_major_mat2x3");
ld_case!(ld_row_major_mat2, "row_major_mat2");
ld_case!(ld_column_major_mat3, "column_major_mat3");
ld_case!(ld_float_array_3, "float_array_3");
ld_case!(ld_vec2_array_3, "vec2_array_3");
ld_case!(ld_bool_bvec3, "bool_bvec3");
ld_case!(ld_bvec4, "bvec4");
ld_case!(ld_int_ivec3_uint, "int_ivec3_uint");
ld_case!(ld_struct_member, "struct_member");
