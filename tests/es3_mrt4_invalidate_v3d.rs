// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glInvalidateFramebuffer and glInvalidateSubFramebuffer (ES 3.0 4.5.1) of
//! GL_COLOR_ATTACHMENT3 over the V3D host back end, with the four-target
//! stand-in on (`backend::HOST_DRAW_TARGETS` = 4: what `DRAW_TARGETS` = 4
//! would be, plan #368 RC14). The rig is tests/es3_invalidate_v3d.rs's: the
//! real GL core and V3D back end, the job builder faked, so every job's
//! target (`V3dJobTarget`: RT 1..3 at `rt[0..3]`, address 0 = not stored) and
//! load words (`V3dJobClear::load_rt`) are seen. RT 3 is `rt[2]` and
//! `load_rt[2]`.
//!
//! What is pinned:
//! * with four colour attachments, GL_COLOR_ATTACHMENT3 alone, with 0..2,
//!   and with depth/stencil is accepted (no error), whole and as a
//!   sub-rectangle, on the draw, read and combined targets;
//! * GL_COLOR_ATTACHMENT4 (m = GL_MAX_COLOR_ATTACHMENTS = 4) is
//!   GL_INVALID_OPERATION; with the stand-in off (three, the build as it
//!   is) GL_COLOR_ATTACHMENT3 is past the limit and GL_INVALID_OPERATION;
//! * the back end: the invalidation is ignored, as the spec allows (the
//!   contents become undefined, not cleared) and as es3_invalidate_v3d.rs
//!   pins for RT 0: src/gl/framebuffer.rs `invalidate_framebuffer` validates
//!   and returns, so the job after it configures, loads and stores RT 3
//!   exactly as without the call (RT 3's address set, so the tile list
//!   stores it; loaded on the frame after a frame). The `#[ignore]`d case
//!   holds the load Mesa's v3d drops for an invalidated buffer
//!   (`v3d_invalidate_resource`), which needs the GL core to pass the
//!   invalidation on (plan #186) and a `GlesDraw` field for it.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test
//! es3_mrt4_invalidate_v3d`.

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
use std::sync::atomic::Ordering;
use std::sync::{Mutex, MutexGuard, Once, OnceLock};

use yakogl::gl::consts::*;
use yakogl::gl::context::{self, GlContext};
use yakogl::gl::draw::glDrawArrays;
use yakogl::gl::framebuffer::*;
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("v3d_depth_storage");
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
    trampoline!(v3d_tex_pack_sampler_compare(out: *mut u8, s: *const V3dSampler, compare_op: u32) -> bool);
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
    four();
    g
}

/// The back end draws four targets (`DRAW_TARGETS` = 4 for the V3D).
fn four() {
    yakogl::backend::HOST_DRAW_TARGETS.store(4, Ordering::Relaxed);
}

