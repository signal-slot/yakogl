// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! `float`, `int`, `uint` and `vec2` arrays at the end of a 16384-byte
//! std140 uniform block on the V3D (docs/notes/ubo-limits.md section 8):
//! `f[K]`, `i[K]`, `u[K]` and `v[K]` for K = 4..8, behind padding `vec4`
//! arrays (at most 255 entries apiece, `UniformRow::array_len` is a u8) and
//! a closing run of four scalar `float`s, so the block is exactly 16384
//! bytes. In std140 (ES 3.0 2.12.6.4) every element of such an array takes
//! its own 16-byte slot -- `GL_UNIFORM_ARRAY_STRIDE` is 16 for `float`,
//! `int`, `uint` and `vec2` arrays alike -- and only the first 4 (scalar)
//! or 8 (vec2) bytes of the slot are the element's; consecutive scalar
//! `float`s pack at 4-byte alignment, so `t3` sits at byte 16380, the
//! block's last dword.
//!
//! The harness is `tests/v3d_ubo_buffer_range.rs`'s: a program linked
//! through the GL API on a mock back end (`uniforms_host_only: true`, as
//! the V3D's table) whose `draw` keeps the stage block exactly as
//! `fill_uniform_blocks` (src/gl/program.rs) filled it from the bound
//! buffer -- glBufferData and glBindBufferBase included -- and the QPU
//! interpreter then runs the same fragment shader with a uniform stream
//! built from that captured block. No layout is assumed at the compiler
//! level: what is measured is the GL core's std140 read and per-element
//! copy, on the draw path.
//!
//! (1) `GL_UNIFORM_OFFSET` and `GL_UNIFORM_ARRAY_STRIDE` of every member,
//! from glGetActiveUniformsiv, against the std140 offsets and strides, and
//! `GL_UNIFORM_BLOCK_DATA_SIZE` against 16384.
//! (2) The fragment shader reads the first and last element of each array
//! and the block's last dword (the four trailing scalars), 14 reads with
//! constant indexes, summed into `gl_FragDepth`. Each read dword of the
//! buffer holds a distinct power of two (as `int` for the `i` / `u`
//! members, as `float` elsewhere); every other dword holds `j + 0.5`, a
//! marker naming the buffer dword `j` it sits in. The 4096 buffer values
//! are all distinct, and so the power-of-two sum names a wrong read: a
//! different sum says which read landed on which dword.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_ubo_small_arrays
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::collections::HashSet;
use std::ffi::{CStr, CString};
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
use yakogl::shaderc::online::arena::Arena;
use yakogl::shaderc::online::v3d::chain::{compile_program, Prim, Target};
use yakogl::shaderc::online::v3d::select::TLB_Z_PER_PIXEL;
use yakogl::shaderc::online::v3d::sim::{self, Env, FsInputs, Inputs, Varying, LANES};
use yakogl::shaderc::online::{lower, sema};

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

/// The fragment stage's block of the last draw, as handed to the back end.
static DRAWN: Mutex<Option<Vec<u32>>> = Mutex::new(None);

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let block = if d.fs_uniform_cpu.is_null() {
        Vec::new()
    } else {
        core::slice::from_raw_parts(d.fs_uniform_cpu as *const u32, usize::from(d.fs_uniform_dwords)).to_vec()
    };
    *DRAWN.lock().unwrap_or_else(|e| e.into_inner()) = Some(block);
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
    name: c"v3d-ubo-small-arrays".as_ptr() as *const c_char,
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
    /* as the V3D's table: the GL core hands over the host array */
    uniforms_host_only: true,
    ds_write: None,
    stencil_resolve: None,
};

/// One context for the whole file, and the compiler's one arena: the tests
/// take turns.
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

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the block ---------------------------------------------------------------- */

const BLOCK_BYTES: usize = 16384;
const BLOCK_DWORDS: usize = BLOCK_BYTES / 4;

/// The `K` of the four small-element arrays: `f[K]`, `i[K]`, `u[K]`, `v[K]`.
const K_RANGE: [usize; 5] = [4, 5, 6, 7, 8];

