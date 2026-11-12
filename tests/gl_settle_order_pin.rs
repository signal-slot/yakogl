// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! A CPU mipmap filter of a render target sees the render: glGenerateMipmap
//! on a texture that an FBO has just rendered into, with no glFinish before
//! it, along the route that filters on the CPU (`texture::glGenerateMipmap`,
//! the GL core's box filter after `settle_texture`'s drain, src/gl/texture.rs).
//! It pins the order today's code guarantees, for the item that narrows the
//! drain to a wait for the target's own kick (docs/notes/gl-settle-drain.md):
//! levels 0..=last of the texture are the box filter ((sum + 2) / 4 a
//! channel, exactly) of the picture THIS frame rendered, not of the frame
//! before's. The render's kick is queued and never runs on its own: it runs
//! only when the CPU waits for it (or, on the V3D, when the next job is
//! submitted), and the stand-in GPU stores the picture when it runs, so a
//! filter that read the storage before the wait would read the old picture.
//!
//! Each backend runs it twice, in one context over four frames: the render
//! alone, and with an unrelated render on the canvas queued before the
//! target's and another after it (before the call).
//!
//! * V3D (`gpu-v3d`): the OSAL has no TFU (`OSAL_QUERY_V3D_TFU_MIN_BYTES`
//!   answers -ENOTSUP, as src/osal/linux.c without `YAKOGL_V3D_TFU`), so the
//!   chain is the GL core's. The platform is that of
//!   tests/v3d_tfu_mipmap_nowait.rs: one job in flight, finished by a wait or
//!   the next submit; the job builder is stood in for as in
//!   tests/mipmap_offer_guard.rs, and the job's store writes the raster
//!   storage and the render image.
//! * PowerVR (`gpu-powervr`): `Backend::mip_cpu_tail` is private and off, so
//!   the CPU route is a refusal the backend makes unchanged: a 2x64 target's
//!   level 1 is one texel wide above a taller tail, which the PBE cannot
//!   write (`mip_chain_gpu`: "a level one texel wide above a taller tail"),
//!   so `pvr_backend_mip_chains` counts one refused chain and the GL core
//!   filters all of it. (`mip_cpu_tail` itself would leave the stand-in
//!   kicks' levels 1..=3 unwritten, so its tail could not be checked here.)
//!
//! The levels are read from the texture's storage (`level_rows`), the place
//! the filter writes them.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test gl_settle_order_pin
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test gl_settle_order_pin
//! ```

#![cfg(all(
    feature = "host-test",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64"),
    any(
        all(feature = "gpu-powervr", not(feature = "no-shader-table")),
        all(feature = "gpu-v3d", feature = "shaderc-online")
    )
))]
/* the stand-ins take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

/// The texels and the filter both back ends' tests share.
pub mod common {
    /// Texel (x, y) of the render of frame `phase`: every channel a different
    /// ramp, so no average comes out right by accident.
    pub fn texel(phase: u32, x: u32, y: u32) -> [u8; 4] {
        [
            (x * 3 + phase * 29) as u8,
            (y * 5 + phase * 7) as u8,
            ((x ^ y) * 11 + phase) as u8,
            (255 - ((x + y) & 63)) as u8,
        ]
    }

    /// Level 0 of frame `phase`: `h` rows of `w` texels, no padding.
    pub fn level0(phase: u32, w: u32, h: u32) -> Vec<u8> {
        (0..h).flat_map(|y| (0..w).flat_map(move |x| texel(phase, x, y))).collect()
    }

    /// One level of `w` x `h` RGBA8 texels into the next, as the GL core's
    /// glGenerateMipmap filters 8-bit channels: each side halved (rounded
    /// down) unless it is 1, the box two texels along a halved side and one
    /// along a side of 1, the sum's average rounded half up.
    pub fn box_level(src: &[u8], w: u32, h: u32) -> Vec<u8> {
        let (dw, dh) = ((w / 2).max(1), (h / 2).max(1));
        let (sx, sy) = (if w > 1 { 2 } else { 1 }, if h > 1 { 2 } else { 1 });
        let mut out = Vec::with_capacity((dw * dh * 4) as usize);
        for y in 0..dh {
            for x in 0..dw {
                for c in 0..4 {
                    let sum: u32 = (0..sy)
                        .flat_map(|j| (0..sx).map(move |i| (j, i)))
                        .map(|(j, i)| u32::from(src[(((y * sy + j) * w + x * sx + i) * 4 + c) as usize]))
                        .sum();
                    out.push(((sum + sx * sy / 2) / (sx * sy)) as u8);
                }
            }
        }
        out
    }

    /// Every level of the chain of frame `phase`'s render: entry `l` is level
    /// `l`'s rows, `(w >> l).max(1)` texels wide.
    pub fn chain(phase: u32, w: u32, h: u32) -> Vec<Vec<u8>> {
        let mut levels = vec![level0(phase, w, h)];
        let (mut lw, mut lh) = (w, h);
        while lw > 1 || lh > 1 {
            let next = box_level(levels.last().unwrap(), lw, lh);
            (lw, lh) = ((lw / 2).max(1), (lh / 2).max(1));
            levels.push(next);
        }
        levels
    }
}

/* ===================================================================== PowerVR == */

