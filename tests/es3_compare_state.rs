// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth-compare state the GL core hands a back end with each texture
//! bind (`GlesTextureBind::compare_op`, src/gl/draw.rs `compare_op`), through
//! the GL API on the host. ES 3.0.6 3.8.2 (sampler objects override the
//! texture's sampler state), 3.8.15.1 (TEXTURE_COMPARE_MODE, _FUNC, the
//! fixed-point clamp) and table 6.10 (the defaults: NONE, LEQUAL).
//!
//! `compare_op` is the function less GL_NEVER (0..=7) under
//! GL_COMPARE_REF_TO_TEXTURE, 16 with the compare off, plus 8 for a
//! fixed-point depth texture (DEPTH_COMPONENT16/24, DEPTH24_STENCIL8). What a
//! back end does with it (V3D's sampler record) is its own test, e.g.
//! tests/es3_shadow_v3d.rs.
//!
//! - `default_state_is_compare_off_with_lequal`: no glTexParameter at all.
//! - `compare_mode_none_gives_16_or_24`: mode NONE, whatever the function.
//! - `each_compare_function_reaches_the_bind`: the 8 functions, 0..=7, and
//!   bit 3 for DEPTH_COMPONENT16/24 and DEPTH24_STENCIL8 but not for
//!   DEPTH_COMPONENT32F.
//! - `a_bound_sampler_object_overrides_the_textures_compare`: the sampler's
//!   mode and function replace the texture's, and unbinding restores them.
//!
//! The stand-in back end draws nothing; it keeps the first texture bind of
//! the last draw.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_compare_state
//! ```
//!
//! Needs the in-process compiler (the quad's program is not in the offline
//! table) and the BXS target: the GC7000L code generator refuses an ES 3.00
//! shadow sampler ("a ... texture look-up ... is not selected yet"), so the
//! quad only compiles for `gpu-powervr`; the state under test is the GL
//! core's, the same through either build. An ES 2.0-reporting build has no
//! ES 3.0 depth compare to test.

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{GlesDraw, GlesHeap, YakoglBackend};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::{glBindSampler, glGenSamplers, glSamplerParameteri};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- the stand-in back end: it records the compare op of each draw --------- */

/// The draws seen, and `compare_op` of the first texture bind of the last.
static SEEN: Mutex<(u32, u8)> = Mutex::new((0, 0));

unsafe extern "C" fn b_init() -> i32 {
    0
}
unsafe extern "C" fn b_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn b_ok() -> i32 {
    0
}
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core passes a live job.
    let d = unsafe { &*d };
    let mut s = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    assert!(d.num_textures >= 1, "the program samples one texture");
    *s = (s.0 + 1, d.textures[0].compare_op);
    0
}

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-compare-state".as_ptr() as *const c_char,
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

unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    // SAFETY: `out` is the core's.
    unsafe { *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu } };
    0
}
unsafe extern "C" fn o_gpu_free(_m: *mut OsalGpuMem) -> i32 {
    0
}
unsafe extern "C" fn o_cache(_cpu: *mut c_void, _n: usize) {}

