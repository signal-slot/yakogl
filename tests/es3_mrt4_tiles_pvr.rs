// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Four colour targets across PowerVR tile boundaries (tiles4-pvr of plan
//! #368, the PowerVR counterpart of tests/es3_mrt4_tiles_v3d.rs), with the
//! draw-target count at 4 (`pvr_debug_set_draw_targets`, restored to the
//! build's default by a drop guard on every path, a panic included).
//!
//! The host has no PowerVR rasteriser: the platform here is the one of
//! tests/es3_mrt4_pvr.rs (host memory, the C command generators stood in
//! for, the "GPU" keeping each kick and drawing nothing), so no pixel is
//! read back; glReadPixels of an attachment would return what the texture
//! or renderbuffer held, not what the PBE stores. What the host can pin
//! across tile edges is what the back end hands the hardware for four
//! RGBA8 attachments of sizes that are not a multiple of the ISP tile (16)
//! or the ZLS tile (32), and of sizes of several tiles each way:
//!
//! - the FBO is complete and the draw is one kick whose size is the
//!   frame's, with `num_rt` 4;
//! - each render target's PBE REG_WORD1 clip (`maxclip_x/y`) is its own
//!   size minus one, so the last partial tile is stored to the last pixel
//!   and not past it (the words are `pbe_reg_words`' of that size);
//! - the end-of-tile emits name four distinct surfaces, mrt index 0..3 at
//!   registers 0..3, for a draw, for glClear and for glClearBufferfv of
//!   draw buffer 3;
//! - the fragment programs the draw ran include the clear object writing
//!   register 3 for glClearBufferfv(3) at a size across tiles.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_mrt4_tiles_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", feature = "test-knobs"))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,test-knobs --test es3_mrt4_pvr
//! ```

#![cfg(all(feature = "host-test", feature = "gpu-powervr", feature = "shaderc-online", feature = "test-knobs"))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_void, CStr};
use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, Once};

use yakogl::backend::GLES_SURF_RGBA8888;
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::es3::*;
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
use yakogl::pvr::internal::{PVR_INTERNAL_CLEAR_ATTACH1, PVR_INTERNAL_FBLOAD, PVR_INTERNAL_TABLE};

/* ---- the platform ------------------------------------------------------------ */

const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

struct Alloc {
    /// u64 backing: every u32 view of it is aligned.
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    bump: [u64; 3],
    allocs: BTreeMap<u64, Alloc>,
    fence: i64,
    /// Every kick submitted, in order.
    kicks: Vec<PvrRenderKick>,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The dwords at GPU address `gpu`, `n` of them, from the allocation that
/// holds them.
fn gpu_words(gpu: u64, n: usize) -> Option<Vec<u32>> {
    with_platform(|p| {
        let (&base, a) = p.allocs.range(..=gpu).next_back()?;
        let off = (gpu - base) as usize;
        if off + 4 * n > a.size {
            return None;
        }
        // SAFETY: inside the allocation (checked above); u64 backing, so the
        // u32 view is aligned for any 4-byte offset.
        let w = unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off) as *const u32, n) };
        Some(w.to_vec())
    })
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

/// The "GPU" draws nothing: the kick is kept and completes at once.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<PvrRenderKick>() {
        return -22;
    }
    // SAFETY: a PvrRenderKick (the length says so).
    let k = unsafe { *(kick as *const PvrRenderKick) };
    with_platform(|p| {
        p.fence += 1;
        p.kicks.push(k);
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

/// The library logs every line as `log("%s", line)` (`osal::log_str`).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

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
 * same stand-ins as tests/es3_mrt4_clear_pvr.rs, two of them recording. */

/// Every end-of-tile data generation: the pairs asked for and the emit words.
static EOT: Mutex<Vec<(u32, Vec<u32>)>> = Mutex::new(Vec::new());
/// The USC-heap offset of every fragment program a kick-USC PDS program was
/// built for, in order.
static KICK_FS: Mutex<Vec<u64>> = Mutex::new(Vec::new());

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
    KICK_FS.lock().unwrap().push(fs_usc_exec_offset);
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

/// The generator's real data layout is the driver's (pinned by
/// tests/pvr_render_rt4.rs and tests/pvr_eot_words_pin.rs); this one keeps
/// what the back end asked for: `num_rt` pairs of emit words.
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
    // SAFETY: the backend's emit words (2 * num_rt), buffers and sizes.
    unsafe {
        let emit = core::slice::from_raw_parts(emit_words, 2 * num_rt as usize).to_vec();
        EOT.lock().unwrap().push((num_rt, emit));
        core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
        core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
        *data_size_dw = 4;
        *code_size_dw = 2;
    }
    0
}

/* ---- the context, the stand-in and the framebuffers --------------------------- */

const FS4: &str = "#version 300 es\nprecision mediump float;\n\
    layout(location = 0) out vec4 o0;\nlayout(location = 1) out vec4 o1;\n\
    layout(location = 2) out vec4 o2;\nlayout(location = 3) out vec4 o3;\n\
    void main() { o0 = vec4(1.0, 0.0, 0.0, 1.0); o1 = vec4(0.0, 1.0, 0.0, 1.0);\n\
    o2 = vec4(0.0, 0.0, 1.0, 1.0); o3 = vec4(1.0, 1.0, 0.0, 1.0); }\n";
const VS: &str = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = p; }\n";

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
/* ---- the programs the objects ran ----------------------------------------------- */

