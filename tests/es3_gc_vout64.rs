// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The GC7000UL's (i.MX8MP) vertex output: the 15-vector case ES 3.0 asks of
//! GL_MAX_VARYING_VECTORS today, and the 64-component case it asks of
//! GL_MAX_VERTEX_OUTPUT_COMPONENTS (reported 60, MAX_VARYING_VECTORS * 4 in
//! gl/context.rs).
//!
//! Live half: a `#version 300 es` program of fifteen vec4 varyings (three of
//! them flat) beside gl_Position links and draws through the GL and the real
//! Vivante back end over a stand-in OSAL. The back end takes the draw (its
//! `draw` returns 0, a DRAW_INSTANCED reaches the submitted stream) and
//! programs VS_OUTPUT_COUNT 16 when the fragment shader reads all fifteen:
//! position and one output per varying it reads (vivante/shader.rs).
//!
//! There is no GC7000 to read pixels back from on the host, so the pixels
//! are the interpreter's (`tests/common`, which runs the ENCODED
//! instructions) on the very records the draw was handed: the vertex
//! program runs for the three vertices with each attribute in the register
//! its record names, its outputs are checked against the shader's own
//! arithmetic, and the fragment program runs at the triangle's centre with
//! each input register the VS output `link()` paired it with
//! (`vs_reg_of_in`), smooth ones at the mean of the three vertices and flat
//! ones at the last (provoking) vertex. The colour register it leaves is the
//! pixel read back. That checks the compiler's registers and the two
//! programs' agreement, not the rasteriser's iterators (a board's).
//!
//! Ignored half: GL_MAX_VERTEX_OUTPUT_COMPONENTS is 64 and a vertex shader
//! writing 64 output components -- fifteen vec4, one vec3 and gl_PointSize,
//! gl_Position not counted -- links and draws. Plan #359's gc-vout64-study
//! found 64 reachable (position is not an `out[]` row, so position plus
//! fifteen varyings, or sixteen hardware varyings); only the cap is 60.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_gc_vout64
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_gc_vout64 -- --ignored
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", not(feature = "report-es2")))]

mod common;

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};