#[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))]
mod pvr {
    use core::ffi::{c_char, c_void, CStr};
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard, Once};

    use super::common;
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

    /// The backing of a GPU allocation, kept alive by the table (the CPU
    /// reaches it through the descriptor's pointer).
    #[allow(dead_code)]
    struct Alloc {
        /// u64 backing: every u32 view of it is aligned.
        mem: Box<[u64]>,
        size: usize,
    }

    #[derive(Default)]
    struct Platform {
        bump: [u64; 3],
        allocs: BTreeMap<u64, Alloc>,
        /// Submits so far: the last fence.
        fence: i64,
        /// Completed through this fence (only a wait advances it).
        completed_to: i64,
        /// The kicks of the target not yet run: (fence, frame's phase).
        queued: Vec<(i64, u32)>,
        /// The phases whose picture the GPU has stored, in order.
        stored: Vec<u32>,
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

    /// The "GPU": a kick is only queued. It runs when the CPU waits for it
    /// (`p_wait`), never on its own, so a CPU read of what it renders is right
    /// only if the CPU waited first. A kick of the target's size stores the
    /// picture of the frame it was submitted in into the target's level 0 at
    /// that point.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<PvrRenderKick>() {
            return -22;
        }
        // SAFETY: a PvrRenderKick (the length says so).
        let k = unsafe { *(kick as *const PvrRenderKick) };
        let is_target = TARGET.load(Ordering::SeqCst) == (u64::from(k.width) << 32 | u64::from(k.height));
        let phase = PHASE.load(Ordering::SeqCst);
        with_platform(|p| {
            p.fence += 1;
            if is_target {
                p.queued.push((p.fence, phase));
            }
            p.fence
        })
    }

    /// Run the kicks through `fence`: the stores of the target's picture.
    fn complete_to(fence: i64) {
        let run = with_platform(|p| {
            p.completed_to = p.completed_to.max(fence);
            let (run, keep): (Vec<_>, Vec<_>) = core::mem::take(&mut p.queued).into_iter().partition(|q| q.0 <= fence);
            p.queued = keep;
            p.stored.extend(run.iter().map(|q| q.1));
            run
        });
        for (_, phase) in run {
            gpu_store(phase);
        }
    }

    unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
        complete_to(fence);
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.completed_to)
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

    /* The base-instance entry point: the stand-in ignores the base. */
    #[no_mangle]
    unsafe extern "C" fn pvr_pds_render_vertex_program_ids_base(
        vs_usc_exec_offset: u64,
        vs_temps: u32,
        streams: *const PvrPdsVertexStream,
        num_streams: u32,
        has_slc_mcu_cache_controls: i32,
        vertex_id_reg: u32,
        instance_id_reg: u32,
        _instance_id_base: u32,
        data_out: *mut u32,
        code_out: *mut u32,
        info: *mut PvrPdsRenderVertexInfo,
    ) -> i32 {
        // SAFETY: the caller's buffers, as for the stand-in above.
        unsafe {
            pvr_pds_render_vertex_program_ids(
                vs_usc_exec_offset,
                vs_temps,
                streams,
                num_streams,
                has_slc_mcu_cache_controls,
                vertex_id_reg,
                instance_id_reg,
                data_out,
                code_out,
                info,
            )
        }
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

    const CANVAS: (i32, i32) = (40, 30);

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();
    static mut CTX: *mut GlContext = core::ptr::null_mut();
    static mut PROGRAMS: (GLuint, GLuint) = (0, 0);

    /// The one context and backend of the file, set up once; the tests take
    /// turns.
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
                PROGRAMS = (program("passthrough.vert", "uniform_color.frag"), program("uv.vert", "texture.frag"));
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        with_platform(|p| p.log.clear());
        g
    }

    fn ctx() -> &'static GlContext {
        // SAFETY: set once in `rig`, never freed; read under the lock.
        unsafe { &*CTX }
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

    /* a triangle at z = 0 covering the target, (x, y, z, w) and (u, v) per vertex */
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];
    static UV: [f32; 6] = [0.0, 0.0, 2.0, 0.0, 0.0, 2.0];

    fn draw(prog: GLuint) {
        // SAFETY: plain GL calls; the arrays are statics.
        unsafe {
            glUseProgram(prog);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glVertexAttribPointer(1, 2, GL_FLOAT, 0, 0, UV.as_ptr() as *const c_void);
            glEnableVertexAttribArray(1);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }

    /* ---- glmark2's mipmapped render target ---------------------------------------- */

    /// What glmark2 terrain and refract make: an RGBA8 texture of `w` x `h`,
    /// level 0 only (glTexImage2D with no data), sampled with a mipmap filter,
    /// and the FBO with it as the colour attachment.
    fn render_target(w: i32, h: i32) -> (GLuint, GLuint) {
        let (mut tex, mut fbo) = (0, 0);
        // SAFETY: room for one name each; a null image.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        (tex, fbo)
    }

    /// The scene's render pass: bind the FBO, draw into it, bind the canvas back.
    fn render_pass(fbo: GLuint, w: i32, h: i32) {
        let (fill, _) = unsafe { PROGRAMS };
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glViewport(0, 0, w, h);
        }
        draw(fill);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0, CANVAS.1);
        }
    }

    /* ---- the order ---------------------------------------------------------------- */

    /// The target's size (`w << 32 | h`), the texture it renders into and the
    /// frame's phase (what the kick that stores it will store).
    static TARGET: AtomicU64 = AtomicU64::new(0);
    static TEX: AtomicU32 = AtomicU32::new(0);
    static PHASE: AtomicU32 = AtomicU32::new(0);

    /// The GPU's store of the render pass into level 0 of the target: the
    /// stand-in GPU draws nothing, so this writes what the pass leaves, rows
    /// at the texture's own pitch, when the kick that renders it runs. Levels
    /// above 0 are not touched.
    fn gpu_store(phase: u32) {
        let t = &ctx().objs_ref().textures[TEX.load(Ordering::SeqCst) as usize];
        let (w, h) = (t.width, t.height);
        let pitch = t.level_pitch_at(0) as usize;
        // SAFETY: the texture's live storage, `mem.size` bytes.
        let mem = unsafe { core::slice::from_raw_parts_mut(t.mem.cpu as *mut u8, t.mem.size) };
        let off0 = t.level_off[0] as usize;
        for y in 0..h as usize {
            for x in 0..w as usize {
                let at = off0 + (y * pitch + x) * 4;
                mem[at..at + 4].copy_from_slice(&common::texel(phase, x as u32, y as u32));
            }
        }
    }

    /// Level `l` of `tex` as the storage holds it: rows of `w >> l` texels, 4
    /// bytes each, without the row padding.
    fn level_rows(tex: GLuint, l: u32) -> Vec<u8> {
        let t = &ctx().objs_ref().textures[tex as usize];
        let (lw, lh) = ((t.width >> l).max(1) as usize, (t.height >> l).max(1) as usize);
        let (pitch, off) = (t.level_pitch_at(l) as usize, t.level_off[l as usize] as usize);
        // SAFETY: the texture's live storage, `mem.size` bytes.
        let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
        (0..lh).flat_map(|y| mem[off + y * pitch * 4..off + (y * pitch + lw) * 4].iter().copied()).collect()
    }

    /// The GPU mipmap chains made, and asked for but left to the CPU, since
    /// the backend started.
    fn mip_chains() -> (u32, u32) {
        let (mut made, mut refused) = (0u32, 0u32);
        // SAFETY: live out-parameters.
        unsafe { pvr_backend_mip_chains(&mut made, &mut refused) };
        (made, refused)
    }

    /// The pictures the GPU has stored so far (the phases, in order).
    fn stored() -> Vec<u32> {
        with_platform(|p| p.stored.clone())
    }

    /// An unrelated render: a draw on the canvas.
    fn unrelated() {
        let (fill, _) = unsafe { PROGRAMS };
        draw(fill);
    }

    /// glmark2's frame up to the mipmap call, four times, in one context: the
    /// render pass into the texture (its kick queued, not run), optionally an
    /// unrelated render on the canvas before it and after it, then
    /// glGenerateMipmap on the texture with no glFinish. The chain the backend
    /// refuses (so the GL core filters it) must be the box filter of THIS
    /// frame's render, not of the frame before's or of what was never written.
    fn cpu_filter_sees_the_render(w: i32, h: i32, around: bool) {
        let _g = rig();
        let (tex, fbo) = render_target(w, h);
        TARGET.store(u64::from(w as u32) << 32 | u64::from(h as u32), Ordering::SeqCst);
        TEX.store(tex, Ordering::SeqCst);
        with_platform(|p| p.stored.clear());
        let what = format!("{w}x{h} around={around}");
        for phase in 1..=4 {
            PHASE.store(phase, Ordering::SeqCst);
            if around {
                unrelated();
            }
            render_pass(fbo, w, h);
            if around {
                unrelated();
            }
            assert!(!stored().contains(&phase), "{what} frame {phase}: the render has not run yet");
            let (m0, r0) = mip_chains();
            // SAFETY: plain GL calls on a live texture.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, tex);
                glGenerateMipmap(GL_TEXTURE_2D);
                assert_eq!(glGetError(), GL_NO_ERROR, "{what} frame {phase}: {:?}", logs());
            }
            let (m1, r1) = mip_chains();
            assert_eq!((m1 - m0, r1 - r0), (0, 1), "{what} frame {phase}: the backend left the chain to the GL core");
            assert!(!ctx().objs_ref().textures[tex as usize].chain_in_backend, "{what}: the CPU filtered it");
            assert!(stored().contains(&phase), "{what} frame {phase}: the render ran before the filter read it");
            for (l, want) in common::chain(phase, w as u32, h as u32).iter().enumerate() {
                assert_eq!(&level_rows(tex, l as u32), want, "{what} frame {phase}: level {l}");
            }
            // SAFETY: plain GL calls.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, 0);
                glFinish();
            }
        }
        // SAFETY: names this test made.
        unsafe {
            glDeleteFramebuffers(1, &fbo);
            glDeleteTextures(1, &tex);
        }
    }

    #[test]
    fn a_cpu_filter_of_a_just_rendered_target_sees_the_render() {
        cpu_filter_sees_the_render(2, 64, false);
    }

    #[test]
    fn with_unrelated_renders_queued_before_and_after() {
        cpu_filter_sees_the_render(2, 64, true);
    }
}

