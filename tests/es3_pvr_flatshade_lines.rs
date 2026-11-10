// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR: the line topologies tests/es3_pvr_flatshade.rs leaves out, the
//! rows `line_loop`, `lines_wide`, `line_strip_wide` and `line_loop_wide` of
//! dEQP-GLES3.functional.rasterization.flatshading
//! (docs/notes/es3-flatshading.md section 2, "Host test"):
//!
//!  1. `GL_LINE_LOOP` draws the native PVR_VDM_TOPOLOGY_LINE_LOOP (TA object
//!     LINE) with TA_STATE_PPP_CTRL.flatshade_vtx and
//!     VDM_STATE0.flatshade_control VERTEX_2, the words Mesa writes for the
//!     last vertex on every topology;
//!  2. `glLineWidth(5)` on `GL_LINES`, `GL_LINE_STRIP` and `GL_LINE_LOOP`
//!     keeps VERTEX_2 and the topology and writes ISPA.pointlinewidth 79
//!     (5 px in 4.4 fixed point, minus 1): no software widening, no other
//!     vertex;
//!  3. a line loop whose client arrays are too large for one piece is not
//!     cut (`cut_unit` (0, 0)), so its closing segment (n-1, 0), whose
//!     provoking vertex is vertex 0, stays in the draw; a line strip of the
//!     same arrays is cut, so the size really is past the cut;
//!  4. a `flat` ES 3.00 varying on each of these draws is iterated by the
//!     coefficient program as PVR_PDS_COEFF_FLAT (the DOUTI FLAT_VERTEX2 of
//!     driver/src/pvr_pds_render_gen.c, which tests/es3_pvr_flatshade.rs
//!     checks on the real generator), and a smooth one beside it is not.
//!
//! As in tests/es3_pvr_flatshade.rs, the words are read from the command
//! stream the draws leave through the REAL PowerVR back end on a platform
//! written here, with stand-ins for the C generators that record their
//! inputs. HOST-ONLY: which vertex the TA picks for a loop's closing segment
//! and for a widened line is the AM62P board's check (the note's section 5).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_flatshade_lines
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
    /// TA_STATE_PPP_CTRL.flatshade_vtx of the draw's PPP block.
    flat_ppp: u32,
    /// The TA object type and ISPA.pointlinewidth.
    objtype: u32,
    point_line_width: u32,
    /// TA_STATE_VARYING0: f32 components iterated linear, flat, non-perspective.
    varying0: (u32, u32, u32),
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
    /// VDM_STATE0.flatshade_control of each VDM state, the topology and
    /// index count of each index list.
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
        p.draws.push(DrawRec {
            kick,
            coeff,
            varyingsize: (st.pds_varyingsize, st.usc_varyingsize),
            flat_ppp: st.flatshade_vtx,
            objtype: st.objtype,
            point_line_width: st.point_line_width,
            varying0: (st.f32_linear, st.f32_flat, st.f32_npc),
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

/// The file's one-sample window context and glmark2's uniform-colour program
/// (no varying but the position), made in it.
#[derive(Clone, Copy)]
struct Ctx {
    ctx: *mut context::GlContext,
    program: GLuint,
}

static mut CTX: Ctx = Ctx { ctx: core::ptr::null_mut(), program: 0 };

/// The context and the backend, set up once; the tests take turns. Each
/// starts with the context current, every kick submitted, the line width 1
/// and the records empty.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            let one = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!one.is_null(), "{:?}", logs());
            context::gl_context_set_current(one);
            CTX = Ctx { ctx: one, program: program("passthrough.vert", "uniform_color.frag") };
        }
    });
    // SAFETY: set once above, read under the lock; plain GL calls.
    unsafe {
        context::gl_context_set_current(CTX.ctx);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
        glLineWidth(1.0);
    }
    with_platform(|p| {
        p.draws.clear();
        p.vdm_flat.clear();
        p.lists.clear();
    });
    g
}

