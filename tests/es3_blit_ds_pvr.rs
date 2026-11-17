// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer between depth/stencil renderbuffers through the GL API
//! over the PowerVR host back end: the real GL core and the REAL PowerVR
//! back end (`yakogl_backend_pvr`, whose table has `ds_write` and
//! `stencil_resolve`: `pvr_backend_ds_write`, `pvr_backend_stencil_resolve`,
//! src/pvr/backend.rs), on a platform written here (tests/
//! depth_resolve_pvr_backend.rs's rig: host memory for the GPU's, every
//! submit complete at once, stand-ins for the C command generators), the 16
//! `fbo.blit.depth_stencil` shapes of dEQP-GLES3, the PowerVR counterpart of
//! tests/es3_blit_ds_v3d.rs. Until now the two entries were driven only by
//! src/pvr/backend_tests.rs's unit tests, never through the GL API.
//!
//! Per shape, in a module of its own: `depth_component16_basic` ..
//! `stencil_index8_scale` (depth_component16/24/32f basic and scale;
//! depth24_stencil8 and depth32f_stencil8 basic, scale, depth_only,
//! stencil_only; stencil_index8 basic and scale), `reads_back_the_source`
//! holds what the blit leaves: no GL error, each destination pixel takes the
//! nearest source pixel in the planes the mask names, the others keep their
//! clear. The planes are read through the table's `depth_resolve` and
//! `stencil_resolve` (`pvr_backend_depth_resolve`: 24-bit, `n / 16777215`).
//!
//! The sources are filled by the fake GPU after a clear: a depth-tested
//! draw, whose kick stores `depth_word` (a depth per texel in bits 23..0,
//! stencil byte 0x3c) into the packed D24S8 buffer at its ZLS-tile-aligned
//! width; the destinations are clear-only. A depth32f format is a 24-bit
//! round trip on the PowerVR too (one D24S8 buffer): this test pins that the
//! round trip is exact in 24 bits, not that dEQP's tolerance accepts it.
//!
//! HOST-ONLY: the GPU's own depth writes are not seen. What the ISP stores,
//! and the 24-bit words' layout the board writes, is the stand-in's reading
//! of the code (confirmed on the AM62P for the layout, gl_test 169, per
//! `pvr_backend_depth_resolve`); no board was used.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_blit_ds_pvr
//! ```
//!
//! Not built with `report-es2` (the 16 shapes are ES 3.0 calls the ES 2.0
//! report refuses).

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

/* ---- the tests ----------------------------------------------------------------- */

/// The source and destination renderbuffers' initial clears: what a
/// destination holds before a blit and what a source holds when no draw
/// ramped it.
const SRC_CLEAR: (f32, i32) = (0.25, 0x5a);
const DST_CLEAR: (f32, i32) = (0.75, 0x11);

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// One `fbo.blit.depth_stencil.*` case: the renderbuffer format on both
/// sides, where it attaches, the blit mask, and whether the destination is
/// another size than the source (the `_scale` cases).
#[derive(Clone, Copy)]
struct Shape {
    format: GLenum,
    point: GLenum,
    mask: GLbitfield,
    scale: bool,
}

impl Shape {
    fn has_depth(&self) -> bool {
        self.format != GL_STENCIL_INDEX8
    }
    fn has_stencil(&self) -> bool {
        matches!(self.format, GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 | GL_STENCIL_INDEX8)
    }
    /// The size of the source and of the destination (dEQP scales the
    /// rectangle by a non-integer factor in each axis, here 32x32 -> 20x40).
    fn sizes(&self) -> ((i32, i32), (i32, i32)) {
        ((32, 32), if self.scale { (20, 40) } else { (32, 32) })
    }
}

const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;

/// A `w` x `h` renderbuffer of `format` at `point` of a new framebuffer,
/// checked complete and bound: (framebuffer, renderbuffer).
fn ds_fbo(format: GLenum, point: GLenum, w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage 0x{format:x}: {:?}", logs());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "0x{format:x}: {:?}", logs());
        glViewport(0, 0, w, h);
    }
    (fb, rb)
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

