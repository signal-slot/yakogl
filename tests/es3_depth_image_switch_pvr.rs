// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth plane of a cube or array depth texture when draws switch faces
//! or layers, over the REAL PowerVR host back end (plan item
//! switch-test-pvr; docs/notes/blit-depth-layers.md, "What this means today"
//! 1 and 2, and design (b)). The PowerVR twin of
//! tests/es3_depth_image_switch.rs (the same scenarios on a stand-in back
//! end); the platform rig is copied from tests/es3_blit_ds_layers_pvr.rs
//! (host memory for the GPU's, every submit complete at once, stand-ins for
//! the C command generators). The GL core, `pvr_backend_depth_resolve`,
//! `pvr_backend_ds_write` and the draw's seeding of the depth buffer
//! (`ds_seed_depth`, src/pvr/backend.rs) are the real ones; only the GPU is
//! faked.
//!
//! The PowerVR keeps ONE depth buffer per texture, keyed by the texture's
//! `ds_object_id` with no face, layer or level (blit-depth-layers.md (a)).
//! Today `texture::resolve_depth` writes that plane into face/layer 0 of the
//! level, and `texture::depth_seed` seeds only from face/layer 0. Design (b)
//! records the (level, layer) the plane holds (`depth_image_switch`): a draw
//! into another face first resolves the old face into its own rows, then
//! seeds the new face from its rows.
//!
//! Two kinds of test:
//! - plain tests pin what must not change: 2D depth textures resolve and
//!   seed as before, face/layer 0 of a layered texture resolves into its own
//!   rows, and a whole-depth clear skips the seed;
//! - `#[ignore = "needs gl-ds-layers-core"]` tests hold the design (the item
//!   gl-ds-layers-core removes the ignores): draw face 0 then face 1 (array
//!   layers 0 then 2) and face 0 gets the back end's depth, the new image is
//!   seeded from its own rows, a later read sees each image's own depth, and
//!   a whole-plane clear on the switch draw hands no seed. Run them with
//!   `-- --ignored`; they fail today.
//!
//! How the planes are set and seen (HOST-ONLY: no board, the GPU's own depth
//! writes are not seen):
//! - "what the back end holds" is set through `pvr_backend_ds_write(id, ...)`
//!   by the texture's id, to a distinct word per pixel (`plane_word`, a new
//!   set per write), and the rows the GL core resolved are compared with it:
//!   a depth is the 24-bit word n written as `n / 16777215`, and every
//!   compare is on the integer n (the model of the expected values is
//!   `plane_word` / `image_word`, written apart from the code);
//! - "the seed" is what the depth buffer holds when the draw's kick reaches
//!   the (fake) GPU: `p_submit` snapshots the depth words of the buffer it is
//!   about to store into (`Platform::snaps`) before it overwrites them, so a
//!   draw's seed, or the plane left in place when no seed was handed over, is
//!   read from the snapshot of that draw's kick;
//! - a CPU read of a layered texture is a one-texel glTexSubImage of an
//!   image with the value already there: it settles (resolves) the texture
//!   and changes nothing the tests look at.
//!
//! Every draw is the depth-tested triangle of the rig into a depth-only
//! framebuffer, ended by glFinish, so each draw is one kick (one snapshot).
//!
//! Left out: the event order of the stand-in test ("the old image resolves
//! BEFORE the new draw reaches the back end") and its counted resolves: the
//! real back end has no event log to read. The order is shown instead by the
//! switch draw's snapshot (the new image's rows, which the seed puts into the
//! buffer, and the plane words of the old image, resolved out of it first),
//! and every write of the plane uses a distinct set of words.
//!
//! ```text
//! cargo test --features host-test,gpu-powervr --test es3_depth_image_switch_pvr
//! ```
//!
//! That command builds the file EMPTY (0 tests): the gate below needs
//! `shaderc-online` too, as in tests/es3_blit_ds_layers_pvr.rs. The tests run with
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_depth_image_switch_pvr
//! ```
//!
//! (add `-- --ignored` for the design tests). Not built with `report-es2`
//! (cube/array depth and glBlitFramebuffer are ES 3.0 only).

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
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
use yakogl::backend;

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
    /// The depth words (24 bits) of the buffer each storing kick found, as
    /// the kick reached the "GPU": `SNAP_ROWS` rows of `AW` words.
    snaps: Vec<(i64, Vec<u32>)>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The ZLS-tile-aligned row width of the sources' buffers (32 pixels wide