/* ========================================================================= V3D == */

#[cfg(all(feature = "gpu-v3d", feature = "shaderc-online"))]
mod v3d {
    use core::ffi::{c_char, c_int, c_void, CStr};
    use std::collections::BTreeMap;
    use std::ffi::CString;
    use std::path::PathBuf;
    use std::process::Command;
    use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
    use std::sync::{Mutex, MutexGuard, OnceLock};

    use super::common;

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
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf};
    use yakogl::v3d::backend::*;
    use yakogl::v3d::ffi::*;
    use yakogl::v3d::kick::V3dKick;

    /* ---- the real layout code, built from driver/src/v3d/v3d_tex.c ------------- */

    extern "C" {
        fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
        fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
    }

    /// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
    /// object of this test's own; its symbols are reached through the handle
    /// only (RTLD_LOCAL), so the trampolines below are the ones the library
    /// links.
    fn lib() -> *mut c_void {
        static L: OnceLock<usize> = OnceLock::new();
        *L.get_or_init(|| {
            let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("depth_resolve_v3d");
            std::fs::create_dir_all(&dir).expect("the scratch directory");
            let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
            /* the compiler's own temporary files next to the object: a host
             * whose /tmp is full must not fail the check */
            let st = Command::new("cc")
                .current_dir(&root)
                .env("TMPDIR", &dir)
                .args(["-std=c11", "-O1", "-shared", "-fPIC", "-Wall", "-Wextra", "-Werror"])
                .args(["-Idriver/src/v3d", "-Idriver/src/v3d/generated"])
                .args(["-Ithird_party/mesa-26.2.1/src/broadcom", "-Ithird_party/mesa-26.2.1/src"])
                .arg("-Ithird_party/mesa-26.2.1/include")
                .args(["driver/src/v3d/v3d_tex.c", "driver/src/v3d/v3d_tiling_mesa.c", "-lm", "-o"])
                .arg(&so)
                .status()
                .expect("cc runs");
            assert!(st.success(), "driver/src/v3d/v3d_tex.c builds");
            let path = CString::new(so.to_str().expect("a utf-8 path")).unwrap();
            // SAFETY: a path of a library this test just built; RTLD_NOW.
            let h = unsafe { dlopen(path.as_ptr(), 2) };
            assert!(!h.is_null(), "the layout code loads");
            let _ = std::fs::remove_file(&so);
            h as usize
        }) as *mut c_void
    }

    fn sym(name: &str) -> *mut c_void {
        let c = CString::new(name).unwrap();
        // SAFETY: a handle `lib` made and a NUL-terminated name.
        let p = unsafe { dlsym(lib(), c.as_ptr()) };
        assert!(!p.is_null(), "{name} in v3d_tex.c");
        p
    }

    /// The trampolines: the library's `extern "C"` texture functions
    /// (src/v3d/ffi.rs), each handed to the real C. `v3d_tex_pack_state` also
    /// records the state it packed, by the record's address, for the checks.
    macro_rules! trampoline {
    ($name:ident ( $($arg:ident : $ty:ty),* ) $(-> $ret:ty)?) => {
        #[no_mangle]
        unsafe extern "C" fn $name($($arg: $ty),*) $(-> $ret)? {
            // SAFETY: the C function's own signature, as src/v3d/ffi.rs declares it.
            let f: unsafe extern "C" fn($($ty),*) $(-> $ret)? =
                unsafe { std::mem::transmute(sym(stringify!($name))) };
            // SAFETY: the library's call, passed on.
            unsafe { f($($arg),*) }
        }
    };
}

    /* The signatures are src/v3d/ffi.rs's. Kept as written: a macro call wider
     * than the width is wrapped by some rustfmt releases and left by others, so
     * the gate's `cargo fmt --check` must not depend on the host's release. */
    #[rustfmt::skip]
