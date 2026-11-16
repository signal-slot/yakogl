// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR end-of-tile data of the draws the back end accepts today (plan #361,
//! item pvr-emit-pin). The GL core drives the REAL PowerVR back end
//! (`yakogl_backend_pvr`) on a host platform, as tests/es3_mrt4_gl_pvr.rs does;
//! the stand-in of `pvr_pds_generate_pixel_event_tq` records what the back end
//! hands it at the kick: the end-of-tile program selected (`doutu`, one of the
//! distinct values `p_query` reports per target count) and the PBESTATE
//! STATE_WORD0/1 pair of each target slot (WORD0 is the address and is not pinned).
//!
//! Pinned, as literals of what the back end does today, for one target of each
//! colour format class, two and three RGBA8 targets, two R32UI and two RG32I (two RGBA16F and two R32F are pinned as refused):
//! the program selection, WORD1 of every slot, and per format the
//! output-register dword count implied by the source positions
//! (`source_pos` WORD1 bits 14..15, `mrt_index` bits 16..18). The count table is
//! the one tests/pvr_pbe_mixed_words.rs models (RGBA8, R32UI 1; RGBA16F, RG32UI
//! 2), copied, and extended with the formats it does not name.
//!
//! The low six bits of WORD1 (`address_high`) are masked: the
//! allocator decides them. These are pins of today's behaviour: item
//! pvr-accept (mixed-class draws) must not move any of them. Mixed-class draws
//! are refused today and are not drawn here.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test pvr_eot_words_pin
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    feature = "shaderc-online",
    not(feature = "report-es2"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use core::sync::atomic::Ordering;
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend;
use yakogl::gl::buffer::glBindBuffer;
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

/* ---- the platform: heaps, a fence, no GPU ---------------------------------- */

const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

#[allow(dead_code)] // the memory is held, the back end reads it through its pointer
struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    /// Kicks submitted so far: the last fence.
    fence: i64,
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

/// The "GPU": a kick is counted and completes at once.
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
    doutu: u64,
    emit_words: *const u32,
    _num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    // SAFETY: the backend's emit words: two per target, PVR_BACKEND_MAX_RT of them.
    let words: [u32; 6] = unsafe { core::array::from_fn(|i| *emit_words.add(i)) };
    EOT.lock().unwrap_or_else(|e| e.into_inner()).push(Eot { doutu, words });
    // SAFETY: the backend's emit words, buffers and sizes.
    unsafe {
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the context --------------------------------------------------------------- */

/// One call of the end-of-tile generator: the program selected and the state
/// words emitted (two per target slot).
struct Eot {
    doutu: u64,
    words: [u32; 6],
}

static EOT: Mutex<Vec<Eot>> = Mutex::new(Vec::new());

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// Holds the file's lock; puts the stand-in back to 0 (the build's own
/// count) when a test ends, passed or not.
struct Rig(#[allow(dead_code)] MutexGuard<'static, ()>);

impl Drop for Rig {
    fn drop(&mut self) {
        backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
    }
}

/// The one context and back end of the file; the tests take turns. The
/// stand-in claims `targets` colour targets (0: the build's own count).
fn rig(targets: u8) -> Rig {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    backend::HOST_DRAW_TARGETS.store(targets, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(8, 8);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(0);
        glFinish();
    }
    drain_errors();
    with_platform(|p| p.log.clear());
    Rig(g)
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn kicks() -> i64 {
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    with_platform(|p| p.fence)
}

fn drain_errors() {
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// A bound framebuffer with an 8x8 renderbuffer of format `fmts[i]` on
/// colour attachment i.
fn framebuffer(fmts: &[GLenum]) -> (GLuint, Vec<GLuint>) {
    let n = fmts.len();
    let mut rbs = vec![0; n];
    let mut fb = 0;
    // SAFETY: room for `n` and one names.
    unsafe {
        glGenRenderbuffers(n as GLsizei, rbs.as_mut_ptr());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        for (i, &rb) in rbs.iter().enumerate() {
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, fmts[i], 8, 8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as GLenum, GL_RENDERBUFFER, rb);
        }
    }
    (fb, rbs)
}

fn release(fb: GLuint, rbs: &[GLuint]) {
    // SAFETY: names this test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fb);
        glDeleteRenderbuffers(rbs.len() as GLsizei, rbs.as_ptr());
    }
}

const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/* ---- the draws ------------------------------------------------------------------- */

const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

/// What a fragment output of a format class is written as.
#[derive(Clone, Copy)]
enum Ty {
    F,
    I,
    U,
}

/// A fragment program with one output per entry, at location i.
fn fs_for(tys: &[Ty]) -> String {
    let mut s = String::from("#version 300 es\nprecision mediump float;\nprecision highp int;\n");
    for (i, t) in tys.iter().enumerate() {
        let n = match t {
            Ty::F => "vec4",
            Ty::I => "ivec4",
            Ty::U => "uvec4",
        };
        s += &format!("layout(location = {i}) out {n} o{i};\n");
    }
    s += "void main() {\n";
    for (i, t) in tys.iter().enumerate() {
        let k = match t {
            Ty::F => "vec4(0.25, 0.5, 0.75, 1.0)",
            Ty::I => "ivec4(1, 2, 3, 4)",
            Ty::U => "uvec4(1u, 2u, 3u, 4u)",
        };
        s += &format!("o{i} = {k};\n");
    }
    s + "}\n"
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let c = std::ffi::CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [c.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

fn use_program(fs: &str) {
    let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, fs));
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "the program links");
        glUseProgram(p);
    }
}

fn draw_triangle() {
    static TRI: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    // SAFETY: TRI is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, TRI.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
    }
}

