// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Three mip levels a render on the PowerVR backend
//! (`pvr_backend_set_mip_three_levels`, off by default; candidate 2 of
//! docs/notes/am62p-slow-terrain.md, designed in
//! docs/notes/pvr-mip-three-levels.md), with the knob on:
//!
//! 1. glmark2 terrain's kicks a frame, on the platform of
//!    tests/pvr_terrain_kicks.rs (copied: the REAL backend, host memory,
//!    every submit completes at once): 4 renders a chain instead of 9 or 8.
//!    That test pins the knob-off frame, 51 kicks of which 43 are mip
//!    levels and 8 passes. The design note's 52 -> 29 counted the window's
//!    dead clear kick, which fix 5 took away since (51 = 52 - 1). With the
//!    knob: 43 - 9 - 8 - 8 - 9 - 9 + 5 x 4 = 20 mip renders, and 8 passes,
//!    28 kicks a frame (= 29 - 1).
//! 2. What each render stores: every level's PBE register words (its pitch,
//!    linestride, and its clip, maxclip) on the knob-on render's attachment
//!    are those of the knob-off level kick for that level, at terrain's
//!    sizes. That pins the secondary targets' storage pitch
//!    (`Backend::mip_pitch`): level 2 of 800x600 is 200 texels wide in a
//!    256-texel row.
//! 3. The level contents. The host's stand-in GPU draws nothing, so the
//!    programs run in the reference IR evaluator (src/shaderc/online/eval.rs)
//!    at every pixel, the look-ups answered by a model of the TPU's bilinear
//!    sample (ES 3.0.6 3.8.10's four-weight sum over value / 255 in f32) and
//!    the outputs packed to 8 bits by round-half-up, as
//!    docs/notes/pvr-mip-tail-e1.md's appendix B did. Knob off, every level
//!    is the model's sample of the level above (mipgen.frag); knob on, level
//!    1 and the first level of each render are that sample of their source
//!    level at the same coordinate bits, and levels l+2 and l+3 are
//!    mipgen3.frag's ALU bilinear over those. Levels 1 and 2 (the only ones
//!    terrain's per-frame chains are read at) must be byte-identical to the
//!    knob-off chain; every level from 3 on is pinned by its FNV-1a 64 hash,
//!    and may differ from the knob-off chain by 1 at most (`MAX_D`; 0 on the
//!    256x256 chain, at most 1 in up to 125 texels a level on 800x600).
//!    The model is not the board's TPU: the board's bytes of levels 3 on are
//!    a board read-back's to tell (docs/notes/pvr-mip-three-levels.md, Open).
//!
//! How the pinned hashes were generated: once, from this commit's code, by
//! running the level test with `PINNED` empty (it then prints the table
//! instead of asserting, `-- --nocapture`) and pasting the output.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mip_three_levels
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
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
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
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
/// ISP_ZLSCTL.zloaden: the kick loads its depth (src/pvr/backend.rs).
const ZLSCTL_ZLOADEN: u64 = 1 << 15;

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

/// One submitted kick, as far as the mip chain is concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KickRec {
    size: (u32, u32),
    num_rt: u32,
    pbe: [[u64; 2]; yakogl::osal::render::PVR_RENDER_EOT_MAX_RT],
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
    /// (width, height) of every kick submitted, in order: index + 1 is its fence.
    sizes: Vec<(u32, u32)>,
    /// What each kick submitted stores: its size, colour attachments and
    /// PBE register words, in order.
    kicks: Vec<KickRec>,
    /// The depth word each kick stored, by fence.
    stored: BTreeMap<i64, u32>,
    /// The words of the depth/stencil buffer each kick that loads depth
    /// loads (from its base to the allocation's end), by fence.
    loads: BTreeMap<i64, Vec<u32>>,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The D24S8 word kick `fence` stores: depth 0.25 + fence/1024, stencil 0.
