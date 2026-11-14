// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Colour attachment 3 as a texture layer, a 3D slice, a cube face, a mip
//! level or a renderbuffer, with four draw buffers on the V3D back end's host
//! build (plan #368, item v3d-layers4-test).
//!
//! `backend::HOST_DRAW_TARGETS` = 4 stands in for the knob
//! (tests/es3_rt_masks_v3d.rs). The back end is the smallest one that makes a
//! context, and its `draw` is a stand-in GPU: for a clear (`do_clear`) it
//! fills the RGBA8 image at each selected `targets[k].gpu` with `clear_rgba`,
//! for an application draw it fills target k with the colour output k of the
//! program writes, both as a plain raster of `w * 4` bytes a row. So what is
//! judged is the address the GL core hands the back end for render target 3
//! (`targets[3].gpu`) and what glReadBuffer(GL_COLOR_ATTACHMENT3) +
//! glReadPixels finds there: the drawn values at the layer, face, slice or
//! level attached, and the other layers, faces, slices and levels as they
//! were. Each image of the attachment-3 object is also read through a
//! framebuffer of its own (as attachment 0).
//!
//! Attachments 0-2 are plain 8x8 RGBA8 textures.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_layers_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- a context on the host: the smallest OSAL and back end that make one -- */

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

/// The colour output k of the program writes, as bytes.
const OUT: [[u8; 4]; 4] = [[255, 0, 0, 255], [0, 255, 0, 255], [0, 0, 255, 255], [255, 255, 0, 255]];

