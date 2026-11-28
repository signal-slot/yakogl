// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL_RASTERIZER_DISCARD with four colour targets, through the real PowerVR
//! back end under `HOST_DRAW_TARGETS` = 4 (item rdisc4-pvr of plan #368; RC14,
//! docs/notes/mrt4-v3d-pvr.md section 2).
//!
//! `backend::DRAW_TARGETS` is 3, so the stand-in count makes the GL core
//! accept four draw buffers. The platform is the one of
//! tests/es3_mrt4_clear_pvr.rs (host memory, the C command generators stood
//! in for, every kick kept): what the back end records is `PvrRenderKick`'s
//! `frag.num_rt` (the EOT program is selected by it), `frag.pbe_word[rt]` and
//! `frag.usc_clear_register`.
//!
//! Pinned:
//! 1. with the cap enabled a four-output draw stops in the GL core (ES 3.0
//!    3.1: primitives are discarded before rasterisation): no kick, and a
//!    clear that was pending before the cap went on is submitted as the same
//!    frame it is without the draw;
//! 2. after `glDisable` the next draw writes all four attachments, with the
//!    four-target EOT (`num_rt` 4, a PBE state for each render target);
//! 3. ES 3.0 3.1 / 4.2.3: with the cap enabled glClear and glClearBuffer*
//!    (draw buffer 3 included) have no effect and are no error; the core
//!    returns at `glClear`, which every glClearBuffer* form reaches. The
//!    argument errors of glClearBuffer* still come first. The core matches
//!    the specification on every form, so no `#[ignore]` defect pin is needed.
//!
//! Not measurable on the host: that the PBE stores render target 3 to memory
//! (the board's: docs/notes/mrt4-v3d-pvr.md section 5).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_rdiscard_pvr
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::es3::*;
use yakogl::gl::framebuffer::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::render::{PvrRenderKick, PvrRenderStatic};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OSAL_QUERY_RENDER_STATIC};
use yakogl::pvr::backend::*;
use yakogl::pvr::ffi::*;

/* ---- the platform ------------------------------------------------------------ */

const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    _mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    /// Every kick submitted, in order.
    kicks: Vec<PvrRenderKick>,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let Some(h) = HEAPS.iter().position(|&b| b == heap_base) else { return -22 };
    with_platform(|p| {
        if size == 0 || size > 1 << 30 {
            return -12;
        }
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = heap_base + 0x1000 + p.bump[h];
        p.bump[h] += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { _mem: mem });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { *mem };
    if !m.impl_.is_null() {
        with_platform(|p| p.allocs.remove(&m.gpu));
    }
    0
}

unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { *mem };
    if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
        return -22;
    }
    // SAFETY: inside the allocation (checked above).
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
    0
}

unsafe extern "C" fn p_cache(_cpu: *mut c_void, _n: usize) {}

/// The "GPU" draws nothing: the kick is kept and completes at once.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.fence += 1;
        p.kicks.push(k);
        p.fence
    })
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.fence)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    if what != OSAL_QUERY_RENDER_STATIC || out_sz != size_of::<PvrRenderStatic>() {
        return -22;
    }
    let st = PvrRenderStatic {
        eot_doutu_src0: 0x1234,
        event_pixel_pds_info: 0x55,
        eot_doutu_src0_n: [0, 0x1001, 0x1002, 0x1003, 0x1004],
        event_pixel_pds_info_n: [0, 0x61, 0x62, 0x63, 0x64],
        eot_doutu_src0_tb_n: [0, 0x2001, 0x2002, 0x2003, 0x2004],
        event_pixel_pds_info_tb_n: [0, 0x71, 0x72, 0x73, 0x74],
        eot_doutu_src0_tb4_k: [0, 0x2004, 0x3002, 0x3003],
        tile_buffer_gpu: [
            HEAP_GENERAL_BASE + 0x3E00_0000,
            HEAP_GENERAL_BASE + 0x3E00_8000,
            HEAP_GENERAL_BASE + 0x3E01_0000,
        ],
        border_gpu: HEAP_GENERAL_BASE + 0x3F00_0000,
        heap_base: HEAPS,
    };
    // SAFETY: the caller's buffer is a PvrRenderStatic (the size says so).
    unsafe { *(out as *mut PvrRenderStatic) = st };
    0
}

/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

