// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Edge cases of the PowerVR hardware instance divisors (P3, opt-in cargo
//! feature `pvr-instance-divisor`; docs/notes/instanceid-per-draw.md, issue
//! #18), the ones tests/es3_pvr_divisor.rs does not pin:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_divisor_edges
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,pvr-instance-divisor --test es3_pvr_divisor_edges
//! ```
//!
//! Without the feature one test pins `max_instance_divisor` 1. With it:
//! (a) glDrawElementsInstanced, divisors 2 and 3, ushort indices; (b)
//! glDrawArraysInstanced with first > 0: per-vertex streams start at `first`,
//! divisor streams do not move; (c) a plain glDrawArrays reading divisor-2
//! arrays reads element 0; (d) divisor 5 with 3 instances: every instance
//! reads element 0; (e) one buffer interleaving a divisor-1 and a divisor-2
//! stream is not one DMA; (f) two streams of divisor 3 in one buffer, which
//! the back end may coalesce, still fetch the right elements.
//!
//! Each draw goes through the GL API and is checked twice: the PDS stream
//! divisor and instance base the back end recorded, and a CPU reference
//! fetch: instance i of a chunk with instance base B reads element
//! (B + i) / divisor from the stream's address and stride, which must equal
//! the element floor(instance / divisor) of the array the test filled. The
//! harness is copied from tests/es3_pvr_divisor.rs (the real PowerVR table
//! with its `draw` recorded, stand-ins for the C command generators). What
//! the GPU draws is the AM62P board check.
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
/* the no-feature arm uses only the table */
#![cfg_attr(not(feature = "pvr-instance-divisor"), allow(dead_code))]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
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

/* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE). */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

/// One vertex-fetch stream as the back end handed it to the PDS generator,
/// with its first element's words read back from the arena.
#[derive(Debug, Clone, PartialEq)]
struct Stream {
    address: u64,
    dst_reg: u32,
    stride: u32,
    dma_size: u32,
    per_instance: u32,
    divisor: u32,
    first: Vec<f32>,
    /// The words from `address` on (up to 512), for the reference fetch.
    tail: Vec<u32>,
}

/// What the back end wrote for one draw of the table: its VDM index lists
/// (vertex count, instance count) and the vertex streams of its last vertex
/// program. The first draw of a frame also writes the frame-start quad (an
/// index list of 6) ahead of its own: [`Hw::own`] leaves it out.
#[derive(Debug, Clone, Default)]
struct Hw {
    lists: Vec<(u32, u32)>,
    streams: Vec<Stream>,
    /// Each index list with the vertex program generated last before it
    /// (the one it runs).
    chunks: Vec<Chunk>,
}

/// One vertex program as the generator was asked for it (read by the
/// feature arm).
#[derive(Debug, Clone, Default)]
struct VertexRec {
    streams: Vec<Stream>,
    instance_id_reg: u32,
    instance_id_base: u32,
}

/// One index list: its vertex and instance counts and its vertex program.
#[derive(Debug, Clone)]
struct Chunk {
    vertices: u32,
    instances: u32,
    vertex: VertexRec,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    /// The table draw being recorded (its index in `TableRec`s), or None
    /// outside a draw (a flush's own blocks).
    current: Option<usize>,
    hw: Vec<Hw>,
    /// The vertex program generated last.
    vertex: VertexRec,
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

/// `n` words at GPU address `gpu`, as the arena holds them now.
fn words_at(p: &Platform, gpu: u64, n: usize) -> Vec<u32> {
    let (&base, a) = p.allocs.range(..=gpu).next_back().expect("an address outside every allocation");
    let at = (gpu - base) as usize;
    assert!(at.is_multiple_of(4) && at + 4 * n <= a.size, "0x{gpu:x} + {n} words runs past its allocation");
    // SAFETY: inside the allocation (checked above); u64 backing, 4-aligned offset.
    unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(at) as *const u32, n) }.to_vec()
}

/// Up to `n` words from GPU address `gpu` to the end of its allocation.
fn tail_at(p: &Platform, gpu: u64, n: usize) -> Vec<u32> {
    let (&base, a) = p.allocs.range(..=gpu).next_back().expect("an address outside every allocation");
    let left = (a.size - (gpu - base) as usize) / 4;
    words_at(p, gpu, left.min(n))
}

/// The current table draw's hardware record.
fn hw_of(p: &mut Platform) -> Option<&mut Hw> {
    let i = p.current?;
    p.hw.get_mut(i)
}

