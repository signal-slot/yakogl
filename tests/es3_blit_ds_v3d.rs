// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glBlitFramebuffer between depth/stencil renderbuffers through the GL API
//! over the V3D host back end: the real GL core and V3D back end, the job
//! builder faked (tests/es3_invalidate_v3d.rs's rig), the 16
//! `fbo.blit.depth_stencil` shapes of dEQP-GLES3 (docs/notes/v3d-fbo-blit.md
//! 2, 3.4, 3.5, 5 issue draft 1; Pi 4 FB2 fails all 16).
//!
//! tests/es3_blit_ds.rs is the GL core's half on a stand-in back end that
//! has `ds_write`/`stencil_resolve`; this file is the V3D's, whose table has
//! neither (`ds_write: None, stencil_resolve: None`, src/v3d/backend.rs).
//!
//! Per shape, in a module of its own: `depth_component16_basic` ..
//! `stencil_index8_scale` (depth_component16/24/32f basic and scale;
//! depth24_stencil8 and depth32f_stencil8 basic, scale, depth_only,
//! stencil_only; stencil_index8 basic and scale):
//! * `is_refused_today` pins the answer of today's V3D: GL_INVALID_OPERATION
//!   (src/gl/blit.rs REFUSED_DEPTH_DST for a depth blit into a renderbuffer,
//!   REFUSED_STENCIL for a stencil one), one error, the destination left as
//!   cleared, the context usable;
//! * `reads_back_the_source` (`#[ignore]`d) holds what a working blit leaves:
//!   each destination pixel takes the nearest source pixel in the planes the
//!   mask names, the others keep their clear; the planes are read through
//!   `v3d_depth_resolve` and the table's `stencil_resolve`. It runs when the
//!   two table entries exist (issue draft 1: a CPU read-modify-write of the
//!   `ZsBuf` through `v3d_tex_load_box`/`v3d_tex_store_box`).
//!
//! Open in the note and left open here: D32F is a 24-bit round trip (the
//! buffer is D24S8); the ignored test pins that the round trip is exact in
//! 24 bits, not that dEQP's tolerance accepts it (not measured).
//!
//! The sources are filled by the fake GPU (`depth_word`: a depth per texel,
//! stencil byte 0x3c) after a clear; the destinations are clear-only.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test es3_blit_ds_v3d`.

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
use yakogl::gl::names::{self, Kind};
use yakogl::gl::program::*;
use yakogl::gl::shader::*;
use yakogl::gl::state::*;
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_blit_ds_v3d");
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
unsafe extern "C" fn v3d_frame_alloc_sizes(t: *const V3dTiling, _draws: u32, _page: u32, ta: *mut u32, ts: *mut u32) {
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
    /// Bytes of the live allocations and their high-water since the last
    /// `mark_peak`: the carve-out use as the GL core and the back end make it.
    live: usize,
    peak: usize,
}

static PLATFORM: Mutex<Option<Platform>> = Mutex::new(None);

fn with_platform<R>(f: impl FnOnce(&mut Platform) -> R) -> R {
    let mut g = PLATFORM.lock().unwrap_or_else(|e| e.into_inner());
    f(g.get_or_insert_with(Platform::default))
}

/// The D24S8 word the "GPU" stores at (x, y) in the job of fence `fence`:
/// a depth that differs per texel and per frame in bits 31..8, and a
/// stencil byte the texture unit and the resolve must both ignore.
fn depth_word(fence: usize, x: u32, y: u32) -> u32 {
    let d = (x * 7 + y * 13 + fence as u32 * 1009) & 0x00ff_ffff;
    (d << 8) | 0x3c
}

/// The CPU view of `n` bytes at GPU address `gpu`.
fn platform_cpu(p: &mut Platform, gpu: u64, n: usize) -> *mut u8 {
    let (base, a) = p.allocs.range_mut(..=gpu).next_back().expect("an allocation below the address");
    let off = (gpu - base) as usize;
    assert!(off + n <= a.size, "0x{gpu:x}+{n} is past its allocation");
    // SAFETY: inside the allocation.
    unsafe { (a.mem.as_mut_ptr() as *mut u8).add(off) }
}

