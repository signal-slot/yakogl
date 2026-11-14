// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! dEQP-GLES3 `fragment_out.*` shapes on the V3D host back end
//! (t-fragout-v3d): what the job state carries for them today, and what
//! docs/notes/v3d-fragment-out.md designs for the 158 rows that are not Pass
//! on the Pi 4.
//!
//! The GL core is driven through the API into a stand-in back end, which at
//! the draw runs the V3D's own `state::refusal`, `state::unsupported` and
//! `state::job_state` on what it was handed, and keeps the `state::rt_format`
//! row of every target (the stand-in does not record a job; the rows and the
//! clear words are what the job builder is given).
//!
//! Passing tests pin today:
//! - a draw of a float, an unsigned and a signed integer output into
//!   RGBA8 + RGBA32UI + RG16I (the 782 passing rows' mixed shapes);
//! - the `rt_format` rows and `clear_words` of the 16F and 8-bit formats;
//! - what the note's G1-G3 (RG32F, RGBA32F, R11F_G11F_B10F), G4 (RG16F at an
//!   attachment past the first) and G5 (four draw buffers) rows do now.
//!
//! `#[ignore]`d tests hold the expected result, each naming the note section
//! and the layer whose step is missing (src/gl, src/shaderc, src/v3d). They
//! are the acceptance tests of those steps.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_fragment_out_v3d
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
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::v3d::ffi::*;
use yakogl::v3d::state;

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

/// What the back end was handed by one draw, and what the V3D made of it.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `state::refusal` / `state::unsupported`.
    refused: Option<&'static str>,
    /// `state::job_state`, when the V3D takes the draw.
    job: Option<V3dJobState>,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let refused = state::refusal(d).or_else(|| state::unsupported(d));
    let job = if d.index_count != 0 { state::job_state(d).ok() } else { None };
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        refused,
        job,
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
    name: c"es3-fragment-out".as_ptr() as *const c_char,
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

/// A context whose back end writes `targets` colour targets in one draw
/// (`backend::HOST_DRAW_TARGETS`); 0 is the build's own `DRAW_TARGETS`, which
/// is 3 for the V3D.
fn context(targets: u8) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
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
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- the framebuffer, the program, the draw ---------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// dEQP's setup (`es3fFragmentOutputTests.cpp`): one renderbuffer of each
/// sized format on colour attachments 0.., all of them draw buffers. The
/// first GL error of the sequence, and which attachment it came at, is the
/// `Err`.
fn try_framebuffer(formats: &[GLenum]) -> Result<Fbo, (usize, GLenum)> {
    let (mut fb, mut rbs) = (0, Vec::new());
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let mut fbo = Fbo { fb, rbs: Vec::new() };
        for (i, f) in formats.iter().enumerate() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            rbs.push(rb);
            fbo.rbs = rbs.clone();
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, *f, 8, 8);
            let e = err();
            if e != GL_NO_ERROR {
                return Err((i, e));
            }
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
            let e = err();
            if e != GL_NO_ERROR {
                return Err((i, e));
            }
        }
        let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        let e = err();
        if e != GL_NO_ERROR {
            return Err((formats.len(), e));
        }
        let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if st != GL_FRAMEBUFFER_COMPLETE {
            return Err((formats.len(), st));
        }
        Ok(fbo)
    }
}

