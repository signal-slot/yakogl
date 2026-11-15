// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64) COLOR_ATTACHMENT3 as a texture layer, a 3D slice, a
//! cube face, mip level 1 or a renderbuffer, under the four-target stand-in
//! (`backend::HOST_DRAW_TARGETS` = 4), on the host (item pvr-layers4-test of
//! plan #368; the PowerVR mirror of v3d-layers4-test).
//!
//! The GL core is driven through the API into a stand-in back end that records
//! the `GlesDraw` it is handed (the four `GlesDrawTarget`s: surface address,
//! size, format; the frame's clear) and fills no pixels; the stand-in's GPU
//! addresses are host pointers, so a test writes the surface a target names
//! and reads it back through glReadPixels. That is as far as the host records
//! the fourth PBE state: the PowerVR back end's own words need the board's job
//! memory.
//!
//! RGBA8 renderbuffers sit on 0-2; attachment 3 takes in turn a 2D array
//! layer, a cube face, level 1 of a 2D texture and a renderbuffer, and a 3D
//! slice. Pinned: completeness, the address and size each gets (layer and
//! face are whole images from the first, level 1 follows level 0), a clear and
//! a four-output draw reaching the back end with the same four surfaces, and
//! the read-back of attachment 3. Today's outcome passes; the defect is
//! `#[ignore]`d with its owner: a 3D slice (a grid of slices, `grid_cols` > 1)
//! on attachments 1..3 is UNSUPPORTED because `GlesDrawTarget` carries no row
//! pitch (`slot_pitch_ok`), so nothing draws into it.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_layers_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-powervr", not(feature = "report-es2")))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend::{self, GlesDraw, GlesHeap, YakoglBackend, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D, glTexParameteri,
};
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

/// What the back end was handed by one draw.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `do_clear`.
    cleared: bool,
    /// `targets[..num_targets]`: (gpu, w, h, format).
    targets: Vec<(u64, u32, u32, u8)>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        cleared: d.do_clear,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.w, t.h, t.format)).collect(),
    });
    0
}

unsafe extern "C" fn b_gone(_id: u32) {}

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
    name: c"es3-mrt4-layers-pvr".as_ptr() as *const c_char,
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
    attachment_gone: Some(b_gone),
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
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- framebuffers: RGBA8 on 0-2, the kind under test on 3 ---------------- */

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// `glFramebufferTextureLayer`, a layer of a 3-layer 8x8 array.
    ArrayLayer,
    /// `glFramebufferTextureLayer`, a slice of an 8x8x3 3D texture.
    Slice3d,
    /// `glFramebufferTexture2D`, a face of an 8x8 cube map.
    CubeFace,
    /// `glFramebufferTexture2D`, level 1 of a 16x16 2D texture (8x8).
    Mip1,
    /// `glFramebufferRenderbuffer`, an 8x8 RGBA8 renderbuffer.
    Renderbuffer,
}

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
    texs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glDeleteTextures(self.texs.len() as GLsizei, self.texs.as_ptr());
            glUseProgram(0);
        }
    }
}

fn rgba8_rb(fbo: &mut Fbo) -> GLuint {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
    }
    fbo.rbs.push(rb);
    rb
}