/// A `vec4` array holds at most 255 entries (`UniformRow::array_len` is a u8).
const PAD_MAX: usize = 255;

/// The fourth padding array: `K` slots apiece for the four arrays and one
/// slot for the closing scalars leave `1024 - 3 * 255 - 4 * K - 1` for it.
fn pad_d_len(k: usize) -> usize {
    let n = 1024 - 3 * PAD_MAX - 4 * k - 1;
    assert!(n > 0 && n <= PAD_MAX, "padD[{n}] for K = {k}");
    n
}

/// The std140 byte offsets of the members, in declaration order. Every array
/// here (of `vec4`, `float`, `int`, `uint` or `vec2`) is 16-aligned and
/// 16-strided; the four trailing scalars pack at 4 bytes apiece, and `t3` is
/// the block's last dword.
struct Offs {
    pad: [usize; 4],
    f: usize,
    i: usize,
    u: usize,
    v: usize,
    t: usize,
}

fn offs(k: usize) -> Offs {
    let lens = [PAD_MAX, PAD_MAX, PAD_MAX, pad_d_len(k)];
    let (mut slot, mut pad) = (0usize, [0usize; 4]);
    for (p, &n) in pad.iter_mut().zip(&lens) {
        *p = slot * 16;
        slot += n;
    }
    let f = slot * 16;
    slot += k;
    let i = slot * 16;
    slot += k;
    let u = slot * 16;
    slot += k;
    let v = slot * 16;
    slot += k;
    let t = slot * 16;
    slot += 1;
    assert_eq!((slot * 16, t + 16), (BLOCK_BYTES, BLOCK_BYTES), "the block is not exactly {BLOCK_BYTES} for K = {k}");
    Offs { pad, f, i, u, v, t }
}

/// The 14 dwords the fragment shader reads, in expression order:
/// (diagnostic name, buffer dword, the bits stored there, the value once
/// read). Read `e` holds the value 2^e -- as an `int` / `uint` for the
/// `i` and `u` members, as a `float` elsewhere.
fn reads(k: usize) -> Vec<(String, usize, u32, f32)> {
    let o = offs(k);
    let mut out: Vec<(String, usize, u32, f32)> = Vec::new();
    let mut put = |name: String, byte: usize, as_int: bool| {
        let e = out.len() as u32;
        let pow = 2f32.powi(e as i32);
        out.push((name, byte / 4, if as_int { 1u32 << e } else { pow.to_bits() }, pow));
    };
    put("f[0]".into(), o.f, false);
    put(format!("f[{}]", k - 1), o.f + 16 * (k - 1), false);
    put("i[0]".into(), o.i, true);
    put(format!("i[{}]", k - 1), o.i + 16 * (k - 1), true);
    put("u[0]".into(), o.u, true);
    put(format!("u[{}]", k - 1), o.u + 16 * (k - 1), true);
    put("v[0].x".into(), o.v, false);
    put("v[0].y".into(), o.v + 4, false);
    put(format!("v[{}].x", k - 1), o.v + 16 * (k - 1), false);
    put(format!("v[{}].y", k - 1), o.v + 16 * (k - 1) + 4, false);
    for c in 0..4 {
        put(format!("t{c}"), o.t + 4 * c, false);
    }
    let dwords: HashSet<usize> = out.iter().map(|&(_, d, _, _)| d).collect();
    assert_eq!(dwords.len(), 14, "two reads share a dword (K = {k})");
    out
}

/// What the depth is when every read lands where std140 says: the sum of
/// the 14 distinct powers of two.
const WANT: f32 = 16383.0;

