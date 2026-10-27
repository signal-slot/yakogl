// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glGenerateMipmap on the Pi 4 (src/v3d/backend.rs `v3d_generate_mipmap`)
//! queues the TFU job that makes a render target's chain behind the render
//! job that stores its level 0, without waiting for that job on the CPU, as
//! Mesa's v3d_generate_mipmap queues its TFU job with an in-sync on the
//! render's out-sync. Before, every call submitted the open render and
//! waited for it on the CPU, five times a frame in glmark2 terrain
//! (docs/notes/pi4-glmark2-cpu.md).
//!
//! The GL calls are glmark2 refract's and terrain's shape: a framebuffer
//! whose colour attachment is a mipmapped RGBA8 texture, a render into it,
//! glGenerateMipmap with the texture bound, a draw on the canvas that
//! samples it through a mipmap filter. They run through the GL core against
//! the REAL V3D backend (`yakogl_backend_v3d`) and the REAL layout and TFU
//! packing code the board runs, `driver/src/v3d/v3d_tex.c` (built here with
//! `cc` and loaded with `dlopen`, as tests/depth_resolve_v3d.rs builds it).
//! The job builder is stood in for by fakes that record each job's target
//! and texture records. The platform is an OSAL written here that runs the
//! board's order (src/osal/zephyr_v3d.c): one job in flight, finished only
//! by a wait() on it or the next submit (the GPU never finishes on its own
//! here, so every completion the backend sees is one it waited for or
//! caused), a TFU job queued behind the job in flight run when that job is
//! finished, before the next job starts, and one queued with nothing in
//! flight run at once. Every submit, wait and TFU job is an event in order.
//!
//! What is pinned:
//! * the render job open at the call: glGenerateMipmap submits it and queues
//!   the TFU job behind its fence (`after`), and no wait() happens inside the
//!   call at all; the TFU job runs when that render is finished, before the
//!   job of the draw that samples the chain is submitted, and that draw reads
//!   the render image through its whole chain;
//! * the render already submitted, another job in flight after it: no wait,
//!   the TFU job queued behind the job in flight (later than the render);
//!   the render completed: the TFU job runs inside the call, no wait;
//! * against the version before (a wait for the render inside the call,
//!   then OSAL_QUERY_V3D_TFU_RUN) the first two fail: they see the wait.
//!
//! The test names the query and its struct itself (osal_internal.h
//! OSAL_QUERY_V3D_TFU_QUEUE, struct osal_v3d_tfu_queue), so it builds against
//! the version before as well and shows the wait there.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_tfu_mipmap_nowait`.

#![cfg(all(
    feature = "host-test",
    feature = "gpu-v3d",
    feature = "shaderc-online",
    target_os = "linux",
    any(target_arch = "x86_64", target_arch = "aarch64")
))]
/* the fakes and trampolines take the C functions' parameter lists as they are */
#![allow(clippy::too_many_arguments)]

use core::ffi::{c_char, c_int, c_void, CStr};
use std::collections::BTreeMap;
use std::ffi::CString;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};

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
use yakogl::osal::{osal_set_ops, OsalGpuMem, OsalOps, OsalV3dCpuOf, OsalV3dTfu};
use yakogl::v3d::backend::*;
use yakogl::v3d::ffi::*;
use yakogl::v3d::kick::V3dKick;

/* ---- the real layout code, built from driver/src/v3d/v3d_tex.c ------------- */

extern "C" {
    fn dlopen(path: *const c_char, flags: c_int) -> *mut c_void;
    fn dlsym(h: *mut c_void, name: *const c_char) -> *mut c_void;
}

