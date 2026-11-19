// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A host draw of a 16384-byte uniform block on the PowerVR build: plan
//! #378's acceptance c4 ("a host draw reads a full 16384-byte block at its
//! first and last dword"), item `ubo-backend`.
//!
//! Design: docs/notes/pvr-ubo-memory-load.md §8.5. The compiler reads block
//! members with `ld` from the address in a shared-register pair it names
//! (`ubo_addr_off1`); per draw the back end snapshots the stage's whole flat
//! array into the GENERAL arena, DMAs only its default prefix
//! (`uniform_sh_dwords`) into the shared registers, and writes the
//! snapshot's 64-bit address into the pair with one more DMA kick.
//!
//! The draws go through the REAL GL core and PowerVR back end on a platform
//! written here (as tests/es3_pvr_instanceid.rs: GPU memory is host memory,
//! the C generators are stand-ins). The common-store DMA stand-in records
//! every kick and does what the PDS would: it copies each kick's source
//! dwords into a shared-register file. The vertex program then runs in
//! `usc_exec` over that file and over the GENERAL arena as `ld`'s memory,
//! so "the first and last dword reads resolve to the right bytes" is what
//! the compiled program computes, not an address formula of the test's.
//!
//! The 16384-byte cases compile the stage with the online compiler
//! directly and a wrapper back end puts it, and a stage array of the
//! compiled layout, into an ordinary GL draw before the PowerVR back end
//! records it, so the draw carries the test's own stage array (a block of
//! the raised limit is in range on this build since item `ctx-limits`). The
//! 256-byte case goes through GL alone (glBindBufferRange, the core's own
//! flattened array).
//!
//! The block is `vec4 a[255]; b[255]; c[255]; d[255]; e[4];` (4096 dwords;
//! reflect.rs `fit_u8` caps an array at 255 elements). Block dword k holds
//! 0xA5000000 | k. Two default ints, `ia` and `ie`, declared before the
//! block, are the run-time indices and stay in the shared registers.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_ubo_draw_16k
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard};

use yakogl::backend::{GlesDraw, YakoglBackend};
use yakogl::gl::buffer::*;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::*;
use yakogl::gl::es3::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::render::{PvrRenderKick, PvrRenderStatic};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OSAL_QUERY_RENDER_STATIC};
use yakogl::pvr::backend::*;
use yakogl::pvr::ffi::*;
use yakogl::shaderc::{ShcShader, SHC_STAGE_FRAGMENT, SHC_STAGE_VERTEX};

mod usc_exec;

/* ---- the platform ------------------------------------------------------------ */

/* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE). */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

/// The USC's shared registers (chain.rs `LIMITS.shared`).
const SHARED_REGS: usize = 2048;

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

/// One common-store DMA program the back end built: its kicks
/// `(address, dwords, dest_offset)`, the shared-register file they fill
/// (unwritten registers read as `UNWRITTEN`), and, when a kick wrote a
/// 64-bit value into a pair, the GENERAL-arena allocation that value points
/// into, as `ld` would see it (its base address and its dwords).
#[derive(Clone)]
struct Dma {
    kicks: Vec<(u64, u32, u32)>,
    shared: Vec<u32>,
    regions: Vec<(u64, Vec<u32>)>,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    dmas: Vec<Dma>,
    /// `usc_common_size_bytes` of every vertex-stage PDS state.
    vs_common_bytes: Vec<u32>,
    log: Vec<String>,
}

const UNWRITTEN: u32 = 0xDEAD_5EED;

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The allocation holding device address `gpu`: (its base, its dwords).
fn alloc_of(p: &Platform, gpu: u64) -> Option<(u64, &[u32])> {
    let (&base, a) = p.allocs.range(..=gpu).next_back()?;
    if gpu - base >= a.size as u64 {
        return None;
    }
    // SAFETY: u64 backing of `size` bytes; read as dwords.
    Some((base, unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, a.size / 4) }))
}

