// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Unit tests of the PowerVR backend's recording logic (frames, arenas,
//! per-frame tables, draw splitting, the kick descriptor, the C-ABI
//! validation), driven through its C entry points.
//!
//! # The generators are fakes
//!
//! The backend calls the driver's C command generators (ffi.rs). They live in
//! the driver repository, which a `cargo test` of this crate does not have
//! (the crate has no build.rs; the C parts are linked by CMake / baremetal.mk
//! / the host harness). This module therefore defines those symbols itself:
//! small recording fakes that emit words of the documented lengths (so the
//! backend's room checks see realistic sizes) and remember their inputs (the
//! PPP state, the vertex streams, the index lists). The tests check what the
//! backend decides and hands over, not the encodings; the real generators'
//! output is covered bit for bit by the host harness (tools/hostharness),
//! which links them. A build.rs compiling the driver sources only for tests
//! would make `cargo test` depend on a checkout outside this repository.
//!
//! # The platform is a small OSAL of its own
//!
//! testutil's mock OSAL has no submit/wait/query (its mock backend never
//! kicks). [`Rig`] installs a table with those, while holding a
//! [`TestGl`](crate::testutil::TestGl) guard: that serialises the tests with
//! every other test that touches the process-global OSAL pointer, and routes
//! log lines to its recorder. The rig puts testutil's table back before the
//! guard is dropped.

use super::*;
use crate::osal::OsalOps;
use crate::shaderc::ShcUniform;
use crate::testutil::TestGl;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::vec::Vec;
use std::{boxed::Box, format, string::String, vec};

/* ---- fake command generators ------------------------------------------------ */

#[derive(Default)]
struct GenLog {
    ppp: Vec<PvrPppState>,
    /// Each PPP block's pass type, in the same order as `ppp`.
    passtypes: Vec<u32>,
    streams: Vec<Vec<PvrPdsVertexStream>>,
    /// topology, index address, count, instances
    index_lists: Vec<(u32, u64, u32, u32)>,
    /// Per common-store DMA program: (source address, dwords, dest_offset).
    dma_kicks: Vec<Vec<(u64, u32, u32)>>,
    /// uses_pntc of each coefficient program built.
    coeff_pntc: Vec<i32>,
    /// uses_z of each coefficient program built.
    coeff_z: Vec<i32>,
    /// Per end-of-tile program: the emit words it was given (two per
    /// colour-attachment state, `num_rt` pairs).
    eot_emits: Vec<Vec<u32>>,
    /// The VDM state emitter refuses (as the real one does for a size that
    /// does not fit its field) while this is set.
    refuse_vdm_state: bool,
    /// The PPP block builder refuses the same way while this is set.
    refuse_ppp_block: bool,
    /// The index-list emitters refuse (a misaligned index address) while
    /// this is set.
    refuse_index_list: bool,
    /// The terminate-block builder refuses (a target past 16384 px) while
    /// this is set.
    refuse_terminate: bool,
    /// The ISP scissor word builder refuses (a coordinate past 16 bits)
    /// while this is set.
    refuse_scissor: bool,
}

static GEN: Mutex<Option<GenLog>> = Mutex::new(None);

fn with_gen<R>(f: impl FnOnce(&mut GenLog) -> R) -> R {
    let mut g = GEN.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(GenLog::default))
}

const PPP_WORDS: u32 = 12;
const W_PPP_STATE: u32 = 0xA000_0000;
const W_PDS_STATE: u32 = 0xB000_0000;
const W_VDM_STATE: u32 = 0xB100_0000;
const W_INDEX: u32 = 0xC000_0000;
const W_TERM: u32 = 0xE000_0000;

#[no_mangle]
unsafe extern "C" fn pvr_ppp_build_block(
    input: *const PvrPppFragInputs,
    st: *const PvrPppState,
    out: *mut u32,
    out_count: *mut u32,
    why: *mut PvrVdmFieldRefusal,
) -> i32 {
    if with_gen(|g| g.refuse_ppp_block) {
        // SAFETY: the backend passes its own refusal record and count.
        unsafe {
            *out_count = 0;
            *why = PvrVdmFieldRefusal {
                field: c"TA_STATE_PDS_SIZEINFO2.usc_sharedsize".as_ptr(),
                value: 512,
                max: 511,
                unit_bytes: 64,
            };
        }
        return PVR_VDM_FIELD_REFUSED;
    }
    let (fi, st) = unsafe { (*input, *st) };
    let w = [
        0x5050_5000,
        st.objtype,
        st.depth_cmp,
        st.dbias_enable as u32,
        st.dbias_index,
        st.scissor_enable as u32,
        st.scissor_index,
        st.color_write_disable as u32,
        fi.passtype as u32,
        fi.shared_reg_count,
        fi.pixel_program_code_offset,
        0,
    ];
    unsafe {
        core::ptr::copy_nonoverlapping(w.as_ptr(), out, w.len());
        *out_count = PPP_WORDS;
    }
    with_gen(|g| {
        g.ppp.push(st);
        g.passtypes.push(fi.passtype as u32);
    });
    PPP_WORDS as i32
}

#[no_mangle]
unsafe extern "C" fn pvr_ppp_build_terminate_block(
    width: u32,
    height: u32,
    out: *mut u32,
    why: *mut PvrVdmFieldRefusal,
) -> i32 {
    if with_gen(|g| g.refuse_terminate) {
        // SAFETY: the backend passes its own refusal record.
        unsafe {
            *why = PvrVdmFieldRefusal {
                field: c"TA_STATE_TERMINATE0.clip_right".as_ptr(),
                value: 512,
                max: 511,
                unit_bytes: 0,
            };
        }
        return PVR_VDM_FIELD_REFUSED;
    }
    unsafe { core::ptr::copy_nonoverlapping([0x7E7E_0000, width, height].as_ptr(), out, 3) };
    3
}

#[no_mangle]
unsafe extern "C" fn pvr_isp_build_scissor_word(
    xmin: u32,
    xmax: u32,
    ymin: u32,
    ymax: u32,
    out: *mut u32,
    why: *mut PvrVdmFieldRefusal,
) -> i32 {
    if with_gen(|g| g.refuse_scissor) {
        // SAFETY: the backend passes its own refusal record.
        unsafe {
            *why = PvrVdmFieldRefusal {
                field: c"IPF_SCISSOR_WORD_0.scw0_xmax".as_ptr(),
                value: 0x1_0000,
                max: 0xffff,
                unit_bytes: 0,
            };
        }
        return PVR_VDM_FIELD_REFUSED;
    }
    // Same packing as the real builder (csbgen rogue/ipf.xml:150-166).
    unsafe { core::ptr::copy_nonoverlapping([(xmin << 16) | xmax, (ymin << 16) | ymax].as_ptr(), out, 2) };
    2
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_ppp_state(
    dw: *mut u32,
    ppp_dev_addr: u64,
    word_count: u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    unsafe { core::ptr::copy_nonoverlapping([W_PPP_STATE | word_count, ppp_dev_addr as u32].as_ptr(), dw, 2) };
    2
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_pds_state(
    dw: *mut u32,
    ps: *const PvrVdmPdsState,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    let ps = unsafe { *ps };
    unsafe { core::ptr::copy_nonoverlapping([W_PDS_STATE, ps.pds_data_offset, ps.pds_code_offset].as_ptr(), dw, 3) };
    3
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_vdm_state(
    dw: *mut u32,
    vs: *const PvrVdmVsState,
    why: *mut PvrVdmFieldRefusal,
) -> i32 {
    if with_gen(|g| g.refuse_vdm_state) {
        // SAFETY: the backend passes its own refusal record.
        unsafe {
            *why = PvrVdmFieldRefusal {
                field: c"VDM_STATE5.vs_pds_data_size".as_ptr(),
                value: 64,
                max: 63,
                unit_bytes: 16,
            };
        }
        return PVR_VDM_FIELD_REFUSED;
    }
    let v = unsafe { *vs };
    let w = [W_VDM_STATE, v.vs_pds_data_offset, v.vs_pds_code_offset, v.vs_output_size_bytes, v.vs_input_size_bytes];
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
    unsafe { pvr_vdm_emit_index_list_instanced(dw, topology, addr, count, 0, why) }
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_index_list_instanced(
    dw: *mut u32,
    topology: u32,
    addr: u64,
    count: u32,
    instance_count: u32,
    why: *mut PvrVdmFieldRefusal,
) -> i32 {
    if with_gen(|g| g.refuse_index_list) {
        // SAFETY: the backend passes its own refusal record.
        unsafe {
            *why = PvrVdmFieldRefusal {
                field: c"VDMCTRL_INDEX_LIST1.index_base_addrlsb (address bits 1:0)".as_ptr(),
                value: 2,
                max: 0,
                unit_bytes: 0,
            };
        }
        return PVR_VDM_FIELD_REFUSED;
    }
    with_gen(|g| g.index_lists.push((topology, addr, count, instance_count)));
    let mut w = vec![W_INDEX | topology];
    if addr != 0 {
        w.push(addr as u32);
    }
    w.push(count);
    if instance_count > 1 {
        w.push(instance_count - 1);
    }
    unsafe { core::ptr::copy_nonoverlapping(w.as_ptr(), dw, w.len()) };
    w.len() as i32
}

#[no_mangle]
unsafe extern "C" fn pvr_vdm_emit_terminate(dw: *mut u32) -> u32 {
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
    with_platform(|p| p.kick_usc_rates.push((fs_usc_exec_offset, sample_rate)));
    with_platform(|p| p.kick_usc_temps.push((fs_usc_exec_offset, fs_temps)));
    /* the generator's own refusal (driver/src/pvr_pds_render_gen.c) */
    if fs_temps > PVR_PDS_DOUTU_MAX_TEMPS {
        return -1;
    }
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
    vs_temps: u32,
    streams: *const PvrPdsVertexStream,
    num_streams: u32,
    _has_slc_mcu_cache_controls: i32,
    _vertex_id_reg: u32,
    _instance_id_reg: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderVertexInfo,
) -> i32 {
    /* the generator's own refusals (driver/src/pvr_pds_render_gen.c) */
    if vs_temps > PVR_PDS_DOUTU_MAX_TEMPS || num_streams > PVR_PDS_VERTEX_MAX_STREAMS {
        return -1;
    }
    let st = unsafe { core::slice::from_raw_parts(streams, num_streams as usize) }.to_vec();
    let n = num_streams as usize;
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
    with_gen(|g| g.streams.push(st));
    0
}

#[no_mangle]
unsafe extern "C" fn pvr_pds_render_coeff_program(
    _uses_w: i32,
    uses_z: i32,
    uses_pntc: i32,
    _varyings: *const PvrPdsCoeffVarying,
    num_varyings: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsCoeffInfo,
) -> i32 {
    with_gen(|g| {
        g.coeff_pntc.push(uses_pntc);
        g.coeff_z.push(uses_z);
    });
    let (nd, nc) = (2 * (num_varyings + 2) + 2, num_varyings + 2);
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
    unsafe {
        let mut seen = Vec::new();

        for i in 0..num_kicks as usize {
            let k = *kicks.add(i);
            seen.push((k.address, k.dwords, k.dest_offset));
            core::ptr::copy_nonoverlapping(
                [k.address as u32, k.dwords, k.dest_offset].as_ptr(),
                data_out.add(3 * i),
                3,
            );
            *code_out.add(i) = 0xD0A0_0000 | i as u32;
        }
        with_gen(|g| g.dma_kicks.push(seen));
        *code_out.add(num_kicks as usize) = 0;
        *info = PvrPdsRenderColorUploadInfo { data_size_dw: 3 * num_kicks, code_size_dw: num_kicks + 1, temps_used: 1 };
    }
    0
}

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
    unsafe {
        let words = core::slice::from_raw_parts(emit_words, 2 * num_rt as usize).to_vec();
        with_gen(|g| g.eot_emits.push(words));
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the platform ----------------------------------------------------------- */

const BORDER_GPU: u64 = HEAP_GENERAL_BASE + 0x3F00_0000;
const TILE_BUFFER_GPU: u64 = HEAP_GENERAL_BASE + 0x3E00_0000;

struct Alloc {
    /// u64 backing: every u16/u32 view of the mapping is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    allocations: u32,
    /// Fail the n-th allocation from now (1 = the next).
    fail_alloc_in: u32,
    /// Submits so far (the fences).
    fence: i64,
    fail_submit: bool,
    /// Every kick: the descriptor and the VDM stream words it points at.
    kicks: Vec<(PvrRenderKick, Vec<u32>)>,
    flushed_bytes: usize,
    /// Allocations handed back (`gpu_free`), oldest first.
    frees: Vec<u64>,
    /// What the platform found wrong (a callback cannot panic: it is an
    /// `extern "C"` function); the rig fails the test with them.
    faults: Vec<String>,
    /// Every fragment kick-USC program built: (USC offset, sample rate).
    kick_usc_rates: Vec<(u64, u32)>,
    /// The temporaries every fragment kick program declared, in order.
    kick_usc_temps: Vec<(u64, u32)>,
    /// false: every kick completes at its submit (`completed` = the last
    /// fence). true: a kick completes only when a wait names it or a later
    /// one, so the waits the backend makes are seen.
    hold_completion: bool,
    completed_to: i64,
    /// Every fence the backend waited for, in order.
    waits: Vec<i64>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

fn heap_slot(base: u64) -> Option<usize> {
    match base {
        HEAP_GENERAL_BASE => Some(0),
        HEAP_PDS_BASE => Some(1),
        HEAP_USC_BASE => Some(2),
        _ => None,
    }
}

unsafe extern "C" fn p_gpu_alloc(heap_base: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let Some(h) = heap_slot(heap_base) else { return -EINVAL };
    with_platform(|p| {
        if p.fail_alloc_in > 0 {
            p.fail_alloc_in -= 1;
            if p.fail_alloc_in == 0 {
                return -ENOMEM;
            }
        }
        if size == 0 || size > 1 << 30 {
            return -ENOMEM;
        }
        let mut mem = vec![0xA5A5_A5A5_A5A5_A5A5u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = heap_base + 0x1000 + p.bump[h];
        p.bump[h] += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem, size });
        p.allocations += 1;
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    let m = unsafe { *mem };

    with_platform(|p| {
        if m.impl_.is_null() {
            return;
        }
        if p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, which is no live allocation", m.gpu));
        }
        p.frees.push(m.gpu);
    });
    0
}

unsafe extern "C" fn p_gpu_write(mem: *mut OsalGpuMem, off: usize, src: *const c_void, n: usize) -> i32 {
    let m = unsafe { *mem };
    if m.cpu.is_null() || off.checked_add(n).is_none_or(|e| e > m.size) {
        with_platform(|p| p.faults.push(format!("gpu_write of {n} bytes at {off} outside {m:?}")));
        return -EINVAL;
    }
    unsafe { core::ptr::copy_nonoverlapping(src as *const u8, (m.cpu as *mut u8).add(off), n) };
    0
}

unsafe extern "C" fn p_cache_flush(cpu: *mut c_void, n: usize) {
    with_platform(|p| {
        let inside = p.allocs.values().any(|a| {
            let base = a.mem.as_ptr() as usize;
            (cpu as usize) >= base && cpu as usize + n <= base + a.size
        });
        if !inside {
            p.faults.push(format!("cache_flush of {n} bytes at {cpu:p} outside every allocation"));
        }
        p.flushed_bytes += n;
    });
}

unsafe extern "C" fn p_cache_invd(_cpu: *mut c_void, _n: usize) {}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -(EINVAL as i64);
    }
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        if p.fail_submit {
            return -(EIO as i64);
        }
        let Some(a) = p.allocs.get(&k.geom.vdm_ctrl_stream_base) else {
            p.faults.push(format!("the kick's VDM stream 0x{:x} is no allocation", k.geom.vdm_ctrl_stream_base));
            return -(EINVAL as i64);
        };
        let words: &[u32] = unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, a.size / 4) };
        let Some(end) = words.iter().position(|&w| w == W_TERM) else {
            p.faults.push(String::from("an unterminated VDM stream"));
            return -(EINVAL as i64);
        };
        let vdm = words[..=end].to_vec();
        p.kicks.push((k, vdm));
        p.fence += 1;
        p.fence
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| {
        p.waits.push(fence);
        p.completed_to = p.completed_to.max(fence);
    });
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| if p.hold_completion { p.completed_to } else { p.fence })
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    if what != OSAL_QUERY_RENDER_STATIC || out_sz != size_of::<PvrRenderStatic>() {
        return -EINVAL;
    }
    let st = PvrRenderStatic {
        eot_doutu_src0: 0x1234,
        event_pixel_pds_info: 0x55,
        /* one end-of-tile program per attachment count, and a second set for
         * a render whose last attachment comes back from a tile buffer */
        eot_doutu_src0_n: [0, 0x1001, 0x1002, 0x1003],
        event_pixel_pds_info_n: [0, 0x61, 0x62, 0x63],
        eot_doutu_src0_tb_n: [0, 0x2001, 0x2002, 0x2003],
        event_pixel_pds_info_tb_n: [0, 0x71, 0x72, 0x73],
        tile_buffer_gpu: [TILE_BUFFER_GPU, TILE_BUFFER_GPU + 0x8000],
        border_gpu: BORDER_GPU,
        heap_base: [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE],
    };
    unsafe { *(out as *mut PvrRenderStatic) = st };
    0
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
extern "C" {
    /// testutil's log trampoline (a variadic function to Rust): the lines
    /// land in the TestGl recorder.
    fn yakogl_test_log(fmt: *const c_char, ...);
}
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
const RIG_LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = Some(yakogl_test_log);
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
const RIG_LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = None;

static RIG_OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: Some(p_gpu_write),
    gpu_read: None,
    cache_flush: Some(p_cache_flush),
    cache_invd: Some(p_cache_invd),
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: RIG_LOG,
    wait_vblank: None,
    lock: None,
    unlock: None,
    thread_id: None,
};

/// An initialised backend on [`RIG_OSAL`] with the fake generators.
struct Rig {
    prev_osal: *const OsalOps,
    gl: TestGl,
    fvs: Box<ShcShader>,
    ffs: Box<ShcShader>,
}

impl Rig {
    fn new() -> Rig {
        let gl = TestGl::new(4, 4);
        *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Platform::default());
        *GEN.lock().unwrap_or_else(|e| e.into_inner()) = Some(GenLog::default());
        // SAFETY: tests are serialised by the TestGl guard.
        let prev_osal = unsafe {
            let prev = crate::osal::g_osal;
            crate::osal::osal_set_ops(&RIG_OSAL);
            prev
        };
        pvr_backend_shutdown();
        /* the product's frame sets: a test that asked for others restores them */
        assert_eq!(pvr_backend_set_frame_arenas(4, 1024, 768), 0);
        for o in 0..PVR_BACKEND_OPT_COUNT as u32 {
            pvr_backend_set_option(o, 0);
        }
        pvr_debug_set_masked_clear(0);
        /* the product's default: the rig runs as the product does */
        pvr_split_zs(PVR_SPLIT_ZS_DEFAULT != 0);
        pvr_set_sprite_yup(PVR_SPRITE_YUP_DEFAULT);
        pvr_set_mrt_clear(PVR_MRT_CLEAR_DEFAULT);
        pvr_set_dbias_int(PVR_DBIAS_INT_DEFAULT);
        pvr_set_swap_drops_zs(PVR_SWAP_DROPS_ZS_DEFAULT);
        assert_eq!(pvr_backend_init(), 0);
        let mut vs = ShcShader::ZERO;
        vs.meta_off = SHC_NO_META;
        vs.vs_psize_off = SHC_NO_PSIZE;
        vs.vs_output_size = 4;
        vs.temps = 2;
        let mut fs = ShcShader::ZERO;
        fs.meta_off = SHC_NO_META;
        fs.num_outputs = 1;
        fs.temps = 3;
        let rig = Rig { prev_osal, gl, fvs: Box::new(vs), ffs: Box::new(fs) };
        rig.reset_records();
        rig
    }

    fn b(&self) -> &'static mut Backend {
        // SAFETY: serialised; no entry point is running.
        unsafe { &mut PVR.get().b }
    }

    fn fbload(&self) -> &'static mut FbLoad {
        // SAFETY: as `b`.
        unsafe { &mut PVR.get().fbload }
    }

    fn reset_records(&self) {
        with_gen(|g| *g = GenLog::default());
        with_platform(|p| p.kicks.clear());
    }

    /// A 64x64 target, one vec4 attribute from a buffer, `count` sequential
    /// vertices as a triangle list.
    fn draw(&self, count: u32) -> GlesDraw {
        let mut d = GlesDraw::ZERO;
        d.target_gpu = HEAP_GENERAL_BASE + 0x2000_0000;
        d.target_w = 64;
        d.target_h = 64;
        d.do_clear = true;
        d.prim = GLES_PRIM_TRIANGLES;
        d.index_count = count;
        d.vs = &*self.fvs;
        d.fs = &*self.ffs;
        d.vs_usc_gpu = HEAP_USC_BASE + 0x10_0000;
        d.fs_usc_gpu = HEAP_USC_BASE + 0x20_0000;
        d.attribs[0] = GlesAttrib {
            components: 4,
            stride: 16,
            offset: 8,
            buffer_gpu: HEAP_GENERAL_BASE + 0x1000_0000,
            ..GlesAttrib::ZERO
        };
        d.num_attribs = 1;
        d.viewport = [0, 0, 64, 64];
        d.depth_range = [0.0, 1.0];
        d.color_write = true;
        /* every channel written, as the GL core sends a draw (and a clear)
         * with glColorMask all on */
        d.color_mask = 0xf;
        d.clear_color_mask = 0xf;
        d.line_width = 1.0;
        d
    }

    fn kicks(&self) -> Vec<(PvrRenderKick, Vec<u32>)> {
        with_platform(|p| p.kicks.clone())
    }

    fn index_lists(&self) -> Vec<(u32, u64, u32, u32)> {
        with_gen(|g| g.index_lists.clone())
    }

    fn streams(&self) -> Vec<Vec<PvrPdsVertexStream>> {
        with_gen(|g| g.streams.clone())
    }

    fn ppp(&self) -> Vec<PvrPppState> {
        with_gen(|g| g.ppp.clone())
    }

    /// The common-store DMA kicks of every draw recorded so far.
    fn dma_kicks(&self) -> Vec<Vec<(u64, u32, u32)>> {
        with_gen(|g| g.dma_kicks.clone())
    }

    /// The emit words of every end-of-tile program built, in order.
    fn eot_emits(&self) -> Vec<Vec<u32>> {
        with_gen(|g| g.eot_emits.clone())
    }

    /// The dword the platform holds at GPU address `gpu`.
    fn gpu_u32(&self, gpu: u64) -> u32 {
        with_platform(|p| {
            let (base, a) = p.allocs.range(..=gpu).next_back().expect("an allocation at or below the address");
            let off = (gpu - base) as usize;
            assert!(off + 4 <= a.size, "0x{gpu:x} is past the allocation at 0x{base:x}");
            // SAFETY: the backing is u64-aligned and `off + 4` bytes long.
            unsafe { (a.mem.as_ptr() as *const u8).add(off).cast::<u32>().read_unaligned() }
        })
    }

    fn logs(&self) -> Vec<String> {
        self.gl.logs()
    }

    fn frees(&self) -> Vec<u64> {
        with_platform(|p| p.frees.clone())
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        pvr_debug_set_masked_clear(0);
        pvr_backend_shutdown();
        // SAFETY: serialised; testutil's table goes back before its guard drops.
        unsafe { crate::osal::osal_set_ops(self.prev_osal) };
        let faults = with_platform(|p| core::mem::take(&mut p.faults));
        if !faults.is_empty() && !std::thread::panicking() {
            panic!("platform faults:\n  {}", faults.join("\n  "));
        }
    }
}

