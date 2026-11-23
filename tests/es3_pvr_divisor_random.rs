// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR hardware instance divisors (P3, feature `pvr-instance-divisor`;
//! docs/notes/instanceid-per-draw.md, issue #18) against integer division,
//! on random draws: a seeded property test.
//!
//! Each draw has 1 to 16 vertex arrays (array 0, the position, is always per
//! vertex), the others per vertex or per instance with a divisor from
//! {1, 2, 3, 4, 5, 7, 255, 256, 0xfffe, 0xffff} or random in 1..=0xffff.
//! Some arrays share one buffer object, interleaved with a random stride and
//! random offsets. The draw is `glDrawArraysInstanced` or
//! `glDrawElementsInstanced` of 1 to 300 instances from a random first
//! vertex; now and then it is so large that the back end splits it into
//! chunks (P1's `instance_id_modifier`). The model: instance `i` of a
//! per-instance array of divisor `d` fetches element `floor(i / d)`.
//!
//! What the back end records for each draw is checked against it:
//!
//!  - one table draw, and in the command stream one index list, or the
//!    chunks, whose instances add up to the draw's, each chunk with the
//!    instance base set to its first instance (`PVR_PDS_ID_BASE_TO_STREAMS`)
//!    and no base move of any stream (every per-instance stream starts at its
//!    array's element 0);
//!  - every array's data is loaded by a stream of its own rate: per-instance
//!    arrays by a per-instance stream whose `divisor` is the array's, per-
//!    vertex ones by a stream of divisor 0, so arrays of different divisors
//!    are never coalesced, and each array's words are those of its element;
//!  - the stream's multiplier and shift words, computed as Mesa does
//!    (pvr_pipeline_pds.c:846-864, the model of tests/es3_pvr_divisor_magic.rs;
//!    a power of two is one shift), applied the way the hardware does
//!    ((i * multiplier) >> (31 + shift), or i >> log2 for a power of two),
//!    give `floor((first + i) / d)` for every instance of every chunk.
//!
//! The generator is a fixed-seed xorshift; a failure prints the seed and the
//! draw (`YAKOGL_DIVISOR_SEED=<n>` reruns one draw seed). The C generator's
//! own data words are pinned by tests/es3_pvr_divisor_pds.rs: here the
//! command generators are stood in for (recording their inputs), as in
//! tests/es3_pvr_divisor.rs, whose harness this copies. What the GPU draws
//! is the AM62P board check.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,pvr-instance-divisor --test es3_pvr_divisor_random
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
    feature = "pvr-instance-divisor",
    not(feature = "report-es2"),
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

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
}

/// What the back end wrote for one draw of the table: its VDM index lists
/// (vertex count, instance count) and the vertex streams of its last vertex
/// program. The first draw of a frame also writes the frame-start quad (an
/// index list of 6) ahead of its own: the checks leave it out.
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
                first: words_at(p, s.address, (s.dma_size as usize / 4).min(1024))
                    .into_iter()
                    .map(f32::from_bits)
                    .collect(),
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

/// One array of a draw as the table saw it.
#[derive(Debug, Clone, PartialEq)]
struct TableAttrib {
    vi_reg: u8,
    per_instance: bool,
    divisor: u32,
}

/// One draw of the table.
#[derive(Debug, Clone)]
struct TableRec {
    instance_count: u32,
    instance_id: u32,
    /// Every array the core hands the back end, in location order.
    attribs: Vec<TableAttrib>,
    rc: i32,
}

static DRAWS: Mutex<Vec<TableRec>> = Mutex::new(Vec::new());

