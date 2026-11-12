// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! How the GL core (`draw_instanced`, src/gl/draw.rs) splits an instanced
//! draw on the REAL PowerVR table (`yakogl_backend_pvr`) as it is before P3
//! (docs/notes/instanceid-per-draw.md, issue #18). The PowerVR reports
//! `max_instance_divisor` 1 (its PDS fetches a per-instance array by the
//! instance number), so:
//!
//!  1. a draw whose per-instance arrays all have divisor 0 or 1 reaches the
//!     back end as ONE instanced draw, the divisor-1 arrays fetched per
//!     instance, and the back end emits one index list of every instance;
//!  2. any divisor of 2 or more (2, 3 and 4 in one draw; a mixed 1/2 draw)
//!     makes the core draw instance by instance: N back-end draws, each with
//!     its `instance_id`, every per-instance array (divisor 1 included) fed
//!     as a constant holding its element `instance / divisor`, and one index
//!     list of one instance each;
//!  3. a divisor of 0x10000 takes the same per-instance split.
//!
//! After P3 (hardware divisors up to 0xffff behind the cargo feature
//! `pvr-instance-divisor`, `max_instance_divisor` 0xffff) case 2 becomes one
//! draw for every divisor <= 0xffff, so its tests change with P3; case 3 is
//! the documented fallback above the cap and must stay as it is
//! (tests/es3_instanced_divisor_cap.rs pins the same split on a 0xffff
//! table).
//!
//! The draws are recorded at the table's `draw` entry: the table here is
//! `yakogl_backend_pvr` with `draw` replaced by a recorder (struct update
//! from the public table, so no `YakoglBackend` field but `draw` is listed)
//! that calls the real `pvr_backend_draw` and notes the `GlesDraw` and its
//! answer. Recording the table's draw was simpler than telling the GL
//! core's draws apart from the back end's kicks through the stand-ins. The
//! back end runs on a platform written here (GPU memory is host memory,
//! every submit completes at once), and its C command generators
//! (driver/src/pvr) are stand-ins that write words of the right count, as
//! tests/depth_resolve_pvr_backend.rs and src/pvr/backend_tests.rs do: an
//! integration test does not link the C. The index-list stand-in records
//! the instance count of each list the back end emits.
//!
//! The program does not read gl_InstanceID: the PowerVR refuses an instance
//! drawn on its own whose program reads it (`pvr_backend_draw`, the PDS
//! starts every draw at instance 0), which is P1's business, not this file's.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_divisor_split
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", not(feature = "report-es2")))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend::{GlesDraw, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
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

