// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 program binaries (G7; ES 3.0.6 2.12.4; docs/notes/es3-program-binary.md,
//! the design this file pins, section (g)): `GL_NUM_PROGRAM_BINARY_FORMATS` 1
//! with the one format `GL_PROGRAM_BINARY_FORMAT_YAKOGL` (0x6B47),
//! `glGetProgramBinary`, `glProgramBinary` and `GL_PROGRAM_BINARY_LENGTH`.
//!
//! The dEQP-GLES3 groups are `shader_api.program_binary.*`,
//! `state_query.shader.program_binary` and
//! `negative_api.shader.{get_program_binary,program_binary,program_parameteri}`.
//! The patched VK-GL-CTS 93bca01 was not in the dependency cache when this
//! file was written, so the sequences of cases 3, 4, 5, 10 and 11 are
//! written from the design's description of `es3fShaderApiTests.cpp`,
//! `es3fShaderStateQueryTests.cpp` and `es3fNegativeShaderApiTests.cpp`
//! (design section (a)), not copied from those sources.
//!
//! Until `g7-impl` wires the entry points (`glGetProgramBinary` answers
//! `GL_INVALID_OPERATION`, `GL_NUM_PROGRAM_BINARY_FORMATS` is 0), each case
//! that asserts the new behaviour is `#[ignore]`d with a reason naming
//! g7-impl. A case whose every assertion holds today (case 10) is not
//! ignored. No GPU runs: the stand-in back end of
//! `es3_program_binary_hint.rs` draws nothing.
//!
//! Not in a `report-es2` build: ES 2.0 has no program binaries. That half is
//! pinned by the `es3.rs` unit test and `es2_error_rules_state` /
//! `es2_report_guard` (design (f)). Mali is not a row: its compiler refuses
//! every program.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_program_binary
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test es3_program_binary
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_program_binary
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    any(feature = "gpu-powervr", feature = "gpu-vivante", feature = "gpu-v3d")
))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::shaderc::sha256::sha256;

/// The one format (design (b)); the constant lives in `src/gl/program_binary.rs`
/// once g7-format has landed, and this file does not depend on it.
const FORMAT: GLenum = 0x6B47;
/// Header bytes, and the offsets of its fields (design (b), Header).
const HEADER: usize = 80;
const OFF_MAGIC: usize = 0;
const OFF_VERSION: usize = 4;
const OFF_HEADER_SIZE: usize = 6;
const OFF_PAYLOAD_BYTES: usize = 8;
const OFF_GPU: usize = 12;
const OFF_IDENTITY: usize = 16;
const OFF_HASH: usize = 48;

/* ---- a context on the host (the rig of es3_program_binary_hint.rs) -------- */

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
    name: c"es3-program-binary".as_ptr() as *const c_char,
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
    // SAFETY: the tables are statics; the lock serialises the tests.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!context::gl_context_create(16, 16).is_null());
        });
        // back to a known state after the previous test
        if geti(GL_TRANSFORM_FEEDBACK_ACTIVE) == 1 {
            glEndTransformFeedback();
        }
        glBindTransformFeedback(GL_TRANSFORM_FEEDBACK, 0);
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/* ---- programs and binaries ------------------------------------------------ */

const VS: &str = "#version 300 es\nin highp vec4 a_position;\nout highp vec4 v;\nout highp float f;\n\
    void main() { v = a_position; f = a_position.x; gl_Position = a_position; }\n";
const FS: &str = "#version 300 es\nlayout(location = 0) out mediump vec4 o_fragColor;\n\
    void main() { o_fragColor = vec4(1.0); }\n";

fn shader_src(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        s
    }
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let s = shader_src(ty, text);
    // SAFETY: plain GL calls.
    unsafe {
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
    }
    s
}

/// A program and its vertex shader object.
struct Prog {
    p: GLuint,
    vs: GLuint,
}