/// A one-level D24S8 layout as the backend lays out a depth/stencil buffer
/// (`zs_buf`: `v3d_tex_layout(w, h, 1, 4)` with the UIF XOR dropped), from
/// the real C.
fn zs_layout(w: u32, h: u32) -> V3dTexLayout {
    let mut l = V3dTexLayout::default();
    // SAFETY: a live layout to fill.
    assert!(unsafe { v3d_tex_layout(w, h, 1, 4, &mut l) }, "{w}x{h} lays out");
    if l.slices[0].tiling == V3D_TEX_TILING_UIF_XOR {
        l.slices[0].tiling = V3D_TEX_TILING_UIF_NO_XOR;
    }
    l
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
        p.live += size;
        p.peak = p.peak.max(p.live);
        0
    })
}

unsafe extern "C" fn p_gpu_free(mem: *mut OsalGpuMem) -> i32 {
    // SAFETY: the caller passes its descriptor.
    let m = unsafe { *mem };
    if !m.impl_.is_null() {
        with_platform(|p| {
            if let Some(a) = p.allocs.remove(&m.gpu) {
                p.live -= a.size;
            }
        });
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
/// and a job that stores its depth/stencil buffer leaves `depth_word` in
/// every texel of it, tiled as the TLB tiles it (the layout `zs_buf` gave
/// the buffer, which the job's clear names).
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
    let c = job.clear.expect("the job was ended");
    if c.zs_store && c.zs_address != 0 {
        let (w, h) = (job.target.width, job.target.height);
        let l = zs_layout(w, h);
        assert_eq!(c.zs_memory_format, l.slices[0].tiling, "the store's memory format is the layout's tiling");
        let words: Vec<u32> = (0..w * h).map(|i| depth_word(fence, i % w, i / w)).collect();
        with_platform(|p| {
            let image = platform_cpu(p, u64::from(c.zs_address), l.size as usize);
            // SAFETY: the buffer's live mapping holds the laid-out level
            // (platform_cpu checked l.size bytes); `words` is h rows of w words.
            unsafe { v3d_tex_store_level(image, &l, 0, words.as_ptr() as *const u8, w * 4) };
        });
    }
    fence as i64
}

unsafe extern "C" fn p_wait(_fence: i64, _timeout_us: i64) -> i32 {
    0
}

unsafe extern "C" fn p_completed() -> i64 {
    with_platform(|p| p.kicks.len() as i64)
}

unsafe extern "C" fn p_query(what: u32, out: *mut c_void, out_sz: usize) -> i32 {
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
    -22 /* no TFU, the default budgets, no clock */
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

/* ---- the context ------------------------------------------------------------- */

/// The default framebuffer's size.
const CANVAS: (i32, i32) = (40, 30);

static LOCK: Mutex<()> = Mutex::new(());
static ONCE: Once = Once::new();
static mut CTX: *mut GlContext = core::ptr::null_mut();

/// The one context and backend of the file, set up once; the tests take
/// turns.
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

/* ---- the tests ----------------------------------------------------------------- */

const VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
const FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(0.25, 0.5, 0.75, 1.0); }\n";

/// The source and destination renderbuffers' initial clears: what a
/// destination holds before a blit and what a source holds when no draw
/// ramped it.
const SRC_CLEAR: (f32, i32) = (0.25, 0x5a);
const DST_CLEAR: (f32, i32) = (0.75, 0x11);

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// One `fbo.blit.depth_stencil.*` case: the renderbuffer format on both
/// sides, where it attaches, the blit mask, and whether the destination is
/// another size than the source (the `_scale` cases).
#[derive(Clone, Copy)]
struct Shape {
    format: GLenum,
    point: GLenum,
    mask: GLbitfield,
    scale: bool,
}

impl Shape {
    fn has_depth(&self) -> bool {
        self.format != GL_STENCIL_INDEX8
    }
    fn has_stencil(&self) -> bool {
        matches!(self.format, GL_DEPTH24_STENCIL8 | GL_DEPTH32F_STENCIL8 | GL_STENCIL_INDEX8)
    }
    /// The size of the source and of the destination (dEQP scales the
    /// rectangle by a non-integer factor in each axis, here 32x32 -> 20x40).
    fn sizes(&self) -> ((i32, i32), (i32, i32)) {
        ((32, 32), if self.scale { (20, 40) } else { (32, 32) })
    }
}

const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;

/// A `w` x `h` renderbuffer of `format` at `point` of a new framebuffer,
/// checked complete and bound: (framebuffer, renderbuffer).
fn ds_fbo(format: GLenum, point: GLenum, w: i32, h: i32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, format, w, h);
        assert_eq!(glGetError(), GL_NO_ERROR, "storage 0x{format:x}: {:?}", logs());
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, point, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "0x{format:x}: {:?}", logs());
    }
    (fb, rb)
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

