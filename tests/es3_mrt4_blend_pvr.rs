// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Blend, colour mask and integer targets with four colour attachments on the
//! PowerVR (BXS-4-64, AM62P), through the GL API, on the host (plan #368,
//! pvr-blend4-test). The V3D twin is es3_rt_masks_v3d.rs.
//!
//! `backend::HOST_DRAW_TARGETS` stands in for the knob: the GL core then
//! reports four draw buffers and hands the back end four targets. The back end
//! here is a stand-in that records the `GlesDraw` it is handed (the PowerVR's
//! own draw needs the board's job memory). On the PowerVR blending and the
//! colour mask are lowered into the fragment shader variant (`GlesDraw::fs`;
//! `blend` only documents it), so what reaches render target 3 is the one
//! `blend` / `color_mask` state of the draw, the variant (`fs.uses_fbfetch`,
//! `fs.num_outputs`) and the target formats.
//!
//! ES 3.0 has ONE blend state and ONE colour mask for every draw buffer; the
//! indexed forms (glColorMaski, glBlendFunci, glEnablei) are
//! GL_EXT_draw_buffers_indexed and the GL core does not export them, so they
//! are not covered. Blending applies to fixed-point and float buffers only; an
//! integer buffer is never blended (ES 3.0 4.1.8) but does take the mask.
//!
//! Passing tests pin today. An `#[ignore]`d test is a defect, named in its
//! reason: `GlesDraw::blend` and the blend variant key come from the FIRST
//! target's format only (`gl/draw.rs`, `current_blend_key` and `d.blend`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_blend_pvr
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
    blend: bool,
    /// src rgb, dst rgb, src alpha, dst alpha, eq rgb, eq alpha.
    func: [u8; 6],
    color_mask: u8,
    color_write: bool,
    fs_outputs: u8,
    fs_fbfetch: u8,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());

unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    let d = &*d;
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    let (fs_outputs, fs_fbfetch) = if d.fs.is_null() { (0, 0) } else { ((*d.fs).num_outputs, (*d.fs).uses_fbfetch) };
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        blend: d.blend,
        func: [
            d.blend_src_rgb,
            d.blend_dst_rgb,
            d.blend_src_alpha,
            d.blend_dst_alpha,
            d.blend_eq_rgb,
            d.blend_eq_alpha,
        ],
        color_mask: d.color_mask,
        color_write: d.color_write,
        fs_outputs,
        fs_fbfetch,
    });
    0
}

/// Every library log line (`log("%s", line)`, `osal::log_str`): a two-pointer
/// function takes it; on x86-64 and AArch64 Linux a variadic call passes those
/// two arguments as a plain one does.
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { core::ffi::CStr::from_ptr(line) }.to_string_lossy().into_owned();
    LOGS.lock().unwrap().push(s);
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

static LOGS: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn osal() -> &'static OsalOps {
    // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
    let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
    Box::leak(Box::new(osal_table(log)))
}

fn osal_table(log: LogFn) -> OsalOps {
    OsalOps {
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
        log: Some(log),
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    }
}

static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-mrt4-blend-pvr".as_ptr() as *const c_char,
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
            osal_set_ops(osal());
            backend::g_backend = &BACKEND;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ZERO);
        glBlendEquation(GL_FUNC_ADD);
        glColorMask(1, 1, 1, 1);
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

/// Draw one triangle and hand back what the back end was given for it.
fn draw() -> Result<Seen, String> {
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
    let s = core::mem::take(&mut *SEEN.lock().unwrap_or_else(|e| e.into_inner()));
    s.into_iter().rev().find(|s| s.draws).ok_or_else(|| LOGS.lock().unwrap_or_else(|e| e.into_inner()).join("\n"))
}

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

fn blend_on() {
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
        glBlendEquation(GL_FUNC_ADD);
    }
}