/// A texture of `target` with `levels` levels of RGBA8 (`size` square, and
/// `depth` layers for the layered ones), left bound.
fn texture(fbo: &mut Fbo, target: GLenum, size: GLsizei, depth: GLsizei, levels: GLint) -> GLuint {
    let mut t = 0;
    let zeros = vec![0u8; (size * size * depth.max(1) * 4) as usize];
    let none = zeros.as_ptr() as *const c_void;
    // SAFETY: plain GL calls; no pixel data.
    unsafe {
        glGenTextures(1, &mut t);
        glBindTexture(target, t);
        for l in 0..levels {
            let s = size >> l;
            match target {
                GL_TEXTURE_2D => glTexImage2D(target, l, GL_RGBA as GLint, s, s, 0, GL_RGBA, GL_UNSIGNED_BYTE, none),
                GL_TEXTURE_CUBE_MAP => {
                    for f in 0..6 {
                        let face = GL_TEXTURE_CUBE_MAP_POSITIVE_X + f;
                        glTexImage2D(face, l, GL_RGBA as GLint, s, s, 0, GL_RGBA, GL_UNSIGNED_BYTE, none);
                    }
                }
                _ => glTexImage3D(target, l, GL_RGBA as GLint, s, s, depth, 0, GL_RGBA, GL_UNSIGNED_BYTE, none),
            }
        }
        glTexParameteri(target, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(target, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
    }
    assert_eq!(err(), GL_NO_ERROR, "{target:#x}");
    fbo.texs.push(t);
    t
}

/// The texture or renderbuffer `kind` is made of (a renderbuffer for
/// `Renderbuffer`, whose name comes back), made once; [`attach`] then points
/// attachment 3 at a layer, slice, face or level of it.
fn make(fbo: &mut Fbo, kind: Kind) -> GLuint {
    match kind {
        Kind::ArrayLayer => texture(fbo, GL_TEXTURE_2D_ARRAY, 8, 3, 1),
        Kind::Slice3d => texture(fbo, GL_TEXTURE_3D, 8, 3, 1),
        Kind::CubeFace => texture(fbo, GL_TEXTURE_CUBE_MAP, 8, 0, 1),
        Kind::Mip1 => texture(fbo, GL_TEXTURE_2D, 16, 0, 2),
        Kind::Renderbuffer => rgba8_rb(fbo),
    }
}

/// Attach `obj` (from [`make`]) to `point`: `which` picks the layer, slice or
/// face for the layered kinds, `level` the level of the 2D kind.
fn attach(kind: Kind, obj: GLuint, point: GLenum, which: GLint, level: GLint) {
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        match kind {
            Kind::ArrayLayer | Kind::Slice3d => glFramebufferTextureLayer(GL_FRAMEBUFFER, point, obj, 0, which),
            Kind::CubeFace => {
                glFramebufferTexture2D(GL_FRAMEBUFFER, point, GL_TEXTURE_CUBE_MAP_POSITIVE_X + which as GLenum, obj, 0)
            }
            Kind::Mip1 => glFramebufferTexture2D(GL_FRAMEBUFFER, point, GL_TEXTURE_2D, obj, level),
            Kind::Renderbuffer => glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, obj),
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{kind:?}");
}

/// A bound framebuffer: renderbuffers on 0-2, all four listed in
/// glDrawBuffers, and the object `kind` is made of, not yet attached to 3.
fn framebuffer(kind: Kind) -> (Fbo, GLuint) {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new(), texs: Vec::new() };
    for i in 0..3 {
        let rb = rgba8_rb(&mut fbo);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_RENDERBUFFER, rb) };
    }
    let obj = make(&mut fbo, kind);
    let bufs: [GLenum; 4] = core::array::from_fn(|i| GL_COLOR_ATTACHMENT0 + i as GLenum);
    // SAFETY: plain GL call.
    unsafe { glDrawBuffers(4, bufs.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "{kind:?}");
    (fbo, obj)
}

const A3: GLenum = GL_COLOR_ATTACHMENT0 + 3;

fn status() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glCheckFramebufferStatus(GL_FRAMEBUFFER) }
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

/// The four outputs, exact as RGBA8.
const COLOURS: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 1.0]];

