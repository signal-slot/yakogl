// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of the PowerVR integer colour attachments beside an
//! attachment of another layout whose outputs pass the pixel's four output
//! registers (RC1 and RC14, plan #325 item mrt-frameload): the
//! `dEQP-GLES3.functional.fragment_out.random.*` lists the back end refused
//! as "integer and other colour attachments past the pixel's four output
//! registers".
//!
//! Through the GL core on the host (the real `pvr_backend_draw`, the
//! PowerVR command generators and the kick stood in for, as
//! tests/es3_pvr_mrt_load.rs does):
//!
//! - three attachments whose third does not fit the four registers (layout
//!   B, `target_key::shc_mixed_spans`, the compiler's `target_layout`) are
//!   drawn: no GL error, no refusal line, a kick of `num_rt` 3 whose
//!   end-of-tile program is the one of three emits with the last from tile
//!   buffer 0 (`eot_doutu_src0_tb_n[3]`, `event_pixel_pds_info_tb_n[3]`),
//!   the first two attachments' PBE `source_pos` at the registers the
//!   compiler wrote them, the third's at 0 (the tile buffer's dword 0), and
//!   the fragment program's PDS DMAs tile buffer 0's address into its shared
//!   registers. The lists are dEQP's random.50 (R16UI, RGBA16UI, RG8 as
//!   RGBA8), random.62 (R16I, RG8I, RGBA32I) and random.97 (RG8I, RG32I,
//!   RGBA8UI); random.85 and random.9 have the same shapes;
//! - a continued frame names the spilled attachment its load does not
//!   restore (the frame load writes output registers, not tile buffers):
//!   the draw is still made, and the gap is a log line, not silent;
//! - the layouts no end-of-tile program was captured for stay refused by
//!   name (`GL_INVALID_OPERATION`): two attachments with the second
//!   spilled (random.11: RG32I, RGBA32I) and three with the last two
//!   (random.89: R32UI, RGBA32UI, RG16UI); and so does SRGB8_ALPHA8 beside
//!   an integer attachment (random.69), which no per-target key carries.
//!
//! Three draw buffers, set with `backend::HOST_DRAW_TARGETS`.
//!
//! HOST-ONLY: no GPU runs and no pixel is read back; the picture is the
//! board's (AM62P single-case check of plan #325).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_int_beside
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
/// Each end-of-tile program built: the DOUTU of the USC program it runs
/// and its PBE STATE word pairs.
static EMITS: Mutex<Vec<(u64, Vec<u32>)>> = Mutex::new(Vec::new());
/// Each kick submitted.
static KICKS: Mutex<Vec<Kick>> = Mutex::new(Vec::new());
/// Each DMA a fragment PDS program was built with: the dwords it reads and
/// the shared register they land at.
static DMAS: Mutex<Vec<(Vec<u32>, u32)>> = Mutex::new(Vec::new());

/// One kick submitted.
#[derive(Clone, Debug)]
struct Kick {
    num_rt: u32,
    /// The end-of-tile program's PDS info (`event_pixel_pds_info`).
    pixel_info: u32,
}

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

/// `n` dwords of the "GPU" memory at `gpu` (zeros past what is mapped).
fn read_gpu(gpu: u64, n: usize) -> Vec<u32> {
    with_platform(|p| {
        let Some((&base, a)) = p.allocs.range(..=gpu).next_back() else { return vec![0; n] };
        // SAFETY: the u64 backing viewed as twice as many u32s.
        let words: &[u32] = unsafe { core::slice::from_raw_parts(a.mem.as_ptr() as *const u32, a.mem.len() * 2) };
        let at = ((gpu - base) / 4) as usize;
        (0..n).map(|i| words.get(at + i).copied().unwrap_or(0)).collect()
    })
}

/// The "GPU": records the kick's PBE REG words and completes it.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    let kick = Kick { num_rt: k.frag.num_rt, pixel_info: k.frag.event_pixel_pds_info };
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).push(kick);
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
            let words = read_gpu(k.address, k.dwords as usize);
            DMAS.lock().unwrap_or_else(|e| e.into_inner()).push((words, k.dest_offset));
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

/// Records each end-of-tile program's DOUTU and PBE STATE word pairs.
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
    // SAFETY: the backend's emit words, buffers and sizes.
    unsafe {
        EMITS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((doutu_src0, core::slice::from_raw_parts(emit_words, 2 * num_rt as usize).to_vec()));
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

const VS: &CStr = c"#version 300 es
layout(location = 0) in vec4 pos;
void main() { gl_Position = pos; }
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

/// The one context and backend of the file, set up once; the tests take
/// turns. Three draw buffers (`HOST_DRAW_TARGETS`) for the whole file.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    yakogl::osal::new_log_epoch();
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            backend::HOST_DRAW_TARGETS.store(3, Ordering::Relaxed);
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

/// `source_pos` (the first output register of the source, in dwords) and
/// `mrt_index` of a PBE STATE word 1 (pbestate.xml STATE_WORD1).
fn source_and_index(w1: u32) -> (u32, u32) {
    ((w1 >> 14) & 3, (w1 >> 16) & 7)
}

/// Links `fs` with the file's vertex shader and makes it current with the
/// full-screen triangle's array and the target's viewport.
fn use_program(fs: &CStr) {
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, fs));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        glViewport(0, 0, SIZE, SIZE);
        glUseProgram(p);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
    }
}