fn draw(d: &GlesDraw) -> i32 {
    // SAFETY: a valid draw for the call.
    unsafe { pvr_backend_draw(d) }
}

/* ---- tests ------------------------------------------------------------------ */

#[test]
fn a_draw_and_a_flush_submit_one_terminated_kick() {
    let rig = Rig::new();
    let d = rig.draw(3);

    assert_eq!(draw(&d), 0);
    assert!(rig.kicks().is_empty(), "draws are recorded, not submitted");
    assert_eq!(pvr_backend_flush(), 0);
    let kicks = rig.kicks();
    assert_eq!(kicks.len(), 1);
    let (k, vdm) = &kicks[0];
    assert_eq!((k.width, k.height, k.samples), (64, 64, 1));
    /* the end-of-tile PDS info of the one-attachment program */
    assert_eq!(k.frag.event_pixel_pds_info, 0x61);
    assert_eq!(k.frag.num_rt, 1);
    assert_eq!(k.frag.isp_ctl, ISP_CTL_DEFAULT);
    /* PPP state, VDM state, one sequential index list, trailing PPP, terminate */
    let heads: Vec<u32> = vdm.iter().copied().filter(|w| w >> 24 >= 0xA0).collect();
    assert_eq!(
        heads,
        [W_PPP_STATE | PPP_WORDS, W_VDM_STATE, W_INDEX | PVR_VDM_TOPOLOGY_TRI_LIST, W_PPP_STATE | 3, W_TERM]
    );
    let il = rig.index_lists();
    assert_eq!(il, [(PVR_VDM_TOPOLOGY_TRI_LIST, rig.b().seq_idx.gpu, 3, 0)]);
    assert_eq!(rig.streams()[0][0].address, HEAP_GENERAL_BASE + 0x1000_0008);
    assert!(with_platform(|p| p.flushed_bytes) > 0);
    /* an empty flush submits nothing */
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks().len(), 1);
}

/// The fence a free is held for (CONFIG_YAKOGL_PVR_POOL_RECLAIM in the Zephyr
/// OSAL): the kick being recorded while a frame is open, the last submitted
/// one otherwise. Both sides: a free between frames (glmark2 deletes a scene
/// after its last swap) waits only for the last submitted kick, and a free
/// while a draw is recorded still waits for that draw's kick.
#[test]
fn a_free_waits_for_the_recording_kick_only_while_a_frame_is_open() {
    let rig = Rig::new();
    let d = rig.draw(3);

    assert_eq!(pvr_backend_free_fence(0), 0, "nothing recorded, nothing submitted");
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_free_fence(0), 1, "the open frame is kick 1");
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks().len(), 1);
    assert_eq!(pvr_backend_free_fence(1), 1, "after the flush only kick 1 can read it");
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_free_fence(1), 2);
}

/// Growing the sequential index buffer (a draw of more vertices than it
/// holds) gives the smaller one back instead of losing it.
#[test]
fn a_grown_sequential_index_buffer_gives_the_smaller_one_back() {
    let rig = Rig::new();
    assert_eq!(draw(&rig.draw(3)), 0);
    let small = rig.b().seq_idx.gpu;
    assert_eq!(rig.b().seq_idx_count, 1024);
    assert_eq!(draw(&rig.draw(5000)), 0);
    assert_eq!(rig.b().seq_idx_count, 5000);
    assert_ne!(rig.b().seq_idx.gpu, small);
    assert!(rig.frees().contains(&small), "the 1024-entry buffer was given back");
}

/// A shutdown gives back everything the backend holds (frame sets,
/// persistent programs and tables, depth/stencil buffers) after the last
/// kick, and a following init takes the same again: shutdown + init leaves
/// the pool's live bytes where they were. Before, the shutdown freed
/// nothing and the init zeroed the descriptors: ~21 MiB lost per restart.
#[test]
fn shutdown_gives_back_everything_and_a_restart_takes_the_same_again() {
    let rig = Rig::new();
    let live = || with_platform(|p| p.allocs.values().map(|a| a.size.div_ceil(4096) * 4096).sum::<usize>());
    /* a frame with a depth/stencil buffer, left open */
    let mut d = rig.draw(3);
    d.ds_persist = true;
    d.ds_id = (2 << 16) | 3;
    (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen) = (12, 1, 13, 1);
    d.depth_test = true;
    d.depth_write = true;
    assert_eq!(draw(&d), 0);
    assert!(rig.b().ds.iter().any(|s| !s.mem.impl_.is_null()), "the frame holds a depth/stencil buffer");
    let held = live();
    assert!(held > 7 * 1024 * 1024, "the frame sets alone are 7488 KiB: {held}");
    pvr_backend_shutdown();
    assert_eq!(live(), 0, "every block back after the shutdown: {:?}", rig.logs());
    assert_eq!(rig.kicks().len(), 1, "the open frame was submitted first");
    assert_eq!(pvr_backend_init(), 0);
    let again = live();
    assert_eq!(draw(&d), 0);
    assert_eq!(live(), held, "the restart takes what the first init and the frame took (init alone {again})");
    pvr_backend_shutdown();
    assert_eq!(live(), 0);
}

/// The owner report's backend half (`pvr_backend_pool_owners`) claims every
/// byte the backend holds: on the rig, the platform's live allocations are
/// all the backend's, so the lines' KiB equal them. And the frame arenas'
/// high-water mark shows a recorded frame (never printed before; CLAUDE.md:
/// every arena prints its high-water mark).
#[test]
fn the_backend_owner_lines_claim_every_live_byte_and_show_the_arena_high_water() {
    let rig = Rig::new();
    let d = rig.draw(3);
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let _ = rig.logs();
    let kib = pvr_backend_pool_owners();
    let live: usize = with_platform(|p| p.allocs.values().map(|a| a.size.div_ceil(4096) * 4096).sum());
    let logs = rig.logs();
    assert_eq!(kib as usize * 1024, live, "the backend owner lines do not balance: {logs:?}");
    /* the default since the frame-set measurement: 4 x (1024 + 768 KiB) */
    assert!(
        logs.iter()
            .any(|l| l
                == "pool owner backend frame sets: 28 blocks, 7488 KiB (general 4400, pds 3088, usc 0, elsewhere 0)"),
        "{logs:?}"
    );
    let hw = logs
        .iter()
        .find(|l| l.starts_with("pool owner backend frame arenas high-water:"))
        .expect("the high-water line");
    assert!(!hw.contains("general 0 of"), "a recorded frame used its general arena: {hw}");
}

/// Restart the rig's backend with `sets` frame sets of the given arena
/// sizes (`pvr_backend_set_frame_arenas`).
fn restart_with_arenas(sets: u32, gen_kib: u32, pds_kib: u32) {
    pvr_backend_shutdown();
    assert_eq!(pvr_backend_set_frame_arenas(sets, gen_kib, pds_kib), 0);
    assert_eq!(pvr_backend_init(), 0);
}

/// A draw whose indices are staged into the GENERAL arena: 10000 u16, 20000
/// bytes, more than a 16 KiB arena holds.
fn staged_index_draw(rig: &Rig, idx: &[u16]) -> GlesDraw {
    let mut d = rig.draw(3);
    d.index_cpu = idx.as_ptr() as *const c_void;
    d.index_bytes = (idx.len() * 2) as u32;
    d.index_count = idx.len() as u32;
    d
}

/// The frame-set knob's refusal (CONFIG_YAKOGL_PVR_ARENA_GEN_KB): a frame
/// that overflows a smaller GENERAL arena is refused, never written past
/// the arena, and the line names the arena and its size. The control, the
/// same draw with the default arenas, is drawn.
#[test]
fn a_smaller_general_arena_refuses_a_draw_it_cannot_hold_and_names_it() {
    let rig = Rig::new();
    let idx = vec![0u16; 10_000];
    let d = staged_index_draw(&rig, &idx);
    assert_eq!(draw(&d), 0, "the control: the default 4 MiB arena holds it");
    assert_eq!(pvr_backend_flush(), 0);

    restart_with_arenas(4, 16, 1024);
    let _ = rig.logs();
    let d = staged_index_draw(&rig, &idx);
    assert_eq!(draw(&d), -ENOMEM, "refused, not masked");
    assert!(rig.b().gen.bump as usize <= rig.b().gen.mem.size, "nothing written past the arena");
    let logs = rig.logs();
    assert!(
        logs.iter().any(|l| l
            == "pvr_backend: a draw (10000 vertices/indices) does not fit an empty frame: the general arena (16384 bytes) is too small; not drawn"),
        "{logs:?}"
    );
    pvr_backend_shutdown();
}

/// The default arenas since the frame-set measurement: 4 x (1024 KiB
/// GENERAL + 768 KiB PDS). What it costs, pinned: a draw's client-side
/// indices are staged whole into the GENERAL arena, so 1.5 MB of them no
/// longer fits an empty frame (-ENOMEM, the arena named), where the 4 MiB
/// arena drew it; 512 KiB still fits.
#[test]
fn the_default_arenas_are_1024_and_768_kib_and_bound_a_draws_client_indices() {
    let rig = Rig::new();
    assert_eq!((rig.b().nsets, rig.b().gen.mem.size, rig.b().pds.mem.size), (4, 1024 * 1024, 768 * 1024));
    let small = vec![0u16; 256 * 1024];
    let mut d = staged_index_draw(&rig, &small);
    d.index_buf_gpu = HEAP_GENERAL_BASE + 0x3000_0000;
    assert_eq!(draw(&d), 0, "512 KiB of client indices fit");
    assert_eq!(pvr_backend_flush(), 0);
    let big = vec![0u16; 750_000];
    let mut d = staged_index_draw(&rig, &big);
    d.index_buf_gpu = HEAP_GENERAL_BASE + 0x3000_0000;
    let _ = rig.logs();
    assert_eq!(draw(&d), -ENOMEM, "1.5 MB of client indices do not fit a 1 MiB arena");
    assert!(
        rig.logs().iter().any(|l| l
            == "pvr_backend: a draw (750000 vertices/indices) does not fit an empty frame: the general arena (1048576 bytes) is too small; not drawn"),
        "{:?}",
        rig.logs()
    );
}

/// Fewer, smaller frame sets: the owner report counts what was built, and
/// kicks rotate through the sets in use only.
#[test]
fn two_small_frame_sets_are_what_the_backend_builds_and_rotates() {
    let rig = Rig::new();
    restart_with_arenas(2, 64, 16);
    for _ in 0..3 {
        assert_eq!(draw(&rig.draw(3)), 0);
        assert_eq!(pvr_backend_flush(), 0);
        assert!(rig.b().cur < 2, "only the first two sets are used");
    }
    let _ = rig.logs();
    let kib = pvr_backend_pool_owners();
    let per_set = 16 + 4 + 64 + 64 + 4 + 4 + 4;
    let logs = rig.logs();
    assert!(
        logs.iter().any(|l| l
            == &format!(
                "pool owner backend frame sets: 14 blocks, {} KiB (general {}, pds {}, usc 0, elsewhere 0)",
                2 * per_set,
                2 * (64 + 64 + 4 + 4 + 4),
                2 * (16 + 4)
            )),
        "{logs:?}"
    );
    let live: usize = with_platform(|p| p.allocs.values().map(|a| a.size.div_ceil(4096) * 4096).sum());
    assert_eq!(kib as usize * 1024, live, "the restart left the first init's memory live: {logs:?}");
    pvr_backend_shutdown();
}

/// The setter's refusals leave the configuration as it was.
#[test]
fn the_frame_arena_setter_refuses_what_it_cannot_build() {
    let rig = Rig::new();
    /* running: the same configuration is accepted, another is refused */
    assert_eq!(pvr_backend_set_frame_arenas(4, 1024, 768), 0);
    assert_eq!(pvr_backend_set_frame_arenas(4, 4096, 1024), -EBUSY);
    pvr_backend_shutdown();
    for (sets, gen, pds) in
        [(0, 4096, 1024), (5, 4096, 1024), (4, 15, 1024), (4, 4097, 1024), (4, 4096, 7), (4, 4096, 1025)]
    {
        assert_eq!(pvr_backend_set_frame_arenas(sets, gen, pds), -EINVAL, "{sets} x ({gen} + {pds})");
    }
    assert_eq!(pvr_backend_init(), 0);
    assert_eq!(rig.b().nsets, 4, "the refusals changed nothing");
    assert_eq!((rig.b().gen.mem.size, rig.b().pds.mem.size), (1024 * 1024, 768 * 1024));
}

#[test]
fn a_sequential_triangle_list_past_65535_vertices_is_drawn_in_pieces() {
    let rig = Rig::new();
    let d = rig.draw(70_000);

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let il: Vec<u32> = rig.index_lists().iter().map(|l| l.2).collect();
    assert_eq!(il, [65535, 70_000 - 65535], "whole triangles per piece");
    let st = rig.streams();
    assert_eq!(st.len(), 2);
    assert_eq!(st[1][0].address - st[0][0].address, 65535 * 16, "the second piece starts at vertex 65535");
    assert_eq!(rig.kicks().len(), 1, "both pieces in one frame");
    assert!(rig.b().seq_idx_count >= 65535);
}

#[test]
fn strips_are_cut_with_their_shared_vertices_repeated() {
    let rig = Rig::new();
    let mut d = rig.draw(140_000);

    d.prim = GLES_PRIM_TRI_STRIP;
    assert_eq!(draw(&d), 0);
    let st = rig.streams();
    let firsts: Vec<u64> = st.iter().map(|s| (s[0].address - st[0][0].address) / 16).collect();
    let counts: Vec<u32> = rig.index_lists().iter().map(|l| l.2).collect();
    /* 65534 vertices per piece (an even number of triangles keeps the
     * winding), each piece restarting two vertices back */
    assert_eq!(firsts, [0, 65532, 131064]);
    assert_eq!(counts, [65534, 65534, 140_000 - 131064]);

    rig.reset_records();
    d.prim = GLES_PRIM_LINE_STRIP;
    d.index_count = 65536;
    assert_eq!(draw(&d), 0);
    let st = rig.streams();
    assert_eq!((st[1][0].address - st[0][0].address) / 16, 65534);
    assert_eq!(rig.index_lists().iter().map(|l| l.2).collect::<Vec<_>>(), [65535, 2]);

    rig.reset_records();
    d.prim = GLES_PRIM_LINES;
    d.index_count = 65536;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.index_lists().iter().map(|l| l.2).collect::<Vec<_>>(), [65534, 2]);
}

#[test]
fn a_piece_keeps_per_instance_streams_and_copies_only_its_staged_vertices() {
    let rig = Rig::new();
    let mut d = rig.draw(66_000);
    /* a staged per-vertex array (2 floats per vertex) and a per-instance one */
    let verts: Vec<f32> = (0..66_000 * 2).map(|i| i as f32).collect();
    let inst: Vec<f32> = vec![7.0; 3 * 4];
    d.attribs[0] = GlesAttrib {
        components: 2,
        stride: 8,
        cpu: verts.as_ptr() as *const c_void,
        cpu_bytes: (verts.len() * 4) as u32,
        ..GlesAttrib::ZERO
    };
    d.attribs[1] = GlesAttrib {
        vi_reg: 4,
        components: 4,
        stride: 16,
        per_instance: 1,
        cpu: inst.as_ptr() as *const c_void,
        cpu_bytes: 48,
        ..GlesAttrib::ZERO
    };
    d.num_attribs = 2;
    d.instance_count = 3;

    let gen0 = rig.b().gen.bump;
    assert_eq!(draw(&d), 0);
    let st = rig.streams();
    assert_eq!(st.len(), 2);
    /* piece 2's per-vertex stream is a copy of vertices 65535.. */
    let a = st[1][0].address;
    let gen = rig.b().gen;
    let off = (a - gen.mem.gpu) as usize;
    // SAFETY: the arena is a live CPU mapping; the copy lies inside it.
    let first = unsafe { ((gen.mem.cpu as *const u8).add(off) as *const f32).read_unaligned() };
    assert_eq!(first, (65535 * 2) as f32);
    assert_eq!(st[1][1].per_instance, 1);
    assert!(rig.index_lists().iter().all(|l| l.3 == 3), "every piece is instanced");
    /* the pieces copied about the array once, not once per piece */
    assert!((gen.bump - gen0) < (verts.len() * 4) as u32 + 64 * 1024);
}

#[test]
fn draws_that_cannot_be_cut_report_e2big() {
    let rig = Rig::new();
    let mut d = rig.draw(70_000);

    for prim in [GLES_PRIM_TRI_FAN, GLES_PRIM_LINE_LOOP] {
        d.prim = prim;
        assert_eq!(draw(&d), -E2BIG);
    }
    /* and a count that would need more pieces than the backend allows */
    d.prim = GLES_PRIM_TRIANGLES;
    d.index_count = i32::MAX as u32;
    assert_eq!(draw(&d), -E2BIG);
    assert_eq!(draw(&rig.draw(MAX_PIECES * 65535 + 1)), -E2BIG, "one piece too many");
    assert_eq!(draw(&rig.draw(MAX_PIECES * 65535)), 0, "the largest draw the backend splits");
    rig.reset_records();
    d.index_count = 70_000;
    let mut vs = *rig.fvs;
    vs.vs_vertex_id_reg1 = 5;
    d.vs = &vs;
    assert_eq!(draw(&d), -E2BIG, "gl_VertexID would restart per piece");
    assert!(rig.index_lists().is_empty());
    assert!(rig.logs().iter().any(|l| l.contains("cannot be split")), "{:?}", rig.logs());
    d.vs = &*rig.fvs;
    /* a batched second index list would be emitted once per piece */
    d.index_count = 70_000;
    d.index_count2 = 3;
    d.index_buf2_gpu = HEAP_GENERAL_BASE + 0x4000_0000;
    assert_eq!(draw(&d), -E2BIG);
    d.index_count2 = 0;
    /* 65535 vertices still fit one list */
    d.index_count = 65535;
    assert_eq!(draw(&d), 0);
}

#[test]
fn a_draw_that_does_not_fit_the_frame_goes_into_the_next_one() {
    let rig = Rig::new();
    /* the split, at the 4 MiB arena this test was sized for (the default is
     * 1 MiB since the frame-set measurement) */
    restart_with_arenas(4, 4096, 1024);
    let mut d = rig.draw(3);
    /* staged indices of 1.5 MB: two fit the 4 MB arena, the third does not */
    let idx = vec![0u16; 750_000];
    d.index_cpu = idx.as_ptr() as *const c_void;
    d.index_bytes = 1_500_000;
    d.index_count = 750_000;
    d.index_buf_gpu = HEAP_GENERAL_BASE + 0x3000_0000;

    assert_eq!(draw(&d), 0);
    d.do_clear = false;
    assert_eq!(draw(&d), 0);
    assert!(rig.kicks().is_empty());
    assert_eq!(draw(&d), 0, "retried in a fresh frame");
    assert_eq!(rig.kicks().len(), 1, "the full frame was submitted");
    assert_eq!(rig.b().ndraws, 2, "the new frame: the colour-load quad and the draw");
    assert_eq!(pvr_backend_flush(), 0);
    let indexed = rig.index_lists().iter().filter(|l| l.2 == 750_000).count();
    assert_eq!(indexed, 3);
}

#[test]
fn a_draw_larger_than_an_empty_frame_is_out_of_memory_and_leaves_the_frame_alone() {
    let rig = Rig::new();
    let mut d = rig.draw(3);

    assert_eq!(draw(&d), 0);
    let idx = vec![0u16; 3_000_000];
    let mut big = d;
    big.do_clear = false;
    big.index_cpu = idx.as_ptr() as *const c_void;
    big.index_bytes = 6_000_000;
    big.index_count = 3_000_000;
    big.index_buf_gpu = HEAP_GENERAL_BASE + 0x3000_0000;
    assert_eq!(draw(&big), -ENOMEM);
    assert_eq!(rig.kicks().len(), 1, "the retry submitted the frame");
    assert!(rig.logs().iter().any(|l| l.contains("does not fit an empty frame")), "{:?}", rig.logs());
    /* the failed draw left no trace in the new frame */
    let fresh = (rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws);
    assert_eq!(rig.b().ndraws, 1, "only the load quad");
    assert_eq!(draw(&big), -ENOMEM);
    assert_eq!(rig.kicks().len(), 1, "no retry when the frame holds no draw of the application");
    assert_eq!((rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws), fresh);
    /* a draw after it records normally */
    d.do_clear = false;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().ndraws, 2);
}