mod trampolines {
    use super::*;

    trampoline!(v3d_tex_layout(width: u32, height: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_layout_msaa(width: u32, height: u32, cpp: u32, l: *mut V3dTexLayout) -> bool);
    trampoline!(v3d_tex_store_level(image: *mut u8, l: *const V3dTexLayout, level: u32, src: *const u8, stride: u32));
    trampoline!(v3d_tex_tfu_pack(l: *const V3dTexLayout, ttype: u32, src: u32, stride: u32, dst: u32,
                                 j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_tfu_pack_mipmap(l: *const V3dTexLayout, ttype: u32, image: u32, j: *mut V3dTexTfuJob) -> bool);
    trampoline!(v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32);
    trampoline!(v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool);
    trampoline!(v3d_tex_store_box(image: *mut u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                  src: *const u8, stride: u32) -> bool);
    trampoline!(v3d_tex_load_box(image: *const u8, l: *const V3dTexLayout, level: u32, x: u32, y: u32, w: u32, h: u32,
                                 dst: *mut u8, stride: u32) -> bool);
    trampoline!(v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout));
    trampoline!(v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool);
}

    /// Every texture state packed, by the record's CPU address.
    static STATES: Mutex<Vec<(usize, V3dTexState)>> = Mutex::new(Vec::new());

