// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Several colour targets of a 4x framebuffer object on the PowerVR
//! (BXS-4-64, AM62P), through the GL API, on the host (plan #339, item
//! pvr-msaa-mrt-tests).
//!
//! docs/notes/es3-msaa-renderbuffers.md (c1). Same rig as
//! tests/es3_msaa_pvr.rs (copied): the REAL PowerVR back end
//! (`yakogl_backend_pvr`, its draw and `attachment_gone` wrapped to record
//! what the GL core hands it) on a host platform whose "GPU" paints memory.
//! The platform paints, per 4x kick, one image over each end-of-tile emit's
//! surface, so every colour target of a kick resolves to its own image.
//!
//! Today `pvr_backend_draw` (src/pvr/backend.rs, "Several colour
//! attachments at 4x") refuses a 4x draw with more than one colour target
//! with `ENOTSUP`, by name, never drawing with lost samples. Two tests pin
//! that refusal for 2 and 4 targets. The tests of the expected result (a
//! `glClearBufferfv` of each target, a draw writing a constant per output,
//! each attachment resolved in turn through `glReadBuffer` into a 1x
//! framebuffer, bytes checked) are `#[ignore]`d, owner src/pvr (plan #325);
//! they run with `--ignored` and fail today at the refused draw.
//!
//! HOST-ONLY: the sample values are the stand-in's own.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr,shaderc-online,msaa-fbo --test es3_msaa_pvr_mrt
//! ```

#![cfg(all(
    feature = "host-test",
    feature = "shaderc-online",
    feature = "gpu-powervr",
    not(feature = "report-es2"),
    feature = "msaa-fbo",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#![allow(clippy::too_many_arguments)]
/* helpers copied from tests/es3_msaa_pvr.rs that this file does not call */
#![allow(dead_code)]

use core::ffi::{c_char, c_void};
use std::ffi::CString;
#[cfg(not(feature = "msaa-fbo"))]
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once};
#[cfg(not(feature = "msaa-fbo"))]
use yakogl::backend::{self, GlesHeap, YakoglBackend};
use yakogl::backend::{GlesDraw, *};
use yakogl::gl::buffer::glBindBuffer;
use yakogl::gl::consts::*;
use yakogl::gl::draw::*;
use yakogl::gl::es3::glClearBufferfv;
use yakogl::gl::framebuffer::*;
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
use yakogl::gl::types::*;
use yakogl::gl::vertex::{glEnableVertexAttribArray, glVertexAttribPointer};
#[cfg(feature = "msaa-fbo")]
use yakogl::gl::{context, names};
#[cfg(not(feature = "msaa-fbo"))]
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps};

/* ---- without msaa-fbo: the smallest OSAL and stand-in back end ----------- */

#[cfg(not(feature = "msaa-fbo"))]
unsafe extern "C" fn m_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    let mem = Box::leak(vec![0u8; size].into_boxed_slice());
    let p = mem.as_mut_ptr();
    *out = OsalGpuMem { cpu: p as *mut c_void, gpu: p as u64, phys: p as u64, size, impl_: p as *mut c_void };
    0
}
#[cfg(not(feature = "msaa-fbo"))]
unsafe extern "C" fn m_gpu_free(_mem: *mut OsalGpuMem) -> i32 {
    0
}
#[cfg(not(feature = "msaa-fbo"))]
unsafe extern "C" fn m_cache(_cpu: *mut c_void, _n: usize) {}
#[cfg(not(feature = "msaa-fbo"))]
unsafe extern "C" fn b_init() -> i32 {
    0
}
#[cfg(not(feature = "msaa-fbo"))]
unsafe extern "C" fn b_heap_base(_heap: GlesHeap) -> u64 {
    0x1000_0000
}
#[cfg(not(feature = "msaa-fbo"))]
unsafe extern "C" fn b_ok() -> i32 {
    0
}

/// What the back end was handed by one draw.
#[derive(Clone, Debug)]
struct Seen {
    /// An application draw (vertices), not a clear alone.
    draws: bool,
    /// `targets[..num_targets]`: (gpu, format).
    targets: Vec<(u64, u8)>,
    /// `GlesDraw::samples`.
    samples: u8,
    /// `GlesDraw::depth_id`, 0 without a depth attachment.
    depth_id: u32,
}

