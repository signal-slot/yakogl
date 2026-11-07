// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glmark2 terrain's steady frame on the Pi 4, replayed on the host: the V3D
//! jobs, the CPU waits, the TFU chains, the tiled copies and the render
//! images a frame (plan #329, item `v3d-terrain-job-pin`; #1209). The Pi 4
//! counterpart of tests/pvr_terrain_kicks.rs.
//!
//! The GL calls are one steady frame of the V3D host trace in
//! docs/notes/terrain-mip-usage.md ("One steady frame"): the height map and
//! the normal map (256x256, one level), the terrain target (800x600 with a
//! D16 renderbuffer, a chain), bloom H and bloom V (256x256, chains), the
//! overlay blended into the terrain target, tilt-shift H (800x600, chain),
//! tilt-shift V to the window, five `glGenerateMipmap` calls on the rendered
//! targets, and the swap. The set-up is the scene's too: `update_mipmap` on
//! the five FBO textures before any render, three 512x512 uploads with a
//! chain, and the specular map rendered once and mipmapped. The shaders are
//! stand-ins that sample the same textures (every texture of a pass is read
//! by its fragment shader, the vertex-stage fetches of the height and normal
//! maps included) and the terrain's 393,216 vertices are a quad: the job
//! count, the targets and the texture bindings are terrain's, the shader
//! cost is not.
//!
//! They run through the GL core against the REAL V3D backend
//! (`yakogl_backend_v3d`) and the REAL layout and TFU packing code the board
//! runs (`driver/src/v3d/v3d_tex.c`, built with `cc` and loaded with
//! `dlopen`, as tests/v3d_tfu_mipmap_nowait.rs does, whose platform this is:
//! one job in flight, the queued TFU jobs run when it is finished, the
//! synchronous TFU run finishing the job in flight first). The job builder
//! is stood in for. The swap is `eglSwapBuffers`'s calls on the backend
//! without EGL: `surface_swapped`, the flush, and `v3d_finish_previous` (the
//! wait for the frame before).
//!
//! What is pinned is what the code does at the main tip, not what it should
//! do. Two tests:
//! * `a_clean_terrain_frame`: terrain alone, nothing stale;
//! * `terrain_after_an_fbo_scene_that_deleted_its_textures`: a scene whose
//!   FBO textures were rendered into and deleted runs first, so terrain's
//!   uploads may take slots with a stale render-into mark
//!   (docs/notes/v3d-terrain-copies.md, "The finding").
//!
//! The literals that other items change are marked `CHANGES` at each:
//! * `pi4-target-mark-clear` (the mark cleared when a slot is freed) moves
//!   the second test to the first's values (5 chains a frame, no `unknown`
//!   refusal, no `first-sample` copy);
//! * `v3d-rt-evict` (the render-image table no longer evicts targets a job
//!   stores every frame) does the same where a table is overfilled, and may
//!   move the first test's `image-store` and `level0-back` counts.
//!
//! Whoever changes a number updates its literal in the same commit.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_terrain_jobs`.

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
use std::sync::atomic::{AtomicU32, Ordering};
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_terrain_jobs");
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