/// The build as it is: `DRAW_TARGETS` (3).
fn as_built() {
    yakogl::backend::HOST_DRAW_TARGETS.store(0, Ordering::Relaxed);
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

const SIZE: i32 = 16;
const VS3: &str = "#version 300 es\nin vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
/// Four outputs, one per draw buffer.
const FS3_MRT4: &str = "#version 300 es\nprecision mediump float;\n\
    layout(location = 0) out vec4 c0;\nlayout(location = 1) out vec4 c1;\n\
    layout(location = 2) out vec4 c2;\nlayout(location = 3) out vec4 c3;\n\
    void main() { c0 = vec4(1.0, 0.0, 0.0, 1.0); c1 = vec4(0.0, 1.0, 0.0, 1.0);\n\
    c2 = vec4(0.0, 0.0, 1.0, 1.0); c3 = vec4(0.25, 0.5, 0.75, 1.0); }\n";
/// Three outputs, o0..o2.
const FS3_MRT3: &str = "#version 300 es\nprecision mediump float;\n\
    layout(location = 0) out vec4 c0;\nlayout(location = 1) out vec4 c1;\nlayout(location = 2) out vec4 c2;\n\
    void main() { c0 = vec4(1.0, 0.0, 0.0, 1.0); c1 = vec4(0.0, 1.0, 0.0, 1.0); c2 = vec4(0.0, 0.0, 1.0, 1.0); }\n";
const ALL4: [GLenum; 4] = [GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3];

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

fn rb(format: GLenum) -> GLuint {
    let mut r = 0;
    // SAFETY: room for one name.
    unsafe {
        glGenRenderbuffers(1, &mut r);
        glBindRenderbuffer(GL_RENDERBUFFER, r);
        glRenderbufferStorage(GL_RENDERBUFFER, format, SIZE, SIZE);
    }
    assert_eq!(err(), GL_NO_ERROR, "storage 0x{format:x}");
    r
}

/// A complete framebuffer, bound: `colours` RGBA8 renderbuffers on
/// attachment 0.., and (with `ds`) a D24S8 renderbuffer on the
/// depth-stencil point.
fn fbo(colours: u32, ds: bool) -> GLuint {
    let mut f = 0;
    // SAFETY: room for one name; live renderbuffer names.
    unsafe {
        glGenFramebuffers(1, &mut f);
        glBindFramebuffer(GL_FRAMEBUFFER, f);
        for i in 0..colours {
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0 + i, GL_RENDERBUFFER, rb(GL_RGBA8));
        }
        if ds {
            let z = rb(GL_DEPTH24_STENCIL8);
            glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, z);
        }
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    assert_eq!(err(), GL_NO_ERROR);
    f
}

fn inv(target: GLenum, atts: &[GLenum]) -> GLenum {
    // SAFETY: the slice is live for the call.
    unsafe { glInvalidateFramebuffer(target, atts.len() as GLsizei, atts.as_ptr()) };
    err()
}

fn inv_sub(target: GLenum, atts: &[GLenum], x: i32, y: i32, w: i32, h: i32) -> GLenum {
    // SAFETY: the slice is live for the call.
    unsafe { glInvalidateSubFramebuffer(target, atts.len() as GLsizei, atts.as_ptr(), x, y, w, h) };
    err()
}

/// Jobs begun so far; a test looks at those after the mark.
fn mark() -> usize {
    jobs().len()
}

fn since(m: usize) -> Vec<SeenJob> {
    jobs()[m..].to_vec()
}

/// What a job keeps between jobs: the render targets it configures, RT 0
/// and RT 1..3 loaded, RT 1..3 stored (an address: the tile list stores the
/// RT; 0 is a GL_NONE draw buffer, never loaded or stored), depth/stencil
/// loaded and stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Words {
    rts: u32,
    load_rt0: bool,
    load_rt: [bool; 3],
    store_rt: [bool; 3],
    zs: (bool, bool),
}

fn words(j: &SeenJob) -> Words {
    let c = j.clear.expect("the job was ended");
    Words {
        rts: j.target.rts,
        load_rt0: c.load_rt0,
        load_rt: c.load_rt,
        store_rt: j.target.rt.map(|r| r.address != 0),
        zs: (c.zs_load, c.zs_store),
    }
}

/// RT 3's words: (configured, loaded, stored).
fn rt3(w: &Words) -> (bool, bool, bool) {
    (w.rts >= 4, w.load_rt[2], w.store_rt[2])
}

fn draw_buffers(bufs: &[GLenum]) {
    // SAFETY: a live array.
    unsafe { glDrawBuffers(bufs.len() as GLsizei, bufs.as_ptr()) };
    assert_eq!(err(), GL_NO_ERROR, "glDrawBuffers({bufs:x?})");
}

fn get(pname: GLenum) -> GLint {
    let mut v = -1;
    // SAFETY: room for one value.
    unsafe { glGetIntegerv(pname, &mut v) };
    v
}