/// The uniform-colour program of the rig.
fn plain() -> GLuint {
    // SAFETY: set once in `rig`, read under its lock.
    unsafe { CTX.program }
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader_src(type_: GLenum, name: &str, src: *const GLchar) -> GLuint {
    // SAFETY: plain GL calls; `src` is a NUL-terminated string.
    unsafe {
        let s = glCreateShader(type_);
        glShaderSource(s, 1, &src, core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{name}: compile failed ({:?})", logs());
        s
    }
}

fn shader(type_: GLenum, name: &str) -> GLuint {
    let cname = std::ffi::CString::new(name).unwrap();
    // SAFETY: a NUL-terminated name; the answer is a static row or null.
    let src = unsafe { yakogl::shaderc::offline::gles_offline_shader_source(cname.as_ptr()) };
    assert!(!src.is_null(), "{name}: not in the offline shader table");
    shader_src(type_, name, src)
}

fn link(v: GLuint, f: GLuint, what: &str) -> GLuint {
    // SAFETY: plain GL calls on valid handles; the names are NUL-terminated.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glBindAttribLocation(p, 0, c"a_position".as_ptr());
        glBindAttribLocation(p, 1, c"a_color".as_ptr());
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "{what}: link failed ({:?})", logs());
        p
    }
}

fn program(vs: &str, fs: &str) -> GLuint {
    link(shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs), &format!("{vs} + {fs}"))
}

/// Four vertices, (x, y, z, w) and a vec4 colour (glmark2's program reads
/// the first two as its uv) each: a square outline.
static POS: [f32; 16] = [-0.5, -0.5, 0.0, 1.0, 0.5, -0.5, 0.0, 1.0, 0.5, 0.5, 0.0, 1.0, -0.5, 0.5, 0.0, 1.0];
static COLOR: [f32; 16] = [1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0];
static IDX: [u16; 4] = [0, 1, 2, 3];

