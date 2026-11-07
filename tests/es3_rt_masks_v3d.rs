// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Per-render-target colour masks in the V3D job words (t-rt-masks-v3d), on
//! the host.
//!
//! ES 3.0 has ONE glColorMask for every draw buffer (the per-buffer
//! glColorMaski is GL_EXT_draw_buffers_indexed, Draft 1 of
//! docs/notes/v3d-draw-buffers-indexed.md, not in the GL core yet). Mesa's
//! v3d (note section (c)) emits COLOR_WRITE_MASKS as one packet with a nibble
//! per RT, `~mask & 0xf` at `4 * i` (a set bit disables the channel), R and B
//! swapped for an RT in `swap_color_rb` (an XRGB8888 target). So even with one
//! GL mask the nibbles differ when the RTs differ in channels: an RT without
//! alpha gets alpha masked, an RT with alpha does not.
//!
//! The words are built in `driver/src/v3d/v3d_job.c` (C), which a host test
//! does not reach; the test goes as far as `V3dJobState`, what `emit_state`
//! reads. It has ONE `colormask` (`state::job_state`: the GL mask, `& 0x7`
//! when attachment 0 has no alpha), and `v3d_job.c:701-705` writes
//! `off | off << 4 | off << 8 | off << 12` from it. `carried_word()` models
//! that, `expected_word()` is the Mesa word. The tests that pin today pass; the
//! per-RT words are `#[ignore]`d with their owner:
//!
//! - V3D (note section (e) items 2-4): a `rt[4]` array in `struct
//!   v3d_job_state`, `job_state` filling it, `emit_state` packing it;
//! - GL core (note Draft 1): per-buffer masks, and alpha bits per target
//!   (`GlesDraw::target_alpha_bits` is attachment 0 only).
//!
//! The BLEND_ENABLES mask is es3_mrt_blend_v3d's; a partial mask on formats
//! lacking channels is es3_colormask_formats_v3d's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_rt_masks_v3d
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-v3d", not(feature = "report-es2")))]
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
#[derive(Clone)]
struct DrawCopy(GlesDraw);
// SAFETY: the pointers in the copy are read only by this thread, under LOCK.
unsafe impl Send for DrawCopy {}

#[derive(Clone)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// The draw as the GL core built it (a copy; tests may change it and re-run `job_state`).
    draw: DrawCopy,
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
        draw: DrawCopy(*d),
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
    name: c"es3-rt-masks".as_ptr() as *const c_char,
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

/* ---- the words ------------------------------------------------------------ */

/// What `emit_state` writes into COLOR_WRITE_MASKS from the one `colormask`
/// (`v3d_job.c:701-705`): RT 0's inverted mask in all four nibbles.
fn carried_word(s: &Seen) -> u16 {
    let off = !(s.job.as_ref().expect("the V3D takes the draw").colormask as u16) & 0xf;
    off | off << 4 | off << 8 | off << 12
}

/// Mesa's word for the given render targets, `(has_alpha, swap_rb)` each: per
/// RT `~mask & 0xf` at `4 * i`, alpha disabled where the RT has none, R and B
/// swapped where `swap_rb`. Nibbles of unused RTs are not compared.
fn expected_word(gl_mask: u8, rts: &[(bool, bool)]) -> u16 {
    let mut w = 0u16;
    for (i, &(has_alpha, swap_rb)) in rts.iter().enumerate() {
        let mut m = gl_mask & 0xf;
        if !has_alpha {
            m &= 0x7;
        }
        let mut off = !m & 0xf;
        if swap_rb {
            off = (off & 0b1010) | (off & 1) << 2 | (off >> 2) & 1;
        }
        w |= (off as u16) << (4 * i);
    }
    w
}

/// The low `rts` nibbles of a word.
fn low(w: u16, rts: usize) -> u16 {
    w & ((1u32 << (4 * rts)) - 1) as u16
}

/// glColorMask from a 4-bit mask (bit 0 R .. 3 A).
fn color_mask(m: u8) {
    // SAFETY: plain GL call.
    unsafe {
        glColorMask(
            (m & 1 != 0) as GLboolean,
            (m & 2 != 0) as GLboolean,
            (m & 4 != 0) as GLboolean,
            (m & 8 != 0) as GLboolean,
        )
    };
}

