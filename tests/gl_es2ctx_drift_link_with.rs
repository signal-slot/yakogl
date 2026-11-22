// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 830858db (glLinkProgram links through
//! `link_with(LinkInputs)`, the tf `link_resolve` takes the request) and
//! ae96bfc7 (a failed link's info log goes to the console, once). Both
//! commits moved no answer on purpose; the whole dEQP-GLES2 trees of the
//! i.MX8MP (0fbe61a3) and the Pi 4 (46f93767) predate them. This file pins the
//! link outcomes dEQP-GLES2 reads (shaders.linkage, uniform_api,
//! attribute_location, program_state): GL_LINK_STATUS, the info log,
//! GL_INVALID_OPERATION of glUseProgram on a program that did not link, no
//! error from glLinkProgram itself, the attribute locations after
//! glBindAttribLocation, GL_ATTACHED_SHADERS, GL_ACTIVE_ATTRIBUTES,
//! GL_ACTIVE_UNIFORMS, glValidateProgram. A report-es2 build has no
//! transform feedback: `tf_is_not_reachable` asserts the ES 3.0 queries stay
//! GL_INVALID_ENUM and a link needs no request.
//!
//! One suite, two back ends (the shader compiler differs, the GL core is
//! the same): module `gc` (GPU=vivante) and module `v3d` (GPU=v3d).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gl_es2ctx_drift_link_with
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table,report-es2 --test gl_es2ctx_drift_link_with
//! ```
#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "report-es2",
    any(feature = "gpu-vivante", feature = "gpu-v3d")
))]

macro_rules! suite {
    () => {
use core::ffi::{c_char, c_void, CStr};
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::*;
use yakogl::gl::consts::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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
unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
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
    name: c"gl-es2ctx-link-with".as_ptr() as *const c_char,
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
    max_samples: 0,
};


static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The one ES 2.0 context, the online compiler behind it; serialises the tests.
fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(16, 16).is_null());
        }
    });
    // SAFETY: plain GL calls on the one context.
    unsafe {
        glUseProgram(0);
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn cs(s: &str) -> CString {
    CString::new(s).unwrap()
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let ctext = cs(text);
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [ctext.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "the shader compiles: {text:?}");
        s
    }
}

fn program(shaders: &[GLuint]) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        for &s in shaders {
            glAttachShader(p, s);
        }
        p
    }
}

/// glLinkProgram, asserting it raised no error itself; the link status.
fn link(p: GLuint) -> bool {
    // SAFETY: plain GL calls.
    unsafe {
        glLinkProgram(p);
        assert_eq!(glGetError(), GL_NO_ERROR, "glLinkProgram raises no error");
    }
    iv(p, GL_LINK_STATUS) == GL_TRUE as GLint
}

fn iv(p: GLuint, pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one GLint out.
    unsafe { glGetProgramiv(p, pname, &mut v) };
    v
}

fn log(p: GLuint) -> String {
    let mut buf = [0 as c_char; 2048];
    let mut len: GLsizei = 0;
    // SAFETY: a buffer of the size announced; GL NUL-terminates the log.
    unsafe {
        glGetProgramInfoLog(p, buf.len() as GLsizei, &mut len, buf.as_mut_ptr());
        CStr::from_ptr(buf.as_ptr()).to_string_lossy().into_owned()
    }
}

fn attrib(p: GLuint, name: &str) -> GLint {
    let n = cs(name);
    // SAFETY: a NUL-terminated name.
    unsafe { glGetAttribLocation(p, n.as_ptr()) }
}

fn uniform(p: GLuint, name: &str) -> GLint {
    let n = cs(name);
    // SAFETY: a NUL-terminated name.
    unsafe { glGetUniformLocation(p, n.as_ptr()) }
}

fn current_program() -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(GL_CURRENT_PROGRAM, &mut v) };
    v
}

/// The program did not link: status FALSE, a log, glUseProgram refuses.
fn assert_failed(p: GLuint, why: &str) {
    assert_eq!(iv(p, GL_LINK_STATUS), GL_FALSE as GLint, "{why}: GL_LINK_STATUS is FALSE");
    assert!(!log(p).trim().is_empty(), "{why}: the info log is not empty");
    assert!(iv(p, GL_INFO_LOG_LENGTH) > 1, "{why}: GL_INFO_LOG_LENGTH counts the log");
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "{why}: glUseProgram of an unlinked program");
        glValidateProgram(p);
        assert_eq!(glGetError(), GL_NO_ERROR, "{why}: glValidateProgram raises no error");
        assert_eq!(iv(p, GL_VALIDATE_STATUS), GL_FALSE as GLint, "{why}: validate FALSE");
    }
}

