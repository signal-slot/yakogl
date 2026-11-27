// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64): a colour target whose GL format has fewer channels
//! than its store keeps GL's values in the lanes it lacks (issue #313, the
//! dEQP-GLES3 functional.fbo family F5 on the AM62P).
//!
//! GL_R8 and GL_RG8 are kept as RGBA8888 texels and GL_RG16F (and GL_RGB16F)
//! as four halves; the GL core hands the back end the store's surface class
//! (`GLES_SURF_RGBA8888`, `GLES_SURF_RGBA16F`). The PBE stored the
//! fragment's green and blue into those lanes, and a clear its green and
//! blue. glReadPixels forces the absent channels (`read_pixels`), but a
//! texture is sampled from its store: dEQP fbo.render.shared_colorbuffer
//! and shared_colorbuffer_clear draw the tex2d_r8 / tex2d_rg8 / tex2d_rg16f
//! attachment to the window afterwards and saw the blue of the clear colour.
//! ES 3.0.6 4.3.2 and 3.8.13 (table 3.24): an absent G or B is 0, an absent
//! A is 1.
//!
//! The fix: when a frame opens, the back end asks the GL core which channels
//! each attachment's format lacks (`framebuffer::bound_target_absent_channels`,
//! by the image's address, because `GlesDraw` carries the store's class
//! only), and the kick's PBESTATE_REG_WORD0 swizzle takes SWIZ.ZERO for an
//! absent green or blue lane and SWIZ.ONE for an absent alpha lane
//! (`pbe_swizzle`, `pbe_reg_words_absent`). The swizzle is the store's, so
//! it covers the background-object clear, a clear quad and every draw of the
//! frame alike.
//!
//! The GL calls run against the REAL PowerVR back end (`yakogl_backend_pvr`)
//! on the host platform of tests/es3_half_float_rt_pvr.rs: GPU memory is
//! host memory, every submit completes at once, and the submit hook records
//! PBESTATE_REG_WORD0 of each state the kick emits. What is pinned:
//!
//! - per format (R8, RG8, RG16F, RGB8, RGB16F; RGBA8 and RGBA16F as the
//!   contrast), renderbuffer and texture, after a clear, a clear and a draw,
//!   a draw alone and a scissored clear (the clear quad): the four swizzle
//!   fields of attachment 0;
//! - an RG8 texture at attachment 1 beside an RGBA8 one: its own fields;
//! - R16F (a one-channel surface) and R8UI / RG8UI (integer: a write mask,
//!   `fb_target_channel_mask_n`) keep the swizzle they had;
//! - `fb_absent_channels_at` itself, and that it answers 0 once another
//!   framebuffer is bound;
//! - `pbe_reg_words_absent` on the B,G,R,X order (XRGB8888) and that 0 is
//!   `pbe_reg_words`;
//! - glReadPixels of each target: the absent channels read 0, 0, 255;
//! - glBlitFramebuffer from the window into an R8 target: the CPU copy
//!   stores red, 0, 0, 255 (no kick).
//!
//! What it cannot see: the GPU. No pixel is written on the host, so the
//! read-back row shows the GL core's forcing over a zeroed store, not the
//! PBE; that the PBE's SWIZ.ZERO / SWIZ.ONE store 0 and 1 in an 8-bit and a
//! half-float lane is a board result (SWIZ.ONE in the alpha lane is how
//! GL_RGB targets have been stored since stage 1).
//!
//! Without the fix (`frame_begin_with` not asking, `target_absent` 0) the
//! R8, RG8 and RG16F rows fail: the lanes hold SOURCE_CHAN1 and
//! SOURCE_CHAN2.
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

use yakogl::backend::{GLES_SURF_R16F, GLES_SURF_R8UI, GLES_SURF_RGBA16F, GLES_SURF_RGBA8888, GLES_SURF_XRGB8888};
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
/// PBE REG_WORD0 of each emitted state of each kick submitted, and how many
/// states the kick has.
static KICKS: Mutex<Vec<([u64; 4], u32)>> = Mutex::new(Vec::new());

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
    let w0 = core::array::from_fn(|i| k.frag.pbe_word[i][0]);
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).push((w0, k.frag.num_rt.max(1)));
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