fn osal() -> &'static OsalOps {
    // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
    let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
    Box::leak(Box::new(OsalOps {
        init: None,
        shutdown: None,
        gpu_alloc: Some(p_gpu_alloc),
        gpu_free: Some(p_gpu_free),
        mmu_map: None,
        map_phys: None,
        gpu_write: Some(p_gpu_write),
        gpu_read: None,
        cache_flush: Some(p_cache),
        cache_invd: Some(p_cache),
        submit: Some(p_submit),
        wait: Some(p_wait),
        completed: Some(p_completed),
        reg_read: None,
        reg_write: None,
        query: Some(p_query),
        log: Some(log),
        wait_vblank: None,
        present: None,
        lock: None,
        unlock: None,
        thread_id: None,
        display_busy: None,
    }))
}

/* ---- the PowerVR's command generators (driver/src/pvr, C), stood in for -------
 * Each writes words of the count the backend expects and returns it; the
 * same stand-ins the backend's unit tests link (src/pvr/backend_tests.rs). */

const W_TERM: u32 = 0xE000_0000;

#[no_mangle]
unsafe extern "C" fn pvr_ppp_build_block(
    _input: *const c_void,
    _st: *const c_void,
    out: *mut u32,
    out_count: *mut u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: the backend's buffer holds a PPP block (12 words here).
    unsafe {
        for i in 0..12 {
            *out.add(i) = 0x5050_5000 | i as u32;
        }
        *out_count = 12;
    }
    12
}

#[no_mangle]
unsafe extern "C" fn pvr_ppp_build_terminate_block(
    w: u32,
    h: u32,
    out: *mut u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: room for the terminate block.
    unsafe { core::ptr::copy_nonoverlapping([0x7E7E_0000, w, h].as_ptr(), out, 3) };
    3
}