/// A colour clear, one glDrawElements of `idx` with program `prog`, then the
/// frame's end (eglSwapBuffers' submit).
fn draw(prog: GLuint, mode: GLenum, pos: &[f32], color: &[f32], idx: &[u16]) {
    // SAFETY: plain GL calls; the arrays outlive the draw (the core stages
    // client arrays at the draw).
    unsafe {
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glUseProgram(prog);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, pos.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glVertexAttribPointer(1, 4, GL_FLOAT, 0, 0, color.as_ptr() as *const c_void);
        glEnableVertexAttribArray(1);
        glDrawElements(mode, idx.len() as GLsizei, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
        assert_eq!(glGetError(), GL_NO_ERROR, "mode 0x{mode:x}: {:?}", logs());
        glFlush();
    }
}

/// The index lists recorded since the records were last cleared, without
/// the triangle lists: every draw of this file is a line draw, so a triangle
/// list is the back end's own frame-start block (the clear of a process's
/// first frame, or the colour load of a frame a large draw opens).
fn line_lists() -> Vec<(u32, u32)> {
    with_platform(|p| p.lists.iter().copied().filter(|&(t, _)| t != PVR_VDM_TOPOLOGY_TRI_LIST).collect())
}

/// What the last draw left: its PPP record (the last: the frame start leaves
/// blocks of its own), the last VDM state's flatshade_control and the line
/// index lists.
fn last_draw(mode: GLenum) -> (DrawRec, u32, Vec<(u32, u32)>) {
    let (rec, vdm) = with_platform(|p| {
        assert!(!p.draws.is_empty(), "mode 0x{mode:x}: no PPP block ({:?})", p.log);
        assert!(!p.vdm_flat.is_empty(), "mode 0x{mode:x}: no VDM state");
        (p.draws.last().unwrap().clone(), *p.vdm_flat.last().unwrap())
    });
    let lists = line_lists();
    assert!(!lists.is_empty(), "mode 0x{mode:x}: no line index list");
    (rec, vdm, lists)
}

/* ---- the tests --------------------------------------------------------------- */

/// TA_STATE_PPP_CTRL.flatshade_vtx VERTEX_2 and VDMCTRL_FLATSHADE_CONTROL
/// VERTEX_2 (ppp.xml FLATSHADE 3, vdm.xml FLATSHADE_CONTROL 2), as
/// tests/es3_pvr_flatshade.rs.
const PPP_VERTEX_2: u32 = 3;
const VDM_VERTEX_2: u32 = 2;

/// ISPA.pointlinewidth of a `px`-pixel line: 4.4 fixed point minus 1
/// (src/pvr/ffi.rs `PvrPppState::point_line_width`, Mesa's encoding).
fn line_width_word(px: f32) -> u32 {
    (px * 16.0) as u32 - 1
}

/// Check the last draw of `mode` at `width`: the native `topology` last, a
/// LINE object of that width and VERTEX_2 in both words.
fn check_last(mode: GLenum, width: f32, topology: u32) -> (DrawRec, Vec<(u32, u32)>) {
    let (rec, vdm, lists) = last_draw(mode);
    let what = format!("mode 0x{mode:x} width {width}");
    assert_eq!(lists.last().unwrap().0, topology, "{what}: the VDM topology");
    assert_eq!(rec.objtype, PVR_TA_OBJTYPE_LINE, "{what}: the TA object type");
    assert_eq!(rec.point_line_width, line_width_word(width), "{what}: ISPA.pointlinewidth");
    assert_eq!(rec.flat_ppp, PPP_VERTEX_2, "{what}: TA_STATE_PPP_CTRL.flatshade_vtx is the last vertex");
    assert_eq!(vdm, VDM_VERTEX_2, "{what}: VDM_STATE0.flatshade_control is the last vertex");
    (rec, lists)
}

/// Draw the square as `mode` at `width` with `prog` and check it.
fn check_line(prog: GLuint, mode: GLenum, width: f32, topology: u32) -> (DrawRec, Vec<(u32, u32)>) {
    with_platform(|p| {
        p.draws.clear();
        p.vdm_flat.clear();
        p.lists.clear();
    });
    // SAFETY: plain GL call.
    unsafe { glLineWidth(width) };
    draw(prog, mode, &POS, &COLOR, &IDX);
    check_last(mode, width, topology)
}

#[test]
fn line_loops_flatshade_from_the_last_vertex() {
    let _g = rig();
    let (rec, lists) = check_line(plain(), GL_LINE_LOOP, 1.0, PVR_VDM_TOPOLOGY_LINE_LOOP);
    assert_eq!(rec.point_line_width, 15, "a 1-pixel line");
    /* one list of the four vertices: the core neither closes the loop with
     * a fifth index nor splits it (the note's section 1) */
    assert_eq!(lists, vec![(PVR_VDM_TOPOLOGY_LINE_LOOP, 4)]);
}

#[test]
fn wide_lines_flatshade_from_the_last_vertex() {
    let _g = rig();
    let (_, lists) = check_line(plain(), GL_LINES, 5.0, PVR_VDM_TOPOLOGY_LINE_LIST);
    assert_eq!(lists, vec![(PVR_VDM_TOPOLOGY_LINE_LIST, 4)], "no triangles made of a wide line");
}

#[test]
fn wide_line_strips_flatshade_from_the_last_vertex() {
    let _g = rig();
    let (_, lists) = check_line(plain(), GL_LINE_STRIP, 5.0, PVR_VDM_TOPOLOGY_LINE_STRIP);
    assert_eq!(lists, vec![(PVR_VDM_TOPOLOGY_LINE_STRIP, 4)], "no triangles made of a wide line");
}

#[test]
fn wide_line_loops_flatshade_from_the_last_vertex() {
    let _g = rig();
    let (_, lists) = check_line(plain(), GL_LINE_LOOP, 5.0, PVR_VDM_TOPOLOGY_LINE_LOOP);
    /* widened by the TA: still one native loop of four vertices */
    assert_eq!(lists, vec![(PVR_VDM_TOPOLOGY_LINE_LOOP, 4)], "no triangles made of a wide line");
}

/// A ring of `n` points with a colour each: past half the 1 MiB GENERAL
/// arena as client arrays for `BIG` (30,000 x (16 + 16) bytes of attributes
/// and 2 of index, 1,020,000 bytes against 524,288), so a list or a strip
/// of it is drawn in pieces (`draw_indexed_pieces`).
fn big_ring(n: usize) -> (Vec<f32>, Vec<f32>, Vec<u16>) {
    let mut pos = Vec::with_capacity(4 * n);
    let mut color = Vec::with_capacity(4 * n);
    for i in 0..n {
        let a = i as f32 / n as f32 * core::f32::consts::TAU;
        let odd = (i % 2) as f32;
        pos.extend([0.5 * a.cos(), 0.5 * a.sin(), 0.0, 1.0]);
        color.extend([odd, 0.0, 1.0 - odd, 1.0]);
    }
    (pos, color, (0..n as u16).collect())
}

const BIG: usize = 30_000;

/// A strip of the same arrays is cut: the size is past the cut, so the
/// loop's single list below is the loop staying whole, not a small draw.
#[test]
fn a_big_line_strip_is_drawn_in_pieces() {
    let _g = rig();
    let (pos, color, idx) = big_ring(BIG);
    draw(plain(), GL_LINE_STRIP, &pos, &color, &idx);
    let lists = line_lists();
    assert!(lists.len() > 1, "a strip past half the arena is cut: {lists:?} ({:?})", logs());
    assert!(lists.iter().all(|&(t, _)| t == PVR_VDM_TOPOLOGY_LINE_STRIP), "{lists:?}");
    /* each piece repeats the vertex the next continues from */
    let drawn: u32 = lists.iter().map(|&(_, n)| n).sum();
    assert_eq!(drawn, BIG as u32 + lists.len() as u32 - 1, "{lists:?}");
}

/// `cut_unit(GL_LINE_LOOP)` is (0, 0): a loop past half the arena still goes
/// whole, one native LINE_LOOP list of every vertex, at width 1 and 5, so
/// the closing segment (n-1, 0) is the hardware's and takes vertex 0 as GL's
/// table 2.12 says.
#[test]
fn a_big_line_loop_is_not_cut() {
    let _g = rig();
    let (pos, color, idx) = big_ring(BIG);
    for width in [1.0, 5.0] {
        with_platform(|p| p.lists.clear());
        // SAFETY: plain GL call.
        unsafe { glLineWidth(width) };
        draw(plain(), GL_LINE_LOOP, &pos, &color, &idx);
        let (_, lists) = check_last(GL_LINE_LOOP, width, PVR_VDM_TOPOLOGY_LINE_LOOP);
        assert_eq!(lists, vec![(PVR_VDM_TOPOLOGY_LINE_LOOP, BIG as u32)], "width {width}: {:?}", logs());
    }
}

/* ---- a flat ES 3.00 varying ---------------------------------------------------- */

/// dEQP's flatshading program shape: a colour per vertex handed to the
/// fragment shader `flat`, here beside a smooth copy (the control).
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
const FLAT_VS: &CStr = c"#version 300 es
in highp vec4 a_position;
in mediump vec4 a_color;
flat out mediump vec4 v_flat;
out mediump vec4 v_smooth;
void main (void)
{
    gl_Position = a_position;
    v_flat = a_color;
    v_smooth = a_color;
}
";

#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
const FLAT_FS: &CStr = c"#version 300 es
flat in mediump vec4 v_flat;
in mediump vec4 v_smooth;
layout(location = 0) out mediump vec4 o_color;
void main (void)
{
    o_color = v_flat + 0.0 * v_smooth;
}
";

/// The online compiler as the GL core's shader back end until dropped (a
/// build with the table keeps the offline one by default); taken under
/// `rig`'s lock.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
struct Online(*const yakogl::shaderc::ShcBackend);

#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
impl Online {
    fn new() -> Online {
        // SAFETY: the back-end pointer is only touched under rig's lock.
        unsafe {
            let saved = yakogl::shaderc::g_shaderc;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::backend::SHC_ONLINE;
            Online(saved)
        }
    }
}

#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
impl Drop for Online {
    fn drop(&mut self) {
        // SAFETY: as in `new`.
        unsafe { yakogl::shaderc::g_shaderc = self.0 };
    }
}

/// The flat program, compiled by the online compiler through the GL core.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
fn flat_program() -> GLuint {
    let v = shader_src(GL_VERTEX_SHADER, "flat vs", FLAT_VS.as_ptr());
    let f = shader_src(GL_FRAGMENT_SHADER, "flat fs", FLAT_FS.as_ptr());
    link(v, f, "flat program")
}

/// The coefficient program's varyings as the stand-in wrote them:
/// (components, interpolation, output offset) after its four flag words,
/// before its two code words.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
fn coeff_varyings(rec: &DrawRec) -> Vec<(u32, u32, u32)> {
    assert!(rec.coeff.len() >= 6, "no coefficient program: {rec:?}");
    let (varyings, rest) = rec.coeff[4..rec.coeff.len() - 2].as_chunks::<3>();
    assert!(rest.is_empty(), "three words a varying: {rec:?}");
    varyings.iter().map(|&[n, interp, off]| (n, interp, off)).collect()
}

/// On a line loop and on each wide line form, the flat varying is iterated
/// FLAT (the DOUTI FLAT_VERTEX2 tests/es3_pvr_flatshade.rs checks on the
/// real generator) and the smooth one beside it is not.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
#[test]
fn a_flat_varying_is_iterated_flat_on_every_line_form() {
    let _g = rig();
    let _online = Online::new();
    let prog = flat_program();
    for (mode, width, topology) in [
        (GL_LINE_LOOP, 1.0, PVR_VDM_TOPOLOGY_LINE_LOOP),
        (GL_LINES, 5.0, PVR_VDM_TOPOLOGY_LINE_LIST),
        (GL_LINE_STRIP, 5.0, PVR_VDM_TOPOLOGY_LINE_STRIP),
        (GL_LINE_LOOP, 5.0, PVR_VDM_TOPOLOGY_LINE_LOOP),
    ] {
        let (rec, _) = check_line(prog, mode, width, topology);
        let what = format!("mode 0x{mode:x} width {width}");
        let vs = coeff_varyings(&rec);
        assert_eq!(vs.len(), 2, "{what}: two varyings iterated: {vs:?}");
        let flat: Vec<_> = vs.iter().filter(|v| v.1 == PVR_PDS_COEFF_FLAT).collect();
        assert_eq!(flat.len(), 1, "{what}: one varying iterated FLAT: {vs:?}");
        assert_eq!(flat[0].0, 4, "{what}: the flat vec4's four components: {vs:?}");
        assert!(
            vs.iter().any(|v| v.1 != PVR_PDS_COEFF_FLAT && v.0 == 4),
            "{what}: the smooth vec4 is not iterated flat: {vs:?}"
        );
    }
}

/// Mesa sorts the vertex outputs it hands the TA by interpolation:
/// TA_STATE_VARYING0.f32_flat counts the flat components and f32_linear the
/// smooth ones (pvr_arch_pipeline.c:1813-1828, pvr_arch_cmd_buffer.c:6618-6622).
/// yakogl counts every varying dword as f32_linear (f32_npc without the W
/// plane) and leaves f32_flat 0 (src/pvr/backend.rs:8271-8287, "ShcShader
/// carries no per-VS varying list"). Not the provoking vertex, which the
/// DOUTI word chooses (FLAT_VERTEX2 above), and the same on every topology.
/// Whether the TA's partition changes a flat value is not documented
/// anywhere we can read: the AM62P flatshading rows decide. Ignored until
/// then because it fails on main.
#[cfg(all(feature = "shaderc-online", not(feature = "report-es2")))]
#[test]
#[ignore = "PowerVR TA_STATE_VARYING0 counts flat components as f32_linear, not f32_flat as Mesa does (src/pvr/backend.rs, #325); the AM62P flatshading run decides whether it matters"]
fn the_ta_counts_flat_components_as_flat() {
    let _g = rig();
    let _online = Online::new();
    let prog = flat_program();
    let (rec, _) = check_line(prog, GL_LINE_LOOP, 1.0, PVR_VDM_TOPOLOGY_LINE_LOOP);
    let (linear, flat, npc) = rec.varying0;
    assert_eq!((linear + npc, flat), (4, 4), "TA_STATE_VARYING0 (linear, flat, npc) = {:?}", rec.varying0);
}