const VS: &str = "attribute vec4 a_pos;\nattribute vec2 a_uv;\nvarying vec2 v_uv;\nuniform mat4 u_mvp;\nvoid main() { v_uv = a_uv; gl_Position = u_mvp * a_pos; }\n";
const FS: &str = "precision mediump float;\nvarying vec2 v_uv;\nuniform vec4 u_tint;\nuniform sampler2D u_tex;\nvoid main() { gl_FragColor = u_tint * texture2D(u_tex, v_uv); }\n";
/// The same stages with a second uniform and no sampler.
const FS2: &str = "precision mediump float;\nvarying vec2 v_uv;\nuniform vec4 u_tint;\nuniform vec4 u_extra;\nvoid main() { gl_FragColor = u_tint + u_extra + vec4(v_uv, 0.0, 0.0); }\n";

#[test]
fn a_program_links_and_reports_its_interface() {
    let _g = context();
    let (vs, fs) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    let p = program(&[vs, fs]);
    assert_eq!(iv(p, GL_ATTACHED_SHADERS), 2);
    assert_eq!(iv(p, GL_LINK_STATUS), GL_FALSE as GLint, "not linked yet");
    assert!(link(p), "log: {}", log(p));
    assert_eq!(iv(p, GL_ATTACHED_SHADERS), 2);
    assert_eq!(iv(p, GL_ACTIVE_ATTRIBUTES), 2);
    assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 3);
    assert!(attrib(p, "a_pos") >= 0 && attrib(p, "a_uv") >= 0);
    assert_ne!(attrib(p, "a_pos"), attrib(p, "a_uv"));
    for u in ["u_mvp", "u_tint", "u_tex"] {
        assert!(uniform(p, u) >= 0, "{u} has a location");
    }
    assert_eq!(uniform(p, "nothing"), -1);
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(current_program(), p as GLint);
        glValidateProgram(p);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(iv(p, GL_VALIDATE_STATUS), GL_TRUE as GLint, "a linked program validates");
        glUseProgram(0);
    }
}

#[test]
fn relink_follows_glbindattriblocation() {
    let _g = context();
    let (vs, fs) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    let p = program(&[vs, fs]);
    // SAFETY: plain GL calls; names are NUL-terminated.
    unsafe {
        let (a, b) = (cs("a_pos"), cs("a_uv"));
        glBindAttribLocation(p, 5, a.as_ptr());
        glBindAttribLocation(p, 2, b.as_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert!(link(p), "log: {}", log(p));
        assert_eq!((attrib(p, "a_pos"), attrib(p, "a_uv")), (5, 2));
        /* a binding made after the link takes effect at the next link only */
        glBindAttribLocation(p, 1, a.as_ptr());
        glBindAttribLocation(p, 6, b.as_ptr());
        assert_eq!((attrib(p, "a_pos"), attrib(p, "a_uv")), (5, 2));
        assert!(link(p), "log: {}", log(p));
        assert_eq!((attrib(p, "a_pos"), attrib(p, "a_uv")), (1, 6));
        assert_eq!(iv(p, GL_ACTIVE_ATTRIBUTES), 2);
        /* an index past GL_MAX_VERTEX_ATTRIBS is refused at the bind */
        let mut max: GLint = 0;
        glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut max);
        glBindAttribLocation(p, max as GLuint, a.as_ptr());
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        /* a gl_ name is reserved */
        let g = cs("gl_Foo");
        glBindAttribLocation(p, 3, g.as_ptr());
        assert_eq!(glGetError(), GL_INVALID_OPERATION);
    }
}

