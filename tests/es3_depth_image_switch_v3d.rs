// SPDX-License-Identifier: Apache-2.0
// SPDX-FileCopyrightText: 2026 Signal Slot, Inc.
//! The depth plane of a cube or array depth texture when draws switch faces
//! or layers, over the real V3D back end (plan item switch-test-v3d;
//! docs/notes/blit-depth-layers.md "What this means today" 1 and 2, design
//! (b)). tests/es3_depth_image_switch.rs is the same on a stand-in back end;
//! this file is the V3D's: the real GL core and the real `v3d_depth_resolve`
//! / `v3d_ds_write` (src/v3d/backend.rs, src/v3d/ds_plane.rs), the job
//! builder faked (the rig of tests/es3_blit_ds_layers_v3d.rs, copied).
//!
//! Layout on the V3D: ONE depth buffer PER TEXTURE, keyed by the ds object id
//! (`ds_object_id(serial, true)`) with no face, layer or level. Today the GL
//! core writes that plane into face/layer 0 of the level (`resolve_depth`)
//! and seeds only from face/layer 0 (`depth_seed`). Design (b) records the
//! (level, layer) the plane holds: a draw into another face first resolves
//! the old face into its own rows, then seeds the new face from its rows.
//!
//! Two kinds of test:
//! - plain tests pin what must not change: a 2D depth texture resolves into
//!   its rows and seeds from them, a whole-depth clear skips the seed, and
//!   face/layer 0 of a cube or array resolves into its own rows and leaves
//!   the other images alone;
//! - `#[ignore = "needs gl-ds-layers-core"]` twins hold design (b): draw
//!   face 0 then face 1 (array layers 0 then 2); the old face's rows get the
//!   plane the back end held, the new face is seeded from its own rows (a
//!   whole-plane clear on the switch draw hands no seed and leaves the
//!   cleared value), and a later read puts the plane into the new face
//!   leaving the old one. Run them with `-- --ignored`; they fail today.
//!
//! The planes are not what the fake GPU stores: after each draw has been
//! finished, the plane is set through the back end's own `ds_write` (by id)
//! to a distinct value per pixel, and read back through `depth_resolve`.
//! Every draw is a colour clear (and, where a test says so, a depth clear)
//! of an FBO that also has a colour renderbuffer, so no shader is needed.
//! The face/layer texels are written and read in the GL core's texel store
//! (faces stacked in the level, face k's row y at row k * height + y). A CPU
//! read of a layered texture is a one-texel glTexSubImage of an image the
//! plane does not hold, with the value already there: it settles (resolves)
//! the texture and changes nothing else. Compared in 24-bit depth units (the
//! buffer is D24S8; all values are multiples of 2^-16 so they round-trip).
//!
//! Run: `cargo test --no-default-features --features
//! host-test,gpu-v3d,shaderc-online,no-shader-table --test
//! es3_depth_image_switch_v3d`, and the ignored twins with `-- --ignored`.

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
use yakogl::gl::texture::{
    glBindTexture, glDeleteTextures, glGenTextures, glTexImage2D, glTexImage3D, glTexSubImage2D, glTexSubImage3D,
};
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
        let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("es3_depth_image_switch_v3d");
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

/* ---- planes and objects ------------------------------------------------------- */

const D: u32 = yakogl::backend::DS_PLANE_DEPTH;
/// 24-bit depth: the unit the buffer holds.
const UNORM: f32 = 16_777_215.0;

fn err() -> GLenum {
    // SAFETY: plain GL call.
    unsafe { glGetError() }
}

/// The id the back end keeps texture `tex`'s depth and stencil under.
fn tex_ds_id(tex: GLuint) -> u32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    context::ds_object_id(c.texture_object(slot).expect("a texture object").serial, true)
}