/// The end-of-tile program: its emit words copied, nothing recorded.
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

const CANVAS: i32 = 16;
/// The target's size: a whole tile, so the stride is the width.
const SIZE: i32 = 16;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAM: GLuint = 0;
static mut PROGRAM2: GLuint = 0;
static mut PROGRAM_UI: GLuint = 0;

const VS: &CStr = c"#version 300 es
layout(location = 0) in vec4 pos;
void main() { gl_Position = pos; }
";
const FS: &CStr = c"#version 300 es
precision highp float;
layout(location = 0) out vec4 color;
void main() { color = vec4(0.25, 0.5, 0.75, 0.5); }
";
/// Two outputs, for the attachment-1 row.
const FS2: &CStr = c"#version 300 es
precision highp float;
layout(location = 0) out vec4 c0;
layout(location = 1) out vec4 c1;
void main() { c0 = vec4(0.25, 0.5, 0.75, 0.5); c1 = vec4(0.75, 0.5, 0.25, 0.5); }
";
const FS_UI: &CStr = c"#version 300 es
precision highp float;
layout(location = 0) out highp uvec4 color;
void main() { color = uvec4(1u, 2u, 3u, 4u); }
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
    // SAFETY: plain GL calls.
    unsafe {
        let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
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
            PROGRAM_UI = program(FS_UI);
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_SCISSOR_TEST);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    KICKS.lock().unwrap().clear();
    g
}

fn ctx() -> &'static GlContext {
    // SAFETY: set once in `rig`, never freed; read under the lock.
    unsafe { &*CTX }
}

/// The two ways to give a framebuffer a colour image.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Renderbuffer,
    Texture,
}

/// A `SIZE`-square image of `internal`, attached at colour attachment `at`
/// of the bound framebuffer.
fn attach(kind: Kind, internal: GLenum, at: GLenum) -> GLuint {
    let mut name = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, internal, SIZE, SIZE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, at, GL_RENDERBUFFER, name);
            }
            Kind::Texture => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, 1, internal, SIZE, SIZE);
                glFramebufferTexture2D(GL_FRAMEBUFFER, at, GL_TEXTURE_2D, name, 0);
                glBindTexture(GL_TEXTURE_2D, 0);
            }
        }
    }
    name
}

/// A framebuffer with a `SIZE`-square image of `internal` at attachment 0,
/// bound and complete.
fn target(kind: Kind, internal: GLenum, name: &str) -> (GLuint, GLuint) {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        let image = attach(kind, internal, GL_COLOR_ATTACHMENT0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{name} {kind:?}: storage: {:?}", logs());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{name} {kind:?}: status");
        (fb, image)
    }
}

fn release(kind: Kind, fb: GLuint, name: GLuint) {
    // SAFETY: names made by the test.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        match kind {
            Kind::Renderbuffer => glDeleteRenderbuffers(1, &name),
            Kind::Texture => glDeleteTextures(1, &name),
        }
        glFinish();
    }
    KICKS.lock().unwrap().clear();
}

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/* PBESTATE_SWIZ (csbgen/rogue/pbestate.xml): the fragment's channel 0..3,
 * or a constant */
const R: u64 = 0;
const G: u64 = 1;
const B: u64 = 2;
const A: u64 = 3;
const ONE: u64 = 4;
const ZERO: u64 = 5;

/// The four swizzle fields of a PBESTATE_REG_WORD0, lane 0 first:
/// swiz_chan0 [20:22], swiz_chan1 [23:25], swiz_chan2 [26:28], swiz_chan3
/// [29:31] (pbestate.xml REG_WORD0).
fn lanes(w0: u64) -> [u64; 4] {
    core::array::from_fn(|k| (w0 >> (20 + 3 * k)) & 0x7)
}

/// What one frame of the bound framebuffer is made of.
#[derive(Clone, Copy, Debug)]
enum Frame {
    /// glClear of the whole target: the background object.
    Clear,
    /// glClear, then a triangle over the target.
    ClearDraw,
    /// The triangle alone: the frame loads the target first.
    Draw,
    /// A scissored glClear: the clear quad.
    ScissoredClear,
}

