// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Sampling a texture that an earlier pass rendered through COLOR_ATTACHMENT3,
//! on the PowerVR back end at four draw targets (item loop4-pvr of plan
//! #368; docs/es3-cts-failures.md RC14; docs/notes/mrt4-v3d-pvr.md).
//!
//! Pass 1 renders four RGBA8 textures T0..T3 (one four-output draw). Pass 2
//! binds T3, samples it and draws into a second four-target FBO U0..U3 (every
//! output derived from the sample). Pass 3 swaps the roles: it samples U3 and
//! draws into T0..T3 again. No feedback loop is built (a texture is never
//! sampled while it is attached to the bound FBO).
//!
//! WHAT THE RIG CAN SEE. The platform draws nothing: a kick is recorded and
//! completes at once, so no pixel is ever written and "every pass-2 target
//! holds the value derived from T3" cannot be read back here (it is the
//! board's: gl_test case 245 and docs/notes/mrt4-v3d-pvr.md section 5). What
//! it sees, and what is pinned, is what makes that value possible: the
//! kicks and their order, the fourth PBE surface of each pass (read from the
//! end-of-tile data's emit words: the surface address of render target 3),
//! that pass 1's render target 3 is texture T3's own storage and the sampled
//! surface of pass 2, that pass 2's four surfaces are four others (so T3 is
//! not stored over), and the roles swapped for pass 3.
//!
//! The count is set by `set_draw_targets`, the stand-in
//! `backend::HOST_DRAW_TARGETS`, as in tests/es3_mrt4_pvr.rs.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_mrt4_sample_pvr
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
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::{glActiveTexture, glBindTexture, glGenTextures, glTexImage2D, glTexParameteri};
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
        assert_eq!(ok, 1, "the program links");
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

/* ---- what the file reads ------------------------------------------------------------- */

/// The surface address of a PBE emit (pbestate.xml STATE_WORD0/1: the low
/// bits' dword address in word 0, the high byte in word 1's low byte).
fn emit_address(w: &[u32; 2]) -> u64 {
    (u64::from(w[1] & 0xff) << 34) | (u64::from(w[0]) << 2)
}

/// The four surface addresses of one end-of-tile generation (emit words,
/// two a render target).
fn surfaces(words: &[u32]) -> [u64; 4] {
    core::array::from_fn(|i| emit_address(&[words[2 * i], words[2 * i + 1]]))
}