fn table_words(i: usize) -> &'static [u32] {
    let sh = &PVR_INTERNAL_TABLE[i].shader;
    // SAFETY: a row of the generated table points at its own words.
    unsafe { core::slice::from_raw_parts(sh.usc, sh.usc_dw as usize) }
}

/// `words` against `base`: Some(k) when they are `base` with every byte that
/// differs moved up by the same `k` (the back end moves a program's pixel
/// output writes by rewriting their destination bytes), 0 when equal.
fn moved_by(words: &[u32], base: &[u32]) -> Option<u8> {
    if words.len() != base.len() {
        return None;
    }
    let bytes = |w: &[u32]| w.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>();
    let (a, b) = (bytes(words), bytes(base));
    let mut k = None;
    for (x, y) in a.iter().zip(&b) {
        if x != y {
            let d = x.wrapping_sub(*y);
            if *k.get_or_insert(d) != d {
                return None;
            }
        }
    }
    Some(k.unwrap_or(0))
}

/// What the object of fragment program `off` is: a frame load writing from
/// register r, or a one-dword clear writing register r.
#[derive(Debug, PartialEq, Eq)]
enum Object {
    Load(u8),
    Clear(u8),
    Other,
}

fn object(off: u64) -> Object {
    let load = table_words(PVR_INTERNAL_FBLOAD);
    let clear = &PVR_INTERNAL_CLEAR_ATTACH1[..];
    let gpu = HEAP_USC_BASE + off;
    if let Some(r) = gpu_words(gpu, load.len()).and_then(|w| moved_by(&w, load)) {
        return Object::Load(r);
    }
    if let Some(r) = gpu_words(gpu, clear.len()).and_then(|w| moved_by(&w, clear)) {
        return Object::Clear(r);
    }
    Object::Other
}

const SIZE: i32 = 16;
const ATT: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

/// Sizes that are not a multiple of the ISP tile (16) or the ZLS tile (32),
/// then sizes of several tiles each way.
const PARTIAL: [(i32, i32); 3] = [(37, 21), (70, 50), (33, 65)];
const MANY: [(i32, i32); 2] = [(64, 64), (96, 80)];

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// Holds the file's lock with the count at 4 and puts the build's default
/// back when dropped, on a panic too (`pvr_debug_set_draw_targets(0)`).
struct Rig {
    _g: MutexGuard<'static, ()>,
}

impl Drop for Rig {
    fn drop(&mut self) {
        yakogl::pvr::debug::pvr_debug_set_draw_targets(0);
    }
}