/// A program with `vs` and `fs` compiled and attached, not linked.
fn attach(vs: &str, fs: &str) -> Prog {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    let p = unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        p
    };
    let _ = f;
    Prog { p, vs: v }
}

fn link(p: GLuint) -> bool {
    // SAFETY: plain GL calls.
    unsafe { glLinkProgram(p) };
    progi(p, GL_LINK_STATUS) == 1
}

/// A linked program of the stock sources; the link must succeed.
fn linked() -> Prog {
    let pr = attach(VS, FS);
    assert!(link(pr.p), "{}", log(pr.p));
    pr
}

fn varyings(p: GLuint, names: &[&str], mode: GLenum) {
    let names: Vec<CString> = names.iter().map(|n| CString::new(*n).unwrap()).collect();
    let ptrs: Vec<*const c_char> = names.iter().map(|n| n.as_ptr()).collect();
    // SAFETY: plain GL call; every pointer outlives it.
    unsafe { glTransformFeedbackVaryings(p, ptrs.len() as GLsizei, ptrs.as_ptr(), mode) };
}

fn progi(p: GLuint, pname: GLenum) -> GLint {
    let mut v = i32::MIN;
    // SAFETY: room for one value.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    assert_eq!(err(), GL_NO_ERROR, "glGetProgramiv({pname:#x})");
    v
}

fn log(p: GLuint) -> String {
    let mut buf = [0u8; 1024];
    let mut len: GLsizei = 0;
    // SAFETY: plain GL call; the buffer is as long as it says.
    unsafe { glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr() as *mut c_char) };
    String::from_utf8_lossy(&buf[..len.max(0) as usize]).into_owned()
}

fn uloc(p: GLuint, name: &str) -> GLint {
    let n = CString::new(name).unwrap();
    // SAFETY: plain GL call.
    unsafe { glGetUniformLocation(p, n.as_ptr()) }
}

fn aloc(p: GLuint, name: &str) -> GLint {
    let n = CString::new(name).unwrap();
    // SAFETY: plain GL call.
    unsafe { glGetAttribLocation(p, n.as_ptr()) }
}

/// The binary of `p`, taken the way dEQP's `getProgramBinary` does: the
/// length from GL_PROGRAM_BINARY_LENGTH, then one call, no error allowed.
/// (format, bytes); the bytes are empty for a length of 0.
fn get_binary(p: GLuint) -> (GLenum, Vec<u8>) {
    let len = progi(p, GL_PROGRAM_BINARY_LENGTH);
    if len <= 0 {
        return (0, Vec::new());
    }
    let mut buf = vec![0u8; len as usize];
    let (mut written, mut format) = (-1, 0);
    // SAFETY: the buffer holds `len` bytes.
    unsafe { glGetProgramBinary(p, len, &mut written, &mut format, buf.as_mut_ptr().cast()) };
    assert_eq!(err(), GL_NO_ERROR, "glGetProgramBinary");
    assert_eq!(written, len, "the bytes written are the length asked");
    (format, buf)
}

fn load(p: GLuint, format: GLenum, bytes: &[u8]) {
    // SAFETY: the slice holds `len` bytes.
    unsafe { glProgramBinary(p, format, bytes.as_ptr().cast(), bytes.len() as GLsizei) };
}

fn put_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

/// Gives the header the payload hash of the bytes after it, so that only the
/// parser can find what is wrong with the payload.
fn rehash(b: &mut [u8]) {
    let h = sha256(&b[HEADER..]);
    b[OFF_HASH..OFF_HASH + 32].copy_from_slice(&h);
}

/// What a rejected binary leaves (design (b), (d)): link status FALSE, no GL
/// error, a log that says why, length 0 and a program that cannot be used.
fn assert_rejected(p: GLuint, what: &str) {
    assert_eq!(err(), GL_NO_ERROR, "{what}: no GL error");
    assert_eq!(progi(p, GL_LINK_STATUS), 0, "{what}: link status");
    assert!(!log(p).is_empty(), "{what}: the info log names the reason");
    assert_eq!(progi(p, GL_PROGRAM_BINARY_LENGTH), 0, "{what}: length");
    // SAFETY: plain GL call.
    unsafe { glUseProgram(p) };
    assert_eq!(err(), GL_INVALID_OPERATION, "{what}: not usable");
}

