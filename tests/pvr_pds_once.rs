// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: a frame generates the kick-USC and coefficient PDS programs once
//! per (program, variant, sample rate), and its later draws point at them
//! (docs/notes/pvr-draw-pds.md; fix F4 of docs/notes/gl-draw-fixed-cost.md).
//!
//! `record_draw` (src/pvr/backend.rs) ran both generators at every draw,
//! although their inputs (the fragment program's code offset and temps, the
//! sample rate; its W/Z/point-coord flags and varyings) are the same for
//! every draw of a program. `pds_put` already pointed a draw at equal words
//! the frame held, so the arena did not grow per draw; the generator, the
//! copy and the hash did run per draw.
//!
//! These tests drive glDrawElements through the GL core into the REAL
//! PowerVR back end (`yakogl_backend_pvr`), on a platform written here (host
//! memory, every submit completed at once) with stand-ins for the C command
//! generators (driver/src/pvr), as tests/depth_resolve_pvr_backend.rs does.
//! The stand-ins count their calls and write words that encode their inputs,
//! and the PPP stand-in reads, at each draw, the kick-USC and coefficient
//! words the draw points at. The same GL sequence runs with the cache on and
//! off (`pvr_backend_set_pds_once`): every draw must point at the same words
//! either way, and with it on the generators must run once per key and frame,
//! not once per draw.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_pds_once
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
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend::GLES_SURF_RGBA8888;
use yakogl::gl::consts::*;
use yakogl::gl::context;
use yakogl::gl::draw::glDrawElements;
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

/* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE). */
const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

/// What one draw pointed the PPP block at: the kick-USC program's words, the
/// coefficient program's words, and the varying sizes from its info.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DrawRec {
    kick: Vec<u32>,
    coeff: Vec<u32>,
    varyingsize: (u32, u32),
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    /// Generator calls: kick-USC, coefficient.
    kick_gens: u32,
    coeff_gens: u32,
    draws: Vec<DrawRec>,
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

unsafe extern "C" fn p_submit(_kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
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

/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

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

/// `n` words at PDS heap offset `off`, as the arena holds them now.
fn pds_words(p: &Platform, off: u32, n: usize) -> Vec<u32> {
    let gpu = HEAP_PDS_BASE + off as u64;
    let (&base, a) = p.allocs.range(..=gpu).next_back().expect("a PDS offset outside every allocation");
    let at = (gpu - base) as usize;
    assert!(at + 4 * n <= a.size, "PDS program at 0x{off:x} runs past its allocation");
    // SAFETY: inside the allocation (checked above); u64 backing, 4-aligned offset.
    unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(at) as *const u32, n) }.to_vec()
}

/* ---- the PowerVR's command generators (driver/src/pvr, C), stood in for ------- */

const W_TERM: u32 = 0xE000_0000;
/// Tags of the stand-ins' first words: a program's first word says what it is
/// and (low 16 bits) how many words it has.
const KICK_TAG: u32 = 0x4B00_0000;
const COEFF_TAG: u32 = 0xC0EF_0000;