/// Records the draw, then hands it to the real PowerVR back end with the
/// platform's `current` set, so the stand-ins file what it writes under it.
unsafe extern "C" fn rec_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core's draw, valid for the call.
    let dr = unsafe { &*d };
    let attribs = dr.attribs[..(dr.num_attribs as usize).min(dr.attribs.len())]
        .iter()
        .map(|a| TableAttrib { vi_reg: a.vi_reg, per_instance: a.per_instance != 0, divisor: a.divisor as u32 })
        .collect();
    let at = {
        let mut g = DRAWS.lock().unwrap();
        g.push(TableRec { instance_count: dr.instance_count, instance_id: dr.instance_id, attribs, rc: 0 });
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

/// Sixteen vec4 inputs, all read, so every enabled array is live.
const VS: &str = "#version 300 es
layout(location = 0) in highp vec4 a_0;
layout(location = 1) in highp vec4 a_1;
layout(location = 2) in highp vec4 a_2;
layout(location = 3) in highp vec4 a_3;
layout(location = 4) in highp vec4 a_4;
layout(location = 5) in highp vec4 a_5;
layout(location = 6) in highp vec4 a_6;
layout(location = 7) in highp vec4 a_7;
layout(location = 8) in highp vec4 a_8;
layout(location = 9) in highp vec4 a_9;
layout(location = 10) in highp vec4 a_10;
layout(location = 11) in highp vec4 a_11;
layout(location = 12) in highp vec4 a_12;
layout(location = 13) in highp vec4 a_13;
layout(location = 14) in highp vec4 a_14;
layout(location = 15) in highp vec4 a_15;
out highp vec4 v;
void main() {
    gl_Position = a_0;
    v = a_1 + a_2 + a_3 + a_4 + a_5 + a_6 + a_7 + a_8 + a_9 + a_10 + a_11 + a_12 + a_13 + a_14 + a_15;
}
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

/// The context and the program, set up once; each call starts with nothing
/// pending and the recorders empty.
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

/* ---- the generator ----------------------------------------------------------- */

/// xorshift64*: small, seeded, no dependency.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// 0..n
    fn below(&mut self, n: u64) -> u64 {
        (self.next() >> 11) % n
    }
    /// lo..=hi
    fn range(&mut self, lo: u64, hi: u64) -> u64 {
        lo + self.below(hi - lo + 1)
    }
}

const DIVISORS: [u32; 10] = [1, 2, 3, 4, 5, 7, 255, 256, 0xfffe, 0xffff];

/// One array: its divisor (0: per vertex), its buffer and byte offset.
#[derive(Debug, Clone)]
struct Attr {
    divisor: u32,
    buf: usize,
    offset: usize,
}

/// One buffer object: its stride and the arrays interleaved in it.
#[derive(Debug, Clone)]
struct Buf {
    stride: usize,
    members: Vec<usize>,
}

#[derive(Debug, Clone)]
struct Case {
    seed: u64,
    attrs: Vec<Attr>,
    bufs: Vec<Buf>,
    instances: u32,
    first: u32,
    vertices: u32,
    elements: bool,
}

impl Case {
    fn generate(seed: u64, big: bool) -> Case {
        let mut r = Rng::new(seed);
        let n_attr = if big { r.range(1, 8) } else { r.range(1, 16) } as usize;
        let mut divs = vec![0u32];
        for _ in 1..n_attr {
            divs.push(match r.below(5) {
                0 => 0,
                1 => r.range(1, 0xffff) as u32,
                _ => DIVISORS[r.below(DIVISORS.len() as u64) as usize],
            });
        }
        /* group the arrays into buffers: each joins the previous buffer with
         * probability 1/2 */
        let mut bufs: Vec<Buf> = Vec::new();
        let mut attrs = Vec::new();
        for (k, &divisor) in divs.iter().enumerate() {
            if k == 0 || bufs.is_empty() || r.below(2) == 0 {
                let lead = 4 * r.below(3) as usize;
                bufs.push(Buf { stride: lead, members: vec![] });
            }
            let b = bufs.len() - 1;
            let gap = 4 * r.below(3) as usize;
            let offset = bufs[b].stride + gap;
            bufs[b].stride = offset + 16;
            bufs[b].members.push(k);
            attrs.push(Attr { divisor, buf: b, offset });
        }
        for b in &mut bufs {
            b.stride += 4 * r.below(3) as usize;
        }
        let (instances, vertices, first) = if big {
            /* 10000 triangles an instance: more than the 1.5M primitives a
             * kick takes in 150 instances */
            (r.range(160, 300) as u32, 30000, r.range(0, 1000) as u32)
        } else {
            let mut v = 3 * r.range(1, 10) as u32;
            if v == 6 {
                /* the frame-start quad's index list has 6 vertices */
                v = 9;
            }
            (r.range(1, 300) as u32, v, r.range(0, 40) as u32)
        };
        Case { seed, attrs, bufs, instances, first, vertices, elements: r.below(2) == 0 }
    }

