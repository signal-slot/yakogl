// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.

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

use yakogl::backend::{self, *};
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::{glClearBufferfv, glClearBufferiv, glClearBufferuiv};
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

/// A fragment program per class of the fourth output: the first three are
/// RGBA8 float outputs, `o3` is what the attachment-3 format reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Class {
    Float,
    Int,
    Uint,
}

const CLASSES: [Class; 3] = [Class::Float, Class::Int, Class::Uint];

static mut PROGRAMS: [GLuint; 3] = [0; 3];

const VS: &CStr = c"#version 300 es
layout(location = 0) in vec4 pos;
void main() { gl_Position = pos; }
";

fn fs(class: Class) -> String {
    let (ty, val) = match class {
        Class::Float => ("vec4", "vec4(0.25, 0.5, 0.75, 1.0)"),
        Class::Int => ("ivec4", "ivec4(1, 2, 3, 4)"),
        Class::Uint => ("uvec4", "uvec4(1u, 2u, 3u, 4u)"),
    };
    format!(
        "#version 300 es
precision highp float;
precision highp int;
layout(location = 0) out vec4 o0;
layout(location = 1) out vec4 o1;
layout(location = 2) out vec4 o2;
layout(location = 3) out {ty} o3;
void main() {{
    o0 = vec4(1.0, 0.0, 0.0, 1.0);
    o1 = vec4(0.0, 1.0, 0.0, 1.0);
    o2 = vec4(0.0, 0.0, 1.0, 1.0);
    o3 = {val};
}}
"
    )
}

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
/// The four-target stand-in is on for the whole file.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
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
            let v = shader(GL_VERTEX_SHADER, VS);
            for (i, class) in CLASSES.iter().enumerate() {
                let src = std::ffi::CString::new(fs(*class)).unwrap();
                let f = shader(GL_FRAGMENT_SHADER, &src);
                let p = glCreateProgram();
                glAttachShader(p, v);
                glAttachShader(p, f);
                glLinkProgram(p);
                let mut ok: GLint = 0;
                glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
                assert_eq!(ok, 1, "{class:?}: link failed: {:?}", logs());
                PROGRAMS[i] = p;
            }
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

static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

/// A framebuffer of four `SIZE`-square renderbuffers: RGBA8 at 0-2 and
/// `internal` at attachment 3, all four draw buffers on, bound.
struct Fbo {
    fb: GLuint,
    rbs: [GLuint; 4],
    /// glCheckFramebufferStatus, and the first GL error of building it.
    status: GLenum,
    build_err: GLenum,
}

impl Fbo {
    fn new(internal: GLenum) -> Fbo {
        let (mut fb, mut rbs) = (0, [0; 4]);
        // SAFETY: room for the names; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glGenRenderbuffers(4, rbs.as_mut_ptr());
            for (i, &rb) in rbs.iter().enumerate() {
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorage(GL_RENDERBUFFER, if i == 3 { internal } else { GL_RGBA8 }, SIZE, SIZE);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i as u32, GL_RENDERBUFFER, rb);
            }
            let bufs = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];
            glDrawBuffers(4, bufs.as_ptr());
            let build_err = glGetError();
            Fbo { fb, rbs, status: glCheckFramebufferStatus(GL_FRAMEBUFFER), build_err }
        }
    }
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names made by `new`.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(4, self.rbs.as_ptr());
            glFinish();
        }
    }
}

/// The PBE STATE word pair of one emit, decoded (pbestate.xml STATE_WORD1).
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

/// What one frame of a four-target draw handed the hardware.
#[derive(Debug)]
struct Frame {
    /// Every PBE STATE pair the one end-of-tile program was built from,
    /// `PVR_RENDER_MAX_RT` of them (unused ones zero).
    emits: Vec<Emit>,
    /// The kick's `num_rt`.
    num_rt: u32,
}

/// Clears and draws over the bound framebuffer with the program of `class`,
/// then submits.
fn drawn(class: Class) -> Result<Frame, GLenum> {
    EMITS.lock().unwrap().clear();
    KICKS.lock().unwrap().clear();
    let prog = CLASSES.iter().position(|c| *c == class).unwrap();
    // SAFETY: plain GL calls; POS is a static.
    unsafe {
        glViewport(0, 0, SIZE, SIZE);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glUseProgram(PROGRAMS[prog]);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        let e = glGetError();
        if e != GL_NO_ERROR {
            glFlush();
            return Err(e);
        }
        glFlush();
    }
    let emits = EMITS.lock().unwrap().clone();
    let kicks = KICKS.lock().unwrap().clone();
    assert_eq!(emits.len(), 1, "one end-of-tile program: {:?}", logs());
    assert_eq!(kicks.len(), 1, "one kick: {:?}", logs());
    Ok(Frame { emits: emits[0].chunks(2).map(|p| decode(p[1])).collect(), num_rt: kicks[0].0 })
}

