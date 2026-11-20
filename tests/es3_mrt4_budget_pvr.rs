// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! RC14 on the PowerVR BXS-4-64: four colour targets whose formats want more
//! than the pixel's four 32-bit output registers (plan #368, item
//! pvr-budget4-test).
//!
//! The budget (docs/notes/mrt4-v3d-pvr.md section 2.3, src/pvr/mrt_layout.rs):
//! a pixel has four 32-bit output registers. An RGBA8-class target takes one,
//! RGBA16F (and sRGB) an even pair, RGBA32UI (and RGBA32F) all four; a
//! target of two registers starts at an even one and nothing back-fills a
//! left-over odd one. `mrt_layout::register_layout` refuses such a layout by
//! name; the handoff of pvr-budget-note (issue #6237) names GL_FRAMEBUFFER_-
//! UNSUPPORTED the GL answer for over-budget layouts, spilling to tile
//! buffers the back-end goal.
//!
//! Four combinations fit the budget: RGBA8 x4 (o0-o3), RGBA16F x2 (a pair
//! each), RGBA32F alone (all four), RGBA16F + RGBA8 x2 (a pair and two). The
//! four-target combinations of {RGBA8, RGBA16F, RGBA32UI, RGBA32F} that do
//! not are enumerated below as the fifteen `OVER` rows: the twelve RGBA8 +
//! RGBA16F orders that spend six or eight registers, the RGBA32UI
//! combinations (four registers a target, four targets), and the RGBA32F
//! combinations, which the GL core still cannot even make renderable (no
//! GL_EXT_color_buffer_float, plan #360). `register_layout` is the arbiter:
//! every OVER row it refuses by name, every fitting one it places.
//!
//! Today the GL core has no budget in glCheckFramebufferStatus
//! (src/gl/framebuffer.rs answers UNSUPPORTED only for pitch and
//! depth-stencil planes), so an over-budget frame is complete. The draw then
//! answers one of four ways, and the pinned `today` table holds all of
//! them:
//!
//! - an RGBA8-class frame, and RGBA16F x4, is drawn short: the back end
//!   keeps four attachments under the stand-in (item pvr-backend, plan #368;
//!   three before it) and stops emitting where the next target would pass
//!   register 3 (only the last attachment of three is emitted from a tile
//!   buffer) — GL_NO_ERROR, a short kick;
//! - RGBA16F beside RGBA8 is refused by name: the RGBA16F key's program
//!   lays every output out as RGBA16F's (issue #313's per-target build), so
//!   the back end does not store the RGBA8 targets from it —
//!   GL_INVALID_OPERATION, no kick;
//! - an integer frame (the 16-dword RGBA32UI x4 layout, RGBA32UI beside
//!   RGBA8) never reaches the back end: the online compiler has no variant
//!   for it (`mrt_layout` refuses the four-RGBA32UI walk), so the GL core
//!   drops the draw as a missing variant — counted, logged, no GL error, no
//!   kick;
//! - an RGBA32F attachment is not renderable at all (plan #360):
//!   GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT and GL_INVALID_FRAMEBUFFER_-
//!   OPERATION for draw and clear.
//!
//! A clear of a fitting float frame reaches every target today (its frame
//! runs the back end's own program and emits all attachments on their
//! budget registers); a GL program's draw reaches both of RGBA16F x2 and is
//! refused for RGBA16F + RGBA8 x2 (second family above).
//! The `#[ignore = "defect: ..."]` tests hold the recommendation: over-budget
//! combinations answer UNSUPPORTED, and the fitting combinations draw and
//! clear every target. No out-of-range register is written: the emitted
//! sources of every over-budget kick are checked to land inside registers
//! 0-3, and no kick names a target the budget leaves no room for.
//!
//! The GL calls run against the REAL PowerVR back end on the host platform
//! of tests/es3_mrt4_formats_pvr.rs (cut from it): GPU memory is host
//! memory, the C command generators are stand-ins, and the pixel-event
//! generator records the PBE STATE word pair of every end-of-tile program
//! (pack mode, norm, source format, `source_pos`, `mrt_index`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_mrt4_budget_pvr
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

use yakogl::backend;
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::{glClearBufferfv, glClearBufferuiv};
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::render::{PvrRenderKick, PvrRenderStatic, PVR_RENDER_EOT_MAX_RT, PVR_RENDER_EOT_PDS_PAIRS};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OSAL_QUERY_RENDER_STATIC};
use yakogl::pvr::backend::*;
use yakogl::pvr::ffi::*;
use yakogl::pvr::mrt_layout::{register_layout, LayoutError};

