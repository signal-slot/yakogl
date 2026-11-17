// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! GL-level host test of a PowerVR draw into an integer colour attachment
//! beside attachments of another class (plan #361, item mixed-gl-e2e;
//! docs/notes/pvr-int-mixed-targets.md, RC1's "Left on the PowerVR" row).
//!
//! Through the GL core on the host (the real `pvr_backend_draw`, the
//! PowerVR command generators and the kick stood in for, as
//! tests/es3_mrt4_formats_pvr.rs does) each case builds a framebuffer,
//! draws with a fragment shader writing every output, and checks:
//!
//! - the back end accepts the draw: no GL error, one kick of `num_rt`
//!   targets, one end-of-tile program;
//! - the PBE STATE_WORD1 of every attachment (the words of
//!   tests/pvr_pbe_mixed_words.rs, `address_high` masked off);
//! - the key the GL core gave the draw (`gl::target_keys::draw_mixed_key`)
//!   is the mixed key of the formats the framebuffer holds;
//! - the packed pixel output registers: the shader of that key (the online
//!   chain, built here from the same source) run through the host byte
//!   model (tests/usc_exec, as tests/es3_int_out_alu_pvr.rs does) writes
//!   each target's dwords to its own registers. The program is built by the
//!   test, not read back from the back end.
//!
//! The mixes: A, RGBA8 + R32UI; B, R32I + RG32I (two integer layouts, the
//! shape of dEQP's `common.modf`: an `int` output beside an `ivec2` one;
//! registers 0 and 2, 1 is a hole); C, RGBA16F + R32UI + RGBA8. Draws that
//! stay refused by name: a colour mask that keeps some channels, and
//! blending, each with an integer attachment beside another.
//!
//! HOST-ONLY: no GPU runs and no pixel is read back. The draws are made
//! without a colour clear (the clear of a mix is the item pvr-clear-load's).
//! The three accepted cases are `#[ignore]`d: they pass only once the items
//! pvr-accept and chain-pack have landed (un-ignore them there). The two
//! refusals run now.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_pvr_int_mixed_gl
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
use yakogl::shaderc::online::powervr::chain::compile_fragment_for;
use yakogl::shaderc::online::sema::Stage;
use yakogl::shaderc::online::{lower, store};
use yakogl::target_key::{gles_surf_target_key, shc_blend_key_mixed, shc_mixed_out_layout};

mod usc_exec;

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
    _doutu_src0: u64,
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

/// The one context and backend of the file, set up once; the tests take turns.
/// Three draw buffers are on for the whole file.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
}

/// Draws one triangle over the bound framebuffer with `fs`, no clear, then
/// submits; the frame, or the GL error of the draw.
fn drawn(fs: &str) -> Result<Frame, GLenum> {
    let (e, frame) = try_draw(fs);
    match (e, frame) {
        (GL_NO_ERROR, Some(f)) => Ok(f),
        (GL_NO_ERROR, None) => panic!("no error and no frame: {:?}", logs()),
        (e, _) => Err(e),
    }
}

/// [`drawn`] without the assertion that a frame came out: the GL error and,
/// when the back end recorded the draw, its frame. A draw the back end
/// refuses gives an error or, for a variant it cannot build, no frame.
fn try_draw(fs: &str) -> (GLenum, Option<Frame>) {
    EMITS.lock().unwrap().clear();
    KICKS.lock().unwrap().clear();
    with_platform(|p| p.log.clear());
    let src = std::ffi::CString::new(fs).unwrap();
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, shader(GL_VERTEX_SHADER, VS));
        glAttachShader(p, shader(GL_FRAGMENT_SHADER, &src));
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link failed: {:?}", logs());
        glViewport(0, 0, SIZE, SIZE);
        glUseProgram(p);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        glFlush();
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
    (GL_NO_ERROR, Some(Frame { emits: emits[0].chunks(2).map(|p| decode(p[1])).collect(), num_rt: kicks[0].0 }))
}

/* ---- the mixes ------------------------------------------------------------------ */

/* PBE packmodes and source formats (csbgen/rogue/pbestate.xml, the values of
 * tests/pvr_pbe_mixed_words.rs and src/pvr/csbgen.rs) */
const U8U8U8U8: u32 = 0x00;
const F16F16F16F16: u32 = 0x0a;
const S32S32: u32 = 0x29;
const S32: u32 = 0x35;
const U32: u32 = 0x34;
const SOURCE_F16: u32 = 0;
const SOURCE_8: u32 = 1;