/* ---- the PowerVR's command generators (driver/src/pvr, C), stood in for ------- */

const W_TERM: u32 = 0xE000_0000;

#[no_mangle]
unsafe extern "C" fn pvr_ppp_build_block(
    _input: *const PvrPppFragInputs,
    _st: *const PvrPppState,
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

/// Records the index list's vertex and instance counts (0 and 1 both mean
/// one instance).
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
        let vertex = p.vertex.clone();
        if let Some(hw) = hw_of(p) {
            hw.lists.push((count, instance_count.max(1)));
            hw.chunks.push(Chunk { vertices: count, instances: instance_count.max(1), vertex });
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
    sample_rate: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderKickUscInfo,
) -> i32 {
    // SAFETY: the backend's buffers (8 data words, 1 code word) and record.
    unsafe {
        let w = [0x4B00_0009, fs_usc_exec_offset as u32, fs_temps, sample_rate, 0x4B4B_0004, 5, 6, 7];
        core::ptr::copy_nonoverlapping(w.as_ptr(), data_out, 8);
        *code_out = 0x4BC0_DE00;
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

/// Records the draw's vertex streams, each with its first element read back
/// from the arena (up to four words), and the instance register and base.
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_vertex_program_ids_base(
    _vs_usc_exec_offset: u64,
    _vs_temps: u32,
    streams: *const PvrPdsVertexStream,
    num_streams: u32,
    _has_slc_mcu_cache_controls: i32,
    _vertex_id_reg: u32,
    instance_id_reg: u32,
    instance_id_base: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderVertexInfo,
) -> i32 {
    let n = num_streams as usize;
    // SAFETY: the backend passes num_streams streams.
    let st: Vec<PvrPdsVertexStream> = (0..n).map(|i| unsafe { *streams.add(i) }).collect();
    with_platform(|p| {
        let rec: Vec<Stream> = st
            .iter()
            .map(|s| Stream {
                address: s.address,
                dst_reg: s.dst_reg,
                stride: s.stride,
                dma_size: s.dma_size,
                per_instance: s.per_instance,
                divisor: s.divisor,
                first: words_at(p, s.address, (s.dma_size as usize / 4).min(16))
                    .into_iter()
                    .map(f32::from_bits)
                    .collect(),
                tail: tail_at(p, s.address, 512),
            })
            .collect();
        p.vertex = VertexRec { streams: rec.clone(), instance_id_reg, instance_id_base };
        if let Some(hw) = hw_of(p) {
            hw.streams = rec;
        }
    });
    // SAFETY: the backend's buffers (8 data words a stream, n + 2 code words)
    // and record.
    unsafe {
        for (i, s) in st.iter().enumerate() {
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
    _varyings: *const PvrPdsCoeffVarying,
    num_varyings: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsCoeffInfo,
) -> i32 {
    // SAFETY: the backend's buffers (at least 4 data words, 2 code words) and
    // record.
    unsafe {
        core::ptr::copy_nonoverlapping([0xC0EF_0006, 0, 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0xC0DE_0000 | num_varyings, 0xC0DE_FFFF].as_ptr(), code_out, 2);
        *info = PvrPdsCoeffInfo {
            data_size_dw: 4,
            code_size_dw: 2,
            coeff_dwords: 4 * num_varyings,
            pds_varyingsize: num_varyings + 1,
            usc_varyingsize: num_varyings + 2,
        };
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

/* ---- the table: the PowerVR's own, its draws recorded ------------------------ */

/// One per-instance array of a draw as the table saw it: its vtxin register,
/// and either its divisor (fetched per instance) or its value (a constant).
#[derive(Debug, Clone, PartialEq)]
enum Arr {
    Fetched { vi_reg: u8, divisor: u16 },
    Constant { vi_reg: u8, value: [f32; 4] },
}

/// One draw of the table.
#[derive(Debug, Clone)]
struct TableRec {
    instance_count: u32,
    instance_id: u32,
    /// Every array but the position (vtxin register 0..4), in register order.
    arrays: Vec<Arr>,
    rc: i32,
}

static DRAWS: Mutex<Vec<TableRec>> = Mutex::new(Vec::new());

/// Records the draw, then hands it to the real PowerVR back end with the
/// platform's `current` set, so the stand-ins file what it writes under it.
unsafe extern "C" fn rec_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core's draw, valid for the call.
    let dr = unsafe { &*d };
    let attribs = &dr.attribs[..(dr.num_attribs as usize).min(dr.attribs.len())];
    let mut arrays: Vec<Arr> = attribs
        .iter()
        .filter_map(|a| {
            if a.per_instance != 0 {
                Some(Arr::Fetched { vi_reg: a.vi_reg, divisor: a.divisor.max(1) })
            } else if a.stride == 0 && a.cpu_bytes == 16 && !a.cpu.is_null() {
                // SAFETY: the core stages 16 bytes (one vec4) at a.cpu.
                let value = unsafe { *(a.cpu as *const [f32; 4]) };
                Some(Arr::Constant { vi_reg: a.vi_reg, value })
            } else {
                None
            }
        })
        .collect();
    arrays.sort_by_key(|a| match a {
        Arr::Fetched { vi_reg, .. } | Arr::Constant { vi_reg, .. } => *vi_reg,
    });
    let at = {
        let mut g = DRAWS.lock().unwrap();
        g.push(TableRec { instance_count: dr.instance_count, instance_id: dr.instance_id, arrays, rc: 0 });
        g.len() - 1
    };
    with_platform(|p| {
        p.hw.resize(at + 1, Hw::default());
        p.current = Some(at);
    });
    // SAFETY: the real back end's draw, with the core's draw.
    let rc = unsafe { (yakogl_backend_pvr.draw.unwrap())(d) };
    with_platform(|p| p.current = None);
    DRAWS.lock().unwrap()[at].rc = rc;
    rc
}

/// The PowerVR table, its `draw` recorded and forwarded (struct update, so
/// every other field, `max_instance_divisor` included, is the PowerVR's).
static PVR_RECORDED: YakoglBackend = YakoglBackend { draw: Some(rec_draw), ..yakogl_backend_pvr };

/* ---- the context ------------------------------------------------------------- */

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut context::GlContext = core::ptr::null_mut();
static mut PROG: GLuint = 0;

const VS: &str = "#version 300 es
layout(location = 0) in highp vec4 a_pos;
layout(location = 1) in highp vec4 a_1;
layout(location = 2) in highp vec4 a_2;
layout(location = 3) in highp vec4 a_3;
out highp vec4 v;
void main() { gl_Position = a_pos; v = a_1 + a_2 + a_3; }
";
const FS: &str = "#version 300 es
precision mediump float;
in highp vec4 v;
layout(location = 0) out mediump vec4 o;
void main() { o = v; }
";

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(type_: GLenum, src: &str) -> GLuint {
    let csrc = std::ffi::CString::new(src).unwrap();
    // SAFETY: plain GL calls; a NUL-terminated source.
    unsafe {
        let s = glCreateShader(type_);
        let p = csrc.as_ptr();
        glShaderSource(s, 1, &p, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{src}: compile failed ({:?})", logs());
        s
    }
}

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
            CTX = context::gl_context_create(64, 48);
            assert!(!CTX.is_null(), "{:?}", logs());
            context::gl_context_set_current(CTX);
            let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "the program: link failed ({:?})", logs());
            PROG = p;
        }
    });
    // SAFETY: set once above, read under the lock; plain GL calls.
    let prog = unsafe {
        context::gl_context_set_current(CTX);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        PROG
    };
    DRAWS.lock().unwrap().clear();
    with_platform(|p| {
        p.hw.clear();
        p.current = None;
        p.vertex = VertexRec::default();
    });
    (g, prog)
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

fn bytes_of(v: &[f32]) -> &[u8] {
    // SAFETY: a plain-old-data slice of f32.
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, core::mem::size_of_val(v)) }
}

const TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];