/* ---- the cases of design (g) ---------------------------------------------- */

/// Case 1: `GL_NUM_PROGRAM_BINARY_FORMATS` is 1 and the list is `[0x6B47]` through
/// the three integer query forms.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn formats_are_the_one_yakogl_format() {
    let _g = context();
    assert_eq!(geti(GL_NUM_PROGRAM_BINARY_FORMATS), 1);
    // each form writes one value; the neighbours stay untouched
    let mut i = [i32::MIN; 3];
    let mut l = [i64::MIN; 3];
    let mut b = [9u8; 3];
    // SAFETY: room for three values in each array, one is written.
    unsafe {
        glGetIntegerv(GL_PROGRAM_BINARY_FORMATS, i.as_mut_ptr().add(1));
        glGetInteger64v(GL_PROGRAM_BINARY_FORMATS, l.as_mut_ptr().add(1));
        glGetBooleanv(GL_PROGRAM_BINARY_FORMATS, b.as_mut_ptr().add(1));
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(i, [i32::MIN, FORMAT as i32, i32::MIN]);
    assert_eq!(l, [i64::MIN, FORMAT as i64, i64::MIN]);
    assert_eq!(b, [9, 1, 9], "a non-zero value is TRUE");
}

/// Case 2: Length, and a binary loaded into a new program is the same program.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn length_and_round_trip() {
    let _g = context();
    let un = attach(VS, FS);
    assert_eq!(progi(un.p, GL_PROGRAM_BINARY_LENGTH), 0, "unlinked");
    let src = linked();
    let len = progi(src.p, GL_PROGRAM_BINARY_LENGTH);
    assert!(len > HEADER as GLint, "length {len}");
    let (fmt, bytes) = get_binary(src.p);
    assert_eq!(fmt, FORMAT);
    assert_eq!(bytes.len(), len as usize);

    let dst = attach(VS, FS);
    load(dst.p, fmt, &bytes);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(progi(dst.p, GL_LINK_STATUS), 1, "{}", log(dst.p));
    for q in [GL_ACTIVE_UNIFORMS, GL_ACTIVE_ATTRIBUTES, GL_ACTIVE_UNIFORM_MAX_LENGTH, GL_ACTIVE_ATTRIBUTE_MAX_LENGTH] {
        assert_eq!(progi(dst.p, q), progi(src.p, q), "{q:#x}");
    }
    assert_eq!(aloc(dst.p, "a_position"), aloc(src.p, "a_position"));
    assert!(aloc(dst.p, "a_position") >= 0);
    assert_eq!(progi(dst.p, GL_PROGRAM_BINARY_LENGTH), len);
    assert_eq!(get_binary(dst.p), (fmt, bytes), "the loaded bytes are kept");
}

/// Case 3: dEQP `shader_api.program_binary.simple.get_program_binary_vertex_fragment`:
/// link `gl_Position = vec4(0.0)` / `o_fragColor = vec4(0.0)`, get the binary,
/// and its format must be one of `GL_PROGRAM_BINARY_FORMATS`.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn deqp_get_program_binary_vertex_fragment() {
    let _g = context();
    let n = geti(GL_NUM_PROGRAM_BINARY_FORMATS);
    assert!(n > 0, "dEQP reports NotSupported for an empty list");
    let mut formats = vec![0i32; n as usize];
    // SAFETY: room for `n` values.
    unsafe { glGetIntegerv(GL_PROGRAM_BINARY_FORMATS, formats.as_mut_ptr()) };
    let pr = attach(
        "#version 300 es\nvoid main() { gl_Position = vec4(0.0); }\n",
        "#version 300 es\nlayout(location = 0) out mediump vec4 o_fragColor;\nvoid main() { o_fragColor = vec4(0.0); }\n",
    );
    assert!(link(pr.p), "{}", log(pr.p));
    let (fmt, bytes) = get_binary(pr.p);
    assert!(!bytes.is_empty(), "length > 0, or dEQP's format check fails");
    assert!(formats.contains(&(fmt as i32)), "Invalid format {fmt:#x}");
    assert_eq!(err(), GL_NO_ERROR);
}