/// glmark2's window on every board record: 800x600.
const W: i32 = 800;
const H: i32 = 600;

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The backend once, a fresh GL context for each test (the previous one is
/// destroyed with its textures), so no test inherits another's texture
/// names or render-into marks.
fn rig() -> MutexGuard<'static, ()> {
    let g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    ONCE.call_once(|| {
        // SAFETY: statics; the lock serialises every GL call of the file.
        unsafe {
            osal_set_ops(osal());
            yakogl::backend::g_backend = &yakogl_backend_v3d;
            assert_eq!(yakogl::backend::init(), 0, "{:?}", logs());
        }
    });
    // SAFETY: statics under the lock; plain GL calls.
    unsafe {
        if !CTX.is_null() {
            glFinish();
            context::gl_context_destroy(CTX);
        }
        CTX = context::gl_context_create(W as u32, H as u32);
        assert!(!CTX.is_null(), "{:?}", logs());
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

/// A program that averages `n` samplers (`u0`..), unit i for sampler i.
fn program(n: usize) -> GLuint {
    let vs = "attribute vec4 a_position;\nvarying vec2 v_coord;\n\
        void main() { v_coord = a_position.xy * 0.5 + 0.5; gl_Position = a_position; }\n";
    let mut fs = String::from("precision mediump float;\nvarying vec2 v_coord;\n");
    for i in 0..n {
        fs += &format!("uniform sampler2D u{i};\n");
    }
    fs += "void main() {\n  vec4 c = vec4(0.25, 0.5, 0.75, 1.0);\n";
    for i in 0..n {
        fs += &format!("  c += texture2D(u{i}, v_coord);\n");
    }
    fs += &format!("  gl_FragColor = c * {:.4};\n}}\n", 1.0 / (n as f32 + 1.0));
    let (v, f) = (shader(GL_VERTEX_SHADER, vs), shader(GL_FRAGMENT_SHADER, &fs));
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
        glUseProgram(p);
        for i in 0..n {
            let u = CString::new(format!("u{i}")).unwrap();
            glUniform1i(glGetUniformLocation(p, u.as_ptr()), i as GLint);
        }
        p
    }
}

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

/// A render target: a texture and the framebuffer that renders into it.
#[derive(Clone, Copy)]
struct Target {
    tex: GLuint,
    fbo: GLuint,
    w: i32,
    h: i32,
}

/// `BaseRenderer::create_texture`: RGBA8, `glTexImage2D` with no data, then
/// `setup_texture`'s filters when `mip` (the height and normal maps keep
/// GL_LINEAR). `depth` adds a D16 renderbuffer (the terrain target).
fn target(w: i32, h: i32, mip: bool, depth: bool) -> Target {
    let (mut tex, mut fbo) = (0u32, 0u32);
    // SAFETY: plain GL calls with live out pointers.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(
            GL_TEXTURE_2D,
            GL_TEXTURE_MIN_FILTER,
            if mip { GL_LINEAR_MIPMAP_LINEAR } else { GL_LINEAR } as GLint,
        );
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, w, h, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, tex, 0);
        if depth {
            let mut rb = 0u32;
            glGenRenderbuffers(1, &mut rb);
            glBindRenderbuffer(GL_RENDERBUFFER, rb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH_COMPONENT16, w, h);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_RENDERBUFFER, rb);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    Target { tex, fbo, w, h }
}

/// A 512x512 `GL_RGB` upload with its chain (glmark2's `Texture::load`).
fn upload_jpeg() -> GLuint {
    let mut tex = 0u32;
    let px = vec![0x80u8; 512 * 512 * 3];
    // SAFETY: plain GL calls; `px` lives for the call.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_LINEAR as GLint);
        glPixelStorei(GL_UNPACK_ALIGNMENT, 1);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGB as GLint, 512, 512, 0, GL_RGB, GL_UNSIGNED_BYTE, px.as_ptr() as *const _);
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    tex
}

fn genmip(tex: GLuint) {
    // SAFETY: plain GL calls on a live name.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glGenerateMipmap(GL_TEXTURE_2D);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/// One pass: into `to` (None: the window), `prog`, the textures on units
/// 0.., a clear first when `clear`.
fn pass(to: Option<Target>, prog: GLuint, from: &[GLuint], clear: bool) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        let (fbo, w, h) = to.map_or((0, W, H), |t| (t.fbo, t.w, t.h));
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glViewport(0, 0, w, h);
        glUseProgram(prog);
        for (i, t) in from.iter().enumerate() {
            glActiveTexture(GL_TEXTURE0 + i as GLenum);
            glBindTexture(GL_TEXTURE_2D, *t);
        }
        glActiveTexture(GL_TEXTURE0);
        if clear {
            glClearColor(0.1, 0.2, 0.3, 1.0);
            glClear(GL_COLOR_BUFFER_BIT | if fbo != 0 { GL_DEPTH_BUFFER_BIT } else { 0 });
        }
        draw_quad();
    }
}

/// `eglSwapBuffers` on the backend: the hook, the submit, the wait for the
/// frame before (src/gl/egl.rs).
fn swap() {
    v3d_surface_swapped(0);
    // SAFETY: a plain GL call.
    unsafe { glFlush() };
    let n = events().len();
    v3d_finish_previous();
    let waits = events()[n..].iter().filter(|e| matches!(e, Ev::Wait(_))).count();
    SWAP_WAITS.fetch_add(waits as u32, Ordering::Relaxed);
}