/// The GL error of one full-screen draw.
fn draw_triangle() -> GLenum {
    // SAFETY: plain GL calls.
    unsafe {
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glGetError()
    }
}

fn flush() {
    // SAFETY: plain GL call.
    unsafe { glFlush() };
}

fn forget() {
    EMITS.lock().unwrap().clear();
    KICKS.lock().unwrap().clear();
    DMAS.lock().unwrap().clear();
    with_platform(|p| p.log.clear());
}

/// One list of attachments and the fragment shader writing one output to
/// each.
struct Mix {
    name: &'static str,
    formats: &'static [GLenum],
    fs: &'static CStr,
}

/// The register attachments 0 and 1 are emitted from (layout B: each at the
/// next free register, two or more dwords on an even one), the third from
/// tile buffer 0's dword 0.
struct Spilled {
    mix: Mix,
    source_pos: [u32; 3],
}

const SPILLED: [Spilled; 3] = [
    Spilled {
        mix: Mix {
            name: "random.50: R16UI + RGBA16UI + RG8",
            formats: &[GL_R16UI, GL_RGBA16UI, GL_RG8],
            fs: c"#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out uint o0;
layout(location = 1) out uvec4 o1;
layout(location = 2) out vec2 o2;
void main() { o0 = 3u; o1 = uvec4(1u, 2u, 3u, 4u); o2 = vec2(0.25, 1.0); }
",
        },
        source_pos: [0, 2, 0],
    },
    Spilled {
        mix: Mix {
            name: "random.62: R16I + RG8I + RGBA32I",
            formats: &[GL_R16I, GL_RG8I, GL_RGBA32I],
            fs: c"#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out int o0;
layout(location = 1) out ivec2 o1;
layout(location = 2) out ivec4 o2;
void main() { o0 = -7; o1 = ivec2(5, -6); o2 = ivec4(-100000, 2, 300000, -4); }
",
        },
        source_pos: [0, 1, 0],
    },
    Spilled {
        mix: Mix {
            name: "random.97: RG8I + RG32I + RGBA8UI",
            formats: &[GL_RG8I, GL_RG32I, GL_RGBA8UI],
            fs: c"#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out ivec2 o0;
layout(location = 1) out ivec2 o1;
layout(location = 2) out uvec4 o2;
void main() { o0 = ivec2(-1, 1); o1 = ivec2(70000, -70000); o2 = uvec4(9u, 8u, 7u, 6u); }
",
        },
        source_pos: [0, 2, 0],
    },
];

/* the platform's end-of-tile programs (p_query): three emits, the last from
 * tile buffer 0 (`eot3_tb`), and tile buffer 0's address */
const EOT3_TB_DOUTU: u64 = 0x2003;
const EOT3_TB_PIXEL_INFO: u32 = 0x73;
const TILE_BUFFER_0: u64 = HEAP_GENERAL_BASE + 0x3E00_0000;

/// Draws `s` into a new frame and checks the kick, its end-of-tile program
/// and the fragment program's tile-buffer DMA.
fn spilled_third_is_drawn(s: &Spilled) {
    let m = &s.mix;
    let _g = rig();
    let _f = Fbo::new(m.formats);
    use_program(m.fs);
    forget();
    assert_eq!(draw_triangle(), GL_NO_ERROR, "{}: the draw: {:?}", m.name, logs());
    flush();
    let logs = logs();
    assert!(
        !logs.iter().any(|l| l.contains("refused") || l.contains("no fragment shader variant")),
        "{}: {logs:?}",
        m.name
    );
    let kicks = KICKS.lock().unwrap().clone();
    assert_eq!(kicks.len(), 1, "{}: one kick: {logs:?}", m.name);
    assert_eq!(kicks[0].num_rt, 3, "{}: num_rt", m.name);
    assert_eq!(kicks[0].pixel_info, EOT3_TB_PIXEL_INFO, "{}: the tile-buffer end-of-tile program's PDS info", m.name);
    let emits = EMITS.lock().unwrap().clone();
    assert!(!emits.is_empty(), "{}: an end-of-tile program", m.name);
    for (doutu, words) in &emits {
        assert_eq!(*doutu, EOT3_TB_DOUTU, "{}: three emits, the last from tile buffer 0", m.name);
        let got: Vec<(u32, u32)> = words.chunks(2).take(3).map(|p| source_and_index(p[1])).collect();
        let want: Vec<(u32, u32)> = (0..3).map(|i| (s.source_pos[i], i as u32)).collect();
        assert_eq!(got, want, "{}: (source_pos, mrt_index) of each emit", m.name);
    }
    /* the fragment program reads tile buffer 0's address from the shared
     * registers its record names */
    let dmas = DMAS.lock().unwrap().clone();
    let tb = [TILE_BUFFER_0 as u32, (TILE_BUFFER_0 >> 32) as u32];
    assert!(dmas.iter().any(|(w, _)| w[..] == tb), "{}: tile buffer 0's address DMA'd: {dmas:x?}", m.name);
}