fn depth_word(fence: i64) -> u32 {
    let d = 0.25 + fence as f64 / 1024.0;
    (d * 16_777_215.0).round() as u32 & 0xff_ffff
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

/// The "GPU": a kick that loads depth reads its depth/stencil buffer
/// (recorded in `loads`), and a kick that stores depth leaves
/// `depth_word(fence)` in every word of it.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.fence += 1;
        let fence = p.fence;
        p.sizes.push((k.width, k.height));
        p.kicks.push(KickRec { size: (k.width, k.height), num_rt: k.frag.num_rt, pbe: k.frag.pbe_word });
        let zs = k.frag.isp_zload_store_base;
        if zs != 0 && k.frag.isp_zlsctl & ZLSCTL_ZLOADEN != 0 {
            if let Some((&base, a)) = p.allocs.range(..=zs).next_back() {
                let off = (zs - base) as usize;
                let first = (a.mem.as_ptr() as *const u8).wrapping_add(off) as *const u32;
                // SAFETY: the words from `zs` to the allocation's end.
                let words = unsafe { core::slice::from_raw_parts(first, a.size.saturating_sub(off) / 4) }.to_vec();
                p.loads.insert(fence, words);
            }
        }
        let stores = zs != 0 && k.frag.isp_zlsctl & ZLSCTL_FORCEZSTORE != 0;
        let Some((&base, a)) = p.allocs.range_mut(..=zs).next_back().filter(|_| stores) else { return fence };
        let off = (zs - base) as usize;
        if off < a.size {
            let w = depth_word(fence);
            let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
            // SAFETY: the words from `zs` to the allocation's end.
            unsafe { core::slice::from_raw_parts_mut(first, (a.size - off) / 4) }.fill(w);
            p.stored.insert(fence, w);
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

/// glmark2's canvas, as the host harness runs terrain.
const CANVAS: (i32, i32) = (800, 600);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAMS: (GLuint, GLuint) = (0, 0);

fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
            PROGRAMS = (program("passthrough.vert", "uniform_color.frag"), program("uv.vert", "texture.frag"));
        }
    });
    with_platform(|p| p.log.clear());
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

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static UV: [f32; 6] = [0.0, 0.0, 2.0, 0.0, 0.0, 2.0];

fn draw(prog: GLuint) {
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

/// One of terrain's render targets: an RGBA8 texture (glmark2's
/// `RenderObject`s: LINEAR_MIPMAP_LINEAR / LINEAR, CLAMP_TO_EDGE) in an FBO,
/// with a 16-bit depth renderbuffer when `depth`.
struct Target {
    tex: GLuint,
    fbo: GLuint,
    size: (i32, i32),
}

fn target(size: (i32, i32), depth: bool) -> Target {
    let (mut tex, mut fbo, mut rb) = (0, 0, 0);
    // SAFETY: room for one name each; a null image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            size.0,
            size.1,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        if depth {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, size.0, size.1);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    Target { tex, fbo, size }
}

/// One pass of terrain's frame, as glmark2's `TextureRenderer` /
/// `RenderObject` run it: bind the FBO, viewport, clear (`clear`: a pass that
/// loads its target, the overlay, does not), draw sampling `src` when given,
/// then `glGenerateMipmap` on the target's texture when `mip`.
fn pass(dst: &Target, src: Option<&Target>, clear: bool, mip: bool) {
    let (_, tex_prog) = unsafe { PROGRAMS };
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, dst.fbo);
        glViewport(0, 0, dst.size.0, dst.size.1);
        if clear {
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, src.map_or(0, |s| s.tex));
    }
    draw(if src.is_some() { tex_prog } else { unsafe { PROGRAMS.0 } });
    if mip {
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, dst.tex);
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindTexture(GL_TEXTURE_2D, 0);
        }
    }
}

struct Terrain {
    height: Target,
    normal: Target,
    terrain: Target,
    bloom_h: Target,
    bloom_v: Target,
    tilt: Target,
}

