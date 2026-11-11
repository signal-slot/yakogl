// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR flat shading on the line topologies tests/es3_pvr_flatshade.rs
//! does not cover: GL_LINE_LOOP, and the wide forms of GL_LINES,
//! GL_LINE_STRIP and GL_LINE_LOOP (rasterization.flatshading.line_loop and
//! *_wide; docs/notes/es3-flatshading.md section 2, host test 1-3).
//!
//! A flat varying takes the provoking vertex, the LAST vertex of each
//! primitive; for a loop's closing segment (n-1, 0) that is vertex 0. Mesa's
//! PowerVR driver writes VERTEX_2 ("the primitive's last vertex") to the PPP
//! and VDM words for every topology, so the checks here are that the loop and
//! the wide lines keep the same words as the thin lines:
//!
//!  1. GL_LINE_LOOP draws the native LINE_LOOP topology with PPP and VDM
//!     VERTEX_2, as a LINE object.
//!  2. glLineWidth(5) on the three line modes keeps VERTEX_2 and writes
//!     ISPA.pointlinewidth 79 (5 * 16 - 1); width 1 writes 15.
//!  3. A line-loop draw too large for one piece goes whole (one index list
//!     of every index), so the closing segment stays in the draw, while a
//!     line strip of the same arrays is cut.
//!  4. A real ES 3.00 `flat out vec4` drawn as each line mode iterates that
//!     varying FLAT in the coefficient program (and the smooth one not).
//!
//! The words are read from the command stream through the REAL PowerVR back
//! end on a platform written here, with stand-ins for the C generators that
//! record their inputs (as tests/es3_pvr_flatshade.rs does). Which vertex the
//! TA takes under VERTEX_2 for the loop's closing segment and for a widened
//! line is not visible on the host: that is the AM62P board check. The
//! program is ES 3.00, so the file compiles to nothing under report-es2.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_flatshade_lines
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

