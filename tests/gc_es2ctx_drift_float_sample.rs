// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The ES 2.0 context against 8f019dae (vivante: the GC7000UL samples
//! RGBA32F/UI/I as two G32R32 planes; the classes join GC_SAMPLED_TEXFMTS).
//! The i.MX8MP whole tree (0fbe61a3) predates it, and its stage-1 promise is
//! 0 Fail.
//!
//! The ES 2.0 context does NOT reach the two-plane path: `report-es2` does
//! not list GL_OES_texture_float (nor _linear), and `es3_only_pixel_type`
//! refuses every GL_FLOAT upload with GL_INVALID_ENUM, so no 32-bit float
//! texture is stored from ES 2.0 (the only float texture is the half-float
//! one of GL_OES_texture_half_float, in the 16-bit classes the join does not
//! touch). This file asserts that, on the real GL core with the Vivante back
//! end over a stand-in OSAL that keeps every allocation in host memory:
//!
//! * the extension string lists GL_OES_texture_half_float and neither
//!   GL_OES_texture_float nor GL_OES_texture_float_linear (nor other 32-bit
//!   float names);
//! * glTexImage2D of RGBA, RGB, LUMINANCE_ALPHA, LUMINANCE and ALPHA with
//!   GL_FLOAT, and of RGBA and RGB with ES 3.0's GL_HALF_FLOAT, answers
//!   GL_INVALID_ENUM, as before 8f019dae, and defines no level;
//! * the sized 32-bit internal formats (RGBA32F, RGB32F, RG32F, RGBA32UI,
//!   RGBA32I) are no ES 2.0 formats: an error, never a store;
//! * the same five formats with GL_HALF_FLOAT_OES are taken (GL_NO_ERROR) and
//!   stored in a class of at most 8 bytes a texel, not RGBA32F;
//! * a GLSL ES 1.00 shader with `#extension GL_OES_texture_float : require`
//!   does not compile.
//!
//! What it cannot see: the board. dEQP-GLES2 reaches float textures only
//! through fbo.completeness.renderable.*.{red,rg,rgb,rgba}_float (and the
//! _half_float_oes siblings): the _float cases expect an incomplete
//! framebuffer, which the refused upload gives. No mustpass case samples one.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online,report-es2 --test gc_es2ctx_drift_float_sample
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online", feature = "report-es2"))]

use core::ffi::c_void;
use std::collections::BTreeMap;
use std::sync::Mutex;
use yakogl::backend::{gles_texfmt_bytes, GLES_TEXFMT_RGBA32F};
use yakogl::gl::consts::*;
use yakogl::gl::context::GlTexture;
use yakogl::gl::shader::*;
use yakogl::gl::state::{glGetError, glGetString};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_refused_draws, gc_shutdown, yakogl_backend_vivante};

/* ---- the stand-in OSAL: every allocation in host memory ---------------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
    bump: BTreeMap<u64, u64>,
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
        if !m.impl_.is_null() {
            p.allocs.remove(&m.gpu);
        }
    });
    // SAFETY: as above; a freed descriptor is cleared (the OSAL contract).
    unsafe { *mem = OsalGpuMem::ZERO };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn p_submit(_kick: *const c_void, _len: usize) -> i64 {
    1
}
unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn p_completed() -> i64 {
    i64::MAX
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

fn with_backend<R>(case: impl FnOnce() -> R) -> R {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: SERIAL is held; the tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &yakogl_backend_vivante;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert_eq!(gc_init(), 0, "gc_init");
        let c = yakogl::gl::context::gl_context_create(64, 64);
        assert!(!c.is_null(), "context pool exhausted");
        c
    };
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(case));
    // SAFETY: the context made above.
    unsafe {
        yakogl::gl::context::gl_context_destroy(ctx);
        gc_shutdown();
    }
    /* leaked, not freed: a static of the library may still name the block */
    with_platform(|p| {
        for (_, m) in core::mem::take(&mut p.allocs) {
            core::mem::forget(m);
        }
    });
    r.unwrap_or_else(|e| std::panic::resume_unwind(e))
}

/* ---- GL plumbing ------------------------------------------------------------------- */

fn err() -> GLenum {
    // SAFETY: a plain GL call.
    unsafe { glGetError() }
}

fn extensions() -> Vec<String> {
    // SAFETY: glGetString returns a NUL-terminated static string.
    let s = unsafe { std::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const _) };
    s.to_str().expect("utf-8").split_whitespace().map(str::to_owned).collect()
}

fn texture_2d() -> &'static GlTexture {
    let c = yakogl::gl::context::current().expect("a current context");
    c.objs_ref().textures.iter().find(|t| t.used && t.target == GL_TEXTURE_2D).expect("the texture")
}