/// `driver/src/v3d/v3d_tex.c` and the Mesa tiling it includes, as a shared
/// object of this test's own, reached through the handle only (RTLD_LOCAL).
fn lib() -> *mut c_void {
    static L: OnceLock<usize> = OnceLock::new();
    *L.get_or_init(|| {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_tfu_mipmap_nowait");
        std::fs::create_dir_all(&dir).expect("the scratch directory");
        let so = dir.join(format!("v3d_tex-{}.so", std::process::id()));
        /* the compiler's own temporary files next to the object */
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
/// (src/v3d/ffi.rs), each handed to the real C.
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

/* The signatures are src/v3d/ffi.rs's, kept as written (see
 * tests/depth_resolve_v3d.rs: rustfmt releases wrap these differently). */
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
    jobs().push(SeenJob { target: t, tmu: Vec::new() });
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
    _c: *const V3dJobClear,
    _tile_list_start: *mut u32,
    _tile_list_end: *mut u32,
) -> bool {
    // SAFETY: the backend passes its live job; its lists are live.
    unsafe {
        (*(*j).bcl).used += 1;
        (*(*j).rcl).used = 200;
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
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _draws: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
    // SAFETY: the backend's parameters.
    unsafe {
        let n = (*t).tiles_x * (*t).tiles_y;
        *ta = n * 128 + 8192;
        *ts = n * 256;
    }
}

/* ---- the platform ------------------------------------------------------------ */

/// osal_internal.h's OSAL_QUERY_V3D_TFU_QUEUE and struct osal_v3d_tfu_queue,
/// named here so the test builds against a library without them.
const QUERY_V3D_TFU_QUEUE: u32 = 17;
#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct TfuQueue {
    tfu: OsalV3dTfu,
    after: i64,
    fence: i64,
}
const _: () = assert!(size_of::<TfuQueue>() == 56);

/// The answer to OSAL_QUERY_V3D_TFU_MIN_BYTES: the TFU on, as the Pi 4's
/// glmark2 image has it; the 16 KiB copies here stay below it (CPU tiling).
const TFU_MIN_BYTES: u32 = 65536;

/// What the platform saw, in order.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Ev {
    /// A job submitted: its fence.
    Submit(usize),
    /// A wait() on a fence: the CPU waiting for the GPU.
    Wait(i64),
    /// A TFU job queued (OSAL_QUERY_V3D_TFU_QUEUE): the fence the backend
    /// named (`after`), and the fence the platform put it behind (0: none in
    /// flight, it ran in the call).
    TfuQueued { after: i64, behind: usize },
    /// A TFU job ran on the "GPU": after finishing job `behind` (0: within
    /// the query that asked for it). `run`: through OSAL_QUERY_V3D_TFU_RUN.
    TfuRan { behind: usize, run: bool },
}

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
    /// The highest completed fence and the job in flight (one at most).
    completed: usize,
    inflight: Option<usize>,
    /// The TFU jobs queued behind the job in flight.
    queued: Vec<OsalV3dTfu>,
    events: Vec<Ev>,
    log: Vec<String>,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// Finish the job in flight (zephyr_v3d.c finish_inflight): its render is
/// over, then the TFU jobs queued behind it run, then its fence completes.
fn finish(p: &mut Platform) {
    if let Some(f) = p.inflight.take() {
        for _ in core::mem::take(&mut p.queued) {
            p.events.push(Ev::TfuRan { behind: f, run: false });
        }
        p.completed = f;
    }
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

/// One job in flight: the one before is finished first (zephyr_v3d.c
/// z_submit), and this one runs until a wait() or the next submit.
unsafe extern "C" fn p_submit(kick: *const c_void, len: usize) -> i64 {
    if len != size_of::<V3dKick>() {
        return -22;
    }
    // SAFETY: the backend's live descriptor of that size.
    let k = unsafe { *(kick as *const V3dKick) };
    with_platform(|p| {
        finish(p);
        p.kicks.push(k);
        let fence = p.kicks.len();
        p.inflight = Some(fence);
        p.events.push(Ev::Submit(fence));
        fence as i64
    })
}

unsafe extern "C" fn p_wait(fence: i64, _timeout_us: i64) -> i32 {
    with_platform(|p| {
        p.events.push(Ev::Wait(fence));
        if p.inflight.is_some_and(|f| fence >= f as i64) {
            finish(p);
        }
        0
    })
}

/// The GPU never finishes on its own: only what a wait or a submit finished.
unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.completed as i64)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
    use yakogl::osal::{
        OSAL_QUERY_V3D_BIN_POOL, OSAL_QUERY_V3D_CPU_OF_GPU, OSAL_QUERY_V3D_JOB_OPEN, OSAL_QUERY_V3D_TFU_MIN_BYTES,
        OSAL_QUERY_V3D_TFU_RUN, OSAL_QUERY_V3D_VPM_BYTES,
    };
    if what == OSAL_QUERY_V3D_VPM_BYTES && out_sz == 4 {
        // SAFETY: a u32 out-parameter.
        unsafe { *(out as *mut u32) = 65536 };
        return 0;
    }
    if what == OSAL_QUERY_V3D_TFU_MIN_BYTES && out_sz == 4 {
        // SAFETY: a u32 out-parameter.
        unsafe { *(out as *mut u32) = TFU_MIN_BYTES };
        return 0;
    }
    if what == OSAL_QUERY_V3D_TFU_RUN && out_sz == size_of::<OsalV3dTfu>() {
        /* zephyr_v3d.c osal_v3d_tfu: the job in flight finished first */
        // SAFETY: the backend's in/out struct of that size.
        let q = unsafe { &mut *(out as *mut OsalV3dTfu) };
        with_platform(|p| {
            finish(p);
            p.events.push(Ev::TfuRan { behind: 0, run: true });
        });
        q.rc = 0;
        return 0;
    }
    if what == QUERY_V3D_TFU_QUEUE && out_sz == size_of::<TfuQueue>() {
        // SAFETY: the backend's in/out struct of that size.
        let q = unsafe { &mut *(out as *mut TfuQueue) };
        return with_platform(|p| {
            if q.after < 0 || q.after as usize > p.kicks.len() {
                return -22;
            }
            match p.inflight {
                Some(f) => {
                    p.queued.push(q.tfu);
                    p.events.push(Ev::TfuQueued { after: q.after, behind: f });
                    (q.fence, q.tfu.rc) = (f as i64, 0);
                }
                None => {
                    p.events.push(Ev::TfuQueued { after: q.after, behind: 0 });
                    p.events.push(Ev::TfuRan { behind: 0, run: false });
                    (q.fence, q.tfu.rc) = (0, 0);
                }
            }
            0
        });
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
/// two-pointer function takes it (see tests/depth_resolve_v3d.rs).
unsafe extern "C" fn p_log2(_fmt: *const c_char, line: *const c_char) {
    // SAFETY: the library passes a NUL-terminated line.
    let s = unsafe { CStr::from_ptr(line) }.to_string_lossy().into_owned();
    with_platform(|p| p.log.push(s));
}

type LogFn = unsafe extern "C" fn(*const c_char, ...);

unsafe extern "C" fn p_init(_platform_arg: *mut c_void) -> i32 {
    0
}

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

/* ---- the context ------------------------------------------------------------- */

/// glmark2's canvas and refract's target are 800x600 and 1600x1200; a pair
/// of the same roles keeps the test quick. 64x64: a UIF image of 7 levels.
const CANVAS: (i32, i32) = (32, 32);
const TARGET: i32 = 64;
const LEVELS: u32 = 7;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The one context and backend of the file, set up once; the tests take
/// turns, each starting with every job finished.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
            CTX = context::gl_context_create(CANVAS.0 as u32, CANVAS.1 as u32);
            assert!(!CTX.is_null(), "{:?}", logs());
        }
    });
    // SAFETY: plain GL calls.
    unsafe {
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

fn events() -> Vec<Ev> {
    with_platform(|p| p.events.clone())
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

const PLAIN_VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
const PLAIN_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0); }\n";
const SAMPLE_VS: &str = "attribute vec4 a_position;\nvarying vec2 v_coord;\n\
    void main() { v_coord = a_position.xy * 0.5 + 0.5; gl_Position = a_position; }\n";