/// = one tile, `ZLS_TILE` in src/pvr/backend.rs): the layout the ISP stores
/// at and `pvr_backend_depth_resolve` reads.
const AW: usize = 32;
/// The rows of a buffer a snapshot keeps (the tests' textures are this tall).
const SNAP_ROWS: usize = 8;

/// The D24S8 word the "GPU" stores at (x, y) in the job of fence `fence`: a
/// depth that differs per texel and per job in bits 23..0, and a stencil
/// byte (bits 31..24) the depth resolve must ignore.
fn depth_word(fence: i64, x: u32, y: u32) -> u32 {
    let d = (x * 7 + y * 13 + fence as u32 * 1009) & 0x00ff_ffff;
    d | 0x3c00_0000
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

/// The "GPU": a kick that stores depth leaves `depth_word` in every word of
/// its depth/stencil buffer, rows `AW` words wide.
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
        let stores = zs != 0 && k.frag.isp_zlsctl & ZLSCTL_FORCEZSTORE != 0;
        let Some((&base, a)) = p.allocs.range_mut(..=zs).next_back().filter(|_| stores) else { return fence };
        let off = (zs - base) as usize;
        if off < a.size {
            let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
            // SAFETY: the words from `zs` to the allocation's end.
            let words = unsafe { core::slice::from_raw_parts_mut(first, (a.size - off) / 4) };
            let n = (AW * SNAP_ROWS).min(words.len());
            p.snaps.push((fence, words[..n].iter().map(|w| w & 0x00ff_ffff).collect()));
            for (i, w) in words.iter_mut().enumerate() {
                *w = depth_word(fence, (i % AW) as u32, (i / AW) as u32);
            }
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
/// A 24-bit depth word's denominator (`n / 16777215`).
const MAXD: f32 = 16_777_215.0;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The bound framebuffer's depth/stencil cleared, then a depth-tested draw
/// whose kick the fake GPU answers by storing `depth_word`: the buffer the
/// back end keeps under the target's id is live afterwards.
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D, glTexSubImage2D, glTexSubImage3D,
};

const D: u32 = backend::DS_PLANE_DEPTH;

/* ---- objects ------------------------------------------------------------------- */

const SIDE: u32 = 8;
const D32F: (GLenum, GLenum, GLenum) = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

#[derive(Clone, Copy, Debug)]
enum Layered {
    Cube,
    Array,
}

impl Layered {
    fn images(self) -> u32 {
        match self {
            Layered::Cube => 6,
            Layered::Array => 3,
        }
    }
}

/* The model, written apart from the code: 24-bit words n, exact in f32. */

/// The word the test puts into image `k` of a texture at (x, y): distinct per
/// image and texel, below 2^22.
fn image_word(k: u32, x: u32, y: u32) -> u32 {
    100_000 + k * 400_000 + (y * SIDE + x) * 811
}

/// The word `set` of the back end's plane holds at (x, y) after the test
/// wrote it through `ds_write`: distinct per set and texel, above every
/// `image_word`, below 2^24.
fn plane_word(set: u32, x: u32, y: u32) -> u32 {
    3_000_000 + set * 1_500_000 + (y * SIDE + x) * 313
}

fn rows(f: impl Fn(u32, u32) -> u32) -> Vec<u32> {
    (0..SIDE).flat_map(|y| (0..SIDE).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

fn texel_ptr(tex: GLuint, layer: u32, x: u32, y: u32) -> *mut f32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert!(x < t.width && y < t.height && layer < t.layers.max(1) as u32);
    let row = layer as usize * t.height as usize + y as usize;
    let off = t.level_off[0] as usize + (row * t.level_pitch_at(0) as usize + x as usize) * 4;
    assert!(off + 4 <= t.mem.size);
    (t.mem.cpu as *mut u8).wrapping_add(off) as *mut f32
}

/// The word n of the R32F texel (x, y) of image `layer` of level 0, as the
/// GL core stores it (image k's row y at row k * height + y).
fn texel_n(tex: GLuint, layer: u32, x: u32, y: u32) -> u32 {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    (unsafe { texel_ptr(tex, layer, x, y).read() } * MAXD).round() as u32
}

/// Image `layer` of `tex` as the CPU holds it, as words.
fn image(tex: GLuint, layer: u32) -> Vec<u32> {
    rows(|x, y| texel_n(tex, layer, x, y))
}

/// A cube map or 3-layer array of 8x8 depth, image k holding `image_word(k)`.
fn layered_texture(kind: Layered) -> GLuint {
    let mut tex = 0;
    let s = SIDE as i32;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                for face in FACES {
                    glTexImage2D(face, 0, D32F.0 as GLint, s, s, 0, D32F.1, D32F.2, core::ptr::null());
                }
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                let n = kind.images() as i32;
                glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, D32F.0 as GLint, s, s, n, 0, D32F.1, D32F.2, core::ptr::null());
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "a {kind:?} depth texture");
    for k in 0..kind.images() {
        for y in 0..SIDE {
            for x in 0..SIDE {
                // SAFETY: as in `texel_n`.
                unsafe { texel_ptr(tex, k, x, y).write(image_word(k, x, y) as f32 / MAXD) };
            }
        }
    }
    tex
}