    #[no_mangle]
    unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
        // SAFETY: the C function's signature.
        let f: unsafe extern "C" fn(*mut u8, *const V3dTexState) -> bool =
            unsafe { std::mem::transmute(sym("v3d_tex_pack_state")) };
        // SAFETY: the library's call, passed on; `s` is its live state.
        let ok = unsafe { f(out, s) };
        if ok {
            // SAFETY: the backend's live state.
            STATES.lock().unwrap_or_else(|e| e.into_inner()).push((out as usize, unsafe { *s }));
        }
        ok
    }

    /* ---- the job builder, stood in for (driver/src/v3d/v3d_job.c) -------------- */

    #[derive(Clone, Debug)]
    struct SeenJob {
        target: V3dJobTarget,
        clear: Option<V3dJobClear>,
        /// The FS texture records of each draw, in order.
        tmu: Vec<Vec<V3dJobTmu>>,
    }

    static JOBS: Mutex<Vec<SeenJob>> = Mutex::new(Vec::new());

    fn jobs() -> MutexGuard<'static, Vec<SeenJob>> {
        JOBS.lock().unwrap_or_else(|e| e.into_inner())
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_begin(
        j: *mut V3dJob,
        t: *const V3dJobTarget,
        bcl: *mut V3dCl,
        rcl: *mut V3dCl,
        ind: *mut V3dCl,
    ) -> bool {
        // SAFETY: the backend passes its live job, target and lists.
        let (j, t) = unsafe { (&mut *j, *t) };
        *j = V3dJob { bcl, rcl, ind, t, draws: 0, defaults: 0, broken: false, pack: V3dJobPack::ZERO };
        // SAFETY: as above.
        unsafe {
            (*bcl).used = 16;
            (*ind).used = 256;
        }
        jobs().push(SeenJob { target: t, clear: None, tmu: Vec::new() });
        t.width != 0 && t.height != 0
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, _layout: *mut c_void) -> i32 {
        // SAFETY: the backend passes its live job and a draw whose pointers live for the call.
        let (j, d) = unsafe { (&mut *j, &*d) };
        let tmu = if d.tmu.is_null() || d.num_tmu == 0 {
            Vec::new()
        } else {
            // SAFETY: `num_tmu` records, live for the call.
            unsafe { std::slice::from_raw_parts(d.tmu, d.num_tmu as usize) }.to_vec()
        };
        j.draws += 1;
        // SAFETY: the job's BCL is live.
        unsafe { (*j.bcl).used += 120 };
        if let Some(job) = jobs().last_mut() {
            job.tmu.push(tmu);
        }
        V3D_JOB_OK
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_job_end(
        j: *mut V3dJob,
        c: *const V3dJobClear,
        _tile_list_start: *mut u32,
        _tile_list_end: *mut u32,
    ) -> bool {
        // SAFETY: the backend passes its live job and clear.
        let (j, c) = unsafe { (&mut *j, *c) };
        // SAFETY: the job's lists are live.
        unsafe {
            (*j.bcl).used += 1;
            (*j.rcl).used = 200;
        }
        if let Some(job) = jobs().last_mut() {
            job.clear = Some(c);
        }
        true
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, 1, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, false, 0, t) }
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
        // SAFETY: the backend's out-parameter, passed on.
        unsafe { v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t) }
    }

    /// The tile-size table of v3d_frame.c (Mesa's v3d_choose_tile_size), as
    /// the backend's unit tests fake it.
    #[no_mangle]
    unsafe extern "C" fn v3d_frame_tiling_bpp(
        width: u32,
        height: u32,
        rts: u32,
        msaa: bool,
        max_bpp: u32,
        t: *mut V3dTiling,
    ) {
        const SIZES: [(u32, u32); 7] = [(64, 64), (64, 32), (32, 32), (32, 16), (16, 16), (16, 8), (8, 8)];
        let idx = match rts {
            0 | 1 => 0,
            2 => 1,
            _ => 2,
        } + if msaa { 2 } else { 0 }
            + max_bpp.min(2) as usize;
        let (tw, th) = SIZES[idx];
        let tiles = V3dTiling {
            tile_w: tw,
            tile_h: th,
            tiles_x: width.div_ceil(tw),
            tiles_y: height.div_ceil(th),
            supertile_w: 1,
            supertile_h: 1,
            frame_w_supertiles: width.div_ceil(tw),
            frame_h_supertiles: height.div_ceil(th),
        };
        // SAFETY: the backend's out-parameter.
        unsafe { *t = tiles };
    }

    #[no_mangle]
    unsafe extern "C" fn v3d_frame_alloc_sizes(
        t: *const V3dTiling,
        _draws: u32,
        _page: u32,
        ta: *mut u32,
        ts: *mut u32,
    ) {
        // SAFETY: the backend's parameters.
        unsafe {
            let n = (*t).tiles_x * (*t).tiles_y;
            *ta = n * 128 + 8192;
            *ts = n * 256;
        }
    }

    /* ---- the platform ------------------------------------------------------------ */

    /// GPU addresses are 32-bit (the V3D's MMU), from here up.
    const GPU_BASE: u64 = 0x0100_0000;

    struct Alloc {
        /// u64 backing: every u32 view of it is aligned.
        mem: Box<[u64]>,
        size: usize,
    }

    #[derive(Default)]
    struct Platform {
        next: u64,
        allocs: BTreeMap<u64, Alloc>,
        kicks: Vec<V3dKick>,
        /// The highest completed fence, and the job in flight with the
        /// frame's phase it was submitted in.
        completed: usize,
        inflight: Option<(usize, u32)>,
        /// The phases whose picture the GPU has stored, in order.
        stored: Vec<u32>,
        log: Vec<String>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    /// The CPU view of `n` bytes at GPU address `gpu`.
    fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
        let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
        let off = (gpu - base) as usize;
        assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
        // SAFETY: inside the allocation.
        unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
    }

    unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
        with_platform(|p| {
            if size == 0 || size > 256 << 20 {
                return -12;
            }
            let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
            let gpu = GPU_BASE + p.next;
            p.next += (size as u64).div_ceil(4096) * 4096;
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

    /// Finish the job in flight (zephyr_v3d.c finish_inflight): it runs, and a
    /// job into the target the test renders into stores its picture, then its
    /// fence completes.
    fn finish() {
        let Some((fence, phase)) = with_platform(|p| p.inflight.take()) else { return };
        let job = jobs().get(fence - 1).cloned().expect("the job this kick runs was begun and ended");
        if store_target(&job.target, phase) {
            with_platform(|p| p.stored.push(phase));
        }
        with_platform(|p| p.completed = fence);
    }

    /// The "GPU" as the board's OSAL runs it (src/osal/zephyr_v3d.c): one job
    /// in flight, finished only by a wait() on it or by the next submit.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        // SAFETY: the backend's live descriptor of that size.
        let k = unsafe { *(kick as *const V3dKick) };
        finish();
        let phase = PHASE.load(Ordering::SeqCst);
        with_platform(|p| {
            p.kicks.push(k);
            p.inflight = Some((p.kicks.len(), phase));
            p.kicks.len() as i64
        })
    }

    unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
        if with_platform(|p| p.inflight.is_some_and(|(f, _)| fence >= f as i64)) {
            finish();
        }
        0
    }

    /// The GPU never finishes on its own: only what a wait or a submit finished.
    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.completed as i64)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        use yakogl::osal::{
            OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_TFU_MIN_BYTES,
            OSAL_QUERY_V3D_VPM_BYTES,
        };
        /* no TFU: the CPU filter is the only route (-ENOTSUP, as linux.c without YAKOGL_V3D_TFU) */
        if what == OSAL_QUERY_V3D_TFU_MIN_BYTES {
            return -95;
        }
        if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
            // SAFETY: a u32 out-parameter.
            unsafe { *(out as *mut u32) = 65536 };
            return 0;
        }
        if what == OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<OsalV3dCpuOf>() {
            // SAFETY: the backend's in/out struct of that size.
            let q = unsafe { &mut *(out as *mut OsalV3dCpuOf) };
            return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
                Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                    // SAFETY: inside the allocation.
                    q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                    0
                }
                _ => -22,
            });
        }
        if what == OSAL_QUERY_V3D_JOB_OPEN && out_sz == 4 {
            return 0;
        }
        if what == OSAL_QUERY_V3D_BIN_POOL {
            return -61; /* ENODATA, as zephyr_v3d.c answers before the first job */
        }
        -22 /* the default budgets, no clock */
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

    unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
        0
    }

    /// The platform's table (made once, at run time: the log hook is a cast).
    fn osal() -> &'static OsalOps {
        // SAFETY: see p_log2: the library only ever calls it with ("%s", line).
        let log = unsafe { core::mem::transmute::<unsafe extern "C" fn(*const c_char, *const c_char), LogFn>(p_log2) };
        Box::leak(Box::new(OsalOps {
            init: Some(p_init),
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

    /// The layout of a `w` x `h` render image of `levels` levels, as
    /// `texture::rt_layout` lays it out: level 0 without the UIF XOR.
    fn rt_layout(w: u32, h: u32, levels: u32) -> Option<V3dTexLayout> {
        let mut l = V3dTexLayout::default();
        // SAFETY: a live layout to fill.
        if !unsafe { v3d_tex_layout(w, h, levels, 4, &mut l) } {
            return None;
        }
        if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
            l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
        }
        Some(l)
    }

    /// The render pass's store, which the stand-in GPU does when a job into
    /// the target the test is rendering into (`TARGET`) finishes: the picture
    /// of the frame it was submitted in (`phase`) into the raster storage the
    /// job writes, and into the render image (`uif_address`) in its layout, as
    /// the tile list stores both. Whether it stored.
    fn store_target(t: &V3dJobTarget, phase: u32) -> bool {
        let (w, h) = (t.width, t.height);
        if t.address == 0 || TARGET.load(Ordering::SeqCst) != (u64::from(w) << 32 | u64::from(h)) {
            return false;
        }
        let rows = common::level0(phase, w, h);
        with_platform(|p| {
            let raster = platform_cpu(p, u64::from(t.address), (t.stride * h) as usize);
            for y in 0..h as usize {
                // SAFETY: row `y` of the raster target, `w` texels inside `stride * h` bytes.
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        rows.as_ptr().add(y * w as usize * 4),
                        raster.add(y * t.stride as usize),
                        w as usize * 4,
                    )
                };
            }
            if t.uif_address == 0 {
                return;
            }
            /* the image's layout: the one whose allocation (the layout's size
             * and a page) starts a page before level 0's base */
            let found = (1..=16).find_map(|levels| {
                let l = rt_layout(w, h, levels)?;
                let base = t.uif_address.checked_sub(l.slices[0].offset)?;
                let (start, a) = p.allocs.range(..=u64::from(base)).next_back()?;
                (start.next_multiple_of(4096) == u64::from(base) && a.size == l.size as usize + 4096)
                    .then_some((l, base))
            });
            let (l, base) = found.expect("the render image is an allocation of a layout the job's address names");
            assert_eq!(t.uif_memory_format, l.slices[0].tiling, "the store's memory format is the layout's tiling");
            let image = platform_cpu(p, u64::from(base), l.size as usize);
            // SAFETY: the image's live mapping holds the laid-out chain; `rows` is level 0.
            unsafe { v3d_tex_store_level(image, &l, 0, rows.as_ptr(), w * 4) };
        });
        true
    }

    /* ---- the context ------------------------------------------------------------- */

    const CANVAS: (i32, i32) = (40, 30);

    static LOCK: Mutex<()> = Mutex::new(());
    static TARGET: AtomicU64 = AtomicU64::new(0);
    static PHASE: AtomicU32 = AtomicU32::new(0);
    static mut CTX: *mut GlContext = core::ptr::null_mut();

    /// A back end started on a platform with no TFU (the CPU filter is the
    /// only route), and a context on it; torn down when dropped.
    struct Rig {
        _g: MutexGuard<'static, ()>,
    }

    fn rig() -> Rig {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        /* a fresh back end counts kicks from 1; memory the previous one left
         * allocated stays (the GL core may still point at it) */
        with_platform(|p| {
            p.kicks.clear();
            p.log.clear();
            p.completed = 0;
            p.inflight = None;
            p.stored.clear();
        });
        jobs().clear();
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
        Rig { _g: g }
    }

    impl Drop for Rig {
        fn drop(&mut self) {
            // SAFETY: the context `rig` made, under the lock.
            unsafe { context::gl_context_destroy(CTX) };
            yakogl::backend::shutdown();
        }
    }

    fn ctx() -> &'static mut GlContext {
        // SAFETY: set in `rig`, used under the lock.
        unsafe { &mut *CTX }
    }

    fn logs() -> Vec<String> {
        with_platform(|p| p.log.clone())
    }

    fn shader(type_: GLenum, src: &str) -> GLuint {
        let c = CString::new(src).unwrap();
        // SAFETY: plain GL calls; the string lives for the call.
        unsafe {
            let s = glCreateShader(type_);
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok: GLint = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile 0x{type_:x}: {src}\n{:?}", logs());
            s
        }
    }

    fn program(vs: &str, fs: &str) -> GLuint {
        let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, fs));
        let name = CString::new("a_position").unwrap();
        // SAFETY: plain GL calls on valid handles.
        unsafe {
            let p = glCreateProgram();
            glAttachShader(p, v);
            glAttachShader(p, f);
            glBindAttribLocation(p, 0, name.as_ptr());
            glLinkProgram(p);
            let mut ok: GLint = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            assert_eq!(ok, 1, "link failed: {:?}", logs());
            p
        }
    }

    /// The render pass's program draws a constant (the stand-in GPU stores the
    /// picture).
    const FILL_VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    const FILL_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";

    /* a quad over the whole target at z = 0, (x, y, z) per vertex */
    static QUAD: [f32; 18] = [
        -1.0, -1.0, 0.0, 1.0, -1.0, 0.0, -1.0, 1.0, 0.0, //
        -1.0, 1.0, 0.0, 1.0, -1.0, 0.0, 1.0, 1.0, 0.0,
    ];

    fn draw_quad() {
        // SAFETY: plain GL calls; the array is a static.
        unsafe {
            glVertexAttribPointer(0, 3, GL_FLOAT, 0, 12, QUAD.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 6);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }

    /// What glmark2 terrain and refract make: an RGBA8 texture of `w` x `h`,
    /// level 0 only (glTexImage2D with no data), sampled with a mipmap filter,
    /// and the FBO with it as the colour attachment.
    fn render_target(w: i32, h: i32) -> (GLuint, GLuint) {
        let (mut tex, mut fbo) = (0, 0);
        // SAFETY: room for one name each; a null image.
        unsafe {
            glGenTextures(1, &mut tex);
            glBindTexture(GL_TEXTURE_2D, tex);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
            glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
            glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        (tex, fbo)
    }

    /* ---- the order ---------------------------------------------------------------- */

    /// The render images' counters since init: chains the TFU made.
    fn made_chains() -> u32 {
        let mut chains = 0u32;
        // SAFETY: a live out-parameter; the others null.
        unsafe {
            v3d_rt_images(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut chains,
                core::ptr::null_mut(),
            )
        };
        chains
    }

    /// Level `l` of `tex` as the storage holds it: rows of `w >> l` texels, 4
    /// bytes each, without the row padding.
    fn level_rows(tex: GLuint, l: u32) -> Vec<u8> {
        let t = &ctx().objs_ref().textures[tex as usize];
        let (lw, lh) = ((t.width >> l).max(1) as usize, (t.height >> l).max(1) as usize);
        let (pitch, off) = (t.level_pitch_at(l) as usize, t.level_off[l as usize] as usize);
        // SAFETY: the texture's live storage, `mem.size` bytes.
        let mem = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, t.mem.size) };
        (0..lh).flat_map(|y| mem[off + y * pitch * 4..off + (y * pitch + lw) * 4].iter().copied()).collect()
    }

    /// The pictures the GPU has stored so far (the phases, in order).
    fn stored() -> Vec<u32> {
        with_platform(|p| p.stored.clone())
    }

    /// An unrelated render: a draw on the canvas.
    fn unrelated(fill: GLuint) {
        // SAFETY: plain GL calls on live names.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0, CANVAS.1);
            glUseProgram(fill);
        }
        draw_quad();
    }

    /// glmark2's frame up to the mipmap call, four times, in one context: the
    /// render pass into the texture (its job open or in flight, not
    /// finished), optionally an unrelated render on the canvas before it and
    /// after it, then glGenerateMipmap on the texture with no glFinish. With
    /// no TFU the GL core filters the chain, and it must be the box filter of
    /// THIS frame's render, not of the frame before's.
    fn cpu_filter_sees_the_render(w: i32, h: i32, around: bool) {
        let _r = rig();
        let fill = program(FILL_VS, FILL_FS);
        let (tex, fbo) = render_target(w, h);
        TARGET.store(u64::from(w as u32) << 32 | u64::from(h as u32), Ordering::SeqCst);
        let what = format!("{w}x{h} around={around}");
        for phase in 1..=4 {
            PHASE.store(phase, Ordering::SeqCst);
            if around {
                unrelated(fill);
            }
            // SAFETY: plain GL calls on live names.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glViewport(0, 0, w, h);
                glUseProgram(fill);
            }
            draw_quad();
            if around {
                unrelated(fill);
            }
            // SAFETY: plain GL calls on live names.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glViewport(0, 0, CANVAS.0, CANVAS.1);
            }
            assert!(!stored().contains(&phase), "{what} frame {phase}: the render has not run yet");
            let made = made_chains();
            // SAFETY: plain GL calls on a live texture.
            unsafe {
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(GL_TEXTURE_2D, tex);
                glGenerateMipmap(GL_TEXTURE_2D);
                assert_eq!(glGetError(), GL_NO_ERROR, "{what} frame {phase}: {:?}", logs());
            }
            assert_eq!(made_chains(), made, "{what} frame {phase}: no TFU chain: the GL core filtered it");
            assert!(!ctx().objs_ref().textures[tex as usize].chain_in_backend, "{what}: the CPU filtered it");
            assert!(stored().contains(&phase), "{what} frame {phase}: the render ran before the filter read it");
            for (l, want) in common::chain(phase, w as u32, h as u32).iter().enumerate() {
                assert_eq!(&level_rows(tex, l as u32), want, "{what} frame {phase}: level {l}");
            }
            // SAFETY: plain GL calls.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, 0);
                glFinish();
            }
        }
        // SAFETY: names this test made.
        unsafe {
            glDeleteFramebuffers(1, &fbo);
            glDeleteTextures(1, &tex);
        }
    }

    #[test]
    fn a_cpu_filter_of_a_just_rendered_target_sees_the_render() {
        cpu_filter_sees_the_render(64, 64, false);
    }

    #[test]
    fn with_unrelated_renders_queued_before_and_after() {
        cpu_filter_sees_the_render(64, 64, true);
    }
}