/// The current program: output `k` is the constant `COLOURS[k]`.
fn program() {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for k in 0..4 {
        fs += &format!("layout(location = {k}) out vec4 o{k};\n");
    }
    fs += "void main() {\n";
    for (k, [r, g, b, a]) in COLOURS.iter().enumerate() {
        fs += &format!("    o{k} = vec4({r:?}, {g:?}, {b:?}, {a:?});\n");
    }
    fs += "}\n";
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let v = shader(GL_VERTEX_SHADER, vs);
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

/// The 8x8 target's whole area: one oversized triangle.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw `FULL` with the four-output program and hand back what the back end
/// was given for it.
fn draw() -> Seen {
    program();
    SEEN.lock().unwrap().clear();
    // SAFETY: `FULL` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/// Read attachment `k` of the bound framebuffer as 8x8 RGBA8 bytes.
fn read(k: GLenum) -> Vec<[u8; 4]> {
    let mut v = vec![[0u8; 4]; 64];
    // SAFETY: room for 64 RGBA8 pixels.
    unsafe {
        glReadBuffer(GL_COLOR_ATTACHMENT0 + k);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

/// What the back end got from `glClear` of the colour buffer.
fn clear_all() -> Vec<Seen> {
    SEEN.lock().unwrap().clear();
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.25, 0.5, 0.75, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    core::mem::take(&mut *SEEN.lock().unwrap())
}

/// Fill the 8x8 RGBA8 surface at `gpu` (host memory in the stand-in) with
/// pixel `i` = `[i, 0, 0, 255]`, row-major from the lowest address.
fn fill(gpu: u64) {
    for i in 0..64u64 {
        // SAFETY: the stand-in's gpu addresses are host pointers into
        // allocations of at least 8x8x4 bytes from there.
        unsafe { core::ptr::copy_nonoverlapping([i as u8, 0, 0, 255].as_ptr(), (gpu + i * 4) as *mut u8, 4) };
    }
}

/// The kinds a four-target draw can use today (a 3D slice is no target on
/// attachment 3, see `a_3d_slice_on_attachment_3_is_unsupported_today`).
const DRAWABLE: [Kind; 4] = [Kind::ArrayLayer, Kind::CubeFace, Kind::Mip1, Kind::Renderbuffer];

/// Bytes of one 8x8 RGBA8 image.
const IMG8: u64 = 8 * 8 * 4;

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/* ---- the stand-in's four targets ------------------------------------------ */

#[test]
fn the_stand_in_reports_four_targets() {
    let _g = context();
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), 4);
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), 4);
}

/* ---- completeness ----------------------------------------------------------- */

/// A 2D array layer, a cube face, level 1 of a 2D texture and a renderbuffer
/// on COLOR_ATTACHMENT3 beside RGBA8 renderbuffers on 0-2 are complete.
#[test]
fn attachment_3_completes_for_layer_face_level_and_renderbuffer() {
    let _g = context();
    for kind in DRAWABLE {
        let (_f, obj) = framebuffer(kind);
        for (which, level) in [(0, 0), (1, 0), (2, 0), (0, 1)] {
            if kind == Kind::Renderbuffer && which != 0 {
                continue;
            }
            attach(kind, obj, A3, which, level);
            assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{kind:?} {which} {level}");
        }
    }
}

/// Past the storage a texture attachment is incomplete (not a GL error at
/// the attach call): layer 3 of a 3-layer array, level 5 of the 16x16
/// texture (whose chain is 16, 8, 4, 2, 1). Level 2 is within the chain and
/// complete although the test never specified it (the core judges a level by
/// the chain, as for attachment 0).
#[test]
fn a_layer_or_level_past_the_storage_is_incomplete_on_attachment_3() {
    let _g = context();
    let (_f, obj) = framebuffer(Kind::ArrayLayer);
    attach(Kind::ArrayLayer, obj, A3, 3, 0);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "layer 3 of 3");
    let (_f, obj) = framebuffer(Kind::Mip1);
    attach(Kind::Mip1, obj, A3, 0, 2);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "level 2, in the chain");
    attach(Kind::Mip1, obj, A3, 0, 5);
    assert_eq!(status(), GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "level 5");
}

/// A slice of a 3D texture (kept as a grid of slices, `grid_cols` > 1) is a
/// target on COLOR_ATTACHMENT0, which carries the grid's row pitch
/// (`GlesDraw::target_pitch`), but not on attachments 1..: `GlesDrawTarget`
/// has no pitch, so `slot_pitch_ok` refuses it (UNSUPPORTED) rather than
/// write over the neighbouring slice.
#[test]
fn a_3d_slice_on_attachment_3_is_unsupported_today() {
    let _g = context();
    let (_f, obj) = framebuffer(Kind::Slice3d);
    for slice in 0..3 {
        attach(Kind::Slice3d, obj, A3, slice, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_UNSUPPORTED, "slice {slice}");
    }
}

/// The same slice alone on attachment 0 is complete (the contrast above).
#[test]
fn a_3d_slice_on_attachment_0_is_complete() {
    let _g = context();
    let (_f, obj) = framebuffer(Kind::Slice3d);
    // SAFETY: plain GL calls.
    unsafe {
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 1, GL_RENDERBUFFER, 0);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + 2, GL_RENDERBUFFER, 0);
    }
    attach(Kind::Slice3d, obj, GL_COLOR_ATTACHMENT0, 1, 0);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
}

