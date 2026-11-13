// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: a chunked instanced draw fetches each per-instance element once.
//! Instance i of a chunk that starts at instance F reads element F + i of a
//! divisor-1 attribute, never 2F + i (docs/notes/instanceid-per-draw.md, P1
//! and P3; issue #18).
//!
//! `draw_sized` (src/pvr/backend.rs) splits an instanced draw that alone
//! would tile more primitives than a kick holds (`PRIMS_PER_KICK_MAX`) into
//! chunks of instances. The PDS indexes a per-instance stream by the
//! instance number, which restarts at 0 in every chunk, so
//! `attribs_advance_instances` moves each per-instance stream's base on by
//! F * stride. Fix P1 adds Mesa's `instance_id_modifier` to the vertex PDS
//! program (`pvr_pds_render_vertex_program_ids_base`), which adds a base to
//! the instance number when the shader reads gl_InstanceID. If a chunk gets
//! both the base move and a modifier of F, it fetches element 2F + i: the
//! double advance found in #325's instanceid result (010be461). This file
//! guards against it: for each chunk the effective first element,
//!
//!     (stream base - buffer base) / stride + (modifier applied ? base : 0),
//!
//! must equal F, the instances the draw's earlier chunks drew. The element
//! at the stream base is read back from the buffer, whose element k holds
//! k, so the first term is read straight from the data the PDS would DMA.
//! The modifier is the `instance_id_base` the back end passes to the
//! `_ids_base` generator. The generator applies it only with an instance-id
//! register (pvr_pds_render_gen.c), so it counts only then.
//!
//! The draw is a triangle drawn 1,600,000 times (one primitive an instance).
//! That is past `PRIMS_PER_KICK_MAX` (1,500,000), so the back end chunks it
//! the way a real draw is chunked, without the `test-knobs` primitive
//! budget. It runs twice, with a vertex shader that reads gl_InstanceID and
//! with one that does not. On today's main only the second one is chunked
//! (two chunks, F = 0 and F = 1,500,000). `draw_sized` sends a draw whose
//! shader reads gl_InstanceID through whole, because the instance number
//! restarts in each chunk. So the first one pins the single-chunk case
//! (F = 0, no modifier). When P1 or P3 chunks such draws through the
//! modifier, the same check covers every chunk: the base move and the
//! modifier must not both add F.
//!
//! The words come from the REAL PowerVR back end on a platform written
//! here, with stand-ins for the C generators that record their inputs (as
//! tests/es3_pvr_flatshade_lines.rs does). The vertex fetch stand-in writes
//! each stream's address, stride, flags, instance-id register and base into
//! its data words. The VDM-state stand-in reads them back from the PDS heap
//! for each draw, and the index-list stand-in records each draw's instance
//! count. The shaders are ES 3.00 and compiled online.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_chunk_fetch
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
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawArraysInstanced;
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

/// One vertex stream as the vertex fetch generator was given it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct StreamRec {
    address: u64,
    stride: u32,
    per_instance: u32,
}

/// What one draw's VDM state and index list said: the vertex fetch
/// program's streams, its instance-id register and base (the modifier), and
/// the instance count of the draw's index list.
#[derive(Debug, Clone, Default)]
struct DrawRec {
    streams: Vec<StreamRec>,
    instance_id_reg: u32,
    instance_id_base: u32,
    instances: Option<u32>,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    draws: Vec<DrawRec>,
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

/// `n` words at GPU address `gpu`, as the platform's memory holds them now.
fn gpu_words(p: &Platform, gpu: u64, n: usize) -> Vec<u32> {
    let (&base, a) = p.allocs.range(..=gpu).next_back().expect("an address outside every allocation");
    let at = (gpu - base) as usize;
    assert!(at % 4 == 0 && at + 4 * n <= a.size, "0x{gpu:x} + {n} words runs past its allocation");
    // SAFETY: inside the allocation (checked above); u64 backing, 4-aligned offset.
    unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(at) as *const u32, n) }.to_vec()
}

/* ---- the PowerVR's command generators (driver/src/pvr, C), stood in for ------- */