/// The block's buffer: every dword `j` holds `j + 0.5` -- a marker naming
/// the buffer dword it sits in, exact in f32 -- except the 14 read dwords,
/// which hold their powers of two. All 4096 dword values are distinct.
fn buffer(k: usize) -> Vec<u32> {
    let mut buf: Vec<u32> = (0..BLOCK_DWORDS).map(|j| (j as f32 + 0.5).to_bits()).collect();
    for &(_, dword, bits, _) in reads(k).iter() {
        buf[dword] = bits;
    }
    let values: HashSet<u32> = buf.iter().copied().collect();
    assert_eq!(values.len(), BLOCK_DWORDS, "the buffer's dwords are not all distinct (K = {k})");
    buf
}

/// The fragment shader: the block `Big` and the 14 reads summed into
/// `gl_FragDepth` (every member and operation highp: the sum must be exact).
fn fs_src(k: usize) -> String {
    format!(
        "#version 300 es\nlayout(std140) uniform Big {{ \
         highp vec4 padA[255]; highp vec4 padB[255]; highp vec4 padC[255]; highp vec4 padD[{}]; \
         highp float f[{}]; highp int i[{}]; highp uint u[{}]; highp vec2 v[{}]; \
         highp float t0; highp float t1; highp float t2; highp float t3; }};\n\
         layout(location = 0) out mediump vec4 o_color;\n\
         void main (void)\n{{\n    o_color = vec4(1.0);\n    gl_FragDepth = \
         f[0] + f[{}] + float(i[0]) + float(i[{}]) + float(u[0]) + float(u[{}]) \
         + v[0].x + v[0].y + v[{}].x + v[{}].y + t0 + t1 + t2 + t3;\n}}\n",
        pad_d_len(k),
        k,
        k,
        k,
        k,
        k - 1,
        k - 1,
        k - 1,
        k - 1,
        k - 1
    )
}

/// The names of the 12 block members, as the GL reports them (a member of a
/// block with no instance name is named bare).
const MEMBER_NAMES: [&str; 12] = ["padA", "padB", "padC", "padD", "f", "i", "u", "v", "t0", "t1", "t2", "t3"];

/// Why the depth is not `WANT`: the 14 reads with their dwords, and, when a
/// single misplaced read explains the sum, the (read, buffer dword) pair it
/// stands for.
fn why(k: usize, got: f32, buf: &[u32]) -> String {
    let mut s = format!(
        "K = {k}: gl_FragDepth is {got} (bits {:#010x}), want {WANT} -- the 14 std140 reads summed.\nreads:",
        got.to_bits(),
    );
    for (name, dword, bits, value) in reads(k) {
        s += &format!("\n  {name}: buffer dword {dword} (byte {}), value {value}, stored {bits:#010x}", dword * 4);
    }
    let mut hits = 0usize;
    for (name, _, _, value) in reads(k) {
        for (j, &bits) in buf.iter().enumerate() {
            let read_instead = f32::from_bits(bits);
            if (WANT - value + read_instead).to_bits() == got.to_bits() {
                s += &format!("\nconsistent with {name} having read buffer dword {j} (value {read_instead})");
                hits += 1;
                break;
            }
        }
    }
    if hits == 0 {
        s += "\nno single misplaced read explains the sum: more than one read landed wrong";
    }
    s
}

/* ---- the GL path: link, bind, draw --------------------------------------------- */

const TARGET: Target = Target { vpm_size: 65536 };
const VS: &str = "#version 300 es\nin highp vec4 a_position;\nvoid main (void)\n{\n    gl_Position = a_position;\n}\n";

fn log_of(buf: &[c_char]) -> String {
    // SAFETY: the buffer is zero-initialised and the driver NUL-terminates.
    unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
}

fn shader(ty: GLenum, src: &str) -> Result<GLuint, String> {
    let c = CString::new(src).unwrap();
    // SAFETY: plain GL calls on a CString that outlives them; the log has room.
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
        glGetShaderInfoLog(s, 1023, core::ptr::null_mut(), log.as_mut_ptr());
        Err(log_of(&log))
    }
}