/// A draw into `formats` with `outs` as fragment outputs under `mask`.
fn masked(formats: &[GLenum], outs: &[&str], mask: u8) -> Seen {
    let fbo = framebuffer(formats);
    program(outs);
    color_mask(mask);
    let s = draw();
    color_mask(0xf);
    drop(fbo);
    s
}

const F_AND_UI: [GLenum; 2] = [GL_RGBA8, GL_RGBA32UI];
const F_I_F: [GLenum; 3] = [GL_RGBA16F, GL_RG16I, GL_RGBA8];
const F_UI_I_F: [GLenum; 4] = [GL_RGBA8, GL_RGBA32UI, GL_RG16I, GL_RGBA16F];

/* ---- one mask over RTs that all have the channels: right today ----------- */

/// {RGBA8, RGBA32UI} under R+B: both RTs carry the mask (ES 3.0 4.2.2 applies
/// glColorMask to integer buffers too), the job's one `colormask` is it and the
/// word has the same nibble twice, which is Mesa's.
#[test]
fn mask_reaches_float_and_integer_rt() {
    let _g = context();
    let s = masked(&F_AND_UI, &["vec4", "uvec4"], 0b0101);
    assert_eq!(s.refused, None);
    assert_eq!(s.job.as_ref().unwrap().colormask, 0b0101);
    assert_eq!(low(carried_word(&s), 2), 0xaa);
    assert_eq!(low(carried_word(&s), 2), expected_word(0b0101, &[(true, false), (true, false)]));
}

/// Three RTs, an integer one between two float: every mask gives Mesa's word.
#[test]
fn mask_reaches_three_rts_float_integer_float() {
    let _g = context();
    for m in [0b0000u8, 0b0001, 0b0110, 0b1000, 0b1111] {
        let s = masked(&F_I_F, &["vec4", "ivec4", "vec4"], m);
        assert_eq!(s.refused, None, "mask {m:#b}");
        assert_eq!(s.job.as_ref().unwrap().colormask, m as u32, "mask {m:#b}");
        assert_eq!(low(carried_word(&s), 3), expected_word(m, &[(true, false); 3]), "mask {m:#b}");
    }
}

/// Four RTs {RGBA8, RGBA32UI, RG16I, RGBA16F}: the same, four nibbles.
#[test]
fn mask_reaches_four_rts() {
    let _g = context();
    let s = masked(&F_UI_I_F, &["vec4", "uvec4", "ivec4", "vec4"], 0b1010);
    assert_eq!(s.refused, None);
    assert_eq!(s.targets.len(), 4);
    assert_eq!(carried_word(&s), 0x5555);
    assert_eq!(carried_word(&s), expected_word(0b1010, &[(true, false); 4]));
}

/// glColorMask(false x4): nothing is written, the word disables every channel
/// of every RT.
#[test]
fn mask_none_disables_every_rt() {
    let _g = context();
    let s = masked(&F_AND_UI, &["vec4", "uvec4"], 0);
    assert_eq!(s.job.as_ref().unwrap().colormask, 0);
    assert_eq!(low(carried_word(&s), 2), 0xff);
    assert_eq!(low(carried_word(&s), 2), expected_word(0, &[(true, false); 2]));
}

/// The mask is not trimmed for an integer RT 0 next to a float RT 1.
#[test]
fn integer_rt0_keeps_the_mask() {
    let _g = context();
    let s = masked(&[GL_RGBA32UI, GL_RGBA8], &["uvec4", "vec4"], 0b0011);
    assert_eq!(s.refused, None);
    assert_eq!(s.job.as_ref().unwrap().colormask, 0b0011);
    assert_eq!(low(carried_word(&s), 2), expected_word(0b0011, &[(true, false); 2]));
}

/* ---- RTs that differ in alpha: the one `colormask` is RT 0's -------------- */

/// RGB565 (no alpha) on RT 0, RGBA8 on RT 1, mask RGBA: today the GL core
/// reports 0 alpha bits for attachment 0 and the job drops A for every RT,
/// RT 1 included.
#[test]
fn alpha_less_rt0_masks_alpha_of_rt1_today() {
    let _g = context();
    let s = masked(&[GL_RGB565, GL_RGBA8], &["vec4", "vec4"], 0xf);
    assert_eq!(s.refused, None);
    assert_eq!(s.draw.0.target_alpha_bits, 0);
    assert_eq!(s.job.as_ref().unwrap().colormask, 0x7);
    assert_eq!(low(carried_word(&s), 2), 0x88);
}

