// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A cube-map face rendered into on the PowerVR, then sampled by a program
//! that reads the TPU's own cube (`gl::native_cube`, `pvr_native_cube`).
//!
//! dEQP-GLES3 (texture.specification, fbo, texture) renders into a face
//! through a framebuffer, binds another framebuffer and samples the cube
//! with no glFlush in between. The render is then still in the frame being
//! recorded when the sampling draw is built, and src/gl/draw.rs kept the
//! stacked image for it (`open`): the twiddled copy was not made, so the
//! native-cube program read the stacked storage as a twiddled cube. Now
//! the frame is submitted and waited for there and the copy is made from
//! the image that was drawn; only a cube attached to the framebuffer the
//! draw renders into (a feedback loop, undefined in GL) keeps the stacked
//! image.
//!
//! A stand-in back end models the frame: a draw is recorded into the open
//! frame (`kick_id`), and `flush` submits it, which "renders" by filling
//! each target the frame's draws named with a marker colour. The tests read
//! the copy the sampling draw was handed through `native_cube`'s placement.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_cube_rt
//! ```
#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::Mutex;
use yakogl::backend::{
    gles_texfmt_bytes, GlesDraw, GlesHeap, YakoglBackend, GLES_TEXTYPE_CUBE, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE,
};
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::native_cube;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ------------------------------------------------------- the stand-in -- */

/// The bytes a submitted frame writes over every pixel of the targets its
/// draws named (RGBA8).
const MARKER: [u8; 4] = [0x11, 0x22, 0x33, 0x44];

/// The stand-in's frames: the open one's id, the draws it holds, and what
/// the GL core asked of it.
struct Frames {
    open: u32,
    /// The last kick submitted and the last one waited for.
    submitted: u32,
    finished: u32,
    /// (target address, width, height) of each draw in the open frame.
    targets: Vec<(u64, u32, u32)>,
    flushes: u32,
    /// Binding 0 of every draw that drew something.
    seen: Vec<Seen>,
}

/// What a test reads of a binding (`GlesTextureBind` holds a CPU pointer,
/// which a static cannot).
#[derive(Clone, Copy)]
struct Seen {
    type_: u8,
    format: u8,
    base_gpu: u64,
    bytes: u32,
}

static FRAMES: Mutex<Frames> =
    Mutex::new(Frames { open: 1, submitted: 0, finished: 0, targets: Vec::new(), flushes: 0, seen: Vec::new() });

fn frames() -> std::sync::MutexGuard<'static, Frames> {
    FRAMES.lock().unwrap_or_else(|e| e.into_inner())
}

unsafe extern "C" fn s_init() -> i32 {
    0
}
unsafe extern "C" fn s_heap_base(_h: GlesHeap) -> u64 {
    0x1000_0000
}
unsafe extern "C" fn s_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let mut f = frames();
    if d.target_gpu != 0 {
        f.targets.push((d.target_gpu, d.target_w, d.target_h));
    }
    if d.index_count != 0 {
        let t = &d.textures[0];
        f.seen.push(Seen { type_: t.type_, format: t.format, base_gpu: t.base_gpu, bytes: t.bytes });
    }
    0
}
unsafe extern "C" fn s_flush() -> i32 {
    let mut f = frames();
    f.flushes += 1;
    if f.targets.is_empty() {
        return 0;
    }
    /* the frame renders: every target it named is the marker */
    for (gpu, w, h) in core::mem::take(&mut f.targets) {
        // SAFETY: the target is GL-core storage the OSAL below allocated,
        // `w * h` RGBA8 pixels at a 4-texel-multiple pitch (side 4).
        let px = core::slice::from_raw_parts_mut(gpu as *mut u8, (w * h * 4) as usize);
        for p in px.as_chunks_mut::<4>().0 {
            *p = MARKER;
        }
    }
    f.submitted = f.open;
    f.open += 1;
    0
}
unsafe extern "C" fn s_finish() -> i32 {
    let mut f = frames();
    f.finished = f.submitted;
    0
}
unsafe extern "C" fn s_kick_id() -> u32 {
    frames().open
}
unsafe extern "C" fn s_kick_pending(id: u32) -> bool {
    let f = frames();
    id > f.finished && id <= f.submitted
}