const SAMPLE_FS: &str = "precision mediump float;\nuniform sampler2D u_tex;\nvarying vec2 v_coord;\n\
    void main() { gl_FragColor = texture2D(u_tex, v_coord); }\n";

/* a quad over the whole target, (x, y, z) per vertex */
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

struct Scene {
    plain: GLuint,
    sample: GLuint,
    tex: GLuint,
    fbo: GLuint,
}

/// Refract's `RenderTarget` shape: a TARGET-square RGBA8 texture with a
/// mipmap filter (LINEAR_MIPMAP_LINEAR), a framebuffer with it as the colour
/// attachment, and the two programs.
fn setup() -> Scene {
    let plain = program(PLAIN_VS, PLAIN_FS);
    let sample = program(SAMPLE_VS, SAMPLE_FS);
    let (mut tex, mut fbo) = (0u32, 0u32);
    let name = CString::new("u_tex").unwrap();
    // SAFETY: plain GL calls with live out pointers.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            GL_RGBA as GLint,
            TARGET,
            TARGET,
            0,
            GL_RGBA,
            GL_UNSIGNED_BYTE,
            core::ptr::null(),
        );
        glGenerateMipmap(GL_TEXTURE_2D);
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(sample);
        glUniform1i(glGetUniformLocation(sample, name.as_ptr()), 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    Scene { plain, sample, tex, fbo }
}

