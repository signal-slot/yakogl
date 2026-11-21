// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64): a colour target whose GL format has fewer channels
//! than the store behind it keeps GL's missing channels (issue #313, family
//! F5 of the AM62P dEQP-GLES3 fbo failures).
//!
//! GL_R8, GL_RG8 and a GL_RED / GL_RG byte texture are kept in an RGBA8888
//! store, GL_RG16F and GL_RGB16F in RGBA16F's four halves, GL_RGB8 / a
//! GL_RGB texture in RGBA8888. GL says a missing red, green or blue reads 0
//! and a missing alpha 1 (ES 3.0 4.3.2; table 3.23 for a sampled texture).
//! The PBE wrote the fragment's green and blue into those lanes (only the
//! alpha lane of attachment 0 was SWIZ.ONE), so a later glReadPixels, blit
//! or sampling saw them: dEQP-GLES3.functional.fbo.color.blend.{r8,rg8,
//! rg16f}_src_over, fbo.render.shared_colorbuffer{,_clear}.*{r8,rg8,rg16f}*
//! and the r8 rows of fbo.blit.default_framebuffer. Now the GL core hands
//! each target's missing set over (`GlesDraw::target_missing_channels`,
//! `framebuffer::fb_target_missing_channels_n`) and the PowerVR's PBE writes
//! SWIZ.ZERO / SWIZ.ONE in those lanes (src/pvr/backend.rs `pbe_swizzle`),
//! for the clear and every draw alike.
//!
//! The GL calls run against the REAL PowerVR back end (`yakogl_backend_pvr`)
//! on a host platform written here (the one of tests/es3_half_float_rt_pvr.rs):
//! GPU memory is host memory and every submit completes at once. Its submit
//! hook records each kick's PBE REG words and stands in for the PBE's store:
//! it writes the shader's colour into every pixel of each target through the
//! recorded swizzle and pack mode (`pbe_store`), so glReadPixels reads what
//! the hardware would have stored. Pinned, per format, renderbuffer and
//! texture, after a clear alone and after a clear and a draw: the REG_WORD0
//! lanes, and the read-back of the missing channels (0, and 1 for alpha).
//! Also: a target with every channel (RGBA8) and the integer and
//! single-channel float surfaces keep the identity lanes, and a second
//! colour attachment gets its own lanes.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_fbo_channels
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
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::glClearBufferuiv;
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

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
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
/// The PBE STATE word pairs (one per emitted target) of the last end-of-tile
/// program built: where each target is and how it is packed.
static EMITS: Mutex<Vec<[u32; 2]>> = Mutex::new(Vec::new());
/// The PBE REG word pairs (one per emitted target) of each kick submitted.
static KICKS: Mutex<Vec<Vec<[u64; 2]>>> = Mutex::new(Vec::new());
/// The colour the stand-in PBE stores into every pixel of a frame's targets:
/// the fragment shader's output, which the clear colour equals.
const SOURCE: [f32; 4] = [0.25, 0.5, 0.75, 0.5];

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

/// The "GPU": records the kick's PBE REG words, stores [`SOURCE`] into every
/// pixel of every target as the PBE would ([`pbe_store`]) and completes it.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    /* num_rt 0 means one */
    let n = (k.frag.num_rt.max(1) as usize).min(k.frag.pbe_word.len());
    let regs = k.frag.pbe_word[..n].to_vec();
    let emits = EMITS.lock().unwrap_or_else(|e| e.into_inner()).clone();
    with_platform(|p| {
        for (reg, emit) in regs.iter().zip(&emits) {
            pbe_store(p, *emit, reg[0]);
        }
    });
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).push(regs);
    with_platform(|p| {
        p.fence += 1;
        p.fence
    })
}