use common::{run, Run};
use yakogl::backend::{self, GlesDraw, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::{glFinish, glGetError};
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::online::vivante::isa::Instr;
use yakogl::shaderc::online::vivante::{decode, listing};
use yakogl::shaderc::{ShcShader, ShcVivIo, SHC_VIV_NO_REG};
use yakogl::vivante::backend::{gc_init, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::{GC_OP_DRAW_INSTANCED, GC_OP_LOAD_STATE, GC_OP_STALL, GC_VS_OUTPUT_COUNT};

/// ES 3.0's table 6.31: fifteen vec4 varyings.
const N: usize = 15;
/// The varyings qualified `flat` in the live program.
const FLAT: [usize; 3] = [4, 9, 14];

/* ---- the program ----------------------------------------------------------------- */

/// A vertex shader writing `n` varyings `v0..v<n>` (vec4, the last one
/// `last_width` wide; each a different swizzle of one attribute, scaled and
/// offset, so no two are the same value and all stay inside [0, 1)), and a
/// fragment shader that reads the first `reads` of them and outputs `v<pick>`
/// (or a sum when `pick` is None). `point_size` adds a gl_PointSize write.
fn sources(n: usize, last_width: usize, reads: usize, pick: Option<usize>, point_size: bool) -> (String, String) {
    let ty = |i: usize| if i + 1 == n && last_width < 4 { format!("vec{last_width}") } else { "vec4".into() };
    let mut vs = String::from("#version 300 es\nin highp vec4 a_pos;\nin highp vec4 a_c;\n");
    let mut fs = String::from("#version 300 es\nprecision highp float;\nlayout(location = 0) out highp vec4 o;\n");
    for i in 0..n {
        let q = if FLAT.contains(&i) { "flat " } else { "" };
        vs += &format!("{q}out highp {} v{i};\n", ty(i));
        if i < reads {
            fs += &format!("{q}in highp {} v{i};\n", ty(i));
        }
    }
    vs += "void main() {\n    gl_Position = a_pos;\n";
    if point_size {
        vs += "    gl_PointSize = 1.0;\n";
    }
    for i in 0..n {
        let sw = ["xyzw", "yzwx", "zwxy", "wxyz"][i % 4];
        let sw = if ty(i) == "vec4" { sw } else { &sw[..last_width] };
        vs += &format!("    v{i} = a_c.{sw} * 0.5 + {:.6};\n", i as f32 / 40.0);
    }
    vs += "}\n";
    fs += "void main() {\n";
    match pick {
        Some(k) => fs += &format!("    o = v{k};\n"),
        None => {
            fs += "    highp vec4 s = vec4(0.0);\n";
            for i in 0..reads {
                fs += &format!("    s += v{i};\n");
            }
            fs += "    o = s * 0.0625;\n";
        }
    }
    fs += "}\n";
    (vs, fs)
}

/// The three vertices: positions (a triangle that covers the 8 x 8 target)
/// and the attribute each varying is made of.
static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static COL: [f32; 12] = [0.10, 0.20, 0.30, 0.40, 0.50, 0.60, 0.70, 0.80, 0.90, 0.15, 0.25, 0.35];

/// What the shader computes for varying `i`, component `c`, at vertex `j`.
fn vertex_value(i: usize, c: usize, j: usize) -> f32 {
    let sw = [[0, 1, 2, 3], [1, 2, 3, 0], [2, 3, 0, 1], [3, 0, 1, 2]][i % 4];
    COL[4 * j + sw[c]] * 0.5 + i as f32 / 40.0
}

/// What the fragment shader receives of varying `i`, component `c`, at the
/// triangle's centre: the last vertex's value when flat, else the mean.
fn centre_value(i: usize, c: usize) -> f32 {
    if FLAT.contains(&i) {
        vertex_value(i, c, 2)
    } else {
        (0..3).map(|j| vertex_value(i, c, j)).sum::<f32>() / 3.0
    }
}

/* ---- the stand-in OSAL: host memory, every stream kept ------------------------------ */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    streams: Vec<Vec<u32>>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    if size == 0 || size > 1 << 30 {
        return -12;
    }
    with_platform(|p| {
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let at = p.bump.entry(heap_base).or_insert(0);
        let gpu = heap_base + 0x1000 + *at;
        *at += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes a live descriptor.
    let m = unsafe { *mem };
    /* leaked, not dropped: a static of the library may still name the block */
    with_platform(|p| p.allocs.remove(&m.gpu).map(core::mem::forget));
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            return -22;
        }
        let Some((&base, a)) = p.allocs.range(..=k.stream_gpu).next_back() else { return -22 };
        let (at, n) = ((k.stream_gpu - base) as usize, k.stream_bytes as usize);
        if at + n > a.len() * 8 {
            return -22;
        }
        // SAFETY: the stand-in's own allocation, read as u32 words inside its size.
        let words = unsafe { core::slice::from_raw_parts((a.as_ptr() as *const u8).add(at) as *const u32, n / 4) };
        p.streams.push(words.to_vec());
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{OSAL_QUERY_DRAW_STATS, OSAL_QUERY_GC_FRAME_OPEN};
    if (what == OSAL_QUERY_GC_FRAME_OPEN && out_sz == 4 && !out.is_null()) || what == OSAL_QUERY_DRAW_STATS {
        return 0;
    }
    -22
}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache),
    cache_invd: Some(p_cache),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// `(register, value)` of every LOAD_STATE write and the DRAW_INSTANCED
/// count of one stream.
fn walk(words: &[u32]) -> (Vec<(u32, u32)>, u32) {
    let (mut out, mut draws, mut i) = (Vec::new(), 0, 0usize);
    while i < words.len() {
        let w = words[i];
        match w & 0xf800_0000 {
            GC_OP_LOAD_STATE => {
                let count = ((w >> 16) & 0x3ff) as usize;
                let base = (w & 0xffff) * 4;
                for (k, &v) in words.iter().skip(i + 1).take(count).enumerate() {
                    out.push((base + 4 * k as u32, v));
                }
                i += 1 + count;
            }
            GC_OP_DRAW_INSTANCED => {
                draws += 1;
                i += 4;
            }
            GC_OP_STALL => i += 2,
            _ => i += 1,
        }
    }
    (out, draws)
}

/* ---- the interpreter, on the records the draw was handed ---------------------------- */

fn io_name(io: &ShcVivIo) -> String {
    io.name().iter().map(|c| *c as u8 as char).collect()
}

/// One stage's code, decoded, and its constant file (literals only: these
/// programs have no uniforms).
fn decoded(s: &ShcShader) -> (Vec<Instr>, Vec<[u32; 4]>) {
    // SAFETY: the record's code, `usc_dw` dwords, alive during the draw.
    let words = unsafe { core::slice::from_raw_parts(s.usc, s.usc_dw as usize) };
    let code = words.chunks(4).map(|c| decode::decode_instr(&c.try_into().unwrap()).expect("decodes")).collect();
    let mut consts = vec![[0u32; 4]; 512];
    for (k, (&kind, &data)) in s.viv.const_contents().iter().zip(s.viv.const_data()).enumerate() {
        consts[k / 4][k % 4] = if kind == yakogl::shaderc::SHC_VIV_CONST_LITERAL { data } else { 0 };
    }
    (code, consts)
}

fn listing_of(code: &[Instr]) -> String {
    let mut t = String::new();
    for (i, ins) in code.iter().enumerate() {
        listing::instr_text(&mut t, i, ins).unwrap();
    }
    t
}

fn interpret(code: &[Instr], regs: &[[u32; 4]], consts: &[[u32; 4]]) -> Result<Vec<[u32; 4]>, String> {
    match run(code, regs, consts) {
        Run::Done { regs, .. } => Ok(regs),
        Run::Unsupported(why) => Err(format!("the interpreter cannot run it: {why}\n{}", listing_of(code))),
    }
}

/// What one draw handed the back end, and what the interpreter made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// The back end's `draw` return code.
    rc: i32,
    vs_outputs: usize,
    fs_inputs: usize,
    /// The vertex program's output registers of the three vertices, by
    /// varying name, and its position.
    vs_out: Vec<BTreeMap<String, [f32; 4]>>,
    vs_pos: Vec<[f32; 4]>,
    /// The fragment program's colour register at the centre.
    pixel: Result<[f32; 4], String>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

/// The vertex outputs by name and the position of each vertex, and the pixel.
type Modelled = (Vec<BTreeMap<String, [f32; 4]>>, Vec<[f32; 4]>, [f32; 4]);

fn model(vs: &ShcShader, fs: &ShcShader) -> Result<Modelled, String> {
    let (vcode, vconsts) = decoded(vs);
    let reg_of = |name: &str| {
        vs.viv.in_[..vs.viv.num_in as usize]
            .iter()
            .find(|x| io_name(x) == name)
            .map(|x| x.reg as usize)
            .ok_or_else(|| format!("the vertex record has no input {name}"))
    };
    let (pos_reg, col_reg) = (reg_of("a_pos")?, reg_of("a_c")?);
    let mut outs = Vec::new();
    let mut by_reg: Vec<BTreeMap<u8, [u32; 4]>> = Vec::new();
    let mut positions = Vec::new();
    for j in 0..3 {
        let mut regs = vec![[0u32; 4]; 128];
        regs[pos_reg] = core::array::from_fn(|c| POS[4 * j + c].to_bits());
        regs[col_reg] = core::array::from_fn(|c| COL[4 * j + c].to_bits());
        let r = interpret(&vcode, &regs, &vconsts)?;
        let mut named = BTreeMap::new();
        let mut regs_out = BTreeMap::new();
        for o in &vs.viv.out[..vs.viv.num_out as usize] {
            named.insert(io_name(o), r[o.reg as usize].map(f32::from_bits));
            regs_out.insert(o.reg, r[o.reg as usize]);
        }
        positions.push(r[vs.viv.vs_pos_out_reg as usize].map(f32::from_bits));
        outs.push(named);
        by_reg.push(regs_out);
    }
    /* the fragment program at the centre: r0 is gl_FragCoord, each input
     * register the VS output link() paired it with */
    let (fcode, fconsts) = decoded(fs);
    let mut regs = vec![[0u32; 4]; 128];
    regs[0] = [4.5f32, 4.5, 0.5, 1.0].map(f32::to_bits);
    for (i, inp) in fs.viv.in_[..fs.viv.num_in as usize].iter().enumerate() {
        let vr = fs.viv.vs_reg_of_in[i];
        if vr == SHC_VIV_NO_REG {
            return Err(format!("fragment input {} has no vertex output", io_name(inp)));
        }
        let at = |j: usize, c: usize| f32::from_bits(by_reg[j].get(&vr).map_or(0, |v| v[c]));
        regs[inp.reg as usize] = core::array::from_fn(|c| {
            if inp.interp == 2 { at(2, c) } else { (at(0, c) + at(1, c) + at(2, c)) / 3.0 }.to_bits()
        });
    }
    let r = interpret(&fcode, &regs, &fconsts)?;
    Ok((outs, positions, r[fs.viv.ps_color_out_reg as usize].map(f32::from_bits)))
}

unsafe extern "C" fn t_draw(d: *const GlesDraw) -> i32 {
    let real = yakogl_backend_vivante.draw.expect("the GC7000 back end draws");
    // SAFETY: the GL core's draw, handed on as it came.
    let rc = unsafe { real(d) };
    // SAFETY: as above; the records live for the call.
    let d = unsafe { &*d };
    if d.index_count == 0 || d.vs.is_null() || d.fs.is_null() {
        return rc;
    }
    // SAFETY: non-null records of the program drawn.
    let (vs, fs) = unsafe { (&*d.vs, &*d.fs) };
    let m = model(vs, fs);
    let (vs_out, vs_pos, pixel) = match m {
        Ok((o, p, px)) => (o, p, Ok(px)),
        Err(e) => (Vec::new(), Vec::new(), Err(e)),
    };
    SEEN.lock().unwrap().push(Seen {
        rc,
        vs_outputs: vs.viv.num_out as usize,
        fs_inputs: fs.viv.num_in as usize,
        vs_out,
        vs_pos,
        pixel,
    });
    rc
}

/// A copy of the Vivante table whose `draw` runs the interpreter as well.
fn table() -> *const YakoglBackend {
    static TABLE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *TABLE.get_or_init(|| {
        // SAFETY: the table is plain data (pointers, function pointers and
        // integers, no Drop); the copy is a second table of the same entries.
        let mut t: YakoglBackend = unsafe { core::ptr::read(&yakogl_backend_vivante) };
        t.name = c"es3-gc-vout64".as_ptr() as *const c_char;
        t.draw = Some(t_draw);
        Box::leak(Box::new(t)) as *const YakoglBackend as usize
    }) as *const YakoglBackend
}

/* ---- the GL ------------------------------------------------------------------------- */

/// One context for the whole file; the tests take turns on it and on the
/// compiler (one arena, not reentrant).
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = table();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(gc_init(), 0, "gc_init");
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    SEEN.lock().unwrap().clear();
    with_platform(|p| p.streams.clear());
    g
}

