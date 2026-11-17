// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth and stencil to and from the window (the default
//! framebuffer) through the GL API over the REAL PowerVR host back end (plan
//! item window-test-pvr; docs/notes/blit-depth-window.md, "Design: the window
//! in `depth_source` and `copy_depth`"; ES 3.0.6 4.3.3). The PowerVR twin of
//! tests/es3_blit_ds_window.rs (the GL core's window path on a stand-in back
//! end) and the window counterpart of tests/es3_blit_ds_pvr.rs (renderbuffer
//! to renderbuffer, whose platform rig is copied here: host memory for the
//! GPU's, every submit complete at once, stand-ins for the C command
//! generators).
//!
//! What this file pins that the others do not:
//!
//! * `the_table_entries_take_the_windows_id`: after the window is cleared and
//!   drawn through the GL API, `pvr_backend_depth_resolve`,
//!   `pvr_backend_stencil_resolve` and `pvr_backend_ds_write` accept the id
//!   `0x8000_0000 | (default_surface.gpu >> 12)` (the formula of gl/draw.rs)
//!   and keep the window's packed D24S8 buffer under it; a write and a read
//!   round-trip in MEMORY order. Passes today.
//! * `the_window_to_the_window_is_an_error`: ES 3.0.6 4.3.3 (the same buffer is
//!   read and written), nothing changes. Passes today.
//! * the blits between the window and a D24S8 renderbuffer, each with
//!   `GlContext::default_y_up` false (the window stored top-down, as the note
//!   reads the code: memory row r is GL row `H - 1 - r`, so the core mirrors a
//!   plane it reads and maps a cut rectangle to the memory rows `H - (y + h)
//!   .. H - y`) and true (no mirror): depth renderbuffer -> window, window ->
//!   depth renderbuffer, the same for stencil, and a D24S8 window with a
//!   depth-only, a stencil-only and a both-planes mask (the plane the mask does
//!   not name must keep its value). Both sides' planes are seeded and read
//!   back through the table's own entries. They go through the GL core's
//!   window path (`DepthSource::Window`, `PlaneDst::Window`: the gl-ds-window
//!   item).
//!
//! The window's ds buffer exists in the back end after a GL clear of the
//! window and a depth-tested draw into it, with a `glFinish` (the way the rig
//! makes a renderbuffer's buffer exist): the kick's store makes the packed
//! D24S8 buffer under the window's id live, at the ZLS-tile-aligned width (64
//! x 32 words for the 40 x 30 window). The tests then seed and read its planes
//! through the table in memory order. `default_y_up` is set on the current
//! context after that: the PowerVR back end does not look at it for a plane (a
//! plane is stored as the ISP wrote it), only the GL core does, to mirror.
//!
//! Depth is a 24-bit word (`n / 16777215`); the model works in the integer `n`
//! (exact in f32) and the planes are compared as integers.
//!
//! HOST-ONLY: the GPU's own depth writes are not seen (the fake GPU stores
//! `depth_word`), and that the PowerVR stores the window top-down is the
//! note's reading of the code; no board was used.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_blit_ds_window_pvr
//! ```
//!
//! (the feature set of tests/es3_blit_ds_pvr.rs). Not built with `report-es2` (glBlitFramebuffer
//! is an ES 3.0 call).

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
    not(feature = "no-shader-table"),
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, Once};
use yakogl::backend;

use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
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

/* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE): init refuses
 * a platform that answers another. */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

/// ISP_ZLSCTL.forcezstore: the kick stores its depth/stencil.
const ZLSCTL_FORCEZSTORE: u64 = 1 << 2;

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    /// Submits so far: the last fence.
    fence: i64,
    /// Completed through this fence while `hold` is set.
    completed_to: i64,
    /// Kicks complete only when waited for (a GPU still busy).
    hold: bool,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The ZLS-tile-aligned row width of the sources' buffers (32 pixels wide
/// = one tile, `ZLS_TILE` in src/pvr/backend.rs): the layout the ISP stores
/// at and `pvr_backend_depth_resolve` reads.
const AW: usize = 32;