const W_TERM: u32 = 0xE000_0000;
/// The coefficient stand-in's first word: its tag and (low 16 bits) how many
/// words it has.
const COEFF_TAG: u32 = 0xC0EF_0000;
/// The vertex fetch stand-in's data words per stream.
const VSTREAM_DW: usize = 8;

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

/// The VDM-state stand-in: reads the draw's vertex fetch data words (the
/// vertex stand-in's, see below) back from the PDS heap.
#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_vdm_state(
    dw: *mut u32,
    vs: *const PvrVdmVsState,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: the backend's state.
    let v = unsafe { *vs };
    with_platform(|p| {
        let n = v.vs_pds_data_size_bytes as usize / 4 / VSTREAM_DW;
        let w = gpu_words(p, HEAP_PDS_BASE + v.vs_pds_data_offset as u64, n * VSTREAM_DW);
        let mut rec = DrawRec { instance_id_reg: PVR_PDS_NO_ID_REG, ..Default::default() };
        for s in w.chunks(VSTREAM_DW) {
            rec.streams.push(StreamRec {
                address: s[0] as u64 | (s[1] as u64) << 32,
                stride: s[2],
                per_instance: s[5],
            });
            (rec.instance_id_reg, rec.instance_id_base) = (s[6], s[7]);
        }
        p.draws.push(rec);
    });
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

/// The index-list stand-in: records the instance count of the draw whose
/// VDM state came just before.
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
        let d = p.draws.last_mut().expect("an index list before any VDM state");
        assert!(d.instances.is_none(), "two index lists for one VDM state: {d:?}");
        d.instances = Some(instance_count.max(1));
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

/// The vertex fetch stand-in: per stream, its address, stride, DMA size,
/// register and per-instance flag, then the instance-id register and the
/// instance-id base (the modifier), 8 data words in all. The base is 0
/// unless an instance-id register is written, as the generator ignores it
/// then.
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
    let base = if instance_id_reg != PVR_PDS_NO_ID_REG { instance_id_base } else { 0 };
    // SAFETY: the backend's streams, buffers (8 data words a stream, n + 2
    // code words) and record.
    unsafe {
        for i in 0..n {
            let s = *streams.add(i);
            let w = [
                s.address as u32,
                (s.address >> 32) as u32,
                s.stride,
                s.dma_size,
                s.dst_reg,
                s.per_instance,
                instance_id_reg,
                base,
            ];
            core::ptr::copy_nonoverlapping(w.as_ptr(), data_out.add(VSTREAM_DW * i), VSTREAM_DW);
        }
        for i in 0..n + 2 {
            *code_out.add(i) = 0x7C00_0000 | i as u32;
        }
        *info = PvrPdsRenderVertexInfo {
            data_size_dw: (VSTREAM_DW * n) as u32,
            code_size_dw: n as u32 + 2,
            temps_used: 1,
            ..Default::default()
        };
    }
    0
}

/// `pvr_pds_render_vertex_program_ids` is the `_ids_base` program with base 0
/// (pvr_pds_render_gen.h).
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_vertex_program_ids(
    vs_usc_exec_offset: u64,
    vs_temps: u32,
    streams: *const PvrPdsVertexStream,
    num_streams: u32,
    has_slc_mcu_cache_controls: i32,
    vertex_id_reg: u32,
    instance_id_reg: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderVertexInfo,
) -> i32 {
    // SAFETY: the caller's arguments, passed on.
    unsafe {
        pvr_pds_render_vertex_program_ids_base(
            vs_usc_exec_offset,
            vs_temps,
            streams,
            num_streams,
            has_slc_mcu_cache_controls,
            vertex_id_reg,
            instance_id_reg,
            0,
            data_out,
            code_out,
            info,
        )
    }
}

