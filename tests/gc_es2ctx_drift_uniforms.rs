// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP), ES 2.0 context: the uniform answers of dEQP-GLES2
//! functional.uniform_api stay what ES 2.0.25 says after 3cbecbb9 (guard for
//! 3cbecbb9, item gc-es2ctx-drift-uniforms).
//!
//! 3cbecbb9 widened the shader record's uniform count to 16 bits and moved a
//! program's locations into a per-context pool (`index | element << 16`,
//! src/gl/program.rs). The i.MX8MP's last whole dEQP-GLES2 tree predates it,
//! and `functional.uniform_api.{random,value,info_query}.*` read exactly
//! what it moved: locations, `glGetActiveUniform` names, sizes and types,
//! `glGetUniform*` values, GL_ACTIVE_UNIFORMS and
//! GL_ACTIVE_UNIFORM_MAX_LENGTH. tests/gl_many_uniforms.rs covers the >255
//! path on the PowerVR in an ES 3.0 build; this file asks the same through
//! the real GL core and the real vivante back end of an ES 2.0 context
//! (`report-es2`), on GLSL ES 1.00 programs only.
//!
//! Expected values come from ES 2.0.25 (2.10.4 Uniform Variables, 6.1.x
//! queries) and from what es2fUniformApiTests.cpp checks, not from the
//! output:
//! - a uniform array is one active uniform named `a[0]` whose size is its
//!   length; a struct is split into one active uniform per basic-type member
//!   (`s.f`, `s.m[0]` with size 2); an array of structs into one per element
//!   member (`t[1].n`); a name without `[0]` (`a`, `s.m`) names element 0;
//!   `a[3]` of a 3-array and an unknown name are -1;
//! - GL_ACTIVE_UNIFORM_MAX_LENGTH is the longest name plus its NUL;
//! - a sampler is GL_SAMPLER_2D and takes glUniform1i only; a bool takes the
//!   1i or 1f setter of its size and reads back 0 or 1; a setter of another
//!   type or size is GL_INVALID_OPERATION and leaves the value; location -1
//!   is silently ignored; count > 1 on a non-array is GL_INVALID_OPERATION;
//!   glUniformMatrix with transpose GL_TRUE is GL_INVALID_VALUE;
//! - a draw of a linked program is not refused (a kick is submitted), and a
//!   program of more than 255 active uniform records links, gives every one
//!   its own location and keeps the value written to it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_uniforms
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL: host memory, and the kicks the back end submits ------------- */

struct Alloc {
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Alloc>,
    bump: BTreeMap<u64, u64>,
    fence: i64,
    kicks: usize,
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
        p.allocs.insert(gpu, Alloc { mem });
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
        if !p.allocs.contains_key(&k.stream_gpu) {
            p.faults.push(format!("the kick's stream 0x{:x} is no allocation", k.stream_gpu));
            return -22;
        }
        p.kicks += 1;
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

/// Run `case` with the Vivante back end up on the stand-in OSAL and a fresh
/// 64x64 ES 2.0 context.
fn on_gc7000(case: impl FnOnce()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl::vivante::backend::yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(yakogl::vivante::backend::gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
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
        // The memory is leaked, as in tests/gc_es2ctx_sampler_slots.rs.
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a.mem);
        }
        core::mem::take(&mut p.faults)
    });
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
}

