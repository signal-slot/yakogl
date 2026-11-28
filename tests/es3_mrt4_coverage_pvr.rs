// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64, AM62P): sample coverage and alpha-to-coverage with four
//! 4x RGBA8 colour renderbuffers on COLOR_ATTACHMENT0-3, through the GL API,
//! on the host (plan #368, docs/notes/mrt4-v3d-pvr.md section 2; companion of
//! tests/es3_mrt4_msaa_pvr.rs).
//!
//! The stand-in back end here answers `max_samples` 4 (the GL core then
//! allows 4x renderbuffers and draws them with `GlesDraw::samples` 4; the
//! real PowerVR refuses a 4x draw of several colour attachments by name, see
//! tests/es3_mrt4_msaa_pvr.rs) and four draw targets
//! (`backend::HOST_DRAW_TARGETS`), and records what the GL core hands it:
//! `sample_mask`, `alpha_to_coverage`, the targets and the fragment variant
//! (`uses_discard`, `blend_key`, the USC words). It fills no pixels, so the
//! pixel results are not checked. What the PowerVR back end derives from
//! these (`fs_meta_word` in src/pvr/backend.rs: the control word's sample
//! mask field and alpha-to-coverage bit) is a pure function of the draw's
//! `samples`, `sample_mask` and `alpha_to_coverage`; it reads no target
//! count, so pinning those fields equal at one and four targets pins it.
//! Which output's alpha feeds alpha-to-coverage is the compiler's (output 0,
//! ES 3.0 4.1.3); the host sees only that the kill variant is chosen from the
//! same state whatever the other outputs' alphas are.
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

/// What the back end was handed by one draw.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::sample_mask`, `GlesDraw::alpha_to_coverage`.
    mask: u8,
    a2c: bool,
    /// The fragment variant the draw runs: `uses_discard`, `blend_key`, code.
    fs: (u32, u32, Vec<u32>),
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The ids `attachment_gone` was told, in order.
static GONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        samples: d.samples,
        mask: d.sample_mask,
        a2c: d.alpha_to_coverage,
        // SAFETY: the variant lives in the draw path's cache for this call.
        fs: unsafe { d.fs.as_ref() }
            .map_or((0, 0, Vec::new()), |f| (f.uses_discard as u32, f.blend_key as u32, f.usc_words().to_vec())),
    });
    0
}

unsafe extern "C" fn b_gone(id: u32) {
    GONE.lock().unwrap().push(id);
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
    GONE.lock().unwrap().clear();
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

/// The colour output `i` of the four-output program.
fn colour(i: usize) -> [f32; 4] {
    [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 0.5]][i]
}

/// The 8x8 target's whole area: one oversized triangle.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// Draw `verts` (`n` vertices of 4 floats) and hand back what the back end was
/// given for it.
fn draw(verts: &'static [f32], n: GLsizei) -> Seen {
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
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

/* ---- tests ----------------------------------------------------------------- */

/// A 4x framebuffer with `n` colour attachments (1..=4), `n` draw buffers.
fn framebuffer_n(n: usize) -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    for a in &ATTACHMENTS[..n] {
        let (e, rb) = storage(4);
        assert_eq!(e, GL_NO_ERROR, "4x storage under max_samples 4");
        fbo.rbs.push(rb);
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, *a, GL_RENDERBUFFER, rb) };
    }
    // SAFETY: plain GL call; `n` enums.
    unsafe { glDrawBuffers(n as GLsizei, ATTACHMENTS.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    assert_eq!(status(), GL_FRAMEBUFFER_COMPLETE);
    fbo
}

/// A program whose four outputs are `colour(i)` with output `a0` / `a3` as the
/// alphas of outputs 0 and 3 (the others keep `colour`'s); `n` outputs.
fn program_alphas(n: usize, a0: f32, a3: f32) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        fs += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    fs += "void main() {\n";
    for i in 0..n {
        let [r, g, b, mut a] = colour(i);
        if i == 0 {
            a = a0;
        }
        if i == 3 {
            a = a3;
        }
        fs += &format!("  o{i} = vec4({r:?}, {g:?}, {b:?}, {a:?});\n");
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
        assert_eq!(ok, 1, "{n} outputs link");
        glUseProgram(p);
    }
}