/// One draw into `formats` with `outs` as fragment outputs; `state` runs after
/// the framebuffer and the program are made.
/// The draw as the back end got it, or the library's log when the GL core
/// refused it (a variant it cannot have: `yakogl_draws_refused`).
fn try_run(formats: &[GLenum], outs: &[&str], state: impl FnOnce()) -> Result<Seen, String> {
    LOGS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    // SAFETY: plain GL calls.
    unsafe {
        glDisable(GL_BLEND);
        glBlendFunc(GL_ONE, GL_ZERO);
        glBlendEquation(GL_FUNC_ADD);
        glColorMask(1, 1, 1, 1);
    }
    let refused = backend::draws_refused();
    let fbo = framebuffer(formats);
    program(outs);
    state();
    let s = draw().map_err(|log| format!("{} refused: {log}", backend::draws_refused() - refused));
    drop(fbo);
    s
}

/// [`try_run`] of a draw that must reach the back end.
fn run(formats: &[GLenum], outs: &[&str], state: impl FnOnce()) -> Seen {
    try_run(formats, outs, state).unwrap_or_else(|log| panic!("the draw did not reach the back end: {log}"))
}

/// A draw the GL core refused: the variant its blend or mask key needs cannot
/// be had (`yakogl_draws_refused` went up by one), nothing reached the back end.
fn refused(r: Result<Seen, String>) -> String {
    let log = r.expect_err("the draw was refused");
    assert!(log.starts_with("1 refused"), "{log}");
    log
}

const F4: [GLenum; 4] = [GL_RGBA8, GL_RGBA8, GL_RGBA8, GL_RGBA8];
const V4: [&str; 4] = ["vec4"; 4];
const UI3: [GLenum; 4] = [GL_RGBA8, GL_RGBA8, GL_RGBA8, GL_RGBA8UI];
const UO3: [&str; 4] = ["vec4", "vec4", "vec4", "uvec4"];

/* ---- the four-target stand-in: what reaches the back end ------------------ */

#[test]
fn four_draw_buffers_reach_the_back_end() {
    let _g = context();
    let mut v = 0;
    // SAFETY: one value.
    unsafe { glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut v) };
    assert_eq!(v, 4);
    let s = run(&F4, &V4, || {});
    assert_eq!(s.targets.len(), 4);
    assert_eq!(s.fs_outputs, 4);
    assert!(!s.blend);
    assert_eq!(s.color_mask, 0xf);
    assert_eq!(s.fs_fbfetch, 0);
}

/// glColorMask(all on) and glColorMask(all off) are the draw's one mask for
/// all four targets; all off is the ISP's tag-write disable.
#[test]
fn full_and_empty_mask_reach_a_four_target_draw() {
    let _g = context();
    let s = run(&F4, &V4, || color_mask(0xf));
    assert_eq!((s.targets.len(), s.color_mask, s.color_write), (4, 0xf, true));
    let s = run(&F4, &V4, || color_mask(0));
    assert_eq!((s.targets.len(), s.color_mask, s.color_write), (4, 0, false));
}

/// RGBA8UI at attachment 3, blending off: drawn, the integer format passes
/// through to target 3, four outputs, the plain variant.
#[test]
fn uint_target_3_draws_plain() {
    let _g = context();
    let s = run(&UI3, &UO3, || {});
    assert_eq!(s.targets.len(), 4);
    assert!(gles_surf_is_integer(s.targets[3].1));
    assert!(!gles_surf_is_integer(s.targets[2].1));
    assert_eq!((s.fs_outputs, s.fs_fbfetch), (4, 0));
    assert!(!s.blend);
}

/* ---- blend and masks: refused beside more than one output, today ------------ */

/// The PowerVR's blend variant is built for ONE colour output ("a blend
/// variant of a shader with more than one colour output"), so a blended draw
/// of four float targets is refused, as is one of two. One target works and
/// carries the state. Pinned.
#[test]
fn blend_with_four_float_targets_is_refused_today() {
    let _g = context();
    refused(try_run(&F4, &V4, blend_on));
    refused(try_run(&F4[..2], &V4[..2], blend_on));
    let s = run(&F4[..1], &V4[..1], blend_on);
    assert!(s.blend);
    assert_eq!((s.fs_outputs, s.fs_fbfetch), (1, 1));
}

