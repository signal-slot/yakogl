// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Three mip levels a render on the PowerVR backend
//! (`pvr_backend_set_mip_three_levels`, docs/notes/pvr-mip-three-levels.md)
//! on degenerate chain shapes, with the knob on and off: chains where one
//! axis clamps at 1 inside a three-level render, and chains whose length is
//! not 1 + a multiple of 3. tests/pvr_mip_three_levels.rs covers terrain's
//! 800x600 and 256x256 (and the note's 64x64 to 128x128); this file takes
//! 1x1, 2x2, 4x4, 1x64, 64x1, 2x128, 128x2, 8x1, 3x5 (NPOT: ES 2.0 with
//! GL_OES_texture_npot, which every build has, makes its chain) and 16x16.
//!
//! On the platform of that file (the REAL backend, host memory, every
//! submit completes at once, the command generators stood in for), for
//! each shape:
//!
//! 1. the kicks of one `glGenerateMipmap` with the knob off and on (each
//!    level render is one kick), pinned in [`SHAPES`], and the chains the
//!    backend made and left to the CPU (`pvr_backend_mip_chains`);
//! 2. which levels each render stores: the address of every colour
//!    attachment's emit (PBESTATE_STATE_WORD0/1, recorded by the stand-in
//!    end-of-tile generator) is that of the knob-off kick of one level, and
//!    the levels are 1 alone, then up to three from the level above;
//! 3. that each stored level's PBE register words (linestride: the pitch,
//!    maxclip: the size) are the knob-off kick's for that level;
//! 4. with `shaderc-online`, the level contents: the reference IR evaluator
//!    runs mipgen3.frag over a model of the TPU's bilinear sample, as
//!    tests/pvr_mip_three_levels.rs does, for the renders the backend
//!    submitted, and every level is within 1 LSB of the knob-off chain's.
//!
//! A chain the backend leaves to the CPU (a level one texel wide above a
//! taller tail: the PBE writes pixel pairs) makes no kick with the knob on
//! or off, so the GL core's CPU filter makes both chains alike.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_mip_shapes
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mip_shapes
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend::GLES_SURF_RGBA8888;
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
    /// The emit pair (PBESTATE_STATE_WORD0/1) of each attachment, as the
    /// frame's end-of-tile program was generated with them.
    emit: [[u32; 2]; yakogl::osal::render::PVR_RENDER_EOT_MAX_RT],
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
    /// The emit pairs the last end-of-tile program was generated with: the
    /// next kick's.
    emit: [[u32; 2]; yakogl::osal::render::PVR_RENDER_EOT_MAX_RT],
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
        let emit = core::mem::take(&mut p.emit);
        p.kicks.push(KickRec { size: (k.width, k.height), num_rt: k.frag.num_rt, pbe: k.frag.pbe_word, emit });
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
    num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    let mut emit = [[0u32; 2]; yakogl::osal::render::PVR_RENDER_EOT_MAX_RT];
    for (i, e) in emit.iter_mut().enumerate().take(num_rt.max(1) as usize) {
        // SAFETY: the backend's emit words hold a pair for each of `num_rt`.
        *e = unsafe { [*emit_words.add(2 * i), *emit_words.add(2 * i + 1)] };
    }
    with_platform(|p| p.emit = emit);
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
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAM: GLuint = 0;

fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(64, 64);
            assert!(!CTX.is_null(), "{:?}", logs());
            PROGRAM = program("passthrough.vert", "uniform_color.frag");
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

/// An RGBA8 texture of `size` (LINEAR_MIPMAP_LINEAR / LINEAR,
/// CLAMP_TO_EDGE) in an FBO, its level 0 rendered by one draw and the frame
/// submitted: the kicks after it are the chain's.
fn rendered_target(size: (i32, i32)) -> GLuint {
    let (mut tex, mut fbo) = (0, 0);
    // SAFETY: room for one name each; a null image; the array is a static.
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
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{size:?}: {:?}", logs());
        glViewport(0, 0, size.0, size.1);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glUseProgram(PROGRAM);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFlush();
        assert_eq!(glGetError(), GL_NO_ERROR, "{size:?}: {:?}", logs());
    }
    tex
}

/// `f` with three levels a render on, off again after it (also when `f`
/// panics); the rig's lock keeps every other test of this file out.
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