/// The bound framebuffer's depth/stencil cleared to `(depth, stencil)` (the
/// planes its format has) and, with `ramp`, a depth-tested draw after it:
/// the fake GPU stores `depth_word` (a depth that differs per texel, stencil
/// byte 0x3c) into the buffer the job stores.
fn fill(s: &Shape, clear: (f32, i32), ramp: bool) {
    let bits =
        if s.has_depth() { GL_DEPTH_BUFFER_BIT } else { 0 } | if s.has_stencil() { GL_STENCIL_BUFFER_BIT } else { 0 };
    // SAFETY: plain GL calls.
    unsafe {
        glClearDepthf(clear.0);
        glClearStencil(clear.1);
        glClear(bits);
        if ramp && s.has_depth() {
            glEnable(GL_DEPTH_TEST);
        }
    }
    if ramp && s.has_depth() {
        // SAFETY: set once in `rig`, read under the lock.
        draw_tri(unsafe { PROGRAM });
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_DEPTH_TEST) };
    }
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

struct Pair {
    src_fb: GLuint,
    src_rb: GLuint,
    dst_fb: GLuint,
    dst_rb: GLuint,
}

/// The source (ramped by a draw where it has depth, else cleared) and the
/// destination (cleared to `DST_CLEAR`), both bound as read and draw
/// framebuffers of the blit.
fn pair(s: &Shape) -> Pair {
    let ((sw, sh), (dw, dh)) = s.sizes();
    let (src_fb, src_rb) = ds_fbo(s.format, s.point, sw, sh);
    fill(s, SRC_CLEAR, true);
    let (dst_fb, dst_rb) = ds_fbo(s.format, s.point, dw, dh);
    fill(s, DST_CLEAR, false);
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src_fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst_fb);
    }
    Pair { src_fb, src_rb, dst_fb, dst_rb }
}

fn blit(s: &Shape) -> GLenum {
    let ((sw, sh), (dw, dh)) = s.sizes();
    // SAFETY: plain GL calls.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, sw, sh, 0, 0, dw, dh, s.mask, GL_NEAREST);
    }
    err()
}

fn release(p: &Pair) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(2, [p.src_fb, p.dst_fb].as_ptr());
        glDeleteRenderbuffers(2, [p.src_rb, p.dst_rb].as_ptr());
    }
}

