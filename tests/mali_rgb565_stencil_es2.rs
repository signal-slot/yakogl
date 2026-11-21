// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The rgb565 stencil clear through the GL core on the Mali mock in the
//! report-es2 build, the board's (es2pin-rgb565, plan #373 cause 8).
//!
//! dEQP-GLES2 `fbo.render.stencil_clear.rbo_rgb565_stencil_index8` and
//! `..._depth_component16_stencil_index8`. tests/mali_rgb565_stencil.rs pins
//! the same sequence without `report-es2`, where the GL core answers
//! `GL_FRAMEBUFFER_UNSUPPORTED` for the separate D16 + S8 pair (as Mesa's ES 3
//! context does, so Mesa has no frame for it). The board runs `report-es2`:
//! the pair is complete (see tests/mali_zs_sharing.rs) and the case draws.
//! This test replays both variants and checks, per variant:
//!
//! - `glCheckFramebufferStatus` is complete, `glGetError` is `GL_NO_ERROR`, and
//!   no draw is refused (`draws_refused` unchanged);
//! - one frame of four draws whose Depth/stencil descriptors equal the
//!   descriptors of Mesa's trace of the stencil-only case
//!   (`tools/mali/testdata/pan_zs_ref/stencil_clear__rbo_rgb565_stencil_index8.txt`);
//!   Mesa made no frame for the D16 pair, so that trace is the reference for
//!   both (the depth test is off in both);
//! - the stencil clear value, the colour write mask (red, green, blue: no
//!   alpha write, misc8-fix) and the RGBA8 storage format of the target (Mesa's
//!   is R5G6B5; see mali_rgb565_stencil.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table,report-es2 --test mali_rgb565_stencil_es2
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", feature = "report-es2"))]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::sync::Mutex;
use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::cs::Instr;
use yakogl::mali::genxml::v10::*;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

const STENCIL_CLEAR_565: &str =
    include_str!("../tools/mali/testdata/pan_zs_ref/stencil_clear__rbo_rgb565_stencil_index8.txt");

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu = OsalMaliGpu {
    gpu_id: 0xa867_0005,
    tiler_features: 0x809,
    cs_reg_count: 96,
    unpreserved_cs_reg_count: 4,
    core_features: 0,
};
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (a few MiB for the run).
static ALLOCS: AtomicU64 = AtomicU64::new(0);
static FENCES: AtomicI64 = AtomicI64::new(0);
/// Every buffer: (GPU VA, CPU address, size).
static BUFFERS: Mutex<Vec<(u64, usize, usize)>> = Mutex::new(Vec::new());
/// Every submit: the stream's GPU VA and length in bytes.
static KICKS: Mutex<Vec<(u64, usize)>> = Mutex::new(Vec::new());

unsafe extern "C" fn m_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let n = ALLOCS.fetch_add(1, Ordering::Relaxed) + 1;
    assert!(size <= 16 << 20);
    let cpu = Box::leak(vec![0u64; size.div_ceil(8)].into_boxed_slice()).as_mut_ptr().cast::<c_void>();
    let gpu = 0x40_0000_0000 + n * (16 << 20);
    BUFFERS.lock().unwrap().push((gpu, cpu as usize, size));
    *out = OsalGpuMem { cpu, gpu, phys: 0, size, impl_: n as *mut c_void };
    0
}
unsafe extern "C" fn m_free(mem: *mut OsalGpuMem) -> i32 {
    *mem = OsalGpuMem::ZERO;
    0
}
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
unsafe extern "C" fn m_init(_arg: *mut c_void) -> i32 {
    0
}
unsafe extern "C" fn m_submit(kick: *const c_void, len: usize) -> i64 {
    assert_eq!(len, core::mem::size_of::<OsalMaliKick>());
    let k = &*kick.cast::<OsalMaliKick>();
    KICKS.lock().unwrap().push((k.stream_gpu, k.stream_bytes as usize));
    FENCES.fetch_add(1, Ordering::Relaxed) + 1
}
unsafe extern "C" fn m_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}
unsafe extern "C" fn m_completed() -> i64 {
    FENCES.load(Ordering::Relaxed)
}
unsafe extern "C" fn m_query(what: u32, out: *mut c_void, sz: usize) -> i32 {
    match what {
        OSAL_QUERY_MALI_GPU if sz == core::mem::size_of::<OsalMaliGpu>() => *out.cast::<OsalMaliGpu>() = GPU,
        OSAL_QUERY_MALI_TILER_HEAP if sz == core::mem::size_of::<OsalMaliHeap>() => *out.cast::<OsalMaliHeap>() = HEAP,
        _ => return -ENOTSUP,
    }
    0
}

