// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Fragment-shader `discard` with four colour outputs on the PowerVR
//! BXS-4-64 host back end (item discard4-pvr of plan #368; RC14,
//! docs/es3-cts-failures.md; docs/notes/pvr-mrt4-budget.md), with the
//! draw-target count at 4.
//!
//! WHAT THE RIG CAN AND CANNOT SEE. The platform draws nothing: a kick is
//! kept and completes at once, so no pixel of an attachment is ever written
//! or kept by this file, and "a discarded pixel leaves its four attachments
//! at their cleared colours" cannot be read back here. The byte model of
//! tests/usc_exec runs one instance and does not model the ISP's alpha
//! feedback either: a discard compiles to a kill test (`tstz` into p0, no
//! destination), then `alphaf` (the feedback that drops the pixel at the
//! ISP) and `wdf`, THEN the four pixel-output writes, which every instance
//! runs (the model writes them for a killed one too, and this file pins that
//! it does, so a change that moves a write behind a branch shows). Whether
//! a killed pixel keeps its cleared colour on all four targets is the ISP's
//! and the PBE's, a board result. What this file pins is what the back end
//! builds around it:
//!
//! - (1) a shader that always discards (`v.x < 1.0e30` on a varying, which
//!   the compiler cannot fold) builds the same shape: kill test, `alphaf`,
//!   `wdf`, writes o0-o3; and the draw ends in a four-target store (one
//!   kick, `num_rt` 4, four PBE words, four-pair EOT, four distinct emit
//!   surfaces), so the cleared contents of all four targets are loaded and
//!   stored back by the same job that the discard acts in;
//! - (2) a conditional discard (`v.x < 0.5`) does the same, and the byte
//!   model's four outputs each carry their own colour for a kept and for a
//!   discarded value of v.x;
//! - (3) the program the back end builds spans four output registers
//!   (`num_outputs` 4, the writes o0-o3 in registers 0..3, once each),
//!   after the `alphaf`; the same holds with three targets.
//!
//! The GL-level draws of these shaders are ignored defect pins: glCompileShader
//! refuses them today (see `defect_gl_draw_...`); the four-target store they
//! must end in is `assert_four_target_store`.
//!
//! The per-target compile (`compile_fragment_targets`, four RGBA8 classes)
//! is used: key 0 refuses a discard beside several outputs ("alpha-to-one in
//! a shader with more than one colour output"), which is not what a draw
//! compiles under.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_mrt4_discard_pvr
//! ```
#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", feature = "test-knobs"))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]
/* the rig is the one of es3_mrt4_pvr.rs; what this file does not read stays */
#![allow(dead_code)]

mod usc_exec;

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
use yakogl::pvr::internal::{PVR_INTERNAL_CLEAR_ATTACH1, PVR_INTERNAL_FBLOAD, PVR_INTERNAL_TABLE};

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

const C: [f32; 4] = [1.0, 0.5, 0.25, 1.0];

/// The PBE STATE_WORD1 fields of one emit: `source_pos` (the first output
/// register) and `mrt_index` (pbestate.xml).
fn source_and_index(w1: u32) -> (u32, u32) {
    ((w1 >> 14) & 3, (w1 >> 16) & 7)
}

/// The (source_pos, mrt_index) of the first `n` emits of one generation.
fn emits(words: &[u32], n: usize) -> Vec<(u32, u32)> {
    words.chunks(2).take(n).map(|p| source_and_index(p[1])).collect()
}

/* ---- the programs the objects ran ----------------------------------------------- */

fn table_words(i: usize) -> &'static [u32] {
    let sh = &PVR_INTERNAL_TABLE[i].shader;
    // SAFETY: a row of the generated table points at its own words.
    unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }
}

/// `words` against `base`: Some(k) when they are `base` with every byte that
/// differs moved up by the same `k` (the back end moves a program's pixel
/// output writes by rewriting their destination bytes), 0 when equal.
fn moved_by(words: &[u32], base: &[u32]) -> Option<u8> {
    if words.len() != base.len() {
        return None;
    }
    let bytes = |w: &[u32]| w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let (a, b) = (bytes(words), bytes(base));
    let mut k = None;
    for (x, y) in a.iter().zip(&b) {
        if x != y {
            let d = x.wrapping_sub(*y);
            if *k.get_or_insert(d) != d {
                return None;
            }
        }
    }
    Some(k.unwrap_or(0))
}