#[test]
fn a_failed_draw_leaves_no_trace_in_the_frame() {
    let rig = Rig::new();
    let d = rig.draw(3);
    let idx = [0u16; 3];
    let mut bad = d;
    bad.index_cpu = idx.as_ptr() as *const c_void;
    bad.index_bytes = u32::MAX; /* the bound check must not wrap */
    bad.index_count = 3;

    assert_eq!(draw(&d), 0);
    /* the arena cannot take it; the frame has an application draw, so it
     * is retried once in a new frame (which, as `bad` clears, needs no
     * load quad), where it fails again */
    assert_eq!(draw(&bad), -ENOMEM);
    assert_eq!(rig.kicks().len(), 1);
    assert_eq!((rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().term_gpu), (0, 0, 0, 0));
    /* the next draw records from the frame's start */
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().ndraws, 1);
    assert_eq!(rig.streams().len(), 2);
}

/// A VDM size field that does not fit refuses the draw: -E2BIG (not retried
/// in a new frame: a fresh frame would not make the value fit), and the
/// frame is as it was; the next draw records normally.
#[test]
fn a_refused_vdm_size_field_refuses_the_draw_and_leaves_the_frame_alone() {
    let rig = Rig::new();
    let d = rig.draw(3);
    /* no clear: it joins the open frame instead of starting the next one */
    let more = GlesDraw { do_clear: false, ..d };

    assert_eq!(draw(&d), 0);
    let before = (rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu);
    with_gen(|g| g.refuse_vdm_state = true);
    assert_eq!(draw(&more), -E2BIG);
    with_gen(|g| g.refuse_vdm_state = false);
    assert_eq!((rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu), before);
    assert!(rig.kicks().is_empty(), "a refusal submits nothing");
    assert_eq!(draw(&more), 0);
    assert_eq!(rig.b().ndraws, 2);
}

/// A PPP size field that does not fit refuses the draw the same way: -E2BIG,
/// nothing allocated or recorded, and the next draw records normally.
#[test]
fn a_refused_ppp_size_field_refuses_the_draw_and_leaves_the_frame_alone() {
    let rig = Rig::new();
    let d = rig.draw(3);
    /* no clear: it joins the open frame instead of starting the next one */
    let more = GlesDraw { do_clear: false, ..d };

    assert_eq!(draw(&d), 0);
    let before = (rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu);
    with_gen(|g| g.refuse_ppp_block = true);
    assert_eq!(draw(&more), -E2BIG);
    with_gen(|g| g.refuse_ppp_block = false);
    assert_eq!((rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu), before);
    assert!(rig.kicks().is_empty(), "a refusal submits nothing");
    assert_eq!(draw(&more), 0);
    assert_eq!(rig.b().ndraws, 2);
}

/// A non-size field that does not fit (here the index address's low bits)
/// refuses the draw the same way: -E2BIG, the frame as it was, and the next
/// draw records normally.
#[test]
fn a_refused_index_list_field_refuses_the_draw_and_leaves_the_frame_alone() {
    let rig = Rig::new();
    let d = rig.draw(3);
    /* no clear: it joins the open frame instead of starting the next one */
    let more = GlesDraw { do_clear: false, ..d };

    assert_eq!(draw(&d), 0);
    let before = (rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu);
    with_gen(|g| g.refuse_index_list = true);
    assert_eq!(draw(&more), -E2BIG);
    with_gen(|g| g.refuse_index_list = false);
    assert_eq!((rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu), before);
    assert!(rig.kicks().is_empty(), "a refusal submits nothing");
    assert_eq!(draw(&more), 0);
    assert_eq!(rig.b().ndraws, 2);
}

/// A refused terminate block refuses the draw that would carry it, with the
/// frame as it was.
#[test]
fn a_refused_terminate_block_refuses_the_draw_and_leaves_the_frame_alone() {
    let rig = Rig::new();
    let d = rig.draw(3);
    let more = GlesDraw { do_clear: false, ..d };

    assert_eq!(draw(&d), 0);
    let before = (rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu);
    with_gen(|g| g.refuse_terminate = true);
    assert_eq!(draw(&more), -E2BIG);
    with_gen(|g| g.refuse_terminate = false);
    assert_eq!((rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu), before);
    assert!(rig.kicks().is_empty(), "a refusal submits nothing");
    assert_eq!(draw(&more), 0);
    assert_eq!(rig.b().ndraws, 2);
}

/// A refused ISP scissor word (a coordinate that does not fit its 16-bit
/// field) refuses the draw the same way: -E2BIG, the frame as it was, and
/// the next draw records normally.
#[test]
fn a_refused_scissor_word_refuses_the_draw_and_leaves_the_frame_alone() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.scissor = true;
    d.scissor_rect = [4, 4, 8, 8]; /* not the whole target: reaches the scissor table */
    let more = GlesDraw { do_clear: false, ..d };

    assert_eq!(draw(&d), 0);
    let before = (rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu);
    with_gen(|g| g.refuse_scissor = true);
    assert_eq!(draw(&more), -E2BIG);
    with_gen(|g| g.refuse_scissor = false);
    assert_eq!((rig.b().pds.bump, rig.b().gen.bump, rig.b().ndraws, rig.b().vdm_dw, rig.b().term_gpu), before);
    assert!(rig.kicks().is_empty(), "a refusal submits nothing");
    assert_eq!(draw(&more), 0);
    assert_eq!(rig.b().ndraws, 2);
}

#[test]
fn a_full_depth_bias_table_splits_the_frame() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.polygon_offset = true;

    for i in 0..DBIAS_MAX_ENTRIES + 6 {
        d.polygon_offset_units = i as f32;
        assert_eq!(draw(&d), 0);
        d.do_clear = false;
    }
    assert_eq!(rig.kicks().len(), 1, "the 65th distinct bias opened a new frame");
    assert_eq!(rig.b().ndbias, 6);
    let idx: Vec<u32> = rig.ppp().iter().filter(|s| s.dbias_enable != 0).map(|s| s.dbias_index).collect();
    let want: Vec<u32> = (0..DBIAS_MAX_ENTRIES).chain(0..6).collect();
    assert_eq!(idx, want, "every draw names its own entry");
    /* the same state again shares the entry */
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().ndbias, 6);
}

#[test]
fn a_full_scissor_table_splits_the_frame() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.scissor = true;

    for i in 0..SCISSOR_MAX_ENTRIES + 2 {
        d.scissor_rect = [(i % 60) as i32, (i / 60) as i32, 4, 4];
        assert_eq!(draw(&d), 0);
        d.do_clear = false;
    }
    assert_eq!(rig.kicks().len(), 1);
    assert_eq!(rig.b().nscissor, 2);
    let idx: Vec<u32> = rig.ppp().iter().filter(|s| s.scissor_enable != 0).map(|s| s.scissor_index).collect();
    assert_eq!(idx.len(), SCISSOR_MAX_ENTRIES as usize + 2);
    assert_eq!(idx[SCISSOR_MAX_ENTRIES as usize - 1], SCISSOR_MAX_ENTRIES - 1);
    assert_eq!(idx[SCISSOR_MAX_ENTRIES as usize..], [0, 1]);
}

#[test]
fn the_draw_table_limit_splits_the_frame() {
    let rig = Rig::new();
    let mut d = rig.draw(3);

    for _ in 0..MAX_DRAWS_PER_FRAME + 1 {
        assert_eq!(draw(&d), 0);
        d.do_clear = false;
    }
    assert_eq!(rig.kicks().len(), 1);
    assert_eq!(rig.b().ndraws, 2, "load quad + the draw that did not fit");
}

#[test]
fn malformed_tables_are_rejected_without_indexing_them() {
    let rig = Rig::new();
    let d = rig.draw(3);

    assert_eq!(unsafe { pvr_backend_draw(core::ptr::null()) }, -EINVAL);
    for n in [17u8, 255] {
        let mut m = d;
        m.num_attribs = n;
        assert_eq!(draw(&m), -EINVAL, "{n} attributes");
    }
    let mut m = d;
    m.attribs[0].components = 0;
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 1);
    assert_eq!(draw(&m), -EINVAL);
    m.attribs[0].components = 5;
    assert_eq!(draw(&m), -EINVAL);
    m.attribs[0].components = 4;
    m.attribs[0].format = 1;
    assert_eq!(draw(&m), -EINVAL);
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 0);

    /* staged indices shorter than the count (and a wrapped count * 2) */
    let idx = [0u16; 4];
    let mut m = d;
    m.index_cpu = idx.as_ptr() as *const c_void;
    m.index_bytes = 5;
    m.index_count = 3;
    assert_eq!(draw(&m), -EINVAL);
    m.index_count = 0x8000_0002;
    m.index_bytes = m.index_count.wrapping_mul(2);
    assert_eq!(draw(&m), -EINVAL);

    let mut m = d;
    m.fs = core::ptr::null();
    assert_eq!(draw(&m), -EINVAL);
    let mut m = d;
    m.target_w = 0;
    assert_eq!(draw(&m), -EINVAL);
    let mut ffs = *rig.ffs;
    ffs.num_varyings = 17;
    let mut m = d;
    m.fs = &ffs;
    assert_eq!(draw(&m), -EINVAL);
    assert!(rig.index_lists().is_empty());
}

#[test]
fn the_attribute_dump_survives_a_zero_component_count() {
    let _rig = Rig::new();
    let v = [1.0f32; 4];
    let a = GlesAttrib { components: 0, cpu: v.as_ptr() as *const c_void, cpu_bytes: 16, ..GlesAttrib::ZERO };
    dump_attrib(&a);
}

#[test]
fn huge_targets_do_not_overflow_the_kick_arithmetic() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.target_w = u32::MAX;
    d.target_h = u32::MAX;
    d.ds_persist = true;
    d.ds_id = 9;
    d.do_clear_depth = true;
    d.clear_depth = 1.0;

    /* no depth buffer can be allocated for it: a depth clear is refused
     * (ds_refuse_lost), and the colour-only draw goes ahead without one */
    assert_eq!(draw(&d), -ENOMEM);
    d.do_clear_depth = false;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().frame_ds, DsStore::NoMemory);
    assert_eq!(pvr_backend_flush(), 0);
    let (k, _) = rig.kicks()[0];
    assert_eq!(k.frag.pbe_word[0][1], (u32::MAX - 1) as u64 | (((u32::MAX - 1) as u64) << 32));
    assert_eq!(ds_bytes(u32::MAX, u32::MAX, 4), None);
    /* whole 32-pixel ISP store tiles (upstream 3696af1), per sample */
    assert_eq!(ds_bytes(17, 1, 4), Some(64 * 64 * 4));
    assert_eq!(ds_bytes(16, 16, 1), Some(32 * 32 * 4));
    assert_eq!(ds_bytes(512, 300, 1), Some(512 * 320 * 4), "a 300-row target stores 320 rows");
}

#[test]
fn arena_sizes_are_checked_without_wrapping() {
    let _rig = Rig::new();
    let mut backing = vec![0u64; 16];
    let mut a = Arena {
        mem: OsalGpuMem {
            cpu: backing.as_mut_ptr() as *mut c_void,
            gpu: HEAP_GENERAL_BASE + 0x1000,
            phys: 0,
            size: 128,
            impl_: core::ptr::null_mut(),
        },
        bump: 5,
    };
    let src = [0u8; 8];
    let mut gpu = 0;
    // SAFETY: the sizes are rejected before `src` is read.
    unsafe {
        assert_eq!(arena_put(&mut a, src.as_ptr(), u32::MAX, 16, None, None), -EFRAME_FULL);
        assert_eq!(arena_put(&mut a, src.as_ptr(), u32::MAX - 20, 16, None, None), -EFRAME_FULL);
        a.bump = u32::MAX - 3;
        assert_eq!(arena_put(&mut a, src.as_ptr(), 1, 16, None, None), -EFRAME_FULL, "the alignment wraps");
        a.bump = 5;
        assert_eq!(arena_put(&mut a, src.as_ptr(), 8, 16, None, Some(&mut gpu)), 0);
    }
    assert_eq!((a.bump, gpu), (24, HEAP_GENERAL_BASE + 0x1010));
    // SAFETY: 104 bytes are left but 112 are asked for.
    unsafe { assert_eq!(arena_put(&mut a, [0u8; 112].as_ptr(), 112, 1, None, None), -EFRAME_FULL) };
    assert_eq!(a.bump, 24, "a rejected copy leaves the bump alone");
}

#[test]
fn an_unsupported_sample_count_does_not_split_every_draw() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.samples = 3;

    for _ in 0..4 {
        assert_eq!(draw(&d), 0);
        d.do_clear = false;
    }
    assert!(rig.kicks().is_empty());
    assert_eq!(rig.b().samples, 1);
    d.samples = 4;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.kicks().len(), 1, "a real change of the sample count still splits");
}

/// The sample rates of the draws' fragment programs (the fixed programs the
/// backend builds at init, the clears, are left out: they are not at the
/// draw's USC address).
fn draw_rates(rig: &Rig) -> Vec<u32> {
    let off = heap_off_of(rig.draw(3).fs_usc_gpu) as u64;
    with_platform(|p| p.kick_usc_rates.iter().filter(|r| r.0 == off).map(|r| r.1).collect())
}

#[test]
fn a_multisampled_draw_that_reads_the_pixel_runs_per_sample() {
    let rig = Rig::new();
    with_platform(|p| p.kick_usc_rates.clear());
    let mut d = rig.draw(3);

    d.samples = 4;
    assert_eq!(draw(&d), 0);
    d.do_clear = false;
    d.blend = true;
    assert_eq!(draw(&d), 0);
    d.samples = 1;
    assert_eq!(draw(&d), 0);
    assert_eq!(
        draw_rates(&rig),
        [PDS_DOUTU_SAMPLE_RATE_INSTANCE, PDS_DOUTU_SAMPLE_RATE_FULL, PDS_DOUTU_SAMPLE_RATE_INSTANCE],
        "4x plain, 4x blended, 1x blended"
    );
}

#[test]
fn a_multisampled_draw_with_the_kill_epilogue_runs_per_sample() {
    let rig = Rig::new();
    let mut fs = *rig.ffs;
    fs.uses_discard = 1;
    let mut d = rig.draw(3);
    d.fs = &fs;
    d.samples = 4;
    assert_eq!(draw(&d), 0);
    assert_eq!(draw_rates(&rig), [PDS_DOUTU_SAMPLE_RATE_FULL]);
}

#[test]
fn the_control_word_carries_the_draws_sample_mask_and_alpha_to_coverage() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.sample_mask = 0x5;
    d.alpha_to_coverage = true;
    assert_eq!(fs_meta_word(1, &d), SHC_FS_META_DEFAULT, "one sample: GL ignores both");
    assert_eq!(fs_meta_word(4, &d), 0x0000_0bfe | 1 << 25, "colour writes, mask 0b0101, a2c");
    d.sample_mask = 0xf;
    d.alpha_to_coverage = false;
    assert_eq!(fs_meta_word(4, &d), 0x0000_1ffe, "every sample of four");
}

#[test]
fn a_sequential_index_allocation_failure_is_out_of_memory() {
    let rig = Rig::new();
    let d = rig.draw(2000); /* more than the first 1024-index buffer */

    with_platform(|p| p.fail_alloc_in = 1);
    assert_eq!(draw(&d), -ENOMEM);
    assert!(rig.kicks().is_empty(), "not a frame-full retry");
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().seq_idx_count, 2000);
}

#[test]
fn a_failed_submit_drops_the_frame_and_the_next_draw_starts_a_new_one() {
    let rig = Rig::new();
    let mut d = rig.draw(3);

    assert_eq!(draw(&d), 0);
    with_platform(|p| p.fail_submit = true);
    d.target_gpu += 0x1000_0000; /* a target change submits the open frame */
    assert_eq!(draw(&d), -EIO);
    assert!(!rig.b().frame_open);
    with_platform(|p| p.fail_submit = false);
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks().len(), 1);
}

/// The kick's clear-depth word is the unfused `dz * 16777215 + 0.5` (the
/// Zephyr C build: gcc -std=c17 does not contract it; see the top of
/// backend.rs), not the fused-multiply-add result a gnu11 build gives.
#[test]
fn the_clear_depth_word_is_the_unfused_product_plus_a_half() {
    let rig = Rig::new();
    let unfused = |dz: f32| (dz * 16777215.0 + 0.5) as u32;
    let fused = |dz: f32| dz.mul_add(16777215.0, 0.5) as u32;
    let dz =
        (1..10_000).map(|i| i as f32 / 10_000.0).find(|&z| unfused(z) != fused(z)).expect("a depth where they differ");
    let mut d = rig.draw(3);
    d.ds_persist = true;
    d.ds_id = 1;
    d.do_clear_depth = true;
    d.clear_depth = dz;

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let (k, _) = rig.kicks()[0];
    assert_eq!(k.frag.isp_bgobjdepth, unfused(dz));
    assert_ne!(k.frag.isp_bgobjdepth, fused(dz));
    /* ISP_CTL.dbias_is_int reaches a kick with a depth/stencil buffer by
     * default (pvr_dbias_int, on since the AM62P A/B, RESULTS 1i: dEQP
     * polygon_offset 14/0 with it, 10/4 without; gl_test case 3 passes
     * either way). */
    assert_eq!(k.frag.isp_ctl & ISP_CTL_DBIAS_IS_INT, ISP_CTL_DBIAS_IS_INT);
    assert_eq!(k.frag.cmd_flags, FRAG_FLAGS_DEPTHBUFFER | FRAG_FLAGS_STENCILBUFFER);
}

#[test]
fn the_depth_bias_constant_is_integer_units_only_with_a_depth_buffer() {
    let rig = Rig::new();
    /* no depth/stencil: nothing to bias, and the bit must stay clear */
    let mut d = rig.draw(3);
    d.polygon_offset = true;
    d.polygon_offset_units = 4.0;
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks()[0].0.frag.isp_ctl & ISP_CTL_DBIAS_IS_INT, 0, "no D/S buffer, no dbias_is_int");
}

/// dEQP fbo.render.shared_depthbuffer's shape: FBO A (depth 2, stencil 3)
/// draws, then FBO B (the same depth 2, its own stencil 4). With
/// `pvr_split_zs` the two kicks store and load one depth buffer and each
/// its own stencil, through CR_ISP_STENCIL_LOAD_BASE with the
/// separate-stencil enable, and B loads the depth A stored. Off, each pair
/// has a packed buffer of its own: B's depth is not A's (the AM62P's 14
/// fbo.render failures, a60104c2).
#[test]
fn split_depth_and_stencil_share_the_depth_between_fbos() {
    for split in [false, true] {
        let rig = Rig::new();
        pvr_split_zs(split);
        /* the GL core's identities: depth object serial 12, stencil objects
         * 13 and 14, each at storage stamp 1; ds_id names them by slot */
        for stencil in [3u32, 4] {
            let mut d = rig.draw(3);
            d.ds_persist = true;
            d.ds_id = (2 << 16) | stencil;
            (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen) = (12, 1, 10 + stencil, 1);
            d.depth_test = true;
            assert_eq!(draw(&d), 0);
            assert_eq!(pvr_backend_flush(), 0);
        }
        /* the depth object specified again (a new stamp, the same names):
         * its old contents are gone, so it is another buffer */
        let mut d = rig.draw(3);
        d.ds_persist = true;
        d.ds_id = (2 << 16) | 3;
        (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen) = (12, 2, 13, 1);
        d.depth_test = true;
        assert_eq!(draw(&d), 0);
        assert_eq!(pvr_backend_flush(), 0);
        let k = rig.kicks();
        assert_eq!(k.len(), 3);
        if split {
            let (a, c) = (&k[0].0.frag, &k[2].0.frag);
            assert_ne!(a.isp_zload_store_base, c.isp_zload_store_base, "a respecified depth is a new buffer");
            assert_eq!(a.isp_stencil_load_store_base, c.isp_stencil_load_store_base, "the same stencil object");
        }
        let (a, b) = (&k[0].0.frag, &k[1].0.frag);
        if split {
            assert_eq!(a.isp_zload_store_base, b.isp_zload_store_base, "one depth buffer for both FBOs");
            assert_ne!(a.isp_stencil_load_store_base, b.isp_stencil_load_store_base, "a stencil each");
            for f in [a, b] {
                assert_eq!(f.isp_stencil_load_store_base & ISP_STENCIL_BASE_SEPARATE, 1, "the separate-stencil enable");
                assert_ne!(f.isp_stencil_load_store_base & !1, f.isp_zload_store_base, "apart from the depth");
            }
            assert_ne!(b.isp_zlsctl & ZLSCTL_ZLOADEN, 0, "B loads the depth A stored");
            assert_eq!(b.isp_zlsctl & ZLSCTL_SLOADEN, 0, "B's own stencil holds no store yet");
        } else {
            assert_ne!(a.isp_zload_store_base, b.isp_zload_store_base, "the FB2 symptom: a depth per pair");
            assert_eq!(a.isp_stencil_load_store_base, a.isp_zload_store_base, "packed D24S8");
        }
        pvr_split_zs(false);
        drop(rig);
    }
}