static SEEN: Mutex<Vec<Seen>> = Mutex::new(Vec::new());
/// The ids `attachment_gone` was told, in order.
static GONE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// The back end's draw: recorded, then, with `msaa-fbo`, the PowerVR's.
unsafe extern "C" fn b_draw(d: *const GlesDraw) -> i32 {
    // SAFETY: the GL core passes a live draw.
    let d = unsafe { &*d };
    let n = (d.num_targets.max(1) as usize).min(GLES_MAX_TARGETS);
    SEEN.lock().unwrap().push(Seen {
        draws: d.index_count != 0,
        targets: d.targets[..n].iter().map(|t| (t.gpu, t.format)).collect(),
        samples: d.samples,
        depth_id: d.depth_id,
    });
    #[cfg(feature = "msaa-fbo")]
    return yakogl::pvr::backend::pvr_backend_draw(d);
    #[cfg(not(feature = "msaa-fbo"))]
    0
}

unsafe extern "C" fn b_gone(id: u32) {
    GONE.lock().unwrap().push(id);
    #[cfg(feature = "msaa-fbo")]
    yakogl::pvr::backend::pvr_backend_attachment_gone(id);
}

#[cfg(not(feature = "msaa-fbo"))]
static OSAL: OsalOps = OsalOps {
    init: None,
    shutdown: None,
    gpu_alloc: Some(m_gpu_alloc),
    gpu_free: Some(m_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(m_cache),
    cache_invd: Some(m_cache),
    submit: None,
    wait: None,
    completed: None,
    reg_read: None,
    reg_write: None,
    query: None,
    log: None,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

#[cfg(not(feature = "msaa-fbo"))]
static BACKEND: YakoglBackend = YakoglBackend {
    name: c"es3-msaa-pvr".as_ptr() as *const c_char,
    native_attrib_formats: 0,
    rt_rgba8888_is_bgra: false,
    mip_level_align: 0,
    compressed_blocks_in_rows: false,
    native_layered_textures: false,
    front_facing_shader_variant: false,
    etc1_decoded_at_upload: false,
    init: Some(b_init),
    shutdown: None,
    draw: Some(b_draw),
    flush: Some(b_ok),
    finish: Some(b_ok),
    finish_previous: None,
    kick_id: None,
    kick_pending: None,
    heap_base: Some(b_heap_base),
    counters: None,
    texture_released: None,
    surface_swapped: None,
    texture_uploaded: None,
    max_instance_divisor: 1,
    wait_kick: None,
    samples_texel_copies: false,
    attachment_gone: Some(b_gone),
    depth_resolve: None,
    query_begin: None,
    query_end: None,
    query_result: None,
    generate_mipmap: None,
    return_mipmap_chain: None,
    linear_row_align: 0,
    uniforms_host_only: false,
    ds_write: None,
    stencil_resolve: None,
    max_samples: 0,
};

/* ---- with msaa-fbo: the REAL PowerVR back end on a host platform ----------- */

/// The platform and the C command generators of tests/pvr_msaa_rb.rs
/// (copied), with a "GPU" that writes what this file's draws store: a 4x
/// kick writes [`rig::Platform::paint`] over the surface of its first emit
/// (the PBE's `downscale` store resolves into the renderbuffer there) and,
/// when it stores depth, the depth of the [`EDGE`] quad into the 4x plane.
#[cfg(feature = "msaa-fbo")]
mod rig {
    use core::ffi::{c_char, c_void, CStr};
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use yakogl::osal::render::{PvrRenderKick, PvrRenderStatic};
    use yakogl::osal::{OsalGpuMem, OsalOps, OSAL_QUERY_RENDER_STATIC};
    use yakogl::pvr::ffi::*;

    /* The PowerVR's heap layout (src/pvr/backend.rs, HEAP_*_BASE): init
     * refuses a platform that answers another. */
    const HEAP_GENERAL_BASE: u64 = 0x80_0000_0000;
    const HEAP_PDS_BASE: u64 = 0xDA_0000_0000;
    const HEAP_USC_BASE: u64 = 0xE0_0000_0000;
    const HEAPS: [u64; 3] = [HEAP_GENERAL_BASE, HEAP_PDS_BASE, HEAP_USC_BASE];

    /// ISP_ZLSCTL.forcezstore: the kick stores its depth/stencil.
    const ZLSCTL_FORCEZSTORE: u64 = 1 << 2;

    /// The ZLS tile (`ZLS_TILE` in src/pvr/backend.rs): a depth/stencil
    /// buffer's rows are its width aligned to it, doubled at 4x.
    const ZLS_TILE: u32 = 32;

    /// The D24 word the "GPU" stores for a sample the [`super::EDGE`] quad
    /// covers (depth 0.25) and for one it leaves at the clear (1.0).
    pub const DEPTH_COVERED: u32 = 0x40_0000;
    pub const DEPTH_CLEAR: u32 = 0xff_ffff;

    struct Alloc {
        /// u64 backing: every u32 view of it is aligned.
        mem: Box<[u64]>,
        size: usize,
    }

    #[derive(Default)]
    pub struct Platform {
        bump: [u64; 3],
        allocs: BTreeMap<u64, Alloc>,
        /// Submits so far: the last fence.
        fence: i64,
        pub log: Vec<String>,
        /// Every kick submitted, in order.
        pub kicks: Vec<PvrRenderKick>,
        /// The PBESTATE_STATE_WORD0/1 pairs of each end-of-tile program built
        /// (`pvr_pds_generate_pixel_event_tq`), in order: the kick submitted
        /// next is the one they belong to.
        pub emits: Vec<Vec<[u32; 2]>>,
        /// The packed RGBA8 pixels (row-major from the bottom left, rows at
        /// the target's even width) a 4x kick resolves into its first emit's
        /// surface; none, nothing written.
        pub paint: Option<Vec<u32>>,
        /// The same for emit `k` of a 4x kick's end-of-tile program, one
        /// image per colour target; an emit without one is left alone.
        pub paints: Vec<Vec<u32>>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    pub fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    /// The surface address of a PBESTATE_STATE_WORD0/1 pair (pbestate.h:
    /// address_low is address bits 33..2 in word 0, address_high bits 39..34
    /// in word 1's bits 5..0).
    pub fn emit_address(w: [u32; 2]) -> u64 {
        (w[0] as u64) << 2 | ((w[1] & 0x3f) as u64) << 34
    }

    /// The kicks submitted so far with four samples, with the emit pairs of
    /// the end-of-tile program built for each.
    pub fn four_sample_kicks() -> Vec<(PvrRenderKick, Vec<[u32; 2]>)> {
        with_platform(|p| {
            assert_eq!(p.kicks.len(), p.emits.len(), "one end-of-tile program a kick");
            p.kicks.iter().zip(&p.emits).filter(|(k, _)| k.samples == 4).map(|(k, e)| (*k, e.clone())).collect()
        })
    }

    pub fn logs() -> Vec<String> {
        with_platform(|p| p.log.clone())
    }

    /// The words from `gpu` to the end of its allocation.
    fn words_at(p: &mut Platform, gpu: u64) -> Option<&mut [u32]> {
        let (&base, a) = p.allocs.range_mut(..=gpu).next_back()?;
        let off = (gpu - base) as usize;
        if off >= a.size {
            return None;
        }
        let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
        // SAFETY: the words from `gpu` to the allocation's end.
        Some(unsafe { core::slice::from_raw_parts_mut(first, (a.size - off) / 4) })
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

    /// The "GPU". A kick that stores depth leaves, in its depth buffer (rows
    /// the ZLS-aligned width apart, twice that and twice the rows at 4x),
    /// [`DEPTH_CLEAR`] everywhere at one sample; at 4x, [`DEPTH_COVERED`] in
    /// the samples the [`super::EDGE`] quad covers: every sample of pixel
    /// columns 0..=2 and the left sample column (even X of the 2-by-2 scaled
    /// image) of pixel column 3. A 4x kick writes `paint` over its first
    /// emit's surface. Every kick completes at once.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<PvrRenderKick>() {
            return -22;
        }
        // SAFETY: a PvrRenderKick (the length says so).
        let k = unsafe { *(kick as *const PvrRenderKick) };
        with_platform(|p| {
            p.fence += 1;
            p.kicks.push(k);
            let ms = k.samples == 4;
            let scale = if ms { 2 } else { 1 };
            let row = (k.width.div_ceil(ZLS_TILE) * ZLS_TILE * scale) as usize;
            let zs = k.frag.isp_zload_store_base;
            if zs != 0 && k.frag.isp_zlsctl & ZLSCTL_FORCEZSTORE != 0 {
                if let Some(words) = words_at(p, zs) {
                    for (i, w) in words.iter_mut().enumerate() {
                        let x = (i % row) as u32;
                        let covered = ms && (x / 2 < 3 || (x / 2 == 3 && x.is_multiple_of(2)));
                        *w = if covered { DEPTH_COVERED } else { DEPTH_CLEAR };
                    }
                }
            }
            let emits = p.emits.last().cloned().unwrap_or_default();
            for (n, &w) in emits.iter().enumerate() {
                let target = emit_address(w);
                let img = p.paints.get(n).cloned();
                if let (true, Some(img), true) = (ms, img, target != 0) {
                    if let Some(words) = words_at(p, target) {
                        let n = img.len().min(words.len());
                        words[..n].copy_from_slice(&img[..n]);
                    }
                }
            }
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

    /// The library logs every line as `log("%s", line)` (`osal::log_str`), so
    /// a two-pointer function takes it; on x86-64 and AArch64 Linux a
    /// variadic call passes those two arguments as a plain one does.
    unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
        // SAFETY: the library passes a NUL-terminated line.
        let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
        with_platform(|p| p.log.push(s));
    }

    type LogFn = unsafe extern "C" fn(*const c_char, ...);

    /// The platform's table (made once, at run time: the log hook is a cast).
    pub fn osal() -> &'static OsalOps {
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

    /// Install the platform and the PowerVR's table with this file's draw
    /// and `attachment_gone` (each records, then calls the PowerVR's).
    ///
    /// # Safety
    /// Once, with the file's lock held: it swaps the process's back end.
    pub unsafe fn install() {
        yakogl::osal::osal_set_ops(osal());
        // SAFETY: a table of entry points and plain values; the copy is
        // leaked, so it outlives every call through it.
        let mut t = unsafe { core::ptr::read(&yakogl::pvr::backend::yakogl_backend_pvr) };
        assert_eq!(t.max_samples, 4, "msaa-fbo turns the PowerVR's 4x framebuffer objects on");
        t.draw = Some(super::b_draw);
        t.attachment_gone = Some(super::b_gone);
        // SAFETY: the caller holds the lock; the table is leaked.
        unsafe { yakogl::backend::g_backend = Box::leak(Box::new(t)) };
        assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
    }

    /* ---- the PowerVR's command generators (driver/src/pvr, C), stood in for */

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
        unsafe {
            core::ptr::copy_nonoverlapping([0xB000_0000, ps.pds_data_offset, ps.pds_code_offset].as_ptr(), dw, 3)
        };
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
        let w =
            [0xB100_0000, v.vs_pds_data_offset, v.vs_pds_code_offset, v.vs_output_size_bytes, v.vs_input_size_bytes];
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
                let w =
                    [s.address as u32, (s.address >> 32) as u32, s.stride, s.dma_size, s.dst_reg, s.per_instance, 0, 0];
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
            *info =
                PvrPdsRenderColorUploadInfo { data_size_dw: 3 * num_kicks, code_size_dw: num_kicks + 1, temps_used: 1 };
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
        // SAFETY: the backend passes `num_rt` state pairs.
        let pairs = (0..num_rt as usize).map(|i| unsafe { [*emit_words.add(2 * i), *emit_words.add(2 * i + 1)] });
        let pairs: Vec<[u32; 2]> = pairs.collect();
        with_platform(|p| p.emits.push(pairs));
        // SAFETY: the backend's emit words, buffers and sizes.
        unsafe {
            core::ptr::copy_nonoverlapping([*emit_words, *emit_words.add(1), 0, 0].as_ptr(), data_out, 4);
            core::ptr::copy_nonoverlapping([0x0E0E_0000, 0].as_ptr(), code_out, 2);
            *data_size_dw = 4;
            *code_size_dw = 2;
        }
        0
    }
}

/// One context for the whole file; the tests take turns on it.
static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();

fn context() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    #[cfg(not(feature = "msaa-fbo"))]
    backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
    ONCE.call_once(|| {
        // SAFETY: the tables are statics or leaked; the lock above
        // serialises the tests.
        unsafe {
            #[cfg(not(feature = "msaa-fbo"))]
            {
                osal_set_ops(&OSAL);
                backend::g_backend = &BACKEND;
            }
            #[cfg(feature = "msaa-fbo")]
            rig::install();
            yakogl::shaderc::g_shaderc = &yakogl::shaderc::online::SHC_ONLINE;
            assert!(!yakogl::gl::context::gl_context_create(8, 8).is_null());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDisable(GL_DEPTH_TEST);
        glFinish();
    }
    SEEN.lock().unwrap().clear();
    GONE.lock().unwrap().clear();
    #[cfg(feature = "msaa-fbo")]
    rig::with_platform(|p| {
        p.log.clear();
        p.kicks.clear();
        p.emits.clear();
        p.paint = None;
        p.paints.clear();
    });
    // SAFETY: plain GL call.
    while unsafe { glGetError() } != GL_NO_ERROR {}
    g
}

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/* ---- framebuffers, the program, the draw ------------------------------- */

struct Fbo {
    fb: GLuint,
    rbs: Vec<GLuint>,
}

impl Drop for Fbo {
    fn drop(&mut self) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &self.fb);
            glDeleteRenderbuffers(self.rbs.len() as GLsizei, self.rbs.as_ptr());
            glUseProgram(0);
        }
    }
}

/// The error of `glRenderbufferStorageMultisample(.., samples, format, 8, 8)`
/// on a fresh renderbuffer, and the renderbuffer.
fn storage(samples: GLsizei, format: GLenum) -> (GLenum, GLuint) {
    let mut rb = 0;
    // SAFETY: plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        while glGetError() != GL_NO_ERROR {}
        glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, 8, 8);
    }
    (err(), rb)
}