#[test]
#[ignore = "GL core + PowerVR back end: GlesDrawTarget carries no row pitch, so slot_pitch_ok refuses a 3D grid slice on attachments 1..3 (UNSUPPORTED); the fourth PBE state needs the grid pitch"]
fn a_3d_slice_on_attachment_3_completes() {
    let _g = context();
    let (_f, obj) = framebuffer(Kind::Slice3d);
    for slice in 0..3 {
        attach(Kind::Slice3d, obj, A3, slice, 0);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "slice {slice}");
    }
}

/* ---- what the fourth PBE state gets ---------------------------------------- */

/// Every drawable kind: four targets, the fourth 8x8 RGBA8888 (level 1 of
/// the 16x16 texture is 8x8), distinct from 0-2.
#[test]
fn a_four_output_draw_hands_the_back_end_four_targets() {
    let _g = context();
    for kind in DRAWABLE {
        let (_f, obj) = framebuffer(kind);
        attach(kind, obj, A3, 0, 1);
        assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE, "{kind:?}");
        let s = draw();
        assert_eq!(s.targets.len(), 4, "{kind:?}");
        for (k, t) in s.targets.iter().enumerate() {
            assert_eq!((t.1, t.2, t.3), (8, 8, GLES_SURF_RGBA8888), "{kind:?} target {k}");
        }
        let gpus: Vec<u64> = s.targets.iter().map(|t| t.0).collect();
        for a in 0..4 {
            for b in a + 1..4 {
                assert_ne!(gpus[a], gpus[b], "{kind:?} targets {a} and {b} share a surface");
            }
        }
    }
}

/// A texture attachment names the image's own offset in the stack: layer k
/// of the array and face k of the cube are k whole images from the first,
/// level 1 of the 16x16 texture is the 1024 bytes of level 0 after it, and
/// level 0 is 16x16. Targets 0-2 do not move with it.
#[test]
fn attachment_3_addresses_the_layer_face_or_level() {
    let _g = context();
    for (kind, which, level, delta) in [
        (Kind::ArrayLayer, 1, 0, IMG8),
        (Kind::ArrayLayer, 2, 0, 2 * IMG8),
        (Kind::CubeFace, 1, 0, IMG8),
        (Kind::CubeFace, 5, 0, 5 * IMG8),
        (Kind::Mip1, 0, 1, 16 * 16 * 4),
    ] {
        let (_f, obj) = framebuffer(kind);
        attach(kind, obj, A3, 0, 0);
        let first = draw();
        attach(kind, obj, A3, which, level);
        let moved = draw();
        assert_eq!(moved.targets[3].0 - first.targets[3].0, delta, "{kind:?} {which} {level}");
        assert_eq!((moved.targets[3].1, moved.targets[3].2), (8, 8), "{kind:?} {which} {level}");
        assert_eq!(&moved.targets[..3], &first.targets[..3], "{kind:?}: targets 0-2 stay");
    }
    let (_f, obj) = framebuffer(Kind::Mip1);
    attach(Kind::Mip1, obj, A3, 0, 0);
    let s = draw();
    assert_eq!((s.targets[3].1, s.targets[3].2), (16, 16), "level 0 of the 16x16 texture");
}

/// A second renderbuffer on attachment 3 moves target 3 to its own surface
/// and leaves 0-2 alone.
#[test]
fn a_renderbuffer_on_attachment_3_is_its_own_allocation() {
    let _g = context();
    let (mut f, obj) = framebuffer(Kind::Renderbuffer);
    attach(Kind::Renderbuffer, obj, A3, 0, 0);
    let a = draw();
    let rb2 = rgba8_rb(&mut f);
    attach(Kind::Renderbuffer, rb2, A3, 0, 0);
    let b = draw();
    assert_ne!(a.targets[3].0, b.targets[3].0);
    assert_eq!(&a.targets[..3], &b.targets[..3]);
}

/* ---- clear and four-output draw --------------------------------------------- */

/// glClear of the colour buffer is one back-end call with the frame's clear,
/// no vertices, and the same four surfaces a draw writes.
#[test]
fn a_clear_reaches_the_back_end_with_the_four_surfaces_of_a_draw() {
    let _g = context();
    for kind in DRAWABLE {
        let (_f, obj) = framebuffer(kind);
        attach(kind, obj, A3, 1, 1);
        let drawn = draw();
        let cleared = clear_all();
        assert_eq!(cleared.len(), 1, "{kind:?}");
        assert!(cleared[0].cleared && !cleared[0].draws, "{kind:?}");
        assert_eq!(cleared[0].targets, drawn.targets, "{kind:?}");
    }
}

