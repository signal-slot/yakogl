// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 `shadow` (and `refract`) on the PowerVR: the depth texture the
//! scene samples must hold the depth its own frame rendered
//! (tools/hostharness/glmark2/FINDINGS.md, entries 23 and 24: "the backend
//! did not resolve its depth (-16)" once a frame).
//!
//! The GL calls are glmark2's `SceneShadow` frame, in its order
//! (scene-shadow.cpp at the pinned 22c527cb: `MainLoop` clears the canvas,
//! `DepthRenderTarget::enable` binds the FBO, re-attaches the depth texture,
//! masks colour and clears depth, the horse is drawn, `disable` binds the
//! canvas back, and the ground samples the depth texture). They run against
//! the REAL PowerVR backend (`yakogl_backend_pvr`), on a platform written
//! here: GPU memory is host memory, every submit completes at once (as
//! src/osal/linux.c, the host harness's OSAL) unless a test holds
//! completion back, and the "GPU" stores a depth into the depth/stencil
//! buffer of every kick whose ZLSCTL stores it, different for every kick, so
//! a texel says which kick's depth it holds. The PowerVR's command
//! generators are C (driver/src/pvr); they are stand-ins here that write
//! words of the right count, as the backend's own unit tests do
//! (src/pvr/backend_tests.rs).
//!
//! tests/depth_resolve_pvr.rs runs the same sequence on a stand-in back end;
//! this file runs it on the real one.
//!
//! What was measured before the fix (item pvr-depth-resolve; the diagnosis
//! is in tools/hostharness/glmark2/FINDINGS.md under entries 23-24; the line
//! numbers are those of that time):
//!
//! * The depth draw stamps the texture with `backend::kick_id()` in
//!   `build_common` (src/gl/draw.rs:2011, `stamp_depth_target`, ->
//!   src/gl/framebuffer.rs:229) BEFORE `backend::draw` (src/gl/draw.rs:402).
//!   The canvas clear has left a frame open on the window, so the stamp is
//!   that frame's kick K; the PowerVR's `draw_one` then ends that frame for
//!   the target change (src/pvr/backend.rs:4000) and records the depth draw
//!   in kick K+1.
//! * At the ground draw `resolve_depth` (src/gl/texture.rs:654) asks
//!   `kick_pending(K)`: K completed, so nothing is submitted, and
//!   `pvr_backend_depth_resolve` finds K+1 still open on that depth buffer:
//!   -EBUSY (src/pvr/backend.rs:3361). `set_depth_resolved(K)` then marks
//!   the texels current (src/gl/texture.rs:678), so the ground samples
//!   whatever the texels held: the previous frame's depth, or the
//!   allocation's contents on frame 1.
//! * With K still in flight (the board, when the canvas clear's kick has
//!   not finished yet) the same frame resolves: the wait submits K+1 too.
//!   Without the open canvas frame the stamp is right and it resolves. The
//!   object id, the depth plane, the level and the re-attachment every frame
//!   are not the cause (the tests below hold them).
//!
//! The fix (bd8df7a4): the core moves a draw's stamps to `kick_id()` once
//! the back end has recorded the draw (`draw::draw_stamp`,
//! `draw::restamp_draw`), so the stamp is K+1, pending at the ground draw,
//! and `resolve_depth` submits and waits for it. The two reproducers are
//! regression guards now.
//!
//! The last test runs the other way: depth the CPU writes into the texture
//! (glTexSubImage2D) is what the next depth pass's kick loads
//! (`GlesDraw::depth_texels`, the backend's `ds_seed_depth`).
//!
//! Run: `cargo test --no-default-features --features host-test,gpu-powervr
//! --test depth_resolve_pvr_backend`.

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
        eot_doutu_src0_n: [0, 0x1001, 0x1002, 0x1003],
        event_pixel_pds_info_n: [0, 0x61, 0x62, 0x63],
        eot_doutu_src0_tb_n: [0, 0x2001, 0x2002, 0x2003],
        event_pixel_pds_info_tb_n: [0, 0x71, 0x72, 0x73],
        tile_buffer_gpu: [HEAP_GENERAL_BASE + 0x3E00_0000, HEAP_GENERAL_BASE + 0x3E00_8000],
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