/// The PBE's store of one target, as pbestate.xml describes it: memory lane
/// `i` takes REG_WORD0.swiz_chan`i` (bits [20 + 3i, 22 + 3i]):
/// SOURCE_CHAN0..3 (0..3) that source channel, SWIZ.ONE (4) the constant 1,
/// SWIZ.ZERO (5) 0; packed as STATE_WORD1.packmode [7:12] says (U8U8U8U8 0:
/// bytes, F16F16F16F16 10: halves; any other is left alone). The target is
/// the allocation at STATE_WORD0's address (address bits [2:31] in [0:29]),
/// `SIZE` square at a pitch of `SIZE`.
fn pbe_store(p: &mut Platform, emit: [u32; 2], reg0: u64) {
    let low = (emit[0] as u64 & 0x3fff_ffff) << 2;
    let Some((_, a)) = p.allocs.iter_mut().find(|(gpu, _)| **gpu & 0xffff_fffc == low) else { return };
    let lane = |i: u64| match (reg0 >> (20 + 3 * i)) & 7 {
        c @ 0..=3 => SOURCE[c as usize],
        4 => 1.0,
        _ => 0.0,
    };
    let px = [lane(0), lane(1), lane(2), lane(3)];
    let bytes: Vec<u8> = match (emit[1] >> 7) & 0x3f {
        0 => px.iter().map(|v| (v * 255.0).round() as u8).collect(),
        10 => px.iter().flat_map(|&v| f16_bits(v).to_le_bytes()).collect(),
        _ => return,
    };
    let pixels = (SIZE * SIZE) as usize;
    if pixels * bytes.len() > a.mem.len() * 8 {
        return;
    }
    let mem = a.mem.as_mut_ptr() as *mut u8;
    for i in 0..pixels {
        // SAFETY: inside the allocation (checked above).
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr(), mem.add(i * bytes.len()), bytes.len()) };
    }
}

/// A half's bits, for 0 and the normal values of [`SOURCE`] and 1 (all exact).
fn f16_bits(v: f32) -> u16 {
    if v == 0.0 {
        return 0;
    }
    let b = v.to_bits();
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    (((b >> 31) as u16) << 15) | ((exp as u16) << 10) | ((b >> 13) & 0x3ff) as u16
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
    num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    // SAFETY: the backend's emit words (a pair per target), buffers and sizes.
    unsafe {
        let mut e = EMITS.lock().unwrap_or_else(|e| e.into_inner());
        e.clear();
        for rt in 0..num_rt.max(1) as usize {
            e.push([*emit_words.add(2 * rt), *emit_words.add(2 * rt + 1)]);
        }
        drop(e);
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the context ------------------------------------------------------------- */

const CANVAS: i32 = 16;
/// The target's size: a whole tile, so the stride is the width.
const SIZE: i32 = 16;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAM: GLuint = 0;
static mut PROGRAM2: GLuint = 0;

const VS: &CStr = c"#version 300 es
layout(location = 0) in vec4 pos;
void main() { gl_Position = pos; }
";
/// Writes [`SOURCE`].
const FS: &CStr = c"#version 300 es
precision highp float;
layout(location = 0) out vec4 color;
void main() { color = vec4(0.25, 0.5, 0.75, 0.5); }
";
/// Writes [`SOURCE`] to two targets.
const FS2: &CStr = c"#version 300 es
precision highp float;
layout(location = 0) out vec4 c0;
layout(location = 1) out vec4 c1;
void main() { c0 = vec4(0.25, 0.5, 0.75, 0.5); c1 = c0; }
";

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

fn program(fs: &CStr) -> GLuint {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        p
    }
}

/// The one context and backend of the file, set up once; the tests take turns.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS as u32, CANVAS as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
            PROGRAM = program(FS);
            PROGRAM2 = program(FS2);
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

/// The two ways to give a framebuffer a colour image.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Renderbuffer,
    Texture,
}

/// A `SIZE`-square image of `internal` at colour attachment `att` of the
/// bound framebuffer.
fn attach(kind: Kind, internal: GLenum, att: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, internal, SIZE, SIZE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, att, GL_RENDERBUFFER, name);
            }
            Kind::Texture => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, 1, internal, SIZE, SIZE);
                glFramebufferTexture2D(GL_FRAMEBUFFER, att, GL_TEXTURE_2D, name, 0);
            }
        }
    }
    name
}

/// A new framebuffer, bound, with an image of `internal` at attachment 0.
fn target(kind: Kind, internal: GLenum, label: &str) -> (GLuint, GLuint) {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let name = attach(kind, internal, GL_COLOR_ATTACHMENT0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{label}: storage");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{label}: status");
        (fb, name)
    }
}

