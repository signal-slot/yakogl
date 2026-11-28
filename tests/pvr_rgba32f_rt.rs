// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64): a GL_RGBA32F colour attachment is a 128-bit render
//! target whose four channels are full 32-bit floats (plan #388, issue
//! #313, item fix-rgba32f-render). fix-rgba32f-sample gave the format its
//! own 16-byte texel class for sampling (tests/es3_rgba32f_pvr.rs); this
//! is the render half.
//!
//! The PowerVR draws the surface (`GLES_SURF_RGBA32F`) as four raw 32-bit
//! lanes, RGBA32UI's layout (`pvr::backend::RGBA32F_LANES`): the GL core
//! asks for the RGBA32UI variant of the fragment shader (`ShcIntRt` 4 x 32),
//! whose integer pack moves each 32-bit channel into its output register as
//! it is, so a float lands unconverted; the end-of-tile PBE state is Mesa's
//! for R32G32B32A32_SFLOAT (pack mode F32F32F32F32, an 8-per-channel source,
//! not normalised); a clear's four dwords are the floats' bits; the frame
//! load reads the pixel back raw (RAW128). glReadPixels (GL_RGBA, GL_FLOAT)
//! and glBlitFramebuffer read and copy the 32-bit floats.
//!
//! The GL calls run against the REAL PowerVR back end (`yakogl_backend_pvr`)
//! on the host platform of tests/es3_half_float_rt_pvr.rs (GPU memory is
//! host memory, every submit completes at once; the command generators are
//! stand-ins that record the PBE words). The draw entry of the table is
//! wrapped to record what the GL core hands the back end: the surface, the
//! clear colour and the fragment program, whose USC bytes then run through
//! the host byte model (tests/usc_exec). What is pinned:
//!
//! - RGBA32F renderbuffer and texture: complete, drawn as
//!   `GLES_SURF_RGBA32F`, 16 bytes a pixel;
//! - a draw: PBE pack mode F32F32F32F32, 8-per-channel source, norm clear;
//!   the program is the RGBA32UI variant with four output dwords, and it
//!   writes 1e5, -1e5, 5e4 and 100001 (beyond the half maximum 65504) to
//!   pixel outputs 0..3 as exact float bits;
//! - a clear: the clear colour reaches the back end unclamped and packs to
//!   the floats' bits (`clear_uniform`), and is drawn as the 128-bit target;
//! - glReadPixels of an RGBA32F image as GL_RGBA / GL_FLOAT: the stored
//!   floats exactly; glBlitFramebuffer RGBA32F to RGBA32F (texture and
//!   renderbuffer): the floats exactly;
//! - RGBA32F beside another colour attachment (eight or more output
//!   registers, the tile-buffer spill): refused by name, not drawn;
//! - RGBA32UI and RGBA16F keep their words (the contrast).
//!
//! What it cannot see: the GPU. No pixel is written on the host, so the
//! read-back rows read what an upload put in the store, not the PBE's
//! output; the PBE storing F32F32F32F32 from the four registers is a board
//! result. RGBA32F is a colour target only with GL_EXT_color_buffer_float,
//! which the PowerVR build does not list (RG32F and R11F_G11F_B10F are not
//! drawn yet, `backend::color_buffer_float_capable`): `rig` stands in for a
//! build that does (`framebuffer::HOST_R32F_RT`, and the RGBA32F bit of
//! `backend::HOST_FLOAT_RT_FORMATS`).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_rgba32f_rt
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
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend::{
    self, GlesDraw, YakoglBackend, GLES_SURF_FLOAT_FIRST, GLES_SURF_RGBA16F, GLES_SURF_RGBA32F, GLES_SURF_RGBA32UI,
};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::glClearBufferfv;
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
use yakogl::pvr::csbgen as csb;
use yakogl::pvr::ffi::*;
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};

mod usc_exec;

/* ---- the platform ------------------------------------------------------------ */

/* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE): init refuses
 * a platform that answers another. */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    #[allow(dead_code)]
    mem: Box<[u64]>,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);
/// The PBE STATE word pair of each end-of-tile program built (slot 0).
static EMITS: Mutex<Vec<[u32; 2]>> = Mutex::new(Vec::new());
/// The PBE REG word pair of attachment 0 of each kick submitted.
static KICKS: Mutex<Vec<[u64; 2]>> = Mutex::new(Vec::new());

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
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = heap_base + 0x1000 + p.bump[h];
        p.bump[h] += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, Alloc { mem });
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