/// The OSAL's log hook: the library only calls it as `log("%s", line)`, so a
/// two-pointer function takes its place (the same calling convention for
/// these arguments).
unsafe extern "C" fn m_log(_fmt: *const c_char, line: *const c_char) {
    eprintln!("osal log: {}", core::ffi::CStr::from_ptr(line).to_string_lossy());
}

static OSAL: OsalOps = OsalOps {
    init: Some(m_init),
    shutdown: None,
    gpu_alloc: Some(m_alloc),
    gpu_free: Some(m_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: Some(m_submit),
    wait: Some(m_wait),
    completed: Some(m_completed),
    reg_read: None,
    reg_write: None,
    query: Some(m_query),
    // SAFETY: see `m_log`.
    log: Some(unsafe {
        core::mem::transmute::<
            unsafe extern "C" fn(*const c_char, *const c_char),
            unsafe extern "C" fn(*const c_char, ...),
        >(m_log)
    }),
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/// `len` bytes of the buffer holding GPU address `gpu`.
fn mem(gpu: u64, len: usize) -> Vec<u8> {
    let b = BUFFERS.lock().unwrap();
    let &(base, cpu, _) = b
        .iter()
        .find(|&&(base, _, size)| gpu >= base && gpu + len as u64 <= base + size as u64)
        .unwrap_or_else(|| panic!("no buffer holds 0x{gpu:x}+{len}"));
    // SAFETY: the buffer is leaked, `size` bytes long, and the range is inside it.
    unsafe { core::slice::from_raw_parts((cpu + (gpu - base) as usize) as *const u8, len).to_vec() }
}

/* ---- what the back end built --------------------------------------------- */

#[derive(Debug)]
struct DrawOut {
    zsd: DepthStencil,
    /// The Blend's equation colour mask.
    colour_mask: u32,
}

#[derive(Debug)]
struct FrameOut {
    draws: Vec<DrawOut>,
    s_clear: u32,
    rt: RgbRenderTarget,
}

/// The frames of the kicks since `from`, read as the GPU would: the stream's
/// register moves are followed, each RUN_IDVS reads the Depth/stencil and the
/// Blend pointers, and RUN_FRAGMENT the framebuffer descriptor.
fn frames(from: usize) -> Vec<FrameOut> {
    let kicks = KICKS.lock().unwrap()[from..].to_vec();
    let mut out = Vec::new();
    for (gpu, bytes) in kicks {
        let raw = mem(gpu, bytes);
        let mut regs = [0u32; 128];
        let reg64 = |regs: &[u32; 128], r: usize| u64::from(regs[r]) | u64::from(regs[r + 1]) << 32;
        let mut draws = Vec::new();
        for w in raw.as_chunks::<8>().0 {
            match Instr::decode(u64::from_le_bytes(*w)) {
                Ok(Instr::Move48(m)) => {
                    regs[m.destination as usize] = m.immediate as u32;
                    regs[m.destination as usize + 1] = (m.immediate >> 32) as u32;
                }
                Ok(Instr::Move32(m)) => regs[m.destination as usize] = m.immediate,
                Ok(Instr::RunIdvs(_)) => {
                    let zsd = reg64(&regs, 52);
                    let blend = reg64(&regs, 50) & !0xf;
                    let zsd = DepthStencil::unpack(&mem(zsd, DepthStencil::LENGTH).try_into().unwrap()).unwrap();
                    /* the Blend's `Equation` is bits 32..=63 */
                    let eq = BlendEquation::unpack(&mem(blend + 4, 4).try_into().unwrap()).unwrap();
                    draws.push(DrawOut { zsd, colour_mask: eq.color_mask });
                }
                Ok(Instr::RunFragment(_)) => {
                    let p = reg64(&regs, 40);
                    let fbd = p & !0x3f;
                    let params = FramebufferParameters::unpack(&mem(fbd, 64).try_into().unwrap()).unwrap();
                    /* the framebuffer, its ZS/CRC extension, the render target */
                    let rt_at = fbd + Framebuffer::LENGTH as u64 + if p & 2 != 0 { 64 } else { 0 };
                    let rt = RgbRenderTarget::unpack_as(
                        &mem(rt_at, RgbRenderTarget::LENGTH).try_into().unwrap(),
                        &["Writeback Format"],
                    )
                    .unwrap();
                    out.push(FrameOut { draws: core::mem::take(&mut draws), s_clear: params.s_clear, rt });
                }
                _ => {}
            }
        }
    }
    out
}

/* ---- Mesa's trace --------------------------------------------------------- */

/// The value of `key` in the trace's block `block`.
fn ref_field<'a>(trace: &'a str, block: &str, key: &str) -> &'a str {
    let mut inside = false;
    for line in trace.lines() {
        if let Some(h) = line.strip_prefix("## ") {
            inside = h.starts_with(&format!("{block} lines "));
            continue;
        }
        if let Some(v) = line.trim().strip_prefix(key).and_then(|r| r.strip_prefix(": ")).filter(|_| inside) {
            return v;
        }
    }
    panic!("{block}: no field {key}")
}