/// Element `e` of array `k` (location k, 1..=3): four distinct words.
fn element(k: usize, e: usize) -> [f32; 4] {
    let x = (100 * k + e) as f32;
    [x, x + 0.25, x + 0.5, x + 0.75]
}

/// Vertex `j` of the position array: distinct words, so `first` is visible.
fn pos(j: usize) -> [f32; 4] {
    [0.1 * j as f32, 0.2, 0.0, 1.0]
}

/// One draw: arrays 1..=3 have the divisors `divs` and are laid out in
/// buffers by `groups` (each group of locations interleaved in one buffer,
/// one vec4 of each an element); array 0 is the position.
struct Spec {
    divs: [GLuint; 3],
    groups: &'static [&'static [usize]],
    instances: usize,
    /// `first` of glDrawArrays*.
    first: usize,
    /// glDrawElementsInstanced with these ushort indices instead.
    indices: Option<&'static [u16]>,
    /// false: glDrawArrays (one instance, whatever `instances` says).
    instanced: bool,
}

const SEPARATE: &[&[usize]] = &[&[1], &[2], &[3]];

/// Runs the draw through the GL API; returns the table's and the back end's
/// records.
fn run(s: &Spec) -> (Vec<TableRec>, Vec<Hw>) {
    let (_g, prog) = rig();
    let elems = s.instances.max(3) + 1;
    /* the triangle sits at vertices `first`.. (1.. for the indexed draw) */
    let at = s.first.max(1);
    let mesh: Vec<f32> = (0..8usize)
        .flat_map(|j| match j.checked_sub(at) {
            Some(t) if t < 3 => [TRI[4 * t], TRI[4 * t + 1], TRI[4 * t + 2], TRI[4 * t + 3]],
            _ => pos(j),
        })
        .collect();
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        glUseProgram(prog);
        let pb = buffer(bytes_of(&mesh));
        let mut bufs = vec![pb];
        glBindBuffer(GL_ARRAY_BUFFER, pb);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, core::ptr::null());
        glEnableVertexAttribArray(0);
        glVertexAttribDivisor(0, 0);
        for g in s.groups {
            let data: Vec<f32> = (0..elems).flat_map(|e| g.iter().flat_map(move |&k| element(k, e))).collect();
            let b = buffer(bytes_of(&data));
            bufs.push(b);
            for (n, &k) in g.iter().enumerate() {
                glBindBuffer(GL_ARRAY_BUFFER, b);
                glVertexAttribPointer(k as GLuint, 4, GL_FLOAT, 0, 16 * g.len() as GLsizei, (16 * n) as *const c_void);
                glEnableVertexAttribArray(k as GLuint);
                glVertexAttribDivisor(k as GLuint, s.divs[k - 1]);
            }
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        let mut ib = 0;
        if let Some(ix) = s.indices {
            glGenBuffers(1, &mut ib);
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, ib);
            glBufferData(GL_ELEMENT_ARRAY_BUFFER, 2 * ix.len() as isize, ix.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glDrawElementsInstanced(
                GL_TRIANGLES,
                ix.len() as GLsizei,
                GL_UNSIGNED_SHORT,
                core::ptr::null(),
                s.instances as GLsizei,
            );
        } else if s.instanced {
            glDrawArraysInstanced(GL_TRIANGLES, s.first as GLint, 3, s.instances as GLsizei);
        } else {
            glDrawArrays(GL_TRIANGLES, s.first as GLint, 3);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glFlush();
        for loc in 0..4 {
            glDisableVertexAttribArray(loc);
            glVertexAttribDivisor(loc, 0);
        }
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glUseProgram(0);
        if ib != 0 {
            glDeleteBuffers(1, &ib);
        }
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
    }
    let draws = DRAWS.lock().unwrap().clone();
    let hw = with_platform(|p| p.hw.clone());
    for (i, d) in draws.iter().enumerate() {
        assert_eq!(d.rc, 0, "draw {i} refused by the back end: {d:?} ({:?})", logs());
    }
    assert_eq!(hw.len(), draws.len(), "a hardware record per table draw");
    (draws, hw)
}