/// A bound 8x8 framebuffer with one RGBA8 colour renderbuffer of `samples`
/// and, with `depth`, a DEPTH_COMPONENT24 one of the same count. A storage
/// error comes back as `Err(error)`.
fn framebuffer(samples: GLsizei, depth: bool) -> Result<Fbo, GLenum> {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    let (e, rb) = storage(samples, GL_RGBA8);
    fbo.rbs.push(rb);
    if e != GL_NO_ERROR {
        return Err(e);
    }
    // SAFETY: a renderbuffer this test made.
    unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb) };
    if depth {
        let (e, rb) = storage(samples, GL_DEPTH_COMPONENT24);
        fbo.rbs.push(rb);
        if e != GL_NO_ERROR {
            return Err(e);
        }
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb) };
    }
    let buf = GL_COLOR_ATTACHMENT0;
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(1, &buf);
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(
            glCheckFramebufferStatus(GL_FRAMEBUFFER),
            GL_FRAMEBUFFER_COMPLETE,
            "{samples} samples, depth {depth}"
        );
    }
    Ok(fbo)
}

fn shader(ty: GLenum, text: &str) -> GLuint {
    let text = CString::new(text).unwrap();
    // SAFETY: plain GL calls; one NUL-terminated source.
    unsafe {
        let s = glCreateShader(ty);
        let p: [*const c_char; 1] = [text.as_ptr()];
        glShaderSource(s, 1, p.as_ptr(), core::ptr::null());
        glCompileShader(s);
        let mut ok: GLint = 0;
        glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
        assert_eq!(ok, 1, "{text:?} did not compile");
        s
    }
}