/// The Depth/stencil descriptor pandecode printed in `block`.
fn ref_depth_stencil(trace: &str, block: &str) -> DepthStencil {
    let f = |k: &str| ref_field(trace, block, k);
    let func = |k: &str| match f(k) {
        "Never" => Func::NEVER,
        "Equal" => Func::EQUAL,
        "Always" => Func::ALWAYS,
        v => panic!("{block}: {k} {v}"),
    };
    let op = |k: &str| match f(k) {
        "Keep" => StencilOp::KEEP,
        "Replace" => StencilOp::REPLACE,
        v => panic!("{block}: {k} {v}"),
    };
    let hex = |k: &str| u32::from_str_radix(f(k).trim_start_matches("0x"), 16).unwrap();
    let flag = |k: &str| f(k) == "true";
    assert_eq!(
        (f("Depth clamp mode"), f("Depth source"), f("Depth units"), f("Depth factor"), f("Depth bias clamp")),
        ("[0, 1]", "Fixed function", "0.000000", "0.000000", "0.000000"),
        "{block}"
    );
    let depth_func = func("Depth function");
    DepthStencil {
        front_compare_function: func("Front compare function"),
        front_stencil_fail: op("Front stencil fail"),
        front_depth_fail: op("Front depth fail"),
        front_depth_pass: op("Front depth pass"),
        back_compare_function: func("Back compare function"),
        back_stencil_fail: op("Back stencil fail"),
        back_depth_fail: op("Back depth fail"),
        back_depth_pass: op("Back depth pass"),
        stencil_from_shader: flag("Stencil from shader"),
        stencil_test_enable: flag("Stencil test enable"),
        front_write_mask: hex("Front write mask"),
        back_write_mask: hex("Back write mask"),
        front_value_mask: hex("Front value mask"),
        back_value_mask: hex("Back value mask"),
        front_reference_value: hex("Front reference value"),
        back_reference_value: hex("Back reference value"),
        shader_read_only_z_s: flag("Shader read only Z/S"),
        depth_cull_enable: flag("Depth cull enable"),
        depth_write_enable: flag("Depth write enable"),
        depth_bias_enable: flag("Depth bias enable"),
        depth_function: depth_func,
        ..DepthStencil::default()
    }
}

/* ---- the case -------------------------------------------------------------- */

const VS: &str = "attribute vec4 a_position;\nattribute vec2 a_coord;\nvarying vec2 v_coord;\n\
                  void main() { gl_Position = a_position; v_coord = a_coord; }\n";
const FS: &str = "precision mediump float;\nvarying vec2 v_coord;\nuniform sampler2D u_sampler0;\n\
                  void main() { gl_FragColor = texture2D(u_sampler0, v_coord); }\n";

/// The quad of dEQP's `drawQuad`: (-1,-1) to (1,1), z 0.
static QUAD: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
static COORD: [f32; 8] = [0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
static INDICES: [u16; 6] = [0, 1, 2, 2, 1, 3];

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
        assert_eq!(ok, 1, "the shader does not compile");
        s
    }
}

fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(128, 128).is_null());
    }
}