fn release(kind: Kind, fb: GLuint, names: &[GLuint]) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        for name in names {
            match kind {
                Kind::Renderbuffer => glDeleteRenderbuffers(1, name),
                Kind::Texture => glDeleteTextures(1, name),
            }
        }
        glFinish();
    }
}

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// The PBE REG word 0 of each target of the one kick submitted since
/// `KICKS` was emptied.
fn the_kick() -> Vec<u64> {
    let kicks = KICKS.lock().unwrap().clone();
    assert_eq!(kicks.len(), 1, "one kick: {:?}", logs());
    kicks[0].iter().map(|w| w[0]).collect()
}

/// Clears the bound framebuffer's targets to [`SOURCE`], draws a triangle
/// over them with `program` when `draw`, and submits: [`the_kick`].
fn frame(draw: bool, program: GLuint) -> Vec<u64> {
    KICKS.lock().unwrap().clear();
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        glViewport(0, 0, SIZE, SIZE);
        glClearColor(SOURCE[0], SOURCE[1], SOURCE[2], SOURCE[3]);
        glClear(GL_COLOR_BUFFER_BIT);
        if draw {
            glUseProgram(program);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glFinish();
    }
    the_kick()
}

/// REG_WORD0's four lanes (swiz_chan0..3, bits [20:31], pbestate.xml).
fn lanes(w0: u64) -> [u64; 4] {
    [w0 >> 20 & 7, w0 >> 23 & 7, w0 >> 26 & 7, w0 >> 29 & 7]
}

/* SWIZ.ONE and SWIZ.ZERO (SOURCE_CHAN0..3 are 0..3) */
const ONE: u64 = 4;
const ZERO: u64 = 5;

/// glReadPixels of pixel (3, 5) of the bound read framebuffer as four floats
/// in [0, 1]: GL_RGBA / GL_UNSIGNED_BYTE for a byte store, GL_RGBA /
/// GL_FLOAT for a half-float one.
fn read(float: bool, label: &str) -> [f32; 4] {
    // SAFETY: room for one pixel each; plain GL calls.
    unsafe {
        let v = if float {
            let mut px = [0f32; 4];
            glReadPixels(3, 5, 1, 1, GL_RGBA, GL_FLOAT, px.as_mut_ptr().cast());
            px
        } else {
            let mut px = [0u8; 4];
            glReadPixels(3, 5, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr().cast());
            px.map(|b| b as f32 / 255.0)
        };
        assert_eq!(glGetError(), GL_NO_ERROR, "{label}: glReadPixels");
        v
    }
}

/// What GL says a format with the first `channels` of R, G, B, A reads after
/// [`SOURCE`] was written (bytes rounded as the store rounds them).
fn expected(channels: usize, float: bool) -> [f32; 4] {
    let q = |v: f32| if float { v } else { (v * 255.0).round() / 255.0 };
    let mut e = [0.0, 0.0, 0.0, 1.0];
    for (k, ek) in e.iter_mut().enumerate().take(channels) {
        *ek = q(SOURCE[k]);
    }
    e
}

/// The lanes a target with the first `channels` of R, G, B, A takes.
fn lanes_of(channels: usize) -> [u64; 4] {
    let mut l = [0, 1, 2, 3];
    for (k, lk) in l.iter_mut().enumerate().skip(channels) {
        *lk = if k == 3 { ONE } else { ZERO };
    }
    l
}

/// One row: GL name, label, the channels the format has, a half-float store.
const ROWS: [(GLenum, &str, usize, bool); 5] = [
    (GL_R8, "R8", 1, false),
    (GL_RG8, "RG8", 2, false),
    (GL_RG16F, "RG16F", 2, true),
    (GL_RGB8, "RGB8", 3, false),
    (GL_RGB16F, "RGB16F", 3, true),
];

fn check_rows(kind: Kind, draw: bool) {
    for (internal, name, channels, float) in ROWS {
        let label = format!("{name} {kind:?} {}", if draw { "clear + draw" } else { "clear" });
        let (fb, image) = target(kind, internal, &label);
        // SAFETY: read under the lock, set once in `rig`.
        let words = frame(draw, unsafe { PROGRAM });
        assert_eq!(lanes(words[0]), lanes_of(channels), "{label}: PBE REG_WORD0 lanes");
        assert_eq!(read(float, &label), expected(channels, float), "{label}: read back");
        release(kind, fb, &[image]);
    }
}

