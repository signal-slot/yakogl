// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The resolve blit of a 2x or 4x depth/stencil renderbuffer on the
//! PowerVR (BXS-4-64, AM62P) writes the right values, through the GL API
//! over the REAL PowerVR host back end (plan #400, item fbo-msaablit).
//!
//! dEQP-GLES3 `fbo.msaa.{2,4}_samples.{depth_component16, depth_component24,
//! depth_component32f, depth24_stencil8, depth32f_stencil8, stencil_index8}`
//! (12 lines of docs/es3/am62p-gles3-notpass.tsv, "GL_INVALID_OPERATION at
//! es3fFboTestCase.cpp:90") draw into a multisampled FBO and then blit its
//! colour and depth/stencil into `resolveFbo`. That FBO's depth/stencil
//! renderbuffer is new and no frame has drawn into it
//! (es3fFboMultisampleTests.cpp, BasicFboMultisampleCase::render). Then they
//! draw into `resolveFbo` with depth and stencil tests against what the
//! blit wrote.
//!
//! The cause (tests/es3_msaa_ds_resolve.rs): `pvr_backend_ds_write` answered
//! 1 ("nothing held") for an object no buffer held. It has no size to make a
//! buffer by. The GL core's probe (`blit.rs::dst_writable`) then refused the
//! whole blit. The fix is in src/pvr only (src/pvr/ds_stage.rs). Such a
//! write is staged in a CPU plane of its own. The resolves read it. The
//! object's first frame writes it into the buffer it gets and loads it
//! (`ds_seed_staged`). No src/gl change: the GL core's probe, copy and
//! refusal rules stay as they are.
//!
//! What this file checks, for the six formats at 2 and 4 samples:
//! * the blit answers GL_NO_ERROR;
//! * the destination's depth/stencil, read back through the back end's
//!   resolves, is sample 0 of each source pixel (the word at (2x, 2y) of
//!   the ISP's 2-by-2 scaled image; 2 samples are stored as 4). Which
//!   sample the BXS-4-64 stores there is BOARD-UNVERIFIED
//!   (docs/notes/es3-msaa-renderbuffers.md (e) open point 6);
//! * the next draw into `resolveFbo` loads exactly those words
//!   (CR_ISP_ZLSCTL zloaden / sloaden, and the buffer as its kick finds it);
//! * a blit of part of the plane leaves depth 1.0 and stencil 0 around it;
//! * re-specifying the renderbuffer drops what was staged.
//!
//! HOST-ONLY: the GPU is a stand-in that paints memory; no board was used.
//! The rig is the one of tests/es3_msaa_ds_resolve.rs and
//! tests/pvr_msaa_rb.rs, with a snapshot of each kick's loaded buffer.
//!
//! The multisampled storage needs the PowerVR's 4x framebuffer objects,
//! which it has only with the `msaa-fbo` feature:
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,msaa-fbo --test pvr_msaa_ds_blit
//! ```
//!
//! Not built with `report-es2`.

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
    feature = "msaa-fbo",
    not(feature = "no-shader-table"),
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]
/* the rig is shared with tests/pvr_msaa_rb.rs; this file uses part of it */
#![allow(dead_code)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
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

/* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE): init refuses
 * a platform that answers another. */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

/// ISP_ZLSCTL.forcezstore: the kick stores its depth/stencil.
const ZLSCTL_FORCEZSTORE: u64 = 1 << 2;

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
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
    log: Vec<String>,
    /// Every kick submitted, in order.
    kicks: Vec<PvrRenderKick>,
    /// The PBESTATE_STATE_WORD0/1 pairs of each end-of-tile program built
    /// (`pvr_pds_generate_pixel_event_tq`), in order: the kick submitted
    /// next is the one they belong to.
    emits: Vec<Vec<[u32; 2]>>,
    /// The packed RGBA8 colour a 4x kick resolves into its first emit's
    /// surface (the end-of-tile `downscale` store), 0 for none.
    paint: u32,
    /// Per kick: its CR_ISP_ZLSCTL and the words of its depth/stencil buffer
    /// as the kick found them (before the "GPU" stores), from its base to
    /// the allocation's end.
    loads: Vec<(u64, Vec<u32>)>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The ZLS tile (`ZLS_TILE` in src/pvr/backend.rs): a depth/stencil
