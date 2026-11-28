// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64, AM62P): a 4x multisample framebuffer with four RGBA8
//! colour renderbuffers on COLOR_ATTACHMENT0-3, through the GL API, on the
//! host (plan #368, docs/notes/mrt4-v3d-pvr.md section 2,
//! PowerVR (BXS-4-64, AM62P): sample coverage (glSampleCoverage) and
//! alpha-to-coverage (GL_SAMPLE_ALPHA_TO_COVERAGE) with four 4x RGBA8 colour
//! renderbuffers on COLOR_ATTACHMENT0-3, through the GL API, on the host
//! (plan #368 item a2c4-pvr, docs/notes/mrt4-v3d-pvr.md section 2).
//!
//! The GL core is driven into a stand-in back end that records what it is
//! handed and fills no pixels. Its table says `max_samples` 4 (this file's
//! own table, not the real PowerVR's: the real one refuses a 4x draw into
//! several colour attachments, tests/es3_mrt4_msaa_pvr.rs), so four 4x
//! attachments exist and a draw reaches the back end with `samples` 4. What
//! is pinned is what the GL core hands the PowerVR back end for the coverage
//! state: ONE `sample_mask` and ONE `alpha_to_coverage` flag per draw (the
//! back end writes them into the fragment control word, `fs_meta_word` in
//! src/pvr/backend.rs, once per draw, so they apply to every target alike),
//! and the compiled fragment program's variant (`uses_discard`, the
//! sample-kill epilogue) that depends on the coverage state and not on how
//! many targets are drawn. Alpha-to-coverage takes its alpha from output 0
//! only (ES 3.0 4.1.3); the host cannot run the USC program, so that rule is
//! pinned as far as it reaches the host: the flag and variant do not change
//! with outputs 1-3 or with the alpha they write. `fs_meta_word` itself is
//! private and its own test is in src/pvr/backend_tests.rs.
//!
//! Found here: a program with more than one colour output gets no sample-kill
//! variant (the compiler refuses it), so neither coverage feature takes effect
//! on a four-output draw; `several_outputs_get_no_sample_kill_variant_today`
//! pins it and two `#[ignore]`d tests are the expected result.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_coverage_pvr
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
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `num_targets`.
    targets: usize,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::sample_mask`: one for the whole draw.
    mask: u8,
    /// `GlesDraw::alpha_to_coverage`: one for the whole draw.
    a2c: bool,
    /// The fragment program's sample-kill epilogue (`ShcShader::uses_discard`).
    kill: u8,
    /// `ShcShader::num_outputs`.
    outputs: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let (kill, outputs) = d.fs.as_ref().map_or((0, 0), |f| (f.uses_discard, f.num_outputs));
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS),
        samples: d.samples,
        mask: d.sample_mask,
        a2c: d.alpha_to_coverage,
        kill,
        outputs,
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
    name: c"es3-mrt4-coverage-pvr".as_ptr() as *const c_char,
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
    max_samples: 4,
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

/* ---- framebuffers, the program, the draw ------------------------------- */

const ATTACHMENTS: [GLenum; 4] =
    [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

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

/// The error of `glRenderbufferStorageMultisample(.., samples, GL_RGBA8, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with four RGBA8 colour renderbuffers of `samples`
/// on COLOR_ATTACHMENT0-3, all four drawn. A storage error comes back as
/// `Err(error)`.
fn framebuffer4(samples: GLsizei) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for a in ATTACHMENTS {
        let (e, rb) = storage(samples);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, a, GL_RENDERBUFFER, rb) };
    }
    // SAFETY: plain GL calls; four enums.
    unsafe {
        glDrawBuffers(4, ATTACHMENTS.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "four draw buffers under the stand-in");
    }
    Ok(fbo)
}

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

/// The current program: four float outputs `o0..o3`, colours differing per
/// output, output `i` writing alpha `alphas[i]`.
fn program4(alphas: [f32; 4]) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..4 {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for (i, a) in alphas.iter().enumerate() {
        fs += &format!("  o{i} = vec4({:?}, {:?}, 0.25, {a:?});\n", i as f32 / 4.0, 1.0 - i as f32 / 4.0);
    }
    fs += "}\n";
    link(&fs);
}