/// Mesa's word: RT 0 loses alpha, RT 1 (RGBA8) keeps it (nibbles 0x8, 0x0).
#[test]
#[ignore = "V3dJobState has one colormask and v3d_job.c:701-705 repeats it per RT (note v3d-draw-buffers-indexed.md (e) items 2-4); alpha bits are attachment 0's only (GL core, Draft 1)"]
fn alpha_less_rt0_keeps_alpha_of_rt1() {
    let _g = context();
    let s = masked(&[GL_RGB565, GL_RGBA8], &["vec4", "vec4"], 0xf);
    assert_eq!(expected_word(0xf, &[(false, false), (true, false)]), 0x08);
    assert_eq!(low(carried_word(&s), 2), 0x08);
}

/// RGBA8 on RT 0, RGB565 on RT 1: the job writes alpha for both. Writing
/// alpha to an RGB565 target is a no-op (it has no alpha to store), so today's
/// word is Mesa's in effect; pinned, not ignored.
#[test]
fn alpha_less_rt1_is_unmasked_today() {
    let _g = context();
    let s = masked(&[GL_RGBA8, GL_RGB565], &["vec4", "vec4"], 0xf);
    assert_eq!(s.refused, None);
    assert_eq!(s.draw.0.target_alpha_bits, 8);
    assert_eq!(s.job.as_ref().unwrap().colormask, 0xf);
    assert_eq!(low(carried_word(&s), 2), 0x00);
}

/* ---- an XRGB8888 render target: R and B swapped in the word -------------- */

/// The back end gets an XRGB8888 target only from a window or EGLImage, never
/// from a renderbuffer, so the draw the GL core built for RGBA8 is turned into
/// one (as `state::tests::an_xrgb_target_is_drawn_without_alpha_writes` does):
/// `job_state` carries the GL mask in GL's channel order, alpha dropped.
#[test]
fn xrgb_target_job_mask_is_gl_order_today() {
    let _g = context();
    let s = masked(&[GL_RGBA8], &["vec4"], 0b0001);
    let mut d = s.draw.0;
    (d.target_format, d.target_alpha_bits) = (GLES_SURF_XRGB8888, 0);
    assert_eq!(state::refusal(&d), None);
    let j = state::job_state(&d).unwrap();
    assert_eq!(j.colormask, 0b0001, "R only, not swapped");
    /* the nibble the C emits for it is ~0001 = 1110 */
    assert_eq!(!(j.colormask as u16) & 0xf, 0b1110);
}

/// Mesa's nibble for an XRGB RT writing R only: `translate_colormask` swaps R
/// and B, so the disabled set 1110 becomes 1011.
#[test]
fn xrgb_expected_nibble_swaps_r_and_b() {
    assert_eq!(expected_word(0b0001, &[(false, true)]), 0b1011);
    assert_eq!(expected_word(0b0100, &[(false, true)]), 0b1110);
    assert_eq!(expected_word(0b0011, &[(false, false), (false, true)]), 0b1001_1100);
}

/// The swap is the job builder's (`v3d_job.c` has no per-RT word and no swap
/// today). Unclear whether the TLB's B,G,R,X load/store swap
/// (`src/v3d/backend.rs:2684`) already compensates in the mask; settle that
/// with the per-RT implementation before enabling.
#[test]
#[ignore = "emit_state has no per-RT nibble and no XRGB swap (v3d_job.c:701-705, note (e) item 3); check against the TLB swap at backend.rs:2684"]
fn xrgb_rt_word_swaps_r_and_b() {
    let _g = context();
    let s = masked(&[GL_RGBA8], &["vec4"], 0b0001);
    let mut d = s.draw.0;
    (d.target_format, d.target_alpha_bits) = (GLES_SURF_XRGB8888, 0);
    let j = state::job_state(&d).unwrap();
    let off = !(j.colormask as u16) & 0xf;
    assert_eq!(off, expected_word(0b0001, &[(false, true)]));
}