/// One draw the GL core handed the table: its instance count and id, the
/// divisor of every array fetched per instance, the value of every array fed
/// as a constant (sorted), and what the real back end answered and emitted
/// for it: the instance count of every index list, the per-instance flag of
/// every vertex-fetch stream of every vertex program. The first draw of a
/// frame also carries the frame's own first object (an index list of one
/// instance before the draw's), so the draw's own are the last ones.
#[derive(Debug, Clone, PartialEq)]
struct Rec {
    instance_count: u32,
    instance_id: u32,
    fetched: Vec<u16>,
    constants: Vec<[f32; 4]>,
    rc: i32,
    lists: Vec<u32>,
    streams: Vec<Vec<u32>>,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    /// Allocations by GPU address (u64 backing: every u32 view is aligned).
    allocs: BTreeMap<u64, Box<[u64]>>,
    fence: i64,
    draws: Vec<Rec>,
    /// Inside `rec_draw`: the stand-ins below record only what a draw
    /// emits, into `lists` and `streams` (`Rec`'s).
    in_draw: bool,
    lists: Vec<u32>,
    streams: Vec<Vec<u32>>,
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
        p.allocs.insert(gpu, mem);
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

unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
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

/// Records the instance count of every index list (0 and 1: one instance).
#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_index_list_instanced(
    dw: *mut u32,
    topology: u32,
    addr: u64,
    count: u32,
    instance_count: u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    with_platform(|p| {
        if p.in_draw {
            p.lists.push(instance_count.max(1));
        }
    });
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

/// Records the per-instance flag of every stream of the vertex program.
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_vertex_program_ids(
    _vs_usc_exec_offset: u64,
    _vs_temps: u32,
    streams: *const PvrPdsVertexStream,
    num_streams: u32,
    _has_slc_mcu_cache_controls: i32,
    _vertex_id_reg: u32,
    _instance_id_reg: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderVertexInfo,
) -> i32 {
    let n = num_streams as usize;
    let mut per_instance = Vec::with_capacity(n);
    // SAFETY: the backend's streams, buffers (8 data words a stream, n + 2
    // code words) and record.
    unsafe {
        for i in 0..n {
            let s = *streams.add(i);
            per_instance.push(s.per_instance);
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
    with_platform(|p| {
        if p.in_draw {
            p.streams.push(per_instance);
        }
    });
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

/* ---- the table: the PowerVR's, its draw recorded ------------------------------ */

/// Draws what the GL core hands the table on the real back end, then notes
/// the draw and the answer.
unsafe extern "C" fn rec_draw(d: *const GlesDraw) -> i32 {
    with_platform(|p| {
        p.in_draw = true;
        p.lists.clear();
        p.streams.clear();
    });
    // SAFETY: the GL core's draw, valid for the call.
    let rc = unsafe { pvr_backend_draw(d) };
    let (lists, streams) = with_platform(|p| {
        p.in_draw = false;
        (core::mem::take(&mut p.lists), core::mem::take(&mut p.streams))
    });
    // SAFETY: as above.
    let d = unsafe { &*d };
    let attribs = &d.attribs[..(d.num_attribs as usize).min(d.attribs.len())];
    let mut fetched: Vec<u16> = attribs.iter().filter(|a| a.per_instance != 0).map(|a| a.divisor).collect();
    fetched.sort_unstable();
    let mut constants: Vec<[f32; 4]> = attribs
        .iter()
        .filter(|a| a.stride == 0 && a.cpu_bytes == 16 && !a.cpu.is_null())
        // SAFETY: a constant's 16 bytes of CPU copy (cpu_bytes says so).
        .map(|a| unsafe { (a.cpu as *const [f32; 4]).read_unaligned() })
        .collect();
    constants.sort_by(|a, b| a[0].total_cmp(&b[0]));
    let rec =
        Rec { instance_count: d.instance_count, instance_id: d.instance_id, fetched, constants, rc, lists, streams };
    with_platform(|p| p.draws.push(rec));
    rc
}

/// `yakogl_backend_pvr` as it is (`max_instance_divisor` included), its draw
/// recorded.
static PVR_RECORDED: YakoglBackend = YakoglBackend { draw: Some(rec_draw), ..yakogl_backend_pvr };

/* ---- the context ------------------------------------------------------------- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut context::GlContext = core::ptr::null_mut();
static mut PROG: GLuint = 0;

/// Three per-instance inputs beside the position; gl_InstanceID unread (see
/// the header).
const VS: &str = "#version 300 es
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_1;
layout(location = 2) in vec4 a_2;
layout(location = 3) in vec4 a_3;
out vec4 v;
void main() { gl_Position = a_pos; v = a_1 + a_2 + a_3; }
";
const FS: &str = "#version 300 es
precision mediump float;
in vec4 v;
out vec4 o;
void main() { o = v; }
";

/// The context and the program, set up once; the tests take turns. Each
/// starts with nothing pending and the recorders empty.
fn rig() -> (MutexGuard<'static, ()>, GLuint) {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &PVR_RECORDED;
            /* the ES 3.00 program is not in the offline table */
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(16, 16);
            assert!(!CTX.is_null(), "{:?}", logs());
            context::gl_context_set_current(CTX);
            PROG = program();
        }
    });
    // SAFETY: set once above, read under the lock; plain GL calls.
    let prog = unsafe {
        context::gl_context_set_current(CTX);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        PROG
    };
    with_platform(|p| {
        p.draws.clear();
    });
    (g, prog)
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile ({:?})", logs());
        s
    }
}