/// The D24S8 word the "GPU" stores at (x, y) in the job of fence `fence`: a
/// depth that differs per texel and per job in bits 23..0, and a stencil
/// byte (bits 31..24) the depth resolve must ignore.
fn depth_word(fence: i64, x: u32, y: u32) -> u32 {
    let d = (x * 7 + y * 13 + fence as u32 * 1009) & 0x00ff_ffff;
    d | 0x3c00_0000
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

/// The "GPU": a kick that stores depth leaves `depth_word` in every word of
/// its depth/stencil buffer, rows `AW` words wide.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.fence += 1;
        let fence = p.fence;
        let zs = k.frag.isp_zload_store_base;
        let stores = zs != 0 && k.frag.isp_zlsctl & ZLSCTL_FORCEZSTORE != 0;
        let Some((&base, a)) = p.allocs.range_mut(..=zs).next_back().filter(|_| stores) else { return fence };
        let off = (zs - base) as usize;
        if off < a.size {
            let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
            // SAFETY: the words from `zs` to the allocation's end.
            let words = unsafe { core::slice::from_raw_parts_mut(first, (a.size - off) / 4) };
            for (i, w) in words.iter_mut().enumerate() {
                *w = depth_word(fence, (i % AW) as u32, (i / AW) as u32);
            }
        }
        fence
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| p.completed_to = p.completed_to.max(fence));
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| if p.hold { p.completed_to } else { p.fence })
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

/* ---- the context ------------------------------------------------------------- */

/// The default framebuffer's size.
const CANVAS: (i32, i32) = (40, 30);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut PROGRAM: GLuint = 0;

/// The one context and backend of the file, set up once; the tests take
/// turns. Each starts with every kick submitted and completed.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            assert!(!context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32).is_null(), "{:?}", logs());
            PROGRAM = program("passthrough.vert", "uniform_color.frag");
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    with_platform(|p| {
        p.completed_to = p.fence;
        p.hold = false;
        p.log.clear();
    });
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(type_: GLenum, name: &str) -> GLuint {
    let cname = std::ffi::CString::new(name).unwrap();
    // SAFETY: a NUL-terminated name; the answer is a static row or null.
    let src = unsafe { yakogl::shaderc::offline::gles_offline_shader_source(cname.as_ptr()) };
    assert!(!src.is_null(), "{name}: not in the offline shader table");
    // SAFETY: plain GL calls; `src` is a NUL-terminated string.
    unsafe {
        let s = glCreateShader(type_);
        glShaderSource(s, 1, &src, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{name}: compile failed");
        s
    }
}

fn program(vs: &str, fs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls on valid handles.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{vs} + {fs}: link failed");
        p
    }
}

/* a triangle at z = 0 covering the whole target, (x, y, z, w) per vertex */
static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static UV: [f32; 6] = [0.0, 0.0, 2.0, 0.0, 0.0, 2.0];