fn geti(p: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: plain GL call with a real output.
    unsafe { yakogl::gl::state::glGetIntegerv(p, &mut v) };
    v
}

fn text_of(log: &[c_char]) -> String {
    log.iter().take_while(|c| **c != 0).map(|c| *c as u8 as char).collect()
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
            panic!("did not compile: {}\n{text:?}", text_of(&log));
        }
        s
    }
}

/// The two sources linked: Ok(program) or Err(info log).
fn program((vs, fs): &(String, String)) -> Result<GLuint, String> {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glBindAttribLocation(p, 1, c"a_c".as_ptr());
        glLinkProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, log.len() as GLsizei, core::ptr::null_mut(), log.as_mut_ptr());
        glDeleteProgram(p);
        Err(text_of(&log))
    }
}

/// Draws the triangle with `p` and deletes it; gives what the back end saw
/// and the VS_OUTPUT_COUNT values and draw count of the streams submitted.
fn draw(p: GLuint) -> (Seen, Vec<u32>, u32) {
    // SAFETY: the arrays are statics and outlive the draw.
    unsafe {
        glUseProgram(p);
        yakogl::gl::buffer::glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, COL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "the draw");
        glUseProgram(0);
        glDeleteProgram(p);
    }
    let mut seen = SEEN.lock().unwrap();
    assert_eq!(seen.len(), 1, "the back end saw one draw");
    let s = seen.pop().unwrap();
    let (counts, draws) = with_platform(|pl| {
        let mut counts = Vec::new();
        let mut draws = 0;
        for st in core::mem::take(&mut pl.streams) {
            let (w, d) = walk(&st);
            counts.extend(w.iter().filter(|(r, _)| *r == GC_VS_OUTPUT_COUNT).map(|(_, v)| *v));
            draws += d;
        }
        (counts, draws)
    });
    (s, counts, draws)
}