/// What the object of fragment program `off` is: a frame load writing from
/// register r, or a one-dword clear writing register r.
#[derive(Debug, PartialEq, Eq)]
enum Object {
    Load(u8),
    Clear(u8),
    Other,
}

fn object(off: u64) -> Object {
    let load = table_words(PVR_INTERNAL_FBLOAD);
    let clear = &PVR_INTERNAL_CLEAR_ATTACH1[..];
    let gpu = HEAP_USC_BASE + off;
    if let Some(r) = gpu_words(gpu, load.len()).and_then(|w| moved_by(&w, load)) {
        return Object::Load(r);
    }
    if let Some(r) = gpu_words(gpu, clear.len()).and_then(|w| moved_by(&w, clear)) {
        return Object::Clear(r);
    }
    Object::Other
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
        if ok != 1 {
            let mut buf = [0 as c_char; 512];
            let mut n: GLsizei = 0;
            glGetShaderInfoLog(s, 512, &mut n, buf.as_mut_ptr());
            panic!("{text:?} did not compile: {}", CStr::from_ptr(buf.as_ptr()).to_string_lossy());
        }
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

fn get_int(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: a plain GL call; room for one int.
    unsafe { glGetIntegerv(pname, &mut v) };
    assert_eq!(error(), GL_NO_ERROR, "{pname:#x}");
    v
}

/// The surface address of a PBE emit (pbestate.xml STATE_WORD0/1: the low
/// bits' dword address in word 0, the high byte in word 1's low byte).
fn emit_address(w: &[u32; 2]) -> u64 {
    (u64::from(w[1] & 0xff) << 34) | (u64::from(w[0]) << 2)
}

fn pair(words: &[u32], i: usize) -> [u32; 2] {
    [words[2 * i], words[2 * i + 1]]
}

/// A four-RGBA8 draw: the one kick and its end-of-tile generation.
fn four_target_draw() -> Frame {
    fbo(&[GL_RGBA8; 4]);
    use_program(FS4);
    let f = frame_of(|| {
        // SAFETY: plain GL calls.
        unsafe { glViewport(0, 0, SIZE, SIZE) };
        draw_triangle();
        assert_eq!(error(), GL_NO_ERROR, "{:?}", logs());
    });
    assert_eq!(f.kicks.len(), 1, "{:?}", logs());
    f
}

/* ---- the shaders -------------------------------------------------------------------- */

const HEAD: &str = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
    layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n";
const WRITES: &str = "o0 = vec4(0.1); o1 = vec4(0.2); o2 = vec4(0.3); o3 = vec4(0.4);";

/// Every instance discards (v.x is a varying: the compiler cannot fold it).
fn always() -> String {
    format!("{HEAD}void main() {{ if (v.x < 1.0e30) discard; {WRITES} }}\n")
}

/// Discards where v.x < 0.5, writes four colours elsewhere.
fn conditional() -> String {
    format!("{HEAD}void main() {{ if (v.x < 0.5) discard; {WRITES} }}\n")
}

const VS2: &str = "#version 300 es\nin vec4 p;\nout vec4 v;\nvoid main() { v = p; gl_Position = p; }\n";

/// The fragment program of `src` as the back end builds it for four RGBA8
/// targets (the per-target list of four RGBA8 classes; the key-0 variant refuses a
/// discard beside several outputs: alpha-to-one): its record's output count and its bytes.
fn compile(src: &str) -> (u8, Vec<u8>) {
    use yakogl::shaderc::online::powervr::chain::{compile_fragment_targets, TargetPack};
    use yakogl::shaderc::online::sema::Stage;
    use yakogl::shaderc::online::{lower, store};
    use yakogl::shaderc::SHC_RT_RGBA8;
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).expect("front end");
        let rec = compile_fragment_targets(&l.module, &l.reflection, a, &[TargetPack::Class(SHC_RT_RGBA8); 4])
            .expect("chain");
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        (rec.num_outputs, code.iter().flat_map(|w| w.to_le_bytes()).collect())
    })
    .unwrap()
}