/// The "GPU": records the kick's PBE REG words and completes it.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).push(k.frag.pbe_word[0]);
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

/// Records the first PBE STATE word pair of each end-of-tile program.
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
        EMITS.lock().unwrap_or_else(|e| e.into_inner()).push([*emit_words, *emit_words.add(1)]);
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the draw entry, wrapped ------------------------------------------------- */

/// What the GL core handed the back end for one draw or clear.
#[derive(Clone, Debug)]
struct Handed {
    target_format: u8,
    do_clear: bool,
    clear_rgba: [f32; 4],
    /// The fragment program's variant key, output dwords and USC bytes
    /// (none for a clear alone).
    fs: Option<(u32, u8, Vec<u8>)>,
    /// What the back end answered.
    ret: i32,
}

static HANDED: Mutex<Vec<Handed>> = Mutex::new(Vec::new());

/// The PowerVR's draw entry, recording what it is handed first.
unsafe extern "C" fn rec_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core passes a live draw.
    let ret = unsafe { pvr_backend_draw(d) };
    // SAFETY: as above; `fs` is null or a live record whose code is `usc_dw` dwords.
    let h = unsafe {
        let d = &*d;
        Handed {
            target_format: d.target_format,
            do_clear: d.do_clear,
            clear_rgba: d.clear_rgba,
            fs: d.fs.as_ref().filter(|fs| !fs.usc.is_null()).map(|fs| {
                let code = core::slice::from_raw_parts(fs.usc, fs.usc_dw as usize);
                (fs.blend_key, fs.num_outputs, code.iter().flat_map(|w| w.to_le_bytes()).collect())
            }),
            ret,
        }
    };
    HANDED.lock().unwrap_or_else(|e| e.into_inner()).push(h);
    ret
}

/// The PowerVR's table with the draw entry wrapped.
fn wrapped_backend() -> &'static YakoglBackend {
    // SAFETY: a bitwise copy of the static table (function pointers and
    // plain values), which is never freed; only `draw` is changed.
    let mut t = unsafe { core::ptr::read(&yakogl_backend_pvr) };
    t.draw = Some(rec_draw);
    Box::leak(Box::new(t))
}

/* ---- the context ------------------------------------------------------------- */

const CANVAS: i32 = 16;
/// The target's size: a whole tile, so the stride is the width.
const SIZE: i32 = 16;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAM: GLuint = 0;

const VS: &CStr = c"#version 300 es
layout(location = 0) in vec4 pos;
flat out highp float v;
void main() { v = pos.z; gl_Position = vec4(pos.xy, 0.0, 1.0); }
";
/// Four floats from one flat varying `v`: v, -v, v / 2 and v + 1, each exact
/// in 32 bits and, for v = 1e5, all beyond the half maximum 65504.
const FS: &CStr = c"#version 300 es
precision highp float;
flat in highp float v;
layout(location = 0) out vec4 color;
void main() { color = vec4(v, -v, v * 0.5, v + 1.0); }
";
/// The value `v` the draws give the varying, and what the shader makes of it.
const V: f32 = 1.0e5;
const EXPECT: [f32; 4] = [V, -V, V * 0.5, V + 1.0];

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(type_: GLenum, src: &CStr) -> GLuint {
    let p = src.as_ptr();
    // SAFETY: plain GL calls; `p` is a NUL-terminated string.
    unsafe {
        let s = glCreateShader(type_);
        glShaderSource(s, 1, &p, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "shader compile failed: {:?}", logs());
        s
    }
}

/// The one context and backend of the file, set up once; the tests take
/// turns. It stands in for a build that lists GL_EXT_color_buffer_float on a
/// back end that draws RGBA32F (the PowerVR does; it does not draw RG32F and
/// R11F_G11F_B10F yet, so the extension is not listed).
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = wrapped_backend();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS as u32, CANVAS as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
            HOST_R32F_RT.store(true, Ordering::Relaxed);
            backend::HOST_FLOAT_RT_FORMATS.store(1 << (GLES_SURF_RGBA32F - GLES_SURF_FLOAT_FIRST), Ordering::Relaxed);
            let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed: {:?}", logs());
            PROGRAM = p;
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glColorMask(1, 1, 1, 1);
        glDisable(GL_BLEND);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn ctx() -> &'static GlContext {
    // SAFETY: set once in `rig`, never freed; read under the lock.
    unsafe { &*CTX }
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The two ways to give a framebuffer a colour image.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Renderbuffer,
    Texture,
}