/// The stream of `streams` that loads vtxin register `reg..reg + 4`.
fn stream_at(streams: &[Stream], reg: u32) -> &Stream {
    streams
        .iter()
        .find(|s| s.dst_reg <= reg && reg + 4 <= s.dst_reg + s.dma_size / 4)
        .unwrap_or_else(|| panic!("no stream loads vtxin {reg}: {streams:?}"))
}

/// The CPU reference fetch: the four words a stream gives vtxin register
/// `reg` for element `e` (`address + e * stride`).
fn fetch(s: &Stream, reg: u32, e: usize) -> [f32; 4] {
    let at = (e * s.stride as usize + 4 * (reg - s.dst_reg) as usize) / 4;
    let w = &s.tail[at..at + 4];
    [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2]), f32::from_bits(w[3])]
}

/// The triangle's own index lists of a hardware record.
fn own(h: &Hw) -> Vec<&Chunk> {
    h.chunks.iter().filter(|c| c.vertices == 3).collect()
}

/// Every instance of the GL draw gets, from every array k with a divisor,
/// element floor(instance / divisor): as the constant of a split draw
/// (instance_id), or by the reference fetch of the stream its chunk's vertex
/// program was given (instance base + i, divided). Returns the number of
/// instances fetched through a stream and the instance base of each chunk.
fn check_elements(s: &Spec, draws: &[TableRec], hw: &[Hw]) -> (usize, Vec<u32>) {
    let (mut fetched, mut bases, mut seen) = (0, vec![], 0usize);
    let total = if s.instanced || s.indices.is_some() { s.instances } else { 1 };
    let ks: Vec<usize> = (1..=3).filter(|&k| s.divs[k - 1] != 0).collect();
    for (d, h) in draws.iter().zip(hw) {
        assert_eq!(d.arrays.len(), ks.len(), "every array with a divisor is a per-instance array: {d:?}");
        let mut inst0 = d.instance_id as usize;
        for c in own(h) {
            let base = c.vertex.instance_id_base as usize;
            bases.push(base as u32);
            for (a, &k) in d.arrays.iter().zip(&ks) {
                let div = s.divs[k - 1] as usize;
                match a {
                    Arr::Constant { value, .. } => {
                        assert_eq!(c.instances, 1, "a constant is a one-instance draw: {d:?}");
                        assert_eq!(*value, element(k, inst0 / div), "array {k}: the constant of instance {inst0}");
                    }
                    Arr::Fetched { vi_reg, divisor } => {
                        assert_eq!(*divisor as usize, div, "array {k}: the table's divisor");
                        let st = stream_at(&c.vertex.streams, *vi_reg as u32);
                        let flag = c.vertex.instance_id_reg & PVR_PDS_ID_BASE_TO_STREAMS;
                        assert_ne!(flag, 0, "the instance base goes to the streams: {:?}", c.vertex);
                        assert_eq!(st.per_instance, 1, "array {k}: {st:?}");
                        assert_eq!(st.divisor as usize, div, "array {k}: the PDS stream's divisor: {st:?}");
                        for i in 0..c.instances as usize {
                            let got = fetch(st, *vi_reg as u32, (base + i) / div);
                            assert_eq!(got, element(k, (inst0 + i) / div), "array {k}, instance {}", inst0 + i);
                        }
                        if k == ks[0] {
                            fetched += c.instances as usize;
                        }
                    }
                }
            }
            inst0 += c.instances as usize;
            seen += c.instances as usize;
        }
    }
    assert_eq!(seen, total, "every instance is drawn once");
    (fetched, bases)
}

