// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Blending over mixed integer and float colour attachments, what the V3D
//! job state carries (t-mrt-blend), on the host.
//!
//! ES 3.0 4.1.8: blending is skipped for an integer colour buffer, and one
//! GL_BLEND enable applies to every draw buffer that is not integer. Mesa's
//! v3d (`v3dx_emit.c` BLEND_ENABLES / BLEND_CFG, `v3dx_state.c`; those two
//! files are not among the vendored sources, the lines are the ones
//! docs/notes/v3d-format-matrix.md G3 cites) programs one BLEND_CFG and a
//! BLEND_ENABLES mask with a bit for each render target that blends: so
//! {RGBA8, RGBA32UI} is mask 0b01 and {RGBA16F, RG16I, RGBA8} is 0b101.
//!
//! The GL core is driven through the API into a stand-in back end, which at
//! the draw runs the V3D's own `state::job_state` on what it was handed. The
//! job state has ONE blend flag (`V3dJobState::blend_enabled`), and
//! `driver/src/v3d/v3d_job.c` `emit_blend` turns it into BLEND_ENABLES
//! `mask = 1`: RT 0 alone. The GL core sets `GlesDraw::blend` from attachment
//! 0's format alone (`src/gl/draw.rs:3433`). So today:
//!
//! - {RGBA8, RGBA32UI}: right (RT 0 blends, the integer RT 1 does not);
//! - {RGBA16F, RG16I, RGBA8}: RT 2 should blend and does not (mask 0b001);
//! - {RGBA32UI, RGBA8}: RT 1 should blend and nothing does (`blend` false).
//!
//! The last two are pinned as they are; the expected mask is the `#[ignore]`d
//! test of each, waiting on a per-RT enable mask in `V3dJobState` and
//! `emit_blend`, and on the GL core's `blend` being per target.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt_blend_v3d
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
use yakogl::shaderc::{SHC_BLEND_ADD, SHC_BLEND_REVERSE_SUBTRACT};
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
    /// `GlesDraw::blend` as the GL core set it.
    blend: bool,
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
        blend: d.blend,
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
    name: c"es3-mrt-blend".as_ptr() as *const c_char,
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
        glDisable(GL_BLEND);
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

/// A bound 8x8 framebuffer with one renderbuffer of each sized format on
/// colour attachments 0.., all of them draw buffers.
fn framebuffer(formats: &[GLenum]) -> Fbo {
    let (mut fb, mut rbs) = (0, Vec::new());
    // SAFETY: room for one name each.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, f) in formats.iter().enumerate() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, *f, 8, 8);
            assert_eq!(err(), GL_NO_ERROR, "storage {f:#x}");
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
            rbs.push(rb);
        }
        let bufs: Vec<GLenum> = (0..formats.len()).map(|i| GL_COLOR_ATTACHMENT0 + i as GLenum).collect();
        glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{formats:#x?}");
    }
    Fbo { fb, rbs }
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

/// The current program: one `out` per entry of `types` ("vec4", "ivec4",
/// "uvec4").
fn program(types: &[&str]) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, t) in types.iter().enumerate() {
        fs += &format!("layout(location = {i}) out {t} o{i};\n");
    }
    fs += "void main() {\n";
    for (i, t) in types.iter().enumerate() {
        fs += &match *t {
            "vec4" => format!("o{i} = vec4(0.25, 0.5, 0.75, 0.5);\n"),
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

/// GL_BLEND on: colour `src * a + dst * (1 - a)`, alpha `dst - src`.
fn blend_on() {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_BLEND);
        glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_REVERSE_SUBTRACT);
        glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ONE, GL_ONE);
    }
}

/// Draw one triangle and hand back what the V3D's job state carries for it.
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

/// The render targets BLEND_ENABLES should name (Mesa: every written target
/// that is not an integer format), one bit a target.
fn expected_mask(s: &Seen) -> u32 {
    s.targets.iter().enumerate().filter(|(_, t)| t.0 != 0 && !gles_surf_is_integer(t.1)).fold(0, |m, (i, _)| m | 1 << i)
}

/// The mask the job builder writes: `emit_blend` (driver/src/v3d/v3d_job.c)
/// puts `mask = 1` when the one blend flag is set, and writes nothing else.
fn carried_mask(s: &Seen) -> u32 {
    u32::from(s.job.as_ref().expect("the V3D takes the draw").blend_enabled)
}

const F_AND_UI: [GLenum; 2] = [GL_RGBA8, GL_RGBA32UI];
const F_I_F: [GLenum; 3] = [GL_RGBA16F, GL_RG16I, GL_RGBA8];
const UI_AND_F: [GLenum; 2] = [GL_RGBA32UI, GL_RGBA8];

fn mixed(formats: &[GLenum], outs: &[&str]) -> Seen {
    let fbo = framebuffer(formats);
    program(outs);
    blend_on();
    let s = draw();
    drop(fbo);
    s
}

/* ---- {RGBA8, RGBA32UI}: right today ------------------------------------- */

