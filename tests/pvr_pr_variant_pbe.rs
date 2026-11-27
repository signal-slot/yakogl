// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: the partial render's PBE words, SPM scratch size and end-of-tile
//! data of an 800x600 RGBA16F frame and of an 800x600 4x MSAA RGBA8 frame
//! equal what Mesa 26.2.1's `pvr_spm_setup_pbe_state` computes.
//!
//! The expected words are packed here, bit by bit, from Mesa's algorithm
//! and csbgen field layout (every constant cites its Mesa file:line, paths
//! under src/imagination/), not by `yakogl::pvr::spm`: the test checks that
//! module's output as the back end ships it in `frag_pr` (what the driver's
//! 3D_PR and the PR end-of-tile program are built from). A frame is driven
//! through the GL core into the real PowerVR back end with
//! `PVR_BACKEND_OPT_PARTIAL_RENDER` on, on a host platform (stand-ins for the
//! C generators as in tests/pvr_pr_eligibility.rs), and its kick's `frag_pr`
//! is read.
//!
//! The 4x case needs the `msaa-fbo` feature.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_pr_variant_pbe
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,msaa-fbo --test pvr_pr_variant_pbe
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
#![allow(dead_code)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::osal::render::{PvrRenderFragPr, PvrRenderKick, PvrRenderStatic};
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
    /// The next allocation of 4 MB or more is refused (once): the sample
    /// store of a 4x frame, so that frame is not a store-samples one.
    refuse_big_once: bool,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The dwords at GPU address `gpu`, `n` of them, from the allocation that
