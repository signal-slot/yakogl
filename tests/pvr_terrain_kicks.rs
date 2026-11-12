// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 `terrain`'s kicks a frame on the PowerVR backend, by kind
//! (docs/notes/am62p-slow-terrain.md, "Kicks (`GM2_KICKS=1`): 52 a frame, and
//! what each is"; tools/hostharness/am62p-terrain/kicks.py counts them from
//! the host harness's log). The AM62P's terrain frame is its kicks (about
//! 1.1 ms each, the CPU waits for the GPU between them), so every fix of the
//! note's ranked list moves one of the numbers pinned here, and updates it
//! in the same commit.
//!
//! The GL calls are terrain's frame in glmark2's order, 8 render passes, 5
//! `glGenerateMipmap` chains (on the 800x600 terrain texture twice and the
//! tilt-shift h one, and the two 256x256 bloom ones) and the window's clear
//! and swap, run against the REAL PowerVR backend (`yakogl_backend_pvr`) on
//! a platform written here (as tests/depth_resolve_pvr_backend.rs: GPU
//! memory is host memory, every submit completes at once, the PowerVR's
//! command generators are stand-ins). Each submitted `PvrRenderKick`'s size
//! tells its kind. The host's 52 equals the note's `kicks.py` figure; the
//! board's 52.7 also counts the setup's kicks.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_terrain_kicks
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
    /// (width, height) of every kick submitted, in order: index + 1 is its fence.
    sizes: Vec<(u32, u32)>,
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
        p.sizes.push((k.width, k.height));
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

/* The base-instance entry point: the stand-in ignores the base. */
#[no_mangle]
unsafe extern "C" fn pvr_pds_render_vertex_program_ids_base(
    vs_usc_exec_offset: u64,
    vs_temps: u32,
    streams: *const PvrPdsVertexStream,
    num_streams: u32,
    has_slc_mcu_cache_controls: i32,
    vertex_id_reg: u32,
    instance_id_reg: u32,
    _instance_id_base: u32,
    data_out: *mut u32,
    code_out: *mut u32,
    info: *mut PvrPdsRenderVertexInfo,
) -> i32 {
    // SAFETY: the caller's buffers, as for the stand-in above.
    unsafe {
        pvr_pds_render_vertex_program_ids(
            vs_usc_exec_offset,
            vs_temps,
            streams,
            num_streams,
            has_slc_mcu_cache_controls,
            vertex_id_reg,
            instance_id_reg,
            data_out,
            code_out,
            info,
        )
    }
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

/// glmark2's canvas, as the host harness runs terrain.
const CANVAS: (i32, i32) = (800, 600);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAMS: (GLuint, GLuint) = (0, 0);

fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
            PROGRAMS = (program("passthrough.vert", "uniform_color.frag"), program("uv.vert", "texture.frag"));
        }
    });
    with_platform(|p| p.log.clear());
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

/// One of terrain's render targets: an RGBA8 texture (glmark2's
/// `RenderObject`s: LINEAR_MIPMAP_LINEAR / LINEAR, CLAMP_TO_EDGE) in an FBO,
/// with a 16-bit depth renderbuffer when `depth`.
struct Target {
    tex: GLuint,
    fbo: GLuint,
    size: (i32, i32),
}

fn target(size: (i32, i32), depth: bool) -> Target {
    let (mut tex, mut fbo, mut rb) = (0, 0, 0);
    // SAFETY: room for one name each; a null image.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            size.0,
            size.1,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        if depth {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, size.0, size.1);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    Target { tex, fbo, size }
}

/// One pass of terrain's frame, as glmark2's `TextureRenderer` /
/// `RenderObject` run it: bind the FBO, viewport, clear (`clear`: a pass that
/// loads its target, the overlay, does not), draw sampling `src` when given,
/// then `glGenerateMipmap` on the target's texture when `mip`.
fn pass(dst: &Target, src: Option<&Target>, clear: bool, mip: bool) {
    let (_, tex_prog) = unsafe { PROGRAMS };
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, dst.fbo);
        glViewport(0, 0, dst.size.0, dst.size.1);
        if clear {
            glClearColor(0.0, 0.0, 0.0, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        }
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, src.map_or(0, |s| s.tex));
    }
    draw(if src.is_some() { tex_prog } else { unsafe { PROGRAMS.0 } });
    if mip {
        // SAFETY: plain GL calls.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, dst.tex);
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindTexture(GL_TEXTURE_2D, 0);
        }
    }
}

struct Terrain {
    height: Target,
    normal: Target,
    terrain: Target,
    bloom_h: Target,
    bloom_v: Target,
    tilt: Target,
}

fn terrain_targets() -> Terrain {
    Terrain {
        height: target((256, 256), false),
        normal: target((256, 256), false),
        terrain: target(CANVAS, true),
        bloom_h: target((256, 256), false),
        bloom_v: target((256, 256), false),
        tilt: target(CANVAS, false),
    }
}