#[test]
fn relink_after_detach_and_replace() {
    let _g = context();
    let (vs, fs, fs2) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS), shader(GL_FRAGMENT_SHADER, FS2));
    let p = program(&[vs, fs]);
    assert!(link(p), "log: {}", log(p));
    assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 3);
    // SAFETY: plain GL calls.
    unsafe {
        /* the detach alone changes nothing of the executable */
        glDetachShader(p, fs);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(iv(p, GL_ATTACHED_SHADERS), 1);
        assert_eq!(iv(p, GL_LINK_STATUS), GL_TRUE as GLint);
        assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 3);
        /* a relink with no fragment shader fails */
        assert!(!link(p));
        assert_failed(p, "no fragment shader after the detach");
        assert_eq!(iv(p, GL_ATTACHED_SHADERS), 1);
        /* the replacement links, with the replacement's uniforms */
        glAttachShader(p, fs2);
        assert_eq!(iv(p, GL_ATTACHED_SHADERS), 2);
        assert!(link(p), "log: {}", log(p));
        assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 3, "u_mvp, u_tint, u_extra");
        assert_eq!(iv(p, GL_ACTIVE_ATTRIBUTES), 2);
        assert!(uniform(p, "u_extra") >= 0);
        assert_eq!(uniform(p, "u_tex"), -1);
        /* a shader that is not attached cannot be detached; one that is can */
        glDetachShader(p, fs);
        assert_eq!(glGetError(), GL_INVALID_OPERATION);
        glDetachShader(p, fs2);
        glDetachShader(p, vs);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(iv(p, GL_ATTACHED_SHADERS), 0);
        assert!(!link(p));
        assert_failed(p, "no shader at all");
    }
}

#[test]
fn a_relink_of_the_program_in_use() {
    let _g = context();
    let (vs, fs, fs2) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS), shader(GL_FRAGMENT_SHADER, FS2));
    let p = program(&[vs, fs]);
    assert!(link(p), "log: {}", log(p));
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(p);
        assert_eq!(glGetError(), GL_NO_ERROR);
        /* a successful relink of the program in use: no error, still current,
         * the new executable's interface */
        glDetachShader(p, fs);
        glAttachShader(p, fs2);
        assert!(link(p), "log: {}", log(p));
        assert_eq!(current_program(), p as GLint);
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert!(uniform(p, "u_extra") >= 0);
        assert_eq!(uniform(p, "u_tex"), -1);
        /* a failed relink: the program stays the current one (2.10.3: the
         * executable in use stays until a link succeeds), the link status
         * says FALSE and the log says why */
        glDetachShader(p, fs2);
        assert!(!link(p));
        assert_eq!(iv(p, GL_LINK_STATUS), GL_FALSE as GLint);
        assert!(!log(p).trim().is_empty());
        assert_eq!(current_program(), p as GLint, "the failed relink did not unuse the program");
        assert_eq!(glGetError(), GL_NO_ERROR);
        /* and a relink that succeeds again */
        glAttachShader(p, fs);
        assert!(link(p), "log: {}", log(p));
        assert_eq!(iv(p, GL_ACTIVE_UNIFORMS), 3);
        assert_eq!(current_program(), p as GLint);
        glUseProgram(0);
    }
}

