// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! glInvalidateFramebuffer / glInvalidateSubFramebuffer on an FBO's depth and
//! stencil attachments, then glBlitFramebuffer of that FBO's depth/stencil
//! into the window, through the GL API over the V3D host back end (plan item
//! invalidate-blit-test-v3d). This is the sequence of the six dEQP-GLES3 cases
//! fbo.invalidate.{whole,sub}.unbind_blit_{depth,stencil,depth_stencil}
//! (HOST-ONLY: an API verdict, never a picture). dEQP's EGL surface is a
//! pbuffer, so the window is stored bottom-up: `default_y_up` is true.
//!
//! The rig is tests/es3_blit_ds_window_v3d.rs's, copied: the real GL core and
//! the real V3D entries, the job builder faked. Both planes of the window and
//! of a D24S8 renderbuffer are set through the back end's own `ds_write` to a
//! distinct value per pixel and read back through the resolves.
//! tests/es3_blit_ds_window_v3d.rs pins the window blit without the
//! invalidation, tests/es3_invalidate_v3d.rs the invalidation without the blit.
//!
//! Pinned (the gl-ds-window lift is in src/gl/blit.rs): the invalidation
//! raises no error, whole or as a sub-rectangle, for GL_DEPTH_ATTACHMENT,
//! GL_STENCIL_ATTACHMENT and GL_DEPTH_STENCIL_ATTACHMENT, and the blit into
//! the window that follows is GL_NO_ERROR for depth, stencil and
//! depth|stencil. After a whole invalidation the contents are undefined
//! (ES 3.0.6 4.5.1), so only the error is asserted. After a sub-rectangle
//! invalidation the pixels outside the rectangle reach the window with the
//! source values (no vertical mirror: `default_y_up`); the pixels inside it
//! are not asserted, and a plane the mask does not name is unchanged.
//!
//! Left open: the renderbuffer is D24S8 and the window single-sampled; the
//! GL core ignores the invalidation (tests/es3_invalidate_v3d.rs), so the
//! "undefined" contents are not exercised. Nothing here ran on a Pi 4.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test
//! es3_invalidate_blit_v3d`.

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
use yakogl::gl::framebuffer::*;
use yakogl::gl::names::{self, Kind};
use yakogl::gl::state::*;
use yakogl::gl::types::*;
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_invalidate_blit_v3d");
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
    g
}

fn logs() -> Vec<String> {
    with_platform(|p| p.log.clone())
}

/* ---- the window, the planes, the model ------------------------------------- */

/// The window's size (`CANVAS`).
const WW: u32 = CANVAS.0 as u32;
const WH: u32 = CANVAS.1 as u32;
const D: u32 = yakogl::backend::DS_PLANE_DEPTH;
const S: u32 = yakogl::backend::DS_PLANE_STENCIL;
const DEPTH: GLbitfield = GL_DEPTH_BUFFER_BIT;
const STENCIL: GLbitfield = GL_STENCIL_BUFFER_BIT;
/// 24-bit depth: the unit the buffer holds.
const UNORM: f32 = 16_777_215.0;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// Store the window bottom-up (`true`) or top-down (`false`).
fn window_y_up(up: bool) {
    let c = context::current().expect("a current context");
    assert_eq!((c.target_w, c.target_h), (WW, WH));
    c.default_y_up = up;
}

/// The id the back end keeps the window's depth and stencil under (the
/// formula of gl/draw.rs).
fn window_id() -> u32 {
    let c = context::current().expect("a current context");
    assert!(c.default_surface_valid);
    0x8000_0000 | (c.default_surface.gpu >> 12) as u32
}

/// The id the back end keeps renderbuffer `rb`'s depth and stencil under.
fn ds_id(rb: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Renderbuffer, rb).expect("a renderbuffer name");
    context::ds_object_id(c.renderbuffer_object(slot).expect("a renderbuffer").serial, false)
}