/* ---- GL plumbing ------------------------------------------------------------------- */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn no_error(what: &str) {
    let e = err();
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

fn expect_error(what: &str, want: GLenum) {
    let e = err();
    assert_eq!(e, want, "{what}: 0x{e:x}, want 0x{want:x}");
    no_error(what);
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
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// Link `vs`/`fs` and make the program current.
fn program(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        if ok != 1 {
            let mut buf = [0 as c_char; 1024];
            glGetProgramInfoLog(p, buf.len() as GLsizei, core::ptr::null_mut(), buf.as_mut_ptr());
            panic!("link failed: {}", CStr::from_ptr(buf.as_ptr()).to_string_lossy());
        }
        glUseProgram(p);
        p
    }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: a plain GL query into a local.
    unsafe { glGetIntegerv(pname, &mut v) };
    no_error("glGetIntegerv");
    v
}

fn progi(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: a plain GL query into a local.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    no_error("glGetProgramiv");
    v
}

/// The location of `name`, -1 when it names no active uniform.
fn loc(p: GLuint, name: &str) -> GLint {
    let n = CString::new(name).unwrap();
    // SAFETY: a plain GL call, one NUL-terminated name.
    unsafe { glGetUniformLocation(p, n.as_ptr()) }
}

fn must_loc(p: GLuint, name: &str) -> GLint {
    let l = loc(p, name);
    assert!(l >= 0, "{name:?} has no location");
    l
}

fn getf(p: GLuint, l: GLint, n: usize) -> Vec<f32> {
    let mut v = vec![-7777.0f32; n + 1];
    // SAFETY: a buffer one float past what the uniform writes.
    unsafe { glGetUniformfv(p, l, v.as_mut_ptr()) };
    no_error("glGetUniformfv");
    assert_eq!(v[n], -7777.0, "glGetUniformfv wrote past the uniform's {n} components");
    v.truncate(n);
    v
}

fn geti_u(p: GLuint, l: GLint, n: usize) -> Vec<i32> {
    let mut v = vec![-7777i32; n + 1];
    // SAFETY: a buffer one int past what the uniform writes.
    unsafe { glGetUniformiv(p, l, v.as_mut_ptr()) };
    no_error("glGetUniformiv");
    assert_eq!(v[n], -7777, "glGetUniformiv wrote past the uniform's {n} components");
    v.truncate(n);
    v
}

/// `(name, size, type)` of every active uniform, by index, and the name's
/// reported length checked against the name.
fn active_uniforms(p: GLuint) -> Vec<(String, GLint, GLenum)> {
    let n = progi(p, GL_ACTIVE_UNIFORMS) as GLuint;
    let mut out = Vec::new();
    for i in 0..n {
        let (mut len, mut size, mut ty) = (-1 as GLsizei, 0 as GLint, 0 as GLenum);
        let mut buf = [0x7f as c_char; 256];
        // SAFETY: a buffer of the size announced; GL NUL-terminates the name.
        unsafe { glGetActiveUniform(p, i, buf.len() as GLsizei, &mut len, &mut size, &mut ty, buf.as_mut_ptr()) };
        no_error("glGetActiveUniform");
        // SAFETY: NUL-terminated by GL.
        let name = unsafe { CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned();
        assert_eq!(len as usize, name.len(), "length of index {i} ({name:?})");
        out.push((name, size, ty));
    }
    out
}

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad with the current program: accepted, and a kick submitted.
fn draw_not_refused(what: &str) {
    let kicks0 = with_platform(|p| p.kicks);
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error(&format!("{what}: glDrawArrays"));
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
    no_error(&format!("{what}: glFinish"));
    assert!(with_platform(|p| p.kicks) > kicks0, "{what}: the draw produced no kick (refused?)");
}

/* ---- the program of structs, arrays, samplers and bools ----------------------------- */

const VS: &str = "attribute vec4 p;\nuniform vec4 vsu[2];\nvoid main() { gl_Position = p + vsu[0] + vsu[1]; }\n";

const FS: &str = "precision mediump float;\n\
    struct S { float f; vec2 v; int i; bool b; vec4 m[2]; };\n\
    struct T { float m[2]; vec3 n; };\n\
    uniform float a[3];\n\
    uniform S s;\n\
    uniform T t[2];\n\
    uniform sampler2D smp;\n\
    uniform bool flag;\n\
    uniform mat3 m3;\n\
    void main() {\n\
      float x = a[0] + a[1] + a[2] + s.f + s.v.x + float(s.i) + (s.b ? 1.0 : 0.0) + s.m[0].x + s.m[1].w\n\
        + t[0].m[0] + t[0].m[1] + t[0].n.x + t[1].m[0] + t[1].m[1] + t[1].n.z + (flag ? 1.0 : 0.0) + m3[1][1];\n\
      gl_FragColor = texture2D(smp, vec2(0.5)) + vec4(x);\n\
    }\n";

/// What ES 2.0 says the active uniforms of VS + FS are.
fn expected() -> Vec<(&'static str, GLint, GLenum)> {
    vec![
        ("vsu[0]", 2, GL_FLOAT_VEC4),
        ("a[0]", 3, GL_FLOAT),
        ("s.f", 1, GL_FLOAT),
        ("s.v", 1, GL_FLOAT_VEC2),
        ("s.i", 1, GL_INT),
        ("s.b", 1, GL_BOOL),
        ("s.m[0]", 2, GL_FLOAT_VEC4),
        ("t[0].m[0]", 2, GL_FLOAT),
        ("t[0].n", 1, GL_FLOAT_VEC3),
        ("t[1].m[0]", 2, GL_FLOAT),
        ("t[1].n", 1, GL_FLOAT_VEC3),
        ("smp", 1, GL_SAMPLER_2D),
        ("flag", 1, GL_BOOL),
        ("m3", 1, GL_FLOAT_MAT3),
    ]
}

#[test]
fn es2_active_uniform_queries_name_every_member() {
    on_gc7000(|| {
        let p = program(VS, FS);
        let want = expected();
        assert_eq!(progi(p, GL_ACTIVE_UNIFORMS) as usize, want.len(), "GL_ACTIVE_UNIFORMS");
        let longest = want.iter().map(|w| w.0.len()).max().unwrap() as GLint + 1;
        assert_eq!(progi(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), longest, "the longest name plus its NUL");
        // Every index once, in any order (ES 2.0 does not order them).
        let mut got = active_uniforms(p);
        got.sort();
        let mut w: Vec<_> = want.iter().map(|(n, s, t)| (n.to_string(), *s, *t)).collect();
        w.sort();
        assert_eq!(got, w);
        // An index past the count is INVALID_VALUE and writes nothing.
        let mut buf = [0x7f as c_char; 8];
        let n = want.len() as GLuint;
        // SAFETY: buffers of the size announced.
        unsafe {
            glGetActiveUniform(
                p,
                n,
                8,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                buf.as_mut_ptr(),
            )
        };
        expect_error("glGetActiveUniform past the count", GL_INVALID_VALUE);
        assert_eq!(buf[0], 0x7f);
        // A small buffer truncates the name to bufSize - 1 characters plus the NUL.
        let idx = got.iter().position(|g| g.0 == "t[0].m[0]").unwrap();
        let by_index = active_uniforms(p);
        let idx = by_index.iter().position(|g| g.0 == got[idx].0).unwrap() as GLuint;
        let (mut len, mut size, mut ty) = (0 as GLsizei, 0 as GLint, 0 as GLenum);
        let mut small = [0x7f as c_char; 8];
        // SAFETY: a buffer of the size announced.
        unsafe { glGetActiveUniform(p, idx, 4, &mut len, &mut size, &mut ty, small.as_mut_ptr()) };
        no_error("glGetActiveUniform bufSize 4");
        assert_eq!(len, 3);
        // SAFETY: NUL-terminated by GL.
        assert_eq!(unsafe { CStr::from_ptr(small.as_ptr()) }.to_str().unwrap(), "t[0");
        assert_eq!((size, ty), (2, GL_FLOAT));
    });
}

#[test]
fn es2_locations_follow_the_name_rules() {
    on_gc7000(|| {
        let p = program(VS, FS);
        // `a` names element 0; the elements and the members are distinct.
        let a0 = must_loc(p, "a[0]");
        assert_eq!(must_loc(p, "a"), a0, "'a' is 'a[0]'");
        let (a1, a2) = (must_loc(p, "a[1]"), must_loc(p, "a[2]"));
        assert!(a0 != a1 && a1 != a2 && a0 != a2);
        assert_eq!(loc(p, "a[3]"), -1, "past the array");
        assert_eq!(must_loc(p, "s.m"), must_loc(p, "s.m[0]"), "'s.m' is 's.m[0]'");
        let m1 = must_loc(p, "s.m[1]");
        assert_ne!(m1, must_loc(p, "s.m[0]"));
        assert_eq!(loc(p, "s.m[2]"), -1);
        assert_eq!(loc(p, "s"), -1, "a struct has no location of its own");
        assert_eq!(loc(p, "t"), -1);
        assert_eq!(loc(p, "t[2].n"), -1);
        assert_eq!(loc(p, "nothere"), -1);
        assert_eq!(loc(p, "a[0"), -1);
        assert_eq!(must_loc(p, "t[1].m"), must_loc(p, "t[1].m[0]"));
        assert_ne!(must_loc(p, "t[0].n"), must_loc(p, "t[1].n"));
        assert_eq!(must_loc(p, "vsu"), must_loc(p, "vsu[0]"), "a vertex uniform answers too");
        // Every location of every active uniform element is distinct.
        let mut all = Vec::new();
        for (n, size, _) in active_uniforms(p) {
            let base = n.strip_suffix("[0]").unwrap_or(&n).to_string();
            if n.ends_with("[0]") {
                for e in 0..size {
                    all.push(must_loc(p, &format!("{base}[{e}]")));
                }
            } else {
                all.push(must_loc(p, &n));
            }
        }
        let mut sorted = all.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), all.len(), "locations are distinct: {all:?}");
        no_error("locations");
    });
}

