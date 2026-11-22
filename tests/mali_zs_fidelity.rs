// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth/stencil descriptor words of fragment_ops.random-style states
//! through the GL core on the Mali mock equal Mesa 26.2.1's rules (zs-fidelity,
//! plan #373; fragrandom-diffs' Z1 and Z2).
//!
//! The Mali back end builds the Depth/stencil descriptor of a draw from GL's
//! state; Mesa builds it from st/mesa's `pipe_depth_stencil_alpha_state` and
//! `pipe_stencil_ref` (st_atom_depth.c `st_update_depth_stencil_alpha`) and
//! `panfrost_create_depth_stencil_state` / `panfrost_emit_depth_stencil`
//! (pan_cmdstream.c:4222-4297, :867-898). Two fields differed:
//!
//! - Z1: with the stencil test off st/mesa leaves the reference values 0
//!   (the `pipe_stencil_ref` is memset and filled only with the test on,
//!   st_atom_depth.c:83-84,108,119,125); the back end wrote GL's reference.
//!   `Front/Back reference value` are 0 now. With the test on and one-sided
//!   state, st/mesa copies the front face and its reference to the back
//!   (:125-126), which the back end already does (GL's back state equals the
//!   front then).
//! - Z2: with the depth function EQUAL st/mesa leaves `depth_writemask` 0
//!   (st_atom_depth.c:88-90), so `Depth write enable` is false; the back end
//!   wrote GL's mask.
//!
//! None changes a pixel (a depth EQUAL draw rewrites the depth it passed
//! on; a disabled stencil test does not read its reference). Each case below
//! draws one triangle in an 8x8 RGBA8 + DEPTH24_STENCIL8 framebuffer object
//! and reads the descriptor the draw's RUN_IDVS points at.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-mali,shaderc-online,no-shader-table --test mali_zs_fidelity
//! ```

#![cfg(all(feature = "host-test", feature = "shaderc-online", feature = "gpu-mali", not(feature = "report-es2")))]

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
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
use yakogl::mali::cs::Instr;
use yakogl::mali::genxml::v10::*;
use yakogl::osal::mali::{OsalMaliGpu, OsalMaliHeap, OsalMaliKick, OSAL_QUERY_MALI_GPU, OSAL_QUERY_MALI_TILER_HEAP};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};
use yakogl::util::ENOTSUP;

/* ---- the Mali over a mock OSAL ------------------------------------------ */

/// The OSAL's answers: the board's (panthor-frame on the T6).
const GPU: OsalMaliGpu = OsalMaliGpu::T6;
const HEAP: OsalMaliHeap =
    OsalMaliHeap { ctx_gpu: 0x8000_0001_5000, first_chunk_gpu: 0x8000_00a0_0000, chunk_size: 2 << 20, reserved: 0 };

/// Buffers made so far: each one's GPU VA is its number times 16 MiB, far
/// from any CPU address. Nothing is given back (some MiB for the run).
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

/// One frame's draws and framebuffer parameters.
#[derive(Debug)]
struct Frame {
    draws: Vec<DepthStencil>,
    #[allow(dead_code)]
    params: FramebufferParameters,
    #[allow(dead_code)]
    zs: Option<ZsTarget>,
}

/// The frames of the kicks since `from`, read as the GPU would: the stream's
/// register moves are followed, each RUN_IDVS reads the Depth/stencil
/// pointer, RUN_FRAGMENT the framebuffer descriptor and its ZS/CRC extension.
fn frames(from: usize) -> Vec<Frame> {
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
                    draws.push(DepthStencil::unpack(&mem(zsd, DepthStencil::LENGTH).try_into().unwrap()).unwrap());
                }
                Ok(Instr::RunFragment(_)) => {
                    let p = reg64(&regs, 40);
                    let fbd = p & !0x3f;
                    let params = FramebufferParameters::unpack(&mem(fbd, 64).try_into().unwrap()).unwrap();
                    let zs = (p & 2 != 0).then(|| {
                        let at = fbd + Framebuffer::LENGTH as u64;
                        ZsTarget::unpack(&mem(at, ZsTarget::LENGTH).try_into().unwrap()).unwrap()
                    });
                    out.push(Frame { draws: core::mem::take(&mut draws), params, zs });
                }
                _ => {}
            }
        }
    }
    out
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

fn renderbuffer(format: GLenum, attachment: GLenum) {
    // SAFETY: plain GL calls.
    unsafe {
        let mut rb = 0;
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, 8, 8);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, rb);
    }
}