/// What the back end was handed by one call: `targets[..n]` (gpu, w, h).
#[derive(Clone)]
struct Seen {
    draws: bool,
    clear: bool,
    targets: Vec<(u64, u32, u32)>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe fn fill(t: &GlesDrawTarget, px: [u8; 4]) {
    let p = t.gpu as *mut u8;
    for i in 0..(t.w * t.h) as usize {
        core::ptr::copy_nonoverlapping(px.as_ptr(), p.add(i * 4), 4);
    }
}

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let draws = d.index_count != 0 && !d.clear_quad;
    if d.do_clear {
        let px = d.clear_rgba.map(|v| (v * 255.0 + 0.5) as u8);
        for k in 0..n {
            if d.clear_target_mask == 0 || d.clear_target_mask & (1 << k) != 0 {
                fill(&d.targets[k], px);
            }
        }
    }
    if draws {
        for k in 0..n {
            fill(&d.targets[k], OUT[k + d.targets[0].out_shift as usize]);
        }
    }
    SEEN.lock().unwrap().push(Seen {
        draws,
        clear: d.do_clear,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h)).collect(),
    });
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
    name: c"es3-mrt4-layers".as_ptr() as *const c_char,
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: both tables are statics; the lock above serialises the tests.
        unsafe {
            osal_set_ops(&OSAL);
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    while err() != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
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

/// The current program: four outputs, `OUT[k]` on location k.
fn program() {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for k in 0..4 {
        fs += &format!("layout(location = {k}) out vec4 o{k};\n");
    }
    fs += "void main() {\n";
    for (k, c) in OUT.iter().enumerate() {
        fs += &format!("o{k} = vec4({:.1}, {:.1}, {:.1}, {:.1});\n", c[0] / 255, c[1] / 255, c[2] / 255, c[3] / 255);
    }
    fs += "}\n";
    let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Draw one full-screen triangle; the back end's view of the application draw.
fn draw() -> Seen {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    SEEN.lock().unwrap().clear();
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// The last clear the back end was handed.
fn last_clear() -> Seen {
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.clear).expect("the clear reached the back end")
}

/* ---- the attachment 3 variants ---------------------------------------------- */

#[derive(Clone, Copy, Debug)]
enum Kind {
    Layer(i32),
    Slice(i32),
    Face(u32),
    Level(i32),
    Renderbuffer,
}

const DEPTH: i32 = 3;

/// The draw framebuffer, and every image of the attachment-3 object as a
/// framebuffer of its own, `.1` set for the one that is attachment 3.
struct Rig {
    fb: GLuint,
    /// glCheckFramebufferStatus of the draw framebuffer.
    status: GLenum,
    images: Vec<(GLuint, bool)>,
}

unsafe fn tex2d(w: i32, levels: i32) -> GLuint {
    let mut t = 0;
    glGenTextures(1, &mut t);
    glBindTexture(GL_TEXTURE_2D, t);
    glTexStorage2D(GL_TEXTURE_2D, levels, GL_RGBA8, w, w);
    assert_eq!(err(), GL_NO_ERROR);
    t
}

unsafe fn image_fbo(images: &mut Vec<(GLuint, bool)>, is3: bool, attach: impl Fn(GLenum)) {
    let mut f = 0;
    glGenFramebuffers(1, &mut f);
    glBindFramebuffer(GL_FRAMEBUFFER, f);
    attach(GL_COLOR_ATTACHMENT0);
    images.push((f, is3));
}

/// A framebuffer whose attachments 0-2 are plain 8x8 RGBA8 textures and
/// attachment 3 is `kind`.
fn rig(kind: Kind) -> Rig {
    let (mut fb, mut images) = (0, Vec::new());
    let status;
    // SAFETY: plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for i in 0..3 {
            let t = tex2d(8, 1);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_TEXTURE_2D, t, 0);
        }
        let a3 = GL_COLOR_ATTACHMENT3;
        match kind {
            Kind::Layer(l) | Kind::Slice(l) => {
                let tgt = if matches!(kind, Kind::Layer(_)) { GL_TEXTURE_2D_ARRAY } else { GL_TEXTURE_3D };
                let mut t = 0;
                glGenTextures(1, &mut t);
                glBindTexture(tgt, t);
                glTexStorage3D(tgt, 1, GL_RGBA8, 8, 8, DEPTH);
                assert_eq!(err(), GL_NO_ERROR);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTextureLayer(GL_FRAMEBUFFER, a3, t, 0, l);
                for z in 0..DEPTH {
                    image_fbo(&mut images, z == l, |a| glFramebufferTextureLayer(GL_FRAMEBUFFER, a, t, 0, z));
                }
            }
            Kind::Face(face) => {
                let mut t = 0;
                glGenTextures(1, &mut t);
                glBindTexture(GL_TEXTURE_CUBE_MAP, t);
                glTexStorage2D(GL_TEXTURE_CUBE_MAP, 1, GL_RGBA8, 8, 8);
                assert_eq!(err(), GL_NO_ERROR);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, a3, GL_TEXTURE_CUBE_MAP_POSITIVE_X + face, t, 0);
                for f in 0..6 {
                    image_fbo(&mut images, f == face, |a| {
                        glFramebufferTexture2D(GL_FRAMEBUFFER, a, GL_TEXTURE_CUBE_MAP_POSITIVE_X + f, t, 0)
                    });
                }
            }
            Kind::Level(lv) => {
                /* levels 32x32, 16x16, 8x8: level 2 is the size of the others */
                let t = tex2d(32, 3);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferTexture2D(GL_FRAMEBUFFER, a3, GL_TEXTURE_2D, t, lv);
                for l in 0..3 {
                    image_fbo(&mut images, l == lv, |a| glFramebufferTexture2D(GL_FRAMEBUFFER, a, GL_TEXTURE_2D, t, l));
                }
            }
            Kind::Renderbuffer => {
                let mut rb = 0;
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
                glBindFramebuffer(GL_FRAMEBUFFER, fb);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, a3, GL_RENDERBUFFER, rb);
                image_fbo(&mut images, true, |a| glFramebufferRenderbuffer(GL_FRAMEBUFFER, a, GL_RENDERBUFFER, rb));
            }
        }
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let bufs: Vec<GLenum> = (0..4).map(|i| GL_COLOR_ATTACHMENT0 + i).collect();
        glDrawBuffers(4, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "{kind:?}");
        status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        for (f, _) in &images {
            glBindFramebuffer(GL_FRAMEBUFFER, *f);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{kind:?} image");
        }
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    Rig { fb, status, images }
}