/// Case 4: dEQP `uniform_reset_on_binary_load`: set bool, int and float uniforms,
/// draw, load the saved binary into the same current program; all read 0.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn deqp_uniform_reset_on_binary_load() {
    let _g = context();
    let vs = "#version 300 es\nin highp vec4 a_position;\nuniform bool u_boolVar;\nuniform highp int u_intVar;\n\
        uniform highp float u_floatVar;\nvoid main() {\n \
        highp float s = (u_boolVar ? 1.0 : 0.0) + float(u_intVar) + u_floatVar;\n \
        gl_Position = a_position + vec4(s * 0.0); }\n";
    let fs = "#version 300 es\nlayout(location = 0) out mediump vec4 o_fragColor;\nuniform bool u_boolVar;\n\
        uniform highp int u_intVar;\nuniform highp float u_floatVar;\nvoid main() {\n \
        highp float s = (u_boolVar ? 1.0 : 0.0) + float(u_intVar) + u_floatVar;\n \
        o_fragColor = vec4(s * 0.0); }\n";
    let pr = attach(vs, fs);
    assert!(link(pr.p), "the link must succeed: {}", log(pr.p));
    let (fmt, bytes) = get_binary(pr.p);
    assert_eq!(fmt, FORMAT);
    let (lb, li, lf) = (uloc(pr.p, "u_boolVar"), uloc(pr.p, "u_intVar"), uloc(pr.p, "u_floatVar"));
    assert!(lb >= 0 && li >= 0 && lf >= 0);
    let quad: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
    // SAFETY: plain GL calls; the client array outlives the draws.
    unsafe {
        glUseProgram(pr.p);
        glUniform1i(lb, GL_TRUE as GLint);
        glUniform1i(li, 417);
        glUniform1f(lf, 733.0);
        let a = aloc(pr.p, "a_position") as GLuint;
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glEnableVertexAttribArray(a);
        glVertexAttribPointer(a, 4, GL_FLOAT, GL_FALSE as GLboolean, 0, quad.as_ptr().cast());
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        assert_eq!(err(), GL_NO_ERROR, "the first draw");
        let (mut b, mut i, mut f) = (-1, -1, -1.0f32);
        glGetUniformiv(pr.p, lb, &mut b);
        glGetUniformiv(pr.p, li, &mut i);
        glGetUniformfv(pr.p, lf, &mut f);
        assert_eq!((b, i, f), (1, 417, 733.0), "the uniforms were set");

        load(pr.p, fmt, &bytes); /* into the same program, which is current */
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(progi(pr.p, GL_LINK_STATUS), 1, "{}", log(pr.p));
        glGetUniformiv(pr.p, lb, &mut b);
        glGetUniformiv(pr.p, li, &mut i);
        glGetUniformfv(pr.p, lf, &mut f);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!((b, i, f), (0, 0, 0.0), "a load resets every uniform");
        let mut cur = 0;
        glGetIntegerv(GL_CURRENT_PROGRAM, &mut cur);
        assert_eq!(cur as GLuint, pr.p, "still current");
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        assert_eq!(err(), GL_NO_ERROR, "and it draws");
        glDisableVertexAttribArray(a);
    }
}