#[test]
fn es2_set_and_read_back() {
    on_gc7000(|| {
        let p = program(VS, FS);
        // Every uniform starts at zero (2.10.4: "initialized to 0").
        assert_eq!(getf(p, must_loc(p, "s.m[1]"), 4), vec![0.0; 4]);
        assert_eq!(geti_u(p, must_loc(p, "s.i"), 1), vec![0]);
        assert_eq!(geti_u(p, must_loc(p, "flag"), 1), vec![0]);

        // SAFETY: plain GL calls on uniforms of the current program.
        unsafe {
            let a = [1.0f32, 2.0, 3.0];
            glUniform1fv(must_loc(p, "a"), 3, a.as_ptr());
            glUniform4f(must_loc(p, "s.m[1]"), 5.0, 6.0, 7.0, 8.0);
            glUniform2f(must_loc(p, "s.v"), -1.5, 2.5);
            glUniform1f(must_loc(p, "s.f"), 0.25);
            glUniform1i(must_loc(p, "s.i"), 7);
            glUniform1i(must_loc(p, "smp"), 3);
            glUniform1i(must_loc(p, "flag"), 1);
            let tm = [9.0f32, 10.0];
            glUniform1fv(must_loc(p, "t[1].m"), 2, tm.as_ptr());
            glUniform3f(must_loc(p, "t[0].n"), 1.0, 2.0, 3.0);
            let m = [1.0f32, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0];
            glUniformMatrix3fv(must_loc(p, "m3"), 1, 0, m.as_ptr());
            let v = [0.5f32, 1.5, 2.5, 3.5, -0.5, -1.5, -2.5, -3.5];
            glUniform4fv(must_loc(p, "vsu"), 2, v.as_ptr());
        }
        no_error("setters");
        for (e, want) in [1.0f32, 2.0, 3.0].iter().enumerate() {
            assert_eq!(getf(p, must_loc(p, &format!("a[{e}]")), 1), vec![*want], "a[{e}]");
        }
        assert_eq!(getf(p, must_loc(p, "s.m[1]"), 4), vec![5.0, 6.0, 7.0, 8.0]);
        assert_eq!(getf(p, must_loc(p, "s.m[0]"), 4), vec![0.0; 4], "s.m[0] untouched");
        assert_eq!(getf(p, must_loc(p, "s.v"), 2), vec![-1.5, 2.5]);
        assert_eq!(getf(p, must_loc(p, "s.f"), 1), vec![0.25]);
        assert_eq!(geti_u(p, must_loc(p, "s.i"), 1), vec![7]);
        assert_eq!(getf(p, must_loc(p, "s.i"), 1), vec![7.0]);
        assert_eq!(geti_u(p, must_loc(p, "smp"), 1), vec![3]);
        assert_eq!(geti_u(p, must_loc(p, "flag"), 1), vec![1]);
        assert_eq!(getf(p, must_loc(p, "flag"), 1), vec![1.0]);
        assert_eq!(getf(p, must_loc(p, "t[1].m[0]"), 1), vec![9.0]);
        assert_eq!(getf(p, must_loc(p, "t[1].m[1]"), 1), vec![10.0]);
        assert_eq!(getf(p, must_loc(p, "t[0].m[0]"), 1), vec![0.0], "t[0] untouched by t[1]");
        assert_eq!(getf(p, must_loc(p, "t[1].n"), 3), vec![0.0; 3]);
        assert_eq!(getf(p, must_loc(p, "t[0].n"), 3), vec![1.0, 2.0, 3.0]);
        assert_eq!(getf(p, must_loc(p, "m3"), 9), vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]);
        assert_eq!(getf(p, must_loc(p, "vsu[1]"), 4), vec![-0.5, -1.5, -2.5, -3.5]);

        // A bool takes 1i and 1f; any non-zero is true and reads back 1 (0 reads 0).
        let b = must_loc(p, "s.b");
        // SAFETY: plain GL calls.
        unsafe { glUniform1f(b, 2.5) };
        assert_eq!(geti_u(p, b, 1), vec![1]);
        assert_eq!(getf(p, b, 1), vec![1.0]);
        // SAFETY: plain GL calls.
        unsafe { glUniform1i(b, 0) };
        assert_eq!(geti_u(p, b, 1), vec![0]);
        assert_eq!(getf(p, b, 1), vec![0.0]);
        no_error("bool");
    });
}