/// The PPP stand-in: records what the draw's PPP block points at.
#[no_mangle]
unsafe extern "C" fn pvr_ppp_build_block(
    input: *const PvrPppFragInputs,
    st: *const PvrPppState,
    out: *mut u32,
    out_count: *mut u32,
    _why: *mut PvrVdmFieldRefusal,
) -> i32 {
    // SAFETY: the backend's inputs and state.
    let (fi, st) = unsafe { (*input, *st) };
    with_platform(|p| {
        let kick = pds_words(p, fi.pixel_program_code_offset, 9);
        assert_eq!(kick[0] & 0xFF00_0000, KICK_TAG, "the PPP block points at no kick-USC program");
        /* 0: the dummy program, no varying iterated */
        let coeff = if fi.varying_program_code_offset == 0 {
            Vec::new()
        } else {
            let head = pds_words(p, fi.varying_program_code_offset, 1)[0];
            assert_eq!(head & 0xFFFF_0000, COEFF_TAG, "the PPP block points at no coefficient program");
            pds_words(p, fi.varying_program_code_offset, (head & 0xFFFF) as usize)
        };
        p.draws.push(DrawRec { kick, coeff, varyingsize: (st.pds_varyingsize, st.usc_varyingsize) });
    });
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

/// Kick-USC stand-in: its words are its inputs.
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_kick_usc_program(
    fs_usc_exec_offset: u64,
    fs_temps: u32,
    sample_rate: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderKickUscInfo,
) -> i32 {
    with_platform(|p| p.kick_gens += 1);
    // SAFETY: the backend's buffers (8 data words, 1 code word) and record.
    unsafe {
        let w = [KICK_TAG | 9, fs_usc_exec_offset as u32, fs_temps, sample_rate, 0x4B4B_0004, 5, 6, 7];
        core::ptr::copy_nonoverlapping(w.as_ptr(), data_out, 8);
        *code_out = 0x4BC0_DE00;
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

/// Coefficient stand-in: its words are its inputs (flags, then each
/// varying's components, interpolation and output offset), its info sizes
/// follow the varying count.
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_coeff_program(
    uses_w: i32,
    uses_z: i32,
    uses_pntc: i32,
    varyings: *const PvrPdsCoeffVarying,
    num_varyings: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsCoeffInfo,
) -> i32 {
    with_platform(|p| p.coeff_gens += 1);
    let n = num_varyings as usize;
    let mut data = vec![0u32, uses_w as u32, uses_z as u32, uses_pntc as u32];
    for i in 0..n {
        // SAFETY: the backend passes num_varyings records.
        let v = unsafe { *varyings.add(i) };
        data.extend([v.components, v.interp, v.vtxout_off]);
    }
    let code = [0xC0DE_0000 | num_varyings, 0xC0DE_FFFF];
    data[0] = COEFF_TAG | (data.len() + code.len()) as u32;
    // SAFETY: the backend's buffers (2 * (16 + 3) + 2 data words: 52 >= 4 +
    // 3 * 16) and record.
    unsafe {
        core::ptr::copy_nonoverlapping(data.as_ptr(), data_out, data.len());
        core::ptr::copy_nonoverlapping(code.as_ptr(), code_out, code.len());
        *info = PvrPdsCoeffInfo {
            data_size_dw: data.len() as u32,
            code_size_dw: code.len() as u32,
            coeff_dwords: 4 * num_varyings,
            pds_varyingsize: num_varyings + 1,
            usc_varyingsize: num_varyings + 2,
        };
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

const CANVAS: (i32, i32) = (64, 48);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// A context and glmark2's two shader pairs made in it: a uniform colour
/// (no varying but the position) and a textured one (a uv varying).
#[derive(Clone, Copy)]
struct Ctx {
    ctx: *mut context::GlContext,
    programs: (GLuint, GLuint),
}

/// The file's contexts: a one-sample window, and a 4x window surface (the
/// GL core gives no multisampled renderbuffer; a 4x frame is the window's).
static mut CTXS: [Ctx; 2] = [Ctx { ctx: core::ptr::null_mut(), programs: (0, 0) }; 2];

/// The contexts and the backend, set up once; the tests take turns. Each
/// starts with the one-sample context current and every kick submitted.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            let surface: &'static mut OsalGpuMem = Box::leak(Box::new(OsalGpuMem::default()));
            let bytes = (CANVAS.0 * CANVAS.1 * 4) as usize;
            assert_eq!(p_gpu_alloc(HEAP_GENERAL_BASE, bytes, 0, surface), 0);
            let ms = context::gl_context_create_for_surface(
                surface,
                CANVAS.0 as u32,
                CANVAS.1 as u32,
                GLES_SURF_RGBA8888,
                4,
                context::GROUP_NONE,
            );
            assert!(!ms.is_null(), "{:?}", logs());
            CTXS[1] = Ctx { ctx: ms, programs: programs() };
            let one = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!one.is_null(), "{:?}", logs());
            CTXS[0] = Ctx { ctx: one, programs: programs() };
        }
    });
    use_ctx(0);
    g
}

fn programs() -> (GLuint, GLuint) {
    (program("passthrough.vert", "uniform_color.frag"), program("uv.vert", "texture.frag"))
}