/// The four-output program draws into the four-target framebuffer: it
/// compiles and links, and the draw is an application draw (vertices) with the
/// frame's four targets.
#[test]
fn a_four_output_program_draws_into_each_kind() {
    let _g = context();
    for kind in DRAWABLE {
        let (_f, obj) = framebuffer(kind);
        attach(kind, obj, A3, 2, 1);
        let s = draw();
        assert!(s.draws && !s.cleared, "{kind:?}");
        assert_eq!(s.targets.len(), 4, "{kind:?}");
    }
}

/* ---- read-back ---------------------------------------------------------------- */

/// What lies in the surface the fourth target names is what glReadPixels of
/// COLOR_ATTACHMENT3 returns, row 0 first, and attachment 0 reads its own.
#[test]
fn reading_attachment_3_returns_the_surface_the_draw_targets() {
    let _g = context();
    for kind in DRAWABLE {
        let (_f, obj) = framebuffer(kind);
        for (which, level) in [(0, 0), (1, 0), (2, 0), (0, 1)] {
            if kind == Kind::Renderbuffer && which != 0 {
                continue;
            }
            attach(kind, obj, A3, which, level);
            let s = draw();
            if (s.targets[3].1, s.targets[3].2) != (8, 8) {
                continue; // level 0 of the 16x16 texture
            }
            fill(s.targets[3].0);
            let px = read(3);
            for (i, p) in px.iter().enumerate() {
                assert_eq!(*p, [i as u8, 0, 0, 255], "{kind:?} {which} {level} pixel {i}");
            }
            assert!(read(0).iter().all(|p| *p == [0, 0, 0, 0]), "{kind:?}: attachment 0 reads its own surface");
        }
    }
}

/// Two layers of one array hold their own pixels: filling layer 1 does not
/// show in layer 0 or 2.
#[test]
fn neighbouring_layers_read_back_apart() {
    let _g = context();
    let (_f, obj) = framebuffer(Kind::ArrayLayer);
    attach(Kind::ArrayLayer, obj, A3, 1, 0);
    fill(draw().targets[3].0);
    for (layer, filled) in [(0, false), (1, true), (2, false)] {
        attach(Kind::ArrayLayer, obj, A3, layer, 0);
        let px = read(3);
        assert_eq!(px[5] == [5, 0, 0, 255], filled, "layer {layer}");
        if !filled {
            assert!(px.iter().all(|p| *p == [0, 0, 0, 0]), "layer {layer}");
        }
    }
}

/// A 3D slice cannot be drawn on attachment 3 today: the draw is
/// GL_INVALID_FRAMEBUFFER_OPERATION and the back end sees none. (Reading the
/// unsupported framebuffer is not refused.)
#[test]
fn a_3d_slice_on_attachment_3_draws_nothing_today() {
    let _g = context();
    let (_f, obj) = framebuffer(Kind::Slice3d);
    attach(Kind::Slice3d, obj, A3, 1, 0);
    program();
    SEEN.lock().unwrap().clear();
    // SAFETY: `FULL` is static; plain GL calls.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(glGetError(), GL_INVALID_FRAMEBUFFER_OPERATION);
        let mut px = [0u8; 4];
        glReadBuffer(A3);
        glReadPixels(0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        // The read is not refused: it takes the slice's own surface.
        assert_eq!(glGetError(), GL_NO_ERROR);
        assert_eq!(px, [0, 0, 0, 0]);
    }
    assert!(SEEN.lock().unwrap().iter().all(|s| !s.draws));
}

#[test]
#[ignore = "GL core + PowerVR back end: a 3D slice on attachment 3 is UNSUPPORTED (GlesDrawTarget has no pitch), so no draw or clear reaches it"]
fn a_3d_slice_on_attachment_3_draws() {
    let _g = context();
    let (_f, obj) = framebuffer(Kind::Slice3d);
    attach(Kind::Slice3d, obj, A3, 1, 0);
    let s = draw();
    assert_eq!(s.targets.len(), 4);
    assert_eq!((s.targets[3].1, s.targets[3].2), (8, 8));
}
