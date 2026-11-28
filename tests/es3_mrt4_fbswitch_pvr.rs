// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Switching between a four-target FBO, a one-target FBO and the default
//! framebuffer on the PowerVR host back end (plan #368 item `fbswitch4-pvr`;
//! RC14, docs/notes/mrt4-v3d-pvr.md), with the draw-target count at 4.
//!
//! The rig is tests/es3_mrt4_pvr.rs's: the real GL core and PowerVR back end,
//! the render kicks and the end-of-tile (EOT) data generations recorded, so
//! each render pass's render-target count (`frag.num_rt`), PBE words
//! (`frag.pbe_word`) and EOT pairs are visible. The count is set by
//! `set_draw_targets`, the file's ONE write of the stand-in
//! `backend::HOST_DRAW_TARGETS`; the `knob` item replaces its body with
//! `pvr_debug_set_draw_targets(n)`. Not measurable on the host: what the PBE
//! stores.
//!
//! Pinned, with FBO A (four RGBA8 renderbuffers), FBO B (one) and the default
//! framebuffer, A, B, default, A within one frame (no glFinish between) and
//! across frames (glFinish between):
//! * (1) each render pass carries exactly its own framebuffer's target count
//!   and PBE words: A's passes four non-zero words and four EOT emits, B's
//!   one word, none for A's attachment 3, and one EOT emit; the default
//!   framebuffer's pass one target; A's second pass is as its first, four
//!   EOT pairs, with A's own surfaces;
//! * (2) a `glClearBufferfv` of A's attachment 3 followed by a switch to B
//!   and a draw: B's pass emits nothing for A's attachment-3 surface;
//! * (3) the draw-buffer list {NONE, NONE, NONE, COLOR_ATTACHMENT3} set on A
//!   survives binding B and re-binding A (per-FBO state), and A's draw
//!   after it stores attachment 3 alone.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_mrt4_fbswitch_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", feature = "test-knobs"))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::*;
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

/* ---- the platform ------------------------------------------------------------ */

const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

#[allow(dead_code)] // `mem` only keeps the host memory alive
struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
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
 * same stand-ins as tests/es3_mrt4_clear_pvr.rs, two of them recording. */

/// Every end-of-tile data generation: the pairs asked for and the emit words.
static EOT: Mutex<Vec<(u32, Vec<u32>)>> = Mutex::new(Vec::new());
/// The USC-heap offset of every fragment program a kick-USC PDS program was
/// built for, in order.
static KICK_FS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

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
    KICK_FS.lock().unwrap().push(fs_usc_exec_offset);
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

