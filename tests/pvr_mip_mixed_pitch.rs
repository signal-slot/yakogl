// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Three mip levels a render on the PowerVR backend
//! (`pvr_backend_set_mip_three_levels`, docs/notes/pvr-mip-three-levels.md)
//! at awkward sizes: the note's second Open question, whether a render
//! whose emits have different pitches and maxclips stores each level as it
//! should. tests/pvr_mip_three_levels.rs pins the store words of terrain's
//! 800x600 and 256x256 chains; this file drives the same knob-on chain on
//! non-power-of-two sizes, odd widths, one-texel-wide and one-texel-high
//! chains and levels whose storage pitch is padded past their width.
//!
//! For every level a three-levels render stores (slot 0, 1 or 2):
//!
//! - its PBE register words (REG_WORD0's linestride, REG_WORD1's maxclip)
//!   are the knob-off level kick's for that level, and those of
//!   `pbe_reg_words` at the level's GL extent and its storage pitch by the
//!   chain layout rule (`one_wide_level_pitch` of the power-of-two extent's
//!   level side, src/pvr/backend.rs `mip_chain_layout`);
//! - its emit words (PBESTATE_STATE_WORD0/1, the level's address) are the
//!   level kick's, but for the attachment's own source register and
//!   mrt_index (`pbe_emit_words(_, _, slot, slot)`);
//! - slots 1 and 2 never carry slot 0's pitch where their level's differs.
//!
//! The platform and the generator stand-ins are tests/pvr_mip_three_levels.rs's
//! (the REAL backend, host memory, every submit completes at once), with the
//! end-of-tile program's emit words recorded per kick.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test pvr_mip_mixed_pitch
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

use yakogl::backend::GLES_SURF_RGBA8888;
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, one_wide_level_pitch, pot_level_side, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::texture::*;
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

/// One submitted kick, as far as the mip chain's stores are concerned.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct KickRec {
    size: (u32, u32),
    num_rt: u32,
    pbe: [[u64; 2]; PVR_RENDER_EOT_MAX_RT],
    /// The end-of-tile program's emit words (PBESTATE_STATE_WORD0/1 per
    /// attachment, its address) the frame handed its pixel event program.
    emit: [[u32; 2]; PVR_RENDER_EOT_MAX_RT],
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    /// The allocations' host memory, by GPU address (u64 backing: every
    /// u32 view of it is aligned).
    allocs: BTreeMap<u64, Box<[u64]>>,
    /// Submits so far: the last fence.
    fence: i64,
    /// The emit words of the last pixel event program generated: the frame
    /// being closed's, handed to the kick it submits.
    emit: [[u32; 2]; PVR_RENDER_EOT_MAX_RT],
    /// What each kick submitted stores, in order.
    kicks: Vec<KickRec>,
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
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = heap_base + 0x1000 + p.bump[h];
        p.bump[h] += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the caller passes a writable descriptor.
        unsafe { *out = OsalGpuMem { cpu, gpu, phys: gpu, size, impl_: cpu } };
        p.allocs.insert(gpu, mem);
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

/// The "GPU": records what the kick stores and completes it at once.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.fence += 1;
        let emit = core::mem::take(&mut p.emit);
        p.kicks.push(KickRec { size: (k.width, k.height), num_rt: k.frag.num_rt, pbe: k.frag.pbe_word, emit });
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
    _doutu_src0: u64,
    emit_words: *const u32,
    num_rt: u32,
    _has_slc_mcu_cache_controls: i32,
    data_out: *mut u32,
    code_out: *mut u32,
    data_size_dw: *mut u32,
    code_size_dw: *mut u32,
) -> i32 {
    let mut emit = [[0u32; 2]; PVR_RENDER_EOT_MAX_RT];
    for (i, e) in emit.iter_mut().enumerate().take((num_rt as usize).min(PVR_RENDER_EOT_MAX_RT)) {
        // SAFETY: the backend hands 2 * num_rt emit words.
        *e = unsafe { [*emit_words.add(2 * i), *emit_words.add(2 * i + 1)] };
    }
    with_platform(|p| p.emit = emit);
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

/// The window: glmark2's canvas.
const CANVAS: (i32, i32) = (800, 600);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();
static mut PROGRAM: GLuint = 0;

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
            PROGRAM = program("passthrough.vert", "uniform_color.frag");
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

/* ---- the chain ---------------------------------------------------------------- */

/// A `size` RGBA8 texture (LINEAR_MIPMAP_LINEAR, CLAMP_TO_EDGE) in an FBO.
struct Target {
    tex: GLuint,
    fbo: GLuint,
    size: (i32, i32),
}

fn target(size: (i32, i32)) -> Target {
    let (mut tex, mut fbo) = (0, 0);
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
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    Target { tex, fbo, size }
}

/// Level 0 of `t` cleared and drawn over by a full-screen triangle: the
/// frame left open for the chain's first kick to submit.
fn render_level0(t: &Target) {
    // SAFETY: plain GL calls on names this test made; the array is a static.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, t.fbo);
        glViewport(0, 0, t.size.0, t.size.1);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glUseProgram(PROGRAM);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 3);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/// `f` with three levels a render on, off again after it (also when `f`
/// panics): the switch is the backend's, the rig's lock keeps every other
/// test of this file out meanwhile.
fn with_three_levels<R>(f: impl FnOnce() -> R) -> R {
    struct Off;
    impl Drop for Off {
        fn drop(&mut self) {
            pvr_backend_set_mip_three_levels(false);
        }
    }
    let _off = Off;
    pvr_backend_set_mip_three_levels(true);
    f()
}

/// GL's side of level 1: every kick of a chain declares it
/// (src/pvr/backend.rs `mip_chain_gpu`, one render target set a chain).
fn level1(size: (i32, i32)) -> (u32, u32) {
    ((size.0 as u32 / 2).max(1), (size.1 as u32 / 2).max(1))
}

/// The kicks of one `glGenerateMipmap` on `t` after its level 0 was
/// rendered again, knob on (`three`) or off: the chain's kicks, each
/// declaring level 1's size (the render of level 0 is the kick before them).
/// Both arms run on the same texture, so the level addresses are comparable.
fn chain_kicks(t: &Target, three: bool) -> Vec<KickRec> {
    render_level0(t);
    let from = with_platform(|p| p.kicks.len());
    let run = || {
        // SAFETY: plain GL calls on a name this test made.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, t.tex);
            glGenerateMipmap(GL_TEXTURE_2D);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindTexture(GL_TEXTURE_2D, 0);
            glFlush();
        }
    };
    if three {
        with_three_levels(run);
    } else {
        run();
    }
    let l1 = level1(t.size);
    with_platform(|p| p.kicks[from..].iter().filter(|k| k.size == l1).copied().collect())
}