fn terrain_targets() -> Terrain {
    Terrain {
        height: target((256, 256), false),
        normal: target((256, 256), false),
        terrain: target(CANVAS, true),
        bloom_h: target((256, 256), false),
        bloom_v: target((256, 256), false),
        tilt: target(CANVAS, false),
    }
}

/// Terrain's frame (glmark2 scene-terrain.cpp:332-339 and its renderers):
/// the window's clear (`MainLoop`), height map and normal map (256x256, no
/// depth), terrain (800x600 with depth) and its chain, bloom h and v (256x256)
/// and their chains, the overlay into the terrain texture and its chain
/// again, tilt-shift h (800x600) and its chain, then tilt-shift v into the
/// window and the swap.
fn terrain_frame(t: &Terrain) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    pass(&t.height, None, true, false);
    pass(&t.normal, None, true, false);
    pass(&t.terrain, Some(&t.height), true, true);
    pass(&t.bloom_h, Some(&t.terrain), true, true);
    pass(&t.bloom_v, Some(&t.bloom_h), true, true);
    pass(&t.terrain, Some(&t.bloom_v), false, true);
    pass(&t.tilt, Some(&t.terrain), true, true);
    // the window: tilt-shift v clears it again and draws, the swap submits
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t.tilt.tex);
    }
    draw(unsafe { PROGRAMS.1 });
    // SAFETY: plain GL calls (eglSwapBuffers' submit).
    unsafe {
        glBindTexture(GL_TEXTURE_2D, 0);
        glFlush();
    }
}

/// What a kick is, from its place in the frame and its size: a mip level
/// kick declares the size of level 1 of its chain (400x300 of an 800x600
/// target, 128x128 of a 256x256 one; src/pvr/backend.rs `record_mip_level`),
/// which no pass of terrain has; the frame's first kick is the window's
/// clear (a 800x600 kick followed by the height map's 256x256 pass); every
/// other kick is a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    WindowClear,
    Pass,
    Mip,
}

fn kinds(sizes: &[(u32, u32)]) -> Vec<Kind> {
    sizes
        .iter()
        .enumerate()
        .map(|(i, &s)| match s {
            (400, 300) | (128, 128) => Kind::Mip,
            (800, 600) if i == 0 && sizes.get(1) == Some(&(256, 256)) => Kind::WindowClear,
            _ => Kind::Pass,
        })
        .collect()
}

/* ---- the knob ------------------------------------------------------------------ */

/// `f` with three levels a render on, off again after it (also when `f`
/// panics): the switch is the backend's, the rig's lock keeps every other
/// test of this file out meanwhile.
fn with_three_levels<R>(f: impl FnOnce() -> R) -> R {
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            pvr_backend_set_mip_three_levels(false);
        }
    }
    let _off = Off;
    pvr_backend_set_mip_three_levels(true);
    f()
}

/// The colour attachments a kick stores (`num_rt` 0 is one).
fn rts(k: &KickRec) -> u32 {
    k.num_rt.max(1)
}

/// The levels each render of a chain of `n` levels (level 0 included)
/// draws with the knob on: level 1 alone, then up to three from the level
/// above them (src/pvr/backend.rs `mip_chain_gpu`).
fn groups(n: usize) -> Vec<Vec<usize>> {
    let mut g = vec![vec![1]];
    let mut l = 2;
    while l < n {
        let m = (n - l).min(3);
        g.push((l..l + m).collect());
        l += m;
    }
    g
}

