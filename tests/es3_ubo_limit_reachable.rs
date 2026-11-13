// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The reported `GL_MAX_UNIFORM_BLOCK_SIZE` (S) is a block size the stack can
//! really link and read, on every GPU build, whatever S is: a test of the
//! limit being honest, not of its value (`tests/es3_caps.rs` holds the ES 3.0
//! minimum, 16384, and `known_gaps()` the GPUs below it).
//!
//! - A std140 block of exactly S bytes (`vec4 v[S/16]`) links when the VS
//!   reads it and when the FS reads it, and a draw hands the back end the
//!   first, the middle and the LAST vec4 with the values the application
//!   uploaded, one draw per entry through a dynamic index (`uniform int`).
//!   The host cannot render, so "reads back" is what `tests/es3_ubo.rs`
//!   measures: the stage's `v` row of the draw's uniform block, as handed to
//!   the back end, and the `u_i` row the index is read from.
//! - A block of S + 16 bytes (one more vec4) does not link, and says why.
//! - `GL_MAX_COMBINED_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` are at least
//!   `blocks * S / 4 + MAX_{VERTEX,FRAGMENT}_UNIFORM_COMPONENTS` (ES 3.0
//!   6.2 / table 6.32: the minimum is that sum, an implementation may report
//!   more).
//!
//! The file needs no edit when a limit is raised. Five checks are `#[ignore]`d
//! with their reasons (`-- --ignored` runs them): the single-array form, the
//! dynamic index, the over-limit link refusal and the combined-components
//! formula; each fails today for the reason in its attribute.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_ubo_limit_reachable
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_ubo_limit_reachable
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_ubo_limit_reachable
//! ```
//!
//! The Mali build is not covered: its Valhall back end refuses a uniform block
//! of more than 256 rows and a mixed fp16/fp32 fragment shader today (the
//! stage-5 plans own that; its limits in `gl/context.rs` stay as they are).
//!
//! An ES 2.0-reporting build (`report-es2`) has no uniform blocks and runs
//! nothing here.

#![cfg(all(feature = "host-test", feature = "shaderc-online", not(feature = "report-es2"), not(feature = "gpu-mali")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
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

/// One stage as a draw handed it over: the compiled shader's uniform rows
/// (name, dword offset, dwords) and the block's dwords.
#[derive(Clone, Default)]
struct Stage {
    rows: Vec<(String, usize, usize)>,
    block: Vec<u32>,
}

impl Stage {
    /// The dwords of the row `name`, or why there are none.
    fn row(&self, name: &str) -> Result<&[u32], String> {
        let Some(&(_, off, dw)) = self.rows.iter().find(|r| r.0 == name) else {
            let names: Vec<&str> = self.rows.iter().map(|r| r.0.as_str()).collect();
            return Err(format!("no row '{name}' in the compiled stage (rows: {names:?})"));
        };
        self.block.get(off..off + dw).ok_or(format!(
            "row '{name}' [{off}, {}) is past the block of {} dwords",
            off + dw,
            self.block.len()
        ))
    }
}

/// The last draw's two stages.
static DRAWN: Mutex<Option<(Stage, Stage)>> = Mutex::new(None);

unsafe fn stage_of(sh: *const yakogl::shaderc::ShcShader, cpu: *const c_void, dwords: u16) -> Stage {
    let Some(sh) = sh.as_ref() else { return Stage::default() };
    let rows = sh
        .uniform_table()
        .iter()
        .map(|u| {
            let n: Vec<u8> = u.name.iter().take_while(|c| **c != 0).map(|c| *c as u8).collect();
            (String::from_utf8_lossy(&n).into_owned(), usize::from(u.dword_off), usize::from(u.dwords))
        })
        .collect();
    let block = if cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(cpu as *const u32, usize::from(dwords)).to_vec()
    };
    Stage { rows, block }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let vs = stage_of(d.vs, d.vs_uniform_cpu, d.vs_uniform_dwords);
    let fs = stage_of(d.fs, d.fs_uniform_cpu, d.fs_uniform_dwords);
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some((vs, fs));
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
    name: c"es3-ubo-limit-reachable".as_ptr() as *const c_char,
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

fn geti(pname: GLenum) -> i64 {
    let mut v: GLint = i32::MIN;
    // SAFETY: plain GL call, one value.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glGetIntegerv(pname, &mut v);
        assert_eq!(glGetError(), GL_NO_ERROR, "glGetIntegerv(0x{pname:x}) raised an error");
    }
    assert_ne!(v, i32::MIN, "glGetIntegerv(0x{pname:x}) wrote nothing");
    i64::from(v)
}

/* ---- the shaders: a block of `entries` vec4s read by one stage ----------- */

#[derive(Clone, Copy, PartialEq, Debug)]
enum Reader {
    Vertex,
    Fragment,
}

