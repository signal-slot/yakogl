// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The vertex-attribute and divisor shapes of the dEQP-GLES3 instanced cases
//! on the PowerVR, with and without the hardware instance divisors (P3, opt-in
//! cargo feature `pvr-instance-divisor`; docs/notes/instanceid-per-draw.md,
//! issue #18):
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_divisor_deqp
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,pvr-instance-divisor --test es3_pvr_divisor_deqp
//! ```
//!
//! The shapes are copied from VK-GL-CTS 93bca01 (`modules/gles3/functional/`):
//!
//! * `es3fInstancedRenderingTests.cpp`: `instanced.{draw_arrays_instanced,
//!   draw_elements_instanced}.{instance_id,attribute_divisor,mixed}.{1,2,4,20}_instances`
//!   (cases built at :663-:715, the shader at :204-:314, the arrays at :338-:451,
//!   the pointers and divisors at :532-:581, the divisors 3/2/1 of R/G/B at
//!   :55-:57) and `instanced.types.*` (:717-:742, the pointers of a matrix, int
//!   or uint attribute at :497-:530, 4 instances);
//! * `es3fDrawTests.cpp`: `draw.{draw_arrays_instanced,draw_elements_instanced}.
//!   instanced_attributes` (:265-:322, instance counts 1, 4, 11 from :48-:78,
//!   attribute 0 divisor 0, attributes 1 and 2 divisor 1) and
//!   `draw.instancing.draw_{arrays,elements}_instanced_grid_*` (:645-:780 for
//!   the arrays and divisors, :858-:884 for the grid sides 2, 5, 10, 32, 100).
//!
//! `draw.random` (:928-:1080: divisors 0, 1, 3, 129 chosen at random) is not a
//! fixed shape and is not here; tests/es3_pvr_divisor_random.rs holds its
//! divisors.
//!
//! Without the feature the PowerVR takes divisor 1 only (`max_instance_divisor`
//! 1): the GL core draws a shape with a divisor above 1 instance by instance,
//! each draw with `instance_id` and the arrays' elements as constants; a shape
//! with divisors 0 and 1 only is one draw. With the feature every shape is one
//! draw of all its instances, each per-instance array one stream with its GL
//! divisor, and the element fetched for instance i (chunk instance base + i,
//! divided) is floor(i / divisor) of the array the test filled, read from the
//! recorded stream address and stride. The harness is copied from
//! tests/es3_pvr_divisor_edges.rs (the real PowerVR table with its `draw`
//! recorded, stand-ins for the C command generators). The grids are 3x3 quads
//! here (the CTS uses 127x127): the vertex count does not enter the divisor
//! shape. What the GPU draws, the pictures the CTS compares, is the AM62P
//! board check, not this host test.
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
/* the no-feature arm uses only part of the helpers */
#![cfg_attr(not(feature = "pvr-instance-divisor"), allow(dead_code, unused_variables))]

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
    /// The words from `address` on (up to 65536), for the reference fetch.
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
                tail: tail_at(p, s.address, 1 << 16),
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

const FS: &str = "#version 300 es
precision mediump float;
in mediump vec4 v_color;
layout(location = 0) out mediump vec4 o_color;
void main() { o_color = v_color; }
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

fn program(vs: &str) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, FS));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{vs}: link failed ({:?})", logs());
        p
    }
}

/// The context, set up once; the tests take turns. Each starts with nothing
/// pending and the recorders empty.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &PVR_RECORDED;
            /* the ES 3.00 programs are not in the offline table */
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(64, 48);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: set once above, read under the lock; plain GL calls.
    unsafe {
        context::gl_context_set_current(CTX);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    DRAWS.lock().unwrap().clear();
    with_platform(|p| {
        p.hw.clear();
        p.current = None;
        p.vertex = VertexRec::default();
    });
    g
}

/* ---- the shapes -------------------------------------------------------------- */

#[derive(Debug, Clone, Copy, PartialEq)]
enum Class {
    Float,
    Int,
    Uint,
}

