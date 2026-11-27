// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Three mip levels a render (`pvr_backend_set_mip_three_levels`,
//! docs/notes/pvr-mip-three-levels.md) on every colour format of dEQP-GLES2
//! `functional.texture.mipmap.2d.generate.*`, on the PowerVR backend.
//!
//! dEQP's list (VK-GL-CTS at 93bca01, modules/gles2/functional/
//! es2fTextureMipmapTests.cpp, `formats[]`, `tex2DSizes[]`, `genHints[]`):
//! a8, l8, la88, rgb565, rgb888, rgba4444, rgba5551 and rgba8888, each at
//! 64x64 and at `non_square` 32x64 (32 wide, 64 high), with the hints
//! fastest and nicest: 32 cases. Each case uploads level 0 (an 8-texel grid
//! of (1, 0.5, 0, 0.5) and (0, 0, 1, 1)) with glTexImage2D and asks
//! glGenerateMipmap for the rest. This file runs those chains at 64x64,
//! 32x64 and also 64x32 (the transposed shape), with the knob off and on,
//! on the platform of tests/pvr_mip_three_levels.rs (copied: the REAL
//! backend, host memory, every submit completes at once), and pins:
//!
//! 1. Which path each chain takes (`PINNED_PATHS`): three levels a render,
//!    a kick a level, refused by the backend (the GL core's CPU filter), or
//!    never offered to it (the GL core keeps the chain: a format other than
//!    plain RGBA8, src/gl/texture.rs glGenerateMipmap). The knob must not
//!    move a chain from one of the last two. At this commit: rgb565,
//!    rgb888, rgba4444, rgba5551 and rgba8888 are stored as RGBA8 and take
//!    three levels a render at 64x64 and 64x32; a8, l8 and la88 are the GL
//!    core's at every size; the 32x64 chain of each of the five is refused
//!    at either setting (its 1x2 level: the PBE writes pixel pairs). With
//!    the dEQP grid and random bytes, every knob-on level matched the
//!    knob-off model byte for byte (power-of-two sides: weights of 1/2).
//! 2. The kick count of the chain: a kick a level (n - 1 for n levels) off,
//!    `groups(n)` renders on (1; 2-4; 5-6 of a 7-level chain), none for a
//!    chain the GPU does not make. Every kick declares level 1's size.
//! 3. The store words: each level a knob-on render stores goes out with
//!    the PBE register words (format, pitch, packing, clip) of the knob-off
//!    kick for that level.
//! 4. The hints: fastest and nicest submit the same kicks.
//! 5. With `shaderc-online` (not in a `report-es2` build: mipgen3.frag is
//!    GLSL ES 3.00), the level contents of each chain the three-level path
//!    takes, from dEQP's grid and from random bytes, in the reference IR
//!    evaluator with tests/pvr_mip_three_levels.rs's TPU model: levels 1
//!    and 2 byte for byte the knob-off chain's, every later level within 1
//!    per channel.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_mip_formats
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_mip_formats
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,report-es2 --test pvr_mip_formats
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

use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
use yakogl::gl::types::*;
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

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    _mem: Box<[u64]>,
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
    /// What each kick submitted stores, in order.
    kicks: Vec<KickRec>,
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
        p.allocs.insert(gpu, Alloc { _mem: mem });
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

/// The "GPU": records the kick, draws nothing.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.fence += 1;
        p.kicks.push(KickRec { size: (k.width, k.height), num_rt: k.frag.num_rt, pbe: k.frag.pbe_word });
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
static mut CTX: *mut GlContext = core::ptr::null_mut();

fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(128, 128);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    with_platform(|p| p.log.clear());
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

/* ---- dEQP's formats -------------------------------------------------------------- */

/// One of dEQP-GLES2 mipmap.2d.generate's formats (`formats[]`).
#[derive(Clone, Copy, Debug)]
struct Fmt {
    name: &'static str,
    format: GLenum,
    type_: GLenum,
}