/// The generator's real data layout is the driver's (pinned by
/// tests/pvr_render_rt4.rs and tests/pvr_eot_words_pin.rs); this one keeps
/// what the back end asked for: `num_rt` pairs of emit words.
#[no_mangle]
unsafe extern "C" fn pvr_pds_generate_pixel_event_tq(
    _doutu_src0: u64,
    emit_words: *const u32,
    num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    // SAFETY: the backend's emit words (2 * num_rt), buffers and sizes.
    unsafe {
        let emit = core::slice::from_raw_parts(emit_words, 2 * num_rt as usize).to_vec();
        EOT.lock().unwrap().push((num_rt, emit));
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the context, the stand-in and the framebuffers --------------------------- */

const SIZE: i32 = 16;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// THE one write of the draw-target count in this file. It is the stand-in
/// `backend::HOST_DRAW_TARGETS` today; the `knob` item replaces its body
/// with `pvr_debug_set_draw_targets(n)`. Call it before the back end is
/// brought up (`rig` does): `pvr_render_eot_four_rt` is read from it then.
fn set_draw_targets(n: u8) {
    yakogl::backend::HOST_DRAW_TARGETS.store(n, Ordering::Relaxed);
}

/// The one context of the file, with `targets` draw targets set (3 or 4) and the back end brought up for it: a change of
/// count restarts the back end, as a board brings the driver and the back
/// end up after the knob. The tests take turns.
fn rig(targets: u8) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    set_draw_targets(targets);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
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
        glUseProgram(0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    if pvr_backend_eot_pairs() != eot_pds_pairs(eot_four_rt()) {
        pvr_backend_shutdown();
        assert_eq!(pvr_backend_init(), 0, "{:?}", logs());
    }
    pvr_backend_set_option(PVR_BACKEND_OPT_PARTIAL_RENDER, 0);
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

const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/// An FBO with a renderbuffer of each of `formats` at COLOR_ATTACHMENT0..,
/// every one a draw buffer, bound for drawing.
fn fbo(formats: &[GLenum]) -> GLuint {
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
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, ATT[i], GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(formats.len() as GLsizei, ATT.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    fbo
}

/// What one piece of GL work handed the hardware.
#[allow(dead_code)] // `fs` is read by other files of this rig
struct Frame {
    kicks: Vec<PvrRenderKick>,
    /// Every end-of-tile data generation: pairs asked for, emit words.
    eot: Vec<(u32, Vec<u32>)>,
    /// The fragment programs of the recorded objects (USC-heap offsets).
    fs: Vec<u64>,
}

/// The kicks, end-of-tile generations and fragment programs of `f`.
fn frame_of(f: impl FnOnce()) -> Frame {
    // SAFETY: plain GL calls.
    unsafe { glFinish() };
    with_platform(|p| p.kicks.clear());
    EOT.lock().unwrap().clear();
    KICK_FS.lock().unwrap().clear();
    f();
    // SAFETY: plain GL calls.
    unsafe { glFinish() };
    Frame {
        kicks: with_platform(|p| p.kicks.clone()),
        eot: EOT.lock().unwrap().clone(),
        fs: KICK_FS.lock().unwrap().clone(),
    }
}

const C: [f32; 4] = [1.0, 0.5, 0.25, 1.0];

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

const FS1: &str = "#version 300 es\nprecision mediump float;\nlayout(location = 0) out vec4 o0;\n\
    void main() { o0 = vec4(1.0, 0.0, 0.0, 1.0); }\n";

/// Programs: the four-output one and the one-output one, linked once.
fn program(fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program links");
        p
    }
}

struct Rig {
    a: GLuint,
    b: GLuint,
    p4: GLuint,
    p1: GLuint,
}

fn rig_fbos() -> Rig {
    let a = fbo(&[GL_RGBA8; 4]);
    let b = fbo(&[GL_RGBA8]);
    let (p4, p1) = (program(FS4), program(FS1));
    // SAFETY: plain GL calls.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    Rig { a, b, p4, p1 }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Which {
    A,
    B,
    Default,
}

fn bind(r: &Rig, w: Which) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(
            GL_FRAMEBUFFER,
            match w {
                Which::A => r.a,
                Which::B => r.b,
                Which::Default => 0,
            },
        );
        glUseProgram(if w == Which::A { r.p4 } else { r.p1 });
        glViewport(0, 0, SIZE, SIZE);
    }
}

/// Draw into `w`, and (`flush`) glFinish after it.
fn draw_to(r: &Rig, w: Which, flush: bool) {
    bind(r, w);
    draw_triangle();
    assert_eq!(error(), GL_NO_ERROR, "{:?}", logs());
    if flush {
        // SAFETY: plain GL call.
        unsafe { glFinish() };
    }
}

const SEQ: [Which; 4] = [Which::A, Which::B, Which::Default, Which::A];

fn sequence(r: &Rig, flush: bool) -> Frame {
    let f = frame_of(|| {
        for w in SEQ {
            draw_to(r, w, flush);
        }
    });
    assert_eq!(f.kicks.len(), 4, "one render pass a draw: {:?}", logs());
    assert_eq!(f.eot.len(), 4, "one end-of-tile generation a pass");
    f
}

fn addrs(f: &Frame, i: usize, n: usize) -> Vec<u64> {
    (0..n).map(|t| emit_address(&pair(&f.eot[i].1, t))).collect()
}

fn check_sequence(f: &Frame) {
    for (i, w) in SEQ.iter().enumerate() {
        let k = &f.kicks[i].frag;
        let n = if *w == Which::A { 4 } else { 1 };
        assert_eq!(k.num_rt.max(1), n, "pass {i} ({w:?})");
        /* the back end's EOT program has four pairs whatever the pass; a
         * pass of fewer targets leaves the rest of its emit words zero */
        assert_eq!(f.eot[i].0, 4, "pass {i} ({w:?}): EOT pairs");
        assert!(f.eot[i].1[2 * n as usize..].iter().all(|w| *w == 0), "pass {i} ({w:?}): no emit past target {n}");
        assert!(f.eot[i].1[..2 * n as usize].iter().any(|w| *w != 0));
        for rt in 0..4 {
            if (rt as u32) < n {
                assert_ne!(k.pbe_word[rt], [0, 0], "pass {i} ({w:?}): PBE word {rt}");
            } else {
                assert_eq!(k.pbe_word[rt], [0, 0], "pass {i} ({w:?}): no PBE word {rt}");
            }
        }
    }
    let a0 = addrs(f, 0, 4);
    let a1 = addrs(f, 3, 4);
    assert_eq!(a0, a1, "A's second pass stores A's own four surfaces");
    let b = addrs(f, 1, 1);
    let d = addrs(f, 2, 1);
    assert!(!a0.contains(&b[0]), "B's surface is not one of A's");
    assert!(!a0.contains(&d[0]), "the default framebuffer's surface is not one of A's");
    assert_ne!(b, d);
    assert_eq!(f.kicks[0].frag.event_pixel_pds_info, f.kicks[3].frag.event_pixel_pds_info);
    assert_eq!(f.kicks[0].frag.event_pixel_pds_info, 0x64, "A: the four-target EOT program");
    assert_eq!(f.kicks[1].frag.event_pixel_pds_info, 0x61, "B: the one-target info word");
    assert_eq!(f.kicks[2].frag.event_pixel_pds_info, 0x61, "default: the one-target info word");
}

/* ---- (1) the render passes of A, B, default, A -------------------------------------- */

#[test]
fn one_a_b_default_a_in_one_frame() {
    let _g = rig(4);
    let r = rig_fbos();
    check_sequence(&sequence(&r, false));
}

#[test]
fn one_a_b_default_a_across_frames() {
    let _g = rig(4);
    let r = rig_fbos();
    for _ in 0..2 {
        check_sequence(&sequence(&r, true));
    }
}

/* ---- (2) a clear of A's attachment 3, then B -------------------------------------------- */

#[test]
fn two_clear_of_attachment_3_then_b_emits_nothing_for_it() {
    let _g = rig(4);
    let r = rig_fbos();
    for flush in [false, true] {
        let f = frame_of(|| {
            bind(&r, Which::A);
            // SAFETY: four floats.
            unsafe { glClearBufferfv(GL_COLOR, 3, C.as_ptr()) };
            assert_eq!(error(), GL_NO_ERROR, "{:?}", logs());
            if flush {
                // SAFETY: plain GL call.
                unsafe { glFinish() };
            }
            draw_to(&r, Which::B, false);
        });
        assert_eq!(f.kicks.len(), 2, "A's clear pass and B's draw: {:?}", logs());
        assert_eq!(f.eot.len(), 2);
        let a = addrs(&f, 0, 4);
        let b = addrs(&f, 1, 1);
        assert_eq!(f.kicks[0].frag.num_rt, 4);
        assert_ne!(f.kicks[0].frag.pbe_word[3], [0, 0], "A's pass stores attachment 3");
        assert_eq!(f.kicks[1].frag.num_rt.max(1), 1, "B's pass has one target");
        assert_eq!(f.kicks[1].frag.pbe_word[3], [0, 0]);
        assert!(f.eot[1].1[2..].iter().all(|w| *w == 0), "B emits one target only");
        assert!(!b.contains(&a[3]), "B's pass emits nothing for A's attachment-3 surface");
    }
}

/* ---- (3) the draw-buffer list survives the switch ----------------------------------------- */

fn db(fb: GLuint, i: u32) -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        get_int(GL_DRAW_BUFFER0 + i) as GLenum
    }
}