/// A program with `n` float outputs (`n` 1..=4), alpha `a` on each.
fn program_n(n: usize, a: f32) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..n {
        fs += &format!("  o{i} = vec4(0.5, 0.25, 0.75, {a:?});\n");
    }
    fs += "}\n";
    link(&fs);
}

fn link(fs: &str) {
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program links under the stand-in");
        glUseProgram(p);
    }
}

/// The 8x8 target's whole area: one oversized triangle.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw `verts` (`n` vertices of 4 floats) and hand back what the back end was
/// given for it; `None` when the core sent nothing.
fn draw(verts: &'static [f32], n: GLsizei) -> Option<Seen> {
    SEEN.lock().unwrap().clear();
    // SAFETY: `verts` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, n);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws)
}

fn drawn() -> Seen {
    draw(&FULL, 3).expect("the draw reached the back end")
}

fn set_coverage(on: bool, value: f32, invert: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(GL_SAMPLE_COVERAGE);
        } else {
            glDisable(GL_SAMPLE_COVERAGE);
        }
        glSampleCoverage(value, invert as GLboolean);
    }
}

fn set_a2c(on: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE);
        } else {
            glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
        }
    }
}

/// A bound 4x four-target framebuffer with its program, coverage state off.
fn rig(samples: GLsizei) -> Fbo {
    let fbo = framebuffer4(samples).expect("storage of the requested samples");
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    set_coverage(false, 1.0, false);
    set_a2c(false);
    fbo
}