/// What one `glGenerateMipmap` did.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Chain {
    /// The GL error it raised.
    error: GLenum,
    /// Its kicks, in order.
    kicks: Vec<KickRec>,
    /// Chains the backend made and left to the CPU in this call
    /// (`pvr_backend_mip_chains` deltas).
    made: u32,
    refused: u32,
}

fn mip_chains() -> (u32, u32) {
    let (mut m, mut r) = (0, 0);
    // SAFETY: two writable words.
    unsafe { pvr_backend_mip_chains(&mut m, &mut r) };
    (m, r)
}

/// One `glGenerateMipmap` of a fresh `size` texture whose level 0 was just
/// rendered, knob `three`, and its flush.
fn chain(size: (i32, i32), three: bool) -> Chain {
    let tex = rendered_target(size);
    let from = with_platform(|p| p.kicks.len());
    let before = mip_chains();
    let run = || {
        // SAFETY: plain GL calls on a name this test made.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex);
            glGenerateMipmap(GL_TEXTURE_2D);
            let e = glGetError();
            glBindTexture(GL_TEXTURE_2D, 0);
            glFlush();
            e
        }
    };
    let error = if three { with_three_levels(run) } else { run() };
    let after = mip_chains();
    let kicks = with_platform(|p| p.kicks[from..].to_vec());
    // SAFETY: a name this test made.
    unsafe { glDeleteTextures(1, &tex) };
    Chain { error, kicks, made: after.0 - before.0, refused: after.1 - before.1 }
}

/// The colour attachments a kick stores (`num_rt` 0 is one).
fn rts(k: &KickRec) -> usize {
    k.num_rt.max(1) as usize
}

/// The bits of an emit pair that are the store address.
fn address_bits() -> [u32; 2] {
    let a = pbe_emit_words(0, GLES_SURF_RGBA8888, 0, 0);
    let b = pbe_emit_words(0xFF_FFFF_FFFF, GLES_SURF_RGBA8888, 0, 0);
    [a[0] ^ b[0], a[1] ^ b[1]]
}

/// The levels each render of a GPU-drawn chain of `n` levels (level 0
/// included) stores with the knob on: level 1 alone, then up to three from
/// the level above them (src/pvr/backend.rs `mip_chain_gpu`).
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

/// One shape: its size, its GL levels, and what the backend does with it --
/// kicks with the knob off and on, the colour attachments of each knob-on
/// render, and whether the chain is the backend's (`true`) or the CPU's.
struct Shape {
    size: (i32, i32),
    levels: usize,
    off_kicks: usize,
    on_rts: &'static [usize],
    gpu: bool,
}

/// The shapes, pinned at this commit. A level one texel wide above a
/// taller tail is refused by the backend (2x128: level 1 is 1x64 at a pitch
/// of 1; 3x5: laid out as 4x8, level 2 is 1x1 at a pitch of 1 with 2 rows).
const SHAPES: &[Shape] = &[
    Shape { size: (1, 1), levels: 1, off_kicks: 0, on_rts: &[], gpu: false },
    Shape { size: (2, 2), levels: 2, off_kicks: 1, on_rts: &[1], gpu: true },
    Shape { size: (4, 4), levels: 3, off_kicks: 2, on_rts: &[1, 1], gpu: true },
    Shape { size: (1, 64), levels: 7, off_kicks: 6, on_rts: &[1, 3, 2], gpu: true },
    Shape { size: (64, 1), levels: 7, off_kicks: 6, on_rts: &[1, 3, 2], gpu: true },
    Shape { size: (2, 128), levels: 8, off_kicks: 0, on_rts: &[], gpu: false },
    Shape { size: (128, 2), levels: 8, off_kicks: 7, on_rts: &[1, 3, 3], gpu: true },
    Shape { size: (8, 1), levels: 4, off_kicks: 3, on_rts: &[1, 2], gpu: true },
    Shape { size: (3, 5), levels: 3, off_kicks: 0, on_rts: &[], gpu: false },
    Shape { size: (16, 16), levels: 5, off_kicks: 4, on_rts: &[1, 3], gpu: true },
];

/// The GL extent of level `l` of a `w` x `h` chain.
fn side(size: (i32, i32), l: usize) -> (u32, u32) {
    (((size.0 as u32) >> l).max(1), ((size.1 as u32) >> l).max(1))
}