/// Write a whole plane through the table's `ds_write` of object `id`.
fn write_depth(id: u32, w: u32, h: u32, v: &[f32]) {
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { v3d_ds_write(id, D, v.as_ptr() as *const c_void, 0, 0, w, h, w) };
    assert_eq!(r, 0, "ds_write of the depth of 0x{id:x}: {:?}", logs());
}
/// The planes as the back end resolves them (row 0 first).
fn read_depth(id: u32, w: u32, h: u32) -> Vec<f32> {
    let mut v = vec![-1.0f32; (w * h) as usize];
    // SAFETY: `h` rows of `w` floats.
    let r = unsafe { v3d_depth_resolve(id, v.as_mut_ptr(), w, h, w) };
    assert_eq!(r, 0, "depth_resolve of 0x{id:x}: {:?}", logs());
    v
}
/// A `w` x `h` plane (row-major) of `f(x, y)`.
fn plane<T>(w: u32, h: u32, f: impl Fn(u32, u32) -> T) -> Vec<T> {
    (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect()
}

fn assert_close(got: &[f32], want: &[f32], what: &str) {
    assert_eq!(got.len(), want.len(), "{what}");
    for (i, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= 0.5 / UNORM, "{what}: pixel {i}: {g} want {w}");
    }
}

/* ---- objects ------------------------------------------------------------ */

const SIDE: u32 = 8;
const D32F: (GLenum, GLenum, GLenum) = (GL_DEPTH_COMPONENT32F, GL_DEPTH_COMPONENT, GL_FLOAT);
const FACES: [GLenum; 6] = [
    GL_TEXTURE_CUBE_MAP_POSITIVE_X,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_X,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Y,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Y,
    GL_TEXTURE_CUBE_MAP_POSITIVE_Z,
    GL_TEXTURE_CUBE_MAP_NEGATIVE_Z,
];

#[derive(Clone, Copy, Debug)]
enum Layered {
    Cube,
    Array,
}

impl Layered {
    fn images(self) -> u32 {
        match self {
            Layered::Cube => 6,
            Layered::Array => 3,
        }
    }
}

/// What image `k` of a layered texture holds at (x, y) as the test made it:
/// a ramp per image from 0.5 up, exact in f32 and in 24 bits.
fn image_depth(k: u32, x: u32, y: u32) -> f32 {
    (k * 256 + y * 16 + x) as f32 / 65536.0 + 0.5
}

/// What the test puts into the plane (`ds_write`) as its `n`th plane: a
/// value per pixel, distinct from every `image_depth` (below 0.25, the clear
/// value) and exact in 24 bits.
fn plane_value(n: u32, x: u32, y: u32) -> f32 {
    (n * 512 + y * 16 + x) as f32 / 65536.0 + 0.0625
}

const CLEAR: f32 = 0.25;

fn texel_ptr(tex: GLuint, layer: u32, x: u32, y: u32) -> *mut f32 {
    let c = context::current().expect("a current context");
    let slot = names::slot_of(c, Kind::Texture, tex).expect("a texture name");
    let t = c.texture_object(slot).expect("a texture object");
    assert!(x < t.width && y < t.height && layer < t.layers.max(1) as u32);
    let row = layer as usize * t.height as usize + y as usize;
    let off = t.level_off[0] as usize + (row * t.level_pitch_at(0) as usize + x as usize) * 4;
    assert!(off + 4 <= t.mem.size);
    (t.mem.cpu as *mut u8).wrapping_add(off) as *mut f32
}

/// The R32F texel (x, y) of image `layer` of level 0, as the GL core stores
/// it (src/gl/context.rs `layers`).
fn texel(tex: GLuint, layer: u32, x: u32, y: u32) -> f32 {
    // SAFETY: inside the texture's host allocation (texel_ptr checks).
    unsafe { texel_ptr(tex, layer, x, y).read() }
}

/// Image `layer` of `tex` as the CPU holds it.
fn image(tex: GLuint, layer: u32) -> Vec<f32> {
    plane(SIDE, SIDE, |x, y| texel(tex, layer, x, y))
}