/// glmark2 terrain's steady frame with three levels a render: 28 kicks, 20
/// of them mip renders (4 a chain: 1; 2-4; 5-7; 8-9 of an 800x600 chain,
/// 1; 2-4; 5-7; 8 of a 256x256 one) and the same 8 passes. The knob-off
/// frame is tests/pvr_terrain_kicks.rs's 51 = 43 + 8. Each render's colour
/// attachments are its levels: 1, 3, 3 and 2 (800x600) or 1 (256x256).
#[test]
fn terrain_frame_with_three_levels_is_28_kicks_20_mip_renders_and_8_passes() {
    let _g = rig();
    let t = terrain_targets();
    terrain_frame(&t); /* the first frame creates the targets' storage and programs */
    let frames: Vec<Vec<KickRec>> = with_three_levels(|| {
        terrain_frame(&t); /* the first chains upload the three-levels program */
        (0..3)
            .map(|_| {
                let from = with_platform(|p| p.kicks.len());
                terrain_frame(&t);
                with_platform(|p| p.kicks[from..].to_vec())
            })
            .collect()
    });
    let big: Vec<u32> = groups(10).iter().map(|g| g.len() as u32).collect();
    let small: Vec<u32> = groups(9).iter().map(|g| g.len() as u32).collect();
    assert_eq!((big.as_slice(), small.as_slice()), ([1, 3, 3, 2].as_slice(), [1, 3, 3, 1].as_slice()));
    for (i, kicks) in frames.iter().enumerate() {
        let sizes: Vec<(u32, u32)> = kicks.iter().map(|k| k.size).collect();
        let k = kinds(&sizes);
        let count = |w: Kind| k.iter().filter(|&&x| x == w).count();
        assert_eq!(count(Kind::Mip), 20, "frame {i}: mip renders (43 with a kick a level); sizes {sizes:?}");
        assert_eq!(count(Kind::Pass), 8, "frame {i}: pass kicks, as with the knob off; sizes {sizes:?}");
        assert_eq!(count(Kind::WindowClear), 0, "frame {i}: the window's dead clear kick (fix 5); sizes {sizes:?}");
        assert_eq!(kicks.len(), 28, "frame {i}: kicks a frame (51 with the knob off); sizes {sizes:?}");
        /* each chain's renders, and the levels each one stores */
        let mut runs: Vec<Vec<u32>> = Vec::new();
        let mut run = Vec::new();
        for (x, kick) in k.iter().zip(kicks) {
            if *x == Kind::Mip {
                run.push(rts(kick));
            } else if !run.is_empty() {
                runs.push(core::mem::take(&mut run));
            }
        }
        if !run.is_empty() {
            runs.push(run);
        }
        assert_eq!(
            runs,
            [big.clone(), small.clone(), small.clone(), big.clone(), big.clone()],
            "frame {i}: the colour attachments of each chain's renders (terrain, bloom h, bloom v, terrain again, \
             tilt-shift h)"
        );
    }
    assert!(frames.iter().all(|f| *f == frames[0]), "every steady frame submits the same kicks in the same order");
}

/// The kicks of one `glGenerateMipmap` on a fresh `size` RGBA8 texture whose
/// level 0 was just rendered: the chain's kicks, each declaring level 1's
/// size (the render of level 0 is the kick before them).
fn chain_kicks(size: (i32, i32), three: bool) -> Vec<KickRec> {
    let tgt = target(size, false);
    pass(&tgt, None, true, false);
    let from = with_platform(|p| p.kicks.len());
    let run = || {
        // SAFETY: plain GL calls on a name this test made.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tgt.tex);
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindTexture(GL_TEXTURE_2D, 0);
            glFlush();
        }
    };
    if three {
        with_three_levels(run);
    } else {
        run();
    }
    let level1 = (size.0 as u32 / 2, size.1 as u32 / 2);
    with_platform(|p| p.kicks[from..].iter().filter(|k| k.size == level1).copied().collect())
}

