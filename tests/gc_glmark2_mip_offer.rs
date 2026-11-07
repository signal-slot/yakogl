// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GC7000 (i.MX8MP): every glGenerateMipmap call shape of glmark2's terrain,
//! refract and texture scenes (docs/notes/gc-mipmap-cpu-cases.md) reaches the
//! GPU chain of `src/vivante/mipmap.rs`, through the real GL core and the
//! real Vivante back end over a host stand-in OSAL (#1106, plan #328).
//!
//! Each call is made as glmark2 makes it: the texture created with
//! glTexImage2D (pixels NULL for a render target), a framebuffer with level
//! 0 as its colour, a draw into it, then glGenerateMipmap. After every call
//! the test asserts that the back end's chain was taken and the CPU box
//! filter was not (`gc_backend_mip_chains` counted one more made and no more
//! refused; the texture's `chain_in_backend`), the chain's level count, one
//! kick per level above 0, and no GL error. The chain's texels are
//! `gc_mipmap_gpu.rs`'s business, not this file's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-vivante,shaderc-online --test gc_glmark2_mip_offer
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-vivante", feature = "shaderc-online"))]

use core::ffi::{c_char, c_void};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::gl::consts::*;
use yakogl::gl::context::GlTexture;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::{glBindFramebuffer, glCheckFramebufferStatus, glFramebufferTexture2D, glGenFramebuffers};
use yakogl::gl::program::{
    glAttachShader, glBindAttribLocation, glCreateProgram, glGetProgramiv, glLinkProgram, glUseProgram,
};
use yakogl::gl::shader::{glCompileShader, glCreateShader, glGetShaderiv, glShaderSource};
use yakogl::gl::state::{glFinish, glGetError, glPixelStorei, glViewport};
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::vivante::backend::{gc_init, gc_shutdown, yakogl_backend_vivante};
use yakogl::vivante::kick::{GcKick, GC_KICK_MAGIC};

/* ---- the stand-in OSAL: host memory, kicks counted ---------------------------------- */

#[derive(Default)]
struct Platform {
    allocs: BTreeMap<u64, Box<[u64]>>,
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

/// The kick is counted and its fence raised; every kick completes at once.
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

/* ---- GL plumbing -------------------------------------------------------------------- */

/// Run `case` with the real Vivante back end up over the stand-in OSAL and a
/// fresh 64x64 context.
fn with_vivante<R>(case: impl FnOnce() -> R) -> R {
    // SAFETY: the caller holds SERIAL; the tables are statics.
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
    let faults = with_platform(|p| {
        /* leaked, not freed: a static of the library may still name the block */
        for (_, a) in core::mem::take(&mut p.allocs) {
            core::mem::forget(a);
        }
        p.kicks = 0;
        core::mem::take(&mut p.faults)
    });
    let r = r.unwrap_or_else(|e| std::panic::resume_unwind(e));
    assert!(faults.is_empty(), "the back end broke the OSAL contract: {faults:?}");
    r
}

fn no_error(what: &str) {
    // SAFETY: a plain GL call.
    let e = unsafe { glGetError() };
    assert_eq!(e, GL_NO_ERROR, "{what}: 0x{e:x}");
}

/// (made, refused) since the back end started.
fn mip_counts() -> (u32, u32) {
    extern "C" {
        fn gc_backend_mip_chains(made: *mut u32, refused: *mut u32);
    }
    let (mut made, mut refused) = (0u32, 0u32);
    // SAFETY: two valid u32 out-pointers.
    unsafe { gc_backend_mip_chains(&mut made, &mut refused) };
    (made, refused)
}

fn texture(w: u32, h: u32) -> &'static GlTexture {
    let c = yakogl::gl::context::current().expect("a current context");
    c.objs_ref()
        .textures
        .iter()
        .find(|t| t.used && t.width == w && t.height == h && t.target == GL_TEXTURE_2D)
        .expect("the texture")
}

/// Levels of a full chain of a `w` x `h` texture.
fn full_chain(w: u32, h: u32) -> u32 {
    w.max(h).ilog2() + 1
}

/// glTexImage2D of an RGBA8 `w` x `h` texture, bound, under glmark2's mipmap
/// filter; `pixels` NULL is a render target (glmark2's `Texture::create`
/// path for the targets), otherwise an image upload (`Texture::load`).
fn create(w: u32, h: u32, pixels: Option<&[u8]>) -> GLuint {
    let mut tex: GLuint = 0;
    // SAFETY: plain GL calls; `pixels` holds the w x h texels when given.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            w as GLsizei,
            h as GLsizei,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            pixels.map_or(core::ptr::null(), |p| p.as_ptr() as *const c_void),
        );
    }
    no_error("glTexImage2D");
    tex
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

const RENDER_VS: &str = "attribute vec4 p;\nvoid main() { gl_Position = p; }\n";
const RENDER_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0); }\n";
static RENDER_QUAD: [f32; 16] = [-1.0, -1.0, 0.5, 1.0, 1.0, -1.0, 0.5, 1.0, -1.0, 1.0, 0.5, 1.0, 1.0, 1.0, 0.5, 1.0];