fn program() -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls on valid handles.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program did not link ({:?})", logs());
        p
    }
}

fn buffer(bytes: &[u8]) -> GLuint {
    let mut b = 0;
    // SAFETY: plain GL calls; `bytes` outlives the call.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(GL_ARRAY_BUFFER, b);
        glBufferData(GL_ARRAY_BUFFER, bytes.len() as isize, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
    }
    b
}

fn bytes_of<T>(v: &[T]) -> &[u8] {
    // SAFETY: a plain-old-data slice.
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, core::mem::size_of_val(v)) }
}

const TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
const TRI_IDX: [u16; 3] = [0, 1, 2];

/// Element `e` of array `k` (1..=3): (100 k + e, k, e, 1), so a constant
/// says which array and which element it is.
fn element(k: usize, e: usize) -> [f32; 4] {
    [(100 * k + e) as f32, k as f32, e as f32, 1.0]
}

/// The (sorted) constants of instance `inst` when arrays 1..=3 have the
/// divisors `divs`: element `inst / divisor` of every per-instance array.
fn constants_of(divs: [GLuint; 3], inst: usize) -> Vec<[f32; 4]> {
    (1..=3).filter(|&k| divs[k - 1] != 0).map(|k| element(k, inst / divs[k - 1] as usize)).collect()
}

