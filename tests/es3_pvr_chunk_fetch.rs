// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: a chunked instanced draw fetches each per-instance element once
//! (docs/notes/instanceid-per-draw.md; issue #18).
//!
//! An instanced draw that alone would tile more than a kick's primitive
//! budget is split by instances (src/pvr/backend.rs `draw_sized`). The PDS
//! numbers the instances of each chunk from 0 again, so the back end moves
//! every per-instance stream's base on to the chunk's first instance F
//! (`attribs_advance_instances`). Instance i of the chunk must then fetch
//! element F + i of a divisor-1 attribute: the base move and P1's
//! `instance_id_modifier` (which adds a per-draw constant to the instance
//! number, `pvr_pds_render_vertex_program_ids_base`) together must add F
//! once. Both adding F fetches element 2F + i.
//!
//! The draw goes through the GL core into the REAL PowerVR back end, on a
//! platform written here, with stand-ins for the C generators that record
//! their inputs (as tests/es3_pvr_flatshade_lines.rs does). For each chunk
//! with a per-instance stream the test reads the stream's address, the
//! buffer object's GPU address and the instance base the vertex program was
//! generated with, and computes the element instance 0 of the chunk fetches:
//!
//! ```text
//! (stream address - buffer address) / stride + (instance base, if the
//! program adds it to the instance number; else 0)  ==  F
//! ```
//!
//! That runs once with a vertex shader that reads gl_InstanceID and once
//! with one that does not. The draw is split by the back end's real budget
//! (1.5M primitives a kick), not by the PRIM_BUDGET test knob, so it runs
//! without `test-knobs`. What the PDS DMA fetches on silicon is not visible
//! on the host: that is the AM62P instanced draws.
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

/// One DMA stream of a vertex-fetch program, as far as the fetch address
/// goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Stream {
    address: u64,
    stride: u32,
    per_instance: bool,
}

/// One vertex-fetch program as the generator was asked for it: its streams,
/// the gl_InstanceID register and the instance base (0 from the `_ids`
/// entry, which has none).
#[derive(Debug, Clone, Default)]
struct VertexRec {
    streams: Vec<Stream>,
    instance_id_reg: u32,
    instance_id_base: u32,
}

impl VertexRec {
    /// What the program adds to the instance number before the fetch: the
    /// base, which the generator ignores without an instance register.
    fn modifier(&self) -> u64 {
        if self.instance_id_reg != PVR_PDS_NO_ID_REG {
            self.instance_id_base as u64
        } else {
            0
        }
    }
}

/// One index list: its instance count (1 for a list that is not instanced)
/// and the vertex-fetch program generated last before it, the one its draw
/// runs.
#[derive(Debug, Clone)]
struct ListRec {
    instances: u32,
    vertex: VertexRec,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    vertex: VertexRec,
    lists: Vec<ListRec>,
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

/// The GPU addresses where `bytes` start in the GPU memory handed out so
/// far, at 4-byte steps.
fn gpu_addresses_of(bytes: &[u8]) -> Vec<u64> {
    with_platform(|p| {
        let mut found = Vec::new();
        for (&gpu, a) in &p.allocs {
            // SAFETY: the allocation's own bytes (u64 backing of `size` bytes or more).
            let mem = unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u8, a.size) };
            for at in (0..mem.len().saturating_sub(bytes.len()) + 1).step_by(4) {
                if mem[at..].starts_with(bytes) {
                    found.push(gpu + at as u64);
                }
            }
        }
        found
    })
}

/* ---- the PowerVR's command generators (driver/src/pvr, C), stood in for ------- */

const W_TERM: u32 = 0xE000_0000;
/// The coefficient stand-in's first word: its tag and (low 16 bits) how many
/// words it has.
const COEFF_TAG: u32 = 0xC0EF_0000;

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
        p.lists.push(ListRec { instances: instance_count.max(1), vertex });
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

/// The vertex-fetch program without an instance base: base 0.
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

/// P1's entry (driver/src/pvr_pds_render_gen.h): gl_InstanceID, and the
/// instance number the per-instance streams are fetched by, start at
/// `instance_id_base`. Records the streams and the base for the index list
/// that follows; its words are its inputs.
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
    let recorded = (0..n)
        .map(|i| {
            // SAFETY: the backend passes num_streams streams.
            let s = unsafe { *streams.add(i) };
            Stream { address: s.address, stride: s.stride, per_instance: s.per_instance != 0 }
        })
        .collect();
    with_platform(|p| p.vertex = VertexRec { streams: recorded, instance_id_reg, instance_id_base });
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
                instance_id_base,
                0,
            ];
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut context::GlContext = core::ptr::null_mut();

/// A per-instance offset (divisor 1) moves each instance's quad; the first
/// shader also reads gl_InstanceID, the second does not.
const VS_INSTANCE_ID: &str = "#version 300 es
layout(location = 0) in highp vec2 a_pos;
layout(location = 1) in highp float a_off;
out mediump vec4 v_col;
void main() {
    gl_Position = vec4(a_pos.x + a_off, a_pos.y, 0.0, 1.0);
    v_col = vec4(float(gl_InstanceID & 255) / 255.0, a_off, 0.0, 1.0);
}
";
const VS_PLAIN: &str = "#version 300 es
layout(location = 0) in highp vec2 a_pos;
layout(location = 1) in highp float a_off;
out mediump vec4 v_col;
void main() {
    gl_Position = vec4(a_pos.x + a_off, a_pos.y, 0.0, 1.0);
    v_col = vec4(a_off, 0.0, 0.0, 1.0);
}
";
const FS: &str = "#version 300 es
in mediump vec4 v_col;
layout(location = 0) out mediump vec4 o;
void main() { o = v_col; }
";