/// A `SIDE` x `SIDE` 2D depth texture holding `image_word(0)`.
fn depth_texture_2d() -> GLuint {
    let mut tex = 0;
    let zero = vec![0u32; (SIDE * SIDE) as usize];
    // SAFETY: room for one name; `SIDE` x `SIDE` words of pixels (the GL core
    // allocates the storage of a 2D texture for them).
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        let s = SIDE as i32;
        glTexImage2D(GL_TEXTURE_2D, 0, D32F.0 as GLint, s, s, 0, D32F.1, D32F.2, zero.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    for y in 0..SIDE {
        for x in 0..SIDE {
            // SAFETY: as in `texel_n`.
            unsafe { texel_ptr(tex, 0, x, y).write(image_word(0, x, y) as f32 / MAXD) };
        }
    }
    tex
}

/// The id the back end keeps depth texture `tex` under.
fn tex_id(tex: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    context::ds_object_id(c.texture_object(slot).expect("a texture object").serial, true)
}

/// A depth-only framebuffer with image `k` of `tex` at the depth attachment
/// (the 2D texture itself for `None`); left bound.
fn target(tex: GLuint, kind: Option<Layered>, k: u32) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            None => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0),
            Some(Layered::Cube) => {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, FACES[k as usize], tex, 0)
            }
            Some(Layered::Array) => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, tex, 0, k as GLint),
        }
        assert_eq!(err(), GL_NO_ERROR, "an image at the depth attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    fb
}