/// Make the window's depth/stencil buffer exist: clear both through the GL
/// API on framebuffer 0 (as a renderbuffer's is made by its first clear).
fn make_window_ds() {
    // SAFETY: plain GL calls.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glClearDepthf(0.5);
        glClearStencil(0x21);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

/// A `w` x `h` D24S8 renderbuffer at GL_DEPTH_STENCIL_ATTACHMENT of a new
/// framebuffer, cleared (so its buffer exists): (framebuffer, renderbuffer).
fn rb_fbo(w: u32, h: u32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_DEPTH24_STENCIL8, w as i32, h as i32);
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_DEPTH_STENCIL_ATTACHMENT, GL_RENDERBUFFER, rb);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glClearDepthf(0.25);
        glClearStencil(0x5a);
        glClear(GL_DEPTH_BUFFER_BIT | GL_STENCIL_BUFFER_BIT);
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    (fb, rb)
}

fn release(fbs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/// A `w` x `h` plane (row-major, memory order) of `f(x, row)`.
fn plane<T>(w: u32, h: u32, f: impl Fn(u32, u32) -> T) -> Vec<T> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

/// Write a whole plane through the table's `ds_write` of object `id`.
fn write_depth(id: u32, w: u32, h: u32, v: &[f32]) {
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { v3d_ds_write(id, D, v.as_ptr() as *const c_void, 0, 0, w, h, w) };
    assert_eq!(r, 0, "ds_write of the depth of 0x{id:x}: {:?}", logs());
}
fn write_stencil(id: u32, w: u32, h: u32, v: &[u8]) {
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { v3d_ds_write(id, S, v.as_ptr() as *const c_void, 0, 0, w, h, w) };
    assert_eq!(r, 0, "ds_write of the stencil of 0x{id:x}: {:?}", logs());
}

/// The planes as the back end resolves them (memory order, row 0 first).
fn read_depth(id: u32, w: u32, h: u32) -> Vec<f32> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { v3d_depth_resolve(id, v.as_mut_ptr(), w, h, w) };
    assert_eq!(r, 0, "depth_resolve of 0x{id:x}: {:?}", logs());
    v
}
fn read_stencil(id: u32, w: u32, h: u32) -> Vec<u8> {
    let f = yakogl_backend_v3d.stencil_resolve.expect("the V3D table has a stencil_resolve entry");
    let mut v = vec![0xffu8; (w * h) as usize];
    // SAFETY: `h` rows of `w` bytes.
    let r = unsafe { f(id, v.as_mut_ptr(), w, h, w) };
    assert_eq!(r, 0, "stencil_resolve of 0x{id:x}: {:?}", logs());
    v
}

/* Patterns: a distinct value per pixel, exact in 24 bits (depth) and a byte
 * that is never 0 (stencil). `win_*` at the window's MEMORY (x, row); `rb_*`
 * at a renderbuffer's (x, y); `dst_*` what a renderbuffer destination held. */
fn win_depth(x: u32, r: u32) -> f32 {
    ((r * WW + x + 1) * 997) as f32 / UNORM
}
fn win_stencil(x: u32, r: u32) -> f32 {
    (1 + (r * WW + x) % 250) as f32
}
fn rb_depth(x: u32, y: u32) -> f32 {
    ((y * WW + x + 1) * 1009 + 500_000) as f32 / UNORM
}
fn rb_stencil(x: u32, y: u32) -> f32 {
    (2 + (y * 7 + x * 3) % 250) as f32
}

/// The sub-rectangle the sub cases invalidate: x, y, width, height.
const SUB: [i32; 4] = [8, 6, 16, 12];

fn inside_sub(x: u32, y: u32) -> bool {
    let [sx, sy, sw, sh] = SUB.map(|v| v as u32);
    (sx..sx + sw).contains(&x) && (sy..sy + sh).contains(&y)
}

/// The attachments a blit of `mask` would have an application invalidate.
fn attachments(mask: GLbitfield) -> Vec<GLenum> {
    match mask {
        DEPTH => vec![GL_DEPTH_ATTACHMENT],
        STENCIL => vec![GL_STENCIL_ATTACHMENT],
        _ => vec![GL_DEPTH_STENCIL_ATTACHMENT],
    }
}

/// What one dEQP-shaped run left: the blit's error and the window's planes
/// (memory order, which is GL order as the window is y-up).
struct Run {
    blit_err: GLenum,
    win_depth: Vec<f32>,
    win_stencil: Vec<u8>,
}