/// buffer's rows are its width aligned to it, doubled at 4x.
const ZLS_TILE: u32 = 32;

/// The D24S8 word the "GPU" stores at (X, Y) of a depth/stencil buffer, in
/// the buffer's own coordinates (the ISP's 2-by-2 scaled image at 4x): a
/// depth that differs per word in bits 23..0 and a stencil byte (31..24)
/// that differs too, never 0.
fn ds_word(x: u32, y: u32) -> u32 {
    let d = (x * 7 + y * 131 + 5_000) & 0x00ff_ffff;
    let s = ((x * 3 + y * 5) & 0x7f) + 1;
    d | s << 24
}

/// The surface address of a PBESTATE_STATE_WORD0/1 pair (pbestate.h:
/// address_low is address bits 33..2 in word 0, address_high bits 39..34 in
/// word 1's bits 5..0).
fn emit_address(w: [u32; 2]) -> u64 {
    (w[0] as u64) << 2 | ((w[1] & 0x3f) as u64) << 34
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

/// The "GPU": a kick that stores depth leaves `ds_word` in every word of
/// its depth/stencil buffer, rows the target's ZLS-aligned width (twice that
/// at 4x) apart; a 4x kick writes `paint` over the surface of its first
/// emit, as the PBE's `downscale` store resolves into it.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.fence += 1;
        p.kicks.push(k);
        let scale = if k.samples == 4 { 2 } else { 1 };
        let row = (k.width.div_ceil(ZLS_TILE) * ZLS_TILE * scale) as usize;
        let zs = k.frag.isp_zload_store_base;
        let found = p.allocs.range(..=zs).next_back().filter(|_| zs != 0).map(|(&base, a)| {
            let off = (zs - base) as usize;
            let first = (a.mem.as_ptr() as *const u8).wrapping_add(off) as *const u32;
            // SAFETY: the words from `zs` to the allocation's end.
            unsafe { core::slice::from_raw_parts(first, a.size.saturating_sub(off) / 4) }.to_vec()
        });
        p.loads.push((k.frag.isp_zlsctl, found.unwrap_or_default()));
        let stores = zs != 0 && k.frag.isp_zlsctl & ZLSCTL_FORCEZSTORE != 0;
        if let Some((&base, a)) = p.allocs.range_mut(..=zs).next_back().filter(|_| stores) {
            let off = (zs - base) as usize;
            if off < a.size {
                let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
                // SAFETY: the words from `zs` to the allocation's end.
                let words = unsafe { core::slice::from_raw_parts_mut(first, (a.size - off) / 4) };
                for (i, w) in words.iter_mut().enumerate() {
                    *w = ds_word((i % row) as u32, (i / row) as u32);
                }
            }
        }
        let target = p.emits.last().and_then(|e| e.first()).map_or(0, |&w| emit_address(w));
        let paint = p.paint;
        if k.samples == 4 && paint != 0 && target != 0 {
            if let Some((&base, a)) = p.allocs.range_mut(..=target).next_back() {
                let off = (target - base) as usize;
                let n = (k.height * k.width.next_multiple_of(2)) as usize;
                if off + n * 4 <= a.size {
                    let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
                    // SAFETY: n words from `target`, inside the allocation.
                    unsafe { core::slice::from_raw_parts_mut(first, n) }.fill(paint);
                }
            }
        }
        p.fence
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
    // SAFETY: the backend passes `num_rt` state pairs.
    let pairs = (0..num_rt as usize).map(|i| unsafe { [*emit_words.add(2 * i), *emit_words.add(2 * i + 1)] });
    let pairs: Vec<[u32; 2]> = pairs.collect();
    with_platform(|p| p.emits.push(pairs));
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

/// The default framebuffer's size.
const CANVAS: (i32, i32) = (40, 30);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut PROGRAM: GLuint = 0;

/// The one context and backend of the file, set up once; the tests take
/// turns. Each starts with every kick submitted and completed.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            assert!(!context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32).is_null(), "{:?}", logs());
            PROGRAM = program("passthrough.vert", "uniform_color.frag");
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    with_platform(|p| {
        p.completed_to = p.fence;
        p.hold = false;
        p.log.clear();
        p.kicks.clear();
        p.emits.clear();
        p.paint = 0;
        p.loads.clear();
    });
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

/* a triangle at z = 0 covering the whole target, (x, y, z, w) per vertex */
static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
static UV: [f32; 6] = [0.0, 0.0, 2.0, 0.0, 0.0, 2.0];

fn draw_tri(prog: GLuint) {
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

/* ---- helpers ------------------------------------------------------------------ */

/// The renderbuffers' size: one ZLS tile.
const W: i32 = 32;
const H: i32 = 32;

/// PBESTATE_REG_WORD0 `downscale` (bit 56 of CR_PBE_WORD0, pbestate.xml):
/// the PBE resolves the tile's samples on the store.
const PBE_DOWNSCALE: u64 = 1 << 56;

/// A 24-bit depth word's denominator (`n / 16777215`).
const MAXD: f32 = 16_777_215.0;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn geti(pname: GLenum) -> GLint {
    let mut v: GLint = -1;
    // SAFETY: one GLint.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/// glGetInternalformativ(GL_RENDERBUFFER, `format`, GL_SAMPLES): every count.
fn sample_counts(format: GLenum) -> Vec<GLint> {
    let mut n: GLint = -1;
    // SAFETY: one GLint, then `n` of them.
    unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, GL_NUM_SAMPLE_COUNTS, 1, &mut n) };
    assert_eq!(err(), GL_NO_ERROR, "GL_NUM_SAMPLE_COUNTS of {format:#x}");
    let mut v = vec![-1; n.max(0) as usize];
    if n > 0 {
        // SAFETY: room for `n` GLints.
        unsafe { glGetInternalformativ(GL_RENDERBUFFER, format, GL_SAMPLES, n, v.as_mut_ptr()) };
        assert_eq!(err(), GL_NO_ERROR, "GL_SAMPLES of {format:#x}");
    }
    v
}

/// A `W` x `H` renderbuffer of `format` with `samples` requested.
fn renderbuffer(format: GLenum, samples: i32) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, W, H);
    }
    assert_eq!(err(), GL_NO_ERROR, "{samples} samples of {format:#x}: {:?}", logs());
    rb
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one GLint; plain GL calls.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    v
}