/// A depth-tested draw into the bound framebuffer (`clear`: after a clear of
/// the whole depth plane), submitted and completed. One kick, so one new
/// snapshot.
fn draw(clear: bool) {
    // SAFETY: plain GL calls; `PROGRAM` is set once in `rig`.
    unsafe {
        glViewport(0, 0, SIDE as i32, SIDE as i32);
        if clear {
            glClearDepthf(1.0);
            glClear(GL_DEPTH_BUFFER_BIT);
        }
        glEnable(GL_DEPTH_TEST);
        draw_tri(PROGRAM);
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

/// What the plane held when kick `n` (0-based, since the test began) reached
/// the GPU, as the words of the `SIDE` x `SIDE` picture.
fn snapshot(n: usize) -> Vec<u32> {
    with_platform(|p| {
        let (_, s) = p.snaps.get(n).unwrap_or_else(|| panic!("no kick {n}: {} so far, {:?}", p.snaps.len(), p.log));
        rows(|x, y| s[y as usize * AW + x as usize])
    })
}

fn kicks() -> usize {
    with_platform(|p| p.snaps.len())
}

/// The words the fake GPU stored in the depth buffer when kick `n` ran (the
/// `depth_word` of its fence): what the buffer holds after it, and what the
/// next kick finds if nothing is written in between.
fn stored_by(n: usize) -> Vec<u32> {
    let fence = with_platform(|p| p.snaps[n].0);
    rows(|x, y| depth_word(fence, x, y) & 0x00ff_ffff)
}

/// The back end's plane of object `id` set to set `set` of `plane_word`,
/// through its own `ds_write`.
fn set_plane(id: u32, set: u32) {
    let v: Vec<f32> = rows(|x, y| plane_word(set, x, y)).into_iter().map(|n| n as f32 / MAXD).collect();
    // SAFETY: `SIDE` rows of `SIDE` floats.
    let r = unsafe { pvr_backend_ds_write(id, D, v.as_ptr() as *const c_void, 0, 0, SIDE, SIDE, SIDE) };
    assert_eq!(r, 0, "ds_write of 0x{id:x}: {:?}", logs());
}

fn planes(set: u32) -> Vec<u32> {
    rows(|x, y| plane_word(set, x, y))
}

fn seeded(k: u32) -> Vec<u32> {
    rows(|x, y| image_word(k, x, y))
}

/// A CPU read of layered texture `tex`: a one-texel glTexSubImage of image
/// `last` with the value it holds, which settles the texture first. The
/// settle resolves the plane into the recorded image and the write then puts
/// back the value read BEFORE the resolve, so `last` must not be the image
/// the plane holds (the last one drawn): it would overwrite the resolve with
/// the stale value.
fn settle_at(tex: GLuint, kind: Layered, last: u32) {
    let v = [texel_n(tex, last, 0, 0) as f32 / MAXD];
    // SAFETY: one float of pixels, a 1x1 sub-image inside the level.
    unsafe {
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                glTexSubImage2D(FACES[last as usize], 0, 0, 0, 1, 1, D32F.1, D32F.2, v.as_ptr() as *const c_void);
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                let z = last as GLint;
                glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, z, 1, 1, 1, D32F.1, D32F.2, v.as_ptr() as *const c_void);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

fn release(fbs: &[GLuint], texs: &[GLuint]) {
    // SAFETY: names these tests made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
    }
}

/// 0.25 as the 24-bit word a texel written with it reads back as.
const QUARTER: u32 = 4_194_304;

/// A one-texel CPU write of 0.25 at (1, 1) of the bound 2D depth texture.
fn write_quarter_2d(tex: GLuint) {
    let one = [0.25f32];
    // SAFETY: one float, a 1x1 sub-image inside the level.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

/* ---- what must not change: 2D, image 0, whole-depth clears --------------------- */

/// A 2D depth texture drawn into and read by the CPU: the rows hold what the
/// back end held, all of them (the model: the words `ds_write` put there).
#[test]
fn a_2d_depth_texture_resolves_into_its_rows() {
    let _g = rig();
    let tex = depth_texture_2d();
    let id = tex_id(tex);
    let fb = target(tex, None, 0);
    draw(false);
    set_plane(id, 0);
    write_quarter_2d(tex);
    let mut want = planes(0);
    want[(SIDE + 1) as usize] = QUARTER;
    assert_eq!(image(tex, 0), want, "the texture's rows: the plane, but for the texel the CPU wrote");
    release(&[fb], &[tex]);
}

/// Texels the CPU wrote reach the back end with the first draw after them,
/// the whole level; the draw after that carries none.
#[test]
fn a_2d_depth_texture_seeds_from_its_rows() {
    let _g = rig();
    let tex = depth_texture_2d();
    let id = tex_id(tex);
    let fb = target(tex, None, 0);
    draw(false);
    set_plane(id, 0);
    write_quarter_2d(tex);
    let mut want = planes(0);
    want[(SIDE + 1) as usize] = QUARTER;
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    let k = kicks();
    draw(false);
    draw(false);
    assert_eq!(kicks(), k + 2, "one kick a draw: {:?}", logs());
    assert_eq!(snapshot(k), want, "the first draw found the rows in the buffer");
    assert_eq!(snapshot(k + 1), stored_by(k), "the second draw found what the first stored: no seed");
    release(&[fb], &[tex]);
}

/// A whole-depth clear replaces what the rows held: the clear draw is handed
/// no seed (it finds the plane as it was).
#[test]
fn a_whole_depth_clear_skips_the_seed() {
    let _g = rig();
    let tex = depth_texture_2d();
    let id = tex_id(tex);
    let fb = target(tex, None, 0);
    draw(false);
    set_plane(id, 0);
    write_quarter_2d(tex);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    let k = kicks();
    draw(true);
    assert_eq!(kicks(), k + 1, "{:?}", logs());
    assert_eq!(snapshot(k), planes(0), "the clear draw found the plane, not the rows the CPU wrote");
    release(&[fb], &[tex]);
}

/// Image 0 of a cube or array is the one image today's plane lands in: a draw
/// into it, then a CPU read, writes the plane into image 0's rows and leaves
/// every other image's rows alone.
#[test]
fn image_0_of_a_layered_texture_resolves_into_its_own_rows() {
    for kind in [Layered::Cube, Layered::Array] {
        let _g = rig();
        let tex = layered_texture(kind);
        let id = tex_id(tex);
        let fb = target(tex, Some(kind), 0);
        draw(false);
        set_plane(id, 0);
        settle_at(tex, kind, kind.images() - 1);
        assert_eq!(image(tex, 0), planes(0), "{kind:?}: image 0");
        for k in 1..kind.images() {
            assert_eq!(image(tex, k), seeded(k), "{kind:?}: image {k} untouched");
        }
        release(&[fb], &[tex]);
    }
}

/// A whole-depth clear of image 0 of a layered texture hands no seed, as in 2D.
#[test]
fn a_whole_depth_clear_of_image_0_skips_the_seed() {
    for kind in [Layered::Cube, Layered::Array] {
        let _g = rig();
        let tex = layered_texture(kind);
        let id = tex_id(tex);
        let fb = target(tex, Some(kind), 0);
        draw(false);
        set_plane(id, 0);
        // The CPU writes image 0 (a sub-image, after the settle put the plane
        // there): a plain draw would seed those rows, a clearing one must not.
        settle_at(tex, kind, kind.images() - 1);
        let one = [0.25f32];
        // SAFETY: one float, a 1x1 sub-image inside the level.
        unsafe {
            match kind {
                Layered::Cube => {
                    glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                    glTexSubImage2D(FACES[0], 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
                }
                Layered::Array => {
                    glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                    glTexSubImage3D(
                        GL_TEXTURE_2D_ARRAY,
                        0,
                        1,
                        1,
                        0,
                        1,
                        1,
                        1,
                        D32F.1,
                        D32F.2,
                        one.as_ptr() as *const c_void,
                    );
                }
            }
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
        }
        assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
        let k = kicks();
        draw(true);
        assert_eq!(kicks(), k + 1, "{kind:?}: {:?}", logs());
        assert_eq!(snapshot(k), planes(0), "{kind:?}: the clear draw found the plane, not the rows");
        release(&[fb], &[tex]);
    }
}

/* ---- the design: the plane follows the image drawn ----------------------------- */

/// Draw image `a`, then image `b` (`clear` makes the switch draw clear the
/// whole depth plane), then read the texture, and check design (b). Between
/// the draws the back end's plane is set (`ds_write`) to set 0, after the
/// switch to set 1:
/// - the switch to `b` resolves the plane into `a`'s rows BEFORE the draw
///   into `b` reaches the GPU, and the draw is seeded from `b`'s rows (the
///   buffer the kick finds holds them; with `clear` it finds the plane as it
///   was, no seed);
/// - a later read resolves into `b`'s rows and leaves `a`'s as the first
///   resolve wrote them, and the other images' rows stay as they were.
fn switch_case(kind: Layered, a: u32, b: u32, clear: bool) {
    let _g = rig();
    let tex = layered_texture(kind);
    let id = tex_id(tex);
    let fa = target(tex, Some(kind), a);
    draw(false);
    set_plane(id, 0);
    let fb = target(tex, Some(kind), b);
    let k = kicks();
    draw(clear);
    let what = format!("{kind:?} {a} -> {b} (clear {clear})");
    assert_eq!(kicks(), k + 1, "{what}: {:?}", logs());
    let seed = if clear { planes(0) } else { seeded(b) };
    assert_eq!(snapshot(k), seed, "{what}: what the switch draw's kick found in the buffer");
    assert_eq!(image(tex, a), planes(0), "{what}: image {a} took the plane");
    assert_eq!(image(tex, b), seeded(b), "{what}: image {b} not yet read");
    set_plane(id, 1);
    // Read through an image outside {a, b}: a write into b would put the
    // stale seed back over the plane the read resolved into b.
    let other = (0..kind.images()).rev().find(|k| *k != a && *k != b).unwrap();
    settle_at(tex, kind, other);
    assert_eq!(image(tex, a), planes(0), "{what}: image {a} keeps its depth");
    assert_eq!(image(tex, b), planes(1), "{what}: image {b} took the plane");
    for k in (0..kind.images()).filter(|k| *k != a && *k != b) {
        assert_eq!(image(tex, k), seeded(k), "{what}: image {k} untouched");
    }
    release(&[fa, fb], &[tex]);
}

/// Cube face +X drawn, then -X: +X's rows get the back end's depth, -X's
/// seed comes from -X's rows, and later each face reads its own depth.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn drawing_face_1_after_face_0_resolves_face_0_and_seeds_face_1() {
    switch_case(Layered::Cube, 0, 1, false);
}

/// The same for array layers 0 and 2 (layer 1 between them is not touched).
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn drawing_layer_2_after_layer_0_resolves_layer_0_and_seeds_layer_2() {
    switch_case(Layered::Array, 0, 2, false);
}

/// A switch draw that clears the whole plane resolves the old image but
/// hands no seed (the new image's rows are overwritten anyway).
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn a_whole_plane_clear_on_the_switch_draw_hands_no_seed() {
    switch_case(Layered::Cube, 0, 1, true);
    switch_case(Layered::Array, 0, 2, true);
}

/// Switching between faces other than 0 (+Y, then -Z): the plane goes to the
/// face drawn, never to face 0.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn a_switch_between_faces_other_than_0_leaves_face_0_alone() {
    switch_case(Layered::Cube, 2, 5, false);
}

/// Switching back: face 0, face 1, face 0 again. The second switch resolves
/// the plane into face 1's rows and seeds face 0 from face 0's rows, which
/// hold the first resolve.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn switching_back_seeds_from_the_rows_the_first_resolve_wrote() {
    let kind = Layered::Cube;
    let _g = rig();
    let tex = layered_texture(kind);
    let id = tex_id(tex);
    let f0 = target(tex, Some(kind), 0);
    draw(false);
    set_plane(id, 0);
    let f1 = target(tex, Some(kind), 1);
    let k = kicks();
    draw(false);
    set_plane(id, 1);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f0) };
    draw(false);
    assert_eq!(kicks(), k + 2, "{:?}", logs());
    assert_eq!(snapshot(k), seeded(1), "face 1 was seeded from its own rows");
    assert_eq!(snapshot(k + 1), planes(0), "face 0 was seeded from the rows the first resolve wrote");
    assert_eq!(image(tex, 0), planes(0));
    assert_eq!(image(tex, 1), planes(1), "the second switch resolved the plane into face 1");
    release(&[f0, f1], &[tex]);
}