/// glmark2's canvas is 800x600 and its depth target twice that; a smaller
/// pair of the same shape keeps the test quick.
const CANVAS: (i32, i32) = (40, 30);
const DEPTH: (i32, i32) = (80, 60);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAMS: (GLuint, GLuint) = (0, 0);

/// The one context and backend of the file, set up once; the tests take
/// turns. Each starts with every kick submitted and completed.
fn rig(hold: bool) -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
            /* the depth pass's program draws no texture (glmark2's depth
             * shaders sample nothing); the ground's samples unit 0 */
            PROGRAMS = (program("passthrough.vert", "uniform_color.frag"), program("uv.vert", "texture.frag"));
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    with_platform(|p| {
        p.completed_to = p.fence;
        p.hold = hold;
        p.log.clear();
    });
    g
}

fn ctx() -> &'static GlContext {
    // SAFETY: set once in `rig`, never freed; read under the lock.
    unsafe { &*CTX }
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

/* a triangle at z = 0 covering the target's lower-left half, (x, y, z, w) and
 * (u, v) per vertex */
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

/// glmark2's `DepthRenderTarget::setup`: a GL_DEPTH_COMPONENT texture twice
/// the canvas, nearest, clamped, and an FBO with it at the depth attachment
/// only.
fn depth_target() -> (GLuint, GLuint) {
    let (mut tex, mut fbo) = (0, 0);
    // SAFETY: room for one name each; a null image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
            DEPTH.0,
            DEPTH.1,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glEnable(GL_DEPTH_TEST);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    (tex, fbo)
}

fn release(tex: GLuint, fbo: GLuint) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteTextures(1, &tex);
        glFinish();
    }
}

/// `MainLoop::draw`'s canvas clear (colour and depth of the window).
fn canvas_clear() {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
}

/// `DepthRenderTarget::enable` and the horse drawn into it, then `disable`.
/// Its glClear emits a canvas clear still pending (`pending_clear_for`),
/// which opens a frame on the window: the frame the depth draw then ends.
fn depth_pass(tex: GLuint, fbo: GLuint) {
    let (depth_prog, _) = unsafe { PROGRAMS };
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(depth_prog);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        glViewport(0, 0, DEPTH.0, DEPTH.1);
        glColorMask(0, 0, 0, 0);
        glClear(GL_DEPTH_BUFFER_BIT);
    }
    draw(depth_prog);
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glColorMask(1, 1, 1, 1);
    }
}

/// `GroundRenderer::draw`: sample the depth texture on unit 0.
fn ground(tex: GLuint) {
    let (_, ground_prog) = unsafe { PROGRAMS };
    // SAFETY: plain GL calls.
    unsafe {
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, tex);
    }
    draw(ground_prog);
    // SAFETY: plain GL call.
    unsafe { glBindTexture(GL_TEXTURE_2D, 0) };
}

/// Texel (x, y) of level 0 of `tex`, as the bits the GPU samples.
fn texel(tex: GLuint, x: usize, y: usize) -> u32 {
    let t = &ctx().objs_ref().textures[tex as usize];
    let pitch = t.level_pitch(t.width) as usize;
    // SAFETY: level 0 holds `height` rows of `pitch` R32F texels.
    unsafe { (t.mem.cpu as *const u32).add(t.level_off[0] as usize / 4 + y * pitch + x).read() }
}

/// The R32F texel a resolve of `word` writes.
fn resolved(word: u32) -> u32 {
    yakogl::ds_retention::unorm24_depth(word).to_bits()
}

/// The depth plane of texture `tex` as the backend holds it, resolved into
/// `w` x `h` floats by `pvr_backend_depth_resolve` (the CPU conversion of
/// the buffer the draw sampled); `None` when the backend keeps nothing for
/// it (1) or the frame still draws into it (-EBUSY).
fn buffer_depth(tex: GLuint, w: u32, h: u32) -> Option<Vec<u32>> {
    let serial = ctx().objs_ref().textures[tex as usize].serial;
    let mut out = vec![0f32; (w * h) as usize];
    // SAFETY: `out` holds h rows of w floats.
    let rc = unsafe { pvr_backend_depth_resolve(context::ds_object_id(serial, true), out.as_mut_ptr(), w, h, w) };
    (rc == 0).then(|| out.iter().map(|f| f.to_bits()).collect())
}