static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(o_gpu_alloc),
    gpu_free: Some(o_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(o_cache),
    cache_invd: Some(o_cache),
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

/* ---- a fresh context per case ----------------------------------------------- */

static SERIAL: Mutex<()> = Mutex::new(());

fn on_fresh_context(case: impl Fn()) {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &BACKEND;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    *SEEN.lock().unwrap_or_else(|e| e.into_inner()) = (0, 0);
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe { yakogl::gl::context::gl_context_destroy(ctx) };
    if let Err(e) = r {
        std::panic::resume_unwind(e);
    }
}

/* ---- GL plumbing ------------------------------------------------------------- */

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
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

const VS: &str = "#version 300 es\nin vec4 p;\nout vec2 v;\nvoid main() { v = p.xy * 0.5 + 0.5; gl_Position = p; }\n";
const FS: &str = concat!(
    "#version 300 es\nprecision mediump float;\nuniform mediump sampler2DShadow s;\n",
    "in vec2 v;\nout vec4 c;\n",
    "void main() { c = vec4(texture(s, vec3(v, 0.5))); }\n"
);

static QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// One quad sampling texture unit 0; returns the compare op the back end got.
fn draw_op() -> u8 {
    let before = SEEN.lock().unwrap_or_else(|e| e.into_inner()).0;
    // SAFETY: plain GL calls; QUAD is a static.
    unsafe {
        glViewport(0, 0, 64, 64);
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glUniform1i(glGetUniformLocation(p, c"s".as_ptr()), 0);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
    }
    no_error("draw");
    let s = *SEEN.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(s.0, before + 1, "the quad was drawn once");
    s.1
}

/// Make and bind a 4x4 depth texture of `ifmt`, NEAREST so it is complete.
fn depth_texture(ifmt: GLenum) {
    let (fmt, ty) = match ifmt {
        GL_DEPTH_COMPONENT16 => (GL_DEPTH_COMPONENT, GL_UNSIGNED_SHORT),
        GL_DEPTH_COMPONENT24 => (GL_DEPTH_COMPONENT, GL_UNSIGNED_INT),
        GL_DEPTH_COMPONENT32F => (GL_DEPTH_COMPONENT, GL_FLOAT),
        GL_DEPTH24_STENCIL8 => (GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8),
        _ => unreachable!(),
    };
    let px = [0u8; 4 * 4 * 8];
    let mut t: GLuint = 0;
    // SAFETY: `px` covers a 4x4 image of up to 8 bytes a texel; one name.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(GL_TEXTURE_2D, t);
        glTexImage2D(GL_TEXTURE_2D, 0, ifmt as GLint, 4, 4, 0, fmt, ty, px.as_ptr() as *const c_void);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    no_error("depth texture");
}

fn tex_param(pname: GLenum, v: GLenum) {
    // SAFETY: a plain GL call.
    unsafe { glTexParameteri(GL_TEXTURE_2D, pname, v as GLint) };
    no_error("glTexParameteri");
}

fn tex_param_get(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one GLint out.
    unsafe { glGetTexParameteriv(GL_TEXTURE_2D, pname, &mut v) };
    v
}

const FORMATS: [(GLenum, bool); 4] = [
    (GL_DEPTH_COMPONENT16, true),
    (GL_DEPTH_COMPONENT24, true),
    (GL_DEPTH24_STENCIL8, true),
    (GL_DEPTH_COMPONENT32F, false),
];

const FUNCS: [GLenum; 8] = [GL_NEVER, GL_LESS, GL_EQUAL, GL_LEQUAL, GL_GREATER, GL_NOTEQUAL, GL_GEQUAL, GL_ALWAYS];

/* ---- the tests ---------------------------------------------------------------- */

/// ES 3.0.6 table 6.10: TEXTURE_COMPARE_MODE starts NONE, TEXTURE_COMPARE_FUNC
/// LEQUAL. With no glTexParameter call the bind has the compare off; LEQUAL
/// is only stored (switching the mode on later reads 3).
#[test]
fn default_state_is_compare_off_with_lequal() {
    on_fresh_context(|| {
        for (ifmt, fixed) in FORMATS {
            depth_texture(ifmt);
            assert_eq!(tex_param_get(GL_TEXTURE_COMPARE_MODE), GL_NONE as GLint, "0x{ifmt:x} default mode");
            assert_eq!(tex_param_get(GL_TEXTURE_COMPARE_FUNC), GL_LEQUAL as GLint, "0x{ifmt:x} default function");
            assert_eq!(draw_op(), 16 | if fixed { 8 } else { 0 }, "0x{ifmt:x} default: no compare");
            tex_param(GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE);
            assert_eq!(draw_op(), 3 | if fixed { 8 } else { 0 }, "0x{ifmt:x} LEQUAL stored");
        }
    });
}

/// Mode NONE gives 16 (24 for a fixed-point depth), whatever the function.
#[test]
fn compare_mode_none_gives_16_or_24() {
    on_fresh_context(|| {
        for (ifmt, fixed) in FORMATS {
            depth_texture(ifmt);
            for f in FUNCS {
                tex_param(GL_TEXTURE_COMPARE_FUNC, f);
                tex_param(GL_TEXTURE_COMPARE_MODE, GL_NONE);
                assert_eq!(draw_op(), if fixed { 24 } else { 16 }, "0x{ifmt:x} func 0x{f:x}, mode NONE");
            }
        }
    });
}

/// Each of the 8 functions under COMPARE_REF_TO_TEXTURE, per depth format.
#[test]
fn each_compare_function_reaches_the_bind() {
    on_fresh_context(|| {
        for (ifmt, fixed) in FORMATS {
            depth_texture(ifmt);
            tex_param(GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE);
            for (i, f) in FUNCS.into_iter().enumerate() {
                tex_param(GL_TEXTURE_COMPARE_FUNC, f);
                let want = i as u8 | if fixed { 8 } else { 0 };
                assert_eq!(draw_op(), want, "0x{ifmt:x} func 0x{f:x}");
            }
        }
    });
}

/// ES 3.0.6 3.8.2: a sampler object bound to the unit replaces the texture's
/// compare mode and function; unbinding it puts the texture's back.
#[test]
fn a_bound_sampler_object_overrides_the_textures_compare() {
    on_fresh_context(|| {
        depth_texture(GL_DEPTH_COMPONENT16);
        let mut s: GLuint = 0;
        // SAFETY: plain GL calls; `s` is one name.
        unsafe {
            glGenSamplers(1, &mut s);
            glSamplerParameteri(s, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glSamplerParameteri(s, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        }
        // the sampler's defaults are NONE and LEQUAL: the texture compares, the sampler does not
        tex_param(GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE);
        tex_param(GL_TEXTURE_COMPARE_FUNC, GL_GREATER);
        assert_eq!(draw_op(), 4 | 8, "the texture's GREATER");
        // SAFETY: plain GL call.
        unsafe { glBindSampler(0, s) };
        no_error("glBindSampler");
        assert_eq!(draw_op(), 16 | 8, "the sampler's default mode NONE hides the texture's compare");
        for (i, f) in FUNCS.into_iter().enumerate() {
            // SAFETY: plain GL calls.
            unsafe {
                glSamplerParameteri(s, GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE as GLint);
                glSamplerParameteri(s, GL_TEXTURE_COMPARE_FUNC, f as GLint);
            }
            no_error("glSamplerParameteri");
            assert_eq!(draw_op(), i as u8 | 8, "the sampler's func 0x{f:x}, not the texture's GREATER");
        }
        // the other way: a texture with the compare off, a sampler with it on
        tex_param(GL_TEXTURE_COMPARE_MODE, GL_NONE);
        // SAFETY: plain GL call.
        unsafe { glSamplerParameteri(s, GL_TEXTURE_COMPARE_FUNC, GL_NOTEQUAL as GLint) };
        assert_eq!(draw_op(), 5 | 8, "the sampler compares though the texture does not");
        // SAFETY: plain GL call.
        unsafe { glBindSampler(0, 0) };
        no_error("unbind");
        assert_eq!(draw_op(), 16 | 8, "unbound: the texture's own NONE");
        tex_param(GL_TEXTURE_COMPARE_MODE, GL_COMPARE_REF_TO_TEXTURE);
        assert_eq!(draw_op(), 4 | 8, "unbound: the texture's own GREATER");
    });
}