/// holds them.
fn gpu_words(gpu: u64, n: usize) -> Option<Vec<u32>> {
    with_platform(|p| {
        let (&base, a) = p.allocs.range(..=gpu).next_back()?;
        let off = (gpu - base) as usize;
        if off + 4 * n > a.size {
            return None;
        }
        // SAFETY: inside the allocation (checked above); u64 backing, so the
        // u32 view is aligned for any 4-byte offset.
        let w = unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off) as *const u32, n) };
        Some(w.to_vec())
    })
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let Some(h) = HEAPS.iter().position(|&b| b == heap_base) else { return -22 };
    with_platform(|p| {
        if size == 0 || size > 1 << 30 {
            return -12;
        }
        if size >= 4_000_000 && core::mem::take(&mut p.refuse_big_once) {
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
    KICK_FS_EVER.lock().unwrap().push(fs_usc_exec_offset);
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

const C: [f32; 4] = [1.0, 0.5, 0.25, 1.0];
const SIZE: i32 = 800;
const HEIGHT: i32 = 600;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The one context of the file, with `targets` draw targets stood in (0:
/// the build's own three) and the back end brought up for it: a change of
/// count restarts the back end, as a board brings the driver and the back
/// end up after the knob. The tests take turns.
fn rig(targets: u8) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(SIZE as u32, HEIGHT as u32);
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
            glRenderbufferStorage(GL_RENDERBUFFER, *f, SIZE, HEIGHT);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, ATT[i], GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(formats.len() as GLsizei, ATT.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    fbo
}

/// What one piece of GL work handed the hardware.
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

fn clear_all() -> Frame {
    frame_of(|| {
        // SAFETY: plain GL calls.
        unsafe {
            glClearColor(C[0], C[1], C[2], C[3]);
            glClear(GL_COLOR_BUFFER_BIT);
            assert_eq!(glGetError(), GL_NO_ERROR);
        }
    })
}

/// Frame kinds of the guard. A cleared frame of each, through the GL core.
fn kicks_of(formats: &[GLenum], samples: GLsizei, knob: u32) -> Vec<PvrRenderKick> {
    pvr_backend_set_option(PVR_BACKEND_OPT_PARTIAL_RENDER, knob);
    let mut fbo = 0;
    // SAFETY: plain GL calls; room for the names.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        for (i, f) in formats.iter().enumerate() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            if samples > 1 {
                glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, *f, SIZE, HEIGHT);
            } else {
                glRenderbufferStorage(GL_RENDERBUFFER, *f, SIZE, HEIGHT);
            }
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, ATT[i], GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(formats.len() as GLsizei, ATT.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    let f = clear_all();
    pvr_backend_set_option(PVR_BACKEND_OPT_PARTIAL_RENDER, 0);
    assert!(!f.kicks.is_empty(), "{formats:x?} x{samples}: a kick: {:?}", logs());
    f.kicks
}

fn is_zero(p: &PvrRenderFragPr) -> bool {
    p.spm_scratch_gpu == 0
        && p.spm_scratch_bytes == 0
        && p.reserved == 0
        && p.pbe_word.iter().all(|w| *w == [0, 0])
        && p.eot_pds_data == 0
        && p.eot_pds_data_bytes == 0
        && p.pds_pr_bgnd == [0; 3]
        && p.isp_zlsctl == 0
}

/// Every USC offset a kick-USC program was ever built for (unused here;
/// `p_*` stand-ins above push to it).
static KICK_FS_EVER: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/* ---- Mesa's pvr_spm_setup_pbe_state, packed by hand --------------------------- */

/// What `pvr_spm_setup_pbe_state` returns and fills for one output.
struct MesaPbe {
    state: [u32; 2],
    reg: [u64; 2],
    bytes: u64,
}

/// `pvr_spm_setup_pbe_state(dev_info, {width, height}, dword_count,
/// PVR_PBE_STARTPOS_BIT0, sample_count, addr, ..)`
/// (vulkan/pvr_arch_spm.c:285-326) with `pvr_arch_pbe_pack_state`
/// (vulkan/pvr_arch_job_common.c:82-297) and the csbgen layouts of
/// csbgen/rogue/pbestate.xml. Our core has no eight_output_registers
/// (pvr_arch_job_common.c:125-130), so source_pos_offset_128 stays 0.
fn mesa_pbe(width: u32, height: u32, dword_count: u32, sample_count: u32, addr: u64) -> MesaPbe {
    /* pvr_spm_get_pbe_packmode, pvr_arch_spm.c:258-273; the values of
     * pbestate.xml:72 (U32U32U32U32), :87, :101 (U32U32), :113 (U32) */
    let packmode: u64 = match dword_count {
        1 => 0x34,
        2 => 0x28,
        3 => 0x1a,
        4 => 0x0b,
        _ => unreachable!("Unsupported dword_count"),
    };
    /* the row: ALIGN_POT(width, REG_WORD0.linestride UNIT_SIZE),
     * pvr_arch_spm.c:295-297; UNIT_SIZE 2: pbestate.xml:258-260 */
    let unit = 2u32;
    let stride = width.next_multiple_of(unit);
    /* source_format 8_PER_CHANNEL (pvr_arch_spm.c:307; pbestate.xml:157,
     * value 1) whatever the frame's format: the PBE moves raw dwords */
    let source_format: u64 = 1;
    /* source_pos START_BIT0 (pvr_arch_spm.c:545 via pvr_pbestate_source_pos,
     * pvr_csb_enum_helpers.h:133-135; pbestate.xml:161, value 0) */
    let source_pos: u64 = 0;
    /* STATE_WORD0: address_low, start 0, shift 2 (pbestate.xml:227;
     * pvr_arch_job_common.c:115-117) */
    let w0 = (addr >> 2) as u32;
    /* STATE_WORD1 (pbestate.xml:230-243; pvr_arch_job_common.c:119-139):
     * address_high = addr >> 34 at 0, packmode at 7, norm 0 at 13,
     * source_pos at 14, mrt_index 0 at 16, source_format at 19 */
    let w1 = ((addr >> 34) & 0x3f) | packmode << 7 | source_pos << 14 | source_format << 19;
    /* REG_WORD0 (pbestate.xml:245-273; pvr_arch_job_common.c:141-283):
     * tilerelative (bit 57), rotation 0_DEG (pbestate.xml:131) and y_flip
     * DISABLED (:189): zero, linestride (stride - 1) / UNIT_SIZE at 34
     * (:174-175), memlayout LINEAR 0 at 32 (:155; pbestate.xml:54), swizzle
     * X,Y,Z,W from channel 0..3 (:179-277; SWIZ SOURCE_CHAN0..3 =
     * 0..3, pbestate.xml:168-171) at 20/23/26/29, minclip_x 0 at 6 */
    let r0 = 1u64 << 57 | u64::from((stride - 1) / unit) << 34 | (3u64 << 29 | 2 << 26 | 1 << 23);
    /* REG_WORD1 (pbestate.xml:275-282; pvr_arch_job_common.c:285-296):
     * maxclip_x at 32, maxclip_y at 0, the rest zero */
    let r1 = u64::from(width - 1) << 32 | u64::from(height - 1);
    MesaPbe {
        state: [w0, w1 as u32],
        reg: [r0, r1],
        /* pvr_arch_spm.c:324-325, PVR_DW_TO_BYTES(n) = 4 n */
        bytes: u64::from(stride) * u64::from(height) * u64::from(sample_count) * 4 * u64::from(dword_count),
    }
}

/// `pvr_arch_spm_scratch_buffer_calc_required_size`
/// (vulkan/pvr_arch_spm.c:39-75) for a pass of one render with
/// `output_regs_count` dwords and no tile buffers: dwords_per_pixel =
/// samples x dwords x (1 + 0) (:67), the row ALIGN_POT(width,
/// LINESTRIDE_ALIGNMENT 2) (:69-70; cr.xml:612-614) times height times 4 B
/// a dword (:71-72).
fn mesa_scratch_size(width: u32, height: u32, samples: u32, dwords: u32) -> u64 {
    u64::from(width.next_multiple_of(2)) * u64::from(height) * 4 * u64::from(samples) * u64::from(dwords)
}

/// The kick of a cleared 800x600 frame of `format` at `samples`, option on,
/// with the end-of-tile generations of that frame.
fn pr_kick(format: GLenum, samples: GLsizei) -> (PvrRenderKick, Vec<(u32, Vec<u32>)>) {
    let kicks = kicks_of(&[format], samples, 1);
    let eot = EOT.lock().unwrap().clone();
    (*kicks.last().unwrap(), eot)
}

/// frag_pr against Mesa's words for `dwords` output dwords at `samples`.
fn check_against_mesa(what: &str, format: GLenum, samples: GLsizei, dwords: u32, bytes: u64) {
    let _g = rig(4);
    let (k, eot) = pr_kick(format, samples);
    let p = &k.frag_pr;
    assert!(!is_zero(p), "{what}: {p:?}");
    assert_eq!((k.width, k.height, k.samples.max(1)), (SIZE as u32, HEIGHT as u32, samples as u32), "{what}");

    /* the scratch size: Mesa's two formulas agree, and so does the back end */
    let (w, h, s) = (SIZE as u32, HEIGHT as u32, samples as u32);
    assert_eq!(mesa_scratch_size(w, h, s, dwords), bytes, "{what}: the hand-computed size");
    assert_eq!(u64::from(p.spm_scratch_bytes), bytes, "{what}: spm_scratch_bytes");
    let a = p.spm_scratch_gpu;
    assert_eq!(with_platform(|pl| pl.allocs.get(&a).map(|x| x.size)), Some(bytes as usize), "{what}: the allocation");

    /* the PBE words, with the address the back end allocated */
    let m = mesa_pbe(w, h, dwords, s, a);
    assert_eq!(m.bytes, bytes, "{what}: the size pvr_spm_setup_pbe_state returns");
    assert_eq!(p.pbe_word[0], m.reg, "{what}: slot 0 is REG_WORD0, REG_WORD1");
    assert!(p.pbe_word[1..].iter().all(|x| *x == [0, 0]), "{what}: {:x?}", p.pbe_word);

    /* the EOT data address: the PR end-of-tile program's data slot holds the
     * STATE words Mesa packs (pvr_arch_spm.c:541-549 hands them to the
     * pixel-event program), whose address fields are the scratch's */
    assert_ne!(p.eot_pds_data, 0, "{what}");
    assert_ne!(p.eot_pds_data, k.frag.event_pixel_pds_data, "{what}: the PR's own slot");
    assert!(
        eot.iter().any(|(_, e)| e[..2] == m.state),
        "{what}: the PR emit is Mesa's STATE words {:x?}: {eot:x?}",
        m.state
    );
    assert_eq!(gpu_words(HEAP_PDS_BASE + u64::from(p.eot_pds_data), 2), Some(m.state.to_vec()), "{what}: the EOT data");
    let at = (u64::from(m.state[0]) << 2) | (u64::from(m.state[1]) & 0x3f) << 34;
    assert_eq!(at, a, "{what}: STATE_WORD0/1 address fields name the scratch");
}

/// (a) 800x600 RGBA16F: two output dwords a pixel (the 2-dword SPM load),
/// one sample: 800 x 600 x 4 B x 2 = 3 840 000 B, pack mode U32U32,
/// source_format 8_PER_CHANNEL like every SPM store (pvr_arch_spm.c:307).
#[test]
fn rgba16f_pr_words_follow_mesa() {
    check_against_mesa("RGBA16F", GL_RGBA16F, 1, 2, 3_840_000);
}

/// (b) 800x600 4x MSAA RGBA8: one output dword a pixel at four samples:
/// 800 x 600 x 4 B x 4 = 7 680 000 B. Mesa's PBE words do not depend on the
/// sample count (it only scales the size, pvr_arch_spm.c:324-325): the words
/// are RGBA8's, the PR store writes every sample.
#[cfg(feature = "msaa-fbo")]
#[test]
fn msaa4_pr_words_follow_mesa() {
    check_against_mesa("4x RGBA8", GL_RGBA8, 4, 1, 7_680_000);
}

/// The hand-packed words themselves, against the values the existing
/// RGBA8 record (src/pvr/backend_tests.rs `pr_frag_pr_for_800x600_rgba8`,
/// docs/notes) pins, so a slip in `mesa_pbe` shows here and not as a
/// matching slip in both sides.
#[test]
fn mesa_pbe_words_are_the_known_rgba8_ones() {
    let m = mesa_pbe(800, 600, 1, 1, 0x4_8010_0000);
    assert_eq!(m.state, [0x2004_0000, 0x0008_1a01]);
    assert_eq!(m.reg, [0x0200_063c_6880_0000, 0x0000_031f_0000_0257]);
    let m2 = mesa_pbe(800, 600, 2, 1, 0x4_8010_0000);
    assert_eq!(m2.state, [0x2004_0000, 0x0008_1401], "U32U32 (0x28 << 7)");
    assert_eq!(mesa_pbe(800, 600, 1, 4, 0).bytes, 7_680_000);
}