/* ---- the platform ------------------------------------------------------------ */

/* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE): init refuses
 * a platform that answers another. */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    #[allow(dead_code)]
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// The PBE STATE word pairs of each end-of-tile program built, one Vec per
/// call of the pixel-event generator (per kick).
static EMITS: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());
/// The kick's `num_rt` of every submit.
static KICKS: Mutex<Vec<u32>> = Mutex::new(Vec::new());

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
        p.allocs.insert(gpu, Alloc { mem });
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

/// The "GPU": records the kick's `num_rt` and completes it.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).push(k.frag.num_rt);
    with_platform(|p| {
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

/// Records the PBE STATE word pairs of each end-of-tile program built.
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
    // SAFETY: the backend's emit words, buffers and sizes.
    unsafe {
        EMITS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(core::slice::from_raw_parts(emit_words, 2 * num_rt as usize).to_vec());
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the context ------------------------------------------------------------- */

const CANVAS: i32 = 16;
/// The target's size: a whole tile, so the stride is the width.
const SIZE: i32 = 16;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The colour class a fragment program of a row's targets must write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Float,
    Uint,
}

/// `PROGRAMS[0..4]`: float programs of one to four `vec4` outputs;
/// `PROGRAMS[4]`: four `uvec4` outputs (the RGBA32UI rows).
static mut PROGRAMS: [GLuint; 5] = [0; 5];

const VS: &CStr = c"#version 300 es
layout(location = 0) in vec4 pos;
void main() { gl_Position = pos; }
";

/// `n` float outputs, each written its own constant.
fn fs_float(n: usize) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\n");
    for i in 0..n {
        s += &format!("layout(location = {i}) out vec4 o{i};\n");
    }
    s += "void main() {\n";
    for i in 0..n {
        s += &format!("    o{i} = vec4({}.0, 0.0, 0.0, 1.0);\n", i + 1);
    }
    s += "}\n";
    s
}

fn fs_uint() -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for i in 0..4 {
        s += &format!("layout(location = {i}) out uvec4 o{i};\n");
    }
    s += "void main() {\n";
    for i in 0..4 {
        s += &format!("    o{i} = uvec4({}u, 2u, 3u, 4u);\n", i + 1);
    }
    s += "}\n";
    s
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(type_: GLenum, src: &CStr) -> GLuint {
    let p = src.as_ptr();
    // SAFETY: plain GL calls; `p` is a NUL-terminated string.
    unsafe {
        let s = glCreateShader(type_);
        glShaderSource(s, 1, &p, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "shader compile failed: {:?}", logs());
        s
    }
}

/// The one context and backend of the file, set up once; the tests take
/// turns. The four-target stand-in is on for the whole file.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS as u32, CANVAS as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
            let v = shader(GL_VERTEX_SHADER, VS);
            let mut links: Vec<(GLuint, usize)> = Vec::new();
            for n in 1..=4 {
                let src = std::ffi::CString::new(fs_float(n)).unwrap();
                links.push((shader(GL_FRAGMENT_SHADER, &src), n - 1));
            }
            let src = std::ffi::CString::new(fs_uint()).unwrap();
            links.push((shader(GL_FRAGMENT_SHADER, &src), 4));
            for (i, (f, slot)) in links.into_iter().enumerate() {
                let p = glCreateProgram();
                glAttachShader(p, v);
                glAttachShader(p, f);
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                assert_eq!(ok, 1, "program {i}: link failed: {:?}", logs());
                PROGRAMS[slot] = p;
            }
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/* ---- the rows ----------------------------------------------------------------- */

const A8: GLenum = GL_RGBA8;
const F16: GLenum = GL_RGBA16F;
const U4: GLenum = GL_RGBA32UI;
const F4: GLenum = GL_RGBA32F;

/// The output registers a format's target occupies (the `surf_out_dwords`
/// rule of src/pvr/backend.rs: an RGBA8-class target one, RGBA16F and sRGB
/// two, a 32-bit integer one and RGBA32F four).
fn regs_of(fmt: GLenum) -> u32 {
    match fmt {
        A8 => 1,
        F16 => 2,
        _ => 4,
    }
}

fn class_of(fmts: &[GLenum]) -> Class {
    if fmts.iter().all(|f| *f == U4) {
        Class::Uint
    } else {
        Class::Float
    }
}

/// A framebuffer of `SIZE`-square renderbuffers of the row's formats, all
/// its attachments drawn, bound; glCheckFramebufferStatus and the first GL
/// error of building it.
struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
    status: GLenum,
    build_err: GLenum,
}