/// The class of the fourth output a format needs.
fn class_of(internal: GLenum) -> Class {
    match internal {
        GL_R8I | GL_R16I | GL_R32I | GL_RG8I | GL_RG16I | GL_RGBA8I => Class::Int,
        GL_R8UI | GL_R16UI | GL_R32UI | GL_RG8UI | GL_RG16UI | GL_RGBA8UI | GL_RGB10_A2UI => Class::Uint,
        _ => Class::Float,
    }
}

/// `glClearBufferfv/iv/uiv` of attachment 3 and a read of it in the format and
/// type a read of `internal` is valid with; the errors of the two (the host
/// draws no pixels, so the value is not looked at).
fn clear_and_read(internal: GLenum) -> (GLenum, GLenum) {
    let class = class_of(internal);
    let (fmt, ty) = match class {
        Class::Int => (GL_RGBA_INTEGER, GL_INT),
        Class::Uint => (GL_RGBA_INTEGER, GL_UNSIGNED_INT),
        Class::Float if matches!(internal, GL_R32F | GL_RG16F) => (GL_RGBA, GL_FLOAT),
        Class::Float => (GL_RGBA, GL_UNSIGNED_BYTE),
    };
    let mut px = [0u32; 4];
    // SAFETY: plain GL calls; `px` holds 16 bytes, one pixel of any of them.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        match class {
            Class::Float => glClearBufferfv(GL_COLOR, 3, [0.25f32, 0.5, 0.75, 1.0].as_ptr()),
            Class::Int => glClearBufferiv(GL_COLOR, 3, [1i32, 2, 3, 4].as_ptr()),
            Class::Uint => glClearBufferuiv(GL_COLOR, 3, [1u32, 2, 3, 4].as_ptr()),
        }
        let clear_err = glGetError();
        glReadBuffer(GL_COLOR_ATTACHMENT3);
        glReadPixels(0, 0, 1, 1, fmt, ty, px.as_mut_ptr() as *mut c_void);
        (clear_err, glGetError())
    }
}

/* ---- what the back end does with attachment 3, per format ---------------------- */

/// What a four-target draw (RGBA8 on 0-2, the format on 3) comes to today.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// Complete and drawn, three PBE states (attachment 3 has none).
    Draws,
    /// Complete; the back end refuses the draw by name: an integer
    /// attachment beside another one (`GL_INVALID_OPERATION`).
    RefusedInteger,
    /// Complete; refused by name: an sRGB attachment beside another format.
    RefusedSrgb,
    /// The renderbuffer storage is refused (`GL_INVALID_ENUM`), so the
    /// framebuffer is incomplete.
    NotRenderable,
}

use Outcome::*;