/// The current program: one float output of `colour`, vertices at depth `z`
/// (clip space; 0.5 is window depth 0.75).
fn program(colour: [f32; 4], z: f32) {
    let [r, g, b, a] = colour;
    let fs = format!(
        "#version 300 es\nprecision highp float;\nlayout(location = 0) out vec4 o0;\nvoid main() {{ o0 = vec4({r:?}, {g:?}, {b:?}, {a:?}); }}\n"
    );
    let vs = format!("#version 300 es\nin vec4 p;\nvoid main() {{ gl_Position = vec4(p.xy, {z:?}, 1.0); }}\n");
    let v = shader(GL_VERTEX_SHADER, &vs);
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// The 8x8 target's whole area: one oversized triangle.
static FULL: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
/// Left of x = -0.125: three whole columns of 8 (0..=2) and the left half of
/// column 3, so a 4x pixel there has two of its four samples covered.
static EDGE: [f32; 24] = [
    -1.0, -1.0, 0.0, 1.0, -0.125, -1.0, 0.0, 1.0, -0.125, 1.0, 0.0, 1.0, -1.0, -1.0, 0.0, 1.0, -0.125, 1.0, 0.0, 1.0,
    -1.0, 1.0, 0.0, 1.0,
];

/// Draw `verts` (`n` vertices of 4 floats) and hand back what the back end was
/// given for it.
fn draw(verts: &'static [f32], n: GLsizei) -> Seen {
    SEEN.lock().unwrap().clear();
    // SAFETY: `verts` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, verts.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, n);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR);
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end")
}

fn clear(colour: [f32; 4]) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearColor(colour[0], colour[1], colour[2], colour[3]);
        glClearDepthf(1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    assert_eq!(err(), GL_NO_ERROR);
}

/// Resolve blit of `read` (colour, and depth when `depth`) into a fresh
/// single-sample framebuffer of the same size; the GL error, and the
/// framebuffer (left bound for reading) when it worked.
fn resolve_blit(read: &Fbo, depth: bool) -> (GLenum, Fbo) {
    let dst = framebuffer(0, depth).expect("a single-sample target");
    let mask = GL_COLOR_BUFFER_BIT | if depth { GL_DEPTH_BUFFER_BIT } else { 0 };
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, mask, GL_NEAREST);
    }
    (err(), dst)
}