/// `n` dwords at device address `gpu`.
fn words(p: &Platform, gpu: u64, n: usize) -> Vec<u32> {
    let (base, w) = alloc_of(p, gpu).unwrap_or_else(|| panic!("a DMA source 0x{gpu:x} outside every allocation"));
    let at = ((gpu - base) / 4) as usize;
    assert!(at + n <= w.len(), "a DMA of {n} dwords at 0x{gpu:x} runs past its allocation");
    w[at..at + n].to_vec()
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
    with_platform(|p| p.vs_common_bytes.push(ps.usc_common_size_bytes));
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
    // SAFETY: the backend's streams, buffers (8 data words a stream + 10,
    // n + 5 code words) and record.
    unsafe {
        core::ptr::copy_nonoverlapping([0x7E70_0000, 0, 0].as_ptr(), data_out, 3);
        for i in 0..n {
            let s = *streams.add(i);
            let w = [s.address as u32, (s.address >> 32) as u32, s.stride, s.dma_size, s.dst_reg, s.per_instance, 0, 0];
            core::ptr::copy_nonoverlapping(w.as_ptr(), data_out.add(3 + 8 * i), 8);
        }
        for i in 0..n + 2 {
            *code_out.add(i) = 0x7C00_0000 | i as u32;
        }
        *info = PvrPdsRenderVertexInfo {
            data_size_dw: 3 + 8 * n as u32,
            code_size_dw: n as u32 + 2,
            temps_used: 1,
            ..Default::default()
        };
    }
    0
}

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
    data[0] = 0xC0EF_0000 | (data.len() + code.len()) as u32;
    // SAFETY: the backend's buffers and record.
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

/// The common-store DMA program: its words are its kicks, and it records
/// what the PDS would do with them ([`Dma`]): each kick's source dwords,
/// read from the platform's memory now, into the shared-register file at
/// its `dest_offset`.
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_dma_program(
    kicks: *const PvrPdsDmaKick,
    num_kicks: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderColorUploadInfo,
) -> i32 {
    // SAFETY: the backend's kicks.
    let ks: Vec<PvrPdsDmaKick> = (0..num_kicks as usize).map(|i| unsafe { *kicks.add(i) }).collect();
    with_platform(|p| {
        let mut shared = vec![UNWRITTEN; SHARED_REGS];
        let mut regions = Vec::new();
        for k in &ks {
            let src = words(p, k.address, k.dwords as usize);
            let at = k.dest_offset as usize;
            assert!(at + src.len() <= SHARED_REGS, "a kick writes past sh{SHARED_REGS}: {} at {at}", src.len());
            shared[at..at + src.len()].copy_from_slice(&src);
            if k.dwords == 2 {
                let v = src[0] as u64 | (src[1] as u64) << 32;
                if let Some((base, w)) = alloc_of(p, v) {
                    regions.push((base, w.to_vec()));
                }
            }
        }
        p.dmas.push(Dma { kicks: ks.iter().map(|k| (k.address, k.dwords, k.dest_offset)).collect(), shared, regions });
    });
    // SAFETY: the backend's buffers and record.
    unsafe {
        for (i, k) in ks.iter().enumerate() {
            core::ptr::copy_nonoverlapping(
                [k.address as u32, k.dwords, k.dest_offset].as_ptr(),
                data_out.add(3 * i),
                3,
            );
            *code_out.add(i) = 0xD0A0_0000 | i as u32;
        }
        *code_out.add(ks.len()) = 0;
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

/* ---- the wrapper back end: a stage the GL core cannot link yet ---------------- */

/// What the wrapper puts into the next draws: a stage's compiled shader and
/// its flat array.
#[derive(Clone, Copy)]
struct Swap {
    stage: u32,
    sh: *const ShcShader,
    cpu: *const u32,
    dwords: u16,
}
// SAFETY: leaked, immutable data; the lock serialises every use.
unsafe impl Send for Swap {}

static SWAP: Mutex<Option<Swap>> = Mutex::new(None);
static DRAW_RC: Mutex<Vec<i32>> = Mutex::new(Vec::new());

unsafe extern "C" fn wrap_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the core's draw, for the call.
    let mut draw = unsafe { *d };
    if let Some(s) = *SWAP.lock().unwrap() {
        if s.stage == SHC_STAGE_VERTEX {
            draw.vs = s.sh;
            (draw.vs_uniform_cpu, draw.vs_uniform_dwords, draw.vs_uniform_gpu) = (s.cpu.cast(), s.dwords, 0);
        } else {
            draw.fs = s.sh;
            (draw.fs_uniform_cpu, draw.fs_uniform_dwords, draw.fs_uniform_gpu) = (s.cpu.cast(), s.dwords, 0);
        }
    }
    let real = yakogl_backend_pvr.draw.expect("the PowerVR back end draws");
    // SAFETY: a draw the core built, with at most the stage swapped.
    let rc = unsafe { real(&draw) };
    DRAW_RC.lock().unwrap().push(rc);
    rc
}

fn wrapper() -> &'static YakoglBackend {
    static ONCE: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    let p = *ONCE.get_or_init(|| {
        // SAFETY: a bitwise copy of a table of plain data and fn pointers.
        let mut b: YakoglBackend = unsafe { core::ptr::read(&yakogl_backend_pvr) };
        b.draw = Some(wrap_draw);
        Box::leak(Box::new(b)) as *const YakoglBackend as usize
    });
    // SAFETY: leaked above, never freed.
    unsafe { &*(p as *const YakoglBackend) }
}

