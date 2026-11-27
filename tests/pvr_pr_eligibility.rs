// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Partial-render eligibility per format, MRT and MSAA: which frames the PowerVR
//! back end gives a filled `frag_pr` block (`pr_fill`, src/pvr/backend.rs) with
//! `PVR_BACKEND_OPT_PARTIAL_RENDER` on, and which an all-zero one (the
//! driver's 3D_PR then stays a byte copy of the 3D). Written before the 2-dword
//! and 4x MSAA SPM load shaders (tools/pco_shc/spm_load_2dw, spm_load_msaa4) were
//! wired, so that wiring is tested against it.
//!
//! Three load shaders ship: Mesa 26.2.1's SPM load for {output_reg_count 1,
//! tile_buffer_count 0, not multisampled} (`spm_load_blob.rs`), for
//! {output_reg_count 2, 0, not multisampled} (`spm_load_2dw_blob.rs`, an
//! RGBA16F target) and for {output_reg_count 1, 0, multisampled}
//! (`spm_load_msaa4_blob.rs`, a 4x RGBA8 target); `spm::spm_load_for` selects
//! one. What pr-wider-read found a frame needs, per kind, is in the comment of
//! each case: Mesa picks the SPM load shader by output_reg_count (1, 2, 4, 8
//! dwords), tile_buffer_count (tile-buffer outputs after the registers) and
//! sample count (the msaa load reads every sample).
//!
//! Every case pins the exact outcome. With the option off every case has a
//! zero block. Nothing here ran on a board.
//!
//! The 4x cases need the `msaa-fbo` feature.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_pr_eligibility
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,msaa-fbo --test pvr_pr_eligibility
//! ```

//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_pr_fill_guard
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,msaa-fbo --test pvr_pr_fill_guard
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

/// One RGBA8 attachment: eligible, the one shipped load shader covers it.
#[test]
fn single_rgba8_gets_a_filled_block_only_with_the_option_on() {
    let _g = rig(4);
    let k = kicks_of(&[GL_RGBA8], 1, 1);
    assert_eq!(k.len(), 1, "{:?}", logs());
    assert!(!is_zero(&k[0].frag_pr), "{:?}", k[0].frag_pr);
    assert_ne!(k[0].frag_pr.spm_scratch_gpu, 0);
    assert_ne!(k[0].frag_pr.eot_pds_data, 0);
    for k in kicks_of(&[GL_RGBA8], 1, 0) {
        assert!(is_zero(&k.frag_pr), "knob off: {:?}", k.frag_pr);
    }
}

/// With the option on, `formats` x `samples` is zero today.
fn zero_today(what: &str, formats: &[GLenum], samples: GLsizei) {
    for k in kicks_of(formats, samples, 1) {
        assert!(is_zero(&k.frag_pr), "{what}, knob on: {:?}", k.frag_pr);
    }
}

/// With the option on, `formats` x `samples` has a filled block.
fn filled(what: &str, formats: &[GLenum], samples: GLsizei) {
    let k = kicks_of(formats, samples, 1);
    assert!(!is_zero(&k.last().unwrap().frag_pr), "{what}, knob on: {:?}", k.last().unwrap().frag_pr);
}

/// With the option off every case has a zero block.
fn off_is_zero(what: &str, formats: &[GLenum], samples: GLsizei) {
    for k in kicks_of(formats, samples, 0) {
        assert!(is_zero(&k.frag_pr), "{what}, knob off: {:?}", k.frag_pr);
    }
}

/// MRT (two RGBA8): zero today (the guard is `nrt == 1`). pr-wider-read: Mesa's
/// SPM load runs per output_reg_count; two RGBA8 outputs are two registers, a
/// different load program (and a PBE word per target), not tile buffers.
#[test]
fn mrt_gets_no_block_today() {
    let _g = rig(4);
    zero_today("two RGBA8", &[GL_RGBA8; 2], 1);
    zero_today("four RGBA8", &[GL_RGBA8; 4], 1);
    off_is_zero("two RGBA8", &[GL_RGBA8; 2], 1);
    off_is_zero("four RGBA8", &[GL_RGBA8; 4], 1);
}

/// The words of a committed load shader (tools/pco_shc/<variant>/spm_load.hex).
fn hex_words(hex: &str) -> Vec<u32> {
    hex.lines()
        .filter(|l| !l.starts_with('#'))
        .flat_map(|l| l.split_whitespace())
        .map(|w| u32::from_str_radix(w.trim_start_matches("0x"), 16).unwrap())
        .collect()
}

