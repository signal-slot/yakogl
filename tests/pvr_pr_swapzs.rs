// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: the partial render's depth/stencil words on a frame whose Z/S
//! store the swap dropped (docs/notes/pvr-pr-zs.md, "Swapped frame").
//!
//! A swap/present turns the frame's ZLS store bits off (`swap_drops_zs`,
//! `pvr_backend_surface_swapped`): nothing reads the depth after the swap. A
//! partial render does read it: the PR stores into, and the resumed render
//! loads from, the frame's own depth buffer, so the 3D_PR's `isp_zlsctl`
//! must have the store bits on (`pr_zs_words`, src/pvr/pr_zs.rs) while the
//! normal 3D keeps them off.
//!
//! These tests drive a depth-tested frame that ends in a swap through the GL
//! core into the REAL PowerVR back end (`yakogl_backend_pvr`), on a platform
//! written here (host memory, every submit completed at once; stand-ins for
//! the C command generators as in tests/pvr_pds_once.rs), capture the kick
//! it submits and read its `frag` and `frag_pr` blocks, with
//! `PVR_BACKEND_OPT_PARTIAL_RENDER` off and on.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_pr_swapzs
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
use yakogl::pvr::pr_zs::pr_zs_words;

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
    /// Every kick the back end submitted.
    kicks: Vec<PvrRenderKick>,
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

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: the length says the caller passes a PvrRenderKick.
    let k = unsafe { core::ptr::read_unaligned(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.kicks.push(k);
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

const CANVAS: (u32, u32) = (64, 48);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

/// The window surface's colour buffer (its GPU address names the surface for
/// `pvr_backend_surface_swapped`) and the context rendering into it.
static mut WINDOW: (u64, *mut context::GlContext) = (0, core::ptr::null_mut());
static mut PROG: GLuint = 0;

/// The backend and a one-sample window context with a depth buffer, set up
/// once; the tests take turns, each with every kick submitted.
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
            let gpu = surface.gpu;
            let c = context::gl_context_create_for_surface(
                surface,
                CANVAS.0,
                CANVAS.1,
                GLES_SURF_RGBA8888,
                1,
                context::GROUP_NONE,
            );
            assert!(!c.is_null(), "{:?}", logs());
            WINDOW = (gpu, c);
            context::gl_context_set_current(c);
            PROG = program("passthrough.vert", "uniform_color.frag");
        }
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

/* a quad, (x, y, z, w) per vertex, drawn as two indexed triangles */
static POS: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0];
static IDX: [u16; 6] = [0, 1, 2, 0, 2, 3];

/// One depth-tested frame (colour and depth cleared, two draws of the quad),
/// ended by a flush: after a `swap` call, as eglSwapBuffers does. The kick
/// the flush submitted.
fn frame(swap: bool) -> PvrRenderKick {
    // SAFETY: set once in `rig`, read under its lock; plain GL calls.
    unsafe {
        let (gpu, c) = WINDOW;
        context::gl_context_set_current(c);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        with_platform(|p| p.kicks.clear());
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        glDepthFunc(GL_LESS);
        glUseProgram(PROG);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        for _ in 0..2 {
            glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, IDX.as_ptr() as *const c_void);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        if swap {
            pvr_backend_surface_swapped(gpu);
        }
        glFlush();
        glDisable(GL_DEPTH_TEST);
    }
    let kicks = with_platform(|p| core::mem::take(&mut p.kicks));
    assert_eq!(kicks.len(), 1, "one kick for the frame: {:?}", logs());
    kicks[0]
}

/// `frame(swap)` with the partial-render option `on` (restored off).
fn frame_with(on: bool, swap: bool) -> PvrRenderKick {
    /* a swapped frame first: it leaves the depth buffer not valid, so every
     * measured frame starts from the same state (no load bits) */
    pvr_backend_set_option(PVR_BACKEND_OPT_PARTIAL_RENDER, 0);
    frame(true);
    pvr_backend_set_option(PVR_BACKEND_OPT_PARTIAL_RENDER, on as u32);
    let k = frame(swap);
    pvr_backend_set_option(PVR_BACKEND_OPT_PARTIAL_RENDER, 0);
    k
}

/// The three ZLS store bits, as the partial render's word builder ORs them in.
fn store_mask() -> u64 {
    pr_zs_words(0, 0, 0, 0, 0).isp_zlsctl
}

/// A block's bytes.
fn bytes<T>(v: &T) -> Vec<u8> {
    // SAFETY: plain-old-data repr(C) structs read as bytes.
    unsafe { core::slice::from_raw_parts(v as *const T as *const u8, size_of::<T>()) }.to_vec()
}

/* ---- the tests --------------------------------------------------------------- */