/// The PowerVR's flags (src/pvr/backend.rs `yakogl_backend_pvr`): one 2D
/// image per binding, the GL core makes the texel copies.
static STAND_IN: YakoglBackend = YakoglBackend {
    name: c"pvr-cube-rt stand-in".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(s_init),
    shutdown: None,
    draw: Some(s_draw),
    flush: Some(s_flush),
    finish: Some(s_finish),
    finish_previous: None,
    kick_id: Some(s_kick_id),
    kick_pending: Some(s_kick_pending),
    heap_base: Some(s_heap_base),
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

/* ---------------------------------------------------------------- OSAL -- */

/// Pointer-valued device addresses: a test reads texels through `base_gpu`.
unsafe extern "C" fn o_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice());
    let cpu = mem.as_mut_ptr() as *mut c_void;
    *out = OsalGpuMem { cpu, gpu: cpu as u64, phys: cpu as u64, size, impl_: cpu };
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

/// One test at a time: the back end, the OSAL and the GL core are statics.
static SERIAL: Mutex<()> = Mutex::new(());

/// The context `start` made; a failing assertion still frees its slot.
struct Ctx(*mut yakogl::gl::context::GlContext);

impl Drop for Ctx {
    fn drop(&mut self) {
        // SAFETY: the context `start` made.
        unsafe { yakogl::gl::context::gl_context_destroy(self.0) };
    }
}

/// A fresh 16x16 context on the stand-in, the native cube at its default
/// (on); the guard serialises the tests.
fn start() -> (std::sync::MutexGuard<'static, ()>, Ctx) {
    let g = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: the guard serialises the tests; both tables are statics.
    let ctx = unsafe {
        osal_set_ops(&OSAL);
        yakogl::backend::g_backend = &STAND_IN;
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        yakogl::shaderc::pvr_set_native_cube(yakogl::shaderc::PVR_NATIVE_CUBE_DEFAULT);
        let ctx = yakogl::gl::context::gl_context_create(16, 16);
        assert!(!ctx.is_null(), "context pool exhausted");
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        while glGetError() != GL_NO_ERROR {}
        ctx
    };
    {
        let mut f = frames();
        /* a fresh frame: a render mark an earlier test left on a texture
         * slot this context reuses names a kick already done */
        let open = f.open + 1;
        *f =
            Frames { open, submitted: open - 1, finished: open - 1, targets: Vec::new(), flushes: 0, seen: Vec::new() };
    }
    (g, Ctx(ctx))
}

/* ------------------------------------------------------------ the GL -- */

const VS: &str = "#version 300 es\nin vec4 a_pos;\nvoid main() { gl_Position = a_pos; }\n";
const FS_SOLID: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\n\
                        void main() { o = vec4(0.25, 0.5, 0.75, 1.0); }\n";
const FS_CUBE: &str = "#version 300 es\nprecision mediump float;\nuniform samplerCube s;\nout vec4 o;\n\
                       void main() { o = texture(s, vec3(0.0, 1.0, 0.0)); }\n";

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

fn program(fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glBindAttribLocation(p, 0, c"a_pos".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed");
        p
    }
}

/// Side of the cube: one level, RGBA8.
const SIDE: u32 = 4;

/// Face `f`'s uploaded texel colour.
fn face_rgba(f: u32) -> [u8; 4] {
    [0x80 + f as u8, 0x10 * f as u8, 0xF0 - f as u8, 0xFF]
}

/// A 4x4 RGBA8 cube of one level on unit 0, each face its own colour.
fn cube() -> GLuint {
    let mut t = 0;
    // SAFETY: plain GL calls; each face's data outlives its glTexImage2D.
    unsafe {
        glGenTextures(1, &mut t);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_CUBE_MAP, t);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_CUBE_MAP, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        for f in 0..6 {
            let data: Vec<u8> = (0..SIDE * SIDE).flat_map(|_| face_rgba(f)).collect();
            glTexImage2D(
                GL_TEXTURE_CUBE_MAP_POSITIVE_X + f,
                0,
                GL_RGBA8 as GLint,
                SIDE as GLsizei,
                SIDE as GLsizei,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                data.as_ptr() as *const c_void,
            );
        }
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    t
}

/// A framebuffer with `face` of `cube` as colour attachment 0, bound.
fn face_fbo(cube: GLuint, face: GLenum) -> GLuint {
    let mut fbo = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, face, cube, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fbo
}