/// The render into the target, left open (as glmark2's is at its
/// glGenerateMipmap).
fn render_target(s: &Scene) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
        glViewport(0, 0, TARGET, TARGET);
        glUseProgram(s.plain);
        glClearColor(0.1, 0.2, 0.3, 1.0);
        glClear(GL_COLOR_BUFFER_BIT);
        draw_quad();
    }
}

/// glGenerateMipmap on the target: the events inside the call.
fn generate(s: &Scene) -> Vec<Ev> {
    let n = events().len();
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    events()[n..].to_vec()
}

/// A draw on the canvas: sampling the target, or plain.
fn canvas(s: &Scene, sample: bool) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glUseProgram(if sample { s.sample } else { s.plain });
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, s.tex);
        draw_quad();
    }
}

fn flush() {
    // SAFETY: a plain GL call.
    unsafe { glFlush() };
}

fn finish_all() {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
}

/// The mip chains the TFU made since init (`v3d_rt_images`).
fn chains() -> u32 {
    let mut n = 0u32;
    // SAFETY: a live out-parameter; the others null.
    unsafe {
        v3d_rt_images(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut n,
            core::ptr::null_mut(),
        )
    };
    n
}

/// Whole frames until glGenerateMipmap is the TFU's: the first render has
/// no image (nothing sampled the target yet), the first sample registers
/// it, the next render stores its image.
fn warm_up(s: &Scene) {
    let before = chains();
    for _ in 0..6 {
        render_target(s);
        generate(s);
        canvas(s, true);
        finish_all();
        if chains() > before {
            return;
        }
    }
    panic!("glGenerateMipmap never took the TFU path: {:?}", logs());
}

/// The fence of the last job submitted.
fn last_fence() -> usize {
    with_platform(|p| p.kicks.len())
}

/// Where in `ev` the first event `want` is.
fn at(ev: &[Ev], want: Ev) -> usize {
    ev.iter().position(|e| *e == want).unwrap_or_else(|| panic!("{want:?} in {ev:?}"))
}

/// The texture state the last draw of job `fence` sampled unit 0 with.
fn sampled_state(fence: usize) -> V3dTexState {
    let gpu = {
        let jobs = jobs();
        let draw = jobs[fence - 1].tmu.last().expect("a draw in the sampling job");
        assert_eq!(draw.len(), 1, "one texture: {draw:?}");
        draw[0].tex_state
    };
    let cpu = with_platform(|p| {
        let (base, a) = p.allocs.range(..=u64::from(gpu)).next_back().expect("the record's allocation");
        // SAFETY: inside the allocation.
        unsafe { (a.mem.as_ptr() as *const u8).add((u64::from(gpu) - base) as usize) as usize }
    });
    let states = STATES.lock().unwrap_or_else(|e| e.into_inner());
    states.iter().rev().find(|(at, _)| *at == cpu).map(|(_, s)| *s).expect("the record was packed")
}