impl Fbo {
    fn new(fmts: &[GLenum]) -> Fbo {
        let n = fmts.len();
        let mut fb = 0;
        let mut rbs = vec![0; n];
        // SAFETY: room for the names; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(n as i32, rbs.as_mut_ptr());
            for (i, (&fmt, &rb)) in fmts.iter().zip(rbs.iter()).enumerate() {
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, fmt, SIZE, SIZE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
            }
            let bufs: Vec<GLuint> = (0..n).map(|i| GL_COLOR_ATTACHMENT0 + i as u32).collect();
            glDrawBuffers(n as i32, bufs.as_ptr());
            let build_err = glGetError();
            Fbo { fb, rbs, status: glCheckFramebufferStatus(GL_FRAMEBUFFER), build_err }
        }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names made by `new`.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as i32, self.rbs.as_ptr());
            glFinish();
        }
    }
}

/* ---- what the hardware is handed ------------------------------------------------ */

/// The PBE STATE word pair of one emit, decoded (pbestate.xml STATE_WORD1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Emit {
    packmode: u32,
    norm: u32,
    source_format: u32,
    /// The first output register of the source (`source_pos`, in dwords).
    source_pos: u32,
    mrt_index: u32,
}

fn decode(w1: u32) -> Emit {
    Emit {
        packmode: (w1 >> 7) & 0x3f,
        norm: (w1 >> 13) & 1,
        source_format: (w1 >> 19) & 1,
        source_pos: (w1 >> 14) & 3,
        mrt_index: (w1 >> 16) & 7,
    }
}

/* PBE packmodes and source formats (csbgen/rogue/pbestate.xml) */
const PACKMODE_U8U8U8U8: u32 = 0;
const PACKMODE_F16F16F16F16: u32 = 10;
const SOURCE_F16_PER_CHANNEL: u32 = 0;
const SOURCE_8_PER_CHANNEL: u32 = 1;

/// The PBE pair an RGBA8 target on register `reg` is handed.
fn rgba8_emit(reg: u32) -> Emit {
    Emit { packmode: PACKMODE_U8U8U8U8, norm: 1, source_format: SOURCE_8_PER_CHANNEL, source_pos: reg, mrt_index: reg }
}

/// The PBE state the back end hands attachment `i` of a kicked frame today:
/// format `fmt`'s own packing, on output register `pos`, as the `mrt`-th
/// emitted source of the end-of-tile program.
fn emit_want(fmt: GLenum, pos: u32, mrt: u32) -> Emit {
    let (packmode, norm, source_format) = match fmt {
        F16 => (PACKMODE_F16F16F16F16, 0, SOURCE_F16_PER_CHANNEL),
        _ => (PACKMODE_U8U8U8U8, 1, SOURCE_8_PER_CHANNEL),
    };
    Emit { packmode, norm, source_format, source_pos: pos, mrt_index: mrt }
}

/// The expected emitted list of a frame whose targets 0..pos.len() reach the
/// hardware at registers `pos` (each as the next `mrt_index`): the emit loop
/// numbers its states by emission, not by attachment.
fn want_emits(fmts: &[GLenum], pos: &[u32]) -> Vec<Emit> {
    pos.iter().enumerate().map(|(i, &p)| emit_want(fmts[i], p, i as u32)).collect()
}

/// What one flush's worth of GL calls handed the hardware.
#[derive(Debug)]
struct Obs {
    /// The GL error of the calls (draw, clear) and of the flush.
    err: GLenum,
    /// `num_rt` of every kick submitted.
    kicks: Vec<u32>,
    /// The PBE STATE pairs (the non-zero ones) of every end-of-tile program.
    emits: Vec<Vec<Emit>>,
    /// The log lines of the calls.
    log: Vec<String>,
}

impl Obs {
    /// The one kick and program of the call, or a panic naming the row.
    fn one(&self, what: &str) -> (u32, &[Emit]) {
        assert_eq!((self.kicks.len(), self.emits.len()), (1, 1), "{what}: {:?}", self.log);
        (self.kicks[0], &self.emits[0])
    }
}