/// Four RGBA8 textures attached at COLOR_ATTACHMENT0..3 of a new FBO, every
/// attachment a draw buffer; the FBO stays bound for drawing.
fn tex_fbo() -> (GLuint, [GLuint; 4]) {
    let (mut fbo, mut tex) = (0, [0; 4]);
    // SAFETY: plain GL calls; room for the names; a null image is an
    // allocation without data.
    unsafe {
        glGenTextures(4, tex.as_mut_ptr());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        for (i, t) in tex.iter().enumerate() {
            glBindTexture(GL_TEXTURE_2D, *t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8 as GLint,
                SIZE,
                SIZE,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            glFramebufferTexture2D(GL_FRAMEBUFFER, ATT[i], GL_TEXTURE_2D, *t, 0);
        }
        glDrawBuffers(4, ATT.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    (fbo, tex)
}

/// Pass 1's program: four constant outputs.
const FS_WRITE: &str = FS4;

/// Pass 2's program: the texel under the fragment, four ways.
const FS_SAMPLE: &str = "#version 300 es\nprecision mediump float;\nuniform sampler2D s;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
    layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
    void main() { vec4 v = texelFetch(s, ivec2(gl_FragCoord.xy), 0);\n\
    o0 = v; o1 = v.bgra; o2 = v.gbra; o3 = vec4(1.0) - v; }\n";

/// Link `fs`, make it current; the sampler (if any) reads unit 0.
fn program(fs: &str) {
    use_program(fs);
    // SAFETY: plain GL calls; a NUL-terminated name.
    unsafe {
        let mut p: GLint = 0;
        glGetIntegerv(GL_CURRENT_PROGRAM, &mut p);
        let loc = glGetUniformLocation(p as GLuint, c"s".as_ptr());
        if loc >= 0 {
            glUniform1i(loc, 0);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

fn draw_into(fbo: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glViewport(0, 0, SIZE, SIZE);
    }
    draw_triangle();
    assert_eq!(error(), GL_NO_ERROR, "{:?}", logs());
}

fn sample_unit0(t: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t);
    }
}

/// The three passes in one frame: 1 writes T0..T3, 2 samples T3 into
/// U0..U3, 3 samples U3 into T0..T3.
struct Passes {
    f: Frame,
    /// The surfaces each pass stored, from its end-of-tile data.
    t: [u64; 4],
    u: [u64; 4],
}

fn three_passes() -> Passes {
    let (a, ta) = tex_fbo();
    let (b, ub) = tex_fbo();
    let f = frame_of(|| {
        program(FS_WRITE);
        draw_into(a);
        program(FS_SAMPLE);
        sample_unit0(ta[3]);
        draw_into(b);
        sample_unit0(ub[3]);
        draw_into(a);
    });
    assert_eq!(f.kicks.len(), 3, "one kick a pass: {:?}", logs());
    assert_eq!(f.eot.len(), 3, "{:?}", logs());
    let (t, u) = (surfaces(&f.eot[0].1), surfaces(&f.eot[1].1));
    Passes { f, t, u }
}

/* ---- (1) pass 1 renders four targets; pass 2 samples T3 into a second four-target FBO ---- */

#[test]
fn pass_1_stores_four_surfaces_and_pass_2_four_others() {
    let _g = rig(4);
    let p = three_passes();
    for (n, k) in p.f.kicks.iter().enumerate() {
        assert_eq!(k.frag.num_rt, 4, "pass {}: four render targets", n + 1);
        for (rt, w) in k.frag.pbe_word.iter().enumerate().take(4) {
            assert_ne!(*w, [0, 0], "pass {}: render target {rt} has its PBE state", n + 1);
        }
        assert_eq!(p.f.eot[n].0, 4, "pass {}: the four-target end-of-tile program", n + 1);
    }
    let all: Vec<u64> = p.t.iter().chain(&p.u).copied().collect();
    for (i, a) in all.iter().enumerate() {
        assert_ne!(*a, 0, "surface {i}");
        assert!(gpu_words(*a, 1).is_some(), "surface {i} ({a:#x}) is in an allocation");
        for (j, b) in all.iter().enumerate().take(i) {
            assert_ne!(a, b, "surfaces {j} and {i}: eight textures, eight storages");
        }
    }
}

/// Pass 2 stores into U0..U3 and never into T3 (it would be a feedback loop
/// and the sampled value would be lost): none of its surfaces is one of
/// pass 1's.
#[test]
fn pass_2_does_not_store_over_the_sampled_t3() {
    let _g = rig(4);
    let p = three_passes();
    for (rt, s) in p.u.iter().enumerate() {
        assert!(!p.t.contains(s), "pass 2 render target {rt} ({s:#x}) is one of pass 1's surfaces");
    }
}

/// The order: pass 1's kick is submitted before pass 2's, so T3 is stored
/// when pass 2 samples it; the kicks are told apart by their end-of-tile data.
#[test]
fn pass_1_is_kicked_before_pass_2() {
    let _g = rig(4);
    let p = three_passes();
    assert_eq!(surfaces(&p.f.eot[0].1), p.t);
    assert_eq!(surfaces(&p.f.eot[1].1), p.u);
    // the programs: pass 1 and pass 2 ran different fragment programs
    assert!(p.f.fs.len() >= 3, "{:?}", p.f.fs);
    assert_ne!(p.f.fs.first(), p.f.fs.get(1), "pass 2 runs the sampling program, not pass 1's");
}

/// The fourth PBE state: the PBE register words describe format and layout
/// (the surface address is in the end-of-tile emit, read above), so every
/// pass of RGBA8 textures of one size has the same render target 3 words,
/// non-zero and not render target 2's copy; what differs is the surface.
#[test]
fn the_fourth_pbe_state_of_each_pass_names_its_own_surface() {
    let _g = rig(4);
    let p = three_passes();
    let (k1, k2) = (&p.f.kicks[0].frag, &p.f.kicks[1].frag);
    assert_ne!(k1.pbe_word[3], [0, 0]);
    assert_eq!(k1.pbe_word[3], k2.pbe_word[3], "same format and layout in both passes");
    assert_ne!(p.t[3], p.u[3], "T3 and U3 are different surfaces");
    assert_ne!(p.t[3], p.t[2]);
}

/* ---- (2) the roles swapped ---------------------------------------------------------------- */

/// Pass 3 samples U3 and draws into T0..T3 again: its four surfaces are
/// pass 1's, in the same order, and none is U3.
#[test]
fn swapped_roles_pass_3_stores_t_and_samples_u3() {
    let _g = rig(4);
    let p = three_passes();
    let third = surfaces(&p.f.eot[2].1);
    assert_eq!(third, p.t, "pass 3 stores T0..T3, the targets of pass 1");
    assert!(!third.contains(&p.u[3]), "U3 is sampled, not stored");
    assert_eq!(p.f.kicks[2].frag.num_rt, 4);
    assert_eq!(p.f.kicks[2].frag.pbe_word[3], p.f.kicks[0].frag.pbe_word[3], "T3's PBE state, as in pass 1");
    assert_eq!(surfaces(&p.f.eot[2].1)[3], p.t[3], "render target 3 is T3 again");
}

/// Sampling a texture other than T3 in pass 2 (T0, rendered through
/// COLOR_ATTACHMENT0) kicks the same passes: attachment 3 is not special
/// to the sampler.
#[test]
fn sampling_t0_instead_of_t3_gives_the_same_passes() {
    let _g = rig(4);
    let (a, ta) = tex_fbo();
    let (b, ub) = tex_fbo();
    let f = frame_of(|| {
        program(FS_WRITE);
        draw_into(a);
        program(FS_SAMPLE);
        sample_unit0(ta[0]);
        draw_into(b);
        sample_unit0(ub[0]);
        draw_into(a);
    });
    assert_eq!(f.kicks.len(), 3, "{:?}", logs());
    let (t, u, third) = (surfaces(&f.eot[0].1), surfaces(&f.eot[1].1), surfaces(&f.eot[2].1));
    assert_eq!(third, t);
    assert!(u.iter().all(|s| !t.contains(s)));
    assert!(f.kicks.iter().all(|k| k.frag.num_rt == 4));
}

/* ---- with three targets the same sampling stays as it was ----------------------------------- */

#[test]
fn with_the_count_at_three_pass_2_has_no_fourth_surface() {
    let _g = rig(3);
    let (a, ta) = {
        let (fbo, tex) = tex_fbo3();
        (fbo, tex)
    };
    let (b, _) = tex_fbo3();
    let f = frame_of(|| {
        program(FS_WRITE3);
        draw_into(a);
        program(FS_SAMPLE3);
        sample_unit0(ta[2]);
        draw_into(b);
    });
    assert_eq!(f.kicks.len(), 2, "{:?}", logs());
    for k in &f.kicks {
        assert_eq!(k.frag.num_rt, 3);
        assert_eq!(k.frag.pbe_word[3], [0, 0], "no fourth PBE state");
    }
}

fn tex_fbo3() -> (GLuint, [GLuint; 3]) {
    let (mut fbo, mut tex) = (0, [0; 3]);
    // SAFETY: plain GL calls; room for the names.
    unsafe {
        glGenTextures(3, tex.as_mut_ptr());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        for (i, t) in tex.iter().enumerate() {
            glBindTexture(GL_TEXTURE_2D, *t);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                GL_RGBA8 as GLint,
                SIZE,
                SIZE,
                0,
                GL_RGBA,
                GL_UNSIGNED_BYTE,
                core::ptr::null(),
            );
            glFramebufferTexture2D(GL_FRAMEBUFFER, ATT[i], GL_TEXTURE_2D, *t, 0);
        }
        glDrawBuffers(3, ATT.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    (fbo, tex)
}

const FS_WRITE3: &str = "#version 300 es\nprecision mediump float;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\nlayout(location = 2) out vec4 o2;\n\
    void main() { o0 = vec4(1.0, 0.0, 0.0, 1.0); o1 = vec4(0.0, 1.0, 0.0, 1.0); o2 = vec4(0.0, 0.0, 1.0, 1.0); }\n";
const FS_SAMPLE3: &str = "#version 300 es\nprecision mediump float;\nuniform sampler2D s;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\nlayout(location = 2) out vec4 o2;\n\
    void main() { vec4 v = texelFetch(s, ivec2(gl_FragCoord.xy), 0); o0 = v; o1 = v.bgra; o2 = v.gbra; }\n";
