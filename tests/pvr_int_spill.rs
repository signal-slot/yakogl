// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
// SPDX-FileCopyrightText: 2014 The Android Open Source Project
//! PowerVR: integer colour attachments of one layout past the pixel's four
//! output registers are drawn, the attachments that do not fit through tile
//! buffers (issue #313, family "PowerVR: integer attachments past four
//! output registers need a tile-buffer spill").
//!
//! A BXS-4-64 pixel has four 32-bit output registers. dEQP-GLES3's shader
//! executor (`FragmentOutExecutor`, VK-GL-CTS 93bca01
//! `modules/glshared/glsShaderExecUtil.cpp`, Apache-2.0, Copyright 2014 The
//! Android Open Source Project) writes every result as its bits to integer
//! outputs, a vector an output and a matrix a column an output, into
//! renderbuffers of R32UI, RG32UI or RGBA32UI (three and four components):
//!
//! - `common.modf` of `vec3`/`vec4`: two RGBA32UI, eight dwords;
//! - `precision.{matrixcompmult,outerproduct,transpose}.*.mat4x2`: four
//!   RG32UI, eight dwords;
//! - the same for `mat4x3` and `mat4`: four RGBA32UI, sixteen dwords;
//!
//! and `fragment_out.array.{int,uint}.rgba32*` draws three RGBA32I/UI,
//! twelve. The fragment program of the layout's key
//! (`chain::build_fragment_with`) writes the attachments that fit to the
//! registers and stores the first that does not, and every one after it, to
//! tile buffers 0, 1, 2 (`pvr::mrt_layout::layout`); the end-of-tile program
//! that loads them back is the captured one of that layout (Mesa 26.2.1's
//! pco, tools/pco_shc/usc_blobs.txt): `eot2_tb1` for two targets with the
//! second spilled, `eot3_tb12` for three with the last two, `eot4_tb2` and
//! `eot4_tb3` for four with the last two and three.
//!
//! Through the GL core on the host (the real `pvr_backend_draw`, the
//! PowerVR command generators and the kick stood in for, as
//! tests/es3_pvr_int_mixed_gl.rs does) each case builds the framebuffer,
//! draws, and checks: no GL error and no refusal line; one kick of every
//! target; the end-of-tile program (`p_query`'s distinct DOUTU per static
//! slot); each attachment's PBE state, a spilled one's source at dword 0;
//! and the record of the key's program (the registers it writes, its tile
//! buffers, its key). The matrix cases are the 54 dEQP programs, both
//! stages, generated as tests/pvr_mat4_precision.rs generates them.
//!
//! Four draw buffers come from the host stand-in
//! (`backend::HOST_DRAW_TARGETS`); the PowerVR's own count is RC14's.
//!
//! HOST-ONLY: no GPU runs and no pixel is read back. The byte model has no
//! tile-buffer store, so the programs are not run through it
//! (tests/es3_pvr_mrt4.rs reads the stores). A spilled attachment is not
//! loaded at the start of a continued frame and a clear of it alone is not
//! carried out (src/pvr/backend.rs `attachment_out_reg`): the draws here
//! are made without a clear.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_int_spill
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
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend;
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::*;
use yakogl::osal::render::{PvrRenderKick, PvrRenderStatic, PVR_RENDER_EOT_MAX_RT};
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OSAL_QUERY_RENDER_STATIC};
use yakogl::pvr::backend::*;
use yakogl::pvr::ffi::*;
use yakogl::pvr::mrt_layout::{layout, EotProgram, Place};
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::shaderc::{shc_blend_key_int_rt, ShcIntRt};
use yakogl::target_key::gles_surf_target_key;

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
/// The PBE STATE word pair of each end-of-tile program built (slot 0).
static EMITS: Mutex<Vec<Vec<u32>>> = Mutex::new(Vec::new());
/// The DOUTU task control (the end-of-tile USC program) of each end-of-tile
/// PDS program built.
static DOUTUS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
/// The PBE REG word pair of attachment 0 of each kick submitted.
static KICKS: Mutex<Vec<(u32, [[u64; 2]; PVR_RENDER_EOT_MAX_RT])>> = Mutex::new(Vec::new());

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
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).push((k.frag.num_rt, k.frag.pbe_word));
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