/// The waits inside the swaps since the last `measure`.
static SWAP_WAITS: AtomicU32 = AtomicU32::new(0);

fn finish_all() {
    // SAFETY: a plain GL call.
    unsafe { glFinish() };
}

/* ---- terrain ------------------------------------------------------------------- */

struct Terrain {
    height: Target,
    normal: Target,
    specular: Target,
    jpegs: [GLuint; 3],
    terrain: Target,
    bloom_h: Target,
    bloom_v: Target,
    tilt_h: Target,
    p0: GLuint,
    p1: GLuint,
    p6: GLuint,
}

/// `SceneTerrain::init_renderers`, `setup_texture` and `setup`, in the
/// order of the host trace: the height and normal maps, the specular map
/// and the three uploads, the five mipmapped targets (`update_mipmap`
/// before any render), then the specular map rendered once and mipmapped.
fn terrain_setup() -> Terrain {
    let (p0, p1, p6) = (program(0), program(1), program(6));
    let height = target(256, 256, false, false);
    let normal = target(256, 256, false, false);
    let specular = target(512, 512, true, false);
    let jpegs = [upload_jpeg(), upload_jpeg(), upload_jpeg()];
    let terrain = target(W, H, true, true);
    let bloom_h = target(256, 256, true, false);
    let bloom_v = target(256, 256, true, false);
    let tilt_h = target(W, H, true, false);
    for t in [specular, terrain, bloom_h, bloom_v, tilt_h] {
        genmip(t.tex);
    }
    pass(Some(specular), p0, &[], true);
    genmip(specular.tex);
    Terrain { height, normal, specular, jpegs, terrain, bloom_h, bloom_v, tilt_h, p0, p1, p6 }
}

/// One steady frame, the trace's order (docs/notes/terrain-mip-usage.md).
fn terrain_frame(t: &Terrain) {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, W, H);
        glClearColor(0.0, 0.0, 0.0, 1.0);
        glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);
    }
    pass(Some(t.height), t.p0, &[], true);
    pass(Some(t.normal), t.p1, &[t.height.tex], true);
    let tex = [t.height.tex, t.normal.tex, t.specular.tex, t.jpegs[0], t.jpegs[1], t.jpegs[2]];
    pass(Some(t.terrain), t.p6, &tex, true);
    genmip(t.terrain.tex);
    pass(Some(t.bloom_h), t.p1, &[t.terrain.tex], true);
    genmip(t.bloom_h.tex);
    pass(Some(t.bloom_v), t.p1, &[t.bloom_h.tex], true);
    genmip(t.bloom_v.tex);
    // SAFETY: plain GL calls.
    unsafe {
        glEnable(GL_BLEND);
        glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
    }
    pass(Some(t.terrain), t.p1, &[t.bloom_v.tex], false);
    // SAFETY: a plain GL call.
    unsafe { glDisable(GL_BLEND) };
    genmip(t.terrain.tex);
    pass(Some(t.tilt_h), t.p1, &[t.terrain.tex], true);
    genmip(t.tilt_h.tex);
    pass(None, t.p1, &[t.tilt_h.tex], false);
    swap();
}

/* ---- the counters --------------------------------------------------------------- */