/// The one context of the file, four draw targets set and the back end
/// brought up for them (a change of count restarts it, as a board does).
fn rig() -> Rig {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let r = Rig { _g: g };
    yakogl::pvr::debug::pvr_debug_set_draw_targets(4);
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_pvr;
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(SIZE as u32, SIZE as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(0);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    if pvr_backend_eot_pairs() != eot_pds_pairs(eot_four_rt()) {
        pvr_backend_shutdown();
        assert_eq!(pvr_backend_init(), 0, "{:?}", logs());
    }
    pvr_backend_set_option(PVR_BACKEND_OPT_PARTIAL_RENDER, 0);
    with_platform(|p| {
        p.kicks.clear();
        p.log.clear();
    });
    r
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn error() -> GLenum {
    // SAFETY: a plain GL call on the current context.
    unsafe { glGetError() }
}

/// The FBO and renderbuffers of the last `fbo`, deleted by the next one so
/// the host heaps are not run out by a loop over sizes.
static LIVE: Mutex<Vec<(GLuint, [GLuint; 4])>> = Mutex::new(Vec::new());

/// An FBO of four RGBA8 renderbuffers of `w` x `h`, all four draw buffers.
fn fbo(w: i32, h: i32) {
    for (f, rbs) in LIVE.lock().unwrap().drain(..) {
        // SAFETY: names this file made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &f);
            glDeleteRenderbuffers(4, rbs.as_ptr());
        }
    }
    let mut fbo = 0;
    let mut names = [0; 4];
    // SAFETY: plain GL calls; room for the names.
    unsafe {
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        for (i, a) in ATT.into_iter().enumerate() {
            let mut rb = 0;
            glGenRenderbuffers(1, &mut rb);
            names[i] = rb;
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, w, h);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, a, GL_RENDERBUFFER, rb);
        }
        glDrawBuffers(4, ATT.as_ptr());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{w}x{h}: {:?}", logs());
        glViewport(0, 0, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    LIVE.lock().unwrap().push((fbo, names));
}

/// What one piece of GL work handed the hardware.
struct Frame {
    kicks: Vec<PvrRenderKick>,
    /// Every end-of-tile data generation: pairs asked for, emit words.
    eot: Vec<(u32, Vec<u32>)>,
    /// The fragment programs of the recorded objects (USC-heap offsets).
    fs: Vec<u64>,
}

fn frame_of(f: impl FnOnce()) -> Frame {
    // SAFETY: plain GL calls.
    unsafe { glFinish() };
    with_platform(|p| p.kicks.clear());
    EOT.lock().unwrap().clear();
    KICK_FS.lock().unwrap().clear();
    f();
    // SAFETY: plain GL calls.
    unsafe { glFinish() };
    Frame {
        kicks: with_platform(|p| p.kicks.clone()),
        eot: EOT.lock().unwrap().clone(),
        fs: KICK_FS.lock().unwrap().clone(),
    }
}

/// The surface address of a PBE emit (pbestate.xml STATE_WORD0/1).
fn emit_address(w: &[u32]) -> u64 {
    (u64::from(w[1] & 0xff) << 34) | (u64::from(w[0]) << 2)
}

/// The (source_pos, mrt_index) of emit `i`: STATE_WORD1 bits 14-15, 16-18.
fn source_and_index(words: &[u32], i: usize) -> (u32, u32) {
    let w1 = words[2 * i + 1];
    ((w1 >> 14) & 3, (w1 >> 16) & 7)
}

/// What a four-RGBA8 frame of `w` x `h` must hand the hardware.
fn check_frame(f: &Frame, w: i32, h: i32, what: &str) {
    assert_eq!(f.kicks.len(), 1, "{what} {w}x{h}: one kick, {:?}", logs());
    let k = &f.kicks[0];
    assert_eq!((k.width, k.height), (w as u32, h as u32), "{what} {w}x{h}: the frame's size");
    assert_eq!(k.frag.num_rt, 4, "{what} {w}x{h}");
    /* the clip of each target is its own size: the last partial tile stores
     * to the last pixel and not past it (REG_WORD1 maxclip_x/y) */
    let want = pbe_reg_words(GLES_SURF_RGBA8888, false, w as u32, h as u32, 0, false, false)[1];
    for rt in 0..4 {
        assert_eq!(k.frag.pbe_word[rt][1], want, "{what} {w}x{h}: render target {rt}'s clip");
    }
    for rt in 1..4 {
        let pitch = (w as u32 + 1) & !1;
        let w0 = pbe_reg_words(GLES_SURF_RGBA8888, false, w as u32, h as u32, pitch, false, false)[0];
        assert_eq!(k.frag.pbe_word[rt][0], w0, "{what} {w}x{h}: render target {rt}'s row pitch");
    }
    assert_eq!(f.eot.len(), 1, "{what} {w}x{h}");
    let (pairs, words) = &f.eot[0];
    assert_eq!(*pairs, 4, "{what} {w}x{h}");
    let addrs: Vec<u64> = (0..4).map(|i| emit_address(&words[2 * i..2 * i + 2])).collect();
    for (i, a) in addrs.iter().enumerate() {
        assert!(gpu_words(*a, 1).is_some(), "{what} {w}x{h}: render target {i}'s address {a:#x} is allocated");
        for (j, b) in addrs.iter().enumerate().take(i) {
            assert_ne!(a, b, "{what} {w}x{h}: render targets {j} and {i} share a surface");
        }
        assert_eq!(source_and_index(words, i), (i as u32, i as u32), "{what} {w}x{h}: emit {i}");
    }
}

fn sizes() -> impl Iterator<Item = (i32, i32)> {
    PARTIAL.into_iter().chain(MANY)
}

#[test]
fn a_four_target_draw_across_tiles() {
    let _r = rig();
    for (w, h) in sizes() {
        fbo(w, h);
        use_program(FS4);
        let f = frame_of(|| {
            draw_triangle();
            assert_eq!(error(), GL_NO_ERROR, "{w}x{h}: {:?}", logs());
        });
        check_frame(&f, w, h, "draw");
    }
}

#[test]
fn b_a_clear_across_tiles() {
    let _r = rig();
    for (w, h) in sizes() {
        fbo(w, h);
        let f = frame_of(|| {
            // SAFETY: plain GL calls.
            unsafe {
                glClearColor(1.0, 0.5, 0.25, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
            }
            assert_eq!(error(), GL_NO_ERROR, "{w}x{h}: {:?}", logs());
        });
        check_frame(&f, w, h, "clear");
    }
}

/// A distinct colour for each attachment, one glClearBufferfv each: each
/// is its own kick whose clear object writes that attachment's register.
#[test]
fn c_distinct_clears_reach_their_own_register() {
    let _r = rig();
    let colours: [[f32; 4]; 4] =
        [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 0.0, 1.0]];
    for (w, h) in sizes() {
        fbo(w, h);
        for (k, c) in colours.iter().enumerate() {
            let f = frame_of(|| {
                // SAFETY: four floats.
                unsafe { glClearBufferfv(GL_COLOR, k as GLint, c.as_ptr()) };
                assert_eq!(error(), GL_NO_ERROR, "{w}x{h}: {:?}", logs());
            });
            check_frame(&f, w, h, &format!("clear of draw buffer {k}"));
            let objects: Vec<Object> = f.fs.iter().map(|&o| object(o)).collect();
            let clears: Vec<&Object> = objects.iter().filter(|o| matches!(o, Object::Clear(_))).collect();
            let want = if k == 0 { Object::Clear(0) } else { Object::Clear(k as u8) };
            assert!(clears.contains(&&want), "{w}x{h} buffer {k}: a clear object writing register {k}: {objects:?}");
        }
    }
}

/// A clear, then a draw writing four colours, on the same frame: two kicks,
/// each with the four targets' clip.
#[test]
fn d_clear_then_draw_in_one_fbo() {
    let _r = rig();
    for (w, h) in sizes() {
        fbo(w, h);
        use_program(FS4);
        let f = frame_of(|| {
            // SAFETY: plain GL calls.
            unsafe {
                glClearColor(0.25, 0.25, 0.25, 1.0);
                glClear(GL_COLOR_BUFFER_BIT);
            }
            draw_triangle();
            assert_eq!(error(), GL_NO_ERROR, "{w}x{h}: {:?}", logs());
        });
        assert!(!f.kicks.is_empty() && f.kicks.len() <= 2, "{w}x{h}: {:?}", logs());
        for k in &f.kicks {
            assert_eq!((k.width, k.height, k.frag.num_rt), (w as u32, h as u32, 4), "{w}x{h}");
        }
    }
}

/// The guard puts the default back on a panic: after one the count is the
/// build's again (read under the file's lock, which the unwind released).
#[test]
fn e_the_guard_restores_the_default_after_a_panic() {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _r = Rig { _g: g };
        /* a count that is not the build's (PowerVR answers 4 by default) */
        yakogl::pvr::debug::pvr_debug_set_draw_targets(2);
        assert_eq!(yakogl::backend::draw_targets(), 2);
        panic!("a test failing with the count raised");
    }));
    assert!(r.is_err());
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    assert_eq!(yakogl::backend::draw_targets(), yakogl::backend::DRAW_TARGETS, "the default is back");
}