/// `glDrawBuffers` with only the first `n` attachments drawn (the rest NONE).
fn draw_first(n: usize) {
    let mut l = [GL_NONE; 4];
    l[..n].copy_from_slice(&ATTACHMENTS[..n]);
    // SAFETY: four enums.
    unsafe { glDrawBuffers(4, l.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
}

/* ---- pins: what the GL core hands the PowerVR back end today ------------ */

/// Four 4x RGBA8 attachments are complete on a back end of `max_samples` 4,
/// and a draw into them is a four-target, four-sample draw.
#[test]
fn four_4x_attachments_draw_four_targets_at_four_samples() {
    let _g = context();
    let fbo = rig(4);
    for rb in &fbo.rbs {
        let mut v = -1;
        // SAFETY: one value; a renderbuffer this test made.
        unsafe {
            glBindRenderbuffer(GL_RENDERBUFFER, *rb);
            glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
        }
        assert_eq!(v, 4);
    }
    program4([1.0; 4]);
    let s = drawn();
    assert_eq!((s.targets, s.samples, s.mask, s.a2c), (4, 4, 0xf, false));
}

/// (2) glSampleCoverage: one mask for the draw, the first floor(value x 4)
/// samples, inverted by `invert`; every one of the four targets is in that
/// one draw, so the same samples are masked on all four. Coverage off (and
/// any value or invert then) is every sample.
#[test]
fn sample_coverage_is_one_mask_for_all_four_targets() {
    let _g = context();
    let _fbo = rig(4);
    program4([1.0; 4]);
    for (value, invert, mask) in [
        (0.5, false, 0b0011),
        (0.25, false, 0b0001),
        (0.75, false, 0b0111),
        (0.25, true, 0b1110),
        (0.5, true, 0b1100),
        (1.0, false, 0b1111),
        (1.0 / 3.0, false, 0b0001), // floor(1.333)
    ] {
        set_coverage(true, value, invert);
        let s = drawn();
        assert_eq!((s.targets, s.samples), (4, 4), "value {value} invert {invert}");
        assert_eq!(s.mask, mask, "value {value} invert {invert}");
        set_coverage(false, value, invert);
        assert_eq!(drawn().mask, 0xf, "coverage off ignores {value} {invert}");
    }
}

/// An empty coverage mask (0.0, or 1.0 inverted) writes no sample of any
/// target: the core sends no draw at all, not four targets with an empty mask.
#[test]
fn an_empty_coverage_mask_draws_nothing_to_any_target() {
    let _g = context();
    let _fbo = rig(4);
    program4([1.0; 4]);
    for (value, invert) in [(0.0, false), (1.0, true)] {
        set_coverage(true, value, invert);
        assert!(draw(&FULL, 3).is_none(), "value {value} invert {invert}");
    }
}

/// The mask does not depend on how many of the four attachments are drawn.
#[test]
fn the_sample_mask_does_not_depend_on_the_target_count() {
    let _g = context();
    let _fbo = rig(4);
    program4([1.0; 4]);
    set_coverage(true, 0.5, false);
    for n in 1..=4 {
        draw_first(n);
        let s = drawn();
        assert_eq!((s.mask, s.samples, s.targets), (0b0011, 4, n), "{n} targets");
    }
}

/// (1) Alpha-to-coverage is one flag on the draw, set for all four targets
/// together; it is the GL state alone (enable bit), whatever alpha outputs 0-3
/// write, and it does not touch the sample mask.
#[test]
fn alpha_to_coverage_is_one_flag_for_all_four_targets() {
    let _g = context();
    let _fbo = rig(4);
    set_a2c(true);
    for alphas in [[1.0; 4], [0.5, 1.0, 1.0, 1.0], [1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.5, 0.25]] {
        program4(alphas);
        let s = drawn();
        assert_eq!((s.targets, s.samples, s.a2c, s.mask), (4, 4, true, 0xf), "{alphas:?}");
    }
    set_a2c(false);
    program4([0.5; 4]);
    assert!(!drawn().a2c, "off with alpha 0.5 written");
}

/// Alpha-to-coverage and sample coverage together: both reach the one draw,
/// neither changes the other.
#[test]
fn alpha_to_coverage_with_sample_coverage_carries_both() {
    let _g = context();
    let _fbo = rig(4);
    program4([0.5, 1.0, 1.0, 1.0]);
    set_a2c(true);
    set_coverage(true, 0.5, true);
    let s = drawn();
    assert_eq!((s.targets, s.a2c, s.mask), (4, true, 0b1100));
}

/// (3) The fragment program variant follows the coverage state and not the
/// target count: a program writing output 0 only, drawn into one, two, three
/// and four draw buffers, carries the sample-kill epilogue (`uses_discard`)
/// with alpha-to-coverage or a partial mask, and not with neither or a full
/// mask; the variant is the same at every count.
#[test]
fn the_program_variant_does_not_depend_on_the_target_count() {
    let _g = context();
    let _fbo = rig(4);
    program_n(1, 0.5);
    let mut seen = Vec::new();
    for n in 1..=4 {
        draw_first(n);
        set_a2c(false);
        set_coverage(false, 1.0, false);
        let plain = drawn();
        assert_eq!((plain.kill, plain.targets), (0, n), "{n} targets, no coverage state");
        set_coverage(true, 1.0, false);
        assert_eq!(drawn().kill, 0, "{n} targets, full mask");
        set_coverage(true, 0.5, false);
        let part = drawn();
        assert_eq!((part.kill, part.mask), (1, 0b0011), "{n} targets, partial mask");
        set_coverage(false, 1.0, false);
        set_a2c(true);
        let a2c = drawn();
        assert_eq!((a2c.kill, a2c.a2c), (1, true), "{n} targets, a2c");
        assert_eq!((part.outputs, a2c.outputs, plain.outputs), (1, 1, 1));
        seen.push((plain.kill, part.kill, a2c.kill));
        set_a2c(false);
    }
    assert!(seen.windows(2).all(|w| w[0] == w[1]), "{seen:?}");
}

/// A program with several colour outputs gets NO sample-kill variant today:
/// the PowerVR compiler refuses it ("alpha-to-one in a shader with more than
/// one colour output", src/shaderc/online/powervr/chain.rs, the
/// `alpha_to_one.is_some()` arm of the multi-output branch; the kill epilogue
/// carries alpha-to-one's pair for output 0), the program's own shader is
/// drawn, and the mask and the alpha-to-coverage flag the back end writes
/// into the control word (`fs_meta_word`) are read by no epilogue: neither
/// coverage feature changes a sample of any target. It is not four-target
/// specific (two outputs fail alike); at four targets it is the whole
/// feature. Pins what is true today.
#[test]
fn several_outputs_get_no_sample_kill_variant_today() {
    let _g = context();
    let _fbo = rig(4);
    for n in 2..=4 {
        draw_first(n);
        program_n(n, 0.5);
        set_a2c(true);
        let s = drawn();
        assert_eq!((s.targets, s.a2c, s.kill), (n, true, 0), "{n} outputs, a2c");
        set_a2c(false);
        set_coverage(true, 0.5, false);
        let s = drawn();
        assert_eq!((s.targets, s.mask, s.kill), (n, 0b0011, 0), "{n} outputs, partial mask");
        set_coverage(false, 1.0, false);
    }
}

/// The expected result: alpha-to-coverage (output 0's alpha, ES 3.0 4.1.3)
/// and glSampleCoverage reach all four targets of a four-output program as
/// the sample-kill variant.
#[test]
#[ignore = "defect: the PowerVR online compiler refuses the sample-kill variant of a program with more than one colour output (\"alpha-to-one in a shader with more than one colour output\", src/shaderc/online/powervr/chain.rs), so a2c and glSampleCoverage change no sample of a multi-output draw; the kill epilogue must carry output 0's alpha-to-one pair beside the other outputs"]
fn a_four_output_program_gets_the_sample_kill_variant() {
    let _g = context();
    let _fbo = rig(4);
    program4([0.5, 1.0, 1.0, 1.0]);
    set_a2c(true);
    assert_eq!(drawn().kill, 1, "a2c");
    set_a2c(false);
    set_coverage(true, 0.5, false);
    assert_eq!(drawn().kill, 1, "partial mask");
}

/// Same, at every output count from two.
#[test]
#[ignore = "defect: as a_four_output_program_gets_the_sample_kill_variant, at 2 and 3 outputs"]
fn two_and_three_output_programs_get_the_sample_kill_variant() {
    let _g = context();
    let _fbo = rig(4);
    for n in 2..=3 {
        draw_first(n);
        program_n(n, 0.5);
        set_a2c(true);
        assert_eq!(drawn().kill, 1, "{n} outputs, a2c");
    }
}

/// A single-sample four-target framebuffer ignores both (ES 3.0 4.1.3:
/// SAMPLE_BUFFERS is not one): full mask, no flag, no epilogue.
#[test]
fn a_single_sample_four_target_framebuffer_ignores_both() {
    let _g = context();
    let _fbo = rig(0);
    program4([0.5; 4]);
    set_a2c(true);
    set_coverage(true, 0.5, false);
    let s = drawn();
    assert_eq!((s.targets, s.samples, s.mask, s.a2c, s.kill), (4, 1, 0xf, false, 0));
}

/// The two enables are plain GL state: they read back, and a framebuffer
/// switch does not change them.
#[test]
fn the_coverage_enables_read_back_with_four_targets_bound() {
    let _g = context();
    let _fbo = rig(4);
    set_a2c(true);
    set_coverage(true, 0.5, true);
    // SAFETY: plain GL calls; one value each.
    unsafe {
        assert_eq!(glIsEnabled(GL_SAMPLE_ALPHA_TO_COVERAGE), 1);
        assert_eq!(glIsEnabled(GL_SAMPLE_COVERAGE), 1);
    }
    let mut v = [0.0f32];
    // SAFETY: one float.
    unsafe { glGetFloatv(GL_SAMPLE_COVERAGE_VALUE, v.as_mut_ptr()) };
    assert_eq!(v[0], 0.5);
    assert_eq!(geti(GL_SAMPLE_COVERAGE_INVERT), 1);
    assert_eq!(geti(GL_SAMPLES), 4);
    assert_eq!(err(), GL_NO_ERROR);
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}