/// The context, set up once; the tests take turns. Each starts with nothing
/// pending and the recorder empty.
fn rig() -> MutexGuard<'static, ()> {
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
        }
    });
    // SAFETY: set once above, read under the lock; plain GL calls.
    unsafe {
        context::gl_context_set_current(CTX);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    with_platform(|p| {
        p.vertex = VertexRec::default();
        p.lists.clear();
    });
    g
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

/// Two triangles an instance: 2 primitives, so the back end's budget of
/// 1,500,000 primitives a kick (src/pvr/backend.rs PRIMS_PER_KICK_MAX) takes
/// 750,000 instances, and this draw is two chunks, the second starting at
/// instance 750,000.
const INSTANCES: u32 = 750_003;
const CHUNK: u32 = 750_000;
static QUAD: [f32; 12] = [-0.1, -0.1, 0.1, -0.1, 0.1, 0.1, -0.1, -0.1, 0.1, 0.1, -0.1, 0.1];

/// Bytes ahead of the per-instance array in its buffer object (the
/// attribute's offset), so the buffer's address is not the array's.
const LEAD: usize = 64;

/// The per-instance array: one float an instance, values no other memory
/// of the run holds.
fn offsets() -> Vec<f32> {
    (0..INSTANCES).map(|i| 1234.5 + i as f32 * 0.25).collect()
}

/// One glDrawArraysInstanced of [`QUAD`] with `vs`: the per-vertex position
/// from a client array, the per-instance offset (divisor 1) from a buffer
/// object at offset [`LEAD`]. Returns the per-instance array's GPU address.
fn draw(vs: &str) -> u64 {
    let prog = program(vs, FS);
    let mut bytes = vec![0u8; LEAD];
    bytes.extend(offsets().iter().flat_map(|f| f.to_le_bytes()));
    let mut vbo: GLuint = 0;
    // SAFETY: plain GL calls; the arrays outlive the draw (glFlush below).
    unsafe {
        glUseProgram(prog);
        glGenBuffers(1, &mut vbo);
        glBindBuffer(GL_ARRAY_BUFFER, vbo);
        glBufferData(GL_ARRAY_BUFFER, bytes.len() as GLsizeiptr, bytes.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glVertexAttribPointer(1, 1, GL_FLOAT, 0, 4, LEAD as *const c_void);
        glEnableVertexAttribArray(1);
        glVertexAttribDivisor(1, 1);
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, QUAD.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArraysInstanced(GL_TRIANGLES, 0, 6, INSTANCES as GLsizei);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glFlush();
    }
    let at = gpu_addresses_of(&bytes[LEAD..LEAD + 16]);
    // SAFETY: plain GL calls on valid handles.
    unsafe {
        glVertexAttribDivisor(1, 0);
        glDisableVertexAttribArray(1);
        glDeleteBuffers(1, &vbo);
        glDeleteProgram(prog);
    }
    assert_eq!(at.len(), 1, "the per-instance array is in GPU memory once: {at:x?}");
    at[0]
}

/// The chunks of the draw: for each index list whose vertex program has a
/// per-instance stream, its instance count and the element its instance 0
/// fetches, from the recorded state.
fn chunks(array: u64) -> Vec<(u32, u64)> {
    with_platform(|p| {
        p.lists
            .iter()
            .filter_map(|l| {
                let s = l.vertex.streams.iter().find(|s| s.per_instance)?;
                assert_eq!(s.stride, 4, "the per-instance stream's stride: {l:?}");
                assert!(s.address >= array, "the stream starts before the array: {l:?} (array 0x{array:x})");
                let skip = s.address - array;
                assert!(skip.is_multiple_of(4), "the stream starts inside an element: {l:?}");
                Some((l.instances, skip / 4 + l.vertex.modifier()))
            })
            .collect()
    })
}

/// Draw with `vs` and check that the chunks cover the instances in order
/// and that each chunk's instance 0 fetches the element of its first
/// instance F (F, not 2F: the base move and the modifier add it once).
/// Returns the chunk sizes.
fn check(vs: &str) -> Vec<u32> {
    let _g = rig();
    let array = draw(vs);
    let chunks = chunks(array);
    assert!(!chunks.is_empty(), "no index list with a per-instance stream ({:?})", logs());
    let mut first = 0u64;
    for &(n, element) in &chunks {
        assert_eq!(element, first, "the chunk at instance {first} fetches element {element}: {chunks:?}");
        first += n as u64;
    }
    assert_eq!(first, INSTANCES as u64, "the chunks draw every instance once: {chunks:?}");
    chunks.iter().map(|c| c.0).collect()
}

/* ---- the tests --------------------------------------------------------------- */

/// A shader that does not read gl_InstanceID: the draw is split, and the
/// second chunk's base move alone puts it on element 750,000.
#[test]
fn a_chunked_draw_fetches_each_per_instance_element_once() {
    assert_eq!(check(VS_PLAIN), [CHUNK, INSTANCES - CHUNK], "two chunks");
}

/// A shader that reads gl_InstanceID: whether the back end splits the draw
/// (P1's instance base) or draws it whole (main before P1), each chunk
/// fetches the element of its first instance, never base move + modifier.
#[test]
fn a_draw_that_reads_gl_instance_id_fetches_each_per_instance_element_once() {
    let sizes = check(VS_INSTANCE_ID);
    assert!(sizes == [INSTANCES] || sizes == [CHUNK, INSTANCES - CHUNK], "{sizes:?}");
}