/// Case 5: dEQP `binary_persistence.*`: after a change to one stage that is not a
/// relink, the bytes are the same; a relink after a new source changes them.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn persistence_of_the_kept_binary() {
    let _g = context();
    let vs_src = |k: u32| {
        format!(
            "#version 300 es\nin highp vec4 a_position;\nout highp vec4 v_c;\n\
             void main() {{ v_c = vec4({k}.0 / 255.0); gl_Position = a_position; }}\n"
        )
    };
    let fs_src = |k: u32| {
        format!(
            "#version 300 es\nin highp vec4 v_c;\nlayout(location = 0) out mediump vec4 o_fragColor;\n\
             void main() {{ o_fragColor = v_c + vec4({k}.0 / 255.0); }}\n"
        )
    };
    for stage in [GL_VERTEX_SHADER, GL_FRAGMENT_SHADER] {
        let src = |k| if stage == GL_VERTEX_SHADER { vs_src(k) } else { fs_src(k) };
        for variant in ["detach", "reattach", "delete", "replace", "recompile", "replace_source"] {
            let (v, f) = (shader(GL_VERTEX_SHADER, &vs_src(11)), shader(GL_FRAGMENT_SHADER, &fs_src(23)));
            // SAFETY: plain GL calls.
            let p = unsafe {
                let p = glCreateProgram();
                glAttachShader(p, v);
                glAttachShader(p, f);
                p
            };
            assert!(link(p), "{variant}: {}", log(p));
            let before = get_binary(p);
            assert!(!before.1.is_empty(), "{variant}");
            let s = if stage == GL_VERTEX_SHADER { v } else { f };
            // SAFETY: plain GL calls.
            unsafe {
                match variant {
                    "detach" => glDetachShader(p, s),
                    "reattach" => {
                        glDetachShader(p, s);
                        glAttachShader(p, s);
                    }
                    "delete" => {
                        glDetachShader(p, s);
                        glDeleteShader(s);
                    }
                    "replace" => {
                        glDetachShader(p, s);
                        let n = shader_src(stage, &src(99)); /* new and uncompiled */
                        glAttachShader(p, n);
                    }
                    "recompile" => glCompileShader(s),
                    _ => {
                        let t = CString::new(src(77)).unwrap();
                        let pp: [*const c_char; 1] = [t.as_ptr()];
                        glShaderSource(s, 1, pp.as_ptr(), core::ptr::null());
                        glCompileShader(s);
                    }
                }
            }
            assert_eq!(err(), GL_NO_ERROR, "{variant}");
            assert_eq!(get_binary(p), before, "{variant}: Program binary changed");
            if variant == "replace_source" {
                assert!(link(p), "{}", log(p));
                assert_ne!(get_binary(p).1, before.1, "a relink of the new source gives new bytes");
            }
        }
    }
}

/// Case 6: The binary's attribute assignments are restored; the program's own stay.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn attribute_bindings_come_from_the_binary() {
    let _g = context();
    let src = attach(VS, FS);
    let name = CString::new("a_position").unwrap();
    // SAFETY: plain GL call.
    unsafe { glBindAttribLocation(src.p, 3, name.as_ptr()) };
    assert!(link(src.p), "{}", log(src.p));
    assert_eq!(aloc(src.p, "a_position"), 3);
    let (fmt, bytes) = get_binary(src.p);

    let dst = attach(VS, FS);
    // SAFETY: plain GL call.
    unsafe { glBindAttribLocation(dst.p, 5, name.as_ptr()) };
    load(dst.p, fmt, &bytes);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(progi(dst.p, GL_LINK_STATUS), 1, "{}", log(dst.p));
    assert_eq!(aloc(dst.p, "a_position"), 3, "the binary's assignment");
    assert!(link(dst.p), "{}", log(dst.p));
    assert_eq!(aloc(dst.p, "a_position"), 5, "the program's own binding survived the load");
}