#[no_mangle]
unsafe extern "C" fn pvr_isp_build_scissor_word(
    xmin: u32,
    xmax: u32,
    ymin: u32,
    ymax: u32,
    out: *mut u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: room for the two scissor words.
    unsafe { core::ptr::copy_nonoverlapping([(xmin << 16) | xmax, (ymin << 16) | ymax].as_ptr(), out, 2) };
    2
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_ppp_state(dw: *mut u32, addr: u64, n: u32, _why: *mut PvrVdmFieldRefusal) -> i32 {
    // SAFETY: room for the words.
    unsafe { core::ptr::copy_nonoverlapping([0xA000_0000 | n, addr as u32].as_ptr(), dw, 2) };
    2
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_pds_state(
    dw: *mut u32,
    ps: *const PvrVdmPdsState,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: the backend's state.
    let ps = unsafe { *ps };
    // SAFETY: room for the words.
    unsafe { core::ptr::copy_nonoverlapping([0xB000_0000, ps.pds_data_offset, ps.pds_code_offset].as_ptr(), dw, 3) };
    3
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_vdm_state(
    dw: *mut u32,
    vs: *const PvrVdmVsState,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: the backend's state.
    let v = unsafe { *vs };
    let w = [0xB100_0000, v.vs_pds_data_offset, v.vs_pds_code_offset, v.vs_output_size_bytes, v.vs_input_size_bytes];
    // SAFETY: room for the words.
    unsafe { core::ptr::copy_nonoverlapping(w.as_ptr(), dw, 5) };
    5
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_index_list(
    dw: *mut u32,
    topology: u32,
    addr: u64,
    count: u32,
    why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: as the instanced emitter.
    unsafe { pvr_vdm_emit_index_list_instanced(dw, topology, addr, count, 0, why) }
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_index_list_instanced(
    dw: *mut u32,
    topology: u32,
    addr: u64,
    count: u32,
    instance_count: u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    let mut w = vec![0xC000_0000 | topology];
    if addr != 0 {
        w.push(addr as u32);
    }
    w.push(count);
    if instance_count > 1 {
        w.push(instance_count - 1);
    }
    // SAFETY: room for an index list (at most four words).
    unsafe { core::ptr::copy_nonoverlapping(w.as_ptr(), dw, w.len()) };
    w.len() as i32
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_terminate(dw: *mut u32) -> u32 {
    // SAFETY: room for one word.
    unsafe { *dw = W_TERM };
    1
}

#[no_mangle]
unsafe extern "C" fn pvr_pds_render_kick_usc_program(
    fs_usc_exec_offset: u64,
    fs_temps: u32,
    _sample_rate: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderKickUscInfo,
) -> i32 {
    // SAFETY: the backend's buffers (8 data words, 1 code word) and record.
    unsafe {
        for i in 0..8 {
            *data_out.add(i) = 0x4B00_0000 | i as u32;
        }
        *data_out = fs_usc_exec_offset as u32;
        *code_out = fs_temps;
        *info = PvrPdsRenderKickUscInfo { data_size_dw: 8, code_size_dw: 1, doutu_offset: 0 };
    }
    0
}

#[no_mangle]
unsafe extern "C" fn pvr_pds_render_color_upload_program(
    color_buf_addr: u64,
    dma_dword_count: u32,
    _dest_offset: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderColorUploadInfo,
) -> i32 {
    // SAFETY: the backend's buffers and record.
    unsafe {
        core::ptr::copy_nonoverlapping([color_buf_addr as u32, dma_dword_count, 0].as_ptr(), data_out, 3);
        core::ptr::copy_nonoverlapping([0x0C0C_0000, 0].as_ptr(), code_out, 2);
        *info = PvrPdsRenderColorUploadInfo { data_size_dw: 3, code_size_dw: 2, temps_used: 1 };
    }
    0
}

#[no_mangle]
unsafe extern "C" fn pvr_pds_render_vertex_program_ids_base(
    _vs_usc_exec_offset: u64,
    _vs_temps: u32,
    streams: *const PvrPdsVertexStream,
    num_streams: u32,
    _has_slc_mcu_cache_controls: i32,
    _vertex_id_reg: u32,
    _instance_id_reg: u32,
    _instance_id_base: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderVertexInfo,
) -> i32 {
    let n = num_streams as usize;
    // SAFETY: the backend's streams, buffers (8 data words a stream, n + 2
    // code words) and record.
    unsafe {
        for i in 0..n {
            let s = *streams.add(i);
            let w = [s.address as u32, (s.address >> 32) as u32, s.stride, s.dma_size, s.dst_reg, s.per_instance, 0, 0];
            core::ptr::copy_nonoverlapping(w.as_ptr(), data_out.add(8 * i), 8);
        }
        for i in 0..n + 2 {
            *code_out.add(i) = 0x7C00_0000 | i as u32;
        }
        *info = PvrPdsRenderVertexInfo {
            data_size_dw: 8 * n as u32,
            code_size_dw: n as u32 + 2,
            temps_used: 1,
            ..Default::default()
        };
    }
    0
}

#[no_mangle]
unsafe extern "C" fn pvr_pds_render_coeff_program(
    _uses_w: i32,
    _uses_z: i32,
    _uses_pntc: i32,
    _varyings: *const c_void,
    num_varyings: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsCoeffInfo,
) -> i32 {
    let (nd, nc) = (2 * (num_varyings + 2) + 2, num_varyings + 2);
    // SAFETY: the backend's buffers and record.
    unsafe {
        for i in 0..nd as usize {
            *data_out.add(i) = 0xC0EF_0000 | i as u32;
        }
        for i in 0..nc as usize {
            *code_out.add(i) = 0xC0DE_0000 | i as u32;
        }
        *info = PvrPdsCoeffInfo { data_size_dw: nd, code_size_dw: nc, ..Default::default() };
    }
    0
}

#[no_mangle]
unsafe extern "C" fn pvr_pds_render_dma_program(
    kicks: *const PvrPdsDmaKick,
    num_kicks: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderColorUploadInfo,
) -> i32 {
    // SAFETY: the backend's kicks, buffers and record.
    unsafe {
        for i in 0..num_kicks as usize {
            let k = *kicks.add(i);
            core::ptr::copy_nonoverlapping(
                [k.address as u32, k.dwords, k.dest_offset].as_ptr(),
                data_out.add(3 * i),
                3,
            );
            *code_out.add(i) = 0xD0A0_0000 | i as u32;
        }
        *code_out.add(num_kicks as usize) = 0;
        *info = PvrPdsRenderColorUploadInfo { data_size_dw: 3 * num_kicks, code_size_dw: num_kicks + 1, temps_used: 1 };
    }
    0
}

#[no_mangle]
unsafe extern "C" fn pvr_pds_generate_pixel_event_tq(
    _doutu_src0: u64,
    emit_words: *const u32,
    _num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    // SAFETY: the backend's emit words, buffers and sizes.
    unsafe {
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the context and the four-target framebuffers --------------------------- */

const SIZE: i32 = 16;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The one context and backend of the file, with four draw targets stood in.
/// The tests take turns.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(SIZE as u32, SIZE as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    with_platform(|p| {
        p.kicks.clear();
        p.log.clear();
    });
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn error() -> GLenum {
    // SAFETY: a plain GL call on the current context.
    unsafe { glGetError() }
}

/// An FBO with four renderbuffers of `formats` at COLOR_ATTACHMENT0..3, all
/// four draw buffers enabled, bound for drawing.
fn fbo4(formats: [GLenum; 4]) -> GLuint {
    let mut fbo = 0;
    // SAFETY: plain GL calls; room for the names.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        for (i, f) in formats.iter().enumerate() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, *f, SIZE, SIZE);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
        }
        let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];
        glDrawBuffers(4, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    fbo
}

/// The kicks the back end recorded for `f`.
fn kicks_of(f: impl FnOnce()) -> Vec<PvrRenderKick> {
    // SAFETY: plain GL calls.
    unsafe { glFinish() };
    with_platform(|p| p.kicks.clear());
    f();
    // SAFETY: plain GL calls.
    unsafe { glFinish() };
    with_platform(|p| p.kicks.clone())
}

use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::vertex::*;

const C: [f32; 4] = [1.0, 0.5, 0.25, 1.0];

fn attach3(k: &PvrRenderKick) -> [u64; 2] {
    k.frag.pbe_word[3]
}

/* ---- the scene -------------------------------------------------------------- */

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// A program with four float outputs, o0-o3, one colour each; current.
fn four_output_program() {
    let vs = c"#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
    let fs = c"#version 300 es\nprecision highp float;\n\
        layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
        layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
        void main() { o0 = vec4(1.0, 0.0, 0.0, 1.0); o1 = vec4(0.0, 1.0, 0.0, 1.0);\n\
        o2 = vec4(0.0, 0.0, 1.0, 1.0); o3 = vec4(1.0, 1.0, 0.0, 1.0); }\n";
    let sh = |ty: GLenum, src: &CStr| {
        let p = src.as_ptr();
        // SAFETY: plain GL calls; `p` is NUL-terminated.
        unsafe {
            let s = glCreateShader(ty);
            glShaderSource(s, 1, &p, core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "shader compile failed: {:?}", logs());
            s
        }
    };
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, sh(GL_VERTEX_SHADER, vs));
        glAttachShader(p, sh(GL_FRAGMENT_SHADER, fs));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        glUseProgram(p);
    }
}

fn draw() {
    // SAFETY: POS is a static that outlives the draw.
    unsafe {
        glViewport(0, 0, SIZE, SIZE);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

fn set_discard(on: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        if on {
            glEnable(GL_RASTERIZER_DISCARD)
        } else {
            glDisable(GL_RASTERIZER_DISCARD)
        }
    }
}

/// The part of a kick that says which render-pass work it holds.
fn shape(k: &PvrRenderKick) -> (u32, [[u64; 2]; 4], [u32; 4]) {
    let mut pbe = [[0u64; 2]; 4];
    for (d, s) in pbe.iter_mut().zip(k.frag.pbe_word.iter()) {
        *d = *s;
    }
    (k.frag.num_rt, pbe, k.frag.usc_clear_register)
}

fn checked(label: &str, f: impl FnOnce()) -> Vec<PvrRenderKick> {
    let k = kicks_of(|| {
        f();
        assert_eq!(error(), GL_NO_ERROR, "{label}: {:?}", logs());
    });
    assert_eq!(error(), GL_NO_ERROR, "{label} at the flush: {:?}", logs());
    k
}

/// Leaves the capability off for the next test.
struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        set_discard(false);
    }
}

/* ---- (2) the control: the draw writes all four ------------------------------- */

#[test]
fn a_four_output_draw_without_discard_kicks_four_targets() {
    let _g = rig();
    fbo4([GL_RGBA8; 4]);
    four_output_program();
    let k = checked("draw", draw);
    assert_eq!(k.len(), 1, "{:?}", logs());
    assert_eq!(k[0].frag.num_rt, 4, "the four-target EOT");
    for (rt, w) in k[0].frag.pbe_word.iter().enumerate() {
        assert_ne!(*w, [0, 0], "render target {rt} has its PBE state");
    }
}

/* ---- (1) discard: nothing is drawn --------------------------------------------- */

#[test]
fn a_four_output_draw_under_discard_submits_nothing() {
    let _g = rig();
    let _r = Reset;
    fbo4([GL_RGBA8; 4]);
    four_output_program();
    set_discard(true);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glIsEnabled(GL_RASTERIZER_DISCARD) }, GL_TRUE_B);
    let k = checked("draw under discard", draw);
    assert!(k.is_empty(), "no draw reaches the back end, no render pass: {k:?}");
}