const FORMATS: [Fmt; 8] = [
    Fmt { name: "a8", format: GL_ALPHA, type_: GL_UNSIGNED_BYTE },
    Fmt { name: "l8", format: GL_LUMINANCE, type_: GL_UNSIGNED_BYTE },
    Fmt { name: "la88", format: GL_LUMINANCE_ALPHA, type_: GL_UNSIGNED_BYTE },
    Fmt { name: "rgb565", format: GL_RGB, type_: GL_UNSIGNED_SHORT_5_6_5 },
    Fmt { name: "rgb888", format: GL_RGB, type_: GL_UNSIGNED_BYTE },
    Fmt { name: "rgba4444", format: GL_RGBA, type_: GL_UNSIGNED_SHORT_4_4_4_4 },
    Fmt { name: "rgba5551", format: GL_RGBA, type_: GL_UNSIGNED_SHORT_5_5_5_1 },
    Fmt { name: "rgba8888", format: GL_RGBA, type_: GL_UNSIGNED_BYTE },
];

/// dEQP's two sizes (64x64 and `non_square` 32x64, width x height) and the
/// transposed non-square shape.
const SIZES: [(u32, u32); 3] = [(64, 64), (32, 64), (64, 32)];

/// dEQP's two hints (`genHints[]`).
const HINTS: [(&str, GLenum); 2] = [("fastest", GL_FASTEST), ("nicest", GL_NICEST)];

/// dEQP's level 0 (`tcu::fillWithGrid(level, 8, (1, 0.5, 0, 0.5), (0, 0,
/// 1, 1))`) as RGBA8 bytes: 8-texel cells, alternating.
fn grid(w: u32, h: u32) -> Vec<[u8; 4]> {
    let (a, b) = ([255, 128, 0, 128], [0, 0, 255, 255]);
    (0..h).flat_map(|y| (0..w).map(move |x| if (x / 8 + y / 8) % 2 == 0 { a } else { b })).collect()
}

/// `px` in `f`'s client layout, rows tightly packed (every row of these
/// sizes is a multiple of 4 bytes: GL_UNPACK_ALIGNMENT's default holds).
fn client_bytes(f: &Fmt, px: &[[u8; 4]]) -> Vec<u8> {
    let s = |v: u8, bits: u32| (u16::from(v) * ((1 << bits) - 1) + 127) / 255;
    let mut out = Vec::new();
    for &[r, g, b, a] in px {
        match (f.format, f.type_) {
            (GL_ALPHA, _) => out.push(a),
            (GL_LUMINANCE, _) => out.push(r),
            (GL_LUMINANCE_ALPHA, _) => out.extend([r, a]),
            (GL_RGB, GL_UNSIGNED_BYTE) => out.extend([r, g, b]),
            (GL_RGBA, GL_UNSIGNED_BYTE) => out.extend([r, g, b, a]),
            (GL_RGB, GL_UNSIGNED_SHORT_5_6_5) => out.extend(((s(r, 5) << 11) | (s(g, 6) << 5) | s(b, 5)).to_ne_bytes()),
            (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4) => {
                out.extend(((s(r, 4) << 12) | (s(g, 4) << 8) | (s(b, 4) << 4) | s(a, 4)).to_ne_bytes())
            }
            (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1) => {
                out.extend(((s(r, 5) << 11) | (s(g, 5) << 6) | (s(b, 5) << 1) | s(a, 1)).to_ne_bytes())
            }
            _ => unreachable!("{f:?}"),
        }
    }
    out
}

/* ---- the knob and the chains --------------------------------------------------- */

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

/// The levels of a `w` x `h` chain (ES 2.0 3.7.11: down to 1x1).
fn levels(w: u32, h: u32) -> usize {
    (32 - w.max(h).leading_zeros()) as usize
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

/// The colour attachments a kick stores (`num_rt` 0 is one).
fn rts(k: &KickRec) -> u32 {
    k.num_rt.max(1)
}

/// The path a chain took.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Path {
    /// The backend drew it, three levels a render.
    Three,
    /// The backend drew it, a kick a level.
    KickALevel,
    /// Offered to the backend, which refused it: the GL core's CPU filter.
    Refused,
    /// Never offered to the backend: the GL core's chain.
    Core,
}