const fn emit(packmode: u32, norm: u32, source_format: u32, source_pos: u32, mrt_index: u32) -> Emit {
    Emit { packmode, norm, source_format, source_pos, mrt_index }
}

/// One mix: the attachments' formats, the fragment shader writing one
/// output per attachment, the PBE state each attachment must get, the first
/// output register of each target, and the value of every register the
/// program writes (register, dword), in program order.
struct Mix {
    name: &'static str,
    formats: &'static [GLenum],
    fs: &'static str,
    pbe: &'static [Emit],
    starts: &'static [u8],
    regs: &'static [(u32, u32)],
}

/// A: RGBA8 at r0, R32UI at r1. RGBA8 of (1, 0, 1, 0) is the bytes ff 00 ff
/// 00, R in the lowest.
const MIX_A: Mix = Mix {
    name: "A: RGBA8 + R32UI",
    formats: &[GL_RGBA8, GL_R32UI],
    fs: "#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out vec4 o0;
layout(location = 1) out uint o1;
void main() {
    o0 = vec4(1.0, 0.0, 1.0, 0.0);
    o1 = 7u;
}
",
    pbe: &[emit(U8U8U8U8, 1, SOURCE_8, 0, 0), emit(U32, 0, SOURCE_8, 1, 1)],
    starts: &[0, 1],
    regs: &[(0, 0x00ff_00ff), (1, 7)],
};

/// B: two integer layouts, the shape of `common.modf`: R32I at r0, RG32I at
/// r2-r3 (a two-register source sits on an even register; r1 is a hole the
/// program does not write).
const MIX_B: Mix = Mix {
    name: "B: R32I + RG32I",
    formats: &[GL_R32I, GL_RG32I],
    fs: "#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out int o0;
layout(location = 1) out ivec2 o1;
void main() {
    o0 = -5;
    o1 = ivec2(-6, 7);
}
",
    pbe: &[emit(S32, 0, SOURCE_8, 0, 0), emit(S32S32, 0, SOURCE_8, 2, 1)],
    starts: &[0, 2],
    regs: &[(0, -5i32 as u32), (2, -6i32 as u32), (3, 7)],
};

/// C: RGBA16F at r0-r1 (R, G then B, A as halves), R32UI at r2, RGBA8 at r3.
const MIX_C: Mix = Mix {
    name: "C: RGBA16F + R32UI + RGBA8",
    formats: &[GL_RGBA16F, GL_R32UI, GL_RGBA8],
    fs: "#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out vec4 o0;
layout(location = 1) out uint o1;
layout(location = 2) out vec4 o2;
void main() {
    o0 = vec4(1.0, 0.5, 0.25, 2.0);
    o1 = 9u;
    o2 = vec4(0.0, 1.0, 0.0, 1.0);
}
",
    pbe: &[emit(F16F16F16F16, 0, SOURCE_F16, 0, 0), emit(U32, 0, SOURCE_8, 2, 1), emit(U8U8U8U8, 1, SOURCE_8, 3, 2)],
    starts: &[0, 2, 3],
    regs: &[(0, 0x3800_3c00), (1, 0x4000_3400), (2, 9), (3, 0xff00_ff00)],
};

/// The compiler keeps its output in one arena that every compile resets.
static COMPILING: Mutex<()> = Mutex::new(());

/// The fragment program `src` under the variant `key`: the record's
/// `num_outputs` and its USC bytes.
fn compile_for(src: &str, key: u32) -> (u8, Vec<u8>) {
    let _g = COMPILING.lock().unwrap_or_else(|e| e.into_inner());
    store::with_scratch(|a| {
        let _keep = store::begin_keep().expect("keep");
        let text = a.alloc_str(src).unwrap();
        let l = lower::compile_to_ir(a, Stage::Fragment, text).unwrap_or_else(|e| panic!("front end: {}", e.msg));
        let rec = compile_fragment_for(&l.module, &l.reflection, a, key).unwrap_or_else(|e| panic!("chain: {e:?}"));
        // SAFETY: the record's code lives in `keep`, `usc_dw` dwords.
        let code = unsafe { core::slice::from_raw_parts(rec.usc, rec.usc_dw as usize) };
        (rec.num_outputs, code.iter().flat_map(|w| w.to_le_bytes()).collect())
    })
    .unwrap()
}