/// Twin of the pin above: ES 3.0 blends every float draw buffer.
#[test]
#[ignore = "defect: the PowerVR blend variant is built for one colour output only, so a blended draw of four float targets is refused"]
fn blend_reaches_four_float_targets() {
    let _g = context();
    let s = run(&F4, &V4, || {
        blend_on();
        // SAFETY: plain GL call.
        unsafe { glBlendEquationSeparate(GL_FUNC_ADD, GL_FUNC_SUBTRACT) };
    });
    assert_eq!((s.targets.len(), s.fs_outputs), (4, 4));
    assert!(s.blend);
    assert_ne!(s.fs_fbfetch, 0, "the blend variant reads the destination");
    assert_ne!(s.func[4], s.func[5], "separate equations kept");
}

/// A partial colour mask is a variant that keeps the masked channels from the
/// destination; with four outputs it is refused like the blend one. One
/// target works. Pinned.
#[test]
fn partial_mask_with_four_float_targets_is_refused_today() {
    let _g = context();
    refused(try_run(&F4, &V4, || color_mask(0b0101)));
    let s = run(&F4[..1], &V4[..1], || color_mask(0b0101));
    assert_eq!((s.color_mask, s.fs_fbfetch), (0b0101, 1));
}

/// Twin of the pin above.
#[test]
#[ignore = "defect: a partial colour mask compiles to a variant that reads the destination and the PowerVR variant builder takes one colour output only, so a masked draw of four float targets is refused"]
fn partial_mask_reaches_four_float_targets() {
    let _g = context();
    for m in [0b0101u8, 0b1000, 0b0111] {
        let s = run(&F4, &V4, || color_mask(m));
        assert_eq!((s.targets.len(), s.color_mask), (4, m));
        assert_ne!(s.fs_fbfetch, 0);
    }
}

/* ---- integer targets ------------------------------------------------------------ */

/// RGBA8UI at attachment 3 with blending enabled: ES 3.0 4.1.8 says the
/// integer buffer is not blended, the three float ones are. Today the float
/// blend variant (more than one output) is refused. Pinned.
#[test]
fn uint_target_3_with_blend_is_refused_today() {
    let _g = context();
    refused(try_run(&UI3, &UO3, blend_on));
}

/// Twin of the pin above: the float targets blend, the integer output is
/// stored as is.
#[test]
#[ignore = "defect: blending beside an integer target 3 needs the float blend variant for outputs 0-2 plus the integer output 3; the PowerVR variant builder (one colour output) refuses it"]
fn float_blend_variant_beside_integer_target_3() {
    let _g = context();
    let s = run(&UI3, &UO3, blend_on);
    assert!(s.blend);
    assert_eq!((s.fs_outputs, s.fs_fbfetch), (4, 1));
    assert!(gles_surf_is_integer(s.targets[3].1));
}

/// A mask that keeps some channels on an integer target 3 beside floats:
/// refused today. Pinned.
#[test]
fn partial_mask_with_uint_target_3_is_refused_today() {
    let _g = context();
    refused(try_run(&UI3, &UO3, || color_mask(0b0001)));
}

/// An integer target 0 turns the draw's blend flag and variant off for all
/// the targets: the GL core asks target 0 only. Pinned.
#[test]
fn integer_target_0_turns_blend_off_today() {
    let _g = context();
    let s = run(&[GL_RGBA8UI, GL_RGBA8, GL_RGBA8, GL_RGBA8], &["uvec4", "vec4", "vec4", "vec4"], blend_on);
    assert_eq!(s.targets.len(), 4);
    assert!(!s.blend);
    assert_eq!(s.fs_fbfetch, 0);
}

/// ES 3.0 4.1.8: blending applies per buffer, so float targets 1-3 blend even
/// when target 0 is an integer one.
#[test]
#[ignore = "defect: GlesDraw::blend and the blend variant key come from target 0's format only (gl/draw.rs current_blend_key, d.blend), so float targets 1-3 are not blended when target 0 is an integer target"]
fn float_targets_blend_beside_integer_target_0() {
    let _g = context();
    let s = run(&[GL_RGBA8UI, GL_RGBA8, GL_RGBA8, GL_RGBA8], &["uvec4", "vec4", "vec4", "vec4"], blend_on);
    assert!(s.blend);
    assert_ne!(s.fs_fbfetch, 0);
}