#[test]
fn the_dbias_int_option_selects_the_integer_encoding() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.ds_persist = true;
    d.ds_id = 1;
    d.polygon_offset = true;
    d.polygon_offset_units = 4.0;

    /* The rig runs with the product's default, which is on; the option (the
     * C clients' way, YAKOGL_PVR_DBIAS_INT and the Zephyr switches) turns it
     * off and on again, and each kick says which. */
    assert_eq!(PVR_DBIAS_INT_DEFAULT, 1, "the default the AM62P A/B chose (RESULTS 1i)");
    let mut bits = Vec::new();
    for value in [None, Some(0), Some(1)] {
        if let Some(v) = value {
            pvr_backend_set_option(PVR_BACKEND_OPT_DBIAS_INT, v);
        }
        assert_eq!(draw(&d), 0);
        assert_eq!(pvr_backend_flush(), 0);
        bits.push(rig.kicks().last().unwrap().0.frag.isp_ctl & ISP_CTL_DBIAS_IS_INT);
    }
    assert_eq!(bits, [ISP_CTL_DBIAS_IS_INT, 0, ISP_CTL_DBIAS_IS_INT], "default, set to 0, set to 1");
    pvr_set_dbias_int(PVR_DBIAS_INT_DEFAULT);
}

#[test]
fn a_piece_that_opens_a_new_frame_loads_the_colour_instead_of_clearing_it() {
    let rig = Rig::new();
    /* sized for the 4 MiB arena (the default is 1 MiB since the frame-set
     * measurement) */
    restart_with_arenas(4, 4096, 1024);
    let mut d = rig.draw(66_000);
    /* 64 bytes per vertex of staged data: a piece is 32766 vertices (2.1 MB
     * of the 4 MB arena), so the second piece does not fit the frame */
    let verts = vec![0u8; 66_000 * 64];
    d.attribs[0] = GlesAttrib {
        components: 4,
        stride: 64,
        cpu: verts.as_ptr() as *const c_void,
        cpu_bytes: verts.len() as u32,
        ..GlesAttrib::ZERO
    };
    assert!(d.do_clear, "the draw clears the target");

    assert_eq!(draw(&d), 0);
    assert_eq!(rig.kicks().len(), 1, "the frame was split inside the draw");
    assert!(!rig.b().frame_cleared, "the continuation frame keeps the colour");
    assert_eq!(rig.b().frame_quads, 1, "it starts with the colour-load quad");
    let counts: Vec<u32> = rig.index_lists().iter().map(|l| l.2).collect();
    /* two pieces fill the arena, then the load quad (6) opens the next
     * frame; no piece is drawn twice */
    assert_eq!(counts, [32766, 32766, 6, 468]);
}

/* ---- gl_PointCoord (case 83) ------------------------------------------------- */

#[test]
fn a_fragment_shader_reading_gl_pointcoord_gets_a_point_sprite_iterator() {
    /* pco reads gl_PointCoord from the coefficients after W and places the
     * varyings after it; the driver has to iterate them with a point-sprite
     * DOUTI. Without it case 83 read s = t = 1.0 across the sprite on the
     * AM62P (2026-09-23): the record's uses_pntc must reach the generator. */
    let rig = Rig::new();
    for pntc in [0u8, 1] {
        rig.reset_records();
        let mut ffs = *rig.ffs;
        ffs.uses_pntc = pntc;
        let mut d = rig.draw(3);
        d.fs = &ffs;
        d.prim = GLES_PRIM_POINTS;
        assert_eq!(draw(&d), 0, "uses_pntc={pntc}");
        assert_eq!(with_gen(|g| g.coeff_pntc.clone()), vec![pntc as i32], "uses_pntc={pntc}");
    }
}

/// pvr_sprite_yup: a point sprite on a target stored y-up (flip_y 0) is
/// drawn as the chosen sprite object type, so gl_PointCoord.t starts at the
/// window's top there too (dEQP builtin_variable.pointcoord into a y-up
/// pbuffer). Off, and on a target stored top-down, SPRITE_01UV as always.
#[test]
fn a_point_on_a_y_up_target_is_drawn_as_the_chosen_sprite_kind() {
    let rig = Rig::new();
    /* the default: SPRITE_10UV on a y-up target (the AM62P: pointcoord
     * passes with it, 27ebd89a) */
    {
        let mut d = rig.draw(3);
        d.prim = GLES_PRIM_POINTS;
        d.flip_y = false;
        assert_eq!(draw(&d), 0);
        assert!(with_gen(|g| g.ppp.iter().all(|st| st.objtype == 2)), "the default is SPRITE_10UV");
    }
    for (yup, flip_y, want) in [
        (0u32, false, PVR_TA_OBJTYPE_SPRITE_01UV),
        (3, false, 3),
        (2, false, 2),
        (3, true, PVR_TA_OBJTYPE_SPRITE_01UV),
        (9, false, PVR_TA_OBJTYPE_SPRITE_01UV), /* not a sprite kind: off */
    ] {
        pvr_set_sprite_yup(yup);
        rig.reset_records();
        let mut d = rig.draw(3);
        d.prim = GLES_PRIM_POINTS;
        d.flip_y = flip_y;
        assert_eq!(draw(&d), 0);
        let ppp = with_gen(|g| g.ppp.clone());
        assert!(!ppp.is_empty());
        assert!(
            ppp.iter().all(|st| st.objtype == want),
            "yup {yup} flip_y {flip_y}: {:?}",
            ppp.iter().map(|s| s.objtype).collect::<Vec<_>>()
        );
    }
    pvr_set_sprite_yup(PVR_SPRITE_YUP_DEFAULT);
}

/* ---- a fragment shader that reads the pixel ---------------------------------- */

#[test]
fn a_depth_writing_draw_that_reads_the_pixel_is_translucent_not_opaque() {
    /* OPAQUE lets the ISP remove hidden surfaces: only the front-most
     * fragment of a pixel is shaded. A shader that reads the pixel -- a
     * colour-masked draw or clear keeps channels of what is under it -- then
     * reads a pixel the object behind never reached. Mesa's pvr runs
     * `uses.fbfetch` TRANSLUCENT (pvr_arch_pipeline.c); so does this, and a
     * depth-writing draw that does not read the pixel stays OPAQUE. */
    let rig = Rig::new();
    /* switch off: the pass type as it always was (fbfetch still OPAQUE) */
    {
        let mut ffs = *rig.ffs;
        ffs.uses_fbfetch = 1;
        let mut d = rig.draw(3);
        d.fs = &ffs;
        d.do_clear = false;
        d.depth_test = true;
        d.depth_write = true;
        assert_eq!(draw(&d), 0);
        assert_eq!(
            with_gen(|g| g.passtypes.last().copied()),
            Some(PVR_PPP_PASSTYPE_OPAQUE),
            "pvr_debug_masked_clear off: unchanged"
        );
    }
    pvr_debug_set_masked_clear(1);
    for fbfetch in [0u8, 1] {
        rig.reset_records();
        let mut ffs = *rig.ffs;
        ffs.uses_fbfetch = fbfetch;
        let mut d = rig.draw(3);
        d.fs = &ffs;
        d.do_clear = false;
        d.depth_test = true;
        d.depth_write = true;
        assert_eq!(draw(&d), 0, "uses_fbfetch={fbfetch}");
        let want = if fbfetch != 0 { PVR_PPP_PASSTYPE_TRANSLUCENT } else { PVR_PPP_PASSTYPE_OPAQUE };
        assert_eq!(with_gen(|g| g.passtypes.last().copied()), Some(want), "uses_fbfetch={fbfetch}");
    }
}

/* ---- glClear with glColorMask partly on --------------------------------------- */

#[test]
fn a_partially_masked_clear_quad_draws_the_masked_program_for_its_mask() {
    /* The PowerVR has no per-channel colour write mask: the clear quad is
     * drawn with the masked clear program for the mask (it reads the pixel
     * back for the channels left off), its four uniform dwords the clear
     * colour as floats, and TRANSLUCENT so the pixel it reads is there. A
     * full mask keeps the plain clear program; a one-channel float target
     * with red off clears no colour. */
    let rig = Rig::new();
    let mut first = rig.draw(3);
    first.do_clear = false;
    assert_eq!(draw(&first), 0);
    let mut cq = rig.draw(0);
    cq.clear_quad = true;
    cq.do_clear = true;
    cq.clear_rgba = [1.0, 0.5, 0.25, 1.0];
    cq.clear_color_mask = 0b0101;
    /* switch off: the plain clear, every channel */
    cq.color_mask = 0xf;
    assert_eq!(draw(&cq), 0);
    assert_eq!(rig.fbload().load_draw.fs, &rig.fbload().clear_fs_sh as *const ShcShader, "switch off: the plain clear");
    pvr_debug_set_masked_clear(1);
    /* the draw's own mask is not the clear's (gc-backend's contract) */
    cq.color_mask = 0xf;
    assert_eq!(draw(&cq), 0);
    assert_eq!(
        with_gen(|g| g.passtypes.last().copied()),
        Some(PVR_PPP_PASSTYPE_TRANSLUCENT),
        "the masked clear reads the pixel"
    );
    let fl = rig.fbload();
    let i = clear_masked_index(0, 0b0101).expect("a partial mask");
    assert_eq!(fl.load_draw.fs, &fl.clear_mask_sh[i] as *const ShcShader, "the program for R and B");
    assert_eq!(fl.load_draw.fs_usc_gpu, rig.b().clear_mask_fs[i].gpu);
    assert_eq!(fl.load_draw.fs_uniform_dwords, 4);
    // SAFETY: the uniform pointer is the backend's own four floats.
    let u = unsafe { core::slice::from_raw_parts(fl.load_draw.fs_uniform_cpu as *const f32, 4) };
    assert_eq!(u, [1.0, 0.5, 0.25, 1.0]);
    assert_eq!(fl.load_draw.color_mask, 0b0101);

    cq.clear_color_mask = 0xf;
    assert_eq!(draw(&cq), 0);
    assert_eq!(
        rig.fbload().load_draw.fs,
        &rig.fbload().clear_fs_sh as *const ShcShader,
        "every channel: the plain clear"
    );

    let before = rig.index_lists().len();
    let mut r16 = cq;
    r16.target_format = GLES_SURF_R16F;
    r16.clear_color_mask = 0b1110;
    assert_eq!(draw(&r16), 0);
    assert_eq!(rig.index_lists().len(), before, "R16F with red off: nothing to clear, no quad");
}

#[test]
fn a_partially_masked_clear_fused_into_a_draw_becomes_the_masked_quad_before_it() {
    /* gc-backend's contract: a partial mask can arrive on the background
     * clear (`do_clear` without `clear_quad`), fused into the draw that
     * follows it. The background object writes every channel, so the
     * PowerVR records the masked clear quad first and the draw after it, and
     * the frame is not background-cleared (the pixels the mask leaves alone
     * are kept). */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.do_clear = true;
    d.clear_rgba = [1.0, 0.0, 0.0, 1.0];
    d.clear_color_mask = 0b0011;
    /* switch off: the background clear, every channel, as before */
    assert_eq!(draw(&d), 0);
    assert!(rig.b().frame_cleared, "switch off: the frame's background clear");
    drop(rig);
    /* a fresh rig, and the draw rebuilt from it (the old one's shaders went
     * with it) */
    let rig = Rig::new();
    pvr_debug_set_masked_clear(1);
    let mut d = rig.draw(3);
    d.do_clear = true;
    d.clear_rgba = [1.0, 0.0, 0.0, 1.0];
    d.clear_color_mask = 0b0011;
    assert_eq!(draw(&d), 0);
    assert!(!rig.b().frame_cleared, "not the frame's background clear");
    let fl = rig.fbload();
    let i = clear_masked_index(0, 0b0011).expect("a partial mask");
    assert_eq!(fl.load_draw.fs, &fl.clear_mask_sh[i] as *const ShcShader, "the masked quad for R and G");
    /* the colour-load quad (6), the masked clear quad (6), then the draw (3) */
    let counts: Vec<u32> = rig.index_lists().iter().map(|l| l.2).collect();
    assert_eq!(counts, [6, 6, 3], "the clear lands before the draw it was fused into");
    assert_eq!(rig.kicks().len(), 0, "and forces no flush");
}

/* ---- gl_FragCoord.z (the record's uses_z) ------------------------------------- */

#[test]
fn a_fragment_shader_reading_the_z_plane_gets_a_z_iterator_and_the_te_hands_z_over() {
    /* pco reads gl_FragCoord.z from coefficient set 0, ahead of W; the driver
     * iterates it only when the record says so, and the TE only passes Z to
     * the TSP when TA_OUTPUT_SEL.tsp_unclamped_z_pres is set (Mesa
     * pvr_arch_pipeline.c:1503, pvr_arch_cmd_buffer.c:6587). Both have to
     * follow the flag, or the shader reads a set nobody wrote. */
    let rig = Rig::new();
    for z in [0u8, 1] {
        rig.reset_records();
        let mut ffs = *rig.ffs;
        ffs.uses_z = z;
        let mut d = rig.draw(3);
        d.fs = &ffs;
        assert_eq!(draw(&d), 0, "uses_z={z}");
        assert_eq!(with_gen(|g| g.coeff_z.clone()), vec![z as i32], "uses_z={z}");
        let ppp = with_gen(|g| g.ppp.clone());
        assert_eq!(ppp.len(), 1, "uses_z={z}: one PPP block per draw");
        assert_eq!(ppp[0].z_pres, z as i32, "uses_z={z}");
    }
}

/* ---- gl_FrontFacing (upstream 3b827f2) --------------------------------------- */

#[test]
fn a_fragment_shader_reading_gl_frontfacing_gets_the_front_face_op() {
    /* The USC combines the ISP's facing bit with an op in a shared register:
     * 0 keep, 1 swap, 2 always front. The ISP calls its clockwise triangles
     * front, and without pretransform the TA's winding is swapped for a
     * non-flipped target, so GL's default (counter-clockwise front) is
     * clockwise there and the op is "keep". */
    let rig = Rig::new();
    let mut ffs = *rig.ffs;
    ffs.fs_front_face_off1 = 4; /* the op goes to sh3 */
    let base = rig.draw(3);

    for (prim, front_cw, flip_y, want) in [
        (GLES_PRIM_TRIANGLES, false, false, 0u32),
        (GLES_PRIM_TRIANGLES, true, false, 1),
        (GLES_PRIM_TRIANGLES, false, true, 1),
        (GLES_PRIM_TRIANGLES, true, true, 0),
        (GLES_PRIM_POINTS, false, false, 2),
        (GLES_PRIM_LINES, true, false, 2),
    ] {
        rig.reset_records();
        let mut d = base;
        d.fs = &ffs;
        d.prim = prim;
        d.front_cw = front_cw;
        d.flip_y = flip_y;
        assert_eq!(draw(&d), 0, "prim {prim} front_cw={front_cw} flip_y={flip_y}");
        let kicks = rig.dma_kicks();
        let k = kicks.last().expect("a common-store DMA program");
        assert_eq!(k.len(), 1, "one kick: the front-face op ({k:?})");
        assert_eq!((k[0].1, k[0].2), (1, 3), "one dword into sh3");
        assert_eq!(rig.gpu_u32(k[0].0), want, "prim {prim} front_cw={front_cw} flip_y={flip_y}");
    }
}

#[test]
fn a_fragment_shader_that_ignores_gl_frontfacing_uploads_no_front_face_op() {
    let rig = Rig::new();
    let d = rig.draw(3);

    assert_eq!(d.fs_uniform_dwords, 0);
    assert_eq!(draw(&d), 0);
    assert!(rig.dma_kicks().is_empty(), "no shared register to fill");
}

/* ---- gl_FragCoord's y pair, and the common-store DMA kick count ------------ */

#[test]
fn gl_fragcoords_pair_follows_the_targets_flip() {
    /* The online compiler's program computes y' = y * scale + offset from
     * the pair at fragcoord_ytf_off1 - 1: (-1, height) on a target stored
     * top-down, (1, 0) on any other. */
    let rig = Rig::new();
    let mut ffs = *rig.ffs;
    ffs.fragcoord_ytf_off1 = 3; /* the pair goes to sh2..3 */
    let base = rig.draw(3);

    for (flip_y, want) in [(false, (1.0f32, 0.0f32)), (true, (-1.0, 48.0))] {
        rig.reset_records();
        let mut d = base;
        d.fs = &ffs;
        d.flip_y = flip_y;
        d.target_h = 48;
        assert_eq!(draw(&d), 0, "flip_y={flip_y}");
        let kicks = rig.dma_kicks();
        let k = kicks.last().expect("a common-store DMA program");
        assert_eq!(k.len(), 1, "one kick: the pair ({k:?})");
        assert_eq!((k[0].1, k[0].2), (2, 2), "two dwords into sh2");
        let got = (f32::from_bits(rig.gpu_u32(k[0].0)), f32::from_bits(rig.gpu_u32(k[0].0 + 4)));
        assert_eq!(got, want, "flip_y={flip_y}");
    }
    /* none when the record names no pair (every table program) */
    rig.reset_records();
    assert_eq!(draw(&base), 0);
    assert!(rig.dma_kicks().is_empty(), "no pair to write");
}

#[test]
fn a_draw_that_needs_six_common_store_kicks_gets_all_six() {
    /* Descriptors, uniforms, the control word, the front-face op, the y
     * pair and the tile-buffer addresses: six kicks. With four the last two
     * were dropped without a word, and the shader read registers nobody
     * wrote. */
    let rig = Rig::new();
    let mut ffs = *rig.ffs;
    ffs.uniform_dwords = 4;
    ffs.fragcoord_ytf_off1 = 25; /* sh24..25 */
    ffs.meta_off = 26;
    ffs.fs_front_face_off1 = 28; /* sh27 */
    ffs.num_outputs = 3;
    ffs.num_tile_buffers = 1;
    ffs.tile_buffer_sh = 40;
    let block = [0u32; 4];
    let mut d = rig.draw(3);
    d.fs = &ffs;
    d.fs_uniform_dwords = 4;
    d.fs_uniform_cpu = block.as_ptr() as *const c_void;
    d.textures[0] = GlesTextureBind {
        unit: 0,
        base_gpu: HEAP_GENERAL_BASE + 0x100_0000,
        width: 4,
        height: 4,
        stride_texels: 4,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    };
    d.num_textures = 1;
    d.target_format = GLES_SURF_RGBA16F;
    d.num_targets = 3;
    for i in 0..3u64 {
        d.targets[i as usize] = GlesDrawTarget {
            gpu: HEAP_GENERAL_BASE + 0x2000_0000 + i * 0x10_0000,
            w: 64,
            h: 64,
            format: GLES_SURF_RGBA16F,
        };
    }
    d.target_gpu = d.targets[0].gpu;

    assert_eq!(draw(&d), 0);
    let kicks = rig.dma_kicks();
    let k = kicks.last().expect("a common-store DMA program");
    let dests: Vec<(u32, u32)> = k.iter().map(|k| (k.2, k.1)).collect();
    assert_eq!(dests, [(0, 20), (20, 4), (26, 1), (27, 1), (24, 2), (40, 2)], "every kick, in order");
    assert!(!rig.logs().iter().any(|l| l.contains("common-store DMA kicks")), "{:?}", rig.logs());
}

#[test]
fn a_kick_past_the_dma_program_is_refused_not_dropped() {
    /* No draw reaches eight kicks today, so the refusal is checked where it
     * is decided: the ninth push says no, and the draw that got that answer
     * fails with -E2BIG and one log line, however often it happens. */
    let rig = Rig::new();
    let mut kicks = [PvrPdsDmaKick::ZERO; PVR_PDS_DMA_MAX_KICKS as usize];
    let mut n = 0usize;
    for i in 0..PVR_PDS_DMA_MAX_KICKS {
        assert!(dma_push(&mut kicks, &mut n, PvrPdsDmaKick { address: 0, dwords: 1, dest_offset: i }));
    }
    assert!(!dma_push(&mut kicks, &mut n, PvrPdsDmaKick { address: 0, dwords: 1, dest_offset: 99 }));
    assert_eq!(n, PVR_PDS_DMA_MAX_KICKS as usize);
    assert_eq!(dma_kicks_full(rig.b()), -E2BIG);
    assert_eq!(dma_kicks_full(rig.b()), -E2BIG);
    let said = rig.logs().iter().filter(|l| l.contains("common-store DMA kicks; refused")).count();
    assert_eq!(said, 1, "{:?}", rig.logs());
}

/* ---- diagnostics (upstream 43efb73, 0fa1a7c, a3b7374) ------------------------ */

#[test]
fn the_workload_counters_add_up_the_draws_and_tiles_of_every_kick() {
    let rig = Rig::new();
    let d = rig.draw(3);
    let (mut draws, mut tiles, mut kicks, mut waits) = (0u32, 0u32, 0u32, 0u32);

    // SAFETY: the pointers are writable.
    unsafe {
        pvr_backend_workload(&mut draws, &mut tiles);
        pvr_backend_counters(&mut kicks, &mut waits);
    }
    let (draws0, tiles0, kicks0) = (draws, tiles, kicks);
    assert_eq!(draw(&d), 0);
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    // SAFETY: as above.
    unsafe {
        pvr_backend_workload(&mut draws, &mut tiles);
        pvr_backend_counters(&mut kicks, &mut waits);
    }
    /* the two draws, and the 64x64 target's 4 x 4 tiles of 16x16 pixels
     * once per kick (a draw that clears closes the open frame) */
    assert_eq!(draws - draws0, 2);
    assert_eq!(tiles - tiles0, 16 * (kicks - kicks0));
    assert!(kicks > kicks0);
    // SAFETY: null is ignored.
    unsafe { pvr_backend_workload(core::ptr::null_mut(), core::ptr::null_mut()) };
}