/// The value of entry `k`, component `c`: distinct per entry and component,
/// exact in a float.
fn value(k: usize, c: usize) -> f32 {
    (k * 4 + c) as f32 + 0.5
}

/// How the block's vec4s are declared: `Chunked` as arrays of at most
/// `CHUNK` entries (`v0`, `v1`, ...), `Single` as the one array `v`.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Shape {
    Chunked,
    Single,
}

/// An array the reflection can record: its length is a `u8` there
/// (`fit_u8` in `src/shaderc/online/reflect.rs`), so 255 at most.
const CHUNK: usize = 128;

/// The arrays of a block of `entries` vec4s: (name, first entry, length).
fn arrays(shape: Shape, entries: usize) -> Vec<(String, usize, usize)> {
    match shape {
        Shape::Single => vec![("v".to_string(), 0, entries)],
        Shape::Chunked => {
            (0..entries.div_ceil(CHUNK)).map(|c| (format!("v{c}"), c * CHUNK, CHUNK.min(entries - c * CHUNK))).collect()
        }
    }
}

/// How the reading stage picks its entries.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Access {
    /// The first, the middle and the last entry by constant index.
    Constant,
    /// The entry `u_i` names, a dynamic index (the draws set it to the first,
    /// the middle and the last entry).
    Dynamic,
}

/// The entries a draw is checked on: the first, the middle and the last.
fn probes(entries: usize) -> [usize; 3] {
    [0, entries / 2, entries - 1]
}

/// The array of `arrs` that holds entry `k`: (name, first entry, length).
fn holder(arrs: &[(String, usize, usize)], k: usize) -> (String, usize, usize) {
    arrs.iter().find(|(_, first, len)| k >= *first && k < first + len).cloned().unwrap()
}

/// The reading stage returns an expression of the block's entries; the other
/// stage is a pass-through that declares nothing of the block.
fn sources(reader: Reader, shape: Shape, access: Access, entries: usize) -> (String, String) {
    let arrs = arrays(shape, entries);
    let at = |k: usize| {
        let (n, first, _) = holder(&arrs, k);
        format!("{n}[{}]", k - first)
    };
    let (u_i, read) = match access {
        Access::Dynamic => ("uniform highp int u_i;\n", "pick(u_i)".to_string()),
        Access::Constant => {
            let [a, b, c] = probes(entries);
            ("", format!("{} + 2.0 * {} + 3.0 * {}", at(a), at(b), at(c)))
        }
    };
    let members: String = arrs.iter().map(|(n, _, len)| format!("highp vec4 {n}[{len}]; ")).collect();
    let mut pick = String::new();
    if access == Access::Dynamic {
        pick += "highp vec4 pick(int i) {\n";
        for (n, first, len) in &arrs {
            pick += &format!("  if (i < {}) return {n}[i - {first}];\n", first + len);
        }
        pick += "  return vec4(0.0);\n}\n";
    }
    let decl = format!("layout(std140) uniform B {{ {members}}};\n{u_i}{pick}");
    match reader {
        Reader::Vertex => (
            format!(
                "#version 300 es\n{decl}in highp vec4 a_position;\nout highp vec4 v_c;\n\
                 void main() {{ gl_Position = a_position; v_c = {read}; }}\n"
            ),
            "#version 300 es\nin highp vec4 v_c;\nlayout(location = 0) out highp vec4 o;\n\
             void main() { o = v_c; }\n"
                .to_string(),
        ),
        Reader::Fragment => (
            "#version 300 es\nin highp vec4 a_position;\nvoid main() { gl_Position = a_position; }\n".into(),
            format!("#version 300 es\n{decl}layout(location = 0) out highp vec4 o;\nvoid main() {{ o = {read}; }}\n"),
        ),
    }
}

fn shader(ty: GLenum, text: &str) -> Result<GLuint, String> {
    let c = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source and a log buffer.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        if ok == 1 {
            return Ok(s);
        }
        let mut log = [0 as c_char; 1024];
        let mut n: GLsizei = 0;
        glGetShaderInfoLog(s, 1024, &mut n, log.as_mut_ptr());
        glDeleteShader(s);
        Err(core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned())
    }
}

/// Compile and link; `Err` is the compile or link log.
fn build(reader: Reader, shape: Shape, access: Access, entries: usize) -> Result<GLuint, String> {
    let (vs_src, fs_src) = sources(reader, shape, access, entries);
    let vs = shader(GL_VERTEX_SHADER, &vs_src).map_err(|e| format!("vertex compile: {e}"))?;
    let fs = shader(GL_FRAGMENT_SHADER, &fs_src).map_err(|e| format!("fragment compile: {e}"))?;
    // SAFETY: plain GL calls on handles this function owns.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, vs);
        glAttachShader(p, fs);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glLinkProgram(p);
        glDeleteShader(vs);
        glDeleteShader(fs);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, 1024, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteProgram(p);
        while glGetError() != GL_NO_ERROR {}
        let text = core::ffi::CStr::from_ptr(log.as_ptr()).to_string_lossy().into_owned();
        Err(if text.trim().is_empty() { "(empty log)".to_string() } else { text })
    }
}