/// The sequence of fbo.invalidate.*.unbind_blit_*: both planes of the window
/// and of the renderbuffer set to their patterns, the FBO's `mask` attachments
/// invalidated (whole, or `SUB` when `sub`; no error), framebuffer 0 bound for
/// drawing, the FBO read, and the whole-surface GL_NEAREST blit of `mask`.
fn run(mask: GLbitfield, sub: bool) -> Run {
    let _g = rig();
    make_window_ds();
    let (fb, rb) = rb_fbo(WW, WH);
    window_y_up(true);
    let (wid, rid) = (window_id(), ds_id(rb));
    write_depth(wid, WW, WH, &plane(WW, WH, win_depth));
    write_stencil(wid, WW, WH, &plane(WW, WH, |x, r| win_stencil(x, r) as u8));
    write_depth(rid, WW, WH, &plane(WW, WH, rb_depth));
    write_stencil(rid, WW, WH, &plane(WW, WH, |x, y| rb_stencil(x, y) as u8));
    let atts = attachments(mask);
    // SAFETY: the slice is live for the call; plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        if sub {
            glInvalidateSubFramebuffer(
                GL_FRAMEBUFFER,
                atts.len() as GLsizei,
                atts.as_ptr(),
                SUB[0],
                SUB[1],
                SUB[2],
                SUB[3],
            );
        } else {
            glInvalidateFramebuffer(GL_FRAMEBUFFER, atts.len() as GLsizei, atts.as_ptr());
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "the invalidation of {atts:x?} (sub {sub}): {:?}", logs());
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, fb);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, 0);
        glBlitFramebuffer(0, 0, WW as i32, WH as i32, 0, 0, WW as i32, WH as i32, mask, GL_NEAREST);
    }
    let blit_err = err();
    let out = Run { blit_err, win_depth: read_depth(wid, WW, WH), win_stencil: read_stencil(wid, WW, WH) };
    release(&[fb], &[rb]);
    out
}

/// The case: no error; a whole invalidation leaves undefined contents,
/// so nothing else is asserted; after a sub one the window's pixels outside
/// `SUB` take the source values of the named planes (and the plane the mask
/// does not name is unchanged).
fn lifted(mask: GLbitfield, sub: bool) {
    let r = run(mask, sub);
    let what = format!("mask {mask:#x}, sub {sub}");
    assert_eq!(r.blit_err, GL_NO_ERROR, "{what}: {:?}", logs());
    if !sub {
        return;
    }
    let depth_want = |x: u32, y: u32| if mask & DEPTH != 0 { rb_depth(x, y) } else { win_depth(x, y) };
    let stencil_want = |x: u32, y: u32| if mask & STENCIL != 0 { rb_stencil(x, y) } else { win_stencil(x, y) };
    for y in 0..WH {
        for x in 0..WW {
            let i = (y * WW + x) as usize;
            if !inside_sub(x, y) || mask & DEPTH == 0 {
                assert!((r.win_depth[i] - depth_want(x, y)).abs() <= 0.5 / UNORM, "{what}: depth at ({x}, {y})");
            }
            if !inside_sub(x, y) || mask & STENCIL == 0 {
                assert_eq!(f32::from(r.win_stencil[i]), stencil_want(x, y), "{what}: stencil at ({x}, {y})");
            }
        }
    }
}

/* ---- the lift: src/gl/blit.rs DepthSource::Window / PlaneDst::Window -------- */

#[test]
fn whole_invalidate_then_depth_blit_is_no_error() {
    lifted(DEPTH, false);
}

#[test]
fn whole_invalidate_then_stencil_blit_is_no_error() {
    lifted(STENCIL, false);
}

#[test]
fn whole_invalidate_then_depth_stencil_blit_is_no_error() {
    lifted(DEPTH | STENCIL, false);
}

#[test]
fn sub_invalidate_then_depth_blit_reaches_the_window_outside_the_rectangle() {
    lifted(DEPTH, true);
}

#[test]
fn sub_invalidate_then_stencil_blit_reaches_the_window_outside_the_rectangle() {
    lifted(STENCIL, true);
}

#[test]
fn sub_invalidate_then_depth_stencil_blit_reaches_the_window_outside_the_rectangle() {
    lifted(DEPTH | STENCIL, true);
}
