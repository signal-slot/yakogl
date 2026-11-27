// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The edges of the PowerVR's CPU depth and stencil resolves
//! (`pvr_backend_depth_resolve`, `pvr_backend_stencil_resolve`) that
//! tests/depth_resolve_pvr_rows.rs leaves out, pinned through the back end's
//! entry points so the rewrite to whole rows (docs/notes/pvr-refract-draw.md,
//! "Ranked fixes" 2) keeps every return code:
//! * 1, with the destination left as it was, for a back end not inited, id 0,
//!   a null dst, `w > pitch`, an id that holds nothing and a deleted target;
//! * -EBUSY while the open frame draws into the plane (both resolves, nothing
//!   written), and the rule once it is submitted;
//! * -ENOTSUP for a stencil kept apart (`split_zs_on`), the depth buffer
//!   beside it still converting by the rule;
//! * the multisampled layout, with `msaa-fbo`: a request of 2 or 3 samples is
//!   stored as 4x (`msaa::stored_samples`), and its depth and stencil are
//!   sample 0 (the word at (2x, 2y) of rows twice the ZLS-tile-aligned
//!   width), bit for bit against `ds_retention::unorm24_depth` and
//!   `word >> 24`; without it (the PowerVR's `max_samples` 0), 2 to 4 samples
//!   are GL_INVALID_OPERATION and 1 sample resolves by the one-sample rule.
//!
//! Not reachable from tests/, left to src/pvr/backend_tests.rs:
//! * the 2x layout (pixel (x, y) reads word (x, 2y); a row is
//!   ceil(w / ZLS_TILE) times ZLS_TILE words): the GL core stores 4 samples
//!   for every request of 1..=4 on this back end with `msaa-fbo`
//!   (`max_samples` 4; 1 sample without it, `max_samples` 0), so no draw
//!   names a 2x buffer; build a `DsBuf` with `samples` 2 in the unit test;
//! * -EINVAL from `ds_sample0` (`ds_bytes(w, h, samples)` > `mem.size`): every
//!   buffer the back end allocates is `ds_bytes` long; shrink `mem.size` of a
//!   live slot in the unit test;
//! * the return of 1 for a live plane whose buffer is not `valid` or has no
//!   CPU view (`mem.cpu` null): a swap that drops the depth/stencil
//!   (`swap_drops_zs`) clears `valid` together with the planes (so the plane
//!   is no longer live and the path before answers 1), a kick that makes a
//!   plane live sets `valid`, and the platform here maps all GPU memory; set
//!   `valid` false or `mem.cpu` null on a live slot in the unit test.
//!
//! The platform is that of tests/depth_resolve_pvr_rows.rs (copied): GPU
//! memory is host memory, every submit completes at once, and the "GPU"
//! stores [`zls_word`]`(fence, i)` into word i of the depth/stencil buffer of
//! every kick that stores it. The sentinel marks a texel no resolve wrote.
//!
//! The cases run in ONE test, in order: the back end not inited has to be
//! tried before the file's rig inits it (a test binary shares one back end,
//! and tests run in any order).
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-powervr,shaderc-online --test depth_resolve_pvr_edges` (and
//! with `,report-es2` for the stencil kept apart, and with `,msaa-fbo` for
//! the 4x layout).

/// The packed D24S8 word kick `fence` stores at word `i` of its
/// depth/stencil buffer: six extremes first (depth 0, all ones, the top bit
/// alone, all but the top bit; stencil 0 and 0xff), then a hash of both, so
/// every depth bit and every stencil bit is set in some words and clear in
/// others, and a texel taken from the wrong word or kick shows.
fn zls_word(fence: i64, i: usize) -> u32 {
    match i {
        0 => 0x0000_0000,
        1 => 0xff00_0001,
        2 => 0xffff_ffff,
        3 => 0x0080_0000,
        4 => 0x007f_ffff,
        5 => 0xff00_0000,
        _ => {
            let mut x = (i as u32).wrapping_mul(0x9e37_79b1) ^ (fence as u32).wrapping_mul(0x85eb_ca6b);
            x ^= x >> 15;
            x = x.wrapping_mul(0x2c1b_3c6d);
            x ^= x >> 12;
            x = x.wrapping_mul(0x297a_2d39);
            x ^ (x >> 15)
        }
    }
}

/// An R32F the resolve never writes (a NaN `unorm24_depth` cannot make).
const SENTINEL: u32 = 0x7fba_d00d;

/// Without the PowerVR's host-test build the file still has a test, so no
/// row of the gate runs a binary with none (nextest exits 4).
#[test]
fn the_sentinel_is_no_depth_the_rule_makes() {
    for fence in 1..=64i64 {
        for i in 0..512 {
            let d = yakogl::ds_retention::unorm24_depth(zls_word(fence, i));
            assert_ne!(d.to_bits(), SENTINEL);
        }
    }
}

/// The real back end: the build of the PowerVR host tests.
#[cfg(all(
    feature = "host-test",
    feature = "gpu-powervr",
    not(feature = "no-shader-table"),
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the stand-ins take the C generators' parameter lists as they are */
#[allow(clippy::too_many_arguments)]
mod backend {
    use super::{zls_word, SENTINEL};

    use core::ffi::{c_char, c_void, CStr};
    use std::collections::BTreeMap;
    use std::sync::{Mutex, MutexGuard, Once};

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
        /// Waited for through this fence.
        completed_to: i64,
        /// The fence of the last kick that stored a depth/stencil buffer.
        last_store: Option<i64>,
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

    /// The "GPU": every submit completes at once, and a kick that stores its
    /// depth/stencil leaves `zls_word(fence, i)` in word i of the buffer, from
    /// its base to the allocation's end.
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
                let first = (a.mem.as_mut_ptr() as *mut u8).wrapping_add(off) as *mut u32;
                // SAFETY: the words from `zs` to the allocation's end.
                let words = unsafe { core::slice::from_raw_parts_mut(first, (a.size - off) / 4) };
                for (i, w) in words.iter_mut().enumerate() {
                    *w = zls_word(fence, i);
                }
                p.last_store = Some(fence);
            }
            fence
        })
    }

    unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
        with_platform(|p| p.completed_to = p.completed_to.max(fence));
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

    const CANVAS: (u32, u32) = (40, 30);

    static LOCK: Mutex<()> = Mutex::new(());
    static ONCE: Once = Once::new();
    static mut PROGRAM: GLuint = 0;

    /// The one context and backend of the file, set up once; the tests take
    /// turns. Each starts with every kick submitted and completed.
    fn rig() -> MutexGuard<'static, ()> {
        let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
        ONCE.call_once(|| {
            // SAFETY: statics; the lock serialises every GL call of the file.
            unsafe {
                osal_set_ops(osal());
                yakogl::backend::g_backend = &yakogl_backend_pvr;
                assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
                let ctx = context::gl_context_create(CANVAS.0, CANVAS.1);
                assert!(!ctx.is_null(), "{:?}", logs());
                PROGRAM = program("passthrough.vert", "uniform_color.frag");
            }
        });
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            while glGetError() != GL_NO_ERROR {}
        }
        with_platform(|p| {
            p.completed_to = p.fence;
            p.log.clear();
        });
        g
    }

    fn ctx() -> &'static GlContext {
        /* the context `rig` made, current, never freed; read under the lock */
        context::current().expect("no current context")
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

    /* a triangle at z = 0 covering the whole target, (x, y, z, w) per vertex */
    static POS: [f32; 12] = [-1.0, -1.0, 0.0, 1.0, 3.0, -1.0, 0.0, 1.0, -1.0, 3.0, 0.0, 1.0];

    fn draw() {
        // SAFETY: plain GL calls; the array is a static.
        unsafe {
            glUseProgram(PROGRAM);
            glVertexAttribPointer(0, 4, GL_FLOAT, 0, 0, POS.as_ptr() as *const c_void);
            glEnableVertexAttribArray(0);
            glDrawArrays(GL_TRIANGLES, 0, 3);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }

    /* ---- the targets ------------------------------------------------------------- */

    /// What a target's depth and stencil are attached as.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Kind {
        /// One GL_DEPTH24_STENCIL8 renderbuffer at both points: a packed buffer.
        Packed,
        /// A GL_DEPTH_COMPONENT texture only: a depth buffer of its own.
        DepthTexture,
        /// A depth renderbuffer and a GL_STENCIL_INDEX8 one: each its own buffer.
        Separate,
    }

    /// A framebuffer object with a depth/stencil the back end keeps.
    #[derive(Debug)]
    struct Target {
        kind: Kind,
        fbo: GLuint,
        /// The renderbuffers (or the texture in `[0]`) to delete.
        names: [GLuint; 2],
        /// The back end's object ids of the depth and the stencil plane.
        depth: u32,
        stencil: u32,
        w: u32,
        h: u32,
        /// Samples of the buffer (0 or 1: one).
        samples: u32,
    }

    fn renderbuffer(format: GLenum, w: u32, h: u32, samples: i32) -> (GLuint, u32, u32) {
        let mut rb = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            if samples > 0 {
                glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, format, w as i32, h as i32);
            } else {
                glRenderbufferStorage(GL_RENDERBUFFER, format, w as i32, h as i32);
            }
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
            glBindRenderbuffer(GL_RENDERBUFFER, 0);
        }
        let r = &ctx().objs_ref().renderbuffers[rb as usize];
        (rb, context::ds_object_id(r.serial, false), r.samples as u32)
    }

    fn target(kind: Kind, w: u32, h: u32, samples: i32) -> Target {
        let mut fbo = 0;
        // SAFETY: room for one name; plain GL calls.
        unsafe {
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        }
        let t = match kind {
            Kind::Packed => {
                let (rb, id, s) = renderbuffer(GL_DEPTH24_STENCIL8, w, h, samples);
                // SAFETY: plain GL calls.
                unsafe {
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
                }
                Target { kind, fbo, names: [rb, 0], depth: id, stencil: id, w, h, samples: s }
            }
            Kind::Separate => {
                let (d, did, _) = renderbuffer(GL_DEPTH_COMPONENT16, w, h, 0);
                let (s, sid, _) = renderbuffer(GL_STENCIL_INDEX8, w, h, 0);
                // SAFETY: plain GL calls.
                unsafe {
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, d);
                    glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, s);
                }
                Target { kind, fbo, names: [d, s], depth: did, stencil: sid, w, h, samples: 0 }
            }
            Kind::DepthTexture => {
                let mut tex = 0;
                // SAFETY: room for one name; a null image.
                unsafe {
                    glGenTextures(1, &mut tex);
                    glBindTexture(GL_TEXTURE_2D, tex);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
                    glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        0,
                        (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
                        w as i32,
                        h as i32,
                        0,
                        GL_DEPTH_COMPONENT,
                        GL_UNSIGNED_INT,
                        core::ptr::null(),
                    );
                    glBindTexture(GL_TEXTURE_2D, 0);
                    glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
                }
                let id = context::ds_object_id(ctx().objs_ref().textures[tex as usize].serial, true);
                Target { kind, fbo, names: [tex, 0], depth: id, stencil: 0, w, h, samples: 0 }
            }
        };
        // SAFETY: plain GL calls.
        unsafe {
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{t:?}");
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        t
    }

    fn release(t: Target) {
        // SAFETY: names this test made.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glDeleteFramebuffers(1, &t.fbo);
            if t.kind == Kind::DepthTexture {
                glDeleteTextures(1, &t.names[0]);
            } else {
                for n in t.names.iter().filter(|&&n| n != 0) {
                    glDeleteRenderbuffers(1, n);
                }
            }
            glFinish();
        }
    }

    /// Clear `t`'s depth to `depth` and stencil to `stencil`, then draw one
    /// triangle over it: with `write`, the depth test (GL_ALWAYS) and the
    /// stencil test (GL_REPLACE) on, so both planes are live and the kick
    /// stores them; without, both tests off, so the planes hold the clear only.
    /// Left open (`finish` false) the frame still draws into the buffer and 0
    /// is returned; else the fence of the last kick that stored a buffer.
    fn render(t: &Target, depth: f32, stencil: i32, write: bool, finish: bool) -> i64 {
        with_platform(|p| p.last_store = None);
        // SAFETY: plain GL calls.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, t.fbo);
            glViewport(0, 0, t.w as i32, t.h as i32);
            glColorMask(0, 0, 0, 0);
            glDepthMask(1);
            glStencilMask(0xff);
            glClearDepthf(depth);
            glClearStencil(stencil);
            glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            if write {
                glEnable(GL_DEPTH_TEST);
                glDepthFunc(GL_ALWAYS);
                glEnable(GL_STENCIL_TEST);
                glStencilFunc(GL_ALWAYS, 0x5a, 0xff);
                glStencilOp(GL_REPLACE, GL_REPLACE, GL_REPLACE);
            } else {
                glDisable(GL_DEPTH_TEST);
                glDisable(GL_STENCIL_TEST);
            }
        }
        draw();
        if !finish {
            return 0;
        }
        // SAFETY: plain GL calls.
        unsafe {
            glFinish();
            glDisable(GL_DEPTH_TEST);
            glDisable(GL_STENCIL_TEST);
            glColorMask(1, 1, 1, 1);
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0 as i32, CANVAS.1 as i32);
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
        with_platform(|p| p.last_store).unwrap_or(0)
    }

    /// A rendered target and the fence whose [`zls_word`]s its buffer holds.
    fn rendered(kind: Kind, w: u32, h: u32, samples: i32) -> (Target, i64) {
        let t = target(kind, w, h, samples);
        let fence = render(&t, 1.0, 0, true, true);
        assert_ne!(fence, 0, "no kick stored {t:?}: {:?}", logs());
        (t, fence)
    }

    /* ---- the rule ---------------------------------------------------------------- */

    /// The word pixel (x, y) of a one-sample or 4x (2 by 2) buffer reads, as
    /// the rows are written: linear at the ZLS-tile-aligned width.
    fn word_at(t: &Target, fence: i64, x: u32, y: u32) -> u32 {
        let s = if t.samples == 4 { 2 } else { 1 };
        let row = t.w.div_ceil(32) as usize * 32 * s;
        zls_word(fence, y as usize * s * row + x as usize * s)
    }

    fn resolve_depth(id: u32, w: u32, h: u32, pitch: u32) -> (i32, Vec<u32>) {
        let mut dst = vec![f32::from_bits(SENTINEL); h as usize * pitch as usize];
        // SAFETY: `dst` holds h rows of pitch floats.
        let rc = unsafe { pvr_backend_depth_resolve(id, dst.as_mut_ptr(), w, h, pitch) };
        (rc, dst.into_iter().map(f32::to_bits).collect())
    }

    fn resolve_stencil(id: u32, w: u32, h: u32, pitch: u32) -> (i32, Vec<u8>) {
        let mut dst = vec![0xa5u8; h as usize * pitch as usize];
        // SAFETY: `dst` holds h rows of pitch bytes.
        let rc = unsafe { pvr_backend_stencil_resolve(id, dst.as_mut_ptr(), w, h, pitch) };
        (rc, dst)
    }

    /// A resolve of the whole `t` (pitch w + 3, padding untouched) is the rule.
    fn check_rule(t: &Target, fence: i64, stencil: bool) {
        let (w, h, pitch) = (t.w, t.h, t.w + 3);
        let (rc, d) = resolve_depth(t.depth, w, h, pitch);
        assert_eq!(rc, 0, "{t:?}: {:?}", logs());
        for y in 0..h {
            for x in 0..pitch {
                let want = if x < w {
                    yakogl::ds_retention::unorm24_depth(word_at(t, fence, x, y)).to_bits()
                } else {
                    SENTINEL
                };
                assert_eq!(d[(y * pitch + x) as usize], want, "{t:?}: depth texel ({x}, {y})");
            }
        }
        if !stencil {
            return;
        }
        let (rc, s) = resolve_stencil(t.stencil, w, h, pitch);
        assert_eq!(rc, 0, "{t:?}: {:?}", logs());
        for y in 0..h {
            for x in 0..pitch {
                let want = if x < w { (word_at(t, fence, x, y) >> 24) as u8 } else { 0xa5 };
                assert_eq!(s[(y * pitch + x) as usize], want, "{t:?}: stencil texel ({x}, {y})");
            }
        }
    }

    /* ---- the cases --------------------------------------------------------------- */

    /// 1 and nothing written, for every refusal and for every id of the back
    /// end not inited (`not_inited`: called before the rig inits it).
    fn refusals(depth: u32, stencil: u32, unknown: u32) {
        let mut f = [f32::from_bits(SENTINEL); 8];
        let mut s = [0xa5u8; 8];
        // SAFETY: each call names at most 2 rows of 4 elements of an 8-element buffer, or a null one.
        unsafe {
            for (what, id) in [("id 0", 0), ("nothing held", unknown)] {
                assert_eq!(pvr_backend_depth_resolve(id, f.as_mut_ptr(), 4, 2, 4), 1, "{what}");
                assert_eq!(pvr_backend_stencil_resolve(id, s.as_mut_ptr(), 4, 2, 4), 1, "{what}");
            }
            assert_eq!(pvr_backend_depth_resolve(depth, core::ptr::null_mut(), 4, 2, 4), 1, "null dst");
            assert_eq!(pvr_backend_stencil_resolve(stencil, core::ptr::null_mut(), 4, 2, 4), 1, "null dst");
            assert_eq!(pvr_backend_depth_resolve(depth, f.as_mut_ptr(), 5, 1, 4), 1, "w > pitch");
            assert_eq!(pvr_backend_stencil_resolve(stencil, s.as_mut_ptr(), 5, 1, 4), 1, "w > pitch");
            assert_eq!(pvr_backend_depth_resolve(depth, f.as_mut_ptr(), 1, 1, 0), 1, "w > pitch 0");
            assert_eq!(pvr_backend_stencil_resolve(stencil, s.as_mut_ptr(), 1, 1, 0), 1, "w > pitch 0");
        }
        assert!(f.iter().all(|v| v.to_bits() == SENTINEL), "a refused depth resolve wrote {f:?}");
        assert_eq!(s, [0xa5; 8], "a refused stencil resolve wrote");
    }

    fn not_inited() {
        // The platform is untouched: nothing of the back end exists yet.
        let id = context::ds_object_id(1, false);
        refusals(id, id, id);
        /* with all arguments good, too: the back end answers 1 before it reads a thing */
        assert_eq!(resolve_depth(id, 4, 2, 4), (1, vec![SENTINEL; 8]));
        assert_eq!(resolve_stencil(id, 4, 2, 4), (1, vec![0xa5; 8]));
    }

    fn bad_arguments_and_nothing_held() {
        let (t, fence) = rendered(Kind::Packed, 20, 5, 0);
        let unknown = context::ds_object_id(0x3fff_fff0, false);
        refusals(t.depth, t.stencil, unknown);
        /* the refusals left the buffer as it was */
        check_rule(&t, fence, true);
        let (depth, stencil) = (t.depth, t.stencil);
        release(t);
        assert_eq!(resolve_depth(depth, 20, 5, 20), (1, vec![SENTINEL; 100]), "deleted");
        assert_eq!(resolve_stencil(stencil, 20, 5, 20), (1, vec![0xa5; 100]), "deleted");
    }

    /// -EBUSY while the open frame draws into the plane, nothing written; the
    /// kick, once submitted, is what the next resolve reads.
    fn busy() {
        let (t, fence) = rendered(Kind::Packed, 33, 7, 0);
        render(&t, 1.0, 0, true, false);
        assert_eq!(resolve_depth(t.depth, 33, 7, 33), (-16, vec![SENTINEL; 231]), "{:?}", logs());
        assert_eq!(resolve_stencil(t.stencil, 33, 7, 33), (-16, vec![0xa5; 231]), "{:?}", logs());
        /* a depth texture's own depth buffer is just as busy */
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        let fence2 = with_platform(|p| p.last_store).expect("the frame stored");
        assert_ne!(fence2, fence);
        check_rule(&t, fence2, true);
        release(t);

        let t = target(Kind::DepthTexture, 40, 9, 0);
        render(&t, 1.0, 0, true, false);
        assert_eq!(resolve_depth(t.depth, 40, 9, 40), (-16, vec![SENTINEL; 360]), "{:?}", logs());
        // SAFETY: plain GL call.
        unsafe { glFinish() };
        let fence = with_platform(|p| p.last_store).expect("the frame stored");
        check_rule(&t, fence, false);
        release(t);
    }

    /// -ENOTSUP for a stencil buffer of its own; its depth buffer is the rule.
    /// ES 3.0 4.4.4.2 makes separate images GL_FRAMEBUFFER_UNSUPPORTED, so
    /// this runs with `report-es2` only.
    fn stencil_kept_apart() {
        if !cfg!(feature = "report-es2") {
            return;
        }
        let (t, fence) = rendered(Kind::Separate, 40, 6, 0);
        assert_eq!(resolve_stencil(t.stencil, 40, 6, 40), (-95, vec![0xa5; 240]), "{:?}", logs());
        check_rule(&t, fence, false);
        release(t);
    }

    /// A request of 2 or 3 samples is a 4x buffer: sample 0, the word at
    /// (2x, 2y) of rows twice the aligned width, for widths on and off the tile.
    /// The PowerVR keeps 4x framebuffer objects only with `msaa-fbo`.
    #[cfg(feature = "msaa-fbo")]
    fn multisample_requests() {
        /* an ES 2.0 report has no multisampled renderbuffers */
        if cfg!(feature = "report-es2") {
            return;
        }
        for samples in [2, 3] {
            for (w, h) in [(33, 5), (64, 3), (7, 40)] {
                let (t, fence) = rendered(Kind::Packed, w, h, samples);
                assert_eq!(t.samples, 4, "{samples} samples requested: {t:?}");
                check_rule(&t, fence, true);
                release(t);
            }
        }
    }

    /// Without `msaa-fbo` the PowerVR's `max_samples` is 0: GL_MAX_SAMPLES is
    /// 1, a request of 2 to 4 samples is GL_INVALID_OPERATION and stores
    /// nothing, and a request of 1 is a one-sample buffer that resolves by the
    /// one-sample rule (the word at (x, y)).
    #[cfg(not(feature = "msaa-fbo"))]
    fn multisample_requests() {
        /* an ES 2.0 report has no multisampled renderbuffers */
        if cfg!(feature = "report-es2") {
            return;
        }
        let mut max = -1;
        // SAFETY: one GLint.
        unsafe { glGetIntegerv(GL_MAX_SAMPLES, &mut max) };
        assert_eq!(max, 1);
        for samples in [2, 3, 4] {
            let mut rb = 0;
            // SAFETY: room for one name; plain GL calls.
            unsafe {
                glGenRenderbuffers(1, &mut rb);
                glBindRenderbuffer(GL_RENDERBUFFER, rb);
                glRenderbufferStorageMultisample(GL_RENDERBUFFER, samples, GL_DEPTH24_STENCIL8, 33, 5);
                assert_eq!(glGetError(), GL_INVALID_OPERATION, "{samples} samples");
                glBindRenderbuffer(GL_RENDERBUFFER, 0);
                glDeleteRenderbuffers(1, &rb);
            }
        }
        for (w, h) in [(33, 5), (64, 3), (7, 40)] {
            let (t, fence) = rendered(Kind::Packed, w, h, 1);
            assert_eq!(t.samples, 1, "one sample requested: {t:?}");
            check_rule(&t, fence, true);
            release(t);
        }
    }

    #[test]
    fn the_edges_of_the_resolves() {
        not_inited();
        let _g = rig();
        bad_arguments_and_nothing_held();
        busy();
        stencil_kept_apart();
        multisample_requests();
    }
}