/// Runs `body`, flushes, and reports what the hardware got and what GL said.
fn observed(body: impl FnOnce()) -> Obs {
    EMITS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).clear();
    with_platform(|p| p.log.clear());
    let mut err;
    // SAFETY: plain GL calls.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        body();
        err = glGetError();
        glFlush();
        glFinish();
        if err == GL_NO_ERROR {
            err = glGetError();
        }
    }
    let kicks = KICKS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    let emits = EMITS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|w| w.chunks(2).filter(|p| p[0] != 0 && p[1] != 0).map(|p| decode(p[1])).collect())
        .collect();
    Obs { err, kicks, emits, log: logs() }
}

/// A triangle over the bound framebuffer with the program of `class` for `n`
/// outputs.
fn drawn(n: usize, class: Class) -> Obs {
    // SAFETY: the programs are built once in `rig`; the lock serialises.
    let prog = unsafe {
        match class {
            Class::Float => PROGRAMS[n - 1],
            Class::Uint => PROGRAMS[4],
        }
    };
    // SAFETY: plain GL calls; POS is a static.
    observed(|| unsafe {
        glViewport(0, 0, SIZE, SIZE);
        glUseProgram(prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    })
}

/// `glClearBufferfv/uiv` of attachment `target` over the bound framebuffer.
fn cleared(target: usize, class: Class) -> Obs {
    let v = [0.25f32, 0.5, 0.75, 1.0];
    let u = [1u32, 2, 3, 4];
    // SAFETY: plain GL calls.
    observed(|| unsafe {
        match class {
            Class::Float => glClearBufferfv(GL_COLOR, target as i32, v.as_ptr()),
            Class::Uint => glClearBufferuiv(GL_COLOR, target as i32, u.as_ptr()),
        }
    })
}

/* ---- the over-budget table ------------------------------------------------------ */

/// Today's answer to an over-budget combination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Today {
    /// Complete; the draw kicks, its targets are emitted at these output
    /// registers (`source_pos`; each as the next `mrt_index`), and the rest
    /// of the frame is dropped: the back end stops emitting where the next
    /// target would pass register 3 (the glog_once line naming it
    /// fires once for the process, so it is not asserted per row).
    Emitted(&'static [u32]),
    /// Complete, but the draw is refused before the back end sees it: the
    /// online compiler has no fragment variant for a multi-output program
    /// of the frame's class (an integer variant, or the 16-dword
    /// four-RGBA32UI layout `mrt_layout` refuses). A
    /// missing variant is counted (yakogl_draws_refused), logged, and
    /// raises no GL error (src/gl/draw.rs). No kick, so nothing out of
    /// range: attachment 0 of the frame decides, hence the RGBA8-class
    /// rows above kick short while these disappear.
    NoVariant,
    /// Complete, attachment 0 is RGBA16F and another attachment RGBA8: the
    /// GL core keys the draw with attachment 0's class, the online compiler
    /// lays every output out as RGBA16F's (one target of that class each,
    /// `chain::build_fragment_with`), and the back end refuses the draw by
    /// name rather than store the RGBA8 targets from registers holding
    /// halves (`GL_INVALID_OPERATION`, no kick; the glog_once line fires
    /// once for the process, so it is not asserted per row). A mix of float
    /// classes has no key of its own yet.
    MixRefused,
    /// The RGBA32F storage is refused (`GL_INVALID_ENUM`), the framebuffer
    /// is `GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT` and the draw is
    /// `GL_INVALID_FRAMEBUFFER_OPERATION` (plan #360: not colour-renderable
    /// without GL_EXT_color_buffer_float).
    NotRenderable,
}

/// A four-target combination over the budget, and today's answer to it:
/// the twelve RGBA8 + RGBA16F orders of six and eight registers, the
/// RGBA32UI combinations (a target of four registers), and the RGBA32F ones.
/// `Emitted` lists the `source_pos` of the targets the back end emits today;
/// every row passes register 3 by its fourth target at the latest, so the
/// lists are the same with the back end's four-target clamp (item
/// pvr-backend) as they were with the three-target one before it.
struct Over {
    label: &'static str,
    fmts: [GLenum; 4],
    today: Today,
}

use Today::*;

const OVER: [Over; 15] = [
    Over { label: "RGBA8 x3 + RGBA16F", fmts: [A8, A8, A8, F16], today: Emitted(&[0, 1, 2]) },
    Over { label: "RGBA8 x2 + RGBA16F + RGBA8", fmts: [A8, A8, F16, A8], today: Emitted(&[0, 1, 2]) },
    Over { label: "RGBA8 + RGBA16F + RGBA8 x2", fmts: [A8, F16, A8, A8], today: Emitted(&[0, 2]) },
    Over { label: "RGBA16F + RGBA8 x3", fmts: [F16, A8, A8, A8], today: MixRefused },
    Over { label: "RGBA8 x2 + RGBA16F x2", fmts: [A8, A8, F16, F16], today: Emitted(&[0, 1, 2]) },
    Over { label: "RGBA16F + RGBA8 + RGBA16F + RGBA8", fmts: [F16, A8, F16, A8], today: MixRefused },
    Over { label: "RGBA16F x2 + RGBA8 x2", fmts: [F16, F16, A8, A8], today: MixRefused },
    Over { label: "RGBA8 + RGBA16F x3", fmts: [A8, F16, F16, F16], today: Emitted(&[0, 2]) },
    Over { label: "RGBA16F + RGBA8 x2 + RGBA16F", fmts: [F16, A8, A8, F16], today: MixRefused },
    Over { label: "RGBA16F x3 + RGBA8", fmts: [F16, F16, F16, A8], today: MixRefused },
    Over { label: "RGBA16F x4", fmts: [F16, F16, F16, F16], today: Emitted(&[0, 2]) },
    Over { label: "RGBA32UI x4", fmts: [U4, U4, U4, U4], today: NoVariant },
    Over { label: "RGBA32UI + RGBA8 x3", fmts: [U4, A8, A8, A8], today: NoVariant },
    Over { label: "RGBA8 x3 + RGBA32F", fmts: [A8, A8, A8, F4], today: NotRenderable },
    Over { label: "RGBA32F + RGBA8 x3", fmts: [F4, A8, A8, A8], today: NotRenderable },
];

/// The four-target combinations the budget leaves room for.
#[derive(Clone, Copy, Debug)]
struct Fit {
    label: &'static str,
    fmts: &'static [GLenum],
    /// The registers the back end hands the targets (what `register_layout`
    /// and the back end's emit loop agree on).
    regs: &'static [u32],
}

const FITS: [Fit; 4] = [
    Fit { label: "RGBA8 x4", fmts: &[A8, A8, A8, A8], regs: &[0, 1, 2, 3] },
    Fit { label: "RGBA16F x2", fmts: &[F16, F16], regs: &[0, 2] },
    Fit { label: "RGBA32F", fmts: &[F4], regs: &[0] },
    Fit { label: "RGBA16F + RGBA8 x2", fmts: &[F16, A8, A8], regs: &[0, 2, 3] },
];

/* ---- the tests ------------------------------------------------------------------- */

/// The stand-in gives the GL core four draw buffers, and every OVER row is a
/// layout `mrt_layout::register_layout` refuses by name (past the fourth
/// output register), while every FITS row it places on `regs`.
#[test]
fn the_standin_offers_four_targets_and_the_table_matches_the_layout_rule() {
    let _g = rig();
    let mut db = 0;
    // SAFETY: plain GL call.
    unsafe { glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut db) };
    assert_eq!(db, 4);
    assert_eq!(PVR_RENDER_EOT_PDS_PAIRS, 3);
    assert_eq!(PVR_RENDER_EOT_MAX_RT, 4);
    for row in OVER {
        let dwords: Vec<u8> = row.fmts.iter().map(|f| regs_of(*f) as u8).collect();
        assert!(
            matches!(register_layout(&dwords), Err(LayoutError::OutputRegs { .. })),
            "{}: {dwords:?} is not past the four registers for register_layout",
            row.label
        );
    }
    for row in FITS {
        let dwords: Vec<u8> = row.fmts.iter().map(|f| regs_of(*f) as u8).collect();
        let l = register_layout(&dwords).unwrap_or_else(|e| panic!("{}: {dwords:?}: {e:?}", row.label));
        assert_eq!(l.tile_buffers, 0, "{}", row.label);
        let pos: Vec<u32> = (0..row.fmts.len()).map(|i| u32::from(l.place[i].source_pos())).collect();
        assert_eq!(pos, row.regs, "{}", row.label);
    }
}

