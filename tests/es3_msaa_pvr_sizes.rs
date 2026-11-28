// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64) 4x multisample renderbuffer FBOs at the sizes dEQP
//! draws at and around the 16x16 ISP tile (plan #339, item
//! pvr-msaa-size-tests; docs/notes/es3-msaa-renderbuffers.md), through the GL
//! API over the REAL PowerVR host back end. It covers PowerVR the way
//! tests/es3_msaa_gc_pad.rs covers GC7000UL. The rig is the one of
//! tests/pvr_msaa_rb.rs.
//!
//! Sizes: 119x131 and 40x40 (dEQP's fbo.msaa), 1x1, and one under, on and
//! one over the tile (16x16: 15, 16, 17) and the ZLS tile (32: 31, 32, 33).
//! For each, a 4x colour renderbuffer FBO is cleared, a triangle is drawn
//! and the FBO is resolve-blitted to a 1x renderbuffer of the same size:
//!
//! * the 4x kick is issued for the full extent (width and height of the
//!   renderbuffer, CR_ISP_AA 4X, resolving store `downscale`, a second
//!   unresolved emit into the sample store);
//! * the sample store and the resolved surface lie inside their allocations;
//! * every pixel of the resolved destination, edges included, holds what
//!   the end-of-tile store resolved (the fake GPU's `paint`).
//!
//! HOST-ONLY: the GPU is a stand-in that paints memory; no board was used.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,msaa-fbo --test es3_msaa_pvr_sizes
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

/// PBESTATE_REG_WORD0 `downscale` (bit 56 of CR_PBE_WORD0, pbestate.xml).
const PBE_DOWNSCALE: u64 = 1 << 56;

/// The ISP tile (16x16 pixels) and the ZLS tile.
const ISP_TILE: i32 = 16;
const ZLS: i32 = 32;

const PAINT: u32 = 0xff33_66cc;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn renderbuffer(w: i32, h: i32, samples: i32) -> GLuint {
    let mut rb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_RGBA8, w, h);
    }
    assert_eq!(err(), GL_NO_ERROR, "{w}x{h} {samples} samples: {:?}", logs());
    rb
}

fn rb_gpu(rb: GLuint) -> u64 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    c.renderbuffer_object(slot).expect("a renderbuffer").mem.gpu
}

fn framebuffer(rb: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    fb
}