#[test]
fn the_per_draw_debug_dump_has_a_budget_of_draws_from_when_it_is_switched_on() {
    /* upstream a3b7374: ~30 console lines a draw would flood a running app */
    let rig = Rig::new();
    let d = rig.draw(3);
    /* logs() drains the recorder: every call counts the lines since the last */
    let dumps = |r: &Rig| r.logs().iter().filter(|l| l.starts_with("pvr_backend: draw ")).count();

    assert_eq!(draw(&d), 0);
    assert_eq!(dumps(&rig), 0, "the dump is off");
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 1);
    assert_eq!(rig.b().dbg_draws_left, DBG_DRAW_BUDGET);
    for _ in 0..DBG_DRAW_BUDGET {
        assert_eq!(draw(&d), 0);
    }
    assert_eq!(dumps(&rig), DBG_DRAW_BUDGET as usize, "every draw of the budget is dumped");
    assert_eq!(rig.b().dbg_draws_left, 0);
    assert_eq!(draw(&d), 0);
    assert_eq!(dumps(&rig), 0, "the next draw is not dumped");
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 0);
}

#[test]
fn a_dump_switched_on_before_the_backend_starts_keeps_its_budget() {
    /* gl_test and the harness set the option before the first GL call; the
     * C zeroes the budget in pvr_backend_init() and prints nothing. */
    let rig = Rig::new();
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 1);
    pvr_backend_shutdown();
    assert_eq!(pvr_backend_init(), 0);
    assert_eq!(rig.b().dbg_draws_left, DBG_DRAW_BUDGET, "init keeps the budget the option handed out");
    let d = rig.draw(3);
    assert_eq!(draw(&d), 0);
    assert!(rig.logs().iter().any(|l| l.starts_with("pvr_backend: draw ")), "the draw is dumped");
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 0);
}

#[test]
fn the_per_draw_debug_line_names_each_shaders_usc_size() {
    /* upstream 0fa1a7c: usc_dw identifies the shader in the offline table */
    let rig = Rig::new();
    let mut fvs = *rig.fvs;
    let mut ffs = *rig.ffs;
    let (vs_usc, fs_usc) = ([0u32; 7], [0u32; 9]);

    fvs.usc = vs_usc.as_ptr();
    fvs.usc_dw = vs_usc.len() as u32;
    ffs.usc = fs_usc.as_ptr();
    ffs.usc_dw = fs_usc.len() as u32;
    let mut d = rig.draw(3);
    d.vs = &fvs;
    d.fs = &ffs;
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 1);
    assert_eq!(draw(&d), 0);
    pvr_backend_set_option(PVR_BACKEND_OPT_DEBUG, 0);
    let logs = rig.logs();
    let line = logs.iter().find(|l| l.starts_with("pvr_backend: draw ")).expect("the per-draw line");
    assert!(line.contains("vs usc off 0x100000 dw 7 temps 2"), "{line}");
    assert!(line.contains("fs usc off 0x200000 dw 9 temps 3"), "{line}");
}

/// PVR_BACKEND_OPT_FS_TEMPS_MIN raises the temporaries the fragment kick
/// declares to at least its value, capped at FS_TEMPS_MIN_CAP, and leaves a
/// larger count alone; 0 declares the shader's own count.
#[test]
fn the_fs_temps_floor_raises_only_the_kick_allocation() {
    let rig = Rig::new();
    let declared = |floor: u32| {
        pvr_backend_set_option(PVR_BACKEND_OPT_FS_TEMPS_MIN, floor);
        with_platform(|p| p.kick_usc_temps.clear());
        assert_eq!(draw(&rig.draw(3)), 0);
        pvr_backend_set_option(PVR_BACKEND_OPT_FS_TEMPS_MIN, 0);
        let off = heap_off_of(rig.draw(3).fs_usc_gpu) as u64;
        with_platform(|p| p.kick_usc_temps.iter().filter(|k| k.0 == off).map(|k| k.1).collect::<Vec<u32>>())
    };
    assert_eq!(declared(0), [3], "the shader's own 3 temps (the rig's fs)");
    assert_eq!(declared(2), [3], "a floor below the shader's count changes nothing");
    assert_eq!(declared(30), [30]);
    assert_eq!(declared(1000), [FS_TEMPS_MIN_CAP], "capped");
}

/// A shader that declares more temporaries than a DOUTU word can hold is
/// refused by the PDS generator; the draw is refused by name instead of
/// uploading the empty program the generator left behind.
#[test]
fn a_shader_past_the_doutu_temps_bound_is_refused_by_name() {
    let mut rig = Rig::new();
    rig.ffs.temps = PVR_PDS_DOUTU_MAX_TEMPS as u16;
    assert_eq!(draw(&rig.draw(3)), 0, "the bound itself fits: {:?}", rig.logs());

    rig.ffs.temps = PVR_PDS_DOUTU_MAX_TEMPS as u16 + 1;
    assert_eq!(draw(&rig.draw(3)), -EINVAL);
    let want = "yakogl/pvr: draw refused: the fragment kick program was refused (253 temps, max 252)";
    assert!(rig.logs().iter().any(|l| l.contains(want)), "{want} not in {:?}", rig.logs());

    rig.ffs.temps = 3;
    rig.fvs.temps = PVR_PDS_DOUTU_MAX_TEMPS as u16 + 1;
    assert_eq!(draw(&rig.draw(3)), -EINVAL);
    let want = "yakogl/pvr: draw refused: the vertex fetch program was refused (1 streams, 253 temps)";
    assert!(rig.logs().iter().any(|l| l.contains(want)), "{want} not in {:?}", rig.logs());
}

/* ---- the sync to upstream e6c449c ------------------------------------------ */

#[test]
fn a_frame_past_the_primitive_budget_is_split_into_kicks() {
    /* upstream 47a31c6: the parameter buffer the firmware fills while tiling
     * is finite, so the host splits the frame itself. */
    let rig = Rig::new();
    let before = overhead();
    pvr_backend_set_option(PVR_BACKEND_OPT_PRIM_BUDGET, 4);
    let d = rig.draw(9); /* three triangles */

    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().prims, 3);
    let mut d2 = rig.draw(9);
    d2.do_clear = false;
    assert_eq!(draw(&d2), 0, "the second draw does not fit the budget: the frame is flushed");
    assert_eq!(rig.kicks().len(), 1, "the first three triangles went out");
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks().len(), 2);
    assert_eq!(overhead().1 - before.1, 1, "one split");
    pvr_backend_set_option(PVR_BACKEND_OPT_PRIM_BUDGET, 0);
}

#[test]
fn an_instanced_draw_past_the_budget_is_split_by_instances() {
    let rig = Rig::new();
    pvr_backend_set_option(PVR_BACKEND_OPT_PRIM_BUDGET, 2);
    let mut d = rig.draw(3); /* one triangle per instance */
    d.instance_count = 4;
    d.attribs[1] = GlesAttrib {
        components: 4,
        stride: 32,
        offset: 64,
        buffer_gpu: HEAP_GENERAL_BASE + 0x1000_0000,
        per_instance: 1,
        ..GlesAttrib::ZERO
    };
    d.num_attribs = 2;

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    /* two chunks of two instances, one kick each */
    let il = rig.index_lists();
    /* the frame the second chunk opens starts with a colour-load quad
     * (instances 0), which is not part of the split */
    assert_eq!(il.iter().map(|l| l.3).filter(|&n| n != 0).collect::<Vec<_>>(), [2, 2], "instances per chunk");
    assert_eq!(rig.kicks().len(), 2);
    /* the per-instance stream of the second chunk starts at instance 2 */
    let streams = rig.streams();
    let per_inst: Vec<u64> =
        streams.iter().filter_map(|ss| ss.iter().find(|s| s.per_instance != 0).map(|s| s.address)).collect();
    assert_eq!(per_inst, [HEAP_GENERAL_BASE + 0x1000_0040, HEAP_GENERAL_BASE + 0x1000_0040 + 64]);
    pvr_backend_set_option(PVR_BACKEND_OPT_PRIM_BUDGET, 0);
}

#[test]
fn a_draw_that_reads_gl_instance_id_is_never_split_by_instances() {
    /* upstream 575288b: the vertex fetch numbers instances per index list, so
     * the ID would restart at 0 in every chunk. */
    let rig = Rig::new();
    let mut fvs = *rig.fvs;
    fvs.vs_instance_id_reg1 = 3;
    pvr_backend_set_option(PVR_BACKEND_OPT_PRIM_BUDGET, 2);
    let mut d = rig.draw(3);
    d.vs = &fvs;
    d.instance_count = 4;

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let il = rig.index_lists();
    assert_eq!(il.iter().map(|l| l.3).collect::<Vec<_>>(), [4], "one index list of four instances");
    pvr_backend_set_option(PVR_BACKEND_OPT_PRIM_BUDGET, 0);
}

/// One instance of an instanced draw the GL core draws instance by instance
/// (GlesDraw::instance_id, a divisor past 1): the PDS starts the instance
/// number at 0 for every draw, so a program that reads gl_InstanceID is
/// refused, not drawn with 0. Instance 0, and a program that does not read
/// it, are drawn. Revert check: without the refusal the first draw goes in.
#[test]
fn an_instance_drawn_on_its_own_that_reads_gl_instance_id_is_refused() {
    let rig = Rig::new();
    let mut fvs = *rig.fvs;
    fvs.vs_instance_id_reg1 = 3;
    let mut d = rig.draw(3);
    d.vs = &fvs;
    d.instance_id = 2;
    assert_eq!(draw(&d), -EINVAL, "reads gl_InstanceID, instance 2");
    d.instance_id = 0;
    assert_eq!(draw(&d), 0, "instance 0");
    let mut e = rig.draw(3);
    e.instance_id = 2;
    assert_eq!(draw(&e), 0, "does not read gl_InstanceID");
    assert_eq!(pvr_backend_flush(), 0);
}

/// glDrawBuffers({GL_NONE, GL_COLOR_ATTACHMENT1}) (review R-09): the GL
/// core gives slot 0 `gpu` 0, and slot 0 is the frame's own target, which
/// the end-of-tile emit always stores. The draw is refused by name with
/// -ENOTSUP (GL_INVALID_OPERATION), not drawn into attachment 0. Revert
/// check: without the refusal the first draw goes in.
#[test]
fn a_draw_whose_draw_buffer_0_is_none_is_refused_by_name() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_targets = 2;
    d.targets[0] = GlesDrawTarget { gpu: 0, w: d.target_w, h: d.target_h, format: d.target_format };
    d.targets[1] = GlesDrawTarget { gpu: d.target_gpu, w: d.target_w, h: d.target_h, format: d.target_format };
    assert_eq!(draw(&d), -util::ENOTSUP);
    assert_eq!(draw(&d), -util::ENOTSUP);
    let named = rig.logs().iter().filter(|l| l.contains("draw buffer 0 is GL_NONE")).count();
    assert_eq!(named, 1, "one line: {:?}", rig.logs());
    /* slot 0 written, one target: drawn */
    d.num_targets = 1;
    d.targets[0].gpu = d.target_gpu;
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
}

#[test]
fn a_per_instance_offset_that_would_wrap_empties_the_attribute() {
    /* the C wraps `first * stride` and hands the GPU a wild address */
    let mut d = GlesDraw::ZERO;
    d.num_attribs = 2;
    d.attribs[0] = GlesAttrib { components: 4, stride: 16, offset: 32, per_instance: 0, ..GlesAttrib::ZERO };
    d.attribs[1] = GlesAttrib { components: 4, stride: u32::MAX, offset: 32, per_instance: 1, ..GlesAttrib::ZERO };
    attribs_advance_instances(&mut d, 3);
    assert_eq!(d.attribs[0].offset, 32, "a per-vertex attribute is left alone");
    assert_eq!(d.attribs[1].offset, u32::MAX, "an offset that cannot be computed reads nothing");
    assert_eq!(d.attribs[1].cpu_bytes, 0);
}

#[test]
fn four_kicks_may_be_in_flight() {
    /* upstream 11a728d: one set of per-kick resources each. */
    let rig = Rig::new();

    for i in 0..5u32 {
        let mut d = rig.draw(3);
        d.target_gpu = HEAP_GENERAL_BASE + 0x2000_0000 + i as u64 * 0x10_0000;
        assert_eq!(draw(&d), 0);
        assert_eq!(pvr_backend_flush(), 0);
        assert_eq!(rig.b().cur, (i as usize + 1) % 4, "the sets rotate through all four");
    }
    /* a swap waits for everything but the most recent kick, by fence */
    assert_eq!(pvr_backend_finish_previous(), 0);
}

/// Three frames of one draw each, a swap's wait after each flush, kicks
/// completing only when waited for: the fences the swaps waited for, the
/// kicks (descriptor bytes and VDM words) and the backend's wait counter.
/// One kick as the platform saw it: the descriptor's bytes and its VDM words.
type SeenKick = (Vec<u8>, Vec<u32>);

fn three_swaps(one_in_flight: u32) -> (Vec<i64>, Vec<SeenKick>, u32) {
    let rig = Rig::new();
    pvr_backend_set_option(PVR_BACKEND_OPT_ONE_IN_FLIGHT, one_in_flight);
    with_platform(|p| p.hold_completion = true);
    /* the counters live across init: take deltas */
    let (mut kicks0, mut waits0) = (0u32, 0u32);
    // SAFETY: two valid u32 out-pointers.
    unsafe { pvr_backend_counters(&mut kicks0, &mut waits0) };
    for _ in 0..3 {
        let d = rig.draw(3);
        assert_eq!(draw(&d), 0);
        assert_eq!(pvr_backend_flush(), 0);
        assert_eq!(pvr_backend_finish_previous(), 0);
    }
    let (mut kicks, mut waits) = (0u32, 0u32);
    // SAFETY: two valid u32 out-pointers.
    unsafe { pvr_backend_counters(&mut kicks, &mut waits) };
    let (kicks, waits) = (kicks.wrapping_sub(kicks0), waits.wrapping_sub(waits0));
    assert_eq!(kicks, 3);
    let (fences, k) = with_platform(|p| {
        let k = p
            .kicks
            .iter()
            .map(|(desc, vdm)| {
                // SAFETY: a repr(C) descriptor read as its bytes.
                let bytes = unsafe {
                    core::slice::from_raw_parts(desc as *const PvrRenderKick as *const u8, size_of::<PvrRenderKick>())
                };
                (bytes.to_vec(), vdm.clone())
            })
            .collect::<Vec<_>>();
        (p.waits.clone(), k)
    });
    pvr_backend_set_option(PVR_BACKEND_OPT_ONE_IN_FLIGHT, 0);
    (fences, k, waits)
}

#[test]
fn one_kick_in_flight_changes_only_the_swaps_wait_target() {
    let (off_waits, off_kicks, off_count) = three_swaps(0);
    let (on_waits, on_kicks, on_count) = three_swaps(1);
    /* the default: a swap waits for every kick but its own (the first swap
     * has nothing before it) */
    assert_eq!(off_waits, [1, 2]);
    assert_eq!(off_count, 2);
    /* the knob: each swap waits for its own kick */
    assert_eq!(on_waits, [1, 2, 3]);
    assert_eq!(on_count, 3);
    /* and nothing else moves: the same kicks, byte for byte */
    assert_eq!(off_kicks.len(), 3);
    assert!(off_kicks == on_kicks, "the knob changed a kick");
}

/// Four frames into the rig's target with a depth/stencil buffer, each
/// clearing colour and depth (glmark2's frame): 1 swapped, 2 flushed with no
/// swap (glFinish, a GL-split frame), 3 flushed after the swap of another
/// surface, 4 swapped. The kicks the platform saw, with their VDM words.
fn four_frames_with_depth(swap_drops_zs: u32) -> Vec<(PvrRenderKick, Vec<u32>)> {
    let rig = Rig::new();
    pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, swap_drops_zs);
    for f in 1..=4 {
        let mut d = rig.draw(3);
        d.ds_persist = true;
        d.ds_id = 1;
        d.do_clear = true;
        d.do_clear_depth = true;
        d.clear_depth = 1.0;
        assert_eq!(draw(&d), 0);
        match f {
            1 | 4 => pvr_backend_surface_swapped(d.target_gpu),
            3 => pvr_backend_surface_swapped(d.target_gpu + 0x10_0000),
            _ => {}
        }
        assert_eq!(pvr_backend_flush(), 0);
    }
    let dropped = rig.b().swap_zs_dropped;
    assert_eq!(dropped, if swap_drops_zs != 0 { 2 } else { 0 }, "kicks whose store was dropped");
    let kicks = with_platform(|p| p.kicks.clone());
    pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, 0);
    kicks
}

fn kick_bytes(k: &PvrRenderKick) -> Vec<u8> {
    // SAFETY: a repr(C) descriptor read as its bytes.
    unsafe { core::slice::from_raw_parts(k as *const PvrRenderKick as *const u8, size_of::<PvrRenderKick>()) }.to_vec()
}

/// The swapped window frames (kicks 0 and 3) lose exactly the three store
/// bits. The frame after a dropped one (kick 1) loses what it would have
/// loaded: glmark2's clear is colour and depth (canvas-generic.cpp:103), so
/// by default it loads the stencil the kick before stored; with the store
/// dropped there is none, and its frame quad initialises the stencil
/// instead (that state is in the PPP words the VDM points at, which the rig
/// does not record). Kick 2 follows a stored frame in both runs and is the
/// same. Every other descriptor byte and every VDM word is the same.
#[test]
fn swap_drops_zs_changes_only_the_swapped_frames_store_and_the_next_frames_load() {
    const STORE: u64 = ZLSCTL_ZSTOREEN | ZLSCTL_SSTOREEN | ZLSCTL_FORCEZSTORE;
    const STENCIL_LOAD: u64 = ZLSCTL_SLOADEN | ZLSCTL_FORCEZLOAD;
    let off = four_frames_with_depth(0);
    let on = four_frames_with_depth(1);
    assert_eq!((off.len(), on.len()), (4, 4));
    let want = [STORE, STENCIL_LOAD, 0, STORE];
    for (i, ((a, av), (b, bv))) in off.iter().zip(&on).enumerate() {
        assert_eq!(a.frag.isp_zlsctl & STORE, STORE, "kick {i}: stored by default");
        assert_eq!(a.frag.isp_zlsctl ^ b.frag.isp_zlsctl, want[i], "kick {i}: the ZLSCTL bits that moved");
        let (mut a2, mut b2) = (*a, *b);
        a2.frag.isp_zlsctl = 0;
        b2.frag.isp_zlsctl = 0;
        assert!(kick_bytes(&a2) == kick_bytes(&b2), "kick {i}: a descriptor byte other than ZLSCTL moved");
        assert!(av == bv, "kick {i}: the VDM words moved");
    }
    /* by default kick 1 loads the stencil (the cost the option removes) */
    assert_eq!(off[1].0.frag.isp_zlsctl & STENCIL_LOAD, STENCIL_LOAD);
}

#[test]
fn after_a_dropped_swap_the_next_frame_initialises_its_depth_instead_of_loading() {
    let rig = Rig::new();
    pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, 1);
    let mut d = rig.draw(3);
    d.ds_persist = true;
    d.ds_id = 1;
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0);
    pvr_backend_surface_swapped(d.target_gpu);
    assert_eq!(pvr_backend_flush(), 0);
    /* a frame that does not clear depth: by default it would load what the
     * kick before stored; that kick stored nothing, so it starts defined */
    d.do_clear_depth = false;
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let k = rig.kicks();
    pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, 0);
    assert_eq!(k.len(), 2);
    assert_eq!(k[1].0.frag.isp_zlsctl & (ZLSCTL_ZLOADEN | ZLSCTL_SLOADEN), 0, "nothing stored, nothing loaded");
    assert_ne!(k[1].0.frag.isp_zlsctl & ZLSCTL_ZSTOREEN, 0, "no swap: this frame stores");
}

#[test]
fn swap_drops_zs_off_leaves_the_hook_inert() {
    let rig = Rig::new();
    pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, 0);
    let mut d = rig.draw(3);
    d.ds_persist = true;
    d.ds_id = 1;
    assert_eq!(draw(&d), 0);
    pvr_backend_surface_swapped(d.target_gpu);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.b().swap_zs_dropped, 0);
    assert_ne!(rig.kicks()[0].0.frag.isp_zlsctl & ZLSCTL_ZSTOREEN, 0);
    pvr_backend_set_option(PVR_BACKEND_OPT_SWAP_DROPS_ZS, PVR_SWAP_DROPS_ZS_DEFAULT);
}

#[test]
fn the_swap_drops_the_windows_depth_stencil_by_default() {
    /* PVR_SWAP_DROPS_ZS_DEFAULT is the product's default and the rig's */
    assert_eq!(PVR_SWAP_DROPS_ZS_DEFAULT, 1);
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.ds_persist = true;
    d.ds_id = 1;
    assert_eq!(draw(&d), 0);
    pvr_backend_surface_swapped(d.target_gpu);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.b().swap_zs_dropped, 1);
    assert_eq!(rig.kicks()[0].0.frag.isp_zlsctl & ZLSCTL_ZSTOREEN, 0);
}

#[test]
fn a_kick_with_nothing_to_tile_still_carries_one_object() {
    /* upstream a4afa3a: a freshly allocated HWRT set is zeros, so a geometry
     * phase that writes no region headers makes the ISP follow a null
     * pointer. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.index_count = 0; /* a clear with no draw behind it */
    d.do_clear = true;

    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().ndraws, 0, "a colour clear alone records nothing");
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks().len(), 1);
    assert_eq!(rig.index_lists().len(), 1, "one covering object reached the tiler");
    assert!(rig.ppp()[0].color_write_disable != 0, "it writes no colour");
}

#[test]
fn a_colour_clear_does_not_join_a_frame_that_began_by_loading_colour() {
    /* The frame's first object reloads the old colour when it began without
     * a colour clear; a colour clear made its background afterwards was
     * painted over by that load quad. AM62P 2026-09-24: dEQP color_clear's
     * scissored cases, and glColorMask(0,0,0,0) + glClear(COLOR|DEPTH) then
     * glClear(COLOR), read back the picture from before the clear. */
    let rig = Rig::new();
    let mut first = rig.draw(3);
    first.index_count = 0;
    first.do_clear = false; /* depth only: the frame loads colour */
    first.do_clear_depth = true;
    assert_eq!(draw(&first), 0);
    assert!(rig.b().frame_loads_color);

    let mut colour = rig.draw(3);
    colour.index_count = 0;
    colour.do_clear = true;
    colour.clear_rgba = [0.0, 1.0, 0.0, 1.0];
    assert_eq!(draw(&colour), 0);
    assert!(rig.b().frame_cleared && !rig.b().frame_loads_color, "the colour clear starts its own frame");
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks().len(), 2, "the loading frame was submitted first");
}