fn bind_new() {
    let mut tex: GLuint = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

fn upload(internal: GLenum, format: GLenum, type_: GLenum, bpt: usize) -> GLenum {
    let data = vec![0x3Cu8; 4 * 4 * bpt];
    // SAFETY: the buffer holds 4x4 texels of `bpt` bytes.
    unsafe {
        glTexImage2D(GL_TEXTURE_2D, 0, internal as GLint, 4, 4, 0, format, type_, data.as_ptr() as *const c_void)
    };
    err()
}

/// (name, format, components)
const FORMATS: [(&str, GLenum, usize); 5] = [
    ("RGBA", GL_RGBA, 4),
    ("RGB", GL_RGB, 3),
    ("LUMINANCE_ALPHA", GL_LUMINANCE_ALPHA, 2),
    ("LUMINANCE", GL_LUMINANCE, 1),
    ("ALPHA", GL_ALPHA, 1),
];

#[test]
fn the_extension_string_claims_half_float_and_not_float() {
    with_backend(|| {
        let list = extensions();
        let has = |n: &str| list.iter().any(|e| e == n);
        assert!(has("GL_OES_texture_half_float"), "{list:?}");
        for n in [
            "GL_OES_texture_float",
            "GL_OES_texture_float_linear",
            "GL_EXT_color_buffer_float",
            "GL_EXT_float_blend",
            "GL_OES_texture_32bit",
        ] {
            assert!(!has(n), "{n} must stay unclaimed: {list:?}");
        }
    });
}

#[test]
fn gl_float_uploads_are_refused_as_before() {
    with_backend(|| {
        bind_new();
        let refused0 = gc_refused_draws();
        for (name, format, comps) in FORMATS {
            assert_eq!(upload(format, format, GL_FLOAT, comps * 4), GL_INVALID_ENUM, "{name} with GL_FLOAT");
            assert_eq!(texture_2d().level_defined, 0, "{name}: a refused upload defines no level");
        }
        for (name, format, comps) in [FORMATS[0], FORMATS[1]] {
            assert_eq!(
                upload(format, format, GL_HALF_FLOAT, comps * 2),
                GL_INVALID_ENUM,
                "{name} with ES 3.0's GL_HALF_FLOAT"
            );
            assert_eq!(texture_2d().level_defined, 0, "{name}: a refused upload defines no level");
        }
        assert_eq!(gc_refused_draws(), refused0);
    });
}

#[test]
fn sized_32bit_internal_formats_are_not_es2_formats() {
    with_backend(|| {
        bind_new();
        for (name, internal) in [
            ("RGBA32F", GL_RGBA32F),
            ("RGB32F", GL_RGB32F),
            ("RG32F", GL_RG32F),
            ("RGBA32UI", GL_RGBA32UI),
            ("RGBA32I", GL_RGBA32I),
        ] {
            assert_ne!(upload(internal, GL_RGBA, GL_FLOAT, 16), GL_NO_ERROR, "{name} with FLOAT");
            assert_ne!(upload(internal, GL_RGBA, GL_UNSIGNED_BYTE, 4), GL_NO_ERROR, "{name} with UNSIGNED_BYTE");
            assert_eq!(texture_2d().level_defined, 0, "{name}: no level defined");
        }
    });
}

#[test]
fn half_float_oes_uploads_are_taken_in_16_bit_classes() {
    with_backend(|| {
        bind_new();
        for (name, format, comps) in FORMATS {
            assert_eq!(upload(format, format, 0x8D61, comps * 2), GL_NO_ERROR, "{name} with HALF_FLOAT_OES");
            let t = texture_2d();
            let bytes = gles_texfmt_bytes(t.texfmt) as usize;
            assert!(bytes <= 8, "{name}: stored in {bytes} bytes a texel, not a 128-bit class");
            assert_ne!(t.texfmt, GLES_TEXFMT_RGBA32F, "{name}: not RGBA32F");
        }
    });
}

#[test]
fn a_shader_requiring_texture_float_does_not_compile() {
    with_backend(|| {
        let src = b"#extension GL_OES_texture_float : require\nprecision mediump float;\nuniform sampler2D t;\nvarying vec2 uv;\nvoid main() { gl_FragColor = texture2D(t, uv); }\n\0";
        // SAFETY: plain GL calls; the source is NUL-terminated.
        unsafe {
            let s = glCreateShader(GL_FRAGMENT_SHADER);
            let p = src.as_ptr() as *const GLchar;
            glShaderSource(s, 1, &p, core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = -1;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, GL_FALSE as GLint, "an unadvertised extension is a compile error");
        }
    });
}