/// The back end took the draw: `draw` returned 0, the stream drew, and its
/// last VS_OUTPUT_COUNT is `outputs`.
fn assert_drawn(what: &str, seen: &Seen, counts: &[u32], draws: u32, outputs: u32) {
    assert_eq!(seen.rc, 0, "{what}: the back end refused the draw");
    assert!(draws >= 1, "{what}: no DRAW_INSTANCED reached the submitted streams");
    assert_eq!(counts.last().copied(), Some(outputs), "{what}: VS_OUTPUT_COUNT ({counts:?})");
}

/* ---- the live half ------------------------------------------------------------------ */

/// Fifteen vec4 varyings (three flat): one program per varying read back,
/// each drawn on its own. The vertex program writes all fifteen, the back
/// end programs position and the one read; every vertex output and every
/// pixel is held to the arithmetic.
#[test]
fn fifteen_vec4_varyings_draw_and_read_back() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VARYING_VECTORS), N as GLint, "ES 3.0 table 6.31");
    let refused_before = backend::draws_refused();
    for k in 0..N {
        let p = program(&sources(N, 4, N, Some(k), false))
            .unwrap_or_else(|e| panic!("v{k}: fifteen vec4 do not link: {e}"));
        let (seen, counts, draws) = draw(p);
        /* position is vs_pos_out_reg, not an out[] row */
        assert_eq!(seen.vs_outputs, N, "v{k}: the vertex record lists the fifteen");
        assert_eq!(seen.fs_inputs, 1, "v{k}: the fragment record lists the one it reads");
        /* the back end hands on what the fragment shader reads: position and v<k> */
        assert_drawn(&format!("v{k}"), &seen, &counts, draws, 2);

        let px = seen.pixel.clone().unwrap_or_else(|e| panic!("v{k}: {e}"));
        for (j, pos) in seen.vs_pos.iter().enumerate() {
            for c in [0, 1, 3] {
                assert_eq!(pos[c], POS[4 * j + c], "v{k}: gl_Position[{c}] of vertex {j}");
            }
        }
        for (j, out) in seen.vs_out.iter().enumerate() {
            for i in 0..N {
                let got = out.get(&format!("v{i}")).unwrap_or_else(|| panic!("v{k}: no output v{i}"));
                for (c, got) in got.iter().enumerate() {
                    let want = vertex_value(i, c, j);
                    assert!((got - want).abs() < 1e-6, "v{k}: vertex {j} v{i}[{c}] = {got}, want {want}");
                }
            }
        }
        for (c, px) in px.iter().enumerate() {
            let want = centre_value(k, c);
            let flat = if FLAT.contains(&k) { " (flat)" } else { "" };
            assert!((px - want).abs() < 1e-5, "v{k}{flat}[{c}]: pixel {px} vs {want}");
        }
    }
    assert_eq!(backend::draws_refused(), refused_before, "no draw was refused");
}