/// Coefficient stand-in: its words are its inputs (flags, then each
/// varying's components, interpolation and output offset).
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_coeff_program(
    uses_w: i32,
    uses_z: i32,
    uses_pntc: i32,
    varyings: *const PvrPdsCoeffVarying,
    num_varyings: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsCoeffInfo,
) -> i32 {
    let n = num_varyings as usize;
    let mut data = vec![0u32, uses_w as u32, uses_z as u32, uses_pntc as u32];
    for i in 0..n {
        // SAFETY: the backend passes num_varyings records.
        let v = unsafe { *varyings.add(i) };
        data.extend([v.components, v.interp, v.vtxout_off]);
    }
    let code = [0xC0DE_0000 | num_varyings, 0xC0DE_FFFF];
    data[0] = COEFF_TAG | (data.len() + code.len()) as u32;
    // SAFETY: the backend's buffers (2 * (16 + 3) + 2 data words: 52 >= 4 +
    // 3 * 16) and record.
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), data_out, data.len());
        core::ptr::copy_nonoverlapping(code.as_ptr(), code_out, code.len());
        *info = PvrPdsCoeffInfo {
            data_size_dw: data.len() as u32,
            code_size_dw: code.len() as u32,
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

/* ---- the context ------------------------------------------------------------- */

const CANVAS: (u32, u32) = (64, 48);

/// Instances of the draw: past `PRIMS_PER_KICK_MAX` (src/pvr/backend.rs) at
/// one triangle an instance, so the back end chunks it.
const INSTANCES: u32 = 1_600_000;
/// `PRIMS_PER_KICK_MAX`: the chunk size of a one-triangle instance.
const PRIMS_PER_KICK_MAX: u32 = 1_500_000;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut context::GlContext = core::ptr::null_mut();
/// The programs (with gl_InstanceID, without) and the per-instance buffer.
static mut OBJS: (GLuint, GLuint, GLuint) = (0, 0, 0);

/// A per-vertex position and a per-instance float (divisor 1), and a use of
/// gl_InstanceID that the compiler cannot drop.
const VS_IID: &str = "#version 300 es
layout(location = 0) in highp vec2 a_pos;
layout(location = 1) in highp float a_k;
out highp vec4 v_col;
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); v_col = vec4(a_k, float(gl_InstanceID), 0.0, 1.0); }
";
/// The same without gl_InstanceID.
const VS_NO_IID: &str = "#version 300 es
layout(location = 0) in highp vec2 a_pos;
layout(location = 1) in highp float a_k;
out highp vec4 v_col;
void main() { gl_Position = vec4(a_pos, 0.0, 1.0); v_col = vec4(a_k, 0.0, 0.0, 1.0); }
";
const FS: &str = "#version 300 es
in highp vec4 v_col;
layout(location = 0) out highp vec4 o;
void main() { o = v_col; }
";

/// The context, the programs and the per-instance buffer (element k holds
/// k, 2 * INSTANCES elements), set up once; the tests take turns. Each starts with nothing pending
/// and the recorder empty.
fn rig() -> (MutexGuard<'static, ()>, (GLuint, GLuint, GLuint)) {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            /* the ES 3.00 programs are not in the offline table */
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0, CANVAS.1);
            assert!(!CTX.is_null(), "{:?}", logs());
            context::gl_context_set_current(CTX);
            /* twice the instances: a chunk that advances twice still lands
             * on an element, and the check names it */
            let k: Vec<f32> = (0..2 * INSTANCES).map(|k| k as f32).collect();
            let mut buf = 0;
            glGenBuffers(1, &mut buf);
            glBindBuffer(GL_ARRAY_BUFFER, buf);
            glBufferData(GL_ARRAY_BUFFER, (k.len() * 4) as GLsizeiptr, k.as_ptr() as *const c_void, GL_STATIC_DRAW);
            glBindBuffer(GL_ARRAY_BUFFER, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            OBJS = (program(VS_IID, FS), program(VS_NO_IID, FS), buf);
        }
    });
    // SAFETY: set once above, read under the lock; plain GL calls.
    let objs = unsafe {
        context::gl_context_set_current(CTX);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        OBJS
    };
    with_platform(|p| p.draws.clear());
    (g, objs)
}

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
        assert_eq!(ok, 1, "{vs}: link failed ({:?})", logs());
        p
    }
}