/// Does texture `tex` still have no R32F storage (`storage_deferred`: the
/// GL core allocated none, so nothing was resolved into it)?
fn storage_deferred(tex: GLuint) -> bool {
    ctx().objs_ref().textures[tex as usize].storage_deferred
}

/// What one frame left: the depth the frame's depth pass stored, the depth
/// the buffer the ground sampled holds, whether the ground read the buffer
/// (the backend's count of such draws moved by one), and the library's
/// depth-resolve lines.
#[derive(Debug)]
struct Frame {
    /// (the texture's render stamp, the kick the backend was recording) just
    /// after the depth draw.
    stamp: (u32, u32),
    stored: u32,
    sampled: u32,
    direct: bool,
    lines: Vec<String>,
}

/// glmark2's shadow frame: canvas clear, depth pass, ground (sampling), the
/// horse in the canvas, the frame's end. `flush_between` submits the depth
/// pass before the ground draw (a glFlush the application does not make).
fn shadow_frame(tex: GLuint, fbo: GLuint, flush_between: bool) -> Frame {
    with_platform(|p| p.log.clear());
    canvas_clear();
    depth_pass(tex, fbo);
    let stamp = (texture_target_use(ctx(), tex as usize), pvr_backend_kick_id());
    if flush_between {
        // SAFETY: plain GL call.
        unsafe { glFlush() };
    }
    let direct_before = pvr_backend_depth_sample_stats();
    ground(tex);
    let direct_after = pvr_backend_depth_sample_stats();
    draw(unsafe { PROGRAMS.0 }); /* the horse, in the canvas */
    // SAFETY: plain GL call (eglSwapBuffers' submit).
    unsafe { glFlush() };
    /* the depth pass is the kick the stamp was taken in (stamp right) or the
     * one after it (stamp a kick early): the later one that stored depth */
    let stored = with_platform(|p| {
        let k = stamp.1 as i64;
        p.stored.get(&k).copied().or_else(|| p.stored.get(&(k + 1)).copied())
    });
    let sampled =
        buffer_depth(tex, DEPTH.0 as u32, DEPTH.1 as u32).expect("the backend keeps no depth for the texture");
    let lines = logs().into_iter().filter(|l| l.contains("depth") && !l.contains("no CPU resolve")).collect();
    assert_eq!(direct_after.1, direct_before.1, "a binding named a buffer that was not there: {:?}", logs());
    Frame {
        stamp,
        stored: stored.expect("the depth pass stored no depth"),
        sampled: sampled[DEPTH.0 as usize + 1],
        direct: direct_after.0 == direct_before.0 + 1,
        lines,
    }
}

/* ---- the tests --------------------------------------------------------------- */

/// glmark2 shadow, three frames as the host harness runs it, on a GPU that
/// finishes every kick at once (linux.c; on the board: whenever the canvas
/// clear's kick has completed by the ground draw): each frame's ground must
/// sample that frame's depth. Since the TPU reads the buffer (X8U24,
/// `pvr_depth_sampled`) the ground samples the buffer itself: no resolve
/// line, the backend's count of such draws moves by one a frame, the
/// buffer holds the frame's depth, and the texture's R32F storage is never
/// allocated (docs/notes/pvr-depth-sample.md).
#[test]
fn glmark2_shadow_samples_the_depth_its_frame_rendered() {
    let _g = rig(false);
    let (tex, fbo) = depth_target();
    assert!(storage_deferred(tex), "a depth texture laid out with no pixels has no storage yet");
    let frames: Vec<Frame> = (0..3).map(|_| shadow_frame(tex, fbo, false)).collect();
    let deferred = storage_deferred(tex);
    release(tex, fbo);
    for (i, f) in frames.iter().enumerate() {
        let (stamp, recording) = f.stamp;
        assert!(f.lines.is_empty(), "frame {i}: stamp {stamp}, recording {recording}: {f:?}");
        assert!(f.direct, "frame {i}: the ground did not sample the depth buffer: {f:?}");
        assert_eq!(f.sampled, resolved(f.stored), "frame {i} sampled other depth than it rendered: {f:?}");
    }
    assert!(deferred, "no CPU read of the texels: the R32F storage stays unallocated");
    let (direct, refused) = pvr_backend_depth_sample_stats();
    assert!(direct >= 3 && refused == 0, "the exit line's counts: {direct} sampled from the buffer, {refused} refused");
}

