// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR end-of-tile emit with spilled colour targets at four draw targets
//! (item spill4-pin-pvr of plan #368; docs/notes/pvr-mrt4-budget.md 4.2;
//! defect D2 of item mesa-rt4-pvr), through the GL core and the REAL back
//! end on a host platform, under the stand-in `backend::HOST_DRAW_TARGETS`
//! at 4. The platform and the command-generator stand-ins are those of
//! tests/pvr_backend_rt4.rs.
//!
//! `mrt_layout::layout` places targets in four 32-bit output registers and
//! spills the rest to tile buffers: `Eot3Tb` (three targets, the last
//! spilled) and `Eot4Tb { spilled: 1..=3 }`. The shader compiler follows the
//! layout. The back end's end-of-tile emit, though, takes a tile buffer for
//! the LAST attachment alone (src/pvr/backend.rs, "Only the last attachment
//! can come from a tile buffer"), so with two or three spills the emit stops
//! at the first spilled target. Mesa emits every spilled target from its tile
//! buffer (imagination/vulkan/pvr_arch_cmd_buffer.c:1524-1570).
//!
//! Passing: RGBA16F x3, (2,2,2) dwords, gives three PBE states and `eot3_tb`
//! (the last emitted from a tile buffer).
//! Today: a one-spill four-target list of mixed classes is refused by name
//! (pinned), RGBA16F x4 emits two targets, and the eot4_tb3 draw is refused
//! by the compiler. `#[ignore = "defect: ..."]`: RGBA16F x4 (`eot4_tb2`) and RGBA32UI beside
//! three RGBA8 (`eot4_tb3`) should give four emits, the spilled ones from tile buffers.
//!
//! Note: item pvr-budget-fb may make glCheckFramebufferStatus answer
//! GL_FRAMEBUFFER_UNSUPPORTED for lists over the register budget. A list
//! refused on a tip is pinned as refused (`fbo` returns None, the test
//! asserts the refusal and returns) instead of drawn.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_spill_pvr
//! ```