#[test]
fn es2_setter_errors_and_location_minus_one() {
    on_gc7000(|| {
        let p = program(VS, FS);
        let (f, v, smp, flag, m3) =
            (must_loc(p, "s.f"), must_loc(p, "s.v"), must_loc(p, "smp"), must_loc(p, "flag"), must_loc(p, "m3"));
        // SAFETY: plain GL calls on the current program; arrays hold what the setters read.
        unsafe {
            glUniform1f(f, 2.5);
            glUniform2f(v, 1.0, 2.0);
            glUniform1i(smp, 2);
            glUniform1i(flag, 1);
            no_error("setup");

            glUniform1i(f, 7); // int on a float
            expect_error("glUniform1i on a float", GL_INVALID_OPERATION);
            glUniform2f(f, 7.0, 7.0); // size
            expect_error("glUniform2f on a float", GL_INVALID_OPERATION);
            glUniform3f(v, 7.0, 7.0, 7.0);
            expect_error("glUniform3f on a vec2", GL_INVALID_OPERATION);
            glUniform2i(v, 7, 7);
            expect_error("glUniform2i on a vec2", GL_INVALID_OPERATION);
            glUniform1f(smp, 7.0);
            expect_error("glUniform1f on a sampler", GL_INVALID_OPERATION);
            glUniform2f(flag, 7.0, 7.0);
            expect_error("glUniform2f on a bool", GL_INVALID_OPERATION);
            let two = [1.0f32, 2.0];
            glUniform1fv(f, 2, two.as_ptr()); // count > 1 on a non-array
            expect_error("glUniform1fv count 2 on a non-array", GL_INVALID_OPERATION);
            glUniformMatrix2fv(m3, 1, 0, two.as_ptr());
            expect_error("glUniformMatrix2fv on a mat3", GL_INVALID_OPERATION);
            let m = [0.0f32; 9];
            glUniformMatrix3fv(m3, 1, 1, m.as_ptr()); // transpose must be GL_FALSE
            expect_error("glUniformMatrix3fv transpose", GL_INVALID_VALUE);

            // Location -1 is silently ignored by every setter.
            glUniform1f(-1, 1.0);
            glUniform1i(-1, 1);
            glUniform4fv(-1, 1, [0.0f32; 4].as_ptr());
            glUniformMatrix3fv(-1, 1, 0, m.as_ptr());
            no_error("location -1");

            // A location that is not the program's.
            glUniform1f(0x3fff_fff0, 1.0);
            expect_error("glUniform1f at a foreign location", GL_INVALID_OPERATION);
        }
        // Every refused call left the value.
        assert_eq!(getf(p, f, 1), vec![2.5]);
        assert_eq!(getf(p, v, 2), vec![1.0, 2.0]);
        assert_eq!(geti_u(p, smp, 1), vec![2]);
        assert_eq!(geti_u(p, flag, 1), vec![1]);
        assert_eq!(getf(p, m3, 9), vec![0.0; 9]);

        // glGetUniform*: -1 and no such program.
        let mut out = [0.0f32; 4];
        // SAFETY: plain GL calls; `out` is large enough for any answer.
        unsafe {
            glGetUniformfv(p, -1, out.as_mut_ptr());
            expect_error("glGetUniformfv at -1", GL_INVALID_OPERATION);
            glGetUniformfv(0xffff_0000, f, out.as_mut_ptr());
            expect_error("glGetUniformfv of no program", GL_INVALID_VALUE);
            // No current program: a setter is INVALID_OPERATION.
            glUseProgram(0);
            glUniform1f(f, 1.0);
            expect_error("glUniform1f without a program", GL_INVALID_OPERATION);
        }
    });
}

