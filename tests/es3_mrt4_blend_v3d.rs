// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Per-target blend, colour mask and integer targets on V3D render target 3
//! (mrt4-blend-v3d), on the host, with `backend::HOST_DRAW_TARGETS` = 4.
//!
//! The GL core is driven through the API into a stand-in back end that runs
//! the V3D's own `state::job_state` on what it was handed (the harness of
//! tests/es3_mrt_blend_v3d.rs). The job state carries ONE blend flag and ONE
//! colour mask; `driver/src/v3d/v3d_job.c` turns them into the packets, and
//! this file models those two emitters (`emitted_*`):
//!
//! - `emit_blend` (v3d_job.c:660-681): BLEND_ENABLES `mask = 1` when the flag
//!   is set, BLEND_CFG `render_target_mask = 0xf`;
//! - COLOR_WRITE_MASKS (v3d_job.c:746-754): `off | off << 4 | off << 8 |
//!   off << 12`, `off = ~colormask & 0xf`, RT 3 in bits 12-15.
//!
//! Mesa 26.2.1's v3d (`src/gallium/drivers/v3d/v3dx_emit.c`, not among the
//! vendored sources: the lines are those v3d_job.c and
//! docs/notes/v3d-format-matrix.md G3 cite) writes BLEND_ENABLES and
//! BLEND_CFG at :525-568, `blend_enables` one bit per render target that
//! blends (integer targets skipped), BLEND_CFG over `rt_mask` (0xf with one
//! blend state, bit 3 included), and COLOR_WRITE_MASKS at :571-585, one
//! nibble per RT, a set bit disabling a channel; a render target with no
//! colour buffer (GL_NONE) has all four bits set.
//!
//! What passes today is asserted; what the code fails is the `#[ignore]`d
//! expected test beside it, with the defect in its reason.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_mrt4_blend_v3d
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
    name: c"es3-mrt4-blend".as_ptr() as *const c_char,
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

/// Mesa's BLEND_ENABLES mask: the targets with a surface that blend (one
/// that is not an integer format), one bit a target.
fn expected_mask(s: &Seen) -> u32 {
    s.targets.iter().enumerate().filter(|(_, t)| t.0 != 0 && !gles_surf_is_integer(t.1)).fold(0, |m, (i, _)| m | 1 << i)
}

/// `emit_blend`'s BLEND_ENABLES `mask` (v3d_job.c:665-671).
fn emitted_blend_enables(s: &Seen) -> u32 {
    u32::from(s.job.as_ref().expect("the V3D takes the draw").blend_enabled)
}

/// `emit_blend`'s BLEND_CFG `render_target_mask` (v3d_job.c:672-674); None
/// when no BLEND_CFG is written.
fn emitted_blend_cfg_rt_mask(s: &Seen) -> Option<u32> {
    s.job.as_ref().expect("the V3D takes the draw").blend_enabled.then_some(0xf)
}

/// COLOR_WRITE_MASKS `mask` (v3d_job.c:749-752): RT 0's disabled-channel
/// nibble copied to all four.
fn emitted_write_masks(s: &Seen) -> u32 {
    let off = !s.job.as_ref().expect("the V3D takes the draw").colormask & 0xf;
    off | off << 4 | off << 8 | off << 12
}

/// RT `rt`'s nibble of a COLOR_WRITE_MASKS word.
fn nibble(word: u32, rt: usize) -> u32 {
    (word >> (4 * rt)) & 0xf
}