fn framebuffer(formats: &[GLenum]) -> Fbo {
    try_framebuffer(formats).unwrap_or_else(|e| panic!("{formats:#x?}: {e:#x?}"))
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

/// The current program: one `out` per entry of `types` ("vec2", "vec4", "ivec4",
/// "uvec4"), at location `i`, highp as dEQP's highp rows.
fn program(types: &[&str]) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, t) in types.iter().enumerate() {
        fs += &format!("layout(location = {i}) out {t} o{i};\n");
    }
    fs += "void main() {\n";
    for (i, t) in types.iter().enumerate() {
        fs += &match *t {
            "vec4" => format!("o{i} = vec4(0.25, 0.5, 0.75, 0.5);\n"),
            "vec2" => format!("o{i} = vec2(0.25, 0.5);\n"),
            "ivec4" => format!("o{i} = ivec4(-1, 2, -3, 4);\n"),
            _ => format!("o{i} = uvec4(1u, 2u, 3u, 4u);\n"),
        };
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

/// Draw one triangle and hand back what the V3D made of it.
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

/// A draw into `formats` with one output of each class in `outs`.
fn drawn(formats: &[GLenum], outs: &[&str]) -> Seen {
    let fbo = framebuffer(formats);
    program(outs);
    let s = draw();
    drop(fbo);
    s
}

/// The surface classes the back end saw, one per target.
fn surfs(s: &Seen) -> Vec<u8> {
    s.targets.iter().map(|t| t.1).collect()
}

/// Largest internal bpp of the targets' `rt_format` rows: what the job's
/// common RT config carries (and with the RT count, the tile size,
/// `v3d_frame.c:85`).
fn max_bpp(s: &Seen) -> u32 {
    surfs(s).iter().map(|&f| state::rt_format(f).expect("a row").internal_bpp).max().unwrap()
}

/// `(output, internal type, internal bpp, clamp, bytes)` of a row.
fn row(surf: u8) -> (u32, u32, u32, u32, u32) {
    let r = state::rt_format(surf).unwrap_or_else(|| panic!("surface {surf} has a row"));
    (r.output, r.internal_type, r.internal_bpp, r.clamp, r.bytes)
}

/* ---- (1) shapes the V3D takes today -------------------------------------- */

/// fragment_out `random` with a float, an unsigned and a signed integer
/// output in one draw: RGBA8 + RGBA32UI + RG16I. No refusal, a job state,
/// the three surface classes in order, 128 bpp for the common config (the
/// tile size then follows from three RTs at 128 bpp, `v3d_frame.c:85`).
#[test]
fn float_uint_int_outputs_into_rgba8_rgba32ui_rg16i_are_taken() {
    let _g = context(0);
    let s = drawn(&[GL_RGBA8, GL_RGBA32UI, GL_RG16I], &["vec4", "uvec4", "ivec4"]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some(), "job_state accepts the mixed draw");
    assert_eq!(surfs(&s), [GLES_SURF_RGBA8888, GLES_SURF_RGBA32UI, GLES_SURF_RG16I]);
    assert!(s.targets.iter().all(|t| t.0 != 0));
    assert_eq!(max_bpp(&s), V3D_INTERNAL_BPP_128);
}

/// The same outputs in the other order, an integer first: the class is the
/// target's own, whichever slot it is in.
#[test]
fn int_first_order_is_taken_too() {
    let _g = context(0);
    let s = drawn(&[GL_RG16I, GL_RGBA8, GL_RGBA32UI], &["ivec4", "vec4", "uvec4"]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
    assert_eq!(surfs(&s), [GLES_SURF_RG16I, GLES_SURF_RGBA8888, GLES_SURF_RGBA32UI]);
}

/// Colour write masks are a job-state word for every type: the all-on mask
/// of a fresh context reaches the job for the mixed draw.
#[test]
fn mixed_draw_has_no_blend_and_the_full_colour_mask() {
    let _g = context(0);
    let s = drawn(&[GL_RGBA8, GL_RGBA32UI, GL_RG16I], &["vec4", "uvec4", "ivec4"]);
    let j = s.job.unwrap();
    assert!(!j.blend_enabled, "fragment_out does not blend");
    assert_eq!(j.colormask, 0xf);
}

/// The `rt_format` rows of the 16F and 8-bit formats the basic group draws
/// (`v3dx_format_table.c`, rows cited in `state::rt_format`): RGBA16F and
/// R16F are 16F at 64 bpp (the TLB keeps alpha), RGBA8 is the 8-bit type at
/// 32 bpp.
#[test]
fn rt_format_rows_of_16f_and_8bit_formats() {
    assert_eq!(
        row(GLES_SURF_RGBA16F),
        (V3D_OUTPUT_IMAGE_FORMAT_RGBA16F, V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, V3D_RENDER_TARGET_CLAMP_NONE, 8)
    );
    assert_eq!(
        row(GLES_SURF_R16F),
        (V3D_OUTPUT_IMAGE_FORMAT_R16F, V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, V3D_RENDER_TARGET_CLAMP_NONE, 2)
    );
    assert_eq!(
        row(GLES_SURF_RGBA8888),
        (V3D_OUTPUT_IMAGE_FORMAT_RGBA8, V3D_INTERNAL_TYPE_8, V3D_INTERNAL_BPP_32, V3D_RENDER_TARGET_CLAMP_NONE, 4)
    );
    assert_eq!(row(GLES_SURF_XRGB8888), row(GLES_SURF_RGBA8888));
    let r565 = row(GLES_SURF_RGB565);
    assert_eq!((r565.1, r565.2, r565.4), (V3D_INTERNAL_TYPE_8, V3D_INTERNAL_BPP_32, 2));
    /* the fixed-point formats of the random list over a 16F tile buffer */
    assert_eq!(row(GLES_SURF_SRGB8_A8).1, V3D_INTERNAL_TYPE_16F);
    assert_eq!(row(GLES_SURF_RGB10_A2).1, V3D_INTERNAL_TYPE_16F);
    assert_eq!(row(GLES_SURF_RGB10_A2).2, V3D_INTERNAL_BPP_64);
    /* R32F has a row (32F at 32 bpp) although its program writes halves, note G7 */
    assert_eq!(
        row(GLES_SURF_R32F),
        (V3D_OUTPUT_IMAGE_FORMAT_R32F, V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_32, V3D_RENDER_TARGET_CLAMP_NONE, 4)
    );
}

/// The clear words of the 16F and 8-bit formats: halves two to a word for
/// the 16F ones, the RGBA8 word for RGBA8.
#[test]
fn clear_words_of_16f_and_8bit_formats() {
    let c = [1.0f32, 0.5, -2.0, 0.0];
    assert_eq!(state::clear_words(GLES_SURF_RGBA16F, c), [0x3800_3c00, 0x0000_c000, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_R16F, c), [0x3800_3c00, 0x0000_c000, 0, 0], "R16F is a 64-bpp 16F row");
    assert_eq!(state::clear_words(GLES_SURF_RGBA8888, [1.0, 0.0, 0.0, 1.0]), [0xff00_00ff, 0, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RGBA8888, [0.0, 0.0, 0.0, 0.0]), [0, 0, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_XRGB8888, [1.0, 0.0, 0.0, 1.0])[1..], [0, 0, 0]);
    /* the fixed-point formats clamp to [0, 1] over their 16F tile buffer */
    assert_eq!(state::clear_words(GLES_SURF_SRGB8_A8, c), [0x3800_3c00, 0, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RGB10_A2, c), [0x3800_3c00, 0, 0, 0]);
    /* the integer classes of the mixed draw: 16 bits a channel, 32 as they are */
    let ints = [1u32, -2i32 as u32, 3, 4].map(f32::from_bits);
    assert_eq!(state::clear_words(GLES_SURF_RG16I, ints), [0xfffe_0001, 0x0004_0003, 0, 0]);
    assert_eq!(state::clear_words(GLES_SURF_RGBA32UI, ints), ints.map(f32::to_bits));
}

/// A float renderbuffer at every slot of a three-attachment `array` case
/// that the V3D has a row for: RGBA16F x 3 is taken, as the passing basic
/// RGBA16F rows.
#[test]
fn three_rgba16f_attachments_are_taken() {
    let _g = context(0);
    let s = drawn(&[GL_RGBA16F; 3], &["vec4", "vec4", "vec4"]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
    assert_eq!(surfs(&s), [GLES_SURF_RGBA16F; 3]);
    assert_eq!(max_bpp(&s), V3D_INTERNAL_BPP_64);
}

/* ---- G4: RG16F at an attachment past the first ----------------------------- */

/// Note G4 (`array.float.rg16f_*`): RG16F is RGBA16F storage at attachments 1
/// and 2 as it is at attachment 0 (`fb_target_n` maps GL_RG16F and GL_RGB16F
/// to RGBA16F).
#[test]
fn rg16f_past_the_first_attachment_is_rgba16f() {
    let _g = context(0);
    let s = drawn(&[GL_RG16F; 3], &["vec2", "vec2", "vec2"]);
    assert_eq!(surfs(&s), [GLES_SURF_RGBA16F; 3]);
}

/* ---- (2) refused today: RG32F, RGBA32F, R11F_G11F_B10F -------------------- */

/// G1-G3: no 32-bit float or R11F_G11F_B10F renderbuffer can be a colour
/// attachment. `GL_EXT_color_buffer_float` is not claimed
/// (`docs/es3-cts-failures.md` "Float targets: not claimed"), and the sized
/// format is not a colour-renderable one, so the GL core stops the setup at
/// the storage call and no draw reaches the V3D. dEQP reads the extension
/// string and answers NotSupported before it gets this far; the GL error
/// here is what an application that tries anyway sees.
#[test]
fn rg32f_rgba32f_r11f_renderbuffers_are_refused_by_the_gl_core() {
    let _g = context(0);
    for f in [GL_R32F, GL_RG32F, GL_RGBA32F, GL_R11F_G11F_B10F] {
        let e = try_framebuffer(&[f]).err().unwrap_or_else(|| panic!("{f:#x} unexpectedly set up"));
        assert_eq!(e.0, 0, "{f:#x}");
        assert_eq!(e.1, GL_INVALID_ENUM, "{f:#x}");
    }
}

/// The extension is not in the string: the NotSupported of G1/G2/G3.
#[test]
fn color_buffer_float_is_not_claimed() {
    let _g = context(0);
    // SAFETY: GL_EXTENSIONS is a NUL-terminated string the core owns.
    let s =
        unsafe { std::ffi::CStr::from_ptr(glGetString(GL_EXTENSIONS) as *const c_char) }.to_str().unwrap().to_owned();
    assert!(!s.split(' ').any(|e| e == "GL_EXT_color_buffer_float"), "{s}");
    assert!(s.split(' ').any(|e| e == "GL_EXT_color_buffer_half_float"));
}

/// Mixed with the formats that do render, the setup of an array case with a
/// 32F attachment stops at that attachment too (G3: a `random` seed with
/// one of the four formats).
#[test]
fn a_32f_attachment_among_rendering_ones_stops_the_setup() {
    let _g = context(0);
    let e = try_framebuffer(&[GL_RGBA8, GL_RGBA32F, GL_RG16I]).err().expect("refused");
    assert_eq!(e.0, 1, "at the RGBA32F attachment");
}

/// The four-draw-buffer shape (G5): the V3D build's `DRAW_TARGETS` is 3, so
/// `GL_COLOR_ATTACHMENT3` is not a colour attachment point
/// (`is_color_point`, `max_color_attachments`) and dEQP's "After framebuffer
/// setup" check sees GL_INVALID_ENUM.
#[test]
fn four_draw_buffers_are_refused_by_the_gl_core() {
    let _g = context(0);
    let mut n: GLint = 0;
    // SAFETY: plain GL call, one GLint.
    unsafe { glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut n) };
    assert_eq!(n, 3, "backend::DRAW_TARGETS for the V3D");
    let e = try_framebuffer(&[GL_RGBA8; 4]).err().expect("refused");
    assert_eq!(e, (3, GL_INVALID_ENUM));
}

/// Three draw buffers of the mixed classes are the most that is taken now.
#[test]
fn three_draw_buffers_of_mixed_classes_are_the_cap() {
    let _g = context(0);
    let s = drawn(&[GL_RGBA8, GL_RGBA32UI, GL_RG16I], &["vec4", "uvec4", "ivec4"]);
    assert_eq!(s.targets.len(), 3);
}

/* ---- (2) expected: the formats render (note "Design for the src/v3d part") - */

/// Set up `f` as the one attachment, draw `out`, and hand back the draw.
/// The format class is whatever the GL core numbers it; the assertions go
/// through `state::rt_format` of it, so they hold in any numbering.
fn drawn_expected(f: GLenum, out: &str) -> Seen {
    let s = drawn(&[f], &[out]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
    s
}

/// RG32F renders: the row is `V3D_OUTPUT_IMAGE_FORMAT_RG32F` (10) 32F, 64 bpp,
/// clamp NONE, 8 bytes (`v3dx_format_table.c:96`, bpp `:323-326`); a clear
/// hands the words as they are.
///
/// Owners: src/gl (claim `GL_EXT_color_buffer_float`, `is_color_format`, a
/// surface class and `fb_target_n` arm: note "What is not a src/v3d change",
/// order step 3) and src/v3d (`state::rt_format`, `ffi.rs` constants: "Design
/// for the src/v3d part" piece 1).
#[test]
#[ignore = "note design piece 1 / order step 3: src/gl (extension, surface class, is_color_format) then src/v3d (rt_format row RG32F = 10/32F/64/NONE/8)"]
fn rg32f_renders_with_the_32f_64bpp_row() {
    let _g = context(0);
    let s = drawn_expected(GL_RG32F, "vec4");
    let (out, ty, bpp, clamp, bytes) = row(s.targets[0].1);
    assert_eq!(
        (out, ty, bpp, clamp, bytes),
        (10, V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_64, V3D_RENDER_TARGET_CLAMP_NONE, 8)
    );
    let c = [1.5f32, -2.0, 1e30, 0.0];
    assert_eq!(state::clear_words(s.targets[0].1, c), c.map(f32::to_bits));
}

/// RGBA32F renders: format 9, 32F, 128 bpp, clamp NONE, 16 bytes
/// (`v3dx_format_table.c:101`, `:319-322`).
///
/// Owners: as RG32F (src/gl then src/v3d rows, note piece 1).
#[test]
#[ignore = "note design piece 1 / order step 3: src/gl (extension, surface class) then src/v3d (rt_format row RGBA32F = 9/32F/128/NONE/16)"]
fn rgba32f_renders_with_the_32f_128bpp_row() {
    let _g = context(0);
    let s = drawn_expected(GL_RGBA32F, "vec4");
    let (out, ty, bpp, clamp, bytes) = row(s.targets[0].1);
    assert_eq!(
        (out, ty, bpp, clamp, bytes),
        (9, V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_BPP_128, V3D_RENDER_TARGET_CLAMP_NONE, 16)
    );
    let c = [1.5f32, -2.0, 1e30, 3.0e-5];
    assert_eq!(state::clear_words(s.targets[0].1, c), c.map(f32::to_bits));
}

/// R11F_G11F_B10F renders: format 8, 16F, 64 bpp, clamp NONE, 4 bytes
/// (`v3dx_format_table.c:146`, `:253`); it clears as halves, like RGBA16F,
/// and the TLB packs at the store.
///
/// Owners: as RG32F (src/gl then src/v3d rows, note piece 1).
#[test]
#[ignore = "note design piece 1 / order step 3: src/gl (extension, surface class) then src/v3d (rt_format row R11F_G11F_B10F = 8/16F/64/NONE/4)"]
fn r11f_g11f_b10f_renders_with_the_16f_64bpp_row() {
    let _g = context(0);
    let s = drawn_expected(GL_R11F_G11F_B10F, "vec4");
    let (out, ty, bpp, clamp, bytes) = row(s.targets[0].1);
    assert_eq!(
        (out, ty, bpp, clamp, bytes),
        (8, V3D_INTERNAL_TYPE_16F, V3D_INTERNAL_BPP_64, V3D_RENDER_TARGET_CLAMP_NONE, 4)
    );
    let c = [1.0f32, 0.5, -2.0, 0.0];
    assert_eq!(state::clear_words(s.targets[0].1, c), [0x3800_3c00, 0x0000_c000, 0, 0], "halves, not clamped");
}

/// A draw into RGBA32F + RG32F + R11F_G11F_B10F (the note's host test 2): all
/// three taken, three RTs, the common config at 128 bpp (tile 16x16 for three
/// RTs at 128 bpp, `v3d_frame.c:85`).
///
/// Owners: src/gl then src/v3d rows. The fragment program's TLB words (`f32_rt`
/// mask 0b011: `TLB_TYPE_F32_COLOR` for RT 0 and 1, `VFPACK` for RT 2) are
/// src/shaderc (`src/shaderc/online/v3d`, design piece 3) and are not visible
/// to this stand-in; the compiler's own tests own them.
#[test]
#[ignore = "note design pieces 1 and 3 / order steps 3-4: src/gl (claim, surfaces), src/v3d (rows, f32_rt mask), src/shaderc (f32 write arm)"]
fn rgba32f_rg32f_r11f_draw_is_taken_at_128bpp() {
    let _g = context(0);
    let s = drawn(&[GL_RGBA32F, GL_RG32F, GL_R11F_G11F_B10F], &["vec4", "vec4", "vec4"]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
    assert_eq!(s.targets.len(), 3);
    assert_eq!(max_bpp(&s), V3D_INTERNAL_BPP_128);
    let types: Vec<u32> = surfs(&s).iter().map(|&f| row(f).1).collect();
    assert_eq!(types, [V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_TYPE_32F, V3D_INTERNAL_TYPE_16F]);
}

/// A 32F float next to an `ivec4` and an RGBA8 target: all three classes in
/// one program (the note's host test 3).
///
/// Owners: as above; the 32F write is src/shaderc (piece 3).
#[test]
#[ignore = "note design piece 3 / order steps 3-4: src/gl (RGBA32F renderbuffer), src/shaderc (f32 write keyed by target), src/v3d (rows, f32_rt mask)"]
fn rgba32f_next_to_ivec4_and_rgba8_is_taken() {
    let _g = context(0);
    let s = drawn(&[GL_RGBA32F, GL_RGBA8I, GL_RGBA8], &["vec4", "ivec4", "vec4"]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
    assert_eq!(row(s.targets[0].1).1, V3D_INTERNAL_TYPE_32F);
    assert_eq!(s.targets[2].1, GLES_SURF_RGBA8888);
}

/// 32F targets are not blended: design piece 2 asks `state::job_state` to
/// refuse "blending into a 32-bit float colour attachment" (EXT_float_blend
/// is not claimed), and leave the 16F ones as they are.
///
/// Owners: src/gl then src/v3d (`state::job_state`, piece 2).
#[test]
#[ignore = "note design piece 2: src/gl (RGBA32F renderbuffer) then src/v3d (state::job_state refuses blending into a 32F target)"]
fn blending_into_a_32f_target_is_refused_by_name() {
    let _g = context(0);
    let fbo = framebuffer(&[GL_RGBA32F]);
    program(&["vec4"]);
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_BLEND) };
    let s = draw();
    drop(fbo);
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_BLEND) };
    assert!(s.job.is_none(), "job_state refuses");
}

/// Four draw buffers (G5, order step 2): with `DRAW_TARGETS` 4 for the V3D
/// the setup passes, `MAX_DRAW_BUFFERS` is 4, and a four-output draw reaches
/// the back end with four targets, all RGBA8, no refusal. The job's `rts` is
/// then 4 (`rt_mask` 0b1111 against four slots); `extra_targets` and the
/// compiler's `MAX_RTS` already take four.
///
/// Owner: src/backend.rs (the V3D arm of `DRAW_TARGETS` 3 -> 4, RC14); the
/// first look on the host is this test with the arm moved. Not a src/gl,
/// src/shaderc or src/v3d change.
#[test]
#[ignore = "note G5 / order step 2: src/backend.rs DRAW_TARGETS V3D arm 3 -> 4 (this test runs with the build's own value)"]
fn four_draw_buffers_are_taken() {
    let _g = context(0);
    let mut n: GLint = 0;
    // SAFETY: plain GL call, one GLint.
    unsafe { glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut n) };
    assert_eq!(n, 4);
    let s = drawn(&[GL_RGBA8; 4], &["vec4", "vec4", "vec4", "vec4"]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
    assert_eq!(s.targets.len(), 4);
    assert!(s.targets.iter().all(|t| t.0 != 0 && t.1 == GLES_SURF_RGBA8888));
}

/// The stand-in with four targets (what the arm at 4 would be): the V3D's own
/// `state` takes the four-target draw of mixed classes, so only the constant
/// is missing. Passes today; it is what lets step 2 be a one-line change.
#[test]
fn a_four_target_mixed_draw_passes_the_v3d_state_with_the_arm_at_4() {
    let _g = context(4);
    let s = drawn(&[GL_RGBA8, GL_RGBA32UI, GL_RG16I, GL_RGBA16F], &["vec4", "uvec4", "ivec4", "vec4"]);
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
    assert_eq!(s.targets.len(), 4);
    assert_eq!(max_bpp(&s), V3D_INTERNAL_BPP_128);
}