/// The (name, first destination) of every operation of `image`, in order.
fn ops(image: &[u8]) -> Vec<(&'static str, Option<(u32, u32)>)> {
    use yakogl::shaderc::online::powervr::decode::decode_group;
    use yakogl::shaderc::online::powervr::isa::ops_for;
    use yakogl::shaderc::online::powervr::operands::dsts;
    let mut off = 0usize;
    let mut out = Vec::new();
    while off < image.len() {
        let g = decode_group(image, off).expect("decodes");
        for (_, ph, r) in g.instrs() {
            let Some(op) = ops_for(r, ph).next() else { continue };
            out.push((op.name, g.dst.as_ref().and_then(|d| dsts(d)[0]).map(|o| (o.bank, o.index))));
        }
        off += g.total;
    }
    out
}

/// The pixel-output registers (0..3) the program writes, in order.
fn out_regs(image: &[u8]) -> Vec<u32> {
    let ops = ops(image);
    let (_, Some((bank, _))) = *ops.last().expect("a program") else { panic!("ends without a destination") };
    ops.iter().filter_map(|(_, d)| d.filter(|(b, _)| *b == bank).map(|(_, i)| i - 32)).collect()
}

/// One run on the byte model with `v.x` = `x`: the pixel outputs it wrote
/// (register -> value), and whether it ran any pixel-output write.
fn run(image: &[u8], x: f32) -> std::collections::BTreeMap<u32, u32> {
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    for c in 0..4 {
        inp.coeff[c] = x.to_bits();
    }
    let _ = usc_exec::take_pixout_writes();
    let out = usc_exec::run_binary(image, &inp, true).expect("runs");
    let w = usc_exec::take_pixout_writes();
    out.into_iter().filter(|(r, _)| w.contains(r)).collect()
}

/// A four-RGBA8 FBO cleared to four different colours, then drawn with
/// `fs`: the draw's frame.
fn draw_discarding(fs: &str) -> Frame {
    fbo(&[GL_RGBA8; 4]);
    // SAFETY: plain GL calls; four floats.
    unsafe {
        for (i, c) in [[1.0f32, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 1.0]]
            .iter()
            .enumerate()
        {
            glClearBufferfv(GL_COLOR, i as GLint, c.as_ptr());
        }
        glViewport(0, 0, SIZE, SIZE);
    }
    assert_eq!(error(), GL_NO_ERROR, "{:?}", logs());
    // the draw
    draw_with(fs)
}

/// `fs` made current under VS2, then one triangle: its frame.
fn draw_with(fs: &str) -> Frame {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS2), shader(GL_FRAGMENT_SHADER, fs));
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
        glViewport(0, 0, SIZE, SIZE);
    }
    let fr = frame_of(draw_triangle);
    assert_eq!(error(), GL_NO_ERROR, "{:?}", logs());
    fr
}

/// What the end of the draw stores: one kick over four targets, four PBE
/// words, four emits on registers 0..3 with their own surfaces, the
/// four-pair end-of-tile program.
fn assert_four_target_store(f: &Frame) {
    let k = f.kicks.last().unwrap_or_else(|| panic!("a kick: {:?}", logs()));
    assert_eq!(k.frag.num_rt, 4, "{:?}", logs());
    for (rt, w) in k.frag.pbe_word.iter().enumerate().take(4) {
        assert_ne!(*w, [0, 0], "render target {rt}: its PBE register words");
    }
    assert_eq!(k.frag.event_pixel_pds_info, 0x64, "the four-target end-of-tile program");
    let (pairs, words) = f.eot.last().expect("an end-of-tile generation");
    assert_eq!(*pairs, 4);
    assert_eq!(emits(words, 4), [(0, 0), (1, 1), (2, 2), (3, 3)]);
    let addrs: Vec<u64> = (0..4).map(|i| emit_address(&pair(words, i))).collect();
    for i in 0..4 {
        assert_ne!(addrs[i], 0);
        for j in 0..i {
            assert_ne!(addrs[i], addrs[j], "render targets {j} and {i} are different surfaces");
        }
    }
}