    /// Elements of array `k` the draw reads: every vertex up to the last
    /// (the first vertex included), or the instances' elements.
    fn needed(&self, k: usize) -> usize {
        match self.attrs[k].divisor {
            0 => (self.first + self.vertices) as usize,
            d => ((self.instances - 1) / d) as usize + 1,
        }
    }
}

/// Element `e` of array `k`: four words that name both (the position's
/// are small, so every vertex is a valid position).
fn elem(k: usize, e: usize) -> [f32; 4] {
    if k == 0 {
        let y = (e % 3) as f32 - 1.0;
        return [e as f32 / 65536.0 - 0.5, y, 0.0, 1.0];
    }
    let x = (100_000 * k + e) as f32;
    [x, x + 0.25, x + 0.5, x + 0.75]
}

fn buffer(target: GLenum, bytes: &[u8]) -> GLuint {
    let mut b = 0;
    // SAFETY: plain GL calls; `bytes` outlives the call.
    unsafe {
        glGenBuffers(1, &mut b);
        glBindBuffer(target, b);
        glBufferData(target, bytes.len() as isize, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
    }
    b
}

/// The buffer's bytes: every member's elements at their offsets, the gaps
/// zero.
fn buf_bytes(c: &Case, b: &Buf) -> Vec<u8> {
    let count = b.members.iter().map(|&k| c.needed(k)).max().unwrap();
    let mut out = vec![0u8; count * b.stride];
    for &k in &b.members {
        for e in 0..c.needed(k) {
            for (w, x) in elem(k, e).iter().enumerate() {
                let at = e * b.stride + c.attrs[k].offset + 4 * w;
                out[at..at + 4].copy_from_slice(&x.to_ne_bytes());
            }
        }
    }
    out
}

/// What the table and the back end saw for one draw.
struct Seen {
    draws: Vec<TableRec>,
    hw: Vec<Hw>,
}

fn run(c: &Case) -> Seen {
    let (_g, prog) = rig();
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        glUseProgram(prog);
        let bufs: Vec<GLuint> = c.bufs.iter().map(|b| buffer(GL_ARRAY_BUFFER, &buf_bytes(c, b))).collect();
        for (k, a) in c.attrs.iter().enumerate() {
            glBindBuffer(GL_ARRAY_BUFFER, bufs[a.buf]);
            glVertexAttribPointer(
                k as GLuint,
                4,
                GL_FLOAT,
                0,
                c.bufs[a.buf].stride as GLsizei,
                a.offset as *const c_void,
            );
            glEnableVertexAttribArray(k as GLuint);
            glVertexAttribDivisor(k as GLuint, a.divisor);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        if c.elements {
            let idx: Vec<u8> = (0..c.vertices).flat_map(|j| ((c.first + j) as u16).to_ne_bytes()).collect();
            let ib = buffer(GL_ELEMENT_ARRAY_BUFFER, &idx);
            glDrawElementsInstanced(
                GL_TRIANGLES,
                c.vertices as GLsizei,
                GL_UNSIGNED_SHORT,
                core::ptr::null(),
                c.instances as GLsizei,
            );
            glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
            glDeleteBuffers(1, &ib);
        } else {
            glDrawArraysInstanced(GL_TRIANGLES, c.first as GLint, c.vertices as GLsizei, c.instances as GLsizei);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "seed {:#x}: {:?}", c.seed, logs());
        glFlush();
        for k in 0..c.attrs.len() {
            glDisableVertexAttribArray(k as GLuint);
            glVertexAttribDivisor(k as GLuint, 0);
        }
        glUseProgram(0);
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
    }
    let draws = DRAWS.lock().unwrap().clone();
    let hw = with_platform(|p| p.hw.clone());
    Seen { draws, hw }
}

/* ---- the model --------------------------------------------------------------- */

/// Mesa's multiplier and shift for a divisor above 1
/// (tests/es3_pvr_divisor_magic.rs `magic_wide`): shift = ceil(log2 d),
/// multiplier = ceil(2^(shift + 31) / d), the 32-bit data word.
fn magic(d: u32) -> (u32, u32) {
    let shift = 32 - (d - 1).leading_zeros();
    let m = (1u64 << (shift + 31)).div_ceil(d as u64);
    (m as u32, shift)
}

/// The element the hardware fetches for instance id `id` of a stream with
/// divisor `d`: the multiply and shift of the stream's data words, one shift
/// for a power of two.
fn hw_index(d: u32, id: u32) -> u32 {
    match d {
        0 | 1 => id,
        d if d.is_power_of_two() => id >> d.trailing_zeros(),
        d => {
            let (m, shift) = magic(d);
            ((id as u64 * m as u64) >> (31 + shift)) as u32
        }
    }
}

macro_rules! ensure {
    ($cond:expr, $($arg:tt)*) => {
        if !$cond {
            return Err(format!($($arg)*));
        }
    };
}

/// The stream that loads vtxin register `reg..reg + 4`.
fn stream_at(streams: &[Stream], reg: u32) -> Option<&Stream> {
    streams.iter().find(|s| s.dst_reg <= reg && reg + 4 <= s.dst_reg + s.dma_size / 4)
}

/// Checks one draw against the model.
fn check(c: &Case, s: &Seen) -> Result<(), String> {
    ensure!(s.draws.len() == 1 && s.hw.len() == 1, "{} table draws (want one): {:?}", s.draws.len(), s.draws);
    let (d, hw) = (&s.draws[0], &s.hw[0]);
    ensure!(d.rc == 0, "the back end refused the draw: {d:?} ({:?})", logs());
    ensure!((d.instance_count, d.instance_id) == (c.instances, 0), "table draw {d:?}");
    ensure!(d.attribs.len() >= c.attrs.len(), "table attribs {:?}", d.attribs);
    for (k, a) in c.attrs.iter().enumerate() {
        let t = &d.attribs[k];
        ensure!(
            t.per_instance == (a.divisor != 0) && (a.divisor == 0 || t.divisor == a.divisor),
            "array {k}: the table has {t:?}, the draw divisor {}",
            a.divisor
        );
    }
    /* the index lists of the draw: one, or P1's chunks, adding up to it */
    let chunks: Vec<&Chunk> = hw.chunks.iter().filter(|ch| ch.vertices == c.vertices).collect();
    ensure!(!chunks.is_empty(), "no index list of {} vertices: {:?}", c.vertices, hw.lists);
    ensure!(
        chunks.len() == hw.chunks.iter().filter(|ch| ch.vertices != 6).count(),
        "an index list of another size: {:?}",
        hw.lists
    );
    let any_per_instance = c.attrs.iter().any(|a| a.divisor != 0);
    let mut first_instance = 0u32;
    for (ci, ch) in chunks.iter().enumerate() {
        let v = &ch.vertex;
        let what = format!("chunk {ci} (instances {first_instance}..+{})", ch.instances);
        if any_per_instance {
            ensure!(
                v.instance_id_reg == PVR_PDS_NO_ID_REG | PVR_PDS_ID_BASE_TO_STREAMS,
                "{what}: the instance register {:#x}: {v:?}",
                v.instance_id_reg
            );
            ensure!(v.instance_id_base == first_instance, "{what}: the instance base {}", v.instance_id_base);
        }
        for (k, a) in c.attrs.iter().enumerate() {
            let reg = d.attribs[k].vi_reg as u32;
            let Some(st) = stream_at(&v.streams, reg) else {
                return Err(format!("{what}: array {k} (reg {reg}) is in no stream: {:?}", v.streams));
            };
            /* a stream has one rate and one divisor: never coalesced across */
            ensure!(
                (st.per_instance == 1) == (a.divisor != 0),
                "{what}: array {k} (divisor {}) is in a stream with per_instance {}: {st:?}",
                a.divisor,
                st.per_instance
            );
            ensure!(
                st.divisor == a.divisor,
                "{what}: array {k} (divisor {}) is in a stream of divisor {}",
                a.divisor,
                st.divisor
            );
            /* its words: element 0 of a per-instance array (no base move); a per-vertex
             * array of glDrawArrays from `first` (the stream starts at it), of
             * glDrawElements at the buffer's start (the indices carry `first`) */
            let e = if a.divisor != 0 || c.elements { 0 } else { c.first as usize };
            let at = (reg - st.dst_reg) as usize;
            ensure!(
                st.first.len() >= at + 4,
                "{what}: array {k}: the stream's words are cut at {} (need {})",
                st.first.len(),
                at + 4
            );
            ensure!(
                st.first[at..at + 4] == elem(k, e),
                "{what}: array {k}: the stream's words {:?}, want element {e} {:?}",
                &st.first[at..at + 4],
                elem(k, e)
            );
            if a.divisor == 0 {
                continue;
            }
            /* the hardware's divide for every instance of the chunk */
            for j in 0..ch.instances {
                let id = first_instance + j;
                let got = hw_index(st.divisor, id);
                ensure!(
                    got == id / a.divisor,
                    "{what}: array {k}: instance {id} fetches element {got} (divisor {}, words {:?}), want {}",
                    a.divisor,
                    if st.divisor > 1 && !st.divisor.is_power_of_two() { Some(magic(st.divisor)) } else { None },
                    id / a.divisor
                );
            }
        }
        first_instance += ch.instances;
    }
    ensure!(first_instance == c.instances, "the index lists draw {first_instance} instances, want {}", c.instances);
    Ok(())
}

const MASTER_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
const DRAWS_RANDOM: u64 = 300;
const DRAWS_BIG: u64 = 6;

fn seed_of(i: u64) -> u64 {
    MASTER_SEED.wrapping_add(i.wrapping_mul(0xD1B5_4A32_D192_ED03))
}

fn run_and_check(seed: u64, big: bool) {
    let c = Case::generate(seed, big);
    report(&c, &run(&c));
}

/// Checks the draw; a failure panics with the seed and the draw.
fn report(c: &Case, seen: &Seen) {
    let (seed, big) = (c.seed, c.vertices == 30000);
    if let Err(e) = check(c, seen) {
        panic!(
            "seed {seed:#x} (big {big}; rerun with YAKOGL_DIVISOR_SEED={seed:#x}): {e}\ndraw: {c:#?}\ntable: {:#?}\nlists: {:?}",
            seen.draws,
            seen.hw.iter().map(|h| h.lists.clone()).collect::<Vec<_>>()
        );
    }
}

/// A seed given as YAKOGL_DIVISOR_SEED (hex with 0x, or decimal) and
/// YAKOGL_DIVISOR_BIG=1, for reproducing a failure.
fn seed_from_env() -> Option<(u64, bool)> {
    let s = std::env::var("YAKOGL_DIVISOR_SEED").ok()?;
    let seed = match s.strip_prefix("0x") {
        Some(h) => u64::from_str_radix(h, 16).ok()?,
        None => s.parse().ok()?,
    };
    Some((seed, std::env::var("YAKOGL_DIVISOR_BIG").is_ok_and(|v| v == "1")))
}

/// The model itself: the multiply and shift of the stream words equal
/// integer division for the divisors the draws use, so a mismatch above is
/// the back end's.
#[test]
fn the_model_is_integer_division() {
    let mut r = Rng::new(MASTER_SEED);
    let mut ds: Vec<u32> = DIVISORS.to_vec();
    ds.extend((0..200).map(|_| r.range(1, 0xffff) as u32));
    for d in ds {
        for id in (0..2000u32).chain([65_535, 65_536, 1 << 24, (1 << 31) - 1]) {
            assert_eq!(hw_index(d, id), id / d, "d {d}, id {id}");
        }
    }
}

/// A few hundred random draws, each checked against the model.
#[test]
fn random_draws_fetch_floor_i_over_divisor() {
    if let Some((seed, big)) = seed_from_env() {
        run_and_check(seed, big);
        return;
    }
    for i in 0..DRAWS_RANDOM {
        run_and_check(seed_of(i), false);
    }
}

/// Draws large enough that the back end chunks them (P1's instance base):
/// the base, not a base move of the streams, carries each chunk's first
/// instance.
#[test]
fn big_random_draws_are_chunked_with_the_instance_base() {
    if seed_from_env().is_some() {
        return;
    }
    for i in 0..DRAWS_BIG {
        let seed = seed_of(1000 + i);
        let c = Case::generate(seed, true);
        let seen = run(&c);
        let n = seen.hw[0].chunks.iter().filter(|ch| ch.vertices == c.vertices).count();
        assert!(n > 1, "seed {seed:#x}: a big draw is one chunk ({n}): {:?}", seen.hw[0].lists);
        report(&c, &seen);
    }
}