/// A `SIZE`-square image of `internal`, attached at `attachment` of the
/// bound framebuffer; its name.
fn image(kind: Kind, internal: GLenum, attachment: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, internal, SIZE, SIZE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, attachment, GL_RENDERBUFFER, name);
            }
            Kind::Texture => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, 1, internal, SIZE, SIZE);
                glFramebufferTexture2D(GL_FRAMEBUFFER, attachment, GL_TEXTURE_2D, name, 0);
            }
        }
    }
    name
}

/// A framebuffer with a `SIZE`-square image of `internal` at attachment 0,
/// bound: (framebuffer, image, status).
fn target(kind: Kind, internal: GLenum) -> (GLuint, GLuint, GLenum) {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let name = image(kind, internal, GL_COLOR_ATTACHMENT0);
        (fb, name, glCheckFramebufferStatus(GL_FRAMEBUFFER))
    }
}

fn delete_image(kind: Kind, name: GLuint) {
    // SAFETY: a name made by the test.
    unsafe {
        match kind {
            Kind::Renderbuffer => glDeleteRenderbuffers(1, &name),
            Kind::Texture => glDeleteTextures(1, &name),
        }
    }
}

fn release(kind: Kind, fb: GLuint, name: GLuint) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        delete_image(kind, name);
        glFinish();
    }
}

/// A full-screen triangle; z carries the varying's value `V`.
static POS: [f32; 12] = [-1.0, -1.0, V, 1.0, 3.0, -1.0, V, 1.0, -1.0, 3.0, V, 1.0];

/// What the hardware was handed for attachment 0 of a frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Words {
    /// PBE_STATE_WORD1 [7:12]
    packmode: u32,
    /// PBE_STATE_WORD1 [13]
    norm: u32,
    /// PBE_STATE_WORD1 [19]: 0 F16_PER_CHANNEL, 1 8_PER_CHANNEL
    source_format: u32,
}

fn take_handed() -> Vec<Handed> {
    core::mem::take(&mut *HANDED.lock().unwrap())
}

/// Runs `f` (GL calls into the bound framebuffer), then flushes: the PBE
/// words of the one frame and what the back end was handed.
fn frame(f: impl FnOnce()) -> (Words, Vec<Handed>) {
    EMITS.lock().unwrap().clear();
    KICKS.lock().unwrap().clear();
    take_handed();
    f();
    // SAFETY: plain GL call.
    unsafe { glFlush() };
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    let emits = EMITS.lock().unwrap().clone();
    let kicks = KICKS.lock().unwrap().clone();
    assert_eq!(emits.len(), 1, "one end-of-tile program: {:?}", logs());
    assert_eq!(kicks.len(), 1, "one kick: {:?}", logs());
    let w1 = emits[0][1] as u64;
    let w = Words {
        packmode: ((w1 >> 7) & 0x3f) as u32,
        norm: ((w1 >> 13) & 1) as u32,
        source_format: ((w1 >> 19) & 1) as u32,
    };
    (w, take_handed())
}