/// The shape every discarding program has: the kill test (a `tst` with no
/// destination), then `alphaf`, then `wdf`, then the pixel-output writes
/// (special bank, registers 32..35), each once.
fn assert_kill_then_four_writes(image: &[u8]) {
    let o = ops(image);
    let kill = o.iter().position(|(n, d)| n.starts_with("tst") && d.is_none()).expect("a kill test");
    let alphaf = o.iter().position(|(n, _)| *n == "alphaf").expect("an alphaf");
    let wdf = o.iter().rposition(|(n, _)| *n == "wdf").expect("a wdf");
    let first_write = o.iter().position(|(_, d)| d.is_some_and(|(b, i)| b == 0 && i == 32)).expect("the write of o0");
    assert!(kill < alphaf && alphaf < wdf && wdf < first_write, "kill, alphaf, wdf, writes: {o:?}");
    assert_eq!(out_regs(image), [0, 1, 2, 3], "o0-o3, each once, in registers 0..3: {o:?}");
}

/* ---- (1) a shader that always discards ---------------------------------------------------- */

#[test]
fn always_discarding_shader_keeps_the_four_writes() {
    let _g = rig(4);
    let (n, image) = compile(&always());
    assert_eq!(n, 4, "the record still spans output registers 0..=3");
    assert_kill_then_four_writes(&image);
    for x in [-1.0f32, 0.75] {
        let out = run(&image, x);
        assert_eq!(out.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3], "v.x = {x}: {out:?}");
    }
}

/* ---- (2) a conditional discard -------------------------------------------------------------- */

#[test]
fn conditional_discard_writes_four_distinct_colours() {
    let _g = rig(4);
    let (n, image) = compile(&conditional());
    assert_eq!(n, 4);
    assert_kill_then_four_writes(&image);
    for x in [-1.0f32, 0.25, 0.5, 3.0] {
        let out = run(&image, x);
        assert_eq!(out.keys().copied().collect::<Vec<_>>(), [0, 1, 2, 3], "v.x = {x}: all four outputs written");
        let vals: Vec<u32> = out.values().copied().collect();
        for i in 0..4 {
            for j in i + 1..4 {
                assert_ne!(vals[i], vals[j], "v.x = {x}: outputs {i} and {j} carry their own colours");
            }
        }
    }
}

/* ---- (3) the program and the state of the discarding shader ----------------------------------- */

/// The same four-RGBA8 draw as an ES 3.0 application issues it. The online
/// compiler's key-0 variant (the one `glCompileShader` validates with)
/// refuses any discard beside more than one colour output, so the shader
/// never compiles at the GL level, at three targets as at four: ES 3.0 §2.12
/// has no such limit. Fix: validate a discarding shader with the per-target
/// variant (`compile_fragment_targets`) the draw compiles under.
#[test]
#[ignore = "defect: glCompileShader refuses a fragment shader with discard and more than one colour output (key-0 compile: 'alpha-to-one in a shader with more than one colour output')"]
fn defect_gl_draw_of_a_discarding_four_output_shader() {
    let _g = rig(4);
    for fs in [always(), conditional()] {
        let f = draw_discarding(&fs);
        assert_eq!(f.kicks.len(), 1, "{:?}", logs());
        assert_four_target_store(&f);
        assert_eq!(f.eot.len(), 1);
    }
}

/// At three targets the same shader (three outputs) is refused the same way;
/// once the defect is fixed the draw stays at three PBE states.
#[test]
#[ignore = "defect: glCompileShader refuses a fragment shader with discard and more than one colour output (key-0 compile: 'alpha-to-one in a shader with more than one colour output')"]
fn defect_gl_draw_of_a_discarding_three_output_shader_stays_three() {
    let _g = rig(3);
    fbo(&[GL_RGBA8; 3]);
    let src = "#version 300 es\nprecision highp float;\nin vec4 v;\n\
        layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
        layout(location = 2) out vec4 o2;\n\
        void main() { if (v.x < 0.5) discard; o0 = vec4(0.1); o1 = vec4(0.2); o2 = vec4(0.3); }\n";
    let fr = draw_with(src);
    assert_eq!(fr.kicks.len(), 1, "{:?}", logs());
    assert_eq!(fr.kicks[0].frag.num_rt, 3);
    assert_eq!(fr.kicks[0].frag.pbe_word[3], [0, 0], "no fourth PBE state at three targets");
    assert_eq!(fr.eot[0].0, 3);
}