/// A clear pending when the cap goes on is the frame's work; the draw adds
/// none: the kick equals the one of the same frame without the draw.
#[test]
fn a_pending_clear_is_submitted_unchanged_when_the_draw_is_discarded() {
    let _g = rig();
    let _r = Reset;
    fbo4([GL_RGBA8; 4]);
    four_output_program();
    let clear = || {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(C[0], C[1], C[2], C[3]);
            glClear(GL_COLOR_BUFFER_BIT);
        }
    };
    let base = checked("clear alone", clear);
    assert_eq!(base.len(), 1, "{:?}", logs());
    let k = checked("clear, discard on, draw", || {
        clear();
        set_discard(true);
        draw();
    });
    assert_eq!(k.len(), 1, "one kick, the clear's: {:?}", logs());
    assert_eq!(shape(&k[0]), shape(&base[0]), "the discarded draw added no render-pass work");
    assert_eq!(k[0].frag.num_rt, 4);
}

/* ---- (2) after glDisable the next draw writes all four -------------------------- */

#[test]
fn the_draw_after_gldisable_writes_all_four() {
    let _g = rig();
    let _r = Reset;
    fbo4([GL_RGBA8; 4]);
    four_output_program();
    set_discard(true);
    assert!(checked("discarded", draw).is_empty());
    set_discard(false);
    // SAFETY: plain GL call.
    assert_eq!(unsafe { glIsEnabled(GL_RASTERIZER_DISCARD) }, GL_FALSE_B);
    let k = checked("draw", draw);
    assert_eq!(k.len(), 1, "{:?}", logs());
    assert_eq!(k[0].frag.num_rt, 4, "the four-target EOT");
    for (rt, w) in k[0].frag.pbe_word.iter().enumerate() {
        assert_ne!(*w, [0, 0], "render target {rt} has its PBE state");
    }
}