#[test]
fn a_clear_quad_whose_box_misses_the_target_opens_no_frame() {
    let rig = Rig::new();
    let mut q = rig.draw(3);
    q.index_count = 0;
    q.clear_quad = true;
    q.scissor = true;
    q.scissor_rect = [-200, 0, 50, 50];
    let before = rig.b().draws_empty_box;
    assert_eq!(draw(&q), 0);
    assert!(!rig.b().frame_open, "nothing to clear, nothing opened");
    assert_eq!(rig.b().draws_empty_box, before + 1);
}

#[test]
fn a_draw_without_attributes_is_recorded() {
    /* upstream b879c58: a vertex shader may build its geometry from
     * gl_VertexID alone (Qt Quick 3D's post-processing passes). */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_attribs = 0;

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.streams()[0].len(), 0, "the vertex-fetch program has no stream");
    assert_eq!(rig.index_lists().len(), 1);
}

#[test]
fn a_depth_only_frame_quad_does_not_sample_the_target() {
    /* upstream 892e5b6: its colour write is masked, so the full-screen fetch
     * was work nothing kept. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.do_clear = true;
    d.do_clear_depth = true;
    d.clear_depth = 1.0;
    d.ds_persist = true;
    d.ds_id = 7;

    assert_eq!(draw(&d), 0);
    /* the frame opens with a depth-init quad: a single uniform dword and no
     * descriptor block, so nothing samples the target */
    let kicks = rig.dma_kicks();
    let quad = &kicks[0];
    assert_eq!(quad, &[(quad[0].0, 1, 0)], "one dword of clear colour, no descriptors");
}

#[test]
fn three_colour_attachments_get_a_pbe_state_and_an_eot_program_each() {
    /* upstream e471a73: a G-buffer pass writes several attachments. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.num_targets = 3;
    d.targets[0] = GlesDrawTarget { gpu: d.target_gpu, w: 64, h: 64, format: GLES_SURF_RGBA8888 };
    d.targets[1] = GlesDrawTarget { gpu: HEAP_GENERAL_BASE + 0x2100_0000, w: 64, h: 64, format: GLES_SURF_RGBA8888 };
    d.targets[2] = GlesDrawTarget { gpu: HEAP_GENERAL_BASE + 0x2200_0000, w: 32, h: 16, format: GLES_SURF_RGBA8888 };

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let (k, _) = rig.kicks()[0];
    assert_eq!(k.frag.num_rt, 3);
    assert_eq!(k.frag.event_pixel_pds_info, 0x63, "the three-attachment end-of-tile program");
    assert_eq!(k.frag.pbe_word[1][1], 63 | (63 << 32));
    assert_eq!(k.frag.pbe_word[2][1], 15 | (31 << 32), "each attachment has its own size");
    assert_eq!(k.frag.pbe_word[2][0] >> 34 & 0x3ff, 15, "and its own line stride");
}

/// The GPU address one emit pair names (PBESTATE word 0 = address >> 2,
/// word 1 bits 0..5 = address >> 34).
fn emit_address(w0: u32, w1: u32) -> u64 {
    ((w0 as u64) << 2) | (((w1 & 0x3f) as u64) << 34)
}

#[test]
fn a_slot_with_no_target_is_configured_but_never_stored() {
    /* glDrawBuffers(ATT0, NONE, ATT2) reaches the backend as targets
     * (A, gpu 0, C) (resolve_targets). The middle output still takes its
     * register, so C reads register 2, but nothing is stored for it: an emit
     * with gpu 0 would store the tile to GPU address 0.
     * Revert check: without the skip in the emit loop there are three emits
     * and the second names address 0. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    let (a, c) = (d.target_gpu, HEAP_GENERAL_BASE + 0x2200_0000);
    d.num_targets = 3;
    d.targets[0] = GlesDrawTarget { gpu: a, w: 64, h: 64, format: GLES_SURF_RGBA8888 };
    d.targets[1] = GlesDrawTarget::ZERO;
    d.targets[2] = GlesDrawTarget { gpu: c, w: 32, h: 16, format: GLES_SURF_RGBA8888 };

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let (k, _) = rig.kicks()[0];
    assert_eq!(k.frag.num_rt, 2, "A and C are stored, the NONE slot is not");
    let emits = rig.eot_emits();
    let e = emits.last().expect("an end-of-tile program");
    assert_eq!(emit_address(e[0], e[1]), a);
    assert_eq!(emit_address(e[2], e[3]), c, "the second emit is C's");
    assert_eq!((e[3] >> 14) & 3, 2, "C's data comes from output register 2: the NONE slot kept register 1");
    assert_eq!((e[3] >> 16) & 7, 1, "and C takes PBE register words 1");
    assert_eq!(&e[4..6], &[0, 0], "no third emit");
    assert_eq!(k.frag.pbe_word[1][1], 15 | (31 << 32), "PBE words 1 are C's size");
    assert!(
        emits.iter().all(|e| e.chunks(2).take(k.frag.num_rt as usize).all(|p| emit_address(p[0], p[1]) != 0)),
        "no emitted state names address 0"
    );
}

#[test]
fn a_change_of_the_other_attachments_starts_a_new_frame() {
    /* A frame stores the slots it began with: a draw into (A, none, C)
     * after one into (A, B, C) must not share its frame, or its output 1
     * would be stored into B.
     * Revert check: without the targets comparison in frame_ends_here both
     * draws are one kick. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    let a = d.target_gpu;
    d.num_targets = 3;
    d.targets[0] = GlesDrawTarget { gpu: a, w: 64, h: 64, format: GLES_SURF_RGBA8888 };
    d.targets[1] = GlesDrawTarget { gpu: HEAP_GENERAL_BASE + 0x2100_0000, w: 64, h: 64, format: GLES_SURF_RGBA8888 };
    d.targets[2] = GlesDrawTarget { gpu: HEAP_GENERAL_BASE + 0x2200_0000, w: 64, h: 64, format: GLES_SURF_RGBA8888 };
    assert_eq!(draw(&d), 0);
    /* no clear on the second draw: a clear after draws is a new frame by
     * itself, which would hide the rule under test */
    d.do_clear = false;
    /* control: the same attachments share the frame */
    assert_eq!(draw(&d), 0);
    d.targets[1] = GlesDrawTarget::ZERO;
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let kicks = rig.kicks();
    assert_eq!(kicks.len(), 2, "(A, B, C) twice in one frame, then (A, none, C) in its own");
    assert_eq!((kicks[0].0.frag.num_rt, kicks[1].0.frag.num_rt), (3, 2));
}

#[test]
fn an_attachment_past_the_pixel_output_registers_is_not_emitted() {
    /* upstream 99f6c1d: a pixel has four output registers and an RGBA16F
     * attachment takes two, so the third does not fit. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.target_format = GLES_SURF_RGBA16F;
    d.num_targets = 3;
    for i in 0..3u64 {
        d.targets[i as usize] = GlesDrawTarget {
            gpu: HEAP_GENERAL_BASE + 0x2000_0000 + i * 0x10_0000,
            w: 64,
            h: 64,
            format: GLES_SURF_RGBA16F,
        };
    }
    d.target_gpu = d.targets[0].gpu;

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let (k, _) = rig.kicks()[0];
    assert_eq!(k.frag.num_rt, 2, "two RGBA16F attachments fill the four registers");
    assert!(rig.logs().iter().any(|l| l.contains("needs output register 4 of 4")), "{:?}", rig.logs());
}

#[test]
fn a_spilled_output_takes_the_tile_buffer_end_of_tile_program() {
    /* upstream 153810d: the shader stores it to memory and the end-of-tile
     * program loads it back before emitting that attachment. */
    let rig = Rig::new();
    let mut ffs = *rig.ffs;
    ffs.num_outputs = 3;
    ffs.num_tile_buffers = 1;
    ffs.tile_buffer_sh = 40;
    let mut d = rig.draw(3);
    d.fs = &ffs;
    d.target_format = GLES_SURF_RGBA16F;
    d.num_targets = 3;
    for i in 0..3u64 {
        d.targets[i as usize] = GlesDrawTarget {
            gpu: HEAP_GENERAL_BASE + 0x2000_0000 + i * 0x10_0000,
            w: 64,
            h: 64,
            format: GLES_SURF_RGBA16F,
        };
    }
    d.target_gpu = d.targets[0].gpu;

    assert_eq!(draw(&d), 0);
    /* the buffer's address reaches the shared registers the compiler named */
    let kicks = rig.dma_kicks();
    let last = kicks.last().expect("a DMA program");
    let tb = last.iter().find(|k| k.2 == 40).expect("a kick at the tile-buffer register");
    assert_eq!(tb.1, 2, "two dwords of address");
    assert_eq!(rig.gpu_u32(tb.0) as u64 | ((rig.gpu_u32(tb.0 + 4) as u64) << 32), TILE_BUFFER_GPU);
    assert_eq!(pvr_backend_flush(), 0);
    let (k, _) = rig.kicks()[0];
    assert_eq!(k.frag.num_rt, 3, "the third attachment is emitted from the tile buffer");
    assert_eq!(k.frag.event_pixel_pds_info, 0x73, "the tile-buffer end-of-tile program");
}

#[test]
fn the_vertex_stage_gets_texture_descriptors_of_its_own() {
    /* upstream 0a57cc6: Qt Quick 3D reads bone matrices from a texture in the
     * vertex shader, and the compiler puts the uniform block after the
     * descriptors. */
    let rig = Rig::new();
    let uni = [ShcUniform { dword_off: SHC_UNIFORM_NO_BLOCK, binding: 0, dwords: 1, ..ShcUniform::ZERO }];
    let mut fvs = *rig.fvs;
    fvs.uniforms = uni.as_ptr();
    fvs.num_uniforms = 1;
    let block = [0u32; 8];
    let mut d = rig.draw(3);
    d.vs = &fvs;
    d.vs_uniform_dwords = 8;
    d.vs_uniform_cpu = block.as_ptr() as *const c_void;
    d.vs_textures[0] = GlesTextureBind {
        unit: 0,
        base_gpu: HEAP_GENERAL_BASE + 0x3000_0000,
        width: 4,
        height: 4,
        stride_texels: 4,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    };
    d.num_vs_textures = 1;

    assert_eq!(draw(&d), 0);
    /* the vertex DMA program is the last one recorded for this draw */
    let vk = rig.dma_kicks().pop().expect("the vertex DMA program");
    assert_eq!(vk.len(), 2);
    assert_eq!((vk[0].1, vk[0].2), (TEX_DESC_DWORDS, 0), "one descriptor at sh0");
    assert_eq!((vk[1].1, vk[1].2), (8, TEX_DESC_DWORDS), "the block behind it");
}

#[test]
fn the_whole_uniform_block_reaches_the_shared_registers() {
    /* upstream 8fdf5b8: the compiler's push-constant count is not a bound on
     * what the shader reads, so the prefix trim is off. */
    let rig = Rig::new();
    let mut ffs = *rig.ffs;
    ffs.uniform_dwords = 100;
    ffs.uniform_sh_dwords = 64;
    let block = [0u32; 100];
    let mut d = rig.draw(3);
    d.fs = &ffs;
    d.fs_uniform_dwords = 100;
    d.fs_uniform_cpu = block.as_ptr() as *const c_void;

    assert_eq!(draw(&d), 0);
    let k = &rig.dma_kicks()[0];
    assert_eq!(k[0].1, 100, "the whole block, not the compiler's 64-dword prefix");
}

#[test]
fn a_line_is_never_culled() {
    /* upstream c521beb: GL culls polygons only (ES 3.0 13.6.1), and the TA
     * would apply the test to the quad it expands a line into. */
    let rig = Rig::new();

    for (prim, culled) in [
        (GLES_PRIM_TRIANGLES, true),
        (GLES_PRIM_LINES, false),
        (GLES_PRIM_LINE_STRIP, false),
        (GLES_PRIM_POINTS, false),
    ] {
        rig.reset_records();
        let mut d = rig.draw(6);
        d.prim = prim;
        d.cull = GLES_CULL_CW;
        d.flip_y = true;
        assert_eq!(draw(&d), 0);
        let st = rig.ppp().pop().expect("a PPP state");
        assert_eq!(st.cullmode != PVR_TA_CULLMODE_NONE, culled, "prim {prim}");
    }
}

#[test]
fn a_resized_depth_stencil_buffer_goes_back_to_the_pool() {
    /* upstream 99f6c1d: the old allocation used to be leaked. */
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.ds_persist = true;
    d.ds_id = 5;

    assert_eq!(draw(&d), 0);
    let first = rig.b().ds[0].mem.gpu;
    assert_eq!(pvr_backend_flush(), 0);
    d.target_w = 128;
    d.viewport[2] = 128;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.frees(), [first], "the buffer sized for the old target was handed back");
    assert_ne!(rig.b().ds[0].mem.gpu, first);
    assert_eq!(pvr_backend_flush(), 0);
}

/// Framebuffer `i` of the cycling tests: a colour target of its own and a
/// depth renderbuffer (object 100 + i), depth test and writes on.
fn fbo_draw(rig: &Rig, i: u32) -> GlesDraw {
    let mut d = rig.draw(3);
    d.target_gpu = HEAP_GENERAL_BASE + 0x2000_0000 + i as u64 * 0x10_0000;
    d.do_clear = false;
    d.ds_persist = true;
    d.ds_id = (i + 1) << 16;
    (d.depth_id, d.depth_gen) = (100 + i, 1);
    (d.depth_test, d.depth_write, d.depth_func) = (true, true, GLES_CMP_LESS);
    d
}

/// The depth buffer the backend keeps for object `id`.
fn ds_of(rig: &Rig, id: u32) -> Option<DsBuf> {
    rig.b().ds.iter().copied().find(|s| !s.mem.impl_.is_null() && s.owners.contains(&id))
}

/// Draw framebuffers 0..8, each into its own depth buffer, and write a mark
/// into framebuffer 0's (as the ISP's store would).
fn eight_fbos_drawn(rig: &Rig) -> DsBuf {
    for i in 0..PVR_BACKEND_MAX_DS as u32 {
        assert_eq!(draw(&fbo_draw(rig, i)), 0, "framebuffer {i}: {:?}", rig.logs());
        assert_eq!(pvr_backend_flush(), 0);
    }
    let first = ds_of(rig, 100).expect("framebuffer 0 keeps a depth buffer");
    with_platform(|p| {
        let a = p.allocs.get_mut(&first.mem.gpu).expect("its allocation");
        // SAFETY: the backing holds the buffer's bytes, u64-aligned.
        unsafe { (a.mem.as_mut_ptr() as *mut u32).write(0x5a5a_1234) };
    });
    first
}

/// The refusal: with all eight buffers holding depth a
/// draw wrote, a ninth framebuffer's first depth-tested draw is refused
/// (-ENOMEM, which the GL core reports as GL_OUT_OF_MEMORY) with one named
/// line, and framebuffer 0's depth is still there: not freed, its bytes
/// unchanged, and loaded by its next kick.
///
/// Revert check: with the old eviction the ninth draw returns 0 and
/// framebuffer 0's buffer is in `frees()`.
#[test]
fn a_ninth_framebuffer_with_depth_is_refused_and_the_first_keeps_its_depth() {
    let rig = Rig::new();
    let first = eight_fbos_drawn(&rig);
    assert_eq!(draw(&fbo_draw(&rig, 8)), -ENOMEM, "the ninth is refused");
    let logs = rig.logs();
    let refused: Vec<_> = logs.iter().filter(|l| l.contains(ds_retention::REFUSED)).collect();
    assert_eq!(refused.len(), 1, "{logs:?}");
    assert!(refused[0].contains("object 0x6c") && refused[0].contains("all 8"), "{refused:?}");
    assert_eq!(draw(&fbo_draw(&rig, 8)), -ENOMEM, "still refused");
    assert!(rig.logs().is_empty(), "the same refusal is not logged again");
    assert!(rig.frees().is_empty(), "no buffer was given up");
    assert_eq!(rig.gpu_u32(first.mem.gpu), 0x5a5a_1234, "framebuffer 0's depth bytes are intact");
    rig.reset_records();
    assert_eq!(draw(&fbo_draw(&rig, 0)), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let k = rig.kicks();
    assert_eq!(k[0].0.frag.isp_zload_store_base, first.mem.gpu, "framebuffer 0 draws with its own buffer");
    assert_ne!(k[0].0.frag.isp_zlsctl & ZLSCTL_ZLOADEN, 0, "and loads what it stored");
}

/// The run's depth/stencil line as the OSAL prints it at exit
/// (`pvr_backend_ds_report`), counted from a clean start: the peaks and
/// refusal counts survive a backend init on purpose (a process-wide
/// report), so each test zeroes them first.
fn ds_line_from_zero(rig: &Rig) -> impl Fn() -> String {
    let b = rig.b();
    (b.ds_slots_peak, b.ds_bytes_peak, b.ds_full_refused, b.ds_refused) = (0, 0, 0, 0);
    || {
        let mut buf = [0 as c_char; 160];
        // SAFETY: buf is writable for its length.
        unsafe { pvr_backend_ds_report(buf.as_mut_ptr(), buf.len()) };
        // SAFETY: the report NUL-terminates inside buf.
        unsafe { core::ffi::CStr::from_ptr(buf.as_ptr()) }.to_string_lossy().into_owned()
    }
}

/// No buffer allocated is its own form, never "0/8" (which would read as
/// measured headroom); refusals are still counted in it.
#[test]
fn the_ds_line_says_none_allocated_when_no_buffer_ever_was() {
    let rig = Rig::new();
    let line = ds_line_from_zero(&rig);
    assert_eq!(line(), "ds slots: none allocated, ds refused 0 (slots full 0, no buffer 0)");
    let mut d = rig.draw(3);
    d.ds_persist = false;
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(line(), "ds slots: none allocated, ds refused 0 (slots full 0, no buffer 0)", "{:?}", rig.logs());
}

/// One, eight and nine framebuffers with depth: the peak counts buffers
/// held at once (of PVR_BACKEND_MAX_DS), the bytes are the sum of their
/// allocations (the rig's 64x64 target: two 32-pixel ZLS tiles each way, 4
/// bytes a pixel),
/// and the ninth is refused with the slots full, counted once per draw.
#[test]
fn the_ds_line_counts_one_eight_and_nine_framebuffers() {
    let rig = Rig::new();
    let line = ds_line_from_zero(&rig);
    let one = ds_bytes(64, 64, 1).expect("fits");
    assert_eq!(one, 16384);
    assert_eq!(draw(&fbo_draw(&rig, 0)), 0);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(ds_of(&rig, 100).expect("framebuffer 0's buffer").mem.size, one);
    assert_eq!(line(), "ds slots peak 1/8, ds bytes peak 16384, ds refused 0 (slots full 0, no buffer 0)");
    for i in 1..PVR_BACKEND_MAX_DS as u32 {
        assert_eq!(draw(&fbo_draw(&rig, i)), 0, "framebuffer {i}: {:?}", rig.logs());
        assert_eq!(pvr_backend_flush(), 0);
    }
    assert_eq!(line(), "ds slots peak 8/8, ds bytes peak 131072, ds refused 0 (slots full 0, no buffer 0)");
    assert_eq!(draw(&fbo_draw(&rig, 8)), -ENOMEM, "the ninth is refused");
    assert_eq!(line(), "ds slots peak 8/8, ds bytes peak 131072, ds refused 1 (slots full 1, no buffer 0)");
    assert_eq!(draw(&fbo_draw(&rig, 8)), -ENOMEM, "and again");
    assert_eq!(line(), "ds slots peak 8/8, ds bytes peak 131072, ds refused 2 (slots full 2, no buffer 0)");
    /* a freed slot lowers what is held, never the peak */
    pvr_backend_attachment_gone(100);
    assert_eq!(draw(&fbo_draw(&rig, 8)), 0, "the ninth takes the freed slot: {:?}", rig.logs());
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(line(), "ds slots peak 8/8, ds bytes peak 131072, ds refused 2 (slots full 2, no buffer 0)");
}

/// A buffer that could not be allocated is counted apart from a full table.
#[test]
fn the_ds_line_counts_a_refusal_for_no_buffer_apart() {
    let rig = Rig::new();
    let line = ds_line_from_zero(&rig);
    with_platform(|p| p.fail_alloc_in = 1);
    assert_eq!(draw(&fbo_draw(&rig, 0)), -ENOMEM, "{:?}", rig.logs());
    assert_eq!(line(), "ds slots: none allocated, ds refused 1 (slots full 0, no buffer 1)");
}

/// `depth_resolve` reads the depth a kick left for an object as the
/// texture's floats: the low 24 bits of each packed D24S8 word (the stencil
/// byte ignored), row-linear at the ZLS-tile-aligned width (64 for the
/// rig's 64x64 target), only inside the buffer. A whole clear is its value
/// everywhere, here and once parked; an object nothing holds answers 1 and
/// leaves the texels; the frame still drawing into it answers -EBUSY.
#[test]
fn depth_resolve_reads_the_packed_depth_as_floats() {
    let rig = Rig::new();
    let id = 0x4000_0064; /* a texture's id form */
    let mut d = fbo_draw(&rig, 0);
    d.depth_id = id;
    assert_eq!(draw(&d), 0);
    assert_eq!(
        unsafe { pvr_backend_depth_resolve(id, [0f32; 1].as_mut_ptr(), 1, 1, 1) },
        -crate::util::EBUSY,
        "the open frame"
    );
    assert_eq!(pvr_backend_flush(), 0);
    let buf = ds_of(&rig, id).expect("the object's buffer");
    assert!(buf.valid && buf.planes[DEPTH].is_live());
    with_platform(|p| {
        let a = p.allocs.get_mut(&buf.mem.gpu).expect("its allocation");
        let w = a.mem.as_mut_ptr() as *mut u32;
        // SAFETY: the backing holds 64x64 words.
        unsafe {
            w.add(2 * 64 + 3).write(0xab80_0000); /* (3, 2): stencil 0xab, depth 2^23 */
            w.add(3 * 64 + 1).write(0x00ff_ffff); /* (1, 3): depth 1.0 */
            w.add(0).write(0xff00_0000); /* (0, 0): depth 0 */
        }
    });
    let mut t = [7.0f32; 5 * 4];
    // SAFETY: 4 rows of 5 floats.
    assert_eq!(unsafe { pvr_backend_depth_resolve(id, t.as_mut_ptr(), 4, 4, 5) }, 0);
    assert_eq!(t[2 * 5 + 3], 8_388_608.0 / 16_777_215.0);
    assert_eq!(t[3 * 5 + 1], 1.0);
    assert_eq!(t[0], 0.0);
    assert_eq!(t[4], 7.0, "past the width: the pitch's padding is not written");
    /* a texture larger than the buffer keeps its texels outside it */
    let mut big = [7.0f32; 70];
    // SAFETY: 1 row of 70 floats.
    assert_eq!(unsafe { pvr_backend_depth_resolve(id, big.as_mut_ptr(), 70, 1, 70) }, 0);
    assert!((0.0..=1.0).contains(&big[63]) && big[64] == 7.0, "written up to x 63 only: {:?}", &big[62..66]);
    /* nothing held for another object */
    // SAFETY: as above.
    assert_eq!(unsafe { pvr_backend_depth_resolve(id + 1, t.as_mut_ptr(), 4, 4, 5) }, 1);
    assert_eq!(t[4], 7.0);
    /* a whole clear */
    let mut clear = fbo_draw(&rig, 0);
    (clear.depth_id, clear.index_count, clear.do_clear_depth, clear.clear_depth) = (id, 0, true, 0.25);
    assert_eq!(draw(&clear), 0);
    assert_eq!(pvr_backend_flush(), 0);
    // SAFETY: as above.
    assert_eq!(unsafe { pvr_backend_depth_resolve(id, t.as_mut_ptr(), 4, 4, 5) }, 0);
    assert_eq!((t[0], t[3 * 5 + 3], t[4]), (0.25, 0.25, 7.0));
    /* parked: the buffer let go, its clear value kept */
    for i in 1..=PVR_BACKEND_MAX_DS as u32 {
        assert_eq!(draw(&fbo_draw(&rig, i)), 0, "framebuffer {i}: {:?}", rig.logs());
        assert_eq!(pvr_backend_flush(), 0);
    }
    assert!(ds_of(&rig, id).is_none() && rig.b().ds_parked.len() == 1, "{:?}", rig.logs());
    t = [7.0; 20];
    // SAFETY: as above.
    assert_eq!(unsafe { pvr_backend_depth_resolve(id, t.as_mut_ptr(), 4, 4, 5) }, 0);
    assert_eq!((t[0], t[18], t[19]), (0.25, 0.25, 7.0));
}

/// The GL core deleting framebuffer 0's depth renderbuffer frees its
/// buffer exactly, and the ninth framebuffer gets that slot.
#[test]
fn a_deleted_depth_object_gives_its_buffer_to_the_ninth() {
    let rig = Rig::new();
    let first = eight_fbos_drawn(&rig);
    pvr_backend_attachment_gone(100);
    assert_eq!(rig.frees(), [first.mem.gpu], "object 100's buffer, and only it");
    assert_eq!(draw(&fbo_draw(&rig, 8)), 0, "{:?}", rig.logs());
    assert_eq!(pvr_backend_flush(), 0);
    assert!(ds_of(&rig, 108).is_some());
}

/// A whole depth clear with no draw after it leaves one value in the
/// buffer: its slot may go to the ninth framebuffer, and the value is kept
/// (parked). When framebuffer 0 draws again (after another object is
/// deleted, for room) its new buffer starts from that clear value, not
/// loaded and not 1.0.
#[test]
fn a_whole_depth_clear_frees_the_slot_and_the_clear_value_comes_back() {
    let rig = Rig::new();
    let first = eight_fbos_drawn(&rig);
    let mut clear = fbo_draw(&rig, 0);
    (clear.index_count, clear.do_clear_depth, clear.clear_depth) = (0, true, 0.25);
    assert_eq!(draw(&clear), 0);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(ds_of(&rig, 100).map(|s| s.planes[DEPTH]), Some(Plane::cleared(0.25f32.to_bits())));
    assert_eq!(draw(&fbo_draw(&rig, 8)), 0, "the cleared buffer's slot is free: {:?}", rig.logs());
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.frees(), [first.mem.gpu]);
    assert_eq!(rig.b().ds_parked.len(), 1, "framebuffer 0's clear value is kept");
    assert_eq!(draw(&fbo_draw(&rig, 0)), -ENOMEM, "every slot holds depth again");
    pvr_backend_attachment_gone(101);
    rig.reset_records();
    let mut again = fbo_draw(&rig, 0);
    again.depth_write = false; /* a test against what is there */
    assert_eq!(draw(&again), 0, "{:?}", rig.logs());
    assert_eq!(rig.b().clear_depth, 0.25, "the new buffer starts from the parked clear value");
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(rig.kicks()[0].0.frag.isp_zlsctl & ZLSCTL_ZLOADEN, 0, "initialised, not loaded");
    assert!(rig.b().ds_parked.is_empty());
    assert_eq!(
        ds_of(&rig, 100).map(|s| s.planes[DEPTH]),
        Some(Plane::cleared(0.25f32.to_bits())),
        "still only the clear"
    );
}