/// The per-instance streams of the draw's last own chunk, by register.
fn per_instance(h: &Hw) -> Vec<Stream> {
    let c = own(h).pop().expect("a chunk");
    let mut v: Vec<Stream> = c.vertex.streams.iter().filter(|s| s.per_instance == 1).cloned().collect();
    v.sort_by_key(|s| s.dst_reg);
    v
}

/* ---- the tests: without the feature ------------------------------------------ */

/// The table: the PowerVR takes divisor 1 only without the feature.
#[cfg(not(feature = "pvr-instance-divisor"))]
#[test]
fn the_pvr_table_reports_max_instance_divisor_1() {
    assert_eq!(yakogl_backend_pvr.max_instance_divisor, 1);
    assert_eq!(PVR_RECORDED.max_instance_divisor, 1);
}

/* ---- the tests: with the feature --------------------------------------------- */

/// (a) glDrawElementsInstanced, ushort indices, divisors 2 and 3: one draw of
/// 7 instances whose divisor streams carry the divisors.
#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn a_draw_elements_instanced_with_divisors_2_and_3() {
    for divs in [[2, 2, 2], [3, 3, 3], [2, 3, 2]] {
        let s = Spec { divs, groups: SEPARATE, instances: 7, first: 0, indices: Some(&[1, 2, 3]), instanced: true };
        let (draws, hw) = run(&s);
        assert_eq!(draws.len(), 1, "{divs:?}: one draw: {draws:?}");
        assert_eq!(draws[0].instance_count, 7, "{divs:?}");
        assert_eq!(own(&hw[0]).iter().map(|c| c.instances).collect::<Vec<_>>(), [7], "{divs:?}: {:?}", hw[0].lists);
        let (fetched, bases) = check_elements(&s, &draws, &hw);
        assert_eq!((fetched, bases), (7, vec![0]), "{divs:?}");
        let got: Vec<u32> = per_instance(&hw[0]).iter().map(|p| p.divisor).collect();
        assert_eq!(got, divs, "{divs:?}: the streams' divisors");
        let p = stream_at(&own(&hw[0])[0].vertex.streams, 0);
        assert_eq!((p.per_instance, p.divisor), (0, 0), "{divs:?}: the position stream: {p:?}");
    }
}

