// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The R32F storage of a depth texture on the Pi 4's V3D back end: who reads
//! it, who writes it, and what it costs in the carve-out. The fix proposed in
//! docs/notes/v3d-sampled-copies.md ("Ranked fix proposals", 2) and designed
//! in docs/notes/v3d-depth-storage.md, made: the 7,680,000-byte R32F backing
//! store of a depth texture whose depth is only ever sampled from its D24S8
//! buffer is not allocated (`GlTexture::storage_deferred`); it is allocated
//! at the first CPU read (`texture::resolve_depth`) or write, or a draw that
//! samples the texels (`texture::ensure_storage`).
//!
//! The rig is tests/depth_resolve_v3d.rs's: the real GL core against the
//! real V3D back end and the real layout code (driver/src/v3d/v3d_tex.c),
//! the job builder faked, the platform an OSAL whose "GPU" stores a known
//! depth word into the depth buffer of every job that stores one. The
//! platform here also counts the bytes of every live allocation and their
//! high-water, which is the carve-out as the GL core and the back end draw
//! on it (`gpu_alloc`).
//!
//! What is pinned (the test that covers each reader/writer of the storage is
//! named in the note):
//! * rendered, then sampled: the texture unit reads the stored depth through
//!   the DEPTH24_X8 record, and a compare of it against a reference depth
//!   (the shadow look-up) gives the compare result of the stored depth;
//! * given data with glTexImage2D / glTexSubImage2D, then sampled: the
//!   texels are the texture (R32F record, a tiled copy), the values are the
//!   upload's;
//! * rendered, then read on the CPU (glTexSubImage2D, glBlitFramebuffer
//!   depth, the settle of glCopyTexImage2D / glGenerateMipmap): the texels
//!   hold the rendered depth, resolved on first CPU use;
//! * re-specified (glTexImage2D again, glCopyTexImage2D) or given a level
//!   above 0: the gates that ask whether the texture has an image keep
//!   working, and the chain grows around the rendered level 0;
//! * the carve-out of a shadow-like and a refract-like sequence, as numbers:
//!   the R32F storage is the bytes the deferral does not take.
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test v3d_depth_storage`.

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

use yakogl::backend::GLES_TEXFMT_R32F;
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

/// The depth value a texel of word `w` reads as (Mesa's S8_UINT_Z24_UNORM:
/// bits 31..8 over 0xffffff), the conversion of `v3d_depth_resolve`.
fn depth_of(w: u32) -> f32 {
    (w >> 8) as f32 / 16_777_215.0
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

/// glmark2's canvas is 800x600 and its depth target twice that; a smaller
/// pair of the same shape keeps the test quick.
const CANVAS: (i32, i32) = (40, 30);
const DEPTH: (i32, i32) = (80, 60);
/// glmark2 shadow and refract at 800x600: the depth target is twice that.
const FULL: (i32, i32) = (1600, 1200);

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

fn ctx() -> &'static mut GlContext {
    // SAFETY: set once in `rig`, never freed; used under the lock.
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

/// glmark2's depth pass draws no texture (depth.vert / depth.frag write
/// position and a constant); its ground samples unit 0 (shadow.frag's
/// ShadowMap).
const DEPTH_VS: &str = "attribute vec4 a_position;\nvoid main() { gl_Position = a_position; }\n";
const DEPTH_FS: &str = "precision mediump float;\nvoid main() { gl_FragColor = vec4(1.0); }\n";
const GROUND_VS: &str = "attribute vec4 a_position;\nvarying vec2 v_coord;\n\
    void main() { v_coord = a_position.xy * 0.5 + 0.5; gl_Position = a_position; }\n";
const GROUND_FS: &str = "precision mediump float;\nuniform sampler2D ShadowMap;\nvarying vec2 v_coord;\n\
    void main() { gl_FragColor = texture2D(ShadowMap, v_coord); }\n";

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

struct Scene {
    depth_prog: GLuint,
    ground_prog: GLuint,
    tex: GLuint,
    fbo: GLuint,
    w: i32,
    h: i32,
}

/// `DepthRenderTarget::setup` and `GroundRenderer::setup`: the depth
/// texture (GL_DEPTH_COMPONENT / GL_UNSIGNED_INT, nearest, clamped), the
/// framebuffer with it as the only attachment, the two programs.
fn setup(size: (i32, i32)) -> Scene {
    let depth_prog = program(DEPTH_VS, DEPTH_FS);
    let ground_prog = program(GROUND_VS, GROUND_FS);
    let (mut tex, mut fbo) = (0u32, 0u32);
    let name = CString::new("ShadowMap").unwrap();
    // SAFETY: plain GL calls with live out pointers.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MAG_FILTER, GL_NEAREST as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE as GLint);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE as GLint);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
            size.0,
            size.1,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glBindTexture(GL_TEXTURE_2D, 0);
        glGenFramebuffers(1, &mut fbo);
        glBindFramebuffer(GL_FRAMEBUFFER, fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(ground_prog);
        glUniform1i(glGetUniformLocation(ground_prog, name.as_ptr()), 0);
        glEnable(GL_DEPTH_TEST);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    let t = &ctx().objs().textures[tex as usize];
    assert_eq!(t.texfmt, GLES_TEXFMT_R32F, "the GL core keeps a depth texture as R32F texels");
    Scene { depth_prog, ground_prog, tex, fbo, w: size.0, h: size.1 }
}

/// `SceneShadow::draw`: `DepthRenderTarget::enable`, the horse,
/// `disable`, the ground; then the swap (a finish here).
fn frame(s: &Scene) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        glUseProgram(s.depth_prog);
        glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, s.tex, 0);
        glViewport(0, 0, s.w, s.h);
        glColorMask(0, 0, 0, 0);
        glClear(GL_DEPTH_BUFFER_BIT);
        draw_quad();
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glViewport(0, 0, CANVAS.0, CANVAS.1);
        glColorMask(1, 1, 1, 1);
        glUseProgram(s.ground_prog);
        glActiveTexture(GL_TEXTURE0);
        glBindTexture(GL_TEXTURE_2D, s.tex);
        draw_quad();
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/// The ground's draw alone (no render since the last one).
fn ground(s: &Scene) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glUseProgram(s.ground_prog);
        glBindTexture(GL_TEXTURE_2D, s.tex);
        draw_quad();
        glFinish();
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
}

/* ---- what the backend did -------------------------------------------------- */

/// (bindings sampled from a depth buffer, CPU resolves into texels, draws
/// refused) since init.
fn depth_samples() -> (u32, u32, u32) {
    let (mut d, mut r, mut f) = (0u32, 0u32, 0u32);
    // SAFETY: live out-parameters.
    unsafe { v3d_depth_samples(&mut d, &mut r, &mut f) };
    (d, r, f)
}

/// Textures tiled since init (a copy made for the texture unit), and the
/// kept copies live now.
fn tilings_and_copies() -> (u32, u32) {
    let (mut t, mut live) = (0u32, 0u32);
    // SAFETY: live out-parameters.
    unsafe {
        v3d_texture_counters(&mut t, core::ptr::null_mut());
        v3d_tiled_copies(&mut live, core::ptr::null_mut(), core::ptr::null_mut());
    }
    (t, live)
}

/// The last depth pass: the fence of its kick and its clear. The job into
/// the depth texture's framebuffer has no colour target (address 0,
/// `DRAWS_WITHOUT_COLOUR`) and stores its depth; the ground's job on the
/// canvas stores the canvas's own depth, which is not it.
fn last_depth_pass() -> (usize, V3dJobClear) {
    let jobs = jobs();
    let (i, j) = jobs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, j)| j.target.address == 0 && j.clear.is_some_and(|c| c.zs_store && c.zs_address != 0))
        .expect("a job into the depth texture that stored its depth");
    (i + 1, j.clear.unwrap())
}

/// The texture state the last draw of the last job sampled unit 0 with.
fn ground_state() -> V3dTexState {
    let gpu = {
        let jobs = jobs();
        let job = jobs.last().expect("a job");
        let draw = job.tmu.last().expect("a draw in the ground's job");
        assert_eq!(draw.len(), 1, "the ground samples one texture: {draw:?}");
        draw[0].tex_state
    };
    let cpu = with_platform(|p| platform_cpu(p, u64::from(gpu), V3D_TEX_STATE_BYTES)) as usize;
    let states = STATES.lock().unwrap_or_else(|e| e.into_inner());
    states.iter().rev().find(|(at, _)| *at == cpu).map(|(_, s)| *s).expect("the record was packed")
}

/// Texel (x, y) of the depth buffer at `address` as the texture unit reads
/// it through texture state `s`: the layout the state's level-0 fields
/// describe (the real C's, UIF without the XOR as the state says), the word
/// at (x, y), its bits 31..8 over 0xffffff (DEPTH24_X8).
fn tmu_depth(s: &V3dTexState, address: u32, x: u32, y: u32) -> f32 {
    assert_eq!(s.type_, V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8, "the record's type");
    let mut l = zs_layout(s.width, s.height);
    assert!(!s.level_0_xor_enable, "the TLB stored the buffer without the XOR, and so is it read");
    assert_eq!(
        s.level_0_is_strictly_uif,
        l.slices[0].tiling == V3D_TEX_TILING_UIF_NO_XOR,
        "the state's tiling is the buffer's"
    );
    if s.level_0_is_strictly_uif {
        assert_eq!(s.level_0_ub_pad, l.slices[0].ub_pad, "the state's UIF padding is the buffer's");
    }
    l.slices[0].tiling = if s.level_0_is_strictly_uif { V3D_TEX_TILING_UIF_NO_XOR } else { l.slices[0].tiling };
    assert_eq!(s.base, address + l.slices[0].offset, "the record's base is the buffer");
    let mut word = 0u32;
    with_platform(|p| {
        let image = platform_cpu(p, u64::from(address), l.size as usize);
        // SAFETY: the buffer's live mapping; one word out.
        assert!(unsafe { v3d_tex_load_box(image, &l, 0, x, y, 1, 1, &mut word as *mut u32 as *mut u8, 4) });
    });
    depth_of(word)
}

/* ---- the tests ----------------------------------------------------------------- */

/// Texel (x, y) of level 0 of the depth texture of `s`, read from the GL
/// core's R32F storage as it is now (no settle: the caller has made the
/// CPU read it wants).
fn storage_texel(s: &Scene, x: u32, y: u32) -> f32 {
    let t = &ctx().objs().textures[s.tex as usize];
    let pitch = t.level_pitch_at(0) as usize;
    let off = t.level_off[0] as usize + (y as usize * pitch + x as usize) * 4;
    assert!(off + 4 <= t.mem.size, "({x}, {y}) is inside the storage");
    // SAFETY: inside the live allocation (checked above); the kicks completed.
    unsafe { core::ptr::read_unaligned((t.mem.cpu as *const u8).add(off) as *const f32) }
}

/// The bytes of the R32F backing store of the depth texture of `s`.
fn storage_bytes(s: &Scene) -> usize {
    ctx().objs().textures[s.tex as usize].mem.size
}

fn live_bytes() -> usize {
    with_platform(|p| p.live)
}

/// Start a high-water measurement at the bytes live now; the returned value
/// is that base.
fn mark_peak() -> usize {
    with_platform(|p| {
        p.peak = p.live;
        p.live
    })
}

fn peak_bytes() -> usize {
    with_platform(|p| p.peak)
}

fn teardown(s: Scene) {
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(1, &s.fbo);
        glDeleteTextures(1, &s.tex);
        glFinish();
        while glGetError() != GL_NO_ERROR {}
    }
}

/// (a) rendered into, then sampled: the texture unit reads the depth the GPU
/// stored through the DEPTH24_X8 record, and the shadow look-up's compare
/// (glmark2 shadow.frag) comes out as the compare of the stored depth, for a
/// reference below, at and above it. No resolve, no tiled copy: the R32F
/// storage is read by nobody here.
#[test]
fn rendered_then_sampled_compares_like_the_stored_depth() {
    let _g = rig();
    let s = setup(DEPTH);
    let (w, h) = (DEPTH.0 as u32, DEPTH.1 as u32);
    let (tilings0, copies0) = tilings_and_copies();
    let (direct0, resolves0, refused0) = depth_samples();
    frame(&s);
    assert_eq!(depth_samples(), (direct0 + 1, resolves0, refused0), "sampled from the buffer: {:?}", logs());
    assert_eq!(tilings_and_copies(), (tilings0, copies0), "nothing tiled");
    let (fence, pass) = last_depth_pass();
    let st = ground_state();
    assert_eq!((st.type_, st.max_level), (V3D_TEXTURE_DATA_FORMAT_DEPTH24_X8, 0));
    let mut lit_at_equal = 0u32;
    for y in 0..h {
        for x in 0..w {
            let stored = depth_of(depth_word(fence, x, y));
            let seen = tmu_depth(&st, pass.zs_address, x, y);
            assert_eq!(seen, stored, "({x}, {y})");
            for reference in [stored - 1e-4, stored, stored + 1e-4] {
                assert_eq!(seen >= reference, stored >= reference, "({x}, {y}) against {reference}");
            }
            lit_at_equal += u32::from(seen >= stored);
        }
    }
    assert_eq!(lit_at_equal, w * h, "a reference equal to the stored depth is never in shadow");
    assert_eq!(storage_bytes(&s), 0, "rendered and sampled from the buffer only: no R32F storage");
    assert!(ctx().objs().textures[s.tex as usize].storage_deferred);
    teardown(s);
}

/// (b) given data with glTexImage2D (this GL core stores it as floats,
/// `a_depth_upload_is_one_float_per_texel`), then sampled: the texels are
/// the texture. No depth buffer was rendered, so the draw samples the
/// texels through a tiled copy with an R32F record and the values are the
/// upload's.
#[test]
fn uploaded_then_sampled_reads_the_uploaded_texels() {
    let _g = rig();
    let s = setup(DEPTH);
    let (w, h) = (DEPTH.0 as u32, DEPTH.1 as u32);
    let word = |x: u32, y: u32| ((x * 3 + y * 5) << 20) | 0xff;
    let data: Vec<u32> = (0..w * h).map(|i| word(i % w, i / w)).collect();
    let (tilings0, copies0) = tilings_and_copies();
    let (direct0, resolves0, refused0) = depth_samples();
    // SAFETY: plain GL calls; `data` is a w x h GL_UNSIGNED_INT image.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
            DEPTH.0,
            DEPTH.1,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            data.as_ptr() as *const c_void,
        );
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    ground(&s);
    assert_eq!(depth_samples(), (direct0, resolves0, refused0), "no depth was rendered: no buffer, no resolve");
    assert_eq!(tilings_and_copies(), (tilings0 + 1, copies0 + 1), "one tiled copy of the texels");
    let st = ground_state();
    assert_eq!(st.type_, V3D_TEXTURE_DATA_FORMAT_R32F);
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (7, 11), (w - 1, h - 1)] {
        let want = word(x, y) as f32 / 4_294_967_295.0;
        assert!((storage_texel(&s, x, y) - want).abs() < 1e-6, "({x}, {y}): {} vs {want}", storage_texel(&s, x, y));
    }
    /* the copy holds the upload */
    let mut l = V3dTexLayout::default();
    // SAFETY: a live layout to fill.
    assert!(unsafe { v3d_tex_layout(w, h, 1, 4, &mut l) });
    let gpu = st.base - l.slices[0].offset;
    let mut got = [0f32; 1];
    with_platform(|p| {
        let image = platform_cpu(p, u64::from(gpu), l.size as usize);
        // SAFETY: the copy's live mapping; one float out.
        assert!(unsafe { v3d_tex_load_box(image, &l, 0, 7, 11, 1, 1, got.as_mut_ptr() as *mut u8, 4) });
    });
    assert_eq!(got[0], storage_texel(&s, 7, 11), "the tiled copy is the texels");
    teardown(s);
}

/// (b') never rendered, never given data, then sampled: the draw samples
/// the texels (no depth buffer holds the texture's depth), so the deferred
/// store is allocated for that draw (`texture::depth_for_draw`), cleared:
/// the texels read 0.0, as they did when glTexImage2D allocated them.
#[test]
fn never_rendered_then_sampled_allocates_cleared_texels() {
    let _g = rig();
    let s = setup(DEPTH);
    let (w, h) = (DEPTH.0 as u32, DEPTH.1 as u32);
    assert_eq!(storage_bytes(&s), 0, "glTexImage2D(NULL) deferred the storage");
    let (tilings0, copies0) = tilings_and_copies();
    let (direct0, resolves0, refused0) = depth_samples();
    ground(&s);
    assert_eq!(depth_samples(), (direct0, resolves0, refused0), "no depth was rendered: no buffer, no resolve");
    assert_eq!(tilings_and_copies(), (tilings0 + 1, copies0 + 1), "one tiled copy of the texels");
    assert_eq!(ground_state().type_, V3D_TEXTURE_DATA_FORMAT_R32F);
    assert!(storage_bytes(&s) >= (w * h * 4) as usize, "allocated for the draw");
    assert!(!ctx().objs().textures[s.tex as usize].storage_deferred);
    for (x, y) in [(0, 0), (w - 1, 0), (0, h - 1), (7, 11), (w - 1, h - 1)] {
        assert_eq!(storage_texel(&s, x, y), 0.0, "({x}, {y}): cleared");
    }
    teardown(s);
}

/// (c) rendered, then read on the CPU through the paths that make
/// `resolve_depth` run and that the GL core accepts for a depth texture:
/// glTexSubImage2D (`settle_texture`, keeps the rendered rest) and
/// glBlitFramebuffer with the texture as the read or the draw side
/// (`blit::copy_depth`). The storage then holds the depth the GPU stored.
#[test]
fn rendered_then_read_on_the_cpu_resolves_into_the_storage() {
    let _g = rig();
    let s = setup(DEPTH);
    let (w, h) = (DEPTH.0 as u32, DEPTH.1 as u32);

    /* glTexSubImage2D into a rendered texture */
    frame(&s);
    let (fence, _) = last_depth_pass();
    let (d0, r0, f0) = depth_samples();
    let one = [0xffff_ffffu32];
    // SAFETY: plain GL calls; `one` is a 1x1 GL_UNSIGNED_INT depth image.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
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
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
    }
    assert_eq!(depth_samples(), (d0, r0 + 1, f0), "the write resolved the texels first");
    assert_eq!(storage_texel(&s, 0, 0), 1.0, "the written texel");
    for (x, y) in [(1, 0), (w - 1, h - 1), (9, 4)] {
        assert_eq!(storage_texel(&s, x, y), depth_of(depth_word(fence, x, y)), "({x}, {y}): the rendered rest is kept");
    }

    /* glBlitFramebuffer: this texture is the read side of a depth copy */
    frame(&s);
    let (fence, _) = last_depth_pass();
    let dst = setup(DEPTH);
    let (_, r1, _) = depth_samples();
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, s.fbo);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fbo);
        glBlitFramebuffer(0, 0, DEPTH.0, DEPTH.1, 0, 0, DEPTH.0, DEPTH.1, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    assert!(depth_samples().1 > r1, "the blit resolved its source: {:?}", logs());
    for (x, y) in [(0, 0), (w - 1, h - 1), (9, 4)] {
        let want = depth_of(depth_word(fence, x, y));
        assert_eq!(storage_texel(&s, x, y), want, "({x}, {y}): the source texels");
        assert_eq!(storage_texel(&dst, x, y), want, "({x}, {y}): the blit copied the rendered depth");
    }

    /* this texture is the draw side of a blit that covers a corner only:
     * the rest it holds is what the GPU rendered into it */
    frame(&dst);
    let (fence, _) = last_depth_pass();
    let (_, r2, _) = depth_samples();
    // SAFETY: plain GL calls on live names.
    unsafe {
        glBindFramebuffer(GL_READ_FRAMEBUFFER, s.fbo);
        glBindFramebuffer(GL_DRAW_FRAMEBUFFER, dst.fbo);
        glBlitFramebuffer(0, 0, 4, 4, 0, 0, 4, 4, GL_DEPTH_BUFFER_BIT, GL_NEAREST);
        assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    assert!(depth_samples().1 > r2, "the blit resolved its destination too");
    assert_eq!(
        storage_texel(&dst, w - 1, h - 1),
        depth_of(depth_word(fence, w - 1, h - 1)),
        "outside the blit: rendered"
    );
    teardown(dst);
    teardown(s);
}

/// The remaining CPU paths into a depth texture. glGenerateMipmap on it is
/// GL_INVALID_OPERATION before anything is settled. glCopyTexSubImage2D into
/// it is accepted (the canvas is the read buffer): it settles the texture, so
/// the rendered depth is resolved first and the rest of the texels keeps it.
#[test]
fn generate_mipmap_is_refused_and_copy_tex_sub_image_settles_a_depth_texture() {
    let _g = rig();
    let s = setup(DEPTH);
    let (w, h) = (DEPTH.0 as u32, DEPTH.1 as u32);
    frame(&s);
    let (fence, _) = last_depth_pass();
    let (d0, r0, f0) = depth_samples();
    // SAFETY: plain GL calls on live names.
    let mip = unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glGenerateMipmap(GL_TEXTURE_2D);
        glGetError()
    };
    assert_eq!(mip, GL_INVALID_OPERATION, "{:?}", logs());
    assert_eq!(depth_samples(), (d0, r0, f0), "refused before any settle: nothing resolved");
    // SAFETY: plain GL calls on live names.
    let copy = unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glCopyTexSubImage2D(GL_TEXTURE_2D, 0, 0, 0, 0, 0, 4, 4);
        glGetError()
    };
    assert_eq!(copy, GL_NO_ERROR, "{:?}", logs());
    assert_eq!(depth_samples(), (d0, r0 + 1, f0), "the copy settled the texture: one resolve");
    assert_eq!(
        storage_texel(&s, w - 1, h - 1),
        depth_of(depth_word(fence, w - 1, h - 1)),
        "outside the copy: rendered"
    );
    teardown(s);
}

/// The two re-specifications of a depth texture that look at the allocation
/// before they decide (`tex_image_2d`'s keep / re-lay-out tests, the early
/// return of `glCopyTexImage2D`). glCopyTexImage2D with a depth internal
/// format is an error before any of that; with a colour one it lays the
/// texture out as colour, with storage of its own. glTexImage2D(NULL) of the
/// same size and format again keeps the texture a working depth target: the
/// next frame renders into it and samples it from the buffer.
#[test]
fn respecifying_a_depth_texture_keeps_it_a_working_target_or_makes_it_colour() {
    let _g = rig();
    let s = setup(DEPTH);
    frame(&s);
    let (d0, r0, f0) = depth_samples();
    // SAFETY: plain GL calls on live names.
    let (again, status) = unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            0,
            (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
            DEPTH.0,
            DEPTH.1,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glBindFramebuffer(GL_FRAMEBUFFER, s.fbo);
        let status = glCheckFramebufferStatus(GL_FRAMEBUFFER);
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        (glGetError(), status)
    };
    assert_eq!((again, status), (GL_NO_ERROR, GL_FRAMEBUFFER_COMPLETE), "{:?}", logs());
    frame(&s);
    let (d1, r1, f1) = depth_samples();
    assert_eq!(d1, d0 + 1, "the next frame samples the new depth from the buffer: {:?}", logs());
    assert_eq!((r1, f1), (r0, f0), "no resolve and no refusal");
    // SAFETY: plain GL calls on live names.
    let refused = unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_COMPONENT, 0, 0, 4, 4, 0);
        glGetError()
    };
    assert!(matches!(refused, GL_INVALID_ENUM | GL_INVALID_OPERATION), "a depth copy is an error: {refused:#x}");
    assert_eq!(depth_samples(), (d1, r1, f1), "refused before any settle: nothing resolved");
    // SAFETY: plain GL calls on live names.
    let colour = unsafe {
        glCopyTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA, 0, 0, 4, 4, 0);
        glGetError()
    };
    assert_eq!(colour, GL_NO_ERROR, "{:?}", logs());
    let t = &ctx().objs().textures[s.tex as usize];
    assert_eq!((t.texfmt, t.width, t.height), (yakogl::backend::GLES_TEXFMT_RGBA8, 4, 4), "now a colour texture");
    assert!(!t.mem.impl_.is_null(), "with storage of its own");
    teardown(s);
}

/// A level above 0 given to a non-power-of-two depth texture (the V3D keeps
/// a chain for it, `grow_npot_chain` from `tex_image_2d`) lays the storage
/// out again with room for the chain, keeping level 0 and settling the
/// rendered depth into it. The texture then has two levels, so the draw
/// resolves it instead of sampling the buffer. A deferral has to allocate
/// before this lay-out, not after.
#[test]
fn a_level_above_zero_grows_the_chain_and_resolves_the_rendered_depth() {
    let _g = rig();
    let s = setup(FULL);
    frame(&s);
    let (d0, r0, f0) = depth_samples();
    /* sampled from the buffer only: the storage is still deferred */
    assert_eq!(storage_bytes(&s), 0, "deferred until the level above 0 needs level 0's texels");
    // SAFETY: plain GL calls on live names.
    let err = unsafe {
        glBindTexture(GL_TEXTURE_2D, s.tex);
        glTexImage2D(
            GL_TEXTURE_2D,
            1,
            (if cfg!(feature = "report-es2") { GL_DEPTH_COMPONENT } else { GL_DEPTH_COMPONENT24 }) as GLint,
            FULL.0 / 2,
            FULL.1 / 2,
            0,
            GL_DEPTH_COMPONENT,
            GL_UNSIGNED_INT,
            core::ptr::null(),
        );
        glGetError()
    };
    assert_eq!(err, GL_NO_ERROR, "{:?}", logs());
    let t = &ctx().objs().textures[s.tex as usize];
    assert!(t.chain_levels > 1 && t.levels == 2, "chain {} levels {}", t.chain_levels, t.levels);
    assert!(storage_bytes(&s) > STORAGE, "allocated, with room for the chain");
    let (fence, _) = last_depth_pass();
    assert_eq!(
        storage_texel(&s, FULL.0 as u32 - 1, FULL.1 as u32 - 1),
        depth_of(depth_word(fence, FULL.0 as u32 - 1, FULL.1 as u32 - 1)),
        "level 0 kept the rendered depth through the re-lay-out"
    );
    frame(&s);
    let (d1, r1, f1) = depth_samples();
    assert_eq!((d1, f1), (d0, f0), "two levels: not sampled from the buffer, not refused");
    assert!(r1 > r0, "resolved into the texels instead");
    teardown(s);
}

/// Allocations the back end and the GL core keep for good (job arenas, the
/// clear program, the canvas): made once before the first measurement, so a
/// high-water is the same whichever test of the file runs first.
fn warm_up() {
    static WARM: Once = Once::new();
    WARM.call_once(|| {
        let s = setup(FULL);
        frame(&s);
        teardown(s);
    });
}

/// The carve-out use of the depth texture's R32F storage, as numbers.
/// `STORAGE` is what the deferral does not allocate for a sampled-only depth
/// texture (1600 x 1200 x 4; the board's 7,684,096 is the D24S8 buffer's
/// padded UIF size). The deferral lowered each high-water below by exactly
/// `STORAGE` (15,390,016 and 23,518,364 before it): a change that moves
/// them by anything else has made a reader allocate, or missed one.
const STORAGE: usize = 7_680_000;
const SHADOW_PEAK: usize = 15_390_016 - STORAGE;
const REFRACT_PEAK: usize = 23_518_364 - STORAGE;

/// glmark2 shadow's sequence (depth pass into a 1600x1200 depth texture, the
/// ground samples it, three frames): the bytes allocated since the start, at
/// their high-water.
#[test]
fn shadow_like_carve_out_high_water() {
    let _g = rig();
    warm_up();
    let base = mark_peak();
    let s = setup(FULL);
    let after_tex = live_bytes() - base;
    assert_eq!(storage_bytes(&s), 0, "the R32F backing store of a 1600x1200 depth texture is deferred");
    assert!(after_tex < STORAGE, "glTexImage2D did not allocate it: {after_tex}");
    for _ in 0..3 {
        frame(&s);
    }
    assert_eq!(storage_bytes(&s), 0, "three frames rendered and sampled it: still not allocated");
    let peak = peak_bytes() - base;
    let live = live_bytes() - base;
    println!("shadow-like: R32F {STORAGE}, after glTexImage2D {after_tex}, high-water {peak}, live {live}");
    assert_eq!(peak, SHADOW_PEAK, "shadow-like high-water");
    teardown(s);
}

/// glmark2 refract's sequence: the depth texture as above, and a colour
/// texture rendered into, mip-mapped and sampled (800x600 RGBA8), every
/// frame. The high-water counts both.
#[test]
fn refract_like_carve_out_high_water() {
    let _g = rig();
    warm_up();
    let base = mark_peak();
    let s = setup(FULL);
    let (mut ctex, mut cfbo) = (0u32, 0u32);
    // SAFETY: plain GL calls with live out pointers.
    unsafe {
        glGenTextures(1, &mut ctex);
        glBindTexture(GL_TEXTURE_2D, ctex);
        glTexParameteri(GL_TEXTURE_2D, GL_TEXTURE_MIN_FILTER, GL_LINEAR_MIPMAP_LINEAR as GLint);
        glTexImage2D(GL_TEXTURE_2D, 0, GL_RGBA as GLint, 800, 600, 0, GL_RGBA, GL_UNSIGNED_BYTE, core::ptr::null());
        glGenFramebuffers(1, &mut cfbo);
        glBindFramebuffer(GL_FRAMEBUFFER, cfbo);
        glFramebufferTexture2D(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_TEXTURE_2D, ctex, 0);
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
    }
    for _ in 0..3 {
        frame(&s);
        // SAFETY: plain GL calls on live names.
        unsafe {
            glBindFramebuffer(GL_FRAMEBUFFER, cfbo);
            glViewport(0, 0, 800, 600);
            glUseProgram(s.depth_prog);
            draw_quad();
            glBindFramebuffer(GL_FRAMEBUFFER, 0);
            glViewport(0, 0, CANVAS.0, CANVAS.1);
            glUseProgram(s.ground_prog);
            glBindTexture(GL_TEXTURE_2D, ctex);
            glGenerateMipmap(GL_TEXTURE_2D);
            draw_quad();
            glFinish();
            assert_eq!(glGetError(), GL_NO_ERROR, "{:?}", logs());
        }
    }
    assert_eq!(storage_bytes(&s), 0, "the depth texture's storage is still deferred");
    let peak = peak_bytes() - base;
    let live = live_bytes() - base;
    println!("refract-like: R32F {STORAGE}, high-water {peak}, live {live}");
    assert_eq!(peak, REFRACT_PEAK, "refract-like high-water");
    // SAFETY: plain GL calls on live names.
    unsafe {
        glDeleteFramebuffers(1, &cfbo);
        glDeleteTextures(1, &ctex);
    }
    teardown(s);
}