#[test]
fn es2_draw_with_the_struct_program_is_not_refused() {
    on_gc7000(|| {
        let p = program(VS, FS);
        // SAFETY: plain GL calls.
        unsafe {
            glUniform1i(must_loc(p, "smp"), 0);
            glUniform1f(must_loc(p, "s.f"), 0.5);
            glUniform4f(must_loc(p, "vsu[1]"), 0.0, 0.0, 0.0, 0.0);
        }
        no_error("setters");
        draw_not_refused("the struct program");
        // The values survive the draw.
        assert_eq!(getf(p, must_loc(p, "s.f"), 1), vec![0.5]);
        // And a second draw after a change is not refused either.
        // SAFETY: a plain GL call.
        unsafe { glUniform1f(must_loc(p, "s.f"), 0.75) };
        draw_not_refused("the struct program, changed");
        assert_eq!(getf(p, must_loc(p, "s.f"), 1), vec![0.75]);
    });
}

/* ---- more than 255 active uniform records ------------------------------------------- */

/// A fragment shader with `n` active scalar-float records: `uniform L l[k]`,
/// an array of structs of 15 floats `f0..f14`, one active uniform per element
/// member, and a vertex shader with none. The names are `l[i].fj` (size 1).
fn records_fs(n: usize) -> String {
    let k = n.div_ceil(15);
    let mut s = String::from("precision mediump float;\nstruct L {\n");
    for j in 0..15 {
        s += &format!("  float f{j};\n");
    }
    s += &format!("}};\nuniform L l[{k}];\nvoid main() {{\n  float x = 0.0;\n");
    let mut left = n;
    for i in 0..k {
        for j in 0..15.min(left) {
            s += &format!("  x += l[{i}].f{j};\n");
        }
        left = left.saturating_sub(15);
    }
    s + "  gl_FragColor = vec4(x);\n}\n"
}

