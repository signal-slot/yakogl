// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! Unit tests of the V3D backend's recording logic (jobs, their memory, the
//! clears, the depth/stencil history, the arrays and indices it stages, the
//! refusals), driven through its C entry points.
//!
//! # The job builder is a fake
//!
//! The backend calls the C job builder (`ffi.rs`, `driver/src/v3d/v3d_job.c`),
//! which a `cargo test` of this crate does not link (the crate has no
//! build.rs; the image build links the C). This module defines those symbols
//! itself: recording fakes that remember every draw the backend hands over
//! (its state, programs, arrays, uniform storage and indices) and every job's
//! clear, and answer OK, FULL or REFUSED as a test scripts them. The tests
//! check what the backend DECIDES and hands over; the real builder's lists
//! are checked byte for byte by `tools/v3d/cl_golden.c` and
//! `tools/v3d/job_check.c`, and on the board.
//!
//! # The platform is an OSAL of its own
//!
//! [`Rig`] installs an OSAL with 32-bit GPU addresses, a VPM query and a
//! submit that records each [`V3dKick`], while holding a
//! [`TestGl`](crate::testutil::TestGl) guard, which serialises it with every
//! other test that touches the process-global OSAL pointer and routes log
//! lines to its recorder.

use super::*;
use crate::osal::OsalOps;
use crate::shaderc::{ShcV3d, ShcV3dFs, ShcV3dUniform, ShcV3dVs};
use crate::testutil::TestGl;
use core::ffi::c_char;
use core::mem::size_of;
use std::collections::BTreeMap;
use std::sync::Mutex;
use std::vec::Vec;
use std::{boxed::Box, format, string::String, vec};

/* ---- the fake job builder ----------------------------------------------------- */

/// One draw as the backend handed it to `v3d_job_draw`, copied out of the
/// pointers (which only live for the call).
#[derive(Clone, Debug)]
struct SeenDraw {
    st: V3dJobState,
    fs_code: u32,
    vs_code: u32,
    cs_code: u32,
    fs_uniforms: Vec<V3dUniform>,
    vs_uniforms: Vec<V3dUniform>,
    vs_storage: Vec<u32>,
    fs_storage: Vec<u32>,
    attrs: Vec<V3dVertexAttr>,
    prim: u32,
    count: u32,
    indexed: bool,
    index_type: u32,
    index_buffer: u32,
    index_bytes: u32,
    tmu: Vec<V3dJobTmu>,
    vs_tmu: Vec<V3dJobTmu>,
    vs_writes_psiz: bool,
    instances: u32,
}

#[derive(Clone, Debug)]
struct SeenJob {
    target: V3dJobTarget,
    clear: Option<V3dJobClear>,
    draws: u32,
}

#[derive(Default)]
struct Builder {
    jobs: Vec<SeenJob>,
    draws: Vec<SeenDraw>,
    /// What the next `v3d_job_draw` calls answer, first first; OK when empty.
    answers: Vec<i32>,
    /// What went wrong inside a fake (it cannot panic across `extern "C"`).
    faults: Vec<String>,
}

static BUILDER: Mutex<Option<Builder>> = Mutex::new(None);

fn with_builder<R>(f: impl FnOnce(&mut Builder) -> R) -> R {
    let mut b = BUILDER.lock().unwrap_or_else(|e| e.into_inner());
    f(b.get_or_insert_with(Builder::default))
}

/// # Safety
/// `p` is null or points at `n` live elements.
unsafe fn copy_out<T: Copy>(p: *const T, n: u32) -> Vec<T> {
    if p.is_null() || n == 0 {
        return Vec::new();
    }
    // SAFETY: the caller's contract.
    unsafe { core::slice::from_raw_parts(p, n as usize) }.to_vec()
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
    /* the prologue: some bytes in the BCL, the defaults in the indirect buffer */
    // SAFETY: as above.
    unsafe {
        (*bcl).used = 16;
        (*ind).used = 256;
    }
    with_builder(|b| b.jobs.push(SeenJob { target: t, clear: None, draws: 0 }));
    t.width != 0 && t.height != 0
}

#[no_mangle]
unsafe extern "C" fn v3d_job_draw(j: *mut V3dJob, d: *const V3dJobDraw, layout: *mut c_void) -> i32 {
    // SAFETY: the backend passes its live job and a draw whose pointers live
    // for the call.
    let (j, d) = unsafe { (&mut *j, &*d) };
    let answer = with_builder(|b| if b.answers.is_empty() { V3D_JOB_OK } else { b.answers.remove(0) });
    if answer != V3D_JOB_OK {
        return answer;
    }
    // SAFETY: as above.
    let seen = unsafe {
        let (fs, vs, cs) = (&*d.fs, &*d.vs, &*d.cs);
        SeenDraw {
            st: *d.st,
            fs_code: fs.code,
            vs_code: vs.code,
            cs_code: cs.code,
            fs_uniforms: copy_out(fs.uniforms, fs.num_uniforms),
            vs_uniforms: copy_out(vs.uniforms, vs.num_uniforms),
            vs_storage: copy_out(d.vs_storage.words, d.vs_storage.count),
            fs_storage: copy_out(d.fs_storage.words, d.fs_storage.count),
            attrs: copy_out(d.attrs, d.num_attrs),
            prim: d.prim,
            count: d.count,
            indexed: d.indexed,
            index_type: d.index_type,
            index_buffer: d.index_buffer,
            index_bytes: d.index_buffer_bytes,
            tmu: copy_out(d.tmu, d.num_tmu),
            vs_tmu: copy_out(d.vs_tmu, d.num_vs_tmu),
            vs_writes_psiz: d.vs_writes_psiz,
            instances: d.instances,
        }
    };
    j.draws += 1;
    // SAFETY: the job's BCL is live.
    unsafe { (*j.bcl).used += 120 };
    /* the streams into the indirect buffer, as the real builder writes them,
     * and where they went into `layout`: a constant keeps its data, any
     * other word is 0x5a5a0000 | its index (the fake resolves nothing) */
    // SAFETY: the job's indirect buffer is live; the programs are the draw's.
    let addrs = unsafe {
        let ind = &mut *j.ind;
        let (fs, vs, cs) = (&*d.fs, &*d.vs, &*d.cs);
        [(fs.uniforms, fs.num_uniforms), (vs.uniforms, vs.num_uniforms), (cs.uniforms, cs.num_uniforms)].map(
            |(u, n)| {
                let at = ind.used.next_multiple_of(4);
                if n == 0 || u.is_null() || at + n * 4 > ind.size {
                    return 0;
                }
                for i in 0..n {
                    let w = *u.add(i as usize);
                    let v = if w.contents == 0 { w.data } else { 0x5a5a_0000 | i };
                    (ind.buf.add((at + i * 4) as usize) as *mut u32).write_unaligned(v);
                }
                ind.used = at + n * 4;
                ind.gpu + at
            },
        )
    };
    if !layout.is_null() {
        // SAFETY: the backend passes a live V3dJobDrawLayout.
        let lo = unsafe { &mut *(layout as *mut V3dJobDrawLayout) };
        (lo.fs_uniforms, lo.vs_uniforms, lo.cs_uniforms) = (addrs[0], addrs[1], addrs[2]);
    }
    with_builder(|b| {
        if let Some(job) = b.jobs.last_mut() {
            job.draws += 1;
        }
        b.draws.push(seen);
    });
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
    with_builder(|b| match b.jobs.last_mut() {
        Some(job) if job.clear.is_none() => job.clear = Some(c),
        _ => b.faults.push(String::from("v3d_job_end of a job that was not begun, or ended twice")),
    });
    true
}

#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling(width: u32, height: u32, t: *mut V3dTiling) {
    let tiles = V3dTiling {
        tile_w: 64,
        tile_h: 64,
        tiles_x: width.div_ceil(64),
        tiles_y: height.div_ceil(64),
        supertile_w: 1,
        supertile_h: 1,
        frame_w_supertiles: width.div_ceil(64),
        frame_h_supertiles: height.div_ceil(64),
    };
    // SAFETY: the backend's out-parameter.
    unsafe { *t = tiles };
}

/// The C rule for `rts` render targets (v3d_frame.c, job_check part 12):
/// 64x64, 64x32 for 2, 32x32 for 3-4.
#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_rts(width: u32, height: u32, rts: u32, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_ms(width, height, rts, false, t) }
}

/// ... and at 4x two steps further down the table (v3d_frame.c
/// v3d_frame_tiling_ms, job_check part 14): 32x32 for one RT.
#[no_mangle]
unsafe extern "C" fn v3d_frame_tiling_ms(width: u32, height: u32, rts: u32, msaa: bool, t: *mut V3dTiling) {
    // SAFETY: the backend's out-parameter, passed on.
    unsafe { v3d_frame_tiling_bpp(width, height, rts, msaa, 0, t) }
}

/// ... and one step further per step of the largest internal bpp
/// (v3d_frame.c v3d_frame_tiling_bpp, Mesa's v3d_choose_tile_size): 64 bpp
/// one, 128 two, down to 8x8.
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
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _draws: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
    // SAFETY: the backend's parameters.
    unsafe {
        let n = (*t).tiles_x * (*t).tiles_y;
        *ta = n * 128 + 8192;
        *ts = n * 256;
    }
}

/* ---- the platform --------------------------------------------------------------- */

const GPU_BASE: u64 = 0x0100_0000;

struct Alloc {
    mem: Box<[u64]>,
    size: usize,
}

#[derive(Default)]
struct Platform {
    next: u64,
    allocs: BTreeMap<u64, Alloc>,
    kicks: Vec<V3dKick>,
    /// The VPM the query answers, `None` for a platform with no answer.
    vpm: Option<u32>,
    /// What `wait` answers.
    wait_rc: i32,
    faults: Vec<String>,
    /// Answer every allocation with a GPU address and no CPU view.
    null_cpu: bool,
    /// The GPU address of every free, in order.
    frees: Vec<u64>,
    /// What OSAL_QUERY_V3D_BIN_POOL answers; `None`: -ENODATA.
    bin_pool: Option<crate::osal::OsalV3dBinPool>,
    /// What OSAL_QUERY_V3D_DRAWS_PER_JOB answers; `None`: no answer.
    draws_per_job: Option<u32>,
    /// What OSAL_QUERY_V3D_TILED_BUDGET answers; `None`: no answer.
    tiled_budget: Option<u32>,
    /// Every OSAL_QUERY_V3D_JOB_OPEN the backend sent, in order.
    job_open: Vec<u32>,
    /// What OSAL_QUERY_V3D_SPLIT_ZS answers; `None`: no answer.
    split_zs: Option<u32>,
    /// What OSAL_QUERY_V3D_S8_STENCIL answers; `None`: no answer.
    s8_stencil: Option<u32>,
    /// OSAL_QUERY_CLOCK_NS: (the next answer, the step each answer
    /// advances it by); `None`: no answer.
    clock: Option<(u64, u64)>,
    /// The OSAL of the async swap: `submit` returns with the job running,
    /// and a job completes only when it is waited for, when the next submit
    /// finishes it first, or when the test says so (`complete_jobs`). Off:
    /// every job completes inside its submit, as the OSAL did before.
    async_jobs: bool,
    /// The highest completed fence (`completed()`).
    completed: usize,
    /// What a job writes into its target when it completes, as the GPU
    /// does: (the job's fence, the target's GPU address, bytes, the byte
    /// written). Applied in `drain`, so a CPU read of the target before the
    /// job completes sees what was there before (fbo.render regression).
    renders: Vec<(usize, u64, usize, u8)>,
    /// Every job paints its RT 0 target with `0x10 + its fence` when it
    /// completes (the GPU's render, as `renders` for one job), so a CPU read
    /// of a target shows which job's render it saw; the jobs painted so far.
    paint_targets: bool,
    painted: usize,
    /// OSAL_QUERY_V3D_TFU_MIN_BYTES's answer (`None`: no TFU jobs), the
    /// result each TFU job gives, and every job run, in order.
    tfu_min: Option<u32>,
    tfu_rc: i32,
    tfu_runs: Vec<crate::osal::OsalV3dTfu>,
    /// TFU jobs queued behind a job not completed (OSAL_QUERY_V3D_TFU_QUEUE):
    /// (the fence they run after, the job), run by `drain` once it completes.
    tfu_queued: Vec<(usize, crate::osal::OsalV3dTfu)>,
    /// The most TFU jobs the OSAL keeps queued (-EBUSY past it); 0: no limit.
    tfu_queue_max: usize,
    /// Allocations of this many bytes or more fail (-ENOMEM); `None`: none.
    fail_alloc_from: Option<usize>,
    /// The sizes of the allocations refused (by either limit), in order.
    refused_allocs: Vec<usize>,
    /// The carve-out's bytes: an allocation fails (-ENOMEM) when the live
    /// allocations, the held frees and it together exceed them; `None`: no
    /// limit. With it, a free made while a job is open is held until that
    /// job (the next submitted) has run, and given back at an allocation or
    /// at the job's close, as src/osal/v3d_frees.h does on the board.
    capacity: Option<usize>,
    /// The frees held for the open job: (bytes, the submit they wait for).
    held: Vec<(usize, usize)>,
    /// Allocations `capacity` refused: with frees held, and with none.
    enomem_held: usize,
    enomem_none_held: usize,
}

impl Platform {
    /// Give back the held frees whose job has run (v3d_frees_drain).
    fn drain(&mut self) {
        let done = self.completed;
        self.held.retain(|&(_, fence)| fence > done);
        let (now, later): (Vec<_>, Vec<_>) = self.renders.iter().partition(|r| r.0 <= done);
        self.renders = later;
        for (_, gpu, n, byte) in now {
            let cpu = platform_cpu(self, gpu, n);
            // SAFETY: inside the allocation (platform_cpu checked).
            unsafe { core::ptr::write_bytes(cpu, byte, n) };
        }
        while self.paint_targets && self.painted < done {
            self.painted += 1;
            let f = self.painted;
            if let Some(t) = with_builder(|b| b.jobs.get(f - 1).map(|j| j.target)) {
                let n = t.stride as usize * t.height as usize;
                let cpu = platform_cpu(self, u64::from(t.address), n);
                // SAFETY: inside the allocation (platform_cpu checked).
                unsafe { core::ptr::write_bytes(cpu, 0x10 + f as u8, n) };
            }
        }
        /* the TFU jobs queued behind a job that has now run, after its render */
        let (now, later): (Vec<_>, Vec<_>) = self.tfu_queued.iter().partition(|q| q.0 <= done);
        self.tfu_queued = later;
        for (_, q) in now {
            self.tfu_runs.push(q);
            if self.tfu_rc == 0 {
                tfu_run(self, &q);
            }
        }
    }

    /// The bytes live allocations and held frees take.
    fn used(&self) -> usize {
        self.allocs.values().map(|a| a.size).sum::<usize>() + self.held.iter().map(|h| h.0).sum::<usize>()
    }
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

unsafe extern "C" fn p_gpu_alloc(_heap: u64, size: usize, _flags: u32, out: *mut OsalGpuMem) -> i32 {
    with_platform(|p| {
        if size == 0 || size > 64 << 20 || p.fail_alloc_from.is_some_and(|n| size >= n) {
            p.refused_allocs.push(size);
            return -ENOMEM;
        }
        if let Some(cap) = p.capacity {
            p.drain();
            if p.used() + size > cap {
                p.refused_allocs.push(size);
                if p.held.is_empty() {
                    p.enomem_none_held += 1;
                } else {
                    p.enomem_held += 1;
                }
                return -ENOMEM;
            }
        }
        let mut mem = vec![0u64; size.div_ceil(8)].into_boxed_slice();
        let gpu = GPU_BASE + p.next;
        p.next += (size as u64).div_ceil(4096) * 4096;
        let cpu = mem.as_mut_ptr() as *mut c_void;
        // SAFETY: the backend's out-parameter.
        unsafe {
            *out = OsalGpuMem {
                cpu: if p.null_cpu { core::ptr::null_mut() } else { cpu },
                gpu,
                phys: gpu,
                size,
                impl_: cpu,
            }
        };
        p.allocs.insert(gpu, Alloc { mem, size });
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the backend's allocation.
    let m = unsafe { *mem };
    with_platform(|p| {
        if !m.impl_.is_null() && p.allocs.remove(&m.gpu).is_none() {
            p.faults.push(format!("gpu_free of 0x{:x}, no live allocation", m.gpu));
        }
        if !m.impl_.is_null() {
            p.frees.push(m.gpu);
            /* src/osal/v3d_frees.h with a job in flight (zephyr_gc.c's
             * rule): held for the job being recorded, else for the last job
             * submitted while it has not completed */
            if p.capacity.is_some() && p.job_open.last() == Some(&1) {
                let fence = p.kicks.len() + 1;
                p.held.push((m.size, fence));
            } else if p.capacity.is_some() && p.completed < p.kicks.len() {
                let fence = p.kicks.len();
                p.held.push((m.size, fence));
            }
        }
    });
    0
}

unsafe extern "C" fn p_cache_flush(cpu: *mut c_void, n: usize) {
    with_platform(|p| {
        let inside = p.allocs.values().any(|a| {
            let base = a.mem.as_ptr() as usize;
            cpu as usize >= base && cpu as usize + n <= base + a.size
        });
        if !inside && n != 0 {
            p.faults.push(format!("cache_flush of {n} bytes at {cpu:p} outside every allocation"));
        }
    });
}

unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        with_platform(|p| p.faults.push(format!("submit of a {len}-byte descriptor")));
        return -(EINVAL as i64);
    }
    // SAFETY: the backend's live descriptor of that size.
    let k = unsafe { *(kick as *const V3dKick) };
    with_platform(|p| {
        p.kicks.push(k);
        let fence = p.kicks.len();
        /* the OSAL keeps one job running and one waiting behind it: a third
         * finishes the running one first (zephyr_v3d.c z_submit) */
        p.completed = if p.async_jobs { p.completed.max(fence.saturating_sub(2)) } else { fence };
        p.drain();
        fence as i64
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| {
        if fence > 0 {
            p.completed = p.completed.max((fence as usize).min(p.kicks.len()));
            p.drain();
        }
        p.wait_rc
    })
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.completed as i64)
}

/// The asynchronous OSAL's jobs so far complete, as when the GPU is done.
fn complete_jobs() {
    with_platform(|p| {
        p.completed = p.kicks.len();
        p.drain();
    });
}

/// The CPU view of `n` bytes at GPU address `gpu` in the platform's memory.
fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
    let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
    let off = (gpu - base) as usize;
    assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
    // SAFETY: inside the allocation.
    unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
}

/// The mock TFU: the job's rows copied from IIA (IIS texels apart) to IOA
/// (the fake layout's raster rows), the texel size from the job's type. A
/// mipmap job (DIMTW in IOA, `v3d_tex_tfu_pack_mipmap`'s fake) leaves level
/// 0 and box-filters each level of the fake layout from the one above it,
/// byte by byte, rounding half up as the GL core's filter does.
fn tfu_run(p: &mut Platform, q: &crate::osal::OsalV3dTfu) {
    let cpp = match q.icfg >> 9 & 0x7f {
        V3D_TFU_TTYPE_R8 => 1,
        V3D_TFU_TTYPE_R16F => 2,
        V3D_TFU_TTYPE_RGBA8 | V3D_TFU_TTYPE_R32F => 4,
        t => panic!("a TFU job of type {t}"),
    };
    let (w, h) = ((q.ios & 0xffff) as usize, (q.ios >> 16) as usize);
    if q.ioa & V3D_TFU_IOA_DIMTW != 0 {
        let levels = ((q.icfg >> 5) & 0xf) as usize + 1;
        /* the fake layout: the levels back to back, smallest first, so
         * level k is the sizes of the levels below it into the image */
        let size = |k: usize| ((w >> k).max(1)) * ((h >> k).max(1)) * cpp;
        let offset = |k: usize| (k + 1..levels).map(size).sum::<usize>();
        let base = (q.iia & !0x3f) as u64 - offset(0) as u64;
        for k in 1..levels {
            let (sw, sh) = ((w >> (k - 1)).max(1), (h >> (k - 1)).max(1));
            let (dw, dh) = ((w >> k).max(1), (h >> k).max(1));
            let (sx, sy) = (if sw > 1 { 2 } else { 1 }, if sh > 1 { 2 } else { 1 });
            let src = platform_cpu(p, base + offset(k - 1) as u64, size(k - 1)) as *const u8;
            let dst = platform_cpu(p, base + offset(k) as u64, size(k));
            for y in 0..dh {
                for x in 0..dw {
                    for c in 0..cpp {
                        let mut sum = 0u32;
                        for j in 0..sy {
                            for i in 0..sx {
                                // SAFETY: inside level k - 1 (platform_cpu checked its size).
                                sum += u32::from(unsafe { *src.add(((y * sy + j) * sw + x * sx + i) * cpp + c) });
                            }
                        }
                        let n = (sx * sy) as u32;
                        // SAFETY: inside level k.
                        unsafe { *dst.add((y * dw + x) * cpp + c) = ((sum + n / 2) / n) as u8 };
                    }
                }
            }
        }
        return;
    }
    let (src_row, dst_row) = (q.iis as usize * cpp, w * cpp);
    for y in 0..h {
        let from = platform_cpu(p, q.iia as u64 + (y * src_row) as u64, dst_row);
        let to = platform_cpu(p, (q.ioa & !0x3f) as u64 + (y * dst_row) as u64, dst_row);
        // SAFETY: both checked inside their allocations.
        unsafe { core::ptr::copy_nonoverlapping(from, to, dst_row) };
    }
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    if what == crate::osal::OSAL_QUERY_V3D_TFU_MIN_BYTES && out_sz == 4 {
        return match with_platform(|p| p.tfu_min) {
            Some(n) => {
                // SAFETY: a u32 out-parameter.
                unsafe { *(out as *mut u32) = n };
                0
            }
            None => -EINVAL,
        };
    }
    if what == crate::osal::OSAL_QUERY_V3D_TFU_RUN && out_sz == size_of::<crate::osal::OsalV3dTfu>() {
        // SAFETY: the backend's live struct of that size.
        let q = unsafe { &mut *(out as *mut crate::osal::OsalV3dTfu) };
        return with_platform(|p| {
            if p.tfu_min.is_none() {
                return -(crate::util::ENOTSUP);
            }
            p.tfu_runs.push(*q);
            q.rc = p.tfu_rc;
            if q.rc == 0 {
                tfu_run(p, q);
            }
            0
        });
    }
    if what == crate::osal::OSAL_QUERY_V3D_TFU_QUEUE && out_sz == size_of::<crate::osal::OsalV3dTfuQueue>() {
        // SAFETY: the backend's live struct of that size.
        let q = unsafe { &mut *(out as *mut crate::osal::OsalV3dTfuQueue) };
        return with_platform(|p| {
            if p.tfu_min.is_none() {
                return -(crate::util::ENOTSUP);
            }
            if q.after < 0 || q.after as usize > p.kicks.len() {
                p.faults.push(format!("a TFU job queued after fence {}, of {} submitted", q.after, p.kicks.len()));
                return -EINVAL;
            }
            /* as zephyr_v3d.c: behind the job not completed, else now (the
             * mock queues behind the newest job, the one in flight) */
            if p.completed < p.kicks.len() {
                if p.tfu_queue_max != 0 && p.tfu_queued.len() >= p.tfu_queue_max {
                    return -EBUSY;
                }
                q.fence = p.kicks.len() as i64;
                q.tfu.rc = 0;
                p.tfu_queued.push((p.kicks.len(), q.tfu));
                return 0;
            }
            q.fence = 0;
            p.tfu_runs.push(q.tfu);
            q.tfu.rc = p.tfu_rc;
            if q.tfu.rc == 0 {
                tfu_run(p, &q.tfu);
            }
            0
        });
    }
    if what == crate::osal::OSAL_QUERY_CLOCK_NS && out_sz == 8 {
        return match with_platform(|p| p.clock.as_mut().map(|c| (c.0, c.0 += c.1).0)) {
            Some(ns) => {
                // SAFETY: a u64 out-parameter.
                unsafe { *(out as *mut u64) = ns };
                0
            }
            None => -EINVAL,
        };
    }
    if what == crate::osal::OSAL_QUERY_V3D_JOB_OPEN && out_sz == 4 {
        // SAFETY: a u32 in-parameter.
        let v = unsafe { *(out as *const u32) };
        with_platform(|p| {
            p.job_open.push(v);
            if v == 0 {
                p.drain(); /* the job has run: what it held goes back */
            }
        });
        return 0;
    }
    if what == crate::osal::OSAL_QUERY_V3D_S8_STENCIL && out_sz == 4 {
        return match with_platform(|p| p.s8_stencil) {
            Some(n) => {
                // SAFETY: a u32 out-parameter.
                unsafe { *(out as *mut u32) = n };
                0
            }
            None => -EINVAL,
        };
    }
    if what == crate::osal::OSAL_QUERY_V3D_SPLIT_ZS && out_sz == 4 {
        return match with_platform(|p| p.split_zs) {
            Some(n) => {
                // SAFETY: a u32 out-parameter.
                unsafe { *(out as *mut u32) = n };
                0
            }
            None => -EINVAL,
        };
    }
    if what == crate::osal::OSAL_QUERY_V3D_TILED_BUDGET && out_sz == 4 {
        return match with_platform(|p| p.tiled_budget) {
            Some(n) => {
                // SAFETY: a u32 out-parameter.
                unsafe { *(out as *mut u32) = n };
                0
            }
            None => -EINVAL,
        };
    }
    if what == crate::osal::OSAL_QUERY_V3D_DRAWS_PER_JOB && out_sz == 4 {
        return match with_platform(|p| p.draws_per_job) {
            Some(n) => {
                // SAFETY: a u32 out-parameter.
                unsafe { *(out as *mut u32) = n };
                0
            }
            None => -EINVAL,
        };
    }
    if what == crate::osal::OSAL_QUERY_V3D_BIN_POOL && out_sz == size_of::<crate::osal::OsalV3dBinPool>() {
        return match with_platform(|p| p.bin_pool) {
            Some(b) => {
                // SAFETY: the backend's out struct of that size.
                unsafe { *(out as *mut crate::osal::OsalV3dBinPool) = b };
                0
            }
            None => -61, /* ENODATA, as zephyr_v3d.c answers before the first job */
        };
    }
    if what == crate::osal::OSAL_QUERY_V3D_CPU_OF_GPU && out_sz == size_of::<crate::osal::OsalV3dCpuOf>() {
        // SAFETY: the backend's in/out struct of that size.
        let q = unsafe { &mut *(out as *mut crate::osal::OsalV3dCpuOf) };
        return with_platform(|p| match p.allocs.range(..=q.gpu).next_back() {
            Some((base, a)) if q.gpu + q.bytes <= base + a.size as u64 => {
                // SAFETY: inside the allocation.
                q.cpu = unsafe { (a.mem.as_ptr() as *mut u8).add((q.gpu - base) as usize) } as *mut c_void;
                0
            }
            _ => -EINVAL,
        });
    }
    with_platform(|p| match p.vpm {
        Some(v) if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 => {
            // SAFETY: a u32 out-parameter.
            unsafe { *(out as *mut u32) = v };
            0
        }
        _ => -EINVAL,
    })
}

/* ---- the fake texture functions (driver/src/v3d/v3d_tex.c) ----------------------- *
 *
 * The C is checked against Mesa by tools/v3d/tex_check.c; these stand in for
 * it here, as the fake job builder does for v3d_job.c, with outputs the
 * tests can read: a layout that is the levels back to back (smallest first,
 * as Mesa's), a "tiling" that copies the level's rows packed, and records
 * that hold their inputs at fixed places. */

/// The fake texture state's word 3: the max level in the low half, the TMU
/// type (`V3D_TEXTURE_DATA_FORMAT_*`) from this bit.
const FAKE_TYPE_SHIFT: u32 = 16;
/// The fake records' marks: bytes 20..24 of a texture state and a sampler.
const FAKE_STATE_MARK: u32 = 0x5354_4154; /* "TATS" */
const FAKE_SAMPLER_MARK: u32 = 0x4d41_5353; /* "SSAM" */

#[no_mangle]
unsafe extern "C" fn v3d_tex_layout(w: u32, h: u32, levels: u32, cpp: u32, l: *mut V3dTexLayout) -> bool {
    if w == 0 || h == 0 || levels == 0 || levels as usize > V3D_TEX_MAX_LEVELS {
        return false;
    }
    let mut out = V3dTexLayout { width: w, height: h, levels, cpp, ..V3dTexLayout::default() };
    /* level 0 on a 64-byte boundary, as the real layout puts it on a page
     * (a TFU job and a texture state take its address with low flag bits):
     * the smaller levels are shifted up by the pad, as Mesa's page
     * alignment shifts every offset */
    let below: u32 = (1..levels).map(|i| ((w >> i).max(1)) * ((h >> i).max(1)) * cpp).sum();
    let mut off = below.next_multiple_of(64) - below;
    for i in (0..levels as usize).rev() {
        let (lw, lh) = ((w >> i).max(1), (h >> i).max(1));
        /* the tiling Mesa's v3d_setup_slices picks by the level's size for
         * 4-byte texels (utile 4x4, UIF block 8x8): LT, UB-linear 1 or 2
         * columns, else UIF without the XOR (the real one pads the height
         * and may pick the XOR; this layout pads nothing) */
        let tiling = if lw <= 4 || lh <= 4 {
            V3D_TEX_TILING_LINEARTILE
        } else if lw <= 8 {
            2 /* UBLINEAR_1_COLUMN */
        } else if lw <= 16 {
            3 /* UBLINEAR_2_COLUMN */
        } else {
            V3D_TEX_TILING_UIF_NO_XOR
        };
        out.slices[i] =
            V3dTexSlice { offset: off, stride: lw * cpp, padded_height: lh, size: lw * lh * cpp, ub_pad: 0, tiling };
        off += lw * lh * cpp;
    }
    out.size = off;
    // SAFETY: the backend's out-parameter.
    unsafe { *l = out };
    true
}

/// The fake 4x layout: one level of twice the size, marked UIF (the real
/// one is checked against Mesa by tools/v3d/tex_check.c).
#[no_mangle]
unsafe extern "C" fn v3d_tex_layout_msaa(w: u32, h: u32, cpp: u32, l: *mut V3dTexLayout) -> bool {
    // SAFETY: the backend's out-parameter, passed on.
    if !unsafe { v3d_tex_layout(2 * w, 2 * h, 1, cpp, l) } {
        return false;
    }
    // SAFETY: as above.
    let l = unsafe { &mut *l };
    l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
    l.slices[0].padded_height = 2 * h;
    true
}

/// The fake layout's TFU job (v3d_tex.c v3d_tex_tfu_pack's refusals, the
/// fake's raster slice as the destination): the mock OSAL's TFU runs it as a
/// row copy (`tfu_run`), so a test sees the source, stride, destination and
/// size the backend handed over.
#[no_mangle]
unsafe extern "C" fn v3d_tex_tfu_pack(
    l: *const V3dTexLayout,
    ttype: u32,
    src_gpu: u32,
    src_stride: u32,
    dst_gpu: u32,
    j: *mut V3dTexTfuJob,
) -> bool {
    // SAFETY: the backend's live layout and job.
    let (l, j) = unsafe { (&*l, &mut *j) };
    let ioa = dst_gpu + l.slices[0].offset;
    if l.levels != 1
        || l.cpp == 0
        || !src_stride.is_multiple_of(l.cpp)
        || src_stride < l.width * l.cpp
        || src_gpu & 3 != 0
        || ioa & 0x3f != 0
    {
        return false;
    }
    *j = V3dTexTfuJob {
        icfg: ttype << 9,
        iia: src_gpu,
        iis: src_stride / l.cpp,
        ioa,
        ios: l.height << 16 | l.width,
        ..V3dTexTfuJob::default()
    };
    true
}

/// The fake layout's mipmap TFU job (v3d_tex.c v3d_tex_tfu_pack_mipmap's
/// refusals): level 0 as the source and the destination, the levels after
/// it in ICFG bits 5.., DIMTW in IOA; the mock TFU box-filters the fake
/// layout's levels from it (`tfu_run`).
#[no_mangle]
unsafe extern "C" fn v3d_tex_tfu_pack_mipmap(
    l: *const V3dTexLayout,
    ttype: u32,
    image_gpu: u32,
    j: *mut V3dTexTfuJob,
) -> bool {
    // SAFETY: the backend's live layout and job.
    let (l, j) = unsafe { (&*l, &mut *j) };
    let cpp = match ttype {
        V3D_TFU_TTYPE_R8 => 1,
        V3D_TFU_TTYPE_R16F => 2,
        V3D_TFU_TTYPE_RGBA8 | V3D_TFU_TTYPE_R32F => 4,
        _ => return false,
    };
    let level0 = image_gpu + l.slices[0].offset;
    if l.levels < 2 || l.cpp != cpp || level0 & 0x3f != 0 {
        return false;
    }
    *j = V3dTexTfuJob {
        icfg: ttype << 9 | (l.levels - 1) << 5,
        iia: level0,
        ioa: level0 | V3D_TFU_IOA_DIMTW,
        ios: l.height << 16 | l.width,
        ..V3dTexTfuJob::default()
    };
    true
}

#[no_mangle]
unsafe extern "C" fn v3d_tex_store_level(
    image: *mut u8,
    l: *const V3dTexLayout,
    level: u32,
    src: *const u8,
    stride: u32,
) {
    // SAFETY: the backend's live layout, an image of layout.size bytes and a
    // source of the level's rows at `stride`.
    unsafe {
        let s = (*l).slices[level as usize];
        for y in 0..s.padded_height {
            core::ptr::copy_nonoverlapping(
                src.add((y * stride) as usize),
                image.add((s.offset + y * s.stride) as usize),
                s.stride as usize,
            );
        }
    }
}

/// The fake of v3d_tex_store_box: the C's refusals (driver/src/v3d/v3d_tex.c:
/// a level the layout lacks, a box outside the level, a stride shorter than
/// a row of the box), and the box's rows copied into the fake's packed
/// level.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn v3d_tex_store_box(
    image: *mut u8,
    l: *const V3dTexLayout,
    level: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    src: *const u8,
    stride: u32,
) -> bool {
    // SAFETY: the backend's live layout.
    let l = unsafe { &*l };
    if image.is_null() || src.is_null() || level >= l.levels {
        return false;
    }
    let (lw, lh) = ((l.width >> level).max(1), (l.height >> level).max(1));
    if x > lw || w > lw - x || y > lh || h > lh - y {
        return false;
    }
    if w == 0 || h == 0 {
        return true;
    }
    if stride < w * l.cpp {
        return false;
    }
    let s = l.slices[level as usize];
    for row in 0..h {
        // SAFETY: row `row` of the box at `stride`, into the packed level
        // inside the image (the box is inside the level).
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.add((row * stride) as usize),
                image.add((s.offset + (y + row) * s.stride + x * l.cpp) as usize),
                (w * l.cpp) as usize,
            )
        };
    }
    true
}

/// The fake of v3d_tex_load_box: v3d_tex_store_box's refusals, and the box's
/// rows copied out of the fake's packed level into rows `stride` apart. The
/// real one is Mesa's v3d_load_tiled_image, which tools/v3d/tex_check.c
/// compares box by box with Mesa's own.
#[no_mangle]
#[allow(clippy::too_many_arguments)]
unsafe extern "C" fn v3d_tex_load_box(
    image: *const u8,
    l: *const V3dTexLayout,
    level: u32,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    dst: *mut u8,
    stride: u32,
) -> bool {
    // SAFETY: the backend's live layout.
    let l = unsafe { &*l };
    if image.is_null() || dst.is_null() || level >= l.levels {
        return false;
    }
    let (lw, lh) = ((l.width >> level).max(1), (l.height >> level).max(1));
    if x > lw || w > lw - x || y > lh || h > lh - y {
        return false;
    }
    if w == 0 || h == 0 {
        return true;
    }
    if stride < w * l.cpp {
        return false;
    }
    let s = l.slices[level as usize];
    for row in 0..h {
        // SAFETY: row `row` of the box inside the packed level, into the
        // destination's row at `stride`.
        unsafe {
            core::ptr::copy_nonoverlapping(
                image.add((s.offset + (y + row) * s.stride + x * l.cpp) as usize),
                dst.add((row * stride) as usize),
                (w * l.cpp) as usize,
            )
        };
    }
    true
}

/// The fake of v3d_tex_layer_stride: the C's rule (level 0's end, 64-byte
/// aligned; level 0 is the last slice of the fake's packed layout too).
#[no_mangle]
unsafe extern "C" fn v3d_tex_layer_stride(l: *const V3dTexLayout) -> u32 {
    // SAFETY: the backend's live layout.
    let l = unsafe { &*l };
    (l.slices[0].offset + l.slices[0].size).next_multiple_of(64)
}

/// The fake of v3d_tex_layers_size: `layers` trees, a stride apart.
#[no_mangle]
unsafe extern "C" fn v3d_tex_layers_size(l: *const V3dTexLayout, layers: u32, out: *mut u32) -> bool {
    if layers == 0 {
        return false;
    }
    // SAFETY: the backend's live layout and out-parameter.
    unsafe { *out = (*l).size + v3d_tex_layer_stride(l) * (layers - 1) };
    true
}

#[no_mangle]
unsafe extern "C" fn v3d_tex_state_from_layout(s: *mut V3dTexState, l: *const V3dTexLayout) {
    // SAFETY: the backend's live values.
    unsafe { (*s).array_stride_64 = (*l).size / 64 };
}

#[no_mangle]
unsafe extern "C" fn v3d_tex_pack_state(out: *mut u8, s: *const V3dTexState) -> bool {
    // SAFETY: V3D_TEX_STATE_BYTES of the arena and the backend's live state.
    unsafe {
        let s = &*s;
        let words = [
            s.base,
            s.width,
            s.height,
            s.max_level | s.type_ << FAKE_TYPE_SHIFT,
            u32::from_le_bytes(s.swizzle),
            FAKE_STATE_MARK,
        ];
        core::ptr::copy_nonoverlapping(words.as_ptr() as *const u8, out, V3D_TEX_STATE_BYTES);
    }
    true
}

#[no_mangle]
unsafe extern "C" fn v3d_tex_pack_sampler(out: *mut u8, s: *const V3dSampler) -> bool {
    // SAFETY: V3D_SAMPLER_STATE_BYTES of the arena and the backend's sampler.
    unsafe {
        let s = &*s;
        let flags =
            s.mag_nearest as u32 | (s.min_nearest as u32) << 1 | (s.mip_nearest as u32) << 2 | (s.mip_none as u32) << 3;
        let words = [s.wrap_s, s.wrap_t, flags, 0, 0, FAKE_SAMPLER_MARK];
        core::ptr::copy_nonoverlapping(words.as_ptr() as *const u8, out, V3D_SAMPLER_STATE_BYTES);
    }
    true
}

#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
extern "C" {
    /// testutil's log trampoline: the lines land in the TestGl recorder.
    fn yakogl_test_log(fmt: *const c_char, ...);
}
#[cfg(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64")))]
const RIG_LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = Some(yakogl_test_log);
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
const RIG_LOG: Option<unsafe extern "C" fn(*const c_char, ...)> = None;

/// The platform is up already (the rig made it); eglInitialize asks.
unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
    0
}

static RIG_OSAL: OsalOps = OsalOps {
    init: Some(p_init),
    shutdown: None,
    gpu_alloc: Some(p_gpu_alloc),
    gpu_free: Some(p_gpu_free),
    mmu_map: None,
    map_phys: None,
    gpu_write: None,
    gpu_read: None,
    cache_flush: Some(p_cache_flush),
    cache_invd: None,
    submit: Some(p_submit),
    wait: Some(p_wait),
    completed: Some(p_completed),
    reg_read: None,
    reg_write: None,
    query: Some(p_query),
    log: RIG_LOG,
    wait_vblank: None,
    present: None,
    lock: None,
    unlock: None,
    thread_id: None,
    display_busy: None,
};

/* ---- the rig ------------------------------------------------------------------- */

const TARGET: u64 = 0x0800_0000;
const FS_USC: u64 = 0x0900_0000;
const BUF: u64 = 0x0a00_0000;

/// The uniform streams the programs carry (by pointer, as the linked
/// shader's `ShcV3d` does).
static FS_STREAM: [ShcV3dUniform; 2] = [
    ShcV3dUniform { contents: V3D_QUNIFORM_CONSTANT, data: 0x3f80_0000 },
    ShcV3dUniform { contents: V3D_QUNIFORM_UNIFORM, data: 1 },
];
static VS_STREAM: [ShcV3dUniform; 1] = [ShcV3dUniform { contents: V3D_QUNIFORM_VIEWPORT_X_SCALE, data: 0 }];

/// A linked program: FS at 0, VS at 72, CS at 136 in 200 bytes of code; two
/// attribute records, location 1 fetched through record 0 and location 3
/// through record 1.
/// Real QPU code for `sh`'s three stages at the offsets and sizes `program`
/// gives them (FS 9 words at 0, VS 8 at 72, CS 8 at 136): each stage's
/// uniform-consuming words first (ldunif, one per entry of its stream as it
/// is now), then nops, the program-end THRSW on the third-to-last word, as
/// the backend's `check_program` wants. Called again after a test swaps a
/// stream.
fn with_code(sh: &mut ShcShader) {
    use crate::shaderc::online::v3d::encode::encode;
    use crate::shaderc::online::v3d::isa::{Alu, Instr};
    let mut words = std::vec![0u32; 50];
    for (off, bytes, n) in [
        (sh.v3d.fs.code_off, sh.v3d.fs.code_bytes, sh.v3d.fs.num_uniforms),
        (sh.v3d.vs.code_off, sh.v3d.vs.code_bytes, sh.v3d.vs.num_uniforms),
        (sh.v3d.cs.code_off, sh.v3d.cs.code_bytes, sh.v3d.cs.num_uniforms),
    ] {
        let len = (bytes / 8) as usize;
        assert!(n as usize + 3 <= len, "a stream of {n} does not fit {len} words");
        for k in 0..len {
            let mut a = Alu::nop();
            a.sig.ldunif = k < n as usize;
            a.sig.thrsw = k == len - 3;
            let w = encode(&Instr::Alu(a)).expect("the rig's QPU word encodes");
            let at = (off / 4) as usize + 2 * k;
            (words[at], words[at + 1]) = (w as u32, (w >> 32) as u32);
        }
    }
    sh.usc = Box::leak(words.into_boxed_slice()).as_ptr();
    sh.usc_dw = 50;
}

fn program() -> ShcShader {
    let mut sh = ShcShader::ZERO;
    sh.usc_dw = 50;
    let vs_half = |code_off| ShcV3dVs {
        code_off,
        code_bytes: 64,
        uniforms: VS_STREAM.as_ptr(),
        num_uniforms: 1,
        threads: 4,
        single_seg: 1,
        vpm_output_size: 2,
        vcm_cache_size: 4,
        vattr_sizes: [4, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
        ..ShcV3dVs::ZERO
    };
    let mut rec = [SHC_V3D_NO_RECORD; 16];
    rec[1] = 0;
    rec[3] = 1;
    sh.v3d = ShcV3d {
        linked: 1,
        num_records: 2,
        record_of_location: rec,
        fs: ShcV3dFs {
            code_off: 0,
            code_bytes: 72,
            uniforms: FS_STREAM.as_ptr(),
            num_uniforms: 2,
            num_inputs: 4,
            threads: 4,
            ..ShcV3dFs::ZERO
        },
        vs: vs_half(72),
        cs: vs_half(136),
        ..ShcV3d::ZERO
    };
    with_code(&mut sh);
    sh
}

struct Rig {
    prev_osal: *const OsalOps,
    gl: TestGl,
    /// The in-process compiler's test lock, for the rig's whole life:
    /// `v3d_init` compiles and links the clear program and `v3d_shutdown`
    /// (in `Drop`, before this field drops) releases it, and a compile or a
    /// release on a thread without the lock races every compile test that
    /// holds it (the v3d_abi tests failed about 4 runs in 20 with "a compile
    /// was started inside a compile", 2026-09-24). Taken after TestGl, the
    /// order every other test uses.
    _compiler: crate::shaderc::online::store::Serial,
    fs: Box<ShcShader>,
    vs_block: Box<[f32; 4]>,
    fs_block: Box<[f32; 2]>,
}

impl Rig {
    fn new() -> Rig {
        Rig::with_vpm(Some(65536))
    }

    fn with_vpm(vpm: Option<u32>) -> Rig {
        let gl = TestGl::new(4, 4);
        let compiler = crate::shaderc::online::store::test_lock();
        *PLATFORM.lock().unwrap_or_else(|e| e.into_inner()) = Some(Platform { vpm, ..Platform::default() });
        *BUILDER.lock().unwrap_or_else(|e| e.into_inner()) = Some(Builder::default());
        // SAFETY: serialised by the TestGl guard.
        let prev_osal = unsafe {
            let prev = crate::osal::g_osal;
            crate::osal::osal_set_ops(&RIG_OSAL);
            prev
        };
        v3d_shutdown();
        /* the refusal log is per process in the product; per test here */
        *g() = V3dState::ZERO;
        Rig {
            prev_osal,
            gl,
            _compiler: compiler,
            fs: Box::new(program()),
            vs_block: Box::new([0.5, 1.0, 1.5, 2.0]),
            fs_block: Box::new([3.0, 4.0]),
        }
    }

    /// A 64x64 window target cleared to red, then a triangle list of 3
    /// vertices from two buffer arrays at locations 3 and 1.
    fn draw(&self) -> GlesDraw {
        let mut d = GlesDraw::ZERO;
        d.target_gpu = TARGET;
        (d.target_w, d.target_h) = (64, 64);
        d.target_format = GLES_SURF_RGBA8888;
        d.target_alpha_bits = 8;
        d.samples = 1;
        d.do_clear = true;
        d.clear_color_mask = 0xf; /* the GL core's mask with do_clear: never 0 */
        d.clear_rgba = [1.0, 0.0, 0.0, 1.0];
        d.clear_depth = 1.0;
        d.prim = GLES_PRIM_TRIANGLES;
        d.index_count = 3;
        d.fs = &*self.fs;
        d.fs_usc_gpu = FS_USC;
        d.attribs[0] =
            GlesAttrib { vi_reg: 3, components: 2, stride: 8, offset: 0x100, buffer_gpu: BUF, ..GlesAttrib::ZERO };
        d.attribs[1] =
            GlesAttrib { vi_reg: 1, components: 4, stride: 16, offset: 0x40, buffer_gpu: BUF, ..GlesAttrib::ZERO };
        d.num_attribs = 2;
        d.vs_uniform_cpu = self.vs_block.as_ptr() as *const c_void;
        d.vs_uniform_dwords = 4;
        d.fs_uniform_cpu = self.fs_block.as_ptr() as *const c_void;
        d.fs_uniform_dwords = 2;
        d.viewport = [0, 0, 64, 64];
        d.depth_range = [0.0, 1.0];
        d.flip_y = true;
        d.color_mask = 0xf;
        d.color_write = true;
        d.line_width = 1.0;
        d
    }

    fn init(&self) {
        assert_eq!(v3d_init(), 0, "{:?}", self.gl.logs());
    }

    fn jobs(&self) -> Vec<SeenJob> {
        with_builder(|b| b.jobs.clone())
    }

    fn draws(&self) -> Vec<SeenDraw> {
        with_builder(|b| b.draws.clone())
    }

    fn kicks(&self) -> Vec<V3dKick> {
        with_platform(|p| p.kicks.clone())
    }

    fn answer(&self, rcs: &[i32]) {
        with_builder(|b| b.answers.extend_from_slice(rcs));
    }

    /// The bytes the platform holds at GPU address `gpu`.
    fn gpu_bytes(&self, gpu: u32, n: usize) -> Vec<u8> {
        with_platform(|p| {
            let (base, a) = p.allocs.range(..=gpu as u64).next_back().expect("an allocation below the address");
            let off = (gpu as u64 - base) as usize;
            assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
            // SAFETY: inside the allocation.
            unsafe { core::slice::from_raw_parts((a.mem.as_ptr() as *const u8).add(off), n) }.to_vec()
        })
    }

    /// The log lines since the last call that contain `word`. It DRAINS the
    /// recorder (`TestGl::logs` takes the lines), so two calls in a row see
    /// two different sets: the second counts only what came after the first.
    fn logs_with(&self, word: &str) -> usize {
        self.gl.logs().iter().filter(|l| l.contains(word)).count()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        v3d_shutdown();
        // SAFETY: serialised; testutil's table goes back before its guard drops.
        unsafe { crate::osal::osal_set_ops(self.prev_osal) };
        let faults = with_platform(|p| core::mem::take(&mut p.faults));
        let more = with_builder(|b| core::mem::take(&mut b.faults));
        if (!faults.is_empty() || !more.is_empty()) && !std::thread::panicking() {
            panic!("faults:\n  {}\n  {}", faults.join("\n  "), more.join("\n  "));
        }
    }
}

fn draw(d: &GlesDraw) -> i32 {
    // SAFETY: a valid draw for the call.
    unsafe { v3d_draw(d) }
}

/* ---- the rig and the compiler's test lock ------------------------------------ */

/// The rig compiles (v3d_init builds the clear program) and releases (in
/// v3d_shutdown) only while it holds the in-process compiler's test lock.
/// Without it, the rig's compile opened its keep transaction and then waited
/// for the lock inside `with_scratch` with the transaction still open, and a
/// test that held the lock and compiled in that window was refused: "a
/// compile was started inside a compile" (the v3d_abi flake, four tests at
/// once, about 4 runs in 20, 2026-09-24). This test is that window, on
/// purpose: it holds the lock, lets the rig start, then compiles.
///
/// Revert check: a rig without the lock fails this at once with the
/// flake's message.
#[test]
fn a_compile_under_the_lock_is_not_refused_while_a_rig_starts() {
    use crate::shaderc::online::store;
    let (tx, rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _s = store::test_lock();
        tx.send(()).unwrap();
        /* long enough for the rig to reach its compile */
        std::thread::sleep(std::time::Duration::from_millis(100));
        /* a source no other test compiles: a cache hit opens no keep */
        let src = b"attribute vec4 p; void main(){ gl_Position = p * 2718.5; }\0";
        let mut sh = ShcShader::ZERO;
        let mut log = [0 as c_char; 256];
        let f = crate::shaderc::online::SHC_ONLINE.compile.unwrap();
        // SAFETY: a NUL-terminated source, a real output and log.
        let rc = unsafe { f(SHC_STAGE_VERTEX, src.as_ptr() as *const c_char, &mut sh, log.as_mut_ptr(), log.len()) };
        crate::shaderc::release_shader(&sh);
        // SAFETY: c_char and u8 have the same size; the log is NUL-terminated.
        let bytes = unsafe { core::slice::from_raw_parts(log.as_ptr() as *const u8, log.len()) };
        let n = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        (rc, String::from_utf8_lossy(&bytes[..n]).into_owned())
    });
    rx.recv().unwrap();
    let rig = Rig::new();
    rig.init();
    drop(rig);
    let (rc, log) = holder.join().unwrap();
    assert_eq!(rc, 0, "a locked compile was refused while a rig started: {log}");
}

/* ---- init ----------------------------------------------------------------------- */

#[test]
fn init_refuses_a_vpm_the_programs_are_not_compiled_for() {
    let rig = Rig::with_vpm(Some(16384));
    assert_eq!(v3d_init(), -ENODEV);
    assert_eq!(rig.logs_with("VPM is 16384"), 1, "{:?}", rig.gl.logs());
    drop(rig);
    let rig = Rig::with_vpm(None);
    assert_eq!(v3d_init(), -ENODEV, "no answer: the VPM cannot be checked, so the backend does not start");
    drop(rig);
    let rig = Rig::new();
    rig.init();
    /* every buffer the lists name must be a 32-bit V3D address */
    let g = g();
    for m in [
        &g.arenas.bcl,
        &g.arenas.rcl,
        &g.arenas.ind,
        &g.arenas.data,
        &g.arenas.tex,
        &g.spare.bcl,
        &g.spare.rcl,
        &g.spare.ind,
        &g.spare.data,
        &g.spare.tex,
        &g.tile_alloc,
        &g.tile_state,
    ] {
        assert!(m.gpu != 0 && m.gpu <= u32::MAX as u64 && !m.cpu.is_null());
    }
    /* sized from the tiling of the largest target with the smallest tiles a
     * job can have (4 render targets: 32x32, so 64 x 64 of them) */
    assert_eq!((g.tile_alloc_size, g.tile_state_size), (64 * 64 * 128 + 8192, 64 * 64 * 256));
}

#[test]
fn a_draw_before_init_is_no_device() {
    let rig = Rig::new();
    assert_eq!(draw(&rig.draw()), -ENODEV);
}

/* ---- clear and draw ----------------------------------------------------------- */

#[test]
fn a_clear_only_draw_is_a_job_with_a_tlb_clear_and_no_draw() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.index_count = 0;
    d.fs = core::ptr::null();
    assert_eq!(draw(&d), 0);
    assert!(rig.kicks().is_empty(), "nothing is submitted before a flush");
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 1);
    let tile_alloc = g().tile_alloc.gpu as u32;
    assert_eq!(
        jobs[0].target,
        V3dJobTarget {
            width: 64,
            height: 64,
            address: TARGET as u32,
            stride: 256,
            tile_alloc,
            rgb565: false,
            rts: 0,
            rt: [V3dJobRt::default(); 3],
            ..V3dJobTarget::default()
        }
    );
    assert_eq!(jobs[0].draws, 0);
    let c = jobs[0].clear.expect("ended");
    assert!(!c.load_rt0, "a cleared colour is not loaded");
    assert_eq!(c.rgba8, 0xff00_00ff, "red, R in the low byte");
    /* one kick, naming the lists and the tile memory */
    let k = rig.kicks();
    assert_eq!(k.len(), 1);
    assert_eq!(k[0].magic, V3D_KICK_MAGIC);
    let (bcl, rcl) = (g().arenas.bcl.gpu as u32, g().arenas.rcl.gpu as u32);
    assert_eq!((k[0].bcl_start, k[0].bcl_end), (bcl, bcl + 17));
    assert_eq!((k[0].rcl_start, k[0].rcl_end), (rcl, rcl + 200));
    assert_eq!((k[0].tile_alloc, k[0].tile_state), (tile_alloc, g().tile_state.gpu as u32));
    /* the tile state the job uses: one 64x64 tile of 256 bytes */
    assert_eq!((k[0].tile_alloc_size, k[0].tile_state_size), (g().tile_alloc_size, 256));
    /* a second flush has nothing to submit */
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rig.kicks().len(), 1);
}

/// The tile state a kick asks the OSAL to zero is the job's own: 256
/// bytes a tile of the job's tiling (Mesa's TSDA per job), never the whole
/// allocation. A single-RT 1920x720 job (the cluster's frame) is 30 x 12
/// tiles of 64x64, 92,160 bytes; the same target at 4x has 32x32 tiles,
/// 60 x 23. Before, every kick named the allocation, which MRT step 3
/// sized for four RTs at 2048x2048 (1 MiB): the OSAL zeroed it every job,
/// DEMO15's "prep" 76 -> 307 us (glue, from the record). Revert check:
/// the kick's size back to `g.tile_state_size` reads 1,048,576 here.
#[test]
fn a_kick_zeroes_the_tile_state_of_its_own_tiles() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    (d.target_w, d.target_h) = (1920, 720);
    d.index_count = 0;
    d.fs = core::ptr::null();
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    d.samples = 4;
    d.target_gpu = TARGET + 0x40_0000;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let sizes: Vec<u32> = rig.kicks().iter().map(|k| k.tile_state_size).collect();
    assert_eq!(sizes, [30 * 12 * 256, 60 * 23 * 256], "1x then 4x, 1920x720");
    assert_eq!(g().tile_state_size, 64 * 64 * 256, "the allocation stays the largest");
}

/// An odd-width target is written at the GL core's row pitch, whole pixel
/// pairs (`gl::context::tex_pitch`), which is where glReadPixels and the
/// texture's storage put its rows; the even widths every board run used are
/// unchanged.
///
/// Revert check: `stride: w * 4` fails the odd width.
#[test]
fn an_odd_width_target_is_written_at_the_gl_cores_pitch() {
    let rig = Rig::new();
    rig.init();
    let mut c = clear_only(&rig);
    for (w, pitch) in [(63u32, 64u32), (64, 64), (1, 2), (1279, 1280)] {
        (c.target_w, c.target_h) = (w, 8);
        assert_eq!(draw(&c), 0);
        assert_eq!(v3d_flush(), 0);
        let t = rig.jobs().last().unwrap().target;
        assert_eq!((t.width, t.stride), (w, pitch * 4), "{w} wide");
        assert_eq!(target_pitch(w), crate::gl::context::tex_pitch(w), "the GL core's pitch, {w} wide");
    }
}

/// A window that is a view into a wider scan-out buffer
/// (`GlesDraw::target_pitch`: an 800x600 window in the Pi 4's 1920-pixel
/// framebuffer): the job target the builder takes carries the view's pitch
/// as its byte stride, which v3d_job.c writes as the raster load's and
/// store's `height_in_ub_or_stride` (:830, :919); width and height stay the
/// window's. The same target packed is another job, at tex_pitch(w).
#[test]
fn a_view_window_puts_its_pitch_into_the_job_targets_stride() {
    let rig = Rig::new();
    rig.init();
    let mut c = clear_only(&rig);
    (c.target_w, c.target_h, c.target_pitch) = (800, 600, 1920);
    assert_eq!(draw(&c), 0);
    let mut packed = c;
    packed.target_pitch = 0;
    assert_eq!(draw(&packed), 0);
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    let n = jobs.len();
    assert!(n >= 2, "a pitch change is another job: {n}");
    let (view, pk) = (jobs[n - 2].target, jobs[n - 1].target);
    assert_eq!((view.width, view.height, view.stride), (800, 600, 1920 * 4), "the view's pitch in bytes");
    assert_eq!((pk.width, pk.height, pk.stride), (800, 600, 800 * 4), "the packed pitch");
}

/* ---- textures ------------------------------------------------------------------- */

/// A `w` x `h` RGBA8 texture on `unit` with `levels` levels filled, each
/// texel (level, x, y, 0xa5) where the GL core keeps it (`texture::gl_level`),
/// in memory of the rig's platform.
fn texture(unit: u8, w: u32, h: u32, levels: u32) -> GlesTextureBind {
    let b = GlesTextureBind {
        unit,
        width: w,
        height: h,
        stride_texels: (w + 1) & !1,
        layers: 1,
        mip_levels: levels as u8,
        mip_filter: if levels > 1 { 1 } else { 0 },
        format: GLES_TEXFMT_RGBA8,
        ..GlesTextureBind::ZERO
    };
    let bytes = texture::gl_bytes(&b);
    let mut mem = OsalGpuMem::ZERO;
    assert_eq!(crate::osal::gpu_alloc(V3D_HEAP_DATA, bytes as usize, OSAL_MEM_DATA, &mut mem), 0);
    for level in 0..levels {
        let (off, pitch, rows) = texture::gl_level(w, h, level);
        for y in 0..rows {
            for x in 0..(w >> level).max(1) {
                let at = (off + (y * pitch + x) * 4) as usize;
                // SAFETY: inside the allocation (gl_bytes).
                unsafe {
                    core::ptr::copy_nonoverlapping(
                        [level as u8, x as u8, y as u8, 0xa5].as_ptr(),
                        (mem.cpu as *mut u8).add(at),
                        4,
                    )
                };
            }
        }
    }
    GlesTextureBind { base_gpu: mem.gpu, ..b }
}

fn words(rig: &Rig, gpu: u32, n: usize) -> Vec<u32> {
    rig.gpu_bytes(gpu, n * 4).chunks(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()
}

/// A textured draw: the unit's texture state points at a tiled image of the
/// GL core's level (the fake tiling copies the rows packed), and its sampler
/// state carries the binding's filters and wraps; both records are 32-byte
/// aligned for the P0/P1 uniforms.
#[test]
fn a_textured_draw_binds_its_unit_to_the_tiled_image_and_its_records() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    let mut t = texture(3, 6, 4, 1);
    (t.linear_filter, t.wrap_s) = (1, GLES_WRAP_CLAMP_TO_EDGE);
    d.textures[0] = t;
    d.num_textures = 1;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let draws = rig.draws();
    assert_eq!(draws[0].tmu.len(), 1);
    let u = draws[0].tmu[0];
    assert_eq!(u.unit, 3);
    assert!(
        u.tex_state.is_multiple_of(32) && u.sampler.is_multiple_of(32),
        "the uniforms OR their bits into the low five"
    );
    let st = words(&rig, u.tex_state, 6);
    assert_eq!(st[5], FAKE_STATE_MARK);
    assert_eq!(
        (st[1], st[2], st[3]),
        (6, 4, V3D_TEXTURE_DATA_FORMAT_RGBA8 << FAKE_TYPE_SHIFT),
        "6x4, one level, RGBA8"
    );
    assert_eq!(st[4], u32::from_le_bytes([V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA]));
    assert_eq!(st[0] % 4096, 0, "the image (level 0 at offset 0 in the fake layout) is page aligned");
    /* the image holds the GL core's rows, packed: texel (x, y) at (y * 6 + x) * 4 */
    let image = rig.gpu_bytes(st[0], 6 * 4 * 4);
    for y in 0..4u32 {
        for x in 0..6u32 {
            let at = ((y * 6 + x) * 4) as usize;
            assert_eq!(image[at..at + 4], [0, x as u8, y as u8, 0xa5], "({x}, {y})");
        }
    }
    let sa = words(&rig, u.sampler, 6);
    assert_eq!(sa[5], FAKE_SAMPLER_MARK);
    assert_eq!((sa[0], sa[1]), (V3D_WRAP_MODE_CLAMP, V3D_WRAP_MODE_REPEAT));
    /* Mesa: mip_filter_nearest = the min mip filter is not LINEAR, so it is
     * set with no mip filter at all */
    assert_eq!(sa[2], 0b1110, "mag linear, min nearest, mip nearest, no mips");
}

/// The fragment and the vertex stage number their sampler bindings apart:
/// binding 0 of each is its own texture, in its own table (the FS stream
/// reads one, the VS and CS streams the other).
///
/// Revert check: one table for both stages gives the VS binding 0 the FS's
/// texture (the first binding of a unit wins).
#[test]
fn each_stage_has_its_own_sampler_bindings() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.textures[0] = texture(0, 4, 4, 1);
    d.num_textures = 1;
    d.vs_textures[0] = texture(0, 8, 8, 1);
    d.num_vs_textures = 1;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let dr = &rig.draws()[0];
    assert_eq!((dr.tmu.len(), dr.vs_tmu.len()), (1, 1));
    assert_eq!((dr.tmu[0].unit, dr.vs_tmu[0].unit), (0, 0));
    assert_ne!(dr.tmu[0].tex_state, dr.vs_tmu[0].tex_state, "two textures");
    assert_eq!(words(&rig, dr.tmu[0].tex_state, 3)[1], 4, "the FS binding 0 is the 4x4");
    assert_eq!(words(&rig, dr.vs_tmu[0].tex_state, 3)[1], 8, "the VS binding 0 is the 8x8");
}

/// A mip chain: every sampled level is taken from where the GL core keeps it.
#[test]
fn a_mip_chain_is_tiled_level_by_level() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.textures[0] = texture(0, 8, 4, 4);
    d.num_textures = 1;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let u = rig.draws()[0].tmu[0];
    let st = words(&rig, u.tex_state, 6);
    assert_eq!(st[3] & 0xffff, 3, "max level");
    /* the fake layout: the levels back to back, smallest first, so level L
     * starts after every level above it; level 0 (last) is at st[0] */
    let size = |i: u32| (8u32 >> i).max(1) * (4u32 >> i).max(1) * 4;
    let image = st[0] - (1..4).map(size).sum::<u32>();
    let level_base = |level: u32| image + (level + 1..4).map(size).sum::<u32>();
    for level in 0..4u32 {
        let (lw, lh) = ((8u32 >> level).max(1), (4u32 >> level).max(1));
        let bytes = rig.gpu_bytes(level_base(level), (lw * lh * 4) as usize);
        for y in 0..lh {
            for x in 0..lw {
                let at = ((y * lw + x) * 4) as usize;
                assert_eq!(bytes[at..at + 4], [level as u8, x as u8, y as u8, 0xa5], "level {level} ({x}, {y})");
            }
        }
    }
}

/// GL_OES_texture_npot on the V3D, glmark2 terrain's case: glGenerateMipmap
/// of an 800x600 RGBA texture under this backend's table lays out its ten
/// levels (800x600 down to 1x1, floor at every step) where
/// `texture::gl_level` says, the binding of the complete, REPEAT-wrapped,
/// LINEAR_MIPMAP_LINEAR texture is sampled whole (no refusal, ten levels,
/// a linear mip filter, REPEAT on both axes), and a draw tiles every level
/// from the GL core's generated texels.
///
/// Revert check: the refusal "a mip chain on a non-power-of-two texture"
/// back in `texture::refusal_for` fails the refusal assert and the draw.
#[test]
fn a_generated_npot_chain_is_sampled_whole() {
    use crate::gl::consts::*;
    use crate::gl::texture::{
        glBindTexture, glDeleteTextures, glGenTextures, glGenerateMipmap, glTexImage2D, glTexParameteri,
    };
    let rig = Rig::new();
    rig.init();
    /* the GL core reads this backend's layout flags, as in the product;
     * TestGl's drop resets the table */
    // SAFETY: serialised by the rig's TestGl.
    unsafe { crate::backend::g_backend = &yakogl_backend_v3d };
    let (w, h) = (800u32, 600u32);
    let texels: Vec<u8> = (0..w * h).flat_map(|i| [(i % w) as u8, (i / w) as u8, 0x5a, 0xff]).collect();
    let mut name = 0;
    // SAFETY: plain GL calls on the rig's context with live pointers.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as i32,
            w as i32,
            h as i32,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            texels.as_ptr() as *const c_void,
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_REPEAT as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_REPEAT as i32);
        glGenerateMipmap(GL_TEXTURE_2D);
    }
    assert_eq!(rig.gl.error(), GL_NO_ERROR, "glGenerateMipmap of 800x600");
    let t = &rig.gl.ctx().objs().textures[name as usize];
    assert_eq!((t.chain_levels, t.levels, t.level_defined, t.level_bad), (10, 10, 0x3ff, 0));
    let sizes = [(800, 600), (400, 300), (200, 150), (100, 75), (50, 37), (25, 18), (12, 9), (6, 4), (3, 2), (1, 1)];
    for (level, &(lw, lh)) in sizes.iter().enumerate() {
        let level = level as u32;
        assert_eq!(((w >> level).max(1), (h >> level).max(1)), (lw, lh));
        let (off, pitch, rows) = texture::gl_level(w, h, level);
        assert_eq!(off, t.level_off[level as usize], "level {level}: offset");
        assert_eq!(pitch, t.level_pitch(lw), "level {level}: pitch");
        assert_eq!(rows, lh, "level {level}: rows");
    }
    let b = crate::gl::draw::texture_bind_2d(t);
    assert_eq!(texture::refusal(&b), None);
    assert_eq!((b.mip_levels, b.mip_filter, texture::levels_sampled(&b)), (10, 2, 10));
    assert!(texture::gl_bytes(&b) as usize <= t.mem.size, "the sampled levels lie inside the storage");
    let s = texture::sampler(&b);
    assert_eq!((s.wrap_s, s.wrap_t), (V3D_WRAP_MODE_REPEAT, V3D_WRAP_MODE_REPEAT));
    assert!(!s.mip_none && !s.mip_nearest && !s.min_nearest && !s.mag_nearest);
    /* level 1 texel (1, 2): the box (2..3, 4..5) of level 0, R = x, G = y */
    let (off1, pitch1, _) = texture::gl_level(w, h, 1);
    // SAFETY: inside the texture's storage (level 1's offset and pitch above).
    let gen =
        unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add((off1 + (2 * pitch1 + 1) * 4) as usize), 4) };
    assert_eq!(gen, [3, 5, 0x5a, 0xff], "level 1 (1, 2) averages level 0 (2..3, 4..5), rounding half up");

    let mut d = rig.draw();
    d.textures[0] = b;
    d.num_textures = 1;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let u = rig.draws()[0].tmu[0];
    let st = words(&rig, u.tex_state, 6);
    assert_eq!((st[1], st[2]), (800, 600));
    assert_eq!(st[3] & 0xffff, 9, "max level: ten levels");
    /* the fake layout: the levels back to back, smallest first, rows packed
     * at the level's width; level 0 (last) at st[0] */
    let size = |i: u32| (w >> i).max(1) * (h >> i).max(1) * 4;
    let image = st[0] - (1..10).map(size).sum::<u32>();
    let level_base = |level: u32| image + (level + 1..10).map(size).sum::<u32>();
    for level in [1u32, 4, 9] {
        let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
        let (off, pitch, _) = texture::gl_level(w, h, level);
        let tiled = rig.gpu_bytes(level_base(level), (lw * lh * 4) as usize);
        for y in 0..lh {
            // SAFETY: inside the texture's storage (the level's rows).
            let row = unsafe {
                core::slice::from_raw_parts(
                    (t.mem.cpu as *const u8).add((off + y * pitch * 4) as usize),
                    (lw * 4) as usize,
                )
            };
            let at = (y * lw * 4) as usize;
            assert_eq!(tiled[at..at + (lw * 4) as usize], *row, "level {level} row {y}");
        }
    }
    assert_eq!(v3d_finish(), 0);
    /* freed while the rig's OSAL, which allocated it, is the one installed */
    // SAFETY: a live name.
    unsafe { glDeleteTextures(1, &name) };
}

fn sampler_counters() -> (u32, u32) {
    let (mut p, mut r) = (0, 0);
    // SAFETY: two writable counters.
    unsafe { v3d_sampler_counters(&mut p, &mut r) };
    (p, r)
}

/// A sampler state record is packed once per job and found again: two draws
/// with the same sampler state share one record, another state gets its own,
/// and a new job packs again (its arena is another). Revert check: packing in
/// every draw gives the second draw a record of its own and (3, 0) counts.
#[test]
fn a_sampler_record_is_packed_once_per_job() {
    let rig = Rig::new();
    rig.init();
    let t = texture(0, 16, 16, 1);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    let mut other = t;
    other.wrap_s = if t.wrap_s == crate::backend::GLES_WRAP_REPEAT {
        crate::backend::GLES_WRAP_CLAMP_TO_EDGE
    } else {
        crate::backend::GLES_WRAP_REPEAT
    };
    assert_ne!(texture::sampler(&other), texture::sampler(&t), "the test's two states differ");
    assert_eq!(draw(&sampling(&rig, other)), 0);
    let draws = rig.draws();
    assert_eq!(draws[0].tmu[0].sampler, draws[1].tmu[0].sampler, "one state, one record");
    assert_ne!(draws[0].tmu[0].sampler, draws[2].tmu[0].sampler, "another state, its own record");
    assert_eq!(sampler_counters(), (2, 1));
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(sampler_counters(), (3, 1), "a new job packs its own");
    assert_eq!(v3d_finish(), 0);
}

/// A texture is tiled once per job: the next draw of the job finds its image
/// and record again; a new job tiles it again (a CPU write in between would
/// have flushed the job first, `kick_pending`).
#[test]
fn a_texture_is_tiled_once_per_job() {
    let rig = Rig::new();
    rig.init();
    let t = texture(0, 16, 16, 1);
    let mut d = rig.draw();
    d.textures[0] = t;
    d.num_textures = 1;
    assert_eq!(draw(&d), 0);
    d.do_clear = false;
    assert_eq!(draw(&d), 0);
    let draws = rig.draws();
    assert_eq!(draws[0].tmu[0].tex_state, draws[1].tmu[0].tex_state, "one image, one record");
    let (mut tilings, mut bytes) = (0u32, 0u64);
    // SAFETY: two live out-parameters.
    unsafe { v3d_texture_counters(&mut tilings, &mut bytes) };
    assert_eq!((tilings, bytes), (1, 16 * 16 * 4));
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&d), 0);
    // SAFETY: as above.
    unsafe { v3d_texture_counters(&mut tilings, &mut bytes) };
    assert_eq!(tilings, 2, "the next job tiles it again");
}

/// GL's incomplete texture samples (0, 0, 0, 1): the backend's own 1x1, made
/// at init, and nothing is tiled.
#[test]
fn an_incomplete_texture_samples_the_black_stand_in() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    let mut t = texture(1, 4, 4, 1);
    t.incomplete = 1;
    d.textures[0] = t;
    d.num_textures = 1;
    assert_eq!(draw(&d), 0);
    let u = rig.draws()[0].tmu[0];
    assert_eq!(u.tex_state, g().black.gpu as u32 + BLACK_STATE_OFF);
    let st = words(&rig, u.tex_state, 6);
    assert_eq!((st[1], st[2], st[5]), (1, 1, FAKE_STATE_MARK));
    assert_eq!(rig.gpu_bytes(st[0], 4), [0, 0, 0, 255], "(0, 0, 0, 1)");
    let mut tilings = 0u32;
    // SAFETY: a live out-parameter.
    unsafe { v3d_texture_counters(&mut tilings, core::ptr::null_mut()) };
    assert_eq!(tilings, 0);
}

/// What this cut does not sample is refused by name and leaves nothing: no
/// draw, the texture arena where it stood.
#[test]
fn a_texture_the_cut_does_not_sample_is_refused_and_leaves_nothing() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    let mut ok = texture(0, 4, 4, 1);
    d.textures[0] = ok;
    let mut cube = texture(1, 4, 4, 1);
    cube.type_ = GLES_TEXTYPE_CUBE;
    d.textures[1] = cube;
    d.num_textures = 2;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("cube map"), 1);
    assert!(rig.draws().is_empty());
    assert_eq!((g().j.tex_used, g().j.ntex), (0, 0), "the first unit's tiling was rolled back");
    ok.unit = 5;
    d.textures[0] = ok;
    d.num_textures = 1;
    assert_eq!(draw(&d), 0, "the good texture alone draws");
}

/// A texture arena that cannot take the next texture ends the job; the draw
/// goes into the next one. A texture larger than an empty arena is refused.
#[test]
fn a_full_texture_arena_ends_the_job() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.textures[0] = texture(0, 1024, 1024, 1); /* 4 MiB */
    d.num_textures = 1;
    assert_eq!(draw(&d), 0);
    d.do_clear = false;
    d.textures[0] = texture(0, 1024, 1024, 1); /* another 4 MiB: the arena is 8 */
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 2, "the first job went out");
    assert_eq!((jobs[0].draws, jobs[1].draws), (1, 1));
    d.textures[0] = texture(0, 2048, 1024, 1); /* 8 MiB: not with its records */
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("larger than the V3D's texture arena"), 1);
}

/// femtovg on ES 2.0 creates GL_LUMINANCE and GL_RGB textures besides RGBA8
/// (v3d-tex's measurement of femtovg 0.27 / Slint 1.17). On the V3D the GL
/// core keeps GL_LUMINANCE narrow (`backend::narrow_la_texels`: one byte a
/// texel, sampled as R8 with XXX1) and widens GL_RGB to RGBA8 as
/// (R, G, B, 1): there is no 3-byte TMU type (Mesa's table has no
/// R8G8B8_UNORM row).
#[test]
fn femtovgs_luminance_is_kept_narrow_and_rgb_widened_to_rgba8() {
    use crate::gl::consts::*;
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
    let rig = Rig::new();
    type Case<'a> = (crate::gl::types::GLenum, &'a [u8], u8, &'a [u8]);
    let cases: [Case; 2] = [
        (GL_LUMINANCE, &[0x40, 0x41, 0x42, 0x43], GLES_TEXFMT_L8, &[0x40, 0x41, 0x42, 0x43]),
        (GL_RGB, &[1, 2, 3, 1, 2, 3, 1, 2, 3, 1, 2, 3], GLES_TEXFMT_RGBA8, &[1, 2, 3, 0xff]),
    ];
    for (format, texels, texfmt, want) in cases {
        let mut name = 0;
        // SAFETY: plain GL calls on the rig's context with live pointers: a
        // 2x2 image packed at an unpack alignment of 1.
        unsafe {
            crate::gl::state::glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D, name);
            glTexImage2D(
                GL_TEXTURE_2D,
                0,
                format as i32,
                2,
                2,
                0,
                format,
                GL_UNSIGNED_BYTE,
                texels.as_ptr() as *const c_void,
            );
        }
        let t = &rig.gl.ctx().objs().textures[name as usize];
        assert_eq!(t.texfmt, texfmt, "0x{format:x}: the stored format");
        /* a 2-wide level's rows are 2 texels a row (pair-padded, already even) */
        // SAFETY: the texture's storage holds its first `want.len()` bytes.
        let first = unsafe { core::slice::from_raw_parts(t.mem.cpu as *const u8, want.len()) };
        assert_eq!(first, want, "0x{format:x}: the first texels as stored");
        // SAFETY: a live name, freed while the rig's OSAL is installed.
        unsafe { glDeleteTextures(1, &name) };
    }
}

/// `texture::gl_level` is where the GL core keeps each level on this build
/// (mip_level_align 0): pinned against the GL core's own `level_off` and
/// `level_pitch`, for a chain whose tail is 1 wide, a square chain and a
/// non-power-of-two single level.
#[test]
fn the_gl_cores_level_layout_is_the_one_the_backend_reads() {
    use crate::gl::consts::*;
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
    let rig = Rig::new();
    for (w, h, levels) in [(16u32, 4u32, 5u32), (8, 8, 4), (13, 7, 1)] {
        let mut name = 0;
        // SAFETY: plain GL calls on the rig's context with live pointers.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_2D, name);
            for level in 0..levels {
                let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
                let texels = std::vec![0u8; (lw * lh * 4) as usize];
                glTexImage2D(
                    GL_TEXTURE_2D,
                    level as i32,
                    GL_RGBA as i32,
                    lw as i32,
                    lh as i32,
                    0,
                    GL_RGBA,
                    GL_UNSIGNED_BYTE,
                    texels.as_ptr() as *const c_void,
                );
            }
        }
        let t = &rig.gl.ctx().objs().textures[name as usize];
        for level in 0..levels {
            let lw = (w >> level).max(1);
            let (off, pitch, _) = texture::gl_level(w, h, level);
            assert_eq!(off, t.level_off[level as usize], "{w}x{h} level {level}: offset");
            assert_eq!(pitch, t.level_pitch(lw), "{w}x{h} level {level}: pitch");
        }
        /* freed while the rig's OSAL, which allocated it, is the one installed */
        // SAFETY: a live name.
        unsafe { glDeleteTextures(1, &name) };
    }
}

/// A cube map in the GL core's storage is where `texture::gl_level_layered`
/// says: each level at the GL core's own `level_off` (packed: this backend
/// walks the layers itself, so a 16 cube's level 0 is 96 rows, not the
/// PowerVR's rounded 128), and face f of the level `f * rows * pitch * 4`
/// bytes into it. The GL core lays the storage out with THIS backend's
/// table installed, as in the product.
/// Each face of each level is uploaded with its own first texel and read
/// back from the storage at that place. A mip chain, a single level and a
/// non-power-of-two single level.
#[test]
fn the_gl_cores_cube_layout_is_the_one_the_backend_reads() {
    use crate::gl::consts::*;
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
    let rig = Rig::new();
    /* the GL core reads this backend's layout flags, as in the product;
     * TestGl's drop resets the table */
    // SAFETY: serialised by the rig's TestGl.
    unsafe { crate::backend::g_backend = &yakogl_backend_v3d };
    for (size, levels) in [(16u32, 5u32), (8, 1), (12, 1)] {
        let mut name = 0;
        // SAFETY: plain GL calls on the rig's context with live pointers.
        unsafe {
            glGenTextures(1, &mut name);
            glBindTexture(GL_TEXTURE_CUBE_MAP, name);
            for level in 0..levels {
                let l = (size >> level).max(1);
                for face in 0..6u32 {
                    let mut texels = std::vec![0u8; (l * l * 4) as usize];
                    texels[..4].copy_from_slice(&[face as u8 + 1, level as u8, 0x5a, 0xa5]);
                    glTexImage2D(
                        GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                        level as i32,
                        GL_RGBA as i32,
                        l as i32,
                        l as i32,
                        0,
                        GL_RGBA,
                        GL_UNSIGNED_BYTE,
                        texels.as_ptr() as *const c_void,
                    );
                }
            }
        }
        assert_eq!(rig.gl.error(), crate::gl::consts::GL_NO_ERROR);
        let t = &rig.gl.ctx().objs().textures[name as usize];
        assert_eq!(t.layers, 6);
        for level in 0..levels {
            let lw = (size >> level).max(1);
            let (off, pitch, rows) = texture::gl_level_layered(size, size, 6, 4, level);
            assert_eq!(off, t.level_off[level as usize], "{size} cube level {level}: offset");
            assert_eq!(pitch, t.level_pitch(lw), "{size} cube level {level}: pitch");
            for face in 0..6u32 {
                let at = (off + face * rows * pitch * 4) as usize;
                // SAFETY: inside the texture's storage (the offsets above).
                let got = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), 4) };
                assert_eq!(got, [face as u8 + 1, level as u8, 0x5a, 0xa5], "{size} cube level {level} face {face}");
            }
        }
        // SAFETY: a live name.
        unsafe { glDeleteTextures(1, &name) };
    }
}

/* ---- the clear quad ------------------------------------------------------------ */

/// A clear-only draw of the rig's target (no geometry, no program).
fn clear_only(rig: &Rig) -> GlesDraw {
    let mut c = rig.draw();
    (c.index_count, c.fs, c.num_attribs) = (0, core::ptr::null(), 0);
    c
}

/// The f32 words of `n` vertices of four floats at `gpu`.
fn vertices_at(rig: &Rig, gpu: u32, n: usize) -> Vec<[f32; 4]> {
    rig.gpu_bytes(gpu, n * 16)
        .chunks(16)
        .map(|v| core::array::from_fn(|k| f32::from_le_bytes(v[k * 4..k * 4 + 4].try_into().unwrap())))
        .collect()
}

/// What every clear quad has in common: the clear program's code (not the
/// application's), six vertices of four floats at the clear depth, the
/// colour as the fragment block the program reads dwords 0..3 of, and none
/// of blend, cull or polygon offset.
fn assert_is_the_quad(rig: &Rig, q: &SeenDraw, depth: f32, rgba: [f32; 4]) {
    assert_ne!(q.fs_code, FS_USC as u32, "the clear program's code, not the application's");
    assert_eq!((q.prim, q.count, q.indexed), (V3D_PRIM_TRIANGLES, 6, false));
    assert_eq!(q.attrs.len(), 1);
    assert_eq!((q.attrs[0].stride, q.attrs[0].components), (16, 4));
    assert_eq!(vertices_at(rig, q.attrs[0].address, 6), state::clear_quad_vertices(depth).to_vec());
    assert_eq!(q.fs_storage, rgba.map(f32::to_bits).to_vec(), "the colour is the fragment block");
    let mut read: Vec<u32> =
        q.fs_uniforms.iter().filter(|u| u.contents == V3D_QUNIFORM_UNIFORM).map(|u| u.data).collect();
    read.sort_unstable();
    read.dedup();
    assert_eq!(read, [0, 1, 2, 3], "the program reads u_color from dwords 0..3");
    assert!(!q.st.blend_enabled && !q.st.cull_front && !q.st.cull_back && !q.st.offset_tri);
}

/// A colour clear under a partial glColorMask is the TLB's no longer: the
/// job loads the colour, clears depth in the TLB as before, and the quad
/// writes the colour with the mask as COLOR_WRITE_MASKS (Mesa: the quad's
/// blend colormask, st_cb_clear.c:239 and :442). A full mask is the TLB
/// clear, with no quad.
///
/// Revert check: without `colour_needs_quad` the masked clear clears every
/// channel in the TLB and the job does not load its colour.
#[test]
fn a_masked_colour_clear_is_a_quad_and_depth_stays_in_the_tlb() {
    let rig = Rig::new();
    rig.init();
    let mut c = clear_only(&rig);
    (c.clear_color_mask, c.clear_rgba) = (0b1001, [0.25, 0.5, 0.75, 1.0]);
    (c.do_clear_depth, c.clear_depth) = (true, 0.5);
    assert_eq!(draw(&c), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let (jobs, draws) = (rig.jobs(), rig.draws());
    assert_eq!((jobs.len(), draws.len()), (1, 1));
    let clear = jobs[0].clear.unwrap();
    assert!(clear.load_rt0, "the colour is loaded, not cleared");
    assert_eq!(clear.z, 0.5, "depth is still the TLB's");
    let q = &draws[0];
    assert_is_the_quad(&rig, q, 0.5, [0.25, 0.5, 0.75, 1.0]);
    assert_eq!(q.st.colormask, 0b1001, "R and A only");
    assert!(!q.st.depth_enabled && !q.st.stencil_enabled, "the quad writes colour only");
    assert!(!q.st.scissor, "a background clear has no scissor");

    /* the full mask: the TLB clears colour, no quad */
    let mut full = clear_only(&rig);
    full.clear_color_mask = 0xf;
    rig.answer(&[]);
    assert_eq!(draw(&full), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rig.draws().len(), 1, "no second quad");
    assert!(!rig.jobs()[1].clear.unwrap().load_rt0, "the TLB clears the colour");
}

/// A masked background clear fused into a draw, with depth and stencil
/// cleared too: exactly the TLB clear of depth and stencil, ONE colour quad
/// whose COLOR_WRITE_MASKS is the GL mask, then the draw with its own state,
/// in that order, in one job (st_cb_clear.c: the TLB part first, :528 the
/// quad after it).
///
/// Revert check: without `colour_needs_quad` the job clears the colour in
/// the TLB and records one draw.
#[test]
fn a_masked_clear_fused_into_a_draw_is_tlb_depth_stencil_then_the_quad_then_the_draw() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.clear_color_mask = 0b0100;
    (d.do_clear_depth, d.clear_depth) = (true, 0.5);
    (d.do_clear_stencil, d.clear_stencil) = (true, 7);
    (d.stencil_write_mask, d.stencil_back_write_mask) = (0xff, 0xff);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let (jobs, draws) = (rig.jobs(), rig.draws());
    assert_eq!((jobs.len(), draws.len()), (1, 2), "one job: the quad and the draw");
    let clear = jobs[0].clear.unwrap();
    assert!(clear.load_rt0, "the TLB does not clear the colour: it is loaded");
    assert_eq!((clear.z, clear.stencil), (0.5, 7), "depth and stencil are the TLB's");
    assert_is_the_quad(&rig, &draws[0], 0.5, [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(draws[0].st.colormask, 0b0100, "COLOR_WRITE_MASKS is the GL mask");
    assert!(!draws[0].st.depth_enabled && !draws[0].st.stencil_enabled, "the quad writes colour only");
    assert_eq!(draws[1].fs_code, FS_USC as u32, "then the application's draw");
    assert_eq!(draws[1].st.colormask, 0xf, "with its own mask");
}

/// A `clear_quad` (the GL core's scissored or stencil-masked clear) leaves
/// the TLB alone and draws every buffer it clears, with Mesa's state
/// (st_cb_clear.c:254-268): depth ALWAYS with the write on; stencil ALWAYS,
/// REPLACE on every outcome, value mask 0xff, the clear's write mask and
/// value, both faces; the clear's scissor box.
#[test]
fn a_clear_quad_draws_every_buffer_with_the_clears_state() {
    let rig = Rig::new();
    rig.init();
    let mut c = clear_only(&rig);
    c.clear_quad = true;
    (c.scissor, c.scissor_rect) = (true, [8, 4, 16, 32]);
    c.clear_rgba = [0.0, 1.0, 0.0, 1.0];
    (c.do_clear_depth, c.clear_depth) = (true, 0.25);
    (c.do_clear_stencil, c.clear_stencil) = (true, 0x5a);
    (c.stencil_write_mask, c.stencil_back_write_mask) = (0x0f, 0x0f);
    assert_eq!(draw(&c), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let (jobs, draws) = (rig.jobs(), rig.draws());
    assert_eq!((jobs.len(), draws.len()), (1, 1));
    let clear = jobs[0].clear.unwrap();
    assert!(clear.load_rt0, "the TLB clears nothing: the colour is loaded");
    let q = &draws[0];
    assert_is_the_quad(&rig, q, 0.25, [0.0, 1.0, 0.0, 1.0]);
    assert_eq!(q.st.colormask, 0xf);
    assert!(q.st.depth_enabled && q.st.depth_writemask);
    assert_eq!(q.st.depth_func, V3D_COMPARE_FUNC_ALWAYS);
    /* both faces get the same state, so st/mesa's rule (state.rs: the back
     * state only when the faces differ) gives one STENCIL_CFG for both */
    assert!(q.st.stencil_enabled && !q.st.stencil_back_enabled);
    let f = q.st.stencil_front;
    assert_eq!(f.func, V3D_COMPARE_FUNC_ALWAYS);
    assert_eq!(
        (f.sfail_op, f.zfail_op, f.zpass_op),
        (V3D_STENCIL_OP_REPLACE, V3D_STENCIL_OP_REPLACE, V3D_STENCIL_OP_REPLACE)
    );
    assert_eq!((f.ref_, f.value_mask, f.write_mask), (0x5a, 0xff, 0x0f));
    let expect = state::scissor(&c).expect("the clear's box");
    assert!(q.st.scissor);
    assert_eq!([q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy], expect);
}

/// Each buffer alone: a quad writes only what its clear clears.
#[test]
fn a_clear_quad_writes_only_the_buffers_it_clears() {
    for (colour, depth, stencil) in [(true, false, false), (false, true, false), (false, false, true)] {
        let rig = Rig::new();
        rig.init();
        let mut c = clear_only(&rig);
        c.clear_quad = true;
        (c.do_clear, c.do_clear_depth, c.do_clear_stencil) = (colour, depth, stencil);
        c.clear_color_mask = if colour { 0xf } else { 0 };
        (c.stencil_write_mask, c.stencil_back_write_mask) = (0xff, 0xff);
        assert_eq!(draw(&c), 0, "{:?}", rig.gl.logs());
        let q = rig.draws()[0].clone();
        let what = (colour, depth, stencil);
        assert_eq!(q.st.colormask, if colour { 0xf } else { 0 }, "{what:?}");
        assert_eq!((q.st.depth_enabled, q.st.depth_writemask), (depth, depth), "{what:?}");
        assert_eq!(q.st.stencil_enabled, stencil, "{what:?}");
    }
}

/// A quad that clears depth tests it (ALWAYS), so on a target whose depth an
/// earlier job wrote it is refused: the pixels outside its box keep a value
/// no job loads.
#[test]
fn a_depth_clear_quad_after_a_job_wrote_depth_is_refused() {
    let rig = Rig::new();
    rig.init();
    let mut d = depth_draw(&rig);
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0, "the job wrote depth");
    let mut c = clear_only(&rig);
    (c.do_clear, c.clear_quad, c.do_clear_depth) = (false, true, true);
    (c.scissor, c.scissor_rect) = (true, [0, 0, 8, 8]);
    assert_eq!(draw(&c), -EINVAL);
    assert_eq!(rig.logs_with("reads depth an earlier job wrote"), 1);
}

/// A shader back end whose compile fails with the compiler's own words.
unsafe extern "C" fn failing_compile(
    _stage: u32,
    _glsl: *const c_char,
    _out: *mut ShcShader,
    log: *mut c_char,
    log_sz: usize,
) -> i32 {
    let msg = b"0:1: 'gl_Position' : the chain says no\0";
    // SAFETY: the caller's log is `log_sz` bytes.
    unsafe { core::ptr::copy_nonoverlapping(msg.as_ptr() as *const c_char, log, msg.len().min(log_sz)) };
    -EINVAL
}
/// Never reached: the compile fails first.
unsafe extern "C" fn unreached_link(_vs: *const ShcShader, _fs: *mut ShcShader, _log: *mut c_char, _sz: usize) -> i32 {
    -EINVAL
}
static FAILING: crate::shaderc::ShcBackend = crate::shaderc::ShcBackend {
    compile: Some(failing_compile),
    link: Some(unreached_link),
    blend_variant: None,
    compile_vs_linked: None,
    lod_sensitivity: None,
    sampler_variant: None,
};

/// When the clear program does not build, init says which step failed and
/// what the compiler said, once, and still starts: only the quads are
/// refused.
#[test]
fn a_clear_program_that_does_not_build_is_named_with_the_chains_error() {
    struct BackEnd(*const crate::shaderc::ShcBackend);
    impl Drop for BackEnd {
        fn drop(&mut self) {
            // SAFETY: restoring what was saved, under the rig's TestGl guard.
            unsafe { crate::shaderc::g_shaderc = self.0 };
        }
    }
    let rig = Rig::new();
    // SAFETY: the rig's guard serialises every test that swaps the back end.
    let _be = BackEnd(unsafe { crate::shaderc::g_shaderc });
    // SAFETY: as above; a static table.
    unsafe { crate::shaderc::g_shaderc = &FAILING };
    rig.init();
    let line = "clear program did not build: the vertex shader did not compile: 0:1: 'gl_Position' : the chain says no";
    /* logs() drains the recorder: take it once, count in it, print it */
    let logs = rig.gl.logs();
    assert_eq!(logs.iter().filter(|l| l.contains(line)).count(), 1, "{logs:?}");
    let mut c = clear_only(&rig);
    c.clear_color_mask = 0b0001;
    assert_eq!(draw(&c), -EINVAL, "the quad is refused");
    assert_eq!(rig.logs_with("the clear program did not build, so a clear"), 1);
    let full = clear_only(&rig);
    assert_eq!(draw(&full), 0, "the TLB clear still works");
}

/// The clear program is built once, by the product compiler, at init, which
/// says so; one that did not build refuses every quad with its own reason
/// rather than drawing it another way.
#[test]
fn the_clear_program_is_built_once_and_a_failed_one_refuses_the_quad() {
    let rig = Rig::new();
    rig.init();
    let mut c = clear_only(&rig);
    c.clear_color_mask = 0b0011;
    assert_eq!(draw(&c), 0, "{:?}", rig.gl.logs());
    assert_eq!(draw(&c), 0);
    let draws = rig.draws();
    assert_eq!(draws.len(), 2);
    assert_eq!(draws[0].fs_code, draws[1].fs_code, "one program, one upload");
    /* logs() drains the recorder: take it once for both counts */
    let logs = rig.gl.logs();
    let count = |w: &str| logs.iter().filter(|l| l.contains(w)).count();
    assert_eq!((count("clear program built:"), count("clear program did not build")), (1, 0), "{logs:?}");

    g().clear_program = ClearProgram::FAILED;
    assert_eq!(draw(&c), -EINVAL);
    assert_eq!(rig.logs_with("the clear program did not build, so a clear"), 1);
    assert_eq!(rig.draws().len(), 2, "nothing more was recorded");
}

#[test]
fn a_draw_hands_the_programs_arrays_and_uniform_blocks_over() {
    let rig = Rig::new();
    rig.init();
    assert_eq!(draw(&rig.draw()), 0);
    let d = &rig.draws()[0];
    /* the three programs at the uploaded code plus their offsets */
    assert_eq!((d.fs_code, d.vs_code, d.cs_code), (FS_USC as u32, FS_USC as u32 + 72, FS_USC as u32 + 136));
    assert_eq!(d.fs_uniforms, [V3dUniform { contents: 0, data: 0x3f80_0000 }, V3dUniform { contents: 1, data: 1 }]);
    assert_eq!(d.vs_uniforms, [V3dUniform { contents: V3D_QUNIFORM_VIEWPORT_X_SCALE, data: 0 }]);
    /* the GL core's blocks, dword for dword */
    assert_eq!(d.vs_storage, [0.5f32, 1.0, 1.5, 2.0].map(f32::to_bits));
    assert_eq!(d.fs_storage, [3.0f32, 4.0].map(f32::to_bits));
    /* the arrays in record order: location 1 is record 0, location 3 record 1 */
    assert_eq!(
        d.attrs,
        [
            V3dVertexAttr { address: (BUF + 0x40) as u32, stride: 16, components: 4, divisor: 0 },
            V3dVertexAttr { address: (BUF + 0x100) as u32, stride: 8, components: 2, divisor: 0 },
        ]
    );
    assert_eq!((d.prim, d.count, d.indexed), (V3D_PRIM_TRIANGLES, 3, false));
    /* the state: a window, so the viewport is flipped */
    assert_eq!(d.st.vp.scale[1], -32.0);
    assert_eq!(d.st.colormask, 0xf);
}

#[test]
fn a_location_the_program_does_not_read_is_skipped_and_a_missing_one_refused() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    /* a third array at location 0, which the program does not read */
    d.attribs[2] = GlesAttrib { vi_reg: 0, components: 4, stride: 16, buffer_gpu: BUF, ..GlesAttrib::ZERO };
    d.num_attribs = 3;
    assert_eq!(draw(&d), 0);
    assert_eq!(rig.draws()[0].attrs.len(), 2);
    /* location 3's array gone: record 1 has no array */
    d.num_attribs = 1;
    d.attribs[0] = d.attribs[1];
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.draws().len(), 1, "not handed to the builder");
    assert_eq!(rig.logs_with("has no array"), 1);
}

#[test]
fn staged_arrays_and_indices_are_copied_into_the_jobs_data() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    let verts: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    d.attribs[0].cpu = verts.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = 24;
    let idx: [u16; 3] = [2, 0, 1];
    d.index_cpu = idx.as_ptr() as *const c_void;
    d.index_bytes = 6;
    d.index_size = 2;
    assert_eq!(draw(&d), 0);
    let seen = &rig.draws()[0];
    let data = g().arenas.data.gpu;
    let a = seen.attrs[1].address;
    assert!(a as u64 >= data && (a as u64) < data + DATA_BYTES as u64, "in the data arena");
    assert_eq!(a % 16, 0);
    assert_eq!(rig.gpu_bytes(a, 24), verts.iter().flat_map(|f| f.to_le_bytes()).collect::<Vec<_>>());
    assert!(seen.indexed);
    assert_eq!((seen.index_type, seen.index_bytes), (V3D_INDEX_TYPE_16, 6));
    assert_eq!(rig.gpu_bytes(seen.index_buffer, 6), [2u8, 0, 0, 0, 1, 0]);
    assert_eq!(v3d_flush(), 0);
}

/* ---- the async swap: a job runs while the next is recorded -------------- */

/// A rig whose OSAL lets a submitted job run (`Platform::async_jobs`).
fn async_rig() -> Rig {
    let rig = Rig::new();
    with_platform(|p| p.async_jobs = true);
    rig.init();
    rig
}

fn counters() -> (u32, u32) {
    let (mut k, mut w) = (0, 0);
    // SAFETY: two writable counters.
    unsafe { v3d_counters(&mut k, &mut w) };
    (k, w)
}

/// A draw with its first array staged from `verts` (client memory, copied
/// into the job's data arena), and where the job put it.
fn staged_draw(rig: &Rig, verts: &[f32; 6]) -> (GlesDraw, u32) {
    let mut d = rig.draw();
    d.attribs[0].cpu = verts.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = 24;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let a = rig.draws().last().unwrap().attrs[1].address;
    (d, a)
}

fn bytes_of(verts: &[f32; 6]) -> Vec<u8> {
    verts.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// The async swap (docs: the async-swap note, 2026-09-26): `flush` submits
/// and returns with the job running; the next job is recorded into the other
/// arena set, so what the running job reads stays as it was; its kick is
/// pending until the OSAL has completed it; the next flush waits for it
/// before it submits; `finish` waits for all.
///
/// Revert checks, each failing here: recording every job into one set (no
/// swap in `choose_arenas`) overwrites job 1's staged vertices; dropping the
/// in-flight clause of `v3d_kick_pending` answers job 1 as run while the
/// GPU still reads it; the wait in `flush_job` before the submit gone, the
/// wait count after the second flush is 0.
#[test]
fn a_flushed_job_runs_while_the_next_is_recorded_into_the_other_arenas() {
    let rig = async_rig();
    let one: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let two: [f32; 6] = [7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
    let job1 = v3d_kick_id();
    let (_, a1) = staged_draw(&rig, &one);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(counters(), (1, 0), "submitted, not waited for");
    assert!(v3d_kick_pending(job1), "job 1 runs: pending until the OSAL has completed it");
    /* job 2 is recorded while job 1 runs */
    let (_, a2) = staged_draw(&rig, &two);
    assert_ne!(a1, a2);
    assert_eq!(rig.gpu_bytes(a1, 24), bytes_of(&one), "job 1's vertices, as it reads them");
    assert_eq!(rig.gpu_bytes(a2, 24), bytes_of(&two));
    assert!(v3d_kick_pending(job1));
    complete_jobs();
    assert!(!v3d_kick_pending(job1), "completed");
    /* the next flush submits job 2 behind job 1, waiting for nothing: two
     * jobs may be outstanding */
    let job2 = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    assert_eq!(counters(), (2, 0));
    assert!(v3d_kick_pending(job2));
    assert_eq!(v3d_finish(), 0);
    assert_eq!(counters(), (2, 2));
    assert!(!v3d_kick_pending(job2), "finish waited for it");
    /* job 3 goes back into job 1's set, which job 1 no longer reads */
    let (_, a3) = staged_draw(&rig, &one);
    assert_eq!(a3, a1, "the 8 MiB set again: the GPU is idle");
    assert_eq!(v3d_finish(), 0);
}

/// A job in the 8 MiB set whose texture arena fills ends; the draw's retry
/// needs the 8 MiB set again, which the job just submitted holds. The tiling
/// asks for it (`want_big_arena`): the retry waits for that job and is drawn
/// there. Revert check: without `want_big_arena` the retry takes the free
/// 1 MiB set, where the 4 MiB image never fits, and the draw is refused
/// (-E2BIG).
#[test]
fn a_tiling_too_large_for_the_small_arena_is_drawn_in_the_8_mib_set() {
    let rig = async_rig();
    let mut d = rig.draw();
    d.textures[0] = texture(0, 1024, 1024, 1); /* 4 MiB tiled */
    d.num_textures = 1;
    assert_eq!(draw(&d), 0);
    assert_eq!(g().arenas.tex_bytes, TEX_BYTES, "the GPU is idle: the 8 MiB set");
    d.do_clear = false;
    d.textures[0] = texture(0, 1024, 1024, 1); /* another 4 MiB: the arena is 8 */
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(g().arenas.tex_bytes, TEX_BYTES, "drawn in the 8 MiB set again");
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 2, "the first job went out");
    assert_eq!((jobs[0].draws, jobs[1].draws), (1, 1));
    assert_eq!(counters(), (1, 1), "the retry waited for the job holding the 8 MiB set");
    assert_eq!(v3d_finish(), 0);
    assert_eq!(rig.logs_with("does not fit an empty job"), 0);
}

/// v3d_wait_kick (the sync waits): with job 1 in flight and job 2 open,
/// waiting for kick 1 waits for job 1 alone and leaves job 2 open and
/// unsubmitted; an older kick answers at once; the open job's own kick is
/// submitted and waited for, as finish does. Revert check: wait_kick as
/// v3d_finish submits job 2 in the first case.
#[test]
fn wait_kick_waits_for_the_job_in_flight_and_leaves_the_open_job_open() {
    let rig = async_rig();
    let one: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let job1 = v3d_kick_id();
    staged_draw(&rig, &one);
    assert_eq!(v3d_flush(), 0);
    let job2 = v3d_kick_id();
    staged_draw(&rig, &one);
    assert!(g().j.open && v3d_kick_pending(job1));
    assert_eq!(v3d_wait_kick(job1), 0);
    assert!(!v3d_kick_pending(job1), "job 1 has run");
    assert!(g().j.open, "job 2 is still being recorded");
    assert_eq!(rig.kicks().len(), 1, "job 2 was not submitted");
    assert_eq!(counters(), (1, 1));
    /* an older kick: at once, nothing waited */
    assert_eq!(v3d_wait_kick(job1), 0);
    assert_eq!(counters(), (1, 1));
    /* the open job's own kick: submitted and waited for */
    assert_eq!(v3d_wait_kick(job2), 0);
    assert!(!g().j.open && !v3d_kick_pending(job2));
    assert_eq!(rig.kicks().len(), 2);
    assert_eq!(v3d_wait_kick(0), 0, "kick 0 is never pending");
}

/// Two jobs are outstanding at most (`flush_job`): a multi-pass frame's
/// first off-screen submit waits for the frame before's off-screen job, no
/// longer for its window job (#1141, docs/notes/egl-swap-timeline.md). Frame
/// 1: an off-screen pass A, the window pass W; frame 2: an off-screen pass B.
/// Before the queue B's submit waited for W, so the CPU recorded B's frame
/// beside W only until then. Revert check: with `wait_in_flight` in
/// `flush_job` (instead of `wait_queued`), W's flush already waits for A and
/// the first counters assert fails.
#[test]
fn a_frames_first_offscreen_submit_does_not_wait_for_the_previous_window_job() {
    let rig = async_rig();
    let one: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let mut off = rig.draw();
    off.target_gpu = TARGET + 0x10_0000;
    off.attribs[0].cpu = one.as_ptr() as *const c_void;
    off.attribs[0].cpu_bytes = 24;
    let window = |rig: &Rig| {
        let mut d = rig.draw();
        d.attribs[0].cpu = one.as_ptr() as *const c_void;
        d.attribs[0].cpu_bytes = 24;
        d
    };
    /* frame 1 */
    assert_eq!(draw(&off), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let kick_a = g().kicks;
    assert_eq!(draw(&window(&rig)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0, "the swap");
    let kick_w = g().kicks;
    assert_eq!(counters(), (kick_w, 0), "jobs submitted, nothing waited for: A and W are outstanding");
    assert!(v3d_kick_pending(kick_a), "A");
    assert!(v3d_kick_pending(kick_w), "W");
    /* frame 2's first off-screen pass */
    assert_eq!(draw(&off), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let kick_b = g().kicks;
    assert_eq!(counters(), (kick_b, 1), "B's submit waited once: for A, the job two back");
    assert!(!v3d_kick_pending(kick_a), "A has run");
    assert!(v3d_kick_pending(kick_w), "the window job of frame 1 is still outstanding");
    assert!(v3d_kick_pending(kick_b));
    /* a wait for the newer job waits for both */
    assert_eq!(v3d_wait_kick(kick_b), 0);
    assert!(!v3d_kick_pending(kick_w) && !v3d_kick_pending(kick_b));
    assert_eq!(v3d_finish(), 0);
}

/// The EGL swap shows the previous frame after `v3d_finish_previous`: with two
/// jobs outstanding that call waits for the older one, so a single-pass scene
/// (window frame after window frame) never shows a frame the GPU still draws.
/// Revert check: with a `v3d_finish_previous` that returns 0 without waiting,
/// the first window job is still pending and the assert below fails.
#[test]
fn finish_previous_waits_for_the_older_of_two_window_jobs() {
    let rig = async_rig();
    let one: [f32; 6] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let window = |rig: &Rig| {
        let mut d = rig.draw();
        d.attribs[0].cpu = one.as_ptr() as *const c_void;
        d.attribs[0].cpu_bytes = 24;
        d
    };
    assert_eq!(draw(&window(&rig)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let kick_1 = g().kicks;
    assert_eq!(draw(&window(&rig)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let kick_2 = g().kicks;
    assert_eq!(counters(), (kick_2, 0), "both outstanding, nothing waited for");
    assert!(v3d_kick_pending(kick_1) && v3d_kick_pending(kick_2));
    /* the swap of frame 2 */
    assert_eq!(v3d_finish_previous(), 0);
    assert!(!v3d_kick_pending(kick_1), "the first window job has completed");
    assert!(v3d_kick_pending(kick_2), "the newest may still run");
    assert_eq!(counters(), (kick_2, 1), "one wait, for the older job");
    assert_eq!(v3d_finish(), 0);
}

/// DEMO13's direct loop on the async OSAL (records/pi4/record_DEMO13_cluster_demo13_4ed1c629.txt):
/// two window surfaces drawn in turn, Slint's FBO depth buffers filling the
/// other zs slots, a two-job frame now and then on the second surface, and
/// the GL core's eglClientWaitSync(previous frame) between drawing a frame
/// and swapping it, which calls finish while the frame is open. Everything
/// must be drawn, the jobs must complete, and the depth/stencil buffers and
/// the GPU memory must stop growing.
#[test]
fn the_demo13_direct_loop_draws_every_frame_and_holds_its_memory() {
    let rig = async_rig();
    /* the check part's FBOs: depth written, holding slots */
    for id in 1..=6u32 {
        let mut d = kept_depth_draw(&rig, id);
        (d.do_clear_depth, d.clear_depth) = (true, 1.0);
        assert_eq!(draw(&d), 0);
        assert_eq!(v3d_finish(), 0);
    }
    let mut halves = [stencil_stroke(&rig), stencil_stroke(&rig)];
    (halves[0].ds_persist, halves[0].ds_id) = (true, 0x8000_1000);
    (halves[1].ds_persist, halves[1].ds_id, halves[1].target_gpu) = (true, 0x8000_2000, TARGET + 0x10_0000);
    let base_refused = v3d_refused_draws();
    let mut fence = 0u32;
    let (mut allocs_at_20, mut live_at_20) = (0, 0);
    for f in 0..200u32 {
        let h = &halves[(f % 2) as usize];
        assert_eq!(draw(h), 0, "frame {f}: {:?}", rig.gl.logs());
        if f % 2 == 1 && f % 3 == 0 {
            assert_eq!(v3d_flush(), 0, "frame {f}: a two-job frame");
            assert_eq!(draw(h), 0, "frame {f}, second job");
        }
        /* eglClientWaitSync(the previous frame's fence) */
        if f > 0 && v3d_kick_pending(fence) {
            assert_eq!(v3d_finish(), 0, "frame {f}: the wait");
        }
        /* eglSwapBuffers */
        v3d_surface_swapped(h.target_gpu);
        assert_eq!(v3d_flush(), 0, "frame {f}: the swap");
        assert_eq!(v3d_finish_previous(), 0);
        /* eglCreateSync after the swap: the last kick submitted */
        fence = v3d_kick_id().wrapping_sub(1);
        if f % 3 == 0 {
            complete_jobs(); /* the GPU finishing on its own */
        }
        if f == 20 {
            allocs_at_20 = with_platform(|p| p.allocs.len());
            live_at_20 = g().zsbufs.iter().filter(|b| b.live()).count();
        }
    }
    assert_eq!(v3d_finish(), 0);
    assert_eq!(v3d_refused_draws(), base_refused, "nothing refused");
    assert!(!v3d_kick_pending(fence), "the last fence completed");
    let live = g().zsbufs.iter().filter(|b| b.live()).count();
    assert_eq!(live, live_at_20, "no depth/stencil buffer added after frame 20");
    assert_eq!(with_platform(|p| p.allocs.len()), allocs_at_20, "no allocation growth after frame 20");
    let logs = rig.gl.logs();
    assert!(!logs.iter().any(|l| l.contains("given up") || l.contains("no depth/stencil")), "{logs:?}");
}

/// A job begun while the job in flight holds the 8 MiB set records into the
/// 1 MiB one, and a job begun after the flush that waits for it (the GPU idle
/// again) into the 8 MiB one.
#[test]
fn a_job_begun_while_the_8_mib_set_runs_takes_the_small_one() {
    let rig = async_rig();
    assert_eq!(draw(&rig.draw()), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&rig.draw()), 0);
    assert_eq!(g().arenas.tex_bytes, SPARE_TEX_BYTES, "the job in flight holds the 8 MiB set");
    assert_eq!(v3d_finish(), 0);
    assert_eq!(draw(&rig.draw()), 0);
    assert_eq!(g().arenas.tex_bytes, TEX_BYTES);
    assert_eq!(v3d_finish(), 0);
}

#[test]
fn a_buffer_index_list_is_addressed_in_place() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    (d.index_buf_gpu, d.index_size, d.index_count) = (BUF + 0x800, 1, 6);
    assert_eq!(draw(&d), 0);
    let s = &rig.draws()[0];
    assert_eq!((s.index_buffer, s.index_type, s.index_bytes, s.count), ((BUF + 0x800) as u32, V3D_INDEX_TYPE_8, 6, 6));
    d.index_size = 3;
    assert_eq!(draw(&d), -EINVAL, "no such index size");
}

/// dEQP fbo.render's tex2d cases on the Pi 4 (56 "Image comparison failed"
/// since the async swap, S8FBOC c9679aa5437e5c06/76df105f93a126df): a job
/// renders into a texture (an FBO's colour attachment), the target changes
/// and that job is submitted and left running (6b6ec1e1), and the next job
/// samples the texture. Its tiled copy is made on the CPU from the GL core's
/// storage, so the job that renders into it must have completed first: the
/// binding's `target_kick` names it (draw.rs, framebuffer::texture_target_use).
/// The rig's GPU writes 0x5a into the target when the job completes; the kept
/// copy, read back, holds 0x5a (the render), not the texels from before it.
///
/// Revert check: without the wait in tex_state_for the copy holds the
/// texture's upload pattern (the render not yet done).
#[test]
fn a_texture_rendered_by_the_job_in_flight_is_tiled_after_that_job_completes() {
    let rig = async_rig();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 5;
    let bytes = texture::gl_bytes(&t) as usize;
    /* the render into the texture: its own target, the texture's storage */
    let mut r = rig.draw();
    (r.target_gpu, r.target_w, r.target_h, r.viewport) = (t.base_gpu, 16, 16, [0, 0, 16, 16]);
    assert_eq!(draw(&r), 0, "{:?}", rig.gl.logs());
    let kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    let fence = with_platform(|p| p.kicks.len());
    with_platform(|p| p.renders.push((fence, t.base_gpu, bytes, 0x5a)));
    assert!(v3d_kick_pending(kick), "submitted and running (the async swap)");
    /* the next job, another target, samples it */
    t.target_kick = kick;
    let (_, waits) = counters();
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    let waited = counters().1 - waits;
    assert_eq!(v3d_flush(), 0);
    complete_jobs();
    let size = texture::layout(&t).unwrap().size as usize;
    let copy = copy_image(t.base_gpu, size);
    assert!(copy.iter().all(|&b| b == 0x5a), "the copy holds the render: {:x?}", &copy[..16]);
    assert_eq!(waited, 1, "the render in flight was waited for, once");
    let (mut w, mut a) = (0, 0);
    // SAFETY: two writable counters.
    unsafe { v3d_tex_waits(&mut w, &mut a) };
    assert_eq!((w, a), (1, 0), "one texture wait, by the job's own kick");
}

/// FBOR5's 16 (resize.tex2d_*, color.blend_*tex2d*_depth_component16):
/// the render into the texture is the draw that ends the open job (the
/// window's), so the GL core's stamp of the texture (taken before the
/// backend sees the draw) names the window's job, and the render lands in
/// the next one. `open_alias` answers that stamp while the job is open; once
/// the job is submitted and running (the async swap) the stamp must still
/// be pending until it completes, or the sampling draw tiles the texture
/// before the render. The rig's GPU writes 0x5a into the texture when the
/// render's job completes; the kept copy, read back, holds 0x5a.
///
/// Revert check: without the alias kept in `InFlight`, kick_pending(stamp)
/// is false once the render's job is submitted, nothing waits, and the copy
/// holds the upload pattern.
#[test]
fn a_render_stamped_with_the_job_it_ended_is_waited_for_after_its_submit() {
    let rig = async_rig();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 5;
    let bytes = texture::gl_bytes(&t) as usize;
    /* the window's draw: a job left open */
    assert_eq!(draw(&rig.draw()), 0);
    /* the GL core stamps the render target with the open job's id, then
     * the render (another target) ends that job and opens the next */
    let stamp = v3d_kick_id();
    let mut r = rig.draw();
    (r.target_gpu, r.target_w, r.target_h, r.viewport) = (t.base_gpu, 16, 16, [0, 0, 16, 16]);
    assert_eq!(draw(&r), 0, "{:?}", rig.gl.logs());
    assert_ne!(v3d_kick_id(), stamp, "the render is in the job after the stamped one");
    assert_eq!(v3d_flush(), 0);
    let fence = with_platform(|p| p.kicks.len());
    with_platform(|p| p.renders.push((fence, t.base_gpu, bytes, 0x5a)));
    let pending = v3d_kick_pending(stamp);
    t.target_kick = stamp;
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    complete_jobs();
    let size = texture::layout(&t).unwrap().size as usize;
    let copy = copy_image(t.base_gpu, size);
    assert!(copy.iter().all(|&b| b == 0x5a), "the copy holds the render: {:x?}", &copy[..16]);
    assert!(pending, "the stamp stays pending while the render's job runs");
    let (mut w, mut a) = (0, 0);
    // SAFETY: two writable counters.
    unsafe { v3d_tex_waits(&mut w, &mut a) };
    assert_eq!((w, a), (1, 1), "one texture wait, by the alias");
    assert!(!v3d_kick_pending(stamp), "and not pending once it has run");
}

/// A texture whose render has completed costs no wait: only a pending
/// target_kick is waited for (the cluster samples its layers every frame).
#[test]
fn a_texture_rendered_by_a_completed_job_is_tiled_without_a_wait() {
    let rig = async_rig();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 5;
    let mut r = rig.draw();
    (r.target_gpu, r.target_w, r.target_h, r.viewport) = (t.base_gpu, 16, 16, [0, 0, 16, 16]);
    assert_eq!(draw(&r), 0);
    let kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    complete_jobs();
    assert!(!v3d_kick_pending(kick));
    t.target_kick = kick;
    let (_, waits) = counters();
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(counters().1, waits, "nothing to wait for");
    assert_eq!(v3d_flush(), 0);
}

/* ---- jobs ------------------------------------------------------------------------- */

#[test]
fn a_new_target_ends_the_job_and_a_clear_after_draws_starts_one() {
    let rig = Rig::new();
    rig.init();
    let d = rig.draw();
    assert_eq!(draw(&d), 0);
    let mut d2 = d;
    d2.do_clear = false;
    assert_eq!(draw(&d2), 0, "the same target, no clear: the same job");
    assert_eq!(rig.jobs().len(), 1);
    /* another target: the first job goes out */
    let mut fbo = d2;
    fbo.target_gpu = TARGET + 0x10_0000;
    fbo.flip_y = false;
    assert_eq!(draw(&fbo), 0);
    assert_eq!(rig.kicks().len(), 1);
    let jobs = rig.jobs();
    assert_eq!((jobs.len(), jobs[0].draws, jobs[1].draws), (2, 2, 1));
    assert!(jobs[1].clear.is_none(), "still open");
    /* a clear after that draw: the draw's job goes out, the clear opens one */
    let mut c = fbo;
    (c.do_clear, c.clear_rgba) = (true, [0.0, 0.0, 1.0, 1.0]);
    assert_eq!(draw(&c), 0);
    let last_use = v3d_kick_id();
    assert_eq!(v3d_finish(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 3);
    let (c1, c2) = (jobs[1].clear.unwrap(), jobs[2].clear.unwrap());
    assert!(c1.load_rt0, "the FBO job did not clear: it loads");
    assert!(!c2.load_rt0 && c2.rgba8 == 0xffff_0000, "the new job clears blue");
    assert_eq!(rig.kicks().len(), 3);
    /* the last draw's kick has run (kick_id() itself now names the next
     * kick, which nothing has been recorded into) */
    assert!(!v3d_kick_pending(last_use));
}

/// A colour format with fewer than 8 alpha bits is drawn, in dEQP
/// lifetime's shape (glsLifetimeTests.cpp:474-490: an RGBA4 renderbuffer,
/// drawn and read back) and gl_test 67's (an RGB target). The GL core stores
/// both as RGBA8. With no alpha (RGB, RGB565) the job loads alpha as 1 and no
/// draw writes it, so the stored alpha stays 1; the TLB clear's alpha is the
/// GL core's quantised 1. With 4 alpha bits it is an RGBA8 target.
///
/// Revert checks: without `force_alpha_1` in begin_job, or without the
/// alpha mask in job_state, this fails; with the old refusal, nothing is drawn.
#[test]
fn a_target_with_fewer_alpha_bits_is_drawn_and_one_with_none_keeps_alpha_one() {
    let rig = Rig::new();
    rig.init();
    /* an RGB565 FBO: cleared (GL's clear alpha, quantised to 0 bits: 1.0), then drawn */
    let mut rgb = rig.draw();
    (rgb.target_gpu, rgb.flip_y, rgb.target_alpha_bits) = (TARGET + 0x10_0000, false, 0);
    rgb.clear_rgba = [0.0, 0.0, 1.0, 1.0];
    assert_eq!(draw(&rgb), 0, "{:?}", rig.gl.logs());
    let mut again = rgb;
    again.do_clear = false;
    assert_eq!(draw(&again), 0);
    /* an RGBA4 FBO, not cleared (lifetime's shape): loaded, every channel written */
    let mut rgba4 = again;
    (rgba4.target_gpu, rgba4.target_alpha_bits) = (TARGET + 0x20_0000, 4);
    assert_eq!(draw(&rgba4), 0);
    /* the RGB565 target again: its job loads, alpha as 1 */
    assert_eq!(draw(&again), 0);
    assert_eq!(v3d_flush(), 0);
    let (jobs, draws) = (rig.jobs(), rig.draws());
    assert_eq!((jobs.len(), draws.len()), (3, 4), "{:?}", rig.gl.logs());
    let c: Vec<_> = jobs.iter().map(|j| j.clear.unwrap()).collect();
    assert!(!c[0].load_rt0 && c[0].rgba8 == 0xffff_0000, "the RGB clear: blue, alpha 0xff");
    assert!(c[0].force_alpha_1 && c[2].force_alpha_1, "an RGB target's jobs load alpha as 1");
    assert!(c[1].load_rt0 && !c[1].force_alpha_1, "the RGBA4 target is loaded as it is");
    assert!(c[2].load_rt0);
    let masks: Vec<_> = draws.iter().map(|d| d.st.colormask).collect();
    assert_eq!(masks, [0x7, 0x7, 0xf, 0x7], "no draw into an RGB target writes alpha");
    assert_eq!(rig.logs_with("alpha bits"), 0);
}

#[test]
fn a_full_job_is_submitted_and_the_draw_goes_into_the_next() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    assert_eq!(draw(&d), 0);
    d.do_clear = false;
    rig.answer(&[V3D_JOB_FULL]);
    assert_eq!(draw(&d), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 2);
    assert_eq!((jobs[0].draws, jobs[1].draws), (1, 1));
    assert!(!jobs[0].clear.unwrap().load_rt0, "the first job clears");
    assert_eq!(rig.kicks().len(), 1);
    assert_eq!(v3d_flush(), 0);
    assert!(rig.jobs()[1].clear.unwrap().load_rt0, "the second loads what the first drew");
    /* full even in a fresh job: too large */
    rig.answer(&[V3D_JOB_FULL, V3D_JOB_FULL]);
    assert_eq!(draw(&d), -E2BIG);
    assert_eq!(rig.logs_with("does not fit an empty job"), 1);
}

/// The lines a run logged, and whether any repeats: the per-draw log rule
/// (osal `glog_once!`): at most one line per cause, however many draws.
fn repeated_lines(lines: &[String]) -> Vec<(String, usize)> {
    let mut seen: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for l in lines {
        *seen.entry(l.as_str()).or_default() += 1;
    }
    seen.into_iter().filter(|(_, n)| *n > 1).map(|(l, n)| (l.to_string(), n)).collect()
}

/// 100 draws no empty job holds (-E2BIG) log at most one line per cause,
/// the repeats counted: the per-draw log rule (osal `glog_once!`).
#[test]
fn a_draw_refused_100_times_logs_each_cause_once() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    assert_eq!(draw(&d), 0);
    d.do_clear = false;
    let _ = rig.gl.logs();
    let (_, repeats0) = crate::osal::log_repeats();
    for _ in 0..100 {
        rig.answer(&[V3D_JOB_FULL, V3D_JOB_FULL]);
        assert_eq!(draw(&d), -E2BIG);
    }
    let lines = rig.gl.logs();
    assert_eq!(repeated_lines(&lines), [], "{lines:?}");
    assert!(lines.len() <= 3, "a few causes, not a line per draw: {lines:?}");
    assert!(crate::osal::log_repeats().1.wrapping_sub(repeats0) >= 99, "the refusal ran 100 times: {lines:?}");
}

#[test]
fn staged_data_larger_than_the_arena_gets_memory_of_its_own_or_is_too_big() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    let big: Vec<u8> = (0..DATA_BYTES + 16).map(|i| (i % 251) as u8).collect();
    d.attribs[0].cpu = big.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = big.len() as u32;
    /* no memory for the piece: refused, the arena where it stood */
    with_platform(|p| p.fail_alloc_from = Some(DATA_BYTES));
    assert_eq!(draw(&d), -E2BIG);
    with_platform(|p| p.fail_alloc_from = None);
    assert_eq!((g().j.data_used, g().j.nspills), (0, 0), "the arena is back where it stood");
    /* with memory: recorded from GPU memory of its own, byte for byte */
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let a = rig.draws().last().unwrap().attrs[1].address;
    let data = g().arenas.data.gpu;
    assert!((a as u64) < data || a as u64 >= data + DATA_BYTES as u64, "outside the data arena");
    assert_eq!(rig.gpu_bytes(a, big.len()), big);
    assert_eq!(g().j.nspills, 1);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(g().j.nspills, 0, "given back once the job has run");
}

/// dEQP texture.vertex.2d's draw (es2fVertexTextureTests.cpp:78, 316-373):
/// a 127x127 quad grid from client arrays, 64,516 vec2 positions and
/// texcoords and 96,774 16-bit indices, 1,225,804 bytes for one draw. No
/// piece is larger than the arena, but the three are, so it never fit a
/// job and every cell was refused (all 45 2D cases drew the clear colour
/// after the TMU fix, 2026-09-25). Now it is recorded: the pieces the
/// arena takes in it, the rest in memory of its own, every byte where the
/// record points, and that memory given back after the job. Revert check:
/// without the spill the draw is -E2BIG again.
#[test]
fn deqp_vertex_2d_grid_draw_is_recorded_whole() {
    let rig = Rig::new();
    rig.init();
    let before = with_platform(|p| p.allocs.len());
    let mut d = rig.draw();
    let pos: Vec<u8> = (0..516_128usize).map(|i| (i % 253) as u8).collect();
    let tex: Vec<u8> = (0..516_128usize).map(|i| (i % 241) as u8 ^ 0x5a).collect();
    let idx: Vec<u8> = (0..193_548usize).map(|i| (i % 239) as u8).collect();
    d.attribs[0].cpu = pos.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = pos.len() as u32;
    d.attribs[1].cpu = tex.as_ptr() as *const c_void;
    d.attribs[1].cpu_bytes = tex.len() as u32;
    d.index_cpu = idx.as_ptr() as *const c_void;
    (d.index_bytes, d.index_size, d.index_count) = (idx.len() as u32, 2, 96_774);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let seen = rig.draws().last().unwrap().clone();
    /* attribs[0] is location 3, fetched through record 1; attribs[1] record 0 */
    assert_eq!(rig.gpu_bytes(seen.attrs[1].address, pos.len()), pos);
    assert_eq!(rig.gpu_bytes(seen.attrs[0].address, tex.len()), tex);
    assert_eq!(rig.gpu_bytes(seen.index_buffer, idx.len()), idx);
    assert_eq!(seen.index_bytes, idx.len() as u32);
    assert!(g().j.nspills >= 1, "at least one piece outside the arena");
    assert_eq!(rig.logs_with("outgrew the job's data arena"), 1);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(with_platform(|p| p.allocs.len()), before, "the pieces given back once the job has run");
}

#[test]
fn a_refused_draw_leaves_the_data_arena_where_it_stood() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    let verts = [0u8; 64];
    d.attribs[0].cpu = verts.as_ptr() as *const c_void;
    d.attribs[0].cpu_bytes = 64;
    rig.answer(&[V3D_JOB_REFUSED]);
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(g().j.data_used, 0);
    assert_eq!(rig.logs_with("job builder refused"), 1);
}

/// A draw's kick is pending until the job that holds it has run: the GL core
/// reads `kick_id()` into a texture's or a buffer's `last_use` at the draw,
/// and before a CPU write it flushes and finishes while `kick_pending` says
/// so (`texture.rs` `settle_texture`). The id is the job being recorded, as
/// the PowerVR's is (the last fence + 1).
///
/// Revert check: the old `kick_pending` answered false always, so a draw in
/// the open job read as run and the GL core wrote under it.
#[test]
fn a_draw_in_the_open_job_is_pending_until_its_job_has_run() {
    let rig = Rig::new();
    rig.init();
    let first = v3d_kick_id();
    assert_ne!(first, 0, "0 means no kick");
    assert_eq!(draw(&rig.draw()), 0);
    let last_use = v3d_kick_id();
    assert_eq!(last_use, first, "the draw is in the job being recorded");
    assert!(v3d_kick_pending(last_use), "its job has not been submitted");
    /* a second draw in the same job: the same kick */
    let mut d = rig.draw();
    d.do_clear = false;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_kick_id(), last_use);
    assert_eq!(rig.jobs().len(), 1);
    /* what the GL core does before a CPU write: flush, finish */
    assert_eq!(v3d_flush(), 0);
    assert_eq!(v3d_finish(), 0);
    assert!(!v3d_kick_pending(last_use), "the job ran");
    assert_eq!(rig.kicks().len(), 1);
    /* the next draw is in the next kick */
    assert!(v3d_kick_id() > last_use);
    assert!(!v3d_kick_pending(0), "0 is never pending");
}

#[test]
fn a_failed_wait_is_reported() {
    let rig = Rig::new();
    rig.init();
    assert_eq!(draw(&rig.draw()), 0);
    with_platform(|p| p.wait_rc = -110);
    /* the flush submits and lets the job run (the async swap): its result
     * comes with the wait */
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rig.logs_with("did not complete"), 0);
    assert_eq!(v3d_finish(), -110);
    assert_eq!(rig.logs_with("did not complete"), 1);
    let (mut k, mut w) = (0, 0);
    // SAFETY: two writable counters.
    unsafe { v3d_counters(&mut k, &mut w) };
    assert_eq!((k, w), (1, 1));
}

/* ---- depth and stencil between jobs ------------------------------------------------ */

fn depth_draw(rig: &Rig) -> GlesDraw {
    let mut d = rig.draw();
    (d.depth_test, d.depth_write, d.depth_func) = (true, true, GLES_CMP_LESS);
    d
}

#[test]
fn a_depth_draw_after_a_job_that_wrote_depth_is_refused_until_a_clear() {
    let rig = Rig::new();
    rig.init();
    let mut d = depth_draw(&rig);
    d.do_clear_depth = true;
    d.clear_depth = 1.0;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0, "the job wrote depth, and nothing keeps it");
    /* the next job cannot see that depth */
    let mut again = depth_draw(&rig);
    again.do_clear = false;
    assert_eq!(draw(&again), -EINVAL);
    assert_eq!(rig.logs_with("reads depth an earlier job wrote"), 1);
    /* colour-only draws go on */
    let mut colour = again;
    colour.depth_test = false;
    assert_eq!(draw(&colour), 0);
    /* a depth clear makes it readable again, in a job of its own */
    again.do_clear_depth = true;
    assert_eq!(draw(&again), 0);
    assert_eq!(rig.draws().len(), 3);
}

#[test]
fn a_clear_with_no_draw_carries_its_value_into_the_next_job() {
    let rig = Rig::new();
    rig.init();
    let mut c = rig.draw();
    (c.index_count, c.do_clear_depth, c.clear_depth, c.do_clear_stencil, c.clear_stencil) = (0, true, 0.25, true, 7);
    assert_eq!(draw(&c), 0);
    assert_eq!(v3d_flush(), 0);
    let mut d = depth_draw(&rig);
    (d.do_clear, d.stencil) = (false, true);
    assert_eq!(draw(&d), 0, "the depth and stencil are known: 0.25 and 7");
    assert_eq!(v3d_flush(), 0);
    let clear = rig.jobs()[1].clear.unwrap();
    assert_eq!((clear.z, clear.stencil, clear.load_rt0), (0.25, 7, true));
}

#[test]
fn an_untouched_target_may_read_its_undefined_depth() {
    let rig = Rig::new();
    rig.init();
    let mut d = depth_draw(&rig);
    d.do_clear = false;
    assert_eq!(draw(&d), 0, "GL leaves a never-cleared depth undefined");
}

#[test]
fn stencil_is_tracked_apart_from_depth() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    (d.stencil, d.stencil_write_mask, d.do_clear_stencil) = (true, 0xff, true);
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    let mut s = rig.draw();
    (s.do_clear, s.stencil) = (false, true);
    assert_eq!(draw(&s), -EINVAL);
    assert_eq!(rig.logs_with("reads stencil"), 1);
    let mut z = depth_draw(&rig);
    z.do_clear = false;
    assert_eq!(draw(&z), 0, "the depth was never written: still readable");
}

#[test]
fn once_the_history_is_full_a_new_target_counts_as_written() {
    let rig = Rig::new();
    rig.init();
    for i in 0..ZS_TARGETS as u64 {
        let mut d = rig.draw();
        d.target_gpu = TARGET + i * 0x10_0000;
        assert_eq!(draw(&d), 0);
        assert_eq!(v3d_flush(), 0);
    }
    let mut d = depth_draw(&rig);
    (d.target_gpu, d.do_clear) = (TARGET + 0x100_0000, false);
    assert_eq!(draw(&d), -EINVAL, "an unknown history is never read as an empty one");
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0);
}

/* ---- refusals ----------------------------------------------------------------------- */

#[test]
fn refusals_are_logged_once_per_cause() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.samples = 2;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("a sample count other than 1 and 4"), 1);
    let mut t = rig.draw();
    t.index_count2 = 3;
    assert_eq!(draw(&t), -EINVAL);
    assert_eq!(draw(&t), -EINVAL);
    assert_eq!(rig.logs_with("a second index list in one draw is not implemented"), 1);
    assert!(rig.draws().is_empty() && rig.jobs().is_empty(), "nothing was recorded");
}

/// The V3D draws one instance (v3d_job.c: SET_INSTANCEID 0, no instanced
/// primitive packet), so a draw of more than one instance is refused and
/// counted; drawing only its first instance would be a wrong picture with no
/// error. glDrawArraysInstanced(.., 4) and glDrawElementsInstanced(.., 4)
/// reach here as instance_count 4 (gl::draw's instanced tests pin that);
/// one instance, and a plain draw (0), are drawn. Since gl_InstanceID
/// compiles (v3d-m5, 2026-09-26), this refusal is where gl_test 47 stops.
/// Revert check: without state::refusal's instance_count arm the draws of 4
/// are recorded.
/// An instanced draw (ES 3.0, gl_test 47): the instance count reaches the
/// job builder, which emits the instanced packet (tools/v3d/job_check.c);
/// a per-instance array (the GL core's divisor 1) gets the record's Instance
/// Divisor 1, a per-vertex one 0. One instance and a plain draw stay one
/// instance, and a per-instance array there still reads element 0 (divisor
/// 1 with the instance ID 0). Revert checks: the instance count not passed
/// (`instances: d.instance_count`) or the divisor not set fail here.
#[test]
fn an_instanced_draw_carries_its_instances_and_the_per_instance_divisor() {
    let rig = Rig::new();
    rig.init();
    let base = v3d_refused_draws();
    for (indexed, n) in [(false, 4), (true, 4), (false, 1), (false, 0)] {
        let mut d = rig.draw();
        if indexed {
            d.index_size = 2;
            d.index_buf_gpu = BUF + 0x800;
        }
        d.instance_count = n;
        d.attribs[1].per_instance = 1; /* location 1: one colour per instance */
        assert_eq!(draw(&d), 0, "indexed {indexed}, {n} instances: {:?}", rig.gl.logs());
        let seen = rig.draws().last().unwrap().clone();
        assert_eq!((seen.instances, seen.indexed), (n, indexed));
        let divisors: Vec<u32> = seen.attrs.iter().map(|a| a.divisor).collect();
        /* record 0 is location 1 (the per-instance one), record 1 location 3 */
        assert_eq!(divisors, [1, 0], "indexed {indexed}, {n} instances");
    }
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 0, "none refused");
}

/// A per-instance array's divisor reaches its record (the GL core passes up
/// to max_instance_divisor, 0xffff); 0 from an older producer means 1.
#[test]
fn a_per_instance_divisor_reaches_the_record() {
    let rig = Rig::new();
    rig.init();
    for (given, want) in [(3u16, 3u32), (0, 1), (0xffff, 0xffff)] {
        let mut d = rig.draw();
        d.instance_count = 7;
        (d.attribs[1].per_instance, d.attribs[1].divisor) = (1, given);
        assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
        let divisors: Vec<u32> = rig.draws().last().unwrap().attrs.iter().map(|a| a.divisor).collect();
        assert_eq!(divisors, [want, 0], "divisor {given}");
    }
}

/// One instance of an instanced draw the GL core draws instance by instance
/// (GlesDraw::instance_id) is refused by name when the program reads
/// gl_InstanceID: the hardware's instance ID would be 0 there. A program
/// that does not read it is drawn, and instance 0 is drawn either way.
/// Revert check: without the refusal the draw with instance_id 2 goes in.
#[test]
fn an_instance_drawn_on_its_own_that_reads_gl_instance_id_is_refused() {
    let rig = Rig::new();
    rig.init();
    let mut reads = program();
    reads.v3d.vs.uses_iid = 1;
    let base = v3d_refused_draws();
    for (prog, id, drawn) in [(&reads, 2u32, false), (&reads, 0, true), (&*rig.fs, 2, true)] {
        let mut d = rig.draw();
        d.fs = prog;
        d.instance_id = id;
        assert_eq!(draw(&d) == 0, drawn, "instance_id {id}, reads {}", prog.v3d.vs.uses_iid);
    }
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 1);
    assert_eq!(rig.logs_with("an instance drawn on its own whose program reads gl_InstanceID"), 1);
}

/* ---- 4x multisampling (docs/online-shaderc/v3d-backend.md "Multisampling") ---------- */

/// A draw on a 4x surface (samples 4) with depth kept: a 4x job, whose RT 0
/// samples go to a 4 KiB-aligned buffer of their own and whose Z/S buffer
/// is 2w x 2h; the draw's sample mask reaches its state. The first job on
/// the surface starts from the clear colour (its samples are undefined:
/// nothing kept them yet); the next job, with no clear, loads them; after
/// eglSwapBuffers they are undefined again. A single-sampled draw on another
/// target is a 1x job. Revert checks: with `msaa` left false the first
/// assertion fails; without the swap's invalidation the last one does.
#[test]
fn a_draw_on_a_4x_surface_is_a_4x_job_whose_samples_are_kept() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.samples = 4;
    d.sample_mask = 0b0011;
    (d.ds_persist, d.ds_id) = (true, 0x8000_1000);
    d.do_clear = false;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    v3d_surface_swapped(TARGET);
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 3);
    let t = jobs[0].target;
    assert!(t.msaa, "a 4x job");
    assert!(t.ms_address != 0 && t.ms_address.is_multiple_of(4096), "its samples' buffer: {t:?}");
    assert_eq!(t.ms_memory_format, V3D_TEX_TILING_UIF_NO_XOR, "UIF, as Mesa lays out an MSAA resource: {t:?}");
    assert_eq!(jobs[1].target, t, "the same buffer for the next job");
    assert_eq!(rig.draws()[0].st.sample_mask, 0b0011, "the draw's mask");
    let c0 = jobs[0].clear.expect("ended");
    assert!(c0.zs_address != 0 && c0.zs_height_in_ub == 16, "a 4x Z/S buffer (128 rows): {c0:?}");
    let loads: Vec<bool> = jobs.iter().map(|j| j.clear.expect("ended").load_rt0).collect();
    assert_eq!(loads, [false, true, false], "undefined, kept, undefined after the swap");
    /* a 1x draw on another target: a 1x job */
    let mut one = rig.draw();
    one.target_gpu = TARGET + 0x10_0000;
    assert_eq!(draw(&one), 0);
    assert_eq!(v3d_flush(), 0);
    let t1 = rig.jobs()[3].target;
    assert!(!t1.msaa && t1.ms_address == 0, "{t1:?}");
}

/// What a 4x draw cannot be yet is refused by name: a sample count the V3D
/// has no mode for (-EINVAL), and alpha-to-coverage (MSAA step 4), which is
/// legal GL state and so a backend gap (-ENOTSUP, src/backend.rs `draw`).
/// The alpha-to-coverage refusal is logged once, with its own reason, and
/// counted each time.
#[test]
fn a_4x_draw_the_backend_cannot_take_is_refused_by_name() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.samples = 2;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("a sample count other than 1 and 4"), 1);
    d.samples = 4;
    d.alpha_to_coverage = true;
    let base = v3d_refused_draws();
    assert_eq!(draw(&d), -ENOTSUP);
    assert_eq!(draw(&d), -ENOTSUP);
    let logs = rig.gl.logs();
    let named: Vec<&String> = logs.iter().filter(|l| l.contains("alpha-to-coverage")).collect();
    assert_eq!(
        named,
        ["yakogl/v3d: draw refused: alpha-to-coverage is not implemented"],
        "one line, the whole reason: {logs:?}"
    );
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 2, "both draws counted");
    assert!(rig.draws().is_empty(), "nothing was recorded");
}

/* ---- multiple render targets (docs/online-shaderc/v3d-backend.md) ----------------- */

/// RT 1 and RT 2 of a draw: RGBA8 attachments at RT 0's size.
const RT1: u64 = TARGET + 0x0010_0000;
const RT2: u64 = TARGET + 0x0020_0000;

/// A draw on three attachments whose program writes RT `mask`.
fn mrt_draw(rig: &Rig, p: &ShcShader, targets: u8) -> GlesDraw {
    let mut d = rig.draw();
    d.fs = p;
    d.num_targets = targets;
    d.targets[0] = GlesDrawTarget { gpu: TARGET, w: 64, h: 64, format: GLES_SURF_RGBA8888, out_shift: 0 };
    for (i, gpu) in [RT1, RT2].into_iter().enumerate().take(targets.saturating_sub(1) as usize) {
        d.targets[i + 1] = GlesDrawTarget { gpu, w: 64, h: 64, format: GLES_SURF_RGBA8888, out_shift: 0 };
    }
    d
}

fn program_writing(mask: u8) -> Box<ShcShader> {
    let mut p = Box::new(program());
    p.v3d.fs.rt_mask = mask;
    p
}

/// A draw on three RGBA8 attachments whose program writes RT 0..2: the job
/// configures three render targets, RT 1 and RT 2 at their own addresses and
/// RT 0's pitch, and a glClear clears every one of them to the same colour
/// (ES 3.0 4.2.3: every draw buffer). Revert check: without extra_targets the
/// job has rts 0 and the RT 1..3 addresses stay 0.
#[test]
fn a_draw_on_three_attachments_configures_three_render_targets() {
    let rig = Rig::new();
    rig.init();
    let p = program_writing(0b111);
    assert_eq!(draw(&mrt_draw(&rig, &p, 3)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 1);
    let t = jobs[0].target;
    assert_eq!(t.rts, 3);
    assert_eq!(
        t.rt,
        [
            V3dJobRt { address: RT1 as u32, stride: 256 },
            V3dJobRt { address: RT2 as u32, stride: 256 },
            V3dJobRt { address: 0, stride: 0 }
        ]
    );
    let c = jobs[0].clear.expect("ended");
    assert!(!c.load_rt0 && !c.load_rt[0] && !c.load_rt[1], "the glClear clears every RT: {c:?}");
    assert_eq!(c.rgba8_rt[..2], [c.rgba8, c.rgba8], "one colour for every RT");
}

/// glDrawBuffers(ATT0, NONE, ATT2), as the GL core hands it since the NONE
/// slot is kept (draw.rs resolve_targets): slot 1 with no target (gpu 0).
/// The job configures three render targets, so the program's RT 2 write has
/// its slot (gl_test 106's hole, refused on UB5 as "a colour output with no
/// attachment slot" while the GL core stopped at the hole); RT 1 has no
/// address, which the job builder neither loads nor stores (job_check part
/// 12), and RT 2 is attachment 2's memory. The fused glClear clears every
/// RT. Revert check: without the gpu 0 skip in extra_targets the draw is
/// refused as "colour attachments of different sizes" (the empty slot's
/// 0x0 against RT 0's 64x64).
#[test]
fn a_none_draw_buffer_is_configured_and_never_given_an_address() {
    let rig = Rig::new();
    rig.init();
    let p = program_writing(0b101);
    let mut d = mrt_draw(&rig, &p, 3);
    d.targets[1] = GlesDrawTarget { gpu: 0, w: 0, h: 0, format: 0, out_shift: 0 };
    d.targets[2] = GlesDrawTarget { gpu: RT2, w: 64, h: 64, format: GLES_SURF_RGBA8888, out_shift: 0 };
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 1);
    let t = jobs[0].target;
    assert_eq!(t.rts, 3, "RT 1 configured though it has no buffer");
    assert_eq!(t.rt[0].address, 0, "RT 1: a GL_NONE draw buffer, no address");
    assert_eq!(t.rt[1].address, RT2 as u32, "RT 2: attachment 2");
    let c = jobs[0].clear.expect("ended");
    assert!(!c.load_rt0 && !c.load_rt[1], "the glClear clears RT 0 and RT 2: {c:?}");
}

/// A program writing a colour output at a location the framebuffer has no
/// attachment slot for is refused by name and counted (the job's RT count
/// is the framebuffer's, fixed when the job starts; GL would drop the
/// write, which needs a program variant: docs, "not built"); with the slot
/// it is drawn. Revert check: without the rt_mask check the first draw is
/// recorded.
#[test]
fn a_colour_output_with_no_attachment_slot_is_refused() {
    let rig = Rig::new();
    rig.init();
    let base = v3d_refused_draws();
    let p = program_writing(0b101);
    assert_eq!(draw(&mrt_draw(&rig, &p, 2)), -EINVAL, "RT 2 written, two slots");
    assert_eq!(rig.logs_with("a colour output with no attachment slot"), 1);
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 1, "counted");
    assert!(rig.draws().is_empty(), "nothing was recorded");
    let mut one = rig.draw();
    one.fs = &*p;
    assert_eq!(draw(&one), -EINVAL, "RT 2 written, the default framebuffer's one slot");
    assert_eq!(draw(&mrt_draw(&rig, &p, 3)), 0, "three slots: drawn");
    assert_eq!(rig.draws().len(), 1);
}

/// A program that reads a texture's size (the compiler's
/// QUNIFORM_TEXTURE_WIDTH / _HEIGHT words, 44 and 45: textureSize, and the
/// face size of the ES 2.0 non-seamless cube look-up's level of detail,
/// prepare.rs `cube_lod`) is drawn, and the job builder's table carries the
/// binding's base-level size for v3d_job.c to write; a unit only a size
/// word names gets the 1x1 stand-in, as an unbound sampled unit does.
/// Revert check: with the size left 0 the job builder would refuse the
/// word (tools/v3d/job_check.c part 5), and before this the draw was
/// refused by name.
#[test]
fn a_program_reading_a_texture_size_gets_the_bindings_size() {
    static FS_SIZE: [ShcV3dUniform; 5] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0 },
        ShcV3dUniform { contents: 44, data: 0 },
        ShcV3dUniform { contents: 45, data: 0 },
        ShcV3dUniform { contents: 44, data: 3 },
    ];
    let rig = Rig::new();
    rig.init();
    let base = v3d_refused_draws();
    let mut p = program();
    (p.v3d.fs.uniforms, p.v3d.fs.num_uniforms) = (FS_SIZE.as_ptr(), 5);
    with_code(&mut p);
    let mut d = sampling(&rig, texture(0, 12, 8, 1));
    d.fs = &p;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 0, "not refused");
    let draws = rig.draws();
    assert_eq!(draws.len(), 1);
    let t = &draws[0].tmu;
    assert_eq!(t.len(), 2, "binding 0 and the stand-in for unit 3: {t:?}");
    assert_eq!((t[0].unit, t[0].width, t[0].height), (0, 12, 8));
    assert_eq!((t[1].unit, t[1].width, t[1].height), (3, 1, 1));
}

/// What the job builder cannot take is refused by name before anything is
/// recorded: an attachment of another size, one of a format with no TLB
/// row (a surface number none names), and a colour clear the quad would do
/// (scissored), which writes RT 0 only.
#[test]
fn attachments_the_job_builder_cannot_take_are_refused_by_name() {
    let rig = Rig::new();
    rig.init();
    let p = program_writing(0b11);
    let mut d = mrt_draw(&rig, &p, 2);
    d.targets[1].w = 32;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("colour attachments of different sizes"), 1);
    let mut d = mrt_draw(&rig, &p, 2);
    d.targets[1].format = 99;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("a colour attachment past the first of a format the backend does not render"), 1);
    let mut d = mrt_draw(&rig, &p, 2);
    d.clear_quad = true;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("a scissored or masked colour clear of several colour attachments"), 1);
    assert!(rig.draws().is_empty() && rig.jobs().is_empty(), "nothing was recorded");
}

/// Another set of attachments is another job, as another RT 0 is: the RCL
/// stores each job's own buffers. Revert check: with rts and rt left out of
/// the Target comparison, the second draw joins the first job.
#[test]
fn another_set_of_attachments_is_another_job() {
    let rig = Rig::new();
    rig.init();
    let p = program_writing(0b11);
    assert_eq!(draw(&mrt_draw(&rig, &p, 2)), 0);
    let mut d = mrt_draw(&rig, &p, 2);
    d.targets[1].gpu = RT2;
    d.do_clear = false;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 2, "one job per attachment set");
    assert_eq!((jobs[0].target.rt[0].address, jobs[1].target.rt[0].address), (RT1 as u32, RT2 as u32));
}

/// The platform's setting for depth and stencil apart
/// (OSAL_QUERY_V3D_SPLIT_ZS, CONFIG_YAKOGL_V3D_SPLIT_ZS on Zephyr) is read at
/// init: 1 turns it on (and says so), 0 and no answer leave it off.
/// Revert check: init not reading the query leaves the first arm off.
#[test]
fn the_platform_says_whether_depth_and_stencil_are_kept_apart() {
    for (answer, on) in [(Some(1), true), (Some(0), false), (None, false)] {
        let rig = Rig::new();
        with_platform(|p| p.split_zs = answer);
        rig.init();
        assert_eq!(g().split_zs, on, "{answer:?}");
        assert_eq!(rig.logs_with("depth and stencil apart (by renderbuffer), the platform's setting"), on as usize);
    }
}

/// The platform's setting for S8 stencil halves (OSAL_QUERY_V3D_S8_STENCIL,
/// CONFIG_YAKOGL_V3D_S8_STENCIL on Zephyr) is read at init, and takes
/// effect only with depth and stencil apart: asked for without them it is
/// off and a line says why; 0 and no answer leave it off.
/// Revert check: init not reading the query leaves the first arm off;
/// ignoring the split leaves the second on.
#[test]
fn the_platform_says_whether_stencil_halves_are_s8() {
    let arms = [
        (Some(1), Some(1), true, 0),
        (Some(0), Some(1), false, 1),
        (Some(1), Some(0), false, 0),
        (Some(1), None, false, 0),
    ];
    for (split, s8, on, why) in arms {
        let rig = Rig::new();
        with_platform(|p| (p.split_zs, p.s8_stencil) = (split, s8));
        rig.init();
        assert_eq!(g().s8_stencil, on, "{split:?} {s8:?}");
        let logs = rig.gl.logs(); /* read once: logs() drains the recorder */
        let count = |what: &str| logs.iter().filter(|l| l.contains(what)).count();
        assert_eq!(count("stencil halves in S8 buffers (1 byte a pixel)"), on as usize, "{logs:?}");
        assert_eq!(count("S8 stencil asked for without depth and stencil apart"), why, "{logs:?}");
    }
}

/// A program reading gl_PointCoord (the FS's implicit point varyings, v3d-m5's
/// uses_implicit_point_line_varyings) is drawn as points only: any other
/// primitive is refused by name, one line per primitive, and counted, with
/// nothing recorded; points go in. Revert check: without the backend's check
/// the triangle draw is recorded (the rig's builder takes it).
#[test]
fn a_program_reading_gl_point_coord_is_drawn_as_points_only() {
    let rig = Rig::new();
    rig.init();
    let mut p = Box::new(program());
    p.v3d.fs.uses_implicit_point_line_varyings = 1;
    let base = v3d_refused_draws();
    let mut d = rig.draw();
    d.fs = &*p;
    for (prim, name) in [(GLES_PRIM_TRIANGLES, "GL_TRIANGLES"), (GLES_PRIM_LINES, "GL_LINES")] {
        d.prim = prim;
        assert_eq!(draw(&d), -EINVAL, "{name}");
        assert_eq!(rig.logs_with(&format!("a program reading gl_PointCoord drawn as {name}")), 1, "{name}");
    }
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 2, "both are counted");
    assert!(rig.draws().is_empty(), "nothing was recorded");
    d.prim = GLES_PRIM_POINTS;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(rig.draws().len(), 1, "points go in");
}

/// `v3d_refused_draws` counts every refused draw, not only the first of a
/// cause the log shows (v3d_deqp marks a case R when it grew across it),
/// and every draw that no empty job holds (-E2BIG: lost by this backend's
/// limits, so a harness scores it as not drawn rather than as a wrong
/// picture; v3d-deqp's note, 2026-09-25). A full job whose draw the next job
/// takes is not a refusal. Revert check: without the count on the -E2BIG
/// path the two lost draws read as 3, not 5.
#[test]
fn every_refused_draw_is_counted_and_nothing_else() {
    let rig = Rig::new();
    rig.init();
    let base = v3d_refused_draws();
    let mut p = rig.draw();
    p.samples = 2;
    assert_eq!((draw(&p), draw(&p)), (-EINVAL, -EINVAL));
    assert_eq!(rig.logs_with("a sample count other than 1 and 4"), 1, "one line for two");
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 2, "both are counted");
    let mut d = rig.draw();
    rig.answer(&[V3D_JOB_REFUSED]);
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 3, "the job builder's refusal");
    /* not refusals */
    assert_eq!(draw(&d), 0);
    d.do_clear = false;
    rig.answer(&[V3D_JOB_FULL]);
    assert_eq!(draw(&d), 0, "a full job: the next one takes it");
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 3, "a full job is not a refusal");
    /* lost draws: counted */
    rig.answer(&[V3D_JOB_FULL, V3D_JOB_FULL]);
    assert_eq!(draw(&d), -E2BIG);
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 4, "a draw no empty job holds is lost: counted");
    let big = vec![0u8; DATA_BYTES + 16];
    let mut s = rig.draw();
    s.attribs[0].cpu = big.as_ptr() as *const c_void;
    s.attribs[0].cpu_bytes = big.len() as u32;
    /* larger than the arena and no memory of its own for it */
    with_platform(|p| p.fail_alloc_from = Some(DATA_BYTES));
    assert_eq!(draw(&s), -E2BIG);
    with_platform(|p| p.fail_alloc_from = None);
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 5, "so is one whose data has no memory of its own");
    assert_eq!(rig.logs_with("draw refused: no empty job holds the draw"), 1, "one line for the cause");
}

#[test]
fn a_program_that_is_not_linked_for_the_v3d_is_refused() {
    let rig = Rig::new();
    rig.init();
    let stage = ShcShader::ZERO;
    let mut d = rig.draw();
    d.fs = &stage;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("not a program linked for the V3D"), 1);
    let mut short = program();
    short.usc_dw = 40; /* the CS (136..200) lies past the 160 bytes uploaded */
    d.fs = &short;
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("outside the uploaded code"), 1);
    d.fs = core::ptr::null();
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("no linked program"), 1);
}

#[test]
fn addresses_the_v3d_cannot_hold_are_refused() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.target_gpu = 1 << 32;
    assert_eq!(draw(&d), -EINVAL);
    let mut d = rig.draw();
    d.attribs[0].buffer_gpu = 1 << 32;
    assert_eq!(draw(&d), -EINVAL);
    let mut d = rig.draw();
    d.target_w = MAX_TARGET + 1;
    assert_eq!(draw(&d), -EINVAL);
    assert!(rig.draws().is_empty());
}

#[test]
fn the_table_names_the_gpu_and_its_formats() {
    let t = &yakogl_backend_v3d;
    // SAFETY: a static C string.
    let name = unsafe { core::ffi::CStr::from_ptr(t.name) }.to_str().unwrap();
    assert!(name.contains("V3D"));
    assert_eq!(t.native_attrib_formats, gles_attribf_bit(GLES_ATTRIBF_FLOAT32));
    assert!(!t.rt_rgba8888_is_bgra && !t.front_facing_shader_variant);
    assert_eq!(v3d_heap_base(GLES_HEAP_SHADER), V3D_HEAP_SHADER);
    assert_eq!(v3d_heap_base(GLES_HEAP_DATA), V3D_HEAP_DATA);
}

/* ---- dEQP attribute_location, reproduced through the GL core ------------- */

/// dEQP's `AttributeLocationTestUtil` (glsAttributeLocationTests.cpp): the
/// attribute a case declares, its condition and its bound location.
struct DqAttr {
    ty: &'static str,
    name: String,
    /* None: COND_ALWAYS; Some((name, negate)); "__never__": COND_NEVER */
    cond: Option<(&'static str, bool)>,
    never: bool,
}

fn dq_to_vec4(a: &DqAttr) -> String {
    match a.ty {
        "vec2" => format!("vec4({0}.xy, {0}.yx)", a.name),
        "vec3" => format!("vec4({0}.xyz, {0}.x)", a.name),
        _ => format!("vec4({})", a.name),
    }
}

/// generateVertexShaderTemplate (:225-247), `#version 100`.
fn dq_vertex_source(attrs: &[DqAttr]) -> String {
    let mut s = String::from("#version 100\nvarying mediump vec4 v_color;\n");
    for a in attrs {
        s += &format!("attribute mediump {} {};\n", a.ty, a.name);
    }
    let mut conds: Vec<&str> = attrs.iter().filter_map(|a| a.cond.map(|c| c.0)).collect();
    conds.sort();
    conds.dedup();
    s += "\n";
    for c in conds {
        s += &format!("uniform mediump float u_{c};\n");
    }
    s += "\nvoid main (void)\n{\n\tmediump vec4 color = vec4(0.0);\n\n";
    for a in attrs {
        let add = format!("color += {};\n", dq_to_vec4(a));
        if a.never {
            s += &format!("\tif (0 != 0)\n\t{{\n\t\t{add}\t}}\n");
        } else if let Some((c, negate)) = a.cond {
            s += &format!("\tif (u_{c}{}0.0)\n\t{{\n\t\t{add}\t}}\n", if negate { " != " } else { " == " });
        } else {
            s += &format!("\t{add}");
        }
    }
    s += "\n\tv_color = color;\n\tgl_Position = color;\n}\n";
    s
}

const DQ_FRAGMENT: &str =
    "#version 100\n\nvarying mediump vec4 v_color;\nvoid main (void)\n{\n\tgl_FragColor = v_color;\n}\n";

const DQ_TYPES: [(&str, i32); 7] =
    [("float", 1), ("vec2", 1), ("vec3", 1), ("vec4", 1), ("mat2", 2), ("mat3", 3), ("mat4", 4)];

fn dq_attr(ty: &'static str, name: String) -> DqAttr {
    DqAttr { ty, name, cond: None, never: false }
}

/// The attributes, the pre-link bindings and the post-link bindings (with a
/// relink) of the four failing shapes, as their iterate() builds them.
#[allow(clippy::type_complexity)]
fn dq_case(
    shape: &str,
    ty: &'static str,
    locs: i32,
    max: i32,
) -> (Vec<DqAttr>, Vec<(String, i32)>, Vec<(String, i32)>) {
    let (mut attrs, mut binds, mut post) = (Vec::new(), Vec::new(), Vec::new());
    match shape {
        "bind_max_attributes" => {
            let mut loc = max - locs;
            let mut ndx = 0;
            while loc >= 0 {
                attrs.push(dq_attr(ty, format!("a_{ndx}")));
                binds.push((format!("a_{ndx}"), loc));
                loc -= locs;
                ndx += 1;
            }
        }
        "max_cond" | "max_inactive" => {
            let mut loc = max - locs;
            let mut ndx = 0;
            while loc >= 0 {
                let mut a = dq_attr(ty, format!("a_{ndx}"));
                let mut b = dq_attr(ty, format!("a_{}", ndx + max));
                if shape == "max_cond" {
                    a.cond = Some(("A", true));
                    b.cond = Some(("A", false));
                } else {
                    a.cond = Some(("A", true)); /* Cond("A"): negate defaults to true */
                    b.never = true;
                }
                attrs.push(a);
                attrs.push(b);
                binds.push((format!("a_{ndx}"), loc));
                binds.push((format!("a_{}", ndx + max), loc));
                loc -= locs;
                ndx += 1;
            }
        }
        "bind_hole" | "bind_relink_hole" => {
            attrs.push(dq_attr("vec4", "a_0".into()));
            binds.push(("a_0".into(), 0));
            attrs.push(dq_attr(ty, "a_1".into()));
            for (ndx, loc) in (2..).zip(1 + locs..max) {
                attrs.push(dq_attr("vec4", format!("a_{ndx}")));
                binds.push((format!("a_{ndx}"), loc));
            }
            if shape == "bind_relink_hole" {
                post.push(("a_2".into(), 1));
            }
        }
        _ => unreachable!(),
    }
    (attrs, binds, post)
}

fn dq_type_enum(ty: &str) -> u32 {
    use crate::gl::consts::*;
    match ty {
        "float" => GL_FLOAT,
        "vec2" => GL_FLOAT_VEC2,
        "vec3" => GL_FLOAT_VEC3,
        "vec4" => GL_FLOAT_VEC4,
        "mat2" => GL_FLOAT_MAT2,
        "mat3" => GL_FLOAT_MAT3,
        _ => GL_FLOAT_MAT4,
    }
}

/// runTest (:683-795) for one case: None when it passes, else why.
fn dq_run(attrs: &[DqAttr], binds: &[(String, i32)], post: &[(String, i32)]) -> Option<String> {
    use crate::gl::consts::*;
    use crate::gl::program::*;
    use crate::gl::shader::*;
    use std::ffi::CString;
    // SAFETY: plain GL calls on the rig's context with live buffers.
    unsafe {
        let p = glCreateProgram();
        let mut shaders = Vec::new();
        for (ty, src) in [(GL_VERTEX_SHADER, dq_vertex_source(attrs)), (GL_FRAGMENT_SHADER, DQ_FRAGMENT.into())] {
            let s = glCreateShader(ty);
            let c = CString::new(src).unwrap();
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            if ok != 1 {
                let mut log = [0u8; 512];
                glGetShaderInfoLog(s, 512, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
                return Some(format!("compile 0x{ty:x}: {}", String::from_utf8_lossy(&log).trim_end_matches('\0')));
            }
            glAttachShader(p, s);
            shaders.push(s);
        }
        let bind = |bs: &[(String, i32)]| {
            for (n, l) in bs {
                let c = CString::new(n.as_str()).unwrap();
                glBindAttribLocation(p, *l as u32, c.as_ptr());
            }
        };
        let link = |what: &str| -> Option<String> {
            glLinkProgram(p);
            let mut ok = 0;
            glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
            if ok == 1 {
                return None;
            }
            let mut log = [0u8; 512];
            glGetProgramInfoLog(p, 512, core::ptr::null_mut(), log.as_mut_ptr() as *mut c_char);
            Some(format!("{what} failed: {}", String::from_utf8_lossy(&log).trim_end_matches('\0')))
        };
        let mut active: BTreeMap<String, i32> = binds.iter().cloned().collect();
        let check = |active: &BTreeMap<String, i32>| -> Option<String> {
            let mut n = 0;
            glGetProgramiv(p, GL_ACTIVE_ATTRIBUTES, &mut n);
            let mut seen = Vec::new();
            for i in 0..n {
                let mut name = [0u8; 128];
                let (mut len, mut size, mut ty) = (0, 0, 0u32);
                glGetActiveAttrib(p, i as u32, 127, &mut len, &mut size, &mut ty, name.as_mut_ptr() as *mut c_char);
                let name = String::from_utf8_lossy(&name[..len.max(0) as usize]).into_owned();
                match attrs.iter().find(|a| a.name == name) {
                    None => return Some(format!("unknown active attribute '{name}'")),
                    Some(a) if ty != dq_type_enum(a.ty) || size != 1 => {
                        return Some(format!("{name}: type 0x{ty:x} size {size}"))
                    }
                    _ => {}
                }
                seen.push(name);
            }
            for a in attrs.iter().filter(|a| !a.never) {
                if !seen.contains(&a.name) {
                    return Some(format!("active {} not returned by glGetActiveAttrib ({n} returned)", a.name));
                }
                let c = CString::new(a.name.as_str()).unwrap();
                let got = glGetAttribLocation(p, c.as_ptr());
                if let Some(&want) = active.get(&a.name) {
                    if got != want {
                        return Some(format!("{}: location {got}, bound {want}", a.name));
                    }
                }
            }
            None
        };
        bind(binds);
        let r = link("link").or_else(|| check(&active)).or_else(|| {
            if post.is_empty() {
                return None;
            }
            bind(post);
            check(&active).or_else(|| {
                let r = link("relink");
                for (n, l) in post {
                    active.insert(n.clone(), *l);
                }
                r.or_else(|| check(&active))
            })
        });
        glDeleteProgram(p);
        for s in shaders {
            glDeleteShader(s);
        }
        r
    }
}

/// dEQP-GLES2.functional.attribute_location on the V3D, pinned. The Pi 4
/// failed 32 (dEQP batch C, 823d20fb: 25 of 57); with the allocator's 2-thread
/// fallback (v3d-m5, 170b88d9) 9 of them link, and with the lazy attribute
/// reads of a vertex shader that does not fit 2 threads (v3d-m5, "attributes
/// read where first needed") the 9 that needed more registers link too. The
/// last 14, the bind_aliasing max_cond / max_inactive cases (43/57 on the
/// Pi 4 at c9eb1078, "Abi(AttribLocation { location: 16 })" at compile: the
/// 17th declared attribute took location 16 before any binding was known),
/// link since the V3D takes the link-time attribute layout (layout_at_link,
/// compile_vs_linked; SHADERC_NOTES.md "Attribute registers decided at
/// link"). None of these cases draws: they compile, link and query (runTest,
/// :683-795). docs/online-shaderc/v3d-backend.md, "M6 attribute_location".
///
/// Written to be re-derived: the day a case moves, this fails and names it,
/// and the expectation below is changed with the evidence.
#[test]
fn deqp_attribute_location_cases_all_link_and_answer() {
    let _rig = Rig::new();
    let mut max = 0;
    // SAFETY: a live out-parameter.
    unsafe { crate::gl::state::glGetIntegerv(crate::gl::consts::GL_MAX_VERTEX_ATTRIBS, &mut max) };
    assert_eq!(max, 16, "the cases below are dEQP's for 16 vertex attributes");
    let (mut run, mut failing) = (0, Vec::new());
    for shape in ["bind_max_attributes", "max_cond", "max_inactive", "bind_hole", "bind_relink_hole"] {
        for (ty, locs) in DQ_TYPES {
            let (a, b, p) = dq_case(shape, ty, locs, max);
            let got = dq_run(&a, &b, &p);
            run += 1;
            let name =
                if shape.starts_with("max_") { format!("bind_aliasing.{shape}_{ty}") } else { format!("{shape}.{ty}") };
            if let Some(why) = got {
                failing.push(format!("{name}: {why}"));
            }
        }
    }
    /* the population: every case built, and none fails */
    assert_eq!(run, 35);
    assert!(failing.is_empty(), "each links and answers its queries: {failing:#?}");
}

/// The compiler and the job builder name gl_FragCoord.y's two uniforms with
/// the same numbers: select.rs QUNIFORM_FB_WPOS_Y_* (what the compiler emits)
/// and ffi.rs V3D_QUNIFORM_FB_WPOS_Y_* (what v3d_job.c / v3d_frame.c write,
/// pinned to the C by ffi_layout_check.c). A stream the compiler emits with
/// one number and the writer reads with another is refused as an unknown
/// contents, never drawn: this makes the mismatch a test failure instead.
#[test]
fn the_compiler_and_the_job_builder_agree_on_the_wpos_codes() {
    use crate::shaderc::online::v3d::select::{QUNIFORM_FB_WPOS_Y_OFFSET, QUNIFORM_FB_WPOS_Y_SCALE};
    assert_eq!(QUNIFORM_FB_WPOS_Y_SCALE, V3D_QUNIFORM_FB_WPOS_Y_SCALE);
    assert_eq!(QUNIFORM_FB_WPOS_Y_OFFSET, V3D_QUNIFORM_FB_WPOS_Y_OFFSET);
    assert_eq!((V3D_QUNIFORM_FB_WPOS_Y_SCALE, V3D_QUNIFORM_FB_WPOS_Y_OFFSET), (0x10000, 0x10001));
    /* gl_PointCoord.y's pair, agreed with v3d-m5 (2026-09-24); the
     * compiler's twins join this test when select.rs carries them */
    assert_eq!((V3D_QUNIFORM_FB_PNTC_Y_SCALE, V3D_QUNIFORM_FB_PNTC_Y_OFFSET), (0x10002, 0x10003));
}

/* ---- the tiled copies kept across jobs (M6 tiled-copy cache) ------------- */

fn tilings() -> u32 {
    let mut t = 0u32;
    // SAFETY: a live out-parameter.
    unsafe { v3d_texture_counters(&mut t, core::ptr::null_mut()) };
    t
}

/// (live copies, their bytes, the high-water mark)
fn tiled() -> (u32, u64, u64) {
    let (mut l, mut b, mut h) = (0u32, 0u64, 0u64);
    // SAFETY: live out-parameters.
    unsafe { v3d_tiled_copies(&mut l, &mut b, &mut h) };
    (l, b, h)
}

fn sampling(rig: &Rig, t: GlesTextureBind) -> GlesDraw {
    let mut d = rig.draw();
    d.do_clear = false;
    d.textures[0] = t;
    d.num_textures = 1;
    d
}

/// A stamped texture (content_gen != 0) is tiled once and sampled from that
/// copy in the next job; a new stamp of the same storage (a CPU write) tiles
/// it again and drops the old copy.
#[test]
fn a_stamped_texture_is_tiled_once_across_jobs_and_again_after_a_new_stamp() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 7;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!((tilings(), tiled().0), (1, 1), "tiled once, kept");
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tilings(), 1, "the second job samples the kept copy");
    let st = |i: usize| words(&rig, rig.draws()[i].tmu[0].tex_state, 1)[0];
    assert_eq!(st(0), st(1), "both jobs' texture states name one image");
    t.content_gen = 8;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!((tilings(), tiled().0), (2, 1), "a new stamp tiles again; the old copy is gone");
}

/// No stamp (an EGLImage-backed texture, content_gen 0): never kept, tiled
/// into each job's arena as before.
#[test]
fn an_unstamped_texture_is_tiled_in_every_job_and_never_kept() {
    let rig = Rig::new();
    rig.init();
    let t = texture(0, 16, 16, 1);
    assert_eq!(t.content_gen, 0);
    for _ in 0..2 {
        assert_eq!(draw(&sampling(&rig, t)), 0);
        assert_eq!(v3d_flush(), 0);
    }
    assert_eq!((tilings(), tiled().0), (2, 0));
}

/// The GL core's stamp only moves on the CPU's writes: a job that renders into
/// the texture's storage drops the copy itself, and the next job that
/// samples it tiles it again from what the render wrote.
#[test]
fn a_job_rendering_into_a_texture_drops_its_tiled_copy() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 3;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tiled().0, 1);
    let mut r = rig.draw();
    (r.target_gpu, r.target_w, r.target_h, r.viewport) = (t.base_gpu, 16, 16, [0, 0, 16, 16]);
    assert_eq!(draw(&r), 0);
    assert_eq!(tiled().0, 0, "the render into it dropped the copy");
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!((tilings(), tiled().0), (2, 1), "tiled again, after the render's job");
}

/// texture_released drops the copies made from that storage and frees them.
#[test]
fn a_released_texture_frees_its_tiled_copy() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 5;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    let (live, bytes, _) = tiled();
    assert_eq!(live, 1);
    assert!(bytes > 0);
    let before = with_platform(|p| p.frees.len());
    v3d_texture_released(t.base_gpu, texture::gl_bytes(&t) as u64);
    assert_eq!(tiled().0, 0);
    assert_eq!(tiled().1, 0);
    assert_eq!(with_platform(|p| p.frees.len()), before + 1, "the copy is freed");
    /* storage elsewhere drops nothing */
    t.content_gen = 6;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    v3d_texture_released(t.base_gpu + (1 << 20), 4096);
    assert_eq!(tiled().0, 1);
}

/* ---- render images (RtImage) ---------------------------------------------------- */

/// A draw into texture `t`'s storage: an FBO whose colour attachment is `t`.
fn render_into(rig: &Rig, t: &GlesTextureBind) -> GlesDraw {
    let mut r = rig.draw();
    (r.target_gpu, r.target_w, r.target_h) = (t.base_gpu, t.width, t.height);
    r.viewport = [0, 0, t.width as i32, t.height as i32];
    r
}

/// `v3d_rt_images`: (images live, their bytes, bindings sampled from an
/// image, samples sent back to a copy by a CPU write, images not made for
/// want of memory, chains the TFU made, chains written back).
fn rt_images() -> (u32, u64, u32, u32, u32, u32, u32) {
    let (mut l, mut b, mut d, mut c, mut n, mut m, mut r) = (0u32, 0u64, 0u32, 0u32, 0u32, 0u32, 0u32);
    // SAFETY: live out-parameters.
    unsafe { v3d_rt_images(&mut l, &mut b, &mut d, &mut c, &mut n, &mut m, &mut r) };
    (l, b, d, c, n, m, r)
}

/// The render image the last job's target names: (level 0's address,
/// memory format, height in UIF blocks); an address of 0 is none.
fn last_uif(rig: &Rig) -> (u32, u32, u32) {
    let t = rig.jobs().last().expect("a job").target;
    (t.uif_address, t.uif_memory_format, t.uif_height_in_ub)
}

/// The render image the job before the last (a `frame`'s render, the last
/// being its sample) stored: level 0's address, 0 for none.
fn render_uif(rig: &Rig) -> u32 {
    let jobs = rig.jobs();
    jobs[jobs.len() - 2].target.uif_address
}

/// The last draw's unit 0 texture state: (its base address, its max level)
/// (the fake packer's words 0 and 3).
fn last_tex_state(rig: &Rig) -> (u32, u32) {
    let w = words(rig, rig.draws().last().expect("a draw").tmu[0].tex_state, 4);
    (w[0], w[3] & 0xffff)
}

fn last_tex_base(rig: &Rig) -> u32 {
    last_tex_state(rig).0
}

/// Render into `t`, submit, and sample it with the render's kick: one
/// frame of a render-to-texture scene. Returns what the sampling job's
/// draw read as its texture base.
fn frame(rig: &Rig, t: &mut GlesTextureBind) -> u32 {
    assert_eq!(draw(&render_into(rig, t)), 0, "{:?}", rig.gl.logs());
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&sampling(rig, *t)), 0, "{:?}", rig.gl.logs());
    let base = last_tex_base(rig);
    assert_eq!(v3d_flush(), 0);
    base
}

/// glGenerateMipmap as the GL core delegates it: through the V3D table's
/// entry, with the chain's levels.
fn generate(t: &GlesTextureBind, levels: u32) -> i32 {
    let f = yakogl_backend_v3d.generate_mipmap.expect("the V3D table has generate_mipmap");
    // SAFETY: `t` is live.
    unsafe { f(t, levels) }
}

/// The GL core asking for the chain back, through the V3D table's entry.
fn chain_back(t: &GlesTextureBind, levels: u32) {
    let f = yakogl_backend_v3d.return_mipmap_chain.expect("the V3D table has return_mipmap_chain");
    // SAFETY: `t` is live.
    unsafe { f(t, levels) }
}

/// RESULTS option (a): a texture rendered into and sampled every frame is
/// sampled from its render image, which the rendering job stores in the
/// TMU's layout, from the second render on: no tiled copy is made or kept
/// for it, and no tiling runs. The first render stores no image (nothing
/// has sampled the target yet), the first sample is a copy and registers
/// the target, the second render stores RT 0 into the image (UIF, no XOR,
/// the padded height in UIF blocks) as well as into the storage, and the
/// second sample's texture state points at the image.
#[test]
fn a_render_target_is_sampled_from_its_render_image_from_the_second_render_on() {
    let rig = Rig::new();
    rig.init();
    /* 64x64: UIF in Mesa's layout (a target under 32 texels wide is UB-linear
     * or LT, which the TLB stores with a height of 0) */
    let mut t = texture(0, 64, 64, 1);
    t.content_gen = 3;
    /* render 1: nothing has sampled the target, so no image */
    assert_eq!(draw(&render_into(&rig, &t)), 0);
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    assert_eq!(last_uif(&rig), (0, 0, 0), "a target nothing has sampled gets no image");
    /* sample 1: the copy, and the registration */
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!((tilings(), tiled().0), (1, 1), "the first sample after a render is a copy");
    assert_eq!(rt_images(), (0, 0, 0, 0, 0, 0, 0), "registered, no image yet");
    /* render 2: the job stores RT 0 into the image too */
    assert_eq!(draw(&render_into(&rig, &t)), 0);
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    let (addr, fmt, hub) = last_uif(&rig);
    assert_ne!(addr, 0, "the second render stores the image");
    assert_eq!(addr % 4096, 0, "level 0 of a one-level layout at a page, as a Mesa BO");
    assert_eq!(fmt, V3D_TEX_TILING_UIF_NO_XOR);
    assert_eq!(hub, 8, "64 padded rows are 8 UIF blocks of 8 (the fake layout pads nothing)");
    assert_eq!(rt_images().0, 1);
    assert_eq!(tiled().0, 0, "the render dropped the copy");
    /* sample 2: the image, no copy */
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    assert_eq!(tilings(), n, "nothing tiled");
    assert_eq!(tiled().0, 0, "no copy made");
    assert_eq!(last_tex_state(&rig), (addr, 0), "the texture state's base is the render image, level 0 alone");
    assert_eq!(rt_images().2, 1);
    assert_eq!(rig.logs_with("sampled from its render image"), 1);
    assert_eq!(v3d_flush(), 0);
    /* every later frame the same: no tiling, no copy */
    for _ in 0..3 {
        assert_eq!(frame(&rig, &mut t), addr);
    }
    assert_eq!((tilings(), tiled().0), (n, 0));
    assert_eq!(rt_images().2, 4);
}

/// A CPU write of the texels (a new stamp) after the render: the image no
/// longer holds what the storage holds, so the sample goes back to a copy
/// (tiled from the storage), and so does every sample until a job renders
/// into the target again and stores the image under the new stamp.
#[test]
fn a_cpu_write_after_the_render_sends_the_sample_back_to_the_copy_until_the_next_render() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 3;
    frame(&rig, &mut t);
    let addr = frame(&rig, &mut t);
    assert_ne!(addr, 0);
    assert_eq!(rt_images().2, 1, "the second frame read the image");
    let n = tilings();
    /* the CPU writes a sub-image: a new stamp, the same storage */
    t.content_gen = 4;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_ne!(last_tex_base(&rig), addr, "a copy, not the image");
    assert_eq!((tilings(), tiled().0), (n + 1, 1), "tiled from the storage, kept");
    assert_eq!(rt_images().3, 1, "counted as a CPU write");
    assert_eq!(v3d_flush(), 0);
    /* still the copy without a render; the kept copy is found, not tiled again */
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_ne!(last_tex_base(&rig), addr);
    assert_eq!(tilings(), n + 1);
    assert_eq!(v3d_flush(), 0);
    /* the next render stores the image under the new stamp: the image again */
    assert_eq!(frame(&rig, &mut t), addr);
    assert_eq!((tilings(), tiled().0), (n + 1, 0));
    assert_eq!(rt_images().2, 2);
}

/// The 2x2 box filter of `w` x `h` RGBA8 rows `src` (rows packed), rounding
/// half up as the GL core's glGenerateMipmap and the mock TFU do.
fn box_filtered(src: &[u8], w: usize, h: usize) -> Vec<u8> {
    let (dw, dh) = ((w / 2).max(1), (h / 2).max(1));
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        for x in 0..dw {
            for c in 0..4 {
                let at = |j: usize, i: usize| src[((y * 2 + j) * w + x * 2 + i) * 4 + c] as u32;
                out[(y * dw + x) * 4 + c] = ((at(0, 0) + at(0, 1) + at(1, 0) + at(1, 1) + 2) / 4) as u8;
            }
        }
    }
    out
}

/// A render target sampled with a mipmap filter (glmark2's refract and
/// terrain) keeps the copy until its chain is the image's: the image is laid
/// out with the chain, a render stores its level 0, and glGenerateMipmap,
/// delegated to the backend, submits the open render, waits for it and has
/// the TFU make levels 1.. from level 0 in one job (DIMTW, the levels after
/// the first in ICFG, the texture's real type). The next mip-filtered sample
/// then reads the image, level 0 as its base and the chain as its levels;
/// nothing is tiled. When the GL core asks for the chain back, the storage's
/// levels 1.. hold the filtered texels and the sample is a copy again.
#[test]
fn a_mip_filtered_render_target_reads_the_image_once_the_tfu_made_its_chain() {
    let rig = tfu_rig(Some(1), 0);
    let mut t = texture(0, 16, 16, 3);
    t.content_gen = 3;
    assert_eq!(texture::levels_sampled(&t), 3);
    /* frame 1: no image; the copy of the CPU's 3-level chain; registered
     * with the 3 levels */
    frame(&rig, &mut t);
    assert_eq!(rt_images().0, 0);
    /* frame 2: the image, 3 levels, level 0 stored; the sample is still a
     * copy: the image's levels below 0 are nobody's */
    assert_eq!(draw(&render_into(&rig, &t)), 0);
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    let (addr, fmt, hub) = last_uif(&rig);
    assert_ne!(addr, 0);
    assert_eq!((fmt, hub), (3, 0), "16 wide: UB-linear 2 columns, no height word");
    let image_bytes = rt_images().1;
    assert!(image_bytes >= (16 * 16 + 8 * 8 + 4 * 4) * 4, "the image holds the chain");
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_ne!(last_tex_base(&rig), addr, "the chain is the CPU's: a copy");
    assert_eq!(tilings(), n + 1);
    assert_eq!(v3d_flush(), 0);
    /* frame 3: the render is left open, as glmark2's is at glGenerateMipmap;
     * level 0 of the image holds a known picture (what the TLB stored) */
    assert_eq!(draw(&render_into(&rig, &t)), 0);
    t.target_kick = v3d_kick_id();
    let level0: Vec<u8> = (0..16 * 16 * 4).map(|i| (i * 7 % 251) as u8).collect();
    with_platform(|p| {
        let cpu = platform_cpu(p, u64::from(addr), level0.len());
        // SAFETY: inside the image's allocation (platform_cpu checked).
        unsafe { core::ptr::copy_nonoverlapping(level0.as_ptr(), cpu, level0.len()) };
    });
    let runs = with_platform(|p| p.tfu_runs.len());
    assert_eq!(generate(&t, 3), 1, "{:?}", rig.gl.logs());
    assert!(!g().j.open, "the open render was submitted first");
    let run = with_platform(|p| p.tfu_runs.clone());
    assert_eq!(run.len(), runs + 1, "one TFU job");
    let j = run[runs];
    assert_eq!(j.ioa & V3D_TFU_IOA_DIMTW, V3D_TFU_IOA_DIMTW, "level 0 is not written");
    assert_eq!((j.ioa & !0x3f, j.iia), (addr, addr), "from level 0 of the image into the image");
    assert_eq!((j.icfg >> 5) & 0xf, 2, "two levels after the first");
    assert_eq!(j.icfg >> 9 & 0x7f, V3D_TFU_TTYPE_RGBA8, "the texture's type, not a byte copy");
    assert_eq!(j.ios, 16 << 16 | 16);
    assert_eq!(rt_images().5, 1);
    assert_eq!(rig.logs_with("glGenerateMipmap filters nothing"), 1);
    /* the fake layout: level 1 (8x8) right below level 0, level 2 below it */
    let level1 = box_filtered(&level0, 16, 16);
    let level2 = box_filtered(&level1, 8, 8);
    assert_eq!(rig.gpu_bytes(addr - 256, 256), level1, "level 1 is the box filter of level 0");
    assert_eq!(rig.gpu_bytes(addr - 256 - 64, 64), level2, "level 2 of level 1");
    /* the sample reads the image through its chain, no tiling */
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    assert_eq!(last_tex_state(&rig), (addr, 2), "the image's level 0, max level 2");
    assert_eq!((tilings(), tiled().0), (n, 0));
    assert_eq!(v3d_flush(), 0);
    /* a render keeps the chain (GL's levels 1.. stay until the next
     * glGenerateMipmap): the next frame reads the image still */
    assert_eq!(frame(&rig, &mut t), addr);
    assert_eq!(tilings(), n);
    /* the GL core asks for the chain back: the storage's level 1 holds the
     * filtered texels at the GL core's pitch, and the next sample is a copy */
    chain_back(&t, 3);
    assert_eq!(rt_images().6, 1);
    let (off, pitch, rows) = texture::gl_level(16, 16, 1);
    assert_eq!((pitch, rows), (8, 8));
    let storage = rig.gpu_bytes(t.base_gpu as u32 + off, 8 * 8 * 4);
    assert_eq!(storage, level1, "the storage's level 1 is the chain's");
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_ne!(last_tex_base(&rig), addr, "a copy again");
    assert_eq!(tilings(), n + 1);
    assert_eq!(v3d_flush(), 0);
    chain_back(&t, 3);
    assert_eq!(rt_images().6, 1, "nothing to give back twice");
}

/// glGenerateMipmap stays the GL core's (the hook answers 0) for a texture
/// with no image: one never sampled after a render, one whose stamp moved
/// since its image was stored, one whose image has fewer levels than the
/// chain (laid out again for the next render), one on an OSAL without the
/// TFU, and one whose TFU job failed (counted, logged once).
#[test]
fn glgeneratemipmap_stays_on_the_cpu_without_an_image_the_tfu_or_the_stamp() {
    let rig = tfu_rig(Some(1), 0);
    let mut t = texture(0, 16, 16, 3);
    t.content_gen = 3;
    assert_eq!(generate(&t, 3), 0, "never sampled after a render: not registered");
    frame(&rig, &mut t);
    assert_eq!(generate(&t, 3), 0, "registered, but no render since: no image");
    frame(&rig, &mut t);
    assert_eq!(rt_images().0, 1);
    t.content_gen = 4;
    assert_eq!(generate(&t, 3), 0, "a stamp the image was not stored under");
    t.content_gen = 3;
    assert_eq!(generate(&t, 4), 0, "more levels than the image was laid out with");
    assert_eq!(rt_images().0, 0, "the image went, to be laid out with the chain");
    frame(&rig, &mut t);
    assert_eq!(rt_images().0, 1, "the next render made the 4-level image");
    assert_eq!(generate(&t, 4), 1);
    assert_eq!(rt_images().5, 1);
    drop(rig);
    /* no TFU: the CPU's chain, as before */
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 16, 16, 3);
    t.content_gen = 3;
    frame(&rig, &mut t);
    frame(&rig, &mut t);
    assert_eq!(rt_images().0, 1);
    assert_eq!(generate(&t, 3), 0, "an OSAL without the TFU");
    assert_eq!(with_platform(|p| p.tfu_runs.len()), 0);
    drop(rig);
    /* a TFU job that fails: 0, the GL core filters, logged once */
    let rig = tfu_rig(Some(1), -5);
    let mut t = texture(0, 16, 16, 3);
    t.content_gen = 3;
    frame(&rig, &mut t);
    frame(&rig, &mut t);
    assert_eq!(generate(&t, 3), 0);
    assert_eq!(generate(&t, 3), 0);
    assert_eq!(rt_images().5, 0);
    assert_eq!(rig.logs_with("the TFU did not make the 3-level chain"), 1);
    let image = render_uif(&rig);
    assert_ne!(image, 0);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_ne!(last_tex_base(&rig), image, "a copy");
}

/// A texture registered from a sample without a mipmap filter has a
/// one-level image; sampled with one later, it is laid out again with the
/// chain by the next render (a copy meanwhile).
#[test]
fn a_sample_with_more_levels_lays_the_image_out_again() {
    let rig = tfu_rig(Some(1), 0);
    let mut chain = texture(0, 16, 16, 3);
    chain.content_gen = 3;
    let mut one = GlesTextureBind { mip_filter: 0, mip_levels: 1, ..chain };
    frame(&rig, &mut one);
    let addr = frame(&rig, &mut one);
    assert_ne!(addr, 0);
    let bytes = rt_images().1;
    chain.target_kick = one.target_kick;
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, chain)), 0);
    assert_ne!(last_tex_base(&rig), addr, "a copy: the image has one level");
    assert_eq!(tilings(), n + 1);
    assert_eq!(rt_images().0, 0, "the one-level image went");
    assert_eq!(v3d_flush(), 0);
    frame(&rig, &mut chain);
    assert!(rt_images().1 > bytes, "the next render made the chain's image");
    assert_eq!(generate(&chain, 3), 1);
    let image = frame(&rig, &mut chain);
    assert_eq!(image, render_uif(&rig), "sampled from it");
    /* and the one-level view reads the same image, level 0 alone */
    one.target_kick = chain.target_kick;
    assert_eq!(draw(&sampling(&rig, one)), 0);
    assert_eq!(last_tex_state(&rig), (image, 0));
    assert_eq!(v3d_flush(), 0);
}

/// glGenerateMipmap delegated while the render is in flight (the async
/// swap): the hook does not wait for it; the TFU job is queued behind the
/// render's fence and runs when the render has (the OSAL's order), and a
/// CPU read of the chain (the GL core asking for it back) waits for that
/// fence first, then reads what the TFU wrote.
#[test]
fn a_delegated_glgeneratemipmap_queues_the_tfu_behind_the_render_in_flight() {
    let rig = Rig::new();
    with_platform(|p| (p.async_jobs, p.tfu_min) = (true, Some(1)));
    rig.init();
    let mut t = texture(0, 16, 16, 3);
    t.content_gen = 3;
    frame(&rig, &mut t);
    complete_jobs();
    frame(&rig, &mut t);
    complete_jobs();
    assert_eq!(draw(&render_into(&rig, &t)), 0);
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    assert!(v3d_kick_pending(t.target_kick), "the render runs");
    let fence = with_platform(|p| p.kicks.len());
    let (_, waits) = counters();
    let runs = with_platform(|p| p.tfu_runs.len());
    assert_eq!(generate(&t, 3), 1, "{:?}", rig.gl.logs());
    assert_eq!(counters().1, waits, "no CPU wait");
    assert!(v3d_kick_pending(t.target_kick), "the render still runs");
    assert_eq!(with_platform(|p| (p.tfu_runs.len(), p.tfu_queued.len())), (runs, 1), "queued, not run");
    assert_eq!(with_platform(|p| p.tfu_queued[0].0), fence, "behind the render's fence");
    /* the GL core takes the chain back: the CPU waits for the render (and so
     * the TFU behind it) before it reads the levels */
    chain_back(&t, 3);
    assert_eq!(counters().1, waits + 2, "the read waits for both jobs outstanding: the older (done) and the render");
    assert_eq!(with_platform(|p| (p.tfu_runs.len(), p.tfu_queued.len())), (runs + 1, 0), "the TFU ran first");
    assert_eq!(rt_images().6, 1, "written back");
}

/// The OSAL's queue full (-EBUSY): the hook waits for the job in flight
/// (which runs the TFU jobs it holds), counted, and the job runs at once.
#[test]
fn a_full_tfu_queue_waits_for_the_job_in_flight_once() {
    let rig = Rig::new();
    with_platform(|p| (p.async_jobs, p.tfu_min, p.tfu_queue_max) = (true, Some(1), 1));
    rig.init();
    let mut a = texture(0, 16, 16, 3);
    a.content_gen = 3;
    let mut b = GlesTextureBind { content_gen: 5, ..texture(0, 16, 16, 3) };
    for t in [&mut a, &mut b] {
        frame(&rig, t);
        complete_jobs();
        frame(&rig, t);
        complete_jobs();
    }
    /* a render in flight, both chains asked for behind it */
    assert_eq!(draw(&render_into(&rig, &a)), 0);
    assert_eq!(v3d_flush(), 0);
    let (_, waits) = counters();
    assert_eq!(generate(&a, 3), 1);
    assert_eq!(counters().1, waits, "queued, no wait");
    assert_eq!(generate(&b, 3), 1, "{:?}", rig.gl.logs());
    assert_eq!(counters().1, waits + 2, "the queue was full: the wait covers both jobs outstanding");
    let (mut queued, mut full) = (0u32, 0u32);
    // SAFETY: two live out-parameters.
    unsafe { v3d_mip_counters(core::ptr::null_mut(), &mut queued, &mut full) };
    assert_eq!((queued, full), (1, 1));
    assert_eq!(with_platform(|p| p.tfu_queued.len()), 0, "both ran");
}

/// Every call glGenerateMipmap stays on the CPU for is counted by its
/// reason (`v3d_mip_counters`), and the counters are printed beside the
/// arena line of frame 1 with the chains made.
#[test]
fn glgeneratemipmap_refusals_are_counted_by_reason_beside_the_frame_1_arena_line() {
    let rig = tfu_rig(Some(1), 0);
    let mut t = texture(0, 16, 16, 3);
    t.content_gen = 3;
    let counters = || {
        let mut r = [0u32; V3D_MIP_REFUSALS];
        let (mut q, mut f) = (0u32, 0u32);
        // SAFETY: V3D_MIP_REFUSALS values and two live out-parameters.
        unsafe { v3d_mip_counters(r.as_mut_ptr(), &mut q, &mut f) };
        (r, q, f)
    };
    // SAFETY: a null binding, which the hook refuses.
    assert_eq!(unsafe { v3d_generate_mipmap(core::ptr::null(), 3) }, 0);
    assert_eq!(generate(&t, 1), 0, "one level");
    assert_eq!(generate(&GlesTextureBind { bgra: 1, ..t }, 3), 0, "not a render image's format");
    assert_eq!(generate(&t, 3), 0, "never sampled after a render");
    frame(&rig, &mut t);
    assert_eq!(generate(&t, 3), 0, "registered, no image stored");
    assert_eq!(generate(&GlesTextureBind { content_gen: 4, ..t }, 3), 0, "a CPU write since");
    assert_eq!(generate(&t, 4), 0, "more levels than the image");
    frame(&rig, &mut t);
    assert_eq!(generate(&t, 4), 1);
    let (r, queued, full) = counters();
    /* uninit levels not-rt unknown stamp relayout no-tfu flush unstored wait layout pack no-query tfu */
    assert_eq!(r, [0, 1, 2, 1, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0]);
    assert_eq!((queued, full), (0, 0), "the job completed inside its submit: the TFU ran at once");
    v3d_surface_swapped(rig.draw().target_gpu);
    assert_eq!(v3d_flush(), 0);
    let line: Vec<String> = rig.gl.logs().into_iter().filter(|l| l.contains("TFU mipmap at frame")).collect();
    assert_eq!(
        line,
        ["yakogl/v3d: TFU mipmap at frame 1: 1 chains (0 queued behind a job, 0 waited), refused: uninit 0, \
          levels 1, not-rt 2, unknown 1, stamp 1, relayout 1, no-tfu 0, flush 0, unstored 1, wait 0, layout 0, \
          pack 0, no-query 0, tfu 0"],
    );
}

/// The textures that keep the copy, by name: one with no stamp (an
/// EGLImage's) and one nothing rendered into. Neither gets an image.
#[test]
fn an_unstamped_and_an_unrendered_texture_keep_the_copy() {
    let rig = Rig::new();
    rig.init();
    let mut ext = texture(0, 16, 16, 1);
    ext.content_gen = 0;
    frame(&rig, &mut ext);
    frame(&rig, &mut ext);
    assert_eq!(rt_images(), (0, 0, 0, 0, 0, 0, 0), "an unstamped texture is never registered");
    let mut plain = texture(0, 16, 16, 1);
    plain.content_gen = 6;
    let (n, kept) = (tilings(), tiled().0);
    assert_eq!(draw(&sampling(&rig, plain)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&sampling(&rig, plain)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rt_images(), (0, 0, 0, 0, 0, 0, 0), "a texture no draw rendered into is not registered");
    assert_eq!((tilings(), tiled().0), (n + 1, kept + 1), "its kept copy, as before");
}

/// texture_released frees the image with the copies: the storage's address
/// may be handed out again.
#[test]
fn a_released_render_target_frees_its_image() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 3;
    frame(&rig, &mut t);
    frame(&rig, &mut t);
    assert_eq!(rt_images().0, 1);
    let before = with_platform(|p| p.frees.len());
    v3d_texture_released(t.base_gpu, texture::gl_bytes(&t) as u64);
    assert_eq!(rt_images().0, 0);
    assert_eq!(with_platform(|p| p.frees.len()), before + 1, "the image is freed");
    /* the storage comes back under another texture: a copy, then an image of its own */
    t.content_gen = 9;
    frame(&rig, &mut t);
    assert_eq!(rt_images().0, 0, "registered again, no image yet");
    frame(&rig, &mut t);
    assert_eq!(rt_images().0, 1);
}

/// No GPU memory for the image: the render stores the raster target alone
/// (logged once), the sample tiles its copy as before, and the image is
/// made by the next render once memory is there.
#[test]
fn no_memory_for_the_render_image_keeps_the_copy_path() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 64, 64, 1);
    t.content_gen = 3;
    frame(&rig, &mut t);
    /* the image is the layout's 16384 bytes + 4096; a kept copy asks the same */
    with_platform(|p| p.fail_alloc_from = Some(16384 + 4096));
    assert_eq!(draw(&render_into(&rig, &t)), 0);
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    assert_eq!(last_uif(&rig).0, 0, "no image: the raster store alone");
    assert_eq!(rt_images().4, 1);
    assert_eq!(rig.logs_with("no GPU memory for the 64x64 render image"), 1);
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(tilings(), n + 1, "tiled (into the arena: the copy found no memory either)");
    assert_eq!(v3d_flush(), 0);
    with_platform(|p| p.fail_alloc_from = None);
    let addr = frame(&rig, &mut t);
    assert_ne!(addr, 0, "the next render made the image, and the sample read it");
    let jobs = rig.jobs();
    assert_eq!(jobs[jobs.len() - 2].target.uif_address, addr, "stored by the render's job");
    assert_eq!(rt_images().0, 1);
}

/// RT_IMAGES slots: one more render target gives up the least recently
/// used, whose next sample is a copy again (and registers it again). A
/// chain the evicted image held goes back into the storage first.
#[test]
fn the_least_recently_used_render_image_goes_when_the_table_is_full() {
    let rig = tfu_rig(Some(1), 0);
    let mut ts: Vec<GlesTextureBind> =
        (0..=RT_IMAGES as u32).map(|i| GlesTextureBind { content_gen: 100 + i, ..texture(0, 16, 16, 3) }).collect();
    for t in ts.iter_mut().take(RT_IMAGES) {
        frame(&rig, t);
        frame(&rig, t);
    }
    assert_eq!(rt_images().0 as usize, RT_IMAGES, "every slot holds an image");
    assert_eq!(generate(&ts[0], 3), 1, "the first texture's chain is the TFU's");
    /* the others used since: the first is the least recently used again */
    for t in ts.iter_mut().take(RT_IMAGES).skip(1) {
        frame(&rig, t);
    }
    let direct = rt_images().2;
    /* one more: registered in the first texture's slot (the least recently
     * used), whose chain goes back into its storage */
    frame(&rig, &mut ts[RT_IMAGES]);
    assert_eq!(rt_images().0 as usize, RT_IMAGES - 1, "the first texture's image went");
    assert_eq!(rt_images().6, 1, "its chain was written back");
    frame(&rig, &mut ts[RT_IMAGES]);
    assert_eq!(rt_images().0 as usize, RT_IMAGES);
    assert_eq!(rt_images().2, direct);
    assert_eq!(generate(&ts[RT_IMAGES], 3), 1);
    let image = frame(&rig, &mut ts[RT_IMAGES]);
    assert_eq!(image, render_uif(&rig));
    assert_eq!(rt_images().2, direct + 1);
    /* the first texture: a copy again (and registered again, in the second's slot) */
    let n = tilings();
    frame(&rig, &mut ts[0]);
    assert_eq!(tilings(), n + 1, "the first texture is a copy again");
    assert!(rt_images().0 as usize <= RT_IMAGES);
}

/// A render into the same storage as another target (another size or pitch
/// at the texture's address, a level above 0 attached): the raster changed
/// under the image, which no longer holds what the storage holds, so the
/// next sample is a copy until the target itself is rendered again; a chain
/// the image held goes back into the storage before the job that writes
/// the storage is recorded.
#[test]
fn a_render_into_another_view_of_the_storage_invalidates_the_image() {
    let rig = tfu_rig(Some(1), 0);
    let mut t = texture(0, 16, 16, 3);
    t.content_gen = 3;
    frame(&rig, &mut t);
    frame(&rig, &mut t);
    assert_eq!(generate(&t, 3), 1);
    let addr = frame(&rig, &mut t);
    assert_ne!(addr, 0);
    /* an 8x8 target at the same address */
    let mut small = t;
    (small.width, small.height, small.stride_texels) = (8, 8, 8);
    assert_eq!(draw(&render_into(&rig, &small)), 0);
    assert_eq!(rt_images().6, 1, "the chain went back into the storage before the render");
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    assert_eq!(last_uif(&rig).0, 0, "no image of its own for the view");
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_ne!(last_tex_base(&rig), addr, "a copy: the image is not what the storage holds");
    assert_eq!(tilings(), n + 1);
    assert_eq!(v3d_flush(), 0);
    /* the target's own render stores level 0 again; the chain is the GL
     * core's until the next glGenerateMipmap, so a mip-filtered sample is
     * a copy, and a level-0 sample reads the image */
    assert_eq!(draw(&render_into(&rig, &t)), 0);
    t.target_kick = v3d_kick_id();
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_ne!(last_tex_base(&rig), addr);
    assert_eq!(v3d_flush(), 0);
    let one = GlesTextureBind { mip_filter: 0, mip_levels: 1, ..t };
    assert_eq!(draw(&sampling(&rig, one)), 0);
    assert_eq!(last_tex_state(&rig), (addr, 0));
    assert_eq!(v3d_flush(), 0);
    assert_eq!(generate(&t, 3), 1);
    assert_eq!(frame(&rig, &mut t), addr);
}

/// At most TILED_ENTRIES copies: the least recently sampled goes, and the
/// live bytes never pass the budget.
#[test]
fn the_least_recently_sampled_copy_goes_when_the_cache_is_full() {
    let rig = Rig::new();
    rig.init();
    let ts: Vec<GlesTextureBind> =
        (0..=TILED_ENTRIES as u32).map(|i| GlesTextureBind { content_gen: 100 + i, ..texture(0, 4, 4, 1) }).collect();
    for t in &ts {
        assert_eq!(draw(&sampling(&rig, *t)), 0);
        assert_eq!(v3d_flush(), 0);
    }
    let (live, bytes, high) = tiled();
    assert_eq!(live as usize, TILED_ENTRIES);
    assert!(bytes as usize <= DEFAULT_TILED_BUDGET && high as usize <= DEFAULT_TILED_BUDGET);
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, ts[1])), 0);
    assert_eq!(tilings(), n, "the second texture is still kept");
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&sampling(&rig, ts[0])), 0);
    assert_eq!(tilings(), n + 1, "the first, sampled longest ago, was dropped");
}

fn evictions() -> ([u32; 2], [u64; 2], [u32; 2]) {
    let (mut c, mut b, mut h) = ([0u32; 2], [0u64; 2], [0u32; 2]);
    // SAFETY: writable pairs.
    unsafe { v3d_tiled_evictions(c.as_mut_ptr(), b.as_mut_ptr(), h.as_mut_ptr()) };
    (c, b, h)
}

/// Draw each texture once, all in one job (no flush between).
fn sample_all(rig: &Rig, ts: &[GlesTextureBind]) {
    for t in ts {
        assert_eq!(draw(&sampling(rig, *t)), 0, "{:?}", rig.gl.logs());
    }
}

fn misses() -> (u32, u32) {
    let (mut m, mut r) = (0u32, 0u32);
    // SAFETY: live out-parameters.
    unsafe { v3d_tiled_misses(&mut m, &mut r) };
    (m, r)
}

/// A job that kicks without sampling any kept copy (an unstamped texture is
/// never kept), so every kept copy ages by one job.
fn an_unrelated_job(rig: &Rig) {
    assert_eq!(draw(&sampling(rig, texture(0, 4, 4, 1))), 0);
    assert_eq!(v3d_flush(), 0);
}

/// The cluster's slow frames (v3d_gl_cluster DEMO3N, 2026-09-25): copies
/// that are equally old tie, and the first slot went -- the texture tiled
/// first, the 1920x720 backdrop -- so a frame re-tiled 8.5 MB. Of equally old
/// copies (none sampled by the last two jobs), the smallest goes now, and
/// the big copy is still kept.
///
/// Revert check: the old victim (the first of the tied) evicts the big one.
#[test]
fn a_tied_eviction_gives_up_the_smallest_copy_not_the_first() {
    let rig = Rig::new();
    rig.init();
    let big = GlesTextureBind { content_gen: 1000, ..texture(0, 64, 64, 1) };
    let mut ts = vec![big];
    ts.extend((1..TILED_ENTRIES as u32).map(|i| GlesTextureBind { content_gen: 1000 + i, ..texture(0, 4, 4, 1) }));
    sample_all(&rig, &ts); /* job A: the table full, the big one in the first slot */
    assert_eq!(v3d_flush(), 0);
    an_unrelated_job(&rig); /* job B: every copy two jobs old, all tie */
    assert_eq!(tiled().0 as usize, TILED_ENTRIES);
    let before = evictions();
    let newcomer = GlesTextureBind { content_gen: 2000, ..texture(0, 4, 4, 1) };
    assert_eq!(draw(&sampling(&rig, newcomer)), 0); /* job C */
    let after = evictions();
    assert_eq!(after.0[0] - before.0[0], 1, "one eviction, for the entry count");
    assert_eq!(after.2[0], before.2[0], "not hot: sampled two jobs ago");
    let small_bytes = 4096 + texture::layout(&ts[1]).unwrap().size as u64;
    assert_eq!(after.1[0] - before.1[0], small_bytes, "a small copy's bytes went, not the big one's");
    assert_eq!(v3d_flush(), 0);
    let n = tilings();
    assert_eq!(draw(&sampling(&rig, big)), 0); /* job D */
    assert_eq!(tilings(), n, "the big copy is still kept: not tiled again");
}

/// A frame's working set far above the old 64 (the cluster's is about 86):
/// 200 copies sampled in one job are all found in the next, with no eviction
/// and no tiling.
///
/// Revert check: with TILED_ENTRIES 64 it fails at "all 200 kept" (64).
#[test]
fn a_200_entry_working_set_survives_a_frame_without_an_eviction() {
    let rig = Rig::new();
    rig.init();
    let ts: Vec<GlesTextureBind> =
        (0..200u32).map(|i| GlesTextureBind { content_gen: 6000 + i, ..texture(0, 4, 4, 1) }).collect();
    sample_all(&rig, &ts); /* job A: 200 tilings */
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tiled().0, 200, "all 200 kept");
    let (before, n) = (evictions(), tilings());
    sample_all(&rig, &ts); /* job B: the same frame again */
    assert_eq!(v3d_flush(), 0);
    let after = evictions();
    assert_eq!(tilings(), n, "nothing tiled again");
    assert_eq!((after.0, after.2), (before.0, before.2), "no eviction, hot or not");
}

/// The working set is never given up for a newcomer (DEMO4: giving it up
/// lost the backdrop's copy, which then found no carve-out room). With the
/// table full of copies the last job sampled, a newcomer evicts nothing: it
/// is tiled into the job's arena and counted as refused; once the working
/// set has aged past the last two jobs, the newcomer is kept.
///
/// Revert check: without the rule, the newcomer evicts a hot copy (the
/// eviction counters move, and the refused counter does not).
#[test]
fn the_working_set_is_never_evicted_for_a_newcomer() {
    let rig = Rig::new();
    rig.init();
    let ts: Vec<GlesTextureBind> =
        (0..TILED_ENTRIES as u32).map(|i| GlesTextureBind { content_gen: 3000 + i, ..texture(0, 4, 4, 1) }).collect();
    sample_all(&rig, &ts); /* job A: the table full */
    assert_eq!(v3d_flush(), 0);
    let (ev, (nomem, refused), n) = (evictions(), misses(), tilings());
    let newcomer = GlesTextureBind { content_gen: 4000, ..texture(0, 4, 4, 1) };
    assert_eq!(draw(&sampling(&rig, newcomer)), 0, "{:?}", rig.gl.logs()); /* job B */
    assert_eq!(evictions().0, ev.0, "nothing evicted: every copy was sampled by the last job");
    assert_eq!(misses(), (nomem, refused + 1), "the newcomer refused, counted; no memory failure");
    assert_eq!(tilings(), n + 1, "the newcomer tiled into the job's arena");
    assert_eq!(tiled().0 as usize, TILED_ENTRIES);
    assert_eq!(v3d_flush(), 0);
    let m = tilings();
    sample_all(&rig, &ts[..1]); /* job C: the working set is still kept */
    assert_eq!(tilings(), m, "the working set was not tiled again");
    assert_eq!(v3d_flush(), 0);
    an_unrelated_job(&rig); /* job D: the rest two jobs old */
    let r = misses().1;
    assert_eq!(draw(&sampling(&rig, newcomer)), 0); /* job E */
    assert_eq!(misses().1, r, "now kept, not refused");
    assert_eq!(evictions().0[0], ev.0[0] + 1, "by one eviction of a copy the last two jobs did not sample");
}

/// An eviction the byte budget forces is counted under that cause, and the
/// dump prints one line per live copy.
#[test]
fn an_eviction_for_the_byte_budget_is_counted_as_such_and_the_dump_lists_the_copies() {
    let rig = Rig::new();
    rig.init();
    let ts: Vec<GlesTextureBind> =
        (0..3u32).map(|i| GlesTextureBind { content_gen: 5000 + i, ..texture(0, 32, 32, 1) }).collect();
    let one = 4096 + texture::layout(&ts[0]).unwrap().size as usize;
    g().tiled_budget = 2 * one; /* room for two copies */
    sample_all(&rig, &ts[..2]); /* job A */
    assert_eq!(v3d_flush(), 0);
    an_unrelated_job(&rig); /* job B: both two jobs old */
    let before = evictions();
    sample_all(&rig, &ts[2..]); /* job C */
    let after = evictions();
    assert_eq!(after.0[1] - before.0[1], 1, "the third copy gave up one for the byte budget");
    assert_eq!(after.0[0], before.0[0], "none for the entry count");
    assert_eq!(after.1[1] - before.1[1], one as u64);
    assert_eq!(v3d_tiled_dump(), 2);
    assert_eq!(rig.gl.logs().iter().filter(|l| l.contains("yakogl/v3d: tiled[")).count(), 2);
}

/// Through the GL core: glDeleteTextures tells the backend before the OSAL's
/// free of the storage is scheduled, so the copy is freed first and nothing
/// is left of it.
#[test]
fn deleting_a_texture_frees_its_copy_before_its_storage() {
    use crate::gl::consts::*;
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
    let rig = Rig::new();
    rig.init();
    /* the GL core reaches this backend's table, as in the product (the rig
     * drives the backend directly otherwise); TestGl's drop resets it */
    // SAFETY: serialised by the rig's TestGl.
    unsafe { crate::backend::g_backend = &yakogl_backend_v3d };
    let mut name = 0;
    let texels = [0x40u8; 4 * 4 * 4];
    // SAFETY: plain GL calls on the rig's context with live pointers.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_2D, name);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as i32,
            4,
            4,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            texels.as_ptr() as *const c_void,
        );
    }
    let (storage, stamp) = {
        let t = &rig.gl.ctx().objs().textures[name as usize];
        (t.mem.gpu, t.content_gen)
    };
    assert_ne!(stamp, 0, "an uploaded texture is stamped");
    let mut b = texture(0, 4, 4, 1);
    (b.base_gpu, b.content_gen) = (storage, stamp);
    assert_eq!(draw(&sampling(&rig, b)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tiled().0, 1);
    let before = with_platform(|p| p.frees.len());
    // SAFETY: a live name.
    unsafe { glDeleteTextures(1, &name) };
    assert_eq!(tiled().0, 0, "nothing is left of the copy");
    let frees = with_platform(|p| p.frees[before..].to_vec());
    let at = |gpu: u64| frees.iter().position(|&f| f == gpu);
    let storage_freed = at(storage).expect("the storage is freed");
    assert_eq!(frees.len(), 2, "the copy and the storage: {frees:x?}");
    assert!(frees[..storage_freed].iter().any(|&f| f != storage), "the copy is freed before the storage: {frees:x?}");
}

/* ---- depth and stencil kept between jobs (M6 step C) --------------------- */

/// A draw whose target owns depth/stencil buffer `id` (the GL core's
/// `ds_persist`/`ds_id`), testing and writing depth.
fn kept_depth_draw(rig: &Rig, id: u32) -> GlesDraw {
    let mut d = depth_draw(rig);
    (d.ds_persist, d.ds_id) = (true, id);
    d
}

fn job_clear(rig: &Rig, i: usize) -> V3dJobClear {
    rig.jobs()[i].clear.expect("the job ended")
}

/// The job that writes depth stores the buffer; the next job loads it, so a
/// depth draw after a split reads what the earlier job wrote instead of
/// being refused.
#[test]
fn a_depth_buffer_is_stored_by_the_job_that_writes_it_and_loaded_by_the_next() {
    let rig = Rig::new();
    rig.init();
    let mut d = kept_depth_draw(&rig, 7);
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    let first = job_clear(&rig, 0);
    assert!(first.zs_store && !first.zs_load, "cleared and written: stored, nothing to load");
    assert!(first.zs_address != 0 && first.zs_address.is_multiple_of(4096), "0x{:x}", first.zs_address);
    let mut again = kept_depth_draw(&rig, 7);
    again.do_clear = false;
    assert_eq!(draw(&again), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let second = job_clear(&rig, 1);
    assert!(second.zs_load && second.zs_store, "the earlier depth is loaded, and stored again");
    assert_eq!(second.zs_address, first.zs_address, "one buffer for the id");
}

/// A job that only clears stores nothing: the next job starts from the
/// clear values, as a job with no buffer does.
#[test]
fn a_job_that_only_clears_stores_nothing_and_the_next_starts_from_the_clear() {
    let rig = Rig::new();
    rig.init();
    let mut c = kept_depth_draw(&rig, 3);
    (c.index_count, c.do_clear_depth, c.clear_depth) = (0, true, 0.25);
    assert_eq!(draw(&c), 0);
    assert_eq!(v3d_flush(), 0);
    let only = job_clear(&rig, 0);
    assert!(!only.zs_store && !only.zs_load);
    let mut d = kept_depth_draw(&rig, 3);
    d.do_clear = false;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    let next = job_clear(&rig, 1);
    assert_eq!(next.z, 0.25, "the clear value carried over");
    assert!(!next.zs_load && next.zs_store);
}

/// Two window surfaces swapped in turn (a double-buffered window, DEMO12)
/// share one D24S8: after surface A's swap its buffer holds nothing (EGL 1.5
/// 3.10.1), so surface B's first job takes it instead of a new 5.5 MB one at
/// 1920x720. A buffer that holds something is never lent: B, not swapped,
/// keeps its depth/stencil, and A's next job gets a buffer of its own.
///
/// Revert check: without the lending in `zs_buf` the two surfaces'
/// addresses differ in the first half and `zs_lent` stays 0.
#[test]
fn two_windows_swapped_in_turn_share_one_depth_stencil() {
    let rig = Rig::new();
    rig.init();
    let live = || g().zsbufs.iter().filter(|b| b.live()).count();
    let mut a = stencil_stroke(&rig);
    (a.ds_persist, a.ds_id) = (true, 0x8000_0001);
    let mut b = a;
    (b.target_gpu, b.ds_id) = (a.target_gpu + 0x10_0000, 0x8000_0002);
    assert_eq!(draw(&a), 0);
    v3d_surface_swapped(a.target_gpu);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&b), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(job_clear(&rig, 0).zs_address, job_clear(&rig, 1).zs_address, "B took A's buffer");
    assert_eq!((g().zs_lent, live()), (1, 1));
    assert!(!job_clear(&rig, 1).zs_load, "it holds nothing: not loaded");
    /* B was not swapped: its buffer holds what its job stored */
    assert!(job_clear(&rig, 1).zs_store);
    assert_eq!(draw(&a), 0);
    assert_eq!(v3d_flush(), 0);
    assert_ne!(job_clear(&rig, 2).zs_address, job_clear(&rig, 1).zs_address, "B's is not lent");
    assert_eq!((g().zs_lent, live()), (1, 2));
    /* and B finds its own again, with what it stored */
    assert_eq!(draw(&b), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(job_clear(&rig, 3).zs_address, job_clear(&rig, 1).zs_address);
    assert!(job_clear(&rig, 3).zs_load, "B's stencil is loaded");
}

/// A depth/stencil allocation the OSAL refuses: the draw is refused by name
/// (-ENOMEM, GL_OUT_OF_MEMORY), never drawn with depth on-chip only (a later
/// job could not load what it wrote); no live buffer is given up for it; the
/// OSAL is not asked again for ZS_RETRY_JOBS requests, and once it has the
/// memory the next ask after the hold-off gets it. DEMO12 asked for the
/// second window's 5.5 MB at every job, 423 times, each first giving up a
/// live buffer ("id 6 is given up", record line 204), until one ask took the
/// carve-out's last 5.5 MB.
///
/// Revert checks: the on-chip fallback returns 0 here; evicting before the
/// allocation leaves 6 live buffers after the refusal; without the hold-off
/// every request asks again; a hold-off counted in jobs never ends (every
/// draw here is refused, so no job runs).
#[test]
fn a_depth_stencil_without_memory_is_refused_by_name_and_asked_again_later() {
    let rig = Rig::new();
    rig.init();
    let live = || g().zsbufs.iter().filter(|b| b.live()).count();
    let depth_job = |id: u32| {
        let mut d = kept_depth_draw(&rig, id);
        (d.do_clear_depth, d.clear_depth) = (true, 1.0);
        let rc = draw(&d);
        assert_eq!(v3d_flush(), 0);
        rc
    };
    /* seven buffers hold stored depth: what is refused below is the
     * memory, not a slot */
    for id in 1..8 {
        assert_eq!(depth_job(id), 0, "{:?}", rig.gl.logs());
    }
    assert_eq!(live(), 7, "seven slots hold a written buffer");
    let asks = || with_platform(|p| p.refused_allocs.len());
    with_platform(|p| p.fail_alloc_from = Some(1));
    assert_eq!(depth_job(100), -ENOMEM, "refused, not drawn on-chip");
    assert_eq!((asks(), live()), (1, 7), "asked once, and no buffer given up for it");
    let logs = rig.gl.logs(); /* read once: logs() drains the recorder */
    let refused: Vec<_> = logs.iter().filter(|l| l.contains(ds_retention::REFUSED)).collect();
    assert_eq!(refused.len(), 1, "{logs:?}");
    assert!(refused[0].contains("no GPU memory") && refused[0].contains("object 0x"), "{refused:?}");
    for _ in 1..ZS_RETRY_JOBS {
        assert_eq!(depth_job(100), -ENOMEM);
    }
    assert_eq!(asks(), 1, "not asked again within {ZS_RETRY_JOBS} requests");
    assert_eq!(g().zs_skipped, ZS_RETRY_JOBS - 1);
    assert_eq!(rig.logs_with(ds_retention::REFUSED), 0, "the same cause is logged once");
    assert_eq!(depth_job(100), -ENOMEM);
    assert_eq!(asks(), 2, "asked again once {ZS_RETRY_JOBS} requests have passed");
    /* the memory back: the next ask after the hold-off gets it */
    with_platform(|p| p.fail_alloc_from = None);
    let tries = (0..=ZS_RETRY_JOBS).position(|_| depth_job(100) == 0);
    assert_eq!(tries, Some(ZS_RETRY_JOBS as usize - 1), "drawn at the first ask after the hold-off");
    assert!(g().zsbufs.iter().any(|b| b.live() && b.id == 100), "allocated once asked again");
    assert_eq!(live(), 8, "into a free slot; nothing given up");
}

/// GFXH-1461: with the stencil stored, a depth-only clear cannot be a TLB
/// clear (the load of the other half loses it): it is drawn by the quad, and
/// the job loads both.
#[test]
fn a_partial_clear_whose_other_half_is_stored_is_drawn_by_the_quad() {
    let rig = Rig::new();
    rig.init();
    let mut s = kept_depth_draw(&rig, 5);
    (s.do_clear_depth, s.do_clear_stencil) = (true, true);
    (s.stencil, s.stencil_func, s.stencil_dppass) = (true, GLES_CMP_ALWAYS, GLES_SOP_REPLACE);
    (s.stencil_write_mask, s.stencil_cmp_mask) = (0xff, 0xff);
    assert_eq!(draw(&s), 0);
    assert_eq!(v3d_flush(), 0);
    assert!(job_clear(&rig, 0).zs_store);
    let before = rig.draws().len();
    let mut d = kept_depth_draw(&rig, 5);
    (d.do_clear, d.do_clear_depth, d.clear_depth) = (false, true, 0.5);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(rig.draws().len(), before + 2, "the depth clear is a quad, then the draw");
    assert_eq!(v3d_flush(), 0);
    let j = job_clear(&rig, 1);
    assert!(j.zs_load, "the stencil is loaded");
    assert_eq!(j.z, 1.0, "the TLB did not take the depth clear");
}

/// A new size under the same id gets a new buffer, which starts Untouched:
/// nothing to load.
#[test]
fn a_new_size_under_the_same_id_gets_a_new_buffer() {
    let rig = Rig::new();
    rig.init();
    let mut d = kept_depth_draw(&rig, 9);
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    let first = job_clear(&rig, 0);
    let mut small = kept_depth_draw(&rig, 9);
    (small.target_w, small.target_h, small.viewport, small.do_clear) = (32, 32, [0, 0, 32, 32], false);
    assert_eq!(draw(&small), 0);
    assert_eq!(v3d_flush(), 0);
    let second = job_clear(&rig, 1);
    assert_ne!(second.zs_address, first.zs_address);
    assert!(!second.zs_load, "a new buffer holds nothing to load");
}

/// Id `i` of the cycling tests: a colour target of its own and a D24S8
/// renderbuffer on both points (object 100 + i, one buffer per id), its
/// depth cleared (or not) and written.
fn fbo_depth_job(rig: &Rig, i: u32, clear: bool) -> V3dJobClear {
    let mut d = kept_depth_draw(rig, (i + 1) << 16);
    d.target_gpu = TARGET + 0x10_0000 * (i as u64 + 1);
    (d.depth_id, d.stencil_id) = (100 + i, 100 + i);
    d.do_clear = false;
    d.do_clear_depth = clear;
    assert_eq!(draw(&d), 0, "id {i}: {:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    job_clear(rig, rig.jobs().len() - 1)
}

/// Draw ids 0..ZS_BUFS, each storing depth into a buffer of its own.
fn every_slot_depth_ids(rig: &Rig) -> Vec<V3dJobClear> {
    (0..ZS_BUFS as u32).map(|i| fbo_depth_job(rig, i, true)).collect()
}

/// The refusal (src/ds_retention.rs): with every slot holding stored depth,
/// the next id's (the 33rd's) first depth-tested draw is refused (-ENOMEM, which the GL
/// core reports as GL_OUT_OF_MEMORY) with one named line; no buffer is
/// given up, and id 0's next job loads the depth it stored from the same
/// buffer.
///
/// Revert check: with the old least-recently-used eviction the 33rd draw
/// returns 0 and id 0's next job loads nothing.
#[test]
fn an_id_past_every_slot_is_refused_and_the_first_keeps_its_depth() {
    let rig = Rig::new();
    rig.init();
    let first = every_slot_depth_ids(&rig)[0];
    let allocs = with_platform(|p| p.allocs.len());
    let n = ZS_BUFS as u32;
    let mut d = kept_depth_draw(&rig, (n + 1) << 16);
    (d.target_gpu, d.depth_id, d.stencil_id, d.do_clear) =
        (TARGET + 0x10_0000 * (n as u64 + 1), 100 + n, 100 + n, false);
    assert_eq!(draw(&d), -ENOMEM, "the 33rd is refused");
    let logs = rig.gl.logs();
    let refused: Vec<_> = logs.iter().filter(|l| l.contains(ds_retention::REFUSED)).collect();
    assert_eq!(refused.len(), 1, "{logs:?}");
    assert!(refused[0].contains("object 0x84") && refused[0].contains("all 32"), "{refused:?}");
    assert_eq!(with_platform(|p| p.allocs.len()), allocs, "nothing allocated, nothing given up");
    let again = fbo_depth_job(&rig, 0, false);
    assert_eq!(again.zs_address, first.zs_address, "id 0 finds its own buffer");
    assert!(again.zs_load, "and loads the depth it stored");
}

/// The GL core deleting id 0's depth object frees its buffer, and the
/// 33rd id gets that slot; a whole clear with no draw after it lets a
/// slot go too, and the cleared value comes back when its id returns.
#[test]
fn a_deleted_or_cleared_depth_lets_its_slot_go() {
    let rig = Rig::new();
    rig.init();
    let live = || g().zsbufs.iter().filter(|b| b.live()).count();
    every_slot_depth_ids(&rig);
    v3d_attachment_gone(100);
    assert_eq!(live(), ZS_BUFS - 1, "object 100's buffer is freed");
    fbo_depth_job(&rig, ZS_BUFS as u32, true);
    assert_eq!(live(), ZS_BUFS);
    /* a whole clear of id 1's depth and stencil, nothing drawn after it:
     * a clear value only */
    let mut clear = kept_depth_draw(&rig, 2 << 16);
    (clear.target_gpu, clear.depth_id, clear.stencil_id, clear.do_clear) = (TARGET + 0x10_0000 * 2, 101, 101, false);
    (clear.do_clear_depth, clear.clear_depth, clear.index_count) = (true, 0.25, 0);
    (clear.do_clear_stencil, clear.clear_stencil) = (true, 3);
    assert_eq!(draw(&clear), 0);
    assert_eq!(v3d_flush(), 0);
    fbo_depth_job(&rig, ZS_BUFS as u32 + 1, true); /* takes id 1's slot; its clear is parked */
    assert_eq!(g().zs_parked.len(), 1);
    v3d_attachment_gone(102); /* room for id 1 again */
    let back = fbo_depth_job(&rig, 1, false);
    assert!(!back.zs_load, "a new buffer: nothing loaded");
    assert_eq!(back.z, 0.25, "it starts from the parked clear value");
    assert!(g().zs_parked.is_empty());
}

/// Another depth/stencil buffer on the same colour target is another job.
#[test]
fn a_new_depth_buffer_on_the_same_colour_target_starts_a_new_job() {
    let rig = Rig::new();
    rig.init();
    let mut a = kept_depth_draw(&rig, 1);
    a.do_clear_depth = true;
    assert_eq!(draw(&a), 0);
    /* no clear of its own (a TLB clear would start a job anyway): only the
     * other buffer id can separate the two draws */
    let mut b = kept_depth_draw(&rig, 2);
    b.do_clear = false;
    assert_eq!(draw(&b), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rig.jobs().len(), 2);
    assert_ne!(job_clear(&rig, 0).zs_address, job_clear(&rig, 1).zs_address);
}

/// The load/store words for a buffer's layout: Mesa's UIF XOR becomes
/// NO_XOR, and the height is the padded height in UIF blocks (8 rows);
/// LT and UB-linear carry no height.
#[test]
fn the_zs_buffers_memory_format_and_height_follow_its_layout() {
    let uif = V3dTexSlice { tiling: V3D_TEX_TILING_UIF_XOR, padded_height: 72, ..V3dTexSlice::default() };
    assert_eq!(zs_layout(&uif, 4), (V3D_TEX_TILING_UIF_NO_XOR, 9));
    let no_xor = V3dTexSlice { tiling: V3D_TEX_TILING_UIF_NO_XOR, ..uif };
    assert_eq!(zs_layout(&no_xor, 4), (V3D_TEX_TILING_UIF_NO_XOR, 9));
    let lt = V3dTexSlice { tiling: V3D_TEX_TILING_LINEARTILE, padded_height: 4, ..V3dTexSlice::default() };
    assert_eq!(zs_layout(&lt, 4), (V3D_TEX_TILING_LINEARTILE, 0));
}

/* ---- eglSwapBuffers drops depth/stencil (EGL 1.5 3.10.1) ------------------ */

/// femtovg's stencil stroke: tests stencil == 0 and increments it.
fn stencil_stroke(rig: &Rig) -> GlesDraw {
    let mut d = rig.draw();
    d.do_clear = false;
    (d.stencil, d.stencil_func, d.stencil_ref, d.stencil_cmp_mask) = (true, GLES_CMP_EQUAL, 0, 0xff);
    (d.stencil_dppass, d.stencil_write_mask) = (GLES_SOP_INCR_SAT, 0xff);
    (d.stencil_back_func, d.stencil_back_cmp_mask, d.stencil_back_dppass, d.stencil_back_write_mask) =
        (GLES_CMP_EQUAL, 0xff, GLES_SOP_INCR_SAT, 0xff);
    d
}

/// A target with no depth/stencil buffer: frame 2's first stencil stroke
/// reads what frame 1 wrote only if no swap came between. After a swap the
/// stencil is undefined (EGL 1.5 3.10.1) and starts from the tile buffer's
/// clear, which is what femtovg expects (Slint never clears the stencil).
#[test]
fn after_a_swap_the_next_frames_stencil_stroke_is_not_refused() {
    let rig = Rig::new();
    rig.init();
    let frame = stencil_stroke(&rig);
    assert_eq!(draw(&frame), 0);
    assert_eq!(v3d_flush(), 0);
    /* no swap: the next job would read a stencil nothing kept */
    assert_eq!(draw(&frame), -EINVAL, "without a swap the second job is still refused");
    assert_eq!(rig.logs_with("reads stencil an earlier job wrote"), 1);
    /* the swap: surface_swapped, then the flush */
    v3d_surface_swapped(frame.target_gpu);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(draw(&frame), 0, "after the swap the stroke starts from the clear");
    assert_eq!(v3d_flush(), 0);
}

/// A window with a depth/stencil buffer: the frame's last job, swapped
/// while open, stores nothing, and the next frame loads nothing.
#[test]
fn a_swapped_frames_depth_stencil_is_neither_stored_nor_loaded() {
    let rig = Rig::new();
    rig.init();
    let mut frame = stencil_stroke(&rig);
    (frame.ds_persist, frame.ds_id) = (true, 0x8000_0001);
    assert_eq!(draw(&frame), 0);
    v3d_surface_swapped(frame.target_gpu);
    assert_eq!(v3d_flush(), 0);
    let first = job_clear(&rig, 0);
    assert!(!first.zs_store, "posted: its depth/stencil are undefined, not stored");
    assert_eq!(draw(&frame), 0);
    assert_eq!(v3d_flush(), 0);
    let second = job_clear(&rig, 1);
    assert!(!second.zs_load, "the next frame starts Untouched");
    assert!(second.zs_store, "a job that is not swapped still stores");
    /* without a swap the next job loads what that one stored */
    assert_eq!(draw(&frame), 0);
    assert_eq!(v3d_flush(), 0);
    assert!(job_clear(&rig, 2).zs_load);
}

/// A swap of another surface leaves this target's depth/stencil alone.
#[test]
fn a_swap_of_another_surface_changes_nothing_here() {
    let rig = Rig::new();
    rig.init();
    let frame = stencil_stroke(&rig);
    assert_eq!(draw(&frame), 0);
    assert_eq!(v3d_flush(), 0);
    v3d_surface_swapped(frame.target_gpu + 0x10_0000);
    assert_eq!(draw(&frame), -EINVAL);
}

/// One line per frame whose largest job raised an arena's high-water, and
/// none for a steady frame (v3d-slint sizes Slint's frames against the job's
/// arenas with it). The swap hook runs before the swap's flush, so the
/// frame's last job, still open at the hook, is counted in that frame; a swap
/// with no job open ends the frame at once.
///
/// Revert checks: ending the frame at the hook even with a job open (before
/// the swap's flush has counted it) loses frame 1's line; logging every
/// frame gives more than the two lines after frame 1.
#[test]
fn a_frame_that_raises_an_arena_high_water_logs_one_line() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    let frame = |n: usize, d: &mut GlesDraw| {
        for i in 0..n {
            d.do_clear = i == 0;
            assert_eq!(draw(d), 0);
        }
        v3d_surface_swapped(d.target_gpu);
        assert_eq!(v3d_flush(), 0);
    };
    let lines = |rig: &Rig| -> Vec<String> {
        rig.gl.logs().into_iter().filter(|l| l.contains("job arenas high-water")).collect()
    };
    frame(2, &mut d);
    let l = lines(&rig);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].contains("at frame 1:") && l[0].contains(", 2 draws"), "{l:?}");
    let (bcl, _) = arena_sizes(DEFAULT_DRAWS_PER_JOB).unwrap();
    assert!(l[0].contains("BCL ") && l[0].contains(&format!("of {bcl}, RCL")), "{l:?}");
    frame(1, &mut d); /* steady: nothing */
    frame(3, &mut d); /* frame 3: more draws, more BCL */
    /* frame 4: a swap with no job open, still counted */
    v3d_surface_swapped(d.target_gpu);
    assert_eq!(v3d_flush(), 0);
    frame(3, &mut d); /* steady */
    frame(4, &mut d); /* frame 6 */
    let l = lines(&rig);
    assert_eq!(l.len(), 2, "{l:?}");
    assert!(l[0].contains("at frame 3:") && l[0].contains(", 3 draws"), "{l:?}");
    assert!(l[1].contains("at frame 6:") && l[1].contains(", 4 draws"), "{l:?}");
}

/// The draw-time sampler variants (each a compile and a whole-program link)
/// print with the arena lines when their count moved since the last line:
/// none for a frame that built none, one for a frame that did, so a program
/// that re-links every frame prints every frame.
///
/// Revert checks: printing only with an arena high-water change loses the
/// line of frame 3 (its arenas are steady); printing every frame gives a
/// line for frames 2 and 4.
#[test]
fn a_frame_that_builds_sampler_variants_logs_the_count() {
    use crate::shaderc::online::backend::{v3d_sampler_variants, v3d_sampler_variants_add};
    let rig = Rig::new(); /* it holds the compiler's test lock: the counts are ours */
    rig.init();
    let mut d = rig.draw();
    let frame = |d: &mut GlesDraw| {
        d.do_clear = true;
        assert_eq!(draw(d), 0);
        v3d_surface_swapped(d.target_gpu);
        assert_eq!(v3d_flush(), 0);
    };
    /* the lines since the last call (`logs` takes them) */
    let lines = |rig: &Rig| -> Vec<String> {
        rig.gl.logs().into_iter().filter(|l| l.contains("sampler variants high-water")).collect()
    };
    frame(&mut d); /* frame 1: the count since boot, whatever other tests left */
    let l = lines(&rig);
    assert!(l.len() <= 1, "{l:?}");
    frame(&mut d); /* frame 2: none built */
    assert_eq!(lines(&rig), Vec::<String>::new());
    let (b, r) = v3d_sampler_variants();
    v3d_sampler_variants_add(2, 1);
    frame(&mut d); /* frame 3: two built, one refused */
    let l = lines(&rig);
    let want = format!("at frame 3: {} built, {} refused", b + 2, r + 1);
    assert!(l.len() == 1 && l[0].contains(&want), "{l:?} is not one line with {want:?}");
    frame(&mut d); /* frame 4: steady again */
    assert_eq!(lines(&rig), Vec::<String>::new());
}

/* ---- tile at upload (texture_uploaded) ----------------------------------- */

fn eager_tilings() -> u32 {
    g().eager_tilings
}

/// Texture `t`'s level 0 as a caller would pass it: the storage's rows, `pad`
/// bytes of garbage after each (so the stride is not the width).
fn caller_rows(t: &GlesTextureBind, pad: usize) -> (Vec<u8>, u32) {
    let (w, h) = (t.width as usize, t.height as usize);
    let stride = w * 4 + pad;
    let src = cpu_of(t.base_gpu, texture::gl_bytes(t)).expect("the rig's storage");
    let mut rows = vec![0xa5u8; stride * h];
    for y in 0..h {
        // SAFETY: row y of level 0 is inside the storage (gl_bytes).
        let row = unsafe { core::slice::from_raw_parts(src.add(y * t.stride_texels as usize * 4), w * 4) };
        rows[y * stride..y * stride + w * 4].copy_from_slice(row);
    }
    (rows, stride as u32)
}

/// An upload as the GL core reports it: through the V3D table's entry (the
/// host-test build links no `g_backend`), so a missing entry shows.
fn upload(t: &GlesTextureBind, level: u32, rect: [u32; 4], rows: &[u8], stride: u32) {
    let f = yakogl_backend_v3d.texture_uploaded.expect("the V3D table has texture_uploaded");
    let src = rows.as_ptr() as *const c_void;
    // SAFETY: `t` is live; `rows` holds rect[3] rows of `stride` bytes.
    unsafe { f(t, level, rect[0], rect[1], rect[2], rect[3], src, stride) };
}

/// The image of the live copy made from `base`, `size` bytes.
fn copy_image(base: u64, size: usize) -> Vec<u8> {
    let t = g().tiled.iter().find(|t| t.live() && t.key.base_gpu == base).expect("a live copy");
    let pad = (t.image_gpu as u64 - t.mem.gpu) as usize;
    // SAFETY: the copy's image is `size` bytes at `pad` into its memory.
    unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(pad), size) }.to_vec()
}

/// A whole level-0 upload makes the tiled copy at once, from the caller's
/// rows at their own stride, under the key the next draw looks for: the
/// draw tiles nothing (v3d-tex's test a, and b: a whole SubImage after a NULL
/// TexImage is this call with the new stamp). The copy is byte for byte the
/// one the draw would have tiled from the storage.
///
/// Revert checks, each failing a test here: the table entry None; the
/// stride replaced by w*4; the copy keyed on another stamp; and, in the next
/// test, each of the whole-box, levels and bgra conditions removed.
#[test]
fn a_whole_upload_is_tiled_once_at_the_upload_and_the_draw_tiles_nothing() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 12, 9, 1);
    t.content_gen = 11;
    let (rows, stride) = caller_rows(&t, 20);
    upload(&t, 0, [0, 0, 12, 9], &rows, stride);
    assert_eq!((tilings(), eager_tilings(), tiled().0), (1, 1, 1), "tiled at the upload");
    let size = texture::layout(&t).unwrap().size as usize;
    let eager = copy_image(t.base_gpu, size);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tilings(), 1, "the draw found the copy: nothing tiled");
    /* the same texels tiled at a draw, from the storage */
    v3d_texture_released(t.base_gpu, texture::gl_bytes(&t) as u64);
    t.content_gen = 12;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!((tilings(), eager_tilings()), (2, 1));
    assert!(eager == copy_image(t.base_gpu, size), "the upload's copy is the draw's, byte for byte");
}

/* ---- the TFU for large kept copies (OSAL_QUERY_V3D_TFU_*) ---------------- */

fn tfu_counters() -> (u32, u32) {
    let (mut j, mut f) = (u32::MAX, u32::MAX);
    // SAFETY: two writable counters.
    unsafe { v3d_tfu_counters(&mut j, &mut f) };
    (j, f)
}

/// A rig whose OSAL runs TFU jobs for kept copies of `min` bytes or more
/// (`None`: no answer, the product default) and gives each job `rc`.
fn tfu_rig(min: Option<u32>, rc: i32) -> Rig {
    let rig = Rig::new();
    with_platform(|p| (p.tfu_min, p.tfu_rc) = (min, rc));
    rig.init();
    rig
}

/// What the CPU tiles from texture `t`'s storage (the reference image).
fn cpu_tiled(t: &GlesTextureBind) -> Vec<u8> {
    let layout = texture::layout(t).unwrap();
    let src = cpu_of(t.base_gpu, texture::gl_bytes(t)).expect("the rig's storage");
    let mut want = vec![0u8; layout.size as usize];
    // SAFETY: `want` is the layout's size; the storage covers level 0.
    unsafe { texture::tile(t, &layout, want.as_mut_ptr(), src) };
    want
}

/// With the OSAL's TFU on, a kept copy made at a draw is one TFU job from
/// the GL core's storage: its base, its rows' pitch (14 texels for a width
/// of 13: the GL core's pairs), the texture's size; the copy is what the
/// CPU tiles. Revert checks: the stride as width * 4 shears the copy; the
/// kept copy's GPU address not passed (the fill's second argument) puts
/// the job elsewhere.
#[test]
fn a_large_kept_copy_is_one_tfu_job_from_the_storage() {
    let rig = tfu_rig(Some(1), 0);
    let mut t = texture(0, 13, 9, 1);
    t.content_gen = 5;
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tfu_counters(), (1, 0));
    let runs = with_platform(|p| p.tfu_runs.clone());
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].iia as u64, runs[0].iis, runs[0].ios), (t.base_gpu, 14, 9 << 16 | 13));
    assert_eq!(runs[0].icfg >> 9, V3D_TFU_TTYPE_R32F, "RGBA8: a 4-byte copy");
    let size = texture::layout(&t).unwrap().size as usize;
    let kept = g().tiled.iter().find(|c| c.live() && c.key.base_gpu == t.base_gpu).map(|c| c.image_gpu).unwrap();
    assert_eq!(runs[0].ioa & !0x3f, kept, "into the kept copy");
    assert!(copy_image(t.base_gpu, size) == cpu_tiled(&t), "the TFU's copy is the CPU's");
}

/// Below the threshold, and without the OSAL's answer (the default), the
/// CPU tiles and no TFU job is asked for.
#[test]
fn the_cpu_tiles_below_the_threshold_and_without_the_osals_tfu() {
    for (min, what) in [(Some(1 << 20), "below the threshold"), (None, "no TFU")] {
        let rig = tfu_rig(min, 0);
        let mut t = texture(0, 13, 9, 1);
        t.content_gen = 5;
        assert_eq!(draw(&sampling(&rig, t)), 0);
        assert_eq!(v3d_flush(), 0);
        assert_eq!((tfu_counters(), with_platform(|p| p.tfu_runs.len())), ((0, 0), 0), "{what}");
        assert_eq!(tilings(), 1, "{what}: tiled on the CPU");
        let size = texture::layout(&t).unwrap().size as usize;
        assert!(copy_image(t.base_gpu, size) == cpu_tiled(&t), "{what}");
    }
}

/// A TFU job that fails is no lost texture: the CPU tiles the copy, the
/// failure is counted and logged once.
#[test]
fn a_failed_tfu_job_is_tiled_on_the_cpu() {
    let rig = tfu_rig(Some(1), -5);
    for (gen, base) in [(5, 0), (6, 1)] {
        let mut t = texture(0, 13, 9, 1);
        t.content_gen = gen;
        assert_eq!(draw(&sampling(&rig, t)), 0);
        assert_eq!(v3d_flush(), 0);
        let size = texture::layout(&t).unwrap().size as usize;
        assert!(copy_image(t.base_gpu, size) == cpu_tiled(&t), "the CPU's copy");
        assert_eq!(tfu_counters(), (0, base + 1));
    }
    assert_eq!(rig.logs_with("TFU job for a tiled copy"), 1, "logged once");
}

/// A whole level-0 upload of a large texture: the kept copy is one TFU job
/// from the GL core's storage (which holds the texels now), not the CPU
/// from the caller's rows; the copy is the same bytes.
#[test]
fn a_large_upload_is_tiled_by_the_tfu_from_the_storage() {
    let rig = tfu_rig(Some(1), 0);
    let mut t = texture(0, 13, 9, 1);
    t.content_gen = 11;
    let (rows, stride) = caller_rows(&t, 20);
    upload(&t, 0, [0, 0, 13, 9], &rows, stride);
    assert_eq!((tilings(), eager_tilings(), tfu_counters()), (1, 1, (1, 0)));
    let runs = with_platform(|p| p.tfu_runs.clone());
    assert_eq!((runs[0].iia as u64, runs[0].iis), (t.base_gpu, 14), "from the storage, not the caller's rows");
    let size = texture::layout(&t).unwrap().size as usize;
    assert!(copy_image(t.base_gpu, size) == cpu_tiled(&t));
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(tilings(), 1, "the draw found the copy");
}

/// An upload whose tiled copy the OSAL refuses (the carve-out full) is no
/// error: the upload makes nothing, the texture stays in the GL core's
/// linear storage, and the draw tiles it into its job's arena; with memory
/// back, a later job keeps a copy again. (DEMO12's GL_OUT_OF_MEMORY was the
/// GL core's own storage allocation, src/gl/texture.rs:990-1005, which
/// failed because the carve-out was full: not this path.)
#[test]
fn an_upload_whose_tiled_copy_is_refused_leaves_the_draw_correct() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 12, 9, 1);
    t.content_gen = 11;
    let (rows, stride) = caller_rows(&t, 20);
    let refused0 = v3d_refused_draws();
    with_platform(|p| p.fail_alloc_from = Some(1));
    upload(&t, 0, [0, 0, 12, 9], &rows, stride);
    assert_eq!((tilings(), eager_tilings(), tiled().0), (0, 0, 0), "nothing made at the upload");
    assert_eq!(rig.logs_with("no GPU memory for a tiled copy"), 1);
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    assert_eq!((tilings(), tiled().0), (1, 0), "tiled into the job's arena, no copy kept");
    assert_eq!(v3d_flush(), 0);
    with_platform(|p| p.fail_alloc_from = None);
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!((tilings(), tiled().0), (2, 1), "memory back: kept again");
    assert_eq!(v3d_flush(), 0);
    assert_eq!(v3d_refused_draws(), refused0, "no draw refused");
}

/// Everything but a whole level-0 upload of a one-level, stamped, unswapped
/// texture the sampler takes makes nothing at the upload; its new stamp makes
/// the next draw tile the storage, as before (tests c, d, e and f).
#[test]
fn only_a_whole_level_0_upload_of_a_one_level_texture_is_tiled_at_the_upload() {
    let rig = Rig::new();
    rig.init();
    let base = texture(0, 16, 16, 1);
    let (rows, stride) = caller_rows(&base, 0);
    let mut cases: Vec<(&str, GlesTextureBind, u32, [u32; 4], u32)> = Vec::new(); /* (what, binding, level, box, stride) */
    let with = |f: fn(&mut GlesTextureBind)| {
        let mut t = base;
        t.content_gen = 21;
        f(&mut t);
        t
    };
    cases.push(("a sub-box", with(|_| {}), 0, [0, 0, 8, 16], stride));
    cases.push(("an offset box", with(|_| {}), 0, [1, 0, 15, 16], stride));
    cases.push(("level 1", with(|_| {}), 1, [0, 0, 16, 16], stride));
    cases.push((
        "sampled with its mip chain",
        with(|t| (t.mip_filter, t.mip_levels) = (1, 2)),
        0,
        [0, 0, 16, 16],
        stride,
    ));
    cases.push(("an XRGB image (f)", with(|t| t.bgra = 1), 0, [0, 0, 16, 16], stride));
    cases.push(("unstamped (an EGLImage)", with(|t| t.content_gen = 0), 0, [0, 0, 16, 16], stride));
    cases.push(("a refused format", with(|t| t.format = GLES_TEXFMT_RGB565), 0, [0, 0, 16, 16], stride));
    cases.push(("a stride shorter than a row", with(|_| {}), 0, [0, 0, 16, 16], 16 * 4 - 4));
    let frees = with_platform(|p| p.frees.len());
    for (what, t, level, rect, stride) in &cases {
        upload(t, *level, *rect, &rows, *stride);
        assert_eq!((tilings(), eager_tilings(), tiled().0), (0, 0, 0), "{what}: nothing at the upload");
    }
    assert_eq!(with_platform(|p| p.frees.len()), frees + 1, "the refused stride's copy was freed, nothing else made");
    /* the positive control, then the lazy path after a sub-box */
    let mut t = with(|_| {});
    upload(&t, 0, [0, 0, 16, 16], &rows, stride);
    assert_eq!((tilings(), eager_tilings(), tiled().0), (1, 1, 1), "the whole level: tiled");
    t.content_gen = 22;
    upload(&t, 0, [0, 0, 8, 8], &rows, stride);
    assert_eq!(tilings(), 1, "a sub-box of the next stamp tiles nothing");
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!((tilings(), eager_tilings(), tiled().0), (2, 1, 1), "the draw tiled the new stamp, the old copy went");
    /* a copy made for one level is not found when the chain is sampled later
     * (rule 4): the key carries the levels sampled */
    let mut m = texture(0, 16, 16, 2);
    (m.mip_filter, m.content_gen) = (0, 31);
    let (mrows, mstride) = caller_rows(&m, 0);
    upload(&m, 0, [0, 0, 16, 16], &mrows, mstride);
    assert_eq!((tilings(), eager_tilings()), (3, 2), "one level sampled: tiled at the upload");
    m.mip_filter = 1;
    assert_eq!(draw(&sampling(&rig, m)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!((tilings(), eager_tilings()), (4, 2), "the chain sampled: tiled again at the draw");
}

/// The tiling time since init: (ns by where, untimed).
fn tiling_time() -> ([u64; 3], u32) {
    let (mut ns, mut untimed) = ([u64::MAX; 3], u32::MAX);
    // SAFETY: three u64 and one u32, writable.
    unsafe { v3d_tiling_time(ns.as_mut_ptr(), &mut untimed) };
    (ns, untimed)
}

/// Each tiling's CPU time is counted under where it ran, from the OSAL's
/// clock (a mock clock that advances 1000 ns per read, so one timed tiling
/// is exactly 1000): the upload's kept copy under [0], a draw's kept copy
/// under [1], a draw's arena copy (an unstamped texture) under [2]. Each
/// bucket is reached once, by the path the tiling counters say ran.
///
/// Revert checks: swapping two `TILE_*` arguments, or dropping a
/// `tile_timed`, fails an assertion here.
#[test]
fn each_tiling_is_timed_under_where_it_ran() {
    let rig = Rig::new();
    with_platform(|p| p.clock = Some((5_000, 1_000)));
    rig.init();
    assert_eq!(tiling_time(), ([0; 3], 0), "nothing tiled yet");
    let mut t = texture(0, 12, 9, 1);
    t.content_gen = 11;
    let (rows, stride) = caller_rows(&t, 20);
    upload(&t, 0, [0, 0, 12, 9], &rows, stride);
    assert_eq!((tilings(), eager_tilings()), (1, 1));
    assert_eq!(tiling_time(), ([1_000, 0, 0], 0), "the upload's copy");
    t.content_gen = 12;
    assert_eq!(draw(&sampling(&rig, t)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!((tilings(), eager_tilings(), tiled().0), (2, 1, 1));
    assert_eq!(tiling_time(), ([1_000, 1_000, 0], 0), "the draw's kept copy");
    let mut u = texture(0, 8, 8, 1);
    u.content_gen = 0; /* unstamped: no kept copy, the job's arena */
    assert_eq!(draw(&sampling(&rig, u)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!((tilings(), tiled().0), (3, 1), "tiled into the arena, not kept");
    assert_eq!(tiling_time(), ([1_000, 1_000, 1_000], 0), "the draw's arena copy");
}

/// Without the OSAL's clock a tiling has no time: it is counted as untimed,
/// one per tiling, and no bucket moves (absent, not zero time).
#[test]
fn a_tiling_without_the_clock_is_untimed_not_zero() {
    let rig = Rig::new();
    rig.init();
    let mut t = texture(0, 12, 9, 1);
    t.content_gen = 11;
    let (rows, stride) = caller_rows(&t, 20);
    upload(&t, 0, [0, 0, 12, 9], &rows, stride);
    let mut u = texture(0, 8, 8, 1);
    u.content_gen = 0;
    assert_eq!(draw(&sampling(&rig, u)), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tilings(), 2);
    assert_eq!(tiling_time(), ([0; 3], 2), "two tilings, neither timed");
}

/// The record's Point size in shaded vertex data follows the program: set
/// exactly when the linked VS writes the point size into its VPM output
/// (`writes_psiz`), as Mesa's is for its points draws (v3dx_draw.c:556-558).
/// Today's VS never writes it, so every draw carries false; a VS that does
/// carries true. Revert check: the backend passing false fails it.
#[test]
fn the_records_point_size_bit_is_the_programs_writes_psiz() {
    let rig = Rig::new();
    rig.init();
    let d = rig.draw();
    let before = v3d_psiz_draws();
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_psiz_draws(), before, "a VS without the word is not counted");
    let mut psiz = program();
    psiz.v3d.vs.writes_psiz = 1;
    let mut e = d;
    e.do_clear = false;
    e.fs = &psiz;
    assert_eq!(draw(&e), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let bits: Vec<_> = rig.draws().iter().map(|d| d.vs_writes_psiz).collect();
    assert_eq!(bits, [false, true]);
    assert_eq!(v3d_psiz_draws(), before + 1, "the draw with the word is counted");
}

/// A points draw is drawn (the board measured the record's point-size bit as
/// a layout description for every primitive, gl_test 110 on 37264ce2, so
/// the triangles program serves points too): recorded as V3D_PRIM_POINTS,
/// with the program's point-size bit, no refusal. A VS without gl_PointSize
/// leaves the bit off and the POINT_SIZE packet's 1.0 applies.
/// Revert check: with the state refusal back, nothing is recorded.
#[test]
fn a_points_draw_is_recorded_with_the_programs_point_size_bit() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.prim = GLES_PRIM_POINTS;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let mut psiz = program();
    psiz.v3d.vs.writes_psiz = 1;
    let mut e = d;
    e.do_clear = false;
    e.fs = &psiz;
    assert_eq!(draw(&e), 0);
    assert_eq!(v3d_flush(), 0);
    let seen: Vec<_> = rig.draws().iter().map(|d| (d.prim, d.vs_writes_psiz)).collect();
    assert_eq!(seen, [(V3D_PRIM_POINTS, false), (V3D_PRIM_POINTS, true)]);
    assert_eq!(rig.logs_with("points"), 0, "no refusal names points");
}

/// A unit a stage samples that no binding names (texture object 0 bound:
/// the GL core leaves it out of the draw) samples the incomplete texture's
/// stand-in, (0, 0, 0, 1), GL ES 2.0 3.7.10: gl_test 61's "nothing bound"
/// draw, which the job builder refused on the board ("a uniform ... it does
/// not write", diag record 27f7382c). A bound unit keeps its own records.
/// Revert check: without the stream scan the tables lack unit 2 and the VS's
/// unit 0.
#[test]
fn a_sampled_unit_with_nothing_bound_samples_the_black_stand_in() {
    static FS_TEX: [ShcV3dUniform; 4] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 2 << 24 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 2 << 24 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0 },
    ];
    static VS_TEX: [ShcV3dUniform; 2] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0 },
    ];
    let rig = Rig::new();
    rig.init();
    let mut p = program();
    (p.v3d.fs.uniforms, p.v3d.fs.num_uniforms) = (FS_TEX.as_ptr(), 4);
    (p.v3d.vs.uniforms, p.v3d.vs.num_uniforms) = (VS_TEX.as_ptr(), 2);
    with_code(&mut p);
    let mut t = texture(0, 8, 8, 1);
    t.content_gen = 41;
    let mut d = sampling(&rig, t); /* unit 0 bound, unit 2 not */
    d.do_clear = true;
    d.fs = &p;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let black = g().black.gpu as u32 + BLACK_STATE_OFF;
    let seen = &rig.draws()[0];
    let fs: Vec<_> = seen.tmu.iter().map(|t| (t.unit, t.tex_state == black)).collect();
    assert_eq!(fs, [(0, false), (2, true)], "unit 0 its texture, unit 2 the stand-in");
    let vs: Vec<_> = seen.vs_tmu.iter().map(|t| (t.unit, t.tex_state == black)).collect();
    assert_eq!(vs, [(0, true)], "the VS's unbound unit 0: the stand-in");
    assert!(seen.tmu[1].sampler != 0 && seen.tmu[1].sampler.is_multiple_of(V3D_TEX_RECORD_ALIGN));
}

/// An XRGB8888 target (gl_test 18's EGLImage-era pbuffer, refused on the
/// board: "a colour target other than RGBA8888", diag record 27f7382c) is
/// drawn: its job loads and stores with R and B swapped (Mesa's r_b_swap for
/// B8G8R8X8, v3dx_rcl.c:98/:150), loads alpha as 1 and writes none. An
/// RGBA8888 target's job swaps nothing, and the two never share a job.
/// Revert check: without the Target's bgr, r_b_swap stays false.
#[test]
fn an_xrgb_target_is_loaded_and_stored_with_red_and_blue_swapped() {
    let rig = Rig::new();
    rig.init();
    let mut x = rig.draw();
    (x.target_format, x.target_alpha_bits, x.do_clear) = (GLES_SURF_XRGB8888, 0, false);
    assert_eq!(draw(&x), 0, "{:?}", rig.gl.logs());
    let mut r = rig.draw();
    (r.target_gpu, r.do_clear) = (TARGET + 0x10_0000, false);
    assert_eq!(draw(&r), 0);
    assert_eq!(v3d_flush(), 0);
    let c: Vec<_> = rig.jobs().iter().map(|j| j.clear.unwrap()).map(|c| (c.r_b_swap, c.force_alpha_1)).collect();
    assert_eq!(c, [(true, true), (false, false)]);
    assert_eq!(rig.draws()[0].st.colormask, 0x7);
}

/// An RGB565 target (gl_test 18's pbuffer: eglChooseConfig with only
/// EGL_SURFACE_TYPE sorts the 16-bit config first, EGL 1.5 3.4.1 rule 4,
/// BUFFER_SIZE smaller first, since no colour size was requested) is drawn:
/// its job's target is 16-bit (bgr565, pitch x 2), loaded with alpha 1 and
/// no R/B swap, alpha not written. Revert check: the Target's rgb565 false
/// gives a 4-byte pitch.
#[test]
fn an_rgb565_target_is_drawn_two_bytes_a_pixel() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    (d.target_format, d.target_alpha_bits, d.do_clear) = (GLES_SURF_RGB565, 0, false);
    (d.target_w, d.target_h) = (13, 5);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let j = &rig.jobs()[0];
    assert!(j.target.rgb565);
    assert_eq!(j.target.stride, 14 * 2, "the GL core's pair-padded pitch, 2 bytes a pixel");
    let c = j.clear.unwrap();
    assert_eq!((c.force_alpha_1, c.r_b_swap), (true, false));
    assert_eq!(rig.draws()[0].st.colormask, 0x7);
}

/* ---- ES 3.0 colour formats (int-rt-v3d) ----------------------------------- */

/// An integer colour target (RGBA8I) is a typed job: RT 0 loaded and stored
/// as output image format RGBA8I at 4 bytes a pixel, and its
/// glClearBufferiv (the raw integers in `clear_rgba`) reaches the job as
/// v3d_tlb_clear's one 8I word, a byte a channel; the untyped RGBA8 word
/// is the same bits, never read. Revert check: with `typed` false the job
/// builder would load RGBA8 and clear with pack_rgba8 of the raw bits.
#[test]
fn an_integer_target_is_a_typed_job_cleared_with_its_own_words() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    d.target_format = GLES_SURF_RGBA8I;
    d.clear_rgba = [1u32, -2i32 as u32, 3, 0x1f4].map(f32::from_bits);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let j = &rig.jobs()[0];
    assert!(j.target.typed);
    assert_eq!(j.target.rt_format[0], V3D_OUTPUT_IMAGE_FORMAT_RGBA8I);
    assert_eq!(j.target.stride, 64 * 4, "4 bytes a pixel");
    let c = j.clear.unwrap();
    assert!(!c.load_rt0, "cleared, not loaded");
    assert_eq!(c.clear_rt[0], [0xf403_fe01, 0, 0, 0], "a byte a channel, the raw integers narrowed");
    assert_eq!(c.rgba8, 0xf403_fe01);
    assert_eq!(rig.kicks()[0].tile_state_size, 256, "one 64x64 tile: 32 bpp");
}

/// An sRGB target is a typed job whose RT is the TLB's SRGB8_ALPHA8 image
/// format at 4 bytes a pixel over a 16F, 64-bpp tile buffer (so 64x32 tiles
/// again), and whose clear is the linear colour clamped to [0, 1], as halves
/// (docs/es3/srgb.md 5.1).
#[test]
fn an_srgb_target_is_a_typed_16f_job_clearing_linear_halves() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    (d.target_format, d.target_alpha_bits) = (GLES_SURF_SRGB8_A8, 8);
    d.clear_rgba = [0.5, 2.0, -1.0, 1.0];
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let j = &rig.jobs()[0];
    assert!(j.target.typed);
    assert_eq!(j.target.rt_format[0], V3D_OUTPUT_IMAGE_FORMAT_SRGB8_ALPHA8);
    assert_eq!(j.target.stride, 64 * 4, "4 bytes a pixel");
    assert_eq!(j.clear.unwrap().clear_rt[0], [0x3c00_3800, 0x3c00_0000, 0, 0], "linear halves, clamped");
    assert_eq!(rig.kicks()[0].tile_state_size, 2 * 256, "64x32 tiles at 64 bpp");
}

/// A half-float target (RGBA16F) is a typed job at 8 bytes a pixel whose
/// clear is four halves in two words (internal type 16F, 64 bpp), and whose
/// 64-bpp tile buffer bins 64x32 tiles: twice the tile state of an RGBA8
/// job of the same size. Revert check: without `max_bpp` in the tiling the
/// kick's tile state stays one tile's.
#[test]
fn a_half_float_target_is_a_64_bpp_job() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    (d.target_format, d.target_alpha_bits) = (GLES_SURF_RGBA16F, 16);
    d.clear_rgba = [1.0, 0.5, -2.0, 0.0];
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let j = &rig.jobs()[0];
    assert!(j.target.typed);
    assert_eq!(j.target.rt_format[0], V3D_OUTPUT_IMAGE_FORMAT_RGBA16F);
    assert_eq!(j.target.stride, 64 * 8, "8 bytes a pixel");
    assert_eq!(j.clear.unwrap().clear_rt[0], [0x3800_3c00, 0x0000_c000, 0, 0], "halves, two a word");
    assert_eq!(rig.kicks()[0].tile_state_size, 2 * 256, "64x32 tiles at 64 bpp");
}

/// Attachments of different formats in one job: RT 0 RGBA8 and RT 1
/// RGBA32UI make a typed job whose RTs carry their own image formats and
/// strides (16 bytes a pixel for RT 1), whose glClear words are each RT's
/// own (one RGBA8 word, four raw words), and whose 128-bpp tile buffer with
/// two RTs bins 32x16 tiles (8 of them for 64x64).
#[test]
fn attachments_of_different_formats_each_keep_their_own() {
    let rig = Rig::new();
    rig.init();
    let p = program_writing(0b11);
    let mut d = mrt_draw(&rig, &p, 2);
    d.targets[1].format = GLES_SURF_RGBA32UI;
    d.clear_rgba = [0x1234_5678u32, 1, 2, 0xffff_ffff].map(f32::from_bits);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let j = &rig.jobs()[0];
    assert!(j.target.typed);
    assert_eq!(j.target.rt_format, [27, V3D_OUTPUT_IMAGE_FORMAT_RGBA32UI, 27, 27]);
    assert_eq!(j.target.stride, 64 * 4);
    assert_eq!(j.target.rt[0], V3dJobRt { address: RT1 as u32, stride: 64 * 16 });
    let c = j.clear.unwrap();
    assert!(!c.load_rt0 && !c.load_rt[0]);
    assert_eq!(c.clear_rt[0], [state::pack_rgba8(d.clear_rgba), 0, 0, 0], "RT 0: the RGBA8 word");
    assert_eq!(c.clear_rt[1], [0x1234_5678, 1, 2, 0xffff_ffff], "RT 1: the four raw words");
    assert_eq!(rig.kicks()[0].tile_state_size, 8 * 256, "32x16 tiles: two RTs at 128 bpp");
}

/// glClearBuffer(GL_COLOR, 1) (`clear_target_mask` bit 1) clears draw
/// buffer 1 alone: RT 0 loads what it holds, RT 1 starts from the colour.
/// Revert check: without the mask every RT was cleared.
#[test]
fn a_clear_of_one_draw_buffer_loads_the_others() {
    let rig = Rig::new();
    rig.init();
    let p = program_writing(0b11);
    let mut d = mrt_draw(&rig, &p, 2);
    d.clear_target_mask = 0b10;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let c = rig.jobs()[0].clear.unwrap();
    assert!(c.load_rt0, "RT 0 is not named: loaded");
    assert!(!c.load_rt[0], "RT 1 cleared");
    assert_eq!(c.rgba8_rt[0], 0xff00_00ff, "red");
    assert_eq!(c.clear_rt[1][0], 0xff00_00ff);
}

/// What the typed job cannot take is refused by name before anything is
/// recorded: 4x on an ES 3.0 format (the kept samples are RGBA8), a
/// scissored or masked clear of an integer target (the clear quad writes
/// halves), and a 128-bpp job with more tiles than the pools hold.
#[test]
fn what_a_typed_job_cannot_take_is_refused_by_name() {
    let rig = Rig::new();
    rig.init();
    let mut d = rig.draw();
    (d.target_format, d.samples) = (GLES_SURF_RGBA16F, 4);
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("4x multisampling of an ES 3.0 colour format"), 1);
    let mut d = rig.draw();
    (d.target_format, d.clear_quad) = (GLES_SURF_R32I, true);
    assert_eq!(draw(&d), -EINVAL);
    assert_eq!(rig.logs_with("a scissored or masked colour clear of an integer colour target"), 1);
    let mut d = rig.draw();
    (d.target_format, d.clear_color_mask) = (GLES_SURF_R32I, 0b0001);
    assert_eq!(draw(&d), -EINVAL, "a masked clear is the quad's too");
    let p = program_writing(0b11);
    let mut d = mrt_draw(&rig, &p, 2);
    (d.target_w, d.target_h) = (2048, 2048);
    (d.targets[0].w, d.targets[0].h) = (2048, 2048);
    (d.targets[1].w, d.targets[1].h) = (2048, 2048);
    d.targets[1].format = GLES_SURF_RGBA32UI;
    assert_eq!(draw(&d), -EINVAL, "32x16 tiles over 2048x2048: 8192 tiles");
    assert_eq!(rig.logs_with("more tiles than the job pools hold"), 1);
    assert!(rig.draws().is_empty() && rig.jobs().is_empty(), "nothing was recorded");
}

/// Why gl_test 18 draws into an RGB565 target: it asks eglChooseConfig for
/// EGL_SURFACE_TYPE = EGL_PBUFFER_BIT alone and takes the first config. With
/// no colour size requested, EGL 1.5 3.4.1's colour-bits rule compares
/// nothing, and the next rule (EGL_BUFFER_SIZE, smaller first) puts the
/// 16-bit config ahead of the 32-bit ones. Host reproduction of the board's
/// refusal (verify record 613dcddd), for the RGB565 target above.
#[test]
fn a_pbuffer_config_chosen_by_surface_type_alone_is_the_16_bit_one() {
    use crate::gl::egl::{eglChooseConfig, eglGetConfigAttrib, eglGetDisplay, eglInitialize, eglTerminate};
    use crate::gl::egl_consts::*;
    let _rig = Rig::new();
    let attribs = [EGL_SURFACE_TYPE, EGL_PBUFFER_BIT, EGL_NONE];
    // SAFETY: EGL calls with live out-parameters and a terminated list.
    unsafe {
        let dpy = eglGetDisplay(core::ptr::null_mut());
        assert_eq!(eglInitialize(dpy, core::ptr::null_mut(), core::ptr::null_mut()), 1);
        let (mut cfg, mut n) = (core::ptr::null_mut(), 0);
        assert_eq!(eglChooseConfig(dpy, attribs.as_ptr(), &mut cfg, 1, &mut n), 1);
        assert_eq!(n, 1);
        let (mut size, mut red, mut alpha) = (0, 0, 0);
        eglGetConfigAttrib(dpy, cfg, EGL_BUFFER_SIZE, &mut size);
        eglGetConfigAttrib(dpy, cfg, EGL_RED_SIZE, &mut red);
        eglGetConfigAttrib(dpy, cfg, EGL_ALPHA_SIZE, &mut alpha);
        assert_eq!((size, red, alpha), (16, 5, 0), "the first config is RGB565");
        eglTerminate(dpy);
    }
}

/// Byte `k` of texel (x, y) of `level` in the narrow tests: distinct for
/// every texel of a level of an 8x4 chain or a 13x7 level (y * 16 + x <
/// 128), each level's shifted by 40 so that two levels' bytes differ, and
/// the two bytes of an LA8 texel apart (the top bit).
fn narrow_byte(level: u32, x: u32, y: u32, k: u32) -> u8 {
    (((y * 16 + x + level * 40) % 128) | (k & 1) << 7) as u8
}

/// A `w` x `h` texture of the narrow `format` (L8, A8: 1 byte a texel; LA8:
/// 2) with `levels` levels filled, texel bytes [`narrow_byte`], where the GL
/// core keeps them (`texture::gl_level_layered` at the format's size), in
/// memory of the rig's platform.
fn narrow_texture(w: u32, h: u32, levels: u32, format: u8) -> GlesTextureBind {
    let b = GlesTextureBind {
        width: w,
        height: h,
        stride_texels: (w + 1) & !1,
        layers: 1,
        mip_levels: levels as u8,
        mip_filter: if levels > 1 { 1 } else { 0 },
        format,
        ..GlesTextureBind::ZERO
    };
    let bpp = texture::texel_bytes(&b);
    let mut mem = OsalGpuMem::ZERO;
    assert_eq!(crate::osal::gpu_alloc(V3D_HEAP_DATA, texture::gl_bytes(&b) as usize, OSAL_MEM_DATA, &mut mem), 0);
    for level in 0..levels {
        let (off, pitch, rows) = texture::gl_level_layered(w, h, 1, bpp, level);
        for y in 0..rows {
            for x in 0..(w >> level).max(1) {
                for k in 0..bpp {
                    let at = (off + (y * pitch + x) * bpp + k) as usize;
                    // SAFETY: inside the allocation (gl_bytes).
                    unsafe { *(mem.cpu as *mut u8).add(at) = narrow_byte(level, x, y, k) };
                }
            }
        }
    }
    GlesTextureBind { base_gpu: mem.gpu, ..b }
}

/// L8, A8 and LA8 are sampled narrow, as Mesa samples L8_UNORM, A8_UNORM and
/// L8A8_UNORM (v3dx_format_table.c): the record names R8 (L8, A8) or RG8
/// (LA8) with the swizzle XXX1, 000X or XXXY, and the image is tiled from
/// the GL core's rows at the format's own size, every level of a chain
/// (with its 1-wide tail) and an odd-width level at its pair-padded pitch.
///
/// Revert checks: tiling at 4 bytes a texel (the RGBA8 rule) reads past
/// each level's rows and fails the bytes; RGBA8's type or swizzle fails the
/// record.
#[test]
fn a_narrow_texture_is_tiled_at_its_own_texel_size_with_its_tmu_type_and_swizzle() {
    let rig = Rig::new();
    rig.init();
    const R: u8 = V3D_SWIZZLE_RED;
    let cases: [(u8, u32, u32, [u8; 4]); 3] = [
        (GLES_TEXFMT_L8, 1, V3D_TEXTURE_DATA_FORMAT_R8, [R, R, R, V3D_SWIZZLE_ONE]),
        (GLES_TEXFMT_A8, 1, V3D_TEXTURE_DATA_FORMAT_R8, [V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, R]),
        (GLES_TEXFMT_LA8, 2, V3D_TEXTURE_DATA_FORMAT_RG8, [R, R, R, V3D_SWIZZLE_GREEN]),
    ];
    let mut checked = 0u32;
    for (format, bpp, type_, swizzle) in cases {
        for (w, h, levels) in [(8u32, 4u32, 4u32), (13, 7, 1)] {
            let t = narrow_texture(w, h, levels, format);
            assert_eq!(texture::refusal(&t), None);
            assert_eq!(texture::texel_bytes(&t), bpp);
            let mut d = rig.draw();
            d.textures[0] = t;
            d.num_textures = 1;
            assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
            let u = rig.draws().last().unwrap().tmu[0];
            let st = words(&rig, u.tex_state, 6);
            let what = std::format!("format {format} {w}x{h} x{levels}");
            assert_eq!(st[3], (levels - 1) | type_ << FAKE_TYPE_SHIFT, "{what}: max level and type");
            assert_eq!(st[4], u32::from_le_bytes(swizzle), "{what}: swizzle");
            /* the fake layout at the format's cpp: the levels back to back,
             * smallest first, level 0 (last) at st[0] */
            let size = |i: u32| (w >> i).max(1) * (h >> i).max(1) * bpp;
            let image = st[0] - (1..levels).map(size).sum::<u32>();
            for level in 0..levels {
                let base = image + (level + 1..levels).map(size).sum::<u32>();
                let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
                let bytes = rig.gpu_bytes(base, (lw * lh * bpp) as usize);
                for y in 0..lh {
                    for x in 0..lw {
                        for k in 0..bpp {
                            let at = ((y * lw + x) * bpp + k) as usize;
                            assert_eq!(
                                bytes[at],
                                narrow_byte(level, x, y, k),
                                "{what}: level {level} ({x}, {y}) byte {k}"
                            );
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    /* L8 and A8: (32 + 8 + 2 + 1 + 91) bytes each; LA8 twice that */
    assert_eq!(checked, 134 * 4, "every texel byte of every level was compared");
}

/// The GL core's narrow store on this build (`narrow_la_texels` is true for
/// the V3D) is where the backend reads it: GL_LUMINANCE, GL_ALPHA and
/// GL_LUMINANCE_ALPHA uploads are kept as L8, A8 and LA8, every level of a
/// chain and an odd-width level at the GL core's own `level_off` and
/// `level_pitch`, and each uploaded texel's bytes are at
/// `texture::gl_level_layered`'s place for the format's size; the binding a
/// draw gives the texture is one the backend samples (not refused). A
/// cube of LA8 checks the layers' stride at 2 bytes a texel.
#[test]
fn the_gl_cores_narrow_store_is_the_one_the_backend_reads() {
    use crate::gl::consts::*;
    use crate::gl::texture::{glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D};
    let rig = Rig::new();
    /* the GL core reads this backend's layout flags, as in the product;
     * TestGl's drop resets the table */
    // SAFETY: serialised by the rig's TestGl.
    unsafe { crate::backend::g_backend = &yakogl_backend_v3d };
    // SAFETY: plain GL call.
    unsafe { crate::gl::state::glPixelStorei(GL_UNPACK_ALIGNMENT, 1) };
    let fmts =
        [(GL_LUMINANCE, GLES_TEXFMT_L8, 1u32), (GL_ALPHA, GLES_TEXFMT_A8, 1), (GL_LUMINANCE_ALPHA, GLES_TEXFMT_LA8, 2)];
    let mut compared = 0u32;
    for (format, texfmt, bpp) in fmts {
        for (w, h, levels) in [(8u32, 4u32, 4u32), (13, 7, 1)] {
            let mut name = 0;
            // SAFETY: plain GL calls on the rig's context with live pointers:
            // each level's texels packed (unpack alignment 1).
            unsafe {
                glGenTextures(1, &mut name);
                glBindTexture(GL_TEXTURE_2D, name);
                for level in 0..levels {
                    let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
                    let mut texels = std::vec::Vec::new();
                    for y in 0..lh {
                        for x in 0..lw {
                            for k in 0..bpp {
                                texels.push(narrow_byte(level, x, y, k));
                            }
                        }
                    }
                    glTexImage2D(
                        GL_TEXTURE_2D,
                        level as i32,
                        format as i32,
                        lw as i32,
                        lh as i32,
                        0,
                        format,
                        GL_UNSIGNED_BYTE,
                        texels.as_ptr() as *const c_void,
                    );
                }
            }
            assert_eq!(rig.gl.error(), GL_NO_ERROR);
            let t = &rig.gl.ctx().objs().textures[name as usize];
            let what = std::format!("0x{format:x} {w}x{h} x{levels}");
            assert_eq!(t.texfmt, texfmt, "{what}: kept narrow");
            let b = crate::gl::draw::texture_bind_2d(t);
            assert_eq!((b.format, texture::refusal(&b)), (texfmt, None), "{what}: sampled");
            for level in 0..levels {
                let (lw, lh) = ((w >> level).max(1), (h >> level).max(1));
                let (off, pitch, _) = texture::gl_level_layered(w, h, 1, bpp, level);
                assert_eq!(off, t.level_off[level as usize], "{what} level {level}: offset");
                assert_eq!(pitch, t.level_pitch(lw), "{what} level {level}: pitch");
                for y in 0..lh {
                    for x in 0..lw {
                        for k in 0..bpp {
                            let at = (off + (y * pitch + x) * bpp + k) as usize;
                            // SAFETY: inside the storage (the offsets above).
                            let got = unsafe { *(t.mem.cpu as *const u8).add(at) };
                            assert_eq!(got, narrow_byte(level, x, y, k), "{what} level {level} ({x}, {y}) byte {k}");
                            compared += 1;
                        }
                    }
                }
            }
            // SAFETY: a live name.
            unsafe { glDeleteTextures(1, &name) };
        }
    }
    assert_eq!(compared, 134 * 4, "every uploaded byte was found");

    /* an LA8 cube: face f of level L at gl_level_layered's place, 2 bytes a texel */
    let (size, levels) = (8u32, 4u32);
    let mut name = 0;
    // SAFETY: as above.
    unsafe {
        glGenTextures(1, &mut name);
        glBindTexture(GL_TEXTURE_CUBE_MAP, name);
        for level in 0..levels {
            let l = (size >> level).max(1);
            for face in 0..6u32 {
                let mut texels = std::vec![0u8; (l * l * 2) as usize];
                texels[..2].copy_from_slice(&[face as u8 + 1, level as u8 | 0x80]);
                glTexImage2D(
                    GL_TEXTURE_CUBE_MAP_POSITIVE_X + face,
                    level as i32,
                    GL_LUMINANCE_ALPHA as i32,
                    l as i32,
                    l as i32,
                    0,
                    GL_LUMINANCE_ALPHA,
                    GL_UNSIGNED_BYTE,
                    texels.as_ptr() as *const c_void,
                );
            }
        }
    }
    assert_eq!(rig.gl.error(), GL_NO_ERROR);
    let t = &rig.gl.ctx().objs().textures[name as usize];
    assert_eq!((t.texfmt, t.layers), (GLES_TEXFMT_LA8, 6));
    for level in 0..levels {
        let (off, pitch, rows) = texture::gl_level_layered(size, size, 6, 2, level);
        assert_eq!(off, t.level_off[level as usize], "LA8 cube level {level}: offset");
        for face in 0..6u32 {
            let at = (off + face * rows * pitch * 2) as usize;
            // SAFETY: inside the texture's storage (the offsets above).
            let got = unsafe { core::slice::from_raw_parts((t.mem.cpu as *const u8).add(at), 2) };
            assert_eq!(got, [face as u8 + 1, level as u8 | 0x80], "LA8 cube level {level} face {face}");
        }
    }
    // SAFETY: a live name; the unpack alignment back to GL's default.
    unsafe {
        glDeleteTextures(1, &name);
        crate::gl::state::glPixelStorei(GL_UNPACK_ALIGNMENT, 4);
    }
}

/// A cube map is sampled (cube step 2): its six faces, stacked in each level
/// of the GL core's storage (`texture::gl_level_layered`), are tiled into six
/// mip trees `v3d_tex_layer_stride` apart, face f's level L from the GL
/// core's rows f * lh into that level. Each face of each level has its own
/// texels here (face, level, x, y) and each is found in its own tree.
/// Revert checks: tiling every face from face 0's rows, or every face into
/// tree 0, fails it.
#[test]
fn a_cube_maps_six_faces_are_tiled_into_six_trees() {
    let rig = Rig::new();
    rig.init();
    let (size, levels) = (8u32, 4u32);
    let mut b = GlesTextureBind {
        width: size,
        height: size,
        stride_texels: size,
        type_: GLES_TEXTYPE_CUBE,
        layers: 6,
        mip_levels: levels as u8,
        mip_filter: 1,
        format: GLES_TEXFMT_RGBA8,
        content_gen: 51,
        ..GlesTextureBind::ZERO
    };
    let bytes = texture::gl_bytes(&b);
    let mut mem = OsalGpuMem::ZERO;
    assert_eq!(crate::osal::gpu_alloc(V3D_HEAP_DATA, bytes as usize, OSAL_MEM_DATA, &mut mem), 0);
    for level in 0..levels {
        let (off, pitch, rows) = texture::gl_level_layered(size, size, 6, 4, level);
        for face in 0..6u32 {
            for y in 0..rows {
                for x in 0..(size >> level).max(1) {
                    let at = (off + ((face * rows + y) * pitch + x) * 4) as usize;
                    // SAFETY: inside the storage (gl_bytes covers every layer).
                    unsafe {
                        core::ptr::copy_nonoverlapping(
                            [face as u8, level as u8, x as u8, y as u8].as_ptr(),
                            (mem.cpu as *mut u8).add(at),
                            4,
                        )
                    };
                }
            }
        }
    }
    b.base_gpu = mem.gpu;
    assert_eq!(texture::refusal(&b), None);
    assert_eq!(draw(&sampling(&rig, b)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let layout = texture::layout(&b).unwrap();
    // SAFETY: a live layout.
    let stride = unsafe { v3d_tex_layer_stride(&layout) } as usize;
    let image = copy_image(b.base_gpu, texture::image_bytes(&b, &layout).unwrap() as usize);
    for face in 0..6usize {
        for (level, &s) in layout.slices.iter().enumerate().take(levels as usize) {
            let l = (size as usize >> level).max(1);
            for y in 0..l {
                for x in 0..l {
                    /* the fake tiling packs a level's rows at its slice */
                    let at = face * stride + s.offset as usize + (y * l + x) * 4;
                    assert_eq!(
                        image[at..at + 4],
                        [face as u8, level as u8, x as u8, y as u8],
                        "face {face} level {level} ({x}, {y})"
                    );
                }
            }
        }
    }
}

/// The arena line carries the binner's tile-pool use, read from the raw
/// registers the OSAL kept after the job's binning: BPCA less the pool's
/// start when it lies inside the pool, "not read" otherwise (no answer, or a
/// position outside the pool), never 0. The first answer is printed raw once.
/// Revert check: taking an outside BPCA as a use prints a number where "not
/// read" is due. A job that took overflow blocks prints "ran out" and the
/// blocks taken; revert check: ignoring overflow_used prints the old line.
#[test]
fn the_arena_line_carries_the_binners_tile_pool_use() {
    use crate::osal::OsalV3dBinPool;
    let rig = Rig::new();
    rig.init();
    let d = rig.draw();
    let mut n = 0;
    /* one draw more each frame: more BCL, so every frame raises the high-water */
    let mut frame = |d: &GlesDraw| {
        n += 1;
        for i in 0..n {
            let mut e = *d;
            e.do_clear = i == 0;
            assert_eq!(draw(&e), 0);
        }
        v3d_surface_swapped(d.target_gpu);
        assert_eq!(v3d_flush(), 0);
    };
    let arena = |rig: &Rig| -> Vec<String> {
        rig.gl
            .logs()
            .into_iter()
            .filter(|l| l.contains("binner tile pool high-water") || l.contains("binner pool"))
            .collect()
    };
    /* no answer: not read */
    frame(&d);
    let l = arena(&rig);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].ends_with("frame 1: not read"), "{l:?}");
    /* an answer inside the pool: its use, and the raw line once */
    let pool = OsalV3dBinPool {
        jobs: 1,
        pool: 0x0400_0000,
        pool_size: 65536,
        bpca: 0x0400_1000,
        bpcs: 61440,
        qma: 0x0400_0000,
        qms: 65536,
        overflow_used: 0,
        overflow_blocks: 2,
        overflow_block_size: 262_144,
    };
    with_platform(|p| p.bin_pool = Some(pool));
    frame(&d);
    let l = arena(&rig);
    assert_eq!(l.len(), 2, "{l:?}");
    assert!(l[0].contains("binner pool after job 1: given 0x04000000 + 65536, BPCA 0x04001000"), "{l:?}");
    assert!(l[1].ends_with("frame 2: 4096 of 65536 bytes (a job's pool not read)"), "{l:?}");
    /* a position outside the pool cannot be a use: the high-water keeps 4096 */
    with_platform(|p| p.bin_pool = Some(OsalV3dBinPool { bpca: 0x0300_0000, jobs: 2, ..pool }));
    frame(&d);
    let l = arena(&rig);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(l[0].ends_with("frame 3: 4096 of 65536 bytes (a job's pool not read)"), "{l:?}");
    /* a job that ran the pool out: BPCA is in an overflow block, so no pool
     * use is read, and the line says "ran out" with the blocks taken (not
     * "not read", and not the BPCA's distance from the pool) */
    with_platform(|p| p.bin_pool = Some(OsalV3dBinPool { bpca: 0x0500_2000, jobs: 3, overflow_used: 1, ..pool }));
    frame(&d);
    let l = arena(&rig);
    assert_eq!(l.len(), 1, "{l:?}");
    assert!(
        l[0].ends_with(
            "frame 4: ran out of 65536 bytes: 1 of 2 overflow blocks of 262144 bytes taken \
             (a job's pool not read)"
        ),
        "{l:?}"
    );
}

/// The job arenas follow the draw budget: 4 KiB + 129 B and 8 KiB + 440 B a
/// draw (15 % over Slint's measured 112 and 382), rounded up to 4 KiB; the
/// OSAL's budget when it gives one, 1024 otherwise; the init line says which.
/// Revert check: the arenas sized from the default whatever the OSAL says
/// fails the 64-draw half.
#[test]
fn the_job_arenas_follow_the_draw_budget() {
    assert_eq!(arena_sizes(1024), Some((139_264, 458_752)));
    assert_eq!(arena_sizes(64), Some((16_384, 36_864)));
    assert_eq!(arena_sizes(u32::MAX), None);
    /* the platform's budget */
    let rig = Rig::new();
    with_platform(|p| p.draws_per_job = Some(64));
    rig.init();
    assert_eq!((g().draws_per_job, g().bcl_bytes, g().ind_bytes), (64, 16_384, 36_864));
    assert_eq!(rig.logs_with("job arenas for 64 draws (the platform's budget): BCL 16384, indirect 36864"), 1);
    drop(rig);
    /* no answer: the default */
    let rig = Rig::new();
    rig.init();
    assert_eq!((g().draws_per_job, g().bcl_bytes, g().ind_bytes), (1024, 139_264, 458_752));
    assert_eq!(rig.logs_with("job arenas for 1024 draws (the default)"), 1);
}

/// A samplerCube with nothing bound, or bound incomplete, samples a 1x1 CUBE
/// stand-in, never the 2D one: a cube look-up steps faces by the record's
/// array stride, so a 2D record (depth 1) would send faces 1..5 past its
/// image. Mesa samples a fallback texture of the sampler's own target too.
/// The unit's type comes from the linked program's uniform table. Revert
/// checks: the cube mask ignored, or the incomplete binding's type ignored,
/// gives the 2D record.
#[test]
fn a_cube_sampler_with_nothing_bound_samples_a_cube_stand_in() {
    use crate::shaderc::{ShcUniform, SHC_UNIFORM_NAME_MAX, SHC_UNIFORM_NO_BLOCK};
    static FS_TEX: [ShcV3dUniform; 4] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 2 << 24 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 2 << 24 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 1 << 24 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 1 << 24 },
    ];
    let sampler = |gl_type: u16, binding: u8| ShcUniform {
        name: [0; SHC_UNIFORM_NAME_MAX],
        dword_off: SHC_UNIFORM_NO_BLOCK,
        dwords: 0,
        gl_type,
        array_len: 1,
        binding,
        inner: 0,
    };
    let table = [sampler(GL_SAMPLER_CUBE, 2), sampler(0x8B5E /* sampler2D */, 1)];
    let rig = Rig::new();
    rig.init();
    let mut p = program();
    (p.v3d.fs.uniforms, p.v3d.fs.num_uniforms) = (FS_TEX.as_ptr(), 4);
    (p.uniforms, p.num_uniforms) = (table.as_ptr(), 2);
    with_code(&mut p);
    let mut d = rig.draw();
    d.fs = &p;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    /* an incomplete cube binding on unit 3 */
    let mut e = d;
    e.do_clear = false;
    e.textures[0] =
        GlesTextureBind { unit: 3, type_: GLES_TEXTYPE_CUBE, layers: 6, incomplete: 1, ..texture(3, 4, 4, 1) };
    e.num_textures = 1;
    assert_eq!(draw(&e), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let (cube, flat) = (black_state(g(), true), black_state(g(), false));
    assert_ne!(cube, flat);
    let seen: Vec<_> = rig.draws()[0].tmu.iter().map(|t| (t.unit, t.tex_state)).collect();
    assert_eq!(seen, [(2, cube), (1, flat)], "the cube unit gets the cube stand-in");
    let seen: Vec<_> = rig.draws()[1].tmu.iter().map(|t| (t.unit, t.tex_state)).collect();
    assert!(seen.contains(&(3, cube)), "{seen:?}: an incomplete cube binding gets it too");
    /* the stand-in: depth 6, and six black faces */
    let b = black_cube_binding();
    let layout = texture::layout(&b).unwrap();
    let st = texture::state(&b, &layout, 0);
    assert_eq!(st.depth, 6);
    /* a 2D-array look-up of a face (`LodKey::cube_noseam`, TMUI) reads these
     * six layers, `array_stride_64` apart. (This module stubs
     * v3d_tex_state_from_layout, so the record's own stride is not what is
     * checked here: the layout's layer stride, which the C derives it from
     * as align(offset + size, 64) / 64 = 1, is, and the faces below.) */
    // SAFETY: a live layout.
    let stride = unsafe { v3d_tex_layer_stride(&layout) } as usize;
    assert_eq!(stride, 64, "the layer stride the record's array stride is made of");
    // SAFETY: the stand-ins' 8192 bytes, written at init.
    let mem = unsafe { core::slice::from_raw_parts(g().black.cpu as *const u8, 8192) };
    for f in 0..6 {
        let at = BLACK_CUBE_OFF as usize + f * stride;
        assert_eq!(mem[at..at + 4], [0, 0, 0, 255], "face {f}");
    }
}

/// A texture whose tiled image is larger than the per-job texture arena
/// (8 MiB) is sampled from its kept copy when the tiled budget holds it
/// (dEQP's 2048x2048 size cases, refused on the board with the budget at
/// 8 MiB: T5, 651755c7). The arena's size check applies only when a texture
/// goes to the arena: an unstamped one (an EGLImage's), or one the kept
/// copies cannot take. The budget is the OSAL's (OSAL_QUERY_V3D_TILED_BUDGET)
/// or 8 MiB. Refused, the message names why the kept copy was not made
/// (unstamped, over the budget, no memory). Revert checks: the arena check
/// before the kept copy refuses the stamped texture; one message for every
/// reason fails the no-memory line.
#[test]
fn a_texture_larger_than_the_arena_is_sampled_from_its_kept_copy() {
    let rig = Rig::new();
    with_platform(|p| p.tiled_budget = Some(24 << 20));
    rig.init();
    assert_eq!(g().tiled_budget, 24 << 20);
    assert_eq!(rig.logs_with("tiled copies kept up to 25165824 bytes"), 1);
    /* 2048 x 1024 RGBA8: 8 MiB tiled in the rig's layout, over the arena */
    let mut t = texture(0, 2048, 1024, 1);
    let layout = texture::layout(&t).unwrap();
    assert!(texture::image_bytes(&t, &layout).unwrap() as usize + 4096 + 64 > TEX_BYTES);
    t.content_gen = 61;
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tiled().0, 1, "sampled from a kept copy");
    /* unstamped: the arena, and its size check */
    t.content_gen = 0;
    assert_eq!(draw(&sampling(&rig, t)), -EINVAL);
    assert_eq!(rig.logs_with("an unstamped (EGLImage) texture larger than the V3D's texture arena (8 MiB tiled)"), 1);
    /* no memory for the kept copy (T5, eaf98dc8: a 22 MB chain inside a
     * 24 MiB budget): the refusal says so, not only the arena's size */
    let mut u = texture(0, 2048, 1024, 1);
    u.content_gen = 63; /* a new stamp: the copy kept above goes */
    with_platform(|p| p.fail_alloc_from = Some(TEX_BYTES));
    assert_eq!(draw(&sampling(&rig, u)), -EINVAL);
    with_platform(|p| p.fail_alloc_from = None);
    let logs = rig.gl.logs();
    assert_eq!(logs.iter().filter(|l| l.contains("no GPU memory for the texture's tiled copy")).count(), 1, "{logs:?}");
    assert!(
        logs.iter().any(|l| l.contains("no GPU memory for a tiled copy of") && l.contains("a job open")),
        "the numbers: {logs:?}"
    );
    drop(rig);
    /* the default budget (no answer): the same stamped texture is refused */
    let rig = Rig::new();
    rig.init();
    assert_eq!(g().tiled_budget, DEFAULT_TILED_BUDGET);
    let mut t = texture(0, 2048, 1024, 1);
    t.content_gen = 62;
    assert_eq!(draw(&sampling(&rig, t)), -EINVAL);
    assert_eq!(rig.logs_with("a texture larger than the tiled-copy budget"), 1);
}

/// The backend tells the OSAL when it opens a job (1) and when none is open
/// (0, after the job has run), so the OSAL gives freed memory back at once
/// without an open job and holds it for the job being recorded otherwise
/// (src/osal/v3d_frees.h; T4's deleted cube textures stayed held without
/// it). A free between jobs reaches the OSAL with 0 last. Revert check: the
/// close not sent leaves 1 last and fails it.
#[test]
fn the_backend_tells_the_osal_when_a_job_is_open() {
    let rig = Rig::new();
    rig.init();
    let d = rig.draw();
    assert_eq!(draw(&d), 0);
    assert_eq!(with_platform(|p| p.job_open.clone()), [1], "the draw opened a job");
    assert_eq!(v3d_flush(), 0);
    assert_eq!(with_platform(|p| p.job_open.clone()), [1, 0], "closed after the job ran");
    /* two jobs: each opens and closes */
    assert_eq!(draw(&d), 0);
    let mut other = d;
    other.target_gpu = TARGET + 0x10_0000;
    assert_eq!(draw(&other), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(with_platform(|p| p.job_open.clone()), [1, 0, 1, 0, 1, 0]);
}

/// glmark2's shadow on the Pi 4: the depth texture's storage takes a new
/// stamp every frame (texture::resolve_depth), after the depth pass's job
/// has been flushed, so the main pass's draw arrives with no job open. Its
/// kept copy of the old stamp is given back then (`drop_stale_copies`),
/// while no job is open, so the free waits only for the job in flight; the
/// new copy then fits a carve-out that holds one copy and not two, with no
/// failed allocation and no extra job. A draw arriving with a job open drops
/// nothing (that job may have sampled the copy).
///
/// Revert check: without `drop_stale_copies`, `tiled_copy` frees the old
/// copy with the new job open, the mock OSAL holds it for that job, and the
/// new copy's allocation fails with it held (enomem_held 1, the assertion
/// "no allocation failed").
#[test]
fn a_stale_copy_is_given_back_before_the_job_opens() {
    let rig = Rig::new();
    with_platform(|p| p.tiled_budget = Some(16 << 20));
    rig.init();
    let mut t = texture(0, 1024, 1024, 1);
    t.content_gen = 81;
    assert_eq!(draw(&sampling(&rig, t)), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    assert_eq!(tiled().0, 1, "the first stamp's copy kept");
    let copy = tiled().1 as usize;
    /* a new stamp of the same storage, and a carve-out with room for one
     * copy more only once the old copy is given back */
    let mut t2 = t;
    t2.content_gen = 82;
    with_platform(|p| p.capacity = Some(p.used() + copy - (64 << 10)));
    let kicks = rig.kicks().len();
    assert_eq!(draw(&sampling(&rig, t2)), 0, "{:?}", rig.gl.logs());
    assert_eq!(rig.kicks().len(), kicks, "no job was ended to make room");
    assert_eq!(with_platform(|p| (p.enomem_held, p.enomem_none_held)), (0, 0), "no allocation failed");
    assert_eq!((g().stale_dropped, g().stale_dropped_bytes), (1, copy as u64));
    assert_eq!(tiled().0, 1, "the new stamp's copy kept, the old one gone");
    assert!(rig.gl.logs().iter().any(|l| l.contains("a stale tiled copy of")), "the first drop is logged");
    /* with the job open, a third stamp drops nothing before the draw */
    let mut t3 = t;
    t3.content_gen = 83;
    with_platform(|p| p.capacity = None);
    assert_eq!(draw(&sampling(&rig, t3)), 0, "{:?}", rig.gl.logs());
    assert_eq!(g().stale_dropped, 1, "a job was open: nothing dropped early");
    assert_eq!(v3d_flush(), 0);
}

/// T5 on 68b6df6a (dEQP texture.size 2048x2048, 64 MiB carve-out): the
/// previous case's draw, still in the open job, samples a kept copy; the
/// next draw's texture needs a copy that fits the budget only with that one
/// evicted. The eviction frees it through gpu_free, the OSAL holds the free
/// for the open job (it samples the copy), and the new copy's allocation
/// fits only once the held free is given back. The draw ends the open job
/// (which drains the held free), tries once in a new one and is drawn. With
/// no memory in the new job too, it is refused with the no-memory reason.
/// The mock OSAL holds frees made with a job open, as v3d_frees.h does.
///
/// Revert check: without Attempt::NoMemory (the no-memory miss refused at
/// once), the second draw returns -EINVAL with the no-memory refusal, one
/// kick fewer, and memory_retries stays 0.
#[test]
fn a_kept_copy_that_fits_only_after_the_open_job_runs_is_made_after_it() {
    let rig = Rig::new();
    with_platform(|p| p.tiled_budget = Some(16 << 20));
    rig.init();
    /* A: 8 MiB tiled, B: 12 MiB tiled; both over the texture arena, and
     * A + B over the 16 MiB budget, so B's copy evicts A's */
    let mut a = texture(0, 2048, 1024, 1);
    a.content_gen = 71;
    let mut b = texture(0, 2048, 1536, 1);
    b.content_gen = 72;
    let bytes = |t: &GlesTextureBind| texture::image_bytes(t, &texture::layout(t).unwrap()).unwrap() as usize;
    let (a_bytes, b_bytes) = (bytes(&a), bytes(&b));
    assert!(a_bytes + 4096 + 64 > TEX_BYTES && b_bytes + 4096 + 64 > TEX_BYTES, "both over the arena");
    assert!(a_bytes + b_bytes + 2 * 4096 > 16 << 20, "B's copy evicts A's");
    /* the first draw keeps A's copy; the job stays open (no flush between
     * dEQP's cases) */
    assert_eq!(draw(&sampling(&rig, a)), 0, "{:?}", rig.gl.logs());
    assert_eq!(tiled().0, 1, "A's copy kept");
    let kicks = rig.kicks().len();
    /* the carve-out: what is live now (A's copy included) and B's copy,
     * less 1 MiB: B fits only once A's copy is given back */
    with_platform(|p| {
        let cap = p.used() + b_bytes + 4096 - (1 << 20);
        p.capacity = Some(cap);
    });
    assert_eq!(draw(&sampling(&rig, b)), 0, "{:?}", rig.gl.logs());
    assert_eq!(rig.kicks().len(), kicks + 1, "the open job ran before B's draw");
    assert_eq!(
        with_platform(|p| (p.enomem_held, p.enomem_none_held)),
        (1, 0),
        "B's first allocation failed with A's copy held, the retry did not fail"
    );
    assert_eq!((g().memory_retries, g().memory_retries_drawn), (1, 1));
    /* one take of the log (logs() drains it), every count on it */
    let logs = rig.gl.logs();
    let count = |w: &str| logs.iter().filter(|l| l.contains(w)).count();
    assert_eq!(count("out of GPU memory") + count("no GPU memory for a tiled copy of"), 1, "{logs:?}");
    assert_eq!(count("the job runs first, and the draw is tried in a new one (retry 1)"), 1, "{logs:?}");
    assert_eq!(count("draw refused"), 0, "not refused: {logs:?}");
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rig.kicks().len(), kicks + 2);
    assert_eq!(tiled().0, 1, "B's copy kept, A's gone");
    let seen = &rig.draws();
    assert_eq!(seen.len(), 2, "both draws recorded");
    /* no room even with every held free given back (C is larger than B's
     * copy and the 1 MiB left): tried once in a new job, refused */
    with_platform(|p| p.capacity = None);
    let mut c = texture(0, 2048, 2000, 1);
    c.content_gen = 73;
    assert!(bytes(&c) + 4096 <= 16 << 20 && bytes(&c) > b_bytes + (1 << 20) + 4096);
    with_platform(|p| p.capacity = Some(p.used() + (1 << 20)));
    let kicks = rig.kicks().len();
    let (_, repeats0) = crate::osal::log_repeats();
    assert_eq!(draw(&sampling(&rig, c)), -EINVAL);
    assert_eq!(rig.kicks().len(), kicks + 1, "the open job ran once, not again");
    assert_eq!(
        with_platform(|p| (p.enomem_held, p.enomem_none_held)),
        (2, 1),
        "C's first allocation failed with B's copy held, the retry with nothing held"
    );
    assert_eq!((g().memory_retries, g().memory_retries_drawn), (2, 1));
    let logs = rig.gl.logs();
    let count = |w: &str| logs.iter().filter(|l| l.contains(w)).count();
    assert_eq!(count("the job runs first, and the draw is tried in a new one (retry 2)"), 1, "{logs:?}");
    /* the line is glog_once!'s (it repeats on every draw while memory is
     * short): logged at B's attempt above, so both of C's attempts are
     * repeats, counted instead of printed */
    assert_eq!(count("no GPU memory for a tiled copy of"), 0, "logged once, at the first attempt: {logs:?}");
    assert!(crate::osal::log_repeats().1.wrapping_sub(repeats0) >= 2, "both attempts reached the line");
    assert_eq!(count("no GPU memory for the texture's tiled copy"), 1, "refused with the reason: {logs:?}");
    with_platform(|p| p.capacity = None);
}

/// A vertex-shader texture look-up (v3d-m5, 38f9d114): the VS and CS streams
/// carry P0/P1 words for the VS's own sampler bindings, and the job builder
/// takes their records from the VS's table (d.vs_textures, filled from
/// vs_sampler_unit). A bound unit gets its texture's record; an unbound one
/// the stand-in, a cube when the vertex shader's reflection (GlesDraw::vs)
/// declares that binding a samplerCube. Revert check: the VS cube mask taken
/// as 0 gives unit 1 the 2D stand-in.
#[test]
fn a_vertex_shader_samples_its_own_units_and_a_cube_stand_in() {
    use crate::shaderc::{ShcUniform, SHC_UNIFORM_NAME_MAX, SHC_UNIFORM_NO_BLOCK};
    static VS_TEX: [ShcV3dUniform; 4] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 4 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 1 << 24 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 1 << 24 | 4 },
    ];
    let cube = ShcUniform {
        name: [0; SHC_UNIFORM_NAME_MAX],
        dword_off: SHC_UNIFORM_NO_BLOCK,
        dwords: 0,
        gl_type: GL_SAMPLER_CUBE,
        array_len: 1,
        binding: 1,
        inner: 0,
    };
    let vs_table = [cube];
    let mut vs = ShcShader::ZERO;
    (vs.uniforms, vs.num_uniforms) = (vs_table.as_ptr(), 1);
    let rig = Rig::new();
    rig.init();
    let mut p = program();
    (p.v3d.vs.uniforms, p.v3d.vs.num_uniforms) = (VS_TEX.as_ptr(), 4);
    with_code(&mut p);
    let mut d = rig.draw();
    d.fs = &p;
    d.vs = &vs;
    let mut t = texture(0, 8, 8, 1);
    t.content_gen = 71;
    d.vs_textures[0] = t;
    d.num_vs_textures = 1;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let seen = &rig.draws()[0];
    assert_eq!(seen.vs_tmu.len(), 2, "{:?}", seen.vs_tmu);
    assert_eq!(seen.vs_tmu[0].unit, 0);
    assert!(seen.vs_tmu[0].tex_state != black_state(g(), false) && seen.vs_tmu[0].tex_state != black_state(g(), true));
    assert_eq!((seen.vs_tmu[1].unit, seen.vs_tmu[1].tex_state), (1, black_state(g(), true)));
    assert!(seen.tmu.is_empty(), "the FS samples nothing");
}

/* ---- dEQP fbo.render sequences through the GL core onto the V3D (FB2) ------ */

/// The rig with the GL core's draws going to this backend instead of
/// TestGl's recording mock, so a dEQP sequence of GL calls reaches the job
/// builder as the product's would (the rig's builder records the jobs).
fn gl_rig() -> Rig {
    let rig = Rig::new();
    // SAFETY: serialised by the rig's TestGl guard; TestGl::new installs its
    // mock again for the next test.
    unsafe { crate::backend::g_backend = &yakogl_backend_v3d };
    rig.init();
    rig
}

/// A program of `vs` and `fs` through the GL core: compiled, attached,
/// linked (a_position at 0), made current, and its shader names. The shaders
/// are deleted right after the link when `delete_shaders` (the program keeps
/// them), as much application code does; otherwise the caller deletes them
/// later, as dEQP's case teardown does.
fn gl_program_of(vs: &str, fs: &str, delete_shaders: bool) -> (u32, Vec<u32>) {
    use crate::gl::consts::*;
    use crate::gl::program::*;
    use crate::gl::shader::*;
    use std::ffi::CString;
    // SAFETY: plain GL calls on the rig's context with live strings.
    unsafe {
        let p = glCreateProgram();
        let mut shaders = Vec::new();
        for (ty, src) in [(GL_VERTEX_SHADER, vs), (GL_FRAGMENT_SHADER, fs)] {
            let s = glCreateShader(ty);
            let c = CString::new(src).unwrap();
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile 0x{ty:x}: {src}");
            glAttachShader(p, s);
            shaders.push(s);
        }
        let name = CString::new("a_position").unwrap();
        glBindAttribLocation(p, 0, name.as_ptr());
        glLinkProgram(p);
        let mut ok = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link: {fs}");
        if delete_shaders {
            for s in shaders.drain(..) {
                glDeleteShader(s);
            }
        }
        glUseProgram(p);
        (p, shaders)
    }
}

/// The dEQP struct.uniform shape (es2fShaderStructTests.cpp,
/// struct_array_dynamic_index_fragment): an array of structs, read at
/// indices uniforms give, every member set by name.
const STRUCT_FS: &str = "precision mediump float;\n\
    struct S { mediump float a; mediump vec3 b; int c; };\n\
    uniform S s[3];\n\
    uniform int ui_one;\n\
    uniform int ui_two;\n\
    void main() { gl_FragColor = vec4(s[ui_one].b.xy, s[ui_two].a, float(s[ui_one].c) * 0.25); }\n";
const PLAIN_VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";

/// STRUCT_FS as a program, its uniforms set through glGetUniformLocation
/// and glUniform*, drawn, finished; the draw the job builder saw.
fn struct_program_draw(rig: &Rig) -> SeenDraw {
    use crate::gl::program::*;
    use std::ffi::CString;
    let (p, _) = gl_program_of(PLAIN_VS, STRUCT_FS, true);
    let loc = |n: &str| {
        let c = CString::new(n).unwrap();
        // SAFETY: a live program and name.
        let l = unsafe { glGetUniformLocation(p, c.as_ptr()) };
        assert!(l >= 0, "{n} has a location");
        l
    };
    // SAFETY: live locations of the current program.
    unsafe {
        for k in 0..3 {
            glUniform1f(loc(&format!("s[{k}].a")), 0.125 + k as f32 * 0.25);
            glUniform3f(loc(&format!("s[{k}].b")), 0.5 + k as f32, 1.5 + k as f32, 2.5 + k as f32);
            glUniform1i(loc(&format!("s[{k}].c")), k + 1);
        }
        glUniform1i(loc("ui_one"), 1);
        glUniform1i(loc("ui_two"), 2);
    }
    let before = rig.draws().len();
    gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
    // SAFETY: plain GL calls.
    unsafe {
        crate::gl::state::glFinish();
        assert_eq!(crate::gl::state::glGetError(), crate::gl::consts::GL_NO_ERROR, "{:?}", rig.gl.logs());
    }
    let draws = rig.draws();
    assert_eq!(draws.len(), before + 1, "one quad: one draw of two triangles");
    let d = draws.last().cloned().expect("the quad");
    gl_delete_program((p, Vec::new()));
    d
}

/// Delete a program, then the shaders it was built from that are still
/// named, and nothing else (the framebuffer stays bound).
fn gl_delete_program((p, shaders): (u32, Vec<u32>)) {
    use crate::gl::program::*;
    use crate::gl::shader::*;
    // SAFETY: live names.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(p);
        for s in shaders {
            glDeleteShader(s);
        }
    }
}

/// What a draw read that a stale keep read would change: the program's
/// uniform streams (read from its keep record during the draw) and the
/// uniform blocks the GL core filled through the uniform table.
#[derive(Debug, PartialEq)]
struct KeepReads {
    /// The program's uniform streams, (contents, data) per entry.
    fs_uniforms: Vec<(u32, u32)>,
    vs_uniforms: Vec<(u32, u32)>,
    /// The uniform blocks the GL core filled.
    fs_storage: Vec<u32>,
    vs_storage: Vec<u32>,
}

fn keep_reads(d: &SeenDraw) -> KeepReads {
    let pairs = |u: &[V3dUniform]| u.iter().map(|x| (x.contents, x.data)).collect::<Vec<_>>();
    KeepReads {
        fs_uniforms: pairs(&d.fs_uniforms),
        vs_uniforms: pairs(&d.vs_uniforms),
        fs_storage: d.fs_storage.clone(),
        vs_storage: d.vs_storage.clone(),
    }
}

/// dEQP's E run failed four struct.uniform cases only after 442 earlier
/// cases in one process (2026-09-24, 8c8df5d1): a stale read of a keep
/// record would look like that, once the keep region reuses what a program
/// let go (10bcfe1a places short-lived records where the next compile
/// reuses them at once). This replays the shape on the host: the struct
/// program drawn on a fresh region, then hundreds of create / compile /
/// link / use / set / draw / finish / delete cycles of varied programs,
/// some deleting their shaders after the link and some kept alive across
/// later cycles, then the same struct program again. Every word the draw
/// read from a keep record or through the uniform table must be the same,
/// and none may be the poison test builds write over a freed extent
/// (store::KEEP_POISON), which is what a read of a freed record returns.
#[test]
fn a_struct_uniform_program_reads_the_same_after_hundreds_of_programs() {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::shaderc::online::store::{keep_in_use, keep_largest_free, keep_used, KEEP_POISON};
    let rig = gl_rig();
    /* a renderbuffer target, as the rig's GL tests draw into (its window
     * surface is host memory) */
    let rb = gl_renderbuffer(GL_RGBA4, 4, 4);
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 1);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        crate::gl::state::glViewport(0, 0, 4, 4);
    }
    let fresh = keep_reads(&struct_program_draw(&rig));
    let held = keep_in_use();
    let mut kept = Vec::new();
    for i in 0..300u32 {
        let fs = match i % 4 {
            0 => format!(
                "precision mediump float;\nvoid main() {{ gl_FragColor = vec4({}.0 / 1000.0, 0.0, 0.0, 1.0); }}\n",
                i
            ),
            1 => format!(
                "precision mediump float;\nstruct T {{ mediump float a; mediump vec3 b; }};\nuniform T t;\n\
                 void main() {{ gl_FragColor = vec4(t.b, t.a + {}.0 / 1000.0); }}\n",
                i
            ),
            2 => format!(
                "precision mediump float;\nuniform vec4 u[8];\n\
                 void main() {{ gl_FragColor = u[{}] + vec4({}.0 / 1000.0); }}\n",
                i % 8,
                i
            ),
            _ => STRUCT_FS.replace("* 0.25", &format!("* 0.25 + {}.0 / 1000.0", i)),
        };
        let p = gl_program_of(PLAIN_VS, &fs, i % 3 != 0);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        // SAFETY: plain GL calls.
        unsafe { crate::gl::state::glFinish() };
        if i % 7 == 0 {
            kept.push(p); /* a program that outlives later ones */
        } else {
            gl_delete_program(p);
        }
        if kept.len() > 5 {
            gl_delete_program(kept.remove(0));
        }
    }
    for p in kept {
        gl_delete_program(p);
    }
    let after = keep_in_use();
    let late = keep_reads(&struct_program_draw(&rig));
    gl_release(0, &[1], &[rb]);
    println!(
        "keep: {} bytes in use, largest free run {}, high-water {}",
        keep_in_use(),
        keep_largest_free(),
        keep_used()
    );
    let poison = u32::from_ne_bytes([KEEP_POISON; 4]);
    let poisoned = |w: &[u32]| w.contains(&poison);
    assert!(
        !(poisoned(&late.fs_storage)
            || poisoned(&late.vs_storage)
            || late.fs_uniforms.iter().chain(&late.vs_uniforms).any(|&(c, d)| c == poison || d == poison)),
        "a draw read a freed keep record: {late:?}"
    );
    assert_eq!(late, fresh, "the struct program read something else after 300 programs");
    assert_eq!(after, held, "every program and shader of the 300 gave its records back");
}

/// A flat-colour program with a vec3 position (dEQP's FlatColorShader,
/// es2fFboRenderTest.cpp: sglr::drawQuad's corners carry z), attribute 0.
fn gl_flat_program() -> u32 {
    use crate::gl::consts::*;
    use crate::gl::program::*;
    use crate::gl::shader::*;
    use std::ffi::CString;
    const VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
    const FS: &str = "precision mediump float;\nuniform vec4 u_color;\nvoid main() { gl_FragColor = u_color; }\n";
    // SAFETY: plain GL calls on the rig's context with live strings.
    unsafe {
        let p = glCreateProgram();
        for (ty, src) in [(GL_VERTEX_SHADER, VS), (GL_FRAGMENT_SHADER, FS)] {
            let s = glCreateShader(ty);
            let c = CString::new(src).unwrap();
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile 0x{ty:x}");
            glAttachShader(p, s);
        }
        let name = CString::new("a_position").unwrap();
        glBindAttribLocation(p, 0, name.as_ptr());
        glLinkProgram(p);
        let mut ok = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        glUseProgram(p);
        let u = CString::new("u_color").unwrap();
        glUniform4f(glGetUniformLocation(p, u.as_ptr()), 0.0, 1.0, 0.0, 1.0);
        p
    }
}

/// sglr::drawQuad(p0, p1): two triangles from (p0.x, p0.y, p0.z) to
/// (p1.x, p1.y, p1.z), z interpolated along the diagonal as dEQP's corners
/// give it, from a client array at attribute 0.
fn gl_quad(p0: [f32; 3], p1: [f32; 3]) {
    use crate::gl::consts::*;
    use crate::gl::draw::glDrawArrays;
    use crate::gl::vertex::*;
    let mid = (p0[2] + p1[2]) * 0.5;
    let v: [f32; 18] = [
        p0[0], p0[1], p0[2], p1[0], p0[1], mid, p0[0], p1[1], mid, //
        p0[0], p1[1], mid, p1[0], p0[1], mid, p1[0], p1[1], p1[2],
    ];
    // SAFETY: `v` outlives the draw, which reads it during the call.
    unsafe {
        glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE as u8, 12, v.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawArrays(GL_TRIANGLES, 0, 6);
    }
}

/// Delete what a GL-level rig test made while the rig's OSAL, which
/// allocated it, is still the one installed.
fn gl_release(prog: u32, fbos: &[u32], rbs: &[u32]) {
    use crate::gl::framebuffer::*;
    use crate::gl::program::*;
    // SAFETY: live names; plain GL calls.
    unsafe {
        glUseProgram(0);
        glDeleteProgram(prog);
        glBindFramebuffer(crate::gl::consts::GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbos.len() as i32, fbos.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

fn gl_renderbuffer(format: u32, w: i32, h: i32) -> u32 {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    let mut rb = 0;
    // SAFETY: plain GL calls with a live out pointer.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
    }
    rb
}

/// A job's Z/S as the rig recorded it: (zs_address, zs_load, zs_store,
/// s_address, s_load, s_store, draws).
type ZsSeen = (u32, bool, bool, u32, bool, bool, u32);

fn zs_seen(rig: &Rig) -> Vec<ZsSeen> {
    rig.jobs()
        .iter()
        .filter_map(|j| {
            j.clear.map(|c| (c.zs_address, c.zs_load, c.zs_store, c.s_address, c.s_load, c.s_store, j.draws))
        })
        .collect()
}

/// SharedDepthbufferTest::render (es2fFboRenderTest.cpp:1540-1631) up to
/// the metaballs draw into fbo B, on RGBA4 renderbuffers: fbo B takes fbo
/// A's depth renderbuffer, and with `stencil` each keeps a stencil
/// renderbuffer of its own (the *_depth_component16_stencil_index8 cases),
/// with depth and stencil apart when `split`. B's depth-tested draw must
/// see the depth A's quad wrote. Returns every job, then A's and B's quad
/// jobs.
fn shared_depth_jobs(stencil: bool, split: bool) -> (Vec<ZsSeen>, ZsSeen, ZsSeen) {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::gl::state::*;
    let rig = gl_rig();
    v3d_split_zs(split);
    let prog = gl_flat_program();
    let (w, h) = (128, 128);
    let mut rbs = Vec::new();
    let mut fbo = [0u32; 2];
    // SAFETY: plain GL calls on the rig's context.
    unsafe {
        let depth = gl_renderbuffer(GL_DEPTH_COMPONENT16, w, h);
        rbs.push(depth);
        glGenFramebuffers(2, fbo.as_mut_ptr());
        for &f in &fbo {
            let colour = gl_renderbuffer(GL_RGBA4, w, h);
            rbs.push(colour);
            glBindFramebuffer(GL_FRAMEBUFFER, f);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, colour);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth);
            if stencil {
                let s = gl_renderbuffer(GL_STENCIL_INDEX8, w, h);
                rbs.push(s);
                glBindFramebuffer(GL_FRAMEBUFFER, f);
                glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, s);
            }
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        }
        let (a, b) = (fbo[0], fbo[1]);
        glViewport(0, 0, w, h);
        /* clear fbo B: colour red, stencil 1 */
        glBindFramebuffer(GL_FRAMEBUFFER, b);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClearStencil(1);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        /* fbo A: clear, then a quad at z 0 */
        glBindFramebuffer(GL_FRAMEBUFFER, a);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        /* fbo B: a quad from z -1 to +1, tested against A's depth */
        glBindFramebuffer(GL_FRAMEBUFFER, b);
        gl_quad([-1.0, -1.0, -1.0], [1.0, 1.0, 1.0]);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
    }
    gl_release(prog, &fbo, &rbs);
    let jobs = zs_seen(&rig);
    let a = *jobs.iter().find(|j| j.6 > 0).expect("fbo A's quad");
    let b = *jobs.iter().rev().find(|j| j.6 > 0).expect("fbo B's quad");
    (jobs, a, b)
}

/// The control: with no stencil renderbuffers the two fbos name the same
/// depth/stencil buffer, and B's job loads what A's stored, either way.
#[test]
fn fbos_sharing_a_depth_renderbuffer_see_each_others_depth() {
    for split in [false, true] {
        let (jobs, a, b) = shared_depth_jobs(false, split);
        assert!(a.2, "split {split}: A's quad wrote depth: stored ({jobs:x?})");
        assert!(b.1 && b.0 == a.0 && b.3 == 0, "split {split}: B loads A's one buffer ({jobs:x?})");
    }
}

/// FB2 (the board, 651755c7): shared_depthbuffer.*_stencil_index8, 7 image
/// failures. Each fbo keeps its own stencil renderbuffer, so the GL core's
/// ds_id ((depth << 16) | stencil) differs. In one buffer per id (the
/// default), B's job starts from its own buffer and never sees A's depth:
/// pinned here as the wrong picture it draws, until the board has measured
/// the halves apart (gl_test case zsapart). Apart, B loads the depth A
/// stored from the depth's own buffer, and each keeps its own stencil.
/// Revert check: `zs_keys` ignoring `split` fails the apart half.
#[test]
#[cfg(feature = "report-es2")] // ES 3.0 4.4.4.2: separate depth and stencil images are unsupported
fn fbos_sharing_a_depth_renderbuffer_with_their_own_stencil() {
    let (jobs, a, b) = shared_depth_jobs(true, false);
    assert!(
        a.2 && !b.1 && b.0 != a.0,
        "in one buffer per id, B does not see A's depth (the FB2 symptom, until switched) ({jobs:x?})"
    );
    let (jobs, a, b) = shared_depth_jobs(true, true);
    assert!(a.2 && a.3 != 0, "apart: A stores its depth, and has a stencil buffer of its own ({jobs:x?})");
    assert!(b.1 && b.0 == a.0, "apart: B's job loads the depth A stored ({jobs:x?})");
    assert!(b.3 != 0 && b.3 != a.3, "apart: B keeps its own stencil ({jobs:x?})");
}

/// dEQP's alpha-to-coverage cases (es2fMultisampleTests.cpp
/// MaskProportionalityCase / MaskConstancyCase) through the GL core. On a 4x
/// surface with GL_SAMPLE_ALPHA_TO_COVERAGE on, the backend refuses the quad
/// by name with -ENOTSUP, and the core's log line carries that errno. The
/// controls are drawn: the same quad with the feature off, and the feature
/// on a 1x surface (ES 2.0.25 4.1.3: no coverage change unless
/// SAMPLE_BUFFERS is one).
/// Revert check: `state::unsupported` answering None records the quad (the
/// named line and the refused count both fail).
#[test]
fn alpha_to_coverage_on_a_4x_surface_is_refused_by_name_through_gl() {
    use crate::gl::consts::*;
    use crate::gl::state::*;
    let rig = gl_rig();
    let prog = gl_flat_program();
    /* the default framebuffer in the V3D's address space: the rig's own
     * 4x4 surface is host memory, which the backend refuses as a target */
    let mut window = OsalGpuMem::ZERO;
    assert_eq!(crate::osal::gpu_alloc(V3D_HEAP_DATA, 4 * 4 * 4, OSAL_MEM_DATA, &mut window), 0);
    let host_surface = core::mem::replace(&mut rig.gl.ctx().default_surface, window);
    let quad = |samples: u8, a2c: bool| -> (u32, u32, Vec<String>) {
        rig.gl.ctx().default_samples = samples;
        let before = (v3d_refused_draws(), rig.draws().len());
        // SAFETY: plain GL calls on the rig's context.
        let err = unsafe {
            if a2c {
                glEnable(GL_SAMPLE_ALPHA_TO_COVERAGE);
            } else {
                glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
            }
            gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
            glGetError()
        };
        let refused = v3d_refused_draws().wrapping_sub(before.0);
        assert_eq!(refused == 0, rig.draws().len() > before.1, "a quad is either recorded or refused");
        (err, refused, rig.gl.logs())
    };
    let (err, refused, logs) = quad(4, false);
    assert_eq!((err, refused), (GL_NO_ERROR, 0), "4x, feature off: drawn {logs:?}");
    let (err, refused, logs) = quad(1, true);
    assert_eq!((err, refused), (GL_NO_ERROR, 0), "1x, feature on: drawn {logs:?}");
    let (err, refused, logs) = quad(4, true);
    assert_eq!(refused, 1, "{logs:?}");
    let named: Vec<&String> = logs.iter().filter(|l| l.contains("yakogl/v3d: draw refused")).collect();
    assert_eq!(named, ["yakogl/v3d: draw refused: alpha-to-coverage is not implemented"], "{logs:?}");
    let core: Vec<&String> = logs.iter().filter(|l| l.contains("was not recorded")).collect();
    assert!(core.len() == 1 && core[0].contains(&format!("({})", -ENOTSUP)), "{logs:?}");
    assert_eq!(
        err, GL_INVALID_OPERATION,
        "a backend gap (-ENOTSUP) is GL_INVALID_OPERATION, never GL_OUT_OF_MEMORY: dEQP turns \
         GL_OUT_OF_MEMORY into a fatal ResourceError that ends the whole run (the MSAAD abort); \
         GL_INVALID_OPERATION fails the case and lets the run go on"
    );
    // SAFETY: plain GL call.
    unsafe {
        glDisable(GL_SAMPLE_ALPHA_TO_COVERAGE);
        glFinish();
    }
    rig.gl.ctx().default_samples = 1;
    let mut window = core::mem::replace(&mut rig.gl.ctx().default_surface, host_surface);
    crate::osal::gpu_free(&mut window);
    gl_release(prog, &[], &[]);
}

/// RecreateBuffersTest<GL_STENCIL_BUFFER_BIT>::render (es2fFboRenderTest.cpp:
/// 1985-2102) on an RGBA4 renderbuffer, with (`depth`) or without a depth
/// renderbuffer, `rebind` or not, halves apart when `split`: a quad at z 0,
/// a scissored stencil clear to 1, the stencil renderbuffer deleted and made
/// again under its name, a stencil-only clear to 0, then a quad from z +1
/// to -1 with the stencil test EQUAL 0. Returns every job.
#[cfg(feature = "report-es2")]
fn recreate_stencil_jobs(depth: bool, rebind: bool, split: bool) -> Vec<ZsSeen> {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::gl::state::*;
    let rig = gl_rig();
    v3d_split_zs(split);
    let prog = gl_flat_program();
    let (w, h) = (128, 128);
    let mut rbs = Vec::new();
    let mut fbo = 0u32;
    // SAFETY: plain GL calls on the rig's context.
    unsafe {
        let colour = gl_renderbuffer(GL_RGBA4, w, h);
        let stencil = gl_renderbuffer(GL_STENCIL_INDEX8, w, h);
        rbs.extend([colour, stencil]);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, colour);
        if depth {
            let d = gl_renderbuffer(GL_DEPTH_COMPONENT16, w, h);
            rbs.push(d);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, d);
        }
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, stencil);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, w, h);
        glClearColor(1.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_DEPTH_TEST);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        glEnable(GL_SCISSOR_TEST);
        glScissor(w / 4, h / 4, w / 2, h / 2);
        glClearStencil(1);
        glClear(GL_STENCIL_BUFFER_BIT);
        glDisable(GL_SCISSOR_TEST);
        if !rebind {
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
        }
        glDeleteRenderbuffers(1, &stencil);
        glBindRenderbuffer(GL_RENDERBUFFER, stencil);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_STENCIL_INDEX8, w, h);
        if rebind {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_STENCIL_ATTACHMENT, GL_RENDERBUFFER, stencil);
        } else {
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        }
        glClearColor(0.0, 0.0, 1.0, 0.0);
        glClearStencil(0);
        glClear(GL_STENCIL_BUFFER_BIT);
        glEnable(GL_STENCIL_TEST);
        glStencilFunc(GL_EQUAL, 0, 0xff);
        gl_quad([-1.0, -1.0, 1.0], [1.0, 1.0, -1.0]);
        glDisable(GL_STENCIL_TEST);
        glDisable(GL_DEPTH_TEST);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
    }
    gl_release(prog, &[fbo], &rbs);
    zs_seen(&rig)
}

/// FB2: recreate_stencilbuffer.no_rebind_*_depth_component16_stencil_index8
/// (7 image failures). Binding the deleted stencil name again detaches it
/// from the fbo (the GL core's orphan rule, framebuffer.rs), so the id
/// drops from (depth << 16) | stencil to (depth << 16). In one buffer per
/// id (the default) that is a new buffer and the depth the first quad wrote
/// is lost: pinned as the wrong picture until switched. Apart, the depth
/// already lived under (depth << 16), and the last job loads it. rebind
/// keeps the id and loads either way.
#[test]
#[cfg(feature = "report-es2")] // ES 3.0 4.4.4.2: separate depth and stencil images are unsupported
fn a_recreated_stencil_renderbuffer_keeps_the_fbos_depth() {
    for split in [false, true] {
        let jobs = recreate_stencil_jobs(true, true, split);
        let (first, last) = (jobs[0], jobs[jobs.len() - 1]);
        assert!(first.2 && last.1 && last.0 == first.0, "rebind, split {split}: depth loaded ({jobs:x?})");
    }
    let jobs = recreate_stencil_jobs(true, false, false);
    assert!(
        jobs[0].2 && jobs[1..].iter().all(|j| j.0 != jobs[0].0),
        "no_rebind in one buffer per id: no later job reads the depth the first stored (the FB2 symptom, \
         until switched) ({jobs:x?})"
    );
    let jobs = recreate_stencil_jobs(true, false, true);
    let (first, last) = (jobs[0], jobs[jobs.len() - 1]);
    assert!(first.2 && first.3 != 0, "no_rebind apart: the first job stores depth and stencil apart ({jobs:x?})");
    assert!(last.1 && last.0 == first.0, "no_rebind apart: the last job loads the depth the first stored ({jobs:x?})");
}

/// One framebuffer more than the slots (33) through the GL core, each with a
/// depth renderbuffer of its own and the depth test on (gl_test case 162's
/// shape): the last one's first draw is GL_OUT_OF_MEMORY with the named
/// line, the first framebuffer's next job loads the depth it stored from its
/// own buffer, and once the first framebuffer's depth renderbuffer is
/// deleted the last draws.
#[test]
fn framebuffers_past_every_slot_through_the_gl_core_refuse_the_last_until_a_depth_is_deleted() {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::gl::state::{glClear, glEnable, glFinish, glGetError, glViewport};
    let rig = gl_rig();
    let prog = gl_flat_program();
    let (w, h) = (32, 32);
    let (mut fbos, mut depths, mut colours) = (Vec::new(), Vec::new(), Vec::new());
    let mut errors = Vec::new();
    // SAFETY: plain GL calls on the rig's context with live pointers.
    unsafe {
        glEnable(GL_DEPTH_TEST);
        /* one colour renderbuffer for all: 33 depth + 33 colour would pass
         * the GL core's 64 renderbuffers (context.rs MAX_RENDERBUFFERS) */
        let colour = gl_renderbuffer(GL_RGBA4, w, h);
        colours.push(colour);
        for _ in 0..=ZS_BUFS {
            let depth = gl_renderbuffer(GL_DEPTH_COMPONENT16, w, h);
            let mut fbo = 0;
            glGenFramebuffers(1, &mut fbo);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, colour);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, depth);
            glViewport(0, 0, w, h);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
            gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
            errors.push(glGetError());
            glFinish();
            fbos.push(fbo);
            depths.push(depth);
        }
    }
    let logs = rig.gl.logs();
    assert_eq!(errors[..ZS_BUFS], [GL_NO_ERROR; ZS_BUFS], "{logs:?}");
    assert_eq!(errors[ZS_BUFS], GL_OUT_OF_MEMORY, "the 33rd framebuffer's draw is refused");
    assert_eq!(logs.iter().filter(|l| l.contains(ds_retention::REFUSED)).count(), 1, "{logs:?}");
    let first = zs_seen(&rig)[0];
    // SAFETY: as above.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fbos[0]);
        gl_quad([-1.0, -1.0, 0.5], [1.0, 1.0, 0.5]);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR);
    }
    let again = *zs_seen(&rig).last().expect("a job");
    assert_eq!((again.0, again.1), (first.0, true), "framebuffer 0 loads its depth from its own buffer");
    // SAFETY: as above; the depth renderbuffer is detached from the bound
    // framebuffer and held by no other, so it is freed.
    unsafe {
        glDeleteRenderbuffers(1, &depths[0]);
        glBindFramebuffer(GL_FRAMEBUFFER, fbos[ZS_BUFS]);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        assert_eq!(glGetError(), GL_NO_ERROR, "the 33rd draws once a depth is deleted: {:?}", rig.gl.logs());
        glFinish();
    }
    colours.extend_from_slice(&depths[1..]);
    gl_release(prog, &fbos, &colours);
}

/// A textured program for the texsubimage sequence: dEQP's SingleTex2DShader
/// with the coordinate made from the position (the quads span the viewport).
fn gl_tex_program() -> u32 {
    use crate::gl::consts::*;
    use crate::gl::program::*;
    use crate::gl::shader::*;
    use std::ffi::CString;
    const VS: &str = "attribute vec4 a_position;\nvarying vec2 v_coord;\n\
        void main() { v_coord = a_position.xy * 0.5 + 0.5; gl_Position = a_position; }\n";
    const FS: &str = "precision mediump float;\nuniform sampler2D u_sampler0;\nvarying vec2 v_coord;\n\
        void main() { gl_FragColor = texture2D(u_sampler0, v_coord); }\n";
    // SAFETY: plain GL calls on the rig's context with live strings.
    unsafe {
        let p = glCreateProgram();
        for (ty, src) in [(GL_VERTEX_SHADER, VS), (GL_FRAGMENT_SHADER, FS)] {
            let s = glCreateShader(ty);
            let c = CString::new(src).unwrap();
            let srcs = [c.as_ptr()];
            glShaderSource(s, 1, srcs.as_ptr(), core::ptr::null());
            glCompileShader(s);
            let mut ok = 0;
            glGetShaderiv(s, GL_COMPILE_STATUS, &mut ok);
            assert_eq!(ok, 1, "compile 0x{ty:x}");
            glAttachShader(p, s);
        }
        let name = CString::new("a_position").unwrap();
        glBindAttribLocation(p, 0, name.as_ptr());
        glLinkProgram(p);
        let mut ok = 0;
        glGetProgramiv(p, GL_LINK_STATUS, &mut ok);
        assert_eq!(ok, 1, "link");
        glUseProgram(p);
        let u = CString::new("u_sampler0").unwrap();
        glUniform1i(glGetUniformLocation(p, u.as_ptr()), 0);
        p
    }
}

/// TexSubImageAfterRenderTest::render (es2fFboRenderTest.cpp:1659-1703),
/// and with `between` TexSubImageBetweenRenderTest's second render into the
/// fbo (:1731-1795), the last draw into another fbo (the rig's window
/// surface is host memory). FB2 failed all four on the board (and the
/// i.MX8MP does); on the rig the order is right, which this pins: the job
/// rendering into the texture is submitted before glTexSubImage2D writes
/// its storage, and the last draw samples a tiled copy of the texture's
/// current stamp. The picture itself is gl_test case texsubfbo's.
fn texsubimage_order(rgba: bool, between: bool) {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::gl::state::*;
    use crate::gl::texture::*;
    let rig = gl_rig();
    let prog = gl_tex_program();
    let fmt = if rgba { GL_RGBA } else { GL_RGB };
    let (quads, fbo_tex, fbo, screen) = (1u32, 2u32, 1u32, 2u32);
    let mut rbs = Vec::new();
    let four_quads = vec![0x40u8; 64 * 64 * 3];
    let metaballs = vec![0xc0u8; 64 * 64 * if rgba { 4 } else { 3 }];
    let (kicks_before, kicks_after);
    // SAFETY: plain GL calls on the rig's context with live buffers.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, quads);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGB as i32,
            64,
            64,
            0,
            GL_RGB,
            GL_UNSIGNED_BYTE,
            four_quads.as_ptr() as *const c_void,
        );
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glBindTexture(GL_TEXTURE_2D, fbo_tex);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt as i32, 128, 128, 0, fmt, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, fbo_tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, 128, 128);
        glBindTexture(GL_TEXTURE_2D, quads);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        glBindTexture(GL_TEXTURE_2D, fbo_tex);
        kicks_before = rig.kicks().len();
        glTexSubImage2D(GL_TEXTURE_2D, 0, 32, 32, 64, 64, fmt, GL_UNSIGNED_BYTE, metaballs.as_ptr() as *const c_void);
        kicks_after = rig.kicks().len();
        if between {
            glBindTexture(GL_TEXTURE_2D, quads);
            glEnable(GL_BLEND);
            glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
            gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
            glDisable(GL_BLEND);
        }
        let screen_rb = gl_renderbuffer(GL_RGBA4, 4, 4);
        rbs.push(screen_rb);
        glBindFramebuffer(GL_FRAMEBUFFER, screen);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, screen_rb);
        glViewport(0, 0, 4, 4);
        glBindTexture(GL_TEXTURE_2D, fbo_tex);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
    }
    let t = &rig.gl.ctx().objs().textures[fbo_tex as usize];
    let (storage, gen_now) = (t.mem.gpu, t.content_gen);
    let copies: Vec<u32> =
        g().tiled.iter().filter(|c| c.live() && c.key.base_gpu == storage).map(|c| c.key.content_gen).collect();
    assert!(kicks_after > kicks_before, "rgba {rgba}: the render into the texture ran before the sub-image");
    assert_eq!(copies, vec![gen_now], "rgba {rgba} between {between}: the last draw sampled the current stamp");
    let jobs = rig.jobs().len();
    assert_eq!(jobs, if between { 3 } else { 2 }, "rgba {rgba} between {between}: jobs");
    // SAFETY: live names; plain GL calls.
    unsafe {
        glDeleteTextures(1, &quads);
        glDeleteTextures(1, &fbo_tex);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &fbo);
        glDeleteFramebuffers(1, &screen);
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
        crate::gl::program::glUseProgram(0);
        crate::gl::program::glDeleteProgram(prog);
    }
}

/// dEQP fbo.render.resize.tex2d_rgba's first half (es2fFboRenderTest.cpp
/// ResizeTest::render), with FboRenderCase::iterate's clear of the default
/// framebuffer first, on the async OSAL whose GPU paints each job's target
/// when the job completes: the window's clear is a job still open when the
/// render into the FBO's texture arrives, so that draw ends it and the GL
/// core's stamp of the texture names the window's job; the render runs in
/// the next one. The draw to the window that samples the texture must see
/// the render's paint (FBOR5 1a61e2cef4bed60a: all 12 resize.tex2d_* still
/// failed with the target_kick wait alone).
///
/// Revert check: without the alias kept in InFlight the kept copy holds the
/// texture's contents before the render (not the render job's paint).
#[test]
fn resize_tex2d_first_half_samples_the_render_the_window_clear_job_was_stamped_for() {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::gl::state::*;
    use crate::gl::texture::*;
    let rig = Rig::new();
    with_platform(|p| (p.async_jobs, p.paint_targets) = (true, true));
    // SAFETY: serialised by the rig's TestGl guard (as gl_rig).
    unsafe { crate::backend::g_backend = &yakogl_backend_v3d };
    rig.init();
    let prog = gl_tex_program();
    let quads = vec![0x40u8; 64 * 64 * 3];
    let mut window = OsalGpuMem::ZERO;
    assert_eq!(crate::osal::gpu_alloc(V3D_HEAP_DATA, 4 * 4 * 4, OSAL_MEM_DATA, &mut window), 0);
    let host_surface = core::mem::replace(&mut rig.gl.ctx().default_surface, window);
    // SAFETY: plain GL calls on the rig's context with live buffers.
    unsafe {
        glViewport(0, 0, 4, 4);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        glBindTexture(GL_TEXTURE_2D, 1);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGB as i32,
            64,
            64,
            0,
            GL_RGB,
            GL_UNSIGNED_BYTE,
            quads.as_ptr() as *const c_void,
        );
        glBindTexture(GL_TEXTURE_2D, 2);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as i32, 128, 128, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glBindFramebuffer(GL_FRAMEBUFFER, 1);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 2, 0);
        glViewport(0, 0, 128, 128);
        glClear(GL_COLOR_BUFFER_BIT);
        glBindTexture(GL_TEXTURE_2D, 1);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        /* the FBO's texture to the window */
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, 4, 4);
        glBindTexture(GL_TEXTURE_2D, 2);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
    }
    let storage = rig.gl.ctx().objs().textures[2].mem.gpu;
    let jobs = rig.jobs();
    let render = jobs.iter().position(|j| u64::from(j.target.address) == storage).expect("the render's job");
    let size = 128 * 128 * 4;
    let copy = copy_image(storage, size);
    let want = 0x10 + (render + 1) as u8;
    assert!(
        copy.iter().all(|&b| b == want),
        "the window's draw sampled the render (job {}, paint 0x{want:x}): {:x?}",
        render + 1,
        &copy[..8]
    );
    rig.gl.ctx().default_surface = host_surface;
    // SAFETY: live names; plain GL calls.
    unsafe {
        glDeleteTextures(1, &1);
        glDeleteTextures(1, &2);
        glDeleteFramebuffers(1, &1);
        crate::gl::program::glUseProgram(0);
        crate::gl::program::glDeleteProgram(prog);
    }
}

/// dEQP fbo.render.color.blend_tex2d_rgba[_depth_component16]
/// (BlendTest::render), after FboRenderCase::iterate's window clear, on the
/// painting async OSAL: the FBO's clear of colour, depth and stencil and one
/// blended quad, then the texture drawn to the window. Returns the render
/// job's paint and what the window's draw sampled (the kept copy's first
/// byte). FBOR5 (the target_kick wait alone) failed the depth variants
/// (4 of 6) and passed the no-depth ones.
fn blend_replay(depth: bool) -> (u8, u8) {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::gl::state::*;
    use crate::gl::texture::*;
    let rig = Rig::new();
    with_platform(|p| (p.async_jobs, p.paint_targets) = (true, true));
    // SAFETY: serialised by the rig's TestGl guard (as gl_rig).
    unsafe { crate::backend::g_backend = &yakogl_backend_v3d };
    rig.init();
    let prog = gl_tex_program();
    let metaballs = vec![0x80u8; 64 * 64 * 4];
    let mut window = OsalGpuMem::ZERO;
    assert_eq!(crate::osal::gpu_alloc(V3D_HEAP_DATA, 4 * 4 * 4, OSAL_MEM_DATA, &mut window), 0);
    let host_surface = core::mem::replace(&mut rig.gl.ctx().default_surface, window);
    let mut rbs = Vec::new();
    // SAFETY: plain GL calls on the rig's context with live buffers.
    unsafe {
        glViewport(0, 0, 4, 4);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glBindTexture(GL_TEXTURE_2D, 1);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as i32,
            64,
            64,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            metaballs.as_ptr() as *const c_void,
        );
        glBindTexture(GL_TEXTURE_2D, 2);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as i32, 128, 128, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glBindFramebuffer(GL_FRAMEBUFFER, 1);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 2, 0);
        if depth {
            let rb = gl_renderbuffer(GL_DEPTH_COMPONENT16, 128, 128);
            glBindFramebuffer(GL_FRAMEBUFFER, 1);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
            rbs.push(rb);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glViewport(0, 0, 128, 128);
        glBindTexture(GL_TEXTURE_2D, 1);
        glClearColor(0.6, 0.0, 0.6, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glEnable(GL_BLEND);
        glBlendFuncSeparate(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA, GL_ZERO, GL_ONE);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        glDisable(GL_BLEND);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, 2);
        glViewport(0, 0, 4, 4);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
    }
    let storage = rig.gl.ctx().objs().textures[2].mem.gpu;
    let jobs = rig.jobs();
    /* the last job that rendered into the texture: its paint is the image */
    let render = jobs.iter().rposition(|j| u64::from(j.target.address) == storage).expect("the render's job");
    let copy = copy_image(storage, 128 * 128 * 4);
    rig.gl.ctx().default_surface = host_surface;
    // SAFETY: live names; plain GL calls.
    unsafe {
        glDeleteTextures(1, &1);
        glDeleteTextures(1, &2);
        glDeleteFramebuffers(1, &1);
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
        crate::gl::program::glUseProgram(0);
        crate::gl::program::glDeleteProgram(prog);
    }
    (0x10 + (render + 1) as u8, copy[0])
}

/// BlendTest with and without a depth attachment: the window's draw samples
/// the render into the texture in both (see `blend_replay`).
#[test]
fn blend_tex2d_with_and_without_depth_samples_the_render() {
    for depth in [false, true] {
        let (want, got) = blend_replay(depth);
        assert_eq!(got, want, "depth {depth}: the window's draw sampled the render's paint");
    }
}

/// dEQP's exact texsubimage call sequence (the QPA of 5ff3076c,
/// record_QPA_TSUB, after_render_tex2d_*): a clear of the default
/// framebuffer first (FboRenderCase::iterate), the draws by glDrawElements
/// from client arrays, the last one to the default framebuffer. The
/// window's clear is a job of its own, still open when the render into the
/// texture arrives; that draw ends it and is recorded into the next job,
/// while the GL core stamped the texture as rendered by the job it ended.
/// Returns the kicks before and after glTexSubImage2D, the texture's
/// storage, its stamp now, and the stamps of its kept copies.
fn texsubimage_deqp_exact(rgba: bool) -> (usize, usize, u64, u32, Vec<u32>) {
    use crate::gl::consts::*;
    use crate::gl::draw::*;
    use crate::gl::framebuffer::*;
    use crate::gl::state::*;
    use crate::gl::texture::*;
    use crate::gl::vertex::*;
    let rig = gl_rig();
    let prog = gl_tex_program();
    let fmt = if rgba { GL_RGBA } else { GL_RGB };
    let four_quads = vec![0x40u8; 64 * 64 * 3];
    let metaballs = vec![0xc0u8; 64 * 64 * if rgba { 4 } else { 3 }];
    let pos: [f32; 16] = [-1.0, -1.0, 0.0, 1.0, -1.0, 1.0, 0.0, 1.0, 1.0, -1.0, 0.0, 1.0, 1.0, 1.0, 0.0, 1.0];
    let idx: [u16; 6] = [0, 1, 2, 2, 1, 3];
    let (before, after);
    /* the default framebuffer in the V3D's address space: the rig's own
     * surface is host memory, which the backend refuses as a target */
    let mut window = OsalGpuMem::ZERO;
    assert_eq!(crate::osal::gpu_alloc(V3D_HEAP_DATA, 4 * 4 * 4, OSAL_MEM_DATA, &mut window), 0);
    let host_surface = core::mem::replace(&mut rig.gl.ctx().default_surface, window);
    // SAFETY: plain GL calls on the rig's context with live buffers.
    unsafe {
        glViewport(0, 0, 4, 4);
        glClearColor(0.125, 0.25, 0.5, 1.0);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT | GL_COLOR_BUFFER_BIT);
        crate::gl::program::glUseProgram(prog);
        glBindTexture(GL_TEXTURE_2D, 1);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        let quads = four_quads.as_ptr() as *const c_void;
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as i32, 64, 64, 0, GL_RGB, GL_UNSIGNED_BYTE, quads);
        glBindFramebuffer(GL_FRAMEBUFFER, 1);
        glBindTexture(GL_TEXTURE_2D, 2);
        glTexImage2D(GL_TEXTURE_2D, 0, fmt as i32, 128, 128, 0, fmt, GL_UNSIGNED_BYTE, core::ptr::null());
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR as i32);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, 2, 0);
        glViewport(0, 0, 128, 128);
        glBindTexture(GL_TEXTURE_2D, 1);
        glVertexAttribPointer(0, 4, GL_FLOAT, GL_FALSE as u8, 0, pos.as_ptr() as *const c_void);
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
        glDisableVertexAttribArray(0);
        glBindTexture(GL_TEXTURE_2D, 2);
        before = rig.kicks().len();
        glTexSubImage2D(GL_TEXTURE_2D, 0, 32, 32, 64, 64, fmt, GL_UNSIGNED_BYTE, metaballs.as_ptr() as *const c_void);
        after = rig.kicks().len();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, 4, 4);
        glEnableVertexAttribArray(0);
        glDrawElements(GL_TRIANGLES, 6, GL_UNSIGNED_SHORT, idx.as_ptr() as *const c_void);
        glDisableVertexAttribArray(0);
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
    }
    let t = &rig.gl.ctx().objs().textures[2];
    let (storage, gen_now) = (t.mem.gpu, t.content_gen);
    let copies: Vec<u32> =
        g().tiled.iter().filter(|c| c.live() && c.key.base_gpu == storage).map(|c| c.key.content_gen).collect();
    let mut window = core::mem::replace(&mut rig.gl.ctx().default_surface, host_surface);
    crate::osal::gpu_free(&mut window);
    // SAFETY: live names; plain GL calls.
    unsafe {
        glDeleteTextures(2, [1u32, 2].as_ptr());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &1);
        crate::gl::program::glUseProgram(0);
        crate::gl::program::glDeleteProgram(prog);
    }
    (before, after, storage, gen_now, copies)
}

/// FB2's texsubimage.after_render_tex2d_{rgb,rgba} (and between_render,
/// the same first half), on 8f2c90a3 and 5ff3076c: the QPA's pictures show
/// the render into the texture and no sub-image. The draw that renders into
/// the texture ends the window's clear job and is recorded into the next
/// job, but the GL core had stamped the texture with the ended job's id,
/// which has run: glTexSubImage2D saw nothing pending, wrote the storage,
/// and the render ran over it at the next flush. The render's job must be
/// submitted before the sub-image is written, and the last draw must sample
/// a copy of the texture's current stamp.
///
/// Revert check (v3d_kick_pending without open_alias): no kick at the
/// sub-image ("before 1, after 1").
#[test]
fn a_sub_image_after_a_render_that_switched_jobs_waits_for_that_render() {
    for rgba in [false, true] {
        let (before, after, storage, gen_now, copies) = texsubimage_deqp_exact(rgba);
        assert!(storage != 0);
        assert!(
            after > before,
            "rgba {rgba}: the render into the texture was not submitted before the sub-image \
             (kicks before {before}, after {after})"
        );
        assert_eq!(copies, vec![gen_now], "rgba {rgba}: the last draw sampled the current stamp");
    }
}

#[test]
fn a_sub_image_into_a_rendered_texture_is_ordered_after_the_render() {
    for rgba in [false, true] {
        for between in [false, true] {
            texsubimage_order(rgba, between);
        }
    }
}

/// The code guards (v3d-m5's contract, 2026-09-24): before a job runs a
/// program, each stage's code must end with the program-end THRSW on its
/// third-to-last word, decode word by word, and consume exactly as many
/// uniforms as its stream holds. Each fault is refused by name and counted,
/// with nothing recorded. A program that passed is remembered by its code's
/// address, length and checksum, so code that changes under the same
/// address is checked again. Revert checks: without check_program in
/// record() every broken program draws; a key without the checksum lets
/// the changed code draw.
#[test]
fn a_program_that_does_not_end_or_reads_past_its_stream_is_refused() {
    let rig = Rig::new();
    rig.init();
    let base = v3d_refused_draws();
    let refused = |p: &ShcShader, why: &str| {
        let mut d = rig.draw();
        d.fs = p;
        assert_eq!(draw(&d), -EINVAL, "{why}");
        assert_eq!(rig.logs_with(why), 1, "{why}: {:?}", rig.gl.logs());
    };
    /* the FS's end one word early: THRSW on the second-to-last word */
    let p = program();
    let words = rig_code(&p);
    let (end, last2) = (6 * 2, 7 * 2);
    words.swap(end, last2);
    words.swap(end + 1, last2 + 1);
    refused(&p, "the fragment shader's code does not end with the program-end thread switch");
    /* the VS reads one uniform more than its stream holds */
    let mut p = program();
    p.v3d.vs.num_uniforms = 0;
    refused(&p, "the vertex shader's uniform stream is not as long as its code reads");
    /* a CS word the decoder does not know */
    let p = program();
    let words = rig_code(&p);
    (words[136 / 4], words[136 / 4 + 1]) = (0xffff_ffff, 0xffff_ffff);
    refused(&p, "the coordinate shader's code holds a word the QPU decoder does not know");
    assert_eq!(v3d_refused_draws().wrapping_sub(base), 3, "each is counted");
    assert!(rig.draws().is_empty(), "nothing was recorded");
    /* a program that passed, then its words change: checked again */
    let good = program();
    let mut d = rig.draw();
    d.fs = &good;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let words = rig_code(&good);
    words.swap(6 * 2, 7 * 2);
    words.swap(6 * 2 + 1, 7 * 2 + 1);
    let mut e = rig.draw();
    (e.fs, e.do_clear) = (&good, false);
    assert_eq!(draw(&e), -EINVAL, "the changed code is checked again: {:?}", rig.gl.logs());
}

/// The words `with_code` leaked for `p`, to break on purpose.
#[allow(clippy::mut_from_ref)]
fn rig_code(p: &ShcShader) -> &mut [u32] {
    // SAFETY: `with_code` leaked these 50 words for the rig; nothing else
    // holds a reference while a test edits them.
    unsafe { std::slice::from_raw_parts_mut(p.usc as *mut u32, 50) }
}

/// A job that ends with an MMU fault (-EFAULT) prints what the GPU could have
/// fetched a wild address from, as it is in memory now: each texture record
/// against the bytes packed for it, and each program's GPU code against its
/// CPU copy. The struct.uniform runs on 8c8df5d1 faulted at VA 0x8080000,
/// past the carve-out, after other sampler cases had run. Here a record
/// byte and a code word are changed after recording: both are named.
/// Revert check: without fault_record() in wait_in_flight no line is
/// printed.
#[test]
fn a_job_that_faults_names_changed_records_and_code() {
    let rig = Rig::new();
    rig.init();
    let p = program();
    /* the code in GPU memory the rig can read, a copy of the CPU words */
    let mut code = OsalGpuMem::ZERO;
    assert_eq!(osal::gpu_alloc(V3D_HEAP_SHADER, 200, OSAL_MEM_DATA, &mut code), 0);
    // SAFETY: 200 bytes each: the allocation, and the program's 50 words.
    unsafe { core::ptr::copy_nonoverlapping(p.usc as *const u8, code.cpu as *mut u8, 200) };
    let mut t = texture(0, 16, 16, 1);
    t.content_gen = 81;
    let mut d = sampling(&rig, t);
    (d.fs, d.fs_usc_gpu) = (&p, code.gpu);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let rec = g().j.tex_states[0];
    let rec_cpu = cpu_of(rec as u64, V3D_TEX_STATE_BYTES as u32).expect("the record") as *mut u8;
    // SAFETY: inside the record and the code copy, both live.
    unsafe {
        *rec_cpu.add(4) ^= 0xff;
        *(code.cpu as *mut u32).add(3) ^= 1;
    }
    with_platform(|p| p.wait_rc = -EFAULT);
    /* the fault is seen at the wait, after the flush returned */
    assert_eq!(v3d_flush(), 0);
    assert_eq!(v3d_finish(), -EFAULT);
    with_platform(|p| p.wait_rc = 0);
    let logs = rig.gl.logs();
    assert!(logs.iter().any(|l| l.contains(&format!("texture record 0x{rec:08x} CHANGED from byte 4"))), "{logs:?}");
    assert!(logs.iter().any(|l| l.contains("DIFFERS from its CPU copy at word 3")), "{logs:?}");
    assert_eq!(
        logs.iter().filter(|l| l.contains("MMU fault: vertex array 0x") && l.contains("a buffer's")).count(),
        2,
        "the draw's two arrays: {logs:?}"
    );
    /* each stage's uniform stream, word by word as it is in memory, beside
     * the compiler's contents and data (the P0/P1 words the TMU took) */
    for (stage, list) in [('F', &FS_STREAM[..]), ('V', &VS_STREAM[..])] {
        assert!(
            logs.iter().any(|l| l.contains(&format!("MMU fault: {stage} uniforms at 0x"))
                && l.contains(&format!(", {} words", list.len()))),
            "{stage}: {logs:?}"
        );
        for (i, u) in list.iter().enumerate() {
            let want = if u.contents == 0 { u.data } else { 0x5a5a_0000 | i as u32 };
            let line = format!("{stage}[{i:2}] contents {:3} data 0x{:08x} -> 0x{want:08x}", u.contents, u.data);
            assert!(logs.iter().any(|l| l.contains(&line)), "{line}: {logs:?}");
        }
    }
    osal::gpu_free(&mut code);
}

/// A tiling that writes past its image breaks the canary in the kept copy's
/// slack, and says so with the copy's shape (the struct.uniform MMU faults:
/// a kept copy's neighbour could be another texture or a program's code).
/// A fill that stays inside prints nothing. Revert check: without the check
/// after fill() the overrun goes unsaid.
#[test]
fn a_tiling_that_writes_past_its_image_is_named() {
    let rig = Rig::new();
    rig.init();
    let key = TexKey {
        base_gpu: 0x0100_0000,
        width: 16,
        height: 16,
        levels: 1,
        bgra: 0,
        format: GLES_TEXFMT_RGBA8,
        content_gen: 5,
        layers: 1,
        depth_id: 0,
    };
    let inside = tiled_copy(g(), key, 1000, 1024, TILE_KEPT_AT_DRAW, |p, _| {
        // SAFETY: the image's 1000 bytes.
        unsafe { core::ptr::write_bytes(p, 0x11, 1000) };
        true
    });
    assert!(inside.is_ok());
    assert_eq!(rig.logs_with("written past its end"), 0);
    let past = TexKey { content_gen: 6, base_gpu: 0x0200_0000, ..key };
    let over = tiled_copy(g(), past, 1000, 1024, TILE_KEPT_AT_DRAW, |p, _| {
        // SAFETY: the test writes one byte into the slack on purpose.
        unsafe { core::ptr::write_bytes(p, 0x11, 1001) };
        true
    });
    assert!(over.is_ok());
    assert_eq!(rig.logs_with("a tiled copy of 1000 bytes was written past its end (from byte 0 after it"), 1);
    assert_eq!(g().overruns, 1);
}

/// An OSAL that maps no CPU view for an allocation that has a GPU address:
/// the kept copy is refused as out of memory, the allocation is freed and
/// the fill never runs. Revert check: without the null check the fill is
/// handed a pointer based at null.
#[test]
fn a_tiled_copy_without_a_cpu_view_is_refused_and_freed() {
    let rig = Rig::new();
    rig.init();
    let live = with_platform(|p| {
        p.null_cpu = true;
        p.allocs.len()
    });
    let key = TexKey {
        base_gpu: 0x0100_0000,
        width: 16,
        height: 16,
        levels: 1,
        bgra: 0,
        format: GLES_TEXFMT_RGBA8,
        content_gen: 5,
        layers: 1,
        depth_id: 0,
    };
    let r = tiled_copy(g(), key, 1000, 1024, TILE_KEPT_AT_DRAW, |_, _| panic!("filled a copy with no CPU view"));
    assert_eq!(r, Err(KeptMiss::NoMemory));
    with_platform(|p| {
        assert_eq!(p.allocs.len(), live, "the allocation leaked");
        assert!(p.faults.is_empty(), "{:?}", p.faults);
    });
}

/* ---- femtovg's layer framebuffers (the DS cap regression) ----------------- */

/// Layer `i` of the femtovg replay: its own colour target and a
/// STENCIL_INDEX8 renderbuffer on the stencil point only (ds_id = the
/// stencil's name, no depth), as femtovg's `Framebuffer::new` makes one per
/// image render target. The draws of one femtovg pass over it, in order:
/// `clear_rect` (colour and stencil, a scissor over the whole image, which
/// the GL core sends as a whole clear), then a concave fill: the stencil
/// pass (INCR_WRAP front, DECR_WRAP back, colour masked) and the cover pass
/// (stencil NOTEQUAL 0, every op ZERO).
fn femtovg_layer(rig: &Rig, i: u32) -> [GlesDraw; 3] {
    let mut base = rig.draw();
    base.target_gpu = TARGET + 0x10_0000 * (i as u64 + 1);
    (base.ds_persist, base.ds_id) = (true, 0xb1 + 2 * i);
    (base.depth_id, base.stencil_id) = (0, 0x300 + i);
    let mut clear = base;
    (clear.do_clear, clear.do_clear_stencil, clear.clear_stencil, clear.index_count) = (true, true, 0, 0);
    let mut stencil = base;
    stencil.do_clear = false;
    (stencil.stencil, stencil.stencil_write_mask, stencil.stencil_back_write_mask) = (true, 0xff, 0xff);
    (stencil.stencil_func, stencil.stencil_back_func) = (GLES_CMP_ALWAYS, GLES_CMP_ALWAYS);
    (stencil.stencil_cmp_mask, stencil.stencil_back_cmp_mask) = (0xff, 0xff);
    (stencil.stencil_dppass, stencil.stencil_back_dppass) = (GLES_SOP_INCR_WRAP, GLES_SOP_DECR_WRAP);
    stencil.color_mask = 0;
    let mut cover = stencil;
    cover.color_mask = 0xf;
    (cover.stencil_func, cover.stencil_back_func) = (GLES_CMP_NOTEQUAL, GLES_CMP_NOTEQUAL);
    (cover.stencil_sfail, cover.stencil_dpfail, cover.stencil_dppass) = (GLES_SOP_ZERO, GLES_SOP_ZERO, GLES_SOP_ZERO);
    (cover.stencil_back_sfail, cover.stencil_back_dpfail, cover.stencil_back_dppass) =
        (GLES_SOP_ZERO, GLES_SOP_ZERO, GLES_SOP_ZERO);
    [clear, stencil, cover]
}

/// Draw the cluster's frame 1 on the host: 13 femtovg layers, each filled
/// with a concave path through its own stencil renderbuffer, then each
/// layer's stencil read again with no clear (a draw testing it). Returns
/// each layer's stencil buffer (address, bytes) and asserts what does not
/// depend on the buffer's kind: no draw refused, 13 buffers, each layer's
/// stencil Stored and loaded again from its own buffer, its depth (it has
/// none) Untouched, and the "zs slots/bytes peak" the diag line prints.
fn femtovg_frame_one(rig: &Rig, s8: bool) -> Vec<(u32, usize)> {
    let layers: Vec<_> = (0..13).map(|i| femtovg_layer(rig, i)).collect();
    let mut rcs = Vec::new();
    let mut kept = Vec::new();
    for (i, l) in layers.iter().enumerate() {
        rcs.extend(l.iter().map(draw));
        assert_eq!(v3d_flush(), 0);
        let j = job_clear(rig, rig.jobs().len() - 1);
        let b = g().zsbufs.iter().find(|b| b.live() && b.id == l[0].ds_id).copied().expect("a buffer of its own");
        assert_eq!((b.z, b.s, b.s8), (ZsKnown::Untouched, ZsKnown::Stored, s8), "layer {i}: stencil kept, no depth");
        if s8 {
            /* the stencil half alone: no Z buffer, the S8 one stored */
            assert!(j.zs_address == 0 && j.s_address == b.address && j.s_s8, "layer {i}: {j:?}");
            assert!(j.s_store && !j.s_load && !j.zs_store, "layer {i}: cleared, filled, stored: {j:?}");
        } else {
            assert!(j.zs_address == b.address && j.s_address == 0 && !j.s_s8, "layer {i}: {j:?}");
            assert!(j.zs_store && !j.zs_load, "layer {i}: cleared, filled, stored: {j:?}");
        }
        kept.push((b.address, b.mem.size));
    }
    let logs = rig.gl.logs();
    assert_eq!(rcs.iter().filter(|&&rc| rc != 0).count(), 0, "no draw refused: {rcs:?}");
    assert_eq!(logs.iter().filter(|l| l.contains(ds_retention::REFUSED)).count(), 0, "{logs:?}");
    let mut distinct: Vec<_> = kept.iter().map(|k| k.0).collect();
    distinct.sort_unstable();
    distinct.dedup();
    assert_eq!(distinct.len(), 13, "13 buffers: {kept:x?}");
    /* each layer's stencil read again: loaded from its own buffer */
    for (i, l) in layers.iter().enumerate() {
        let mut test = l[2];
        (test.stencil_dppass, test.stencil_back_dppass) = (GLES_SOP_KEEP, GLES_SOP_KEEP);
        (test.stencil_sfail, test.stencil_back_sfail) = (GLES_SOP_KEEP, GLES_SOP_KEEP);
        (test.stencil_dpfail, test.stencil_back_dpfail) = (GLES_SOP_KEEP, GLES_SOP_KEEP);
        assert_eq!(draw(&test), 0, "layer {i}: {:?}", rig.gl.logs());
        assert_eq!(v3d_flush(), 0);
        let j = job_clear(rig, rig.jobs().len() - 1);
        let (address, loads) = if s8 { (j.s_address, j.s_load) } else { (j.zs_address, j.zs_load) };
        assert!(loads, "layer {i}: its stencil is loaded");
        assert_eq!(address, kept[i].0, "layer {i}: from its own buffer");
    }
    let (mut peak, mut slots, mut bytes) = (0u32, 0u32, 0u64);
    // SAFETY: three writable values.
    unsafe { v3d_zs_usage(&mut peak, &mut slots, &mut bytes) };
    assert_eq!((peak, slots), (13, ZS_BUFS as u32), "the peak the diag line prints");
    assert_eq!(bytes, kept.iter().map(|k| k.1 as u64).sum::<u64>(), "13 buffers at once");
    kept
}

/// The cluster's frame 1 (`femtovg_frame_one`) with D24S8 buffers: with 8
/// slots the 9th to 13th layers' draws were refused (-ENOMEM, the Pi 4's 21
/// x GL_OUT_OF_MEMORY in record_DEMOG1_cluster_demog1.txt, objects 0xb7..);
/// now none is, and each keeps its stencil in a D24S8 of its own.
///
/// Revert checks: ZS_BUFS back to 8 fails at layer 8 (its draws refused, no
/// buffer of its own); the store marking both planes Stored fails the depth
/// check at layer 0.
#[test]
fn thirteen_femtovg_layers_with_concave_fills_all_keep_their_stencil() {
    let rig = Rig::new();
    rig.init();
    let kept = femtovg_frame_one(&rig, false);
    /* 64x64 at 4 bytes a pixel, + the 4 KiB alignment */
    let mut l = V3dTexLayout::default();
    // SAFETY: `l` is a live layout to fill.
    assert!(unsafe { v3d_tex_layout(64, 64, 1, 4, &mut l) });
    assert!(kept.iter().all(|k| k.1 == l.size as usize + 4096), "{kept:?}");
}

/// The same frame with S8 stencil halves (`V3dState::s8_stencil`, depth and
/// stencil apart): each layer's stencil is an S8 buffer, the stencil half
/// alone (no Z buffer), loaded and stored as buffer STENCIL, format S8; a
/// quarter of the bytes. 64x64 S8 is 64 * 64 bytes (UIF, 16-row blocks)
/// + 4 KiB, against 64 * 64 * 4 + 4 KiB.
///
/// Revert checks: zs_keys not routing a stencil-only id to the stencil
/// half keeps zs_address set (layer 0); the layout at 4 bytes fails the
/// byte count; `s_s8` left false fails layer 0.
#[test]
fn thirteen_femtovg_layers_in_s8_buffers_take_a_quarter_of_the_bytes() {
    let rig = Rig::new();
    with_platform(|p| (p.split_zs, p.s8_stencil) = (Some(1), Some(1)));
    rig.init();
    let kept = femtovg_frame_one(&rig, true);
    let (mut s8, mut d24s8) = (V3dTexLayout::default(), V3dTexLayout::default());
    // SAFETY: live layouts to fill.
    assert!(unsafe { v3d_tex_layout(64, 64, 1, 1, &mut s8) && v3d_tex_layout(64, 64, 1, 4, &mut d24s8) });
    assert_eq!((s8.size, d24s8.size), (64 * 64, 64 * 64 * 4), "the two layouts' bytes");
    assert!(kept.iter().all(|k| k.1 == s8.size as usize + 4096), "{kept:?}");
    /* the packets' UIF height: 16-row blocks at 1 byte a pixel, 8-row at 4
     * (v3dx_rcl.c:62-64; the rig's layout is not UIF, so the rule is read
     * on a UIF slice; the C layout itself is checked by tex_check) */
    let uif = V3dTexSlice { tiling: V3D_TEX_TILING_UIF_XOR, padded_height: 64, ..V3dTexSlice::default() };
    assert_eq!(zs_layout(&uif, 1), (V3D_TEX_TILING_UIF_NO_XOR, 4));
    assert_eq!(zs_layout(&uif, 4), (V3D_TEX_TILING_UIF_NO_XOR, 8));
}

/// A stencil-only framebuffer with S8 halves that gets a depth
/// renderbuffer later (a new attachment set, ES 2.0.25 4.4.5, and a new
/// ds_id): its stencil stays in the S8 buffer under the stencil
/// renderbuffer's key, which the new job loads, and the depth gets a D24S8
/// of its own (the halves apart). The stencil's contents belong to the
/// renderbuffer (4.4.3), whatever the framebuffer's other attachments.
#[test]
fn a_depth_attached_later_keeps_the_s8_stencil() {
    let rig = Rig::new();
    with_platform(|p| (p.split_zs, p.s8_stencil) = (Some(1), Some(1)));
    rig.init();
    let [clear, stencil, cover] = femtovg_layer(&rig, 0);
    for d in [clear, stencil, cover] {
        assert_eq!(draw(&d), 0);
    }
    assert_eq!(v3d_flush(), 0);
    let first = job_clear(&rig, rig.jobs().len() - 1);
    assert!(first.s_s8 && first.s_store && first.zs_address == 0, "{first:?}");
    /* the depth renderbuffer 7 attached: ds_id names both */
    let mut both = cover;
    (both.ds_id, both.depth_id) = ((7 << 16) | cover.ds_id, 0x700);
    (both.depth_test, both.depth_write, both.depth_func) = (true, true, GLES_CMP_LESS);
    both.do_clear_depth = true;
    assert_eq!(draw(&both), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let j = job_clear(&rig, rig.jobs().len() - 1);
    assert_eq!(j.s_address, first.s_address, "the stencil's S8 buffer, found by its key");
    assert!(j.s_s8 && j.s_load, "and its stencil loaded: {j:?}");
    assert!(j.zs_address != 0 && j.zs_address != j.s_address && j.zs_store, "a depth buffer of its own: {j:?}");
    let depth = g().zsbufs.iter().find(|b| b.live() && b.id == 7 << 16).copied().expect("the depth's buffer");
    assert!(!depth.s8, "the depth half is D24S8");
}

/// With the carve-out full, a new id's buffer is made from the memory of a
/// buffer that holds no contents GL keeps (a whole clear, nothing drawn
/// since): that one is let go, its clear value parked, rather than the
/// draw refused. The clear value comes back when its id returns.
#[test]
fn a_full_carve_out_gives_up_a_cleared_buffer_before_refusing() {
    let rig = Rig::new();
    rig.init();
    fbo_depth_job(&rig, 0, true); /* id 0: stored depth, kept */
    let mut clear = kept_depth_draw(&rig, 2 << 16);
    (clear.target_gpu, clear.depth_id, clear.stencil_id, clear.do_clear) = (TARGET + 0x10_0000 * 2, 101, 101, false);
    (clear.do_clear_depth, clear.clear_depth, clear.index_count) = (true, 0.25, 0);
    assert_eq!(draw(&clear), 0);
    assert_eq!(v3d_flush(), 0);
    /* no room for one more buffer */
    with_platform(|p| p.capacity = Some(p.used()));
    let rc = draw(&{
        let mut d = kept_depth_draw(&rig, 3 << 16);
        (d.target_gpu, d.depth_id, d.stencil_id, d.do_clear, d.do_clear_depth) =
            (TARGET + 0x10_0000 * 3, 102, 102, false, true);
        d
    });
    assert_eq!(rc, 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    assert!(g().zsbufs.iter().any(|b| b.live() && b.id == 1 << 16), "id 0's stored depth kept");
    assert!(!g().zsbufs.iter().any(|b| b.live() && b.id == 2 << 16), "id 1's cleared buffer given up");
    assert_eq!(g().zs_parked.len(), 1, "its clear value parked");
}

/* ---- depth_resolve: GL_OES_depth_texture's rendered depth into the texels ---- */

/// The buffer `depth_resolve` reads for depth object `id`, after a draw that
/// stored depth into it (one buffer for the id, its depth half owned by `id`).
fn depth_object_draw(rig: &Rig, id: u32, clear_only: Option<f32>) -> ZsBuf {
    let mut d = kept_depth_draw(rig, 9 << 16);
    (d.depth_id, d.stencil_id) = (id, 0);
    d.do_clear_depth = true;
    if let Some(v) = clear_only {
        (d.index_count, d.clear_depth) = (0, v);
    }
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    *g().zsbufs.iter().find(|b| b.live() && b.owners[0] == id).expect("a buffer owned by the depth object")
}

/// `depth_resolve` reads the depth a job stored for a depth texture as its
/// floats: each D24S8 word's bits 31..8 over 0xffffff (Mesa's
/// S8_UINT_Z24_UNORM, the stencil byte ignored), the buffer read back out of
/// Mesa's zsbuf layout (UIF without the XOR) by Mesa's v3d_load_tiled_image,
/// row 0 at GL's y 0; the pitch's padding is not written.
///
/// Host tests run on the fakes' packed layout (`v3d_tex_store_level` /
/// `v3d_tex_load_box` above): this pins the lookup by owner, the word's
/// reading, the rows and the pitch; Mesa's tiling itself is tex_check's.
///
/// Revert checks: reading the word as a float, or its low 24 bits, fails
/// the values; finding the buffer by its key instead of its depth owner
/// answers 1.
#[test]
fn depth_resolve_reads_the_stored_d24s8_back_as_floats() {
    let rig = Rig::new();
    rig.init();
    let id = 0x4000_0064; /* a depth texture's id form */
    let b = depth_object_draw(&rig, id, None);
    assert_eq!(b.z, ZsKnown::Stored, "cleared and drawn: stored");
    assert!(job_clear(&rig, 0).zs_store);
    /* plant words through Mesa's tiling, in the layout the jobs store */
    let mut l = V3dTexLayout::default();
    // SAFETY: a live layout to fill.
    assert!(unsafe { v3d_tex_layout(b.w, b.h, 1, 4, &mut l) });
    if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
        l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
    }
    let word = |x: u32, y: u32| ((x + 64 * y + 1) << 8) | 0xab;
    let words: Vec<u32> = (0..64 * 64).map(|i| word(i % 64, i / 64)).collect();
    let image = (b.mem.cpu as *mut u8).wrapping_add((u64::from(b.address) - b.mem.gpu) as usize);
    // SAFETY: the buffer's live mapping holds the laid-out level; `words` is 64 rows of 256 bytes.
    unsafe { v3d_tex_store_level(image, &l, 0, words.as_ptr() as *const u8, 256) };
    let mut t = std::vec![7.0f32; 9 * 8];
    // SAFETY: 8 rows of 9 floats.
    assert_eq!(unsafe { v3d_depth_resolve(id, t.as_mut_ptr(), 8, 8, 9) }, 0);
    for y in 0..8u32 {
        for x in 0..8u32 {
            let want = (word(x, y) >> 8) as f32 / 16_777_215.0;
            assert_eq!(t[(y * 9 + x) as usize], want, "({x}, {y})");
        }
        assert_eq!(t[(y * 9 + 8) as usize], 7.0, "row {y}: the pitch's padding is not written");
    }
    /* a level smaller than the buffer reads its corner; one larger reads the buffer's part */
    let mut big = std::vec![7.0f32; 70 * 66];
    // SAFETY: 66 rows of 70 floats.
    assert_eq!(unsafe { v3d_depth_resolve(id, big.as_mut_ptr(), 66, 66, 70) }, 0);
    assert_eq!(big[63 * 70 + 63], (word(63, 63) >> 8) as f32 / 16_777_215.0);
    assert_eq!((big[64], big[64 * 70]), (7.0, 7.0), "outside the buffer: left as it was");
}

/// A draw that carries depth texels (`GlesDraw::depth_texels`: the CPU
/// wrote the depth texture after a job stored its depth) writes them into
/// the depth buffer before its job begins, through Mesa's tiling, as D24S8
/// words (depth in bits 31..8), the depth past the texels as it was, and
/// the stencil byte 0 (no stencil object: its half is Untouched, any value
/// is right, and a new job would start it at 0); the buffer stays Stored, and the
/// draw's job loads it, so the draw tests against the texels. The job open
/// on that target ends first; `depth_resolve` then reads the texels back
/// bit for bit (they are 24-bit depths, as resolved texels are).
///
/// Revert check: without `seed_depth` the buffer keeps the planted words
/// and the resolve answers them.
#[test]
fn a_draw_with_depth_texels_writes_them_into_its_buffer() {
    let rig = Rig::new();
    rig.init();
    let id = 0x4000_0067; /* a depth texture's id form */
    let b = depth_object_draw(&rig, id, None);
    let l = texture::rt_layout(b.w, b.h, 1).expect("the buffer's layout");
    let image = (b.mem.cpu as *mut u8).wrapping_add((u64::from(b.address) - b.mem.gpu) as usize);
    let planted: Vec<u32> = (0..64 * 64).map(|i| ((i + 1) << 8) | 0xab).collect();
    // SAFETY: the buffer's live mapping holds the laid-out level; `planted` is 64 rows of 256 bytes.
    unsafe { v3d_tex_store_level(image, &l, 0, planted.as_ptr() as *const u8, 256) };
    /* the job open on the target ends before the texels go in */
    let mut d = kept_depth_draw(&rig, 9 << 16);
    (d.depth_id, d.stencil_id) = (id, 0);
    /* no clear: a clear after draws would begin a job of its own anyway */
    (d.do_clear, d.do_clear_depth, d.do_clear_stencil) = (false, false, false);
    assert_eq!(draw(&d), 0);
    /* 8 rows of 8 texels at a pitch of 9 (the ninth is padding) */
    let texels: Vec<f32> =
        (0..72u32).map(|i| if i % 9 == 8 { 9.0 } else { ds_retention::unorm24_depth(i * 0x3_1337 + 5) }).collect();
    (d.ds_w, d.ds_h, d.depth_texels, d.depth_texels_pitch) = (8, 8, texels.as_ptr(), 9);
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(g().zsbufs.iter().find(|z| z.live() && z.owners[0] == id).map(|z| z.z), Some(ZsKnown::Stored));
    let mut words = std::vec![0u32; 64 * 64];
    // SAFETY: as above; `words` is 64 rows of 256 bytes.
    assert!(unsafe { v3d_tex_load_box(image, &l, 0, 0, 0, 64, 64, words.as_mut_ptr() as *mut u8, 256) });
    for y in 0..64usize {
        for x in 0..64usize {
            let want = if x < 8 && y < 8 {
                ds_retention::depth_unorm24(texels[y * 9 + x]) << 8
            } else {
                planted[y * 64 + x] & 0xffff_ff00
            };
            assert_eq!(words[y * 64 + x], want, "({x}, {y})");
        }
    }
    d.depth_texels = core::ptr::null();
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 3, "the stored job, the open one, then the draws after the texels in one job: {jobs:?}");
    assert!(jobs[2].clear.expect("ended").zs_load, "the draws' job loads the buffer");
    let mut t = [7.0f32; 72];
    // SAFETY: 8 rows of 9 floats.
    assert_eq!(unsafe { v3d_depth_resolve(id, t.as_mut_ptr(), 8, 8, 9) }, 0);
    for (i, (&got, &want)) in t.iter().zip(&texels).enumerate() {
        let want = if i % 9 == 8 { 7.0 } else { want };
        assert_eq!(got.to_bits(), want.to_bits(), "texel {i}");
    }
}

/// glmark2's shadow pass (gl_test case 177 zonly): a framebuffer whose only
/// attachment is a depth texture reaches the backend with no colour target
/// (`target_gpu` 0, `DRAWS_WITHOUT_COLOUR`). The job is opened at target
/// address 0, which the job builder loads and stores no render target for
/// (v3d_job.c, job_check part 18), and the depth texture's buffer is
/// stored for the read back that samples it. A window draw after it is a
/// job of its own. With no depth/stencil buffer, or at 4x, the draw is
/// refused by name.
///
/// Revert checks: refusing target 0 as "outside the 32-bit address space"
/// fails the first draw; dropping the no-depth/stencil refusal draws the
/// `none` draw (0, not -EINVAL).
#[test]
fn a_depth_texture_alone_is_drawn_into_with_no_colour_target() {
    let rig = Rig::new();
    rig.init();
    let id = 0x4000_0066; /* a depth texture's id form */
    let mut d = kept_depth_draw(&rig, 9 << 16);
    (d.depth_id, d.stencil_id) = (id, 0);
    (d.target_gpu, d.target_w, d.target_h, d.target_pitch) = (0, 128, 128, 0);
    (d.ds_w, d.ds_h) = (128, 128);
    (d.color_write, d.color_mask) = (false, 0);
    d.num_targets = 1;
    d.targets[0].gpu = 0;
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    /* the window draw after it: another target, another job */
    let w = rig.draw();
    assert_eq!(draw(&w), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let jobs = rig.jobs();
    assert_eq!(jobs.len(), 2, "{jobs:?}");
    let (t, c) = (jobs[0].target, jobs[0].clear.expect("the shadow job ended"));
    assert_eq!((t.address, t.width, t.height), (0, 128, 128), "no colour buffer, the depth texture's size");
    assert!(!t.msaa);
    assert!(c.zs_address != 0 && c.zs_store, "the depth texture's buffer is stored: {c:?}");
    assert_eq!(jobs[1].target.address, TARGET as u32, "the window's job");
    let b = *g().zsbufs.iter().find(|b| b.live() && b.owners[0] == id).expect("a buffer owned by the depth texture");
    assert_eq!(b.z, ZsKnown::Stored, "cleared and drawn: stored for the read back");
    let mut px = [7.0f32; 4];
    // SAFETY: 1 row of 4 floats.
    assert_eq!(unsafe { v3d_depth_resolve(id, px.as_mut_ptr(), 4, 1, 4) }, 0, "the depth reads back");

    /* no colour and no depth/stencil buffer; no colour at 4x */
    let mut none = d;
    none.ds_persist = false;
    assert_eq!(draw(&none), -EINVAL);
    let mut ms = d;
    ms.samples = 4;
    assert_eq!(draw(&ms), -EINVAL);
    let logs = rig.gl.logs().join("\n");
    for why in ["a draw with no colour target and no depth/stencil buffer", "4x multisampling with no colour target"] {
        assert!(logs.contains(why), "{why:?} not in {logs}");
    }
    assert_eq!(rig.jobs().len(), 2, "the refused draws recorded nothing");
}

/// A buffer the last job only cleared holds that value everywhere; a depth
/// object nothing holds, a buffer whose depth half another object owns, and
/// a call whose width is over its pitch answer 1 and write nothing.
#[test]
fn depth_resolve_of_a_clear_and_of_what_it_does_not_hold() {
    let rig = Rig::new();
    rig.init();
    let id = 0x4000_0065;
    let b = depth_object_draw(&rig, id, Some(0.25));
    assert_eq!(b.z, ZsKnown::Cleared(0.25));
    let mut t = [7.0f32; 9 * 8];
    // SAFETY: 8 rows of 9 floats.
    assert_eq!(unsafe { v3d_depth_resolve(id, t.as_mut_ptr(), 8, 8, 9) }, 0);
    assert_eq!((t[0], t[7 * 9 + 7], t[8]), (0.25, 0.25, 7.0));
    let mut u = [7.0f32; 9 * 8];
    // SAFETY: as above (the last call's width is over its pitch: refused before any write).
    unsafe {
        assert_eq!(v3d_depth_resolve(id + 1, u.as_mut_ptr(), 8, 8, 9), 1, "nothing held for it");
        assert_eq!(v3d_depth_resolve(0x4000_0066, u.as_mut_ptr(), 8, 8, 9), 1);
        assert_eq!(v3d_depth_resolve(id, u.as_mut_ptr(), 10, 8, 9), 1, "w > pitch");
    }
    assert!(u.iter().all(|&v| v == 7.0));
}

/* ---- depth textures sampled from their depth buffer (pi4-depth-resolve-gpu) ---- */

/// (bindings sampled from a buffer, CPU resolves, draws refused) since init.
fn depth_samples() -> (u32, u32, u32) {
    let (mut d, mut r, mut f) = (0u32, 0u32, 0u32);
    // SAFETY: live out-parameters.
    unsafe { v3d_depth_samples(&mut d, &mut r, &mut f) };
    (d, r, f)
}

/// A program whose fragment look-up on unit 0 takes the 32-bit TMU output
/// (P1 bit 0), as the GL core compiles one for an R32F binding.
fn ret32_program() -> ShcShader {
    static FS_32: [ShcV3dUniform; 2] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0xf },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0x5 },
    ];
    let mut p = program();
    (p.v3d.fs.uniforms, p.v3d.fs.num_uniforms) = (FS_32.as_ptr(), 2);
    with_code(&mut p);
    p
}

/// The draw of glmark2's ground: a 64x64 R32F depth texture on unit 0,
/// its rendered depth to be read from the buffer of depth object `id`
/// (`GlesTextureBind::depth_id`, as the GL core sets it after
/// `v3d_depth_sampled` answered 1).
fn depth_sampling_draw(rig: &Rig, p: &ShcShader, id: u32) -> GlesDraw {
    let mut t = narrow_texture(64, 64, 1, GLES_TEXFMT_R32F);
    (t.unit, t.content_gen, t.depth_id) = (0, 7, id);
    let mut d = sampling(rig, t);
    d.fs = p;
    d
}

/// glmark2's shadow and refract on the Pi 4 (docs/notes/pi4-glmark2-cpu.md):
/// the depth texture a job stored is sampled by the next draw from that
/// D24S8 buffer, as the TMU's DEPTH24_X8 (the texture state's base is the
/// buffer's address, its type Mesa's row for S8_UINT_Z24_UNORM, one level,
/// the GL depth swizzle), with no CPU resolve into the texels and no copy
/// tiled for the TMU: the two passes over 7.68 MB a frame that were all of
/// shadow's frame. `v3d_depth_sampled` says so first (1 for the stored
/// buffer at the texture's size, 0 for another size and for an object no
/// buffer holds), and the CPU resolve still reads the buffer for the CPU's
/// own paths, counted apart.
///
/// Revert checks: `tex_state_for` ignoring `depth_id` tiles a copy (tilings
/// and the kept copies move, the record's base is a copy's address);
/// `depth_buffer_state` keeping R32F fails the type.
#[test]
fn a_depth_texture_is_sampled_from_its_stored_buffer_without_a_resolve_or_a_copy() {
    let rig = Rig::new();
    rig.init();
    let id = 0x4000_0070;
    let b = depth_object_draw(&rig, id, None);
    assert_eq!(b.z, ZsKnown::Stored);
    assert!(v3d_depth_sampled(id, 64, 64), "stored at the texture's size: sampled from the buffer");
    assert!(!v3d_depth_sampled(id, 32, 64), "another size: resolved");
    assert!(!v3d_depth_sampled(id + 1, 64, 64), "no buffer: resolved");
    assert!(!v3d_depth_sampled(0, 64, 64));
    let (tilings0, copies0) = (tilings(), tiled().0);
    let p = ret32_program();
    for frame in 1..=2u32 {
        if frame == 2 {
            /* the next frame's depth pass renders again, then the ground draw */
            depth_object_draw(&rig, id, None);
            assert!(v3d_depth_sampled(id, 64, 64));
        }
        let d = depth_sampling_draw(&rig, &p, id);
        assert_eq!(draw(&d), 0, "frame {frame}: {:?}", rig.gl.logs());
        assert_eq!(v3d_flush(), 0);
        let st = rig.draws().last().unwrap().tmu[0].tex_state;
        let w = words(&rig, st, 6);
        assert_eq!(w[0], b.address, "frame {frame}: the record's base is the depth buffer");
        assert_eq!(w[3] >> FAKE_TYPE_SHIFT, V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8, "frame {frame}: DEPTH24_X8");
        assert_eq!(w[3] & 0xffff, 0, "frame {frame}: one level");
        assert_eq!(w[4].to_le_bytes(), [2, 0, 0, 1], "frame {frame}: the GL depth swizzle (d, 0, 0, 1)");
        assert_eq!((tilings(), tiled().0), (tilings0, copies0), "frame {frame}: nothing tiled, no copy kept");
        assert_eq!(depth_samples(), (frame, 0, 0), "frame {frame}: sampled from the buffer, no resolve");
    }
    assert_eq!(rig.logs_with("depth texture is sampled from its depth buffer (DEPTH24_X8)"), 1, "logged once");
    /* the CPU's own path still reads the buffer, and is counted apart */
    let mut t = [7.0f32; 64 * 64];
    // SAFETY: 64 rows of 64 floats.
    assert_eq!(unsafe { v3d_depth_resolve(id, t.as_mut_ptr(), 64, 64, 64) }, 0);
    assert_eq!(depth_samples(), (2, 1, 0));
    /* the same binding on the ES 2.0 surface: (d, d, d, 1) */
    let mut d = depth_sampling_draw(&rig, &p, id);
    d.textures[0].depth_luminance = 1;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    assert_eq!(v3d_flush(), 0);
    let w = words(&rig, rig.draws().last().unwrap().tmu[0].tex_state, 6);
    assert_eq!((w[0], w[3] >> FAKE_TYPE_SHIFT, w[4].to_le_bytes()), (b.address, 24, [2, 2, 2, 1]));
    assert_eq!(depth_samples(), (3, 1, 0));
}

/// The question is a reading of the buffers' history, and nothing is
/// submitted for it: while the depth pass is still the open job, its depth
/// is not stored yet and the answer is no; the GL core submits that job
/// first (`texture::depth_for_draw` flushes the pending kick, as glmark2's
/// target change would), and then the answer is yes, with no kick of the
/// question's own. A buffer the last job only cleared holds its depth in
/// the tile buffer alone and is not sampled (the CPU resolve writes the
/// clear value, as before), and nor is one at 4x.
///
/// Revert check: a question that submits the open job itself moves the
/// kick count here, and links the C job builder into every GL-core test
/// binary of this configuration.
#[test]
fn the_question_reads_the_buffers_history_and_a_cleared_buffer_is_not_sampled() {
    let rig = Rig::new();
    rig.init();
    let id = 0x4000_0071;
    let mut d = kept_depth_draw(&rig, 9 << 16);
    (d.depth_id, d.stencil_id) = (id, 0);
    d.do_clear_depth = true;
    assert_eq!(draw(&d), 0, "{:?}", rig.gl.logs());
    let kicks = rig.kicks().len();
    assert!(g().j.open, "the depth pass is the open job");
    assert!(!v3d_depth_sampled(id, 64, 64), "the open job has stored nothing yet");
    assert_eq!(rig.kicks().len(), kicks, "the question submitted nothing");
    assert_eq!(v3d_flush(), 0, "the GL core's submit of the pending kick");
    assert!(v3d_depth_sampled(id, 64, 64), "the submitted job stored the depth");
    assert_eq!(rig.kicks().len(), kicks + 1);
    /* cleared, not drawn: the tile buffer held it, the buffer does not */
    let cleared = 0x4000_0072;
    let b = depth_object_draw(&rig, cleared, Some(0.25));
    assert_eq!(b.z, ZsKnown::Cleared(0.25));
    assert!(!v3d_depth_sampled(cleared, 64, 64), "a cleared buffer is resolved on the CPU");
    let mut t = [7.0f32; 4];
    // SAFETY: 1 row of 4 floats.
    assert_eq!(unsafe { v3d_depth_resolve(cleared, t.as_mut_ptr(), 4, 1, 4) }, 0);
    assert_eq!(t, [0.25; 4]);
    assert_eq!(depth_samples(), (0, 1, 0));
}

/// A binding that names a buffer not there to sample (the GL core left the
/// texels stale on the strength of an answer, and the buffer holds no
/// stored depth of that size): refused by name, never a copy of the stale
/// texels. A check, not a path: nothing between the answer and the draw
/// frees a stored buffer.
#[test]
fn a_binding_naming_a_buffer_not_there_to_sample_is_refused_by_name() {
    let rig = Rig::new();
    rig.init();
    let p = ret32_program();
    let id = 0x4000_0073;
    let (tilings0, copies0) = (tilings(), tiled().0);
    /* no buffer at all */
    assert_eq!(draw(&depth_sampling_draw(&rig, &p, id)), -EINVAL);
    /* a buffer the last job only cleared */
    depth_object_draw(&rig, id, Some(0.5));
    assert_eq!(draw(&depth_sampling_draw(&rig, &p, id)), -EINVAL);
    /* a cause is logged the first time it is seen (count_refusal); both are counted */
    assert_eq!(rig.logs_with("holds no stored depth of the texture's size"), 1);
    assert_eq!((tilings(), tiled().0), (tilings0, copies0), "no copy of the stale texels");
    assert_eq!(depth_samples(), (0, 0, 2));
}

/// glmark2's shadow frame through the GL core onto this backend
/// (`gl_rig`): a depth texture as the only attachment of a framebuffer,
/// the depth pass into it, the canvas bound back and the ground sampling
/// the texture. The GL core asks (`texture::depth_for_draw`), skips the
/// resolve and hands the binding the depth object's id; the ground's record
/// reads the depth pass's stored buffer as DEPTH24_X8; no resolve and no
/// tiling in either frame. A CPU write of the texels (glTexSubImage2D)
/// resolves first, as before, and the draw after it, with no render since,
/// samples the texels through a copy; the next render is read from the
/// buffer again.
///
/// Revert check: `depth_for_draw` always resolving fails the resolve count
/// at the first frame (1, not 0) and the tilings.
#[test]
fn glmark2s_shadow_frame_samples_the_depth_pass_buffer_through_the_gl_core() {
    use crate::gl::consts::*;
    use crate::gl::framebuffer::*;
    use crate::gl::program::glUseProgram;
    use crate::gl::state::*;
    use crate::gl::texture::*;
    let rig = gl_rig();
    let flat = gl_flat_program();
    let textured = gl_tex_program();
    let (w, h) = (64, 64);
    let mut depth = 0u32;
    let mut fbo = 0u32;
    /* glmark2's canvas: an fbo of the rig's memory (the rig's window surface
     * is TestGl's host memory, outside the V3D's 32-bit addresses) */
    let canvas_rb = gl_renderbuffer(GL_RGBA4, w, h);
    let mut canvas = 0u32;
    // SAFETY: plain GL calls on the rig's context with live out pointers.
    unsafe {
        glGenFramebuffers(1, &mut canvas);
        glBindFramebuffer(GL_FRAMEBUFFER, canvas);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, canvas_rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glGenTextures(1, &mut depth);
        glBindTexture(GL_TEXTURE_2D, depth);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as i32);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as i32);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            (if crate::REPORT_ES2 { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as i32,
            w,
            h,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE);
        glBindFramebuffer(GL_FRAMEBUFFER, canvas);
        glEnable(GL_DEPTH_TEST);
    }
    let id = crate::gl::context::ds_object_id(rig.gl.ctx().objs().textures[depth as usize].serial, true);
    /* SceneShadow::draw: DepthRenderTarget::enable, the horse, disable, the ground */
    let frame = || {
        // SAFETY: plain GL calls on the rig's context.
        unsafe {
            glUseProgram(flat);
            glBindFramebuffer(GL_FRAMEBUFFER, fbo);
            glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, depth, 0);
            glViewport(0, 0, w, h);
            glColorMask(0, 0, 0, 0);
            glClear(GL_DEPTH_BUFFER_BIT);
            gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
            glBindFramebuffer(GL_FRAMEBUFFER, canvas);
            glViewport(0, 0, w, h);
            glColorMask(1, 1, 1, 1);
            glUseProgram(textured);
            glBindTexture(GL_TEXTURE_2D, depth);
            gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
        }
    };
    let ground_record = || {
        let seen = rig.draws();
        let ground = seen.last().expect("the ground draw");
        assert_eq!(ground.tmu.len(), 1, "the ground samples the depth texture");
        words(&rig, ground.tmu[0].tex_state, 6)
    };
    let tilings0 = tilings();
    frame();
    let buffer = g().zsbufs.iter().find(|b| b.live() && b.owners[0] == id).expect("the depth texture's buffer");
    assert_eq!(buffer.z, ZsKnown::Stored, "the depth pass stored it");
    let rec = ground_record();
    assert_eq!((rec[0], rec[3] >> FAKE_TYPE_SHIFT), (buffer.address, V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8));
    assert_eq!(depth_samples(), (1, 0, 0), "frame 1: the buffer, no CPU resolve");
    assert_eq!(tilings(), tilings0, "frame 1: nothing tiled");
    frame();
    assert_eq!(depth_samples(), (2, 0, 0), "frame 2: the same");
    assert_eq!(tilings(), tilings0);
    /* a CPU write resolves the texels first (settle_texture), and the draw
     * after it, with no render since, samples the texels through a copy */
    let one = [0xffff_ffffu32];
    // SAFETY: plain GL calls; `one` is a 1x1 GL_UNSIGNED_INT depth image.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, depth);
        glTexSubImage2D(
            GL_TEXTURE_2D,
            0,
            0,
            0,
            1,
            1,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            one.as_ptr() as *const c_void,
        );
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", rig.gl.logs());
        glUseProgram(textured);
        gl_quad([-1.0, -1.0, 0.0], [1.0, 1.0, 0.0]);
        glFinish();
    }
    assert_eq!(depth_samples(), (2, 1, 0), "the write resolved once; the draw after it reads the texels");
    assert_eq!(tilings(), tilings0 + 1, "through one tiled copy");
    assert_eq!(ground_record()[3] >> FAKE_TYPE_SHIFT, V3D_TEXTURE_DATA_FORMAT_R32F, "the texels' own record");
    /* the next frame renders again: the buffer again */
    frame();
    assert_eq!(depth_samples(), (3, 1, 0));
    assert_eq!(tilings(), tilings0 + 1);
    assert_eq!(ground_record()[3] >> FAKE_TYPE_SHIFT, V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8);
    // SAFETY: live names, deleted while the rig's OSAL is installed.
    unsafe {
        glDeleteTextures(1, &depth);
    }
    gl_release(flat, &[fbo, canvas], &[canvas_rb]);
    gl_release(textured, &[], &[]);
}

/// The TMU output type and the texture's format agree, or the draw is
/// refused by name (texture::refusal_for): an R32F texture (a depth
/// texture) is sampled by a look-up whose P1 sets bit 0 ("Output Type
/// 32-bit", the program compiled with LodKey::ret32), and refused with a
/// 16-bit look-up; an RGBA8 texture with a 32-bit look-up is refused. The
/// R32F record is Mesa's row (TEXTURE_DATA_FORMAT_R32F, 29).
#[test]
fn a_depth_textures_look_up_must_take_the_32_bit_output() {
    static FS_16: [ShcV3dUniform; 2] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0x3 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0x4 },
    ];
    static FS_32: [ShcV3dUniform; 2] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0xf },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0x5 },
    ];
    let rig = Rig::new();
    rig.init();
    let attempt = |uniforms: &'static [ShcV3dUniform; 2], format: u8| -> (i32, Vec<String>) {
        let mut p = program();
        (p.v3d.fs.uniforms, p.v3d.fs.num_uniforms) = (uniforms.as_ptr(), 2);
        with_code(&mut p);
        let mut t = if format == GLES_TEXFMT_R32F { narrow_texture(4, 4, 1, format) } else { texture(0, 4, 4, 1) };
        (t.unit, t.content_gen) = (0, 7);
        let mut d = sampling(&rig, t);
        d.do_clear = true;
        d.fs = &p;
        let rc = draw(&d);
        (rc, rig.gl.logs())
    };
    let (rc, logs) = attempt(&FS_32, GLES_TEXFMT_R32F);
    assert_eq!(rc, 0, "R32F at the 32-bit output: {logs:?}");
    assert_eq!(v3d_flush(), 0);
    let st = rig.draws().last().unwrap().tmu[0].tex_state;
    assert_eq!(words(&rig, st, 6)[3] >> FAKE_TYPE_SHIFT, V3D_TEXTURE_DATA_FORMAT_R32F, "the record's type");
    let (rc, logs) = attempt(&FS_16, GLES_TEXFMT_R32F);
    assert!(
        rc < 0 && logs.iter().any(|l| l.contains("RGBA32I) read by a look-up compiled for the 16-bit TMU output")),
        "{rc} {logs:?}"
    );
    let (rc, logs) = attempt(&FS_32, GLES_TEXFMT_RGBA8);
    assert!(
        rc < 0
            && logs
                .iter()
                .any(|l| l.contains("a look-up compiled for the 32-bit TMU output reads a texture of a 16-bit format")),
        "{rc} {logs:?}"
    );
}

/// The ES 3.0 classes sample through their own TMU type (formats-v3d):
/// an RGBA16UI texture at the 16-bit output and an RGBA32F one at the
/// 32-bit output are recorded with Mesa's types (107, 31). An incomplete
/// texture of an integer class, or of a 32-bit class read at the 32-bit
/// output, is stood in for by a record of the class's own type over the
/// black image with the swizzle 0, 0, 0, 1 (the RGBA8 stand-in would read
/// f16 1.0 as the integer 0x3c00, or 16-bit words at a 32-bit look-up);
/// an incomplete RGBA8 texture read at the 32-bit output is still refused.
#[test]
fn the_es3_classes_and_their_incomplete_stand_ins_take_their_own_type() {
    static FS_16: [ShcV3dUniform; 2] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0x3 },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0x4 },
    ];
    static FS_32: [ShcV3dUniform; 2] = [
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P0, data: 0xf },
        ShcV3dUniform { contents: V3D_QUNIFORM_TMU_CONFIG_P1, data: 0x5 },
    ];
    let rig = Rig::new();
    rig.init();
    let attempt = |uniforms: &'static [ShcV3dUniform; 2], format: u8, incomplete: bool| -> (i32, Vec<String>) {
        let mut p = program();
        (p.v3d.fs.uniforms, p.v3d.fs.num_uniforms) = (uniforms.as_ptr(), 2);
        with_code(&mut p);
        let mut t = narrow_texture(4, 4, 1, format);
        (t.unit, t.content_gen, t.incomplete) = (0, 7, u8::from(incomplete));
        let mut d = sampling(&rig, t);
        d.do_clear = true;
        d.fs = &p;
        let rc = draw(&d);
        (rc, rig.gl.logs())
    };
    let record = |rig: &Rig| {
        let st = rig.draws().last().unwrap().tmu[0].tex_state;
        let w = words(rig, st, 6);
        (w[3] >> FAKE_TYPE_SHIFT, w[4].to_le_bytes())
    };
    const BLACK: [u8; 4] = [V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ZERO, V3D_SWIZZLE_ONE];
    const XYZW: [u8; 4] = [V3D_SWIZZLE_RED, V3D_SWIZZLE_GREEN, V3D_SWIZZLE_BLUE, V3D_SWIZZLE_ALPHA];
    /* (the stream, the class, incomplete, the record's type and swizzle) */
    type Case = (&'static [ShcV3dUniform; 2], u8, bool, u32, [u8; 4]);
    let cases: [Case; 4] = [
        (&FS_16, GLES_TEXFMT_RGBA16UI, false, V3D_TEXTURE_DATA_FORMAT_RGBA16UI, XYZW),
        (&FS_32, GLES_TEXFMT_RGBA32F, false, V3D_TEXTURE_DATA_FORMAT_RGBA32F, XYZW),
        (&FS_16, GLES_TEXFMT_RGBA8I, true, V3D_TEXTURE_DATA_FORMAT_RGBA8I, BLACK),
        (&FS_32, GLES_TEXFMT_RGBA32UI, true, V3D_TEXTURE_DATA_FORMAT_RGBA32UI, BLACK),
    ];
    for (fs, format, incomplete, type_, swizzle) in cases {
        let (rc, logs) = attempt(fs, format, incomplete);
        assert_eq!(rc, 0, "format {format}, incomplete {incomplete}: {logs:?}");
        assert_eq!(v3d_flush(), 0);
        assert_eq!(record(&rig), (type_, swizzle), "format {format}, incomplete {incomplete}");
    }
    let (rc, logs) = attempt(&FS_32, GLES_TEXFMT_RGBA8, true);
    assert!(
        rc < 0 && logs.iter().any(|l| l.contains("an incomplete texture read by a look-up compiled for the 32-bit")),
        "{rc} {logs:?}"
    );
}

/* ---- occlusion queries: OCCLUSION_QUERY_COUNTER's address per draw ---- */

/// A draw counted on a query carries its counter's address into the job
/// (the job builder writes it into OCCLUSION_QUERY_COUNTER); an uncounted
/// draw and the clear quad carry 0, and the counted draw after the quad has
/// the address again. query_begin zeroes the handle's word (and flushes
/// it), query_result reads what the TLB added there.
///
/// Deliberate breaks: drop the `oq_addr` line in `draw_call` and the
/// counted draws carry 0; drop the zeroing in `v3d_query_begin` and the
/// stale 0xdead is read back as the count.
#[test]
fn a_counted_draw_carries_its_counter_and_the_result_reads_it() {
    let rig = Rig::new();
    rig.init();
    assert!(g().oq.cpu.is_null(), "no counter memory until a query starts");
    assert_eq!(v3d_query_begin(5), 0, "{:?}", rig.gl.logs());
    let base = g().oq.gpu as u32;
    let word = base + 5 * 64;
    with_platform(|p| {
        let w = platform_cpu(p, word as u64, 4) as *mut u32;
        // SAFETY: inside the counter buffer.
        unsafe {
            assert_eq!(w.read(), 0, "begin zeroed the word");
            w.write(0xdead); /* a stale count */
        }
    });
    assert_eq!(v3d_query_begin(5), 0);
    assert_eq!(rig.gpu_bytes(word, 4), [0, 0, 0, 0], "a new begin zeroes it again");

    let mut counted = rig.draw();
    counted.occlusion_query = 5;
    counted.do_clear = false;
    let mut plain = rig.draw();
    plain.do_clear = false;
    let mut quad_then_counted = counted;
    (quad_then_counted.do_clear, quad_then_counted.clear_quad) = (true, true);
    (quad_then_counted.scissor, quad_then_counted.scissor_rect) = (true, [0, 0, 8, 8]);
    for d in [&counted, &plain, &quad_then_counted] {
        assert_eq!(draw(d), 0, "{:?}", rig.gl.logs());
    }
    assert_eq!(v3d_flush(), 0);
    let seen: Vec<u32> = rig.draws().iter().map(|s| s.st.oq_addr).collect();
    assert_eq!(seen, [word, 0, 0, word], "counted, plain, the clear quad, counted");

    /* the TLB added 42 at the end of the job */
    with_platform(|p| {
        let w = platform_cpu(p, word as u64, 4) as *mut u32;
        // SAFETY: inside the counter buffer.
        unsafe { w.write(42) };
    });
    let mut n = 0u64;
    // SAFETY: `n` is a live u64.
    assert_eq!(unsafe { v3d_query_result(5, &mut n) }, 0);
    assert_eq!(n, 42);
}

/// Handles outside 1 .. GLES_QUERY_HANDLES are refused, a result of a
/// handle with no counter memory is an error, and a failed allocation is
/// -ENOMEM with no address handed out.
#[test]
fn query_handles_are_checked_and_no_memory_is_enomem() {
    let rig = Rig::new();
    rig.init();
    let mut n = 7u64;
    // SAFETY: `n` is a live u64.
    assert_eq!(unsafe { v3d_query_result(3, &mut n) }, -EINVAL, "never started");
    assert_eq!(v3d_query_begin(0), -EINVAL);
    assert_eq!(v3d_query_begin(GLES_QUERY_HANDLES), -EINVAL);
    with_platform(|p| p.fail_alloc_from = Some(1));
    assert_eq!(v3d_query_begin(3), -ENOMEM);
    with_platform(|p| p.fail_alloc_from = None);
    let mut d = rig.draw();
    d.occlusion_query = 3;
    assert_eq!(draw(&d), 0);
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rig.draws()[0].st.oq_addr, 0, "no counter: no address");
    assert_eq!(n, 7);
}

/// Doc 5.2, the 500-quad frame (dEQP `long_masked`): 500 scissored
/// `clear_quad` draws in one frame. The job builder reports the job full
/// after 300; the back end submits that job and records the rest in a new
/// one, and the continuation loads the colour the first job drew.
///
/// The fake builder has no capacity of its own, so the "full" is its answer
/// (`V3D_JOB_FULL`), as in `a_full_job_is_submitted_and_the_draw_goes_into_the_next`.
#[test]
fn a_500_quad_frame_flushes_when_the_job_is_full_and_continues_loading_the_colour() {
    const QUADS: usize = 500;
    const FIT: usize = 300;
    let rig = Rig::new();
    rig.init();
    let quad = |i: usize| {
        let mut c = clear_only(&rig);
        c.clear_quad = true;
        (c.scissor, c.scissor_rect) = (true, [(i % 32) as i32, 0, 8, 8]);
        c.clear_rgba = [0.0, 1.0, 0.0, 1.0];
        c
    };
    for i in 0..QUADS {
        if i == FIT {
            assert!(rig.kicks().is_empty(), "nothing is submitted while the job has room");
            rig.answer(&[V3D_JOB_FULL]);
        }
        assert_eq!(draw(&quad(i)), 0, "quad {i}: {:?}", rig.gl.logs());
    }
    /* the full job went to the GPU when the 301st quad did not fit */
    assert_eq!(rig.kicks().len(), 1, "the full job is submitted");
    assert_eq!(v3d_flush(), 0);
    assert_eq!(rig.kicks().len(), 2, "and the continuation at the flush");
    let (jobs, draws) = (rig.jobs(), rig.draws());
    assert_eq!(jobs.iter().map(|j| j.draws as usize).collect::<Vec<_>>(), [FIT, QUADS - FIT]);
    assert_eq!(draws.len(), QUADS, "every quad is recorded once, none twice");
    for (i, q) in draws.iter().enumerate() {
        let want = state::scissor(&quad(i)).expect("the clear's box");
        assert_eq!(
            [q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy],
            want,
            "quad {i} keeps its own box across the flush"
        );
        assert_is_the_quad(&rig, q, 1.0, [0.0, 1.0, 0.0, 1.0]);
    }
    for (n, j) in jobs.iter().enumerate() {
        assert!(j.clear.unwrap().load_rt0, "job {n}: a quad clear never clears the colour in the TLB");
    }
}

/// Doc 5.2, `depth_stencil_clear` in the recorded jobs: a scissored depth
/// quad, a scissored stencil quad, then a full unscissored clear. The quads
/// are drawn into one job, in that order; the full clear is a TLB clear and
/// opens the job after them (a TLB clear cannot follow draws in its job),
/// with no draw of its own and no load of the colour.
#[test]
fn a_depth_quad_a_stencil_quad_then_a_full_clear_are_quad_quad_then_the_tlb_clear() {
    let rig = Rig::new();
    rig.init();
    let mut depth = clear_only(&rig);
    depth.clear_quad = true;
    (depth.do_clear, depth.clear_color_mask) = (false, 0);
    (depth.do_clear_depth, depth.clear_depth) = (true, 0.25);
    (depth.scissor, depth.scissor_rect) = (true, [4, 4, 16, 16]);
    assert_eq!(draw(&depth), 0, "{:?}", rig.gl.logs());
    let mut stencil = clear_only(&rig);
    stencil.clear_quad = true;
    (stencil.do_clear, stencil.clear_color_mask) = (false, 0);
    (stencil.do_clear_stencil, stencil.clear_stencil) = (true, 0x33);
    (stencil.stencil_write_mask, stencil.stencil_back_write_mask) = (0xff, 0xff);
    (stencil.scissor, stencil.scissor_rect) = (true, [20, 20, 16, 16]);
    assert_eq!(draw(&stencil), 0, "{:?}", rig.gl.logs());
    assert!(rig.kicks().is_empty(), "the quads share the open job");
    let mut full = clear_only(&rig);
    (full.do_clear_depth, full.clear_depth) = (true, 0.75);
    (full.do_clear_stencil, full.clear_stencil) = (true, 0x44);
    (full.stencil_write_mask, full.stencil_back_write_mask) = (0xff, 0xff);
    full.clear_rgba = [0.0, 0.0, 1.0, 1.0];
    assert_eq!(draw(&full), 0, "{:?}", rig.gl.logs());
    assert_eq!(rig.kicks().len(), 1, "the full clear ends the job holding the quads");
    assert_eq!(v3d_flush(), 0);
    let (jobs, draws) = (rig.jobs(), rig.draws());
    assert_eq!(jobs.len(), 2);
    assert_eq!((jobs[0].draws, jobs[1].draws), (2, 0), "quad, quad | TLB clear");
    assert_eq!(draws.len(), 2, "the full clear draws nothing");
    let (d, s) = (&draws[0], &draws[1]);
    assert!(d.st.depth_enabled && d.st.depth_writemask && !d.st.stencil_enabled, "the first quad is the depth's");
    assert!(s.st.stencil_enabled && !s.st.depth_enabled, "the second quad is the stencil's");
    assert_eq!(s.st.stencil_front.ref_, 0x33);
    for (q, c) in [(d, &depth), (s, &stencil)] {
        let want = state::scissor(c).expect("the clear's box");
        assert_eq!([q.st.scissor_minx, q.st.scissor_miny, q.st.scissor_maxx, q.st.scissor_maxy], want);
    }
    assert_ne!([d.st.scissor_minx, d.st.scissor_miny], [s.st.scissor_minx, s.st.scissor_miny], "each its own box");
    let (q, c) = (jobs[0].clear.unwrap(), jobs[1].clear.unwrap());
    assert!(q.load_rt0, "the quad job clears nothing in the TLB");
    assert!(!c.load_rt0, "the TLB clear clears the colour");
    assert_eq!(c.rgba8, 0xffff_0000, "blue, R in the low byte");
    assert_eq!((c.z, c.stencil), (0.75, 0x44), "and the depth and stencil of the full clear");
    assert_eq!(rig.kicks().len(), 2);
}