/// Records the first PBE STATE word pair of each end-of-tile program.
#[no_mangle]
unsafe extern "C" fn pvr_pds_generate_pixel_event_tq(
    doutu_src0: u64,
    emit_words: *const u32,
    num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    DOUTUS.lock().unwrap_or_else(|e| e.into_inner()).push(doutu_src0);
    // SAFETY: the backend's emit words, buffers and sizes.
    unsafe {
        EMITS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(core::slice::from_raw_parts(emit_words, 2 * num_rt as usize).to_vec());
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

const VS: &str = "#version 300 es
in highp vec4 a_position;
void main() { gl_Position = a_position; }
";

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn shader(type_: GLenum, src: &str) -> GLuint {
    let src = std::ffi::CString::new(src).unwrap();
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

/// The one context and backend of the file, set up once; the tests take turns.
/// Four draw buffers are on for the whole file (the host stand-in
/// `backend::HOST_DRAW_TARGETS`: the executor's four-column matrix results
/// take four, whatever the build's own count is).
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    /* a refusal another test of the file logged first through glog_once!
     * logs again */
    yakogl::osal::new_log_epoch();
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS as u32, CANVAS as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_BLEND);
        glColorMask(1, 1, 1, 1);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn ctx() -> &'static GlContext {
    // SAFETY: set once in `rig`, never freed; read under the lock.
    unsafe { &*CTX }
}

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// A framebuffer of `SIZE`-square renderbuffers of the given formats on
/// attachments 0.., every one a draw buffer, bound.
struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Fbo {
    fn new(formats: &[GLenum]) -> Fbo {
        let n = formats.len();
        let (mut fb, mut rbs) = (0, vec![0; n]);
        // SAFETY: room for the names; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(n as i32, rbs.as_mut_ptr());
            let mut bufs = Vec::new();
            for (i, (&rb, &f)) in rbs.iter().zip(formats).enumerate() {
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, f, SIZE, SIZE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
                bufs.push(GL_COLOR_ATTACHMENT0 + i as u32);
            }
            glDrawBuffers(n as i32, bufs.as_ptr());
            assert_eq!(glGetError(), GL_NO_ERROR, "building the framebuffer");
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        Fbo { fb, rbs }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names made by `new`.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as i32, self.rbs.as_ptr());
            glFinish();
        }
    }
}

/// The PBE STATE word 1 of one emit, decoded (pbestate.xml STATE_WORD1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Emit {
    packmode: u32,
    norm: u32,
    source_format: u32,
    /// The first output register of the source (`source_pos`, in dwords).
    source_pos: u32,
    mrt_index: u32,
}

fn decode(w1: u32) -> Emit {
    Emit {
        packmode: (w1 >> 7) & 0x3f,
        norm: (w1 >> 13) & 1,
        source_format: (w1 >> 19) & 1,
        source_pos: (w1 >> 14) & 3,
        mrt_index: (w1 >> 16) & 7,
    }
}

/// What one frame of a draw handed the hardware.
#[derive(Debug)]
struct Frame {
    /// Every PBE STATE pair the one end-of-tile program was built from.
    emits: Vec<Emit>,
    /// The kick's `num_rt`.
    num_rt: u32,
    /// The end-of-tile USC program's DOUTU task control.
    doutu: u64,
}

/// Draws one triangle over the bound framebuffer with the program of `vs`
/// and `fs` (its position attribute is `a_position`), no clear, then
/// submits; the frame, or the GL error of the draw.
fn drawn(vs: &str, fs: &str) -> Result<Frame, GLenum> {
    let (e, frame) = try_draw(vs, fs);
    match (e, frame) {
        (GL_NO_ERROR, Some(f)) => Ok(f),
        (GL_NO_ERROR, None) => panic!("no error and no frame: {:?}", logs()),
        (e, _) => Err(e),
    }
}