/// One instanced vertex attribute of a case: `slots` attribute locations from
/// `loc` (a matrix has one per column, all read from the same pointer with
/// `stride`), each reading `ncomp` components of an element of `words`
/// 32-bit words; `data` holds the elements.
#[derive(Debug, Clone)]
struct Attr {
    loc: u32,
    slots: u32,
    ncomp: u32,
    words: u32,
    stride: u32,
    class: Class,
    divisor: u32,
    data: Vec<u32>,
}

impl Attr {
    /// The words slot `_s` reads for element `e`.
    fn expect(&self, e: usize) -> Vec<u32> {
        self.data[e * self.words as usize..][..self.ncomp as usize].to_vec()
    }
}

/// One case: its non-instanced position array, its instanced attributes, its
/// shader, its draw.
struct Shape {
    cts: &'static str,
    /// The position array: `pos_comps` floats a vertex.
    pos: Vec<f32>,
    pos_comps: u32,
    attrs: Vec<Attr>,
    vs: String,
    instances: usize,
    /// Some: glDrawElementsInstanced with these ushort indices.
    indices: Option<Vec<u16>>,
    /// Arrays and indices in buffer objects (draw.*) or client memory
    /// (instanced.*).
    vbo: bool,
}

impl Shape {
    fn nverts(&self) -> u32 {
        match &self.indices {
            Some(ix) => ix.len() as u32,
            None => self.pos.len() as u32 / self.pos_comps,
        }
    }
    /// The divisors of the attributes' slots, in location order.
    fn slots(&self) -> Vec<(&Attr, u32)> {
        let mut v: Vec<(&Attr, u32)> = self.attrs.iter().flat_map(|a| (0..a.slots).map(move |s| (a, s))).collect();
        v.sort_by_key(|(a, s)| a.loc + s);
        v.retain(|(a, _)| a.divisor != 0);
        v
    }
    #[cfg(not(feature = "pvr-instance-divisor"))]
    /// A divisor above 1: without the feature the GL core splits the draw.
    fn splits_without_p3(&self) -> bool {
        self.attrs.iter().any(|a| a.divisor > 1)
    }
}

const GRID: usize = 3;