/// The back end gets both targets in their formats and the V3D takes the
/// draw: no refusal, a job state.
#[test]
fn rgba8_and_rgba32ui_are_taken() {
    let _g = context();
    let s = mixed(&F_AND_UI, &["vec4", "uvec4"]);
    assert_eq!(s.refused, None);
    assert_eq!(s.targets.iter().map(|t| t.1).collect::<Vec<_>>(), [GLES_SURF_RGBA8888, GLES_SURF_RGBA32UI]);
    assert!(s.job.is_some(), "job_state accepts the blended mixed draw");
}

/// Blending on for the float RT 0 only: the GL core's `blend` (attachment
/// 0's format) is on and the job's single flag is RT 0's, which is the mask
/// Mesa has (0b01) because RT 1 is integer.
#[test]
fn rgba8_and_rgba32ui_blend_rt0_only() {
    let _g = context();
    let s = mixed(&F_AND_UI, &["vec4", "uvec4"]);
    assert!(s.blend);
    assert_eq!(expected_mask(&s), 0b01);
    assert_eq!(carried_mask(&s), expected_mask(&s));
    let j = s.job.unwrap();
    /* BLEND_CFG: the equations and factors of the GL state, as V3D codes */
    assert_eq!((j.rgb_func, j.alpha_func), (SHC_BLEND_ADD, SHC_BLEND_REVERSE_SUBTRACT));
    assert_eq!(j.rgb_src, V3D_BLEND_FACTOR_SRC_ALPHA);
    assert_eq!(j.rgb_dst, V3D_BLEND_FACTOR_INV_SRC_ALPHA);
    assert_eq!((j.alpha_src, j.alpha_dst), (V3D_BLEND_FACTOR_ONE, V3D_BLEND_FACTOR_ONE));
    assert_eq!(j.colormask, 0xf);
}

/// Two integer targets: no blending anywhere (the GL core clears `blend`).
#[test]
fn all_integer_targets_do_not_blend() {
    let _g = context();
    let s = mixed(&[GL_RGBA32UI, GL_RG16I], &["uvec4", "ivec4"]);
    assert!(!s.blend);
    assert_eq!(expected_mask(&s), 0);
    assert_eq!(carried_mask(&s), 0);
}

/// GL_BLEND off: no mask on a mixed framebuffer.
#[test]
fn blend_off_is_mask_zero() {
    let _g = context();
    let fbo = framebuffer(&F_I_F);
    program(&["vec4", "ivec4", "vec4"]);
    let s = draw();
    drop(fbo);
    assert_eq!(carried_mask(&s), 0);
}

/* ---- {RGBA16F, RG16I, RGBA8}: RT 2 is not blended ------------------------ */

/// Today's state, pinned: RT 0 blends (mask 0b001), the float RT 2 does not.
#[test]
fn rgba16f_rg16i_rgba8_blend_only_rt0_today() {
    let _g = context();
    let s = mixed(&F_I_F, &["vec4", "ivec4", "vec4"]);
    assert_eq!(s.refused, None);
    assert!(s.blend);
    assert_eq!(expected_mask(&s), 0b101);
    assert_eq!(carried_mask(&s), 0b001, "emit_blend: BLEND_ENABLES mask = 1 (v3d_job.c:621-623)");
}

/// The expected state: blending on for the two float RTs, off for the
/// integer one (Mesa BLEND_ENABLES 0b101).
#[test]
#[ignore = "V3dJobState has one blend flag and emit_blend writes BLEND_ENABLES mask = 1 (driver/src/v3d/v3d_job.c:621-623)"]
fn rgba16f_rg16i_rgba8_blend_the_float_rts() {
    let _g = context();
    let s = mixed(&F_I_F, &["vec4", "ivec4", "vec4"]);
    assert_eq!(carried_mask(&s), 0b101);
}

/* ---- {RGBA32UI, RGBA8}: nothing blends ------------------------------- */

/// Today's state, pinned: the GL core's `blend` follows attachment 0, an
/// integer one, so it is off and the float RT 1 is not blended either.
#[test]
fn rgba32ui_rgba8_blend_nothing_today() {
    let _g = context();
    let s = mixed(&UI_AND_F, &["uvec4", "vec4"]);
    assert_eq!(s.refused, None);
    assert!(!s.blend, "src/gl/draw.rs:3433 tests the format of target 0 only");
    assert_eq!(expected_mask(&s), 0b10);
    assert_eq!(carried_mask(&s), 0);
}

/// The expected state: RT 1 blends (BLEND_ENABLES 0b10).
#[test]
#[ignore = "GlesDraw::blend follows attachment 0 (src/gl/draw.rs:3433) and emit_blend enables RT 0 only (driver/src/v3d/v3d_job.c:621-623)"]
fn rgba32ui_rgba8_blend_the_float_rt() {
    let _g = context();
    let s = mixed(&UI_AND_F, &["uvec4", "vec4"]);
    assert_eq!(carried_mask(&s), 0b10);
}