/* ---- context, shaders, draws -------------------------------------------------- */

static LOCK: Mutex<()> = Mutex::new(());

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn context() -> MutexGuard<'static, ()> {
    static ONCE: std::sync::Once = std::sync::Once::new();
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // SAFETY: statics; the lock serialises every GL call of the file.
    unsafe {
        ONCE.call_once(|| {
            osal_set_ops(osal());
            yakogl::backend::g_backend = wrapper();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            let ctx = context::gl_context_create(64, 48);
            assert!(!ctx.is_null(), "{:?}", logs());
            context::gl_context_set_current(ctx);
        });
    }
    with_platform(|p| {
        p.dmas.clear();
        p.vs_common_bytes.clear();
    });
    DRAW_RC.lock().unwrap().clear();
    *SWAP.lock().unwrap() = None;
    g
}

/// The 16384-byte std140 block: 4 * 255 + 4 = 1024 vec4.
const BLOCK_16K: &str =
    "layout(std140) uniform B { vec4 a[255]; vec4 b[255]; vec4 c[255]; vec4 d[255]; vec4 e[4]; };\n";
/// A 256-byte block (16 vec4).
const BLOCK_256: &str = "layout(std140) uniform B { vec4 a[16]; };\n";
/// The run-time indices, default uniforms declared before the block.
const INDICES: &str = "uniform int ia;\nuniform int ie;\n";

/// The first and last dword of the 16384-byte block at constant indices
/// (x, y), and at the run-time indices `a[ia].x`, `e[ie].w` (z, w).
fn vs_16k() -> String {
    format!(
        "#version 300 es\nin vec4 p;\n{INDICES}{BLOCK_16K}\
         void main() {{ gl_Position = p + vec4(a[0].x, e[3].w, a[ia].x, e[ie].w); }}\n"
    )
}
fn fs_16k() -> String {
    format!(
        "#version 300 es\nprecision highp float;\n{INDICES}{BLOCK_16K}out vec4 o;\n\
         void main() {{ o = vec4(a[0].x, e[3].w, a[ia].x, e[ie].w); }}\n"
    )
}
/// The 256-byte block's first and last dword, the same way.
fn vs_256() -> String {
    format!(
        "#version 300 es\nin vec4 p;\n{INDICES}{BLOCK_256}\
         void main() {{ gl_Position = p + vec4(a[0].x, a[15].w, a[ia].x, a[ie].w); }}\n"
    )
}

const VS_PLAIN: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";
const FS_PLAIN: &str = "#version 300 es\nprecision mediump float;\nout vec4 o;\nvoid main() { o = vec4(1.0); }\n";

fn pat(k: usize) -> u32 {
    0xA500_0000 | k as u32
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = std::ffi::CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        glShaderSource(s, 1, &c.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "compile failed: {text}");
        s
    }
}

fn link(vs: &str, fs: &str) -> GLuint {
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        let mut info = [0 as c_char; 512];
        glGetProgramInfoLog(p, info.len() as GLsizei, core::ptr::null_mut(), info.as_mut_ptr());
        assert_eq!(ok, 1, "link failed: {:?}", CStr::from_ptr(info.as_ptr()));
        glUseProgram(p);
        p
    }
}