/// (GL name, label, the surface the GL core hands over for attachment 3 (a
/// renderbuffer), today's outcome, whether the surface is ONE 32-bit output
/// register (`surf_out_dwords` in src/pvr/backend.rs: sRGB and RGBA16F take
/// two, an RGBA8-class or packed integer one)).
const ROWS: [(GLenum, &str, u8, Outcome, bool); 25] = [
    (GL_RGBA8, "RGBA8", GLES_SURF_RGBA8888, Draws, true),
    (GL_SRGB8_ALPHA8, "SRGB8_ALPHA8", GLES_SURF_SRGB8_A8, RefusedSrgb, false),
    /* the 10-bit class is drawn as RGBA8 on the PowerVR today
     * (docs/es3/backend-caps.md section 3) */
    (GL_RGB10_A2, "RGB10_A2", GLES_SURF_RGBA8888, Draws, true),
    (GL_RGB10_A2UI, "RGB10_A2UI", GLES_SURF_RGB10_A2UI, RefusedInteger, true),
    (GL_R8, "R8", GLES_SURF_RGBA8888, Draws, true),
    (GL_RG8, "RG8", GLES_SURF_RGBA8888, Draws, true),
    (GL_RGB8, "RGB8", GLES_SURF_RGBA8888, Draws, true),
    (GL_R8I, "R8I", GLES_SURF_R8I, RefusedInteger, true),
    (GL_R8UI, "R8UI", GLES_SURF_R8UI, RefusedInteger, true),
    (GL_R16I, "R16I", GLES_SURF_R16I, RefusedInteger, true),
    (GL_R16UI, "R16UI", GLES_SURF_R16UI, RefusedInteger, true),
    (GL_R32I, "R32I", GLES_SURF_R32I, RefusedInteger, true),
    (GL_R32UI, "R32UI", GLES_SURF_R32UI, RefusedInteger, true),
    (GL_RG8I, "RG8I", GLES_SURF_RG8I, RefusedInteger, true),
    (GL_RG8UI, "RG8UI", GLES_SURF_RG8UI, RefusedInteger, true),
    (GL_RG16I, "RG16I", GLES_SURF_RG16I, RefusedInteger, true),
    (GL_RG16UI, "RG16UI", GLES_SURF_RG16UI, RefusedInteger, true),
    (GL_RGBA8I, "RGBA8I", GLES_SURF_RGBA8I, RefusedInteger, true),
    (GL_RGBA8UI, "RGBA8UI", GLES_SURF_RGBA8UI, RefusedInteger, true),
    (GL_R32F, "R32F", GLES_SURF_R32F, Draws, true),
    /* not colour-renderable in ES 3.0 without GL_EXT_color_buffer_float */
    (GL_R11F_G11F_B10F, "R11F_G11F_B10F", GLES_SURF_RGBA8888, NotRenderable, true),
    /* RG16F is kept as four halves (the GL core hands RGBA16F over): two
     * registers, so it does NOT fit beside three RGBA8 targets in four
     * (pvr-budget4-test's); drawn as three targets today like the rest */
    (GL_RG16F, "RG16F", GLES_SURF_RGBA16F, Draws, false),
    (GL_RGB565, "RGB565", GLES_SURF_RGBA8888, Draws, true),
    (GL_RGBA4, "RGBA4", GLES_SURF_RGBA8888, Draws, true),
    (GL_RGB5_A1, "RGB5_A1", GLES_SURF_RGBA8888, Draws, true),
];

/// The PBE pair every RGBA8 target of the file is handed, on register `reg`.
fn rgba8_emit(reg: u32) -> Emit {
    Emit { packmode: PACKMODE_U8U8U8U8, norm: 1, source_format: SOURCE_8_PER_CHANNEL, source_pos: reg, mrt_index: reg }
}

/* PBE packmodes and source formats (csbgen/rogue/pbestate.xml) */
const PACKMODE_U8U8U8U8: u32 = 0;
const PACKMODE_F32: u32 = 54;
const SOURCE_F16_PER_CHANNEL: u32 = 0;
const SOURCE_8_PER_CHANNEL: u32 = 1;

fn int_class(internal: GLenum) -> bool {
    class_of(internal) != Class::Float
}

/* ---- the tests ----------------------------------------------------------------- */

/// The stand-in gives the GL core four draw buffers, and glDrawBuffers takes
/// all four attachments.
#[test]
fn four_draw_buffers_are_offered() {
    let _g = rig();
    let (mut db, mut ca) = (0, 0);
    // SAFETY: plain GL calls.
    unsafe {
        glGetIntegerv(GL_MAX_DRAW_BUFFERS, &mut db);
        glGetIntegerv(GL_MAX_COLOR_ATTACHMENTS, &mut ca);
    }
    assert_eq!((db, ca), (4, 4));
    let f = Fbo::new(GL_RGBA8);
    assert_eq!((f.status, f.build_err), (GL_FRAMEBUFFER_COMPLETE, GL_NO_ERROR));
}