/// The 8x8 RGBA8 pixels of `fbo`'s colour attachment, row-major from the
/// bottom left.
fn pixels(fbo: &Fbo) -> Vec<[u8; 4]> {
    let mut v = vec![[0u8; 4]; 64];
    // SAFETY: room for 64 RGBA8 pixels; a framebuffer this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fbo.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0);
        glReadPixels(0, 0, 8, 8, GL_RGBA, GL_UNSIGNED_BYTE, v.as_mut_ptr() as *mut c_void);
    }
    assert_eq!(err(), GL_NO_ERROR);
    v
}

fn near(a: [u8; 4], b: [u8; 4], tol: i32) -> bool {
    a.iter().zip(b).all(|(x, y)| (*x as i32 - y as i32).abs() <= tol)
}

fn geti(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

fn rb_samples(rb: GLuint) -> GLint {
    let mut v = -1;
    // SAFETY: one value; a renderbuffer this test made.
    unsafe {
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glGetRenderbufferParameteriv(GL_RENDERBUFFER, GL_RENDERBUFFER_SAMPLES, &mut v);
    }
    v
}

const COLOUR: [f32; 4] = [0.25, 0.5, 0.75, 1.0];
/// COLOUR as bytes (round to nearest).
const COLOUR8: [u8; 4] = [64, 128, 191, 255];

/// The packed RGBA8 word of `p` as the PBE stores it (R in the low byte).
#[cfg(feature = "msaa-fbo")]
fn packed(p: [u8; 4]) -> u32 {
    u32::from_le_bytes(p)
}

/// The GPU address of renderbuffer `rb`'s (resolved, single-sample) storage.
#[cfg(feature = "msaa-fbo")]
fn rb_gpu(rb: GLuint) -> u64 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, names::Kind::Renderbuffer, rb).expect("a renderbuffer name");
    c.renderbuffer_object(slot).expect("a renderbuffer").mem.gpu
}

