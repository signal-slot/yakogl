// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP), ES 2.0 context, against 3cbecbb9 (records and the GL
//! program take more than 255 active uniforms). The change moved how a
//! program's uniforms are counted and addressed for every GPU: a program's
//! locations are a range of one pool per context that a relink or a delete
//! gives back (the ranges after it move down), and a location is
//! `index | element << 16` (was `<< 8`, src/gl/program.rs); the shader
//! records count their uniforms past 255 (src/shaderc/online/abi.rs,
//! src/shaderc/mod.rs, src/shaderc/precompiled.rs). dEQP-GLES2
//! functional.uniform_api.* (info_query.*, value.initial.*, value.assigned.*,
//! random.*) reads exactly these answers: the location of each name,
//! glGetActiveUniform's name, size and type, glGetUniform*v's values,
//! GL_ACTIVE_UNIFORMS and GL_ACTIVE_UNIFORM_MAX_LENGTH. The i.MX8MP whole
//! tree (0fbe61a3) predates the change, and its stage-1 promise is 0 Fail.
//!
//! Through the real GL core (a `report-es2` build, GLSL ES 1.00 programs,
//! the online compiler) and the real Vivante back end over a stand-in OSAL
//! that keeps memory on the host and records every submitted stream, this
//! file pins (expected values from ES 2.0.25 2.10.4 and 6.1.8, GLSL ES 1.00
//! Appendix A.7, and what es2fUniformApiTests.cpp checks):
//!
//! - a program with struct, array, array-of-struct, sampler and bool
//!   uniforms in both stages: GL_ACTIVE_UNIFORMS, GL_ACTIVE_UNIFORM_MAX_LENGTH,
//!   every glGetActiveUniform (name, size, type; a truncated name);
//! - one distinct location per active name and array element (`a[0]`, `a`
//!   and `s.m[1]` among them), -1 for the names that are not active uniforms;
//! - the initial value 0 of every uniform, and the value read back after
//!   each setter (bool through the int and the float setters, samplers
//!   through glUniform1i, arrays through one call and per element);
//! - the setter errors of ES 2.0 2.10.4 next to them (a wrong type is
//!   GL_INVALID_OPERATION, location -1 is ignored);
//! - a program of 274 active uniforms (past the old 255; 273 in the fragment
//!   stage), its locations past the old index byte and their values;
//! - the location pool: deleting and relinking a program leaves the
//!   locations and values of the programs after it as they were;
//! - a program of more than 255 vec4 rows of struct-array uniforms, as close
//!   to the reported limits as the GC7000's one 320-register constant file
//!   lets a program link, and (`#[ignore]`d, a known defect) one row under
//!   both reported limits, GL_MAX_VERTEX_UNIFORM_VECTORS and
//!   GL_MAX_FRAGMENT_UNIFORM_VECTORS (384 each), which the shared 320-register
//!   file cannot hold;
//! - draws with these programs are accepted (no GL error, no refused draw,
//!   a kick), and the constant file the kick loads holds the values set.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_uniforms
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::gc_refused_draws;
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};
use yakogl::vivante::regs::*;

/* ---- the stand-in OSAL: host memory, and the streams the back end submits --------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: Vec<Vec<u32>>,
    faults: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// Serialises the tests: the OSAL pointer, the back end and `g_backend` are process-global.
static SERIAL: Mutex<()> = Mutex::new(());

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
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != core::mem::size_of::<GcKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -22;
    }
    // SAFETY: the back end passes a live GcKick of that size.
    let k = unsafe { *(kick as *const GcKick) };
    with_platform(|p| {
        if k.magic != GC_KICK_MAGIC {
            p.faults.push(format!("submit with magic 0x{:x}", k.magic));
            return -22;
        }
        let Some(a) = p.allocs.get(&k.stream_gpu) else {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        };
        // SAFETY: the allocation holds at least `stream_bytes` bytes.
        let words: &[u32] =
            unsafe { core::slice::from_raw_parts(a.as_ptr() as *const u32, k.stream_bytes as usize / 4) };
        p.kicks.push(words.to_vec());
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

/* ---- a fresh context, back end and OSAL per case -------------------------------------- */

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 32x32 context (a fresh location pool, as at the start of a CTS case group).
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(W as u32, H as u32);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        yakogl::vivante::backend::gc_shutdown();
    }
    let faults = with_platform(|p| {
        /* leaked, not freed: a static of the library may still name the block
         * (tests/gc_es2ctx_sampler_slots.rs does the same) */
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
        p.kicks.clear();
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- decoding a submitted stream ------------------------------------------------------ */

/// Every `(register, value)` a LOAD_STATE of the stream writes, in order.
fn writes(words: &[u32]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        let w = words[i];
        if w & 0xf800_0000 == GC_OP_LOAD_STATE {
            let count = ((w >> 16) & 0x3ff) as usize;
            let base = (w & 0xffff) * 4;
            for k in 0..count {
                if let Some(v) = words.get(i + 1 + k) {
                    out.push((base + 4 * k as u32, *v));
                }
            }
            i += 1 + count;
            continue;
        }
        i += match w & 0xf800_0000 {
            GC_OP_DRAW_INSTANCED => 4,
            GC_OP_STALL => 2,
            _ => 1,
        };
    }
    out
}

/// The constant file (vertex stage first, the fragment stage after it, one
/// dword per entry) as the kicks since `since` left it: the last write wins.
fn constant_file(since: usize) -> BTreeMap<u32, u32> {
    let file = GC_SH_HALTI5_UNIFORMS_VS..GC_SH_HALTI5_UNIFORMS_PS + GC_NUM_CONSTANTS * 16;
    let mut out = BTreeMap::new();
    with_platform(|p| {
        for k in &p.kicks[since..] {
            for (r, v) in writes(k) {
                if file.contains(&r) {
                    out.insert(r, v);
                }
            }
        }
    });
    out
}

/// The words of `w` lie in consecutive dwords somewhere in the file.
fn holds(file: &BTreeMap<u32, u32>, w: &[u32]) -> bool {
    !w.is_empty() && file.keys().any(|&r| w.iter().enumerate().all(|(k, v)| file.get(&(r + 4 * k as u32)) == Some(v)))
}

fn bits(v: &[f32]) -> Vec<u32> {
    v.iter().map(|f| f.to_bits()).collect()
}

/* ---- GL plumbing ---------------------------------------------------------------------- */

const W: GLsizei = 32;
const H: GLsizei = 32;

/// The error flag, drained: the first error since the last call.
fn err() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        let e = glGetError();
        while glGetError() != GL_NO_ERROR {}
        e
    }
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
            let mut buf = [0 as c_char; 1024];
            let mut len: GLsizei = 0;
            glGetShaderInfoLog(s, buf.len() as GLsizei, &mut len, buf.as_mut_ptr());
            let log = CStr::from_ptr(buf.as_ptr()).to_string_lossy();
            panic!(
                "a {} shader of {} bytes did not compile: {log}",
                if ty == GL_VERTEX_SHADER { "vertex" } else { "fragment" },
                text.as_bytes().len()
            );
        }
        s
    }
}