/// The GL context, an 8x8 RGBA8 + packed depth/stencil framebuffer object
/// bound, and the program.
fn setup() {
    backend::HOST_DRAW_TARGETS.store(1, Ordering::Relaxed);
    // SAFETY: static tables and one context; this is the only test of the
    // file.
    unsafe {
        osal_set_ops(&OSAL);
        backend::g_backend = &yakogl::mali::backend::yakogl_backend_mali;
        assert_eq!(yakogl::mali::backend::mali_init(), 0);
        yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
        assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        let mut fb = 0;
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        renderbuffer(GL_RGBA8, GL_COLOR_ATTACHMENT0);
        renderbuffer(GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL_ATTACHMENT);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        let v = shader(GL_VERTEX_SHADER, "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n");
        let f = shader(
            GL_FRAGMENT_SHADER,
            "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o;\n\
             void main() { o = vec4(0.25, 0.5, 0.75, 0.6); }\n",
        );
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
        glClearColor(0.5, 0.25, 0.75, 0.4);
        glClearDepthf(1.0);
        glClearStencil(0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        while glGetError() != GL_NO_ERROR {}
    }
}
/// A triangle covering the target at depth `z`.
fn tri(z: f32) -> [f32; 12] {
    [-1.0, -1.0, z, 1.0, 3.0, -1.0, z, 1.0, -1.0, 3.0, z, 1.0]
}

/// Draw one triangle at the current state and return its Depth/stencil
/// descriptor.
fn draw_zsd() -> DepthStencil {
    let from = KICKS.lock().unwrap().len();
    let tri = tri(0.0);
    // SAFETY: `tri` outlives the draw (`glFinish`).
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, tri.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    let d: Vec<DepthStencil> = frames(from).into_iter().flat_map(|f| f.draws).collect();
    assert_eq!(d.len(), 1, "one draw, one descriptor");
    d[0]
}

#[test]
fn zsd_words_equal_mesa() {
    setup();
    // SAFETY: plain GL calls.
    unsafe {
        // Z1: stencil test off, GL references 5 and 9: Mesa writes 0 and 0.
        glDisable(GL_STENCIL_TEST);
        glStencilFuncSeparate(GL_FRONT, GL_ALWAYS, 5, 0xff);
        glStencilFuncSeparate(GL_BACK, GL_ALWAYS, 9, 0xff);
        let d = draw_zsd();
        assert!(!d.stencil_test_enable);
        assert_eq!((d.front_reference_value, d.back_reference_value), (0, 0), "Z1");

        // Test on, one-sided: both faces carry the front state and reference.
        glStencilFunc(GL_LESS, 5, 0xf0);
        glEnable(GL_STENCIL_TEST);
        let d = draw_zsd();
        assert!(d.stencil_test_enable);
        assert_eq!((d.front_reference_value, d.back_reference_value), (5, 5));
        assert_eq!((d.front_value_mask, d.back_value_mask), (0xf0, 0xf0));
        assert_eq!((d.front_compare_function, d.back_compare_function), (Func::LESS, Func::LESS));

        // Test on, two-sided: each face its own reference (:882-883).
        glStencilFuncSeparate(GL_BACK, GL_GREATER, 9, 0x0f);
        let d = draw_zsd();
        assert_eq!((d.front_reference_value, d.back_reference_value), (5, 9));
        assert_eq!(d.back_compare_function, Func::GREATER);
        glDisable(GL_STENCIL_TEST);

        // Z2: the depth write enable is GL's mask except for the function
        // EQUAL; without the depth test there is no write.
        glEnable(GL_DEPTH_TEST);
        glDepthMask(1);
        for (func, write) in [
            (GL_LESS, true),
            (GL_LEQUAL, true),
            (GL_GREATER, true),
            (GL_NOTEQUAL, true),
            (GL_ALWAYS, true),
            (GL_NEVER, true),
            (GL_EQUAL, false),
        ] {
            glDepthFunc(func);
            let d = draw_zsd();
            assert_eq!(d.depth_write_enable, write, "Z2: depth func 0x{func:x}");
        }
        glDepthMask(0);
        glDepthFunc(GL_LESS);
        assert!(!draw_zsd().depth_write_enable);
        glDepthMask(1);
        glDisable(GL_DEPTH_TEST);
        let d = draw_zsd();
        assert!(!d.depth_write_enable, "no depth test, no depth write");
        assert_eq!(d.depth_function, Func::ALWAYS);
    }
}