/// [`drawn`] without the assertion that a frame came out: the GL error and,
/// when the back end recorded the draw, its frame. A draw the back end
/// refuses gives an error or, for a variant it cannot build, no frame.
fn try_draw(vs: &str, fs: &str) -> (GLenum, Option<Frame>) {
    EMITS.lock().unwrap().clear();
    DOUTUS.lock().unwrap().clear();
    KICKS.lock().unwrap().clear();
    with_platform(|p| p.log.clear());
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, vs));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        let at = glGetAttribLocation(p, c"a_position".as_ptr());
        assert!(at >= 0, "a_position has a location");
        glViewport(0, 0, SIZE, SIZE);
        glUseProgram(p);
        glVertexAttribPointer(at as GLuint, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(at as GLuint);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        glFlush();
        glDisableVertexAttribArray(at as GLuint);
        glUseProgram(0);
        glDeleteProgram(p);
        if e != GL_NO_ERROR {
            return (e, None);
        }
    }
    let emits = EMITS.lock().unwrap().clone();
    let kicks = KICKS.lock().unwrap().clone();
    if emits.is_empty() && kicks.is_empty() {
        return (GL_NO_ERROR, None);
    }
    assert_eq!(emits.len(), 1, "one end-of-tile program: {:?}", logs());
    assert_eq!(kicks.len(), 1, "one kick: {:?}", logs());
    let doutu = DOUTUS.lock().unwrap()[0];
    (GL_NO_ERROR, Some(Frame { emits: emits[0].chunks(2).map(|p| decode(p[1])).collect(), num_rt: kicks[0].0, doutu }))
}

/* ---- the lists ------------------------------------------------------------------- */

/* PBE packmodes and source format (csbgen/rogue/pbestate.xml) */
const U32U32U32U32: u32 = 0x0b;
const U32U32: u32 = 0x28;
const S32S32S32S32: u32 = 0x0c;
const SOURCE_8: u32 = 1;

/// The stood-in static slots (`p_query`) of the tile-buffer programs.
fn slot(p: EotProgram) -> u64 {
    match p {
        EotProgram::Eot2Tb1 => 0x2002,
        EotProgram::Eot3Tb => 0x2003,
        EotProgram::Eot3Tb12 => 0x2001,
        EotProgram::Eot4Tb { spilled: 1 } => 0x2004,
        EotProgram::Eot4Tb { spilled: 2 } => 0x3002,
        EotProgram::Eot4Tb { .. } => 0x3003,
        EotProgram::Plain { targets } => 0x1000 + u64::from(targets),
    }
}

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// The record of the fragment program `src` under the variant `key`: its
/// `num_outputs`, `num_tile_buffers` and `blend_key`, or the refusal.
fn record_for(src: &str, key: u32) -> Result<(u8, u8, u32), String> {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).map_err(|e| format!("front end: {}", e.msg))?;
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).map_err(|e| format!("chain: {e:?}"))?;
        Ok((rec.num_outputs, rec.num_tile_buffers, rec.blend_key))
    })
    .unwrap()
}