/// Link `fs_src` against the position-only VS, or the compile / link log.
fn link(fs_src: &str) -> Result<GLuint, String> {
    let v = shader(GL_VERTEX_SHADER, VS).map_err(|e| format!("vs: {e}"))?;
    let f = shader(GL_FRAGMENT_SHADER, fs_src).map_err(|e| format!("fs: {e}"))?;
    // SAFETY: plain GL calls on objects made above; the log has room.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        glDeleteShader(v);
        glDeleteShader(f);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok == 1 {
            return Ok(p);
        }
        let mut log = [0 as c_char; 1024];
        glGetProgramInfoLog(p, 1023, core::ptr::null_mut(), log.as_mut_ptr());
        Err(log_of(&log))
    }
}

fn program_iv(p: GLuint, pname: GLenum) -> GLint {
    let mut x: GLint = -1;
    // SAFETY: one value out.
    unsafe { glGetProgramiv(p, pname, &mut x) };
    x
}

fn block_iv(p: GLuint, index: GLuint, pname: GLenum) -> GLint {
    let mut x: GLint = -1;
    // SAFETY: one value out.
    unsafe { glGetActiveUniformBlockiv(p, index, pname, &mut x) };
    x
}

/// A linked, current program of `fs_src` with block `Big` at binding point 0,
/// its data size checked against 16384.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn current_program(fs_src: &str) -> GLuint {
    let p = link(fs_src).unwrap_or_else(|e| panic!("link (K case):\n{e}\n{fs_src}"));
    glUseProgram(p);
    let idx = glGetUniformBlockIndex(p, c"Big".as_ptr());
    assert_ne!(idx, GL_INVALID_INDEX, "block Big");
    assert_eq!(block_iv(p, idx, GL_UNIFORM_BLOCK_DATA_SIZE) as usize, BLOCK_BYTES, "GL_UNIFORM_BLOCK_DATA_SIZE");
    glUniformBlockBinding(p, idx, 0);
    assert_eq!(err(), GL_NO_ERROR);
    p
}