/// A clear to `rgba` and the triangle over it.
fn clear_and_draw(rgba: [f32; 4]) {
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        glViewport(0, 0, SIZE, SIZE);
        glClearColor(rgba[0], rgba[1], rgba[2], rgba[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        glUseProgram(PROGRAM);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// `image` (the USC bytes of a fragment program reading one flat varying) run
/// once with the varying `v`: pixel outputs 0..n.
fn run_fs(image: &[u8], v: f32, n: u32) -> Vec<u32> {
    let mut inp = usc_exec::sentinel_inputs(1.0e30);
    /* a flat varying reads its value whichever coefficient plane it takes */
    inp.coeff.fill(v.to_bits());
    let out = usc_exec::run_binary(image, &inp, true).unwrap_or_else(|e| panic!("byte model: {e}"));
    (0..n).map(|r| *out.get(&r).unwrap_or_else(|| panic!("nothing in pixel output {r}: {out:?}"))).collect()
}

/* PBE packmodes and source formats (csbgen/rogue/pbestate.xml) */
const PACKMODE_F32F32F32F32: u32 = csb::ROGUE_PBESTATE_PACKMODE_F32F32F32F32;
const PACKMODE_U32U32U32U32: u32 = csb::ROGUE_PBESTATE_PACKMODE_U32U32U32U32;
const PACKMODE_F16F16F16F16: u32 = csb::ROGUE_PBESTATE_PACKMODE_F16F16F16F16;
const SOURCE_F16_PER_CHANNEL: u32 = csb::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL;
const SOURCE_8_PER_CHANNEL: u32 = csb::ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL;

/// The RGBA32UI variant key: what an RGBA32F target's program is built for.
fn lanes_key() -> u32 {
    shc_blend_key_int_rt(ShcIntRt { channels: 4, bits: 32, signed: false })
}

/* ---- the tests --------------------------------------------------------------- */

/// RGBA32F as a renderbuffer and as a texture: complete, drawn as the
/// 128-bit surface.
#[test]
fn rgba32f_is_a_128_bit_target() {
    let _g = rig();
    for kind in [Kind::Renderbuffer, Kind::Texture] {
        let (fb, name, status) = target(kind, GL_RGBA32F);
        assert_eq!(err(), GL_NO_ERROR, "{kind:?}");
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{kind:?}: status");
        assert_eq!(fb_target_format(ctx()), GLES_SURF_RGBA32F, "{kind:?}: surface");
        assert_eq!(backend::gles_surf_bytes_per_pixel(GLES_SURF_RGBA32F), 16);
        assert_eq!(surf_out_dwords(GLES_SURF_RGBA32F), 4, "four output registers");
        release(kind, fb, name);
    }
}

/// A clear and a draw into RGBA32F: the PBE state is Mesa's
/// R32G32B32A32_SFLOAT, and the program the draw is given writes the four
/// floats unconverted into pixel outputs 0..3.
#[test]
fn draw_writes_full_floats() {
    let _g = rig();
    for kind in [Kind::Renderbuffer, Kind::Texture] {
        let (fb, name, status) = target(kind, GL_RGBA32F);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{kind:?}");
        let (w, handed) = frame(|| clear_and_draw([0.0, 0.0, 0.0, 1.0]));
        assert_eq!(
            w,
            Words { packmode: PACKMODE_F32F32F32F32, norm: 0, source_format: SOURCE_8_PER_CHANNEL },
            "{kind:?}: PBE state"
        );
        let draws: Vec<_> = handed.iter().filter(|h| h.fs.is_some()).collect();
        assert_eq!(draws.len(), 1, "{kind:?}: one draw: {handed:?}");
        let h = draws[0];
        assert_eq!((h.target_format, h.ret), (GLES_SURF_RGBA32F, 0), "{kind:?}: drawn: {:?}", logs());
        let (key, outputs, image) = h.fs.clone().unwrap();
        assert_eq!(key, lanes_key(), "{kind:?}: the RGBA32UI variant");
        assert_eq!(outputs, 4, "{kind:?}: four output dwords");
        assert_eq!(run_fs(&image, V, 4), EXPECT.map(f32::to_bits), "{kind:?}: the floats' bits");
        /* and a value past f32's half range on the other side */
        assert_eq!(run_fs(&image, -3.0e38, 2), [(-3.0e38f32).to_bits(), 3.0e38f32.to_bits()], "{kind:?}");
        release(kind, fb, name);
    }
}

/// A colour mask keeps the masked lanes: the variant carries the mask, as
/// an integer target's does, and the draw is still the 128-bit target's.
#[test]
fn colour_mask_takes_the_masked_variant() {
    let _g = rig();
    let (fb, name, status) = target(Kind::Texture, GL_RGBA32F);
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE);
    let (w, handed) = frame(|| {
        clear_and_draw([0.0, 0.0, 0.0, 1.0]);
        // SAFETY: plain GL calls.
        unsafe {
            glColorMask(1, 0, 1, 1);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glColorMask(1, 1, 1, 1);
        }
    });
    assert_eq!(w.packmode, PACKMODE_F32F32F32F32);
    let keys: Vec<u32> = handed.iter().filter_map(|h| h.fs.as_ref().map(|f| f.0)).collect();
    assert_eq!(keys.len(), 2, "{handed:?}");
    assert_eq!(keys[0], lanes_key());
    assert_ne!(keys[1], lanes_key(), "the masked draw takes a variant of its own");
    assert_eq!(
        yakogl::shaderc::ShcIntRt::of_key(keys[1]),
        Some(ShcIntRt { channels: 4, bits: 32, signed: false }),
        "still the four lanes"
    );
    assert_eq!((keys[1] >> 26) & 0xf, 0b0010, "green masked");
    release(Kind::Texture, fb, name);
}

/// A clear alone: the colour reaches the back end unclamped, packs to the
/// floats' bits, and is stored as the 128-bit target.
#[test]
fn clear_keeps_full_floats() {
    let _g = rig();
    let rgba = [1.0e5f32, -1.0e5, 65536.5, 3.0e38];
    assert_eq!(clear_uniform(GLES_SURF_RGBA32F, &rgba), rgba.map(f32::to_bits), "the clear's four dwords");
    for kind in [Kind::Renderbuffer, Kind::Texture] {
        let (fb, name, status) = target(kind, GL_RGBA32F);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{kind:?}");
        let (w, handed) = frame(|| {
            // SAFETY: plain GL calls.
            unsafe {
                glClearColor(rgba[0], rgba[1], rgba[2], rgba[3]);
                glClear(GL_COLOR_BUFFER_BIT);
            }
        });
        assert_eq!(w.packmode, PACKMODE_F32F32F32F32, "{kind:?}");
        let clears: Vec<_> = handed.iter().filter(|h| h.do_clear).collect();
        assert_eq!(clears.len(), 1, "{kind:?}: {handed:?}");
        assert_eq!((clears[0].clear_rgba, clears[0].ret), (rgba, 0), "{kind:?}: {:?}", logs());
        /* glClearBufferfv: the same */
        let (w, handed) = frame(|| {
            // SAFETY: plain GL call; `rgba` is four floats.
            unsafe { glClearBufferfv(GL_COLOR, 0, rgba.as_ptr()) };
        });
        assert_eq!(w.packmode, PACKMODE_F32F32F32F32, "{kind:?}");
        let clears: Vec<_> = handed.iter().filter(|h| h.do_clear).collect();
        assert_eq!(clears.len(), 1, "{kind:?}: {handed:?}");
        assert_eq!((clears[0].clear_rgba, clears[0].ret), (rgba, 0), "{kind:?}: glClearBufferfv");
        release(kind, fb, name);
    }
}

/// Texels beyond the half range, as an RGBA32F texture holds them.
fn texels() -> Vec<f32> {
    (0..SIZE * SIZE)
        .flat_map(|i| {
            let x = i as f32;
            [1.0e5 + x, -7.0e4 - x, 65505.0 + 0.5 * x, 1.0 / (x + 3.0)]
        })
        .collect()
}

/// glReadPixels (GL_RGBA, GL_FLOAT) of an RGBA32F texture image: the
/// stored floats exactly.
#[test]
fn read_pixels_reads_full_floats() {
    let _g = rig();
    let (fb, tex, status) = target(Kind::Texture, GL_RGBA32F);
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE);
    let want = texels();
    let mut got = vec![0f32; want.len()];
    // SAFETY: plain GL calls; the buffers hold SIZE x SIZE RGBA floats.
    unsafe {
        glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, SIZE, SIZE, GL_RGBA, GL_FLOAT, want.as_ptr() as *const c_void);
        assert_eq!(err(), GL_NO_ERROR, "upload");
        glReadPixels(0, 0, SIZE, SIZE, GL_RGBA, GL_FLOAT, got.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    assert_eq!(
        got.iter().map(|f| f.to_bits()).collect::<Vec<_>>(),
        want.iter().map(|f| f.to_bits()).collect::<Vec<_>>()
    );
    release(Kind::Texture, fb, tex);
}

/// glBlitFramebuffer from an RGBA32F texture into an RGBA32F renderbuffer
/// and an RGBA32F texture (NEAREST, same size): the floats exactly.
#[test]
fn blit_copies_full_floats() {
    let _g = rig();
    let want = texels();
    for dst_kind in [Kind::Renderbuffer, Kind::Texture] {
        let (src_fb, src_tex, status) = target(Kind::Texture, GL_RGBA32F);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE);
        let (dst_fb, dst, status) = target(dst_kind, GL_RGBA32F);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{dst_kind:?}");
        let mut got = vec![0f32; want.len()];
        // SAFETY: plain GL calls; the buffers hold SIZE x SIZE RGBA floats.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, src_tex);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, SIZE, SIZE, GL_RGBA, GL_FLOAT, want.as_ptr() as *const c_void);
            glBindFramebuffer(GL_READ_FRAMEBUFFER, src_fb);
            glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst_fb);
            glBlitFramebuffer(0, 0, SIZE, SIZE, 0, 0, SIZE, SIZE, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            assert_eq!(err(), GL_NO_ERROR, "{dst_kind:?}: blit: {:?}", logs());
            glBindFramebuffer(GL_FRAMEBUFFER, dst_fb);
            glReadPixels(0, 0, SIZE, SIZE, GL_RGBA, GL_FLOAT, got.as_mut_ptr() as *mut c_void);
        }
        assert_eq!(err(), GL_NO_ERROR, "{dst_kind:?}");
        assert_eq!(
            got.iter().map(|f| f.to_bits()).collect::<Vec<_>>(),
            want.iter().map(|f| f.to_bits()).collect::<Vec<_>>(),
            "{dst_kind:?}"
        );
        release(dst_kind, dst_fb, dst);
        release(Kind::Texture, src_fb, src_tex);
    }
}