const FRAMES: [Frame; 4] = [Frame::Clear, Frame::ClearDraw, Frame::Draw, Frame::ScissoredClear];

/// Runs `frame` on the bound framebuffer with `prog` and submits it: the
/// swizzle of each state the last kick emitted.
fn kicked(frame: Frame, prog: GLuint, what: &str) -> Vec<[u64; 4]> {
    KICKS.lock().unwrap().clear();
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        glViewport(0, 0, SIZE, SIZE);
        /* green, blue and alpha that the absent lanes must not keep */
        glClearColor(0.0, 0.5, 1.0, 0.25);
        match frame {
            Frame::Clear | Frame::ClearDraw => glClear(GL_COLOR_BUFFER_BIT),
            Frame::ScissoredClear => {
                glEnable(GL_SCISSOR_TEST);
                glScissor(2, 2, 8, 8);
                glClear(GL_COLOR_BUFFER_BIT);
                glDisable(GL_SCISSOR_TEST);
            }
            Frame::Draw => {}
        }
        if matches!(frame, Frame::ClearDraw | Frame::Draw) {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
        }
        assert_eq!(glGetError(), GL_NO_ERROR, "{what} {frame:?}: {:?}", logs());
        glFinish();
    }
    let kicks = KICKS.lock().unwrap().clone();
    let (w0, n) = *kicks.last().unwrap_or_else(|| panic!("{what} {frame:?}: no kick: {:?}", logs()));
    w0[..n as usize].iter().map(|&w| lanes(w)).collect()
}

/// One row: (GL name, label, the channels the format lacks, the swizzle of
/// its store).
const ROWS: [(GLenum, &str, u8, [u64; 4]); 7] = [
    (GL_R8, "R8", 0xe, [R, ZERO, ZERO, ONE]),
    (GL_RG8, "RG8", 0xc, [R, G, ZERO, ONE]),
    (GL_RG16F, "RG16F", 0xc, [R, G, ZERO, ONE]),
    (GL_RGB8, "RGB8", 0x8, [R, G, B, ONE]),
    (GL_RGB16F, "RGB16F", 0x8, [R, G, B, ONE]),
    (GL_RGBA8, "RGBA8", 0, [R, G, B, A]),
    (GL_RGBA16F, "RGBA16F", 0, [R, G, B, A]),
];

/// The address the back end is handed for the bound framebuffer's
/// attachment 0.
fn target_gpu() -> u64 {
    let (m, _, _, off) = fb_target(ctx()).expect("a complete framebuffer");
    m.gpu + off as u64
}

/* ---- the tests --------------------------------------------------------------- */

/// Every format as a renderbuffer and as a texture, through each kind of
/// frame: the lanes of the channels it lacks take the PBE's constants.
#[test]
fn absent_lanes_take_the_pbe_constants() {
    let _g = rig();
    for kind in [Kind::Renderbuffer, Kind::Texture] {
        for (internal, name, absent, swizzle) in ROWS {
            let (fb, image) = target(kind, internal, name);
            assert_eq!(fb_absent_channels_at(ctx(), target_gpu()), absent, "{name} {kind:?}: absent channels");
            for frame in FRAMES {
                // SAFETY: read under the lock.
                let states = kicked(frame, unsafe { PROGRAM }, name);
                assert_eq!(states[0], swizzle, "{name} {kind:?} {frame:?}: REG_WORD0 swizzle");
            }
            release(kind, fb, image);
        }
    }
}