/// Replay one case; the frames it made, and the status `glCheckFramebufferStatus` gave.
fn replay(colour: GLenum, depth16: bool) -> (Vec<FrameOut>, GLenum) {
    let kicks_before = KICKS.lock().unwrap().len();
    let refused_before = backend::draws_refused();
    // SAFETY: plain GL calls; the vertex data is static and outlives the draws.
    let status = unsafe {
        let mut tex = [0; 2];
        let pixels = vec![0x80u8; 128 * 128 * 4];
        glGenTextures(2, tex.as_mut_ptr());
        for t in tex {
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA as GLint,
                128,
                128,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                pixels.as_ptr().cast(),
            );
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        }
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_coord".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program does not link");

        /* the framebuffer object: colour, then (depth,) stencil */
        let (mut fb, mut rb) = (0, [0; 3]);
        glGenFramebuffers(1, &mut fb);
        glGenRenderbuffers(3, rb.as_mut_ptr());
        let attach = |i: usize, fmt: GLenum, point: GLenum| {
            glBindRenderbuffer(GL_RENDERBUFFER, rb[i]);
            glRenderbufferStorage(GL_RENDERBUFFER, fmt, 128, 128);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb[i]);
        };
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if colour == GL_RGBA8 {
            /* dEQP's tex2d_rgba: the colour attachment is a texture */
            let mut t = 0;
            glGenTextures(1, &mut t);
            glBindTexture(GL_TEXTURE_2D, t);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 128, 128, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, t, 0);
        } else {
            attach(0, colour, GL_COLOR_ATTACHMENT0);
        }
        if depth16 {
            attach(1, GL_DEPTH_COMPONENT16, GL_DEPTH_ATTACHMENT);
        }
        attach(2, GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT);
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if status != GL_FRAMEBUFFER_COMPLETE {
            /* dEQP's checkCompleteness: NotSupported, nothing is drawn */
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            return (frames(kicks_before), status);
        }

        glViewport(0, 0, 128, 128);
        /* a fresh context's clear values: an earlier case left its own */
        glClearStencil(0);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_SCISSOR_TEST);
        glScissor(10, 16, 32, 120);
        glClearStencil(1);
        glClear(GL_STENCIL_BUFFER_BIT);
        glScissor(16, 32, 100, 64);
        glClearStencil(2);
        glClear(GL_STENCIL_BUFFER_BIT);
        glDisable(GL_SCISSOR_TEST);

        glUseProgram(p);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, QUAD.as_ptr().cast::<c_void>());
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, COORD.as_ptr().cast::<c_void>());
        glEnableVertexAttribArray(1);
        let loc = glGetUniformLocation(p, c"u_sampler0".as_ptr());
        for (unit, t) in tex.iter().enumerate() {
            glActiveTexture(GL_TEXTURE0 + unit as GLenum);
            glBindTexture(GL_TEXTURE_2D, *t);
        }
        glEnable(GL_STENCIL_TEST);
        for (reference, unit) in [(1, 0), (2, 1)] {
            glStencilFunc(GL_EQUAL, reference, !0);
            glUniform1i(loc, unit);
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, INDICES.as_ptr().cast::<c_void>());
        }
        glDisable(GL_STENCIL_TEST);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "a GL error in the case");
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        status
    };
    assert_eq!(backend::draws_refused(), refused_before, "a draw was refused");
    (frames(kicks_before), status)
}

/// One variant against Mesa's trace.
fn check(name: &str, depth16: bool) {
    let mesa = STENCIL_CLEAR_565;
    let (frames, status) = replay(GL_RGB565, depth16);
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{name}: glCheckFramebufferStatus");
    assert_eq!(frames.len(), 1, "{name}: frames");
    let f = &frames[0];
    assert_eq!(f.draws.len(), 4, "{name}: the two scissored stencil clears and the two quads");
    for (i, d) in f.draws.iter().enumerate() {
        let block = format!("frame1-draw{}", i + 1);
        assert_eq!(d.zsd, ref_depth_stencil(mesa, &block), "{name} {block}: the Depth/stencil descriptor");
    }
    for d in &f.draws[2..] {
        assert_eq!(d.colour_mask, 0x7, "{name}: colour write mask of a quad");
    }
    for d in &f.draws[..2] {
        assert_eq!(d.colour_mask, 0, "{name}: colour write mask of a stencil clear");
    }
    assert_eq!(f.s_clear, ref_field(mesa, "frame1-fbd", "S Clear").parse::<u32>().unwrap(), "{name}: stencil clear");
    assert_eq!(f.rt.internal_format, ColorBufferInternalFormat::R8G8B8A8, "{name}: internal format");
    assert_eq!(f.rt.writeback_format, ColorFormat::R8G8B8A8, "{name}: writeback format");
    assert_eq!(f.rt.swizzle, 1672, "{name}: swizzle RGBA");
    assert_eq!(ref_field(mesa, "frame1-fbd", "Internal Format"), "R5G6B5A0");
    assert_eq!(ref_field(mesa, "frame1-fbd", "Writeback Format"), "R5G6B5");
}

#[test]
fn the_rgb565_stencil_clear_cases_equal_mesa_s() {
    setup();
    check("rbo_rgb565_stencil_index8", false);
    check("rbo_rgb565_depth_component16_stencil_index8", true);
}