/// Case 7: Transform feedback state is in the binary; the target's pending request
/// is not touched.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn transform_feedback_state_round_trips() {
    let _g = context();
    let src = attach(VS, FS);
    varyings(src.p, &["v", "f"], GL_SEPARATE_ATTRIBS);
    assert!(link(src.p), "{}", log(src.p));
    let (fmt, bytes) = get_binary(src.p);

    let dst = attach(VS, FS);
    varyings(dst.p, &["f"], GL_INTERLEAVED_ATTRIBS); /* the target's own request */
    load(dst.p, fmt, &bytes);
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(progi(dst.p, GL_LINK_STATUS), 1, "{}", log(dst.p));
    assert_eq!(progi(dst.p, GL_TRANSFORM_FEEDBACK_VARYINGS), 2);
    assert_eq!(progi(dst.p, GL_TRANSFORM_FEEDBACK_BUFFER_MODE), GL_SEPARATE_ATTRIBS as GLint);
    for i in 0..2 {
        let one = |p: GLuint| {
            let (mut len, mut size, mut ty) = (0, 0, 0);
            let mut name = [0u8; 64];
            // SAFETY: the name buffer is as long as it says.
            unsafe {
                glGetTransformFeedbackVarying(p, i, 64, &mut len, &mut size, &mut ty, name.as_mut_ptr() as *mut c_char)
            };
            assert_eq!(err(), GL_NO_ERROR);
            (String::from_utf8_lossy(&name[..len as usize]).into_owned(), size, ty)
        };
        assert_eq!(one(dst.p), one(src.p), "varying {i}");
    }
    /* the pending request of the target is what its own link uses */
    assert!(link(dst.p), "{}", log(dst.p));
    assert_eq!(progi(dst.p, GL_TRANSFORM_FEEDBACK_VARYINGS), 1);
    assert_eq!(progi(dst.p, GL_TRANSFORM_FEEDBACK_BUFFER_MODE), GL_INTERLEAVED_ATTRIBS as GLint);
}

/// Case 8: A binary that does not load: link status FALSE, a log, no GL error.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn rejected_binaries_set_the_link_status_and_raise_no_error() {
    let _g = context();
    let src = linked();
    let (fmt, good) = get_binary(src.p);
    assert!(good.len() > HEADER);

    let mut cases: Vec<(String, Vec<u8>)> = Vec::new();
    for (what, off) in [
        ("magic", OFF_MAGIC),
        ("version", OFF_VERSION),
        ("header size", OFF_HEADER_SIZE),
        ("gpu tag", OFF_GPU),
        ("identity", OFF_IDENTITY),
        ("payload hash", OFF_HASH),
        ("payload", HEADER),
    ] {
        let mut b = good.clone();
        b[off] ^= 0x01;
        cases.push((format!("flipped {what}"), b));
    }
    let mut trailing = good.clone();
    trailing.extend_from_slice(&[1, 2, 3, 4]);
    let n = (trailing.len() - HEADER) as u32;
    put_u32(&mut trailing, OFF_PAYLOAD_BYTES, n);
    rehash(&mut trailing);
    cases.push(("trailing payload bytes (rehashed)".into(), trailing));
    cases.push(("truncated length".into(), good[..good.len() - 1].to_vec()));
    cases.push(("length below the header".into(), good[..HEADER - 1].to_vec()));
    cases.push(("length 0".into(), Vec::new()));

    for (what, bytes) in &cases {
        let dst = attach(VS, FS);
        let bind = CString::new("a_position").unwrap();
        // SAFETY: plain GL call.
        unsafe { glBindAttribLocation(dst.p, 6, bind.as_ptr()) };
        load(dst.p, fmt, bytes);
        assert_rejected(dst.p, what);
        assert_eq!(progi(dst.p, GL_ATTACHED_SHADERS), 2, "{what}: the shaders stay attached");
        assert!(link(dst.p), "{what}: {}", log(dst.p));
        assert_eq!(aloc(dst.p, "a_position"), 6, "{what}: the binding stays");
    }
    /* a null binary with a length */
    let dst = attach(VS, FS);
    // SAFETY: a null binary is the case under test.
    unsafe { glProgramBinary(dst.p, fmt, core::ptr::null(), good.len() as GLsizei) };
    assert_rejected(dst.p, "null binary");
}