/// An over-budget frame's `glCheckFramebufferStatus` today: no row answers
/// `GL_FRAMEBUFFER_UNSUPPORTED` — the combinations draw (COMPLETE) or fail
/// for another reason (RGBA32F not renderable).
#[test]
fn over_budget_framebuffer_statuses_today() {
    let _g = rig();
    for row in OVER {
        let f = Fbo::new(&row.fmts);
        match row.today {
            NotRenderable => {
                assert_eq!(f.build_err, GL_INVALID_ENUM, "{}: storage", row.label);
                assert_eq!(f.status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{}", row.label);
            }
            _ => {
                assert_eq!(f.build_err, GL_NO_ERROR, "{}: building", row.label);
                assert_eq!(f.status, GL_FRAMEBUFFER_COMPLETE, "{}", row.label);
            }
        }
    }
}

/// The draw of every over-budget combination today: no panic and no
/// out-of-range register anywhere.
///
/// - `Emitted`: GL_NO_ERROR, one kick whose PBE states sit exactly at the
///   listed registers, each numbered by emission.
/// - `MixRefused`: GL_INVALID_OPERATION, no kick (refused by name).
/// - `NoVariant`: no kick and no GL error (the missing variant is refused
///   and counted, src/gl/draw.rs; the refusal's log line is a glog_once for
///   the process, so it is not asserted here).
/// - `NotRenderable`: GL_INVALID_FRAMEBUFFER_OPERATION, no kick.
#[test]
fn over_budget_draws_today() {
    let _g = rig();
    for row in OVER {
        let _f = Fbo::new(&row.fmts);
        let obs = drawn(4, class_of(&row.fmts));
        match row.today {
            Emitted(pos) => {
                assert_eq!(obs.err, GL_NO_ERROR, "{}: {:?}", row.label, obs.log);
                let (num_rt, emits) = obs.one(row.label);
                assert_eq!(num_rt as usize, pos.len(), "{}: kick num_rt: {obs:?}", row.label);
                assert_eq!(emits, want_emits(&row.fmts, pos), "{}: PBE states: {obs:?}", row.label);
            }
            NoVariant => {
                assert_eq!(obs.err, GL_NO_ERROR, "{}: {:?}", row.label, obs.log);
                assert!(obs.kicks.is_empty() && obs.emits.is_empty(), "{}: no kick: {obs:?}", row.label);
            }
            MixRefused => {
                assert_eq!(obs.err, GL_INVALID_OPERATION, "{}: {:?}", row.label, obs.log);
                assert!(obs.kicks.is_empty() && obs.emits.is_empty(), "{}: no kick: {obs:?}", row.label);
            }
            NotRenderable => {
                assert_eq!(obs.err, GL_INVALID_FRAMEBUFFER_OPERATION, "{}: {:?}", row.label, obs.log);
                assert!(obs.kicks.is_empty(), "{}: no kick: {obs:?}", row.label);
            }
        }
    }
}

/// No over-budget kick writes past the fourth output register: the emitted
/// sources of every row occupy disjoint, in-range spans (an RGBA16F source
/// covers registers `source_pos` and `source_pos + 1`), so no PBE store can
/// read a fifth one.
#[test]
fn over_budget_emits_stay_within_four_output_registers() {
    let _g = rig();
    for row in OVER {
        let Today::Emitted(pos) = row.today else { continue };
        let f = Fbo::new(&row.fmts);
        assert_eq!(f.status, GL_FRAMEBUFFER_COMPLETE, "{}", row.label);
        let obs = drawn(4, class_of(&row.fmts));
        let (_, emits) = obs.one(row.label);
        let mut used = [false; 4];
        for (i, e) in emits.iter().enumerate() {
            assert_eq!(e.source_pos, pos[i], "{}: source of target {i}", row.label);
            for r in e.source_pos..e.source_pos + regs_of(row.fmts[i]) {
                assert!(r < 4, "{}: target {i} reads register {r}", row.label);
                assert!(!used[r as usize], "{}: register {r} used twice", row.label);
                used[r as usize] = true;
            }
        }
    }
}

/* ---- the fitting combinations ------------------------------------------------------ */

/// All three renderable fits are complete framebuffers today (RGBA32F
/// alone: `rgba32f_alone_is_not_renderable_today`).
#[test]
fn fitting_combinations_are_complete_today() {
    let _g = rig();
    for row in FITS.iter().filter(|r| r.fmts != [F4]) {
        let f = Fbo::new(row.fmts);
        assert_eq!((f.build_err, f.status), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{}", row.label);
    }
}

/// The two float fits that start on RGBA16F (RGBA16F x2 and RGBA16F + RGBA8
/// x2): the frame is complete and every per-buffer clear reaches every
/// target — the clear frame runs the back end's own programs and needs no
/// shader variant: it emits all the frame's targets on the budget registers
/// (`regs` of the row, each source numbered by emission). Since item
/// pvr-backend (plan #368) the clear of one buffer is a quad of that
/// buffer's own over the frame's load of the others (`clear_target_mask`
/// honoured); before it each buffer took the clear with the others.
/// The draw with a GL program: RGBA16F x2 draws both targets on those
/// registers (the online compiler builds the RGBA16F key of a two-output
/// shader per target, issue #313); RGBA16F + RGBA8 x2 is refused by name
/// (the `MixRefused` rows above: a mix of float classes has no key yet,
/// `fitting_float_combinations_draw_every_target`).
#[test]
fn fitting_float_combinations_clear_every_target_today() {
    let _g = rig();
    for row in FITS.iter().filter(|r| r.fmts[0] == F16) {
        let _f = Fbo::new(row.fmts);
        for k in 0..row.fmts.len() {
            let c = cleared(k, class_of(row.fmts));
            assert_eq!(c.err, GL_NO_ERROR, "{}: clear {k}: {:?}", row.label, c.log);
            let (num_rt, emits) = c.one(&format!("{}: clear {k}", row.label));
            assert_eq!(num_rt as usize, row.fmts.len(), "{}: clear {k}: {c:?}", row.label);
            assert_eq!(emits, want_emits(row.fmts, row.regs), "{}: clear {k} emits: {c:?}", row.label);
        }
        let obs = drawn(row.fmts.len(), class_of(row.fmts));
        if row.fmts.iter().all(|f| *f == F16) {
            assert_eq!(obs.err, GL_NO_ERROR, "{}: {:?}", row.label, obs.log);
            let (num_rt, emits) = obs.one(row.label);
            assert_eq!(num_rt as usize, row.fmts.len(), "{}: {obs:?}", row.label);
            assert_eq!(emits, want_emits(row.fmts, row.regs), "{}: PBE states: {obs:?}", row.label);
        } else {
            assert_eq!(obs.err, GL_INVALID_OPERATION, "{}: {:?}", row.label, obs.log);
            assert!(obs.kicks.is_empty(), "{}: refused, no kick: {obs:?}", row.label);
        }
    }
}

/// RGBA32F alone today: the renderbuffer storage is refused (GL_INVALID_-
/// ENUM), the framebuffer is incomplete and the draw raises
/// GL_INVALID_FRAMEBUFFER_OPERATION (plan #360: not colour-renderable
/// without GL_EXT_color_buffer_float).
#[test]
fn rgba32f_alone_is_not_renderable_today() {
    let _g = rig();
    let f = Fbo::new(&[F4]);
    assert_eq!(f.build_err, GL_INVALID_ENUM);
    assert_eq!(f.status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT);
    let obs = drawn(1, Class::Float);
    assert_eq!(obs.err, GL_INVALID_FRAMEBUFFER_OPERATION, "{:?}", obs.log);
    assert!(obs.kicks.is_empty());
    let c = cleared(0, Class::Float);
    assert_eq!(c.err, GL_INVALID_FRAMEBUFFER_OPERATION, "clear: {:?}", c.log);
    assert!(c.kicks.is_empty());
}

/* ---- defects: what the handoff recommends ------------------------------------------- */

/// The recommendation of pvr-budget-note (issue #6237, docs/notes/
/// mrt4-v3d-pvr.md section 2.3): GL_FRAMEBUFFER_UNSUPPORTED is the GL answer
/// for an over-budget layout, and a draw to such a frame is refused as an
/// incomplete framebuffer. Spilling the targets past the budget to tile
/// buffers (eot3_tb, eot4_tb, eot4_tb2, eot4_tb3 of mrt_layout) is the
/// back-end goal that will replace UNSUPPORTED layout by layout.
#[test]
#[ignore = "defect: plan #368 pvr-budget4: glCheckFramebufferStatus has no output-register budget (src/gl/framebuffer.rs answers UNSUPPORTED for pitch and depth-stencil planes only) and the back end draws over-budget layouts short. The fix is the budget check the note recommends (GL core completeness plus a PowerVR answer) naming GL_FRAMEBUFFER_UNSUPPORTED; the spill programs of pvr-pack replace it per layout once pvr-backend wires them"]
fn over_budget_combinations_answer_framebuffer_unsupported() {
    let _g = rig();
    for row in OVER {
        let f = Fbo::new(&row.fmts);
        assert_eq!(f.status, GL_FRAMEBUFFER_UNSUPPORTED, "{}", row.label);
        let obs = drawn(4, class_of(&row.fmts));
        assert_eq!(obs.err, GL_INVALID_FRAMEBUFFER_OPERATION, "{}: {:?}", row.label, obs.log);
        assert!(obs.kicks.is_empty(), "{}: {obs:?}", row.label);
    }
}

/// Four RGBA8 targets fit the budget: the draw gives attachment 3 its PBE
/// state on register 3 and every per-buffer clear reaches all four targets
/// (item pvr-backend, plan #368; before it the draw and every clear kicked
/// three PBE states, attachment 3 dropped by the clamp, silently).
#[test]
fn four_rgba8_targets_reach_the_hardware() {
    let _g = rig();
    let f = Fbo::new(&[A8, A8, A8, A8]);
    assert_eq!(f.status, GL_FRAMEBUFFER_COMPLETE);
    let obs = drawn(4, Class::Float);
    assert_eq!(obs.err, GL_NO_ERROR, "{:?}", obs.log);
    let (num_rt, emits) = obs.one("four RGBA8");
    assert_eq!(num_rt, 4);
    assert_eq!(emits, [rgba8_emit(0), rgba8_emit(1), rgba8_emit(2), rgba8_emit(3)]);
    for k in 0..4 {
        let c = cleared(k, Class::Float);
        assert_eq!(c.err, GL_NO_ERROR, "clear {k}: {:?}", c.log);
        let (num_rt, _) = c.one(&format!("four RGBA8, clear {k}"));
        assert_eq!(num_rt, 4, "clear {k} reaches all four targets");
    }
}

/// RGBA16F x2 and RGBA16F + RGBA8 x2 fit the budget (a pair, and a pair
/// with two singles): a GL program draws them with every target emitted on
/// its budget register, as the back end's own clear frame already does.
#[test]
#[ignore = "defect: plan #388 fix-float-mix: RGBA16F + RGBA8 x2 is keyed with attachment 0's class (0x40088621), whose program lays every output out as RGBA16F's, so the back end refuses the draw by name (RGBA16F x2 draws since issue #313's per-target RGBA16F key, pinned in fitting_float_combinations_clear_every_target_today). The fix is a per-target key for a mix of float classes, as the integer mixes have (docs/notes/pvr-int-mixed-targets.md)"]
fn fitting_float_combinations_draw_every_target() {
    let _g = rig();
    for row in FITS.iter().filter(|r| r.fmts[0] == F16) {
        let _f = Fbo::new(row.fmts);
        let obs = drawn(row.fmts.len(), class_of(row.fmts));
        assert_eq!(obs.err, GL_NO_ERROR, "{}: {:?}", row.label, obs.log);
        let (num_rt, emits) = obs.one(row.label);
        assert_eq!(num_rt as usize, row.fmts.len(), "{}: {obs:?}", row.label);
        assert_eq!(emits, want_emits(row.fmts, row.regs), "{}: PBE states: {obs:?}", row.label);
    }
}

/// RGBA32F alone fits the budget (it is the four registers): complete, drawn
/// with one PBE state, cleared.
#[test]
#[ignore = "defect: plan #360 (via pvr-budget4-test): GL_RGBA32F is not colour-renderable without GL_EXT_color_buffer_float (src/gl/framebuffer.rs is_color_format), so even a lone RGBA32F target is INCOMPLETE_ATTACHMENT. The fix is #360's float-target work making it renderable; the budget then leaves it the four registers as mrt_layout already gives it"]
fn rgba32f_alone_is_complete_and_drawn() {
    let _g = rig();
    let f = Fbo::new(&[F4]);
    assert_eq!((f.build_err, f.status), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE));
    let obs = drawn(1, Class::Float);
    assert_eq!(obs.err, GL_NO_ERROR, "{:?}", obs.log);
    let (num_rt, emits) = obs.one("RGBA32F");
    assert_eq!(num_rt, 1);
    assert_eq!(emits.len(), 1);
    let c = cleared(0, Class::Float);
    assert_eq!(c.err, GL_NO_ERROR, "clear: {:?}", c.log);
    assert_eq!(c.one("RGBA32F, clear").0, 1);
}