/// Completeness and the surface the GL core hands over for attachment 3, for
/// each format: every name but GL_R11F_G11F_B10F is a complete framebuffer
/// beside RGBA8 on 0-2.
#[test]
fn completeness_and_surface_of_attachment_3() {
    let _g = rig();
    for (internal, name, surf, outcome, _) in ROWS {
        let f = Fbo::new(internal);
        if outcome == NotRenderable {
            assert_eq!(f.build_err, GL_INVALID_ENUM, "{name}: storage");
            assert_eq!(f.status, GL_FRAMEBUFFER_INCOMPLETE_ATTACHMENT, "{name}: status");
            continue;
        }
        assert_eq!(f.build_err, GL_NO_ERROR, "{name}: building");
        assert_eq!(f.status, GL_FRAMEBUFFER_COMPLETE, "{name}: status");
        assert_eq!(fb_target_format_n(ctx(), 3), surf, "{name}: surface of attachment 3");
        for a in 0..3 {
            assert_eq!(fb_target_format_n(ctx(), a), GLES_SURF_RGBA8888, "{name}: attachment {a}");
        }
        /* one 32-bit register, except sRGB and RGBA16F's two (the half_float
         * and sRGB rules of the back end) */
        let bpp = backend::gles_surf_bytes_per_pixel(surf);
        assert!(bpp <= 4 || !ROWS.iter().any(|r| r.0 == internal && r.4), "{name}: {bpp} bytes a pixel");
    }
}

/// A four-target draw today: the formats the back end draws reach the
/// hardware as THREE targets. Attachment 3 has no PBE state pair (the render
/// layer's `PVR_RENDER_MAX_RT` is 3 and `num_targets` is clamped to it), and
/// the three RGBA8 targets are on registers 0-2 as RGBA8.
#[test]
fn three_pbe_states_reach_the_hardware_today() {
    let _g = rig();
    assert_eq!(PVR_RENDER_EOT_MAX_RT, 4);
    for (internal, name, _, outcome, _) in ROWS {
        if outcome != Draws {
            continue;
        }
        let f = Fbo::new(internal);
        assert_eq!(f.status, GL_FRAMEBUFFER_COMPLETE, "{name}");
        let frame = drawn(class_of(internal)).unwrap_or_else(|e| panic!("{name}: draw error {e:#x}: {:?}", logs()));
        assert_eq!(frame.num_rt, 3, "{name}: kick num_rt");
        assert_eq!(frame.emits, [rgba8_emit(0), rgba8_emit(1), rgba8_emit(2)], "{name}: PBE states");
    }
}

/// The back end refuses, by name, an integer attachment 3 beside RGBA8
/// targets, and an sRGB one: `GL_INVALID_OPERATION`, no kick.
#[test]
fn integer_and_srgb_attachment_3_are_refused_by_name_today() {
    let _g = rig();
    for (internal, name, _, outcome, _) in ROWS {
        let want = match outcome {
            RefusedInteger => "an integer colour attachment beside another colour attachment",
            RefusedSrgb => "an SRGB8_ALPHA8 colour attachment beside one of another format",
            _ => continue,
        };
        let f = Fbo::new(internal);
        assert_eq!(f.status, GL_FRAMEBUFFER_COMPLETE, "{name}");
        assert_eq!(drawn(class_of(internal)).err(), Some(GL_INVALID_OPERATION), "{name}");
        assert!(logs().iter().any(|l| l.contains(want)), "{name}: {want}: {:?}", logs());
    }
}

/// Whether a read of attachment 3 beside RGBA8 targets is refused today: the
/// integer formats and sRGB are (`GL_INVALID_OPERATION`, the format's own
/// read pair), the rest read.
fn read_is_refused(internal: GLenum) -> bool {
    int_class(internal) || internal == GL_SRGB8_ALPHA8
}

/// `glClearBuffer{f,i,ui}v` of attachment 3 raises no error for any complete
/// format (the type must match the format's class), and a `glReadPixels` of
/// it, in the read pair valid for the format, reads for every format but the
/// integer ones and sRGB (see `read_of_integer_and_srgb_attachment_3`). The
/// host fills no pixels, so the values are not looked at.
#[test]
fn clear_buffer_and_read_of_attachment_3() {
    let _g = rig();
    for (internal, name, _, outcome, _) in ROWS {
        if outcome == NotRenderable {
            continue;
        }
        let _f = Fbo::new(internal);
        let want = (GL_NO_ERROR, if read_is_refused(internal) { GL_INVALID_OPERATION } else { GL_NO_ERROR });
        assert_eq!(clear_and_read(internal), want, "{name}: clear, read");
        /* the other clear family for the class is a type mismatch */
        let wrong = if int_class(internal) { 0 } else { 1 };
        // SAFETY: plain GL calls.
        let e = unsafe {
            while glGetError() != GL_NO_ERROR {}
            if wrong == 1 {
                glClearBufferiv(GL_COLOR, 3, [0i32; 4].as_ptr());
            } else {
                glClearBufferfv(GL_COLOR, 3, [0f32; 4].as_ptr());
            }
            glGetError()
        };
        assert_eq!(e, GL_INVALID_OPERATION, "{name}: the clear of the wrong type");
    }
}