/// Every shape, knob off and on: the same GL error (none), the pinned
/// kicks, a kick a level off; on, level 1 alone and then up to three levels
/// a render, each attachment stored where the knob-off kick of its level
/// stores, with that kick's PBE register words (pitch, size, maxclip).
#[test]
fn every_degenerate_shape_stores_each_level_as_its_level_kick() {
    let _g = rig();
    let mask = address_bits();
    let addr = |e: [u32; 2]| [e[0] & mask[0], e[1] & mask[1]];
    for s in SHAPES {
        let what = format!("{}x{}", s.size.0, s.size.1);
        assert_eq!(yakogl::gl::context::gl_mip_levels(s.size.0 as u32, s.size.1 as u32) as usize, s.levels, "{what}");
        let off = chain(s.size, false);
        let on = chain(s.size, true);
        println!(
            "{what}: {} levels; knob off {} kicks, on {} kicks {:?}; GL error {:#x} / {:#x}; made {}/{}, refused {}/{}",
            s.levels,
            off.kicks.len(),
            on.kicks.len(),
            on.kicks.iter().map(rts).collect::<Vec<_>>(),
            off.error,
            on.error,
            off.made,
            on.made,
            off.refused,
            on.refused
        );
        assert_eq!((off.error, on.error), (GL_NO_ERROR, GL_NO_ERROR), "{what}: {:?}", logs());
        let gpu = u32::from(s.gpu);
        let cpu = u32::from(!s.gpu && s.levels > 1);
        assert_eq!((off.made, off.refused), (gpu, cpu), "{what}, knob off: chains made / refused");
        assert_eq!((on.made, on.refused), (gpu, cpu), "{what}, knob on: chains made / refused");
        assert_eq!(off.kicks.len(), s.off_kicks, "{what}: kicks with the knob off");
        if s.gpu {
            assert_eq!(s.off_kicks, s.levels - 1, "{what}: a kick a level");
        }
        let on_rts: Vec<usize> = on.kicks.iter().map(rts).collect();
        assert_eq!(on_rts, s.on_rts, "{what}: the colour attachments of each knob-on render");
        if !s.gpu {
            continue;
        }
        let g = groups(s.levels);
        assert_eq!(g.iter().map(Vec::len).collect::<Vec<_>>(), s.on_rts, "{what}: the renders are the groups");
        /* every kick declares level 1's size (the hardware render target set
         * of the chain) and the knob-off kicks store one level each */
        let l1 = side(s.size, 1);
        for (i, k) in off.kicks.iter().chain(&on.kicks).enumerate() {
            assert_eq!(k.size, l1, "{what}: kick {i} is not level 1's size");
        }
        assert!(off.kicks.iter().all(|k| rts(k) == 1), "{what}: one attachment a level kick");
        /* the chains' storage is not the same texture's: the knob-off level
         * addresses, relative to level 1's */
        let rel = |c: &Chain, e: [u32; 2]| -> i64 {
            let a = |w: [u32; 2]| (u64::from(w[1] & mask[1]) << 32 | u64::from(w[0] & mask[0])) as i64;
            a(addr(e)) - a(addr(c.kicks[0].emit[0]))
        };
        let off_addr: Vec<i64> = off.kicks.iter().map(|k| rel(&off, k.emit[0])).collect();
        let distinct: std::collections::BTreeSet<i64> = off_addr.iter().copied().collect();
        assert_eq!(distinct.len(), off_addr.len(), "{what}: every level kick stores its own level");
        for (r, (kick, levels)) in on.kicks.iter().zip(&g).enumerate() {
            for (slot, &l) in levels.iter().enumerate() {
                let at = rel(&on, kick.emit[slot]);
                let stored = off_addr.iter().position(|&a| a == at).map(|i| i + 1);
                assert_eq!(stored, Some(l), "{what}: render {r}, attachment {slot}: not level {l}'s address");
                assert_eq!(
                    kick.pbe[slot],
                    off.kicks[l - 1].pbe[0],
                    "{what}: level {l}, attachment {slot} of render {r}: not the level kick's pitch and clip"
                );
            }
        }
    }
}