/// One colour format of a renderbuffer: the GL name, the output type of its
/// class and the output-register dwords the source positions must imply
/// (copied from tests/pvr_pbe_mixed_words.rs and extended).
struct Fmt {
    name: &'static str,
    gl: GLenum,
    ty: Ty,
    dwords: u32,
}

const RGBA8: Fmt = Fmt { name: "RGBA8", gl: GL_RGBA8, ty: Ty::F, dwords: 1 };
const RGB10_A2: Fmt = Fmt { name: "RGB10_A2", gl: GL_RGB10_A2, ty: Ty::F, dwords: 1 };
const RGBA16F: Fmt = Fmt { name: "RGBA16F", gl: GL_RGBA16F, ty: Ty::F, dwords: 2 };
const R32F: Fmt = Fmt { name: "R32F", gl: GL_R32F, ty: Ty::F, dwords: 1 };
const R32UI: Fmt = Fmt { name: "R32UI", gl: GL_R32UI, ty: Ty::U, dwords: 1 };
const RG32I: Fmt = Fmt { name: "RG32I", gl: GL_RG32I, ty: Ty::I, dwords: 2 };
const RGBA32UI: Fmt = Fmt { name: "RGBA32UI", gl: GL_RGBA32UI, ty: Ty::U, dwords: 4 };

/// What a draw handed the end-of-tile generator.
#[derive(Debug)]
struct Seen {
    /// The program selected.
    doutu: u64,
    /// WORD0, WORD1 per slot, the address bits (low six) cleared.
    words: Vec<[u32; 2]>,
}

/// Draws one triangle into the given attachments and returns the last
/// end-of-tile call of the frame, one entry per attachment.
fn draw(fmts: &[&Fmt]) -> Option<Seen> {
    let _r = rig(0);
    let tys: Vec<Ty> = fmts.iter().map(|f| f.ty).collect();
    let gl: Vec<GLenum> = fmts.iter().map(|f| f.gl).collect();
    let (fb, rbs) = framebuffer(&gl);
    assert_eq!(err(), GL_NO_ERROR);
    // SAFETY: plain GL calls; readable enums.
    unsafe {
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glDrawBuffers(fmts.len() as GLsizei, ATT.as_ptr());
    }
    assert_eq!(err(), GL_NO_ERROR);
    use_program(&fs_for(&tys));
    EOT.lock().unwrap_or_else(|e| e.into_inner()).clear();
    let before = kicks();
    draw_triangle();
    assert_eq!(err(), GL_NO_ERROR);
    let after = kicks();
    release(fb, &rbs);
    if after == before {
        return None;
    }
    let e = EOT.lock().unwrap_or_else(|e| e.into_inner());
    let last = e.last().unwrap_or_else(|| panic!("an end-of-tile call: {:?}", logs()));
    let words = (0..fmts.len()).map(|i| [last.words[2 * i] & !0x3f_u32, last.words[2 * i + 1] & !0x3f]).collect();
    Some(Seen { doutu: last.doutu, words })
}

fn source_pos(w1: u32) -> u32 {
    (w1 >> 14) & 3
}

fn mrt_index(w1: u32) -> u32 {
    (w1 >> 16) & 7
}