/// A framebuffer with level 0 of `tex` as its colour, bound (glmark2's
/// `Texture`-backed `RenderTarget`); a program that draws one flat quad in use.
fn render_target(tex: GLuint) {
    // SAFETY: plain GL calls; RENDER_QUAD is a static.
    unsafe {
        let mut fb: GLuint = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "the texture's framebuffer");
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, RENDER_VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, RENDER_FS));
        glBindAttribLocation(p, 0, c"p".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        glUseProgram(p);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, RENDER_QUAD.as_ptr() as *const c_void);
    }
    no_error("the render target");
}

fn render_into(w: u32, h: u32) {
    // SAFETY: plain GL calls.
    unsafe {
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        glFinish();
    }
    no_error("the draw into the texture");
}

/// One glGenerateMipmap on the texture bound, `what` named in a failure: the
/// back end made the chain (counted, one kick per level above 0, the texture
/// in the back end's hands, GL's whole chain) and nothing was refused.
fn generate_on_gpu(what: &str, w: u32, h: u32) {
    let levels = full_chain(w, h);
    let ((made0, refused0), kicks0) = (mip_counts(), with_platform(|p| p.kicks));
    // SAFETY: a plain GL call.
    unsafe { glGenerateMipmap(GL_TEXTURE_2D) };
    no_error(what);
    let t = texture(w, h);
    assert!(t.chain_in_backend, "{what}: the GL core left the chain to the CPU filter");
    assert_eq!(u32::from(t.chain_levels), levels, "{what}: the chain's level count");
    assert_eq!(mip_counts(), (made0 + 1, refused0), "{what}: the back end counts one chain made, none refused");
    assert_eq!(with_platform(|p| p.kicks) - kicks0, levels as usize - 1, "{what}: one kick per level above 0");
}

/* ---- the call shapes ----------------------------------------------------------------- */

/// A render target glmark2 creates at setup (`update_mipmap` at load, the
/// texture still empty) and then draws into and filters every frame
/// (`calls_a_frame` calls, 3 frames).
fn target_case(name: &str, w: u32, h: u32, calls_a_frame: u32) {
    let tex = create(w, h, None);
    render_target(tex);
    // SAFETY: a plain GL call; the target is the texture bound at creation.
    unsafe { glBindTexture(GL_TEXTURE_2D, tex) };
    generate_on_gpu(&format!("{name} {w}x{h}, setup"), w, h);
    assert!(!texture(w, h).pe_order, "{name}: nothing rendered into it at setup");
    for frame in 1..=3 {
        render_into(w, h);
        assert!(texture(w, h).pe_order, "{name} frame {frame}: rendered into, B,G,R,A");
        for call in 1..=calls_a_frame {
            generate_on_gpu(&format!("{name} {w}x{h}, frame {frame} call {call}"), w, h);
        }
    }
}

/// `scene-terrain/base-renderer.cpp:89`: five calls a frame, the terrain
/// target twice (its own `update_mipmap` and the overlay's), bloom_h,
/// bloom_v (256x256) and tilt_h (800x600), all RGBA8.
#[test]
fn terrains_five_calls_a_frame_reach_the_gpu_chain() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    /* one context each: the textures of one size are told apart by size */
    /* 800x600: ten levels, the NPOT chain grown at the first call */
    with_vivante(|| target_case("terrain", 800, 600, 2));
    with_vivante(|| target_case("tilt_h", 800, 600, 1));
    /* 256x256: nine levels */
    with_vivante(|| target_case("bloom_h", 256, 256, 1));
    with_vivante(|| target_case("bloom_v", 256, 256, 1));
}

/// `scene-refract.cpp:430`: `tex_[COLOR]`, 1600x1200 (eleven levels), once a frame.
#[test]
fn refracts_colour_target_reaches_the_gpu_chain() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    with_vivante(|| target_case("refract", 1600, 1200, 1));
}

/// `texture.cpp:95`: an image upload filtered once at load (the crate
/// texture of `texture-filter=mipmap`, terrain's 512x512 images), and the
/// terrain's specular map, a 512x512 target filtered at setup and once more
/// after its single render.
#[test]
fn texture_uploads_and_the_specular_map_reach_the_gpu_chain() {
    let _g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let img = vec![0x7fu8; 512 * 512 * 4];
    for i in 0..3 {
        with_vivante(|| {
            create(512, 512, Some(&img));
            generate_on_gpu(&format!("upload {i} 512x512"), 512, 512);
            assert!(!texture(512, 512).pe_order, "an upload is not rendered into");
        });
    }
    with_vivante(|| {
        let tex = create(512, 512, None);
        render_target(tex);
        // SAFETY: a plain GL call.
        unsafe { glBindTexture(GL_TEXTURE_2D, tex) };
        generate_on_gpu("specular map 512x512, setup", 512, 512);
        render_into(512, 512);
        generate_on_gpu("specular map 512x512, after its render", 512, 512);
    });
}
