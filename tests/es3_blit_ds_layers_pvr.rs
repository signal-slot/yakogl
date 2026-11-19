// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer depth from a cube face or an array layer of a depth
//! texture, depth into one, and stencil into and out of a D24S8 texture,
//! through the GL API over the REAL PowerVR host back end (plan item
//! layers-test-pvr; docs/notes/blit-depth-layers.md; ES 3.0.6 4.3.3). The
//! PowerVR twin of tests/es3_blit_ds_layers.rs (the GL core's half on a
//! stand-in back end); the platform rig is copied from
//! tests/es3_blit_ds_window_pvr.rs (host memory for the GPU's, every submit
//! complete at once, stand-ins for the C command generators), and the
//! planes of the renderbuffers (and of a D24S8 texture) are seeded and read
//! through `pvr_backend_ds_write`, `pvr_backend_depth_resolve` and
//! `pvr_backend_stencil_resolve` (src/pvr/backend.rs).
//!
//! The PowerVR keeps ONE depth buffer per texture, not one per face or
//! layer (docs/notes/blit-depth-layers.md (a)): a buffer's key is `DsKey {
//! kind, id, gen }` (`zs_keys`, src/pvr/backend.rs) and the id the GL core
//! hands over (`context::ds_object_id`, `ds_identity` of src/gl/draw.rs) is
//! the texture's serial with no face, layer or level; `GlesDraw` has no
//! field for them; `pvr_backend_depth_resolve`, `pvr_backend_stencil_resolve`
//! and `pvr_backend_ds_write` look the buffer up by owner id alone
//! (`ds_plane_buf`). So the plane of a cube or array depth texture is one
//! image, whichever face or layer was drawn last, and a face or layer is
//! reached only through the GL core's texel storage (face/layer k at rows
//! k * height of the level, `texture.rs`), which is what these tests read
//! and write for the texture side (host memory, 4-byte R32F texels). The
//! stencil of a D24S8 texture is the plane under the texture's own id
//! (`ds_object_id(serial, true)`), written and read by the entries with any
//! id; the GL core copies it through those entries (item gl-ds-layers), and
//! `stencil_goes_through_a_texture_and_back` pins that.
//!
//! The GL core copies depth from and into a face or layer through those
//! texels (src/gl/blit.rs `depth_texels` with `texture::image_off`, item
//! layers-unignore-backends). Each depth shape is compared with what a
//! working blit leaves, from the nearest-sample model of
//! tests/es3_blit_ds_layers.rs (written apart from the code). Depth is a 24-bit
//! word `n / 16777215` on the PowerVR (also for DEPTH_COMPONENT32F, one D24S8
//! buffer); the model works in the integer `n`, exact in f32, and every
//! compare is on that integer.
//!
//! HOST-ONLY: the GPU's own depth writes are not seen; no board was used.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_blit_ds_layers_pvr
//! ```
//!
//! (the feature set of tests/es3_blit_ds_window_pvr.rs). Not built with
//! `report-es2` (glBlitFramebuffer is an ES 3.0 call).

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
const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;
/// A 24-bit depth word's denominator (`n / 16777215`).
const MAXD: f32 = 16_777_215.0;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}
fn bind(read: GLuint, draw: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, draw);
    }
}