struct Chain {
    path: Path,
    kicks: Vec<KickRec>,
}

/// One dEQP generate case: a fresh `size` texture of `f` holding `px` at
/// level 0 (glTexImage2D), dEQP's sampler state, the hint, then
/// glGenerateMipmap and glFlush, the knob as `three` says. The kicks are
/// those submitted from glGenerateMipmap on.
fn chain(f: &Fmt, size: (u32, u32), hint: GLenum, three: bool, px: &[[u8; 4]]) -> Chain {
    let data = client_bytes(f, px);
    let mut tex = 0;
    let (mut made0, mut refused0, mut made1, mut refused1) = (0u32, 0u32, 0u32, 0u32);
    // SAFETY: plain GL calls; room for one name; `data` holds the level.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            f.format as GLint,
            size.0 as GLsizei,
            size.1 as GLsizei,
            0,
            f.format,
            f.type_,
            data.as_ptr() as *const c_void,
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST_MIPMAP_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glHint(GL_GENERATE_MIPMAP_HINT, hint);
        assert_eq!(glGetError(), GL_NO_ERROR, "{}: setup; {:?}", f.name, logs());
        glFlush();
        pvr_backend_mip_chains(&mut made0, &mut refused0);
    }
    let from = with_platform(|p| p.kicks.len());
    let run = || {
        // SAFETY: plain GL calls on the bound texture.
        unsafe {
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(glGetError(), GL_NO_ERROR, "{}: glGenerateMipmap; {:?}", f.name, logs());
            glFlush();
        }
    };
    if three {
        with_three_levels(run);
    } else {
        run();
    }
    let kicks = with_platform(|p| p.kicks[from..].to_vec());
    // SAFETY: plain GL calls; room for the counts; a name this test made.
    unsafe {
        pvr_backend_mip_chains(&mut made1, &mut refused1);
        glBindTexture(GL_TEXTURE_2D, 0);
        glDeleteTextures(1, &tex);
        glHint(GL_GENERATE_MIPMAP_HINT, GL_DONT_CARE);
    }
    let what = format!("{} {}x{} three={three}", f.name, size.0, size.1);
    let path = match (made1 - made0, refused1 - refused0) {
        (0, 0) => Path::Core,
        (0, 1) => Path::Refused,
        (1, 0) if kicks.iter().any(|k| rts(k) > 1) => Path::Three,
        (1, 0) => Path::KickALevel,
        d => panic!("{what}: chains made, refused {d:?}"),
    };
    let level1 = ((size.0 / 2).max(1), (size.1 / 2).max(1));
    assert!(
        kicks.iter().all(|k| k.size == level1),
        "{what}: a kick that is not one of the chain's (each declares level 1's size {level1:?}): {:?}",
        kicks.iter().map(|k| k.size).collect::<Vec<_>>()
    );
    Chain { path, kicks }
}