/// One triangle with the bound program, `p` = 0 at every vertex; the GL
/// error and the back end's return codes.
fn draw_triangle(prog: GLuint) -> (GLenum, Vec<i32>) {
    static ZERO: [f32; 12] = [0.0; 12];
    // SAFETY: plain GL calls; the array is static.
    unsafe {
        let loc = glGetAttribLocation(prog, c"p".as_ptr());
        assert!(loc >= 0, "the program reads p");
        glVertexAttribPointer(loc as GLuint, 4, GL_FLOAT, 0, 0, ZERO.as_ptr() as *const c_void);
        glEnableVertexAttribArray(loc as GLuint);
        while glGetError() != GL_NO_ERROR {}
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        glFinish();
        glDisableVertexAttribArray(loc as GLuint);
        (e, DRAW_RC.lock().unwrap().clone())
    }
}

/// The online compiler's record of `text`, kept (leaked) for the draws.
fn compile(stage: u32, text: &str) -> &'static ShcShader {
    let compile = yakogl::shaderc::online::SHC_ONLINE.compile.expect("the online back end compiles");
    let out: &'static mut ShcShader = Box::leak(Box::new(ShcShader::ZERO));
    let mut log = [0 as c_char; 512];
    let src = format!("{text}\0");
    // SAFETY: a NUL-terminated source, a real output, and a log of the size announced.
    let rc = unsafe { compile(stage, src.as_ptr() as *const c_char, out, log.as_mut_ptr(), log.len()) };
    // SAFETY: the compiler NUL-terminates the log.
    assert_eq!(rc, 0, "{:?}", unsafe { CStr::from_ptr(log.as_ptr()) });
    out
}

/// `name`'s dword offset in the stage's flat array (the record's uniform
/// table; a member is named alone or as `B.name`).
fn dword_off(sh: &ShcShader, name: &str) -> usize {
    // SAFETY: the record's table of num_uniforms entries.
    let us = unsafe { core::slice::from_raw_parts(sh.uniforms, sh.num_uniforms as usize) };
    let names: Vec<String> = us
        .iter()
        // SAFETY: NUL-terminated names.
        .map(|u| unsafe { CStr::from_ptr(u.name.as_ptr()) }.to_string_lossy().into_owned())
        .collect();
    let i = names
        .iter()
        .position(|n| n == name || n.ends_with(&format!(".{name}")) || n == &format!("{name}[0]"))
        .unwrap_or_else(|| panic!("no uniform {name} in {names:?}"));
    us[i].dword_off as usize
}

/// A stage array of `sh`'s layout: the block's dwords hold the pattern from
/// `a` on (`blk` dwords), `ia` = 0, `ie` = the last element of the last array.
fn stage_array(sh: &ShcShader, blk: usize, ie: i32) -> (&'static [u32], usize) {
    let n = sh.uniform_dwords as usize;
    let a = dword_off(sh, "a");
    assert!(a + blk <= n, "the block's {blk} dwords from {a} lie inside the {n}-dword array");
    let mut v = vec![0u32; n];
    for k in 0..blk {
        v[a + k] = pat(k);
    }
    v[dword_off(sh, "ia")] = 0;
    v[dword_off(sh, "ie")] = ie as u32;
    (Box::leak(v.into_boxed_slice()), a)
}

/// The record's address pair (`ubo_addr_off1`) and default prefix.
fn pair_of(sh: &ShcShader) -> (u32, u32) {
    let off1 = yakogl::shaderc::online::backend::pvr_ubo_addr_off1(sh) as u32;
    assert_ne!(off1, 0, "a stage that declares a block carries the pair");
    (off1 - 1, sh.uniform_sh_dwords as u32)
}

/// The one common-store DMA program that wrote the pair at `lo`; checks it
/// carries the default prefix and the pair and nothing of the block: no
/// kick's dwords reach `block_at`, the first block dword of the array.
fn the_dma(lo: u32, prefix: u32, block_at: usize) -> Dma {
    let dmas = with_platform(|p| p.dmas.clone());
    let with_pair: Vec<&Dma> = dmas.iter().filter(|d| d.kicks.iter().any(|k| k.1 == 2 && k.2 == lo)).collect();
    assert_eq!(
        with_pair.len(),
        1,
        "one DMA program writes the pair sh{lo}: {:?}",
        dmas.iter().map(|d| &d.kicks).collect::<Vec<_>>()
    );
    let d = with_pair[0].clone();
    let total: u32 = d.kicks.iter().map(|k| k.1).sum();
    assert!(total < 4096, "the shared-register DMA carries {total} dwords: {:?}", d.kicks);
    assert!(prefix as usize <= block_at, "the prefix ({prefix} dwords) ends before the block (dword {block_at})");
    assert!(d.kicks.iter().any(|k| k.1 == prefix), "one kick is the {prefix}-dword default prefix: {:?}", d.kicks);
    assert!(
        !d.shared.windows(4).any(|w| w.iter().enumerate().all(|(i, &x)| x == pat(i))),
        "the block's first dwords reached the shared registers"
    );
    d
}