fn draw_tri(prog: GLuint) {
    // SAFETY: plain GL calls; the arrays are statics.
    unsafe {
        glUseProgram(prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/* ---- the window and its planes ------------------------------------------------- */

/// The window's size (the rig's canvas).
const WW: u32 = CANVAS.0 as u32;
const WH: u32 = CANVAS.1 as u32;

const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;
/// A 24-bit depth word's denominator (`n / 16777215`).
const MAXD: f32 = 16_777_215.0;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The id the back end keeps the window's depth and stencil under (the
/// formula of gl/draw.rs: 0x8000_0000 | (default_surface.gpu >> 12)).
fn window_id() -> u32 {
    let c = context::current().expect("a current context");
    assert!(c.default_surface_valid);
    0x8000_0000 | (c.default_surface.gpu >> 12) as u32
}

/// Store the window bottom-up (`true`, a pbuffer) or top-down (`false`).
fn window_y_up(up: bool) {
    let c = context::current().expect("a current context");
    assert_eq!((c.target_w, c.target_h), (WW, WH));
    c.default_y_up = up;
}

/// The memory row of the window's GL row `g` (the mirror is an involution).
fn mem_row(g: u32, y_up: bool) -> u32 {
    if y_up {
        g
    } else {
        WH - 1 - g
    }
}

fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

/// The bound framebuffer's depth/stencil cleared, then a depth-tested draw
/// whose kick the fake GPU answers by storing `depth_word`: the buffer the
/// back end keeps under the target's id is live afterwards.
fn make_ds_exist(w: i32, h: i32) {
    // SAFETY: plain GL calls; `PROGRAM` is set once in `rig`.
    unsafe {
        glViewport(0, 0, w, h);
        glClearDepthf(0.25);
        glClearStencil(0x5a);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        draw_tri(PROGRAM);
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

/// Make the window's ds buffer exist (a GL clear and a depth-tested draw of
/// the window).
fn make_window_ds() {
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    make_ds_exist(WW as i32, WH as i32);
}

/// A `w` x `h` D24S8 renderbuffer at GL_DEPTH_STENCIL_ATTACHMENT of a new
/// framebuffer, its buffer made to exist: (framebuffer, renderbuffer).
fn d24s8_fbo(w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, w, h);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    make_ds_exist(w, h);
    (fb, rb)
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

fn release(fbs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names these tests made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/* The planes, through the table's own entries, in MEMORY order, as integers: a
 * depth is the 24-bit word n (written as n / 16777215, read back rounded), a
 * stencil the byte. */

/// Write plane `plane` of object `id`, `w` x `h`, pixel (x, row) = `f(x, row)`.
fn write_plane(id: u32, plane: u32, w: u32, h: u32, f: &dyn Fn(u32, u32) -> f32) {
    let px = (0..h).flat_map(|y| (0..w).map(move |x| (x, y)));
    let r = if plane == backend::DS_PLANE_DEPTH {
        let v: Vec<f32> = px.map(|(x, y)| f(x, y) / MAXD).collect();
        // SAFETY: `h` rows of `w` floats.
        unsafe { pvr_backend_ds_write(id, plane, v.as_ptr() as *const c_void, 0, 0, w, h, w) }
    } else {
        let v: Vec<u8> = px.map(|(x, y)| f(x, y) as u8).collect();
        // SAFETY: `h` rows of `w` bytes.
        unsafe { pvr_backend_ds_write(id, plane, v.as_ptr() as *const c_void, 0, 0, w, h, w) }
    };
    assert_eq!(r, 0, "ds_write of plane {plane} of 0x{id:x}: {:?}", logs());
}

/// Read plane `plane` of object `id`, `w` x `h`, through the table.
fn read_plane(id: u32, plane: u32, w: u32, h: u32) -> Vec<f32> {
    if plane == backend::DS_PLANE_DEPTH {
        let mut v = vec![-1.0f32; (w * h) as usize];
        // SAFETY: `h` rows of `w` floats.
        let r = unsafe { pvr_backend_depth_resolve(id, v.as_mut_ptr(), w, h, w) };
        assert_eq!(r, 0, "depth_resolve of 0x{id:x}: {:?}", logs());
        v.into_iter().map(|d| (d * MAXD).round()).collect()
    } else {
        let mut v = vec![0xffu8; (w * h) as usize];
        // SAFETY: `h` rows of `w` bytes.
        let r = unsafe { pvr_backend_stencil_resolve(id, v.as_mut_ptr(), w, h, w) };
        assert_eq!(r, 0, "stencil_resolve of 0x{id:x}: {:?}", logs());
        v.into_iter().map(f32::from).collect()
    }
}

const D: u32 = backend::DS_PLANE_DEPTH;
const S: u32 = backend::DS_PLANE_STENCIL;

/* Distinct per pixel, a 24-bit n below 2M (exact through f32 and the unorm
 * round trip) and a byte that is never 0. */
fn win_depth(x: u32, r: u32) -> f32 {
    (50_000 + (r * WW + x) * 1_001) as f32
}
fn win_stencil(x: u32, r: u32) -> f32 {
    (1 + (r * WW + x) % 250) as f32
}
fn src_depth(x: u32, y: u32) -> f32 {
    (700_000 + (y * 8 + x) * 811) as f32
}
fn src_stencil(x: u32, y: u32) -> f32 {
    (10 + y * 8 + x) as f32
}
fn dst_depth(x: u32, y: u32) -> f32 {
    (1_500_000 + (y * WW + x) * 313) as f32
}
fn dst_stencil(x: u32, y: u32) -> f32 {
    (3 + (y * WW + x) % 240) as f32
}

/* ---- the model (as tests/es3_blit_ds_window.rs) -------------------------------- */

/// The source texel destination pixel `o` takes on one axis, or None when its
/// sample point is outside the source cut to `[0, n)`.
fn model_axis(o: i64, s0: i64, s1: i64, d0: i64, d1: i64, n: i64) -> Option<i64> {
    let p = s0 as f64 + (o as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64;
    let (lo, hi) = (s0.min(s1).max(0), s0.max(s1).min(n));
    let t = p.floor() as i64;
    (p >= lo as f64 && p < hi as f64).then_some(t)
}

/// The destination (GL row order, `dw` x `dh`) after the blit `s` -> `d` of
/// the `sw` x `sh` source `src` (GL row order) into a destination that held
/// `keep`, with the scissor box `scissor` (None: off).
fn model(
    src: &dyn Fn(u32, u32) -> f32,
    (sw, sh): (u32, u32),
    keep: &dyn Fn(u32, u32) -> f32,
    (dw, dh): (u32, u32),
    s: [i32; 4],
    d: [i32; 4],
    scissor: Option<[i32; 4]>,
) -> Vec<f32> {
    let mut out = Vec::new();
    for y in 0..dh {
        for x in 0..dw {
            let (xi, yi) = (x as i64, y as i64);
            let in_dst = xi >= d[0].min(d[2]) as i64
                && xi < d[0].max(d[2]) as i64
                && yi >= d[1].min(d[3]) as i64
                && yi < d[1].max(d[3]) as i64;
            let in_scissor = scissor.is_none_or(|[sx, sy, w, h]| {
                xi >= sx as i64 && xi < (sx + w) as i64 && yi >= sy as i64 && yi < (sy + h) as i64
            });
            let from = (in_dst && in_scissor)
                .then(|| {
                    let tx = model_axis(xi, s[0] as i64, s[2] as i64, d[0] as i64, d[2] as i64, sw as i64)?;
                    let ty = model_axis(yi, s[1] as i64, s[3] as i64, d[1] as i64, d[3] as i64, sh as i64)?;
                    Some((tx as u32, ty as u32))
                })
                .flatten();
            out.push(match from {
                Some((tx, ty)) => src(tx, ty),
                None => keep(x, y),
            });
        }
    }
    out
}

/// The window's memory plane (as `f` of (x, memory row)) in GL row order.
fn gl_order(f: fn(u32, u32) -> f32, y_up: bool) -> impl Fn(u32, u32) -> f32 {
    move |x, g| f(x, mem_row(g, y_up))
}

/// A GL-ordered window picture (`WW` x `WH`) as the memory-ordered plane.
fn to_memory(pic: &[f32], y_up: bool) -> Vec<f32> {
    (0..WH).flat_map(|r| (0..WW).map(move |x| (x, r))).map(|(x, r)| pic[(mem_row(r, y_up) * WW + x) as usize]).collect()
}

/// A `w` x `h` picture of `f`.
fn filled(f: &dyn Fn(u32, u32) -> f32, w: u32, h: u32) -> Vec<f32> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// One blit: the source rectangle, the destination rectangle, the scissor.
type WindowCase = ([i32; 4], [i32; 4], Option<[i32; 4]>);

/// A plane's value at (x, y).
type PlaneFn = fn(u32, u32) -> f32;

/// The blits of the window: whole, scaled, flipped past the edge with the
/// scissor, and a sub-rectangle. Every sample point of the destination
/// rectangle (cut to the target) is inside the source (a `sw` x `sh` image).
fn window_cases(sw: i32, sh: i32) -> Vec<WindowCase> {
    vec![
        ([0, 0, sw, sh], [0, 0, WW as i32, WH as i32], None),
        ([0, 0, sw, sh], [20, 14, 24, 20], None),
        ([sw, 0, 0, sh], [0, 0, WW as i32, WH as i32], Some([3, 5, 6, 4])),
        ([1, 2, sw - 1, sh - 2], [2, 3, 30, 25], None),
    ]
}

/* ---- the table takes the window's id --------------------------------------------- */

/// After the window is cleared and drawn through the GL API, the table's
/// entries keep its packed D24S8 buffer under the window's id: a probe write
/// answers 0, a write round-trips through the two resolves in memory order,
/// each plane on its own, and another window id holds nothing (1). Passes
/// today.
#[test]
fn the_table_entries_take_the_windows_id() {
    let _g = rig();
    make_window_ds();
    let wid = window_id();
    assert_ne!(wid & 0x8000_0000, 0);
    let dummy = [0.0f32; 1];
    // SAFETY: an empty rectangle reads nothing from `src`.
    let probe = |plane| unsafe { pvr_backend_ds_write(wid, plane, dummy.as_ptr() as *const c_void, 0, 0, 0, 0, 0) };
    assert_eq!(probe(D), 0, "the window's depth can be written: {:?}", logs());
    assert_eq!(probe(S), 0, "the window's stencil can be written: {:?}", logs());
    write_plane(wid, D, WW, WH, &win_depth);
    write_plane(wid, S, WW, WH, &win_stencil);
    assert_eq!(read_plane(wid, D, WW, WH), filled(&win_depth, WW, WH), "depth round trip");
    assert_eq!(read_plane(wid, S, WW, WH), filled(&win_stencil, WW, WH), "stencil round trip");
    /* a rectangle writes only itself and only its plane */
    let patch = [7.0f32 / MAXD; 6];
    // SAFETY: 2 rows of 3 floats.
    let r = unsafe { pvr_backend_ds_write(wid, D, patch.as_ptr() as *const c_void, 5, 4, 3, 2, 3) };
    assert_eq!(r, 0);
    let want: Vec<f32> = filled(&win_depth, WW, WH)
        .into_iter()
        .enumerate()
        .map(|(i, v)| {
            let (x, y) = (i as u32 % WW, i as u32 / WW);
            if (5..8).contains(&x) && (4..6).contains(&y) {
                7.0
            } else {
                v
            }
        })
        .collect();
    assert_eq!(read_plane(wid, D, WW, WH), want, "a rectangle of depth");
    assert_eq!(read_plane(wid, S, WW, WH), filled(&win_stencil, WW, WH), "stencil untouched");
    /* the id of no window holds nothing */
    let mut v = [0.0f32; 4];
    // SAFETY: 2 x 2 floats.
    assert_eq!(unsafe { pvr_backend_depth_resolve(wid ^ 1, v.as_mut_ptr(), 2, 2, 2) }, 1);
}

/* ---- window to window ------------------------------------------------------------ */

/// "If the same buffer is read and written" is GL_INVALID_OPERATION (4.3.3):
/// the window to the window, for each of depth, stencil and both, with the
/// window top-down and y-up; the window's planes stay what they were. Passes
/// today and must stay.
#[test]
fn the_window_to_the_window_is_an_error() {
    let _g = rig();
    make_window_ds();
    let wid = window_id();
    write_plane(wid, D, WW, WH, &win_depth);
    write_plane(wid, S, WW, WH, &win_stencil);
    for y_up in [false, true] {
        window_y_up(y_up);
        for mask in [DEPTH, STENCIL, DEPTH | STENCIL] {
            bind(0, 0);
            // SAFETY: plain GL call.
            unsafe { glBlitFramebuffer(0, 0, 8, 8, 8, 8, 16, 16, mask, GL_NEAREST) };
            assert_eq!(err(), GL_INVALID_OPERATION, "y_up {y_up}, mask {mask:#x}");
        }
    }
    window_y_up(false);
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(read_plane(wid, D, WW, WH), filled(&win_depth, WW, WH), "depth untouched");
    assert_eq!(read_plane(wid, S, WW, WH), filled(&win_stencil, WW, WH), "stencil untouched");
}

/* ---- the window as a source or a destination -------------------------------------- */

#[derive(Clone, Copy, Debug)]
enum Dir {
    /// An 8x8 D24S8 renderbuffer into the window.
    IntoWindow,
    /// The window into a `WW` x `WH` D24S8 renderbuffer.
    FromWindow,
}

/// One blit of `dir` per `window_cases` rectangle with the planes `mask`
/// names, for the window top-down and y-up. Both planes of both sides are
/// seeded through the table first, so the plane the mask does not name is
/// checked to keep its value. The model gives the destination's plane in GL
/// row order; the window's is compared in memory order (`to_memory`).
fn check(dir: Dir, mask: GLbitfield) {
    let _g = rig();
    make_window_ds();
    let wid = window_id();
    let (side_w, side_h) = match dir {
        Dir::IntoWindow => (8u32, 8u32),
        Dir::FromWindow => (WW, WH),
    };
    let (fb, rb) = d24s8_fbo(side_w as i32, side_h as i32);
    let rid = ds_id(rb);
    for y_up in [false, true] {
        window_y_up(y_up);
        for (s, d, scissor) in window_cases(
            match dir {
                Dir::IntoWindow => 8,
                Dir::FromWindow => WW as i32,
            },
            match dir {
                Dir::IntoWindow => 8,
                Dir::FromWindow => WH as i32,
            },
        ) {
            write_plane(wid, D, WW, WH, &win_depth);
            write_plane(wid, S, WW, WH, &win_stencil);
            match dir {
                Dir::IntoWindow => {
                    write_plane(rid, D, 8, 8, &src_depth);
                    write_plane(rid, S, 8, 8, &src_stencil);
                    bind(fb, 0);
                }
                Dir::FromWindow => {
                    write_plane(rid, D, WW, WH, &dst_depth);
                    write_plane(rid, S, WW, WH, &dst_stencil);
                    bind(0, fb);
                }
            }
            if let Some([x, y, w, h]) = scissor {
                // SAFETY: plain GL calls.
                unsafe {
                    glEnable(GL_SCISSOR_TEST);
                    glScissor(x, y, w, h);
                }
            }
            // SAFETY: plain GL calls.
            unsafe {
                glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, GL_NEAREST);
                glDisable(GL_SCISSOR_TEST);
            }
            let what = format!("{dir:?}, mask {mask:#x}, y_up {y_up}: {s:?} -> {d:?} scissor {scissor:?}");
            assert_eq!(err(), GL_NO_ERROR, "{what}: {:?}", logs());
            // SAFETY: plain GL call.
            unsafe { glFinish() };
            for (plane, bit) in [(D, DEPTH), (S, STENCIL)] {
                let (win_f, src_f, dst_f): (PlaneFn, PlaneFn, PlaneFn) = if plane == D {
                    (win_depth, src_depth, dst_depth)
                } else {
                    (win_stencil, src_stencil, dst_stencil)
                };
                let moved = mask & bit != 0;
                match dir {
                    Dir::IntoWindow => {
                        let want = if moved {
                            let pic = model(&src_f, (8, 8), &gl_order(win_f, y_up), (WW, WH), s, d, scissor);
                            to_memory(&pic, y_up)
                        } else {
                            filled(&win_f, WW, WH)
                        };
                        assert_eq!(read_plane(wid, plane, WW, WH), want, "{what}: window plane {plane}");
                        assert_eq!(
                            read_plane(rid, plane, 8, 8),
                            filled(&src_f, 8, 8),
                            "{what}: the source's plane {plane} is read only"
                        );
                    }
                    Dir::FromWindow => {
                        let want = if moved {
                            model(&gl_order(win_f, y_up), (WW, WH), &dst_f, (WW, WH), s, d, scissor)
                        } else {
                            filled(&dst_f, WW, WH)
                        };
                        assert_eq!(read_plane(rid, plane, WW, WH), want, "{what}: renderbuffer plane {plane}");
                        assert_eq!(
                            read_plane(wid, plane, WW, WH),
                            filled(&win_f, WW, WH),
                            "{what}: the window's plane {plane} is read only"
                        );
                    }
                }
            }
        }
    }
    window_y_up(false);
    release(&[fb], &[rb]);
}

#[test]
fn depth_from_a_renderbuffer_into_the_window() {
    check(Dir::IntoWindow, DEPTH);
}

#[test]
fn depth_from_the_window_into_a_renderbuffer() {
    check(Dir::FromWindow, DEPTH);
}

#[test]
fn stencil_from_a_renderbuffer_into_the_window() {
    check(Dir::IntoWindow, STENCIL);
}

#[test]
fn stencil_from_the_window_into_a_renderbuffer() {
    check(Dir::FromWindow, STENCIL);
}

/// A D24S8 window with a depth-only mask keeps its stencil, and with a
/// stencil-only mask its depth (`check` asserts the plane the mask does not
/// name is as it was), and with both masks takes both.
#[test]
fn the_d24s8_window_as_a_destination_with_each_mask() {
    check(Dir::IntoWindow, DEPTH);
    check(Dir::IntoWindow, STENCIL);
    check(Dir::IntoWindow, DEPTH | STENCIL);
}

/// The same for the window as the source of a D24S8 renderbuffer.
#[test]
fn the_d24s8_window_as_a_source_with_each_mask() {
    check(Dir::FromWindow, DEPTH);
    check(Dir::FromWindow, STENCIL);
    check(Dir::FromWindow, DEPTH | STENCIL);
}
