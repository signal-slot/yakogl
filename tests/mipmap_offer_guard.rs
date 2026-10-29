// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glGenerateMipmap on glmark2's rendered-into RGBA8 mip chains, on the
//! PowerVR and the V3D: a guard for the GL core's offer of such chains to
//! a back end's GPU mipmap path (`texture::glGenerateMipmap`,
//! `backend::generate_mipmap`).
//!
//! The GC7000's offer widens what the GL core hands a back end (the
//! chains glmark2's terrain and refract render into, `pe_order` storage
//! and a chain pitch of level 0's: tools/hostharness/glmark2/FINDINGS-gc.md,
//! docs/notes/imx8mp-glmark2-cpu.md). The PowerVR and the V3D must answer
//! as they did: these tests pin what each does today for the sequence of
//! glmark2's `terrain` and `refract` (scene-terrain.cpp, scene-refract.cpp
//! at the pinned 22c527cb): an RGBA8 texture with a mipmap filter and level
//! 0 only, rendered into through an FBO every frame, `glGenerateMipmap`
//! on it, then sampled by the scene.
//!
//! Both run the REAL back end through the GL core on a platform written
//! here (the rigs of tests/depth_resolve_pvr_backend.rs and
//! tests/depth_resolve_v3d.rs: GPU memory is host memory, every submit
//! completes at once, the command generators and the job builder are
//! stand-ins). The stand-in GPU draws nothing, so the render pass's store is
//! the test's: level 0 is written with a known picture where the job would
//! have stored it.
//!
//! * PowerVR (`gpu-powervr`): the back end draws levels 1.. itself, one
//!   kick per level, and the GL core filters nothing. Pinned: the back end's
//!   made/refused counters (`pvr_backend_mip_chains`), the kicks, the
//!   storage's levels 1.. left as they were (the stand-in kicks write no
//!   texels, so box-filter contents cannot be checked here: the GL core
//!   leaves the levels alone, which is the path), level 0 untouched, and
//!   the chain handed back when the CPU writes the texture.
//! * V3D (`gpu-v3d`): the chain is made by a TFU job from the render image
//!   when the OSAL has a TFU (`YAKOGL_V3D_TFU=1` on the host's
//!   src/osal/linux.c; here the OSAL answers `OSAL_QUERY_V3D_TFU_MIN_BYTES`
//!   and runs the job with the same box filter as linux.c's model), and by
//!   the GL core on the CPU otherwise (the default, `YAKOGL_V3D_TFU` off:
//!   the query answers -ENOTSUP), or when the TFU job fails. Pinned: the
//!   made-chains counter (`v3d_rt_images`) in each case, and that every level
//!   holds the box filter of the render (within 1 per channel), whichever
//!   path made it.
//!
//! Nothing here asserts on log text; the counters are the back ends' own.
//!
//! ```text
//! cargo test --no-default-features --features host-test,gpu-powervr --test mipmap_offer_guard
//! cargo test --no-default-features --features host-test,gpu-v3d,shaderc-online,no-shader-table --test mipmap_offer_guard
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
    #[cfg(feature = "gpu-v3d")]
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

    /// The largest per-byte difference of two levels.
    #[cfg(feature = "gpu-v3d")]
    pub fn max_diff(a: &[u8], b: &[u8]) -> u32 {
        assert_eq!(a.len(), b.len(), "the levels' sizes");
        a.iter().zip(b).map(|(&x, &y)| u32::from(x.abs_diff(y))).max().unwrap_or(0)
    }
}

/* ===================================================================== PowerVR == */

#[cfg(all(feature = "gpu-powervr", not(feature = "no-shader-table")))]
mod pvr {
    use core::ffi::{c_char, c_void, CStr};
    use std::collections::BTreeMap;
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