/// A CPU write of the texels after a frame (glTexSubImage2D of one texel)
/// allocates the storage and resolves the rendered depth into it first, so
/// the texels hold what the ground sampled, and the next frame's ground
/// samples the buffer again.
#[test]
fn a_cpu_write_after_the_ground_resolves_the_sampled_depth_into_the_texels() {
    let _g = rig(false);
    let (tex, fbo) = depth_target();
    let f = shadow_frame(tex, fbo, false);
    assert!(f.direct && f.lines.is_empty(), "{f:?}");
    let one = [0u32];
    // SAFETY: plain GL calls; `one` is a 1x1 GL_UNSIGNED_INT depth image.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            0,
            0,
            1,
            1,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            one.as_ptr() as *const c_void,
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    assert!(!storage_deferred(tex), "the write allocated the storage");
    assert_eq!(texel(tex, 1, 1), resolved(f.stored), "the rendered depth was resolved into the texels first");
    assert_eq!(texel(tex, 0, 0), 0, "then the texel was written");
    let g = shadow_frame(tex, fbo, false);
    release(tex, fbo);
    assert!(g.direct && g.lines.is_empty(), "{g:?}");
    assert_eq!(g.sampled, resolved(g.stored), "{g:?}");
}

/// The mechanism of the above, pinned on its own: after the depth draw the
/// texture's render stamp must name a kick that has not completed (the one
/// the depth draw is in), so that `resolve_depth` waits for it.
#[test]
fn the_depth_draws_stamp_names_a_kick_still_to_complete() {
    let _g = rig(false);
    let (tex, fbo) = depth_target();
    canvas_clear();
    depth_pass(tex, fbo);
    let stamp = texture_target_use(ctx(), tex as usize);
    let (recording, pending) = (pvr_backend_kick_id(), pvr_backend_kick_pending(stamp));
    release(tex, fbo);
    assert!(pending, "stamp {stamp} has completed while kick {recording} still draws the depth: {:?}", logs());
}

/// The same frames with the GPU still busy with the canvas clear's kick at
/// the ground draw (the board, some of the time): `depth_for_draw` submits
/// the depth pass (no wait), the backend's record of what it stored is
/// complete, and every frame's ground samples the buffer holding its own
/// depth. So the object id the GL core asks about (`ds_object_id(serial,
/// true)`) is the one the backend keeps the plane under, the plane is live
/// after the kick, level 0 is the level rendered, and the re-attachment
/// every frame changes nothing.
#[test]
fn with_the_canvas_kick_in_flight_every_frame_samples_its_own_depth() {
    let _g = rig(true);
    let (tex, fbo) = depth_target();
    let frames: Vec<Frame> = (0..3).map(|_| shadow_frame(tex, fbo, false)).collect();
    release(tex, fbo);
    for (i, f) in frames.iter().enumerate() {
        assert!(f.lines.is_empty() && f.direct, "frame {i}: {f:?}");
        assert_eq!(f.sampled, resolved(f.stored), "frame {i}: {f:?}");
    }
    assert_ne!(frames[0].stored, frames[2].stored, "every frame stores a depth of its own");
}

/// The depth pass submitted before the ground draw: every frame samples
/// the buffer.
#[test]
fn a_depth_pass_submitted_before_the_ground_samples_the_buffer() {
    let _g = rig(false);
    let (tex, fbo) = depth_target();
    let frames: Vec<Frame> = (0..3).map(|_| shadow_frame(tex, fbo, true)).collect();
    release(tex, fbo);
    for (i, f) in frames.iter().enumerate() {
        assert!(f.lines.is_empty() && f.direct, "frame {i}: {f:?}");
        assert_eq!(f.sampled, resolved(f.stored), "frame {i}: {f:?}");
    }
}