/// Case 9: The errors of design (d), each call as the table says.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn errors_of_the_three_calls() {
    let _g = context();
    let pr = linked();
    let un = attach(VS, FS);
    let (fmt, bytes) = get_binary(pr.p);
    let len = bytes.len() as GLsizei;
    let mut buf = vec![0u8; bytes.len()];
    let mut get = |p: GLuint, size: GLsizei| -> (GLenum, GLsizei) {
        let (mut written, mut f) = (77, 0);
        // SAFETY: the buffer holds `len` bytes; `size` may claim less or more.
        unsafe { glGetProgramBinary(p, size, &mut written, &mut f, buf.as_mut_ptr().cast()) };
        (err(), written)
    };
    assert_eq!(get(0, len).0, GL_INVALID_VALUE, "name 0");
    assert_eq!(get(pr.vs, len).0, GL_INVALID_OPERATION, "a shader name");
    assert_eq!(get(un.p, len).0, GL_INVALID_OPERATION, "unlinked");
    for size in [0, len - 1, -1] {
        assert_eq!(get(pr.p, size), (GL_INVALID_OPERATION, 0), "bufSize {size}");
    }
    assert_eq!(get(pr.p, len), (GL_NO_ERROR, len));

    // SAFETY: plain GL calls; the slice holds `len` bytes.
    unsafe {
        let pb = |p: GLuint, f: GLenum, n: GLsizei| {
            glProgramBinary(p, f, bytes.as_ptr().cast(), n);
            err()
        };
        assert_eq!(pb(pr.vs, fmt, len), GL_INVALID_OPERATION, "a shader name");
        assert_eq!(pb(un.p, fmt, -1), GL_INVALID_VALUE, "length -1");
        assert_eq!(pb(pr.p, u32::MAX, len), GL_INVALID_ENUM, "format -1");
        assert_eq!(pb(pr.p, 0, len), GL_INVALID_ENUM, "format 0");
        assert_eq!(progi(pr.p, GL_LINK_STATUS), 1, "a refused format changes nothing");

        glProgramParameteri(0, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint);
        assert_eq!(err(), GL_INVALID_VALUE, "glProgramParameteri(0)");

        /* the program of an active, and of a paused, transform feedback object */
        varyings(pr.p, &["f"], GL_INTERLEAVED_ATTRIBS);
        assert!(link(pr.p), "{}", log(pr.p));
        let (fmt, bytes) = get_binary(pr.p);
        let mut b = 0;
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_TRANSFORM_FEEDBACK_BUFFER, b);
        glBufferData(GL_TRANSFORM_FEEDBACK_BUFFER, 256, core::ptr::null(), GL_DYNAMIC_COPY);
        glBindBufferBase(GL_TRANSFORM_FEEDBACK_BUFFER, 0, b);
        glUseProgram(pr.p);
        glBeginTransformFeedback(GL_POINTS);
        assert_eq!(err(), GL_NO_ERROR, "begin");
        glProgramBinary(pr.p, fmt, bytes.as_ptr().cast(), bytes.len() as GLsizei);
        assert_eq!(err(), GL_INVALID_OPERATION, "active");
        glPauseTransformFeedback();
        glProgramBinary(pr.p, fmt, bytes.as_ptr().cast(), bytes.len() as GLsizei);
        assert_eq!(err(), GL_INVALID_OPERATION, "paused");
        glEndTransformFeedback();
        assert_eq!(progi(pr.p, GL_LINK_STATUS), 1, "the refused load changed nothing");
    }
}