#[test]
fn a_failed_link_gives_status_false_and_a_log() {
    let _g = context();
    let vs = shader(GL_VERTEX_SHADER, VS);
    let fs = shader(GL_FRAGMENT_SHADER, FS);
    /* no vertex shader */
    let p = program(&[fs]);
    assert!(!link(p));
    assert_failed(p, "no vertex shader");
    assert_eq!(iv(p, GL_ATTACHED_SHADERS), 1);
    /* no fragment shader */
    let p = program(&[vs]);
    assert!(!link(p));
    assert_failed(p, "no fragment shader");
    /* neither */
    let p = program(&[]);
    assert!(!link(p));
    assert_failed(p, "no shaders");
    assert_eq!((iv(p, GL_ACTIVE_ATTRIBUTES), iv(p, GL_ACTIVE_UNIFORMS)), (0, 0));
    /* a varying the fragment shader reads and the vertex shader never writes */
    let vs_no = shader(GL_VERTEX_SHADER, "attribute vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n");
    let fs_v = shader(
        GL_FRAGMENT_SHADER,
        "precision mediump float;\nvarying vec2 v_undefined;\nvoid main() { gl_FragColor = vec4(v_undefined, 0.0, 1.0); }\n",
    );
    let p = program(&[vs_no, fs_v]);
    assert!(!link(p));
    assert_failed(p, "an undefined varying");
    /* a varying of two types */
    let vs_t = shader(
        GL_VERTEX_SHADER,
        "attribute vec4 a_pos;\nvarying vec3 v_x;\nvoid main() { v_x = a_pos.xyz; gl_Position = a_pos; }\n",
    );
    let fs_t = shader(
        GL_FRAGMENT_SHADER,
        "precision mediump float;\nvarying vec2 v_x;\nvoid main() { gl_FragColor = vec4(v_x, 0.0, 1.0); }\n",
    );
    let p = program(&[vs_t, fs_t]);
    assert!(!link(p));
    assert_failed(p, "a varying of two types");
    /* the same uniform name with two types in the two stages */
    let vs_u = shader(GL_VERTEX_SHADER, "attribute vec4 a_pos;\nuniform vec4 u_x;\nvoid main() { gl_Position = a_pos + u_x; }\n");
    let fs_u = shader(
        GL_FRAGMENT_SHADER,
        "precision mediump float;\nuniform vec3 u_x;\nvoid main() { gl_FragColor = vec4(u_x, 1.0); }\n",
    );
    let p = program(&[vs_u, fs_u]);
    assert!(!link(p));
    assert_failed(p, "a uniform of two types");
    /* more attributes than GL_MAX_VERTEX_ATTRIBS */
    let mut max: GLint = 0;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(GL_MAX_VERTEX_ATTRIBS, &mut max) };
    let n = max as usize + 1;
    let mut text = String::new();
    for i in 0..n {
        text += &format!("attribute vec4 a{i};\n");
    }
    text += "void main() { gl_Position = a0";
    for i in 1..n {
        text += &format!(" + a{i}");
    }
    text += "; }\n";
    let vs_many = shader(GL_VERTEX_SHADER, &text);
    let fs_plain = shader(GL_FRAGMENT_SHADER, "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n");
    let p = program(&[vs_many, fs_plain]);
    assert!(!link(p));
    assert_failed(p, "too many attributes");
    /* a second glLinkProgram of the failed program says the same and is
     * quiet: no error, the status still FALSE */
    assert!(!link(p));
    assert_failed(p, "too many attributes, again");
    /* the good pair next to all of that still links */
    let p = program(&[vs, fs]);
    assert!(link(p), "log: {}", log(p));
}

#[test]
fn a_failed_link_resets_what_the_old_one_reported() {
    let _g = context();
    let (vs, fs) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    let p = program(&[vs, fs]);
    assert!(link(p));
    assert_eq!(iv(p, GL_INFO_LOG_LENGTH), 0, "a good link has an empty log");
    // SAFETY: plain GL calls.
    unsafe { glDetachShader(p, vs) };
    assert!(!link(p));
    assert!(iv(p, GL_INFO_LOG_LENGTH) > 1);
    // SAFETY: plain GL calls.
    unsafe { glAttachShader(p, vs) };
    assert!(link(p));
    assert_eq!(iv(p, GL_INFO_LOG_LENGTH), 0, "a good relink clears the log");
}

#[test]
fn bad_names_raise_errors_not_the_link() {
    let _g = context();
    // SAFETY: plain GL calls.
    unsafe {
        glLinkProgram(0);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        glLinkProgram(0x7fff_0000);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
        let s = shader(GL_VERTEX_SHADER, VS);
        glLinkProgram(s);
        assert_eq!(glGetError(), GL_INVALID_OPERATION, "a shader is not a program");
        glValidateProgram(0x7fff_0000);
        assert_eq!(glGetError(), GL_INVALID_VALUE);
    }
}

#[test]
fn tf_is_not_reachable() {
    let _g = context();
    let (vs, fs) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    let p = program(&[vs, fs]);
    assert!(link(p), "no transform feedback request is needed: {}", log(p));
    // SAFETY: plain GL calls.
    unsafe {
        let mut v: GLint = -1;
        for pname in [GL_TRANSFORM_FEEDBACK_VARYINGS, GL_TRANSFORM_FEEDBACK_BUFFER_MODE, GL_TRANSFORM_FEEDBACK_VARYING_MAX_LENGTH] {
            glGetProgramiv(p, pname, &mut v);
            assert_eq!(glGetError(), GL_INVALID_ENUM, "{pname:#x} is not an ES 2.0 program query");
        }
    }
}
    };
}

#[cfg(feature = "gpu-vivante")]
mod gc {
    suite!();
}

#[cfg(feature = "gpu-v3d")]
mod v3d {
    suite!();
}