/// No frame open when the depth pass starts (no canvas clear first): the
/// stamp is the depth pass's own kick, and the ground samples its buffer.
#[test]
fn a_depth_pass_into_no_open_frame_samples_the_buffer() {
    let _g = rig(false);
    let (tex, fbo) = depth_target();
    for i in 0..3 {
        with_platform(|p| p.log.clear());
        depth_pass(tex, fbo);
        let stamp = texture_target_use(ctx(), tex as usize);
        assert_eq!(stamp, pvr_backend_kick_id(), "frame {i}: the stamp is the depth pass's kick");
        let before = pvr_backend_depth_sample_stats();
        ground(tex);
        let after = pvr_backend_depth_sample_stats();
        // SAFETY: plain GL call.
        unsafe { glFlush() };
        let stored = with_platform(|p| p.stored.get(&(stamp as i64)).copied()).expect("no depth stored");
        let sampled = buffer_depth(tex, DEPTH.0 as u32, DEPTH.1 as u32).expect("no depth kept")[DEPTH.0 as usize + 1];
        let lines: Vec<String> =
            logs().into_iter().filter(|l| l.contains("depth") && !l.contains("no CPU resolve")).collect();
        assert!(lines.is_empty(), "frame {i}: {lines:?}");
        assert_eq!((after.0, after.1), (before.0 + 1, before.1), "frame {i}: the ground samples the buffer");
        assert_eq!(sampled, resolved(stored), "frame {i}");
    }
    release(tex, fbo);
}

/// The other way round: a depth pass renders the texture, glTexSubImage2D
/// writes a box of it, and the next depth pass draws with GL_LESS and no
/// clear. Its kick loads the depth buffer (ZLSCTL zloaden), and what it
/// loads is the texture's texels as the CPU holds them after the write
/// (the rendered depth resolved in, the uploaded box over it), packed as
/// the ISP's 24BITINT depth at the ZLS-aligned width: the draw tests
/// against the uploaded depth, not the depth the first pass stored.
///
/// Revert check: without `GlesDraw::depth_texels` the kick loads
/// `depth_word` of the first pass everywhere, the box included.
#[test]
fn a_gl_less_depth_pass_after_gltexsubimage2d_loads_the_uploaded_depth() {
    let _g = rig(false);
    let (tex, fbo) = depth_target();
    depth_pass(tex, fbo);
    // SAFETY: plain GL call.
    unsafe { glFlush() };
    let boxed = [0u32, 0x8000_0000, 0xffff_ffff, 0x1234_5678];
    // SAFETY: plain GL calls; `boxed` is a 2x2 GL_UNSIGNED_INT depth image.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            3,
            2,
            2,
            2,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            boxed.as_ptr() as *const c_void,
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    assert_eq!((texel(tex, 4, 2), texel(tex, 3, 3)), (0.5f32.to_bits(), 1.0f32.to_bits()), "the box is in the texels");
    let (depth_prog, _) = unsafe { PROGRAMS };
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glViewport(0, 0, DEPTH.0, DEPTH.1);
        glColorMask(0, 0, 0, 0);
        glDepthFunc(GL_LESS);
    }
    draw(depth_prog);
    // SAFETY: plain GL calls.
    unsafe {
        glFlush();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glColorMask(1, 1, 1, 1);
    }
    let (fence, loaded) = with_platform(|p| (p.fence, p.loads.get(&p.fence).cloned()));
    let loaded = loaded.unwrap_or_else(|| panic!("kick {fence} (the second pass) loads no depth: {:?}", logs()));
    /* 32-pixel ZLS tiles: 80 wide is 96 words a row */
    let aw = (DEPTH.0 as usize).div_ceil(32) * 32;
    for y in 0..DEPTH.1 as usize {
        for x in 0..DEPTH.0 as usize {
            let want = yakogl::ds_retention::depth_unorm24(f32::from_bits(texel(tex, x, y)));
            assert_eq!(loaded[y * aw + x] & 0xff_ffff, want, "({x}, {y}): the kick starts from the texels");
        }
    }
    release(tex, fbo);
}