/// Make context `i` of [`CTXS`] current, with nothing pending; its programs.
fn use_ctx(i: usize) -> (GLuint, GLuint) {
    // SAFETY: set once in `rig`, read under its lock; plain GL calls.
    unsafe {
        let c = CTXS[i];
        context::gl_context_set_current(c.ctx);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        c.programs
    }
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

/* a quad, (x, y, z, w) and (u, v) per vertex, drawn as two indexed triangles */
static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
static UV: [f32; 8] = [0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0];
static IDX: [u16; 6] = [0, 1, 2, 0, 2, 3];

/// One glDrawElements of the quad with program `prog`.
fn draw(prog: GLuint) {
    // SAFETY: plain GL calls; the arrays are statics.
    unsafe {
        glUseProgram(prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/// A frame's start: a colour and depth clear (no load quad).
fn clear() {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
}

/// The frame's end (eglSwapBuffers' submit).
fn end_frame() {
    // SAFETY: plain GL call.
    unsafe { glFlush() };
}

/// What a run left: every draw's record, and the generator calls of each
/// frame (kick-USC, coefficient).
#[derive(Debug)]
struct Run {
    draws: Vec<Vec<DrawRec>>,
    gens: Vec<(u32, u32)>,
}

/// Run `frames` (each a list of steps, one draw each) with the cache `once`.
fn run(once: bool, frames: &[&dyn Fn()]) -> Run {
    pvr_backend_set_pds_once(once);
    let mut r = Run { draws: Vec::new(), gens: Vec::new() };
    for f in frames {
        with_platform(|p| p.draws.clear());
        let before = with_platform(|p| (p.kick_gens, p.coeff_gens));
        f();
        end_frame();
        let (draws, after) = with_platform(|p| (core::mem::take(&mut p.draws), (p.kick_gens, p.coeff_gens)));
        r.draws.push(draws);
        r.gens.push((after.0 - before.0, after.1 - before.1));
    }
    pvr_backend_set_pds_once(true);
    r
}

/// The distinct kick-USC and coefficient programs a frame's draws point at.
fn distinct(draws: &[DrawRec]) -> (u32, u32) {
    let k: BTreeSet<_> = draws.iter().map(|d| &d.kick).collect();
    let c: BTreeSet<_> = draws.iter().filter(|d| !d.coeff.is_empty()).map(|d| &d.coeff).collect();
    (k.len() as u32, c.len() as u32)
}

/// Same draws, same words, generated once per key: the cache's contract.
fn check(on: &Run, off: &Run) {
    assert_eq!(on.draws, off.draws, "a draw points at other kick-USC / coefficient words with the cache");
    for (i, (draws, (gen_on, gen_off))) in off.draws.iter().zip(on.gens.iter().zip(&off.gens)).enumerate() {
        let n = draws.len() as u32;
        assert_eq!(*gen_off, (n, n), "frame {i}: without the cache every draw generates both programs");
        assert_eq!(*gen_on, distinct(draws), "frame {i}: with it, one generation per program and frame ({n} draws)");
    }
}

/* ---- the tests --------------------------------------------------------------- */

/// Many draws of one program in one frame: one kick-USC and one coefficient
/// program, whatever the draw count.
#[test]
fn one_program_generates_its_kick_and_coefficient_programs_once_a_frame() {
    let _g = rig();
    let (a, _) = use_ctx(0);
    let frame = || {
        clear();
        for _ in 0..64 {
            draw(a);
        }
    };
    let frames: [&dyn Fn(); 2] = [&frame, &frame];
    let (on, off) = (run(true, &frames), run(false, &frames));
    check(&on, &off);
    /* each frame: the clear's quad (the backend's clear program, whose
     * coefficient program, no varying, is the uniform-colour one's) and the
     * 64 draws */
    assert!(on.draws.iter().all(|f| f.len() == 65 && f[0].kick[1] != f[1].kick[1]), "{on:?}");
    assert_eq!(on.gens, [(2, 1), (2, 1)], "{on:?}");
}

/// Two programs alternating draw by draw: two of each, not one per draw; the
/// next frame generates them again (its arena starts empty).
#[test]
fn two_alternating_programs_generate_two_of_each_a_frame() {
    let _g = rig();
    let (a, b) = use_ctx(0);
    let frame = || {
        clear();
        for _ in 0..32 {
            draw(a);
            draw(b);
        }
    };
    let frames: [&dyn Fn(); 3] = [&frame, &frame, &frame];
    let (on, off) = (run(true, &frames), run(false, &frames));
    check(&on, &off);
    /* the clear's quad, then the two programs: three kick-USC programs, two
     * coefficient programs (the quad's is the uniform-colour one's) */
    assert!(on.draws.iter().all(|f| f.len() == 65), "{on:?}");
    assert_eq!(on.gens, [(3, 2); 3], "{on:?}");
}

/// A sample-rate change within a frame: into the 4x window, both programs
/// blended (per sample) and not (per pixel), alternating. Every draw keeps
/// its own rate's kick-USC program.
#[test]
fn a_sample_rate_change_within_a_frame_keeps_each_rates_kick_program() {
    let _g = rig();
    let (a, b) = use_ctx(1);
    // SAFETY: plain GL call.
    unsafe { glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA) };
    let frame = || {
        clear();
        for i in 0..24 {
            // SAFETY: plain GL calls.
            unsafe {
                if i % 2 == 0 {
                    glEnable(GL_BLEND)
                } else {
                    glDisable(GL_BLEND)
                }
            };
            draw(if i % 3 == 0 { a } else { b });
        }
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_BLEND) };
    };
    let frames: [&dyn Fn(); 2] = [&frame, &frame];
    let (on, off) = (run(true, &frames), run(false, &frames));
    check(&on, &off);
    /* both rates were drawn with: the kick words carry the rate (word 3) */
    let rates: BTreeSet<u32> = on.draws[0].iter().map(|d| d.kick[3]).collect();
    assert_eq!(rates.len(), 2, "per-pixel and per-sample kicks in one frame: {rates:?}");
    let (kicks, _) = distinct(&on.draws[0]);
    assert!(kicks >= 4 && on.gens[0].0 < 24, "{on:?}");
}
