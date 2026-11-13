// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! PowerVR (BXS-4-64): the float colour targets the back end draws today, and
//! what it hands the hardware for each (plan #360, item test-pvr-half: the
//! baseline for the PowerVR float-target design and arm).
//!
//! The GL calls run against the REAL PowerVR back end (`yakogl_backend_pvr`)
//! on a host platform written here: GPU memory is host memory and every
//! submit completes at once. The PowerVR's command generators are C
//! (driver/src/pvr); they are stand-ins that write words of the right count,
//! as in tests/depth_resolve_pvr_backend.rs, whose platform this one is cut
//! down from. Two of them record what the hardware would be given: the
//! pixel-event generator the PBE STATE word pair (`pbe_emit_words`: pack mode,
//! source format, norm), the submit hook the kick's PBE REG word pair
//! (`pbe_reg_words`: swizzle, size, stride). What is pinned, per target:
//!
//! - RGBA16F, and GL_RGB16F / GL_RG16F kept as four halves (the GL core
//!   hands over `GLES_SURF_RGBA16F`): PACKMODE F16F16F16F16 (10),
//!   SOURCE_FORMAT F16_PER_CHANNEL (0), norm clear, 8 bytes a pixel;
//! - R16F: PACKMODE F16 (51), SOURCE_FORMAT F16_PER_CHANNEL, 2 bytes a pixel;
//! - R32F: PACKMODE F32 (54), 4 bytes a pixel and SOURCE_FORMAT
//!   F16_PER_CHANNEL (0) as well (src/pvr/backend.rs:1038, `pbe_emit_words`:
//!   one source format for every float surface). The hardware field has two
//!   values only, F16_PER_CHANNEL and 8_PER_CHANNEL (pbestate.xml:155-158,
//!   `ROGUE_PBESTATE_SOURCE_FORMAT_*` in src/pvr/csbgen.rs:561-562), so
//!   there is no "F32" to give it: the F32 of Mesa's R32_SFLOAT row
//!   (vulkan/pvr_arch_formats.c:248, docs/notes/es3-float-targets.md
//!   section 3) is the row's accumulator / pack column, which the F32 pack
//!   mode here matches. Whether the PBE stores R32F right from an
//!   F16-per-channel source is a silicon question (#313); this file holds
//!   today's words.
//!
//! Also held: the surface the GL core hands over and its bytes a pixel
//! (renderbuffer and texture), for the formats that render today; and that
//! the 32-bit RG/RGBA names are not drawn (not in the set).
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online --test es3_half_float_rt_pvr
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