/// One frame drawn into the bound four-target framebuffer, then (when
/// `atts` is given) the invalidation, then the frame looked at: its jobs'
/// words.
fn frame_after(prog: GLuint, atts: Option<(&[GLenum], bool)>) -> Vec<Words> {
    draw_with(prog); /* the previous frame */
    if let Some((atts, sub)) = atts {
        let e = if sub { inv_sub(GL_FRAMEBUFFER, atts, 0, 0, 8, 8) } else { inv(GL_FRAMEBUFFER, atts) };
        assert_eq!(e, GL_NO_ERROR, "{atts:x?} (sub {sub})");
    }
    let m = mark();
    draw_with(prog);
    since(m).iter().map(words).collect()
}

fn draw_with(prog: GLuint) {
    // SAFETY: plain GL calls.
    unsafe {
        glUseProgram(prog);
        glEnable(GL_DEPTH_TEST);
    }
    draw_quad();
    // SAFETY: plain GL call.
    unsafe { glFinish() };
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

fn teardown(f: GLuint) {
    // SAFETY: live names.
    unsafe {
        glDisable(GL_DEPTH_TEST);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &f);
    }
}

/// The stand-in is on: GL reports four, and four RGBA8 attachments make a
/// complete framebuffer.
#[test]
fn the_stand_in_reports_four() {
    let _g = rig();
    assert_eq!((get(GL_MAX_DRAW_BUFFERS), get(GL_MAX_COLOR_ATTACHMENTS)), (4, 4));
    let f = fbo(4, false);
    draw_buffers(&ALL4);
    teardown(f);
}

/// GL_COLOR_ATTACHMENT3 alone, with 0..2, with depth/stencil, whole and as
/// a sub-rectangle, on every framebuffer target: no error.
#[test]
fn colour_attachment_3_is_accepted() {
    let _g = rig();
    let f = fbo(4, true);
    draw_buffers(&ALL4);
    let lists: [&[GLenum]; 5] = [
        &[GL_COLOR_ATTACHMENT3],
        &ALL4,
        &[GL_COLOR_ATTACHMENT3, GL_COLOR_ATTACHMENT0],
        &[GL_COLOR_ATTACHMENT2, GL_COLOR_ATTACHMENT3],
        &[GL_COLOR_ATTACHMENT3, GL_DEPTH_ATTACHMENT, GL_STENCIL_ATTACHMENT, GL_DEPTH_STENCIL_ATTACHMENT],
    ];
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
        for atts in lists {
            assert_eq!(inv(t, atts), GL_NO_ERROR, "0x{t:x} {atts:x?}");
            assert_eq!(inv_sub(t, atts, 0, 0, SIZE, SIZE), GL_NO_ERROR, "sub 0x{t:x} {atts:x?}");
            assert_eq!(inv_sub(t, atts, 2, 3, 5, 7), GL_NO_ERROR, "sub 0x{t:x} {atts:x?}");
        }
        /* a rectangle past the surface, and an empty one: legal */
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT3], -4, -4, 100, 100), GL_NO_ERROR);
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT3], 0, 0, 0, 0), GL_NO_ERROR);
    }
    teardown(f);
    /* a point with no image is still a point: three attachments, point 3 */
    let f = fbo(3, false);
    assert_eq!(inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), GL_NO_ERROR, "point 3 with no image");
    teardown(f);
}

/// m = GL_MAX_COLOR_ATTACHMENTS (4) is GL_INVALID_OPERATION (ES 3.0 4.5.1),
/// alone or after legal names; with the stand-in off (three) point 3 is.
#[test]
fn colour_attachment_4_is_invalid_operation() {
    let _g = rig();
    let f = fbo(4, false);
    draw_buffers(&ALL4);
    for t in [GL_FRAMEBUFFER, GL_DRAW_FRAMEBUFFER, GL_READ_FRAMEBUFFER] {
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION, "0x{t:x}");
        assert_eq!(inv_sub(t, &[GL_COLOR_ATTACHMENT4], 0, 0, 4, 4), GL_INVALID_OPERATION, "sub 0x{t:x}");
        assert_eq!(inv(t, &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT3, GL_COLOR_ATTACHMENT4]), GL_INVALID_OPERATION);
    }
    as_built();
    assert_eq!(get(GL_MAX_COLOR_ATTACHMENTS), 3);
    let r =
        (inv(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3]), inv_sub(GL_FRAMEBUFFER, &[GL_COLOR_ATTACHMENT3], 0, 0, 4, 4));
    four();
    assert_eq!(r, (GL_INVALID_OPERATION, GL_INVALID_OPERATION), "three targets: point 3 is past the limit");
    teardown(f);
}