/// The bound framebuffer's depth/stencil cleared to `(depth, stencil)`
/// (the planes its format has) and, with `ramp`, a depth-tested draw after
/// it: the fake GPU stores `depth_word` (a depth that differs per texel,
/// stencil byte 0x3c) into every buffer a job stores.
fn fill(s: &Shape, clear: (f32, i32), ramp: bool, prog: GLuint) {
    let bits =
        if s.has_depth() { GL_DEPTH_BUFFER_BIT } else { 0 } | if s.has_stencil() { GL_STENCIL_BUFFER_BIT } else { 0 };
    // SAFETY: plain GL calls.
    unsafe {
        glClearDepthf(clear.0);
        glClearStencil(clear.1);
        glClear(bits);
        if ramp && s.has_depth() {
            glUseProgram(prog);
            glEnable(GL_DEPTH_TEST);
        }
    }
    if ramp && s.has_depth() {
        draw_quad();
        // SAFETY: plain GL call.
        unsafe { glDisable(GL_DEPTH_TEST) };
    }
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

struct Pair {
    src_fb: GLuint,
    src_rb: GLuint,
    dst_fb: GLuint,
    dst_rb: GLuint,
    prog: GLuint,
}

/// The source (ramped by a draw where it has depth, else cleared) and the
/// destination (cleared to `DST_CLEAR`), both bound as read and draw
/// framebuffers of the blit.
fn pair(s: &Shape) -> Pair {
    let ((sw, sh), (dw, dh)) = s.sizes();
    let prog = program(VS, FS);
    let (src_fb, src_rb) = ds_fbo(s.format, s.point, sw, sh);
    fill(s, SRC_CLEAR, true, prog);
    let (dst_fb, dst_rb) = ds_fbo(s.format, s.point, dw, dh);
    fill(s, DST_CLEAR, false, prog);
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, src_fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst_fb);
    }
    Pair { src_fb, src_rb, dst_fb, dst_rb, prog }
}

fn blit(s: &Shape) -> GLenum {
    let ((sw, sh), (dw, dh)) = s.sizes();
    // SAFETY: plain GL calls.
    unsafe {
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, sw, sh, 0, 0, dw, dh, s.mask, GL_NEAREST);
    }
    err()
}

fn release(p: &Pair) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(2, [p.src_fb, p.dst_fb].as_ptr());
        glDeleteRenderbuffers(2, [p.src_rb, p.dst_rb].as_ptr());
        glDeleteProgram(p.prog);
    }
}