/// The bound framebuffer's depth/stencil cleared, then a depth-tested draw
/// whose kick the fake GPU answers by storing `depth_word`: the buffer the
/// back end keeps under the target's id is live afterwards.
fn make_ds_exist(w: i32, h: i32) {
    // SAFETY: plain GL calls; `PROGRAM` is set once in `rig`.
    unsafe {
        glViewport(0, 0, w, h);
        glClearDepthf(0.25);
        glClearStencil(0x5a);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        draw_tri(PROGRAM);
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}
fn release(fbs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names these tests made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}
/* The planes, through the table's own entries, in MEMORY order, as integers: a
 * depth is the 24-bit word n (written as n / 16777215, read back rounded), a
 * stencil the byte. */

/// Write plane `plane` of object `id`, `w` x `h`, pixel (x, row) = `f(x, row)`.
fn write_plane(id: u32, plane: u32, w: u32, h: u32, f: &dyn Fn(u32, u32) -> f32) {
    let px = (0..h).flat_map(|y| (0..w).map(move |x| (x, y)));
    let r = if plane == backend::DS_PLANE_DEPTH {
        let v: Vec<f32> = px.map(|(x, y)| f(x, y) / MAXD).collect();
        // SAFETY: `h` rows of `w` floats.
        unsafe { pvr_backend_ds_write(id, plane, v.as_ptr() as *const c_void, 0, 0, w, h, w) }
    } else {
        let v: Vec<u8> = px.map(|(x, y)| f(x, y) as u8).collect();
        // SAFETY: `h` rows of `w` bytes.
        unsafe { pvr_backend_ds_write(id, plane, v.as_ptr() as *const c_void, 0, 0, w, h, w) }
    };
    assert_eq!(r, 0, "ds_write of plane {plane} of 0x{id:x}: {:?}", logs());
}

/// Read plane `plane` of object `id`, `w` x `h`, through the table.
fn read_plane(id: u32, plane: u32, w: u32, h: u32) -> Vec<f32> {
    if plane == backend::DS_PLANE_DEPTH {
        let mut v = vec![-1.0f32; (w * h) as usize];
        // SAFETY: `h` rows of `w` floats.
        let r = unsafe { pvr_backend_depth_resolve(id, v.as_mut_ptr(), w, h, w) };
        assert_eq!(r, 0, "depth_resolve of 0x{id:x}: {:?}", logs());
        v.into_iter().map(|d| (d * MAXD).round()).collect()
    } else {
        let mut v = vec![0xffu8; (w * h) as usize];
        // SAFETY: `h` rows of `w` bytes.
        let r = unsafe { pvr_backend_stencil_resolve(id, v.as_mut_ptr(), w, h, w) };
        assert_eq!(r, 0, "stencil_resolve of 0x{id:x}: {:?}", logs());
        v.into_iter().map(f32::from).collect()
    }
}

const D: u32 = backend::DS_PLANE_DEPTH;
const S: u32 = backend::DS_PLANE_STENCIL;

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

use yakogl::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D};

/* ---- objects -------------------------------------------------------------------- */

/// How a depth texture is specified: (internal format, format, type).
type Fmt = (GLenum, GLenum, GLenum);
const D24: Fmt = (GL_DEPTH_COMPONENT24, GL_DEPTH_COMPONENT, GL_UNSIGNED_INT);
const D32F: Fmt = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);
const D24S8: Fmt = (GL_DEPTH24_STENCIL8, GL_DEPTH_STENCIL, GL_UNSIGNED_INT_24_8);

const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];
const FACE_PX: u32 = 0;
const FACE_NZ: u32 = 5;

#[derive(Clone, Copy, Debug)]
enum Layered {
    Cube,
    Array,
}

fn layers_of(kind: Layered) -> u32 {
    match kind {
        Layered::Cube => 6,
        Layered::Array => 3,
    }
}

/// The R32F texel (x, y) of layer/face `layer` of level 0 in the GL core's
/// storage (layer k's row y at row k * height + y).
fn texel_ptr(tex: GLuint, layer: u32, x: u32, y: u32) -> *mut f32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert!(x < t.width && y < t.height && layer < t.layers.max(1) as u32);
    let row = layer as usize * t.height as usize + y as usize;
    let off = t.level_off[0] as usize + (row * t.level_pitch_at(0) as usize + x as usize) * 4;
    assert!(
        off + 4 <= t.mem.size,
        "off {off} size {} pitch {} w {} h {} layers {} x{x} y{y} l{layer}",
        t.mem.size,
        t.level_pitch_at(0),
        t.width,
        t.height,
        t.layers
    );
    (t.mem.cpu as *mut u8).wrapping_add(off) as *mut f32
}

/// The 24-bit depth word n of a texel.
fn texel_n(tex: GLuint, layer: u32, x: u32, y: u32) -> f32 {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    (unsafe { texel_ptr(tex, layer, x, y).read() } * MAXD).round()
}

fn set_texel_n(tex: GLuint, layer: u32, x: u32, y: u32, n: f32) {
    // SAFETY: as in `texel_n`.
    unsafe { texel_ptr(tex, layer, x, y).write(n / MAXD) }
}