/* ---- a depth/stencil buffer that could not be allocated (review #6) ----------- */

/// A 64x64 target with a packed depth/stencil attachment (an FBO's `ds_id`),
/// drawn with no clear and no depth or stencil test.
fn ds_target(rig: &Rig) -> GlesDraw {
    let mut d = rig.draw(3);
    d.do_clear = false;
    d.ds_persist = true;
    d.ds_id = 0x0002_0003;
    d.depth_func = GLES_CMP_LESS;
    d
}

/// The refusal lines logged since the last call.
fn ds_refusals(rig: &Rig) -> Vec<String> {
    rig.logs().into_iter().filter(|l| l.contains("yakogl/pvr: draw refused: no depth/stencil buffer")).collect()
}

/// The finding: when the depth/stencil buffer could not be allocated, the
/// frame used to be built with `frame_ds` 0 -- no load, no store -- and a
/// depth-tested draw tested against on-chip values starting from the clear
/// (or 1.0), then lost what it wrote at the kick's end, with rc 0. Now it is
/// refused (-ENOMEM, the contract's "does not fit" row) with one named line
/// per frame and a count; a draw that uses no depth or stencil goes ahead;
/// the contents stay lost for a depth or stencil test until a clear covers
/// that half, per half.
#[test]
fn a_draw_that_tests_a_depth_buffer_that_could_not_be_allocated_is_refused() {
    let rig = Rig::new();
    let refused0 = pvr_backend_ds_refused();
    rig.logs();

    /* the next allocation is the depth/stencil buffer's: it fails */
    let mut d = ds_target(&rig);
    d.depth_test = true;
    d.depth_write = true;
    with_platform(|p| p.fail_alloc_in = 1);
    assert_eq!(draw(&d), -ENOMEM, "a depth-tested draw without a depth buffer");
    assert!(!rig.b().frame_open, "the refused draw opened no frame");
    let lines = ds_refusals(&rig);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("for a 64x64 target; it tests or clears depth"), "{lines:?}");
    assert!(lines[0].contains(&format!("({} refused so far)", refused0 + 1)), "the count: {lines:?}");
    /* a clear of it is refused the same way, and within one frame the line
     * is not repeated */
    let mut clear = ds_target(&rig);
    (clear.index_count, clear.do_clear_depth, clear.do_clear_stencil) = (0, true, true);
    with_platform(|p| p.fail_alloc_in = 1);
    assert_eq!(draw(&clear), -ENOMEM, "a depth/stencil clear without a buffer");
    assert!(ds_refusals(&rig).is_empty(), "one line per frame");
    assert_eq!(pvr_backend_ds_refused(), refused0 + 2, "the count has every refusal");

    /* the same target, no depth or stencil test: drawn, without a buffer */
    with_platform(|p| p.fail_alloc_in = 1);
    assert_eq!(draw(&ds_target(&rig)), 0, "a draw that uses no depth/stencil goes ahead");
    assert_eq!(rig.b().frame_ds, DsStore::NoMemory, "no memory, not no attachment");
    assert_eq!(pvr_backend_flush(), 0);
    let k = rig.kicks();
    assert_eq!(k.len(), 1);
    assert_eq!(k[0].0.frag.cmd_flags & FRAG_FLAGS_DEPTHBUFFER, 0, "that kick keeps no depth");

    /* memory is back, but what the attachment held is gone: a depth test
     * without a clear is still refused (a new frame: logged again) */
    assert_eq!(draw(&d), -ENOMEM, "the lost depth sticks to the attachment");
    let lines = ds_refusals(&rig);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("it tests depth that was lost with its buffer"), "{lines:?}");
    /* a scissored depth clear covers part of it: still lost */
    let mut part = ds_target(&rig);
    (part.index_count, part.do_clear_depth, part.clear_quad) = (0, true, true);
    (part.scissor, part.scissor_rect) = (true, [0, 0, 8, 8]);
    assert_eq!(draw(&part), 0, "a partial clear of a buffer that exists is drawn");
    assert_eq!(draw(&d), -ENOMEM, "a partial clear does not make the depth defined");

    /* a full depth clear fused into the draw: the target renders again */
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0, "a covering depth clear, then the depth-tested draw");
    d.do_clear_depth = false;
    assert_eq!(draw(&d), 0, "and later draws test it");
    /* the packed buffer's stencil was lost too, and only depth was cleared */
    let mut st = ds_target(&rig);
    (st.stencil, st.stencil_func) = (true, GLES_CMP_EQUAL);
    assert_eq!(draw(&st), -ENOMEM, "the stencil half is still lost");
    st.do_clear_stencil = true;
    st.clear_stencil_write_mask = 0xff;
    assert_eq!(draw(&st), 0, "a covering stencil clear, then the stencil-tested draw");
    st.do_clear_stencil = false;
    assert_eq!(draw(&st), 0);
    assert_eq!(rig.b().nds_lost, 0, "nothing is remembered lost");
    assert_eq!(pvr_backend_flush(), 0);
    assert_ne!(rig.kicks().last().unwrap().0.frag.cmd_flags & FRAG_FLAGS_DEPTHBUFFER, 0, "a kick with depth");
    assert_eq!(pvr_backend_ds_refused(), refused0 + 5);
}

/// A target with no depth/stencil attachment is not a target whose buffer
/// could not be allocated: its depth-tested draw (which the GL core sends
/// only with an attachment, so this is the backend's own guard) is drawn.
#[test]
fn a_target_without_a_depth_attachment_is_not_refused() {
    let rig = Rig::new();
    let refused0 = pvr_backend_ds_refused();
    let mut d = rig.draw(3);
    d.depth_test = true;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().frame_ds, DsStore::NoAttachment);
    assert_eq!(pvr_backend_flush(), 0);
    assert_eq!(pvr_backend_ds_refused(), refused0);
}

/// Apart (pvr_split_zs): a stencil object whose buffer could not be
/// allocated refuses a stencil test; the depth, which has its buffer, is
/// tested.
#[test]
fn a_separate_stencil_that_could_not_be_allocated_refuses_only_the_stencil_test() {
    let rig = Rig::new();
    pvr_split_zs(true);
    let mut d = ds_target(&rig);
    (d.depth_id, d.depth_gen, d.stencil_id, d.stencil_gen) = (12, 1, 13, 1);
    d.depth_test = true;
    /* the depth's allocation succeeds, the stencil's (the second) fails */
    with_platform(|p| p.fail_alloc_in = 2);
    assert_eq!(draw(&d), 0, "a depth test with the depth buffer there");
    assert!(rig.b().frame_ds.buf().is_some());
    assert_eq!(rig.b().frame_s, DsStore::NoMemory);
    d.stencil = true;
    assert_eq!(draw(&d), -ENOMEM, "a stencil test without the stencil's buffer");
    assert_eq!(pvr_backend_flush(), 0);
    pvr_split_zs(false);
}

/// At the GL level: glClear(GL_DEPTH_BUFFER_BIT) on the window surface, whose
/// depth/stencil buffer could not be allocated, is GL_OUT_OF_MEMORY (the
/// clear is fused into the kick glFlush makes, so it is refused as a draw).
#[test]
fn a_gl_depth_clear_without_a_depth_buffer_is_out_of_memory() {
    use crate::gl::consts::*;
    let rig = Rig::new();
    // SAFETY: serialised by the rig's TestGl; TestGl's drop resets the table.
    unsafe { crate::backend::g_backend = &yakogl_backend_pvr };
    rig.logs();
    with_platform(|p| p.fail_alloc_in = 1);
    // SAFETY: plain GL calls on the rig's context.
    unsafe {
        crate::gl::state::glClear(GL_DEPTH_BUFFER_BIT);
        crate::gl::state::glFlush();
    }
    assert_eq!(rig.gl.error(), GL_OUT_OF_MEMORY);
    let lines = ds_refusals(&rig);
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(lines[0].contains("it tests or clears depth"), "{lines:?}");
    assert!(rig.kicks().is_empty(), "nothing was kicked");
    /* with memory: the same clear is kicked */
    // SAFETY: as above.
    unsafe {
        crate::gl::state::glClear(GL_DEPTH_BUFFER_BIT);
        crate::gl::state::glFlush();
    }
    assert_eq!(rig.gl.error(), GL_NO_ERROR);
    assert_eq!(rig.kicks().len(), 1);
}

#[test]
fn a_sampler_image_address_outside_the_heaps_is_reported() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.textures[0] = GlesTextureBind {
        unit: 0,
        base_gpu: 0x1000,
        width: 4,
        height: 4,
        stride_texels: 4,
        mip_levels: 1,
        ..GlesTextureBind::ZERO
    };
    d.num_textures = 1;

    assert_eq!(draw(&d), 0);
    assert!(rig.logs().iter().any(|l| l.contains("image address 0x1000 is not a heap address")), "{:?}", rig.logs());
}

#[test]
fn the_overhead_counters_report_the_drivers_own_quads() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.do_clear = false; /* the frame opens with a colour-load quad */

    let before = overhead();
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let after = overhead();
    assert_eq!((after.0 - before.0, after.1 - before.1), (1, 0));
}

/// `pvr_backend_overhead` as a pair. The counters are totals that survive a
/// backend init, so a test compares two readings.
fn overhead() -> (u32, u32) {
    let (mut quads, mut splits) = (0u32, 0u32);

    // SAFETY: both pointers are valid for a write.
    unsafe { pvr_backend_overhead(&mut quads, &mut splits) };
    (quads, splits)
}

/* ---- glClear the background object cannot carry out -------------------------- */

#[test]
fn a_clear_quad_is_an_object_of_the_frame_and_not_its_background() {
    /* A scissored clear cannot be the ISP background object (that covers the
     * whole target and has no scissor), so the GL core sends it as
     * GlesDraw::clear_quad and the backend draws a full-screen quad with the
     * clear's scissor instead. It joins the frame: it must not clear the
     * frame's background and must not force a flush. */
    let rig = Rig::new();
    let mut first = rig.draw(3);
    first.do_clear = false;
    assert_eq!(draw(&first), 0, "a plain draw opens the frame");
    assert!(!rig.b().frame_cleared);

    let mut cq = rig.draw(0);
    cq.clear_quad = true;
    cq.do_clear = true;
    cq.clear_rgba = [1.0, 0.0, 0.0, 1.0];
    cq.scissor = true;
    cq.scissor_rect = [4, 5, 6, 7];
    assert_eq!(draw(&cq), 0);

    assert_eq!(rig.kicks().len(), 0, "the clear quad did not end the frame");
    assert!(!rig.b().frame_cleared, "and did not become the frame's background clear");
    /* the frame's colour-load quad (6), the draw (3), then the clear quad (6) */
    let counts: Vec<u32> = rig.index_lists().iter().map(|l| l.2).collect();
    assert_eq!(counts, [6, 3, 6]);
    /* and only the clear quad carries a scissor box */
    let sc: Vec<u32> = rig.ppp().iter().map(|s| s.scissor_enable as u32).collect();
    assert_eq!(sc, [0, 0, 1], "the load quad and the draw have none, the clear quad does");
}

#[test]
fn a_masked_scissored_clear_draws_the_masked_program_in_the_clears_box() {
    /* dEQP color_clear masked_scissored_rgb/rgba: the masked program, and
     * the scissor box the clear was issued with (the GL core puts it in the
     * draw's scissor fields for a clear quad, draw.rs plan_clear), not the
     * whole target. */
    let rig = Rig::new();
    pvr_debug_set_masked_clear(1);
    let mut cq = rig.draw(0);
    cq.clear_quad = true;
    cq.do_clear = true;
    cq.clear_rgba = [1.0, 0.5, 0.25, 1.0];
    cq.clear_color_mask = 0b0011;
    cq.scissor = true;
    cq.scissor_rect = [4, 5, 6, 7];
    assert_eq!(draw(&cq), 0);
    let fl = rig.fbload();
    let i = clear_masked_index(0, 0b0011).expect("a partial mask");
    assert_eq!(fl.load_draw.fs, &fl.clear_mask_sh[i] as *const ShcShader, "the program for R and G");
    assert!(fl.load_draw.scissor, "the quad is scissored");
    assert_eq!(fl.load_draw.scissor_rect, [4, 5, 6, 7], "to the clear's box");
    let sc: Vec<u32> = rig.ppp().iter().map(|s| s.scissor_enable as u32).collect();
    assert_eq!(sc.last(), Some(&1), "and the PPP state it records enables the scissor: {sc:?}");
}

#[test]
fn a_clear_quad_that_opens_a_frame_leaves_the_background_clear_alone() {
    let rig = Rig::new();
    let mut cq = rig.draw(0);
    cq.clear_quad = true;
    cq.do_clear = true;
    cq.clear_rgba = [0.0, 1.0, 0.0, 1.0];
    cq.scissor = true;
    cq.scissor_rect = [1, 1, 2, 2];
    assert_eq!(draw(&cq), 0);

    let b = rig.b();
    assert!(!b.frame_cleared, "the frame starts from memory: the quad writes the pixels");
    assert_eq!(b.clear_depth, 1.0, "no background depth clear either");
    assert_eq!(b.clear_stencil, 0);
    /* the colour-load quad opens the frame, then the clear quad */
    let counts: Vec<u32> = rig.index_lists().iter().map(|l| l.2).collect();
    assert_eq!(counts, [6, 6]);
    assert_eq!(b.frame_quads, 1, "only the load quad counts as the frame's own");
}

/* ---- clipping against the near and far planes -------------------------------- */

#[test]
fn near_far_clipping_is_on_by_default() {
    /* GL ES 2.0 2.11 clips against all six planes and ES 2.0 has no depth
     * clamp, so an application that asks for nothing must get near and far
     * clipping. It was off for a while because FRONT_REAR appeared to
     * over-clip: dEQP's clip_z cases rendered a sliver of the reference, the
     * shape you get when the hardware clips 0 <= z <= w and GL wants
     * -w <= z <= w. The volume turned out to be chosen by CR_PPP_CTRL bit 0,
     * which we had inherited clear from Mesa (a Vulkan driver, and Vulkan
     * clips 0 <= z <= w); with it set, measured on silicon by z_probe, the TA
     * clips GL's volume and FRONT_REAR is simply correct. */
    let rig = Rig::new();
    let d = rig.draw(3);

    assert_eq!(draw(&d), 0);
    let ppp = rig.ppp();
    assert!(!ppp.is_empty(), "the draw emitted PPP state");
    for (i, st) in ppp.iter().enumerate() {
        if st.pretransform != 0 {
            continue; /* the rig's screen-space template clips nothing */
        }
        assert_eq!(st.clip_mode, PVR_TA_CLIP_MODE_FRONT_REAR, "PPP block {i}");
    }
}

#[test]
fn the_clip_space_ppp_ctrl_selects_gls_clip_volume() {
    /* CR_PPP_CTRL bit 0 ("opengl") is what makes the TA clip -w <= z <= w
     * rather than Vulkan's 0 <= z <= w, and bit 9 (fixed_point_format) is
     * what makes the PSG read our VS position format. Both have to be set:
     * bit 9 alone was Mesa's 0x202, which is where the wrong volume came
     * from, and bit 0 alone is the DDK's 0x3, which goes with ITS coordinate
     * format and would make the PSG misread ours. */
    let rig = Rig::new();
    let d = rig.draw(3);

    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0); /* draws are recorded; the flush kicks */
    let kicks = rig.kicks();
    assert!(!kicks.is_empty(), "the flush submitted a kick");
    for (i, (k, _)) in kicks.iter().enumerate() {
        assert_eq!(k.geom.ppp_ctrl & 1, 1, "kick {i}: opengl (bit 0), GL's clip volume");
        assert_eq!(k.geom.ppp_ctrl & (1 << 9), 1 << 9, "kick {i}: fixed_point_format (bit 9)");
    }
}