/// Read pixel (x, y) of attachment `att` of framebuffer `fb`.
fn read(fb: GLuint, att: GLenum, x: i32, y: i32) -> [u8; 4] {
    let mut px = [0u8; 4];
    // SAFETY: room for one RGBA8 pixel.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glReadBuffer(att);
        glReadPixels(x, y, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    px
}

/// The pixel every one of the 8x8 pixels of the image has, or `None`.
fn uniform(fb: GLuint) -> Option<[u8; 4]> {
    let first = read(fb, GL_COLOR_ATTACHMENT0, 0, 0);
    for y in 0..8 {
        for x in 0..8 {
            if read(fb, GL_COLOR_ATTACHMENT0, x, y) != first {
                return None;
            }
        }
    }
    Some(first)
}

/// The image of every framebuffer of the rig: the attached one `on`, the
/// others `off`.
fn assert_images(r: &Rig, kind: Kind, what: &str, on: [u8; 4], off: [u8; 4]) {
    for (i, (f, is3)) in r.images.iter().enumerate() {
        let want = if *is3 { on } else { off };
        assert_eq!(uniform(*f), Some(want), "{kind:?}: {what}, image {i} (attachment 3: {is3})");
    }
}

/// One variant: the draw and each clear write only the attached image of
/// attachment 3.
fn check(kind: Kind) {
    let _g = context();
    let r = rig(kind);
    assert_eq!(r.status, GL_FRAMEBUFFER_COMPLETE, "{kind:?}");
    program();
    assert_eq!(r.images.iter().filter(|i| i.1).count(), 1);
    assert_images(&r, kind, "before", [0; 4], [0; 4]);

    /* a draw: attachment 3 gets output 3 */
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, r.fb) };
    let s = draw();
    assert_eq!(s.targets.len(), 4, "{kind:?}: four targets reach the back end");
    let a3_addr = s.targets[3].0;
    for k in 0..3 {
        assert_ne!(s.targets[k].0, a3_addr, "{kind:?}: target {k} is an image of its own");
    }
    assert_eq!(
        read(r.fb, GL_COLOR_ATTACHMENT3, 3, 4),
        OUT[3],
        "{kind:?}: draw, attachment 3 read through the draw framebuffer"
    );
    assert_images(&r, kind, "draw", OUT[3], [0; 4]);
    for k in 0..3 {
        assert_eq!(read(r.fb, GL_COLOR_ATTACHMENT0 + k, 1, 1), OUT[k as usize], "{kind:?}: draw, target {k}");
    }

    /* glClearBufferfv of draw buffer 3 only */
    let v = [0.0f32, 0.0, 0.0, 1.0];
    // SAFETY: four floats.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, r.fb);
        glClearBufferfv(GL_COLOR, 3, v.as_ptr());
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(last_clear().targets[3].0, a3_addr, "{kind:?}: the clear goes to the draw's address");
    assert_images(&r, kind, "ClearBuffer", [0, 0, 0, 255], [0; 4]);
    assert_eq!(read(r.fb, GL_COLOR_ATTACHMENT2, 2, 2), OUT[2], "{kind:?}: ClearBuffer 3 leaves target 2");

    /* glClear of all four */
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 1.0, 1.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(last_clear().targets[3].0, a3_addr, "{kind:?}: glClear goes to the same address");
    assert_images(&r, kind, "glClear", [0, 255, 255, 255], [0; 4]);
    assert_eq!(read(r.fb, GL_COLOR_ATTACHMENT0, 0, 0), [0, 255, 255, 255], "{kind:?}: glClear reaches target 0");
}

#[test]
fn attachment3_a_layer_of_a_2d_array() {
    check(Kind::Layer(2));
}

/// Today: a slice of a 3D texture (a grid in memory, `grid_cols > 1`) is no
/// colour attachment 1.. (`slot_pitch_ok`, src/gl/framebuffer.rs): the back
/// ends write those slots at `tex_pitch(w)`, so the framebuffer is
/// GL_FRAMEBUFFER_UNSUPPORTED and nothing is drawn.
#[test]
fn attachment3_a_slice_of_a_3d_texture_is_unsupported_today() {
    let _g = context();
    let r = rig(Kind::Slice(2));
    assert_eq!(r.status, GL_FRAMEBUFFER_UNSUPPORTED);
}

/// ES 3.0 4.4.4: a slice of a 3D texture on COLOR_ATTACHMENT3 is complete and
/// the draw and the clears write that slice only.
#[test]
#[ignore = "defect: src/gl/framebuffer.rs slot_pitch_ok refuses a 3D slice on colour attachment 1.. (the back end writes slots 1.. at tex_pitch(w), no pitch in GlesDrawTarget); the plan's GL-core owner"]
fn attachment3_a_slice_of_a_3d_texture() {
    check(Kind::Slice(2));
}

#[test]
fn attachment3_a_cube_face() {
    check(Kind::Face(3));
}

#[test]
fn attachment3_a_mip_level() {
    check(Kind::Level(2));
}

#[test]
fn attachment3_a_renderbuffer() {
    check(Kind::Renderbuffer);
}