fn coverage_off() {
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
        glDisable(GL_SAMPLE_COVERAGE);
        glSampleCoverage(1.0, 0);
    }
}

/// The 4x four-target stand-in is a 4 sample, four target draw with a full
/// mask and no alpha-to-coverage: the baseline of everything below.
#[test]
fn four_targets_at_4x_default_to_a_full_mask_and_no_alpha_to_coverage() {
    let _g = context();
    coverage_off();
    let _fbo = framebuffer_n(4);
    program_alphas(4, 1.0, 0.5);
    let s = draw(&FULL, 3);
    assert_eq!((s.samples, s.targets.len()), (4, 4));
    assert_eq!((s.mask, s.a2c), (0xf, false));
    assert_eq!(s.fs.0, 0, "no kill epilogue without coverage state");
}

/// ES 3.0 4.1.3: alpha-to-coverage is one flag of the draw, applied to every
/// target: the same at one, two and four targets, whatever the alphas of the
/// outputs 1..3 are.
#[test]
fn alpha_to_coverage_is_one_flag_for_all_four_targets() {
    let _g = context();
    coverage_off();
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE) };
    let mut seen = Vec::new();
    for n in [1usize, 2, 4] {
        let _fbo = framebuffer_n(n);
        program_alphas(n, 0.5, 1.0);
        let s = draw(&FULL, 3);
        assert_eq!((s.samples, s.targets.len()), (4, n));
        assert!(s.a2c, "{n} targets");
        assert_eq!(s.mask, 0xf, "{n} targets: alpha to coverage leaves the mask alone");
        if n == 1 {
            assert_eq!(s.fs.0, 1, "one target: the kill variant");
        }
        seen.push(s);
    }
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE) };
}

/// The state is the draw's, not an output's: with alpha-to-coverage on, the
/// record is the same whether output 0 or output 3 carries the odd alpha.
#[test]
fn alpha_to_coverage_state_does_not_follow_the_other_outputs_alphas() {
    let _g = context();
    coverage_off();
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE) };
    let _fbo = framebuffer_n(4);
    program_alphas(4, 0.5, 1.0);
    let a = draw(&FULL, 3);
    program_alphas(4, 1.0, 0.5);
    let b = draw(&FULL, 3);
    assert_eq!((a.mask, a.a2c, a.fs.0), (b.mask, b.a2c, b.fs.0));
    assert_eq!(a.targets.len(), b.targets.len());
    // SAFETY: plain GL call.
    unsafe { glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE) };
}

/// glSampleCoverage(value, invert): the same sample mask on one, two and four
/// targets (Mesa's first `value * samples` samples, inverted on request).
#[test]
fn sample_coverage_masks_the_same_samples_on_all_four_targets() {
    let _g = context();
    coverage_off();
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_SAMPLE_COVERAGE) };
    for (value, invert, mask) in [(0.5f32, 0u8, 0x3u8), (0.5, 1, 0xc), (0.25, 0, 0x1), (0.75, 1, 0x8), (1.0, 0, 0xf)] {
        for n in [1usize, 2, 4] {
            let _fbo = framebuffer_n(n);
            // SAFETY: plain GL call.
            unsafe { glSampleCoverage(value, invert) };
            program_alphas(n, 1.0, 0.5);
            let s = draw(&FULL, 3);
            assert_eq!(s.targets.len(), n);
            assert_eq!((s.mask, s.a2c), (mask, false), "value {value} invert {invert}, {n} targets");
            if n == 1 {
                assert_eq!(
                    s.fs.0,
                    u32::from(mask != 0xf),
                    "value {value} invert {invert}: kill variant iff a sample is out"
                );
            }
        }
    }
    coverage_off();
}