/// (format, size, path with the knob off, path with the knob on).
///
/// Only a plain RGBA8 texture is offered to the backend (src/gl/texture.rs
/// glGenerateMipmap; `mip_chain_gpu` takes nothing else). rgba8888 is one,
/// and so are rgb888, rgb565, rgba4444 and rgba5551: glTexImage2D stores
/// them as RGBA8 (alpha 1; the 16-bit words widened by bit replication,
/// src/gl/texture.rs). a8, l8 and la88 keep their own one- and two-byte
/// formats and the GL core filters them. dEQP's `non_square` 32x64
/// is refused at either setting: its level 5 is 1x2, one texel a row above
/// a taller tail, and the PBE writes pixel pairs (`mip_chain_gpu`); the
/// GL core filters that chain on the CPU. 64x32's 1-wide level is its last
/// (1x1), so it is drawn.
const PINNED_PATHS: &[(&str, (u32, u32), Path, Path)] = &[
    ("a8", (64, 64), Path::Core, Path::Core),
    ("a8", (32, 64), Path::Core, Path::Core),
    ("a8", (64, 32), Path::Core, Path::Core),
    ("l8", (64, 64), Path::Core, Path::Core),
    ("l8", (32, 64), Path::Core, Path::Core),
    ("l8", (64, 32), Path::Core, Path::Core),
    ("la88", (64, 64), Path::Core, Path::Core),
    ("la88", (32, 64), Path::Core, Path::Core),
    ("la88", (64, 32), Path::Core, Path::Core),
    ("rgb565", (64, 64), Path::KickALevel, Path::Three),
    ("rgb565", (32, 64), Path::Refused, Path::Refused),
    ("rgb565", (64, 32), Path::KickALevel, Path::Three),
    ("rgb888", (64, 64), Path::KickALevel, Path::Three),
    ("rgb888", (32, 64), Path::Refused, Path::Refused),
    ("rgb888", (64, 32), Path::KickALevel, Path::Three),
    ("rgba4444", (64, 64), Path::KickALevel, Path::Three),
    ("rgba4444", (32, 64), Path::Refused, Path::Refused),
    ("rgba4444", (64, 32), Path::KickALevel, Path::Three),
    ("rgba5551", (64, 64), Path::KickALevel, Path::Three),
    ("rgba5551", (32, 64), Path::Refused, Path::Refused),
    ("rgba5551", (64, 32), Path::KickALevel, Path::Three),
    ("rgba8888", (64, 64), Path::KickALevel, Path::Three),
    ("rgba8888", (32, 64), Path::Refused, Path::Refused),
    ("rgba8888", (64, 32), Path::KickALevel, Path::Three),
];

fn pinned(f: &Fmt, size: (u32, u32)) -> (Path, Path) {
    PINNED_PATHS
        .iter()
        .find(|p| p.0 == f.name && p.1 == size)
        .map(|p| (p.2, p.3))
        .unwrap_or_else(|| panic!("{} {size:?}: no pinned path", f.name))
}

/// Every dEQP generate format at every size, knob off and on, both hints:
/// the path is the pinned one, the kicks are a kick a level off and
/// `groups(n)` renders on (none for a chain the GPU does not make), the
/// hints submit the same kicks, and every level a knob-on render stores
/// has the store words of the knob-off kick for that level.
#[test]
fn every_generate_format_takes_its_pinned_path_with_the_level_kicks_store_words() {
    let _g = rig();
    assert_eq!(PINNED_PATHS.len(), FORMATS.len() * SIZES.len(), "a pinned path for every format and size");
    let mut table = Vec::new();
    for f in &FORMATS {
        for &size in &SIZES {
            let n = levels(size.0, size.1);
            let px = grid(size.0, size.1);
            let (want_off, want_on) = pinned(f, size);
            let mut by_knob = Vec::new();
            for three in [false, true] {
                let runs: Vec<Chain> = HINTS.iter().map(|&(_, h)| chain(f, size, h, three, &px)).collect();
                let what = format!("{} {}x{} three={three}", f.name, size.0, size.1);
                assert_eq!(runs[0].path, runs[1].path, "{what}: fastest and nicest took different paths");
                assert_eq!(runs[0].kicks, runs[1].kicks, "{what}: fastest and nicest submitted different kicks");
                let c = runs.into_iter().next().unwrap();
                let want = if three { want_on } else { want_off };
                assert_eq!(c.path, want, "{what}: not the pinned path; {:?}", logs());
                let kicks = match c.path {
                    Path::Three => groups(n).len(),
                    Path::KickALevel => n - 1,
                    Path::Refused | Path::Core => 0,
                };
                assert_eq!(c.kicks.len(), kicks, "{what}: kicks of the chain ({n} levels)");
                by_knob.push(c);
            }
            let (off, on) = (&by_knob[0], &by_knob[1]);
            table.push(format!(
                "{:>8} {:>2}x{:<2} off {:?} ({} kicks), on {:?} ({} kicks)",
                f.name,
                size.0,
                size.1,
                off.path,
                off.kicks.len(),
                on.path,
                on.kicks.len()
            ));
            if on.path != Path::Three {
                assert_eq!(on.kicks, off.kicks, "{} {size:?}: the knob moved a chain it does not take", f.name);
                continue;
            }
            assert!(off.kicks.iter().all(|k| rts(k) == 1), "{} {size:?}: one attachment a level kick", f.name);
            for (r, (kick, lv)) in on.kicks.iter().zip(groups(n)).enumerate() {
                assert_eq!(rts(kick) as usize, lv.len(), "{} {size:?}: render {r}'s attachments are {lv:?}", f.name);
                for (slot, &l) in lv.iter().enumerate() {
                    assert_eq!(
                        kick.pbe[slot],
                        off.kicks[l - 1].pbe[0],
                        "{} {size:?}: level {l}, attachment {slot} of render {r}: not the level kick's store words",
                        f.name
                    );
                }
            }
        }
    }
    println!("{}", table.join("\n"));
}