/* ---- defects: what a four-target render should give ----------------------------- */

/// Attachment 3 of a drawn four-target frame has a PBE state pair: on output
/// register 3 (`source_pos` 3), `mrt_index` 3, packed as the format needs. The
/// formats that take one register (RGBA8-class, R32F).
#[test]
#[ignore = "pvr-render/pvr-backend (plan #368): num_targets is clamped to PVR_RENDER_MAX_RT 3; attachment 3 gets no PBE state"]
fn attachment_3_gets_its_pbe_state_on_register_3() {
    let _g = rig();
    for (internal, name, surf, outcome, one_reg) in ROWS {
        if outcome != Draws || !one_reg {
            continue;
        }
        let _f = Fbo::new(internal);
        let frame = drawn(class_of(internal)).unwrap_or_else(|e| panic!("{name}: draw error {e:#x}"));
        assert_eq!(frame.num_rt, 4, "{name}: kick num_rt");
        assert_eq!(frame.emits.len(), 4, "{name}: four PBE states");
        let want = if surf == GLES_SURF_R32F {
            Emit { packmode: PACKMODE_F32, norm: 0, source_format: SOURCE_F16_PER_CHANNEL, source_pos: 3, mrt_index: 3 }
        } else {
            rgba8_emit(3)
        };
        assert_eq!(frame.emits[3], want, "{name}: PBE state of attachment 3");
        assert_eq!(&frame.emits[..3], [rgba8_emit(0), rgba8_emit(1), rgba8_emit(2)], "{name}: 0-2 unchanged");
    }
}

/// An integer attachment 3 beside RGBA8 targets draws: one packed dword on
/// register 3, not normalised, not sRGB-style F16.
#[test]
#[ignore = "pvr-backend (plan #368; docs/notes/pvr-int-mixed-targets.md): an integer attachment beside another colour attachment is refused (GL_INVALID_OPERATION)"]
fn integer_attachment_3_beside_rgba8_draws() {
    let _g = rig();
    for (internal, name, _, outcome, _) in ROWS {
        if outcome != RefusedInteger {
            continue;
        }
        let _f = Fbo::new(internal);
        let frame = drawn(class_of(internal)).unwrap_or_else(|e| panic!("{name}: draw error {e:#x}"));
        assert_eq!(frame.emits.len(), 4, "{name}: four PBE states");
        let e = frame.emits[3];
        assert_eq!((e.source_pos, e.mrt_index, e.norm), (3, 3, 0), "{name}: PBE state of attachment 3");
    }
}

/// An sRGB attachment 3 beside RGBA8 targets draws (the PBE encodes it on
/// store, `gamma` of REG_WORD0).
#[test]
#[ignore = "pvr-backend (plan #368): an SRGB8_ALPHA8 attachment beside one of another format is refused (srgb_targets_as_rgba8 serves all-sRGB draws only); as two output registers it would not fit register 3 either (pvr-budget4-test)"]
fn srgb_attachment_3_beside_rgba8_draws() {
    let _g = rig();
    let _f = Fbo::new(GL_SRGB8_ALPHA8);
    let frame = drawn(Class::Float).unwrap_or_else(|e| panic!("draw error {e:#x}"));
    assert_eq!(frame.emits.len(), 4);
}

/// A read of attachment 3 in the pair valid for its format (integer targets:
/// RGBA_INTEGER with INT / UNSIGNED_INT; sRGB: RGBA / UNSIGNED_BYTE) is
/// accepted, as the read of an RGBA8 one is.
#[test]
#[ignore = "gl-readback (plan #368; pvr-read4-test): glReadPixels of an integer or sRGB attachment 3 beside RGBA8 targets raises GL_INVALID_OPERATION"]
fn read_of_integer_and_srgb_attachment_3() {
    let _g = rig();
    for (internal, name, _, outcome, _) in ROWS {
        if outcome == NotRenderable || !read_is_refused(internal) {
            continue;
        }
        let _f = Fbo::new(internal);
        assert_eq!(clear_and_read(internal), (GL_NO_ERROR, GL_NO_ERROR), "{name}: clear, read");
    }
}