/// `n` attachments of one integer format drawn with `vs` and `fs`, end to
/// end: the layout is `mrt_layout::layout`'s for the format's dwords, and
/// the draw, the kick, the end-of-tile program, the PBE states and the
/// record of the layout's key all follow it. `want` is the end-of-tile
/// program and the tile buffers the caller expects of the list.
fn check(name: &str, format: GLenum, packmode: u32, n: usize, vs: &str, fs: &str, want: (EotProgram, u8)) {
    let _g = rig();
    let _f = Fbo::new(&vec![format; n]);

    /* one layout everywhere: no mixed key, the layout's own key */
    let surf = fb_target_format_n(ctx(), 0);
    assert!((1..n).all(|i| fb_target_format_n(ctx(), i as u8) == surf), "{name}: one surface format");
    let key = gles_surf_target_key(surf).unwrap_or_else(|| panic!("{name}: a key"));
    let l = backend::gles_surf_int_layout(surf).unwrap_or_else(|| panic!("{name}: an integer layout"));
    let rt = ShcIntRt { channels: l.channels, bits: l.bits, signed: l.signed };
    assert_eq!(key, shc_blend_key_int_rt(rt), "{name}: the layout's key");
    assert_eq!(yakogl::gl::target_keys::draw_mixed_key(ctx()), None, "{name}: no mixed key");

    /* where each attachment goes, and the program that emits that */
    let lay = layout(&vec![rt.dwords() as u8; n]).unwrap_or_else(|e| panic!("{name}: {e:?}"));
    assert_eq!((lay.program, lay.tile_buffers), want, "{name}: the layout");
    assert!(lay.tile_buffers != 0, "{name}: past the four registers");

    /* the draw is accepted: one kick of every target, the end-of-tile
     * program of the layout, each attachment's PBE state */
    let frame = drawn(vs, fs).unwrap_or_else(|e| panic!("{name}: draw error {e:#x}: {:?}", logs()));
    assert_eq!(frame.num_rt as usize, n, "{name}: kick num_rt");
    assert_eq!(frame.doutu, slot(lay.program), "{name}: the end-of-tile program ({})", lay.program.name());
    let pbe: Vec<Emit> = (0..n)
        .map(|i| Emit {
            packmode,
            norm: 0,
            source_format: SOURCE_8,
            source_pos: u32::from(lay.place[i].source_pos()),
            mrt_index: i as u32,
        })
        .collect();
    assert_eq!(frame.emits[..n], pbe[..], "{name}: PBE states");
    assert!(frame.emits[n..].iter().all(|e| *e == decode(0)), "{name}: no further state");
    let spilled = (0..n).filter(|&i| matches!(lay.place[i], Place::TileBuffer { .. })).count();
    assert_eq!(spilled, usize::from(lay.tile_buffers), "{name}: the spilled attachments");
    assert!((n - spilled..n).all(|i| pbe[i].source_pos == 0), "{name}: a spilled source is at dword 0");
    let bad = ["refused", "no fragment shader variant", "not emitted"];
    assert!(!logs().iter().any(|l| bad.iter().any(|b| l.contains(b))), "{name}: {:?}", logs());

    /* the program of the key: the registers of the attachments that fit,
     * the tile buffers of the others, and the key the back end checks */
    assert_eq!(record_for(fs, key), Ok((lay.output_regs, lay.tile_buffers, key)), "{name}: the record");
}

/// `n` integer vector outputs of `ty` at locations 0.., each a constant.
fn vector_fs(ty: &str, n: usize) -> String {
    let mut s = String::from("#version 300 es\nprecision highp float;\nprecision highp int;\n");
    for i in 0..n {
        s += &format!("layout(location = {i}) out highp {ty} o{i};\n");
    }
    s += "void main() {\n";
    for i in 0..n {
        s += &format!("    o{i} = {ty}({});\n", 3 * i + 1);
    }
    s + "}\n"
}

/* ---- two and three targets ------------------------------------------------------- */

/// Two RGBA32UI targets: the first in the four registers, the second in
/// tile buffer 0, emitted by the two-target program `eot2_tb1`.
#[test]
fn two_rgba32ui_targets_draw() {
    check("RGBA32UI x2", GL_RGBA32UI, U32U32U32U32, 2, VS, &vector_fs("uvec4", 2), (EotProgram::Eot2Tb1, 1));
}

/// dEQP's fragment executor for `modf` of `vec3` and `vec4`
/// (tests/es3_pvr_modf.rs): both results as their bits to two RGBA32UI
/// attachments, a `uvec3` output and a `uvec4` one alike.
#[test]
fn modf_vec3_and_vec4_executor_draws() {
    for (ty, uty) in [("vec3", "uvec3"), ("vec4", "uvec4")] {
        for prec in PRECISIONS {
            let vs = format!(
                "#version 300 es\nin highp vec4 a_position;\nin {prec} {ty} a_in0;\nout {prec} {ty} vtx_out_in0;\n\
                 void main (void)\n{{\n\tgl_Position = a_position;\n\tgl_PointSize = 1.0;\n\tvtx_out_in0 = a_in0;\n}}\n"
            );
            let fs = format!(
                "#version 300 es\nin {prec} {ty} vtx_out_in0;\n\
                 layout(location = 0) out highp {uty} o_out0;\nlayout(location = 1) out highp {uty} o_out1;\n\
                 void main (void)\n{{\n\t{prec} {ty} in0 = vtx_out_in0;\n\t{prec} {ty} out0;\n\t{prec} {ty} out1;\n\
                 \tout0 = modf(in0, out1);\n\to_out0 = floatBitsToUint(out0);\n\to_out1 = floatBitsToUint(out1);\n}}\n"
            );
            let name = format!("common.modf.{ty}_{prec}_fragment");
            check(&name, GL_RGBA32UI, U32U32U32U32, 2, &vs, &fs, (EotProgram::Eot2Tb1, 1));
        }
    }
}