/// The draw buffers of the bound framebuffer, GL_NONE where `None`.
fn draw_buffers(bufs: &[Option<usize>]) {
    let v: Vec<GLenum> = bufs.iter().map(|b| b.map_or(GL_NONE, |i| GL_COLOR_ATTACHMENT0 + i as GLenum)).collect();
    // SAFETY: plain GL call.
    unsafe { glDrawBuffers(v.len() as GLsizei, v.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
}

const FOUR_F: [GLenum; 4] = [GL_RGBA8, GL_RGBA16F, GL_RGBA8, GL_RGB10_A2];
const THREE_F_UI: [GLenum; 4] = [GL_RGBA8, GL_RGBA16F, GL_RGBA8, GL_RGBA8UI];
const THREE_F_I: [GLenum; 4] = [GL_RGBA16F, GL_RGBA8, GL_RGB10_A2, GL_R32I];

fn four(formats: &[GLenum; 4], outs: &[&str; 4]) -> (Fbo, Seen) {
    let fbo = framebuffer(formats);
    program(outs);
    blend_on();
    let s = draw();
    (fbo, s)
}

/* ---- the four targets reach the job ------------------------------------- */

/// Four float/unorm targets: the back end gets all four and the V3D takes
/// the draw.
#[test]
fn four_targets_reach_the_job() {
    let _g = context();
    let (_f, s) = four(&FOUR_F, &["vec4"; 4]);
    assert_eq!(s.refused, None);
    assert_eq!(s.targets.len(), 4);
    assert!(s.targets.iter().all(|t| t.0 != 0));
    assert!(s.job.is_some());
}

/* ---- blend --------------------------------------------------------------- */

/// Blending over four float targets: the GL core enables it and BLEND_CFG's
/// render target mask covers RT 3 (bit 3), as Mesa's emit_rt_blend over
/// rt_mask 0xf (v3dx_emit.c:525-568).
#[test]
fn blend_cfg_mask_covers_rt3() {
    let _g = context();
    let (_f, s) = four(&FOUR_F, &["vec4"; 4]);
    assert!(s.blend);
    let m = emitted_blend_cfg_rt_mask(&s).expect("BLEND_CFG is written");
    assert_eq!(m & 0b1000, 0b1000, "BLEND_CFG render_target_mask bit 3 (v3dx_emit.c:525-568)");
    assert_eq!(m, 0xf);
}

/// Mesa's BLEND_ENABLES for four blendable targets is 0b1111
/// (v3dx_emit.c:525-568).
#[test]
#[ignore = "V3dJobState has one blend flag and emit_blend writes BLEND_ENABLES mask = 1: RTs 1-3 do not blend (driver/src/v3d/v3d_job.c:669-671)"]
fn blend_enables_cover_all_four_float_targets() {
    let _g = context();
    let (_f, s) = four(&FOUR_F, &["vec4"; 4]);
    assert_eq!(expected_mask(&s), 0b1111);
    assert_eq!(emitted_blend_enables(&s), 0b1111, "BLEND_ENABLES (v3dx_emit.c:525-568)");
}

/// Today's BLEND_ENABLES, pinned: RT 0 alone.
#[test]
fn blend_enables_are_rt0_alone_today() {
    let _g = context();
    let (_f, s) = four(&FOUR_F, &["vec4"; 4]);
    assert_eq!(emitted_blend_enables(&s), 0b0001, "v3d_job.c:669-671");
}

/// RGBA8UI on RT 3: the draw is taken, blending stays on (the GL core's
/// `blend` follows target 0) and the integer RT 3 has no bit in Mesa's mask
/// (0b0111, v3dx_emit.c:525-568); the job's mask never names RT 3.
#[test]
fn integer_rt3_is_taken_and_not_blended() {
    let _g = context();
    let (_f, s) = four(&THREE_F_UI, &["vec4", "vec4", "vec4", "uvec4"]);
    assert_eq!(s.refused, None);
    assert_eq!(s.targets[3].1, GLES_SURF_RGBA8UI);
    assert!(s.blend);
    assert_eq!(expected_mask(&s), 0b0111, "RT 3 is integer: no blend bit (v3dx_emit.c:525-568)");
    assert_eq!(emitted_blend_enables(&s) & 0b1000, 0, "RT 3 is not enabled");
}

/// Mesa blends RTs 0-2 while the integer RT 3 is skipped.
#[test]
#[ignore = "V3dJobState has one blend flag and emit_blend writes BLEND_ENABLES mask = 1: RTs 1 and 2 do not blend (driver/src/v3d/v3d_job.c:669-671)"]
fn integer_rt3_blends_rts_0_to_2() {
    let _g = context();
    let (_f, s) = four(&THREE_F_UI, &["vec4", "vec4", "vec4", "uvec4"]);
    assert_eq!(emitted_blend_enables(&s), 0b0111, "BLEND_ENABLES (v3dx_emit.c:525-568)");
}

/// R32I on RT 3 with float RTs 0-2: the blend config is the GL state's
/// (BLEND_CFG, v3dx_emit.c:525-568).
#[test]
fn r32i_rt3_keeps_the_blend_config() {
    let _g = context();
    let (_f, s) = four(&THREE_F_I, &["vec4", "vec4", "vec4", "ivec4"]);
    assert_eq!(s.refused, None);
    assert_eq!(s.targets[3].1, GLES_SURF_R32I);
    assert!(s.blend);
    assert_eq!(expected_mask(&s), 0b0111);
    let j = s.job.as_ref().unwrap();
    assert_eq!((j.rgb_func, j.alpha_func), (SHC_BLEND_ADD, SHC_BLEND_REVERSE_SUBTRACT));
    assert_eq!(j.rgb_src, V3D_BLEND_FACTOR_SRC_ALPHA);
    assert_eq!(j.rgb_dst, V3D_BLEND_FACTOR_INV_SRC_ALPHA);
    assert_eq!(emitted_blend_cfg_rt_mask(&s), Some(0xf));
}

/// R32I on RT 3: RTs 0-2 should blend (0b0111).
#[test]
#[ignore = "V3dJobState has one blend flag and emit_blend writes BLEND_ENABLES mask = 1: RTs 1 and 2 do not blend (driver/src/v3d/v3d_job.c:669-671)"]
fn r32i_rt3_blends_rts_0_to_2() {
    let _g = context();
    let (_f, s) = four(&THREE_F_I, &["vec4", "vec4", "vec4", "ivec4"]);
    assert_eq!(emitted_blend_enables(&s), 0b0111, "BLEND_ENABLES (v3dx_emit.c:525-568)");
}

/// Four integer targets: nothing blends.
#[test]
fn four_integer_targets_do_not_blend() {
    let _g = context();
    let (_f, s) = four(&[GL_RGBA8UI, GL_RGBA32UI, GL_RG16I, GL_R32I], &["uvec4", "uvec4", "ivec4", "ivec4"]);
    assert!(!s.blend);
    assert_eq!(expected_mask(&s), 0);
    assert_eq!(emitted_blend_enables(&s), 0);
    assert_eq!(emitted_blend_cfg_rt_mask(&s), None);
}

/* ---- colour mask --------------------------------------------------------- */

/// glColorMask(R, B) over four targets: the write-mask word has RT 3's
/// nibble, with the same disabled channels (Mesa's COLOR_WRITE_MASKS without
/// independent blend, v3dx_emit.c:571-585).
#[test]
fn color_mask_reaches_rt3_nibble() {
    let _g = context();
    let fbo = framebuffer(&FOUR_F);
    program(&["vec4"; 4]);
    // SAFETY: plain GL call.
    unsafe { glColorMask(1, 0, 1, 0) };
    let s = draw();
    // SAFETY: plain GL call.
    unsafe { glColorMask(1, 1, 1, 1) };
    drop(fbo);
    assert_eq!(s.job.as_ref().unwrap().colormask, 0b0101);
    let w = emitted_write_masks(&s);
    assert_eq!(nibble(w, 3), 0b1010, "RT 3: G and A disabled (v3dx_emit.c:571-585)");
    for rt in 0..4 {
        assert_eq!(nibble(w, rt), 0b1010, "RT {rt}");
    }
}

/// The default mask writes everything on all four: the word is 0.
#[test]
fn default_color_mask_enables_rt3() {
    let _g = context();
    let (_f, s) = four(&FOUR_F, &["vec4"; 4]);
    assert_eq!(emitted_write_masks(&s), 0, "COLOR_WRITE_MASKS (v3dx_emit.c:571-585)");
}

/* ---- RT 3 as GL_NONE ------------------------------------------------------ */

fn none_rt3() -> Seen {
    let fbo = framebuffer(&FOUR_F);
    draw_buffers(&[Some(0), Some(1), Some(2), None]);
    program(&["vec4"; 4]);
    let s = draw();
    drop(fbo);
    s
}

/// glDrawBuffers(COLOR0..2, NONE): the V3D takes the draw.
#[test]
fn draw_buffer_3_none_is_taken() {
    let _g = context();
    let s = none_rt3();
    assert_eq!(s.refused, None);
    assert!(s.job.is_some());
}

/// Mesa masks every channel of an RT with no colour buffer: RT 3's nibble is
/// 0xf, RT 0's stays 0 (v3dx_emit.c:571-585).
#[test]
#[ignore = "V3dJobState has one colour mask for all RTs and GL_NONE draw buffer 3 is not in it: RT 3's nibble is RT 0's (driver/src/v3d/v3d_job.c:749-752)"]
fn draw_buffer_3_none_masks_rt3_writes() {
    let _g = context();
    let s = none_rt3();
    let w = emitted_write_masks(&s);
    assert_eq!(nibble(w, 3), 0xf, "RT 3 masked (v3dx_emit.c:571-585)");
    assert_eq!(nibble(w, 0), 0, "RT 0 still written");
}