/// Case 10: dEQP `negative_api.shader.get_program_binary` and `program_binary`,
/// step by step as design (a) lists them. Every step holds today (the length
/// is 0 there and the guarded steps are skipped), and with the formats wired
/// the guarded ones run.
#[test]
fn deqp_negative_sequences() {
    let _g = context();
    let program = linked();
    let invalid = attach(VS, FS); /* programInvalid: never linked */
    let buf_size = progi(program.p, GL_PROGRAM_BINARY_LENGTH);
    let mut buf = vec![0u8; buf_size.max(1) as usize + 16];
    let (mut length, mut format): (GLsizei, GLenum) = (-1, 0);

    // get_program_binary
    // SAFETY: the buffer is at least `buf_size` bytes; no call may write more.
    unsafe {
        glGetProgramBinary(program.p, 0, &mut length, &mut format, buf.as_mut_ptr().cast());
        assert_eq!(err(), GL_INVALID_OPERATION, "bufSize 0");
        if buf_size > 0 {
            glGetProgramBinary(program.p, buf_size - 1, &mut length, &mut format, buf.as_mut_ptr().cast());
            assert_eq!(err(), GL_INVALID_OPERATION, "bufSize - 1");
        }
        let n = progi(invalid.p, GL_PROGRAM_BINARY_LENGTH);
        glGetProgramBinary(invalid.p, n, &mut length, &mut format, buf.as_mut_ptr().cast());
        assert_eq!(err(), GL_INVALID_OPERATION, "programInvalid");
    }

    // program_binary: take the source program's binary (no error), then ask for
    // a shader name and for an unlinked program with format -1.
    let (mut length, mut format): (GLsizei, GLenum) = (0, 0);
    if buf_size > 0 {
        // SAFETY: the buffer holds `buf_size` bytes.
        unsafe { glGetProgramBinary(program.p, buf_size, &mut length, &mut format, buf.as_mut_ptr().cast()) };
        assert_eq!(err(), GL_NO_ERROR, "the source program's binary");
    }
    // SAFETY: plain GL calls; `length` is the real length or 0.
    unsafe {
        glProgramBinary(program.vs, format, buf.as_ptr().cast(), length);
        assert_eq!(err(), GL_INVALID_OPERATION, "a shader name");
        glProgramBinary(invalid.p, u32::MAX, buf.as_ptr().cast(), length);
        assert_eq!(err(), GL_INVALID_ENUM, "format -1 on an unlinked program");
    }
}

/// Case 11: dEQP `state_query.shader.program_binary`: the hint is FALSE, TRUE after
/// a set and a relink, and the length is written and > 0; a load keeps the hint.
#[test]
#[ignore = "g7-impl: program binaries are not wired yet (docs/notes/es3-program-binary.md (e))"]
fn deqp_state_query_hint_and_length() {
    let _g = context();
    let pr = linked();
    assert_eq!(progi(pr.p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT), GL_FALSE as GLint);
    // SAFETY: plain GL call.
    unsafe { glProgramParameteri(pr.p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT, GL_TRUE as GLint) };
    assert_eq!(err(), GL_NO_ERROR);
    assert!(link(pr.p), "{}", log(pr.p));
    assert_eq!(progi(pr.p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT), GL_TRUE as GLint);
    let mut len = i32::MIN; /* a write guard: the query must write it */
    // SAFETY: room for one value.
    unsafe { glGetProgramiv(pr.p, GL_PROGRAM_BINARY_LENGTH, &mut len) };
    assert_eq!(err(), GL_NO_ERROR);
    assert!(len > 0, "length {len}");

    let (fmt, bytes) = get_binary(pr.p);
    let other = attach(VS, FS);
    load(other.p, fmt, &bytes);
    assert_eq!(progi(other.p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT), GL_FALSE as GLint, "a load does not set it");
    load(pr.p, fmt, &bytes);
    assert_eq!(progi(pr.p, GL_PROGRAM_BINARY_RETRIEVABLE_HINT), GL_TRUE as GLint, "a load does not clear it");
}