/// The id the back end keeps renderbuffer `rb`'s depth under.
#[cfg(feature = "msaa-fbo")]
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, names::Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

/// The resolved image of the [`EDGE`] draw of `COLOUR` over a 0 clear, as the
/// PBE's `downscale` store leaves it (8x8, row-major from the bottom left):
/// columns 0..=2 the colour, column 3 two of four samples (half of every
/// channel), the rest the clear.
#[cfg(feature = "msaa-fbo")]
fn edge_image() -> Vec<u32> {
    (0..64)
        .map(|i| match i % 8 {
            0..=2 => packed(COLOUR8),
            3 => packed(EDGE8),
            _ => 0,
        })
        .collect()
}

/// Two of four samples of COLOUR over a 0 clear.
#[cfg(feature = "msaa-fbo")]
const EDGE8: [u8; 4] = [32, 64, 96, 128];

/* ---- several colour targets at 4x ----------------------------------------- */

/// One colour a target is cleared to and the shader writes to it.
const CLEARS: [[f32; 4]; 4] = [[0.0, 0.0, 0.0, 1.0], [1.0, 1.0, 0.0, 1.0], [0.0, 1.0, 1.0, 1.0], [1.0, 0.0, 1.0, 1.0]];
const WRITES: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 1.0], [0.0, 1.0, 0.0, 1.0], [0.0, 0.0, 1.0, 1.0], [1.0, 1.0, 1.0, 0.0]];

fn bytes(c: [f32; 4]) -> [u8; 4] {
    c.map(|v| (v * 255.0 + 0.5) as u8)
}

/// A bound 8x8 framebuffer of `samples` with `n` RGBA8 colour renderbuffers
/// on attachments 0..n and every one of them a draw buffer.
fn mrt_framebuffer(samples: GLsizei, n: usize) -> Fbo {
    let mut fb = 0;
    // SAFETY: room for one name; plain GL call.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
    }
    let mut fbo = Fbo { fb, rbs: Vec::new() };
    let mut bufs = Vec::new();
    for k in 0..n {
        let (e, rb) = storage(samples, GL_RGBA8);
        fbo.rbs.push(rb);
        assert_eq!(e, GL_NO_ERROR, "{samples} samples, target {k}");
        let at = GL_COLOR_ATTACHMENT0 + k as GLenum;
        // SAFETY: a renderbuffer this test made.
        unsafe { glFramebufferRenderbuffer(GL_FRAMEBUFFER, at, GL_RENDERBUFFER, rb) };
        bufs.push(at);
    }
    // SAFETY: plain GL calls.
    unsafe {
        glDrawBuffers(n as GLsizei, bufs.as_ptr());
        assert_eq!(err(), GL_NO_ERROR);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{samples} samples, {n} targets");
    }
    fbo
}