#[test]
fn the_knob_selects_each_clip_mode() {
    /* z_probe and gl_test case 62 pass these values, so 1 and 2 have to keep
     * meaning what they meant when those were measured. */
    let rig = Rig::new();
    let d = rig.draw(3);

    /* the last PPP block of the kick that is not the pretransform's */
    let clip_mode = || rig.ppp().iter().filter(|st| st.pretransform == 0).map(|st| st.clip_mode).next_back();

    pvr_backend_set_option(PVR_BACKEND_OPT_DEPTH_CLIP, PVR_DEPTH_CLIP_FRONT_REAR);
    assert_eq!(draw(&d), 0);
    assert_eq!(clip_mode(), Some(PVR_TA_CLIP_MODE_FRONT_REAR), "1: GL's near and far planes");

    pvr_backend_set_option(PVR_BACKEND_OPT_DEPTH_CLIP, PVR_DEPTH_CLIP_FRONT_REAR_DEPTH);
    assert_eq!(draw(&d), 0);
    assert_eq!(clip_mode(), Some(PVR_TA_CLIP_MODE_FRONT_REAR_DEPTH), "2: clipping against depth");

    /* off is explicit now that the conformant setting is the default */
    pvr_backend_set_option(PVR_BACKEND_OPT_DEPTH_CLIP, PVR_DEPTH_CLIP_OFF);
    assert_eq!(draw(&d), 0);
    assert_eq!(clip_mode(), Some(PVR_TA_CLIP_MODE_NONE), "3: off, for bring-up");

    /* put the default back for the tests that follow -- and check it is the
     * conformant one, which is the behaviour change this commit is about */
    pvr_backend_set_option(PVR_BACKEND_OPT_DEPTH_CLIP, 0);
    assert_eq!(draw(&d), 0);
    assert_eq!(clip_mode(), Some(PVR_TA_CLIP_MODE_FRONT_REAR), "0: the default, on");
}

#[test]
fn a_draw_is_clipped_to_its_viewport_even_with_the_scissor_test_off() {
    /* GL clips a primitive at the view volume, and the viewport transform
     * maps that volume onto the viewport rectangle -- so nothing GL draws
     * lands outside the viewport. The hardware clipper works to a guard band
     * rather than exactly at the volume, which is a performance feature and
     * fine, but then something has to bound rasterisation to the viewport.
     * Only the scissor path did, so with GL_SCISSOR_TEST off a primitive with
     * a vertex far outside was rasterised past the viewport edge -- a border
     * strip on exactly the side the vertex went out.
     *
     * Clipping to the viewport can never drop a fragment GL asks for, so this
     * holds whatever the guard band turns out to be. */
    let rig = Rig::new();
    let mut d = rig.draw(3);

    d.viewport = [8, 12, 20, 24]; /* a sub-rectangle of the 64x64 target */
    assert!(!d.scissor, "the scissor test is off");
    assert_eq!(draw(&d), 0);

    let st = rig.ppp().into_iter().rfind(|s| s.pretransform == 0).expect("a PPP block");
    assert_eq!(st.clip_enable, 1, "the viewport bounds rasterisation");
    /* the region clip is in REGION_CLIP_PIXELS tiles, top-down; the draw is
     * not flipped, so GL y is the top-down y */
    assert_eq!(st.clip_left, 8 / REGION_CLIP_PIXELS);
    assert_eq!(st.clip_right, (8 + 20 - 1) / REGION_CLIP_PIXELS);
    assert_eq!(st.clip_top, 12 / REGION_CLIP_PIXELS);
    assert_eq!(st.clip_bottom, (12 + 24 - 1) / REGION_CLIP_PIXELS);
}

#[test]
fn no_draw_reaches_the_ppp_with_an_inverted_region_clip() {
    /* A box wholly outside the target used to come out inverted (top below
     * bottom after the y flip) and went to TA_REGION_CLIP as such; the
     * firmware then never started the kick's geometry phase and the GPU
     * wedged (AM62P 2026-09-24: glScissor(0, -200, 100, 50) + glClear on a
     * 256x256 target, and dEQP color_clear.complex_rgb on its first kick).
     * The rule, for every box: a region clip that is emitted is not inverted. */
    let rig = Rig::new();
    let vals = [-300, -64, -40, -1, 0, 1, 20, 63, 64, 100];
    let sizes = [0, 1, 8, 30, 64, 200];
    let mut emitted = 0;
    for flip in [false, true] {
        for &x in &vals {
            for &y in &vals {
                for &w in &sizes {
                    for &h in &sizes {
                        rig.reset_records();
                        let mut d = rig.draw(3);
                        d.flip_y = flip;
                        d.scissor = true;
                        d.scissor_rect = [x, y, w, h];
                        assert_eq!(draw(&d), 0, "box {x},{y} {w}x{h} flip {flip}");
                        for st in rig.ppp().iter().filter(|s| s.pretransform == 0 && s.clip_enable != 0) {
                            emitted += 1;
                            assert!(
                                st.clip_left <= st.clip_right && st.clip_top <= st.clip_bottom,
                                "box {x},{y} {w}x{h} flip {flip}: region clip l{} r{} t{} b{}",
                                st.clip_left,
                                st.clip_right,
                                st.clip_top,
                                st.clip_bottom
                            );
                        }
                    }
                }
            }
        }
    }
    /* the sweep reached the clip path, not only the early return */
    assert!(emitted > 1000, "only {emitted} clipped blocks checked");
}

#[test]
fn a_draw_whose_box_misses_the_target_is_recorded_as_nothing() {
    let rig = Rig::new();
    /* wholly above (after the flip), below, left and right of the 64x64 target */
    for (rect, flip) in
        [([0, -200, 32, 50], true), ([0, 100, 32, 50], false), ([-200, 0, 50, 32], false), ([100, 0, 50, 32], true)]
    {
        rig.reset_records();
        let mut d = rig.draw(3);
        d.do_clear = false;
        d.flip_y = flip;
        d.scissor = true;
        d.scissor_rect = rect;
        let before = rig.b().draws_empty_box;
        assert_eq!(draw(&d), 0, "{rect:?}");
        assert_eq!(rig.b().draws_empty_box, before + 1, "{rect:?}: recorded as nothing");
        /* the frame's own full-target load quad may be there; the draw is not */
        assert!(rig.ppp().iter().all(|s| s.clip_enable == 0), "{rect:?}: no clipped block");
    }
    /* and one that overlaps by a pixel is drawn */
    rig.reset_records();
    let mut d = rig.draw(3);
    d.do_clear = false;
    d.scissor = true;
    d.scissor_rect = [63, 63, 10, 10];
    let before = rig.b().draws_empty_box;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.b().draws_empty_box, before, "the overlapping box is drawn");
    assert!(rig.ppp().iter().any(|s| s.clip_enable != 0), "and clipped to its pixel");
}

#[test]
fn the_scissor_narrows_the_viewport_rather_than_replacing_it() {
    /* With both, the clip is their intersection: GL applies the scissor on
     * top of the viewport, it does not widen it. */
    let rig = Rig::new();
    let mut d = rig.draw(3);

    d.viewport = [10, 10, 40, 40]; /* 10..50 */
    d.scissor = true;
    d.scissor_rect = [0, 20, 64, 8]; /* x covers everything, y 20..28 */
    assert_eq!(draw(&d), 0);

    let st = rig.ppp().into_iter().rfind(|s| s.pretransform == 0).expect("a PPP block");
    assert_eq!(st.clip_enable, 1);
    assert_eq!(st.clip_left, 10 / REGION_CLIP_PIXELS, "x from the viewport, which is narrower");
    assert_eq!(st.clip_right, (10 + 40 - 1) / REGION_CLIP_PIXELS);
    assert_eq!(st.clip_top, 20 / REGION_CLIP_PIXELS, "y from the scissor, which is narrower");
    assert_eq!(st.clip_bottom, (20 + 8 - 1) / REGION_CLIP_PIXELS);
}

#[test]
fn a_full_target_viewport_still_emits_no_region_clip() {
    /* The common case, and the one every sample and gl_test case takes: a
     * viewport covering the whole target with the scissor off. Nothing can be
     * drawn outside the target, so there is nothing to clip away and the
     * block is the one it always was. This is what keeps the viewport clip
     * from churning every draw's command stream. */
    let rig = Rig::new();
    let d = rig.draw(3); /* the rig's viewport is the whole 64x64 target */

    assert_eq!(d.viewport, [0, 0, 64, 64]);
    assert!(!d.scissor);
    assert_eq!(draw(&d), 0);

    let st = rig.ppp().into_iter().rfind(|s| s.pretransform == 0).expect("a PPP block");
    assert_eq!(st.clip_enable, 0, "nothing to clip");
    assert_eq!(st.scissor_enable, 0, "and no ISP scissor entry spent on it");
}

/// The PowerVR clear quad carries the SURFACE as its viewport, never the
/// application's.
///
/// The same defect class the GC7000 backend had (a clear is a full-screen quad
/// in clip space, and the viewport transform maps clip space onto the viewport
/// rectangle, so a clear put through the application's viewport clears only
/// that rectangle). This backend is immune by construction and this test is
/// what keeps it so: `record_clear_quad` builds a FRESH `GlesDraw::ZERO` and
/// sets `viewport[2..4]` to the target's size, rather than inheriting the
/// application's draw state the way the GC7000 path did.
///
/// That structural difference is the whole reason one backend had the bug and
/// the other did not, and it is the thing a later refactor would undo without
/// noticing.
#[test]
fn a_clear_quad_spans_the_surface_and_not_the_applications_viewport() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.viewport = [16, 8, 32, 16];
    d.do_clear = true;
    /* the quad path: a clear the background object cannot carry out */
    d.clear_quad = true;
    d.scissor = true;
    d.scissor_rect = [4, 4, 8, 8];

    assert_eq!(draw(&d), 0);

    // SAFETY: serialised by the rig; no entry point is running.
    let ld = unsafe { &PVR.get().fbload.load_draw };
    assert_eq!(ld.viewport, [0, 0, 64, 64], "the clear spans the target, not the application's viewport");
    /* and the scissor DOES reach it: ES 2.0 3.7.3 lists the scissor test as
     * affecting a clear, which is the half that must keep working. */
    assert!(ld.scissor, "a scissored clear stays scissored");
    assert_eq!(ld.scissor_rect, [4, 4, 8, 8]);
}

#[test]
fn the_pbe_twiddle_probe_sets_the_memlayout_and_log2_sizes_and_is_off_by_default() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.target_w = 16;
    d.target_h = 8;
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    pvr_backend_set_option(PVR_BACKEND_OPT_PBE_TWIDDLE, 1);
    let rc = (draw(&d), pvr_backend_flush());
    pvr_backend_set_option(PVR_BACKEND_OPT_PBE_TWIDDLE, 0);
    assert_eq!(rc, (0, 0));
    let kicks = rig.kicks();
    let (lin, tw) = (kicks[0].0, kicks[1].0);
    assert_eq!((lin.frag.pbe_word[0][0] >> 32) & 3, 0, "LINEAR by default");
    assert_eq!(lin.frag.pbe_word[0][1] >> 60, 0);
    assert_eq!((tw.frag.pbe_word[0][0] >> 32) & 3, 1, "TWIDDLE_2D");
    assert_eq!(tw.frag.pbe_word[0][1] >> 60, 4, "size_x = log2 16");
    assert_eq!((tw.frag.pbe_word[0][1] >> 28) & 0xf, 3, "size_y = log2 8");
    assert_eq!(tw.frag.pbe_word[0][1] & 0x3fff, 7, "maxclip_y unchanged");
    assert_eq!((tw.frag.pbe_word[0][1] >> 32) & 0x3fff, 15, "maxclip_x unchanged");
}

/// [`pvr_mrt_clear`]: a clear of several 8-bit attachments uploads the
/// packed colour once per attachment, kicks the background object with the
/// program of as many output dwords (two: the RGBA16F one, which is the same
/// two moves) and starts every output register from it (the USC clear
/// registers, which a tile with any object starts from); off, or with a
/// float attachment among them, it stays the one-dword program with the
/// colour in dword 0 and register 0 alone.
#[test]
fn a_clear_of_several_8bit_attachments_writes_every_output_with_the_switch() {
    let rig = Rig::new();
    // SAFETY: a plain read of an int only the setter writes; the rig serialises.
    let default = unsafe { core::ptr::addr_of!(pvr_mrt_clear).read() };
    assert_eq!(default, 1, "on by default (106 at cece5c23)");
    let c = [1.0f32, 0.5, 0.25, 1.0];
    let packed = pack_rgba8(&c);
    /* the kick program, whether it is the frame's own one-dword one (a
     * frame set's, so read for the set the frame used), and the colour */
    let frame = |n: u64, float_last: bool, on: bool| -> (u32, Vec<u32>) {
        pvr_set_mrt_clear(u32::from(on));
        let mut d = rig.draw(3);
        d.clear_rgba = c;
        d.num_targets = n as u8;
        for i in 0..n {
            d.targets[i as usize] = GlesDrawTarget {
                gpu: HEAP_GENERAL_BASE + 0x2000_0000 + i * 0x10_0000,
                w: 64,
                h: 64,
                format: if float_last && i + 1 == n { GLES_SURF_RGBA16F } else { d.target_format },
            };
        }
        d.target_gpu = d.targets[0].gpu;
        assert_eq!(draw(&d), 0);
        let (bg, own) = (rig.b().bg_color.gpu, rig.b().bg_kick_off);
        assert_eq!(pvr_backend_flush(), 0);
        let (k, _) = rig.kicks().last().cloned().expect("a kick");
        let col: Vec<u32> = (0..4).map(|i| rig.gpu_u32(bg + 4 * i)).collect();
        /* the tile-start registers hold what the background program uploads:
         * a tile with any object starts from them instead */
        assert_eq!(k.frag.usc_clear_register[..], col[..], "the USC clear registers are the uploaded colours");
        let kick = (k.frag.pds_bgnd[0] & 0xffff_ffff) as u32;
        (if kick == own { ONE } else { kick }, col)
    };
    const ONE: u32 = u32::MAX;
    let b = rig.b();
    let (one, two, three) = (ONE, b.bg_kick2_off, b.bg_kick3_off);
    assert!(two != three && b.bg_kick_off != two && b.bg_kick_off != three, "three distinct programs");
    assert_eq!(frame(3, false, true), (three, vec![packed, packed, packed, 0]), "three 8-bit attachments, on");
    assert_eq!(frame(2, false, true), (two, vec![packed, packed, 0, 0]), "two, on: the two-dword program");
    assert_eq!(frame(3, false, false).0, one, "off: the one-dword program");
    assert_eq!(frame(3, false, false).1[0], packed, "off: the colour in dword 0");
    assert_eq!(frame(3, true, true).0, one, "a float attachment among them: as off");
    assert_eq!(frame(1, false, true), (one, vec![packed, 0, 0, 0]), "one attachment: as before");
    pvr_set_mrt_clear(PVR_MRT_CLEAR_DEFAULT);
}

/// A window that is a view into a wider scan-out buffer (an 800x600 window
/// in a 1920-pixel framebuffer, `GlesDraw::target_pitch`): its pitch, not its
/// width, goes into the PBE's line stride and into the load quad's texture
/// stride, decoded back from the packed words at the bit positions of the
/// generated headers (pbestate.h: linestride is word 0's dw[1] bits 2..17,
/// pixel pairs minus one; texstate.h: STRIDE_IMAGE_WORD1's stride is dw[0]
/// bits 0..14, texels minus one). A packed target keeps tex_pitch(w), and a
/// pitch change ends the frame.
#[test]
fn a_view_window_puts_its_pitch_into_the_pbe_line_stride_and_the_load() {
    let rig = Rig::new();
    let mut d = rig.draw(3);
    d.target_w = 800;
    d.target_h = 600;
    d.target_pitch = 1920;
    d.do_clear = false; /* the frame loads the colour: the load quad is built */
    assert_eq!(draw(&d), 0);
    let ld = rig.fbload().load_draw;
    assert_eq!((ld.target_pitch, ld.textures[0].stride_texels), (1920, 1920));
    let tw1 = tex_stride_image_word1(ld.textures[0].stride_texels, 1, ld.textures[0].base_gpu);
    assert_eq!(tw1[0] & 0x7fff, 1920 - 1, "the load's texture stride, decoded");

    /* the same target packed: another frame (the pitch changed) */
    let mut packed = d;
    packed.target_pitch = 0;
    assert_eq!(draw(&packed), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let kicks = rig.kicks();
    assert_eq!(kicks.len(), 2, "a pitch change ends the frame");
    let linestride = |k: &PvrRenderKick| (k.frag.pbe_word[0][0] >> 34) & 0xffff;
    assert_eq!(linestride(&kicks[0].0), 1920 / 2 - 1, "the view's pitch in pixel pairs minus one");
    assert_eq!(linestride(&kicks[1].0), 800 / 2 - 1, "the packed pitch: tex_pitch(800)");
    /* an odd width packs at tex_pitch, as before */
    assert_eq!((pbe_reg_words(GLES_SURF_RGBA8888, false, 7, 4, 8, false, false)[0] >> 34) & 0xffff, 3);
}

/* ---- a framebuffer with no colour attachment (DRAWS_WITHOUT_COLOUR) -------- */

/// `CR_ISP_ZLSCTL.zonlyrender`: bit 0 (Mesa 26.2.1 csbgen/rogue/cr.xml,
/// ISP_ZLSCTL). Spelled out here, not taken from the backend's packer, so the
/// test does not check the packer against itself.
const ZLSCTL_ZONLYRENDER: u64 = 1 << 0;
/// `PBESTATE_STATE_WORD1.emptytile`: bit 6 (Mesa 26.2.1
/// csbgen/rogue/pbestate.xml, STATE_WORD1).
const PBE_WORD1_EMPTYTILE: u32 = 1 << 6;

/// glmark2's shadow pass as the GL core sends it: no colour target
/// (`target_gpu` 0), the depth texture's size and id, no colour clear, no
/// colour written.
fn z_only_draw(rig: &Rig) -> GlesDraw {
    let mut d = fbo_draw(rig, 0);
    d.target_gpu = 0;
    d.depth_id = 0x4000_0065; /* a texture's id form */
    d.do_clear = false;
    d.do_clear_depth = true;
    d.color_write = false;
    d.color_mask = 0;
    d
}

/// A draw into a depth-only framebuffer is a z-only render, as Mesa's pvr
/// renders a pass with no colour attachment: one end-of-tile emit whose
/// state is word 0 zero and word 1 `emptytile` only
/// (pvr_arch_cmd_buffer.c:1508-1515), the PBE register words zero
/// (pvr_arch_job_common.c:102-111), `zonlyrender` in CR_ISP_ZLSCTL
/// (pvr_arch_job_render.c:1005), and the depth stored to the object's own
/// buffer as for any frame. The frame's first quad initialises depth and
/// samples nothing: there is no colour to load.
#[test]
fn a_depth_only_framebuffer_is_a_z_only_render() {
    let rig = Rig::new();
    let d = z_only_draw(&rig);
    assert_eq!(draw(&d), 0, "a draw with no colour target and a depth attachment: {:?}", rig.logs());
    let fl = rig.fbload();
    assert_eq!(fl.load_draw.num_textures, 0, "the frame's quad samples no colour target");
    assert!(!fl.load_draw.color_write, "and writes no colour");
    assert!(fl.load_draw.depth_write, "it initialises the cleared depth");
    assert_eq!(pvr_backend_flush(), 0);

    let k = rig.kicks();
    assert_eq!(k.len(), 1);
    let f = &k[0].0.frag;
    assert_eq!(rig.eot_emits().last().map(|e| e[..2].to_vec()), Some(vec![0, PBE_WORD1_EMPTYTILE]));
    assert_eq!(f.num_rt, 1, "one emit");
    assert_eq!(f.pbe_word[0], [0, 0], "no PBE register words");
    assert_ne!(f.isp_zlsctl & ZLSCTL_ZONLYRENDER, 0, "zonlyrender");
    assert_ne!(f.isp_zlsctl & ZLSCTL_ZSTOREEN, 0, "the depth is stored");
    let buf = ds_of(&rig, d.depth_id).expect("the depth texture's buffer");
    assert_eq!(f.isp_zload_store_base & !0xf, buf.mem.gpu & !0xf, "stored to the depth texture's own buffer");
    assert_eq!((k[0].0.width, k[0].0.height), (64, 64), "the depth attachment's size");
}

/// The known negative of the test above: a framebuffer with a colour target
/// keeps its colour emit and register words, and no `zonlyrender`.
#[test]
fn a_framebuffer_with_colour_is_not_a_z_only_render() {
    let rig = Rig::new();
    let d = fbo_draw(&rig, 0);
    assert_eq!(draw(&d), 0);
    assert_eq!(pvr_backend_flush(), 0);
    let f = rig.kicks()[0].0.frag;
    let e = rig.eot_emits().last().cloned().expect("an end-of-tile program");
    assert_ne!(e[..2], [0, PBE_WORD1_EMPTYTILE], "the colour attachment's emit");
    assert_eq!(e[1] & PBE_WORD1_EMPTYTILE, 0);
    assert_ne!(f.pbe_word[0], [0, 0]);
    assert_eq!(f.isp_zlsctl & ZLSCTL_ZONLYRENDER, 0);
}

/// No colour target and no depth or stencil either: nothing the draw could
/// write, refused by name of its arguments (-EINVAL), never a kick.
#[test]
fn a_draw_with_no_target_at_all_is_refused() {
    let rig = Rig::new();
    let mut d = z_only_draw(&rig);
    (d.depth_id, d.stencil_id, d.ds_persist, d.ds_id) = (0, 0, false, 0);
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(pvr_backend_flush(), 0);
    assert!(rig.kicks().is_empty());
}

/// The GL core asks `backend::DEPTH_ONLY_TARGETS`, which in a PowerVR build
/// is this backend's `DRAWS_WITHOUT_COLOUR`: with it false, glmark2's shadow
/// framebuffer is GL_FRAMEBUFFER_UNSUPPORTED again ("Set up failed" on the
/// AM62P, round 2 of 2026-10-04) whatever the kick above can do.
/// Checked when the tests compile.
const _: () = assert!(DRAWS_WITHOUT_COLOUR && crate::backend::DEPTH_ONLY_TARGETS);