fn clear_and_draw(w: i32, h: i32) {
    // SAFETY: plain GL calls; `PROGRAM` is set once in `rig`.
    unsafe {
        glViewport(0, 0, w, h);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        draw_tri(PROGRAM);
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

fn isp_aa_4x() -> u32 {
    let mut dw = [0u32; 1];
    let v = yakogl::pvr::csbgen::ROGUE_CR_ISP_AA { mode: yakogl::pvr::csbgen::ROGUE_CR_ISP_AA_MODE_TYPE_AA_4X };
    // SAFETY: the packer writes its one dword from one initialised structure.
    unsafe { yakogl::pvr::csbgen::ROGUE_CR_ISP_AA_pack(dw.as_mut_ptr().cast(), &v) };
    dw[0]
}

fn four_sample_kicks() -> Vec<(PvrRenderKick, Vec<[u32; 2]>)> {
    with_platform(|p| {
        assert_eq!(p.kicks.len(), p.emits.len(), "one end-of-tile program a kick");
        p.kicks.iter().zip(&p.emits).filter(|(k, _)| k.samples == 4).map(|(k, e)| (*k, e.clone())).collect()
    })
}

/// (offset of `addr` in its allocation, the allocation's size), if `addr`
/// lies in one.
fn alloc_room(addr: u64) -> Option<(usize, usize)> {
    with_platform(|p| p.allocs.range(..=addr).next_back().map(|(&b, a)| ((addr - b) as usize, a.size)))
}

/// Draw at `w`x`h` into a 4x FBO, resolve-blit it into a 1x one and return
/// the resolved RGBA8 pixels (and the kick the draw made).
fn draw_and_resolve(w: i32, h: i32) -> (Vec<u8>, PvrRenderKick, Vec<[u32; 2]>, GLuint, [GLuint; 2], [GLuint; 2]) {
    with_platform(|p| p.paint = PAINT);
    let ms = renderbuffer(w, h, 4);
    let ms_fb = framebuffer(ms);
    clear_and_draw(w, h);
    let one = renderbuffer(w, h, 0);
    let one_fb = framebuffer(one);
    let mut px = vec![0u8; (w * h * 4) as usize];
    // SAFETY: plain GL calls; `px` holds w x h RGBA8 pixels.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, ms_fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, one_fb);
        glBlitFramebuffer(0, 0, w, h, 0, 0, w, h, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(glGetError(), GL_NO_ERROR, "{w}x{h}: {:?}", logs());
        glBindFramebuffer(GL_READ_FRAMEBUFFER, one_fb);
        glReadPixels(0, 0, w, h, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "{w}x{h}: {:?}", logs());
    }
    let mut kicks = four_sample_kicks();
    assert_eq!(kicks.len(), 1, "{w}x{h}: the blit submitted one 4x kick: {:?}", logs());
    let (k, e) = kicks.remove(0);
    (px, k, e, ms, [ms_fb, one_fb], [one, one_fb])
}

fn check_size(w: i32, h: i32) {
    let _g = rig();
    let (px, k, emits, ms, fbs, ones) = draw_and_resolve(w, h);
    /* the kick covers the whole renderbuffer */
    assert_eq!((k.width, k.height), (w as u32, h as u32), "{w}x{h}: kick extent");
    assert_eq!(k.frag.isp_aa, isp_aa_4x(), "{w}x{h}: CR_ISP_AA mode 4X");
    assert_eq!(k.frag.num_rt, 2, "{w}x{h}: the resolved store and the sample store");
    assert_ne!(k.frag.pbe_word[0][0] & PBE_DOWNSCALE, 0, "{w}x{h}: store 0 resolves");
    assert_eq!(k.frag.pbe_word[1][0] & PBE_DOWNSCALE, 0, "{w}x{h}: store 1 keeps the samples");
    assert_eq!(emit_address(emits[0]), rb_gpu(ms), "{w}x{h}: emit 0 stores into the renderbuffer");
    /* no out-of-bounds: both surfaces fit their allocations */
    let (off, size) = alloc_room(rb_gpu(ms)).expect("the renderbuffer's allocation");
    assert!(off + (w * h * 4) as usize <= size, "{w}x{h}: resolved surface in its allocation");
    let store = emit_address(emits[1]);
    let (off, size) = alloc_room(store).expect("the sample store's allocation");
    assert!(off + (w * h * 4 * 4) as usize <= size, "{w}x{h}: sample store holds 4 samples a pixel");
    /* every pixel, edges included, is resolved */
    for y in 0..h {
        for x in 0..w {
            let i = ((y * w + x) * 4) as usize;
            assert_eq!(px[i..i + 4], [0xcc, 0x66, 0x33, 0xff], "{w}x{h}: pixel ({x}, {y})");
        }
    }
    release(&fbs, &[ms, ones[0]]);
}

#[test]
fn deqp_119x131() {
    check_size(119, 131);
}

#[test]
fn deqp_40x40() {
    check_size(40, 40);
}

#[test]
fn one_by_one() {
    check_size(1, 1);
}

#[test]
fn around_the_isp_tile() {
    for s in [ISP_TILE - 1, ISP_TILE, ISP_TILE + 1] {
        check_size(s, s);
    }
}

#[test]
fn around_the_zls_tile() {
    for s in [ZLS - 1, ZLS, ZLS + 1] {
        check_size(s, s);
    }
}

#[test]
fn non_square_sizes() {
    for (w, h) in [(17, 33), (33, 17), (1, 17), (17, 1)] {
        check_size(w, h);
    }
}