/// The depth plane of renderbuffer `rb` (size `w` x `h`) as the PowerVR
/// back end resolves it (24-bit: `n / 16777215`).
fn read_depth(rb: GLuint, w: i32, h: i32) -> Vec<f32> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { pvr_backend_depth_resolve(ds_id(rb), v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the depth of the renderbuffer: {:?}", logs());
    v
}

/// The stencil plane through the back end's `stencil_resolve` entry.
fn read_stencil(rb: GLuint, w: i32, h: i32) -> Vec<u8> {
    let f = yakogl_backend_pvr.stencil_resolve.expect("the PowerVR table has a stencil_resolve entry");
    let mut v = vec![0xffu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { f(ds_id(rb), v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the stencil of the renderbuffer: {:?}", logs());
    v
}

/// The source pixel nearest-filtering gives destination pixel `o` of `d`
/// over `s` (ES 3.0.6 4.3.3, the whole source onto the whole destination).
fn src_of(o: i32, s: i32, d: i32) -> i32 {
    (((o as f64 + 0.5) * s as f64 / d as f64).floor() as i32).min(s - 1)
}

/// The expected destination plane: the source plane scaled by the model.
fn scaled<T: Copy>(src: &[T], s: &Shape) -> Vec<T> {
    let ((sw, sh), (dw, dh)) = s.sizes();
    (0..dh)
        .flat_map(|y| (0..dw).map(move |x| (x, y)))
        .map(|(x, y)| src[(src_of(y, sh, dh) * sw + src_of(x, sw, dw)) as usize])
        .collect()
}

/// What a blit that works leaves in the destination: every destination
/// pixel takes the nearest source pixel, in each plane the mask names; a
/// plane it does not name keeps its clear.
fn check_readback(s: Shape) {
    let _g = rig();
    let p = pair(&s);
    let ((sw, sh), (dw, dh)) = s.sizes();
    let src_depth = s.has_depth().then(|| read_depth(p.src_rb, sw, sh));
    let src_stencil = s.has_stencil().then(|| read_stencil(p.src_rb, sw, sh));
    assert_eq!(blit(&s), GL_NO_ERROR, "{:?}", logs());
    if s.has_depth() {
        let want = if s.mask & DEPTH != 0 {
            scaled(src_depth.as_ref().unwrap(), &s)
        } else {
            vec![DST_CLEAR.0; (dw * dh) as usize]
        };
        let got = read_depth(p.dst_rb, dw, dh);
        /* the buffer holds 24 bits: a D32F source reads back as n / 16777215
         * and is written back by round(v * 0xffffff), so a 24-bit round
         * trip is exact. Whether dEQP's tolerance for the depth32f cases
         * accepts a 24-bit buffer is NOT measured. */
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!((g - w).abs() <= 0.5 / 16_777_215.0, "depth pixel {i}: {g} want {w}");
        }
    }
    if s.has_stencil() {
        let want = if s.mask & STENCIL != 0 {
            scaled(src_stencil.as_ref().unwrap(), &s)
        } else {
            vec![DST_CLEAR.1 as u8; (dw * dh) as usize]
        };
        assert_eq!(read_stencil(p.dst_rb, dw, dh), want, "the stencil plane");
    }
    release(&p);
}

macro_rules! shapes {
    ($($name:ident: $format:expr, $point:expr, $mask:expr, $scale:expr;)*) => {$(
        mod $name {
            use super::*;
            const S: Shape = Shape { format: $format, point: $point, mask: $mask, scale: $scale };
            #[test]
            fn reads_back_the_source() {
                check_readback(S);
            }
        }
    )*};
}

const D: GLenum = GL_DEPTH_ATTACHMENT;
const DS: GLenum = GL_DEPTH_STENCIL_ATTACHMENT;
const DS_MASK: GLbitfield = DEPTH | STENCIL;

shapes! {
    depth_component16_basic: GL_DEPTH_COMPONENT16, D, DEPTH, false;
    depth_component16_scale: GL_DEPTH_COMPONENT16, D, DEPTH, true;
    depth_component24_basic: GL_DEPTH_COMPONENT24, D, DEPTH, false;
    depth_component24_scale: GL_DEPTH_COMPONENT24, D, DEPTH, true;
    depth_component32f_basic: GL_DEPTH_COMPONENT32F, D, DEPTH, false;
    depth_component32f_scale: GL_DEPTH_COMPONENT32F, D, DEPTH, true;
    depth24_stencil8_basic: GL_DEPTH24_STENCIL8, DS, DS_MASK, false;
    depth24_stencil8_scale: GL_DEPTH24_STENCIL8, DS, DS_MASK, true;
    depth24_stencil8_depth_only: GL_DEPTH24_STENCIL8, DS, DEPTH, false;
    depth24_stencil8_stencil_only: GL_DEPTH24_STENCIL8, DS, STENCIL, false;
    depth32f_stencil8_basic: GL_DEPTH32F_STENCIL8, DS, DS_MASK, false;
    depth32f_stencil8_scale: GL_DEPTH32F_STENCIL8, DS, DS_MASK, true;
    depth32f_stencil8_depth_only: GL_DEPTH32F_STENCIL8, DS, DEPTH, false;
    depth32f_stencil8_stencil_only: GL_DEPTH32F_STENCIL8, DS, STENCIL, false;
    stencil_index8_basic: GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, STENCIL, false;
    stencil_index8_scale: GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, STENCIL, true;
}