/// Three RGBA32UI and three RGBA32I targets (dEQP
/// `fragment_out.array.{uint,int}.rgba32*`): the first in the registers,
/// the last two in tile buffers 0 and 1, emitted by `eot3_tb12`.
#[test]
fn three_rgba32_targets_draw() {
    check("RGBA32UI x3", GL_RGBA32UI, U32U32U32U32, 3, VS, &vector_fs("uvec4", 3), (EotProgram::Eot3Tb12, 2));
    check("RGBA32I x3", GL_RGBA32I, S32S32S32S32, 3, VS, &vector_fs("ivec4", 3), (EotProgram::Eot3Tb12, 2));
}

/// A colour mask that keeps some channels of two RGBA32UI targets stays
/// refused: the masked key's program reads the pixel back from the
/// registers, which a spilled target has none of, so the compiler has no
/// variant and the draw is dropped (no kick), never drawn unmasked.
#[test]
fn a_partial_colour_mask_over_spilled_targets_is_not_drawn() {
    let _g = rig();
    let _f = Fbo::new(&[GL_RGBA32UI, GL_RGBA32UI]);
    // SAFETY: plain GL call; `rig` restores the mask.
    unsafe { glColorMask(1, 0, 1, 1) };
    let (_, frame) = try_draw(VS, &vector_fs("uvec4", 2));
    assert!(frame.is_none(), "not drawn: {:?}", logs());
    assert!(KICKS.lock().unwrap().is_empty(), "no kick");
}

/* ---- the executor's matrix results: four targets ---------------------------------- */

const PRECISIONS: [&str; 3] = ["lowp", "mediump", "highp"];

/// Columns and rows of a GLSL matrix or vector type (a vector: one column).
fn shape(t: &str) -> (usize, usize) {
    match t {
        "vec2" => (1, 2),
        "vec3" => (1, 3),
        "vec4" => (1, 4),
        "mat2x4" => (2, 4),
        "mat3x4" => (3, 4),
        "mat4" => (4, 4),
        "mat4x2" => (4, 2),
        "mat4x3" => (4, 3),
        _ => panic!("no shape for {t}"),
    }
}

/// One of the 9 case shapes: the function's name, the argument types, the
/// return type (the case's name) and the statement.
struct Case {
    func: &'static str,
    ins: &'static [&'static str],
    out: &'static str,
    stmt: &'static str,
}

const CASES: &[Case] = &[
    Case { func: "matrixcompmult", ins: &["mat4", "mat4"], out: "mat4", stmt: "out0 = matrixCompMult(in0, in1);" },
    Case {
        func: "matrixcompmult",
        ins: &["mat4x2", "mat4x2"],
        out: "mat4x2",
        stmt: "out0 = matrixCompMult(in0, in1);",
    },
    Case {
        func: "matrixcompmult",
        ins: &["mat4x3", "mat4x3"],
        out: "mat4x3",
        stmt: "out0 = matrixCompMult(in0, in1);",
    },
    Case { func: "outerproduct", ins: &["vec4", "vec4"], out: "mat4", stmt: "out0 = outerProduct(in0, in1);" },
    Case { func: "outerproduct", ins: &["vec2", "vec4"], out: "mat4x2", stmt: "out0 = outerProduct(in0, in1);" },
    Case { func: "outerproduct", ins: &["vec3", "vec4"], out: "mat4x3", stmt: "out0 = outerProduct(in0, in1);" },
    Case { func: "transpose", ins: &["mat4"], out: "mat4", stmt: "out0 = transpose(in0);" },
    Case { func: "transpose", ins: &["mat2x4"], out: "mat4x2", stmt: "out0 = transpose(in0);" },
    Case { func: "transpose", ins: &["mat3x4"], out: "mat4x3", stmt: "out0 = transpose(in0);" },
];