/// The pair's value: the snapshot's address, dword-aligned, inside the
/// GENERAL heap, holding the stage array there.
fn pair_value(d: &Dma, lo: u32) -> u64 {
    let v = d.shared[lo as usize] as u64 | (d.shared[lo as usize + 1] as u64) << 32;
    assert_eq!(v & 3, 0, "the pair 0x{v:x} is dword-aligned");
    assert!((HEAP_GENERAL_BASE..HEAP_PDS_BASE).contains(&v), "the pair 0x{v:x} points into the GENERAL heap");
    v
}

/// The dword at byte address `addr` of what the DMA program's pair points into.
fn mem_word(d: &Dma, addr: u64) -> u32 {
    let (base, w) =
        d.regions.iter().find(|(b, w)| (*b..*b + 4 * w.len() as u64).contains(&addr)).expect("a mapped address");
    w[((addr - base) / 4) as usize]
}

/// The vertex program over the shared registers the DMA filled and the
/// GENERAL arena as `ld`'s memory: gl_Position's four dwords.
fn run_vs(sh: &ShcShader, d: &Dma) -> [u32; 4] {
    let image: Vec<u8> = sh.usc_words().iter().flat_map(|w| w.to_le_bytes()).collect();
    let mut inp = usc_exec::Inputs::new(1);
    inp.shared = d.shared.clone();
    inp.vtxin = vec![0; 128];
    inp.mem = d.regions.clone();
    let out =
        usc_exec::run_binary(&image, &inp, true).unwrap_or_else(|e| panic!("the vertex program does not run: {e}"));
    core::array::from_fn(|i| *out.get(&(i as u32)).unwrap_or_else(|| panic!("no vertex output {i}: {out:?}")))
}

/* ---- the 16384-byte block -------------------------------------------------- */

/// The vertex stage reads dword 0 and dword 4095 of a 16384-byte block by
/// constant and by run-time index: the draw records with no error, its
/// shared-register DMA holds the default prefix and the pair but no block
/// dword, the pair points at a copy of the whole array, and the program,
/// run over those registers and that memory, puts dword 0 and 4095 in
/// gl_Position.
#[test]
fn vertex_stage_reads_the_first_and_last_dword_of_a_16384_byte_block() {
    let _g = context();
    let prog = link(VS_PLAIN, FS_PLAIN);
    let sh = compile(SHC_STAGE_VERTEX, &vs_16k());
    let (lo, prefix) = pair_of(sh);
    let (arr, block_at) = stage_array(sh, 4096, 3);
    assert!(arr.len() >= 4096, "the array holds the block");
    *SWAP.lock().unwrap() = Some(Swap { stage: SHC_STAGE_VERTEX, sh, cpu: arr.as_ptr(), dwords: arr.len() as u16 });
    let (err, rcs) = draw_triangle(prog);
    assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rcs, [0], "the back end recorded the draw: {:?}", logs());

    let d = the_dma(lo, prefix, block_at);
    let snap = pair_value(&d, lo);
    for (k, &w) in arr.iter().enumerate() {
        assert_eq!(mem_word(&d, snap + 4 * k as u64), w, "the snapshot's dword {k}");
    }
    assert_eq!(mem_word(&d, snap + 4 * block_at as u64), pat(0));
    assert_eq!(mem_word(&d, snap + 4 * (block_at + 4095) as u64), pat(4095));
    let common = with_platform(|p| p.vs_common_bytes.clone());
    assert!(
        common.iter().any(|&c| c >= 4 * (lo + 2) && (c as usize) <= 4 * SHARED_REGS),
        "the vertex common store covers the pair and fits: {common:?}"
    );

    assert_eq!(run_vs(sh, &d), [pat(0), pat(4095), pat(0), pat(4095)], "a[0].x, e[3].w, a[ia].x, e[ie].w");
}