/// A full-viewport quad of `prog`, from a client array.
fn quad(prog: GLuint, w: u32, h: u32) {
    static POS: [f32; 8] = [-1.0, -1.0, 1.0, -1.0, -1.0, 1.0, 1.0, 1.0];
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        glUseProgram(prog);
        glViewport(0, 0, w as GLsizei, h as GLsizei);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(0, 2, GL_FLOAT, GL_FALSE as GLboolean, 0, POS.as_ptr() as *const c_void);
        glDrawArrays(GL_TRIANGLE_STRIP, 0, 4);
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
}

/// The last drawing draw's binding 0.
fn last_bind() -> Seen {
    *frames().seen.last().expect("a draw that drew")
}

/// Texel (0, 0) of `face` of the one-level native copy `b` points at.
fn copy_texel(b: &Seen, face: u32) -> [u8; 4] {
    let bpp = gles_texfmt_bytes(b.format);
    let at = face * native_cube::face_bytes(SIDE, bpp, true);
    // SAFETY: the copy is `b.bytes` of host memory the OSAL above allocated.
    let mem = unsafe { core::slice::from_raw_parts(b.base_gpu as *const u8, b.bytes as usize) };
    mem[at as usize..at as usize + 4].try_into().unwrap()
}

#[test]
fn a_face_rendered_in_the_open_frame_is_sampled_from_the_drawn_image() {
    let (_g, _ctx) = start();
    let solid = program(FS_SOLID);
    let sample = program(FS_CUBE);
    let tex = cube();
    /* render into +Y (face 2), the face the sampling direction hits */
    face_fbo(tex, GL_TEXTURE_CUBE_MAP_POSITIVE_Y);
    quad(solid, SIDE, SIDE);
    let kick = frames().open;
    assert!(!frames().targets.is_empty(), "the render is in the open frame");
    /* the default surface, no glFlush: dEQP's order */
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    let n0 = native_cube::copies();
    quad(sample, 16, 16);
    let b = last_bind();
    assert_eq!(b.type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE, "the native-cube program is handed the TPU's own cube");
    assert_eq!(native_cube::copies() - n0, 1, "copied from the drawn image");
    let f = frames();
    assert!(f.submitted >= kick && f.finished >= kick, "the frame that rendered the face was submitted and waited for");
    drop(f);
    assert_eq!(copy_texel(&b, 2), MARKER, "+Y holds what was drawn into it");
    for face in [0, 1, 3, 4, 5] {
        assert_eq!(copy_texel(&b, face), face_rgba(face), "face {face} holds its upload");
    }
}

#[test]
fn a_cube_attached_to_the_drawn_framebuffer_keeps_the_stacked_image() {
    let (_g, _ctx) = start();
    let solid = program(FS_SOLID);
    let sample = program(FS_CUBE);
    let tex = cube();
    face_fbo(tex, GL_TEXTURE_CUBE_MAP_POSITIVE_X);
    quad(solid, SIDE, SIDE);
    let flushes = frames().flushes;
    /* a feedback loop (undefined in GL): sampled while it is the target */
    let n0 = native_cube::copies();
    quad(sample, SIDE, SIDE);
    let b = last_bind();
    assert_eq!(b.type_, GLES_TEXTYPE_CUBE, "the stacked image, as before");
    assert_eq!(native_cube::copies() - n0, 0, "no copy");
    assert_eq!(frames().flushes, flushes, "the frame being recorded is not submitted for it");
}

#[test]
fn a_cube_never_rendered_into_submits_nothing() {
    let (_g, _ctx) = start();
    let solid = program(FS_SOLID);
    let sample = program(FS_CUBE);
    let tex = cube();
    /* a draw into a 2D texture's framebuffer is the open frame */
    let mut t2 = 0;
    let mut fbo = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE1);
        glGenTextures(1, &mut t2);
        glBindTexture(GL_TEXTURE_2D, t2);
        glTexStorage2D(GL_TEXTURE_2D, 1, GL_RGBA8, SIDE as GLsizei, SIDE as GLsizei);
        glActiveTexture(GL_TEXTURE0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t2, 0);
    }
    quad(solid, SIDE, SIDE);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    let flushes = frames().flushes;
    quad(sample, 16, 16);
    let b = last_bind();
    assert_eq!(b.type_, GLES_TEXTYPE_CUBE_TWIDDLED_PROBE);
    assert_eq!(copy_texel(&b, 2), face_rgba(2), "the upload");
    assert_eq!(frames().flushes, flushes, "the open frame does not hold the cube: not submitted for it");
    let _ = tex;
}