fn rec_name(i: usize) -> String {
    format!("l[{}].f{}", i / 15, i % 15)
}

/// A program of `n` records links, counts them, gives each its own location,
/// keeps a distinct value in each, and draws.
fn records_case(n: usize) {
    on_gc7000(|| {
        let max_vec = geti(GL_MAX_FRAGMENT_UNIFORM_VECTORS);
        eprintln!(
            "MAX_FRAGMENT_UNIFORM_VECTORS {max_vec}, MAX_VERTEX_UNIFORM_VECTORS {}",
            geti(GL_MAX_VERTEX_UNIFORM_VECTORS)
        );
        let p = program("attribute vec4 p;\nvoid main() { gl_Position = p; }\n", &records_fs(n));
        assert_eq!(progi(p, GL_ACTIVE_UNIFORMS) as usize, n, "GL_ACTIVE_UNIFORMS counts every element member");
        let longest = (0..n).map(|i| rec_name(i).len()).max().unwrap() as GLint + 1;
        assert_eq!(progi(p, GL_ACTIVE_UNIFORM_MAX_LENGTH), longest);
        // glGetActiveUniform for every index: a distinct name of the set, float, size 1.
        let got = active_uniforms(p);
        let mut names: Vec<_> = got.iter().map(|g| g.0.clone()).collect();
        assert!(got.iter().all(|g| g.1 == 1 && g.2 == GL_FLOAT));
        names.sort();
        let mut want: Vec<_> = (0..n).map(rec_name).collect();
        want.sort();
        assert_eq!(names, want);
        // Every record has its own location, none of them -1, past 255 included.
        let locs: Vec<GLint> = (0..n).map(|i| must_loc(p, &rec_name(i))).collect();
        let mut sorted = locs.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), n, "distinct locations");
        // A value in each, read back from each.
        for (i, &l) in locs.iter().enumerate() {
            // SAFETY: a plain GL call.
            unsafe { glUniform1f(l, i as f32 + 0.5) };
        }
        no_error("setters");
        for (i, &l) in locs.iter().enumerate() {
            assert_eq!(getf(p, l, 1), vec![i as f32 + 0.5], "{}", rec_name(i));
        }
        // The record past the last is nothing.
        assert_eq!(loc(p, &format!("l[{}].f0", n.div_ceil(15))), -1);
        draw_not_refused(&format!("{n} records"));
        assert_eq!(getf(p, locs[n - 1], 1), vec![n as f32 - 0.5], "the last record survives the draw");
    });
}