/* ---- (3) clears under discard (ES 3.0 3.1, 4.2.3) -------------------------------- */

/// ES 3.0 4.2.3: with RASTERIZER_DISCARD enabled glClear and glClearBuffer*
/// have no effect and are no error: nothing is recorded, no frame is kicked,
/// attachment 3 included. The control is the same calls with the cap off.
#[test]
fn clears_of_every_attachment_under_discard_record_nothing() {
    let _g = rig();
    let _r = Reset;
    fbo4([GL_RGBA8; 4]);
    set_discard(true);
    let k = checked("clears under discard", || {
        // SAFETY: four words each.
        unsafe {
            glClearColor(C[0], C[1], C[2], C[3]);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            for b in 0..4 {
                glClearBufferfv(GL_COLOR, b, C.as_ptr());
            }
        }
    });
    assert!(k.is_empty(), "a clear under discard has no effect: {k:?}");
    /* the integer forms, on integer attachments */
    let u = [1u32, 2, 3, 4];
    let i = [-1i32, -2, -3, -4];
    for (formats, signed) in [([GL_RGBA8UI; 4], false), ([GL_RGBA8I; 4], true)] {
        fbo4(formats);
        let k = checked("integer clears under discard", || {
            // SAFETY: four words each.
            unsafe {
                if signed {
                    glClearBufferiv(GL_COLOR, 3, i.as_ptr())
                } else {
                    glClearBufferuiv(GL_COLOR, 3, u.as_ptr())
                }
            }
        });
        assert!(k.is_empty(), "{formats:x?}: {k:?}");
    }
}

#[test]
fn glclearbufferfv_of_buffer_three_under_discard_has_no_effect_and_off_it_does() {
    let _g = rig();
    let _r = Reset;
    fbo4([GL_RGBA8; 4]);
    set_discard(true);
    let k = checked("glClearBufferfv(3) under discard", || {
        // SAFETY: four floats.
        unsafe { glClearBufferfv(GL_COLOR, 3, C.as_ptr()) }
    });
    assert!(k.is_empty(), "{k:?}");
    set_discard(false);
    let k = checked("glClearBufferfv(3)", || {
        // SAFETY: four floats.
        unsafe { glClearBufferfv(GL_COLOR, 3, C.as_ptr()) }
    });
    assert_eq!(k.len(), 1, "the control: the clear reaches render target 3: {:?}", logs());
    assert_eq!(k[0].frag.num_rt, 4);
    assert_ne!(attach3(&k[0]), [0, 0]);
}

/// The argument errors of glClearBuffer* are raised under discard too: a
/// drawbuffer of 4 is GL_INVALID_VALUE (ES 3.0 4.2.3), the cap changes
/// nothing about it.
#[test]
fn argument_errors_of_glclearbuffer_survive_discard() {
    let _g = rig();
    let _r = Reset;
    fbo4([GL_RGBA8; 4]);
    set_discard(true);
    // SAFETY: four floats.
    unsafe { glClearBufferfv(GL_COLOR, 4, C.as_ptr()) };
    assert_eq!(error(), GL_INVALID_VALUE);
}