/* ---- the tests --------------------------------------------------------------- */

/// A renderbuffer of each format, cleared: its missing lanes are SWIZ.ZERO
/// (colour) and SWIZ.ONE (alpha), and they read back 0 and 1.
#[test]
fn renderbuffer_clear_keeps_the_missing_channels() {
    let _g = rig();
    check_rows(Kind::Renderbuffer, false);
}

/// The same, cleared and drawn into.
#[test]
fn renderbuffer_draw_keeps_the_missing_channels() {
    let _g = rig();
    check_rows(Kind::Renderbuffer, true);
}

/// A texture of each format, cleared.
#[test]
fn texture_clear_keeps_the_missing_channels() {
    let _g = rig();
    check_rows(Kind::Texture, false);
}

/// A texture of each format, cleared and drawn into.
#[test]
fn texture_draw_keeps_the_missing_channels() {
    let _g = rig();
    check_rows(Kind::Texture, true);
}

/// A target with every channel and a single-channel float one keep the
/// identity lanes, and so does an integer one: their stores are exactly
/// their formats.
#[test]
fn whole_formats_keep_the_identity_lanes() {
    let _g = rig();
    /* R16F's alpha lane was SWIZ.ONE before (`target_alpha_bits` 0); its
     * F16 pack mode stores channel 0 only, so no lane is zeroed */
    for (internal, name, want) in [(GL_RGBA8, "RGBA8", [0, 1, 2, 3]), (GL_R16F, "R16F", [0, 1, 2, ONE])] {
        let (fb, rb) = target(Kind::Renderbuffer, internal, name);
        // SAFETY: read under the lock, set once in `rig`.
        let words = frame(true, unsafe { PROGRAM });
        assert_eq!(lanes(words[0]), want, "{name}: lanes");
        release(Kind::Renderbuffer, fb, &[rb]);
    }
    let (fb, rb) = target(Kind::Renderbuffer, GL_RG8UI, "RG8UI");
    KICKS.lock().unwrap().clear();
    // SAFETY: plain GL calls; `v` holds four values.
    unsafe {
        let v = [1u32, 2, 3, 4];
        glClearBufferuiv(GL_COLOR, 0, v.as_ptr());
        assert_eq!(glGetError(), GL_NO_ERROR, "RG8UI: {:?}", logs());
        glFinish();
    }
    assert_eq!(lanes(the_kick()[0]), [0, 1, 2, 3], "RG8UI: lanes");
    release(Kind::Renderbuffer, fb, &[rb]);
}

/// A second colour attachment takes its own lanes: RGBA8 at attachment 0
/// (identity), RG8 at attachment 1 (blue zero, alpha one), and each reads
/// back as its format says.
#[test]
fn a_second_attachment_keeps_its_own_missing_channels() {
    let _g = rig();
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let a0 = attach(Kind::Renderbuffer, GL_RGBA8, GL_COLOR_ATTACHMENT0);
    let a1 = attach(Kind::Renderbuffer, GL_RG8, GL_COLOR_ATTACHMENT1);
    // SAFETY: plain GL calls; `bufs` holds two names.
    unsafe {
        let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
        glDrawBuffers(2, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    // SAFETY: read under the lock, set once in `rig`.
    let words = frame(true, unsafe { PROGRAM2 });
    assert_eq!(words.len(), 2, "two targets emitted: {:?}", logs());
    assert_eq!(lanes(words[0]), [0, 1, 2, 3], "RGBA8 at attachment 0");
    assert_eq!(lanes(words[1]), [0, 1, ZERO, ONE], "RG8 at attachment 1");
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT1) };
    assert_eq!(read(false, "RG8 at attachment 1"), expected(2, false), "RG8 at attachment 1: read back");
    // SAFETY: plain GL call.
    unsafe { glReadBuffer(GL_COLOR_ATTACHMENT0) };
    assert_eq!(read(false, "RGBA8 at attachment 0"), expected(4, false), "RGBA8 at attachment 0: read back");
    release(Kind::Renderbuffer, fb, &[a0, a1]);
}