/// A `side` x `side` cube map or 3-layer 2D array of depth `fmt`, texel
/// (x, y) of layer k holding the word `f(k, x, y)`.
fn layered_texture(kind: Layered, fmt: Fmt, side: u32, f: impl Fn(u32, u32, u32) -> f32) -> GLuint {
    let mut tex = 0;
    let s = side as i32;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                for face in FACES {
                    glTexImage2D(face, 0, fmt.0 as GLint, s, s, 0, fmt.1, fmt.2, core::ptr::null());
                }
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                let n = layers_of(kind) as i32;
                glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, fmt.0 as GLint, s, s, n, 0, fmt.1, fmt.2, core::ptr::null());
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "a {kind:?} depth texture of {:#x}", fmt.0);
    for k in 0..layers_of(kind) {
        for y in 0..side {
            for x in 0..side {
                set_texel_n(tex, k, x, y, f(k, x, y));
            }
        }
    }
    tex
}

/// A `side` x `side` 2D texture of depth `fmt`.
fn flat_texture(fmt: Fmt, side: u32, f: impl Fn(u32, u32) -> f32) -> GLuint {
    let mut tex = 0;
    // zero pixels: the GL core allocates the storage of a 2D texture for them
    let zero = vec![0u32; (side * side) as usize];
    // SAFETY: room for one name; `side` x `side` words of pixels.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            fmt.0 as GLint,
            side as i32,
            side as i32,
            0,
            fmt.1,
            fmt.2,
            zero.as_ptr() as *const c_void,
        );
    }
    assert_eq!(err(), GL_NO_ERROR, "a {:#x} texture", fmt.0);
    for y in 0..side {
        for x in 0..side {
            set_texel_n(tex, 0, x, y, f(x, y));
        }
    }
    tex
}

fn layer_picture(tex: GLuint, layer: u32, side: u32) -> Vec<f32> {
    (0..side).flat_map(|y| (0..side).map(move |x| (x, y))).map(|(x, y)| texel_n(tex, layer, x, y)).collect()
}

/// A framebuffer with face `k` of cube map `tex` at the depth attachment.
fn face_fbo(tex: GLuint, k: u32) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, FACES[k as usize], tex, 0);
        assert_eq!(err(), GL_NO_ERROR, "a cube face at the depth attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

/// A framebuffer with layer `k` of array `tex` at the depth attachment.
fn layer_fbo(tex: GLuint, k: u32) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, tex, 0, k as i32);
        assert_eq!(err(), GL_NO_ERROR, "an array layer at the depth attachment");
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    fb
}

fn layer_attachment_fbo(kind: Layered, tex: GLuint, k: u32) -> GLuint {
    match kind {
        Layered::Cube => face_fbo(tex, k),
        Layered::Array => layer_fbo(tex, k),
    }
}

/// A framebuffer with the 2D texture `tex` at GL_DEPTH_STENCIL_ATTACHMENT.
fn texture_ds_fbo(tex: GLuint) -> GLuint {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    fb
}

/// A `w` x `h` renderbuffer of `format` (the format of the texture on the
/// other side of the blit) in a new framebuffer, its buffer made to exist:
/// (framebuffer, renderbuffer).
fn rb_fbo(format: GLenum, w: u32, h: u32) -> (GLuint, GLuint) {
    let point = if format == GL_DEPTH24_STENCIL8 { GL_DEPTH_STENCIL_ATTACHMENT } else { GL_DEPTH_ATTACHMENT };
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w as i32, h as i32);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    make_ds_exist(w as i32, h as i32);
    (fb, rb)
}

/// The id the back end keeps texture `tex`'s depth and stencil under.
fn texture_ds_id(tex: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    context::ds_object_id(c.texture_object(slot).expect("a texture").serial, true)
}

fn release_all(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    release(fbs, rbs);
    // SAFETY: names these tests made.
    unsafe { glDeleteTextures(texs.len() as i32, texs.as_ptr()) };
}

/* ---- the model (as tests/es3_blit_ds_layers.rs) ------------------------------------ */

/// The source texel destination pixel `o` takes on one axis, or None when its
/// sample point is outside the source cut to `[0, n)`.
fn model_axis(o: i64, s0: i64, s1: i64, d0: i64, d1: i64, n: i64) -> Option<i64> {
    let p = s0 as f64 + (o as f64 + 0.5 - d0 as f64) * (s1 - s0) as f64 / (d1 - d0) as f64;
    let (lo, hi) = (s0.min(s1).max(0), s0.max(s1).min(n));
    let t = p.floor() as i64;
    (p >= lo as f64 && p < hi as f64).then_some(t)
}