static TRI: [f32; 6] = [-0.5, -0.5, 0.5, -0.5, 0.0, 0.5];

/// One glDrawArraysInstanced of the triangle, INSTANCES times, with `prog`
/// and `a_k` from `buf` at divisor 1, then the frame's end. No clear: the
/// draw opens the frame and every recorded draw is one of its chunks.
fn draw(prog: GLuint, buf: GLuint) {
    // SAFETY: plain GL calls; TRI is a static.
    unsafe {
        glUseProgram(prog);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glBindBuffer(GL_ARRAY_BUFFER, buf);
        glVertexAttribPointer(1, 1, GL_FLOAT, 0, 4, core::ptr::null());
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribDivisor(1, 1);
        glEnableVertexAttribArray(1);
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, INSTANCES as GLsizei);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glFlush();
        glVertexAttribDivisor(1, 0);
        glDisableVertexAttribArray(1);
    }
}

/// Draw with `prog` and check every chunk: its per-instance stream's
/// effective first element is the instances drawn before it, and the chunks
/// add up to the draw. The chunk sizes, to pin how the draw was cut.
fn check_chunks(prog: GLuint, buf: GLuint, reads_instance_id: bool) -> Vec<u32> {
    draw(prog, buf);
    /* the back end's own quads (each frame's background) fetch no
     * per-instance stream: every draw that does is a chunk of ours */
    let draws: Vec<DrawRec> = with_platform(|p| core::mem::take(&mut p.draws))
        .into_iter()
        .filter(|d| d.streams.iter().any(|s| s.per_instance != 0))
        .collect();
    let mut first = 0u32;
    let mut sizes = Vec::new();
    for (c, d) in draws.iter().enumerate() {
        let what = format!("gl_InstanceID {reads_instance_id}, chunk {c} (F = {first}): {d:?}");
        assert_eq!(d.instance_id_reg != PVR_PDS_NO_ID_REG, reads_instance_id, "{what}: the instance-id register");
        let inst: Vec<&StreamRec> = d.streams.iter().filter(|s| s.per_instance != 0).collect();
        assert_eq!(inst.len(), 1, "{what}: one per-instance stream");
        let s = inst[0];
        assert_eq!(s.stride, 4, "{what}: the per-instance stride");
        /* (stream base - buffer base) / stride: element k of the buffer holds k */
        let base_elem = with_platform(|p| f32::from_bits(gpu_words(p, s.address, 1)[0]));
        assert_eq!(base_elem.fract(), 0.0, "{what}: the stream base is not on an element ({base_elem})");
        let modifier = if d.instance_id_reg != PVR_PDS_NO_ID_REG { d.instance_id_base } else { 0 };
        let effective = base_elem as u64 + modifier as u64;
        assert_eq!(
            effective, first as u64,
            "{what}: instance 0 of the chunk fetches element {effective} (base element {base_elem} + modifier \
             {modifier}), not F: an instance advanced twice or not at all"
        );
        let n = d.instances.expect("a chunk without an index list");
        sizes.push(n);
        first += n;
    }
    assert_eq!(first, INSTANCES, "the chunks draw every instance once: {sizes:?}");
    sizes
}

/* ---- the tests --------------------------------------------------------------- */

/// Without gl_InstanceID the draw is cut in two: the second chunk's base
/// moves on by F * stride and no modifier is added.
#[test]
fn a_chunked_draw_without_gl_instance_id_fetches_from_f() {
    let (_g, (_, prog, buf)) = rig();
    let sizes = check_chunks(prog, buf, false);
    assert_eq!(sizes, [PRIMS_PER_KICK_MAX, INSTANCES - PRIMS_PER_KICK_MAX], "the chunks");
}

/// With gl_InstanceID: today's main draws it whole (F = 0, no modifier).
/// Once it is chunked through the modifier, each chunk still has to fetch
/// from F, not 2F.
#[test]
fn a_draw_reading_gl_instance_id_fetches_each_element_once() {
    let (_g, (prog, _, buf)) = rig();
    let sizes = check_chunks(prog, buf, true);
    assert!(!sizes.is_empty(), "no chunk");
}