/// The current program: output `k` of `colours` is a constant, `colours.len()`
/// outputs.
fn mrt_program(colours: &[[f32; 4]]) {
    let mut fs = String::from("#version 300 es\nprecision highp float;\n");
    for k in 0..colours.len() {
        fs += &format!("layout(location = {k}) out vec4 o{k};\n");
    }
    fs += "void main() {\n";
    for (k, [r, g, b, a]) in colours.iter().enumerate() {
        fs += &format!("  o{k} = vec4({r:?}, {g:?}, {b:?}, {a:?});\n");
    }
    fs += "}\n";
    let vs = "#version 300 es\nin vec4 p;\nvoid main() { gl_Position = vec4(p.xy, 0.0, 1.0); }\n";
    let v = shader(GL_VERTEX_SHADER, vs);
    let f = shader(GL_FRAGMENT_SHADER, &fs);
    // SAFETY: plain GL calls.
    unsafe {
        let p = glCreateProgram();
        glAttachShader(p, v);
        glAttachShader(p, f);
        glLinkProgram(p);
        let mut ok: GLint = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1);
        glUseProgram(p);
    }
}

/// Clear target `k` of the bound framebuffer to `colour` (`glClearBufferfv`).
fn clear_target(k: usize, colour: [f32; 4]) {
    // SAFETY: four floats.
    unsafe { glClearBufferfv(GL_COLOR, k as GLint, colour.as_ptr()) };
}

/// Resolve colour attachment `k` of `read` into a fresh 1x framebuffer
/// (`glReadBuffer(k)`), and its pixels.
fn resolve_target(read: &Fbo, k: usize) -> (GLenum, Vec<[u8; 4]>) {
    let dst = framebuffer(0, false).expect("a single-sample target");
    // SAFETY: plain GL calls on names this test made.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, read.fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fb);
        glReadBuffer(GL_COLOR_ATTACHMENT0 + k as GLenum);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 8, 8, 0, 0, 8, 8, GL_COLOR_BUFFER_BIT, GL_NEAREST);
    }
    let e = err();
    (e, pixels(&dst))
}

/// A 4x framebuffer of `n` targets, every one cleared, then one draw of
/// [`EDGE`] writing [`WRITES`]; what the back end was handed and the GL error
/// of the draw (a refusal is the back end's, so the GL errors the clears and
/// the draw raised, in order, come back instead of being asserted).
fn clear_and_draw(n: usize) -> (Fbo, Seen, Vec<GLenum>) {
    let fbo = mrt_framebuffer(4, n);
    let mut errors = Vec::new();
    for (k, c) in CLEARS.iter().enumerate().take(n) {
        clear_target(k, *c);
        errors.push(err());
    }
    mrt_program(&WRITES[..n]);
    SEEN.lock().unwrap().clear();
    // SAFETY: `EDGE` is static and outlives the draw.
    unsafe {
        glBindBuffer(GL_ARRAY_BUFFER, 0);
        glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, EDGE.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
        glFinish();
    }
    errors.push(err());
    let s = core::mem::take(&mut *SEEN.lock().unwrap());
    let s = s.into_iter().rev().find(|s| s.draws).expect("the draw reached the back end");
    (fbo, s, errors.into_iter().filter(|&e| e != GL_NO_ERROR).collect())
}

/// The image the PBE's `downscale` store leaves of the [`EDGE`] draw of
/// `write` over `clear`: columns 0..=2 the written colour, column 3 two of
/// four samples (half the written colour and half the clear, each channel
/// rounded up, as the single-target pin of tests/es3_msaa_pvr.rs has it),
/// the rest the clear.
fn mrt_image(clear: [f32; 4], write: [f32; 4]) -> (Vec<u32>, [u8; 4]) {
    let (c, w) = (bytes(clear), bytes(write));
    let edge = [0, 1, 2, 3].map(|i| ((c[i] as u32 + w[i] as u32).div_ceil(2)) as u8);
    let img = (0..64)
        .map(|i| match i % 8 {
            0..=2 => packed(w),
            3 => packed(edge),
            _ => packed(c),
        })
        .collect();
    (img, edge)
}