/* ---- the tests ----------------------------------------------------------------- */

/// The render job open at the call (refract's and terrain's frame): the call
/// submits it and queues the TFU job behind its fence, with no wait() inside
/// the call at all. The TFU job runs when that render is finished, before
/// the job of the draw that samples the chain is submitted; that draw reads
/// the image the render stored, through the whole chain.
#[test]
fn the_tfu_job_is_queued_behind_the_open_render_with_no_cpu_wait() {
    let _g = rig();
    let s = setup();
    warm_up(&s);
    for frame in 0..3 {
        render_target(&s);
        let made = chains();
        let ev = generate(&s);
        let render = last_fence();
        assert_eq!(chains(), made + 1, "frame {frame}: the TFU made the chain: {:?}", logs());
        let image = jobs()[render - 1].target.uif_address;
        assert_ne!(image, 0, "frame {frame}: the render job stores the render image the TFU reads");
        assert!(
            !ev.iter().any(|e| matches!(e, Ev::Wait(_))),
            "frame {frame}: no CPU wait inside glGenerateMipmap: {ev:?}"
        );
        assert_eq!(
            ev,
            [Ev::Submit(render), Ev::TfuQueued { after: render as i64, behind: render }],
            "frame {frame}: the render submitted, the TFU job queued behind it"
        );
        /* the canvas samples the chain: its job comes after the TFU */
        let n = events().len();
        canvas(&s, true);
        flush();
        let sampler = last_fence();
        let ev = events()[n..].to_vec();
        assert!(
            at(&ev, Ev::TfuRan { behind: render, run: false }) < at(&ev, Ev::Submit(sampler)),
            "frame {frame}: the TFU ran after the render and before the sampling job: {ev:?}"
        );
        let st = sampled_state(sampler);
        assert_eq!(st.base, image, "frame {frame}: the draw samples the render image");
        assert_eq!((st.base_level, st.max_level), (0, LEVELS - 1), "frame {frame}: through its whole chain");
        finish_all();
    }
}

/// The render already submitted when glGenerateMipmap comes: with another
/// job in flight after it, the TFU job is queued behind that job (which the
/// OSAL finishes after the render), and nothing waits; with every job
/// completed, the TFU job runs inside the call, and nothing waits either.
#[test]
fn a_render_already_submitted_needs_no_wait_either() {
    let _g = rig();
    let s = setup();
    warm_up(&s);
    /* the render, then a canvas job submitted after it, still in flight */
    render_target(&s);
    flush();
    let render = last_fence();
    canvas(&s, false);
    flush();
    let later = last_fence();
    assert!(later > render);
    let made = chains();
    let ev = generate(&s);
    assert_eq!(chains(), made + 1, "{:?}", logs());
    assert_eq!(ev, [Ev::TfuQueued { after: later as i64, behind: later }], "queued behind the job in flight, no wait");
    let n = events().len();
    canvas(&s, true);
    flush();
    let sampler = last_fence();
    let ev = events()[n..].to_vec();
    assert!(at(&ev, Ev::TfuRan { behind: later, run: false }) < at(&ev, Ev::Submit(sampler)), "{ev:?}");
    assert_eq!(sampled_state(sampler).max_level, LEVELS - 1);
    finish_all();

    /* every job completed: the TFU job runs in the call */
    render_target(&s);
    finish_all();
    let made = chains();
    let ev = generate(&s);
    assert_eq!(chains(), made + 1, "{:?}", logs());
    assert_eq!(ev, [Ev::TfuQueued { after: 0, behind: 0 }, Ev::TfuRan { behind: 0, run: false }], "at once, no wait");
    canvas(&s, true);
    flush();
    assert_eq!(sampled_state(last_fence()).max_level, LEVELS - 1);
    finish_all();
}