/// The destination after the blit `s` -> `d` of a `sw` x `sh` source into a
/// `dw` x `dh` destination that held `keep`.
fn model(
    src: &dyn Fn(u32, u32) -> f32,
    (sw, sh): (u32, u32),
    keep: &dyn Fn(u32, u32) -> f32,
    (dw, dh): (u32, u32),
    s: [i32; 4],
    d: [i32; 4],
) -> Vec<f32> {
    let mut out = Vec::new();
    for y in 0..dh {
        for x in 0..dw {
            let (xi, yi) = (x as i64, y as i64);
            let in_dst = xi >= d[0].min(d[2]) as i64
                && xi < d[0].max(d[2]) as i64
                && yi >= d[1].min(d[3]) as i64
                && yi < d[1].max(d[3]) as i64;
            let from = in_dst
                .then(|| {
                    let tx = model_axis(xi, s[0] as i64, s[2] as i64, d[0] as i64, d[2] as i64, sw as i64)?;
                    let ty = model_axis(yi, s[1] as i64, s[3] as i64, d[1] as i64, d[3] as i64, sh as i64)?;
                    Some((tx as u32, ty as u32))
                })
                .flatten();
            out.push(match from {
                Some((tx, ty)) => src(tx, ty),
                None => keep(x, y),
            });
        }
    }
    out
}

fn filled(f: &dyn Fn(u32, u32) -> f32, w: u32, h: u32) -> Vec<f32> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// Distinct per layer and texel, a word below 2^19 (exact in f32 and in the
/// unorm round trip).
fn layer_depth(k: u32, x: u32, y: u32) -> f32 {
    (100_000 + k * 60_000 + (y * 8 + x) * 811) as f32
}
/// The depth of a renderbuffer on the other side, up to 16 x 16.
fn rb_depth(x: u32, y: u32) -> f32 {
    (1_500_000 + (y * 16 + x) * 313) as f32
}
fn src_depth(x: u32, y: u32) -> f32 {
    (700_000 + (y * 16 + x) * 811) as f32
}

fn blit_ok(s: [i32; 4], d: [i32; 4], mask: GLbitfield, what: &str) {
    // SAFETY: plain GL call.
    unsafe { glBlitFramebuffer(s[0], s[1], s[2], s[3], d[0], d[1], d[2], d[3], mask, GL_NEAREST) };
    assert_eq!(err(), GL_NO_ERROR, "{what}: {:?}", logs());
    // SAFETY: plain GL call.
    unsafe { glFinish() };
}

/* ---- (1)/(2) depth from a cube face or an array layer ------------------------------- */

const SIDE: u32 = 8;
const WHOLE: [i32; 4] = [0, 0, SIDE as i32, SIDE as i32];

/// Layer/face `k` of a `kind` depth texture of `fmt`, blitted `s` -> `d` into
/// a `dz` x `dz` renderbuffer of the same format, whose plane is compared
/// with the model. The texture is read only.
fn from_layer(kind: Layered, fmt: Fmt, k: u32, dz: u32, s: [i32; 4], d: [i32; 4]) {
    let (dfb, rb) = rb_fbo(fmt.0, dz, dz);
    let rid = ds_id(rb);
    write_plane(rid, D, dz, dz, &rb_depth);
    let tex = layered_texture(kind, fmt, SIDE, layer_depth);
    let sfb = layer_attachment_fbo(kind, tex, k);
    bind(sfb, dfb);
    let what = format!("{kind:?} {:#x} layer {k}: {s:?} -> {d:?}", fmt.0);
    blit_ok(s, d, DEPTH, &what);
    let src = |x, y| layer_depth(k, x, y);
    let want = model(&src, (SIDE, SIDE), &rb_depth, (dz, dz), s, d);
    assert_eq!(read_plane(rid, D, dz, dz), want, "{what}");
    for l in 0..layers_of(kind) {
        let want = filled(&|x, y| layer_depth(l, x, y), SIDE, SIDE);
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} of the source is untouched");
    }
    release_all(&[sfb, dfb], &[tex], &[rb]);
}