/// The chain shapes this file runs: 7 levels each, 6 level kicks, 3
/// renders (1; 2-4; 5-6) with the knob on.
#[test]
fn every_size_is_a_7_level_chain_of_3_renders() {
    for &(w, h) in &SIZES {
        assert_eq!(levels(w, h), 7, "{w}x{h}");
        assert_eq!(groups(7), [vec![1], vec![2, 3, 4], vec![5, 6]], "{w}x{h}");
    }
}

/* ---- the level contents ------------------------------------------------------- */

/* mipgen3.frag is GLSL ES 3.00, which a report-es2 build refuses (pp.rs
 * out_of_scope): this part is left out of it, as in
 * tests/pvr_mip_three_levels.rs. */
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
mod contents {
    use super::*;
    use yakogl::pvr::internal::{PVR_INTERNAL_MIPGEN3, PVR_INTERNAL_TABLE};
    use yakogl::shaderc::online::arena::Arena;
    use yakogl::shaderc::online::eval::{self, Fault, Frame, Inputs, Sink, TexLevel, TexLookup, TexelSource};
    use yakogl::shaderc::online::ir::{IoSlot, Module, TexDim};
    use yakogl::shaderc::online::lower::compile_to_ir;
    use yakogl::shaderc::online::sema::Stage;

    const MIPGEN3: &str = include_str!("../tools/shaders/internal/es/mipgen3.frag");

    /* The model of tests/pvr_mip_three_levels.rs `mod levels`, copied: the
     * TPU's bilinear sample (ES 3.0.6 3.8.10's four-weight sum over
     * value / 255 in f32), the 8-bit pack by round-half-up, the level kick
     * (mipgen.frag) and mipgen3.frag's render in the reference IR
     * evaluator. */

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

    /// The TPU model's bilinear sample of `l` (CLAMP_TO_EDGE) at `c`.
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

    /// mipgen3.frag's render of `dst` (2 or 3 levels) from `src`, with the
    /// block `record_mip_levels` writes.
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

    /// Random bytes (xorshift32), alpha included.
    fn random(w: u32, h: u32) -> Vec<[u8; 4]> {
        let mut s: u32 = 0x1234_5678;
        (0..w * h)
            .map(|_| {
                s ^= s << 13;
                s ^= s >> 17;
                s ^= s << 5;
                s.to_le_bytes()
            })
            .collect()
    }