/// The levels each render of a chain of `n` levels (level 0 included)
/// draws with the knob on: level 1 alone, then up to three from the level
/// above them (src/pvr/backend.rs `mip_chain_gpu`).
fn groups(n: usize) -> Vec<Vec<usize>> {
    let mut g = vec![vec![1]];
    let mut l = 2;
    while l < n {
        let m = (n - l).min(3);
        g.push((l..l + m).collect());
        l += m;
    }
    g
}

/// The levels of a `w` x `h` chain, level 0 included.
fn chain_levels(w: u32, h: u32) -> usize {
    (32 - w.max(h).leading_zeros()) as usize
}

/// Level `l`'s GL extent and storage pitch: the chain is laid out as the
/// chain of its power-of-two extent, a level max(pw >> l, 1) texels a row,
/// pair-padded, a 1-wide level above a wider base one texel a row
/// (src/pvr/backend.rs `mip_chain_layout`, gl/context.rs
/// `one_wide_level_pitch`).
fn level(w: u32, h: u32, l: u32) -> (u32, u32, u32) {
    let pw = w.next_power_of_two();
    (pot_level_side(w, l), pot_level_side(h, l), one_wide_level_pitch(pw, pot_level_side(pw, l), true))
}

/// The PBE register words of an RGBA8 level `lw` x `lh` stored at `pitch`
/// (alpha stored, linear rows, one sample).
fn reg_words(lw: u32, lh: u32, pitch: u32) -> [u64; 2] {
    pbe_reg_words(GLES_SURF_RGBA8888, false, lw, lh, pitch, false, false)
}

/// The colour attachments a kick stores (`num_rt` 0 is one).
fn rts(k: &KickRec) -> usize {
    k.num_rt.max(1) as usize
}

/// What one size's chain exercised: (levels a three-levels render stored in
/// slot 1 or 2, of them those whose storage pitch is padded past their width,
/// those whose REG_WORD0 linestride differs from slot 0's).
#[derive(Debug, Default, PartialEq, Eq)]
struct Covered {
    secondary: usize,
    padded: usize,
    other_stride: usize,
}