/// What image `k`'s rows hold as the test made them.
fn seeded_image(k: u32) -> Vec<f32> {
    plane(SIDE, SIDE, |x, y| image_depth(k, x, y))
}

/// The `n`th plane the test wrote.
fn written(n: u32) -> Vec<f32> {
    plane(SIDE, SIDE, |x, y| plane_value(n, x, y))
}

/// Set the plane of texture `tex` to the `n`th written plane (ds_write by id).
fn set_plane(tex: GLuint, n: u32) {
    write_depth(tex_ds_id(tex), SIDE, SIDE, &written(n));
}

/// The plane of texture `tex` as the back end resolves it.
fn plane_of(tex: GLuint) -> Vec<f32> {
    read_depth(tex_ds_id(tex), SIDE, SIDE)
}

/// A cube map or 3-layer array of 8x8 D32F, image k holding `image_depth(k)`.
fn layered_texture(kind: Layered) -> GLuint {
    let mut tex = 0;
    let s = SIDE as i32;
    // SAFETY: room for one name; no pixels (the texels are written below).
    unsafe {
        glGenTextures(1, &mut tex);
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                for face in FACES {
                    glTexImage2D(face, 0, D32F.0 as GLint, s, s, 0, D32F.1, D32F.2, core::ptr::null());
                }
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                let n = kind.images() as i32;
                glTexImage3D(GL_TEXTURE_2D_ARRAY, 0, D32F.0 as GLint, s, s, n, 0, D32F.1, D32F.2, core::ptr::null());
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "a {kind:?} depth texture: {:?}", logs());
    for k in 0..kind.images() {
        for y in 0..SIDE {
            for x in 0..SIDE {
                // SAFETY: as in `texel`.
                unsafe { texel_ptr(tex, k, x, y).write(image_depth(k, x, y)) };
            }
        }
    }
    tex
}