    /// Level 0 as the RGBA8 texture holds `px` uploaded in `f`
    /// (src/gl/texture.rs's upload): rgb888's and rgb565's alpha is 1, the
    /// 16-bit words' fields are widened by bit replication (4 bits: x 17),
    /// rgba5551's alpha bit is 0 or 255.
    fn as_stored(f: &Fmt, px: &[[u8; 4]]) -> Vec<[u8; 4]> {
        let b = client_bytes(f, px);
        let w = |i: usize| u16::from_ne_bytes([b[2 * i], b[2 * i + 1]]);
        let x5 = |v: u16| ((v << 3) | (v >> 2)) as u8;
        (0..px.len())
            .map(|i| match (f.format, f.type_) {
                (GL_RGBA, GL_UNSIGNED_BYTE) => [b[4 * i], b[4 * i + 1], b[4 * i + 2], b[4 * i + 3]],
                (GL_RGB, GL_UNSIGNED_BYTE) => [b[3 * i], b[3 * i + 1], b[3 * i + 2], 255],
                (GL_RGB, GL_UNSIGNED_SHORT_5_6_5) => {
                    let p = w(i);
                    let g = (p >> 5) & 0x3f;
                    [x5((p >> 11) & 0x1f), ((g << 2) | (g >> 4)) as u8, x5(p & 0x1f), 255]
                }
                (GL_RGBA, GL_UNSIGNED_SHORT_4_4_4_4) => {
                    let p = w(i);
                    [12, 8, 4, 0].map(|s| ((p >> s) & 0xf) as u8 * 17)
                }
                (GL_RGBA, GL_UNSIGNED_SHORT_5_5_5_1) => {
                    let p = w(i);
                    [x5((p >> 11) & 0x1f), x5((p >> 6) & 0x1f), x5((p >> 1) & 0x1f), if p & 1 != 0 { 255 } else { 0 }]
                }
                _ => panic!("{}: not a format the three-level path takes", f.name),
            })
            .collect()
    }

    /// Each chain the three-level path takes (`PINNED_PATHS`), from dEQP's
    /// grid and from random bytes: knob on, levels 1 and 2 are the knob-off
    /// chain's byte for byte, and every later level within 1 per channel.
    #[test]
    fn three_level_chains_are_the_level_kick_chains_within_1() {
        let mut buf = vec![0u8; 8 << 20];
        let a = Arena::new(&mut buf);
        let rec = &PVR_INTERNAL_TABLE[PVR_INTERNAL_MIPGEN3].shader;
        assert_eq!(rec.num_outputs, 3, "three colour outputs");
        let lowered = compile_to_ir(&a, Stage::Fragment, MIPGEN3).unwrap_or_else(|e| panic!("{}: {}", e.line, e.msg));
        let m = &lowered.module;
        let mut checked = 0;
        for f in &FORMATS {
            for &size in &SIZES {
                if pinned(f, size).1 != Path::Three {
                    continue;
                }
                let (w, h) = size;
                let n = levels(w, h);
                let sz: Vec<(u32, u32)> = (0..n).map(|l| ((w >> l).max(1), (h >> l).max(1))).collect();
                for (kind, px) in [("grid", grid(w, h)), ("random", random(w, h))] {
                    let base = Level { w, h, px: as_stored(f, &px) };
                    let mut off = vec![base.clone()];
                    for l in 1..n {
                        let next = level_kick(&off[l - 1], sz[l].0, sz[l].1);
                        off.push(next);
                    }
                    let mut on = vec![base];
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
                        let what = format!("{} {w}x{h} {kind} level {l} ({}x{})", f.name, sz[l].0, sz[l].1);
                        let (mut texd, mut maxd) = (0, 0u8);
                        for (x, y) in on[l].px.iter().zip(&off[l].px) {
                            let d = (0..4).map(|c| x[c].abs_diff(y[c])).max().unwrap();
                            texd += usize::from(d != 0);
                            maxd = maxd.max(d);
                        }
                        println!("{what}: {texd} of {} texels differ, max |d| {maxd}", on[l].px.len());
                        let most = if l <= 2 { 0 } else { 1 };
                        assert!(maxd <= most, "{what}: {maxd} from the knob-off chain (at most {most})");
                        checked += 1;
                    }
                }
            }
        }
        assert_eq!(checked, 5 * 2 * 2 * 6, "five formats at 64x64 and 64x32, two pictures, six levels");
    }
}