/// The executor's integer outputs of `out0`: their declarations and the
/// per-column writes.
fn outputs(out: &str) -> (String, String) {
    let (cols, rows) = shape(out);
    let mut decl = String::new();
    let mut write = String::new();
    for c in 0..cols {
        decl += &format!("layout(location = {c}) out highp uvec{rows} o_out0_{c};\n");
        write += &format!("    o_out0_{c} = floatBitsToUint(out0[{c}]);\n");
    }
    (decl, write)
}

/// The `_vertex` case: the vertex shader evaluates, a flat varying carries
/// the result, the fragment shader writes its columns.
fn vertex_case(p: &str, c: &Case) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_position;\n");
    for (i, t) in c.ins.iter().enumerate() {
        vs += &format!("in {p} {t} a_in{i};\n");
    }
    vs += &format!("flat out {p} {} vtx_out_out0;\nvoid main (void)\n{{\n", c.out);
    vs += "    gl_Position = a_position;\n    gl_PointSize = 1.0;\n";
    for (i, t) in c.ins.iter().enumerate() {
        vs += &format!("    {p} {t} in{i} = a_in{i};\n");
    }
    vs += &format!("    {p} {} out0;\n    {}\n    vtx_out_out0 = out0;\n}}\n", c.out, c.stmt);
    let (decl, write) = outputs(c.out);
    let fs = format!(
        "#version 300 es\nflat in {p} {o} vtx_out_out0;\n{decl}void main (void)\n{{\n    {p} {o} out0 = vtx_out_out0;\n{write}}}\n",
        o = c.out
    );
    (vs, fs)
}

/// The `_fragment` case: flat varyings carry the arguments, the fragment
/// shader evaluates and writes the columns.
fn fragment_case(p: &str, c: &Case) -> (String, String) {
    let mut vs = String::from("#version 300 es\nin highp vec4 a_position;\n");
    for (i, t) in c.ins.iter().enumerate() {
        vs += &format!("in {p} {t} a_in{i};\nflat out {p} {t} vtx_out_in{i};\n");
    }
    vs += "void main (void)\n{\n    gl_Position = a_position;\n    gl_PointSize = 1.0;\n";
    for i in 0..c.ins.len() {
        vs += &format!("    vtx_out_in{i} = a_in{i};\n");
    }
    vs += "}\n";
    let (decl, write) = outputs(c.out);
    let mut fs = String::from("#version 300 es\n");
    for (i, t) in c.ins.iter().enumerate() {
        fs += &format!("flat in {p} {t} vtx_out_in{i};\n");
    }
    fs += &decl;
    fs += "void main (void)\n{\n";
    for (i, t) in c.ins.iter().enumerate() {
        fs += &format!("    {p} {t} in{i} = vtx_out_in{i};\n");
    }
    fs += &format!("    {p} {} out0;\n    {}\n{write}}}\n", c.out, c.stmt);
    (vs, fs)
}

/// dEQP-GLES3 `precision.{matrixcompmult,outerproduct,transpose}.{lowp,
/// mediump,highp}_{vertex,fragment}.{mat4,mat4x2,mat4x3}` (54 cases): the
/// executor's four column outputs into four attachments. A `mat4x2` result
/// is four RG32UI (r0-1, r2-3, tile buffers 0 and 1: `eot4_tb2`); `mat4x3`
/// and `mat4` are four RGBA32UI (r0-3, tile buffers 0, 1, 2: `eot4_tb3`).
#[test]
fn the_54_matrix_executor_cases_draw_into_four_attachments() {
    let mut drawn = 0;
    for p in PRECISIONS {
        for c in CASES {
            let (format, packmode, want) = match shape(c.out).1 {
                2 => (GL_RG32UI, U32U32, (EotProgram::Eot4Tb { spilled: 2 }, 2)),
                _ => (GL_RGBA32UI, U32U32U32U32, (EotProgram::Eot4Tb { spilled: 3 }, 3)),
            };
            for (stage, (vs, fs)) in [("vertex", vertex_case(p, c)), ("fragment", fragment_case(p, c))] {
                let name = format!("precision.{}.{p}_{stage}.{}", c.func, c.out);
                check(&name, format, packmode, 4, &vs, &fs, want);
                drawn += 1;
            }
        }
    }
    assert_eq!(drawn, 54);
}