/// RGBA32F beside another colour attachment: its four output registers
/// leave none for the other (the tile-buffer spill is not built for it), so
/// the draw is not drawn -- refused by the back end by name, or before it
/// by the GL core where the framebuffer is not complete.
#[test]
fn beside_another_attachment_is_refused() {
    let _g = rig();
    let (fb, tex, _) = target(Kind::Texture, GL_RGBA32F);
    let other = image(Kind::Texture, GL_RGBA8, GL_COLOR_ATTACHMENT1);
    take_handed();
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
        glDrawBuffers(2, bufs.as_ptr());
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        if status == GL_FRAMEBUFFER_COMPLETE {
            glViewport(0, 0, SIZE, SIZE);
            glUseProgram(PROGRAM);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            glFlush();
            while glGetError() != GL_NO_ERROR {}
            let handed = take_handed();
            assert!(handed.iter().all(|h| h.ret != 0), "never drawn: {handed:?}");
            assert!(
                handed.is_empty() || logs().iter().any(|l| l == REFUSE_RGBA32F_MRT),
                "refused by name: {:?}",
                logs()
            );
        }
        glDrawBuffers(1, bufs.as_ptr());
    }
    delete_image(Kind::Texture, other);
    release(Kind::Texture, fb, tex);
}

/// The back end itself, handed RGBA32F beside RGBA8 (a clear of both):
/// refused by name.
#[test]
fn back_end_refuses_rgba32f_beside_another() {
    let _g = rig();
    let mut d = GlesDraw::ZERO;
    d.target_w = SIZE as u32;
    d.target_h = SIZE as u32;
    d.target_gpu = 0x80_1000_0000;
    d.target_format = GLES_SURF_RGBA32F;
    d.num_targets = 2;
    d.targets[0].gpu = d.target_gpu;
    d.targets[0].format = GLES_SURF_RGBA32F;
    d.targets[1].gpu = 0x80_2000_0000;
    d.targets[1].format = backend::GLES_SURF_RGBA8888;
    // SAFETY: a draw that outlives the call.
    let r = unsafe { pvr_backend_draw(&d) };
    assert!(r < 0, "refused: {r}");
    assert!(logs().iter().any(|l| l == REFUSE_RGBA32F_MRT), "{:?}", logs());
}

/// The contrast: RGBA32UI keeps U32U32U32U32 and RGBA16F its halves.
#[test]
fn rgba32ui_and_rgba16f_keep_their_words() {
    let _g = rig();
    assert_eq!(pbe_emit_words(0, GLES_SURF_RGBA32UI, 0, 0)[1] >> 7 & 0x3f, PACKMODE_U32U32U32U32);
    assert_eq!(pbe_emit_words(0, GLES_SURF_RGBA32F, 0, 0)[1] >> 7 & 0x3f, PACKMODE_F32F32F32F32);
    let (fb, rb, status) = target(Kind::Renderbuffer, GL_RGBA16F);
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE);
    assert_eq!(fb_target_format(ctx()), GLES_SURF_RGBA16F);
    let (w, _) = frame(|| clear_and_draw([0.0, 0.0, 0.0, 1.0]));
    assert_eq!(w, Words { packmode: PACKMODE_F16F16F16F16, norm: 0, source_format: SOURCE_F16_PER_CHANNEL });
    release(Kind::Renderbuffer, fb, rb);
}