/// The depth plane of renderbuffer `rb` (size `w` x `h`) as the V3D back
/// end resolves it (24-bit: `n / 16777215`).
fn read_depth(rb: GLuint, w: i32, h: i32) -> Vec<f32> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { v3d_depth_resolve(ds_id(rb), v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the depth of the renderbuffer");
    v
}

/// The stencil plane through the back end's `stencil_resolve` entry, which
/// the V3D table lacks today (`None`: the assertion is what stands between).
fn read_stencil(rb: GLuint, w: i32, h: i32) -> Vec<u8> {
    let f = yakogl_backend_v3d.stencil_resolve.expect("the V3D table has a stencil_resolve entry (None today)");
    let mut v = vec![0xffu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { f(ds_id(rb), v.as_mut_ptr(), w as u32, h as u32, w as u32) };
    assert_eq!(r, 0, "the back end holds the stencil of the renderbuffer");
    v
}

/// The source pixel nearest-filtering gives destination pixel `o` of `d`
/// over `s` (ES 3.0.6 4.3.3, the whole source onto the whole destination).
fn src_of(o: i32, s: i32, d: i32) -> i32 {
    (((o as f64 + 0.5) * s as f64 / d as f64).floor() as i32).min(s - 1)
}

/// The expected destination plane: the source plane scaled by the model.
fn scaled<T: Copy>(src: &[T], s: &Shape) -> Vec<T> {
    let ((sw, sh), (dw, dh)) = s.sizes();
    (0..dh)
        .flat_map(|y| (0..dw).map(move |x| (x, y)))
        .map(|(x, y)| src[(src_of(y, sh, dh) * sw + src_of(x, sw, dw)) as usize])
        .collect()
}

/// What the V3D answers today to all 16 shapes: GL_INVALID_OPERATION,
/// nothing copied, the context usable. `ds_write` and `stencil_resolve` are
/// `None` in the V3D table (src/v3d/backend.rs), so src/gl/blit.rs refuses a
/// depth blit whose destination is a renderbuffer (REFUSED_DEPTH_DST) and a
/// stencil blit between renderbuffers (REFUSED_STENCIL), before any plane is
/// read or written.
fn check_refused(s: Shape) {
    let _g = rig();
    let p = pair(&s);
    let ((_, _), (dw, dh)) = s.sizes();
    assert!(yakogl_backend_v3d.ds_write.is_none() && yakogl_backend_v3d.stencil_resolve.is_none());
    assert_eq!(blit(&s), GL_INVALID_OPERATION);
    assert_eq!(err(), GL_NO_ERROR, "one error, raised once");
    /* nothing was copied: the destination still holds its clear */
    if s.mask & DEPTH != 0 {
        assert!(read_depth(p.dst_rb, dw, dh).iter().all(|&d| d == DST_CLEAR.0), "the destination depth is as cleared");
    }
    /* the context is usable: a clear and a read of the next framebuffer work */
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glClear(GL_COLOR_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    release(&p);
}

/// What a blit that works leaves in the destination: every destination
/// pixel takes the nearest source pixel, in each plane the mask names; a
/// plane it does not name keeps its clear.
fn check_readback(s: Shape) {
    let _g = rig();
    let p = pair(&s);
    let ((sw, sh), (dw, dh)) = s.sizes();
    let src_depth = s.has_depth().then(|| read_depth(p.src_rb, sw, sh));
    let src_stencil = s.has_stencil().then(|| read_stencil(p.src_rb, sw, sh));
    assert_eq!(blit(&s), GL_NO_ERROR, "{:?}", logs());
    if s.has_depth() {
        let want = if s.mask & DEPTH != 0 {
            scaled(src_depth.as_ref().unwrap(), &s)
        } else {
            vec![DST_CLEAR.0; (dw * dh) as usize]
        };
        let got = read_depth(p.dst_rb, dw, dh);
        /* the buffer holds 24 bits: a D32F source reads back as n / 16777215
         * and is written back by round(v * 0xffffff), so a 24-bit round
         * trip is exact. Whether dEQP's tolerance for the depth32f cases
         * accepts a 24-bit buffer is NOT measured (docs/notes/v3d-fbo-blit.md
         * 3.4, "Precision"): this test pins the exact round trip, not the
         * dEQP verdict. */
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert!((g - w).abs() <= 0.5 / 16_777_215.0, "depth pixel {i}: {g} want {w}");
        }
    }
    if s.has_stencil() {
        let want = if s.mask & STENCIL != 0 {
            scaled(src_stencil.as_ref().unwrap(), &s)
        } else {
            vec![DST_CLEAR.1 as u8; (dw * dh) as usize]
        };
        assert_eq!(read_stencil(p.dst_rb, dw, dh), want, "the stencil plane");
    }
    release(&p);
}

macro_rules! shapes {
    ($($name:ident: $format:expr, $point:expr, $mask:expr, $scale:expr;)*) => {$(
        mod $name {
            use super::*;
            const S: Shape = Shape { format: $format, point: $point, mask: $mask, scale: $scale };
            #[test]
            fn is_refused_today() {
                check_refused(S);
            }
            #[test]
            #[ignore = "ds_write and stencil_resolve are None in the V3D table (docs/notes/v3d-fbo-blit.md 3.4)"]
            fn reads_back_the_source() {
                check_readback(S);
            }
        }
    )*};
}

const D: GLenum = GL_DEPTH_ATTACHMENT;
const DS: GLenum = GL_DEPTH_STENCIL_ATTACHMENT;
const DS_MASK: GLbitfield = DEPTH | STENCIL;

shapes! {
    depth_component16_basic: GL_DEPTH_COMPONENT16, D, DEPTH, false;
    depth_component16_scale: GL_DEPTH_COMPONENT16, D, DEPTH, true;
    depth_component24_basic: GL_DEPTH_COMPONENT24, D, DEPTH, false;
    depth_component24_scale: GL_DEPTH_COMPONENT24, D, DEPTH, true;
    depth_component32f_basic: GL_DEPTH_COMPONENT32F, D, DEPTH, false;
    depth_component32f_scale: GL_DEPTH_COMPONENT32F, D, DEPTH, true;
    depth24_stencil8_basic: GL_DEPTH24_STENCIL8, DS, DS_MASK, false;
    depth24_stencil8_scale: GL_DEPTH24_STENCIL8, DS, DS_MASK, true;
    depth24_stencil8_depth_only: GL_DEPTH24_STENCIL8, DS, DEPTH, false;
    depth24_stencil8_stencil_only: GL_DEPTH24_STENCIL8, DS, STENCIL, false;
    depth32f_stencil8_basic: GL_DEPTH32F_STENCIL8, DS, DS_MASK, false;
    depth32f_stencil8_scale: GL_DEPTH32F_STENCIL8, DS, DS_MASK, true;
    depth32f_stencil8_depth_only: GL_DEPTH32F_STENCIL8, DS, DEPTH, false;
    depth32f_stencil8_stencil_only: GL_DEPTH32F_STENCIL8, DS, STENCIL, false;
    stencil_index8_basic: GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, STENCIL, false;
    stencil_index8_scale: GL_STENCIL_INDEX8, GL_STENCIL_ATTACHMENT, STENCIL, true;
}

/// The refusal is named in the log, once per cause, not once per call:
/// a depth blit into a renderbuffer says `REFUSED_DEPTH_DST`, a stencil one
/// `REFUSED_STENCIL` (src/gl/blit.rs `refuse_ds`).
#[test]
fn each_refusal_is_named_once_in_the_log() {
    let _g = rig();
    let depth = Shape { format: GL_DEPTH_COMPONENT24, point: D, mask: DEPTH, scale: false };
    let stencil = Shape { format: GL_STENCIL_INDEX8, point: GL_STENCIL_ATTACHMENT, mask: STENCIL, scale: false };
    for s in [depth, stencil] {
        for _ in 0..2 {
            let p = pair(&s);
            assert_eq!(blit(&s), GL_INVALID_OPERATION);
            release(&p);
        }
    }
    let said = |what: &str| logs().iter().filter(|l| l.starts_with("glBlitFramebuffer:") && l.contains(what)).count();
    assert_eq!(said("a depth blit into the window or into a renderbuffer of a back end without the ds_write entry"), 1);
    assert_eq!(said("a stencil blit other than between renderbuffers"), 1);
}

/// "Nothing is copied, the colour neither" (src/gl/blit.rs): a blit of
/// colour and depth whose depth half is refused fails whole (the colour
/// blit alone, between the same framebuffers, is fine). The colour pixels are
/// not read back: the faked job builder draws and clears nothing.
#[test]
fn a_refused_depth_half_fails_the_colour_half_too() {
    let _g = rig();
    let make = |clear: [f32; 4]| {
        let (mut fb, mut cb, mut zb) = (0, 0, 0);
        // SAFETY: room for one name each; plain GL calls.
        unsafe {
            glGenRenderbuffers(1, &mut cb);
            glBindRenderbuffer(GL_RENDERBUFFER, cb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, 16, 16);
            glGenRenderbuffers(1, &mut zb);
            glBindRenderbuffer(GL_RENDERBUFFER, zb);
            glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, 16, 16);
            glGenFramebuffers(1, &mut fb);
            glBindFramebuffer(GL_FRAMEBUFFER, fb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, cb);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, zb);
            assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
            glClearColor(clear[0], clear[1], clear[2], clear[3]);
            glClearDepthf(0.5);
            glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
            glFinish();
        }
        (fb, cb, zb)
    };
    let (sfb, scb, szb) = make([1.0, 0.0, 0.0, 1.0]);
    let (dfb, dcb, dzb) = make([0.0, 0.0, 1.0, 1.0]);
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, sfb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dfb);
        while glGetError() != GL_NO_ERROR {}
        glBlitFramebuffer(0, 0, 16, 16, 0, 0, 16, 16, GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        assert_eq!(glGetError(), GL_INVALID_OPERATION);
        /* the colour half alone is not refused: the refusal is the depth plane's */
        glBlitFramebuffer(0, 0, 16, 16, 0, 0, 16, 16, GL_COLOR_BUFFER_BIT, GL_NEAREST);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(2, [sfb, dfb].as_ptr());
        glDeleteRenderbuffers(4, [scb, szb, dcb, dzb].as_ptr());
    }
}