#[test]
fn r16ui_rgba16ui_rg8_is_drawn_with_the_third_from_a_tile_buffer() {
    spilled_third_is_drawn(&SPILLED[0]);
}

#[test]
fn r16i_rg8i_rgba32i_is_drawn_with_the_third_from_a_tile_buffer() {
    spilled_third_is_drawn(&SPILLED[1]);
}

#[test]
fn rg8i_rg32i_rgba8ui_is_drawn_with_the_third_from_a_tile_buffer() {
    spilled_third_is_drawn(&SPILLED[2]);
}

/// A frame that loads its attachments (no clear, and its continuation
/// after a glFlush) loads the ones it emits from registers; the spilled
/// third is named, not restored, and both kicks are still made with the
/// same end-of-tile program.
#[test]
fn a_continued_frame_names_the_spilled_attachment_it_does_not_restore() {
    let m = &SPILLED[0].mix;
    let _g = rig();
    let _f = Fbo::new(m.formats);
    use_program(m.fs);
    forget();
    assert_eq!(draw_triangle(), GL_NO_ERROR, "{}: {:?}", m.name, logs());
    flush();
    assert_eq!(draw_triangle(), GL_NO_ERROR, "{}: the continued draw: {:?}", m.name, logs());
    flush();
    let logs = logs();
    assert!(!logs.iter().any(|l| l.contains("refused")), "{logs:?}");
    assert!(
        logs.iter().any(|l| l.contains("colour attachment 2") && l.contains("is not restored")),
        "the spilled attachment's load is named: {logs:?}"
    );
    let kicks = KICKS.lock().unwrap().clone();
    assert_eq!(kicks.len(), 2, "{logs:?}");
    for k in &kicks {
        assert_eq!((k.num_rt, k.pixel_info), (3, EOT3_TB_PIXEL_INFO));
    }
}

/// The layouts no end-of-tile program emits, and sRGB beside an integer
/// attachment, are refused by name with GL_INVALID_OPERATION; nothing is
/// kicked for them.
#[test]
fn the_layouts_without_an_end_of_tile_program_stay_refused_by_name() {
    use yakogl::pvr::target_keys::{REFUSE_FOUR_REGISTERS, REFUSE_SRGB};
    let refused = [
        (
            Mix {
                name: "random.11: RG32I + RGBA32I (two, the second spilled)",
                formats: &[GL_RG32I, GL_RGBA32I],
                fs: c"#version 300 es
precision highp int;
layout(location = 0) out ivec2 o0;
layout(location = 1) out ivec4 o1;
void main() { o0 = ivec2(1, 2); o1 = ivec4(3, 4, 5, 6); }
",
            },
            REFUSE_FOUR_REGISTERS,
        ),
        (
            Mix {
                name: "random.89: R32UI + RGBA32UI + RG16UI (three, the last two spilled)",
                formats: &[GL_R32UI, GL_RGBA32UI, GL_RG16UI],
                fs: c"#version 300 es
precision highp int;
layout(location = 0) out uint o0;
layout(location = 1) out uvec4 o1;
layout(location = 2) out uvec2 o2;
void main() { o0 = 1u; o1 = uvec4(2u); o2 = uvec2(3u, 4u); }
",
            },
            REFUSE_FOUR_REGISTERS,
        ),
        (
            Mix {
                name: "random.69: SRGB8_ALPHA8 + RG16UI",
                formats: &[GL_SRGB8_ALPHA8, GL_RG16UI],
                fs: c"#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out vec4 o0;
layout(location = 1) out uvec2 o1;
void main() { o0 = vec4(0.5); o1 = uvec2(7u, 8u); }
",
            },
            REFUSE_SRGB,
        ),
    ];
    for (m, why) in &refused {
        let _g = rig();
        let _f = Fbo::new(m.formats);
        use_program(m.fs);
        forget();
        assert_eq!(draw_triangle(), GL_INVALID_OPERATION, "{}: {:?}", m.name, logs());
        flush();
        assert!(logs().iter().any(|l| l == why), "{}: {why}: {:?}", m.name, logs());
        assert!(KICKS.lock().unwrap().is_empty(), "{}: nothing kicked", m.name);
    }
}