/// The draw after an invalidation of RT 3 is a job that configures,
/// loads and stores RT 3 just as the draw after none: the back end never
/// hears of the call (src/gl/framebuffer.rs `invalidate_framebuffer`).
#[test]
fn rt3_is_loaded_and_stored_as_without_the_call() {
    let _g = rig();
    let prog = program(VS3, FS3_MRT4);
    let f = fbo(4, false);
    draw_buffers(&ALL4);
    let base = frame_after(prog, None);
    assert!(!base.is_empty(), "the draw made a job");
    assert_eq!(base[0].rts, 4, "four render targets configured: {:?}", base[0]);
    assert_eq!(rt3(&base[0]), (true, true, true), "the frame after a frame loads and stores RT 3: {:?}", base[0]);
    assert!(base[0].load_rt0 && base[0].load_rt == [true; 3] && base[0].store_rt == [true; 3], "{:?}", base[0]);
    let lists: [&[GLenum]; 4] = [
        &[GL_COLOR_ATTACHMENT3],
        &ALL4,
        &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT3],
        &[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2],
    ];
    for atts in lists {
        for sub in [false, true] {
            assert_eq!(frame_after(prog, Some((atts, sub))), base, "{atts:x?} (sub {sub})");
        }
    }
    teardown(f);
}

/// GL_NONE on draw buffer 3: RT 3 is not configured nor stored, with or
/// without an invalidation of it. The program writes o0..o2 only: one that
/// also writes o3 is refused today ("a colour output with no attachment
/// slot", src/v3d/backend.rs `draw`, the rt_mask check), a named limit that
/// is no part of the invalidation.
#[test]
fn rt3_as_gl_none_is_neither_loaded_nor_stored() {
    let _g = rig();
    let prog = program(VS3, FS3_MRT3);
    let f = fbo(4, false);
    draw_buffers(&[GL_COLOR_ATTACHMENT0, GL_COLOR_ATTACHMENT1, GL_COLOR_ATTACHMENT2, GL_NONE]);
    let base = frame_after(prog, None);
    assert!(!base.is_empty(), "the draw made a job");
    /* a trailing GL_NONE: the job configures three RTs, so RT 3 is not in
     * the tile list at all (its `load_rt[2]` word is not read then) */
    assert_eq!((base[0].rts, base[0].store_rt[2]), (3, false), "GL_NONE RT 3: {:?}", base[0]);
    assert_eq!(frame_after(prog, Some((&[GL_COLOR_ATTACHMENT3], false))), base);
    teardown(f);
}

/* The load a back end would skip. Ignored: nothing tells the back end. */

#[test]
#[ignore = "glInvalidateFramebuffer is a validated no-op in src/gl/framebuffer.rs (invalidate_framebuffer, plan #186) and GlesDraw has no field for it: the next job loads RT 3"]
fn an_invalidated_rt3_is_not_loaded_by_the_next_job() {
    let _g = rig();
    let prog = program(VS3, FS3_MRT4);
    let f = fbo(4, false);
    draw_buffers(&ALL4);
    let w = frame_after(prog, Some((&[GL_COLOR_ATTACHMENT3], false)));
    assert!(!w[0].load_rt[2], "Mesa drops the load of an invalidated resource");
    assert!(w[0].load_rt0 && w[0].load_rt[..2] == [true; 2], "the others are kept");
    teardown(f);
}