/// Terrain's frame (glmark2 scene-terrain.cpp:332-339 and its renderers):
/// the window's clear (`MainLoop`), height map and normal map (256x256, no
/// depth), terrain (800x600 with depth) and its chain, bloom h and v (256x256)
/// and their chains, the overlay into the terrain texture and its chain
/// again, tilt-shift h (800x600) and its chain, then tilt-shift v into the
/// window and the swap.
fn terrain_frame(t: &Terrain) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    pass(&t.height, None, true, false);
    pass(&t.normal, None, true, false);
    pass(&t.terrain, Some(&t.height), true, true);
    pass(&t.bloom_h, Some(&t.terrain), true, true);
    pass(&t.bloom_v, Some(&t.bloom_h), true, true);
    pass(&t.terrain, Some(&t.bloom_v), false, true);
    pass(&t.tilt, Some(&t.terrain), true, true);
    // the window: tilt-shift v clears it again and draws, the swap submits
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, t.tilt.tex);
    }
    draw(unsafe { PROGRAMS.1 });
    // SAFETY: plain GL calls (eglSwapBuffers' submit).
    unsafe {
        glBindTexture(GL_TEXTURE_2D, 0);
        glFlush();
    }
}

/// What a kick is, from its place in the frame and its size: a mip level
/// kick declares the size of level 1 of its chain (400x300 of an 800x600
/// target, 128x128 of a 256x256 one; src/pvr/backend.rs `record_mip_level`),
/// which no pass of terrain has; the frame's first kick is the window's
/// clear (a 800x600 kick followed by the height map's 256x256 pass); every
/// other kick is a pass.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    WindowClear,
    Pass,
    Mip,
}

fn kinds(sizes: &[(u32, u32)]) -> Vec<Kind> {
    sizes
        .iter()
        .enumerate()
        .map(|(i, &s)| match s {
            (400, 300) | (128, 128) => Kind::Mip,
            (800, 600) if i == 0 && sizes.get(1) == Some(&(256, 256)) => Kind::WindowClear,
            _ => Kind::Pass,
        })
        .collect()
}

/// glmark2 terrain's steady frame on the PowerVR backend: 52 kicks, by kind
/// 43 mip levels, 8 passes and the window's clear
/// (docs/notes/am62p-slow-terrain.md, "Kicks (`GM2_KICKS=1`): 52 a frame, and
/// what each is": `kicks.py`'s `fb1+2 fb2+1 fb4+1 fb5+10 fb6+9 fb4+9 fb7+10
/// fb0+10`). The stand-in platform here completes every kick at once, so
/// nothing waits, but the backend submits the same kicks as on the board:
/// the host count equals the note's 52 (the note's
/// setup frame of 61 also makes the 512x512 specular map's chain, which this
/// test leaves out). A kick's size is read from the submitted
/// `PvrRenderKick`; the chains are 9 kicks for the 800x600 targets (levels
/// 1..=9 of 10) and 8 for the 256x256 ones (levels 1..=8 of 9), five a frame.
///
/// Each number is pinned so that the fix that moves it updates it on
/// purpose, with the note's fix number in the message.
#[test]
fn terrain_frame_is_52_kicks_43_mip_levels_8_passes_and_a_window_clear() {
    let _g = rig();
    let t = terrain_targets();
    terrain_frame(&t); /* the first frame creates the targets' storage and programs */
    let mut frames = Vec::new();
    for _ in 0..3 {
        let from = with_platform(|p| p.sizes.len());
        terrain_frame(&t);
        frames.push(with_platform(|p| p.sizes[from..].to_vec()));
    }
    for (i, sizes) in frames.iter().enumerate() {
        let k = kinds(sizes);
        let count = |w: Kind| k.iter().filter(|&&x| x == w).count();
        assert_eq!(
            count(Kind::Mip),
            43,
            "frame {i}: mip-level kicks. Fix 2 (three levels a render: 20), fix 3 (the CPU tail again, \
             Backend::mip_cpu_tail: 11) or fix 4 (one submit a chain) move this; sizes {sizes:?}"
        );
        assert_eq!(
            count(Kind::Pass),
            8,
            "frame {i}: pass kicks (height, normal, terrain, bloom h, bloom v, overlay, tilt-shift h and v). \
             No candidate of the note changes glmark2's passes; a changed count is a change in the GL core's \
             kick boundaries (target switches, settle_pending_clear); sizes {sizes:?}"
        );
        assert_eq!(
            count(Kind::WindowClear),
            1,
            "frame {i}: the window's dead clear kick. Fix 5 (keep a framebuffer's pending clear across a bind \
             to another one) takes it away: then 51 kicks a frame; sizes {sizes:?}"
        );
        assert_eq!(
            sizes.len(),
            52,
            "frame {i}: kicks a frame. Every candidate's expected total is in the note (fix 2: 29, fix 3: 20, \
             fix 5: 51); sizes {sizes:?}"
        );
        /* the chains: 9 + 8 + 8 + 9 + 9 level kicks, each run ending at the next pass */
        let mut runs = Vec::new();
        let mut n = 0;
        for &x in &k {
            if x == Kind::Mip {
                n += 1;
            } else if n > 0 {
                runs.push(n);
                n = 0;
            }
        }
        if n > 0 {
            runs.push(n);
        }
        assert_eq!(
            runs,
            [9, 8, 8, 9, 9],
            "frame {i}: level kicks of each chain (terrain, bloom h, bloom v, terrain again, tilt-shift h). \
             Fix 2 makes them 4 a chain (1; 2-4; 5-7; 8-9), fix 3 one to three"
        );
    }
    assert!(frames.iter().all(|f| *f == frames[0]), "every steady frame submits the same kicks in the same order");
}