/// The program's info log.
fn program_log(p: GLuint) -> String {
    let mut buf = [0 as c_char; 1024];
    let mut len: GLsizei = 0;
    // SAFETY: a buffer of the size announced; GL NUL-terminates the log.
    unsafe {
        glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

/// A linked program of `vs` and `fs` (attribute `p` at 0), not made current.
fn link(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link: {}", program_log(p));
        /* flagged for deletion with the program */
        glDeleteShader(v);
        glDeleteShader(f);
        assert_eq!(err(), GL_NO_ERROR, "link");
        p
    }
}

fn relink(p: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "relink: {}", program_log(p));
    }
}

fn delete(p: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
    }
    assert_eq!(err(), GL_NO_ERROR, "delete");
}

fn iv(p: GLuint, pname: GLenum) -> GLint {
    let mut v = -7;
    // SAFETY: a plain GL call with room for one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetProgramiv {pname:#x}");
    v
}

fn loc(p: GLuint, name: &str) -> GLint {
    let n = CString::new(name).unwrap();
    // SAFETY: a plain GL call with a NUL-terminated name.
    let l = unsafe { glGetUniformLocation(p, n.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformLocation {name}");
    l
}

/// Every glGetActiveUniform of `p`: (name, size, type), sorted by name.
fn actives(p: GLuint) -> Vec<(String, GLint, GLenum)> {
    let n = iv(p, GL_ACTIVE_UNIFORMS);
    let mut out = Vec::new();
    for i in 0..n as GLuint {
        let mut buf = [0 as c_char; 64];
        let (mut len, mut size, mut ty) = (-1, -1, 0);
        // SAFETY: a plain GL call; the buffer holds 64 bytes.
        unsafe { glGetActiveUniform(p, i, 64, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "glGetActiveUniform {i}");
        // SAFETY: the GL wrote a NUL-terminated name into `buf`.
        let s = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_str().unwrap().to_owned();
        assert_eq!(len as usize, s.len(), "length of {s}");
        out.push((s, size, ty));
    }
    out.sort();
    out
}

fn getf(p: GLuint, l: GLint, n: usize) -> Vec<f32> {
    let mut v = [-99.0f32; 16];
    // SAFETY: a plain GL call; 16 values is room for a mat4.
    unsafe { glGetUniformfv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformfv {l:#x}");
    assert_eq!(v[n], -99.0, "glGetUniformfv {l:#x} wrote past {n} values");
    v[..n].to_vec()
}

fn geti(p: GLuint, l: GLint, n: usize) -> Vec<i32> {
    let mut v = [-99i32; 16];
    // SAFETY: a plain GL call; 16 values is room for a mat4.
    unsafe { glGetUniformiv(p, l, v.as_mut_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetUniformiv {l:#x}");
    assert_eq!(v[n], -99, "glGetUniformiv {l:#x} wrote past {n} values");
    v[..n].to_vec()
}

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// A draw with `p` current is accepted: no GL error, no refused draw, a
/// kick. Returns the constant file the kicks of the draw loaded.
fn draws(p: GLuint, what: &str) -> BTreeMap<u32, u32> {
    let refused = gc_refused_draws();
    let kicks = with_platform(|p| p.kicks.len());
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glViewport(0, 0, W, H);
        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    let (e, n) = (err(), gc_refused_draws().wrapping_sub(refused));
    assert_eq!((e, n), (GL_NO_ERROR, 0), "{what}: error, refused draws");
    assert!(with_platform(|p| p.kicks.len()) > kicks, "{what}: the draw produced no kick");
    constant_file(kicks)
}

/* ---- the mixed program ---- */

const VS_MIX: &str = "attribute vec4 p;\n\
    struct S { float f; vec3 v; bool b; };\n\
    uniform S s;\n\
    uniform vec4 arr[3];\n\
    uniform mat3 m;\n\
    uniform bool flag;\n\
    uniform bvec2 bv;\n\
    uniform ivec3 iv;\n\
    void main() {\n\
      vec4 a = p + vec4(s.f, s.v) + arr[0] + arr[1] + arr[2] + vec4(m[0] + m[1] + m[2], 0.0);\n\
      if (flag) a += vec4(1.0);\n\
      if (s.b) a += vec4(2.0);\n\
      a += vec4(float(bv.x), float(bv.y), float(iv.x + iv.y), float(iv.z));\n\
      gl_Position = a;\n\
    }\n";

const FS_MIX: &str = "precision mediump float;\n\
    struct L { vec3 c; float w; };\n\
    uniform L lights[2];\n\
    uniform sampler2D tex;\n\
    uniform samplerCube cube;\n\
    uniform int count;\n\
    void main() {\n\
      vec4 a = texture2D(tex, vec2(0.5)) + textureCube(cube, vec3(1.0, 0.5, 0.5));\n\
      a += vec4(lights[0].c * lights[0].w + lights[1].c * lights[1].w, 0.0);\n\
      gl_FragColor = a + vec4(float(count));\n\
    }\n";

/// The active uniforms of VS_MIX + FS_MIX as glGetActiveUniform names them
/// (ES 2.0 2.10.4): a struct is one active uniform per member, an array of
/// structs one per element member, an array of a basic type one entry
/// named `[0]` with the array's size.
const MIX: [(&str, GLint, GLenum); 15] = [
    ("arr[0]", 3, GL_FLOAT_VEC4),
    ("bv", 1, GL_BOOL_VEC2),
    ("count", 1, GL_INT),
    ("cube", 1, GL_SAMPLER_CUBE),
    ("flag", 1, GL_BOOL),
    ("iv", 1, GL_INT_VEC3),
    ("lights[0].c", 1, GL_FLOAT_VEC3),
    ("lights[0].w", 1, GL_FLOAT),
    ("lights[1].c", 1, GL_FLOAT_VEC3),
    ("lights[1].w", 1, GL_FLOAT),
    ("m", 1, GL_FLOAT_MAT3),
    ("s.b", 1, GL_BOOL),
    ("s.f", 1, GL_FLOAT),
    ("s.v", 1, GL_FLOAT_VEC3),
    ("tex", 1, GL_SAMPLER_2D),
];

/// Every name a location is asked for: the active ones and each array element.
const MIX_LOCS: [&str; 17] = [
    "arr[0]",
    "arr[1]",
    "arr[2]",
    "bv",
    "count",
    "cube",
    "flag",
    "iv",
    "lights[0].c",
    "lights[0].w",
    "lights[1].c",
    "lights[1].w",
    "m",
    "s.b",
    "s.f",
    "s.v",
    "tex",
];

/// Components of each name of MIX_LOCS (glGetUniform*v writes this many).
fn comps(name: &str) -> usize {
    match name {
        "m" => 9,
        n if n.starts_with("arr") => 4,
        "iv" | "s.v" | "lights[0].c" | "lights[1].c" => 3,
        "bv" => 2,
        _ => 1,
    }
}

/* ---- the cases ---- */

/// Sanity: this is an ES 2.0 context (GL_MAJOR_VERSION is an ES 3.0 query).
#[test]
fn the_context_is_es2() {
    on_gc7000(|| {
        // SAFETY: the context's static version string.
        let version = unsafe { CStr::from_ptr(glGetString(GL_VERSION) as *const c_char) }.to_string_lossy();
        assert!(version.starts_with("OpenGL ES 2.0"), "an ES 2.0 context: {version}");
        let mut m = 0;
        // SAFETY: a plain GL call.
        unsafe { glGetIntegerv(GL_MAJOR_VERSION, &mut m) };
        assert_eq!(err(), GL_INVALID_ENUM);
    });
}

/// GL_ACTIVE_UNIFORMS, GL_ACTIVE_UNIFORM_MAX_LENGTH and every
/// glGetActiveUniform of the mixed program (dEQP-GLES2
/// uniform_api.info_query.*).
#[test]
fn active_uniforms_of_the_mixed_program() {
    on_gc7000(|| {
        let p = link(VS_MIX, FS_MIX);
        assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), MIX.len() as GLint);
        let want: Vec<(String, GLint, GLenum)> = MIX.iter().map(|&(n, s, t)| (n.to_owned(), s, t)).collect();
        assert_eq!(actives(p), want);
        /* "lights[0].c" and its NUL */
        assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 12);

        /* a short buffer: the name is cut and terminated, the length is what was written */
        let i = (0..MIX.len() as GLuint)
            .find(|&i| {
                let mut buf = [0 as c_char; 64];
                let (mut len, mut size, mut ty) = (0, 0, 0);
                // SAFETY: a plain GL call; the buffer holds 64 bytes.
                unsafe { glGetActiveUniform(p, i, 64, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
                // SAFETY: a NUL-terminated name.
                unsafe { CStr::from_ptr(buf.as_ptr()) }.to_bytes() == b"lights[0].c"
            })
            .expect("lights[0].c is active");
        let mut buf = [0x55 as c_char; 8];
        let (mut len, mut size, mut ty) = (-1, -1, 0);
        // SAFETY: a plain GL call; 4 of the buffer's 8 bytes are offered.
        unsafe { glGetActiveUniform(p, i, 4, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
        assert_eq!(err(), GL_NO_ERROR);
        // SAFETY: a NUL-terminated name.
        assert_eq!(unsafe { CStr::from_ptr(buf.as_ptr()) }.to_bytes(), b"lig");
        assert_eq!((len, size, ty), (3, 1, GL_FLOAT_VEC3));
        assert_eq!(buf[4], 0x55, "nothing past bufSize");

        /* an index past the last, and a negative bufSize */
        // SAFETY: plain GL calls; the buffer holds 8 bytes.
        unsafe {
            glGetActiveUniform(p, MIX.len() as GLuint, 8, &mut len, &mut size, &mut ty, buf.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_VALUE, "index past the last");
            glGetActiveUniform(p, 0, -1, &mut len, &mut size, &mut ty, buf.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_VALUE, "bufSize -1");
        }
        delete(p);
    });
}

/// One distinct location per active name and array element; -1 for what
/// is not an active uniform (dEQP-GLES2 uniform_api.value.*, which takes
/// every location before it sets or reads).
#[test]
fn locations_of_the_mixed_program() {
    on_gc7000(|| {
        let p = link(VS_MIX, FS_MIX);
        let mut seen = BTreeMap::new();
        for name in MIX_LOCS {
            let l = loc(p, name);
            assert!(l >= 0, "{name}: {l}");
            if let Some(other) = seen.insert(l, name) {
                panic!("{name} and {other} share location {l:#x}");
            }
        }
        /* the array's own name is its element 0 */
        assert_eq!(loc(p, "arr"), loc(p, "arr[0]"));
        for name in ["arr[3]", "s", "lights", "lights[0]", "lights[2].c", "s.x", "nope", "gl_DepthRange.near"] {
            assert_eq!(loc(p, name), -1, "{name}");
        }
        delete(p);
    });
}

/// Every uniform reads 0 after the link, as an int and as a float (ES 2.0
/// 2.10.4; dEQP-GLES2 uniform_api.value.initial.*).
#[test]
fn values_start_at_zero() {
    on_gc7000(|| {
        let p = link(VS_MIX, FS_MIX);
        for name in MIX_LOCS {
            let (l, n) = (loc(p, name), comps(name));
            assert_eq!(getf(p, l, n), vec![0.0; n], "{name} as float");
            assert_eq!(geti(p, l, n), vec![0; n], "{name} as int");
        }
        delete(p);
    });
}

/// Each setter's value reads back (dEQP-GLES2 uniform_api.value.assigned.*):
/// struct members, array elements through one call and one by one, the
/// struct-array members, the matrix, bools through the int and the float
/// setters, samplers through glUniform1i; a draw carries them.
#[test]
fn values_read_back() {
    on_gc7000(|| {
        let p = link(VS_MIX, FS_MIX);
        let arr: [f32; 12] = core::array::from_fn(|i| i as f32 + 0.5);
        // SAFETY: plain GL calls with the program current; each array holds what its call reads.
        unsafe {
            glUseProgram(p);
            glUniform1f(loc(p, "s.f"), 1.5);
            glUniform3f(loc(p, "s.v"), 2.0, -3.0, 4.25);
            glUniform1i(loc(p, "s.b"), 7);
            glUniform4fv(loc(p, "arr"), 3, arr.as_ptr());
            let m: [f32; 9] = core::array::from_fn(|i| 10.0 + i as f32);
            glUniformMatrix3fv(loc(p, "m"), 1, 0, m.as_ptr());
            glUniform1f(loc(p, "flag"), 0.25);
            glUniform2i(loc(p, "bv"), 0, -4);
            glUniform3i(loc(p, "iv"), -1, 70000, 3);
            glUniform3f(loc(p, "lights[0].c"), 0.5, 0.25, 0.125);
            glUniform1f(loc(p, "lights[0].w"), 8.0);
            glUniform3fv(loc(p, "lights[1].c"), 1, [9.0f32, 10.0, 11.0].as_ptr());
            glUniform1fv(loc(p, "lights[1].w"), 1, [-2.0f32].as_ptr());
            glUniform1i(loc(p, "tex"), 3);
            glUniform1iv(loc(p, "cube"), 1, [5].as_ptr());
            glUniform1i(loc(p, "count"), -12);
        }
        assert_eq!(err(), GL_NO_ERROR, "the setters");
        assert_eq!(getf(p, loc(p, "s.f"), 1), [1.5]);
        assert_eq!(getf(p, loc(p, "s.v"), 3), [2.0, -3.0, 4.25]);
        assert_eq!(geti(p, loc(p, "s.b"), 1), [1], "bool set to 7 reads 1");
        assert_eq!(getf(p, loc(p, "s.b"), 1), [1.0]);
        for e in 0..3 {
            let base = e as f32 * 4.0 + 0.5;
            assert_eq!(
                getf(p, loc(p, &format!("arr[{e}]")), 4),
                [base, base + 1.0, base + 2.0, base + 3.0],
                "arr[{e}]"
            );
        }
        assert_eq!(getf(p, loc(p, "m"), 9), (0..9).map(|i| 10.0 + i as f32).collect::<Vec<_>>());
        assert_eq!(geti(p, loc(p, "flag"), 1), [1], "bool set to 0.25 reads 1");
        assert_eq!(geti(p, loc(p, "bv"), 2), [0, 1]);
        assert_eq!(getf(p, loc(p, "bv"), 2), [0.0, 1.0]);
        assert_eq!(geti(p, loc(p, "iv"), 3), [-1, 70000, 3]);
        assert_eq!(getf(p, loc(p, "iv"), 3), [-1.0, 70000.0, 3.0]);
        assert_eq!(getf(p, loc(p, "lights[0].c"), 3), [0.5, 0.25, 0.125]);
        assert_eq!(getf(p, loc(p, "lights[0].w"), 1), [8.0]);
        assert_eq!(getf(p, loc(p, "lights[1].c"), 3), [9.0, 10.0, 11.0]);
        assert_eq!(getf(p, loc(p, "lights[1].w"), 1), [-2.0]);
        assert_eq!(geti(p, loc(p, "tex"), 1), [3]);
        assert_eq!(geti(p, loc(p, "cube"), 1), [5]);
        assert_eq!(geti(p, loc(p, "count"), 1), [-12]);

        let file = draws(p, "the mixed program");
        assert!(holds(&file, &bits(&[2.0, -3.0, 4.25])), "s.v in the constant file");
        for e in 0..3 {
            assert!(holds(&file, &bits(&arr[e * 4..e * 4 + 4])), "arr[{e}] in the constant file");
        }
        assert!(holds(&file, &bits(&[0.5, 0.25, 0.125])), "lights[0].c in the constant file");
        assert!(holds(&file, &bits(&[9.0, 10.0, 11.0])), "lights[1].c in the constant file");

        /* one element through its own location; the others keep theirs */
        // SAFETY: plain GL calls with the program current.
        unsafe {
            glUniform4f(loc(p, "arr[1]"), -1.0, -2.0, -3.0, -4.0);
            /* from element 2 on, a count past the end is cut to the array */
            glUniform4fv(loc(p, "arr[2]"), 2, [7.0f32; 8].as_ptr());
            glUniform1f(loc(p, "flag"), 0.0);
        }
        assert_eq!(err(), GL_NO_ERROR, "the element setters");
        assert_eq!(getf(p, loc(p, "arr[0]"), 4), [0.5, 1.5, 2.5, 3.5]);
        assert_eq!(getf(p, loc(p, "arr[1]"), 4), [-1.0, -2.0, -3.0, -4.0]);
        assert_eq!(getf(p, loc(p, "arr[2]"), 4), [7.0; 4]);
        assert_eq!(geti(p, loc(p, "flag"), 1), [0]);
        /* every other uniform kept its value */
        assert_eq!(getf(p, loc(p, "s.v"), 3), [2.0, -3.0, 4.25]);
        assert_eq!(geti(p, loc(p, "count"), 1), [-12]);
        let file = draws(p, "the mixed program, again");
        assert!(holds(&file, &bits(&[-1.0, -2.0, -3.0, -4.0])), "the new arr[1] in the constant file");
        delete(p);
    });
}

/// The ES 2.0 2.10.4 errors of the setters and getters next to those
/// answers (dEQP-GLES2 negative_api.shader.uniform*).
#[test]
fn setter_errors() {
    on_gc7000(|| {
        let p = link(VS_MIX, FS_MIX);
        // SAFETY: plain GL calls with the program current; each array holds what its call reads.
        unsafe {
            glUseProgram(p);
            glUniform1f(loc(p, "count"), 1.0);
            assert_eq!(err(), GL_INVALID_OPERATION, "float setter on an int");
            glUniform1i(loc(p, "s.f"), 1);
            assert_eq!(err(), GL_INVALID_OPERATION, "int setter on a float");
            glUniform1f(loc(p, "tex"), 1.0);
            assert_eq!(err(), GL_INVALID_OPERATION, "float setter on a sampler");
            glUniform2f(loc(p, "s.v"), 1.0, 2.0);
            assert_eq!(err(), GL_INVALID_OPERATION, "vec2 setter on a vec3");
            glUniform4fv(loc(p, "m"), 1, [0.0f32; 16].as_ptr());
            assert_eq!(err(), GL_INVALID_OPERATION, "vec4 setter on a mat3");
            glUniform1iv(loc(p, "tex"), 2, [1, 2].as_ptr());
            assert_eq!(err(), GL_INVALID_OPERATION, "count 2 on a sampler that is not an array");
            glUniform3fv(loc(p, "lights[0].c"), 2, [0.0f32; 6].as_ptr());
            assert_eq!(err(), GL_INVALID_OPERATION, "count 2 on a struct member that is not an array");
            glUniform4fv(loc(p, "arr"), -1, [0.0f32; 4].as_ptr());
            assert_eq!(err(), GL_INVALID_VALUE, "count -1");
            glUniformMatrix3fv(loc(p, "m"), 1, 1, [0.0f32; 9].as_ptr());
            assert_eq!(err(), GL_INVALID_VALUE, "transpose in ES 2.0");
            glUniform1f(-1, 1.0);
            assert_eq!(err(), GL_NO_ERROR, "location -1 is ignored");
            glUniform1fv(-1, 1, [1.0f32].as_ptr());
            assert_eq!(err(), GL_NO_ERROR, "location -1 is ignored by the vector setter too");
            glUniform1f(0x7fff, 1.0);
            assert_eq!(err(), GL_INVALID_OPERATION, "a location the program does not have");
            glUniform1f(-2, 1.0);
            assert_eq!(err(), GL_INVALID_OPERATION, "location -2");
            let mut v = [0.0f32; 4];
            glGetUniformfv(p, 0x7fff, v.as_mut_ptr());
            assert_eq!(err(), GL_INVALID_OPERATION, "reading a location the program does not have");
            glUseProgram(0);
            glUniform1f(loc(p, "s.f"), 1.0);
            assert_eq!(err(), GL_INVALID_OPERATION, "no current program");
        }
        /* none of the refused calls stored anything */
        assert_eq!(getf(p, loc(p, "s.f"), 1), [0.0]);
        assert_eq!(geti(p, loc(p, "tex"), 1), [0]);
        assert_eq!(getf(p, loc(p, "m"), 9), [0.0; 9]);
        assert_eq!(geti(p, loc(p, "count"), 1), [0]);
        draws(p, "after the refused setters");
        delete(p);
    });
}

/* ---- struct members that are arrays, bool arrays ---- */

const VS_SM: &str = "attribute vec4 p;\n\
    struct T { vec2 m[3]; float k; };\n\
    uniform T s;\n\
    uniform bool ba[2];\n\
    void main() {\n\
      vec4 a = p + vec4(s.m[0] + s.m[1] + s.m[2], s.k, 0.0);\n\
      if (ba[0]) a.x += 1.0;\n\
      if (ba[1]) a.y += 1.0;\n\
      gl_Position = a;\n\
    }\n";

const VS_PLAIN: &str = "attribute vec4 p;\nuniform vec4 off;\nvoid main() { gl_Position = p + off; }\n";
const FS_PLAIN: &str = "precision mediump float;\nuniform vec4 col;\nvoid main() { gl_FragColor = col; }\n";

/// `s.m[1]`, `s.m` and `ba` (ES 2.0 2.10.4: an array member of a struct is
/// one active uniform named `s.m[0]`; its own name is its element 0; each
/// element has its location), their values, the setter errors on them, and
/// a draw whose constant file holds the values set.
#[test]
fn array_members_of_a_struct_and_bool_arrays() {
    on_gc7000(|| {
        let p = link(VS_SM, FS_PLAIN);
        let want: Vec<(String, GLint, GLenum)> =
            [("ba[0]", 2, GL_BOOL), ("col", 1, GL_FLOAT_VEC4), ("s.k", 1, GL_FLOAT), ("s.m[0]", 3, GL_FLOAT_VEC2)]
                .iter()
                .map(|&(n, s, t)| (n.to_owned(), s, t))
                .collect();
        assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 4);
        assert_eq!(actives(p), want);
        /* "s.m[0]" and its NUL */
        assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 7);

        let m: Vec<GLint> = (0..3).map(|e| loc(p, &format!("s.m[{e}]"))).collect();
        assert!(m.iter().all(|&l| l >= 0), "{m:?}");
        assert!(m[0] != m[1] && m[1] != m[2] && m[0] != m[2], "{m:?}");
        assert_eq!(loc(p, "s.m"), m[0]);
        assert_eq!(loc(p, "ba"), loc(p, "ba[0]"));
        assert!(loc(p, "ba[1]") >= 0 && loc(p, "ba[1]") != loc(p, "ba[0]"));
        for name in ["s.m[3]", "s", "s.m[1].x", "ba[2]"] {
            assert_eq!(loc(p, name), -1, "{name}");
        }

        // SAFETY: plain GL calls with the program current; each array holds what its call reads.
        unsafe {
            glUseProgram(p);
            glUniform2f(m[1], 3.5, -1.25);
            assert_eq!(err(), GL_NO_ERROR);
        }
        assert_eq!(getf(p, m[0], 2), [0.0, 0.0]);
        assert_eq!(getf(p, m[1], 2), [3.5, -1.25]);
        assert_eq!(getf(p, m[2], 2), [0.0, 0.0]);
        let sm = [1.5f32, 2.5, 3.5, 4.5, 5.5, 6.5];
        // SAFETY: as above.
        unsafe {
            glUniform2fv(loc(p, "s.m"), 3, sm.as_ptr());
            glUniform1f(loc(p, "s.k"), 9.75);
            glUniform1iv(loc(p, "ba"), 2, [0, 5].as_ptr());
            glUniform4f(loc(p, "col"), 0.125, 0.25, 0.375, 0.5);
            assert_eq!(err(), GL_NO_ERROR);
            glUniform1f(m[1], 1.0);
            assert_eq!(err(), GL_INVALID_OPERATION, "float setter on a vec2 element");
            glUniform2i(m[1], 1, 2);
            assert_eq!(err(), GL_INVALID_OPERATION, "ivec2 setter on a vec2 element");
            glUniform1f(-1, 1.0);
            assert_eq!(err(), GL_NO_ERROR, "location -1 is ignored");
        }
        for e in 0..3 {
            assert_eq!(getf(p, m[e], 2), sm[e * 2..e * 2 + 2], "s.m[{e}]");
        }
        assert_eq!(getf(p, loc(p, "s.k"), 1), [9.75]);
        assert_eq!(geti(p, loc(p, "ba[0]"), 1), [0]);
        assert_eq!(geti(p, loc(p, "ba[1]"), 1), [1], "bool set to 5 reads 1");
        assert_eq!(getf(p, loc(p, "ba[1]"), 1), [1.0]);

        let file = draws(p, "struct array members");
        for e in 0..3 {
            assert!(holds(&file, &bits(&sm[e * 2..e * 2 + 2])), "s.m[{e}] in the constant file");
        }
        assert!(holds(&file, &bits(&[0.125, 0.25, 0.375, 0.5])), "col in the constant file");
        delete(p);
    });
}

/* ---- more than 255 active uniforms ---- */

const MANY_ELEMS: usize = 17;
const MANY_MEMBERS: usize = 16;

/// A fragment shader of `l[17]` of a struct of 16 floats plus `count`:
/// 273 active uniforms, 272 dwords (68 vec4 rows by the Appendix A packing,
/// inside the 384 vectors the context reports and the 320-register file).
fn fs_many() -> String {
    let members: String = (0..MANY_MEMBERS).map(|m| format!("float m{m}; ")).collect();
    let sum: String = (0..MANY_ELEMS).flat_map(|e| (0..MANY_MEMBERS).map(move |m| format!(" + l[{e}].m{m}"))).collect();
    format!(
        "precision mediump float;\nstruct L {{ {members}}};\nuniform L l[{MANY_ELEMS}];\nuniform int count;\n\
         void main() {{ float a = float(count){sum};\n gl_FragColor = vec4(a); }}\n"
    )
}

fn many_value(e: usize, m: usize) -> f32 {
    (e * 100 + m) as f32 + 0.5
}

/// Sets every `l[e].mN` of the current program `p` to `many_value`.
fn set_many(p: GLuint) {
    for e in 0..MANY_ELEMS {
        for m in 0..MANY_MEMBERS {
            // SAFETY: a plain GL call.
            unsafe { glUniform1f(loc(p, &format!("l[{e}].m{m}")), many_value(e, m)) };
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the 272 setters");
}

fn check_many(p: GLuint, set: bool) {
    let mut seen = BTreeMap::new();
    for e in 0..MANY_ELEMS {
        for m in 0..MANY_MEMBERS {
            let name = format!("l[{e}].m{m}");
            let l = loc(p, &name);
            assert!(l >= 0, "{name}: {l}");
            assert!(seen.insert(l, name.clone()).is_none(), "{name} shares location {l:#x}");
            let want = if set { many_value(e, m) } else { 0.0 };
            assert_eq!(getf(p, l, 1), [want], "{name}");
        }
    }
    assert!(loc(p, "count") >= 0);
    assert!(!seen.contains_key(&loc(p, "count")));
    /* the names past the 255th have an index past the old index byte */
    assert!(seen.keys().any(|&l| l & 0xffff > 255), "{seen:?}");
}

/// 274 active uniforms (the old record and location table stopped at 255):
/// the link, GL_ACTIVE_UNIFORMS, the names, a location per name past the
/// index byte, the value of each (dEQP-GLES2 uniform_api.random.* draws
/// programs of struct arrays like this one), and the draw's constant file.
#[test]
fn more_than_255_active_uniforms() {
    on_gc7000(|| {
        let p = link(VS_PLAIN, &fs_many());
        let n = MANY_ELEMS * MANY_MEMBERS + 2; /* + count + off */
        assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), n as GLint);
        let got = actives(p);
        let mut want: Vec<(String, GLint, GLenum)> = (0..MANY_ELEMS)
            .flat_map(|e| (0..MANY_MEMBERS).map(move |m| (format!("l[{e}].m{m}"), 1, GL_FLOAT)))
            .collect();
        want.push(("count".to_owned(), 1, GL_INT));
        want.push(("off".to_owned(), 1, GL_FLOAT_VEC4));
        want.sort();
        assert_eq!(got, want);
        /* "l[16].m15" and its NUL */
        assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), 10);
        check_many(p, false);
        // SAFETY: plain GL calls.
        unsafe { glUseProgram(p) };
        set_many(p);
        // SAFETY: a plain GL call with the program current.
        unsafe { glUniform1i(loc(p, "count"), 42) };
        check_many(p, true);
        assert_eq!(geti(p, loc(p, "count"), 1), [42]);
        let file = draws(p, "274 active uniforms");
        let values: std::collections::BTreeSet<u32> = file.values().copied().collect();
        for e in 0..MANY_ELEMS {
            for m in 0..MANY_MEMBERS {
                assert!(values.contains(&many_value(e, m).to_bits()), "l[{e}].m{m} in the constant file");
            }
        }
        delete(p);
    });
}

/// The location pool of one context: a program's range moves down when a
/// program before it is deleted or relinked, and its locations and values
/// stay as they were (dEQP-GLES2 keeps several programs alive while it
/// links, deletes and relinks others).
#[test]
fn the_location_pool_moves_programs_without_changing_their_answers() {
    on_gc7000(|| {
        let first = link(VS_MIX, FS_MIX);
        let many = link(VS_PLAIN, &fs_many());
        let plain = link(VS_PLAIN, FS_PLAIN);
        // SAFETY: plain GL calls, each with its program current.
        unsafe {
            glUseProgram(many);
            set_many(many);
            glUseProgram(plain);
            glUniform4f(loc(plain, "off"), 0.25, 0.5, 0.75, 1.0);
            glUniform4f(loc(plain, "col"), 1.0, 2.0, 3.0, 4.0);
        }
        let many_locs_now = || -> Vec<GLint> {
            (0..MANY_ELEMS * MANY_MEMBERS)
                .map(|i| loc(many, &format!("l[{}].m{}", i / MANY_MEMBERS, i % MANY_MEMBERS)))
                .collect()
        };
        let many_locs = many_locs_now();
        let plain_locs = (loc(plain, "off"), loc(plain, "col"));
        let plain_check = |what: &str| {
            assert_eq!((loc(plain, "off"), loc(plain, "col")), plain_locs, "{what}: plain's locations");
            assert_eq!(getf(plain, plain_locs.0, 4), [0.25, 0.5, 0.75, 1.0], "{what}: off");
            assert_eq!(getf(plain, plain_locs.1, 4), [1.0, 2.0, 3.0, 4.0], "{what}: col");
        };

        /* the first program goes: the two after it move down */
        delete(first);
        check_many(many, true);
        assert_eq!(many_locs_now(), many_locs, "many's locations after the delete");
        plain_check("after the delete");
        draws(many, "many, after the delete");
        let file = draws(plain, "plain, after the delete");
        assert!(holds(&file, &bits(&[1.0, 2.0, 3.0, 4.0])), "col in the constant file");

        /* a new program takes a range after them; relinking `many` gives its
         * range back and takes a new one, its values start at 0 again */
        let second = link(VS_MIX, FS_MIX);
        relink(many);
        check_many(many, false);
        plain_check("after the relink");
        // SAFETY: plain GL calls with the program current.
        unsafe {
            glUseProgram(second);
            glUniform1i(loc(second, "count"), 9);
        }
        assert_eq!(geti(second, loc(second, "count"), 1), [9]);
        // SAFETY: a plain GL call.
        unsafe { glUseProgram(many) };
        set_many(many);
        check_many(many, true);
        assert_eq!(geti(second, loc(second, "count"), 1), [9], "second after many's relink and setters");
        plain_check("after many's setters");

        /* `many` goes: the ones after it keep their answers */
        delete(many);
        plain_check("after many's delete");
        assert_eq!(geti(second, loc(second, "count"), 1), [9], "second after many's delete");
        draws(second, "second, after many's delete");
        delete(plain);
        delete(second);
    });
}

/* ---- near the limits ---- */

fn limit(pname: GLenum) -> usize {
    let mut v = -1;
    // SAFETY: a plain GL call with room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetIntegerv {pname:#x}");
    v as usize
}

/// GL_MAX_VERTEX_UNIFORM_VECTORS and GL_MAX_FRAGMENT_UNIFORM_VECTORS, checked
/// against the ES 2.0 minimums (table 6.19: 128 and 16).
fn limits() -> (usize, usize) {
    let (mv, mf) = (limit(GL_MAX_VERTEX_UNIFORM_VECTORS), limit(GL_MAX_FRAGMENT_UNIFORM_VECTORS));
    assert!(mv >= 128 && mf >= 16, "the ES 2.0 minimums: {mv} {mf}");
    (mv, mf)
}

/// `vec4 r` = the sum of `a[0].a` .. `a[n - 1].a`, one term a statement into
/// one variable (long sums of many terms run the GC7000's temporaries out
/// before its constant file: the uniforms are what this measures).
fn sum_of(n: usize, a: char) -> String {
    let mut out = " vec4 r = vec4(0.0);\n".to_owned();
    for i in 0..n {
        out += &format!(" r += {a}[{i}].a;\n");
    }
    out
}

fn near_value(stage: usize, i: usize) -> [f32; 4] {
    let b = (stage * 1000 + i) as f32 * 4.0;
    [b + 0.5, b + 1.5, b + 2.5, b + 3.5]
}

/// One struct-array element per vec4 row, `nv` rows in the vertex stage and
/// `nf` in the fragment stage: the link (GLSL ES 1.00 Appendix A.7: what
/// fits the reported vectors under the packing rules links), one active
/// uniform per row, each row its own location, the values read back and
/// carried by the draw, locations past the old index byte once there are
/// more than 256.
fn rows_program(nv: usize, nf: usize) {
    let (vsum, fsum) = (sum_of(nv, 'v'), sum_of(nf, 'f'));
    let vs = format!(
        "attribute vec4 p;\nstruct V {{ vec4 a; }};\nuniform V v[{nv}];\nvoid main() {{\n{vsum} gl_Position = p + r;\n}}\n"
    );
    let fs = format!(
        "precision mediump float;\nstruct F {{ vec4 a; }};\nuniform F f[{nf}];\n\
         void main() {{\n{fsum} gl_FragColor = r;\n}}\n"
    );
    let p = link(&vs, &fs);
    assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), (nv + nf) as GLint);
    let names = |stage: usize| -> Vec<String> {
        let (c, n) = if stage == 0 { ('v', nv) } else { ('f', nf) };
        (0..n).map(|i| format!("{c}[{i}].a")).collect()
    };
    let mut want: Vec<(String, GLint, GLenum)> =
        (0..2).flat_map(|s| names(s).into_iter().map(|n| (n, 1, GL_FLOAT_VEC4))).collect();
    want.sort();
    assert_eq!(actives(p), want);
    let longest = names(0).iter().chain(names(1).iter()).map(String::len).max().unwrap();
    assert_eq!(iv(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), longest as GLint + 1);

    let mut seen = BTreeMap::new();
    // SAFETY: a plain GL call.
    unsafe { glUseProgram(p) };
    for s in 0..2 {
        for (i, name) in names(s).iter().enumerate() {
            let l = loc(p, name);
            assert!(l >= 0, "{name}: {l}");
            assert!(seen.insert(l, name.clone()).is_none(), "{name} shares location {l:#x}");
            assert_eq!(getf(p, l, 4), [0.0; 4], "{name} before");
            let [x, y, z, w] = near_value(s, i);
            // SAFETY: a plain GL call with the program current.
            unsafe { glUniform4f(l, x, y, z, w) };
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the setters");
    if nv + nf > 256 {
        assert!(seen.keys().any(|&l| l & 0xffff > 255), "{seen:?}");
    }
    for s in 0..2 {
        for (i, name) in names(s).iter().enumerate() {
            assert_eq!(getf(p, loc(p, name), 4), near_value(s, i), "{name}");
        }
    }
    let file = draws(p, "near the limits");
    for i in 0..nv {
        assert!(holds(&file, &bits(&near_value(0, i))), "v[{i}].a in the constant file");
    }
    for i in 0..nf {
        assert!(holds(&file, &bits(&near_value(1, i))), "f[{i}].a in the constant file");
    }
    delete(p);
}

/// More than 255 rows in the program and in the vertex stage, as close to
/// the reported limits as the one 320-register constant file the two stages
/// share holds (src/vivante/shader.rs `constant_files_fit`): 300 vertex rows
/// and 15 fragment rows, 315 active uniforms, a few registers left for
/// literals.
#[test]
fn a_program_of_more_than_255_rows_in_the_constant_file() {
    on_gc7000(|| {
        let (mv, mf) = limits();
        assert!(mv >= 300 && mf >= 15, "the reported limits: {mv} {mf}");
        rows_program(300, 15);
    });
}

/// One row under each stage's reported limit (383 + 383: the context reports
/// 384 vectors a stage, src/gl/context.rs MAX_UNIFORM_DWORDS). GLSL ES 1.00
/// Appendix A.7 says a program within the reported vectors links; on the
/// GC7000 the two stages share one file of 320 registers (GC_NUM_CONSTANTS)
/// and the link refuses this program ("the constant registers the two
/// stages need ... exceed the 320 this GPU has"). Known defect, kept here
/// with its assertion: GL_MAX_VERTEX_UNIFORM_VECTORS and
/// GL_MAX_FRAGMENT_UNIFORM_VECTORS on vivante promise more than the
/// constant file holds (docs/notes/gc-ubo-const-limit.md is the ES 3.0 side
/// of the same 320). No dEQP-GLES2 mustpass case links a program this big.
#[test]
#[ignore = "defect: vivante reports 384 + 384 uniform vectors, but the stages share 320 constant registers"]
fn a_program_near_both_reported_uniform_limits() {
    on_gc7000(|| {
        let (mv, mf) = limits();
        rows_program(mv - 1, mf - 1);
    });
}