/// The GPU address of renderbuffer `rb`'s (resolved, single-sample) storage.
fn rb_gpu(rb: GLuint) -> u64 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    c.renderbuffer_object(slot).expect("a renderbuffer").mem.gpu
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

/// A framebuffer of the given colour renderbuffers and depth/stencil one
/// (with its format, which names the attachment point), checked complete.
fn framebuffer(colour: &[GLuint], ds: Option<(GLuint, GLenum)>) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &rb) in colour.iter().enumerate() {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
        }
        if let Some((rb, format)) = ds {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, ds_point(format), GL_RENDERBUFFER, rb);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    fb
}

/// Clear the bound framebuffer and draw the triangle over it, depth-tested
/// and writing stencil, so both planes hold what the kick stored.
fn clear_and_draw() {
    // SAFETY: plain GL calls; `PROGRAM` is set once in `rig`.
    unsafe {
        glViewport(0, 0, W, H);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClearDepthf(0.75);
        glClearStencil(0x11);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_ALWAYS, 0x22, 0xff);
        glStencilOp(GL_KEEP, GL_KEEP, GL_REPLACE);
        draw_tri(PROGRAM);
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_DEPTH_TEST);
    }
}

fn release(fbs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names these tests made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/// CR_ISP_AA of a 4x frame, packed by Mesa's own packer.
fn isp_aa_4x() -> u32 {
    let mut dw = [0u32; 1];
    let v = yakogl::pvr::csbgen::ROGUE_CR_ISP_AA { mode: yakogl::pvr::csbgen::ROGUE_CR_ISP_AA_MODE_TYPE_AA_4X };
    // SAFETY: the packer writes its one dword from one initialised structure.
    unsafe { yakogl::pvr::csbgen::ROGUE_CR_ISP_AA_pack(dw.as_mut_ptr().cast(), &v) };
    dw[0]
}

/* ---- the cases -------------------------------------------------------------- */

/// The six depth/stencil formats of dEQP's `fbo.msaa` groups: (format, has
/// depth, has stencil).
const FORMATS: [(GLenum, bool, bool); 6] = [
    (GL_DEPTH_COMPONENT16, true, false),
    (GL_DEPTH_COMPONENT24, true, false),
    (GL_DEPTH_COMPONENT32F, true, false),
    (GL_DEPTH24_STENCIL8, true, true),
    (GL_DEPTH32F_STENCIL8, true, true),
    (GL_STENCIL_INDEX8, false, true),
];

/// The attachment point of a depth/stencil format.
fn ds_point(format: GLenum) -> GLenum {
    match format {
        GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 => GL_DEPTH_STENCIL_ATTACHMENT,
        GL_STENCIL_INDEX8 => GL_STENCIL_ATTACHMENT,
        _ => GL_DEPTH_ATTACHMENT,
    }
}

/// The blit mask of a format's planes.
fn ds_mask(depth: bool, stencil: bool) -> GLbitfield {
    (if depth { GL_DEPTH_BUFFER_BIT } else { 0 }) | if stencil { GL_STENCIL_BUFFER_BIT } else { 0 }
}

/// CR_ISP_ZLSCTL with only `zloaden` or `sloaden` set, by Mesa's packer.
fn zlsctl_load(depth: bool) -> u64 {
    use yakogl::pvr::csbgen as csb;
    let v = csb::ROGUE_CR_ISP_ZLSCTL { zloaden: depth, sloaden: !depth, ..Default::default() };
    let mut dw = [0u32; 2];
    // SAFETY: the packer reads one initialised structure and writes its two dwords.
    unsafe { csb::ROGUE_CR_ISP_ZLSCTL_pack(dw.as_mut_ptr().cast(), &v) };
    u64::from(dw[0]) | u64::from(dw[1]) << 32
}

/// dEQP's sequence up to its resolve: a `samples` colour + `format`
/// framebuffer cleared and drawn into, and a fresh single-sample one (storage
/// and attachments only). (msaa fb, resolve fb, resolve ds rb, every rb.)
fn msaa_and_fresh_resolve(format: GLenum, samples: i32) -> (GLuint, GLuint, GLuint, Vec<GLuint>) {
    let (ms_c, ms_ds) = (renderbuffer(GL_RGBA8, samples), renderbuffer(format, samples));
    let ms_fb = framebuffer(&[ms_c], Some((ms_ds, format)));
    clear_and_draw();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    let (one_c, one_ds) = (renderbuffer(GL_RGBA8, 0), renderbuffer(format, 0));
    let one_fb = framebuffer(&[one_c], Some((one_ds, format)));
    (ms_fb, one_fb, one_ds, vec![ms_c, ms_ds, one_c, one_ds])
}

/// glBlitFramebuffer from `read` to `draw` of `src` to `dst` with `mask`,
/// NEAREST; its GL error.
fn blit(read: GLuint, draw: GLuint, src: [i32; 4], dst: [i32; 4], mask: GLbitfield) -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
        glBlitFramebuffer(src[0], src[1], src[2], src[3], dst[0], dst[1], dst[2], dst[3], mask, GL_NEAREST);
        let e = glGetError();
        glFinish();
        e
    }
}