/// The same block in the fragment stage: the fragment DMA program carries
/// the prefix and the pair, not the block, and the pair's memory holds
/// dword 0 and 4095 where the program's reads look (array dword `a` and
/// `a + 4095`).
#[test]
fn fragment_stage_pair_points_at_the_first_and_last_dword_of_a_16384_byte_block() {
    let _g = context();
    let prog = link(VS_PLAIN, FS_PLAIN);
    let sh = compile(SHC_STAGE_FRAGMENT, &fs_16k());
    let (lo, prefix) = pair_of(sh);
    let (arr, block_at) = stage_array(sh, 4096, 3);
    *SWAP.lock().unwrap() = Some(Swap { stage: SHC_STAGE_FRAGMENT, sh, cpu: arr.as_ptr(), dwords: arr.len() as u16 });
    let (err, rcs) = draw_triangle(prog);
    assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rcs, [0], "the back end recorded the draw: {:?}", logs());

    let d = the_dma(lo, prefix, block_at);
    let snap = pair_value(&d, lo);
    assert_eq!(mem_word(&d, snap + 4 * block_at as u64), pat(0));
    assert_eq!(mem_word(&d, snap + 4 * (block_at + 4095) as u64), pat(4095));
    assert_eq!(mem_word(&d, snap + 4 * dword_off(sh, "ie") as u64), 3, "the default ie in the snapshot too");
    let ie = dword_off(sh, "ie");
    assert!(ie < prefix as usize, "the run-time index is in the shared prefix");
}

/// A stage array at the raised ceiling (MAX_STAGE_UNIFORM_DWORDS of the
/// coming raise, 1536 + 12 * 4096 = 50688 dwords): the snapshot takes all
/// of it, the shared registers still only the prefix and the pair.
#[test]
fn a_50688_dword_stage_array_draws_with_only_its_prefix_in_shared_registers() {
    let _g = context();
    let prog = link(VS_PLAIN, FS_PLAIN);
    let sh = compile(SHC_STAGE_VERTEX, &vs_16k());
    let (lo, prefix) = pair_of(sh);
    let (arr16, block_at) = stage_array(sh, 4096, 3);
    let mut big = arr16.to_vec();
    big.resize(50688, 0x5A5A_5A5A);
    let big: &'static [u32] = Box::leak(big.into_boxed_slice());
    *SWAP.lock().unwrap() = Some(Swap { stage: SHC_STAGE_VERTEX, sh, cpu: big.as_ptr(), dwords: 50688 });
    let (err, rcs) = draw_triangle(prog);
    assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rcs, [0], "the back end recorded the draw: {:?}", logs());
    let d = the_dma(lo, prefix, block_at);
    let snap = pair_value(&d, lo);
    assert_eq!(mem_word(&d, snap + 4 * 50687), 0x5A5A_5A5A, "the snapshot's last dword");
    assert_eq!(run_vs(sh, &d), [pat(0), pat(4095), pat(0), pat(4095)]);
}

/// A stage that declares the 16384-byte block and reads none of it (the
/// planner's case: a block declared in both stages, read in one): the pair
/// is laid out all the same, so the record carries its default prefix and
/// `ubo_addr_off1` and the shared-register DMA carries no block dword --
/// the whole-array rule (`uniform_sh_dwords == 0`) stays to stages that
/// declare no block. The defaults before the block still reach the program
/// through the shared prefix.
#[test]
fn a_stage_that_declares_a_block_it_never_reads_keeps_only_its_default_prefix_in_shared_registers() {
    let _g = context();
    let prog = link(VS_PLAIN, FS_PLAIN);
    let vs = format!(
        "#version 300 es\nin vec4 p;\n{INDICES}{BLOCK_16K}\
         void main() {{ gl_Position = p + vec4(float(ia), float(ie), 0.0, 0.0); }}\n"
    );
    let sh = compile(SHC_STAGE_VERTEX, &vs);
    let (lo, prefix) = pair_of(sh);
    assert_eq!(prefix, 2, "ia and ie, declared before the block");
    let (arr, block_at) = stage_array(sh, 4096, 3);
    *SWAP.lock().unwrap() = Some(Swap { stage: SHC_STAGE_VERTEX, sh, cpu: arr.as_ptr(), dwords: arr.len() as u16 });
    let (err, rcs) = draw_triangle(prog);
    assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rcs, [0], "the back end recorded the draw: {:?}", logs());

    /* one DMA program writes the pair, takes the 2-dword default prefix and
    no block dword: `the_dma` checks the total and the pattern */
    let d = the_dma(lo, prefix, block_at);
    let snap = pair_value(&d, lo);
    assert_eq!(mem_word(&d, snap + 4 * block_at as u64), pat(0), "the snapshot holds the block");
    assert_eq!(mem_word(&d, snap + 4 * (block_at + 4095) as u64), pat(4095));
    /* the program reads its defaults from the shared prefix, not from the
    block: gl_Position = p(0) + (float(ia), float(ie), 0, 0) */
    assert_eq!(run_vs(sh, &d), [0.0f32.to_bits(), 3.0f32.to_bits(), 0, 0]);
}

