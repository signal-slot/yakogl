// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! ES 3.0 four draw buffers (docs/es3-cts-failures.md RC14), the GL core with
//! the REAL PowerVR back end (`yakogl_backend_pvr`) on a host platform, under
//! the four-target stand-in `backend::HOST_DRAW_TARGETS` = 4. The PowerVR
//! build's own count (`backend::DRAW_TARGETS`) is 3; the stand-in lets the GL
//! core answer 4 so the back end's half can be built against a test that
//! already holds.
//!
//! Pinned, and what must stay true when the back end draws four targets:
//!
//! - GL_MAX_DRAW_BUFFERS and GL_MAX_COLOR_ATTACHMENTS answer 4 under the
//!   stand-in and 3 without it;
//! - GL_COLOR_ATTACHMENT3 attaches, the framebuffer with four RGBA8
//!   attachments is complete, glDrawBuffers takes four entries;
//! - a glClear and a four-output draw on four RGBA8 attachments do not
//!   panic and either reach the back end or end in a defined GL error. What
//!   happens is the named assertion `FOUR_TARGET_DRAW_TODAY` (tests
//!   `four_target_clear_today` and `four_target_draw_today`). Since item
//!   pvr-backend (plan #368) the back end draws four targets under the
//!   stand-in: both kick with `num_rt` 4.
//!
//! No GPU: the platform's submit counts the kick and the generators of
//! driver/src/pvr are stood in for (as tests/depth_resolve_pvr_backend.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_gl_pvr
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use core::sync::atomic::Ordering;
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::render::{PvrRenderKick, PvrRenderStatic};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OSAL_QUERY_RENDER_STATIC};
use yakogl::pvr::backend::*;
use yakogl::pvr::ffi::*;

/* ---- the platform: heaps, a fence, no GPU ---------------------------------- */

const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

#[allow(dead_code)] // the memory is held, the back end reads it through its pointer
struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    /// Kicks submitted so far: the last fence.
    fence: i64,
    /// The last kick's `frag.num_rt`: the colour attachments it emitted.
    last_num_rt: u32,
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
        /* never-written memory reads as NaN floats and as an unmistakable depth word */
        let mut mem = vec![0xFFC0_0000_FFC0_0000u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = heap_base + 0x1000 + p.bump[h];
        p.bump[h] += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
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

/// The "GPU": a kick is counted and completes at once.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let num_rt = unsafe { (*(kick as *const PvrRenderKick)).frag.num_rt };
    with_platform(|p| {
        p.last_num_rt = num_rt;
        p.fence += 1;
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

/// The library logs every line as `log("%s", line)` (`osal::log_str`), so a
/// two-pointer function takes it; on x86-64 and AArch64 Linux a variadic call
/// passes those two arguments as a plain one does.
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

/// The platform's table (made once, at run time: the log hook is a cast).
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

/* ---- the context --------------------------------------------------------------- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// Holds the file's lock; puts the stand-in back to 0 (the build's own
/// count) when a test ends, passed or not.
struct Rig(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for Rig {
    fn drop(&mut self) {
        backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    }
}

/// The one context and back end of the file; the tests take turns. The
/// stand-in claims `targets` colour targets (0: the build's own count).
fn rig(targets: u8) -> Rig {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(8, 8);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(0);
        glFinish();
    }
    drain_errors();
    with_platform(|p| p.log.clear());
    Rig(g)
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn kicks() -> i64 {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.fence)
}

fn drain_errors() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> Result<GLint, GLenum> {
    let mut v: GLint = -777;
    // SAFETY: one GLint out.
    unsafe { glGetIntegerv(pname, &mut v) };
    match err() {
        GL_NO_ERROR => Ok(v),
        e => Err(e),
    }
}

/// A bound framebuffer with an 8x8 RGBA8 renderbuffer on each of colour
/// attachments 0..n.
fn framebuffer(n: usize) -> (GLuint, Vec<GLuint>) {
    let mut rbs = vec![0; n];
    let mut fb = 0;
    // SAFETY: room for `n` and one names.
    unsafe {
        glGenRenderbuffers(n as GLsizei, rbs.as_mut_ptr());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &rb) in rbs.iter().enumerate() {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
        }
    }
    (fb, rbs)
}

fn release(fb: GLuint, rbs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
    }
}

const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/* ---- the queries and the state --------------------------------------------------- */

/// The count is the stand-in's under it and the build's own without: four
/// since plan #325 item mrt-frameload (`DRAW_TARGETS`); a three-target
/// stand-in still answers three.
#[test]
fn the_limits_are_four_under_the_stand_in_and_without() {
    let _r = rig(4);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Ok(4));
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(4));
    backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    assert_eq!(backend::DRAW_TARGETS, 4, "the PowerVR build answers four in the ES 3.0 report");
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Ok(4));
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(4));
    backend::HOST_DRAW_TARGETS.store(3, Ordering::Relaxed);
    assert_eq!(geti(GL_MAX_DRAW_BUFFERS), Ok(3));
    assert_eq!(geti(GL_MAX_COLOR_ATTACHMENTS), Ok(3));
}