use yakogl::backend::{self, GLES_SURF_R16F, GLES_SURF_R32F, GLES_SURF_RGBA16F};
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
use yakogl::pvr::csbgen as csb;
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
/// The PBE STATE word pair of each end-of-tile program built (slot 0).
static EMITS: Mutex<Vec<[u32; 2]>> = Mutex::new(Vec::new());
/// The PBE REG word pair of attachment 0 of each kick submitted.
static KICKS: Mutex<Vec<[u64; 2]>> = Mutex::new(Vec::new());

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
    KICKS.lock().unwrap_or_else(|e| e.into_inner()).push(k.frag.pbe_word[0]);
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
    _num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    // SAFETY: the backend's emit words, buffers and sizes.
    unsafe {
        EMITS.lock().unwrap_or_else(|e| e.into_inner()).push([*emit_words, *emit_words.add(1)]);
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

const VS: &CStr = c"#version 300 es
layout(location = 0) in vec4 pos;
void main() { gl_Position = pos; }
";
const FS: &CStr = c"#version 300 es
precision highp float;
layout(location = 0) out vec4 color;
void main() { color = vec4(0.25, 0.5, 0.75, 1.0); }
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
            let (v, f) = (shader(GL_VERTEX_SHADER, VS), shader(GL_FRAGMENT_SHADER, FS));
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed: {:?}", logs());
            PROGRAM = p;
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn ctx() -> &'static GlContext {
    // SAFETY: set once in `rig`, never freed; read under the lock.
    unsafe { &*CTX }
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The two ways to give a framebuffer a colour image.
#[derive(Clone, Copy, Debug)]
enum Kind {
    Renderbuffer,
    Texture,
}

/// A framebuffer with a `SIZE`-square image of `internal` at attachment 0,
/// bound. `None` when the GL core refuses the storage or the framebuffer is
/// not complete (the status is the second field).
fn target(kind: Kind, internal: GLenum) -> (GLuint, GLuint, GLenum) {
    let (mut fb, mut name) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            Kind::Renderbuffer => {
                glGenRenderbuffers(1, &mut name);
                glBindRenderbuffer(GL_RENDERBUFFER, name);
                glRenderbufferStorage(GL_RENDERBUFFER, internal, SIZE, SIZE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, name);
            }
            Kind::Texture => {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                glTexStorage2D(GL_TEXTURE_2D, 1, internal, SIZE, SIZE);
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, name, 0);
            }
        }
        (fb, name, glCheckFramebufferStatus(GL_FRAMEBUFFER))
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
}

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// What the hardware was handed for one colour target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Words {
    /// PBE_STATE_WORD1 [7:12]
    packmode: u32,
    /// PBE_STATE_WORD1 [13]
    norm: u32,
    /// PBE_STATE_WORD1 [19]: 0 F16_PER_CHANNEL, 1 F32 (pbestate.xml)
    source_format: u32,
    /// PBE_REG_WORD0 [34:..]: the stride (width / 2 - 1) of the target
    reg_stride: u64,
}

/// Clears the bound framebuffer's target and draws a triangle over it, then
/// submits; the PBE words of that frame.
fn drawn() -> Words {
    EMITS.lock().unwrap().clear();
    KICKS.lock().unwrap().clear();
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        glViewport(0, 0, SIZE, SIZE);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glUseProgram(PROGRAM);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glFlush();
    }
    let emits = EMITS.lock().unwrap().clone();
    let kicks = KICKS.lock().unwrap().clone();
    assert_eq!(emits.len(), 1, "one end-of-tile program: {:?}", logs());
    assert_eq!(kicks.len(), 1, "one kick: {:?}", logs());
    let w1 = emits[0][1] as u64;
    Words {
        packmode: ((w1 >> 7) & 0x3f) as u32,
        norm: ((w1 >> 13) & 1) as u32,
        source_format: ((w1 >> 19) & 1) as u32,
        reg_stride: (kicks[0][0] >> 34) & 0x3fff,
    }
}

/* PBE packmodes and source formats (csbgen/rogue/pbestate.xml) */
const PACKMODE_F16F16F16F16: u32 = 10;
const PACKMODE_F16: u32 = 51;
const PACKMODE_F32: u32 = 54;
const SOURCE_F16_PER_CHANNEL: u32 = csb::ROGUE_PBESTATE_SOURCE_FORMAT_F16_PER_CHANNEL;
const SOURCE_8_PER_CHANNEL: u32 = csb::ROGUE_PBESTATE_SOURCE_FORMAT_8_PER_CHANNEL;

/// One row: (GL name, label, the surface the GL core hands over, bytes a
/// pixel, PACKMODE).
const ROWS: [(GLenum, &str, u8, u32, u32); 5] = [
    (GL_RGBA16F, "RGBA16F", GLES_SURF_RGBA16F, 8, PACKMODE_F16F16F16F16),
    (GL_RGB16F, "RGB16F", GLES_SURF_RGBA16F, 8, PACKMODE_F16F16F16F16),
    (GL_RG16F, "RG16F", GLES_SURF_RGBA16F, 8, PACKMODE_F16F16F16F16),
    (GL_R16F, "R16F", GLES_SURF_R16F, 2, PACKMODE_F16),
    (GL_R32F, "R32F", GLES_SURF_R32F, 4, PACKMODE_F32),
];

/// The surface the bound framebuffer's attachment 0 is drawn as.
fn surface() -> u8 {
    fb_target_format(ctx())
}