/* ---- through GL: a block the GL core links today -------------------------------- */

/// A 256-byte block bound with glBindBufferRange and drawn through GL: the
/// core's own flattened array is the snapshot the pair points at, and the
/// program reads the block's first and last dword from it.
#[test]
fn a_256_byte_block_drawn_through_gl_reads_its_first_and_last_dword() {
    let _g = context();
    let prog = link(&vs_256(), FS_PLAIN);
    let data: Vec<u8> = (0..64).flat_map(|k| pat(k).to_le_bytes()).collect();
    // SAFETY: plain GL calls; every pointer outlives its call.
    unsafe {
        let mut ub = 0;
        glGenBuffers(1, &mut ub);
        glBindBuffer(GL_UNIFORM_BUFFER, ub);
        glBufferData(GL_UNIFORM_BUFFER, 256, data.as_ptr() as *const c_void, GL_STATIC_DRAW);
        glBindBufferRange(GL_UNIFORM_BUFFER, 0, ub, 0, 256);
        glUniformBlockBinding(prog, glGetUniformBlockIndex(prog, c"B".as_ptr()), 0);
        glUniform1i(glGetUniformLocation(prog, c"ia".as_ptr()), 0);
        glUniform1i(glGetUniformLocation(prog, c"ie".as_ptr()), 15);
    }
    let (err, rcs) = draw_triangle(prog);
    assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rcs, [0], "the back end recorded the draw: {:?}", logs());
    /* the linked program's vertex record, as the back end saw it */
    let sh = compile(SHC_STAGE_VERTEX, &vs_256());
    let (lo, prefix) = pair_of(sh);
    let d = the_dma(lo, prefix, dword_off(sh, "a"));
    pair_value(&d, lo);
    assert_eq!(run_vs(sh, &d), [pat(0), pat(63), pat(0), pat(63)], "a[0].x, a[15].w, a[ia].x, a[ie].w");
}

/// A program without uniform blocks is as before: one uniform kick of the
/// whole array, no pair.
#[test]
fn a_default_only_program_dmas_its_whole_array_and_no_pair() {
    let _g = context();
    let vs = "#version 300 es\nin vec4 p;\nuniform vec4 u[3];\nvoid main() { gl_Position = p + u[0] + u[2]; }\n";
    let prog = link(vs, FS_PLAIN);
    let sh = compile(SHC_STAGE_VERTEX, vs);
    assert_eq!(yakogl::shaderc::online::backend::pvr_ubo_addr_off1(sh), 0, "no ld, no pair");
    let (err, rcs) = draw_triangle(prog);
    assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(rcs, [0], "{:?}", logs());
    let dmas = with_platform(|p| p.dmas.clone());
    let n = sh.uniform_dwords as u32;
    let vs_dma: Vec<&Dma> = dmas.iter().filter(|d| d.kicks.iter().any(|k| k.1 == n && k.2 == 0)).collect();
    assert_eq!(
        vs_dma.len(),
        1,
        "one DMA program takes the {n}-dword array: {:?}",
        dmas.iter().map(|d| &d.kicks).collect::<Vec<_>>()
    );
    assert_eq!(vs_dma[0].kicks.len(), 1, "one kick, the whole array, no pair: {:?}", vs_dma[0].kicks);
    assert_eq!(&vs_dma[0].shared[..12], &[0; 12], "u[0..3] as the core holds them (unset: zero)");
}