/// Generating the same chain again with the other knob gives the same
/// kicks as a fresh texture does: the knob is read at each chain, and a
/// chain made with it on leaves nothing behind for the next one off.
#[test]
fn the_knob_is_read_at_each_chain_of_every_shape() {
    let _g = rig();
    for s in SHAPES {
        let a = chain(s.size, false);
        let b = chain(s.size, true);
        let c = chain(s.size, false);
        let d = chain(s.size, true);
        let sizes = |c: &Chain| c.kicks.iter().map(|k| (k.size, rts(k), k.pbe)).collect::<Vec<_>>();
        assert_eq!(sizes(&a), sizes(&c), "{}x{}: knob off, before and after a knob-on chain", s.size.0, s.size.1);
        assert_eq!(sizes(&b), sizes(&d), "{}x{}: knob on, twice", s.size.0, s.size.1);
    }
}

/* ---- the level contents ------------------------------------------------------- */

/* mipgen3.frag is GLSL ES 3.00, which a report-es2 build refuses
 * (pp.rs out_of_scope): left out of it, as in tests/pvr_mip_three_levels.rs. */
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
mod levels {
    use super::*;
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

    /// The TPU model's bilinear sample of `l` (one level, CLAMP_TO_EDGE) at
    /// the normalised point `c` (tests/pvr_mip_three_levels.rs).
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

    /// A level kick (mipgen.frag): `w` x `h` texels, each the sample of `l`
    /// at gl_FragCoord.xy * u_scale.
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

    /// Level 0: `random` bytes, or a `smooth` picture with a little noise
    /// (tests/pvr_mip_three_levels.rs).
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

    /// mipgen3.frag's render of `dst` (2 or 3 levels) from `src`: every
    /// pixel of `dst[0]`'s extent run by the evaluator with the block
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

    /// The most a level may differ from the knob-off chain in a channel.
    /// Measured at this commit: 0 at every level of every shape here (each
    /// one a power of two on both axes, so every weight is exactly 1/2 or 1;
    /// the 1-texel axis samples one texel twice).
    const MAX_D: u8 = 1;

    /// Every GPU-drawn shape, knob on against knob off, the renders those
    /// the backend submitted with the knob on: level 1 is the level kick's
    /// byte for byte, and every level is within [`MAX_D`] of the knob-off
    /// chain's.
    #[test]
    fn every_degenerate_shape_s_levels_are_within_1_lsb_of_the_level_kick_chain() {
        let _g = rig();
        let mut buf = vec![0u8; 8 << 20];
        let a = Arena::new(&mut buf);
        let lowered = compile_to_ir(&a, Stage::Fragment, MIPGEN3).unwrap_or_else(|e| panic!("{}: {}", e.line, e.msg));
        let m = &lowered.module;
        for s in SHAPES.iter().filter(|s| s.gpu) {
            let n = s.levels;
            /* the renders as the backend submits them: their attachments */
            let on_rts: Vec<usize> = chain(s.size, true).kicks.iter().map(rts).collect();
            assert_eq!(on_rts, s.on_rts, "{}x{}", s.size.0, s.size.1);
            let sz: Vec<(u32, u32)> = (0..n).map(|l| side(s.size, l)).collect();
            for kind in ["smooth", "random"] {
                let (w, h) = sz[0];
                let mut off = vec![level0(w, h, kind)];
                for l in 1..n {
                    let next = level_kick(&off[l - 1], sz[l].0, sz[l].1);
                    off.push(next);
                }
                let mut on: Vec<Level> = vec![off[0].clone()];
                for &k in &on_rts {
                    let first = on.len();
                    let src = on[first - 1].clone();
                    if k == 1 {
                        on.push(level_kick(&src, sz[first].0, sz[first].1));
                    } else {
                        on.extend(three_level_render(m, &src, &sz[first..first + k]));
                    }
                }
                assert_eq!(on.len(), n, "{w}x{h}: the renders store every level once");
                for l in 1..n {
                    let what = format!("{w}x{h} {kind} level {l} ({}x{})", sz[l].0, sz[l].1);
                    let (mut texd, mut maxd) = (0, 0u8);
                    for (a, b) in on[l].px.iter().zip(&off[l].px) {
                        let d = (0..4).map(|c| a[c].abs_diff(b[c])).max().unwrap();
                        texd += usize::from(d != 0);
                        maxd = maxd.max(d);
                    }
                    println!("{what}: {texd} of {} texels differ, max |d| {maxd}", on[l].px.len());
                    if l == 1 {
                        assert_eq!(maxd, 0, "{what}: not the level kick's bytes");
                    }
                    assert!(maxd <= MAX_D, "{what}: {maxd} from the level kick chain");
                }
            }
        }
    }
}