/// glReadPixels of each target after a clear and a draw: an absent green or
/// blue reads 0 and an absent alpha 255 (ES 3.0 4.3.2). The host stores no
/// pixel, so this is the GL core's half over a zeroed store.
#[test]
fn absent_channels_read_back_zero_zero_one() {
    let _g = rig();
    for kind in [Kind::Renderbuffer, Kind::Texture] {
        for (internal, name, absent, _) in ROWS {
            if absent == 0 {
                continue;
            }
            let (fb, image) = target(kind, internal, name);
            // SAFETY: read under the lock.
            kicked(Frame::ClearDraw, unsafe { PROGRAM }, name);
            let mut px = [0x5au8; 4 * (SIZE * SIZE) as usize];
            // SAFETY: room for SIZE x SIZE RGBA8 pixels.
            unsafe { glReadPixels(0, 0, SIZE, SIZE, GL_RGBA, GL_UNSIGNED_BYTE, px.as_mut_ptr() as *mut c_void) };
            // SAFETY: plain GL call.
            assert_eq!(unsafe { glGetError() }, GL_NO_ERROR, "{name} {kind:?}: glReadPixels");
            for p in px.as_chunks::<4>().0 {
                for (k, &v) in p.iter().enumerate() {
                    if absent & (1 << k) != 0 {
                        assert_eq!(v, if k == 3 { 255 } else { 0 }, "{name} {kind:?}: channel {k} of {p:?}");
                    }
                }
            }
            release(kind, fb, image);
        }
    }
}

/// An RG8 texture at attachment 1 beside an RGBA8 one: each state takes its
/// own attachment's swizzle.
#[test]
fn a_second_attachment_takes_its_own_constants() {
    let _g = rig();
    let (fb, c0) = target(Kind::Texture, GL_RGBA8, "RGBA8 + RG8");
    let c1 = attach(Kind::Texture, GL_RG8, GL_COLOR_ATTACHMENT1);
    let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1];
    // SAFETY: two draw buffers.
    unsafe {
        glDrawBuffers(2, bufs.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
    }
    for frame in [Frame::Clear, Frame::ClearDraw, Frame::Draw] {
        // SAFETY: read under the lock.
        let states = kicked(frame, unsafe { PROGRAM2 }, "RGBA8 + RG8");
        assert_eq!(states.len(), 2, "{frame:?}: two emitted states");
        assert_eq!(states[0], [R, G, B, A], "{frame:?}: RGBA8 at attachment 0");
        assert_eq!(states[1], [R, G, ZERO, ONE], "{frame:?}: RG8 at attachment 1");
    }
    // SAFETY: a name made here.
    unsafe { glDeleteTextures(1, &c1) };
    release(Kind::Texture, fb, c0);
}

/// A one-channel surface and the integer formats are left as they were: R16F
/// has no lane for the others (alpha's field reads ONE through
/// `target_alpha_bits`, as before), and an R or RG integer image is written
/// under a mask, not through the swizzle.
#[test]
fn one_channel_and_integer_surfaces_keep_their_swizzle() {
    let _g = rig();
    let (fb, tex) = target(Kind::Texture, GL_R16F, "R16F");
    assert_eq!(fb_target_format(ctx()), GLES_SURF_R16F);
    // SAFETY: read under the lock.
    assert_eq!(kicked(Frame::ClearDraw, unsafe { PROGRAM }, "R16F")[0], [R, G, B, ONE]);
    release(Kind::Texture, fb, tex);
    for (internal, name) in [(GL_R8UI, "R8UI"), (GL_RG8UI, "RG8UI")] {
        let (fb, tex) = target(Kind::Texture, internal, name);
        assert_eq!(fb_absent_channels_at(ctx(), target_gpu()), 0, "{name}: an integer image answers 0");
        // SAFETY: read under the lock.
        assert_eq!(kicked(Frame::Draw, unsafe { PROGRAM_UI }, name)[0], [R, G, B, A], "{name}");
        release(Kind::Texture, fb, tex);
    }
}

/// The question is the bound framebuffer's: an image that is not one of its
/// attachments, the default framebuffer and address 0 answer 0.
#[test]
fn only_an_attachment_of_the_bound_framebuffer_answers() {
    let _g = rig();
    let (fb, tex) = target(Kind::Texture, GL_R8, "R8");
    let gpu = target_gpu();
    assert_eq!(fb_absent_channels_at(ctx(), gpu), 0xe);
    assert_eq!(fb_absent_channels_at(ctx(), gpu + 4096), 0, "no attachment there");
    assert_eq!(fb_absent_channels_at(ctx(), 0), 0);
    let (fb2, tex2) = target(Kind::Texture, GL_RGBA8, "RGBA8");
    assert_eq!(fb_absent_channels_at(ctx(), gpu), 0, "another framebuffer is bound");
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, 0) };
    assert_eq!(fb_absent_channels_at(ctx(), gpu), 0, "the default framebuffer is bound");
    release(Kind::Texture, fb2, tex2);
    release(Kind::Texture, fb, tex);
}

