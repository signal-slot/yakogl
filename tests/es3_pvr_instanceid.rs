// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: gl_InstanceID is the instance's index in an instanced draw the GL
//! core splits per instance (a divisor past `max_instance_divisor` 1, the
//! dEQP `instanced.*.mixed.*` cases; docs/notes/instanceid-per-draw.md, fix
//! P1). The back end no longer refuses such a draw: it passes
//! `GlesDraw::instance_id` as the base to the vertex PDS program
//! (`pvr_pds_render_vertex_program_ids_base`, Mesa's `instance_id_modifier`),
//! so instance 0 keeps the old program and instances 1..3 carry bases 1..3
//! in their PDS data. Run through the REAL back end and the online compiler
//! on a platform written here, with a stand-in for the PDS generator that
//! records its inputs and writes the base as one extra data dword (the real
//! generator's words are checked by tests/es3_pvr_instanceid_pds.rs).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_instanceid
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

/// What one draw pointed the PPP block at: the kick-USC program's words, the
/// coefficient program's words, and the varying sizes from its info.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DrawRec {
    kick: Vec<u32>,
    coeff: Vec<u32>,
    varyingsize: (u32, u32),
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    /// Generator calls: kick-USC, coefficient.
    kick_gens: u32,
    coeff_gens: u32,
    draws: Vec<DrawRec>,
    /// The vertex PDS generator's calls: (instance id register, base).
    vgens: Vec<(u32, u32)>,
    /// Each VDM state's vertex PDS data: heap offset and size in bytes.
    vdata: Vec<(u32, u32)>,
    /// The instance count of each index list.
    instances: Vec<u32>,
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

/// `n` words at PDS heap offset `off`, as the arena holds them now.
fn pds_words(p: &Platform, off: u32, n: usize) -> Vec<u32> {
    let gpu = HEAP_PDS_BASE + off as u64;
    let (&base, a) = p.allocs.range(..=gpu).next_back().expect("a PDS offset outside every allocation");
    let at = (gpu - base) as usize;
    assert!(at + 4 * n <= a.size, "PDS program at 0x{off:x} runs past its allocation");
    // SAFETY: inside the allocation (checked above); u64 backing, 4-aligned offset.
    unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(at) as *const u32, n) }.to_vec()
}

/* ---- the PowerVR's command generators (driver/src/pvr, C), stood in for ------- */

const W_TERM: u32 = 0xE000_0000;
/// Tags of the stand-ins' first words: a program's first word says what it is
/// and (low 16 bits) how many words it has.
const KICK_TAG: u32 = 0x4B00_0000;
const COEFF_TAG: u32 = 0xC0EF_0000;
/// The stand-in's extra data dword: the base in the low 16 bits.
const BASE_TAG: u32 = 0xBA5E_0000;