/// The face's depth lands in the renderbuffer: 1:1, scaled up, scaled down
/// and a flipped sub-rectangle (expected: the nearest-sample model of the
/// face's own ramp, `layer_depth(face, x, y)`).
#[test]
fn depth_from_a_cube_face_copies_that_face() {
    let _g = rig();
    for fmt in [D24, D32F] {
        for face in [FACE_PX, FACE_NZ] {
            from_layer(Layered::Cube, fmt, face, 8, WHOLE, WHOLE);
            from_layer(Layered::Cube, fmt, face, 16, WHOLE, [0, 0, 16, 16]);
            from_layer(Layered::Cube, fmt, face, 4, WHOLE, [0, 0, 4, 4]);
            from_layer(Layered::Cube, fmt, face, 8, [1, 2, 6, 7], [2, 1, 7, 6]);
        }
    }
}

/// As the cube twin, for layers 0 and 2 (expected: `layer_depth(layer, x, y)`
/// through the model).
#[test]
fn depth_from_an_array_layer_copies_that_layer() {
    let _g = rig();
    for fmt in [D24, D32F] {
        for layer in [0, 2] {
            from_layer(Layered::Array, fmt, layer, 8, WHOLE, WHOLE);
            from_layer(Layered::Array, fmt, layer, 16, WHOLE, [0, 0, 16, 16]);
            from_layer(Layered::Array, fmt, layer, 4, WHOLE, [0, 0, 4, 4]);
            from_layer(Layered::Array, fmt, layer, 8, [1, 2, 6, 7], [2, 1, 7, 6]);
        }
    }
}

/* ---- (3) depth into a cube face or an array layer ----------------------------------- */

/// A `sz` x `sz` renderbuffer of `fmt` (depth `src_depth`) blitted `s` -> `d`
/// into layer/face `k` of a `kind` texture. The layer takes the model's
/// picture; every other layer stays as it was, and so does the source's
/// plane.
fn into_layer(kind: Layered, fmt: Fmt, k: u32, sz: u32, s: [i32; 4], d: [i32; 4]) {
    let (sfb, rb) = rb_fbo(fmt.0, sz, sz);
    let rid = ds_id(rb);
    write_plane(rid, D, sz, sz, &src_depth);
    let tex = layered_texture(kind, fmt, SIDE, layer_depth);
    let dfb = layer_attachment_fbo(kind, tex, k);
    bind(sfb, dfb);
    let what = format!("{kind:?} {:#x} into layer {k}: {s:?} -> {d:?}", fmt.0);
    let keep = |x, y| layer_depth(k, x, y);
    blit_ok(s, d, DEPTH, &what);
    let want_k = model(&src_depth, (sz, sz), &keep, (SIDE, SIDE), s, d);
    assert_eq!(layer_picture(tex, k, SIDE), want_k, "{what}: the layer");
    for l in (0..layers_of(kind)).filter(|&l| l != k) {
        let want = filled(&|x, y| layer_depth(l, x, y), SIDE, SIDE);
        assert_eq!(layer_picture(tex, l, SIDE), want, "{what}: layer {l} must not change");
    }
    assert_eq!(read_plane(rid, D, sz, sz), filled(&src_depth, sz, sz), "{what}: the source is read only");
    release_all(&[sfb, dfb], &[tex], &[rb]);
}

/// Face +X takes the whole 8 x 8 renderbuffer, face -Z the 4 x 4 one scaled
/// up, face +Y a 2..6 sub-rectangle; the other five faces keep their ramps.
#[test]
fn depth_into_a_cube_face_changes_only_that_face() {
    let _g = rig();
    for fmt in [D24, D32F] {
        into_layer(Layered::Cube, fmt, FACE_PX, 8, WHOLE, WHOLE);
        into_layer(Layered::Cube, fmt, FACE_NZ, 4, [0, 0, 4, 4], WHOLE);
        into_layer(Layered::Cube, fmt, 2, 8, [0, 0, 8, 8], [2, 2, 6, 6]);
    }
}

/// As the cube twin, for layers 0, 2 and 1 of a 3-layer array.
#[test]
fn depth_into_an_array_layer_changes_only_that_layer() {
    let _g = rig();
    for fmt in [D24, D32F] {
        into_layer(Layered::Array, fmt, 0, 8, WHOLE, WHOLE);
        into_layer(Layered::Array, fmt, 2, 4, [0, 0, 4, 4], WHOLE);
        into_layer(Layered::Array, fmt, 1, 8, [0, 0, 8, 8], [2, 2, 6, 6]);
    }
}