/// One accepted mix, end to end.
fn check(m: &Mix) {
    let _g = rig();
    let _f = Fbo::new(m.formats);
    let n = m.formats.len();

    /* the per-target keys of the formats the framebuffer holds, the key the
     * GL core gave this draw, and the register starts of the layout */
    let targets: Vec<u32> = (0..n)
        .map(|i| {
            gles_surf_target_key(fb_target_format_n(ctx(), i as u8)).unwrap_or_else(|| panic!("{}: a key", m.name))
        })
        .collect();
    let key = shc_blend_key_mixed(&targets).unwrap_or_else(|| panic!("{}: a mixed key", m.name));
    assert_eq!(yakogl::gl::target_keys::draw_mixed_key(ctx()), Some(key), "{}: the GL core's key", m.name);
    let (starts, span) = shc_mixed_out_layout(&targets).expect("a layout");
    assert_eq!(&starts[..n], m.starts, "{}: register starts", m.name);

    /* the draw is accepted: one kick of every target, the PBE state of each */
    let frame = drawn(m.fs).unwrap_or_else(|e| panic!("{}: draw error {e:#x}: {:?}", m.name, logs()));
    assert_eq!(frame.num_rt as usize, n, "{}: kick num_rt", m.name);
    assert_eq!(frame.emits[..n], *m.pbe, "{}: PBE states", m.name);
    assert!(frame.emits[n..].iter().all(|e| *e == decode(0)), "{}: no further state", m.name);

    /* the packed output registers the program of the key writes */
    let (num_outputs, image) = compile_for(m.fs, key);
    assert_eq!(num_outputs, span, "{}: the program's registers", m.name);
    let inp = usc_exec::sentinel_inputs(1.0e30);
    usc_exec::take_pixout_writes();
    let out = usc_exec::run_binary(&image, &inp, true).unwrap_or_else(|e| panic!("{}: byte model: {e}", m.name));
    let order: Vec<u32> = m.regs.iter().map(|r| r.0).collect();
    assert_eq!(usc_exec::take_pixout_writes(), order, "{}: registers written, in order", m.name);
    for (reg, want) in m.regs {
        assert_eq!(out.get(reg), Some(want), "{}: register {reg}: {out:x?}", m.name);
    }
    assert_eq!(out.len(), m.regs.len(), "{}: nothing else written: {out:x?}", m.name);
}

/* ---- the tests ----------------------------------------------------------------- */

#[test]
#[ignore = "needs the items pvr-accept and chain-pack (the back end refuses these draws until then)"]
fn mix_a_rgba8_beside_r32ui_draws() {
    check(&MIX_A);
}

#[test]
#[ignore = "needs the items pvr-accept and chain-pack (the back end refuses these draws until then)"]
fn mix_b_r32i_beside_rg32i_draws() {
    check(&MIX_B);
}

#[test]
#[ignore = "needs the items pvr-accept and chain-pack (the back end refuses these draws until then)"]
fn mix_c_rgba16f_r32ui_rgba8_draws() {
    check(&MIX_C);
}

/// A colour mask that keeps some channels is still refused with an integer
/// attachment beside another: no frame, no kick.
#[test]
fn partial_color_mask_stays_refused_by_name() {
    let _g = rig();
    let _f = Fbo::new(MIX_A.formats);
    // SAFETY: plain GL call; `rig` restores the mask.
    unsafe { glColorMask(1, 0, 1, 1) };
    let (e, frame) = try_draw(MIX_A.fs);
    assert!(frame.is_none(), "refused, not drawn: {e:#x}: {:?}", logs());
    assert!(logs().iter().any(|l| l.contains("refused")), "named: {:?}", logs());
    assert!(KICKS.lock().unwrap().is_empty(), "no kick");
}

/// An RGBA32UI attachment beside RGBA8 needs five output registers: refused
/// by name (the pixel has four), not drawn with the wrong packing.
#[test]
fn rgba32ui_beside_rgba8_stays_refused_by_name() {
    let _g = rig();
    let _f = Fbo::new(&[GL_RGBA8, GL_RGBA32UI]);
    let fs = "#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out vec4 o0;
layout(location = 1) out uvec4 o1;
void main() {
    o0 = vec4(1.0);
    o1 = uvec4(1u, 2u, 3u, 4u);
}
";
    let (e, frame) = try_draw(fs);
    assert!(frame.is_none() && e == GL_INVALID_OPERATION, "refused, not drawn: {e:#x}: {:?}", logs());
    assert!(logs().iter().any(|l| l.contains("draw refused")), "named: {:?}", logs());
    assert!(KICKS.lock().unwrap().is_empty(), "no kick");
}