/// The PPP stand-in: records what the draw's PPP block points at.
#[no_mangle]
unsafe extern "C" fn pvr_ppp_build_block(
    input: *const PvrPppFragInputs,
    st: *const PvrPppState,
    out: *mut u32,
    out_count: *mut u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: the backend's inputs and state.
    let (fi, st) = unsafe { (*input, *st) };
    with_platform(|p| {
        let kick = pds_words(p, fi.pixel_program_code_offset, 9);
        assert_eq!(kick[0] & 0xFF00_0000, KICK_TAG, "the PPP block points at no kick-USC program");
        /* 0: the dummy program, no varying iterated */
        let coeff = if fi.varying_program_code_offset == 0 {
            Vec::new()
        } else {
            let head = pds_words(p, fi.varying_program_code_offset, 1)[0];
            assert_eq!(head & 0xFFFF_0000, COEFF_TAG, "the PPP block points at no coefficient program");
            pds_words(p, fi.varying_program_code_offset, (head & 0xFFFF) as usize)
        };
        p.draws.push(DrawRec { kick, coeff, varyingsize: (st.pds_varyingsize, st.usc_varyingsize) });
    });
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
    with_platform(|p| p.vdata.push((v.vs_pds_data_offset, v.vs_pds_data_size_bytes)));
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
    with_platform(|p| p.instances.push(instance_count));
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

/// Kick-USC stand-in: its words are its inputs.
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_kick_usc_program(
    fs_usc_exec_offset: u64,
    fs_temps: u32,
    sample_rate: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderKickUscInfo,
) -> i32 {
    with_platform(|p| p.kick_gens += 1);
    // SAFETY: the backend's buffers (8 data words, 1 code word) and record.
    unsafe {
        let w = [KICK_TAG | 9, fs_usc_exec_offset as u32, fs_temps, sample_rate, 0x4B4B_0004, 5, 6, 7];
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

/// The base-instance entry point: the `_ids` words, plus the base as one
/// more data dword when it is set and the shader reads gl_InstanceID.
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_vertex_program_ids_base(
    vs_usc_exec_offset: u64,
    vs_temps: u32,
    streams: *const PvrPdsVertexStream,
    num_streams: u32,
    has_slc_mcu_cache_controls: i32,
    vertex_id_reg: u32,
    instance_id_reg: u32,
    instance_id_base: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderVertexInfo,
) -> i32 {
    with_platform(|p| p.vgens.push((instance_id_reg, instance_id_base)));
    // SAFETY: the backend's buffers, sized for the base (2 data and 1 code
    // dword past the `_ids` sizes).
    unsafe {
        let rc = pvr_pds_render_vertex_program_ids(
            vs_usc_exec_offset,
            vs_temps,
            streams,
            num_streams,
            has_slc_mcu_cache_controls,
            vertex_id_reg,
            instance_id_reg,
            data_out,
            code_out,
            info,
        );
        if rc == 0 && instance_id_reg != PVR_PDS_NO_ID_REG && instance_id_base != 0 {
            *data_out.add((*info).data_size_dw as usize) = BASE_TAG | instance_id_base;
            (*info).data_size_dw += 1;
            (*info).code_size_dw += 1;
        }
        rc
    }
}

/// Coefficient stand-in: its words are its inputs (flags, then each
/// varying's components, interpolation and output offset), its info sizes
/// follow the varying count.
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
    with_platform(|p| p.coeff_gens += 1);
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

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The context and the backend, set up once; the tests take turns, each with
/// the recorder emptied.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            let ctx = context::gl_context_create(64, 48);
            assert!(!ctx.is_null(), "{:?}", logs());
        }
    });
    with_platform(|p| {
        p.vgens.clear();
        p.vdata.clear();
        p.instances.clear();
        p.draws.clear();
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = std::ffi::CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// The shape of the dEQP `mixed` cases: a per-vertex position, an array at
/// divisor 1, an array at divisor 2, and a shader that reads `gl_InstanceID`.
const VS_READS_ID: &str = "#version 300 es
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_each;
layout(location = 2) in vec4 a_pair;
out vec4 v;
void main() { gl_Position = a_pos; v = a_each + a_pair + vec4(float(gl_InstanceID)); }
";
const VS_PLAIN: &str = "#version 300 es
layout(location = 0) in vec4 a_pos;
layout(location = 1) in vec4 a_each;
layout(location = 2) in vec4 a_pair;
out vec4 v;
void main() { gl_Position = a_pos; v = a_each + a_pair; }
";
const FS: &str = "#version 300 es
precision mediump float;
in vec4 v;
out vec4 o;
void main() { o = v; }
";

static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0];
static ATTR: [f32; 16] = [0.5; 16];

/// A program of `vs` and the three arrays (divisors 0, 1 and 2), and
/// `instances` instances of one triangle drawn by glDrawArraysInstanced.
fn draw_mixed(vs: &str, instances: GLsizei) {
    // SAFETY: plain GL calls; the arrays are statics.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, FS));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program did not link");
        glUseProgram(p);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        for (loc, (ptr, div)) in [(TRI.as_ptr(), 0), (ATTR.as_ptr(), 1), (ATTR.as_ptr(), 2)].into_iter().enumerate() {
            glVertexAttribPointer(loc as GLuint, 4, GL_FLOAT, 0, 0, ptr as *const c_void);
            glEnableVertexAttribArray(loc as GLuint);
            glVertexAttribDivisor(loc as GLuint, div);
        }
        glDrawArraysInstanced(GL_TRIANGLES, 0, 3, instances);
        let err = glGetError();
        glFlush();
        for loc in 0..3 {
            glDisableVertexAttribArray(loc);
            glVertexAttribDivisor(loc, 0);
        }
        glUseProgram(0);
        glDeleteProgram(p);
        assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    }
}

/* ---- the tests --------------------------------------------------------------- */

/// An instanced draw with a divisor of 2 whose vertex shader reads
/// gl_InstanceID is drawn, not refused: one draw per instance, and the vertex
/// PDS of instance i carries base i (instance 0 the old program, no base).
#[test]
fn a_split_instanced_draw_that_reads_gl_instance_id_is_drawn_with_each_base() {
    let _g = rig();
    draw_mixed(VS_READS_ID, 4);
    with_platform(|p| {
        assert!(!p.log.iter().any(|l| l.contains("draw refused")), "{:?}", p.log);
        /* a clear quad may come first: the last four draws are the instances */
        let n = 4;
        assert!(p.vgens.len() >= n, "one vertex PDS program per instance: {:?}", p.vgens);
        let vgens = &p.vgens[p.vgens.len() - n..];
        for (i, &(reg, base)) in vgens.iter().enumerate() {
            assert_ne!(reg, PVR_PDS_NO_ID_REG, "instance {i}: gl_InstanceID has a register");
            assert_eq!(base, i as u32, "instance {i}: the base");
        }
        assert!(p.instances.len() >= n, "four index lists: {:?}", p.instances);
        assert!(p.instances.iter().all(|&n| n <= 1), "each is one instance: {:?}", p.instances);
        /* the PDS data of the draws: instances 1..3 end in their base */
        assert!(p.vdata.len() >= n);
        for (i, &(off, bytes)) in p.vdata[p.vdata.len() - n..].iter().enumerate() {
            let w = pds_words(p, off, (bytes / 4) as usize);
            let tagged = w.last().is_some_and(|&x| x & 0xFFFF_0000 == BASE_TAG);
            assert_eq!(tagged, i != 0, "instance {i}: base dword present: {w:x?}");
            if i != 0 {
                assert_eq!(w.last().unwrap() & 0xFFFF, i as u32, "instance {i}: PDS data base");
            }
        }
    });
}

/// A program that does not read gl_InstanceID gets no id register: the base
/// the core hands over changes nothing for it (the generator ignores it).
#[test]
fn a_program_without_gl_instance_id_has_no_id_register() {
    let _g = rig();
    draw_mixed(VS_PLAIN, 3);
    with_platform(|p| {
        /* a clear quad may come first: the last three are the instances */
        assert!(p.vgens.len() >= 3, "{:?}", p.vgens);
        let last = &p.vgens[p.vgens.len() - 3..];
        assert!(last.iter().all(|&(reg, _)| reg == PVR_PDS_NO_ID_REG), "{last:?}");
    });
}