/// Option off: the swap drops the store bits of the normal 3D (and only
/// those), and the partial-render block stays all zero.
#[test]
fn option_off_swapped_frame_has_no_pr_block_and_todays_words() {
    let _g = rig();
    let (plain, swapped) = (frame_with(false, false), frame_with(false, true));
    assert_ne!(store_mask(), 0);
    assert_eq!(plain.frag.isp_zlsctl & store_mask(), store_mask(), "an unswapped frame stores depth/stencil");
    assert_eq!(swapped.frag.isp_zlsctl & store_mask(), 0, "the swap drops the store bits");
    assert_eq!(swapped.frag.isp_zlsctl | store_mask(), plain.frag.isp_zlsctl, "only the store bits differ");
    /* the frame's depth buffer is still named (the swap does not unbind it) */
    assert_ne!(swapped.frag.isp_zload_store_base, 0);
    assert_eq!(swapped.frag.isp_zload_store_base, plain.frag.isp_zload_store_base);
    assert!(bytes(&plain.frag_pr).iter().all(|&b| b == 0), "{:?}", plain.frag_pr);
    assert!(bytes(&swapped.frag_pr).iter().all(|&b| b == 0), "{:?}", swapped.frag_pr);
}

/// Option on: the normal 3D command's words are what they are with it off
/// (the swap's store bits still dropped), and the frame is partial-render
/// eligible: its block carries state.
#[test]
fn option_on_swapped_frame_keeps_the_normal_3d_words() {
    let _g = rig();
    let (off, on) = (frame_with(false, true), frame_with(true, true));
    /* (not the whole block: it holds per-frame arena addresses) */
    let zs = |f: &yakogl::osal::render::PvrRenderFragRegs| {
        (f.isp_zlsctl, f.isp_zload_store_base, f.isp_stencil_load_store_base, f.isp_bgobjdepth, f.cmd_flags)
    };
    assert_eq!(zs(&off.frag), zs(&on.frag), "the option changes the normal 3D command's Z/S words");
    assert_eq!(on.frag.isp_zlsctl & store_mask(), 0, "the normal 3D keeps the swap's store bits off");
    assert_ne!(on.frag_pr.spm_scratch_gpu, 0, "the frame carries no PR state: {:?} {:?}", on.frag_pr, logs());
}

/// The builder, applied to the swapped frame's own words: the PR's word
/// stores depth/stencil and reloads at the frame's depth buffer.
#[test]
fn pr_zs_words_on_the_swapped_frames_words_store_at_the_frames_buffer() {
    let _g = rig();
    let k = frame_with(true, true).frag;
    let w = pr_zs_words(
        k.isp_zlsctl,
        k.isp_zload_store_base,
        k.isp_stencil_load_store_base,
        k.isp_bgobjdepth as u64,
        k.cmd_flags,
    );
    assert_eq!(w.isp_zlsctl & store_mask(), store_mask());
    assert_eq!(w.isp_zlsctl & !store_mask(), k.isp_zlsctl & !store_mask());
    assert_eq!(w.zload_base, k.isp_zload_store_base);
    assert_eq!(w.stencil_load_base, k.isp_stencil_load_store_base);
}

/// (b) The kick's `frag_pr` block must carry the PR's Z/S words: the store
/// bits on, at the frame's depth buffer. Not so at the tip: `pr_fill`
/// (src/pvr/backend.rs) fills the colour state only, `PvrRenderFragPr` has
/// no Z/S field, and `pr_zs_words` is called by nothing outside its tests
/// (its module header says "nothing here is wired into a kick yet"). The
/// driver copies the normal 3D's words, whose store bits the swap dropped:
/// the resumed render would load depth the PR never stored. The block is
/// scanned for the words rather than read by field, so the test does not
/// care which layout the field lands in.
#[test]
#[ignore = "defect: frag_pr carries no Z/S words; the PR on a swapped frame would use the 3D's store-off ZLSCTL"]
fn option_on_swapped_frame_pr_block_stores_and_loads_depth() {
    let _g = rig();
    let k = frame_with(true, true);
    let w = pr_zs_words(
        k.frag.isp_zlsctl,
        k.frag.isp_zload_store_base,
        k.frag.isp_stencil_load_store_base,
        k.frag.isp_bgobjdepth as u64,
        k.frag.cmd_flags,
    );
    let pr = bytes(&k.frag_pr);
    let has = |v: u64| pr.as_chunks::<8>().0.iter().any(|c| u64::from_le_bytes(*c) == v);
    assert!(has(w.isp_zlsctl), "no ZLSCTL with the store bits on in {:?}", k.frag_pr);
    assert!(has(w.zload_base), "the frame's depth buffer is not named in {:?}", k.frag_pr);
}