/* ---- (4) stencil to and from a D24S8 texture ---------------------------------------- */

fn stencil_of(x: u32, y: u32) -> f32 {
    (10 + y * 4 + x) as f32
}
fn tex_depth(x: u32, y: u32) -> f32 {
    (1_200_000 + (y * 4 + x) * 7) as f32
}
fn tex_stencil(x: u32, y: u32) -> f32 {
    (150 + y * 4 + x) as f32
}

/// A D24S8 renderbuffer (stencil `stencil_of`), a D24S8 2D texture whose
/// buffer exists (its stencil plane `tex_stencil`, its texels `tex_depth`)
/// and a second renderbuffer holding stencil 200.
struct StencilRig {
    src_fb: GLuint,
    src_rb: GLuint,
    tex: GLuint,
    tex_fb: GLuint,
    tex_id: u32,
    out_fb: GLuint,
    out_rb: GLuint,
    out_id: u32,
}

fn stencil_rig() -> StencilRig {
    let (src_fb, src_rb) = rb_fbo(GL_DEPTH24_STENCIL8, 4, 4);
    let (out_fb, out_rb) = rb_fbo(GL_DEPTH24_STENCIL8, 4, 4);
    let (sid, out_id) = (ds_id(src_rb), ds_id(out_rb));
    write_plane(sid, S, 4, 4, &stencil_of);
    write_plane(sid, D, 4, 4, &src_depth);
    write_plane(out_id, S, 4, 4, &|_, _| 200.0);
    write_plane(out_id, D, 4, 4, &rb_depth);
    let tex = flat_texture(D24S8, 4, tex_depth);
    let tex_fb = texture_ds_fbo(tex);
    make_ds_exist(4, 4);
    let tex_id = texture_ds_id(tex);
    write_plane(tex_id, S, 4, 4, &tex_stencil);
    /* the texels stand as the test wrote them (the draw above made the plane) */
    for y in 0..4 {
        for x in 0..4 {
            set_texel_n(tex, 0, x, y, tex_depth(x, y));
        }
    }
    StencilRig { src_fb, src_rb, tex, tex_fb, tex_id, out_fb, out_rb, out_id }
}

impl StencilRig {
    fn release(self) {
        release_all(&[self.src_fb, self.tex_fb, self.out_fb], &[self.tex], &[self.src_rb, self.out_rb]);
    }
    fn texels(&self) -> Vec<f32> {
        layer_picture(self.tex, 0, 4)
    }
}

const R: [i32; 4] = [0, 0, 4, 4];

/// Renderbuffer -> texture -> second renderbuffer, flipped in x on the second
/// hop. Expected: the texture's stencil plane (read by `stencil_resolve` under
/// the texture's id) holds `stencil_of(x, y)` after the first hop; the second
/// renderbuffer's holds `stencil_of(3 - x, y)` after the second; every depth
/// (the texture's texels, both renderbuffers' planes) is untouched, as the
/// mask names only stencil. (Its refused-today pair went with the lift of
/// gl-ds-layers; the shapes still refused are in tests/es3_blit_ds_layers.rs.)
#[test]
fn stencil_goes_through_a_texture_and_back() {
    let _g = rig();
    let r = stencil_rig();
    let texels = r.texels();
    bind(r.src_fb, r.tex_fb);
    blit_ok(R, R, STENCIL, "stencil into a texture");
    assert_eq!(read_plane(r.tex_id, S, 4, 4), filled(&stencil_of, 4, 4), "the texture's stencil");
    assert_eq!(r.texels(), texels, "the texture's depth is not what the stencil blit wrote");
    bind(r.tex_fb, r.out_fb);
    blit_ok(R, [4, 0, 0, 4], STENCIL, "stencil from a texture");
    assert_eq!(read_plane(r.out_id, S, 4, 4), filled(&|x, y| stencil_of(3 - x, y), 4, 4), "the second stencil");
    assert_eq!(read_plane(r.out_id, D, 4, 4), filled(&rb_depth, 4, 4), "the second depth is untouched");
    assert_eq!(read_plane(r.tex_id, S, 4, 4), filled(&stencil_of, 4, 4), "the texture's stencil is read only");
    r.release();
}