/// (b) glDrawArraysInstanced with first = 2: the position stream starts at
/// vertex 2, the divisor streams stay at element 0 and the instance base is 0
/// (first moves vertices, never instances).
#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn b_first_moves_the_vertex_stream_not_the_divisor_streams() {
    let s = Spec { divs: [2, 3, 4], groups: SEPARATE, instances: 9, first: 2, indices: None, instanced: true };
    let (draws, hw) = run(&s);
    assert_eq!(draws.len(), 1, "{draws:?}");
    let (fetched, bases) = check_elements(&s, &draws, &hw);
    assert_eq!((fetched, bases), (9, vec![0]));
    let c = &own(&hw[0])[0];
    let p = stream_at(&c.vertex.streams, 0);
    assert_eq!(p.per_instance, 0, "{p:?}");
    assert_eq!(fetch(p, 0, 0), TRI[..4], "the position stream starts at vertex `first`: {p:?}");
    for (p, k) in per_instance(&hw[0]).iter().zip(1..) {
        assert_eq!(p.first[..4], element(k, 0), "array {k}'s stream does not move with first: {p:?}");
    }
}

/// (c) A plain glDrawArrays reading arrays of divisor 2: one instance, so
/// element 0 of each, however the back end gets it (a constant or a stream).
#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn c_a_plain_draw_reads_element_0_of_divisor_arrays() {
    let s = Spec { divs: [2, 2, 2], groups: SEPARATE, instances: 1, first: 0, indices: None, instanced: false };
    let (draws, hw) = run(&s);
    assert_eq!(draws.len(), 1, "{draws:?}");
    assert_eq!(draws[0].instance_count, 1, "{:?}", draws[0]);
    let (_, bases) = check_elements(&s, &draws, &hw);
    assert_eq!(bases, [0]);
}

/// (d) Divisor 5 with 3 instances: the divisor is larger than the instance
/// count, every instance reads element 0.
#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn d_a_divisor_above_the_instance_count_reads_element_0() {
    let s = Spec { divs: [5, 5, 5], groups: SEPARATE, instances: 3, first: 0, indices: None, instanced: true };
    let (draws, hw) = run(&s);
    assert_eq!(draws.len(), 1, "{draws:?}");
    assert_eq!(draws[0].instance_count, 3, "{:?}", draws[0]);
    let (fetched, bases) = check_elements(&s, &draws, &hw);
    assert_eq!((fetched, bases), (3, vec![0]));
    for p in per_instance(&hw[0]) {
        assert_eq!(p.divisor, 5, "{p:?}");
    }
}

/// (e) One buffer interleaving a divisor-1 array and a divisor-2 array (the
/// third, divisor 3, in its own buffer): never one DMA.
#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn e_streams_of_different_divisors_in_one_buffer_are_not_coalesced() {
    let s = Spec { divs: [1, 2, 3], groups: &[&[1, 2], &[3]], instances: 12, first: 0, indices: None, instanced: true };
    let (draws, hw) = run(&s);
    assert_eq!(draws.len(), 1, "{draws:?}");
    let (fetched, bases) = check_elements(&s, &draws, &hw);
    assert_eq!((fetched, bases), (12, vec![0]));
    let per = per_instance(&hw[0]);
    let got: Vec<(u32, u32, u32)> = per.iter().map(|p| (p.dma_size, p.stride, p.divisor)).collect();
    assert_eq!(got, [(16, 32, 1), (16, 32, 2), (16, 16, 3)], "{per:?}");
}

/// (f) Two arrays of divisor 3 interleaved in one buffer (the third, divisor
/// 3, in its own): the back end may coalesce the pair into one 32-byte DMA;
/// either way each array's elements are the right ones.
#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn f_streams_of_equal_divisor_may_coalesce_and_still_fetch_right() {
    let s = Spec { divs: [3, 3, 3], groups: &[&[1, 2], &[3]], instances: 12, first: 0, indices: None, instanced: true };
    let (draws, hw) = run(&s);
    assert_eq!(draws.len(), 1, "{draws:?}");
    let (fetched, bases) = check_elements(&s, &draws, &hw);
    assert_eq!((fetched, bases), (12, vec![0]));
    let per = per_instance(&hw[0]);
    assert!(per.iter().all(|p| p.divisor == 3), "{per:?}");
    let bytes: u32 = per.iter().map(|p| p.dma_size).sum();
    assert_eq!(bytes, 48, "no array is dropped or fetched twice: {per:?}");
    assert!(per.len() <= 3, "{per:?}");
}