#[test]
fn three_draw_buffer_list_survives_binding_b_and_a() {
    let _g = rig(4);
    let r = rig_fbos();
    let list = [GL_NONE, GL_NONE, GL_NONE, GL_COLOR_ATTACHMENT3];
    /* A's four surfaces, from a draw with all four draw buffers on */
    let before = frame_of(|| draw_to(&r, Which::A, false));
    let surfaces = addrs(&before, 0, 4);
    // SAFETY: plain GL calls; four enums.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, r.a);
        glDrawBuffers(4, list.as_ptr());
        assert_eq!(error(), GL_NO_ERROR);
    }
    for round in 0..2 {
        assert_eq!(db(r.b, 0), GL_COLOR_ATTACHMENT0, "round {round}: B keeps its own list");
        assert_eq!(db(0, 0), GL_BACK, "round {round}: the default framebuffer's");
        for (i, want) in list.iter().enumerate() {
            assert_eq!(db(r.a, i as u32), *want, "round {round}: A's draw buffer {i}");
        }
        /* a draw to B in between does not disturb A's list */
        draw_to(&r, Which::B, true);
    }
    /* A's draw with the list stores attachment 3 alone */
    bind(&r, Which::A);
    let f = frame_of(|| {
        draw_triangle();
        assert_eq!(error(), GL_NO_ERROR, "{:?}", logs());
    });
    assert_eq!(f.kicks.len(), 1, "{:?}", logs());
    eprintln!("DBG num_rt {} pbe {:x?} eot {:x?}", f.kicks[0].frag.num_rt, f.kicks[0].frag.pbe_word, f.eot[0]);
    /* the list is one target, attachment 3's: the pass stores its surface and no other */
    assert_eq!(f.kicks[0].frag.num_rt.max(1), 1, "{:?}", f.kicks[0].frag.pbe_word);
    assert_ne!(f.kicks[0].frag.pbe_word[0], [0, 0]);
    assert_eq!(addrs(&f, 0, 1), [surfaces[3]], "the one emit is attachment 3's surface");
    assert!(f.eot[0].1[2..].iter().all(|w| *w == 0), "no emit for attachments 0..2");
    /* binding B and A once more leaves the list and the draw as they were */
    draw_to(&r, Which::B, true);
    bind(&r, Which::A);
    let f = frame_of(draw_triangle);
    assert_eq!(addrs(&f, 0, 1), [surfaces[3]]);
}

/* ---- helpers ------------------------------------------------------------------------- */

fn get_int(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: a plain GL call; room for one int.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(error(), GL_NO_ERROR, "{pname:#x}");
    v
}

/// The surface address of a PBE emit (pbestate.xml STATE_WORD0/1).
fn emit_address(w: &[u32; 2]) -> u64 {
    (u64::from(w[1] & 0xff) << 34) | (u64::from(w[0]) << 2)
}

fn pair(words: &[u32], i: usize) -> [u32; 2] {
    [words[2 * i], words[2 * i + 1]]
}