/// A mask that covers no sample draws nothing at four targets as at one
/// (glSampleCoverage(0, FALSE) and (1, TRUE)).
#[test]
fn an_empty_coverage_mask_draws_nothing_on_four_targets() {
    let _g = context();
    coverage_off();
    // SAFETY: plain GL call.
    unsafe { glEnable(GL_SAMPLE_COVERAGE) };
    let _fbo = framebuffer_n(4);
    program_alphas(4, 1.0, 0.5);
    for (v, inv) in [(0.0f32, 0u8), (1.0, 1)] {
        // SAFETY: plain GL call.
        unsafe { glSampleCoverage(v, inv) };
        SEEN.lock().unwrap().clear();
        // SAFETY: `FULL` is static.
        unsafe {
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, FULL.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFinish();
        }
        assert_eq!(err(), GL_NO_ERROR);
        assert!(SEEN.lock().unwrap().iter().all(|s| !s.draws), "({v}, {inv}) draws no sample");
    }
    coverage_off();
}

/// Both on at once: the mask and the flag are carried together, and the
/// four-target record equals the one-target record.
#[test]
fn coverage_and_alpha_to_coverage_together_do_not_depend_on_the_target_count() {
    let _g = context();
    coverage_off();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SAMPLE_COVERAGE);
        glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE);
        glSampleCoverage(0.5, 0);
    }
    let rec = |n: usize| {
        let _fbo = framebuffer_n(n);
        program_alphas(n, 0.5, 1.0);
        let s = draw(&FULL, 3);
        (s.samples, s.mask, s.a2c)
    };
    let one = rec(1);
    assert_eq!(one, (4, 0x3, true));
    assert_eq!(rec(4), one);
    coverage_off();
}

/// The coverage state is not made of the target count: a single-sample
/// four-target framebuffer ignores both (ES 3.0 4.1.3: SAMPLE_BUFFERS not one).
#[test]
fn a_one_sample_four_target_draw_ignores_both() {
    let _g = context();
    coverage_off();
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_SAMPLE_COVERAGE);
        glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE);
        glSampleCoverage(0.5, 0);
    }
    let _fbo = framebuffer4(0).unwrap();
    program_alphas(4, 0.5, 1.0);
    let s = draw(&FULL, 3);
    assert_eq!((s.samples, s.targets.len(), s.mask, s.a2c, s.fs.0), (1, 4, 0xf, false, 0));
    coverage_off();
}

/* ---- defect pins ------------------------------------------------------------ */

/// The fragment variant of an alpha-to-coverage or partial-mask draw at `n`
/// targets: `uses_discard` (1 = the kill epilogue that applies the control
/// word's mask and bit is in the program).
fn kill_variant(n: usize, a2c: bool) -> u32 {
    coverage_off();
    // SAFETY: plain GL calls.
    unsafe {
        if a2c {
            glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE);
        } else {
            glEnable(GL_SAMPLE_COVERAGE);
            glSampleCoverage(0.5, 0);
        }
    }
    let _fbo = framebuffer_n(n);
    program_alphas(n, 0.5, 1.0);
    let s = draw(&FULL, 3);
    coverage_off();
    s.fs.0
}

/// With one target a 4x alpha-to-coverage draw runs the kill variant; with
/// two, three or four the GL core hands the back end the flag but the
/// fragment program is the plain one (`uses_discard` 0), so the control word's
/// alpha-to-coverage bit has no epilogue to read it. Found by this file.
#[test]
#[ignore = "defect: a multi-output fragment program gets no sample-kill variant (uses_discard 0 at 2..4 targets, 1 at one); src/gl/draw.rs draw_lod_key asks lod_sensitivity (src/shaderc/online/backend.rs sensitivity_of), which reports no sample_kill for it"]
fn alpha_to_coverage_runs_the_kill_variant_at_four_targets() {
    let _g = context();
    assert_eq!(kill_variant(1, true), 1, "one target (today)");
    for n in [2, 4] {
        assert_eq!(kill_variant(n, true), 1, "{n} targets");
    }
}

/// The same for a partial glSampleCoverage mask.
#[test]
#[ignore = "defect: as alpha_to_coverage_runs_the_kill_variant_at_four_targets: no sample-kill variant for a multi-output fragment program, so the control word's sample mask is not applied"]
fn a_partial_sample_mask_runs_the_kill_variant_at_four_targets() {
    let _g = context();
    assert_eq!(kill_variant(1, false), 1, "one target (today)");
    for n in [2, 4] {
        assert_eq!(kill_variant(n, false), 1, "{n} targets");
    }
}