/// A uniform buffer of `data`, bound to uniform-buffer binding point 0.
///
/// # Safety
/// Plain GL calls on the current context; `data` outlives the buffer's copy.
unsafe fn bound_buffer(data: &[u32]) -> GLuint {
    let mut b = 0;
    glGenBuffers(1, &mut b);
    glBindBuffer(GL_UNIFORM_BUFFER, b);
    glBufferData(GL_UNIFORM_BUFFER, (data.len() * 4) as GLsizeiptr, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
    glBindBufferBase(GL_UNIFORM_BUFFER, 0, b);
    assert_eq!(err(), GL_NO_ERROR, "glBufferData / glBindBufferBase of the 16384-byte block");
    b
}

/// One draw of the current program; the stage block it handed over.
///
/// # Safety
/// Plain GL calls on the current context.
unsafe fn draw(p: GLuint) -> Vec<u32> {
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0];
    let loc = glGetAttribLocation(p, c"a_position".as_ptr());
    glBindBuffer(GL_ARRAY_BUFFER, 0);
    glVertexAttribPointer(loc as u32, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
    glEnableVertexAttribArray(loc as u32);
    *DRAWN.lock().unwrap() = None;
    glDrawArrays(GL_TRIANGLES, 0, 3);
    glDisableVertexAttribArray(loc as u32);
    assert_eq!(err(), GL_NO_ERROR, "the draw");
    DRAWN.lock().unwrap().take().expect("the draw reached the back end")
}

/* ---- the sim: the same shader, reading the captured block --------------------- */

/// The depth lane 0 writes for `fs` when the QPU reads `block` (the block as
/// the draw handed it over).
fn sim_depth(fs: &str, block: &[u32]) -> Result<f32, String> {
    let mut mem = vec![0u8; 64 << 20];
    let a = Arena::new(&mut mem);
    let vs = a.alloc_str(VS).expect("arena");
    let fs_ = a.alloc_str(fs).expect("arena");
    let v = lower::compile_to_ir(&a, sema::Stage::Vertex, vs).map_err(|e| e.msg.to_string()).expect("the VS lowers");
    let f = lower::compile_to_ir(&a, sema::Stage::Fragment, fs_).map_err(|e| format!("lower: {}", e.msg))?;
    let p =
        compile_program(&v.module, &f.module, Prim::Triangles, TARGET, &a).map_err(|e| format!("compile: {e:?}"))?;
    let mut stream = Vec::new();
    for u in p.fs.code.uniforms.iter() {
        stream.push(match u.contents {
            0 => u.data,
            1 => *block.get(u.data as usize).ok_or(format!(
                "stream dword {} past the draw's block of {}",
                u.data,
                block.len()
            ))?,
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

/* ---- (1) the GL queries answer std140 ------------------------------------------ */

#[test]
fn queries_answer_std140_offsets_and_strides() {
    let _g = context();
    for k in K_RANGE {
        let o = offs(k);
        let want_off: [GLint; 12] = [
            o.pad[0] as GLint,
            o.pad[1] as GLint,
            o.pad[2] as GLint,
            o.pad[3] as GLint,
            o.f as GLint,
            o.i as GLint,
            o.u as GLint,
            o.v as GLint,
            o.t as GLint,
            (o.t + 4) as GLint,
            (o.t + 8) as GLint,
            (o.t + 12) as GLint,
        ];
        let want_stride: [GLint; 12] = [16, 16, 16, 16, 16, 16, 16, 16, 0, 0, 0, 0];
        let want_size: [GLint; 12] = [
            PAD_MAX as GLint,
            PAD_MAX as GLint,
            PAD_MAX as GLint,
            pad_d_len(k) as GLint,
            k as GLint,
            k as GLint,
            k as GLint,
            k as GLint,
            1,
            1,
            1,
            1,
        ];
        // SAFETY: plain GL calls on handles this case owns; the out arrays have room.
        unsafe {
            let p = current_program(&fs_src(k));
            assert_eq!(program_iv(p, GL_ACTIVE_UNIFORMS), MEMBER_NAMES.len() as GLint, "K = {k}");
            let cnames: Vec<CString> = MEMBER_NAMES.iter().map(|n| CString::new(*n).unwrap()).collect();
            let ptrs: Vec<*const c_char> = cnames.iter().map(|c| c.as_ptr()).collect();
            let mut idx = [GL_INVALID_INDEX; 12];
            glGetUniformIndices(p, 12, ptrs.as_ptr(), idx.as_mut_ptr());
            for (name, &ix) in MEMBER_NAMES.iter().zip(&idx) {
                assert_ne!(ix, GL_INVALID_INDEX, "no active uniform {name} (K = {k})");
            }
            let mut out = [0 as GLint; 12];
            let mut query = |pname: GLenum, want: [GLint; 12]| {
                glGetActiveUniformsiv(p, 12, idx.as_ptr(), pname, out.as_mut_ptr());
                for (i, (&got, &exp)) in out.iter().zip(want.iter()).enumerate() {
                    assert_eq!(got, exp, "{:?} of {} (K = {k})", pname, MEMBER_NAMES[i]);
                }
            };
            query(GL_UNIFORM_OFFSET, want_off);
            query(GL_UNIFORM_ARRAY_STRIDE, want_stride);
            query(GL_UNIFORM_SIZE, want_size);
            assert_eq!(err(), GL_NO_ERROR);
            glDeleteProgram(p);
        }
    }
}

/* ---- (2) the draw path and the sim read the arrays at those offsets ----------- */

#[test]
fn draw_and_sim_read_the_arrays_at_their_std140_offsets() {
    let _g = context();
    for k in K_RANGE {
        let src = fs_src(k);
        let data = buffer(k);
        // SAFETY: plain GL calls on handles this case owns; `data` outlives the buffer's copy.
        unsafe {
            let buf = bound_buffer(&data);
            let p = current_program(&src);
            let block = draw(p);
            glDeleteProgram(p);
            glDeleteBuffers(1, &buf);
            let got = sim_depth(&src, &block).unwrap_or_else(|why| panic!("K = {k}: {why}\n{src}"));
            if got != WANT {
                panic!("{}", why(k, got, &data));
            }
        }
    }
}