/// The chain of a `size` texture, knob off and on: every level each
/// three-levels render stores goes out with the knob-off level kick's PBE
/// register words and emit words (bar the attachment's own source
/// register and mrt_index), which are the layout rule's.
fn check(size: (i32, i32)) -> Covered {
    let _g = rig();
    let (w, h) = (size.0 as u32, size.1 as u32);
    let n = chain_levels(w, h);
    let t = target(size);
    let off = chain_kicks(&t, false);
    assert_eq!(off.len(), n - 1, "{size:?}: a kick a level with the knob off; {:?}", logs());
    for (i, k) in off.iter().enumerate() {
        let l = i as u32 + 1;
        let (lw, lh, pitch) = level(w, h, l);
        assert_eq!(rts(k), 1, "{size:?}: level {l}'s kick stores one attachment");
        assert_eq!(
            k.pbe[0],
            reg_words(lw, lh, pitch),
            "{size:?}: level {l} ({lw}x{lh} at pitch {pitch}): the knob-off kick's register words"
        );
    }
    let on = chain_kicks(&t, true);
    let g = groups(n);
    assert_eq!(on.len(), g.len(), "{size:?}: renders with the knob on ({g:?}); {:?}", logs());
    assert!(g.iter().any(|r| r.len() > 1), "{size:?}: no render of two or three levels");
    let zero = pbe_emit_words(0, GLES_SURF_RGBA8888, 0, 0);
    let mut cov = Covered::default();
    for (r, (kick, levels)) in on.iter().zip(&g).enumerate() {
        assert_eq!(rts(kick), levels.len(), "{size:?}: render {r}'s attachments are levels {levels:?}");
        let (_, _, pitch0) = level(w, h, levels[0] as u32);
        for (slot, &l) in levels.iter().enumerate() {
            let what = format!("{size:?}: level {l}, attachment {slot} of render {r}");
            let (lw, lh, pitch) = level(w, h, l as u32);
            let lk = &off[l - 1];
            assert_eq!(kick.pbe[slot], lk.pbe[0], "{what} ({lw}x{lh} at pitch {pitch}): not the level kick's words");
            /* the same address and packing as the level kick, from this
             * attachment's output register, as this attachment's state */
            let s = slot as u32;
            let delta = pbe_emit_words(0, GLES_SURF_RGBA8888, s, s);
            let want = [lk.emit[0][0] ^ zero[0] ^ delta[0], lk.emit[0][1] ^ zero[1] ^ delta[1]];
            assert_eq!(kick.emit[slot], want, "{what}: not the level kick's address");
            if slot == 0 {
                continue;
            }
            cov.secondary += 1;
            cov.padded += usize::from(pitch > lw);
            /* slot 0's pitch on a smaller level: its rows would land a
             * level's row apart */
            if (pitch - 1) / 2 != (pitch0 - 1) / 2 {
                cov.other_stride += 1;
                assert_ne!(
                    kick.pbe[slot][0],
                    reg_words(lw, lh, pitch0)[0],
                    "{what}: stored at slot 0's pitch {pitch0}, not its own {pitch}"
                );
            }
        }
        /* each attachment its own level: the addresses, every state put
         * into slot 0's form, differ */
        let addr: Vec<[u32; 2]> = (0..levels.len())
            .map(|k| {
                let d = pbe_emit_words(0, GLES_SURF_RGBA8888, k as u32, k as u32);
                [kick.emit[k][0] ^ d[0] ^ zero[0], kick.emit[k][1] ^ d[1] ^ zero[1]]
            })
            .collect();
        for i in 0..addr.len() {
            for j in 0..i {
                assert_ne!(addr[i], addr[j], "{size:?}: render {r} stores attachments {j} and {i} at one address");
            }
        }
    }
    // SAFETY: names this test made.
    unsafe {
        glDeleteFramebuffers(1, &t.fbo);
        glDeleteTextures(1, &t.tex);
    }
    cov
}

/// Not a power of two either way, both sides odd at level 0: level 1 is
/// 150x86 in a 256-texel row, levels 2..=8 odd and even by turns
/// (75x43 at 128, 37x21 at 64, 18x10, 9x5, 4x2, 2x1 at 2, 1x1 at 2).
#[test]
fn npot_300x173() {
    let c = check((300, 173));
    assert_eq!(c, Covered { secondary: 4, padded: 4, other_stride: 4 });
}

/// One texel wide: every level 1x(64 >> l) in a pair-padded 2-texel row,
/// so slots 1 and 2 have slot 0's pitch (rightly: it is theirs too) and
/// only their maxclip differs.
#[test]
fn one_wide_1x64() {
    let c = check((1, 64));
    assert_eq!(c, Covered { secondary: 3, padded: 3, other_stride: 0 });
}

/// Odd width just past a power of two, two rows, ten levels: level 1 is
/// 256x1 in a 512-texel row (the power-of-two extent's), every level from
/// 1 on one row high, and the last render stores 2x1 at 4 and 1x1 at 2.
#[test]
fn odd_513x2() {
    let c = check((513, 2));
    assert_eq!(c, Covered { secondary: 5, padded: 5, other_stride: 5 });
}

/// One texel high, eleven levels, no padding: the last render stores 4x1
/// at pitch 4 in slot 0, 2x1 at pitch 2 and 1x1 at pitch 1 (a 1-wide level
/// above a wider base is one texel a row) in slots 1 and 2.
#[test]
fn one_high_1024x1() {
    let c = check((1024, 1));
    assert_eq!(c, Covered { secondary: 6, padded: 0, other_stride: 6 });
}

/// Odd and small: 8x4, then 4x2, 2x1 and 1x1 in the 32-wide extent's
/// 16-, 8-, 4- and 2-texel rows, one render of three levels.
#[test]
fn small_17x9() {
    let c = check((17, 9));
    assert_eq!(c, Covered { secondary: 2, padded: 2, other_stride: 2 });
}

/// Wide and short, NPOT: 500x12 in a 512 row, then 250x6 at 256, ..., the
/// height reaching 1 at level 4 while the width goes on to level 9 (1x1
/// at 2).
#[test]
fn wide_1000x24() {
    let c = check((1000, 24));
    assert_eq!(c, Covered { secondary: 5, padded: 5, other_stride: 5 });
}