/* ---- the tests --------------------------------------------------------------- */

/// Each float renderbuffer: complete, the surface and bytes a pixel the GL
/// core hands over, and the PBE words (pack mode, norm, source format).
#[test]
fn renderbuffer_words() {
    let _g = rig();
    for (internal, name, surf, bpp, packmode) in ROWS {
        let (fb, rb, status) = target(Kind::Renderbuffer, internal);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{name}: renderbuffer status");
        assert_eq!(surface(), surf, "{name}: surface");
        assert_eq!(backend::gles_surf_bytes_per_pixel(surf), bpp, "{name}: bytes a pixel");
        let w = drawn();
        assert_eq!(w.packmode, packmode, "{name}: PACKMODE");
        assert_eq!(w.norm, 0, "{name}: norm clear (a float is not fully normalised)");
        /* today: F16 per channel for every float surface, R32F included */
        assert_eq!(w.source_format, SOURCE_F16_PER_CHANNEL, "{name}: SOURCE_FORMAT");
        release(Kind::Renderbuffer, fb, rb);
    }
}

/// The same for a texture attachment (all but GL_RGB16F, which is incomplete).
#[test]
fn texture_words() {
    let _g = rig();
    for (internal, name, surf, bpp, packmode) in ROWS {
        let (fb, tex, status) = target(Kind::Texture, internal);
        assert_eq!(err(), GL_NO_ERROR, "{name}: glTexStorage2D");
        /* a GL_RGB16F texture is not a colour attachment today (the
         * renderbuffer is); the other four are */
        if internal == GL_RGB16F {
            assert_eq!(status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{name}: texture status");
            release(Kind::Texture, fb, tex);
            continue;
        }
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{name}: texture status");
        assert_eq!(surface(), surf, "{name}: surface");
        assert_eq!(backend::gles_surf_bytes_per_pixel(surf), bpp, "{name}: bytes a pixel");
        let w = drawn();
        assert_eq!(w.packmode, packmode, "{name}: PACKMODE");
        assert_eq!(w.norm, 0, "{name}: norm clear");
        assert_eq!(w.source_format, SOURCE_F16_PER_CHANNEL, "{name}: SOURCE_FORMAT");
        release(Kind::Texture, fb, tex);
    }
}

/// An RGBA8 target next to the floats, for the contrast: normalised, 8 bits
/// per channel source, U8U8U8U8 (0).
#[test]
fn rgba8_is_the_contrast() {
    let _g = rig();
    let (fb, rb, status) = target(Kind::Renderbuffer, GL_RGBA8);
    assert_eq!(status, GL_FRAMEBUFFER_COMPLETE);
    let w = drawn();
    assert_eq!((w.packmode, w.norm, w.source_format), (0, 1, SOURCE_8_PER_CHANNEL));
    release(Kind::Renderbuffer, fb, rb);
}

/// The PBE register words' stride is the target's (the width in pairs less
/// one), the same for every float format: none of them has a stride rule of
/// its own.
#[test]
fn reg_words_stride_is_the_width() {
    let _g = rig();
    for (internal, name, ..) in ROWS {
        let (fb, rb, status) = target(Kind::Renderbuffer, internal);
        assert_eq!(status, GL_FRAMEBUFFER_COMPLETE, "{name}");
        let w = drawn();
        assert_eq!(w.reg_stride, (SIZE as u64 - 1) / 2, "{name}: REG_WORD0 stride");
        release(Kind::Renderbuffer, fb, rb);
    }
}

/// The hardware's source-format field has no F32 value: a PBE state word
/// for R32F cannot name one (pbestate.xml SOURCE_FORMAT, csbgen.rs:561-562),
/// so today's F16_PER_CHANNEL is the only non-8-bit choice.
#[test]
fn source_format_has_no_f32_value() {
    assert_eq!((SOURCE_F16_PER_CHANNEL, SOURCE_8_PER_CHANNEL), (0, 1));
}