//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_backend_rt4
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
use yakogl::pvr::mrt_layout::{layout, EotProgram, MrtLayout, Place};

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
/// None when glCheckFramebufferStatus refuses the list (pvr-budget-fb).
fn fbo(formats: &[GLenum]) -> Option<GLuint> {
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
        let st = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        if st == GL_FRAMEBUFFER_UNSUPPORTED {
            return None;
        }
        assert_eq!(st, GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    Some(fbo)
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

/// The PBE STATE_WORD1 fields of one emit: `source_pos` (the first output
/// register) and `mrt_index` (pbestate.xml).
fn source_and_index(w1: u32) -> (u32, u32) {
    ((w1 >> 14) & 3, (w1 >> 16) & 7)
}

/// The (source_pos, mrt_index) of the first `n` emits of one generation.
fn emits(words: &[u32], n: usize) -> Vec<(u32, u32)> {
    words.chunks(2).take(n).map(|p| source_and_index(p[1])).collect()
}

/* ---- four RGBA8 targets: the draw --------------------------------------------------- */

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

/* ---- the layouts ---------------------------------------------------------------- */

fn dwords(f: GLenum) -> u8 {
    match f {
        GL_RGBA32UI => 4,
        GL_RGBA16F => 2,
        _ => 1,
    }
}

/// An output for each of `formats`: `uvec4` for RGBA32UI, `vec4` else.
fn fs_for(formats: &[GLenum]) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for (i, &f) in formats.iter().enumerate() {
        let ty = if f == GL_RGBA32UI { "uvec4" } else { "vec4" };
        s += &format!("layout(location = {i}) out {ty} o{i};\n");
    }
    s += "void main() {\n";
    for (i, &f) in formats.iter().enumerate() {
        if f == GL_RGBA32UI {
            s += &format!("o{i} = uvec4({}u, 2u, 3u, 4u);\n", i % 2);
        } else {
            s += &format!("o{i} = vec4({}.0, 0.25, 0.5, 1.0);\n", i % 2);
        }
    }
    s + "}\n"
}

/// One draw into `formats`: the layout `mrt_layout::layout` gives, the
/// frame, or None when the list is refused (pvr-budget-fb).
fn draw_list(formats: &[GLenum]) -> Option<(MrtLayout, Frame)> {
    let d: Vec<u8> = formats.iter().map(|&f| dwords(f)).collect();
    let l = layout(&d).expect("a layout");
    fbo(formats)?;
    use_program(&fs_for(formats));
    let f = frame_of(|| {
        // SAFETY: plain GL call.
        unsafe { glViewport(0, 0, SIZE, SIZE) };
        draw_triangle();
    });
    assert_eq!(error(), GL_NO_ERROR, "{formats:x?}: {:?}", logs());
    Some((l, f))
}

/// The (source_pos, mrt_index) the layout gives each emit.
fn want_emits(l: &MrtLayout) -> Vec<(u32, u32)> {
    (0..l.targets as usize).map(|i| (u32::from(l.place[i].source_pos()), i as u32)).collect()
}

/// The info word the static program of `l` has in this file's platform.
fn want_info(l: &MrtLayout) -> u32 {
    match l.program {
        EotProgram::Plain { targets } => 0x60 + u32::from(targets),
        EotProgram::Eot3Tb => 0x73,
        EotProgram::Eot4Tb { .. } => 0x74,
    }
}

/// The checks a layout's frame must meet: one kick of `targets` targets,
/// every PBE state set, the program's info word, one end-of-tile generation
/// with every emit at its place.
fn check(formats: &[GLenum], l: &MrtLayout, f: &Frame) {
    let n = l.targets as usize;
    assert_eq!(f.kicks.len(), 1, "{formats:x?}: {:?}", logs());
    let k = &f.kicks[0];
    assert_eq!(k.frag.num_rt as usize, n, "{formats:x?}: emitted targets, {:?}", logs());
    for rt in 0..n {
        assert_ne!(k.frag.pbe_word[rt], [0, 0], "{formats:x?}: render target {rt}'s PBE words");
    }
    assert_eq!(k.frag.event_pixel_pds_info, want_info(l), "{formats:x?}: {:?}", l.program);
    assert_eq!(f.eot.len(), 1, "{formats:x?}");
    let (pairs, words) = &f.eot[0];
    assert_eq!(*pairs, 4);
    assert_eq!(emits(words, n), want_emits(l), "{formats:x?}: (source_pos, mrt_index) per layout");
    assert!(words[..2 * n].iter().all(|&w| w != 0), "{formats:x?}: every used pair is filled: {words:x?}");
}

/// (2,2,2) dwords: RGBA16F x3. The third spills: `eot3_tb`, three PBE
/// states, the last from tile buffer 0.
#[test]
fn rgba16f_x3_is_eot3_tb() {
    let _g = rig(4);
    let formats = [GL_RGBA16F; 3];
    let Some((l, f)) = draw_list(&formats) else { return refused_pinned(&formats) };
    assert_eq!(l.program, EotProgram::Eot3Tb);
    assert_eq!(
        l.place[..3],
        [Place::Register { first: 0 }, Place::Register { first: 2 }, Place::TileBuffer { index: 0 }]
    );
    check(&formats, &l, &f);
}

/// One spill at the last of four targets (`eot4_tb1`, RGBA16F beside three
/// RGBA8 or the other way round): the layout is as the back end's emit
/// expects (the last one spilled), but the draw never gets to the emit: the
/// draw path refuses a mix of float classes by name (GL_INVALID_OPERATION,
/// nothing kicked). Pinned as that refusal; the layout is checked.
#[test]
fn one_spill_at_the_last_of_four_is_refused_as_a_mix_today() {
    let _g = rig(4);
    let formats = [GL_RGBA16F, GL_RGBA8, GL_RGBA8, GL_RGBA8];
    let l = layout(&[2, 1, 1, 1]).unwrap();
    assert_eq!(l.program, EotProgram::Eot4Tb { spilled: 1 });
    assert_eq!(l.place[3], Place::TileBuffer { index: 0 });
    if fbo(&formats).is_none() {
        return refused_pinned(&formats);
    }
    use_program(&fs_for(&formats));
    let f = frame_of(|| {
        // SAFETY: plain GL call.
        unsafe { glViewport(0, 0, SIZE, SIZE) };
        draw_triangle();
    });
    assert_eq!(error(), GL_INVALID_OPERATION, "{:?}", logs());
    assert!(f.kicks.is_empty(), "nothing is kicked");
    assert!(logs().iter().any(|m| m.contains("beside an attachment of another class")), "{:?}", logs());
}

/// A list the back end refuses is pinned as refused: the refusal is
/// UNSUPPORTED and nothing was kicked.
fn refused_pinned(formats: &[GLenum]) {
    assert!(with_platform(|p| p.kicks.is_empty()), "{formats:x?}: refused, so not drawn");
}

/// RGBA16F x4 (2,2,2,2): `eot4_tb2`, two spills. The layout puts targets 2
/// and 3 in tile buffers 0 and 1; the end-of-tile emit must have four
/// states, the last two from tile buffers (Mesa pvr_arch_cmd_buffer.c:1524).
#[test]
#[ignore = "defect: the end-of-tile emit takes a tile buffer for the last attachment only; eot4_tb2 stops at target 2"]
fn rgba16f_x4_emits_both_spilled_targets() {
    let _g = rig(4);
    let formats = [GL_RGBA16F; 4];
    let Some((l, f)) = draw_list(&formats) else { return refused_pinned(&formats) };
    assert_eq!(l.program, EotProgram::Eot4Tb { spilled: 2 });
    assert_eq!(l.place[2..4], [Place::TileBuffer { index: 0 }, Place::TileBuffer { index: 1 }]);
    check(&formats, &l, &f);
}

/// RGBA32UI first, three RGBA8 behind it (4,1,1,1): the first fills the
/// four registers and the three after it spill (`eot4_tb3`). The draw is
/// refused today (integer colour outputs past four registers, no variant),
/// so there is nothing to emit.
#[test]
#[ignore = "defect: eot4_tb3 is chosen by mrt_layout but its draw is refused and the emit takes the last attachment only"]
fn three_spills_emit_all_three_spilled_targets() {
    let _g = rig(4);
    let formats = [GL_RGBA32UI, GL_RGBA8, GL_RGBA8, GL_RGBA8];
    let l = layout(&[4, 1, 1, 1]).unwrap();
    assert_eq!(l.program, EotProgram::Eot4Tb { spilled: 3 });
    let Some((l, f)) = draw_list(&formats) else { return refused_pinned(&formats) };
    check(&formats, &l, &f);
}