/// Draws `fmts`, which must be accepted, and compares the program selected
/// and WORD1 of every slot with the literals. The slot's output-register
/// start (`source_pos`) and `mrt_index` are checked against the layout the
/// formats' dword counts imply: each slot starts where the previous one
/// ended, a two-dword source at an even register.
fn pin(fmts: &[&Fmt], doutu: u64, word1: &[u32]) {
    let s = draw(fmts).unwrap_or_else(|| panic!("draw refused: {:?}", logs()));
    assert_eq!(s.doutu, doutu, "end-of-tile program");
    let got: Vec<u32> = s.words.iter().map(|w| w[1]).collect();
    assert_eq!(got, word1, "STATE_WORD1 (address bits cleared) {got:#010x?}");
    let mut next = 0u32;
    for (i, f) in fmts.iter().enumerate() {
        if f.dwords >= 2 {
            next = next.next_multiple_of(2);
        }
        assert_eq!(source_pos(got[i]), next, "{} (slot {i}) source_pos", f.name);
        assert_eq!(mrt_index(got[i]), i as u32, "{} (slot {i}) mrt_index", f.name);
        next += f.dwords;
    }
}

/* One target of each class: the program of one target (0x1001), source 0,
 * mrt 0; the words differ in source_format (bit 19), norm (13) and the pack
 * mode (7..12). */

#[test]
fn one_rgba8() {
    pin(&[&RGBA8], 0x1001, &[0x0008_2000]);
}

#[test]
fn one_rgb10_a2() {
    pin(&[&RGB10_A2], 0x1001, &[0x0008_2000]);
}

#[test]
fn one_rgba16f() {
    pin(&[&RGBA16F], 0x1001, &[0x0000_0500]);
}

#[test]
fn one_r32f() {
    pin(&[&R32F], 0x1001, &[0x0000_1b00]);
}

#[test]
fn one_r32ui() {
    pin(&[&R32UI], 0x1001, &[0x0008_1a00]);
}

#[test]
fn one_rg32i() {
    pin(&[&RG32I], 0x1001, &[0x0008_1480]);
}

#[test]
fn one_rgba32ui() {
    pin(&[&RGBA32UI], 0x1001, &[0x0008_0580]);
}

/* Several targets of one class: program 0x1000 + n, source positions by the
 * dword counts (the helper checks them), mrt_index = slot. */

#[test]
fn two_rgba8() {
    pin(&[&RGBA8, &RGBA8], 0x1002, &[0x0008_2000, 0x0009_6000]);
}

#[test]
fn three_rgba8() {
    pin(&[&RGBA8, &RGBA8, &RGBA8], 0x1003, &[0x0008_2000, 0x0009_6000, 0x000a_a000]);
}

#[test]
fn two_r32ui() {
    pin(&[&R32UI, &R32UI], 0x1002, &[0x0008_1a00, 0x0009_5a00]);
}

#[test]
fn two_rg32i() {
    /* the second starts at register 2: RG32I takes two dwords */
    pin(&[&RG32I, &RG32I], 0x1002, &[0x0008_1480, 0x0009_9480]);
}

/* RGBA8 and RGB10_A2 are both one dword and the unorm class: the same words
 * in either order. */
#[test]
fn rgba8_beside_rgb10_a2() {
    pin(&[&RGBA8, &RGB10_A2], 0x1002, &[0x0008_2000, 0x0009_6000]);
    pin(&[&RGB10_A2, &RGB10_A2], 0x1002, &[0x0008_2000, 0x0009_6000]);
}

/* Two RGBA16F and two R32F targets are not accepted today: the online
 * compiler refuses the blend variant of a shader with more than one colour
 * output of those formats, so no kick reaches the end-of-tile emit. Pinned as
 * a refusal (item pvr-accept must not move it by accident). */
#[test]
fn two_rgba16f_and_two_r32f_are_refused() {
    assert!(draw(&[&RGBA16F, &RGBA16F]).is_none(), "two RGBA16F");
    assert!(draw(&[&R32F, &R32F]).is_none(), "two R32F");
}

/* The output-register dword count per format, as the source position of a
 * following target shows it. The table is tests/pvr_pbe_mixed_words.rs's
 * (RGBA8, R32UI: 1; RGBA16F, RG32UI: 2), extended; only the formats an
 * accepted pair shows are read off the words, the others are the table's
 * (RGBA16F, R32F and RGBA32UI have no accepted pair today). */
#[test]
fn dwords_per_format_from_source_positions() {
    for (f, dw) in [(&RGBA8, 1), (&RGB10_A2, 1), (&R32UI, 1), (&RG32I, 2)] {
        assert_eq!(f.dwords, dw, "{} table", f.name);
        let s = draw(&[f, f]).unwrap_or_else(|| panic!("{} pair refused", f.name));
        assert_eq!(source_pos(s.words[1][1]), dw, "{}: the second target's source_pos", f.name);
    }
    assert_eq!([RGBA16F.dwords, R32F.dwords, RGBA32UI.dwords], [2, 1, 4]);
}