/// The filled block of a cleared 800x600 frame of `formats` x `samples`,
/// pinned: the scratch (`bytes`, Mesa's size for `dwords` output dwords at
/// `samples`), the PR PBE register words of `spm::pbe_state` over it (slot 0
/// only), the PR end-of-tile's emit (its STATE words) and its data, the
/// background load's three words, and that the load kick runs the code of
/// `load_hex` (the variant's committed words, in the USC heap).
fn filled_words(
    what: &str,
    formats: &[GLenum],
    samples: GLsizei,
    dwords: u32,
    bytes: u32,
    load_hex: &str,
) -> PvrRenderKick {
    let kicks = kicks_of(formats, samples, 1);
    /* the frame's end-of-tile generations (clear_all's frame_of keeps them) */
    let eot = EOT.lock().unwrap().clone();
    let k = kicks.last().unwrap();
    let p = &k.frag_pr;
    assert!(!is_zero(p), "{what}, knob on: {p:?}");
    assert_eq!((k.width, k.height, k.samples.max(1)), (SIZE as u32, HEIGHT as u32, samples as u32), "{what}");
    assert_eq!(p.spm_scratch_bytes, bytes, "{what}");
    let s = p.spm_scratch_gpu;
    assert_eq!(with_platform(|pl| pl.allocs.get(&s).map(|a| a.size)), Some(bytes as usize), "{what}: the scratch");
    let pbe = yakogl::pvr::spm::pbe_state(SIZE as u32, HEIGHT as u32, dwords, 0, samples as u32, s).unwrap();
    assert_eq!(pbe.bytes, bytes as u64);
    assert_eq!(p.pbe_word[0], [pbe.reg[0], pbe.reg[1]], "{what}");
    assert!(p.pbe_word[1..].iter().all(|w| *w == [0, 0]), "{what}: {:x?}", p.pbe_word);
    assert_eq!(p.reserved, 0);
    assert_ne!(p.eot_pds_data, 0);
    assert_ne!(p.eot_pds_data, k.frag.event_pixel_pds_data, "{what}: the PR's own slot");
    assert_eq!(p.eot_pds_data_bytes, 16, "{what}: the stand-in's 4 data dwords");
    assert!(eot.iter().any(|(_, e)| e[..2] == pbe.state && e[2..].iter().all(|&w| w == 0)), "{what}: the PR emit");
    assert!(p.pds_pr_bgnd.iter().all(|&w| w != 0), "{what}: {:x?}", p.pds_pr_bgnd);
    assert_ne!(p.pds_pr_bgnd[0], k.frag.pds_bgnd[0], "{what}: the load kick, not the clear's");
    assert_eq!(p.isp_zlsctl, 0, "{what}: no depth/stencil buffer");
    let want = hex_words(load_hex);
    let ever = KICK_FS_EVER.lock().unwrap().clone();
    let runs = ever.iter().any(|&off| gpu_words(HEAP_USC_BASE + off, want.len()).as_deref() == Some(&want[..]));
    assert!(runs, "{what}: a load kick runs the variant's code");
    *k
}

/// Every USC offset a kick-USC program was ever built for (the load shaders
/// are built once, on the first frame that needs each).
static KICK_FS_EVER: Mutex<Vec<u64>> = Mutex::new(Vec::new());

/// RGBA16F (2 dwords per pixel): reloaded by the 2-dword SPM load
/// (tools/pco_shc/spm_load_2dw; pr-wider-read: output_reg_count 2). The
/// scratch is 800 x 600 x 4 B x 2 dwords; the PR store packs U32U32.
#[test]
fn rgba16f_gets_a_block_with_the_2dw_load() {
    let _g = rig(4);
    filled("RGBA16F", &[GL_RGBA16F], 1);
    filled_words("RGBA16F", &[GL_RGBA16F], 1, 2, 3_840_000, include_str!("../tools/pco_shc/spm_load_2dw/spm_load.hex"));
    off_is_zero("RGBA16F", &[GL_RGBA16F], 1);
}

/// 4x MSAA RGBA8: reloaded by the multisampled SPM load
/// (tools/pco_shc/spm_load_msaa4), which reads every sample; the scratch is
/// sized by the sample count (800 x 600 x 4 B x 4 samples). With its sample
/// store and without it (the store's allocation refused) alike: the PR has
/// one emit whatever the frame's.
#[cfg(feature = "msaa-fbo")]
#[test]
fn msaa4_gets_a_block_with_the_msaa4_load() {
    let _g = rig(4);
    let hex = include_str!("../tools/pco_shc/spm_load_msaa4/spm_load.hex");
    filled("4x RGBA8", &[GL_RGBA8], 4);
    let k = filled_words("4x RGBA8", &[GL_RGBA8], 4, 1, 7_680_000, hex);
    assert_eq!(k.frag.num_rt, 2, "the frame's emits: the target and its sample store");
    /* a fresh back end, so the sample store is asked for (and refused) */
    pvr_backend_shutdown();
    assert_eq!(pvr_backend_init(), 0, "{:?}", logs());
    with_platform(|p| p.refuse_big_once = true);
    let k = filled_words("4x RGBA8 without a sample store", &[GL_RGBA8], 4, 1, 7_680_000, hex);
    assert_eq!(k.frag.num_rt, 1, "the frame's one emit");
    assert!(!with_platform(|p| p.refuse_big_once), "the sample store was asked for and refused");
    off_is_zero("4x RGBA8", &[GL_RGBA8], 4);
}

/// 4x MSAA RGBA16F: no shipped load shader has two dwords multisampled
/// (Mesa has one; tools/pco_shc does not generate it), so zero.
#[cfg(feature = "msaa-fbo")]
#[test]
fn msaa4_rgba16f_gets_no_block() {
    let _g = rig(4);
    zero_today("4x RGBA16F", &[GL_RGBA16F], 4);
    off_is_zero("4x RGBA16F", &[GL_RGBA16F], 4);
}

/// Three RGBA16F overrun the output registers (Mesa then uses tile buffers
/// after the registers: tile_buffer_count > 0). Zero today, and still zero
/// after the 2-dword and 4x wiring, which cover neither tile buffers nor MRT.
#[test]
fn tile_buffer_frame_gets_no_block() {
    let _g = rig(4);
    zero_today("three RGBA16F", &[GL_RGBA16F; 3], 1);
    off_is_zero("three RGBA16F", &[GL_RGBA16F; 3], 1);
}