/// Draws `instances` instances of a triangle whose arrays 1..=3 have the
/// divisors `divs` (array 0, the position, has divisor 0), as
/// glDrawArraysInstanced or (`elements`) glDrawElementsInstanced, and
/// submits the frame; returns the draws the table was handed. Every array
/// holds an element for every instance.
fn run(divs: [GLuint; 3], instances: usize, elements: bool) -> Vec<Rec> {
    let (_g, prog) = rig();
    let arrays: Vec<Vec<[f32; 4]>> = (1..=3).map(|k| (0..instances).map(|e| element(k, e)).collect()).collect();
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        glUseProgram(prog);
        let mut bufs = vec![buffer(bytes_of(&TRI))];
        bufs.extend(arrays.iter().map(|a| buffer(bytes_of(a))));
        for (loc, b) in bufs.iter().enumerate() {
            glBindBuffer(GL_ARRAY_BUFFER, *b);
            glVertexAttribPointer(loc as GLuint, 4, GL_FLOAT, 0, 0, core::ptr::null());
            glEnableVertexAttribArray(loc as GLuint);
            glVertexAttribDivisor(loc as GLuint, if loc == 0 { 0 } else { divs[loc - 1] });
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        if elements {
            glDrawElementsInstanced(
                GL_TRIANGLES,
                3,
                GL_UNSIGNED_SHORT,
                TRI_IDX.as_ptr() as *const c_void,
                instances as GLsizei,
            );
        } else {
            glDrawArraysInstanced(GL_TRIANGLES, 0, 3, instances as GLsizei);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{divs:?} x{instances}: {:?}", logs());
        glFlush();
        for loc in 0..4 {
            glDisableVertexAttribArray(loc);
            glVertexAttribDivisor(loc, 0);
        }
        glUseProgram(0);
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
        glFinish();
    }
    with_platform(|p| p.draws.clone())
}

/// Case 1: one draw of every instance, the arrays with a divisor fetched per
/// instance (divisor 1), none fed as a constant; the back end draws it as
/// one index list of every instance, its vertex program fetching streams per
/// instance (at most one per array: adjacent arrays may coalesce).
fn assert_one_draw(r: &[Rec], divs: [GLuint; 3], instances: usize) {
    let per: Vec<u16> = divs.iter().filter(|&&d| d != 0).map(|&d| d as u16).collect();
    assert_eq!(r.len(), 1, "{divs:?}: {r:?}");
    let d = &r[0];
    assert_eq!((d.instance_count, d.instance_id, d.rc), (instances as u32, 0, 0), "{divs:?}: {d:?}");
    assert_eq!(d.fetched, per, "{divs:?}: the arrays with a divisor are fetched per instance");
    assert!(d.constants.is_empty(), "{divs:?}: no array fed as a constant: {d:?}");
    assert_eq!(d.lists.last(), Some(&(instances as u32)), "{divs:?}: one index list of every instance: {d:?}");
    let fetched_per_instance = d.streams.last().map_or(0, |s| s.iter().filter(|&&f| f != 0).count());
    assert!(
        (1..=per.len()).contains(&fetched_per_instance),
        "{divs:?}: the vertex program fetches the divisor-1 arrays per instance: {d:?}"
    );
}

/// Cases 2 and 3: one back-end draw per instance, each with its instance_id
/// and one instance, nothing fetched per instance, every per-instance array
/// (divisor 1 included) a constant holding its element `inst / divisor`; the
/// back end draws every one (rc 0) as an index list of one instance, its
/// vertex programs fetching no stream per instance.
fn assert_split(r: &[Rec], divs: [GLuint; 3], instances: usize) {
    assert_eq!(r.len(), instances, "{divs:?}: one draw per instance: {r:?}");
    for (i, d) in r.iter().enumerate() {
        assert_eq!((d.instance_count, d.instance_id, d.rc), (1, i as u32, 0), "{divs:?} instance {i}: {d:?}");
        assert!(d.fetched.is_empty(), "{divs:?} instance {i}: nothing fetched per instance: {d:?}");
        assert_eq!(d.constants, constants_of(divs, i), "{divs:?} instance {i}: the instance's elements");
        assert!(
            d.lists.last() == Some(&1) && d.lists.iter().all(|&n| n == 1),
            "{divs:?} instance {i}: an index list of one instance: {d:?}"
        );
        assert!(
            !d.streams.is_empty() && d.streams.iter().all(|s| s.iter().all(|&f| f == 0)),
            "{divs:?} instance {i}: no stream fetched per instance: {d:?}"
        );
    }
}

/* ---- the tests --------------------------------------------------------------- */

/// The table under test is the PowerVR's as it is before P3.
#[test]
fn the_powervr_table_takes_divisor_1_only() {
    let _g = rig();
    assert_eq!(yakogl_backend_pvr.max_instance_divisor, 1);
    assert_eq!(yakogl::backend::max_instance_divisor(), 1);
}

/// Case 1: divisors 0 and 1 only (array 2 per vertex): one draw.
#[test]
fn divisors_0_and_1_are_one_instanced_draw() {
    for elements in [false, true] {
        let divs = [1, 0, 1];
        assert_one_draw(&run(divs, 6, elements), divs, 6);
    }
}

/// Case 1 with every array at divisor 1.
#[test]
fn divisor_1_on_every_array_is_one_instanced_draw() {
    let divs = [1, 1, 1];
    assert_one_draw(&run(divs, 5, false), divs, 5);
}

/// Case 2: divisors 2, 3 and 4 in one draw split instance by instance.
/// After P3 (max_instance_divisor 0xffff) this is one draw.
#[test]
fn divisors_2_3_4_split_into_one_draw_per_instance() {
    for elements in [false, true] {
        let divs = [2, 3, 4];
        assert_split(&run(divs, 9, elements), divs, 9);
    }
}

/// Case 2: a mixed 1/2 draw splits too, its divisor-1 array a constant like
/// the others. After P3 this is one draw.
#[test]
fn mixed_divisors_1_and_2_split_into_one_draw_per_instance() {
    let divs = [1, 2, 0];
    assert_split(&run(divs, 5, false), divs, 5);
}

/// Case 3: a divisor of 0x10000 (above 0xffff) splits; the documented
/// fallback that must survive P3.
#[test]
fn divisor_0x10000_splits_into_one_draw_per_instance() {
    let divs = [1, 2, 0x10000];
    assert_split(&run(divs, 4, false), divs, 4);
}

/// One instance is one draw whatever the divisors (nothing to split).
#[test]
fn one_instance_is_one_draw_whatever_the_divisors() {
    let r = run([2, 3, 0x10000], 1, false);
    assert_eq!(r.len(), 1, "{r:?}");
    assert_eq!((r[0].instance_count, r[0].instance_id, r[0].rc), (1, 0, 0));
}