/// The depth (as 24-bit words) and stencil of renderbuffer `rb` through the
/// back end's resolves, with their return codes.
fn read_back(rb: GLuint) -> ((i32, Vec<u32>), (i32, Vec<u8>)) {
    let (w, h) = (W as u32, H as u32);
    let id = ds_id(rb);
    let mut depth = vec![-1.0f32; (w * h) as usize];
    let mut stencil = vec![0xeeu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats and of `w` bytes.
    let (rd, rs) = unsafe {
        (
            pvr_backend_depth_resolve(id, depth.as_mut_ptr(), w, h, w),
            pvr_backend_stencil_resolve(id, stencil.as_mut_ptr(), w, h, w),
        )
    };
    ((rd, depth.iter().map(|d| (d * MAXD).round() as u32).collect()), (rs, stencil))
}

/// Sample 0 of pixel (x, y) of the multisampled source as the fake GPU
/// stored it: the word at (2x, 2y) of the 2-by-2 scaled image.
fn sample0(x: u32, y: u32) -> u32 {
    ds_word(2 * x, 2 * y)
}

/// The resolve blit of every format at 2 and 4 samples is accepted and
/// writes sample 0 of each pixel into the fresh destination.
#[test]
fn resolve_blit_into_a_fresh_renderbuffer_writes_sample_0() {
    for samples in [2, 4] {
        for (format, depth, stencil) in FORMATS {
            let _g = rig();
            let (ms_fb, one_fb, one_ds, rbs) = msaa_and_fresh_resolve(format, samples);
            let e = blit(ms_fb, one_fb, [0, 0, W, H], [0, 0, W, H], GL_COLOR_BUFFER_BIT | ds_mask(depth, stencil));
            assert_eq!(e, GL_NO_ERROR, "{samples} samples of {format:#x}: {:?}", logs());
            let ((rd, d), (rs, s)) = read_back(one_ds);
            for y in 0..H as u32 {
                for x in 0..W as u32 {
                    let (i, want) = ((y * W as u32 + x) as usize, sample0(x, y));
                    if depth {
                        assert_eq!(rd, 0, "{samples}x {format:#x}: depth resolve");
                        assert_eq!(d[i], want & 0xff_ffff, "{samples}x {format:#x}: depth at ({x}, {y})");
                    }
                    if stencil {
                        assert_eq!(rs, 0, "{samples}x {format:#x}: stencil resolve");
                        assert_eq!(u32::from(s[i]), want >> 24, "{samples}x {format:#x}: stencil at ({x}, {y})");
                    }
                }
            }
            release(&[ms_fb, one_fb], &rbs);
        }
    }
}

/// dEQP's next step: a draw into `resolveFbo` tested against what the blit
/// wrote. Its kick loads the plane(s) (zloaden / sloaden), and the buffer it
/// loads holds sample 0 of each pixel, row-linear at the ZLS-aligned width.
#[test]
fn the_first_draw_after_the_blit_loads_the_blitted_planes() {
    for samples in [2, 4] {
        for (format, depth, stencil) in FORMATS {
            let _g = rig();
            let (ms_fb, one_fb, _one_ds, rbs) = msaa_and_fresh_resolve(format, samples);
            let e = blit(ms_fb, one_fb, [0, 0, W, H], [0, 0, W, H], GL_COLOR_BUFFER_BIT | ds_mask(depth, stencil));
            assert_eq!(e, GL_NO_ERROR, "{samples} samples of {format:#x}: {:?}", logs());
            with_platform(|p| p.loads.clear());
            // SAFETY: plain GL calls; `PROGRAM` is set once in `rig`.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, one_fb);
                glViewport(0, 0, W, H);
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_LESS);
                glDepthMask(0);
                glEnable(GL_STENCIL_TEST);
                glStencilFunc(GL_EQUAL, 0x22, 0xff);
                glStencilOp(GL_KEEP, GL_KEEP, GL_KEEP);
                draw_tri(PROGRAM);
                glFinish();
                glDepthMask(1);
                glDisable(GL_STENCIL_TEST);
                glDisable(GL_DEPTH_TEST);
            }
            let loads = with_platform(|p| p.loads.clone());
            assert_eq!(loads.len(), 1, "{samples}x {format:#x}: one kick");
            let (zlsctl, words) = &loads[0];
            if depth {
                assert_ne!(zlsctl & zlsctl_load(true), 0, "{samples}x {format:#x}: zloaden");
            }
            if stencil {
                assert_ne!(zlsctl & zlsctl_load(false), 0, "{samples}x {format:#x}: sloaden");
            }
            let row = (W as u32).div_ceil(ZLS_TILE) as usize * ZLS_TILE as usize;
            let mask = (if depth { 0x00ff_ffff } else { 0 }) | if stencil { 0xff00_0000u32 } else { 0 };
            for y in 0..H as u32 {
                for x in 0..W as u32 {
                    let got = words[y as usize * row + x as usize];
                    assert_eq!(got & mask, sample0(x, y) & mask, "{samples}x {format:#x}: loaded at ({x}, {y})");
                }
            }
            release(&[ms_fb, one_fb], &rbs);
        }
    }
}