/// The CTS grid of `GRID`x`GRID` quads as a vertical bar of width 2/instances:
/// `instanced.draw_arrays_instanced` (es3fInstancedRenderingTests.cpp:375-405)
/// or, indexed, `draw_elements_instanced` (:340-:374).
fn bar_grid(indexed: bool, n: usize) -> (Vec<f32>, Option<Vec<u16>>) {
    let g = GRID as f32;
    let mut pos = vec![];
    if indexed {
        for y in 0..=GRID {
            for x in 0..=GRID {
                pos.push(-1.0 + x as f32 / g * 2.0 / n as f32);
                pos.push(-1.0 + y as f32 / g * 2.0);
            }
        }
        let mut ix = vec![];
        for y in 0..GRID {
            for x in 0..GRID {
                let (n00, n10) = (y * (GRID + 1) + x, y * (GRID + 1) + x + 1);
                let (n01, n11) = ((y + 1) * (GRID + 1) + x, (y + 1) * (GRID + 1) + x + 1);
                ix.extend([n00, n10, n01, n11, n01, n10].map(|v| v as u16));
            }
        }
        (pos, Some(ix))
    } else {
        for y in 0..GRID {
            for x in 0..GRID {
                let (fx0, fx1) = (-1.0 + x as f32 / g * 2.0 / n as f32, -1.0 + (x + 1) as f32 / g * 2.0 / n as f32);
                let (fy0, fy1) = (-1.0 + y as f32 / g * 2.0, -1.0 + (y + 1) as f32 / g * 2.0);
                pos.extend([fx0, fy0, fx1, fy0, fx0, fy1, fx1, fy1, fx0, fy1, fx1, fy0]);
            }
        }
        (pos, None)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    InstanceId,
    AttribDivisor,
    Mixed,
}

/// A GLSL type of `instanced.types.*` (es3fInstancedRenderingTests.cpp:719-727):
/// name, class, columns (1: not a matrix), rows (the scalar size of a vector).
const TYPES: [(&str, Class, u32, u32); 21] = [
    ("float", Class::Float, 1, 1),
    ("vec2", Class::Float, 1, 2),
    ("vec3", Class::Float, 1, 3),
    ("vec4", Class::Float, 1, 4),
    ("mat2", Class::Float, 2, 2),
    ("mat2x3", Class::Float, 2, 3),
    ("mat2x4", Class::Float, 2, 4),
    ("mat3x2", Class::Float, 3, 2),
    ("mat3", Class::Float, 3, 3),
    ("mat3x4", Class::Float, 3, 4),
    ("mat4x2", Class::Float, 4, 2),
    ("mat4x3", Class::Float, 4, 3),
    ("mat4", Class::Float, 4, 4),
    ("int", Class::Int, 1, 1),
    ("ivec2", Class::Int, 1, 2),
    ("ivec3", Class::Int, 1, 3),
    ("ivec4", Class::Int, 1, 4),
    ("uint", Class::Uint, 1, 1),
    ("uvec2", Class::Uint, 1, 2),
    ("uvec3", Class::Uint, 1, 3),
    ("uvec4", Class::Uint, 1, 4),
];

/// `pushVarCompAttrib` (:169-:184).
fn push_comp(c: Class, v: f32) -> u32 {
    match c {
        Class::Float => v.to_bits(),
        Class::Int => ((v * 100.0 - 50.0) as i32) as u32,
        Class::Uint => (v * 100.0) as u32,
    }
}

/// An `instanced.*` case (es3fInstancedRenderingTests.cpp): `kind` and the
/// `ty` of the colour attributes (`TYPES` index) over `n` instances.
fn instanced(indexed: bool, kind: Kind, ty: usize, n: usize, cts: &'static str) -> Shape {
    let (tname, class, cols, rows) = TYPES[ty];
    let type_size = cols * rows;
    let is_mat = cols > 1;
    let (pos, indices) = bar_grid(indexed, n);
    let mut attrs = vec![];
    let mut loc = 1;
    /* data of one colour attribute: `count` elements of `type_size` words */
    let colour = |count: usize, f: &dyn Fn(usize) -> f32| -> Vec<u32> {
        (0..count)
            .flat_map(|i| (0..type_size).map(move |j| (i, j)))
            .map(|(i, j)| push_comp(class, if j == 0 { f(i) } else { 0.0 }))
            .collect()
    };
    let slots = cols;
    let mk = |loc: u32, divisor: u32, data: Vec<u32>| Attr {
        loc,
        slots,
        ncomp: rows,
        words: type_size,
        stride: if is_mat { 4 * type_size } else { 0 },
        class,
        divisor,
        data,
    };
    if kind == Kind::AttribDivisor {
        /* the offset: a vec3 per instance, divisor 1 (:414-:423, :551-:554) */
        let data = (0..n).flat_map(|i| [(i as f32 * 2.0 / n as f32).to_bits(), 0, 0]).collect();
        attrs.push(Attr { loc, slots: 1, ncomp: 3, words: 3, stride: 0, class: Class::Float, divisor: 1, data });
        loc += 1;
        /* R, divisor 3 (:424-:432, :556-:557) */
        let r_inst = n.div_ceil(3);
        attrs.push(mk(loc, 3, colour(r_inst, &|i| i as f32 / r_inst as f32)));
        loc += slots;
    }
    /* G, divisor 2 (:434-:441, :560-:561) and B, divisor 1 (:443-:450, :563-:564) */
    let g_inst = n.div_ceil(2);
    attrs.push(mk(loc, 2, colour(g_inst, &|i| i as f32 * 2.0 / g_inst as f32)));
    loc += slots;
    attrs.push(mk(loc, 1, colour(n, &|i| 1.0 - i as f32 / n as f32)));
    if kind == Kind::InstanceId {
        attrs.clear();
    }
    /* the shader (:204-:314) */
    let sw = if rows == 1 && !is_mat { "" } else { ".x" };
    let ns = format!("{n}.0");
    let conv = |a: &str| match class {
        Class::Float if is_mat => format!("{a}[0][0]"),
        Class::Float => format!("{a}{sw}"),
        Class::Int => format!("(float({a}{sw}) - (-50.000)) / (100.000)"),
        Class::Uint => format!("(float({a}{sw}) - (0.000)) / (100.000)"),
    };
    let (mut decl, pos_e, r, g, b);
    let conv_decl = |name: &str, l: u32| format!("layout(location = {l}) in mediump {tname} {name};\n");
    match kind {
        Kind::InstanceId => {
            decl = String::new();
            pos_e = format!("a_position + vec4(float(gl_InstanceID) * 2.0 / {ns}, 0.0, 0.0, 0.0)");
            r = format!("float(gl_InstanceID)/{ns}");
            g = format!("float(gl_InstanceID)*2.0/{ns}");
            b = format!("1.0 - float(gl_InstanceID)/{ns}");
        }
        Kind::AttribDivisor => {
            decl = "layout(location = 1) in highp vec3 a_instanceOffset;\n".to_string();
            decl += &conv_decl("a_instanceR", attrs[1].loc);
            decl += &conv_decl("a_instanceG", attrs[2].loc);
            decl += &conv_decl("a_instanceB", attrs[3].loc);
            pos_e = "a_position + vec4(a_instanceOffset, 0.0)".to_string();
            r = conv("a_instanceR");
            g = conv("a_instanceG");
            b = conv("a_instanceB");
        }
        Kind::Mixed => {
            decl = conv_decl("a_instanceG", attrs[0].loc);
            decl += &conv_decl("a_instanceB", attrs[1].loc);
            pos_e = format!("a_position + vec4(float(gl_InstanceID) * 2.0 / {ns}, 0.0, 0.0, 0.0)");
            r = format!("float(gl_InstanceID)/{ns}");
            g = conv("a_instanceG");
            b = conv("a_instanceB");
        }
    }
    let vs = format!(
        "#version 300 es\nlayout(location = 0) in highp vec4 a_position;\n{decl}out mediump vec4 v_color;\nvoid main() {{\n    gl_Position = {pos_e};\n    v_color.r = {r};\n    v_color.g = {g};\n    v_color.b = {b};\n    v_color.a = 1.0;\n}}\n"
    );
    Shape { cts, pos, pos_comps: 2, attrs, vs, instances: n, indices, vbo: false }
}

/// `draw.*.instanced_attributes` (es3fDrawTests.cpp:265-:322): five triangles,
/// attribute 0 (4 floats a vertex) divisor 0, attribute 1 (2 floats) and
/// attribute 2 (3 floats) divisor 1, in buffers; `n` of 1, 4 or 11 instances.
fn draw_instanced_attributes(indexed: bool, n: usize) -> Shape {
    let nv = 15;
    let pos: Vec<f32> = (0..nv * 4).map(|i| 0.01 * i as f32).collect();
    let offs: Vec<u32> = (0..n * 2).map(|i| (0.5 + i as f32).to_bits()).collect();
    let cols: Vec<u32> = (0..n * 3).map(|i| (0.25 + i as f32).to_bits()).collect();
    let attrs = vec![
        Attr { loc: 1, slots: 1, ncomp: 2, words: 2, stride: 0, class: Class::Float, divisor: 1, data: offs },
        Attr { loc: 2, slots: 1, ncomp: 2, words: 3, stride: 0, class: Class::Float, divisor: 1, data: cols },
    ];
    let vs = "#version 300 es
layout(location = 0) in highp vec2 a_position;
layout(location = 1) in highp vec2 a_offset;
layout(location = 2) in mediump vec2 a_color;
out mediump vec4 v_color;
void main() { gl_Position = vec4(a_position + a_offset, 0.0, 1.0); v_color = vec4(a_color, 0.0, 1.0); }
"
    .to_string();
    Shape {
        cts: "es3fDrawTests.cpp:265-322",
        pos,
        pos_comps: 4,
        attrs,
        vs,
        instances: n,
        indices: indexed.then(|| (0..nv as u16).collect()),
        vbo: true,
    }
}

/// `draw.instancing.draw_{arrays,elements}_instanced_grid_SxS`
/// (es3fDrawTests.cpp:697-:770): six vertices (indices {0, 4, 3, 2, 1, 5}),
/// vec4 offset and vec4 colour with divisor 1, S*S instances.
fn draw_grid(indexed: bool, side: usize) -> Shape {
    let (cw, nv) = (2.0 / side as f32, 6);
    let pos = vec![
        0.0, 0.0, 0.0, 1.0, cw, 0.0, 0.0, 1.0, 0.0, cw, 0.0, 1.0, 0.0, cw, 0.0, 1.0, cw, 0.0, 0.0, 1.0, cw, cw, 0.0,
        1.0,
    ];
    let n = side * side;
    let mut offs = vec![];
    let mut cols = vec![];
    for x in 0..side {
        for y in 0..side {
            offs.extend([x as f32 * cw - 1.0, y as f32 * cw - 1.0, 0.0, 0.0].map(f32::to_bits));
            let c = if (x + y) % 2 == 0 { [0.0, 1.0, 0.0, 1.0] } else { [1.0, 1.0, 0.0, 1.0] };
            cols.extend(c.map(f32::to_bits));
        }
    }
    let attr = |loc, data| Attr { loc, slots: 1, ncomp: 4, words: 4, stride: 0, class: Class::Float, divisor: 1, data };
    let vs = "#version 300 es
layout(location = 0) in highp vec4 a_position;
layout(location = 1) in highp vec4 a_offset;
layout(location = 2) in mediump vec4 a_color;
out mediump vec4 v_color;
void main() { gl_Position = a_position + a_offset; v_color = a_color; }
"
    .to_string();
    Shape {
        cts: "es3fDrawTests.cpp:697-770, :858-884",
        pos,
        pos_comps: 4,
        attrs: vec![attr(1, offs), attr(2, cols)],
        vs,
        instances: n,
        indices: indexed.then(|| vec![0, 4, 3, 2, 1, 5]),
        vbo: true,
    }
    .with_vertices(nv)
}

impl Shape {
    fn with_vertices(self, nv: usize) -> Shape {
        assert_eq!(self.pos.len() / self.pos_comps as usize, nv);
        self
    }
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

fn bytes_of<T>(v: &[T]) -> &[u8] {
    // SAFETY: a plain-old-data slice of 16- or 32-bit words.
    unsafe { core::slice::from_raw_parts(v.as_ptr() as *const u8, core::mem::size_of_val(v)) }
}

/// Runs the case through the GL API; returns the table's and the back end's
/// records.
fn run(s: &Shape) -> (Vec<TableRec>, Vec<Hw>) {
    let _g = rig();
    let prog = program(&s.vs);
    let mut bufs = vec![];
    // SAFETY: plain GL calls; every pointer outlives its call (the client
    // arrays live in `s`).
    unsafe {
        glUseProgram(prog);
        /* where an array's pointer goes: a buffer offset 0, or client memory */
        let mut ptr_of = |bytes: &[u8]| -> *const c_void {
            if s.vbo {
                bufs.push(buffer(GL_ARRAY_BUFFER, bytes));
                core::ptr::null()
            } else {
                glBindBuffer(GL_ARRAY_BUFFER, 0);
                bytes.as_ptr() as *const c_void
            }
        };
        let p = ptr_of(bytes_of(&s.pos));
        glVertexAttribPointer(0, s.pos_comps as GLint, GL_FLOAT, 0, 0, p);
        glEnableVertexAttribArray(0);
        glVertexAttribDivisor(0, 0);
        for a in &s.attrs {
            let p = ptr_of(bytes_of(&a.data));
            for slot in 0..a.slots {
                let loc = a.loc + slot;
                let pc = (if a.stride != 0 { a.ncomp } else { a.words }) as GLint;
                match a.class {
                    Class::Float => glVertexAttribPointer(loc, pc, GL_FLOAT, 0, a.stride as GLsizei, p),
                    Class::Int => glVertexAttribIPointer(loc, pc, GL_INT, a.stride as GLsizei, p),
                    Class::Uint => glVertexAttribIPointer(loc, pc, GL_UNSIGNED_INT, a.stride as GLsizei, p),
                }
                glEnableVertexAttribArray(loc);
                glVertexAttribDivisor(loc, a.divisor);
            }
        }
        let n = s.instances as GLsizei;
        if let Some(ix) = &s.indices {
            let ip = if s.vbo {
                bufs.push(buffer(GL_ELEMENT_ARRAY_BUFFER, bytes_of(ix)));
                core::ptr::null()
            } else {
                ix.as_ptr() as *const c_void
            };
            glDrawElementsInstanced(GL_TRIANGLES, ix.len() as GLsizei, GL_UNSIGNED_SHORT, ip, n);
        } else {
            glDrawArraysInstanced(GL_TRIANGLES, 0, s.nverts() as GLsizei, n);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{}: {:?}", s.cts, logs());
        glFlush();
        for loc in 0..16 {
            glDisableVertexAttribArray(loc);
            glVertexAttribDivisor(loc, 0);
        }
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glBindBuffer(GL_ELEMENT_ARRAY_BUFFER, 0);
        glUseProgram(0);
        glDeleteProgram(prog);
        glDeleteBuffers(bufs.len() as GLsizei, bufs.as_ptr());
    }
    let draws = DRAWS.lock().unwrap().clone();
    let hw = with_platform(|p| p.hw.clone());
    for (i, d) in draws.iter().enumerate() {
        assert_eq!(d.rc, 0, "{}: draw {i} refused by the back end: {d:?} ({:?})", s.cts, logs());
    }
    assert_eq!(hw.len(), draws.len(), "a hardware record per table draw");
    (draws, hw)
}

/// The stream of `streams` that loads vtxin register `reg..reg + 4`.
fn stream_at(streams: &[Stream], reg: u32) -> &Stream {
    streams
        .iter()
        .find(|s| s.dst_reg <= reg && reg < s.dst_reg + s.dma_size / 4)
        .unwrap_or_else(|| panic!("no stream loads vtxin {reg}: {streams:?}"))
}

/// The CPU reference fetch: the four words a stream gives vtxin register
/// `reg` for element `e` (`address + e * stride`).
fn fetch(s: &Stream, reg: u32, e: usize, n: usize) -> Vec<u32> {
    let at = (e * s.stride as usize + 4 * (reg - s.dst_reg) as usize) / 4;
    s.tail[at..at + n].to_vec()
}

/// The shape's own index lists of a hardware record: the first draw of a
/// frame leaves its frame-start quad (6 vertices, 1 instance) ahead of them.
fn own(hw: &[Hw], i: usize) -> Vec<&Chunk> {
    let h = &hw[i];
    let skip = if i == 0 {
        assert_eq!(h.lists.first(), Some(&(6, 1)), "the frame-start quad: {:?}", h.lists);
        1
    } else {
        0
    };
    h.chunks.iter().skip(skip).collect()
}

/// Checks every instance of the GL draw: it gets from every instanced array
/// the element floor(instance / divisor), as the constant of a split draw
/// (instance_id) or by the reference fetch of the stream its chunk's vertex
/// program was given (instance base + i, divided). Returns the number of
/// hardware index lists (chunks) of every table draw and the instances fetched
/// through a stream.
fn check(s: &Shape, draws: &[TableRec], hw: &[Hw]) -> (Vec<usize>, usize) {
    let (mut chunks, mut fetched, mut seen) = (vec![], 0usize, 0usize);
    let slots = s.slots();
    for (di, (d, _)) in draws.iter().zip(hw).enumerate() {
        let own = own(hw, di);
        chunks.push(own.len());
        assert_eq!(d.arrays.len(), slots.len(), "{}: every instanced slot is an array of the draw: {d:?}", s.cts);
        let mut inst0 = d.instance_id as usize;
        for c in own {
            assert_eq!(c.vertices, s.nverts(), "{}: the chunk draws the whole mesh: {c:?}", s.cts);
            let base = c.vertex.instance_id_base as usize;
            for (a, (attr, slot)) in d.arrays.iter().zip(&slots) {
                let div = attr.divisor as usize;
                let what = format!("{}: location {}", s.cts, attr.loc + slot);
                match a {
                    Arr::Constant { value, .. } => {
                        assert_eq!(c.instances, 1, "{what}: a constant is a one-instance draw: {d:?}");
                        let got: Vec<u32> = value[..attr.ncomp as usize].iter().map(|f| f.to_bits()).collect();
                        assert_eq!(got, attr.expect(inst0 / div), "{what}: the constant of instance {inst0}");
                    }
                    Arr::Fetched { vi_reg, divisor } => {
                        assert_eq!(*divisor as usize, div, "{what}: the table's divisor");
                        let st = stream_at(&c.vertex.streams, *vi_reg as u32);
                        /* a chunk with a base past 0 needs it added to the streams; at 0 it is
                         * the same either way */
                        let flag = c.vertex.instance_id_reg & PVR_PDS_ID_BASE_TO_STREAMS;
                        let base = if flag != 0 {
                            base
                        } else {
                            assert_eq!((base, inst0), (0, 0), "{what}: a later chunk adds its base: {:?}", c.vertex);
                            0
                        };
                        assert_eq!(st.per_instance, 1, "{what}: {st:?}");
                        #[cfg(feature = "pvr-instance-divisor")]
                        assert_eq!(st.divisor as usize, div, "{what}: the PDS stream's divisor: {st:?}");
                        for i in 0..c.instances as usize {
                            let got = fetch(st, *vi_reg as u32, (base + i) / div, attr.ncomp as usize);
                            assert_eq!(got, attr.expect((inst0 + i) / div), "{what}, instance {}", inst0 + i);
                        }
                        if std::ptr::eq(a, &d.arrays[0]) {
                            fetched += c.instances as usize;
                        }
                    }
                }
            }
            inst0 += c.instances as usize;
            seen += c.instances as usize;
        }
    }
    assert_eq!(seen, s.instances, "{}: every instance is drawn once", s.cts);
    (chunks, fetched)
}

/// The per-instance streams of the draw's last own chunk, by register.
fn per_instance(hw: &[Hw]) -> Vec<Stream> {
    let c = own(hw, 0).pop().expect("a chunk");
    let mut v: Vec<Stream> = c.vertex.streams.iter().filter(|s| s.per_instance == 1).cloned().collect();
    v.sort_by_key(|s| s.dst_reg);
    v
}

/// The arm's expectation of one case: without the feature a divisor above 1
/// splits the draw per instance (a divisor of 1 does not); with it one draw
/// and one hardware draw, each instanced stream carrying its GL divisor.
fn expect_shape(s: &Shape) {
    let (draws, hw) = run(s);
    let (chunks, fetched) = check(s, &draws, &hw);
    let n = s.instances;
    let slots = s.slots();
    #[cfg(not(feature = "pvr-instance-divisor"))]
    {
        let split = s.splits_without_p3() && n > 1;
        assert_eq!(draws.len(), if split { n } else { 1 }, "{}: draws {draws:?}", s.cts);
        assert!(chunks.iter().all(|&c| c == 1), "{}: a hardware draw per table draw: {chunks:?}", s.cts);
        if split {
            for (i, d) in draws.iter().enumerate() {
                assert_eq!((d.instance_count, d.instance_id as usize), (1, i), "{}: draw {i}: {d:?}", s.cts);
                assert!(d.arrays.iter().all(|a| matches!(a, Arr::Constant { .. })), "{}: {d:?}", s.cts);
            }
            assert_eq!(fetched, 0, "{}: no stream fetches", s.cts);
        } else {
            assert_eq!(draws[0].instance_count as usize, n, "{}: {:?}", s.cts, draws[0]);
        }
    }
    #[cfg(feature = "pvr-instance-divisor")]
    {
        assert_eq!(draws.len(), 1, "{}: one draw: {draws:?}", s.cts);
        assert_eq!(draws[0].instance_count as usize, n, "{}: {:?}", s.cts, draws[0]);
        assert_eq!(chunks, [1], "{}: one hardware draw: {:?}", s.cts, hw[0].lists);
        assert_eq!(own(&hw, 0)[0].instances as usize, n, "{}: {:?}", s.cts, hw[0].lists);
        assert_eq!(fetched, if slots.is_empty() { 0 } else { n }, "{}: every instance through a stream", s.cts);
        let want: Vec<u32> = slots.iter().map(|(a, _)| a.divisor).collect();
        let got: Vec<u32> = per_instance(&hw).iter().map(|p| p.divisor).collect();
        if got.len() == want.len() {
            assert_eq!(got, want, "{}: the streams' divisors", s.cts);
        } else {
            /* streams of one buffer and divisor may be coalesced (one DMA for
             * several slots): the divisors seen are a subset */
            assert!(got.iter().all(|g| want.contains(g)), "{}: {got:?} vs {want:?}", s.cts);
        }
        let p = stream_at(&own(&hw, 0)[0].vertex.streams, 0);
        assert_eq!((p.per_instance, p.divisor), (0, 0), "{}: the position stream: {p:?}", s.cts);
    }
}

/* ---- the tests ----------------------------------------------------------------- */

const COUNTS: [usize; 4] = [1, 2, 4, 20];

const INST: &str = "es3fInstancedRenderingTests.cpp";

/// `instanced.{draw_arrays,draw_elements}_instanced.instance_id.N_instances`
/// (:215-:225, :663-:715): gl_InstanceID only, no instanced attribute.
#[test]
fn instanced_instance_id() {
    for indexed in [false, true] {
        for n in COUNTS {
            expect_shape(&instanced(indexed, Kind::InstanceId, 0, n, INST));
        }
    }
}

/// `instanced.*.attribute_divisor.N_instances` (:227-:287, :546-:565): an
/// offset (vec3, divisor 1), R (divisor 3), G (divisor 2), B (divisor 1).
#[test]
fn instanced_attribute_divisor() {
    for indexed in [false, true] {
        for n in COUNTS {
            expect_shape(&instanced(indexed, Kind::AttribDivisor, 0, n, INST));
        }
    }
}

/// `instanced.*.mixed.N_instances` (:215-:225, :227-:287, :560-:564):
/// gl_InstanceID for the position offset and R; G (divisor 2) and B
/// (divisor 1) from instanced attributes.
#[test]
fn instanced_mixed() {
    for indexed in [false, true] {
        for n in COUNTS {
            expect_shape(&instanced(indexed, Kind::Mixed, 0, n, INST));
        }
    }
}

/// `instanced.types.*` (:717-:742, :497-:530): attribute_divisor over 4
/// instances for float, vec2-4, the nine matrices, int, ivec2-4, uint and
/// uvec2-4.
#[test]
fn instanced_types() {
    for ty in 0..TYPES.len() {
        expect_shape(&instanced(false, Kind::AttribDivisor, ty, 4, INST));
    }
}

/// `draw.{draw_arrays_instanced,draw_elements_instanced}.instanced_attributes`
/// (es3fDrawTests.cpp:265-:322, instance counts :48-:78): divisors 0, 1, 1.
#[test]
fn draw_instanced_attributes_shape() {
    for indexed in [false, true] {
        for n in [1, 4, 11] {
            expect_shape(&draw_instanced_attributes(indexed, n));
        }
    }
}

/// `draw.instancing.draw_{arrays,elements}_instanced_grid_SxS`
/// (es3fDrawTests.cpp:858-:884, :697-:770): divisors 0, 1, 1. The sides 2, 5,
/// 10; the larger 32 and 100 (1024 and 10000 instances) only draw once, with
/// the feature.
#[test]
fn draw_instancing_grids() {
    for indexed in [false, true] {
        for side in [2, 5, 10] {
            expect_shape(&draw_grid(indexed, side));
        }
    }
}

#[cfg(feature = "pvr-instance-divisor")]
#[test]
fn draw_instancing_large_grids() {
    for indexed in [false, true] {
        for side in [32, 100] {
            expect_shape(&draw_grid(indexed, side));
        }
    }
}

/// Without the feature the table takes divisor 1 only.
#[cfg(not(feature = "pvr-instance-divisor"))]
#[test]
fn the_pvr_table_reports_max_instance_divisor_1() {
    assert_eq!(yakogl_backend_pvr.max_instance_divisor, 1);
    assert_eq!(PVR_RECORDED.max_instance_divisor, 1);
}