/// The register words themselves: 0 absent channels is `pbe_reg_words`; the
/// constants follow the channel into the lane the format's order gives it
/// (XRGB8888 is B,G,R,X); a one-channel and an integer surface are untouched.
#[test]
fn reg_words_follow_the_lane_order() {
    for fmt in [GLES_SURF_RGBA8888, GLES_SURF_XRGB8888, GLES_SURF_RGBA16F, GLES_SURF_R16F, GLES_SURF_R8UI] {
        for alpha_one in [false, true] {
            assert_eq!(
                pbe_reg_words_absent(fmt, alpha_one, 0, 64, 32, 64, false, false),
                pbe_reg_words(fmt, alpha_one, 64, 32, 64, false, false),
                "fmt {fmt}"
            );
        }
    }
    let w = |fmt, absent| lanes(pbe_reg_words_absent(fmt, false, absent, 64, 32, 64, false, false)[0]);
    assert_eq!(w(GLES_SURF_RGBA8888, 0xe), [R, ZERO, ZERO, ONE]);
    assert_eq!(w(GLES_SURF_RGBA8888, 0xc), [R, G, ZERO, ONE]);
    assert_eq!(w(GLES_SURF_RGBA16F, 0xc), [R, G, ZERO, ONE]);
    assert_eq!(w(GLES_SURF_XRGB8888, 0xc), [ZERO, G, R, ONE], "blue is lane 0 of B,G,R,X");
    assert_eq!(w(GLES_SURF_XRGB8888, 0xe), [ZERO, ZERO, R, ONE]);
    assert_eq!(w(GLES_SURF_R16F, 0xe), [R, G, B, A], "a one-channel surface");
    assert_eq!(w(GLES_SURF_R8UI, 0xe), [R, G, B, A], "an integer surface");
    /* only the swizzle moves */
    let (a, b) = (
        pbe_reg_words_absent(GLES_SURF_RGBA8888, false, 0xe, 64, 32, 64, false, false),
        pbe_reg_words(GLES_SURF_RGBA8888, false, 64, 32, 64, false, false),
    );
    assert_eq!((a[0] & !(0xfff << 20), a[1]), (b[0] & !(0xfff << 20), b[1]));
}

/// glBlitFramebuffer from the window into an R8 target (dEQP
/// fbo.blit.default_framebuffer.r8_*_blit_from_default) is the GL core's
/// copy on the CPU, no kick: it writes the store the same way, the window's
/// red and 0, 0, 255.
#[test]
fn a_blit_from_the_window_stores_zero_zero_one() {
    let _g = rig();
    let (m, w, h, _) = fb_target(ctx()).expect("the window");
    assert_eq!(fb_target_format(ctx()), GLES_SURF_RGBA8888);
    // SAFETY: the window's store, w x h RGBA8888 texels at least.
    unsafe { core::ptr::write_bytes(m.cpu as *mut u8, 0x80, (w * h * 4) as usize) };
    for kind in [Kind::Renderbuffer, Kind::Texture] {
        let (fb, image) = target(kind, GL_R8, "R8");
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_READ_FRAMEBUFFER, 0);
            glBlitFramebuffer(0, 0, SIZE, SIZE, 0, 0, SIZE, SIZE, GL_COLOR_BUFFER_BIT, GL_NEAREST);
            assert_eq!(glGetError(), GL_NO_ERROR, "{kind:?}: {:?}", logs());
            glFinish();
        }
        assert!(KICKS.lock().unwrap().is_empty(), "{kind:?}: the copy is the CPU's");
        let (m, _, _, off) = fb_target(ctx()).expect("the R8 target");
        // SAFETY: the target's store, SIZE x SIZE RGBA8888 texels.
        let texel = unsafe { core::slice::from_raw_parts((m.cpu as *const u8).add(off as usize), 4) };
        assert_eq!(texel, [0x80, 0, 0, 255], "{kind:?}: the stored texel");
        release(kind, fb, image);
    }
}