/// GL_COLOR_ATTACHMENT3 attaches, the four-attachment framebuffer is
/// complete and glDrawBuffers takes four entries.
#[test]
fn attachment_3_and_four_draw_buffers_are_accepted() {
    let _r = rig(4);
    let (fb, rbs) = framebuffer(4);
    assert_eq!(err(), GL_NO_ERROR, "GL_COLOR_ATTACHMENT0..3");
    // SAFETY: plain GL calls; four readable enums.
    unsafe {
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        let mut v: GLint = -1;
        glGetFramebufferAttachmentParameteriv(
            GL_FRAMEBUFFER,
            GL_COLOR_ATTACHMENT3,
            GL_FRAMEBUFFER_ATTACHMENT_OBJECT_NAME,
            &mut v,
        );
        assert_eq!((err(), v), (GL_NO_ERROR, rbs[3] as GLint));
        glDrawBuffers(4, ATT.as_ptr());
        assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers with four entries");
    }
    for (i, a) in ATT.iter().enumerate() {
        assert_eq!(geti(GL_DRAW_BUFFER0 + i as GLenum), Ok(*a as GLint), "GL_DRAW_BUFFER{i}");
    }
    release(fb, &rbs);

    /* a three-target stand-in has no point 3 */
    backend::HOST_DRAW_TARGETS.store(3, Ordering::Relaxed);
    let (fb, rbs) = framebuffer(3);
    // SAFETY: plain GL call.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT3, GL_RENDERBUFFER, rbs[0]) };
    assert_ne!(err(), GL_NO_ERROR, "a three-target build has no attachment point 3");
    release(fb, &rbs);
}

/* ---- today's four-target draw ------------------------------------------------------ */

/// What a clear or a four-output draw on four RGBA8 attachments does.
#[derive(Debug, PartialEq)]
enum Outcome {
    /// It reached the back end: a kick was submitted, emitting `num_rt`
    /// colour attachments.
    Kicked { num_rt: u32 },
    /// It ended in this GL error and no kick.
    GlError(GLenum),
}

/// TODAY's outcome, for the clear and for the draw.
///
/// Both reach the back end and kick with all four attachments: item
/// pvr-backend (plan #368) made the back end keep as many targets as the
/// driver's end-of-tile program moves (`pvr::backend::eot_four_rt`, four
/// under the stand-in; `PVR_BACKEND_MAX_RT` = `PVR_RENDER_MAX_RT`). Before
/// it, both kicked with `num_rt` 3: the target list was clamped to three and
/// attachment 3 dropped without a GL error.
const FOUR_TARGET_DRAW_TODAY: (Outcome, Outcome) = (Outcome::Kicked { num_rt: 4 }, Outcome::Kicked { num_rt: 4 });

const FS4: &str = "#version 300 es\nprecision mediump float;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
    layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
    void main() { o0 = vec4(1.0, 0.0, 0.0, 1.0); o1 = vec4(0.0, 1.0, 0.0, 1.0);\n\
    o2 = vec4(0.0, 0.0, 1.0, 1.0); o3 = vec4(1.0, 1.0, 0.0, 1.0); }\n";
const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = std::ffi::CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

fn use_program(fs: &str) {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the four-output program links");
        glUseProgram(p);
    }
}

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// The outcome of `f` on a four-attachment framebuffer: a kick, or the GL
/// error. Neither panics.
fn outcome_of(f: impl FnOnce()) -> Outcome {
    let (fb, rbs) = framebuffer(4);
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL calls; four readable enums.
    unsafe { glDrawBuffers(4, ATT.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR);
    let before = kicks();
    f();
    let e = err();
    let after = kicks();
    release(fb, &rbs);
    match (e, after > before) {
        (GL_NO_ERROR, true) => Outcome::Kicked { num_rt: with_platform(|p| p.last_num_rt) },
        (GL_NO_ERROR, false) => panic!("no error and no kick: the work was dropped silently: {:?}", logs()),
        (e, false) => Outcome::GlError(e),
        (e, true) => panic!("error {e:#x} and a kick: {:?}", logs()),
    }
}

fn clear_outcome() -> Outcome {
    outcome_of(|| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(0.25, 0.5, 0.75, 1.0);
            glClear(GL_COLOR_BUFFER_BIT);
        }
    })
}

fn draw_outcome() -> Outcome {
    use_program(FS4);
    outcome_of(draw_triangle)
}

#[test]
fn four_target_clear_today() {
    let _r = rig(4);
    let got = clear_outcome();
    eprintln!("four-target glClear on PowerVR today: {got:?}; log {:?}", logs());
    assert_eq!(got, FOUR_TARGET_DRAW_TODAY.0, "glClear on four RGBA8 attachments");
}

#[test]
fn four_target_draw_today() {
    let _r = rig(4);
    let got = draw_outcome();
    eprintln!("four-target draw on PowerVR today: {got:?}; log {:?}", logs());
    assert_eq!(got, FOUR_TARGET_DRAW_TODAY.1, "a four-output draw on four RGBA8 attachments");
}