/// A blit of part of the plane into a fresh D24S8 (the same rectangle on
/// both sides, as a multisampled read framebuffer asks): the rectangle holds the
/// source's sample 0, every other pixel depth 1.0 and stencil 0 (what a
/// frame starts a new buffer with), for the resolves and for the next draw.
#[test]
fn a_partial_blit_leaves_the_frame_start_values_around_it() {
    let _g = rig();
    let (ms_fb, one_fb, one_ds, rbs) = msaa_and_fresh_resolve(GL_DEPTH24_STENCIL8, 4);
    let e = blit(ms_fb, one_fb, [8, 6, 24, 16], [8, 6, 24, 16], GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
    assert_eq!(e, GL_NO_ERROR, "{:?}", logs());
    let want = |x: u32, y: u32| {
        if (8..24).contains(&x) && (6..16).contains(&y) {
            sample0(x, y)
        } else {
            0x00ff_ffff
        }
    };
    let ((rd, d), (rs, s)) = read_back(one_ds);
    assert_eq!((rd, rs), (0, 0));
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            let i = (y * W as u32 + x) as usize;
            assert_eq!(d[i] | u32::from(s[i]) << 24, want(x, y), "resolved at ({x}, {y})");
        }
    }
    with_platform(|p| p.loads.clear());
    // SAFETY: plain GL calls; `PROGRAM` is set once in `rig`.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, one_fb);
        glEnable(GL_DEPTH_TEST);
        glDepthMask(0);
        draw_tri(PROGRAM);
        glFinish();
        glDepthMask(1);
        glDisable(GL_DEPTH_TEST);
    }
    let loads = with_platform(|p| p.loads.clone());
    assert_eq!(loads.len(), 1);
    for y in 0..H as u32 {
        for x in 0..W as u32 {
            assert_eq!(loads[0].1[(y * W as u32 + x) as usize], want(x, y), "loaded at ({x}, {y})");
        }
    }
    release(&[ms_fb, one_fb], &rbs);
}

/// Re-specifying the destination renderbuffer before any draw drops what the
/// blit staged: the resolves answer "nothing held" (1) again.
#[test]
fn respecifying_the_destination_drops_the_staged_planes() {
    let _g = rig();
    let (ms_fb, one_fb, one_ds, rbs) = msaa_and_fresh_resolve(GL_DEPTH24_STENCIL8, 4);
    let e = blit(ms_fb, one_fb, [0, 0, W, H], [0, 0, W, H], GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
    assert_eq!(e, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(read_back(one_ds).0 .0, 0, "staged");
    // SAFETY: plain GL calls.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, one_ds);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, W, H);
    }
    assert_eq!(err(), GL_NO_ERROR);
    let ((rd, _), (rs, _)) = read_back(one_ds);
    assert_eq!((rd, rs), (1, 1), "nothing held after the new storage");
    release(&[ms_fb, one_fb], &rbs);
}