fn expected(n: usize) -> Vec<Vec<[u8; 4]>> {
    (0..n)
        .map(|k| {
            let (_, edge) = mrt_image(CLEARS[k], WRITES[k]);
            (0..64)
                .map(|i| match i % 8 {
                    0..=2 => bytes(WRITES[k]),
                    3 => edge,
                    _ => bytes(CLEARS[k]),
                })
                .collect()
        })
        .collect()
}

/// The GL core's log line of a draw the back end answered -ENOTSUP. The
/// PowerVR's own line that names the cause is `glog_once!`, once a process,
/// so a test cannot count on seeing it.
const REFUSED: &str = "was not recorded (-95)";

/// Pin of today's behaviour: a 4x draw into `n` colour targets reaches the
/// back end as `samples` 4 and `n` targets and the PowerVR refuses it by
/// name: no 4x kick, nothing stored into any renderbuffer.
fn pin_refusal(n: usize) {
    let _g = context();
    let (fbo, s, errors) = clear_and_draw(n);
    assert_eq!((s.samples, s.targets.len()), (4, n), "the GL core hands over every target");
    assert!(rig::logs().iter().any(|l| l.contains(REFUSED)), "refused with ENOTSUP: {:?}", rig::logs());
    assert!(errors.contains(&GL_INVALID_OPERATION), "the refusal surfaces as GL_INVALID_OPERATION: {errors:?}");
    assert!(rig::four_sample_kicks().is_empty(), "no 4x kick of several targets: {:?}", rig::logs());
    drop(fbo);
}

/// 2 colour targets at 4x: refused today (src/pvr, plan #325).
#[test]
fn two_4x_targets_are_refused_by_name() {
    pin_refusal(2);
}

/// 4 colour targets at 4x: refused today (src/pvr, plan #325).
#[test]
fn four_4x_targets_are_refused_by_name() {
    pin_refusal(4);
}

/// One colour target at 4x still draws (the refusal is for several only).
#[test]
fn one_4x_target_still_draws() {
    let _g = context();
    let (fbo, s, errors) = clear_and_draw(1);
    assert_eq!((s.samples, s.targets.len()), (4, 1));
    assert!(errors.is_empty(), "{errors:?}");
    assert_eq!(rig::four_sample_kicks().len(), 1, "{:?}", rig::logs());
    drop(fbo);
}

/// The expected result for `n` targets: each attachment resolves, through
/// `glReadBuffer`, to its own clear and written colours.
fn check_each_target(n: usize) {
    let _g = context();
    rig::with_platform(|p| p.paints = (0..n).map(|k| mrt_image(CLEARS[k], WRITES[k]).0).collect());
    let (fbo, s, errors) = clear_and_draw(n);
    assert!(errors.is_empty(), "GL errors {errors:?}: {:?}", rig::logs());
    assert_eq!((s.samples, s.targets.len()), (4, n));
    let kicks = rig::four_sample_kicks();
    assert!(!kicks.is_empty(), "a 4x kick: {:?}", rig::logs());
    for (k, e) in kicks.last().unwrap().1.iter().enumerate() {
        assert_eq!(rig::emit_address(*e), rb_gpu(fbo.rbs[k]), "target {k} stores into its own renderbuffer");
    }
    let want = expected(n);
    for (k, want) in want.iter().enumerate() {
        let (e, px) = resolve_target(&fbo, k);
        assert_eq!(e, GL_NO_ERROR, "target {k}: {:?}", rig::logs());
        for (i, (p, w)) in px.iter().zip(want).enumerate() {
            assert!(near(*p, *w, 1), "target {k} pixel {i}: {p:?}, wanted {w:?}");
        }
    }
}

/// 2 targets at 4x resolve each to their own image.
/// Owner: src/pvr (plan #325): `pvr_backend_draw` refuses several 4x targets.
#[test]
#[ignore = "src/pvr (plan #325): several colour attachments of a 4x framebuffer are refused (ENOTSUP)"]
fn two_4x_targets_resolve_each_to_its_own_image() {
    check_each_target(2);
}

/// 4 targets at 4x resolve each to their own image.
/// Owner: src/pvr (plan #325), as above.
#[test]
#[ignore = "src/pvr (plan #325): several colour attachments of a 4x framebuffer are refused (ENOTSUP)"]
fn four_4x_targets_resolve_each_to_their_own_image() {
    check_each_target(4);
}