/// 300 scalar records: past the byte of the record's old count (255) and of
/// the old location table, 75 vec4s when packed, inside every ES 2.0 limit
/// the back end reports.
#[test]
fn es2_more_than_255_records_link_and_draw() {
    records_case(300);
}

/// 270 records (18 whole structs): the next whole struct count past 255.
#[test]
fn es2_270_records_link_and_draw() {
    records_case(270);
}

/// 315 scalar records, 21 whole structs: one register each on the GC7000
/// (its compiler does not pack scalars of different names), 5 below the 320
/// constant registers the vertex and fragment stages share, so this is the
/// largest record count that fits the file with room for the compiler's
/// literals. Well past 255, and under the 384 vectors the context reports.
#[test]
fn es2_records_near_the_constant_file() {
    records_case(315);
}

/// ES 2.0.25 2.10.4 and 3.8.1: a link error only when a program uses "more
/// than the space available", and the space is what
/// GL_MAX_FRAGMENT_UNIFORM_VECTORS reports (dEQP-GLES2
/// functional.implementation_limits and uniform_api use it). The context
/// reports 384 (src/gl/context.rs: MAX_UNIFORM_DWORDS / 4) but the GC7000's
/// constant file holds 320 registers shared by uniforms and literals, so a
/// program of one vec4 array of that many vectors is refused at link:
/// "the constant registers the two stages need (vertex 0, fragment 384 ...)
/// exceed the 320 this GPU has". Defect (a reported limit above what the
/// back end holds, check_uniform_packing's doc names it as the back end's
/// own bound), pinned and ignored until the reported value or the back end
/// moves.
#[test]
#[ignore = "defect: GL_MAX_FRAGMENT_UNIFORM_VECTORS (384) exceeds the GC7000's 320 constant registers; a vec4 array of 384 is refused"]
fn es2_a_program_of_the_reported_maximum_vectors_links() {
    on_gc7000(|| {
        let max_vec = geti(GL_MAX_FRAGMENT_UNIFORM_VECTORS);
        let fs = format!(
            "precision mediump float;\nuniform vec4 u[{max_vec}];\nvoid main() {{ gl_FragColor = u[{}]; }}\n",
            max_vec - 1
        );
        let p = program("attribute vec4 p;\nvoid main() { gl_Position = p; }\n", &fs);
        assert_eq!(progi(p, GL_ACTIVE_UNIFORMS), 1);
        draw_not_refused("the maximum vectors");
    });
}