/// The block size S the build reports, in vec4s.
fn limit_entries() -> usize {
    let s = geti(GL_MAX_UNIFORM_BLOCK_SIZE);
    assert!(s > 0 && s % 16 == 0, "GL_MAX_UNIFORM_BLOCK_SIZE {s} is not a positive multiple of a vec4");
    (s / 16) as usize
}

/// Bind a buffer of every entry to block `B` of `p`, set `u_i` to `index`,
/// draw once, and check that the stage that reads the block was handed its
/// `u_i` and every entry it was uploaded.
///
/// # Safety
/// `p` is a linked program of the current context.
unsafe fn draw_and_read(
    p: GLuint,
    reader: Reader,
    shape: Shape,
    access: Access,
    entries: usize,
    index: usize,
) -> Result<(), String> {
    glUseProgram(p);
    let bi = glGetUniformBlockIndex(p, c"B".as_ptr());
    if bi == GL_INVALID_INDEX {
        return Err("block B is not an active uniform block of the program".into());
    }
    let mut size: GLint = 0;
    glGetActiveUniformBlockiv(p, bi, GL_UNIFORM_BLOCK_DATA_SIZE, &mut size);
    if size as usize != entries * 16 {
        return Err(format!("GL_UNIFORM_BLOCK_DATA_SIZE {size}, expected {}", entries * 16));
    }
    glUniformBlockBinding(p, bi, 0);
    let data: Vec<f32> = (0..entries * 4).map(|i| value(i / 4, i % 4)).collect();
    let mut buf = 0;
    glGenBuffers(1, &mut buf);
    glBindBuffer(GL_UNIFORM_BUFFER, buf);
    glBufferData(GL_UNIFORM_BUFFER, (data.len() * 4) as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
    glBindBufferBase(GL_UNIFORM_BUFFER, 0, buf);
    if access == Access::Dynamic {
        glUniform1i(glGetUniformLocation(p, c"u_i".as_ptr()), index as GLint);
    }
    static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
    static IDX: [u16; 6] = [0, 1, 2, 2, 1, 3];
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
    glEnableVertexAttribArray(0);
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = None;
    glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
    glDisableVertexAttribArray(0);
    glDeleteBuffers(1, &buf);
    let e = glGetError();
    if e != GL_NO_ERROR {
        return Err(format!("the draw raised 0x{e:x}"));
    }
    let Some((vst, fst)) = DRAWN.lock().unwrap_or_else(|e| e.into_inner()).take() else {
        return Err("no draw reached the back end".into());
    };
    let st = if reader == Reader::Vertex { &vst } else { &fst };
    if access == Access::Dynamic {
        let got_i = st.row("u_i")?.first().copied();
        if got_i != Some(index as u32) {
            return Err(format!("row u_i is {got_i:?}, expected {index}"));
        }
    }
    // the first, the middle and the last entry
    let arrs = arrays(shape, entries);
    for k in probes(entries) {
        let (name, first, len) = holder(&arrs, k);
        let part = st.row(&name)?;
        if part.len() != len * 4 {
            return Err(format!("row {name} has {} dwords, expected {}", part.len(), len * 4));
        }
        for c in 0..4 {
            let got = f32::from_bits(part[(k - first) * 4 + c]);
            if got != value(k, c) {
                return Err(format!("v[{k}].{c} reads {got}, the application uploaded {}", value(k, c)));
            }
        }
    }
    Ok(())
}

fn reaches_limit(reader: Reader, shape: Shape, access: Access) {
    let _g = context();
    let entries = limit_entries();
    let p = build(reader, shape, access, entries).unwrap_or_else(|e| {
        panic!("{reader:?}: a {}-byte block (the GL_MAX_UNIFORM_BLOCK_SIZE) does not link: {e}", entries * 16)
    });
    let indices = if access == Access::Dynamic { probes(entries).to_vec() } else { vec![0] };
    for index in indices {
        // SAFETY: p is a linked program of the current context.
        unsafe { draw_and_read(p, reader, shape, access, entries, index) }
            .unwrap_or_else(|e| panic!("{reader:?}: draw (index {index}): {e}"));
    }
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
    }
}

fn over_limit_fails(reader: Reader, shape: Shape) {
    let _g = context();
    let entries = limit_entries() + 1;
    match build(reader, shape, Access::Constant, entries) {
        Ok(_) => {
            panic!("{reader:?}: a {}-byte block (GL_MAX_UNIFORM_BLOCK_SIZE + 16) links; it must not", entries * 16)
        }
        Err(log) => assert!(!log.trim().is_empty(), "the refusal has no log"),
    }
}