    /// ISP_ZLSCTL.forcezstore: the kick stores its depth/stencil.
    const ZLSCTL_FORCEZSTORE: u64 = 1 << 2;

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
        /// Completed through this fence while `hold` is set.
        completed_to: i64,
        /// Kicks complete only when waited for (a GPU still busy).
        hold: bool,
        /// The depth word each kick stored, by fence.
        stored: BTreeMap<i64, u32>,
        log: Vec<String>,
    }

    static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

    fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
        let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
        f(g.get_or_insert_with(Platform::default))
    }

    /// The D24S8 word kick `fence` stores: depth 0.25 + fence/1024, stencil 0.
    fn depth_word(fence: i64) -> u32 {
        let d = 0.25 + fence as f64 / 1024.0;
        (d * 16_777_215.0).round() as u32 & 0xff_ffff
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

    /// The "GPU": a kick that stores depth leaves `depth_word(fence)` in every
    /// word of its depth/stencil buffer.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<PvrRenderKick>() {
            return -22;
        }
        // SAFETY: a PvrRenderKick (the length says so).
        let k = unsafe { *(kick as *const PvrRenderKick) };
        with_platform(|p| {
            p.fence += 1;
            let fence = p.fence;
            let zs = k.frag.isp_zload_store_base;
            let stores = zs != 0 && k.frag.isp_zlsctl & ZLSCTL_FORCEZSTORE != 0;
            let Some((&base, a)) = p.allocs.range_mut(..=zs).next_back().filter(|_| stores) else { return fence };
            let off = (zs - base) as usize;
            if off < a.size {
                let w = depth_word(fence);
                let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
                // SAFETY: the words from `zs` to the allocation's end.
                unsafe { core::slice::from_raw_parts_mut(first, (a.size - off) / 4) }.fill(w);
                p.stored.insert(fence, w);
            }
            fence
        })
    }

    unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
        with_platform(|p| p.completed_to = p.completed_to.max(fence));
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| if p.hold { p.completed_to } else { p.fence })
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        if what != OSAL_QUERY_RENDER_STATIC || out_sz != size_of::<PvrRenderStatic>() {
            return -22;
        }
        let st = PvrRenderStatic {
            eot_doutu_src0: 0x1234,
            event_pixel_pds_info: 0x55,
            eot_doutu_src0_n: [0, 0x1001, 0x1002, 0x1003],
            event_pixel_pds_info_n: [0, 0x61, 0x62, 0x63],
            eot_doutu_src0_tb_n: [0, 0x2001, 0x2002, 0x2003],
            event_pixel_pds_info_tb_n: [0, 0x71, 0x72, 0x73],
            tile_buffer_gpu: [HEAP_GENERAL_BASE + 0x3E00_0000, HEAP_GENERAL_BASE + 0x3E00_8000],
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

    /// The GPU's store of the render pass into level 0: the stand-in GPU draws
    /// nothing, so the test writes what the pass would have left, rows at the
    /// texture's own pitch. The levels above 0 are filled with `sentinel`: what
    /// is still there after glGenerateMipmap was not written by the GL core.
    fn gpu_store(tex: GLuint, phase: u32, sentinel: u8) {
        let t = &ctx().objs_ref().textures[tex as usize];
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
        for l in 1..u32::from(t.chain_levels) {
            let (lw, lh) = ((w >> l).max(1) as usize, (h >> l).max(1) as usize);
            let (pitch, off) = (t.level_pitch_at(l) as usize, t.level_off[l as usize] as usize);
            for y in 0..lh {
                mem[off + y * pitch * 4..off + (y * pitch + lw) * 4].fill(sentinel);
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

    fn fence() -> i64 {
        with_platform(|p| p.fence)
    }

    /// What one glGenerateMipmap came to.
    struct Call {
        made: u32,
        refused: u32,
        /// Kicks it submitted.
        kicks: i64,
        /// The GL core left the chain to the backend (`chain_in_backend`).
        in_backend: bool,
    }

    /// glmark2's frame, `frames` times: the render pass into the texture, then
    /// glGenerateMipmap on it, then the scene samples it. The texture, and the
    /// last frame's glGenerateMipmap.
    fn chain_frames(w: i32, h: i32, frames: u32) -> (GLuint, Call) {
        let (_, sample) = unsafe { PROGRAMS };
        let (tex, fbo) = render_target(w, h);
        let mut last = Call { made: 0, refused: 0, kicks: 0, in_backend: false };
        for phase in 1..=frames {
            render_pass(fbo, w, h);
            // SAFETY: plain GL call; every kick completes at once.
            unsafe { glFinish() };
            gpu_store(tex, phase, 0xA5);
            let ((m0, r0), f0) = (mip_chains(), fence());
            // SAFETY: plain GL calls on a live texture.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, tex);
                glGenerateMipmap(GL_TEXTURE_2D);
                assert_eq!(glGetError(), GL_NO_ERROR, "glGenerateMipmap of {w}x{h}: {:?}", logs());
            }
            let (m1, r1) = mip_chains();
            last = Call {
                made: m1 - m0,
                refused: r1 - r0,
                kicks: fence() - f0,
                in_backend: ctx().objs_ref().textures[tex as usize].chain_in_backend,
            };
            draw(sample);
            // SAFETY: plain GL calls.
            unsafe {
                glBindTexture(GL_TEXTURE_2D, 0);
                glFinish();
            }
        }
        // SAFETY: a name this test made.
        unsafe { glDeleteFramebuffers(1, &fbo) };
        (tex, last)
    }

    /// The chain the PowerVR makes for a rendered-into RGBA8 texture: every
    /// frame's glGenerateMipmap is taken by the backend (one made chain, none
    /// refused), one kick per level above 0 (`gpu_levels`; the CPU tail of
    /// src/pvr/backend.rs MIP_CPU_TAIL_TEXELS is off by default, so every
    /// level), and the GL core filters nothing after them. The levels keep what the
    /// storage held (the kicks write them on the board; the stand-in GPU
    /// draws nothing), the tail filtered from such a uniform level is that
    /// same value, and level 0 is the render's.
    fn pvr_makes_the_chain(w: i32, h: i32, gpu_levels: i64) {
        let _g = rig();
        let levels = 32 - (w.max(h) as u32).leading_zeros();
        let (tex, call) = chain_frames(w, h, 3);
        assert_eq!((call.made, call.refused), (1, 0), "{w}x{h}: the backend took the chain: {:?}", logs());
        assert_eq!(call.kicks, gpu_levels, "{w}x{h}: one kick per large level above 0");
        let whole = gpu_levels == i64::from(levels) - 1;
        assert_eq!(call.in_backend, whole, "{w}x{h}: a whole chain lives in the backend's hands, a prefix not");
        let t = &ctx().objs_ref().textures[tex as usize];
        assert_eq!(u32::from(t.chain_levels), levels, "{w}x{h}: GL's whole chain");
        for l in 1..levels {
            assert!(level_rows(tex, l).iter().all(|&b| b == 0xA5), "{w}x{h}: level {l}");
        }
        assert_eq!(level_rows(tex, 0), common::level0(3, w as u32, h as u32), "{w}x{h}: level 0 is the render's");
        /* the CPU writes the texture: the backend hands the chain back, which
         * for the PowerVR (its kicks wrote the storage) is a wait */
        // SAFETY: plain GL calls on a live texture; one texel rewritten as it was.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex);
            let px = common::texel(3, 0, 0);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindTexture(GL_TEXTURE_2D, 0);
        }
        assert!(!ctx().objs_ref().textures[tex as usize].chain_in_backend, "{w}x{h}: the chain came back");
        assert_eq!(mip_chains().1, 0, "{w}x{h}: nothing was refused on the way");
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }

    #[test]
    fn a_square_power_of_two_target_gets_its_chain_from_the_gpu() {
        pvr_makes_the_chain(64, 64, 6);
        pvr_makes_the_chain(256, 256, 8);
        pvr_makes_the_chain(2, 2, 1);
    }

    #[test]
    fn a_rectangular_power_of_two_target_gets_its_chain_from_the_gpu() {
        pvr_makes_the_chain(64, 32, 6);
    }

    /// glmark2 terrain's target is 800x600: a NPOT texture whose chain is laid
    /// out as its power-of-two extent (the PowerVR's `pot_chain`): its nine
    /// levels above 0 on the GPU.
    #[test]
    fn glmark2_terrains_800x600_target_gets_its_chain_from_the_gpu() {
        pvr_makes_the_chain(800, 600, 9);
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
    use std::sync::atomic::{AtomicU32, AtomicU64, AtomicU8, Ordering};
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
    use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf, OsalV3dTfu, OsalV3dTfuQueue};
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

    /// The "GPU": every job completes inside its submit (as src/osal/linux.c),
    /// and a job into the target the test renders into stores its picture.
    unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
        if len != size_of::<V3dKick>() {
            return -22;
        }
        // SAFETY: the backend's live descriptor of that size.
        let k = unsafe { *(kick as *const V3dKick) };
        let fence = with_platform(|p| {
            p.kicks.push(k);
            p.kicks.len()
        });
        let job = jobs().get(fence - 1).cloned().expect("the job this kick runs was begun and ended");
        store_target(&job.target);
        fence as i64
    }

    unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
        0
    }

    unsafe extern "C" fn p_completed() -> i64 {
        with_platform(|p| p.kicks.len() as i64)
    }

    unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
        // SAFETY: the backend's query arguments, passed on.
        if let Some(rc) = unsafe { tfu_query(what, out, out_sz) } {
            return rc;
        }
        use yakogl::osal::{
            OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_VPM_BYTES,
        };
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

    /* ---- the TFU, as src/osal/linux.c models it ------------------------------------ */

    /// How the platform answers the TFU queries.
    #[derive(Clone, Copy, PartialEq, Eq, Debug)]
    enum Tfu {
        /// The default (`YAKOGL_V3D_TFU` unset): -ENOTSUP, the back end never
        /// asks for a job.
        Off,
        /// `YAKOGL_V3D_TFU=1`: jobs run, as linux.c's model runs them.
        On,
        /// The unit is there and every job fails.
        Failing,
    }

    static MODE: AtomicU8 = AtomicU8::new(0);

    fn mode() -> Tfu {
        [Tfu::Off, Tfu::On, Tfu::Failing][usize::from(MODE.load(Ordering::SeqCst))]
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

    /// One TFU mipmap job, as linux.c's `tfu_run` runs it: the job words are
    /// checked against the packer's own for the layout they name, then levels
    /// 1.. of the image are made from its level 0 with the 8-bit box filter.
    fn tfu_run(q: &OsalV3dTfu) -> i32 {
        let (w, h) = (q.ios & 0xffff, q.ios >> 16);
        let levels = ((q.icfg >> 5) & 0xf) + 1;
        let level0 = q.ioa & !0x3f;
        let Some(l) = rt_layout(w, h, levels) else { return -22 };
        let Some(base) = level0.checked_sub(l.slices[0].offset) else { return -22 };
        let mut j = V3dTexTfuJob::default();
        // SAFETY: a live layout and job.
        if levels < 2 || !unsafe { v3d_tex_tfu_pack_mipmap(&l, V3D_TFU_TTYPE_RGBA8, base, &mut j) } {
            return -22;
        }
        let words = [j.icfg, j.iia, j.iis, j.ica, j.iua, j.ioa, j.ios, j.coef0];
        if words != [q.icfg, q.iia, q.iis, q.ica, q.iua, q.ioa, q.ios, q.coef0] {
            return -22;
        }
        with_platform(|p| {
            let image = platform_cpu(p, u64::from(base), l.size as usize);
            let mut rows = vec![0u8; (w * h * 4) as usize];
            // SAFETY: the image's live mapping holds the laid-out chain; `rows` is level 0's size.
            assert!(unsafe { v3d_tex_load_box(image, &l, 0, 0, 0, w, h, rows.as_mut_ptr(), w * 4) });
            let (mut lw, mut lh) = (w, h);
            for level in 1..levels {
                rows = common::box_level(&rows, lw, lh);
                (lw, lh) = ((lw / 2).max(1), (lh / 2).max(1));
                // SAFETY: as above; `rows` is the level's rows.
                unsafe { v3d_tex_store_level(image, &l, level, rows.as_ptr(), lw * 4) };
            }
        });
        0
    }

    /// The TFU's answers to the back end's queries (`OSAL_QUERY_V3D_TFU_MIN_BYTES`
    /// at init, `OSAL_QUERY_V3D_TFU_RUN` or `OSAL_QUERY_V3D_TFU_QUEUE` for a
    /// job); `None` for any other query.
    unsafe fn tfu_query(what: u32, out: *mut c_void, out_sz: usize) -> Option<i32> {
        use yakogl::osal::{OSAL_QUERY_V3D_TFU_MIN_BYTES, OSAL_QUERY_V3D_TFU_QUEUE, OSAL_QUERY_V3D_TFU_RUN};
        match what {
            OSAL_QUERY_V3D_TFU_MIN_BYTES if out_sz == 4 => Some(if mode() == Tfu::Off {
                -95
            } else {
                // SAFETY: a u32 out-parameter.
                unsafe { *(out as *mut u32) = 4096 };
                0
            }),
            OSAL_QUERY_V3D_TFU_RUN => Some(if mode() == Tfu::Off {
                -95
            } else if out_sz != size_of::<OsalV3dTfu>() {
                -22
            } else {
                // SAFETY: the backend's in/out struct of that size.
                let q = unsafe { &mut *(out as *mut OsalV3dTfu) };
                q.rc = if mode() == Tfu::Failing { -5 } else { tfu_run(q) };
                0
            }),
            /* every submit completed inside p_submit: the job the TFU job
             * runs after is done, so it runs now, as linux.c runs it */
            OSAL_QUERY_V3D_TFU_QUEUE => Some(if mode() == Tfu::Off {
                -95
            } else if out_sz != size_of::<OsalV3dTfuQueue>() {
                -22
            } else {
                // SAFETY: the backend's in/out struct of that size.
                let q = unsafe { &mut *(out as *mut OsalV3dTfuQueue) };
                if q.after < 0 || q.after as usize > with_platform(|p| p.kicks.len()) {
                    return Some(-22);
                }
                q.fence = 0;
                q.tfu.rc = if mode() == Tfu::Failing { -5 } else { tfu_run(&q.tfu) };
                0
            }),
            _ => None,
        }
    }

    /// The render pass's store, which the stand-in GPU does for the target
    /// the test is rendering into (`TARGET`): the picture of frame `PHASE`
    /// into the raster storage the job writes, and into the render image
    /// (`uif_address`) in its layout, as the tile list stores both.
    fn store_target(t: &V3dJobTarget) {
        let (w, h) = (t.width, t.height);
        if t.address == 0 || TARGET.load(Ordering::SeqCst) != (u64::from(w) << 32 | u64::from(h)) {
            return;
        }
        let rows = common::level0(PHASE.load(Ordering::SeqCst), w, h);
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
    }

    /* ---- the context ------------------------------------------------------------- */

    const CANVAS: (i32, i32) = (40, 30);

    static LOCK: Mutex<()> = Mutex::new(());
    static TARGET: AtomicU64 = AtomicU64::new(0);
    static PHASE: AtomicU32 = AtomicU32::new(0);
    static mut CTX: *mut GlContext = core::ptr::null_mut();

    /// A back end started on a platform that answers the TFU queries as
    /// `tfu` says, and a context on it; torn down when dropped.
    struct Rig {
        _g: MutexGuard<'static, ()>,
    }

    fn rig(tfu: Tfu) -> Rig {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        MODE.store(tfu as u8, Ordering::SeqCst);
        /* a fresh back end counts kicks from 1; memory the previous one left
         * allocated stays (the GL core may still point at it) */
        with_platform(|p| {
            p.kicks.clear();
            p.log.clear();
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
    /// picture); the scene's samples unit 0.
    const FILL_VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    const FILL_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
    const SAMPLE_VS: &str = "attribute vec4 a_position;\nvarying vec2 v_coord;\n\
        void main() { v_coord = a_position.xy * 0.5 + 0.5; gl_Position = a_position; }\n";
    const SAMPLE_FS: &str = "precision mediump float;\nuniform sampler2D Tex;\nvarying vec2 v_coord;\n\
        void main() { gl_FragColor = texture2D(Tex, v_coord); }\n";

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

    /// The render images' counters since init: (chains the TFU made, samples
    /// read from an image, chains handed back to the GL core's storage).
    fn rt_counters() -> (u32, u32, u32) {
        let (mut chains, mut direct, mut back) = (0u32, 0u32, 0u32);
        // SAFETY: live out-parameters.
        unsafe {
            v3d_rt_images(
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut direct,
                core::ptr::null_mut(),
                core::ptr::null_mut(),
                &mut chains,
                &mut back,
            )
        };
        (chains, direct, back)
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

    /// What the last frame's glGenerateMipmap and the sample after it came to.
    struct Frame {
        /// Chains the back end made (a TFU job) in the call.
        made: u32,
        /// Samples the scene's draw read from the image.
        direct: u32,
        /// The GL core left the chain to the backend (`chain_in_backend`).
        in_backend: bool,
    }

    /// glmark2's frame, `frames` times: the render pass into the texture (the
    /// job is still open when glGenerateMipmap comes, as on the board), then
    /// glGenerateMipmap on it, then the scene samples it. The texture, and the
    /// last frame.
    fn frames(w: i32, h: i32, frames: u32) -> (GLuint, Frame) {
        let (fill, sample) = (program(FILL_VS, FILL_FS), program(SAMPLE_VS, SAMPLE_FS));
        let name = CString::new("Tex").unwrap();
        let (tex, fbo) = render_target(w, h);
        TARGET.store(u64::from(w as u32) << 32 | u64::from(h as u32), Ordering::SeqCst);
        let mut last = Frame { made: 0, direct: 0, in_backend: false };
        for phase in 1..=frames {
            PHASE.store(phase, Ordering::SeqCst);
            // SAFETY: plain GL calls on live names.
            unsafe {
                glUseProgram(fill);
                glBindFramebuffer(GL_FRAMEBUFFER, fbo);
                glViewport(0, 0, w, h);
            }
            draw_quad();
            let (c0, d0, _) = rt_counters();
            // SAFETY: plain GL calls on live names.
            unsafe {
                glBindFramebuffer(GL_FRAMEBUFFER, 0);
                glViewport(0, 0, CANVAS.0, CANVAS.1);
                glActiveTexture(GL_TEXTURE0);
                glBindTexture(GL_TEXTURE_2D, tex);
                glGenerateMipmap(GL_TEXTURE_2D);
                assert_eq!(glGetError(), GL_NO_ERROR, "glGenerateMipmap of {w}x{h}: {:?}", logs());
            }
            let (c1, _, _) = rt_counters();
            let in_backend = ctx().objs_ref().textures[tex as usize].chain_in_backend;
            // SAFETY: plain GL calls on live names.
            unsafe {
                glUseProgram(sample);
                glUniform1i(glGetUniformLocation(sample, name.as_ptr()), 0);
            }
            draw_quad();
            // SAFETY: plain GL call.
            unsafe { glFinish() };
            let (_, d1, _) = rt_counters();
            last = Frame { made: c1 - c0, direct: d1 - d0, in_backend };
        }
        // SAFETY: a name this test made.
        unsafe { glDeleteFramebuffers(1, &fbo) };
        (tex, last)
    }

    /// Every level of `tex` is the box filter of frame `phase`'s render, within
    /// 1 a channel, however it was made. A CPU write of one texel as it was
    /// first, which hands a chain the back end holds back to the storage.
    fn assert_chain(tex: GLuint, phase: u32, what: &str) {
        let (w, h) = {
            let t = &ctx().objs_ref().textures[tex as usize];
            (t.width, t.height)
        };
        // SAFETY: plain GL calls on a live texture; one texel rewritten as it was.
        unsafe {
            glBindTexture(GL_TEXTURE_2D, tex);
            let px = common::texel(phase, 0, 0);
            glTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 1, 1, GL_RGBA, GL_UNSIGNED_BYTE, px.as_ptr() as *const c_void);
            assert_eq!(glGetError(), GL_NO_ERROR, "{what}: {:?}", logs());
            glBindTexture(GL_TEXTURE_2D, 0);
        }
        let chain = common::chain(phase, w, h);
        let t = &ctx().objs_ref().textures[tex as usize];
        assert!(!t.chain_in_backend, "{what}: the chain is in the storage");
        assert_eq!(u32::from(t.chain_levels) as usize, chain.len(), "{what}: GL's whole chain");
        for (l, want) in chain.iter().enumerate() {
            let d = common::max_diff(&level_rows(tex, l as u32), want);
            assert!(d <= 1, "{what}: level {l} of {w}x{h} differs from the box filter by {d}: {:?}", logs());
        }
    }

    /// glmark2's target, four frames: the chain of the last frame's render.
    /// The path is the TFU's when the OSAL has a working one: one made chain
    /// in the frame's glGenerateMipmap, the GL core filtering nothing
    /// (`chain_in_backend`), the scene's sample read from the image; and the
    /// GL core's CPU filter otherwise (no TFU, or a failing one): no chain made.
    fn chain_of(tfu: Tfu, w: i32, h: i32) {
        let _r = rig(tfu);
        let (tex, f) = frames(w, h, 4);
        let what = format!("{tfu:?} {w}x{h}");
        match tfu {
            Tfu::On => {
                assert_eq!((f.made, f.direct, f.in_backend), (1, 1, true), "{what}: the TFU made the chain");
                assert_eq!(rt_counters().2, 0, "{what}: nothing came back to the storage yet");
            }
            Tfu::Off | Tfu::Failing => {
                assert_eq!((f.made, f.direct, f.in_backend), (0, 0, false), "{what}: the GL core filtered it");
                assert_eq!(rt_counters().0, 0, "{what}: no chain was ever made");
            }
        }
        assert_chain(tex, 4, &what);
        if tfu == Tfu::On {
            assert_eq!(rt_counters().2, 1, "{what}: the chain went back into the storage once");
        }
        // SAFETY: a name this test made.
        unsafe { glDeleteTextures(1, &tex) };
    }

    #[test]
    fn the_default_tfu_setting_filters_the_chain_on_the_cpu() {
        chain_of(Tfu::Off, 64, 64);
        chain_of(Tfu::Off, 64, 32);
    }

    #[test]
    fn with_the_tfu_on_the_chain_is_made_from_the_render_image() {
        chain_of(Tfu::On, 64, 64);
        chain_of(Tfu::On, 64, 32);
    }

    #[test]
    fn a_failing_tfu_leaves_the_chain_to_the_cpu() {
        chain_of(Tfu::Failing, 64, 64);
    }

    /// glmark2 terrain's target is 800x600, refract's 1600x1200 halved by the
    /// scene's own scale: NPOT textures.
    #[test]
    fn glmark2_terrains_800x600_target_with_the_default_tfu_setting() {
        chain_of(Tfu::Off, 800, 600);
    }

    #[test]
    fn glmark2_terrains_800x600_target_with_the_tfu_on() {
        chain_of(Tfu::On, 800, 600);
    }
}