/// Every level a three-levels render stores goes out with the PBE register
/// words the knob-off chain's kick for that level has: the same row pitch
/// (linestride: the storage pitch, 256 for level 2 of 800x600, not its width
/// 200) and the same clip (maxclip: the level's GL extent), at terrain's two
/// chain sizes.
#[test]
fn each_level_of_a_three_level_render_is_stored_as_its_level_kick_stores_it() {
    let _g = rig();
    for (size, n) in [((800, 600), 10usize), ((256, 256), 9)] {
        let off = chain_kicks(size, false);
        assert_eq!(off.len(), n - 1, "{size:?}: a kick a level with the knob off");
        assert!(off.iter().all(|k| rts(k) == 1), "{size:?}: one attachment a level kick");
        let on = chain_kicks(size, true);
        let g = groups(n);
        assert_eq!(on.len(), g.len(), "{size:?}: renders with the knob on");
        for (r, (kick, levels)) in on.iter().zip(&g).enumerate() {
            assert_eq!(rts(kick) as usize, levels.len(), "{size:?}: render {r}'s attachments are levels {levels:?}");
            for (slot, &l) in levels.iter().enumerate() {
                assert_eq!(
                    kick.pbe[slot],
                    off[l - 1].pbe[0],
                    "{size:?}: level {l}, attachment {slot} of render {r}: not the level kick's pitch and clip"
                );
            }
        }
    }
}

/* ---- the level contents ------------------------------------------------------- */

/* The level contents compile mipgen3.frag, GLSL ES 3.00, in the IR
 * evaluator: a report-es2 build refuses that source (pp.rs out_of_scope), so
 * this part is left out of it. The kicks and the store words above run in
 * every build. */
#[cfg(not(feature = "report-es2"))]
mod levels {
    use super::*;
    use yakogl::pvr::internal::{PVR_INTERNAL_MIPGEN3, PVR_INTERNAL_TABLE};
    use yakogl::shaderc::online::arena::Arena;
    use yakogl::shaderc::online::eval::{self, Fault, Frame, Inputs, Sink, TexLevel, TexLookup, TexelSource};
    use yakogl::shaderc::online::ir::{IoSlot, Module, TexDim};
    use yakogl::shaderc::online::lower::compile_to_ir;
    use yakogl::shaderc::online::sema::Stage;

    const MIPGEN3: &str = include_str!("../tools/shaders/internal/es/mipgen3.frag");

    /// One RGBA8 level, its GL extent, tightly packed.
    #[derive(Clone)]
    struct Level {
        w: u32,
        h: u32,
        px: Vec<[u8; 4]>,
    }

    impl Level {
        fn at(&self, x: i64, y: i64) -> [u8; 4] {
            let x = x.clamp(0, self.w as i64 - 1) as u32;
            let y = y.clamp(0, self.h as i64 - 1) as u32;
            self.px[(y * self.w + x) as usize]
        }
    }

    /// The 8-bit pack: round half up.
    fn q(v: f32) -> u8 {
        (v * 255.0 + 0.5).floor().clamp(0.0, 255.0) as u8
    }

    /// The TPU model's bilinear sample of `l` (one level, CLAMP_TO_EDGE) at the
    /// normalised point `c`: ES 3.0.6 3.8.10's four-weight sum over value / 255.
    fn sample(l: &Level, c: [f32; 2]) -> [f32; 4] {
        let u = c[0] * l.w as f32 - 0.5;
        let v = c[1] * l.h as f32 - 0.5;
        let (i0, j0) = (u.floor(), v.floor());
        let (a, b) = (u - i0, v - j0);
        let (i0, j0) = (i0 as i64, j0 as i64);
        let t = [l.at(i0, j0), l.at(i0 + 1, j0), l.at(i0, j0 + 1), l.at(i0 + 1, j0 + 1)];
        let mut out = [0.0f32; 4];
        for (ch, o) in out.iter_mut().enumerate() {
            let g = |k: usize| t[k][ch] as f32 / 255.0;
            *o = (1.0 - a) * (1.0 - b) * g(0) + a * (1.0 - b) * g(1) + (1.0 - a) * b * g(2) + a * b * g(3);
        }
        out
    }