/// All fifteen read in one fragment shader (the case a draw really has):
/// fifteen inputs, and the sum of all of them reaches the pixel.
#[test]
fn fifteen_vec4_varyings_read_by_one_fragment_shader() {
    let _g = context();
    let p = program(&sources(N, 4, N, None, false)).unwrap_or_else(|e| panic!("fifteen vec4 do not link: {e}"));
    let (seen, counts, draws) = draw(p);
    assert_drawn("the sum", &seen, &counts, draws, 1 + N as u32);
    assert_eq!((seen.vs_outputs, seen.fs_inputs), (N, N), "both records list the fifteen");
    let px = seen.pixel.expect("the interpreter runs both programs");
    for (c, px) in px.iter().enumerate() {
        let want = (0..N).map(|i| centre_value(i, c)).sum::<f32>() * 0.0625;
        assert!((px - want).abs() < 1e-5, "sum[{c}]: pixel {px} vs {want}");
    }
}

/* ---- the 64-component half ---------------------------------------------------------- */

/// The raise to GL_MAX_VERTEX_OUTPUT_COMPONENTS 64: the limit reads 64 and a
/// vertex shader writing 64 output components (fifteen vec4, a vec3 and
/// gl_PointSize; gl_Position besides) links and draws, the fragment shader
/// reading fourteen vec4 and the vec3 (59 of GL_MAX_FRAGMENT_INPUT_COMPONENTS'
/// 60). The study (plan #359 gc-vout64-study) found nothing in the hardware
/// against it: position is not an `out[]` row. Past the 60, the vertex
/// shader is refused today at compile time, before the link: `Abi(
/// LoadBalancingTempsUnmeasured { num_out: 16, temps: 18, temps_max: 16 })`,
/// so the raise also needs a VS_LOAD_BALANCING entry for sixteen outputs.
#[test]
#[ignore = "needs the GC raise to MAX_VERTEX_OUTPUT_COMPONENTS 64"]
fn sixty_four_vertex_output_components_link_and_draw() {
    let _g = context();
    assert_eq!(geti(GL_MAX_VERTEX_OUTPUT_COMPONENTS), 64, "ES 3.0 table 6.31 asks 64");
    let (n, last) = (N + 1, 3);
    let (vs, _) = sources(n, last, 0, None, true);
    /* the fragment shader: v0..v13 and v15, summed */
    let (_, fs) = sources(n, last, n, None, false);
    let fs = fs
        .replace("flat in highp vec4 v14;\n", "")
        .replace("    s += v14;\n", "")
        .replace("    s += v15;\n", "    s += vec4(v15, 0.0);\n");
    let refused_before = backend::draws_refused();
    let p = program(&(vs, fs)).unwrap_or_else(|e| panic!("64 vertex output components do not link: {e}"));
    let (seen, counts, draws) = draw(p);
    /* position, the fifteen the fragment shader reads and the point size */
    assert_drawn("64 components", &seen, &counts, draws, 2 + N as u32);
    assert_eq!(backend::draws_refused(), refused_before, "the draw was not refused");
    assert_eq!(seen.fs_inputs, N, "the fragment record lists the fifteen it reads");
    let px = seen.pixel.expect("the interpreter runs both programs");
    for (c, px) in px.iter().enumerate() {
        let want = (0..n).filter(|i| *i != 14).map(|i| if i == 15 && c == 3 { 0.0 } else { centre_value(i, c) });
        let want = want.sum::<f32>() * 0.0625;
        assert!((px - want).abs() < 1e-5, "sum[{c}]: pixel {px} vs {want}");
    }
}