/// What a frame did, as differences of the counters.
#[derive(Debug, PartialEq, Eq)]
struct Frame {
    /// Jobs submitted (`v3d_counters` kicks).
    jobs: u32,
    /// Of them, the jobs whose target stores a render image (`uif_address`).
    imaged: u32,
    /// Draws recorded into the jobs.
    draws: u32,
    /// CPU waits inside the swap (`v3d_finish_previous`).
    swap_waits: u32,
    /// CPU waits the platform saw (OSAL `wait`).
    waits: u32,
    /// `v3d_counters` waits (the backend's count of its waits for a job).
    backend_waits: u32,
    /// TFU jobs queued behind a job (mipmap chains).
    tfu_queued: u32,
    /// TFU jobs run synchronously (`OSAL_QUERY_V3D_TFU_RUN`, the tiling of a
    /// single-level copy).
    tfu_run: u32,
    /// Mip chains the TFU made (`v3d_rt_images` chains).
    chains: u32,
    /// glGenerateMipmap calls refused, by reason (`v3d_mip_counters`).
    refused: Vec<(&'static str, u32)>,
    /// Copies and stores, by reason (`v3d_copy_counters`).
    copies: Vec<(&'static str, u32)>,
}

const COPY_NAMES: [&str; 16] = [
    "plain",
    "r32f",
    "first-sample",
    "cpu-write",
    "relayout",
    "not-stored",
    "no-chain",
    "layout",
    "depth-refused",
    "upload",
    "arena",
    "stamp-replaced",
    "stale-given-back",
    "image-store",
    "level0-back",
    "chain-back",
];
const REFUSAL_NAMES: [&str; 14] = [
    "uninit", "levels", "not-rt", "unknown", "stamp", "relayout", "no-tfu", "flush", "unstored", "wait", "layout",
    "pack", "no-query", "tfu",
];

struct Counts {
    kicks: u32,
    waits: u32,
    chains: u32,
    refused: [u32; 14],
    copies: [u32; 16],
    events: usize,
    seen: usize,
}

fn counts() -> Counts {
    assert_eq!((V3D_COPY_REASONS, V3D_MIP_REFUSALS), (16, 14), "a counter was added: name it above");
    let mut c = Counts {
        kicks: 0,
        waits: 0,
        chains: 0,
        refused: [0; 14],
        copies: [0; 16],
        events: events().len(),
        seen: jobs().len(),
    };
    // SAFETY: live out-parameters of the sizes the counters document; the others null.
    unsafe {
        v3d_counters(&mut c.kicks, &mut c.waits);
        v3d_copy_counters(c.copies.as_mut_ptr());
        v3d_mip_counters(c.refused.as_mut_ptr(), core::ptr::null_mut(), core::ptr::null_mut());
        v3d_rt_images(
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            core::ptr::null_mut(),
            &mut c.chains,
            core::ptr::null_mut(),
        );
    }
    c
}

fn delta<const N: usize>(names: [&'static str; N], now: [u32; N], before: [u32; N]) -> Vec<(&'static str, u32)> {
    (0..N).filter(|&i| now[i] != before[i]).map(|i| (names[i], now[i] - before[i])).collect()
}

/// Run `f` and report what it did.
fn measure(f: impl FnOnce()) -> Frame {
    SWAP_WAITS.store(0, Ordering::Relaxed);
    let before = counts();
    f();
    let now = counts();
    let ev = events()[before.events..].to_vec();
    let count = |p: fn(&Ev) -> bool| ev.iter().filter(|e| p(e)).count() as u32;
    let seen = jobs();
    let new = &seen[before.seen..];
    Frame {
        jobs: now.kicks - before.kicks,
        imaged: new.iter().filter(|j| j.target.uif_address != 0).count() as u32,
        draws: new.iter().map(|j| j.tmu.len() as u32).sum(),
        swap_waits: SWAP_WAITS.swap(0, Ordering::Relaxed),
        waits: count(|e| matches!(e, Ev::Wait(_))),
        backend_waits: now.waits - before.waits,
        tfu_queued: count(|e| matches!(e, Ev::TfuQueued { .. })),
        tfu_run: count(|e| matches!(e, Ev::TfuRan { run: true, .. })),
        chains: now.chains - before.chains,
        refused: delta(REFUSAL_NAMES, now.refused, before.refused),
        copies: delta(COPY_NAMES, now.copies, before.copies),
    }
}

/// A frame as a board run sees it: the frame's calls and the swap's wait,
/// no `glFinish` (the newest job runs on; the next frame's first submit
/// finishes it).
fn frame(t: &Terrain) -> Frame {
    measure(|| terrain_frame(t))
}

/// Frames until two in a row are alike, at most `max`; the steady frame and
/// the number of frames it took (the render-image table needs a few to settle).
fn steady(t: &Terrain, max: usize) -> (Frame, usize) {
    let mut last = frame(t);
    for n in 1..max {
        let f = frame(t);
        if f == last {
            return (f, n);
        }
        last = f;
    }
    panic!("no two frames alike in {max}: {last:?}\n{:?}", logs());
}

/* ---- the FBO scene that leaves stale marks ------------------------------------------- */

/// An FBO scene (glmark2's refract, shadow or ideas in shape): `n` 512x512
/// render targets with chains, each rendered into, mipmapped and sampled,
/// then every texture and framebuffer deleted, as the scene's teardown
/// does. Their slots are handed out again by the next `glGenTextures`.
fn fbo_scene_that_deletes(n: usize) {
    let p1 = program(1);
    let ts: Vec<Target> = (0..n).map(|_| target(512, 512, true, false)).collect();
    for _ in 0..3 {
        for t in &ts {
            pass(Some(*t), p1, &[], true);
            genmip(t.tex);
            pass(None, p1, &[t.tex], false);
        }
        swap();
    }
    finish_all();
    // SAFETY: plain GL calls on names this function made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        for t in &ts {
            glDeleteFramebuffers(1, &t.fbo);
            glDeleteTextures(1, &t.tex);
        }
        glDeleteProgram(p1);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, 0);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/* ---- the tests ----------------------------------------------------------------- */

/// Print what a frame did, for the author who changes a literal below.
fn show(tag: &str, f: &Frame, settled_after: usize) {
    eprintln!("{tag}: settled after {settled_after} frame(s): {f:#?}");
}

/// Terrain alone. The values are the host's at the main tip (`pi4-glmark2-cpu.md`
/// "5 TFU chains a frame", `pi4-slow-terrain.md` "9 render jobs a frame"),
/// pinned as the code gives them.
#[test]
fn a_clean_terrain_frame() {
    let _g = rig();
    let t = terrain_setup();
    let (f, after) = steady(&t, 8);
    show("clean", &f, after);
    /* every frame after it the same */
    for n in 0..3 {
        assert_eq!(frame(&t), f, "frame +{n} differs from the settled one: {:?}", logs());
    }
    /* the board's figure: 9 jobs a frame; 8 draws (the window's clear is the 9th job) */
    assert_eq!((f.jobs, f.draws), (9, 8), "{f:?}");
    /* 5 glGenerateMipmap calls, all made by the TFU behind its render, none refused */
    assert_eq!((f.chains, f.tfu_queued, f.tfu_run), (5, 5, 0), "{f:?}");
    assert_eq!(f.refused, [], "{f:?}");
    /* the 7 jobs into the sampled targets store a render image: no tiled copy */
    assert_eq!(f.imaged, 7, "{f:?}");
    assert_eq!(f.copies, [("image-store", 7)], "{f:?}");
    /* The swap waits for the frame before it. `waits` counts every OSAL wait
     * of this double, whose GPU never finishes on its own (a job completes
     * only when waited for or when the next one is submitted), so it is an
     * upper bound on the board's, which polls `completed`: 9 is one a job.
     * The pins that carry to the board are `swap_waits` and the zero
     * `tfu_run`: no synchronous TFU tiling, no CPU wait to tile. */
    assert_eq!((f.waits, f.backend_waits, f.swap_waits), (9, 9, 1), "{f:?}");
}

/// Terrain after an FBO scene whose textures were rendered into and deleted
/// (the stale-mark case of docs/notes/v3d-terrain-copies.md).
#[test]
fn terrain_after_an_fbo_scene_that_deletes_its_textures() {
    let _g = rig();
    fbo_scene_that_deletes(6);
    let t = terrain_setup();
    let (f, after) = steady(&t, 8);
    show("stale", &f, after);
    assert_eq!((f.jobs, f.draws), (9, 8), "{f:?}");
    /* CHANGES (pi4-target-mark-clear, v3d-rt-evict): the clean frame's values
     * below. Stale marks make the stale uploads render images, the table
     * (8 slots) is overfilled and evicts a target every frame: 3 chains
     * refused `unknown` (the CPU filter, with its GPU drain), 4 tiled copies
     * `first-sample`, 4 jobs store an image where the clean frame has 7. */
    assert_eq!((f.chains, f.tfu_queued, f.tfu_run), (2, 2, 0), "{f:?}");
    assert_eq!(f.refused, [("unknown", 3)], "{f:?}");
    assert_eq!(f.imaged, 4, "{f:?}");
    assert_eq!(f.copies, [("first-sample", 4), ("image-store", 4)], "{f:?}");
    /* CHANGES (the same two items): the clean frame's 1 swap wait; the total
     * stays the double's artifact (see the first test) */
    assert_eq!((f.waits, f.backend_waits, f.swap_waits), (9, 9, 0), "{f:?}");
}