/// What one draw's PPP block said: the flat vertex, the object type, the
/// line width, and the coefficient program's varyings (components, interp).
#[derive(Debug, Clone, PartialEq, Eq)]
struct DrawRec {
    flat_ppp: u32,
    objtype: u32,
    point_line_width: u32,
    varyings: Vec<(u32, u32)>,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    draws: Vec<DrawRec>,
    /// VDM_STATE0.flatshade_control of each VDM state, the (topology, index
    /// count) of each index list.
    vdm_flat: Vec<u32>,
    lists: Vec<(u32, u32)>,
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
/// The coefficient stand-in's first word: its tag and (low 16 bits) how many
/// words it has.
const COEFF_TAG: u32 = 0xC0EF_0000;

/// The PPP stand-in: records the draw's PPP state and its coefficient
/// program's varyings.
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
        /* 0: the dummy program, no varying iterated */
        let varyings = if fi.varying_program_code_offset == 0 {
            Vec::new()
        } else {
            let head = pds_words(p, fi.varying_program_code_offset, 1)[0];
            assert_eq!(head & 0xFFFF_0000, COEFF_TAG, "the PPP block points at no coefficient program");
            let w = pds_words(p, fi.varying_program_code_offset, (head & 0xFFFF) as usize);
            /* tag, uses_w, uses_z, uses_pntc, then (components, interp,
             * vtxout_off) per varying, then two code words */
            w[4..w.len() - 2].as_chunks::<3>().0.iter().map(|v| (v[0], v[1])).collect()
        };
        p.draws.push(DrawRec {
            flat_ppp: st.flatshade_vtx,
            objtype: st.objtype,
            point_line_width: st.point_line_width,
            varyings,
        });
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
    with_platform(|p| p.vdm_flat.push(v.flatshade_control));
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
    with_platform(|p| p.lists.push((topology, count)));
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
    sample_rate: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderKickUscInfo,
) -> i32 {
    // SAFETY: the backend's buffers (8 data words, 1 code word) and record.
    unsafe {
        let w = [0x4B00_0009, fs_usc_exec_offset as u32, fs_temps, sample_rate, 0x4B4B_0004, 5, 6, 7];
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
/// varying's components, interpolation and output offset).
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
static mut CTX: *mut context::GlContext = core::ptr::null_mut();
static mut PROG: GLuint = 0;

/// dEQP's flatshading shape: a flat colour next to a smooth one, so the
/// coefficient program shows both interpolations.
const VS: &str = "#version 300 es
layout(location = 0) in highp vec4 a_pos;
layout(location = 1) in mediump vec4 a_col;
flat out mediump vec4 v_flat;
out mediump vec4 v_smooth;
void main() { gl_Position = a_pos; v_flat = a_col; v_smooth = a_col; }
";
const FS: &str = "#version 300 es
flat in mediump vec4 v_flat;
in mediump vec4 v_smooth;
layout(location = 0) out mediump vec4 o;
void main() { o = v_flat + 0.0 * v_smooth; }
";

/// The context and the program, set up once; the tests take turns. Each
/// starts with nothing pending, line width 1 and the recorders empty.
fn rig() -> (MutexGuard<'static, ()>, GLuint) {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            /* the ES 3.00 program is not in the offline table */
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0, CANVAS.1);
            assert!(!CTX.is_null(), "{:?}", logs());
            context::gl_context_set_current(CTX);
            PROG = program(VS, FS);
        }
    });
    // SAFETY: set once above, read under the lock; plain GL calls.
    let prog = unsafe {
        context::gl_context_set_current(CTX);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        glLineWidth(1.0);
        PROG
    };
    with_platform(|p| {
        p.draws.clear();
        p.vdm_flat.clear();
        p.lists.clear();
    });
    (g, prog)
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(type_: GLenum, src: &str) -> GLuint {
    let csrc = std::ffi::CString::new(src).unwrap();
    // SAFETY: plain GL calls; a NUL-terminated source.
    unsafe {
        let s = glCreateShader(type_);
        let p = csrc.as_ptr();
        glShaderSource(s, 1, &p, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{src}: compile failed ({:?})", logs());
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
        assert_eq!(ok, 1, "the flat program: link failed ({:?})", logs());
        p
    }
}

/// glDrawElements of `idx` as `mode` with client arrays: (x, y) float
/// positions and one float RGBA colour per vertex, then the frame's end.
fn draw(prog: GLuint, mode: GLenum, pos: &[f32], col: &[f32], idx: &[u16]) {
    // SAFETY: plain GL calls; the arrays outlive the draw (glFlush below).
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glUseProgram(prog);
        glVertexAttribPointer(0, 2, GL_FLOAT, 0, 0, pos.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, col.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawElements(mode, idx.len() as GLsizei, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glFlush();
    }
}

/// Four vertices on a square, a distinct colour each (dEQP's shape).
static POS4: [f32; 8] = [-0.5, -0.5, 0.5, -0.5, 0.5, 0.5, -0.5, 0.5];
static COL4: [f32; 16] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0];
static IDX4: [u16; 4] = [0, 1, 2, 3];

/* ---- the tests --------------------------------------------------------------- */

/// TA_STATE_PPP_CTRL.flatshade_vtx VERTEX_2 and VDMCTRL_FLATSHADE_CONTROL
/// VERTEX_2 (ppp.xml FLATSHADE 3, vdm.xml FLATSHADE_CONTROL 2).
const PPP_VERTEX_2: u32 = 3;
const VDM_VERTEX_2: u32 = 2;

/// What the last draw of the frame left: its PPP record, its VDM word and
/// its index lists (the clear leaves blocks of its own before it).
fn last_draw(mode: GLenum) -> (DrawRec, u32, Vec<(u32, u32)>) {
    with_platform(|p| {
        assert!(!p.draws.is_empty(), "mode 0x{mode:x}: no PPP block ({:?})", p.log);
        assert!(!p.vdm_flat.is_empty(), "mode 0x{mode:x}: no VDM state");
        assert!(!p.lists.is_empty(), "mode 0x{mode:x}: no index list");
        (p.draws.last().unwrap().clone(), *p.vdm_flat.last().unwrap(), p.lists.clone())
    })
}

/// Draw the square as `mode` at `width`, and check the line, the flat words
/// and the line width it leaves.
fn check_line(mode: GLenum, topology: u32, width: f32, isp_width: u32) {
    let (_g, prog) = rig();
    // SAFETY: plain GL call.
    unsafe { glLineWidth(width) };
    draw(prog, mode, &POS4, &COL4, &IDX4);
    let (rec, vdm, lists) = last_draw(mode);
    let what = format!("mode 0x{mode:x} width {width}");
    assert_eq!(lists.last().unwrap(), &(topology, 4), "{what}: the VDM topology and count");
    assert_eq!(rec.objtype, PVR_TA_OBJTYPE_LINE, "{what}: a LINE object");
    assert_eq!(rec.point_line_width, isp_width, "{what}: ISPA.pointlinewidth");
    assert_eq!(rec.flat_ppp, PPP_VERTEX_2, "{what}: TA_STATE_PPP_CTRL.flatshade_vtx is the last vertex");
    assert_eq!(vdm, VDM_VERTEX_2, "{what}: VDM_STATE0.flatshade_control is the last vertex");
}

#[test]
fn line_loops_flatshade_from_the_last_vertex() {
    check_line(GL_LINE_LOOP, PVR_VDM_TOPOLOGY_LINE_LOOP, 1.0, 15);
}

#[test]
fn wide_lines_flatshade_from_the_last_vertex() {
    check_line(GL_LINES, PVR_VDM_TOPOLOGY_LINE_LIST, 5.0, 79);
}

#[test]
fn wide_line_strips_flatshade_from_the_last_vertex() {
    check_line(GL_LINE_STRIP, PVR_VDM_TOPOLOGY_LINE_STRIP, 5.0, 79);
}

#[test]
fn wide_line_loops_flatshade_from_the_last_vertex() {
    check_line(GL_LINE_LOOP, PVR_VDM_TOPOLOGY_LINE_LOOP, 5.0, 79);
}

/// The flat output is iterated FLAT (one 4-component varying) and the
/// smooth one is not, on every line mode, thin and wide: the coefficient
/// program's input, which the real generator turns into DOUTI FLAT_VERTEX2
/// (tests/es3_pvr_flatshade.rs runs that generator).
#[test]
fn the_flat_varying_is_iterated_flat_on_every_line_mode() {
    for mode in [GL_LINES, GL_LINE_STRIP, GL_LINE_LOOP] {
        for width in [1.0, 5.0] {
            let (_g, prog) = rig();
            // SAFETY: plain GL call.
            unsafe { glLineWidth(width) };
            draw(prog, mode, &POS4, &COL4, &IDX4);
            let (rec, _, _) = last_draw(mode);
            let what = format!("mode 0x{mode:x} width {width}: {:?}", rec.varyings);
            let flat: Vec<_> = rec.varyings.iter().filter(|v| v.1 == PVR_PDS_COEFF_FLAT).collect();
            assert_eq!(flat, [&(4, PVR_PDS_COEFF_FLAT)], "{what}: one flat vec4");
            assert!(
                rec.varyings.iter().any(|&(c, i)| c == 4 && i != PVR_PDS_COEFF_FLAT),
                "{what}: the smooth vec4 is not flat"
            );
        }
    }
}

/// Vertices of the large draws: client arrays of 25,000 * (8 + 16) bytes and
/// 50,000 bytes of indices, more than half the PowerVR's default 1 MiB
/// GENERAL arena (src/pvr/backend.rs draw_dispatch cuts such a draw into
/// pieces), but less than all of it, so a draw that is not cut still fits.
const BIG: usize = 25_000;

fn big_arrays() -> (Vec<f32>, Vec<f32>, Vec<u16>) {
    let pos = (0..BIG)
        .flat_map(|i| {
            let t = i as f32 / BIG as f32 * core::f32::consts::TAU;
            [0.9 * t.cos(), 0.9 * t.sin()]
        })
        .collect();
    let col = (0..BIG).flat_map(|i| [(i % 251) as f32 / 250.0, (i % 241) as f32 / 240.0, 0.0, 1.0]).collect();
    let idx = (0..BIG as u16).collect();
    (pos, col, idx)
}

/// A line loop too large for one piece is drawn whole: one LINE_LOOP index
/// list of every index, so the closing segment (n-1, 0) and its provoking
/// vertex 0 stay in the draw (cut_unit (0, 0)). The line strip of the same
/// arrays shows the draw is large enough to be cut.
#[test]
fn a_large_line_loop_is_not_cut() {
    let (pos, col, idx) = big_arrays();
    {
        let (_g, prog) = rig();
        draw(prog, GL_LINE_STRIP, &pos, &col, &idx);
        let (_, _, lists) = last_draw(GL_LINE_STRIP);
        let strips: Vec<_> = lists.iter().filter(|l| l.0 == PVR_VDM_TOPOLOGY_LINE_STRIP).collect();
        assert!(strips.len() > 1, "the large line strip is cut into pieces: {lists:?}");
    }
    let (_g, prog) = rig();
    draw(prog, GL_LINE_LOOP, &pos, &col, &idx);
    let (rec, vdm, lists) = last_draw(GL_LINE_LOOP);
    let loops: Vec<_> = lists.iter().filter(|l| l.0 == PVR_VDM_TOPOLOGY_LINE_LOOP).collect();
    assert_eq!(loops, [&(PVR_VDM_TOPOLOGY_LINE_LOOP, BIG as u32)], "the large line loop is one index list: {lists:?}");
    assert_eq!(rec.flat_ppp, PPP_VERTEX_2, "the large loop: TA_STATE_PPP_CTRL.flatshade_vtx");
    assert_eq!(vdm, VDM_VERTEX_2, "the large loop: VDM_STATE0.flatshade_control");
}