    /// A level kick (mipgen.frag): `w` x `h` texels, each the sample of `l` at
    /// gl_FragCoord.xy * u_scale, u_scale made as `record_mip_level` makes it.
    fn level_kick(l: &Level, w: u32, h: u32) -> Level {
        let s = [1.0 / w as f32, 1.0 / h as f32];
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                px.push(sample(l, [(x as f32 + 0.5) * s[0], (y as f32 + 0.5) * s[1]]).map(q));
            }
        }
        Level { w, h, px }
    }

    struct Src<'a> {
        l: &'a Level,
    }

    impl TexelSource for Src<'_> {
        fn sample(&mut self, t: &TexLookup) -> Result<[u32; 4], Fault> {
            if t.sampler != 0 || t.dim != TexDim::D2 || t.level != TexLevel::Lod(0.0) {
                return Err(Fault::new(format_args!("a look-up mipgen3.frag does not make: {t:?}")));
            }
            Ok(eval::f32x4(sample(self.l, [f32::from_bits(t.coord[0]), f32::from_bits(t.coord[1])])))
        }
        fn size(&mut self, _: u8, _: TexDim, _: i32) -> Result<[u32; 4], Fault> {
            Ok([self.l.w, self.l.h, 1, 0])
        }
    }

    #[derive(Default)]
    struct Outs {
        o: [Option<[u32; 4]>; 3],
    }

    impl Sink for Outs {
        fn store(&mut self, slot: IoSlot, value: [u32; 4], _: [bool; 4]) {
            if let IoSlot::FragColor(k) = slot {
                if let Some(o) = self.o.get_mut(k as usize) {
                    *o = Some(value);
                }
            }
        }
    }

    /// The sides of a `w` x `h` chain's `n` levels (GL).
    fn sides(w: u32, h: u32, n: usize) -> Vec<(u32, u32)> {
        (0..n).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect()
    }

    /// Level 0: `random` bytes, or a `smooth` picture with a little noise, as a
    /// render makes (docs/notes/pvr-mip-tail-e1.md appendix B's two).
    fn level0(w: u32, h: u32, kind: &str) -> Level {
        let mut s: u32 = 0x1234_5678;
        let mut rnd = || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        let mut px = Vec::with_capacity((w * h) as usize);
        for y in 0..h {
            for x in 0..w {
                let r = rnd().to_le_bytes();
                px.push(if kind == "random" {
                    r
                } else {
                    let g = |a: u32, b: u32, n: u8| (((a * 255) / b.max(1)) as u8).wrapping_add(n % 7);
                    [g(x, w, r[0]), g(y, h, r[1]), g(x + y, w + h, r[2]), 255]
                });
            }
        }
        Level { w, h, px }
    }

    /// mipgen3.frag's render of `dst` (2 or 3 levels) from `src`: every pixel
    /// of `dst[0]`'s extent run by the evaluator with the block
    /// `record_mip_levels` writes, each output kept where its level is.
    fn three_level_render(m: &Module<'_>, src: &Level, dst: &[(u32, u32)]) -> Vec<Level> {
        let mut block = [0u32; 12];
        for (k, &(w, h)) in dst.iter().enumerate() {
            let v = [1.0 / w as f32, 1.0 / h as f32, w as f32, h as f32];
            for c in 0..4 {
                block[4 * k + c] = v[c].to_bits();
            }
        }
        let mut values = vec![[0u32; 4]; m.instrs.len()];
        let mut def = vec![[false; 4]; m.instrs.len()];
        let mut locals = vec![[0u32; 4]; m.locals.len()];
        let mut ldef = vec![[false; 4]; m.locals.len()];
        let mut out: Vec<Level> =
            dst.iter().map(|&(w, h)| Level { w, h, px: vec![[0; 4]; (w * h) as usize] }).collect();
        let mut texels = Src { l: src };
        let (w1, h1) = dst[0];
        for y in 0..h1 {
            for x in 0..w1 {
                let io = [(IoSlot::FragCoord, eval::f32x4([x as f32 + 0.5, y as f32 + 0.5, 0.5, 1.0]))];
                let mut o = Outs::default();
                let frame = Frame { values: &mut values, def: &mut def, locals: &mut locals, ldef: &mut ldef };
                eval::run_in_with_textures(m, &Inputs { io: &io, uniforms: &block }, frame, &mut o, &mut texels)
                    .unwrap_or_else(|f| panic!("pixel ({x}, {y}): {}", f.as_str()));
                for (k, lv) in out.iter_mut().enumerate() {
                    if x < lv.w && y < lv.h {
                        let v = o.o[k].unwrap_or_else(|| panic!("pixel ({x}, {y}): output {k} not written"));
                        lv.px[(y * lv.w + x) as usize] = v.map(|b| q(f32::from_bits(b)));
                    }
                }
            }
        }
        out
    }

    /// FNV-1a 64 of a level's bytes, row by row.
    fn fnv(l: &Level) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for b in l.px.iter().flatten() {
            h ^= *b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    /// (size, level 0's content, level, FNV-1a 64 of the knob-on level's
    /// bytes), levels 3 and on: the ALU levels (3, 4, 6, 7, 9 of 800x600; 3, 4,
    /// 6, 7 of 256x256) and the levels sampled from one (5, 8).
    /// The most a level from 3 on may differ from the knob-off chain in a
    /// channel. Measured at this commit: 1, in levels 3..=9 of 800x600 (up to 125
    /// of level 3's 7500 texels), where 1/W is not exact and the ALU's
    /// `mix` rounds a near tie otherwise than the model's four-weight sum; 0 at
    /// every level of 256x256, whose weights are exactly 1/2.
    const MAX_D: u8 = 1;

    const PINNED: &[((u32, u32), &str, usize, u64)] = &[
        // PINNED-BEGIN
        ((800, 600), "smooth", 3, 0xece446206df20ee4),
        ((800, 600), "smooth", 4, 0x69f64f1984addd68),
        ((800, 600), "smooth", 5, 0xf69c45a2438d03c1),
        ((800, 600), "smooth", 6, 0xce4e80c2d55c3c89),
        ((800, 600), "smooth", 7, 0xb61ca876a6721137),
        ((800, 600), "smooth", 8, 0x9e39aa2fea4e1873),
        ((800, 600), "smooth", 9, 0x462d0ab58659e773),
        ((800, 600), "random", 3, 0xe100f3daa244b1ce),
        ((800, 600), "random", 4, 0x1f1ad6576632884d),
        ((800, 600), "random", 5, 0xebd88f5e67888b64),
        ((800, 600), "random", 6, 0x5f793b8b5453ce74),
        ((800, 600), "random", 7, 0xf9b437c9b33e4645),
        ((800, 600), "random", 8, 0x16333fe9063052ce),
        ((800, 600), "random", 9, 0x2a78b6a985056d13),
        ((256, 256), "smooth", 3, 0x184d3bcc20542f7f),
        ((256, 256), "smooth", 4, 0x48dfa29f93b91d8f),
        ((256, 256), "smooth", 5, 0xd44ba099ca0285e3),
        ((256, 256), "smooth", 6, 0x57476e639bd97a29),
        ((256, 256), "smooth", 7, 0xe2a25b7c0d29e0ed),
        ((256, 256), "smooth", 8, 0x9d8c8bad2b3f7f86),
        ((256, 256), "random", 3, 0x8792bfaf12da810d),
        ((256, 256), "random", 4, 0xd22e048b30cba11e),
        ((256, 256), "random", 5, 0xb8350a066037d9d9),
        ((256, 256), "random", 6, 0x46672176f9ca04ba),
        ((256, 256), "random", 7, 0x60edefb83806be71),
        ((256, 256), "random", 8, 0x540fce9fed04dc64),
        // PINNED-END
    ];

    /// glmark2 terrain's chains with three levels a render, against the
    /// knob-off chain. Levels 1 and 2 are the knob-off chain's byte for byte.
    /// Levels 3 on are pinned, and differ from the knob-off chain by
    /// [`MAX_D`] at most: the first level of a later render (5, 8) is the TPU's
    /// sample, as a level kick's, but of a level the ALU made.
    #[test]
    fn levels_1_and_2_are_the_level_kick_chains_and_the_rest_are_pinned() {
        let rec = &PVR_INTERNAL_TABLE[PVR_INTERNAL_MIPGEN3].shader;
        assert_eq!(rec.num_outputs, 3, "three colour outputs, or the frame logs the missing ones");
        assert_eq!(rec.uniform_dwords, 12, "u_l1, u_l2, u_l3");
        assert_eq!(rec.fragcoord_ytf_off1, 33, "gl_FragCoord.y's pair straight after the block");
        assert!(rec.temps <= 128, "{} temporaries: the program grew", rec.temps);
        let mut buf = vec![0u8; 8 << 20];
        let a = Arena::new(&mut buf);
        let lowered = compile_to_ir(&a, Stage::Fragment, MIPGEN3).unwrap_or_else(|e| panic!("{}: {}", e.line, e.msg));
        let m = &lowered.module;
        for (name, off) in [("u_l1", 0u16), ("u_l2", 4), ("u_l3", 8)] {
            let d = m.uniforms.iter().find(|u| u.name == name).expect("the block's uniform");
            assert_eq!(d.dword_off, off, "{name}: where record_mip_levels writes it");
        }
        let mut printed = Vec::new();
        let mut checked = 0;
        for (w, h, n) in [(800u32, 600u32, 10usize), (256, 256, 9)] {
            for kind in ["smooth", "random"] {
                let sz = sides(w, h, n);
                let mut off = vec![level0(w, h, kind)];
                for l in 1..n {
                    let next = level_kick(&off[l - 1], sz[l].0, sz[l].1);
                    off.push(next);
                }
                let mut on: Vec<Level> = vec![off[0].clone()];
                for g in groups(n) {
                    let src = on[g[0] - 1].clone();
                    if g.len() == 1 {
                        on.push(level_kick(&src, sz[g[0]].0, sz[g[0]].1));
                    } else {
                        let dst: Vec<(u32, u32)> = g.iter().map(|&l| sz[l]).collect();
                        on.extend(three_level_render(m, &src, &dst));
                    }
                }
                assert_eq!(on.len(), n);
                for l in 1..n {
                    let what = format!("{w}x{h} {kind} level {l} ({}x{})", sz[l].0, sz[l].1);
                    let (mut texd, mut maxd) = (0, 0u8);
                    for (a, b) in on[l].px.iter().zip(&off[l].px) {
                        let d = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                        texd += usize::from(d != 0);
                        maxd = maxd.max(d);
                    }
                    println!(
                        "{what}: {texd} of {} texels differ from the level kick chain, max |d| {maxd}",
                        on[l].px.len()
                    );
                    if l <= 2 {
                        assert_eq!(maxd, 0, "{what}: not the level kick chain's bytes");
                        continue;
                    }
                    assert!(maxd <= MAX_D, "{what}: {maxd} from the level kick chain");
                    let hash = fnv(&on[l]);
                    printed.push(format!("    (({w}, {h}), \"{kind}\", {l}, 0x{hash:016x}),"));
                    if !PINNED.is_empty() {
                        let pin = PINNED
                            .iter()
                            .find(|p| p.0 == (w, h) && p.1 == kind && p.2 == l)
                            .unwrap_or_else(|| panic!("{what}: no pinned hash"));
                        assert_eq!(hash, pin.3, "{what}: the level's bytes moved");
                        checked += 1;
                    }
                }
            }
        }
        if PINNED.is_empty() {
            println!("// PINNED-BEGIN\n{}\n// PINNED-END", printed.join("\n"));
        } else {
            assert_eq!(checked, PINNED.len(), "every pinned level checked");
        }
    }
}