#[test]
fn vertex_block_of_the_limit_links_and_reads() {
    reaches_limit(Reader::Vertex, Shape::Chunked, Access::Constant);
}

#[test]
fn fragment_block_of_the_limit_links_and_reads() {
    reaches_limit(Reader::Fragment, Shape::Chunked, Access::Constant);
}

#[test]
#[ignore = "a block of S + 16 bytes links on every GPU build (ES 3.0 6.1.2 / 2.12.6.4: the link must fail); the block size is not checked at link"]
fn vertex_block_over_the_limit_fails_to_link() {
    over_limit_fails(Reader::Vertex, Shape::Chunked);
}

#[test]
#[ignore = "a block of S + 16 bytes links on every GPU build (ES 3.0 6.1.2 / 2.12.6.4: the link must fail); the block size is not checked at link"]
fn fragment_block_over_the_limit_fails_to_link() {
    over_limit_fails(Reader::Fragment, Shape::Chunked);
}

/* The block of the limit as ONE `vec4 v[S/16]` array: `reflect.rs` records an
 * array's length in a u8 (`fit_u8`), so a vec4 array of 256 entries or more is
 * refused at compile ("too many of this kind of declaration") on every GPU
 * build, below the 1024 entries of a 16384-byte block. The chunked tests above
 * reach the same bytes; these two are the single-array form. */

#[test]
#[ignore = "reflect.rs fit_u8: a uniform array longer than 255 entries does not compile, so v[S/16] cannot be declared"]
fn vertex_single_array_of_the_limit_links_and_reads() {
    reaches_limit(Reader::Vertex, Shape::Single, Access::Constant);
}

#[test]
#[ignore = "reflect.rs fit_u8: a uniform array longer than 255 entries does not compile, so v[S/16] cannot be declared"]
fn fragment_single_array_of_the_limit_links_and_reads() {
    reaches_limit(Reader::Fragment, Shape::Single, Access::Constant);
}

/* The dynamic index from `uniform int u_i`. Not reachable today, for a reason
 * of each back end rather than of S:
 *  - V3D: prepare.rs selects a uniform array at a run-time index by loading
 *    every element (`MAX_SELECTED` 256 dwords) and has no ldunifa path, so a
 *    block-sized array is refused ("a uniform array too long to select from").
 *  - PowerVR and GC7000UL: a block of S bytes fills the whole 1536-dword stream
 *    (MAX_UNIFORM_DWORDS), so the default-block `u_i` lands at dword 1536,
 *    past the block the draw hands the back end, and the link does not refuse
 *    it. */

#[test]
#[ignore = "no run-time index into a block-sized uniform array on the V3D (no ldunifa); u_i past the 1536-dword stream on PowerVR/GC7000UL"]
fn vertex_dynamic_index_reads_the_limit_block() {
    reaches_limit(Reader::Vertex, Shape::Chunked, Access::Dynamic);
}

#[test]
#[ignore = "no run-time index into a block-sized uniform array on the V3D (no ldunifa); u_i past the 1536-dword stream on PowerVR/GC7000UL"]
fn fragment_dynamic_index_reads_the_limit_block() {
    reaches_limit(Reader::Fragment, Shape::Chunked, Access::Dynamic);
}

#[test]
#[ignore = "the value is below the ES 3.0 formula: 1536 combined components on every GPU build (state.rs:1268); un-ignore with the raise (ubo-v3d)"]
fn combined_components_follow_the_es3_formula() {
    let _g = context();
    let s = geti(GL_MAX_UNIFORM_BLOCK_SIZE);
    for (blocks, comps, combined, what) in [
        (
            GL_MAX_VERTEX_UNIFORM_BLOCKS,
            GL_MAX_VERTEX_UNIFORM_COMPONENTS,
            GL_MAX_COMBINED_VERTEX_UNIFORM_COMPONENTS,
            "vertex",
        ),
        (
            GL_MAX_FRAGMENT_UNIFORM_BLOCKS,
            GL_MAX_FRAGMENT_UNIFORM_COMPONENTS,
            GL_MAX_COMBINED_FRAGMENT_UNIFORM_COMPONENTS,
            "fragment",
        ),
    ] {
        let (blocks, comps, combined) = (geti(blocks), geti(comps), geti(combined));
        let formula = blocks * s / 4 + comps;
        assert!(
            combined >= formula,
            "{what}: MAX_COMBINED_UNIFORM_COMPONENTS {combined} < blocks {blocks} * S {s} / 4 + MAX_UNIFORM_COMPONENTS {comps} = {formula}"
        );
    }
}