/// An 8x8 2D depth texture holding `image_depth(0)`.
fn depth_texture_2d() -> GLuint {
    let mut tex = 0;
    let s = SIDE as i32;
    let px = seeded_image(0);
    // SAFETY: room for one name; `px` is 8 rows of 8 floats.
    unsafe {
        glGenTextures(1, &mut tex);
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexImage2D(GL_TEXTURE_2D, 0, D32F.0 as GLint, s, s, 0, D32F.1, D32F.2, px.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
    tex
}

/// A framebuffer with image `k` of `tex` at the depth attachment (the 2D
/// texture itself for `None`) and an 8x8 colour renderbuffer; left bound.
/// (framebuffer, renderbuffer).
fn target(tex: GLuint, kind: Option<Layered>, k: u32) -> (GLuint, GLuint) {
    let (mut fb, mut rb) = (0, 0);
    // SAFETY: room for one name each; plain GL calls.
    unsafe {
        glGenFramebuffers(1, &mut fb);
        glBindFramebuffer(GL_FRAMEBUFFER, fb);
        match kind {
            None => glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, GL_TEXTURE_2D, tex, 0),
            Some(Layered::Cube) => {
                glFramebufferTexture2D(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, FACES[k as usize], tex, 0)
            }
            Some(Layered::Array) => glFramebufferTextureLayer(GL_FRAMEBUFFER, GL_DEPTH_ATTACHMENT, tex, 0, k as GLint),
        }
        glGenRenderbuffers(1, &mut rb);
        glBindRenderbuffer(GL_RENDERBUFFER, rb);
        glRenderbufferStorage(GL_RENDERBUFFER, GL_RGBA8, SIDE as i32, SIDE as i32);
        glFramebufferRenderbuffer(GL_FRAMEBUFFER, GL_COLOR_ATTACHMENT0, GL_RENDERBUFFER, rb);
        assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
        assert_eq!(glCheckFramebufferStatus(GL_FRAMEBUFFER), GL_FRAMEBUFFER_COMPLETE, "{:?}", logs());
    }
    (fb, rb)
}

/// A colour clear of the bound framebuffer (`depth_too`: and a clear of the
/// whole depth plane to `CLEAR`), finished, so the job has run and the plane
/// can be written.
fn draw(depth_too: bool) {
    // SAFETY: plain GL calls.
    unsafe {
        glClearDepthf(CLEAR);
        glClear(GL_COLOR_BUFFER_BIT | if depth_too { GL_DEPTH_BUFFER_BIT } else { 0 });
        glFinish();
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

/// A CPU read of layered texture `tex`: a one-texel glTexSubImage of image
/// `last` with the value it holds, which settles the texture first
/// (`settle_texture`: `resolve_depth`, then the sub-image write). `last` must
/// not be the image the plane holds, or the write would put the stale value
/// back over the resolve.
fn settle_at(tex: GLuint, kind: Layered, last: u32) {
    let v = [texel(tex, last, 0, 0)];
    // SAFETY: one float of pixels, a 1x1 sub-image inside the level.
    unsafe {
        match kind {
            Layered::Cube => {
                glBindTexture(GL_TEXTURE_CUBE_MAP, tex);
                glTexSubImage2D(FACES[last as usize], 0, 0, 0, 1, 1, D32F.1, D32F.2, v.as_ptr() as *const c_void);
            }
            Layered::Array => {
                glBindTexture(GL_TEXTURE_2D_ARRAY, tex);
                let z = last as GLint;
                glTexSubImage3D(GL_TEXTURE_2D_ARRAY, 0, 0, 0, z, 1, 1, 1, D32F.1, D32F.2, v.as_ptr() as *const c_void);
            }
        }
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

fn release(fbs: &[GLuint], texs: &[GLuint], rbs: &[GLuint]) {
    // SAFETY: names the test made.
    unsafe {
        glBindFramebuffer(GL_FRAMEBUFFER, 0);
        glDeleteFramebuffers(fbs.len() as i32, fbs.as_ptr());
        glDeleteTextures(texs.len() as i32, texs.as_ptr());
        glDeleteRenderbuffers(rbs.len() as i32, rbs.as_ptr());
    }
}

/* ---- what must not change: 2D, image 0, whole-depth clears --------------- */

/// A one-texel sub-image of the bound 2D depth texture at (1, 1).
const MARK: f32 = 0.3125;

fn mark_2d(tex: GLuint) {
    let one = [MARK];
    // SAFETY: one float, a 1x1 sub-image inside the level.
    unsafe {
        glBindTexture(GL_TEXTURE_2D, tex);
        glTexSubImage2D(GL_TEXTURE_2D, 0, 1, 1, 1, 1, D32F.1, D32F.2, one.as_ptr() as *const c_void);
    }
    assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
}

/// A 2D depth texture drawn into, its plane set, then written by the CPU:
/// the upload settled the texture first, so the rows hold the plane the back
/// end had (all of them) and then the texel the CPU wrote.
#[test]
fn a_2d_depth_texture_resolves_into_its_rows() {
    let _g = rig();
    let tex = depth_texture_2d();
    let (fb, rb) = target(tex, None, 0);
    draw(true);
    set_plane(tex, 1);
    mark_2d(tex);
    let mut want = written(1);
    want[(SIDE + 1) as usize] = MARK;
    assert_close(&image(tex, 0), &want, "the texture's rows");
    release(&[fb], &[tex], &[rb]);
}

/// Texels the CPU wrote reach the back end with the first draw after them,
/// the whole level (the plane holds them); the draw after that seeds nothing.
#[test]
fn a_2d_depth_texture_seeds_from_its_rows() {
    let _g = rig();
    let tex = depth_texture_2d();
    let (fb, rb) = target(tex, None, 0);
    draw(true);
    set_plane(tex, 1);
    mark_2d(tex);
    let mut rows = written(1);
    rows[(SIDE + 1) as usize] = MARK;
    // The plane moves away from the rows; the draw puts them back.
    set_plane(tex, 2);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, fb) };
    draw(false);
    assert_close(&plane_of(tex), &rows, "the first draw after the write seeds the rows");
    set_plane(tex, 3);
    draw(false);
    assert_close(&plane_of(tex), &written(3), "the draw after that seeds nothing");
    release(&[fb], &[tex], &[rb]);
}

/// A whole-depth clear replaces what the rows held: the clear draw takes no
/// seed (the plane is the clear value), and the draw after it none either.
#[test]
fn a_whole_depth_clear_skips_the_seed() {
    let _g = rig();
    let tex = depth_texture_2d();
    let (fb, rb) = target(tex, None, 0);
    draw(true);
    set_plane(tex, 1);
    mark_2d(tex);
    draw(true);
    assert_close(&plane_of(tex), &plane(SIDE, SIDE, |_, _| CLEAR), "the cleared plane");
    set_plane(tex, 2);
    draw(false);
    assert_close(&plane_of(tex), &written(2), "no seed after the clear");
    release(&[fb], &[tex], &[rb]);
}

/// Image 0 of a cube or array is the one image today's plane lands in: a draw
/// into it, then a CPU read, writes the plane into image 0's rows and leaves
/// every other image's rows alone.
#[test]
fn image_0_of_a_layered_texture_resolves_into_its_own_rows() {
    for kind in [Layered::Cube, Layered::Array] {
        let _g = rig();
        let tex = layered_texture(kind);
        let (fb, rb) = target(tex, Some(kind), 0);
        draw(true);
        set_plane(tex, 1);
        settle_at(tex, kind, kind.images() - 1);
        assert_close(&image(tex, 0), &written(1), &format!("{kind:?}: image 0"));
        for k in 1..kind.images() {
            assert_eq!(image(tex, k), seeded_image(k), "{kind:?}: image {k} untouched");
        }
        release(&[fb], &[tex], &[rb]);
    }
}

/// Image 0 of a layered texture, as in 2D: the CPU writes its rows, then a
/// whole-depth clear takes no seed and the draw after it none either.
#[test]
fn a_whole_depth_clear_of_image_0_skips_the_seed() {
    for kind in [Layered::Cube, Layered::Array] {
        let _g = rig();
        let tex = layered_texture(kind);
        let (fb, rb) = target(tex, Some(kind), 0);
        draw(true);
        set_plane(tex, 1);
        settle_at(tex, kind, kind.images() - 1);
        // The CPU writes image 0 (a sub-image with the value it holds).
        let v = [texel(tex, 0, 0, 0)];
        // SAFETY: one float, a 1x1 sub-image inside the level.
        unsafe {
            match kind {
                Layered::Cube => glTexSubImage2D(FACES[0], 0, 0, 0, 1, 1, D32F.1, D32F.2, v.as_ptr() as *const c_void),
                Layered::Array => glTexSubImage3D(
                    GL_TEXTURE_2D_ARRAY,
                    0,
                    0,
                    0,
                    0,
                    1,
                    1,
                    1,
                    D32F.1,
                    D32F.2,
                    v.as_ptr() as *const c_void,
                ),
            }
        }
        assert_eq!(err(), GL_NO_ERROR, "{:?}", logs());
        draw(true);
        assert_close(&plane_of(tex), &plane(SIDE, SIDE, |_, _| CLEAR), &format!("{kind:?}: the cleared plane"));
        set_plane(tex, 2);
        draw(false);
        assert_close(&plane_of(tex), &written(2), &format!("{kind:?}: no seed after the clear"));
        release(&[fb], &[tex], &[rb]);
    }
}

/* ---- the design: the plane follows the image drawn ----------------------- */

/// Draw image `a` (its plane set to `written(1)`), then image `b` (`clear`
/// makes the switch draw clear the whole depth plane), then read the
/// texture, and check design (b):
/// - the switch to `b` resolves the plane into `a`'s rows before the draw
///   (the back end held `written(1)` and nothing else wrote it);
/// - the draw into `b` is seeded from `b`'s rows: the plane holds them (the
///   clear value, if the draw clears the whole plane: no seed);
/// - a later read (the plane set to `written(2)`) resolves into `b`'s rows
///   and leaves `a`'s as the first resolve wrote them, and the other images'
///   rows stay as they were.
fn switch_case(kind: Layered, a: u32, b: u32, clear: bool) {
    let _g = rig();
    let tex = layered_texture(kind);
    let what = format!("{kind:?} {a} -> {b} (clear {clear})");
    let (fa, ra) = target(tex, Some(kind), a);
    draw(true);
    set_plane(tex, 1);
    let (fb, rb) = target(tex, Some(kind), b);
    draw(clear);
    assert_close(&image(tex, a), &written(1), &format!("{what}: image {a} took the plane before the switch draw"));
    assert_eq!(image(tex, b), seeded_image(b), "{what}: image {b} not yet read");
    let want = if clear { plane(SIDE, SIDE, |_, _| CLEAR) } else { seeded_image(b) };
    assert_close(&plane_of(tex), &want, &format!("{what}: the plane after the switch draw"));
    // Read through an image outside {a, b}: a write into b would put the
    // stale seed back over the plane the read resolved into b.
    set_plane(tex, 2);
    let other = (0..kind.images()).rev().find(|k| *k != a && *k != b).unwrap();
    settle_at(tex, kind, other);
    assert_close(&image(tex, a), &written(1), &format!("{what}: image {a} keeps its depth"));
    assert_close(&image(tex, b), &written(2), &format!("{what}: image {b} took the plane"));
    for k in (0..kind.images()).filter(|k| *k != a && *k != b) {
        assert_eq!(image(tex, k), seeded_image(k), "{what}: image {k} untouched");
    }
    release(&[fa, fb], &[tex], &[ra, rb]);
}

/// Cube face +X drawn, then -X: +X's rows get the back end's depth, -X's
/// seed comes from -X's rows, and later each face reads its own depth.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn drawing_face_1_after_face_0_resolves_face_0_and_seeds_face_1() {
    switch_case(Layered::Cube, 0, 1, false);
}

/// The same for array layers 0 and 2 (layer 1 between them is not touched).
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn drawing_layer_2_after_layer_0_resolves_layer_0_and_seeds_layer_2() {
    switch_case(Layered::Array, 0, 2, false);
}

/// A switch draw that clears the whole plane resolves the old image but
/// hands no seed (the new image's rows are overwritten anyway).
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn a_whole_plane_clear_on_the_switch_draw_hands_no_seed() {
    switch_case(Layered::Cube, 0, 1, true);
    switch_case(Layered::Array, 0, 2, true);
}

/// Switching between faces other than 0 (+Y, then -Z): the plane goes to the
/// face drawn, never to face 0.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn a_switch_between_faces_other_than_0_leaves_face_0_alone() {
    switch_case(Layered::Cube, 2, 5, false);
}

/// Switching back: face 0, face 1, face 0 again. The second switch resolves
/// the plane into face 1's rows and seeds face 0 from face 0's rows, which
/// hold the first resolve.
#[test]
#[ignore = "needs gl-ds-layers-core"]
fn switching_back_seeds_from_the_rows_the_first_resolve_wrote() {
    let kind = Layered::Cube;
    let _g = rig();
    let tex = layered_texture(kind);
    let (f0, r0) = target(tex, Some(kind), 0);
    draw(true);
    set_plane(tex, 1);
    let (f1, r1) = target(tex, Some(kind), 1);
    draw(false);
    set_plane(tex, 2);
    // SAFETY: plain GL call.
    unsafe { glBindFramebuffer(GL_FRAMEBUFFER, f0) };
    draw(false);
    assert_close(&image(tex, 0), &written(1), "face 0 holds the first resolve");
    assert_close(&image(tex, 1), &written(2), "face 1 took the plane at the second switch");
    assert_close(&plane_of(tex), &written(1), "face 0 seeded from its rows");
    release(&[f0, f1], &[tex], &[r0, r1]);
}
